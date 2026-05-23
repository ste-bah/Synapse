use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use rmcp::ErrorData;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_action::{
    ActionBackend, ActionError, ActionHandle, EmitState, RecordedInput, RecordingBackend,
};
use synapse_core::{Action, Backend, Point};

use crate::m1::mcp_error;

const SMOOTH_SCROLL_INTERVAL_MS: u32 = 30;
const MAX_SMOOTH_SCROLL_STEPS: u32 = 120;

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActScrollParams {
    #[serde(default)]
    #[schemars(default)]
    pub dy: i32,
    #[serde(default)]
    #[schemars(default)]
    pub dx: i32,
    pub at: Option<ActScrollPoint>,
    #[serde(default)]
    #[schemars(default)]
    pub smooth: bool,
}

#[derive(Copy, Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActScrollPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActScrollResponse {
    pub ok: bool,
    pub dy: i32,
    pub dx: i32,
    pub smooth: bool,
    pub scrolled: bool,
    pub wheel_event_count: u32,
    pub smooth_interval_ms: u32,
    pub scheduled_smooth_total_ms: u32,
    pub backend_used: String,
    pub elapsed_ms: u32,
}

pub async fn act_scroll_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActScrollParams,
) -> Result<ActScrollResponse, ErrorData> {
    validate_scroll_params(&params)?;
    let started = Instant::now();
    if params.dy == 0 && params.dx == 0 {
        if let Some(recording) = recording {
            execute_recording_noop(&recording, &params);
        }
        return Ok(response(&params, false, 0, "none", started));
    }

    let actions = scroll_actions(&params)?;
    let wheel_event_count = actions.len();

    if let Some(recording) = recording {
        execute_recording(&recording, &actions, &params).await?;
    } else {
        execute_scroll_actions(&handle, actions, params.smooth).await?;
    }

    Ok(response(
        &params,
        true,
        wheel_event_count,
        "software",
        started,
    ))
}

impl From<ActScrollPoint> for Point {
    fn from(value: ActScrollPoint) -> Self {
        Self {
            x: value.x,
            y: value.y,
        }
    }
}

fn validate_scroll_params(params: &ActScrollParams) -> Result<(), ErrorData> {
    if params.smooth {
        let step_count = smooth_step_count(params.dy, params.dx);
        if step_count > MAX_SMOOTH_SCROLL_STEPS {
            return Err(mcp_error(
                synapse_core::error_codes::TOOL_PARAMS_INVALID,
                format!(
                    "act_scroll smooth=true step count {step_count} exceeds max {MAX_SMOOTH_SCROLL_STEPS}"
                ),
            ));
        }
    }
    Ok(())
}

fn scroll_actions(params: &ActScrollParams) -> Result<Vec<Action>, ErrorData> {
    if !params.smooth {
        return Ok(vec![scroll_action(
            params.dy,
            params.dx,
            params.at.map(Into::into),
        )]);
    }
    let step_count = smooth_step_count(params.dy, params.dx);
    let capacity = usize::try_from(step_count).map_err(|_err| {
        mcp_error(
            synapse_core::error_codes::TOOL_PARAMS_INVALID,
            "act_scroll smooth=true step count cannot fit in memory",
        )
    })?;
    let mut actions = Vec::with_capacity(capacity);
    let mut vertical_ticks_remaining = params.dy;
    let mut horizontal_ticks_remaining = params.dx;
    for step_index in 0..step_count {
        let vertical_tick = take_tick(&mut vertical_ticks_remaining);
        let horizontal_tick = take_tick(&mut horizontal_ticks_remaining);
        actions.push(scroll_action(
            vertical_tick,
            horizontal_tick,
            if step_index == 0 {
                params.at.map(Into::into)
            } else {
                None
            },
        ));
    }
    Ok(actions)
}

const fn scroll_action(dy: i32, dx: i32, at: Option<Point>) -> Action {
    Action::MouseScroll {
        dy,
        dx,
        at,
        backend: Backend::Auto,
    }
}

fn smooth_step_count(dy: i32, dx: i32) -> u32 {
    dy.unsigned_abs().max(dx.unsigned_abs())
}

fn take_tick(value: &mut i32) -> i32 {
    match (*value).cmp(&0) {
        std::cmp::Ordering::Less => {
            *value += 1;
            -1
        }
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => {
            *value -= 1;
            1
        }
    }
}

