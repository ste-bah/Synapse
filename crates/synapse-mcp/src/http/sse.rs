use std::{
    collections::{BTreeMap, VecDeque},
    convert::Infallible,
    num::NonZeroUsize,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, Sse},
    },
};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use synapse_core::{Event, EventFilter};
use synapse_reflex::{
    EventBus, EventBusError, PublishReport, SUBSCRIBER_QUEUE_CAPACITY, SubscriberHandle,
};

const LAST_EVENT_ID: &str = "Last-Event-ID";
const SUBSCRIPTION_ID_HEADER: &str = "Synapse-Subscription-Id";
const MANUAL_ENV: &str = "SYNAPSE_HTTP_SSE_MANUAL";
const SSE_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Clone, Debug)]
pub struct SseState {
    inner: Arc<SseStateInner>,
}

#[derive(Debug)]
struct SseStateInner {
    event_bus: EventBus,
    subscriptions: Mutex<BTreeMap<String, Arc<Subscription>>>,
    manual_routes_enabled: bool,
}

#[derive(Debug)]
struct Subscription {
    handle: SubscriberHandle,
    ring: Mutex<VecDeque<BufferedEvent>>,
    next_stream_seq: AtomicU64,
    dropped_total: AtomicU64,
    lossy_pending: AtomicBool,
}

