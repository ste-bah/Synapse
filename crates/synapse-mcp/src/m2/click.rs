use std::{sync::Arc, time::Instant};

use rmcp::ErrorData;
use synapse_action::{ActionError, ActionHandle, RecordingBackend, cached_double_click_timing};
use synapse_core::{Action, Backend, ButtonAction, MouseTarget, Point, error_codes};

use crate::m1::mcp_error;

mod element;
mod record;
mod schema;
#[cfg(test)]
mod tests;

use schema::ActClickTarget;
pub use schema::{ActClickParams, ActClickResponse};

pub async fn act_click_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActClickParams,
) -> Result<ActClickResponse, ErrorData> {
    validate_click_params(&params)?;
    let started = Instant::now();
    let double_click_timing = cached_double_click_timing();
    if let ActClickTarget::Element(element) = &params.target {
        return element::execute_element_click(
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
        curve: params.curve.to_aim_curve(),
        duration_ms: params.duration_ms,
        backend: params.backend,
    });
    for _ in 0..params.clicks {
        actions.push(Action::MouseButton {
            button: params.button,
            action: ButtonAction::Press,
            hold_ms: 0,
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
        double_click_window_ms: double_click_timing.window_ms,
        inter_click_delay_ms: double_click_timing.inter_click_delay_ms,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

fn validate_click_params(params: &ActClickParams) -> Result<(), ErrorData> {
    if !(1..=3).contains(&params.clicks) {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("act_click clicks must be in 1..=3, got {}", params.clicks),
        ));
    }
    if !params.modifiers.is_empty() {
        return Err(action_error_to_mcp(&ActionError::BackendUnavailable {
            detail: "act_click modifiers are not wired in the M2 click schema slice".to_owned(),
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
    mcp_error(error.code(), error.to_string())
}

const fn backend_used_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Auto | Backend::Software => "software",
        Backend::Vigem => "vigem",
        Backend::Hardware => "hardware",
    }
}
