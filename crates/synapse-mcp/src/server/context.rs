use super::{
    Arc, CancellationToken, ErrorData, M1State, Mutex, MutexGuard, ProfileActivateParams,
    ProfileActivateResponse, RecordingBackend, RequiredPermissions, SseState, SynapseService,
    activate_profile, authorization_error, error_codes, mcp_error,
};
use rmcp::model::ErrorCode;
use serde_json::json;
use synapse_core::ProfileUseScope;

type M2ActionContext = (
    synapse_action::ActionHandle,
    Option<Arc<RecordingBackend>>,
    Option<CancellationToken>,
);

impl SynapseService {
    pub(super) fn m1_state(&self) -> Result<MutexGuard<'_, M1State>, ErrorData> {
        self.m1_state.lock().map_err(|_err| {
            mcp_error(
                synapse_core::error_codes::OBSERVE_INTERNAL,
                "M1 service state lock poisoned",
            )
        })
    }

    pub(super) fn instructions(&self) -> &'static str {
        let recording_enabled = self
            .m2_state
            .lock()
            .is_ok_and(|state| state.recording_enabled());
        let m3_stub_count = crate::m3::m3_tool_stubs().len();
        let m3_scaffold_ready = self.m3_state.lock().is_ok_and(|state| {
            let _state_readback = (
                state.db_path.as_ref(),
                state.profile_dir.as_ref(),
                state.reflex_disabled,
                state.bearer_token.as_ref(),
                state.permission_grants.names(),
                state.enable_audio,
                state.allow_unknown_profile,
                state.shutdown_cancel.is_cancelled(),
                state.shutdown_reason,
                state
                    .connection_closed_cancel
                    .as_ref()
                    .map(CancellationToken::is_cancelled),
            );
            state.scaffold_ready() && m3_stub_count == 16
        });
        match (recording_enabled, m3_scaffold_ready) {
            (true, true) => {
                "Synapse M1 perception MCP server with M2 action scaffold and M3 scaffold (recording enabled)"
            }
            (false, true) => {
                "Synapse M1 perception MCP server with M2 action scaffold and M3 scaffold"
            }
            (true, false) => {
                "Synapse M1 perception MCP server with M2 action scaffold (recording enabled)"
            }
            (false, false) => "Synapse M1 perception MCP server with M2 action scaffold",
        }
    }

    pub(super) fn require_m3_permissions(
        &self,
        tool: &'static str,
        required: &RequiredPermissions,
    ) -> Result<(), ErrorData> {
        let missing = self
            .m3_state
            .lock()
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    "M3 service state lock poisoned",
                )
            })?
            .permission_grants
            .first_missing(required);
        if let Some(missing) = missing {
            tracing::warn!(
                code = synapse_core::error_codes::SAFETY_PERMISSION_DENIED,
                tool,
                missing_permission = missing.as_str(),
                "tool.permission_denied tool={} missing_permission={}",
                tool,
                missing.as_str()
            );
            return Err(authorization_error(tool, missing));
        }
        Ok(())
    }

    pub(super) fn allow_unknown_profile(&self) -> Result<bool, ErrorData> {
        self.m3_state
            .lock()
            .map(|state| state.allow_unknown_profile)
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    "M3 service state lock poisoned",
                )
            })
    }

    pub(super) fn m2_action_context(&self) -> Result<M2ActionContext, ErrorData> {
        self.m2_state
            .lock()
            .map(|state| {
                (
                    state.emitter_handle.clone(),
                    state.recording.clone(),
                    state.connection_closed_cancel.clone(),
                )
            })
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::OBSERVE_INTERNAL,
                    "M2 service state lock poisoned",
                )
            })
    }

    pub(super) fn ensure_supported_use_allows_action(
        &self,
        tool: &'static str,
    ) -> Result<(), ErrorData> {
        let runtime = self.profile_runtime()?;
        self.ensure_profile_scope_allows_action(&runtime, tool)?;
        let foreground = {
            let state = self.m1_state()?;
            let input = crate::m1::current_input(&state, 1)?;
            drop(state);
            input.foreground
        };
        super::target_policy::ensure_supported_use_allows(&runtime, &foreground, tool)
    }

    fn ensure_profile_scope_allows_action(
        &self,
        runtime: &synapse_profiles::ProfileRuntime,
        tool: &'static str,
    ) -> Result<(), ErrorData> {
        let active_profile_id = runtime
            .active_profile_id()
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        let Some(active_profile_id) = active_profile_id else {
            return Err(profile_action_scope_denied_error(
                tool,
                "no_profile",
                None,
                None,
                "action tools require an active profile before dispatch",
            ));
        };

        let profile = runtime
            .profile(&active_profile_id)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?
            .ok_or_else(|| {
                profile_action_scope_denied_error(
                    tool,
                    "active_profile_missing",
                    Some(&active_profile_id),
                    None,
                    "active profile id does not resolve to a loaded profile",
                )
            })?;

        match profile.use_scope {
            ProfileUseScope::Productivity
            | ProfileUseScope::SinglePlayer
            | ProfileUseScope::OperatorOwnedTest
            | ProfileUseScope::SanctionedResearch => Ok(()),
            ProfileUseScope::Unknown if self.allow_unknown_profile()? => Ok(()),
            ProfileUseScope::Unknown => Err(profile_action_scope_denied_error(
                tool,
                "unknown_scope",
                Some(&profile.id),
                Some(profile.use_scope),
                "active profile has use_scope=\"unknown\"; start with --allow-unknown-profile to dispatch action tools",
            )),
        }
    }

    pub(super) fn m2_release_all_context(
        &self,
    ) -> Result<
        (
            synapse_action::ActionHandle,
            synapse_action::ActionEmitterSnapshotHandle,
        ),
        ErrorData,
    > {
        self.m2_state
            .lock()
            .map(|state| (state.emitter_handle.clone(), state.snapshot_handle.clone()))
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::OBSERVE_INTERNAL,
                    "M2 service state lock poisoned",
                )
            })
    }

    pub(super) fn profile_runtime(
        &self,
    ) -> Result<Arc<synapse_profiles::ProfileRuntime>, ErrorData> {
        self.m3_state
            .lock()
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    "M3 service state lock poisoned",
                )
            })?
            .ensure_profile_runtime()
            .map_err(|error| mcp_error(error.code(), error.to_string()))
    }

    pub(super) fn sse_state(&self) -> Result<SseState, ErrorData> {
        self.m3_state
            .lock()
            .map(|state| state.sse_state.clone())
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    "M3 service state lock poisoned",
                )
            })
    }

    pub(super) fn reflex_runtime(
        &self,
    ) -> Result<Arc<Mutex<synapse_reflex::ReflexRuntime>>, ErrorData> {
        let event_bus = self.sse_state()?.event_bus();
        let (action_handle, _recording, _connection_closed_cancel) = self.m2_action_context()?;
        let mut state = self.m3_state.lock().map_err(|_err| {
            mcp_error(
                synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                "M3 service state lock poisoned",
            )
        })?;
        let runtime = state
            .ensure_reflex_runtime(action_handle, event_bus)
            .map_err(|error| m3_state_error(&error))?;
        drop(state);
        Ok(runtime)
    }

    pub(super) fn ensure_a11y_event_bridge(&self) -> Result<(), ErrorData> {
        let event_bus = self.sse_state()?.event_bus();
        self.m3_state
            .lock()
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    "M3 service state lock poisoned",
                )
            })?
            .ensure_a11y_event_bridge(event_bus)
            .map_err(|error| mcp_error(error.code(), error.to_string()))
    }

    #[allow(clippy::significant_drop_tightening)]
    pub(super) fn activate_profile_locked(
        &self,
        params: &ProfileActivateParams,
        allow_unknown_profile: bool,
    ) -> Result<ProfileActivateResponse, ErrorData> {
        // Keep the M3 mutex held so concurrent activations preserve changed=false idempotency.
        let mut state = self.m3_state.lock().map_err(|_err| {
            mcp_error(
                synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                "M3 service state lock poisoned",
            )
        })?;
        let runtime = state
            .ensure_profile_runtime()
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        activate_profile(&runtime, params, allow_unknown_profile)
    }

    pub(super) fn apply_backend_resolution_for_profile(
        &self,
        profile_id: &str,
    ) -> Result<(), ErrorData> {
        let runtime = self.profile_runtime()?;
        let profile = runtime
            .profile(profile_id)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::PROFILE_NOT_FOUND,
                    format!("profile {profile_id} was not found after activation"),
                )
            })?;
        let policy =
            synapse_action::BackendResolutionPolicy::from_profile_backends(profile.backends);
        let source = format!("profile:{profile_id}");
        self.m2_state
            .lock()
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::OBSERVE_INTERNAL,
                    "M2 service state lock poisoned",
                )
            })?
            .set_backend_resolution(source.clone(), policy)
            .map_err(|error| {
                mcp_error(
                    error_codes::ACTION_BACKEND_UNAVAILABLE,
                    format!("could not update action backend resolution: {error}"),
                )
            })?;
        tracing::info!(
            code = "ACTION_BACKEND_RESOLUTION_UPDATED",
            profile_id,
            source,
            default_backend = ?policy.default_backend,
            keyboard_default = ?policy.keyboard_default,
            mouse_default = ?policy.mouse_default,
            pad_default = ?policy.pad_default,
            keyboard_auto = policy.keyboard_auto_backend().as_str(),
            mouse_auto = policy.mouse_auto_backend().as_str(),
            pad_auto = policy.pad_auto_backend().as_str(),
            release_all_auto = policy.release_all_auto_backend().as_str(),
            "action backend resolution updated from active profile"
        );
        Ok(())
    }

    pub(super) fn ensure_act_type_foreground(
        &self,
        recording: Option<&Arc<RecordingBackend>>,
    ) -> Result<(), ErrorData> {
        let (expected, actual) = {
            let state = self.m1_state()?;
            let Some(expected) = state.last_observed_foreground.clone() else {
                return Ok(());
            };
            let actual = crate::m1::current_input(&state, 1).map(|input| input.foreground);
            drop(state);
            (expected, actual)
        };
        let actual = actual.map_err(|error| {
            mcp_error(
                error_codes::ACTION_FOREGROUND_LOST,
                format!(
                    "act_type could not read current foreground for expected hwnd 0x{:x}: {error}",
                    expected.hwnd
                ),
            )
        })?;
        if actual.hwnd == expected.hwnd {
            return Ok(());
        }

        let recording_event_count_before =
            recording.map_or(0, |recording| recording.events().len());
        let recording_event_count_after = recording.map_or(0, |recording| recording.events().len());
        tracing::warn!(
            code = "M2_ACT_TYPE_FOREGROUND_LOST",
            expected_hwnd = expected.hwnd,
            actual_hwnd = actual.hwnd,
            expected_pid = expected.pid,
            actual_pid = actual.pid,
            expected_title = %expected.window_title,
            actual_title = %actual.window_title,
            recording_event_count_before,
            recording_event_count_after,
            "readback=foreground edge=lost before_hwnd=0x{:x} after_hwnd=0x{:x} code=ACTION_FOREGROUND_LOST recording_events_before={} recording_events_after={}",
            expected.hwnd,
            actual.hwnd,
            recording_event_count_before,
            recording_event_count_after
        );
        Err(mcp_error(
            error_codes::ACTION_FOREGROUND_LOST,
            format!(
                "act_type expected foreground hwnd 0x{:x} ({}) but current foreground is hwnd 0x{:x} ({})",
                expected.hwnd, expected.window_title, actual.hwnd, actual.window_title
            ),
        ))
    }
}

