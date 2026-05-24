use enigo::{Direction, Enigo, Key as EnigoKey, Keyboard};
use synapse_core::{Key, KeyCode};

use crate::{ActionError, EmitState};

use super::utils::{enigo, enigo_error, enigo_preserving_held_keys, sleep_ms};

#[tracing::instrument(skip_all, fields(action_kind = "software_key_press"))]
pub(super) fn press_key(key: &Key, hold_ms: u32, state: &mut EmitState) -> Result<(), ActionError> {
    let mut enigo = enigo()?;
    state.hold_key(key);
    emit_key(&mut enigo, key, Direction::Press)?;
    sleep_ms(hold_ms);
    emit_key(&mut enigo, key, Direction::Release)?;
    state.release_key(key);
    Ok(())
}

#[tracing::instrument(skip_all, fields(action_kind = "software_key_state"))]
pub(super) fn key_down(key: &Key, state: &mut EmitState) -> Result<(), ActionError> {
    let mut enigo = enigo_preserving_held_keys()?;
    state.hold_key(key);
    emit_key(&mut enigo, key, Direction::Press)
}

#[tracing::instrument(skip_all, fields(action_kind = "software_key_state"))]
pub(super) fn key_up(key: &Key, state: &mut EmitState) -> Result<(), ActionError> {
    let mut enigo = enigo()?;
    emit_key(&mut enigo, key, Direction::Release)?;
    state.release_key(key);
    Ok(())
}

#[tracing::instrument(skip_all, fields(action_kind = "software_key_chord"))]
pub(super) fn key_chord(
    keys: &[Key],
    hold_ms: u32,
    state: &mut EmitState,
) -> Result<(), ActionError> {
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

pub(super) fn release_keys_with(enigo: &mut Enigo, keys: &[Key]) -> Result<(), ActionError> {
    for key in keys.iter().rev() {
        emit_key(enigo, key, Direction::Release)?;
    }
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
