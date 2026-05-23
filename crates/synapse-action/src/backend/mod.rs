use synapse_core::{Action, Backend};

use crate::{ActionError, EmitState};

#[cfg(windows)]
pub mod software;
#[cfg(not(windows))]
#[path = "software_non_windows.rs"]
pub mod software;

pub trait ActionBackend: Send + Sync {
    /// Executes one action against a concrete backend while updating emitter state.
    ///
    /// # Errors
    ///
    /// Returns an `ActionError` when the concrete backend cannot execute the
    /// action or when action validation fails.
    fn execute(&self, action: &Action, state: &mut EmitState) -> Result<(), ActionError>;
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ResolvedBackend {
    Software,
    Vigem,
}

impl ResolvedBackend {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Software => "software",
            Self::Vigem => "vigem",
        }
    }
}

/// Resolves the concrete backend for one per-call backend request.
///
/// # Errors
///
/// Returns `ActionError::BackendUnavailable` for `Backend::Hardware` in M2.
#[tracing::instrument(skip_all, fields(requested_backend = ?requested))]
pub fn resolve_backend(
    requested: Backend,
    action: &Action,
) -> Result<ResolvedBackend, ActionError> {
    match requested {
        Backend::Software => Ok(ResolvedBackend::Software),
        Backend::Vigem => Ok(ResolvedBackend::Vigem),
        Backend::Hardware => Err(ActionError::BackendUnavailable {
            detail: "hardware HID backend is unavailable in M2".to_owned(),
        }),
        Backend::Auto => Ok(auto_backend_for(action)),
    }
}

const fn auto_backend_for(action: &Action) -> ResolvedBackend {
    match action {
        Action::PadButton { .. }
        | Action::PadStick { .. }
        | Action::PadTrigger { .. }
        | Action::PadReport { .. } => ResolvedBackend::Vigem,
        Action::KeyPress { .. }
        | Action::KeyDown { .. }
        | Action::KeyUp { .. }
        | Action::KeyChord { .. }
        | Action::TypeText { .. }
        | Action::MouseMove { .. }
        | Action::MouseMoveRelative { .. }
        | Action::MouseButton { .. }
        | Action::MouseDrag { .. }
        | Action::MouseScroll { .. }
        | Action::AimAt { .. }
        | Action::Combo { .. }
        | Action::ReleaseAll => ResolvedBackend::Software,
    }
}
