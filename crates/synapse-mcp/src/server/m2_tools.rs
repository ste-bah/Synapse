use super::{
    ActClickParams, ActClickResponse, ActClipboardParams, ActClipboardResponse, ActClipboardVerb,
    ActFocusWindowParams, ActFocusWindowResponse, ActKeymapParams, ActKeymapResponse, ActPadParams,
    ActPadResponse, ActPressParams, ActPressResponse, ActScrollParams, ActScrollResponse,
    ActSetValueParams, ActSetValueResponse, ActStrokeParams, ActStrokeResponse, ActTypeParams,
    ActTypeResponse, ErrorData, Json, Parameters, ReleaseAllParams, ReleaseAllResponse,
    SynapseService, act_click_with_handle_and_lease, act_clipboard, act_focus_window,
    act_focus_window_request_details, act_keymap_with_handle, act_pad_with_handle,
    act_press_with_handle, act_scroll_with_handle, act_set_value, act_set_value_request_details,
    act_stroke_validation_failure_details, act_stroke_with_handle, act_type_with_handle,
    action_preflight::{ActionPreflightReadback, ForegroundProof},
    release_all_with_handles, tool, tool_router, validate_act_stroke_params,
};
use crate::m1::mcp_error;
use crate::m2::postcondition::{
    ActPostcondition, hash_json as verify_hash_json,
    no_observed_delta_error as source_no_observed_delta_error, postcondition_failed_error,
    postcondition_observed_delta,
};
use crate::m2::{
    ActClickPostcondition, ActClickTierAttempt, CLICK_REASON_NO_OBSERVED_DELTA,
    act_click_postmessage_with_params, act_stroke_error_details, act_stroke_request_details,
    attach_click_tier_attempts, click_params_can_route_background_first, click_tier_failed,
    emitted_text,
};
use rmcp::{RoleServer, model::ErrorCode, service::RequestContext};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{sync::Arc, time::Duration};
use synapse_action::{
    ACTION_QUEUE_CAPACITY, ActionEmitterSnapshotHandle, ActionError, ActionHandle,
    ActionStateSnapshot, RecordingBackend, ResolvedBackend, TokenBucketSnapshot,
};
use synapse_core::{
    AccessibleNode, Action, Backend, FocusedElement, ForegroundContext, PathPoint, PathSpec, Point,
    Rect, StrokeMotionModel, StrokeTiming, UiaPattern, VelocityProfile, error_codes,
};
use tokio_util::sync::CancellationToken;

const ACT_STROKE_FOREGROUND_MONITOR_INTERVAL_MS: u64 = 10;
const ACTION_DIAGNOSTIC_RATE_LIMIT_CONFIRM: &str = "force-real-rate-limit-for-fsv";
const ACTION_DIAGNOSTIC_QUEUE_FULL_CONFIRM: &str = "saturate-real-action-queue-for-fsv";
const ACTION_DIAGNOSTIC_MAX_TTL_MS: u64 = 10_000;
const ACTION_DIAGNOSTIC_MIN_TTL_MS: u64 = 100;
const ACTION_DIAGNOSTIC_MAX_QUEUE_BLOCKER_MS: u32 = 10_000;
const ACTION_DIAGNOSTIC_MIN_QUEUE_BLOCKER_MS: u32 = 250;
const ACTION_DIAGNOSTIC_QUEUE_SETTLE_MS: u64 = 50;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionDiagnosticRateLimitOverrideParams {
    #[serde(default)]
    pub confirm: String,
    #[serde(default = "default_diagnostic_ttl_ms")]
    #[schemars(default = "default_diagnostic_ttl_ms", range(min = 100, max = 10000))]
    pub ttl_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionDiagnosticRateLimitOverrideResponse {
    pub backend: String,
    pub ttl_ms: u64,
    pub before: TokenBucketReadback,
    pub after: TokenBucketReadback,
    pub reset_scheduled: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionDiagnosticQueueFullSetupParams {
    #[serde(default)]
    pub confirm: String,
    #[serde(default = "default_queue_blocker_duration_ms")]
    #[schemars(
        default = "default_queue_blocker_duration_ms",
        range(min = 250, max = 10000)
    )]
    pub blocker_duration_ms: u32,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionDiagnosticQueueFullSetupResponse {
    pub backend: String,
    pub expected_queue_capacity: u32,
    pub blocker_duration_ms: u32,
    pub blocker_from: Point,
    pub blocker_to: Point,
    pub blocker_queued: bool,
    pub filler_attempts: u32,
    pub queued_fillers: u32,
    pub queue_full_observed: bool,
    pub next_act_stroke_expected_error: String,
}

#[derive(Copy, Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenBucketReadback {
    pub capacity: u32,
    pub tokens: u32,
    pub refill_rate_per_s: u32,
    pub last_refill_ns: u64,
}

const fn default_diagnostic_ttl_ms() -> u64 {
    5_000
}

const fn default_queue_blocker_duration_ms() -> u32 {
    5_000
}

