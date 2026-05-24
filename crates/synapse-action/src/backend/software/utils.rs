use std::{thread, time::Duration};

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

pub(super) fn sleep_ms(milliseconds: u32) {
    if milliseconds > 0 {
        thread::sleep(Duration::from_millis(u64::from(milliseconds)));
    }
}
