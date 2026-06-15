import { asArray, asRecord, rawText } from "@/lib/utils";

export interface DashboardPanel<T = unknown> {
  status: "ok" | "error" | "unavailable";
  source: string;
  data: T;
  error?: string;
}

export interface DashboardState {
  schema_version: number;
  generated_at_unix_ms: number;
  bind_addr: string;
  token_policy: string;
  auth: DashboardPanel;
  daemon: DashboardPanel;
  sessions: DashboardPanel;
  lease: DashboardPanel;
  storage: DashboardPanel;
  target_claims: DashboardPanel;
  timeline: DashboardPanel;
  events: DashboardPanel;
  hidden_desktops: DashboardPanel;
  cdp_attachments: DashboardPanel;
  shell_jobs: DashboardPanel;
  command_audit: DashboardPanel;
  approvals: DashboardPanel;
  suggestions: DashboardPanel;
  armed_runs: DashboardPanel;
  agent_transcripts: DashboardPanel;
  hygiene: DashboardPanel;
  local_models: DashboardPanel;
}

export interface AgentSummary {
  id: string;
  spawnId?: string;
  killId?: string;
  killable?: boolean;
  kind: string;
  lifecycle: string;
  status: FleetStatus;
  summary: string;
  lastSeenMs?: number;
  lastAction?: string;
  target?: string;
  reason?: string;
  diffStats: {
    events: number;
    transcripts: number;
    actions: number;
  };
  raw: unknown;
}

export type FleetStatus =
  | "working"
  | "idle"
  | "ready_for_review"
  | "needs_input"
  | "awaiting_approval"
  | "stuck"
  | "done";

export interface ToolCallSummary {
  id: string;
  tool: string;
  lifecycle: "pending" | "running" | "success" | "error";
  summary: string;
  actor?: string;
  target?: string;
  time?: string;
  raw: unknown;
}

export interface DashboardAuthStatus {
  ok: boolean;
  authenticated: boolean;
  method: "cookie" | "bearer";
  csrf_token?: string;
  expires_unix_ms?: number;
  source_of_truth: string;
}

export interface DashboardSavedView {
  schema_version: number;
  view_id: string;
  row_key: string;
  name: string;
  route: string;
  filters: Record<string, unknown>;
  created_unix_ms: number;
  updated_unix_ms: number;
}

export interface DashboardSavedViewsResponse {
  ok: boolean;
  source_of_truth: string;
  views: DashboardSavedView[];
  corrupt_row_count: number;
}

export interface AuditQueryFilters {
  limit?: number | string;
  scan_limit?: number | string;
  cursor?: string;
  start_ts_ns?: number | string;
  end_ts_ns?: number | string;
  session_id?: string;
  tool?: string;
  status?: string;
  error_code?: string;
  row_kind?: string;
}

export interface AuditQueryRow {
  key_hex: string;
  value_len_bytes: number;
  value_sha256: string;
  row_kind: string;
  audit_id: string;
  ts_ns: number;
  ts_ns_text?: string;
  phase?: string | null;
  status?: string | null;
  outcome?: string | null;
  session_id?: string | null;
  actor_session_id?: string | null;
  target_session_id?: string | null;
  tool: string;
  verb?: string | null;
  channel?: string | null;
  error_code?: string | null;
  payload_sha256?: string | null;
  payload_truncated?: boolean | null;
  source_of_truth: Record<string, unknown>;
  record: Record<string, unknown>;
}

export interface AuditQueryResponse {
  source_of_truth: string;
  cf_name: string;
  filters: Record<string, unknown>;
  limit: number;
  scan_limit: number;
  scanned_rows: number;
  matched_rows: number;
  returned_count: number;
  corrupt_row_count: number;
  partial: boolean;
  exhausted: boolean;
  start_key_hex?: string | null;
  next_start_key_hex?: string | null;
  rows: AuditQueryRow[];
}

export interface SaveDashboardViewRequest {
  view_id?: string;
  name: string;
  route: string;
  filters: Record<string, unknown>;
}

let csrfToken: string | null = null;

export function dashboardCsrfToken() {
  return csrfToken;
}

function setCsrfToken(value?: string | null) {
  csrfToken = value || null;
}

