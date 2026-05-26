use std::collections::VecDeque;
use std::io::{self, ErrorKind, Read, Write};
use std::time::Duration;

use pico_hid::dispatch::{DispatchState, IdentifyInfo, Telemetry, dispatch_frame};
use pico_hid::protocol::{
    DeviceCommand, DropReason, MAX_FRAME_LEN, NakReason, ParseResult, encode_device_frame,
    encode_nak, parse_host_frame,
};
use pico_hid::safety::{Watchdog, WatchdogPoll};
use serialport::{ClearBuffer, DataBits, FlowControl, Parity, SerialPort, StopBits};
use synapse_core::{
    SYNAPSE_PICO_HID_BUILD_HASH_LEN, SYNAPSE_PICO_HID_USB_PID, SYNAPSE_PICO_HID_USB_VID,
};

pub const MOCK_PICO_PORT_NAME: &str = "mock://synapse-pico-hid";
pub const MOCK_PICO_BAUD_RATE: u32 = 1_000_000;
pub const MOCK_PICO_READ_TIMEOUT: Duration = Duration::from_millis(5);
pub const MOCK_PICO_BUILD_HASH: [u8; SYNAPSE_PICO_HID_BUILD_HASH_LEN] = *b"MOCKPICO";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoopbackTelemetry {
    pub uptime_ms: u32,
    pub frames_received: u32,
    pub frames_dropped: u32,
    pub link_errors: u32,
    pub commands_executed: u32,
    pub watchdog_fires: u32,
    pub crc_errors: u32,
}

