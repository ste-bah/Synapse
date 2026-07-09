use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use serde::Serialize;
use synapse_core::Event;
use synapse_reflex::{EVENTS_DROPPED_METRIC, SUBSCRIBER_QUEUE_CAPACITY, SubscriberHandle};

use super::lossy;

#[derive(Debug)]
pub(super) struct Subscription {
    pub(super) handle: SubscriberHandle,
    ring: Mutex<VecDeque<BufferedEvent>>,
    next_stream_seq: AtomicU64,
    dropped_total: AtomicU64,
    lossy_pending: AtomicBool,
}

#[derive(Clone, Debug)]
pub(super) struct BufferedEvent {
    pub(super) stream_seq: u64,
    pub(super) event: Event,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct StatsResponse {
    pub(super) subscription_id: String,
    pub(super) ring_len: usize,
    pub(super) oldest_seq: Option<u64>,
    pub(super) latest_seq: Option<u64>,
    pub(super) oldest_event_seq: Option<u64>,
    pub(super) latest_event_seq: Option<u64>,
    pub(super) dropped_total: u64,
    pub(super) events_dropped_for_subscriber: u64,
    pub(super) lossy_pending: bool,
}

impl Subscription {
    pub(super) fn new(handle: SubscriberHandle) -> Self {
        Self {
            handle,
            ring: Mutex::new(VecDeque::with_capacity(SUBSCRIBER_QUEUE_CAPACITY)),
            next_stream_seq: AtomicU64::new(1),
            dropped_total: AtomicU64::new(0),
            lossy_pending: AtomicBool::new(false),
        }
    }

    pub(super) fn id(&self) -> &str {
        self.handle.id()
    }

    pub(super) fn push_events(&self, events: Vec<Event>) {
        let mut ring_dropped = 0_u64;
        let Ok(mut ring) = self.ring.lock() else {
            return;
        };
        for event in events {
            if ring.len() == SUBSCRIBER_QUEUE_CAPACITY {
                ring.pop_front();
                ring_dropped = ring_dropped.saturating_add(1);
            }
            let stream_seq = self.next_stream_seq.fetch_add(1, Ordering::AcqRel);
            ring.push_back(BufferedEvent { stream_seq, event });
        }
        drop(ring);
        if ring_dropped > 0 {
            self.record_dropped(ring_dropped);
            metrics::counter!(synapse_telemetry::metrics::SSE_BUFFER_OVERFLOWS_TOTAL)
                .increment(ring_dropped);
            metrics::counter!(
                EVENTS_DROPPED_METRIC,
                "subscription_id" => self.id().to_owned()
            )
            .increment(ring_dropped);
        }
    }

    pub(super) fn events_after(&self, last_event_id: Option<u64>) -> (Vec<BufferedEvent>, bool) {
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

    pub(super) fn latest_seq(&self) -> Option<u64> {
        self.ring
            .lock()
            .ok()
            .and_then(|ring| ring.back().map(|event| event.stream_seq))
    }

    pub(super) fn record_dropped(&self, count: u64) {
        lossy::record_dropped(&self.dropped_total, &self.lossy_pending, count);
    }

    pub(super) fn record_lossy(&self) {
        lossy::record_lossy(&self.lossy_pending);
    }

    pub(super) fn take_lossy_pending(&self) -> bool {
        lossy::take_pending(&self.lossy_pending)
    }

    pub(super) fn stats(&self) -> StatsResponse {
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
        let dropped_total = self.dropped_total.load(Ordering::Acquire);
        StatsResponse {
            subscription_id: self.id().to_owned(),
            ring_len,
            oldest_seq,
            latest_seq,
            oldest_event_seq,
            latest_event_seq,
            dropped_total,
            events_dropped_for_subscriber: dropped_total,
            lossy_pending: lossy::pending(&self.lossy_pending),
        }
    }
}
