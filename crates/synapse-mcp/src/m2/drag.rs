use std::{sync::Arc, time::Instant};

use rmcp::ErrorData;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_action::{
    ActionBackend, ActionError, ActionHandle, EmitState, RecordedInput, RecordingBackend,
};
use synapse_core::{
    Action, AimCurve, AimNaturalParams, Backend, ElementId, MouseButton, MouseTarget, Point,
};

#[cfg(windows)]
use synapse_a11y::uiautomation::types::Rect as UiaRect;

use crate::m1::mcp_error;

const DEFAULT_DRAG_DURATION_MS: u32 = 200;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActDragParams {
    pub from: ActDragTarget,
    pub to: ActDragTarget,
    #[serde(default = "default_drag_button")]
    #[schemars(default = "default_drag_button")]
    pub button: DragButton,
    #[serde(default = "default_drag_curve")]
    #[schemars(default = "default_drag_curve")]
    pub curve: DragCurve,
    #[serde(default = "default_drag_duration_ms")]
    #[schemars(default = "default_drag_duration_ms")]
    pub duration_ms: u32,
    #[serde(default = "default_drag_backend")]
    #[schemars(default = "default_drag_backend")]
    pub backend: DragBackend,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(untagged)]
pub enum ActDragTarget {
    Point(ActDragPointTarget),
    Element(ActDragElementTarget),
}