async fn execute_scroll_actions(
    handle: &ActionHandle,
    actions: Vec<Action>,
    smooth: bool,
) -> Result<(), ErrorData> {
    let last_index = actions.len().saturating_sub(1);
    for (index, action) in actions.into_iter().enumerate() {
        handle
            .execute(action)
            .await
            .map_err(|error| action_error_to_mcp(&error))?;
        if smooth && index < last_index {
            tokio::time::sleep(Duration::from_millis(u64::from(SMOOTH_SCROLL_INTERVAL_MS))).await;
        }
    }
    Ok(())
}

fn response(
    params: &ActScrollParams,
    scrolled: bool,
    wheel_event_count: usize,
    backend_used: &'static str,
    started: Instant,
) -> ActScrollResponse {
    let wheel_event_count = u32::try_from(wheel_event_count).unwrap_or(u32::MAX);
    ActScrollResponse {
        ok: true,
        dy: params.dy,
        dx: params.dx,
        smooth: params.smooth,
        scrolled,
        wheel_event_count,
        smooth_interval_ms: if params.smooth {
            SMOOTH_SCROLL_INTERVAL_MS
        } else {
            0
        },
        scheduled_smooth_total_ms: scheduled_smooth_total_ms(params.smooth, wheel_event_count),
        backend_used: backend_used.to_owned(),
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    }
}

const fn scheduled_smooth_total_ms(smooth: bool, wheel_event_count: u32) -> u32 {
    if !smooth || wheel_event_count == 0 {
        return 0;
    }
    wheel_event_count
        .saturating_sub(1)
        .saturating_mul(SMOOTH_SCROLL_INTERVAL_MS)
}

async fn execute_recording(
    recording: &RecordingBackend,
    actions: &[Action],
    params: &ActScrollParams,
) -> Result<(), ErrorData> {
    let before_events = recording.events();
    let before_event_count = before_events.len();
    let mut emit_state = EmitState::new();
    let last_index = actions.len().saturating_sub(1);
    for (index, action) in actions.iter().enumerate() {
        recording
            .execute(action, &mut emit_state)
            .map_err(|error| action_error_to_mcp(&error))?;
        if params.smooth && index < last_index {
            tokio::time::sleep(Duration::from_millis(u64::from(SMOOTH_SCROLL_INTERVAL_MS))).await;
        }
    }
    let after_events = recording.events();
    let new_events = &after_events[before_event_count..];
    log_recording_readback(before_event_count, &after_events, new_events, params);
    Ok(())
}

fn execute_recording_noop(recording: &RecordingBackend, params: &ActScrollParams) {
    let before_events = recording.events();
    let before_event_count = before_events.len();
    let after_events = recording.events();
    let new_events = &after_events[before_event_count..];
    log_recording_readback(before_event_count, &after_events, new_events, params);
}

fn log_recording_readback(
    before_event_count: usize,
    after_events: &[RecordedInput],
    new_events: &[RecordedInput],
    params: &ActScrollParams,
) {
    let event_sequence = event_sequence(new_events);
    let smooth_step_count = if params.smooth {
        smooth_step_count(params.dy, params.dx)
    } else {
        0
    };
    tracing::info!(
        code = "M2_ACT_SCROLL_RECORDING_READBACK",
        kind = "act_scroll",
        before_event_count,
        after_event_count = after_events.len(),
        new_event_count = new_events.len(),
        dy = params.dy,
        dx = params.dx,
        smooth = params.smooth,
        smooth_step_count,
        smooth_interval_ms = if params.smooth {
            SMOOTH_SCROLL_INTERVAL_MS
        } else {
            0
        },
        scheduled_smooth_total_ms = scheduled_smooth_total_ms(params.smooth, smooth_step_count),
        event_sequence,
        ?new_events,
        "source_of_truth=recording_backend tool=act_scroll after_events_readback"
    );
}

fn event_sequence(events: &[RecordedInput]) -> String {
    events.iter().map(event_label).collect::<Vec<_>>().join(">")
}

fn event_label(event: &RecordedInput) -> String {
    match event {
        RecordedInput::MouseScroll { dy, dx, at } => {
            format!("mouse_scroll:dy={dy}:dx={dx}:at={}", at_label(*at))
        }
        other => format!("{other:?}"),
    }
}

fn at_label(at: Option<Point>) -> String {
    at.map_or_else(
        || "none".to_owned(),
        |point| format!("screen({},{})", point.x, point.y),
    )
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}
