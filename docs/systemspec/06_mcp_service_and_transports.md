# 06 — MCP Service and Transports (`synapse-mcp`)

Source files covered:
- `crates/synapse-mcp/src/main.rs`
- `crates/synapse-mcp/src/server.rs`
- `crates/synapse-mcp/src/safety.rs`
- `crates/synapse-mcp/src/http/mod.rs`
- `crates/synapse-mcp/src/http/transport.rs`
- `crates/synapse-mcp/src/http/auth.rs`
- `crates/synapse-mcp/src/http/session.rs`
- `crates/synapse-mcp/src/http/sse.rs`

## 1. `SynapseService` (the MCP server handler)

`crates/synapse-mcp/src/server.rs` defines:

```rust
pub struct SynapseService {
    started_at: Instant,
    tool_router: ToolRouter<Self>,
    m1_state: SharedM1State,            // Arc<Mutex<M1State>>
    m2_state: SharedM2State,            // Arc<Mutex<M2State>>
    m3_state: SharedM3State,            // Arc<Mutex<M3State>>
}
```

Implements `rmcp::ServerHandler` via `#[tool_handler(router = self.tool_router)]` (`server.rs:1185`). The 30 tools are declared on the same struct under `#[tool_router(router = tool_router)]` (`server.rs:741`) — `#[tool(description = "...")]` annotations produce JSON-Schema entries for `tools/list` automatically.

### 1.1 Constructors

| Constructor | Use site | Behavior |
|---|---|---|
| `SynapseService::new()` | `Default` impl + tests | Calls `try_new`, panics on failure |
| `SynapseService::try_new() -> anyhow::Result<Self>` | tests | Builds states from env (`SharedM1State::default()`, `shared_m2_state_from_env()`, `shared_m3_state_from_env()?`) |
| `try_with_m2_shutdown_reason_and_m3_config(shutdown_cancel, shutdown_reason, connection_closed_cancel, m3_config)` | stdio mode (`main.rs::run_stdio`) | Wires shutdown tokens and uses a `SseState::with_max_subscriptions(m3_config.max_subscriptions)` |
| `try_with_m2_shutdown_reason_and_sse_state_and_m3_config(shutdown_cancel, shutdown_reason, connection_closed_cancel, sse_state, m3_config)` | HTTP mode (`http::transport::http_service`) | Same but receives an already-built `SseState` so it can be shared with the axum router for `/events` |

### 1.2 ServerInfo and instructions

`get_info()` (`server.rs:1214`) returns a `ServerInfo` with:

- name = `"synapse-mcp"`, version = `env!("CARGO_PKG_VERSION")` (= `"0.1.0"`)
- capabilities: `ServerCapabilities::builder().enable_tools().build()`
- instructions string varies by state (`instructions()` at `server.rs:470`):
  - `"Synapse M1 perception MCP server with M2 action scaffold and M3 scaffold (recording enabled)"` if `M2State::recording_enabled() && m3_scaffold_ready`
  - `"... and M3 scaffold"` if only M3 ready
  - `"... (recording enabled)"` if only recording on
  - else `"... with M2 action scaffold"`

`m3_scaffold_ready` requires `M3State::scaffold_ready() && m3_tool_stubs().len() == 15` (i.e. all 15 M3 tools registered: subscribe + subscribe_cancel + reflex_register/cancel/list/history + profile_list/activate + replay_record + audio_tail/transcribe + storage_inspect/put_probe_rows/gc_once/pressure_sample).

### 1.3 Health payload

`health_payload()` / `health_payload_with_http_sessions(Option<usize>)` (`server.rs:180`–`459`) builds the `Health` payload by walking each subsystem:

| Subsystem | Source | Status values |
|---|---|---|
| `storage` | `storage_health()` (`server.rs:204`) — locks m3_state, reads `storage_last_error`, otherwise locks the reflex runtime's `Db` for `pressure_level`/`schema_version`/`cf_sizes` | `initializing` / `ok` / `disk_pressure_l1..4` / `error` |
| `reflex` | `reflex_health()` (`server.rs:259`) — `reflex_last_error`, `reflex_disabled` → `disabled`, else uses scheduler `degraded_latency` + `last_tick_jitter_us` + `recursion_clamps_total` | `initializing` / `ok` / `degraded_latency` / `disabled` / `error` |
| `profiles` | `profile_health()` (`server.rs:319`) — checks `profile_last_error`, otherwise reads `ProfileRuntime::active_profile_id`, `list(true)`, `last_reload_at` | `initializing` / `ok` / `error` |
| `audio` | `audio_health()` (`server.rs:376`) — checks `audio_last_error`, `enable_audio=false` → `disabled`, else uses `LoopbackStatus::last_error_code`/`running` and `stt_model_loaded` | `initializing` / `ok` / `disabled` / `error` |
| `http` | `http_health(active_sessions)` (`server.rs:434`) — when `m3_state.shutdown_reason == "http"` → `ok`, else `disabled`; reports `bind_addr`, `active_sessions`, `sse_subscribers` | `ok` / `disabled` / `error` |

