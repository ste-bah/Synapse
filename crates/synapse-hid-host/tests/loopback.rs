use std::io::{Read, Write};
use std::time::Duration;

use pico_hid::safety::{DEFAULT_WATCHDOG_TIMEOUT_MS, WatchdogPoll};
use synapse_core::{
    SYNAPSE_PICO_HID_BUILD_HASH_LEN, SYNAPSE_PICO_HID_USB_PID, SYNAPSE_PICO_HID_USB_VID,
};
use synapse_hid_host::{
    DEVICE_COMMAND_NAK, HOST_COMMAND_KEY_DOWN, HOST_COMMAND_MOUSE_MOVE_REL, HidPipeline,
    NAK_REASON_CRC_INVALID, NAK_REASON_PAYLOAD_INVALID, PipelineResponse,
    perform_identify_handshake,
};
use synapse_test_utils::hid_loopback::{
    MOCK_PICO_BAUD_RATE, MOCK_PICO_BUILD_HASH, MOCK_PICO_PORT_NAME, MockPicoFirmware,
};

#[test]
fn loopback_identify_uses_hid_host_handshake_path() {
    let mut firmware = MockPicoFirmware::new();
    assert_eq!(firmware.pending_rx_len(), 0);
    assert_eq!(firmware.pending_tx_len(), 0);
    assert_eq!(firmware.host_frames_processed(), 0);

    let identity = match perform_identify_handshake(&mut firmware, Duration::from_millis(200)) {
        Ok(identity) => identity,
        Err(error) => panic!("mock Pico identify should pass: {error}"),
    };

    assert_eq!(identity.build_hash, MOCK_PICO_BUILD_HASH);
    assert_eq!(identity.vid, SYNAPSE_PICO_HID_USB_VID);
    assert_eq!(identity.pid, SYNAPSE_PICO_HID_USB_PID);
    assert_eq!(identity.build_hash.len(), SYNAPSE_PICO_HID_BUILD_HASH_LEN);
    assert_eq!(firmware.host_frames_processed(), 1);
    assert_eq!(firmware.pending_rx_len(), 0);
    assert_eq!(firmware.pending_tx_len(), 0);
    assert!(firmware.host_bytes_written() > 0);
    assert!(firmware.device_bytes_read() > 0);
    assert_eq!(firmware.telemetry().frames_received, 1);
    assert_eq!(firmware.telemetry().commands_executed, 1);
}

#[test]
fn loopback_pipeline_dispatches_mouse_command_and_ack() {
    let mut firmware = MockPicoFirmware::new();
    let mut pipeline = HidPipeline::new();
    let payload = [12u8, 0, 250, 255];
    assert_eq!(firmware.reports().mouse, [0, 0, 0, 0]);

    let seq = match pipeline.send_command(&mut firmware, HOST_COMMAND_MOUSE_MOVE_REL, &payload) {
        Ok(seq) => seq,
        Err(error) => panic!("mock Pico mouse command should ACK: {error}"),
    };

    assert_eq!(seq, 1);
    assert_eq!(pipeline.pending_inflight_len(), 0);
    assert_eq!(firmware.reports().mouse, [0, 12, 250, 0]);
    assert_eq!(firmware.host_frames_processed(), 1);
    assert_eq!(firmware.pending_rx_len(), 0);
    assert_eq!(firmware.pending_tx_len(), 0);
    assert_eq!(firmware.telemetry().frames_received, 1);
    assert_eq!(firmware.telemetry().commands_executed, 1);
    assert_eq!(firmware.telemetry().link_errors, 0);
}

#[test]
fn loopback_pipeline_surfaces_payload_nak_without_mutating_reports() {
    let mut firmware = MockPicoFirmware::new();
    let mut pipeline = HidPipeline::new();
    assert_eq!(firmware.reports().mouse, [0, 0, 0, 0]);

    let seq = match pipeline.try_send_command(&mut firmware, HOST_COMMAND_MOUSE_MOVE_REL, &[128]) {
        Ok(seq) => seq,
        Err(error) => panic!("malformed command should still enqueue before NAK: {error}"),
    };
    assert_eq!(seq, 1);
    let response = match pipeline.poll_response(&mut firmware) {
        Ok(Some(response)) => response,
        Ok(None) => panic!("mock Pico should queue a NAK response"),
        Err(error) => panic!("mock Pico NAK should be parseable: {error}"),
    };

    assert_eq!(
        response,
        PipelineResponse::Nak {
            seq: 1,
            reason: NAK_REASON_PAYLOAD_INVALID,
        }
    );
    assert_eq!(firmware.reports().mouse, [0, 0, 0, 0]);
    assert_eq!(firmware.telemetry().frames_received, 2);
    assert_eq!(firmware.telemetry().commands_executed, 0);
    assert_eq!(firmware.telemetry().link_errors, 2);
    assert!(firmware.pending_tx_len() > 0);
}

