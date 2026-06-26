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
  dashboard_assets?: DashboardPanel<DashboardAssetSurface>;
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
  tasks?: DashboardPanel;
  approvals: DashboardPanel;
  suggestions: DashboardPanel;
  armed_runs: DashboardPanel;
  agent_transcripts: DashboardPanel;
  agent_cost?: DashboardPanel;
  agent_stats?: DashboardPanel;
  context: DashboardPanel;
  hygiene: DashboardPanel;
  local_models: DashboardPanel;
}

export interface DashboardAssetSurface {
  schema_version: number;
  source_of_truth: string;
  js_file: string;
  css_file: string;
}

export interface DashboardAssetReloadDecision {
  shouldReload: boolean;
  reason:
    | "asset_match"
    | "asset_mismatch"
    | "missing_server_asset_id"
    | "invalid_server_asset_id"
    | "missing_client_asset_id"
    | "state_unavailable";
  expectedJsFile?: string;
  currentJsFile?: string;
  reloadKey?: string;
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
  usage?: AgentUsageSummary;
  raw: unknown;
}

export interface AgentUsageSummary {
  inputTokens: number;
  outputTokens: number;
  cacheReadInputTokens: number;
  cacheCreationInputTokens: number;
  reasoningOutputTokens: number;
  totalCostMicroUsd?: number;
  sourceLine?: number;
}

export interface AttachedAgentRegistry {
  source_of_truth?: string;
  count_basis?: string;
  generated_at_unix_ms?: number;
  exact_live_count?: number;
  row_count?: number;
  killable_live_count?: number;
  unprobeable_observed_count?: number;
  rows?: AttachedAgentRegistryRow[];
  window_lookup_error?: string;
}

export interface AttachedAgentRegistryRow {
  registry_id?: string;
  kind?: string;
  source?: string;
  lifecycle?: string;
  state?: string;
  reason_code?: string;
  counts_as_live?: boolean;
  not_counted_reason?: string;
  session_id?: string;
  spawn_id?: string;
  spawn_dir?: string;
  last_seen_unix_ms?: number;
  last_seen_ms_ago?: number;
  process?: {
    probeable?: boolean;
    launcher_process_id?: number;
    agent_process_id?: number;
    parent_process_id?: number;
    process_name?: string;
    command_line?: string;
    cwd?: string;
    process_tree_ids?: number[];
    live_process_ids?: number[];
  };
  visible_window?: {
    window_hwnd?: number;
    process_id?: number;
    process_name?: string;
    window_title?: string;
  };
  controlling_terminal_window?: {
    window_hwnd?: number;
    process_id?: number;
    process_name?: string;
    window_title?: string;
  };
  kill_handle?: {
    available?: boolean;
    kind?: string;
    target_id?: string;
    reason?: string;
  };
}

export type FleetStatus =
  | "working"
  | "idle"
  | "ready_for_review"
  | "needs_input"
  | "awaiting_approval"
  | "stuck"
  | "failed"
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

export interface AgentEventsQueryRequest {
  limit?: number | string;
  scan_limit?: number | string;
  start_ts_ns?: number | string;
  end_ts_ns?: number | string;
  spawn_id?: string;
  session_id?: string;
  kind?: string;
}

export interface AgentEventQueryRow {
  key_hex: string;
  ts_ns: number;
  seq: number;
  kind: string;
  spawn_id?: string | null;
  session_id?: string | null;
  record: Record<string, unknown>;
}

export interface AgentEventsQueryResponse {
  ok: boolean;
  source_of_truth: string;
  filters: Record<string, unknown>;
  limit: number;
  scan_limit: number;
  scanned_rows: number;
  matched_rows: number;
  returned_count: number;
  corrupt_row_count: number;
  partial: boolean;
  exhausted: boolean;
  rows: AgentEventQueryRow[];
}

export interface AgentRecordingMetadata {
  schema_version: number;
  source: string;
  log_dir: string;
  asciicast_path: string;
  status_path: string;
  final_screen_path: string;
  input_audit_path: string;
  asciicast_bytes: number;
  status_bytes: number;
  final_screen_bytes: number;
  input_audit_bytes: number;
  status?: Record<string, unknown> | null;
  capture_status: string;
  exit_code?: number | null;
  bytes_captured?: number | null;
  output_events?: number | null;
  duration_secs: number;
  recording_truncated: boolean;
  response_truncated: boolean;
  crash_declared: boolean;
  missing_artifact_count: number;
}

export interface AsciicastReplayEvent {
  time_secs: number;
  interval_secs: number;
  code: string;
  data: unknown;
}

export interface AsciicastReplayReadback {
  header: Record<string, unknown>;
  events: AsciicastReplayEvent[];
  returned_event_count: number;
  parsed_event_count: number;
  corrupt_event_count: number;
  output_event_count: number;
  marker_event_count: number;
  resize_event_count: number;
  input_event_count: number;
  exit_code?: number | null;
  duration_secs: number;
  response_truncated: boolean;
  recording_truncated: boolean;
}

export interface AgentRecordingResponse {
  ok: boolean;
  source_of_truth: string;
  spawn_id: string;
  session_id?: string | null;
  agent_kind: string;
  lifecycle: string;
  metadata: AgentRecordingMetadata;
  asciicast: AsciicastReplayReadback;
  journal: AgentEventsQueryResponse;
}

export interface SaveDashboardViewRequest {
  view_id?: string;
  name: string;
  route: string;
  filters: Record<string, unknown>;
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
  last_probe?: {
    healthy?: boolean;
    status?: string;
    latency_ms?: number;
    observed_at_unix_ms?: number;
    error_code?: string | null;
    error_phase?: string | null;
    error_kind?: string | null;
    error_detail?: string | null;
  } | null;
}

export const LOCAL_MODEL_SPAWN_MAX_PROBE_AGE_MS = 15 * 60 * 1000;

export type ModelRegistryStatusReason =
  | "disabled"
  | "unprobed"
  | "ready"
  | "stale_probe"
  | "unhealthy"
  | "invalid_probe_timestamp";

export interface ModelRegistryStatusSummary {
  status: FleetStatus;
  reason: ModelRegistryStatusReason;
  label: string;
  launchReady: boolean;
  probeAgeMs: number | null;
}

