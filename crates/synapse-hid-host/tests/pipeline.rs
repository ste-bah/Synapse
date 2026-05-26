use std::io::{self, ErrorKind, Read, Write};

use proptest::test_runner::{Config as ProptestConfig, TestCaseError};
use synapse_core::error_codes;
use synapse_hid_host::{
    ACK_RETRY_BACKOFF_MS, ACK_TIMEOUT_MS, DEVICE_COMMAND_ACK, DEVICE_COMMAND_NAK,
    HOST_COMMAND_MOUSE_MOVE_REL, HOST_MAGIC, HidError, HidPipeline, MAX_ACK_RETRIES, MAX_FRAME_LEN,
    MAX_OUTSTANDING_FRAMES, NAK_REASON_BUFFER_FULL, NAK_REASON_PAYLOAD_INVALID, ParseError,
    PipelineConfig, encode_device_frame, parse_device_frame_prefix,
};

#[test]
fn pipeline_defaults_match_m4_contract() {
    let config = PipelineConfig::default();

    assert_eq!(config.max_outstanding, 16);
    assert_eq!(config.max_outstanding, MAX_OUTSTANDING_FRAMES);
    assert_eq!(config.ack_timeout_ms, 5);
    assert_eq!(config.ack_timeout_ms, ACK_TIMEOUT_MS);
    assert_eq!(config.max_retries, 3);
    assert_eq!(config.max_retries, MAX_ACK_RETRIES);
    assert_eq!(config.retry_backoff_ms, [5, 10, 20]);
    assert_eq!(config.retry_backoff_ms, ACK_RETRY_BACKOFF_MS);
}

#[test]
fn send_commands_writes_sixteen_frames_before_first_ack_read() {
    let mut responses = Vec::new();
    for seq in 1..=20 {
        responses.extend_from_slice(&ack(seq));
    }
    let mut transport = ScriptedTransport::new(responses);
    let commands = vec![move_rel_command(); 20];
    let mut pipeline = HidPipeline::new();
    assert_eq!(pipeline.pending_rx_len(), 0);
    assert_eq!(pipeline.pending_rx_capacity(), 0);
    assert_eq!(pipeline.next_sequence(), 1);

    let seqs = match pipeline.send_commands(&mut transport, &commands) {
        Ok(seqs) => seqs,
        Err(error) => panic!("twenty ACKed commands should pass: {error}"),
    };

    assert_eq!(seqs, (1..=20).collect::<Vec<u32>>());
    assert_eq!(transport.first_read_write_count, Some(16));
    assert_eq!(transport.written.len(), 20);
    assert_eq!(host_frame_seq(&transport.written[0]), 1);
    assert_eq!(host_frame_seq(&transport.written[15]), 16);
    assert_eq!(host_frame_seq(&transport.written[19]), 20);
    assert_eq!(pipeline.pending_rx_len(), 0);
    assert!(pipeline.pending_rx_capacity() <= MAX_FRAME_LEN * 2);
    assert_eq!(pipeline.next_sequence(), 21);
}

#[test]
fn nak_retries_same_sequence_once_and_succeeds() {
    let mut responses = Vec::new();
    responses.extend_from_slice(&nak(1, NAK_REASON_BUFFER_FULL));
    responses.extend_from_slice(&ack(1));
    let mut transport = ScriptedTransport::new(responses);
    let mut pipeline = HidPipeline::new();

    let seq =
        match pipeline.send_command(&mut transport, HOST_COMMAND_MOUSE_MOVE_REL, &[1, 0, 2, 0]) {
            Ok(seq) => seq,
            Err(error) => panic!("single NAK followed by ACK should pass: {error}"),
        };

    assert_eq!(seq, 1);
    assert_eq!(transport.written.len(), 2);
    assert_eq!(transport.written[0], transport.written[1]);
    assert_eq!(host_frame_seq(&transport.written[0]), 1);
}

