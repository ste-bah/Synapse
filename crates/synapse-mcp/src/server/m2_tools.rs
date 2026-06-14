use super::{
    ActClickParams, ActClickResponse, ActClipboardParams, ActClipboardResponse,
    ActFocusWindowParams, ActFocusWindowResponse, ActKeymapParams, ActKeymapResponse, ActPadParams,
    ActPadResponse, ActPressParams, ActPressResponse, ActScrollParams, ActScrollResponse,
    ActSetValueParams, ActSetValueResponse, ActStrokeParams, ActStrokeResponse, ActTypeParams,
    ActTypeResponse, ErrorData, Json, Parameters, ReleaseAllParams, ReleaseAllResponse,
    SessionTarget, SynapseService, act_click_with_handle_and_lease, act_clipboard_session_buffer,
    act_focus_window, act_focus_window_request_details, act_focus_window_target_hwnd,
    act_pad_with_handle, act_press_with_handle, act_scroll_with_handle, act_set_value,
    act_set_value_request_details, act_stroke_validation_failure_details, act_stroke_with_handle,
    act_type_with_handle,
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
    ActClickPostcondition, ActClickTierAttempt, ActStrokePlan, CLICK_REASON_NO_OBSERVED_DELTA,
    CLICK_TIER_FOREGROUND, CLICK_TIER_POSTMESSAGE, ForegroundClickPolicy, HwndKeyboardTargetState,
    PressBackend, ResolvedKeymapPress, act_click_postmessage_with_params,
    act_keymap_response_from_press, act_press_cdp_target, act_press_normalized_labels,
    act_press_postmessage_target, act_stroke_cdp_target, act_stroke_error_details,
    act_stroke_request_details, action_from_press_params, action_from_type_params,
    attach_click_tier_attempts, click_params_can_route_background_first, click_target_root_hwnd,
    click_tier_delivered, click_tier_failed, emitted_text, hwnd_keyboard_target_state,
    resolve_keymap_press,
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
    AccessibleNode, Action, AimCurve, Backend, ButtonAction, ElementId, FocusedElement,
    ForegroundContext, KeystrokeDynamics, KeystrokeNaturalParams, MouseButton, MouseTarget,
    PathPoint, PathSpec, Point, Rect, StrokeMotionModel, StrokeTiming, UiaPattern, VelocityProfile,
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
const ACT_TYPE_FOREGROUND_TEXT_SOURCE_OF_TRUTH: &str = "foreground_text_readback";
const ACT_TYPE_TEXT_INTEGRITY_PREFIX: &str = "verify_delta_text_readback";
const ACT_TYPE_TEXT_SOURCE_UIA_VALUE: &str = "uia_focused_value";
const ACT_TYPE_TEXT_SOURCE_UIA_EMPTY: &str = "uia_focused_empty_value_or_text";
const ACT_TYPE_TEXT_SOURCE_CDP_ACTIVE: &str = "cdp_active_element_value";
const ACT_TYPE_TEXT_SOURCE_OCR_FOCUSED_RECT: &str = "ocr_focused_rect_text";
const ACT_TYPE_FOREGROUND_FALLBACK_CLICK_HOLD_MS: u32 = 120;
const ACT_TYPE_FOREGROUND_FALLBACK_CLICK_DURATION_MS: u32 = 50;
const ACT_TYPE_VERIFY_POLL_INTERVAL_MS: u64 = 50;
const BACKGROUND_FOREGROUND_SOURCE_OF_TRUTH: &str = "os_foreground_window";

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
        description = "Click a screen coordinate or UI Automation element. Default element delivery uses background UIA control patterns (Invoke, Toggle, SelectionItem, ExpandCollapse, LegacyIAccessible.DoDefaultAction). When element coordinate delivery is needed, Synapse tries a background HWND PostMessage click to the resolved child window before escalating to the leased foreground coordinate tier; enabled keyboard-focusable edit/document/text or Value/Text targets bypass PostMessage and use the leased foreground coordinate tier so the real caret/focus state is placed. verify_delta reads the target window SoT for element clicks. coordinate_fallback_on_unsupported=true allows bbox-center coordinate delivery only for enabled keyboard-focusable edit/document/text targets or elements exposing Value/Text patterns; set false to fail closed with ACTION_ELEMENT_PATTERN_UNSUPPORTED. This mouse click tool does not synthesize WM_CHAR/dead-key keyboard text; use act_type/act_set_value for text. velocity_profile controls coordinate-move timing only, while explicit spatial paths belong to act_stroke. If a previously observed transient element expired before dispatch, returns TRANSIENT_ELEMENT_EXPIRED with re-observe/find guidance."
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
        let foreground_click_policy = self.foreground_click_policy_for_request(&request_context)?;
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
                foreground_click_policy,
                started,
            )
            .await
        } else {
            act_click_with_handle_and_lease(handle, recording, params, foreground_click_policy)
                .await
        };
        self.audit_action_result_for_request("act_click", &result, &request_context)?;
        result.map(Json)
    }

    #[tool(
        description = "Type text. With into_element, routes through background CDP insertText for web nodes, foreground-safe native HWND text messages for UIA-resolved edit controls, UIA ValuePattern.SetValue with value readback for native elements without a native edit HWND, or a leased foreground click/type fallback for verified Chromium UIA editable targets when CDP is unavailable and the target window is already foreground. Without into_element, types through the leased foreground keyboard backend."
    )]
    pub async fn act_type(
        &self,
        params: Parameters<ActTypeParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActTypeResponse>, ErrorData> {
        let mut params = params.0;
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
        let foreground_fallback =
            match act_type_chromium_foreground_fallback_target(params.into_element.as_ref()) {
                Ok(target) => target,
                Err(error) => {
                    let result: Result<ActTypeResponse, ErrorData> = Err(error);
                    self.audit_action_result_for_request("act_type", &result, &request_context)?;
                    return result.map(Json);
                }
            };
        let requires_foreground_route =
            act_type_requires_foreground_route(&params, foreground_fallback.as_ref());
        let (handle, recording, _connection_closed_cancel) =
            self.m2_action_context_for_request(&request_context)?;
        if requires_foreground_route
            && let Err(error) = self.ensure_act_type_foreground(&preflight, recording.as_ref())
        {
            let result: Result<ActTypeResponse, ErrorData> = Err(error);
            self.audit_action_result_for_request("act_type", &result, &request_context)?;
            return result.map(Json);
        }
        let _lease_guard = if requires_foreground_route {
            match acquire_tool_foreground_input_lease(self, "act_type", &request_context) {
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
        let before_text_signature = if let Some(target) = foreground_fallback.as_ref() {
            let mut foreground_params = params.clone();
            foreground_params.into_element = None;
            if let Err(error) = action_from_type_params(&foreground_params) {
                let result: Result<ActTypeResponse, ErrorData> = Err(error);
                self.audit_action_result_for_request("act_type", &result, &request_context)?;
                return result.map(Json);
            }
            if let Err(error) = self.ensure_act_type_foreground_fallback_target(
                &preflight,
                target,
                recording.as_ref(),
            ) {
                let result: Result<ActTypeResponse, ErrorData> = Err(error);
                self.audit_action_result_for_request("act_type", &result, &request_context)?;
                return result.map(Json);
            }
            if let Err(error) = self
                .click_act_type_foreground_fallback_target(handle.clone(), target)
                .await
            {
                let result: Result<ActTypeResponse, ErrorData> = Err(error);
                self.audit_action_result_for_request("act_type", &result, &request_context)?;
                return result.map(Json);
            }
            let focus_readback = match self.capture_act_type_text_signature(160, true, false).await
            {
                Ok(signature) => signature,
                Err(error) => {
                    let result: Result<ActTypeResponse, ErrorData> = Err(error);
                    self.audit_action_result_for_request("act_type", &result, &request_context)?;
                    return result.map(Json);
                }
            };
            if let Err(error) =
                act_type_foreground_fallback_focus_matches_target(target, &focus_readback.signature)
            {
                let result: Result<ActTypeResponse, ErrorData> = Err(error);
                self.audit_action_result_for_request("act_type", &result, &request_context)?;
                return result.map(Json);
            }
            tracing::info!(
                code = "M2_ACT_TYPE_CHROMIUM_FOREGROUND_FALLBACK_READY",
                element_id = %target.element_id,
                root_hwnd = target.root_hwnd,
                role = %target.role,
                "readback=foreground_text tool=act_type into_element_fallback=chromium_uia_value_pattern_refused"
            );
            params = foreground_params;
            params.verify_delta.then_some(focus_readback)
        } else if act_type_should_capture_text_signature(&params) {
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
        description = "Set a UI Automation element's text/value without foreground. For known native edit HWNDs, routes through Win32 WM_SETTEXT; otherwise routes through UIA ValuePattern.SetValue. Both tiers require a separate target readback from the same Source of Truth and fail closed with probed tier details; there is no keyboard/foreground fallback."
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
        let foreground_guard = match act_set_value_target_foreground_guard(&params.element_id) {
            Ok(guard) => guard,
            Err(error) => {
                let result: Result<ActSetValueResponse, ErrorData> = Err(error);
                self.audit_action_result_for_request("act_set_value", &result, &request_context)?;
                return result.map(Json);
            }
        };
        let foreground_before = match self.current_audit_foreground() {
            Ok(foreground) => foreground,
            Err(error) => {
                let result: Result<ActSetValueResponse, ErrorData> = Err(
                    act_set_value_foreground_read_error("before", "unknown", &error),
                );
                self.audit_action_result_for_request("act_set_value", &result, &request_context)?;
                return result.map(Json);
            }
        };
        let result = act_set_value(params).await;
        let result = match result {
            Ok(response) if !response.required_foreground => {
                match self.current_audit_foreground() {
                    Ok(foreground_after) => match verify_background_target_not_activated(
                        "act_set_value",
                        &response.source_of_truth,
                        foreground_guard,
                        &foreground_before,
                        &foreground_after,
                    ) {
                        Ok(()) => Ok(response),
                        Err(error) => Err(error),
                    },
                    Err(error) => Err(act_set_value_foreground_read_error(
                        "after",
                        &response.source_of_truth,
                        &error,
                    )),
                }
            }
            other => other,
        };
        self.audit_action_result_for_request("act_set_value", &result, &request_context)?;
        result.map(Json)
    }

    #[tool(
        description = "Set a field's text by REPLACING its full content — clear + type + verify in one call (the form-filling primitive; #882). Routing is decided up front from the target: CDP web element ids use a background select-all + insertText replace with a separate node-value readback; Chromium UIA editable targets (when CDP is unavailable) use the leased foreground tier — the target window must already be foreground (act_focus_window first), then click, Ctrl+A, type, and a separate UIA value readback; native elements route through the act_set_value background tiers. Empty text clears the field. Every tier fails closed with its own reason code — there is no cross-tier fallback and no append behavior."
    )]
    pub async fn act_set_field_text(
        &self,
        params: Parameters<crate::m2::ActSetFieldTextParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::m2::ActSetFieldTextResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_set_field_text",
            "tool.invocation kind=act_set_field_text"
        );
        let request_details = crate::m2::act_set_field_text_request_details(&params);
        let preflight = match self.ensure_supported_use_allows_action("act_set_field_text") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied_with_details_for_request(
                    "act_set_field_text",
                    &error,
                    &request_details,
                    &request_context,
                );
                return Err(error);
            }
        };
        self.audit_action_started_with_details_for_request(
            "act_set_field_text",
            &json!({
                "request": request_details,
                "preflight": preflight,
            }),
            &request_context,
        )?;
        if let Err(error) = crate::m2::validate_set_field_text_params(&params) {
            let result: Result<crate::m2::ActSetFieldTextResponse, ErrorData> = Err(error);
            self.audit_action_result_for_request("act_set_field_text", &result, &request_context)?;
            return result.map(Json);
        }
        if let Err(error) = self.ensure_target_claim_allows_action(
            "act_set_field_text",
            element_claim_target(&params.element_id),
            &request_context,
        ) {
            return audit_target_claim_denial(self, "act_set_field_text", error, &request_context);
        }
        let route = match crate::m2::set_field_text_route(&params.element_id) {
            Ok(route) => route,
            Err(error) => {
                let result: Result<crate::m2::ActSetFieldTextResponse, ErrorData> = Err(error);
                self.audit_action_result_for_request(
                    "act_set_field_text",
                    &result,
                    &request_context,
                )?;
                return result.map(Json);
            }
        };
        let result = match route {
            #[cfg(windows)]
            crate::m2::SetFieldTextRoute::Web { backend_node_id } => {
                self.act_set_field_text_background_guarded(&params, |params| {
                    Box::pin(crate::m2::act_set_field_text_web(params, backend_node_id))
                })
                .await
            }
            #[cfg(not(windows))]
            crate::m2::SetFieldTextRoute::Web { .. } => Err(mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "act_set_field_text web tier is only available on Windows",
            )),
            crate::m2::SetFieldTextRoute::ChromiumForeground {
                root_hwnd,
                process_name,
                metadata,
            } => {
                self.act_set_field_text_foreground_tier(
                    &params,
                    root_hwnd,
                    &process_name,
                    metadata,
                    &preflight,
                    &request_context,
                )
                .await
            }
            crate::m2::SetFieldTextRoute::NativeBackground => {
                self.act_set_field_text_background_guarded(&params, |params| {
                    Box::pin(crate::m2::act_set_field_text_native(params))
                })
                .await
            }
        };
        self.audit_action_result_for_request("act_set_field_text", &result, &request_context)?;
        result.map(Json)
    }

    #[tool(
        description = "Operator-intent foreground action: lease-gated focus/activation of one visible top-level native window by exact hwnd, unique title_regex, or unique pid. Do not use as an action/perception precondition. It fails closed on missing, ambiguous, or contended targets and verifies success with a separate GetForegroundWindow readback."
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
            match acquire_tool_foreground_input_lease(self, "act_focus_window", &request_context) {
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

    #[tool(
        description = "Press a keyboard key or ordered chord. With an active session target and backend=auto/software, Synapse first uses background delivery: CDP Input.dispatchKeyEvent for CDP targets or HWND PostMessage keyboard messages for window targets. PostMessage delivery is accepted only after a separate target text/selection readback changes; ignored posted keys fail with ACTION_NO_OBSERVED_DELTA. backend=hardware, recording, no active target, or declared foreground-transition verification uses the leased foreground keyboard path."
    )]
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
        if let Err(error) = action_from_press_params(&params) {
            let result: Result<ActPressResponse, ErrorData> = Err(error);
            self.audit_action_result_for_request("act_press", &result, &request_context)?;
            return result.map(Json);
        }
        let (handle, recording, connection_closed_cancel) =
            self.m2_action_context_for_request(&request_context)?;
        match self
            .try_act_press_background_target(params.clone(), recording.is_some(), &request_context)
            .await
        {
            Ok(Some(response)) => {
                let result = Ok(response);
                self.audit_action_result_for_request("act_press", &result, &request_context)?;
                return result.map(Json);
            }
            Ok(None) => {}
            Err(error) => {
                let result: Result<ActPressResponse, ErrorData> = Err(error);
                self.audit_action_result_for_request("act_press", &result, &request_context)?;
                return result.map(Json);
            }
        }
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
        let _lease_guard = match acquire_tool_foreground_input_lease_with_ttl(
            self,
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

    #[tool(
        description = "Press a keyboard alias from the active profile keymap. With an active session target and backend=auto/software, resolves the alias before any lease and routes through the same background CDP/PostMessage keyboard tiers as act_press; hardware, recording, or no active target uses the leased foreground keyboard path."
    )]
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
        let resolved = match resolve_keymap_press(&profile, &params) {
            Ok(resolved) => resolved,
            Err(error) => {
                let result: Result<ActKeymapResponse, ErrorData> = Err(error);
                self.audit_action_result_for_request("act_keymap", &result, &request_context)?;
                return result.map(Json);
            }
        };
        let (handle, recording, connection_closed_cancel) =
            self.m2_action_context_for_request(&request_context)?;
        match self
            .try_act_keymap_background_target(&resolved, recording.is_some(), &request_context)
            .await
        {
            Ok(Some(response)) => {
                let result = Ok(response);
                self.audit_action_result_for_request("act_keymap", &result, &request_context)?;
                return result.map(Json);
            }
            Ok(None) => {}
            Err(error) => {
                let result: Result<ActKeymapResponse, ErrorData> = Err(error);
                self.audit_action_result_for_request("act_keymap", &result, &request_context)?;
                return result.map(Json);
            }
        }
        let _lease_guard = match acquire_tool_foreground_input_lease_with_ttl(
            self,
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
        let result = act_press_with_handle(
            handle,
            recording,
            connection_closed_cancel,
            resolved.press.clone(),
        )
        .await
        .map(|response| act_keymap_response_from_press(&resolved, response));
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
        let verify_timeout_ms = params.verify_timeout_ms;
        let (handle, recording, _connection_closed_cancel) =
            self.m2_action_context_for_request(&request_context)?;
        match self
            .try_act_stroke_cdp_background_target(
                &params,
                &plan,
                recording.is_some(),
                &request_context,
            )
            .await
        {
            Ok(Some(response)) => {
                self.audit_action_ok_with_details_for_request(
                    "act_stroke",
                    &json!({
                        "response": response,
                        "stroke": stroke_details,
                        "preflight": preflight,
                    }),
                    &request_context,
                )?;
                return Ok(Json(response));
            }
            Ok(None) => {}
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
        let _lease_guard = if should_acquire_act_stroke_input_lease(
            recording.is_some(),
            plan.requires_input_lease(),
        ) {
            match acquire_tool_foreground_input_lease(self, "act_stroke", &request_context) {
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
        let foreground_monitor =
            should_monitor_act_stroke_foreground(recording.is_some(), plan.requires_input_lease())
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
        let foreground_guard = if params.uses_element_target() {
            match act_scroll_target_foreground_guard(&params) {
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
        let foreground_before = if foreground_guard.is_some() {
            match self.current_audit_foreground() {
                Ok(foreground) => Some(foreground),
                Err(error) => {
                    let result: Result<ActScrollResponse, ErrorData> = Err(
                        act_scroll_foreground_read_error("before", "unknown", &error),
                    );
                    self.audit_action_result_for_request("act_scroll", &result, &request_context)?;
                    return result.map(Json);
                }
            }
        } else {
            None
        };
        let _lease_guard = if params.requires_input_lease() {
            match acquire_tool_foreground_input_lease(self, "act_scroll", &request_context) {
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
        let result = match (result, foreground_guard, foreground_before) {
            (Ok(response), Some(guard), Some(before)) if !response.required_foreground => {
                let source_of_truth = response
                    .postcondition
                    .source_of_truth
                    .as_deref()
                    .unwrap_or("act_scroll.background_target");
                match self.current_audit_foreground() {
                    Ok(after) => {
                        match verify_background_target_not_activated(
                            "act_scroll",
                            source_of_truth,
                            guard,
                            &before,
                            &after,
                        ) {
                            Ok(()) => Ok(response),
                            Err(error) => Err(error),
                        }
                    }
                    Err(error) => Err(act_scroll_foreground_read_error(
                        "after",
                        source_of_truth,
                        &error,
                    )),
                }
            }
            (other, _, _) => other,
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

impl SynapseService {
    fn foreground_click_policy_for_request(
        &self,
        request_context: &RequestContext<RoleServer>,
    ) -> Result<ForegroundClickPolicy, ErrorData> {
        let session_id = foreground_lease_session_id(request_context)?;
        let Some(session_id_ref) = session_id.as_deref() else {
            return Ok(ForegroundClickPolicy::allowed(None));
        };
        let Some(hidden_desktop) = self.session_hidden_desktop_readback(session_id_ref)? else {
            return Ok(ForegroundClickPolicy::allowed(session_id));
        };
        Ok(ForegroundClickPolicy::refuse_hidden_desktop(
            session_id_ref.to_owned(),
            hidden_desktop.desktop_names,
        ))
    }
}

fn hidden_desktop_foreground_refusal(
    tool: &'static str,
    hidden_desktop: &super::session_lifecycle::SessionHiddenDesktopReadback,
) -> ErrorData {
    let detail = format!(
        "{tool} cannot use the visible foreground input tier because MCP session {:?} owns hidden desktop(s) {:?}; hidden Win32 desktops are not the active input desktop, so raw SendInput/cursor/foreground activation is refused. Use a background CDP/UIA/PostMessage target route or a separate Windows session/RDP path for raw-input-required apps.",
        hidden_desktop.session_id, hidden_desktop.desktop_names
    );
    ErrorData::new(
        ErrorCode(-32099),
        detail,
        Some(json!({
            "code": error_codes::FOREGROUND_ACTIVATION_REFUSED,
            "reason": "hidden_desktop_foreground_tier_refused",
            "tool": tool,
            "session_id": hidden_desktop.session_id,
            "desktop_names": hidden_desktop.desktop_names,
            "launch_pids": hidden_desktop.launch_pids,
            "resource_count": hidden_desktop.resource_count,
            "foreground_tier_allowed": false,
        })),
    )
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
    service: &SynapseService,
    tool: &'static str,
    request_context: &RequestContext<RoleServer>,
) -> Result<crate::m2::ForegroundInputLeaseGuard, ErrorData> {
    acquire_tool_foreground_input_lease_with_ttl(
        service,
        tool,
        request_context,
        synapse_action::DEFAULT_LEASE_TTL_MS,
    )
}

fn acquire_tool_foreground_input_lease_with_ttl(
    service: &SynapseService,
    tool: &'static str,
    request_context: &RequestContext<RoleServer>,
    ttl_ms: u64,
) -> Result<crate::m2::ForegroundInputLeaseGuard, ErrorData> {
    // `None` here means the stdio transport by construction — an HTTP request
    // without Mcp-Session-Id fails hard upstream. stdio is single-client, so
    // the stable "stdio" owner (the same idiom the m3 layer uses) gives the
    // lease registry a real owner instead of refusing every foreground tier
    // over stdio.
    let session_id =
        foreground_lease_session_id(request_context)?.unwrap_or_else(|| "stdio".to_owned());
    if let Some(hidden_desktop) = service.session_hidden_desktop_readback(&session_id)? {
        return Err(hidden_desktop_foreground_refusal(tool, &hidden_desktop));
    }
    crate::m2::acquire_foreground_input_lease_with_ttl(tool, Some(&session_id), ttl_ms)
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CdpKeyboardDeltaSignature {
    target_id: String,
    has_active_element: bool,
    tag_name: String,
    id_sha256: Option<String>,
    name_sha256: Option<String>,
    value_len: usize,
    value_sha256: String,
    selection_start: Option<u32>,
    selection_end: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct HwndKeyboardDeltaSignature {
    target: HwndKeyboardTargetState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum HwndKeyboardExpectedEffect {
    AnyDelta,
    PrintableText { text: String },
    SelectAll,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ActTypeForegroundFallbackTarget {
    element_id: String,
    root_hwnd: i64,
    process_name: String,
    role: String,
    automation_id_present: bool,
    bbox: Rect,
    enabled: bool,
    keyboard_focusable: bool,
    patterns: Vec<UiaPattern>,
    name_len: usize,
    value_len: Option<usize>,
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
    text_readback_attempts: Vec<String>,
    cdp_status: Option<String>,
    cdp_endpoint_present: bool,
    cdp_selected_target_id: Option<String>,
    cdp_active_has_element: Option<bool>,
    cdp_active_is_editable: Option<bool>,
    cdp_active_tag_name: Option<String>,
    cdp_active_id_sha256: Option<String>,
    cdp_active_name_sha256: Option<String>,
    cdp_active_value_len: Option<usize>,
    cdp_active_value_sha256: Option<String>,
    cdp_active_error_code: Option<String>,
    cdp_active_error_detail_sha256: Option<String>,
    ocr_word_count: usize,
    ocr_text_len: Option<usize>,
    ocr_text_sha256: Option<String>,
    web_path: Option<String>,
    browser_url_len: Option<usize>,
    browser_url_sha256: Option<String>,
    browser_cdp_target_id: Option<String>,
    browser_url_readback_source: Option<String>,
}

#[derive(Clone, Debug)]
struct CdpActiveTextReadback {
    value: Option<String>,
    target_id: Option<String>,
    has_active_element: Option<bool>,
    is_editable: Option<bool>,
    tag_name: Option<String>,
    id_sha256: Option<String>,
    name_sha256: Option<String>,
    value_len: Option<usize>,
    value_sha256: Option<String>,
    error_code: Option<String>,
    error_detail_sha256: Option<String>,
    attempt: String,
}

#[derive(Clone, Debug)]
struct OcrTextReadback {
    value: Option<String>,
    word_count: usize,
    value_len: Option<usize>,
    value_sha256: Option<String>,
    attempt: String,
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
        foreground_click_policy: ForegroundClickPolicy,
        started: Instant,
    ) -> Result<ActClickResponse, ErrorData> {
        match act_click_with_handle_and_lease(
            handle.clone(),
            recording.clone(),
            params.clone(),
            foreground_click_policy.clone(),
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
                                foreground_click_policy,
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
                                foreground_click_policy,
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
                        foreground_click_policy,
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
                        foreground_click_policy,
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
        foreground_click_policy: ForegroundClickPolicy,
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
                                foreground_click_policy,
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
                        foreground_click_policy,
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
        foreground_click_policy: ForegroundClickPolicy,
    ) -> Result<ActClickResponse, ErrorData> {
        params.use_invoke_pattern = false;
        match act_click_with_handle_and_lease(handle, recording, params, foreground_click_policy)
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
        let (uia_value, uia_readback_source) = focused_text_readback(focused.as_ref());
        let cdp_readback = cdp_active_text_readback(&input).await;
        let ocr_readback = ocr_focused_rect_text_readback(focused.as_ref(), &input.elements);
        let mut text_readback_attempts = Vec::new();
        text_readback_attempts.push(match uia_readback_source {
            Some(source) => format!("{source}:available"),
            None => "uia_focused_value:unavailable".to_owned(),
        });
        text_readback_attempts.push(cdp_readback.attempt.clone());
        text_readback_attempts.push(ocr_readback.attempt.clone());
        let (value, readback_source) = choose_act_type_text_readback(
            focused.as_ref(),
            uia_value,
            uia_readback_source,
            &cdp_readback,
            &ocr_readback,
        );
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
            readback_source,
            has_text_readback: value.is_some(),
            text_readback_attempts,
            cdp_status: input.cdp.as_ref().map(|cdp| cdp.status.as_str().to_owned()),
            cdp_endpoint_present: input.cdp.as_ref().is_some_and(|cdp| cdp.endpoint.is_some()),
            cdp_selected_target_id: cdp_readback.target_id.clone().or_else(|| {
                input
                    .cdp
                    .as_ref()
                    .and_then(|cdp| cdp.selected_target_id.clone())
            }),
            cdp_active_has_element: cdp_readback.has_active_element,
            cdp_active_is_editable: cdp_readback.is_editable,
            cdp_active_tag_name: cdp_readback.tag_name,
            cdp_active_id_sha256: cdp_readback.id_sha256,
            cdp_active_name_sha256: cdp_readback.name_sha256,
            cdp_active_value_len: cdp_readback.value_len,
            cdp_active_value_sha256: cdp_readback.value_sha256,
            cdp_active_error_code: cdp_readback.error_code,
            cdp_active_error_detail_sha256: cdp_readback.error_detail_sha256,
            ocr_word_count: ocr_readback.word_count,
            ocr_text_len: ocr_readback.value_len,
            ocr_text_sha256: ocr_readback.value_sha256,
            web_path: input.web_path.map(|path| path.as_str().to_owned()),
            browser_url_len: browser_url.url.as_ref().map(|url| url.chars().count()),
            browser_url_sha256: browser_url.url.as_deref().and_then(non_empty_sha256),
            browser_cdp_target_id: browser_url.target_id.clone(),
            browser_url_readback_source: browser_url.source,
        };
        if require_focused_text_value && value.is_none() {
            let signature_hash = verify_hash_json(&signature)?;
            return Err(act_type_verify_surface_unavailable_error(
                "no UIA Value/Text, CDP active-element, or focused-rectangle OCR text readback was available for act_type verify_delta",
                signature_hash,
                signature,
            ));
        }

        Ok(ActTypeTextReadback {
            signature,
            value,
            browser_url: browser_url.url,
        })
    }

    fn ensure_act_type_foreground_fallback_target(
        &self,
        preflight: &ActionPreflightReadback,
        target: &ActTypeForegroundFallbackTarget,
        recording: Option<&Arc<RecordingBackend>>,
    ) -> Result<(), ErrorData> {
        if recording.is_some() {
            return Err(act_type_foreground_fallback_recording_error(target));
        }
        let expected = preflight.after.as_ref().unwrap_or(&preflight.before);
        if expected.hwnd != target.root_hwnd {
            return Err(act_type_foreground_fallback_target_not_foreground_error(
                expected, target,
            ));
        }
        Ok(())
    }

    async fn click_act_type_foreground_fallback_target(
        &self,
        handle: ActionHandle,
        target: &ActTypeForegroundFallbackTarget,
    ) -> Result<(), ErrorData> {
        let point = act_type_target_center_point(target)?;
        let actions = [
            Action::MouseMove {
                to: MouseTarget::Screen { point },
                curve: AimCurve::Instant,
                duration_ms: ACT_TYPE_FOREGROUND_FALLBACK_CLICK_DURATION_MS,
                backend: Backend::Auto,
            },
            Action::MouseButton {
                button: MouseButton::Left,
                action: ButtonAction::Press,
                hold_ms: ACT_TYPE_FOREGROUND_FALLBACK_CLICK_HOLD_MS,
                backend: Backend::Auto,
            },
        ];
        for action in actions {
            handle
                .execute(action)
                .await
                .map_err(|error| act_type_foreground_fallback_click_error(target, point, &error))?;
        }
        tracing::info!(
            code = "M2_ACT_TYPE_CHROMIUM_FOREGROUND_FALLBACK_CLICKED",
            element_id = %target.element_id,
            root_hwnd = target.root_hwnd,
            screen_x = point.x,
            screen_y = point.y,
            role = %target.role,
            "readback=foreground_click tool=act_type into_element_fallback=chromium_uia_value_pattern_refused"
        );
        Ok(())
    }

    /// Runs one background `act_set_field_text` tier inside the same
    /// no-foreground-steal guard `act_set_value` uses: the OS foreground is
    /// read before and after, and an activation of the target window fails
    /// the call (epic #771).
    async fn act_set_field_text_background_guarded<'params, Run>(
        &self,
        params: &'params crate::m2::ActSetFieldTextParams,
        run: Run,
    ) -> Result<crate::m2::ActSetFieldTextResponse, ErrorData>
    where
        Run: FnOnce(
            &'params crate::m2::ActSetFieldTextParams,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<crate::m2::ActSetFieldTextResponse, ErrorData>,
                    > + Send
                    + 'params,
            >,
        >,
    {
        let foreground_guard = act_set_value_target_foreground_guard(&params.element_id)?;
        let foreground_before = self
            .current_audit_foreground()
            .map_err(|error| act_set_value_foreground_read_error("before", "unknown", &error))?;
        let response = run(params).await?;
        let foreground_after = self.current_audit_foreground().map_err(|error| {
            act_set_value_foreground_read_error("after", &response.source_of_truth, &error)
        })?;
        verify_background_target_not_activated(
            "act_set_field_text",
            &response.source_of_truth,
            foreground_guard,
            &foreground_before,
            &foreground_after,
        )?;
        Ok(response)
    }

    /// Leased foreground tier for Chromium UIA editable targets: click the
    /// element, prove focus landed on it, Ctrl+A, type (or Delete for empty
    /// text), then a separate UIA value readback must equal the requested
    /// text. The target window must already be foreground — Synapse never
    /// activates it implicitly (epic #771).
    async fn act_set_field_text_foreground_tier(
        &self,
        params: &crate::m2::ActSetFieldTextParams,
        root_hwnd: i64,
        process_name: &str,
        metadata: synapse_a11y::ElementMetadataReadback,
        preflight: &ActionPreflightReadback,
        request_context: &RequestContext<RoleServer>,
    ) -> Result<crate::m2::ActSetFieldTextResponse, ErrorData> {
        let started = Instant::now();
        let target = ActTypeForegroundFallbackTarget {
            element_id: params.element_id.to_string(),
            root_hwnd,
            process_name: process_name.to_owned(),
            role: metadata.role.clone(),
            automation_id_present: metadata.automation_id.is_some(),
            bbox: metadata.bbox,
            enabled: metadata.enabled,
            keyboard_focusable: metadata.keyboard_focusable,
            patterns: metadata.patterns.clone(),
            name_len: metadata.name.chars().count(),
            value_len: metadata.value.as_ref().map(|value| value.chars().count()),
        };
        let (handle, recording, _connection_closed_cancel) =
            self.m2_action_context_for_request(request_context)?;
        if recording.is_some() {
            return Err(set_field_text_foreground_error(
                &target,
                error_codes::ACTION_BACKEND_UNAVAILABLE,
                "foreground_tier_recording_backend_unsupported",
                "act_set_field_text Chromium foreground tier requires the live foreground input tier and cannot run against the recording backend",
            ));
        }
        let expected = preflight.after.as_ref().unwrap_or(&preflight.before);
        if expected.hwnd != target.root_hwnd {
            return Err(set_field_text_foreground_error(
                &target,
                error_codes::ACTION_FOREGROUND_LOST,
                "target_window_not_foreground",
                format!(
                    "act_set_field_text Chromium foreground tier requires target hwnd 0x{:x} to be the current foreground hwnd, but preflight foreground was 0x{:x}; call act_focus_window first — Synapse never activates a window implicitly",
                    target.root_hwnd, expected.hwnd
                ),
            ));
        }

        let before = synapse_a11y::element_value(&params.element_id).map_err(|error| {
            set_field_text_foreground_error(
                &target,
                error.code(),
                "before_value_read_failed",
                format!(
                    "act_set_field_text before-value UIA readback failed for element {}: {error}",
                    params.element_id
                ),
            )
        })?;

        let _lease_guard =
            acquire_tool_foreground_input_lease(self, "act_set_field_text", request_context)?;

        // Actionability before the coordinate click (the Playwright `fill`
        // discipline): scroll an off-viewport target into view, re-read its
        // live bbox, and require the click point to land inside the target
        // window — a stale below-the-fold bbox must never steer a foreground
        // click into another window.
        let mut target = target;
        if metadata.patterns.contains(&UiaPattern::ScrollItem) {
            synapse_a11y::scroll_element_into_view(&params.element_id).map_err(|error| {
                set_field_text_foreground_error(
                    &target,
                    error.code(),
                    "scroll_into_view_failed",
                    format!(
                        "act_set_field_text ScrollItemPattern scroll-into-view failed for element {}: {error}",
                        params.element_id
                    ),
                )
            })?;
            target.bbox = synapse_a11y::element_bounding_rect(&params.element_id).map_err(
                |error| {
                    set_field_text_foreground_error(
                        &target,
                        error.code(),
                        "post_scroll_bbox_read_failed",
                        format!(
                            "act_set_field_text post-scroll bounding-rect readback failed for element {}: {error}",
                            params.element_id
                        ),
                    )
                },
            )?;
            tracing::info!(
                code = "M2_ACT_SET_FIELD_TEXT_SCROLLED_INTO_VIEW",
                element_id = %params.element_id,
                bbox_x = target.bbox.x,
                bbox_y = target.bbox.y,
                bbox_w = target.bbox.w,
                bbox_h = target.bbox.h,
                "readback=scroll_into_view tool=act_set_field_text"
            );
        }
        let point = act_type_target_center_point(&target)?;
        let window_bounds = synapse_a11y::foreground_context(root_hwnd)
            .map_err(|error| {
                set_field_text_foreground_error(
                    &target,
                    error.code(),
                    "window_bounds_read_failed",
                    format!(
                        "act_set_field_text target window bounds readback failed for hwnd 0x{root_hwnd:x}: {error}"
                    ),
                )
            })?
            .window_bounds;
        if !rect_contains_point(window_bounds, point) {
            return Err(set_field_text_foreground_error(
                &target,
                error_codes::ACTION_TARGET_INVALID,
                "target_click_point_outside_window",
                format!(
                    "act_set_field_text click point ({}, {}) is outside the target window bounds ({}, {}, {}x{}); the element is not visible in the viewport and could not be scrolled into it",
                    point.x,
                    point.y,
                    window_bounds.x,
                    window_bounds.y,
                    window_bounds.w,
                    window_bounds.h
                ),
            ));
        }
        let click_actions = [
            Action::MouseMove {
                to: MouseTarget::Screen { point },
                curve: AimCurve::Instant,
                duration_ms: ACT_TYPE_FOREGROUND_FALLBACK_CLICK_DURATION_MS,
                backend: Backend::Auto,
            },
            Action::MouseButton {
                button: MouseButton::Left,
                action: ButtonAction::Press,
                hold_ms: ACT_TYPE_FOREGROUND_FALLBACK_CLICK_HOLD_MS,
                backend: Backend::Auto,
            },
        ];
        for action in click_actions {
            handle.execute(action).await.map_err(|error| {
                set_field_text_foreground_error(
                    &target,
                    error.code(),
                    "target_click_failed",
                    format!(
                        "act_set_field_text foreground click at ({}, {}) failed: {error}",
                        point.x, point.y
                    ),
                )
            })?;
        }
        tracing::info!(
            code = "M2_ACT_SET_FIELD_TEXT_FOREGROUND_CLICKED",
            element_id = %target.element_id,
            root_hwnd = target.root_hwnd,
            screen_x = point.x,
            screen_y = point.y,
            role = %target.role,
            "readback=foreground_click tool=act_set_field_text tier=foreground_keys"
        );

        let focus_readback = self
            .capture_act_type_text_signature(160, false, false)
            .await?;
        act_type_foreground_fallback_focus_matches_target(&target, &focus_readback.signature)?;

        let select_all = crate::m2::select_all_chord_action(60, Backend::Auto)?;
        handle.execute(select_all).await.map_err(|error| {
            set_field_text_foreground_error(
                &target,
                error.code(),
                "select_all_failed",
                format!("act_set_field_text Ctrl+A select-all failed: {error}"),
            )
        })?;

        let (method, replace_action) = if params.text.is_empty() {
            (
                crate::m2::METHOD_FOREGROUND_CLEAR,
                crate::m2::delete_key_action(40, Backend::Auto)?,
            )
        } else {
            (
                crate::m2::METHOD_FOREGROUND_REPLACE,
                Action::TypeText {
                    text: params.text.clone(),
                    dynamics: KeystrokeDynamics::Natural {
                        params: KeystrokeNaturalParams::FAST,
                    },
                    backend: Backend::Auto,
                },
            )
        };
        handle.execute(replace_action).await.map_err(|error| {
            set_field_text_foreground_error(
                &target,
                error.code(),
                "replacement_input_failed",
                format!("act_set_field_text foreground replacement input failed: {error}"),
            )
        })?;

        tokio::time::sleep(Duration::from_millis(u64::from(params.verify_timeout_ms))).await;
        let after = synapse_a11y::element_value(&params.element_id).map_err(|error| {
            set_field_text_foreground_error(
                &target,
                error.code(),
                "after_value_read_failed",
                format!(
                    "act_set_field_text after-value UIA readback failed for element {}: {error}",
                    params.element_id
                ),
            )
        })?;

        if before.is_password || after.is_password {
            return set_field_text_password_response(params, started, method, &before, &after);
        }
        crate::m2::finish_replace_response(
            params,
            started,
            method,
            crate::m2::TIER_FOREGROUND_KEYS,
            true,
            crate::m2::SOURCE_UIA_VALUE,
            &before.value,
            &after.value,
            json!({
                "element_id": params.element_id.to_string(),
                "root_hwnd": target.root_hwnd,
                "process_name": target.process_name,
                "role": target.role,
                "click_point": { "x": point.x, "y": point.y },
                "focused_element_id": focus_readback.signature.focused_element_id,
            }),
        )
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
        response: ActTypeResponse,
        before: ActTypeTextReadback,
        verify_timeout_ms: u32,
        emitted: &str,
        browser_url_policy: Option<&ActTypeBrowserUrlPolicy>,
    ) -> Result<ActTypeResponse, ErrorData> {
        let started = Instant::now();
        let timeout = Duration::from_millis(u64::from(verify_timeout_ms));
        let poll_interval = Duration::from_millis(ACT_TYPE_VERIFY_POLL_INTERVAL_MS);
        let mut last_error: Option<ErrorData> = None;

        loop {
            let elapsed = started.elapsed();
            if elapsed >= timeout {
                break;
            }
            tokio::time::sleep(std::cmp::min(poll_interval, timeout - elapsed)).await;

            let after = self
                .capture_act_type_text_signature(160, false, browser_url_policy.is_some())
                .await?;
            let before_hash = verify_hash_json(&before.signature)?;
            let after_hash = verify_hash_json(&after.signature)?;
            let result = if let Some(policy) = browser_url_policy {
                verify_act_type_browser_url_response(
                    response.clone(),
                    before.clone(),
                    after,
                    before_hash,
                    after_hash,
                    verify_timeout_ms,
                    policy,
                )
            } else {
                let terminal_failure = act_type_text_terminal_failure(&before, &after);
                let result = verify_act_type_text_response(
                    response.clone(),
                    before.clone(),
                    after,
                    before_hash,
                    after_hash,
                    verify_timeout_ms,
                    emitted,
                );
                if terminal_failure && result.is_err() {
                    return result;
                }
                result
            };

            match result {
                Ok(response) => return Ok(response),
                Err(error) => {
                    last_error = Some(error);
                    if started.elapsed() >= timeout {
                        break;
                    }
                }
            }
        }

        let after = match self
            .capture_act_type_text_signature(160, false, browser_url_policy.is_some())
            .await
        {
            Ok(after) => after,
            Err(error) => return Err(last_error.unwrap_or(error)),
        };
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
        verify_act_type_text_response(
            response,
            before,
            after,
            before_hash,
            after_hash,
            verify_timeout_ms,
            emitted,
        )
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

    async fn try_act_press_background_target(
        &self,
        params: ActPressParams,
        recording_active: bool,
        request_context: &RequestContext<RoleServer>,
    ) -> Result<Option<ActPressResponse>, ErrorData> {
        if !press_background_target_candidate(&params, recording_active) {
            return Ok(None);
        }
        let session_id = super::context::mcp_session_id_from_request_context(request_context)?;
        let Some(target) = self.action_session_target_override(
            params.window_hwnd,
            params.cdp_target_id.as_deref(),
            session_id.as_deref(),
        )?
        else {
            return Ok(None);
        };
        match target {
            SessionTarget::Cdp {
                window_hwnd,
                cdp_target_id,
            } => self
                .act_press_cdp_background_target(window_hwnd, cdp_target_id, params)
                .await
                .map(Some),
            SessionTarget::Window { hwnd } => self
                .act_press_postmessage_background_target(hwnd, params)
                .await
                .map(Some),
        }
    }

    async fn try_act_keymap_background_target(
        &self,
        resolved: &ResolvedKeymapPress,
        recording_active: bool,
        request_context: &RequestContext<RoleServer>,
    ) -> Result<Option<ActKeymapResponse>, ErrorData> {
        self.try_act_press_background_target(
            resolved.press.clone(),
            recording_active,
            request_context,
        )
        .await
        .map(|response| response.map(|response| act_keymap_response_from_press(resolved, response)))
    }

    async fn try_act_stroke_cdp_background_target(
        &self,
        params: &ActStrokeParams,
        plan: &ActStrokePlan,
        recording_active: bool,
        request_context: &RequestContext<RoleServer>,
    ) -> Result<Option<ActStrokeResponse>, ErrorData> {
        if recording_active || params.requests_hardware_backend() {
            return Ok(None);
        }
        let Some(session_id) =
            super::context::mcp_session_id_from_request_context(request_context)?
        else {
            return Ok(None);
        };
        let Some(target) = self.session_target(Some(&session_id))? else {
            return Ok(None);
        };
        let SessionTarget::Cdp {
            window_hwnd,
            cdp_target_id,
        } = target
        else {
            return Ok(None);
        };
        if plan.is_cdp_element_aim() {
            return Ok(None);
        }
        if !plan.can_try_cdp_target_stroke() {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                "act_stroke active CDP targets require an explicit path for background mouse strokes; refusing to fall back to the real cursor for a browser target",
            ));
        }
        let endpoint = synapse_a11y::endpoint_for_window(window_hwnd).ok_or_else(|| {
            mcp_error(
                error_codes::A11Y_CDP_UNREACHABLE,
                format!(
                    "act_stroke background CDP target requires a reachable raw CDP endpoint for window_hwnd {window_hwnd:#x}; the normal chrome.tabs bridge cannot dispatch Input.dispatchMouseEvent"
                ),
            )
        })?;
        act_stroke_cdp_target(&endpoint, &cdp_target_id, params.clone(), plan.clone())
            .await
            .map(Some)
    }

    async fn act_press_cdp_background_target(
        &self,
        window_hwnd: i64,
        cdp_target_id: String,
        params: ActPressParams,
    ) -> Result<ActPressResponse, ErrorData> {
        let endpoint = synapse_a11y::endpoint_for_window(window_hwnd).ok_or_else(|| {
            mcp_error(
                error_codes::A11Y_CDP_UNREACHABLE,
                format!(
                    "act_press background CDP target requires a reachable CDP endpoint for window_hwnd {window_hwnd:#x}"
                ),
            )
        })?;
        let before = if params.verify_delta {
            Some(
                self.capture_cdp_keyboard_delta_signature(&endpoint, &cdp_target_id)
                    .await?,
            )
        } else {
            None
        };
        let verify_timeout_ms = params.verify_timeout_ms;
        let mut response = act_press_cdp_target(&endpoint, &cdp_target_id, params).await?;
        if let Some(before) = before {
            tokio::time::sleep(Duration::from_millis(u64::from(verify_timeout_ms))).await;
            let after = self
                .capture_cdp_keyboard_delta_signature(&endpoint, &cdp_target_id)
                .await?;
            response.postcondition = verify_keyboard_delta_signature(
                "act_press",
                "cdp_active_element_value_or_selection",
                verify_timeout_ms,
                before,
                after,
                "observed CDP target active-element value/selection change after Input.dispatchKeyEvent delivery",
            )?;
        }
        Ok(response)
    }

    async fn act_press_postmessage_background_target(
        &self,
        root_hwnd: i64,
        params: ActPressParams,
    ) -> Result<ActPressResponse, ErrorData> {
        let expected_effect = hwnd_keyboard_expected_effect(&params)?;
        let before = self.capture_hwnd_keyboard_delta_signature(root_hwnd)?;
        let verify_timeout_ms = params.verify_timeout_ms;
        let mut response = act_press_postmessage_target(root_hwnd, params).await?;
        tokio::time::sleep(Duration::from_millis(u64::from(verify_timeout_ms))).await;
        let after = self.capture_hwnd_keyboard_delta_signature(root_hwnd)?;
        response.postcondition = verify_hwnd_keyboard_delta_signature(
            "act_press",
            "target_hwnd_text_or_selection",
            verify_timeout_ms,
            before,
            after,
            expected_effect,
            "observed target HWND text/selection change after PostMessage keyboard delivery",
        )?;
        Ok(response)
    }

    async fn capture_cdp_keyboard_delta_signature(
        &self,
        endpoint: &str,
        cdp_target_id: &str,
    ) -> Result<CdpKeyboardDeltaSignature, ErrorData> {
        let state = synapse_a11y::cdp_active_element_state(endpoint, cdp_target_id)
            .await
            .map_err(|error| {
                mcp_error(
                    error.code(),
                    format!(
                        "act_press CDP active-element Source-of-Truth readback failed for target {cdp_target_id:?}: {error}"
                    ),
                )
            })?;
        Ok(cdp_keyboard_delta_signature(state))
    }

    fn capture_hwnd_keyboard_delta_signature(
        &self,
        root_hwnd: i64,
    ) -> Result<HwndKeyboardDeltaSignature, ErrorData> {
        Ok(HwndKeyboardDeltaSignature {
            target: hwnd_keyboard_target_state(root_hwnd)?,
        })
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

fn verify_keyboard_delta_signature<T>(
    tool: &str,
    source_of_truth: &str,
    timeout_ms: u32,
    before: T,
    after: T,
    success_detail: &str,
) -> Result<ActPostcondition, ErrorData>
where
    T: Serialize + PartialEq,
{
    let before_hash = hash_json(&before)?;
    let after_hash = hash_json(&after)?;
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
        success_detail,
    ))
}

fn verify_hwnd_keyboard_delta_signature(
    tool: &str,
    source_of_truth: &str,
    timeout_ms: u32,
    before: HwndKeyboardDeltaSignature,
    after: HwndKeyboardDeltaSignature,
    expected_effect: HwndKeyboardExpectedEffect,
    success_detail: &str,
) -> Result<ActPostcondition, ErrorData> {
    let before_hash = hash_json(&before)?;
    let after_hash = hash_json(&after)?;
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
                "expected_effect": hwnd_keyboard_expected_effect_name(&expected_effect),
            }),
        ));
    }
    if let Some(reason) = hwnd_keyboard_effect_mismatch(&before, &after, &expected_effect) {
        return Err(postcondition_failed_error(
            tool,
            source_of_truth,
            reason,
            before_hash,
            after_hash,
            json!({
                "before": before,
                "after": after,
                "expected_effect": hwnd_keyboard_expected_effect_name(&expected_effect),
            }),
        ));
    }
    Ok(postcondition_observed_delta(
        tool,
        source_of_truth,
        before_hash,
        after_hash,
        success_detail,
    ))
}

fn hwnd_keyboard_expected_effect(
    params: &ActPressParams,
) -> Result<HwndKeyboardExpectedEffect, ErrorData> {
    let labels = act_press_normalized_labels(params)?;
    if labels == ["ctrl", "a"] {
        return Ok(HwndKeyboardExpectedEffect::SelectAll);
    }
    let has_command_modifier = labels
        .iter()
        .any(|label| matches!(label.as_str(), "ctrl" | "alt" | "super"));
    if !has_command_modifier && labels.len() == 1 {
        if let Some(text) = hwnd_printable_text_for_label(&labels[0]) {
            return Ok(HwndKeyboardExpectedEffect::PrintableText { text });
        }
    }
    Ok(HwndKeyboardExpectedEffect::AnyDelta)
}

fn hwnd_printable_text_for_label(label: &str) -> Option<String> {
    if label.len() == 1 && label.as_bytes()[0].is_ascii_alphanumeric() {
        return Some(label.to_owned());
    }
    match label {
        "`" => Some("`".to_owned()),
        "space" => Some(" ".to_owned()),
        _ => None,
    }
}

fn hwnd_keyboard_effect_mismatch(
    before: &HwndKeyboardDeltaSignature,
    after: &HwndKeyboardDeltaSignature,
    expected_effect: &HwndKeyboardExpectedEffect,
) -> Option<&'static str> {
    match expected_effect {
        HwndKeyboardExpectedEffect::AnyDelta => None,
        HwndKeyboardExpectedEffect::SelectAll => {
            if !same_hwnd_keyboard_target(before, after) {
                return Some("target HWND changed while verifying Ctrl+A select-all delivery");
            }
            if before.target.text_len != after.target.text_len
                || before.target.text_sha256 != after.target.text_sha256
            {
                return Some("Ctrl+A select-all changed target text instead of preserving it");
            }
            if !selection_covers_text(&after.target) {
                return Some("Ctrl+A select-all did not select the full target text");
            }
            None
        }
        HwndKeyboardExpectedEffect::PrintableText { text } => {
            if !same_hwnd_keyboard_target(before, after) {
                return Some("target HWND changed while verifying printable key delivery");
            }
            if before.target.text_sha256 == after.target.text_sha256 {
                return Some("printable key did not change target text");
            }
            if selection_covers_text(&before.target) {
                let expected_len = text.chars().count();
                let Ok(expected_len_u32) = u32::try_from(expected_len) else {
                    return Some("printable key expected text length exceeded u32::MAX");
                };
                let expected_sha256 = text_sha256(text);
                if after.target.text_len != Some(expected_len)
                    || after.target.text_sha256.as_deref() != Some(expected_sha256.as_str())
                    || after.target.selection_start != Some(expected_len_u32)
                    || after.target.selection_end != Some(expected_len_u32)
                {
                    return Some(
                        "printable key after full selection did not replace target text with the emitted character",
                    );
                }
            }
            None
        }
    }
}

fn same_hwnd_keyboard_target(
    before: &HwndKeyboardDeltaSignature,
    after: &HwndKeyboardDeltaSignature,
) -> bool {
    before.target.root_hwnd == after.target.root_hwnd
        && before.target.hwnd == after.target.hwnd
        && before.target.class_name == after.target.class_name
}

fn selection_covers_text(target: &HwndKeyboardTargetState) -> bool {
    let Some(text_len) = target.text_len else {
        return false;
    };
    let Ok(text_len) = u32::try_from(text_len) else {
        return false;
    };
    target.selection_start == Some(0) && target.selection_end == Some(text_len)
}

fn hwnd_keyboard_expected_effect_name(
    expected_effect: &HwndKeyboardExpectedEffect,
) -> &'static str {
    match expected_effect {
        HwndKeyboardExpectedEffect::AnyDelta => "any_delta",
        HwndKeyboardExpectedEffect::PrintableText { .. } => "printable_text",
        HwndKeyboardExpectedEffect::SelectAll => "select_all",
    }
}

fn cdp_keyboard_delta_signature(
    state: synapse_a11y::CdpActiveElementState,
) -> CdpKeyboardDeltaSignature {
    CdpKeyboardDeltaSignature {
        target_id: state.target_id,
        has_active_element: state.has_active_element,
        tag_name: state.tag_name,
        id_sha256: non_empty_sha256(&state.id),
        name_sha256: non_empty_sha256(&state.name),
        value_len: state.value.chars().count(),
        value_sha256: text_sha256(&state.value),
        selection_start: state.selection_start,
        selection_end: state.selection_end,
    }
}

fn press_background_target_candidate(params: &ActPressParams, recording_active: bool) -> bool {
    if recording_active {
        return false;
    }
    if !matches!(params.backend, PressBackend::Auto | PressBackend::Software) {
        return false;
    }
    !params.allow_foreground_change
        && params.expected_foreground_process_regex.is_none()
        && params.expected_foreground_title_regex.is_none()
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

fn act_type_should_capture_text_signature(params: &ActTypeParams) -> bool {
    params.verify_delta && params.into_element.is_none()
}

fn act_type_requires_foreground_route(
    params: &ActTypeParams,
    fallback_target: Option<&ActTypeForegroundFallbackTarget>,
) -> bool {
    params.into_element.is_none() || fallback_target.is_some()
}

#[cfg(windows)]
fn act_type_chromium_foreground_fallback_target(
    element_id: Option<&ElementId>,
) -> Result<Option<ActTypeForegroundFallbackTarget>, ErrorData> {
    let Some(element_id) = element_id else {
        return Ok(None);
    };
    if synapse_a11y::cdp_backend_from_element_id(element_id).is_some() {
        return Ok(None);
    }
    let root_hwnd = element_id
        .parts()
        .map_err(|err| {
            mcp_error(
                error_codes::ACTION_ELEMENT_NOT_RESOLVED,
                format!("act_type into_element id is malformed: {err}"),
            )
        })?
        .hwnd;
    let context = synapse_a11y::foreground_context(root_hwnd).map_err(|error| {
        mcp_error(
            error.code(),
            format!(
                "act_type into_element foreground fallback target HWND readback failed: {error}"
            ),
        )
    })?;
    if !synapse_a11y::is_chromium_family(&context.process_name) {
        return Ok(None);
    }
    let metadata = synapse_a11y::element_metadata(element_id).map_err(|error| {
        mcp_error(
            error.code(),
            format!(
                "act_type into_element foreground fallback target metadata readback failed: {error}"
            ),
        )
    })?;
    if !chromium_editable_value_pattern_requires_foreground_fallback(
        &context.process_name,
        &metadata,
    ) {
        return Ok(None);
    }
    Ok(Some(ActTypeForegroundFallbackTarget {
        element_id: element_id.to_string(),
        root_hwnd,
        process_name: context.process_name,
        role: metadata.role,
        automation_id_present: metadata.automation_id.is_some(),
        bbox: metadata.bbox,
        enabled: metadata.enabled,
        keyboard_focusable: metadata.keyboard_focusable,
        patterns: metadata.patterns,
        name_len: metadata.name.chars().count(),
        value_len: metadata.value.as_ref().map(|value| value.chars().count()),
    }))
}

#[cfg(not(windows))]
fn act_type_chromium_foreground_fallback_target(
    _element_id: Option<&ElementId>,
) -> Result<Option<ActTypeForegroundFallbackTarget>, ErrorData> {
    Ok(None)
}

fn chromium_editable_value_pattern_requires_foreground_fallback(
    process_name: &str,
    metadata: &synapse_a11y::ElementMetadataReadback,
) -> bool {
    if !synapse_a11y::is_chromium_family(process_name) || !metadata.enabled {
        return false;
    }
    if !metadata.patterns.contains(&UiaPattern::Value) {
        return false;
    }
    metadata.keyboard_focusable
        && (act_type_editable_role(&metadata.role) || metadata.patterns.contains(&UiaPattern::Text))
}

fn act_type_editable_role(role: &str) -> bool {
    let role = role.to_ascii_lowercase();
    role.contains("edit") || role.contains("document") || role.contains("text")
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
        .is_none_or(|regex| regex.is_match(&after.foreground_process));
    let title_matches = policy
        .expected_title_regex
        .as_ref()
        .is_none_or(|regex| regex.is_match(&after.foreground_title));

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
        return (Some(value.clone()), Some(ACT_TYPE_TEXT_SOURCE_UIA_VALUE));
    }
    if has_text_readback_pattern(&focused.patterns) {
        return (Some(String::new()), Some(ACT_TYPE_TEXT_SOURCE_UIA_EMPTY));
    }
    (None, None)
}

fn choose_act_type_text_readback(
    focused: Option<&ActTypeFocusedTextCandidate>,
    uia_value: Option<String>,
    uia_readback_source: Option<&'static str>,
    cdp_readback: &CdpActiveTextReadback,
    ocr_readback: &OcrTextReadback,
) -> (Option<String>, Option<String>) {
    if should_prefer_cdp_active_text(
        focused,
        uia_value.as_deref(),
        uia_readback_source,
        cdp_readback,
    ) {
        if let Some(value) = cdp_readback.value.clone() {
            return (
                Some(value),
                Some(ACT_TYPE_TEXT_SOURCE_CDP_ACTIVE.to_owned()),
            );
        }
    }

    let skip_uia = is_browser_shell_uia_readback(focused, uia_value.as_deref());
    if !skip_uia {
        if let Some(value) = uia_value {
            return (Some(value), uia_readback_source.map(str::to_owned));
        }
    }

    if let Some(value) = cdp_readback.value.clone() {
        return (
            Some(value),
            Some(ACT_TYPE_TEXT_SOURCE_CDP_ACTIVE.to_owned()),
        );
    }
    if !skip_uia {
        if let Some(value) = ocr_readback.value.clone() {
            return (
                Some(value),
                Some(ACT_TYPE_TEXT_SOURCE_OCR_FOCUSED_RECT.to_owned()),
            );
        }
    }
    (None, None)
}

fn should_prefer_cdp_active_text(
    focused: Option<&ActTypeFocusedTextCandidate>,
    uia_value: Option<&str>,
    uia_readback_source: Option<&'static str>,
    cdp_readback: &CdpActiveTextReadback,
) -> bool {
    if cdp_readback.value.is_none()
        || cdp_readback.has_active_element != Some(true)
        || cdp_readback.is_editable != Some(true)
    {
        return false;
    }
    let tag = cdp_readback.tag_name.as_deref().unwrap_or("").trim();
    if tag.is_empty() || tag.eq_ignore_ascii_case("BODY") || tag.eq_ignore_ascii_case("HTML") {
        return false;
    }
    if uia_readback_source == Some(ACT_TYPE_TEXT_SOURCE_UIA_EMPTY) {
        return true;
    }
    is_browser_shell_uia_readback(focused, uia_value)
}

fn is_browser_shell_uia_readback(
    focused: Option<&ActTypeFocusedTextCandidate>,
    uia_value: Option<&str>,
) -> bool {
    let Some(focused) = focused else {
        return false;
    };
    if !is_shell_text_focus_role(&focused.role) {
        return false;
    }
    match uia_value.map(str::trim) {
        Some(value) if value.is_empty() => true,
        Some(value) => looks_like_browser_page_value(value),
        None => false,
    }
}

fn is_shell_text_focus_role(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "window" | "document" | "pane" | "region" | "rootwebarea" | "webarea" | "web view"
    )
}

fn looks_like_browser_page_value(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("data:")
        || value.starts_with("about:")
        || value.starts_with("chrome://")
        || value.starts_with("edge://")
        || value.starts_with("file://")
}

async fn cdp_active_text_readback(input: &ObservationInput) -> CdpActiveTextReadback {
    let Some(cdp) = input.cdp.as_ref() else {
        return CdpActiveTextReadback {
            value: None,
            target_id: None,
            has_active_element: None,
            is_editable: None,
            tag_name: None,
            id_sha256: None,
            name_sha256: None,
            value_len: None,
            value_sha256: None,
            error_code: None,
            error_detail_sha256: None,
            attempt: "cdp_active_element_value:unavailable:no_cdp_diagnostics".to_owned(),
        };
    };
    let Some(endpoint) = cdp.endpoint.as_deref() else {
        return CdpActiveTextReadback {
            value: None,
            target_id: cdp.selected_target_id.clone(),
            has_active_element: None,
            is_editable: None,
            tag_name: None,
            id_sha256: None,
            name_sha256: None,
            value_len: None,
            value_sha256: None,
            error_code: cdp.reason_code.clone(),
            error_detail_sha256: cdp.detail.as_deref().and_then(non_empty_sha256),
            attempt: format!(
                "cdp_active_element_value:unavailable:status={}",
                cdp.status.as_str()
            ),
        };
    };
    let Some(target_id) = cdp.selected_target_id.as_deref() else {
        return CdpActiveTextReadback {
            value: None,
            target_id: None,
            has_active_element: None,
            is_editable: None,
            tag_name: None,
            id_sha256: None,
            name_sha256: None,
            value_len: None,
            value_sha256: None,
            error_code: Some(error_codes::A11Y_CDP_ATTACH_FAILED.to_owned()),
            error_detail_sha256: None,
            attempt: "cdp_active_element_value:unavailable:no_selected_target".to_owned(),
        };
    };

    match synapse_a11y::cdp_active_element_state(endpoint, target_id).await {
        Ok(state) => {
            let value =
                (state.has_active_element && state.is_editable).then(|| state.value.clone());
            CdpActiveTextReadback {
                value,
                target_id: Some(state.target_id),
                has_active_element: Some(state.has_active_element),
                is_editable: Some(state.is_editable),
                tag_name: (!state.tag_name.trim().is_empty()).then_some(state.tag_name),
                id_sha256: non_empty_sha256(&state.id),
                name_sha256: non_empty_sha256(&state.name),
                value_len: Some(state.value.chars().count()),
                value_sha256: Some(text_sha256(&state.value)),
                error_code: None,
                error_detail_sha256: None,
                attempt: if state.has_active_element && state.is_editable {
                    "cdp_active_element_value:available".to_owned()
                } else if state.has_active_element {
                    "cdp_active_element_value:unavailable:active_element_not_editable".to_owned()
                } else {
                    "cdp_active_element_value:unavailable:no_active_element".to_owned()
                },
            }
        }
        Err(error) => CdpActiveTextReadback {
            value: None,
            target_id: Some(target_id.to_owned()),
            has_active_element: None,
            is_editable: None,
            tag_name: None,
            id_sha256: None,
            name_sha256: None,
            value_len: None,
            value_sha256: None,
            error_code: Some(error.code().to_owned()),
            error_detail_sha256: non_empty_sha256(&error.to_string()),
            attempt: format!("cdp_active_element_value:error:{}", error.code()),
        },
    }
}

fn ocr_focused_rect_text_readback(
    focused: Option<&ActTypeFocusedTextCandidate>,
    elements: &[AccessibleNode],
) -> OcrTextReadback {
    let Some(focused) = focused else {
        return OcrTextReadback {
            value: None,
            word_count: 0,
            value_len: None,
            value_sha256: None,
            attempt: "ocr_focused_rect_text:unavailable:no_focused_element".to_owned(),
        };
    };
    if focused.bbox.w <= 0 || focused.bbox.h <= 0 {
        return OcrTextReadback {
            value: None,
            word_count: 0,
            value_len: None,
            value_sha256: None,
            attempt: "ocr_focused_rect_text:unavailable:empty_focused_bbox".to_owned(),
        };
    }

    let mut words = elements
        .iter()
        .filter(|node| {
            node.automation_id
                .as_deref()
                .is_some_and(|automation_id| automation_id.starts_with("ocr:word:"))
                && rects_intersect(node.bbox, focused.bbox)
                && !node.name.trim().is_empty()
        })
        .collect::<Vec<_>>();
    words.sort_by_key(|node| (node.bbox.y, node.bbox.x));
    let text = words
        .iter()
        .map(|node| node.name.trim())
        .collect::<Vec<_>>()
        .join(" ");
    let word_count = words.len();
    if word_count == 0 {
        return OcrTextReadback {
            value: None,
            word_count,
            value_len: None,
            value_sha256: None,
            attempt: "ocr_focused_rect_text:unavailable:no_ocr_words_in_focused_bbox".to_owned(),
        };
    }
    OcrTextReadback {
        value: Some(text.clone()),
        word_count,
        value_len: Some(text.chars().count()),
        value_sha256: Some(text_sha256(&text)),
        attempt: "ocr_focused_rect_text:available".to_owned(),
    }
}

fn rect_contains_point(rect: Rect, point: Point) -> bool {
    if rect.w <= 0 || rect.h <= 0 {
        return false;
    }
    point.x >= rect.x
        && point.x < rect.x.saturating_add(rect.w)
        && point.y >= rect.y
        && point.y < rect.y.saturating_add(rect.h)
}

fn rects_intersect(a: Rect, b: Rect) -> bool {
    if a.w <= 0 || a.h <= 0 || b.w <= 0 || b.h <= 0 {
        return false;
    }
    let a_right = a.x.saturating_add(a.w);
    let a_bottom = a.y.saturating_add(a.h);
    let b_right = b.x.saturating_add(b.w);
    let b_bottom = b.y.saturating_add(b.h);
    a.x < b_right && a_right > b.x && a.y < b_bottom && a_bottom > b.y
}

fn act_type_foreground_fallback_focus_matches_target(
    target: &ActTypeForegroundFallbackTarget,
    after_click: &ActTypeTextSignature,
) -> Result<(), ErrorData> {
    if after_click.foreground_hwnd != target.root_hwnd {
        return Err(act_type_foreground_fallback_focus_error(
            target,
            after_click,
            "foreground_hwnd_changed_after_target_click",
        ));
    }
    let role = after_click.focused_role.as_deref().unwrap_or_default();
    if !act_type_editable_role(role) {
        return Err(act_type_foreground_fallback_focus_error(
            target,
            after_click,
            "focused_role_is_not_text_editable",
        ));
    }
    if after_click
        .focused_element_id
        .as_deref()
        .is_some_and(|focused_id| focused_id == target.element_id)
    {
        return Ok(());
    }
    if after_click
        .focused_bbox
        .is_some_and(|bbox| rects_intersect(bbox, target.bbox))
    {
        return Ok(());
    }
    Err(act_type_foreground_fallback_focus_error(
        target,
        after_click,
        "focused_element_did_not_match_target_or_bbox",
    ))
}

fn act_type_target_center_point(
    target: &ActTypeForegroundFallbackTarget,
) -> Result<Point, ErrorData> {
    if target.bbox.w <= 0 || target.bbox.h <= 0 {
        return Err(act_type_foreground_fallback_target_invalid_error(
            target,
            "target bbox is empty or inverted",
        ));
    }
    let x = i64::from(target.bbox.x) + i64::from(target.bbox.w) / 2;
    let y = i64::from(target.bbox.y) + i64::from(target.bbox.h) / 2;
    let x = i32::try_from(x).map_err(|err| {
        act_type_foreground_fallback_target_invalid_error(
            target,
            format!("target bbox center x overflowed i32: {err}"),
        )
    })?;
    let y = i32::try_from(y).map_err(|err| {
        act_type_foreground_fallback_target_invalid_error(
            target,
            format!("target bbox center y overflowed i32: {err}"),
        )
    })?;
    Ok(Point { x, y })
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

fn act_type_text_terminal_failure(
    before: &ActTypeTextReadback,
    after: &ActTypeTextReadback,
) -> bool {
    act_type_foreground_identity_changed(&before.signature, &after.signature)
        || act_type_text_target_changed(&before.signature, &after.signature)
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

fn verify_act_type_text_response(
    mut response: ActTypeResponse,
    before: ActTypeTextReadback,
    after: ActTypeTextReadback,
    before_hash: String,
    after_hash: String,
    verify_timeout_ms: u32,
    emitted: &str,
) -> Result<ActTypeResponse, ErrorData> {
    let source_of_truth = act_type_text_source_of_truth(&before.signature, &after.signature);
    if act_type_foreground_identity_changed(&before.signature, &after.signature) {
        return Err(act_type_text_foreground_lost_error(
            verify_timeout_ms,
            &before_hash,
            &after_hash,
            &before.signature,
            &after.signature,
        ));
    }
    if act_type_text_target_changed(&before.signature, &after.signature) {
        return Err(postcondition_failed_error(
            "act_type",
            &source_of_truth,
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
            &source_of_truth,
            verify_timeout_ms,
            before_hash,
            after_hash,
            json!({
                "before": before.signature,
                "after": after.signature,
            }),
        ));
    }
    let Some(after_value) = after.value.as_deref() else {
        return Err(act_type_verify_surface_unavailable_error(
            "after-read had no UIA, CDP, or OCR text Source-of-Truth surface for act_type verify_delta",
            after_hash,
            after.signature,
        ));
    };
    if !normalized_text_contains(after_value, emitted) {
        return Err(postcondition_failed_error(
            "act_type",
            &source_of_truth,
            "text Source-of-Truth changed but did not contain the emitted text",
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
        &source_of_truth,
        before_hash,
        after_hash,
        "observed selected text Source-of-Truth changed and containing emitted text after delivery",
    );
    response.target_readback_required = false;
    response.target_text_integrity = format!(
        "{}:{}",
        ACT_TYPE_TEXT_INTEGRITY_PREFIX,
        after
            .signature
            .readback_source
            .as_deref()
            .unwrap_or("unknown")
    );
    Ok(response)
}

fn act_type_text_source_of_truth(
    before: &ActTypeTextSignature,
    after: &ActTypeTextSignature,
) -> String {
    after
        .readback_source
        .as_ref()
        .or(before.readback_source.as_ref())
        .map(|source| format!("{ACT_TYPE_FOREGROUND_TEXT_SOURCE_OF_TRUTH}:{source}"))
        .unwrap_or_else(|| ACT_TYPE_FOREGROUND_TEXT_SOURCE_OF_TRUTH.to_owned())
}

fn act_type_text_target_changed(
    before: &ActTypeTextSignature,
    after: &ActTypeTextSignature,
) -> bool {
    if before.focused_element_id != after.focused_element_id {
        return true;
    }
    if before.readback_source.as_deref() == Some(ACT_TYPE_TEXT_SOURCE_CDP_ACTIVE)
        || after.readback_source.as_deref() == Some(ACT_TYPE_TEXT_SOURCE_CDP_ACTIVE)
    {
        return before.cdp_selected_target_id != after.cdp_selected_target_id
            || before.cdp_active_tag_name != after.cdp_active_tag_name
            || before.cdp_active_id_sha256 != after.cdp_active_id_sha256
            || before.cdp_active_name_sha256 != after.cdp_active_name_sha256;
    }
    if before.readback_source.as_deref() == Some(ACT_TYPE_TEXT_SOURCE_OCR_FOCUSED_RECT)
        || after.readback_source.as_deref() == Some(ACT_TYPE_TEXT_SOURCE_OCR_FOCUSED_RECT)
    {
        return before.focused_bbox != after.focused_bbox;
    }
    false
}

/// Structured failure for the `act_set_field_text` foreground tier: precise
/// reason code, full target evidence, and the action error detail.
fn set_field_text_foreground_error(
    target: &ActTypeForegroundFallbackTarget,
    code: &'static str,
    reason: &'static str,
    detail: impl Into<String>,
) -> ErrorData {
    let detail = detail.into();
    tracing::error!(
        code,
        tool = "act_set_field_text",
        element_id = %target.element_id,
        root_hwnd = target.root_hwnd,
        reason,
        detail = %detail,
        "act_set_field_text foreground tier failed"
    );
    ErrorData::new(
        ErrorCode(-32099),
        detail.clone(),
        Some(json!({
            "code": code,
            "tool": "act_set_field_text",
            "reason": reason,
            "detail": detail,
            "target": target,
            "backend_tier_used": crate::m2::TIER_FOREGROUND_KEYS,
            "required_foreground": true,
        })),
    )
}

/// Password targets never expose value content; verification compares the
/// UTF-16 password length Source of Truth instead, mirroring `act_set_value`.
fn set_field_text_password_response(
    params: &crate::m2::ActSetFieldTextParams,
    started: Instant,
    method: &str,
    before: &synapse_a11y::ElementValueReadback,
    after: &synapse_a11y::ElementValueReadback,
) -> Result<crate::m2::ActSetFieldTextResponse, ErrorData> {
    let expected_len = params.text.encode_utf16().count();
    let before_len = before.password_len.unwrap_or(0);
    let after_len = after.password_len.unwrap_or(0);
    let signature = |len: usize| format!("password_len:{len}");
    let requested_len = u32::try_from(params.text.chars().count()).unwrap_or(u32::MAX);
    if after.password_len != Some(expected_len) {
        tracing::error!(
            code = error_codes::ACTION_POSTCONDITION_FAILED,
            tool = "act_set_field_text",
            element_id = %params.element_id,
            method,
            before_len,
            after_len,
            expected_len,
            "act_set_field_text password-length readback did not equal requested length"
        );
        return Err(ErrorData::new(
            ErrorCode(-32099),
            "act_set_field_text Source-of-Truth postcondition failed: password target length readback does not equal requested text length",
            Some(json!({
                "code": error_codes::ACTION_POSTCONDITION_FAILED,
                "tool": "act_set_field_text",
                "method": method,
                "source_of_truth": crate::m2::SOURCE_UIA_PASSWORD_LENGTH,
                "before_len": before_len,
                "after_len": after_len,
                "expected_len": expected_len,
                "is_password": true,
            })),
        ));
    }
    let changed = before.password_len != after.password_len;
    Ok(crate::m2::ActSetFieldTextResponse {
        ok: true,
        method: method.to_owned(),
        backend_tier_used: crate::m2::TIER_FOREGROUND_KEYS.to_owned(),
        required_foreground: true,
        source_of_truth: crate::m2::SOURCE_UIA_PASSWORD_LENGTH.to_owned(),
        requested_len,
        before_len: u32::try_from(before_len).unwrap_or(u32::MAX),
        after_len: u32::try_from(after_len).unwrap_or(u32::MAX),
        requested_sha256: signature(expected_len),
        before_sha256: signature(before_len),
        after_sha256: signature(after_len),
        changed,
        postcondition: ActPostcondition {
            status: "verified_state".to_owned(),
            observed_delta: Some(changed),
            source_of_truth: Some(crate::m2::SOURCE_UIA_PASSWORD_LENGTH.to_owned()),
            before_signature: Some(signature(before_len)),
            after_signature: Some(signature(after_len)),
            detail: Some(
                "act_set_field_text password target length equals requested length; value content intentionally not read or compared"
                    .to_owned(),
            ),
        },
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

fn act_type_foreground_fallback_recording_error(
    target: &ActTypeForegroundFallbackTarget,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        "act_type into_element Chromium foreground fallback requires the live foreground input tier and cannot run against the recording backend",
        Some(json!({
            "code": error_codes::ACTION_BACKEND_UNAVAILABLE,
            "tool": "act_type",
            "reason": "chromium_foreground_fallback_recording_backend_unsupported",
            "target": target,
            "required_foreground": true,
            "target_readback_required": true,
        })),
    )
}

fn act_type_foreground_fallback_target_not_foreground_error(
    expected: &ForegroundProof,
    target: &ActTypeForegroundFallbackTarget,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "act_type into_element Chromium foreground fallback requires target hwnd 0x{:x} to be the current foreground hwnd, but preflight foreground was 0x{:x}",
            target.root_hwnd, expected.hwnd
        ),
        Some(json!({
            "code": error_codes::ACTION_FOREGROUND_LOST,
            "tool": "act_type",
            "reason": "into_element_foreground_fallback_target_not_foreground",
            "foreground_expected": expected,
            "target": target,
            "required_foreground": true,
            "target_readback_required": true,
        })),
    )
}

fn act_type_foreground_fallback_target_invalid_error(
    target: &ActTypeForegroundFallbackTarget,
    detail: impl Into<String>,
) -> ErrorData {
    let detail = detail.into();
    ErrorData::new(
        ErrorCode(-32099),
        format!("act_type into_element Chromium foreground fallback target invalid: {detail}"),
        Some(json!({
            "code": error_codes::ACTION_TARGET_INVALID,
            "tool": "act_type",
            "reason": "chromium_foreground_fallback_target_invalid",
            "detail": detail,
            "target": target,
            "required_foreground": true,
            "target_readback_required": true,
        })),
    )
}

fn act_type_foreground_fallback_click_error(
    target: &ActTypeForegroundFallbackTarget,
    point: Point,
    error: &ActionError,
) -> ErrorData {
    let mapped = crate::m2::action_error_to_mcp(error);
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "act_type into_element Chromium foreground fallback click failed at ({},{}): {}",
            point.x, point.y, error
        ),
        Some(json!({
            "code": mapped
                .data
                .as_ref()
                .and_then(|data| data.get("code"))
                .and_then(Value::as_str)
                .unwrap_or(error_codes::ACTION_BACKEND_UNAVAILABLE),
            "tool": "act_type",
            "reason": "chromium_foreground_fallback_click_failed",
            "point": point,
            "target": target,
            "cause": mapped.data,
            "required_foreground": true,
            "target_readback_required": true,
        })),
    )
}

fn act_type_foreground_fallback_focus_error(
    target: &ActTypeForegroundFallbackTarget,
    after_click: &ActTypeTextSignature,
    reason: &'static str,
) -> ErrorData {
    let after_hash =
        verify_hash_json(after_click).unwrap_or_else(|_| "hash_unavailable".to_owned());
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "act_type into_element Chromium foreground fallback click did not focus the requested editable target: {reason}"
        ),
        Some(json!({
            "code": error_codes::ACTION_TARGET_INVALID,
            "tool": "act_type",
            "reason": reason,
            "source_of_truth": ACT_TYPE_FOREGROUND_TEXT_SOURCE_OF_TRUTH,
            "after_click_signature": after_hash,
            "after_click": after_click,
            "target": target,
            "required_foreground": true,
            "target_readback_required": true,
        })),
    )
}

fn act_type_verify_surface_unavailable_error(
    detail: impl Into<String>,
    signature_hash: String,
    readback: ActTypeTextSignature,
) -> ErrorData {
    let detail = detail.into();
    tracing::error!(
        code = error_codes::ACTION_VERIFY_SURFACE_UNAVAILABLE,
        tool = "act_type",
        source_of_truth = ACT_TYPE_FOREGROUND_TEXT_SOURCE_OF_TRUTH,
        signature = %signature_hash,
        readback_source = ?readback.readback_source,
        attempts = ?readback.text_readback_attempts,
        "act_type verify_delta text Source-of-Truth surface unavailable"
    );
    ErrorData::new(
        ErrorCode(-32099),
        format!("act_type verify_delta Source-of-Truth surface unavailable: {detail}"),
        Some(json!({
            "code": error_codes::ACTION_VERIFY_SURFACE_UNAVAILABLE,
            "tool": "act_type",
            "source_of_truth": ACT_TYPE_FOREGROUND_TEXT_SOURCE_OF_TRUTH,
            "detail": detail,
            "signature": signature_hash,
            "readback": readback,
        })),
    )
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
        source_of_truth = ACT_TYPE_FOREGROUND_TEXT_SOURCE_OF_TRUTH,
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
            "source_of_truth": ACT_TYPE_FOREGROUND_TEXT_SOURCE_OF_TRUTH,
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

const fn should_monitor_act_stroke_foreground(
    recording_active: bool,
    requires_input_lease: bool,
) -> bool {
    !recording_active && requires_input_lease
}

const fn should_acquire_act_stroke_input_lease(
    recording_active: bool,
    requires_input_lease: bool,
) -> bool {
    !recording_active && requires_input_lease
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

#[derive(Copy, Clone, Debug, Serialize)]
struct BackgroundTargetForegroundGuard {
    element_hwnd: i64,
    root_hwnd: i64,
}

impl BackgroundTargetForegroundGuard {
    fn contains(self, hwnd: i64) -> bool {
        hwnd == self.element_hwnd || hwnd == self.root_hwnd
    }
}

fn act_set_value_target_foreground_guard(
    element_id: &ElementId,
) -> Result<BackgroundTargetForegroundGuard, ErrorData> {
    let hwnd = element_id.parts().map_err(|error| {
        mcp_error(
            error_codes::ACTION_TARGET_INVALID,
            format!(
                "act_set_value element id {element_id} could not be parsed for foreground guard: {error}"
            ),
        )
    })?.hwnd;
    let root_hwnd = synapse_a11y::top_level_root_hwnd(hwnd).map_err(|error| {
        mcp_error(
            error_codes::ACTION_TARGET_INVALID,
            format!(
                "act_set_value element id {element_id} HWND 0x{hwnd:x} could not be normalized to a live top-level target root for foreground guard: {error}"
            ),
        )
    })?;
    Ok(BackgroundTargetForegroundGuard {
        element_hwnd: hwnd,
        root_hwnd,
    })
}

fn act_scroll_target_foreground_guard(
    params: &ActScrollParams,
) -> Result<BackgroundTargetForegroundGuard, ErrorData> {
    let Some(target) = params.target.as_ref() else {
        return Err(mcp_error(
            error_codes::ACTION_TARGET_INVALID,
            "act_scroll foreground guard requires target.element_id",
        ));
    };
    let hwnd = target
        .element_id
        .parts()
        .map_err(|error| {
            mcp_error(
                error_codes::ACTION_TARGET_INVALID,
                format!(
                    "act_scroll element id {} could not be parsed for foreground guard: {error}",
                    target.element_id
                ),
            )
        })?
        .hwnd;
    let root_hwnd = synapse_a11y::top_level_root_hwnd(hwnd).map_err(|error| {
        mcp_error(
            error_codes::ACTION_TARGET_INVALID,
            format!(
                "act_scroll element id {} HWND 0x{hwnd:x} could not be normalized to a live top-level target root for foreground guard: {error}",
                target.element_id
            ),
        )
    })?;
    Ok(BackgroundTargetForegroundGuard {
        element_hwnd: hwnd,
        root_hwnd,
    })
}

fn verify_background_target_not_activated(
    tool: &'static str,
    action_source_of_truth: &str,
    target: BackgroundTargetForegroundGuard,
    before: &ForegroundContext,
    after: &ForegroundContext,
) -> Result<(), ErrorData> {
    if before.hwnd == after.hwnd && before.pid == after.pid {
        return Ok(());
    }
    if !target.contains(before.hwnd) && target.contains(after.hwnd) {
        return Err(background_foreground_lost_error(
            tool,
            action_source_of_truth,
            target,
            before,
            after,
        ));
    }
    tracing::warn!(
        code = "BACKGROUND_FOREGROUND_CHANGED_NON_TARGET",
        tool,
        source_of_truth = BACKGROUND_FOREGROUND_SOURCE_OF_TRUTH,
        action_source_of_truth,
        target_element_hwnd = target.element_hwnd,
        target_root_hwnd = target.root_hwnd,
        before_hwnd = before.hwnd,
        after_hwnd = after.hwnd,
        before_pid = before.pid,
        after_pid = after.pid,
        before_process_name = %before.process_name,
        after_process_name = %after.process_name,
        "background action completed while foreground changed to a non-target window"
    );
    Ok(())
}

fn background_foreground_lost_error(
    tool: &'static str,
    action_source_of_truth: &str,
    target: BackgroundTargetForegroundGuard,
    before: &ForegroundContext,
    after: &ForegroundContext,
) -> ErrorData {
    let detail = format!(
        "{tool} returned a background result but foreground changed from hwnd 0x{:x} ({}) to hwnd 0x{:x} ({})",
        before.hwnd, before.process_name, after.hwnd, after.process_name
    );
    tracing::error!(
        code = error_codes::ACTION_FOREGROUND_LOST,
        reason = "background_action_changed_foreground",
        tool,
        source_of_truth = BACKGROUND_FOREGROUND_SOURCE_OF_TRUTH,
        action_source_of_truth,
        target_element_hwnd = target.element_hwnd,
        target_root_hwnd = target.root_hwnd,
        before_hwnd = before.hwnd,
        after_hwnd = after.hwnd,
        before_pid = before.pid,
        after_pid = after.pid,
        before_process_name = %before.process_name,
        after_process_name = %after.process_name,
        before_window_title = %before.window_title,
        after_window_title = %after.window_title,
        "background action changed foreground after reporting required_foreground=false"
    );
    ErrorData::new(
        ErrorCode(-32099),
        detail.clone(),
        Some(json!({
            "code": error_codes::ACTION_FOREGROUND_LOST,
            "reason": "background_action_changed_foreground",
            "tool": tool,
            "source_of_truth": BACKGROUND_FOREGROUND_SOURCE_OF_TRUTH,
            "action_source_of_truth": action_source_of_truth,
            "required_foreground": false,
            "target_element_hwnd": target.element_hwnd,
            "target_root_hwnd": target.root_hwnd,
            "detail": detail,
            "foreground_before": foreground_context_details(before),
            "foreground_after": foreground_context_details(after),
        })),
    )
}

fn act_set_value_foreground_read_error(
    stage: &'static str,
    action_source_of_truth: &str,
    error: &ErrorData,
) -> ErrorData {
    let detail = format!(
        "act_set_value could not read foreground {stage} background dispatch: {}",
        error.message
    );
    tracing::error!(
        code = error_codes::ACTION_FOREGROUND_CONTEXT_CAPTURE_FAILED,
        reason = "background_foreground_read_failed",
        tool = "act_set_value",
        source_of_truth = BACKGROUND_FOREGROUND_SOURCE_OF_TRUTH,
        action_source_of_truth,
        stage,
        read_error = %error.message,
        "act_set_value background foreground guard could not read OS foreground Source of Truth"
    );
    ErrorData::new(
        ErrorCode(-32099),
        detail.clone(),
        Some(json!({
            "code": error_codes::ACTION_FOREGROUND_CONTEXT_CAPTURE_FAILED,
            "reason": "background_foreground_read_failed",
            "tool": "act_set_value",
            "source_of_truth": BACKGROUND_FOREGROUND_SOURCE_OF_TRUTH,
            "action_source_of_truth": action_source_of_truth,
            "required_foreground": false,
            "stage": stage,
            "detail": detail,
            "read_error": {
                "message": error.message.to_string(),
                "data": error.data.clone(),
            },
        })),
    )
}

fn act_scroll_foreground_read_error(
    stage: &'static str,
    action_source_of_truth: &str,
    error: &ErrorData,
) -> ErrorData {
    let detail = format!(
        "act_scroll could not read foreground {stage} background dispatch: {}",
        error.message
    );
    tracing::error!(
        code = error_codes::ACTION_FOREGROUND_CONTEXT_CAPTURE_FAILED,
        tool = "act_scroll",
        stage,
        source_of_truth = BACKGROUND_FOREGROUND_SOURCE_OF_TRUTH,
        action_source_of_truth,
        error = %error.message,
        "background scroll foreground read failed"
    );
    ErrorData::new(
        ErrorCode(-32099),
        detail.clone(),
        Some(json!({
            "code": error_codes::ACTION_FOREGROUND_CONTEXT_CAPTURE_FAILED,
            "reason": "background_foreground_read_failed",
            "tool": "act_scroll",
            "stage": stage,
            "source_of_truth": BACKGROUND_FOREGROUND_SOURCE_OF_TRUTH,
            "action_source_of_truth": action_source_of_truth,
            "required_foreground": false,
            "detail": detail,
        })),
    )
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
    use synapse_core::{ElementId, Rect};

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
    fn hidden_desktop_foreground_refusal_carries_physical_route_context() {
        let hidden_desktop = crate::server::session_lifecycle::SessionHiddenDesktopReadback {
            session_id: "session-743".to_owned(),
            desktop_names: vec!["SynapseAgent_abc123".to_owned()],
            launch_pids: vec![4242],
            resource_count: 1,
        };

        let error = hidden_desktop_foreground_refusal("act_press", &hidden_desktop);
        let data = error.data.as_ref().expect("structured error data");
        println!(
            "readback=hidden_desktop_foreground_refusal before=session:{} desktop:{:?} after=data:{}",
            hidden_desktop.session_id, hidden_desktop.desktop_names, data
        );

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::FOREGROUND_ACTIVATION_REFUSED)
        );
        assert_eq!(
            data.get("reason").and_then(Value::as_str),
            Some("hidden_desktop_foreground_tier_refused")
        );
        assert_eq!(data.get("tool").and_then(Value::as_str), Some("act_press"));
        assert_eq!(
            data.get("foreground_tier_allowed").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            data.get("desktop_names")
                .and_then(Value::as_array)
                .and_then(|names| names.first())
                .and_then(Value::as_str),
            Some("SynapseAgent_abc123")
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
    fn act_stroke_foreground_monitor_only_runs_for_live_leased_strokes() {
        assert!(
            should_monitor_act_stroke_foreground(false, true),
            "live real-cursor strokes require foreground-loss monitoring"
        );
        assert!(
            should_acquire_act_stroke_input_lease(false, true),
            "live real-cursor strokes require the foreground input lease"
        );
        assert!(
            !should_monitor_act_stroke_foreground(false, false),
            "background CDP strokes must not be aborted by the global foreground monitor"
        );
        assert!(
            !should_acquire_act_stroke_input_lease(false, false),
            "background CDP strokes must not acquire the foreground input lease"
        );
        assert!(
            !should_monitor_act_stroke_foreground(true, true),
            "recording strokes do not touch live foreground input"
        );
        assert!(
            !should_acquire_act_stroke_input_lease(true, true),
            "recording strokes do not need the foreground input lease"
        );
        assert!(
            !should_monitor_act_stroke_foreground(true, false),
            "recording background strokes also skip live foreground monitoring"
        );
        assert!(
            !should_acquire_act_stroke_input_lease(true, false),
            "recording background strokes also skip foreground lease acquisition"
        );
    }

    #[test]
    fn act_set_value_background_guard_rejects_target_activation() {
        let before = foreground_context(100, 10, "chrome.exe", "before");
        let after = foreground_context(200, 20, "wpf-test.exe", "after");
        let target = BackgroundTargetForegroundGuard {
            element_hwnd: 150,
            root_hwnd: 200,
        };

        let error = verify_background_target_not_activated(
            "act_set_value",
            "uia_value_pattern.value",
            target,
            &before,
            &after,
        )
        .expect_err("background set_value must fail if it activates the target root");
        let data = error.data.as_ref().expect("structured error data");

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::ACTION_FOREGROUND_LOST)
        );
        assert_eq!(
            data.get("reason").and_then(Value::as_str),
            Some("background_action_changed_foreground")
        );
        assert_eq!(
            data.get("target_root_hwnd").and_then(Value::as_i64),
            Some(200)
        );
        assert_eq!(
            data.get("target_element_hwnd").and_then(Value::as_i64),
            Some(150)
        );
        assert_eq!(
            data.pointer("/foreground_before/hwnd")
                .and_then(Value::as_i64),
            Some(100)
        );
        assert_eq!(
            data.pointer("/foreground_after/hwnd")
                .and_then(Value::as_i64),
            Some(200)
        );
    }

    #[test]
    fn act_scroll_background_guard_rejects_target_activation() {
        let before = foreground_context(100, 10, "Code.exe", "before");
        let after = foreground_context(200, 20, "notepad.exe", "after");
        let target = BackgroundTargetForegroundGuard {
            element_hwnd: 150,
            root_hwnd: 200,
        };

        let error = verify_background_target_not_activated(
            "act_scroll",
            "uia_scroll_pattern.scroll_state",
            target,
            &before,
            &after,
        )
        .expect_err("background scroll must fail if a UIA provider activates the target");
        let data = error.data.as_ref().expect("structured error data");

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::ACTION_FOREGROUND_LOST)
        );
        assert_eq!(data.get("tool").and_then(Value::as_str), Some("act_scroll"));
        assert_eq!(
            data.get("action_source_of_truth").and_then(Value::as_str),
            Some("uia_scroll_pattern.scroll_state")
        );
    }

    #[test]
    fn act_set_value_background_guard_rejects_target_child_activation() {
        let before = foreground_context(100, 10, "chrome.exe", "before");
        let after = foreground_context(150, 20, "winforms-test.exe", "after child");
        let target = BackgroundTargetForegroundGuard {
            element_hwnd: 150,
            root_hwnd: 200,
        };

        let error = verify_background_target_not_activated(
            "act_set_value",
            "uia_value_pattern.value",
            target,
            &before,
            &after,
        )
        .expect_err("background set_value must fail if it activates the target child hwnd");
        let data = error.data.as_ref().expect("structured error data");

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::ACTION_FOREGROUND_LOST)
        );
        assert_eq!(
            data.get("target_element_hwnd").and_then(Value::as_i64),
            Some(150)
        );
        assert_eq!(
            data.get("target_root_hwnd").and_then(Value::as_i64),
            Some(200)
        );
        assert_eq!(
            data.pointer("/foreground_after/hwnd")
                .and_then(Value::as_i64),
            Some(150)
        );
    }

    #[test]
    fn act_set_value_background_guard_allows_non_target_foreground_change() {
        let before = foreground_context(100, 10, "chrome.exe", "before");
        let after = foreground_context(300, 30, "code.exe", "human moved");
        let target = BackgroundTargetForegroundGuard {
            element_hwnd: 150,
            root_hwnd: 200,
        };

        verify_background_target_not_activated(
            "act_set_value",
            "win32_window_text",
            target,
            &before,
            &after,
        )
        .expect("non-target foreground changes should not be treated as target activation");
    }

    #[test]
    fn act_set_value_background_guard_allows_already_target_foreground() {
        let before = foreground_context(150, 20, "winforms-test.exe", "already target");
        let after = foreground_context(200, 20, "winforms-test.exe", "root after");
        let target = BackgroundTargetForegroundGuard {
            element_hwnd: 150,
            root_hwnd: 200,
        };

        verify_background_target_not_activated(
            "act_set_value",
            "uia_value_pattern.value",
            target,
            &before,
            &after,
        )
        .expect("background guard should not fail when the target was already foreground");
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
    fn act_press_background_target_candidate_is_strict() {
        let mut params = act_press_params(false, false, None, None);
        params.backend = PressBackend::Auto;
        assert!(press_background_target_candidate(&params, false));

        params.backend = PressBackend::Software;
        assert!(press_background_target_candidate(&params, false));

        params.backend = PressBackend::Hardware;
        assert!(!press_background_target_candidate(&params, false));

        params.backend = PressBackend::Auto;
        assert!(!press_background_target_candidate(&params, true));

        params.verify_delta = true;
        params.allow_foreground_change = true;
        assert!(!press_background_target_candidate(&params, false));

        params.allow_foreground_change = false;
        params.expected_foreground_title_regex = Some("Chrome".to_owned());
        assert!(!press_background_target_candidate(&params, false));
    }

    #[test]
    fn hwnd_keyboard_ctrl_a_requires_full_selection_without_text_mutation() {
        let before = hwnd_keyboard_signature("alpha beta gamma", 16, 16);
        let after_inserted_a = hwnd_keyboard_signature("alpha beta gammaa", 17, 17);

        let error = verify_hwnd_keyboard_delta_signature(
            "act_press",
            "target_hwnd_text_or_selection",
            250,
            before.clone(),
            after_inserted_a,
            HwndKeyboardExpectedEffect::SelectAll,
            "observed target HWND text/selection change after PostMessage keyboard delivery",
        )
        .expect_err("Ctrl+A must not pass when it inserts a literal a");
        let data = error.data.as_ref().expect("structured error data");

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::ACTION_POSTCONDITION_FAILED)
        );
        assert_eq!(
            data.get("detail").and_then(Value::as_str),
            Some("Ctrl+A select-all changed target text instead of preserving it")
        );

        let selected = hwnd_keyboard_signature("alpha beta gamma", 0, 16);
        let postcondition = verify_hwnd_keyboard_delta_signature(
            "act_press",
            "target_hwnd_text_or_selection",
            250,
            before,
            selected,
            HwndKeyboardExpectedEffect::SelectAll,
            "observed target HWND text/selection change after PostMessage keyboard delivery",
        )
        .expect("Ctrl+A should pass only when readback shows full selection");
        assert_eq!(postcondition.status, "observed_delta");
    }

    #[test]
    fn hwnd_keyboard_printable_after_full_selection_requires_exact_replacement() {
        let before = hwnd_keyboard_signature("alpha beta gamma", 0, 16);
        let wrong_after = hwnd_keyboard_signature("alpha beta gammaz", 17, 17);

        let error = verify_hwnd_keyboard_delta_signature(
            "act_press",
            "target_hwnd_text_or_selection",
            250,
            before.clone(),
            wrong_after,
            HwndKeyboardExpectedEffect::PrintableText {
                text: "z".to_owned(),
            },
            "observed target HWND text/selection change after PostMessage keyboard delivery",
        )
        .expect_err("full-selection replacement must match the emitted character");
        let data = error.data.as_ref().expect("structured error data");

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::ACTION_POSTCONDITION_FAILED)
        );

        let replaced = hwnd_keyboard_signature("z", 1, 1);
        let postcondition = verify_hwnd_keyboard_delta_signature(
            "act_press",
            "target_hwnd_text_or_selection",
            250,
            before,
            replaced,
            HwndKeyboardExpectedEffect::PrintableText {
                text: "z".to_owned(),
            },
            "observed target HWND text/selection change after PostMessage keyboard delivery",
        )
        .expect("single printable key should pass when it replaces full selection exactly");
        assert_eq!(postcondition.status, "observed_delta");
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
    fn act_type_text_signature_capture_respects_verify_delta_opt_out() {
        let params = act_type_params(false, None);

        assert!(
            !act_type_should_capture_text_signature(&params),
            "verify_delta=false must not collect foreground text signatures or run postconditions"
        );
    }

    #[test]
    fn act_type_text_signature_capture_only_for_foreground_verify_delta() {
        let params = act_type_params(true, None);

        assert!(
            act_type_should_capture_text_signature(&params),
            "foreground act_type with verify_delta=true must keep fail-closed SoT verification"
        );
    }

    #[test]
    fn act_type_text_signature_capture_skips_into_element_route() {
        let mut params = act_type_params(true, None);
        params.into_element = Some(
            ElementId::parse("0x1000:0000002a00000001")
                .expect("synthetic element id must be valid"),
        );

        assert!(
            !act_type_should_capture_text_signature(&params),
            "into_element routes own background readback and must not use foreground text signatures"
        );
    }

    #[test]
    fn act_type_chromium_fallback_requires_foreground_route_for_refused_target() {
        let mut params = act_type_params(true, None);
        params.into_element = Some(
            ElementId::parse("0x1000:0000002a00000001")
                .expect("synthetic element id must be valid"),
        );
        let target = act_type_foreground_fallback_target(
            0x1000,
            "edit",
            Rect {
                x: 100,
                y: 200,
                w: 300,
                h: 40,
            },
        );

        println!(
            "readback=act_type_foreground_fallback_route before=into_element after=requires_foreground:{}",
            act_type_requires_foreground_route(&params, Some(&target))
        );

        assert!(act_type_requires_foreground_route(&params, Some(&target)));
        assert!(
            !act_type_requires_foreground_route(&params, None),
            "ordinary into_element routes stay background-only unless the Chromium fallback target is detected"
        );
        params.into_element = None;
        assert!(act_type_requires_foreground_route(&params, None));
    }

    #[test]
    fn chromium_foreground_fallback_eligibility_matches_unsafe_value_pattern_shape() {
        let metadata = act_type_element_metadata("edit", true, true, vec![UiaPattern::Value]);

        assert!(
            chromium_editable_value_pattern_requires_foreground_fallback("chrome.exe", &metadata)
        );
        assert!(
            !chromium_editable_value_pattern_requires_foreground_fallback("notepad.exe", &metadata)
        );
        assert!(
            !chromium_editable_value_pattern_requires_foreground_fallback(
                "chrome.exe",
                &act_type_element_metadata("button", true, true, vec![UiaPattern::Value])
            )
        );
        assert!(
            !chromium_editable_value_pattern_requires_foreground_fallback(
                "chrome.exe",
                &act_type_element_metadata("edit", true, false, vec![UiaPattern::Value])
            )
        );
        assert!(
            !chromium_editable_value_pattern_requires_foreground_fallback(
                "chrome.exe",
                &act_type_element_metadata("edit", true, true, vec![UiaPattern::Text])
            )
        );
    }

    #[test]
    fn act_type_foreground_fallback_focus_accepts_matching_edit_bbox() {
        let target = act_type_foreground_fallback_target(
            0x1000,
            "edit",
            Rect {
                x: 100,
                y: 200,
                w: 300,
                h: 40,
            },
        );
        let readback = act_type_signature_for_fallback(
            0x1000,
            Some("edit"),
            Some(Rect {
                x: 120,
                y: 205,
                w: 120,
                h: 30,
            }),
        );

        act_type_foreground_fallback_focus_matches_target(&target, &readback)
            .expect("intersecting focused edit bbox should identify the clicked target");
    }

    #[test]
    fn act_type_foreground_fallback_focus_rejects_wrong_target() {
        let target = act_type_foreground_fallback_target(
            0x1000,
            "edit",
            Rect {
                x: 100,
                y: 200,
                w: 300,
                h: 40,
            },
        );
        let wrong_role = act_type_signature_for_fallback(
            0x1000,
            Some("button"),
            Some(Rect {
                x: 120,
                y: 205,
                w: 120,
                h: 30,
            }),
        );
        let wrong_bbox = act_type_signature_for_fallback(
            0x1000,
            Some("edit"),
            Some(Rect {
                x: 800,
                y: 900,
                w: 120,
                h: 30,
            }),
        );

        let role_error = act_type_foreground_fallback_focus_matches_target(&target, &wrong_role)
            .expect_err("non-edit focused role must fail closed before typing");
        let bbox_error = act_type_foreground_fallback_focus_matches_target(&target, &wrong_bbox)
            .expect_err("focused edit outside target bbox must fail closed before typing");

        assert_eq!(
            role_error
                .data
                .as_ref()
                .and_then(|data| data.get("reason"))
                .and_then(Value::as_str),
            Some("focused_role_is_not_text_editable")
        );
        assert_eq!(
            bbox_error
                .data
                .as_ref()
                .and_then(|data| data.get("reason"))
                .and_then(Value::as_str),
            Some("focused_element_did_not_match_target_or_bbox")
        );
    }

    #[test]
    fn act_type_foreground_fallback_rejects_empty_target_bbox() {
        let target = act_type_foreground_fallback_target(
            0x1000,
            "edit",
            Rect {
                x: 100,
                y: 200,
                w: 0,
                h: 40,
            },
        );

        let error = act_type_target_center_point(&target)
            .expect_err("empty target bbox must fail closed before foreground input");

        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("code"))
                .and_then(Value::as_str),
            Some(error_codes::ACTION_TARGET_INVALID)
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
    fn act_type_verify_delta_accepts_cdp_active_element_text_surface() {
        let before = act_type_text_readback_with_source(
            None,
            Some("document"),
            Some("draft"),
            Some(ACT_TYPE_TEXT_SOURCE_CDP_ACTIVE),
        );
        let after = act_type_text_readback_with_source(
            None,
            Some("document"),
            Some("draft issue786-cdp-text"),
            Some(ACT_TYPE_TEXT_SOURCE_CDP_ACTIVE),
        );
        let response = act_type_response_for_verify_delta();

        let verified = verify_act_type_text_response(
            response,
            before,
            after,
            "before-cdp-hash".to_owned(),
            "after-cdp-hash".to_owned(),
            250,
            "issue786-cdp-text",
        )
        .expect("CDP active-element text readback should satisfy act_type verify_delta");

        assert_eq!(verified.postcondition.status, "observed_delta");
        assert_eq!(verified.postcondition.observed_delta, Some(true));
        assert_eq!(
            verified.postcondition.source_of_truth.as_deref(),
            Some("foreground_text_readback:cdp_active_element_value")
        );
        assert_eq!(
            verified.target_text_integrity,
            "verify_delta_text_readback:cdp_active_element_value"
        );
        assert!(!verified.target_readback_required);
    }

    #[test]
    fn act_type_verify_delta_keeps_no_delta_distinct_from_no_surface() {
        let before = act_type_text_readback_with_source(
            None,
            Some("document"),
            Some("unchanged"),
            Some(ACT_TYPE_TEXT_SOURCE_CDP_ACTIVE),
        );
        let after = before.clone();
        let response = act_type_response_for_verify_delta();

        let error = verify_act_type_text_response(
            response,
            before,
            after,
            "before-same-hash".to_owned(),
            "after-same-hash".to_owned(),
            250,
            "issue786",
        )
        .expect_err("same CDP active-element text must be verified no-delta, not no-surface");
        let data = error.data.as_ref().expect("structured error data");

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::ACTION_NO_OBSERVED_DELTA)
        );
    }

    #[test]
    fn act_type_verify_polling_keeps_target_switch_terminal() {
        let before = act_type_text_readback_with_source(
            None,
            Some("title-field"),
            Some("draft"),
            Some(ACT_TYPE_TEXT_SOURCE_UIA_VALUE),
        );
        let after_same_target = act_type_text_readback_with_source(
            None,
            Some("title-field"),
            Some("draft issue880"),
            Some(ACT_TYPE_TEXT_SOURCE_UIA_VALUE),
        );
        let after_switched_target = act_type_text_readback_with_source(
            None,
            Some("description-field"),
            Some("draft issue880"),
            Some(ACT_TYPE_TEXT_SOURCE_UIA_VALUE),
        );

        println!(
            "readback=act_type_verify_polling same_target_terminal={} switched_target_terminal={}",
            act_type_text_terminal_failure(&before, &after_same_target),
            act_type_text_terminal_failure(&before, &after_switched_target)
        );
        assert!(!act_type_text_terminal_failure(&before, &after_same_target));
        assert!(act_type_text_terminal_failure(
            &before,
            &after_switched_target
        ));
    }

    #[test]
    fn act_type_verify_delta_reports_distinct_surface_unavailable_code() {
        let no_surface = act_type_text_readback_with_source(None, Some("document"), None, None);

        let error = act_type_verify_surface_unavailable_error(
            "synthetic no-surface regression",
            "no-surface-hash".to_owned(),
            no_surface.signature,
        );
        let data = error.data.as_ref().expect("structured error data");

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::ACTION_VERIFY_SURFACE_UNAVAILABLE)
        );
        assert_ne!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::ACTION_NO_OBSERVED_DELTA)
        );
    }

    #[test]
    fn act_type_text_readback_prefers_editable_cdp_when_uia_is_browser_shell_url() {
        let focused = act_type_focused_candidate("document", Some("data:text/html,issue786"));
        let cdp = cdp_active_text_readback_for_test(Some("alpha issue786"), true, "DIV");
        let ocr = ocr_text_readback_for_test(Some("visible page words"));

        let (value, source) = choose_act_type_text_readback(
            Some(&focused),
            Some("data:text/html,issue786".to_owned()),
            Some(ACT_TYPE_TEXT_SOURCE_UIA_VALUE),
            &cdp,
            &ocr,
        );

        assert_eq!(value.as_deref(), Some("alpha issue786"));
        assert_eq!(source.as_deref(), Some(ACT_TYPE_TEXT_SOURCE_CDP_ACTIVE));
    }

    #[test]
    fn act_type_text_readback_rejects_browser_shell_url_without_editable_cdp() {
        let focused = act_type_focused_candidate("document", Some("data:text/html,issue786"));
        let cdp = cdp_active_text_readback_for_test(None, false, "BODY");
        let ocr = ocr_text_readback_for_test(Some("visible page words"));

        let (value, source) = choose_act_type_text_readback(
            Some(&focused),
            Some("data:text/html,issue786".to_owned()),
            Some(ACT_TYPE_TEXT_SOURCE_UIA_VALUE),
            &cdp,
            &ocr,
        );

        assert_eq!(value, None);
        assert_eq!(source, None);
    }

    #[test]
    fn act_type_text_readback_prefers_editable_cdp_over_empty_uia_text_placeholder() {
        let focused = act_type_focused_candidate("group", None);
        let cdp = cdp_active_text_readback_for_test(Some("alpha issue786"), true, "DIV");
        let ocr = ocr_text_readback_for_test(None);

        let (value, source) = choose_act_type_text_readback(
            Some(&focused),
            Some(String::new()),
            Some(ACT_TYPE_TEXT_SOURCE_UIA_EMPTY),
            &cdp,
            &ocr,
        );

        assert_eq!(value.as_deref(), Some("alpha issue786"));
        assert_eq!(source.as_deref(), Some(ACT_TYPE_TEXT_SOURCE_CDP_ACTIVE));
    }

    #[test]
    fn act_type_text_readback_keeps_real_uia_edit_control_authoritative() {
        let focused = act_type_focused_candidate("Edit", Some("https://example.test/search"));
        let cdp = cdp_active_text_readback_for_test(Some("dom editor text"), true, "DIV");
        let ocr = ocr_text_readback_for_test(Some("visible words"));

        let (value, source) = choose_act_type_text_readback(
            Some(&focused),
            Some("https://example.test/search".to_owned()),
            Some(ACT_TYPE_TEXT_SOURCE_UIA_VALUE),
            &cdp,
            &ocr,
        );

        assert_eq!(value.as_deref(), Some("https://example.test/search"));
        assert_eq!(source.as_deref(), Some(ACT_TYPE_TEXT_SOURCE_UIA_VALUE));
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
            window_hwnd: None,
            cdp_target_id: None,
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
        act_type_text_readback_with_source(
            browser_url,
            focused_element_id,
            focused_value,
            focused_value.map(|_| "focused.value"),
        )
    }

    fn act_type_text_readback_with_source(
        browser_url: Option<&str>,
        focused_element_id: Option<&str>,
        focused_value: Option<&str>,
        readback_source: Option<&str>,
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
                readback_source: readback_source.map(str::to_owned),
                has_text_readback: focused_value.is_some(),
                text_readback_attempts: vec![
                    readback_source
                        .map(|source| format!("{source}:available"))
                        .unwrap_or_else(|| "all_text_surfaces:unavailable".to_owned()),
                ],
                cdp_status: Some("ok".to_owned()),
                cdp_endpoint_present: true,
                cdp_selected_target_id: Some("TARGET810".to_owned()),
                cdp_active_has_element: Some(true),
                cdp_active_is_editable: Some(true),
                cdp_active_tag_name: Some("DIV".to_owned()),
                cdp_active_id_sha256: non_empty_sha256("issue786-editor"),
                cdp_active_name_sha256: None,
                cdp_active_value_len: focused_value.as_ref().map(|value| value.chars().count()),
                cdp_active_value_sha256: focused_value.as_deref().map(text_sha256),
                cdp_active_error_code: None,
                cdp_active_error_detail_sha256: None,
                ocr_word_count: 0,
                ocr_text_len: None,
                ocr_text_sha256: None,
                web_path: None,
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

    fn act_type_element_metadata(
        role: &str,
        enabled: bool,
        keyboard_focusable: bool,
        patterns: Vec<UiaPattern>,
    ) -> synapse_a11y::ElementMetadataReadback {
        synapse_a11y::ElementMetadataReadback {
            name: "synthetic chrome edit".to_owned(),
            role: role.to_owned(),
            automation_id: Some("synthetic-input".to_owned()),
            bbox: Rect {
                x: 100,
                y: 200,
                w: 300,
                h: 40,
            },
            enabled,
            keyboard_focusable,
            patterns,
            value: Some("before".to_owned()),
        }
    }

    fn act_type_foreground_fallback_target(
        root_hwnd: i64,
        role: &str,
        bbox: Rect,
    ) -> ActTypeForegroundFallbackTarget {
        ActTypeForegroundFallbackTarget {
            element_id: format!("0x{root_hwnd:x}:0000002a00000001"),
            root_hwnd,
            process_name: "chrome.exe".to_owned(),
            role: role.to_owned(),
            automation_id_present: true,
            bbox,
            enabled: true,
            keyboard_focusable: true,
            patterns: vec![UiaPattern::Value, UiaPattern::Text],
            name_len: "synthetic chrome edit".chars().count(),
            value_len: Some("before".chars().count()),
        }
    }

    fn act_type_signature_for_fallback(
        foreground_hwnd: i64,
        focused_role: Option<&str>,
        focused_bbox: Option<Rect>,
    ) -> ActTypeTextSignature {
        ActTypeTextSignature {
            foreground_hwnd,
            foreground_pid: 20,
            foreground_process: "chrome.exe".to_owned(),
            foreground_title_sha256: non_empty_sha256("Synthetic - Google Chrome"),
            focused_element_id: focused_role
                .map(|_| format!("0x{foreground_hwnd:x}:0000002a00000002")),
            focused_role: focused_role.map(str::to_owned),
            focused_name_sha256: focused_role.and_then(non_empty_sha256),
            focused_value_len: Some("before".chars().count()),
            focused_value_sha256: Some(text_sha256("before")),
            focused_selected_text_sha256: None,
            focused_bbox,
            readback_source: Some(ACT_TYPE_TEXT_SOURCE_UIA_VALUE.to_owned()),
            has_text_readback: true,
            text_readback_attempts: vec![format!("{ACT_TYPE_TEXT_SOURCE_UIA_VALUE}:available")],
            cdp_status: None,
            cdp_endpoint_present: false,
            cdp_selected_target_id: None,
            cdp_active_has_element: None,
            cdp_active_is_editable: None,
            cdp_active_tag_name: None,
            cdp_active_id_sha256: None,
            cdp_active_name_sha256: None,
            cdp_active_value_len: None,
            cdp_active_value_sha256: None,
            cdp_active_error_code: None,
            cdp_active_error_detail_sha256: None,
            ocr_word_count: 0,
            ocr_text_len: None,
            ocr_text_sha256: None,
            web_path: Some("uia_only".to_owned()),
            browser_url_len: None,
            browser_url_sha256: None,
            browser_cdp_target_id: None,
            browser_url_readback_source: None,
        }
    }

    fn act_type_focused_candidate(role: &str, value: Option<&str>) -> ActTypeFocusedTextCandidate {
        ActTypeFocusedTextCandidate {
            element_id: "issue786-focused".to_owned(),
            role: role.to_owned(),
            name: String::new(),
            selected_text: None,
            bbox: Rect {
                x: 10,
                y: 10,
                w: 400,
                h: 40,
            },
            value: value.map(str::to_owned),
            patterns: Vec::new(),
        }
    }

    fn cdp_active_text_readback_for_test(
        value: Option<&str>,
        is_editable: bool,
        tag_name: &str,
    ) -> CdpActiveTextReadback {
        CdpActiveTextReadback {
            value: value.map(str::to_owned),
            target_id: Some("TARGET810".to_owned()),
            has_active_element: Some(true),
            is_editable: Some(is_editable),
            tag_name: Some(tag_name.to_owned()),
            id_sha256: non_empty_sha256("issue786-editor"),
            name_sha256: None,
            value_len: value.map(|value| value.chars().count()),
            value_sha256: value.map(text_sha256),
            error_code: None,
            error_detail_sha256: None,
            attempt: if value.is_some() {
                "cdp_active_element_value:available".to_owned()
            } else {
                "cdp_active_element_value:unavailable:active_element_not_editable".to_owned()
            },
        }
    }

    fn ocr_text_readback_for_test(value: Option<&str>) -> OcrTextReadback {
        OcrTextReadback {
            value: value.map(str::to_owned),
            word_count: value
                .map(|value| value.split_whitespace().count())
                .unwrap_or(0),
            value_len: value.map(|value| value.chars().count()),
            value_sha256: value.map(text_sha256),
            attempt: if value.is_some() {
                "ocr_focused_rect_text:available".to_owned()
            } else {
                "ocr_focused_rect_text:unavailable:no_ocr_words_in_focused_bbox".to_owned()
            },
        }
    }

    fn act_type_response_for_verify_delta() -> ActTypeResponse {
        ActTypeResponse {
            ok: true,
            chars_typed: 16,
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

    fn hwnd_keyboard_signature(
        text: &str,
        selection_start: u32,
        selection_end: u32,
    ) -> HwndKeyboardDeltaSignature {
        HwndKeyboardDeltaSignature {
            target: HwndKeyboardTargetState {
                root_hwnd: 0x1000,
                hwnd: 0x2000,
                class_name: "WindowsForms10.EDIT.synthetic".to_owned(),
                text_len: Some(text.chars().count()),
                text_sha256: Some(text_sha256(text)),
                selection_start: Some(selection_start),
                selection_end: Some(selection_end),
            },
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