export function localModelProbeAgeMs(model: ModelRow, nowMs = Date.now()): number | null {
  const observed = model.last_probe?.observed_at_unix_ms;
  return typeof observed === "number" && Number.isFinite(observed) ? Math.max(0, nowMs - observed) : null;
}

export function localModelLaunchReady(model: ModelRow, nowMs = Date.now()): boolean {
  const status = modelRegistryStatus(model, nowMs);
  return status.launchReady;
}

export function localModelProbeLabel(model: ModelRow, nowMs = Date.now()): string {
  return modelRegistryStatus(model, nowMs).label;
}

export function modelRegistryStatus(model: ModelRow, nowMs = Date.now()): ModelRegistryStatusSummary {
  if (!model.enabled) {
    return {
      status: "idle",
      reason: "disabled",
      label: "disabled",
      launchReady: false,
      probeAgeMs: localModelProbeAgeMs(model, nowMs)
    };
  }

  if (!model.last_probe) {
    return {
      status: "needs_input",
      reason: "unprobed",
      label: "unprobed",
      launchReady: false,
      probeAgeMs: null
    };
  }

  const ageMs = localModelProbeAgeMs(model, nowMs);
  if (!model.last_probe.healthy) {
    return {
      status: "failed",
      reason: "unhealthy",
      label: model.last_probe.status || model.last_probe.error_code || "unhealthy",
      launchReady: false,
      probeAgeMs: ageMs
    };
  }

  if (ageMs === null) {
    return {
      status: "needs_input",
      reason: "invalid_probe_timestamp",
      label: "healthy probe missing timestamp",
      launchReady: false,
      probeAgeMs: null
    };
  }

  if (ageMs > LOCAL_MODEL_SPAWN_MAX_PROBE_AGE_MS) {
    return {
      status: "needs_input",
      reason: "stale_probe",
      label: `stale healthy probe (${Math.round(ageMs / 1000)}s old)`,
      launchReady: false,
      probeAgeMs: ageMs
    };
  }

  return {
    status: "done",
    reason: "ready",
    label: model.last_probe.status || "healthy",
    launchReady: true,
    probeAgeMs: ageMs
  };
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
  name: string;
  description: string;
  /** "claude", "codex", or the name of a registered local/API model. */
  model: string;
  directory?: string | null;
  prompt: string;
  config_hash: string;
  created_unix_ms: number;
  updated_unix_ms: number;
}

export type AgentTaskState = "todo" | "in_progress" | "review" | "done" | "cancelled";
export type AgentTaskAttemptOutcome = "pending" | "succeeded" | "failed" | "orphaned";

export interface AgentTaskAttempt {
  attempt_id: number;
  session_id: string;
  spawn_id?: string | null;
  template_version?: number | null;
  outcome: AgentTaskAttemptOutcome;
  started_unix_ms: number;
  ended_unix_ms?: number | null;
  reason?: string | null;
}

export interface AgentTaskRow {
  schema_version: number;
  task_id: string;
  state: AgentTaskState;
  title: string;
  description?: string | null;
  acceptance?: string | null;
  priority: number;
  template_id: string;
  template_params: Record<string, string>;
  enqueue_seq: number;
  attempts: AgentTaskAttempt[];
  review_reason?: string | null;
  created_unix_ms: number;
  updated_unix_ms: number;
}

export interface AgentTaskNextReadback {
  ok: boolean;
  decision: string;
  task?: AgentTaskRow | null;
  in_flight: number;
  concurrency_cap: number;
}

export interface DashboardTaskSurface {
  tool: string;
  available: boolean;
  source_of_truth: string;
  row_count: number;
  tasks: AgentTaskRow[];
  reconciled_orphans: string[];
  next: AgentTaskNextReadback;
}

export interface AgentTaskCreateRequest {
  task_id: string;
  title: string;
  description?: string;
  acceptance?: string;
  priority?: number;
  template_id: string;
  template_params?: Record<string, string>;
}

export interface AgentTaskUpdateRequest {
  task_id: string;
  state?: AgentTaskState;
  reason?: string;
  priority?: number;
  title?: string;
  description?: string;
  acceptance?: string;
}

export interface AgentTaskCancelRequest {
  task_id: string;
  reason?: string;
}

export interface AgentTaskDispatchOnceRequest {
  concurrency_cap?: number;
  wait_timeout_ms?: number;
}

export interface AgentTaskMutationResponse {
  ok: boolean;
  task: AgentTaskRow;
  written_row: {
    cf_name: string;
    row_key: string;
    value_len_bytes: number;
  };
}

export interface AgentTaskDispatchOnceResponse {
  ok: boolean;
  decision: string;
  task?: AgentTaskRow | null;
  spawn?: Record<string, unknown>;
  in_flight: number;
  concurrency_cap: number;
}

export interface AgentTaskCancelResponse {
  ok: boolean;
  cancel: AgentTaskMutationResponse;
  interrupt?: Record<string, unknown>;
}

