use synapse_core::{ButtonAction, MouseButton};
use synapse_hid_host::{
    HOST_COMMAND_MOUSE_BUTTON, HOST_COMMAND_MOUSE_MOVE_REL, HOST_COMMAND_MOUSE_WHEEL,
};

use super::{HardwareGateway, send_empty_if_zero, sleep_ms};
use crate::{ActionError, EmitState};

pub(super) fn move_relative<G>(gateway: &mut G, dx: f32, dy: f32) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let dx = bounded_delta(dx, "dx")?;
    let dy = bounded_delta(dy, "dy")?;
    let payload = [
        dx.to_le_bytes()[0],
        dx.to_le_bytes()[1],
        dy.to_le_bytes()[0],
        dy.to_le_bytes()[1],
    ];
    if let Some((command, payload)) = send_empty_if_zero(HOST_COMMAND_MOUSE_MOVE_REL, &payload) {
        gateway.send_command(command, payload)?;
    }
    Ok(())
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
