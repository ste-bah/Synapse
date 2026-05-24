use enigo::{Button as EnigoButton, Direction, Enigo, Mouse};
use synapse_core::{AimCurve, AimStyle, AimTarget, ButtonAction, MouseButton, MouseTarget, Point};
use windows::Win32::{
    Foundation::POINT as WinPoint,
    UI::{
        Input::KeyboardAndMouse::{
            INPUT, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_MOVE,
            MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
        },
        WindowsAndMessaging::{
            GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
            SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
        },
    },
};

use super::{
    input::{mouse_input, send_input_batch},
    utils::{enigo, enigo_error, sleep_ms},
};
use crate::backend::mouse_coordinates::{VirtualDesktop, normalize_absolute_mouse_point};
use crate::{ActionError, EmitState, sample_curve};

const WHEEL_DELTA: i32 = 120;

pub(super) fn cursor_position() -> Result<Point, ActionError> {
    let mut point = WinPoint { x: 0, y: 0 };
    // SAFETY: `point` is a valid writable POINT for the duration of the call.
    unsafe { GetCursorPos(&raw mut point) }.map_err(|err| ActionError::BackendUnavailable {
        detail: format!("GetCursorPos failed: {err}"),
    })?;
    Ok(Point {
        x: point.x,
        y: point.y,
    })
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_move"))]
pub(super) fn mouse_move(target: &MouseTarget) -> Result<(), ActionError> {
    let MouseTarget::Screen { point } = target else {
        return Err(ActionError::TargetInvalid {
            detail: "software backend requires a resolved screen point for mouse movement"
                .to_owned(),
        });
    };
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
    let target = Point {
        x: current.x.saturating_add(rounded.0),
        y: current.y.saturating_add(rounded.1),
    };
    send_absolute_mouse_move(target, "relative absolute mouse move")
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_button"))]
pub(super) fn mouse_button(
    button: MouseButton,
    action: ButtonAction,
    hold_ms: u32,
    state: &mut EmitState,
) -> Result<(), ActionError> {
    let mut enigo = enigo()?;
    let enigo_button = enigo_button(button);
    match action {
        ButtonAction::Down => {
            state.apply_mouse_button(button, ButtonAction::Down);
            enigo
                .button(enigo_button, Direction::Press)
                .map_err(enigo_error("emit mouse button"))
        }
        ButtonAction::Up => {
            enigo
                .button(enigo_button, Direction::Release)
                .map_err(enigo_error("emit mouse button"))?;
            state.apply_mouse_button(button, ButtonAction::Up);
            Ok(())
        }
        ButtonAction::Press => {
            state.apply_mouse_button(button, ButtonAction::Down);
            enigo
                .button(enigo_button, Direction::Press)
                .map_err(enigo_error("emit mouse button"))?;
            sleep_ms(hold_ms);
            enigo
                .button(enigo_button, Direction::Release)
                .map_err(enigo_error("emit mouse button"))?;
            state.apply_mouse_button(button, ButtonAction::Up);
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
    mouse_move(&MouseTarget::Screen { point: *point })
}

pub(super) fn release_buttons_with(
    enigo: &mut Enigo,
    buttons: &[MouseButton],
) -> Result<(), ActionError> {
    for button in buttons.iter().rev() {
        enigo
            .button(enigo_button(*button), Direction::Release)
            .map_err(enigo_error("release held mouse button"))?;
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

const fn enigo_button(button: MouseButton) -> EnigoButton {
    match button {
        MouseButton::Left => EnigoButton::Left,
        MouseButton::Right => EnigoButton::Right,
        MouseButton::Middle => EnigoButton::Middle,
        MouseButton::X1 => EnigoButton::Back,
        MouseButton::X2 => EnigoButton::Forward,
    }
}

fn send_absolute_mouse_move(point: Point, detail: &'static str) -> Result<(), ActionError> {
    let input = absolute_mouse_input(point)?;
    send_input_batch(&[input], detail)
}

fn absolute_mouse_input(point: Point) -> Result<INPUT, ActionError> {
    let desktop = virtual_desktop()?;
    Ok(absolute_mouse_input_for_desktop(point, desktop))
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

fn virtual_desktop() -> Result<VirtualDesktop, ActionError> {
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
