use std::sync::Arc;

use synapse_core::{ReflexState, ReflexStatus};

use crate::{
    MAX_REFLEX_PRIORITY, ReflexCancelOutcome, ReflexError, ReflexResult, ReflexRuntime,
    ScheduledReflex, scheduler,
};

impl ReflexRuntime {
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
        let terminal_ids = self.terminal_runtime_reflex_ids();
        let mut next = self
            .reflexes
            .iter()
            .filter(|reflex| !terminal_ids.contains(&reflex.reflex_id))
            .cloned()
            .collect::<Vec<_>>();
        if let Some(existing) = next
            .iter()
            .find(|existing| same_reflex_definition(existing, reflex))
        {
            return Err(ReflexError::ParamsInvalid {
                detail: format!(
                    "duplicate active reflex registration matches existing reflex {}",
                    existing.reflex_id
                ),
            });
        }
        next.push(reflex.clone());
        scheduler::validate_reflexes(&next)?;

        let new_scheduler = match (self.action_gate.clone(), self.aim_track_target_source.clone()) {
            (Some(action_gate), Some(target_source)) => scheduler::ReflexScheduler::spawn_with_audit_db_context_action_gate_and_aim_track_source(
                self.event_bus.clone(),
                self.action_handle.clone(),
                next.clone(),
                self.scheduler_config.clone(),
                Arc::clone(&self.db),
                self.audit_context.clone(),
                action_gate,
                target_source,
            )?,
            (Some(action_gate), None) => {
                scheduler::ReflexScheduler::spawn_with_audit_db_context_and_action_gate(
                    self.event_bus.clone(),
                    self.action_handle.clone(),
                    next.clone(),
                    self.scheduler_config.clone(),
                    Arc::clone(&self.db),
                    self.audit_context.clone(),
                    action_gate,
                )?
            }
            (None, Some(target_source)) => {
                scheduler::ReflexScheduler::spawn_with_audit_db_context_and_aim_track_source(
                    self.event_bus.clone(),
                    self.action_handle.clone(),
                    next.clone(),
                    self.scheduler_config.clone(),
                    Arc::clone(&self.db),
                    self.audit_context.clone(),
                    target_source,
                )?
            }
            (None, None) => scheduler::ReflexScheduler::spawn_with_audit_db_and_context(
                self.event_bus.clone(),
                self.action_handle.clone(),
                next.clone(),
                self.scheduler_config.clone(),
                Arc::clone(&self.db),
                self.audit_context.clone(),
            )?,
        };
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
            ReflexState::ActionDenied | ReflexState::Expired => {
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
        self.reflexes
            .retain(|reflex| reflex.reflex_id.as_str() != reflex_id);
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

    /// Disables every active scheduler reflex for the operator panic hotkey and
    /// stops the scheduler so no in-flight tick can reassert held input after
    /// the action emitter drains state.
    ///
    /// # Errors
    ///
    /// Returns a [`ReflexError`] when the disabled audit rows cannot be
    /// persisted.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn disable_all_by_operator(&mut self) -> ReflexResult<Vec<ReflexStatus>> {
        self.disable_all_with_reason("operator_hotkey")
    }

    /// Disables every active scheduler reflex for a tool-triggered `release_all`
    /// and stops the scheduler so no in-flight tick can reassert held input
    /// after the action emitter drains state.
    ///
    /// # Errors
    ///
    /// Returns a [`ReflexError`] when stopping the scheduler or writing the
    /// disabled audit rows fails.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn disable_all_for_release_all(&mut self) -> ReflexResult<Vec<ReflexStatus>> {
        self.disable_all_with_reason("release_all")
    }

    fn disable_all_with_reason(&mut self, reason: &'static str) -> ReflexResult<Vec<ReflexStatus>> {
        let Some(scheduler) = self.scheduler.as_mut() else {
            return Ok(Vec::new());
        };
        let disabled = scheduler.disable_all_reflexes();
        if !disabled.is_empty() {
            scheduler.stop()?;
        }
        for status in &disabled {
            self.disabled_reflex_ids.insert(status.id.clone());
        }
        self.write_disabled_audits_with_reason(&disabled, reason)?;
        Ok(disabled)
    }
}

fn same_reflex_definition(left: &ScheduledReflex, right: &ScheduledReflex) -> bool {
    left.trigger == right.trigger
        && left.then == right.then
        && left.driver == right.driver
        && left.priority == right.priority
        && left.lifetime == right.lifetime
        && left.exclusive == right.exclusive
        && left.debounce == right.debounce
}
