use std::{error::Error, io, time::Duration};

use serde_json::json;
use synapse_action::{ACTION_QUEUE_CAPACITY, ActionHandle, ActionMessage};
use synapse_core::{
    Action, Backend, Event, EventFilter, EventSource, ReflexLifetime, ReflexState, SCHEMA_VERSION,
    StoredReflexAudit, error_codes,
};
use synapse_reflex::{
    DEFAULT_REFLEX_PRIORITY, EventBus, REFLEX_LIFETIME_EXPIRED_KIND, REFLEX_RECURSION_LIMIT_KIND,
    REFLEX_STARVED_KIND, REFLEX_TICK_LATE_KIND, ReflexScheduler, ScheduledReflex, SchedulerConfig,
    SchedulerTrigger,
};
use synapse_storage::{Db, cf, decode_json};
use tempfile::tempdir;

const WAIT_TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn zero_reflexes_tick_fires_without_dispatch() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let (action_handle, action_rx) = ActionHandle::channel();
    assert_eq!(action_rx.len(), 0);

    let mut scheduler = ReflexScheduler::spawn(
        bus,
        action_handle,
        Vec::new(),
        SchedulerConfig::default().with_max_ticks(24),
    )?;
    let samples = scheduler.wait_for_samples(24, WAIT_TIMEOUT);
    scheduler.stop()?;

    assert_eq!(samples.len(), 24);
    assert!(samples.iter().all(|sample| sample.dispatched_actions == 0));
    assert_eq!(action_rx.len(), 0);
    Ok(())
}

#[test]
fn on_event_reflex_pulls_bus_event_and_dispatches() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let (action_handle, action_rx) = ActionHandle::channel();
    let reflex = ScheduledReflex::on_event(
        "reflex-on-event",
        EventFilter::Kind {
            kind: "wanted".to_owned(),
        },
        vec![Action::ReleaseAll],
    );
    assert_eq!(action_rx.len(), 0);
    let mut scheduler = ReflexScheduler::spawn(
        bus.clone(),
        action_handle,
        vec![reflex],
        SchedulerConfig::default().with_max_ticks(8),
    )?;
    let _report = bus.publish(event(1, "wanted"));
    let samples = scheduler.wait_for_samples(8, WAIT_TIMEOUT);
    scheduler.stop()?;

    let pulled = samples
        .iter()
        .map(|sample| sample.pulled_events)
        .sum::<usize>();
    let dispatched = samples
        .iter()
        .map(|sample| sample.dispatched_actions)
        .sum::<usize>();

    assert!(pulled >= 1);
    assert_eq!(dispatched, 1);
    assert_eq!(action_rx.len(), 1);
    Ok(())
}

#[test]
fn one_shot_on_event_expires_after_first_fire() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let db = std::sync::Arc::new(Db::open(&temp.path().join("db"), SCHEMA_VERSION)?);
    let bus = EventBus::default();
    let (action_handle, action_rx) = ActionHandle::channel();
    let reflex = ScheduledReflex::on_event(
        "reflex-one-shot",
        EventFilter::Kind {
            kind: "once".to_owned(),
        },
        vec![Action::ReleaseAll],
    )
    .with_lifetime(ReflexLifetime::OneShot);
    assert_eq!(action_rx.len(), 0);

    let mut scheduler = ReflexScheduler::spawn_with_audit_db(
        bus.clone(),
        action_handle,
        vec![reflex],
        slow_ticks_config(16),
        std::sync::Arc::clone(&db),
    )?;
    let _report = bus.publish(event(1, "once"));
    assert!(wait_for_status(
        &scheduler,
        "reflex-one-shot",
        ReflexState::Expired,
        WAIT_TIMEOUT
    ));
    let _report = bus.publish(event(2, "once"));
    let samples = scheduler.wait_for_samples(16, WAIT_TIMEOUT);
    scheduler.stop()?;
    db.flush()?;

    let statuses = scheduler.statuses();
    let status = status(&statuses, "reflex-one-shot")?;
    let audits = read_audits(&db)?;
    assert_eq!(samples.len(), 16);
    assert_eq!(action_rx.len(), 1);
    assert_eq!(status.state, ReflexState::Expired);
    assert_eq!(status.fire_count, 1);
    assert!(audits.iter().any(|audit| {
        audit.status == ReflexState::Expired
            && audit.details["kind"].as_str() == Some(REFLEX_LIFETIME_EXPIRED_KIND)
    }));
    Ok(())
}

