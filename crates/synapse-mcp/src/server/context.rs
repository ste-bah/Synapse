use super::{
    Arc, CancellationToken, ErrorData, M1State, Mutex, MutexGuard, ProfileActivateParams,
    ProfileActivateResponse, RecordingBackend, RequiredPermissions, SseState, SynapseService,
    action_preflight::{ActionPreflightReadback, attach_action_preflight_to_error},
    activate_profile, authorization_error, error_codes, mcp_error,
};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use rmcp::model::ErrorCode;
use serde_json::json;
use synapse_core::{
    AccessibleNode, Action, ElementId, Event, EventSource, FocusedElement, ForegroundContext,
    ProfileUseScope, ReflexId,
};
use synapse_profiles::ForegroundProfileTransition;
use synapse_reflex::{
    AimTrackTargetSnapshot, AimTrackTargetSource, EventBus, ReflexActionGate,
    ReflexActionGateHandle, ReflexActionPermissionDenied, ResolvedElementBox,
};

type M2ActionContext = (
    synapse_action::ActionHandle,
    Option<Arc<RecordingBackend>>,
    Option<CancellationToken>,
);
type M2ReleaseAllContext = (
    synapse_action::ActionHandle,
    synapse_action::ActionEmitterSnapshotHandle,
    Option<Arc<Mutex<synapse_reflex::ReflexRuntime>>>,
);