fn profile_action_scope_denied_error(
    tool: &'static str,
    reason: &'static str,
    profile_id: Option<&str>,
    use_scope: Option<ProfileUseScope>,
    detail: &'static str,
) -> ErrorData {
    tracing::warn!(
        code = error_codes::SAFETY_PROFILE_ACTION_DENIED,
        tool,
        reason,
        profile_id,
        use_scope = use_scope.map(profile_use_scope_label),
        detail,
        "profile scope denied action dispatch"
    );
    ErrorData::new(
        ErrorCode(-32099),
        format!("profile scope denied {tool}: {detail}"),
        Some(json!({
            "code": error_codes::SAFETY_PROFILE_ACTION_DENIED,
            "tool": tool,
            "reason": reason,
            "profile_id": profile_id,
            "use_scope": use_scope.map(profile_use_scope_label),
            "detail": detail,
        })),
    )
}

const fn profile_use_scope_label(scope: ProfileUseScope) -> &'static str {
    match scope {
        ProfileUseScope::Productivity => "productivity",
        ProfileUseScope::SinglePlayer => "single_player",
        ProfileUseScope::OperatorOwnedTest => "operator_owned_test",
        ProfileUseScope::SanctionedResearch => "sanctioned_research",
        ProfileUseScope::Unknown => "unknown",
    }
}

