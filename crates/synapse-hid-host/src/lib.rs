#![allow(unsafe_code)]

use std::time::Duration;

pub mod error;
pub mod handshake;

pub use error::{HidError, HidResult};
pub use handshake::{
    FirmwareIdentity, HandshakeError, IDENTIFY_RESP_LEN, expected_version_triplet,
    parse_and_validate_identify_response, parse_identify_response, validate_expected_major,
};

pub const DEFAULT_BAUD_RATE: u32 = 1_000_000;
pub const DEFAULT_READ_TIMEOUT_MS: u64 = 5;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HidGateway {
    port_name: String,
    baud_rate: u32,
    read_timeout: Duration,
}

impl HidGateway {
    #[must_use]
    pub fn new(port_name: impl Into<String>) -> Self {
        Self {
            port_name: port_name.into(),
            baud_rate: DEFAULT_BAUD_RATE,
            read_timeout: Duration::from_millis(DEFAULT_READ_TIMEOUT_MS),
        }
    }

    #[must_use]
    pub fn port_name(&self) -> &str {
        &self.port_name
    }

    #[must_use]
    pub const fn baud_rate(&self) -> u32 {
        self.baud_rate
    }

    #[must_use]
    pub const fn read_timeout(&self) -> Duration {
        self.read_timeout
    }
}
