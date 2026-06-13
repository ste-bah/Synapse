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
    extract::{
        Query,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use synapse_core::{AccessibleNode, CdpCapability, CdpStatus, Rect, error_codes};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Notify, RwLock, oneshot},
    time::timeout,
};
use uuid::Uuid;

const EXTENSION_ID: &str = "leoocgnkjnplbfdbklajepahofecgfbk";
const NATIVE_HOST_NAME: &str = "com.synapse.chrome_debugger";
const EXTENSION_ORIGIN: &str = "chrome-extension://leoocgnkjnplbfdbklajepahofecgfbk";
const BRIDGE_TOKEN_HEADER: &str = "x-synapse-bridge-token";
const BRIDGE_PROTOCOL_VERSION: u32 = 1;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const NATIVE_POLL_TIMEOUT: Duration = Duration::from_secs(15);
const DIRECT_WS_COMMAND_WAIT: Duration = Duration::from_secs(25);
const NATIVE_DAEMON_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_NATIVE_MESSAGE_FROM_CHROME: usize = 64 * 1024 * 1024;
const MAX_NATIVE_MESSAGE_TO_CHROME: usize = 1024 * 1024;
const UNKNOWN_NATIVE_HOST_ID_FRAGMENT: &str = "unknown chrome debugger native host_id";
const INSTALL_GUIDANCE: &str = "install the bundled Synapse Chrome extension from extensions\\synapse-chrome-debugger with scripts\\install-synapse-chrome-debugger.ps1; the normal end-user bridge uses chrome.tabs over direct localhost WebSocket without nativeMessaging or debugger permissions, and attach-capable debugger commands are disabled in the normal bridge; expected extension_id=leoocgnkjnplbfdbklajepahofecgfbk";
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
                "Chrome debugger extension bridge is not connected; {INSTALL_GUIDANCE}"
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

    fn normal_bridge_attach_disabled(hwnd: i64, command_kind: &str) -> Self {
        let external_surface_hint = external_chrome_surface_hint();
        Self {
            code: error_codes::A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED,
            detail: format!(
                "normal Synapse Chrome Bridge refused attach-capable command {command_kind:?} before queueing any Chrome command; hwnd={hwnd} reason=the normal end-user bridge is tabs-only and contains no daemon-side chrome.debugger attach transport{external_surface_hint} remediation=use raw CDP from a Synapse-launched automation profile for DOM/action CDP; if an end-user popup remains, disable/remove the named external Chrome extension or apply Chrome ExtensionSettings blocked_permissions=[debugger,nativeMessaging] and rerun scripts\\install-synapse-chrome-debugger.ps1"
            ),
        }
    }

    fn normal_bridge_external_popup_risk(hwnd: i64, command_kind: &str, risks: &[String]) -> Self {
        Self {
            code: error_codes::A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED,
            detail: format!(
                "normal Synapse Chrome Bridge refused command {command_kind:?} before queueing it to Chrome; hwnd={hwnd} reason=the current Chrome profile/process Source of Truth is not popup-free because another active extension or native host can use debugger/nativeMessaging; external_chrome_popup_risk={} remediation=apply Chrome ExtensionSettings wildcard blocked_permissions=[debugger,nativeMessaging], disable/remove the named external extension/native host, refresh or restart Chrome, then rerun scripts\\install-synapse-chrome-debugger.ps1; use raw CDP from a Synapse-launched automation profile for background browser work until the normal profile is certified",
                format_external_chrome_popup_risks(risks)
            ),
        }
    }

    fn normal_bridge_registration_external_popup_risk(risks: &[String]) -> Self {
        Self {
            code: error_codes::A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED,
            detail: format!(
                "normal Synapse Chrome Bridge refused direct registration before accepting a Chrome-hosted command channel; reason=the current Chrome profile/process Source of Truth is not popup-free because another active extension or native host can use debugger/nativeMessaging; external_chrome_popup_risk={} remediation=apply Chrome ExtensionSettings wildcard blocked_permissions=[debugger,nativeMessaging], disable/remove the named external extension/native host, refresh or restart Chrome, then rerun scripts\\install-synapse-chrome-debugger.ps1; use raw CDP from a Synapse-launched automation profile for background browser work until the normal profile is certified",
                format_external_chrome_popup_risks(risks)
            ),
        }
    }
}

fn external_chrome_surface_hint() -> String {
    let rows = external_chrome_popup_risks();
    if rows.is_empty() {
        return String::new();
    }
    format!(
        " external_chrome_popup_risk={}",
        format_external_chrome_popup_risks(&rows)
    )
}

fn external_chrome_popup_risks() -> Vec<String> {
    let mut rows = external_chrome_profile_surfaces();
    rows.extend(external_chrome_native_messaging_processes());
    rows.sort();
    rows.dedup();
    rows
}

