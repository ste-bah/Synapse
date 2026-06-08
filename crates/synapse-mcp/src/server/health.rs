use super::{BTreeMap, Health, SubsystemHealth, SynapseService};
use sha2::{Digest as _, Sha256};
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
        subsystems.insert("perception".to_owned(), self.perception_health());
        subsystems.insert("action".to_owned(), self.action_health());
        subsystems.insert("audio".to_owned(), self.audio_health());
        subsystems.insert("http".to_owned(), self.http_health(active_sessions));
        subsystems.insert("daemon_drain".to_owned(), self.daemon_drain_health());
        subsystems.insert(
            "daemon_lifecycle".to_owned(),
            crate::daemon_lifecycle::health_subsystem(),
        );
        let tool_surface = self.tool_surface_fingerprint();
        if let Some(error) = &tool_surface.error {
            subsystems.insert(
                "tool_surface".to_owned(),
                SubsystemHealth {
                    status: "error".to_owned(),
                    detail: Some(error.clone()),
                    ..SubsystemHealth::default()
                },
            );
        }
        let ok = subsystems.values().all(|health| health.status != "error");
        Health {
            ok,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            build: option_env!("VERGEN_GIT_SHA").unwrap_or("dev").to_owned(),
            pid: std::process::id(),
            uptime_s: self.started_at.elapsed().as_secs(),
            tool_count: tool_surface.names.len(),
            tool_surface_sha256: tool_surface.sha256,
            tool_names: tool_surface.names,
            subsystems,
        }
    }

    fn daemon_drain_health(&self) -> SubsystemHealth {
        let snapshot = self.drain_state_handle().snapshot();
        let status = if snapshot.state_error.is_some() {
            "error"
        } else if snapshot.draining {
            "draining"
        } else {
            "ok"
        };
        let detail = if let Some(error) = snapshot.state_error {
            error
        } else if snapshot.draining {
            format!(
                "reason_code={} source={} started_at_unix_ms={}",
                snapshot.reason_code.unwrap_or("unknown"),
                snapshot.source.unwrap_or("unknown"),
                snapshot.started_at_unix_ms.unwrap_or_default()
            )
        } else {
            "daemon accepting work".to_owned()
        };
        SubsystemHealth {
            status: status.to_owned(),
            detail: Some(detail),
            ..SubsystemHealth::default()
        }
    }

    fn tool_surface_fingerprint(&self) -> ToolSurfaceFingerprint {
        let mut tools = super::schema_sanitize::sanitize_tools(self.tool_router.list_all());
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        let names = tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        let canonical = serde_json::json!({
            "mcp_surface": "tools/list",
            "tools": tools,
        });
        let bytes = match serde_json::to_vec(&canonical) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::error!(
                    code = "MCP_TOOL_SURFACE_FINGERPRINT_SERIALIZE_FAILED",
                    %error,
                    "sanitized MCP tool surface failed to serialize for health fingerprinting"
                );
                return ToolSurfaceFingerprint {
                    names,
                    sha256: "TOOL_SURFACE_FINGERPRINT_ERROR".to_owned(),
                    error: Some(format!(
                        "sanitized MCP tool surface failed to serialize for health fingerprinting: {error}"
                    )),
                };
            }
        };
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        ToolSurfaceFingerprint {
            names,
            sha256: hex_lower(&hasher.finalize()),
            error: None,
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
                    if state.db.is_some() {
                        return SubsystemHealth {
                            status: "ok".to_owned(),
                            detail: Some(
                                "storage opened at daemon startup (reflex runtime idle)".to_owned(),
                            ),
                            db_path,
                            schema_version: Some(synapse_core::SCHEMA_VERSION),
                            ..SubsystemHealth::default()
                        };
                    }
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
                            sample_count: Some(runtime.sample_count()),
                            sample_limit: Some(runtime.sample_limit()),
                            last_tick_jitter_us: runtime.last_tick_jitter_us(),
                            p99_tick_jitter_us: runtime.p99_tick_jitter_us(),
                            late_tick_count: Some(runtime.late_tick_count()),
                            degraded_tick_count: Some(runtime.degraded_tick_count()),
                            recursion_clamps_total: Some(recursion_clamps_total),
                            ..SubsystemHealth::default()
                        },
                        Err(error) => SubsystemHealth {
                            status: "error".to_owned(),
                            detail: Some(error.to_string()),
                            active_count: Some(runtime.active_count()),
                            sample_count: Some(runtime.sample_count()),
                            sample_limit: Some(runtime.sample_limit()),
                            last_tick_jitter_us: runtime.last_tick_jitter_us(),
                            p99_tick_jitter_us: runtime.p99_tick_jitter_us(),
                            late_tick_count: Some(runtime.late_tick_count()),
                            degraded_tick_count: Some(runtime.degraded_tick_count()),
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

    fn perception_health(&self) -> SubsystemHealth {
        match self.m1_state.lock() {
            Ok(state) => SubsystemHealth {
                status: "ok".to_owned(),
                detail: Some("perception runtime initialized".to_owned()),
                perception_mode: Some(state.perception_mode),
                capture_config: Some(state.active_capture_config.clone()),
                capture_runtime: Some(state.capture_runtime_readback()),
                ..SubsystemHealth::default()
            },
            Err(_err) => SubsystemHealth {
                status: "error".to_owned(),
                detail: Some("M1 service state lock poisoned".to_owned()),
                ..SubsystemHealth::default()
            },
        }
    }

    fn action_health(&self) -> SubsystemHealth {
        match self.m2_state.lock() {
            Ok(state) => match state.backend_resolution_readback() {
                Ok((source, policy)) => {
                    let emitter_available = state.emitter_available();
                    let operator_hotkey = synapse_action::operator_hotkey_status().label();
                    let allow_shell = if self.m4_config.allow_shell_any() {
                        "any".to_owned()
                    } else {
                        self.m4_config.allow_shell_count().to_string()
                    };
                    let allow_launch = if self.m4_config.allow_launch_any() {
                        "any".to_owned()
                    } else {
                        self.m4_config.allow_launch_count().to_string()
                    };
                    let lease = synapse_action::lease::status();
                    let lease_detail = lease.owner_session_id.as_deref().map_or_else(
                        || "input_lease_held=false".to_owned(),
                        |owner| {
                            format!(
                                "input_lease_held=true input_lease_owner={owner} input_lease_expires_in_ms={}",
                                lease.expires_in_ms.unwrap_or(0)
                            )
                        },
                    );
                    SubsystemHealth {
                        status: if emitter_available { "ok" } else { "error" }.to_owned(),
                        detail: Some(format!(
                            "emitter_available={} recording_enabled={} operator_hotkey={} allow_shell_patterns={} allow_launch_patterns={} {}",
                            emitter_available,
                            state.recording_enabled(),
                            operator_hotkey,
                            allow_shell,
                            allow_launch,
                            lease_detail
                        )),
                        backend_resolution: Some(backend_resolution_health(source, policy)),
                        run_shell_inline_await_limit_ms: Some(
                            self.m4_config.run_shell_inline_await_limit_ms(),
                        ),
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

struct ToolSurfaceFingerprint {
    names: Vec<String>,
    sha256: String,
    error: Option<String>,
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
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
