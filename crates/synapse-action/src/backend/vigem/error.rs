#[cfg(windows)]
use std::time::{Duration, Instant};

use synapse_core::{Action, PadId};

use crate::ActionError;

#[cfg(windows)]
const VIGEM_UPDATE_RETRY_TIMEOUT: Duration = Duration::from_millis(250);
#[cfg(windows)]
const VIGEM_UPDATE_RETRY_INTERVAL: Duration = Duration::from_millis(2);
#[cfg(windows)]
const ERROR_NO_MORE_ITEMS: u32 = 259;

#[cfg(windows)]
pub(super) fn retry_vigem_update<F>(context: &'static str, mut update: F) -> Result<(), ActionError>
where
    F: FnMut() -> Result<(), vigem_client::Error>,
{
    let started = Instant::now();
    let mut attempts = 0_u32;
    loop {
        attempts = attempts.saturating_add(1);
        match update() {
            Ok(()) => {
                if attempts > 1 {
                    tracing::debug!(
                        backend = "vigem",
                        context,
                        attempts,
                        elapsed_ms = started.elapsed().as_millis(),
                        "ViGEm report update succeeded after retry"
                    );
                }
                return Ok(());
            }
            Err(error)
                if is_transient_vigem_update_error(error)
                    && started.elapsed() < VIGEM_UPDATE_RETRY_TIMEOUT =>
            {
                std::thread::sleep(VIGEM_UPDATE_RETRY_INTERVAL);
            }
            Err(error) => return Err(map_vigem_error(context, error)),
        }
    }
}

#[cfg(windows)]
const fn is_transient_vigem_update_error(error: vigem_client::Error) -> bool {
    matches!(
        error,
        vigem_client::Error::TargetNotReady | vigem_client::Error::WinError(ERROR_NO_MORE_ITEMS)
    )
}

#[cfg(windows)]
pub(super) fn add_pad_context(pad: PadId, error: ActionError) -> ActionError {
    match error {
        ActionError::VigemNotInstalled { detail } => ActionError::VigemNotInstalled {
            detail: format!("pad={pad} {detail}"),
        },
        ActionError::VigemPluginFailed { detail } => ActionError::VigemPluginFailed {
            detail: format!("pad={pad} {detail}"),
        },
        other => other,
    }
}

#[cfg(windows)]
pub(super) fn map_vigem_error(context: &'static str, error: vigem_client::Error) -> ActionError {
    match error {
        vigem_client::Error::BusNotFound => ActionError::VigemNotInstalled {
            detail: format!("backend=vigem context={context} driver=ViGEmBus error={error}"),
        },
        vigem_client::Error::BusAccessFailed(code) => ActionError::VigemPluginFailed {
            detail: format!(
                "backend=vigem context={context} driver=ViGEmBus access_failed_win32={code}"
            ),
        },
        vigem_client::Error::WinError(code) => ActionError::VigemPluginFailed {
            detail: format!("backend=vigem context={context} win32={code}"),
        },
        _ => ActionError::VigemPluginFailed {
            detail: format!("backend=vigem context={context} error={error}"),
        },
    }
}

#[cfg(windows)]
pub(super) fn routed_non_gamepad_error(action: &Action) -> ActionError {
    ActionError::BackendUnavailable {
        detail: format!(
            "backend=vigem reason=routed non-gamepad action through gamepad backend action_kind={}",
            action_kind(action)
        ),
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