export interface TemplateUpsertRequest {
  template_id: string;
  name: string;
  description?: string;
  model: string;
  directory?: string;
  prompt: string;
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

export interface AgentBroadcastRequest {
  selector: "all" | "agent_kinds" | "sessions";
  agent_kinds?: string[];
  sessions?: string[];
  kind: string;
  payload: Record<string, unknown>;
  ttl_ms?: number;
  request_receipt?: boolean;
}

export interface AgentBroadcastResponse {
  ok: boolean;
  trigger: string;
  source_of_truth: string;
  broadcast: Record<string, unknown>;
}

export interface FleetStopRequest {
  mode: "kill" | "interrupt";
  confirm: string;
  agent_kinds?: string[];
  grace_ms?: number;
}

export interface FleetStopResponse {
  ok: boolean;
  trigger: string;
  source_of_truth: string;
  fleet_stop: Record<string, unknown>;
}

export interface AgentLookupRequest {
  session_id: string;
}

export interface AgentRespawnRequest {
  session_id: string;
  prompt: string;
  carry_context?: boolean;
  grace_ms?: number;
}

export interface TimelineControlResponse {
  ok: boolean;
  trigger: string;
  source_of_truth: string;
  readback: Record<string, unknown>;
}

export interface TimelineQueryRequest {
  start_ts_ns?: string;
  end_ts_ns?: string;
  apps?: string[];
  text?: string;
  kinds?: string[];
  actor?: string;
  limit?: number;
  cursor?: string;
}

export interface TimelineRow {
  key_hex: string;
  ts_ns: number;
  seq?: number;
  kind: string;
  actor: string;
  app?: string;
  payload: Record<string, unknown>;
}

export interface TimelineGetReadback {
  rows: TimelineRow[];
  scanned_rows: number;
  invalid_rows: number;
  next_cursor?: string;
  stopped_because: string;
}

export interface TimelineSearchReadback {
  matches: TimelineRow[];
  scanned_rows: number;
  invalid_rows: number;
  next_cursor?: string;
  stopped_because: string;
}

export interface TimelineDigestRequest {
  period: "day" | "week";
  date?: string;
  anchor_ts_ns?: number;
  include_agent_activity?: boolean;
  top_n?: number;
}

export interface TimelineDigestReadback {
  period: string;
  period_start_ns: number;
  period_end_ns: number;
  days_covered: number;
  actor_filter: string;
  episode_count: number;
  active_ms: number;
  idle_ms: number;
  first_activity_ns?: number;
  last_activity_ns?: number;
  total_keystrokes: number;
  total_clicks: number;
  total_interruptions: number;
  total_interrupted_ms: number;
  by_app: Array<Record<string, unknown>>;
  by_app_other: Record<string, unknown>;
  top_documents: Array<Record<string, unknown>>;
  top_documents_other: Record<string, unknown>;
  per_day: Array<Record<string, unknown>>;
  routines_touched: Array<Record<string, unknown>>;
  episodes_scanned_rows: number;
  routines_scanned_rows: number;
}

export interface EpisodeListRequest {
  start_ts_ns?: string;
  end_ts_ns?: string;
  apps?: string[];
  actor?: string;
  min_duration_ms?: number;
  limit?: number;
  cursor?: string;
}

export interface EpisodeRow {
  key_hex: string;
  ordinal: number;
  episode_id: string;
  start_ts_ns: number;
  end_ts_ns: number;
  duration_ms: number;
  actor: string;
  app?: string;
  document?: string;
  url?: string;
  title_first?: string;
  title_last?: string;
  distinct_title_count: number;
  row_count: number;
  keystroke_count: number;
  click_count: number;
  interruption_count: number;
  interrupted_ms: number;
  started_because: Record<string, unknown> | string;
  ended_because: Record<string, unknown> | string;
}

export interface EpisodeListReadback {
  episodes: EpisodeRow[];
  scanned_rows: number;
  next_cursor?: string;
  stopped_because: string;
}

export interface EpisodeGetRequest {
  episode_id: string;
  start_ts_ns?: string;
  refs_limit?: number;
  refs_cursor?: string;
}

export interface EpisodeGetReadback {
  episode: EpisodeRow;
  episode_scanned_rows: number;
  timeline_refs: Array<Record<string, unknown>>;
  refs_scanned_rows: number;
  refs_invalid_rows: number;
  next_refs_cursor?: string;
  refs_stopped_because: string;
}

export interface RoutineListRequest {
  lifecycle?: string[];
  min_confidence?: number;
  app?: string;
  granularity?: string;
  include_unmined?: boolean;
  limit?: number;
}

export interface RoutineEntry {
  routine_id: string;
  lifecycle: string;
  label?: string;
  mined: boolean;
  state_row_exists: boolean;
  granularity?: string;
  steps: Array<Record<string, unknown>>;
  schedule_label?: string;
  confidence?: number;
  support_days?: number;
  occurrence_count?: number;
  last_mined_ts_ns?: number;
  updated_ts_ns: number;
  tainted: boolean;
  taint?: Record<string, unknown>;
}

export interface RoutineListReadback {
  total_mined: number;
  total_state_rows: number;
  matched: number;
  returned: number;
  truncated: boolean;
  entries: RoutineEntry[];
}

export interface RoutineInspectRequest {
  routine_id: string;
}

export type RoutineUpdateAction = "confirm" | "disable" | "enable" | "archive" | "rename" | "arm" | "disarm";

export interface RoutineUpdateRequest {
  routine_id: string;
  action: RoutineUpdateAction;
  label?: string;
  note?: string;
  arm_schedule?: boolean;
  arm_intent?: boolean;
  failure_threshold?: number;
}

export interface RoutineUpdateReadback {
  routine_id: string;
  action: RoutineUpdateAction;
  lifecycle_before: string;
  lifecycle_after: string;
  label_before?: string;
  label_after?: string;
  state_row_created: boolean;
  state: Record<string, unknown>;
  armed?: Record<string, unknown>;
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

function jsonHeaders(): Record<string, string> {
  return { "Content-Type": "application/json" };
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

export async function probeModel(name: string, timeoutMs = 120000): Promise<DashboardRouteReadback> {
  const response = await fetch("/dashboard/models/probe", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify({ name, timeout_ms: timeoutMs })
  });
  return readJsonOrThrow(response);
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

export async function putTemplate(request: TemplateUpsertRequest): Promise<AgentTemplateRow> {
  const response = await fetch("/dashboard/templates", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  const put = (body.put ?? {}) as { template?: AgentTemplateRow };
  if (!put.template) throw new Error("template upsert returned no template row");
  return put.template;
}

export async function deleteTemplate(templateId: string): Promise<void> {
  const response = await fetch(`/dashboard/templates/${encodeURIComponent(templateId)}`, {
    method: "DELETE",
    cache: "no-store",
    credentials: "same-origin"
  });
  await readJsonOrThrow(response);
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

export async function fetchAgentEvents(filters: AgentEventsQueryRequest): Promise<AgentEventsQueryResponse> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value === undefined || value === null || value === "") continue;
    params.set(key, String(value));
  }
  const suffix = params.toString();
  const response = await fetch(`/dashboard/agent-events/query${suffix ? `?${suffix}` : ""}`, {
    cache: "no-store",
    credentials: "same-origin"
  });
  const body = await readJsonOrThrow(response);
  return {
    ...(body as unknown as AgentEventsQueryResponse),
    rows: asArray<AgentEventQueryRow>(body.rows)
  };
}

export async function fetchAgentRecording(
  spawnId: string,
  options: { event_limit?: number; event_scan_limit?: number; max_cast_bytes?: number } = {}
): Promise<AgentRecordingResponse> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(options)) {
    if (value === undefined || value === null) continue;
    params.set(key, String(value));
  }
  const suffix = params.toString();
  const response = await fetch(`/dashboard/agent-recordings/${encodeURIComponent(spawnId)}${suffix ? `?${suffix}` : ""}`, {
    cache: "no-store",
    credentials: "same-origin"
  });
  const body = await readJsonOrThrow(response);
  const asciicast = asRecord(body.asciicast) as unknown as AsciicastReplayReadback;
  const journal = asRecord(body.journal) as unknown as AgentEventsQueryResponse;
  return {
    ...(body as unknown as AgentRecordingResponse),
    metadata: asRecord(body.metadata) as unknown as AgentRecordingMetadata,
    asciicast: {
      ...asciicast,
      header: asRecord(asciicast.header),
      events: asArray<AsciicastReplayEvent>(asciicast.events)
    },
    journal: {
      ...journal,
      rows: asArray<AgentEventQueryRow>(journal.rows)
    }
  };
}

