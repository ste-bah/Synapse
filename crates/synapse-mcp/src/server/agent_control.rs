//! `agent_interrupt` / `agent_kill` (#904) — first-class stop verbs for
//! Synapse-spawned sub-agents.
//!
//! Until this module, the only way to stop a spawned agent was the
//! coarse-grained `session_end` teardown of the *caller's* whole session. These
//! two verbs target **one** agent by its own MCP session id (or `agent-spawn-*`
//! id) and stop it with Full-State-Verification baked into the response: the
//! actual OS process table is read back before and after, the registry/journal
//! state transition is recorded, and every channel reports its real outcome.
//!
//! # Channel ranking (research-grounded, #904)
//!
//! The issue asks for a *channel-ranked* graceful interrupt. The honest reality
//! of the current spawn architecture (agents are launched as plain CLI
//! processes that connect back over MCP HTTP — there is no owned PTY and no
//! held stdin pipe) is that only one graceful channel is actually wired today.
//! Each channel reports its true status; **no channel ever silently "succeeds"**:
//!
//! 1. `codex_app_server_turn_interrupt` — Codex `turn/interrupt` JSON-RPC
//!    (`{threadId,turnId}` → `{}`, turn ends `interrupted`). Requires the daemon
//!    to be the Codex **app-server** JSON-RPC client; Synapse spawns codex as a
//!    plain CLI, so this channel is **not wired** (`channel_not_wired`).
//! 2. `claude_stream_json_control` — there is **no supported stdin cancel frame**
//!    for `claude -p` today (anthropics/claude-code#51078 is an open feature
//!    request); the Agent SDK's `interruptTurn` only works when the SDK owns the
//!    persistent stream-json stdin pipe, which the daemon does not. **Not wired.**
//! 3. `mailbox_interrupt` — **wired.** A durable `interrupt` mailbox row (#908)
//!    delivered to the agent's steering inbox. Cooperative agents drain it
//!    between tool calls and bail (`steering_requests_shutdown` honors
//!    `kill|stop|cancel|interrupt|shutdown`). Delivery is proven by the
//!    persisted `CF_KV` row readback.
//! 4. `pty_esc` — the documented last-resort interrupt key requires owned-PTY
//!    capture (#902), which does not exist yet. **Not wired.**
//!
//! `agent_kill` reuses the authoritative per-session teardown machinery
//! (`session_lifecycle::teardown_session`): every spawned agent's *own* session
//! id owns both its process resource (the Windows job handle) and its
//! leases/claims/desktops, so a single teardown of that session does job-close →
//! force-kill of the process tree and releases all of the agent's resources.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synapse_core::{AgentEndState, AgentEventKind, AgentEventRecord, error_codes};

use rmcp::{RoleServer, service::RequestContext};

use super::agent_events::{record_agent_event_durable, unix_time_ns_now};
use super::command_audit::CommandAuditInput;
use super::session_lifecycle::SessionTeardownReport;
use super::session_registry::unix_time_ms_now;
use super::{ErrorData, Json, Parameters, SynapseService, mcp_error, tool, tool_router};

// ----------------------------------------------------------------------------
// Tunables
// ----------------------------------------------------------------------------

/// Default graceful window between the interrupt attempt and the force-kill.
const DEFAULT_KILL_GRACE_MS: u64 = 3_000;
/// Hard ceiling on the graceful window so a kill cannot block unbounded.
const MAX_KILL_GRACE_MS: u64 = 120_000;
/// Poll cadence while waiting for the process tree to exit during the grace
/// window.
const GRACE_POLL_INTERVAL_MS: u64 = 100;
/// TTL for the cooperative interrupt mailbox row — short, because a stale
/// interrupt request is noise once the agent is gone.
const INTERRUPT_MESSAGE_TTL_MS: u64 = 60_000;
/// Mailbox kind the cooperative shutdown contract recognizes.
const INTERRUPT_MAILBOX_KIND: &str = "interrupt";

const TOOL_AGENT_INTERRUPT: &str = "agent_interrupt";
const TOOL_AGENT_KILL: &str = "agent_kill";