#[derive(Copy, Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActDragPointTarget {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActDragElementTarget {
    pub element_id: ElementId,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DragButton {
    Left,
    Right,
    Middle,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DragCurve {
    Natural,
    Instant,
    Linear,
    EaseInOut,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DragBackend {
    Software,
    Hardware,
    Auto,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActDragResponse {
    pub ok: bool,
    pub button_used: DragButton,
    pub curve_used: DragCurve,
    pub duration_ms: u32,
    pub distance_px: f64,
    pub backend_used: String,
    pub elapsed_ms: u32,
}

pub async fn act_drag_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActDragParams,
) -> Result<ActDragResponse, ErrorData> {
    let started = Instant::now();
    let from = target_point(&params.from, "from")?;
    let to = target_point(&params.to, "to")?;
    let backend = params.backend.to_backend();
    let action = Action::MouseDrag {
        from,
        to,
        button: params.button.to_mouse_button(),
        curve: params.curve.to_aim_curve(),
        duration_ms: params.duration_ms,
        backend,
    };

    if let Some(recording) = recording {
        execute_recording(&recording, &action)?;
    } else {
        handle
            .execute(action)
            .await
            .map_err(|error| action_error_to_mcp(&error))?;
    }

    Ok(ActDragResponse {
        ok: true,
        button_used: params.button,
        curve_used: params.curve,
        duration_ms: params.duration_ms,
        distance_px: from.distance_to(to),
        backend_used: backend_used_name(backend).to_owned(),
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

impl DragButton {
    const fn to_mouse_button(self) -> MouseButton {
        match self {
            Self::Left => MouseButton::Left,
            Self::Right => MouseButton::Right,
            Self::Middle => MouseButton::Middle,
        }
    }
}

impl DragCurve {
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

impl DragBackend {
    const fn to_backend(self) -> Backend {
        match self {
            Self::Software => Backend::Software,
            Self::Hardware => Backend::Hardware,
            Self::Auto => Backend::Auto,
        }
    }
}

fn target_point(target: &ActDragTarget, role: &'static str) -> Result<Point, ErrorData> {
    match target {
        ActDragTarget::Point(point) => Ok(Point {
            x: point.x,
            y: point.y,
        }),
        ActDragTarget::Element(element) => element_center(&element.element_id, role),
    }
}

#[cfg(windows)]
fn element_center(element_id: &ElementId, role: &'static str) -> Result<Point, ErrorData> {
    let element = synapse_a11y::re_resolve(element_id).map_err(|err| {
        action_error_to_mcp(&ActionError::ElementNotResolved {
            detail: format!("act_drag {role} element {element_id} could not be resolved: {err}"),
        })
    })?;
    let rect = element.get_bounding_rectangle().map_err(|err| {
        action_error_to_mcp(&ActionError::TargetInvalid {
            detail: format!("act_drag {role} element {element_id} bbox unavailable: {err}"),
        })
    })?;
    center_from_rect_edges(RectEdges::from(rect)).map_err(|error| action_error_to_mcp(&error))
}

#[cfg(not(windows))]
fn element_center(element_id: &ElementId, role: &'static str) -> Result<Point, ErrorData> {
    Err(action_error_to_mcp(&ActionError::BackendUnavailable {
        detail: format!(
            "act_drag {role} element target {element_id} requires Windows UI Automation bbox resolution"
        ),
    }))
}

#[cfg(windows)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct RectEdges {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(windows)]
impl From<UiaRect> for RectEdges {
    fn from(value: UiaRect) -> Self {
        Self {
            left: value.get_left(),
            top: value.get_top(),
            right: value.get_right(),
            bottom: value.get_bottom(),
        }
    }
}

#[cfg(windows)]
fn center_from_rect_edges(rect: RectEdges) -> Result<Point, ActionError> {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return Err(ActionError::TargetInvalid {
            detail: format!("act_drag element bbox is empty or inverted: {rect:?}"),
        });
    }

    let width = i64::from(rect.right) - i64::from(rect.left);
    let height = i64::from(rect.bottom) - i64::from(rect.top);
    let x = i64::from(rect.left) + width / 2;
    let y = i64::from(rect.top) + height / 2;

    Ok(Point {
        x: i32::try_from(x).map_err(|err| ActionError::TargetInvalid {
            detail: format!("act_drag element bbox center x overflowed i32: {err}"),
        })?,
        y: i32::try_from(y).map_err(|err| ActionError::TargetInvalid {
            detail: format!("act_drag element bbox center y overflowed i32: {err}"),
        })?,
    })
}

fn execute_recording(recording: &RecordingBackend, action: &Action) -> Result<(), ErrorData> {
    let before_events = recording.events();
    let before_event_count = before_events.len();
    let mut emit_state = EmitState::new();
    recording
        .execute(action, &mut emit_state)
        .map_err(|error| action_error_to_mcp(&error))?;
    let after_events = recording.events();
    let new_events = &after_events[before_event_count..];
    let event_sequence = event_sequence(new_events);
    tracing::info!(
        code = "M2_ACT_DRAG_RECORDING_READBACK",
        kind = "act_drag",
        before_event_count,
        after_event_count = after_events.len(),
        new_event_count = new_events.len(),
        event_sequence,
        ?new_events,
        "source_of_truth=recording_backend tool=act_drag after_events_readback"
    );
    Ok(())
}

fn event_sequence(events: &[RecordedInput]) -> String {
    events.iter().map(event_label).collect::<Vec<_>>().join(">")
}

fn event_label(event: &RecordedInput) -> String {
    match event {
        RecordedInput::MouseButtonDown { button } => format!("down:{}", button_label(*button)),
        RecordedInput::MouseMove {
            to,
            curve,
            duration_ms,
        } => format!(
            "mouse_move:{}:{}:{duration_ms}",
            mouse_target_label(to),
            curve_label(curve)
        ),
        RecordedInput::MouseButtonUp { button } => format!("up:{}", button_label(*button)),
        other => format!("{other:?}"),
    }
}

fn mouse_target_label(target: &MouseTarget) -> String {
    match target {
        MouseTarget::Screen { point } => format!("screen({},{})", point.x, point.y),
        MouseTarget::Element { element_id } => format!("element({element_id})"),
    }
}

fn curve_label(curve: &AimCurve) -> &'static str {
    match curve {
        AimCurve::Natural {
            params: AimNaturalParams::FAST,
        } => "natural_fast",
        AimCurve::Natural { .. } => "natural",
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

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

const fn default_drag_button() -> DragButton {
    DragButton::Left
}

const fn default_drag_curve() -> DragCurve {
    DragCurve::Natural
}

const fn default_drag_duration_ms() -> u32 {
    DEFAULT_DRAG_DURATION_MS
}

const fn default_drag_backend() -> DragBackend {
    DragBackend::Auto
}

const fn backend_used_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Auto | Backend::Software => "software",
        Backend::Hardware => "hardware",
        Backend::Vigem => "vigem",
    }
}
