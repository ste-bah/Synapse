use std::{
    thread,
    time::{Duration, Instant},
};

use enigo::{Enigo, Settings};

use crate::ActionError;

pub(super) fn enigo() -> Result<Enigo, ActionError> {
    Enigo::new(&Settings::default()).map_err(|err| ActionError::BackendUnavailable {
        detail: format!("failed to initialize enigo: {err}"),
    })
}

pub(super) fn enigo_preserving_held_keys() -> Result<Enigo, ActionError> {
    Enigo::new(&Settings {
        release_keys_when_dropped: false,
        ..Settings::default()
    })
    .map_err(|err| ActionError::BackendUnavailable {
        detail: format!("failed to initialize enigo: {err}"),
    })
}

pub(super) fn enigo_error(context: &'static str) -> impl FnOnce(enigo::InputError) -> ActionError {
    move |err| ActionError::BackendUnavailable {
        detail: format!("{context}: {err}"),
    }
}

pub(super) fn sleep_ms(milliseconds: u32) -> bool {
    sleep_ms_since(milliseconds, crate::hotkey::operator_release_epoch())
}

pub(super) fn sleep_ms_since(milliseconds: u32, epoch: u64) -> bool {
    if milliseconds == 0 {
        return crate::hotkey::operator_release_requested_since(epoch);
    }
    let deadline = Instant::now() + Duration::from_millis(u64::from(milliseconds));
    loop {
        if crate::hotkey::operator_release_requested_since(epoch) {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        thread::sleep((deadline - now).min(Duration::from_millis(1)));
    }
}
