use std::{sync::Arc, time::Instant};

use rmcp::{ErrorData, model::ErrorCode};
use serde_json::json;
use synapse_action::{ActionError, ActionHandle, RecordingBackend, cached_double_click_timing};
use synapse_core::{Action, Backend, ButtonAction, ElementId, MouseTarget, Point, error_codes};

use crate::m1::mcp_error;

mod element;
mod record;
mod schema;
#[cfg(test)]
mod tests;

use schema::ActClickTarget;
pub use schema::{ActClickParams, ActClickPostcondition, ActClickResponse};

const MAX_CLICK_HOLD_MS: u32 = 30_000;
const SUPPORTED_UIA_CLICK_PATTERNS: [&str; 5] = [
    "InvokePattern",
    "TogglePattern",
    "SelectionItemPattern",
    "ExpandCollapsePattern",
    "LegacyIAccessiblePattern.DoDefaultAction",
];

pub async fn act_click_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActClickParams,
) -> Result<ActClickResponse, ErrorData> {
    validate_click_params(&params)?;
    if params.deprecated_curve_alias_used {
        tracing::warn!(
            code = "M2_ACT_CLICK_DEPRECATED_CURVE_ALIAS",
            kind = "act_click",
            replacement = "velocity_profile",
            "act_click deprecated curve alias accepted; use velocity_profile for coordinate-move timing"
        );
    }
    let started = Instant::now();
    let double_click_timing = cached_double_click_timing();
    // #686: a web element id (cdcd sentinel) routes through CDP instead of UIA.
    #[cfg(windows)]
    if let ActClickTarget::Element(element) = &params.target
        && let Some(backend) = synapse_a11y::cdp_backend_from_element_id(&element.element_id)
    {
        ensure_element_transport_backend_allowed(&params, "CDP")?;
        return execute_cdp_click(&params, element, backend, double_click_timing, started).await;
    }
    if let ActClickTarget::Element(element) = &params.target {
        ensure_element_transport_backend_allowed(&params, "UIA")?;
        return element::execute_element_click(
            handle,
            &params,
            element,
            recording.as_deref(),
            double_click_timing,
            started,
        )
        .await;
    }

    let target = point_mouse_target(&params.target)?;
    let mut actions = Vec::with_capacity(usize::from(params.clicks) + 1);
    actions.push(Action::MouseMove {
        to: target,
        curve: params.velocity_profile.to_aim_curve(),
        duration_ms: params.duration_ms,
        backend: params.backend,
    });
    for _ in 0..params.clicks {
        actions.push(Action::MouseButton {
            button: params.button,
            action: ButtonAction::Press,
            hold_ms: params.hold_ms,
            backend: params.backend,
        });
    }

    if let Some(recording) = recording {
        record::execute_recording(&recording, &actions, params.clicks, double_click_timing).await?;
    } else {
        record::execute_actor_actions(handle, actions, double_click_timing).await?;
    }

    Ok(ActClickResponse {
        ok: true,
        used_invoke_pattern: false,
        backend_used: backend_used_name(params.backend).to_owned(),
        postcondition: schema::postcondition_not_requested(),
        press_hold_ms: params.hold_ms,
        double_click_window_ms: double_click_timing.window_ms,
        inter_click_delay_ms: double_click_timing.inter_click_delay_ms,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

/// Routes a click on a CDP web element id through CDP (#686): resolve the
/// browser's debug endpoint from the element's window, scroll the node into
/// view, and dispatch the click in viewport coordinates. Fail-loud if the
/// endpoint is gone or the node cannot be resolved.
#[cfg(windows)]
async fn execute_cdp_click(
    params: &ActClickParams,
    element: &schema::ActClickElementTarget,
    backend_node_id: i64,
    double_click_timing: synapse_action::DoubleClickTiming,
    started: Instant,
) -> Result<ActClickResponse, ErrorData> {
    use synapse_core::MouseButton;

    let hwnd = element
        .element_id
        .parts()
        .map_err(|err| {
            mcp_error(
                error_codes::ACTION_ELEMENT_NOT_RESOLVED,
                format!("web element id is malformed: {err}"),
            )
        })?
        .hwnd;
    let endpoint = synapse_a11y::endpoint_for_window(hwnd).ok_or_else(|| {
        mcp_error(
            error_codes::A11Y_CDP_UNREACHABLE,
            format!(
                "no reachable CDP endpoint for web element {} (browser closed or debug port gone)",
                element.element_id
            ),
        )
    })?;
    // Foreground window title disambiguates which tab owns the per-document node.
    let title_hint = synapse_a11y::foreground_context(hwnd)
        .map(|context| context.window_title)
        .unwrap_or_default();
    let button = match params.button {
        MouseButton::Left => synapse_a11y::CdpMouseButton::Left,
        MouseButton::Right => synapse_a11y::CdpMouseButton::Right,
        MouseButton::Middle => synapse_a11y::CdpMouseButton::Middle,
        other => {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!("act_click button {other:?} is not supported for web (CDP) elements"),
            ));
        }
    };

    synapse_a11y::cdp_click_node(
        &endpoint,
        &title_hint,
        backend_node_id,
        button,
        i64::from(params.clicks),
    )
    .await
    .map_err(|err| action_error_to_mcp(&a11y_to_action_error(&err)))?;

    Ok(ActClickResponse {
        ok: true,
        used_invoke_pattern: false,
        backend_used: "cdp".to_owned(),
        postcondition: schema::postcondition_not_requested(),
        press_hold_ms: params.hold_ms,
        double_click_window_ms: double_click_timing.window_ms,
        inter_click_delay_ms: double_click_timing.inter_click_delay_ms,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

/// Maps an a11y CDP error to an action error so it surfaces with the same shape
/// as other action failures.
#[cfg(windows)]
fn a11y_to_action_error(err: &synapse_a11y::A11yError) -> ActionError {
    ActionError::TargetInvalid {
        detail: format!("{} ({})", err, err.code()),
    }
}

fn validate_click_params(params: &ActClickParams) -> Result<(), ErrorData> {
    if !(1..=3).contains(&params.clicks) {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("act_click clicks must be in 1..=3, got {}", params.clicks),
        ));
    }
    if params.hold_ms == 0 {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_click hold_ms must be at least 1",
        ));
    }
    if params.hold_ms > MAX_CLICK_HOLD_MS {
        return Err(action_error_to_mcp(&ActionError::HoldExceededMax {
            detail: format!(
                "act_click hold_ms {} exceeds max {MAX_CLICK_HOLD_MS}",
                params.hold_ms
            ),
        }));
    }
    if !(50..=5000).contains(&params.verify_timeout_ms) {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!(
                "act_click verify_timeout_ms must be in 50..=5000, got {}",
                params.verify_timeout_ms
            ),
        ));
    }
    if !params.modifiers.is_empty() {
        return Err(action_error_to_mcp(&ActionError::BackendUnavailable {
            detail: "act_click modifiers are not wired in the M2 click schema slice".to_owned(),
        }));
    }
    Ok(())
}

