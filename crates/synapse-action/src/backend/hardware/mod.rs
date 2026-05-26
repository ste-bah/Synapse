use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use synapse_core::{Action, ComboInput, Key};
use synapse_hid_host::{
    HOST_COMMAND_KEY_DOWN, HOST_COMMAND_KEY_UP, HOST_COMMAND_RELEASE_ALL, HidError, HidGateway,
};

use crate::{ActionBackend, ActionError, EmitState};

mod keyboard;
mod mouse;
mod pad;

#[cfg(test)]
mod tests;

pub trait HardwareGateway: Send {
    /// Sends one firmware command and waits for the Pico ACK/NAK result.
    ///
    /// # Errors
    ///
    /// Returns an `ActionError` when the underlying HID link rejects or cannot
    /// deliver the command.
    fn send_command(&mut self, command: u8, payload: &[u8]) -> Result<u32, ActionError>;
}

impl HardwareGateway for HidGateway {
    #[allow(clippy::use_self)]
    fn send_command(&mut self, command: u8, payload: &[u8]) -> Result<u32, ActionError> {
        HidGateway::send_command(self, command, payload).map_err(action_error_from_hid)
    }
}

#[derive(Debug)]
pub struct HardwareBackend<G = HidGateway> {
    gateway: Mutex<G>,
}

impl HardwareBackend<HidGateway> {
    #[must_use]
    pub const fn new(gateway: HidGateway) -> Self {
        Self::with_gateway(gateway)
    }
}

impl<G> HardwareBackend<G>
where
    G: HardwareGateway,
{
    #[must_use]
    pub const fn with_gateway(gateway: G) -> Self {
        Self {
            gateway: Mutex::new(gateway),
        }
    }

    fn lock_gateway(&self) -> Result<MutexGuard<'_, G>, ActionError> {
        self.gateway
            .lock()
            .map_err(|_error| ActionError::BackendUnavailable {
                detail: "backend=hardware reason=gateway mutex poisoned".to_owned(),
            })
    }
}

impl<G> ActionBackend for HardwareBackend<G>
where
    G: HardwareGateway + Send,
{
    #[tracing::instrument(skip_all, fields(backend = "hardware"))]
    fn execute(&self, action: &Action, state: &mut EmitState) -> Result<(), ActionError> {
        crate::validate_action(action)?;
        let mut gateway = self.lock_gateway()?;
        execute_with_gateway(&mut *gateway, action, state)
    }
}

fn execute_with_gateway<G>(
    gateway: &mut G,
    action: &Action,
    state: &mut EmitState,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    match action {
        Action::KeyPress { key, hold_ms, .. } => key_press(gateway, key, *hold_ms, state),
        Action::KeyDown { key, .. } => key_down(gateway, key, state),
        Action::KeyUp { key, .. } => key_up(gateway, key, state),
        Action::KeyChord { keys, hold_ms, .. } => key_chord(gateway, keys, *hold_ms, state),
        Action::TypeText { .. } => Err(ActionError::UnsupportedKey {
            detail: "hardware text typing requires the HID usage keymap from issue #394"
                .to_owned(),
        }),
        Action::MouseMove { .. } | Action::MouseDrag { .. } | Action::AimAt { .. } => {
            Err(ActionError::TargetInvalid {
                detail:
                    "hardware absolute mouse targets require the relative-coordinate fallback from issue #396"
                        .to_owned(),
            })
        }
        Action::MouseMoveRelative { dx, dy, .. } => mouse::move_relative(gateway, *dx, *dy),
        Action::MouseButton {
            button,
            action,
            hold_ms,
            ..
        } => mouse::button(gateway, *button, *action, *hold_ms, state),
        Action::MouseScroll { dy, dx, .. } => mouse::scroll(gateway, *dy, *dx),
        Action::PadButton {
            pad,
            button,
            action,
            hold_ms,
        } => pad::button(gateway, state, *pad, *button, *action, *hold_ms),
        Action::PadStick { pad, stick, x, y } => {
            pad::stick(gateway, state, *pad, *stick, *x, *y)
        }
        Action::PadTrigger {
            pad,
            trigger,
            value,
        } => pad::trigger(gateway, state, *pad, *trigger, *value),
        Action::PadReport { pad, report } => pad::report(gateway, state, *pad, report.clone()),
        Action::Combo { steps, .. } => combo(gateway, steps, state),
        Action::ReleaseAll => release_all(gateway, state),
    }
}

