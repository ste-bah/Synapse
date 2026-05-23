use std::collections::{BTreeSet, HashMap};

use synapse_core::{
    Action, AimCurve, ButtonAction, ComboInput, GamepadReport, Key, KeyCode, KeystrokeDynamics,
    MouseButton, MouseTarget, PadButton, PadId, Point, Stick, Trigger,
};

use super::RecordedInput;
use crate::{EmitState, ModifierMask, sample_typing_schedule};

#[derive(Clone, Debug, Default)]
pub(super) struct RecordingState {
    pub(super) events: Vec<RecordedInput>,
    pub(super) held_keys: BTreeSet<KeyCode>,
    pub(super) held_buttons: BTreeSet<MouseButton>,
    pub(super) pad_state: HashMap<PadId, GamepadReport>,
}

impl RecordingState {
    pub(super) fn apply_action(&mut self, action: &Action, state: &mut EmitState) {
        match action {
            Action::KeyPress { key, hold_ms, .. } => self.key_press(key, *hold_ms, state),
            Action::KeyDown { key, .. } => self.key_down(key, state),
            Action::KeyUp { key, .. } => self.key_up(key, state),
            Action::KeyChord { keys, hold_ms, .. } => self.key_chord(keys, *hold_ms, state),
            Action::TypeText { text, dynamics, .. } => self.type_text(text, dynamics, state),
            Action::MouseMove {
                to,
                curve,
                duration_ms,
                ..
            } => self.events.push(RecordedInput::MouseMove {
                to: to.clone(),
                curve: curve.clone(),
                duration_ms: *duration_ms,
            }),
            Action::MouseMoveRelative { dx, dy, .. } => {
                self.mouse_move_relative(f64::from(*dx), f64::from(*dy));
            }
            Action::MouseButton {
                button,
                action,
                hold_ms,
                ..
            } => self.mouse_button(*button, *action, *hold_ms, state),
            Action::MouseDrag {
                to,
                button,
                curve,
                duration_ms,
                ..
            } => self.mouse_drag(*to, *button, curve, *duration_ms, state),
            Action::MouseScroll { dy, dx, at, .. } => {
                self.events.push(RecordedInput::MouseScroll {
                    dy: *dy,
                    dx: *dx,
                    at: *at,
                });
            }
            Action::PadButton {
                pad,
                button,
                action,
                hold_ms,
            } => self.pad_button(*pad, *button, *action, *hold_ms, state),
            Action::PadStick { pad, stick, x, y } => self.pad_stick(*pad, *stick, *x, *y, state),
            Action::PadTrigger {
                pad,
                trigger,
                value,
            } => self.pad_trigger(*pad, *trigger, *value, state),
            Action::PadReport { pad, report } => self.pad_report(*pad, report.clone(), state),
            Action::AimAt {
                target,
                style,
                deadline_ms,
                ..
            } => self.events.push(RecordedInput::AimAt {
                target: target.clone(),
                style: *style,
                deadline_ms: *deadline_ms,
            }),
            Action::Combo { steps, .. } => {
                for step in steps {
                    self.events
                        .push(RecordedInput::ComboAt { at_ms: step.at_ms });
                    self.combo_input(&step.input, state);
                }
            }
            Action::ReleaseAll => self.release_all(state),
        }
    }

    fn key_press(&mut self, key: &Key, hold_ms: u32, state: &mut EmitState) {
        self.key_down(key, state);
        self.delay(hold_ms);
        self.key_up(key, state);
    }

    fn key_down(&mut self, key: &Key, state: &mut EmitState) {
        self.events
            .push(RecordedInput::KeyDown { key: key.clone() });
        self.held_keys.insert(key.code.clone());
        state.hold_key(key);
    }

    fn key_up(&mut self, key: &Key, state: &mut EmitState) {
        self.events.push(RecordedInput::KeyUp { key: key.clone() });
        self.held_keys.remove(&key.code);
        state.release_key(key);
    }

    fn key_chord(&mut self, keys: &[Key], hold_ms: u32, state: &mut EmitState) {
        for key in keys {
            self.key_down(key, state);
        }
        self.delay(hold_ms);
        for key in keys.iter().rev() {
            self.key_up(key, state);
        }
    }

