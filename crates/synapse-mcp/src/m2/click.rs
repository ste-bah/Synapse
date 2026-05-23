use std::time::Instant;

use rmcp::ErrorData;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_action::{ActionError, ActionHandle};
use synapse_core::{
    Action, AimCurve, AimNaturalParams, Backend, ButtonAction, ElementId, MouseButton, MouseTarget,
    Point, error_codes,
};

use crate::m1::mcp_error;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActClickParams {
    pub target: ActClickTarget,
    #[serde(default = "default_click_button")]
    #[schemars(default = "default_click_button")]
    pub button: MouseButton,
    #[serde(default = "default_click_count")]
    #[schemars(default = "default_click_count", range(min = 1, max = 3))]
    pub clicks: u8,
    #[serde(default)]
    pub modifiers: Vec<ClickModifier>,
    #[serde(default = "default_click_curve")]
    #[schemars(default = "default_click_curve")]
    pub curve: ClickCurve,
    #[serde(default = "default_click_duration_ms")]
    #[schemars(default = "default_click_duration_ms")]
    pub duration_ms: u32,
    #[serde(default = "default_click_backend")]
    #[schemars(default = "default_click_backend")]
    pub backend: Backend,
    #[serde(default = "default_use_invoke_pattern")]
    #[schemars(default = "default_use_invoke_pattern")]
    pub use_invoke_pattern: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(untagged)]
pub enum ActClickTarget {
    Element(ActClickElementTarget),
    Point(ActClickPointTarget),
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActClickElementTarget {
    pub element_id: ElementId,
}

#[derive(Copy, Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActClickPointTarget {
    pub x: i32,
    pub y: i32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ClickModifier {
    Ctrl,
    Shift,
    Alt,
    Super,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClickCurve {
    Natural,
    Instant,
    Linear,
    EaseInOut,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActClickResponse {
    pub ok: bool,
    pub used_invoke_pattern: bool,
    pub backend_used: String,
    pub elapsed_ms: u32,
}

pub async fn act_click_with_handle(
    handle: ActionHandle,
    params: ActClickParams,
) -> Result<ActClickResponse, ErrorData> {
    validate_click_params(&params)?;
    let started = Instant::now();
    let target = mouse_target(&params)?;
    handle
        .execute(Action::MouseMove {
            to: target,
            curve: params.curve.to_aim_curve(),
            duration_ms: params.duration_ms,
            backend: params.backend,
        })
        .await
        .map_err(|error| action_error_to_mcp(&error))?;
    for _ in 0..params.clicks {
        handle
            .execute(Action::MouseButton {
                button: params.button,
                action: ButtonAction::Press,
                hold_ms: 0,
                backend: params.backend,
            })
            .await
            .map_err(|error| action_error_to_mcp(&error))?;
    }
    Ok(ActClickResponse {
        ok: true,
        used_invoke_pattern: false,
        backend_used: backend_used_name(params.backend).to_owned(),
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

impl ClickCurve {
    const fn to_aim_curve(self) -> AimCurve {
        match self {
            Self::Natural => AimCurve::Natural {
                params: AimNaturalParams::FAST,
            },
            Self::Instant => AimCurve::Instant,
            Self::Linear => AimCurve::Linear,
            Self::EaseInOut => AimCurve::EaseInOut,
        }
    }
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

fn mouse_target(params: &ActClickParams) -> Result<MouseTarget, ErrorData> {
    match &params.target {
        ActClickTarget::Point(point) => Ok(MouseTarget::Screen {
            point: Point {
                x: point.x,
                y: point.y,
            },
        }),
        ActClickTarget::Element(element) => {
            let mode = if params.use_invoke_pattern {
                "InvokePattern"
            } else {
                "coordinate fallback"
            };
            Err(action_error_to_mcp(&ActionError::BackendUnavailable {
                detail: format!(
                    "act_click element target {} requires the dedicated {mode} wiring issue",
                    element.element_id
                ),
            }))
        }
    }
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

const fn default_click_button() -> MouseButton {
    MouseButton::Left
}

const fn default_click_count() -> u8 {
    1
}

const fn default_click_curve() -> ClickCurve {
    ClickCurve::Natural
}

const fn default_click_duration_ms() -> u32 {
    50
}

const fn default_click_backend() -> Backend {
    Backend::Auto
}

const fn default_use_invoke_pattern() -> bool {
    true
}

const fn backend_used_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Auto | Backend::Software => "software",
        Backend::Vigem => "vigem",
        Backend::Hardware => "hardware",
    }
}

#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

    use super::{
        ActClickParams, ActClickPointTarget, ActClickTarget, act_click_with_handle,
        default_click_backend, default_click_button, default_click_count, default_click_curve,
        default_click_duration_ms, default_use_invoke_pattern,
    };
    use synapse_action::ActionEmitter;

    #[tokio::test]
    async fn coordinate_click_leaves_actor_held_state_empty() {
        let cancel = CancellationToken::new();
        let (handle, snapshot_handle, join) = ActionEmitter::spawn(cancel.clone());
        let before = match snapshot_handle.snapshot().await {
            Ok(snapshot) => snapshot,
            Err(err) => panic!("before snapshot failed: {err}"),
        };
        println!(
            "source_of_truth=act_click_actor edge=coordinate before=held_buttons:{:?} held_keys:{:?}",
            before.held_buttons, before.held_keys
        );
        let response = match act_click_with_handle(
            handle,
            ActClickParams {
                target: ActClickTarget::Point(ActClickPointTarget { x: 12, y: 34 }),
                button: default_click_button(),
                clicks: default_click_count(),
                modifiers: Vec::new(),
                curve: default_click_curve(),
                duration_ms: default_click_duration_ms(),
                backend: default_click_backend(),
                use_invoke_pattern: default_use_invoke_pattern(),
            },
        )
        .await
        {
            Ok(response) => response,
            Err(err) => panic!("act_click failed: {err}"),
        };
        let after = match snapshot_handle.snapshot().await {
            Ok(snapshot) => snapshot,
            Err(err) => panic!("after snapshot failed: {err}"),
        };
        println!(
            "source_of_truth=act_click_actor edge=coordinate after=ok:{} backend_used:{} held_buttons:{:?} held_keys:{:?}",
            response.ok, response.backend_used, after.held_buttons, after.held_keys
        );
        assert!(response.ok);
        assert!(!response.used_invoke_pattern);
        assert_eq!(response.backend_used, "software");
        assert!(after.held_buttons.is_empty());
        assert!(after.held_keys.is_empty());
        cancel.cancel();
        let _final_snapshot = match join.await {
            Ok(snapshot) => snapshot,
            Err(err) => panic!("join failed: {err}"),
        };
    }
}
