use synapse_core::{Action, ButtonAction, GamepadReport, PadButton, PadId, Stick, Trigger};

use crate::{ActionBackend, ActionError, EmitState};

/// M2 placeholder for the ViGEm driver-backed gamepad backend.
///
/// The production gamepad driver is tracked under issue #156. Until that ships,
/// every `Action::Pad*` is applied to the emitter's `EmitState::pad_state` only,
/// so safety paths (`ReleaseAll`, snapshots, neutral coalescing) still see the
/// commanded gamepad state. Non-pad actions return
/// `ACTION_BACKEND_UNAVAILABLE` because dispatching them through the ViGEm
/// path would be a category error.
#[derive(Debug, Default)]
pub struct VigemStateOnlyBackend;

impl VigemStateOnlyBackend {
    #[must_use]
    #[tracing::instrument(fields(backend = "vigem"))]
    pub fn new() -> Self {
        Self
    }
}

impl ActionBackend for VigemStateOnlyBackend {
    #[tracing::instrument(skip_all, fields(backend = "vigem"))]
    fn execute(&self, action: &Action, state: &mut EmitState) -> Result<(), ActionError> {
        crate::validate_action(action)?;
        match action {
            Action::PadButton {
                pad,
                button,
                action: btn_action,
                ..
            } => {
                apply_pad_button(state, *pad, *button, *btn_action);
                Ok(())
            }
            Action::PadStick { pad, stick, x, y } => {
                apply_pad_stick(state, *pad, *stick, *x, *y);
                Ok(())
            }
            Action::PadTrigger {
                pad,
                trigger,
                value,
            } => {
                apply_pad_trigger(state, *pad, *trigger, *value);
                Ok(())
            }
            Action::PadReport { pad, report } => {
                apply_pad_report(state, *pad, report.clone());
                Ok(())
            }
            _ => Err(ActionError::BackendUnavailable {
                detail: format!(
                    "backend=vigem reason=routed non-gamepad action through gamepad backend action_kind={}",
                    action_kind(action)
                ),
            }),
        }
    }
}

pub(crate) fn apply_pad_button(
    state: &mut EmitState,
    pad: PadId,
    button: PadButton,
    action: ButtonAction,
) {
    let should_remove = {
        let report = state
            .pad_state
            .entry(pad)
            .or_insert_with(neutral_gamepad_report);
        match action {
            ButtonAction::Down => push_unique(&mut report.buttons, button),
            ButtonAction::Up | ButtonAction::Press => {
                report.buttons.retain(|held| *held != button);
            }
        }
        is_neutral_report(report)
    };

    if should_remove {
        state.pad_state.remove(&pad);
    }
}

pub(crate) fn apply_pad_stick(state: &mut EmitState, pad: PadId, stick: Stick, x: f32, y: f32) {
    let should_remove = {
        let report = state
            .pad_state
            .entry(pad)
            .or_insert_with(neutral_gamepad_report);
        match stick {
            Stick::Left => report.thumb_l = (x, y),
            Stick::Right => report.thumb_r = (x, y),
        }
        is_neutral_report(report)
    };

    if should_remove {
        state.pad_state.remove(&pad);
    }
}

pub(crate) fn apply_pad_trigger(state: &mut EmitState, pad: PadId, trigger: Trigger, value: f32) {
    let should_remove = {
        let report = state
            .pad_state
            .entry(pad)
            .or_insert_with(neutral_gamepad_report);
        match trigger {
            Trigger::Left => report.lt = value,
            Trigger::Right => report.rt = value,
        }
        is_neutral_report(report)
    };

    if should_remove {
        state.pad_state.remove(&pad);
    }
}

pub(crate) fn apply_pad_report(state: &mut EmitState, pad: PadId, report: GamepadReport) {
    if is_neutral_report(&report) {
        state.pad_state.remove(&pad);
    } else {
        state.pad_state.insert(pad, report);
    }
}

const fn neutral_gamepad_report() -> GamepadReport {
    GamepadReport {
        buttons: Vec::new(),
        thumb_l: (0.0, 0.0),
        thumb_r: (0.0, 0.0),
        lt: 0.0,
        rt: 0.0,
    }
}

fn is_neutral_report(report: &GamepadReport) -> bool {
    report.buttons.is_empty()
        && report.thumb_l == (0.0, 0.0)
        && report.thumb_r == (0.0, 0.0)
        && report.lt == 0.0
        && report.rt == 0.0
}

fn push_unique(buttons: &mut Vec<PadButton>, button: PadButton) {
    if !buttons.contains(&button) {
        buttons.push(button);
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
