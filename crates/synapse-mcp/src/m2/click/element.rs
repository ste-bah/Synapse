use std::time::Instant;

use rmcp::ErrorData;
use synapse_action::{
    ActionError, DoubleClickTiming, ElementClickOutcome, EmitState, RecordingBackend,
    click_element_or_fallback,
};
use synapse_core::Point;
use tokio::time::{Duration, sleep};

use super::{
    action_error_to_mcp,
    schema::{ActClickElementTarget, ActClickParams, ActClickResponse},
};

pub(super) async fn execute_element_click(
    params: &ActClickParams,
    element: &ActClickElementTarget,
    recording: Option<&RecordingBackend>,
    timing: DoubleClickTiming,
    started: Instant,
) -> Result<ActClickResponse, ErrorData> {
    if !params.use_invoke_pattern {
        return Err(action_error_to_mcp(&ActionError::BackendUnavailable {
            detail: format!(
                "act_click element target {} requires the dedicated coordinate fallback wiring issue when use_invoke_pattern=false",
                element.element_id
            ),
        }));
    }

    let mut state = EmitState::new();
    let mut used_invoke_pattern = false;
    let mut backend_used = "software";
    for click_index in 0..params.clicks {
        let outcome = if let Some(recording) = recording {
            click_element_or_fallback(&element.element_id, recording, &mut state, params.button)
        } else {
            let backend = synapse_action::backend::software::SoftwareBackend::new();
            click_element_or_fallback(&element.element_id, &backend, &mut state, params.button)
        }
        .map_err(|error| action_error_to_mcp(&error))?;

        match outcome {
            ElementClickOutcome::Invoked => {
                trace_element_click_outcome(element, click_index, "invoked", None);
                used_invoke_pattern = true;
                backend_used = "uia";
            }
            ElementClickOutcome::CoordinateFallback(plan) => {
                trace_element_click_outcome(
                    element,
                    click_index,
                    "coordinate_fallback",
                    Some(plan.screen_point),
                );
                backend_used = "software";
            }
        }

        if click_index + 1 < params.clicks {
            sleep(Duration::from_millis(u64::from(
                timing.inter_click_delay_ms,
            )))
            .await;
        }
    }

    Ok(ActClickResponse {
        ok: true,
        used_invoke_pattern,
        backend_used: backend_used.to_owned(),
        double_click_window_ms: timing.window_ms,
        inter_click_delay_ms: timing.inter_click_delay_ms,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

fn trace_element_click_outcome(
    element: &ActClickElementTarget,
    click_index: u8,
    outcome: &'static str,
    fallback_screen_point: Option<Point>,
) {
    tracing::info!(
        code = "M2_ACT_CLICK_ELEMENT_READBACK",
        kind = "act_click",
        element_id = %element.element_id,
        click_number = u32::from(click_index) + 1,
        outcome,
        fallback_screen_x = fallback_screen_point.map(|point| point.x),
        fallback_screen_y = fallback_screen_point.map(|point| point.y),
        "source_of_truth=action_backend tool=act_click element_click_after"
    );
}