// ----------------------------------------------------------------------------
// Params
// ----------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentInterruptParams {
    /// The agent to interrupt: its own MCP session id, or its `agent-spawn-*`
    /// id. Resolves through the live session registry.
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentKillParams {
    /// The agent to kill: its own MCP session id, or its `agent-spawn-*` id.
    pub session_id: String,
    /// Graceful window (ms) to wait for the agent to stop on its own after the
    /// interrupt attempt, before force-terminating the process tree.
    #[serde(default = "default_kill_grace_ms")]
    #[schemars(default = "default_kill_grace_ms", range(min = 0, max = 120_000))]
    pub grace_ms: u64,
    /// When true (default) a graceful interrupt is attempted first; when false
    /// the process tree is force-terminated immediately.
    #[serde(default = "default_true")]
    #[schemars(default = "default_true")]
    pub interrupt_first: bool,
}

const fn default_kill_grace_ms() -> u64 {
    DEFAULT_KILL_GRACE_MS
}
const fn default_true() -> bool {
    true
}

// ----------------------------------------------------------------------------
// Response types
// ----------------------------------------------------------------------------

/// A live readback of the agent's process tree from the OS process table — the
/// source of truth for "is it actually dead".
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcessReadback {
    pub launcher_process_id: u32,
    /// Every pid the owned job/process tree currently maps to.
    pub process_tree_ids: Vec<u32>,
    /// The subset of `process_tree_ids` that are still alive right now.
    pub live_process_ids: Vec<u32>,
}

/// One ranked channel's real outcome. `status` is one of `delivered`,
/// `unavailable`, or `failed` — never a fabricated success.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelAttempt {
    pub channel: String,
    pub rank: u32,
    pub status: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_key: Option<String>,
}

/// Physical readback of a persisted journal row.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JournalReadback {
    pub kind: String,
    pub ts_ns: u64,
    pub seq: u32,
    pub value_len_bytes: u64,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentInterruptResponse {
    pub requested_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spawn_id: Option<String>,
    pub agent_kind: String,
    pub lifecycle: String,
    /// True when at least one channel actually delivered the interrupt.
    pub delivered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_via: Option<String>,
    /// Every ranked channel and its true outcome.
    pub channels: Vec<ChannelAttempt>,
    /// The `interrupted` journal row written when a channel delivered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal_event: Option<JournalReadback>,
    /// The OS process table at interrupt time (an interrupt never kills, so it
    /// is read once for evidence).
    pub process: ProcessReadback,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentKillResponse {
    pub requested_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spawn_id: Option<String>,
    pub agent_kind: String,
    /// True when the process tree was already gone before this call acted —
    /// makes double-kill idempotent (the second call reports `already_dead`).
    pub already_dead: bool,
    /// The graceful interrupt attempt, when `interrupt_first` was set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupt: Option<AgentInterruptResponse>,
    pub grace_ms: u64,
    /// True when the agent exited on its own during the grace window, with no
    /// force-termination needed.
    pub natural_exit: bool,
    pub process_before: ProcessReadback,
    pub process_after: ProcessReadback,
    /// Live pids still standing after teardown. MUST be empty for `killed`.
    pub orphan_process_ids: Vec<u32>,
    /// True iff zero orphan processes remain (the OS process table is the SoT).
    pub killed: bool,
    /// The `killed` journal row, written before teardown when a force-kill was
    /// actually required.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal_killed_event: Option<JournalReadback>,
    /// Full per-resource teardown report (process job close/force, lease, claim,
    /// desktop, registry transitions) when teardown succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teardown: Option<SessionTeardownReport>,
    /// Set when teardown returned an error; the kill's success is still judged
    /// by `orphan_process_ids` (the OS process table), never by this alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teardown_error: Option<String>,
}

// ----------------------------------------------------------------------------
// Resolved-target model
// ----------------------------------------------------------------------------

/// A spawned agent located in the live session registry.
#[derive(Clone, Debug)]
struct ResolvedAgent {
    /// The agent's own MCP session id (owns the process resource and leases).
    session_id: String,
    spawn_id: Option<String>,
    agent_kind: String,
    lifecycle: String,
    launcher_process_id: u32,
}

// ----------------------------------------------------------------------------
// Tools
// ----------------------------------------------------------------------------

#[tool_router(router = agent_control_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Gracefully interrupt one running spawned agent (#904) by its MCP session id or agent-spawn-* id, via ranked clean channels. Currently only the cooperative mailbox channel is wired (codex turn/interrupt, claude stream-json cancel, and PTY ESC are reported unavailable with the issue that would wire them, never faked). Reports each channel's real outcome plus a process-table readback; errors if no channel can deliver. Use agent_kill to force-terminate."
    )]
    pub async fn agent_interrupt(
        &self,
        params: Parameters<AgentInterruptParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<AgentInterruptResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "agent_interrupt",
            "tool.invocation kind=agent_interrupt"
        );
        let caller = super::context::mcp_session_id_from_request_context(&request_context)?;
        self.agent_interrupt_impl(params.0, caller.as_deref())
            .map(Json)
    }

    #[tool(
        description = "Force-stop one spawned agent (#904): attempt a graceful interrupt, wait grace_ms, then terminate the recorded process tree (Windows job-close → force kill) by reusing per-session teardown, releasing the agent's leases/claims/desktops and journaling a durable killed event. Full-State-Verification is in the response: the OS process table is read back before and after, and killed is true only when zero orphan processes remain. Double-kill is idempotent (reports already_dead); unknown/non-spawned sessions error."
    )]
    pub async fn agent_kill(
        &self,
        params: Parameters<AgentKillParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<AgentKillResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "agent_kill",
            "tool.invocation kind=agent_kill"
        );
        let caller = super::context::mcp_session_id_from_request_context(&request_context)?;
        self.agent_kill_impl(params.0, caller.as_deref())
            .await
            .map(Json)
    }
}

