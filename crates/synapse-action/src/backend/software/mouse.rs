use std::{fmt::Write as _, sync::Once};

use enigo::Enigo;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use synapse_core::{
    AimCurve, AimStyle, AimTarget, ButtonAction, HumanizeParams, MouseButton, MouseTarget,
    PathSpec, Point, StrokeMotionModel, StrokeTiming, VelocityProfile,
};
use windows::Win32::{
    Foundation::{E_ACCESSDENIED, POINT as WinPoint},
    Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint},
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
            SetProcessDpiAwarenessContext, SetThreadDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            INPUT, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
            MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
            MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
            MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP,
        },
        WindowsAndMessaging::{
            GetPhysicalCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
            SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SetPhysicalCursorPos,
        },
    },
};

use super::{
    input::{mouse_input, send_input_batch},
    utils::sleep_ms,
};
use crate::backend::mouse_coordinates::{VirtualDesktop, normalize_absolute_mouse_point};
use crate::{
    ActionError, EmitState, StrokeError, TimedPathPoint, plan_timed_stroke, recovery, sample_curve,
    screen_point_from_path_point,
};

const WHEEL_DELTA: i32 = 120;
const XBUTTON1_DATA: u32 = 0x0001;
const XBUTTON2_DATA: u32 = 0x0002;
const USER_DEFAULT_DPI: u32 = 96;
const CURSOR_READBACK_TOLERANCE_PX: i32 = 2;
static DPI_AWARENESS: Once = Once::new();

