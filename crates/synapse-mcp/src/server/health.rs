use super::{BTreeMap, Health, SubsystemHealth, SynapseService};
use synapse_action::BackendResolutionPolicy;
use synapse_core::Backend;

fn state_lock_health() -> SubsystemHealth {
    SubsystemHealth {
        status: "error".to_owned(),
        detail: Some("M3 service state lock poisoned".to_owned()),
        ..SubsystemHealth::default()
    }
}

fn storage_pressure_status(level: synapse_storage::DiskPressureLevel) -> String {
    match level {
        synapse_storage::DiskPressureLevel::Normal => "ok",
        synapse_storage::DiskPressureLevel::Level1 => "disk_pressure_l1",
        synapse_storage::DiskPressureLevel::Level2 => "disk_pressure_l2",
        synapse_storage::DiskPressureLevel::Level3 => "disk_pressure_l3",
        synapse_storage::DiskPressureLevel::Level4 => "disk_pressure_l4",
    }
    .to_owned()
}

impl SynapseService {
    pub(crate) fn health_payload(&self) -> Health {
        self.health_payload_with_http_sessions(None)
    }

    pub(crate) fn health_payload_with_http_sessions(
        &self,
        active_sessions: Option<usize>,
    ) -> Health {
        let mut subsystems = BTreeMap::new();
        subsystems.insert("storage".to_owned(), self.storage_health());
        subsystems.insert("reflex".to_owned(), self.reflex_health());
        subsystems.insert("profiles".to_owned(), self.profile_health());
        subsystems.insert("action".to_owned(), self.action_health());
        subsystems.insert("hid_host".to_owned(), self.hid_host_health());
        subsystems.insert("audio".to_owned(), self.audio_health());
        subsystems.insert("http".to_owned(), self.http_health(active_sessions));
        let ok = subsystems.values().all(|health| health.status != "error");
        Health {
            ok,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            build: option_env!("VERGEN_GIT_SHA").unwrap_or("dev").to_owned(),
            uptime_s: self.started_at.elapsed().as_secs(),
            subsystems,
        }
    }

    fn storage_health(&self) -> SubsystemHealth {
        match self.m3_state.lock() {
            Ok(state) => {
                let db_path = state
                    .db_path
                    .as_ref()
                    .map(|path| path.display().to_string());
                if let Some(error) = &state.storage_last_error {
                    return SubsystemHealth {
                        status: "error".to_owned(),
                        detail: Some(error.clone()),
                        db_path,
                        ..SubsystemHealth::default()
                    };
                }
                let Some(runtime) = &state.reflex_runtime else {
                    return SubsystemHealth {
                        status: "initializing".to_owned(),
                        detail: Some("storage opens on first reflex tool call".to_owned()),
                        db_path,
                        ..SubsystemHealth::default()
                    };
                };
                match runtime.lock() {
                    Ok(runtime) => match runtime.storage_cf_sizes() {
                        Ok(cf_sizes) => SubsystemHealth {
                            status: storage_pressure_status(runtime.storage_pressure_level()),
                            detail: Some("storage runtime initialized".to_owned()),
                            db_path: Some(runtime.storage_path().display().to_string()),
                            schema_version: Some(runtime.schema_version()),
                            cf_sizes: Some(cf_sizes),
                            ..SubsystemHealth::default()
                        },
                        Err(error) => SubsystemHealth {
                            status: "error".to_owned(),
                            detail: Some(error.to_string()),
                            db_path: Some(runtime.storage_path().display().to_string()),
                            schema_version: Some(runtime.schema_version()),
                            ..SubsystemHealth::default()
                        },
                    },
                    Err(_err) => SubsystemHealth {
                        status: "error".to_owned(),
                        detail: Some(
                            "reflex runtime lock poisoned while reading storage".to_owned(),
                        ),
                        db_path,
                        ..SubsystemHealth::default()
                    },
                }
            }
            Err(_err) => state_lock_health(),
        }
    }

