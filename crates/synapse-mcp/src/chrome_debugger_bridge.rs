use std::{
    collections::{HashMap, VecDeque},
    ffi::OsString,
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synapse_core::{AccessibleNode, CdpCapability, CdpStatus, Rect, error_codes};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Notify, RwLock, oneshot},
    time::timeout,
};

const EXTENSION_ID: &str = "leoocgnkjnplbfdbklajepahofecgfbk";
const NATIVE_HOST_NAME: &str = "com.synapse.chrome_debugger";
const CHROME_DEBUGGER_SILENT_FLAG: &str = "--silent-debugger-extension-api";
const BRIDGE_PROTOCOL_VERSION: u32 = 1;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const NATIVE_POLL_TIMEOUT: Duration = Duration::from_secs(15);
const NATIVE_DAEMON_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_NATIVE_MESSAGE_FROM_CHROME: usize = 64 * 1024 * 1024;
const MAX_NATIVE_MESSAGE_TO_CHROME: usize = 1024 * 1024;
const UNKNOWN_NATIVE_HOST_ID_FRAGMENT: &str = "unknown chrome debugger native host_id";
const INSTALL_GUIDANCE: &str = "install the bundled Synapse Chrome extension from extensions\\synapse-chrome-debugger, register the native host with scripts\\install-synapse-chrome-debugger.ps1, and launch Chrome with --silent-debugger-extension-api before using attach-capable debugger bridge commands; expected extension_id=leoocgnkjnplbfdbklajepahofecgfbk native_host=com.synapse.chrome_debugger";
const TOKEN_ENV: &str = "SYNAPSE_BEARER_TOKEN";
const APPDATA_ENV: &str = "APPDATA";

#[derive(Clone, Debug)]
pub(crate) struct NativeHostInvocation {
    pub origin: String,
    pub parent_window: Option<String>,
}

#[must_use]
pub(crate) fn native_host_invocation_from_args<I>(args: I) -> Option<NativeHostInvocation>
where
    I: IntoIterator<Item = OsString>,
{
    let mut origin = None;
    let mut parent_window = None;
    for arg in args {
        let value = arg.to_string_lossy();
        if value.starts_with("chrome-extension://") {
            origin = Some(value.into_owned());
        } else if let Some(parent) = value.strip_prefix("--parent-window=") {
            parent_window = Some(parent.to_owned());
        }
    }
    origin.map(|origin| NativeHostInvocation {
        origin,
        parent_window,
    })
}

#[derive(Debug)]
pub(crate) struct ChromeDebuggerBridgeError {
    code: &'static str,
    detail: String,
}

