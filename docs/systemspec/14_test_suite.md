# 14 — Test Suite

Source files covered:
- Every file under `crates/*/tests/`
- Every file under `crates/*/benches/`
- `tests/fixtures/audio/*.wav`
- `crates/synapse-test-utils/src/*.rs`
- `docs/impplan/00_methodology.md` (FSV doctrine)

## 1. Test categories

| Category | Where | Purpose |
|---|---|---|
| **Unit tests** | `#[test]` / `#[tokio::test]` inside `mod tests` of each `src/*.rs` file | Per-function / per-module correctness |
| **Integration tests** | `crates/*/tests/*.rs` (each file = its own binary) | Per-crate public surface against the real OS where applicable |
| **Property tests** | `proptest` macros embedded in unit + integration tests | Round-trip and invariant checks |
| **Snapshot tests** | `insta` macros in `synapse-core` and `synapse-action` | Canonicalized JSON output of types and recorded action sequences |
| **Benchmarks** | `crates/*/benches/*.rs` (criterion) | Perf-regression detection against §7 budgets in [12_milestones_and_roadmap.md](12_milestones_and_roadmap.md) |
| **Repo-level fixtures** | `tests/fixtures/audio/*.wav` | Shared WAV samples used by audio tests |
| **Manual FSV (operator-driven)** | NOT in this repo as automated tests | Per `docs/impplan/00_methodology.md` §5, FSV is the shipping gate and is manual; "supporting evidence only" applies to everything else in this section |

## 2. Integration-test inventory by crate (76 files)

### 2.1 `synapse-action` (21 files)
- `auto_release_keyboard_hook.rs` — verifies `HELD_KEY_MAX_DURATION_MS` auto-release path
- `backend_resolution.rs` — `resolve_backend(Backend, &Action)` mapping
- `curve_natural_seed_42.rs` — fixed-seed natural-curve sampling determinism
- `curve_sampling.rs` — `sample_curve` for `Linear`/`EaseInOut`/`Bezier`/`Natural`
- `dynamics_modifier_order_proptest.rs` — keystroke modifier-ordering invariants
- `dynamics_natural_hello_world.rs` — fixed-input keystroke schedule
- `dynamics_round_trip_proptest.rs` — schedule round-trip
- `dynamics_schedule.rs` — `sample_typing_schedule` correctness
- `emitter_state.rs` — held bitset / pad cache after sequences
- `error_codes_match.rs` — `ActionError::code()` mapping
- `handle_queue.rs` — bounded mpsc + ack behavior
- `hardware_unavailable.rs` — `HardwareUnavailableBackend` returns `ACTION_BACKEND_UNAVAILABLE` with `--hardware-hid <port|auto>` guidance
- `mouse_drag_validation.rs` — `MAX_DRAG_DISTANCE_PX` enforcement
- `rate_limit_overshoot.rs` — token bucket retry_after_ms accuracy
- `recording_backend.rs` — `RecordingBackend` event log
- `release_all_logging.rs` — `Action::ReleaseAll` drains snapshot
- `safety_no_handle.rs` — operator hotkey fallback when `RELEASE_ALL_HANDLE` unset
- `safety_panic_hook.rs` — `install_panic_hook` releases held inputs
- `safety_timeout.rs` — `fire_release_all_blocking_with_timeout` timing
- `software_non_windows.rs` — Linux/macOS stub returns `ACTION_BACKEND_UNAVAILABLE`
- `vigem_xinput.rs` — ViGEm X360 plug + report round-trip
- Plus emitter sub-tests under `src/emitter/tests/`: `mod.rs`, `auto_release.rs`, `rate_limit.rs`

### 2.2 `synapse-audio` (4 files)
- `direction.rs` — `estimate_direction` over pan fixture
- `ring_detectors.rs` — ring buffer + detector pipeline
- `runtime_scaffold.rs` — `AudioRuntime::spawn` lifecycle
- `stt.rs` — Whisper-tiny load + silence handling

