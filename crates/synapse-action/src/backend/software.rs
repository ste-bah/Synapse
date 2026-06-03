use synapse_core::{Action, ComboInput, Point};

use crate::{ActionBackend, ActionError, EmitState, recovery};

mod input;
mod keyboard;
mod mouse;
mod text;
mod utils;

#[derive(Debug, Default)]
pub struct SoftwareBackend;

impl SoftwareBackend {
    #[must_use]
    #[tracing::instrument(fields(backend = "software"))]
    pub fn new() -> Self {
        Self
    }
}

/// Reads the current software cursor position in Win32 screen coordinates.
///
/// # Errors
///
/// Returns `ActionError::BackendUnavailable` when the OS cursor location cannot
/// be read from the active input desktop.
pub fn cursor_position() -> Result<Point, ActionError> {
    mouse::cursor_position()
}

impl ActionBackend for SoftwareBackend {
    #[tracing::instrument(skip_all, fields(backend = "software"))]
    fn execute(&self, action: &Action, state: &mut EmitState) -> Result<(), ActionError> {
        crate::validate_action(action)?;
        match action {
            Action::KeyPress { key, hold_ms, .. } => keyboard::press_key(key, *hold_ms, state),
            Action::KeyDown { key, .. } => keyboard::key_down(key, state),
            Action::KeyUp { key, .. } => keyboard::key_up(key, state),
            Action::KeyChord { keys, hold_ms, .. } => keyboard::key_chord(keys, *hold_ms, state),
            Action::TypeText { text, dynamics, .. } => text::type_text(text, dynamics),
            Action::MouseMove {
                to,
                curve,
                duration_ms,
                ..
            } => mouse::mouse_move(to, curve, *duration_ms),
            Action::MouseMoveRelative { dx, dy, .. } => mouse::mouse_move_relative(*dx, *dy),
            Action::MouseButton {
                button,
                action,
                hold_ms,
                ..
            } => mouse::mouse_button(*button, *action, *hold_ms, state),
            Action::MouseDrag {
                from,
                to,
                button,
                curve,
                duration_ms,
                ..
            } => mouse::mouse_drag(*from, *to, *button, curve, *duration_ms, state),
            Action::MouseStroke {
                path,
                button,
                profile,
                timing,
                humanize,
                ..
            } => mouse::mouse_stroke(path, *button, *profile, timing, *humanize, state),
            Action::MouseScroll { dy, dx, at, .. } => mouse::mouse_scroll(*dy, *dx, *at),
            Action::AimAt { target, style, .. } => mouse::aim_at(target, *style),
            Action::Combo { steps, .. } => combo(steps, state),
            Action::ReleaseAll => release_all(state),
            Action::PadButton { .. }
            | Action::PadStick { .. }
            | Action::PadTrigger { .. }
            | Action::PadReport { .. } => Err(ActionError::BackendUnavailable {
                detail: "software backend cannot emit gamepad actions".to_owned(),
            }),
        }
    }
}

#[tracing::instrument(skip_all, fields(action_kind = "software_combo"))]
fn combo(steps: &[synapse_core::ComboStep], state: &mut EmitState) -> Result<(), ActionError> {
    for step in steps {
        match &step.input {
            ComboInput::KeyDown { key } => keyboard::key_down(key, state)?,
            ComboInput::KeyUp { key } => keyboard::key_up(key, state)?,
            ComboInput::KeyPress { key, hold_ms } => {
                keyboard::press_key(key, u32::from(*hold_ms), state)?;
            }
            ComboInput::MouseButton { button, action } => {
                mouse::mouse_button(*button, *action, 0, state)?;
            }
            ComboInput::MouseMoveRel { dx, dy } => mouse::mouse_move_relative(*dx, *dy)?,
            ComboInput::PadButton { .. } | ComboInput::PadStick { .. } => {
                return Err(ActionError::BackendUnavailable {
                    detail: "software backend cannot emit gamepad combo steps".to_owned(),
                });
            }
        }
    }
    Ok(())
}

#[tracing::instrument(skip_all, fields(action_kind = "software_release_all"))]
fn release_all(state: &mut EmitState) -> Result<(), ActionError> {
    let snapshot = state.snapshot();
    let mut enigo = utils::enigo()?;
    keyboard::release_keys_with(&mut enigo, &snapshot.held_keys)?;
    mouse::release_buttons_with(&mut enigo, &snapshot.held_buttons)?;
    for key in &snapshot.held_keys {
        recovery::clear_held_key(key)?;
    }
    for button in &snapshot.held_buttons {
        recovery::clear_held_button(*button)?;
    }
    state.release_all();
    Ok(())
}