impl SynapseService {
    // ------------------------------------------------------------------
    // agent_interrupt
    // ------------------------------------------------------------------

    fn agent_interrupt_impl(
        &self,
        params: AgentInterruptParams,
        caller_session: Option<&str>,
    ) -> Result<AgentInterruptResponse, ErrorData> {
        let lookup = validate_lookup_id(&params.session_id, TOOL_AGENT_INTERRUPT)?;
        let target = self.resolve_spawned_agent(&lookup, TOOL_AGENT_INTERRUPT)?;
        if target.lifecycle == "closed" {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!(
                    "AGENT_ALREADY_DEAD: agent {} (session {}) is closed; interrupt targets a live agent — use agent_kill to reclaim a dead agent's resources",
                    lookup, target.session_id
                ),
            ));
        }
        let process = process_readback(target.launcher_process_id);

        let payload = json!({
            "reason": "operator_interrupt",
            "requested_id": lookup,
            "from": caller_session,
        });
        let before = json!({ "process": &process, "lifecycle": target.lifecycle });
        self.command_audit_intent(
            CommandAuditInput::mcp(
                TOOL_AGENT_INTERRUPT,
                "interrupt",
                caller_session.map(ToOwned::to_owned),
                Some(target.session_id.clone()),
                payload.clone(),
                before.clone(),
                Value::Null,
                "pending",
            )
            .with_target(json!({ "spawn_id": target.spawn_id, "agent_kind": target.agent_kind })),
        )?;

        let response = self.interrupt_core(&lookup, &target, caller_session)?;

        let after = json!({
            "delivered": response.delivered,
            "delivered_via": response.delivered_via,
            "channels": response.channels,
        });
        self.command_audit_final(
            CommandAuditInput::mcp(
                TOOL_AGENT_INTERRUPT,
                "interrupt",
                caller_session.map(ToOwned::to_owned),
                Some(target.session_id.clone()),
                payload,
                before,
                after,
                if response.delivered { "ok" } else { "error" },
            )
            .with_target(json!({ "spawn_id": target.spawn_id, "agent_kind": target.agent_kind })),
        )?;

        if !response.delivered {
            return Err(mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!(
                    "AGENT_INTERRUPT_NO_CHANNEL: no clean channel could deliver an interrupt to agent {} (session {}); see the per-channel `channels` report for why each was unavailable. Use agent_kill to force-terminate.",
                    response.requested_id, response.session_id
                ),
            ));
        }
        Ok(response)
    }

    /// Builds an interrupt response (channels attempted + durable `interrupted`
    /// journal row on delivery) without auditing or erroring on non-delivery.
    /// Shared by `agent_interrupt` (which wraps it with audit + a no-channel
    /// error) and `agent_kill` (which calls it best-effort before force-kill).
    fn interrupt_core(
        &self,
        requested_id: &str,
        target: &ResolvedAgent,
        caller_session: Option<&str>,
    ) -> Result<AgentInterruptResponse, ErrorData> {
        let process = process_readback(target.launcher_process_id);
        let (channels, delivered_via, _send_row) =
            self.attempt_interrupt_channels(target, caller_session);
        let delivered = delivered_via.is_some();
        // Journal a durable `interrupted` event only when a channel actually
        // delivered — never claim an interruption that did not happen.
        let journal_event = if delivered {
            Some(self.journal_lifecycle_event(
                AgentEventKind::Interrupted,
                target,
                "agent_interrupt",
                None,
                json!({ "delivered_via": delivered_via, "process": &process }),
            )?)
        } else {
            None
        };
        Ok(AgentInterruptResponse {
            requested_id: requested_id.to_owned(),
            session_id: target.session_id.clone(),
            spawn_id: target.spawn_id.clone(),
            agent_kind: target.agent_kind.clone(),
            lifecycle: target.lifecycle.clone(),
            delivered,
            delivered_via,
            channels,
            journal_event,
            process,
        })
    }

    /// Attempts each ranked channel and returns `(attempts, delivered_via,
    /// send_row)`. Only the cooperative mailbox channel is wired today; the
    /// rest report their true unavailability with the issue that would wire
    /// them.
    fn attempt_interrupt_channels(
        &self,
        target: &ResolvedAgent,
        caller_session: Option<&str>,
    ) -> (Vec<ChannelAttempt>, Option<String>, Option<String>) {
        let mut attempts = Vec::new();
        let mut delivered_via = None;
        let mut send_row = None;

        attempts.push(ChannelAttempt {
            channel: "codex_app_server_turn_interrupt".to_owned(),
            rank: 1,
            status: "unavailable".to_owned(),
            reason: "channel_not_wired: codex turn/interrupt needs an app-server JSON-RPC \
                     connection; Synapse spawns codex as a plain CLI process"
                .to_owned(),
            message_id: None,
            row_key: None,
        });
        attempts.push(ChannelAttempt {
            channel: "claude_stream_json_control".to_owned(),
            rank: 2,
            status: "unavailable".to_owned(),
            reason: "channel_not_wired: no supported claude -p stdin cancel frame today \
                     (anthropics/claude-code#51078); the daemon does not own the stream-json pipe"
                .to_owned(),
            message_id: None,
            row_key: None,
        });

        // Rank 3: cooperative mailbox interrupt — the one wired channel.
        let mailbox = self.deliver_mailbox_interrupt(target, caller_session);
        if mailbox.status == "delivered" {
            delivered_via = Some(mailbox.channel.clone());
            send_row.clone_from(&mailbox.row_key);
        }
        attempts.push(mailbox);

        attempts.push(ChannelAttempt {
            channel: "pty_esc".to_owned(),
            rank: 4,
            status: "unavailable".to_owned(),
            reason: "channel_not_wired: PTY ESC (the documented interrupt key) needs owned-PTY \
                     capture (#902), which is not implemented yet"
                .to_owned(),
            message_id: None,
            row_key: None,
        });

        (attempts, delivered_via, send_row)
    }

    /// Delivers a durable `interrupt` mailbox row to the target's steering
    /// inbox, proving delivery by the persisted `CF_KV` row readback.
    fn deliver_mailbox_interrupt(
        &self,
        target: &ResolvedAgent,
        caller_session: Option<&str>,
    ) -> ChannelAttempt {
        let Some(caller) = caller_session else {
            return ChannelAttempt {
                channel: "mailbox_interrupt".to_owned(),
                rank: 3,
                status: "unavailable".to_owned(),
                reason: "needs the caller's MCP session id (run the daemon in HTTP mode so each \
                         agent has its own Mcp-Session-Id)"
                    .to_owned(),
                message_id: None,
                row_key: None,
            };
        };
        let send = self.agent_send_impl(
            super::agent_mailbox::AgentSendParams {
                to_session: target.session_id.clone(),
                kind: INTERRUPT_MAILBOX_KIND.to_owned(),
                payload: json!({
                    "control": "interrupt",
                    "from": caller,
                    "reason": "operator_interrupt",
                    "instructions": "stop the current turn at the next safe point",
                }),
                artifact_handle: None,
                ttl_ms: INTERRUPT_MESSAGE_TTL_MS,
                request_receipt: false,
            },
            caller,
        );
        match send {
            Ok(response) => ChannelAttempt {
                channel: "mailbox_interrupt".to_owned(),
                rank: 3,
                status: "delivered".to_owned(),
                reason: format!(
                    "durable {} row persisted to CF_KV (queue_depth_after={}); cooperative agents \
                     drain it between tool calls and bail",
                    INTERRUPT_MAILBOX_KIND, response.queue_depth_after
                ),
                message_id: Some(response.message_id),
                row_key: Some(response.row_key),
            },
            Err(error) => ChannelAttempt {
                channel: "mailbox_interrupt".to_owned(),
                rank: 3,
                status: "failed".to_owned(),
                reason: format!("mailbox delivery failed: {}", error.message),
                message_id: None,
                row_key: None,
            },
        }
    }

    // ------------------------------------------------------------------
    // agent_kill
    // ------------------------------------------------------------------

    async fn agent_kill_impl(
        &self,
        params: AgentKillParams,
        caller_session: Option<&str>,
    ) -> Result<AgentKillResponse, ErrorData> {
        let lookup = validate_lookup_id(&params.session_id, TOOL_AGENT_KILL)?;
        if params.grace_ms > MAX_KILL_GRACE_MS {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!(
                    "AGENT_KILL_GRACE_INVALID: grace_ms must be 0..={MAX_KILL_GRACE_MS}, got {}",
                    params.grace_ms
                ),
            ));
        }
        let target = self.resolve_spawned_agent(&lookup, TOOL_AGENT_KILL)?;
        let process_before = process_readback(target.launcher_process_id);
        let already_dead = process_before.live_process_ids.is_empty();

        let payload = json!({
            "requested_id": lookup,
            "grace_ms": params.grace_ms,
            "interrupt_first": params.interrupt_first,
            "from": caller_session,
        });
        let before = json!({ "process": &process_before, "lifecycle": target.lifecycle });
        self.command_audit_intent(
            CommandAuditInput::mcp(
                TOOL_AGENT_KILL,
                "kill",
                caller_session.map(ToOwned::to_owned),
                Some(target.session_id.clone()),
                payload.clone(),
                before.clone(),
                Value::Null,
                "pending",
            )
            .with_target(json!({ "spawn_id": target.spawn_id, "agent_kind": target.agent_kind })),
        )?;

        // Graceful first: attempt the interrupt (best-effort — its failure must
        // never block the force-kill), then wait the grace window for the tree
        // to exit on its own. Skipped entirely when already dead.
        let interrupt = if params.interrupt_first && !already_dead {
            self.interrupt_core(&target.session_id.clone(), &target, caller_session)
                .ok()
        } else {
            None
        };
        let natural_exit = if !already_dead && params.grace_ms > 0 {
            let tree = crate::m4::owned_process_tree_ids(target.launcher_process_id);
            let (remaining, _waited) = wait_for_tree_exit_async(&tree, params.grace_ms).await;
            remaining.is_empty()
        } else {
            false
        };

        // Was a force-kill actually required? (the tree is still alive)
        let live_after_grace =
            crate::m4::owned_live_process_ids(&process_before.process_tree_ids);
        let force_needed = !live_after_grace.is_empty();

        // Journal the durable `killed` event BEFORE teardown when a force-kill
        // is required, so the terminal transition is recorded as killed (not a
        // generic exit). The reducer treats a dead agent as dead, so teardown's
        // later `exited` row is absorbed without a spurious transition.
        let journal_killed_event = if force_needed {
            Some(self.journal_lifecycle_event(
                AgentEventKind::Killed,
                &target,
                "agent_kill",
                Some(AgentEndState::Error),
                json!({
                    "process_before": &process_before,
                    "live_before_force": live_after_grace,
                    "grace_ms": params.grace_ms,
                    "interrupt_first": params.interrupt_first,
                }),
            )?)
        } else {
            None
        };

        // Reuse the authoritative per-session teardown: job-close → force kill
        // of the process tree, plus lease/claim/desktop release and registry
        // close. Keyed by the agent's OWN session id, which owns all of it.
        let lifecycle = self.session_lifecycle_state()?;
        let (teardown, teardown_error) =
            match lifecycle.teardown_session(&target.session_id, "agent_kill").await {
                Ok(report) => (Some(report), None),
                Err(error) => (None, Some(error.message.to_string())),
            };

        // Source of truth for "is it dead": re-read the OS process table.
        let process_after = process_readback(target.launcher_process_id);
        let orphan_process_ids = process_after.live_process_ids.clone();
        let killed = orphan_process_ids.is_empty();

        let response = AgentKillResponse {
            requested_id: lookup,
            session_id: target.session_id.clone(),
            spawn_id: target.spawn_id.clone(),
            agent_kind: target.agent_kind.clone(),
            already_dead,
            interrupt,
            grace_ms: params.grace_ms,
            natural_exit,
            process_before,
            process_after,
            orphan_process_ids,
            killed,
            journal_killed_event,
            teardown,
            teardown_error,
        };

        let after = json!({
            "killed": response.killed,
            "already_dead": response.already_dead,
            "natural_exit": response.natural_exit,
            "orphan_process_ids": response.orphan_process_ids,
            "process_after": response.process_after,
            "teardown_error": response.teardown_error,
        });
        self.command_audit_final(
            CommandAuditInput::mcp(
                TOOL_AGENT_KILL,
                "kill",
                caller_session.map(ToOwned::to_owned),
                Some(target.session_id.clone()),
                payload,
                before,
                after,
                if response.killed { "ok" } else { "error" },
            )
            .with_target(json!({ "spawn_id": target.spawn_id, "agent_kind": target.agent_kind })),
        )?;

        if !response.killed {
            return Err(mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!(
                    "AGENT_KILL_ORPHANS: agent {} (session {}) still has live processes after teardown: {:?}{}. The kill is reported as failed; these pids survived.",
                    response.requested_id,
                    response.session_id,
                    response.orphan_process_ids,
                    response
                        .teardown_error
                        .as_ref()
                        .map(|error| format!(" (teardown error: {error})"))
                        .unwrap_or_default(),
                ),
            ));
        }
        Ok(response)
    }

    // ------------------------------------------------------------------
    // Shared helpers
    // ------------------------------------------------------------------

    /// Locates a spawned agent in the live session registry by its own session
    /// id or its `agent-spawn-*` id. Errors structurally for unknown ids and
    /// for known sessions that are not Synapse-spawned (no owned process tree).
    fn resolve_spawned_agent(
        &self,
        lookup: &str,
        tool: &str,
    ) -> Result<ResolvedAgent, ErrorData> {
        let now = unix_time_ms_now();
        let registry = self.session_registry.lock().map_err(|_error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "session registry lock poisoned while resolving agent target",
            )
        })?;
        let mut session_match = None;
        let mut non_spawned_session_hit = false;
        for read in registry.reads(now) {
            let Some(spawned) = read.spawned_agent.as_ref() else {
                if read.session_id == lookup {
                    non_spawned_session_hit = true;
                }
                continue;
            };
            if read.session_id == lookup || spawned.spawn_id == lookup {
                session_match = Some(ResolvedAgent {
                    session_id: read.session_id.clone(),
                    spawn_id: Some(spawned.spawn_id.clone()),
                    agent_kind: read.agent_kind.clone(),
                    lifecycle: read.lifecycle.clone(),
                    launcher_process_id: spawned.launcher_process_id,
                });
                break;
            }
        }
        drop(registry);

        if let Some(resolved) = session_match {
            return Ok(resolved);
        }
        if non_spawned_session_hit {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!(
                    "AGENT_NOT_SPAWNED: session {lookup} exists but is not a Synapse-spawned agent; {tool} owns no process tree for it"
                ),
            ));
        }
        Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!(
                "AGENT_NOT_FOUND: no live spawned agent resolves to '{lookup}' (try its MCP session id or agent-spawn-* id from act_spawn_agent / agent_query)"
            ),
        ))
    }

    /// Writes a durable lifecycle journal row (`Interrupted` / `Killed`) for an
    /// agent and returns its physical readback. Mirrors the attribution that
    /// `session_lifecycle` uses for its `Exited` rows.
    fn journal_lifecycle_event(
        &self,
        kind: AgentEventKind,
        target: &ResolvedAgent,
        reason_code: &str,
        end_state: Option<AgentEndState>,
        payload: Value,
    ) -> Result<JournalReadback, ErrorData> {
        let db = self.agent_control_db()?;
        let mut record = AgentEventRecord::new(unix_time_ns_now(), kind);
        record.session_id = Some(target.session_id.clone());
        record.spawn_id.clone_from(&target.spawn_id);
        record.reason_code = Some(reason_code.to_owned());
        record.end_state = end_state;
        record.attributes.conversation_id = Some(target.session_id.clone());
        if target.agent_kind != "unknown" {
            record.attributes.agent_name = Some(target.agent_kind.clone());
        }
        record.payload = payload;
        let readback = record_agent_event_durable(&db, &record)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        Ok(JournalReadback {
            kind: format!("{kind:?}"),
            ts_ns: readback.ts_ns,
            seq: readback.seq,
            value_len_bytes: readback.value_len_bytes as u64,
        })
    }

    /// Opens the shared M3 storage handle (same path `agent_query` uses).
    fn agent_control_db(&self) -> Result<std::sync::Arc<synapse_storage::Db>, ErrorData> {
        let state = self.m3_state_handle();
        let mut guard = state.lock().map_err(|_error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "M3 service state lock poisoned while opening agent control storage",
            )
        })?;
        guard
            .ensure_storage()
            .map_err(|error| mcp_error(error.code(), error.to_string()))
    }
}

