import type { DashboardState } from "@/lib/dashboard-state";

const baseTime = Date.UTC(2026, 5, 13, 16, 0, 0);

function panel<T>(source: string, data: T) {
  return { status: "ok" as const, source, data };
}

export function dashboardFixture(kind: "populated" | "empty" = "populated"): DashboardState {
  const empty = kind === "empty";
  const sessions = empty
    ? []
    : [
        liveSession("agent-codex-001", "codex", "tools/call:approval_request", 1200, "awaiting_approval"),
        liveSession("agent-local-002", "local-model", "tools/call:agent_wait", 45000, "needs_input"),
        liveSession("agent-shell-003", "codex", "tools/call:act_run_shell", 310000, "stale")
      ];
  const unbound = empty
    ? []
    : [
        {
          agent_kind: "codex",
          anchor: "agent-spawn-closed-004",
          spawn_id: "agent-spawn-closed-004",
          state: "dead",
          reason_code: "completed"
        }
      ];

  return {
    schema_version: 1,
    generated_at_unix_ms: baseTime,
    bind_addr: "127.0.0.1:7700",
    token_policy: "httpOnly",
    daemon: panel("health", {
      version: "0.1.0",
      pid: 45352,
      build: "dev",
      tool_count: 112,
      subsystems: {
        storage: { status: "ok" },
        perception: { capture_runtime: { status: "inactive" } }
      }
    }),
    sessions: panel("session_list", {
      sessions,
      unbound_agent_states: unbound
    }),
    lease: panel("control_lease_status", {
      held: false,
      owner_session_id: null,
      ttl_ms: null,
      expires_in_ms: null
    }),
    storage: panel("storage_inspect", {
      schema_version: 1,
      audit_retention_policy_count: 12,
      pressure_level: { name: "Normal" },
      cf_row_counts: empty
        ? {}
        : {
            CF_ACTION_LOG: 7400,
            CF_AGENT_TRANSCRIPTS: 837,
            CF_SESSIONS: 279,
            CF_KV: 587,
            CF_TIMELINE: 6739
          }
    }),
    command_audit: panel("audit_intelligence_query", {
      row_count: empty ? 0 : 3,
      scanned_rows: empty ? 0 : 64,
      rows: empty
        ? []
        : [
            toolRow("act_run_shell", "ok", "", "agent-codex-001", "daemon", 1),
            toolRow("approval_request", "running", "", "agent-codex-001", "human", 2),
            toolRow("agent_wait", "error", "ACTION_BUDGET_EXPIRED", "agent-local-002", "daemon", 3)
          ]
    }),
    approvals: panel("approval_list", { rows: empty ? [] : [{ item: { status: "pending", kind: "shell", title: "Review command", body: "Approve durable action", updated_at_unix_ms: baseTime } }] }),
    suggestions: panel("suggestions", { rows: [] }),
    armed_runs: panel("armed_runs", { rows: [] }),
    agent_transcripts: panel("agent_transcripts", {
      row_count: empty ? 0 : 2,
      rows: empty
        ? []
        : [
            transcriptRow("agent-codex-001", 1, "assistant", "local.assistant.message", "Need approval before editing the plan artifact."),
            transcriptRow("agent-local-002", 2, "assistant", "local.assistant.message", "Local model response sanitized; details are in raw disclosure.")
          ]
    }),
    hygiene: panel("hygiene_flags", { rows: [], scanned_rows: 0, next_cursor: "" }),
    local_models: panel("local_model_list", {
      enabled_count: 1,
      unhealthy_count: 0,
      rows: [
        {
          name: "ollama-gemma4-e4b",
          model_id: "gemma4:e4b",
          base_url: "http://127.0.0.1:11434/v1",
          enabled: true,
          last_probe: { healthy: true, checked_at: "2026-06-13T16:00:00Z" },
          notes: "Stable cached local model row"
        }
      ]
    })
  };
}

export function attentionAgent() {
  return {
    id: "agent-codex-001",
    kind: "codex",
    lifecycle: "live",
    status: "needs_input" as const,
    summary: "Awaiting operator approval for shell action",
    lastSeenMs: 1200,
    lastAction: "tools/call:approval_request",
    target: "dashboard",
    reason: "approval_required",
    diffStats: { events: 4, transcripts: 2, actions: 6 },
    raw: { session_id: "agent-codex-001", state: "needs_input", reason_code: "approval_required" }
  };
}

export function toolCall(kind: "success" | "error" = "success") {
  return {
    id: `tool-${kind}`,
    tool: kind === "success" ? "act_run_shell" : "agent_wait",
    lifecycle: kind,
    summary: kind === "success" ? "wrote marker and verified bytes" : "budget expired with actionable error",
    actor: "agent-codex-001",
    target: "daemon",
    time: String(baseTime * 1_000_000),
    raw: {
      tool: kind === "success" ? "act_run_shell" : "agent_wait",
      args_sha256: "4f4b6d9c8a8d",
      response: { status: kind, details: "Raw verification stays behind disclosure." }
    }
  } as const;
}

export function transcriptSample() {
  return {
    spawn_id: "agent-codex-001",
    line_no: 7,
    record: {
      role: "assistant",
      event_kind: "local.assistant.message",
      content_summary: "Rendered **Markdown** is sanitized and raw rows stay collapsed.",
      tool_calls: [toolCall("success")]
    }
  };
}

function liveSession(id: string, kind: string, action: string, lastSeenMs: number, reason: string) {
  return {
    session_id: id,
    agent_kind: kind,
    lifecycle: "live",
    transport: "http",
    last_seen_ms_ago: lastSeenMs,
    last_action: action,
    agent_state: { state: "live", reason_code: reason }
  };
}

function toolRow(tool: string, outcome: string, errorCode: string, actor: string, target: string, index: number) {
  return {
    key_hex: `issue947-${index}`,
    ts_ns: String((baseTime + index * 1000) * 1_000_000),
    actor_session_id: actor,
    target_session_id: target,
    verb: tool,
    tool,
    channel: "mcp",
    phase: "after",
    outcome,
    error_code: errorCode,
    payload_sha256: `sha-${index}`
  };
}

function transcriptRow(spawnId: string, lineNo: number, role: string, event: string, content: string) {
  return {
    spawn_id: spawnId,
    line_no: lineNo,
    record: {
      role,
      event_kind: event,
      model: "gemma4:e4b",
      content_summary: content
    }
  };
}