export async function fetchDashboardAuthStatus(): Promise<DashboardAuthStatus> {
  const response = await fetch("/dashboard/auth/status", {
    cache: "no-store",
    credentials: "same-origin"
  });
  if (response.status === 401) {
    setCsrfToken(null);
    return {
      ok: false,
      authenticated: false,
      method: "cookie",
      source_of_truth: "CF_KV dashboard-auth/v1"
    };
  }
  if (!response.ok) {
    throw new Error(`dashboard auth failed: ${response.status}`);
  }
  const status = (await response.json()) as DashboardAuthStatus;
  setCsrfToken(status.csrf_token);
  return status;
}

export async function loginDashboard(credential: string): Promise<DashboardAuthStatus> {
  const response = await fetch("/dashboard/auth/login", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: {
      "Content-Type": "application/json"
    },
    body: JSON.stringify({ credential })
  });
  if (!response.ok) {
    setCsrfToken(null);
    throw new Error(`dashboard login failed: ${response.status}`);
  }
  const status = (await response.json()) as DashboardAuthStatus;
  setCsrfToken(status.csrf_token);
  return status;
}

export async function logoutDashboard(): Promise<void> {
  const headers: Record<string, string> = {};
  if (csrfToken) headers["X-CSRF-Token"] = csrfToken;
  const response = await fetch("/dashboard/auth/logout", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers
  });
  setCsrfToken(null);
  if (!response.ok) {
    throw new Error(`dashboard logout failed: ${response.status}`);
  }
}

export interface ModelRow {
  name: string;
  base_url: string;
  model_id: string;
  api_shape: string;
  runtime_preset?: string;
  enabled: boolean;
  allow_non_loopback: boolean;
  api_key_env_var?: string | null;
  context_length?: number | null;
  max_tools?: number | null;
  /** Whether an encrypted API key is stored at rest for this model. */
  has_api_key_secret?: boolean;
  last_probe?: { healthy?: boolean; status?: string; latency_ms?: number } | null;
}

export interface RegisterApiModelRequest {
  name: string;
  base_url: string;
  model_id: string;
  runtime_preset: string;
  api_key_env_var: string;
  /** Plaintext API key; encrypted at rest (DPAPI) by the daemon. Never stored or returned in plaintext. */
  api_key?: string;
  context_length?: number;
  max_tools?: number;
  notes?: string;
  probe_timeout_ms?: number;
}

export interface SpawnRequest {
  model_ref: string;
  prompt: string;
  working_dir?: string;
  wait_timeout_ms?: number;
  hold_open_ms?: number;
}

export interface AgentTemplateRow {
  schema_version: number;
  template_id: string;
  version: number;
  name: string;
  agent_kind: "claude" | "codex" | "local_model" | string;
  model?: string | null;
  model_ref?: string | null;
  prompt_template?: string | null;
  required_params: string[];
  working_dir?: string | null;
  config_hash: string;
  created_unix_ms: number;
  updated_unix_ms: number;
}

export interface TemplateListResponse {
  ok: boolean;
  source_of_truth: string;
  list: {
    ok: boolean;
    count: number;
    templates: AgentTemplateRow[];
  };
}

export interface SpawnAgentRequest {
  fan_out?: number;
  template_id?: string;
  template_version?: number;
  template_params?: Record<string, string>;
  cli?: "codex" | "claude" | "local_model";
  kind?: "codex" | "claude" | "local_model";
  model?: string;
  model_ref?: string;
  prompt?: string;
  target?: { kind: "window"; window_hwnd: number } | { kind: "cdp"; window_hwnd: number; cdp_target_id: string };
  working_dir?: string;
  wait_timeout_ms?: number;
  hold_open_ms?: number;
}

export interface SpawnAgentAttempt {
  index: number;
  status: "ok" | "error";
  spawn?: Record<string, unknown>;
  error_code?: string;
  message?: string;
  data?: unknown;
}

export interface SpawnAgentResponse {
  ok: boolean;
  trigger: string;
  source_of_truth: string;
  requested_count: number;
  succeeded_count: number;
  failed_count: number;
  attempts: SpawnAgentAttempt[];
}

export interface AgentKillRequest {
  session_id: string;
  grace_ms?: number;
  interrupt_first?: boolean;
}

export interface AgentKillResponse {
  ok: boolean;
  trigger: string;
  source_of_truth: string;
  kill: Record<string, unknown>;
}

export interface TimelineControlResponse {
  ok: boolean;
  trigger: string;
  source_of_truth: string;
  readback: Record<string, unknown>;
}

export interface LeaseForceReleaseRequest {
  owner_session_id: string;
  confirmed: boolean;
}

export interface LeaseHandoffRequest {
  from_session_id: string;
  to_session_id: string;
  ttl_ms?: number;
}

