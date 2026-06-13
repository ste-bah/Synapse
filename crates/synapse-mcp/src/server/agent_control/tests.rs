//! Tests for the `agent_interrupt` / `agent_kill` verbs (#904).
//!
//! The deterministic helpers are unit-tested; the acceptance behaviour
//! (force-killing a real process tree to zero orphans, journaling, command
//! audit, double-kill idempotence, cooperative interrupt delivery) is verified
//! against a REAL spawned OS process through the real code path with the OS
//! process table and the storage column families as the sources of truth — no
//! mocks. The owning Windows job (KILL_ON_JOB_CLOSE) guarantees no orphan
//! survives even if an assertion fails, because the service drop closes it.

use super::*;

use std::num::NonZeroUsize;
use std::path::Path;
use std::process::Command as StdCommand;

use synapse_storage::{Db, cf};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use crate::m2::M2ServiceConfig;
use crate::m3::M3ServiceConfig;
use crate::m4::M4ServiceConfig;
use crate::server::session_lifecycle::SessionProcessResource;
use crate::server::session_registry::SpawnedAgentRead;

// ---------------------------------------------------------------------------
// Deterministic unit tests
// ---------------------------------------------------------------------------

#[test]
fn validate_lookup_id_trims_and_rejects_empty() {
    assert_eq!(
        validate_lookup_id("  agent-spawn-123  ", TOOL_AGENT_KILL).unwrap(),
        "agent-spawn-123"
    );
    let err = validate_lookup_id("   ", TOOL_AGENT_INTERRUPT).unwrap_err();
    assert!(
        err.message.contains("must be a non-empty"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn kill_params_defaults_are_graceful() {
    let params: AgentKillParams =
        serde_json::from_value(json!({ "session_id": "s-1" })).expect("defaults parse");
    assert_eq!(params.grace_ms, DEFAULT_KILL_GRACE_MS);
    assert!(params.interrupt_first);
}

#[test]
fn kill_params_reject_unknown_fields() {
    let result: Result<AgentKillParams, _> =
        serde_json::from_value(json!({ "session_id": "s-1", "bogus": true }));
    assert!(result.is_err(), "deny_unknown_fields must reject extra keys");
}

#[test]
fn interrupt_params_reject_unknown_fields() {
    let result: Result<AgentInterruptParams, _> =
        serde_json::from_value(json!({ "session_id": "s-1", "grace_ms": 10 }));
    assert!(
        result.is_err(),
        "agent_interrupt takes no grace_ms; unknown fields must be rejected"
    );
}

#[test]
fn process_readback_of_dead_pid_reports_no_live_processes() {
    // An impossible pid owns no live process tree — the OS process table (the
    // source of truth) backs this, so `live_process_ids` must be empty.
    let readback = process_readback(0xFFFF_FFFE);
    assert!(
        readback.live_process_ids.is_empty(),
        "a non-existent pid must have zero live processes, got {:?}",
        readback.live_process_ids
    );
}

#[tokio::test]
async fn wait_for_tree_exit_returns_immediately_for_empty_tree() {
    let (remaining, waited) = wait_for_tree_exit_async(&[], 5_000).await;
    assert!(remaining.is_empty(), "no pids means nothing remains alive");
    assert!(
        waited < 1_000,
        "an already-empty tree must not burn the grace window, waited {waited}ms"
    );
}

#[tokio::test]
async fn wait_for_tree_exit_reports_survivors_after_grace() {
    // The current process is alive, so `owned_live_process_ids` reports it as a
    // survivor — proving the timeout path returns the still-live pid rather than
    // looping forever.
    let me = std::process::id();
    let (remaining, _waited) = wait_for_tree_exit_async(&[me], 150).await;
    assert_eq!(
        remaining,
        vec![me],
        "a live pid must be reported as a survivor after the grace window"
    );
}

// ---------------------------------------------------------------------------
// Real-process Full-State-Verification (#904 acceptance)
// ---------------------------------------------------------------------------

fn fsv_service(path: &Path) -> SynapseService {
    SynapseService::try_with_m2_shutdown_reason_and_m3_config(
        CancellationToken::new(),
        "test",
        CancellationToken::new(),
        &M2ServiceConfig::default(),
        M3ServiceConfig::from_cli_parts(
            Some(path.join("db")),
            Some(path.to_path_buf()),
            false,
            "127.0.0.1:0".to_owned(),
            NonZeroUsize::new(4).expect("nonzero"),
            false,
            true,
            None,
            false,
            None,
        ),
        M4ServiceConfig::default(),
    )
    .expect("construct storage-backed service")
}

/// Spawns a benign long-lived process (a 600s sleep) and returns its pid.
/// Killed by the owning job at the latest when the service drops.
#[expect(
    clippy::zombie_processes,
    reason = "the victim's lifecycle is owned by the Windows job object \
              (KILL_ON_JOB_CLOSE / TerminateJobObject via the kill path under test), \
              not by the std::process::Child handle, which we keep only for its pid"
)]
fn spawn_victim() -> u32 {
    let child = StdCommand::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 600",
        ])
        .spawn()
        .expect("spawn victim process");
    child.id()
}