Top-level `ok` is `true` iff no subsystem reports `error`.

`uptime_s` is `started_at.elapsed().as_secs()` (monotonic).

`build` is `option_env!("VERGEN_GIT_SHA").unwrap_or("dev")` — `dev` unless a build SHA is injected at compile time. There is no `build.rs` in `synapse-mcp` to set it, so all current builds report `"dev"`.

### 1.4 Tool dispatch (`call_tool`)

`call_tool` (`server.rs:1187`) wraps the standard `ToolRouter::call`:

1. If the router returns `error.message == "tool not found"` (the rmcp default) and no data: re-emit as `mcp_error(TOOL_NOT_FOUND, format!("tool not found: {tool_name}"))`.
2. If error.code == `INVALID_PARAMS` and no data: re-emit as `mcp_error(TOOL_PARAMS_INVALID, error.message)`.
3. Otherwise propagate the error as-is.

All inner tool returns use `Json<T>` (the `rmcp::handler::server::wrapper::Json` envelope) so responses serialize as MCP `CallToolResult` bodies.

### 1.5 Permission gates and helpers

| Helper | Purpose |
|---|---|
| `require_m3_permissions(tool, required: &RequiredPermissions)` | Locks m3_state, asks `PermissionGrants::first_missing`. If any missing → `tracing::warn` with `code = SAFETY_PERMISSION_DENIED` and returns `authorization_error(tool, missing)` (data `{ code: "SAFETY_PERMISSION_DENIED", tool, missing_permission }`) |
| `allow_unknown_profile()` | Reads `m3_state.allow_unknown_profile` |
| `m2_action_context()` | Returns `(ActionHandle, Option<Arc<RecordingBackend>>, Option<CancellationToken>)` from `m2_state` |
| `m2_release_all_context()` | Returns `(ActionHandle, ActionEmitterSnapshotHandle)` |
| `profile_runtime()` | `m3_state.lock()?.ensure_profile_runtime()` |
| `sse_state()` | clones `m3_state.sse_state` |
| `reflex_runtime()` | Builds reflex runtime: takes the event_bus from `SseState`, the action handle from M2, calls `M3State::ensure_reflex_runtime` (which opens RocksDB on first call), then `ensure_a11y_event_bridge` to bridge UIA events into the bus |
| `ensure_act_type_foreground(recording)` | Reads `m1_state.last_observed_foreground` and `synapse_a11y::current_foreground_context()`. If the live foreground hwnd differs from the last-observed hwnd, returns `ACTION_FOREGROUND_LOST` with a structured tracing warn (`M2_ACT_TYPE_FOREGROUND_LOST`) so `act_type` never types into the wrong window |

### 1.6 Debug-only knob

`maybe_force_panic_during_act(tool)` (`server.rs:732`): in debug builds, if `SYNAPSE_MCP_FORCE_PANIC_DURING_ACT == "1"`, panics during `act_press` (used to validate the operator-hotkey + panic-hook path).

## 2. Binary entrypoint (`crates/synapse-mcp/src/main.rs`)

### 2.1 CLI shape

`Cli` (clap derive) with `mode: Mode { Stdio | Http }` plus the flags table in [03_configuration.md §2](03_configuration.md). Constructor `Cli::m3_config()` builds an `M3ServiceConfig` (`m3.rs::M3ServiceConfig::from_cli_parts`) that also reads `SYNAPSE_BEARER_TOKEN`.

### 2.2 Stdio mode

`run_stdio(telemetry_guard, m3_config)` (`main.rs:143`):

1. Creates three `CancellationToken`s:
   - `rmcp_token` — passed to `service.serve_with_ct`
   - `emitter_shutdown_token` — propagated to `M2State` so the action emitter task exits on shutdown
   - `emitter_connection_closed_token` — additionally observed by tool calls so they can refuse work after EOF
