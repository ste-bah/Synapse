//! Always-on operator-activity recorder (#837, epic #829).
//!
//! Consumes the daemon's single WinEvent hook stream — teed from
//! [`super::a11y_events::A11yEventBridge`], because the process-wide
//! `WIN_EVENT_SENDER` permits exactly one hook subscription — and persists
//! `CF_TIMELINE` rows: foreground app switches, foreground window title
//! changes, idle/active transitions (`GetLastInputInfo` polled at a coarse
//! interval), and recorder session boundaries.
//!
//! Design constraints carried from ADR 2026-06-11-timeline-data-model and
//! field-tested foreground-tracking practice:
//!
//! - WinEvents are *triggers*, not truth. `EVENT_SYSTEM_FOREGROUND` is
//!   delivered asynchronously and frequently names an invisible Alt-Tab
//!   staging window (`ForegroundStaging`), a window not yet shown, or one
//!   already destroyed. When the event hwnd is unusable the recorder
//!   re-reads `GetForegroundWindow` — the source of truth — so a real app
//!   switch hiding behind a transient event still lands in the timeline.
//! - Every idle poll tick also reconciles recorded foreground state against
//!   the real foreground (rows tagged `source: "poll"`), so a missed
//!   WinEvent can never desync the timeline for longer than one interval.
//! - `EVENT_OBJECT_NAMECHANGE` fires for child objects too; a title row is
//!   written only when the *foreground* window's title actually changed.
//! - Idle detection mirrors ActivityWatch's defaults (180 s timeout, coarse
//!   poll); the `idle_start` row is backdated to the last-input instant so
//!   the timeline reflects when input actually stopped.
//!
//! Attribution: rows produced while an agent session holds the real-input
//! lease are tagged `agent { session_id }` (the lease is the canonical "an
//! agent owns the foreground" signal, epic #719); everything else is `human`.

use std::time::{Duration, Instant};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::json;
use sha2::{Digest, Sha256};
use synapse_a11y::{AccessibleEvent, AccessibleEventKind};
use synapse_core::types::{
    FsEventKind, Observation, TIMELINE_RECORD_VERSION, TimelineActor, TimelineKind, TimelineRecord,
};
use synapse_storage::{Db, cf, timeline::timeline_key};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use super::{
    interaction_cadence::{InteractionEvent, InteractionEventKind, InteractionHook},
    timeline_control::{RecorderControl, SuppressReason},
};
use crate::m1::{
    ClipboardTimelineSample, FsTimelineEvent, timeline_clipboard_enabled,
    timeline_file_activity_enabled,
};

/// Idle threshold override, in milliseconds. Default mirrors ActivityWatch.
pub const IDLE_TIMEOUT_ENV: &str = "SYNAPSE_TIMELINE_IDLE_TIMEOUT_MS";
const DEFAULT_IDLE_TIMEOUT_MS: u64 = 180_000;
const MIN_IDLE_POLL_INTERVAL_MS: u64 = 250;
const MAX_IDLE_POLL_INTERVAL_MS: u64 = 5_000;
const SHUTDOWN_ACK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecorderConfig {
    pub idle_timeout_ms: u64,
    pub idle_poll_interval_ms: u64,
}

impl RecorderConfig {
    /// Reads `SYNAPSE_TIMELINE_IDLE_TIMEOUT_MS` and derives the poll cadence.
    ///
    /// # Errors
    ///
    /// Returns an error when the variable is set but is not a positive
    /// integer; the daemon must refuse to start rather than record with a
    /// silently-wrong idle policy.
    pub fn from_env() -> Result<Self> {
        Self::from_raw(std::env::var(IDLE_TIMEOUT_ENV).ok().as_deref())
    }

    fn from_raw(raw: Option<&str>) -> Result<Self> {
        let idle_timeout_ms = match raw {
            None => DEFAULT_IDLE_TIMEOUT_MS,
            Some(value) => value.trim().parse::<u64>().with_context(|| {
                format!(
                    "{IDLE_TIMEOUT_ENV} must be a positive integer of milliseconds, got {value:?}"
                )
            })?,
        };
        if idle_timeout_ms == 0 {
            bail!("{IDLE_TIMEOUT_ENV} must be at least 1 millisecond, got 0");
        }
        let idle_poll_interval_ms = (idle_timeout_ms / 4)
            .clamp(MIN_IDLE_POLL_INTERVAL_MS, MAX_IDLE_POLL_INTERVAL_MS)
            .min(idle_timeout_ms);
        Ok(Self {
            idle_timeout_ms,
            idle_poll_interval_ms,
        })
    }
}

enum RecorderMessage {
    Accessible(AccessibleEvent),
    Interaction(InteractionEvent),
    IdleProbe { idle_ms: u64 },
    FlushInteractions { done: oneshot::Sender<()> },
    Shutdown { done: oneshot::Sender<()> },
}

/// Shared write path: every producer (worker, spawn, drop backstop) goes
/// through one row encoder so key allocation and failure accounting are
/// uniform — and one gate, so pause/exclusion (#843) can never be bypassed
/// by a feed that forgot to check.
#[derive(Clone)]
struct TimelineWriter {
    db: Arc<Db>,
    control: Arc<RecorderControl>,
    seq: Arc<AtomicU32>,
    rows_written: Arc<AtomicU64>,
    write_failures: Arc<AtomicU64>,
    rows_suppressed_paused: Arc<AtomicU64>,
    rows_suppressed_excluded: Arc<AtomicU64>,
}

impl TimelineWriter {
    fn try_write(
        &self,
        ts_ns: u64,
        kind: TimelineKind,
        actor: TimelineActor,
        app: Option<String>,
        payload: serde_json::Value,
    ) -> Result<()> {
        let record = TimelineRecord {
            record_version: TIMELINE_RECORD_VERSION,
            ts_ns,
            kind,
            actor,
            app,
            payload,
        };
        let value = serde_json::to_vec(&record)
            .with_context(|| format!("encode CF_TIMELINE {kind:?} record"))?;
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let key = timeline_key(ts_ns, seq);
        self.db
            .put_batch(cf::CF_TIMELINE, [(key, value)])
            .with_context(|| format!("write CF_TIMELINE {kind:?} row ts_ns={ts_ns} seq={seq}"))?;
        self.rows_written.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(
            code = "TIMELINE_ROW_WRITTEN",
            kind = ?kind,
            ts_ns,
            seq,
            "timeline row written"
        );
        Ok(())
    }

    /// Forces pending batched writes to disk. The batcher acks `put_batch`
    /// on enqueue and flushes on a 100 ms cadence, so anything that must be
    /// durable *now* (session boundaries at shutdown) needs an explicit
    /// flush — a return value alone does not prove the row is on disk.
    fn flush_logged(&self) {
        if let Err(error) = self.db.flush() {
            tracing::error!(
                code = "TIMELINE_FLUSH_FAILED",
                detail = %error,
                "failed to flush batched timeline writes"
            );
        }
    }

    /// The pause/exclusion gate (#843). Checked by every steady-state write;
    /// suppression is counted and debug-logged, never silent.
    fn suppressed(&self, kind: TimelineKind, app: Option<&str>) -> bool {
        match self.control.suppress_reason(app) {
            None => false,
            Some(SuppressReason::Paused) => {
                self.rows_suppressed_paused.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(
                    code = "TIMELINE_ROW_SUPPRESSED_PAUSED",
                    kind = ?kind,
                    "timeline row suppressed: recorder is paused"
                );
                true
            }
            Some(SuppressReason::ExcludedApp) => {
                self.rows_suppressed_excluded
                    .fetch_add(1, Ordering::Relaxed);
                tracing::debug!(
                    code = "TIMELINE_ROW_SUPPRESSED_EXCLUDED",
                    kind = ?kind,
                    app = app.unwrap_or_default(),
                    "timeline row suppressed: process is excluded"
                );
                true
            }
        }
    }

    /// Write path for the steady-state worker: a failed row is a loud
    /// structured error plus a failure count (surfaced by `timeline_stats`,
    /// #842), never a panic that kills the recorder.
    fn write_logged(
        &self,
        ts_ns: u64,
        kind: TimelineKind,
        actor: TimelineActor,
        app: Option<String>,
        payload: serde_json::Value,
    ) {
        if self.suppressed(kind, app.as_deref()) {
            return;
        }
        if let Err(error) = self.try_write(ts_ns, kind, actor, app, payload) {
            self.write_failures.fetch_add(1, Ordering::Relaxed);
            tracing::error!(
                code = "TIMELINE_WRITE_FAILED",
                kind = ?kind,
                ts_ns,
                detail = %format!("{error:#}"),
                "failed to persist timeline row"
            );
        }
    }
}

/// Last recorded foreground window; the dedup baseline for focus/title rows.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ForegroundSnapshot {
    hwnd: i64,
    pid: u32,
    process_name: String,
    process_path: String,
    title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ForegroundTransition {
    Duplicate,
    TitleChanged,
    Switched,
}

