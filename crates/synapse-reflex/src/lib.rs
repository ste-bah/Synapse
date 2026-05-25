pub mod audit;
pub mod bus;
pub mod conflict;
pub mod error;
pub mod kinds;
pub mod scheduler;

use std::{fmt, path::Path, sync::Arc};

use chrono::Utc;
use serde_json::json;
use synapse_action::ActionHandle;
use synapse_core::{ReflexState, ReflexStatus, SCHEMA_VERSION, StoredReflexAudit};
use synapse_storage::Db;
use uuid::Uuid;

pub use audit::write_audit;
pub use bus::{
    DEFAULT_MAX_SUBSCRIPTIONS, EVENTS_DROPPED_METRIC, EventBus, EventBusError, EventBusResult,
    PublishReport, SUBSCRIBER_QUEUE_CAPACITY, SubscriberHandle,
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

pub const REFLEX_REGISTERED_KIND: &str = "reflex_registered";

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
    reflexes: Vec<ScheduledReflex>,
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
        Ok(Self {
            db,
            action_handle,
            event_bus,
            reflexes: Vec::new(),
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
            SchedulerConfig::default(),
            Arc::clone(&self.db),
        )?;
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

    /// Returns the current scheduler status snapshot for active reflexes.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn statuses(&self) -> Vec<ReflexStatus> {
        self.scheduler
            .as_ref()
            .map_or_else(Vec::new, SchedulerHandle::statuses)
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
}

fn now_ts_ns() -> u64 {
    Utc::now()
        .timestamp_nanos_opt()
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::{error::Error, sync::Arc};

    use synapse_action::ActionHandle;
    use synapse_core::Action;
    use synapse_storage::Db;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    use super::{EventBus, ReflexRuntime};

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
}
