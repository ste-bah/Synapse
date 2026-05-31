# 11 — Profiles, Hardware HID, Telemetry, Test Utilities

Source files covered:
- `crates/synapse-profiles/src/lib.rs`
- `crates/synapse-profiles/src/error.rs`
- `crates/synapse-profiles/src/package/mod.rs`
- `crates/synapse-profiles/src/package/digest.rs`
- `crates/synapse-profiles/src/package/types.rs`
- `crates/synapse-profiles/src/package/validation.rs`
- `crates/synapse-profiles/src/parser.rs`
- `crates/synapse-profiles/src/resolver.rs`
- `crates/synapse-profiles/src/toml_format.rs`
- `crates/synapse-profiles/src/watcher.rs`
- `crates/synapse-hid-host/src/lib.rs`
- `crates/synapse-telemetry/src/lib.rs`
- `crates/synapse-telemetry/src/metrics.rs`
- `crates/synapse-test-utils/src/lib.rs`
- `crates/synapse-test-utils/src/fixtures.rs`
- `crates/synapse-test-utils/src/stdio_mcp_client.rs`

## 1. `synapse-profiles` — TOML profile loader + package manifests + live reload

### 1.1 Public surface

| Symbol | Source |
|---|---|
| `ProfileError`, `ProfileLoadError` | `error.rs` |
| `ProfilePackageManifest`, `PackageAuthor`, `PackageSource`, `PackageTarget`, `PackageAssumptions`, `PackageInput`, `PackagePermissions`, `PackageExecutionPermissions`, `PackageContributionPermissions`, `PackageHashes`, `PackageFiles` | `package/types.rs` |
| `PROFILE_PACKAGE_KIND`, `PROFILE_PACKAGE_SCHEMA_VERSION`, `parse_package_manifest_file`, `parse_package_manifest_bytes`, `parse_package_manifest_bytes_with_digest`, `package_manifest_digest` | `package/mod.rs` / `package/digest.rs` |
| `LoadedProfile`, `ProfileDefaults`, `ScreenBounds`, `bundled_profiles_dir`, `parse_profile_bytes`, `parse_profile_file`, `parse_profile_file_with_bounds` | `parser.rs` |
| `ForegroundWindow`, `ProfileMatchResolution`, `resolve_active_profile` | `resolver.rs` |
| `ProfileRuntime`, `ProfileStatus` | `watcher.rs` |

`toml_format.rs` is private (holds `RawProfile`, the TOML-shaped intermediate).
`package/validation.rs` is private and enforces fail-closed package manifest
rules for schema version, kind, id shape, semver, RFC3339 timestamps,
provenance, compatibility targets, local assumptions, input backends,
permissions, SPDX allow-list, SHA-256 digests, file paths, and manifest digest
mismatch.

### 1.2 `ProfileDefaults` and `ScreenBounds`

```rust
pub struct ProfileDefaults {
    pub mouse_curve_default: String,        // "natural" (OQ-004 default)
    pub keyboard_dynamics_default: String,  // "natural"
}

pub struct ScreenBounds {
    pub width: i32,   // default 3840
    pub height: i32,  // default 2160
}
```

Parser constants (`parser.rs`):

| Constant | Value | Used for |
|---|---|---|
| `DEFAULT_CAPTURE_INTERVAL_MS` | `50` | `ProfileCapture.min_update_interval_ms` |
| `DEFAULT_CONFIDENCE_THRESHOLD` | `0.5` | `ProfileDetection.confidence_threshold` |
| `DEFAULT_MAX_DETECTIONS` | `32` | `ProfileDetection.max_detections` |
| `DEFAULT_SCREEN_WIDTH` | `3840` | HUD `FractionOfWindow` baseline |
| `DEFAULT_SCREEN_HEIGHT` | `2160` | HUD `FractionOfWindow` baseline |

### 1.3 `LoadedProfile`

```rust
pub struct LoadedProfile {
    pub profile: Profile,            // synapse_core::Profile
    pub schema_version: u32,
    pub defaults: ProfileDefaults,
    pub source_path: PathBuf,
    pub modified: SystemTime,
}
```

### 1.4 Parsing

`parse_profile_file(path: impl AsRef<Path>) -> Result<LoadedProfile, ProfileError>` (and `_with_bounds`):

