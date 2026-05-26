use std::sync::Arc;

use synapse_hid_host::{HidError, HidGateway, connect_auto};

use crate::{
    ActionBackend, HardwareBackend, HardwareUnavailableBackend, ResolvedBackend, VigemBackend,
    backend::software::SoftwareBackend,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HardwareHidConfig {
    Disabled,
    Auto,
    Port(String),
}

impl HardwareHidConfig {
    #[must_use]
    pub fn from_setting(value: Option<&str>) -> Self {
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            return Self::Disabled;
        };
        if value.eq_ignore_ascii_case("auto") {
            Self::Auto
        } else {
            Self::Port(value.to_owned())
        }
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        !matches!(self, Self::Disabled)
    }
}

pub struct Backends {
    software: Arc<dyn ActionBackend>,
    vigem: Arc<dyn ActionBackend>,
    hardware: Arc<dyn ActionBackend>,
    hardware_release_enabled: bool,
}

impl Backends {
    #[must_use]
    pub fn production() -> Self {
        Self {
            software: Arc::new(SoftwareBackend::new()),
            vigem: Arc::new(VigemBackend::new()),
            hardware: Arc::new(HardwareUnavailableBackend::new()),
            hardware_release_enabled: false,
        }
    }

    /// Builds production backends and replaces the fail-closed hardware slot
    /// only after the HID gateway connects and completes IDENTIFY.
    ///
    /// # Errors
    ///
    /// Returns the concrete HID connection/handshake error when hardware HID
    /// was requested but could not be made ready.
    pub fn production_with_hardware_hid(config: &HardwareHidConfig) -> Result<Self, HidError> {
        match config {
            HardwareHidConfig::Disabled => Ok(Self::production()),
            HardwareHidConfig::Auto => connect_auto().map(Self::production_with_hardware_gateway),
            HardwareHidConfig::Port(port_name) => {
                HidGateway::connect(port_name.clone()).map(Self::production_with_hardware_gateway)
            }
        }
    }

    #[must_use]
    pub fn production_with_hardware_gateway(gateway: HidGateway) -> Self {
        Self::production_with_hardware_backend(Arc::new(HardwareBackend::new(gateway)))
    }

    #[must_use]
    pub fn production_with_hardware_backend(hardware: Arc<dyn ActionBackend>) -> Self {
        Self {
            software: Arc::new(SoftwareBackend::new()),
            vigem: Arc::new(VigemBackend::new()),
            hardware,
            hardware_release_enabled: true,
        }
    }

    #[must_use]
    pub fn all_routed_to(backend: Arc<dyn ActionBackend>) -> Self {
        Self {
            software: Arc::clone(&backend),
            vigem: Arc::clone(&backend),
            hardware: backend,
            hardware_release_enabled: false,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(super) fn from_parts(
        software: Arc<dyn ActionBackend>,
        vigem: Arc<dyn ActionBackend>,
        hardware: Arc<dyn ActionBackend>,
        hardware_release_enabled: bool,
    ) -> Self {
        Self {
            software,
            vigem,
            hardware,
            hardware_release_enabled,
        }
    }

    pub(super) fn pick(&self, resolved: ResolvedBackend) -> Arc<dyn ActionBackend> {
        match resolved {
            ResolvedBackend::Software => Arc::clone(&self.software),
            ResolvedBackend::Vigem => Arc::clone(&self.vigem),
            ResolvedBackend::Hardware => Arc::clone(&self.hardware),
        }
    }

    pub(super) fn pick_vigem_for_release(&self) -> Arc<dyn ActionBackend> {
        Arc::clone(&self.vigem)
    }

    pub(super) fn pick_hardware_for_release(&self) -> Arc<dyn ActionBackend> {
        Arc::clone(&self.hardware)
    }

    pub(super) const fn hardware_release_enabled(&self) -> bool {
        self.hardware_release_enabled
    }
}

impl std::fmt::Debug for Backends {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backends").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use synapse_core::{Action, Backend, Key, KeyCode};

    use super::{Backends, HardwareHidConfig};
    use crate::{ActionBackend, EmitState, RecordedInput, RecordingBackend, ResolvedBackend};

    #[test]
    fn hardware_hid_config_parses_disabled_auto_and_port_edges() {
        let cases = [
            ("absent", None, HardwareHidConfig::Disabled),
            ("empty", Some(""), HardwareHidConfig::Disabled),
            ("whitespace", Some("   "), HardwareHidConfig::Disabled),
            ("auto_lower", Some("auto"), HardwareHidConfig::Auto),
            ("auto_upper", Some("AUTO"), HardwareHidConfig::Auto),
            (
                "port_trimmed",
                Some("  COM7  "),
                HardwareHidConfig::Port("COM7".to_owned()),
            ),
        ];

        for (edge, before, expected) in cases {
            let after = HardwareHidConfig::from_setting(before);
            println!("readback=hardware_hid_config edge={edge} before={before:?} after={after:?}");
            assert_eq!(after, expected);
        }
    }

    #[test]
    fn configured_hardware_backend_receives_hardware_route() {
        let hardware = Arc::new(RecordingBackend::new());
        let hardware_backend = Arc::clone(&hardware) as Arc<dyn ActionBackend>;
        let backends = Backends::production_with_hardware_backend(hardware_backend);
        let picked = backends.pick(ResolvedBackend::Hardware);
        let mut state = EmitState::new();
        let before_events = hardware.events();
        let before_snapshot = state.snapshot();

        picked
            .execute(
                &Action::KeyDown {
                    key: Key {
                        code: KeyCode::Named {
                            value: "hardware-route".to_owned(),
                        },
                        use_scancode: false,
                    },
                    backend: Backend::Hardware,
                },
                &mut state,
            )
            .unwrap_or_else(|error| panic!("configured hardware backend should execute: {error}"));

        let after_events = hardware.events();
        let after_snapshot = state.snapshot();
        println!(
            "readback=hardware_route before_events={} before_state={before_snapshot:?} after_events={} after_state={after_snapshot:?}",
            before_events.len(),
            after_events.len()
        );
        assert_eq!(before_events.len(), 0);
        assert_eq!(
            after_events,
            vec![RecordedInput::KeyDown {
                key: Key {
                    code: KeyCode::Named {
                        value: "hardware-route".to_owned(),
                    },
                    use_scancode: false,
                }
            }]
        );
        assert_eq!(after_snapshot.held_keys.len(), 1);
    }
}