#[tool_router(router = m2_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Click a screen coordinate or UI Automation element. Default element delivery uses background UIA control patterns (Invoke, Toggle, SelectionItem, ExpandCollapse, LegacyIAccessible.DoDefaultAction). When those patterns are unsupported, coordinate_fallback_on_unsupported=true allows a recorded bbox-center coordinate click only for enabled keyboard-focusable edit/document/text targets or elements exposing Value/Text patterns; set false to fail closed with ACTION_ELEMENT_PATTERN_UNSUPPORTED. velocity_profile controls coordinate-move timing only, while explicit spatial paths belong to act_stroke. If a previously observed transient element expired before dispatch, returns TRANSIENT_ELEMENT_EXPIRED with re-observe/find guidance."
    )]
    pub async fn act_click(
        &self,
        params: Parameters<ActClickParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActClickResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_click",
            "tool.invocation kind=act_click"
        );
        let preflight = match self.ensure_supported_use_allows_action("act_click") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied("act_click", &error);
                return Err(error);
            }
        };
        self.audit_action_started_with_details("act_click", &action_preflight_details(&preflight))?;
        if let Err(error) = ensure_everquest_click_backend(&params, &preflight) {
            let result: Result<ActClickResponse, ErrorData> = Err(error);
            self.audit_action_result("act_click", &result)?;
            return result.map(Json);
        }
        let before_delta_signature = if params.verify_delta {
            match self.capture_click_delta_signature(160).await {
                Ok(signature) => Some(signature),
                Err(error) => {
                    let result: Result<ActClickResponse, ErrorData> = Err(error);
                    self.audit_action_result("act_click", &result)?;
                    return result.map(Json);
                }
            }
        } else {
            None
        };
        let verify_timeout_ms = params.verify_timeout_ms;
        let foreground_lease_session_id = foreground_lease_session_id(&request_context)?;
        let (handle, recording, _connection_closed_cancel) =
            self.m2_action_context_for_request(&request_context)?;
        let result = if let Some(before) = before_delta_signature {
            self.act_click_with_verified_router(
                handle,
                recording,
                params,
                before,
                verify_timeout_ms,
                foreground_lease_session_id,
            )
            .await
        } else {
            act_click_with_handle_and_lease(
                handle,
                recording,
                params,
                foreground_lease_session_id.as_deref(),
            )
            .await
        };
        self.audit_action_result("act_click", &result)?;
        result.map(Json)
    }

    #[tool(description = "Type text through the active keyboard backend")]
    pub async fn act_type(
        &self,
        params: Parameters<ActTypeParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActTypeResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_type",
            "tool.invocation kind=act_type"
        );
        let preflight = match self.ensure_supported_use_allows_action("act_type") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied("act_type", &error);
                return Err(error);
            }
        };
        self.audit_action_started_with_details("act_type", &action_preflight_details(&preflight))?;
        let (handle, recording, _connection_closed_cancel) =
            self.m2_action_context_for_request(&request_context)?;
        if params.into_element.is_none()
            && let Err(error) = self.ensure_act_type_foreground(&preflight, recording.as_ref())
        {
            let result: Result<ActTypeResponse, ErrorData> = Err(error);
            self.audit_action_result("act_type", &result)?;
            return result.map(Json);
        }
        let _lease_guard = if params.into_element.is_none() {
            match acquire_tool_foreground_input_lease("act_type", &request_context) {
                Ok(guard) => Some(guard),
                Err(error) => {
                    let result: Result<ActTypeResponse, ErrorData> = Err(error);
                    self.audit_action_result("act_type", &result)?;
                    return result.map(Json);
                }
            }
        } else {
            None
        };
        let verify_timeout_ms = params.verify_timeout_ms;
        let emitted = emitted_text(&params);
        let before_text_signature = if params.into_element.is_none() {
            match self.capture_act_type_text_signature(160).await {
                Ok(signature) => Some(signature),
                Err(error) => {
                    let result: Result<ActTypeResponse, ErrorData> = Err(error);
                    self.audit_action_result("act_type", &result)?;
                    return result.map(Json);
                }
            }
        } else {
            None
        };
        let result = act_type_with_handle(handle, recording, params).await;
        let result = match (result, before_text_signature) {
            (Ok(response), Some(before)) => {
                self.verify_act_type_response(response, before, verify_timeout_ms, &emitted)
                    .await
            }
            (other, _) => other,
        };
        self.audit_action_result("act_type", &result)?;
        result.map(Json)
    }

    #[tool(
        description = "Set a UI Automation element's ValuePattern value directly and verify with a separate UIA value readback. Requires a real enabled non-read-only ValuePattern target; does not fall back to keyboard typing."
    )]
    pub async fn act_set_value(
        &self,
        params: Parameters<ActSetValueParams>,
    ) -> Result<Json<ActSetValueResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_set_value",
            "tool.invocation kind=act_set_value"
        );
        let request_details = act_set_value_request_details(&params);
        let preflight = match self.ensure_supported_use_allows_action("act_set_value") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied_with_details("act_set_value", &error, &request_details);
                return Err(error);
            }
        };
        self.audit_action_started_with_details(
            "act_set_value",
            &json!({
                "request": request_details,
                "preflight": preflight,
            }),
        )?;
        let result = act_set_value(params).await;
        self.audit_action_result("act_set_value", &result)?;
        result.map(Json)
    }

    #[tool(
        description = "Focus or activate one visible top-level native window by exact hwnd, unique title_regex, or unique pid. The action fails closed on missing or ambiguous targets and verifies success with a separate GetForegroundWindow readback."
    )]
    pub async fn act_focus_window(
        &self,
        params: Parameters<ActFocusWindowParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActFocusWindowResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_focus_window",
            "tool.invocation kind=act_focus_window"
        );
        let request_details = act_focus_window_request_details(&params);
        let preflight = match self.ensure_supported_use_allows_action("act_focus_window") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied_with_details("act_focus_window", &error, &request_details);
                return Err(error);
            }
        };
        self.audit_action_started_with_details(
            "act_focus_window",
            &json!({
                "request": request_details,
                "preflight": preflight,
            }),
        )?;
        let mut lease_guard =
            match acquire_tool_foreground_input_lease("act_focus_window", &request_context) {
                Ok(guard) => guard,
                Err(error) => {
                    self.audit_action_error_with_details(
                        "act_focus_window",
                        &error,
                        &request_details,
                    )?;
                    return Err(error);
                }
            };
        lease_guard.disable_context_restore("act_focus_window_intentional_foreground_change");
        let result = act_focus_window(params).await;
        self.audit_action_result("act_focus_window", &result)?;
        result.map(Json)
    }

    #[tool(description = "Press a keyboard key or ordered chord")]
    pub async fn act_press(
        &self,
        params: Parameters<ActPressParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActPressResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_press",
            "tool.invocation kind=act_press"
        );
        super::context::maybe_force_panic_during_act("act_press");
        let preflight = match self.ensure_supported_use_allows_action("act_press") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied("act_press", &error);
                return Err(error);
            }
        };
        self.audit_action_started_with_details("act_press", &action_preflight_details(&preflight))?;
        let foreground_change_policy = match act_press_foreground_change_policy(&params) {
            Ok(policy) => policy,
            Err(error) => {
                let result: Result<ActPressResponse, ErrorData> = Err(error);
                self.audit_action_result("act_press", &result)?;
                return result.map(Json);
            }
        };
        let before_delta_signature = if params.verify_delta {
            match self.capture_action_delta_signature(160, None, false).await {
                Ok(signature) => Some(signature),
                Err(error) => {
                    let result: Result<ActPressResponse, ErrorData> = Err(error);
                    self.audit_action_result("act_press", &result)?;
                    return result.map(Json);
                }
            }
        } else {
            None
        };
        let verify_timeout_ms = params.verify_timeout_ms;
        let (handle, recording, connection_closed_cancel) =
            self.m2_action_context_for_request(&request_context)?;
        let _lease_guard = match acquire_tool_foreground_input_lease_with_ttl(
            "act_press",
            &request_context,
            lease_ttl_for_hold_ms(params.hold_ms),
        ) {
            Ok(guard) => guard,
            Err(error) => {
                let result: Result<ActPressResponse, ErrorData> = Err(error);
                self.audit_action_result("act_press", &result)?;
                return result.map(Json);
            }
        };
        let result =
            act_press_with_handle(handle, recording, connection_closed_cancel, params).await;
        let result = match (result, before_delta_signature) {
            (Ok(response), Some(before)) => {
                self.verify_act_press_response(
                    response,
                    before,
                    verify_timeout_ms,
                    foreground_change_policy,
                )
                .await
            }
            (other, _) => other,
        };
        self.audit_action_result("act_press", &result)?;
        result.map(Json)
    }

    #[tool(description = "Press a keyboard alias from the active profile keymap")]
    pub async fn act_keymap(
        &self,
        params: Parameters<ActKeymapParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActKeymapResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_keymap",
            alias = %params.alias,
            "tool.invocation kind=act_keymap"
        );
        let request_details = json!({
            "alias": params.alias.trim(),
            "hold_ms": params.hold_ms,
            "backend": params.backend,
        });
        let preflight = match self.ensure_supported_use_allows_action("act_keymap") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied_with_details("act_keymap", &error, &request_details);
                return Err(error);
            }
        };
        self.audit_action_started_with_details(
            "act_keymap",
            &json!({
                "request": request_details,
                "preflight": preflight,
            }),
        )?;
        let profile = {
            let runtime = self.profile_runtime()?;
            let active_profile_id = runtime
                .active_profile_id()
                .map_err(|error| mcp_error(error.code(), error.to_string()))?
                .ok_or_else(|| {
                    mcp_error(
                        error_codes::PROFILE_NOT_FOUND,
                        "act_keymap requires an active profile",
                    )
                })?;
            runtime
                .profile(&active_profile_id)
                .map_err(|error| mcp_error(error.code(), error.to_string()))?
                .ok_or_else(|| {
                    mcp_error(
                        error_codes::PROFILE_NOT_FOUND,
                        format!("active profile {active_profile_id} was not found"),
                    )
                })?
        };
        let (handle, recording, connection_closed_cancel) =
            self.m2_action_context_for_request(&request_context)?;
        let _lease_guard = match acquire_tool_foreground_input_lease_with_ttl(
            "act_keymap",
            &request_context,
            lease_ttl_for_hold_ms(params.hold_ms),
        ) {
            Ok(guard) => guard,
            Err(error) => {
                self.audit_action_error_with_details("act_keymap", &error, &request_details)?;
                return Err(error);
            }
        };
        let result = act_keymap_with_handle(
            handle,
            recording,
            connection_closed_cancel,
            &profile,
            params,
        )
        .await;
        self.audit_action_result("act_keymap", &result)?;
        result.map(Json)
    }

    #[tool(
        description = "Move, aim, or drag to a point/element target or along an explicit spatial path using timed continuous mouse samples; button unset moves/aims and button set drags; motion_model defaults to path and can use wind_mouse for point-to-point line strokes"
    )]
    pub async fn act_stroke(
        &self,
        params: Parameters<ActStrokeParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActStrokeResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_stroke",
            "tool.invocation kind=act_stroke"
        );
        let plan = match validate_act_stroke_params(&params) {
            Ok(plan) => plan,
            Err(error) => {
                let failure_details = act_stroke_validation_failure_details(&params, &error);
                log_act_stroke_failure(&failure_details, &error);
                self.audit_action_error_with_details("act_stroke", &error, &failure_details)?;
                return Err(error);
            }
        };
        let stroke_details = act_stroke_request_details(&params, &plan);
        let preflight = match self.ensure_supported_use_allows_action("act_stroke") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied_with_details(
                    "act_stroke",
                    &error,
                    &json!({
                        "stroke": stroke_details,
                        "failure": act_stroke_error_details(&error),
                    }),
                );
                return Err(error);
            }
        };
        self.audit_action_started_with_details(
            "act_stroke",
            &act_stroke_audit_details(&stroke_details, &preflight),
        )?;
        let before_delta_signature = if params.verify_delta {
            match self.capture_action_delta_signature(160, None, true).await {
                Ok(signature) => Some(signature),
                Err(error) => {
                    self.audit_action_error_with_details(
                        "act_stroke",
                        &error,
                        &act_stroke_failure_audit_details(&stroke_details, &preflight, &error),
                    )?;
                    return Err(error);
                }
            }
        } else {
            None
        };
        let verify_timeout_ms = params.verify_timeout_ms;
        let (handle, recording, _connection_closed_cancel) =
            self.m2_action_context_for_request(&request_context)?;
        let _lease_guard = if plan.requires_input_lease() {
            match acquire_tool_foreground_input_lease("act_stroke", &request_context) {
                Ok(guard) => Some(guard),
                Err(error) => {
                    let failure_details =
                        act_stroke_failure_audit_details(&stroke_details, &preflight, &error);
                    log_act_stroke_failure(&failure_details, &error);
                    self.audit_action_error_with_details("act_stroke", &error, &failure_details)?;
                    return Err(error);
                }
            }
        } else {
            None
        };
        let foreground_monitor = recording
            .is_none()
            .then(|| self.start_act_stroke_foreground_monitor(&preflight));
        let result = act_stroke_with_handle(handle, recording, params.clone(), plan.clone()).await;
        let foreground_error = await_act_stroke_foreground_monitor(foreground_monitor).await;
        let result = match foreground_error {
            Some(error) => Err(error),
            None => result,
        };
        let result = match (result, before_delta_signature) {
            (Ok(response), Some(before)) => {
                self.verify_act_stroke_response(response, before, verify_timeout_ms)
                    .await
            }
            (other, _) => other,
        };
        match &result {
            Ok(response) => {
                self.audit_action_ok_with_details(
                    "act_stroke",
                    &json!({
                        "response": response,
                        "stroke": stroke_details,
                        "preflight": preflight,
                    }),
                )?;
            }
            Err(error) => {
                let failure_details =
                    act_stroke_failure_audit_details(&stroke_details, &preflight, error);
                log_act_stroke_failure(&failure_details, error);
                self.audit_action_error_with_details("act_stroke", error, &failure_details)?;
            }
        }
        result.map(Json)
    }

    #[tool(
        description = "Scroll vertically or horizontally at the current pointer or screen point"
    )]
    pub async fn act_scroll(
        &self,
        params: Parameters<ActScrollParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActScrollResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_scroll",
            "tool.invocation kind=act_scroll"
        );
        let preflight = match self.ensure_supported_use_allows_action("act_scroll") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied("act_scroll", &error);
                return Err(error);
            }
        };
        self.audit_action_started_with_details(
            "act_scroll",
            &action_preflight_details(&preflight),
        )?;
        let point_region = params.verify_delta_point_region();
        let before_delta_signature = if params.verify_delta && !params.uses_element_target() {
            match self
                .capture_action_delta_signature(160, point_region, false)
                .await
            {
                Ok(signature) => Some(signature),
                Err(error) => {
                    let result: Result<ActScrollResponse, ErrorData> = Err(error);
                    self.audit_action_result("act_scroll", &result)?;
                    return result.map(Json);
                }
            }
        } else {
            None
        };
        let verify_timeout_ms = params.verify_timeout_ms;
        let (handle, recording, _connection_closed_cancel) =
            self.m2_action_context_for_request(&request_context)?;
        let _lease_guard = if params.requires_input_lease() {
            match acquire_tool_foreground_input_lease("act_scroll", &request_context) {
                Ok(guard) => Some(guard),
                Err(error) => {
                    let result: Result<ActScrollResponse, ErrorData> = Err(error);
                    self.audit_action_result("act_scroll", &result)?;
                    return result.map(Json);
                }
            }
        } else {
            None
        };
        let result = act_scroll_with_handle(handle, recording, params).await;
        let result = match (result, before_delta_signature) {
            (Ok(response), Some(before)) => {
                self.verify_act_scroll_response(response, before, verify_timeout_ms, point_region)
                    .await
            }
            (other, _) => other,
        };
        self.audit_action_result("act_scroll", &result)?;
        result.map(Json)
    }

    #[tool(description = "Apply a virtual gamepad report and optionally return it to neutral")]
    pub async fn act_pad(
        &self,
        params: Parameters<ActPadParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActPadResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_pad",
            "tool.invocation kind=act_pad"
        );
        let preflight = match self.ensure_supported_use_allows_action("act_pad") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied("act_pad", &error);
                return Err(error);
            }
        };
        self.audit_action_started_with_details("act_pad", &action_preflight_details(&preflight))?;
        let (handle, recording, _connection_closed_cancel) =
            self.m2_action_context_for_request(&request_context)?;
        let snapshot_handle = if params.verify_delta {
            Some(self.m2_snapshot_handle()?)
        } else {
            None
        };
        let before_snapshot = if let Some(snapshot_handle) = &snapshot_handle {
            match snapshot_handle.snapshot().await {
                Ok(snapshot) => Some(snapshot),
                Err(error) => {
                    let error = mcp_error(error.code(), error.to_string());
                    let result: Result<ActPadResponse, ErrorData> = Err(error);
                    self.audit_action_result("act_pad", &result)?;
                    return result.map(Json);
                }
            }
        } else {
            None
        };
        let verify_timeout_ms = params.verify_timeout_ms;
        let result = act_pad_with_handle(handle, recording, params).await;
        let result = match (result, before_snapshot, snapshot_handle) {
            (Ok(response), Some(before), Some(snapshot_handle)) => {
                self.verify_act_pad_response(response, before, snapshot_handle, verify_timeout_ms)
                    .await
            }
            (other, _, _) => other,
        };
        self.audit_action_result("act_pad", &result)?;
        result.map(Json)
    }

    #[tool(description = "Read, write, or clear the system clipboard")]
    pub async fn act_clipboard(
        &self,
        params: Parameters<ActClipboardParams>,
    ) -> Result<Json<ActClipboardResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_clipboard",
            "tool.invocation kind=act_clipboard"
        );
        let request_details = json!({
            "verb": params.verb,
            "format": params.format,
            "text_len": params.text.as_ref().map(|text| text.chars().count()),
        });
        if matches!(
            params.verb,
            ActClipboardVerb::Write | ActClipboardVerb::Clear
        ) && let Err(error) = self.ensure_supported_use_allows_action("act_clipboard")
        {
            self.audit_action_denied_with_details("act_clipboard", &error, &request_details);
            return Err(error);
        }
        self.audit_action_started_with_details("act_clipboard", &request_details)?;
        let _lease_guard = if matches!(
            params.verb,
            ActClipboardVerb::Write | ActClipboardVerb::Clear
        ) {
            match crate::m2::acquire_foreground_input_lease(
                "act_clipboard",
                crate::http::current_mcp_session_id().as_deref(),
            ) {
                Ok(guard) => Some(guard),
                Err(error) => {
                    self.audit_action_error_with_details(
                        "act_clipboard",
                        &error,
                        &request_details,
                    )?;
                    return Err(error);
                }
            }
        } else {
            None
        };
        let result = act_clipboard(params).await;
        match &result {
            Ok(response) => {
                self.audit_action_ok_with_details(
                    "act_clipboard",
                    &clipboard_response_audit_details(response),
                )?;
            }
            Err(error) => {
                self.audit_action_error_with_details("act_clipboard", error, &request_details)?;
            }
        }
        result.map(Json)
    }

    #[tool(
        description = "FSV diagnostic: temporarily force the real software action rate limiter empty so the next real act_stroke proves ACTION_RATE_LIMITED through the normal MCP action path"
    )]
    pub async fn action_diagnostic_rate_limit_override(
        &self,
        params: Parameters<ActionDiagnosticRateLimitOverrideParams>,
    ) -> Result<Json<ActionDiagnosticRateLimitOverrideResponse>, ErrorData> {
        let params = params.0;
        const TOOL: &str = "action_diagnostic_rate_limit_override";
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TOOL,
            "tool.invocation kind=action_diagnostic_rate_limit_override"
        );
        let request_details = json!({
            "backend": ResolvedBackend::Software.as_str(),
            "ttl_ms": params.ttl_ms,
        });
        if let Err(error) =
            require_diagnostic_confirm(&params.confirm, ACTION_DIAGNOSTIC_RATE_LIMIT_CONFIRM, TOOL)
        {
            self.audit_action_denied_with_details(TOOL, &error, &request_details);
            return Err(error);
        }
        if let Err(error) = validate_diagnostic_ttl_ms(params.ttl_ms) {
            self.audit_action_denied_with_details(TOOL, &error, &request_details);
            return Err(error);
        }
        let preflight = match self.ensure_supported_use_allows_action("act_stroke") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied_with_details(TOOL, &error, &request_details);
                return Err(error);
            }
        };
        self.audit_action_started_with_details(
            TOOL,
            &json!({
                "request": request_details,
                "preflight": preflight,
            }),
        )?;
        let control = self.m2_rate_limit_control()?;
        let override_readback = control
            .override_backend(ResolvedBackend::Software, 0, 0)
            .map_err(diagnostic_action_error_to_mcp)?;
        let response = ActionDiagnosticRateLimitOverrideResponse {
            backend: override_readback.backend.as_str().to_owned(),
            ttl_ms: params.ttl_ms,
            before: token_bucket_readback(override_readback.before),
            after: token_bucket_readback(override_readback.after),
            reset_scheduled: true,
        };
        schedule_rate_limit_reset(control, ResolvedBackend::Software, params.ttl_ms);
        self.audit_action_ok_with_details(
            TOOL,
            &json!({
                "response": response,
                "preflight": preflight,
            }),
        )?;
        Ok(Json(response))
    }

    #[tool(
        description = "FSV diagnostic: saturate the real bounded action queue behind a long software blocker so the next real act_stroke proves ACTION_QUEUE_FULL through the normal MCP action path"
    )]
    pub async fn action_diagnostic_queue_full_setup(
        &self,
        params: Parameters<ActionDiagnosticQueueFullSetupParams>,
    ) -> Result<Json<ActionDiagnosticQueueFullSetupResponse>, ErrorData> {
        let params = params.0;
        const TOOL: &str = "action_diagnostic_queue_full_setup";
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TOOL,
            "tool.invocation kind=action_diagnostic_queue_full_setup"
        );
        let request_details = json!({
            "backend": ResolvedBackend::Software.as_str(),
            "expected_queue_capacity": ACTION_QUEUE_CAPACITY,
            "blocker_duration_ms": params.blocker_duration_ms,
        });
        if let Err(error) =
            require_diagnostic_confirm(&params.confirm, ACTION_DIAGNOSTIC_QUEUE_FULL_CONFIRM, TOOL)
        {
            self.audit_action_denied_with_details(TOOL, &error, &request_details);
            return Err(error);
        }
        if let Err(error) = validate_queue_blocker_duration_ms(params.blocker_duration_ms) {
            self.audit_action_denied_with_details(TOOL, &error, &request_details);
            return Err(error);
        }
        let preflight = match self.ensure_supported_use_allows_action("act_stroke") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied_with_details(TOOL, &error, &request_details);
                return Err(error);
            }
        };
        self.audit_action_started_with_details(
            TOOL,
            &json!({
                "request": request_details,
                "preflight": preflight,
            }),
        )?;
        let (handle, recording, _connection_closed_cancel) =
            self.m2_action_context_for_session_id(None)?;
        if recording.is_some() {
            let error = mcp_error(
                error_codes::ACTION_BACKEND_UNAVAILABLE,
                "action_diagnostic_queue_full_setup requires the real action emitter, not the recording backend",
            );
            self.audit_action_error_with_details(TOOL, &error, &request_details)?;
            return Err(error);
        }
        let from = synapse_action::backend::software::cursor_position()
            .map_err(diagnostic_action_error_to_mcp)?;
        let to = diagnostic_adjacent_point(from);
        handle
            .try_execute(diagnostic_queue_blocker_action(
                from,
                to,
                params.blocker_duration_ms,
            ))
            .map_err(diagnostic_action_error_to_mcp)?;
        tokio::time::sleep(Duration::from_millis(ACTION_DIAGNOSTIC_QUEUE_SETTLE_MS)).await;
        let (filler_attempts, queued_fillers, queue_full_observed) =
            saturate_action_queue(&handle)?;
        if !queue_full_observed {
            let error = mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!(
                    "action_diagnostic_queue_full_setup failed to observe ACTION_QUEUE_FULL after {filler_attempts} attempts and {queued_fillers} queued fillers"
                ),
            );
            self.audit_action_error_with_details(TOOL, &error, &request_details)?;
            return Err(error);
        }
        let response = ActionDiagnosticQueueFullSetupResponse {
            backend: ResolvedBackend::Software.as_str().to_owned(),
            expected_queue_capacity: u32::try_from(ACTION_QUEUE_CAPACITY).unwrap_or(u32::MAX),
            blocker_duration_ms: params.blocker_duration_ms,
            blocker_from: from,
            blocker_to: to,
            blocker_queued: true,
            filler_attempts,
            queued_fillers,
            queue_full_observed,
            next_act_stroke_expected_error: error_codes::ACTION_QUEUE_FULL.to_owned(),
        };
        self.audit_action_ok_with_details(
            TOOL,
            &json!({
                "response": response,
                "preflight": preflight,
            }),
        )?;
        Ok(Json(response))
    }

    #[tool(description = "Release all held keyboard, mouse, and gamepad input state")]
    pub async fn release_all(
        &self,
        params: Parameters<ReleaseAllParams>,
    ) -> Result<Json<ReleaseAllResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "release_all",
            "tool.invocation kind=release_all"
        );
        let (handle, snapshot_handle, reflex_runtime) = self.m2_release_all_context()?;
        let result =
            release_all_with_handles(handle, snapshot_handle, reflex_runtime, params.0).await;
        self.audit_action_result_best_effort("release_all", &result);
        result.map(Json)
    }
}