1. `fs::read(path)` → `Vec<u8>`.
2. `parse_profile_bytes(bytes, bounds)`:
   - `toml::from_slice::<RawProfile>(bytes)` → `ProfileError::Parse { path, message }` on failure.
   - Validate schema version against the parser's supported version; future versions → `PROFILE_VERSION_INCOMPATIBLE`.
   - Convert `RawProfile` into a `synapse_core::Profile`:
     - Keymap entries are parsed and validated; failure → `PROFILE_KEYMAP_INVALID`.
     - HUD regions of kind `FractionOfWindow` are projected onto the supplied `ScreenBounds`; invalid fractions (`< 0` or `> 1`) → `PROFILE_HUD_REGION_INVALID`.
     - HUD regions of kind `Absolute` must lie inside `ScreenBounds`.
   - Capture/Detection/OCR defaults filled in from the constants above where the TOML omits them.
3. Return `LoadedProfile` with `modified` set from the file `mtime`.

`bundled_profiles_dir()` returns a `PathBuf` resolved at runtime — looking adjacent to the binary first, then under `%LOCALAPPDATA%/synapse/profiles`, then under the source tree. This is the default consumed by `M3State::ensure_profile_runtime` when `--profile-dir` is not set.

### 1.5 `ProfileRuntime` (watcher)

`ProfileRuntime::spawn(profile_dir)` / `spawn_with_screen_bounds(profile_dir, bounds)`:

1. `fs::create_dir_all(profile_dir)` (creates the directory if absent — `ProfileError::Io` on failure).
2. Initial `refresh_state` walks the directory non-recursively, parses every `.toml` file, and seeds the state map keyed by `Profile.id`. Parse errors are collected as `Vec<ProfileLoadError>` and held in `state.last_errors` instead of aborting startup — the daemon keeps running with whatever profiles parsed.
3. Spawn a `notify::RecommendedWatcher` on the directory. Events are debounced with `WATCH_DEBOUNCE = 200 ms` (`watcher.rs:22`): the worker thread coalesces events arriving within 200 ms and then re-runs `refresh_state`.
4. Return the `ProfileRuntime` (drops kill the watcher thread).

`ProfileStatus`:

```rust
pub struct ProfileStatus {
    pub id: ProfileId,
    pub label: String,
    pub use_scope: ProfileUseScope,
    pub mode: PerceptionMode,
    pub detection_model_id: Option<String>,
    pub detection_classes: Vec<String>,
    pub hud_fields: Vec<String>,
    pub keymap_actions: Vec<String>,
    pub backends: ProfileBackends,
    pub event_extensions: Vec<ProfileEventExtensionStatus>,
    pub active: bool,
    pub schema_version: u32,
    pub matches: Vec<ProfileMatch>,
    pub metadata: BTreeMap<String, String>,
    pub source_path: PathBuf,
}
```

Public methods on `ProfileRuntime`:

| Method | Behavior |
|---|---|
| `profile_dir() -> &Path` | borrow |
| `list(include_inactive: bool) -> Result<Vec<ProfileStatus>>` | returns currently parsed profiles |
| `active_profile_id() -> Result<Option<ProfileId>>` | reads the cached active id |
| `profile(id) -> Result<Option<Profile>>` | look up a parsed profile by id |
| `activate(id) -> Result<(), ProfileError>` | stamps the active id on the state (no FS writes) |
| `resolve_foreground(foreground) -> Result<Option<ProfileMatchResolution>>` | resolves the best matching profile without activating it |
| `activate_for_foreground(foreground) -> Result<Option<ProfileMatchResolution>>` | resolves and activates the best matching foreground profile |
| `last_reload_at() -> Result<Option<String>>` | RFC3339 timestamp of the last successful refresh |

### 1.6 `resolve_active_profile`

`resolve_active_profile(loaded: &[LoadedProfile], foreground: &ForegroundWindow) -> ProfileMatchResolution`:

1. For each loaded profile, iterate `Profile.matches: Vec<ProfileMatch>`.
2. A profile matches if **all** of its non-`None` match fields agree with the `ForegroundWindow`:
   - `exe` — exact match on `foreground.process_name`.
   - `title_regex` — compiled regex matches `foreground.window_title`.
   - `steam_appid` — equal to `foreground.steam_appid`.
   - `window_class` — exact match on `foreground.window_class`.
   - `process_args` — each entry must be present in `foreground.process_args`.