    fn reflex_health(&self) -> SubsystemHealth {
        match self.m3_state.lock() {
            Ok(state) => {
                if let Some(error) = &state.reflex_last_error {
                    return SubsystemHealth {
                        status: "error".to_owned(),
                        detail: Some(error.clone()),
                        ..SubsystemHealth::default()
                    };
                }
                if state.reflex_disabled {
                    return SubsystemHealth {
                        status: "disabled".to_owned(),
                        detail: Some("reflex runtime disabled by operator".to_owned()),
                        active_count: Some(0),
                        ..SubsystemHealth::default()
                    };
                }
                let Some(runtime) = &state.reflex_runtime else {
                    return SubsystemHealth {
                        status: "initializing".to_owned(),
                        detail: Some("reflex runtime starts on first reflex tool call".to_owned()),
                        active_count: Some(0),
                        recursion_clamps_total: Some(0),
                        ..SubsystemHealth::default()
                    };
                };
                match runtime.lock() {
                    Ok(runtime) => match runtime.recursion_clamps_total() {
                        Ok(recursion_clamps_total) => SubsystemHealth {
                            status: if runtime.degraded_latency() {
                                "degraded_latency".to_owned()
                            } else {
                                "ok".to_owned()
                            },
                            detail: Some("reflex runtime initialized".to_owned()),
                            active_count: Some(runtime.active_count()),
                            last_tick_jitter_us: runtime.last_tick_jitter_us(),
                            recursion_clamps_total: Some(recursion_clamps_total),
                            ..SubsystemHealth::default()
                        },
                        Err(error) => SubsystemHealth {
                            status: "error".to_owned(),
                            detail: Some(error.to_string()),
                            active_count: Some(runtime.active_count()),
                            last_tick_jitter_us: runtime.last_tick_jitter_us(),
                            ..SubsystemHealth::default()
                        },
                    },
                    Err(_err) => SubsystemHealth {
                        status: "error".to_owned(),
                        detail: Some("reflex runtime lock poisoned".to_owned()),
                        ..SubsystemHealth::default()
                    },
                }
            }
            Err(_err) => state_lock_health(),
        }
    }

    fn profile_health(&self) -> SubsystemHealth {
        match self.m3_state.lock() {
            Ok(state) => {
                if let Some(error) = &state.profile_last_error {
                    return SubsystemHealth {
                        status: "error".to_owned(),
                        detail: Some(error.clone()),
                        ..SubsystemHealth::default()
                    };
                }
                state.profile_runtime.as_ref().map_or_else(
                    || SubsystemHealth {
                        status: "initializing".to_owned(),
                        detail: Some(
                            "profile runtime initializes on first profile tool call".to_owned(),
                        ),
                        ..SubsystemHealth::default()
                    },
                    |runtime| {
                        let active_profile_id = runtime.active_profile_id();
                        let profiles = runtime.list(true);
                        let last_reload_at = runtime.last_reload_at();
                        match (active_profile_id, profiles, last_reload_at) {
                            (Ok(active_profile_id), Ok(profiles), Ok(last_reload_at)) => {
                                SubsystemHealth {
                                    status: "ok".to_owned(),
                                    detail: Some(format!(
                                        "profile_dir={}",
                                        runtime.profile_dir().display()
                                    )),
                                    active_profile_id,
                                    profile_count: Some(profiles.len()),
                                    last_reload_at,
                                    ..SubsystemHealth::default()
                                }
                            }
                            (active_profile_id, profiles, last_reload_at) => {
                                let detail = active_profile_id
                                    .err()
                                    .map(|error| error.to_string())
                                    .or_else(|| profiles.err().map(|error| error.to_string()))
                                    .or_else(|| last_reload_at.err().map(|error| error.to_string()))
                                    .unwrap_or_else(|| "profile runtime error".to_owned());
                                SubsystemHealth {
                                    status: "error".to_owned(),
                                    detail: Some(detail),
                                    ..SubsystemHealth::default()
                                }
                            }
                        }
                    },
                )
            }
            Err(_err) => state_lock_health(),
        }
    }

    fn action_health(&self) -> SubsystemHealth {
        match self.m2_state.lock() {
            Ok(state) => match state.backend_resolution_readback() {
                Ok((source, policy)) => {
                    let emitter_available = state.emitter_available();
                    SubsystemHealth {
                        status: if emitter_available { "ok" } else { "error" }.to_owned(),
                        detail: Some(format!(
                            "emitter_available={} recording_enabled={} hardware_hid={} allow_shell_patterns={} allow_launch_patterns={}",
                            emitter_available,
                            state.recording_enabled(),
                            state.hardware_hid().unwrap_or("disabled"),
                            self.m4_config.allow_shell_count(),
                            self.m4_config.allow_launch_count()
                        )),
                        device_name: state.hardware_hid().map(str::to_owned),
                        backend_resolution: Some(backend_resolution_health(source, policy)),
                        ..SubsystemHealth::default()
                    }
                }
                Err(error) => SubsystemHealth {
                    status: "error".to_owned(),
                    detail: Some(error),
                    ..SubsystemHealth::default()
                },
            },
            Err(_err) => SubsystemHealth {
                status: "error".to_owned(),
                detail: Some("M2 service state lock poisoned".to_owned()),
                ..SubsystemHealth::default()
            },
        }
    }