fn foreground_lease_session_id(
    request_context: &RequestContext<RoleServer>,
) -> Result<Option<String>, ErrorData> {
    super::context::mcp_session_id_from_request_context(request_context)
}

fn acquire_tool_foreground_input_lease(
    tool: &'static str,
    request_context: &RequestContext<RoleServer>,
) -> Result<crate::m2::ForegroundInputLeaseGuard, ErrorData> {
    acquire_tool_foreground_input_lease_with_ttl(
        tool,
        request_context,
        synapse_action::DEFAULT_LEASE_TTL_MS,
    )
}

fn acquire_tool_foreground_input_lease_with_ttl(
    tool: &'static str,
    request_context: &RequestContext<RoleServer>,
    ttl_ms: u64,
) -> Result<crate::m2::ForegroundInputLeaseGuard, ErrorData> {
    let session_id = foreground_lease_session_id(request_context)?;
    crate::m2::acquire_foreground_input_lease_with_ttl(tool, session_id.as_deref(), ttl_ms)
}

fn lease_ttl_for_hold_ms(hold_ms: u32) -> u64 {
    crate::m2::foreground_input_lease_ttl_for_hold_ms(hold_ms)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ClickDeltaSignature {
    foreground_hwnd: i64,
    foreground_pid: u32,
    foreground_process: String,
    foreground_title: String,
    foreground_title_sha256: Option<String>,
    focused_element_id: Option<String>,
    focused_role: Option<String>,
    focused_name_sha256: Option<String>,
    focused_value_sha256: Option<String>,
    focused_bbox: Option<Rect>,
    element_count: usize,
    elements_sha256: String,
    cdp_status: Option<String>,
    cdp_endpoint_present: bool,
    web_path: Option<String>,
    cursor_position: Option<Point>,
    pixel: ClickPixelSignature,
    point_pixel: Option<ClickPixelSignature>,
}

#[derive(Clone, Debug)]
struct ForegroundChangePolicy {
    allow: bool,
    expected_process_regex: Option<regex::Regex>,
    expected_process_pattern: Option<String>,
    expected_title_regex: Option<regex::Regex>,
    expected_title_pattern: Option<String>,
}

impl ForegroundChangePolicy {
    fn reject() -> Self {
        Self {
            allow: false,
            expected_process_regex: None,
            expected_process_pattern: None,
            expected_title_regex: None,
            expected_title_pattern: None,
        }
    }

    fn has_expectations(&self) -> bool {
        self.expected_process_regex.is_some() || self.expected_title_regex.is_some()
    }
}

#[derive(Clone, Debug)]
struct ActTypeTextReadback {
    signature: ActTypeTextSignature,
    value: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ActTypeTextSignature {
    foreground_hwnd: i64,
    foreground_pid: u32,
    foreground_process: String,
    foreground_title_sha256: Option<String>,
    focused_element_id: Option<String>,
    focused_role: Option<String>,
    focused_name_sha256: Option<String>,
    focused_value_len: Option<usize>,
    focused_value_sha256: Option<String>,
    focused_selected_text_sha256: Option<String>,
    focused_bbox: Option<Rect>,
    readback_source: Option<String>,
    has_text_readback: bool,
}

#[derive(Clone, Debug)]
struct ActTypeFocusedTextCandidate {
    element_id: String,
    role: String,
    name: String,
    selected_text: Option<String>,
    bbox: Rect,
    value: Option<String>,
    patterns: Vec<UiaPattern>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ClickPixelSignature {
    status: String,
    region: Rect,
    bitmap_sha256: Option<String>,
    detail: Option<String>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ClickElementFingerprint {
    element_id: String,
    role: String,
    automation_id: Option<String>,
    name_sha256: Option<String>,
    value_sha256: Option<String>,
    bbox: Rect,
    enabled: bool,
    focused: bool,
}

impl SynapseService {
    async fn act_click_with_verified_router(
        &self,
        handle: ActionHandle,
        recording: Option<Arc<RecordingBackend>>,
        params: ActClickParams,
        before: ClickDeltaSignature,
        verify_timeout_ms: u32,
        foreground_lease_session_id: Option<String>,
    ) -> Result<ActClickResponse, ErrorData> {
        match act_click_with_handle_and_lease(
            handle.clone(),
            recording.clone(),
            params.clone(),
            foreground_lease_session_id.as_deref(),
        )
        .await
        {
            Ok(response) => {
                match self
                    .verify_click_response(response, before.clone(), verify_timeout_ms)
                    .await
                {
                    Ok(response) => Ok(response),
                    Err(error)
                        if can_route_click_element_background_first(
                            &params,
                            recording.as_ref(),
                        ) && should_try_next_click_tier(&error) =>
                    {
                        let tier_attempts = click_tier_attempts_from_error(&error);
                        self.act_click_try_postmessage_then_foreground(
                            handle,
                            recording,
                            params,
                            before,
                            verify_timeout_ms,
                            tier_attempts,
                            foreground_lease_session_id,
                        )
                        .await
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error)
                if can_route_click_element_background_first(&params, recording.as_ref())
                    && should_try_next_click_tier(&error) =>
            {
                let tier_attempts = click_tier_attempts_from_error(&error);
                self.act_click_try_postmessage_then_foreground(
                    handle,
                    recording,
                    params,
                    before,
                    verify_timeout_ms,
                    tier_attempts,
                    foreground_lease_session_id,
                )
                .await
            }
            Err(error) => Err(error),
        }
    }

    async fn act_click_try_postmessage_then_foreground(
        &self,
        handle: ActionHandle,
        recording: Option<Arc<RecordingBackend>>,
        params: ActClickParams,
        before: ClickDeltaSignature,
        verify_timeout_ms: u32,
        tier_attempts: Vec<ActClickTierAttempt>,
        foreground_lease_session_id: Option<String>,
    ) -> Result<ActClickResponse, ErrorData> {
        match act_click_postmessage_with_params(&params, tier_attempts).await {
            Ok(response) => {
                match self
                    .verify_click_response(response, before.clone(), verify_timeout_ms)
                    .await
                {
                    Ok(response) => Ok(response),
                    Err(error) if should_try_next_click_tier(&error) => {
                        let tier_attempts = click_tier_attempts_from_error(&error);
                        self.act_click_try_foreground(
                            handle,
                            recording,
                            params,
                            before,
                            verify_timeout_ms,
                            tier_attempts,
                            foreground_lease_session_id,
                        )
                        .await
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) if should_try_next_click_tier(&error) => {
                let tier_attempts = click_tier_attempts_from_error(&error);
                self.act_click_try_foreground(
                    handle,
                    recording,
                    params,
                    before,
                    verify_timeout_ms,
                    tier_attempts,
                    foreground_lease_session_id,
                )
                .await
            }
            Err(error) => Err(error),
        }
    }

    async fn act_click_try_foreground(
        &self,
        handle: ActionHandle,
        recording: Option<Arc<RecordingBackend>>,
        mut params: ActClickParams,
        before: ClickDeltaSignature,
        verify_timeout_ms: u32,
        prior_attempts: Vec<ActClickTierAttempt>,
        foreground_lease_session_id: Option<String>,
    ) -> Result<ActClickResponse, ErrorData> {
        params.use_invoke_pattern = false;
        match act_click_with_handle_and_lease(
            handle,
            recording,
            params,
            foreground_lease_session_id.as_deref(),
        )
        .await
        {
            Ok(mut response) => {
                let current_attempts = std::mem::take(&mut response.tier_attempts);
                response.tier_attempts =
                    merge_click_tier_attempts(prior_attempts, current_attempts);
                self.verify_click_response(response, before, verify_timeout_ms)
                    .await
            }
            Err(error) => {
                let mut tier_attempts = prior_attempts;
                tier_attempts.extend(click_tier_attempts_from_error(&error));
                Err(attach_click_tier_attempts(error, tier_attempts))
            }
        }
    }

    async fn verify_click_response(
        &self,
        mut response: ActClickResponse,
        before: ClickDeltaSignature,
        verify_timeout_ms: u32,
    ) -> Result<ActClickResponse, ErrorData> {
        match self.verify_click_delta(before, verify_timeout_ms).await {
            Ok(postcondition) => {
                response.postcondition = postcondition;
                Ok(response)
            }
            Err(error) => {
                let mut tier_attempts = response.tier_attempts.clone();
                let error_code = click_error_data_code(&error)
                    .unwrap_or(error_codes::ACTION_NO_OBSERVED_DELTA)
                    .to_owned();
                tier_attempts.push(click_tier_failed(
                    response.backend_tier_used.clone(),
                    CLICK_REASON_NO_OBSERVED_DELTA,
                    &error_code,
                    response.required_foreground,
                    error.message.to_string(),
                ));
                Err(attach_click_tier_attempts(error, tier_attempts))
            }
        }
    }

    async fn capture_click_delta_signature(
        &self,
        max_elements: usize,
    ) -> Result<ClickDeltaSignature, ErrorData> {
        self.capture_action_delta_signature(max_elements, None, false)
            .await
    }

    async fn capture_act_type_text_signature(
        &self,
        max_elements: usize,
    ) -> Result<ActTypeTextReadback, ErrorData> {
        let mut input = {
            let state = self.m1_state()?;
            crate::m1::current_input(&state, 6)?
        };
        crate::m1::enrich_input_with_cdp(&mut input, 6, max_elements).await;
        crate::m1::enrich_input_with_browser_ocr(&mut input, max_elements);

        let focused = focused_text_candidate(input.focused.as_ref(), &input.elements);
        let (value, readback_source) = focused_text_readback(focused.as_ref());
        let signature = ActTypeTextSignature {
            foreground_hwnd: input.foreground.hwnd,
            foreground_pid: input.foreground.pid,
            foreground_process: input.foreground.process_name,
            foreground_title_sha256: non_empty_sha256(&input.foreground.window_title),
            focused_element_id: focused.as_ref().map(|item| item.element_id.clone()),
            focused_role: focused.as_ref().map(|item| item.role.clone()),
            focused_name_sha256: focused
                .as_ref()
                .and_then(|item| non_empty_sha256(&item.name)),
            focused_value_len: value.as_ref().map(|value| value.chars().count()),
            focused_value_sha256: value.as_deref().and_then(non_empty_sha256),
            focused_selected_text_sha256: focused
                .as_ref()
                .and_then(|item| item.selected_text.as_deref())
                .and_then(non_empty_sha256),
            focused_bbox: focused.as_ref().map(|item| item.bbox),
            readback_source: readback_source.map(str::to_owned),
            has_text_readback: value.is_some(),
        };
        if value.is_none() {
            let signature_hash = verify_hash_json(&signature)?;
            return Err(postcondition_failed_error(
                "act_type",
                "foreground_focused_text_value",
                "focused element does not expose a UIA Value/Text readback for fail-closed text verification",
                signature_hash.clone(),
                signature_hash,
                json!({
                    "readback": signature,
                }),
            ));
        }

        Ok(ActTypeTextReadback { signature, value })
    }

    async fn capture_action_delta_signature(
        &self,
        max_elements: usize,
        point_region: Option<Point>,
        include_cursor: bool,
    ) -> Result<ClickDeltaSignature, ErrorData> {
        let mut input = {
            let state = self.m1_state()?;
            crate::m1::current_input(&state, 6)?
        };
        crate::m1::enrich_input_with_cdp(&mut input, 6, max_elements).await;
        crate::m1::enrich_input_with_browser_ocr(&mut input, max_elements);

        let focused = input.focused.clone();
        let elements_sha256 = elements_fingerprint_hash(&input.elements)?;
        let foreground_title_sha256 = non_empty_sha256(&input.foreground.window_title);
        let pixel = capture_pixel_signature(input.foreground.window_bounds);
        Ok(ClickDeltaSignature {
            foreground_hwnd: input.foreground.hwnd,
            foreground_pid: input.foreground.pid,
            foreground_process: input.foreground.process_name,
            foreground_title: input.foreground.window_title,
            foreground_title_sha256,
            focused_element_id: focused.as_ref().map(|item| item.element_id.to_string()),
            focused_role: focused.as_ref().map(|item| item.role.clone()),
            focused_name_sha256: focused
                .as_ref()
                .and_then(|item| non_empty_sha256(&item.name)),
            focused_value_sha256: focused
                .as_ref()
                .and_then(|item| item.value.as_deref())
                .and_then(non_empty_sha256),
            focused_bbox: focused.as_ref().map(|item| item.bbox),
            element_count: input.elements.len(),
            elements_sha256,
            cdp_status: input.cdp.as_ref().map(|cdp| cdp.status.as_str().to_owned()),
            cdp_endpoint_present: input.cdp.as_ref().is_some_and(|cdp| cdp.endpoint.is_some()),
            web_path: input.web_path.map(|path| path.as_str().to_owned()),
            cursor_position: include_cursor
                .then(|| synapse_action::backend::software::cursor_position().ok())
                .flatten(),
            pixel,
            point_pixel: point_region.map(|point| capture_pixel_signature(point_delta_rect(point))),
        })
    }

    async fn verify_click_delta(
        &self,
        before: ClickDeltaSignature,
        timeout_ms: u32,
    ) -> Result<ActClickPostcondition, ErrorData> {
        tokio::time::sleep(Duration::from_millis(u64::from(timeout_ms))).await;
        let after = self
            .capture_action_delta_signature(160, None, false)
            .await?;
        let before_hash = signature_hash(&before)?;
        let after_hash = signature_hash(&after)?;
        if foreground_identity_changed(&before, &after) {
            return Err(foreground_lost_delta_error(
                "act_click",
                "foreground_focused_ui_or_pixels",
                timeout_ms,
                &before_hash,
                &after_hash,
                &before,
                &after,
            ));
        }
        if before == after {
            return Err(source_no_observed_delta_error(
                "act_click",
                "foreground_focused_ui_or_pixels",
                timeout_ms,
                before_hash,
                after_hash,
                json!({
                    "before": before,
                    "after": after,
                }),
            ));
        }
        Ok(postcondition_observed_delta(
            "act_click",
            "foreground_focused_ui_or_pixels",
            before_hash,
            after_hash,
            "observed a changed focused/UI/pixel signature after delivery",
        ))
    }

    async fn verify_act_type_response(
        &self,
        mut response: ActTypeResponse,
        before: ActTypeTextReadback,
        verify_timeout_ms: u32,
        emitted: &str,
    ) -> Result<ActTypeResponse, ErrorData> {
        tokio::time::sleep(Duration::from_millis(u64::from(verify_timeout_ms))).await;
        let after = self.capture_act_type_text_signature(160).await?;
        let before_hash = verify_hash_json(&before.signature)?;
        let after_hash = verify_hash_json(&after.signature)?;
        if act_type_foreground_identity_changed(&before.signature, &after.signature) {
            return Err(act_type_text_foreground_lost_error(
                verify_timeout_ms,
                &before_hash,
                &after_hash,
                &before.signature,
                &after.signature,
            ));
        }
        if before.signature.focused_element_id != after.signature.focused_element_id {
            return Err(postcondition_failed_error(
                "act_type",
                "foreground_focused_text_value",
                "focused text target changed before postcondition readback",
                before_hash,
                after_hash,
                json!({
                    "before": before.signature,
                    "after": after.signature,
                }),
            ));
        }
        if before.value == after.value {
            return Err(source_no_observed_delta_error(
                "act_type",
                "foreground_focused_text_value",
                verify_timeout_ms,
                before_hash,
                after_hash,
                json!({
                    "before": before.signature,
                    "after": after.signature,
                }),
            ));
        }
        let after_value = after.value.as_deref().unwrap_or_default();
        if !normalized_text_contains(after_value, emitted) {
            return Err(postcondition_failed_error(
                "act_type",
                "foreground_focused_text_value",
                "focused text value changed but did not contain the emitted text",
                before_hash,
                after_hash,
                json!({
                    "expected_emitted_len": emitted.chars().count(),
                    "expected_emitted_sha256": text_sha256(emitted),
                    "before": before.signature,
                    "after": after.signature,
                }),
            ));
        }
        response.postcondition = postcondition_observed_delta(
            "act_type",
            "foreground_focused_text_value",
            before_hash,
            after_hash,
            "observed focused text value changed and containing emitted text after delivery",
        );
        response.target_readback_required = false;
        response.target_text_integrity = "foreground_focused_text_value_readback".to_owned();
        Ok(response)
    }

    async fn verify_act_press_response(
        &self,
        mut response: ActPressResponse,
        before: ClickDeltaSignature,
        verify_timeout_ms: u32,
        foreground_change_policy: ForegroundChangePolicy,
    ) -> Result<ActPressResponse, ErrorData> {
        response.postcondition = self
            .verify_action_delta(
                "act_press",
                "foreground_focused_ui_or_pixels",
                before,
                verify_timeout_ms,
                None,
                foreground_change_policy,
            )
            .await?;
        Ok(response)
    }

    async fn verify_act_scroll_response(
        &self,
        mut response: ActScrollResponse,
        before: ClickDeltaSignature,
        verify_timeout_ms: u32,
        point_region: Option<Point>,
    ) -> Result<ActScrollResponse, ErrorData> {
        response.postcondition = self
            .verify_action_delta(
                "act_scroll",
                if point_region.is_some() {
                    "target_point_pixels"
                } else {
                    "foreground_focused_ui_or_pixels"
                },
                before,
                verify_timeout_ms,
                point_region,
                ForegroundChangePolicy::reject(),
            )
            .await?;
        Ok(response)
    }

    async fn verify_act_stroke_response(
        &self,
        mut response: ActStrokeResponse,
        before: ClickDeltaSignature,
        verify_timeout_ms: u32,
    ) -> Result<ActStrokeResponse, ErrorData> {
        response.postcondition = self
            .verify_action_delta(
                "act_stroke",
                "cursor_foreground_ui_or_pixels",
                before,
                verify_timeout_ms,
                None,
                ForegroundChangePolicy::reject(),
            )
            .await?;
        Ok(response)
    }

    async fn verify_action_delta(
        &self,
        tool: &str,
        source_of_truth: &str,
        before: ClickDeltaSignature,
        timeout_ms: u32,
        point_region: Option<Point>,
        foreground_change_policy: ForegroundChangePolicy,
    ) -> Result<ActPostcondition, ErrorData> {
        tokio::time::sleep(Duration::from_millis(u64::from(timeout_ms))).await;
        let after = self
            .capture_action_delta_signature(160, point_region, source_of_truth.contains("cursor"))
            .await?;
        verify_captured_action_delta(
            tool,
            source_of_truth,
            timeout_ms,
            before,
            after,
            point_region,
            foreground_change_policy,
        )
    }

    async fn verify_act_pad_response(
        &self,
        mut response: ActPadResponse,
        before: ActionStateSnapshot,
        snapshot_handle: ActionEmitterSnapshotHandle,
        verify_timeout_ms: u32,
    ) -> Result<ActPadResponse, ErrorData> {
        tokio::time::sleep(Duration::from_millis(u64::from(verify_timeout_ms))).await;
        let after = snapshot_handle
            .snapshot()
            .await
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        let before_hash = verify_hash_json(&before.pad_state)?;
        let after_hash = verify_hash_json(&after.pad_state)?;
        if before.pad_state == after.pad_state {
            return Err(source_no_observed_delta_error(
                "act_pad",
                "action_emitter.pad_state",
                verify_timeout_ms,
                before_hash,
                after_hash,
                json!({
                    "before_pad_state": before.pad_state,
                    "after_pad_state": after.pad_state,
                }),
            ));
        }
        response.postcondition = postcondition_observed_delta(
            "act_pad",
            "action_emitter.pad_state",
            before_hash,
            after_hash,
            "observed action emitter pad_state change after delivery",
        );
        Ok(response)
    }
}

fn verify_captured_action_delta(
    tool: &str,
    source_of_truth: &str,
    timeout_ms: u32,
    before: ClickDeltaSignature,
    after: ClickDeltaSignature,
    point_region: Option<Point>,
    foreground_change_policy: ForegroundChangePolicy,
) -> Result<ActPostcondition, ErrorData> {
    if point_region.is_some() {
        let before_point = before.point_pixel.clone();
        let after_point = after.point_pixel.clone();
        let before_hash = verify_hash_json(&before_point)?;
        let after_hash = verify_hash_json(&after_point)?;
        if before_point == after_point {
            return Err(source_no_observed_delta_error(
                tool,
                source_of_truth,
                timeout_ms,
                before_hash,
                after_hash,
                json!({
                    "point": point_region,
                    "before_point_pixel": before_point,
                    "after_point_pixel": after_point,
                }),
            ));
        }
        return Ok(postcondition_observed_delta(
            tool,
            source_of_truth,
            before_hash,
            after_hash,
            "observed a target point pixel signature change after delivery",
        ));
    }

    let before_hash = signature_hash(&before)?;
    let after_hash = signature_hash(&after)?;
    if foreground_identity_changed(&before, &after) {
        return verify_foreground_transition(
            tool,
            source_of_truth,
            timeout_ms,
            before_hash,
            after_hash,
            &before,
            &after,
            &foreground_change_policy,
        );
    }
    if foreground_change_policy.has_expectations() {
        return Err(postcondition_failed_error(
            tool,
            source_of_truth,
            "expected foreground transition did not occur",
            before_hash,
            after_hash,
            json!({
                "before": before,
                "after": after,
                "foreground_change_policy": foreground_change_policy_readback(&foreground_change_policy),
            }),
        ));
    }
    if before == after {
        return Err(source_no_observed_delta_error(
            tool,
            source_of_truth,
            timeout_ms,
            before_hash,
            after_hash,
            json!({
                "before": before,
                "after": after,
            }),
        ));
    }
    Ok(postcondition_observed_delta(
        tool,
        source_of_truth,
        before_hash,
        after_hash,
        "observed a Source-of-Truth signature change after delivery",
    ))
}

fn act_press_foreground_change_policy(
    params: &ActPressParams,
) -> Result<ForegroundChangePolicy, ErrorData> {
    let has_process_expectation = params.expected_foreground_process_regex.is_some();
    let has_title_expectation = params.expected_foreground_title_regex.is_some();
    let has_expectation = has_process_expectation || has_title_expectation;

    if !params.verify_delta && (params.allow_foreground_change || has_expectation) {
        return Err(act_press_policy_params_invalid(
            "verify_delta",
            "act_press foreground-change verification policy requires verify_delta=true",
            "verify_delta_required",
        ));
    }

    if !params.allow_foreground_change && has_expectation {
        return Err(act_press_policy_params_invalid(
            "allow_foreground_change",
            "expected foreground regex fields require allow_foreground_change=true",
            "allow_foreground_change_required",
        ));
    }

    if !params.allow_foreground_change {
        return Ok(ForegroundChangePolicy::reject());
    }

    let expected_process_pattern = params.expected_foreground_process_regex.clone();
    let expected_title_pattern = params.expected_foreground_title_regex.clone();
    let expected_process_regex = compile_act_press_policy_regex(
        "expected_foreground_process_regex",
        expected_process_pattern.as_deref(),
    )?;
    let expected_title_regex = compile_act_press_policy_regex(
        "expected_foreground_title_regex",
        expected_title_pattern.as_deref(),
    )?;

    Ok(ForegroundChangePolicy {
        allow: true,
        expected_process_regex,
        expected_process_pattern,
        expected_title_regex,
        expected_title_pattern,
    })
}

fn compile_act_press_policy_regex(
    field: &'static str,
    pattern: Option<&str>,
) -> Result<Option<regex::Regex>, ErrorData> {
    let Some(pattern) = pattern else {
        return Ok(None);
    };
    if pattern.trim().is_empty() {
        return Err(act_press_policy_params_invalid(
            field,
            format!("{field} must not be empty"),
            "empty_expected_foreground_regex",
        ));
    }
    regex::Regex::new(pattern)
        .map(Some)
        .map_err(|error| act_press_policy_invalid_regex(field, pattern, &error))
}

fn verify_foreground_transition(
    tool: &str,
    source_of_truth: &str,
    timeout_ms: u32,
    before_hash: String,
    after_hash: String,
    before: &ClickDeltaSignature,
    after: &ClickDeltaSignature,
    policy: &ForegroundChangePolicy,
) -> Result<ActPostcondition, ErrorData> {
    if !policy.allow {
        return Err(foreground_lost_delta_error(
            tool,
            source_of_truth,
            timeout_ms,
            &before_hash,
            &after_hash,
            before,
            after,
        ));
    }

    let process_matches = policy
        .expected_process_regex
        .as_ref()
        .map_or(true, |regex| regex.is_match(&after.foreground_process));
    let title_matches = policy
        .expected_title_regex
        .as_ref()
        .map_or(true, |regex| regex.is_match(&after.foreground_title));

    if !process_matches || !title_matches {
        return Err(foreground_change_policy_mismatch_error(
            tool,
            source_of_truth,
            timeout_ms,
            &before_hash,
            &after_hash,
            before,
            after,
            policy,
            process_matches,
            title_matches,
        ));
    }

    tracing::info!(
        code = "ACTION_FOREGROUND_CHANGE_ACCEPTED",
        tool,
        source_of_truth,
        timeout_ms,
        before_hwnd = before.foreground_hwnd,
        after_hwnd = after.foreground_hwnd,
        before_pid = before.foreground_pid,
        after_pid = after.foreground_pid,
        before_process = %before.foreground_process,
        after_process = %after.foreground_process,
        after_title_sha256 = ?after.foreground_title_sha256,
        expected_process_regex = ?policy.expected_process_pattern,
        expected_title_regex = ?policy.expected_title_pattern,
        before_signature = before_hash,
        after_signature = after_hash,
        "verify_delta accepted declared foreground target transition"
    );

    Ok(postcondition_observed_delta(
        tool,
        source_of_truth,
        before_hash,
        after_hash,
        format!(
            "observed expected foreground transition after delivery; before_hwnd=0x{:x}; after_hwnd=0x{:x}; after_process={}; after_title_sha256={}; expected_process_regex_present={}; expected_title_regex_present={}",
            before.foreground_hwnd,
            after.foreground_hwnd,
            after.foreground_process,
            after.foreground_title_sha256.as_deref().unwrap_or("none"),
            policy.expected_process_regex.is_some(),
            policy.expected_title_regex.is_some()
        ),
    ))
}

fn foreground_change_policy_readback(policy: &ForegroundChangePolicy) -> Value {
    json!({
        "allow_foreground_change": policy.allow,
        "expected_foreground_process_regex": policy.expected_process_pattern,
        "expected_foreground_title_regex": policy.expected_title_pattern,
    })
}

fn act_press_policy_params_invalid(
    field: &'static str,
    detail: impl Into<String>,
    reason: &'static str,
) -> ErrorData {
    let detail = detail.into();
    tracing::error!(
        code = error_codes::TOOL_PARAMS_INVALID,
        tool = "act_press",
        field,
        reason,
        detail = %detail,
        "act_press foreground-change policy parameters invalid"
    );
    ErrorData::new(
        ErrorCode(-32099),
        detail.clone(),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "tool": "act_press",
            "field": field,
            "reason": reason,
            "detail": detail,
        })),
    )
}

fn act_press_policy_invalid_regex(
    field: &'static str,
    pattern: &str,
    error: &regex::Error,
) -> ErrorData {
    let detail = format!("{field} is not a valid regex: {error}");
    tracing::error!(
        code = error_codes::TOOL_PARAMS_INVALID,
        tool = "act_press",
        field,
        reason = "invalid_expected_foreground_regex",
        pattern,
        regex_error = %error,
        "act_press foreground-change policy regex invalid"
    );
    ErrorData::new(
        ErrorCode(-32099),
        detail.clone(),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "tool": "act_press",
            "field": field,
            "reason": "invalid_expected_foreground_regex",
            "pattern": pattern,
            "detail": detail,
        })),
    )
}

fn can_route_click_element_background_first(
    params: &ActClickParams,
    recording: Option<&Arc<RecordingBackend>>,
) -> bool {
    recording.is_none() && click_params_can_route_background_first(params)
}

fn should_try_next_click_tier(error: &ErrorData) -> bool {
    matches!(
        click_error_data_code(error),
        Some(
            error_codes::ACTION_ELEMENT_PATTERN_UNSUPPORTED
                | error_codes::ACTION_NO_OBSERVED_DELTA
                | error_codes::ACTION_BACKEND_UNAVAILABLE
        )
    )
}

fn click_error_data_code(error: &ErrorData) -> Option<&str> {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
}

fn click_tier_attempts_from_error(error: &ErrorData) -> Vec<ActClickTierAttempt> {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("tier_attempts"))
        .cloned()
        .and_then(|attempts| serde_json::from_value(attempts).ok())
        .unwrap_or_default()
}

fn merge_click_tier_attempts(
    mut prior_attempts: Vec<ActClickTierAttempt>,
    current_attempts: Vec<ActClickTierAttempt>,
) -> Vec<ActClickTierAttempt> {
    prior_attempts.extend(current_attempts);
    prior_attempts
}

fn elements_fingerprint_hash(elements: &[AccessibleNode]) -> Result<String, ErrorData> {
    let fingerprints: Vec<_> = elements
        .iter()
        .take(160)
        .map(|node| ClickElementFingerprint {
            element_id: node.element_id.to_string(),
            role: node.role.clone(),
            automation_id: node.automation_id.clone(),
            name_sha256: non_empty_sha256(&node.name),
            value_sha256: node.value.as_deref().and_then(non_empty_sha256),
            bbox: node.bbox,
            enabled: node.enabled,
            focused: node.focused,
        })
        .collect();
    hash_json(&fingerprints)
}

fn capture_pixel_signature(region: Rect) -> ClickPixelSignature {
    if region.w <= 0 || region.h <= 0 {
        return ClickPixelSignature {
            status: "unavailable".to_owned(),
            region,
            bitmap_sha256: None,
            detail: Some("foreground window bounds are empty".to_owned()),
        };
    }
    match synapse_capture::screen_region_to_bgra_bitmap(region) {
        Ok(bitmap) => {
            let mut hasher = Sha256::new();
            hasher.update(bitmap.width.to_le_bytes());
            hasher.update(bitmap.height.to_le_bytes());
            hasher.update(&bitmap.bytes);
            ClickPixelSignature {
                status: "ok".to_owned(),
                region: bitmap.region,
                bitmap_sha256: Some(hex_encode(&hasher.finalize())),
                detail: Some(format!(
                    "captured {}x{} BGRA bytes={}",
                    bitmap.width,
                    bitmap.height,
                    bitmap.bytes.len()
                )),
            }
        }
        Err(error) => ClickPixelSignature {
            status: "unavailable".to_owned(),
            region,
            bitmap_sha256: None,
            detail: Some(error.to_string()),
        },
    }
}

fn point_delta_rect(point: Point) -> Rect {
    // Wide list rows often leave blank padding under the pointer; include nearby content.
    const HALF_WIDTH: i32 = 512;
    const HALF_HEIGHT: i32 = 192;
    Rect {
        x: point.x.saturating_sub(HALF_WIDTH),
        y: point.y.saturating_sub(HALF_HEIGHT),
        w: HALF_WIDTH * 2,
        h: HALF_HEIGHT * 2,
    }
}

fn signature_hash(signature: &ClickDeltaSignature) -> Result<String, ErrorData> {
    hash_json(signature)
}

fn focused_text_candidate(
    focused: Option<&FocusedElement>,
    elements: &[AccessibleNode],
) -> Option<ActTypeFocusedTextCandidate> {
    if let Some(node) = elements
        .iter()
        .find(|node| node.focused && has_text_readback_pattern(&node.patterns))
    {
        return Some(ActTypeFocusedTextCandidate {
            element_id: node.element_id.to_string(),
            role: node.role.clone(),
            name: node.name.clone(),
            selected_text: None,
            bbox: node.bbox,
            value: node.value.clone(),
            patterns: node.patterns.clone(),
        });
    }
    focused.map(|focused| ActTypeFocusedTextCandidate {
        element_id: focused.element_id.to_string(),
        role: focused.role.clone(),
        name: focused.name.clone(),
        selected_text: focused.selected_text.clone(),
        bbox: focused.bbox,
        value: focused.value.clone(),
        patterns: focused.patterns.clone(),
    })
}

fn focused_text_readback(
    focused: Option<&ActTypeFocusedTextCandidate>,
) -> (Option<String>, Option<&'static str>) {
    let Some(focused) = focused else {
        return (None, None);
    };
    if let Some(value) = &focused.value {
        return (Some(value.clone()), Some("uia_focused_value"));
    }
    if has_text_readback_pattern(&focused.patterns) {
        return (Some(String::new()), Some("uia_focused_empty_value_or_text"));
    }
    (None, None)
}

fn has_text_readback_pattern(patterns: &[UiaPattern]) -> bool {
    patterns
        .iter()
        .any(|pattern| matches!(pattern, UiaPattern::Value | UiaPattern::Text))
}

fn normalized_text_contains(value: &str, emitted: &str) -> bool {
    if emitted.is_empty() {
        return false;
    }
    normalize_newlines(value).contains(&normalize_newlines(emitted))
}

fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

fn text_sha256(value: &str) -> String {
    hex_encode(&Sha256::digest(value.as_bytes()))
}

fn hash_json<T: Serialize>(value: &T) -> Result<String, ErrorData> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("failed to encode click delta signature: {error}"),
        )
    })?;
    Ok(hex_encode(&Sha256::digest(bytes)))
}