impl ChromeDebuggerBridgeError {
    #[must_use]
    pub(crate) const fn code(&self) -> &'static str {
        self.code
    }

    #[must_use]
    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }

    #[must_use]
    pub(crate) fn cdp_status(&self) -> CdpStatus {
        if self.code == error_codes::A11Y_CDP_EXTENSION_UNAVAILABLE {
            CdpStatus::ExtensionUnavailable
        } else {
            CdpStatus::AttachFailed
        }
    }

    fn unavailable() -> Self {
        Self {
            code: error_codes::A11Y_CDP_EXTENSION_UNAVAILABLE,
            detail: format!(
                "Chrome debugger extension/native host is not connected; {INSTALL_GUIDANCE}"
            ),
        }
    }

    fn timeout(command_kind: &str) -> Self {
        Self {
            code: error_codes::A11Y_CDP_EXTENSION_TIMEOUT,
            detail: format!(
                "Chrome debugger extension command {command_kind:?} timed out after {}s",
                COMMAND_TIMEOUT.as_secs()
            ),
        }
    }

    fn protocol(detail: impl Into<String>) -> Self {
        Self {
            code: error_codes::A11Y_CDP_ATTACH_FAILED,
            detail: detail.into(),
        }
    }

    fn extension(code: Option<&str>, detail: impl Into<String>) -> Self {
        let code = match code {
            Some(error_codes::A11Y_CDP_EXTENSION_UNAVAILABLE) => {
                error_codes::A11Y_CDP_EXTENSION_UNAVAILABLE
            }
            Some(error_codes::A11Y_CDP_EXTENSION_DETACHED) => {
                error_codes::A11Y_CDP_EXTENSION_DETACHED
            }
            Some(error_codes::A11Y_CDP_EXTENSION_TIMEOUT) => {
                error_codes::A11Y_CDP_EXTENSION_TIMEOUT
            }
            Some(error_codes::A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED) => {
                error_codes::A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED
            }
            Some(error_codes::A11Y_CDP_AXTREE_FAILED) => error_codes::A11Y_CDP_AXTREE_FAILED,
            Some(error_codes::A11Y_CDP_ATTACH_FAILED) => error_codes::A11Y_CDP_ATTACH_FAILED,
            _ => error_codes::A11Y_CDP_ATTACH_FAILED,
        };
        Self {
            code,
            detail: detail.into(),
        }
    }

    fn debugger_warning_unsuppressed(
        hwnd: i64,
        pid: Option<u32>,
        process_name: Option<&str>,
        command_line: Option<&[String]>,
        reason: impl Into<String>,
    ) -> Self {
        let process_name = process_name.unwrap_or("unknown");
        let command_line = command_line
            .map(|parts| format!("{parts:?}"))
            .unwrap_or_else(|| "unreadable".to_owned());
        Self {
            code: error_codes::A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED,
            detail: format!(
                "Chrome debugger extension attach refused before chrome.debugger.attach because Chrome would surface its debugger warning UI; hwnd={hwnd} pid={} process_name={process_name:?} required_flag={CHROME_DEBUGGER_SILENT_FLAG:?} command_line={command_line} reason={}",
                pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string()),
                reason.into()
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ChromeDebuggerDomSnapshot {
    pub nodes: Vec<AccessibleNode>,
    pub total_ax_nodes: u32,
    pub page_url: String,
    pub target_id: String,
    pub session_id: String,
    pub target_candidate_count: u32,
    pub target_selection_reason: String,
    pub extension_id: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum ChromeDebuggerMouseButton {
    Left,
    Right,
    Middle,
}

impl ChromeDebuggerMouseButton {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ChromeDebuggerClickPoint {
    pub x: f64,
    pub y: f64,
    pub target_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ChromeDebuggerTypeResult {
    pub x: f64,
    pub y: f64,
    pub target_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ChromeDebuggerNodeValue {
    pub value: String,
    pub target_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ChromeDebuggerOpenTabResult {
    pub target_id: String,
    pub tab_id: u32,
    pub target_type: String,
    pub url: String,
    pub title: String,
    pub target_attached: bool,
    pub target_count_before: u32,
    pub target_count_after: u32,
    pub extension_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ChromeDebuggerCloseTabResult {
    pub target_id: String,
    pub tab_id: u32,
    pub target_count_before: u32,
    pub target_count_after: u32,
    pub extension_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ChromeDebuggerTargetInfo {
    pub target_id: String,
    pub tab_id: u32,
    pub target_type: String,
    pub url: String,
    pub title: String,
    pub target_candidate_count: u32,
    pub target_selection_reason: String,
    pub extension_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ChromeDebuggerNavigateResult {
    pub target_id: String,
    pub tab_id: u32,
    pub action: String,
    pub requested_url: Option<String>,
    pub before_url: String,
    pub before_title: String,
    pub after_url: String,
    pub after_title: String,
    pub ready_state: String,
    pub history_current_index: i64,
    pub history_entry_count: u32,
    pub history_readback_source: String,
    pub readback_backend: String,
    pub navigation_error_text: Option<String>,
    pub is_download: Option<bool>,
    pub target_candidate_count: u32,
    pub target_selection_reason: String,
    pub extension_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExtensionSnapshotResponse {
    nodes: Vec<ExtensionDomNode>,
    total_ax_nodes: u32,
    page_url: String,
    target_id: String,
    session_id: String,
    target_candidate_count: u32,
    target_selection_reason: String,
    extension_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ExtensionDomNode {
    backend_node_id: i64,
    parent_backend_node_id: Option<i64>,
    role: String,
    name: String,
    value: Option<String>,
    bbox: Option<Rect>,
    child_count: u32,
    enabled: bool,
    focused: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct NativeRegisterResponse {
    ok: bool,
    host_id: String,
    bridge_protocol_version: u32,
    native_host_name: String,
    expected_extension_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct NativeRegisterRequest {
    origin: String,
    pid: u32,
    parent_window: Option<String>,
    bridge_protocol_version: u32,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct NativeMessageRequest {
    host_id: String,
    message: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NativeNextQuery {
    host_id: String,
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NativeNextResponse {
    ok: bool,
    command: Option<ChromeCommand>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ChromeCommand {
    id: String,
    kind: String,
    params: Value,
}

#[derive(Debug, Deserialize)]
struct ChromeResponse {
    id: String,
    ok: bool,
    result: Option<Value>,
    error: Option<ChromeResponseError>,
}

#[derive(Debug, Deserialize)]
struct ChromeResponseError {
    code: Option<String>,
    detail: Option<String>,
}

struct PendingResponse {
    host_id: String,
    kind: String,
    sender: oneshot::Sender<ChromeResponse>,
}

#[derive(Clone, Debug)]
struct HostRecord {
    origin: String,
    extension_id: Option<String>,
    pid: u32,
    parent_window: Option<String>,
    registered_unix_ms: u64,
    last_seen_unix_ms: u64,
    last_disconnect_detail: Option<String>,
    last_detach_reason: Option<String>,
}

struct QueuedCommand {
    host_id: String,
    command: ChromeCommand,
}

#[derive(Default)]
struct BridgeInner {
    active_host_id: Option<String>,
    hosts: HashMap<String, HostRecord>,
    commands: VecDeque<QueuedCommand>,
    pending: HashMap<String, PendingResponse>,
}

struct ChromeDebuggerBridge {
    inner: Mutex<BridgeInner>,
    notify: Notify,
    command_seq: AtomicU64,
}

impl ChromeDebuggerBridge {
    fn register(&self, request: NativeRegisterRequest) -> Result<NativeRegisterResponse, String> {
        if request.bridge_protocol_version != BRIDGE_PROTOCOL_VERSION {
            return Err(format!(
                "bridge protocol mismatch: host={} daemon={}",
                request.bridge_protocol_version, BRIDGE_PROTOCOL_VERSION
            ));
        }
        if !request.origin.starts_with("chrome-extension://") || !request.origin.ends_with('/') {
            return Err(format!(
                "native host origin must be a chrome-extension:// origin with trailing slash, got {:?}",
                request.origin
            ));
        }
        let now = now_unix_ms();
        let host_id = format!("chrome-native-{}-{}", request.pid, now);
        let record = HostRecord {
            origin: request.origin,
            extension_id: None,
            pid: request.pid,
            parent_window: request.parent_window,
            registered_unix_ms: now,
            last_seen_unix_ms: now,
            last_disconnect_detail: None,
            last_detach_reason: None,
        };
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "chrome debugger bridge lock poisoned during register".to_owned())?;
        inner.active_host_id = Some(host_id.clone());
        inner.hosts.insert(host_id.clone(), record);
        tracing::info!(
            code = "CHROME_DEBUGGER_NATIVE_HOST_REGISTERED",
            host_id = %host_id,
            pid = request.pid,
            "Chrome debugger native host registered"
        );
        Ok(NativeRegisterResponse {
            ok: true,
            host_id,
            bridge_protocol_version: BRIDGE_PROTOCOL_VERSION,
            native_host_name: NATIVE_HOST_NAME.to_owned(),
            expected_extension_id: EXTENSION_ID.to_owned(),
        })
    }

    fn post_message(&self, request: NativeMessageRequest) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "chrome debugger bridge lock poisoned during message".to_owned())?;
        let Some(host) = inner.hosts.get_mut(&request.host_id) else {
            return Err(format!(
                "unknown chrome debugger native host_id {:?}",
                request.host_id
            ));
        };
        host.last_seen_unix_ms = now_unix_ms();
        let message_type = request
            .message
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match message_type {
            "hello" => {
                host.extension_id = request
                    .message
                    .get("extensionId")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                tracing::info!(
                    code = "CHROME_DEBUGGER_EXTENSION_HELLO",
                    host_id = %request.host_id,
                    origin = %host.origin,
                    extension_id = host.extension_id.as_deref().unwrap_or_default(),
                    pid = host.pid,
                    parent_window = host.parent_window.as_deref().unwrap_or_default(),
                    registered_unix_ms = host.registered_unix_ms,
                    "Chrome debugger extension connected through native host"
                );
            }
            "response" => {
                let response = serde_json::from_value::<ChromeResponse>(request.message)
                    .map_err(|error| format!("decode chrome debugger response: {error}"))?;
                let id = response.id.clone();
                let Some(pending) = inner.pending.remove(&id) else {
                    tracing::warn!(
                        code = "CHROME_DEBUGGER_RESPONSE_WITHOUT_PENDING_COMMAND",
                        host_id = %request.host_id,
                        command_id = %id,
                        "Chrome debugger response had no pending daemon command"
                    );
                    return Ok(());
                };
                let _ = pending.sender.send(response);
            }
            "event" => {
                let event = request
                    .message
                    .get("event")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if event == "debuggerDetached" {
                    host.last_detach_reason = request
                        .message
                        .get("reason")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                    tracing::warn!(
                        code = error_codes::A11Y_CDP_EXTENSION_DETACHED,
                        host_id = %request.host_id,
                        reason = host.last_detach_reason.as_deref().unwrap_or_default(),
                        "Chrome debugger session detached"
                    );
                } else if event == "nativePortDisconnected" {
                    let detail = request
                        .message
                        .get("detail")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                    host.last_disconnect_detail = detail.clone();
                    let detail_for_log = host.last_disconnect_detail.clone().unwrap_or_default();
                    let _ = host;
                    if inner.active_host_id.as_deref() == Some(request.host_id.as_str()) {
                        inner.active_host_id = None;
                    }
                    inner
                        .commands
                        .retain(|queued| queued.host_id != request.host_id);
                    let pending_ids = inner
                        .pending
                        .iter()
                        .filter_map(|(id, pending)| {
                            if pending.host_id == request.host_id {
                                Some(id.clone())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    for id in pending_ids {
                        if let Some(pending) = inner.pending.remove(&id) {
                            let _ = pending.sender.send(ChromeResponse {
                                id,
                                ok: false,
                                result: None,
                                error: Some(ChromeResponseError {
                                    code: Some(
                                        error_codes::A11Y_CDP_EXTENSION_UNAVAILABLE.to_owned(),
                                    ),
                                    detail: Some(detail.clone().unwrap_or_else(|| {
                                        "Chrome debugger native port disconnected".to_owned()
                                    })),
                                }),
                            });
                        }
                    }
                    tracing::warn!(
                        code = "CHROME_DEBUGGER_NATIVE_PORT_DISCONNECTED",
                        host_id = %request.host_id,
                        detail = %detail_for_log,
                        "Chrome debugger native port disconnected"
                    );
                } else {
                    tracing::info!(
                        code = "CHROME_DEBUGGER_EXTENSION_EVENT",
                        host_id = %request.host_id,
                        event = %event,
                        "Chrome debugger extension event"
                    );
                }
            }
            "log" => {
                tracing::info!(
                    code = "CHROME_DEBUGGER_EXTENSION_LOG",
                    host_id = %request.host_id,
                    message = request.message.get("message").and_then(|value| value.as_str()).unwrap_or_default(),
                    "Chrome debugger extension log"
                );
            }
            other => {
                tracing::warn!(
                    code = "CHROME_DEBUGGER_MESSAGE_UNKNOWN",
                    host_id = %request.host_id,
                    message_type = %other,
                    "unknown Chrome debugger extension message"
                );
            }
        }
        Ok(())
    }

    async fn next_command(
        &self,
        host_id: &str,
        timeout_duration: Duration,
    ) -> Result<Option<ChromeCommand>, String> {
        let started = tokio::time::Instant::now();
        loop {
            let notified = {
                let mut inner = self.inner.lock().map_err(|_| {
                    "chrome debugger bridge lock poisoned during next command".to_owned()
                })?;
                if !inner.hosts.contains_key(host_id) {
                    return Err(format!(
                        "unknown chrome debugger native host_id {host_id:?}"
                    ));
                }
                if let Some(host) = inner.hosts.get_mut(host_id) {
                    host.last_seen_unix_ms = now_unix_ms();
                }
                if let Some(index) = inner
                    .commands
                    .iter()
                    .position(|queued| queued.host_id == host_id)
                {
                    let queued = inner
                        .commands
                        .remove(index)
                        .ok_or_else(|| "queued command disappeared".to_owned())?;
                    return Ok(Some(queued.command));
                }
                self.notify.notified()
            };
            let elapsed = started.elapsed();
            if elapsed >= timeout_duration {
                return Ok(None);
            }
            let remaining = timeout_duration.saturating_sub(elapsed);
            if timeout(remaining, notified).await.is_err() {
                return Ok(None);
            }
        }
    }

    async fn send_command(
        &self,
        kind: &str,
        params: Value,
    ) -> Result<Value, ChromeDebuggerBridgeError> {
        let id = format!(
            "chrome-cdp-{}-{}",
            std::process::id(),
            self.command_seq.fetch_add(1, Ordering::Relaxed)
        );
        let (sender, receiver) = oneshot::channel();
        let command = ChromeCommand {
            id: id.clone(),
            kind: kind.to_owned(),
            params,
        };
        {
            let mut inner = self.inner.lock().map_err(|_| {
                ChromeDebuggerBridgeError::protocol(
                    "chrome debugger bridge lock poisoned during command enqueue",
                )
            })?;
            let Some(host_id) = inner.active_host_id.clone() else {
                return Err(ChromeDebuggerBridgeError::unavailable());
            };
            if !inner.hosts.contains_key(&host_id) {
                inner.active_host_id = None;
                return Err(ChromeDebuggerBridgeError::unavailable());
            }
            inner.pending.insert(
                id.clone(),
                PendingResponse {
                    host_id: host_id.clone(),
                    kind: kind.to_owned(),
                    sender,
                },
            );
            inner.commands.push_back(QueuedCommand { host_id, command });
        }
        self.notify.notify_waiters();

        let response = match timeout(COMMAND_TIMEOUT, receiver).await {
            Ok(Ok(response)) => response,
            Ok(Err(_closed)) => {
                self.drop_pending(&id);
                return Err(ChromeDebuggerBridgeError::protocol(format!(
                    "Chrome debugger command {kind:?} response channel closed"
                )));
            }
            Err(_elapsed) => {
                self.drop_pending(&id);
                return Err(ChromeDebuggerBridgeError::timeout(kind));
            }
        };
        if response.ok {
            return response.result.ok_or_else(|| {
                ChromeDebuggerBridgeError::protocol(format!(
                    "Chrome debugger command {kind:?} returned ok without result"
                ))
            });
        }
        let error = response.error.as_ref();
        Err(ChromeDebuggerBridgeError::extension(
            error.and_then(|error| error.code.as_deref()),
            error
                .and_then(|error| error.detail.clone())
                .unwrap_or_else(|| format!("Chrome debugger command {kind:?} failed")),
        ))
    }

    fn drop_pending(&self, id: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(pending) = inner.pending.remove(id) {
                tracing::warn!(
                    code = "CHROME_DEBUGGER_PENDING_DROPPED",
                    command_id = %id,
                    command_kind = %pending.kind,
                    "Chrome debugger pending command removed"
                );
            }
        }
    }
}

fn bridge() -> &'static ChromeDebuggerBridge {
    static BRIDGE: OnceLock<ChromeDebuggerBridge> = OnceLock::new();
    BRIDGE.get_or_init(|| ChromeDebuggerBridge {
        inner: Mutex::new(BridgeInner::default()),
        notify: Notify::new(),
        command_seq: AtomicU64::new(1),
    })
}

async fn send_attach_command(
    hwnd: i64,
    kind: &'static str,
    mut payload: Value,
) -> Result<Value, ChromeDebuggerBridgeError> {
    let state = verify_debugger_warning_suppressed(hwnd)?;
    attach_suppression_attestation(&mut payload, &state)?;
    bridge().send_command(kind, payload).await
}

#[derive(Clone, Debug)]
struct DebuggerAttachProcessState {
    pid: u32,
    process_name: String,
    command_line: Vec<String>,
}

#[cfg(windows)]
fn verify_debugger_warning_suppressed(
    hwnd: i64,
) -> Result<DebuggerAttachProcessState, ChromeDebuggerBridgeError> {
    let state = debugger_attach_process_state(hwnd)?;
    if command_line_has_silent_debugger_flag(&state.command_line) {
        return Ok(state);
    }
    Err(ChromeDebuggerBridgeError::debugger_warning_unsuppressed(
        hwnd,
        Some(state.pid),
        Some(&state.process_name),
        Some(&state.command_line),
        "target browser process command line lacks the required silent debugger switch",
    ))
}

#[cfg(not(windows))]
fn verify_debugger_warning_suppressed(
    hwnd: i64,
) -> Result<DebuggerAttachProcessState, ChromeDebuggerBridgeError> {
    Err(ChromeDebuggerBridgeError::debugger_warning_unsuppressed(
        hwnd,
        None,
        None,
        None,
        "target browser process command line cannot be verified on this platform",
    ))
}

fn attach_suppression_attestation(
    payload: &mut Value,
    state: &DebuggerAttachProcessState,
) -> Result<(), ChromeDebuggerBridgeError> {
    let Some(object) = payload.as_object_mut() else {
        return Err(ChromeDebuggerBridgeError::protocol(
            "Chrome debugger attach command payload was not a JSON object",
        ));
    };
    object.insert("debuggerAttachSuppressionVerified".to_owned(), true.into());
    object.insert(
        "debuggerAttachSuppression".to_owned(),
        json!({
            "requiredSwitch": CHROME_DEBUGGER_SILENT_FLAG,
            "processId": state.pid,
            "processName": state.process_name,
        }),
    );
    Ok(())
}

#[cfg(windows)]
fn debugger_attach_process_state(
    hwnd: i64,
) -> Result<DebuggerAttachProcessState, ChromeDebuggerBridgeError> {
    use std::ffi::c_void;

    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow},
    };

    let hwnd_value = HWND(hwnd as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd_value)) }.as_bool() {
        return Err(ChromeDebuggerBridgeError::debugger_warning_unsuppressed(
            hwnd,
            None,
            None,
            None,
            "target hwnd is not a live window",
        ));
    }

    let mut raw_pid = 0_u32;
    unsafe { GetWindowThreadProcessId(hwnd_value, Some(&raw mut raw_pid)) };
    if raw_pid == 0 {
        return Err(ChromeDebuggerBridgeError::debugger_warning_unsuppressed(
            hwnd,
            None,
            None,
            None,
            "target hwnd owner pid was unavailable",
        ));
    }

    let pid = Pid::from_u32(raw_pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always)
            .without_tasks(),
    );
    let Some(process) = system.process(pid) else {
        return Err(ChromeDebuggerBridgeError::debugger_warning_unsuppressed(
            hwnd,
            Some(raw_pid),
            None,
            None,
            "target hwnd owner pid was not present in the process table",
        ));
    };

    let process_name = process.name().to_string_lossy().into_owned();
    let command_line = process
        .cmd()
        .iter()
        .map(|part| part.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if command_line.is_empty() {
        return Err(ChromeDebuggerBridgeError::debugger_warning_unsuppressed(
            hwnd,
            Some(raw_pid),
            Some(&process_name),
            None,
            "target browser process command line was unreadable",
        ));
    }

    Ok(DebuggerAttachProcessState {
        pid: raw_pid,
        process_name,
        command_line,
    })
}

fn command_line_has_silent_debugger_flag(command_line: &[String]) -> bool {
    let exact_or_valued = |arg: &str| {
        let arg = arg.trim_matches('"').to_ascii_lowercase();
        arg == CHROME_DEBUGGER_SILENT_FLAG
            || arg
                .strip_prefix(CHROME_DEBUGGER_SILENT_FLAG)
                .is_some_and(|rest| rest.starts_with('='))
    };

    command_line.iter().any(|arg| exact_or_valued(arg))
        || command_line
            .join(" ")
            .split_whitespace()
            .any(exact_or_valued)
}

pub(crate) async fn fetch_dom_snapshot(
    hwnd: i64,
    foreground_title: &str,
    foreground_url_hint: Option<&str>,
    target_id_hint: Option<&str>,
    max_nodes: usize,
) -> Result<ChromeDebuggerDomSnapshot, ChromeDebuggerBridgeError> {
    let result = send_attach_command(
        hwnd,
        "snapshot",
        json!({
            "hwnd": hwnd,
            "foregroundTitle": foreground_title,
            "foregroundUrlHint": foreground_url_hint,
            "targetIdHint": target_id_hint,
            "maxNodes": max_nodes,
        }),
    )
    .await?;
    let snapshot =
        serde_json::from_value::<ExtensionSnapshotResponse>(result).map_err(|error| {
            ChromeDebuggerBridgeError::protocol(format!(
                "decode Chrome debugger DOM snapshot response: {error}"
            ))
        })?;
    let dom_nodes = snapshot
        .nodes
        .into_iter()
        .map(|node| synapse_a11y::CdpDomNode {
            backend_node_id: node.backend_node_id,
            parent_backend_node_id: node.parent_backend_node_id,
            role: node.role,
            name: node.name,
            value: node.value,
            bbox: node.bbox,
            child_count: node.child_count,
            enabled: node.enabled,
            focused: node.focused,
        })
        .collect::<Vec<_>>();
    let nodes = synapse_a11y::build_accessible_nodes_for_target(
        hwnd,
        Some(&snapshot.target_id),
        &dom_nodes,
        max_nodes,
    );
    Ok(ChromeDebuggerDomSnapshot {
        nodes,
        total_ax_nodes: snapshot.total_ax_nodes,
        page_url: snapshot.page_url,
        target_id: snapshot.target_id,
        session_id: snapshot.session_id,
        target_candidate_count: snapshot.target_candidate_count,
        target_selection_reason: snapshot.target_selection_reason,
        extension_id: snapshot
            .extension_id
            .unwrap_or_else(|| EXTENSION_ID.to_owned()),
    })
}

pub(crate) async fn click_node(
    hwnd: i64,
    foreground_title: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
    button: ChromeDebuggerMouseButton,
    click_count: i64,
) -> Result<ChromeDebuggerClickPoint, ChromeDebuggerBridgeError> {
    let result = send_attach_command(
        hwnd,
        "clickNode",
        json!({
            "hwnd": hwnd,
            "foregroundTitle": foreground_title,
            "targetIdHint": target_id_hint,
            "backendNodeId": backend_node_id,
            "button": button.as_str(),
            "clickCount": click_count.max(1),
        }),
    )
    .await?;
    serde_json::from_value::<ChromeDebuggerClickPoint>(result).map_err(|error| {
        ChromeDebuggerBridgeError::protocol(format!(
            "decode Chrome debugger click response: {error}"
        ))
    })
}

pub(crate) async fn type_node(
    hwnd: i64,
    foreground_title: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
    text: &str,
) -> Result<ChromeDebuggerTypeResult, ChromeDebuggerBridgeError> {
    let result = send_attach_command(
        hwnd,
        "typeNode",
        json!({
            "hwnd": hwnd,
            "foregroundTitle": foreground_title,
            "targetIdHint": target_id_hint,
            "backendNodeId": backend_node_id,
            "text": text,
        }),
    )
    .await?;
    serde_json::from_value::<ChromeDebuggerTypeResult>(result).map_err(|error| {
        ChromeDebuggerBridgeError::protocol(format!(
            "decode Chrome debugger type response: {error}"
        ))
    })
}

pub(crate) async fn node_value(
    hwnd: i64,
    foreground_title: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
) -> Result<ChromeDebuggerNodeValue, ChromeDebuggerBridgeError> {
    let result = send_attach_command(
        hwnd,
        "nodeValue",
        json!({
            "hwnd": hwnd,
            "foregroundTitle": foreground_title,
            "targetIdHint": target_id_hint,
            "backendNodeId": backend_node_id,
        }),
    )
    .await?;
    serde_json::from_value::<ChromeDebuggerNodeValue>(result).map_err(|error| {
        ChromeDebuggerBridgeError::protocol(format!(
            "decode Chrome debugger node value response: {error}"
        ))
    })
}

pub(crate) async fn open_tab(
    hwnd: i64,
    url: &str,
) -> Result<ChromeDebuggerOpenTabResult, ChromeDebuggerBridgeError> {
    let result = bridge()
        .send_command(
            "openTab",
            json!({
                "hwnd": hwnd,
                "url": url,
            }),
        )
        .await?;
    serde_json::from_value::<ChromeDebuggerOpenTabResult>(result).map_err(|error| {
        ChromeDebuggerBridgeError::protocol(format!(
            "decode Chrome debugger open tab response: {error}"
        ))
    })
}

pub(crate) async fn close_tab(
    hwnd: i64,
    target_id: &str,
) -> Result<ChromeDebuggerCloseTabResult, ChromeDebuggerBridgeError> {
    let result = bridge()
        .send_command(
            "closeTab",
            json!({
                "hwnd": hwnd,
                "targetIdHint": target_id,
            }),
        )
        .await?;
    serde_json::from_value::<ChromeDebuggerCloseTabResult>(result).map_err(|error| {
        ChromeDebuggerBridgeError::protocol(format!(
            "decode Chrome debugger close tab response: {error}"
        ))
    })
}

pub(crate) async fn target_info(
    hwnd: i64,
    target_id: &str,
) -> Result<ChromeDebuggerTargetInfo, ChromeDebuggerBridgeError> {
    let result = bridge()
        .send_command(
            "targetInfo",
            json!({
                "hwnd": hwnd,
                "targetIdHint": target_id,
            }),
        )
        .await?;
    serde_json::from_value::<ChromeDebuggerTargetInfo>(result).map_err(|error| {
        ChromeDebuggerBridgeError::protocol(format!(
            "decode Chrome debugger target info response: {error}"
        ))
    })
}

pub(crate) async fn navigate_tab(
    hwnd: i64,
    target_id: &str,
    action: &str,
    url: Option<&str>,
    wait_timeout_ms: u64,
    ignore_cache: bool,
) -> Result<ChromeDebuggerNavigateResult, ChromeDebuggerBridgeError> {
    let result = bridge()
        .send_command(
            "navigateTab",
            json!({
                "hwnd": hwnd,
                "targetIdHint": target_id,
                "action": action,
                "url": url,
                "waitTimeoutMs": wait_timeout_ms,
                "ignoreCache": ignore_cache,
            }),
        )
        .await?;
    serde_json::from_value::<ChromeDebuggerNavigateResult>(result).map_err(|error| {
        ChromeDebuggerBridgeError::protocol(format!(
            "decode Chrome debugger navigate response: {error}"
        ))
    })
}

pub(crate) fn cdp_capabilities() -> Vec<CdpCapability> {
    synapse_a11y::cdp_capabilities()
}

pub(crate) async fn http_register(Json(request): Json<NativeRegisterRequest>) -> Response {
    match bridge().register(request) {
        Ok(response) => Json(response).into_response(),
        Err(detail) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "code": error_codes::A11Y_CDP_EXTENSION_UNAVAILABLE,
                "detail": detail,
            })),
        )
            .into_response(),
    }
}

pub(crate) async fn http_message(Json(request): Json<NativeMessageRequest>) -> Response {
    match bridge().post_message(request) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(detail) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "code": error_codes::A11Y_CDP_ATTACH_FAILED,
                "detail": detail,
            })),
        )
            .into_response(),
    }
}

pub(crate) async fn http_next(Query(query): Query<NativeNextQuery>) -> Response {
    let timeout_ms = query
        .timeout_ms
        .unwrap_or_else(|| u64::try_from(NATIVE_POLL_TIMEOUT.as_millis()).unwrap_or(15_000))
        .min(30_000);
    match bridge()
        .next_command(&query.host_id, Duration::from_millis(timeout_ms))
        .await
    {
        Ok(command) => Json(NativeNextResponse { ok: true, command }).into_response(),
        Err(detail) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "code": error_codes::A11Y_CDP_EXTENSION_UNAVAILABLE,
                "detail": detail,
            })),
        )
            .into_response(),
    }
}