#[derive(Clone, Debug)]
struct BufferedEvent {
    stream_seq: u64,
    event: Event,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EventsQuery {
    pub subscription_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StatsQuery {
    pub subscription_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PublishRequest {
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, Serialize)]
struct PublishResponse {
    matched: usize,
    queued: usize,
    dropped: u64,
    subscriptions_synced: usize,
}

#[derive(Clone, Debug, Serialize)]
struct StatsResponse {
    subscription_id: String,
    ring_len: usize,
    oldest_seq: Option<u64>,
    latest_seq: Option<u64>,
    oldest_event_seq: Option<u64>,
    latest_event_seq: Option<u64>,
    dropped_total: u64,
    events_dropped_for_subscriber: u64,
    lossy_pending: bool,
}

#[derive(Clone, Debug)]
enum SseFrame {
    SubscriptionStarted {
        subscription_id: String,
        lossy: bool,
    },
    Event {
        subscription_id: String,
        stream_seq: u64,
        event: Event,
        lossy: bool,
    },
}

#[derive(Debug)]
struct LiveStreamState {
    subscription: Arc<Subscription>,
    pending: VecDeque<SseFrame>,
    last_sent_seq: Option<u64>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum SseOpenError {
    BadLastEventId,
    SubscribeUnavailable(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SseSubscribeError {
    CapReached { limit: usize },
    FilterInvalid { detail: String },
    StateUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SseCancelError {
    NotFound,
    StateUnavailable,
}

impl SseState {
    #[cfg(test)]
    pub(crate) fn from_env() -> Self {
        Self::with_max_subscriptions(synapse_reflex::DEFAULT_MAX_SUBSCRIPTIONS_NONZERO)
    }

    pub(crate) fn with_max_subscriptions(max_subscriptions: NonZeroUsize) -> Self {
        Self {
            inner: Arc::new(SseStateInner {
                event_bus: EventBus::with_max_subscriptions(max_subscriptions),
                subscriptions: Mutex::new(BTreeMap::new()),
                manual_routes_enabled: manual_routes_enabled(),
            }),
        }
    }

    pub(super) fn open(&self, headers: &HeaderMap, query: EventsQuery) -> Response {
        let last_event_id = match parse_last_event_id(headers) {
            Ok(value) => value,
            Err(error) => return error.into_response(),
        };
        let subscription = match self.subscription_for(query.subscription_id, last_event_id) {
            Ok(subscription) => subscription,
            Err(error) => return error.into_response(),
        };
        let frames = Self::frames_after(&subscription, last_event_id);
        Self::sse_response(subscription, frames, last_event_id)
    }

    pub(crate) fn subscribe(
        &self,
        filter: EventFilter,
        kinds: Vec<String>,
        snapshot_first: bool,
    ) -> Result<String, SseSubscribeError> {
        self.create_subscription_with(filter, kinds, snapshot_first)
            .map(|subscription| subscription.id().to_owned())
    }

    pub(crate) fn event_bus(&self) -> EventBus {
        self.inner.event_bus.clone()
    }

    pub(crate) fn active_subscription_count(&self) -> usize {
        self.inner
            .subscriptions
            .lock()
            .map_or(0, |subscriptions| subscriptions.len())
    }

    pub(crate) fn cancel(&self, id: &str) -> Result<(), SseCancelError> {
        let removed_from_map = {
            let mut subscriptions = self
                .inner
                .subscriptions
                .lock()
                .map_err(|_| SseCancelError::StateUnavailable)?;
            subscriptions.remove(id).is_some()
        };
        let removed_from_bus = self.inner.event_bus.unsubscribe(id);
        if removed_from_map || removed_from_bus {
            Ok(())
        } else {
            Err(SseCancelError::NotFound)
        }
    }

    pub(super) fn publish(&self, request: PublishRequest) -> Response {
        if !self.inner.manual_routes_enabled {
            return StatusCode::NOT_FOUND.into_response();
        }
        let report = self.publish_events(request.events);
        let subscriptions_synced = self.sync_all();
        axum::Json(PublishResponse {
            matched: report.matched,
            queued: report.queued,
            dropped: report.dropped,
            subscriptions_synced,
        })
        .into_response()
    }

    fn publish_events(&self, events: Vec<Event>) -> PublishReport {
        let mut total = PublishReport::default();
        for event in events {
            let report = self.inner.event_bus.publish(event);
            total.matched = total.matched.saturating_add(report.matched);
            total.queued = total.queued.saturating_add(report.queued);
            total.dropped = total.dropped.saturating_add(report.dropped);
        }
        total
    }

    pub(super) fn stats(&self, query: &StatsQuery) -> Response {
        if !self.inner.manual_routes_enabled {
            return StatusCode::NOT_FOUND.into_response();
        }
        let Some(subscription) = self.existing_subscription(&query.subscription_id) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        Self::sync_subscription(&subscription);
        axum::Json(subscription.stats()).into_response()
    }

    fn frames_after(
        subscription: &Arc<Subscription>,
        last_event_id: Option<u64>,
    ) -> VecDeque<SseFrame> {
        Self::sync_subscription(subscription);
        let (events, gap_lossy) = subscription.events_after(last_event_id);
        if events.is_empty() {
            return VecDeque::new();
        }
        let pending_lossy = subscription.take_lossy_pending();
        let lossy = gap_lossy || pending_lossy;
        let mut frames = VecDeque::with_capacity(events.len() + usize::from(lossy));
        if lossy {
            frames.push_back(SseFrame::subscription_started(subscription.id(), true));
        }
        for (index, event) in events.into_iter().enumerate() {
            frames.push_back(SseFrame::event(
                subscription.id(),
                event,
                lossy && index == 0,
            ));
        }
        frames
    }

    fn sse_response(
        subscription: Arc<Subscription>,
        frames: VecDeque<SseFrame>,
        last_sent_seq: Option<u64>,
    ) -> Response {
        let subscription_id = subscription.id().to_owned();
        let stream = live_stream(subscription, frames, last_sent_seq);
        let mut response = Sse::new(stream).into_response();
        if let Ok(header_value) = HeaderValue::from_str(&subscription_id) {
            response
                .headers_mut()
                .insert(SUBSCRIPTION_ID_HEADER, header_value);
        }
        response
    }

    fn subscription_for(
        &self,
        subscription_id: Option<String>,
        last_event_id: Option<u64>,
    ) -> Result<Arc<Subscription>, SseOpenError> {
        if let Some(id) = subscription_id
            && let Some(subscription) = self.existing_subscription(&id)
        {
            Self::sync_subscription(&subscription);
            match last_event_id {
                None => return Ok(subscription),
                Some(last_id)
                    if subscription
                        .latest_seq()
                        .is_some_and(|latest_seq| last_id <= latest_seq) =>
                {
                    return Ok(subscription);
                }
                Some(_) => {}
            }
        }
        self.create_subscription()
    }

    fn existing_subscription(&self, id: &str) -> Option<Arc<Subscription>> {
        let subscriptions = self.inner.subscriptions.lock().ok()?;
        subscriptions.get(id).cloned()
    }

    fn create_subscription(&self) -> Result<Arc<Subscription>, SseOpenError> {
        self.create_subscription_with(EventFilter::All, Vec::new(), false)
            .map_err(|error| SseOpenError::SubscribeUnavailable(error.code()))
    }

    fn create_subscription_with(
        &self,
        filter: EventFilter,
        kinds: Vec<String>,
        snapshot_first: bool,
    ) -> Result<Arc<Subscription>, SseSubscribeError> {
        let handle = self
            .inner
            .event_bus
            .subscribe(filter, kinds, snapshot_first)
            .map_err(SseSubscribeError::from)?;
        let id = handle.id().to_owned();
        let subscription = Arc::new(Subscription {
            handle,
            ring: Mutex::new(VecDeque::with_capacity(SUBSCRIBER_QUEUE_CAPACITY)),
            next_stream_seq: AtomicU64::new(1),
            dropped_total: AtomicU64::new(0),
            lossy_pending: AtomicBool::new(false),
        });
        {
            let mut subscriptions = self
                .inner
                .subscriptions
                .lock()
                .map_err(|_| SseSubscribeError::StateUnavailable)?;
            subscriptions.insert(id, Arc::clone(&subscription));
        }
        Ok(subscription)
    }

    fn sync_all(&self) -> usize {
        let subscriptions = self
            .inner
            .subscriptions
            .lock()
            .map(|items| items.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for subscription in &subscriptions {
            Self::sync_subscription(subscription);
        }
        subscriptions.len()
    }

    fn sync_subscription(subscription: &Subscription) {
        let events = subscription.handle.drain();
        if events.is_empty() {
            let bus_dropped = subscription.handle.take_dropped_since_read();
            if bus_dropped > 0 {
                subscription
                    .dropped_total
                    .fetch_add(bus_dropped, Ordering::AcqRel);
                subscription.lossy_pending.store(true, Ordering::Release);
            }
            if subscription.handle.take_lossy() {
                subscription.lossy_pending.store(true, Ordering::Release);
            }
            return;
        }
        let bus_dropped = subscription.handle.take_dropped_since_read();
        if bus_dropped > 0 {
            subscription
                .dropped_total
                .fetch_add(bus_dropped, Ordering::AcqRel);
            subscription.lossy_pending.store(true, Ordering::Release);
        }
        if subscription.handle.take_lossy() {
            subscription.lossy_pending.store(true, Ordering::Release);
        }
        subscription.push_events(events);
    }
}

impl Subscription {
    fn id(&self) -> &str {
        self.handle.id()
    }

    fn push_events(&self, events: Vec<Event>) {
        let Ok(mut ring) = self.ring.lock() else {
            return;
        };
        for event in events {
            if ring.len() == SUBSCRIBER_QUEUE_CAPACITY {
                ring.pop_front();
                self.dropped_total.fetch_add(1, Ordering::AcqRel);
                self.lossy_pending.store(true, Ordering::Release);
            }
            let stream_seq = self.next_stream_seq.fetch_add(1, Ordering::AcqRel);
            ring.push_back(BufferedEvent { stream_seq, event });
        }
    }

    fn events_after(&self, last_event_id: Option<u64>) -> (Vec<BufferedEvent>, bool) {
        let Ok(ring) = self.ring.lock() else {
            return (Vec::new(), false);
        };
        let Some(last_event_id) = last_event_id else {
            return (ring.iter().cloned().collect(), false);
        };
        let gap_lossy = ring
            .front()
            .is_some_and(|first| last_event_id.saturating_add(1) < first.stream_seq);
        let events = ring
            .iter()
            .filter(|event| event.stream_seq > last_event_id)
            .cloned()
            .collect();
        (events, gap_lossy)
    }

    fn latest_seq(&self) -> Option<u64> {
        self.ring
            .lock()
            .ok()
            .and_then(|ring| ring.back().map(|event| event.stream_seq))
    }

    fn take_lossy_pending(&self) -> bool {
        self.lossy_pending.swap(false, Ordering::AcqRel)
    }

    fn stats(&self) -> StatsResponse {
        let (ring_len, oldest_seq, latest_seq, oldest_event_seq, latest_event_seq) = self
            .ring
            .lock()
            .map_or((0, None, None, None, None), |ring| {
                (
                    ring.len(),
                    ring.front().map(|event| event.stream_seq),
                    ring.back().map(|event| event.stream_seq),
                    ring.front().map(|event| event.event.seq),
                    ring.back().map(|event| event.event.seq),
                )
            });
        StatsResponse {
            subscription_id: self.id().to_owned(),
            ring_len,
            oldest_seq,
            latest_seq,
            oldest_event_seq,
            latest_event_seq,
            dropped_total: self.dropped_total.load(Ordering::Acquire),
            events_dropped_for_subscriber: self.dropped_total.load(Ordering::Acquire),
            lossy_pending: self.lossy_pending.load(Ordering::Acquire),
        }
    }
}

fn parse_last_event_id(headers: &HeaderMap) -> Result<Option<u64>, SseOpenError> {
    let Some(raw) = headers.get(LAST_EVENT_ID) else {
        return Ok(None);
    };
    let raw = raw
        .to_str()
        .map_err(|_| SseOpenError::BadLastEventId)?
        .trim();
    raw.parse::<u64>()
        .map(Some)
        .map_err(|_| SseOpenError::BadLastEventId)
}

impl SseOpenError {
    fn into_response(self) -> Response {
        match self {
            Self::BadLastEventId => {
                (StatusCode::BAD_REQUEST, "malformed Last-Event-ID").into_response()
            }
            Self::SubscribeUnavailable(code) => {
                (StatusCode::SERVICE_UNAVAILABLE, code).into_response()
            }
        }
    }
}

impl SseSubscribeError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::CapReached { .. } => synapse_core::error_codes::SUBSCRIPTION_CAP_REACHED,
            Self::FilterInvalid { .. } => synapse_core::error_codes::TOOL_PARAMS_INVALID,
            Self::StateUnavailable => synapse_core::error_codes::TOOL_INTERNAL_ERROR,
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::CapReached { limit } => {
                format!("subscription cap reached: limit {limit}")
            }
            Self::FilterInvalid { detail } => format!("event filter invalid: {detail}"),
            Self::StateUnavailable => "subscription state lock poisoned".to_owned(),
        }
    }
}

impl SseCancelError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::NotFound => synapse_core::error_codes::SUBSCRIPTION_NOT_FOUND,
            Self::StateUnavailable => synapse_core::error_codes::TOOL_INTERNAL_ERROR,
        }
    }

    pub(crate) fn message(&self, subscription_id: &str) -> String {
        match self {
            Self::NotFound => format!("subscription not found: {subscription_id}"),
            Self::StateUnavailable => "subscription state lock poisoned".to_owned(),
        }
    }
}

impl From<EventBusError> for SseSubscribeError {
    fn from(value: EventBusError) -> Self {
        match value {
            EventBusError::SubscriptionCapReached { limit } => Self::CapReached { limit },
            EventBusError::FilterInvalid { detail } => Self::FilterInvalid { detail },
        }
    }
}

fn live_stream(
    subscription: Arc<Subscription>,
    pending: VecDeque<SseFrame>,
    last_sent_seq: Option<u64>,
) -> impl futures_util::Stream<Item = Result<SseEvent, Infallible>> + Send + 'static {
    stream::unfold(
        LiveStreamState {
            subscription,
            pending,
            last_sent_seq,
        },
        |mut state| async move {
            loop {
                if let Some(frame) = state.pending.pop_front() {
                    if let Some(seq) = frame.seq() {
                        state.last_sent_seq = Some(seq);
                    }
                    return Some((Ok(frame.into_event()), state));
                }
                state.pending.extend(SseState::frames_after(
                    &state.subscription,
                    state.last_sent_seq,
                ));
                if state.pending.is_empty() {
                    tokio::time::sleep(SSE_POLL_INTERVAL).await;
                }
            }
        },
    )
}