fn classify_foreground_transition(
    prev: Option<&ForegroundSnapshot>,
    next: &ForegroundSnapshot,
) -> ForegroundTransition {
    match prev {
        Some(prev) if prev.hwnd == next.hwnd && prev.pid == next.pid => {
            // Same window: only the title can have moved.
            if prev.title == next.title {
                ForegroundTransition::Duplicate
            } else {
                ForegroundTransition::TitleChanged
            }
        }
        _ => ForegroundTransition::Switched,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdleEdge {
    Start,
    End,
}

const fn idle_transition(currently_idle: bool, idle_ms: u64, timeout_ms: u64) -> Option<IdleEdge> {
    if !currently_idle && idle_ms >= timeout_ms {
        Some(IdleEdge::Start)
    } else if currently_idle && idle_ms < timeout_ms {
        Some(IdleEdge::End)
    } else {
        None
    }
}

fn now_ts_ns() -> u64 {
    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or(i64::MAX);
    u64::try_from(nanos).unwrap_or(0)
}

/// Resolves who is driving the session right now. An agent session holding
/// the real-input lease owns foreground changes; the operator-preempt
/// sentinel and an unheld lease both mean the human.
fn current_actor() -> TimelineActor {
    let status = synapse_action::lease::status();
    match status.owner_session_id {
        Some(owner) if status.held && owner != synapse_action::OPERATOR_LEASE_OWNER_SESSION_ID => {
            TimelineActor::Agent { session_id: owner }
        }
        _ => TimelineActor::Human,
    }
}

const INTERACTION_BUCKET_NS: u64 = 30_000_000_000;
const INJECTED_UNATTRIBUTED_SESSION_ID: &str = "injected-input";
const MAX_BROWSER_NAV_DEDUPE_KEYS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserNavigationEvent {
    pub actor: TimelineActor,
    pub app: Option<String>,
    pub source: String,
    pub event: String,
    pub action: Option<String>,
    pub url: String,
    pub title: String,
    pub tab_id: Option<u32>,
    pub chrome_window_id: Option<i64>,
    pub window_hwnd: Option<i64>,
    pub cdp_target_id: Option<String>,
    pub endpoint: Option<String>,
    pub transport: Option<String>,
    pub requested_url: Option<String>,
    pub before_url: Option<String>,
    pub before_title: Option<String>,
    pub ready_state: Option<String>,
    pub observed_at_unix_ms: Option<u64>,
    pub active: Option<bool>,
    pub highlighted: Option<bool>,
    pub pinned: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InteractionBucket {
    bucket_start_ns: u64,
    bucket_end_ns: u64,
    first_event_ns: u64,
    last_event_ns: u64,
    hwnd: i64,
    pid: u32,
    process_name: String,
    process_path: String,
    title_sha256: String,
    actor: TimelineActor,
    input_origin: &'static str,
    keystroke_count: u64,
    click_count: u64,
    vertical_scroll_delta: i64,
    horizontal_scroll_delta: i64,
    app_switch_count: u64,
}

impl InteractionBucket {
    fn new(
        ts_ns: u64,
        context: &synapse_core::ForegroundContext,
        actor: TimelineActor,
        input_origin: &'static str,
    ) -> Self {
        let bucket_start_ns = bucket_start(ts_ns);
        Self {
            bucket_start_ns,
            bucket_end_ns: bucket_start_ns.saturating_add(INTERACTION_BUCKET_NS),
            first_event_ns: ts_ns,
            last_event_ns: ts_ns,
            hwnd: context.hwnd,
            pid: context.pid,
            process_name: context.process_name.clone(),
            process_path: context.process_path.clone(),
            title_sha256: sha256_hex(&context.window_title),
            actor,
            input_origin,
            keystroke_count: 0,
            click_count: 0,
            vertical_scroll_delta: 0,
            horizontal_scroll_delta: 0,
            app_switch_count: 0,
        }
    }

    fn accepts(
        &self,
        ts_ns: u64,
        context: &synapse_core::ForegroundContext,
        actor: &TimelineActor,
        input_origin: &'static str,
    ) -> bool {
        self.bucket_start_ns == bucket_start(ts_ns)
            && self.hwnd == context.hwnd
            && self.pid == context.pid
            && self.process_name == context.process_name
            && &self.actor == actor
            && self.input_origin == input_origin
    }

    fn note_event_time(&mut self, ts_ns: u64) {
        self.first_event_ns = self.first_event_ns.min(ts_ns);
        self.last_event_ns = self.last_event_ns.max(ts_ns);
    }

    fn input_event_count(&self) -> u64 {
        self.keystroke_count
            .saturating_add(self.click_count)
            .saturating_add(u64::from(self.vertical_scroll_delta != 0))
            .saturating_add(u64::from(self.horizontal_scroll_delta != 0))
    }

    fn is_empty(&self) -> bool {
        self.keystroke_count == 0
            && self.click_count == 0
            && self.vertical_scroll_delta == 0
            && self.horizontal_scroll_delta == 0
            && self.app_switch_count == 0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct InteractionAccumulator {
    current: Option<InteractionBucket>,
}

impl InteractionAccumulator {
    fn record_input(
        &mut self,
        event: &InteractionEvent,
        context: &synapse_core::ForegroundContext,
        actor: TimelineActor,
        input_origin: &'static str,
        writer: &TimelineWriter,
    ) {
        self.ensure_bucket(event.ts_ns, context, actor, input_origin, writer);
        let Some(bucket) = self.current.as_mut() else {
            return;
        };
        bucket.note_event_time(event.ts_ns);
        match event.kind {
            InteractionEventKind::Keystroke => {
                bucket.keystroke_count = bucket.keystroke_count.saturating_add(1);
            }
            InteractionEventKind::Click => {
                bucket.click_count = bucket.click_count.saturating_add(1);
            }
            InteractionEventKind::VerticalScroll { delta } => {
                bucket.vertical_scroll_delta = bucket
                    .vertical_scroll_delta
                    .saturating_add(i64::from(delta));
            }
            InteractionEventKind::HorizontalScroll { delta } => {
                bucket.horizontal_scroll_delta = bucket
                    .horizontal_scroll_delta
                    .saturating_add(i64::from(delta));
            }
        }
    }

    fn record_app_switch(
        &mut self,
        ts_ns: u64,
        context: &synapse_core::ForegroundContext,
        actor: TimelineActor,
        writer: &TimelineWriter,
    ) {
        self.ensure_bucket(ts_ns, context, actor, "foreground", writer);
        let Some(bucket) = self.current.as_mut() else {
            return;
        };
        bucket.note_event_time(ts_ns);
        bucket.app_switch_count = bucket.app_switch_count.saturating_add(1);
    }

    fn ensure_bucket(
        &mut self,
        ts_ns: u64,
        context: &synapse_core::ForegroundContext,
        actor: TimelineActor,
        input_origin: &'static str,
        writer: &TimelineWriter,
    ) {
        let needs_new = self.current.as_ref().map_or(true, |bucket| {
            !bucket.accepts(ts_ns, context, &actor, input_origin)
        });
        if needs_new {
            self.flush(writer);
            self.current = Some(InteractionBucket::new(ts_ns, context, actor, input_origin));
        }
    }

    fn flush(&mut self, writer: &TimelineWriter) {
        let Some(bucket) = self.current.take() else {
            return;
        };
        if bucket.is_empty() {
            return;
        }
        write_interaction_summary(writer, bucket);
    }
}

fn bucket_start(ts_ns: u64) -> u64 {
    (ts_ns / INTERACTION_BUCKET_NS).saturating_mul(INTERACTION_BUCKET_NS)
}

fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn browser_nav_dedupe_key(event: &BrowserNavigationEvent) -> String {
    format!(
        "{:?}\n{:?}\n{:?}\n{:?}\n{}\n{}",
        event.actor,
        event.tab_id,
        event.cdp_target_id,
        event.window_hwnd,
        event.url.trim(),
        event.title
    )
}

fn interaction_actor(injected: bool) -> (TimelineActor, &'static str) {
    if !injected {
        return (TimelineActor::Human, "physical");
    }
    match current_actor() {
        TimelineActor::Agent { session_id } => (TimelineActor::Agent { session_id }, "injected"),
        TimelineActor::Human => (
            TimelineActor::Agent {
                session_id: INJECTED_UNATTRIBUTED_SESSION_ID.to_owned(),
            },
            "injected",
        ),
    }
}

fn write_interaction_summary(writer: &TimelineWriter, bucket: InteractionBucket) {
    let duration_ms = bucket.last_event_ns.saturating_sub(bucket.first_event_ns) / 1_000_000;
    let payload = json!({
        "bucket_start_ns": bucket.bucket_start_ns,
        "bucket_end_ns": bucket.bucket_end_ns,
        "bucket_ms": INTERACTION_BUCKET_NS / 1_000_000,
        "first_event_ns": bucket.first_event_ns,
        "last_event_ns": bucket.last_event_ns,
        "duration_ms": duration_ms,
        "pid": bucket.pid,
        "hwnd": bucket.hwnd,
        "process_path": bucket.process_path,
        "window_title_sha256": bucket.title_sha256,
        "input_origin": bucket.input_origin,
        "keystroke_count": bucket.keystroke_count,
        "click_count": bucket.click_count,
        "scroll_vertical_delta": bucket.vertical_scroll_delta,
        "scroll_horizontal_delta": bucket.horizontal_scroll_delta,
        "app_switch_count": bucket.app_switch_count,
        "input_event_count": bucket.input_event_count(),
    });
    if let Err(error) = writer.try_write(
        bucket.last_event_ns,
        TimelineKind::InteractionSummary,
        bucket.actor,
        Some(bucket.process_name),
        payload,
    ) {
        writer.write_failures.fetch_add(1, Ordering::Relaxed);
        tracing::error!(
            code = "TIMELINE_INTERACTION_SUMMARY_WRITE_FAILED",
            detail = %format!("{error:#}"),
            "failed to persist interaction cadence summary row"
        );
    }
}

struct WorkerState {
    writer: TimelineWriter,
    config: RecorderConfig,
    foreground: Option<ForegroundSnapshot>,
    idle: bool,
    interactions: InteractionAccumulator,
}

impl WorkerState {
    fn handle_accessible(&mut self, event: &AccessibleEvent) {
        // Paused means *perceive nothing*: skip even the foreground/title
        // readbacks, not just the row writes. The snapshot is dropped so the
        // first post-resume trigger re-records reality from scratch.
        if self.writer.control.is_paused() {
            self.foreground = None;
            self.writer
                .rows_suppressed_paused
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        match event.kind {
            AccessibleEventKind::ForegroundChanged => self.handle_foreground(event.window_id),
            AccessibleEventKind::NameChanged => self.handle_name_change(event.window_id),
            _ => {}
        }
    }

    fn handle_interaction(&mut self, event: &InteractionEvent) {
        if self.writer.control.is_paused() {
            self.writer
                .rows_suppressed_paused
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        let context = match synapse_a11y::current_foreground_context() {
            Ok(context) => context,
            Err(error) => {
                tracing::debug!(
                    code = "TIMELINE_INTERACTION_FOREGROUND_NONE",
                    detail = %error,
                    "interaction cadence event had no foreground context"
                );
                return;
            }
        };
        if self.writer.suppressed(
            TimelineKind::InteractionSummary,
            Some(&context.process_name),
        ) {
            return;
        }
        let (actor, input_origin) = interaction_actor(event.injected);
        self.interactions
            .record_input(event, &context, actor, input_origin, &self.writer);
    }

    /// A `ForegroundChanged` WinEvent is a *trigger*, not the truth: it is
    /// delivered asynchronously, and its hwnd can be an Alt-Tab transient
    /// (`ForegroundStaging`), a window that has not been shown yet, or one
    /// that is already destroyed. When the event hwnd is not a usable visible
    /// window, the recorder re-reads the actual foreground window instead of
    /// dropping the trigger — otherwise a real app switch hiding behind a
    /// transient event would silently vanish from the timeline.
    fn handle_foreground(&mut self, window_id: i64) {
        let context = match self.resolve_foreground_trigger(window_id) {
            Some(context) => context,
            None => return,
        };
        self.apply_foreground(&context, "win_event");
    }

    fn resolve_foreground_trigger(
        &self,
        window_id: i64,
    ) -> Option<synapse_core::ForegroundContext> {
        match synapse_a11y::is_window_visible(window_id) {
            Ok(true) => match synapse_a11y::foreground_context(window_id) {
                Ok(context) => return Some(context),
                Err(error) => {
                    tracing::debug!(
                        code = "TIMELINE_FOREGROUND_EVENT_HWND_STALE",
                        hwnd = window_id,
                        detail = %error,
                        "event window vanished mid-resolve; re-reading the real foreground"
                    );
                }
            },
            Ok(false) => {
                tracing::debug!(
                    code = "TIMELINE_FOREGROUND_EVENT_HWND_INVISIBLE",
                    hwnd = window_id,
                    "event window is invisible (transient); re-reading the real foreground"
                );
            }
            Err(error) => {
                tracing::debug!(
                    code = "TIMELINE_FOREGROUND_EVENT_HWND_STALE",
                    hwnd = window_id,
                    detail = %error,
                    "event window vanished before visibility readback; re-reading the real foreground"
                );
            }
        }
        // Source of truth: whatever is actually foreground right now.
        match synapse_a11y::current_foreground_context() {
            Ok(context) => {
                if matches!(synapse_a11y::is_window_visible(context.hwnd), Ok(true)) {
                    Some(context)
                } else {
                    tracing::debug!(
                        code = "TIMELINE_FOREGROUND_UNSETTLED",
                        event_hwnd = window_id,
                        current_hwnd = context.hwnd,
                        "current foreground is itself transient; next trigger or poll will settle it"
                    );
                    None
                }
            }
            Err(error) => {
                tracing::debug!(
                    code = "TIMELINE_FOREGROUND_NONE",
                    event_hwnd = window_id,
                    detail = %error,
                    "no resolvable foreground window for this trigger"
                );
                None
            }
        }
    }

    /// Records the resolved foreground state, deduplicating against the last
    /// recorded snapshot. `source` records which trigger produced the row.
    fn apply_foreground(&mut self, context: &synapse_core::ForegroundContext, source: &str) {
        let next = ForegroundSnapshot {
            hwnd: context.hwnd,
            pid: context.pid,
            process_name: context.process_name.clone(),
            process_path: context.process_path.clone(),
            title: context.window_title.clone(),
        };
        // Excluded processes leave the dedup snapshot untouched: the moment
        // the exclusion lifts (or focus moves to a recordable app), the next
        // trigger classifies as a switch and records reality instead of
        // deduplicating against a window that was never written.
        if self
            .writer
            .suppressed(TimelineKind::FocusChange, Some(&next.process_name))
        {
            return;
        }
        match classify_foreground_transition(self.foreground.as_ref(), &next) {
            ForegroundTransition::Duplicate => {}
            ForegroundTransition::TitleChanged => self.write_title_change(&next),
            ForegroundTransition::Switched => {
                let ts_ns = now_ts_ns();
                let actor = current_actor();
                self.writer.write_logged(
                    ts_ns,
                    TimelineKind::FocusChange,
                    actor.clone(),
                    Some(next.process_name.clone()),
                    json!({
                        "title": next.title,
                        "process_path": next.process_path,
                        "pid": next.pid,
                        "hwnd": next.hwnd,
                        "source": source,
                    }),
                );
                self.interactions
                    .record_app_switch(ts_ns, context, actor, &self.writer);
            }
        }
        self.foreground = Some(next);
    }

    fn handle_name_change(&mut self, window_id: i64) {
        let Some(previous) = self.foreground.as_ref() else {
            return;
        };
        if previous.hwnd != window_id {
            return;
        }
        // NAMECHANGE also fires for child objects of the same HWND; re-read
        // the top-level title and only record a real change.
        let context = match synapse_a11y::foreground_context(window_id) {
            Ok(context) => context,
            Err(error) => {
                tracing::debug!(
                    code = "TIMELINE_TITLE_CONTEXT_UNRESOLVED",
                    hwnd = window_id,
                    detail = %error,
                    "foreground window vanished before title readback"
                );
                return;
            }
        };
        if context.window_title == previous.title {
            return;
        }
        let next = ForegroundSnapshot {
            hwnd: context.hwnd,
            pid: context.pid,
            process_name: context.process_name,
            process_path: context.process_path,
            title: context.window_title,
        };
        self.write_title_change(&next);
        self.foreground = Some(next);
    }

    fn write_title_change(&self, next: &ForegroundSnapshot) {
        let previous_title = self
            .foreground
            .as_ref()
            .map(|snapshot| snapshot.title.clone());
        self.writer.write_logged(
            now_ts_ns(),
            TimelineKind::TitleChange,
            current_actor(),
            Some(next.process_name.clone()),
            json!({
                "title": next.title,
                "previous_title": previous_title,
                "pid": next.pid,
                "hwnd": next.hwnd,
            }),
        );
    }

    fn handle_idle_probe(&mut self, idle_ms: u64) {
        if self.writer.control.is_paused() {
            self.foreground = None;
            // The idle tick doubles as the auto-resume clock: a pause armed
            // with `duration_ms` reopens the gate within one poll interval.
            if self.writer.control.auto_resume_due(now_ts_ns()) {
                match resume_recording(&self.writer, "auto_resume") {
                    Ok(_state) => {
                        tracing::info!(
                            code = "TIMELINE_RECORDER_AUTO_RESUMED",
                            "timeline recorder auto-resumed: pause deadline passed"
                        );
                    }
                    Err(error) => {
                        tracing::error!(
                            code = "TIMELINE_RECORDER_AUTO_RESUME_FAILED",
                            detail = %format!("{error:#}"),
                            "timeline auto-resume failed; retrying next idle tick"
                        );
                        return;
                    }
                }
            } else {
                return;
            }
        }
        self.reconcile_foreground();
        let Some(edge) = idle_transition(self.idle, idle_ms, self.config.idle_timeout_ms) else {
            return;
        };
        // Backdate to the last-input instant: the timeline records when input
        // actually stopped/resumed, not when the coarse poll noticed.
        let ts_ns = now_ts_ns().saturating_sub(idle_ms.saturating_mul(1_000_000));
        match edge {
            IdleEdge::Start => {
                self.idle = true;
                self.writer.write_logged(
                    ts_ns,
                    TimelineKind::IdleStart,
                    TimelineActor::Human,
                    None,
                    json!({
                        "idle_ms_at_detection": idle_ms,
                        "idle_timeout_ms": self.config.idle_timeout_ms,
                    }),
                );
            }
            IdleEdge::End => {
                self.idle = false;
                self.writer.write_logged(
                    ts_ns,
                    TimelineKind::IdleEnd,
                    TimelineActor::Human,
                    None,
                    json!({ "idle_ms_at_detection": idle_ms }),
                );
            }
        }
    }

    /// Poll-driven safety net: if a foreground change was missed (hook
    /// hiccup, transient-only event stream), the next idle tick re-syncs the
    /// recorded state to reality, so the timeline can never silently diverge
    /// for longer than one poll interval.
    fn reconcile_foreground(&mut self) {
        let context = match synapse_a11y::current_foreground_context() {
            Ok(context) => context,
            Err(error) => {
                tracing::debug!(
                    code = "TIMELINE_FOREGROUND_NONE",
                    detail = %error,
                    "no foreground window at reconcile tick"
                );
                return;
            }
        };
        if !matches!(synapse_a11y::is_window_visible(context.hwnd), Ok(true)) {
            return;
        }
        self.apply_foreground(&context, "poll");
    }

    fn write_session_end(&self, edge: &str) {
        self.writer.write_logged(
            now_ts_ns(),
            TimelineKind::SessionEnd,
            TimelineActor::Human,
            None,
            session_end_payload(&self.writer, edge),
        );
    }

    fn flush_interactions(&mut self) {
        self.interactions.flush(&self.writer);
    }
}

fn session_end_payload(writer: &TimelineWriter, edge: &str) -> serde_json::Value {
    json!({
        "pid": std::process::id(),
        "rows_written": writer.rows_written.load(Ordering::Relaxed),
        "write_failures": writer.write_failures.load(Ordering::Relaxed),
        "edge": edge,
    })
}

/// Outcome of a pause/resume control action, for tool readback (#843).
#[derive(Clone, Debug)]
pub struct RecorderControlOutcome {
    pub was_paused: bool,
    /// Whether a session boundary row was written (and flushed) for this
    /// transition. Re-pausing while paused / re-resuming while recording
    /// writes no row.
    pub boundary_row_written: bool,
    pub state: super::timeline_control::PersistedControlState,
}

/// Pause sequencing: boundary row while still recording, durable control row,
/// then the gate flips. A failure at any step propagates with the system left
/// in the last consistent state it reached.
fn pause_recording(
    writer: &TimelineWriter,
    paused_until_ns: Option<u64>,
    changed_by: &str,
) -> Result<RecorderControlOutcome> {
    let was_paused = writer.control.is_paused();
    let mut boundary_row_written = false;
    if !was_paused {
        writer
            .try_write(
                now_ts_ns(),
                TimelineKind::SessionEnd,
                TimelineActor::Human,
                None,
                json!({
                    "edge": "pause",
                    "by_session": changed_by,
                    "paused_until_ns": paused_until_ns,
                    "pid": std::process::id(),
                    "rows_written": writer.rows_written.load(Ordering::Relaxed),
                    "write_failures": writer.write_failures.load(Ordering::Relaxed),
                }),
            )
            .context("write session_end pause boundary row; recording is unchanged")?;
        writer
            .db
            .flush()
            .context("flush session_end pause boundary row; recording is unchanged")?;
        boundary_row_written = true;
    }
    let state =
        writer
            .control
            .persist_pause(&writer.db, paused_until_ns, now_ts_ns(), changed_by)?;
    tracing::info!(
        code = "TIMELINE_RECORDER_PAUSED",
        paused_until_ns,
        by_session = changed_by,
        "timeline recorder paused"
    );
    Ok(RecorderControlOutcome {
        was_paused,
        boundary_row_written,
        state,
    })
}

/// Resume sequencing: durable control row, the gate opens, then a
/// `session_start { edge: "resume" }` boundary row is written and flushed —
/// the resume-time proof that the write path works. A boundary failure is a
/// hard error: recording IS resumed at that point and the caller must know
/// the write path is broken.
fn resume_recording(writer: &TimelineWriter, changed_by: &str) -> Result<RecorderControlOutcome> {
    let was_paused = writer.control.is_paused();
    let state = writer
        .control
        .persist_resume(&writer.db, now_ts_ns(), changed_by)?;
    let mut boundary_row_written = false;
    if was_paused {
        writer
            .try_write(
                now_ts_ns(),
                TimelineKind::SessionStart,
                TimelineActor::Human,
                None,
                json!({
                    "edge": "resume",
                    "by_session": changed_by,
                    "pid": std::process::id(),
                }),
            )
            .context(
                "write session_start resume boundary row — recording IS resumed but the \
                 timeline write path is broken",
            )?;
        writer.db.flush().context(
            "flush session_start resume boundary row — recording IS resumed but the \
                 timeline write path is broken",
        )?;
        boundary_row_written = true;
        tracing::info!(
            code = "TIMELINE_RECORDER_RESUMED",
            by_session = changed_by,
            "timeline recorder resumed"
        );
    }
    Ok(RecorderControlOutcome {
        was_paused,
        boundary_row_written,
        state,
    })
}

async fn run_worker(
    mut receiver: mpsc::UnboundedReceiver<RecorderMessage>,
    mut state: WorkerState,
) {
    while let Some(message) = receiver.recv().await {
        match message {
            RecorderMessage::Accessible(event) => state.handle_accessible(&event),
            RecorderMessage::Interaction(event) => state.handle_interaction(&event),
            RecorderMessage::IdleProbe { idle_ms } => state.handle_idle_probe(idle_ms),
            RecorderMessage::FlushInteractions { done } => {
                state.flush_interactions();
                state.writer.flush_logged();
                let _ = done.send(());
            }
            RecorderMessage::Shutdown { done } => {
                state.flush_interactions();
                state.write_session_end("shutdown");
                state.writer.flush_logged();
                let _ = done.send(());
                tracing::info!(
                    code = "TIMELINE_RECORDER_STOPPED",
                    rows_written = state.writer.rows_written.load(Ordering::Relaxed),
                    write_failures = state.writer.write_failures.load(Ordering::Relaxed),
                    "activity recorder stopped"
                );
                return;
            }
        }
    }
    tracing::warn!(
        code = "TIMELINE_RECORDER_CHANNEL_CLOSED",
        "activity recorder channel closed without shutdown; session_end is written by the drop backstop"
    );
}

async fn run_idle_probe(sender: mpsc::UnboundedSender<RecorderMessage>, poll_interval_ms: u64) {
    let period = Duration::from_millis(poll_interval_ms.max(1));
    // First tick after one full period (not immediately): spawn already
    // probed the idle source, and the WinEvent path covers startup state.
    let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        match synapse_a11y::millis_since_last_input() {
            Ok(idle_ms) => {
                if sender.send(RecorderMessage::IdleProbe { idle_ms }).is_err() {
                    return;
                }
            }
            Err(error) => {
                tracing::error!(
                    code = "TIMELINE_IDLE_PROBE_FAILED",
                    detail = %error,
                    "idle probe failed; idle/active transitions are not being recorded this tick"
                );
            }
        }
    }
}

fn start_interaction_pipeline(
    recorder_sender: &mpsc::UnboundedSender<RecorderMessage>,
) -> Result<(InteractionHook, JoinHandle<()>)> {
    let (interaction_tx, mut interaction_rx) = mpsc::unbounded_channel();
    let hook = InteractionHook::start(interaction_tx)?;
    let recorder_sender = recorder_sender.clone();
    let bridge = tokio::spawn(async move {
        while let Some(event) = interaction_rx.recv().await {
            if recorder_sender
                .send(RecorderMessage::Interaction(event))
                .is_err()
            {
                return;
            }
        }
    });
    Ok((hook, bridge))
}

/// Always-on operator-activity recorder. One per daemon; owns the timeline
/// write path for foreground/title/idle/session rows.
pub struct ActivityRecorder {
    sender: mpsc::UnboundedSender<RecorderMessage>,
    writer: TimelineWriter,
    config: RecorderConfig,
    last_clipboard_sha256: Mutex<Option<String>>,
    browser_nav_dedupe_keys: Mutex<VecDeque<String>>,
    shutdown_requested: AtomicBool,
    sink_closed_logged: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
    idle_probe: Mutex<Option<JoinHandle<()>>>,
    interaction_hook: Mutex<Option<InteractionHook>>,
    interaction_bridge: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for ActivityRecorder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActivityRecorder")
            .field("config", &self.config)
            .field(
                "rows_written",
                &self.writer.rows_written.load(Ordering::Relaxed),
            )
            .field(
                "write_failures",
                &self.writer.write_failures.load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

impl ActivityRecorder {
    /// Starts the recorder: probes the idle source once (fail-fast on a
    /// platform where idle tracking cannot work), writes the `session_start`
    /// row synchronously (fail-fast on a broken write path), then spawns the
    /// event worker and the idle-poll task.
    ///
    /// # Errors
    ///
    /// Returns an error when the idle probe or the `session_start` write
    /// fails; the daemon must refuse to start with a recorder that cannot
    /// record. A recorder hydrated into the paused state (#843) writes no
    /// `session_start` — paused means zero rows — unless its auto-resume
    /// deadline already passed while the daemon was down, in which case it
    /// resumes immediately.
    pub fn spawn(
        db: Arc<Db>,
        config: RecorderConfig,
        control: Arc<RecorderControl>,
    ) -> Result<Self> {
        let initial_idle_ms = synapse_a11y::millis_since_last_input()
            .context("probe GetLastInputInfo for the activity recorder idle source")?;
        if control.auto_resume_due(now_ts_ns()) {
            control
                .persist_resume(&db, now_ts_ns(), "startup_auto_resume")
                .context("auto-resume expired timeline pause at recorder startup")?;
            tracing::info!(
                code = "TIMELINE_RECORDER_AUTO_RESUMED",
                "timeline pause deadline passed while the daemon was down; resuming at startup"
            );
        }
        let writer = TimelineWriter {
            db,
            control,
            seq: Arc::new(AtomicU32::new(0)),
            rows_written: Arc::new(AtomicU64::new(0)),
            write_failures: Arc::new(AtomicU64::new(0)),
            rows_suppressed_paused: Arc::new(AtomicU64::new(0)),
            rows_suppressed_excluded: Arc::new(AtomicU64::new(0)),
        };
        if writer.control.is_paused() {
            tracing::info!(
                code = "TIMELINE_RECORDER_STARTED_PAUSED",
                paused_until_ns = writer.control.paused_until_ns(),
                "activity recorder started in the persisted paused state; no rows until resume"
            );
        } else {
            writer
                .try_write(
                    now_ts_ns(),
                    TimelineKind::SessionStart,
                    TimelineActor::Human,
                    None,
                    json!({
                        "edge": "startup",
                        "pid": std::process::id(),
                        "idle_timeout_ms": config.idle_timeout_ms,
                        "idle_poll_interval_ms": config.idle_poll_interval_ms,
                        "initial_idle_ms": initial_idle_ms,
                    }),
                )
                .context("write CF_TIMELINE session_start row at recorder startup")?;
            // The batcher acks on enqueue; flush so a broken write path fails
            // the daemon at startup instead of surfacing 100 ms later in a log.
            writer
                .db
                .flush()
                .context("flush CF_TIMELINE session_start row at recorder startup")?;
        }

        let (sender, receiver) = mpsc::unbounded_channel();
        let state = WorkerState {
            writer: writer.clone(),
            config,
            foreground: None,
            idle: false,
            interactions: InteractionAccumulator::default(),
        };
        let worker = tokio::spawn(run_worker(receiver, state));
        let idle_probe = tokio::spawn(run_idle_probe(sender.clone(), config.idle_poll_interval_ms));
        let (interaction_hook, interaction_bridge) = if writer.control.is_paused() {
            (None, None)
        } else {
            let (hook, bridge) = start_interaction_pipeline(&sender)
                .context("start counts-only interaction cadence hook")?;
            (Some(hook), Some(bridge))
        };
        tracing::info!(
            code = "TIMELINE_RECORDER_STARTED",
            idle_timeout_ms = config.idle_timeout_ms,
            idle_poll_interval_ms = config.idle_poll_interval_ms,
            initial_idle_ms,
            interaction_hook_thread_id = interaction_hook
                .as_ref()
                .map(|hook| hook.readback().thread_id)
                .unwrap_or(0),
            "activity recorder started"
        );
        Ok(Self {
            sender,
            writer,
            config,
            last_clipboard_sha256: Mutex::new(None),
            browser_nav_dedupe_keys: Mutex::new(VecDeque::new()),
            shutdown_requested: AtomicBool::new(false),
            sink_closed_logged: AtomicBool::new(false),
            worker: Mutex::new(Some(worker)),
            idle_probe: Mutex::new(Some(idle_probe)),
            interaction_hook: Mutex::new(interaction_hook),
            interaction_bridge: Mutex::new(interaction_bridge),
        })
    }

    /// Cheap, non-blocking sink for the WinEvent bridge. Irrelevant kinds are
    /// filtered before crossing the channel.
    pub fn record_accessible_event(&self, event: &AccessibleEvent) {
        if !matches!(
            event.kind,
            AccessibleEventKind::ForegroundChanged | AccessibleEventKind::NameChanged
        ) {
            return;
        }
        if self
            .sender
            .send(RecorderMessage::Accessible(event.clone()))
            .is_err()
            && !self.sink_closed_logged.swap(true, Ordering::Relaxed)
        {
            tracing::error!(
                code = "TIMELINE_RECORDER_DOWN",
                "activity recorder worker is gone; foreground timeline rows are no longer recorded"
            );
        }
    }

    /// Records observation-derived enrichment feeds (#839): plaintext
    /// clipboard snippets and full file-activity paths in `CF_TIMELINE`.
    ///
    /// Observation/audit CFs stay redacted; this method writes only the
    /// operator-decided plaintext timeline rows.
    pub fn record_observation_enrichment(
        &self,
        observation: &Observation,
        clipboard: Option<&ClipboardTimelineSample>,
        fs_events: &[FsTimelineEvent],
    ) {
        let mut wrote_any = false;
        if let Some(sample) = clipboard {
            wrote_any |= self.record_clipboard_sample(observation, sample);
        }
        for event in fs_events {
            wrote_any |= self.record_file_activity(observation, event);
        }
        if wrote_any {
            self.writer.flush_logged();
        }
    }

    pub fn record_browser_navigation(&self, event: BrowserNavigationEvent) -> bool {
        let url = event.url.trim();
        if url.is_empty() {
            tracing::warn!(
                code = "TIMELINE_BROWSER_NAV_EMPTY_URL",
                source = %event.source,
                "skipping browser navigation timeline row with empty URL"
            );
            return false;
        }
        let app = event
            .app
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| Some("chrome.exe".to_owned()));
        if self
            .writer
            .suppressed(TimelineKind::BrowserNav, app.as_deref())
        {
            return false;
        }
        let dedupe_key = browser_nav_dedupe_key(&event);
        if self.browser_nav_seen(&dedupe_key) {
            return false;
        }
        let payload = json!({
            "url": url,
            "title": event.title.as_str(),
            "tab_id": event.tab_id,
            "chrome_window_id": event.chrome_window_id,
            "window_hwnd": event.window_hwnd,
            "cdp_target_id": event.cdp_target_id.as_deref(),
            "endpoint": event.endpoint.as_deref(),
            "transport": event.transport.as_deref(),
            "source": event.source.as_str(),
            "event": event.event.as_str(),
            "action": event.action.as_deref(),
            "requested_url": event.requested_url.as_deref(),
            "before_url": event.before_url.as_deref(),
            "before_title": event.before_title.as_deref(),
            "ready_state": event.ready_state.as_deref(),
            "observed_at_unix_ms": event.observed_at_unix_ms,
            "active": event.active,
            "highlighted": event.highlighted,
            "pinned": event.pinned,
        });
        match self.writer.try_write(
            now_ts_ns(),
            TimelineKind::BrowserNav,
            event.actor,
            app,
            payload,
        ) {
            Ok(()) => {
                self.remember_browser_nav_key(dedupe_key);
                self.writer.flush_logged();
                true
            }
            Err(error) => {
                self.writer.write_failures.fetch_add(1, Ordering::Relaxed);
                tracing::error!(
                    code = "TIMELINE_BROWSER_NAV_WRITE_FAILED",
                    detail = %format!("{error:#}"),
                    "failed to persist browser navigation timeline row"
                );
                false
            }
        }
    }

    /// Graceful stop: drains the worker, writes `session_end`, and stops the
    /// idle probe. Idempotent.
    pub async fn shutdown(&self) {
        if self.shutdown_requested.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(probe) = self.take_task(&self.idle_probe) {
            probe.abort();
        }
        self.stop_interaction_hook();
        let (done_tx, done_rx) = oneshot::channel();
        if self
            .sender
            .send(RecorderMessage::Shutdown { done: done_tx })
            .is_err()
        {
            tracing::error!(
                code = "TIMELINE_RECORDER_SHUTDOWN_WORKER_GONE",
                "activity recorder worker was already gone at shutdown; writing session_end directly"
            );
            self.write_session_end_direct("shutdown_worker_gone");
            return;
        }
        match tokio::time::timeout(SHUTDOWN_ACK_TIMEOUT, done_rx).await {
            Ok(Ok(())) => {
                if let Some(worker) = self.take_task(&self.worker) {
                    let _ = worker.await;
                }
            }
            _ => {
                tracing::error!(
                    code = "TIMELINE_RECORDER_SHUTDOWN_TIMEOUT",
                    timeout_ms =
                        u64::try_from(SHUTDOWN_ACK_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
                    "activity recorder worker did not acknowledge shutdown; aborting it"
                );
                if let Some(worker) = self.take_task(&self.worker) {
                    worker.abort();
                }
                self.write_session_end_direct("shutdown_timeout");
            }
        }
    }

    fn record_clipboard_sample(
        &self,
        observation: &Observation,
        sample: &ClipboardTimelineSample,
    ) -> bool {
        if !timeline_clipboard_enabled() {
            return false;
        }
        if self.writer.suppressed(
            TimelineKind::Clipboard,
            Some(&observation.foreground.process_name),
        ) {
            return false;
        }
        let mut last = match self.last_clipboard_sha256.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if last.as_deref() == Some(sample.text_sha256.as_str()) {
            return false;
        }
        let payload = json!({
            "snippet": sample.snippet.as_str(),
            "text_len": sample.text_len,
            "text_sha256": sample.text_sha256.as_str(),
            "formats": &sample.formats,
            "source_app": observation.foreground.process_name.as_str(),
            "source_process_path": observation.foreground.process_path.as_str(),
            "source_pid": observation.foreground.pid,
            "source_hwnd": observation.foreground.hwnd,
            "source_window_title": observation.foreground.window_title.as_str(),
            "observation_seq": observation.seq,
        });
        match self.writer.try_write(
            now_ts_ns(),
            TimelineKind::Clipboard,
            current_actor(),
            Some(observation.foreground.process_name.clone()),
            payload,
        ) {
            Ok(()) => {
                *last = Some(sample.text_sha256.clone());
                true
            }
            Err(error) => {
                self.writer.write_failures.fetch_add(1, Ordering::Relaxed);
                tracing::error!(
                    code = "TIMELINE_CLIPBOARD_WRITE_FAILED",
                    detail = %format!("{error:#}"),
                    "failed to persist clipboard timeline row"
                );
                false
            }
        }
    }

    fn browser_nav_seen(&self, key: &str) -> bool {
        let guard = match self.browser_nav_dedupe_keys.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.iter().any(|existing| existing == key)
    }

    fn remember_browser_nav_key(&self, key: String) {
        let mut guard = match self.browser_nav_dedupe_keys.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.iter().any(|existing| existing == &key) {
            return;
        }
        guard.push_back(key);
        while guard.len() > MAX_BROWSER_NAV_DEDUPE_KEYS {
            let _ = guard.pop_front();
        }
    }

    fn record_file_activity(&self, observation: &Observation, event: &FsTimelineEvent) -> bool {
        if !timeline_file_activity_enabled() {
            return false;
        }
        if self.writer.suppressed(
            TimelineKind::FileActivity,
            Some(&observation.foreground.process_name),
        ) {
            return false;
        }
        let payload = json!({
            "path": event.path.as_str(),
            "event_kind": fs_event_kind_name(event.kind),
            "size_bytes": event.size_bytes,
            "observed_at": event.at.to_rfc3339(),
            "source_app": observation.foreground.process_name.as_str(),
            "source_process_path": observation.foreground.process_path.as_str(),
            "source_pid": observation.foreground.pid,
            "source_hwnd": observation.foreground.hwnd,
            "source_window_title": observation.foreground.window_title.as_str(),
            "observation_seq": observation.seq,
        });
        match self.writer.try_write(
            now_ts_ns(),
            TimelineKind::FileActivity,
            current_actor(),
            Some(observation.foreground.process_name.clone()),
            payload,
        ) {
            Ok(()) => true,
            Err(error) => {
                self.writer.write_failures.fetch_add(1, Ordering::Relaxed);
                tracing::error!(
                    code = "TIMELINE_FILE_ACTIVITY_WRITE_FAILED",
                    detail = %format!("{error:#}"),
                    path = %event.path,
                    "failed to persist file-activity timeline row"
                );
                false
            }
        }
    }

    /// Live counters for health/FSV readback.
    #[must_use]
    pub fn readback(&self) -> (u64, u64) {
        (
            self.writer.rows_written.load(Ordering::Relaxed),
            self.writer.write_failures.load(Ordering::Relaxed),
        )
    }

    /// Suppressed-row counters: `(paused, excluded)` (#843 FSV readback).
    #[must_use]
    pub fn suppressed_counters(&self) -> (u64, u64) {
        (
            self.writer.rows_suppressed_paused.load(Ordering::Relaxed),
            self.writer.rows_suppressed_excluded.load(Ordering::Relaxed),
        )
    }

    /// Pauses recording: boundary row, durable control state, gate closed.
    ///
    /// # Errors
    ///
    /// Returns an error when the boundary row or the durable control write
    /// fails; the error states exactly which step failed and what state the
    /// recorder was left in.
    pub fn pause(
        &self,
        paused_until_ns: Option<u64>,
        changed_by: &str,
    ) -> Result<RecorderControlOutcome> {
        self.flush_interactions_blocking();
        let outcome = pause_recording(&self.writer, paused_until_ns, changed_by)?;
        if !outcome.was_paused {
            self.stop_interaction_hook();
        }
        Ok(outcome)
    }

    /// Resumes recording: durable control state, gate open, boundary row.
    ///
    /// # Errors
    ///
    /// Returns an error when the durable control write fails (still paused)
    /// or when the boundary row fails (resumed, write path broken — the
    /// error says so explicitly).
    pub fn resume(&self, changed_by: &str) -> Result<RecorderControlOutcome> {
        let outcome = resume_recording(&self.writer, changed_by)?;
        if outcome.was_paused {
            self.start_interaction_hook()
                .context("timeline resumed but starting the interaction cadence hook failed")?;
        }
        Ok(outcome)
    }

    fn take_task(&self, slot: &Mutex<Option<JoinHandle<()>>>) -> Option<JoinHandle<()>> {
        match slot.lock() {
            Ok(mut guard) => guard.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        }
    }

    fn start_interaction_hook(&self) -> Result<()> {
        let mut guard = match self.interaction_hook.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.is_some() {
            return Ok(());
        }
        let (hook, bridge) = start_interaction_pipeline(&self.sender)
            .context("start counts-only interaction cadence hook")?;
        tracing::info!(
            code = "TIMELINE_INTERACTION_HOOK_STARTED",
            thread_id = hook.readback().thread_id,
            keyboard_hook_installed = hook.readback().keyboard_hook_installed,
            mouse_hook_installed = hook.readback().mouse_hook_installed,
            "interaction cadence hook started"
        );
        *guard = Some(hook);
        match self.interaction_bridge.lock() {
            Ok(mut bridge_guard) => *bridge_guard = Some(bridge),
            Err(poisoned) => *poisoned.into_inner() = Some(bridge),
        }
        Ok(())
    }

    fn stop_interaction_hook(&self) {
        let mut guard = match self.interaction_hook.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.take().is_some() {
            tracing::info!(
                code = "TIMELINE_INTERACTION_HOOK_STOPPED",
                "interaction cadence hook stopped"
            );
        }
        let bridge = match self.interaction_bridge.lock() {
            Ok(mut bridge_guard) => bridge_guard.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(bridge) = bridge {
            bridge.abort();
        }
    }

    fn flush_interactions_blocking(&self) {
        let (done_tx, mut done_rx) = oneshot::channel();
        if self
            .sender
            .send(RecorderMessage::FlushInteractions { done: done_tx })
            .is_err()
        {
            tracing::error!(
                code = "TIMELINE_INTERACTION_FLUSH_WORKER_GONE",
                "activity recorder worker is gone; interaction cadence bucket cannot be flushed"
            );
            return;
        }
        let deadline = Instant::now() + SHUTDOWN_ACK_TIMEOUT;
        loop {
            match done_rx.try_recv() {
                Ok(()) => return,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        tracing::error!(
                            code = "TIMELINE_INTERACTION_FLUSH_TIMEOUT",
                            "activity recorder did not acknowledge interaction cadence flush"
                        );
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    tracing::error!(
                        code = "TIMELINE_INTERACTION_FLUSH_ACK_DROPPED",
                        "activity recorder did not acknowledge interaction cadence flush"
                    );
                    return;
                }
            }
        }
    }

    fn write_session_end_direct(&self, edge: &str) {
        if let Err(error) = self.writer.try_write(
            now_ts_ns(),
            TimelineKind::SessionEnd,
            TimelineActor::Human,
            None,
            session_end_payload(&self.writer, edge),
        ) {
            self.writer.write_failures.fetch_add(1, Ordering::Relaxed);
            tracing::error!(
                code = "TIMELINE_WRITE_FAILED",
                kind = ?TimelineKind::SessionEnd,
                detail = %format!("{error:#}"),
                "failed to persist session_end row"
            );
        }
        self.writer.flush_logged();
    }
}

const fn fs_event_kind_name(kind: FsEventKind) -> &'static str {
    match kind {
        FsEventKind::Created => "created",
        FsEventKind::Modified => "modified",
        FsEventKind::Deleted => "deleted",
        FsEventKind::Renamed => "renamed",
    }
}

impl Drop for ActivityRecorder {
    fn drop(&mut self) {
        if let Some(probe) = self.take_task(&self.idle_probe) {
            probe.abort();
        }
        self.stop_interaction_hook();
        if let Some(worker) = self.take_task(&self.worker) {
            worker.abort();
        }
        // Backstop: an unwound daemon still closes the recorder session so
        // the timeline never shows a session_start without a matching end.
        if !self.shutdown_requested.swap(true, Ordering::SeqCst) {
            self.write_session_end_direct("drop");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synapse_core::types::Rect;

    fn snapshot(hwnd: i64, pid: u32, title: &str) -> ForegroundSnapshot {
        ForegroundSnapshot {
            hwnd,
            pid,
            process_name: "test.exe".to_owned(),
            process_path: r"C:\test.exe".to_owned(),
            title: title.to_owned(),
        }
    }

    fn foreground(
        hwnd: i64,
        pid: u32,
        process_name: &str,
        title: &str,
    ) -> synapse_core::ForegroundContext {
        synapse_core::ForegroundContext {
            hwnd,
            pid,
            process_name: process_name.to_owned(),
            process_path: format!(r"C:\Program Files\{process_name}"),
            window_title: title.to_owned(),
            window_bounds: Rect {
                x: 10,
                y: 20,
                w: 800,
                h: 600,
            },
            monitor_index: 0,
            dpi_scale: 1.0,
            profile_id: None,
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
        }
    }

    fn temp_writer() -> (tempfile::TempDir, TimelineWriter) {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        let db = Arc::new(
            Db::open(dir.path(), synapse_core::SCHEMA_VERSION)
                .unwrap_or_else(|error| panic!("open temp db: {error}")),
        );
        let control = Arc::new(
            RecorderControl::hydrate(&db).unwrap_or_else(|error| panic!("hydrate: {error:#}")),
        );
        let writer = TimelineWriter {
            db,
            control,
            seq: Arc::new(AtomicU32::new(0)),
            rows_written: Arc::new(AtomicU64::new(0)),
            write_failures: Arc::new(AtomicU64::new(0)),
            rows_suppressed_paused: Arc::new(AtomicU64::new(0)),
            rows_suppressed_excluded: Arc::new(AtomicU64::new(0)),
        };
        (dir, writer)
    }

    fn timeline_records(writer: &TimelineWriter) -> Vec<TimelineRecord> {
        writer
            .db
            .flush()
            .unwrap_or_else(|error| panic!("flush: {error}"));
        writer
            .db
            .scan_cf(cf::CF_TIMELINE)
            .unwrap_or_else(|error| panic!("scan: {error}"))
            .into_iter()
            .map(|(_key, value)| {
                serde_json::from_slice(&value).unwrap_or_else(|error| panic!("decode: {error}"))
            })
            .collect()
    }

    fn recorder_for_writer(writer: TimelineWriter) -> ActivityRecorder {
        let (sender, _receiver) = mpsc::unbounded_channel();
        ActivityRecorder {
            sender,
            writer,
            config: RecorderConfig {
                idle_timeout_ms: DEFAULT_IDLE_TIMEOUT_MS,
                idle_poll_interval_ms: MAX_IDLE_POLL_INTERVAL_MS,
            },
            last_clipboard_sha256: Mutex::new(None),
            browser_nav_dedupe_keys: Mutex::new(VecDeque::new()),
            shutdown_requested: AtomicBool::new(true),
            sink_closed_logged: AtomicBool::new(false),
            worker: Mutex::new(None),
            idle_probe: Mutex::new(None),
            interaction_hook: Mutex::new(None),
            interaction_bridge: Mutex::new(None),
        }
    }

    fn observation(context: synapse_core::ForegroundContext) -> Observation {
        Observation {
            seq: 839,
            at: Utc::now(),
            mode: synapse_core::PerceptionMode::A11yOnly,
            foreground: context,
            focused: None,
            elements: Vec::new(),
            entities: Vec::new(),
            hud: synapse_core::HudReadings::default(),
            audio: synapse_core::AudioContext::default(),
            recent_events: Vec::new(),
            clipboard_summary: None,
            fs_recent: Vec::new(),
            diagnostics: synapse_core::ObservationDiagnostics {
                assembled_in_ms: 0.0,
                sensor_latency_ms: std::collections::BTreeMap::new(),
                a11y_enabled: false,
                pixel_enabled: false,
                audio_enabled: false,
                a11y_status: synapse_core::SensorStatus::Healthy,
                capture_status: synapse_core::SensorStatus::Disabled,
                detection_status: synapse_core::SensorStatus::Disabled,
                audio_status: synapse_core::SensorStatus::Disabled,
                is_minimized: false,
                capture_config: None,
                capture_runtime: None,
                input_backends: None,
                cdp: None,
                web_path: None,
                elements_truncated: false,
                elements_page: None,
                entities_truncated: false,
                size_bytes: 0,
                size_estimate_tokens: 0,
            },
        }
    }

    #[test]
    fn config_defaults_match_activitywatch_prior_art() {
        let config = RecorderConfig::from_raw(None).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(config.idle_timeout_ms, 180_000);
        assert_eq!(config.idle_poll_interval_ms, 5_000);
    }

    #[test]
    fn config_short_timeout_derives_proportional_poll() {
        let config =
            RecorderConfig::from_raw(Some("2000")).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(config.idle_timeout_ms, 2_000);
        assert_eq!(config.idle_poll_interval_ms, 500);
    }

    #[test]
    fn config_rejects_zero_and_garbage() {
        assert!(
            RecorderConfig::from_raw(Some("0")).is_err(),
            "0 must be rejected"
        );
        assert!(
            RecorderConfig::from_raw(Some("fast")).is_err(),
            "non-numeric must be rejected"
        );
        assert!(
            RecorderConfig::from_raw(Some("")).is_err(),
            "empty string must be rejected"
        );
    }

    #[test]
    fn foreground_transitions_classify_switch_title_duplicate() {
        let first = snapshot(100, 7, "Inbox");
        assert_eq!(
            classify_foreground_transition(None, &first),
            ForegroundTransition::Switched,
            "first foreground must be a switch"
        );
        assert_eq!(
            classify_foreground_transition(Some(&first), &snapshot(100, 7, "Inbox")),
            ForegroundTransition::Duplicate,
            "identical foreground must not produce a row"
        );
        assert_eq!(
            classify_foreground_transition(Some(&first), &snapshot(100, 7, "Drafts")),
            ForegroundTransition::TitleChanged,
            "same window with new title is a title change"
        );
        assert_eq!(
            classify_foreground_transition(Some(&first), &snapshot(200, 7, "Inbox")),
            ForegroundTransition::Switched,
            "new hwnd is a switch even with identical title"
        );
        assert_eq!(
            classify_foreground_transition(Some(&first), &snapshot(100, 8, "Inbox")),
            ForegroundTransition::Switched,
            "hwnd reuse by a different pid is a switch"
        );
    }

    #[test]
    fn clipboard_enrichment_writes_plaintext_snippet_and_dedupes() {
        let (_dir, writer) = temp_writer();
        let recorder = recorder_for_writer(writer.clone());
        let observation = observation(foreground(
            83901,
            839,
            "notepad.exe",
            "issue839 clipboard source - Notepad",
        ));
        let sample = ClipboardTimelineSample {
            formats: vec!["text/plain".to_owned(), "text/unicode".to_owned()],
            text_len: 28,
            snippet: "issue839-plain-clipboard-row".to_owned(),
            text_sha256: "sha256:issue839clipboard".to_owned(),
        };

        recorder.record_observation_enrichment(&observation, Some(&sample), &[]);
        recorder.record_observation_enrichment(&observation, Some(&sample), &[]);
        let records = timeline_records(&writer);
        println!("readback=timeline_enrichment edge=clipboard after={records:?}");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, TimelineKind::Clipboard);
        assert_eq!(records[0].app.as_deref(), Some("notepad.exe"));
        assert_eq!(
            records[0].payload["snippet"],
            "issue839-plain-clipboard-row"
        );
        assert_eq!(records[0].payload["source_app"], "notepad.exe");
        assert_eq!(records[0].payload["observation_seq"], 839);
    }

    #[test]
    fn file_activity_enrichment_writes_full_path() {
        let (_dir, writer) = temp_writer();
        let recorder = recorder_for_writer(writer.clone());
        let observation = observation(foreground(
            83902,
            840,
            "notepad.exe",
            "issue839 file source - Notepad",
        ));
        let path = r"C:\Users\hotra\Documents\issue839-known-save.txt";
        let event = FsTimelineEvent {
            at: Utc::now(),
            path: path.to_owned(),
            kind: FsEventKind::Modified,
            size_bytes: Some(42),
        };

        recorder.record_observation_enrichment(&observation, None, &[event]);
        let records = timeline_records(&writer);
        println!("readback=timeline_enrichment edge=file_activity after={records:?}");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, TimelineKind::FileActivity);
        assert_eq!(records[0].app.as_deref(), Some("notepad.exe"));
        assert_eq!(records[0].payload["path"], path);
        assert_eq!(records[0].payload["event_kind"], "modified");
        assert_eq!(records[0].payload["size_bytes"], 42);
    }

    #[test]
    fn browser_navigation_writes_url_title_tab_id_and_dedupes() {
        let (_dir, writer) = temp_writer();
        let recorder = recorder_for_writer(writer.clone());
        let event = BrowserNavigationEvent {
            actor: TimelineActor::Human,
            app: Some("chrome.exe".to_owned()),
            source: "tabs.onUpdated".to_owned(),
            event: "tabNavigation".to_owned(),
            action: None,
            url: "https://example.com/issue840".to_owned(),
            title: "Issue 840 Example".to_owned(),
            tab_id: Some(84001),
            chrome_window_id: Some(11),
            window_hwnd: None,
            cdp_target_id: Some("chrome-tab:84001".to_owned()),
            endpoint: Some("chrome-extension://leoocgnkjnplbfdbklajepahofecgfbk".to_owned()),
            transport: Some("direct_http".to_owned()),
            requested_url: None,
            before_url: None,
            before_title: None,
            ready_state: Some("complete".to_owned()),
            observed_at_unix_ms: Some(1_781_280_000_000),
            active: Some(true),
            highlighted: Some(true),
            pinned: Some(false),
        };

        assert!(recorder.record_browser_navigation(event.clone()));
        assert!(
            !recorder.record_browser_navigation(event),
            "duplicate browser nav rows should be suppressed"
        );
        let records = timeline_records(&writer);
        println!("readback=browser_nav edge=human_tab_event after={records:?}");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, TimelineKind::BrowserNav);
        assert_eq!(records[0].actor, TimelineActor::Human);
        assert_eq!(records[0].app.as_deref(), Some("chrome.exe"));
        assert_eq!(records[0].payload["url"], "https://example.com/issue840");
        assert_eq!(records[0].payload["title"], "Issue 840 Example");
        assert_eq!(records[0].payload["tab_id"], 84001);
        assert_eq!(records[0].payload["ready_state"], "complete");
    }

    #[test]
    fn browser_navigation_agent_event_keeps_session_id() {
        let (_dir, writer) = temp_writer();
        let recorder = recorder_for_writer(writer.clone());
        let actor = TimelineActor::Agent {
            session_id: "issue840-session".to_owned(),
        };

        assert!(recorder.record_browser_navigation(BrowserNavigationEvent {
            actor: actor.clone(),
            app: Some("chrome.exe".to_owned()),
            source: "cdp_navigate_tab".to_owned(),
            event: "tool_call".to_owned(),
            action: Some("navigate".to_owned()),
            url: "data:text/html,<title>Issue840Agent</title>".to_owned(),
            title: "Issue840Agent".to_owned(),
            tab_id: Some(84002),
            chrome_window_id: None,
            window_hwnd: Some(0x840),
            cdp_target_id: Some("chrome-tab:84002".to_owned()),
            endpoint: Some("chrome-extension://leoocgnkjnplbfdbklajepahofecgfbk".to_owned()),
            transport: Some("chrome_tabs_extension".to_owned()),
            requested_url: Some("data:text/html,<title>Issue840Agent</title>".to_owned()),
            before_url: Some("about:blank".to_owned()),
            before_title: Some(String::new()),
            ready_state: Some("complete".to_owned()),
            observed_at_unix_ms: None,
            active: None,
            highlighted: None,
            pinned: None,
        }));
        let records = timeline_records(&writer);
        println!("readback=browser_nav edge=agent_cdp after={records:?}");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, TimelineKind::BrowserNav);
        assert_eq!(records[0].actor, actor);
        assert_eq!(records[0].payload["action"], "navigate");
        assert_eq!(records[0].payload["window_hwnd"], 0x840);
    }

    #[test]
    fn interaction_summary_counts_only_and_hashes_title() {
        let (_dir, writer) = temp_writer();
        let mut accumulator = InteractionAccumulator::default();
        let context = foreground(100, 7, "notepad.exe", "Private Draft - Notepad");
        accumulator.record_input(
            &InteractionEvent {
                ts_ns: 30_000_000_001,
                kind: InteractionEventKind::Keystroke,
                injected: false,
            },
            &context,
            TimelineActor::Human,
            "physical",
            &writer,
        );
        accumulator.record_input(
            &InteractionEvent {
                ts_ns: 30_000_000_002,
                kind: InteractionEventKind::Click,
                injected: false,
            },
            &context,
            TimelineActor::Human,
            "physical",
            &writer,
        );
        accumulator.record_input(
            &InteractionEvent {
                ts_ns: 30_000_000_003,
                kind: InteractionEventKind::VerticalScroll { delta: -120 },
                injected: false,
            },
            &context,
            TimelineActor::Human,
            "physical",
            &writer,
        );
        println!(
            "readback=interaction_summary edge=counts_only before_rows={}",
            timeline_records(&writer).len()
        );
        accumulator.flush(&writer);
        let records = timeline_records(&writer);
        println!("readback=interaction_summary edge=counts_only after={records:?}");
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.kind, TimelineKind::InteractionSummary);
        assert_eq!(record.actor, TimelineActor::Human);
        assert_eq!(record.app.as_deref(), Some("notepad.exe"));
        assert_eq!(record.payload["keystroke_count"], 1);
        assert_eq!(record.payload["click_count"], 1);
        assert_eq!(record.payload["scroll_vertical_delta"], -120);
        assert_eq!(record.payload["input_origin"], "physical");
        assert_eq!(
            record.payload["window_title_sha256"],
            sha256_hex("Private Draft - Notepad")
        );
        assert!(
            record.payload.get("title").is_none(),
            "interaction summaries must not store raw window titles"
        );
        assert!(
            record.payload.get("key").is_none(),
            "interaction summaries must not store raw key names"
        );
    }

    #[test]
    fn injected_interactions_are_agent_tagged_not_human() {
        let (_dir, writer) = temp_writer();
        let mut accumulator = InteractionAccumulator::default();
        let context = foreground(200, 9, "chrome.exe", "Form - Chrome");
        let actor = TimelineActor::Agent {
            session_id: INJECTED_UNATTRIBUTED_SESSION_ID.to_owned(),
        };
        accumulator.record_input(
            &InteractionEvent {
                ts_ns: 60_000_000_001,
                kind: InteractionEventKind::Keystroke,
                injected: true,
            },
            &context,
            actor.clone(),
            "injected",
            &writer,
        );
        accumulator.flush(&writer);
        let records = timeline_records(&writer);
        println!("readback=interaction_summary edge=injected after={records:?}");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, TimelineKind::InteractionSummary);
        assert_eq!(records[0].actor, actor);
        assert_ne!(records[0].actor, TimelineActor::Human);
        assert_eq!(records[0].payload["input_origin"], "injected");
        assert_eq!(records[0].payload["keystroke_count"], 1);
    }

    #[test]
    fn app_switches_create_summary_rows_without_input_content() {
        let (_dir, writer) = temp_writer();
        let mut accumulator = InteractionAccumulator::default();
        let first = foreground(100, 7, "code.exe", "Repo - Code");
        let second = foreground(200, 8, "notepad.exe", "Notes - Notepad");
        accumulator.record_app_switch(90_000_000_001, &first, TimelineActor::Human, &writer);
        accumulator.record_app_switch(90_000_000_002, &second, TimelineActor::Human, &writer);
        accumulator.flush(&writer);
        let records = timeline_records(&writer);
        println!("readback=interaction_summary edge=app_switch after={records:?}");
        assert_eq!(records.len(), 2);
        assert!(
            records
                .iter()
                .all(|record| record.kind == TimelineKind::InteractionSummary)
        );
        assert!(
            records
                .iter()
                .all(|record| record.payload["app_switch_count"] == 1)
        );
        assert!(
            records
                .iter()
                .all(|record| record.payload.get("title").is_none())
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn recorder_writes_real_foreground_rows_into_cf_timeline() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_test_writer()
            .try_init();
        let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        let db = Arc::new(
            Db::open(temp.path(), synapse_core::SCHEMA_VERSION)
                .unwrap_or_else(|error| panic!("open temp db: {error}")),
        );
        let config = RecorderConfig::from_raw(Some("600000"))
            .unwrap_or_else(|error| panic!("config: {error}"));
        let control = Arc::new(
            crate::m3::timeline_control::RecorderControl::hydrate(&db)
                .unwrap_or_else(|error| panic!("hydrate control: {error:#}")),
        );
        let recorder = ActivityRecorder::spawn(Arc::clone(&db), config, control)
            .unwrap_or_else(|error| panic!("spawn recorder: {error}"));
        let (after_start, _failures) = recorder.readback();
        assert_eq!(
            after_start, 1,
            "session_start must be written synchronously"
        );

        // Real foreground window: the event the WinEvent hook would deliver.
        let context = synapse_a11y::current_foreground_context()
            .unwrap_or_else(|error| panic!("real foreground context: {error}"));
        let event = AccessibleEvent {
            seq: 1,
            at_ms: 1,
            window_id: context.hwnd,
            element_id: None,
            kind: AccessibleEventKind::ForegroundChanged,
            name: None,
            value: None,
        };
        println!(
            "readback=cf_timeline edge=real_foreground before=rows:{} foreground:{}",
            recorder.readback().0,
            context.process_name
        );
        recorder.record_accessible_event(&event);
        wait_for_rows(&recorder, 2).await;

        // Edge: identical foreground event must not produce another row.
        recorder.record_accessible_event(&event);
        // Edge: a vanished/invalid event hwnd re-resolves to the real
        // foreground (already recorded), so it dedups to no row — and must
        // never crash.
        let vanished = AccessibleEvent {
            window_id: 0x000d_ead0,
            ..event.clone()
        };
        recorder.record_accessible_event(&vanished);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            recorder.readback().0,
            2,
            "duplicate and vanished-window events must not write rows"
        );

        // Agent attribution: while a session holds the real-input lease, a
        // foreground change must be tagged agent{session_id}. Uses the real
        // lease registry and a second real visible window.
        let other_window = synapse_a11y::visible_top_level_window_contexts()
            .unwrap_or_else(|error| panic!("enumerate windows: {error}"))
            .into_iter()
            .find(|candidate| candidate.hwnd != context.hwnd);
        if let Some(other) = other_window {
            let lease_session = format!("fsv-agent-{}", std::process::id());
            // The lease registry is process-global and other tests in this
            // binary exercise it; retry briefly instead of flaking on overlap.
            let acquire_deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let outcome =
                    synapse_action::lease::try_acquire(&lease_session, Duration::from_secs(30));
                match outcome {
                    synapse_action::LeaseOutcome::Acquired(_)
                    | synapse_action::LeaseOutcome::Renewed(_) => break,
                    other => {
                        assert!(
                            std::time::Instant::now() < acquire_deadline,
                            "real-input lease must be acquirable for the attribution edge: {other:?}"
                        );
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
            println!(
                "readback=cf_timeline edge=agent_attribution before=lease_held_by:{lease_session} window:{}",
                other.process_name
            );
            let agent_event = AccessibleEvent {
                window_id: other.hwnd,
                ..event.clone()
            };
            recorder.record_accessible_event(&agent_event);
            wait_for_rows(&recorder, 3).await;
            synapse_action::lease::release(&lease_session)
                .unwrap_or_else(|error| panic!("release lease: {error:?}"));
        } else {
            panic!("attribution edge needs a second visible window; none found");
        }

        recorder.shutdown().await;
        println!(
            "readback=cf_timeline edge=post_shutdown counters={:?}",
            recorder.readback()
        );
        let rows = db
            .scan_cf(cf::CF_TIMELINE)
            .unwrap_or_else(|error| panic!("scan CF_TIMELINE: {error}"));
        println!(
            "readback=cf_timeline edge=real_foreground after=rows:{}",
            rows.len()
        );
        assert_eq!(
            rows.len(),
            4,
            "session_start + human focus_change + agent focus_change + session_end"
        );
        let records: Vec<TimelineRecord> = rows
            .iter()
            .map(|(key, value)| {
                if let Err(error) = synapse_storage::timeline::decode_timeline_key(key) {
                    panic!("decode key: {error}");
                }
                serde_json::from_slice(value)
                    .unwrap_or_else(|error| panic!("decode record: {error}"))
            })
            .collect();
        assert_eq!(records[0].kind, TimelineKind::SessionStart);
        assert_eq!(records[1].kind, TimelineKind::FocusChange);
        assert_eq!(records[2].kind, TimelineKind::FocusChange);
        assert_eq!(records[3].kind, TimelineKind::SessionEnd);
        assert_eq!(
            records[1].app.as_deref(),
            Some(context.process_name.as_str()),
            "focus_change row must carry the real foreground process"
        );
        assert_eq!(
            records[1].actor,
            TimelineActor::Human,
            "unleased foreground change must be attributed to the human"
        );
        let expected_session = format!("fsv-agent-{}", std::process::id());
        assert_eq!(
            records[2].actor,
            TimelineActor::Agent {
                session_id: expected_session
            },
            "leased foreground change must be attributed to the acting agent session"
        );
        assert!(
            records
                .windows(2)
                .all(|pair| pair[0].ts_ns <= pair[1].ts_ns),
            "rows must iterate in chronological order"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn pause_and_exclusion_gates_suppress_real_rows() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_test_writer()
            .try_init();
        let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        let db = Arc::new(
            Db::open(temp.path(), synapse_core::SCHEMA_VERSION)
                .unwrap_or_else(|error| panic!("open temp db: {error}")),
        );
        let config = RecorderConfig::from_raw(Some("600000"))
            .unwrap_or_else(|error| panic!("config: {error}"));
        let control = Arc::new(
            RecorderControl::hydrate(&db).unwrap_or_else(|error| panic!("hydrate: {error:#}")),
        );
        let recorder = ActivityRecorder::spawn(Arc::clone(&db), config, Arc::clone(&control))
            .unwrap_or_else(|error| panic!("spawn recorder: {error}"));
        assert_eq!(recorder.readback().0, 1, "session_start");

        let context = synapse_a11y::current_foreground_context()
            .unwrap_or_else(|error| panic!("real foreground context: {error}"));
        let event = AccessibleEvent {
            seq: 1,
            at_ms: 1,
            window_id: context.hwnd,
            element_id: None,
            kind: AccessibleEventKind::ForegroundChanged,
            name: None,
            value: None,
        };

        // Pause: boundary row written while still recording, then silence.
        println!(
            "readback=cf_timeline edge=pause before=rows:{}",
            recorder.readback().0
        );
        let outcome = recorder
            .pause(None, "fsv-pause")
            .unwrap_or_else(|error| panic!("pause: {error:#}"));
        assert!(!outcome.was_paused);
        assert!(outcome.boundary_row_written);
        assert_eq!(recorder.readback().0, 2, "session_end{{edge=pause}}");
        recorder.record_accessible_event(&event);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            recorder.readback().0,
            2,
            "paused recorder must write zero rows for real events"
        );
        assert!(
            recorder.suppressed_counters().0 >= 1,
            "paused suppression must be counted: {:?}",
            recorder.suppressed_counters()
        );
        // Re-pause is honest about being a no-op.
        let again = recorder
            .pause(None, "fsv-pause")
            .unwrap_or_else(|error| panic!("re-pause: {error:#}"));
        assert!(again.was_paused);
        assert!(!again.boundary_row_written);

        // Resume: boundary row proves the write path, recording restarts.
        let resumed = recorder
            .resume("fsv-pause")
            .unwrap_or_else(|error| panic!("resume: {error:#}"));
        assert!(resumed.was_paused);
        assert!(resumed.boundary_row_written);
        assert_eq!(recorder.readback().0, 3, "session_start{{edge=resume}}");
        recorder.record_accessible_event(&event);
        wait_for_rows(&recorder, 4).await;

        // Exclusion: the current foreground exe stops producing rows.
        control
            .persist_exclusion_update(
                &db,
                std::slice::from_ref(&context.process_name),
                &[],
                now_ts_ns(),
                "fsv-exclude",
            )
            .unwrap_or_else(|error| panic!("exclude: {error:#}"));
        println!(
            "readback=cf_timeline edge=excluded before=rows:{} app:{}",
            recorder.readback().0,
            context.process_name
        );
        let title_changed = AccessibleEvent {
            kind: AccessibleEventKind::NameChanged,
            ..event.clone()
        };
        recorder.record_accessible_event(&event);
        recorder.record_accessible_event(&title_changed);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            recorder.readback().0,
            4,
            "excluded process must write zero rows even while focused"
        );
        assert!(
            recorder.suppressed_counters().1 >= 1,
            "exclusion suppression must be counted: {:?}",
            recorder.suppressed_counters()
        );

        // Removing the exclusion restores recording for a different window.
        control
            .persist_exclusion_update(
                &db,
                &[],
                std::slice::from_ref(&context.process_name),
                now_ts_ns(),
                "fsv-exclude",
            )
            .unwrap_or_else(|error| panic!("un-exclude: {error:#}"));
        let other_window = synapse_a11y::visible_top_level_window_contexts()
            .unwrap_or_else(|error| panic!("enumerate windows: {error}"))
            .into_iter()
            .find(|candidate| {
                candidate.hwnd != context.hwnd && candidate.process_name != context.process_name
            });
        if let Some(other) = other_window {
            let other_event = AccessibleEvent {
                window_id: other.hwnd,
                ..event.clone()
            };
            recorder.record_accessible_event(&other_event);
            wait_for_rows(&recorder, 5).await;
        } else {
            println!("readback=cf_timeline edge=unexclude skipped=no_second_window");
        }

        recorder.shutdown().await;
        let rows = db
            .scan_cf(cf::CF_TIMELINE)
            .unwrap_or_else(|error| panic!("scan CF_TIMELINE: {error}"));
        let records: Vec<TimelineRecord> = rows
            .iter()
            .map(|(_key, value)| {
                serde_json::from_slice(value).unwrap_or_else(|error| panic!("decode: {error}"))
            })
            .collect();
        let kinds: Vec<TimelineKind> = records.iter().map(|record| record.kind).collect();
        println!("readback=cf_timeline edge=physical_sot kinds={kinds:?}");
        assert_eq!(records[0].kind, TimelineKind::SessionStart);
        assert_eq!(records[1].kind, TimelineKind::SessionEnd);
        assert_eq!(
            records[1].payload["edge"], "pause",
            "pause boundary row must carry edge=pause: {:?}",
            records[1].payload
        );
        assert_eq!(records[2].kind, TimelineKind::SessionStart);
        assert_eq!(
            records[2].payload["edge"], "resume",
            "resume boundary row must carry edge=resume: {:?}",
            records[2].payload
        );
        assert_eq!(records[3].kind, TimelineKind::FocusChange);
        assert_eq!(
            records.last().map(|record| record.kind),
            Some(TimelineKind::SessionEnd),
            "shutdown must close the session"
        );
        // The only NameChanged event ever sent arrived while the process was
        // excluded, so no title row may exist; and the excluded-window focus
        // events must not have added a second focus row for that process.
        assert!(
            !kinds.contains(&TimelineKind::TitleChange),
            "excluded-window title event must not produce a row: {records:?}"
        );
        let focus_rows_for_excluded = records
            .iter()
            .filter(|record| {
                record.kind == TimelineKind::FocusChange
                    && record.app.as_deref() == Some(context.process_name.as_str())
            })
            .count();
        assert_eq!(
            focus_rows_for_excluded, 1,
            "only the pre-exclusion focus row may exist for {}: {records:?}",
            context.process_name
        );
    }

    #[cfg(windows)]
    async fn wait_for_rows(recorder: &ActivityRecorder, want: u64) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if recorder.readback().0 >= want {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "recorder did not reach {want} rows in time; readback={:?}",
                recorder.readback()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[test]
    fn idle_transitions_fire_exactly_on_edges() {
        assert_eq!(
            idle_transition(false, 179_999, 180_000),
            None,
            "below threshold"
        );
        assert_eq!(
            idle_transition(false, 180_000, 180_000),
            Some(IdleEdge::Start),
            "threshold is inclusive"
        );
        assert_eq!(idle_transition(true, 200_000, 180_000), None, "still idle");
        assert_eq!(
            idle_transition(true, 1_000, 180_000),
            Some(IdleEdge::End),
            "input resumption ends idle"
        );
        assert_eq!(
            idle_transition(false, 0, 180_000),
            None,
            "active stays active"
        );
    }
}