pub(crate) async fn run_native_host(
    bind: &str,
    invocation: NativeHostInvocation,
) -> anyhow::Result<ExitCode> {
    let token = load_token_value().context("load Synapse HTTP bearer token")?;
    let base_url = http_base_url(bind);
    let client = reqwest::Client::new();
    let pid = std::process::id();
    let registered = register_native_host(
        &client,
        &base_url,
        &token,
        &invocation,
        pid,
        "native_host_start",
    )
    .await?;
    tracing::info!(
        code = "CHROME_DEBUGGER_NATIVE_HOST_STARTED",
        host_id = %registered.host_id,
        origin = %invocation.origin,
        pid,
        "Chrome debugger native host bridge started"
    );

    let host_id = Arc::new(RwLock::new(registered.host_id));
    let reader_client = client.clone();
    let reader_token = token.clone();
    let reader_base_url = base_url.clone();
    let reader_invocation = invocation.clone();
    let reader_host_id = Arc::clone(&host_id);
    let mut reader_task = tokio::spawn(async move {
        read_native_messages(
            reader_client,
            reader_base_url,
            reader_token,
            reader_invocation,
            pid,
            reader_host_id,
        )
        .await
    });
    let mut poll_task = tokio::spawn(async move {
        poll_commands_to_chrome(client, base_url, token, invocation, pid, host_id).await
    });

    tokio::select! {
        reader_result = &mut reader_task => {
            poll_task.abort();
            match reader_result {
                Ok(Ok(())) => {
                    tracing::info!(
                        code = "CHROME_DEBUGGER_NATIVE_HOST_EXITED",
                        pid,
                        "Chrome debugger native host exiting after stdin EOF"
                    );
                    Ok(ExitCode::SUCCESS)
                }
                Ok(Err(error)) => Err(error),
                Err(error) => Err(anyhow::anyhow!(
                    "Chrome debugger native host reader task failed: {error}"
                )),
            }
        }
        poll_result = &mut poll_task => {
            reader_task.abort();
            match poll_result {
                Ok(result) => result,
                Err(error) => Err(anyhow::anyhow!(
                    "Chrome debugger native host poll task failed: {error}"
                )),
            }
        }
    }
}