2. Builds `SynapseService::try_with_m2_shutdown_reason_and_m3_config(emitter_shutdown_token, "sigint", emitter_connection_closed_token, m3_config)`.
3. Installs the action panic hook (`synapse_action::install_panic_hook`) and the operator hotkey guard (`safety::install_operator_hotkey(service.m3_state_handle())`).
4. Builds the rmcp stdio transport (`rmcp::transport::stdio()`) and wraps stdin in `CancelOnEofRead` so a closed pipe cancels both tokens after the first EOF read.
5. `service.serve_with_ct((stdin, stdout), rmcp_token)` returns the rmcp service future. Selected against `wait_for_shutdown_signal` (Ctrl-C on POSIX, Ctrl-C or Ctrl-Break on Windows).
6. On EOF or service exit, drains the M2 emitter task by waiting on its `watch::Receiver<Option<ActionStateSnapshot>>` for up to 1 second (`wait_for_m2_emitter_done`).

`CancelOnEofRead` (`main.rs:215`) wraps any `AsyncRead`: the first read that returns `Poll::Ready(Ok(()))` with no bytes filled flips `eof_seen` and cancels both tokens, logging `MCP_STDIO_EOF_CONNECTION_CLOSED`.

### 2.3 HTTP mode

`http::serve(bind, allow_non_loopback, m3_config)` is forwarded to `http::transport::serve` (`crates/synapse-mcp/src/http/transport.rs:36`):

1. Parses `bind` as `SocketAddr`. Non-loopback IPs require `--allow-non-loopback`; otherwise emit `code = HTTP_BIND_NON_LOOPBACK_REFUSED` and exit `2`.
2. `TcpListener::bind(addr).await`.
3. Build `SseState::with_max_subscriptions(m3_config.max_subscriptions)`.
4. Build `SynapseService` via `http_service` (uses `shutdown_reason = "http"`).
5. Install operator hotkey.
6. Construct the axum `Router` (see §3).
7. `axum::serve(listener, app).with_graceful_shutdown(shutdown_cancel.cancelled_owned())`. Joined or shut down via `wait_for_shutdown_signal("http")`, after which `wait_for_server_stop` gives the server up to 2 seconds to drain before aborting.

## 3. Axum router (HTTP)

`transport.rs::router` (`transport.rs:106`) constructs:

```rust
Router::new()
  .route("/health", get(health))
  .route("/events", get(events).post(publish_event))
  .route("/events/stats", get(event_stats))
  .nest_service("/mcp", mcp_service)            // rmcp StreamableHttpService
  .layer(middleware::from_fn(session::require_mcp_session))
  .layer(middleware::from_fn_with_state(auth, auth::require_http_security))
  .with_state(state)
```

`HttpState` shared with handlers: `{ health_service: Arc<SynapseService>, session_manager: Arc<LocalSessionManager>, sse_state: SseState }`.

### 3.1 `GET /health`

Returns a JSON `Health` payload — same as the MCP `health` tool — plus `active_sessions = state.session_manager.sessions.read().await.len()` populated into `subsystems.http.active_sessions`.

### 3.2 `/mcp` (streamable HTTP MCP)

Provided by `rmcp::transport::streamable_http_server::StreamableHttpService<SynapseService, LocalSessionManager>`. Initialized via `streamable_service`:

- `StreamableHttpServerConfig::default().with_cancellation_token(shutdown_cancel.child_token())`
- `LocalSessionManager` whose `session_config` is loaded from `http::session::load_session_config()`:
  - `keep_alive = Some(Duration::from_secs(SYNAPSE_HTTP_SESSION_IDLE_TIMEOUT_SECS or 1800))`. Zero/non-integer values refuse startup.

The transport handles the rmcp wire protocol over POST/GET/DELETE on `/mcp`.

### 3.3 `GET /events` (SSE bridge)

Calls `SseState::open(headers, EventsQuery)`. See §5.

### 3.4 `POST /events` and `GET /events/stats` (manual-only)

Both routes check `SseState.inner.manual_routes_enabled` (set from `SYNAPSE_HTTP_SSE_MANUAL`). When the env var is not `1`/`true`, both routes return `404 NOT_FOUND` so the surface is silent in production.

When enabled, `publish_event` decodes a JSON `{ "events": [...] }`, publishes each via `EventBus::publish`, syncs every subscription's ring with `sync_all`, and returns `{ matched, queued, dropped, subscriptions_synced }`. `event_stats` returns per-subscription ring stats (`ring_len`, `oldest_seq`, `latest_seq`, `oldest_event_seq`, `latest_event_seq`, `dropped_total`, `lossy_pending`).

## 4. Middleware

### 4.1 `auth::require_http_security` (`http/auth.rs`)

For every HTTP request:

