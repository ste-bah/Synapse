//! `approval_gate` — the permission-prompt tool that turns a spawned agent's
//! tool call into a human approval (#927).
//!
//! Synapse launches Claude agents with
//! `--permission-prompt-tool mcp__synapse__approval_gate`. When the agent wants
//! a tool that its static `permissions.allow` rules don't cover, Claude calls
//! THIS tool synchronously and blocks on the result. We:
//!
//! 1. classify the (tool_name, input) with [`super::permission_policy`];
//! 2. **auto-allow** read-only / low-consequence calls instantly — no inbox
//!    item, no human in the loop (the fatigue guard for a 50-agent fleet);
//! 3. for risky calls, create a `Pending` `ApprovalKind::AgentPermission` row
//!    (the same durable `CF_KV` queue the dashboard reads) and **block** until a
//!    human decides in the Approvals inbox or the deadline elapses;
//! 4. return Claude the permission verdict as `{"behavior":"allow"|"deny"}` —
//!    *returning from this call is the agent's resume*. No stdin injection.
//!
//! The block is woken instantly by [`signal_decision`] (called from the
//! dashboard decide path in the same daemon process) and, as a race-proof
//! backstop, re-reads the `CF_KV` row as source of truth every poll tick.
//! On the deadline we decline the row ourselves and return a `deny` carrying a
//! clear reason — the agent never silently proceeds on a risky action.
//!
//! Failure contract: a storage/internal error returns a **loud MCP error**
//! (never a silent allow), so a broken gate fails closed and visibly.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rmcp::model::{CallToolResult, Content};
use rmcp::{RoleServer, service::RequestContext};
use serde_json::{Value, json};

use super::permission_policy::{self, GateDecision};
use super::{ErrorData, Parameters, SynapseService, tool, tool_router};
use crate::m1::mcp_error;
use crate::m3::approvals::{
    self, ApprovalDecideParams, ApprovalDecision, ApprovalKind, ApprovalRequestParams,
    ApprovalStatus, ApprovalTimeoutDecision,
};
use crate::m3::permissions::{Permission, required};
use synapse_core::error_codes;

/// Header the daemon injects into each spawned agent's MCP config so the gate
/// can attribute a call to its originating spawn (the bearer token is shared
/// across spawns and cannot distinguish them).
pub(crate) const SPAWN_ID_HEADER: &str = "x-synapse-spawn-id";

/// How often the blocking loop re-reads the `CF_KV` row as source of truth even
/// without a wake signal (covers any missed in-process notification).
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Default max time the gate blocks before declining and returning `deny`.
/// Kept comfortably under the agent's per-server MCP `timeout` (30 min) so we
/// return a clean verdict before Claude's client would abort the call.
const DEFAULT_GATE_TIMEOUT_MS: u64 = 25 * 60 * 1_000;

const MAX_PAYLOAD_INPUT_BYTES: usize = 16 * 1024;

fn gate_timeout() -> Duration {
    let ms = std::env::var("SYNAPSE_APPROVAL_GATE_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|ms| *ms >= 1_000)
        .unwrap_or(DEFAULT_GATE_TIMEOUT_MS);
    Duration::from_millis(ms)
}

/// In-process registry of approval ids a gate call is currently blocked on, so
/// the dashboard decide path can wake the exact waiter instantly.
fn waiters() -> &'static Mutex<HashMap<String, Arc<tokio::sync::Notify>>> {
    static WAITERS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>> = OnceLock::new();
    WAITERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_waiter(approval_id: &str) -> Arc<tokio::sync::Notify> {
    let notify = Arc::new(tokio::sync::Notify::new());
    if let Ok(mut map) = waiters().lock() {
        map.insert(approval_id.to_owned(), Arc::clone(&notify));
    }
    notify
}

fn unregister_waiter(approval_id: &str) {
    if let Ok(mut map) = waiters().lock() {
        map.remove(approval_id);
    }
}

/// Wake the gate call blocked on `approval_id` (if any). Called from the
/// dashboard/MCP decide paths the instant a human resolves the approval.
pub(crate) fn signal_decision(approval_id: &str) {
    if let Ok(map) = waiters().lock() {
        if let Some(notify) = map.get(approval_id) {
            notify.notify_waiters();
        }
    }
}

// Closed schema — strict MCP clients (the spawned `--strict-mcp-config` agent)
// reject open schemas, and the project enforces additionalProperties:false.
// The permission-prompt-tool contract is undocumented; community evidence and
// the SDK `canUseTool` shape agree on exactly these fields. The live spike
// confirms the wire shape before we depend on it; if Claude ever sends more,
// switch this to a flatten-captured map.
#[derive(Clone, Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApprovalGateParams {
    /// Name of the tool the agent wants to use (e.g. "Bash", "WebFetch",
    /// "mcp__synapse__act_run_shell"). Sent by Claude's permission system.
    #[serde(default)]
    pub tool_name: Option<String>,
    /// The arguments the agent is about to pass to that tool.
    #[serde(default)]
    pub input: Option<Value>,
    /// The tool_use id of the pending call (used to dedupe retries).
    #[serde(default)]
    pub tool_use_id: Option<String>,
}