#[test]
fn timeout_retries_three_times_then_returns_link_timeout() {
    let mut transport = ScriptedTransport::new(Vec::new());
    let mut pipeline = HidPipeline::with_config(PipelineConfig {
        max_outstanding: MAX_OUTSTANDING_FRAMES,
        ack_timeout_ms: 0,
        max_retries: MAX_ACK_RETRIES,
        retry_backoff_ms: [0, 0, 0],
    });

    let error =
        match pipeline.send_command(&mut transport, HOST_COMMAND_MOUSE_MOVE_REL, &[1, 0, 2, 0]) {
            Ok(seq) => panic!("silent transport should time out, accepted seq {seq}"),
            Err(error) => error,
        };

    assert_eq!(
        error,
        HidError::LinkTimeout {
            operation: "waiting for ACK",
            timeout_ms: 0,
        }
    );
    assert_eq!(error.code(), error_codes::HID_LINK_TIMEOUT);
    assert_eq!(transport.written.len(), 4);
}

#[test]
fn malformed_ack_payload_seq_is_rejected() {
    let payload_seq = 2u32.to_le_bytes();
    let mut frame = [0u8; MAX_FRAME_LEN];
    let len = match encode_device_frame(1, DEVICE_COMMAND_ACK, &payload_seq, &mut frame) {
        Ok(len) => len,
        Err(error) => panic!("malformed ACK test frame should encode: {error:?}"),
    };
    let mut transport = ScriptedTransport::new(frame[..len].to_vec());
    let mut pipeline = HidPipeline::new();

    let error =
        match pipeline.send_command(&mut transport, HOST_COMMAND_MOUSE_MOVE_REL, &[1, 0, 2, 0]) {
            Ok(seq) => panic!("mismatched ACK payload should fail, accepted seq {seq}"),
            Err(error) => error,
        };

    assert_eq!(
        error,
        HidError::CommandRejected {
            seq: 1,
            command: DEVICE_COMMAND_ACK,
            reason: NAK_REASON_PAYLOAD_INVALID,
        }
    );
    assert_eq!(error.code(), error_codes::HID_COMMAND_REJECTED);
}

#[test]
fn send_command_reassembles_ack_across_single_byte_reads() {
    let mut transport = ScriptedTransport::with_read_chunks(ack(1), repeating_chunks(1, 32));
    let mut pipeline = HidPipeline::new();
    assert_eq!(pipeline.pending_rx_len(), 0);
    assert_eq!(pipeline.pending_rx_capacity(), 0);
    assert_eq!(pipeline.next_sequence(), 1);

    let seq =
        match pipeline.send_command(&mut transport, HOST_COMMAND_MOUSE_MOVE_REL, &[1, 0, 2, 0]) {
            Ok(seq) => seq,
            Err(error) => panic!("single-byte ACK stream should pass: {error}"),
        };

    assert_eq!(seq, 1);
    assert_eq!(transport.read_offset, transport.read_data.len());
    assert_eq!(pipeline.pending_rx_len(), 0);
    assert!(pipeline.pending_rx_capacity() <= MAX_FRAME_LEN * 2);
    assert_eq!(pipeline.next_sequence(), 2);
}

#[test]
fn send_command_discards_garbage_before_magic_and_reassembles_ack() {
    let mut responses = vec![0x00, 0xFF, 0x7E];
    responses.extend_from_slice(&ack(1));
    let mut transport = ScriptedTransport::with_read_chunks(responses, repeating_chunks(1, 64));
    let mut pipeline = HidPipeline::new();
    assert_eq!(pipeline.pending_rx_len(), 0);
    assert_eq!(pipeline.pending_rx_capacity(), 0);
    assert_eq!(pipeline.next_sequence(), 1);

    let seq =
        match pipeline.send_command(&mut transport, HOST_COMMAND_MOUSE_MOVE_REL, &[1, 0, 2, 0]) {
            Ok(seq) => seq,
            Err(error) => panic!("garbage prefix should resync before ACK: {error}"),
        };

    assert_eq!(seq, 1);
    assert_eq!(transport.read_offset, transport.read_data.len());
    assert_eq!(pipeline.pending_rx_len(), 0);
    assert!(pipeline.pending_rx_capacity() <= MAX_FRAME_LEN * 2);
    assert_eq!(pipeline.next_sequence(), 2);
}