3. A matching entry is ranked by its strongest matched field: `exe`, then `title_regex`, then `steam_appid`, then `window_class`.
4. Same-rank ties are broken by newer profile mtime, then source path, profile id, and loaded index.
5. Returns `ProfileMatchResolution { profile_id, rank_name }`.

`ForegroundWindow` is the input contract:

```rust
pub struct ForegroundWindow {
    pub exe: Option<String>,
    pub title: Option<String>,
    pub window_class: Option<String>,
    pub steam_appid: Option<u32>,
}
```

`observe` resolves the observed foreground through this runtime and populates
`Observation.foreground.profile_id` when a profile matches. This read-only
match does not activate the profile or overwrite a manual `profile_activate`.

### 1.7 Errors

`ProfileError::code()` mapping:

| Variant | Code |
|---|---|
| `Io { .. }`, `Parse { .. }`, `Watch { .. }`, `StatePoisoned` | `PROFILE_PARSE_ERROR` |
| `VersionIncompatible { .. }` | `PROFILE_VERSION_INCOMPATIBLE` |
| `KeymapInvalid { .. }` | `PROFILE_KEYMAP_INVALID` |
| `HudRegionInvalid { .. }` | `PROFILE_HUD_REGION_INVALID` |
| `NotFound { .. }` | `PROFILE_NOT_FOUND` |

`ProfileLoadError::from_error(&ProfileError) -> Self` copies the path + code + message so the daemon can keep a vector of per-file load errors after a refresh.

### 1.8 MCP wrappers

`crates/synapse-mcp/src/m3/profile.rs` wraps the runtime in two tools:

| Tool | Behavior |
|---|---|
| `profile_list { include_inactive: bool default true }` | calls `runtime.list(include_inactive)` and `runtime.active_profile_id()`; permission: `READ_PROFILE` |
| `profile_activate { profile_id }` | look up the profile; if `use_scope = Unknown` and `--restrict-unknown-profile` IS set, return `SAFETY_PROFILE_ACTION_DENIED` (default is permissive — unknown scopes are allowed); activate the profile (or return `changed = false` if already active); then apply `Profile.backends` to M2's `BackendResolutionPolicy`; permission: `WRITE_PROFILE_ACTIVE` |

Activating a profile updates the action emitter's shared backend-resolution
policy. `[backends] default_backend = "hardware"` is accepted as a TOML alias
for `default = "hardware"` and causes `Backend::Auto` actions to resolve to
hardware for that profile unless a class-specific default overrides it. The
separate source-of-truth readback is `health.subsystems.action.backend_resolution`.

## 2. `synapse-hid-host`

```text
crates/synapse-hid-host/src/
  discover.rs, error.rs, handshake.rs, lib.rs, pipeline.rs,
  protocol.rs, reconnect.rs, telemetry.rs, transport.rs
```

Direct dependencies (`Cargo.toml`): `crc16`, `serde`, `serialport`, `synapse-core`, `thiserror`, `tokio`, `tracing`. The crate implements the host-side serial gateway used by `HardwareBackend`: port discovery, `HidGateway::connect`, IDENTIFY parsing/version validation, CRC16 frames, pipelined send, reconnect state, and structured HID errors.

The live driver talks to the RP2040 firmware over USB CDC at 1 Mbaud, with CRC16 frames and a firmware version handshake. `HidGateway::get_telemetry` and `HidReconnectGateway::get_telemetry` issue `GET_TELEMETRY` and parse the 28-byte base `TELEMETRY_RESP` payload into `HidTelemetrySnapshot { uptime_ms, frames_received, frames_dropped, link_errors, commands_executed, watchdog_fires, crc_errors }`. Current firmware extends that payload to 44 bytes with optional device-clock timing fields: `timed_commands`, `previous_command_delta_us`, `last_command_delta_us`, and `last_timed_command_uptime_us`. Host parsing accepts the old 28-byte payload for diagnostic compatibility, but hardware timing benchmarks require the 44-byte timing extension and fail closed when it is absent. Error codes surfaced by the driver include `HID_PORT_NOT_FOUND`, `HID_PORT_OPEN_FAILED`, `HID_PROTOCOL_HANDSHAKE_FAILED`, `HID_FIRMWARE_VERSION_MISMATCH`, `HID_COMMAND_REJECTED`, and `HID_LINK_TIMEOUT`.

## 3. `synapse-telemetry`

