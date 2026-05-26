use anyhow::Context;
use synapse_action::{Backends, HardwareHidConfig};

pub(super) const HARDWARE_HID_ENV: &str = "SYNAPSE_HARDWARE_HID";
pub(super) const RECORDING_BACKEND_ENV: &str = "SYNAPSE_MCP_RECORDING_BACKEND";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M2ServiceConfig {
    pub recording_backend: Option<String>,
    pub hardware_hid: Option<String>,
}

impl M2ServiceConfig {
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            recording_backend: std::env::var(RECORDING_BACKEND_ENV).ok(),
            hardware_hid: std::env::var(HARDWARE_HID_ENV).ok(),
        }
    }

    #[must_use]
    pub fn from_cli_parts(hardware_hid: Option<String>) -> Self {
        Self {
            recording_backend: std::env::var(RECORDING_BACKEND_ENV).ok(),
            hardware_hid,
        }
    }

    #[must_use]
    pub fn hardware_hid_readback(&self) -> Option<String> {
        self.hardware_hid
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }

    pub fn action_backends(&self) -> anyhow::Result<Option<Backends>> {
        let config = HardwareHidConfig::from_setting(self.hardware_hid.as_deref());
        if !config.enabled() {
            return Ok(None);
        }
        Backends::production_with_hardware_hid(&config)
            .map(Some)
            .map_err(|error| anyhow::anyhow!("{}: {}", error.code(), error))
            .with_context(|| format!("initialize hardware HID backend from {config:?}"))
    }
}

#[cfg(test)]
mod tests {
    use synapse_action::HardwareHidConfig;

    use super::M2ServiceConfig;

    #[test]
    fn cli_config_preserves_hardware_hid_source_for_backend_selection() {
        let cases = [
            ("absent", None, None, HardwareHidConfig::Disabled),
            (
                "empty",
                Some(String::new()),
                None,
                HardwareHidConfig::Disabled,
            ),
            (
                "auto",
                Some("auto".to_owned()),
                Some("auto".to_owned()),
                HardwareHidConfig::Auto,
            ),
            (
                "port",
                Some("  COM7  ".to_owned()),
                Some("COM7".to_owned()),
                HardwareHidConfig::Port("COM7".to_owned()),
            ),
        ];

        for (edge, before, expected_readback, expected_config) in cases {
            let config = M2ServiceConfig {
                recording_backend: None,
                hardware_hid: before.clone(),
            };
            let after_readback = config.hardware_hid_readback();
            let after_config = HardwareHidConfig::from_setting(config.hardware_hid.as_deref());
            println!(
                "readback=m2_hardware_hid_config edge={edge} before={before:?} after_readback={after_readback:?} after_config={after_config:?}"
            );
            assert_eq!(after_readback, expected_readback);
            assert_eq!(after_config, expected_config);
        }
    }
}