/// Registers a spawned agent (registry row + owned process resource) exactly the
/// way act_spawn_agent does, keyed by the agent's own session id.
fn register_spawned_victim(service: &SynapseService, session_id: &str, spawn_id: &str, pid: u32) {
    let now = unix_time_ms_now();
    {
        let mut registry = service
            .session_registry_ref()
            .lock()
            .expect("registry lock");
        registry.record_seen(session_id, Some("test".to_owned()), now);
        registry.record_spawned_agent(
            session_id,
            SpawnedAgentRead {
                spawn_id: spawn_id.to_owned(),
                cli: "local-model".to_owned(),
                launcher_process_id: pid,
                agent_process_id: Some(pid),
                started_by_session_id: Some("operator-fsv".to_owned()),
                launched_at_unix_ms: now,
                launch_target: "powershell.exe".to_owned(),
                log_dir: "C:\\temp\\fsv".to_owned(),
                template_id: None,
                template_version: None,
            },
            now,
        );
    }
    let job = crate::m4::assign_owned_process_job(pid, "act_spawn_agent", Some(spawn_id))
        .expect("assign owned job to victim");
    service
        .register_session_process_resource(
            SessionProcessResource::new(
                session_id.to_owned(),
                "act_spawn_agent",
                pid,
                Some(spawn_id.to_owned()),
                "powershell.exe".to_owned(),
                job,
            )
            .with_agent_cli("local-model"),
        )
        .expect("register session process resource");
}

fn journal_count_kind(db: &Db, session_id: &str, kind: AgentEventKind) -> usize {
    let (rows, _more) = db
        .scan_cf_from(cf::CF_AGENT_EVENTS, &[], 1_000_000)
        .expect("scan CF_AGENT_EVENTS");
    rows.iter()
        .filter_map(|(_key, value)| serde_json::from_slice::<AgentEventRecord>(value).ok())
        .filter(|record| record.kind == kind && record.session_id.as_deref() == Some(session_id))
        .count()
}

#[tokio::test]
async fn agent_kill_terminates_real_process_tree_and_journals_killed() {
    let temp = TempDir::new().expect("temp dir");
    let service = fsv_service(temp.path());
    let session = "session-fsv-kill-1";
    let spawn = "agent-spawn-fsv-kill-1";
    let pid = spawn_victim();
    register_spawned_victim(&service, session, spawn, pid);

    // BEFORE: the process is alive in the OS process table (source of truth).
    assert!(
        crate::m4::process_exists(pid),
        "precondition: victim pid {pid} must be alive before the kill"
    );

    let response = service
        .agent_kill_impl(
            AgentKillParams {
                session_id: session.to_owned(),
                grace_ms: 0,
                interrupt_first: false,
            },
            Some("operator-fsv"),
        )
        .await
        .expect("agent_kill must succeed");

    // FSV 1: the tool reports the kill with zero orphans.
    assert!(response.killed, "agent_kill must report killed=true");
    assert!(
        response.orphan_process_ids.is_empty(),
        "no orphan processes may remain: {:?}",
        response.orphan_process_ids
    );
    assert!(!response.already_dead, "the victim was alive when killed");
    assert_eq!(response.session_id, session);
    assert_eq!(response.spawn_id.as_deref(), Some(spawn));

    // FSV 2: AFTER — the OS process table, read back independently, confirms the
    // pid is gone. This is the authoritative proof, not the return value.
    assert!(
        !crate::m4::process_exists(pid),
        "victim pid {pid} must be gone from the OS process table after the kill"
    );

    // FSV 3: the durable killed event is physically present in CF_AGENT_EVENTS.
    let db = service.agent_control_db().expect("open storage");
    assert_eq!(
        journal_count_kind(&db, session, AgentEventKind::Killed),
        1,
        "exactly one Killed journal row must exist for {session}"
    );
    assert!(
        response.journal_killed_event.is_some(),
        "the response must carry the killed journal readback"
    );

    // FSV 4: command audit rows for agent_kill are physically present in
    // CF_ACTION_LOG (intent + final).
    let audit = service.command_audit_snapshot().expect("audit snapshot");
    let kill_rows = audit
        .rows
        .iter()
        .filter(|row| row.tool == "agent_kill")
        .count();
    assert!(
        kill_rows >= 2,
        "expected intent+final agent_kill audit rows, found {kill_rows}"
    );
}

