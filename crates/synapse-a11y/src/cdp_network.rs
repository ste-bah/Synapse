//! Persistent per-target CDP Network capture (#1080).
//!
//! Browser automation needs the same request/response visibility Playwright
//! exposes through `page.on("request")`, `page.on("response")`, and request
//! completion/failure events. CDP does not replay old Network events after
//! `Network.enable`, so this module mirrors `cdp_console`: one long-lived CDP
//! connection per armed target, a live event pump, and a bounded ring buffer
//! that can be read by cursor without consuming entries.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use chromiumoxide::cdp::browser_protocol::fetch::{
    ContinueRequestParams as FetchContinueRequestParams, DisableParams as FetchDisableParams,
    EnableParams as FetchEnableParams, EventRequestPaused as FetchEventRequestPaused,
    RequestId as FetchRequestId, RequestPattern as FetchRequestPattern,
    RequestStage as FetchRequestStage,
};
use chromiumoxide::cdp::browser_protocol::network::{
    EnableParams as NetworkEnableParams, EventLoadingFailed, EventLoadingFinished,
    EventRequestWillBeSent, EventResponseReceived, GetRequestPostDataParams, GetResponseBodyParams,
    Headers, ResourceType as NetworkResourceType, Response,
};
use chromiumoxide::{Browser, Page};
use futures_util::StreamExt as _;
use serde::Serialize;
use serde_json::Value;
use tokio::task::JoinHandle;

use crate::{A11yError, A11yResult};

/// Default network buffer capacity (request records) per captured target.
pub const DEFAULT_NETWORK_BUFFER_CAPACITY: usize = 1000;
/// Hard ceiling on requested network buffer capacity.
pub const MAX_NETWORK_BUFFER_CAPACITY: usize = 10_000;

/// A response snapshot captured either as the current response or as a redirect
/// response attached to the next `Network.requestWillBeSent` event.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CdpNetworkResponseSnapshot {
    pub url: String,
    pub status: i64,
    pub status_text: String,
    pub headers: Value,
    pub request_headers: Option<Value>,
    pub mime_type: String,
    pub protocol: Option<String>,
    pub remote_ip_address: Option<String>,
    pub remote_port: Option<i64>,
    pub encoded_data_length: f64,
    pub timing: Option<Value>,
    pub response_time_ms: Option<f64>,
    pub from_disk_cache: Option<bool>,
    pub from_service_worker: Option<bool>,
    pub from_prefetch_cache: Option<bool>,
    pub from_early_hints: Option<bool>,
    pub timestamp_s: Option<f64>,
    pub resource_type: Option<String>,
}

/// One request record keyed by CDP `requestId`. `seq` is the last event cursor
/// for this request; `first_seq` is the cursor when the record was created.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CdpNetworkEntry {
    pub seq: u64,
    pub first_seq: u64,
    pub request_id: String,
    pub loader_id: Option<String>,
    pub frame_id: Option<String>,
    pub document_url: Option<String>,
    pub url: Option<String>,
    pub method: Option<String>,
    pub resource_type: Option<String>,
    pub request_headers: Option<Value>,
    pub request_has_post_data: Option<bool>,
    pub request_timestamp_s: Option<f64>,
    pub request_wall_time_ms: Option<f64>,
    pub initiator: Option<Value>,
    pub redirects: Vec<CdpNetworkResponseSnapshot>,
    pub response: Option<CdpNetworkResponseSnapshot>,
    pub response_timestamp_s: Option<f64>,
    pub response_received: bool,
    pub loading_finished: bool,
    pub loading_failed: bool,
    pub finished_timestamp_s: Option<f64>,
    pub failed_timestamp_s: Option<f64>,
    pub encoded_data_length: Option<f64>,
    pub failure_error_text: Option<String>,
    pub failure_canceled: Option<bool>,
    pub failure_blocked_reason: Option<String>,
    pub failure_cors_error_status: Option<Value>,
}

impl CdpNetworkEntry {
    fn new(seq: u64, request_id: String) -> Self {
        Self {
            seq,
            first_seq: seq,
            request_id,
            loader_id: None,
            frame_id: None,
            document_url: None,
            url: None,
            method: None,
            resource_type: None,
            request_headers: None,
            request_has_post_data: None,
            request_timestamp_s: None,
            request_wall_time_ms: None,
            initiator: None,
            redirects: Vec::new(),
            response: None,
            response_timestamp_s: None,
            response_received: false,
            loading_finished: false,
            loading_failed: false,
            finished_timestamp_s: None,
            failed_timestamp_s: None,
            encoded_data_length: None,
            failure_error_text: None,
            failure_canceled: None,
            failure_blocked_reason: None,
            failure_cors_error_status: None,
        }
    }
}

/// Result of arming network capture for a target.
#[derive(Clone, Debug, Serialize)]
pub struct CdpNetworkCaptureStatus {
    pub newly_armed: bool,
    pub armed_at_unix_ms: f64,
    pub endpoint: String,
    pub cdp_target_id: String,
    pub capacity: usize,
}