### 2.3 `synapse-core` (11 files)
- `action_serde_proptest.rs`, `action_snapshots.rs`, `action_types.rs` — `Action` enum
- `error_codes_literal.rs` — every error-code constant matches its name (no typos)
- `event_filter_types.rs` — `EventFilter::validate` + `matches`
- `ocr_types.rs` — `OcrResult` / `OcrWord` round-trips
- `profile_types.rs` — `Profile` round-trip
- `reflex_types.rs` — `ReflexKind` / `ReflexRegistration` round-trips
- `snapshots.rs` — global insta JSON snapshots
- `stored_types.rs` — `StoredEvent` / `StoredObservation` / `StoredReflexAudit` / `StoredSession` round-trips
- `types.rs` — primitives (`ElementId`, `Point`, `Rect`, `Size`)

### 2.4 `synapse-mcp` (18 files — covers M0/M2/M3 end-to-end)
- `cli_modes.rs` — `--mode stdio`/`http` + `--help` parsing
- `drop_kills_child.rs` — `StdioMcpClient` cleanly kills the child on drop
- `health_tools_list.rs` — `tools/list` returns the expected M0+M1 tools
- `m0_demo_gate.rs` — M0 acceptance: stdio + `health` end-to-end
- `m2_notepad_type_save.rs` — M2 acceptance: launch Notepad, type, save, verify file
- `m3_audio_tail_tool.rs`, `m3_audio_transcribe_tool.rs` — M3 audio tools (uses WAV fixtures)
- `m3_default_resolution.rs` — pins M3 tool defaults (`reflex_history.limit=50`, etc.)
- `m3_permissions_tool.rs` — `SAFETY_PERMISSION_DENIED` paths for every M3 tool
- `m3_profile_tools.rs` — `profile_list`/`profile_activate` + use_scope=unknown gating
- `m3_reflex_cancel_tool.rs`, `m3_reflex_history_tool.rs`, `m3_reflex_list_tool.rs`, `m3_reflex_register_tool.rs` — reflex CRUD
- `m3_replay_record_tool.rs` — replay JSONL writer
- `m3_subscribe_tool.rs` — subscribe + cancel
- `m3_tools_list.rs` — `tools/list` returns all 30 tools (15 M1+M2 + 15 M3 incl. 4 `storage_*` diagnostics)
- `sigint_clean_exit.rs` — Ctrl-C / Ctrl-Break shuts the daemon down within deadline

### 2.5 `synapse-models` (1 file)
- `model_loader.rs` — SHA-256 verification + missing-file paths

### 2.6 `synapse-perception` (1 file)
- `perception_regression.rs` — observe-assembly invariants + fixture-driven regression

### 2.7 `synapse-profiles` (2 files)
- `parse_bundled.rs` — bundled profile TOML loads without error
- `runtime_refresh.rs` — notify-driven refresh + `last_reload_at`

### 2.8 `synapse-reflex` (5 files)
- `aim_track_behavior.rs` — EMA smoothing + deadzone + axis lock + track-lost
- `bus_behavior.rs` — EventBus subscribe + filter + drop accounting + cap
- `combo_behavior.rs` — combo step scheduling + completion event
- `hold_move_behavior.rs` — hold lifecycle + re-assert
- `scheduler_behavior.rs` — tick jitter sampling + `degraded_latency` flag

### 2.9 `synapse-storage` (7 files)
- `batch_throughput.rs` — batcher correctness under put_batch storm
- `cf_names.rs` — `ALL_COLUMN_FAMILIES` shape (11 entries, exact names)
- `compaction_ttl_proptest.rs` — TTL filter retains fresh rows, drops stale ones
- `disk_pressure_4_levels.rs` — synthetic free-byte samples flip pressure levels
- `gc_soft_cap.rs` — GC evicts above soft cap and stops at it
- `open_all_cfs.rs` — every CF handle is available after open
- `scaffold.rs` — `Db::open` happy path + schema-mismatch error