async fn register_native_host(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    invocation: &NativeHostInvocation,
    pid: u32,
    reason: &'static str,
) -> anyhow::Result<NativeRegisterResponse> {
    let register = NativeRegisterRequest {
        origin: invocation.origin.clone(),
        pid,
        parent_window: invocation.parent_window.clone(),
        bridge_protocol_version: BRIDGE_PROTOCOL_VERSION,
    };
    let response = client
        .post(format!("{base_url}/chrome-debugger/native/register"))
        .bearer_auth(&token)
        .json(&register)
        .send()
        .await
        .context("register Chrome debugger native host with Synapse daemon")?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "Chrome debugger native host register failed status={status} detail={detail}"
        );
    }
    let registered = response
        .json::<NativeRegisterResponse>()
        .await
        .context("decode Chrome debugger native register response")?;
    tracing::info!(
        code = "CHROME_DEBUGGER_NATIVE_HOST_REGISTERED_WITH_DAEMON",
        host_id = %registered.host_id,
        origin = %invocation.origin,
        pid,
        reason,
        "Chrome debugger native host registered with daemon"
    );
    Ok(registered)
}

fn load_token_value() -> anyhow::Result<String> {
    match token_file_path() {
        Some(path) if path.is_file() => {
            let token = std::fs::read_to_string(&path)
                .with_context(|| format!("read HTTP bearer token file {}", path.display()))?;
            normalize_token(&token)
                .with_context(|| format!("HTTP bearer token file is empty: {}", path.display()))
        }
        Some(_) | None => {
            let token = std::env::var(TOKEN_ENV)
                .with_context(|| format!("{TOKEN_ENV} is unset and token.txt is absent"))?;
            normalize_token(&token).with_context(|| format!("{TOKEN_ENV} is empty"))
        }
    }
}