/// Optional filters for [`network_capture_read`].
#[derive(Clone, Debug, Default)]
pub struct CdpNetworkReadFilter<'a> {
    /// Only entries whose latest event has `seq >= since_seq`.
    pub since_seq: Option<u64>,
    /// Exact CDP request id match.
    pub request_id: Option<&'a str>,
    /// Case-insensitive substring match against the current request URL.
    pub url_contains: Option<&'a str>,
    /// Exact resource type match, e.g. `Document`, `Script`, `XHR`.
    pub resource_type: Option<&'a str>,
    /// Exact HTTP response status match.
    pub status: Option<i64>,
    /// If set, returns only failed or non-failed records.
    pub failed: Option<bool>,
    /// Maximum entries to return (oldest-first by latest event cursor).
    pub max: usize,
}

/// A cursor-delimited view of the network capture buffer.
#[derive(Clone, Debug, Serialize)]
pub struct CdpNetworkReadResult {
    pub entries: Vec<CdpNetworkEntry>,
    pub next_cursor: u64,
    pub returned: usize,
    pub total_buffered: usize,
    pub dropped: u64,
    pub armed_at_unix_ms: f64,
    pub capacity: usize,
}

/// Response body returned by `Network.getResponseBody` for a captured request.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CdpNetworkResponseBody {
    pub request_id: String,
    pub body: String,
    pub base64_encoded: bool,
}

/// Request body returned by `Network.getRequestPostData` for a captured request.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CdpNetworkRequestPostData {
    pub request_id: String,
    pub post_data: String,
}

/// Fetch interception stage for [`CdpFetchInterceptionPattern`].
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
pub enum CdpFetchInterceptionStage {
    #[default]
    Request,
    Response,
}

/// CDP Fetch interception pattern. An empty pattern list passed to
/// [`fetch_interception_ensure`] means "intercept all requests".
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct CdpFetchInterceptionPattern {
    pub url_pattern: Option<String>,
    pub resource_type: Option<String>,
    pub request_stage: CdpFetchInterceptionStage,
}

/// Status/readback for the continue-by-default Fetch interception scaffold.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CdpFetchInterceptionStatus {
    pub newly_armed: bool,
    pub endpoint: String,
    pub cdp_target_id: String,
    pub armed_at_unix_ms: u64,
    pub pattern_count: usize,
    pub paused_count: u64,
    pub continued_count: u64,
    pub continue_error_count: u64,
    pub last_request_id: Option<String>,
    pub last_url: Option<String>,
    pub last_error: Option<String>,
}

struct RingBuffer {
    entries: VecDeque<CdpNetworkEntry>,
    capacity: usize,
    next_seq: u64,
    dropped: u64,
}