#[test]
fn on_event_recursion_guard_limits_same_tick_firings_and_audits() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let db = std::sync::Arc::new(Db::open(&temp.path().join("db"), SCHEMA_VERSION)?);
    let bus = EventBus::default();
    let recursion_events = bus.subscribe(
        EventFilter::Kind {
            kind: REFLEX_RECURSION_LIMIT_KIND.to_owned(),
        },
        Vec::new(),
        false,
    )?;
    let (action_handle, action_rx) = ActionHandle::channel();
    let reflex = ScheduledReflex::on_event(
        "reflex-recursion",
        EventFilter::Kind {
            kind: "loop".to_owned(),
        },
        vec![Action::ReleaseAll],
    );
    let mut scheduler = ReflexScheduler::spawn_with_audit_db(
        bus.clone(),
        action_handle,
        vec![reflex],
        slow_one_tick_config(),
        std::sync::Arc::clone(&db),
    )?;
    for seq in 1..=5 {
        let _report = bus.publish(event(seq, "loop"));
    }
    let samples = scheduler.wait_for_samples(1, WAIT_TIMEOUT);
    scheduler.stop()?;
    db.flush()?;

    let audits = db
        .scan_cf(cf::CF_REFLEX_AUDIT)?
        .iter()
        .map(|(_key, value)| decode_json::<StoredReflexAudit>(value))
        .collect::<Result<Vec<_>, _>>()?;
    let fired = audits
        .iter()
        .filter(|audit| audit.error_code.is_none())
        .count();
    let limited = audits
        .iter()
        .filter(|audit| audit.error_code.as_deref() == Some(error_codes::REFLEX_RECURSION_LIMIT))
        .count();

    assert_eq!(samples.len(), 1);
    assert_eq!(action_rx.len(), 4);
    assert_eq!(recursion_events.drain().len(), 1);
    assert_eq!(fired, 4);
    assert_eq!(limited, 1);
    Ok(())
}

#[test]
fn on_event_debounce_suppresses_same_tick_duplicates() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let (action_handle, action_rx) = ActionHandle::channel();
    let reflex = ScheduledReflex::on_event_with_debounce(
        "reflex-debounced",
        EventFilter::Kind {
            kind: "debounced".to_owned(),
        },
        vec![Action::ReleaseAll],
        Duration::from_secs(1),
    );
    let mut scheduler = ReflexScheduler::spawn(
        bus.clone(),
        action_handle,
        vec![reflex],
        slow_one_tick_config(),
    )?;
    let _report = bus.publish(event(1, "debounced"));
    let _report = bus.publish(event(2, "debounced"));
    let samples = scheduler.wait_for_samples(1, WAIT_TIMEOUT);
    scheduler.stop()?;

    assert_eq!(samples.len(), 1);
    assert_eq!(action_rx.len(), 1);
    Ok(())
}

#[test]
fn thirty_two_reflexes_fire_same_tick_without_tick_late() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let late_events = bus.subscribe(
        EventFilter::Kind {
            kind: REFLEX_TICK_LATE_KIND.to_owned(),
        },
        Vec::new(),
        false,
    )?;
    let (action_handle, action_rx) = ActionHandle::channel();
    let reflexes = (0..32)
        .map(|index| {
            ScheduledReflex::every_tick(format!("reflex-{index:02}"), vec![Action::ReleaseAll])
        })
        .collect::<Vec<_>>();
    assert_eq!(reflexes.len(), 32);
    assert_eq!(action_rx.len(), 0);
    assert_eq!(late_events.len(), 0);

    let mut scheduler = ReflexScheduler::spawn(
        bus,
        action_handle,
        reflexes,
        SchedulerConfig::default().with_max_ticks(1),
    )?;
    let samples = scheduler.wait_for_samples(1, WAIT_TIMEOUT);
    scheduler.stop()?;

    let late = late_events.drain();
    let Some(sample) = samples.first().copied() else {
        return Err(Box::new(io::Error::other(
            "scheduler did not record the expected tick sample",
        )));
    };

    assert_eq!(sample.dispatched_actions, 32);
    assert!(!sample.late);
    assert_eq!(action_rx.len(), 32);
    assert!(late.is_empty());
    Ok(())
}

