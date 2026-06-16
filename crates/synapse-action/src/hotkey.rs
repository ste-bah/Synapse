use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use crate::ActionResult;

static OPERATOR_RELEASE_EPOCH: AtomicU64 = AtomicU64::new(0);

pub const OPERATOR_HOTKEY_ENV: &str = "SYNAPSE_OPERATOR_HOTKEY";
pub const OPERATOR_HOTKEY_COMPAT_ENV: &str = "SYNAPSE_MCP_OPERATOR_HOTKEY";
pub const DEFAULT_OPERATOR_HOTKEY: &str = "ctrl+alt+shift+p";

/// Process-global record of how the operator panic hotkey resolved at startup,
/// so liveness/health surfaces can report a degraded kill-switch instead of the
/// failure being invisible. Lock-free: written once during startup, read by
/// `/health`.
static OPERATOR_HOTKEY_STATUS: AtomicU8 = AtomicU8::new(OperatorHotkeyStatus::Unknown as u8);

/// Resolution of the operator panic hotkey for this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorHotkeyStatus {
    /// Startup has not recorded a result yet.
    Unknown = 0,
    /// The global hotkey is registered and the kill-switch is armed.
    Registered = 1,
    /// Disabled by explicit operator environment override.
    DisabledByEnv = 2,
    /// Registration failed (e.g. another process owns the combo). The
    /// kill-switch is NOT armed; this is a degraded safety state.
    Unavailable = 3,
}

/// Records the resolved operator hotkey status for later health readback.
pub fn set_operator_hotkey_status(status: OperatorHotkeyStatus) {
    OPERATOR_HOTKEY_STATUS.store(status as u8, Ordering::Release);
}

/// Reads the resolved operator hotkey status.
#[must_use]
pub fn operator_hotkey_status() -> OperatorHotkeyStatus {
    match OPERATOR_HOTKEY_STATUS.load(Ordering::Acquire) {
        1 => OperatorHotkeyStatus::Registered,
        2 => OperatorHotkeyStatus::DisabledByEnv,
        3 => OperatorHotkeyStatus::Unavailable,
        _ => OperatorHotkeyStatus::Unknown,
    }
}

impl OperatorHotkeyStatus {
    /// Stable lowercase label for health/diagnostics output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Registered => "registered",
            Self::DisabledByEnv => "disabled_by_env",
            Self::Unavailable => "unavailable",
        }
    }
}

#[must_use]
pub fn operator_release_epoch() -> u64 {
    OPERATOR_RELEASE_EPOCH.load(Ordering::Acquire)
}

#[must_use]
pub fn operator_release_requested_since(epoch: u64) -> bool {
    OPERATOR_RELEASE_EPOCH.load(Ordering::Acquire) != epoch
}

pub fn request_release_interrupt() {
    OPERATOR_RELEASE_EPOCH.fetch_add(1, Ordering::AcqRel);
}