impl RingBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity.min(256)),
            capacity: capacity.max(1),
            next_seq: 0,
            dropped: 0,
        }
    }

    fn cursor(&self) -> u64 {
        self.next_seq
    }

    fn reserve_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    fn entry_for_event(&mut self, request_id: &str) -> usize {
        let seq = self.reserve_seq();
        if let Some(index) = self.entries.iter().position(|e| e.request_id == request_id) {
            if let Some(entry) = self.entries.get_mut(index) {
                entry.seq = seq;
            }
            return index;
        }

        while self.entries.len() >= self.capacity {
            self.entries.pop_front();
            self.dropped += 1;
        }
        self.entries
            .push_back(CdpNetworkEntry::new(seq, request_id.to_owned()));
        self.entries.len() - 1
    }

    fn apply_request_will_be_sent(&mut self, event: &EventRequestWillBeSent) {
        let index = self.entry_for_event(event.request_id.inner());
        let entry = self
            .entries
            .get_mut(index)
            .expect("entry_for_event inserted or found entry");

        if let Some(redirect) = &event.redirect_response {
            let resource_type = event.r#type.as_ref().map(enum_str);
            entry
                .redirects
                .push(response_snapshot(redirect, None, resource_type.as_deref()));
            entry.response = None;
            entry.response_timestamp_s = None;
            entry.response_received = false;
            entry.loading_finished = false;
            entry.loading_failed = false;
            entry.finished_timestamp_s = None;
            entry.failed_timestamp_s = None;
            entry.encoded_data_length = None;
            entry.failure_error_text = None;
            entry.failure_canceled = None;
            entry.failure_blocked_reason = None;
            entry.failure_cors_error_status = None;
        }

        entry.loader_id = Some(event.loader_id.inner().clone());
        entry.frame_id = event.frame_id.as_ref().map(|id| id.inner().clone());
        entry.document_url = Some(event.document_url.clone());
        entry.url = Some(event.request.url.clone());
        entry.method = Some(event.request.method.clone());
        entry.resource_type = event.r#type.as_ref().map(enum_str);
        entry.request_headers = Some(headers_value(&event.request.headers));
        entry.request_has_post_data = event.request.has_post_data;
        entry.request_timestamp_s = Some(timestamp_s(&event.timestamp));
        entry.request_wall_time_ms = Some(timestamp_s(&event.wall_time) * 1000.0);
        entry.initiator = serde_json::to_value(&event.initiator).ok();
    }

    fn apply_response_received(&mut self, event: &EventResponseReceived) {
        let index = self.entry_for_event(event.request_id.inner());
        let entry = self
            .entries
            .get_mut(index)
            .expect("entry_for_event inserted or found entry");
        entry.loader_id = Some(event.loader_id.inner().clone());
        entry.frame_id = event.frame_id.as_ref().map(|id| id.inner().clone());
        entry.resource_type = Some(enum_str(&event.r#type));
        entry.response = Some(response_snapshot(
            &event.response,
            Some(timestamp_s(&event.timestamp)),
            Some(&enum_str(&event.r#type)),
        ));
        entry.response_timestamp_s = Some(timestamp_s(&event.timestamp));
        entry.response_received = true;
        entry.loading_failed = false;
        entry.failure_error_text = None;
        entry.failure_canceled = None;
        entry.failure_blocked_reason = None;
        entry.failure_cors_error_status = None;
    }

    fn apply_loading_finished(&mut self, event: &EventLoadingFinished) {
        let index = self.entry_for_event(event.request_id.inner());
        let entry = self
            .entries
            .get_mut(index)
            .expect("entry_for_event inserted or found entry");
        entry.loading_finished = true;
        entry.loading_failed = false;
        entry.finished_timestamp_s = Some(timestamp_s(&event.timestamp));
        entry.encoded_data_length = Some(event.encoded_data_length);
    }

    fn apply_loading_failed(&mut self, event: &EventLoadingFailed) {
        let index = self.entry_for_event(event.request_id.inner());
        let entry = self
            .entries
            .get_mut(index)
            .expect("entry_for_event inserted or found entry");
        entry.resource_type = Some(enum_str(&event.r#type));
        entry.loading_finished = false;
        entry.loading_failed = true;
        entry.failed_timestamp_s = Some(timestamp_s(&event.timestamp));
        entry.failure_error_text = Some(event.error_text.clone());
        entry.failure_canceled = event.canceled;
        entry.failure_blocked_reason = event.blocked_reason.as_ref().map(enum_str);
        entry.failure_cors_error_status = event
            .cors_error_status
            .as_ref()
            .and_then(|status| serde_json::to_value(status).ok());
    }
}

struct NetworkCaptureSlot {
    buffer: Arc<Mutex<RingBuffer>>,
    endpoint: String,
    armed_at_unix_ms: f64,
    capacity: usize,
    page: Page,
    _browser: Browser,
    handler_task: JoinHandle<()>,
    listener_task: JoinHandle<()>,
}

impl Drop for NetworkCaptureSlot {
    fn drop(&mut self) {
        self.handler_task.abort();
        self.listener_task.abort();
    }
}

#[derive(Default)]
struct NetworkCaptureRegistry {
    slots: Mutex<HashMap<String, Arc<NetworkCaptureSlot>>>,
}

#[derive(Default)]
struct FetchInterceptionCounters {
    paused_count: u64,
    continued_count: u64,
    continue_error_count: u64,
    last_request_id: Option<String>,
    last_url: Option<String>,
    last_error: Option<String>,
}

struct FetchInterceptionSlot {
    endpoint: String,
    target_id: String,
    armed_at_unix_ms: u64,
    patterns: Vec<CdpFetchInterceptionPattern>,
    counters: Arc<Mutex<FetchInterceptionCounters>>,
    page: Page,
    _browser: Browser,
    handler_task: JoinHandle<()>,
    listener_task: JoinHandle<()>,
}

impl Drop for FetchInterceptionSlot {
    fn drop(&mut self) {
        self.handler_task.abort();
        self.listener_task.abort();
    }
}

#[derive(Default)]
struct FetchInterceptionRegistry {
    slots: Mutex<HashMap<String, Arc<FetchInterceptionSlot>>>,
}

fn registry() -> &'static NetworkCaptureRegistry {
    static REGISTRY: OnceLock<NetworkCaptureRegistry> = OnceLock::new();
    REGISTRY.get_or_init(NetworkCaptureRegistry::default)
}

fn fetch_registry() -> &'static FetchInterceptionRegistry {
    static REGISTRY: OnceLock<FetchInterceptionRegistry> = OnceLock::new();
    REGISTRY.get_or_init(FetchInterceptionRegistry::default)
}

/// Arms (or re-arms) persistent CDP Network capture for `target_id`.
///
/// Idempotent: a live capture is reused. Capture starts at the arm time because
/// Chrome only sends Network events live after `Network.enable`.
pub async fn network_capture_ensure(
    endpoint: &str,
    target_id: &str,
    capacity: usize,
) -> A11yResult<CdpNetworkCaptureStatus> {
    let target_id = target_id.trim();
    if target_id.is_empty() {
        return Err(A11yError::CdpAttachFailed {
            detail: "network capture target id must not be empty".to_owned(),
        });
    }

    if let Some(slot) = lookup_live(target_id) {
        return Ok(CdpNetworkCaptureStatus {
            newly_armed: false,
            armed_at_unix_ms: slot.armed_at_unix_ms,
            endpoint: slot.endpoint.clone(),
            cdp_target_id: target_id.to_owned(),
            capacity: slot.capacity,
        });
    }

    let capacity = capacity.clamp(1, MAX_NETWORK_BUFFER_CAPACITY);
    let (browser, mut handler) =
        Browser::connect(endpoint)
            .await
            .map_err(|err| A11yError::CdpAttachFailed {
                detail: format!("network capture connect {endpoint}: {err}"),
            })?;
    let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let armed = async {
        let page = crate::cdp_action::get_target_page_with_discovery(&browser, target_id).await?;
        page.execute(NetworkEnableParams::default())
            .await
            .map_err(|err| A11yError::CdpAxtreeFailed {
                detail: format!("Network.enable for network capture: {err}"),
            })?;
        let request_started = page
            .event_listener::<EventRequestWillBeSent>()
            .await
            .map_err(|err| A11yError::CdpAxtreeFailed {
                detail: format!("subscribe Network.requestWillBeSent: {err}"),
            })?;
        let response_received = page
            .event_listener::<EventResponseReceived>()
            .await
            .map_err(|err| A11yError::CdpAxtreeFailed {
                detail: format!("subscribe Network.responseReceived: {err}"),
            })?;
        let loading_finished = page
            .event_listener::<EventLoadingFinished>()
            .await
            .map_err(|err| A11yError::CdpAxtreeFailed {
                detail: format!("subscribe Network.loadingFinished: {err}"),
            })?;
        let loading_failed = page
            .event_listener::<EventLoadingFailed>()
            .await
            .map_err(|err| A11yError::CdpAxtreeFailed {
                detail: format!("subscribe Network.loadingFailed: {err}"),
            })?;
        Ok::<_, A11yError>((
            page,
            request_started,
            response_received,
            loading_finished,
            loading_failed,
        ))
    }
    .await;

    let (
        page,
        mut request_started,
        mut response_received,
        mut loading_finished,
        mut loading_failed,
    ) = match armed {
        Ok(streams) => streams,
        Err(err) => {
            handler_task.abort();
            return Err(err);
        }
    };

    let buffer = Arc::new(Mutex::new(RingBuffer::new(capacity)));
    let pump_buffer = Arc::clone(&buffer);
    let slot_page = page.clone();
    let listener_task = tokio::spawn(async move {
        let _page = page;
        loop {
            tokio::select! {
                Some(event) = request_started.next() => {
                    apply_request(&pump_buffer, event.as_ref());
                }
                Some(event) = response_received.next() => {
                    apply_response(&pump_buffer, event.as_ref());
                }
                Some(event) = loading_finished.next() => {
                    apply_finished(&pump_buffer, event.as_ref());
                }
                Some(event) = loading_failed.next() => {
                    apply_failed(&pump_buffer, event.as_ref());
                }
                else => break,
            }
        }
    });

    let armed_at_unix_ms = now_unix_ms();
    let slot = Arc::new(NetworkCaptureSlot {
        buffer,
        endpoint: endpoint.to_owned(),
        armed_at_unix_ms,
        capacity,
        page: slot_page,
        _browser: browser,
        handler_task,
        listener_task,
    });
    if let Ok(mut slots) = registry().slots.lock() {
        if let Some(existing) = slots.get(target_id)
            && !existing.listener_task.is_finished()
        {
            return Ok(CdpNetworkCaptureStatus {
                newly_armed: false,
                armed_at_unix_ms: existing.armed_at_unix_ms,
                endpoint: existing.endpoint.clone(),
                cdp_target_id: target_id.to_owned(),
                capacity: existing.capacity,
            });
        }
        slots.insert(target_id.to_owned(), slot);
    }

    Ok(CdpNetworkCaptureStatus {
        newly_armed: true,
        armed_at_unix_ms,
        endpoint: endpoint.to_owned(),
        cdp_target_id: target_id.to_owned(),
        capacity,
    })
}

/// Compatibility alias for callers that use the issue's CDP-prefixed naming.
pub async fn cdp_network_capture_start(
    endpoint: &str,
    target_id: &str,
    capacity: usize,
) -> A11yResult<CdpNetworkCaptureStatus> {
    network_capture_ensure(endpoint, target_id, capacity).await
}

/// Reads a filtered, cursor-delimited slice of a target's network buffer.
pub fn network_capture_read(
    target_id: &str,
    filter: &CdpNetworkReadFilter<'_>,
) -> Option<CdpNetworkReadResult> {
    let slot = {
        let slots = registry().slots.lock().ok()?;
        Arc::clone(slots.get(target_id.trim())?)
    };
    let buffer = slot.buffer.lock().ok()?;
    let total_buffered = buffer.entries.len();
    let next_cursor = buffer.cursor();
    let dropped = buffer.dropped;
    let max = if filter.max == 0 {
        usize::MAX
    } else {
        filter.max
    };
    let mut entries: Vec<CdpNetworkEntry> = buffer
        .entries
        .iter()
        .filter(|entry| filter.since_seq.is_none_or(|since| entry.seq >= since))
        .filter(|entry| filter.request_id.is_none_or(|id| entry.request_id == id))
        .filter(|entry| {
            filter.url_contains.is_none_or(|needle| {
                entry
                    .url
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&needle.to_lowercase())
            })
        })
        .filter(|entry| {
            filter.resource_type.is_none_or(|resource_type| {
                entry
                    .resource_type
                    .as_deref()
                    .is_some_and(|entry_type| entry_type.eq_ignore_ascii_case(resource_type))
            })
        })
        .filter(|entry| {
            filter
                .status
                .is_none_or(|status| entry.response.as_ref().is_some_and(|r| r.status == status))
        })
        .filter(|entry| {
            filter
                .failed
                .is_none_or(|failed| entry.loading_failed == failed)
        })
        .cloned()
        .collect();
    entries.sort_by_key(|entry| entry.seq);
    entries.truncate(max);

    Some(CdpNetworkReadResult {
        returned: entries.len(),
        entries,
        next_cursor,
        total_buffered,
        dropped,
        armed_at_unix_ms: slot.armed_at_unix_ms,
        capacity: slot.capacity,
    })
}

/// Reads the response body for a captured request from the persistent capture
/// CDP session. Chrome may evict bodies; that surfaces as a CDP error.
pub async fn network_response_body(
    target_id: &str,
    request_id: &str,
) -> A11yResult<CdpNetworkResponseBody> {
    let target_id = target_id.trim();
    let request_id = normalize_request_id(request_id)?;
    let slot = lookup_live(target_id).ok_or_else(|| A11yError::CdpAttachFailed {
        detail: format!("network capture for target {target_id} is not armed"),
    })?;
    let body = slot
        .page
        .execute(GetResponseBodyParams::new(request_id.to_owned()))
        .await
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("Network.getResponseBody request_id={request_id}: {err}"),
        })?;
    Ok(CdpNetworkResponseBody {
        request_id: request_id.to_owned(),
        body: body.body.clone(),
        base64_encoded: body.base64_encoded,
    })
}