    fn hid_host_health(&self) -> SubsystemHealth {
        match self.m2_state.lock() {
            Ok(state) => {
                let Some(target) = state.hardware_hid() else {
                    return SubsystemHealth {
                        status: "disabled".to_owned(),
                        detail: Some(
                            "hardware HID disabled; start with --hardware-hid <port|auto> or SYNAPSE_HARDWARE_HID".to_owned(),
                        ),
                        ..SubsystemHealth::default()
                    };
                };
                let emitter_available = state.emitter_available();
                SubsystemHealth {
                    status: if emitter_available { "ok" } else { "error" }.to_owned(),
                    detail: Some(format!(
                        "hardware_hid={target} emitter_available={emitter_available}"
                    )),
                    device_name: Some(target.to_owned()),
                    ..SubsystemHealth::default()
                }
            }
            Err(_err) => SubsystemHealth {
                status: "error".to_owned(),
                detail: Some("M2 service state lock poisoned while reading HID host".to_owned()),
                ..SubsystemHealth::default()
            },
        }
    }

    fn audio_health(&self) -> SubsystemHealth {
        match self.m3_state.lock() {
            Ok(state) => {
                if let Some(error) = &state.audio_last_error {
                    return SubsystemHealth {
                        status: "error".to_owned(),
                        detail: Some(error.clone()),
                        ..SubsystemHealth::default()
                    };
                }
                if !state.enable_audio {
                    return SubsystemHealth {
                        status: "disabled".to_owned(),
                        detail: Some("audio is disabled; start with --enable-audio".to_owned()),
                        ring_buffer_seconds: Some(synapse_audio::DEFAULT_RING_SECONDS),
                        stt_model_loaded: Some(false),
                        ..SubsystemHealth::default()
                    };
                }
                let Some(runtime) = &state.audio_runtime else {
                    return SubsystemHealth {
                        status: "initializing".to_owned(),
                        detail: Some(
                            "audio runtime initializes on buffered audio or transcription requests"
                                .to_owned(),
                        ),
                        ring_buffer_seconds: Some(synapse_audio::DEFAULT_RING_SECONDS),
                        stt_model_loaded: Some(false),
                        ..SubsystemHealth::default()
                    };
                };
                let loopback_status = runtime.loopback_status();
                let status = if loopback_status.last_error_code.is_some() {
                    "error"
                } else {
                    "ok"
                };
                SubsystemHealth {
                    status: status.to_owned(),
                    detail: Some(loopback_status.last_error_code.map_or_else(
                        || {
                            if loopback_status.running {
                                "audio loopback running".to_owned()
                            } else {
                                "audio runtime initialized; loopback disabled".to_owned()
                            }
                        },
                        |code| format!("audio loopback error: {code}"),
                    )),
                    ring_buffer_seconds: Some(runtime.config().ring_seconds),
                    stt_model_loaded: Some(runtime.stt_model_loaded()),
                    ..SubsystemHealth::default()
                }
            }
            Err(_err) => state_lock_health(),
        }
    }

    fn http_health(&self, active_sessions: Option<usize>) -> SubsystemHealth {
        match self.m3_state.lock() {
            Ok(state) => {
                if state.shutdown_reason == "http" {
                    SubsystemHealth {
                        status: "ok".to_owned(),
                        detail: Some("HTTP transport initialized".to_owned()),
                        bind_addr: Some(state.bind.clone()),
                        active_sessions,
                        sse_subscribers: Some(state.sse_state.active_subscription_count()),
                        ..SubsystemHealth::default()
                    }
                } else {
                    SubsystemHealth {
                        status: "disabled".to_owned(),
                        detail: Some("HTTP transport disabled in stdio mode".to_owned()),
                        bind_addr: Some(state.bind.clone()),
                        active_sessions: Some(0),
                        sse_subscribers: Some(state.sse_state.active_subscription_count()),
                        ..SubsystemHealth::default()
                    }
                }
            }
            Err(_err) => state_lock_health(),
        }
    }
}

fn backend_resolution_health(
    source: String,
    policy: BackendResolutionPolicy,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("source".to_owned(), source),
        (
            "default_backend".to_owned(),
            backend_config_name(policy.default_backend).to_owned(),
        ),
        (
            "keyboard_default".to_owned(),
            backend_config_name(policy.keyboard_default).to_owned(),
        ),
        (
            "mouse_default".to_owned(),
            backend_config_name(policy.mouse_default).to_owned(),
        ),
        (
            "pad_default".to_owned(),
            backend_config_name(policy.pad_default).to_owned(),
        ),
        (
            "keyboard_auto".to_owned(),
            policy.keyboard_auto_backend().as_str().to_owned(),
        ),
        (
            "mouse_auto".to_owned(),
            policy.mouse_auto_backend().as_str().to_owned(),
        ),
        (
            "pad_auto".to_owned(),
            policy.pad_auto_backend().as_str().to_owned(),
        ),
        (
            "release_all_auto".to_owned(),
            policy.release_all_auto_backend().as_str().to_owned(),
        ),
    ])
}

const fn backend_config_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Software => "software",
        Backend::Vigem => "vigem",
        Backend::Hardware => "hardware",
        Backend::Auto => "auto",
    }
}
