//! Authoritative session-scoped resource teardown (#801).
//!
//! Every resource that can outlive an MCP request should either be owned by an
//! explicit handle or be reclaimed here when the owning `Mcp-Session-Id` ends.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use rmcp::{ErrorData, model::ErrorCode};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::json;
use synapse_action::ActionHandle;
use synapse_core::error_codes;
use synapse_storage::{Db, cf};

use crate::{
    http::sse::SseState,
    m1::mcp_error,
    m3::SharedM3State,
    m4::{self, OwnedProcessJob},
};

use super::{
    CdpTargetOwner, SharedCdpTargetOwners, SharedSessionTargets, SynapseService,
    session_registry::{SharedSessionRegistry, unix_time_ms_now},
};

const MCP_SESSION_STORE_PREFIX: &str = "mcp/session/v1/";
const PROCESS_JOB_CLOSE_WAIT: Duration = Duration::from_secs(5);

pub(crate) type SharedSessionProcessResources =
    Arc<Mutex<BTreeMap<String, BTreeMap<u32, SessionProcessResource>>>>;
pub(crate) type SharedTerminatedSessions = Arc<Mutex<BTreeSet<String>>>;

pub(crate) fn mcp_session_store_key(session_id: &str) -> Vec<u8> {
    format!("{MCP_SESSION_STORE_PREFIX}{session_id}").into_bytes()
}

#[derive(Debug)]
pub(crate) struct SessionProcessResource {
    pub session_id: String,
    pub tool: &'static str,
    pub pid: u32,
    pub registered_at_unix_ms: u64,
    pub resource_id: Option<String>,
    pub launch_target: String,
    pub process_job: Option<OwnedProcessJob>,
}

impl SessionProcessResource {
    pub(crate) fn new(
        session_id: String,
        tool: &'static str,
        pid: u32,
        resource_id: Option<String>,
        launch_target: String,
        process_job: OwnedProcessJob,
    ) -> Self {
        Self {
            session_id,
            tool,
            pid,
            registered_at_unix_ms: unix_time_ms_now(),
            resource_id,
            launch_target,
            process_job: Some(process_job),
        }
    }
}