impl SseFrame {
    fn subscription_started(subscription_id: &str, lossy: bool) -> Self {
        Self::SubscriptionStarted {
            subscription_id: subscription_id.to_owned(),
            lossy,
        }
    }

    fn event(subscription_id: &str, buffered: BufferedEvent, lossy: bool) -> Self {
        Self::Event {
            subscription_id: subscription_id.to_owned(),
            stream_seq: buffered.stream_seq,
            event: buffered.event,
            lossy,
        }
    }

    const fn seq(&self) -> Option<u64> {
        match self {
            Self::SubscriptionStarted { .. } => None,
            Self::Event { stream_seq, .. } => Some(*stream_seq),
        }
    }

    fn into_event(self) -> SseEvent {
        match self {
            Self::SubscriptionStarted {
                subscription_id,
                lossy,
            } => SseEvent::default()
                .event("subscription_started")
                .data(subscription_started_data(&subscription_id, lossy).to_string()),
            Self::Event {
                subscription_id,
                stream_seq,
                event,
                lossy,
            } => {
                let id = stream_seq.to_string();
                SseEvent::default()
                    .id(id)
                    .event("synapse/event")
                    .data(event_data(&subscription_id, stream_seq, &event, lossy).to_string())
            }
        }
    }
}

fn subscription_started_data(subscription_id: &str, lossy: bool) -> serde_json::Value {
    serde_json::json!({
        "subscription_id": subscription_id,
        "lossy": lossy,
        "buffer_capacity": SUBSCRIBER_QUEUE_CAPACITY,
    })
}