#[test]
fn loopback_malformed_crc_returns_device_nak_and_records_crc_error() {
    let mut firmware = MockPicoFirmware::new();
    let mut frame = [0u8; synapse_hid_host::MAX_FRAME_LEN];
    let len = match synapse_hid_host::encode_host_frame(
        7,
        HOST_COMMAND_MOUSE_MOVE_REL,
        &[1, 0, 2, 0],
        &mut frame,
    ) {
        Ok(len) => len,
        Err(error) => panic!("host frame should encode: {error:?}"),
    };
    frame[len - 1] ^= 0x55;

    assert_eq!(firmware.telemetry().crc_errors, 0);
    firmware
        .write_all(&frame[..len])
        .unwrap_or_else(|error| panic!("mock Pico write should accept corrupt frame: {error}"));
    assert_eq!(firmware.naks().len(), 1);
    assert_eq!(firmware.naks()[0].reason as u8, NAK_REASON_CRC_INVALID);

    let mut response = [0u8; synapse_hid_host::MAX_FRAME_LEN];
    let count = firmware
        .read(&mut response)
        .unwrap_or_else(|error| panic!("mock Pico should queue CRC NAK: {error}"));
    let parsed = match synapse_hid_host::parse_device_frame(&response[..count]) {
        Ok(frame) => frame,
        Err(error) => panic!("CRC NAK should parse as device frame: {error:?}"),
    };

    assert_eq!(parsed.seq, 7);
    assert_eq!(parsed.command, DEVICE_COMMAND_NAK);
    assert_eq!(parsed.payload[4], NAK_REASON_CRC_INVALID);
    assert_eq!(firmware.telemetry().crc_errors, 1);
    assert_eq!(firmware.telemetry().link_errors, 1);
}

#[test]
fn loopback_watchdog_releases_held_keyboard_state() {
    let mut firmware = MockPicoFirmware::new();
    let mut pipeline = HidPipeline::new();
    let key_a = [0x04];
    let seq = match pipeline.send_command(&mut firmware, HOST_COMMAND_KEY_DOWN, &key_a) {
        Ok(seq) => seq,
        Err(error) => panic!("mock Pico key down should ACK: {error}"),
    };
    assert_eq!(seq, 1);
    assert_eq!(firmware.reports().keyboard, [0, 0, 4, 0, 0, 0, 0, 0]);

    assert_eq!(
        firmware.advance_time(DEFAULT_WATCHDOG_TIMEOUT_MS - 1),
        WatchdogPoll::Noop
    );
    assert_eq!(firmware.reports().keyboard, [0, 0, 4, 0, 0, 0, 0, 0]);
    assert!(!firmware.watchdog_fired());

    assert_eq!(firmware.advance_time(1), WatchdogPoll::Fired);
    assert_eq!(firmware.reports().keyboard, [0; 8]);
    assert!(firmware.watchdog_fired());
    assert_eq!(firmware.telemetry().watchdog_fires, 1);
}

#[test]
fn loopback_serialport_settings_are_inspectable() {
    let mut firmware = MockPicoFirmware::new();
    assert_eq!(
        serialport::SerialPort::name(&firmware).as_deref(),
        Some(MOCK_PICO_PORT_NAME)
    );
    assert_eq!(
        serialport::SerialPort::baud_rate(&firmware)
            .unwrap_or_else(|error| panic!("baud read should pass: {error}")),
        MOCK_PICO_BAUD_RATE
    );

    serialport::SerialPort::set_baud_rate(&mut firmware, 115_200)
        .unwrap_or_else(|error| panic!("baud write should pass: {error}"));
    assert_eq!(
        serialport::SerialPort::baud_rate(&firmware)
            .unwrap_or_else(|error| panic!("baud read after write should pass: {error}")),
        115_200
    );
}