#[cfg(windows)]
mod platform {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::{
            Mutex, OnceLock,
            atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
            mpsc,
        },
        thread::{self, JoinHandle},
        time::Duration,
    };

    use synapse_core::error_codes;
    use windows::Win32::{
        Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
        System::{
            LibraryLoader::GetModuleHandleW,
            SystemInformation::GetTickCount64,
            Threading::{
                GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
            },
        },
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey,
                UnregisterHotKey,
            },
            WindowsAndMessaging::{
                CallNextHookEx, DispatchMessageW, HHOOK, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE,
                PM_REMOVE, PeekMessageW, PostThreadMessageW, SetWindowsHookExW, TranslateMessage,
                UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP, WM_QUIT,
                WM_SYSKEYDOWN, WM_SYSKEYUP,
            },
        },
    };

    use crate::{ActionError, ActionResult};

    const HOTKEY_ID: i32 = 0x5359;
    const HOTKEY_WPARAM: usize = 0x5359;
    const STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
    const KEY_DOWN_MASK: i16 = i16::MIN;
    const HOTKEY_SIGNAL_DEBOUNCE_MS: u64 = 750;
    const HOTKEY_HOOK_REARM_INTERVAL_MS: u64 = 500;
    const HOTKEY_MESSAGE_POLL_MS: u64 = 25;
    const VK_CONTROL_CODE: i32 = 0x11;
    const VK_MENU_CODE: i32 = 0x12;
    const VK_SHIFT_CODE: i32 = 0x10;

    static HOTKEY_SIGNAL_SENDER: OnceLock<Mutex<Option<mpsc::Sender<HotkeySignal>>>> =
        OnceLock::new();
    static HOTKEY_KEY_VK: AtomicU32 = AtomicU32::new(0);
    static CHORD_DOWN: AtomicBool = AtomicBool::new(false);
    static LAST_SIGNAL_TICK_MS: AtomicU64 = AtomicU64::new(0);

    pub struct OperatorHotkeyGuard {
        thread_id: u32,
        hook_join: Option<JoinHandle<()>>,
        worker_join: Option<JoinHandle<()>>,
    }

    pub fn install_operator_hotkey<F>(handler: F) -> ActionResult<OperatorHotkeyGuard>
    where
        F: Fn() + Send + 'static,
    {
        let config = HotkeyConfig::from_env()?;
        let (signal_tx, signal_rx) = mpsc::channel::<HotkeySignal>();
        set_signal_sender(Some(signal_tx.clone()))?;
        HOTKEY_KEY_VK.store(config.key_vk, Ordering::Release);
        CHORD_DOWN.store(false, Ordering::Release);
        LAST_SIGNAL_TICK_MS.store(0, Ordering::Release);

        let worker_join = match thread::Builder::new()
            .name("synapse-operator-hotkey-worker".to_owned())
            .spawn(move || run_hotkey_worker(signal_rx, handler))
        {
            Ok(join) => join,
            Err(error) => {
                clear_install_state();
                return Err(ActionError::BackendUnavailable {
                    detail: format!("operator hotkey worker thread spawn failed: {error}"),
                });
            }
        };

        let (ready_tx, ready_rx) = mpsc::channel::<Result<HookReady, String>>();
        let hook_join = match thread::Builder::new()
            .name("synapse-operator-hotkey".to_owned())
            .spawn(move || run_hotkey_thread(&config, &ready_tx))
        {
            Ok(join) => join,
            Err(error) => {
                clear_install_state();
                drop(signal_tx);
                let _join_result = worker_join.join();
                return Err(ActionError::BackendUnavailable {
                    detail: format!("operator hotkey thread spawn failed: {error}"),
                });
            }
        };

        match ready_rx.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Ok(ready)) => Ok(OperatorHotkeyGuard {
                thread_id: ready.thread_id,
                hook_join: Some(hook_join),
                worker_join: Some(worker_join),
            }),
            Ok(Err(detail)) => {
                clear_install_state();
                drop(signal_tx);
                let _hook_join_result = hook_join.join();
                let _worker_join_result = worker_join.join();
                Err(ActionError::BackendUnavailable { detail })
            }
            Err(error) => {
                clear_install_state();
                drop(signal_tx);
                let _hook_join_result = hook_join.join();
                let _worker_join_result = worker_join.join();
                Err(ActionError::BackendUnavailable {
                    detail: format!("operator hotkey registration timed out: {error}"),
                })
            }
        }
    }

    #[derive(Clone, Debug)]
    struct HotkeyConfig {
        key_vk: u32,
        label: String,
    }

    impl HotkeyConfig {
        fn default() -> Self {
            Self {
                key_vk: u32::from(b'P'),
                label: super::DEFAULT_OPERATOR_HOTKEY.to_owned(),
            }
        }

        fn from_env() -> ActionResult<Self> {
            let raw = std::env::var_os(super::OPERATOR_HOTKEY_ENV)
                .or_else(|| std::env::var_os(super::OPERATOR_HOTKEY_COMPAT_ENV));
            let Some(raw) = raw else {
                return Ok(Self::default());
            };
            let value = raw.to_string_lossy();
            Self::parse(&value)
        }

        fn parse(raw: &str) -> ActionResult<Self> {
            let mut ctrl = false;
            let mut alt = false;
            let mut shift = false;
            let mut key_vk = None;

            for token in raw
                .split('+')
                .map(str::trim)
                .filter(|token| !token.is_empty())
            {
                match token.to_ascii_lowercase().as_str() {
                    "ctrl" | "control" => ctrl = true,
                    "alt" => alt = true,
                    "shift" => shift = true,
                    other => {
                        let upper = other.to_ascii_uppercase();
                        let bytes = upper.as_bytes();
                        if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
                            if key_vk.replace(u32::from(bytes[0])).is_some() {
                                return Err(invalid_hotkey(raw, "multiple non-modifier keys"));
                            }
                        } else {
                            return Err(invalid_hotkey(raw, "unsupported token"));
                        }
                    }
                }
            }

            if !ctrl || !alt || !shift {
                return Err(invalid_hotkey(raw, "must include Ctrl+Alt+Shift"));
            }
            let Some(key_vk) = key_vk else {
                return Err(invalid_hotkey(raw, "missing non-modifier key"));
            };
            let key = char::from_u32(key_vk).unwrap_or('P').to_ascii_lowercase();
            Ok(Self {
                key_vk,
                label: format!("ctrl+alt+shift+{key}"),
            })
        }
    }

    #[derive(Clone, Debug)]
    struct HookReady {
        thread_id: u32,
    }

    #[derive(Clone, Debug)]
    struct HotkeySignal {
        source: &'static str,
    }

    struct HookGuard(HHOOK);

    impl Drop for HookGuard {
        fn drop(&mut self) {
            if let Err(error) = unsafe { UnhookWindowsHookEx(self.0) } {
                tracing::warn!(
                    component = "operator_hotkey",
                    detail = %error,
                    "operator low-level keyboard hook unregister failed"
                );
            }
        }
    }

    struct RegisteredHotkeyGuard;

    impl Drop for RegisteredHotkeyGuard {
        fn drop(&mut self) {
            if let Err(error) = unsafe { UnregisterHotKey(None, HOTKEY_ID) } {
                tracing::warn!(
                    component = "operator_hotkey",
                    detail = %error,
                    "operator RegisterHotKey backup unregister failed"
                );
            }
        }
    }

    fn run_hotkey_worker<F>(receiver: mpsc::Receiver<HotkeySignal>, handler: F)
    where
        F: Fn() + Send + 'static,
    {
        let priority_high = set_current_thread_high_priority("worker");
        tracing::info!(
            component = "operator_hotkey",
            worker_thread_priority_high = priority_high,
            "operator hotkey worker thread started"
        );
        for signal in receiver {
            let result = catch_unwind(AssertUnwindSafe(&handler));
            if result.is_err() {
                tracing::error!(
                    code = error_codes::SAFETY_OPERATOR_HOTKEY_FIRED,
                    component = "operator_hotkey",
                    source = signal.source,
                    result = "handler_panic",
                    "operator hotkey handler panicked"
                );
            }
        }
    }

    fn run_hotkey_thread(config: &HotkeyConfig, ready: &mpsc::Sender<Result<HookReady, String>>) {
        let thread_id = unsafe { GetCurrentThreadId() };
        let priority_high = set_current_thread_high_priority("hook");
        let mut msg = MSG::default();
        unsafe {
            let _queue_created = PeekMessageW(&raw mut msg, None, 0, 0, PM_NOREMOVE);
        }

        let module = match unsafe { GetModuleHandleW(None) } {
            Ok(module) => module,
            Err(error) => {
                let _send_result = ready.send(Err(format!(
                    "GetModuleHandleW failed for operator hotkey hook: {error}"
                )));
                return;
            }
        };
        let mut hook_guard = match install_keyboard_hook(module, config) {
            Ok(hook_guard) => hook_guard,
            Err(error) => {
                let _send_result = ready.send(Err(error));
                return;
            }
        };

        let registered_hotkey_guard = match unsafe {
            RegisterHotKey(
                None,
                HOTKEY_ID,
                MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
                config.key_vk,
            )
        } {
            Ok(()) => Some(RegisteredHotkeyGuard),
            Err(error) => {
                tracing::warn!(
                    component = "operator_hotkey",
                    hotkey = config.label.as_str(),
                    detail = %error,
                    "operator RegisterHotKey backup unavailable; WH_KEYBOARD_LL hook remains armed"
                );
                None
            }
        };

        let _send_result = ready.send(Ok(HookReady { thread_id }));
        tracing::info!(
            component = "operator_hotkey",
            hotkey = config.label.as_str(),
            low_level_hook_installed = true,
            rearm_interval_ms = HOTKEY_HOOK_REARM_INTERVAL_MS,
            register_hotkey_backup = registered_hotkey_guard.is_some(),
            hook_thread_priority_high = priority_high,
            "operator panic hotkey armed"
        );

        let mut last_rearm_ms = unsafe { GetTickCount64() };
        loop {
            while unsafe { PeekMessageW(&raw mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                if msg.message == WM_QUIT {
                    return;
                }

                if msg.message == WM_HOTKEY && msg.wParam.0 == HOTKEY_WPARAM {
                    maybe_send_hotkey_signal("register_hotkey_backup");
                    continue;
                }

                unsafe {
                    let _translated = TranslateMessage(&raw const msg);
                    let _dispatch_result = DispatchMessageW(&raw const msg);
                }
            }

            let now_ms = unsafe { GetTickCount64() };
            if now_ms.saturating_sub(last_rearm_ms) >= HOTKEY_HOOK_REARM_INTERVAL_MS {
                match install_keyboard_hook(module, config) {
                    Ok(new_hook_guard) => {
                        let old_hook_guard = std::mem::replace(&mut hook_guard, new_hook_guard);
                        drop(old_hook_guard);
                        last_rearm_ms = now_ms;
                        tracing::debug!(
                            component = "operator_hotkey",
                            hotkey = config.label.as_str(),
                            "operator low-level keyboard hook re-armed at hook-chain head"
                        );
                    }
                    Err(error) => {
                        last_rearm_ms = now_ms;
                        tracing::error!(
                            component = "operator_hotkey",
                            hotkey = config.label.as_str(),
                            detail = error,
                            "operator low-level keyboard hook re-arm failed"
                        );
                    }
                }
            }

            thread::sleep(Duration::from_millis(HOTKEY_MESSAGE_POLL_MS));
        }
    }

    fn install_keyboard_hook(
        module: windows::Win32::Foundation::HMODULE,
        config: &HotkeyConfig,
    ) -> Result<HookGuard, String> {
        match unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_hook_proc),
                Some(HINSTANCE(module.0)),
                0,
            )
        } {
            Ok(hook) => Ok(HookGuard(hook)),
            Err(error) => Err(format!(
                "SetWindowsHookExW(WH_KEYBOARD_LL) failed for {}: {error}",
                config.label
            )),
        }
    }

    impl Drop for OperatorHotkeyGuard {
        fn drop(&mut self) {
            clear_install_state();
            if let Err(error) =
                unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }
            {
                tracing::warn!(
                    component = "operator_hotkey",
                    detail = %error,
                    "operator hotkey stop signal failed"
                );
            }
            if let Some(join) = self.hook_join.take()
                && join.join().is_err()
            {
                tracing::warn!(
                    component = "operator_hotkey",
                    "operator hotkey hook thread join failed"
                );
            }
            if let Some(join) = self.worker_join.take()
                && join.join().is_err()
            {
                tracing::warn!(
                    component = "operator_hotkey",
                    "operator hotkey worker thread join failed"
                );
            }
        }
    }

    unsafe extern "system" fn keyboard_hook_proc(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if code >= 0 {
            let message = u32::try_from(wparam.0).unwrap_or(0);
            if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN) {
                let data = unsafe { *(lparam.0 as *const KBDLLHOOKSTRUCT) };
                if data.vkCode == HOTKEY_KEY_VK.load(Ordering::Acquire)
                    && modifiers_down()
                    && !CHORD_DOWN.swap(true, Ordering::AcqRel)
                {
                    maybe_send_hotkey_signal("wh_keyboard_ll");
                }
            } else if matches!(message, WM_KEYUP | WM_SYSKEYUP) {
                let data = unsafe { *(lparam.0 as *const KBDLLHOOKSTRUCT) };
                if data.vkCode == HOTKEY_KEY_VK.load(Ordering::Acquire) {
                    CHORD_DOWN.store(false, Ordering::Release);
                }
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    fn modifiers_down() -> bool {
        key_down(VK_CONTROL_CODE) && key_down(VK_MENU_CODE) && key_down(VK_SHIFT_CODE)
    }

    fn key_down(vk: i32) -> bool {
        (unsafe { GetAsyncKeyState(vk) } & KEY_DOWN_MASK) != 0
    }

    fn maybe_send_hotkey_signal(source: &'static str) {
        let now_ms = unsafe { GetTickCount64() };
        if !claim_hotkey_signal_slot(now_ms, source) {
            return;
        }
        super::request_release_interrupt();
        let sender = match hotkey_signal_sender().lock() {
            Ok(guard) => guard.clone(),
            Err(_error) => {
                tracing::error!(
                    code = error_codes::SAFETY_OPERATOR_HOTKEY_FIRED,
                    component = "operator_hotkey",
                    source,
                    result = "sender_lock_poisoned",
                    "operator hotkey signal sender lock poisoned"
                );
                None
            }
        };
        let Some(sender) = sender else {
            tracing::error!(
                code = error_codes::SAFETY_OPERATOR_HOTKEY_FIRED,
                component = "operator_hotkey",
                source,
                result = "sender_missing",
                "operator hotkey fired but worker sender is missing"
            );
            return;
        };
        if let Err(error) = sender.send(HotkeySignal { source }) {
            tracing::error!(
                code = error_codes::SAFETY_OPERATOR_HOTKEY_FIRED,
                component = "operator_hotkey",
                source,
                detail = %error,
                result = "send_failed",
                "operator hotkey fired but worker dispatch failed"
            );
        }
    }

    fn claim_hotkey_signal_slot(now_ms: u64, source: &'static str) -> bool {
        let mut previous = LAST_SIGNAL_TICK_MS.load(Ordering::Acquire);
        loop {
            if previous != 0 && now_ms >= previous && now_ms - previous < HOTKEY_SIGNAL_DEBOUNCE_MS
            {
                tracing::debug!(
                    component = "operator_hotkey",
                    source,
                    elapsed_ms = now_ms - previous,
                    debounce_ms = HOTKEY_SIGNAL_DEBOUNCE_MS,
                    "operator hotkey duplicate signal ignored"
                );
                return false;
            }
            match LAST_SIGNAL_TICK_MS.compare_exchange_weak(
                previous,
                now_ms,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => previous = actual,
            }
        }
    }

    fn hotkey_signal_sender() -> &'static Mutex<Option<mpsc::Sender<HotkeySignal>>> {
        HOTKEY_SIGNAL_SENDER.get_or_init(|| Mutex::new(None))
    }

    fn set_signal_sender(sender: Option<mpsc::Sender<HotkeySignal>>) -> ActionResult<()> {
        let mut guard =
            hotkey_signal_sender()
                .lock()
                .map_err(|_error| ActionError::BackendUnavailable {
                    detail: "operator hotkey sender lock poisoned".to_owned(),
                })?;
        if guard.is_some() && sender.is_some() {
            return Err(ActionError::BackendUnavailable {
                detail: "operator hotkey is already installed in this process".to_owned(),
            });
        }
        *guard = sender;
        drop(guard);
        Ok(())
    }

    fn clear_install_state() {
        if let Ok(mut guard) = hotkey_signal_sender().lock() {
            *guard = None;
        }
        HOTKEY_KEY_VK.store(0, Ordering::Release);
        CHORD_DOWN.store(false, Ordering::Release);
        LAST_SIGNAL_TICK_MS.store(0, Ordering::Release);
    }

    fn set_current_thread_high_priority(role: &'static str) -> bool {
        let ok = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) }.is_ok();
        if !ok {
            tracing::warn!(
                component = "operator_hotkey",
                role,
                "failed to raise operator hotkey thread priority"
            );
        }
        ok
    }

    fn invalid_hotkey(raw: &str, reason: &str) -> ActionError {
        ActionError::BackendUnavailable {
            detail: format!(
                "{} / {} must be Ctrl+Alt+Shift+<A-Z|0-9>; got {:?}: {reason}",
                super::OPERATOR_HOTKEY_ENV,
                super::OPERATOR_HOTKEY_COMPAT_ENV,
                raw
            ),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn hotkey_config_accepts_distinctive_ctrl_alt_shift_key() {
            let config = match HotkeyConfig::parse("Ctrl+Alt+Shift+K") {
                Ok(config) => config,
                Err(error) => panic!("valid hotkey rejected: {error}"),
            };
            assert_eq!(config.key_vk, u32::from(b'K'));
            assert_eq!(config.label, "ctrl+alt+shift+k");
        }

        #[test]
        fn hotkey_config_rejects_missing_required_modifiers() {
            let error = match HotkeyConfig::parse("Ctrl+K") {
                Ok(config) => panic!("missing modifiers accepted: {config:?}"),
                Err(error) => error,
            };
            assert!(
                error.to_string().contains("must include Ctrl+Alt+Shift"),
                "unexpected error: {error}"
            );
        }

        #[test]
        fn hotkey_config_rejects_unsupported_keys() {
            let error = match HotkeyConfig::parse("Ctrl+Alt+Shift+F12") {
                Ok(config) => panic!("unsupported key accepted: {config:?}"),
                Err(error) => error,
            };
            assert!(
                error.to_string().contains("unsupported token"),
                "unexpected error: {error}"
            );
        }

        #[test]
        fn hotkey_signal_debounce_suppresses_backup_duplicate() {
            LAST_SIGNAL_TICK_MS.store(0, Ordering::Release);
            assert!(claim_hotkey_signal_slot(10_000, "wh_keyboard_ll"));
            assert!(!claim_hotkey_signal_slot(10_010, "register_hotkey_backup"));
            assert!(claim_hotkey_signal_slot(
                10_000 + HOTKEY_SIGNAL_DEBOUNCE_MS,
                "register_hotkey_backup"
            ));
            LAST_SIGNAL_TICK_MS.store(0, Ordering::Release);
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use crate::ActionResult;

    pub struct OperatorHotkeyGuard;

    #[allow(clippy::unnecessary_wraps)]
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
/// Returns a [`crate::ActionError`] when the platform hotkey thread or
/// low-level keyboard hook cannot start.
pub fn install_operator_hotkey<F>(handler: F) -> ActionResult<OperatorHotkeyGuard>
where
    F: Fn() + Send + 'static,
{
    platform::install_operator_hotkey(handler)
}
