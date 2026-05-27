use std::time::Duration;

use synapse_core::{Event, error_codes};

use super::RuntimeState;
use crate::{
    ReflexError,
    kinds::{
        aim_track::{AimTrackContext, AimTrackOutput},
        combo::{ComboContext, ComboOutput},
        hold_button::{HoldButtonOutput, HoldButtonPhase},
        hold_lifetime::HoldLifetimeContext,
        hold_move::{HoldMoveOutput, HoldMovePhase},
    },
};

pub(super) fn step_stateful_controllers(
    runtime: &mut RuntimeState,
    events: &[Event],
    elapsed: Duration,
    dispatched_actions: &mut usize,
    dispatch_blocked: &mut bool,
) {
    let controls = super::lock_controls(&runtime.controls).clone();
    for index in 0..runtime.reflexes.len() {
        if !controls.get(index).is_some_and(|control| control.active) {
            continue;
        }

        for outcome in [
            step_aim_track(runtime, index, elapsed),
            step_hold_move(runtime, index, events, elapsed),
            step_hold_button(runtime, index, events, elapsed),
            step_combo(runtime, index, elapsed),
        ]
        .into_iter()
        .flatten()
        {
            match outcome {
                StatefulOutcome::Progressed { actions } => {
                    *dispatched_actions = dispatched_actions.saturating_add(actions);
                }
                StatefulOutcome::Fired { actions } => {
                    *dispatched_actions = dispatched_actions.saturating_add(actions);
                    super::mark_reflex_fired(runtime, index);
                }
                StatefulOutcome::Expired { actions, reason } => {
                    *dispatched_actions = dispatched_actions.saturating_add(actions);
                    super::mark_reflex_lifetime_expired(runtime, index, reason);
                }
                StatefulOutcome::Idle => {}
                StatefulOutcome::Blocked { error } => {
                    *dispatch_blocked = true;
                    super::mark_reflex_error(runtime, index, error.code());
                    warn_stateful_dispatch_blocked(index, &error);
                    return;
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
enum StatefulOutcome {
    Progressed {
        actions: usize,
    },
    Fired {
        actions: usize,
    },
    Expired {
        actions: usize,
        reason: &'static str,
    },
    Idle,
    Blocked {
        error: ReflexError,
    },
}

fn step_combo(
    runtime: &mut RuntimeState,
    index: usize,
    elapsed: Duration,
) -> Option<StatefulOutcome> {
    let controller = runtime.combo_states.get_mut(index)?.as_mut()?;
    let context = ComboContext {
        tick_elapsed: elapsed,
    };
    match controller.step_dispatch(&context, &runtime.action_handle, &runtime.event_bus) {
        Ok(ComboOutput::Completed { actions, .. }) => Some(StatefulOutcome::Expired {
            actions,
            reason: "completed",
        }),
        Ok(output) if output.action_count() > 0 => Some(StatefulOutcome::Progressed {
            actions: output.action_count(),
        }),
        Ok(
            ComboOutput::Idle { .. } | ComboOutput::Started { .. } | ComboOutput::Dispatched { .. },
        ) => Some(StatefulOutcome::Idle),
        Err(error) => Some(StatefulOutcome::Blocked { error }),
    }
}

fn step_aim_track(
    runtime: &mut RuntimeState,
    index: usize,
    elapsed: Duration,
) -> Option<StatefulOutcome> {
    let controller = runtime.aim_track_states.get_mut(index)?.as_mut()?;
    let cursor = match synapse_action::backend::software::cursor_position() {
        Ok(cursor) => cursor,
        Err(error) => {
            return Some(StatefulOutcome::Blocked {
                error: ReflexError::ParamsInvalid {
                    detail: format!("aim_track cursor read failed: {error}"),
                },
            });
        }
    };
    let context = AimTrackContext {
        cursor,
        entities: &[],
        elements: &[],
        tick_index: runtime.tick_index,
        tick_elapsed: elapsed,
    };
    match controller.step_dispatch(&context, &runtime.action_handle, &runtime.event_bus) {
        Ok(AimTrackOutput::Dispatched { .. }) => Some(StatefulOutcome::Fired { actions: 1 }),
        Ok(AimTrackOutput::Idle { .. }) => Some(StatefulOutcome::Idle),
        Err(ReflexError::TrackLost { .. }) => Some(StatefulOutcome::Blocked {
            error: ReflexError::TrackLost {
                reflex_id: runtime.reflexes[index].reflex.reflex_id.clone(),
            },
        }),
        Err(error) => Some(StatefulOutcome::Blocked { error }),
    }
}

fn step_hold_move(
    runtime: &mut RuntimeState,
    index: usize,
    events: &[Event],
    elapsed: Duration,
) -> Option<StatefulOutcome> {
    let controller = runtime.hold_move_states.get_mut(index)?.as_mut()?;
    let mut actions = 0_usize;
    let mut registered = false;
    if matches!(controller.phase(), HoldMovePhase::Pending) {
        match controller.register_dispatch(&runtime.action_handle) {
            Ok(HoldMoveOutput::Registered {
                actions: registered_actions,
            }) => {
                actions = actions.saturating_add(registered_actions);
                registered = true;
            }
            Ok(
                HoldMoveOutput::Holding { .. }
                | HoldMoveOutput::Released { .. }
                | HoldMoveOutput::Idle { .. },
            ) => {}
            Err(error) => return Some(StatefulOutcome::Blocked { error }),
        }
    }

    let context = HoldLifetimeContext {
        tick_elapsed: elapsed,
        events,
        cancelled: false,
    };
    match controller.step_dispatch(&context, &runtime.action_handle, &runtime.event_bus) {
        Ok(
            HoldMoveOutput::Holding { .. }
            | HoldMoveOutput::Idle { .. }
            | HoldMoveOutput::Registered { .. },
        ) if registered => Some(StatefulOutcome::Fired { actions }),
        Ok(
            HoldMoveOutput::Holding { .. }
            | HoldMoveOutput::Idle { .. }
            | HoldMoveOutput::Registered { .. },
        ) => Some(StatefulOutcome::Idle),
        Ok(HoldMoveOutput::Released {
            actions: released_actions,
            ..
        }) => Some(StatefulOutcome::Expired {
            actions: actions.saturating_add(released_actions),
            reason: "released",
        }),
        Err(ReflexError::LifetimeExpired { .. }) => Some(StatefulOutcome::Expired {
            actions: actions.saturating_add(controller.params().keys.len()),
            reason: "lifetime",
        }),
        Err(error) => Some(StatefulOutcome::Blocked { error }),
    }
}

fn step_hold_button(
    runtime: &mut RuntimeState,
    index: usize,
    events: &[Event],
    elapsed: Duration,
) -> Option<StatefulOutcome> {
    let controller = runtime.hold_button_states.get_mut(index)?.as_mut()?;
    let mut actions = 0_usize;
    let mut registered = false;
    if matches!(controller.phase(), HoldButtonPhase::Pending) {
        match controller.register_dispatch(&runtime.action_handle) {
            Ok(HoldButtonOutput::Registered) => {
                actions = actions.saturating_add(1);
                registered = true;
            }
            Ok(
                HoldButtonOutput::Holding { .. }
                | HoldButtonOutput::Released { .. }
                | HoldButtonOutput::Idle { .. },
            ) => {}
            Err(error) => return Some(StatefulOutcome::Blocked { error }),
        }
    }

    let context = HoldLifetimeContext {
        tick_elapsed: elapsed,
        events,
        cancelled: false,
    };
    match controller.step_dispatch(&context, &runtime.action_handle, &runtime.event_bus) {
        Ok(
            HoldButtonOutput::Holding { .. }
            | HoldButtonOutput::Idle { .. }
            | HoldButtonOutput::Registered,
        ) if registered => Some(StatefulOutcome::Fired { actions }),
        Ok(
            HoldButtonOutput::Holding { .. }
            | HoldButtonOutput::Idle { .. }
            | HoldButtonOutput::Registered,
        ) => Some(StatefulOutcome::Idle),
        Ok(HoldButtonOutput::Released { .. }) => Some(StatefulOutcome::Expired {
            actions: actions.saturating_add(1),
            reason: "released",
        }),
        Err(ReflexError::LifetimeExpired { .. }) => Some(StatefulOutcome::Expired {
            actions: actions.saturating_add(1),
            reason: "lifetime",
        }),
        Err(error) => Some(StatefulOutcome::Blocked { error }),
    }
}

fn warn_stateful_dispatch_blocked(index: usize, error: &ReflexError) {
    tracing::warn!(
        component = "reflex_scheduler",
        reflex_index = index,
        error_code = error.code(),
        detail = %error,
        code = error_codes::REFLEX_TICK_LATE,
        "reflex stateful controller dispatch blocked"
    );
}
