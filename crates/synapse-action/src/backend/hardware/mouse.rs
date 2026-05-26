use synapse_core::{
    AimCurve, AimNaturalParams, AimStyle, AimTarget, ButtonAction, MouseButton, MouseTarget, Point,
};
use synapse_hid_host::{
    HOST_COMMAND_MOUSE_BUTTON, HOST_COMMAND_MOUSE_MOVE_REL, HOST_COMMAND_MOUSE_WHEEL,
    HostCommandRequest,
};

use super::{HardwareGateway, send_empty_if_zero, sleep_ms};
use crate::{ActionError, EmitState, sample_curve};

pub(super) fn move_relative<G>(gateway: &mut G, dx: f32, dy: f32) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let dx = bounded_delta(dx, "dx")?;
    let dy = bounded_delta(dy, "dy")?;
    send_relative_deltas(gateway, &[RelativeMouseDelta { dx, dy }])
}

pub(super) fn move_absolute<G>(
    gateway: &mut G,
    target: &MouseTarget,
    curve: &AimCurve,
    duration_ms: u32,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let target = screen_mouse_target(target)?;
    let start = crate::backend::software::cursor_position()?;
    move_curve_from(gateway, start, target, curve, duration_ms)
}

pub(super) fn drag<G>(
    gateway: &mut G,
    current: Point,
    to: Point,
    mouse_button: MouseButton,
    curve: &AimCurve,
    duration_ms: u32,
    state: &mut EmitState,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let start = crate::backend::software::cursor_position()?;
    move_curve_from(gateway, start, current, &AimCurve::Instant, 0)?;
    button(gateway, mouse_button, ButtonAction::Down, 0, state)?;
    if let Err(error) = move_curve_from(gateway, current, to, curve, duration_ms) {
        let _ = button(gateway, mouse_button, ButtonAction::Up, 0, state);
        return Err(error);
    }
    button(gateway, mouse_button, ButtonAction::Up, 0, state)
}

pub(super) fn aim_at<G>(
    gateway: &mut G,
    target: &AimTarget,
    style: AimStyle,
    deadline_ms: u32,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    if style == AimStyle::Track {
        return Err(ActionError::BackendUnavailable {
            detail: "hardware track aim requires the M3 reflex runtime".to_owned(),
        });
    }
    let target = screen_aim_target(target)?;
    let start = crate::backend::software::cursor_position()?;
    let curve = AimCurve::Natural {
        params: AimNaturalParams::FAST,
    };
    move_curve_from(gateway, start, target, &curve, deadline_ms)
}

