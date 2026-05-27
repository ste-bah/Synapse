use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use chrono::Utc;
use serde_json::json;
use synapse_action::ActionHandle;
use synapse_core::{
    ReflexLifetime, ReflexState, ReflexStatus, SCHEMA_VERSION, StoredReflexAudit, error_codes,
};
use synapse_storage::Db;
use uuid::Uuid;

use super::{
    ScheduledReflex, ScheduledReflexDriver, SchedulerConfig, SchedulerTrigger, TickSample,
    scheduler_tick::tick,
};
use crate::{
    EventBus, REFLEX_LIFETIME_EXPIRED_KIND, SubscriberHandle,
    error::ReflexResult,
    kinds::{
        aim_track::AimTrackController, combo::ComboController, hold_button::HoldButtonController,
        hold_move::HoldMoveController, on_event::OnEventState,
    },
    write_audit,
};

#[derive(Clone, Debug)]
pub(super) struct RuntimeReflex {
    pub(super) registration_order: usize,
    pub(super) reflex: ScheduledReflex,
}

#[derive(Clone, Debug)]
pub(super) struct ReflexControl {
    pub(super) priority: u32,
    pub(super) active: bool,
}

pub(super) struct RuntimeState {
    pub(super) event_bus: EventBus,
    pub(super) action_handle: ActionHandle,
    pub(super) reflexes: Vec<RuntimeReflex>,
    pub(super) active_combos: Vec<ComboController>,
    pub(super) aim_track_states: Vec<Option<AimTrackController>>,
    pub(super) hold_move_states: Vec<Option<HoldMoveController>>,
    pub(super) hold_button_states: Vec<Option<HoldButtonController>>,
    pub(super) combo_states: Vec<Option<ComboController>>,
    pub(super) on_event_states: Vec<OnEventState>,
    pub(super) starvation_states: Vec<crate::conflict::StarvationState>,
    pub(super) subscription: SubscriberHandle,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) samples: Arc<Mutex<VecDeque<TickSample>>>,
    pub(super) controls: Arc<Mutex<Vec<ReflexControl>>>,
    pub(super) statuses: Arc<Mutex<Vec<ReflexStatus>>>,
    pub(super) config: SchedulerConfig,
    pub(super) audit_db: Option<Arc<Db>>,
    pub(super) tick_index: u64,
}

pub(super) fn aim_track_states(
    reflexes: &[ScheduledReflex],
) -> ReflexResult<Vec<Option<AimTrackController>>> {
    reflexes
        .iter()
        .map(|reflex| match &reflex.driver {
            ScheduledReflexDriver::AimTrack(params) => {
                AimTrackController::new(reflex.reflex_id.clone(), params.clone()).map(Some)
            }
            ScheduledReflexDriver::Actions
            | ScheduledReflexDriver::HoldMove(_)
            | ScheduledReflexDriver::HoldButton(_)
            | ScheduledReflexDriver::Combo(_) => Ok(None),
        })
        .collect()
}

pub(super) fn hold_move_states(
    reflexes: &[ScheduledReflex],
) -> ReflexResult<Vec<Option<HoldMoveController>>> {
    reflexes
        .iter()
        .map(|reflex| match &reflex.driver {
            ScheduledReflexDriver::HoldMove(params) => HoldMoveController::new(
                reflex.reflex_id.clone(),
                params.clone(),
                reflex.lifetime.clone(),
            )
            .map(Some),
            ScheduledReflexDriver::Actions
            | ScheduledReflexDriver::AimTrack(_)
            | ScheduledReflexDriver::HoldButton(_)
            | ScheduledReflexDriver::Combo(_) => Ok(None),
        })
        .collect()
}

pub(super) fn hold_button_states(
    reflexes: &[ScheduledReflex],
) -> ReflexResult<Vec<Option<HoldButtonController>>> {
    reflexes
        .iter()
        .map(|reflex| match &reflex.driver {
            ScheduledReflexDriver::HoldButton(params) => HoldButtonController::new(
                reflex.reflex_id.clone(),
                params.clone(),
                reflex.lifetime.clone(),
            )
            .map(Some),
            ScheduledReflexDriver::Actions
            | ScheduledReflexDriver::AimTrack(_)
            | ScheduledReflexDriver::HoldMove(_)
            | ScheduledReflexDriver::Combo(_) => Ok(None),
        })
        .collect()
}

