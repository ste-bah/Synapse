#![allow(unsafe_code)]

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use synapse_core::{AccessibleSubtree, ElementId, ForegroundContext, Point, error_codes};
use thiserror::Error;
use tokio::{net::TcpStream, sync::mpsc::UnboundedSender, time::timeout};

#[cfg(windows)]
pub use uiautomation;
#[cfg(windows)]
pub use uiautomation::UIElement;

#[cfg(not(windows))]
#[derive(Clone, Debug)]
pub struct UIElement;

pub type A11yResult<T> = Result<T, A11yError>;
pub type AccessibleEventSender = UnboundedSender<AccessibleEvent>;

#[derive(Debug, Error)]
pub enum A11yError {
    #[error("Windows UI Automation is not available: {detail}")]
    NotAvailable { detail: String },
    #[error("no foreground window is available: {detail}")]
    NoForeground { detail: String },
    #[error("UI Automation element is stale: {detail}")]
    ElementStale { detail: String },
    #[error("Chromium DevTools Protocol is unreachable: {detail}")]
    CdpUnreachable { detail: String },
    #[error("invalid element id: {detail}")]
    InvalidElementId { detail: String },
    #[error("accessibility backend failed: {detail}")]
    Internal { detail: String },
}

impl A11yError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotAvailable { .. } => error_codes::A11Y_NOT_AVAILABLE,
            Self::NoForeground { .. } => error_codes::A11Y_NO_FOREGROUND,
            Self::ElementStale { .. } => error_codes::A11Y_ELEMENT_STALE,
            Self::CdpUnreachable { .. } => error_codes::A11Y_CDP_UNREACHABLE,
            Self::InvalidElementId { .. } | Self::Internal { .. } => error_codes::OBSERVE_INTERNAL,
        }
    }

    #[must_use]
    pub fn not_available(detail: impl Into<String>) -> Self {
        Self::NotAvailable {
            detail: detail.into(),
        }
    }

    #[must_use]
    pub fn internal(detail: impl Into<String>) -> Self {
        Self::Internal {
            detail: detail.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessibleEvent {
    pub seq: u64,
    pub at_ms: u64,
    pub window_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<ElementId>,
    pub kind: AccessibleEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibleEventKind {
    ForegroundChanged,
    FocusChanged,
    ValueChanged,
    NameChanged,
    ElementAppeared,
    ElementDisappeared,
    SelectionChanged,
    MenuStart,
    MenuEnd,
    Alert,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WinEventHookReadback {
    pub thread_id: u32,
    pub apartment: ComApartmentKind,
    pub hook_count: usize,
    pub event_ids: Vec<u32>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComApartmentKind {
    Sta,
    Mta,
    Neutral,
    MainSta,
    Unknown,
    Unsupported,
}

impl ComApartmentKind {
    #[must_use]
    pub const fn is_sta_family(self) -> bool {
        matches!(self, Self::Sta | Self::MainSta)
    }
}

pub struct WinEventSubscription {
    inner: platform::WinEventSubscription,
}

impl WinEventSubscription {
    #[allow(clippy::missing_const_for_fn)]
    #[must_use]
    pub fn readback(&self) -> &WinEventHookReadback {
        self.inner.readback()
    }
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
struct EventKey {
    window_id: i64,
    kind: AccessibleEventKind,
    element_id_hash: u64,
}

impl EventKey {
    fn from_event(event: &AccessibleEvent) -> Self {
        use std::{
            collections::hash_map::DefaultHasher,
            hash::{Hash, Hasher},
        };

        let mut hasher = DefaultHasher::new();
        event.element_id.hash(&mut hasher);
        Self {
            window_id: event.window_id,
            kind: event.kind,
            element_id_hash: hasher.finish(),
        }
    }
}

#[must_use]
pub fn coalesce_events<I>(events: I, window: Duration) -> Vec<AccessibleEvent>
where
    I: IntoIterator<Item = AccessibleEvent>,
{
    let window_ms = u64::try_from(window.as_millis()).unwrap_or(u64::MAX);
    let mut output = Vec::new();
    let mut pending: Option<AccessibleEvent> = None;

    for event in events {
        let Some(current) = pending.take() else {
            pending = Some(event);
            continue;
        };

        let same_key = EventKey::from_event(&current) == EventKey::from_event(&event);
        let within_window = event.at_ms.saturating_sub(current.at_ms) < window_ms;

        if !(same_key && within_window) {
            output.push(current);
        }
        pending = Some(event);
    }

    if let Some(event) = pending {
        output.push(event);
    }

    output
}

#[must_use]
pub fn debounce_value_changes<I>(events: I, window: Duration) -> Vec<AccessibleEvent>
where
    I: IntoIterator<Item = AccessibleEvent>,
{
    let window_ms = u64::try_from(window.as_millis()).unwrap_or(u64::MAX);
    let mut output = Vec::new();
    let mut last_emitted = HashMap::<EventKey, u64>::new();
    let mut pending = HashMap::<EventKey, AccessibleEvent>::new();

    for event in events {
        if event.kind == AccessibleEventKind::FocusChanged {
            flush_pending(&mut pending, &mut output);
            output.push(event);
            continue;
        }

        if event.kind != AccessibleEventKind::ValueChanged {
            output.push(event);
            continue;
        }

        let key = EventKey::from_event(&event);
        match last_emitted.get(&key).copied() {
            Some(last_at) if event.at_ms.saturating_sub(last_at) < window_ms => {
                pending.insert(key, event);
            }
            _ => {
                pending.remove(&key);
                last_emitted.insert(key, event.at_ms);
                output.push(event);
            }
        }
    }

    flush_pending(&mut pending, &mut output);
    output.sort_by_key(|event| (event.at_ms, event.seq));
    output
}

fn flush_pending(
    pending: &mut HashMap<EventKey, AccessibleEvent>,
    output: &mut Vec<AccessibleEvent>,
) {
    output.extend(pending.drain().map(|(_key, event)| event));
}

#[must_use]
pub fn runtime_id_hex(runtime_id: &[i32]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(runtime_id.len().saturating_mul(8));
    for part in runtime_id {
        if write!(&mut output, "{:08x}", part.cast_unsigned()).is_err() {
            break;
        }
    }
    output
}

/// Returns the UIA element for the current foreground window.
///
/// # Errors
///
/// Returns `A11Y_NO_FOREGROUND` when Windows has no foreground HWND, or
/// `A11Y_NOT_AVAILABLE` on non-Windows platforms.
pub fn focused_window() -> A11yResult<UIElement> {
    platform::focused_window()
}

/// Returns a top-level UIA window for a native HWND.
///
/// # Errors
///
/// Returns `A11Y_NO_FOREGROUND` when the HWND is invalid, or
/// `A11Y_NOT_AVAILABLE` on non-Windows platforms.
pub fn window_from_hwnd(hwnd: i64) -> A11yResult<UIElement> {
    platform::window_from_hwnd(hwnd)
}

/// Returns the top-level UIA window for a process id.
///
/// # Errors
///
/// Returns `A11Y_NO_FOREGROUND` when no visible window exists for the pid, or
/// `A11Y_NOT_AVAILABLE` on non-Windows platforms.
pub fn window_for_process(pid: u32) -> A11yResult<UIElement> {
    platform::window_for_process(pid)
}

/// Returns foreground-window process, title, bounds, and display metadata.
///
/// # Errors
///
/// Returns a structured UIA error when the HWND cannot be inspected, or
/// `A11Y_NOT_AVAILABLE` on non-Windows platforms.
pub fn foreground_context(hwnd: i64) -> A11yResult<ForegroundContext> {
    platform::foreground_context(hwnd)
}

/// Returns the currently focused UIA element with cached basic properties.
///
/// # Errors
///
/// Returns a structured UIA error when the focused element cannot be resolved,
/// or `A11Y_NOT_AVAILABLE` on non-Windows platforms.
pub fn focused_element() -> A11yResult<UIElement> {
    platform::focused_element()
}

/// Returns the UIA element at a screen-space point.
///
/// # Errors
///
/// Returns a structured UIA error when hit testing fails, or
/// `A11Y_NOT_AVAILABLE` on non-Windows platforms.
pub fn element_from_point(point: Point) -> A11yResult<UIElement> {
    platform::element_from_point(point)
}

/// Captures a depth-limited accessible subtree from a UIA root element.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the root no longer produces a node, a
/// structured UIA error for OS failures, or `A11Y_NOT_AVAILABLE` on
/// non-Windows platforms.
pub fn snapshot(root: &UIElement, depth: u32) -> A11yResult<AccessibleSubtree> {
    platform::snapshot(root, depth)
}

/// Re-resolves a composite Synapse element id back to a live UIA element.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the runtime id cannot be found under the
/// HWND, `OBSERVE_INTERNAL` for invalid ids, or `A11Y_NOT_AVAILABLE` on
/// non-Windows platforms.
pub fn re_resolve(id: &ElementId) -> A11yResult<UIElement> {
    platform::re_resolve(id)
}

/// Starts the dedicated `WinEvent` hook thread and marshals events into `sender`.
///
/// # Errors
///
/// Returns a structured UIA error when the hook thread cannot initialize, or
/// `A11Y_NOT_AVAILABLE` on non-Windows platforms.
pub fn subscribe_win_events(sender: AccessibleEventSender) -> A11yResult<WinEventSubscription> {
    let inner = platform::subscribe_win_events(sender)?;
    Ok(WinEventSubscription { inner })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CdpDiagnostics {
    pub process_name: String,
    pub status: CdpStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<CdpCapability>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CdpStatus {
    Ok,
    NotChromium,
    Unreachable,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CdpCapability {
    DomSnapshot,
    AccessibilityFullAxTree,
    DomQuerySelector,
    PageCaptureScreenshot,
}

#[must_use]
pub fn cdp_capabilities() -> Vec<CdpCapability> {
    vec![
        CdpCapability::DomSnapshot,
        CdpCapability::AccessibilityFullAxTree,
        CdpCapability::DomQuerySelector,
        CdpCapability::PageCaptureScreenshot,
    ]
}

#[must_use]
pub fn is_chromium_family(process_name: &str) -> bool {
    let lower = process_name.to_ascii_lowercase();
    [
        "chrome.exe",
        "chromium.exe",
        "msedge.exe",
        "brave.exe",
        "vivaldi.exe",
        "opera.exe",
        "chrome",
        "chromium",
        "msedge",
        "brave",
        "vivaldi",
        "opera",
    ]
    .iter()
    .any(|candidate| lower.ends_with(candidate))
}

pub async fn probe_chromium_cdp(
    process_name: &str,
    ports: &[u16],
    connect_timeout: Duration,
) -> CdpDiagnostics {
    if !is_chromium_family(process_name) {
        return CdpDiagnostics {
            process_name: process_name.to_owned(),
            status: CdpStatus::NotChromium,
            endpoint: None,
            reason_code: None,
            capabilities: Vec::new(),
        };
    }

    for port in ports {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), *port);
        if timeout(connect_timeout, TcpStream::connect(addr))
            .await
            .is_ok_and(|result| result.is_ok())
        {
            return CdpDiagnostics {
                process_name: process_name.to_owned(),
                status: CdpStatus::Ok,
                endpoint: Some(format!("http://127.0.0.1:{port}")),
                reason_code: None,
                capabilities: cdp_capabilities(),
            };
        }
    }

    CdpDiagnostics {
        process_name: process_name.to_owned(),
        status: CdpStatus::Unreachable,
        endpoint: None,
        reason_code: Some(error_codes::A11Y_CDP_UNREACHABLE.to_owned()),
        capabilities: Vec::new(),
    }
}

#[cfg(windows)]
#[derive(Debug)]
pub struct CdpAttachment {
    pub browser: chromiumoxide::Browser,
    pub handler: chromiumoxide::Handler,
    pub endpoint: String,
}

/// Attaches a `chromiumoxide` browser client to a reachable CDP endpoint.
///
/// # Errors
///
/// Returns `A11Y_CDP_UNREACHABLE` when `chromiumoxide` cannot connect to the
/// supplied endpoint.
#[cfg(windows)]
pub async fn attach_chromiumoxide(endpoint: &str) -> A11yResult<CdpAttachment> {
    let (browser, handler) = chromiumoxide::Browser::connect(endpoint)
        .await
        .map_err(|err| A11yError::CdpUnreachable {
            detail: err.to_string(),
        })?;
    Ok(CdpAttachment {
        browser,
        handler,
        endpoint: endpoint.to_owned(),
    })
}

#[cfg(windows)]
mod platform {
    use std::{
        ffi::c_void,
        path::Path,
        sync::{
            Arc, Mutex, OnceLock,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };

    use synapse_core::{
        AccessibleNode, AccessibleSubtree, ElementId, ForegroundContext, Point, Rect, UiaPattern,
        element_id,
    };
    use tokio::sync::mpsc::UnboundedSender;
    use uiautomation::{
        UIAutomation, UIElement,
        core::UICacheRequest,
        types::{ControlType, ElementMode, Handle, Point as UiaPoint, TreeScope, UIProperty},
        variants::{Value, Variant},
    };
    use windows::{
        Win32::{
            Foundation::{CloseHandle, HWND, LPARAM, RECT, WPARAM},
            System::{
                Com::{
                    APTTYPE, APTTYPE_MAINSTA, APTTYPE_MTA, APTTYPE_NA, APTTYPE_STA,
                    APTTYPEQUALIFIER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
                    CoGetApartmentType, CoInitializeEx, CoUninitialize,
                },
                Threading::{
                    GetCurrentThreadId, OpenProcess, PROCESS_NAME_FORMAT,
                    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
                },
            },
            UI::{
                Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
                WindowsAndMessaging::{
                    DispatchMessageW, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY,
                    EVENT_OBJECT_FOCUS, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SELECTION,
                    EVENT_OBJECT_VALUECHANGE, EVENT_SYSTEM_ALERT, EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_MENUEND, EVENT_SYSTEM_MENUSTART, EnumWindows, GetForegroundWindow,
                    GetMessageW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
                    IsWindowVisible, MSG, PostThreadMessageW, TranslateMessage,
                    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP,
                },
            },
        },
        core::{BOOL, PWSTR},
    };

    use super::{
        A11yError, A11yResult, AccessibleEvent, AccessibleEventKind, ComApartmentKind,
        WinEventHookReadback, runtime_id_hex,
    };

    static UIA_CLIENT: OnceLock<ProcessUiaClient> = OnceLock::new();
    static WIN_EVENT_SENDER: Mutex<Option<UnboundedSender<AccessibleEvent>>> = Mutex::new(None);
    static SNAPSHOT_CACHE: Mutex<Option<SnapshotCache>> = Mutex::new(None);
    static SNAPSHOT_DEPTH1_DEGRADED: AtomicBool = AtomicBool::new(false);

    #[derive(Clone)]
    struct SnapshotCache {
        requested_depth: u32,
        captured_at: Instant,
        tree: AccessibleSubtree,
    }

    const WIN_EVENT_IDS: [u32; 10] = [
        EVENT_SYSTEM_FOREGROUND,
        EVENT_OBJECT_FOCUS,
        EVENT_OBJECT_VALUECHANGE,
        EVENT_OBJECT_NAMECHANGE,
        EVENT_OBJECT_CREATE,
        EVENT_OBJECT_DESTROY,
        EVENT_OBJECT_SELECTION,
        EVENT_SYSTEM_MENUSTART,
        EVENT_SYSTEM_MENUEND,
        EVENT_SYSTEM_ALERT,
    ];

    struct ProcessUiaClient {
        automation: Mutex<UIAutomation>,
    }

    #[allow(clippy::non_send_fields_in_send_ty)]
    unsafe impl Send for ProcessUiaClient {}
    unsafe impl Sync for ProcessUiaClient {}

    pub struct WinEventSubscription {
        stop: Arc<AtomicBool>,
        thread_id: u32,
        join: Option<JoinHandle<()>>,
        readback: WinEventHookReadback,
    }

    impl WinEventSubscription {
        pub const fn readback(&self) -> &WinEventHookReadback {
            &self.readback
        }
    }

    impl Drop for WinEventSubscription {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = unsafe { PostThreadMessageW(self.thread_id, WM_APP, WPARAM(0), LPARAM(0)) };
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
            if let Ok(mut guard) = WIN_EVENT_SENDER.lock() {
                *guard = None;
            }
        }
    }

    pub fn focused_window() -> A11yResult<UIElement> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return Err(A11yError::NoForeground {
                detail: "GetForegroundWindow returned null".to_owned(),
            });
        }

        element_from_hwnd(hwnd)
    }

    pub fn window_from_hwnd(hwnd: i64) -> A11yResult<UIElement> {
        let hwnd = HWND(hwnd as *mut c_void);
        if hwnd.0.is_null() {
            return Err(A11yError::NoForeground {
                detail: "HWND was null".to_owned(),
            });
        }
        element_from_hwnd(hwnd)
    }

    fn element_from_hwnd(hwnd: HWND) -> A11yResult<UIElement> {
        with_automation(|automation| {
            automation
                .element_from_handle(Handle::from(hwnd))
                .map_err(map_uia_error)
        })
    }

    pub fn window_for_process(pid: u32) -> A11yResult<UIElement> {
        let hwnd = find_window_for_pid(pid).ok_or_else(|| A11yError::NoForeground {
            detail: format!("no visible top-level window for pid {pid}"),
        })?;
        with_automation(|automation| {
            automation
                .element_from_handle(Handle::from(hwnd))
                .map_err(map_uia_error)
        })
    }

    pub fn foreground_context(hwnd: i64) -> A11yResult<ForegroundContext> {
        let hwnd = HWND(hwnd as *mut c_void);
        let mut pid = 0_u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
        }
        let process_path = process_path(pid).unwrap_or_default();
        let process_name = Path::new(&process_path).file_name().map_or_else(
            || format!("pid-{pid}"),
            |name| name.to_string_lossy().into_owned(),
        );
        Ok(ForegroundContext {
            hwnd: hwnd.0 as isize as i64,
            pid,
            process_name,
            process_path,
            window_title: window_title(hwnd),
            window_bounds: window_rect(hwnd)?,
            monitor_index: 0,
            dpi_scale: 1.0,
            profile_id: None,
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
        })
    }

    pub fn focused_element() -> A11yResult<UIElement> {
        with_automation(|automation| {
            let cache = create_cache_request(automation, 0, ElementMode::Full)?;
            automation
                .get_focused_element_build_cache(&cache)
                .map_err(map_uia_error)
        })
    }

    pub fn element_from_point(point: Point) -> A11yResult<UIElement> {
        with_automation(|automation| {
            let cache = create_cache_request(automation, 0, ElementMode::Full)?;
            automation
                .element_from_point_build_cache(UiaPoint::new(point.x, point.y), &cache)
                .map_err(map_uia_error)
        })
    }

    pub fn snapshot(root: &UIElement, depth: u32) -> A11yResult<AccessibleSubtree> {
        if let Some(tree) = cached_snapshot(depth) {
            return Ok(tree);
        }

        with_automation(|automation| {
            let requested_depth = if depth > 1 && SNAPSHOT_DEPTH1_DEGRADED.load(Ordering::Relaxed) {
                1
            } else {
                depth
            };
            let start = Instant::now();
            let mut tree = snapshot_at_depth(automation, root, requested_depth)?;
            if depth > 1 && requested_depth > 1 && start.elapsed() > Duration::from_millis(25) {
                SNAPSHOT_DEPTH1_DEGRADED.store(true, Ordering::Relaxed);
                tree = snapshot_at_depth(automation, root, 1)?;
                tree.truncated = true;
            }
            if requested_depth < depth {
                tree.truncated = true;
            }
            store_snapshot(depth, &tree);
            Ok(tree)
        })
    }

    pub fn re_resolve(id: &ElementId) -> A11yResult<UIElement> {
        let parts = id.parts().map_err(|err| A11yError::InvalidElementId {
            detail: err.to_string(),
        })?;
        with_automation(|automation| {
            let cache = create_cache_request(automation, 8, ElementMode::Full)?;
            let hwnd = isize::try_from(parts.hwnd).map_err(|err| A11yError::InvalidElementId {
                detail: err.to_string(),
            })?;
            let root = automation
                .element_from_handle_build_cache(Handle::from(hwnd), &cache)
                .map_err(map_uia_error)?;
            find_by_runtime_id_hex(&root, &parts.runtime_id_hex, 0, 8)?.ok_or_else(|| {
                A11yError::ElementStale {
                    detail: format!(
                        "element id {id} was not found under hwnd 0x{:x}",
                        parts.hwnd
                    ),
                }
            })
        })
    }

    pub fn subscribe_win_events(
        sender: UnboundedSender<AccessibleEvent>,
    ) -> A11yResult<WinEventSubscription> {
        {
            let mut guard = WIN_EVENT_SENDER
                .lock()
                .map_err(|err| A11yError::internal(err.to_string()))?;
            *guard = Some(sender);
        }

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let (ready_tx, ready_rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name("synapse-a11y-winevent".to_owned())
            .spawn(move || win_event_thread(thread_stop, ready_tx))
            .map_err(|err| A11yError::internal(err.to_string()))?;
        let readback = ready_rx
            .recv_timeout(Duration::from_secs(3))
            .map_err(|err| {
                A11yError::internal(format!("WinEvent hook did not initialize: {err}"))
            })??;

        Ok(WinEventSubscription {
            stop,
            thread_id: readback.thread_id,
            join: Some(join),
            readback,
        })
    }

    fn client() -> A11yResult<&'static ProcessUiaClient> {
        if let Some(client) = UIA_CLIENT.get() {
            return Ok(client);
        }

        let automation = UIAutomation::new()
            .or_else(|_err| UIAutomation::new_direct())
            .map_err(map_uia_error)?;
        UIA_CLIENT
            .set(ProcessUiaClient {
                automation: Mutex::new(automation),
            })
            .map_err(|_client| A11yError::internal("UIA client was initialized concurrently"))?;
        UIA_CLIENT
            .get()
            .ok_or_else(|| A11yError::internal("UIA client missing after initialization"))
    }

    fn with_automation<T>(action: impl FnOnce(&UIAutomation) -> A11yResult<T>) -> A11yResult<T> {
        let guard = client()?
            .automation
            .lock()
            .map_err(|err| A11yError::internal(err.to_string()))?;
        action(&guard)
    }

    fn create_cache_request(
        automation: &UIAutomation,
        depth: u32,
        element_mode: ElementMode,
    ) -> A11yResult<UICacheRequest> {
        let cache = automation.create_cache_request().map_err(map_uia_error)?;
        for property in [
            UIProperty::RuntimeId,
            UIProperty::BoundingRectangle,
            UIProperty::ProcessId,
            UIProperty::ControlType,
            UIProperty::LocalizedControlType,
            UIProperty::Name,
            UIProperty::HasKeyboardFocus,
            UIProperty::IsEnabled,
            UIProperty::AutomationId,
            UIProperty::ClassName,
            UIProperty::NativeWindowHandle,
            UIProperty::IsInvokePatternAvailable,
            UIProperty::IsTogglePatternAvailable,
            UIProperty::IsValuePatternAvailable,
            UIProperty::IsSelectionPatternAvailable,
            UIProperty::IsExpandCollapsePatternAvailable,
            UIProperty::IsScrollPatternAvailable,
            UIProperty::IsTextPatternAvailable,
            UIProperty::IsWindowPatternAvailable,
            UIProperty::IsTransformPatternAvailable,
            UIProperty::IsRangeValuePatternAvailable,
        ] {
            cache.add_property(property).map_err(map_uia_error)?;
        }
        cache
            .set_tree_filter(
                automation
                    .get_control_view_condition()
                    .map_err(map_uia_error)?,
            )
            .map_err(map_uia_error)?;
        let scope = if depth == 0 {
            TreeScope::Element
        } else {
            TreeScope::Subtree
        };
        cache.set_tree_scope(scope).map_err(map_uia_error)?;
        cache
            .set_element_mode(element_mode)
            .map_err(map_uia_error)?;
        Ok(cache)
    }

    fn snapshot_at_depth(
        automation: &UIAutomation,
        root: &UIElement,
        depth: u32,
    ) -> A11yResult<AccessibleSubtree> {
        let cache = create_cache_request(automation, depth, ElementMode::None)?;
        let cached_root = root.build_updated_cache(&cache).map_err(map_uia_error)?;
        let root_hwnd = cached_hwnd(&cached_root).unwrap_or(0);
        let mut nodes = Vec::new();
        collect_nodes(&cached_root, None, 0, depth, root_hwnd, &mut nodes)?;
        let root = nodes
            .first()
            .map(|node| node.element_id.clone())
            .ok_or_else(|| A11yError::ElementStale {
                detail: "snapshot root produced no UIA node".to_owned(),
            })?;
        Ok(AccessibleSubtree {
            root,
            nodes,
            max_depth: depth,
            truncated: false,
        })
    }

    fn cached_snapshot(depth: u32) -> Option<AccessibleSubtree> {
        let guard = SNAPSHOT_CACHE.lock().ok()?;
        let cache = guard.as_ref()?;
        let is_fresh = cache.requested_depth == depth
            && cache.captured_at.elapsed() <= Duration::from_millis(50);
        let tree = is_fresh.then(|| cache.tree.clone());
        drop(guard);
        tree
    }

    fn store_snapshot(depth: u32, tree: &AccessibleSubtree) {
        if let Ok(mut guard) = SNAPSHOT_CACHE.lock() {
            *guard = Some(SnapshotCache {
                requested_depth: depth,
                captured_at: Instant::now(),
                tree: tree.clone(),
            });
        }
    }

    fn invalidate_snapshot_cache() {
        if let Ok(mut guard) = SNAPSHOT_CACHE.lock() {
            *guard = None;
        }
    }

    fn collect_nodes(
        element: &UIElement,
        parent: Option<ElementId>,
        depth: u32,
        max_depth: u32,
        root_hwnd: i64,
        nodes: &mut Vec<AccessibleNode>,
    ) -> A11yResult<ElementId> {
        let children = if depth < max_depth {
            element.get_cached_children().unwrap_or_default()
        } else {
            Vec::new()
        };
        let node = node_from_cached_element(element, parent, depth, root_hwnd, children.len())?;
        let node_id = node.element_id.clone();
        nodes.push(node);
        for child in children {
            collect_nodes(
                &child,
                Some(node_id.clone()),
                depth + 1,
                max_depth,
                root_hwnd,
                nodes,
            )?;
        }
        Ok(node_id)
    }

    fn node_from_cached_element(
        element: &UIElement,
        parent: Option<ElementId>,
        depth: u32,
        root_hwnd: i64,
        children_count: usize,
    ) -> A11yResult<AccessibleNode> {
        let runtime_id = cached_runtime_id(element)?;
        let runtime_id_hex = runtime_id_hex(&runtime_id);
        let hwnd = cached_hwnd(element)
            .filter(|value| *value != 0)
            .unwrap_or(root_hwnd);
        Ok(AccessibleNode {
            element_id: element_id(hwnd, &runtime_id_hex),
            parent,
            name: element.get_cached_name().unwrap_or_default(),
            role: cached_role(element),
            automation_id: non_empty(element.get_cached_automation_id().unwrap_or_default()),
            bbox: cached_rect(element),
            enabled: element.is_cached_enabled().unwrap_or(false),
            focused: element.has_cached_keyboard_focus().unwrap_or(false),
            patterns: cached_patterns(element),
            children_count: u32::try_from(children_count).unwrap_or(u32::MAX),
            depth,
        })
    }

    fn cached_patterns(element: &UIElement) -> Vec<UiaPattern> {
        let mut patterns = Vec::new();
        push_pattern(
            element,
            &mut patterns,
            UIProperty::IsInvokePatternAvailable,
            UiaPattern::Invoke,
        );
        push_pattern(
            element,
            &mut patterns,
            UIProperty::IsTogglePatternAvailable,
            UiaPattern::Toggle,
        );
        push_pattern(
            element,
            &mut patterns,
            UIProperty::IsValuePatternAvailable,
            UiaPattern::Value,
        );
        push_pattern(
            element,
            &mut patterns,
            UIProperty::IsSelectionPatternAvailable,
            UiaPattern::Selection,
        );
        push_pattern(
            element,
            &mut patterns,
            UIProperty::IsExpandCollapsePatternAvailable,
            UiaPattern::ExpandCollapse,
        );
        push_pattern(
            element,
            &mut patterns,
            UIProperty::IsScrollPatternAvailable,
            UiaPattern::Scroll,
        );
        push_pattern(
            element,
            &mut patterns,
            UIProperty::IsTextPatternAvailable,
            UiaPattern::Text,
        );
        push_pattern(
            element,
            &mut patterns,
            UIProperty::IsWindowPatternAvailable,
            UiaPattern::Window,
        );
        push_pattern(
            element,
            &mut patterns,
            UIProperty::IsTransformPatternAvailable,
            UiaPattern::Transform,
        );
        push_pattern(
            element,
            &mut patterns,
            UIProperty::IsRangeValuePatternAvailable,
            UiaPattern::RangeValue,
        );
        patterns
    }

    fn push_pattern(
        element: &UIElement,
        patterns: &mut Vec<UiaPattern>,
        property: UIProperty,
        pattern: UiaPattern,
    ) {
        if cached_bool(element, property) {
            patterns.push(pattern);
        }
    }

    fn cached_bool(element: &UIElement, property: UIProperty) -> bool {
        element
            .get_cached_property_value(property)
            .ok()
            .and_then(|variant| <&Variant as TryInto<bool>>::try_into(&variant).ok())
            .unwrap_or(false)
    }

    fn cached_rect(element: &UIElement) -> Rect {
        element.get_cached_bounding_rectangle().map_or(
            Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            },
            |rect| Rect {
                x: rect.get_left(),
                y: rect.get_top(),
                w: rect.get_right().saturating_sub(rect.get_left()),
                h: rect.get_bottom().saturating_sub(rect.get_top()),
            },
        )
    }

    fn cached_role(element: &UIElement) -> String {
        let localized = element
            .get_cached_localized_control_type()
            .unwrap_or_default();
        if !localized.is_empty() {
            return localized;
        }

        element
            .get_cached_control_type()
            .map_or_else(|_err| "unknown".to_owned(), control_type_name)
    }

    fn control_type_name(control_type: ControlType) -> String {
        format!("{control_type:?}")
    }

    fn cached_hwnd(element: &UIElement) -> Option<i64> {
        let handle = element.get_cached_native_window_handle().ok()?;
        let raw: isize = handle.into();
        Some(raw as i64)
    }

    fn non_empty(value: String) -> Option<String> {
        if value.is_empty() { None } else { Some(value) }
    }

    fn window_title(hwnd: HWND) -> String {
        let mut buffer = vec![0_u16; 512];
        let len = unsafe { GetWindowTextW(hwnd, &mut buffer) };
        String::from_utf16_lossy(&buffer[..usize::try_from(len).unwrap_or(0)])
    }

    fn window_rect(hwnd: HWND) -> A11yResult<Rect> {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &raw mut rect) }
            .map_err(|err| A11yError::internal(err.to_string()))?;
        Ok(Rect {
            x: rect.left,
            y: rect.top,
            w: rect.right.saturating_sub(rect.left),
            h: rect.bottom.saturating_sub(rect.top),
        })
    }

    fn process_path(pid: u32) -> A11yResult<String> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .map_err(|err| A11yError::internal(err.to_string()))?;
        let mut buffer = vec![0_u16; 32_768];
        let mut len = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let result = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_FORMAT(0),
                PWSTR(buffer.as_mut_ptr()),
                &raw mut len,
            )
        };
        let _ = unsafe { CloseHandle(handle) };
        result.map_err(|err| A11yError::internal(err.to_string()))?;
        Ok(String::from_utf16_lossy(
            &buffer[..usize::try_from(len).unwrap_or(0)],
        ))
    }

    fn find_window_for_pid(pid: u32) -> Option<HWND> {
        struct Search {
            pid: u32,
            hwnd: Option<HWND>,
        }

        unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let search = unsafe { &mut *(lparam.0 as *mut Search) };
            let mut window_pid = 0_u32;
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&raw mut window_pid));
            }
            if window_pid == search.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
                search.hwnd = Some(hwnd);
                return BOOL(0);
            }
            BOOL(1)
        }

        let mut search = Search { pid, hwnd: None };
        unsafe {
            let _ = EnumWindows(
                Some(enum_window),
                LPARAM((&raw mut search).cast::<core::ffi::c_void>() as isize),
            );
        }
        search.hwnd
    }

    fn find_by_runtime_id_hex(
        root: &UIElement,
        runtime_id_hex_expected: &str,
        depth: u32,
        max_depth: u32,
    ) -> A11yResult<Option<UIElement>> {
        let runtime_id = cached_runtime_id(root)?;
        if runtime_id_hex(&runtime_id).eq_ignore_ascii_case(runtime_id_hex_expected) {
            return Ok(Some(root.clone()));
        }
        if depth >= max_depth {
            return Ok(None);
        }

        for child in root.get_cached_children().unwrap_or_default() {
            if let Some(found) =
                find_by_runtime_id_hex(&child, runtime_id_hex_expected, depth + 1, max_depth)?
            {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    fn cached_runtime_id(element: &UIElement) -> A11yResult<Vec<i32>> {
        let value = element
            .get_cached_property_value(UIProperty::RuntimeId)
            .map_err(map_uia_error)?;
        match value.get_value().map_err(map_uia_error)? {
            Value::ArrayI4(items) => Ok(items),
            Value::SAFEARRAY(array) => array.try_into().map_err(map_uia_error),
            other => Err(A11yError::internal(format!(
                "cached RuntimeId had unexpected type {other}"
            ))),
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn win_event_thread(
        stop: Arc<AtomicBool>,
        ready: mpsc::Sender<A11yResult<WinEventHookReadback>>,
    ) {
        let thread_id = unsafe { GetCurrentThreadId() };
        let init =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if init.is_err() {
            let _ = ready.send(Err(A11yError::internal(format!("{init:?}"))));
            return;
        }

        let hooks = install_hooks();
        if hooks.is_empty() {
            let _ = ready.send(Err(A11yError::internal(
                "SetWinEventHook returned no hooks",
            )));
            unsafe {
                CoUninitialize();
            }
            return;
        }

        let readback = WinEventHookReadback {
            thread_id,
            apartment: read_current_apartment(),
            hook_count: hooks.len(),
            event_ids: WIN_EVENT_IDS.to_vec(),
        };
        let _ = ready.send(Ok(readback));

        let mut msg = MSG::default();
        while !stop.load(Ordering::SeqCst) {
            let result = unsafe { GetMessageW(&raw mut msg, None, 0, 0) };
            if result.0 <= 0 || msg.message == WM_APP {
                break;
            }
            unsafe {
                let _ = TranslateMessage(&raw const msg);
                DispatchMessageW(&raw const msg);
            }
        }

        for hook in hooks {
            unsafe {
                let _ = UnhookWinEvent(hook);
            }
        }
        unsafe {
            CoUninitialize();
        }
    }

    fn install_hooks() -> Vec<HWINEVENTHOOK> {
        WIN_EVENT_IDS
            .iter()
            .filter_map(|event_id| {
                let hook = unsafe {
                    SetWinEventHook(
                        *event_id,
                        *event_id,
                        None,
                        Some(win_event_proc),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                };
                (!hook.is_invalid()).then_some(hook)
            })
            .collect()
    }

    unsafe extern "system" fn win_event_proc(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        idobject: i32,
        idchild: i32,
        _event_thread: u32,
        event_time_ms: u32,
    ) {
        let Ok(guard) = WIN_EVENT_SENDER.lock() else {
            return;
        };
        let Some(sender) = guard.as_ref() else {
            return;
        };
        let Some(kind) = event_kind(event) else {
            return;
        };
        invalidate_snapshot_cache();

        let window_id = hwnd.0 as isize as i64;
        let event = AccessibleEvent {
            seq: u64::from(event_time_ms),
            at_ms: u64::from(event_time_ms),
            window_id,
            element_id: event_element_id(hwnd, event, idobject, idchild),
            kind,
            name: None,
            value: None,
        };
        let _ = sender.send(event);
    }

    const fn event_kind(event: u32) -> Option<AccessibleEventKind> {
        match event {
            EVENT_SYSTEM_FOREGROUND => Some(AccessibleEventKind::ForegroundChanged),
            EVENT_OBJECT_FOCUS => Some(AccessibleEventKind::FocusChanged),
            EVENT_OBJECT_VALUECHANGE => Some(AccessibleEventKind::ValueChanged),
            EVENT_OBJECT_NAMECHANGE => Some(AccessibleEventKind::NameChanged),
            EVENT_OBJECT_CREATE => Some(AccessibleEventKind::ElementAppeared),
            EVENT_OBJECT_DESTROY => Some(AccessibleEventKind::ElementDisappeared),
            EVENT_OBJECT_SELECTION => Some(AccessibleEventKind::SelectionChanged),
            EVENT_SYSTEM_MENUSTART => Some(AccessibleEventKind::MenuStart),
            EVENT_SYSTEM_MENUEND => Some(AccessibleEventKind::MenuEnd),
            EVENT_SYSTEM_ALERT => Some(AccessibleEventKind::Alert),
            _ => None,
        }
    }

    fn event_element_id(hwnd: HWND, event: u32, idobject: i32, idchild: i32) -> Option<ElementId> {
        if hwnd.0.is_null() {
            return None;
        }
        let window_id = hwnd.0 as isize as i64;
        let runtime_id = format!(
            "{event:08x}{:08x}{:08x}",
            idobject.cast_unsigned(),
            idchild.cast_unsigned()
        );
        Some(element_id(window_id, &runtime_id))
    }

    fn read_current_apartment() -> ComApartmentKind {
        let mut apartment = APTTYPE::default();
        let mut qualifier = APTTYPEQUALIFIER::default();
        if unsafe { CoGetApartmentType(&raw mut apartment, &raw mut qualifier) }.is_err() {
            return ComApartmentKind::Unknown;
        }
        match apartment {
            value if value == APTTYPE_STA => ComApartmentKind::Sta,
            value if value == APTTYPE_MTA => ComApartmentKind::Mta,
            value if value == APTTYPE_NA => ComApartmentKind::Neutral,
            value if value == APTTYPE_MAINSTA => ComApartmentKind::MainSta,
            _ => ComApartmentKind::Unknown,
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    #[allow(clippy::missing_const_for_fn)]
    fn map_uia_error(err: uiautomation::Error) -> A11yError {
        A11yError::internal(err.to_string())
    }

    #[allow(clippy::missing_const_for_fn)]
    fn _hwnd_from_i64(hwnd: i64) -> HWND {
        HWND(hwnd as *mut c_void)
    }
}

#[cfg(not(windows))]
mod platform {
    use synapse_core::{AccessibleSubtree, ElementId, ForegroundContext, Point};
    use tokio::sync::mpsc::UnboundedSender;

    use super::{A11yError, A11yResult, AccessibleEvent, UIElement, WinEventHookReadback};

    pub struct WinEventSubscription {
        readback: WinEventHookReadback,
    }

    impl WinEventSubscription {
        pub const fn readback(&self) -> &WinEventHookReadback {
            &self.readback
        }
    }

    pub fn focused_window() -> A11yResult<UIElement> {
        Err(A11yError::not_available(
            "UIA foreground window lookup requires Windows",
        ))
    }

    pub fn window_from_hwnd(_hwnd: i64) -> A11yResult<UIElement> {
        Err(A11yError::not_available(
            "UIA HWND window lookup requires Windows",
        ))
    }

    pub fn window_for_process(_pid: u32) -> A11yResult<UIElement> {
        Err(A11yError::not_available(
            "UIA process window lookup requires Windows",
        ))
    }

    pub fn foreground_context(_hwnd: i64) -> A11yResult<ForegroundContext> {
        Err(A11yError::not_available(
            "foreground context lookup requires Windows",
        ))
    }

    pub fn focused_element() -> A11yResult<UIElement> {
        Err(A11yError::not_available(
            "UIA focused element lookup requires Windows",
        ))
    }

    pub fn element_from_point(_point: Point) -> A11yResult<UIElement> {
        Err(A11yError::not_available(
            "UIA element hit testing requires Windows",
        ))
    }

    pub fn snapshot(_root: &UIElement, _depth: u32) -> A11yResult<AccessibleSubtree> {
        Err(A11yError::not_available(
            "UIA tree snapshots require Windows",
        ))
    }

    pub fn re_resolve(_id: &ElementId) -> A11yResult<UIElement> {
        Err(A11yError::not_available(
            "UIA element re-resolution requires Windows",
        ))
    }

    pub fn subscribe_win_events(
        _sender: UnboundedSender<AccessibleEvent>,
    ) -> A11yResult<WinEventSubscription> {
        Err(A11yError::not_available("WinEvent hooks require Windows"))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use proptest::prelude::*;
    use synapse_core::{ElementId, element_id, error_codes};
    use tokio::net::TcpListener;

    use super::*;

    fn event(
        seq: u64,
        at_ms: u64,
        element: Option<ElementId>,
        kind: AccessibleEventKind,
    ) -> AccessibleEvent {
        AccessibleEvent {
            seq,
            at_ms,
            window_id: 0x1234,
            element_id: element,
            kind,
            name: None,
            value: None,
        }
    }

    #[test]
    fn coalesce_empty_input_prints_before_after_state() {
        let before = Vec::<AccessibleEvent>::new();
        println!("source_of_truth=coalesced_events edge=empty before={before:?}");
        let after = coalesce_events(before, Duration::from_millis(50));
        println!("source_of_truth=coalesced_events edge=empty after={after:?}");
        assert!(after.is_empty());
    }

    #[test]
    fn coalesce_same_key_within_window_keeps_latest_state() -> Result<(), Box<dyn std::error::Error>>
    {
        let id = ElementId::parse("0x1234:00000001")?;
        let mut first = event(1, 0, Some(id.clone()), AccessibleEventKind::NameChanged);
        first.name = Some("old".to_owned());
        let mut second = event(2, 49, Some(id), AccessibleEventKind::NameChanged);
        second.name = Some("new".to_owned());
        let before = vec![first, second];
        println!("source_of_truth=coalesced_events edge=within_50ms before={before:?}");
        let after = coalesce_events(before, Duration::from_millis(50));
        println!("source_of_truth=coalesced_events edge=within_50ms after={after:?}");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].name.as_deref(), Some("new"));
        assert_eq!(after[0].seq, 2);
        Ok(())
    }

    #[test]
    fn coalesce_exact_boundary_does_not_merge() -> Result<(), Box<dyn std::error::Error>> {
        let id = ElementId::parse("0x1234:00000002")?;
        let before = vec![
            event(1, 0, Some(id.clone()), AccessibleEventKind::FocusChanged),
            event(2, 50, Some(id), AccessibleEventKind::FocusChanged),
        ];
        println!("source_of_truth=coalesced_events edge=exact_50ms before={before:?}");
        let after = coalesce_events(before, Duration::from_millis(50));
        println!("source_of_truth=coalesced_events edge=exact_50ms after={after:?}");
        assert_eq!(after.len(), 2);
        Ok(())
    }

    #[test]
    fn value_debounce_rapid_typing_keeps_first_and_latest_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let id = ElementId::parse("0x1234:00000003")?;
        let before: Vec<_> = "abcdef"
            .chars()
            .enumerate()
            .map(|(index, character)| {
                let mut item = event(
                    u64::try_from(index).unwrap_or(u64::MAX),
                    u64::try_from(index * 20).unwrap_or(u64::MAX),
                    Some(id.clone()),
                    AccessibleEventKind::ValueChanged,
                );
                item.value = Some(character.to_string());
                item
            })
            .collect();
        println!("source_of_truth=debounced_events edge=rapid_typing before={before:?}");
        let after = debounce_value_changes(before, Duration::from_millis(200));
        println!("source_of_truth=debounced_events edge=rapid_typing after={after:?}");
        assert!(after.len() <= 2);
        assert_eq!(
            after.last().and_then(|item| item.value.as_deref()),
            Some("f")
        );
        Ok(())
    }

    #[test]
    fn value_debounce_focus_loss_flushes_pending_state() -> Result<(), Box<dyn std::error::Error>> {
        let id = ElementId::parse("0x1234:00000004")?;
        let mut first = event(1, 0, Some(id.clone()), AccessibleEventKind::ValueChanged);
        first.value = Some("a".to_owned());
        let mut second = event(2, 20, Some(id.clone()), AccessibleEventKind::ValueChanged);
        second.value = Some("b".to_owned());
        let focus = event(3, 25, Some(id), AccessibleEventKind::FocusChanged);
        let before = vec![first, second, focus];
        println!("source_of_truth=debounced_events edge=focus_loss before={before:?}");
        let after = debounce_value_changes(before, Duration::from_millis(200));
        println!("source_of_truth=debounced_events edge=focus_loss after={after:?}");
        assert_eq!(after.len(), 3);
        assert_eq!(after[1].value.as_deref(), Some("b"));
        assert_eq!(after[2].kind, AccessibleEventKind::FocusChanged);
        Ok(())
    }

    #[test]
    fn runtime_id_hex_round_trips_through_composite_element_id()
    -> Result<(), Box<dyn std::error::Error>> {
        let runtime = [42, -1, 0x1234_abcd_u32.cast_signed()];
        let runtime_hex = runtime_id_hex(&runtime);
        let id = element_id(0x12ab, &runtime_hex);
        println!("source_of_truth=element_id edge=runtime_hex before={runtime:?}");
        println!("source_of_truth=element_id edge=runtime_hex after={id}");
        let parts = id.parts()?;
        assert_eq!(parts.hwnd, 0x12ab);
        assert_eq!(parts.runtime_id_hex, "0000002affffffff1234abcd");
        Ok(())
    }

    #[test]
    fn non_windows_uia_reports_not_available() {
        #[cfg(not(windows))]
        {
            let before = "focused_window";
            println!("source_of_truth=a11y_error edge=non_windows before={before}");
            let after = focused_window();
            println!("source_of_truth=a11y_error edge=non_windows after={after:?}");
            assert_eq!(
                after.err().map(|err| err.code()),
                Some(error_codes::A11Y_NOT_AVAILABLE)
            );
        }
    }

    #[tokio::test]
    async fn cdp_probe_non_chromium_is_explicitly_not_chromium() {
        let before = ("notepad.exe", Vec::<u16>::new());
        println!("source_of_truth=cdp_diagnostics edge=non_chromium before={before:?}");
        let after = probe_chromium_cdp("notepad.exe", &[], Duration::from_millis(10)).await;
        println!("source_of_truth=cdp_diagnostics edge=non_chromium after={after:?}");
        assert_eq!(after.status, CdpStatus::NotChromium);
        assert!(after.reason_code.is_none());
    }

    #[tokio::test]
    async fn cdp_probe_chromium_without_port_surfaces_unreachable_code() {
        let before = ("chrome.exe", Vec::<u16>::new());
        println!("source_of_truth=cdp_diagnostics edge=no_debug_port before={before:?}");
        let after = probe_chromium_cdp("chrome.exe", &[], Duration::from_millis(10)).await;
        println!("source_of_truth=cdp_diagnostics edge=no_debug_port after={after:?}");
        assert_eq!(after.status, CdpStatus::Unreachable);
        assert_eq!(
            after.reason_code.as_deref(),
            Some(error_codes::A11Y_CDP_UNREACHABLE)
        );
    }

    #[tokio::test]
    async fn cdp_probe_reachable_debug_port_surfaces_capabilities()
    -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        println!("source_of_truth=cdp_diagnostics edge=reachable_port before=port:{port}");
        let after = probe_chromium_cdp("msedge.exe", &[port], Duration::from_secs(1)).await;
        println!("source_of_truth=cdp_diagnostics edge=reachable_port after={after:?}");
        assert_eq!(after.status, CdpStatus::Ok);
        assert_eq!(
            after.endpoint.as_deref(),
            Some(format!("http://127.0.0.1:{port}").as_str())
        );
        assert_eq!(after.capabilities, cdp_capabilities());
        Ok(())
    }

    proptest! {
        #[test]
        fn coalescing_never_outputs_same_key_inside_window(times in proptest::collection::vec(0_u64..500, 1..80)) {
            let id = element_id(0x1234, "00000005");
            let mut sorted = times;
            sorted.sort_unstable();
            let input: Vec<_> = sorted
                .iter()
                .enumerate()
                .map(|(index, at_ms)| event(u64::try_from(index).unwrap_or(u64::MAX), *at_ms, Some(id.clone()), AccessibleEventKind::NameChanged))
                .collect();
            let output = coalesce_events(input, Duration::from_millis(50));
            for pair in output.windows(2) {
                prop_assert!(
                    pair[1].at_ms.saturating_sub(pair[0].at_ms) >= 50,
                    "source_of_truth=coalesced_events edge=proptest after={output:?}"
                );
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_win_event_hook_apartment_readback_is_sta() -> Result<(), Box<dyn std::error::Error>>
    {
        let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
        println!("source_of_truth=winevent_hook edge=apartment before=unsubscribed");
        let subscription = subscribe_win_events(sender)?;
        let readback = subscription.readback().clone();
        println!("source_of_truth=winevent_hook edge=apartment after={readback:?}");
        assert!(readback.apartment.is_sta_family());
        assert_eq!(readback.hook_count, 10);
        assert_eq!(readback.event_ids.len(), 10);
        drop(subscription);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_foreground_snapshot_round_trips_element_id() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = focused_window()?;
        println!("source_of_truth=uia_snapshot edge=depth2 before=focused_window_resolved");
        let tree = snapshot(&root, 2)?;
        println!(
            "source_of_truth=uia_snapshot edge=depth2 after=root:{} nodes:{} max_depth:{}",
            tree.root,
            tree.nodes.len(),
            tree.max_depth
        );
        assert!(!tree.nodes.is_empty());
        let resolved = re_resolve(&tree.root)?;
        let round_trip = snapshot(&resolved, 0)?;
        println!(
            "source_of_truth=uia_snapshot edge=round_trip after=root:{} nodes:{}",
            round_trip.root,
            round_trip.nodes.len()
        );
        assert_eq!(round_trip.root, tree.root);
        Ok(())
    }
}