fn non_empty_sha256(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| hex_encode(&Sha256::digest(trimmed.as_bytes())))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn foreground_identity_changed(before: &ClickDeltaSignature, after: &ClickDeltaSignature) -> bool {
    before.foreground_hwnd != after.foreground_hwnd
        || before.foreground_pid != after.foreground_pid
        || before.foreground_process != after.foreground_process
}

fn act_type_foreground_identity_changed(
    before: &ActTypeTextSignature,
    after: &ActTypeTextSignature,
) -> bool {
    before.foreground_hwnd != after.foreground_hwnd
        || before.foreground_pid != after.foreground_pid
        || before.foreground_process != after.foreground_process
}

fn act_type_text_foreground_lost_error(
    timeout_ms: u32,
    before_hash: &str,
    after_hash: &str,
    before: &ActTypeTextSignature,
    after: &ActTypeTextSignature,
) -> ErrorData {
    tracing::warn!(
        code = error_codes::ACTION_FOREGROUND_LOST,
        tool = "act_type",
        source_of_truth = "foreground_focused_text_value",
        timeout_ms,
        before_hwnd = before.foreground_hwnd,
        after_hwnd = after.foreground_hwnd,
        before_pid = before.foreground_pid,
        after_pid = after.foreground_pid,
        before_process = %before.foreground_process,
        after_process = %after.foreground_process,
        before_signature = before_hash,
        after_signature = after_hash,
        "act_type text readback foreground target identity changed before postcondition readback"
    );
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "act_type text readback cannot accept observed delta because foreground target changed within {timeout_ms} ms"
        ),
        Some(json!({
            "code": error_codes::ACTION_FOREGROUND_LOST,
            "tool": "act_type",
            "source_of_truth": "foreground_focused_text_value",
            "verify_delta": {
                "timeout_ms": timeout_ms,
                "before_signature": before_hash,
                "after_signature": after_hash,
                "before": before,
                "after": after,
            }
        })),
    )
}

