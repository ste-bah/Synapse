use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{HidError, HidResult};
use crate::protocol::{
    DEVICE_COMMAND_ACK, DEVICE_COMMAND_NAK, EncodeError, MAX_FRAME_LEN, MAX_PAYLOAD_LEN,
    ParseError, encode_host_frame, parse_device_frame_prefix,
};

pub const FIRST_PIPELINE_SEQUENCE: u32 = 1;
pub const MAX_OUTSTANDING_FRAMES: usize = 16;
pub const ACK_TIMEOUT_MS: u64 = 5;
pub const MAX_ACK_RETRIES: u8 = 3;
pub const ACK_RETRY_BACKOFF_MS: [u64; MAX_ACK_RETRIES as usize] = [5, 10, 20];

pub const NAK_REASON_CRC_INVALID: u8 = 0x01;
pub const NAK_REASON_LEN_INVALID: u8 = 0x02;
pub const NAK_REASON_UNKNOWN_COMMAND: u8 = 0x03;
pub const NAK_REASON_PAYLOAD_INVALID: u8 = 0x04;
pub const NAK_REASON_BUFFER_FULL: u8 = 0x05;
pub const NAK_REASON_WATCHDOG_EXPIRED: u8 = 0x06;

const READ_CHUNK_LEN: usize = 64;
const MAX_RX_BUFFER_LEN: usize = MAX_FRAME_LEN * 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PipelineConfig {
    pub max_outstanding: usize,
    pub ack_timeout_ms: u64,
    pub max_retries: u8,
    pub retry_backoff_ms: [u64; MAX_ACK_RETRIES as usize],
}

impl PipelineConfig {
    #[must_use]
    pub const fn m4_default() -> Self {
        Self {
            max_outstanding: MAX_OUTSTANDING_FRAMES,
            ack_timeout_ms: ACK_TIMEOUT_MS,
            max_retries: MAX_ACK_RETRIES,
            retry_backoff_ms: ACK_RETRY_BACKOFF_MS,
        }
    }
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self::m4_default()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostCommandRequest<'a> {
    pub command: u8,
    pub payload: &'a [u8],
}