/// Reads request post data for a captured request from the persistent capture
/// CDP session. Chrome returns an error when no post data exists.
pub async fn network_request_post_data(
    target_id: &str,
    request_id: &str,
) -> A11yResult<CdpNetworkRequestPostData> {
    let target_id = target_id.trim();
    let request_id = normalize_request_id(request_id)?;
    let slot = lookup_live(target_id).ok_or_else(|| A11yError::CdpAttachFailed {
        detail: format!("network capture for target {target_id} is not armed"),
    })?;
    let post_data = slot
        .page
        .execute(GetRequestPostDataParams::new(request_id.to_owned()))
        .await
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("Network.getRequestPostData request_id={request_id}: {err}"),
        })?;
    Ok(CdpNetworkRequestPostData {
        request_id: request_id.to_owned(),
        post_data: post_data.post_data.clone(),
    })
}

/// Enables the Fetch domain for a target and continues every paused request by
/// default. This is the route/interception substrate; rules are layered later.
pub async fn fetch_interception_ensure(
    endpoint: &str,
    target_id: &str,
    patterns: Vec<CdpFetchInterceptionPattern>,
) -> A11yResult<CdpFetchInterceptionStatus> {
    let target_id = target_id.trim();
    if target_id.is_empty() {
        return Err(A11yError::CdpAttachFailed {
            detail: "fetch interception target id must not be empty".to_owned(),
        });
    }

    if let Some(slot) = lookup_fetch_live(target_id) {
        if slot.patterns == patterns {
            return Ok(fetch_interception_status_from_slot(&slot, false));
        }
        let _ = fetch_interception_stop(target_id).await?;
    }

    let fetch_patterns = fetch_patterns_to_cdp(&patterns)?;
    let (browser, mut handler) =
        Browser::connect(endpoint)
            .await
            .map_err(|err| A11yError::CdpAttachFailed {
                detail: format!("fetch interception connect {endpoint}: {err}"),
            })?;
    let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let armed = async {
        let page = crate::cdp_action::get_target_page_with_discovery(&browser, target_id).await?;
        let mut enable = FetchEnableParams::builder().handle_auth_requests(false);
        if !fetch_patterns.is_empty() {
            enable = enable.patterns(fetch_patterns);
        }
        page.execute(enable.build())
            .await
            .map_err(|err| A11yError::CdpAxtreeFailed {
                detail: format!("Fetch.enable for interception scaffold: {err}"),
            })?;
        let request_paused = page
            .event_listener::<FetchEventRequestPaused>()
            .await
            .map_err(|err| A11yError::CdpAxtreeFailed {
                detail: format!("subscribe Fetch.requestPaused: {err}"),
            })?;
        Ok::<_, A11yError>((page, request_paused))
    }
    .await;

    let (page, mut request_paused) = match armed {
        Ok(armed) => armed,
        Err(err) => {
            handler_task.abort();
            return Err(err);
        }
    };

    let counters = Arc::new(Mutex::new(FetchInterceptionCounters::default()));
    let pump_counters = Arc::clone(&counters);
    let slot_page = page.clone();
    let listener_page = page.clone();
    let listener_task = tokio::spawn(async move {
        let _page = page;
        while let Some(event) = request_paused.next().await {
            let event = event.as_ref();
            let request_id = fetch_request_id_string(&event.request_id);
            if let Ok(mut counters) = pump_counters.lock() {
                counters.paused_count = counters.paused_count.saturating_add(1);
                counters.last_request_id = Some(request_id.clone());
                counters.last_url = Some(event.request.url.clone());
            }
            let continued = listener_page
                .execute(FetchContinueRequestParams::new(event.request_id.clone()))
                .await;
            if let Ok(mut counters) = pump_counters.lock() {
                match continued {
                    Ok(_) => {
                        counters.continued_count = counters.continued_count.saturating_add(1);
                    }
                    Err(error) => {
                        counters.continue_error_count =
                            counters.continue_error_count.saturating_add(1);
                        counters.last_error = Some(format!(
                            "Fetch.continueRequest request_id={request_id}: {error}"
                        ));
                    }
                }
            }
        }
    });

    let armed_at_unix_ms = now_unix_ms() as u64;
    let slot = Arc::new(FetchInterceptionSlot {
        endpoint: endpoint.to_owned(),
        target_id: target_id.to_owned(),
        armed_at_unix_ms,
        patterns,
        counters,
        page: slot_page,
        _browser: browser,
        handler_task,
        listener_task,
    });
    if let Ok(mut slots) = fetch_registry().slots.lock() {
        if let Some(existing) = slots.get(target_id)
            && !existing.listener_task.is_finished()
        {
            return Ok(fetch_interception_status_from_slot(existing, false));
        }
        slots.insert(target_id.to_owned(), Arc::clone(&slot));
    }
    Ok(fetch_interception_status_from_slot(&slot, true))
}

