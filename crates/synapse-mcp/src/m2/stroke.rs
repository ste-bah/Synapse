use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use rmcp::ErrorData;
use rmcp::model::ErrorCode;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use synapse_action::{
    ActionBackend, ActionError, ActionHandle, ArcLengthPath, EmitState, PathError, RecordedInput,
    RecordingBackend, StrokeError, StrokePlan, plan_timed_stroke, screen_point_from_path_point,
};
use synapse_core::{
    Action, Backend, HumanizeParams, MouseButton, PathPoint, PathSpec, Point, Rect,
    StrokeMotionModel, StrokeTiming, VelocityProfile, error_codes,
};

use crate::m1::mcp_error;

pub const MAX_STROKE_PATH_POINTS: usize = 4096;
pub const MAX_STROKE_SAMPLES: usize = 60_001;
const MAX_STROKE_DURATION_MS: f64 = 60_000.0;
const MODIFIER_RELEASE_SETTLE_MS: u64 = 200;
const STROKE_DETAIL_COORD_NONFINITE: &str = "STROKE_COORD_NONFINITE";
const STROKE_DETAIL_COORD_OUT_OF_I32_RANGE: &str = "STROKE_COORD_OUT_OF_I32_RANGE";
const STROKE_DETAIL_POINT_OUT_OF_VIRTUAL_SCREEN: &str = "STROKE_POINT_OUT_OF_VIRTUAL_SCREEN";
const STROKE_DETAIL_PATH_DEGENERATE: &str = "STROKE_PATH_DEGENERATE";
const STROKE_DETAIL_PATH_POINT_CAP_EXCEEDED: &str = "STROKE_PATH_POINT_CAP_EXCEEDED";
const STROKE_DETAIL_SAMPLE_CAP_EXCEEDED: &str = "STROKE_SAMPLE_CAP_EXCEEDED";
const STROKE_DETAIL_DURATION_INVALID: &str = "STROKE_DURATION_INVALID";
const STROKE_DETAIL_DURATION_CAP_EXCEEDED: &str = "STROKE_DURATION_CAP_EXCEEDED";
const STROKE_DETAIL_SPEED_INVALID: &str = "STROKE_SPEED_INVALID";
const STROKE_DETAIL_PATH_PARAMETER_INVALID: &str = "STROKE_PATH_PARAMETER_INVALID";
const STROKE_DETAIL_VELOCITY_INVALID: &str = "STROKE_VELOCITY_INVALID";
const STROKE_DETAIL_HUMANIZE_INVALID: &str = "STROKE_HUMANIZE_INVALID";
const STROKE_DETAIL_MOTION_MODEL_INVALID: &str = "STROKE_MOTION_MODEL_INVALID";

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActStrokeParams {
    pub path: PathSpec,
    #[serde(default)]
    #[schemars(default)]
    pub button: Option<MouseButton>,
    #[serde(default = "default_stroke_velocity_profile")]
    #[schemars(default = "default_stroke_velocity_profile")]
    pub velocity_profile: VelocityProfile,
    pub duration_or_speed: StrokeTiming,
    #[serde(default = "default_stroke_motion_model")]
    #[schemars(default = "default_stroke_motion_model")]
    pub motion_model: StrokeMotionModel,
    #[serde(default)]
    #[schemars(default)]
    pub humanize: Option<HumanizeParams>,
    #[serde(default = "default_stroke_backend")]
    #[schemars(default = "default_stroke_backend")]
    pub backend: StrokeBackend,
    #[serde(default)]
    #[schemars(default)]
    pub modifiers: Vec<StrokeModifier>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StrokeBackend {
    Software,
    Hardware,
    Auto,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StrokeModifier {
    Ctrl,
    Shift,
    Alt,
    Super,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActStrokeResponse {
    pub ok: bool,
    pub path_kind: String,
    pub control_point_count: u32,
    pub button_used: Option<MouseButton>,
    pub velocity_profile_used: VelocityProfile,
    pub duration_or_speed_used: StrokeTiming,
    pub motion_model_used: StrokeMotionModel,
    pub humanized: bool,
    pub point_stream_count: u32,
    pub path_length_px: f64,
    pub duration_ms: f64,
    pub modifiers_used: Vec<StrokeModifier>,
    pub backend_used: String,
    pub elapsed_ms: u32,
}

pub async fn act_stroke_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActStrokeParams,
    plan: StrokePlan,
) -> Result<ActStrokeResponse, ErrorData> {
    let started = Instant::now();
    let backend = params.backend.to_backend();
    let action = Action::MouseStroke {
        path: params.path.clone(),
        button: params.button,
        profile: params.velocity_profile,
        timing: params.duration_or_speed.clone(),
        motion_model: params.motion_model,
        humanize: params.humanize,
        backend,
    };
    let modifier_keys: Vec<_> = params
        .modifiers
        .iter()
        .map(|modifier| modifier.to_key())
        .collect();

    if let Some(recording) = recording {
        execute_recording(&recording, &modifier_keys, &action, backend)?;
    } else {
        execute_with_modifiers(&handle, &modifier_keys, action, backend).await?;
    }

    Ok(response(&params, &plan, started, backend))
}

pub fn validate_act_stroke_params(params: &ActStrokeParams) -> Result<StrokePlan, ErrorData> {
    validate_and_plan(params)
}

pub fn act_stroke_request_details(params: &ActStrokeParams, plan: &StrokePlan) -> Value {
    json!({
        "path_id": act_stroke_path_id(params, plan),
        "path_kind": path_kind(&params.path),
        "control_point_count": control_point_count(&params.path),
        "button": params.button,
        "velocity_profile": params.velocity_profile,
        "duration_or_speed": &params.duration_or_speed,
        "motion_model": params.motion_model,
        "humanized": params.humanize.is_some(),
        "humanize": params.humanize,
        "backend_requested": params.backend,
        "backend_resolved": backend_used_name(params.backend.to_backend()),
        "modifiers": &params.modifiers,
        "plan": {
            "point_stream_count": plan.samples.len(),
            "path_length_px": plan.path_length_px,
            "duration_ms": plan.duration_ms,
            "first_sample": plan.samples.first().map(stroke_sample_details),
            "last_sample": plan.samples.last().map(stroke_sample_details),
        },
        "fallback_path_executed": false,
    })
}

pub fn act_stroke_error_details(error: &ErrorData) -> Value {
    let data = error.data.as_ref();
    json!({
        "code": data
            .and_then(|data| data.get("code"))
            .and_then(Value::as_str),
        "message": error.message.to_string(),
        "data": error.data.clone(),
        "point_index": data
            .and_then(|data| data.get("point_index"))
            .cloned()
            .unwrap_or(Value::Null),
        "queue_rate_state": data
            .and_then(|data| data.get("queue_rate_state"))
            .cloned()
            .unwrap_or_else(|| json!({ "kind": "not_rate_or_queue" })),
        "fallback_path_executed": false,
    })
}

impl StrokeBackend {
    const fn to_backend(self) -> Backend {
        match self {
            Self::Software => Backend::Software,
            Self::Hardware => Backend::Hardware,
            Self::Auto => Backend::Auto,
        }
    }
}

impl StrokeModifier {
    fn to_key(self) -> synapse_core::Key {
        let value = match self {
            Self::Ctrl => "ctrl",
            Self::Shift => "shift",
            Self::Alt => "alt",
            Self::Super => "super",
        };
        synapse_core::Key {
            code: synapse_core::KeyCode::Named {
                value: value.to_owned(),
            },
            use_scancode: false,
        }
    }
}

fn validate_and_plan(params: &ActStrokeParams) -> Result<StrokePlan, ErrorData> {
    validate_and_plan_with_screen_bounds(params, current_virtual_screen_bounds()?)
}

fn validate_and_plan_with_screen_bounds(
    params: &ActStrokeParams,
    screen_bounds: Option<StrokeScreenBounds>,
) -> Result<StrokePlan, ErrorData> {
    validate_control_point_cap(&params.path)?;
    validate_path_points(&params.path, screen_bounds)?;
    validate_duration_cap(params)?;
    let plan = plan_timed_stroke(
        &params.path,
        params.velocity_profile,
        &params.duration_or_speed,
        params.motion_model,
        params.humanize,
    )
    .map_err(|error| stroke_error_to_mcp(&error))?;
    if plan.samples.len() > MAX_STROKE_SAMPLES {
        return Err(params_invalid_detail(
            STROKE_DETAIL_SAMPLE_CAP_EXCEEDED,
            format!(
                "act_stroke planned point stream count {} exceeds max {MAX_STROKE_SAMPLES}",
                plan.samples.len()
            ),
        ));
    }
    validate_plan_points(&plan, screen_bounds)?;
    Ok(plan)
}

fn validate_control_point_cap(path: &PathSpec) -> Result<(), ErrorData> {
    let count = control_point_count(path);
    if count > MAX_STROKE_PATH_POINTS {
        return Err(params_invalid_detail(
            STROKE_DETAIL_PATH_POINT_CAP_EXCEEDED,
            format!(
                "act_stroke path control point count {count} exceeds max {MAX_STROKE_PATH_POINTS}"
            ),
        ));
    }
    Ok(())
}

fn validate_duration_cap(params: &ActStrokeParams) -> Result<(), ErrorData> {
    let path_length_px = ArcLengthPath::new(&params.path)
        .map_err(|error| path_error_to_mcp(&error))?
        .length();
    let duration_ms = match &params.duration_or_speed {
        StrokeTiming::DurationMs { duration_ms } => f64::from(*duration_ms),
        StrokeTiming::SpeedPxPerSec { px_per_sec } => {
            if !px_per_sec.is_finite() || *px_per_sec <= 0.0 {
                return Err(params_invalid_detail(
                    STROKE_DETAIL_SPEED_INVALID,
                    format!(
                        "act_stroke speed px_per_sec must be finite and greater than zero, got {px_per_sec}"
                    ),
                ));
            }
            path_length_px / px_per_sec * 1000.0
        }
    };
    if !duration_ms.is_finite() || duration_ms <= 0.0 {
        return Err(params_invalid_detail(
            STROKE_DETAIL_DURATION_INVALID,
            format!(
                "act_stroke duration_ms must be finite and greater than zero, got {duration_ms}"
            ),
        ));
    }
    if duration_ms > MAX_STROKE_DURATION_MS {
        return Err(params_invalid_detail(
            STROKE_DETAIL_DURATION_CAP_EXCEEDED,
            format!(
                "act_stroke planned duration_ms {duration_ms:.3} exceeds max {MAX_STROKE_DURATION_MS:.0}"
            ),
        ));
    }
    Ok(())
}

fn validate_path_points(
    path: &PathSpec,
    screen_bounds: Option<StrokeScreenBounds>,
) -> Result<(), ErrorData> {
    match path {
        PathSpec::Line { from, to } => {
            validate_path_point(*from, "path.line.from", screen_bounds)?;
            validate_path_point(*to, "path.line.to", screen_bounds)?;
        }
        PathSpec::Arc { center, .. } => {
            validate_path_point(*center, "path.arc.center", screen_bounds)?;
        }
        PathSpec::Circle { center, .. } => {
            validate_path_point(*center, "path.circle.center", screen_bounds)?;
        }
        PathSpec::CubicBezier { p0, p1, p2, p3 } => {
            validate_path_point(*p0, "path.cubic_bezier.p0", screen_bounds)?;
            validate_path_point(*p1, "path.cubic_bezier.p1", screen_bounds)?;
            validate_path_point(*p2, "path.cubic_bezier.p2", screen_bounds)?;
            validate_path_point(*p3, "path.cubic_bezier.p3", screen_bounds)?;
        }
        PathSpec::Polyline { points, .. } => {
            for (index, point) in points.iter().enumerate() {
                validate_path_point(
                    *point,
                    &format!("path.polyline.points[{index}]"),
                    screen_bounds,
                )?;
            }
        }
        PathSpec::CatmullRom { waypoints, .. } => {
            for (index, point) in waypoints.iter().enumerate() {
                validate_path_point(
                    *point,
                    &format!("path.catmull_rom.waypoints[{index}]"),
                    screen_bounds,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_plan_points(
    plan: &StrokePlan,
    screen_bounds: Option<StrokeScreenBounds>,
) -> Result<(), ErrorData> {
    for (index, sample) in plan.samples.iter().enumerate() {
        validate_path_point(
            sample.point,
            &format!("planned_samples[{index}]"),
            screen_bounds,
        )?;
    }
    Ok(())
}

fn validate_path_point(
    point: PathPoint,
    label: &str,
    screen_bounds: Option<StrokeScreenBounds>,
) -> Result<Point, ErrorData> {
    if !point.is_finite() {
        return Err(params_invalid_detail(
            STROKE_DETAIL_COORD_NONFINITE,
            format!(
                "act_stroke {label} must have finite coordinates, got x={} y={}",
                point.x, point.y
            ),
        ));
    }
    let screen_point = screen_point_from_path_point(point, 0).map_err(|error| match error {
        StrokeError::ScreenPointOutOfRange { x, y, .. } => params_invalid_detail(
            STROKE_DETAIL_COORD_OUT_OF_I32_RANGE,
            format!("act_stroke {label} is outside i32 screen coordinate range: x={x} y={y}"),
        ),
        other => stroke_error_to_mcp(&other),
    })?;
    if let Some(bounds) = screen_bounds
        && !bounds.rect.contains(screen_point)
    {
        let right = bounds.rect.x.saturating_add(bounds.rect.w);
        let bottom = bounds.rect.y.saturating_add(bounds.rect.h);
        return Err(params_invalid_detail(
            STROKE_DETAIL_POINT_OUT_OF_VIRTUAL_SCREEN,
            format!(
                "act_stroke {label} is outside the virtual screen bounds: point=({}, {}) bounds=left:{} top:{} right_exclusive:{} bottom_exclusive:{} source:{}",
                screen_point.x,
                screen_point.y,
                bounds.rect.x,
                bounds.rect.y,
                right,
                bottom,
                bounds.source
            ),
        ));
    }
    Ok(screen_point)
}

#[derive(Copy, Clone, Debug)]
struct StrokeScreenBounds {
    rect: Rect,
    source: &'static str,
}

fn current_virtual_screen_bounds() -> Result<Option<StrokeScreenBounds>, ErrorData> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN,
        };

        // SAFETY: GetSystemMetrics is read-only for these process desktop metrics.
        let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        // SAFETY: GetSystemMetrics is read-only for these process desktop metrics.
        let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        // SAFETY: GetSystemMetrics is read-only for these process desktop metrics.
        let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        // SAFETY: GetSystemMetrics is read-only for these process desktop metrics.
        let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
        if width <= 0 || height <= 0 {
            return Err(mcp_error(
                error_codes::ACTION_BACKEND_UNAVAILABLE,
                format!(
                    "act_stroke could not read a valid virtual screen before validation: left={left} top={top} width={width} height={height}"
                ),
            ));
        }
        Ok(Some(StrokeScreenBounds {
            rect: Rect {
                x: left,
                y: top,
                w: width,
                h: height,
            },
            source: "GetSystemMetrics(SM_*VIRTUALSCREEN)",
        }))
    }

    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

async fn execute_with_modifiers(
    handle: &ActionHandle,
    modifier_keys: &[synapse_core::Key],
    stroke_action: Action,
    backend: Backend,
) -> Result<(), ErrorData> {
    let mut pressed = Vec::with_capacity(modifier_keys.len());
    for key in modifier_keys {
        if let Err(error) = handle
            .execute(Action::KeyDown {
                key: key.clone(),
                backend,
            })
            .await
        {
            let _ = release_pressed_modifiers(handle, &pressed, backend).await;
            return Err(action_error_to_mcp(&error));
        }
        pressed.push(key.clone());
    }

    let stroke_result = handle.execute(stroke_action).await;
    if stroke_result.is_ok() && !pressed.is_empty() {
        tokio::time::sleep(Duration::from_millis(MODIFIER_RELEASE_SETTLE_MS)).await;
    }
    let release_result = release_pressed_modifiers(handle, &pressed, backend).await;

    if let Err(error) = stroke_result {
        return Err(action_error_to_mcp(&error));
    }
    if let Err(error) = release_result {
        return Err(action_error_to_mcp(&error));
    }
    Ok(())
}

async fn release_pressed_modifiers(
    handle: &ActionHandle,
    pressed: &[synapse_core::Key],
    backend: Backend,
) -> Result<(), ActionError> {
    let mut release_error = None;
    for key in pressed.iter().rev() {
        if let Err(error) = handle
            .execute(Action::KeyUp {
                key: key.clone(),
                backend,
            })
            .await
            && release_error.is_none()
        {
            release_error = Some(error);
        }
    }
    release_error.map_or(Ok(()), Err)
}

fn execute_recording(
    recording: &RecordingBackend,
    modifier_keys: &[synapse_core::Key],
    stroke_action: &Action,
    backend: Backend,
) -> Result<(), ErrorData> {
    let before_events = recording.events();
    let before_event_count = before_events.len();
    let mut emit_state = EmitState::new();
    for key in modifier_keys {
        recording
            .execute(
                &Action::KeyDown {
                    key: key.clone(),
                    backend,
                },
                &mut emit_state,
            )
            .map_err(|error| action_error_to_mcp(&error))?;
    }
    recording
        .execute(stroke_action, &mut emit_state)
        .map_err(|error| action_error_to_mcp(&error))?;
    for key in modifier_keys.iter().rev() {
        recording
            .execute(
                &Action::KeyUp {
                    key: key.clone(),
                    backend,
                },
                &mut emit_state,
            )
            .map_err(|error| action_error_to_mcp(&error))?;
    }
    let after_events = recording.events();
    let new_events = &after_events[before_event_count..];
    let event_sequence = event_sequence(new_events);
    tracing::info!(
        code = "M2_ACT_STROKE_RECORDING_READBACK",
        kind = "act_stroke",
        before_event_count,
        after_event_count = after_events.len(),
        new_event_count = new_events.len(),
        event_sequence,
        ?new_events,
        "readback=recording_backend tool=act_stroke after_events_readback"
    );
    Ok(())
}

fn response(
    params: &ActStrokeParams,
    plan: &StrokePlan,
    started: Instant,
    backend: Backend,
) -> ActStrokeResponse {
    ActStrokeResponse {
        ok: true,
        path_kind: path_kind(&params.path).to_owned(),
        control_point_count: u32::try_from(control_point_count(&params.path)).unwrap_or(u32::MAX),
        button_used: params.button,
        velocity_profile_used: params.velocity_profile,
        duration_or_speed_used: params.duration_or_speed.clone(),
        motion_model_used: params.motion_model,
        humanized: params.humanize.is_some(),
        point_stream_count: u32::try_from(plan.samples.len()).unwrap_or(u32::MAX),
        path_length_px: plan.path_length_px,
        duration_ms: plan.duration_ms,
        modifiers_used: params.modifiers.clone(),
        backend_used: backend_used_name(backend).to_owned(),
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    }
}

fn event_sequence(events: &[RecordedInput]) -> String {
    events.iter().map(event_label).collect::<Vec<_>>().join(">")
}

fn event_label(event: &RecordedInput) -> String {
    match event {
        RecordedInput::KeyDown { key } => format!("key_down:{}", key_label(key)),
        RecordedInput::KeyUp { key } => format!("key_up:{}", key_label(key)),
        RecordedInput::MouseButtonDown { button } => format!("down:{}", button_label(*button)),
        RecordedInput::MouseButtonUp { button } => format!("up:{}", button_label(*button)),
        RecordedInput::MouseStrokePoint { elapsed_ms, point } => {
            format!(
                "stroke_point:{elapsed_ms:.3}:screen({},{})",
                point.x, point.y
            )
        }
        other => format!("{other:?}"),
    }
}

fn key_label(key: &synapse_core::Key) -> String {
    match &key.code {
        synapse_core::KeyCode::Named { value } => value.clone(),
        synapse_core::KeyCode::Symbol { value } => value.to_string(),
        synapse_core::KeyCode::HidCode { value } => format!("hid:{value}"),
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

fn control_point_count(path: &PathSpec) -> usize {
    match path {
        PathSpec::Line { .. } => 2,
        PathSpec::Arc { .. } | PathSpec::Circle { .. } => 1,
        PathSpec::CubicBezier { .. } => 4,
        PathSpec::Polyline { points, .. } => points.len(),
        PathSpec::CatmullRom { waypoints, .. } => waypoints.len(),
    }
}

fn path_kind(path: &PathSpec) -> &'static str {
    match path {
        PathSpec::Line { .. } => "line",
        PathSpec::Arc { .. } => "arc",
        PathSpec::Circle { .. } => "circle",
        PathSpec::CubicBezier { .. } => "cubic_bezier",
        PathSpec::Polyline { .. } => "polyline",
        PathSpec::CatmullRom { .. } => "catmull_rom",
    }
}

fn stroke_error_to_mcp(error: &StrokeError) -> ErrorData {
    match error {
        StrokeError::Path(error) => path_error_to_mcp(error),
        StrokeError::Velocity(error) => params_invalid_detail(
            STROKE_DETAIL_VELOCITY_INVALID,
            format!("act_stroke velocity profile invalid: {error}"),
        ),
        StrokeError::Humanize(error) => params_invalid_detail(
            STROKE_DETAIL_HUMANIZE_INVALID,
            format!("act_stroke humanize params invalid: {error}"),
        ),
        StrokeError::InvalidDuration { duration_ms } => params_invalid_detail(
            STROKE_DETAIL_DURATION_INVALID,
            format!(
                "act_stroke duration_ms must be finite and greater than zero, got {duration_ms}"
            ),
        ),
        StrokeError::InvalidSpeed { px_per_sec } => params_invalid_detail(
            STROKE_DETAIL_SPEED_INVALID,
            format!(
                "act_stroke speed px_per_sec must be finite and greater than zero, got {px_per_sec}"
            ),
        ),
        StrokeError::SampleCountOverflow { duration_ms } => params_invalid_detail(
            STROKE_DETAIL_SAMPLE_CAP_EXCEEDED,
            format!("act_stroke sample count overflow for duration_ms={duration_ms}"),
        ),
        StrokeError::ScreenPointOutOfRange { index, x, y } => params_invalid_detail(
            STROKE_DETAIL_COORD_OUT_OF_I32_RANGE,
            format!(
                "act_stroke planned point {index} is outside i32 screen coordinate range: x={x} y={y}"
            ),
        ),
        StrokeError::WindMouseRequiresLine { path_kind } => params_invalid_detail(
            STROKE_DETAIL_MOTION_MODEL_INVALID,
            format!("act_stroke motion_model wind_mouse requires path.kind=line, got {path_kind}"),
        ),
        StrokeError::InvalidWindMouseParameter { field, value } => params_invalid_detail(
            STROKE_DETAIL_MOTION_MODEL_INVALID,
            format!(
                "act_stroke motion_model wind_mouse parameter {field} must be finite and greater than zero, got {value}"
            ),
        ),
        StrokeError::WindMouseNonFinitePoint { index, x, y } => params_invalid_detail(
            STROKE_DETAIL_MOTION_MODEL_INVALID,
            format!(
                "act_stroke motion_model wind_mouse generated non-finite point {index}: x={x} y={y}"
            ),
        ),
        StrokeError::WindMouseDidNotConverge {
            max_points,
            remaining_distance_px,
        } => params_invalid_detail(
            STROKE_DETAIL_SAMPLE_CAP_EXCEEDED,
            format!(
                "act_stroke motion_model wind_mouse did not converge within {max_points} points; remaining distance {remaining_distance_px:.3}px"
            ),
        ),
    }
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        error.to_string(),
        Some(json!({
            "code": error.code(),
            "detail": error.detail(),
            "retry_after_ms": error.retry_after_ms(),
            "point_index": extract_sample_index(error.detail()),
            "queue_rate_state": queue_rate_state(error),
        })),
    )
}

fn path_error_to_mcp(error: &PathError) -> ErrorData {
    let detail_code = match error {
        PathError::NotEnoughPoints { .. }
        | PathError::DegenerateSegment { .. }
        | PathError::DegenerateCurve { .. }
        | PathError::ZeroLengthPath
        | PathError::InvalidSampleCount { .. } => STROKE_DETAIL_PATH_DEGENERATE,
        PathError::NonFinitePoint { .. } | PathError::InvalidT { .. } => {
            STROKE_DETAIL_COORD_NONFINITE
        }
        PathError::NonFiniteParameter { .. }
        | PathError::NonPositiveParameter { .. }
        | PathError::InvalidCatmullRomAlpha { .. }
        | PathError::InvalidCatmullRomTension { .. }
        | PathError::InvalidArcLengthSegments { .. }
        | PathError::InvalidArcLength { .. } => STROKE_DETAIL_PATH_PARAMETER_INVALID,
    };
    params_invalid_detail(detail_code, format!("act_stroke path invalid: {error}"))
}

fn params_invalid_detail(detail_code: &'static str, message: impl Into<String>) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        message.into(),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "detail_code": detail_code,
        })),
    )
}

const fn default_stroke_velocity_profile() -> VelocityProfile {
    VelocityProfile::Constant
}

const fn default_stroke_motion_model() -> StrokeMotionModel {
    StrokeMotionModel::Path
}

const fn default_stroke_backend() -> StrokeBackend {
    StrokeBackend::Auto
}

const fn backend_used_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Auto | Backend::Software => "software",
        Backend::Hardware => "hardware",
        Backend::Vigem => "vigem",
    }
}