export interface DashboardControlResponse {
  ok: boolean;
  trigger: string;
  source_of_truth: string;
  readback: Record<string, unknown>;
}

export type DashboardRouteReadback = Record<string, unknown>;

function csrfHeaders(): Record<string, string> {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (csrfToken) headers["X-CSRF-Token"] = csrfToken;
  return headers;
}

async function readJsonOrThrow(response: Response): Promise<Record<string, unknown>> {
  let body: Record<string, unknown> = {};
  try {
    body = (await response.json()) as Record<string, unknown>;
  } catch {
    /* non-JSON error body */
  }
  if (!response.ok || body.ok === false) {
    const code = typeof body.code === "string" ? body.code : `HTTP ${response.status}`;
    const message = typeof body.message === "string" ? body.message : response.statusText;
    throw new Error(`${code}: ${message}`);
  }
  return body;
}

export async function fetchModels(): Promise<ModelRow[]> {
  const response = await fetch("/dashboard/models", {
    cache: "no-store",
    credentials: "same-origin"
  });
  const body = await readJsonOrThrow(response);
  const list = (body.list ?? {}) as { rows?: ModelRow[] };
  return list.rows ?? [];
}

export async function fetchTemplates(): Promise<AgentTemplateRow[]> {
  const response = await fetch("/dashboard/templates", {
    cache: "no-store",
    credentials: "same-origin"
  });
  const body = await readJsonOrThrow(response);
  const list = (body.list ?? {}) as { templates?: AgentTemplateRow[] };
  return list.templates ?? [];
}

export async function fetchSavedViews(): Promise<DashboardSavedViewsResponse> {
  const response = await fetch("/dashboard/saved-views", {
    cache: "no-store",
    credentials: "same-origin"
  });
  const body = await readJsonOrThrow(response);
  return {
    ok: body.ok === true,
    source_of_truth: rawText(body.source_of_truth),
    views: asArray<DashboardSavedView>(body.views),
    corrupt_row_count: Number(body.corrupt_row_count || 0)
  };
}

export async function fetchAuditQuery(filters: AuditQueryFilters): Promise<AuditQueryResponse> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value === undefined || value === null || value === "") continue;
    params.set(key, String(value));
  }
  const suffix = params.toString();
  const response = await fetch(`/dashboard/audit/query${suffix ? `?${suffix}` : ""}`, {
    cache: "no-store",
    credentials: "same-origin"
  });
  const body = await readJsonOrThrow(response);
  const query = asRecord(body.query) as unknown as AuditQueryResponse;
  return {
    ...query,
    rows: asArray<AuditQueryRow>(query.rows)
  };
}

export async function saveDashboardView(
  request: SaveDashboardViewRequest
): Promise<{ ok: boolean; row_key: string; source_of_truth: string; view: DashboardSavedView }> {
  const response = await fetch("/dashboard/saved-views", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return {
    ok: body.ok === true,
    row_key: rawText(body.row_key),
    source_of_truth: rawText(body.source_of_truth),
    view: body.view as DashboardSavedView
  };
}

export async function deleteDashboardView(viewId: string): Promise<{ ok: boolean; deleted_row_key: string; source_of_truth: string }> {
  const response = await fetch(`/dashboard/saved-views/${encodeURIComponent(viewId)}`, {
    method: "DELETE",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders()
  });
  const body = await readJsonOrThrow(response);
  return {
    ok: body.ok === true,
    deleted_row_key: rawText(body.deleted_row_key),
    source_of_truth: rawText(body.source_of_truth)
  };
}

export async function registerApiModel(
  request: RegisterApiModelRequest
): Promise<DashboardRouteReadback> {
  const response = await fetch("/dashboard/api-model/register", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders(),
    body: JSON.stringify(request)
  });
  return readJsonOrThrow(response);
}

export async function spawnLocalModelAgent(
  request: SpawnRequest
): Promise<DashboardRouteReadback> {
  const response = await fetch("/dashboard/local-model-spawn", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders(),
    body: JSON.stringify(request)
  });
  return readJsonOrThrow(response);
}

