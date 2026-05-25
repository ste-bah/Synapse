use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use chrono::Utc;
use serde_json::json;
use synapse_action::ActionHandle;
use synapse_core::{
    Action, EventFilter, ReflexId, ReflexLifetime, ReflexState, ReflexStatus, SCHEMA_VERSION,
    StoredReflexAudit, error_codes,
};
use synapse_storage::Db;
use uuid::Uuid;

use crate::{
    EventBus, REFLEX_LIFETIME_EXPIRED_KIND, SubscriberHandle,
    error::{ReflexError, ReflexResult},
    kinds::{combo::ComboController, on_event::OnEventState},
    write_audit,
};
use scheduler_tick::tick;

pub const MAX_SCHEDULED_REFLEXES: usize = 32;
pub const MAX_REFLEX_PRIORITY: u32 = 1000;
pub const REFLEX_TICK_LATE_KIND: &str = "reflex_tick_late";
pub const DEFAULT_SAMPLE_LIMIT: usize = 4096;
pub const DEFAULT_REFLEX_PRIORITY: u32 = 100;

#[derive(Clone, Debug)]
pub struct SchedulerConfig {
    pub target_interval: Duration,
    pub fallback_interval: Duration,
    pub late_after: Duration,
    pub sample_limit: usize,
    pub max_ticks: Option<u64>,
    pub force_degraded: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        let target_interval = Duration::from_millis(1);
        Self {
            target_interval,
            fallback_interval: Duration::from_millis(2),
            late_after: target_interval.saturating_mul(2),
            sample_limit: DEFAULT_SAMPLE_LIMIT,
            max_ticks: None,
            force_degraded: false,
        }
    }
}

impl SchedulerConfig {
    #[must_use]
    pub const fn with_max_ticks(mut self, max_ticks: u64) -> Self {
        self.max_ticks = Some(max_ticks);
        self
    }