### 2.10 `synapse-telemetry` (3 files)
- `file_sink.rs` — JSON file appender writes structured spans
- `periodic_gc.rs` — log GC removes files older than `keep_days`
- `periodic_gc_size_cap.rs` — log GC drops oldest when `max_dir_bytes` exceeded

## 3. Test method count

`#[test]` + `#[tokio::test]` attributes across `crates/`: **381** (counted via `awk` on the tree; includes both unit `mod tests` blocks and integration test files).

## 4. Bench inventory (13 files)

| Crate | Bench | Tests budget |
|---|---|---|
| `synapse-a11y` | `uia_snapshot_depth2_60elem.rs` | UIA tree snapshot p99 ≤ 10 ms |
| `synapse-action` | `action_curve_step_calc_natural.rs` | Curve sampling cost |
| `synapse-action` | `action_software_press.rs` | Software backend key press latency (`act_press` to electrical signal ≤ 2 ms) |
| `synapse-action` | `action_recording_round_trip.rs` | Recording backend overhead |
| `synapse-capture` | `capture_loop.rs` | Frame capture p99 ≤ 3 ms |
| `synapse-perception` | `observe_warm_a11y_only.rs` | `observe()` p99 ≤ 30 ms (a11y_only) |
| `synapse-perception` | `observe_warm_hybrid.rs` | `observe()` p99 ≤ 30 ms (hybrid; `REFERENCE_OBSERVE_WARM_HYBRID_P99_MS`) |
| `synapse-perception` | `ocr_read_text.rs` | OCR cost on canonical fixture |
| `synapse-reflex` | `event_to_subscriber.rs` | Event push p99 ≤ 50 ms (`REFERENCE_EVENT_TO_SUBSCRIBER_P99_MS`) |
| `synapse-reflex` | `reflex_combo_step_interval.rs` | Combo step accuracy |
| `synapse-reflex` | `reflex_tick_jitter_idle.rs` | Scheduler tick jitter idle p99 ≤ 200 µs (`REFERENCE_REFLEX_TICK_JITTER_IDLE_P99_US`) |
| `synapse-reflex` | `reflex_tick_jitter_under_load.rs` | Scheduler tick jitter under load |
| `synapse-storage` | `batch_throughput.rs` | put_batch / flush rates |

All benches use `criterion 0.8` with `harness = false`. The `scripts/check-bench-delta.ps1` script compares two `critcmp` JSON outputs and enforces a ≤20% regression on tracked benches.

## 5. How to run tests

From `crates/synapse-mcp/Cargo.toml` and the impplan's per-PR contract:

```powershell
# Full release + dev compile gate
cargo build --release --workspace
cargo build --workspace --all-targets

# Lint gate (workspace + tests + benches; zero warnings expected)
cargo clippy --workspace --all-targets -- -D warnings

# Run all tests (unit + integration + proptest + snapshot)
cargo test --workspace

# Targeted test binary
cargo test -p synapse-mcp --test m3_reflex_register_tool

# Run a specific bench (criterion default mode)
cargo bench -p synapse-reflex --bench reflex_tick_jitter_idle

# Compare two bench runs against the 20% gate
./scripts/check-bench-delta.ps1 -BaselinePath bench_main.json -CandidatePath bench_pr.json
```

**Important constraints** (per `AGENTS.md` and `docs/impplan/00_methodology.md`):

- All agent commits must include `[skip ci]`. GitHub Actions are not the shipping gate.
- Tests are **supporting evidence**, not FSV. Manual FSV (operator-driven source-of-truth readback before and after) is required for the configured Windows host.
- Many tests assume Windows (UIA / ViGEm / WinRT OCR / WASAPI loopback). Non-Windows runs skip those via `cfg(windows)` guards and fall back to the `software_non_windows`-style stubs.
- `synapse-mcp/tests/m2_notepad_type_save.rs` launches a real Notepad process and writes to a real path; clean up `%LOCALAPPDATA%/synapse/replays` between runs if needed.

