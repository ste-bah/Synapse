use synapse_core::{Action, Backend};

use crate::{ActionResult, BackendResolutionPolicy, ResolvedBackend, resolve_backend_with_policy};

pub(super) fn resolved_backend_for_action(
    action: &Action,
    policy: BackendResolutionPolicy,
) -> ActionResult<ResolvedBackend> {
    resolve_backend_with_policy(requested_backend(action), action, policy)
}

pub(super) const fn action_consumes_rate_limit(action: &Action) -> bool {
    !matches!(action, Action::ReleaseAll | Action::KeyUp { .. })
}

const fn requested_backend(action: &Action) -> Backend {
    match action {
        Action::KeyPress { backend, .. }
        | Action::KeyDown { backend, .. }
        | Action::KeyUp { backend, .. }
        | Action::KeyChord { backend, .. }
        | Action::TypeText { backend, .. }
        | Action::MouseMove { backend, .. }
        | Action::MouseMoveRelative { backend, .. }
        | Action::MouseButton { backend, .. }
        | Action::MouseDrag { backend, .. }
        | Action::MouseScroll { backend, .. }
        | Action::AimAt { backend, .. }
        | Action::Combo { backend, .. } => *backend,
        Action::PadButton { .. }
        | Action::PadStick { .. }
        | Action::PadTrigger { .. }
        | Action::PadReport { .. }
        | Action::ReleaseAll => Backend::Auto,
    }
}

pub(super) const fn action_kind(action: &Action) -> &'static str {
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