#[test]
fn send_commands_drains_rx_after_ten_thousand_split_frames() {
    let command_count = 10_000usize;
    let last_seq = count_to_u32(command_count);
    let responses = ack_stream(last_seq);
    let chunk_sizes = chunk_sizes_for_len(responses.len(), 0xC0DE_CAFE_F00D_BAAD);
    let mut transport = ScriptedTransport::with_read_chunks(responses, chunk_sizes);
    let commands = vec![move_rel_command(); command_count];
    let mut pipeline = HidPipeline::new();
    assert_eq!(pipeline.pending_rx_len(), 0);
    assert_eq!(pipeline.pending_rx_capacity(), 0);
    assert_eq!(pipeline.next_sequence(), 1);

    let seqs = match pipeline.send_commands(&mut transport, &commands) {
        Ok(seqs) => seqs,
        Err(error) => panic!("ten thousand split ACK frames should pass: {error}"),
    };

    assert_eq!(seqs.len(), command_count);
    assert_eq!(seqs.first().copied(), Some(1));
    assert_eq!(seqs.last().copied(), Some(last_seq));
    assert_eq!(transport.read_offset, transport.read_data.len());
    assert_eq!(pipeline.pending_rx_len(), 0);
    assert!(pipeline.pending_rx_capacity() <= MAX_FRAME_LEN * 2);
    assert_eq!(pipeline.next_sequence(), last_seq + 1);
}

proptest::proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn device_frames_reassemble_byte_equal_across_random_chunks(
        frame_count in 1usize..128,
        chunk_seed in proptest::prelude::any::<u64>(),
    ) {
        let frames = ack_frames(count_to_u32(frame_count));
        let stream = flatten_frames(&frames);
        let chunks = split_stream(&stream, chunk_seed);
        let reconstructed = match reassemble_device_frame_bytes(&chunks) {
            Ok(frames) => frames,
            Err(error) => {
                return Err(TestCaseError::fail(format!(
                    "known-valid frame stream failed to reassemble: {error:?}"
                )));
            }
        };

        proptest::prop_assert_eq!(reconstructed, frames);
    }

    #[test]
    fn pipeline_reassembles_acks_across_random_usb_chunks(
        command_count in 1usize..128,
        chunk_seed in proptest::prelude::any::<u64>(),
    ) {
        let last_seq = count_to_u32(command_count);
        let responses = ack_stream(last_seq);
        let chunk_sizes = chunk_sizes_for_len(responses.len(), chunk_seed);
        let mut transport = ScriptedTransport::with_read_chunks(responses, chunk_sizes);
        let commands = vec![move_rel_command(); command_count];
        let mut pipeline = HidPipeline::new();

        let seqs = match pipeline.send_commands(&mut transport, &commands) {
            Ok(seqs) => seqs,
            Err(error) => {
                return Err(TestCaseError::fail(format!(
                    "pipeline failed on valid chunked ACK stream: {error}"
                )));
            }
        };

        proptest::prop_assert_eq!(seqs, expected_seqs(command_count));
        proptest::prop_assert_eq!(transport.read_offset, transport.read_data.len());
        proptest::prop_assert_eq!(pipeline.pending_rx_len(), 0);
    }
}

const fn move_rel_command() -> synapse_hid_host::HostCommandRequest<'static> {
    synapse_hid_host::HostCommandRequest::new(HOST_COMMAND_MOUSE_MOVE_REL, &[1, 0, 2, 0])
}

fn ack_frames(last_seq: u32) -> Vec<Vec<u8>> {
    (1..=last_seq).map(ack).collect()
}

fn ack_stream(last_seq: u32) -> Vec<u8> {
    let mut stream = Vec::new();
    for frame in ack_frames(last_seq) {
        stream.extend_from_slice(&frame);
    }
    stream
}

fn ack(seq: u32) -> Vec<u8> {
    let payload = seq.to_le_bytes();
    let mut frame = [0u8; MAX_FRAME_LEN];
    let len = match encode_device_frame(seq, DEVICE_COMMAND_ACK, &payload, &mut frame) {
        Ok(len) => len,
        Err(error) => panic!("ACK frame should encode: {error:?}"),
    };
    frame[..len].to_vec()
}

fn nak(seq: u32, reason: u8) -> Vec<u8> {
    let mut payload = [0u8; 5];
    payload[..4].copy_from_slice(&seq.to_le_bytes());
    payload[4] = reason;
    let mut frame = [0u8; MAX_FRAME_LEN];
    let len = match encode_device_frame(seq, DEVICE_COMMAND_NAK, &payload, &mut frame) {
        Ok(len) => len,
        Err(error) => panic!("NAK frame should encode: {error:?}"),
    };
    frame[..len].to_vec()
}