fn token_file_path() -> Option<PathBuf> {
    let appdata = std::env::var_os(APPDATA_ENV)?;
    Some(PathBuf::from(appdata).join("synapse").join("token.txt"))
}

fn normalize_token(raw: &str) -> anyhow::Result<String> {
    let token = raw.trim();
    if token.is_empty() {
        bail!("empty token")
    }
    Ok(token.to_owned())
}

async fn read_native_messages(
    client: reqwest::Client,
    base_url: String,
    token: String,
    invocation: NativeHostInvocation,
    pid: u32,
    host_id: Arc<RwLock<String>>,
) -> anyhow::Result<()> {
    let mut stdin = tokio::io::stdin();
    loop {
        let Some(message) = read_native_frame(&mut stdin).await? else {
            let current_host_id = host_id.read().await.clone();
            let _ = post_native_message(
                &client,
                &base_url,
                &token,
                &current_host_id,
                json!({
                    "type": "event",
                    "event": "nativePortDisconnected",
                    "detail": "stdin EOF from Chrome native messaging port",
                }),
            )
            .await;
            return Ok(());
        };
        let mut current_host_id = host_id.read().await.clone();
        match post_native_message(
            &client,
            &base_url,
            &token,
            &current_host_id,
            message.clone(),
        )
        .await
        {
            Ok(()) => {}
            Err(error) if is_unknown_native_host_error(&error) => {
                reregister_native_host_until_available(
                    &client,
                    &base_url,
                    &token,
                    &invocation,
                    pid,
                    &host_id,
                    &current_host_id,
                    "message_unknown_host_id",
                )
                .await?;
                current_host_id = host_id.read().await.clone();
                post_native_message(&client, &base_url, &token, &current_host_id, message).await?;
            }
            Err(error) => return Err(error),
        }
    }
}

