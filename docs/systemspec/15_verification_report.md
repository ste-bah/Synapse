# 15 — Verification Report

Source files covered:
- All workspace `Cargo.toml`s
- All Rust source files
- `CHANGELOG.md`, `README.md`, `AGENTS.md`
- `docs/impplan/README.md`

This is the snapshot of the codebase health and basic metrics derived directly from the source tree on branch `main`, HEAD `e54ca57`, post-M3 (tag `v0.1.0-m3` @ `97019ec`, 2026-05-25).

## 1. Test results summary

`cargo test --workspace` was not executed as part of generating this documentation (per the user task instruction, this is a "review and document what is" pass, not an execution pass). The counts below are derived from static parsing of attributes in source files.

| Metric | Count |
|---|---|
| `#[test]` + `#[tokio::test]` attributes across the workspace | **381** |
| Integration-test files (`crates/*/tests/*.rs`) | **76** |
| Bench files (`crates/*/benches/*.rs`) | **13** |
| Source files using `#[test]` or `#[tokio::test]` somewhere | 104 |

Per the doctrine in [12_milestones_and_roadmap.md §5](12_milestones_and_roadmap.md) ("operator-level invariants"), the **shipping gate is manual FSV on the configured Windows host, not `cargo test`**. Running `cargo test --workspace` is the supporting-evidence test gate for the per-PR contract.