### 3.1 `TelemetryConfig`

```rust
pub struct TelemetryConfig {
    pub log_dir: Option<PathBuf>,                // defaults to default_log_dir()
    pub file_level: LevelFilter,                 // INFO
    pub console_level: LevelFilter,              // INFO
    pub max_dir_bytes: u64,                      // 500 MiB
    pub keep_days: u32,                          // 7
    pub gc_interval: Option<Duration>,           // Some(6 h), Some(ZERO) disables
}
```

Constants:

| Constant | Value |
|---|---|
| `DEFAULT_MAX_DIR_BYTES` | `500 * 1024 * 1024` |
| `DEFAULT_KEEP_DAYS` | `7` |
| `DEFAULT_GC_INTERVAL` | `6 hours` |
| `GC_INTERVAL_ENV` | `"SYNAPSE_LOG_GC_INTERVAL_S"` |

### 3.2 `init_tracing(cfg)` algorithm

1. Resolve `log_dir`. `default_log_dir()`:
   - Windows: `%LOCALAPPDATA%/synapse/logs`
   - else: `$XDG_STATE_HOME/synapse/logs` or `$HOME/.local/state/synapse/logs` or `.synapse-state/synapse/logs`
2. `prepare_log_dir(log_dir)`: `fs::create_dir_all`, then write+delete a `.synapse-write-probe` file. Failure → `TelemetryError::LogDirNotWritable`.
3. Run an immediate `run_log_gc` pass (see §3.4).
4. Build `tracing_appender::rolling::daily(log_dir, "synapse.log")` → non-blocking writer + `WorkerGuard`.
5. Layer composition:
   - File layer: JSON, includes target, file, line number, thread id/name, current span + span list. Filtered to `cfg.file_level`.
   - Console layer: stderr writer, no ANSI, filtered by `EnvFilter::builder().with_default_directive(cfg.console_level.into()).from_env_lossy()` so the operator's `RUST_LOG` overrides remain effective.
6. `Registry::default().with(file_layer).with(console_layer).try_init()` (returns `SubscriberInit` if another global subscriber is installed).
7. `install_panic_hook()` (idempotent via `Once`): wraps any prior hook and emits a `tracing::error!(code = "TELEMETRY_PANIC_HOOK_FIRED", payload, location, ...)` before delegating.
8. `metrics::register_m3_metrics()` (see §3.5).
9. Spawn the GC worker (§3.4) and store in the `TelemetryGuard`.

Returned `TelemetryGuard` ties: `_file_guard: WorkerGuard` (flushes on drop) and `_gc_worker: Option<GcWorker>` (shuts down on drop).

### 3.3 `effective_gc_interval`

`SYNAPSE_LOG_GC_INTERVAL_S` env var overrides the configured `gc_interval` at startup:

- Missing or non-numeric → use the configured value.
- `0` or `Some(Duration::ZERO)` → disable GC.
- Otherwise interpret as seconds.

### 3.4 `run_log_gc(log_dir, keep_days, max_dir_bytes)`

1. Walk `log_dir` (non-recursive). For each file:
   - If older than `keep_days * 86_400 s`, delete and continue.
   - Else collect `(path, modified, size)` into a vector.
2. Sum sizes. If `total <= max_dir_bytes`, return.
3. Sort entries oldest-first by `modified`. Delete oldest until `total <= max_dir_bytes`.

The background `GcWorker` thread re-runs this on `recv_timeout(interval)`; channel disconnect (parent guard dropped) breaks the loop cleanly.

### 3.5 Metrics registry (`metrics.rs`)

`M3_METRICS: &[MetricSpec; 19]` declares all M3-era metrics with bounded label cardinality. `register_m3_metrics()` calls `describe_metric` per spec and emits one `tracing::info!(code = "M3_METRIC_REGISTERED", ...)` per metric. Recorded `Once` so repeat calls are no-ops.

