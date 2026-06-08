use super::{
    ActClickParams, ActClickResponse, ActClipboardParams, ActClipboardResponse,
    ActFocusWindowParams, ActFocusWindowResponse, ActKeymapParams, ActKeymapResponse, ActPadParams,
    ActPadResponse, ActPressParams, ActPressResponse, ActScrollParams, ActScrollResponse,
    ActSetValueParams, ActSetValueResponse, ActStrokeParams, ActStrokeResponse, ActTypeParams,
    ActTypeResponse, ErrorData, Json, Parameters, ReleaseAllParams, ReleaseAllResponse,
    SessionTarget, SynapseService, act_click_with_handle_and_lease, act_clipboard_session_buffer,
    act_focus_window, act_focus_window_request_details, act_focus_window_target_hwnd,
    act_keymap_with_handle, act_pad_with_handle, act_press_with_handle, act_scroll_with_handle,
    act_set_value, act_set_value_request_details, act_stroke_validation_failure_details,
    act_stroke_with_handle, act_type_with_handle,
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
    CLICK_TIER_FOREGROUND, CLICK_TIER_POSTMESSAGE, act_click_postmessage_with_params,
    act_stroke_error_details, act_stroke_request_details, attach_click_tier_attempts,
    click_params_can_route_background_first, click_target_root_hwnd, click_tier_delivered,
    click_tier_failed, emitted_text,
};
use rmcp::{RoleServer, model::ErrorCode, service::RequestContext};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use synapse_action::{
    ACTION_QUEUE_CAPACITY, ActionEmitterSnapshotHandle, ActionError, ActionHandle,
    ActionStateSnapshot, RecordingBackend, ResolvedBackend, TokenBucketSnapshot,
};
use synapse_core::{
    AccessibleNode, Action, Backend, ElementId, FocusedElement, ForegroundContext, PathPoint,
    PathSpec, Point, Rect, StrokeMotionModel, StrokeTiming, UiaPattern, VelocityProfile,
    error_codes,
};
use synapse_perception::ObservationInput;
use tokio_util::sync::CancellationToken;