fn format_external_chrome_popup_risks(rows: &[String]) -> String {
    let shown = rows.iter().take(8).cloned().collect::<Vec<_>>().join(" | ");
    let extra = rows.len().saturating_sub(8);
    let suffix = if extra == 0 {
        String::new()
    } else {
        format!(" | +{extra} more")
    };
    format!("{shown}{suffix}")
}

fn ensure_normal_bridge_popup_safe(
    hwnd: i64,
    command_kind: &'static str,
) -> Result<(), ChromeDebuggerBridgeError> {
    let risks = external_chrome_popup_risks();
    if risks.is_empty() {
        return Ok(());
    }
    let error =
        ChromeDebuggerBridgeError::normal_bridge_external_popup_risk(hwnd, command_kind, &risks);
    tracing::warn!(
        code = error.code(),
        hwnd,
        command_kind,
        risk_count = risks.len(),
        detail = %error.detail(),
        "normal Chrome bridge refused command because live Chrome popup-risk Source of Truth is not clean"
    );
    Err(error)
}

fn ensure_normal_bridge_registration_popup_safe() -> Result<(), ChromeDebuggerBridgeError> {
    let risks = external_chrome_popup_risks();
    if risks.is_empty() {
        return Ok(());
    }
    let error = ChromeDebuggerBridgeError::normal_bridge_registration_external_popup_risk(&risks);
    tracing::warn!(
        code = error.code(),
        risk_count = risks.len(),
        detail = %error.detail(),
        "normal Chrome bridge refused registration because live Chrome popup-risk Source of Truth is not clean"
    );
    Err(error)
}

fn external_chrome_profile_surfaces() -> Vec<String> {
    let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") else {
        return Vec::new();
    };
    let user_data_root = PathBuf::from(local_appdata)
        .join("Google")
        .join("Chrome")
        .join("User Data");
    let Ok(profile_dirs) = std::fs::read_dir(user_data_root) else {
        return Vec::new();
    };

    let mut rows = Vec::new();
    for profile_dir in profile_dirs.flatten() {
        let Ok(file_type) = profile_dir.file_type() else {
            continue;
        };
        if !file_type.is_dir() || profile_dir.file_name() == "Snapshots" {
            continue;
        }
        let profile = profile_dir.file_name().to_string_lossy().into_owned();
        let mut runtime_by_id: HashMap<String, ChromeExtensionRuntimeState> = HashMap::new();
        for pref_file in ["Preferences", "Secure Preferences"] {
            let pref_path = profile_dir.path().join(pref_file);
            let Ok(raw) = std::fs::read_to_string(&pref_path) else {
                continue;
            };
            let Ok(pref) = serde_json::from_str::<Value>(&raw) else {
                rows.push(format!(
                    "profile={profile} pref={pref_file} parse_error=true"
                ));
                continue;
            };
            let Some(settings) = pref
                .get("extensions")
                .and_then(|value| value.get("settings"))
                .and_then(Value::as_object)
            else {
                continue;
            };
            for (extension_id, setting) in settings {
                if extension_id == EXTENSION_ID {
                    continue;
                }
                let mut runtime_state = chrome_extension_runtime_state(setting);
                if pref_file == "Preferences" {
                    runtime_by_id.insert(extension_id.clone(), runtime_state.clone());
                } else if let Some(preferences_runtime_state) = runtime_by_id.get(extension_id) {
                    runtime_state = preferences_runtime_state.clone();
                }
                let permissions = active_api_permissions(setting);
                let has_debugger = permissions
                    .iter()
                    .any(|permission| permission == "debugger");
                let has_native = permissions
                    .iter()
                    .any(|permission| permission == "nativeMessaging");
                if !has_debugger && !has_native {
                    continue;
                }
                if !runtime_state.runtime_enabled {
                    continue;
                }
                let name = setting
                    .get("manifest")
                    .and_then(|manifest| manifest.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("<unnamed>");
                rows.push(format!(
                    "profile={profile} pref={pref_file} extension_id={extension_id} name={name:?} active_api={} runtime_enabled=true active_bit={} disable_reasons={}",
                    permissions.join(","),
                    runtime_state
                        .active_bit
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "<absent>".to_owned()),
                    format_disable_reasons(&runtime_state.disable_reasons)
                ));
            }
        }
    }
    rows
}

