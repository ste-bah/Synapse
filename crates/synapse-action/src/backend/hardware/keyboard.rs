use synapse_core::{Key, KeyCode};

use crate::ActionError;

pub(super) fn hid_key_code(key: &Key) -> Result<u8, ActionError> {
    match &key.code {
        KeyCode::HidCode { value } if *value != 0 => Ok(*value),
        KeyCode::HidCode { .. } => Err(ActionError::UnsupportedKey {
            detail: "hardware HID usage code 0 is reserved".to_owned(),
        }),
        _ => Err(ActionError::UnsupportedKey {
            detail: format!(
                "hardware backend currently requires KeyCode::HidCode; full keymap is issue #394: {:?}",
                key.code
            ),
        }),
    }
}