#[tool_router(router = permission_gate_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Permission-prompt tool for spawned agents (#927). Claude calls this automatically (via --permission-prompt-tool) when it wants to run a tool not covered by its static allow rules. Read-only / low-consequence calls are auto-allowed instantly; risky calls (destructive or mutating shell, network access, outward-facing or destructive MCP tools) create a Pending approval in the dashboard Approvals inbox and BLOCK until a human approves or denies — the verdict is returned to the still-running agent as {\"behavior\":\"allow\"|\"deny\"}. Not intended for direct human/agent invocation."
    )]
    pub async fn approval_gate(
        &self,
        params: Parameters<ApprovalGateParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let tool_name = params.tool_name.clone().unwrap_or_default();
        let input = params.input.clone().unwrap_or(Value::Null);
        // Raw-shape logging: the permission-prompt-tool contract is undocumented,
        // so we record exactly what Claude sent to verify/repair field mapping.
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "approval_gate",
            tool_name = %tool_name,
            tool_use_id = ?params.tool_use_id,
            input_kind = input_kind(&input),
            "tool.invocation kind=approval_gate"
        );

        self.require_m3_permissions(
            "approval_gate",
            &required([Permission::ReadStorage, Permission::WriteStorage]),
        )?;

        let by_session = super::context::mcp_session_id_from_request_context(&request_context)?
            .unwrap_or_else(|| "stdio".to_owned());
        let spawn_id = header_value(&request_context, SPAWN_ID_HEADER);

        self.run_gate(
            &tool_name,
            &input,
            params.tool_use_id.as_deref(),
            &by_session,
            spawn_id.as_deref(),
        )
        .await
    }
}

impl SynapseService {
    /// Core gate logic, decoupled from the MCP `RequestContext` so it is
    /// directly testable in-process (mirrors the `*_without_request_context`
    /// convention). Classifies the call, auto-allows safe ones, or creates a
    /// Pending approval and blocks until a human decides / the deadline.
    pub(crate) async fn run_gate(
        &self,
        tool_name: &str,
        input: &Value,
        tool_use_id: Option<&str>,
        by_session: &str,
        spawn_id: Option<&str>,
    ) -> Result<CallToolResult, ErrorData> {
        let decision = permission_policy::classify(tool_name, input);
        if !decision.is_gate() {
            tracing::info!(
                code = "APPROVAL_GATE_AUTO_ALLOW",
                tool_name = %tool_name,
                spawn_id = ?spawn_id,
                "approval_gate auto-allowed a low-consequence tool call"
            );
            return Ok(allow_result(input));
        }

        let db = self.m3_storage()?;
        let now = now_unix_ms();
        let request = build_request(tool_name, input, tool_use_id, spawn_id, decision)?;
        let created = approvals::request_approval(&db, &request, by_session)?;
        let approval_id = created.item.approval_id.clone();
        tracing::warn!(
            code = "APPROVAL_GATE_PENDING",
            approval_id = %approval_id,
            tool_name = %tool_name,
            spawn_id = ?spawn_id,
            destructive = decision.destructive(),
            deduped = created.deduped,
            "approval_gate is blocking on a human decision"
        );

        self.block_for_decision(&db, &approval_id, input, now).await
    }