fn active_api_permissions(setting: &Value) -> Vec<String> {
    let mut permissions = setting
        .get("active_permissions")
        .and_then(|value| value.get("api"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    permissions.sort();
    permissions.dedup();
    permissions
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ChromeExtensionRuntimeState {
    active_bit: Option<bool>,
    disable_reasons: Vec<u64>,
    runtime_enabled: bool,
}

fn chrome_extension_runtime_state(setting: &Value) -> ChromeExtensionRuntimeState {
    let active_bit = setting.get("active_bit").and_then(Value::as_bool);
    let mut disable_reasons = setting
        .get("disable_reasons")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_u64).collect::<Vec<_>>())
        .unwrap_or_default();
    disable_reasons.sort_unstable();
    disable_reasons.dedup();
    let runtime_enabled = active_bit != Some(false) && disable_reasons.is_empty();
    ChromeExtensionRuntimeState {
        active_bit,
        disable_reasons,
        runtime_enabled,
    }
}

fn format_disable_reasons(disable_reasons: &[u64]) -> String {
    if disable_reasons.is_empty() {
        "[]".to_owned()
    } else {
        format!(
            "[{}]",
            disable_reasons
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

fn external_chrome_native_messaging_processes() -> Vec<String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::everything(),
    );
    system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let command_line = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            if !command_line.contains("chrome.nativeMessaging")
                || command_line.contains(EXTENSION_ID)
            {
                return None;
            }
            let extension_id = command_line
                .split("chrome-extension://")
                .nth(1)
                .and_then(|tail| tail.get(0..32))
                .filter(|candidate| {
                    candidate.len() == 32
                        && candidate
                            .chars()
                            .all(|character| ('a'..='p').contains(&character))
                })
                .unwrap_or("<unknown>");
            Some(format!(
                "native_messaging_process pid={} name={} extension_id={extension_id}",
                pid.as_u32(),
                process.name().to_string_lossy()
            ))
        })
        .collect()
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
    bridge_token: String,
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
    transport: Option<String>,
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

#[derive(Debug, Deserialize)]
pub(crate) struct NativeWsQuery {
    host_id: String,
    bridge_token: String,
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

#[derive(Clone, Debug, Deserialize)]
struct ExtensionTabNavigationEvent {
    #[serde(default)]
    source: String,
    #[serde(default)]
    target_id: String,
    tab_id: u32,
    #[serde(default)]
    chrome_window_id: Option<i64>,
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    highlighted: bool,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    observed_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub(crate) struct ChromeDebuggerBrowserNavigationEvent {
    pub source: String,
    pub event: String,
    pub url: String,
    pub title: String,
    pub tab_id: Option<u32>,
    pub chrome_window_id: Option<i64>,
    pub cdp_target_id: Option<String>,
    pub endpoint: Option<String>,
    pub transport: Option<String>,
    pub ready_state: Option<String>,
    pub observed_at_unix_ms: Option<u64>,
    pub active: Option<bool>,
    pub highlighted: Option<bool>,
    pub pinned: Option<bool>,
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
    transport: Option<String>,
    bridge_token_digest: [u8; 32],
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

type BrowserNavigationSink = dyn Fn(ChromeDebuggerBrowserNavigationEvent) + Send + Sync + 'static;

fn browser_navigation_sink_slot() -> &'static Mutex<Option<Arc<BrowserNavigationSink>>> {
    static SINK: OnceLock<Mutex<Option<Arc<BrowserNavigationSink>>>> = OnceLock::new();
    SINK.get_or_init(|| Mutex::new(None))
}

pub(crate) fn set_browser_navigation_sink(sink: Arc<BrowserNavigationSink>) {
    match browser_navigation_sink_slot().lock() {
        Ok(mut guard) => *guard = Some(sink),
        Err(poisoned) => *poisoned.into_inner() = Some(sink),
    }
}

fn emit_browser_navigation_event(event: ChromeDebuggerBrowserNavigationEvent) {
    let sink = match browser_navigation_sink_slot().lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    if let Some(sink) = sink {
        sink(event);
    } else {
        tracing::warn!(
            code = "CHROME_DEBUGGER_BROWSER_NAV_SINK_MISSING",
            "Chrome debugger browser navigation event had no recorder sink"
        );
    }
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
        let bridge_token = Uuid::new_v4().to_string();
        let record = HostRecord {
            origin: request.origin,
            extension_id: None,
            pid: request.pid,
            parent_window: request.parent_window,
            transport: request.transport,
            bridge_token_digest: digest_bridge_token(&bridge_token),
            registered_unix_ms: now,
            last_seen_unix_ms: now,
            last_disconnect_detail: None,
            last_detach_reason: None,
        };
        let transport_label = record
            .transport
            .as_deref()
            .unwrap_or("native_messaging")
            .to_owned();
        let is_direct_http = transport_label == "direct_http";
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "chrome debugger bridge lock poisoned during register".to_owned())?;
        if is_direct_http {
            replace_active_direct_http_host(&mut inner, &host_id);
        }
        inner.active_host_id = Some(host_id.clone());
        inner.hosts.insert(host_id.clone(), record);
        tracing::info!(
            code = "CHROME_DEBUGGER_NATIVE_HOST_REGISTERED",
            host_id = %host_id,
            pid = request.pid,
            transport = %transport_label,
            "Chrome debugger native host registered"
        );
        Ok(NativeRegisterResponse {
            ok: true,
            host_id,
            bridge_token,
            bridge_protocol_version: BRIDGE_PROTOCOL_VERSION,
            native_host_name: NATIVE_HOST_NAME.to_owned(),
            expected_extension_id: EXTENSION_ID.to_owned(),
        })
    }

    fn post_message(&self, request: NativeMessageRequest) -> Result<(), String> {
        let mut browser_navigation_event = None;
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
                    transport = host.transport.as_deref().unwrap_or("native_messaging"),
                    registered_unix_ms = host.registered_unix_ms,
                    "Chrome debugger extension connected through native host"
                );
            }
            "response" => {
                let response = serde_json::from_value::<ChromeResponse>(request.message)
                    .map_err(|error| format!("decode chrome debugger response: {error}"))?;
                let id = response.id.clone();
                if inner
                    .pending
                    .get(&id)
                    .is_some_and(|pending| pending.host_id != request.host_id)
                {
                    tracing::warn!(
                        code = "CHROME_DEBUGGER_RESPONSE_HOST_MISMATCH",
                        host_id = %request.host_id,
                        command_id = %id,
                        "Chrome debugger response came from a different host than the pending command owner"
                    );
                    return Ok(());
                }
                let Some(pending) = inner.pending.remove(&id) else {
                    tracing::warn!(
                        code = "CHROME_DEBUGGER_RESPONSE_WITHOUT_PENDING_COMMAND",
                        host_id = %request.host_id,
                        command_id = %id,
                        "Chrome debugger response had no pending daemon command"
                    );
                    return Ok(());
                };
                let readback_summary =
                    chrome_response_readback_summary(&pending.kind, response.result.as_ref());
                tracing::info!(
                    code = "CHROME_DEBUGGER_RESPONSE_ACCEPTED",
                    host_id = %request.host_id,
                    command_id = %id,
                    command_kind = %pending.kind,
                    response_ok = response.ok,
                    readback = %readback_summary.as_deref().unwrap_or(""),
                    "Chrome debugger response accepted"
                );
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
                } else if event == "tabNavigation" {
                    let decoded = serde_json::from_value::<ExtensionTabNavigationEvent>(
                        request.message.clone(),
                    )
                    .map_err(|error| format!("decode Chrome tabNavigation event: {error}"))?;
                    browser_navigation_event = Some(ChromeDebuggerBrowserNavigationEvent {
                        source: decoded.source,
                        event: "tabNavigation".to_owned(),
                        url: decoded.url,
                        title: decoded.title,
                        tab_id: Some(decoded.tab_id),
                        chrome_window_id: decoded.chrome_window_id,
                        cdp_target_id: (!decoded.target_id.is_empty()).then_some(decoded.target_id),
                        endpoint: host
                            .extension_id
                            .as_deref()
                            .map(|extension_id| format!("chrome-extension://{extension_id}")),
                        transport: host.transport.clone(),
                        ready_state: (!decoded.status.is_empty()).then_some(decoded.status),
                        observed_at_unix_ms: decoded.observed_at_unix_ms,
                        active: Some(decoded.active),
                        highlighted: Some(decoded.highlighted),
                        pinned: Some(decoded.pinned),
                    });
                    tracing::info!(
                        code = "CHROME_DEBUGGER_BROWSER_NAVIGATION_EVENT",
                        host_id = %request.host_id,
                        "Chrome debugger tab navigation event accepted"
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
        drop(inner);
        if let Some(event) = browser_navigation_event {
            emit_browser_navigation_event(event);
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
                    tracing::info!(
                        code = "CHROME_DEBUGGER_COMMAND_DELIVERED",
                        host_id = %host_id,
                        command_id = %queued.command.id,
                        command_kind = %queued.command.kind,
                        "Chrome debugger command delivered to bridge host"
                    );
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
            let transport_label = inner
                .hosts
                .get(&host_id)
                .and_then(|host| host.transport.as_deref())
                .unwrap_or("native_messaging")
                .to_owned();
            inner.pending.insert(
                id.clone(),
                PendingResponse {
                    host_id: host_id.clone(),
                    kind: kind.to_owned(),
                    sender,
                },
            );
            inner.commands.push_back(QueuedCommand {
                host_id: host_id.clone(),
                command,
            });
            tracing::info!(
                code = "CHROME_DEBUGGER_COMMAND_QUEUED",
                host_id = %host_id,
                command_id = %id,
                command_kind = %kind,
                transport = %transport_label,
                queue_depth = inner.commands.len(),
                "Chrome debugger command queued for bridge host"
            );
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
        if let Ok(mut inner) = self.inner.lock()
            && let Some(pending) = inner.pending.remove(id)
        {
            tracing::warn!(
                code = "CHROME_DEBUGGER_PENDING_DROPPED",
                command_id = %id,
                command_kind = %pending.kind,
                "Chrome debugger pending command removed"
            );
        }
    }

    fn direct_http_bridge_token_matches(&self, token: &str) -> bool {
        let token = token.trim();
        if token.is_empty() {
            return false;
        }
        let candidate = digest_bridge_token(token);
        self.inner.lock().is_ok_and(|inner| {
            inner.hosts.values().any(|host| {
                host.transport.as_deref() == Some("direct_http")
                    && bool::from(
                        host.bridge_token_digest
                            .as_slice()
                            .ct_eq(candidate.as_slice()),
                    )
            })
        })
    }

    fn direct_http_bridge_token_matches_host(&self, host_id: &str, token: &str) -> bool {
        let token = token.trim();
        if token.is_empty() {
            return false;
        }
        let candidate = digest_bridge_token(token);
        self.inner.lock().is_ok_and(|inner| {
            inner.hosts.get(host_id).is_some_and(|host| {
                host.transport.as_deref() == Some("direct_http")
                    && bool::from(
                        host.bridge_token_digest
                            .as_slice()
                            .ct_eq(candidate.as_slice()),
                    )
            })
        })
    }

    fn touch_host(&self, host_id: &str) -> bool {
        let now = now_unix_ms();
        self.inner.lock().is_ok_and(|mut inner| {
            inner.hosts.get_mut(host_id).is_some_and(|host| {
                if host.transport.as_deref() != Some("direct_http") {
                    return false;
                }
                host.last_seen_unix_ms = now;
                true
            })
        })
    }

    fn disconnect_direct_http_host(&self, host_id: &str, detail: &str) {
        let Ok(mut inner) = self.inner.lock() else {
            tracing::error!(
                code = "CHROME_DEBUGGER_DIRECT_HTTP_WS_DISCONNECT_LOCK_POISONED",
                host_id = %host_id,
                detail = %detail,
                "Chrome debugger direct HTTP bridge disconnect could not acquire lock"
            );
            return;
        };
        if inner
            .hosts
            .get(host_id)
            .and_then(|host| host.transport.as_deref())
            != Some("direct_http")
        {
            return;
        }
        if let Some(host) = inner.hosts.get_mut(host_id) {
            host.last_disconnect_detail = Some(detail.to_owned());
        }
        if inner.active_host_id.as_deref() == Some(host_id) {
            inner.active_host_id = None;
        }
        let queued_before = inner.commands.len();
        inner.commands.retain(|queued| queued.host_id != host_id);
        let queued_removed = queued_before.saturating_sub(inner.commands.len());
        let pending_ids = inner
            .pending
            .iter()
            .filter_map(|(id, pending)| {
                if pending.host_id == host_id {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for id in &pending_ids {
            if let Some(pending) = inner.pending.remove(id) {
                let _ = pending.sender.send(ChromeResponse {
                    id: id.clone(),
                    ok: false,
                    result: None,
                    error: Some(ChromeResponseError {
                        code: Some(error_codes::A11Y_CDP_EXTENSION_UNAVAILABLE.to_owned()),
                        detail: Some(format!(
                            "Chrome debugger direct HTTP bridge host {host_id} disconnected before command response: {detail}"
                        )),
                    }),
                });
            }
        }
        inner.hosts.remove(host_id);
        tracing::warn!(
            code = "CHROME_DEBUGGER_DIRECT_HTTP_WS_DISCONNECTED",
            host_id = %host_id,
            detail = %detail,
            queued_removed,
            pending_failed = pending_ids.len(),
            "Chrome debugger direct HTTP WebSocket disconnected"
        );
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
    _payload: Value,
) -> Result<Value, ChromeDebuggerBridgeError> {
    let error = ChromeDebuggerBridgeError::normal_bridge_attach_disabled(hwnd, kind);
    tracing::warn!(
        code = error.code(),
        hwnd,
        command_kind = kind,
        detail = %error.detail(),
        "normal Chrome bridge refused attach-capable debugger command before queueing it to Chrome"
    );
    Err(error)
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
    agent_session_id: Option<&str>,
) -> Result<ChromeDebuggerOpenTabResult, ChromeDebuggerBridgeError> {
    ensure_normal_bridge_popup_safe(hwnd, "openTab")?;
    let result = bridge()
        .send_command(
            "openTab",
            json!({
                "hwnd": hwnd,
                "url": url,
                "agentSessionId": agent_session_id,
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
    ensure_normal_bridge_popup_safe(hwnd, "closeTab")?;
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
    ensure_normal_bridge_popup_safe(hwnd, "targetInfo")?;
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
    agent_session_id: Option<&str>,
) -> Result<ChromeDebuggerNavigateResult, ChromeDebuggerBridgeError> {
    ensure_normal_bridge_popup_safe(hwnd, "navigateTab")?;
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
                "agentSessionId": agent_session_id,
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

pub(crate) fn is_direct_http_extension_bridge_request(headers: &HeaderMap, uri: &Uri) -> bool {
    let path = uri.path();
    if !path.starts_with("/chrome-debugger/native/") {
        return false;
    }
    if headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .is_some_and(|origin| origin == EXTENSION_ORIGIN)
    {
        return true;
    }
    if !matches!(
        path,
        "/chrome-debugger/native/next"
            | "/chrome-debugger/native/message"
            | "/chrome-debugger/native/ws"
    ) {
        return false;
    }
    if path == "/chrome-debugger/native/ws"
        && uri
            .query()
            .and_then(bridge_token_from_query)
            .is_some_and(|token| bridge().direct_http_bridge_token_matches(token))
    {
        return true;
    }
    headers
        .get(BRIDGE_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|token| bridge().direct_http_bridge_token_matches(token))
}

pub(crate) async fn http_register(Json(request): Json<NativeRegisterRequest>) -> Response {
    if request.transport.as_deref() == Some("direct_http")
        && let Err(error) = ensure_normal_bridge_registration_popup_safe()
    {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "code": error.code(),
                "detail": error.detail(),
            })),
        )
            .into_response();
    }
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

pub(crate) async fn http_ws(Query(query): Query<NativeWsQuery>, ws: WebSocketUpgrade) -> Response {
    if !bridge().direct_http_bridge_token_matches_host(&query.host_id, &query.bridge_token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "ok": false,
                "code": error_codes::A11Y_CDP_EXTENSION_UNAVAILABLE,
                "detail": "direct Chrome debugger bridge WebSocket token did not match registered host",
            })),
        )
            .into_response();
    }
    let host_id = query.host_id;
    ws.on_upgrade(move |socket| direct_http_ws_loop(socket, host_id))
}

async fn direct_http_ws_loop(socket: WebSocket, host_id: String) {
    tracing::info!(
        code = "CHROME_DEBUGGER_DIRECT_HTTP_WS_CONNECTED",
        host_id = %host_id,
        "Chrome debugger direct HTTP WebSocket connected"
    );
    let (mut sender, mut receiver) = socket.split();
    let mut disconnect_detail = "client closed direct HTTP WebSocket".to_owned();
    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(_))) | Some(Ok(Message::Binary(_))) | Some(Ok(Message::Pong(_))) => {
                        if !bridge().touch_host(&host_id) {
                            disconnect_detail = "registered direct HTTP host disappeared while processing WebSocket keepalive".to_owned();
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if !bridge().touch_host(&host_id) {
                            disconnect_detail = "registered direct HTTP host disappeared while processing WebSocket ping".to_owned();
                            break;
                        }
                        if let Err(error) = sender.send(Message::Pong(payload)).await {
                            disconnect_detail = format!("failed to send direct HTTP WebSocket pong: {error}");
                            break;
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        disconnect_detail = format!("client closed direct HTTP WebSocket frame={frame:?}");
                        break;
                    }
                    Some(Err(error)) => {
                        disconnect_detail = format!("direct HTTP WebSocket receive failed: {error}");
                        break;
                    }
                    None => {
                        disconnect_detail = "direct HTTP WebSocket receive stream ended".to_owned();
                        break;
                    }
                }
            }
            command_result = bridge().next_command(&host_id, DIRECT_WS_COMMAND_WAIT) => {
                let payload = match command_result {
                    Ok(command) => json!({
                        "ok": true,
                        "command": command,
                    }),
                    Err(detail) => {
                        disconnect_detail = format!("direct HTTP WebSocket command wait failed: {detail}");
                        json!({
                            "ok": false,
                            "code": error_codes::A11Y_CDP_EXTENSION_UNAVAILABLE,
                            "detail": disconnect_detail,
                        })
                    }
                };
                if let Err(error) = sender.send(Message::Text(payload.to_string().into())).await {
                    disconnect_detail = format!("failed to send direct HTTP WebSocket payload: {error}");
                    break;
                }
                if payload.get("ok").and_then(Value::as_bool) == Some(false) {
                    break;
                }
            }
        }
    }
    bridge().disconnect_direct_http_host(&host_id, &disconnect_detail);
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
        transport: Some("native_messaging".to_owned()),
    };
    let response = client
        .post(format!("{base_url}/chrome-debugger/native/register"))
        .bearer_auth(token)
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
    let registration = NativeHostRegistrationContext {
        client: &client,
        base_url: &base_url,
        token: &token,
        invocation: &invocation,
        pid,
    };
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
                    &registration,
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
    let registration = NativeHostRegistrationContext {
        client: &client,
        base_url: &base_url,
        token: &token,
        invocation: &invocation,
        pid,
    };
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
                    &registration,
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

