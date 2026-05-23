use std::{sync::Arc, time::Instant};

use rmcp::ErrorData;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_action::{
    ActionBackend, ActionError, ActionHandle, EmitState, RecordedInput, RecordingBackend,
};
use synapse_core::{Action, AimCurve, AimNaturalParams, Backend, ElementId, MouseTarget, Point};

use crate::m1::mcp_error;

const DEFAULT_DEADLINE_MS: u32 = 80;
const SNAP_DURATION_MS: u32 = 50;
const FLICK_DURATION_MS: u32 = 35;
const NATURAL_DURATION_MS: u32 = 150;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActAimParams {
    pub target: ActAimTarget,
    #[serde(default = "default_aim_style")]
    #[schemars(default = "default_aim_style")]
    pub style: AimStyleParam,
    #[serde(default = "default_deadline_ms")]
    #[schemars(default = "default_deadline_ms")]
    pub deadline_ms: u32,
    #[serde(default = "default_aim_backend")]
    #[schemars(default = "default_aim_backend")]
    pub backend: AimBackend,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(untagged)]
pub enum ActAimTarget {
    Point(ActAimPointTarget),
    Element(ActAimElementTarget),
    Track(ActAimTrackTarget),
}

#[derive(Copy, Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActAimPointTarget {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActAimElementTarget {
    pub element_id: ElementId,
}

#[derive(Copy, Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActAimTrackTarget {
    pub track_id: u64,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AimStyleParam {
    Snap,
    Flick,
    Natural,
    Track,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AimBackend {
    Software,
    Hardware,
    Auto,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActAimResponse {
    pub ok: bool,
    pub style_used: AimStyleParam,
    pub duration_ms: u32,
    pub backend_used: String,
    pub elapsed_ms: u32,
}

pub async fn act_aim_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActAimParams,
) -> Result<ActAimResponse, ErrorData> {
    let started = Instant::now();
    let target = mouse_target(&params)?;
    let duration_ms = duration_ms(&params);
    let backend = params.backend.to_backend();
    let action = Action::MouseMove {
        to: target,
        curve: AimCurve::Natural {
            params: AimNaturalParams::FAST,
        },
        duration_ms,
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

    Ok(ActAimResponse {
        ok: true,
        style_used: params.style,
        duration_ms,
        backend_used: backend_used_name(backend).to_owned(),
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

impl AimBackend {
    const fn to_backend(self) -> Backend {
        match self {
            Self::Software => Backend::Software,
            Self::Hardware => Backend::Hardware,
            Self::Auto => Backend::Auto,
        }
    }
}

fn mouse_target(params: &ActAimParams) -> Result<MouseTarget, ErrorData> {
    if params.style == AimStyleParam::Track {
        return Err(track_unavailable());
    }
    match &params.target {
        ActAimTarget::Point(point) => Ok(MouseTarget::Screen {
            point: Point {
                x: point.x,
                y: point.y,
            },
        }),
        ActAimTarget::Element(element) => {
            Err(action_error_to_mcp(&ActionError::BackendUnavailable {
                detail: format!(
                    "act_aim element target {} requires the dedicated target resolution issue",
                    element.element_id
                ),
            }))
        }
        ActAimTarget::Track(track) => Err(action_error_to_mcp(&ActionError::BackendUnavailable {
            detail: format!(
                "act_aim track target {} requires the reflex runtime lands at M3",
                track.track_id
            ),
        })),
    }
}

const fn duration_ms(params: &ActAimParams) -> u32 {
    if params.deadline_ms != DEFAULT_DEADLINE_MS {
        return params.deadline_ms;
    }
    match params.style {
        AimStyleParam::Snap => SNAP_DURATION_MS,
        AimStyleParam::Flick => FLICK_DURATION_MS,
        AimStyleParam::Natural => NATURAL_DURATION_MS,
        AimStyleParam::Track => DEFAULT_DEADLINE_MS,
    }
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
        code = "M2_ACT_AIM_RECORDING_READBACK",
        kind = "act_aim",
        before_event_count,
        after_event_count = after_events.len(),
        new_event_count = new_events.len(),
        event_sequence,
        ?new_events,
        "source_of_truth=recording_backend tool=act_aim after_events_readback"
    );
    Ok(())
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
            "mouse_move:{}:{}:{duration_ms}",
            mouse_target_label(to),
            curve_label(curve)
        ),
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

fn track_unavailable() -> ErrorData {
    action_error_to_mcp(&ActionError::BackendUnavailable {
        detail: "act_aim track style requires the reflex runtime lands at M3".to_owned(),
    })
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

const fn default_aim_style() -> AimStyleParam {
    AimStyleParam::Snap
}

const fn default_deadline_ms() -> u32 {
    DEFAULT_DEADLINE_MS
}

const fn default_aim_backend() -> AimBackend {
    AimBackend::Auto
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
    use std::sync::Arc;

    use super::{
        ActAimParams, ActAimPointTarget, ActAimTarget, AimStyleParam, act_aim_with_handle,
        default_aim_backend, default_aim_style, default_deadline_ms, duration_ms, event_sequence,
    };
    use synapse_action::ActionEmitter;

    #[tokio::test]
    async fn recording_backend_readback_uses_natural_fast_snap_duration() {
        let (handle, _snapshot_handle, _emitter) = ActionEmitter::channel();
        let recording = Arc::new(synapse_action::RecordingBackend::new());
        let params = ActAimParams {
            target: ActAimTarget::Point(ActAimPointTarget { x: 200, y: 200 }),
            style: default_aim_style(),
            deadline_ms: default_deadline_ms(),
            backend: default_aim_backend(),
        };
        let before = recording.events();
        println!("source_of_truth=act_aim_recording edge=snap before={before:?}");

        let response = act_aim_with_handle(handle, Some(Arc::clone(&recording)), params)
            .await
            .unwrap_or_else(|error| panic!("act_aim recording should succeed: {error}"));
        let after = recording.events();
        let sequence = event_sequence(&after);
        println!(
            "source_of_truth=act_aim_recording edge=snap after={after:?} sequence={sequence} duration_ms={}",
            response.duration_ms
        );

        assert!(response.ok);
        assert_eq!(response.duration_ms, 50);
        assert_eq!(sequence, "mouse_move:screen(200,200):natural_fast:50");
    }

    #[test]
    fn defaults_are_issue_required_values() {
        assert_eq!(default_aim_style(), AimStyleParam::Snap);
        assert_eq!(default_deadline_ms(), 80);
        assert_eq!(default_aim_backend(), super::AimBackend::Auto);
    }

    #[test]
    fn style_default_durations_match_compile_table() {
        let target = ActAimTarget::Point(ActAimPointTarget { x: 1, y: 2 });
        for (style, expected) in [
            (AimStyleParam::Snap, 50),
            (AimStyleParam::Flick, 35),
            (AimStyleParam::Natural, 150),
        ] {
            let params = ActAimParams {
                target: target.clone(),
                style,
                deadline_ms: default_deadline_ms(),
                backend: default_aim_backend(),
            };
            println!(
                "source_of_truth=act_aim_compile edge=duration before=style:{style:?} after=duration_ms:{}",
                duration_ms(&params)
            );
            assert_eq!(duration_ms(&params), expected);
        }
    }
}