fn m3_state_error(error: &anyhow::Error) -> ErrorData {
    if let Some(reflex_error) = error.downcast_ref::<synapse_reflex::ReflexError>() {
        return mcp_error(reflex_error.code(), reflex_error.to_string());
    }
    mcp_error(
        synapse_core::error_codes::TOOL_INTERNAL_ERROR,
        error.to_string(),
    )
}

#[cfg(debug_assertions)]
pub(super) fn maybe_force_panic_during_act(tool: &'static str) {
    if std::env::var("SYNAPSE_MCP_FORCE_PANIC_DURING_ACT").as_deref() == Ok("1") {
        tokio::task::block_in_place(|| panic!("forced panic during {tool}"));
    }
}

#[cfg(not(debug_assertions))]
pub(super) fn maybe_force_panic_during_act(_tool: &'static str) {}

#[cfg(test)]
mod scope_gate_tests {
    use std::{fs, num::NonZeroUsize, path::Path};

    use serde_json::json;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{m2::M2ServiceConfig, m3::M3ServiceConfig, m4::M4ServiceConfig};

    #[test]
    fn action_scope_gate_denies_no_active_profile_before_dispatch() -> anyhow::Result<()> {
        let profiles = TempDir::new()?;
        write_profile(&profiles.path().join("known.toml"), "known", "productivity")?;
        let service = service_with_profiles(profiles.path(), false)?;

        let error = match service.ensure_supported_use_allows_action("act_type") {
            Ok(()) => anyhow::bail!("action tools must fail closed without an active profile"),
            Err(error) => error,
        };

        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("code")),
            Some(&json!(error_codes::SAFETY_PROFILE_ACTION_DENIED))
        );
        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("tool")),
            Some(&json!("act_type"))
        );
        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("reason")),
            Some(&json!("no_profile"))
        );
        Ok(())
    }

    #[test]
    fn action_scope_gate_denies_active_unknown_profile_without_override() -> anyhow::Result<()> {
        let profiles = TempDir::new()?;
        write_profile(&profiles.path().join("unknown.toml"), "unknown", "unknown")?;
        let service = service_with_profiles(profiles.path(), false)?;
        let runtime = service.profile_runtime()?;
        runtime.activate("unknown")?;

        let error = match service.ensure_supported_use_allows_action("act_type") {
            Ok(()) => anyhow::bail!("unknown scope must fail closed without the explicit override"),
            Err(error) => error,
        };

        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("code")),
            Some(&json!(error_codes::SAFETY_PROFILE_ACTION_DENIED))
        );
        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("tool")),
            Some(&json!("act_type"))
        );
        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("reason")),
            Some(&json!("unknown_scope"))
        );
        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("profile_id")),
            Some(&json!("unknown"))
        );
        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("use_scope")),
            Some(&json!("unknown"))
        );
        Ok(())
    }

    fn service_with_profiles(
        profile_dir: &Path,
        allow_unknown_profile: bool,
    ) -> anyhow::Result<SynapseService> {
        let shutdown_cancel = CancellationToken::new();
        let connection_closed_cancel = CancellationToken::new();
        SynapseService::try_with_m2_shutdown_reason_and_m3_config(
            shutdown_cancel,
            "test",
            connection_closed_cancel,
            &M2ServiceConfig::default(),
            M3ServiceConfig::from_cli_parts(
                None,
                Some(profile_dir.to_path_buf()),
                true,
                "127.0.0.1:0".to_owned(),
                NonZeroUsize::new(4)
                    .ok_or_else(|| anyhow::anyhow!("max subscriptions must be nonzero"))?,
                false,
                allow_unknown_profile,
                None,
                false,
                None,
            ),
            M4ServiceConfig::default(),
        )
    }

    fn write_profile(path: &Path, id: &str, use_scope: &str) -> anyhow::Result<()> {
        fs::write(
            path,
            format!(
                r#"
id = "{id}"
label = "{id}"
schema_version = 2
use_scope = "{use_scope}"
mouse_curve_default = "natural"
keyboard_dynamics_default = "natural"

[[matches]]
exe = "{id}.exe"

[detection]
classes_of_interest = ["window"]
confidence_threshold = 0.50
max_detections = 8
"#
            ),
        )?;
        Ok(())
    }
}