fn foreground_lost_delta_error(
    tool: &str,
    source_of_truth: &str,
    timeout_ms: u32,
    before_hash: &str,
    after_hash: &str,
    before: &ClickDeltaSignature,
    after: &ClickDeltaSignature,
) -> ErrorData {
    tracing::warn!(
        code = error_codes::ACTION_FOREGROUND_LOST,
        tool,
        source_of_truth,
        timeout_ms,
        before_hwnd = before.foreground_hwnd,
        after_hwnd = after.foreground_hwnd,
        before_pid = before.foreground_pid,
        after_pid = after.foreground_pid,
        before_process = %before.foreground_process,
        after_process = %after.foreground_process,
        before_title_sha256 = ?before.foreground_title_sha256,
        after_title_sha256 = ?after.foreground_title_sha256,
        before_signature = before_hash,
        after_signature = after_hash,
        "verify_delta foreground target identity changed before postcondition readback"
    );
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "{tool} verify_delta cannot accept observed delta because foreground target changed within {timeout_ms} ms"
        ),
        Some(json!({
            "code": error_codes::ACTION_FOREGROUND_LOST,
            "reason": "unexpected_foreground_change",
            "tool": tool,
            "source_of_truth": source_of_truth,
            "verify_delta": {
                "timeout_ms": timeout_ms,
                "before_signature": before_hash,
                "after_signature": after_hash,
                "before": before,
                "after": after,
            }
        })),
    )
}