const ACT_STROKE_FOREGROUND_MONITOR_INTERVAL_MS: u64 = 10;
const ACTION_DIAGNOSTIC_RATE_LIMIT_CONFIRM: &str = "force-real-rate-limit-for-fsv";
const ACTION_DIAGNOSTIC_QUEUE_FULL_CONFIRM: &str = "saturate-real-action-queue-for-fsv";
const ACTION_DIAGNOSTIC_MAX_TTL_MS: u64 = 10_000;
const ACTION_DIAGNOSTIC_MIN_TTL_MS: u64 = 100;
const ACTION_DIAGNOSTIC_MAX_QUEUE_BLOCKER_MS: u32 = 10_000;
const ACTION_DIAGNOSTIC_MIN_QUEUE_BLOCKER_MS: u32 = 250;
const ACTION_DIAGNOSTIC_QUEUE_SETTLE_MS: u64 = 50;
const ACT_TYPE_BROWSER_URL_SOURCE_OF_TRUTH: &str = "cdp_target.url";
const ACT_TYPE_BROWSER_URL_TEXT_INTEGRITY: &str = "cdp_target_url_readback";

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
        description = "Click a screen coordinate or UI Automation element. Default element delivery uses background UIA control patterns (Invoke, Toggle, SelectionItem, ExpandCollapse, LegacyIAccessible.DoDefaultAction). When element coordinate delivery is needed, Synapse tries a background HWND PostMessage click to the resolved child window before escalating to the leased foreground coordinate tier; verify_delta reads the target window SoT for element clicks. coordinate_fallback_on_unsupported=true allows bbox-center coordinate delivery only for enabled keyboard-focusable edit/document/text targets or elements exposing Value/Text patterns; set false to fail closed with ACTION_ELEMENT_PATTERN_UNSUPPORTED. This mouse click tool does not synthesize WM_CHAR/dead-key keyboard text; use act_type/act_set_value for text. velocity_profile controls coordinate-move timing only, while explicit spatial paths belong to act_stroke. If a previously observed transient element expired before dispatch, returns TRANSIENT_ELEMENT_EXPIRED with re-observe/find guidance."
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
                self.audit_action_denied_for_request("act_click", &error, &request_context);
                return Err(error);
            }
        };
        self.audit_action_started_with_details_for_request(
            "act_click",
            &action_preflight_details(&preflight),
            &request_context,
        )?;
        if let Err(error) = ensure_everquest_click_backend(&params, &preflight) {
            let result: Result<ActClickResponse, ErrorData> = Err(error);
            self.audit_action_result_for_request("act_click", &result, &request_context)?;
            return result.map(Json);
        }
        if let Err(error) = self.ensure_target_claim_allows_action(
            "act_click",
            click_claim_target(&params),
            &request_context,
        ) {
            return audit_target_claim_denial(self, "act_click", error, &request_context);
        }
        let target_window_hwnd = if params.verify_delta {
            match click_target_root_hwnd(&params) {
                Ok(hwnd) => hwnd,
                Err(error) => {
                    let result: Result<ActClickResponse, ErrorData> = Err(error);
                    self.audit_action_result_for_request("act_click", &result, &request_context)?;
                    return result.map(Json);
                }
            }
        } else {
            None
        };
        let before_delta_signature = if params.verify_delta {
            match self
                .capture_click_delta_signature(160, target_window_hwnd)
                .await
            {
                Ok(signature) => Some(signature),
                Err(error) => {
                    let result: Result<ActClickResponse, ErrorData> = Err(error);
                    self.audit_action_result_for_request("act_click", &result, &request_context)?;
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
        let started = Instant::now();
        let result = if let Some(before) = before_delta_signature {
            self.act_click_with_verified_router(
                handle,
                recording,
                params,
                before,
                verify_timeout_ms,
                target_window_hwnd,
                foreground_lease_session_id,
                started,
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
        self.audit_action_result_for_request("act_click", &result, &request_context)?;
        result.map(Json)
    }

    #[tool(
        description = "Type text. With into_element, routes through background CDP insertText for web nodes, foreground-safe native HWND text messages for UIA-resolved edit controls, or UIA ValuePattern.SetValue with value readback for native elements without a native edit HWND; into_element routing does not require foreground. Without into_element, types through the leased foreground keyboard backend."
    )]
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
                self.audit_action_denied_for_request("act_type", &error, &request_context);
                return Err(error);
            }
        };
        self.audit_action_started_with_details_for_request(
            "act_type",
            &action_preflight_details(&preflight),
            &request_context,
        )?;
        if let Err(error) = self.ensure_target_claim_allows_action(
            "act_type",
            params.into_element.as_ref().and_then(element_claim_target),
            &request_context,
        ) {
            return audit_target_claim_denial(self, "act_type", error, &request_context);
        }
        let browser_url_policy = match act_type_browser_url_policy(&params) {
            Ok(policy) => policy,
            Err(error) => {
                let result: Result<ActTypeResponse, ErrorData> = Err(error);
                self.audit_action_result_for_request("act_type", &result, &request_context)?;
                return result.map(Json);
            }
        };
        let (handle, recording, _connection_closed_cancel) =
            self.m2_action_context_for_request(&request_context)?;
        if params.into_element.is_none()
            && let Err(error) = self.ensure_act_type_foreground(&preflight, recording.as_ref())
        {
            let result: Result<ActTypeResponse, ErrorData> = Err(error);
            self.audit_action_result_for_request("act_type", &result, &request_context)?;
            return result.map(Json);
        }
        let _lease_guard = if params.into_element.is_none() {
            match acquire_tool_foreground_input_lease("act_type", &request_context) {
                Ok(guard) => Some(guard),
                Err(error) => {
                    let result: Result<ActTypeResponse, ErrorData> = Err(error);
                    self.audit_action_result_for_request("act_type", &result, &request_context)?;
                    return result.map(Json);
                }
            }
        } else {
            None
        };
        let verify_timeout_ms = params.verify_timeout_ms;
        let emitted = emitted_text(&params);
        let before_text_signature = if params.into_element.is_none() {
            match self
                .capture_act_type_text_signature(160, true, browser_url_policy.is_some())
                .await
            {
                Ok(signature) => Some(signature),
                Err(error) => {
                    let result: Result<ActTypeResponse, ErrorData> = Err(error);
                    self.audit_action_result_for_request("act_type", &result, &request_context)?;
                    return result.map(Json);
                }
            }
        } else {
            None
        };
        let result = act_type_with_handle(handle, recording, params).await;
        let result = match (result, before_text_signature) {
            (Ok(response), Some(before)) => {
                self.verify_act_type_response(
                    response,
                    before,
                    verify_timeout_ms,
                    &emitted,
                    browser_url_policy.as_ref(),
                )
                .await
            }
            (other, _) => other,
        };
        self.audit_action_result_for_request("act_type", &result, &request_context)?;
        result.map(Json)
    }

    #[tool(
        description = "Set a UI Automation element's ValuePattern value directly and verify with a separate UIA value readback. Requires a real enabled non-read-only ValuePattern target; does not fall back to keyboard typing."
    )]
    pub async fn act_set_value(
        &self,
        params: Parameters<ActSetValueParams>,
        request_context: RequestContext<RoleServer>,
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
                self.audit_action_denied_with_details_for_request(
                    "act_set_value",
                    &error,
                    &request_details,
                    &request_context,
                );
                return Err(error);
            }
        };
        self.audit_action_started_with_details_for_request(
            "act_set_value",
            &json!({
                "request": request_details,
                "preflight": preflight,
            }),
            &request_context,
        )?;
        if let Err(error) = self.ensure_target_claim_allows_action(
            "act_set_value",
            element_claim_target(&params.element_id),
            &request_context,
        ) {
            return audit_target_claim_denial(self, "act_set_value", error, &request_context);
        }
        let result = act_set_value(params).await;
        self.audit_action_result_for_request("act_set_value", &result, &request_context)?;
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
                self.audit_action_denied_with_details_for_request(
                    "act_focus_window",
                    &error,
                    &request_details,
                    &request_context,
                );
                return Err(error);
            }
        };
        self.audit_action_started_with_details_for_request(
            "act_focus_window",
            &json!({
                "request": request_details,
                "preflight": preflight,
            }),
            &request_context,
        )?;
        let focus_claim_target = match act_focus_window_target_hwnd(&params) {
            Ok(hwnd) => super::target_claims::window_session_target(hwnd),
            Err(error) => {
                return audit_target_claim_denial(
                    self,
                    "act_focus_window",
                    error,
                    &request_context,
                );
            }
        };
        if let Err(error) = self.ensure_target_claim_allows_action(
            "act_focus_window",
            Some(focus_claim_target),
            &request_context,
        ) {
            return audit_target_claim_denial(self, "act_focus_window", error, &request_context);
        }
        let mut lease_guard =
            match acquire_tool_foreground_input_lease("act_focus_window", &request_context) {
                Ok(guard) => guard,
                Err(error) => {
                    self.audit_action_error_with_details_for_request(
                        "act_focus_window",
                        &error,
                        &request_details,
                        &request_context,
                    )?;
                    return Err(error);
                }
            };
        lease_guard.disable_context_restore("act_focus_window_intentional_foreground_change");
        let result = act_focus_window(params).await;
        self.audit_action_result_for_request("act_focus_window", &result, &request_context)?;
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
                self.audit_action_denied_for_request("act_press", &error, &request_context);
                return Err(error);
            }
        };
        self.audit_action_started_with_details_for_request(
            "act_press",
            &action_preflight_details(&preflight),
            &request_context,
        )?;
        if let Err(error) =
            self.ensure_target_claim_allows_action("act_press", None, &request_context)
        {
            return audit_target_claim_denial(self, "act_press", error, &request_context);
        }
        let foreground_change_policy = match act_press_foreground_change_policy(&params) {
            Ok(policy) => policy,
            Err(error) => {
                let result: Result<ActPressResponse, ErrorData> = Err(error);
                self.audit_action_result_for_request("act_press", &result, &request_context)?;
                return result.map(Json);
            }
        };
        let before_delta_signature = if params.verify_delta {
            match self
                .capture_action_delta_signature(160, None, false, None)
                .await
            {
                Ok(signature) => Some(signature),
                Err(error) => {
                    let result: Result<ActPressResponse, ErrorData> = Err(error);
                    self.audit_action_result_for_request("act_press", &result, &request_context)?;
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
                self.audit_action_result_for_request("act_press", &result, &request_context)?;
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
        self.audit_action_result_for_request("act_press", &result, &request_context)?;
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
                self.audit_action_denied_with_details_for_request(
                    "act_keymap",
                    &error,
                    &request_details,
                    &request_context,
                );
                return Err(error);
            }
        };
        self.audit_action_started_with_details_for_request(
            "act_keymap",
            &json!({
                "request": request_details,
                "preflight": preflight,
            }),
            &request_context,
        )?;
        if let Err(error) =
            self.ensure_target_claim_allows_action("act_keymap", None, &request_context)
        {
            return audit_target_claim_denial(self, "act_keymap", error, &request_context);
        }
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
                self.audit_action_error_with_details_for_request(
                    "act_keymap",
                    &error,
                    &request_details,
                    &request_context,
                )?;
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
        self.audit_action_result_for_request("act_keymap", &result, &request_context)?;
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
                self.audit_action_error_with_details_for_request(
                    "act_stroke",
                    &error,
                    &failure_details,
                    &request_context,
                )?;
                return Err(error);
            }
        };
        let stroke_details = act_stroke_request_details(&params, &plan);
        let preflight = match self.ensure_supported_use_allows_action("act_stroke") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied_with_details_for_request(
                    "act_stroke",
                    &error,
                    &json!({
                        "stroke": stroke_details,
                        "failure": act_stroke_error_details(&error),
                    }),
                    &request_context,
                );
                return Err(error);
            }
        };
        self.audit_action_started_with_details_for_request(
            "act_stroke",
            &act_stroke_audit_details(&stroke_details, &preflight),
            &request_context,
        )?;
        if let Err(error) =
            self.ensure_target_claim_allows_action("act_stroke", None, &request_context)
        {
            return audit_target_claim_denial(self, "act_stroke", error, &request_context);
        }
        let before_delta_signature = if params.verify_delta {
            match self
                .capture_action_delta_signature(160, None, true, None)
                .await
            {
                Ok(signature) => Some(signature),
                Err(error) => {
                    self.audit_action_error_with_details_for_request(
                        "act_stroke",
                        &error,
                        &act_stroke_failure_audit_details(&stroke_details, &preflight, &error),
                        &request_context,
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
                    self.audit_action_error_with_details_for_request(
                        "act_stroke",
                        &error,
                        &failure_details,
                        &request_context,
                    )?;
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
                self.audit_action_ok_with_details_for_request(
                    "act_stroke",
                    &json!({
                        "response": response,
                        "stroke": stroke_details,
                        "preflight": preflight,
                    }),
                    &request_context,
                )?;
            }
            Err(error) => {
                let failure_details =
                    act_stroke_failure_audit_details(&stroke_details, &preflight, error);
                log_act_stroke_failure(&failure_details, error);
                self.audit_action_error_with_details_for_request(
                    "act_stroke",
                    error,
                    &failure_details,
                    &request_context,
                )?;
            }
        }
        result.map(Json)
    }

    #[cfg(test)]
    pub(crate) async fn act_clipboard_for_session_test_entrypoint(
        &self,
        params: Parameters<ActClipboardParams>,
        session_id: &str,
    ) -> Result<Json<ActClipboardResponse>, ErrorData> {
        let params = params.0;
        let request_details = clipboard_request_audit_details(&params);
        self.audit_action_started_with_details_for_session(
            "act_clipboard",
            &request_details,
            session_id,
        )?;
        let result = self.act_clipboard_for_session(params, session_id, "session_clipboard_buffer");
        match &result {
            Ok(response) => {
                self.audit_action_ok_with_details_for_session(
                    "act_clipboard",
                    &clipboard_response_audit_details(response),
                    session_id,
                )?;
            }
            Err(error) => {
                self.audit_action_error_with_details_for_session(
                    "act_clipboard",
                    error,
                    &request_details,
                    session_id,
                )?;
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
                self.audit_action_denied_for_request("act_scroll", &error, &request_context);
                return Err(error);
            }
        };
        self.audit_action_started_with_details_for_request(
            "act_scroll",
            &action_preflight_details(&preflight),
            &request_context,
        )?;
        if let Err(error) = self.ensure_target_claim_allows_action(
            "act_scroll",
            scroll_claim_target(&params),
            &request_context,
        ) {
            return audit_target_claim_denial(self, "act_scroll", error, &request_context);
        }
        let point_region = params.verify_delta_point_region();
        let before_delta_signature = if params.verify_delta && !params.uses_element_target() {
            match self
                .capture_action_delta_signature(160, point_region, false, None)
                .await
            {
                Ok(signature) => Some(signature),
                Err(error) => {
                    let result: Result<ActScrollResponse, ErrorData> = Err(error);
                    self.audit_action_result_for_request("act_scroll", &result, &request_context)?;
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
                    self.audit_action_result_for_request("act_scroll", &result, &request_context)?;
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
        self.audit_action_result_for_request("act_scroll", &result, &request_context)?;
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
                self.audit_action_denied_for_request("act_pad", &error, &request_context);
                return Err(error);
            }
        };
        self.audit_action_started_with_details_for_request(
            "act_pad",
            &action_preflight_details(&preflight),
            &request_context,
        )?;
        if let Err(error) =
            self.ensure_target_claim_allows_action("act_pad", None, &request_context)
        {
            return audit_target_claim_denial(self, "act_pad", error, &request_context);
        }
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
                    self.audit_action_result_for_request("act_pad", &result, &request_context)?;
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
        self.audit_action_result_for_request("act_pad", &result, &request_context)?;
        result.map(Json)
    }

    #[tool(
        description = "Read, write, or clear this MCP session's virtual clipboard buffer. The default path is background-safe: it does not touch the real OS clipboard and does not acquire the foreground/input lease."
    )]
    pub async fn act_clipboard(
        &self,
        params: Parameters<ActClipboardParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActClipboardResponse>, ErrorData> {
        let params = params.0;
        let session_id = super::context::mcp_session_id_from_request_context(&request_context)?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::HTTP_SESSION_INVALID,
                    "act_clipboard requires an MCP session id for session-scoped virtual clipboard state",
                )
            })?;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_clipboard",
            "tool.invocation kind=act_clipboard"
        );
        let request_details = clipboard_request_audit_details(&params);
        self.audit_action_started_with_details_for_session(
            "act_clipboard",
            &request_details,
            &session_id,
        )?;
        let result =
            self.act_clipboard_for_session(params, &session_id, "session_clipboard_buffer");
        match &result {
            Ok(response) => {
                self.audit_action_ok_with_details_for_session(
                    "act_clipboard",
                    &clipboard_response_audit_details(response),
                    &session_id,
                )?;
            }
            Err(error) => {
                self.audit_action_error_with_details_for_session(
                    "act_clipboard",
                    error,
                    &request_details,
                    &session_id,
                )?;
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
        request_context: RequestContext<RoleServer>,
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
            self.audit_action_denied_with_details_for_request(
                TOOL,
                &error,
                &request_details,
                &request_context,
            );
            return Err(error);
        }
        if let Err(error) = validate_diagnostic_ttl_ms(params.ttl_ms) {
            self.audit_action_denied_with_details_for_request(
                TOOL,
                &error,
                &request_details,
                &request_context,
            );
            return Err(error);
        }
        let preflight = match self.ensure_supported_use_allows_action("act_stroke") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied_with_details_for_request(
                    TOOL,
                    &error,
                    &request_details,
                    &request_context,
                );
                return Err(error);
            }
        };
        self.audit_action_started_with_details_for_request(
            TOOL,
            &json!({
                "request": request_details,
                "preflight": preflight,
            }),
            &request_context,
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
        self.audit_action_ok_with_details_for_request(
            TOOL,
            &json!({
                "response": response,
                "preflight": preflight,
            }),
            &request_context,
        )?;
        Ok(Json(response))
    }

    #[tool(
        description = "FSV diagnostic: saturate the real bounded action queue behind a long software blocker so the next real act_stroke proves ACTION_QUEUE_FULL through the normal MCP action path"
    )]
    pub async fn action_diagnostic_queue_full_setup(
        &self,
        params: Parameters<ActionDiagnosticQueueFullSetupParams>,
        request_context: RequestContext<RoleServer>,
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
            self.audit_action_denied_with_details_for_request(
                TOOL,
                &error,
                &request_details,
                &request_context,
            );
            return Err(error);
        }
        if let Err(error) = validate_queue_blocker_duration_ms(params.blocker_duration_ms) {
            self.audit_action_denied_with_details_for_request(
                TOOL,
                &error,
                &request_details,
                &request_context,
            );
            return Err(error);
        }
        let preflight = match self.ensure_supported_use_allows_action("act_stroke") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied_with_details_for_request(
                    TOOL,
                    &error,
                    &request_details,
                    &request_context,
                );
                return Err(error);
            }
        };
        self.audit_action_started_with_details_for_request(
            TOOL,
            &json!({
                "request": request_details,
                "preflight": preflight,
            }),
            &request_context,
        )?;
        let (handle, recording, _connection_closed_cancel) =
            self.m2_action_context_for_session_id(None)?;
        if recording.is_some() {
            let error = mcp_error(
                error_codes::ACTION_BACKEND_UNAVAILABLE,
                "action_diagnostic_queue_full_setup requires the real action emitter, not the recording backend",
            );
            self.audit_action_error_with_details_for_request(
                TOOL,
                &error,
                &request_details,
                &request_context,
            )?;
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
            self.audit_action_error_with_details_for_request(
                TOOL,
                &error,
                &request_details,
                &request_context,
            )?;
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
        self.audit_action_ok_with_details_for_request(
            TOOL,
            &json!({
                "response": response,
                "preflight": preflight,
            }),
            &request_context,
        )?;
        Ok(Json(response))
    }

    #[tool(description = "Release all held keyboard, mouse, and gamepad input state")]
    pub async fn release_all(
        &self,
        params: Parameters<ReleaseAllParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ReleaseAllResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "release_all",
            "tool.invocation kind=release_all"
        );
        let (handle, snapshot_handle, reflex_runtime) = self.m2_release_all_context()?;
        let result =
            release_all_with_handles(handle, snapshot_handle, reflex_runtime, params.0).await;
        self.audit_action_result_for_request_best_effort("release_all", &result, &request_context);
        result.map(Json)
    }
}