To execute the suite locally (per `docs/impplan/README.md`'s per-PR contract):

```powershell
cargo build --release --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## 2. Lint configuration and status

Workspace-wide lints from `Cargo.toml`:

```toml
[workspace.lints.rust]
unsafe_code = "forbid"
unused      = { level = "warn", priority = -1 }

[workspace.lints.clippy]
all     = "deny"
pedantic = { level = "warn", priority = -1 }
nursery  = { level = "warn", priority = -1 }
unwrap_used = "deny"
expect_used = "deny"
```

Crate-level lints override `unsafe_code` to `allow` in five FFI crates:

| Crate | Reason |
|---|---|
| `synapse-action` | Win32 SendInput, low-level keyboard hook, ViGEm FFI |
| `synapse-a11y` | UI Automation COM interop |
| `synapse-audio` | WASAPI loopback FFI |
| `synapse-capture` | DXGI / Direct3D11 FFI |
| `synapse-hid-host` | serial-port + OS-handle interop |

`synapse-mcp`, `synapse-core`, `synapse-perception`, `synapse-models`, `synapse-profiles`, `synapse-reflex`, `synapse-storage`, `synapse-telemetry`, `synapse-test-utils`, `synapse-overlay` all retain `unsafe_code = forbid`.

Lint status was not executed during this documentation pass. The doctrine target is **zero warnings** (`cargo clippy --workspace --all-targets -- -D warnings` per the per-PR contract).

## 3. Codebase metrics

Counted by walking `crates/` and slicing by path. Comments, blank lines, and `mod tests` blocks are all counted in "Lines of code".

| Metric | Count |
|---|---|
| Workspace member crates | **15** |
| Default workspace binaries (`synapse-mcp`, `synapse-overlay`) | 2 |
| Total Rust source files (excluding tests/benches) | **151** |
| Total Rust integration-test files | 76 |
| Total Rust bench files | 13 |
| MCP tools registered in `server.rs` | **64** (M1/M2/M3/M4/M5 plus EverQuest runtime tools including `/loc`, chat-input state, current-state, map sensor, outcome ingest, memory, planner guard, route plan, DynamicJEPA domain normalize, and action-prior scorecard) |
| MCP tools approved by `05_mcp_tool_surface.md` (agent surface cap) | 64 |
| RocksDB column families | **11** (`ALL_COLUMN_FAMILIES.len() == 11`; excludes implicit `default` CF) |
| Stable error-code constants in `synapse_core::error_codes` | **105** |
| Reserved subsystem error enums (mapped to those codes) | 11 (`StorageError`, `ReflexError`, `ActionError`, `ProfileError`, `ProfileLoadError`, `AudioError`, `PerceptionError`, `CaptureError`, `ModelError`, `A11yError`, `TelemetryError` + parse errors `ElementIdParseError`/`EventFilterValidationError`) |
| M3 metric specs declared in `synapse_telemetry::metrics::M3_METRICS` | **19** (12 counters, 5 gauges, 2 histograms) |
| Permissions in M3 grant model | **13** (`READ_EVENTS`, `WRITE_REFLEX`, `READ_REFLEX`, `READ_PROFILE`, `WRITE_PROFILE_ACTIVE`, `WRITE_REPLAY`, `READ_AUDIO`, `READ_STORAGE`, `WRITE_STORAGE`, `INPUT_KEYBOARD`, `INPUT_MOUSE`, `INPUT_PAD`, `INPUT_HARDWARE_HID`) |
| Reflex kinds | 5 (`AimTrack`, `HoldMove`, `HoldButton`, `Combo`, `OnEvent`) |
| Lines of code (source, excl. tests/benches) | **~36 914** |
| Lines of code (integration tests) | **~18 260** |
| Lines of code (benches) | **~2 389** |
| Total LoC across `crates/` | **~57 563** |

Per-crate `lib.rs`/`main.rs` size (the deepest single-file entry points):

| Crate | Entry file | LoC |
|---|---|---|
| `synapse-mcp` | `src/main.rs` | 302 |
| `synapse-mcp` | `src/server.rs` | 1 335 |
| `synapse-core` | `src/types.rs` | 1 567 |
| `synapse-core` | `src/error_codes.rs` | 122 |
| `synapse-storage` | `src/lib.rs` | (per source) |
| `synapse-reflex` | `src/lib.rs` | 986 |
| `synapse-reflex` | `src/scheduler.rs` | 890 |
| `synapse-action` | `src/lib.rs` | re-exports only |
| `synapse-audio` | `src/lib.rs` | (per source) |
| `synapse-a11y` | `src/lib.rs` | 2 087 (single-file lib on `main`; modular split queued for M4 Block A.0) |
| `synapse-capture` | `src/lib.rs` | 1 798 |
| `synapse-perception` | `src/lib.rs` | re-exports only |
| `synapse-profiles` | `src/lib.rs` | re-exports only |
| `synapse-models` | `src/lib.rs` | 535 |
| `synapse-telemetry` | `src/lib.rs` | (per source) |
| `synapse-telemetry` | `src/metrics.rs` | (per source) |
| `synapse-hid-host` | `src/` | multi-file serial gateway (see source map) |
| `synapse-overlay` | `src/main.rs` | (M5 stub) |

Files exceeding the 500-LoC impplan rule on `main` (M3 carry-over per `docs/impplan/04_m3_reflex_mcp_surface.md` — M4 Block A.0 splits before adding hardware HID):

| File | LoC | Note |
|---|---|---|
| `crates/synapse-mcp/src/server.rs` | 1 335 | tool router; exempt by design |
| `crates/synapse-core/src/types.rs` | 1 567 | type catalog; exempt by design |
| `crates/synapse-a11y/src/lib.rs` | 2 087 | M4 Block A.0 splits into `platform/*` modules |
| `crates/synapse-capture/src/lib.rs` | 1 798 | M4 Block A.0 splits |
| `crates/synapse-mcp/src/m3/reflex.rs` | 1 165 | M4 Block A.0 splits |
| `crates/synapse-reflex/src/lib.rs` | 986 | M4 Block A.0 splits |
| `crates/synapse-reflex/src/scheduler.rs` | 890 | M4 Block A.0 splits |
| `crates/synapse-mcp/src/http/sse.rs` | 764 | M4 Block A.0 splits |
| `crates/synapse-mcp/src/m3/replay.rs` | 651 | M4 Block A.0 splits |
| `crates/synapse-models/src/lib.rs` | 535 | M4 Block A.0 splits |

## 4. Build / packaging summary

| Field | Value |
|---|---|
| Cargo edition | `2024` |
| Rust toolchain | `rust-version = "1.95"` (current installed stable; ADR-0001) |
| Workspace package version | `0.1.0` |
| Workspace license | `MIT OR Apache-2.0` |
| Repository | `https://github.com/ChrisRoyse/Synapse` |
| Default release profile | `opt-level=3`, `lto="thin"`, `codegen-units=16`, `strip=true`, `panic="abort"` |
| Release-max profile | inherits release with `lto="fat"`, `codegen-units=1` |
| Excluded paths | `firmware/pico-hid` (M4 work; not yet in tree) |
| Binary outputs | `target/release/synapse-mcp[.exe]`, `target/release/synapse-overlay[.exe]` |

## 5. Schema version

| Setting | Value | Source |
|---|---|---|
| `synapse_core::SCHEMA_VERSION` | `1` | `crates/synapse-core/src/defaults.rs` |
| RocksDB sentinel key | `__schema_version` | `crates/synapse-storage/src/lib.rs::SCHEMA_VERSION_KEY` |
| Stored payload variants carrying `schema_version: u32` | `StoredEvent`, `StoredObservation`, `StoredReflexAudit`, `StoredSession` | `crates/synapse-core/src/types.rs` |
| Persisted codecs | JSON only (`encode_json`/`decode_json`) | `crates/synapse-storage/src/codecs.rs` |
| Schema-mismatch behavior | `STORAGE_SCHEMA_MISMATCH` returned; **no migration framework**. Pre-v1 doctrine: wipe-and-rebuild. | `crates/synapse-storage/src/lib.rs::verify_schema_version` |

## 6. Notable constants and magic numbers

(Derived from the source; cross-referenced from earlier docs.)

| Name | Value | Location |
|---|---|---|
| `SCHEMA_VERSION` | `1` | `synapse_core::defaults` |
| `REFERENCE_OBSERVE_WARM_HYBRID_P99_MS` | `30.0` | `synapse_core::defaults` |
| `REFERENCE_REFLEX_TICK_JITTER_IDLE_P99_US` | `200` | `synapse_core::defaults` |
| `REFERENCE_EVENT_TO_SUBSCRIBER_P99_MS` | `50.0` | `synapse_core::defaults` |
| `EVENT_FILTER_MAX_DEPTH` | `8` | `synapse_core::types` |
| `ACTION_QUEUE_CAPACITY` | `256` | `synapse_action::handle` |
| `MAX_DRAG_DISTANCE_PX` | `4096.0` | `synapse_action::validation` |
| `SOFTWARE_RATE_LIMIT_PER_S` | `5000` | `synapse_action::rate_limit` |
| `VIGEM_RATE_LIMIT_PER_S` | `1000` | `synapse_action::rate_limit` |
| `CAPTURE_CHANNEL_CAPACITY` | `2` | `synapse_capture` |
| `SUBSCRIBER_QUEUE_CAPACITY` | `4096` | `synapse_reflex::bus` |
| `DEFAULT_MAX_SUBSCRIPTIONS` | `64` | `synapse_reflex::bus` |
| `MAX_ON_EVENT_FIRINGS_PER_TICK` | `4` | `synapse_reflex::kinds::on_event` |
| `MAX_REFLEX_PRIORITY` | `1000` | `synapse_reflex::scheduler` |
| `MAX_SCHEDULED_REFLEXES` | `32` | `synapse_reflex::scheduler` |
| `DEFAULT_REFLEX_PRIORITY` | `100` | `synapse_reflex::scheduler` |
| `DEFAULT_SAMPLE_LIMIT` (scheduler) | `4096` | `synapse_reflex::scheduler` |
| Scheduler target/fallback intervals | `1 ms` / `2 ms` | `synapse_reflex::scheduler::SchedulerConfig::default` |
| `STARVATION_AFTER` (reflex) | (see `synapse_reflex::conflict`) | — |
| `DEFAULT_RING_SECONDS` | `5` | `synapse_audio` |
| `MAX_RING_SECONDS` | `5` | `synapse_audio` |
| `DEFAULT_SAMPLE_RATE_HZ` | `48_000` | `synapse_audio::ring` |
| `STEREO_CHANNELS` | `2` | `synapse_audio::ring` |
| `WHISPER_TINY_MODEL_ID` | `"whisper_tiny_int8"` | `synapse_mcp::m3::audio` |
| `CARDINALITY_LIMIT` (metrics) | `1000` | `synapse_telemetry::metrics` |
| `BLOCK_CACHE_BYTES` | `64 MiB` | `synapse_storage` |
| `DEFAULT_WRITE_BUFFER_BYTES` | `64 MiB` | `synapse_storage` |
| `MODEL_CACHE_WRITE_BUFFER_BYTES` | `256 MiB` | `synapse_storage` |
| Storage disk-pressure thresholds | `2 GB / 1 GB / 500 MB / 200 MB` | `synapse_storage::pressure` |
| Storage GC interval | `5 min` | `synapse_storage::gc::GC_INTERVAL` |
| Storage pressure poll | `30 s` | `synapse_storage::pressure::POLL_INTERVAL` |
| Telemetry log dir size cap | `500 MiB` | `synapse_telemetry::DEFAULT_MAX_DIR_BYTES` |
| Telemetry log retention | `7 days` | `synapse_telemetry::DEFAULT_KEEP_DAYS` |
| Telemetry GC default interval | `6 h` | `synapse_telemetry::DEFAULT_GC_INTERVAL` |
| Profile watcher debounce | `200 ms` | `synapse_profiles::watcher::WATCH_DEBOUNCE` |
| HTTP session idle default | `30 min` | `synapse_mcp::http::session::DEFAULT_SESSION_IDLE_TIMEOUT_SECS` |
| HTTP MCP body cap | `1 MiB` | `synapse_mcp::http::session::MAX_MCP_REQUEST_BYTES` |
| SSE poll interval | `20 ms` | `synapse_mcp::http::sse::SSE_POLL_INTERVAL` |
| Subscribe `buffer_size` (pinned) | `4096` | `synapse_mcp::m3::subscribe::DEFAULT_BUFFER_SIZE` |
| `reflex_history.limit` cap | `1000` | `synapse_mcp::m3::reflex::MAX_REFLEX_HISTORY_LIMIT` |
| `act_press` default hold_ms | `33` | `synapse_mcp::m2::press::schema::DEFAULT_HOLD_MS` |
| `act_aim` default deadline_ms | `80` | `synapse_mcp::m2::aim::DEFAULT_DEADLINE_MS` |
| `act_aim` style → duration | Snap 50 / Flick 35 / Natural 150 | `synapse_mcp::m2::aim` |
| `act_drag` default duration_ms | `200` | `synapse_mcp::m2::drag` |
| `act_click` default duration_ms | `50` | `synapse_mcp::m2::click::schema::default_click_duration_ms` |
| `act_scroll` smooth interval / max steps | `30 ms` / `120` | `synapse_mcp::m2::scroll` |
| `replay_record` observation interval | `250 ms` | `synapse_mcp::m3::replay::OBSERVATION_SAMPLE_INTERVAL` |
| `replay_record` event drain interval | `20 ms` | `synapse_mcp::m3::replay::EVENT_DRAIN_INTERVAL` |
| Operator hotkey | `Ctrl+Alt+Shift+P` | `synapse_action::hotkey` |
| Operator release_all timeout | `50 ms` | `synapse_mcp::safety::OPERATOR_RELEASE_ALL_TIMEOUT` |
| `MCP_CLI_PARSED` / `MCP_STDIO_STARTED` / `MCP_STDIO_EOF_CONNECTION_CLOSED` / `MCP_HTTP_STARTED` / `MCP_HTTP_HEALTH` / `MCP_HTTP_AUTH_CONFIGURED` / `MCP_HTTP_SESSION_CONFIGURED` / `MCP_HTTP_SHUTDOWN_TIMEOUT` / `MCP_SHUTDOWN_GRACEFUL` | tracing-only event codes | `synapse_mcp::main` / `http::*` |

## 7. Documentation map (this systemspec)

| File | Topic |
|---|---|
| [01_system_overview.md](01_system_overview.md) | High-level architecture, tech stack, all 50 live tools |
| [02_source_code_map.md](02_source_code_map.md) | Per-file tree with descriptions + dep graph + entry-point traces |
| [03_configuration.md](03_configuration.md) | All CLI flags, env vars, validation rules, default constants |
| [04_storage_layer.md](04_storage_layer.md) | RocksDB CFs, schema sentinel, TTL filter, GC, disk pressure |
| [05_core_types_and_errors.md](05_core_types_and_errors.md) | `synapse-core` types, error codes, Stored* variants |
| [06_mcp_service_and_transports.md](06_mcp_service_and_transports.md) | `SynapseService`, stdio + HTTP routers, Bearer/Origin/Session middleware, SSE bridge |
| [07_reflex_runtime.md](07_reflex_runtime.md) | EventBus, scheduler, the 5 reflex kinds, audit persistence |
| [08_action_subsystem.md](08_action_subsystem.md) | Emitter actor, backends, rate limits, hotkey, curves/dynamics, error mapping |
| [09_perception_and_capture.md](09_perception_and_capture.md) | Frame capture, UIA + WinEvent, perception assembler, OCR |
| [10_audio_and_models.md](10_audio_and_models.md) | WASAPI loopback, ring + STT (Whisper-tiny), ONNX model loader |
| [11_profiles_hid_telemetry.md](11_profiles_hid_telemetry.md) | TOML profile loader + watcher, HID stub, tracing + metrics, test utils |
| [12_milestones_and_roadmap.md](12_milestones_and_roadmap.md) | Milestone state, ADRs, doctrine, open questions |
| [13_mcp_tool_reference.md](13_mcp_tool_reference.md) | Every tool's params, defaults, ranges, side effects, errors |
| [14_test_suite.md](14_test_suite.md) | Test inventory by crate, run commands, fixtures |
| [15_verification_report.md](15_verification_report.md) | This file |

## 8. Recent commits (informational)

Recent commits on `main` after the M3 tag was cut:

```
e54ca57 docs(impplan): roll forward to M3-closed / M4-active state [skip ci]
6ed52e4 docs: add systemspec reference and compression prompt [skip ci]
97019ec fix: tolerate transient replay perception gaps [skip ci]    ← v0.1.0-m3 tag here
95af9a0 docs: add m3 release notes [skip ci]
eef654f fix: route recording mode through action actor [skip ci]
6c9fec0 bench: pace software press benchmark [skip ci]
```

The doctrine `[skip ci]` suffix is present on every agent commit, consistent with `AGENTS.md`.

## 9. Limitations of this verification

- **No code execution.** This pass did not run `cargo build`, `cargo clippy`, `cargo test`, or `cargo bench`. All counts come from static source scans. The "supporting evidence" gate for the per-PR contract still needs to be passed by the operator manually.
- **No FSV.** Per `AGENTS.md`, FSV must be performed manually by the agent at the configured Windows host with explicit source-of-truth readback before and after each trigger. This documentation pass does not constitute FSV.
- **No live state.** RocksDB CF row counts, current audit history, active session counts, and operator-configured profiles are runtime state; the documentation captures schema and code-path behavior, not live observations.

## 10. What is NOT covered anywhere in this systemspec

- **The RP2040 firmware** (`firmware/pico-hid/`) is referenced as `Cargo.toml::exclude` but the directory is not in the repository in this snapshot. M4 work.
- **The Florence-2 VLM `describe` tool** has no source code; reserved for M5.
- **Installer / signing** (`docs/computergames/14_build_and_packaging.md` references a planned MSIX installer) — not yet implemented.
- **Debug overlay UI** (`synapse-overlay` is a 1-line `fn main() {}`) — M5 work.
- **Linux/macOS production paths** — every OS-bound subsystem returns the appropriate `*_NOT_AVAILABLE` / `*_BACKEND_UNAVAILABLE` error on non-Windows; PRD treats cross-platform as v2.