impl From<Telemetry> for LoopbackTelemetry {
    fn from(value: Telemetry) -> Self {
        Self {
            uptime_ms: value.uptime_ms,
            frames_received: value.frames_received,
            frames_dropped: value.frames_dropped,
            link_errors: value.link_errors,
            commands_executed: value.commands_executed,
            watchdog_fires: value.watchdog_fires,
            crc_errors: value.crc_errors,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoopbackReports {
    pub mouse: [u8; 4],
    pub keyboard: [u8; 8],
    pub gamepad: [u8; 14],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoopbackDrop {
    pub reason: DropReason,
    pub consumed: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoopbackNak {
    pub seq: u32,
    pub command: u8,
    pub reason: NakReason,
}

#[derive(Clone, Debug)]
pub struct MockPicoFirmware {
    port_name: String,
    baud_rate: u32,
    data_bits: DataBits,
    flow_control: FlowControl,
    parity: Parity,
    stop_bits: StopBits,
    timeout: Duration,
    identify: IdentifyInfo,
    state: DispatchState,
    watchdog: Watchdog,
    now_ms: u32,
    rx: Vec<u8>,
    tx: VecDeque<u8>,
    host_bytes_written: usize,
    device_bytes_read: usize,
    host_frames_processed: u32,
    drops: Vec<LoopbackDrop>,
    naks: Vec<LoopbackNak>,
}

impl MockPicoFirmware {
    #[must_use]
    pub fn new() -> Self {
        Self::with_identity(IdentifyInfo::new(
            MOCK_PICO_BUILD_HASH,
            SYNAPSE_PICO_HID_USB_VID,
            SYNAPSE_PICO_HID_USB_PID,
        ))
    }

    #[must_use]
    pub fn with_identity(identify: IdentifyInfo) -> Self {
        Self {
            port_name: MOCK_PICO_PORT_NAME.to_owned(),
            baud_rate: MOCK_PICO_BAUD_RATE,
            data_bits: DataBits::Eight,
            flow_control: FlowControl::None,
            parity: Parity::None,
            stop_bits: StopBits::One,
            timeout: MOCK_PICO_READ_TIMEOUT,
            identify,
            state: DispatchState::new(),
            watchdog: Watchdog::new(),
            now_ms: 0,
            rx: Vec::new(),
            tx: VecDeque::new(),
            host_bytes_written: 0,
            device_bytes_read: 0,
            host_frames_processed: 0,
            drops: Vec::new(),
            naks: Vec::new(),
        }
    }

    #[must_use]
    pub const fn pending_rx_len(&self) -> usize {
        self.rx.len()
    }

    #[must_use]
    pub fn pending_tx_len(&self) -> usize {
        self.tx.len()
    }

    #[must_use]
    pub const fn host_bytes_written(&self) -> usize {
        self.host_bytes_written
    }

    #[must_use]
    pub const fn device_bytes_read(&self) -> usize {
        self.device_bytes_read
    }

    #[must_use]
    pub const fn host_frames_processed(&self) -> u32 {
        self.host_frames_processed
    }

    #[must_use]
    pub const fn now_ms(&self) -> u32 {
        self.now_ms
    }

    #[must_use]
    pub const fn watchdog_fired(&self) -> bool {
        self.watchdog.fired()
    }

    #[must_use]
    pub const fn watchdog_timeout_ms(&self) -> u32 {
        self.watchdog.timeout_ms()
    }

    #[must_use]
    pub const fn identify(&self) -> IdentifyInfo {
        self.identify
    }

    #[must_use]
    pub fn telemetry(&self) -> LoopbackTelemetry {
        self.state.telemetry.into()
    }

    #[must_use]
    pub fn reports(&self) -> LoopbackReports {
        LoopbackReports {
            mouse: self.state.mouse.to_bytes(),
            keyboard: self.state.keyboard.to_bytes(),
            gamepad: self.state.gamepad.to_bytes(),
        }
    }

    #[must_use]
    pub fn drops(&self) -> &[LoopbackDrop] {
        &self.drops
    }

    #[must_use]
    pub fn naks(&self) -> &[LoopbackNak] {
        &self.naks
    }

    pub fn advance_time(&mut self, elapsed_ms: u32) -> WatchdogPoll {
        self.now_ms = self.now_ms.wrapping_add(elapsed_ms);
        self.state.telemetry.uptime_ms = self.now_ms;
        self.watchdog.poll(self.now_ms, &mut self.state)
    }

    fn process_rx(&mut self) -> io::Result<()> {
        loop {
            let consumed = match parse_host_frame(&self.rx) {
                ParseResult::Frame { frame, consumed } => {
                    let outcome = dispatch_frame(&mut self.state, frame, self.identify);
                    let mut response = [0u8; MAX_FRAME_LEN];
                    let len = encode_device_frame(
                        frame.seq,
                        outcome.command,
                        &outcome.payload[..outcome.payload_len],
                        &mut response,
                    )
                    .map_err(encode_error)?;
                    self.enqueue_tx(&response[..len]);
                    if outcome.command != DeviceCommand::Nak {
                        self.watchdog
                            .record_valid_command(self.now_ms, self.state.watchdog_timeout_ms);
                    }
                    self.host_frames_processed = self.host_frames_processed.wrapping_add(1);
                    consumed
                }
                ParseResult::Nak { nak, consumed } => {
                    if matches!(nak.reason, NakReason::CrcInvalid) {
                        self.state.telemetry.record_crc_error();
                    } else {
                        self.state.telemetry.record_link_error();
                    }
                    self.naks.push(LoopbackNak {
                        seq: nak.seq,
                        command: nak.command,
                        reason: nak.reason,
                    });
                    let mut response = [0u8; MAX_FRAME_LEN];
                    let len =
                        encode_nak(nak.seq, nak.reason, &mut response).map_err(encode_error)?;
                    self.enqueue_tx(&response[..len]);
                    consumed
                }
                ParseResult::Drop { reason, consumed } => {
                    self.state.telemetry.record_frame_dropped();
                    self.drops.push(LoopbackDrop { reason, consumed });
                    consumed
                }
                ParseResult::NeedMore { .. } => break,
            };

            consume_rx(&mut self.rx, consumed);
        }

        Ok(())
    }

    fn enqueue_tx(&mut self, bytes: &[u8]) {
        self.tx.extend(bytes.iter().copied());
    }
}

impl Default for MockPicoFirmware {
    fn default() -> Self {
        Self::new()
    }
}

impl Read for MockPicoFirmware {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.tx.is_empty() {
            return Err(io::Error::new(
                ErrorKind::TimedOut,
                "mock Pico has no queued response",
            ));
        }

        let count = buffer.len().min(self.tx.len());
        for slot in &mut buffer[..count] {
            let Some(byte) = self.tx.pop_front() else {
                return Err(io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "mock Pico TX queue drained early",
                ));
            };
            *slot = byte;
        }
        self.device_bytes_read += count;
        Ok(count)
    }
}

impl Write for MockPicoFirmware {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.host_bytes_written += buffer.len();
        self.rx.extend_from_slice(buffer);
        self.process_rx()?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl SerialPort for MockPicoFirmware {
    fn name(&self) -> Option<String> {
        Some(self.port_name.clone())
    }

    fn baud_rate(&self) -> serialport::Result<u32> {
        Ok(self.baud_rate)
    }

    fn data_bits(&self) -> serialport::Result<DataBits> {
        Ok(self.data_bits)
    }

    fn flow_control(&self) -> serialport::Result<FlowControl> {
        Ok(self.flow_control)
    }

    fn parity(&self) -> serialport::Result<Parity> {
        Ok(self.parity)
    }

    fn stop_bits(&self) -> serialport::Result<StopBits> {
        Ok(self.stop_bits)
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    fn set_baud_rate(&mut self, baud_rate: u32) -> serialport::Result<()> {
        self.baud_rate = baud_rate;
        Ok(())
    }

    fn set_data_bits(&mut self, data_bits: DataBits) -> serialport::Result<()> {
        self.data_bits = data_bits;
        Ok(())
    }

    fn set_flow_control(&mut self, flow_control: FlowControl) -> serialport::Result<()> {
        self.flow_control = flow_control;
        Ok(())
    }

    fn set_parity(&mut self, parity: Parity) -> serialport::Result<()> {
        self.parity = parity;
        Ok(())
    }

    fn set_stop_bits(&mut self, stop_bits: StopBits) -> serialport::Result<()> {
        self.stop_bits = stop_bits;
        Ok(())
    }

    fn set_timeout(&mut self, timeout: Duration) -> serialport::Result<()> {
        self.timeout = timeout;
        Ok(())
    }

    fn write_request_to_send(&mut self, _level: bool) -> serialport::Result<()> {
        Ok(())
    }

    fn write_data_terminal_ready(&mut self, _level: bool) -> serialport::Result<()> {
        Ok(())
    }

    fn read_clear_to_send(&mut self) -> serialport::Result<bool> {
        Ok(true)
    }

    fn read_data_set_ready(&mut self) -> serialport::Result<bool> {
        Ok(true)
    }

    fn read_ring_indicator(&mut self) -> serialport::Result<bool> {
        Ok(false)
    }

    fn read_carrier_detect(&mut self) -> serialport::Result<bool> {
        Ok(true)
    }

    fn bytes_to_read(&self) -> serialport::Result<u32> {
        len_to_u32(self.tx.len())
    }

    fn bytes_to_write(&self) -> serialport::Result<u32> {
        Ok(0)
    }

    fn clear(&self, _buffer_to_clear: ClearBuffer) -> serialport::Result<()> {
        Err(serial_error(
            "mock Pico clear requires mutable access to firmware queues",
        ))
    }

    fn try_clone(&self) -> serialport::Result<Box<dyn SerialPort>> {
        Err(serial_error("mock Pico serial state cannot be cloned"))
    }

    fn set_break(&self) -> serialport::Result<()> {
        Ok(())
    }

    fn clear_break(&self) -> serialport::Result<()> {
        Ok(())
    }
}

fn consume_rx(rx: &mut Vec<u8>, consumed: usize) {
    if consumed >= rx.len() {
        rx.clear();
    } else {
        rx.drain(..consumed);
    }
}

fn encode_error(error: pico_hid::protocol::EncodeError) -> io::Error {
    io::Error::new(
        ErrorKind::InvalidData,
        format!("mock Pico failed to encode device frame: {error:?}"),
    )
}

fn len_to_u32(value: usize) -> serialport::Result<u32> {
    u32::try_from(value).map_err(|_error| serial_error("mock Pico buffer length exceeded u32"))
}

fn serial_error(description: &str) -> serialport::Error {
    serialport::Error::new(serialport::ErrorKind::Unknown, description)
}
