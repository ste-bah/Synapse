use synapse_core::Action;

use crate::{ActionBackend, ActionError, EmitState};

#[derive(Debug, Default)]
pub struct HardwareUnavailableBackend;

impl HardwareUnavailableBackend {
    #[must_use]
    #[tracing::instrument(fields(backend = "hardware"))]
    pub fn new() -> Self {
        Self
    }
}

impl ActionBackend for HardwareUnavailableBackend {
    #[tracing::instrument(skip_all, fields(backend = "hardware"))]
    fn execute(&self, action: &Action, _state: &mut EmitState) -> Result<(), ActionError> {
        crate::validate_action(action)?;
        Err(ActionError::BackendUnavailable {
            detail: format!(
                "backend=hardware reason=hardware HID backend ships in M4 action_kind={}",
                action_kind(action)
            ),
        })
    }
}

const fn action_kind(action: &Action) -> &'static str {
    match action {
        Action::KeyPress { .. } => "key_press",
        Action::KeyDown { .. } => "key_down",
        Action::KeyUp { .. } => "key_up",
        Action::KeyChord { .. } => "key_chord",
        Action::TypeText { .. } => "type_text",
        Action::MouseMove { .. } => "mouse_move",
        Action::MouseMoveRelative { .. } => "mouse_move_relative",
        Action::MouseButton { .. } => "mouse_button",
        Action::MouseDrag { .. } => "mouse_drag",
        Action::MouseScroll { .. } => "mouse_scroll",
        Action::PadButton { .. } => "pad_button",
        Action::PadStick { .. } => "pad_stick",
        Action::PadTrigger { .. } => "pad_trigger",
        Action::PadReport { .. } => "pad_report",
        Action::AimAt { .. } => "aim_at",
        Action::Combo { .. } => "combo",
        Action::ReleaseAll => "release_all",
    }
}