pub(super) fn combo_states(reflexes: &[ScheduledReflex]) -> Vec<Option<ComboController>> {
    reflexes
        .iter()
        .map(|reflex| match &reflex.driver {
            ScheduledReflexDriver::Combo(params) => Some(ComboController::new(
                reflex.reflex_id.clone(),
                params.clone(),
            )),
            ScheduledReflexDriver::Actions
            | ScheduledReflexDriver::AimTrack(_)
            | ScheduledReflexDriver::HoldMove(_)
            | ScheduledReflexDriver::HoldButton(_) => None,
        })
        .collect()
}

#[cfg(windows)]
pub(super) fn run_scheduler_thread(mut runtime: RuntimeState) {
    if runtime.config.force_degraded {
        run_degraded(runtime, "forced_degraded_config");
        return;
    }

    match super::windows_timer::WindowsHighResolutionTimer::start(runtime.config.target_interval) {
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
pub(super) fn run_scheduler_thread(runtime: RuntimeState) {
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

pub(super) fn lock_samples(
    samples: &Arc<Mutex<VecDeque<TickSample>>>,
) -> std::sync::MutexGuard<'_, VecDeque<TickSample>> {
    match samples.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(super) fn lock_controls(
    controls: &Arc<Mutex<Vec<ReflexControl>>>,
) -> std::sync::MutexGuard<'_, Vec<ReflexControl>> {
    match controls.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(super) fn lock_statuses(
    statuses: &Arc<Mutex<Vec<ReflexStatus>>>,
) -> std::sync::MutexGuard<'_, Vec<ReflexStatus>> {
    match statuses.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(super) fn status_index(
    statuses: &Arc<Mutex<Vec<ReflexStatus>>>,
    reflex_id: &str,
) -> Option<usize> {
    lock_statuses(statuses)
        .iter()
        .position(|status| status.id == reflex_id)
}

pub(super) fn status_for_reflex(
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
    match &reflex.driver {
        ScheduledReflexDriver::Actions => match &reflex.trigger {
            SchedulerTrigger::EveryTick => format!("every_tick:{} actions", reflex.then.len()),
            SchedulerTrigger::OnEvent(_filter) => format!("on_event:{} actions", reflex.then.len()),
        },
        ScheduledReflexDriver::AimTrack(_params) => "aim_track".to_owned(),
        ScheduledReflexDriver::HoldMove(params) => format!("hold_move:{} keys", params.keys.len()),
        ScheduledReflexDriver::HoldButton(_params) => "hold_button".to_owned(),
        ScheduledReflexDriver::Combo(params) => format!("combo:{} steps", params.steps.len()),
    }
}

pub(super) fn mark_reflex_fired(runtime: &RuntimeState, index: usize) {
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
        write_lifetime_expired_audit(runtime, &status, "one_shot");
    }
}

pub(super) fn mark_reflex_lifetime_expired(runtime: &RuntimeState, index: usize, reason: &str) {
    if let Some(control) = lock_controls(&runtime.controls).get_mut(index) {
        control.active = false;
    }
    let expired_status = if let Some(status) = lock_statuses(&runtime.statuses).get_mut(index) {
        status.state = ReflexState::Expired;
        status.last_fired_at = Some(Utc::now());
        status.fire_count = status.fire_count.saturating_add(1);
        status.last_error_code = Some(error_codes::REFLEX_LIFETIME_EXPIRED.to_owned());
        Some(status.clone())
    } else {
        None
    };
    if let Some(status) = expired_status {
        write_lifetime_expired_audit(runtime, &status, reason);
    }
}

pub(super) fn mark_reflex_error(runtime: &RuntimeState, index: usize, code: &str) {
    if let Some(status) = lock_statuses(&runtime.statuses).get_mut(index) {
        status.last_error_code = Some(code.to_owned());
    }
}

fn write_lifetime_expired_audit(runtime: &RuntimeState, status: &ReflexStatus, reason: &str) {
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
            "reason": reason,
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

pub(super) fn mark_reflex_starved(runtime: &RuntimeState, index: usize) {
    if let Some(status) = lock_statuses(&runtime.statuses).get_mut(index) {
        status.state = ReflexState::Starved;
        status.last_error_code = Some(synapse_core::error_codes::REFLEX_STARVED.to_owned());
    }
}

pub(super) fn mark_reflex_active_if_starved(runtime: &RuntimeState, index: usize) {
    if let Some(status) = lock_statuses(&runtime.statuses).get_mut(index)
        && matches!(status.state, ReflexState::Starved)
    {
        status.state = ReflexState::Active;
        status.last_error_code = None;
    }
}
