use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use chrono::Utc;
use serde_json::json;
use synapse_core::{
    Action, Event, EventSource, ReflexId, ReflexState, SCHEMA_VERSION, StoredReflexAudit,
    error_codes,
};
use uuid::Uuid;

use super::{
    REFLEX_TICK_LATE_KIND, RuntimeState, ScheduledReflexDriver, SchedulerTrigger, TickSample,
    scheduler_combo::{dispatch_reflex_action, step_active_combos},
    scheduler_stateful::step_stateful_controllers,
};
use crate::{
    ReflexError, ReflexResult,
    conflict::{ConflictCandidate, ConflictLoser, REFLEX_STARVED_KIND, resolve_conflicts},
    kinds::on_event::{OnEventTickGuard, publish_fired},
    write_audit,
};

pub(super) fn tick(runtime: &mut RuntimeState, elapsed: Duration, degraded: bool) {
    let events = runtime.subscription.drain();
    let mut dispatched_actions = 0_usize;
    let mut dispatch_blocked = false;
    step_active_combos(
        runtime,
        elapsed,
        &mut dispatched_actions,
        &mut dispatch_blocked,
    );
    if !dispatch_blocked {
        step_stateful_controllers(
            runtime,
            &events,
            elapsed,
            &mut dispatched_actions,
            &mut dispatch_blocked,
        );
    }

    if !dispatch_blocked {
        dispatch_triggered_reflexes(
            runtime,
            &events,
            elapsed,
            &mut dispatched_actions,
            &mut dispatch_blocked,
        );
    }

    record_tick_sample(
        runtime,
        elapsed,
        degraded,
        events.len(),
        dispatched_actions,
        dispatch_blocked,
    );
}