#[derive(Clone)]
pub(crate) struct SessionLifecycleState {
    action_handle: ActionHandle,
    m3_state: SharedM3State,
    session_targets: SharedSessionTargets,
    cdp_target_owners: SharedCdpTargetOwners,
    session_registry: SharedSessionRegistry,
    session_processes: SharedSessionProcessResources,
    terminated_sessions: SharedTerminatedSessions,
    sse_state: SseState,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionTeardownReport {
    pub session_id: String,
    pub reason: String,
    pub started_at_unix_ms: u64,
    pub finished_at_unix_ms: u64,
    pub already_terminated: bool,
    pub marked_terminated: bool,
    pub termination_marker_failed: bool,
    pub termination_marker_error_message: Option<String>,
    pub input: SessionInputCleanupReport,
    pub target: SessionTargetCleanupReport,
    pub continuity: SessionContinuityCleanupReport,
    pub audit_session: SessionAuditSessionCleanupReport,
    pub cdp: SessionCdpCleanupReport,
    pub shell: SessionShellCleanupReport,
    pub processes: SessionProcessCleanupReport,
    pub subscriptions: SessionSubscriptionCleanupReport,
    pub session_store: SessionStoreCleanupReport,
    pub registry: SessionRegistryCleanupReport,
    pub failure_count: u32,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionInputCleanupReport {
    pub snapshot_read_before: bool,
    pub owned_inputs_before: usize,
    pub lease_held_before: bool,
    pub lease_owner_before: Option<String>,
    pub released_keys: u32,
    pub released_buttons: u32,
    pub neutralized_pads: u32,
    pub retained_shared_inputs: u32,
    pub lease_released: bool,
    pub expired_lease_cleanup_completed: bool,
    pub snapshot_read_after: bool,
    pub owned_inputs_after: usize,
    pub lease_held_after: bool,
    pub lease_owner_after: Option<String>,
    pub failed: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "shutdown FSV readback reports exact before/after lease row booleans"
)]
pub struct SessionShutdownInputCleanupReport {
    pub session_id: String,
    pub reason: String,
    pub input: SessionInputCleanupReport,
    pub lease_row_existed_before: bool,
    pub lease_row_deleted: bool,
    pub lease_row_exists_after: bool,
    pub failed: bool,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionTargetCleanupReport {
    pub target_cleared: bool,
    pub target_sessions_before: usize,
    pub target_sessions_after: usize,
    pub failed: bool,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionContinuityCleanupReport {
    pub target_row_existed_before: bool,
    pub target_row_deleted: bool,
    pub target_row_exists_after: bool,
    pub lease_row_existed_before: bool,
    pub lease_row_deleted: bool,
    pub lease_row_exists_after: bool,
    pub failed: bool,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionAuditSessionCleanupReport {
    pub cache_sessions_before: usize,
    pub cache_sessions_after: usize,
    pub removed: bool,
    pub failed: bool,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionCdpCleanupReport {
    pub owned_before: usize,
    pub closed: usize,
    pub failed: usize,
    pub target_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionShellCleanupReport {
    pub job_root: Option<String>,
    pub status_files_read: usize,
    pub live_jobs_before: usize,
    pub termination_attempted: usize,
    pub termination_succeeded: usize,
    pub failed: usize,
    pub job_ids: Vec<String>,
    pub remaining_process_ids: Vec<u32>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionProcessCleanupReport {
    pub owned_before: usize,
    pub job_close_attempted: usize,
    pub force_termination_attempted: usize,
    pub terminated: usize,
    pub failed: usize,
    pub items: Vec<SessionProcessCleanupItem>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionProcessCleanupItem {
    pub tool: String,
    pub pid: u32,
    pub resource_id: Option<String>,
    pub launch_target: String,
    pub registered_at_unix_ms: u64,
    pub process_ids_before: Vec<u32>,
    pub live_process_ids_before: Vec<u32>,
    pub job_handle_dropped: bool,
    pub force_termination_status: Option<String>,
    pub remaining_process_ids_after: Vec<u32>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionSubscriptionCleanupReport {
    pub owned_before: usize,
    pub cancelled: usize,
    pub subscription_ids: Vec<String>,
    pub failed: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionStoreCleanupReport {
    pub key: String,
    pub existed_before: bool,
    pub deleted: bool,
    pub exists_after: bool,
    pub failed: bool,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionRegistryCleanupReport {
    pub closed_recorded: bool,
    pub reason_code: String,
    pub failed: bool,
    pub error_message: Option<String>,
}

impl SessionTeardownReport {
    fn new(session_id: &str, reason: &str) -> Self {
        let now = unix_time_ms_now();
        Self {
            session_id: session_id.to_owned(),
            reason: reason.to_owned(),
            started_at_unix_ms: now,
            finished_at_unix_ms: now,
            already_terminated: false,
            marked_terminated: false,
            termination_marker_failed: false,
            termination_marker_error_message: None,
            input: SessionInputCleanupReport::default(),
            target: SessionTargetCleanupReport::default(),
            continuity: SessionContinuityCleanupReport::default(),
            audit_session: SessionAuditSessionCleanupReport::default(),
            cdp: SessionCdpCleanupReport::default(),
            shell: SessionShellCleanupReport::default(),
            processes: SessionProcessCleanupReport::default(),
            subscriptions: SessionSubscriptionCleanupReport::default(),
            session_store: SessionStoreCleanupReport::default(),
            registry: SessionRegistryCleanupReport::default(),
            failure_count: 0,
        }
    }

    fn finalize(&mut self) {
        self.finished_at_unix_ms = unix_time_ms_now();
        self.failure_count = 0;
        if self.termination_marker_failed {
            self.failure_count = self.failure_count.saturating_add(1);
        }
        if self.input.failed {
            self.failure_count = self.failure_count.saturating_add(1);
        }
        if self.target.failed {
            self.failure_count = self.failure_count.saturating_add(1);
        }
        if self.continuity.failed {
            self.failure_count = self.failure_count.saturating_add(1);
        }
        if self.audit_session.failed {
            self.failure_count = self.failure_count.saturating_add(1);
        }
        self.failure_count = self
            .failure_count
            .saturating_add(u32::try_from(self.cdp.failed).unwrap_or(u32::MAX));
        self.failure_count = self
            .failure_count
            .saturating_add(u32::try_from(self.shell.failed).unwrap_or(u32::MAX));
        self.failure_count = self
            .failure_count
            .saturating_add(u32::try_from(self.processes.failed).unwrap_or(u32::MAX));
        if self.subscriptions.failed {
            self.failure_count = self.failure_count.saturating_add(1);
        }
        if self.session_store.failed {
            self.failure_count = self.failure_count.saturating_add(1);
        }
        if self.registry.failed {
            self.failure_count = self.failure_count.saturating_add(1);
        }
    }
}

impl SynapseService {
    pub(crate) fn session_lifecycle_state(&self) -> Result<SessionLifecycleState, ErrorData> {
        Ok(SessionLifecycleState {
            action_handle: self.unscoped_action_handle().map_err(|error| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    format!("read unscoped action handle for session lifecycle: {error}"),
                )
            })?,
            m3_state: self.m3_state_handle(),
            session_targets: Arc::clone(&self.session_targets),
            cdp_target_owners: Arc::clone(&self.cdp_target_owners),
            session_registry: Arc::clone(&self.session_registry),
            session_processes: Arc::clone(&self.session_processes),
            terminated_sessions: Arc::clone(&self.terminated_sessions),
            sse_state: self.sse_state()?,
        })
    }

    pub(crate) fn register_session_process_resource(
        &self,
        resource: SessionProcessResource,
    ) -> Result<(), ErrorData> {
        let session_id = resource.session_id.clone();
        let pid = resource.pid;
        let mut guard = self.session_processes.lock().map_err(|_error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "session process resource ledger lock poisoned",
            )
        })?;
        let processes = guard.entry(session_id.clone()).or_default();
        if processes.contains_key(&pid) {
            return Err(ErrorData::new(
                ErrorCode(-32099),
                format!("session process resource already registered for pid {pid}"),
                Some(json!({
                    "code": error_codes::TOOL_INTERNAL_ERROR,
                    "session_id": session_id,
                    "pid": pid,
                    "reason": "duplicate_session_process_resource",
                })),
            ));
        }
        processes.insert(pid, resource);
        tracing::info!(
            code = "MCP_SESSION_PROCESS_RESOURCE_REGISTERED",
            session_id,
            pid,
            "readback=session_process_ledger after=registered"
        );
        Ok(())
    }

    pub(crate) fn terminated_sessions_handle(&self) -> SharedTerminatedSessions {
        Arc::clone(&self.terminated_sessions)
    }
}

impl SessionLifecycleState {
    pub(crate) fn is_session_terminated(&self, session_id: &str) -> bool {
        self.terminated_sessions
            .lock()
            .is_ok_and(|terminated| terminated.contains(session_id))
    }

    pub(crate) async fn teardown_session(
        &self,
        session_id: &str,
        reason: &str,
    ) -> Result<SessionTeardownReport, ErrorData> {
        validate_lifecycle_session_id(session_id)?;
        let mut report = SessionTeardownReport::new(session_id, reason);
        self.mark_terminated_session(&mut report);
        report.input = self.cleanup_inputs_and_lease(session_id).await;
        report.target = self.cleanup_target(session_id);
        report.continuity = self.cleanup_continuity(session_id);
        report.audit_session = self.cleanup_audit_session(session_id);
        report.cdp = cleanup_session_cdp_targets(&self.cdp_target_owners, session_id).await;
        report.shell = cleanup_shell_jobs(session_id, reason);
        report.processes = self.cleanup_owned_processes(session_id);
        report.subscriptions = self.cleanup_subscriptions(session_id);
        report.session_store = self.delete_session_store_row(session_id);
        report.registry = self.record_registry_closed(session_id, reason);
        report.finalize();
        if report.failure_count == 0 {
            tracing::info!(
                code = "MCP_SESSION_TEARDOWN_COMPLETED",
                session_id,
                reason,
                report = ?report,
                "readback=session_lifecycle after=all_owned_resources_reclaimed"
            );
            Ok(report)
        } else {
            tracing::error!(
                code = "MCP_SESSION_TEARDOWN_FAILED",
                session_id,
                reason,
                failure_count = report.failure_count,
                report = ?report,
                "session lifecycle teardown encountered cleanup failures"
            );
            Err(session_teardown_error(report))
        }
    }

    pub(crate) async fn release_session_inputs_for_daemon_shutdown(
        &self,
        session_id: &str,
        reason: &str,
    ) -> SessionShutdownInputCleanupReport {
        let mut report = SessionShutdownInputCleanupReport {
            session_id: session_id.to_owned(),
            reason: reason.to_owned(),
            ..SessionShutdownInputCleanupReport::default()
        };
        if let Err(error) = validate_lifecycle_session_id(session_id) {
            report.failed = true;
            report.error_message = Some(error.message.to_string());
            return report;
        }
        report.input = self.cleanup_inputs_and_lease(session_id).await;
        match super::session_continuity::delete_persisted_session_lease_row(
            &self.m3_state,
            session_id,
        ) {
            Ok(readback) => {
                report.lease_row_existed_before = readback.row_existed_before;
                report.lease_row_deleted = readback.row_deleted;
                report.lease_row_exists_after = readback.row_exists_after;
            }
            Err(error) => {
                report.failed = true;
                report.error_message = Some(error);
            }
        }
        if report.input.failed {
            report.failed = true;
        }
        tracing::info!(
            code = "MCP_SESSION_SHUTDOWN_INPUT_CLEANUP",
            session_id,
            reason,
            report = ?report,
            "readback=session_input_ownership edge=daemon_shutdown after_cleanup"
        );
        report
    }

    pub(crate) async fn cleanup_expired_lease_inputs_once(&self) {
        let pending = synapse_action::lease::expired_cleanup_snapshot();
        for expired in pending {
            let Some(session_id) = expired.owner_session_id.clone() else {
                continue;
            };
            let before = self.action_handle.session_inputs_snapshot();
            let before_lease = synapse_action::lease::status();
            let result = self
                .action_handle
                .release_session_inputs_and_lease(&session_id)
                .await;
            let after = self.action_handle.session_inputs_snapshot();
            let after_lease = synapse_action::lease::status();
            match result {
                Ok(summary) => {
                    tracing::warn!(
                        code = "MCP_SESSION_LEASE_EXPIRED_INPUT_CLEANUP",
                        session_id,
                        released_keys = summary.input_summary.released_keys,
                        released_buttons = summary.input_summary.released_buttons,
                        neutralized_pads = summary.input_summary.neutralized_pads,
                        retained_shared_inputs = summary.input_summary.retained_shared_inputs,
                        input_lease_released = summary.lease_released,
                        expired_lease_cleanup_completed = summary.expired_lease_cleanup_completed,
                        expired = ?expired,
                        before = ?before,
                        after = ?after,
                        before_lease = ?before_lease,
                        after_lease = ?after_lease,
                        "readback=session_input_ownership edge=input_lease_expired after_cleanup"
                    );
                }
                Err(error) => {
                    tracing::error!(
                        code = error.code(),
                        session_id,
                        detail = %error.detail(),
                        expired = ?expired,
                        before = ?before,
                        after = ?after,
                        before_lease = ?before_lease,
                        after_lease = ?after_lease,
                        "session lifecycle expired-lease cleanup failed while releasing owned inputs"
                    );
                }
            }
        }
    }

    pub(crate) fn stale_session_candidates(
        &self,
        active_sessions: &BTreeSet<String>,
    ) -> BTreeSet<String> {
        let mut candidates = BTreeSet::new();
        if let Ok(snapshot) = self.action_handle.session_inputs_snapshot() {
            for session in snapshot.sessions {
                add_if_stale(&mut candidates, active_sessions, &session.session_id);
            }
        }
        let lease_status = synapse_action::lease::status();
        if let Some(owner) = lease_status.owner_session_id.as_ref()
            && owner != synapse_action::OPERATOR_LEASE_OWNER_SESSION_ID
        {
            add_if_stale(&mut candidates, active_sessions, owner);
        }
        if let Ok(targets) = self.session_targets.lock() {
            for session_id in targets.keys() {
                add_if_stale(&mut candidates, active_sessions, session_id);
            }
        }
        if let Ok(state) = self.m3_state.lock() {
            for session_id in state.mcp_audit_sessions.keys() {
                add_if_stale(&mut candidates, active_sessions, session_id);
            }
        }
        if let Ok(owners) = self.cdp_target_owners.lock() {
            for owner in owners.values() {
                add_if_stale(&mut candidates, active_sessions, &owner.session_id);
            }
        }
        if let Ok(processes) = self.session_processes.lock() {
            for session_id in processes.keys() {
                add_if_stale(&mut candidates, active_sessions, session_id);
            }
        }
        if let Ok(session_ids) = self.sse_state.subscription_owner_session_ids() {
            for session_id in session_ids {
                add_if_stale(&mut candidates, active_sessions, &session_id);
            }
        }
        if let Ok(registry) = self.session_registry.lock() {
            for read in registry.reads(unix_time_ms_now()) {
                if read.lifecycle == "stale" {
                    add_if_stale(&mut candidates, active_sessions, &read.session_id);
                }
            }
        }
        candidates
    }

    fn mark_terminated_session(&self, report: &mut SessionTeardownReport) {
        match self.terminated_sessions.lock() {
            Ok(mut terminated) => {
                report.already_terminated = terminated.contains(&report.session_id);
                report.marked_terminated = terminated.insert(report.session_id.clone());
            }
            Err(_error) => {
                report.termination_marker_failed = true;
                report.termination_marker_error_message =
                    Some("terminated-session registry lock poisoned".to_owned());
                tracing::error!(
                    code = error_codes::TOOL_INTERNAL_ERROR,
                    session_id = %report.session_id,
                    "session lifecycle could not lock terminated-session registry"
                );
            }
        }
    }

    async fn cleanup_inputs_and_lease(&self, session_id: &str) -> SessionInputCleanupReport {
        let before_snapshot = self.action_handle.session_inputs_snapshot();
        let before_lease = synapse_action::lease::status();
        let mut report = SessionInputCleanupReport {
            snapshot_read_before: before_snapshot.is_ok(),
            owned_inputs_before: before_snapshot
                .as_ref()
                .map_or(0, |snapshot| owned_input_count(snapshot, session_id)),
            lease_held_before: before_lease.held,
            lease_owner_before: before_lease.owner_session_id.clone(),
            ..SessionInputCleanupReport::default()
        };
        match self
            .action_handle
            .release_session_inputs_and_lease(session_id)
            .await
        {
            Ok(summary) => {
                report.released_keys = summary.input_summary.released_keys;
                report.released_buttons = summary.input_summary.released_buttons;
                report.neutralized_pads = summary.input_summary.neutralized_pads;
                report.retained_shared_inputs = summary.input_summary.retained_shared_inputs;
                report.lease_released = summary.lease_released;
                report.expired_lease_cleanup_completed = summary.expired_lease_cleanup_completed;
            }
            Err(error) => {
                report.failed = true;
                report.error_code = Some(error.code().to_owned());
                report.error_message = Some(error.detail().to_owned());
            }
        }
        let after_snapshot = self.action_handle.session_inputs_snapshot();
        let after_lease = synapse_action::lease::status();
        report.snapshot_read_after = after_snapshot.is_ok();
        report.owned_inputs_after = after_snapshot
            .as_ref()
            .map_or(0, |snapshot| owned_input_count(snapshot, session_id));
        report.lease_held_after = after_lease.held;
        report.lease_owner_after = after_lease.owner_session_id;
        if after_snapshot.is_err() {
            report.failed = true;
            if report.error_message.is_none() {
                report.error_code = Some(error_codes::TOOL_INTERNAL_ERROR.to_owned());
                report.error_message = Some("input ownership after-read failed".to_owned());
            }
        }
        report
    }

    fn cleanup_target(&self, session_id: &str) -> SessionTargetCleanupReport {
        match self.session_targets.lock() {
            Ok(mut targets) => {
                let before = targets.len();
                let target_cleared = targets.remove(session_id).is_some();
                let after = targets.len();
                SessionTargetCleanupReport {
                    target_cleared,
                    target_sessions_before: before,
                    target_sessions_after: after,
                    failed: false,
                    error_message: None,
                }
            }
            Err(_error) => SessionTargetCleanupReport {
                failed: true,
                error_message: Some("session target registry lock poisoned".to_owned()),
                ..SessionTargetCleanupReport::default()
            },
        }
    }

    fn cleanup_continuity(&self, session_id: &str) -> SessionContinuityCleanupReport {
        match super::session_continuity::delete_persisted_session_continuity_rows(
            &self.m3_state,
            session_id,
        ) {
            Ok(readback) => SessionContinuityCleanupReport {
                target_row_existed_before: readback.target_row_existed_before,
                target_row_deleted: readback.target_row_deleted,
                target_row_exists_after: readback.target_row_exists_after,
                lease_row_existed_before: readback.lease_row_existed_before,
                lease_row_deleted: readback.lease_row_deleted,
                lease_row_exists_after: readback.lease_row_exists_after,
                failed: false,
                error_message: None,
            },
            Err(error) => {
                tracing::error!(
                    code = error_codes::TOOL_INTERNAL_ERROR,
                    session_id,
                    detail = %error,
                    "session lifecycle failed to delete persisted continuity rows"
                );
                SessionContinuityCleanupReport {
                    failed: true,
                    error_message: Some(error),
                    ..SessionContinuityCleanupReport::default()
                }
            }
        }
    }

    fn cleanup_audit_session(&self, session_id: &str) -> SessionAuditSessionCleanupReport {
        match self.m3_state.lock() {
            Ok(mut state) => {
                let before = state.mcp_audit_sessions.len();
                let removed = state.mcp_audit_sessions.remove(session_id).is_some();
                let after = state.mcp_audit_sessions.len();
                tracing::info!(
                    code = "MCP_SESSION_AUDIT_CACHE_CLEANUP",
                    session_id,
                    before,
                    after,
                    removed,
                    "readback=m3_state.mcp_audit_sessions after=session_cache_removed"
                );
                SessionAuditSessionCleanupReport {
                    cache_sessions_before: before,
                    cache_sessions_after: after,
                    removed,
                    failed: false,
                    error_message: None,
                }
            }
            Err(_error) => SessionAuditSessionCleanupReport {
                failed: true,
                error_message: Some("M3 service state lock poisoned".to_owned()),
                ..SessionAuditSessionCleanupReport::default()
            },
        }
    }

    fn cleanup_owned_processes(&self, session_id: &str) -> SessionProcessCleanupReport {
        let resources = match self.session_processes.lock() {
            Ok(mut processes) => processes.remove(session_id).unwrap_or_default(),
            Err(_error) => {
                return SessionProcessCleanupReport {
                    failed: 1,
                    ..SessionProcessCleanupReport::default()
                };
            }
        };
        let mut report = SessionProcessCleanupReport {
            owned_before: resources.len(),
            ..SessionProcessCleanupReport::default()
        };
        for mut resource in resources.into_values() {
            let process_ids = m4::owned_process_tree_ids(resource.pid);
            let live_before = m4::owned_live_process_ids(&process_ids);
            let job_handle_dropped = resource.process_job.is_some();
            if job_handle_dropped {
                report.job_close_attempted = report.job_close_attempted.saturating_add(1);
            }
            drop(resource.process_job.take());
            let (mut remaining, _waited_ms) =
                m4::wait_for_owned_process_tree_exit(&process_ids, PROCESS_JOB_CLOSE_WAIT);
            let mut force_termination_status = None;
            if !remaining.is_empty() {
                report.force_termination_attempted =
                    report.force_termination_attempted.saturating_add(1);
                let forced = m4::terminate_owned_process_tree(resource.pid);
                force_termination_status = Some(forced.status);
                remaining = forced.remaining_process_ids;
            }
            if remaining.is_empty() {
                report.terminated = report.terminated.saturating_add(1);
            } else {
                report.failed = report.failed.saturating_add(1);
            }
            report.items.push(SessionProcessCleanupItem {
                tool: resource.tool.to_owned(),
                pid: resource.pid,
                resource_id: resource.resource_id,
                launch_target: resource.launch_target,
                registered_at_unix_ms: resource.registered_at_unix_ms,
                process_ids_before: process_ids,
                live_process_ids_before: live_before,
                job_handle_dropped,
                force_termination_status,
                remaining_process_ids_after: remaining,
            });
        }
        report
    }

    fn cleanup_subscriptions(&self, session_id: &str) -> SessionSubscriptionCleanupReport {
        match self.sse_state.subscription_ids_for_session(session_id) {
            Ok(ids_before) => match self.sse_state.cancel_session_subscriptions(session_id) {
                Ok(cancelled) => SessionSubscriptionCleanupReport {
                    owned_before: ids_before.len(),
                    cancelled: cancelled.len(),
                    subscription_ids: cancelled,
                    failed: false,
                    error_code: None,
                    error_message: None,
                },
                Err(error) => SessionSubscriptionCleanupReport {
                    owned_before: ids_before.len(),
                    failed: true,
                    error_code: Some(error.code().to_owned()),
                    error_message: Some(error.message(session_id)),
                    ..SessionSubscriptionCleanupReport::default()
                },
            },
            Err(error) => SessionSubscriptionCleanupReport {
                failed: true,
                error_code: Some(error.code().to_owned()),
                error_message: Some(error.message(session_id)),
                ..SessionSubscriptionCleanupReport::default()
            },
        }
    }

    fn delete_session_store_row(&self, session_id: &str) -> SessionStoreCleanupReport {
        let key = mcp_session_store_key(session_id);
        let key_string = String::from_utf8_lossy(&key).into_owned();
        let db = match session_store_db(&self.m3_state) {
            Ok(db) => db,
            Err(error) => {
                return SessionStoreCleanupReport {
                    key: key_string,
                    failed: true,
                    error_message: Some(error),
                    ..SessionStoreCleanupReport::default()
                };
            }
        };
        let existed_before = match session_store_row_exists(&db, &key) {
            Ok(exists) => exists,
            Err(error) => {
                return SessionStoreCleanupReport {
                    key: key_string,
                    failed: true,
                    error_message: Some(error),
                    ..SessionStoreCleanupReport::default()
                };
            }
        };
        if existed_before && let Err(error) = db.delete_batch(cf::CF_KV, [key.clone()]) {
            return SessionStoreCleanupReport {
                key: key_string,
                existed_before,
                failed: true,
                error_message: Some(error.to_string()),
                ..SessionStoreCleanupReport::default()
            };
        }
        let exists_after = match session_store_row_exists(&db, &key) {
            Ok(exists) => exists,
            Err(error) => {
                return SessionStoreCleanupReport {
                    key: key_string,
                    existed_before,
                    deleted: existed_before,
                    failed: true,
                    error_message: Some(error),
                    ..SessionStoreCleanupReport::default()
                };
            }
        };
        SessionStoreCleanupReport {
            key: key_string,
            existed_before,
            deleted: existed_before && !exists_after,
            exists_after,
            failed: exists_after,
            error_message: exists_after
                .then(|| "session store row still exists after delete".to_owned()),
        }
    }

    fn record_registry_closed(
        &self,
        session_id: &str,
        reason: &str,
    ) -> SessionRegistryCleanupReport {
        match self.session_registry.lock() {
            Ok(mut registry) => {
                registry.record_closed_with_reason(session_id, unix_time_ms_now(), Some(reason));
                SessionRegistryCleanupReport {
                    closed_recorded: true,
                    reason_code: reason.to_owned(),
                    failed: false,
                    error_message: None,
                }
            }
            Err(_error) => SessionRegistryCleanupReport {
                closed_recorded: false,
                reason_code: reason.to_owned(),
                failed: true,
                error_message: Some("session registry lock poisoned".to_owned()),
            },
        }
    }
}

fn cleanup_shell_jobs(session_id: &str, reason: &str) -> SessionShellCleanupReport {
    match m4::cleanup_shell_jobs_for_session(session_id, reason) {
        Ok(readback) => SessionShellCleanupReport {
            job_root: readback.job_root,
            status_files_read: readback.status_files_read,
            live_jobs_before: readback.live_jobs_before,
            termination_attempted: readback.termination_attempted,
            termination_succeeded: readback.termination_succeeded,
            failed: readback.failed,
            job_ids: readback.job_ids,
            remaining_process_ids: readback.remaining_process_ids,
            error_code: None,
            error_message: None,
        },
        Err(error) => SessionShellCleanupReport {
            failed: 1,
            error_code: error
                .data
                .as_ref()
                .and_then(|data| data.get("code"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            error_message: Some(error.message.to_string()),
            ..SessionShellCleanupReport::default()
        },
    }
}

async fn cleanup_session_cdp_targets(
    cdp_target_owners: &SharedCdpTargetOwners,
    session_id: &str,
) -> SessionCdpCleanupReport {
    let owned = match remove_session_cdp_target_owners(cdp_target_owners, session_id) {
        Ok(owned) => owned,
        Err(detail) => {
            tracing::error!(
                code = error_codes::TOOL_INTERNAL_ERROR,
                session_id,
                detail = %detail,
                "session lifecycle could not lock CDP target ownership registry"
            );
            return SessionCdpCleanupReport {
                failed: 1,
                ..SessionCdpCleanupReport::default()
            };
        }
    };
    let mut report = SessionCdpCleanupReport {
        owned_before: owned.len(),
        target_ids: owned
            .iter()
            .map(|(target_id, _owner)| target_id.clone())
            .collect(),
        ..SessionCdpCleanupReport::default()
    };
    for (target_id, owner) in owned {
        match close_cdp_target_for_cleanup(&target_id, &owner).await {
            Ok(()) => {
                report.closed = report.closed.saturating_add(1);
                tracing::info!(
                    code = "MCP_SESSION_CDP_TARGET_CLEANUP",
                    session_id,
                    hwnd = owner.window_hwnd,
                    endpoint = %owner.endpoint,
                    cdp_target_id = %target_id,
                    "readback=Target.closeTarget edge=session_cleanup after=closed"
                );
            }
            Err(detail) => {
                report.failed = report.failed.saturating_add(1);
                tracing::error!(
                    code = error_codes::A11Y_CDP_AXTREE_FAILED,
                    session_id,
                    hwnd = owner.window_hwnd,
                    endpoint = %owner.endpoint,
                    cdp_target_id = %target_id,
                    detail = %detail,
                    "session lifecycle removed CDP owner but failed to close target"
                );
            }
        }
    }
    report
}

fn remove_session_cdp_target_owners(
    cdp_target_owners: &SharedCdpTargetOwners,
    session_id: &str,
) -> Result<Vec<(String, CdpTargetOwner)>, String> {
    let mut guard = cdp_target_owners
        .lock()
        .map_err(|_error| "CDP target ownership registry lock poisoned".to_owned())?;
    let owned_ids = guard
        .iter()
        .filter_map(|(target_id, owner)| {
            (owner.session_id == session_id).then(|| target_id.clone())
        })
        .collect::<Vec<_>>();
    let owned = owned_ids
        .into_iter()
        .filter_map(|target_id| guard.remove(&target_id).map(|owner| (target_id, owner)))
        .collect();
    Ok(owned)
}

#[cfg(windows)]
async fn close_cdp_target_for_cleanup(
    target_id: &str,
    owner: &CdpTargetOwner,
) -> Result<(), String> {
    synapse_a11y::cdp_close_target(&owner.endpoint, target_id)
        .await
        .map(|_closed| ())
        .map_err(|error| error.to_string())
}

#[cfg(not(windows))]
async fn close_cdp_target_for_cleanup(
    target_id: &str,
    owner: &CdpTargetOwner,
) -> Result<(), String> {
    Err(format!(
        "CDP target cleanup is only available on Windows; target_id={target_id:?} endpoint={:?}",
        owner.endpoint
    ))
}

fn owned_input_count(snapshot: &synapse_action::SessionInputSnapshot, session_id: &str) -> usize {
    snapshot
        .sessions
        .iter()
        .find(|session| session.session_id == session_id)
        .map_or(0, |session| {
            session.keys.len() + session.mouse_buttons.len() + session.pads.len()
        })
}

fn add_if_stale(
    candidates: &mut BTreeSet<String>,
    active_sessions: &BTreeSet<String>,
    session_id: &str,
) {
    if !active_sessions.contains(session_id) {
        candidates.insert(session_id.to_owned());
    }
}

fn session_store_db(m3_state: &SharedM3State) -> Result<Arc<Db>, String> {
    let mut state = m3_state.lock().map_err(|_error| {
        "M3 service state lock poisoned during session-store cleanup".to_owned()
    })?;
    state
        .ensure_storage()
        .map_err(|error| format!("open storage for session-store cleanup: {error}"))
}

fn session_store_row_exists(db: &Db, key: &[u8]) -> Result<bool, String> {
    db.scan_cf_prefix(cf::CF_KV, key)
        .map_err(|error| error.to_string())
        .map(|rows| {
            rows.into_iter()
                .any(|(row_key, _value)| row_key.as_slice() == key)
        })
}

fn validate_lifecycle_session_id(session_id: &str) -> Result<(), ErrorData> {
    if session_id.trim().is_empty() {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "session id must not be empty",
        ));
    }
    if session_id.chars().count() > 512 {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "session id must be at most 512 Unicode scalar values",
        ));
    }
    if !session_id.chars().all(|ch| ('!'..='~').contains(&ch)) {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "session id must contain only visible ASCII characters",
        ));
    }
    Ok(())
}

fn session_teardown_error(report: SessionTeardownReport) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "session teardown for {:?} failed with {} cleanup failure(s)",
            report.session_id, report.failure_count
        ),
        Some(json!({
            "code": error_codes::TOOL_INTERNAL_ERROR,
            "report": report,
        })),
    )
}