fn process_readback(launcher_pid: u32) -> ProcessReadback {
    let process_tree_ids = crate::m4::owned_process_tree_ids(launcher_pid);
    let live_process_ids = crate::m4::owned_live_process_ids(&process_tree_ids);
    ProcessReadback {
        launcher_process_id: launcher_pid,
        process_tree_ids,
        live_process_ids,
    }
}

/// Polls the owned process tree for exit up to `grace_ms`, yielding to the async
/// runtime between polls so the daemon stays responsive during the grace window.
async fn wait_for_tree_exit_async(process_ids: &[u32], grace_ms: u64) -> (Vec<u32>, u64) {
    let deadline = Duration::from_millis(grace_ms);
    let started = tokio::time::Instant::now();
    loop {
        let remaining = crate::m4::owned_live_process_ids(process_ids);
        if remaining.is_empty() {
            return (remaining, u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
        }
        if started.elapsed() >= deadline {
            return (remaining, u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
        }
        tokio::time::sleep(Duration::from_millis(GRACE_POLL_INTERVAL_MS)).await;
    }
}

fn validate_lookup_id(session_id: &str, tool: &str) -> Result<String, ErrorData> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("{tool}: session_id must be a non-empty MCP session id or agent-spawn-* id"),
        ));
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests;
