use std::{sync::Arc, time::Instant};

use rmcp::ErrorData;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_action::{
    ActionBackend, ActionError, ActionHandle, DoubleClickTiming, EmitState, RecordedInput,
    RecordingBackend, cached_double_click_timing,
};
use synapse_core::{
    Action, AimCurve, AimNaturalParams, Backend, ButtonAction, ElementId, MouseButton, MouseTarget,
    Point, error_codes,
};
use tokio::time::{Duration, sleep};

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
    pub double_click_window_ms: u32,
    pub inter_click_delay_ms: u32,
    pub elapsed_ms: u32,
}

pub async fn act_click_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActClickParams,
) -> Result<ActClickResponse, ErrorData> {
    validate_click_params(&params)?;
    let started = Instant::now();
    let double_click_timing = cached_double_click_timing();
    let target = mouse_target(&params)?;
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
        execute_recording(&recording, &actions, params.clicks, double_click_timing).await?;
    } else {
        execute_actor_actions(handle, actions, double_click_timing).await?;
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

async fn execute_actor_actions(
    handle: ActionHandle,
    actions: Vec<Action>,
    timing: DoubleClickTiming,
) -> Result<(), ErrorData> {
    let action_count = actions.len();
    for (action_index, action) in actions.into_iter().enumerate() {
        handle
            .execute(action)
            .await
            .map_err(|error| action_error_to_mcp(&error))?;
        maybe_sleep_between_clicks(action_index, action_count, timing).await;
    }
    Ok(())
}

async fn execute_recording(
    recording: &RecordingBackend,
    actions: &[Action],
    click_count: u8,
    timing: DoubleClickTiming,
) -> Result<(), ErrorData> {
    let before_events = recording.events();
    let before_event_count = before_events.len();
    let mut emit_state = EmitState::new();
    let action_count = actions.len();
    for (action_index, action) in actions.iter().enumerate() {
        recording
            .execute(action, &mut emit_state)
            .map_err(|error| action_error_to_mcp(&error))?;
        maybe_sleep_between_clicks(action_index, action_count, timing).await;
    }
    let after_events = recording.events();
    let new_events = &after_events[before_event_count..];
    let event_sequence = event_sequence(new_events);
    let button_event_count = mouse_button_event_count(new_events);
    let scheduled_inter_click_total_ms =
        u32::from(click_count.saturating_sub(1)) * timing.inter_click_delay_ms;
    tracing::info!(
        code = "M2_ACT_CLICK_RECORDING_READBACK",
        kind = "act_click",
        before_event_count,
        after_event_count = after_events.len(),
        new_event_count = new_events.len(),
        click_count,
        button_event_count,
        double_click_window_ms = timing.window_ms,
        inter_click_delay_ms = timing.inter_click_delay_ms,
        scheduled_inter_click_total_ms,
        event_sequence,
        ?new_events,
        "source_of_truth=recording_backend tool=act_click after_events_readback"
    );
    Ok(())
}

async fn maybe_sleep_between_clicks(
    action_index: usize,
    action_count: usize,
    timing: DoubleClickTiming,
) {
    if should_delay_between_clicks(action_index, action_count) {
        let delay = Duration::from_millis(u64::from(timing.inter_click_delay_ms));
        sleep(delay).await;
    }
}

const fn should_delay_between_clicks(action_index: usize, action_count: usize) -> bool {
    action_index >= 1 && action_index + 1 < action_count
}

fn mouse_button_event_count(events: &[RecordedInput]) -> usize {
    events
        .iter()
        .filter(|event| {
            matches!(
                event,
                RecordedInput::MouseButtonDown { .. } | RecordedInput::MouseButtonUp { .. }
            )
        })
        .count()
}

fn event_sequence(events: &[RecordedInput]) -> String {
    events.iter().map(event_label).collect::<Vec<_>>().join(">")
}

fn event_label(event: &RecordedInput) -> String {
    match event {
        RecordedInput::MouseMove {
            to,
            curve,
            duration_ms,
        } => format!(
            "mouse_move:{}:{}:{}",
            mouse_target_label(to),
            curve_label(curve),
            duration_ms
        ),
        RecordedInput::MouseButtonDown { button } => format!("down:{}", button_label(*button)),
        RecordedInput::MouseButtonUp { button } => format!("up:{}", button_label(*button)),
        RecordedInput::DelayMs { ms } => format!("delay:{ms}"),
        other => format!("{other:?}"),
    }
}

fn mouse_target_label(target: &MouseTarget) -> String {
    match target {
        MouseTarget::Screen { point } => format!("screen({},{})", point.x, point.y),
        MouseTarget::Element { element_id } => format!("element({element_id})"),
    }
}

const fn curve_label(curve: &AimCurve) -> &'static str {
    match curve {
        AimCurve::Natural { .. } => "natural_fast",
        AimCurve::Instant => "instant",
        AimCurve::Linear => "linear",
        AimCurve::EaseInOut => "ease_in_out",
        AimCurve::Bezier { .. } => "bezier",
    }
}

const fn button_label(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
        MouseButton::X1 => "x1",
        MouseButton::X2 => "x2",
    }
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
            None,
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
