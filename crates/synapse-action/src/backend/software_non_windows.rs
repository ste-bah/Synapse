use synapse_core::{Action, Point};

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

/// Reads the current software cursor position from the OS cursor backend.
///
/// # Errors
///
/// Always returns `ActionError::BackendUnavailable` on non-Windows targets.
pub fn cursor_position() -> Result<Point, ActionError> {
    Err(ActionError::BackendUnavailable {
        detail: "software cursor position requires Windows".to_owned(),
    })
}

impl ActionBackend for SoftwareBackend {
    #[tracing::instrument(skip_all, fields(backend = "software"))]
    fn execute(&self, action: &Action, state: &mut EmitState) -> Result<(), ActionError> {
        crate::validate_action(action)?;
        if matches!(action, Action::ReleaseAll) {
            // Empty `ReleaseAll` performs no I/O on any platform; only the
            // state bitmaps get cleared. Matching the Windows backend's
            // behavior for the same input means safety paths
            // (cancel/shutdown, panic hook, M2 `release_all` tool) succeed
            // when there is nothing held, instead of falsely returning
            // ACTION_BACKEND_UNAVAILABLE. Non-empty state on a non-Windows
            // host still fails-closed below — that would be a real
            // platform-mismatch bug we want surfaced.
            let snapshot = state.snapshot();
            if snapshot.held_keys.is_empty()
                && snapshot.held_buttons.is_empty()
                && snapshot.pad_state.is_empty()
            {
                state.release_all();
                return Ok(());
            }
        }
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