export async function spawnAgent(request: SpawnAgentRequest): Promise<SpawnAgentResponse> {
  const response = await fetch("/dashboard/spawn-agent", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as SpawnAgentResponse;
}

export async function killAgent(request: AgentKillRequest): Promise<AgentKillResponse> {
  const response = await fetch("/dashboard/agent-kill", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as AgentKillResponse;
}

export type ApprovalDecisionVerb = "approve" | "deny";

export interface ApprovalDecideRequest {
  approval_id: string;
  decision: ApprovalDecisionVerb;
  note?: string;
}

export interface ApprovalDecideResponse {
  ok: boolean;
  trigger: string;
  source_of_truth: string;
  decision: Record<string, unknown>;
}

// Resolve one pending approval from the inbox (#927). For an `agent_permission`
// row this unblocks the still-running agent's permission_gate call.
export async function decideApproval(
  request: ApprovalDecideRequest
): Promise<ApprovalDecideResponse> {
  const response = await fetch("/dashboard/approval/decide", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as ApprovalDecideResponse;
}

export async function pauseTimeline(duration_ms?: number): Promise<TimelineControlResponse> {
  const response = await fetch("/dashboard/timeline/pause", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders(),
    body: JSON.stringify({ duration_ms })
  });
  return (await readJsonOrThrow(response)) as unknown as TimelineControlResponse;
}

export async function resumeTimeline(): Promise<TimelineControlResponse> {
  const response = await fetch("/dashboard/timeline/resume", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders()
  });
  return (await readJsonOrThrow(response)) as unknown as TimelineControlResponse;
}

export async function forceReleaseLease(
  request: LeaseForceReleaseRequest
): Promise<DashboardControlResponse> {
  const response = await fetch("/dashboard/control-lease/force-release", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as DashboardControlResponse;
}

export async function handoffLease(request: LeaseHandoffRequest): Promise<DashboardControlResponse> {
  const response = await fetch("/dashboard/control-lease/handoff", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as DashboardControlResponse;
}

export interface TimelinePurgeRequest {
  /** Inclusive lower bound, epoch ns as a decimal string (avoids JS precision loss). */
  start_ts_ns?: string;
  /** Inclusive upper bound, epoch ns as a decimal string. */
  end_ts_ns?: string;
  kinds?: string[];
  apps?: string[];
  actor?: string;
  all?: boolean;
  dry_run?: boolean;
  cursor?: string;
}

export interface TimelinePurgeReadback {
  matched_rows: number;
  deleted_rows: number;
  scanned_rows: number;
  invalid_rows: number;
  protected_audit_rows: number;
  dry_run: boolean;
  audit_key_hex?: string;
  compacted: boolean;
  next_cursor?: string;
  stopped_because: string;
}

export interface StorageGcRequest {
  cf_name: string;
  soft_cap_rows: number;
  hard_cap_rows: number;
}

export interface StorageGcCfReport {
  cf_name: string;
  before_value: number;
  after_value: number;
  evicted_rows: number;
  hard_cap_reached: boolean;
  hard_cap_code?: string | null;
}

export interface StorageGcReadback {
  cf_name: string;
  before_rows: number;
  after_rows: number;
  total_evicted_rows: number;
  cache_evictions_total_delta: number;
  cf_reports: StorageGcCfReport[];
  audit_retention_report_key?: string;
}

export async function purgeTimelineRecordings(request: TimelinePurgeRequest): Promise<TimelinePurgeReadback> {
  const response = await fetch("/dashboard/storage/timeline-purge", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as TimelinePurgeReadback;
}

export async function runStorageGc(request: StorageGcRequest): Promise<StorageGcReadback> {
  const response = await fetch("/dashboard/storage/gc", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as StorageGcReadback;
}

export async function pruneTargetClaims(): Promise<DashboardControlResponse> {
  const response = await fetch("/dashboard/target-claims/prune", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: csrfHeaders()
  });
  return (await readJsonOrThrow(response)) as unknown as DashboardControlResponse;
}

export async function fetchDashboardState(): Promise<DashboardState> {
  const response = await fetch("/dashboard/state.json", {
    cache: "no-store",
    credentials: "same-origin"
  });
  if (!response.ok) {
    throw new Error(`dashboard state failed: ${response.status}`);
  }
  return response.json();
}

export function panelData<T = Record<string, unknown>>(panel?: DashboardPanel): T {
  return (panel?.data ?? {}) as T;
}

export function buildAgents(state?: DashboardState): AgentSummary[] {
  if (!state) return [];
  const sessionData = asRecord(state.sessions.data);
  const sessionRows = asArray<Record<string, unknown>>(sessionData.sessions);
  const unbound = asArray<Record<string, unknown>>(sessionData.unbound_agent_states);
  const transcripts = asArray<Record<string, unknown>>(asRecord(state.agent_transcripts.data).rows);
  const actions = asArray<Record<string, unknown>>(asRecord(state.command_audit.data).rows);

  const transcriptCounts = new Map<string, number>();
  for (const row of transcripts) {
    const spawnId = rawText(row.spawn_id);
    if (spawnId) transcriptCounts.set(spawnId, (transcriptCounts.get(spawnId) ?? 0) + 1);
  }

  const actionCounts = new Map<string, number>();
  for (const row of actions) {
    const target = rawText(row.target_session_id);
    const actor = rawText(row.actor_session_id);
    for (const id of [target, actor]) {
      if (id) actionCounts.set(id, (actionCounts.get(id) ?? 0) + 1);
    }
  }

  const live = sessionRows.map((row) => {
    const agentState = asRecord(row.agent_state);
    const sessionId = rawText(row.session_id);
    const spawnId = rawText(row.spawn_id || agentState.spawn_id);
    const stateName = rawText(agentState.state || row.lifecycle);
    const lastSeenMs = Number(row.last_seen_ms_ago);
    const lastAction = rawText(row.last_action);
    return {
      id: sessionId,
      spawnId: spawnId || undefined,
      killId: spawnId || sessionId,
      killable: true,
      kind: rawText(row.agent_kind || row.client_name || "agent"),
      lifecycle: rawText(row.lifecycle),
      status: statusFromLiveSession(stateName, lastSeenMs, lastAction),
      summary: lastAction || stateName || "session live",
      lastSeenMs: Number.isFinite(lastSeenMs) ? lastSeenMs : undefined,
      lastAction,
      target: row.active_target ? rawText(row.active_target) : "",
      reason: rawText(agentState.reason_code),
      diffStats: {
        events: 1,
        transcripts: transcriptCounts.get(sessionId) ?? 0,
        actions: actionCounts.get(sessionId) ?? 0
      },
      raw: row
    } satisfies AgentSummary;
  });

  const historical = unbound.map((row) => {
    const id = rawText(row.session_id || row.spawn_id || row.anchor);
    const spawnId = rawText(row.spawn_id);
    const stateName = rawText(row.state);
    const reason = rawText(row.reason_code);
    return {
      id,
      spawnId: spawnId || undefined,
      killId: spawnId || id,
      killable: false,
      kind: rawText(row.agent_kind || "agent"),
      lifecycle: stateName || "unbound",
      status: statusFromHistorical(stateName, reason),
      summary: [stateName, reason].filter(Boolean).join(" / ") || "historical state",
      lastSeenMs: undefined,
      lastAction: "",
      target: "",
      reason,
      diffStats: {
        events: 1,
        transcripts: transcriptCounts.get(id) ?? 0,
        actions: actionCounts.get(id) ?? 0
      },
      raw: row
    } satisfies AgentSummary;
  });

  return [...live, ...historical].filter((agent) => agent.id);
}

export function buildToolCalls(state?: DashboardState): ToolCallSummary[] {
  if (!state) return [];
  const rows = asArray<Record<string, unknown>>(asRecord(state.command_audit.data).rows);
  return rows.slice(0, 16).map((row, index) => {
    const error = rawText(row.error_code);
    const outcome = rawText(row.outcome);
    return {
      id: rawText(row.key_hex) || `${index}`,
      tool: rawText(row.tool || row.verb || "tool"),
      lifecycle: error ? "error" : outcome === "ok" ? "success" : "running",
      summary: [rawText(row.verb), outcome, error].filter(Boolean).join(" / ") || "tool call",
      actor: rawText(row.actor_session_id),
      target: rawText(row.target_session_id),
      time: rawText(row.ts_ns),
      raw: row
    };
  });
}

function statusFromLiveSession(stateName: string, lastSeenMs: number, lastAction: string): FleetStatus {
  if (Number.isFinite(lastSeenMs) && lastSeenMs > 300000) return "stuck";
  if (/approval/i.test(lastAction)) return "awaiting_approval";
  if (/wait|inbox|input/i.test(lastAction)) return "needs_input";
  if (/review/i.test(lastAction)) return "ready_for_review";
  if (/idle/i.test(stateName)) return "idle";
  return "working";
}

function statusFromHistorical(stateName: string, reason: string): FleetStatus {
  if (/failed|unhealthy|missing|error|interrupted|timeout/i.test(`${stateName} ${reason}`)) return "stuck";
  if (/dead|done|exited|closed/i.test(stateName)) return "done";
  return "idle";
}