#[tokio::test]
async fn agent_kill_is_idempotent_double_kill_reports_already_dead() {
    let temp = TempDir::new().expect("temp dir");
    let service = fsv_service(temp.path());
    let session = "session-fsv-kill-2";
    let spawn = "agent-spawn-fsv-kill-2";
    let pid = spawn_victim();
    register_spawned_victim(&service, session, spawn, pid);

    let first = service
        .agent_kill_impl(
            AgentKillParams {
                session_id: session.to_owned(),
                grace_ms: 0,
                interrupt_first: false,
            },
            Some("operator-fsv"),
        )
        .await
        .expect("first kill succeeds");
    assert!(first.killed && !first.already_dead);
    assert!(!crate::m4::process_exists(pid));

    // Second kill: the agent is already dead — idempotent success, no new
    // Killed event (nothing was force-terminated this time).
    let second = service
        .agent_kill_impl(
            AgentKillParams {
                session_id: session.to_owned(),
                grace_ms: 0,
                interrupt_first: false,
            },
            Some("operator-fsv"),
        )
        .await
        .expect("second kill is idempotent, not an error");
    assert!(second.already_dead, "second kill must report already_dead");
    assert!(second.killed, "already-dead is still a successful kill");
    assert!(
        second.journal_killed_event.is_none(),
        "no force-kill happened, so no new Killed event"
    );
}

#[tokio::test]
async fn agent_kill_unknown_session_errors_structurally() {
    let temp = TempDir::new().expect("temp dir");
    let service = fsv_service(temp.path());
    let error = service
        .agent_kill_impl(
            AgentKillParams {
                session_id: "session-does-not-exist".to_owned(),
                grace_ms: 0,
                interrupt_first: false,
            },
            Some("operator-fsv"),
        )
        .await
        .expect_err("unknown session must error");
    assert!(
        error.message.contains("AGENT_NOT_FOUND"),
        "unexpected error: {}",
        error.message
    );
}

#[tokio::test]
async fn agent_interrupt_delivers_cooperative_mailbox_and_journals_interrupted() {
    let temp = TempDir::new().expect("temp dir");
    let service = fsv_service(temp.path());
    let session = "session-fsv-interrupt-1";
    let spawn = "agent-spawn-fsv-interrupt-1";
    let pid = spawn_victim();
    register_spawned_victim(&service, session, spawn, pid);

    let response = service
        .agent_interrupt_impl(
            AgentInterruptParams {
                session_id: session.to_owned(),
            },
            Some("operator-fsv"),
        )
        .expect("interrupt must deliver via the mailbox channel");

    // FSV 1: delivery is via the one wired channel; the other three are honestly
    // reported unavailable — never faked.
    assert!(response.delivered, "interrupt must be delivered");
    assert_eq!(response.delivered_via.as_deref(), Some("mailbox_interrupt"));
    assert_eq!(response.channels.len(), 4, "all four ranked channels reported");
    let delivered: Vec<&str> = response
        .channels
        .iter()
        .filter(|c| c.status == "delivered")
        .map(|c| c.channel.as_str())
        .collect();
    assert_eq!(delivered, vec!["mailbox_interrupt"]);
    let unavailable = response
        .channels
        .iter()
        .filter(|c| c.status == "unavailable")
        .count();
    assert_eq!(unavailable, 3, "codex/claude/pty channels are unavailable");

    // FSV 2: the durable interrupt mailbox row is physically present in CF_KV.
    let db = service.agent_control_db().expect("open storage");
    let mailbox_channel = response
        .channels
        .iter()
        .find(|c| c.channel == "mailbox_interrupt")
        .expect("mailbox channel present");
    let row_key = mailbox_channel
        .row_key
        .as_ref()
        .expect("delivered mailbox row has a key");
    let (rows, _more) = db
        .scan_cf_from(cf::CF_KV, row_key.as_bytes(), 1)
        .expect("scan CF_KV for the mailbox row");
    assert!(
        rows.first().map(|(k, _)| k.as_slice()) == Some(row_key.as_bytes()),
        "the durable interrupt mailbox row must exist at {row_key}"
    );

    // FSV 3: the durable Interrupted journal row exists.
    assert_eq!(
        journal_count_kind(&db, session, AgentEventKind::Interrupted),
        1,
        "exactly one Interrupted journal row must exist"
    );

    // Clean up the still-live victim deterministically.
    let _ = service
        .agent_kill_impl(
            AgentKillParams {
                session_id: session.to_owned(),
                grace_ms: 0,
                interrupt_first: false,
            },
            Some("operator-fsv"),
        )
        .await
        .expect("cleanup kill");
    assert!(!crate::m4::process_exists(pid));
}
