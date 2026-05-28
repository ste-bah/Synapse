# 03 — Configuration

Source files covered:
- `crates/synapse-mcp/src/main.rs`
- `crates/synapse-mcp/src/m1.rs`
- `crates/synapse-mcp/src/m2.rs`
- `crates/synapse-mcp/src/m3.rs`
- `crates/synapse-mcp/src/m3/permissions.rs`
- `crates/synapse-mcp/src/http/auth.rs`
- `crates/synapse-mcp/src/http/session.rs`
- `crates/synapse-mcp/src/http/sse.rs`
- `crates/synapse-telemetry/src/lib.rs`
- `crates/synapse-storage/src/lib.rs`
- `crates/synapse-storage/src/pressure.rs`
- `crates/synapse-storage/src/gc.rs`
- `crates/synapse-reflex/src/scheduler.rs`
- `crates/synapse-reflex/src/bus.rs`
- `crates/synapse-audio/src/lib.rs`
- `crates/synapse-action/src/handle.rs`
- `crates/synapse-action/src/rate_limit.rs`
- `crates/synapse-capture/src/lib.rs`

## 1. Configuration model

Synapse has **no config file**. All knobs are exposed either as CLI flags on the `synapse-mcp` binary or as environment variables that the binary reads at startup. Every flag has a matching env var (see `crates/synapse-mcp/src/main.rs::Cli`). Precedence is the clap default: CLI flag > env var > built-in default (clap's `env = "..."` attribute reads the env only when the flag is not given).

A small number of additional env vars are read directly by helper crates (telemetry, storage, capture, action, perception). Those are listed in §3.

## 2. `synapse-mcp` CLI flags

All flags are defined in `crates/synapse-mcp/src/main.rs::Cli` (clap derive).

| Flag | Env var | Type | Default | Valid values / range | Description |
|---|---|---|---|---|---|
| `--mode` | `SYNAPSE_MODE` | `Mode` enum | `stdio` | `stdio`, `http` | Transport mode |
| `--bind` | `SYNAPSE_BIND` | `String` | `127.0.0.1:7700` | `IP:PORT` parsable by `SocketAddr` | HTTP bind address. Non-loopback rejected unless `--allow-non-loopback`. (`crates/synapse-mcp/src/http/transport.rs::serve`) |
| `--allow-non-loopback` | `SYNAPSE_ALLOW_NON_LOOPBACK` | `bool` | `false` | flag | Permits binding to a non-loopback IP. When set, missing `Origin` headers on non-loopback HTTP requests are rejected. (`crates/synapse-mcp/src/http/auth.rs::validate_origin`) |
| `--db` | `SYNAPSE_DB` | `Option<PathBuf>` | `%LOCALAPPDATA%/synapse/db` (Windows) or `std::env::temp_dir()/synapse/db` | absolute path | RocksDB database directory. Lazily opened on first reflex tool call. (`crates/synapse-mcp/src/m3.rs::default_db_path`) |
| `--profile-dir` | `SYNAPSE_PROFILE_DIR` | `Option<PathBuf>` | result of `synapse_profiles::bundled_profiles_dir()` | absolute path | Watched profile directory. Created on first profile tool call. |
| `--log-level` | `SYNAPSE_LOG_LEVEL` | `String` (parsed `LevelFilter`) | `info` | `trace`/`debug`/`info`/`warn`/`error`/`off` | Sets both file and console layer filters (`crates/synapse-mcp/src/main.rs::configure_telemetry`) |
| `--reflex-disabled` | `SYNAPSE_REFLEX_DISABLED` | `bool` | `false` | `0`, `1`, `true`, `false` (case-insensitive); other values reject startup (`parse_bool_env` in `m3.rs`) | Disables the reflex runtime; reflex tool calls return `REFLEX_DISABLED_BY_OPERATOR`. |
| `--enable-audio` | `SYNAPSE_ENABLE_AUDIO` | `bool` | `false` | same as above | Required to grant `READ_AUDIO` and to spawn the WASAPI loopback. `audio_tail` / `audio_transcribe` require this. |
| `--allow-unknown-profile` | `SYNAPSE_ALLOW_UNKNOWN_PROFILE` | `bool` | `false` | same as above | Permits activating profiles whose `use_scope = unknown`. |
| `--allowed-permissions` | `SYNAPSE_MCP_ALLOWED_PERMISSIONS` | `Option<String>` | derived default set (see §4.4) | comma/semicolon/whitespace-separated permission names (`READ_EVENTS`, `WRITE_REFLEX`, `READ_REFLEX`, `READ_PROFILE`, `WRITE_PROFILE_ACTIVE`, `WRITE_REPLAY`, `READ_AUDIO`, `INPUT_KEYBOARD`, `INPUT_MOUSE`, `INPUT_PAD`, `INPUT_HARDWARE_HID`; aliases `KEYBOARD`/`MOUSE`/`PAD`/`HARDWARE_HID`; sentinel values `NONE` and `DENY_ALL` produce an empty set) | M3 permission grant list. Invalid permission names refuse startup. |
| `--reflex-force-degraded` | `SYNAPSE_REFLEX_FORCE_DEGRADED` | `bool` | `false` | same as bool flags above | Forces the reflex scheduler into degraded-latency mode (test-only knob). |
| `--storage-pressure-free-bytes-sample` | `SYNAPSE_STORAGE_PRESSURE_FREE_BYTES_SAMPLE` | `Option<u64>` | `None` | unsigned integer | If set, applies one synthetic free-byte sample at startup to validate disk-pressure responder paths (`Db::run_pressure_check_with_free_bytes_sample`). |
| `--max-subscriptions` | `SYNAPSE_MAX_SUBSCRIPTIONS` | `NonZeroUsize` | `synapse_reflex::DEFAULT_MAX_SUBSCRIPTIONS_NONZERO` | `>=1` | SSE event subscription cap on the bus. |
| `--hardware-hid` | `SYNAPSE_HARDWARE_HID` | `Option<String>` | `None` | `auto` or a serial port name such as `COM7` | Enables the hardware HID backend. First use requires the exact console phrase `I AUTHORIZE HARDWARE INPUT`; refusal exits 2 with `SAFETY_PROFILE_ACTION_DENIED reason=hardware_consent_refused` before the backend starts. Accepted consent writes `%APPDATA%/synapse/agreement.json`; `auto` enumerates matching Synapse Pico serial ports and proves identity; a port value opens that port directly. Missing/no-match fails startup with `HID_PORT_NOT_FOUND`; omission leaves `Backend::Hardware` fail-closed through `ACTION_BACKEND_UNAVAILABLE`. |
| `--reset-hardware-consent` | — | `bool` | `false` | flag | Deletes the existing hardware HID agreement file, then requires the exact hardware consent phrase again before startup continues. |

CLI examples (`README.md`):

```bash
synapse-mcp --mode stdio
synapse-mcp --mode http --bind 127.0.0.1:7700
synapse-mcp --help
```

## 3. Additional environment variables (read directly by libraries)

| Env var | Read by | Type | Default | Effect |
|---|---|---|---|---|
| `SYNAPSE_BEARER_TOKEN` | `crates/synapse-mcp/src/http/auth.rs::load_env_token` | `String` | unset | HTTP bearer token used when `%APPDATA%/synapse/token.txt` is absent. Empty value rejected. |
| `APPDATA` | `crates/synapse-mcp/src/http/auth.rs::token_file_path` | `OsString` | unset | If set, the token file at `%APPDATA%/synapse/token.txt` is consulted before env. |
| `LOCALAPPDATA` | `crates/synapse-mcp/src/m3.rs::default_db_path`, `crates/synapse-mcp/src/m3/permissions.rs::replay_root`, `crates/synapse-telemetry/src/lib.rs::default_log_dir` | `OsString` | unset → falls back to `temp_dir()` / `.` | Base for `db`, `replays`, `logs` directories on Windows. |
| `SYNAPSE_LOG_DIR` | `crates/synapse-mcp/src/main.rs::configure_telemetry` | `OsString` | unset | Overrides the log directory (otherwise `default_log_dir()` is used). |
| `SYNAPSE_LOG_GC_INTERVAL_S` | `crates/synapse-telemetry/src/lib.rs::effective_gc_interval` | `u64` seconds | unset → 6 hours; `0` disables | Overrides the periodic log-GC interval. |
| `SYNAPSE_HTTP_SESSION_IDLE_TIMEOUT_SECS` | `crates/synapse-mcp/src/http/session.rs::session_idle_timeout_secs` | `u64` seconds (>0) | `1800` (30 min) | Mcp session idle timeout. Zero or non-integer values refuse startup. |
| `SYNAPSE_HTTP_SSE_MANUAL` | `crates/synapse-mcp/src/http/sse.rs::manual_routes_enabled` | `bool` (`1`/`true`) | unset → `false` | Enables the `POST /events` and `GET /events/stats` debug routes. |
| `SYNAPSE_AUDIO_LOOPBACK` | `crates/synapse-mcp/src/m3.rs::audio_loopback_enabled` | `bool` (`0`/`1`/`true`/`false`) | unset → `true` | When false, the audio runtime spawns without starting the WASAPI loopback. |
| `SYNAPSE_CAPTURE_FORCE_DXGI` | `crates/synapse-capture/src/lib.rs::capture_backend_from_env` (via `CaptureBackendPreference::from_force_dxgi_value`) | `bool`/preference token | unset → `Auto` | Forces the DXGI duplication backend over the Windows.Graphics.Capture backend. |
| `SYNAPSE_MCP_SYNTHETIC_FIXTURE` | `crates/synapse-mcp/src/m1.rs::M1State::from_env` | `String` | unset | If equal to `notepad` (case-insensitive), the M1 layer feeds a synthetic Notepad observation source instead of the live OS. |
| `SYNAPSE_MCP_FORCE_NO_PERCEPTION` | `crates/synapse-mcp/src/m1.rs::M1State::from_env` | `bool` (`1`/`true`) | unset | Force every `observe` call to return `OBSERVE_NO_PERCEPTION_AVAILABLE`. Test knob. |
| `SYNAPSE_MCP_FORCE_OBSERVE_INTERNAL` | `crates/synapse-mcp/src/m1.rs::M1State::from_env` | `bool` (`1`/`true`) | unset | Force every `observe` call to return `OBSERVE_INTERNAL`. Test knob. |
| `SYNAPSE_MCP_FORCE_PANIC_DURING_ACT` | `crates/synapse-mcp/src/server.rs::maybe_force_panic_during_act` (debug builds only) | `String` | unset | When equal to `1` in a debug build, `act_press` panics inside `block_in_place`. Used to validate the operator-hotkey + panic-hook path. |
| `SYNAPSE_MCP_RECORDING_BACKEND` | `crates/synapse-mcp/src/m2.rs::M2State::from_env` | `bool` (`1`/`true`/`TRUE`) | unset | Routes M2 emits to a `RecordingBackend` instead of the live Windows backends. Used for integration tests. |

Additional crate-private env vars used only in `#[cfg(test)]` or doc-hidden API paths (e.g., `synapse-storage` test toggles) are not part of the runtime contract and are intentionally omitted here.

## 4. Validation rules

### 4.1 HTTP transport
- `--bind` must parse as a `SocketAddr`. If the resulting IP is not a loopback address, `--allow-non-loopback` must be passed; otherwise the daemon exits with `HTTP_BIND_NON_LOOPBACK_REFUSED` and `ExitCode 2`. (`crates/synapse-mcp/src/http/transport.rs::serve`)
- The `Host` header on incoming HTTP must be `127.0.0.1`, `localhost`, or `::1` (case-insensitive, brackets stripped). Otherwise the response is `403 HTTP_ORIGIN_REFUSED`. (`crates/synapse-mcp/src/http/auth.rs::host_allowed`)
- The `Origin` header, if present, must scheme `http` and have a loopback host. Missing Origin is accepted only when the bind is itself loopback. (`crates/synapse-mcp/src/http/auth.rs::validate_origin`)
- The `Authorization` header must carry `Bearer <token>` (case-insensitive scheme). Token compared by constant-time SHA-256 (`subtle::ConstantTimeEq`). Missing or malformed → `401 HTTP_TOKEN_INVALID`.
- Token source priority: `%APPDATA%/synapse/token.txt` (if file exists), otherwise `SYNAPSE_BEARER_TOKEN`. Empty token in either source refuses startup.
- `/mcp` routes require a non-empty `Mcp-Session-Id` header on GET/DELETE. POST is allowed without the header only if the body is a JSON-RPC `initialize` (parsed in `enforce_session_header`).
- POST body to `/mcp` is capped at 1 MiB (`MAX_MCP_REQUEST_BYTES`); larger → `413 PAYLOAD_TOO_LARGE`.
- `Last-Event-ID` header on `GET /events` must parse as `u64` or → `400 BAD REQUEST` "malformed Last-Event-ID". (`crates/synapse-mcp/src/http/sse.rs::parse_last_event_id`)
- Idle timeout `> 0` strictly. Zero or non-integer values refuse startup.

### 4.2 Storage
- RocksDB `__schema_version` sentinel key must match `synapse_core::SCHEMA_VERSION` (=`1`); mismatch returns `StorageError::SchemaMismatch` → `STORAGE_SCHEMA_MISMATCH`.
- Every column family listed in `crates/synapse-storage/src/cf.rs::ALL_COLUMN_FAMILIES` (11 CFs) must be open after `Db::open`; otherwise `STORAGE_OPEN_FAILED`.
- Writes during a non-`Normal` disk-pressure level are silently dropped for CFs the responder has frozen (`Db::put_batch` returns `Ok(())` after a warn-level trace). (`crates/synapse-storage/src/lib.rs::put_batch`)

### 4.3 Reflex
- `priority` ≤ `MAX_REFLEX_PRIORITY` (`crates/synapse-reflex/src/scheduler.rs::MAX_REFLEX_PRIORITY`); larger → `REFLEX_PRIORITY_INVALID`.
- Total active reflexes ≤ `MAX_SCHEDULED_REFLEXES`; reaching the cap returns `REFLEX_CAP_REACHED` on subsequent registration.
- `EventFilter::validate` rejects empty `And`/`Or` and trees deeper than `synapse_core::EVENT_FILTER_MAX_DEPTH = 8` → `REFLEX_FILTER_INVALID` (or `TOOL_PARAMS_INVALID` when surfaced through a tool).
- Subscription count on the SSE bus ≤ `--max-subscriptions`. Exceeding → `SUBSCRIPTION_CAP_REACHED`.
- On-event recursion guard caps fires per tick at `MAX_ON_EVENT_FIRINGS_PER_TICK`; over-cap clamps audit `REFLEX_RECURSION_LIMIT`.

### 4.4 Permissions (M3)

Permission names and aliases (`crates/synapse-mcp/src/m3/permissions.rs::Permission::parse`):

| Canonical | Accepted aliases | Required by tool(s) |
|---|---|---|
| `READ_EVENTS` | — | `subscribe`, `subscribe_cancel` |
| `WRITE_REFLEX` | — | `reflex_register` |
| `READ_REFLEX` | — | `reflex_cancel`, `reflex_list`, `reflex_history` |
| `READ_PROFILE` | — | `profile_list` |
| `WRITE_PROFILE_ACTIVE` | — | `profile_activate` |
| `WRITE_REPLAY` | — | `replay_record` |
| `READ_AUDIO` | — | `audio_tail`, `audio_transcribe` (also requires `--enable-audio`) |
| `INPUT_KEYBOARD` | `KEYBOARD` | implicitly required by any reflex whose `then` actions touch the keyboard |
| `INPUT_MOUSE` | `MOUSE` | reflex actions touching the mouse |
| `INPUT_PAD` | `PAD` | reflex actions touching the gamepad |
| `INPUT_HARDWARE_HID` | `HARDWARE_HID` | reflex actions whose `backend = Hardware` |

Default permission set when `--allowed-permissions` is omitted (`default_grants` in `permissions.rs`): all the above except `READ_AUDIO` (added only when `--enable-audio`) and `INPUT_HARDWARE_HID`.

If `--allowed-permissions` includes `READ_AUDIO` but `--enable-audio` is not passed, startup fails with `READ_AUDIO requires --enable-audio or SYNAPSE_ENABLE_AUDIO=true`.

Sentinel values `NONE` / `DENY_ALL` produce an empty grants set (every M3 tool will return `SAFETY_PERMISSION_DENIED`).

### 4.5 Replay paths
- `replay_record`'s optional `path` parameter is normalized (`lexical_normalize`) and must resolve under `replay_root()` (default `%LOCALAPPDATA%/synapse/replays`). Anything escaping the root → `SAFETY_PERMISSION_DENIED` with detail `path_outside_allow_root`. (`crates/synapse-mcp/src/m3/permissions.rs::normalize_replay_path`)
- Default name when `path` is omitted: `replay-<uuid-v7>.jsonl`.

### 4.6 Profiles
- Activating a profile whose `use_scope = unknown` without `--allow-unknown-profile` → `SAFETY_PROFILE_ACTION_DENIED`. (`crates/synapse-mcp/src/m3/profile.rs::activate_profile`)

### 4.7 Audio
- `audio_tail.seconds` ≤ `synapse_audio::MAX_RING_SECONDS = 5`; larger → `TOOL_PARAMS_INVALID`.
- `audio_transcribe.language` accepts `"en"` (or trimmed-empty, which means `"en"`); other values → `TOOL_PARAMS_INVALID`.

### 4.8 Action
- `act_click.clicks` ∈ `1..=3`.
- `act_click.modifiers` must be empty in this build (`ACTION_BACKEND_UNAVAILABLE` with detail "act_click modifiers are not wired in the M2 click schema slice").
- `act_press.hold_ms` ∈ `1..=30000`.
- `act_press.keys` parsed by `m2/press/keys.rs`; unknown names → `ACTION_UNSUPPORTED_KEY`.
- `act_drag` distance is bounded by `synapse_action::MAX_DRAG_DISTANCE_PX`; over-limit → `ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT`.
- `act_pad.hold_ms` is internally bounded at `30_000` (`crates/synapse-mcp/src/m2/pad.rs::MAX_HOLD_MS`).
- `act_scroll.smooth` schedules at `30 ms` interval up to `120` events (`SMOOTH_SCROLL_INTERVAL_MS`, `MAX_SMOOTH_SCROLL_STEPS`).
- Token-bucket per-backend rate limits: `SOFTWARE_RATE_LIMIT_PER_S`, `VIGEM_RATE_LIMIT_PER_S` (`crates/synapse-action/src/rate_limit.rs`). Over-rate emits return `ACTION_RATE_LIMITED`.
- Action emitter queue capacity: `ACTION_QUEUE_CAPACITY = 256` (`crates/synapse-action/src/handle.rs`). Backpressure → `ACTION_QUEUE_FULL`.

### 4.9 Hardware HID Consent
- If `--hardware-hid <port|auto>` is set and `SYNAPSE_MCP_RECORDING_BACKEND` is not active, startup checks `%APPDATA%/synapse/agreement.json` (or `SYNAPSE_AGREEMENT_PATH` when set) before constructing the action backend.
- Missing agreement triggers the startup console prompt. The response must exactly equal `I AUTHORIZE HARDWARE INPUT` after line-ending removal only; leading/trailing spaces, case changes, empty input, or EOF are refused.
- Refusal logs and prints `SAFETY_PROFILE_ACTION_DENIED reason=hardware_consent_refused`, exits 2, and leaves the agreement file absent.
- A valid agreement records schema version, `acknowledged_at`, the accepted `hardware_hid.port`, the SHA-256 of the phrase, and `supported_use_scopes=["productivity","single_player"]`; Windows readback verifies the protected ACL before continuing.
- `--reset-hardware-consent` removes the existing agreement first, then follows the same prompt/write/ACL-readback path.

### 4.10 Telemetry
- Log directory must be writable: probe writes `.synapse-write-probe` and deletes it; failure → `TELEMETRY_LOG_DIR_NOT_WRITABLE`.
- Max log directory size default: `DEFAULT_MAX_DIR_BYTES = 500 * 1024 * 1024` (500 MiB), keep `DEFAULT_KEEP_DAYS = 7`, GC default interval `6 h` (overridable by `SYNAPSE_LOG_GC_INTERVAL_S`).

## 5. Config loading order

The `Cli::m2_config` method constructs `M2ServiceConfig` from `--hardware-hid` / `SYNAPSE_HARDWARE_HID` and `SYNAPSE_MCP_RECORDING_BACKEND`; `Cli::m3_config` constructs `M3ServiceConfig` from clap fields and additionally consults `SYNAPSE_BEARER_TOKEN` at that point (`crates/synapse-mcp/src/m3.rs::from_cli_parts`). All other env vars are read at their respective construction sites:

```text
clap (CLI flag > env via clap) → Cli
        │
        ├→ Cli::m2_config()  → M2ServiceConfig (hardware HID + recording backend)
        └→ Cli::m3_config()  → M3ServiceConfig (also reads SYNAPSE_BEARER_TOKEN)
                │
                ├→ configure_telemetry()  → reads SYNAPSE_LOG_DIR + SYNAPSE_LOG_GC_INTERVAL_S
                │
                └→ run_stdio / http::serve
                    ├→ M2State::try_from_config → connects hardware HID if configured; reads SYNAPSE_MCP_RECORDING_BACKEND from M2ServiceConfig
                    ├→ M1State::from_env        → reads SYNAPSE_MCP_SYNTHETIC_FIXTURE, _FORCE_NO_PERCEPTION, _FORCE_OBSERVE_INTERNAL
                    ├→ M3State::from_*          → reads SYNAPSE_BIND, SYNAPSE_BEARER_TOKEN, SYNAPSE_AUDIO_LOOPBACK
                    │      (additional env mirrors of --reflex-disabled etc. when used via M3ServiceConfig::from_env)
                    ├→ SseState::with_max_subscriptions   → reads SYNAPSE_HTTP_SSE_MANUAL
                    ├→ http::session::load_session_config → reads SYNAPSE_HTTP_SESSION_IDLE_TIMEOUT_SECS
                    └→ http::auth::HttpAuth::load         → reads %APPDATA%/synapse/token.txt, else SYNAPSE_BEARER_TOKEN
```

There is no merge step: CLI/env values configure individual subsystems independently, each at the moment the subsystem is constructed. There is no hot-reload of CLI flags or env vars — restart the daemon to change any of them. Profile TOML files, by contrast, are watched and hot-reloaded with a 200 ms debounce (see [11_profiles_hid_telemetry.md](11_profiles_hid_telemetry.md) in the deep-dives).

## 6. Defaults summary (most-referenced constants)

| Constant | Value | Source |
|---|---|---|
| `SCHEMA_VERSION` | `1` | `crates/synapse-core/src/defaults.rs` |
| `REFERENCE_OBSERVE_WARM_HYBRID_P99_MS` | `30.0` | `crates/synapse-core/src/defaults.rs` |
| `REFERENCE_REFLEX_TICK_JITTER_IDLE_P99_US` | `200` | `crates/synapse-core/src/defaults.rs` |
| `REFERENCE_EVENT_TO_SUBSCRIBER_P99_MS` | `50.0` | `crates/synapse-core/src/defaults.rs` |
| `ACTION_QUEUE_CAPACITY` | `256` | `crates/synapse-action/src/handle.rs` |
| `MAX_DRAG_DISTANCE_PX` | `4096.0` | `crates/synapse-action/src/validation.rs` |
| `SOFTWARE_RATE_LIMIT_PER_S` | `5000` | `crates/synapse-action/src/rate_limit.rs` |
| `VIGEM_RATE_LIMIT_PER_S` | `1000` | `crates/synapse-action/src/rate_limit.rs` |
| `CAPTURE_CHANNEL_CAPACITY` | `2` | `crates/synapse-capture/src/lib.rs` |
| `SUBSCRIBER_QUEUE_CAPACITY` | `4096` | `crates/synapse-reflex/src/bus.rs` |
| `DEFAULT_MAX_SUBSCRIPTIONS_NONZERO` | `64` | `crates/synapse-reflex/src/bus.rs` |
| `MAX_ON_EVENT_FIRINGS_PER_TICK` | `4` | `crates/synapse-reflex/src/kinds/on_event.rs` |
| `MAX_REFLEX_PRIORITY` / `MAX_SCHEDULED_REFLEXES` / `DEFAULT_REFLEX_PRIORITY` | `1000` / `32` / `100` | `crates/synapse-reflex/src/scheduler.rs` |
| `STARVATION_AFTER` / `REFLEX_STARVED_KIND` | conflict-resolver constants | `crates/synapse-reflex/src/conflict.rs` |
| `DEFAULT_RING_SECONDS` / `MAX_RING_SECONDS` | `5` / `5` | `crates/synapse-audio/src/lib.rs` |
| `DEFAULT_SAMPLE_RATE_HZ` / `STEREO_CHANNELS` | `48_000` / `2` | `crates/synapse-audio/src/ring.rs` |
| `WHISPER_TINY_MODEL_ID` | `"whisper_tiny_int8"` | `crates/synapse-mcp/src/m3/audio.rs` |
| `CARDINALITY_LIMIT` (metrics) | `1000` | `crates/synapse-telemetry/src/metrics.rs` |
| `BLOCK_CACHE_BYTES` | `64 MiB` | `crates/synapse-storage/src/lib.rs` |
| `DEFAULT_WRITE_BUFFER_BYTES` | `64 MiB` | `crates/synapse-storage/src/lib.rs` |
| `MODEL_CACHE_WRITE_BUFFER_BYTES` | `256 MiB` | `crates/synapse-storage/src/lib.rs` |
| `LEVEL_1..4_FREE_BYTES` (disk pressure) | `2 GB / 1 GB / 500 MB / 200 MB` | `crates/synapse-storage/src/pressure.rs` |
| Storage GC interval | `5 min` | `crates/synapse-storage/src/gc.rs` |
| Storage pressure poll | `30 s` | `crates/synapse-storage/src/pressure.rs` |
| Log GC default interval | `6 h` | `crates/synapse-telemetry/src/lib.rs` |
| Log GC keep | `7 days` | `crates/synapse-telemetry/src/lib.rs` |
| Log GC size ceiling | `500 MiB` | `crates/synapse-telemetry/src/lib.rs` |
| Profile watcher debounce | `200 ms` | `crates/synapse-profiles/src/watcher.rs::WATCH_DEBOUNCE` |
| HTTP session idle timeout default | `30 min` | `crates/synapse-mcp/src/http/session.rs::DEFAULT_SESSION_IDLE_TIMEOUT_SECS` |
| HTTP MCP body cap | `1 MiB` | `crates/synapse-mcp/src/http/session.rs::MAX_MCP_REQUEST_BYTES` |
| SSE poll interval | `20 ms` | `crates/synapse-mcp/src/http/sse.rs::SSE_POLL_INTERVAL` |
| SSE `buffer_size` (subscribe tool) | hard-pinned `4096` | `crates/synapse-mcp/src/m3/subscribe.rs::DEFAULT_BUFFER_SIZE` |
| `reflex_history.limit` cap | `1000` | `crates/synapse-mcp/src/m3/reflex.rs::MAX_REFLEX_HISTORY_LIMIT` |
| `audit_export_bundle.max_rows` default / cap | `100` / `1000` | `crates/synapse-mcp/src/m3/audit_export.rs` |
| `audit_export_bundle.max_row_bytes` default / cap | `65536` / `524288` | `crates/synapse-mcp/src/m3/audit_export.rs` |
| `audit_export_consent_set.redaction_policy` default | `strict` | `crates/synapse-mcp/src/m3/audit_export.rs` |
| `reflex_register` default priority | `100` | `crates/synapse-core/src/types.rs::default_reflex_priority` + `synapse_reflex::DEFAULT_REFLEX_PRIORITY` |
| `act_press` default hold_ms | `33` | `crates/synapse-mcp/src/m2/press/schema.rs::DEFAULT_HOLD_MS` |
| `act_aim` default deadline_ms | `80` | `crates/synapse-mcp/src/m2/aim.rs::DEFAULT_DEADLINE_MS` |
| `act_aim` snap/flick/natural duration | `50 / 35 / 150 ms` | `crates/synapse-mcp/src/m2/aim.rs` |
| `act_drag` default duration_ms | `200` | `crates/synapse-mcp/src/m2/drag.rs::DEFAULT_DRAG_DURATION_MS` |
| `act_click` default duration_ms | `50` | `crates/synapse-mcp/src/m2/click/schema.rs::default_click_duration_ms` |
| `act_scroll` smooth interval | `30 ms` (≤120 steps) | `crates/synapse-mcp/src/m2/scroll.rs` |
| `replay_record` observation sample | `250 ms` | `crates/synapse-mcp/src/m3/replay.rs::OBSERVATION_SAMPLE_INTERVAL` |
| `replay_record` event drain | `20 ms` | `crates/synapse-mcp/src/m3/replay.rs::EVENT_DRAIN_INTERVAL` |
| Operator hotkey | `Ctrl+Alt+Shift+P` | `crates/synapse-action/src/hotkey.rs` (referenced by `synapse-mcp/src/safety.rs`) |
| Operator release_all timeout | `50 ms` | `crates/synapse-mcp/src/safety.rs::OPERATOR_RELEASE_ALL_TIMEOUT` |