struct NativeHostRegistrationContext<'a> {
    client: &'a reqwest::Client,
    base_url: &'a str,
    token: &'a str,
    invocation: &'a NativeHostInvocation,
    pid: u32,
}

async fn reregister_native_host_until_available(
    registration: &NativeHostRegistrationContext<'_>,
    host_id: &Arc<RwLock<String>>,
    observed_host_id: &str,
    reason: &'static str,
) -> anyhow::Result<()> {
    if host_id.read().await.as_str() != observed_host_id {
        return Ok(());
    }
    loop {
        match register_native_host(
            registration.client,
            registration.base_url,
            registration.token,
            registration.invocation,
            registration.pid,
            reason,
        )
        .await
        {
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

fn digest_bridge_token(token: &str) -> [u8; 32] {
    let digest = Sha256::digest(token.as_bytes());
    let mut output = [0_u8; 32];
    output.copy_from_slice(&digest);
    output
}

fn chrome_response_readback_summary(kind: &str, result: Option<&Value>) -> Option<String> {
    let result = result?;
    let summary = match kind {
        "openTab" => json!({
            "target_id": result.get("target_id"),
            "tab_id": result.get("tab_id"),
            "url": result.get("url"),
            "target_attached": result.get("target_attached"),
            "target_count_before": result.get("target_count_before"),
            "target_count_after": result.get("target_count_after"),
            "extension_id": result.get("extension_id"),
        }),
        "closeTab" => json!({
            "target_id": result.get("target_id"),
            "tab_id": result.get("tab_id"),
            "target_count_before": result.get("target_count_before"),
            "target_count_after": result.get("target_count_after"),
            "extension_id": result.get("extension_id"),
        }),
        "navigateTab" => json!({
            "target_id": result.get("target_id"),
            "tab_id": result.get("tab_id"),
            "action": result.get("action"),
            "requested_url": result.get("requested_url"),
            "before_url": result.get("before_url"),
            "after_url": result.get("after_url"),
            "ready_state": result.get("ready_state"),
            "readback_backend": result.get("readback_backend"),
        }),
        "targetInfo" => json!({
            "target_id": result.get("target_id"),
            "tab_id": result.get("tab_id"),
            "target_type": result.get("target_type"),
            "url": result.get("url"),
            "title": result.get("title"),
            "target_candidate_count": result.get("target_candidate_count"),
            "target_selection_reason": result.get("target_selection_reason"),
            "extension_id": result.get("extension_id"),
        }),
        _ => return None,
    };
    serde_json::to_string(&summary).ok()
}

fn bridge_token_from_query(query: &str) -> Option<&str> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        if key == "bridge_token" {
            Some(value)
        } else {
            None
        }
    })
}