#[test]
fn blocked_dispatch_path_emits_reflex_tick_late() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let late_events = bus.subscribe(
        EventFilter::Kind {
            kind: REFLEX_TICK_LATE_KIND.to_owned(),
        },
        Vec::new(),
        false,
    )?;
    let (action_handle, action_rx) = ActionHandle::channel();
    for _ in 0..ACTION_QUEUE_CAPACITY {
        action_handle.try_execute(Action::ReleaseAll)?;
    }
    assert_eq!(action_rx.len(), ACTION_QUEUE_CAPACITY);
    assert_eq!(late_events.len(), 0);

    let reflex = ScheduledReflex::every_tick("reflex-blocked", vec![Action::ReleaseAll]);
    let mut scheduler = ReflexScheduler::spawn(
        bus,
        action_handle,
        vec![reflex],
        SchedulerConfig::default().with_max_ticks(1),
    )?;
    let samples = scheduler.wait_for_samples(1, WAIT_TIMEOUT);
    scheduler.stop()?;

    let late = late_events.drain();
    let Some(sample) = samples.first().copied() else {
        return Err(Box::new(io::Error::other(
            "scheduler did not record blocked-dispatch sample",
        )));
    };

    assert_eq!(action_rx.len(), ACTION_QUEUE_CAPACITY);
    assert_eq!(sample.dispatched_actions, 0);
    assert!(sample.late);
    assert_eq!(late.len(), 1);
    assert_eq!(late[0].data["code"], error_codes::REFLEX_TICK_LATE);
    assert_eq!(late[0].data["reason"], "dispatch_blocked");
    Ok(())
}

#[test]
fn lower_priority_number_wins_cursor_conflict_and_starves_loser() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let db = std::sync::Arc::new(Db::open(&temp.path().join("db"), SCHEMA_VERSION)?);
    let bus = EventBus::default();
    let starved_events = bus.subscribe(
        EventFilter::Kind {
            kind: REFLEX_STARVED_KIND.to_owned(),
        },
        Vec::new(),
        false,
    )?;
    let (action_handle, mut action_rx) = ActionHandle::channel();
    let mut winner = mouse_move_reflex("reflex-priority-winner", 10.0);
    winner.priority = 10;
    let mut loser = mouse_move_reflex("reflex-priority-loser", 99.0);
    loser.priority = 100;

    let mut scheduler = ReflexScheduler::spawn_with_audit_db(
        bus,
        action_handle,
        vec![winner, loser],
        slow_ticks_config(45),
        std::sync::Arc::clone(&db),
    )?;
    let samples = scheduler.wait_for_samples(45, WAIT_TIMEOUT);
    scheduler.stop()?;
    db.flush()?;

    let actions = drain_actions(&mut action_rx);
    let statuses = scheduler.statuses();
    let audits = read_audits(&db)?;
    let starved_audits = audits
        .iter()
        .filter(|audit| audit.error_code.as_deref() == Some(error_codes::REFLEX_STARVED))
        .count();

    assert_eq!(samples.len(), 45);
    assert_eq!(actions.len(), 45);
    assert!(actions.iter().all(|action| matches!(
        action,
        Action::MouseMoveRelative { dx, .. } if dx_is(*dx, 10.0)
    )));
    assert_eq!(starved_events.drain().len(), 1);
    assert_eq!(starved_audits, 1);
    let loser_status = status(&statuses, "reflex-priority-loser")?;
    assert_eq!(loser_status.state, ReflexState::Starved);
    assert_eq!(
        loser_status.last_error_code.as_deref(),
        Some(error_codes::REFLEX_STARVED)
    );
    let winner_status = status(&statuses, "reflex-priority-winner")?;
    assert_eq!(winner_status.fire_count, 45);
    Ok(())
}