1. **Host / Origin validation** (`validate_origin_and_host`).
   - `Host` header must parse as a valid authority and have a host of `127.0.0.1`, `localhost`, or `::1` (case-insensitive, brackets stripped). Otherwise `403 HTTP_ORIGIN_REFUSED` with the structured warn `code = HTTP_ORIGIN_REFUSED`.
   - `Origin` header (if present) must be `http://` with a loopback host. Missing Origin is accepted only when the bind is itself loopback (so a non-loopback bind without an Origin header is refused).
2. **Bearer auth** (`authorize`).
   - Reads `Authorization` header, expects `Bearer <token>` (scheme is case-insensitive).
   - Trims surrounding whitespace; empty token → `401 HTTP_TOKEN_INVALID`.
   - Compares SHA-256 hash with the configured token in constant time via `subtle::ConstantTimeEq`.
   - Token source priority (loaded once at startup, `HttpAuth::load`):
     1. `%APPDATA%/synapse/token.txt` (UTF-8, trimmed; empty refuses startup).
     2. Otherwise `SYNAPSE_BEARER_TOKEN`. Missing both refuses startup with context-rich anyhow chain.

### 4.2 `session::require_mcp_session` (`http/session.rs`)

Enforced only for paths matching `/mcp` or `/mcp/...`. Behavior:

| Request kind | Behavior |
|---|---|
| Has non-empty `Mcp-Session-Id` header | Forwarded |
| POST without header | Reads body (capped at `MAX_MCP_REQUEST_BYTES = 1 MiB`; oversize → `413`). If the body parses as JSON with `method == "initialize"`, the request is forwarded (so a client can initialize without already having a session id); otherwise `404 HTTP_SESSION_INVALID`. If body fails to parse as JSON, it is forwarded too (rmcp will reject it). |
| GET or DELETE without header | `404 HTTP_SESSION_INVALID` |
| Other methods without header | Forwarded |

If the inner rmcp handler returns `404 NOT_FOUND`, the middleware reinterprets it as a session-invalid response (rmcp emits 404 for unknown session ids).

## 5. SSE bridge (`http/sse.rs`)

`SseState` wraps an `EventBus`, a `Mutex<BTreeMap<String, Arc<Subscription>>>` of live subscriptions, and the `manual_routes_enabled` toggle.

### 5.1 Subscription

`SseState::subscribe(filter, kinds, snapshot_first) -> Result<String, SseSubscribeError>`:

1. Calls `EventBus::subscribe(filter, kinds, snapshot_first)` → `SubscriberHandle` (with a new `SubscriptionId`).
2. Allocates a `Subscription` with:
   - the handle
   - a per-subscription ring `Mutex<VecDeque<BufferedEvent>>` initialized with capacity `SUBSCRIBER_QUEUE_CAPACITY = 4096`
   - `next_stream_seq: AtomicU64 = 1`
   - `dropped_total: AtomicU64 = 0`
   - `lossy_pending: AtomicBool = false`
3. Inserts into the subscriptions map.

`SseSubscribeError`:
- `CapReached { limit }` → `SUBSCRIPTION_CAP_REACHED`
- `FilterInvalid { detail }` → `TOOL_PARAMS_INVALID`
- `StateUnavailable` → `TOOL_INTERNAL_ERROR` (mutex poison)

### 5.2 Event ring + sync

`sync_subscription` (`sse.rs:359`):
- Drains the SubscriberHandle's bounded channel into a `Vec<Event>`.
- Adds `handle.take_dropped_since_read()` to `dropped_total` and sets `lossy_pending` if non-zero or `handle.take_lossy()`.
- Pushes events into the local ring; if the ring is full it pops front, increments `dropped_total`, and sets `lossy_pending`.
- Each event gets a fresh `stream_seq = next_stream_seq.fetch_add(1)`.

`SseState::sync_all` iterates every subscription and calls `sync_subscription`.

### 5.3 Opening an SSE stream

`SseState::open(headers, EventsQuery { subscription_id })`:

1. Parses `Last-Event-ID` header as `Option<u64>` (malformed → `400 BAD REQUEST "malformed Last-Event-ID"`).
2. Picks or creates a subscription:
   - If `subscription_id` given and exists and `last_event_id` is either absent or `<= latest_seq`, reuse it.
   - Otherwise create a new subscription with `filter = EventFilter::All`, `kinds = vec![]`, `snapshot_first = false`. Failure → `503 SERVICE_UNAVAILABLE` with the error code.
3. Computes initial frames via `frames_after(subscription, last_event_id)`:
   - Syncs the subscription.
   - `events_after(last_event_id)` extracts events with `stream_seq > last_event_id`. Detects a gap (`gap_lossy`) if the oldest buffered seq is past `last_event_id + 1`.
   - If gap-lossy OR `lossy_pending`, prepends a `SubscriptionStarted { lossy = true }` frame; the first event frame also carries `lossy = true`.