pub(super) fn cursor_position() -> Result<Point, ActionError> {
    activate_thread_dpi_awareness();
    let point = read_physical_cursor_position("cursor position")?;
    // PER_MONITOR_AWARE_V2: the physical cursor APIs and `observe`/a11y bboxes
    // share one physical-pixel coordinate space, so the readback passes through
    // unchanged. (Previously this divided by GetDpiForSystem/96, which
    // double-counted DPI on scaled displays and disagreed with observe.)
    Ok(Point {
        x: point.x,
        y: point.y,
    })
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_move"))]
pub(super) fn mouse_move(
    target: &MouseTarget,
    curve: &AimCurve,
    duration_ms: u32,
) -> Result<(), ActionError> {
    let MouseTarget::Screen { point } = target else {
        return Err(ActionError::TargetInvalid {
            detail: "software backend requires a resolved screen point for mouse movement"
                .to_owned(),
        });
    };
    if duration_ms > 0 && !matches!(curve, AimCurve::Instant) {
        let from = cursor_position()?;
        mouse_move_curve(from, *point, curve, duration_ms)?;
    }
    send_absolute_mouse_move(*point, "absolute mouse move")
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_move_relative"))]
pub(super) fn mouse_move_relative(dx: f32, dy: f32) -> Result<(), ActionError> {
    #[allow(clippy::cast_possible_truncation)]
    let rounded = (dx.round() as i32, dy.round() as i32);
    if rounded.0 == 0 && rounded.1 == 0 {
        return Ok(());
    }
    let current = cursor_position()?;
    send_absolute_mouse_move(
        relative_mouse_target(current, rounded),
        "relative mouse move",
    )
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_button"))]
pub(super) fn mouse_button(
    button: MouseButton,
    action: ButtonAction,
    hold_ms: u32,
    state: &mut EmitState,
) -> Result<(), ActionError> {
    match action {
        ButtonAction::Down => {
            recovery::record_held_button(button)?;
            send_mouse_button_event(button, ButtonAction::Down)?;
            state.apply_mouse_button(button, ButtonAction::Down);
            Ok(())
        }
        ButtonAction::Up => {
            send_mouse_button_event(button, ButtonAction::Up)?;
            state.apply_mouse_button(button, ButtonAction::Up);
            recovery::clear_held_button(button)?;
            Ok(())
        }
        ButtonAction::Press => {
            recovery::record_held_button(button)?;
            send_mouse_button_event(button, ButtonAction::Down)?;
            state.apply_mouse_button(button, ButtonAction::Down);
            let _interrupted = sleep_ms(hold_ms);
            send_mouse_button_event(button, ButtonAction::Up)?;
            state.apply_mouse_button(button, ButtonAction::Up);
            recovery::clear_held_button(button)?;
            Ok(())
        }
    }
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_drag"))]
pub(super) fn mouse_drag(
    from: Point,
    to: Point,
    button: MouseButton,
    curve: &AimCurve,
    duration_ms: u32,
    state: &mut EmitState,
) -> Result<(), ActionError> {
    send_absolute_mouse_move(from, "drag origin absolute mouse move")?;
    mouse_button(button, ButtonAction::Down, 0, state)?;
    mouse_move_curve(from, to, curve, duration_ms)?;
    mouse_button(button, ButtonAction::Up, 0, state)
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_stroke"))]
pub(super) fn mouse_stroke(
    path: &PathSpec,
    button: Option<MouseButton>,
    profile: VelocityProfile,
    timing: &StrokeTiming,
    motion_model: StrokeMotionModel,
    humanize: Option<HumanizeParams>,
    state: &mut EmitState,
) -> Result<(), ActionError> {
    let plan = plan_timed_stroke(path, profile, timing, motion_model, humanize)
        .map_err(|error| stroke_error(&error))?;
    let context =
        StrokeEmitLogContext::new(path, button, profile, timing, motion_model, humanize, &plan);
    let first = match plan.samples.first() {
        Some(first) => first,
        None => {
            let error = ActionError::TargetInvalid {
                detail: "mouse_stroke planner returned an empty point stream".to_owned(),
            };
            log_stroke_emit_error(&context, None, None, "plan_empty", &error);
            return Err(error);
        }
    };
    let first_point = match screen_point_from_path_point(first.point, 0) {
        Ok(point) => point,
        Err(error) => {
            let error = annotate_stroke_emit_error(stroke_error(&error), "origin_point", Some(0));
            log_stroke_emit_error(&context, Some(0), Some(first.point), "origin_point", &error);
            return Err(error);
        }
    };
    if let Err(error) =
        send_absolute_mouse_move(first_point, "mouse stroke origin absolute mouse move")
    {
        let error = annotate_stroke_emit_error(error, "origin_move", Some(0));
        log_stroke_emit_error(&context, Some(0), Some(first.point), "origin_move", &error);
        return Err(error);
    }

    if let Some(button) = button {
        if let Err(error) = mouse_button(button, ButtonAction::Down, 0, state) {
            let error = annotate_stroke_emit_error(error, "button_down", Some(0));
            log_stroke_emit_error(&context, Some(0), Some(first.point), "button_down", &error);
            return Err(error);
        }
    }

    let stream_result = emit_stroke_stream(&plan.samples, &context);
    if let Err(error) = stream_result {
        if let Some(button) = button
            && let Err(release_error) = mouse_button(button, ButtonAction::Up, 0, state)
        {
            tracing::error!(
                code = release_error.code(),
                detail = release_error.detail(),
                original_code = error.code(),
                original_detail = error.detail(),
                path_id = %context.path_id,
                path_kind = context.path_kind,
                backend = context.backend,
                point_stream_count = context.point_stream_count,
                duration_ms = context.duration_ms,
                path_length_px = context.path_length_px,
                button = ?context.button,
                action_kind = "software_mouse_stroke",
                "mouse_stroke failed and button cleanup also failed"
            );
            return Err(ActionError::BackendUnavailable {
                detail: format!(
                    "mouse_stroke failed with code={} detail={}; cleanup release failed with code={} detail={}",
                    error.code(),
                    error.detail(),
                    release_error.code(),
                    release_error.detail()
                ),
            });
        }
        return Err(error);
    }

    if let Some(button) = button {
        mouse_button(button, ButtonAction::Up, 0, state)?;
    }
    Ok(())
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_scroll"))]
pub(super) fn mouse_scroll(dy: i32, dx: i32, at: Option<Point>) -> Result<(), ActionError> {
    if let Some(point) = at {
        send_absolute_mouse_move(point, "scroll point absolute mouse move")?;
    }
    let mut inputs = Vec::with_capacity(2);
    if dy != 0 {
        inputs.push(mouse_input(
            0,
            0,
            signed_to_u32(dy.saturating_mul(WHEEL_DELTA)),
            MOUSEEVENTF_WHEEL,
        ));
    }
    if dx != 0 {
        inputs.push(mouse_input(
            0,
            0,
            signed_to_u32(dx.saturating_mul(WHEEL_DELTA)),
            MOUSEEVENTF_HWHEEL,
        ));
    }
    send_input_batch(&inputs, "mouse scroll")
}

#[tracing::instrument(skip_all, fields(action_kind = "software_aim_at"))]
pub(super) fn aim_at(target: &AimTarget, style: AimStyle) -> Result<(), ActionError> {
    if style == AimStyle::Track {
        return Err(ActionError::BackendUnavailable {
            detail: "track aim requires the M3 reflex runtime".to_owned(),
        });
    }
    let AimTarget::Screen { point } = target else {
        return Err(ActionError::TargetInvalid {
            detail: "software aim requires a resolved screen point".to_owned(),
        });
    };
    mouse_move(
        &MouseTarget::Screen { point: *point },
        &AimCurve::Instant,
        0,
    )
}

pub(super) fn release_buttons_with(
    _enigo: &mut Enigo,
    buttons: &[MouseButton],
) -> Result<(), ActionError> {
    for button in buttons.iter().rev() {
        send_mouse_button_event(*button, ButtonAction::Up)?;
    }
    Ok(())
}

fn mouse_move_curve(
    from: Point,
    to: Point,
    curve: &AimCurve,
    duration_ms: u32,
) -> Result<(), ActionError> {
    let samples = sample_curve(curve, from, to, duration_ms, None);
    let desktop = virtual_desktop()?;
    let mut inputs = Vec::with_capacity(samples.len().saturating_sub(1));
    for point in samples.into_iter().skip(1) {
        inputs.push(absolute_mouse_input_for_desktop(point, desktop));
    }
    send_input_batch(&inputs, "drag curve absolute mouse move")
}

fn emit_stroke_stream(
    samples: &[TimedPathPoint],
    context: &StrokeEmitLogContext,
) -> Result<(), ActionError> {
    let desktop = match virtual_desktop() {
        Ok(desktop) => desktop,
        Err(error) => {
            let error = annotate_stroke_emit_error(error, "virtual_desktop", None);
            log_stroke_emit_error(context, None, None, "virtual_desktop", &error);
            return Err(error);
        }
    };
    let mut previous_elapsed = samples.first().map_or(0.0, |sample| sample.elapsed_ms);
    for (index, sample) in samples.iter().enumerate().skip(1) {
        let delay_ms = match stroke_delay_ms(previous_elapsed, sample.elapsed_ms, index) {
            Ok(delay_ms) => delay_ms,
            Err(error) => {
                let error = annotate_stroke_emit_error(error, "delay", Some(index));
                log_stroke_emit_error(context, Some(index), Some(sample.point), "delay", &error);
                return Err(error);
            }
        };
        if sleep_ms(delay_ms) {
            let error = ActionError::SafetyOperatorHotkeyFired {
                detail: format!(
                    "operator release requested during mouse_stroke at sample_index={index}"
                ),
            };
            log_stroke_emit_error(
                context,
                Some(index),
                Some(sample.point),
                "operator_release",
                &error,
            );
            return Err(error);
        }
        previous_elapsed = sample.elapsed_ms;
        let point = match screen_point_from_path_point(sample.point, index) {
            Ok(point) => point,
            Err(error) => {
                let error =
                    annotate_stroke_emit_error(stroke_error(&error), "screen_point", Some(index));
                log_stroke_emit_error(
                    context,
                    Some(index),
                    Some(sample.point),
                    "screen_point",
                    &error,
                );
                return Err(error);
            }
        };
        let result = send_input_batch(
            &[absolute_mouse_input_for_desktop(point, desktop)],
            "mouse stroke absolute move",
        );
        if let Err(error) = result {
            let error = annotate_stroke_emit_error(error, "send_input", Some(index));
            log_stroke_emit_error(
                context,
                Some(index),
                Some(sample.point),
                "send_input",
                &error,
            );
            return Err(error);
        }
    }

    if let Some((index, final_sample)) = samples
        .len()
        .checked_sub(1)
        .map(|index| (index, samples[index]))
    {
        let requested = match screen_point_from_path_point(final_sample.point, index) {
            Ok(point) => point,
            Err(error) => {
                let error = annotate_stroke_emit_error(
                    stroke_error(&error),
                    "final_screen_point",
                    Some(index),
                );
                log_stroke_emit_error(
                    context,
                    Some(index),
                    Some(final_sample.point),
                    "final_screen_point",
                    &error,
                );
                return Err(error);
            }
        };
        let actual = match read_physical_cursor_position("mouse stroke final cursor readback") {
            Ok(actual) => actual,
            Err(error) => {
                let error = annotate_stroke_emit_error(error, "final_cursor_readback", Some(index));
                log_stroke_emit_error(
                    context,
                    Some(index),
                    Some(final_sample.point),
                    "final_cursor_readback",
                    &error,
                );
                return Err(error);
            }
        };
        if !cursor_readback_matches(requested, actual) {
            let error = ActionError::BackendUnavailable {
                detail: format!(
                    "mouse_stroke final_cursor_readback sample_index={index}: mouse stroke final cursor readback mismatch: requested={requested:?} actual=({},{}) tolerance_px={CURSOR_READBACK_TOLERANCE_PX}",
                    actual.x, actual.y
                ),
            };
            log_stroke_emit_error(
                context,
                Some(index),
                Some(final_sample.point),
                "final_cursor_readback_mismatch",
                &error,
            );
            return Err(error);
        }
    }
    Ok(())
}

fn stroke_delay_ms(
    previous_elapsed: f64,
    current_elapsed: f64,
    index: usize,
) -> Result<u32, ActionError> {
    if !previous_elapsed.is_finite()
        || !current_elapsed.is_finite()
        || current_elapsed + 1.0e-9 < previous_elapsed
    {
        return Err(ActionError::TargetInvalid {
            detail: format!(
                "mouse_stroke sample_index={index} has non-monotonic elapsed_ms previous={previous_elapsed} current={current_elapsed}"
            ),
        });
    }
    let delay = (current_elapsed - previous_elapsed).max(0.0);
    if delay > f64::from(u32::MAX) {
        return Err(ActionError::TargetInvalid {
            detail: format!("mouse_stroke sample_index={index} delay_ms={delay} exceeds u32 range"),
        });
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "validated finite u32-range delay is rounded to backend millisecond sleep"
    )]
    Ok(delay.round() as u32)
}

fn send_absolute_mouse_move(point: Point, detail: &'static str) -> Result<(), ActionError> {
    activate_thread_dpi_awareness();
    // Physical cursor APIs avoid DPI virtualization drift between the MCP
    // process and the operator-visible screen coordinate space. When this
    // succeeds, do not also send an absolute mouse-move packet: on mixed-DPI
    // desktops Windows can map MOUSEEVENTF_ABSOLUTE through a logical desktop
    // surface and move the cursor away from the physical UIA point.
    let compensation = dpi_compensation_for_point(point);
    if set_physical_cursor_pos(point, detail) {
        let first_actual = read_physical_cursor_position(detail)?;
        if cursor_readback_matches(point, first_actual) {
            return Ok(());
        }

        if let Some(compensation) = compensation {
            tracing::warn!(
                code = "M2_CURSOR_READBACK_DPI_COMPENSATION",
                requested_x = point.x,
                requested_y = point.y,
                first_actual_x = first_actual.x,
                first_actual_y = first_actual.y,
                adjusted_x = compensation.adjusted.x,
                adjusted_y = compensation.adjusted.y,
                dpi_x = compensation.dpi_x,
                dpi_y = compensation.dpi_y,
                detail,
                "physical cursor move read back a scaled coordinate; retrying with monitor-DPI compensation"
            );
            if set_physical_cursor_pos(compensation.adjusted, detail) {
                let compensated_actual = read_physical_cursor_position(detail)?;
                if cursor_readback_matches(point, compensated_actual) {
                    return Ok(());
                }
                tracing::warn!(
                    code = "M2_CURSOR_READBACK_DPI_COMPENSATION_MISMATCH",
                    requested_x = point.x,
                    requested_y = point.y,
                    first_actual_x = first_actual.x,
                    first_actual_y = first_actual.y,
                    adjusted_x = compensation.adjusted.x,
                    adjusted_y = compensation.adjusted.y,
                    compensated_actual_x = compensated_actual.x,
                    compensated_actual_y = compensated_actual.y,
                    dpi_x = compensation.dpi_x,
                    dpi_y = compensation.dpi_y,
                    detail,
                    "physical cursor move DPI compensation still mismatched; trying SendInput fallback"
                );
            }
        }

        tracing::warn!(
            code = "M2_CURSOR_READBACK_PHYSICAL_FALLBACK",
            requested_x = point.x,
            requested_y = point.y,
            first_actual_x = first_actual.x,
            first_actual_y = first_actual.y,
            detail,
            "physical cursor move readback mismatched; trying SendInput fallback"
        );
    }

    if let Some(compensation) = compensation
        && set_physical_cursor_pos(compensation.adjusted, detail)
    {
        let compensated_actual = read_physical_cursor_position(detail)?;
        if cursor_readback_matches(point, compensated_actual) {
            return Ok(());
        }
    }

    let desktop = virtual_desktop()?;
    send_input_batch(&[absolute_mouse_input_for_desktop(point, desktop)], detail)?;
    let send_input_actual = read_physical_cursor_position(detail)?;
    if cursor_readback_matches(point, send_input_actual) {
        Ok(())
    } else if let Some(compensation) = compensation {
        tracing::warn!(
            code = "M2_SEND_INPUT_CURSOR_DPI_COMPENSATION",
            requested_x = point.x,
            requested_y = point.y,
            first_actual_x = send_input_actual.x,
            first_actual_y = send_input_actual.y,
            adjusted_x = compensation.adjusted.x,
            adjusted_y = compensation.adjusted.y,
            dpi_x = compensation.dpi_x,
            dpi_y = compensation.dpi_y,
            detail,
            "SendInput cursor move read back a scaled coordinate; retrying with monitor-DPI compensation"
        );
        send_input_batch(
            &[absolute_mouse_input_for_desktop(
                compensation.adjusted,
                desktop,
            )],
            detail,
        )?;
        let compensated_actual = read_physical_cursor_position(detail)?;
        if cursor_readback_matches(point, compensated_actual) {
            Ok(())
        } else {
            Err(cursor_readback_mismatch_error(
                detail,
                point,
                send_input_actual,
                Some((compensation, compensated_actual)),
            ))
        }
    } else {
        Err(ActionError::BackendUnavailable {
            detail: format!(
                "{detail} cursor readback mismatch after SendInput fallback: requested={point:?} actual={send_input_actual:?} tolerance_px={CURSOR_READBACK_TOLERANCE_PX}"
            ),
        })
    }
}

fn stroke_error(error: &StrokeError) -> ActionError {
    ActionError::TargetInvalid {
        detail: format!("mouse_stroke planning failed: {error}"),
    }
}

#[derive(Clone, Debug)]
struct StrokeEmitLogContext {
    path_id: String,
    path_kind: &'static str,
    backend: &'static str,
    button: Option<MouseButton>,
    point_stream_count: usize,
    duration_ms: f64,
    path_length_px: f64,
}

impl StrokeEmitLogContext {
    fn new(
        path: &PathSpec,
        button: Option<MouseButton>,
        profile: VelocityProfile,
        timing: &StrokeTiming,
        motion_model: StrokeMotionModel,
        humanize: Option<HumanizeParams>,
        plan: &crate::StrokePlan,
    ) -> Self {
        Self {
            path_id: stroke_path_id(path, profile, timing, motion_model, humanize, plan),
            path_kind: path_kind(path),
            backend: "software",
            button,
            point_stream_count: plan.samples.len(),
            duration_ms: plan.duration_ms,
            path_length_px: plan.path_length_px,
        }
    }
}

fn stroke_path_id(
    path: &PathSpec,
    profile: VelocityProfile,
    timing: &StrokeTiming,
    motion_model: StrokeMotionModel,
    humanize: Option<HumanizeParams>,
    plan: &crate::StrokePlan,
) -> String {
    let payload = serde_json::to_vec(&json!({
        "path": path,
        "velocity_profile": profile,
        "duration_or_speed": timing,
        "motion_model": motion_model,
        "humanize": humanize,
        "plan": {
            "point_stream_count": plan.samples.len(),
            "duration_ms": plan.duration_ms,
            "path_length_px": plan.path_length_px,
        },
    }))
    .unwrap_or_else(|_error| {
        format!("{path:?}:{profile:?}:{timing:?}:{motion_model:?}:{humanize:?}").into_bytes()
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

fn annotate_stroke_emit_error(
    error: ActionError,
    stage: &'static str,
    sample_index: Option<usize>,
) -> ActionError {
    let index = sample_index
        .map(|index| format!(" sample_index={index}"))
        .unwrap_or_default();
    let detail = format!("mouse_stroke {stage}{index}: {}", error.detail());
    error.with_detail(detail)
}

fn log_stroke_emit_error(
    context: &StrokeEmitLogContext,
    sample_index: Option<usize>,
    requested_path_point: Option<synapse_core::PathPoint>,
    failure_stage: &'static str,
    error: &ActionError,
) {
    tracing::error!(
        code = error.code(),
        detail = error.detail(),
        sample_index,
        failure_stage,
        path_id = %context.path_id,
        path_kind = context.path_kind,
        backend = context.backend,
        point_stream_count = context.point_stream_count,
        duration_ms = context.duration_ms,
        path_length_px = context.path_length_px,
        button = ?context.button,
        requested_x = requested_path_point.map(|point| point.x),
        requested_y = requested_path_point.map(|point| point.y),
        queue_rate_state = "backend_dispatch",
        fallback_path_executed = false,
        action_kind = "software_mouse_stroke",
        "mouse_stroke emit failed without fallback"
    );
}

fn set_physical_cursor_pos(point: Point, detail: &'static str) -> bool {
    match unsafe { SetPhysicalCursorPos(point.x, point.y) } {
        Ok(()) => true,
        Err(error) if error.code() != windows::core::HRESULT(0) => {
            tracing::warn!(
                code = "M2_SET_PHYSICAL_CURSOR_POS_UNAVAILABLE",
                point_x = point.x,
                point_y = point.y,
                detail,
                error = %error,
                "SetPhysicalCursorPos failed; trying SendInput cursor move with readback"
            );
            false
        }
        Err(_error) => {
            tracing::warn!(
                code = "M2_SET_PHYSICAL_CURSOR_POS_FALSE_NO_ERROR",
                point_x = point.x,
                point_y = point.y,
                detail,
                "SetPhysicalCursorPos returned false without a Win32 error"
            );
            false
        }
    }
}

fn read_physical_cursor_position(detail: &'static str) -> Result<WinPoint, ActionError> {
    let mut point = WinPoint { x: 0, y: 0 };
    // SAFETY: `point` is a valid writable POINT for the duration of the call.
    unsafe { GetPhysicalCursorPos(&raw mut point) }.map_err(|err| {
        ActionError::BackendUnavailable {
            detail: format!("GetPhysicalCursorPos failed for {detail}: {err}"),
        }
    })?;
    Ok(point)
}

const fn cursor_readback_matches(requested: Point, actual: WinPoint) -> bool {
    requested.x.abs_diff(actual.x) <= CURSOR_READBACK_TOLERANCE_PX as u32
        && requested.y.abs_diff(actual.y) <= CURSOR_READBACK_TOLERANCE_PX as u32
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct DpiCompensation {
    adjusted: Point,
    dpi_x: u32,
    dpi_y: u32,
}

fn dpi_compensation_for_point(point: Point) -> Option<DpiCompensation> {
    let monitor = unsafe {
        MonitorFromPoint(
            WinPoint {
                x: point.x,
                y: point.y,
            },
            MONITOR_DEFAULTTONEAREST,
        )
    };
    if monitor.0.is_null() {
        return None;
    }

    let mut dpi_x = USER_DEFAULT_DPI;
    let mut dpi_y = USER_DEFAULT_DPI;
    if let Err(error) =
        unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y) }
    {
        tracing::warn!(
            code = "M2_CURSOR_DPI_READ_FAILED",
            point_x = point.x,
            point_y = point.y,
            error = %error,
            "failed to read monitor DPI for cursor compensation"
        );
        return None;
    }
    if dpi_x == 0 || dpi_y == 0 || (dpi_x == USER_DEFAULT_DPI && dpi_y == USER_DEFAULT_DPI) {
        return None;
    }

    let adjusted = scale_point_for_dpi(point, dpi_x, dpi_y);
    if adjusted == point {
        None
    } else {
        Some(DpiCompensation {
            adjusted,
            dpi_x,
            dpi_y,
        })
    }
}

fn cursor_readback_mismatch_error(
    detail: &'static str,
    requested: Point,
    first_actual: WinPoint,
    compensated: Option<(DpiCompensation, WinPoint)>,
) -> ActionError {
    let mut message = format!(
        "{detail} cursor readback mismatch: requested={requested:?} first_actual=({},{}) tolerance_px={CURSOR_READBACK_TOLERANCE_PX}",
        first_actual.x, first_actual.y
    );
    if let Some((compensation, compensated_actual)) = compensated {
        let _ = write!(
            message,
            " compensated_request={:?} dpi=({}, {}) compensated_actual=({}, {})",
            compensation.adjusted,
            compensation.dpi_x,
            compensation.dpi_y,
            compensated_actual.x,
            compensated_actual.y
        );
    }
    ActionError::BackendUnavailable { detail: message }
}

fn scale_point_for_dpi(point: Point, dpi_x: u32, dpi_y: u32) -> Point {
    Point {
        x: scale_coordinate_for_dpi(point.x, dpi_x),
        y: scale_coordinate_for_dpi(point.y, dpi_y),
    }
}

fn scale_coordinate_for_dpi(coord: i32, dpi: u32) -> i32 {
    let numerator = i128::from(coord) * i128::from(dpi);
    let denominator = i128::from(USER_DEFAULT_DPI);
    let rounded = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        (numerator - denominator / 2) / denominator
    };
    match i32::try_from(rounded) {
        Ok(value) => value,
        Err(_err) if rounded < 0 => i32::MIN,
        Err(_err) => i32::MAX,
    }
}

fn send_mouse_button_event(button: MouseButton, action: ButtonAction) -> Result<(), ActionError> {
    let (flags, data) = mouse_button_event_parts(button, action);
    send_input_batch(
        &[mouse_input(0, 0, data, flags)],
        match action {
            ButtonAction::Down => "mouse button down",
            ButtonAction::Up => "mouse button up",
            ButtonAction::Press => "mouse button press",
        },
    )
}

const fn mouse_button_event_parts(
    button: MouseButton,
    action: ButtonAction,
) -> (
    windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
    u32,
) {
    match (button, action) {
        (MouseButton::Left, ButtonAction::Down | ButtonAction::Press) => (MOUSEEVENTF_LEFTDOWN, 0),
        (MouseButton::Left, ButtonAction::Up) => (MOUSEEVENTF_LEFTUP, 0),
        (MouseButton::Right, ButtonAction::Down | ButtonAction::Press) => {
            (MOUSEEVENTF_RIGHTDOWN, 0)
        }
        (MouseButton::Right, ButtonAction::Up) => (MOUSEEVENTF_RIGHTUP, 0),
        (MouseButton::Middle, ButtonAction::Down | ButtonAction::Press) => {
            (MOUSEEVENTF_MIDDLEDOWN, 0)
        }
        (MouseButton::Middle, ButtonAction::Up) => (MOUSEEVENTF_MIDDLEUP, 0),
        (MouseButton::X1, ButtonAction::Down | ButtonAction::Press) => {
            (MOUSEEVENTF_XDOWN, XBUTTON1_DATA)
        }
        (MouseButton::X1, ButtonAction::Up) => (MOUSEEVENTF_XUP, XBUTTON1_DATA),
        (MouseButton::X2, ButtonAction::Down | ButtonAction::Press) => {
            (MOUSEEVENTF_XDOWN, XBUTTON2_DATA)
        }
        (MouseButton::X2, ButtonAction::Up) => (MOUSEEVENTF_XUP, XBUTTON2_DATA),
    }
}

fn absolute_mouse_input_for_desktop(point: Point, desktop: VirtualDesktop) -> INPUT {
    let normalized = normalize_absolute_mouse_point(point, desktop);
    mouse_input(
        normalized.dx,
        normalized.dy,
        0,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    )
}

const fn relative_mouse_target(current: Point, rounded: (i32, i32)) -> Point {
    Point {
        x: current.x.saturating_add(rounded.0),
        y: current.y.saturating_add(rounded.1),
    }
}

fn virtual_desktop() -> Result<VirtualDesktop, ActionError> {
    activate_thread_dpi_awareness();
    // SAFETY: GetSystemMetrics is read-only for these virtual-screen metrics.
    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    VirtualDesktop::new(left, top, width, height).ok_or_else(|| ActionError::BackendUnavailable {
        detail: format!(
            "invalid virtual desktop metrics left={left} top={top} width={width} height={height}"
        ),
    })
}

const fn signed_to_u32(value: i32) -> u32 {
    u32::from_ne_bytes(value.to_ne_bytes())
}

fn ensure_dpi_awareness() {
    DPI_AWARENESS.call_once(|| {
        match unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } {
            Ok(()) => {}
            Err(error) if error.code() == E_ACCESSDENIED => {}
            Err(error) => {
                tracing::warn!(
                    component = "software_mouse",
                    error = %error,
                    "failed to set process DPI awareness; cursor coordinates may be virtualized"
                );
            }
        }
    });
}

fn activate_thread_dpi_awareness() {
    ensure_dpi_awareness();
    let _previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_mouse_target_uses_current_cursor_plus_delta() {
        let target = relative_mouse_target(Point { x: 10, y: 20 }, (7, -3));

        assert_eq!(target, Point { x: 17, y: 17 });
    }

    #[test]
    #[allow(
        clippy::expect_used,
        reason = "unit test asserts on a known-valid desktop"
    )]
    fn absolute_mouse_input_uses_raw_physical_point_without_extra_scaling() {
        // Regression for the DPI double-scaling bug (#591): when the SendInput
        // absolute fallback is needed, it feeds the raw point into
        // `absolute_mouse_input_for_desktop` without an extra process-global
        // DPI multiply.
        let desktop =
            VirtualDesktop::new(0, 0, 5120, 2160).expect("non-degenerate virtual desktop");
        let point = Point { x: 1600, y: 1000 };

        let normalized = normalize_absolute_mouse_point(point, desktop);
        let origin = unsafe {
            absolute_mouse_input_for_desktop(point, desktop)
                .Anonymous
                .mi
        };
        let curve = unsafe {
            absolute_mouse_input_for_desktop(point, desktop)
                .Anonymous
                .mi
        };

        assert_eq!(origin.dx, normalized.dx);
        assert_eq!(origin.dy, normalized.dy);
        assert_eq!((origin.dx, origin.dy), (curve.dx, curve.dy));
    }

    #[test]
    fn dpi_compensation_scales_requested_cursor_point_to_monitor_dpi() {
        let requested = Point { x: 2905, y: 1165 };
        let adjusted = scale_point_for_dpi(requested, 144, 144);

        println!(
            "readback=mouse_dpi_compensation before=requested:{requested:?} dpi=(144,144) after={adjusted:?} expected=(4358,1748)"
        );
        assert_eq!(adjusted, Point { x: 4358, y: 1748 });
    }

    #[test]
    fn dpi_compensation_rounds_negative_coordinates_symmetrically() {
        let requested = Point { x: -101, y: 101 };
        let adjusted = scale_point_for_dpi(requested, 120, 144);

        println!(
            "readback=mouse_dpi_compensation edge=negative before=requested:{requested:?} dpi=(120,144) after={adjusted:?} expected=(-126,152)"
        );
        assert_eq!(adjusted, Point { x: -126, y: 152 });
    }

    #[test]
    fn cursor_readback_tolerance_accepts_small_os_jitter_only() {
        let requested = Point { x: 400, y: 500 };
        let within = WinPoint { x: 402, y: 498 };
        let outside = WinPoint { x: 403, y: 500 };

        println!(
            "readback=mouse_cursor_tolerance before=requested:{requested:?} within=({},{}) outside=({},{})",
            within.x, within.y, outside.x, outside.y
        );
        assert!(cursor_readback_matches(requested, within));
        assert!(!cursor_readback_matches(requested, outside));
    }

    #[test]
    fn stroke_emit_annotation_preserves_code_and_sample_index() {
        let error = ActionError::BackendUnavailable {
            detail: "SendInput returned 0".to_owned(),
        };

        let annotated = annotate_stroke_emit_error(error, "send_input", Some(7));

        assert_eq!(
            annotated.code(),
            synapse_core::error_codes::ACTION_BACKEND_UNAVAILABLE
        );
        assert!(annotated.detail().contains("mouse_stroke send_input"));
        assert!(annotated.detail().contains("sample_index=7"));
        assert!(annotated.detail().contains("SendInput returned 0"));
    }
}