    fn validate(&self) -> ReflexResult<()> {
        if self.target_interval.is_zero() {
            return Err(ReflexError::ParamsInvalid {
                detail: "scheduler target interval must be non-zero".to_owned(),
            });
        }
        if self.fallback_interval.is_zero() {
            return Err(ReflexError::ParamsInvalid {
                detail: "scheduler fallback interval must be non-zero".to_owned(),
            });
        }
        if self.sample_limit == 0 {
            return Err(ReflexError::ParamsInvalid {
                detail: "scheduler sample limit must be non-zero".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ScheduledReflex {
    pub reflex_id: ReflexId,
    pub trigger: SchedulerTrigger,
    pub then: Vec<Action>,
    pub priority: u32,
    pub lifetime: ReflexLifetime,
    pub exclusive: bool,
    pub debounce: Duration,
}

impl ScheduledReflex {
    #[must_use]
    pub fn every_tick(reflex_id: impl Into<ReflexId>, then: Vec<Action>) -> Self {
        Self {
            reflex_id: reflex_id.into(),
            trigger: SchedulerTrigger::EveryTick,
            then,
            priority: DEFAULT_REFLEX_PRIORITY,
            lifetime: ReflexLifetime::UntilCancelled,
            exclusive: false,
            debounce: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn on_event(
        reflex_id: impl Into<ReflexId>,
        filter: EventFilter,
        then: Vec<Action>,
    ) -> Self {
        Self {
            reflex_id: reflex_id.into(),
            trigger: SchedulerTrigger::OnEvent(filter),
            then,
            priority: DEFAULT_REFLEX_PRIORITY,
            lifetime: ReflexLifetime::UntilCancelled,
            exclusive: false,
            debounce: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn on_event_with_debounce(
        reflex_id: impl Into<ReflexId>,
        filter: EventFilter,
        then: Vec<Action>,
        debounce: Duration,
    ) -> Self {
        Self {
            reflex_id: reflex_id.into(),
            trigger: SchedulerTrigger::OnEvent(filter),
            then,
            priority: DEFAULT_REFLEX_PRIORITY,
            lifetime: ReflexLifetime::UntilCancelled,
            exclusive: false,
            debounce,
        }
    }

    #[must_use]
    pub const fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub fn with_lifetime(mut self, lifetime: ReflexLifetime) -> Self {
        self.lifetime = lifetime;
        self
    }

    #[must_use]
    pub const fn with_exclusive(mut self, exclusive: bool) -> Self {
        self.exclusive = exclusive;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerTrigger {
    EveryTick,
    OnEvent(EventFilter),
}

impl SchedulerTrigger {
    fn validate(&self) -> ReflexResult<()> {
        match self {
            Self::EveryTick => Ok(()),
            Self::OnEvent(filter) => {
                filter
                    .validate()
                    .map_err(|error| ReflexError::FilterInvalid {
                        detail: error.to_string(),
                    })
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TickSample {
    pub tick_index: u64,
    pub elapsed_us: u64,
    pub jitter_us: u64,
    pub target_us: u64,
    pub pulled_events: usize,
    pub dispatched_actions: usize,
    pub late: bool,
    pub degraded: bool,
}

pub struct SchedulerHandle {
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
    samples: Arc<Mutex<VecDeque<TickSample>>>,
    controls: Arc<Mutex<Vec<ReflexControl>>>,
    statuses: Arc<Mutex<Vec<ReflexStatus>>>,
}

impl SchedulerHandle {
    #[must_use]
    pub fn samples(&self) -> Vec<TickSample> {
        lock_samples(&self.samples).iter().copied().collect()
    }

    #[must_use]
    pub fn wait_for_samples(&self, count: usize, timeout: Duration) -> Vec<TickSample> {
        let deadline = Instant::now() + timeout;
        loop {
            let samples = self.samples();
            if samples.len() >= count || Instant::now() >= deadline {
                return samples;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[must_use]
    pub fn statuses(&self) -> Vec<ReflexStatus> {
        lock_statuses(&self.statuses).clone()
    }

    #[must_use]
    pub fn set_priority(&self, reflex_id: &str, priority: u32) -> bool {
        let Some(index) = status_index(&self.statuses, reflex_id) else {
            return false;
        };
        if let Some(control) = lock_controls(&self.controls).get_mut(index) {
            control.priority = priority;
        }
        if let Some(status) = lock_statuses(&self.statuses).get_mut(index) {
            status.priority = priority;
        }
        true
    }

    #[must_use]
    pub fn cancel_reflex(&self, reflex_id: &str) -> bool {
        let Some(index) = status_index(&self.statuses, reflex_id) else {
            return false;
        };
        if let Some(control) = lock_controls(&self.controls).get_mut(index) {
            control.active = false;
        }
        if let Some(status) = lock_statuses(&self.statuses).get_mut(index) {
            status.state = ReflexState::Cancelled;
        }
        true
    }

    /// Stops the scheduler thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the scheduler thread panicked before joining.
    pub fn stop(&mut self) -> ReflexResult<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            join.join().map_err(|error| ReflexError::ParamsInvalid {
                detail: format!("scheduler thread panicked: {error:?}"),
            })?;
        }
        Ok(())
    }
}

impl Drop for SchedulerHandle {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub struct ReflexScheduler;

impl ReflexScheduler {
    /// Spawns the dedicated reflex scheduler thread.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid timing config, invalid reflex filters, reflex cap overflow,
    /// event-bus subscription failure, or scheduler thread spawn failure.
    pub fn spawn(
        event_bus: EventBus,
        action_handle: ActionHandle,
        reflexes: Vec<ScheduledReflex>,
        config: SchedulerConfig,
    ) -> ReflexResult<SchedulerHandle> {
        Self::spawn_inner(event_bus, action_handle, reflexes, config, None)
    }

    /// Spawns the scheduler and writes reflex audit rows into `audit_db`.
    ///
    /// # Errors
    ///
    /// Returns the same setup errors as [`Self::spawn`].
    pub fn spawn_with_audit_db(
        event_bus: EventBus,
        action_handle: ActionHandle,
        reflexes: Vec<ScheduledReflex>,
        config: SchedulerConfig,
        audit_db: Arc<Db>,
    ) -> ReflexResult<SchedulerHandle> {
        Self::spawn_inner(event_bus, action_handle, reflexes, config, Some(audit_db))
    }

    fn spawn_inner(
        event_bus: EventBus,
        action_handle: ActionHandle,
        reflexes: Vec<ScheduledReflex>,
        config: SchedulerConfig,
        audit_db: Option<Arc<Db>>,
    ) -> ReflexResult<SchedulerHandle> {
        config.validate()?;
        validate_reflexes(&reflexes)?;
        let subscription = event_bus
            .subscribe(EventFilter::All, Vec::new(), false)
            .map_err(|error| ReflexError::CapReached {
                detail: format!("scheduler event subscription failed: {error}"),
            })?;
        let stop = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(Mutex::new(VecDeque::with_capacity(config.sample_limit)));
        let registered_at = Utc::now();
        let statuses = Arc::new(Mutex::new(
            reflexes
                .iter()
                .map(|reflex| status_for_reflex(reflex, registered_at))
                .collect::<Vec<_>>(),
        ));
        let controls = Arc::new(Mutex::new(
            reflexes
                .iter()
                .map(|reflex| ReflexControl {
                    priority: reflex.priority,
                    active: true,
                })
                .collect::<Vec<_>>(),
        ));
        let reflexes = reflexes
            .into_iter()
            .enumerate()
            .map(|(registration_order, reflex)| RuntimeReflex {
                registration_order,
                reflex,
            })
            .collect::<Vec<_>>();
        let on_event_states = reflexes
            .iter()
            .map(|_| OnEventState::default())
            .collect::<Vec<_>>();
        let starvation_states = reflexes
            .iter()
            .map(|_| crate::conflict::StarvationState::default())
            .collect::<Vec<_>>();

        let runtime = RuntimeState {
            event_bus,
            action_handle,
            reflexes,
            active_combos: Vec::new(),
            on_event_states,
            starvation_states,
            subscription,
            stop: Arc::clone(&stop),
            samples: Arc::clone(&samples),
            controls: Arc::clone(&controls),
            statuses: Arc::clone(&statuses),
            config,
            audit_db,
            tick_index: 0,
        };

        let join = thread::Builder::new()
            .name("synapse-reflex-scheduler".to_owned())
            .spawn(move || run_scheduler_thread(runtime))
            .map_err(|error| ReflexError::ParamsInvalid {
                detail: format!("scheduler thread spawn failed: {error}"),
            })?;

        Ok(SchedulerHandle {
            stop,
            join: Some(join),
            samples,
            controls,
            statuses,
        })
    }
}

#[derive(Clone, Debug)]
struct RuntimeReflex {
    registration_order: usize,
    reflex: ScheduledReflex,
}

#[derive(Clone, Debug)]
struct ReflexControl {
    priority: u32,
    active: bool,
}

struct RuntimeState {
    event_bus: EventBus,
    action_handle: ActionHandle,
    reflexes: Vec<RuntimeReflex>,
    active_combos: Vec<ComboController>,
    on_event_states: Vec<OnEventState>,
    starvation_states: Vec<crate::conflict::StarvationState>,
    subscription: SubscriberHandle,
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<VecDeque<TickSample>>>,
    controls: Arc<Mutex<Vec<ReflexControl>>>,
    statuses: Arc<Mutex<Vec<ReflexStatus>>>,
    config: SchedulerConfig,
    audit_db: Option<Arc<Db>>,
    tick_index: u64,
}

pub(crate) fn validate_reflexes(reflexes: &[ScheduledReflex]) -> ReflexResult<()> {
    if reflexes.len() > MAX_SCHEDULED_REFLEXES {
        return Err(ReflexError::CapReached {
            detail: format!(
                "scheduler reflex cap {MAX_SCHEDULED_REFLEXES} exceeded by {}",
                reflexes.len()
            ),
        });
    }
    for reflex in reflexes {
        if reflex.priority > MAX_REFLEX_PRIORITY {
            return Err(ReflexError::PriorityInvalid {
                detail: format!(
                    "priority {} exceeds maximum {MAX_REFLEX_PRIORITY}",
                    reflex.priority
                ),
            });
        }
        reflex.trigger.validate()?;
    }
    Ok(())
}

#[cfg(windows)]
fn run_scheduler_thread(mut runtime: RuntimeState) {
    if runtime.config.force_degraded {
        run_degraded(runtime, "forced_degraded_config");
        return;
    }

    match windows_timer::WindowsHighResolutionTimer::start(runtime.config.target_interval) {
        Ok(timer) => {
            let mut last = Instant::now();
            while should_tick(&runtime) {
                let deadline = last + runtime.config.target_interval;
                if let Err(error) = timer.wait_until(deadline) {
                    run_degraded(runtime, &error);
                    return;
                }
                let now = Instant::now();
                let elapsed = now.duration_since(last);
                last = now;
                tick(&mut runtime, elapsed, false);
            }
        }
        Err(error) => run_degraded(runtime, &error),
    }
}

#[cfg(not(windows))]
fn run_scheduler_thread(runtime: RuntimeState) {
    run_degraded(
        runtime,
        "high-resolution waitable timer is only available on Windows",
    );
}

fn run_degraded(mut runtime: RuntimeState, reason: &str) {
    tracing::warn!(
        component = "reflex_scheduler",
        degraded = true,
        reason = %reason,
        "reflex scheduler falling back to tokio interval"
    );
    let Ok(tokio_runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    else {
        tracing::error!(
            component = "reflex_scheduler",
            degraded = true,
            "reflex scheduler could not build fallback tokio runtime"
        );
        return;
    };
    tokio_runtime.block_on(async move {
        let mut interval = tokio::time::interval(runtime.config.fallback_interval);
        interval.tick().await;
        let mut last = Instant::now();
        while should_tick(&runtime) {
            interval.tick().await;
            let now = Instant::now();
            let elapsed = now.duration_since(last);
            last = now;
            tick(&mut runtime, elapsed, true);
        }
    });
}

fn should_tick(runtime: &RuntimeState) -> bool {
    if runtime.stop.load(Ordering::Acquire) {
        return false;
    }
    runtime
        .config
        .max_ticks
        .is_none_or(|max_ticks| runtime.tick_index < max_ticks)
}

fn lock_samples(
    samples: &Arc<Mutex<VecDeque<TickSample>>>,
) -> std::sync::MutexGuard<'_, VecDeque<TickSample>> {
    match samples.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn lock_controls(
    controls: &Arc<Mutex<Vec<ReflexControl>>>,
) -> std::sync::MutexGuard<'_, Vec<ReflexControl>> {
    match controls.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn lock_statuses(
    statuses: &Arc<Mutex<Vec<ReflexStatus>>>,
) -> std::sync::MutexGuard<'_, Vec<ReflexStatus>> {
    match statuses.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn status_index(statuses: &Arc<Mutex<Vec<ReflexStatus>>>, reflex_id: &str) -> Option<usize> {
    lock_statuses(statuses)
        .iter()
        .position(|status| status.id == reflex_id)
}

fn status_for_reflex(
    reflex: &ScheduledReflex,
    registered_at: chrono::DateTime<Utc>,
) -> ReflexStatus {
    ReflexStatus {
        id: reflex.reflex_id.clone(),
        kind_summary: kind_summary(reflex),
        state: ReflexState::Active,
        registered_at,
        last_fired_at: None,
        fire_count: 0,
        priority: reflex.priority,
        lifetime: reflex.lifetime.clone(),
        exclusive: reflex.exclusive,
        last_error_code: None,
    }
}

fn kind_summary(reflex: &ScheduledReflex) -> String {
    match &reflex.trigger {
        SchedulerTrigger::EveryTick => format!("every_tick:{} actions", reflex.then.len()),
        SchedulerTrigger::OnEvent(_filter) => format!("on_event:{} actions", reflex.then.len()),
    }
}

fn mark_reflex_fired(runtime: &RuntimeState, index: usize) {
    let expired = runtime
        .reflexes
        .get(index)
        .is_some_and(|reflex| matches!(reflex.reflex.lifetime, ReflexLifetime::OneShot));
    if expired && let Some(control) = lock_controls(&runtime.controls).get_mut(index) {
        control.active = false;
    }
    let mut expired_status = None;
    if let Some(status) = lock_statuses(&runtime.statuses).get_mut(index) {
        status.state = if expired {
            ReflexState::Expired
        } else {
            ReflexState::Active
        };
        status.last_fired_at = Some(Utc::now());
        status.fire_count = status.fire_count.saturating_add(1);
        status.last_error_code = None;
        if expired {
            expired_status = Some(status.clone());
        }
    }
    if let Some(status) = expired_status {
        write_lifetime_expired_audit(runtime, &status);
    }
}

fn write_lifetime_expired_audit(runtime: &RuntimeState, status: &ReflexStatus) {
    let Some(db) = runtime.audit_db.as_deref() else {
        return;
    };
    let audit = StoredReflexAudit {
        schema_version: SCHEMA_VERSION,
        audit_id: Uuid::now_v7().to_string(),
        reflex_id: status.id.clone(),
        ts_ns: now_ts_ns(),
        status: ReflexState::Expired,
        event_id: None,
        steps: Vec::new(),
        error_code: Some(error_codes::REFLEX_LIFETIME_EXPIRED.to_owned()),
        details: json!({
            "kind": REFLEX_LIFETIME_EXPIRED_KIND,
            "reason": "one_shot",
            "tick_index": runtime.tick_index,
            "fire_count": status.fire_count,
        }),
        redacted: false,
        redactions: Vec::new(),
    };
    if let Err(error) = write_audit(db, &audit) {
        tracing::warn!(
            component = "reflex_lifetime",
            reflex_id = %audit.reflex_id,
            audit_id = %audit.audit_id,
            detail = %error,
            "reflex lifetime audit write failed"
        );
    }
}

fn now_ts_ns() -> u64 {
    Utc::now()
        .timestamp_nanos_opt()
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or_default()
}

fn mark_reflex_starved(runtime: &RuntimeState, index: usize) {
    if let Some(status) = lock_statuses(&runtime.statuses).get_mut(index) {
        status.state = ReflexState::Starved;
        status.last_error_code = Some(synapse_core::error_codes::REFLEX_STARVED.to_owned());
    }
}

fn mark_reflex_active_if_starved(runtime: &RuntimeState, index: usize) {
    if let Some(status) = lock_statuses(&runtime.statuses).get_mut(index)
        && matches!(status.state, ReflexState::Starved)
    {
        status.state = ReflexState::Active;
        status.last_error_code = None;
    }
}

#[path = "scheduler_stats.rs"]
mod scheduler_stats;
pub use scheduler_stats::p99_jitter_us;

#[path = "scheduler_combo.rs"]
mod scheduler_combo;

#[path = "scheduler_tick.rs"]
mod scheduler_tick;

#[cfg(windows)]
#[path = "scheduler_windows.rs"]
mod windows_timer;
