use std::time::Duration;

use synapse_core::{EXPECTED_FW_MAJOR, error_codes};
use synapse_hid_host::{
    DEFAULT_BAUD_RATE, DEFAULT_READ_TIMEOUT_MS, HandshakeError, HidError, HidGateway,
    IDENTIFY_RESP_LEN,
};

#[test]
fn gateway_scaffold_records_port_and_defaults() {
    let gateway = HidGateway::new("COM7");

    assert_eq!(gateway.port_name(), "COM7");
    assert_eq!(gateway.baud_rate(), DEFAULT_BAUD_RATE);
    assert_eq!(
        gateway.read_timeout(),
        Duration::from_millis(DEFAULT_READ_TIMEOUT_MS)
    );
}

#[test]
fn hid_error_code_mapping_covers_m4_contract() {
    let cases = [
        (
            HidError::PortNotFound {
                port_name: "COM404".to_owned(),
            },
            error_codes::HID_PORT_NOT_FOUND,
        ),
        (
            HidError::PortOpenFailed {
                port_name: "COM7".to_owned(),
                detail: "access denied".to_owned(),
            },
            error_codes::HID_PORT_OPEN_FAILED,
        ),
        (
            HidError::ProtocolHandshakeFailed {
                detail: "short identify payload".to_owned(),
            },
            error_codes::HID_PROTOCOL_HANDSHAKE_FAILED,
        ),
        (
            HidError::FirmwareVersionMismatch {
                expected: EXPECTED_FW_MAJOR,
                actual: EXPECTED_FW_MAJOR.wrapping_add(1),
            },
            error_codes::HID_FIRMWARE_VERSION_MISMATCH,
        ),
        (
            HidError::CommandRejected {
                seq: 7,
                command: 0x10,
                reason: 0x04,
            },
            error_codes::HID_COMMAND_REJECTED,
        ),
        (
            HidError::LinkTimeout {
                operation: "waiting for ACK",
                timeout_ms: DEFAULT_READ_TIMEOUT_MS,
            },
            error_codes::HID_LINK_TIMEOUT,
        ),
    ];

    for (error, code) in cases {
        assert_eq!(error.code(), code);
    }
}

#[test]
fn handshake_errors_promote_to_hid_errors() {
    let malformed = HidError::from(HandshakeError::InvalidIdentifyPayloadLength {
        actual: IDENTIFY_RESP_LEN - 1,
        expected: IDENTIFY_RESP_LEN,
    });
    assert_eq!(malformed.code(), error_codes::HID_PROTOCOL_HANDSHAKE_FAILED);

    let mismatched = HidError::from(HandshakeError::FirmwareVersionMismatch {
        expected: EXPECTED_FW_MAJOR,
        actual: EXPECTED_FW_MAJOR.wrapping_add(1),
    });
    assert_eq!(
        mismatched,
        HidError::FirmwareVersionMismatch {
            expected: EXPECTED_FW_MAJOR,
            actual: EXPECTED_FW_MAJOR.wrapping_add(1),
        }
    );
    assert_eq!(
        mismatched.code(),
        error_codes::HID_FIRMWARE_VERSION_MISMATCH
    );
}
