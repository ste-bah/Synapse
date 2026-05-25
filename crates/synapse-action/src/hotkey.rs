use std::sync::atomic::{AtomicU64, Ordering};

use crate::ActionResult;

static OPERATOR_RELEASE_EPOCH: AtomicU64 = AtomicU64::new(0);

#[must_use]
pub fn operator_release_epoch() -> u64 {
    OPERATOR_RELEASE_EPOCH.load(Ordering::Acquire)
}

#[must_use]
pub fn operator_release_requested_since(epoch: u64) -> bool {
    OPERATOR_RELEASE_EPOCH.load(Ordering::Acquire) != epoch
}

fn request_operator_release_interrupt() {
    OPERATOR_RELEASE_EPOCH.fetch_add(1, Ordering::AcqRel);
}

#[cfg(windows)]
mod platform {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::mpsc,
        thread::{self, JoinHandle},
        time::Duration,
    };

    use synapse_core::error_codes;
    use windows::Win32::{
        Foundation::{LPARAM, WPARAM},
        System::Threading::GetCurrentThreadId,
        UI::{
            Input::KeyboardAndMouse::{
                MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey,
            },
            WindowsAndMessaging::{
                DispatchMessageW, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
                TranslateMessage, WM_HOTKEY, WM_QUIT,
            },
        },
    };

    use crate::{ActionError, ActionResult};

    const HOTKEY_ID: i32 = 0x5359;
    const HOTKEY_WPARAM: usize = 0x5359;
    const HOTKEY_VK: u32 = b'P' as u32;
    const STARTUP_TIMEOUT: Duration = Duration::from_secs(2);

    pub struct OperatorHotkeyGuard {
        thread_id: u32,
        join: Option<JoinHandle<()>>,
    }

    pub fn install_operator_hotkey<F>(handler: F) -> ActionResult<OperatorHotkeyGuard>
    where
        F: Fn() + Send + 'static,
    {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
        let join = thread::Builder::new()
            .name("synapse-operator-hotkey".to_owned())
            .spawn(move || run_hotkey_thread(handler, &ready_tx))
            .map_err(|error| ActionError::BackendUnavailable {
                detail: format!("operator hotkey thread spawn failed: {error}"),
            })?;

        match ready_rx.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Ok(thread_id)) => Ok(OperatorHotkeyGuard {
                thread_id,
                join: Some(join),
            }),
            Ok(Err(detail)) => {
                let _join_result = join.join();
                Err(ActionError::BackendUnavailable { detail })
            }
            Err(error) => {
                let _join_result = join.join();
                Err(ActionError::BackendUnavailable {
                    detail: format!("operator hotkey registration timed out: {error}"),
                })
            }
        }
    }

    fn run_hotkey_thread<F>(handler: F, ready: &mpsc::Sender<Result<u32, String>>)
    where
        F: Fn() + Send + 'static,
    {
        let thread_id = unsafe { GetCurrentThreadId() };
        let mut msg = MSG::default();
        unsafe {
            let _queue_created = PeekMessageW(&raw mut msg, None, 0, 0, PM_NOREMOVE);
        }

        let modifiers = MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT;
        match unsafe { RegisterHotKey(None, HOTKEY_ID, modifiers, HOTKEY_VK) } {
            Ok(()) => {
                let _send_result = ready.send(Ok(thread_id));
                tracing::info!(
                    component = "operator_hotkey",
                    hotkey = "ctrl+alt+shift+p",
                    "operator hotkey registered"
                );
            }
            Err(error) => {
                let _send_result = ready.send(Err(format!(
                    "RegisterHotKey Ctrl+Alt+Shift+P failed: {error}"
                )));
                return;
            }
        }

        loop {
            let received = unsafe { GetMessageW(&raw mut msg, None, 0, 0) };
            if received.0 == -1 {
                tracing::error!(
                    component = "operator_hotkey",
                    "operator hotkey message loop failed"
                );
                break;
            }
            if !received.as_bool() {
                break;
            }

            if msg.message == WM_HOTKEY && msg.wParam.0 == HOTKEY_WPARAM {
                super::request_operator_release_interrupt();
                let result = catch_unwind(AssertUnwindSafe(&handler));
                if result.is_err() {
                    tracing::error!(
                        code = error_codes::SAFETY_OPERATOR_HOTKEY_FIRED,
                        component = "operator_hotkey",
                        result = "handler_panic",
                        "operator hotkey handler panicked"
                    );
                }
                continue;
            }

            unsafe {
                let _translated = TranslateMessage(&raw const msg);
                let _dispatch_result = DispatchMessageW(&raw const msg);
            }
        }

        if let Err(error) = unsafe { UnregisterHotKey(None, HOTKEY_ID) } {
            tracing::warn!(
                component = "operator_hotkey",
                hotkey = "ctrl+alt+shift+p",
                detail = %error,
                "operator hotkey unregister failed"
            );
        }
    }

    impl Drop for OperatorHotkeyGuard {
        fn drop(&mut self) {
            if let Err(error) =
                unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }
            {
                tracing::warn!(
                    component = "operator_hotkey",
                    detail = %error,
                    "operator hotkey stop signal failed"
                );
            }
            if let Some(join) = self.join.take()
                && join.join().is_err()
            {
                tracing::warn!(
                    component = "operator_hotkey",
                    "operator hotkey thread join failed"
                );
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use crate::ActionResult;

    pub struct OperatorHotkeyGuard;

    pub fn install_operator_hotkey<F>(_handler: F) -> ActionResult<OperatorHotkeyGuard>
    where
        F: Fn() + Send + 'static,
    {
        tracing::warn!(
            component = "operator_hotkey",
            "operator hotkey is only registered on Windows"
        );
        Ok(OperatorHotkeyGuard)
    }
}

pub use platform::OperatorHotkeyGuard;

/// Registers the system-wide operator panic hotkey.
///
/// # Errors
///
/// Returns a [`crate::ActionError`] when the platform hotkey thread cannot
/// start or the Windows `RegisterHotKey` call fails.
pub fn install_operator_hotkey<F>(handler: F) -> ActionResult<OperatorHotkeyGuard>
where
    F: Fn() + Send + 'static,
{
    platform::install_operator_hotkey(handler)
}