fn foreground_lease_session_id(
    request_context: &RequestContext<RoleServer>,
) -> Result<Option<String>, ErrorData> {
    super::context::mcp_session_id_from_request_context(request_context)
}

fn audit_target_claim_denial<T: Serialize>(
    service: &SynapseService,
    tool: &'static str,
    error: ErrorData,
    request_context: &RequestContext<RoleServer>,
) -> Result<Json<T>, ErrorData> {
    let result: Result<T, ErrorData> = Err(error);
    service.audit_action_result_for_request(tool, &result, request_context)?;
    result.map(Json)
}

fn click_claim_target(params: &ActClickParams) -> Option<SessionTarget> {
    click_target_root_hwnd(params)
        .ok()
        .flatten()
        .map(super::target_claims::window_session_target)
}

fn element_claim_target(element_id: &ElementId) -> Option<SessionTarget> {
    element_id
        .parts()
        .ok()
        .map(|parts| super::target_claims::window_session_target(parts.hwnd))
}

fn scroll_claim_target(params: &ActScrollParams) -> Option<SessionTarget> {
    params
        .target
        .as_ref()
        .and_then(|target| element_claim_target(&target.element_id))
}

const fn click_delta_source_of_truth(target_window_hwnd: Option<i64>) -> &'static str {
    if target_window_hwnd.is_some() {
        "target_window_ui_or_pixels"
    } else {
        "foreground_focused_ui_or_pixels"
    }
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
    browser_url: Option<String>,
}