export async function saveDashboardView(
  request: SaveDashboardViewRequest
): Promise<{ ok: boolean; row_key: string; source_of_truth: string; view: DashboardSavedView }> {
  const response = await fetch("/dashboard/saved-views", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
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
    headers: jsonHeaders()
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
    headers: jsonHeaders(),
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
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return readJsonOrThrow(response);
}

export async function spawnAgent(request: SpawnAgentRequest): Promise<SpawnAgentResponse> {
  const response = await fetch("/dashboard/spawn-agent", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as SpawnAgentResponse;
}

export async function killAgent(request: AgentKillRequest): Promise<AgentKillResponse> {
  const response = await fetch("/dashboard/agent-kill", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as AgentKillResponse;
}

export async function broadcastAgents(request: AgentBroadcastRequest): Promise<AgentBroadcastResponse> {
  const response = await fetch("/dashboard/agent-broadcast", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as AgentBroadcastResponse;
}

export async function stopFleet(request: FleetStopRequest): Promise<FleetStopResponse> {
  const response = await fetch("/dashboard/fleet-stop", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as FleetStopResponse;
}

export async function interruptAgent(request: AgentLookupRequest): Promise<DashboardControlResponse> {
  const response = await fetch("/dashboard/agent-interrupt", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as DashboardControlResponse;
}

export async function pauseAgent(request: AgentLookupRequest): Promise<DashboardControlResponse> {
  const response = await fetch("/dashboard/agent-pause", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as DashboardControlResponse;
}

export async function resumeAgent(request: AgentLookupRequest): Promise<DashboardControlResponse> {
  const response = await fetch("/dashboard/agent-resume", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as DashboardControlResponse;
}

export async function respawnAgent(request: AgentRespawnRequest): Promise<DashboardControlResponse> {
  const response = await fetch("/dashboard/agent-respawn", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as DashboardControlResponse;
}

export type ApprovalDecisionVerb = "approve" | "deny";

export interface ApprovalDecideRequest {
  approval_id: string;
  decision: ApprovalDecisionVerb;
  note?: string;
  // #1030 approve-with-edits: full-replacement tool input as a JSON object
  // string. Honored only with an approve decision on an allow.edit item.
  edited_args?: string;
  // #1030 respond: the operator's answer to a needs-input / agent_question
  // item. Honored only with an approve decision on an allow.respond item.
  response?: string;
}

export interface ApprovalDecideResponse {
  ok: boolean;
  trigger: string;
  source_of_truth: string;
  decision: Record<string, unknown>;
}

export type ContextInjectChannel = "steer" | "mailbox" | "workspace";

export interface ContextInjectRequest {
  session_id: string;
  channel: ContextInjectChannel;
  packet: string;
  kind?: string;
  workspace_key?: string;
  request_receipt?: boolean;
}

export interface ContextInjectResponse {
  ok: boolean;
  trigger: string;
  source_of_truth: string;
  channel: ContextInjectChannel;
  payload_sha256: string;
  readback: Record<string, unknown>;
}

export interface ContextPlanRequest {
  session_id: string;
  plan: unknown;
  expected_version?: number;
  notify_agent?: boolean;
}

export interface ContextPlanResponse {
  ok: boolean;
  trigger: string;
  source_of_truth: string;
  key: string;
  payload_sha256: string;
  workspace_put: Record<string, unknown>;
  notification: Record<string, unknown>;
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
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as ApprovalDecideResponse;
}

export async function injectAgentContext(
  request: ContextInjectRequest
): Promise<ContextInjectResponse> {
  const response = await fetch("/dashboard/context/inject", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as ContextInjectResponse;
}

export async function updateAgentPlan(
  request: ContextPlanRequest
): Promise<ContextPlanResponse> {
  const response = await fetch("/dashboard/context/plan", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as ContextPlanResponse;
}

export async function pauseTimeline(duration_ms?: number): Promise<TimelineControlResponse> {
  const response = await fetch("/dashboard/timeline/pause", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify({ duration_ms })
  });
  return (await readJsonOrThrow(response)) as unknown as TimelineControlResponse;
}

export async function resumeTimeline(): Promise<TimelineControlResponse> {
  const response = await fetch("/dashboard/timeline/resume", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders()
  });
  return (await readJsonOrThrow(response)) as unknown as TimelineControlResponse;
}

export async function fetchTimelineRows(request: TimelineQueryRequest): Promise<TimelineGetReadback> {
  const response = await fetch("/dashboard/timeline/get", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as TimelineGetReadback;
}

export async function searchTimelineRows(request: TimelineQueryRequest): Promise<TimelineSearchReadback> {
  const response = await fetch("/dashboard/timeline/search", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as TimelineSearchReadback;
}

export async function fetchTimelineDigest(request: TimelineDigestRequest): Promise<TimelineDigestReadback> {
  const response = await fetch("/dashboard/timeline/digest", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as TimelineDigestReadback;
}

export async function fetchEpisodes(request: EpisodeListRequest): Promise<EpisodeListReadback> {
  const response = await fetch("/dashboard/episodes/list", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as EpisodeListReadback;
}

export async function fetchEpisodeDetail(request: EpisodeGetRequest): Promise<EpisodeGetReadback> {
  const response = await fetch("/dashboard/episodes/get", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as EpisodeGetReadback;
}

export async function fetchRoutines(request: RoutineListRequest): Promise<RoutineListReadback> {
  const response = await fetch("/dashboard/routines/list", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as RoutineListReadback;
}

export async function inspectRoutine(request: RoutineInspectRequest): Promise<Record<string, unknown>> {
  const response = await fetch("/dashboard/routines/inspect", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as Record<string, unknown>;
}

export async function updateRoutine(request: RoutineUpdateRequest): Promise<RoutineUpdateReadback> {
  const response = await fetch("/dashboard/routines/update", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as RoutineUpdateReadback;
}

export async function forceReleaseLease(
  request: LeaseForceReleaseRequest
): Promise<DashboardControlResponse> {
  const response = await fetch("/dashboard/control-lease/force-release", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  return (await readJsonOrThrow(response)) as unknown as DashboardControlResponse;
}

export async function handoffLease(request: LeaseHandoffRequest): Promise<DashboardControlResponse> {
  const response = await fetch("/dashboard/control-lease/handoff", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
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
    headers: jsonHeaders(),
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
    headers: jsonHeaders(),
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
    headers: jsonHeaders()
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

const DASHBOARD_ASSET_RELOAD_PARAM = "_synapse_dashboard_asset";
const DASHBOARD_ASSET_RELOAD_ATTEMPT_PARAM = "_synapse_dashboard_asset_attempt";
const DASHBOARD_ASSET_RELOAD_SESSION_KEY = "synapse.dashboard.asset-reload";
const DASHBOARD_ASSET_RELOAD_MAX_ATTEMPTS = 2;

function dashboardLoadedJsFile(): string {
  if (typeof document === "undefined") return "";
  for (const script of Array.from(document.scripts)) {
    const src = script.getAttribute("src") || "";
    const match = src.match(/(?:^|\/)(dashboard-[^/?#]+\.js)(?:[?#].*)?$/);
    if (match?.[1]) return match[1];
  }
  return "";
}

function isDashboardJsAssetFile(value: string): boolean {
  return /^dashboard-[A-Za-z0-9_-]+\.js$/.test(value);
}

export function dashboardAssetReloadDecision(state?: DashboardState): DashboardAssetReloadDecision {
  if (!state) return { shouldReload: false, reason: "state_unavailable" };
  const expectedJsFile = rawText(asRecord(state.dashboard_assets?.data).js_file);
  if (!expectedJsFile) return { shouldReload: false, reason: "missing_server_asset_id" };
  if (!isDashboardJsAssetFile(expectedJsFile)) {
    return { shouldReload: false, reason: "invalid_server_asset_id", expectedJsFile };
  }
  const currentJsFile = dashboardLoadedJsFile();
  if (!currentJsFile) {
    return { shouldReload: false, reason: "missing_client_asset_id", expectedJsFile };
  }
  if (currentJsFile === expectedJsFile) {
    return { shouldReload: false, reason: "asset_match", expectedJsFile, currentJsFile };
  }
  return {
    shouldReload: true,
    reason: "asset_mismatch",
    expectedJsFile,
    currentJsFile,
    reloadKey: `${currentJsFile}->${expectedJsFile}`
  };
}

export function claimDashboardAssetReload(decision: DashboardAssetReloadDecision): boolean {
  if (!decision.shouldReload || !decision.reloadKey || !decision.expectedJsFile) return false;
  if (typeof window === "undefined") return false;
  const url = new URL(window.location.href);
  let attempts = parseDashboardAssetReloadAttempt(url.searchParams.get(DASHBOARD_ASSET_RELOAD_ATTEMPT_PARAM));
  try {
    const previous = window.sessionStorage.getItem(DASHBOARD_ASSET_RELOAD_SESSION_KEY);
    attempts = Math.max(attempts, dashboardAssetReloadAttempts(previous, decision.reloadKey));
  } catch {
    // URL attempts below still bound reloads if sessionStorage is unavailable.
  }
  if (attempts >= DASHBOARD_ASSET_RELOAD_MAX_ATTEMPTS) {
    const message = [
      "DASHBOARD_ASSET_RELOAD_EXHAUSTED",
      `current=${decision.currentJsFile || "unknown"}`,
      `expected=${decision.expectedJsFile}`,
      `attempts=${attempts}`
    ].join(": ");
    console.error(message);
    throw new Error(message);
  }
  try {
    window.sessionStorage.setItem(
      DASHBOARD_ASSET_RELOAD_SESSION_KEY,
      JSON.stringify({ reloadKey: decision.reloadKey, attempts: attempts + 1 })
    );
  } catch {
    // The URL attempt marker below still bounds reloads if sessionStorage is unavailable.
  }
  return true;
}

export function dashboardAssetReloadUrl(expectedJsFile: string): string {
  const url = new URL(window.location.href);
  url.searchParams.set(DASHBOARD_ASSET_RELOAD_PARAM, expectedJsFile);
  const attempts = parseDashboardAssetReloadAttempt(url.searchParams.get(DASHBOARD_ASSET_RELOAD_ATTEMPT_PARAM));
  url.searchParams.set(DASHBOARD_ASSET_RELOAD_ATTEMPT_PARAM, String(attempts + 1));
  return url.toString();
}

function parseDashboardAssetReloadAttempt(value: string | null): number {
  if (!value) return 0;
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
}

function dashboardAssetReloadAttempts(value: string | null, reloadKey: string): number {
  if (!value) return 0;
  if (value === reloadKey) return 1;
  try {
    const parsed = JSON.parse(value) as { reloadKey?: unknown; attempts?: unknown };
    if (parsed.reloadKey !== reloadKey) return 0;
    return typeof parsed.attempts === "number" && Number.isFinite(parsed.attempts) && parsed.attempts > 0
      ? parsed.attempts
      : 0;
  } catch {
    return 0;
  }
}

export function panelData<T = Record<string, unknown>>(panel?: DashboardPanel): T {
  return (panel?.data ?? {}) as T;
}

export function buildTaskRows(state?: DashboardState): AgentTaskRow[] {
  if (!state) return [];
  const data = asRecord(panelData(state.tasks));
  return asArray<AgentTaskRow>(data.tasks).filter((task) => rawText(task.task_id));
}

export function buildAgents(state?: DashboardState): AgentSummary[] {
  if (!state) return [];
  const sessionData = asRecord(state.sessions.data);
  const attachedRegistry = attachedAgentRegistry(state);
  const attachedRows = asArray<Record<string, unknown>>(attachedRegistry?.rows);
  const sessionRows = asArray<Record<string, unknown>>(sessionData.sessions);
  const unbound = asArray<Record<string, unknown>>(sessionData.unbound_agent_states);
  const transcripts = asArray<Record<string, unknown>>(asRecord(state.agent_transcripts.data).rows);
  const actions = asArray<Record<string, unknown>>(asRecord(state.command_audit.data).rows);
  const transcriptStats = buildTranscriptStats(transcripts);

  const actionCounts = new Map<string, number>();
  for (const row of actions) {
    const target = rawText(row.target_session_id);
    const actor = rawText(row.actor_session_id);
    for (const id of [target, actor]) {
      if (id) actionCounts.set(id, (actionCounts.get(id) ?? 0) + 1);
    }
  }

  const attachedAgents = attachedRegistry
    ? attachedRows
      .map((row) => {
        const process = asRecord(row.process);
        const killHandle = asRecord(row.kill_handle);
        const visibleWindow = asRecord(row.controlling_terminal_window || row.visible_window);
        const id = rawText(row.registry_id || row.spawn_id || row.session_id);
        const spawnId = rawText(row.spawn_id);
        const sessionId = rawText(row.session_id);
        const stateName = rawText(row.state || row.lifecycle);
        const reason = rawText(row.reason_code || row.not_counted_reason);
        const lastSeenMs = Number(row.last_seen_ms_ago);
        const liveProcessIds = asArray(process.live_process_ids).map(rawText).filter(Boolean);
        const processSummary = liveProcessIds.length ? `pids ${liveProcessIds.join(", ")}` : rawText(row.not_counted_reason || "no live pid");
        const windowTitle = rawText(visibleWindow.window_title);
        const source = rawText(row.source);
        const countsAsLive = Boolean(row.counts_as_live);
        const terminal = terminalStatus(stateName, reason);
        const killable = !terminal && countsAsLive && Boolean(killHandle.available);
        const stats = findTranscriptStats(transcriptStats, spawnId, sessionId, id);
        return {
          id,
          spawnId: spawnId || undefined,
          killId: rawText(killHandle.target_id) || spawnId || sessionId || id,
          killable,
          kind: rawText(row.kind || "agent"),
          lifecycle: countsAsLive ? "live" : rawText(row.lifecycle || "observed"),
          status: terminal ?? statusFromAgentState(stateName, reason),
          summary: stats?.latestSummary || [processSummary, windowTitle, source].filter(Boolean).join(" / "),
          lastSeenMs: Number.isFinite(lastSeenMs) ? lastSeenMs : undefined,
          lastAction: source,
          target: windowTitle,
          reason,
          diffStats: {
            events: 1,
            transcripts: stats?.count ?? 0,
            actions: actionCounts.get(sessionId || id) ?? 0
          },
          usage: stats?.usage,
          raw: row
        } satisfies AgentSummary;
      })
      .filter((agent) => agent.id)
    : [];

  const live = sessionRows.map((row) => {
    const agentState = asRecord(row.agent_state);
    const sessionId = rawText(row.session_id);
    const spawnId = rawText(row.spawn_id || agentState.spawn_id);
    const stateName = rawText(agentState.state || row.lifecycle);
    const lastSeenMs = Number(row.last_seen_ms_ago);
    const lastAction = rawText(row.last_action);
    const lifecycle = rawText(row.lifecycle);
    const reason = rawText(agentState.reason_code);
    const status = statusFromLiveSession(stateName, lastSeenMs, lastAction, reason);
    const stats = findTranscriptStats(transcriptStats, spawnId, sessionId);
    return {
      id: sessionId,
      spawnId: spawnId || undefined,
      killId: spawnId || sessionId,
      killable: Boolean(spawnId) && lifecycle === "live" && status !== "done" && status !== "failed",
      kind: rawText(row.agent_kind || row.client_name || "agent"),
      lifecycle,
      status,
      summary: stats?.latestSummary || lastAction || stateName || "session live",
      lastSeenMs: Number.isFinite(lastSeenMs) ? lastSeenMs : undefined,
      lastAction,
      target: row.active_target ? rawText(row.active_target) : "",
      reason,
      diffStats: {
        events: 1,
        transcripts: stats?.count ?? 0,
        actions: actionCounts.get(sessionId) ?? 0
      },
      usage: stats?.usage,
      raw: row
    } satisfies AgentSummary;
  });

  const historical = unbound.map((row) => {
    const id = rawText(row.session_id || row.spawn_id || row.anchor);
    const spawnId = rawText(row.spawn_id);
    const stateName = rawText(row.state);
    const reason = rawText(row.reason_code);
    const stats = findTranscriptStats(transcriptStats, spawnId, id);
    // Ambient agents (discovered from ~/.claude/projects, not spawned by
    // Synapse) may be live or terminal. Render their server-authoritative
    // lifecycle directly, but keep terminal history out of attention counts.
    const isAmbient = spawnId.startsWith("agent-spawn-ambient-");
    if (isAmbient) {
      const lastTool = rawText(row.last_tool_name);
      const silentMs = Number(row.silent_ms);
      return {
        id,
        spawnId: spawnId || undefined,
        killId: spawnId || id,
        killable: false,
        kind: `${rawText(row.agent_kind || "claude")} · ambient`,
        lifecycle: stateName || "ambient",
        status: statusFromAgentState(stateName, reason),
        summary: stats?.latestSummary || (lastTool ? `tool: ${lastTool}` : stateName || "observed"),
        lastSeenMs: Number.isFinite(silentMs) ? silentMs : undefined,
        lastAction: lastTool,
        target: "",
        reason,
        diffStats: {
          events: 1,
          transcripts: stats?.count ?? 0,
          actions: actionCounts.get(id) ?? 0
        },
        usage: stats?.usage,
        raw: row
      } satisfies AgentSummary;
    }
    return {
      id,
      spawnId: spawnId || undefined,
      killId: spawnId || id,
      killable: false,
      kind: rawText(row.agent_kind || "agent"),
      lifecycle: stateName || "unbound",
      status: statusFromHistorical(stateName, reason),
      summary: stats?.latestSummary || [stateName, reason].filter(Boolean).join(" / ") || "historical state",
      lastSeenMs: undefined,
      lastAction: "",
      target: "",
      reason,
      diffStats: {
        events: 1,
        transcripts: stats?.count ?? 0,
        actions: actionCounts.get(id) ?? 0
      },
      usage: stats?.usage,
      raw: row
    } satisfies AgentSummary;
  });

  return dedupeAgentSummaries([...attachedAgents, ...live, ...historical]);
}

function dedupeAgentSummaries(agents: AgentSummary[]): AgentSummary[] {
  const seen = new Set<string>();
  return agents.filter((agent) => {
    if (!agent.id) return false;
    const ids = [agent.id, agent.spawnId, agent.killId].filter((value): value is string => Boolean(value));
    if (ids.some((id) => seen.has(id))) return false;
    ids.forEach((id) => seen.add(id));
    return true;
  });
}

export async function createAgentTask(request: AgentTaskCreateRequest): Promise<AgentTaskMutationResponse> {
  const response = await fetch("/dashboard/tasks/create", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as AgentTaskMutationResponse;
}

export async function updateAgentTask(request: AgentTaskUpdateRequest): Promise<AgentTaskMutationResponse> {
  const response = await fetch("/dashboard/tasks/update", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as AgentTaskMutationResponse;
}

export async function cancelAgentTask(request: AgentTaskCancelRequest): Promise<AgentTaskCancelResponse> {
  const response = await fetch("/dashboard/tasks/cancel", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as AgentTaskCancelResponse;
}

export async function dispatchAgentTaskOnce(request: AgentTaskDispatchOnceRequest): Promise<AgentTaskDispatchOnceResponse> {
  const response = await fetch("/dashboard/tasks/dispatch-once", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: jsonHeaders(),
    body: JSON.stringify(request)
  });
  const body = await readJsonOrThrow(response);
  return body.readback as AgentTaskDispatchOnceResponse;
}

interface TranscriptAgentStats {
  count: number;
  latestLine: number;
  latestSummary: string;
  usage?: AgentUsageSummary;
  usageLine: number;
}

function buildTranscriptStats(rows: Record<string, unknown>[]): Map<string, TranscriptAgentStats> {
  const stats = new Map<string, TranscriptAgentStats>();
  rows.forEach((row, index) => {
    const id = rawText(row.spawn_id || row.session_id);
    if (!id) return;
    const line = transcriptLineNumber(row, index);
    const record = asRecord(row.record);
    const current = stats.get(id) ?? { count: 0, latestLine: -1, latestSummary: "", usageLine: -1 };
    current.count += 1;

    const summary = transcriptSummary(record);
    if (summary && line >= current.latestLine) {
      current.latestLine = line;
      current.latestSummary = summary;
    }

    const usage = agentUsageFromTranscriptRecord(record, line);
    if (usage && line >= current.usageLine) {
      current.usageLine = line;
      current.usage = usage;
    }

    stats.set(id, current);
  });
  return stats;
}

function findTranscriptStats(stats: Map<string, TranscriptAgentStats>, ...ids: Array<string | undefined>) {
  for (const id of ids) {
    if (!id) continue;
    const match = stats.get(id);
    if (match) return match;
  }
  return undefined;
}

function transcriptLineNumber(row: Record<string, unknown>, fallback: number): number {
  const line = Number(row.line_no ?? row.line ?? row.sequence ?? row.seq);
  return Number.isFinite(line) ? line : fallback;
}

function transcriptSummary(record: Record<string, unknown>): string {
  const text = rawText(record.content_summary).trim();
  if (text) return text;
  const sourceError = rawText(record.source_error || record.parse_error).trim();
  if (sourceError) return sourceError;
  const toolCalls = asArray<Record<string, unknown>>(record.tool_calls).map(asRecord);
  const toolNames = toolCalls.map((call) => rawText(call.tool_name)).filter(Boolean);
  if (toolNames.length > 0) return `tool: ${toolNames.slice(0, 3).join(", ")}`;
  return "";
}

export function agentUsageFromTranscriptRecord(record: Record<string, unknown>, sourceLine?: number): AgentUsageSummary | undefined {
  const usage = asRecord(record.usage);
  if (Object.keys(usage).length === 0) return undefined;
  const summary: AgentUsageSummary = {
    inputTokens: usageNumber(usage.input_tokens),
    outputTokens: usageNumber(usage.output_tokens),
    cacheReadInputTokens: usageNumber(usage.cache_read_input_tokens ?? usage.cached_input_tokens),
    cacheCreationInputTokens: usageNumber(usage.cache_creation_input_tokens),
    reasoningOutputTokens: usageNumber(usage.reasoning_output_tokens),
    totalCostMicroUsd: optionalUsageNumber(usage.total_cost_micro_usd),
    sourceLine
  };
  if (
    summary.inputTokens ||
    summary.outputTokens ||
    summary.cacheReadInputTokens ||
    summary.cacheCreationInputTokens ||
    summary.reasoningOutputTokens ||
    summary.totalCostMicroUsd !== undefined
  ) {
    return summary;
  }
  return undefined;
}

function usageNumber(value: unknown): number {
  const number = Number(value ?? 0);
  return Number.isFinite(number) && number > 0 ? number : 0;
}

function optionalUsageNumber(value: unknown): number | undefined {
  if (value === null || value === undefined) return undefined;
  const number = Number(value);
  return Number.isFinite(number) && number >= 0 ? number : undefined;
}

export function attachedAgentRegistry(state?: DashboardState): AttachedAgentRegistry | null {
  if (!state) return null;
  const registry = asRecord(asRecord(state.sessions.data).attached_agent_registry);
  if (!registry || Object.keys(registry).length === 0) return null;
  return registry as unknown as AttachedAgentRegistry;
}

export interface TargetDashboardRows {
  liveTargetSessions: Record<string, unknown>[];
  retainedTargetSessions: Record<string, unknown>[];
  persistedCdpOwnerRows: Record<string, unknown>[];
}

export function buildTargetDashboardRows(sessions: Record<string, unknown>[]): TargetDashboardRows {
  const liveTargetSessions: Record<string, unknown>[] = [];
  const retainedTargetSessions: Record<string, unknown>[] = [];
  const persistedCdpOwnerRows: Record<string, unknown>[] = [];

  for (const session of sessions) {
    const sessionId = rawText(session.session_id);
    const lifecycle = rawText(session.lifecycle);
    const attentionClass = rawText(session.attention_class);
    const foregroundLane = asRecord(session.foreground_lane);
    const targetStatus = rawText(foregroundLane.status);
    const activeTarget = asRecord(session.active_target);
    const persistedOwners = asArray<Record<string, unknown>>(session.persisted_cdp_target_owners);
    const hasActiveTarget = Object.keys(activeTarget).length > 0;
    const cleanupAction = sessionId ? `session_end ${sessionId}` : "";

    for (const owner of persistedOwners) {
      persistedCdpOwnerRows.push({
        ...owner,
        session_id: rawText(owner.owner_session_id) || sessionId,
        lifecycle,
        attention_class: attentionClass,
        cleanup_action: rawText(owner.cleanup_action) || cleanupAction
      });
    }

    if (!hasActiveTarget) continue;
    const row = {
      ...session,
      target_status: targetStatus,
      cleanup_action: lifecycle === "live" ? "" : cleanupAction
    };
    if (lifecycle === "live") {
      liveTargetSessions.push(row);
    } else {
      retainedTargetSessions.push(row);
    }
  }

  return { liveTargetSessions, retainedTargetSessions, persistedCdpOwnerRows };
}

export function shellJobNeedsReview(row: Record<string, unknown>): boolean {
  const job = asRecord(row.job);
  const status = rawText(job.status || row.status);
  if (!status || status === "ok") return false;
  return true;
}

export function shellJobCommandLabel(row: Record<string, unknown>): string {
  const job = asRecord(row.job);
  return rawText(job.command_line || job.command || row.command || row.job_id);
}

export function shellJobEvidence(row: Record<string, unknown>): string {
  const job = asRecord(row.job);
  const diagnostics = asRecord(job.diagnostics);
  const hints = asArray(diagnostics.actionable_hints).map(rawText).filter(Boolean);
  const parts = [
    rawText(job.error_code),
    rawText(job.error_message),
    job.exit_code === null || job.exit_code === undefined ? "" : `exit_code=${rawText(job.exit_code)}`,
    rawText(row.stderr_tail),
    rawText(row.stdout_tail),
    hints.join(", "),
    rawText(diagnostics.output_state)
  ].filter(Boolean);
  return parts.join(" · ");
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

const TERMINAL_STATE_RE = /dead|done|exited|closed/i;
const FAILURE_REASON_RE = /failed|failure|unhealthy|missing|error|interrupted|timeout|readback/i;

function terminalStatus(stateName: string, reason: string): FleetStatus | null {
  if (!TERMINAL_STATE_RE.test(stateName)) return null;
  return FAILURE_REASON_RE.test(`${stateName} ${reason}`) ? "failed" : "done";
}

function statusFromLiveSession(stateName: string, lastSeenMs: number, lastAction: string, reason: string): FleetStatus {
  const authoritative = authoritativeAgentStatus(stateName, reason);
  if (authoritative) return authoritative;
  if (Number.isFinite(lastSeenMs) && lastSeenMs > 300000) return "stuck";
  if (FAILURE_REASON_RE.test(reason)) return "stuck";
  if (/approval/i.test(`${stateName} ${lastAction} ${reason}`)) return "awaiting_approval";
  if (/wait|inbox|input/i.test(`${stateName} ${lastAction} ${reason}`)) return "needs_input";
  if (/review/i.test(`${stateName} ${lastAction} ${reason}`)) return "ready_for_review";
  if (/idle/i.test(stateName)) return "idle";
  return "working";
}

// Maps the server-authoritative #898 lifecycle state straight onto a fleet
// status. The backend already folds heartbeat-silence and process-liveness into
// this state, so the UI trusts it verbatim rather than re-deriving from idle ms.
function statusFromAgentState(stateName: string, reason = ""): FleetStatus {
  return authoritativeAgentStatus(stateName, reason) ?? "idle";
}

function authoritativeAgentStatus(stateName: string, reason = ""): FleetStatus | null {
  const terminal = terminalStatus(stateName, reason);
  if (terminal) return terminal;
  switch (stateName.toLowerCase()) {
    case "working":
    case "spawning":
      return "working";
    case "idle":
      return "idle";
    case "needs_input":
      return "needs_input";
    case "awaiting_approval":
      return "awaiting_approval";
    case "ready_for_review":
      return "ready_for_review";
    case "stuck":
      return "stuck";
    default:
      return null;
  }
}

function statusFromHistorical(stateName: string, reason: string): FleetStatus {
  const terminal = terminalStatus(stateName, reason);
  if (terminal) return terminal;
  if (/approval/i.test(`${stateName} ${reason}`)) return "awaiting_approval";
  if (/wait|inbox|input/i.test(`${stateName} ${reason}`)) return "needs_input";
  if (/review/i.test(`${stateName} ${reason}`)) return "ready_for_review";
  if (/stuck|failed|failure|unhealthy|missing|error|interrupted|timeout/i.test(`${stateName} ${reason}`)) return "stuck";
  if (/working|spawning/i.test(stateName)) return "working";
  return "idle";
}
