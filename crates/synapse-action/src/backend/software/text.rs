use std::thread;

use synapse_core::KeystrokeDynamics;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
};

use super::{
    input::{keyboard_input, send_input_batch, virtual_keyboard_input},
    utils::sleep_ms,
};
use crate::ActionError;
use crate::backend::text_dispatch::{TextDispatchInput, text_dispatch_plan};

#[tracing::instrument(skip_all, fields(action_kind = "software_type_text"))]
pub(super) fn type_text(text: &str, dynamics: &KeystrokeDynamics) -> Result<(), ActionError> {
    for step in text_dispatch_plan(text, dynamics) {
        sleep_ms(step.iki_ms_before);
        for input in step.inputs {
            send_text_input(input)?;
            thread::yield_now();
        }
        thread::yield_now();
    }
    Ok(())
}

fn send_text_input(input: TextDispatchInput) -> Result<(), ActionError> {
    match input {
        TextDispatchInput::UnicodeUnit(unit) => send_unicode_unit(unit),
        TextDispatchInput::VirtualKey(vkey) => {
            send_virtual_key(VIRTUAL_KEY(vkey), "text virtual key")
        }
    }
}

fn send_unicode_unit(unit: u16) -> Result<(), ActionError> {
    let inputs = [
        keyboard_input(unit, KEYEVENTF_UNICODE),
        keyboard_input(unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
    ];
    send_input_batch(&inputs, "unicode text character")
}

fn send_virtual_key(vkey: VIRTUAL_KEY, detail: &'static str) -> Result<(), ActionError> {
    let inputs = [
        virtual_keyboard_input(vkey, KEYBD_EVENT_FLAGS(0)),
        virtual_keyboard_input(vkey, KEYEVENTF_KEYUP),
    ];
    send_input_batch(&inputs, detail)
}