fn act_stroke_path_id(params: &ActStrokeParams, plan: &StrokePlan) -> String {
    let payload = serde_json::to_vec(&json!({
        "path": &params.path,
        "velocity_profile": params.velocity_profile,
        "duration_or_speed": &params.duration_or_speed,
        "humanize": params.humanize,
        "plan": {
            "point_stream_count": plan.samples.len(),
            "duration_ms": plan.duration_ms,
            "path_length_px": plan.path_length_px,
        },
    }))
    .unwrap_or_else(|_error| {
        format!(
            "{:?}:{:?}:{:?}:{:?}",
            params.path, params.velocity_profile, params.duration_or_speed, params.humanize
        )
        .into_bytes()
    });
    format!("stroke:{}", sha256_hex(payload))
}

fn sha256_hex(payload: Vec<u8>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(payload);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn stroke_sample_details(sample: &synapse_action::TimedPathPoint) -> Value {
    json!({
        "elapsed_ms": sample.elapsed_ms,
        "arclen": sample.arclen,
        "point": {
            "x": sample.point.x,
            "y": sample.point.y,
        },
    })
}

fn queue_rate_state(error: &ActionError) -> Value {
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

fn extract_sample_index(detail: &str) -> Option<usize> {
    let marker = "sample_index=";
    let start = detail.find(marker)? + marker.len();
    let digits = detail[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    (!digits.is_empty())
        .then(|| digits.parse::<usize>().ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stroke_validation_accepts_valid_line_inside_bounds() {
        let params = line_params(
            PathPoint::new(1.0, 1.0),
            PathPoint::new(5.0, 1.0),
            StrokeTiming::DurationMs { duration_ms: 4 },
        );

        let plan = match validate_and_plan_with_screen_bounds(&params, test_bounds()) {
            Ok(plan) => plan,
            Err(error) => panic!("valid stroke rejected: {error:?}"),
        };

        assert_eq!(plan.samples.len(), 5);
        assert_eq!(plan.path_length_px, 4.0);
        assert_eq!(plan.duration_ms, 4.0);
    }

    #[test]
    fn stroke_validation_rejects_nonfinite_coordinate() {
        let params = line_params(
            PathPoint::new(f64::NAN, 1.0),
            PathPoint::new(5.0, 1.0),
            StrokeTiming::DurationMs { duration_ms: 4 },
        );

        let error = validation_error(&params);

        assert_tool_params_invalid_detail(&error, STROKE_DETAIL_COORD_NONFINITE);
        assert!(error.message.contains("finite coordinates"));
    }

    #[test]
    fn stroke_validation_rejects_one_point_path() {
        let params = params_for_path(PathSpec::Polyline {
            points: vec![PathPoint::new(1.0, 1.0)],
            closed: false,
        });

        let error = validation_error(&params);

        assert_tool_params_invalid_detail(&error, STROKE_DETAIL_PATH_DEGENERATE);
        assert!(error.message.contains("requires at least 2 points"));
    }

    #[test]
    fn stroke_validation_rejects_waypoint_count_over_cap() {
        let points = vec![PathPoint::new(1.0, 1.0); MAX_STROKE_PATH_POINTS + 1];
        let params = params_for_path(PathSpec::Polyline {
            points,
            closed: false,
        });

        let error = validation_error(&params);

        assert_tool_params_invalid_detail(&error, STROKE_DETAIL_PATH_POINT_CAP_EXCEEDED);
        assert!(error.message.contains("control point count"));
    }

    #[test]
    fn stroke_validation_rejects_out_of_virtual_screen_bounds() {
        let params = line_params(
            PathPoint::new(1.0, 1.0),
            PathPoint::new(200.0, 1.0),
            StrokeTiming::DurationMs { duration_ms: 4 },
        );

        let error = validation_error(&params);

        assert_tool_params_invalid_detail(&error, STROKE_DETAIL_POINT_OUT_OF_VIRTUAL_SCREEN);
        assert!(error.message.contains("virtual screen bounds"));
    }

    #[test]
    fn stroke_validation_rejects_zero_duration() {
        let params = line_params(
            PathPoint::new(1.0, 1.0),
            PathPoint::new(5.0, 1.0),
            StrokeTiming::DurationMs { duration_ms: 0 },
        );

        let error = validation_error(&params);

        assert_tool_params_invalid_detail(&error, STROKE_DETAIL_DURATION_INVALID);
        assert!(error.message.contains("greater than zero"));
    }

    #[test]
    fn stroke_validation_rejects_infinite_speed() {
        let params = line_params(
            PathPoint::new(1.0, 1.0),
            PathPoint::new(5.0, 1.0),
            StrokeTiming::SpeedPxPerSec {
                px_per_sec: f64::INFINITY,
            },
        );

        let error = validation_error(&params);

        assert_tool_params_invalid_detail(&error, STROKE_DETAIL_SPEED_INVALID);
        assert!(error.message.contains("px_per_sec"));
    }

    #[test]
    fn stroke_request_details_include_stable_path_context() {
        let params = line_params(
            PathPoint::new(1.0, 1.0),
            PathPoint::new(5.0, 1.0),
            StrokeTiming::DurationMs { duration_ms: 4 },
        );
        let plan = match validate_and_plan_with_screen_bounds(&params, test_bounds()) {
            Ok(plan) => plan,
            Err(error) => panic!("valid stroke should plan: {error:?}"),
        };

        let details = act_stroke_request_details(&params, &plan);

        let path_id = details
            .get("path_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(path_id.starts_with("stroke:"));
        assert_eq!(path_id.len(), "stroke:".len() + 64);
        assert_eq!(
            details.get("path_kind").and_then(serde_json::Value::as_str),
            Some("line")
        );
        assert_eq!(
            details
                .pointer("/plan/point_stream_count")
                .and_then(serde_json::Value::as_u64),
            Some(5)
        );
        assert_eq!(
            details
                .get("fallback_path_executed")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn stroke_action_error_data_preserves_rate_state_and_sample_index() {
        let error = ActionError::RateLimited {
            detail: "mouse_stroke send_input sample_index=7: backend=software retry_after_ms=25 requested_tokens=1 available_tokens=0 refill_rate_per_s=10".to_owned(),
            retry_after_ms: 25,
        };

        let mcp_error = action_error_to_mcp(&error);
        let data = mcp_error
            .data
            .as_ref()
            .expect("action error data should be present");

        assert_eq!(
            data.get("code").and_then(serde_json::Value::as_str),
            Some(error_codes::ACTION_RATE_LIMITED)
        );
        assert_eq!(
            data.get("point_index").and_then(serde_json::Value::as_u64),
            Some(7)
        );
        assert_eq!(
            data.pointer("/queue_rate_state/kind")
                .and_then(serde_json::Value::as_str),
            Some("rate_limited")
        );
        assert_eq!(
            data.pointer("/queue_rate_state/retry_after_ms")
                .and_then(serde_json::Value::as_u64),
            Some(25)
        );
    }

    fn validation_error(params: &ActStrokeParams) -> ErrorData {
        match validate_and_plan_with_screen_bounds(params, test_bounds()) {
            Ok(plan) => panic!("invalid stroke was accepted with plan: {plan:?}"),
            Err(error) => error,
        }
    }

    fn line_params(from: PathPoint, to: PathPoint, timing: StrokeTiming) -> ActStrokeParams {
        params_for_path_with_timing(PathSpec::Line { from, to }, timing)
    }

    fn params_for_path(path: PathSpec) -> ActStrokeParams {
        params_for_path_with_timing(path, StrokeTiming::DurationMs { duration_ms: 4 })
    }

    fn params_for_path_with_timing(path: PathSpec, timing: StrokeTiming) -> ActStrokeParams {
        ActStrokeParams {
            path,
            button: None,
            velocity_profile: VelocityProfile::Constant,
            duration_or_speed: timing,
            motion_model: StrokeMotionModel::Path,
            humanize: None,
            backend: StrokeBackend::Software,
            modifiers: Vec::new(),
        }
    }

    fn test_bounds() -> Option<StrokeScreenBounds> {
        Some(StrokeScreenBounds {
            rect: Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 100,
            },
            source: "test",
        })
    }

    fn assert_tool_params_invalid_detail(error: &ErrorData, expected_detail_code: &str) {
        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("code"))
                .and_then(|code| code.as_str()),
            Some(error_codes::TOOL_PARAMS_INVALID)
        );
        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("detail_code"))
                .and_then(|code| code.as_str()),
            Some(expected_detail_code)
        );
    }
}