#[derive(Clone, Debug)]
struct ActTypeBrowserUrlPolicy {
    expected_url_regex: regex::Regex,
    expected_url_pattern: String,
}

#[derive(Clone, Debug)]
struct CdpTargetUrlReadback {
    url: Option<String>,
    target_id: Option<String>,
    source: Option<String>,
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
    browser_url_len: Option<usize>,
    browser_url_sha256: Option<String>,
    browser_cdp_target_id: Option<String>,
    browser_url_readback_source: Option<String>,
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
    pub(crate) fn act_clipboard_for_session(
        &self,
        params: ActClipboardParams,
        session_id: &str,
        source_of_truth: &'static str,
    ) -> Result<ActClipboardResponse, ErrorData> {
        let result =
            act_clipboard_session_buffer(params, session_id, self.session_clipboards_ref());
        if let Err(error) = &result {
            tracing::error!(
                code = error
                    .data
                    .as_ref()
                    .and_then(|data| data.get("code"))
                    .and_then(|value| value.as_str())
                    .unwrap_or(error_codes::TOOL_INTERNAL_ERROR),
                session_id,
                source_of_truth,
                detail = %error.message,
                data = ?error.data,
                "act_clipboard session buffer operation failed"
            );
        }
        result
    }

    async fn act_click_with_verified_router(
        &self,
        handle: ActionHandle,
        recording: Option<Arc<RecordingBackend>>,
        params: ActClickParams,
        before: ClickDeltaSignature,
        verify_timeout_ms: u32,
        target_window_hwnd: Option<i64>,
        foreground_lease_session_id: Option<String>,
        started: Instant,
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
                    .verify_click_response(
                        response,
                        before.clone(),
                        verify_timeout_ms,
                        target_window_hwnd,
                    )
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
                        if should_try_click_postmessage_tier(&tier_attempts) {
                            self.act_click_try_postmessage_then_foreground(
                                handle,
                                recording,
                                params,
                                before,
                                verify_timeout_ms,
                                target_window_hwnd,
                                tier_attempts,
                                foreground_lease_session_id,
                            )
                            .await
                        } else if should_try_click_foreground_tier(&tier_attempts) {
                            self.act_click_try_foreground(
                                handle,
                                recording,
                                params,
                                before,
                                verify_timeout_ms,
                                target_window_hwnd,
                                tier_attempts,
                                foreground_lease_session_id,
                            )
                            .await
                        } else {
                            Err(error)
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) if click_postdispatch_readback_failed(&error) => {
                match self
                    .reconcile_click_postdispatch_error(
                        &params,
                        &error,
                        before.clone(),
                        verify_timeout_ms,
                        target_window_hwnd,
                        started,
                    )
                    .await
                {
                    Ok(response) => return Ok(response),
                    Err(verify_error) => {
                        tracing::warn!(
                            code = click_error_data_code(&verify_error)
                                .unwrap_or(error_codes::ACTION_POSTCONDITION_FAILED),
                            kind = "act_click",
                            original_error_code = click_error_data_code(&error)
                                .unwrap_or(error_codes::ACTION_TARGET_INVALID),
                            detail = %verify_error.message,
                            "act_click post-dispatch UIA readback error was not reconciled by target-window SoT; considering next eligible delivery tier"
                        );
                    }
                }
                if !can_route_click_element_background_first(&params, recording.as_ref())
                    || !should_try_next_click_tier(&error)
                {
                    return Err(error);
                }
                let tier_attempts = click_tier_attempts_from_error(&error);
                if should_try_click_postmessage_tier(&tier_attempts) {
                    self.act_click_try_postmessage_then_foreground(
                        handle,
                        recording,
                        params,
                        before,
                        verify_timeout_ms,
                        target_window_hwnd,
                        tier_attempts,
                        foreground_lease_session_id,
                    )
                    .await
                } else if should_try_click_foreground_tier(&tier_attempts) {
                    self.act_click_try_foreground(
                        handle,
                        recording,
                        params,
                        before,
                        verify_timeout_ms,
                        target_window_hwnd,
                        tier_attempts,
                        foreground_lease_session_id,
                    )
                    .await
                } else {
                    Err(error)
                }
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
        target_window_hwnd: Option<i64>,
        tier_attempts: Vec<ActClickTierAttempt>,
        foreground_lease_session_id: Option<String>,
    ) -> Result<ActClickResponse, ErrorData> {
        match act_click_postmessage_with_params(&params, tier_attempts).await {
            Ok(response) => {
                match self
                    .verify_click_response(
                        response,
                        before.clone(),
                        verify_timeout_ms,
                        target_window_hwnd,
                    )
                    .await
                {
                    Ok(response) => Ok(response),
                    Err(error) if should_try_next_click_tier(&error) => {
                        let tier_attempts = click_tier_attempts_from_error(&error);
                        if should_try_click_foreground_tier(&tier_attempts) {
                            self.act_click_try_foreground(
                                handle,
                                recording,
                                params,
                                before,
                                verify_timeout_ms,
                                target_window_hwnd,
                                tier_attempts,
                                foreground_lease_session_id,
                            )
                            .await
                        } else {
                            Err(error)
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) if should_try_next_click_tier(&error) => {
                let tier_attempts = click_tier_attempts_from_error(&error);
                if should_try_click_foreground_tier(&tier_attempts) {
                    self.act_click_try_foreground(
                        handle,
                        recording,
                        params,
                        before,
                        verify_timeout_ms,
                        target_window_hwnd,
                        tier_attempts,
                        foreground_lease_session_id,
                    )
                    .await
                } else {
                    Err(error)
                }
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
        target_window_hwnd: Option<i64>,
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
                self.verify_click_response(response, before, verify_timeout_ms, target_window_hwnd)
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
        target_window_hwnd: Option<i64>,
    ) -> Result<ActClickResponse, ErrorData> {
        match self
            .verify_click_delta(before, verify_timeout_ms, target_window_hwnd)
            .await
        {
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

    async fn reconcile_click_postdispatch_error(
        &self,
        params: &ActClickParams,
        error: &ErrorData,
        before: ClickDeltaSignature,
        verify_timeout_ms: u32,
        target_window_hwnd: Option<i64>,
        started: Instant,
    ) -> Result<ActClickResponse, ErrorData> {
        let postcondition = self
            .verify_click_delta(before, verify_timeout_ms, target_window_hwnd)
            .await?;
        let mut tier_attempts = click_tier_attempts_from_error(error);
        let tier = tier_attempts
            .first()
            .map(|attempt| attempt.tier.clone())
            .unwrap_or_else(|| "uia".to_owned());
        tier_attempts.push(click_tier_delivered(
            tier.clone(),
            tier_attempts
                .iter()
                .any(|attempt| attempt.required_foreground),
            format!(
                "delivery reported a post-dispatch UIA readback failure, but verify_delta separately observed target-window SoT mutation; original_error={}",
                error.message
            ),
        ));
        let timing = synapse_action::cached_double_click_timing();
        tracing::info!(
            code = "M2_ACT_CLICK_POSTDISPATCH_READBACK_RECONCILED",
            kind = "act_click",
            backend_tier_used = %tier,
            error_message = %error.message,
            postcondition_status = %postcondition.status,
            "readback=target_window_delta tool=act_click outcome=reconciled_postdispatch_error"
        );
        Ok(ActClickResponse {
            ok: true,
            used_invoke_pattern: tier == "uia",
            backend_used: tier.clone(),
            backend_tier_used: tier,
            required_foreground: tier_attempts
                .iter()
                .any(|attempt| attempt.required_foreground),
            tier_attempts,
            postcondition,
            press_hold_ms: params.hold_ms,
            double_click_window_ms: timing.window_ms,
            inter_click_delay_ms: timing.inter_click_delay_ms,
            elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        })
    }

    async fn capture_click_delta_signature(
        &self,
        max_elements: usize,
        target_window_hwnd: Option<i64>,
    ) -> Result<ClickDeltaSignature, ErrorData> {
        self.capture_action_delta_signature(max_elements, None, false, target_window_hwnd)
            .await
    }

    async fn capture_act_type_text_signature(
        &self,
        max_elements: usize,
        require_focused_text_value: bool,
        require_browser_url: bool,
    ) -> Result<ActTypeTextReadback, ErrorData> {
        let mut input = {
            let state = self.m1_state()?;
            crate::m1::current_input(&state, 6)?
        };
        crate::m1::enrich_input_with_cdp(&mut input, 6, max_elements).await;
        crate::m1::enrich_input_with_browser_ocr(&mut input, max_elements);
        let browser_url = Self::cdp_selected_target_url(&input, require_browser_url).await?;

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
            browser_url_len: browser_url.url.as_ref().map(|url| url.chars().count()),
            browser_url_sha256: browser_url.url.as_deref().and_then(non_empty_sha256),
            browser_cdp_target_id: browser_url.target_id.clone(),
            browser_url_readback_source: browser_url.source,
        };
        if require_focused_text_value && value.is_none() {
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

        Ok(ActTypeTextReadback {
            signature,
            value,
            browser_url: browser_url.url,
        })
    }

    async fn capture_action_delta_signature(
        &self,
        max_elements: usize,
        point_region: Option<Point>,
        include_cursor: bool,
        target_window_hwnd: Option<i64>,
    ) -> Result<ClickDeltaSignature, ErrorData> {
        let mut input = {
            let state = self.m1_state()?;
            if let Some(hwnd) = target_window_hwnd {
                crate::m1::observe_input(
                    &state,
                    &crate::m1::ObserveParams {
                        depth: Some(6),
                        max_elements: Some(max_elements),
                        window_hwnd: Some(hwnd),
                        ..crate::m1::ObserveParams::default()
                    },
                    None,
                )?
            } else {
                crate::m1::current_input(&state, 6)?
            }
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

    async fn cdp_selected_target_url(
        input: &ObservationInput,
        require_browser_url: bool,
    ) -> Result<CdpTargetUrlReadback, ErrorData> {
        let Some(cdp) = input.cdp.as_ref() else {
            if require_browser_url {
                return Err(act_type_browser_url_readback_error(
                    error_codes::A11Y_CDP_UNREACHABLE,
                    "act_type expected_browser_url_regex requires a Chromium CDP readback, but the foreground is not a CDP-observable browser",
                    None,
                    None,
                    None,
                ));
            }
            return Ok(CdpTargetUrlReadback {
                url: None,
                target_id: None,
                source: None,
            });
        };
        let Some(endpoint) = cdp.endpoint.as_deref() else {
            if require_browser_url {
                return Err(act_type_browser_url_readback_error(
                    cdp.reason_code
                        .as_deref()
                        .unwrap_or(error_codes::A11Y_CDP_UNREACHABLE),
                    "act_type expected_browser_url_regex requires a reachable Chromium CDP endpoint",
                    Some(cdp.status.as_str()),
                    None,
                    cdp.detail.as_deref(),
                ));
            }
            return Ok(CdpTargetUrlReadback {
                url: None,
                target_id: cdp.selected_target_id.clone(),
                source: Some(cdp.status.as_str().to_owned()),
            });
        };
        let Some(target_id) = cdp.selected_target_id.as_deref() else {
            if require_browser_url {
                return Err(act_type_browser_url_readback_error(
                    error_codes::A11Y_CDP_ATTACH_FAILED,
                    "act_type expected_browser_url_regex requires a selected CDP target id from the observation readback",
                    Some(cdp.status.as_str()),
                    Some(endpoint),
                    cdp.detail.as_deref(),
                ));
            }
            return Ok(CdpTargetUrlReadback {
                url: None,
                target_id: None,
                source: Some("cdp_without_selected_target".to_owned()),
            });
        };
        let targets = synapse_a11y::cdp_list_targets(endpoint)
            .await
            .map_err(|error| {
                act_type_browser_url_readback_error(
                    error.code(),
                    format!("Target.getTargets readback failed for act_type browser URL verification: {error}"),
                    Some(cdp.status.as_str()),
                    Some(endpoint),
                    cdp.detail.as_deref(),
                )
            })?;
        let Some(target) = targets
            .iter()
            .find(|target| target.target_id.eq_ignore_ascii_case(target_id))
        else {
            return Err(act_type_browser_url_readback_error(
                error_codes::ACTION_POSTCONDITION_FAILED,
                format!(
                    "Target.getTargets readback did not contain selected target id {target_id:?} for act_type browser URL verification"
                ),
                Some(cdp.status.as_str()),
                Some(endpoint),
                cdp.detail.as_deref(),
            ));
        };
        if target.url.trim().is_empty() && require_browser_url {
            return Err(act_type_browser_url_readback_error(
                error_codes::ACTION_POSTCONDITION_FAILED,
                format!(
                    "Target.getTargets readback for selected target id {target_id:?} contained an empty URL"
                ),
                Some(cdp.status.as_str()),
                Some(endpoint),
                cdp.detail.as_deref(),
            ));
        }
        Ok(CdpTargetUrlReadback {
            url: (!target.url.trim().is_empty()).then(|| target.url.clone()),
            target_id: Some(target.target_id.clone()),
            source: Some("Target.getTargets".to_owned()),
        })
    }

    async fn verify_click_delta(
        &self,
        before: ClickDeltaSignature,
        timeout_ms: u32,
        target_window_hwnd: Option<i64>,
    ) -> Result<ActClickPostcondition, ErrorData> {
        tokio::time::sleep(Duration::from_millis(u64::from(timeout_ms))).await;
        let after = self
            .capture_action_delta_signature(160, None, false, target_window_hwnd)
            .await?;
        let source_of_truth = click_delta_source_of_truth(target_window_hwnd);
        let before_hash = signature_hash(&before)?;
        let after_hash = signature_hash(&after)?;
        if foreground_identity_changed(&before, &after) {
            return Err(foreground_lost_delta_error(
                "act_click",
                source_of_truth,
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
            "act_click",
            source_of_truth,
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
        browser_url_policy: Option<&ActTypeBrowserUrlPolicy>,
    ) -> Result<ActTypeResponse, ErrorData> {
        tokio::time::sleep(Duration::from_millis(u64::from(verify_timeout_ms))).await;
        let after = self
            .capture_act_type_text_signature(
                160,
                browser_url_policy.is_none(),
                browser_url_policy.is_some(),
            )
            .await?;
        let before_hash = verify_hash_json(&before.signature)?;
        let after_hash = verify_hash_json(&after.signature)?;
        if let Some(policy) = browser_url_policy {
            return verify_act_type_browser_url_response(
                response,
                before,
                after,
                before_hash,
                after_hash,
                verify_timeout_ms,
                policy,
            );
        }
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
            .capture_action_delta_signature(
                160,
                point_region,
                source_of_truth.contains("cursor"),
                None,
            )
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

fn act_type_browser_url_policy(
    params: &ActTypeParams,
) -> Result<Option<ActTypeBrowserUrlPolicy>, ErrorData> {
    let Some(pattern) = params.expected_browser_url_regex.as_deref() else {
        return Ok(None);
    };
    if !params.verify_delta {
        return Err(act_type_url_policy_params_invalid(
            "verify_delta",
            "act_type expected_browser_url_regex requires verify_delta=true",
            "verify_delta_required",
        ));
    }
    if params.into_element.is_some() {
        return Err(act_type_url_policy_params_invalid(
            "into_element",
            "act_type expected_browser_url_regex applies only to foreground typing, not into_element routing",
            "foreground_typing_required",
        ));
    }
    if pattern.trim().is_empty() {
        return Err(act_type_url_policy_params_invalid(
            "expected_browser_url_regex",
            "expected_browser_url_regex must not be empty",
            "empty_expected_browser_url_regex",
        ));
    }
    let expected_url_regex = regex::Regex::new(pattern).map_err(|error| {
        act_type_url_policy_params_invalid(
            "expected_browser_url_regex",
            format!("expected_browser_url_regex is not a valid regex: {error}"),
            "invalid_expected_browser_url_regex",
        )
    })?;
    Ok(Some(ActTypeBrowserUrlPolicy {
        expected_url_regex,
        expected_url_pattern: pattern.to_owned(),
    }))
}

fn act_type_url_policy_params_invalid(
    field: &'static str,
    detail: impl Into<String>,
    reason: &'static str,
) -> ErrorData {
    let detail = detail.into();
    tracing::error!(
        code = error_codes::TOOL_PARAMS_INVALID,
        tool = "act_type",
        field,
        reason,
        detail = %detail,
        "act_type browser URL policy parameters invalid"
    );
    ErrorData::new(
        ErrorCode(-32099),
        detail.clone(),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "tool": "act_type",
            "field": field,
            "reason": reason,
            "detail": detail,
        })),
    )
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
    match click_error_data_code(error) {
        Some(
            error_codes::ACTION_ELEMENT_PATTERN_UNSUPPORTED
            | error_codes::ACTION_NO_OBSERVED_DELTA
            | error_codes::ACTION_BACKEND_UNAVAILABLE,
        ) => true,
        Some(error_codes::ACTION_TARGET_INVALID) => click_postdispatch_readback_failed(error),
        _ => false,
    }
}

fn click_postdispatch_readback_failed(error: &ErrorData) -> bool {
    let detail = click_error_detail(error).to_ascii_lowercase();
    (detail.contains("togglepattern.toggle returned") && detail.contains("togglestate stayed"))
        || (detail.contains("selectionitempattern.select returned")
            && detail.contains("isselected stayed false"))
}

fn click_error_detail(error: &ErrorData) -> String {
    let mut detail = error.message.to_string();
    if let Some(data_detail) = error
        .data
        .as_ref()
        .and_then(|data| data.get("detail"))
        .and_then(Value::as_str)
    {
        detail.push(' ');
        detail.push_str(data_detail);
    }
    for attempt in click_tier_attempts_from_error(error) {
        if let Some(attempt_detail) = attempt.detail {
            detail.push(' ');
            detail.push_str(&attempt_detail);
        }
    }
    detail
}

fn should_try_click_postmessage_tier(tier_attempts: &[ActClickTierAttempt]) -> bool {
    !click_tier_attempted(tier_attempts, CLICK_TIER_POSTMESSAGE)
        && !click_tier_attempted(tier_attempts, CLICK_TIER_FOREGROUND)
}

fn should_try_click_foreground_tier(tier_attempts: &[ActClickTierAttempt]) -> bool {
    click_tier_attempted(tier_attempts, CLICK_TIER_POSTMESSAGE)
        && !click_tier_attempted(tier_attempts, CLICK_TIER_FOREGROUND)
}

fn click_tier_attempted(tier_attempts: &[ActClickTierAttempt], tier: &str) -> bool {
    tier_attempts.iter().any(|attempt| attempt.tier == tier)
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

fn verify_act_type_browser_url_response(
    mut response: ActTypeResponse,
    before: ActTypeTextReadback,
    after: ActTypeTextReadback,
    before_hash: String,
    after_hash: String,
    verify_timeout_ms: u32,
    policy: &ActTypeBrowserUrlPolicy,
) -> Result<ActTypeResponse, ErrorData> {
    let before_signature_readback = before.signature.clone();
    let after_signature_readback = after.signature.clone();
    let after_url = after.browser_url.as_deref().ok_or_else(|| {
        postcondition_failed_error(
            "act_type",
            ACT_TYPE_BROWSER_URL_SOURCE_OF_TRUTH,
            "expected_browser_url_regex was set but after-read CDP target URL was absent",
            before_hash.clone(),
            after_hash.clone(),
            json!({
                "expected_browser_url_regex": &policy.expected_url_pattern,
                "before": before_signature_readback,
                "after": after_signature_readback,
            }),
        )
    })?;
    if !policy.expected_url_regex.is_match(after_url) {
        return Err(postcondition_failed_error(
            "act_type",
            ACT_TYPE_BROWSER_URL_SOURCE_OF_TRUTH,
            "after-read CDP target URL did not match expected_browser_url_regex",
            before_hash,
            after_hash,
            json!({
                "expected_browser_url_regex": &policy.expected_url_pattern,
                "after_url_len": after_url.chars().count(),
                "after_url_sha256": non_empty_sha256(after_url),
                "before": before.signature,
                "after": after.signature,
            }),
        ));
    }
    let before_url = before.browser_url.as_deref();
    response.postcondition = if before_url == Some(after_url) {
        ActPostcondition {
            status: "verified_state".to_owned(),
            observed_delta: Some(false),
            source_of_truth: Some(ACT_TYPE_BROWSER_URL_SOURCE_OF_TRUTH.to_owned()),
            before_signature: Some(before_hash),
            after_signature: Some(after_hash),
            detail: Some(format!(
                "act_type verify_delta verified after-read CDP target URL matched expected_browser_url_regex; no URL delta was observed within {verify_timeout_ms} ms"
            )),
        }
    } else {
        postcondition_observed_delta(
            "act_type",
            ACT_TYPE_BROWSER_URL_SOURCE_OF_TRUTH,
            before_hash,
            after_hash,
            "observed after-read CDP target URL matching expected_browser_url_regex after delivery",
        )
    };
    response.target_readback_required = false;
    response.target_text_integrity = ACT_TYPE_BROWSER_URL_TEXT_INTEGRITY.to_owned();
    Ok(response)
}

fn act_type_browser_url_readback_error(
    code: &str,
    detail: impl Into<String>,
    cdp_status: Option<&str>,
    endpoint: Option<&str>,
    cdp_detail: Option<&str>,
) -> ErrorData {
    let detail = detail.into();
    tracing::error!(
        code,
        tool = "act_type",
        source_of_truth = ACT_TYPE_BROWSER_URL_SOURCE_OF_TRUTH,
        cdp_status,
        endpoint,
        detail = %detail,
        cdp_detail,
        "act_type browser URL Source-of-Truth readback failed"
    );
    ErrorData::new(
        ErrorCode(-32099),
        detail.clone(),
        Some(json!({
            "code": code,
            "tool": "act_type",
            "source_of_truth": ACT_TYPE_BROWSER_URL_SOURCE_OF_TRUTH,
            "cdp_status": cdp_status,
            "endpoint": endpoint,
            "cdp_detail": cdp_detail,
            "detail": detail,
        })),
    )
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
            "backing": response.backing,
            "backend_tier_used": response.backend_tier_used,
            "source_of_truth": response.source_of_truth,
            "os_clipboard_touched": response.os_clipboard_touched,
            "required_foreground": response.required_foreground,
            "lease_required": response.lease_required,
            "elapsed_ms": response.elapsed_ms,
        },
    })
}

fn clipboard_request_audit_details(params: &ActClipboardParams) -> Value {
    json!({
        "verb": params.verb,
        "format": params.format,
        "text_len": params.text.as_ref().map(|text| text.chars().count()),
        "session_scoped": true,
        "source_of_truth": "session_clipboard_buffer",
        "os_clipboard_touched": false,
        "backend_tier_used": "session_buffer",
        "required_foreground": false,
        "lease_required": false,
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

    #[test]
    fn act_type_browser_url_policy_requires_verify_delta_before_input() {
        let params = act_type_params(false, Some("^file:///synapse-810\\.html$"));

        let error = act_type_browser_url_policy(&params)
            .expect_err("browser URL policy without verify_delta must fail before input");
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
    fn act_type_browser_url_policy_rejects_invalid_regex_before_input() {
        let params = act_type_params(true, Some("["));

        let error = act_type_browser_url_policy(&params)
            .expect_err("invalid browser URL regex must fail before input");
        let data = error.data.as_ref().expect("structured error data");

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::TOOL_PARAMS_INVALID)
        );
        assert_eq!(
            data.get("reason").and_then(Value::as_str),
            Some("invalid_expected_browser_url_regex")
        );
        assert_eq!(
            data.get("field").and_then(Value::as_str),
            Some("expected_browser_url_regex")
        );
    }

    #[test]
    fn act_type_browser_url_policy_accepts_navigation_focus_change_when_url_matches() {
        let policy = act_type_browser_url_policy(&act_type_params(
            true,
            Some("^file:///C:/synapse-810-after\\.html$"),
        ))
        .expect("valid browser URL policy")
        .expect("policy should be present");
        let before = act_type_readback(
            Some("file:///C:/synapse-810-before.html"),
            Some("address-bar"),
            Some("file:///C:/synapse-810-before.html"),
        );
        let after = act_type_readback(
            Some("file:///C:/synapse-810-after.html"),
            Some("document-body"),
            None,
        );
        let response = ActTypeResponse {
            ok: true,
            chars_typed: 36,
            elapsed_ms: 10,
            backend_tier_used: "foreground".to_owned(),
            required_foreground: true,
            target_text_integrity: "dispatch_only_requires_target_readback".to_owned(),
            target_readback_required: true,
            minimum_linear_ms_per_char: 20,
            postcondition: crate::m2::postcondition::postcondition_not_requested(
                "act_type",
                "foreground_focused_ui_or_pixels",
            ),
        };

        let verified = verify_act_type_browser_url_response(
            response,
            before,
            after,
            "before-hash".to_owned(),
            "after-hash".to_owned(),
            250,
            &policy,
        )
        .expect("matching browser URL should verify despite focus moving to the document");

        assert_eq!(verified.postcondition.status, "observed_delta");
        assert_eq!(verified.postcondition.observed_delta, Some(true));
        assert_eq!(
            verified.postcondition.source_of_truth.as_deref(),
            Some(ACT_TYPE_BROWSER_URL_SOURCE_OF_TRUTH)
        );
        assert_eq!(
            verified.target_text_integrity,
            ACT_TYPE_BROWSER_URL_TEXT_INTEGRITY
        );
        assert!(!verified.target_readback_required);
    }

    #[test]
    fn click_router_respects_coordinate_fallback_disabled() {
        let mut params = act_click_element_params();
        params.use_invoke_pattern = true;
        params.coordinate_fallback_on_unsupported = false;

        let can_route = can_route_click_element_background_first(&params, None);

        assert!(!can_route);
    }

    #[test]
    fn click_router_keeps_direct_coordinate_element_route() {
        let mut params = act_click_element_params();
        params.use_invoke_pattern = false;
        params.coordinate_fallback_on_unsupported = false;

        let can_route = can_route_click_element_background_first(&params, None);

        assert!(can_route);
    }

    #[test]
    fn click_router_advances_without_replaying_attempted_tiers() {
        let uia_failed = click_attempt(
            "uia",
            "failed",
            Some(error_codes::ACTION_ELEMENT_PATTERN_UNSUPPORTED),
        );
        assert!(should_try_click_postmessage_tier(std::slice::from_ref(
            &uia_failed
        )));
        assert!(!should_try_click_foreground_tier(std::slice::from_ref(
            &uia_failed
        )));

        let postmessage_no_delta = click_attempt(
            CLICK_TIER_POSTMESSAGE,
            "failed",
            Some(error_codes::ACTION_NO_OBSERVED_DELTA),
        );
        let after_postmessage = vec![uia_failed, postmessage_no_delta];
        assert!(!should_try_click_postmessage_tier(&after_postmessage));
        assert!(should_try_click_foreground_tier(&after_postmessage));

        let foreground_no_delta = click_attempt(
            CLICK_TIER_FOREGROUND,
            "failed",
            Some(error_codes::ACTION_NO_OBSERVED_DELTA),
        );
        let exhausted = vec![
            click_attempt(
                "uia",
                "failed",
                Some(error_codes::ACTION_ELEMENT_PATTERN_UNSUPPORTED),
            ),
            click_attempt(
                CLICK_TIER_POSTMESSAGE,
                "failed",
                Some(error_codes::ACTION_NO_OBSERVED_DELTA),
            ),
            foreground_no_delta,
        ];
        assert!(!should_try_click_postmessage_tier(&exhausted));
        assert!(!should_try_click_foreground_tier(&exhausted));
    }

    #[test]
    fn click_router_treats_toggle_readback_failure_as_postdispatch_retry_eligible() {
        let error = postdispatch_click_error(
            "accessibility backend failed: TogglePattern.toggle returned for element 0x1:0000002a00000001, but ToggleState stayed Off",
        );

        println!(
            "readback=act_click_postdispatch edge=toggle detail={:?} retry={}",
            error.message,
            should_try_next_click_tier(&error)
        );
        assert!(click_postdispatch_readback_failed(&error));
        assert!(should_try_next_click_tier(&error));
    }

    #[test]
    fn click_router_recognizes_toggle_readback_failure_when_background_route_disabled() {
        let mut params = act_click_element_params();
        params.use_invoke_pattern = true;
        params.coordinate_fallback_on_unsupported = false;
        let error = postdispatch_click_error(
            "accessibility backend failed: TogglePattern.toggle returned for element 0x1:0000002a00000001, but ToggleState stayed Off",
        );

        println!(
            "readback=act_click_postdispatch edge=toggle_background_route_disabled can_route={} reconcile={}",
            can_route_click_element_background_first(&params, None),
            click_postdispatch_readback_failed(&error)
        );
        assert!(!can_route_click_element_background_first(&params, None));
        assert!(click_postdispatch_readback_failed(&error));
    }

    #[test]
    fn click_router_keeps_generic_target_invalid_fail_closed() {
        let error = postdispatch_click_error("element bbox is empty or inverted");

        println!(
            "readback=act_click_postdispatch edge=generic_target_invalid detail={:?} retry={}",
            error.message,
            should_try_next_click_tier(&error)
        );
        assert!(!click_postdispatch_readback_failed(&error));
        assert!(!should_try_next_click_tier(&error));
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

    fn act_type_params(
        verify_delta: bool,
        expected_browser_url_regex: Option<&str>,
    ) -> ActTypeParams {
        serde_json::from_value(json!({
            "text": "file:///C:/synapse-810-after.html",
            "dynamics": "burst",
            "press_enter_after": true,
            "backend": "auto",
            "verify_delta": verify_delta,
            "expected_browser_url_regex": expected_browser_url_regex,
            "verify_timeout_ms": crate::m2::default_verify_timeout_ms(),
        }))
        .expect("synthetic act_type params must deserialize through the public tool schema")
    }

    fn act_type_readback(
        browser_url: Option<&str>,
        focused_element_id: Option<&str>,
        focused_value: Option<&str>,
    ) -> ActTypeTextReadback {
        let focused_value = focused_value.map(str::to_owned);
        let browser_url_owned = browser_url.map(str::to_owned);
        ActTypeTextReadback {
            signature: ActTypeTextSignature {
                foreground_hwnd: 100,
                foreground_pid: 20,
                foreground_process: "chrome.exe".to_owned(),
                foreground_title_sha256: non_empty_sha256("Synthetic - Google Chrome"),
                focused_element_id: focused_element_id.map(str::to_owned),
                focused_role: focused_element_id.map(|_| "Edit".to_owned()),
                focused_name_sha256: focused_element_id.and_then(non_empty_sha256),
                focused_value_len: focused_value.as_ref().map(|value| value.chars().count()),
                focused_value_sha256: focused_value.as_deref().and_then(non_empty_sha256),
                focused_selected_text_sha256: None,
                focused_bbox: Some(Rect {
                    x: 10,
                    y: 10,
                    w: 400,
                    h: 32,
                }),
                readback_source: focused_value.as_ref().map(|_| "focused.value".to_owned()),
                has_text_readback: focused_value.is_some(),
                browser_url_len: browser_url_owned
                    .as_ref()
                    .map(|value| value.chars().count()),
                browser_url_sha256: browser_url_owned.as_deref().and_then(non_empty_sha256),
                browser_cdp_target_id: Some("TARGET810".to_owned()),
                browser_url_readback_source: Some("Target.getTargets".to_owned()),
            },
            value: focused_value,
            browser_url: browser_url_owned,
        }
    }

    fn act_click_element_params() -> ActClickParams {
        serde_json::from_value(json!({
            "target": {
                "element_id": "0x1000:0000002a00000001"
            },
            "verify_delta": true
        }))
        .expect("synthetic act_click params must deserialize through the public tool schema")
    }

    fn postdispatch_click_error(detail: &str) -> ErrorData {
        let tier_attempts = vec![ActClickTierAttempt {
            tier: "uia".to_owned(),
            status: "failed".to_owned(),
            reason_code: Some("target_invalid".to_owned()),
            error_code: Some(error_codes::ACTION_TARGET_INVALID.to_owned()),
            detail: Some(detail.to_owned()),
            required_foreground: false,
        }];
        ErrorData::new(
            ErrorCode(-32099),
            format!("action target invalid: {detail}"),
            Some(json!({
                "code": error_codes::ACTION_TARGET_INVALID,
                "tier_attempts": tier_attempts,
            })),
        )
    }

    fn click_attempt(tier: &str, status: &str, error_code: Option<&str>) -> ActClickTierAttempt {
        ActClickTierAttempt {
            tier: tier.to_owned(),
            status: status.to_owned(),
            reason_code: error_code.map(str::to_owned),
            error_code: error_code.map(str::to_owned),
            detail: Some("synthetic regression attempt".to_owned()),
            required_foreground: tier == CLICK_TIER_FOREGROUND,
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