    async fn block_for_decision(
        &self,
        db: &Arc<synapse_storage::Db>,
        approval_id: &str,
        input: &Value,
        started: u64,
    ) -> Result<CallToolResult, ErrorData> {
        let notify = register_waiter(approval_id);
        let deadline = Instant::now() + gate_timeout();
        let result = loop {
            let item = approvals::get_approval(db, approval_id)?
                .map(|queued| queued.item)
                .ok_or_else(|| {
                    mcp_internal(format!(
                        "approval_gate approval row {approval_id} vanished while blocked"
                    ))
                })?;
            match item.status {
                ApprovalStatus::Accepted => {
                    break Ok(allow_result(input));
                }
                ApprovalStatus::Declined | ApprovalStatus::Ignored => {
                    let message = item
                        .decision_note
                        .clone()
                        .unwrap_or_else(|| "Denied by the human operator.".to_owned());
                    break Ok(deny_result(&message));
                }
                // Snoozed = "not yet"; keep waiting until the deadline.
                ApprovalStatus::Pending | ApprovalStatus::Snoozed => {}
            }
            if Instant::now() >= deadline {
                let elapsed_s = now_unix_ms().saturating_sub(started) / 1_000;
                let message = format!(
                    "No human decision within {elapsed_s}s; gate timed out and denied this action."
                );
                // Reflect the timeout in the durable row so the inbox stops
                // showing it as pending. Best-effort: a failure here still
                // returns deny (fail closed).
                let decline = ApprovalDecideParams {
                    approval_id: approval_id.to_owned(),
                    decision: ApprovalDecision::Decline,
                    note: Some(message.clone()),
                    snooze_ms: None,
                };
                if let Err(error) = approvals::decide_approval(db, &decline, "approval_gate_timeout")
                {
                    tracing::error!(
                        code = "APPROVAL_GATE_TIMEOUT_DECLINE_FAILED",
                        approval_id = %approval_id,
                        detail = %error.message,
                        "approval_gate could not record its timeout decline; returning deny anyway"
                    );
                }
                break Ok(deny_result(&message));
            }
            tokio::select! {
                () = notify.notified() => {}
                () = tokio::time::sleep(POLL_INTERVAL) => {}
            }
        };
        unregister_waiter(approval_id);
        result
    }
}

fn build_request(
    tool_name: &str,
    input: &Value,
    tool_use_id: Option<&str>,
    spawn_id: Option<&str>,
    decision: GateDecision,
) -> Result<ApprovalRequestParams, ErrorData> {
    let input_repr = truncate_for_payload(input);
    let payload = json!({
        "tool_name": tool_name,
        "tool_use_id": tool_use_id,
        "spawn_id": spawn_id,
        "input": input_repr,
        "destructive": decision.destructive(),
    });
    let payload_json = serde_json::to_string(&payload)
        .map_err(|error| mcp_internal(format!("approval_gate failed to encode payload: {error}")))?;
    let title = {
        let mut title = format!("Approval needed: {}", display_tool_name(tool_name));
        title.truncate(160);
        title
    };
    let body = build_body(tool_name, input);
    Ok(ApprovalRequestParams {
        kind: ApprovalKind::AgentPermission,
        title,
        body,
        payload_json: Some(payload_json),
        // One pending item per (spawn, tool_use_id): retries re-attach instead
        // of stacking duplicates in the inbox.
        dedupe_key: Some(format!(
            "gate:{}:{}",
            spawn_id.unwrap_or("unknown"),
            tool_use_id.unwrap_or(tool_name)
        )),
        // Expiry sits just beyond the gate's own block deadline so OUR deadline
        // (with its descriptive message) fires first.
        timeout_ms: Some(gate_timeout().as_millis() as u64 + 60_000),
        timeout_decision: Some(ApprovalTimeoutDecision::Declined),
        destructive: decision.destructive(),
        notify: true,
        suppress_popup: false,
    })
}