/// Returns the current Fetch interception scaffold status for a target.
#[must_use]
pub fn fetch_interception_status(target_id: &str) -> Option<CdpFetchInterceptionStatus> {
    lookup_fetch_live(target_id.trim())
        .map(|slot| fetch_interception_status_from_slot(&slot, false))
}

/// Disables Fetch interception for a target. Returns false when no slot exists.
pub async fn fetch_interception_stop(target_id: &str) -> A11yResult<bool> {
    let target_id = target_id.trim();
    let slot = fetch_registry()
        .slots
        .lock()
        .ok()
        .and_then(|mut slots| slots.remove(target_id));
    let Some(slot) = slot else {
        return Ok(false);
    };
    slot.page
        .execute(FetchDisableParams::default())
        .await
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("Fetch.disable for target {target_id}: {err}"),
        })?;
    Ok(true)
}

/// Number of targets with an active Fetch interception scaffold.
#[must_use]
pub fn fetch_interception_active_count() -> usize {
    fetch_registry().slots.lock().map(|s| s.len()).unwrap_or(0)
}

/// Tears down network capture for a target. Idempotent.
pub fn network_capture_stop(target_id: &str) -> bool {
    registry()
        .slots
        .lock()
        .ok()
        .and_then(|mut slots| slots.remove(target_id.trim()))
        .is_some()
}

