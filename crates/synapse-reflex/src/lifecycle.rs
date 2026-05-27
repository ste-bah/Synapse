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
        next.push(reflex.clone());
        scheduler::validate_reflexes(&next)?;

        let new_scheduler = scheduler::ReflexScheduler::spawn_with_audit_db_and_context(
            self.event_bus.clone(),
            self.action_handle.clone(),
            next.clone(),
            self.scheduler_config.clone(),
            Arc::clone(&self.db),
            self.audit_context.clone(),
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
}