fn build_body(tool_name: &str, input: &Value) -> String {
    // The approvals store rejects control characters in title/body (they must
    // be single-line display strings); the full, exact input — including any
    // newlines — is preserved losslessly in payload_json for the UI to render.
    let mut body = if tool_name == "Bash" {
        match input.get("command").and_then(Value::as_str) {
            Some(command) => format!("Run shell command: {}", single_line(command)),
            None => format!("Agent wants to use {tool_name}."),
        }
    } else {
        let rendered = serde_json::to_string(input).unwrap_or_else(|_| "{}".to_owned());
        format!("Agent wants to use {tool_name} with input: {}", single_line(&rendered))
    };
    body.truncate(4_000);
    body
}

/// Collapse control characters (newlines/tabs/etc.) to spaces so the string is
/// a valid single-line approval title/body.
fn single_line(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn display_tool_name(tool_name: &str) -> String {
    if tool_name.trim().is_empty() {
        "(unknown tool)".to_owned()
    } else {
        tool_name.to_owned()
    }
}

fn truncate_for_payload(input: &Value) -> Value {
    let encoded = input.to_string();
    if encoded.len() <= MAX_PAYLOAD_INPUT_BYTES {
        input.clone()
    } else {
        json!({
            "_truncated": true,
            "_original_bytes": encoded.len(),
            "preview": encoded.chars().take(2_000).collect::<String>(),
        })
    }
}

fn input_kind(input: &Value) -> &'static str {
    match input {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn allow_result(input: &Value) -> CallToolResult {
    let updated = if input.is_null() {
        json!({})
    } else {
        input.clone()
    };
    verdict_result(&json!({ "behavior": "allow", "updatedInput": updated }))
}

fn deny_result(message: &str) -> CallToolResult {
    verdict_result(&json!({ "behavior": "deny", "message": message }))
}

fn verdict_result(verdict: &Value) -> CallToolResult {
    // The permission-prompt-tool reads the result's TEXT content as JSON.
    let text = verdict.to_string();
    CallToolResult::success(vec![Content::text(text)])
}

fn mcp_internal(message: String) -> ErrorData {
    mcp_error(error_codes::TOOL_INTERNAL_ERROR, message)
}

fn now_unix_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| u64::try_from(delta.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn header_value(request_context: &RequestContext<RoleServer>, name: &str) -> Option<String> {
    let parts = request_context
        .extensions
        .get::<axum::http::request::Parts>()?;
    parts
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    //! In-process gate FSV: a real `SynapseService` over a temp RocksDB. We
    //! drive `run_gate` with synthetic risky input, watch the Pending
    //! `agent_permission` row appear in `CF_KV` (physical source of truth),
    //! decide it through the real dashboard decide path, and assert both the
    //! verdict JSON returned to the (would-be) agent AND the durable row state.
    use std::num::NonZeroUsize;
    use std::path::Path;
    use std::time::Duration;

    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::m2::M2ServiceConfig;
    use crate::m3::M3ServiceConfig;
    use crate::m3::approvals::{ApprovalDecision, ApprovalKind, ApprovalStatus, approval_snapshot};
    use crate::m4::M4ServiceConfig;

    fn service_with_db(path: &Path) -> SynapseService {
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
        .expect("construct service")
    }

    fn verdict_of(result: &CallToolResult) -> Value {
        let text = result
            .content
            .iter()
            .filter_map(|content| content.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("");
        serde_json::from_str(&text).expect("verdict is JSON")
    }

    /// Wait for the single pending `agent_permission` approval to appear and
    /// return its id (the gate generates the id internally).
    async fn await_pending_id(service: &SynapseService) -> String {
        let db = service.m3_storage().expect("storage");
        for _ in 0..200 {
            let pending = approval_snapshot(&db, Some(ApprovalKind::AgentPermission)).expect("snap");
            if let Some(item) = pending.into_iter().next() {
                return item.item.approval_id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no pending agent_permission approval ever appeared");
    }

    #[tokio::test]
    async fn auto_allows_safe_tool_without_creating_a_row() {
        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let result = service
            .run_gate("Read", &json!({"file":"x"}), Some("t1"), "sess", Some("agent-spawn-a"))
            .await
            .expect("gate");
        let verdict = verdict_of(&result);
        assert_eq!(verdict["behavior"], "allow");
        // Physical SoT: no approval row was created for an auto-allowed call.
        let db = service.m3_storage().expect("storage");
        let pending = approval_snapshot(&db, Some(ApprovalKind::AgentPermission)).expect("snap");
        assert!(pending.is_empty(), "auto-allow must not enqueue an approval");
    }

    #[tokio::test]
    async fn risky_tool_blocks_then_resumes_allow_on_approval() {
        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let input = json!({ "command": "git push origin main" });

        let gate = service.run_gate("Bash", &input, Some("tuse-1"), "sess", Some("agent-spawn-b"));
        let decide = async {
            let id = await_pending_id(&service).await;
            // Physical SoT before deciding: row is Pending, destructive, gate kind.
            let db = service.m3_storage().expect("storage");
            let item = crate::m3::approvals::get_approval(&db, &id)
                .expect("read")
                .expect("exists")
                .item;
            assert_eq!(item.status, ApprovalStatus::Pending);
            assert_eq!(item.kind, ApprovalKind::AgentPermission);
            assert!(item.destructive, "git push must be flagged destructive");
            service
                .approval_decide_from_dashboard(&id, ApprovalDecision::Accept, None, "tester")
                .expect("decide");
            id
        };
        let (result, id) = tokio::join!(gate, decide);
        let verdict = verdict_of(&result.expect("gate ok"));
        assert_eq!(verdict["behavior"], "allow");
        assert_eq!(verdict["updatedInput"]["command"], "git push origin main");
        // Physical SoT after: durable row is Accepted.
        let db = service.m3_storage().expect("storage");
        let item = crate::m3::approvals::get_approval(&db, &id)
            .expect("read")
            .expect("exists")
            .item;
        assert_eq!(item.status, ApprovalStatus::Accepted);
    }

    #[tokio::test]
    async fn risky_tool_resumes_deny_on_decline_with_reason() {
        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let input = json!({ "command": "rm -rf build" });

        let gate = service.run_gate("Bash", &input, Some("tuse-2"), "sess", Some("agent-spawn-c"));
        let decide = async {
            let id = await_pending_id(&service).await;
            service
                .approval_decide_from_dashboard(
                    &id,
                    ApprovalDecision::Decline,
                    Some("not safe right now"),
                    "tester",
                )
                .expect("decide");
            id
        };
        let (result, id) = tokio::join!(gate, decide);
        let verdict = verdict_of(&result.expect("gate ok"));
        assert_eq!(verdict["behavior"], "deny");
        assert_eq!(verdict["message"], "not safe right now");
        let db = service.m3_storage().expect("storage");
        let item = crate::m3::approvals::get_approval(&db, &id)
            .expect("read")
            .expect("exists")
            .item;
        assert_eq!(item.status, ApprovalStatus::Declined);
    }

    #[tokio::test]
    async fn deadline_denies_and_records_timeout_decline() {
        // SAFETY: single-threaded within this async test; the small value only
        // shortens THIS gate's block. Other gate tests decide in <1s so a leaked
        // value cannot cause a false timeout there.
        unsafe {
            std::env::set_var("SYNAPSE_APPROVAL_GATE_TIMEOUT_MS", "1000");
        }
        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let result = service
            .run_gate(
                "Bash",
                &json!({ "command": "curl https://evil.example" }),
                Some("tuse-3"),
                "sess",
                Some("agent-spawn-d"),
            )
            .await
            .expect("gate");
        unsafe {
            std::env::remove_var("SYNAPSE_APPROVAL_GATE_TIMEOUT_MS");
        }
        let verdict = verdict_of(&result);
        assert_eq!(verdict["behavior"], "deny");
        assert!(
            verdict["message"].as_str().unwrap_or_default().contains("timed out"),
            "timeout deny must explain itself: {verdict}"
        );
        // Physical SoT: the gate recorded its own timeout decline.
        let db = service.m3_storage().expect("storage");
        let pending = approval_snapshot(&db, Some(ApprovalKind::AgentPermission)).expect("snap");
        assert!(pending.is_empty(), "timed-out approval must no longer be pending");
    }
}