fn key_press<G>(
    gateway: &mut G,
    key: &Key,
    hold_ms: u32,
    state: &mut EmitState,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    key_down(gateway, key, state)?;
    sleep_ms(hold_ms);
    key_up(gateway, key, state)
}

fn key_down<G>(gateway: &mut G, key: &Key, state: &mut EmitState) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let payload = [keyboard::hid_key_code(key)?];
    gateway.send_command(HOST_COMMAND_KEY_DOWN, &payload)?;
    state.hold_key(key);
    Ok(())
}

fn key_up<G>(gateway: &mut G, key: &Key, state: &mut EmitState) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let payload = [keyboard::hid_key_code(key)?];
    gateway.send_command(HOST_COMMAND_KEY_UP, &payload)?;
    state.release_key(key);
    Ok(())
}

fn key_chord<G>(
    gateway: &mut G,
    keys: &[Key],
    hold_ms: u32,
    state: &mut EmitState,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    for key in keys {
        key_down(gateway, key, state)?;
    }
    sleep_ms(hold_ms);
    for key in keys.iter().rev() {
        key_up(gateway, key, state)?;
    }
    Ok(())
}

fn combo<G>(
    gateway: &mut G,
    steps: &[synapse_core::ComboStep],
    state: &mut EmitState,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    for step in steps {
        match &step.input {
            ComboInput::KeyDown { key } => key_down(gateway, key, state)?,
            ComboInput::KeyUp { key } => key_up(gateway, key, state)?,
            ComboInput::KeyPress { key, hold_ms } => {
                key_press(gateway, key, u32::from(*hold_ms), state)?;
            }
            ComboInput::MouseButton { button, action } => {
                mouse::button(gateway, *button, *action, 0, state)?;
            }
            ComboInput::MouseMoveRel { dx, dy } => mouse::move_relative(gateway, *dx, *dy)?,
            ComboInput::PadButton {
                pad,
                button,
                action,
            } => pad::button(gateway, state, *pad, *button, *action, 0)?,
            ComboInput::PadStick { pad, stick, x, y } => {
                pad::stick(gateway, state, *pad, *stick, *x, *y)?;
            }
        }
    }
    Ok(())
}

fn release_all<G>(gateway: &mut G, state: &mut EmitState) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    gateway.send_command(HOST_COMMAND_RELEASE_ALL, &[])?;
    state.release_all();
    Ok(())
}

fn send_empty_if_zero(command: u8, payload: &[u8]) -> Option<(u8, &[u8])> {
    (!payload.iter().all(|byte| *byte == 0)).then_some((command, payload))
}

fn sleep_ms(ms: u32) {
    if ms > 0 {
        std::thread::sleep(Duration::from_millis(u64::from(ms)));
    }
}

fn action_error_from_hid(error: HidError) -> ActionError {
    match error {
        HidError::QueueFull {
            outstanding,
            capacity,
        } => ActionError::QueueFull {
            detail: format!("backend=hardware outstanding={outstanding} capacity={capacity}"),
        },
        HidError::PortNotFound { port_name } => ActionError::HidPortDisconnected {
            detail: format!("backend=hardware port={port_name} reason=port not found"),
        },
        HidError::PortOpenFailed {
            port_name, detail, ..
        } => ActionError::HidPortDisconnected {
            detail: format!("backend=hardware port={port_name} reason=port open failed: {detail}"),
        },
        HidError::PortDisconnected { detail } => ActionError::HidPortDisconnected {
            detail: format!("backend=hardware reason={detail}"),
        },
        HidError::LinkTimeout { operation, .. } => ActionError::HidPortDisconnected {
            detail: format!("backend=hardware reason={operation}"),
        },
        HidError::ProtocolHandshakeFailed { detail } => ActionError::BackendUnavailable {
            detail: format!("backend=hardware reason=protocol handshake failed: {detail}"),
        },
        HidError::FirmwareVersionMismatch { expected, actual } => ActionError::BackendUnavailable {
            detail: format!(
                "backend=hardware reason=firmware major mismatch expected={expected} actual={actual}"
            ),
        },
        HidError::CommandRejected {
            seq,
            command,
            reason,
        } => ActionError::BackendUnavailable {
            detail: format!(
                "backend=hardware seq={seq} command=0x{command:02X} reason=0x{reason:02X}"
            ),
        },
    }
}