| Metric name | Kind | Unit | Labels | Cardinality cap |
|---|---|---|---|---|
| `events_dropped_for_subscriber` | counter | Count | `subscription_id` | 64 |
| `events_published_total` | counter | Count | `source`, `kind` | 832 |
| `reflex_fires_total` | counter | Count | `kind`, `reflex_id` | 64 |
| `reflex_tick_jitter_us` | histogram | Microseconds | — | 1 |
| `reflex_recursion_clamps_total` | counter | Count | — | 1 |
| `reflex_starved_total` | counter | Count | `reflex_id` | 32 |
| `cache_evictions_total` | counter | Count | `cf`, `reason` | 64 |
| `storage_disk_pressure_level` | gauge | Count | — | 1 |
| `storage_cf_bytes` | gauge | Bytes | `cf` | 16 |
| `storage_write_batch_flushes_total` | counter | Count | `trigger` | 8 |
| `profiles_active` | gauge | Count | `profile_id` | 128 |
| `profile_reloads_total` | counter | Count | `profile_id`, `outcome` | 256 |
| `audio_loopback_underruns_total` | counter | Count | — | 1 |
| `audio_stt_inferences_total` | counter | Count | `outcome` | 8 |
| `audio_stt_latency_ms` | histogram | Milliseconds | — | 1 |
| `http_requests_total` | counter | Count | `path`, `status` | 64 |
| `http_active_sessions` | gauge | Count | — | 1 |
| `sse_active_subscribers` | gauge | Count | — | 1 |
| `sse_buffer_overflows_total` | counter | Count | — | 1 |

Cardinality limit: `CARDINALITY_LIMIT = 1000`. Tests in the same file assert every spec stays under the limit.

### 3.6 Errors

`TelemetryError` variants and codes:

| Variant | Code |
|---|---|
| `LogDirNotWritable(PathBuf)` | `TELEMETRY_LOG_DIR_NOT_WRITABLE` |
| `SubscriberInit(String)` | `TELEMETRY_SUBSCRIBER_INIT_FAILED` |
| `Gc(String)` | `TELEMETRY_GC_FAILED` |

(These error names are crate-private constants — they are not in `synapse_core::error_codes` because telemetry initialization happens before that module is reachable.)

## 4. `synapse-test-utils`

### 4.1 `StdioMcpClient`

`crates/synapse-test-utils/src/stdio_mcp_client.rs`:

- `StdioMcpClient::launch_and_init_with_env(env: BTreeMap<&str, &str>) -> Result<Self>`: builds a `std::process::Command` for the workspace-built `synapse-mcp` binary, sets the env, redirects stdin/stdout to pipes, drives the JSON-RPC `initialize` + `notifications/initialized` sequence, and returns a ready client.
- `call_tool(name, params) -> Result<serde_json::Value>`: sends one `tools/call` request and parses the response.
- `tools_list() -> Result<serde_json::Value>`: lists available tools.
- On `Drop`, terminates the child process (used by `drop_kills_child.rs` integration test).

### 4.2 Fixtures

`crates/synapse-test-utils/src/fixtures.rs` includes:

- `launch_notepad()` — spawns Notepad and returns a handle that kills the process on drop.
- `wait_for_window_title_regex(regex, timeout)` — polls UIA top-level windows until a match.
- `notepad_process_ids()` — enumerates current Notepad PIDs.

These are gated behind `cfg(windows)` and the Notepad fixture is the basis for the `m2_notepad_type_save.rs` end-to-end test in `synapse-mcp`.

## 5. Cross-references

- Permission gates that consult profile use-scope: [03_configuration.md §4.4](03_configuration.md), [06_mcp_service_and_transports.md §1.5](06_mcp_service_and_transports.md).
- Profile schema types: [05_core_types_and_errors.md §5.5](05_core_types_and_errors.md).
- Metric usage: [07_reflex_runtime.md §10](07_reflex_runtime.md), [10_audio_and_models.md §4](10_audio_and_models.md), [04_storage_layer.md §7](04_storage_layer.md).

## 6. What is NOT covered

- **Physical `synapse-hid-host` runtime FSV.** Source inspection covers the host driver shape; issue closure still requires real Pico/COM-device source-of-truth evidence on the configured host.
- **OTLP export.** `opentelemetry` and `opentelemetry-otlp` are in workspace deps but not wired in `synapse-telemetry::init_tracing` — the file/console layers are the only sinks.
- **Prometheus exporter binding.** `metrics-exporter-prometheus` is referenced in workspace deps but not bound to an HTTP port by `synapse-telemetry`; the `register_m3_metrics` path only describes the metrics so the `metrics` crate global recorder can hold them.
- **Profile activation persistence.** Activating a profile updates in-memory state only; nothing is persisted to `CF_PROFILES` in this build (PRD §7 reserves that CF for future use).