fn foreground_change_policy_mismatch_error(
    tool: &str,
    source_of_truth: &str,
    timeout_ms: u32,
    before_hash: &str,
    after_hash: &str,
    before: &ClickDeltaSignature,
    after: &ClickDeltaSignature,
    policy: &ForegroundChangePolicy,
    process_matches: bool,
    title_matches: bool,
) -> ErrorData {
    tracing::error!(
        code = error_codes::ACTION_FOREGROUND_LOST,
        reason = "foreground_change_policy_mismatch",
        tool,
        source_of_truth,
        timeout_ms,
        before_hwnd = before.foreground_hwnd,
        after_hwnd = after.foreground_hwnd,
        before_pid = before.foreground_pid,
        after_pid = after.foreground_pid,
        before_process = %before.foreground_process,
        after_process = %after.foreground_process,
        before_title_sha256 = ?before.foreground_title_sha256,
        after_title_sha256 = ?after.foreground_title_sha256,
        expected_process_regex = ?policy.expected_process_pattern,
        expected_title_regex = ?policy.expected_title_pattern,
        process_matches,
        title_matches,
        before_signature = before_hash,
        after_signature = after_hash,
        "verify_delta foreground target changed but did not match declared policy"
    );
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "{tool} verify_delta foreground target changed but did not match expected foreground policy within {timeout_ms} ms"
        ),
        Some(json!({
            "code": error_codes::ACTION_FOREGROUND_LOST,
            "reason": "foreground_change_policy_mismatch",
            "tool": tool,
            "source_of_truth": source_of_truth,
            "foreground_change_policy": foreground_change_policy_readback(policy),
            "matches": {
                "process": process_matches,
                "title": title_matches,
            },
            "verify_delta": {
                "timeout_ms": timeout_ms,
                "before_signature": before_hash,
                "after_signature": after_hash,
                "before": before,
                "after": after,
            }
        })),
    )
}

