use std::{mem, thread, time::Duration};

use enigo::{Button as EnigoButton, Direction, Enigo, Key as EnigoKey, Keyboard, Mouse, Settings};
use synapse_core::{
    Action, AimStyle, AimTarget, ButtonAction, ComboInput, Key, KeyCode, MouseButton, MouseTarget,
    Point,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, MOUSE_EVENT_FLAGS, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_MOVE, MOUSEEVENTF_WHEEL,
    MOUSEINPUT, SendInput, VIRTUAL_KEY,
};

use crate::{ActionBackend, ActionError, EmitState, sample_curve};

const CURVE_BATCH_STEPS: usize = 50;
const WHEEL_DELTA: i32 = 120;

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
    fn execute(&self, action: &Action, state: &mut EmitState) -> Result<(), ActionError> {
        crate::validate_action(action)?;
        match action {
            Action::KeyPress { key, hold_ms, .. } => press_key(key, *hold_ms, state),
            Action::KeyDown { key, .. } => set_key(key, Direction::Press, state),
            Action::KeyUp { key, .. } => set_key(key, Direction::Release, state),
            Action::KeyChord { keys, hold_ms, .. } => key_chord(keys, *hold_ms, state),
            Action::TypeText { text, .. } => type_text(text),
            Action::MouseMove { to, .. } => mouse_move(to),
            Action::MouseMoveRelative { dx, dy, .. } => mouse_move_relative(*dx, *dy),
            Action::MouseButton {
                button,
                action,
                hold_ms,
                ..
            } => mouse_button(*button, *action, *hold_ms, state),
            Action::MouseDrag {
                from,
                to,
                button,
                curve,
                duration_ms,
                ..
            } => mouse_drag(*from, *to, *button, curve, *duration_ms, state),
            Action::MouseScroll { dy, dx, at, .. } => mouse_scroll(*dy, *dx, *at),
            Action::AimAt { target, style, .. } => aim_at(target, *style),
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

#[tracing::instrument(skip_all, fields(action_kind = "software_key_press"))]
fn press_key(key: &Key, hold_ms: u32, state: &mut EmitState) -> Result<(), ActionError> {
    let mut enigo = enigo()?;
    state.hold_key(key);
    emit_key(&mut enigo, key, Direction::Press)?;
    sleep_ms(hold_ms);
    emit_key(&mut enigo, key, Direction::Release)?;
    state.release_key(key);
    Ok(())
}

#[tracing::instrument(skip_all, fields(action_kind = "software_key_state"))]
fn set_key(key: &Key, direction: Direction, state: &mut EmitState) -> Result<(), ActionError> {
    let mut enigo = enigo()?;
    match direction {
        Direction::Press => {
            state.hold_key(key);
            emit_key(&mut enigo, key, Direction::Press)
        }
        Direction::Release => {
            emit_key(&mut enigo, key, Direction::Release)?;
            state.release_key(key);
            Ok(())
        }
        Direction::Click => press_key(key, 0, state),
    }
}

#[tracing::instrument(skip_all, fields(action_kind = "software_key_chord"))]
fn key_chord(keys: &[Key], hold_ms: u32, state: &mut EmitState) -> Result<(), ActionError> {
    let mut enigo = enigo()?;
    for key in keys {
        validate_key(key)?;
    }
    for key in keys {
        state.hold_key(key);
        emit_key(&mut enigo, key, Direction::Press)?;
    }
    sleep_ms(hold_ms);
    for key in keys.iter().rev() {
        emit_key(&mut enigo, key, Direction::Release)?;
        state.release_key(key);
    }
    Ok(())
}

#[tracing::instrument(skip_all, fields(action_kind = "software_type_text"))]
fn type_text(text: &str) -> Result<(), ActionError> {
    let mut inputs = Vec::with_capacity(text.encode_utf16().count() * 2);
    for unit in text.encode_utf16() {
        inputs.push(keyboard_input(unit, KEYEVENTF_UNICODE));
        inputs.push(keyboard_input(unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    send_input_batch(&inputs, "unicode text")
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_move"))]
fn mouse_move(target: &MouseTarget) -> Result<(), ActionError> {
    let MouseTarget::Screen { point } = target else {
        return Err(ActionError::TargetInvalid {
            detail: "software backend requires a resolved screen point for mouse movement"
                .to_owned(),
        });
    };
    let enigo = enigo()?;
    let (x, y) = enigo
        .location()
        .map_err(enigo_error("read cursor position"))?;
    mouse_move_relative_i32(point.x - x, point.y - y)
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_move_relative"))]
fn mouse_move_relative(dx: f32, dy: f32) -> Result<(), ActionError> {
    #[allow(clippy::cast_possible_truncation)]
    let rounded = (dx.round() as i32, dy.round() as i32);
    mouse_move_relative_i32(rounded.0, rounded.1)
}

fn mouse_move_relative_i32(dx: i32, dy: i32) -> Result<(), ActionError> {
    if dx == 0 && dy == 0 {
        return send_input_batch(&[], "relative mouse move");
    }
    let inputs: Vec<_> = curve_steps(dx, dy)
        .into_iter()
        .map(|(step_x, step_y)| mouse_input(step_x, step_y, 0, MOUSEEVENTF_MOVE))
        .collect();
    send_input_batch(&inputs, "relative mouse move")
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_button"))]
fn mouse_button(
    button: MouseButton,
    action: ButtonAction,
    hold_ms: u32,
    state: &mut EmitState,
) -> Result<(), ActionError> {
    let mut enigo = enigo()?;
    let enigo_button = enigo_button(button);
    match action {
        ButtonAction::Down => {
            state.apply_mouse_button(button, ButtonAction::Down);
            enigo
                .button(enigo_button, Direction::Press)
                .map_err(enigo_error("emit mouse button"))
        }
        ButtonAction::Up => {
            enigo
                .button(enigo_button, Direction::Release)
                .map_err(enigo_error("emit mouse button"))?;
            state.apply_mouse_button(button, ButtonAction::Up);
            Ok(())
        }
        ButtonAction::Press => {
            state.apply_mouse_button(button, ButtonAction::Down);
            enigo
                .button(enigo_button, Direction::Press)
                .map_err(enigo_error("emit mouse button"))?;
            sleep_ms(hold_ms);
            enigo
                .button(enigo_button, Direction::Release)
                .map_err(enigo_error("emit mouse button"))?;
            state.apply_mouse_button(button, ButtonAction::Up);
            Ok(())
        }
    }
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_drag"))]
fn mouse_drag(
    from: Point,
    to: Point,
    button: MouseButton,
    curve: &synapse_core::AimCurve,
    duration_ms: u32,
    state: &mut EmitState,
) -> Result<(), ActionError> {
    let mut enigo = enigo()?;
    enigo
        .move_mouse(from.x, from.y, enigo::Coordinate::Abs)
        .map_err(enigo_error("move to drag origin"))?;
    mouse_button(button, ButtonAction::Down, 0, state)?;
    mouse_move_curve(from, to, curve, duration_ms)?;
    mouse_button(button, ButtonAction::Up, 0, state)
}

fn mouse_move_curve(
    from: Point,
    to: Point,
    curve: &synapse_core::AimCurve,
    duration_ms: u32,
) -> Result<(), ActionError> {
    let samples = sample_curve(curve, from, to, duration_ms, None);
    let mut inputs = Vec::with_capacity(samples.len().saturating_sub(1));
    let mut previous = from;
    for point in samples.into_iter().skip(1) {
        inputs.push(mouse_input(
            point.x.saturating_sub(previous.x),
            point.y.saturating_sub(previous.y),
            0,
            MOUSEEVENTF_MOVE,
        ));
        previous = point;
    }
    send_input_batch(&inputs, "drag curve mouse move")
}

#[tracing::instrument(skip_all, fields(action_kind = "software_mouse_scroll"))]
fn mouse_scroll(dy: i32, dx: i32, at: Option<Point>) -> Result<(), ActionError> {
    if let Some(point) = at {
        let mut enigo = enigo()?;
        enigo
            .move_mouse(point.x, point.y, enigo::Coordinate::Abs)
            .map_err(enigo_error("move to scroll point"))?;
    }
    let mut inputs = Vec::with_capacity(2);
    if dy != 0 {
        inputs.push(mouse_input(
            0,
            0,
            signed_to_u32(dy.saturating_mul(WHEEL_DELTA)),
            MOUSEEVENTF_WHEEL,
        ));
    }
    if dx != 0 {
        inputs.push(mouse_input(
            0,
            0,
            signed_to_u32(dx.saturating_mul(WHEEL_DELTA)),
            MOUSEEVENTF_HWHEEL,
        ));
    }
    send_input_batch(&inputs, "mouse scroll")
}

#[tracing::instrument(skip_all, fields(action_kind = "software_aim_at"))]
fn aim_at(target: &AimTarget, style: AimStyle) -> Result<(), ActionError> {
    if style == AimStyle::Track {
        return Err(ActionError::BackendUnavailable {
            detail: "track aim requires the M3 reflex runtime".to_owned(),
        });
    }
    let AimTarget::Screen { point } = target else {
        return Err(ActionError::TargetInvalid {
            detail: "software aim requires a resolved screen point".to_owned(),
        });
    };
    mouse_move(&MouseTarget::Screen { point: *point })
}

#[tracing::instrument(skip_all, fields(action_kind = "software_combo"))]
fn combo(steps: &[synapse_core::ComboStep], state: &mut EmitState) -> Result<(), ActionError> {
    for step in steps {
        match &step.input {
            ComboInput::KeyDown { key } => set_key(key, Direction::Press, state)?,
            ComboInput::KeyUp { key } => set_key(key, Direction::Release, state)?,
            ComboInput::KeyPress { key, hold_ms } => press_key(key, u32::from(*hold_ms), state)?,
            ComboInput::MouseButton { button, action } => {
                mouse_button(*button, *action, 0, state)?;
            }
            ComboInput::MouseMoveRel { dx, dy } => mouse_move_relative(*dx, *dy)?,
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
    let mut enigo = enigo()?;
    for key in snapshot.held_keys.iter().rev() {
        emit_key(&mut enigo, key, Direction::Release)?;
    }
    for button in snapshot.held_buttons.iter().rev() {
        enigo
            .button(enigo_button(*button), Direction::Release)
            .map_err(enigo_error("release held mouse button"))?;
    }
    state.release_all();
    Ok(())
}

fn emit_key(enigo: &mut Enigo, key: &Key, direction: Direction) -> Result<(), ActionError> {
    if key.use_scancode {
        let KeyCode::HidCode { value } = &key.code else {
            return Err(unsupported_key(key));
        };
        enigo
            .raw(u16::from(*value), direction)
            .map_err(enigo_error("emit raw scancode"))
    } else {
        enigo
            .key(enigo_key(key)?, direction)
            .map_err(enigo_error("emit key"))
    }
}

fn validate_key(key: &Key) -> Result<(), ActionError> {
    if key.use_scancode {
        matches!(&key.code, KeyCode::HidCode { .. })
            .then_some(())
            .ok_or_else(|| unsupported_key(key))
    } else {
        enigo_key(key).map(|_key| ())
    }
}

fn enigo_key(key: &Key) -> Result<EnigoKey, ActionError> {
    match &key.code {
        KeyCode::Symbol { value } => Ok(EnigoKey::Unicode(*value)),
        KeyCode::HidCode { .. } => Err(unsupported_key(key)),
        KeyCode::Named { value } => named_key(value).ok_or_else(|| unsupported_key(key)),
    }
}

fn named_key(value: &str) -> Option<EnigoKey> {
    let lower = value.to_ascii_lowercase();
    if let Some(ch) = single_ascii(&lower) {
        return Some(EnigoKey::Unicode(ch));
    }
    match lower.as_str() {
        "alt" => Some(EnigoKey::Alt),
        "backspace" => Some(EnigoKey::Backspace),
        "ctrl" | "control" => Some(EnigoKey::Control),
        "delete" => Some(EnigoKey::Delete),
        "down" | "arrowdown" => Some(EnigoKey::DownArrow),
        "end" => Some(EnigoKey::End),
        "enter" | "return" => Some(EnigoKey::Return),
        "escape" | "esc" => Some(EnigoKey::Escape),
        "home" => Some(EnigoKey::Home),
        "insert" => Some(EnigoKey::Insert),
        "left" | "arrowleft" => Some(EnigoKey::LeftArrow),
        "meta" | "win" | "windows" | "super" => Some(EnigoKey::Meta),
        "pagedown" => Some(EnigoKey::PageDown),
        "pageup" => Some(EnigoKey::PageUp),
        "right" | "arrowright" => Some(EnigoKey::RightArrow),
        "shift" => Some(EnigoKey::Shift),
        "space" => Some(EnigoKey::Space),
        "tab" => Some(EnigoKey::Tab),
        "up" | "arrowup" => Some(EnigoKey::UpArrow),
        "f1" => Some(EnigoKey::F1),
        "f2" => Some(EnigoKey::F2),
        "f3" => Some(EnigoKey::F3),
        "f4" => Some(EnigoKey::F4),
        "f5" => Some(EnigoKey::F5),
        "f6" => Some(EnigoKey::F6),
        "f7" => Some(EnigoKey::F7),
        "f8" => Some(EnigoKey::F8),
        "f9" => Some(EnigoKey::F9),
        "f10" => Some(EnigoKey::F10),
        "f11" => Some(EnigoKey::F11),
        "f12" => Some(EnigoKey::F12),
        _ => None,
    }
}

const fn enigo_button(button: MouseButton) -> EnigoButton {
    match button {
        MouseButton::Left => EnigoButton::Left,
        MouseButton::Right => EnigoButton::Right,
        MouseButton::Middle => EnigoButton::Middle,
        MouseButton::X1 => EnigoButton::Back,
        MouseButton::X2 => EnigoButton::Forward,
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn curve_steps(dx: i32, dy: i32) -> Vec<(i32, i32)> {
    let mut steps = Vec::with_capacity(CURVE_BATCH_STEPS);
    let mut prev_x = 0;
    let mut prev_y = 0;
    for index in 1..=CURVE_BATCH_STEPS {
        let progress = index as f32 / CURVE_BATCH_STEPS as f32;
        let x = (dx as f32 * progress).round() as i32;
        let y = (dy as f32 * progress).round() as i32;
        steps.push((x - prev_x, y - prev_y));
        prev_x = x;
        prev_y = y;
    }
    steps
}

const fn keyboard_input(scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

const fn mouse_input(dx: i32, dy: i32, mouse_data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_input_batch(inputs: &[INPUT], detail: &'static str) -> Result<(), ActionError> {
    if inputs.is_empty() {
        return Ok(());
    }
    let cb_size =
        i32::try_from(mem::size_of::<INPUT>()).map_err(|_err| ActionError::BackendUnavailable {
            detail: "INPUT struct size does not fit SendInput cbSize".to_owned(),
        })?;
    // SAFETY: `inputs` points to initialized Windows `INPUT` values for the
    // duration of the call, and `cb_size` is exactly `size_of::<INPUT>()`.
    let sent = unsafe { SendInput(inputs, cb_size) };
    let expected = u32::try_from(inputs.len()).map_err(|_err| ActionError::BackendUnavailable {
        detail: "SendInput input count does not fit u32".to_owned(),
    })?;
    if sent == expected {
        Ok(())
    } else {
        Err(ActionError::BackendUnavailable {
            detail: format!(
                "SendInput inserted {sent}/{} events for {detail}",
                inputs.len()
            ),
        })
    }
}

fn enigo() -> Result<Enigo, ActionError> {
    Enigo::new(&Settings::default()).map_err(|err| ActionError::BackendUnavailable {
        detail: format!("failed to initialize enigo: {err}"),
    })
}

fn enigo_error(context: &'static str) -> impl FnOnce(enigo::InputError) -> ActionError {
    move |err| ActionError::BackendUnavailable {
        detail: format!("{context}: {err}"),
    }
}

fn unsupported_key(key: &Key) -> ActionError {
    ActionError::UnsupportedKey {
        detail: format!("software backend does not support key code {:?}", key.code),
    }
}

fn single_ascii(value: &str) -> Option<char> {
    let mut chars = value.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}

const fn signed_to_u32(value: i32) -> u32 {
    u32::from_ne_bytes(value.to_ne_bytes())
}

fn sleep_ms(milliseconds: u32) {
    if milliseconds > 0 {
        thread::sleep(Duration::from_millis(u64::from(milliseconds)));
    }
}