fn replace_active_direct_http_host(inner: &mut BridgeInner, new_host_id: &str) {
    let Some(old_host_id) = inner.active_host_id.clone() else {
        return;
    };
    if old_host_id == new_host_id {
        return;
    }
    let Some(old_host) = inner.hosts.get(&old_host_id) else {
        return;
    };
    if old_host.transport.as_deref() != Some("direct_http") {
        return;
    }
    let queued_before = inner.commands.len();
    inner
        .commands
        .retain(|queued| queued.host_id != old_host_id);
    let queued_removed = queued_before.saturating_sub(inner.commands.len());
    let pending_ids = inner
        .pending
        .iter()
        .filter_map(|(id, pending)| {
            if pending.host_id == old_host_id {
                Some(id.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for id in &pending_ids {
        if let Some(pending) = inner.pending.remove(id) {
            let _ = pending.sender.send(ChromeResponse {
                id: id.clone(),
                ok: false,
                result: None,
                error: Some(ChromeResponseError {
                    code: Some(error_codes::A11Y_CDP_EXTENSION_UNAVAILABLE.to_owned()),
                    detail: Some(format!(
                        "Chrome debugger direct HTTP bridge host {old_host_id} was replaced by {new_host_id} before command response"
                    )),
                }),
            });
        }
    }
    tracing::warn!(
        code = "CHROME_DEBUGGER_DIRECT_HTTP_HOST_REPLACED",
        old_host_id = %old_host_id,
        new_host_id = %new_host_id,
        queued_removed,
        pending_failed = pending_ids.len(),
        "Chrome debugger direct HTTP bridge host replaced"
    );
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, Uri, header};

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
    fn direct_http_bridge_token_authorizes_next_without_origin_only_after_register() {
        let registered = bridge()
            .register(NativeRegisterRequest {
                origin: "chrome-extension://leoocgnkjnplbfdbklajepahofecgfbk/".to_owned(),
                pid: 0,
                parent_window: None,
                bridge_protocol_version: BRIDGE_PROTOCOL_VERSION,
                transport: Some("direct_http".to_owned()),
            })
            .expect("direct bridge register should issue a host token");
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:7700"));
        headers.insert(
            BRIDGE_TOKEN_HEADER,
            HeaderValue::from_str(&registered.bridge_token)
                .expect("bridge token should be a valid header value"),
        );

        assert!(is_direct_http_extension_bridge_request(
            &headers,
            &Uri::from_static("/chrome-debugger/native/next?host_id=anything"),
        ));
        let ws_uri = format!(
            "/chrome-debugger/native/ws?host_id={}&bridge_token={}",
            registered.host_id, registered.bridge_token
        )
        .parse::<Uri>()
        .expect("websocket uri with token should parse");
        assert!(is_direct_http_extension_bridge_request(
            &HeaderMap::new(),
            &ws_uri
        ));
        assert!(!is_direct_http_extension_bridge_request(
            &headers,
            &Uri::from_static("/chrome-debugger/native/register"),
        ));
        assert!(!is_direct_http_extension_bridge_request(
            &headers,
            &Uri::from_static("/mcp"),
        ));
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
    fn normal_bridge_attach_disabled_is_local_refusal() {
        let error = ChromeDebuggerBridgeError::normal_bridge_attach_disabled(1234, "snapshot");

        assert_eq!(
            error.code(),
            error_codes::A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED
        );
        assert_eq!(error.cdp_status(), CdpStatus::AttachFailed);
        assert!(
            error
                .detail()
                .contains("before queueing any Chrome command")
        );
        assert!(
            error
                .detail()
                .contains("normal end-user bridge is tabs-only")
        );
        assert!(error.detail().contains("raw CDP"));
    }

    #[test]
    fn normal_bridge_external_popup_risk_is_local_refusal() {
        let risks = vec![
            "profile=Profile 5 pref=Secure Preferences extension_id=fcoeoabgfenejglbffodgkkbkcdhcgfn name=\"Claude\" active_api=debugger,nativeMessaging".to_owned(),
            "native_messaging_process pid=26616 name=cmd.exe extension_id=fcoeoabgfenejglbffodgkkbkcdhcgfn".to_owned(),
        ];
        let error = ChromeDebuggerBridgeError::normal_bridge_external_popup_risk(
            10356912, "openTab", &risks,
        );

        assert_eq!(
            error.code(),
            error_codes::A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED
        );
        assert_eq!(error.cdp_status(), CdpStatus::AttachFailed);
        assert!(error.detail().contains("before queueing it to Chrome"));
        assert!(error.detail().contains("Source of Truth is not popup-free"));
        assert!(error.detail().contains("ExtensionSettings wildcard"));
        assert!(error.detail().contains("fcoeoabgfenejglbffodgkkbkcdhcgfn"));
        assert!(error.detail().contains("raw CDP"));
    }

    #[test]
    fn normal_bridge_registration_external_popup_risk_is_local_refusal() {
        let risks = vec![
            "profile=Profile 5 pref=Secure Preferences extension_id=fcoeoabgfenejglbffodgkkbkcdhcgfn name=\"Claude\" active_api=debugger,nativeMessaging".to_owned(),
            "native_messaging_process pid=26616 name=cmd.exe extension_id=fcoeoabgfenejglbffodgkkbkcdhcgfn".to_owned(),
        ];
        let error =
            ChromeDebuggerBridgeError::normal_bridge_registration_external_popup_risk(&risks);

        assert_eq!(
            error.code(),
            error_codes::A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED
        );
        assert_eq!(error.cdp_status(), CdpStatus::AttachFailed);
        assert!(
            error
                .detail()
                .contains("refused direct registration before accepting")
        );
        assert!(error.detail().contains("Source of Truth is not popup-free"));
        assert!(error.detail().contains("fcoeoabgfenejglbffodgkkbkcdhcgfn"));
        assert!(error.detail().contains("raw CDP"));
    }

    #[test]
    fn chrome_extension_runtime_state_treats_disabled_permission_rows_as_not_enabled() {
        let setting = json!({
            "active_bit": false,
            "disable_reasons": [65536],
            "active_permissions": {
                "api": ["downloads", "nativeMessaging"]
            }
        });

        let runtime_state = chrome_extension_runtime_state(&setting);

        assert_eq!(runtime_state.active_bit, Some(false));
        assert_eq!(runtime_state.disable_reasons, vec![65536]);
        assert!(!runtime_state.runtime_enabled);
    }

    #[test]
    fn chrome_extension_runtime_state_fails_closed_when_disabled_state_is_absent() {
        let setting = json!({
            "active_permissions": {
                "api": ["nativeMessaging"]
            }
        });

        let runtime_state = chrome_extension_runtime_state(&setting);

        assert_eq!(runtime_state.active_bit, None);
        assert!(runtime_state.disable_reasons.is_empty());
        assert!(runtime_state.runtime_enabled);
    }

    #[test]
    fn external_popup_risk_formatter_caps_noisy_readback() {
        let risks = (0..10)
            .map(|index| format!("risk-{index}"))
            .collect::<Vec<_>>();

        let formatted = format_external_chrome_popup_risks(&risks);

        assert!(formatted.contains("risk-0"));
        assert!(formatted.contains("risk-7"));
        assert!(!formatted.contains("risk-8 |"));
        assert!(formatted.ends_with("+2 more"));
    }
}