/// Clears buffered network records for a target while keeping capture armed.
pub fn network_capture_clear(target_id: &str) -> bool {
    let slot = {
        let slots = match registry().slots.lock() {
            Ok(slots) => slots,
            Err(_) => return false,
        };
        match slots.get(target_id.trim()) {
            Some(slot) => Arc::clone(slot),
            None => return false,
        }
    };
    match slot.buffer.lock() {
        Ok(mut buffer) => {
            *buffer = RingBuffer::new(slot.capacity);
            true
        }
        Err(_) => false,
    }
}

/// Number of targets with a registered network capture slot.
#[must_use]
pub fn network_capture_active_count() -> usize {
    registry().slots.lock().map(|s| s.len()).unwrap_or(0)
}

fn lookup_live(target_id: &str) -> Option<Arc<NetworkCaptureSlot>> {
    let mut slots = registry().slots.lock().ok()?;
    match slots.get(target_id) {
        Some(slot) if !slot.listener_task.is_finished() => Some(Arc::clone(slot)),
        Some(_) => {
            slots.remove(target_id);
            None
        }
        None => None,
    }
}

fn lookup_fetch_live(target_id: &str) -> Option<Arc<FetchInterceptionSlot>> {
    let mut slots = fetch_registry().slots.lock().ok()?;
    match slots.get(target_id) {
        Some(slot) if !slot.listener_task.is_finished() => Some(Arc::clone(slot)),
        Some(_) => {
            slots.remove(target_id);
            None
        }
        None => None,
    }
}

fn fetch_interception_status_from_slot(
    slot: &FetchInterceptionSlot,
    newly_armed: bool,
) -> CdpFetchInterceptionStatus {
    let counters = slot.counters.lock().ok();
    CdpFetchInterceptionStatus {
        newly_armed,
        endpoint: slot.endpoint.clone(),
        cdp_target_id: slot.target_id.clone(),
        armed_at_unix_ms: slot.armed_at_unix_ms,
        pattern_count: slot.patterns.len(),
        paused_count: counters.as_ref().map_or(0, |c| c.paused_count),
        continued_count: counters.as_ref().map_or(0, |c| c.continued_count),
        continue_error_count: counters.as_ref().map_or(0, |c| c.continue_error_count),
        last_request_id: counters.as_ref().and_then(|c| c.last_request_id.clone()),
        last_url: counters.as_ref().and_then(|c| c.last_url.clone()),
        last_error: counters.as_ref().and_then(|c| c.last_error.clone()),
    }
}

fn fetch_patterns_to_cdp(
    patterns: &[CdpFetchInterceptionPattern],
) -> A11yResult<Vec<FetchRequestPattern>> {
    patterns
        .iter()
        .map(fetch_pattern_to_cdp)
        .collect::<A11yResult<Vec<_>>>()
}