fn action_preflight_details(preflight: &ActionPreflightReadback) -> Value {
    json!({
        "preflight": preflight,
    })
}

fn act_stroke_audit_details(stroke_details: &Value, preflight: &ActionPreflightReadback) -> Value {
    json!({
        "stroke": stroke_details,
        "preflight": preflight,
    })
}

fn act_stroke_failure_audit_details(
    stroke_details: &Value,
    preflight: &ActionPreflightReadback,
    error: &ErrorData,
) -> Value {
    json!({
        "stroke": stroke_details,
        "preflight": preflight,
        "failure": act_stroke_error_details(error),
    })
}

fn log_act_stroke_failure(details: &Value, error: &ErrorData) {
    let stroke = details.get("stroke").unwrap_or(&Value::Null);
    let failure = details.get("failure").unwrap_or(&Value::Null);
    let preflight = details.get("preflight").unwrap_or(&Value::Null);
    let error_code = failure
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    let path_id = stroke
        .get("path_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let path_kind = stroke
        .get("path_kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let backend_requested = stroke
        .get("backend_requested")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let backend_resolved = stroke
        .get("backend_resolved")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let point_index = failure.get("point_index").cloned().unwrap_or(Value::Null);
    let queue_rate_state = failure
        .get("queue_rate_state")
        .cloned()
        .unwrap_or_else(|| json!({ "kind": "not_rate_or_queue" }));
    let foreground_proof = preflight
        .get("after")
        .or_else(|| preflight.get("before"))
        .cloned()
        .unwrap_or(Value::Null);

    tracing::error!(
        code = error_code,
        detail = %error.message,
        path_id,
        path_kind,
        backend_requested,
        backend_resolved,
        point_index = ?point_index,
        queue_rate_state = ?queue_rate_state,
        foreground_proof = ?foreground_proof,
        fallback_path_executed = false,
        action_kind = "act_stroke",
        "act_stroke failed without fallback"
    );
}

fn require_diagnostic_confirm(
    actual: &str,
    expected: &'static str,
    tool: &'static str,
) -> Result<(), ErrorData> {
    if actual == expected {
        return Ok(());
    }
    Err(ErrorData::new(
        ErrorCode(-32099),
        format!("{tool} requires confirm=\"{expected}\""),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "detail_code": "ACTION_DIAGNOSTIC_CONFIRM_MISMATCH",
            "expected_confirm": expected,
            "actual_confirm_present": !actual.is_empty(),
        })),
    ))
}

fn validate_diagnostic_ttl_ms(ttl_ms: u64) -> Result<(), ErrorData> {
    if (ACTION_DIAGNOSTIC_MIN_TTL_MS..=ACTION_DIAGNOSTIC_MAX_TTL_MS).contains(&ttl_ms) {
        return Ok(());
    }
    Err(ErrorData::new(
        ErrorCode(-32099),
        format!(
            "diagnostic ttl_ms must be between {ACTION_DIAGNOSTIC_MIN_TTL_MS} and {ACTION_DIAGNOSTIC_MAX_TTL_MS}; got {ttl_ms}"
        ),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "detail_code": "ACTION_DIAGNOSTIC_TTL_OUT_OF_RANGE",
            "min_ttl_ms": ACTION_DIAGNOSTIC_MIN_TTL_MS,
            "max_ttl_ms": ACTION_DIAGNOSTIC_MAX_TTL_MS,
            "ttl_ms": ttl_ms,
        })),
    ))
}

fn validate_queue_blocker_duration_ms(duration_ms: u32) -> Result<(), ErrorData> {
    if (ACTION_DIAGNOSTIC_MIN_QUEUE_BLOCKER_MS..=ACTION_DIAGNOSTIC_MAX_QUEUE_BLOCKER_MS)
        .contains(&duration_ms)
    {
        return Ok(());
    }
    Err(ErrorData::new(
        ErrorCode(-32099),
        format!(
            "diagnostic blocker_duration_ms must be between {ACTION_DIAGNOSTIC_MIN_QUEUE_BLOCKER_MS} and {ACTION_DIAGNOSTIC_MAX_QUEUE_BLOCKER_MS}; got {duration_ms}"
        ),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "detail_code": "ACTION_DIAGNOSTIC_QUEUE_BLOCKER_DURATION_OUT_OF_RANGE",
            "min_blocker_duration_ms": ACTION_DIAGNOSTIC_MIN_QUEUE_BLOCKER_MS,
            "max_blocker_duration_ms": ACTION_DIAGNOSTIC_MAX_QUEUE_BLOCKER_MS,
            "blocker_duration_ms": duration_ms,
        })),
    ))
}

fn token_bucket_readback(snapshot: TokenBucketSnapshot) -> TokenBucketReadback {
    TokenBucketReadback {
        capacity: snapshot.capacity,
        tokens: snapshot.tokens,
        refill_rate_per_s: snapshot.refill_rate_per_s,
        last_refill_ns: snapshot.last_refill_ns,
    }
}

fn schedule_rate_limit_reset(
    control: synapse_action::BackendRateLimitControl,
    backend: ResolvedBackend,
    ttl_ms: u64,
) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(ttl_ms)).await;
        match control.reset_backend(backend) {
            Ok(readback) => {
                tracing::info!(
                    code = "ACTION_DIAGNOSTIC_RATE_LIMIT_RESET",
                    backend = readback.backend.as_str(),
                    before_capacity = readback.before.capacity,
                    before_tokens = readback.before.tokens,
                    before_refill_rate_per_s = readback.before.refill_rate_per_s,
                    after_capacity = readback.after.capacity,
                    after_tokens = readback.after.tokens,
                    after_refill_rate_per_s = readback.after.refill_rate_per_s,
                    "action diagnostic rate limit reset completed"
                );
            }
            Err(error) => {
                tracing::error!(
                    code = error.code(),
                    backend = backend.as_str(),
                    detail = error.detail(),
                    "action diagnostic rate limit reset failed"
                );
            }
        }
    });
}

fn diagnostic_adjacent_point(from: Point) -> Point {
    let x = if from.x == i32::MAX {
        from.x - 1
    } else {
        from.x + 1
    };
    Point { x, y: from.y }
}

fn diagnostic_queue_blocker_action(from: Point, to: Point, duration_ms: u32) -> Action {
    Action::MouseStroke {
        path: PathSpec::Line {
            from: PathPoint::from(from),
            to: PathPoint::from(to),
        },
        button: None,
        profile: VelocityProfile::Constant,
        timing: StrokeTiming::DurationMs { duration_ms },
        motion_model: StrokeMotionModel::Path,
        humanize: None,
        backend: Backend::Software,
    }
}

fn saturate_action_queue(
    handle: &synapse_action::ActionHandle,
) -> Result<(u32, u32, bool), ErrorData> {
    let mut filler_attempts = 0_u32;
    let mut queued_fillers = 0_u32;
    for _ in 0..=ACTION_QUEUE_CAPACITY {
        filler_attempts = filler_attempts.saturating_add(1);
        let action = Action::MouseMoveRelative {
            dx: 0.0,
            dy: 0.0,
            backend: Backend::Software,
        };
        match handle.try_execute(action) {
            Ok(()) => {
                queued_fillers = queued_fillers.saturating_add(1);
            }
            Err(ActionError::QueueFull { .. }) => {
                return Ok((filler_attempts, queued_fillers, true));
            }
            Err(error) => {
                return Err(diagnostic_action_error_to_mcp(error));
            }
        }
    }
    Ok((filler_attempts, queued_fillers, false))
}

fn diagnostic_action_error_to_mcp(error: ActionError) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        error.to_string(),
        Some(json!({
            "code": error.code(),
            "detail": error.detail(),
            "retry_after_ms": error.retry_after_ms(),
            "queue_rate_state": diagnostic_queue_rate_state(&error),
        })),
    )
}

fn diagnostic_queue_rate_state(error: &ActionError) -> Value {
    match error {
        ActionError::RateLimited {
            retry_after_ms,
            detail,
        } => json!({
            "kind": "rate_limited",
            "retry_after_ms": retry_after_ms,
            "detail": detail,
        }),
        ActionError::QueueFull { detail } => json!({
            "kind": "queue_full",
            "detail": detail,
        }),
        _ => json!({
            "kind": "not_rate_or_queue",
        }),
    }
}

impl SynapseService {
    fn start_act_stroke_foreground_monitor(
        &self,
        preflight: &ActionPreflightReadback,
    ) -> ActStrokeForegroundMonitor {
        let cancel = CancellationToken::new();
        let expected = preflight
            .after
            .clone()
            .unwrap_or_else(|| preflight.before.clone());
        let task = tokio::spawn(monitor_act_stroke_foreground(
            self.clone(),
            expected,
            cancel.clone(),
        ));
        ActStrokeForegroundMonitor { cancel, task }
    }
}

struct ActStrokeForegroundMonitor {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<Option<ErrorData>>,
}

async fn await_act_stroke_foreground_monitor(
    monitor: Option<ActStrokeForegroundMonitor>,
) -> Option<ErrorData> {
    let Some(monitor) = monitor else {
        return None;
    };
    monitor.cancel.cancel();
    match monitor.task.await {
        Ok(error) => error,
        Err(error) => Some(mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("act_stroke foreground monitor join failed: {error}"),
        )),
    }
}

async fn monitor_act_stroke_foreground(
    service: SynapseService,
    expected: ForegroundProof,
    cancel: CancellationToken,
) -> Option<ErrorData> {
    loop {
        tokio::select! {
            () = cancel.cancelled() => return None,
            () = tokio::time::sleep(std::time::Duration::from_millis(ACT_STROKE_FOREGROUND_MONITOR_INTERVAL_MS)) => {}
        }

        match service.current_audit_foreground() {
            Ok(actual) if actual.hwnd == expected.hwnd => {}
            Ok(actual) => {
                synapse_action::request_release_interrupt();
                tracing::error!(
                    code = error_codes::ACTION_FOREGROUND_LOST,
                    expected_hwnd = expected.hwnd,
                    actual_hwnd = actual.hwnd,
                    expected_pid = expected.pid,
                    actual_pid = actual.pid,
                    expected_process_name = %expected.process_name,
                    actual_process_name = %actual.process_name,
                    expected_window_title = %expected.window_title,
                    actual_window_title = %actual.window_title,
                    action_kind = "act_stroke",
                    "act_stroke foreground lost mid-stroke; release interrupt requested"
                );
                return Some(act_stroke_foreground_lost_error(
                    &expected,
                    Some(&actual),
                    None,
                ));
            }
            Err(error) => {
                synapse_action::request_release_interrupt();
                tracing::error!(
                    code = error_codes::ACTION_FOREGROUND_LOST,
                    expected_hwnd = expected.hwnd,
                    expected_pid = expected.pid,
                    expected_process_name = %expected.process_name,
                    expected_window_title = %expected.window_title,
                    read_error = %error.message,
                    action_kind = "act_stroke",
                    "act_stroke foreground read failed mid-stroke; release interrupt requested"
                );
                return Some(act_stroke_foreground_lost_error(
                    &expected,
                    None,
                    Some(&error),
                ));
            }
        }
    }
}