#[test]
fn equal_priority_cursor_conflict_prefers_newer_registration() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let (action_handle, mut action_rx) = ActionHandle::channel();
    let mut older = mouse_move_reflex("reflex-older", 1.0);
    older.priority = 50;
    let mut newer = mouse_move_reflex("reflex-newer", 2.0);
    newer.priority = 50;

    let mut scheduler = ReflexScheduler::spawn(
        bus,
        action_handle,
        vec![older, newer],
        slow_ticks_config(45),
    )?;
    let samples = scheduler.wait_for_samples(45, WAIT_TIMEOUT);
    scheduler.stop()?;

    let actions = drain_actions(&mut action_rx);
    let statuses = scheduler.statuses();

    assert_eq!(samples.len(), 45);
    assert_eq!(actions.len(), 45);
    assert!(actions.iter().all(|action| matches!(
        action,
        Action::MouseMoveRelative { dx, .. } if dx_is(*dx, 2.0)
    )));
    assert_eq!(
        status(&statuses, "reflex-older")?.state,
        ReflexState::Starved
    );
    assert_eq!(status(&statuses, "reflex-newer")?.fire_count, 45);
    Ok(())
}

#[test]
fn priority_change_is_used_on_later_ticks() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let (action_handle, mut action_rx) = ActionHandle::channel();
    let mut first = mouse_move_reflex("reflex-first", 1.0);
    first.priority = 10;
    let mut second = mouse_move_reflex("reflex-second", 2.0);
    second.priority = 100;

    let mut scheduler = ReflexScheduler::spawn(
        bus,
        action_handle,
        vec![first, second],
        slow_ticks_config(8),
    )?;
    let before = scheduler.wait_for_samples(2, WAIT_TIMEOUT);
    assert!(before.len() >= 2);
    assert!(scheduler.set_priority("reflex-second", 1));
    let samples = scheduler.wait_for_samples(8, WAIT_TIMEOUT);
    scheduler.stop()?;

    let actions = drain_actions(&mut action_rx);
    assert_eq!(samples.len(), 8);
    assert!(actions.iter().any(|action| matches!(
        action,
        Action::MouseMoveRelative { dx, .. } if dx_is(*dx, 1.0)
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        Action::MouseMoveRelative { dx, .. } if dx_is(*dx, 2.0)
    )));
    assert_eq!(status(&scheduler.statuses(), "reflex-second")?.priority, 1);
    Ok(())
}

#[test]
fn cancelling_winner_allows_starved_loser_to_fire_again() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let (action_handle, mut action_rx) = ActionHandle::channel();
    let mut winner = mouse_move_reflex("reflex-cancel-winner", 3.0);
    winner.priority = 1;
    let mut loser = mouse_move_reflex("reflex-resumes", 4.0);
    loser.priority = 100;

    let mut scheduler = ReflexScheduler::spawn(
        bus,
        action_handle,
        vec![winner, loser],
        slow_ticks_config(55),
    )?;
    let starved = wait_for_status(
        &scheduler,
        "reflex-resumes",
        ReflexState::Starved,
        WAIT_TIMEOUT,
    );
    assert!(starved);
    assert!(scheduler.cancel_reflex("reflex-cancel-winner"));
    let samples = scheduler.wait_for_samples(55, WAIT_TIMEOUT);
    scheduler.stop()?;

    let actions = drain_actions(&mut action_rx);
    let statuses = scheduler.statuses();
    assert_eq!(samples.len(), 55);
    assert!(actions.iter().any(|action| matches!(
        action,
        Action::MouseMoveRelative { dx, .. } if dx_is(*dx, 4.0)
    )));
    assert_eq!(
        status(&statuses, "reflex-cancel-winner")?.state,
        ReflexState::Cancelled
    );
    assert_eq!(
        status(&statuses, "reflex-resumes")?.state,
        ReflexState::Active
    );
    Ok(())
}