const PROFILE_CHANGED_KIND: &str = "profile-changed";
const SCOPE_TRANSITIONED_KIND: &str = "scope-transitioned";
// Match observe's default shallow tree so targets selected from an observation
// can be resolved on scheduler ticks without requiring a deep UIA walk.
const AIM_TRACK_TARGET_SOURCE_DEPTH: u32 = 2;
static NEXT_PROFILE_EVENT_SEQ: AtomicU64 = AtomicU64::new(1);

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
        let m3_has_tools = !crate::m3::m3_tool_stubs().is_empty();
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
            state.scaffold_ready() && m3_has_tools
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
    ) -> Result<ActionPreflightReadback, ErrorData> {
        let runtime = self.profile_runtime()?;
        let active_profile_id_before = runtime
            .active_profile_id()
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        let initial_foreground = self.current_audit_foreground()?;
        let (foreground, preflight) = self.preflight_action_foreground(
            tool,
            &runtime,
            active_profile_id_before,
            initial_foreground,
        )?;
        let transition = self
            .reevaluate_profile_for_foreground(&foreground)
            .map_err(|error| attach_action_preflight_to_error(&error, &preflight))?;
        if let Some(profile_id) = transition.active_profile_id.as_deref() {
            self.apply_backend_resolution_for_profile(profile_id)
                .map_err(|error| attach_action_preflight_to_error(&error, &preflight))?;
        }
        ensure_profile_scope_allows_action(&runtime, tool, self.allow_unknown_profile()?)
            .map_err(|error| attach_action_preflight_to_error(&error, &preflight))?;
        super::target_policy::ensure_supported_use_allows(&runtime, &foreground, tool)
            .map_err(|error| attach_action_preflight_to_error(&error, &preflight))?;
        let preflight = self.ensure_everquest_live_ui_context_for_action(tool, preflight)?;
        Ok(preflight)
    }

    pub(super) fn m2_release_all_context(&self) -> Result<M2ReleaseAllContext, ErrorData> {
        let (handle, snapshot_handle) = self
            .m2_state
            .lock()
            .map(|state| (state.emitter_handle.clone(), state.snapshot_handle.clone()))
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::OBSERVE_INTERNAL,
                    "M2 service state lock poisoned",
                )
            })?;
        let reflex_runtime = self
            .m3_state
            .lock()
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    "M3 service state lock poisoned",
                )
            })?
            .reflex_runtime
            .clone();
        Ok((handle, snapshot_handle, reflex_runtime))
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
        self.install_aim_track_target_source(&runtime)?;
        Ok(runtime)
    }

    fn install_aim_track_target_source(
        &self,
        runtime: &Arc<Mutex<synapse_reflex::ReflexRuntime>>,
    ) -> Result<(), ErrorData> {
        let target_source = Arc::new(M1AimTrackTargetSource {
            m1_state: Arc::clone(&self.m1_state),
        });
        runtime
            .lock()
            .map_err(|_error| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while setting aim_track target source",
                )
            })?
            .set_aim_track_target_source(Some(target_source));
        Ok(())
    }

    pub(super) fn install_reflex_action_gate(
        &self,
        runtime: &Arc<Mutex<synapse_reflex::ReflexRuntime>>,
    ) -> Result<(), ErrorData> {
        let gate = self.reflex_action_gate()?;
        runtime
            .lock()
            .map_err(|_error| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while setting action gate",
                )
            })?
            .set_action_gate(Some(gate));
        Ok(())
    }

    pub(super) fn reflex_action_gate(&self) -> Result<ReflexActionGateHandle, ErrorData> {
        Ok(Arc::new(ReflexScopeActionGate {
            profile_runtime: self.profile_runtime()?,
            m1_state: Arc::clone(&self.m1_state),
            allow_unknown_profile: self.allow_unknown_profile()?,
            event_bus: self.sse_state()?.event_bus(),
        }))
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

    pub(super) fn reevaluate_profile_for_foreground(
        &self,
        foreground: &ForegroundContext,
    ) -> Result<ForegroundProfileTransition, ErrorData> {
        let runtime = self.profile_runtime()?;
        let transition = runtime
            .reevaluate_foreground(&foreground_window(foreground))
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        let event_bus = self.sse_state()?.event_bus();
        publish_profile_transition_events(&event_bus, &transition, foreground);
        Ok(transition)
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

fn ensure_profile_scope_allows_action(
    runtime: &synapse_profiles::ProfileRuntime,
    tool: &'static str,
    allow_unknown_profile: bool,
) -> Result<(), ErrorData> {
    let active_profile_id = runtime
        .active_profile_id()
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    let Some(active_profile_id) = active_profile_id else {
        // Default posture (allow_unknown_profile): general Windows
        // computer-control, so an unprofiled foreground is still actionable.
        // Functional safety (panic hotkey, release-all, rate limits, focus
        // stabilization) is unaffected by this allowance.
        if allow_unknown_profile {
            return Ok(());
        }
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
        ProfileUseScope::Unknown if allow_unknown_profile => Ok(()),
        ProfileUseScope::Unknown => Err(profile_action_scope_denied_error(
            tool,
            "unknown_scope",
            Some(&profile.id),
            Some(profile.use_scope),
            "active profile has use_scope=\"unknown\"; start with --allow-unknown-profile to dispatch action tools",
        )),
    }
}

struct ReflexScopeActionGate {
    profile_runtime: Arc<synapse_profiles::ProfileRuntime>,
    m1_state: super::SharedM1State,
    allow_unknown_profile: bool,
    event_bus: EventBus,
}

impl ReflexActionGate for ReflexScopeActionGate {
    fn ensure_action_allowed(
        &self,
        _reflex_id: &ReflexId,
        _action: &Action,
    ) -> Result<(), ReflexActionPermissionDenied> {
        const TOOL: &str = "reflex_dispatch";
        (|| {
            let foreground = current_reflex_action_foreground(&self.m1_state)?;
            let transition = self
                .profile_runtime
                .reevaluate_foreground(&foreground_window(&foreground))
                .map_err(|error| mcp_error(error.code(), error.to_string()))?;
            publish_profile_transition_events(&self.event_bus, &transition, &foreground);
            ensure_profile_scope_allows_action(
                &self.profile_runtime,
                TOOL,
                self.allow_unknown_profile,
            )
            .and_then(|()| {
                super::target_policy::ensure_supported_use_allows(
                    &self.profile_runtime,
                    &foreground,
                    TOOL,
                )
            })
        })()
        .map_err(|error| reflex_denial_from_error(&error))
    }
}

struct M1AimTrackTargetSource {
    m1_state: super::SharedM1State,
}

impl AimTrackTargetSource for M1AimTrackTargetSource {
    fn snapshot(&self) -> AimTrackTargetSnapshot {
        let input = {
            let state = match self.m1_state.lock() {
                Ok(state) => state,
                Err(_error) => {
                    return target_source_error_snapshot(
                        "M1 service state lock poisoned while resolving aim_track target",
                    );
                }
            };
            crate::m1::current_input(&state, AIM_TRACK_TARGET_SOURCE_DEPTH)
        };
        match input {
            Ok(input) => AimTrackTargetSnapshot {
                entities: input.entities,
                elements: resolved_elements_from_input(&input.focused, &input.elements),
                source_label: Some("m1_current_input".to_owned()),
                source_seq: None,
                source_error: None,
            },
            Err(error) => {
                tracing::warn!(
                    code = "AIM_TRACK_TARGET_SOURCE_UNAVAILABLE",
                    detail = %error,
                    "aim_track target source could not read current M1 input"
                );
                target_source_error_snapshot(error.to_string())
            }
        }
    }
}

fn target_source_error_snapshot(detail: impl Into<String>) -> AimTrackTargetSnapshot {
    AimTrackTargetSnapshot {
        source_label: Some("m1_current_input".to_owned()),
        source_error: Some(detail.into()),
        ..AimTrackTargetSnapshot::default()
    }
}

fn resolved_elements_from_input(
    focused: &Option<FocusedElement>,
    elements: &[AccessibleNode],
) -> Vec<ResolvedElementBox> {
    let mut seen = HashSet::<ElementId>::new();
    let mut resolved = Vec::new();
    if let Some(focused) = focused {
        push_resolved_element(&mut seen, &mut resolved, &focused.element_id, focused.bbox);
    }
    for element in elements {
        push_resolved_element(&mut seen, &mut resolved, &element.element_id, element.bbox);
    }
    resolved
}

fn push_resolved_element(
    seen: &mut HashSet<ElementId>,
    resolved: &mut Vec<ResolvedElementBox>,
    element_id: &ElementId,
    bbox: synapse_core::Rect,
) {
    if seen.insert(element_id.clone()) {
        resolved.push(ResolvedElementBox {
            element_id: element_id.clone(),
            bbox,
        });
    }
}

fn current_reflex_action_foreground(
    m1_state: &super::SharedM1State,
) -> Result<ForegroundContext, ErrorData> {
    {
        let state = m1_state.lock().map_err(|_err| {
            mcp_error(
                error_codes::OBSERVE_INTERNAL,
                "M1 service state lock poisoned while checking reflex dispatch scope",
            )
        })?;
        if state.force_observe_internal {
            return Err(mcp_error(
                error_codes::OBSERVE_INTERNAL,
                "forced observe internal error",
            ));
        }
        if state.force_no_perception {
            return Err(mcp_error(
                error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
                "no perception source is available",
            ));
        }
        if let Some(input) = &state.synthetic {
            return Ok(input.foreground.clone());
        }
    }
    synapse_a11y::current_foreground_context()
        .map_err(|error| mcp_error(error.code(), error.to_string()))
}

fn foreground_window(foreground: &ForegroundContext) -> synapse_profiles::ForegroundWindow {
    synapse_profiles::ForegroundWindow {
        exe: non_empty(&foreground.process_name),
        title: non_empty(&foreground.window_title),
        steam_appid: foreground.steam_appid,
        window_class: None,
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn publish_profile_transition_events(
    event_bus: &EventBus,
    transition: &ForegroundProfileTransition,
    foreground: &ForegroundContext,
) {
    if transition.changed {
        let report = event_bus.publish(profile_transition_event(
            PROFILE_CHANGED_KIND,
            profile_changed_event_data(transition, foreground),
        ));
        tracing::debug!(
            code = "PROFILE_CHANGED_EVENT_PUBLISHED",
            matched = report.matched,
            queued = report.queued,
            dropped = report.dropped,
            previous_profile_id = ?transition.previous_profile_id,
            active_profile_id = ?transition.active_profile_id,
            "profile-changed event published"
        );
    }
    if transition.scope_changed {
        let report = event_bus.publish(profile_transition_event(
            SCOPE_TRANSITIONED_KIND,
            scope_transition_event_data(transition, foreground),
        ));
        tracing::debug!(
            code = "SCOPE_TRANSITIONED_EVENT_PUBLISHED",
            matched = report.matched,
            queued = report.queued,
            dropped = report.dropped,
            old_scope = profile_use_scope_label(transition.effective_previous_scope),
            new_scope = profile_use_scope_label(transition.effective_active_scope),
            "scope-transitioned event published"
        );
    }
}

fn profile_transition_event(kind: &str, data: serde_json::Value) -> Event {
    Event {
        seq: NEXT_PROFILE_EVENT_SEQ.fetch_add(1, Ordering::Relaxed),
        at: Utc::now(),
        source: EventSource::System,
        kind: kind.to_owned(),
        data,
        correlations: Vec::new(),
    }
}

fn profile_changed_event_data(
    transition: &ForegroundProfileTransition,
    foreground: &ForegroundContext,
) -> serde_json::Value {
    json!({
        "old_profile_id": transition.previous_profile_id.clone(),
        "new_profile_id": transition.active_profile_id.clone(),
        "old_scope": transition.previous_scope.map(profile_use_scope_label),
        "new_scope": transition.active_scope.map(profile_use_scope_label),
        "effective_old_scope": profile_use_scope_label(transition.effective_previous_scope),
        "effective_new_scope": profile_use_scope_label(transition.effective_active_scope),
        "match_rank": transition.resolution.as_ref().map(|resolution| resolution.rank_name),
        "foreground": foreground_event_data(foreground),
    })
}

fn scope_transition_event_data(
    transition: &ForegroundProfileTransition,
    foreground: &ForegroundContext,
) -> serde_json::Value {
    json!({
        "old_profile_id": transition.previous_profile_id.clone(),
        "new_profile_id": transition.active_profile_id.clone(),
        "old_scope": profile_use_scope_label(transition.effective_previous_scope),
        "new_scope": profile_use_scope_label(transition.effective_active_scope),
        "old_profile_scope": transition.previous_scope.map(profile_use_scope_label),
        "new_profile_scope": transition.active_scope.map(profile_use_scope_label),
        "match_rank": transition.resolution.as_ref().map(|resolution| resolution.rank_name),
        "foreground": foreground_event_data(foreground),
    })
}

fn foreground_event_data(foreground: &ForegroundContext) -> serde_json::Value {
    json!({
        "hwnd": foreground.hwnd,
        "pid": foreground.pid,
        "process_name": foreground.process_name.clone(),
        "process_path": foreground.process_path.clone(),
        "window_title": foreground.window_title.clone(),
        "steam_appid": foreground.steam_appid,
    })
}

fn reflex_denial_from_error(error: &ErrorData) -> ReflexActionPermissionDenied {
    let data = error.data.as_ref();
    ReflexActionPermissionDenied {
        policy_code: data
            .and_then(|value| value.get("code"))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
        policy_reason: data
            .and_then(|value| value.get("reason"))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
        profile_id: data
            .and_then(|value| value.get("profile_id"))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
        use_scope: data
            .and_then(|value| value.get("use_scope"))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
        detail: data
            .and_then(|value| value.get("detail"))
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| error.message.to_string(), ToOwned::to_owned),
    }
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
    use std::{fs, num::NonZeroUsize, path::Path, time::Duration};

    use rmcp::handler::server::wrapper::Parameters;
    use serde_json::{Value, json};
    use synapse_core::{
        AccessibleNode, Action, EventFilter, FocusedElement, ForegroundContext, Rect, SensorStatus,
        UiaPattern, element_id,
    };
    use synapse_perception::ObservationInput;
    use synapse_storage::cf;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        m1::{FindParams, FindScope, ObserveParams, ReadTextParams},
        m2::M2ServiceConfig,
        m3::{M3ServiceConfig, subscribe::SubscribeParams},
        m4::M4ServiceConfig,
    };

    const ACTION_WRITE_TOOLS: [&str; 13] = [
        "act_click",
        "act_type",
        "act_press",
        "act_keymap",
        "act_aim",
        "act_drag",
        "act_scroll",
        "act_pad",
        "act_clipboard",
        "act_combo",
        "act_run_shell",
        "act_launch",
        "reflex_register",
    ];

    #[test]
    fn instructions_advertise_m3_when_current_m3_tools_are_registered() -> anyhow::Result<()> {
        let profiles = TempDir::new()?;
        let service = service_with_profiles(profiles.path(), false)?;

        assert_eq!(crate::m3::m3_tool_stubs().len(), 33);
        assert!(service.instructions().contains("M3 scaffold"));

        Ok(())
    }

    #[test]
    fn action_scope_gate_denies_no_active_profile_before_dispatch() -> anyhow::Result<()> {
        let profiles = TempDir::new()?;
        write_profile(&profiles.path().join("known.toml"), "known", "productivity")?;
        let service = service_with_profiles(profiles.path(), false)?;
        install_synthetic_notepad_input(&service)?;

        let error = match service.ensure_supported_use_allows_action("act_type") {
            Ok(_) => anyhow::bail!("action tools must fail closed without an active profile"),
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
        install_synthetic_process_input(&service, "unknown.exe", "Unknown App", 0x4567)?;
        let runtime = service.profile_runtime()?;
        runtime.activate("unknown")?;

        for tool in ACTION_WRITE_TOOLS {
            let error = match service.ensure_supported_use_allows_action(tool) {
                Ok(_) => anyhow::bail!(
                    "unknown scope must fail closed for {tool} without the explicit override"
                ),
                Err(error) => error,
            };

            assert_eq!(
                error.data.as_ref().and_then(|data| data.get("code")),
                Some(&json!(error_codes::SAFETY_PROFILE_ACTION_DENIED))
            );
            assert_eq!(
                error.data.as_ref().and_then(|data| data.get("tool")),
                Some(&json!(tool))
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
        }
        Ok(())
    }

    #[test]
    fn action_scope_gate_allows_single_player_profile_for_all_action_write_tools()
    -> anyhow::Result<()> {
        let profiles = TempDir::new()?;
        write_profile(
            &profiles.path().join("single-player.toml"),
            "single-player",
            "single_player",
        )?;
        let service = service_with_profiles(profiles.path(), false)?;
        install_synthetic_process_input(&service, "single-player.exe", "Single Player", 0x4567)?;
        let runtime = service.profile_runtime()?;
        runtime.activate("single-player")?;

        for tool in ACTION_WRITE_TOOLS {
            service.ensure_supported_use_allows_action(tool)?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn observe_persists_audit_session_observation_and_event_rows() -> anyhow::Result<()> {
        let profiles = TempDir::new()?;
        write_profile(
            &profiles.path().join("notepad.toml"),
            "notepad",
            "productivity",
        )?;
        let service = service_with_profiles(profiles.path(), false)?;
        let runtime = service.reflex_runtime()?;
        let before_counts = runtime
            .lock()
            .map_err(|_err| anyhow::anyhow!("reflex runtime lock poisoned"))?
            .storage_cf_row_counts()?;
        assert_eq!(cf_count(&before_counts, cf::CF_EVENTS), 0);
        assert_eq!(cf_count(&before_counts, cf::CF_OBSERVATIONS), 0);
        assert_eq!(cf_count(&before_counts, cf::CF_SESSIONS), 0);

        install_synthetic_notepad_input(&service)?;
        let observation = service
            .observe(Parameters(ObserveParams::default()))
            .await?;
        assert_eq!(
            observation.0.foreground.profile_id.as_deref(),
            Some("notepad")
        );

        let runtime = service.reflex_runtime()?;
        let runtime = runtime
            .lock()
            .map_err(|_err| anyhow::anyhow!("reflex runtime lock poisoned"))?;
        let after_counts = runtime.storage_cf_row_counts()?;
        assert_eq!(cf_count(&after_counts, cf::CF_EVENTS), 1);
        assert_eq!(cf_count(&after_counts, cf::CF_OBSERVATIONS), 1);
        assert_eq!(cf_count(&after_counts, cf::CF_SESSIONS), 1);

        let observation_rows = runtime.storage_cf_tail_rows(cf::CF_OBSERVATIONS, 1)?;
        let stored_observation: Value = serde_json::from_slice(&observation_rows[0].1)?;
        assert_eq!(stored_observation["reason"], "observe");
        assert_eq!(stored_observation["foreground"]["profile_id"], "notepad");
        assert_eq!(
            stored_observation["foreground"]["process_name"],
            "notepad.exe"
        );

        let event_rows = runtime.storage_cf_tail_rows(cf::CF_EVENTS, 1)?;
        let stored_event: Value = serde_json::from_slice(&event_rows[0].1)?;
        assert_eq!(stored_event["kind"], "perception.observed");
        assert_eq!(stored_event["source"], "perception");
        assert_eq!(
            stored_event["data"]["observation_id"],
            stored_observation["observation_id"]
        );
        assert_eq!(stored_event["data"]["hud_fields"], json!([]));
        assert_eq!(stored_event["data"]["hud_error_fields"], json!([]));

        let session_rows = runtime.storage_cf_tail_rows(cf::CF_SESSIONS, 1)?;
        let stored_session: Value = serde_json::from_slice(&session_rows[0].1)?;
        assert_eq!(
            stored_session["session_id"],
            stored_observation["session_id"]
        );
        assert_eq!(stored_session["active_profile"], "notepad");
        drop(runtime);
        Ok(())
    }

    #[tokio::test]
    async fn read_only_tools_remain_available_with_active_unknown_scope() -> anyhow::Result<()> {
        let profiles = TempDir::new()?;
        write_profile(&profiles.path().join("unknown.toml"), "unknown", "unknown")?;
        let service = service_with_profiles(profiles.path(), false)?;
        install_synthetic_notepad_input(&service)?;
        let runtime = service.profile_runtime()?;
        runtime.activate("unknown")?;

        assert!(service.health_payload().ok);

        let observation = service
            .observe(Parameters(ObserveParams::default()))
            .await?;
        assert_eq!(observation.0.foreground.process_name, "notepad.exe");

        let matches = service
            .find(Parameters(FindParams {
                query: Some("Document".to_owned()),
                role: None,
                name_substring: None,
                automation_id: None,
                scope: Some(FindScope::Elements),
                limit: Some(5),
                in_window: None,
            }))
            .await?;
        assert!(
            matches
                .0
                .results
                .iter()
                .any(|result| result.name.as_deref() == Some("Document"))
        );

        let ocr = service
            .read_text(Parameters(ReadTextParams {
                region: Some(Rect {
                    x: 12,
                    y: 80,
                    w: 120,
                    h: 40,
                }),
                element_id: None,
                backend: synapse_core::OcrBackend::Winrt,
                lang_hint: None,
            }))
            .await?;
        assert_eq!(ocr.0.full_text, "Synapse");

        let subscription = service
            .subscribe(Parameters(SubscribeParams {
                kinds: Vec::new(),
                filter: Some(EventFilter::All),
                snapshot_first: false,
                buffer_size: 4096,
            }))
            .await?;
        assert!(!subscription.0.subscription_id.is_empty());
        Ok(())
    }

    #[test]
    fn reflex_action_gate_rechecks_active_profile_scope_on_dispatch() -> anyhow::Result<()> {
        let profiles = TempDir::new()?;
        write_profile(
            &profiles.path().join("single-player.toml"),
            "single-player",
            "single_player",
        )?;
        write_profile(&profiles.path().join("unknown.toml"), "unknown", "unknown")?;
        let service = service_with_profiles(profiles.path(), false)?;
        install_synthetic_process_input(&service, "single-player.exe", "Single Player", 0x4567)?;
        let runtime = service.profile_runtime()?;
        runtime.activate("single-player")?;
        let gate = service.reflex_action_gate()?;
        let reflex_id = "reflex-profile-transition".to_owned();
        let action = Action::ReleaseAll;

        gate.ensure_action_allowed(&reflex_id, &action)
            .map_err(|denial| anyhow::anyhow!("single-player dispatch denied: {denial:?}"))?;

        install_synthetic_process_input(&service, "unknown.exe", "Unknown App", 0x4568)?;
        runtime.activate("unknown")?;
        let denial = match gate.ensure_action_allowed(&reflex_id, &action) {
            Ok(()) => anyhow::bail!("unknown active profile must deny reflex dispatch"),
            Err(denial) => denial,
        };
        assert_eq!(denial.policy_reason.as_deref(), Some("unknown_scope"));
        assert_eq!(denial.profile_id.as_deref(), Some("unknown"));
        assert_eq!(denial.use_scope.as_deref(), Some("unknown"));
        Ok(())
    }

    #[tokio::test]
    async fn observe_reevaluates_foreground_and_publishes_scope_transition_within_200ms()
    -> anyhow::Result<()> {
        let profiles = TempDir::new()?;
        write_profile(
            &profiles.path().join("notepad.toml"),
            "notepad",
            "productivity",
        )?;
        let service = service_with_profiles(profiles.path(), false)?;
        let subscription = service.sse_state()?.event_bus().subscribe(
            EventFilter::All,
            vec![
                PROFILE_CHANGED_KIND.to_owned(),
                SCOPE_TRANSITIONED_KIND.to_owned(),
            ],
            false,
        )?;

        install_synthetic_notepad_input(&service)?;
        let first = service
            .observe(Parameters(ObserveParams::default()))
            .await?;
        assert_eq!(first.0.foreground.profile_id.as_deref(), Some("notepad"));
        let runtime = service.profile_runtime()?;
        assert_eq!(runtime.active_profile_id()?.as_deref(), Some("notepad"));
        let _initial_events = subscription.drain();

        install_synthetic_process_input(&service, "unprofiled.exe", "Unprofiled App", 0x6789)?;
        let started = std::time::Instant::now();
        let second = service
            .observe(Parameters(ObserveParams::default()))
            .await?;
        let elapsed = started.elapsed();
        assert!(elapsed <= Duration::from_millis(200));
        assert_eq!(second.0.foreground.profile_id, None);
        assert_eq!(runtime.active_profile_id()?, None);

        let events = subscription.drain();
        assert!(
            events
                .iter()
                .any(|event| event.kind == PROFILE_CHANGED_KIND)
        );
        let Some(scope_event) = events
            .iter()
            .find(|event| event.kind == SCOPE_TRANSITIONED_KIND)
        else {
            anyhow::bail!("scope-transitioned event missing: {events:?}");
        };
        assert_eq!(
            scope_event.data.get("old_scope"),
            Some(&json!("productivity"))
        );
        assert_eq!(scope_event.data.get("new_scope"), Some(&json!("unknown")));
        assert_eq!(scope_event.data.get("new_profile_id"), Some(&json!(null)));
        Ok(())
    }

    #[test]
    fn action_scope_gate_reevaluates_foreground_and_denies_no_profile_after_transition()
    -> anyhow::Result<()> {
        let profiles = TempDir::new()?;
        write_profile(
            &profiles.path().join("notepad.toml"),
            "notepad",
            "productivity",
        )?;
        let service = service_with_profiles(profiles.path(), false)?;
        install_synthetic_notepad_input(&service)?;

        service.ensure_supported_use_allows_action("act_press")?;
        let runtime = service.profile_runtime()?;
        assert_eq!(runtime.active_profile_id()?.as_deref(), Some("notepad"));

        install_synthetic_process_input(&service, "unprofiled.exe", "Unprofiled App", 0x6789)?;
        let error = match service.ensure_supported_use_allows_action("act_press") {
            Ok(_) => anyhow::bail!("unprofiled foreground must fail closed"),
            Err(error) => error,
        };
        assert_eq!(runtime.active_profile_id()?, None);
        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("code")),
            Some(&json!(error_codes::SAFETY_PROFILE_ACTION_DENIED))
        );
        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("reason")),
            Some(&json!("no_profile"))
        );
        Ok(())
    }

    #[test]
    fn action_scope_gate_reevaluates_foreground_and_denies_unknown_scope_after_transition()
    -> anyhow::Result<()> {
        let profiles = TempDir::new()?;
        write_profile(
            &profiles.path().join("notepad.toml"),
            "notepad",
            "productivity",
        )?;
        write_profile(&profiles.path().join("unknown.toml"), "unknown", "unknown")?;
        let service = service_with_profiles(profiles.path(), false)?;
        install_synthetic_notepad_input(&service)?;

        service.ensure_supported_use_allows_action("act_press")?;
        let runtime = service.profile_runtime()?;
        assert_eq!(runtime.active_profile_id()?.as_deref(), Some("notepad"));

        install_synthetic_process_input(&service, "unknown.exe", "Unknown App", 0x4568)?;
        let error = match service.ensure_supported_use_allows_action("act_press") {
            Ok(_) => anyhow::bail!("unknown-scope foreground must fail closed"),
            Err(error) => error,
        };
        assert_eq!(runtime.active_profile_id()?.as_deref(), Some("unknown"));
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

    #[test]
    fn aim_track_target_source_reads_shallow_observe_child_elements() -> anyhow::Result<()> {
        let profiles = TempDir::new()?;
        let service = service_with_profiles(profiles.path(), false)?;
        install_synthetic_notepad_input(&service)?;

        let source = M1AimTrackTargetSource {
            m1_state: service.m1_state.clone(),
        };
        let snapshot = source.snapshot();

        assert_eq!(AIM_TRACK_TARGET_SOURCE_DEPTH, 2);
        assert!(snapshot.source_error.is_none());
        assert!(
            snapshot
                .elements
                .iter()
                .any(|element| { element.element_id == element_id(0x1234, "0000002a00000000") })
        );
        assert!(
            snapshot
                .elements
                .iter()
                .any(|element| { element.element_id == element_id(0x1234, "0000002a00000001") })
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
                Some(profile_dir.join("db")),
                Some(profile_dir.to_path_buf()),
                false,
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

    fn cf_count(counts: &std::collections::BTreeMap<String, u64>, cf_name: &str) -> u64 {
        counts.get(cf_name).copied().unwrap_or(0)
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

    fn install_synthetic_notepad_input(service: &SynapseService) -> anyhow::Result<()> {
        install_synthetic_input(service, synthetic_notepad_input())
    }

    fn install_synthetic_process_input(
        service: &SynapseService,
        process_name: &str,
        window_title: &str,
        hwnd: i64,
    ) -> anyhow::Result<()> {
        install_synthetic_input(
            service,
            synthetic_process_input(process_name, window_title, hwnd),
        )
    }

    fn install_synthetic_input(
        service: &SynapseService,
        input: ObservationInput,
    ) -> anyhow::Result<()> {
        let mut state = service.m1_state.lock().map_err(|_err| {
            anyhow::anyhow!("M1 service state lock poisoned while installing synthetic input")
        })?;
        state.synthetic = Some(input);
        drop(state);
        Ok(())
    }

    fn synthetic_process_input(
        process_name: &str,
        window_title: &str,
        hwnd: i64,
    ) -> ObservationInput {
        let mut input = synthetic_notepad_input();
        input.foreground.hwnd = hwnd;
        input.foreground.pid = u32::try_from(hwnd & 0xffff).unwrap_or(0);
        input.foreground.process_name = process_name.to_owned();
        input.foreground.process_path = format!("C:\\Synthetic\\{process_name}");
        input.foreground.window_title = window_title.to_owned();
        input.foreground.profile_id = None;
        input
    }

    fn synthetic_notepad_input() -> ObservationInput {
        let document_id = element_id(0x1234, "0000002a00000001");
        let mut input = ObservationInput::new(ForegroundContext {
            hwnd: 0x1234,
            pid: 44,
            process_name: "notepad.exe".to_owned(),
            process_path: "C:\\Windows\\System32\\notepad.exe".to_owned(),
            window_title: "manual.txt - Notepad".to_owned(),
            window_bounds: Rect {
                x: 10,
                y: 20,
                w: 800,
                h: 600,
            },
            monitor_index: 0,
            dpi_scale: 1.0,
            profile_id: None,
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
        });
        input.focused = Some(FocusedElement {
            element_id: document_id.clone(),
            name: "Document".to_owned(),
            role: "Edit".to_owned(),
            automation_id: Some("15".to_owned()),
            bbox: Rect {
                x: 12,
                y: 80,
                w: 760,
                h: 480,
            },
            enabled: true,
            patterns: vec![UiaPattern::Text, UiaPattern::Value],
            value: Some("Synthetic Synapse text".to_owned()),
            selected_text: None,
        });
        input.elements = vec![
            AccessibleNode {
                element_id: element_id(0x1234, "0000002a00000000"),
                parent: None,
                name: "Notepad".to_owned(),
                role: "Window".to_owned(),
                automation_id: None,
                value: None,
                bbox: Rect {
                    x: 10,
                    y: 20,
                    w: 800,
                    h: 600,
                },
                enabled: true,
                focused: false,
                patterns: Vec::new(),
                children_count: 1,
                depth: 0,
            },
            AccessibleNode {
                element_id: document_id,
                parent: Some(element_id(0x1234, "0000002a00000000")),
                name: "Document".to_owned(),
                role: "Edit".to_owned(),
                automation_id: Some("15".to_owned()),
                value: None,
                bbox: Rect {
                    x: 12,
                    y: 80,
                    w: 760,
                    h: 480,
                },
                enabled: true,
                focused: true,
                patterns: vec![UiaPattern::Text, UiaPattern::Value],
                children_count: 0,
                depth: 1,
            },
        ];
        input.a11y_status = SensorStatus::Healthy;
        input.capture_status = SensorStatus::Healthy;
        input.detection_status = SensorStatus::Disabled;
        input.audio_status = SensorStatus::Disabled;
        input
    }
}