## 6. Test fixtures

`crates/synapse-test-utils/`:

| Symbol | Purpose |
|---|---|
| `StdioMcpClient` (`stdio_mcp_client.rs`) | Spawns the workspace-built `synapse-mcp` binary with optional env overrides; drives `initialize` + `notifications/initialized`; exposes `call_tool(name, params)` and `tools_list()`; kills the child on drop |
| `launch_notepad` (`fixtures.rs`, `cfg(windows)`) | Spawns Notepad; returns a guard that kills it on drop |
| `wait_for_window_title_regex(regex, timeout)` | Polls UIA top-level windows |
| `notepad_process_ids` | Enumerates current Notepad PIDs |

`tests/fixtures/audio/`:

| File | Sample rate | Channels | Purpose |
|---|---|---|---|
| `hello_world_5s.wav` | 16 kHz | 1 | English speech for `audio_transcribe` |
| `loud_transient_1s.wav` | 48 kHz | 1 | Transient + RMS detector |
| `pan_minus60_0_plus60.wav` | 48 kHz | 2 | `estimate_direction` azimuth sweep |

Synthesis recipe documented in `tests/fixtures/audio/README.md`.

## 7. Key environment variables used during tests

| Env var | Effect |
|---|---|
| `SYNAPSE_MCP_SYNTHETIC_FIXTURE=notepad` | M1State sources a synthetic Notepad observation so `observe`/`find` are deterministic without a real Notepad window |
| `SYNAPSE_MCP_FORCE_NO_PERCEPTION=1` | `observe` returns `OBSERVE_NO_PERCEPTION_AVAILABLE` |
| `SYNAPSE_MCP_FORCE_OBSERVE_INTERNAL=1` | `observe` returns `OBSERVE_INTERNAL` |
| `SYNAPSE_MCP_RECORDING_BACKEND=1` | Routes all M2 emits to a `RecordingBackend` (used by every M2 integration test) |
| `SYNAPSE_MCP_FORCE_PANIC_DURING_ACT=1` | (debug builds only) `act_press` panics inside `block_in_place` — used by `safety_panic_hook.rs` |
| `SYNAPSE_REFLEX_FORCE_DEGRADED=1` | Scheduler marks every tick `degraded=true` |
| `SYNAPSE_STORAGE_PRESSURE_FREE_BYTES_SAMPLE=<bytes>` | One synthetic free-byte sample on `Db::open` |
| `SYNAPSE_HTTP_SSE_MANUAL=1` | Exposes `POST /events` and `GET /events/stats` for tests |
| `SYNAPSE_HTTP_SESSION_IDLE_TIMEOUT_SECS=<n>` | Overrides the 30-minute default in HTTP tests |

## 8. Coverage metrics

Not determined from source — no coverage tooling configuration is present in the repository (no `tarpaulin.toml`, no `grcov`-style scripts, no `coverage` directory in CI artifacts). Manual FSV is the shipping gate; `cargo test --workspace` is the supporting-evidence completeness signal.

## 9. What is NOT covered

- **Cross-platform CI.** Non-Windows hosts run only the OS-agnostic subset; full coverage requires the configured Windows 11 host with ViGEmBus installed, DX11-capable GPU, working WASAPI default device, and a Whisper-tiny ONNX file at the default model path.
- **Synthetic CDP attach.** The `synapse-a11y::probe_chromium_cdp` path has unit tests but no integration test against a live Chromium instance.
- **Fuzz testing.** `proptest` covers schema round-trips and ordering invariants but there is no `cargo-fuzz` corpus.
- **End-to-end HTTP transport tests.** The `http/*` modules carry inline `#[cfg(test)]` tests (auth, sessions, SSE frames), but there are no integration tests that spawn `synapse-mcp --mode http` and drive the streamable HTTP wire. `cli_modes.rs` exercises the flag parsing only.
