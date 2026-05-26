use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use synapse_core::{Action, ComboInput, Key, KeystrokeDynamics};
use synapse_hid_host::{
    HOST_COMMAND_KEY_DOWN, HOST_COMMAND_KEY_MODS, HOST_COMMAND_KEY_UP, HOST_COMMAND_RELEASE_ALL,
    HidError, HidGateway, HostCommandRequest,
};

use crate::{ActionBackend, ActionError, EmitState};

mod keyboard;
mod keymap;
#[cfg(test)]
mod keymap_tests;
mod mouse;
mod pad;

#[cfg(test)]
mod tests;

const BOOT_KEYBOARD_KEY_LIMIT: usize = 6;

pub trait HardwareGateway: Send {
    /// Sends one firmware command and waits for the Pico ACK/NAK result.
    ///
    /// # Errors
    ///
    /// Returns an `ActionError` when the underlying HID link rejects or cannot
    /// deliver the command.
    fn send_command(&mut self, command: u8, payload: &[u8]) -> Result<u32, ActionError>;

    /// Sends a bounded batch of firmware commands and waits for ACK/NAK
    /// completion.
    ///
    /// # Errors
    ///
    /// Returns an `ActionError` when any command in the batch cannot be
    /// delivered or accepted by the underlying HID link.
    fn send_commands(
        &mut self,
        commands: &[HostCommandRequest<'_>],
    ) -> Result<Vec<u32>, ActionError> {
        commands
            .iter()
            .map(|request| self.send_command(request.command, request.payload))
            .collect()
    }
}

impl HardwareGateway for HidGateway {
    #[allow(clippy::use_self)]
    fn send_command(&mut self, command: u8, payload: &[u8]) -> Result<u32, ActionError> {
        HidGateway::send_command(self, command, payload).map_err(action_error_from_hid)
    }