fn act_stroke_foreground_lost_error(
    expected: &ForegroundProof,
    actual: Option<&ForegroundContext>,
    read_error: Option<&ErrorData>,
) -> ErrorData {
    let detail = match actual {
        Some(actual) => format!(
            "act_stroke expected foreground hwnd 0x{:x} ({}) but current foreground is hwnd 0x{:x} ({})",
            expected.hwnd, expected.window_title, actual.hwnd, actual.window_title
        ),
        None => format!(
            "act_stroke could not read current foreground mid-stroke for expected hwnd 0x{:x} ({})",
            expected.hwnd, expected.window_title
        ),
    };
    ErrorData::new(
        ErrorCode(-32099),
        detail.clone(),
        Some(json!({
            "code": error_codes::ACTION_FOREGROUND_LOST,
            "reason": "act_stroke_foreground_lost_mid_stroke",
            "detail": detail,
            "point_index": Value::Null,
            "queue_rate_state": {
                "kind": "not_rate_or_queue",
            },
            "foreground_expected": expected,
            "foreground_actual": actual.map(foreground_context_details),
            "foreground_read_error": read_error.map(|error| json!({
                "message": error.message.to_string(),
                "data": error.data.clone(),
            })),
        })),
    )
}

fn foreground_context_details(foreground: &ForegroundContext) -> Value {
    json!({
        "hwnd": foreground.hwnd,
        "pid": foreground.pid,
        "process_name": &foreground.process_name,
        "process_path": &foreground.process_path,
        "window_title": &foreground.window_title,
        "monitor_index": foreground.monitor_index,
        "dpi_scale": foreground.dpi_scale,
        "profile_id": &foreground.profile_id,
        "steam_appid": foreground.steam_appid,
        "is_fullscreen": foreground.is_fullscreen,
        "is_dwm_composed": foreground.is_dwm_composed,
        "window_bounds": &foreground.window_bounds,
    })
}

fn clipboard_response_audit_details(response: &ActClipboardResponse) -> Value {
    json!({
        "response": {
            "ok": response.ok,
            "verb": response.verb,
            "format": response.format,
            "written": response.written,
            "cleared": response.cleared,
            "text_len": response.text_len,
            "text_present": response.text.is_some(),
            "elapsed_ms": response.elapsed_ms,
        },
    })
}

fn ensure_everquest_click_backend(
    params: &ActClickParams,
    preflight: &ActionPreflightReadback,
) -> Result<(), ErrorData> {
    if preflight.target_profile_id.as_deref() == Some("everquest.live")
        && params.backend == Backend::Software
    {
        return Err(mcp_error(
            error_codes::ACTION_BACKEND_UNAVAILABLE,
            "everquest.live software mouse clicks are not FSV-accepted; use backend=hardware through the configured HID path or a keyboard keymap equivalent",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use synapse_core::Rect;

    use super::*;

    #[test]
    fn key_hold_lease_ttl_matches_bounded_hold_window() {
        assert_eq!(
            lease_ttl_for_hold_ms(1),
            synapse_action::DEFAULT_LEASE_TTL_MS
        );
        assert_eq!(lease_ttl_for_hold_ms(6_000), 8_500);
        assert_eq!(
            lease_ttl_for_hold_ms(u32::MAX),
            synapse_action::MAX_LEASE_TTL_MS
        );
    }

    #[test]
    fn stroke_foreground_lost_error_carries_specific_code_and_readbacks() {
        let expected = foreground_proof(100, 10, "notepad.exe", "before");
        let actual = foreground_context(200, 20, "calc.exe", "after");

        let error = act_stroke_foreground_lost_error(&expected, Some(&actual), None);
        let data = match error.data.as_ref() {
            Some(data) => data,
            None => panic!("foreground lost error should carry structured data"),
        };

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::ACTION_FOREGROUND_LOST)
        );
        assert_eq!(
            data.pointer("/foreground_expected/hwnd")
                .and_then(Value::as_i64),
            Some(100)
        );
        assert_eq!(
            data.pointer("/foreground_actual/hwnd")
                .and_then(Value::as_i64),
            Some(200)
        );
        assert_eq!(
            data.pointer("/queue_rate_state/kind")
                .and_then(Value::as_str),
            Some("not_rate_or_queue")
        );
    }

    #[test]
    fn act_press_verify_delta_rejects_foreground_change_by_default() {
        let before = click_signature(100, 10, "WindowsTerminal.exe", "Terminal", 1);
        let after = click_signature(
            200,
            20,
            "chrome.exe",
            "Device Activation - Google Chrome",
            1,
        );

        let error = verify_captured_action_delta(
            "act_press",
            "foreground_focused_ui_or_pixels",
            250,
            before,
            after,
            None,
            ForegroundChangePolicy::reject(),
        )
        .expect_err("unexpected foreground changes must remain fail-closed");
        let data = error.data.as_ref().expect("structured error data");

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::ACTION_FOREGROUND_LOST)
        );
        assert_eq!(
            data.get("reason").and_then(Value::as_str),
            Some("unexpected_foreground_change")
        );
    }

    #[test]
    fn act_press_verify_delta_accepts_declared_foreground_transition() {
        let before = click_signature(100, 10, "WindowsTerminal.exe", "Terminal", 1);
        let after = click_signature(
            200,
            20,
            "chrome.exe",
            "Device Activation - Google Chrome",
            1,
        );
        let policy = ForegroundChangePolicy {
            allow: true,
            expected_process_regex: Some(regex::Regex::new("^chrome\\.exe$").unwrap()),
            expected_process_pattern: Some("^chrome\\.exe$".to_owned()),
            expected_title_regex: Some(regex::Regex::new("Device Activation").unwrap()),
            expected_title_pattern: Some("Device Activation".to_owned()),
        };

        let postcondition = verify_captured_action_delta(
            "act_press",
            "foreground_focused_ui_or_pixels",
            250,
            before,
            after,
            None,
            policy,
        )
        .expect("declared foreground transition should satisfy verify_delta");

        assert_eq!(postcondition.status, "observed_delta");
        assert_eq!(postcondition.observed_delta, Some(true));
        assert!(
            postcondition
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("expected foreground transition"))
        );
    }

    #[test]
    fn act_press_verify_delta_rejects_declared_transition_to_wrong_title() {
        let before = click_signature(100, 10, "WindowsTerminal.exe", "Terminal", 1);
        let after = click_signature(200, 20, "chrome.exe", "New Tab - Google Chrome", 1);
        let policy = ForegroundChangePolicy {
            allow: true,
            expected_process_regex: Some(regex::Regex::new("^chrome\\.exe$").unwrap()),
            expected_process_pattern: Some("^chrome\\.exe$".to_owned()),
            expected_title_regex: Some(regex::Regex::new("Device Activation").unwrap()),
            expected_title_pattern: Some("Device Activation".to_owned()),
        };

        let error = verify_captured_action_delta(
            "act_press",
            "foreground_focused_ui_or_pixels",
            250,
            before,
            after,
            None,
            policy,
        )
        .expect_err("wrong foreground title must fail closed");
        let data = error.data.as_ref().expect("structured error data");

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::ACTION_FOREGROUND_LOST)
        );
        assert_eq!(
            data.get("reason").and_then(Value::as_str),
            Some("foreground_change_policy_mismatch")
        );
        assert_eq!(
            data.pointer("/matches/process").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            data.pointer("/matches/title").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn act_press_foreground_policy_requires_verify_delta_before_input() {
        let params = act_press_params(false, true, None, None);

        let error = act_press_foreground_change_policy(&params)
            .expect_err("foreground-change policy without verify_delta must fail before input");
        let data = error.data.as_ref().expect("structured error data");

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::TOOL_PARAMS_INVALID)
        );
        assert_eq!(
            data.get("reason").and_then(Value::as_str),
            Some("verify_delta_required")
        );
    }

    #[test]
    fn act_press_foreground_policy_rejects_invalid_regex_before_input() {
        let params = act_press_params(true, true, None, Some("["));

        let error = act_press_foreground_change_policy(&params)
            .expect_err("invalid foreground regex must fail before input");
        let data = error.data.as_ref().expect("structured error data");

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::TOOL_PARAMS_INVALID)
        );
        assert_eq!(
            data.get("reason").and_then(Value::as_str),
            Some("invalid_expected_foreground_regex")
        );
        assert_eq!(
            data.get("field").and_then(Value::as_str),
            Some("expected_foreground_title_regex")
        );
    }

    fn foreground_proof(
        hwnd: i64,
        pid: u32,
        process_name: &str,
        window_title: &str,
    ) -> ForegroundProof {
        ForegroundProof {
            hwnd,
            pid,
            process_name: process_name.to_owned(),
            process_path: format!(r"C:\test\{process_name}"),
            window_title: window_title.to_owned(),
            is_minimized: Some(false),
            minimized_readback_error: None,
            observed_profile_id: None,
        }
    }

    fn foreground_context(
        hwnd: i64,
        pid: u32,
        process_name: &str,
        window_title: &str,
    ) -> ForegroundContext {
        ForegroundContext {
            hwnd,
            pid,
            process_name: process_name.to_owned(),
            process_path: format!(r"C:\test\{process_name}"),
            window_title: window_title.to_owned(),
            window_bounds: Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            monitor_index: 0,
            dpi_scale: 1.0,
            profile_id: None,
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
        }
    }

    fn act_press_params(
        verify_delta: bool,
        allow_foreground_change: bool,
        expected_process_regex: Option<&str>,
        expected_title_regex: Option<&str>,
    ) -> ActPressParams {
        ActPressParams {
            keys: vec!["enter".to_owned()],
            hold_ms: 33,
            backend: crate::m2::PressBackend::Auto,
            verify_delta,
            allow_foreground_change,
            expected_foreground_process_regex: expected_process_regex.map(str::to_owned),
            expected_foreground_title_regex: expected_title_regex.map(str::to_owned),
            verify_timeout_ms: crate::m2::default_verify_timeout_ms(),
        }
    }

    fn click_signature(
        hwnd: i64,
        pid: u32,
        process_name: &str,
        window_title: &str,
        element_count: usize,
    ) -> ClickDeltaSignature {
        ClickDeltaSignature {
            foreground_hwnd: hwnd,
            foreground_pid: pid,
            foreground_process: process_name.to_owned(),
            foreground_title: window_title.to_owned(),
            foreground_title_sha256: non_empty_sha256(window_title),
            focused_element_id: Some("focused.synthetic".to_owned()),
            focused_role: Some("Edit".to_owned()),
            focused_name_sha256: non_empty_sha256("synthetic focus"),
            focused_value_sha256: non_empty_sha256("synthetic value"),
            focused_bbox: Some(Rect {
                x: 1,
                y: 2,
                w: 300,
                h: 40,
            }),
            element_count,
            elements_sha256: format!("elements-{element_count}"),
            cdp_status: Some("unavailable".to_owned()),
            cdp_endpoint_present: false,
            web_path: None,
            cursor_position: None,
            pixel: ClickPixelSignature {
                status: "synthetic".to_owned(),
                region: Rect {
                    x: 0,
                    y: 0,
                    w: 800,
                    h: 600,
                },
                bitmap_sha256: Some("pixel-signature".to_owned()),
                detail: Some("synthetic pixel signature".to_owned()),
            },
            point_pixel: None,
        }
    }
}
