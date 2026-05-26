use synapse_core::error_codes;

use crate::handshake::HandshakeError;

pub type HidResult<T> = Result<T, HidError>;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum HidError {
    #[error("HID port not found: {port_name}")]
    PortNotFound { port_name: String },
    #[error("HID port open failed for {port_name}: {detail}")]
    PortOpenFailed { port_name: String, detail: String },
    #[error("HID protocol handshake failed: {detail}")]
    ProtocolHandshakeFailed { detail: String },
    #[error("HID firmware major version {actual} did not match expected {expected}")]
    FirmwareVersionMismatch { expected: u8, actual: u8 },
    #[error("HID command rejected: seq={seq}, command=0x{command:02X}, reason=0x{reason:02X}")]
    CommandRejected { seq: u32, command: u8, reason: u8 },
    #[error("HID link timeout after {timeout_ms} ms while {operation}")]
    LinkTimeout {
        operation: &'static str,
        timeout_ms: u64,
    },
}

impl HidError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::PortNotFound { .. } => error_codes::HID_PORT_NOT_FOUND,
            Self::PortOpenFailed { .. } => error_codes::HID_PORT_OPEN_FAILED,
            Self::ProtocolHandshakeFailed { .. } => error_codes::HID_PROTOCOL_HANDSHAKE_FAILED,
            Self::FirmwareVersionMismatch { .. } => error_codes::HID_FIRMWARE_VERSION_MISMATCH,
            Self::CommandRejected { .. } => error_codes::HID_COMMAND_REJECTED,
            Self::LinkTimeout { .. } => error_codes::HID_LINK_TIMEOUT,
        }
    }
}

impl From<HandshakeError> for HidError {
    fn from(error: HandshakeError) -> Self {
        match error {
            HandshakeError::InvalidIdentifyPayloadLength { .. } => Self::ProtocolHandshakeFailed {
                detail: error.to_string(),
            },
            HandshakeError::FirmwareVersionMismatch { expected, actual } => {
                Self::FirmwareVersionMismatch { expected, actual }
            }
        }
    }
}