#[test]
fn scheduler_rejects_invalid_trigger_filter() {
    let bus = EventBus::default();
    let (action_handle, action_rx) = ActionHandle::channel();
    let reflex = ScheduledReflex {
        reflex_id: "reflex-invalid-filter".to_owned(),
        trigger: SchedulerTrigger::OnEvent(EventFilter::And { args: Vec::new() }),
        then: vec![Action::ReleaseAll],
        priority: 0,
        lifetime: ReflexLifetime::UntilCancelled,
        exclusive: false,
        debounce: Duration::ZERO,
    };
    assert_eq!(action_rx.len(), 0);

    let error = match ReflexScheduler::spawn(
        bus,
        action_handle,
        vec![reflex],
        SchedulerConfig::default(),
    ) {
        Ok(_scheduler) => panic!("invalid event filter must prevent scheduler spawn"),
        Err(error) => error,
    };

    assert_eq!(error.code(), error_codes::REFLEX_FILTER_INVALID);
    assert_eq!(action_rx.len(), 0);
}

fn event(seq: u64, kind: &str) -> Event {
    Event {
        seq,
        at: chrono::Utc::now(),
        source: EventSource::System,
        kind: kind.to_owned(),
        data: json!({ "seq": seq, "kind": kind }),
        correlations: Vec::new(),
    }
}

const fn slow_one_tick_config() -> SchedulerConfig {
    SchedulerConfig {
        target_interval: Duration::from_millis(50),
        fallback_interval: Duration::from_millis(50),
        late_after: Duration::from_millis(250),
        sample_limit: 16,
        max_ticks: Some(1),
        force_degraded: false,
    }
}

const fn slow_ticks_config(max_ticks: u64) -> SchedulerConfig {
    SchedulerConfig {
        target_interval: Duration::from_millis(50),
        fallback_interval: Duration::from_millis(50),
        late_after: Duration::from_millis(250),
        sample_limit: 128,
        max_ticks: Some(max_ticks),
        force_degraded: false,
    }
}

fn mouse_move_reflex(id: &str, dx: f32) -> ScheduledReflex {
    ScheduledReflex::every_tick(
        id,
        vec![Action::MouseMoveRelative {
            dx,
            dy: 0.0,
            backend: Backend::Software,
        }],
    )
}

fn drain_actions(action_rx: &mut tokio::sync::mpsc::Receiver<ActionMessage>) -> Vec<Action> {
    let mut actions = Vec::new();
    while let Ok((action, _ack)) = action_rx.try_recv() {
        actions.push(action);
    }
    actions
}

const fn dx_is(actual: f32, expected: f32) -> bool {
    actual.to_bits() == expected.to_bits()
}

fn read_audits(db: &Db) -> Result<Vec<StoredReflexAudit>, Box<dyn Error>> {
    db.scan_cf(cf::CF_REFLEX_AUDIT)?
        .iter()
        .map(|(_key, value)| decode_json::<StoredReflexAudit>(value))
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn status<'a>(
    statuses: &'a [synapse_core::ReflexStatus],
    id: &str,
) -> Result<&'a synapse_core::ReflexStatus, Box<dyn Error>> {
    statuses
        .iter()
        .find(|status| status.id == id)
        .ok_or_else(|| Box::new(io::Error::other(format!("missing status {id}"))) as Box<dyn Error>)
}

fn wait_for_status(
    scheduler: &synapse_reflex::SchedulerHandle,
    id: &str,
    expected: ReflexState,
    timeout: Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if scheduler
            .statuses()
            .iter()
            .any(|status| status.id == id && status.state == expected)
        {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn scheduled_reflex_default_priority_matches_priority_adr() {
    let reflex = ScheduledReflex::every_tick("reflex-default-priority", vec![Action::ReleaseAll]);

    assert_eq!(reflex.priority, DEFAULT_REFLEX_PRIORITY);
    assert_eq!(DEFAULT_REFLEX_PRIORITY, 100);
}
