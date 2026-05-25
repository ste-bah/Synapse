pub mod audit;
pub mod bus;
pub mod conflict;
pub mod error;
pub mod kinds;
pub mod scheduler;

use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    path::Path,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use serde_json::json;
use synapse_action::ActionHandle;
use synapse_core::{
    ReflexId, ReflexLifetime, ReflexState, ReflexStatus, SCHEMA_VERSION, StoredReflexAudit,
    error_codes,
};
use synapse_storage::{Db, DiskPressureLevel, StorageResult, cf, decode_json};
use uuid::Uuid;

pub use audit::write_audit;
pub use bus::{
    DEFAULT_MAX_SUBSCRIPTIONS, DEFAULT_MAX_SUBSCRIPTIONS_NONZERO, EVENTS_DROPPED_METRIC, EventBus,
    EventBusError, EventBusResult, PublishReport, SUBSCRIBER_QUEUE_CAPACITY, SubscriberHandle,
};
pub use conflict::{REFLEX_STARVED_KIND, STARVATION_AFTER};
pub use error::{ReflexError, ReflexResult};
pub use kinds::aim_track::{
    AimTrackContext, AimTrackController, AimTrackOutput, AimTrackParams, AimTrackTarget,
    DEFAULT_EMA_ALPHA, DEFAULT_MAX_SPEED_PX_PER_TICK, REFLEX_TRACK_LOST_KIND, ResolvedElementBox,
    TRACK_LOST_AFTER,
};
pub use kinds::combo::{
    ComboContext, ComboController, ComboOutput, ComboParams, ComboPhase,
    REFLEX_COMBO_COMPLETED_KIND,
};
pub use kinds::hold_button::{
    HoldButtonController, HoldButtonOutput, HoldButtonParams, HoldButtonPhase,
};
pub use kinds::hold_lifetime::{
    HoldLifetimeContext, HoldReleaseReason, REFLEX_LIFETIME_EXPIRED_KIND,
};
pub use kinds::hold_move::{HoldMoveController, HoldMoveOutput, HoldMoveParams, HoldMovePhase};
pub use kinds::on_event::{
    MAX_ON_EVENT_FIRINGS_PER_TICK, REFLEX_FIRED_KIND, REFLEX_RECURSION_LIMIT_KIND,
};
pub use scheduler::{
    DEFAULT_REFLEX_PRIORITY, MAX_REFLEX_PRIORITY, MAX_SCHEDULED_REFLEXES, REFLEX_TICK_LATE_KIND,
    ReflexScheduler, ScheduledReflex, SchedulerConfig, SchedulerHandle, SchedulerTrigger,
    TickSample, p99_jitter_us,
};

pub const REFLEX_CANCELLED_KIND: &str = "reflex_cancelled";
pub const REFLEX_DISABLED_KIND: &str = "reflex_disabled_by_operator";
pub const REFLEX_REGISTERED_KIND: &str = "reflex_registered";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReflexCancelOutcome {
    Cancelled { status: ReflexStatus },
    NotFound,
    AlreadyExpired { status: ReflexStatus },
}

/// Runtime handle for the M3 reflex subsystem.
///
/// Reflex input controllers use the shared [`synapse_action::ActionHandle`] as
/// the `synapse-action::handle` interlock authority. Held input state remains
/// owned by the private `synapse-action` emitter `BitSet`; reflex must enqueue
/// `hold_*` down/up actions through this handle and must not mirror, read, or
/// mutate held state independently.
pub struct ReflexRuntime {
    db: Arc<Db>,
    action_handle: ActionHandle,
    event_bus: EventBus,
    scheduler_config: SchedulerConfig,
    reflexes: Vec<ScheduledReflex>,
    disabled_reflex_ids: HashSet<ReflexId>,
    scheduler: Option<SchedulerHandle>,
}

impl fmt::Debug for ReflexRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReflexRuntime")
            .field("db", &self.db)
            .field("action_handle", &self.action_handle)
            .field("event_bus", &self.event_bus)
            .field("reflex_count", &self.reflexes.len())
            .finish_non_exhaustive()
    }
}