fn event_data(
    subscription_id: &str,
    stream_seq: u64,
    event: &Event,
    lossy: bool,
) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "synapse/event",
        "params": {
            "subscription_id": subscription_id,
            "stream_seq": stream_seq,
            "lossy": lossy,
            "event": event,
        }
    })
}

fn manual_routes_enabled() -> bool {
    std::env::var(MANUAL_ENV).is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use synapse_core::{Event, EventFilter, EventSource};

    use super::{BufferedEvent, SseFrame, SseState, event_data};

    fn event(seq: u64, kind: &str) -> Event {
        Event {
            seq,
            at: Utc::now(),
            source: EventSource::System,
            kind: kind.to_owned(),
            data: serde_json::json!({"value": seq}),
            correlations: Vec::new(),
        }
    }

    #[test]
    fn event_frame_is_stable_for_known_input() {
        let event = event(7, "tick");
        let data = event_data("sub-1", 1, &event, true).to_string();
        assert!(data.contains("\"subscription_id\":\"sub-1\""));
        assert!(data.contains("\"stream_seq\":1"));
        assert!(data.contains("\"seq\":7"));
        assert!(data.contains("\"lossy\":true"));
        assert_eq!(
            SseFrame::event(
                "sub-1",
                BufferedEvent {
                    stream_seq: 1,
                    event,
                },
                true,
            )
            .seq(),
            Some(1)
        );
    }

    #[test]
    fn state_creates_subscription_with_empty_initial_body() {
        let state = SseState::from_env();
        let response = state.open(&axum::http::HeaderMap::new(), super::EventsQuery::default());
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[test]
    fn sparse_domain_seq_gets_contiguous_stream_seq_without_loss() {
        let state = SseState::from_env();
        let subscription = state
            .create_subscription_with(
                EventFilter::Kind {
                    kind: "reflex_fired".to_owned(),
                },
                Vec::new(),
                false,
            )
            .expect("subscription should register");

        state.publish_events(vec![event(62_488, "reflex_fired")]);
        SseState::sync_subscription(&subscription);
        let stats = subscription.stats();
        assert_eq!(stats.ring_len, 1);
        assert_eq!(stats.oldest_seq, Some(1));
        assert_eq!(stats.latest_seq, Some(1));
        assert_eq!(stats.oldest_event_seq, Some(62_488));
        assert_eq!(stats.latest_event_seq, Some(62_488));
        assert_eq!(stats.dropped_total, 0);
        assert!(!stats.lossy_pending);

        let frames = SseState::frames_after(&subscription, None);
        assert_eq!(frames.len(), 1);
        match frames.front().expect("one event frame") {
            SseFrame::Event {
                stream_seq,
                event,
                lossy,
                ..
            } => {
                assert_eq!(*stream_seq, 1);
                assert_eq!(event.seq, 62_488);
                assert!(!lossy);
            }
            other => panic!("expected event frame, got {other:?}"),
        }
    }

    #[test]
    fn last_event_id_uses_stream_seq_not_domain_event_seq() {
        let state = SseState::from_env();
        let subscription = state
            .create_subscription_with(EventFilter::All, Vec::new(), false)
            .expect("subscription should register");

        state.publish_events(vec![event(10, "first"), event(1_000, "second")]);
        let frames = SseState::frames_after(&subscription, Some(1));
        assert_eq!(frames.len(), 1);
        match frames.front().expect("second event frame") {
            SseFrame::Event {
                stream_seq,
                event,
                lossy,
                ..
            } => {
                assert_eq!(*stream_seq, 2);
                assert_eq!(event.seq, 1_000);
                assert!(!lossy);
            }
            other => panic!("expected event frame, got {other:?}"),
        }
    }
}