fn fetch_pattern_to_cdp(pattern: &CdpFetchInterceptionPattern) -> A11yResult<FetchRequestPattern> {
    let mut builder = FetchRequestPattern::builder().request_stage(match pattern.request_stage {
        CdpFetchInterceptionStage::Request => FetchRequestStage::Request,
        CdpFetchInterceptionStage::Response => FetchRequestStage::Response,
    });
    if let Some(url_pattern) = pattern.url_pattern.as_deref() {
        if url_pattern.is_empty() {
            return Err(A11yError::CdpAttachFailed {
                detail: "Fetch request pattern url_pattern must not be empty".to_owned(),
            });
        }
        if url_pattern.contains('\0') {
            return Err(A11yError::CdpAttachFailed {
                detail: "Fetch request pattern url_pattern must not contain NUL".to_owned(),
            });
        }
        builder = builder.url_pattern(url_pattern.to_owned());
    }
    if let Some(resource_type) = pattern.resource_type.as_deref() {
        if resource_type.trim() != resource_type || resource_type.is_empty() {
            return Err(A11yError::CdpAttachFailed {
                detail: "Fetch request pattern resource_type must be non-empty without surrounding whitespace"
                    .to_owned(),
            });
        }
        let resource_type = resource_type
            .parse::<NetworkResourceType>()
            .map_err(|error| A11yError::CdpAttachFailed {
                detail: format!(
                    "Fetch request pattern resource_type {resource_type:?} is invalid: {error}"
                ),
            })?;
        builder = builder.resource_type(resource_type);
    }
    Ok(builder.build())
}

fn fetch_request_id_string(request_id: &FetchRequestId) -> String {
    <FetchRequestId as std::borrow::Borrow<str>>::borrow(request_id).to_owned()
}

fn normalize_request_id(request_id: &str) -> A11yResult<&str> {
    let request_id = request_id.trim();
    if request_id.is_empty() {
        return Err(A11yError::CdpAttachFailed {
            detail: "network request id must not be empty".to_owned(),
        });
    }
    Ok(request_id)
}

fn apply_request(buffer: &Arc<Mutex<RingBuffer>>, event: &EventRequestWillBeSent) {
    if let Ok(mut buf) = buffer.lock() {
        buf.apply_request_will_be_sent(event);
    }
}

fn apply_response(buffer: &Arc<Mutex<RingBuffer>>, event: &EventResponseReceived) {
    if let Ok(mut buf) = buffer.lock() {
        buf.apply_response_received(event);
    }
}

fn apply_finished(buffer: &Arc<Mutex<RingBuffer>>, event: &EventLoadingFinished) {
    if let Ok(mut buf) = buffer.lock() {
        buf.apply_loading_finished(event);
    }
}

fn apply_failed(buffer: &Arc<Mutex<RingBuffer>>, event: &EventLoadingFailed) {
    if let Ok(mut buf) = buffer.lock() {
        buf.apply_loading_failed(event);
    }
}

fn response_snapshot(
    response: &Response,
    event_timestamp_s: Option<f64>,
    resource_type: Option<&str>,
) -> CdpNetworkResponseSnapshot {
    CdpNetworkResponseSnapshot {
        url: response.url.clone(),
        status: response.status,
        status_text: response.status_text.clone(),
        headers: headers_value(&response.headers),
        request_headers: response.request_headers.as_ref().map(headers_value),
        mime_type: response.mime_type.clone(),
        protocol: response.protocol.clone(),
        remote_ip_address: response.remote_ip_address.clone(),
        remote_port: response.remote_port,
        encoded_data_length: response.encoded_data_length,
        timing: response
            .timing
            .as_ref()
            .and_then(|timing| serde_json::to_value(timing).ok()),
        response_time_ms: response
            .response_time
            .as_ref()
            .map(|t| timestamp_s(t) * 1000.0),
        from_disk_cache: response.from_disk_cache,
        from_service_worker: response.from_service_worker,
        from_prefetch_cache: response.from_prefetch_cache,
        from_early_hints: response.from_early_hints,
        timestamp_s: event_timestamp_s,
        resource_type: resource_type.map(str::to_owned),
    }
}

fn headers_value(headers: &Headers) -> Value {
    headers.inner().clone()
}