    fn type_text(&mut self, text: &str, dynamics: &KeystrokeDynamics, state: &mut EmitState) {
        for event in sample_typing_schedule(text, dynamics, None) {
            if event.iki_ms_before > 0 {
                self.delay(event.iki_ms_before);
            }
            if is_reversible_key_character(event.r#char) {
                self.type_key_event(&event.key, event.modifier_state, state);
            } else {
                self.type_unicode_units(event.r#char);
            }
        }
    }

    fn type_key_event(&mut self, key: &Key, modifier_state: ModifierMask, state: &mut EmitState) {
        let modifiers = modifier_keys(modifier_state);
        for modifier in &modifiers {
            self.key_down(modifier, state);
        }
        self.key_down(key, state);
        self.key_up(key, state);
        for modifier in modifiers.iter().rev() {
            self.key_up(modifier, state);
        }
    }

    fn type_unicode_units(&mut self, ch: char) {
        let mut units = [0; 2];
        for unit in ch.encode_utf16(&mut units) {
            self.events
                .push(RecordedInput::UnicodeUnitDown { unit: *unit });
            self.events
                .push(RecordedInput::UnicodeUnitUp { unit: *unit });
        }
    }

    fn mouse_button(
        &mut self,
        button: MouseButton,
        action: ButtonAction,
        hold_ms: u32,
        state: &mut EmitState,
    ) {
        match action {
            ButtonAction::Down => self.mouse_button_down(button, state),
            ButtonAction::Up => self.mouse_button_up(button, state),
            ButtonAction::Press => {
                self.mouse_button_down(button, state);
                self.delay(hold_ms);
                self.mouse_button_up(button, state);
            }
        }
    }

    fn mouse_button_down(&mut self, button: MouseButton, state: &mut EmitState) {
        self.events.push(RecordedInput::MouseButtonDown { button });
        self.held_buttons.insert(button);
        state.apply_mouse_button(button, ButtonAction::Down);
    }

    fn mouse_button_up(&mut self, button: MouseButton, state: &mut EmitState) {
        self.events.push(RecordedInput::MouseButtonUp { button });
        self.held_buttons.remove(&button);
        state.apply_mouse_button(button, ButtonAction::Up);
    }

    fn mouse_drag(
        &mut self,
        to: Point,
        button: MouseButton,
        curve: &AimCurve,
        duration_ms: u32,
        state: &mut EmitState,
    ) {
        self.mouse_button_down(button, state);
        self.events.push(RecordedInput::MouseMove {
            to: MouseTarget::Screen { point: to },
            curve: curve.clone(),
            duration_ms,
        });
        self.mouse_button_up(button, state);
    }

    fn mouse_move_relative(&mut self, dx: f64, dy: f64) {
        self.events
            .push(RecordedInput::MouseMoveRelative { dx, dy });
    }

    fn combo_input(&mut self, input: &ComboInput, state: &mut EmitState) {
        match input {
            ComboInput::KeyDown { key } => self.key_down(key, state),
            ComboInput::KeyUp { key } => self.key_up(key, state),
            ComboInput::KeyPress { key, hold_ms } => {
                self.key_press(key, u32::from(*hold_ms), state);
            }
            ComboInput::MouseButton { button, action } => {
                self.mouse_button(*button, *action, 0, state);
            }
            ComboInput::MouseMoveRel { dx, dy } => {
                self.mouse_move_relative(f64::from(*dx), f64::from(*dy));
            }
            ComboInput::PadButton {
                pad,
                button,
                action,
            } => self.pad_button(*pad, *button, *action, 0, state),
            ComboInput::PadStick { pad, stick, x, y } => {
                self.pad_stick(*pad, *stick, *x, *y, state);
            }
        }
    }

    fn pad_button(
        &mut self,
        pad: PadId,
        button: PadButton,
        action: ButtonAction,
        hold_ms: u32,
        state: &mut EmitState,
    ) {
        match action {
            ButtonAction::Down => self.pad_button_down(pad, button, state),
            ButtonAction::Up => self.pad_button_up(pad, button, state),
            ButtonAction::Press => {
                self.pad_button_down(pad, button, state);
                self.delay(hold_ms);
                self.pad_button_up(pad, button, state);
            }
        }
    }

    fn pad_button_down(&mut self, pad: PadId, button: PadButton, state: &mut EmitState) {
        self.events
            .push(RecordedInput::PadButtonDown { pad, button });
        push_unique(&mut self.pad_report_mut(pad).buttons, button);
        apply_pad_button_to_emit_state(state, pad, button, ButtonAction::Down);
    }

    fn pad_button_up(&mut self, pad: PadId, button: PadButton, state: &mut EmitState) {
        self.events.push(RecordedInput::PadButtonUp { pad, button });
        self.pad_report_mut(pad)
            .buttons
            .retain(|held| *held != button);
        self.trim_neutral_pad(pad);
        apply_pad_button_to_emit_state(state, pad, button, ButtonAction::Up);
    }

    fn pad_stick(&mut self, pad: PadId, stick: Stick, x: f32, y: f32, state: &mut EmitState) {
        self.events
            .push(RecordedInput::PadStick { pad, stick, x, y });
        match stick {
            Stick::Left => self.pad_report_mut(pad).thumb_l = (x, y),
            Stick::Right => self.pad_report_mut(pad).thumb_r = (x, y),
        }
        self.trim_neutral_pad(pad);
        apply_pad_stick_to_emit_state(state, pad, stick, x, y);
    }

    fn pad_trigger(&mut self, pad: PadId, trigger: Trigger, value: f32, state: &mut EmitState) {
        self.events.push(RecordedInput::PadTrigger {
            pad,
            trigger,
            value,
        });
        match trigger {
            Trigger::Left => self.pad_report_mut(pad).lt = value,
            Trigger::Right => self.pad_report_mut(pad).rt = value,
        }
        self.trim_neutral_pad(pad);
        apply_pad_trigger_to_emit_state(state, pad, trigger, value);
    }

    fn pad_report(&mut self, pad: PadId, report: GamepadReport, state: &mut EmitState) {
        self.events.push(RecordedInput::PadReport {
            pad,
            report: report.clone(),
        });
        if is_neutral_report(&report) {
            self.pad_state.remove(&pad);
            state.pad_state.remove(&pad);
        } else {
            self.pad_state.insert(pad, report.clone());
            state.pad_state.insert(pad, report);
        }
    }

    fn release_all(&mut self, state: &mut EmitState) {
        let held_keys = self.held_keys.iter().cloned().collect();
        let held_buttons = self.held_buttons.iter().copied().collect();
        let mut pads: Vec<_> = self.pad_state.keys().copied().collect();
        pads.sort_unstable();
        self.events.push(RecordedInput::ReleaseAll {
            held_keys,
            held_buttons,
            pads,
        });
        self.held_keys.clear();
        self.held_buttons.clear();
        self.pad_state.clear();
        state.release_all();
    }

    fn delay(&mut self, ms: u32) {
        self.events.push(RecordedInput::DelayMs { ms });
    }

    fn pad_report_mut(&mut self, pad: PadId) -> &mut GamepadReport {
        self.pad_state
            .entry(pad)
            .or_insert_with(neutral_gamepad_report)
    }

    fn trim_neutral_pad(&mut self, pad: PadId) {
        if self.pad_state.get(&pad).is_some_and(is_neutral_report) {
            self.pad_state.remove(&pad);
        }
    }
}

fn apply_pad_button_to_emit_state(
    state: &mut EmitState,
    pad: PadId,
    button: PadButton,
    action: ButtonAction,
) {
    let report = state
        .pad_state
        .entry(pad)
        .or_insert_with(neutral_gamepad_report);
    match action {
        ButtonAction::Down => push_unique(&mut report.buttons, button),
        ButtonAction::Up | ButtonAction::Press => report.buttons.retain(|held| *held != button),
    }
    trim_neutral_emit_pad(state, pad);
}

fn apply_pad_stick_to_emit_state(state: &mut EmitState, pad: PadId, stick: Stick, x: f32, y: f32) {
    let report = state
        .pad_state
        .entry(pad)
        .or_insert_with(neutral_gamepad_report);
    match stick {
        Stick::Left => report.thumb_l = (x, y),
        Stick::Right => report.thumb_r = (x, y),
    }
    trim_neutral_emit_pad(state, pad);
}

fn apply_pad_trigger_to_emit_state(
    state: &mut EmitState,
    pad: PadId,
    trigger: Trigger,
    value: f32,
) {
    let report = state
        .pad_state
        .entry(pad)
        .or_insert_with(neutral_gamepad_report);
    match trigger {
        Trigger::Left => report.lt = value,
        Trigger::Right => report.rt = value,
    }
    trim_neutral_emit_pad(state, pad);
}

fn trim_neutral_emit_pad(state: &mut EmitState, pad: PadId) {
    if state.pad_state.get(&pad).is_some_and(is_neutral_report) {
        state.pad_state.remove(&pad);
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

const fn is_reversible_key_character(ch: char) -> bool {
    matches!(
        ch,
        'A'..='Z'
            | 'a'..='z'
            | '0'..='9'
            | '\n'
            | '\t'
            | ' '
            | '!'
            | '@'
            | '#'
            | '$'
            | '%'
            | '^'
            | '&'
            | '*'
            | '('
            | ')'
            | '_'
            | '+'
            | '{'
            | '}'
            | '|'
            | ':'
            | '"'
            | '<'
            | '>'
            | '?'
            | '~'
            | '-'
            | '='
            | '['
            | ']'
            | '\\'
            | ';'
            | '\''
            | ','
            | '.'
            | '/'
            | '`'
    )
}

fn modifier_keys(mask: ModifierMask) -> Vec<Key> {
    [
        (ModifierMask::SHIFT, "shift"),
        (ModifierMask::CTRL, "ctrl"),
        (ModifierMask::ALT, "alt"),
        (ModifierMask::META, "meta"),
    ]
    .into_iter()
    .filter(|(modifier, _name)| mask.contains(*modifier))
    .map(|(_modifier, name)| named_key(name))
    .collect()
}

fn named_key(value: &str) -> Key {
    Key {
        code: KeyCode::Named {
            value: value.to_owned(),
        },
        use_scancode: false,
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