impl<'a> HostCommandRequest<'a> {
    #[must_use]
    pub const fn new(command: u8, payload: &'a [u8]) -> Self {
        Self { command, payload }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineResponse {
    Ack { seq: u32 },
    Nak { seq: u32, reason: u8 },
}

#[derive(Debug)]
pub struct HidPipeline {
    next_seq: u32,
    config: PipelineConfig,
    rx: Vec<u8>,
}

impl HidPipeline {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn with_config(config: PipelineConfig) -> Self {
        Self {
            next_seq: FIRST_PIPELINE_SEQUENCE,
            config,
            rx: Vec::new(),
        }
    }

    #[must_use]
    pub const fn next_sequence(&self) -> u32 {
        self.next_seq
    }

    #[must_use]
    pub const fn config(&self) -> PipelineConfig {
        self.config
    }

    #[must_use]
    pub const fn pending_rx_len(&self) -> usize {
        self.rx.len()
    }

    #[must_use]
    pub const fn pending_rx_capacity(&self) -> usize {
        self.rx.capacity()
    }

    /// Sends one ACK/NAK command and waits until the firmware accepts it.
    ///
    /// # Errors
    ///
    /// Returns [`HidError::LinkTimeout`] after the configured retry budget is
    /// exhausted, [`HidError::CommandRejected`] for malformed NAK/ACK payloads,
    /// or a serial write/read error mapped to the closest HID link failure.
    pub fn send_command<T>(
        &mut self,
        transport: &mut T,
        command: u8,
        payload: &[u8],
    ) -> HidResult<u32>
    where
        T: Read + Write + ?Sized,
    {
        let request = [HostCommandRequest::new(command, payload)];
        let seqs = self.send_commands(transport, &request)?;
        Ok(seqs[0])
    }

    /// Sends ACK/NAK commands with a bounded sliding window.
    ///
    /// The pipeline writes up to [`MAX_OUTSTANDING_FRAMES`] unacknowledged
    /// frames before waiting for responses. Retries resend the same sequence.
    ///
    /// # Errors
    ///
    /// Returns [`HidError::LinkTimeout`] after the configured retry budget is
    /// exhausted, [`HidError::CommandRejected`] for malformed NAK/ACK payloads,
    /// or a serial write/read error mapped to the closest HID link failure.
    pub fn send_commands<T>(
        &mut self,
        transport: &mut T,
        commands: &[HostCommandRequest<'_>],
    ) -> HidResult<Vec<u32>>
    where
        T: Read + Write + ?Sized,
    {
        if commands.is_empty() {
            return Ok(Vec::new());
        }

        let mut queued = VecDeque::with_capacity(commands.len());
        let mut seqs = Vec::with_capacity(commands.len());
        for request in commands {
            let seq = self.next_seq;
            let frame = encode_pending_frame(seq, request.command, request.payload)?;
            self.next_seq = self.next_seq.wrapping_add(1);
            seqs.push(seq);
            queued.push_back(PendingFrame::new(seq, frame));
        }

        let mut inflight = Vec::<PendingFrame>::new();
        let mut accepted = 0usize;
        while accepted < seqs.len() {
            self.fill_window(transport, &mut queued, &mut inflight)?;

            match self.read_response(transport)? {
                Some(PipelineResponse::Ack { seq }) => {
                    if remove_inflight(&mut inflight, seq).is_some() {
                        accepted += 1;
                    }
                }
                Some(PipelineResponse::Nak {
                    seq,
                    reason: _reason,
                }) => {
                    if let Some(pending) = find_inflight_mut(&mut inflight, seq) {
                        retry_pending(transport, pending, self.config)?;
                    }
                }
                None => {
                    if let Some(pending) = oldest_expired_mut(&mut inflight, self.config) {
                        retry_pending(transport, pending, self.config)?;
                    }
                }
            }
        }

        Ok(seqs)
    }

    fn fill_window<T>(
        &self,
        transport: &mut T,
        queued: &mut VecDeque<PendingFrame>,
        inflight: &mut Vec<PendingFrame>,
    ) -> HidResult<()>
    where
        T: Write + ?Sized,
    {
        let max_outstanding = self.config.max_outstanding.clamp(1, MAX_OUTSTANDING_FRAMES);
        while inflight.len() < max_outstanding {
            let Some(mut pending) = queued.pop_front() else {
                break;
            };
            send_pending(transport, &mut pending)?;
            inflight.push(pending);
        }
        Ok(())
    }

    fn read_response<T>(&mut self, transport: &mut T) -> HidResult<Option<PipelineResponse>>
    where
        T: Read + ?Sized,
    {
        let deadline = Instant::now() + Duration::from_millis(self.config.ack_timeout_ms);
        loop {
            if let Some(response) = self.try_parse_response()? {
                return Ok(Some(response));
            }

            if Instant::now() >= deadline {
                return Ok(None);
            }

            let mut chunk = [0u8; READ_CHUNK_LEN];
            match transport.read(&mut chunk) {
                Ok(0) => {}
                Ok(count) => {
                    if self.rx.len() + count > MAX_RX_BUFFER_LEN {
                        self.rx.clear();
                        return Err(link_timeout("reading ACK"));
                    }
                    self.rx.extend_from_slice(&chunk[..count]);
                }
                Err(error)
                    if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) =>
                {
                    if self.config.ack_timeout_ms == 0 {
                        return Ok(None);
                    }
                }
                Err(_error) => return Err(link_timeout("reading ACK")),
            }
        }
    }

    fn try_parse_response(&mut self) -> HidResult<Option<PipelineResponse>> {
        loop {
            match parse_device_frame_prefix(&self.rx) {
                Ok((frame, consumed)) => {
                    let response =
                        decode_pipeline_response(frame.seq, frame.command, frame.payload)?;
                    self.rx.drain(..consumed);
                    return Ok(Some(response));
                }
                Err(ParseError::NeedMore { .. }) => return Ok(None),
                Err(
                    ParseError::BadMagic { .. }
                    | ParseError::LenTooShort { .. }
                    | ParseError::LenOverflow { .. },
                ) => {
                    if self.rx.is_empty() {
                        return Ok(None);
                    }
                    self.rx.remove(0);
                }
                Err(ParseError::CrcInvalid { .. }) => {
                    self.rx.clear();
                    return Err(link_timeout("reading ACK"));
                }
            }
        }
    }
}

impl Default for HidPipeline {
    fn default() -> Self {
        Self::with_config(PipelineConfig::default())
    }
}

#[derive(Debug)]
struct PendingFrame {
    seq: u32,
    frame: Vec<u8>,
    retries: u8,
    last_sent_at: Instant,
}

impl PendingFrame {
    fn new(seq: u32, frame: Vec<u8>) -> Self {
        Self {
            seq,
            frame,
            retries: 0,
            last_sent_at: Instant::now(),
        }
    }
}

fn encode_pending_frame(seq: u32, command: u8, payload: &[u8]) -> HidResult<Vec<u8>> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(HidError::CommandRejected {
            seq,
            command,
            reason: NAK_REASON_PAYLOAD_INVALID,
        });
    }