async fn poll_commands_to_chrome(
    client: reqwest::Client,
    base_url: String,
    token: String,
    invocation: NativeHostInvocation,
    pid: u32,
    host_id: Arc<RwLock<String>>,
) -> anyhow::Result<ExitCode> {
    let mut stdout = tokio::io::stdout();
    loop {
        let current_host_id = host_id.read().await.clone();
        let response = match client
            .get(format!(
                "{base_url}/chrome-debugger/native/next?host_id={current_host_id}&timeout_ms=15000"
            ))
            .bearer_auth(&token)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) if is_transient_daemon_transport_error(&error) => {
                tracing::warn!(
                    code = "CHROME_DEBUGGER_NATIVE_DAEMON_UNREACHABLE",
                    host_id = %current_host_id,
                    error = %error,
                    "Chrome debugger native host waiting for Synapse daemon transport"
                );
                tokio::time::sleep(NATIVE_DAEMON_RECONNECT_DELAY).await;
                continue;
            }
            Err(error) => return Err(error).context("poll Chrome debugger daemon command queue"),
        };
        if !response.status().is_success() {
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            if is_unknown_native_host_detail(&detail) {
                reregister_native_host_until_available(
                    &client,
                    &base_url,
                    &token,
                    &invocation,
                    pid,
                    &host_id,
                    &current_host_id,
                    "poll_unknown_host_id",
                )
                .await?;
                continue;
            }
            anyhow::bail!("Chrome debugger native poll failed status={status} detail={detail}");
        }
        let next = response
            .json::<NativeNextResponse>()
            .await
            .context("decode Chrome debugger native poll response")?;
        if let Some(command) = next.command {
            write_native_frame(&mut stdout, &serde_json::to_value(command)?).await?;
        }
    }
}