fn dispatch_triggered_reflexes(
    runtime: &mut RuntimeState,
    events: &[Event],
    elapsed: Duration,
    dispatched_actions: &mut usize,
    dispatch_blocked: &mut bool,
) {
    let now = Instant::now();
    let controls = super::lock_controls(&runtime.controls).clone();
    let triggered = collect_triggered_reflexes(runtime, events, now, &controls);
    let candidates = triggered
        .iter()
        .enumerate()
        .map(|(candidate_index, trigger)| {
            let runtime_reflex = &runtime.reflexes[trigger.reflex_index];
            let control = &controls[trigger.reflex_index];
            ConflictCandidate::new(
                candidate_index,
                trigger.reflex_index,
                trigger.reflex_id.clone(),
                control.priority,
                runtime_reflex.registration_order,
                &trigger.actions,
            )
        })
        .collect::<Vec<_>>();
    let resolution = resolve_conflicts(&candidates);
    record_starvation(runtime, &resolution.losers, elapsed, &controls);

    let mut guard = OnEventTickGuard::default();

    for candidate_index in resolution.winners {
        let trigger = &triggered[candidate_index];
        if !controls
            .get(trigger.reflex_index)
            .is_some_and(|control| control.active)
        {
            continue;
        }
        match &trigger.trigger_event {
            None => match dispatch_actions(runtime, &trigger.reflex_id, trigger.actions.clone()) {
                Ok(action_count) => {
                    *dispatched_actions = dispatched_actions.saturating_add(action_count);
                    super::mark_reflex_fired(runtime, trigger.reflex_index);
                }
                Err(error) => {
                    *dispatch_blocked = true;
                    warn_dispatch_blocked(&trigger.reflex_id, &error);
                    break;
                }
            },
            Some(event) => {
                if !guard.can_fire() {
                    guard.report_limit_once(
                        &runtime.event_bus,
                        runtime.audit_db.as_deref(),
                        &trigger.reflex_id,
                        runtime.tick_index,
                        event,
                        runtime.audit_context.as_ref(),
                    );
                    break;
                }
                match dispatch_actions(runtime, &trigger.reflex_id, trigger.actions.clone()) {
                    Ok(action_count) => {
                        *dispatched_actions = dispatched_actions.saturating_add(action_count);
                        runtime.on_event_states[trigger.reflex_index].mark_fired(now);
                        guard.record_fire();
                        publish_fired(
                            &runtime.event_bus,
                            runtime.audit_db.as_deref(),
                            &trigger.reflex_id,
                            runtime.tick_index,
                            event,
                            &trigger.actions,
                            runtime.audit_context.as_ref(),
                        );
                        super::mark_reflex_fired(runtime, trigger.reflex_index);
                    }
                    Err(error) => {
                        *dispatch_blocked = true;
                        warn_dispatch_blocked(&trigger.reflex_id, &error);
                        break;
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
struct TriggeredReflex {
    reflex_index: usize,
    reflex_id: ReflexId,
    actions: Vec<Action>,
    trigger_event: Option<Event>,
}

fn collect_triggered_reflexes(
    runtime: &RuntimeState,
    events: &[Event],
    now: Instant,
    controls: &[super::ReflexControl],
) -> Vec<TriggeredReflex> {
    let mut triggered = Vec::new();
    for index in 0..runtime.reflexes.len() {
        if !controls.get(index).is_some_and(|control| control.active) {
            continue;
        }
        let reflex = &runtime.reflexes[index].reflex;
        if !matches!(reflex.driver, ScheduledReflexDriver::Actions) {
            continue;
        }
        match &reflex.trigger {
            SchedulerTrigger::EveryTick => {
                triggered.push(TriggeredReflex {
                    reflex_index: index,
                    reflex_id: reflex.reflex_id.clone(),
                    actions: reflex.then.clone(),
                    trigger_event: None,
                });
            }
            SchedulerTrigger::OnEvent(filter) => {
                for event in events {
                    if !filter.matches(event)
                        || !runtime.on_event_states[index].allows_fire(now, reflex.debounce)
                    {
                        continue;
                    }
                    triggered.push(TriggeredReflex {
                        reflex_index: index,
                        reflex_id: reflex.reflex_id.clone(),
                        actions: reflex.then.clone(),
                        trigger_event: Some(event.clone()),
                    });
                    if !reflex.debounce.is_zero() {
                        break;
                    }
                }
            }
        }
    }
    triggered
}

fn record_starvation(
    runtime: &mut RuntimeState,
    losers: &[crate::conflict::ConflictLoser],
    elapsed: Duration,
    controls: &[super::ReflexControl],
) {
    let mut losing_slots = HashSet::with_capacity(losers.len());
    for loser in losers {
        losing_slots.insert(loser.loser_slot);
        if runtime.starvation_states[loser.loser_slot].record_loss(elapsed) {
            publish_starved(
                &runtime.event_bus,
                runtime.audit_db.as_deref(),
                loser,
                runtime.tick_index,
                runtime.starvation_states[loser.loser_slot].contended_for(),
                runtime.audit_context.as_ref(),
            );
            super::mark_reflex_starved(runtime, loser.loser_slot);
        }
    }

    for index in 0..runtime.starvation_states.len() {
        if losing_slots.contains(&index) {
            continue;
        }
        runtime.starvation_states[index].reset();
        if controls.get(index).is_some_and(|control| control.active) {
            super::mark_reflex_active_if_starved(runtime, index);
        }
    }
}

fn publish_starved(
    event_bus: &crate::EventBus,
    audit_db: Option<&synapse_storage::Db>,
    loser: &ConflictLoser,
    tick_index: u64,
    starved_for: Duration,
    audit_context: Option<&synapse_core::StoredAuditContext>,
) {
    let starved_for_ms = u64::try_from(starved_for.as_millis()).unwrap_or(u64::MAX);
    let event = Event {
        seq: tick_index,
        at: Utc::now(),
        source: EventSource::Reflex,
        kind: REFLEX_STARVED_KIND.to_owned(),
        data: json!({
            "code": error_codes::REFLEX_STARVED,
            "reflex_id": loser.loser_reflex_id,
            "winner_reflex_id": loser.winner_reflex_id,
            "resource": loser.resource,
            "tick_index": tick_index,
            "starved_for_ms": starved_for_ms,
        }),
        correlations: Vec::new(),
    };
    let _report = event_bus.publish(event);
    let Some(db) = audit_db else {
        return;
    };
    let audit = StoredReflexAudit {
        schema_version: SCHEMA_VERSION,
        audit_id: Uuid::now_v7().to_string(),
        reflex_id: loser.loser_reflex_id.clone(),
        ts_ns: now_ts_ns(),
        status: ReflexState::Starved,
        event_id: None,
        audit_context: audit_context.cloned(),
        steps: Vec::new(),
        error_code: Some(error_codes::REFLEX_STARVED.to_owned()),
        details: json!({
            "kind": REFLEX_STARVED_KIND,
            "winner_reflex_id": loser.winner_reflex_id,
            "resource": loser.resource,
            "tick_index": tick_index,
            "starved_for_ms": starved_for_ms,
        }),
        redacted: false,
        redactions: Vec::new(),
    };
    if let Err(error) = write_audit(db, &audit) {
        tracing::warn!(
            component = "reflex_conflict",
            reflex_id = %audit.reflex_id,
            audit_id = %audit.audit_id,
            detail = %error,
            "reflex starvation audit write failed"
        );
    }
}

fn dispatch_actions(
    runtime: &mut RuntimeState,
    reflex_id: &ReflexId,
    actions: Vec<Action>,
) -> ReflexResult<usize> {
    let mut dispatched = 0_usize;
    for action in actions {
        let action_count = dispatch_reflex_action(runtime, reflex_id, action)?;
        dispatched = dispatched.saturating_add(action_count);
    }
    Ok(dispatched)
}

fn record_tick_sample(
    runtime: &mut RuntimeState,
    elapsed: Duration,
    degraded: bool,
    event_count: usize,
    dispatched_actions: usize,
    dispatch_blocked: bool,
) {
    let elapsed_us = duration_us(elapsed);
    let target_us = duration_us(runtime.config.target_interval);
    let jitter_us = elapsed_us.abs_diff(target_us);
    let deadline_late = elapsed > runtime.config.late_after;
    let late = deadline_late || dispatch_blocked;
    if late {
        let reason = if dispatch_blocked {
            "dispatch_blocked"
        } else {
            "deadline_miss"
        };
        emit_tick_late(runtime, elapsed_us, jitter_us, reason);
    }

    let sample = TickSample {
        tick_index: runtime.tick_index,
        elapsed_us,
        jitter_us,
        target_us,
        pulled_events: event_count,
        dispatched_actions,
        late,
        degraded,
    };
    tracing::info!(
        component = "reflex_scheduler",
        tick_index = sample.tick_index,
        elapsed_us = sample.elapsed_us,
        jitter_us = sample.jitter_us,
        target_us = sample.target_us,
        pulled_events = sample.pulled_events,
        dispatched_actions = sample.dispatched_actions,
        late = sample.late,
        degraded = sample.degraded,
        "reflex scheduler tick"
    );
    push_sample(&runtime.samples, runtime.config.sample_limit, sample);
    runtime.tick_index = runtime.tick_index.saturating_add(1);
}

fn emit_tick_late(runtime: &RuntimeState, elapsed_us: u64, jitter_us: u64, reason: &str) {
    let event = Event {
        seq: runtime.tick_index,
        at: Utc::now(),
        source: EventSource::Reflex,
        kind: REFLEX_TICK_LATE_KIND.to_owned(),
        data: json!({
            "code": error_codes::REFLEX_TICK_LATE,
            "elapsed_us": elapsed_us,
            "jitter_us": jitter_us,
            "target_us": duration_us(runtime.config.target_interval),
            "reason": reason,
        }),
        correlations: Vec::new(),
    };
    let _report = runtime.event_bus.publish(event);
}

fn warn_dispatch_blocked(reflex_id: &ReflexId, error: &ReflexError) {
    tracing::warn!(
        component = "reflex_scheduler",
        reflex_id = %reflex_id,
        error_code = error.code(),
        detail = %error,
        "reflex action dispatch blocked"
    );
}

fn push_sample(
    samples: &Arc<Mutex<VecDeque<TickSample>>>,
    sample_limit: usize,
    sample: TickSample,
) {
    let mut samples = lock_samples(samples);
    if samples.len() >= sample_limit {
        let _oldest = samples.pop_front();
    }
    samples.push_back(sample);
}

fn lock_samples(
    samples: &Arc<Mutex<VecDeque<TickSample>>>,
) -> std::sync::MutexGuard<'_, VecDeque<TickSample>> {
    match samples.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn duration_us(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn now_ts_ns() -> u64 {
    Utc::now()
        .timestamp_nanos_opt()
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or_default()
}