fn ensure_element_transport_backend_allowed(
    params: &ActClickParams,
    transport: &str,
) -> Result<(), ErrorData> {
    if matches!(params.backend, Backend::Vigem | Backend::Hardware) {
        return Err(action_error_to_mcp(&ActionError::BackendUnavailable {
            detail: format!(
                "act_click element target requested backend={} but {transport} element delivery is only valid for backend=auto or backend=software; no fallback delivery was attempted",
                backend_used_name(params.backend)
            ),
        }));
    }
    Ok(())
}

fn point_mouse_target(target: &ActClickTarget) -> Result<MouseTarget, ErrorData> {
    match target {
        ActClickTarget::Point(point) => Ok(MouseTarget::Screen {
            point: Point {
                x: point.x,
                y: point.y,
            },
        }),
        ActClickTarget::Element(element) => Err(action_error_to_mcp(&ActionError::TargetInvalid {
            detail: format!(
                "act_click element target {} reached the point-target path unexpectedly",
                element.element_id
            ),
        })),
    }
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    match error {
        ActionError::TransientElementExpired { element_id, detail } => {
            transient_element_expired_error(element_id, detail)
        }
        ActionError::ElementPatternUnsupported { element_id, detail } => {
            element_pattern_unsupported_error(element_id, detail)
        }
        _ => mcp_error(error.code(), error.to_string()),
    }
}

fn transient_element_expired_error(element_id: &ElementId, detail: &str) -> ErrorData {
    let root_hwnd = element_id.parts().ok().map(|parts| parts.hwnd);
    let recommended_pattern = "Call observe or find again immediately before acting on the transient UI, then pass the fresh element_id to act_click; do not reuse element_ids from expired toast/snackbar observations.";
    tracing::warn!(
        code = error_codes::TRANSIENT_ELEMENT_EXPIRED,
        element_id = %element_id,
        root_hwnd,
        detail,
        recommended_pattern,
        "act_click transient UI element expired before dispatch; no fallback click attempted"
    );
    ErrorData::new(
        ErrorCode(-32099),
        format!("transient UI element expired before act_click dispatch: {detail}"),
        Some(json!({
            "code": error_codes::TRANSIENT_ELEMENT_EXPIRED,
            "detail_code": "UIA_ELEMENT_STALE_AFTER_OBSERVE",
            "transient": true,
            "fallback_attempted": false,
            "element_id": element_id.to_string(),
            "root_hwnd": root_hwnd,
            "source_of_truth": "live UI Automation re-resolution under the element_id root HWND",
            "recommended_next_tools": ["observe", "find", "act_click"],
            "recommended_pattern": recommended_pattern,
            "detail": detail,
        })),
    )
}

fn element_pattern_unsupported_error(element_id: &ElementId, detail: &str) -> ErrorData {
    let root_hwnd = element_id.parts().ok().map(|parts| parts.hwnd);
    tracing::warn!(
        code = error_codes::ACTION_ELEMENT_PATTERN_UNSUPPORTED,
        element_id = %element_id,
        root_hwnd,
        attempted_patterns = ?SUPPORTED_UIA_CLICK_PATTERNS,
        detail,
        fallback_attempted = false,
        "act_click element target exposes no supported UIA click control pattern; no fallback delivery attempted"
    );
    ErrorData::new(
        ErrorCode(-32099),
        format!("element target exposes no supported UIA click control pattern: {detail}"),
        Some(json!({
            "code": error_codes::ACTION_ELEMENT_PATTERN_UNSUPPORTED,
            "detail_code": "UIA_CONTROL_PATTERN_UNSUPPORTED",
            "transient": false,
            "fallback_attempted": false,
            "element_id": element_id.to_string(),
            "root_hwnd": root_hwnd,
            "attempted_patterns": SUPPORTED_UIA_CLICK_PATTERNS,
            "source_of_truth": "live UI Automation control-pattern availability on the re-resolved element",
            "router_escalation_required": true,
            "router_next_tier": "postmessage",
            "detail": detail,
        })),
    )
}

const fn backend_used_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Auto | Backend::Software => "software",
        Backend::Vigem => "vigem",
        Backend::Hardware => "hardware",
    }
}