async fn reregister_native_host_until_available(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    invocation: &NativeHostInvocation,
    pid: u32,
    host_id: &Arc<RwLock<String>>,
    observed_host_id: &str,
    reason: &'static str,
) -> anyhow::Result<()> {
    if host_id.read().await.as_str() != observed_host_id {
        return Ok(());
    }
    loop {
        match register_native_host(client, base_url, token, invocation, pid, reason).await {
            Ok(registered) => {
                tracing::warn!(
                    code = "CHROME_DEBUGGER_NATIVE_HOST_REREGISTERED",
                    old_host_id = %observed_host_id,
                    new_host_id = %registered.host_id,
                    reason,
                    "Chrome debugger native host re-registered after daemon bridge state changed"
                );
                *host_id.write().await = registered.host_id;
                return Ok(());
            }
            Err(error) if is_transient_daemon_register_error(&error) => {
                tracing::warn!(
                    code = "CHROME_DEBUGGER_NATIVE_HOST_REREGISTER_RETRY",
                    old_host_id = %observed_host_id,
                    reason,
                    error = %format!("{error:#}"),
                    "Chrome debugger native host waiting to re-register with Synapse daemon"
                );
                tokio::time::sleep(NATIVE_DAEMON_RECONNECT_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_unknown_native_host_error(error: &anyhow::Error) -> bool {
    is_unknown_native_host_detail(&format!("{error:#}"))
}

fn is_unknown_native_host_detail(detail: &str) -> bool {
    detail.contains(UNKNOWN_NATIVE_HOST_ID_FRAGMENT)
}

fn is_transient_daemon_transport_error(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_timeout() || error.is_request()
}

fn is_transient_daemon_register_error(error: &anyhow::Error) -> bool {
    let detail = format!("{error:#}").to_ascii_lowercase();
    detail.contains("error sending request")
        || detail.contains("connection refused")
        || detail.contains("connection reset")
        || detail.contains("timed out")
        || detail.contains("operation timed out")
}

async fn post_native_message(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    host_id: &str,
    message: Value,
) -> anyhow::Result<()> {
    let response = client
        .post(format!("{base_url}/chrome-debugger/native/message"))
        .bearer_auth(token)
        .json(&NativeMessageRequest {
            host_id: host_id.to_owned(),
            message,
        })
        .send()
        .await
        .context("post Chrome debugger extension message to Synapse daemon")?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        anyhow::bail!("Chrome debugger message post failed status={status} detail={detail}");
    }
}

async fn read_native_frame<R>(reader: &mut R) -> anyhow::Result<Option<Value>>
where
    R: AsyncRead + Unpin,
{
    let mut len = [0_u8; 4];
    match reader.read_exact(&mut len).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error).context("read Chrome native message length"),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_NATIVE_MESSAGE_FROM_CHROME {
        anyhow::bail!(
            "Chrome native message length {len} exceeds max {MAX_NATIVE_MESSAGE_FROM_CHROME}"
        );
    }
    let mut body = vec![0_u8; len];
    reader
        .read_exact(&mut body)
        .await
        .context("read Chrome native message body")?;
    let message = serde_json::from_slice(&body).context("decode Chrome native JSON message")?;
    Ok(Some(message))
}

async fn write_native_frame<W>(writer: &mut W, value: &Value) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(value).context("encode Chrome native JSON message")?;
    if body.len() > MAX_NATIVE_MESSAGE_TO_CHROME {
        anyhow::bail!(
            "Chrome native response length {} exceeds max {MAX_NATIVE_MESSAGE_TO_CHROME}",
            body.len()
        );
    }
    let len = u32::try_from(body.len()).context("Chrome native response length overflow")?;
    writer
        .write_all(&len.to_le_bytes())
        .await
        .context("write Chrome native message length")?;
    writer
        .write_all(&body)
        .await
        .context("write Chrome native message body")?;
    writer.flush().await.context("flush Chrome native stdout")?;
    Ok(())
}

fn http_base_url(bind: &str) -> String {
    if bind.starts_with("http://") || bind.starts_with("https://") {
        bind.trim_end_matches('/').to_owned()
    } else {
        format!("http://{}", bind.trim_end_matches('/'))
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_host_invocation_detects_chrome_origin_and_parent_window() {
        let invocation = native_host_invocation_from_args([
            OsString::from("chrome-extension://leoocgnkjnplbfdbklajepahofecgfbk/"),
            OsString::from("--parent-window=1234"),
        ])
        .expect("chrome native host origin should be detected");

        assert_eq!(
            invocation.origin,
            "chrome-extension://leoocgnkjnplbfdbklajepahofecgfbk/"
        );
        assert_eq!(invocation.parent_window.as_deref(), Some("1234"));
    }

    #[test]
    fn native_host_unknown_id_error_is_restart_recoverable() {
        let detail = r#"{"ok":false,"code":"A11Y_CDP_EXTENSION_UNAVAILABLE","detail":"unknown chrome debugger native host_id \"chrome-native-old\""}"#;
        let error =
            anyhow::anyhow!("Chrome debugger native poll failed status=400 detail={detail}");

        assert!(is_unknown_native_host_detail(detail));
        assert!(is_unknown_native_host_error(&error));
        assert!(!is_unknown_native_host_detail("bridge protocol mismatch"));
    }

    #[test]
    fn extension_unavailable_maps_to_explicit_cdp_status() {
        let error = ChromeDebuggerBridgeError::unavailable();

        assert_eq!(error.code(), error_codes::A11Y_CDP_EXTENSION_UNAVAILABLE);
        assert_eq!(error.cdp_status(), CdpStatus::ExtensionUnavailable);
        assert!(
            error
                .detail()
                .contains("install the bundled Synapse Chrome extension")
        );
    }

    #[test]
    fn debugger_warning_preflight_error_maps_to_attach_failed_status() {
        let command_line = vec![
            r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_owned(),
            "--profile-directory=Default".to_owned(),
        ];
        let error = ChromeDebuggerBridgeError::debugger_warning_unsuppressed(
            1234,
            Some(5678),
            Some("chrome.exe"),
            Some(&command_line),
            "missing flag",
        );

        assert_eq!(
            error.code(),
            error_codes::A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED
        );
        assert_eq!(error.cdp_status(), CdpStatus::AttachFailed);
        assert!(error.detail().contains("--silent-debugger-extension-api"));
        assert!(error.detail().contains("pid=5678"));
    }

    #[test]
    fn command_line_silent_debugger_flag_detection_is_exact() {
        assert!(command_line_has_silent_debugger_flag(&[
            "chrome.exe".to_owned(),
            "--silent-debugger-extension-api".to_owned(),
        ]));
        assert!(command_line_has_silent_debugger_flag(&[
            "chrome.exe --silent-debugger-extension-api --profile-directory=Default".to_owned(),
        ]));
        assert!(command_line_has_silent_debugger_flag(&[
            "chrome.exe".to_owned(),
            "--silent-debugger-extension-api=true".to_owned(),
        ]));
        assert!(!command_line_has_silent_debugger_flag(&[
            "chrome.exe".to_owned(),
            "--not-silent-debugger-extension-api".to_owned(),
        ]));
    }
}