fn now_unix_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn enum_str<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn timestamp_s<T: Serialize>(value: &T) -> f64 {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chromiumoxide::cdp::browser_protocol::network::{MonotonicTime, ResourceType};
    use serde_json::json;

    fn request_event(request_id: &str, url: &str) -> EventRequestWillBeSent {
        serde_json::from_value(json!({
            "requestId": request_id,
            "loaderId": "loader-1",
            "documentURL": "https://example.test/",
            "request": {
                "url": url,
                "method": "GET",
                "headers": {"Accept": "text/html"},
                "initialPriority": "VeryHigh",
                "referrerPolicy": "strict-origin-when-cross-origin"
            },
            "timestamp": 10.0,
            "wallTime": 1710000000.25,
            "initiator": {"type": "other"},
            "redirectHasExtraInfo": false,
            "type": "Document",
            "frameId": "frame-1"
        }))
        .expect("request event")
    }

    fn response_event(request_id: &str, status: i64) -> EventResponseReceived {
        serde_json::from_value(json!({
            "requestId": request_id,
            "loaderId": "loader-1",
            "timestamp": 10.5,
            "type": "Document",
            "response": {
                "url": "https://example.test/",
                "status": status,
                "statusText": "OK",
                "headers": {"content-type": "text/html"},
                "mimeType": "text/html",
                "charset": "",
                "connectionReused": false,
                "connectionId": 1,
                "remoteIPAddress": "127.0.0.1",
                "remotePort": 443,
                "encodedDataLength": 512,
                "protocol": "h2",
                "securityState": "secure"
            },
            "hasExtraInfo": false,
            "frameId": "frame-1"
        }))
        .expect("response event")
    }

    fn finished_event(request_id: &str) -> EventLoadingFinished {
        serde_json::from_value(json!({
            "requestId": request_id,
            "timestamp": 11.0,
            "encodedDataLength": 1024
        }))
        .expect("loading finished event")
    }

    fn failed_event(request_id: &str) -> EventLoadingFailed {
        EventLoadingFailed {
            request_id: request_id.to_owned().into(),
            timestamp: MonotonicTime::new(12.0),
            r#type: ResourceType::Image,
            error_text: "net::ERR_ABORTED".to_owned(),
            canceled: Some(true),
            blocked_reason: None,
            cors_error_status: None,
        }
    }

    #[test]
    fn ring_buffer_keeps_bounded_request_records_and_evicts_oldest() {
        let mut buffer = RingBuffer::new(2);
        buffer.apply_request_will_be_sent(&request_event("r1", "https://example.test/one"));
        buffer.apply_request_will_be_sent(&request_event("r2", "https://example.test/two"));
        buffer.apply_request_will_be_sent(&request_event("r3", "https://example.test/three"));

        let request_ids: Vec<&str> = buffer
            .entries
            .iter()
            .map(|entry| entry.request_id.as_str())
            .collect();
        assert_eq!(request_ids, vec!["r2", "r3"]);
        assert_eq!(buffer.dropped, 1);
        assert_eq!(buffer.cursor(), 3);
    }

    #[test]
    fn response_and_finished_events_update_existing_request_cursor() {
        let mut buffer = RingBuffer::new(10);
        buffer.apply_request_will_be_sent(&request_event("r1", "https://example.test/"));
        let after_request_cursor = buffer.cursor();
        buffer.apply_response_received(&response_event("r1", 200));
        buffer.apply_loading_finished(&finished_event("r1"));

        assert_eq!(buffer.entries.len(), 1);
        let entry = &buffer.entries[0];
        assert_eq!(entry.first_seq, 0);
        assert_eq!(entry.seq, 2);
        assert!(entry.seq >= after_request_cursor);
        assert_eq!(entry.method.as_deref(), Some("GET"));
        assert_eq!(entry.url.as_deref(), Some("https://example.test/"));
        assert_eq!(entry.resource_type.as_deref(), Some("Document"));
        assert_eq!(entry.response.as_ref().map(|r| r.status), Some(200));
        assert_eq!(entry.encoded_data_length, Some(1024.0));
        assert!(entry.response_received);
        assert!(entry.loading_finished);
        assert!(!entry.loading_failed);
    }

    #[test]
    fn loading_failed_creates_failure_entry_with_error_text() {
        let mut buffer = RingBuffer::new(10);
        buffer.apply_loading_failed(&failed_event("r-failed"));

        let entry = &buffer.entries[0];
        assert_eq!(entry.request_id, "r-failed");
        assert_eq!(entry.resource_type.as_deref(), Some("Image"));
        assert!(entry.loading_failed);
        assert!(!entry.loading_finished);
        assert_eq!(
            entry.failure_error_text.as_deref(),
            Some("net::ERR_ABORTED")
        );
        assert_eq!(entry.failure_canceled, Some(true));
    }

    #[test]
    fn read_filter_sorts_by_latest_event_cursor_for_delta_reads() {
        let mut buffer = RingBuffer::new(10);
        buffer.apply_request_will_be_sent(&request_event("r1", "https://example.test/one"));
        let since = buffer.cursor();
        buffer.apply_request_will_be_sent(&request_event("r2", "https://example.test/two"));
        buffer.apply_response_received(&response_event("r1", 201));

        let mut entries: Vec<CdpNetworkEntry> = buffer
            .entries
            .iter()
            .filter(|entry| entry.seq >= since)
            .cloned()
            .collect();
        entries.sort_by_key(|entry| entry.seq);

        let ids: Vec<&str> = entries
            .iter()
            .map(|entry| entry.request_id.as_str())
            .collect();
        assert_eq!(ids, vec!["r2", "r1"]);
        assert_eq!(entries[1].response.as_ref().map(|r| r.status), Some(201));
    }

    #[test]
    fn fetch_pattern_conversion_preserves_url_resource_type_and_stage() {
        let converted = fetch_pattern_to_cdp(&CdpFetchInterceptionPattern {
            url_pattern: Some("https://example.test/api/*".to_owned()),
            resource_type: Some("XHR".to_owned()),
            request_stage: CdpFetchInterceptionStage::Response,
        })
        .expect("pattern converts");
        let value = serde_json::to_value(converted).expect("pattern json");
        assert_eq!(value["urlPattern"], "https://example.test/api/*");
        assert_eq!(value["resourceType"], "XHR");
        assert_eq!(value["requestStage"], "Response");
    }

    #[test]
    fn fetch_pattern_conversion_rejects_invalid_values() {
        assert!(
            fetch_pattern_to_cdp(&CdpFetchInterceptionPattern {
                url_pattern: Some(String::new()),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            fetch_pattern_to_cdp(&CdpFetchInterceptionPattern {
                resource_type: Some("NotAResource".to_owned()),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[test]
    fn fetch_request_id_string_round_trips_generated_id() {
        let request_id = FetchRequestId::from("intercept-1".to_owned());
        assert_eq!(fetch_request_id_string(&request_id), "intercept-1");
    }
}