impl ReflexRuntime {
    /// Spawns the reflex runtime scaffold.
    ///
    /// # Errors
    ///
    /// The scaffold currently cannot fail after receiving initialized handles.
    /// Later M3 scheduler/bus work extends this result with OS-thread setup
    /// errors.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn spawn(
        db: Arc<Db>,
        action_handle: ActionHandle,
        event_bus: EventBus,
    ) -> ReflexResult<Self> {
        Self::spawn_with_config(db, action_handle, event_bus, SchedulerConfig::default())
    }

    /// Spawns the reflex runtime with an explicit scheduler config.
    ///
    /// # Errors
    ///
    /// The scaffold currently cannot fail after receiving initialized handles.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn spawn_with_config(
        db: Arc<Db>,
        action_handle: ActionHandle,
        event_bus: EventBus,
        scheduler_config: SchedulerConfig,
    ) -> ReflexResult<Self> {
        Ok(Self {
            db,
            action_handle,
            event_bus,
            scheduler_config,
            reflexes: Vec::new(),
            disabled_reflex_ids: HashSet::new(),
            scheduler: None,
        })
    }

    /// Registers a new reflex into this runtime and persists the registration audit row.
    ///
    /// # Errors
    ///
    /// Returns a [`ReflexError`] when the runtime has reached the reflex cap,
    /// the reflex priority or trigger is invalid, the scheduler cannot be
    /// restarted, or the registration audit row cannot be persisted.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime", reflex_id = %reflex.reflex_id))]
    pub fn register(&mut self, reflex: &ScheduledReflex) -> ReflexResult<ReflexStatus> {
        if reflex.priority > MAX_REFLEX_PRIORITY {
            return Err(ReflexError::PriorityInvalid {
                detail: format!(
                    "priority {} exceeds maximum {MAX_REFLEX_PRIORITY}",
                    reflex.priority
                ),
            });
        }
        let mut next = self.reflexes.clone();
        next.push(reflex.clone());
        scheduler::validate_reflexes(&next)?;

        let new_scheduler = ReflexScheduler::spawn_with_audit_db(
            self.event_bus.clone(),
            self.action_handle.clone(),
            next.clone(),
            self.scheduler_config.clone(),
            Arc::clone(&self.db),
        )?;
        if !self.disabled_reflex_ids.is_empty() {
            let disabled_reflex_ids = self.disabled_reflex_ids.iter().cloned().collect::<Vec<_>>();
            let _disabled_statuses = new_scheduler.disable_reflexes(&disabled_reflex_ids);
        }
        let old_scheduler = self.scheduler.replace(new_scheduler);
        self.reflexes = next;
        if let Some(mut old_scheduler) = old_scheduler {
            old_scheduler.stop()?;
        }
        let status = self
            .scheduler
            .as_ref()
            .and_then(|scheduler| {
                scheduler
                    .statuses()
                    .into_iter()
                    .find(|status| status.id == reflex.reflex_id)
            })
            .ok_or_else(|| ReflexError::ParamsInvalid {
                detail: format!("registered reflex status missing: {}", reflex.reflex_id),
            })?;
        self.write_registration_audit(&status)?;
        Ok(status)
    }

    /// Cancels an active reflex and persists a cancellation audit row.
    ///
    /// # Errors
    ///
    /// Returns a [`ReflexError`] if the cancellation audit row cannot be
    /// persisted.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime", reflex_id = %reflex_id))]
    pub fn cancel(&mut self, reflex_id: &str) -> ReflexResult<ReflexCancelOutcome> {
        let Some(status) = self
            .statuses()
            .into_iter()
            .find(|status| status.id == reflex_id)
        else {
            return Ok(ReflexCancelOutcome::NotFound);
        };

        match status.state {
            ReflexState::Expired => {
                return Ok(ReflexCancelOutcome::AlreadyExpired { status });
            }
            ReflexState::Cancelled => {
                return Ok(ReflexCancelOutcome::Cancelled { status });
            }
            ReflexState::Active
            | ReflexState::Paused
            | ReflexState::Disabled
            | ReflexState::Starved => {}
        }

        let Some(scheduler) = &self.scheduler else {
            return Ok(ReflexCancelOutcome::NotFound);
        };
        if !scheduler.cancel_reflex(reflex_id) {
            return Ok(ReflexCancelOutcome::NotFound);
        }
        self.disabled_reflex_ids.remove(reflex_id);
        let status = scheduler
            .statuses()
            .into_iter()
            .find(|status| status.id == reflex_id)
            .ok_or_else(|| ReflexError::ParamsInvalid {
                detail: format!("cancelled reflex status missing: {reflex_id}"),
            })?;
        self.write_cancellation_audit(&status)?;
        Ok(ReflexCancelOutcome::Cancelled { status })
    }

    /// Disables every active scheduler reflex for the operator panic hotkey.
    ///
    /// # Errors
    ///
    /// Returns a [`ReflexError`] when the disabled audit rows cannot be
    /// persisted.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn disable_all_by_operator(&mut self) -> ReflexResult<Vec<ReflexStatus>> {
        let Some(scheduler) = &self.scheduler else {
            return Ok(Vec::new());
        };
        let disabled = scheduler.disable_all_reflexes();
        for status in &disabled {
            self.disabled_reflex_ids.insert(status.id.clone());
        }
        self.write_disabled_audits(&disabled)?;
        Ok(disabled)
    }

    /// Returns the current scheduler status snapshot for active reflexes.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn statuses(&self) -> Vec<ReflexStatus> {
        self.scheduler
            .as_ref()
            .map_or_else(Vec::new, SchedulerHandle::statuses)
    }

    /// Lists reflex statuses visible to MCP callers.
    ///
    /// By default, terminal cancelled/expired statuses are hidden. When
    /// `include_expired` is set, terminal rows from `CF_REFLEX_AUDIT` are
    /// merged back into the current runtime snapshot so cancelled/expired
    /// reflexes remain inspectable after a daemon restart.
    ///
    /// # Errors
    ///
    /// Returns a [`ReflexError`] when the audit column family cannot be scanned
    /// or an audit row cannot be decoded.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime", include_expired))]
    pub fn list(&self, include_expired: bool) -> ReflexResult<Vec<ReflexStatus>> {
        let mut statuses = self
            .statuses()
            .into_iter()
            .filter(|status| include_expired || is_non_terminal(status.state))
            .collect::<Vec<_>>();

        if include_expired {
            let existing = statuses
                .iter()
                .map(|status| status.id.clone())
                .collect::<HashSet<_>>();
            statuses.extend(
                self.terminal_statuses_from_audit()?
                    .into_iter()
                    .filter(|status| !existing.contains(&status.id)),
            );
        }

        Ok(statuses)
    }

    /// Returns persisted reflex audit rows in newest-first order.
    ///
    /// When `reflex_id` is present, the audit column family is read by the
    /// reflex audit key prefix. Without a `reflex_id`, rows are sorted globally
    /// by persisted timestamp and audit id before the limit is applied.
    ///
    /// # Errors
    ///
    /// Returns a [`ReflexError`] when the audit column family cannot be scanned
    /// or an audit row cannot be decoded.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime", reflex_id, limit))]
    pub fn history(
        &self,
        reflex_id: Option<&str>,
        limit: usize,
    ) -> ReflexResult<Vec<StoredReflexAudit>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        self.db
            .flush()
            .map_err(|error| ReflexError::ParamsInvalid {
                detail: format!("reflex audit flush before scan failed: {error}"),
            })?;

        let rows = reflex_id
            .map_or_else(
                || self.db.scan_cf(cf::CF_REFLEX_AUDIT),
                |reflex_id| {
                    self.db
                        .scan_cf_prefix(cf::CF_REFLEX_AUDIT, audit_key_prefix(reflex_id).as_bytes())
                },
            )
            .map_err(|error| ReflexError::ParamsInvalid {
                detail: format!("reflex audit scan failed: {error}"),
            })?;

        let mut audits = rows
            .into_iter()
            .map(|(_key, value)| {
                decode_json::<StoredReflexAudit>(&value).map_err(|error| {
                    ReflexError::ParamsInvalid {
                        detail: format!("reflex audit decode failed: {error}"),
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        audits.sort_by(|left, right| {
            right
                .ts_ns
                .cmp(&left.ts_ns)
                .then_with(|| right.audit_id.cmp(&left.audit_id))
                .then_with(|| right.reflex_id.cmp(&left.reflex_id))
        });
        audits.truncate(limit);
        Ok(audits)
    }

    /// Returns the storage path backing this runtime.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn storage_path(&self) -> &Path {
        &self.db.path
    }

    /// Returns the storage schema version backing this runtime.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn schema_version(&self) -> u32 {
        self.db.schema_version
    }

    /// Returns the current storage pressure level backing reflex persistence.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn storage_pressure_level(&self) -> DiskPressureLevel {
        self.db.pressure_level()
    }

    /// Returns logical byte sizes for each storage column family.
    ///
    /// # Errors
    ///
    /// Returns a storage error when a column family scan fails.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn storage_cf_sizes(&self) -> StorageResult<BTreeMap<String, u64>> {
        self.db.cf_sizes()
    }

    /// Returns the number of currently active reflexes.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn active_count(&self) -> usize {
        self.statuses()
            .into_iter()
            .filter(|status| status.state == ReflexState::Active)
            .count()
    }

    /// Returns the most recent scheduler tick jitter.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn last_tick_jitter_us(&self) -> Option<u64> {
        self.scheduler
            .as_ref()
            .and_then(|scheduler| scheduler.samples().last().map(|sample| sample.jitter_us))
    }

    /// Returns true when the latest tick ran in degraded mode or missed its deadline.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn degraded_latency(&self) -> bool {
        self.scheduler
            .as_ref()
            .and_then(|scheduler| scheduler.samples().last().copied())
            .is_some_and(|sample| sample.degraded || sample.late)
    }

    /// Counts persisted recursion-guard clamp audit rows.
    ///
    /// # Errors
    ///
    /// Returns a reflex error when audit rows cannot be scanned or decoded.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn recursion_clamps_total(&self) -> ReflexResult<u64> {
        let rows =
            self.db
                .scan_cf(cf::CF_REFLEX_AUDIT)
                .map_err(|error| ReflexError::ParamsInvalid {
                    detail: format!("reflex audit scan failed: {error}"),
                })?;
        let mut total = 0_u64;
        for (_key, value) in rows {
            let audit = decode_json::<StoredReflexAudit>(&value).map_err(|error| {
                ReflexError::ParamsInvalid {
                    detail: format!("reflex audit decode failed: {error}"),
                }
            })?;
            if audit.error_code.as_deref() == Some(error_codes::REFLEX_RECURSION_LIMIT) {
                total = total.saturating_add(1);
            }
        }
        Ok(total)
    }

    /// Returns the action emitter handle used by reflex controllers.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn action_handle(&self) -> &ActionHandle {
        &self.action_handle
    }

    /// Returns the event bus handle used by this runtime.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    fn write_registration_audit(&self, status: &ReflexStatus) -> ReflexResult<()> {
        let audit = StoredReflexAudit {
            schema_version: SCHEMA_VERSION,
            audit_id: Uuid::now_v7().to_string(),
            reflex_id: status.id.clone(),
            ts_ns: now_ts_ns(),
            status: ReflexState::Active,
            event_id: None,
            steps: Vec::new(),
            error_code: None,
            details: json!({
                "kind": REFLEX_REGISTERED_KIND,
                "kind_summary": status.kind_summary,
                "priority": status.priority,
                "lifetime": status.lifetime,
                "exclusive": status.exclusive,
            }),
            redacted: false,
            redactions: Vec::new(),
        };
        write_audit(&self.db, &audit).map_err(|error| ReflexError::ParamsInvalid {
            detail: format!("registration audit write failed: {error}"),
        })?;
        self.db.flush().map_err(|error| ReflexError::ParamsInvalid {
            detail: format!("registration audit flush failed: {error}"),
        })
    }

    fn write_cancellation_audit(&self, status: &ReflexStatus) -> ReflexResult<()> {
        let audit = StoredReflexAudit {
            schema_version: SCHEMA_VERSION,
            audit_id: Uuid::now_v7().to_string(),
            reflex_id: status.id.clone(),
            ts_ns: now_ts_ns(),
            status: ReflexState::Cancelled,
            event_id: None,
            steps: Vec::new(),
            error_code: None,
            details: json!({
                "kind": REFLEX_CANCELLED_KIND,
                "kind_summary": status.kind_summary,
                "priority": status.priority,
                "lifetime": status.lifetime,
                "exclusive": status.exclusive,
            }),
            redacted: false,
            redactions: Vec::new(),
        };
        write_audit(&self.db, &audit).map_err(|error| ReflexError::ParamsInvalid {
            detail: format!("cancellation audit write failed: {error}"),
        })?;
        self.db.flush().map_err(|error| ReflexError::ParamsInvalid {
            detail: format!("cancellation audit flush failed: {error}"),
        })
    }

    fn write_disabled_audits(&self, statuses: &[ReflexStatus]) -> ReflexResult<()> {
        if statuses.is_empty() {
            return Ok(());
        }
        for status in statuses {
            let audit = StoredReflexAudit {
                schema_version: SCHEMA_VERSION,
                audit_id: Uuid::now_v7().to_string(),
                reflex_id: status.id.clone(),
                ts_ns: now_ts_ns(),
                status: ReflexState::Disabled,
                event_id: None,
                steps: Vec::new(),
                error_code: Some(error_codes::REFLEX_DISABLED_BY_OPERATOR.to_owned()),
                details: json!({
                    "kind": REFLEX_DISABLED_KIND,
                    "kind_summary": status.kind_summary,
                    "priority": status.priority,
                    "lifetime": status.lifetime,
                    "exclusive": status.exclusive,
                    "reason": "operator_hotkey",
                }),
                redacted: false,
                redactions: Vec::new(),
            };
            write_audit(&self.db, &audit).map_err(|error| ReflexError::ParamsInvalid {
                detail: format!("disabled audit write failed: {error}"),
            })?;
        }
        self.db.flush().map_err(|error| ReflexError::ParamsInvalid {
            detail: format!("disabled audit flush failed: {error}"),
        })
    }

    fn terminal_statuses_from_audit(&self) -> ReflexResult<Vec<ReflexStatus>> {
        let rows =
            self.db
                .scan_cf(cf::CF_REFLEX_AUDIT)
                .map_err(|error| ReflexError::ParamsInvalid {
                    detail: format!("reflex audit scan failed: {error}"),
                })?;
        let mut audits = rows
            .into_iter()
            .map(|(_key, value)| {
                decode_json::<StoredReflexAudit>(&value).map_err(|error| {
                    ReflexError::ParamsInvalid {
                        detail: format!("reflex audit decode failed: {error}"),
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        audits.sort_by_key(|audit| (audit.reflex_id.clone(), audit.ts_ns, audit.audit_id.clone()));

        let mut accumulators = BTreeMap::<String, AuditStatusAccumulator>::new();
        for audit in audits {
            accumulators
                .entry(audit.reflex_id.clone())
                .or_insert_with(|| AuditStatusAccumulator::new(audit.reflex_id.clone()))
                .record(audit);
        }

        Ok(accumulators
            .into_values()
            .filter_map(AuditStatusAccumulator::into_terminal_status)
            .collect())
    }
}

fn now_ts_ns() -> u64 {
    Utc::now()
        .timestamp_nanos_opt()
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or_default()
}

const fn is_non_terminal(state: ReflexState) -> bool {
    !matches!(state, ReflexState::Cancelled | ReflexState::Expired)
}

#[derive(Clone, Debug)]
struct AuditStatusAccumulator {
    reflex_id: String,
    registered_at: Option<DateTime<Utc>>,
    kind_summary: Option<String>,
    priority: Option<u32>,
    lifetime: Option<ReflexLifetime>,
    exclusive: Option<bool>,
    last_fired_at: Option<DateTime<Utc>>,
    fire_count: u64,
    terminal: Option<StoredReflexAudit>,
}

impl AuditStatusAccumulator {
    const fn new(reflex_id: String) -> Self {
        Self {
            reflex_id,
            registered_at: None,
            kind_summary: None,
            priority: None,
            lifetime: None,
            exclusive: None,
            last_fired_at: None,
            fire_count: 0,
            terminal: None,
        }
    }

    fn record(&mut self, audit: StoredReflexAudit) {
        let at = datetime_from_ts_ns(audit.ts_ns);
        let details_kind = audit
            .details
            .get("kind")
            .and_then(serde_json::Value::as_str);

        if details_kind == Some(REFLEX_REGISTERED_KIND) {
            self.registered_at = Some(at);
            self.update_common_fields(&audit);
        } else if details_kind == Some(REFLEX_FIRED_KIND) {
            self.last_fired_at = Some(at);
            self.fire_count = self.fire_count.saturating_add(1);
        }

        if matches!(audit.status, ReflexState::Cancelled | ReflexState::Expired) {
            self.update_common_fields(&audit);
            self.terminal = Some(audit);
        }
    }

    fn update_common_fields(&mut self, audit: &StoredReflexAudit) {
        let details = &audit.details;
        if let Some(kind_summary) = details
            .get("kind_summary")
            .and_then(serde_json::Value::as_str)
        {
            self.kind_summary = Some(kind_summary.to_owned());
        }
        if let Some(priority) = details
            .get("priority")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
        {
            self.priority = Some(priority);
        }
        if let Some(lifetime) = details
            .get("lifetime")
            .cloned()
            .and_then(|value| serde_json::from_value::<ReflexLifetime>(value).ok())
        {
            self.lifetime = Some(lifetime);
        }
        if let Some(exclusive) = details
            .get("exclusive")
            .and_then(serde_json::Value::as_bool)
        {
            self.exclusive = Some(exclusive);
        }
    }

    fn into_terminal_status(self) -> Option<ReflexStatus> {
        let terminal = self.terminal?;
        let terminal_at = datetime_from_ts_ns(terminal.ts_ns);
        Some(ReflexStatus {
            id: self.reflex_id,
            kind_summary: self.kind_summary.unwrap_or_else(|| "unknown".to_owned()),
            state: terminal.status,
            registered_at: self.registered_at.unwrap_or(terminal_at),
            last_fired_at: self.last_fired_at,
            fire_count: self.fire_count,
            priority: self.priority.unwrap_or(DEFAULT_REFLEX_PRIORITY),
            lifetime: self.lifetime.unwrap_or_default(),
            exclusive: self.exclusive.unwrap_or(false),
            last_error_code: terminal.error_code,
        })
    }
}

fn datetime_from_ts_ns(ts_ns: u64) -> DateTime<Utc> {
    DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_nanos(ts_ns))
}

fn audit_key_prefix(reflex_id: &str) -> String {
    format!("{reflex_id}:")
}

#[cfg(test)]
mod tests {
    use std::{error::Error, sync::Arc};

    use synapse_action::ActionHandle;
    use synapse_core::{Action, EventFilter, ReflexState, StoredReflexAudit};
    use synapse_storage::{Db, cf, decode_json};
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    use super::{
        EventBus, REFLEX_CANCELLED_KIND, REFLEX_DISABLED_KIND, REFLEX_REGISTERED_KIND,
        ReflexCancelOutcome, ReflexRuntime, ScheduledReflex,
    };

    const TEST_SCHEMA_VERSION: u32 = 7;

    #[test]
    fn spawn_retains_runtime_inputs_and_action_handle() -> Result<(), Box<dyn Error>> {
        let temp = tempdir()?;
        let db = Arc::new(Db::open(&temp.path().join("db"), TEST_SCHEMA_VERSION)?);
        let (action_handle, mut action_rx) = ActionHandle::channel();
        assert!(matches!(
            action_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        let runtime = ReflexRuntime::spawn(Arc::clone(&db), action_handle, EventBus::default())?;
        runtime.action_handle().try_execute(Action::ReleaseAll)?;
        let (queued_action, _ack) = action_rx.try_recv()?;

        assert_eq!(runtime.schema_version(), TEST_SCHEMA_VERSION);
        assert_eq!(queued_action, Action::ReleaseAll);
        Ok(())
    }

    #[test]
    fn cancel_registered_reflex_marks_status_and_writes_audit() -> Result<(), Box<dyn Error>> {
        let temp = tempdir()?;
        let db = Arc::new(Db::open(&temp.path().join("db"), TEST_SCHEMA_VERSION)?);
        let (action_handle, _action_rx) = ActionHandle::channel();
        let mut runtime =
            ReflexRuntime::spawn(Arc::clone(&db), action_handle, EventBus::default())?;
        let reflex = ScheduledReflex::on_event(
            "reflex-runtime-cancel",
            EventFilter::Kind {
                kind: "support-cancel".to_owned(),
            },
            vec![Action::ReleaseAll],
        );
        let registered = runtime.register(&reflex)?;
        assert_eq!(registered.state, ReflexState::Active);

        let outcome = runtime.cancel("reflex-runtime-cancel")?;
        let ReflexCancelOutcome::Cancelled { status } = outcome else {
            panic!("registered reflex should cancel");
        };
        assert_eq!(status.state, ReflexState::Cancelled);
        assert_eq!(
            runtime
                .statuses()
                .into_iter()
                .find(|status| status.id == "reflex-runtime-cancel")
                .map(|status| status.state),
            Some(ReflexState::Cancelled)
        );
        assert!(runtime.list(false)?.is_empty());
        let visible_with_expired = runtime.list(true)?;
        assert_eq!(visible_with_expired.len(), 1);
        assert_eq!(visible_with_expired[0].state, ReflexState::Cancelled);

        let audits = db
            .scan_cf(cf::CF_REFLEX_AUDIT)?
            .iter()
            .map(|(_key, value)| decode_json::<StoredReflexAudit>(value))
            .collect::<Result<Vec<_>, _>>()?;
        let kinds = audits
            .iter()
            .map(|audit| audit.details["kind"].as_str())
            .collect::<Vec<_>>();
        assert!(kinds.contains(&Some(REFLEX_REGISTERED_KIND)));
        assert!(kinds.contains(&Some(REFLEX_CANCELLED_KIND)));
        assert!(
            audits
                .iter()
                .any(|audit| audit.status == ReflexState::Cancelled)
        );
        drop(runtime);

        let (action_handle, _action_rx) = ActionHandle::channel();
        let restarted = ReflexRuntime::spawn(Arc::clone(&db), action_handle, EventBus::default())?;
        let restored = restarted.list(true)?;
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, "reflex-runtime-cancel");
        assert_eq!(restored[0].state, ReflexState::Cancelled);
        assert_eq!(restored[0].kind_summary, "on_event:1 actions");
        Ok(())
    }

    #[test]
    fn disable_all_by_operator_marks_statuses_and_writes_audit() -> Result<(), Box<dyn Error>> {
        let temp = tempdir()?;
        let db = Arc::new(Db::open(&temp.path().join("db"), TEST_SCHEMA_VERSION)?);
        let (action_handle, _action_rx) = ActionHandle::channel();
        let mut runtime =
            ReflexRuntime::spawn(Arc::clone(&db), action_handle, EventBus::default())?;
        let first = ScheduledReflex::on_event(
            "reflex-runtime-disable-a",
            EventFilter::Kind {
                kind: "support-disable-a".to_owned(),
            },
            vec![Action::ReleaseAll],
        );
        let second = ScheduledReflex::on_event(
            "reflex-runtime-disable-b",
            EventFilter::Kind {
                kind: "support-disable-b".to_owned(),
            },
            vec![Action::ReleaseAll],
        );
        runtime.register(&first)?;
        runtime.register(&second)?;

        let disabled = runtime.disable_all_by_operator()?;
        assert_eq!(disabled.len(), 2);
        assert!(
            disabled
                .iter()
                .all(|status| status.state == ReflexState::Disabled)
        );
        assert!(
            runtime
                .list(false)?
                .iter()
                .all(|status| status.state == ReflexState::Disabled)
        );
        assert!(runtime.disable_all_by_operator()?.is_empty());

        let audits = db
            .scan_cf(cf::CF_REFLEX_AUDIT)?
            .iter()
            .map(|(_key, value)| decode_json::<StoredReflexAudit>(value))
            .collect::<Result<Vec<_>, _>>()?;
        let disabled_audits = audits
            .iter()
            .filter(|audit| audit.details["kind"].as_str() == Some(REFLEX_DISABLED_KIND))
            .collect::<Vec<_>>();
        assert_eq!(disabled_audits.len(), 2);
        assert!(disabled_audits.iter().all(|audit| {
            audit.status == ReflexState::Disabled
                && audit.error_code.as_deref()
                    == Some(synapse_core::error_codes::REFLEX_DISABLED_BY_OPERATOR)
        }));
        Ok(())
    }
}