    let mut frame = vec![0u8; MAX_FRAME_LEN];
    let len = encode_host_frame(seq, command, payload, &mut frame)
        .map_err(|error| encode_error(seq, command, error))?;
    frame.truncate(len);
    Ok(frame)
}

const fn encode_error(seq: u32, command: u8, error: EncodeError) -> HidError {
    let reason = match error {
        EncodeError::PayloadTooLarge => NAK_REASON_PAYLOAD_INVALID,
        EncodeError::OutputTooSmall { .. } => NAK_REASON_LEN_INVALID,
    };
    HidError::CommandRejected {
        seq,
        command,
        reason,
    }
}

fn send_pending<T>(transport: &mut T, pending: &mut PendingFrame) -> HidResult<()>
where
    T: Write + ?Sized,
{
    transport
        .write_all(&pending.frame)
        .map_err(|_error| link_timeout("writing command frame"))?;
    transport
        .flush()
        .map_err(|_error| link_timeout("flushing command frame"))?;
    pending.last_sent_at = Instant::now();
    Ok(())
}

fn retry_pending<T>(
    transport: &mut T,
    pending: &mut PendingFrame,
    config: PipelineConfig,
) -> HidResult<()>
where
    T: Write + ?Sized,
{
    if pending.retries >= config.max_retries {
        return Err(HidError::LinkTimeout {
            operation: "waiting for ACK",
            timeout_ms: config.ack_timeout_ms,
        });
    }

    let backoff = config
        .retry_backoff_ms
        .get(usize::from(pending.retries))
        .copied()
        .unwrap_or(0);
    pending.retries = pending.retries.saturating_add(1);
    if backoff > 0 {
        thread::sleep(Duration::from_millis(backoff));
    }
    send_pending(transport, pending)
}

fn oldest_expired_mut(
    inflight: &mut [PendingFrame],
    config: PipelineConfig,
) -> Option<&mut PendingFrame> {
    let timeout = Duration::from_millis(config.ack_timeout_ms);
    let now = Instant::now();
    inflight
        .iter_mut()
        .find(|pending| now.duration_since(pending.last_sent_at) >= timeout)
}

fn remove_inflight(inflight: &mut Vec<PendingFrame>, seq: u32) -> Option<PendingFrame> {
    let index = inflight.iter().position(|pending| pending.seq == seq)?;
    Some(inflight.remove(index))
}

fn find_inflight_mut(inflight: &mut [PendingFrame], seq: u32) -> Option<&mut PendingFrame> {
    inflight.iter_mut().find(|pending| pending.seq == seq)
}

fn decode_pipeline_response(seq: u32, command: u8, payload: &[u8]) -> HidResult<PipelineResponse> {
    match command {
        DEVICE_COMMAND_ACK => {
            let acked = parse_seq_payload(seq, command, payload)?;
            Ok(PipelineResponse::Ack { seq: acked })
        }
        DEVICE_COMMAND_NAK => {
            if payload.len() != 5 {
                return Err(HidError::CommandRejected {
                    seq,
                    command,
                    reason: NAK_REASON_PAYLOAD_INVALID,
                });
            }
            let acked = parse_seq_payload(seq, command, &payload[..4])?;
            Ok(PipelineResponse::Nak {
                seq: acked,
                reason: payload[4],
            })
        }
        _ => Err(HidError::CommandRejected {
            seq,
            command,
            reason: NAK_REASON_UNKNOWN_COMMAND,
        }),
    }
}

fn parse_seq_payload(frame_seq: u32, command: u8, payload: &[u8]) -> HidResult<u32> {
    if payload.len() != 4 {
        return Err(HidError::CommandRejected {
            seq: frame_seq,
            command,
            reason: NAK_REASON_PAYLOAD_INVALID,
        });
    }

    let payload_seq = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    if payload_seq != frame_seq {
        return Err(HidError::CommandRejected {
            seq: frame_seq,
            command,
            reason: NAK_REASON_PAYLOAD_INVALID,
        });
    }

    Ok(payload_seq)
}

const fn link_timeout(operation: &'static str) -> HidError {
    HidError::LinkTimeout {
        operation,
        timeout_ms: ACK_TIMEOUT_MS,
    }
}