    #[allow(clippy::use_self)]
    fn send_commands(
        &mut self,
        commands: &[HostCommandRequest<'_>],
    ) -> Result<Vec<u32>, ActionError> {
        HidGateway::send_commands(self, commands).map_err(action_error_from_hid)
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
        Action::TypeText { text, dynamics, .. } => type_text(gateway, text, dynamics, state),
        Action::MouseMove {
            to,
            curve,
            duration_ms,
            ..
        } => mouse::move_absolute(gateway, to, curve, *duration_ms),
        Action::MouseMoveRelative { dx, dy, .. } => mouse::move_relative(gateway, *dx, *dy),
        Action::MouseDrag {
            from,
            to,
            button,
            curve,
            duration_ms,
            ..
        } => mouse::drag(gateway, *from, *to, *button, curve, *duration_ms, state),
        Action::MouseButton {
            button,
            action,
            hold_ms,
            ..
        } => mouse::button(gateway, *button, *action, *hold_ms, state),
        Action::MouseScroll { dy, dx, .. } => mouse::scroll(gateway, *dy, *dx),
        Action::AimAt {
            target,
            style,
            deadline_ms,
            ..
        } => mouse::aim_at(gateway, target, *style, *deadline_ms),
        Action::PadButton {
            pad,
            button,
            action,
            hold_ms,
        } => pad::button(gateway, state, *pad, *button, *action, *hold_ms),
        Action::PadStick { pad, stick, x, y } => pad::stick(gateway, state, *pad, *stick, *x, *y),
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
    let current = held_keyboard_report(state, None)?;
    let mapped = keyboard::hid_key(key)?;
    let after = current.with_added(mapped)?;

    send_key_mods_if_changed(gateway, current.modifiers, after.modifiers)?;
    if let Some(usage) = mapped.key_usage {
        let payload = [usage];
        if let Err(error) = gateway.send_command(HOST_COMMAND_KEY_DOWN, &payload) {
            let _ = send_key_mods_if_changed(gateway, after.modifiers, current.modifiers);
            return Err(error);
        }
    }
    state.hold_key(key);
    Ok(())
}

fn key_up<G>(gateway: &mut G, key: &Key, state: &mut EmitState) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let current = held_keyboard_report(state, None)?;
    let after = held_keyboard_report(state, Some(key))?;
    let mapped = keyboard::hid_key(key)?;

    if let Some(usage) = mapped.key_usage {
        let payload = [usage];
        gateway.send_command(HOST_COMMAND_KEY_UP, &payload)?;
    }
    send_key_mods_if_changed(gateway, current.modifiers, after.modifiers)?;
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
    validate_chord_6kro(keys, state)?;
    for key in keys {
        key_down(gateway, key, state)?;
    }
    sleep_ms(hold_ms);
    for key in keys.iter().rev() {
        key_up(gateway, key, state)?;
    }
    Ok(())
}

fn type_text<G>(
    gateway: &mut G,
    text: &str,
    dynamics: &KeystrokeDynamics,
    state: &EmitState,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    for event in crate::sample_typing_schedule(text, dynamics, None) {
        sleep_ms(event.iki_ms_before);
        let key = keyboard::hid_text_key(event.r#char)?;
        mapped_key_press(gateway, key, 0, state)?;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct KeyboardReportState {
    modifiers: u8,
    keycodes: [u8; BOOT_KEYBOARD_KEY_LIMIT],
    key_count: usize,
}

impl KeyboardReportState {
    const fn neutral() -> Self {
        Self {
            modifiers: 0,
            keycodes: [0; BOOT_KEYBOARD_KEY_LIMIT],
            key_count: 0,
        }
    }

    fn with_added(mut self, key: keyboard::HidKeyboardKey) -> Result<Self, ActionError> {
        self.modifiers |= key.modifiers;
        if let Some(usage) = key.key_usage {
            self.add_key_usage(usage)?;
        }
        Ok(self)
    }

    fn add_key_usage(&mut self, usage: u8) -> Result<(), ActionError> {
        if self.keycodes[..self.key_count].contains(&usage) {
            return Ok(());
        }
        if self.key_count >= BOOT_KEYBOARD_KEY_LIMIT {
            return Err(ActionError::UnsupportedKey {
                detail:
                    "hardware keyboard 6KRO limit exceeded: at most 6 non-modifier keys can be held"
                        .to_owned(),
            });
        }
        self.keycodes[self.key_count] = usage;
        self.key_count += 1;
        Ok(())
    }
}

fn held_keyboard_report(
    state: &EmitState,
    excluded_key: Option<&Key>,
) -> Result<KeyboardReportState, ActionError> {
    let mut report = KeyboardReportState::neutral();
    for key in state.snapshot().held_keys {
        if excluded_key.is_some_and(|excluded| excluded == &key) {
            continue;
        }
        report = report.with_added(keyboard::hid_key(&key)?)?;
    }
    Ok(report)
}

fn validate_chord_6kro(keys: &[Key], state: &EmitState) -> Result<(), ActionError> {
    let mut report = held_keyboard_report(state, None)?;
    for key in keys {
        report = report.with_added(keyboard::hid_key(key)?)?;
    }
    Ok(())
}

fn mapped_key_press<G>(
    gateway: &mut G,
    key: keyboard::HidKeyboardKey,
    hold_ms: u32,
    state: &EmitState,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let current = held_keyboard_report(state, None)?;
    let after = current.with_added(key)?;

    send_key_mods_if_changed(gateway, current.modifiers, after.modifiers)?;
    if let Some(usage) = key.key_usage {
        let payload = [usage];
        if let Err(error) = gateway.send_command(HOST_COMMAND_KEY_DOWN, &payload) {
            let _ = send_key_mods_if_changed(gateway, after.modifiers, current.modifiers);
            return Err(error);
        }
        sleep_ms(hold_ms);
        if let Err(error) = gateway.send_command(HOST_COMMAND_KEY_UP, &payload) {
            let _ = send_key_mods_if_changed(gateway, after.modifiers, current.modifiers);
            return Err(error);
        }
    } else {
        sleep_ms(hold_ms);
    }
    send_key_mods_if_changed(gateway, after.modifiers, current.modifiers)
}

fn send_key_mods_if_changed<G>(gateway: &mut G, before: u8, after: u8) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    if before == after {
        return Ok(());
    }
    gateway.send_command(HOST_COMMAND_KEY_MODS, &[after])?;
    Ok(())
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