pub(super) fn move_curve_from<G>(
    gateway: &mut G,
    start: Point,
    target: Point,
    curve: &AimCurve,
    duration_ms: u32,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let samples = sample_curve(curve, start, target, duration_ms, None);
    let mut current = start;
    let mut deltas = Vec::with_capacity(samples.len().saturating_sub(1));
    for point in samples.into_iter().skip(1) {
        append_relative_deltas_to_point(&mut current, point, &mut deltas);
    }
    send_relative_deltas(gateway, &deltas)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RelativeMouseDelta {
    dx: i16,
    dy: i16,
}

fn send_relative_deltas<G>(
    gateway: &mut G,
    deltas: &[RelativeMouseDelta],
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let payloads = deltas
        .iter()
        .filter_map(|delta| relative_payload(*delta))
        .collect::<Vec<_>>();
    let commands = payloads
        .iter()
        .map(|payload| HostCommandRequest::new(HOST_COMMAND_MOUSE_MOVE_REL, payload))
        .collect::<Vec<_>>();
    if !commands.is_empty() {
        gateway.send_commands(&commands)?;
    }
    Ok(())
}

fn relative_payload(delta: RelativeMouseDelta) -> Option<[u8; 4]> {
    let payload = [
        delta.dx.to_le_bytes()[0],
        delta.dx.to_le_bytes()[1],
        delta.dy.to_le_bytes()[0],
        delta.dy.to_le_bytes()[1],
    ];
    payload.iter().any(|byte| *byte != 0).then_some(payload)
}

fn append_relative_deltas_to_point(
    current: &mut Point,
    target: Point,
    deltas: &mut Vec<RelativeMouseDelta>,
) {
    let mut remaining_x = i64::from(target.x) - i64::from(current.x);
    let mut remaining_y = i64::from(target.y) - i64::from(current.y);
    while remaining_x != 0 || remaining_y != 0 {
        let dx = clamp_relative_step(remaining_x);
        let dy = clamp_relative_step(remaining_y);
        deltas.push(RelativeMouseDelta { dx, dy });
        remaining_x -= i64::from(dx);
        remaining_y -= i64::from(dy);
        current.x = add_step_to_coord(current.x, dx);
        current.y = add_step_to_coord(current.y, dy);
    }
}

fn screen_mouse_target(target: &MouseTarget) -> Result<Point, ActionError> {
    match target {
        MouseTarget::Screen { point } => Ok(*point),
        MouseTarget::Element { element_id } => crate::invoke::element_screen_point(element_id),
    }
}

fn screen_aim_target(target: &AimTarget) -> Result<Point, ActionError> {
    match target {
        AimTarget::Screen { point } => Ok(*point),
        AimTarget::Element { element_id } => crate::invoke::element_screen_point(element_id),
        AimTarget::Track { track_id } => Err(ActionError::BackendUnavailable {
            detail: format!("hardware aim track target {track_id} requires the M3 reflex runtime"),
        }),
    }
}

fn clamp_relative_step(value: i64) -> i16 {
    let step = value.clamp(-127, 127);
    i16::try_from(step).unwrap_or_else(|_| unreachable!("step is clamped to i16 range"))
}

fn add_step_to_coord(coord: i32, step: i16) -> i32 {
    coord.saturating_add(i32::from(step))
}

pub(super) fn button<G>(
    gateway: &mut G,
    button: MouseButton,
    action: ButtonAction,
    hold_ms: u32,
    state: &mut EmitState,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    match action {
        ButtonAction::Down => button_state(gateway, button, true, state),
        ButtonAction::Up => button_state(gateway, button, false, state),
        ButtonAction::Press => {
            button_state(gateway, button, true, state)?;
            sleep_ms(hold_ms);
            button_state(gateway, button, false, state)
        }
    }
}

pub(super) fn scroll<G>(gateway: &mut G, dy: i32, dx: i32) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    if dx != 0 {
        return Err(ActionError::TargetInvalid {
            detail: "hardware firmware currently accepts only vertical wheel dx=0".to_owned(),
        });
    }
    let dy = i8::try_from(dy).map_err(|_error| ActionError::TargetInvalid {
        detail: format!("hardware wheel dy={dy} exceeds i8 range"),
    })?;
    let payload = [dy.cast_unsigned(), 0];
    if let Some((command, payload)) = send_empty_if_zero(HOST_COMMAND_MOUSE_WHEEL, &payload) {
        gateway.send_command(command, payload)?;
    }
    Ok(())
}

fn button_state<G>(
    gateway: &mut G,
    button: MouseButton,
    down: bool,
    state: &mut EmitState,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let payload = [firmware_button(button)?, u8::from(down)];
    gateway.send_command(HOST_COMMAND_MOUSE_BUTTON, &payload)?;
    state.apply_mouse_button(
        button,
        if down {
            ButtonAction::Down
        } else {
            ButtonAction::Up
        },
    );
    Ok(())
}

fn bounded_delta(value: f32, axis: &'static str) -> Result<i16, ActionError> {
    if !value.is_finite() {
        return Err(ActionError::TargetInvalid {
            detail: format!("hardware mouse {axis} must be finite"),
        });
    }
    #[allow(clippy::cast_possible_truncation)]
    let rounded = value.round() as i16;
    if (-127..=127).contains(&rounded) {
        Ok(rounded)
    } else {
        Err(ActionError::TargetInvalid {
            detail: format!(
                "hardware mouse {axis}={rounded} exceeds firmware delta range -127..127"
            ),
        })
    }
}

fn firmware_button(button: MouseButton) -> Result<u8, ActionError> {
    match button {
        MouseButton::Left => Ok(1),
        MouseButton::Right => Ok(2),
        MouseButton::Middle => Ok(3),
        MouseButton::X1 | MouseButton::X2 => Err(ActionError::TargetInvalid {
            detail: "hardware firmware supports only left, right, and middle mouse buttons"
                .to_owned(),
        }),
    }
}