4. Wraps in `axum::response::sse::Sse` with a poll-based stream (`SSE_POLL_INTERVAL = 20 ms`).
5. Sets `Synapse-Subscription-Id` response header to the subscription's id.

### 5.4 Frame format

SSE frames use the standard event-stream encoding:

| Frame | `event:` | `id:` | `data:` |
|---|---|---|---|
| Subscription start | `subscription_started` | — | `{"subscription_id":"...","lossy":<bool>,"buffer_capacity":4096}` |
| Event | `synapse/event` | stream_seq as decimal | `{"jsonrpc":"2.0","method":"synapse/event","params":{"subscription_id":"...","stream_seq":<n>,"lossy":<bool>,"event":<Event>}}` |

The JSON-RPC envelope on the SSE event frame matches what `rmcp` clients expect from notification streams.

### 5.5 Cancel

`SseState::cancel(id)` removes the subscription from the map and also calls `EventBus::unsubscribe(id)`. Returns `SseCancelError::NotFound` if neither side knew the id (the tool `subscribe_cancel` maps this to `SUBSCRIPTION_NOT_FOUND`).

## 6. Operator panic hotkey (`safety.rs`)

`install_operator_hotkey(m3_state)` forwards to `synapse_action::install_operator_hotkey(callback)`. The callback runs on the low-level keyboard hook thread (Ctrl+Alt+Shift+P) and:

1. Builds a `disable_reflexes` report: if `m3_state.reflex_runtime` is `None`, returns `not_initialized`; otherwise locks the runtime and calls `disable_all_by_operator()`, capturing the list of disabled reflex ids or the error code on failure.
2. Builds a `fire_release_all` report: calls `RELEASE_ALL_HANDLE.get()?.fire_release_all_blocking_with_timeout(50 ms)`. Missing handle → `missing_handle` with code `ACTION_BACKEND_UNAVAILABLE`; failure → reports the action error code.
3. Emits a single `tracing::warn` with `code = SAFETY_OPERATOR_HOTKEY_FIRED` and the elapsed time + within-budget flag (`elapsed <= 50 ms`).

The disabling step persists `StoredReflexAudit` rows with `error_code = REFLEX_DISABLED_BY_OPERATOR` for every formerly-active reflex (see [04_storage_layer.md §4.2](04_storage_layer.md)).

## 7. Shutdown semantics

| Trigger | Behavior |
|---|---|
| `Ctrl-C` / `Ctrl-Break` (Windows) | `wait_for_shutdown_signal` returns; daemon logs `MCP_SHUTDOWN_GRACEFUL`, cancels the rmcp service token + emitter token + connection-closed token, waits up to 1 s for the M2 emitter to flush, then calls `std::process::exit(0)`. |
| Stdio EOF | `CancelOnEofRead` flips `eof_seen` and cancels both tokens; the rmcp service exits naturally; emitter drains; daemon returns `ExitCode::SUCCESS`. |
| HTTP shutdown | `axum::serve(...).with_graceful_shutdown(shutdown_cancel.cancelled_owned())`. If the server doesn't stop in 2 s after cancel, `wait_for_server_stop` aborts the task and logs `MCP_HTTP_SHUTDOWN_TIMEOUT`. |
| HTTP bind error | If the bind address is non-loopback without `--allow-non-loopback`, the daemon exits `ExitCode::from(2)` with `HTTP_BIND_NON_LOOPBACK_REFUSED`. |
| Panic (debug only) | `install_panic_hook` from `synapse-action` + the telemetry crate hook capture the payload to logs; if the panic occurred during an `act_*` call, the operator hotkey path is still available because both panic hooks are installed before the service starts. |

## 8. Tool list snapshot

The full list of 30 declared tools is in [13_mcp_tool_reference.md](13_mcp_tool_reference.md). They are: `health`, `observe`, `find`, `read_text`, `set_capture_target`, `set_perception_mode`, `act_click`, `act_type`, `act_press`, `act_aim`, `act_drag`, `act_scroll`, `act_pad`, `act_clipboard`, `release_all`, `subscribe`, `subscribe_cancel`, `reflex_register`, `reflex_cancel`, `reflex_list`, `reflex_history`, `profile_list`, `profile_activate`, `replay_record`, `audio_tail`, `audio_transcribe`, `storage_inspect`, `storage_put_probe_rows`, `storage_gc_once`, `storage_pressure_sample` — note the M3 set lives in `m3_tool_stubs()` (length-asserted at 15 in `instructions()`).