fn host_frame_seq(frame: &[u8]) -> u32 {
    assert_eq!(frame[0], HOST_MAGIC);
    u32::from_le_bytes([frame[3], frame[4], frame[5], frame[6]])
}

fn expected_seqs(count: usize) -> Vec<u32> {
    (1..=count_to_u32(count)).collect()
}

fn count_to_u32(count: usize) -> u32 {
    match u32::try_from(count) {
        Ok(value) => value,
        Err(error) => panic!("test frame count must fit u32: {error}"),
    }
}

fn repeating_chunks(size: usize, count: usize) -> Vec<usize> {
    vec![size; count]
}

fn flatten_frames(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut stream = Vec::new();
    for frame in frames {
        stream.extend_from_slice(frame);
    }
    stream
}

fn split_stream(stream: &[u8], seed: u64) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    let mut offset = 0usize;
    for chunk_size in chunk_sizes_for_len(stream.len(), seed) {
        let end = (offset + chunk_size).min(stream.len());
        chunks.push(stream[offset..end].to_vec());
        offset = end;
    }
    chunks
}

fn chunk_sizes_for_len(byte_len: usize, seed: u64) -> Vec<usize> {
    let mut sizes = Vec::new();
    let mut remaining = byte_len;
    let mut state = seed;

    while remaining > 0 {
        let size = next_chunk_size(&mut state).min(remaining);
        sizes.push(size);
        remaining -= size;
    }

    sizes
}

fn next_chunk_size(state: &mut u64) -> usize {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    usize::from(state.to_le_bytes()[0] % 64) + 1
}

fn reassemble_device_frame_bytes(chunks: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, ParseError> {
    let mut rx = Vec::new();
    let mut frames = Vec::new();

    for chunk in chunks {
        rx.extend_from_slice(chunk);

        loop {
            match parse_device_frame_prefix(&rx) {
                Ok((_frame, consumed)) => {
                    frames.push(rx[..consumed].to_vec());
                    rx.drain(..consumed);
                }
                Err(ParseError::NeedMore { .. }) => break,
                Err(
                    ParseError::BadMagic { .. }
                    | ParseError::LenTooShort { .. }
                    | ParseError::LenOverflow { .. },
                ) => {
                    if rx.is_empty() {
                        break;
                    }
                    rx.remove(0);
                }
                Err(error @ ParseError::CrcInvalid { .. }) => return Err(error),
            }
        }
    }

    Ok(frames)
}

struct ScriptedTransport {
    read_data: Vec<u8>,
    read_offset: usize,
    read_chunk_sizes: Vec<usize>,
    read_calls: usize,
    written: Vec<Vec<u8>>,
    first_read_write_count: Option<usize>,
}

impl ScriptedTransport {
    const fn new(read_data: Vec<u8>) -> Self {
        Self {
            read_data,
            read_offset: 0,
            read_chunk_sizes: Vec::new(),
            read_calls: 0,
            written: Vec::new(),
            first_read_write_count: None,
        }
    }

    const fn with_read_chunks(read_data: Vec<u8>, read_chunk_sizes: Vec<usize>) -> Self {
        Self {
            read_data,
            read_offset: 0,
            read_chunk_sizes,
            read_calls: 0,
            written: Vec::new(),
            first_read_write_count: None,
        }
    }

    fn next_read_limit(&mut self, buffer_len: usize) -> usize {
        let limit = self
            .read_chunk_sizes
            .get(self.read_calls)
            .copied()
            .unwrap_or(buffer_len);
        self.read_calls += 1;
        limit.clamp(1, buffer_len)
    }
}

impl Read for ScriptedTransport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.first_read_write_count.is_none() {
            self.first_read_write_count = Some(self.written.len());
        }

        if self.read_offset >= self.read_data.len() {
            return Err(io::Error::new(ErrorKind::TimedOut, "scripted timeout"));
        }

        let remaining = self.read_data.len() - self.read_offset;
        let count = remaining
            .min(buffer.len())
            .min(self.next_read_limit(buffer.len()));
        buffer[..count]
            .copy_from_slice(&self.read_data[self.read_offset..self.read_offset + count]);
        self.read_offset += count;
        Ok(count)
    }
}

impl Write for ScriptedTransport {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.written.push(buffer.to_vec());
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
