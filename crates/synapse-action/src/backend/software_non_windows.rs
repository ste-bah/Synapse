use synapse_core::Action;

use crate::{ActionBackend, ActionError, EmitState};

#[derive(Debug, Default)]
pub struct SoftwareBackend;

impl SoftwareBackend {
    #[must_use]
    #[tracing::instrument(fields(backend = "software"))]
    pub fn new() -> Self {
        Self
    }
}

impl ActionBackend for SoftwareBackend {
    #[tracing::instrument(skip_all, fields(backend = "software"))]
    fn execute(&self, action: &Action, _state: &mut EmitState) -> Result<(), ActionError> {
        crate::validate_action(action)?;
        Err(ActionError::BackendUnavailable {
            detail: format!(
                "software backend requires Windows; current target is non-Win; action_kind={}",
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
