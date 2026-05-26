# Synapse Systemspec — Bundled Reference

> Auto-generated 2026-05-26 by `docs/systemspec/bundle.ps1`. Source: the 16 individual `docs/systemspec/*.md` files, concatenated in order. In-bundle cross-references between systemspec files are rewritten to anchors; references to files outside the bundle (impplan, computergames, adr, source code) keep their original paths.
>
> Re-run the script after editing any source file so the bundle stays in sync. The individual files remain the authoritative copies.

## Bundle table of contents

- [Index (README)](#index)
- [01 — System Overview](#file-01)
- [02 — Source Code Map](#file-02)
- [03 — Configuration](#file-03)
- [04 — Storage Layer](#file-04)
- [05 — Core Types and Errors](#file-05)
- [06 — MCP Service and Transports](#file-06)
- [07 — Reflex Runtime](#file-07)
- [08 — Action Subsystem](#file-08)
- [09 — Perception and Capture](#file-09)
- [10 — Audio and Models](#file-10)
- [11 — Profiles, HID, Telemetry, Test Utils](#file-11)
- [12 — Milestones and Roadmap](#file-12)
- [13 — MCP Tool Reference](#file-13)
- [14 — Test Suite](#file-14)
- [15 — Verification Report](#file-15)


---

<a id="index"></a>

> Source: `docs/systemspec/README.md`

# Synapse Systemspec

Comprehensive technical reference for the Synapse MCP server, produced by reading the source. Every claim in these documents is derived from `crates/` source files cited inline; no aspirational behavior is documented.

## Read order

1. [01_system_overview.md](#file-01) — architecture map, tech stack, 30-tool inventory, error hierarchy
2. [02_source_code_map.md](#file-02) — file tree with per-file descriptions, dep graph, entry-point traces
3. [03_configuration.md](#file-03) — CLI flags, env vars, validation, all numeric defaults
4. [04_storage_layer.md](#file-04) — RocksDB schema (11 CFs), schema sentinel, TTL filter, GC, disk pressure
5. [05_core_types_and_errors.md](#file-05) — `synapse-core` wire types + 87 error codes
6. [06_mcp_service_and_transports.md](#file-06) — `SynapseService`, stdio + HTTP routers, Bearer/Origin/Session middleware, SSE bridge
7. [07_reflex_runtime.md](#file-07) — EventBus, scheduler, the 5 reflex kinds, audit persistence
8. [08_action_subsystem.md](#file-08) — emitter actor, backends, rate limits, hotkey, curves/dynamics
9. [09_perception_and_capture.md](#file-09) — frame capture, UIA, perception assembler, OCR
10. [10_audio_and_models.md](#file-10) — WASAPI loopback, Whisper-tiny STT, ONNX model loader
11. [11_profiles_hid_telemetry.md](#file-11) — TOML profile loader, HID stub, tracing + metrics, test utils
12. [12_milestones_and_roadmap.md](#file-12) — milestone state, ADRs, doctrine, open decisions
13. [13_mcp_tool_reference.md](#file-13) — every tool's params, defaults, ranges, side effects, errors
14. [14_test_suite.md](#file-14) — test inventory by crate, run commands, fixtures
15. [15_verification_report.md](#file-15) — health snapshot, metrics, schema version, constants

## Authority

- `AGENTS.md` and `docs/impplan/00_methodology.md` are the operating doctrine.
  Manual FSV is the shipping gate; this systemspec is descriptive only.
  Missing configured-host prerequisites are acquisition/setup work: agents use
  Synapse/local control as the operator-equivalent host control surface to make
  reversible local prerequisites real, then read the physical source of truth.
  Do not stop at "missing"; if the operator could do it from this computer,
  the agent must do it through Synapse/local host workflows and inspect the SoT.
  Browser downloads, GUI installers, Device Manager checks, package-manager
  installs, model/file generation, firmware flashing, app launching, and UI
  inspection are agent-owned work when reversible on this host.
- For the contract-level PRD, see `docs/computergames/` (numbered 00–17).
- For the per-milestone work-item ledger, see `docs/impplan/` (numbered 00–07).


---

<a id="file-01"></a>

> Source: `docs/systemspec/01_system_overview.md`

# 01 — System Overview

Source files covered:
- `Cargo.toml`
- `README.md`
- `AGENTS.md`
- `CHANGELOG.md`
- `crates/synapse-mcp/src/main.rs`
- `crates/synapse-mcp/src/server.rs`
- `crates/synapse-mcp/src/m1.rs`
- `crates/synapse-mcp/src/m2.rs`
- `crates/synapse-mcp/src/m3.rs`
- `crates/synapse-mcp/src/http/mod.rs`
- `crates/synapse-mcp/src/http/transport.rs`
- `crates/synapse-mcp/src/http/auth.rs`
- `crates/synapse-mcp/src/http/session.rs`
- `crates/synapse-mcp/src/http/sse.rs`
- `crates/synapse-mcp/src/safety.rs`
- `crates/synapse-core/src/lib.rs`
- `crates/synapse-core/src/error_codes.rs`
- `crates/synapse-core/src/defaults.rs`
- `crates/synapse-core/src/retention.rs`
- `crates/synapse-storage/src/lib.rs`
- `crates/synapse-storage/src/cf.rs`
- `crates/synapse-reflex/src/lib.rs`
- `crates/synapse-action/src/lib.rs`
- `crates/synapse-perception/src/lib.rs`
- `crates/synapse-a11y/src/lib.rs`
- `crates/synapse-audio/src/lib.rs`
- `crates/synapse-capture/src/lib.rs`
- `crates/synapse-profiles/src/lib.rs`
- `crates/synapse-telemetry/src/lib.rs`
- `crates/synapse-models/src/lib.rs`
- `crates/synapse-hid-host/src/lib.rs`
- `crates/synapse-overlay/src/main.rs`

## 1. What Synapse is

Synapse is a Rust [Model Context Protocol](https://modelcontextprotocol.io) (MCP) server that exposes a Windows 11/10 host's local desktop and game state as low-token structured JSON, accepts high-level action intents (click, type, aim, press, drag, combo), and runs sub-frame reflex controllers so model latency never costs a frame. The shipping binary is `synapse-mcp` (`crates/synapse-mcp/src/main.rs`); MCP clients (Claude Desktop/Code, Codex, custom runners) connect over **stdio** (newline-delimited JSON-RPC) or **streamable HTTP** (loopback by default, bearer-auth required).

The repository operates under the doctrine in `AGENTS.md`: manual Full State Verification (FSV) on the configured Windows host is the shipping gate; GitHub Actions, CI, scripts, tests, and benches are supporting evidence only; missing configured-host prerequisites are acquisition/setup work where the agent figures out where the thing must come from, where it must physically appear, uses Synapse/local control as the operator-equivalent host control surface to make it happen when reversible local steps exist, treats browser downloads, GUI installers, Device Manager checks, package-manager installs, model/file generation, firmware flashing, app launching, and UI inspection as agent-owned work on this host, does not stop at "missing" when the operator could do it from this computer, and verifies the real source of truth; and agent commits include `[skip ci]`.

## 2. Architecture map

| Process / surface | Port / transport | Technology | Purpose |
|---|---|---|---|
| `synapse-mcp` (stdio) | newline-delimited JSON-RPC on stdin/stdout | `rmcp` v1.7 | Default MCP transport for local agent clients |
| `synapse-mcp` (http) | `--bind` (default `127.0.0.1:7700`) HTTP+SSE | `axum` v0.8 + `rmcp` streamable_http_server | Remote/multi-client MCP transport, loopback-only unless `--allow-non-loopback`; `Bearer` token required |
| HTTP `/health` | GET | axum route | JSON health probe (no MCP session needed); see `crates/synapse-mcp/src/http/transport.rs::health` |
| HTTP `/events` | GET (SSE) / POST | axum + `Sse` | Server-Sent Events bridge over the reflex `EventBus`; POST publishes events only when `SYNAPSE_HTTP_SSE_MANUAL=1` is set (`crates/synapse-mcp/src/http/sse.rs::publish`) |
| HTTP `/events/stats` | GET | axum route | Per-subscription ring stats; gated by the same manual env var |
| HTTP `/mcp` | POST/GET/DELETE | `rmcp::transport::streamable_http_server` | Streamable HTTP MCP body with `Mcp-Session-Id` headers (`crates/synapse-mcp/src/http/session.rs`) |
| RocksDB | local directory, default `%LOCALAPPDATA%/synapse/db` | `rocksdb` v0.24 (lz4 + zstd) | Persistent storage of events, observations, sessions, reflex audit, profiles, model cache, OCR cache, telemetry, action log, process history, generic kv (11 column families) |
| Profile dir | `crates/synapse-profiles/src/parser.rs::bundled_profiles_dir`, override via `SYNAPSE_PROFILE_DIR` | TOML | Per-app/per-game profiles, hot-reloaded by `notify` watcher |
| Log dir | `%LOCALAPPDATA%/synapse/logs` (Windows) | `tracing` JSON files + `tracing-appender` daily rolling | All structured logs with periodic GC (`crates/synapse-telemetry/src/lib.rs`) |
| Audio loopback | WASAPI loopback on default render device | `wasapi` v0.23 | Ring buffer + STT (Whisper tiny) for `audio_tail` / `audio_transcribe` tools |
| Capture | DXGI duplication or Windows.Graphics.Capture | `windows-capture` v2.0 + `windows` v0.62 | Zero-copy GPU frame surface for OCR/CNN paths |
| ViGEm virtual pad | ViGEmBus driver, user-space client | `vigem-client` v0.1.4 (X360 default, optional DS4) | Virtual Xbox/DualShock controller for `act_pad` |
| Pi Pico HID gateway | USB serial, RP2040 firmware (M4) | `serialport` v4.9 + `crc16` | Hardware HID host surface; `synapse-hid-host` implements serial discovery, connect/IDENTIFY, framing, pipelined send, and reconnect paths. See `docs/computergames/09_hardware_hid_gateway.md` |
| `synapse-overlay` binary | currently a placeholder (`crates/synapse-overlay/src/main.rs` is 1 line) | n/a | Future debug overlay for M5 production polish |

## 3. Technology stack

| Layer | Technology | Version constraint (from `Cargo.toml`) |
|---|---|---|
| Language | Rust | edition 2024, MSRV `1.95` |
| Async runtime | `tokio` (full features) | `1.52.3` |
| Cancellation | `tokio-util` | `0.7.18` |
| MCP framework | `rmcp` (server + transport-io + transport-streamable-http-server + macros + schemars) | `1.7.0` |
| HTTP server | `axum` | `0.8.9` |
| HTTP plumbing | `hyper`, `tower` | `1.9.0`, `0.5.3` |
| Serialization | `serde`, `serde_json`, `toml` | `1.0.228`, `1.0.150`, `1.1.2` |
| Schema | `schemars` (with chrono04, derive) | `1.2.1` |
| Errors | `thiserror`, `anyhow` | `2.0.18`, `1.0.102` |
| Logging | `tracing`, `tracing-subscriber` (env-filter + json), `tracing-appender` | `0.1.44`, `0.3.23`, `0.2.5` |
| Metrics | `metrics`, `metrics-exporter-prometheus` | `0.24.6`, `0.18.3` |
| Tracing/OTLP | `opentelemetry`, `opentelemetry-otlp` | `0.32.0`, `0.32.0` |
| RocksDB | `rocksdb` (lz4 + zstd, multi-threaded-cf) | `0.24.0` |
| Filesystem watcher | `notify` | `9.0.0-rc.4` |
| Windows API | `windows` (broad UIA / D3D11 / DXGI / OCR / Input / HiDPI feature set) | `0.62.2` |
| Frame capture | `windows-capture` | `2.0.0` |
| UI Automation | `uiautomation` (pattern + control + event) | `0.25.0` |
| Chromium DevTools | `chromiumoxide` | `0.9.1` |
| Audio | `wasapi` | `0.23.0` |
| Software input | `enigo` (no default features) | `0.6.1` |
| Virtual controller | `vigem-client` (with `unstable_ds4` from `synapse-action`) | `0.1.4` |
| Serial port | `serialport` | `4.9.0` |
| CRC | `crc16` | `0.4.0` |
| ONNX Runtime | `ort` (optional via `synapse-models` features `ort`/`cuda`/`directml`) | `2.0.0-rc.12` |
| Hashing / constant-time | `sha2`, `subtle` | `0.11.0`, `2.6.1` |
| CLI | `clap` (derive + env) | `4.6.1` |
| Time | `chrono` (serde) | `0.4.44` |
| IDs | `uuid` (v4, v7, serde) | `1.23.1` |
| Regex | `regex` | `1.12.3` |
| Locking helpers | `arc-swap`, `crossbeam`, `fs2` | `1.9.1`, `0.8.4`, `0.4.3` |
| Property tests | `proptest` | `1.11.0` |
| Bench | `criterion` | `0.8.2` |
| Snapshots | `insta` (json) | `1.47.2` |
| Tempfiles | `tempfile` | `3.27.0` |
| Mocking | `mockall` | `0.14.0` |

Workspace-wide lints (`Cargo.toml [workspace.lints]`): `unsafe_code = forbid` at root (relaxed to `allow` in crates with FFI: `synapse-capture`, `synapse-a11y`, `synapse-action`, `synapse-audio`, `synapse-hid-host`); clippy `all = deny`, `pedantic`/`nursery = warn`, `unwrap_used`/`expect_used = deny`.

Release profile: `opt-level=3`, `lto="thin"`, `codegen-units=16`, `panic="abort"`, `strip=true` (`Cargo.toml [profile.release]`). A `release-max` profile inherits release with `lto="fat"` and `codegen-units=1`.

## 4. Public MCP tool surface (live)

All 30 live tools live in `crates/synapse-mcp/src/server.rs` (declared via `#[tool_router]`). Grouped by milestone:

### 4.1 M1 — perception (6 tools)

| Tool | Description | Source |
|---|---|---|
| `health` | Returns server version, build SHA, uptime, subsystem health for storage/reflex/profiles/audio/http | `server.rs::health` |
| `observe` | Returns the structured Observation (foreground, focused element, A11y tree, detected entities, HUD, audio, recent events, clipboard, fs, diagnostics) | `server.rs::observe` |
| `find` | Search visible UIA elements + detected entities by role / name substring / automation id / free-text query | `server.rs::find` |
| `read_text` | OCR a screen region or visible element using the active backend | `server.rs::read_text` |
| `set_capture_target` | Switch active capture target between Primary / Monitor / Window / Element-window | `server.rs::set_capture_target` |
| `set_perception_mode` | Override perception mode (auto, a11y_only, pixel_only, hybrid) | `server.rs::set_perception_mode` |

### 4.2 M2 — action (9 tools)

| Tool | Description | Source |
|---|---|---|
| `act_click` | Click coord or UIA element (1–3 clicks, modifiers (not yet wired), invoke-pattern, natural/instant/linear/ease curve) | `server.rs::act_click`, `m2/click/*` |
| `act_type` | Type Unicode text; burst / linear / natural dynamics; optional element focus + press_enter_after | `server.rs::act_type`, `m2/type_text.rs` |
| `act_press` | Press a single key or ordered chord with hold duration | `server.rs::act_press`, `m2/press/*` |
| `act_aim` | Move pointer to point / element / track-id (snap / flick / natural / track) | `server.rs::act_aim`, `m2/aim.rs` |
| `act_drag` | Drag between point/element points with chosen button and curve | `server.rs::act_drag`, `m2/drag.rs` |
| `act_scroll` | Horizontal/vertical mouse-wheel scroll, optional smoothing | `server.rs::act_scroll`, `m2/scroll.rs` |
| `act_pad` | Apply a virtual gamepad report on ViGEm pad with hold_ms and optional auto-neutral | `server.rs::act_pad`, `m2/pad.rs` |
| `act_clipboard` | Read / write / clear the Win32 clipboard (text or unicode format) | `server.rs::act_clipboard`, `m2/clipboard.rs` |
| `release_all` | Synchronously release all held keys, mouse buttons, pad axes | `server.rs::release_all`, `m2/release_all.rs` |

### 4.3 M3 — reflex / events / profiles / replay / audio / storage diagnostics (15 tools)

| Tool | Description | Source |
|---|---|---|
| `subscribe` | Open a buffered SSE / push-stream subscription to event-bus events with kind list + optional `EventFilter` (`buffer_size` is presently hard-pinned to 4096) | `server.rs::subscribe`, `m3/subscribe.rs` |
| `subscribe_cancel` | Drop a subscription by id | `server.rs::subscribe_cancel`, `m3/subscribe.rs` |
| `reflex_register` | Register an `AimTrack` / `HoldMove` / `HoldButton` / `Combo` / `OnEvent` reflex | `server.rs::reflex_register`, `m3/reflex.rs` |
| `reflex_cancel` | Cancel a registered reflex by id | `server.rs::reflex_cancel`, `m3/reflex.rs` |
| `reflex_list` | List active reflexes (and optionally terminal ones reconstructed from `CF_REFLEX_AUDIT`) | `server.rs::reflex_list`, `m3/reflex.rs` |
| `reflex_history` | Return persisted `StoredReflexAudit` rows (newest-first), optionally for one reflex id, capped at 1000 | `server.rs::reflex_history`, `m3/reflex.rs` |
| `profile_list` | List loaded profiles + active id | `server.rs::profile_list`, `m3/profile.rs` |
| `profile_activate` | Activate a known profile id (use_scope=unknown profiles require `--allow-unknown-profile`) | `server.rs::profile_activate`, `m3/profile.rs` |
| `replay_record` | Stream observations and/or events to a JSONL file under `%LOCALAPPDATA%/synapse/replays` | `server.rs::replay_record`, `m3/replay.rs` |
| `audio_tail` | Return the most-recent loopback audio tail as PCM s16le bytes (max 5 s; `synapse_audio::MAX_RING_SECONDS`) | `server.rs::audio_tail`, `m3/audio.rs` |
| `audio_transcribe` | Transcribe the loopback tail via Whisper-tiny (language pinned to `"en"`) | `server.rs::audio_transcribe`, `m3/audio.rs` |
| `storage_inspect` | Return per-CF row counts and byte sizes from RocksDB for the operator-visible CFs | `server.rs::storage_inspect`, `m3/storage.rs` |
| `storage_put_probe_rows` | Insert bounded probe rows into a chosen CF so manual FSV can trigger storage writes, then separately read the RocksDB/log SoT | `server.rs::storage_put_probe_rows`, `m3/storage.rs` |
| `storage_gc_once` | Run one synchronous GC pass and return the per-CF before/after sizes | `server.rs::storage_gc_once`, `m3/storage.rs` |
| `storage_pressure_sample` | Apply one synthetic free-byte sample to drive the disk-pressure responder | `server.rs::storage_pressure_sample`, `m3/storage.rs` |

Full parameter/return tables: [13_mcp_tool_reference.md](#file-13).

### 4.4 PRD-planned tools NOT live in this build

`docs/computergames/05_mcp_tool_surface.md` defines a 30-tool surface cap for the agent-facing tools. Synapse's live build extends this with four operator-only `storage_*` diagnostics added during M3. The following PRD-planned entries remain unimplemented: `describe` (M5 VLM), `read_hud` (M4 HUD pipeline), `act_combo`, `act_run_shell`, `act_launch` (all M4).

## 5. Entry points

| Binary | Source | Purpose |
|---|---|---|
| `synapse-mcp` | `crates/synapse-mcp/src/main.rs` (clap CLI, `tokio::main`) | The MCP server; `--mode stdio` (default) or `--mode http` |
| `synapse-overlay` | `crates/synapse-overlay/src/main.rs` | Placeholder binary for the future debug overlay; current source is `fn main() {}` |
| (no other workspace binaries) | — | All other crates are libraries |

Default workspace members for build (`Cargo.toml`): `crates/synapse-mcp`, `crates/synapse-overlay`. All other crates are pulled in via the `[workspace.members]` list.

CLI flags on `synapse-mcp` (parsed in `main.rs::Cli`, full table: [03_configuration.md](#file-03)):

```
--mode <stdio|http>             [env: SYNAPSE_MODE]            default: stdio
--bind <ADDR:PORT>              [env: SYNAPSE_BIND]            default: 127.0.0.1:7700
--allow-non-loopback            [env: SYNAPSE_ALLOW_NON_LOOPBACK]
--db <PATH>                     [env: SYNAPSE_DB]
--profile-dir <PATH>            [env: SYNAPSE_PROFILE_DIR]
--log-level <LEVEL>             [env: SYNAPSE_LOG_LEVEL]       default: info
--reflex-disabled               [env: SYNAPSE_REFLEX_DISABLED]
--enable-audio                  [env: SYNAPSE_ENABLE_AUDIO]
--allow-unknown-profile         [env: SYNAPSE_ALLOW_UNKNOWN_PROFILE]
--allowed-permissions <LIST>    [env: SYNAPSE_MCP_ALLOWED_PERMISSIONS]
--reflex-force-degraded         [env: SYNAPSE_REFLEX_FORCE_DEGRADED]
--storage-pressure-free-bytes-sample <BYTES>
                                [env: SYNAPSE_STORAGE_PRESSURE_FREE_BYTES_SAMPLE]
--max-subscriptions <COUNT>     [env: SYNAPSE_MAX_SUBSCRIPTIONS]
                                default: synapse_reflex::DEFAULT_MAX_SUBSCRIPTIONS_NONZERO
--hardware-hid <PORT_OR_AUTO>   [env: SYNAPSE_HARDWARE_HID]
```

## 6. Runtime directory layout

| Path | Created by | Contents |
|---|---|---|
| `%LOCALAPPDATA%/synapse/db/` | `crates/synapse-mcp/src/m3.rs::default_db_path` + `crates/synapse-storage/src/lib.rs::Db::open` | RocksDB column families (events, observations, profiles, model_cache, sessions, reflex_audit, ocr_cache, telemetry, action_log, process_history, kv) plus a `__schema_version` sentinel key (big-endian `u32`, current value `1` from `synapse_core::SCHEMA_VERSION`) |
| `%LOCALAPPDATA%/synapse/logs/` | `crates/synapse-telemetry/src/lib.rs::default_log_dir` | JSON tracing files (`synapse.log` daily rotated); GC keeps 7 days / 500 MiB ceiling, configurable via `SYNAPSE_LOG_GC_INTERVAL_S` |
| `%LOCALAPPDATA%/synapse/replays/` | `crates/synapse-mcp/src/m3/permissions.rs::replay_root` | JSONL files written by `replay_record` (default name `replay-<uuid-v7>.jsonl`) |
| `%APPDATA%/synapse/token.txt` | operator-provisioned | If present, overrides `SYNAPSE_BEARER_TOKEN` for the HTTP transport (`crates/synapse-mcp/src/http/auth.rs::token_file_path`) |
| Bundled profiles | resolved by `crates/synapse-profiles/src/parser.rs::bundled_profiles_dir`, overridable with `SYNAPSE_PROFILE_DIR` | Per-app/per-game TOML profiles |

## 7. Storage tier classification

| Tier | Where it lives | Sample contents | Recovery on loss |
|---|---|---|---|
| **Sacred** (cannot be regenerated) | Profile TOML (`SYNAPSE_PROFILE_DIR`), `%APPDATA%/synapse/token.txt` | Operator-authored profiles and the HTTP bearer token | Must be backed up by operator; daemon will refuse HTTP without token |
| **Regenerable** (rebuilt from sensors / disk) | RocksDB (`CF_EVENTS`, `CF_OBSERVATIONS`, `CF_REFLEX_AUDIT`, `CF_TELEMETRY`, `CF_ACTION_LOG`, `CF_PROCESS_HISTORY`, `CF_PROFILES`, `CF_MODEL_CACHE`, `CF_OCR_CACHE`, `CF_SESSIONS`, `CF_KV`) | Persistent runtime state, audit trails, model cache | Deleting `%LOCALAPPDATA%/synapse/db` is acceptable pre-v1 (no migration shims; schema bumps wipe-and-rebuild — see `docs/computergames/README.md` §"Authoring rules") |
| **Ephemeral** (process-lifetime) | `M1State`, `M2State`, `M3State`, `SseState`, audio ring buffer, reflex scheduler ticks | Last observed foreground, action emitter held bitset, SSE per-subscriber ring | Recreated on every daemon start |

## 8. Error code hierarchy

All errors carry `SCREAMING_SNAKE_CASE` codes defined as `pub const &str` in `crates/synapse-core/src/error_codes.rs`. Tools return them through `rmcp::ErrorData::new(ErrorCode(-32099), message, data={"code": <code>})` (constructor: `crates/synapse-mcp/src/m1.rs::mcp_error`). Categories (matching PRD §8.x):

| Category | Examples | Source-of-truth |
|---|---|---|
| Perception (PRD 8.1) | `OBSERVE_NO_PERCEPTION_AVAILABLE`, `OBSERVE_INTERNAL`, `CAPTURE_GRAPHICS_API_UNSUPPORTED`, `CAPTURE_TARGET_LOST`, `CAPTURE_NO_DIRTY_REGIONS`, `A11Y_NOT_AVAILABLE`, `A11Y_ELEMENT_STALE`, `A11Y_NO_FOREGROUND`, `A11Y_CDP_UNREACHABLE`, `DETECTION_MODEL_NOT_LOADED`, `DETECTION_MODEL_INFER_FAILED`, `DETECTION_NO_FRAME`, `OCR_NO_TEXT`, `OCR_BACKEND_UNAVAILABLE`, `HUD_NO_ACTIVE_PROFILE`, `HUD_FIELD_NOT_DEFINED`, `HUD_EXTRACTION_FAILED`, `AUDIO_DEVICE_LOST`, `AUDIO_LOOPBACK_INIT_FAILED`, `AUDIO_STT_MODEL_NOT_LOADED` | `error_codes.rs` lines 2–21 |
| Action (PRD 8.2) | `ACTION_QUEUE_FULL`, `ACTION_RATE_LIMITED`, `ACTION_BACKEND_UNAVAILABLE`, `ACTION_TARGET_INVALID`, `ACTION_HOLD_EXCEEDED_MAX`, `ACTION_HID_PORT_DISCONNECTED`, `ACTION_VIGEM_NOT_INSTALLED`, `ACTION_VIGEM_PLUGIN_FAILED`, `ACTION_ELEMENT_NOT_RESOLVED`, `ACTION_FOREGROUND_LOST`, `ACTION_UNSUPPORTED_KEY`, `ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT`, `STUCK_KEY_AUTO_RELEASED`, `SAFETY_RELEASE_ALL_FIRED`, `SAFETY_OPERATOR_HOTKEY_FIRED` | lines 24–38 |
| Reflex (PRD 8.3) | `REFLEX_CAP_REACHED`, `REFLEX_KIND_INVALID`, `REFLEX_PARAMS_INVALID`, `REFLEX_TARGET_INVALID`, `REFLEX_FILTER_INVALID`, `REFLEX_PRIORITY_INVALID`, `REFLEX_TICK_LATE`, `REFLEX_TRACK_LOST`, `REFLEX_STARVED`, `REFLEX_DISABLED_BY_OPERATOR`, `REFLEX_LIFETIME_EXPIRED`, `REFLEX_RECURSION_LIMIT`, `REFLEX_ACTION_PERMISSION_DENIED` | lines 41–53 |
| Profile / config (PRD 8.4) | `PROFILE_NOT_FOUND`, `PROFILE_PARSE_ERROR`, `PROFILE_VERSION_INCOMPATIBLE`, `PROFILE_KEYMAP_INVALID`, `PROFILE_HUD_REGION_INVALID`, `CAPTURE_TARGET_INVALID`, `PERCEPTION_MODE_INVALID` | lines 56–62 |
| MCP / session (PRD 8.5) | `SESSION_NOT_FOUND`, `SESSION_EXPIRED`, `SUBSCRIPTION_NOT_FOUND`, `SUBSCRIPTION_CAP_REACHED`, `TOOL_NOT_FOUND`, `TOOL_PARAMS_INVALID`, `TOOL_INTERNAL_ERROR`, `HTTP_BIND_NON_LOOPBACK_REFUSED`, `HTTP_TOKEN_INVALID`, `HTTP_ORIGIN_REFUSED`, `HTTP_SESSION_INVALID`, `REPLAY_TARGET_INVALID`, `REPLAY_FORMAT_INVALID` | lines 65–77 |
| Storage (PRD 8.6) | `STORAGE_OPEN_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_READ_FAILED`, `STORAGE_CORRUPTED`, `STORAGE_SCHEMA_MISMATCH`, `STORAGE_DISK_PRESSURE_LEVEL_1..4`, `STORAGE_CF_HARD_CAP_REACHED` | lines 80–89 |
| Models (PRD 8.7) | `MODEL_DOWNLOAD_FAILED`, `MODEL_HASH_MISMATCH`, `MODEL_LOAD_FAILED`, `MODEL_BACKEND_UNAVAILABLE` | lines 92–95 |
| Hardware HID (PRD 8.8) | `HID_PORT_NOT_FOUND`, `HID_PORT_OPEN_FAILED`, `HID_PROTOCOL_HANDSHAKE_FAILED`, `HID_FIRMWARE_VERSION_MISMATCH`, `HID_COMMAND_REJECTED`, `HID_LINK_TIMEOUT` | lines 98–103 |
| Safety (PRD 8.9) | `SAFETY_KILLSWITCH_ACTIVE`, `SAFETY_PROCESS_DENYLISTED`, `SAFETY_SHELL_DENIED_BY_POLICY`, `SAFETY_LAUNCH_DENIED_BY_POLICY`, `SAFETY_SECRET_REDACTED`, `SAFETY_PERMISSION_DENIED`, `SAFETY_PROFILE_ACTION_DENIED` | lines 106–112 |

Cross-crate `thiserror` enums that surface these codes:

| Enum | Source | Notes |
|---|---|---|
| `synapse_storage::StorageError` | `crates/synapse-storage/src/error.rs` | `OpenFailed`, `ReadFailed`, `WriteFailed`, `SchemaMismatch`, `EncodeJson`, `DecodeJson`, plus disk-pressure variants |
| `synapse_reflex::ReflexError` | `crates/synapse-reflex/src/error.rs` | `CapReached`, `KindInvalid`, `ParamsInvalid`, `TargetInvalid`, `FilterInvalid`, `PriorityInvalid`, `DisabledByOperator`, etc. |
| `synapse_action::ActionError` | `crates/synapse-action/src/error.rs` | `BackendUnavailable`, `RateLimited`, `QueueFull`, etc. |
| `synapse_profiles::ProfileError` / `ProfileLoadError` | `crates/synapse-profiles/src/error.rs` | Loader/IO/parse errors |
| `synapse_audio::AudioError` | `crates/synapse-audio/src/error.rs` | `LoopbackInitFailed`, `DeviceLost`, `SttModelNotLoaded` |
| `synapse_perception::PerceptionError` | `crates/synapse-perception/src/error.rs` | OCR + observe assembly errors |
| `synapse_capture::CaptureError` | `crates/synapse-capture/src/lib.rs` | `GraphicsApiUnsupported`, `TargetLost`, `TargetInvalid`, `NoDirtyRegions`, `ThreadFailed` |
| `synapse_models::ModelError` | `crates/synapse-models/src/lib.rs` | ONNX load / hash / inference failures |
| `synapse_a11y::A11yError` | `crates/synapse-a11y/src/lib.rs` | UIA / CDP / hook failures |
| `synapse_telemetry::TelemetryError` | `crates/synapse-telemetry/src/lib.rs` | `LogDirNotWritable`, `SubscriberInit`, `Gc` |
| `synapse_core::ElementIdParseError`, `EventFilterValidationError` | `crates/synapse-core/src/types.rs` | Public validation errors that bubble up as `TOOL_PARAMS_INVALID` |

## 9. Subsystem summaries

### 9.1 synapse-core (foundation)
Shared types, schema version, retention defaults, and the canonical error code constants. `crates/synapse-core/src/types.rs` (1567 LoC) defines every wire-level struct/enum: `Action`, `Observation`, `Event`, `EventFilter`, `Profile`, `ReflexRegistration`, `Stored*` persistence variants, `Health`, `SubsystemHealth`, etc. `defaults.rs` pins `SCHEMA_VERSION = 1` and the reference-host performance budgets. `retention.rs` declares the 11-CF retention policy (TTL + soft/hard cap MB) used by storage GC. `filter.rs` evaluates `EventFilter` / `DataPredicate` against an `Event`. See [05_core_types_and_errors.md](#file-05).

### 9.2 synapse-mcp (binary + transports)
Owns the `synapse-mcp` binary, the `SynapseService` `ServerHandler` (`crates/synapse-mcp/src/server.rs`), and three milestone-scoped modules:
- **m1**: perception parameter parsing, observation assembly orchestration, OCR `read_text`, find/search across A11y elements + entities, capture-target + perception-mode toggles. Reads the host via `synapse-perception`/`synapse-a11y`/`synapse-capture`.
- **m2**: action tool param/response structs and `act_*_with_handle` orchestrators that build `synapse_core::Action`s and dispatch them through an `ActionHandle`. Holds a `RELEASE_ALL_HANDLE` for the operator hotkey.
- **m3**: event subscription bridge (`SseState`), reflex tool wrappers (kind→`ScheduledReflex`), profile tool wrappers, replay recorder, audio tail/transcribe. Owns `M3State` (db_path, profile_dir, permission grants, lazy reflex/profile/audio runtimes).

Two transports: **stdio** (`run_stdio` in `main.rs`) and **HTTP** (`http::serve`); HTTP requires loopback unless `--allow-non-loopback`, enforces `Bearer` auth with constant-time SHA-256 comparison (`http/auth.rs`), enforces `Mcp-Session-Id` headers on `/mcp` (`http/session.rs`), and exposes the event bus over Server-Sent Events at `/events` (`http/sse.rs`). See [06_mcp_service_and_transports.md](#file-06).

### 9.3 synapse-storage (RocksDB)
Opens RocksDB with LZ4 base, ZSTD on `CF_OBSERVATIONS`/`CF_SESSIONS`, no-compression on `CF_MODEL_CACHE`, fixed-prefix slice transforms on the time-keyed CFs (`CF_EVENTS`, `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`). Schema version sentinel key `__schema_version` (big-endian `u32`) is checked on open and `STORAGE_SCHEMA_MISMATCH` is returned on mismatch. Periodic background tasks: 5-minute GC (`gc.rs`) and 30-second disk-pressure polling (`pressure.rs`, thresholds 2 GB / 1 GB / 500 MB / 200 MB). Writes are aggregated through a background `Batcher` task (`batch.rs`). JSON-only payload codecs (`codecs.rs`) — binary codecs forbidden by ADR-0001 / RUSTSEC-2025-0141 footnote in source. See [04_storage_layer.md](#file-04).

### 9.4 synapse-reflex (sub-frame runtime)
`ReflexRuntime` (`crates/synapse-reflex/src/lib.rs`) owns the scheduler, `EventBus`, and a `Db` handle for audit persistence. Reflex kinds: `AimTrack` (PID-style aim controller with EMA smoothing), `HoldMove` (held key set with optional re-assert), `HoldButton` (held mouse/pad button), `Combo` (timed step list), `OnEvent` (event-filtered action firing with recursion guard, default cap `MAX_ON_EVENT_FIRINGS_PER_TICK`). Scheduler runs at 1 ms tick (default), records `TickSample` with jitter, computes p99, exposes `degraded_latency()` and `recursion_clamps_total()` for `health`. Each register/cancel/disable/fire writes a `StoredReflexAudit` row into `CF_REFLEX_AUDIT`. Subscriber bus is bounded by `SUBSCRIBER_QUEUE_CAPACITY` with drop accounting. See [07_reflex_runtime.md](#file-07).

### 9.5 synapse-action (input emission)
The `ActionEmitter` actor (mpsc channel, `ACTION_QUEUE_CAPACITY=256`) consumes `Action` messages, applies token-bucket rate limits per backend (`SOFTWARE_RATE_LIMIT_PER_S`, `VIGEM_RATE_LIMIT_PER_S`), tracks held keys/buttons/pad state in a `BitSet`, and dispatches to one of: Software (`enigo`), ViGEm (`vigem-client`), Recording (in-memory buffer, used by tests), `HardwareBackend` (when `--hardware-hid <port|auto>` connects and identifies a Synapse Pico), or `HardwareUnavailable` (fail-closed `ACTION_BACKEND_UNAVAILABLE` when hardware HID is not enabled). Owns the operator panic hotkey (`Ctrl+Alt+Shift+P`) which fires a 50 ms-budgeted `ReleaseAll` (`crates/synapse-mcp/src/safety.rs`). Curve sampling, keystroke-dynamics samplers (`BIGRAMS`, `KeystrokeNaturalParams::FAST`), click-timing caches, clipboard read/write/clear, and per-action validation (`validate_action`, `MAX_DRAG_DISTANCE_PX`) all live here. See [08_action_subsystem.md](#file-08).

### 9.6 synapse-perception (observation assembly)
Aggregates inputs from `synapse-a11y` (UIA tree) and `synapse-capture` (frame sources, OCR) into an `Observation`. Owns the `ObservationAssembler` with per-slot include flags (`ObserveInclude`), auto perception-mode resolution (`auto_mode`, `auto_mode_with_a11y`), and `ObservationInput` (synthetic fixture or live OS data). Provides OCR with WinRT and CRNN backends (`OcrProvider`, `read_text`, `read_text_with_provider`). See [09_perception_and_capture.md](#file-09).

### 9.7 synapse-a11y (UIA + CDP)
Windows UI Automation tree walker (`uiautomation` crate), WinEvent hook for focus/mutation events (`subscribe_win_events`), Chromium DevTools Protocol attach via `chromiumoxide` for Chromium-family processes. Provides `current_foreground_context`, `focused_element`, `element_from_point`, `snapshot(root, depth)`, `find_by_name_and_pattern`, `re_resolve(ElementId)`, `expand_state_of`. Event utilities `coalesce_events` and `debounce_value_changes` deduplicate noisy UIA streams. See [09_perception_and_capture.md](#file-09).

### 9.8 synapse-capture (zero-copy GPU capture)
DXGI duplication and Windows.Graphics.Capture backends with a single `CaptureBackendPreference::Auto` resolver. Exposes `D3D11Texture` references via `SendablePtr<T>`, `CapturedFrame { texture, width, height, format, captured_at, frame_seq, dirty_region }`, `screen_region_to_software_bitmap`, DPI-awareness initialization (`init_process_dpi_awareness`, `is_per_monitor_v2_dpi_aware`), and screen↔window coord helpers. Capture loop runs on a high-priority thread and pushes frames into a bounded `crossbeam` channel (`CAPTURE_CHANNEL_CAPACITY=2`). See [09_perception_and_capture.md](#file-09).

### 9.9 synapse-audio (WASAPI + STT)
`AudioRuntime` (`crates/synapse-audio/src/lib.rs`) holds a ring buffer (default 5 s, max 5 s — `DEFAULT_RING_SECONDS = MAX_RING_SECONDS = 5`), a WASAPI loopback handle (started only when `--enable-audio` or `SYNAPSE_AUDIO_LOOPBACK=1`), optional event-emitting detectors (`detectors::DetectorProcessor`), and a Whisper-tiny STT engine (`WhisperTinyStt`). Provides `tail_seconds`, `transcribe_tail`, ring-format negotiation, direction estimate (azimuth + confidence). See [10_audio_and_models.md](#file-10).

### 9.10 synapse-profiles (TOML loader + live reload)
`ProfileRuntime::spawn` parses every `.toml` file in the profile directory at startup, registers a `notify` recursive watcher with a 200 ms debounce, and refreshes parsed profiles on filesystem events. Bundled profile dir is auto-discovered (`bundled_profiles_dir`); operator override is `SYNAPSE_PROFILE_DIR`. Resolves the active profile against the current foreground window via `resolve_active_profile`. Profiles with `use_scope=unknown` require explicit `--allow-unknown-profile`. See [11_profiles_hid_telemetry.md](#file-11) (or section in subsystem deep-dives).

### 9.11 synapse-telemetry (logs + metrics registry)
Owns the process-wide `tracing` subscriber: JSON file appender (daily rolling, lives in `%LOCALAPPDATA%/synapse/logs`), stderr console layer, and a background log-GC thread (default 6-hour interval, 7-day retention, 500 MiB ceiling, all overridable by `SYNAPSE_LOG_GC_INTERVAL_S`). Installs the panic-to-log hook (`install_panic_hook`). The `metrics` module declares 19 M3 metric specs (12 counters, 5 gauges, 2 histograms) with bounded label cardinality (`CARDINALITY_LIMIT = 1000`). Examples: `events_published_total`, `reflex_fires_total`, `reflex_tick_jitter_us`, `storage_disk_pressure_level`, `storage_cf_bytes`, `audio_loopback_underruns_total`, `sse_active_subscribers`. Spawn registers these via `metrics-exporter-prometheus` machinery.

### 9.12 synapse-models (ONNX runtime wrappers)
Holds `ModelDescriptor` (id, path, sha256, input_shape, class_map), `Detector` trait (`infer(frame, opts)`), `DetectionFrame::validate`. Wraps `ort` v2.0.0-rc.12 with optional `cuda` and `directml` features (`directml` is required by `synapse-audio` for the Whisper-tiny inference). Validates SHA-256 before loading; emits `MODEL_HASH_MISMATCH` on drift.

### 9.13 synapse-hid-host
USB-serial driver for the RP2040 HID gateway firmware (`firmware/pico-hid/`, excluded from the root workspace via `Cargo.toml::exclude`). The crate exposes serial discovery, `HidGateway::connect`, IDENTIFY parsing/version validation, CRC16 frame encode/decode, pipelined send, reconnect state, and HID error mapping. `synapse-mcp --hardware-hid <port|auto>` uses this crate to build the live `HardwareBackend`.

### 9.14 synapse-test-utils (shared test rig)
Provides `StdioMcpClient` for spawning `synapse-mcp` over stdio and driving JSON-RPC initialize → tool calls in integration tests, plus Notepad/audio fixtures (`launch_notepad`, `wait_for_window_title_regex`, `notepad_process_ids`).

### 9.15 synapse-overlay (M5 placeholder)
Currently `fn main() {}`. Will hold the debug overlay UI shipped at M5.

## 10. Milestone state (verified against `CHANGELOG.md` and `docs/impplan/`)

| Milestone | Status | Tag | Description |
|---|---|---|---|
| M0 — bootstrap | DONE | `v0.1.0-m0` (2026-05-23) | Rust workspace, `synapse-mcp` stdio, `health` tool, telemetry skeleton |
| M1 — perception MVP | DONE | `v0.1.0-m1` (2026-05-23) | `observe` + `find` + `read_text` + `set_capture_target` + `set_perception_mode` + a11y/capture wiring |
| M2 — action MVP | DONE | `v0.1.0-m2` (2026-05-24) | Nine action tools, Software + ViGEm + Recording backends, operator panic hotkey, `release_all` safety paths |
| M3 — reflex / MCP surface | DONE | `v0.1.0-m3` (2026-05-25, @ `97019ec`) | SSE bus, reflex runtime + 1 ms time-critical scheduler, RocksDB (11 CFs) + GC + 4-level disk pressure, profile loader + watcher (4 bundled), WASAPI loopback + Whisper-tiny STT, replay JSONL recorder, streamable HTTP/SSE transport with Bearer + Origin/Host + Mcp-Session-Id, 15 M3 tools (incl. four `storage_*` diagnostics) |
| M4 — hardware HID + first game | ACTIVE | — | RP2040 firmware (`firmware/pico-hid/`) + `synapse-hid-host` serial driver + Minecraft profile + `act_combo`/`act_run_shell`/`act_launch` |
| M5 — production polish | blocked by M4 | — | Installer, overlay, ≥10 profiles, VLM `describe`, soak |

## 11. What is NOT covered

- **Cross-platform support.** All capture, a11y, action, audio, hotkey, and HID paths are `#[cfg(windows)]`. Non-Windows builds compile (stubs return `ACTION_BACKEND_UNAVAILABLE`/equivalent), but no perception or action paths are wired.
- **Inner LLM / planner.** The agent (Claude / Codex / Cursor) is the brain; `synapse-mcp` is the body. There is no GOAP/MCTS/skill library inside this repo.
- **Goal storage.** RocksDB does not persist agent goals or plans; it only stores sensor/reflex/action traces.
- **Process manipulation / packet sniffing.** Out of scope per PRD §"Out of scope". `synapse-hid-host` and `synapse-action` only talk to OS input APIs or USB serial; no game RAM reads.
- **HTTPS / TLS.** The HTTP transport is loopback HTTP only by default; the Origin allow-list rejects non-`http://` Origins (`http/auth.rs::validate_origin`). For non-loopback binds with TLS termination, the operator is expected to front the daemon with a local reverse proxy.
- **Migration shims pre-v1.** Schema changes wipe-and-rebuild (`docs/computergames/README.md` §Authoring rules); there is no migration framework in `synapse-storage`.


---

<a id="file-02"></a>

> Source: `docs/systemspec/02_source_code_map.md`

# 02 — Source Code Map

Source files covered:
- `Cargo.toml`
- every `crates/*/Cargo.toml`
- every Rust file under `crates/`
- `scripts/`

## 1. Workspace layout

```
synapse/
├── Cargo.toml                      # Rust workspace root; lints, profiles, shared dep versions
├── Cargo.lock
├── AGENTS.md                       # Repository agent doctrine; manual FSV is the shipping gate
├── CHANGELOG.md                    # Release notes (M0/M2 tagged, M1 changelog entry implicit)
├── LICENSE-MIT / LICENSE-APACHE    # Dual license
├── README.md                       # Project README and MCP-client quickstart
├── deny.toml                       # cargo-deny config (advisory checks)
├── docs/                           # PRD + impplan + ADRs (see §4)
├── scripts/                        # PowerShell + Bash helper scripts (see §3)
├── tests/                          # Repo-level fixtures only (audio WAV samples)
│   └── fixtures/audio/             # Test-shared WAV fixtures
├── target/                         # Cargo build output (gitignored)
└── crates/                         # 15 workspace member crates (see §2)
```

`firmware/pico-hid/` is referenced in `Cargo.toml::exclude` and reserved for the M4 RP2040 firmware. It is not present in the current tree.

## 2. Crate tree (with per-file one-line descriptions)

### 2.1 `crates/synapse-mcp/` — the MCP server binary

```
crates/synapse-mcp/
├── Cargo.toml                      # Binary crate; depends on every other library crate
└── src/
    ├── main.rs                     # Process entrypoint, clap CLI, telemetry init, stdio/http dispatch
    ├── server.rs                   # SynapseService: ServerHandler + #[tool_router] declaring 30 MCP tools
    ├── safety.rs                   # Operator-hotkey handler that disables reflexes + fires release_all
    ├── http/
    │   ├── mod.rs                  # Re-exports http::serve entrypoint
    │   ├── transport.rs            # Axum router, TCP bind, loopback enforcement, /health, /events, /mcp
    │   ├── auth.rs                 # Bearer token (file or env) + constant-time compare + Host/Origin allowlist
    │   ├── session.rs              # Mcp-Session-Id enforcement, idle timeout, initialize-without-session shim
    │   └── sse.rs                  # SseState: subscription map, ring buffer, Last-Event-ID resume, publish/stats
    ├── m1.rs                       # M1State + ObserveParams/FindParams/ReadTextParams/SetCapture/SetPerceptionMode
    ├── m1/
    │   ├── ocr.rs                  # read_text_in_state implementation
    │   ├── search.rs               # element_match + entity_match scoring for `find`
    │   └── sources.rs              # platform_input + synthetic_notepad_input observation source
    ├── m2.rs                       # M2State (emitter actor handle + recording backend + snapshot handle)
    ├── m2/
    │   ├── aim.rs                  # act_aim params, response, MouseMove builder, snap/flick/natural durations
    │   ├── click.rs                # act_click orchestrator (delegates to schema/element/record submodules)
    │   ├── click/element.rs        # UIA invoke-pattern + coordinate-fallback element click
    │   ├── click/record.rs         # RecordingBackend dispatch path for click
    │   ├── click/schema.rs         # ActClickParams/ActClickResponse JSON schemas
    │   ├── click/tests.rs          # In-module click tests
    │   ├── clipboard.rs            # act_clipboard read/write/clear (text vs unicode)
    │   ├── drag.rs                 # act_drag params, MouseDrag builder, drag-distance enforcement
    │   ├── pad.rs                  # act_pad ActPadReport→GamepadReport, ViGEm dispatch, auto-neutral hold
    │   ├── press.rs                # act_press orchestrator (delegates to keys/live/record/schema/tests)
    │   ├── press/keys.rs           # Key name parsing (alpha, modifiers, function keys, scancode toggle)
    │   ├── press/live.rs           # Live ActionHandle dispatch path
    │   ├── press/record.rs         # RecordingBackend dispatch path
    │   ├── press/schema.rs         # ActPressParams/ActPressResponse, default hold_ms=33
    │   ├── press/tests.rs          # In-module press tests
    │   ├── release_all.rs          # release_all_with_handles: snapshot before, execute Action::ReleaseAll, ensure drained
    │   ├── scroll.rs               # act_scroll dy/dx, smooth scheduling (max 120 steps @ 30ms)
    │   └── type_text.rs            # act_type Burst/Linear/Natural dynamics + optional Enter
    ├── m3.rs                       # M3State, M3ServiceConfig, lazy reflex/profile/audio runtime init
    └── m3/
        ├── a11y_events.rs          # Bridges synapse-a11y AccessibleEvent stream into the reflex EventBus
        ├── audio.rs                # audio_tail + audio_transcribe tool implementations
        ├── permissions.rs          # PermissionGrants, replay path normalization, profile use-scope gate
        ├── profile.rs              # profile_list + profile_activate tool implementations
        ├── reflex.rs               # reflex_register/cancel/list/history tools + ScheduledReflex construction
        ├── replay.rs               # replay_record: observation + event JSONL writer
        ├── storage.rs              # storage_inspect/_put_probe_rows/_gc_once/_pressure_sample diagnostic tools
        ├── subscribe.rs            # subscribe + subscribe_cancel tool wrappers around SseState
        └── tests.rs                # M3-level integration scaffolding tests
```

### 2.2 `crates/synapse-core/` — shared types & error codes

```
crates/synapse-core/
├── Cargo.toml
└── src/
    ├── lib.rs                      # Re-exports public types + SCHEMA_VERSION
    ├── defaults.rs                 # SCHEMA_VERSION=1 + reference-host perf budgets
    ├── error_codes.rs              # 95 SCREAMING_SNAKE_CASE error-code pub const strs
    ├── filter.rs                   # EventFilter and DataPredicate matchers
    ├── retention.rs                # Per-CF TTL + soft/hard cap MB defaults (11 CFs)
    └── types.rs                    # All wire-level types (1567 LoC): Action, Observation, Event, Profile, Reflex*, Stored*, Health, etc.
```

### 2.3 `crates/synapse-storage/` — RocksDB wrapper

```
crates/synapse-storage/
├── Cargo.toml
├── build.rs                        # Build script (likely RocksDB feature gating)
└── src/
    ├── lib.rs                      # Db open/put_batch/flush/scan/compact, batcher spawn, pressure state
    ├── cf.rs                       # 11 column family name pub const + ALL_COLUMN_FAMILIES array
    ├── codecs.rs                   # encode_json/decode_json (only persisted codecs allowed; ADR-0001 / RUSTSEC-2025-0141)
    ├── error.rs                    # StorageError enum (OpenFailed, WriteFailed, ReadFailed, SchemaMismatch, encode/decode)
    ├── batch.rs                    # Background Batcher actor that aggregates put_batch into WriteBatchWithIndex flushes
    ├── batch_tests.rs              # Unit tests for batcher
    ├── compaction.rs               # install_ttl_filter for time-keyed CFs
    ├── compaction_tests.rs         # TTL compaction tests
    ├── gc.rs                       # Periodic GC pass (5-min interval), per-CF soft/hard cap eviction
    ├── gc_tests.rs                 # GC unit tests
    ├── pressure.rs                 # Disk-pressure poller (30-s interval), 4 thresholds (2GB/1GB/500MB/200MB)
    ├── pressure_tests.rs           # Disk-pressure unit tests
    └── open_tests.rs               # Open-path unit tests
```

### 2.4 `crates/synapse-reflex/` — sub-frame runtime

```
crates/synapse-reflex/
├── Cargo.toml
└── src/
    ├── lib.rs                      # ReflexRuntime: register/cancel/disable_all_by_operator + audit persistence
    ├── audit.rs                    # write_audit helper that persists StoredReflexAudit into CF_REFLEX_AUDIT
    ├── bus.rs                      # EventBus, SubscriberHandle, SUBSCRIBER_QUEUE_CAPACITY, drop accounting
    ├── conflict.rs                 # REFLEX_STARVED_KIND, STARVATION_AFTER conflict-resolution constants
    ├── error.rs                    # ReflexError enum with .code() mapping to error_codes::REFLEX_*
    ├── kinds/
    │   ├── mod.rs                  # Re-exports the five reflex-kind controllers
    │   ├── aim_track.rs            # AimTrackController: target, EMA smoothing, deadzone, axis lock, track-lost detection
    │   ├── combo.rs                # ComboController: step list with at_ms scheduling
    │   ├── hold_button.rs          # HoldButtonController: held mouse/pad button
    │   ├── hold_lifetime.rs        # HoldLifetimeContext for HoldMove/HoldButton; REFLEX_LIFETIME_EXPIRED_KIND
    │   ├── hold_move.rs            # HoldMoveController: held key set, re_assert option
    │   └── on_event.rs             # OnEvent: filter-matched action firing with recursion guard (MAX_ON_EVENT_FIRINGS_PER_TICK)
    ├── scheduler.rs                # ReflexScheduler: spawn, validate_reflexes, MAX_REFLEX_PRIORITY/MAX_SCHEDULED_REFLEXES
    ├── scheduler_combo.rs          # Combo-specific tick handling
    ├── scheduler_stateful.rs       # Stateful tick driver (held reflexes + lifetime + conflict resolution)
    ├── scheduler_stats.rs          # TickSample + p99_jitter_us
    ├── scheduler_tick.rs           # Per-tick driver loop
    └── scheduler_windows.rs        # Windows-specific scheduler (TIME_CRITICAL thread + MMCSS Pro Audio)
```

### 2.5 `crates/synapse-action/` — input emission

```
crates/synapse-action/
├── Cargo.toml
└── src/
    ├── lib.rs                      # Crate-level re-exports
    ├── click_timing.rs             # DoubleClickTiming + cached_double_click_timing (Win32 system metric)
    ├── clipboard.rs                # OpenClipboard/SetClipboardData/EmptyClipboard text + unicode helpers
    ├── curve.rs                    # sample_curve for AimCurve::Instant/Linear/EaseInOut/Bezier/Natural
    ├── dynamics.rs                 # KeystrokeEvent, ModifierMask, BIGRAMS, sample_typing_schedule
    ├── error.rs                    # ActionError with .code() mapping to error_codes::ACTION_*
    ├── handle.rs                   # ActionHandle (mpsc producer), ACTION_QUEUE_CAPACITY=256, RELEASE_ALL_HANDLE
    ├── hotkey.rs                   # OperatorHotkeyGuard, install_operator_hotkey (Ctrl+Alt+Shift+P low-level hook)
    ├── invoke.rs                   # UIA InvokePattern bridge for element-target clicks
    ├── invoke/dispatch.rs          # invoke_element implementation
    ├── invoke/resolver.rs          # CoordinateFallbackPlan (element bbox center fallback)
    ├── invoke/tests.rs             # InvokePattern unit tests
    ├── rate_limit.rs               # TokenBucket; SOFTWARE_RATE_LIMIT_PER_S, VIGEM_RATE_LIMIT_PER_S
    ├── safety.rs                   # install_panic_hook for action subsystem
    ├── validation.rs               # validate_action, MAX_DRAG_DISTANCE_PX
    ├── emitter.rs                  # ActionEmitter actor entrypoint, ActionStateSnapshot, EmitState
    ├── emitter/
    │   ├── backends.rs             # ActionEmitter::channel_with_backend / spawn_with_backend
    │   ├── dispatch.rs             # Per-action dispatch and held-state mutation
    │   ├── keyboard.rs             # Keyboard hold tracking + auto-release timers
    │   ├── lifecycle.rs            # run / run_with_shutdown_reason main loop
    │   ├── rate_limits.rs          # Per-backend rate-limit application
    │   ├── routing.rs              # Backend resolution (auto → software / vigem / hardware)
    │   ├── state.rs                # Snapshot exporter (snapshot_handle)
    │   └── tests/
    │       ├── mod.rs              # Test wiring
    │       ├── auto_release.rs     # Keyboard auto-release timer tests
    │       └── rate_limit.rs       # Token-bucket / rate-limit tests
    └── backend/
        ├── mod.rs                  # ActionBackend trait, ResolvedBackend, resolve_backend
        ├── mouse_coordinates.rs    # Screen→virtual desktop coord conversion
        ├── text_dispatch.rs        # Text-input dispatch (clipboard paste vs synthesized keystrokes)
        ├── hardware.rs             # HardwareBackend public facade
        ├── hardware/keyboard.rs    # Keyboard action helpers
        ├── hardware/keymap.rs      # Synapse key to USB HID Keyboard/Keypad usage mapping
        ├── hardware/keymap_tests.rs # HID usage mapping regression checks
        ├── hardware/mouse.rs       # Relative mouse/button/wheel command encoding
        ├── hardware/pad.rs         # Gamepad report command encoding
        ├── hardware/tests.rs       # HardwareBackend command/state tests
        ├── unavailable.rs          # Fail-closed hardware slot when --hardware-hid is absent
        ├── recording.rs            # RecordingBackend (in-memory event log)
        ├── recording/state.rs      # RecordingBackend internal state
        ├── software.rs             # SoftwareBackend (Windows SendInput)
        ├── software/input.rs       # SendInput-style INPUT struct preparation
        ├── software/keyboard.rs    # Software keyboard down/up + scancode toggle
        ├── software/mouse.rs       # Software mouse move/buttons/wheel
        ├── software/text.rs        # TypeText sampling + scheduling
        ├── software/utils.rs       # Bitmask / coord helpers
        ├── software_non_windows.rs # Compile-stub for non-Windows builds
        ├── vigem.rs                # VigemBackend public façade
        ├── vigem/client.rs         # ViGEm client + pad plug/unplug
        ├── vigem/error.rs          # ViGEm-specific error mapping
        ├── vigem/pad.rs            # PadId allocation
        ├── vigem/reports.rs        # GamepadReport → X360/DS4 report blob conversion
        ├── vigem/state.rs          # ViGEm session/pad state
        └── vigem/tests.rs          # ViGEm unit tests
```

### 2.6 `crates/synapse-perception/` — observation assembler & OCR

```
crates/synapse-perception/
├── Cargo.toml
└── src/
    ├── lib.rs                      # Crate-level re-exports
    ├── error.rs                    # PerceptionError with .code() mapping to OBSERVE_* / OCR_*
    ├── observe.rs                  # ObservationAssembler, ObservationInput, ObserveInclude, auto_mode, A11yTreeSummary
    └── ocr.rs                      # OcrProvider, TextRegion, read_text/read_text_with_provider, WinRT vs CRNN
```

### 2.7 `crates/synapse-a11y/` — UIA + WinEvent + CDP

```
crates/synapse-a11y/
├── Cargo.toml
└── src/
    └── lib.rs                      # 2087 LoC on main (HEAD `e54ca57`): UIA wrapper, AccessibleEvent, snapshot, find, foreground context, CDP attach
```

(Single-file lib as-shipped at `v0.1.0-m3`. A platform/* module split is in-progress as an M4 Block A.0 carry-over per `docs/impplan/04_m3_reflex_mcp_surface.md` — when it lands, `lib.rs` becomes a 30-LoC re-export surface with logic in `cdp.rs`, `events.rs`, `ids.rs`, `re_resolve.rs`, `snapshot.rs`, `window.rs`, `platform/non_windows.rs`, and `platform/windows/{common,events,resolve,snapshot,window}.rs`. Update this section in the same PR that lands the split.)

### 2.8 `crates/synapse-capture/` — frame capture

```
crates/synapse-capture/
├── Cargo.toml
└── src/
    └── lib.rs                      # 1798 LoC: DXGI + Windows.Graphics.Capture loops, DPI awareness, coord helpers, channel
```

### 2.9 `crates/synapse-audio/` — WASAPI loopback + STT

```
crates/synapse-audio/
├── Cargo.toml
└── src/
    ├── lib.rs                      # AudioRuntime, AudioConfig, DEFAULT_RING_SECONDS=5, AudioEventSink
    ├── error.rs                    # AudioError with .code() mapping to AUDIO_*
    ├── detectors.rs                # DetectorProcessor: VAD/transient/RMS detectors that emit Events
    ├── direction.rs                # Azimuth estimate from stereo magnitude/phase
    ├── loopback.rs                 # WASAPI loopback start, LoopbackHandle, LoopbackStatus
    ├── ring.rs                     # AudioRing (lock-free push), AudioFormat, AudioWindow, DEFAULT_SAMPLE_RATE_HZ, STEREO_CHANNELS
    ├── stt.rs                      # WhisperTinyStt + Transcription
    └── stt/window.rs               # STT window normalization
```

### 2.10 `crates/synapse-profiles/` — TOML profile loader

```
crates/synapse-profiles/
├── Cargo.toml
└── src/
    ├── lib.rs                      # Crate-level re-exports
    ├── error.rs                    # ProfileError + ProfileLoadError with .code() mapping to PROFILE_*
    ├── parser.rs                   # parse_profile_file, LoadedProfile, ProfileDefaults, ScreenBounds, bundled_profiles_dir
    ├── resolver.rs                 # ForegroundWindow + resolve_active_profile (regex matching against exe/title/steam_appid)
    ├── toml_format.rs              # RawProfile TOML schema (private)
    └── watcher.rs                  # ProfileRuntime (notify watcher, 200ms debounce, ProfileStatus)
```

### 2.11 `crates/synapse-hid-host/`

```
crates/synapse-hid-host/
├── Cargo.toml
└── src/
    ├── discover.rs                 # Synapse Pico serial-port discovery / auto-detect
    ├── error.rs                    # HidError + code mapping
    ├── handshake.rs                # IDENTIFY parsing and expected-version checks
    ├── lib.rs                      # Public exports
    ├── pipeline.rs                 # ACK/NAK pipeline, retries, backpressure
    ├── protocol.rs                 # CRC16 frame encoding/parsing
    ├── reconnect.rs                # Reconnect state machine and snapshots
    └── transport.rs                # Serialport-backed HidGateway
```

### 2.12 `crates/synapse-models/` — ONNX runtime wrapper

```
crates/synapse-models/
├── Cargo.toml                      # features = default, ort, cuda, directml
└── src/
    └── lib.rs                      # ModelDescriptor, Detector trait, DetectionFrame, ort feature-gated session loader
```

### 2.13 `crates/synapse-telemetry/` — tracing + metrics

```
crates/synapse-telemetry/
├── Cargo.toml
└── src/
    ├── lib.rs                      # TelemetryConfig, init_tracing, GcWorker, install_panic_hook, default_log_dir
    └── metrics.rs                  # M3_METRICS array (19 specs), describe_metric, CARDINALITY_LIMIT=1000
```

### 2.14 `crates/synapse-test-utils/` — shared test rig

```
crates/synapse-test-utils/
├── Cargo.toml
└── src/
    ├── lib.rs                      # Re-exports
    ├── fixtures.rs                 # launch_notepad, wait_for_window_title_regex, notepad_process_ids
    └── stdio_mcp_client.rs         # StdioMcpClient: launches synapse-mcp, drives initialize + tools/call
```

### 2.15 `crates/synapse-overlay/` — M5 placeholder

```
crates/synapse-overlay/
├── Cargo.toml
└── src/
    └── main.rs                     # 1 LoC stub; M5 debug overlay target (default-member alongside synapse-mcp)
```

## 3. Helper scripts

| Path | Description |
|---|---|
| `scripts/check-bench-delta.ps1` | Compares two `critcmp` JSON outputs and fails if any tracked benchmark regressed beyond 20% |
| `scripts/check_dep_graph.sh` | Validates the crate dependency graph against the architecture document |
| `scripts/check_docs.ps1` | Validates cross-references in `docs/` |
| `scripts/check_docs_smoke.ps1` | Smoke variant of the docs link check |
| `scripts/clean-runs.ps1` | Cleans local test artifacts (replays, logs, db) |
| `scripts/new-crate.ps1` | Scaffolds a new workspace member crate |

## 4. Documentation tree

| Path | Description |
|---|---|
| `docs/computergames/` | Product Requirements Document (PRD) — 18 numbered files covering architecture, perception, action, reflex, MCP surface, schemas, storage, supported use, hardware HID, perf budget, security, observability, testing, build, roadmap, open questions, research appendix |
| `docs/impplan/` | Implementation plan — methodology + per-milestone work-item ledger (M0 through M5) + cross-cutting concerns |
| `docs/adr/0001-current-rust-and-dependencies.md` | ADR: pinned to current stable Rust, no MSRV downgrade |
| `docs/adr/0002-rocksdb-primary-storage.md` | ADR: RocksDB chosen over LMDB/sled for primary storage |
| `docs/adr/0003-reflex-recursion-guard.md` | ADR: on-event recursion guard design |
| `docs/adr/0004-reflex-priority.md` | ADR: reflex scheduler priority semantics |
| `docs/adr/0005-multi-monitor-capture-target.md` | ADR: multi-monitor capture target resolution rules |
| `docs/adr/0006-profile-match-precedence.md` | ADR: profile match precedence when multiple profiles match the foreground |
| `docs/adr/0007-per-event-vs-batched-notifications.md` | ADR: per-event notifications over the SSE bus rather than batching |
| `docs/AICodingAgentSuperPrompt.md` | Agent prompt that AGENTS.md references for wake-up context |
| `docs/compressionprompt.md` | Doctrine for compressed implementation-plan authoring |
| `docs/dev-host-hygiene.md` | Configured-host hygiene checklist (toolchain, drivers) |
| `docs/m1_error_throw_map.md` | M1 error code throw-site map |

## 5. Module dependency graph (build-order)

```
synapse-core            (no synapse-* deps; standalone shared types)
  ↑
  ├── synapse-telemetry     (logging + metrics; no synapse-* deps)
  ├── synapse-models        (synapse-core for DetectionBatch)
  ├── synapse-storage       (synapse-core: retention, error_codes)
  ├── synapse-profiles      (synapse-core)
  ├── synapse-a11y          (synapse-core)
  ├── synapse-capture       (synapse-core, synapse-telemetry)
  ├── synapse-audio         (synapse-core, synapse-models{directml})
  ├── synapse-action        (synapse-core; cfg(windows): synapse-a11y, synapse-capture)
  ├── synapse-perception    (synapse-core; cfg(windows): synapse-a11y, synapse-capture, synapse-models)
  ├── synapse-reflex        (synapse-core, synapse-storage, synapse-action)
  └── synapse-hid-host      (synapse-core; serialport + crc16 HID gateway)

synapse-mcp (binary)
  ↑
  └── depends on every library crate above
        +  synapse-test-utils (dev-dep only)

synapse-overlay (binary)
  ↑
  └── no synapse-* deps yet (1 LoC stub)

synapse-test-utils
  ↑
  └── synapse-mcp (for the StdioMcpClient launch path) — dev-only
```

## 6. Entry point traces

### 6.1 `synapse-mcp --mode stdio`

```
main.rs::main
  └─ run()
     ├─ Cli::parse() → Cli (clap)
     ├─ configure_telemetry()  → synapse_telemetry::init_tracing(TelemetryConfig{...})
     ├─ synapse_capture::init_process_dpi_awareness()
     └─ run_stdio(telemetry_guard, cli.m3_config())
        ├─ SynapseService::try_with_m2_shutdown_reason_and_m3_config(...)
        │  ├─ SharedM1State::default()                       (m1.rs::M1State::from_env)
        │  ├─ shared_m2_state_from_env_with_shutdown_reason  (m2.rs)
        │  │  └─ M2State::from_recording_backend_env_with_actor_backend
        │  │     ├─ synapse_action::initialize_double_click_timing_cache
        │  │     └─ synapse_action::ActionEmitter::channel  → spawns emitter actor task
        │  └─ shared_m3_state_from_config_with_shutdown_reason_and_sse_state
        │     └─ M3State::from_parts_with_sse_state  → SseState::with_max_subscriptions
        ├─ synapse_action::install_panic_hook()
        ├─ safety::install_operator_hotkey()  → low-level keyboard hook on Win32
        ├─ rmcp::transport::stdio() → (stdin, stdout) wrapped in CancelOnEofRead
        └─ service.serve_with_ct(...)  → rmcp service loop (rmcp 1.7.0)
            └─ on each tools/call dispatches to tool_router (#[tool_router] in server.rs)
```

### 6.2 `synapse-mcp --mode http`

```
main.rs::main
  └─ http::serve(bind, allow_non_loopback, m3_config)
     └─ transport.rs::serve
        ├─ SocketAddr parsing + loopback enforcement (HTTP_BIND_NON_LOOPBACK_REFUSED)
        ├─ TcpListener::bind
        ├─ SseState::with_max_subscriptions
        ├─ http_service()  → SynapseService::try_with_m2_shutdown_reason_and_sse_state_and_m3_config
        ├─ safety::install_operator_hotkey
        ├─ router(): axum::Router
        │   ├─ GET  /health        → transport.rs::health
        │   ├─ GET  /events        → SseState::open (SSE)
        │   ├─ POST /events        → SseState::publish  (only when SYNAPSE_HTTP_SSE_MANUAL=1)
        │   ├─ GET  /events/stats  → SseState::stats    (only when SYNAPSE_HTTP_SSE_MANUAL=1)
        │   ├─ /mcp/*              → rmcp StreamableHttpService (LocalSessionManager)
        │   ├─ layer: require_mcp_session    (http/session.rs)
        │   └─ layer: require_http_security  (http/auth.rs — Origin/Host + Bearer)
        ├─ axum::serve with graceful_shutdown(shutdown_cancel)
        └─ on SIGINT / Ctrl-Break: cancels shutdown_cancel and connection_closed_cancel
```

### 6.3 Tool invocation: e.g. `tools/call name=reflex_register`

```
rmcp dispatches → SynapseService::call_tool
  └─ tool_router.call(context)
     └─ SynapseService::reflex_register (server.rs)
        ├─ require_m3_permissions("reflex_register", required_permissions_register(params))
        │   └─ m3::permissions::PermissionGrants::first_missing → authorization_error if denied
        ├─ self.reflex_runtime()
        │   ├─ M3State::ensure_reflex_runtime
        │   │   ├─ Db::open(db_path, SCHEMA_VERSION)  → schema sentinel verify
        │   │   ├─ optional run_pressure_check_with_free_bytes_sample
        │   │   └─ ReflexRuntime::spawn_with_config (action_handle, event_bus, scheduler_config)
        │   └─ M3State::ensure_a11y_event_bridge (event_bus)  → A11yEventBridge::start
        └─ register_reflex(&runtime, params)
            ├─ scheduled_reflex_from_params  → builds ScheduledReflex
            ├─ ReflexRuntime::register
            │   ├─ ReflexScheduler::spawn_with_audit_db   → spawns scheduler thread
            │   ├─ write_registration_audit               → encodes JSON into CF_REFLEX_AUDIT
            │   └─ Db::flush
            └─ returns ReflexRegisterResponse{ reflex_id, state }
```

### 6.4 Tool invocation: `tools/call name=observe`

```
SynapseService::observe
  └─ self.m1_state()  (Mutex<M1State>)
     └─ assemble_observation(&state, &params)
        ├─ current_input(state, depth)
        │   ├─ if state.synthetic.is_some()      → returns synthetic fixture
        │   ├─ if state.force_no_perception      → OBSERVE_NO_PERCEPTION_AVAILABLE
        │   ├─ if state.force_observe_internal   → OBSERVE_INTERNAL
        │   └─ otherwise: m1/sources.rs::platform_input(depth, perception_mode)
        │       ├─ synapse_a11y::current_foreground_context
        │       ├─ synapse_a11y::focused_element (optional)
        │       ├─ synapse_a11y::snapshot(root, depth)  (A11y tree)
        │       └─ assembles ObservationInput { foreground, elements, entities, ... }
        └─ ObservationAssembler::new().assemble(ObserveInclude::default(), input)
           └─ returns synapse_core::Observation
```

## 7. Build / package configuration

| File | Purpose |
|---|---|
| `Cargo.toml` (workspace) | Declares 15 workspace members (`crates/synapse-*`), default-members `[synapse-mcp, synapse-overlay]`, `exclude = ["firmware/pico-hid"]`. Workspace package metadata: `version = "0.1.0"`, `edition = 2024`, `rust-version = "1.95"`, `license = "MIT OR Apache-2.0"`, `repository = "https://github.com/ChrisRoyse/Synapse"`. |
| `Cargo.toml [workspace.dependencies]` | Pins all third-party deps (38 entries) at the workspace level; child crates use `<dep>.workspace = true` |
| `Cargo.toml [workspace.lints.rust]` | `unsafe_code = forbid`, `unused = warn` |
| `Cargo.toml [workspace.lints.clippy]` | `all = deny`, `pedantic/nursery = warn`, `unwrap_used = deny`, `expect_used = deny` |
| `Cargo.toml [profile.dev]` | `opt-level=0`, line-only debug, incremental |
| `Cargo.toml [profile.release]` | `opt-level=3`, `lto="thin"`, `codegen-units=16`, `strip=true`, `panic="abort"` |
| `Cargo.toml [profile.release-max]` | inherits release, `lto="fat"`, `codegen-units=1` |
| `Cargo.toml [profile.bench]` | inherits release with line-only debug |
| Per-crate `Cargo.toml` | All workspace members override `[lints.rust]::unsafe_code` to `allow` only in: `synapse-action`, `synapse-a11y`, `synapse-capture`, `synapse-audio`, `synapse-hid-host` (anywhere doing FFI). Default-binary in `synapse-mcp/Cargo.toml`: `[[bin]] name = "synapse-mcp" path = "src/main.rs"`. Bench harness flags (`harness = false`) declare each `criterion` bench. |
| `deny.toml` | cargo-deny configuration (advisory/licensing checks). Repo policy bans GitHub Actions as a shipping gate (`AGENTS.md`), so this is used locally only. |

The release binary path on Windows is `target/release/synapse-mcp.exe` (linux/wsl variant `target/release/synapse-mcp`).

## 8. Test files (integration tests; per-crate `tests/` directories)

See [14_test_suite.md](#file-14) for the full inventory and per-file test counts. Summary: 76 `tests/*.rs` files plus 13 `benches/*.rs` files across the workspace.


---

<a id="file-03"></a>

> Source: `docs/systemspec/03_configuration.md`

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
| `--hardware-hid` | `SYNAPSE_HARDWARE_HID` | `Option<String>` | `None` | `auto` or a serial port name such as `COM7` | Enables the hardware HID backend. `auto` enumerates matching Synapse Pico serial ports and proves identity; a port value opens that port directly. Missing/no-match fails startup with `HID_PORT_NOT_FOUND`; omission leaves `Backend::Hardware` fail-closed through `ACTION_BACKEND_UNAVAILABLE`. |

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

### 4.9 Telemetry
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

There is no merge step: CLI/env values configure individual subsystems independently, each at the moment the subsystem is constructed. There is no hot-reload of CLI flags or env vars — restart the daemon to change any of them. Profile TOML files, by contrast, are watched and hot-reloaded with a 200 ms debounce (see [11_profiles_hid_telemetry.md](#file-11) in the deep-dives).

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


---

<a id="file-04"></a>

> Source: `docs/systemspec/04_storage_layer.md`

# 04 — Storage Layer (RocksDB)

Source files covered:
- `crates/synapse-storage/src/lib.rs`
- `crates/synapse-storage/src/cf.rs`
- `crates/synapse-storage/src/codecs.rs`
- `crates/synapse-storage/src/compaction.rs`
- `crates/synapse-storage/src/batch.rs`
- `crates/synapse-storage/src/gc.rs`
- `crates/synapse-storage/src/pressure.rs`
- `crates/synapse-storage/src/error.rs`
- `crates/synapse-core/src/defaults.rs`
- `crates/synapse-core/src/retention.rs`
- `crates/synapse-core/src/types.rs` (Stored* types)

## 1. Connection management

The single `Db` handle (`crates/synapse-storage/src/lib.rs:33`) owns:

- `path: PathBuf` — root directory passed to `Db::open`
- `schema_version: u32` — pinned at construction (= `synapse_core::SCHEMA_VERSION` = `1`)
- `batcher: batch::Batcher` — background actor consuming write batches
- `inner: Arc<rocksdb::DB>` — the RocksDB handle, shared with GC + pressure tasks
- `pressure: Arc<pressure::PressureState>` — current disk-pressure level (atomic `u8`)

`Db::open(path, schema_version)` (`lib.rs:60`):

1. Builds the base `Options` (`db_options()`):
   - `create_if_missing = true`
   - `create_missing_column_families = true`
   - `max_background_jobs = 2`
   - Default `compression_type = Lz4`
   - `max_open_files = 256`
   - `keep_log_file_num = 8`
   - `write_buffer_size = 64 MiB` (`DEFAULT_WRITE_BUFFER_BYTES`)
   - `max_write_buffer_number = 3`
   - `target_file_size_base = 64 MiB`
   - `level_zero_file_num_compaction_trigger = 4`
   - Block-based table factory with a shared `LruCache` of `64 MiB` (`BLOCK_CACHE_BYTES`)
2. Builds per-CF `Options` via `cf_options(name)`:
   - All CFs: same write buffer + L0 compaction settings as the base
   - **Time-keyed CFs** (`CF_EVENTS`, `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`): `compression = Lz4` (default), `SliceTransform::create_fixed_prefix(8)` for prefix-bloom filters
   - **`CF_MODEL_CACHE`**: `compression = None`, larger write buffer (`MODEL_CACHE_WRITE_BUFFER_BYTES = 256 MiB`) so ONNX blobs spill to L0 less often
   - **`CF_OBSERVATIONS`, `CF_SESSIONS`**: `compression = Zstd` (higher ratio for retained snapshots)
   - All CFs receive the `install_ttl_filter` compaction filter (see §6) and the same shared `LruCache`
3. Opens with `DB::open_cf_descriptors`, wrapping any rocksdb failure into `StorageError::OpenFailed` and emitting a `tracing::warn` with `code = STORAGE_OPEN_FAILED`.
4. Verifies the schema sentinel via `verify_schema_version` (§3).
5. Verifies all CF handles are present; missing → `STORAGE_OPEN_FAILED` with a detail string.
6. Wraps the DB in `Arc`, spawns the `Batcher` task, initializes a fresh `PressureState`, and returns `Db`.

There is **no connection pool** — RocksDB is a single embedded instance per process. Concurrent access goes through the shared `Arc<DB>` (the `multi-threaded-cf` feature is enabled in `Cargo.toml`).

## 2. Codecs

`crates/synapse-storage/src/codecs.rs` (39 LoC) defines the only persisted codecs:

| Function | Signature | Behavior | Error code |
|---|---|---|---|
| `encode_json<T: Serialize>` | `(&T) -> StorageResult<Vec<u8>>` | `serde_json::to_vec` | `STORAGE_WRITE_FAILED` via `StorageError::EncodeJson` |
| `decode_json<T: DeserializeOwned>` | `(&[u8]) -> StorageResult<T>` | `serde_json::from_slice` | `STORAGE_READ_FAILED` via `StorageError::DecodeJson` |

A source-code comment pins the constraint: "ADR-0001 / RUSTSEC-2025-0141 prohibit binary persisted codecs here; storage payloads stay JSON so state-readback bytes remain inspectable." Bincode/postcard/etc. are not used.

## 3. Schema versioning

Schema version is a single big-endian `u32` stored under the key `__schema_version` (`SCHEMA_VERSION_KEY = b"__schema_version"`, `crates/synapse-storage/src/lib.rs:30`).

`verify_schema_version` (`lib.rs:399`):

1. Reads `__schema_version`. If absent (fresh DB), writes the expected value and returns `Ok(())`.
2. If present, decodes as big-endian `u32`. Match → `Ok(())`. Mismatch → `StorageError::SchemaMismatch { expected, actual }` → maps to `STORAGE_SCHEMA_MISMATCH`.

`synapse_core::SCHEMA_VERSION = 1` (`crates/synapse-core/src/defaults.rs`). Pre-v1 doctrine (per `docs/computergames/README.md` "Authoring rules" and `docs/impplan/00_methodology.md`): schema changes wipe-and-rebuild; no migration shims are present in the storage crate.

## 4. Column families

Defined in `crates/synapse-storage/src/cf.rs`. `ALL_COLUMN_FAMILIES` (line 25) is the canonical array of 11 CF names, excluding the implicit RocksDB `default` CF.

| # | CF constant | String value | Purpose | Compression | Prefix extractor |
|---|---|---|---|---|---|
| 1 | `CF_EVENTS` | `"CF_EVENTS"` | Replay event log (M3 reflex bus persistence) | Lz4 | fixed-prefix 8 |
| 2 | `CF_OBSERVATIONS` | `"CF_OBSERVATIONS"` | Observation snapshots retained for replay and debugging | Zstd | — |
| 3 | `CF_PROFILES` | `"CF_PROFILES"` | Cached profile loads; on-disk TOML remains the source of truth | Lz4 | — |
| 4 | `CF_MODEL_CACHE` | `"CF_MODEL_CACHE"` | Downloaded ONNX model cache | None | — |
| 5 | `CF_SESSIONS` | `"CF_SESSIONS"` | MCP session continuity records | Zstd | — |
| 6 | `CF_REFLEX_AUDIT` | `"CF_REFLEX_AUDIT"` | Per-reflex audit trail (registered/fired/cancelled/expired/disabled) | Lz4 | fixed-prefix 8 |
| 7 | `CF_OCR_CACHE` | `"CF_OCR_CACHE"` | OCR memoization cache for stable regions | Lz4 | — |
| 8 | `CF_TELEMETRY` | `"CF_TELEMETRY"` | Local metric ring buffer | Lz4 | — |
| 9 | `CF_ACTION_LOG` | `"CF_ACTION_LOG"` | Emitted action log | Lz4 | fixed-prefix 8 |
| 10 | `CF_PROCESS_HISTORY` | `"CF_PROCESS_HISTORY"` | Process start/exit history | Lz4 | — |
| 11 | `CF_KV` | `"CF_KV"` | Generic bounded key-value extension | Lz4 | — |

The implicit RocksDB `default` CF is created automatically but holds only the `__schema_version` sentinel.

### 4.1 Schema (current persisted value types)

These are the `serde_json` payloads written into each CF. Source: `crates/synapse-core/src/types.rs`, plus call sites in `synapse-reflex`, `synapse-mcp`, `synapse-profiles`, `synapse-models`.

| CF | Persisted type | Fields | Key shape (current writer) |
|---|---|---|---|
| `CF_EVENTS` | `StoredEvent` | `schema_version: u32`, `event_id: String`, `ts_ns: u64`, `session_id: Option<String>`, `source: EventSource`, `kind: String`, `data: serde_json::Value`, `window_id: Option<i64>`, `element_id: Option<ElementId>`, `redacted: bool`, `redactions: Vec<StoredRedaction>` | — (no live writer in this build; PRD §7 calls for `ts_ns` big-endian prefix + `event_id`) |
| `CF_OBSERVATIONS` | `StoredObservation` | `schema_version`, `observation_id`, `ts_ns`, `session_id`, `mode: PerceptionMode`, `foreground: ForegroundContext`, `focused: Option<FocusedElement>`, `elements: Vec<AccessibleNode>`, `entities: Vec<DetectedEntity>`, `hud: HudReadings`, `audio: AudioContext`, `recent_events: Vec<EventSummary>`, `clipboard_summary: Option<ClipboardSummary>`, `fs_recent: Vec<FsEvent>`, `diagnostics: ObservationDiagnostics`, `reason: String`, `redacted: bool`, `redactions: Vec<StoredRedaction>` | — (no live writer in this build; produced by future M3 replay backends. `replay_record` writes JSONL to disk, not to this CF.) |
| `CF_PROFILES` | (per PRD) cached `Profile` rows | `Profile { id, label, version, use_scope, matches, mode, capture, detection, ocr, hud, keymap, backends, event_extensions }` | not written in current build; profiles read from TOML and held in `synapse-profiles::ProfileRuntime` memory |
| `CF_MODEL_CACHE` | raw bytes + `ModelDescriptor` | binary ONNX blob behind a JSON-encoded descriptor key | not exercised in current build (no model auto-download yet) |
| `CF_SESSIONS` | `StoredSession` | `schema_version`, `session_id`, `started_at`, `ended_at`, `transport`, `client`, `mode`, `active_profile`, `profile_history: Vec<StoredProfileHistoryEntry>`, `redacted`, `redactions` | not written in this build |
| `CF_REFLEX_AUDIT` | `StoredReflexAudit` | `schema_version`, `audit_id`, `reflex_id`, `ts_ns`, `status: ReflexState`, `event_id: Option<String>`, `steps: Vec<StoredReflexStep>`, `error_code: Option<String>`, `details: serde_json::Value`, `redacted`, `redactions` | `format!("{reflex_id}:{audit_id}")` (see §4.2) |
| `CF_OCR_CACHE` | not yet wired | — | — |
| `CF_TELEMETRY` | not yet wired | — | — |
| `CF_ACTION_LOG` | not yet wired | — | — |
| `CF_PROCESS_HISTORY` | not yet wired | — | — |
| `CF_KV` | not yet wired (generic) | — | — |

`StoredReflexStep` is `{ index: u32, action: Action, status: String, error_code: Option<String> }`.
`StoredRedaction` is `{ kind: String, offset: u32, len: u32 }`.

### 4.2 Active write paths (current build)

The only CF actively written by the live build is `CF_REFLEX_AUDIT`:

| Caller | Trigger | Audit payload | Key format |
|---|---|---|---|
| `ReflexRuntime::register` (`crates/synapse-reflex/src/lib.rs:146`) | tool `reflex_register` | `details.kind = "reflex_registered"`, `status = Active`, `error_code = None` | `"<reflex_id>:<audit_id>"` (v7 UUID for audit_id) |
| `ReflexRuntime::cancel` (`lib.rs:198`) | tool `reflex_cancel` | `details.kind = "reflex_cancelled"`, `status = Cancelled` | same |
| `ReflexRuntime::disable_all_by_operator` (`lib.rs:245`) | operator panic hotkey (`crates/synapse-mcp/src/safety.rs::handle_operator_hotkey`) | `details.kind = "reflex_disabled_by_operator"`, `status = Disabled`, `error_code = REFLEX_DISABLED_BY_OPERATOR` | same |
| `ReflexScheduler` fire path (in `crates/synapse-reflex/src/scheduler.rs` + `kinds/on_event.rs`) | each reflex fire | `details.kind = "reflex_fired"`, `status = Active`, optional `event_id` and per-step `steps` | same |
| recursion-guard clamp (`kinds/on_event.rs`) | exceeded `MAX_ON_EVENT_FIRINGS_PER_TICK` | `error_code = REFLEX_RECURSION_LIMIT` | same |

Writers go through `synapse_reflex::audit::write_audit` (`crates/synapse-reflex/src/audit.rs`), which is just a thin wrapper around `Db::put_batch(CF_REFLEX_AUDIT, ...)` followed (by the caller) by `Db::flush()`.

## 5. Index strategy

RocksDB indexes by key. Prefix-bloom filters are configured (`SliceTransform::create_fixed_prefix(8)`) on the three time-keyed CFs (`CF_EVENTS`, `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`). For audit lookups by reflex id, callers use `Db::scan_cf_prefix(CF_REFLEX_AUDIT, b"<reflex_id>:")` (`crates/synapse-storage/src/lib.rs:302`), which seeks to the prefix and breaks once the iterator leaves the prefix.

There are no secondary indexes maintained by application code. `ReflexRuntime::history` (`crates/synapse-reflex/src/lib.rs:311`) scans either by `reflex_id` prefix or globally, then sorts the deserialized `Vec<StoredReflexAudit>` by `(ts_ns desc, audit_id desc, reflex_id desc)` before applying `limit`.

## 6. TTL compaction filter

`crates/synapse-storage/src/compaction.rs` installs a per-CF compaction filter using each CF's `RetentionTtl` from `synapse_core::retention::DEFAULTS` (`crates/synapse-core/src/retention.rs`):

| CF | TTL | Soft cap (MB) | Hard cap (MB) |
|---|---|---:|---:|
| `CF_EVENTS` | 24 hours | 2048 | 4096 |
| `CF_OBSERVATIONS` | 6 hours | 500 | 1000 |
| `CF_PROFILES` | none | 20 | 50 |
| `CF_MODEL_CACHE` | LRU-only (no TTL) | 1024 | 2048 |
| `CF_SESSIONS` | 30 days | 50 | 100 |
| `CF_REFLEX_AUDIT` | 7 days | 200 | 500 |
| `CF_OCR_CACHE` | 1 hour | 50 | 100 |
| `CF_TELEMETRY` | 6 hours | 100 | 200 |
| `CF_ACTION_LOG` | 24 hours | 200 | 500 |
| `CF_PROCESS_HISTORY` | 6 hours | 20 | 50 |
| `CF_KV` | none | 10 | 50 |

Filter behavior (`ttl_decision`): the compaction filter parses the JSON value (byte-scanning for the literal `"ts_ns"` field, no full JSON parse), reads its `u64`, and compares `now_ns - ts_ns > ttl_ns`. If `ts_ns` cannot be extracted or the value is fresh enough, the row is kept; otherwise removed.

For CFs without a `ts_ns` field (e.g. `CF_PROFILES`, `CF_KV`) the filter is still attached but every row falls into the "kept" branch.

## 7. Garbage collection (soft/hard caps)

`crates/synapse-storage/src/gc.rs`:

- `GC_INTERVAL = 5 minutes` (`Duration::from_mins(5)`).
- `Db::spawn_gc_task` spawns a tokio task that runs `run_once` at every tick.
- `GcConfig::from_retention_defaults` builds a `GcBudget` per CF in **bytes** (each `soft_cap_mb`/`hard_cap_mb` × 1 MiB), with `unit = CapUnit::Bytes`. A test-only `for_row_caps` variant uses `CapUnit::Rows`.
- `run_cf` (`gc.rs:159`) per-CF algorithm:
  1. `collect_keys` walks the CF (`IteratorMode::Start`) to compute current key list and total measured size.
  2. Records `ESTIMATE_NUM_KEYS` and (for byte units) iterates keys to compute total bytes; for row units, uses key count.
  3. If `before_value >= hard_cap`, emits `tracing::warn` with `code = STORAGE_CF_HARD_CAP_REACHED`.
  4. If `before_value > soft_cap`, calls `evict_oldest`: sorts keys lex-asc and `DB::delete_cf` from the oldest until `before_value - sum_of_evicted_bytes <= soft_cap`.
  5. After eviction, re-collects keys for `after_value`, and increments the Prometheus counter `cache_evictions_total{cf=<name>, reason="soft_cap"}` by the evicted row count.
- Returns `GcReport { cf_reports: Vec<GcCfReport> }` (one entry per budget) so callers can readback before/after sizes.

## 8. Disk pressure monitor

`crates/synapse-storage/src/pressure.rs`:

- `POLL_INTERVAL = 30 s`.
- Pressure thresholds (DB volume free bytes):

| Level | Free-bytes threshold (≤) | Error code |
|---|---|---|
| `Normal` (0) | — | none |
| `Level1` | `2 GB` | `STORAGE_DISK_PRESSURE_LEVEL_1` |
| `Level2` | `1 GB` | `STORAGE_DISK_PRESSURE_LEVEL_2` |
| `Level3` | `500 MB` | `STORAGE_DISK_PRESSURE_LEVEL_3` |
| `Level4` | `200 MB` | `STORAGE_DISK_PRESSURE_LEVEL_4` |

(`GB`/`MB` use decimal: `1_000_000_000` and `1_000_000`.)

- `PressureState` holds the current level as `AtomicU8`; `Db::pressure_level()` reads it.
- `Db::put_batch` consults `pressure.permits_write(cf_name)` before submitting the batch. At higher levels the responder freezes specific CFs; writes are then silently dropped after a `tracing::warn` with `code = STORAGE_WRITE_FAILED`. (`Db::put_batch` in `lib.rs:120-130`.)
- The poller may also trigger compaction on selected CFs at higher levels.
- Test-only entrypoint `Db::run_pressure_check_with_free_bytes_sample(free_bytes)` lets the daemon apply a synthetic sample at startup via `--storage-pressure-free-bytes-sample`. (See [03_configuration.md](#file-03).)

## 9. Write path / batching

`crates/synapse-storage/src/batch.rs` implements a `Batcher` actor wrapping `WriteBatchWithIndex`:

1. `Db::put_batch(cf_name, kvs)` validates the CF handle, checks `pressure.permits_write`, materializes the KV pairs as `Vec<(Vec<u8>, Vec<u8>)>`, and forwards to the batcher.
2. The batcher aggregates writes and flushes either on an explicit `Db::flush()` call or when the next batch arrives.
3. `Db::flush()` issues a synchronous flush (`WriteOptions::sync`-style). The current reflex audit pattern is `write_audit(&db, &audit)` followed by `db.flush()`, so each persisted audit is durable before the tool response returns.
4. Empty key sets are no-ops (`if kvs.is_empty() { return Ok(()) }`).

There are no transactions. Reflex audit consistency is achieved by the single-writer model: only `ReflexRuntime` writes `CF_REFLEX_AUDIT`, holding its own `Mutex` while it does so (`crates/synapse-mcp/src/m3/reflex.rs::register_reflex`).

## 10. Query helpers

| Function | Source | Behavior |
|---|---|---|
| `Db::cf_sizes()` | `lib.rs:209` | Scans every CF in `ALL_COLUMN_FAMILIES`, sums `key.len + value.len`, returns `BTreeMap<String, u64>`. Used by `synapse_mcp::server::SynapseService::storage_health` to populate `health.subsystems.storage.cf_sizes`. |
| `Db::scan_cf(cf_name)` | `lib.rs:282` | Iterates the CF from the start and returns owned `(key, value)` byte pairs. |
| `Db::scan_cf_prefix(cf_name, prefix)` | `lib.rs:302` | Iterates from `IteratorMode::From(prefix, Direction::Forward)` and breaks once iterator leaves the prefix. |
| `Db::compact_cf(cf_name)` | `lib.rs:331` | Triggers `compact_range_cf(None, None)` over the entire CF; used by the pressure responder. |
| `Db::pressure_level()` | `lib.rs:196` | Returns the cached `DiskPressureLevel`. |
| `Db::run_gc_once()` | `lib.rs:150` | Synchronous one-shot GC pass using retention defaults. |
| `Db::run_pressure_check_once()` | `lib.rs:229` | Synchronous one-shot disk-pressure poll. |

There is no higher-level query API (no SQL, no secondary indexes). All RocksDB operations go through this surface.

## 11. Replay JSONL (alternative persistence)

`replay_record` (`crates/synapse-mcp/src/m3/replay.rs`) writes observation/event records to a **flat JSONL file** under `%LOCALAPPDATA%/synapse/replays`, **not** into the RocksDB CFs. Cadence: observations sampled every 250 ms, events drained every 20 ms, both written via `tokio::io::BufWriter<File>` until the requested `duration_ms` elapses. Paths outside `replay_root()` are rejected with `SAFETY_PERMISSION_DENIED`.

## 12. Virtual tables, extensions, special features

- **None.** Synapse uses stock RocksDB compaction filters and slice transforms; no merge operators, no transactions, no secondary indexes, no virtual CFs. The schema sentinel lives in the default CF; the operator-visible CFs are exactly the 11 listed.
- **Compression**: LZ4 default, ZSTD on `CF_OBSERVATIONS`/`CF_SESSIONS`, none on `CF_MODEL_CACHE`.
- **Block cache**: shared 64 MiB LRU across all CFs via `BlockBasedOptions::set_block_cache`.

## 13. What is NOT covered

- **Migrations.** There is no migration framework; bumping `SCHEMA_VERSION` requires deleting the database directory.
- **Cross-process locking.** A single `synapse-mcp` process owns the directory; running two daemons against the same `--db` path will fail on `Db::open` because RocksDB's exclusive lock kicks in.
- **Backups.** The daemon does not export, snapshot, or back up its own DB; any backup strategy is operator-side (e.g. file-system snapshot while the process is stopped).
- **Encryption-at-rest.** RocksDB is not configured with encryption.
- **`CF_EVENTS` / `CF_OBSERVATIONS` writers.** The persistence pipeline for these CFs is wired in retention defaults and tested, but no production code path currently writes events/observations through them — they remain reserved for upcoming work. The reflex audit is the only live writer in M3.


---

<a id="file-05"></a>

> Source: `docs/systemspec/05_core_types_and_errors.md`

# 05 — Core Types and Error Hierarchy (`synapse-core`)

Source files covered:
- `crates/synapse-core/src/lib.rs`
- `crates/synapse-core/src/defaults.rs`
- `crates/synapse-core/src/error_codes.rs`
- `crates/synapse-core/src/filter.rs`
- `crates/synapse-core/src/retention.rs`
- `crates/synapse-core/src/types.rs`

## 1. Crate role

`synapse-core` is the **single dependency-free type/contract crate**. Every other Synapse crate depends on it; it depends on no other Synapse crate. It defines:

- All wire-level structs/enums that travel over MCP (params, responses, observations, events, profiles, reflex registrations, stored persistence variants, health payload).
- The `pub const` error code string set (`SCREAMING_SNAKE_CASE`).
- Retention defaults consumed by `synapse-storage`.
- Reference performance budgets used by tests and tooling.
- The `EventFilter` evaluator (`filter.rs`).

## 2. Constants

| Constant | Value | Used by |
|---|---|---|
| `SCHEMA_VERSION` | `1` | `synapse-storage::Db::open`, `StoredEvent`, `StoredObservation`, `StoredReflexAudit`, `StoredSession` payloads |
| `REFERENCE_OBSERVE_WARM_HYBRID_P99_MS` | `30.0` | Perf budget tests |
| `REFERENCE_REFLEX_TICK_JITTER_IDLE_P99_US` | `200` | Perf budget tests |
| `REFERENCE_EVENT_TO_SUBSCRIBER_P99_MS` | `50.0` | Perf budget tests |
| `EVENT_FILTER_MAX_DEPTH` | `8` | `EventFilter::validate` |

## 3. Error code catalog

All 95 codes are `pub const &'static str` in `crates/synapse-core/src/error_codes.rs`. Mapped from each subsystem's `thiserror` enum's `.code()` method. Categories with line ranges (see [01_system_overview.md §8](#file-01) for the table).

M3 added the following codes to the M2 baseline: `REFLEX_RECURSION_LIMIT`, `REFLEX_ACTION_PERMISSION_DENIED`, `HTTP_BIND_NON_LOOPBACK_REFUSED`, `HTTP_TOKEN_INVALID`, `HTTP_ORIGIN_REFUSED`, `HTTP_SESSION_INVALID`, `REPLAY_TARGET_INVALID`, `REPLAY_FORMAT_INVALID`, `SUBSCRIPTION_CAP_REACHED`, `STORAGE_DISK_PRESSURE_LEVEL_1..4`, `STORAGE_CF_HARD_CAP_REACHED`, `SAFETY_PERMISSION_DENIED`, `SAFETY_PROFILE_ACTION_DENIED`, `SAFETY_OPERATOR_HOTKEY_FIRED`.

## 4. Retention defaults

`retention.rs` exports `DEFAULTS: [RetentionDefault; 11]` and the `RetentionTtl` enum.

```rust
pub enum RetentionTtl { None, Hours(u64), Days(u64), LruOnly }

pub struct RetentionDefault {
    pub cf: &'static str,
    pub ttl: RetentionTtl,
    pub soft_cap_mb: u64,
    pub hard_cap_mb: u64,
}
```

Full table in [04_storage_layer.md §6](#file-04).

## 5. Wire-level types (`types.rs`)

### 5.1 Identity and primitives

| Type | Source | Notes |
|---|---|---|
| `Backend` | enum `Software` \| `Vigem` \| `Hardware` \| `Auto` | All four lowercased on the wire |
| `Point` | `{ x: i32, y: i32 }` | screen coords; provides `distance_to(other: Self) -> f64` |
| `Rect` | `{ x: i32, y: i32, w: i32, h: i32 }` | `contains(point: Point)` with exclusive right/bottom edges; non-positive width/height treated as empty |
| `Size` | `{ w: u32, h: u32 }` | |
| `SessionId` / `EntityId` / `ReflexId` / `SubscriptionId` / `ProfileId` | `type ... = String` | UUIDs (v7 for reflex/subscription/session; v4 elsewhere); profile id is a TOML-supplied label |
| `ElementId` | newtype `String`, formatted `<hwnd_hex>:<runtime_id_hex>` | Pattern `^-?0x[0-9a-fA-F]+:[0-9a-fA-F]+$`; `parse()` / `parts()` / `try_from(String)` |
| `ElementIdParts` | `{ hwnd: i64, runtime_id_hex: String }` | |
| `ElementIdParseError` | enum `MissingSeparator` \| `InvalidHwnd` \| `InvalidRuntimeId` | thiserror |

ID generators (return `String`): `new_session_id()` (uuid v7), `new_reflex_id()` (v7), `new_subscription_id()` (v7), `element_id(hwnd: i64, runtime_id_hex: &str)`, `entity_id(track: u64)` (returns `"track:{track}"`).

### 5.2 Actions

```rust
pub enum Action {
    KeyPress { key: Key, hold_ms: u32, backend: Backend },
    KeyDown { key: Key, backend: Backend },
    KeyUp { key: Key, backend: Backend },
    KeyChord { keys: Vec<Key>, hold_ms: u32, backend: Backend },
    TypeText { text: String, dynamics: KeystrokeDynamics, backend: Backend },
    MouseMove { to: MouseTarget, curve: AimCurve, duration_ms: u32, backend: Backend },
    MouseMoveRelative { dx: f32, dy: f32, backend: Backend },
    MouseButton { button: MouseButton, action: ButtonAction, hold_ms: u32, backend: Backend },
    MouseDrag { from: Point, to: Point, button: MouseButton, curve: AimCurve, duration_ms: u32, backend: Backend },
    MouseScroll { dy: i32, dx: i32, at: Option<Point>, backend: Backend },
    PadButton { pad: PadId, button: PadButton, action: ButtonAction, hold_ms: u32 },
    PadStick { pad: PadId, stick: Stick, x: f32, y: f32 },
    PadTrigger { pad: PadId, trigger: Trigger, value: f32 },
    PadReport { pad: PadId, report: GamepadReport },
    AimAt { target: AimTarget, style: AimStyle, deadline_ms: u32, backend: Backend },
    Combo { steps: Vec<ComboStep>, backend: Backend },
    ReleaseAll,
}
```

`#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]`.

Supporting types:

| Type | Definition |
|---|---|
| `AimCurve` | `Instant` \| `Linear` \| `EaseInOut` \| `Bezier { p1: (f32, f32), p2: (f32, f32) }` \| `Natural { params: AimNaturalParams }` |
| `AimNaturalParams` | `{ control_point_jitter, tremor_stddev_px, overshoot_prob, overshoot_factor_range: (f32, f32), micro_correct_steps: u8, timing_stddev_ms, seed: Option<u64> }`. `FAST` preset: `(0.08, 0.2, 0.25, (1.02, 1.06), 1, 1.5, None)` — pinned by impplan as the default for every tool/profile/reflex (OQ-004 DECIDED 2026-05-22) |
| `AimStyle` | `Snap` \| `Flick` \| `Natural` \| `Track` |
| `KeystrokeDynamics` | `Burst` \| `Linear { ms_per_char: u32 }` \| `Natural { params: KeystrokeNaturalParams }` |
| `KeystrokeNaturalParams` | `{ mean_iki_ms: f32, stddev_ms: f32, bigram_bias: bool }`. `FAST` preset: `{ mean_iki_ms: 32.0, stddev_ms: 10.0, bigram_bias: true }` (~190 WPM) |
| `Key` | `{ code: KeyCode, use_scancode: bool }` |
| `KeyCode` | `Named { value: String }` \| `Symbol { value: char }` \| `HidCode { value: u8 }` |
| `MouseButton` | `Left` \| `Right` \| `Middle` \| `X1` \| `X2` |
| `ButtonAction` | `Press` \| `Down` \| `Up` |
| `MouseTarget` | `Screen { point }` \| `Element { element_id }` |
| `AimTarget` | `Screen { point }` \| `Element { element_id }` \| `Track { track_id: u64 }` |
| `PadId` | `u8` |
| `GamepadController` | `X360` (default) \| `Ds4` |
| `PadButton` | A/B/X/Y/Lb/Rb/Ls/Rs/Back/Start/Up/Down/Left/Right/Guide |
| `Stick` | `Left` \| `Right` |
| `Trigger` | `Left` \| `Right` |
| `GamepadReport` | `{ controller, buttons: Vec<PadButton>, thumb_l: (f32,f32), thumb_r: (f32,f32), lt: f32 (0..1), rt: f32 (0..1) }` with `neutral(controller)` ctor |
| `ComboStep` | `{ at_ms: u32, input: ComboInput }` |
| `ComboInput` | `KeyDown` / `KeyUp` / `KeyPress { hold_ms: u16 }` / `MouseButton` / `MouseMoveRel { dx: f32, dy: f32 }` / `PadButton` / `PadStick` (all with `key`/`button`/`pad` etc. fields) |

### 5.3 Perception observation

```rust
pub struct Observation {
    pub seq: u64,
    pub at: DateTime<Utc>,
    pub mode: PerceptionMode,
    pub foreground: ForegroundContext,
    pub focused: Option<FocusedElement>,
    pub elements: Vec<AccessibleNode>,
    pub entities: Vec<DetectedEntity>,
    pub hud: HudReadings,
    pub audio: AudioContext,
    pub recent_events: Vec<EventSummary>,
    pub clipboard_summary: Option<ClipboardSummary>,
    pub fs_recent: Vec<FsEvent>,
    pub diagnostics: ObservationDiagnostics,
}
```

Sub-structs:

| Struct | Fields |
|---|---|
| `PerceptionMode` | `A11yOnly` \| `PixelOnly` \| `Hybrid` \| `Auto` |
| `ForegroundContext` | `hwnd: i64`, `pid: u32`, `process_name`, `process_path`, `window_title`, `window_bounds: Rect`, `monitor_index: u32`, `dpi_scale: f32`, `profile_id: Option<ProfileId>`, `steam_appid: Option<u32>`, `is_fullscreen: bool`, `is_dwm_composed: bool` |
| `FocusedElement` | `element_id`, `name`, `role`, `automation_id: Option<String>`, `bbox: Rect`, `enabled`, `patterns: Vec<UiaPattern>`, `value: Option<String>`, `selected_text: Option<String>` |
| `UiaPattern` | Invoke / Toggle / Value / Selection / ExpandCollapse / Scroll / Text / Window / Transform / RangeValue |
| `AccessibleNode` | `element_id`, `parent: Option<ElementId>`, `name`, `role`, `automation_id: Option<String>`, `bbox: Rect`, `enabled`, `focused`, `patterns: Vec<UiaPattern>`, `children_count: u32`, `depth: u32` |
| `AccessibleSubtree` | `{ root: ElementId, nodes: Vec<AccessibleNode>, max_depth: u32, truncated: bool }` |
| `AccessibleQuery` | `{ role, name_substring, automation_id, scope: AccessibleQueryScope }` |
| `AccessibleQueryScope` | `FocusedSubtree` (default) \| `ForegroundWindow` \| `Global` |
| `DetectedEntity` | `entity_id`, `track_id: u64`, `class_label`, `bbox`, `confidence: f32`, `first_seen_at`, `last_seen_at`, `velocity_px_per_s: Option<(f32, f32)>` |
| `Detection` | `class_label`, `bbox`, `confidence`, `track_id: Option<u64>` |
| `DetectionBatch` | `model_id`, `frame_seq`, `inferred_at`, `items: Vec<Detection>` |
| `HudReadings` | `{ by_name: BTreeMap<String, HudReading> }` |
| `HudReading` | `{ raw_text, parsed: HudValue, confidence, stale_ms }` |
| `HudValue` | untagged `Number(f64)` \| `Text(String)` \| `Enum(String)` \| `Null` |
| `AudioContext` | `rms_db: f32`, `vad_speech_recent: bool`, `recent_events: Vec<AudioEvent>`, `direction_estimate: Option<DirectionEstimate>` |
| `AudioEvent` | `at`, `kind: String`, `azimuth_deg: Option<f32>`, `confidence` |
| `DirectionEstimate` | `azimuth_deg: f32`, `confidence: f32` |
| `ClipboardSummary` | `formats: Vec<String>`, `text_len: Option<u32>`, `text_excerpt: Option<String>`, `redacted: bool` |
| `FsEvent` | `at`, `path`, `kind: FsEventKind` (Created/Modified/Deleted/Renamed), `size_bytes: Option<u64>` |
| `ObservationDiagnostics` | `assembled_in_ms`, `sensor_latency_ms: BTreeMap<String, f32>`, `a11y_enabled`, `pixel_enabled`, `audio_enabled`, `a11y_status: SensorStatus`, `capture_status`, `detection_status`, `audio_status`, `elements_truncated`, `entities_truncated`, `size_bytes`, `size_estimate_tokens` |
| `SensorStatus` | `Healthy` \| `DegradedLatency { last_p99_ms: f32 }` \| `DegradedSensorFailed { reason_code: String }` \| `Disabled` \| `Unavailable` (default) |

### 5.4 OCR

| Type | Fields |
|---|---|
| `OcrBackend` | `Winrt` \| `Crnn` \| `Auto` (default) |
| `OcrResult` | `{ full_text: String, words: Vec<OcrWord>, confidence: f32, region: Rect, lang: String }` |
| `OcrWord` | `{ text: String, bbox: Rect, confidence: f32 }` |

### 5.5 Profiles

```rust
pub struct Profile {
    pub id: ProfileId,
    pub label: String,
    pub version: String,
    pub use_scope: ProfileUseScope,
    pub matches: Vec<ProfileMatch>,
    pub mode: PerceptionMode,
    pub capture: ProfileCapture,
    pub detection: ProfileDetection,
    pub ocr: ProfileOcr,
    pub hud: Vec<HudFieldSpec>,
    pub keymap: BTreeMap<String, String>,
    pub backends: ProfileBackends,
    pub event_extensions: Vec<EventExtension>,
}
```

Supporting types:

| Type | Definition |
|---|---|
| `ProfileMatch` | `{ exe, title_regex, steam_appid, window_class, process_args }` (all `Option`/`Vec`) |
| `ProfileUseScope` | `Productivity` / `SinglePlayer` / `OperatorOwnedTest` / `SanctionedResearch` / `Unknown` |
| `ProfileCapture` | `{ target: ProfileCaptureTarget, min_update_interval_ms: u32, cursor_visible: bool }` |
| `ProfileCaptureTarget` | `ForegroundWindow` \| `PrimaryMonitor` \| `MonitorIndex { index: u32 }` |
| `ProfileDetection` | `{ model_id, classes_of_interest: Vec<String>, confidence_threshold: f32, max_detections: u32 }` |
| `ProfileOcr` | `{ default_backend: OcrBackend, regions: Vec<HudRegion>, parser_config: BTreeMap<String, String> }` |
| `HudFieldSpec` | `{ name, region: HudRegion, extractor: HudExtractor, parser: HudParser }` |
| `HudRegion` | `Absolute { x, y, w, h }` \| `FractionOfWindow { x, y, w, h }` (f32) \| `AnchoredToEdge { edge: WindowEdge, x_offset, y_offset, w, h }` |
| `WindowEdge` | `TopLeft` / `TopRight` / `BottomLeft` / `BottomRight` |
| `HudExtractor` | `WinrtOcr` \| `Crnn { model_id }` \| `TemplateMatch { templates }` \| `ColorRatio { sample_points: Vec<(i32, i32)>, mapping }` |
| `HudParser` | `Number` \| `FractionNumerator` \| `FractionDenominator` \| `Regex { pattern, group }` \| `Enum { mapping }` |
| `ProfileBackends` | `{ default, keyboard_default, mouse_default, pad_default: Backend }` |
| `EventExtension` | `{ name, from_filter: EventFilter, emits_kind }` |

### 5.6 Reflex

| Type | Definition |
|---|---|
| `ReflexRegistration` | `{ id: ReflexId, kind: ReflexKind, priority: u32 (default 100), lifetime: ReflexLifetime, exclusive: bool }` |
| `ReflexKind` | `AimTrack { target: AimTarget, axis: ReflexAimAxis, gain, deadzone_px, max_speed_px_per_ms, curve_per_step: AimCurve, backend }` \| `HoldMove { keys, backend, re_assert }` \| `HoldButton { button: ReflexButtonTarget, backend }` \| `Combo { steps, backend }` \| `OnEvent { when: EventFilter, then: ReflexThen, debounce_ms }` |
| `ReflexAimAxis` | `Xy` \| `XOnly` \| `YOnly` |
| `ReflexButtonTarget` | `Mouse { button }` \| `Pad { pad, button }` |
| `ReflexThen` | `Action { action }` \| `Actions { actions: Vec<Action> }` \| `Combo { steps, backend }` |
| `ReflexLifetime` | `UntilCancelled` (default) \| `OneShot` \| `Duration { ms }` \| `UntilEvent { filter }` \| `UntilDeadline { ms }` |
| `ReflexState` | `Active` \| `Paused` \| `Cancelled` \| `Expired` \| `Disabled` \| `Starved` |
| `ReflexStatus` | `{ id, kind_summary, state, registered_at, last_fired_at: Option, fire_count: u64, priority, lifetime, exclusive, last_error_code: Option }` |

### 5.7 Events and filtering

```rust
pub struct Event {
    pub seq: u64,
    pub at: DateTime<Utc>,
    pub source: EventSource,
    pub kind: String,
    pub data: serde_json::Value,
    pub correlations: Vec<EventRef>,
}
```

| Type | Definition |
|---|---|
| `EventSource` | `A11yUia` / `A11yWinEvent` / `A11yCdp` / `Perception` / `PerceptionDetection` / `PerceptionHud` / `PerceptionAudio` / `Filesystem` / `Process` / `Clipboard` / `ActionEmitter` / `Reflex` / `System` |
| `EventRef` | `{ seq: u64, relation: String }` |
| `EventSummary` | `{ seq, at, source, kind, data_excerpt: serde_json::Value }` (built via `Event::summary()`) |
| `EventFilter` | `{ op: "all" / "none" / "kind" / "source" / "and" / "or" / "not" / "data" }` recursive; serde tag = `"op"`; depth limit `EVENT_FILTER_MAX_DEPTH = 8` |
| `EventFilterValidationError` | `EmptyAnd` \| `EmptyOr` \| `DepthExceeded { depth, max_depth }` |
| `DataPredicate` | `Eq` / `Ne` / `Lt` / `Le` / `Gt` / `Ge` / `Regex { pattern }` / `InSet { values }` / `Exists` |

`EventFilter::matches(&Event) -> bool` delegates to `crate::filter::matches_event_filter` which dispatches per op:
- `All` → true, `None` → false
- `Kind { kind }` → `event.kind == *kind`
- `Source { source }` → `event.source == *source`
- `And { args }` → `args.iter().all(...)`, `Or { args }` → `.any(...)`, `Not { arg }` → negate
- `Data { path, predicate }` → `predicate.matches(event.data.pointer(path))`

`DataPredicate::matches`: `Exists` is `value.is_some()`; comparison ops use `compare_values` (number/string lexicographic); `Regex` compiles per call and runs `is_match`; `InSet` is `values.iter().any(|v| v == actual)`.

Validation (`EventFilter::validate`): rejects empty `And`/`Or` and trees deeper than `EVENT_FILTER_MAX_DEPTH = 8`. Returns `EventFilterValidationError`, mapped by callers to `REFLEX_FILTER_INVALID`/`TOOL_PARAMS_INVALID`.

### 5.8 Health

```rust
pub struct Health {
    pub ok: bool,
    pub version: String,         // env!("CARGO_PKG_VERSION") = "0.1.0"
    pub build: String,           // option_env!("VERGEN_GIT_SHA") or "dev"
    pub uptime_s: u64,           // monotonic via Instant::elapsed
    pub subsystems: BTreeMap<String, SubsystemHealth>,
}
```

`SubsystemHealth` (open-ended; only `Some(_)` fields are serialized):

```rust
pub struct SubsystemHealth {
    pub status: String,
    pub detail: Option<String>,
    pub active_profile_id: Option<ProfileId>,
    pub db_path: Option<String>,
    pub schema_version: Option<u32>,
    pub cf_sizes: Option<BTreeMap<String, u64>>,
    pub active_count: Option<usize>,
    pub last_tick_jitter_us: Option<u64>,
    pub recursion_clamps_total: Option<u64>,
    pub profile_count: Option<usize>,
    pub last_reload_at: Option<String>,
    pub device_name: Option<String>,
    pub ring_buffer_seconds: Option<u32>,
    pub stt_model_loaded: Option<bool>,
    pub bind_addr: Option<String>,
    pub active_sessions: Option<usize>,
    pub sse_subscribers: Option<usize>,
}
```

Subsystem status strings emitted by `synapse-mcp/src/server.rs`:

| Subsystem | Status values |
|---|---|
| `storage` | `initializing` \| `ok` \| `error` \| `disk_pressure_l1..4` |
| `reflex` | `initializing` \| `ok` \| `degraded_latency` \| `disabled` \| `error` |
| `profiles` | `initializing` \| `ok` \| `error` |
| `audio` | `initializing` \| `ok` \| `disabled` \| `error` |
| `http` | `disabled` (stdio mode) \| `ok` (http mode) \| `error` |

### 5.9 Stored persistence variants

Used as the JSON payload values in RocksDB column families. See [04_storage_layer.md §4.1](#file-04) for table.

- `StoredEvent` (CF_EVENTS)
- `StoredObservation` (CF_OBSERVATIONS)
- `StoredReflexAudit` + `StoredReflexStep` (CF_REFLEX_AUDIT)
- `StoredSession` + `StoredProfileHistoryEntry` (CF_SESSIONS)
- `StoredRedaction` reused across the above (`{ kind: String, offset: u32, len: u32 }`)

Every stored type carries `schema_version: u32` so a future migration framework can branch on version. The current code unconditionally writes `synapse_core::SCHEMA_VERSION = 1`.

## 6. Serde conventions

| Rule | Applied via | Why |
|---|---|---|
| `deny_unknown_fields` | most structs | Forward-incompatible fields fail loudly rather than silently |
| `tag = "kind"` on enums | `Action`, `AimCurve`, `KeystrokeDynamics`, `KeyCode`, `MouseTarget`, `AimTarget`, `ComboInput`, `ProfileCaptureTarget`, `HudRegion`, `HudExtractor`, `HudParser`, `ReflexKind`, `ReflexButtonTarget`, `ReflexThen`, `ReflexLifetime` | Discriminated unions encode as `{ "kind": "...", ... }` for JSON ergonomics |
| `tag = "op"` on filter | `EventFilter`, `DataPredicate` | Same idea but the discriminant key is `"op"` |
| `rename_all = "snake_case"` (mostly) | most enums | Wire ergonomics |
| `rename_all = "lowercase"` | `Backend`, `MouseButton`, `ButtonAction`, `PadButton`, `Stick`, `Trigger`, `GamepadController`, `FsEventKind`, some action enums | Single-word variants stay terse |
| JSON schema generation | `schemars 1.2.1` derives `JsonSchema` on every public param/response | Auto-derived MCP `tools/list` schemas |

## 7. JSON Schema generators (non-derive)

- `normalized_axis_pair_schema` (`types.rs:312`): writes a 2-element array with each component in `[-1.0, 1.0]` for `GamepadReport::thumb_l`/`thumb_r`.
- `ElementId::json_schema` (`types.rs:498`): emits `{ "type": "string", "pattern": "^-?0x[0-9a-fA-F]+:[0-9a-fA-F]+$" }`.

## 8. Public utility functions

| Function | Source | Behavior |
|---|---|---|
| `element_id(hwnd: i64, runtime_id_hex: &str) -> ElementId` | `types.rs:555` | Formats hex hwnd (with `-0x` prefix if negative) then `:<runtime_id_hex>` |
| `entity_id(track: u64) -> EntityId` | `types.rs:565` | Returns `"track:{track}"` |
| `new_session_id() -> SessionId` | `types.rs:540` | uuid v7 string |
| `new_reflex_id() -> ReflexId` | `types.rs:545` | uuid v7 string |
| `new_subscription_id() -> SubscriptionId` | `types.rs:550` | uuid v7 string |
| `EventFilter::matches(&Event) -> bool` | `types.rs:1418` | delegates to `crate::filter::matches_event_filter` |
| `EventFilter::depth() -> u32` | `types.rs:1422` | recursive deepest path length |
| `EventFilter::validate()` / `validate_with_max_depth(max)` | `types.rs:1445` / `1455` | Validation of And/Or non-empty + depth bound |
| `DataPredicate::matches(Option<&serde_json::Value>) -> bool` | `types.rs:1517` | dispatches into `crate::filter::matches_data_predicate` |
| `Event::summary() -> EventSummary` | `types.rs:1342` | clones for SSE wire excerpts |
| `Rect::contains(point) -> bool` | `types.rs:405` | inclusive-left, exclusive-right semantics |
| `Point::distance_to(other) -> f64` | `types.rs:383` | `hypot(dx, dy)` |
| `GamepadReport::neutral(controller) -> Self` | `types.rs:294` | zero axes / no buttons / 0 triggers |

## 9. Tests (within crate)

`crates/synapse-core/tests/` contains 10 integration test files (~ each file covers one type family):

- `action_serde_proptest.rs`, `action_snapshots.rs`, `action_types.rs` — Action enum roundtrips
- `error_codes_literal.rs` — Each `error_codes::*` is exactly its name (no typos)
- `event_filter_types.rs` — EventFilter validation + matching
- `ocr_types.rs`, `profile_types.rs`, `reflex_types.rs`, `stored_types.rs`, `types.rs` — schema/roundtrip coverage
- `snapshots.rs` — insta-driven JSON snapshots

## 10. What is NOT covered

- **No runtime behavior.** `synapse-core` is pure data; it has no I/O, no async, no global state.
- **No backward-compatible deserialization.** `deny_unknown_fields` is everywhere; pre-v1 doctrine says schema bumps wipe-and-rebuild.
- **No re-export of subsystem error enums.** `synapse-core` defines only the `pub const` codes; concrete `thiserror` enums live next to their owning crate (see [01_system_overview.md §8](#file-01)).


---

<a id="file-06"></a>

> Source: `docs/systemspec/06_mcp_service_and_transports.md`

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

`Cli` (clap derive) with `mode: Mode { Stdio | Http }` plus the flags table in [03_configuration.md §2](#file-03). Constructor `Cli::m3_config()` builds an `M3ServiceConfig` (`m3.rs::M3ServiceConfig::from_cli_parts`) that also reads `SYNAPSE_BEARER_TOKEN`.

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

The disabling step persists `StoredReflexAudit` rows with `error_code = REFLEX_DISABLED_BY_OPERATOR` for every formerly-active reflex (see [04_storage_layer.md §4.2](#file-04)).

## 7. Shutdown semantics

| Trigger | Behavior |
|---|---|
| `Ctrl-C` / `Ctrl-Break` (Windows) | `wait_for_shutdown_signal` returns; daemon logs `MCP_SHUTDOWN_GRACEFUL`, cancels the rmcp service token + emitter token + connection-closed token, waits up to 1 s for the M2 emitter to flush, then calls `std::process::exit(0)`. |
| Stdio EOF | `CancelOnEofRead` flips `eof_seen` and cancels both tokens; the rmcp service exits naturally; emitter drains; daemon returns `ExitCode::SUCCESS`. |
| HTTP shutdown | `axum::serve(...).with_graceful_shutdown(shutdown_cancel.cancelled_owned())`. If the server doesn't stop in 2 s after cancel, `wait_for_server_stop` aborts the task and logs `MCP_HTTP_SHUTDOWN_TIMEOUT`. |
| HTTP bind error | If the bind address is non-loopback without `--allow-non-loopback`, the daemon exits `ExitCode::from(2)` with `HTTP_BIND_NON_LOOPBACK_REFUSED`. |
| Panic (debug only) | `install_panic_hook` from `synapse-action` + the telemetry crate hook capture the payload to logs; if the panic occurred during an `act_*` call, the operator hotkey path is still available because both panic hooks are installed before the service starts. |

## 8. Tool list snapshot

The full list of 30 declared tools is in [13_mcp_tool_reference.md](#file-13). They are: `health`, `observe`, `find`, `read_text`, `set_capture_target`, `set_perception_mode`, `act_click`, `act_type`, `act_press`, `act_aim`, `act_drag`, `act_scroll`, `act_pad`, `act_clipboard`, `release_all`, `subscribe`, `subscribe_cancel`, `reflex_register`, `reflex_cancel`, `reflex_list`, `reflex_history`, `profile_list`, `profile_activate`, `replay_record`, `audio_tail`, `audio_transcribe`, `storage_inspect`, `storage_put_probe_rows`, `storage_gc_once`, `storage_pressure_sample` — note the M3 set lives in `m3_tool_stubs()` (length-asserted at 15 in `instructions()`).


---

<a id="file-07"></a>

> Source: `docs/systemspec/07_reflex_runtime.md`

# 07 — Reflex Runtime (`synapse-reflex`)

Source files covered:
- `crates/synapse-reflex/src/lib.rs`
- `crates/synapse-reflex/src/audit.rs`
- `crates/synapse-reflex/src/bus.rs`
- `crates/synapse-reflex/src/conflict.rs`
- `crates/synapse-reflex/src/error.rs`
- `crates/synapse-reflex/src/scheduler.rs`
- `crates/synapse-reflex/src/scheduler_tick.rs`
- `crates/synapse-reflex/src/scheduler_combo.rs`
- `crates/synapse-reflex/src/scheduler_stats.rs`
- `crates/synapse-reflex/src/scheduler_windows.rs`
- `crates/synapse-reflex/src/kinds/{mod, aim_track, combo, hold_button, hold_lifetime, hold_move, on_event}.rs`

## 1. Crate role

`synapse-reflex` runs the **sub-frame reactive controllers** that turn streamed events and operator-registered intents into emitted `synapse_action::ActionHandle` calls without round-tripping through the MCP client. It also owns the in-process event bus consumed by the HTTP SSE bridge.

## 2. Public surface (`lib.rs` re-exports)

| Symbol | Source |
|---|---|
| `ReflexRuntime`, `ReflexCancelOutcome` | `lib.rs` |
| `write_audit` | `audit.rs` |
| `EventBus`, `EventBusError`, `EventBusResult`, `PublishReport`, `SUBSCRIBER_QUEUE_CAPACITY`, `SubscriberHandle`, `EVENTS_DROPPED_METRIC`, `DEFAULT_MAX_SUBSCRIPTIONS`, `DEFAULT_MAX_SUBSCRIPTIONS_NONZERO` | `bus.rs` |
| `REFLEX_STARVED_KIND`, `STARVATION_AFTER` | `conflict.rs` |
| `ReflexError`, `ReflexResult` | `error.rs` |
| `AimTrackContext`, `AimTrackController`, `AimTrackOutput`, `AimTrackParams`, `AimTrackTarget`, `DEFAULT_EMA_ALPHA`, `DEFAULT_MAX_SPEED_PX_PER_TICK`, `REFLEX_TRACK_LOST_KIND`, `ResolvedElementBox`, `TRACK_LOST_AFTER` | `kinds/aim_track.rs` |
| `ComboContext`, `ComboController`, `ComboOutput`, `ComboParams`, `ComboPhase`, `REFLEX_COMBO_COMPLETED_KIND` | `kinds/combo.rs` |
| `HoldButtonController`, `HoldButtonOutput`, `HoldButtonParams`, `HoldButtonPhase` | `kinds/hold_button.rs` |
| `HoldLifetimeContext`, `HoldReleaseReason`, `REFLEX_LIFETIME_EXPIRED_KIND` | `kinds/hold_lifetime.rs` |
| `HoldMoveController`, `HoldMoveOutput`, `HoldMoveParams`, `HoldMovePhase` | `kinds/hold_move.rs` |
| `MAX_ON_EVENT_FIRINGS_PER_TICK`, `REFLEX_FIRED_KIND`, `REFLEX_RECURSION_LIMIT_KIND` | `kinds/on_event.rs` |
| `DEFAULT_REFLEX_PRIORITY`, `MAX_REFLEX_PRIORITY`, `MAX_SCHEDULED_REFLEXES`, `REFLEX_TICK_LATE_KIND`, `ReflexScheduler`, `ScheduledReflex`, `SchedulerConfig`, `SchedulerHandle`, `SchedulerTrigger`, `TickSample`, `p99_jitter_us` | `scheduler.rs` |
| Event-kind name constants `REFLEX_CANCELLED_KIND = "reflex_cancelled"`, `REFLEX_DISABLED_KIND = "reflex_disabled_by_operator"`, `REFLEX_REGISTERED_KIND = "reflex_registered"` | `lib.rs` |

## 3. `ReflexRuntime`

```rust
pub struct ReflexRuntime {
    db: Arc<Db>,
    action_handle: ActionHandle,
    event_bus: EventBus,
    scheduler_config: SchedulerConfig,
    reflexes: Vec<ScheduledReflex>,
    disabled_reflex_ids: HashSet<ReflexId>,
    scheduler: Option<SchedulerHandle>,
}
```

Construction:

| Constructor | Default `SchedulerConfig` |
|---|---|
| `spawn(db, action_handle, event_bus)` | `SchedulerConfig::default()` |
| `spawn_with_config(db, action_handle, event_bus, scheduler_config)` | caller supplied |

`SchedulerConfig::default()` (`scheduler.rs:51`): `target_interval = 1 ms`, `fallback_interval = 2 ms`, `late_after = 2 ms`, `sample_limit = 4096`, `max_ticks = None`, `force_degraded = false`. `validate()` rejects zero intervals or zero sample_limit with `ReflexError::ParamsInvalid`.

### 3.1 `register(&ScheduledReflex)`

(`lib.rs:146`) Algorithm:

1. Check `reflex.priority <= MAX_REFLEX_PRIORITY (= 1000)`; else `ReflexError::PriorityInvalid`.
2. Clone the existing reflex list, append the new one, call `scheduler::validate_reflexes(&next)` — enforces `MAX_SCHEDULED_REFLEXES = 32`, unique reflex ids, and any cross-reflex constraints.
3. Spawn a fresh `ReflexScheduler` (`ReflexScheduler::spawn_with_audit_db`) with the new list and the same `scheduler_config`, sharing the `Arc<Db>` for audit persistence.
4. Replay disabled-state on the new scheduler so previously operator-disabled reflexes stay `Disabled` after the swap.
5. Replace the current scheduler. Stop the old one. (Hot-swap pattern: there is no "add reflex" channel into a live scheduler.)
6. Look up the new reflex's `ReflexStatus` from the new scheduler. Persist a `StoredReflexAudit` row with `details.kind = "reflex_registered"` (helper `write_registration_audit`), then `db.flush()`.
7. Return the `ReflexStatus`.

### 3.2 `cancel(reflex_id)`

(`lib.rs:198`) Algorithm:

1. Look up current status. Missing → `ReflexCancelOutcome::NotFound`.
2. Terminal states: `Expired` → `AlreadyExpired`, `Cancelled` → returns `Cancelled` (idempotent).
3. Tell the scheduler to cancel; failure to find at scheduler layer → `NotFound`.
4. Remove from `disabled_reflex_ids` (cancellation supersedes operator disable).
5. Look up the now-cancelled status, persist a `"reflex_cancelled"` audit row, flush, return `Cancelled { status }`.

### 3.3 `disable_all_by_operator()`

(`lib.rs:245`) Called only by `synapse-mcp/src/safety.rs::handle_operator_hotkey`. Algorithm:

1. If no scheduler is alive (no reflex registered yet), return empty `Vec`.
2. `scheduler.disable_all_reflexes()` flips every reflex to `Disabled` and returns the affected statuses.
3. Track ids in `disabled_reflex_ids` so subsequent re-registration (which spawns a fresh scheduler) preserves disable.
4. Persist one `StoredReflexAudit` per disabled status with `details.kind = "reflex_disabled_by_operator"`, `error_code = REFLEX_DISABLED_BY_OPERATOR`, `details.reason = "operator_hotkey"`. Flush.

### 3.4 `statuses()` / `list(include_expired)` / `history(reflex_id, limit)`

`statuses()` returns `scheduler.statuses()` (or empty if no scheduler).

`list(include_expired)`:
- Always include non-terminal scheduler statuses.
- If `include_expired = true`, also call `terminal_statuses_from_audit()`, which scans `CF_REFLEX_AUDIT`, groups rows by `reflex_id`, sorts each group by `(ts_ns, audit_id)`, and reconstructs a `ReflexStatus` from registration + fire-count + terminal audit (`reflex_cancelled` or expired). Final-state rows from the same reflex id in both the live scheduler and the audit log are deduplicated (the live row wins).
- Returns `ReflexResult<Vec<ReflexStatus>>`.

`history(reflex_id, limit)` (`lib.rs:311`):
- `limit == 0` returns empty.
- `db.flush()` first (so any uncommitted audit batches are durable before scan).
- If `reflex_id` is `Some`, `db.scan_cf_prefix(CF_REFLEX_AUDIT, b"<reflex_id>:")`; else `db.scan_cf(CF_REFLEX_AUDIT)`.
- Decode each value as `StoredReflexAudit`. Sort by `(ts_ns desc, audit_id desc, reflex_id desc)`. Truncate to `limit`.

### 3.5 Health-feeder accessors

| Method | Source | Returned |
|---|---|---|
| `storage_path()` | `lib.rs:361` | `&Path` for `Db.path` |
| `schema_version()` | `lib.rs:368` | `u32` (= `synapse_core::SCHEMA_VERSION`) |
| `storage_pressure_level()` | `lib.rs:375` | `DiskPressureLevel` |
| `storage_cf_sizes()` | `lib.rs:385` | `BTreeMap<String, u64>` from `Db::cf_sizes` |
| `active_count()` | `lib.rs:392` | count of statuses with state `Active` |
| `last_tick_jitter_us()` | `lib.rs:402` | `Option<u64>` from latest `TickSample` |
| `degraded_latency()` | `lib.rs:411` | true if the last sample was `degraded || late` |
| `recursion_clamps_total()` | `lib.rs:424` | counts audit rows whose `error_code == REFLEX_RECURSION_LIMIT` |
| `action_handle()` | `lib.rs:447` | `&ActionHandle` |
| `event_bus()` | `lib.rs:455` | `&EventBus` |

## 4. Audit persistence

`audit.rs::write_audit(db, &StoredReflexAudit)` JSON-encodes the audit and writes one row into `CF_REFLEX_AUDIT` keyed by `"{reflex_id}:{audit_id}"`. The audit_id is a fresh uuid v7, so per-reflex prefix iteration returns rows in registration order.

Audit kinds emitted by the runtime/scheduler (the discriminant is `details.kind` inside the audit payload):

| Kind constant | Emitter | Pairs with status | Error code |
|---|---|---|---|
| `REFLEX_REGISTERED_KIND = "reflex_registered"` | `ReflexRuntime::register` | `Active` | — |
| `REFLEX_CANCELLED_KIND = "reflex_cancelled"` | `ReflexRuntime::cancel` | `Cancelled` | — |
| `REFLEX_DISABLED_KIND = "reflex_disabled_by_operator"` | `ReflexRuntime::disable_all_by_operator` | `Disabled` | `REFLEX_DISABLED_BY_OPERATOR` |
| `REFLEX_FIRED_KIND = "reflex_fired"` | `kinds::on_event::publish_fired` | `Active` | — |
| `REFLEX_RECURSION_LIMIT_KIND = "reflex_recursion_limit"` | `OnEventTickGuard::report_limit_once` | `Active` | `REFLEX_RECURSION_LIMIT` |
| `REFLEX_TICK_LATE_KIND = "reflex_tick_late"` | scheduler when `Δ > late_after` | `Active` | — |
| `REFLEX_STARVED_KIND = "reflex_starved"` | conflict resolver after `STARVATION_AFTER` ticks | `Starved` | `REFLEX_STARVED` |
| `REFLEX_LIFETIME_EXPIRED_KIND = "reflex_lifetime_expired"` | hold-lifetime check | `Expired` | `REFLEX_LIFETIME_EXPIRED` |
| `REFLEX_TRACK_LOST_KIND` | aim-track after `TRACK_LOST_AFTER` of no resolution | `Active` (still) but emits event | `REFLEX_TRACK_LOST` |
| `REFLEX_COMBO_COMPLETED_KIND` | combo controller after last step | `Active` (or `Expired` for OneShot lifetime) | — |

## 5. Event bus (`bus.rs`)

```rust
pub struct EventBus { inner: Arc<EventBusInner> }
struct EventBusInner {
    subscribers: ArcSwap<Vec<Arc<Subscriber>>>,
    updates: Mutex<()>,
    max_subscriptions: NonZeroUsize,
}
struct Subscriber {
    id: SubscriptionId,
    filter: EventFilter,
    kinds: BTreeSet<String>,
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    lossy: Arc<AtomicBool>,
    dropped_since_read: Arc<AtomicU64>,
}
```

Subscribers use a **per-subscriber bounded crossbeam channel**. When a publisher's `try_send` fills the channel, it sets the `lossy` flag and increments `dropped_since_read`, then drops the event. Subscribers read non-blockingly via `SubscriberHandle::drain()` / `take_dropped_since_read()` / `take_lossy()` from the HTTP SSE state (`crates/synapse-mcp/src/http/sse.rs::sync_subscription`).

Constants:

| Constant | Value | Purpose |
|---|---|---|
| `SUBSCRIBER_QUEUE_CAPACITY` | `4096` | Per-subscriber bounded channel size |
| `DEFAULT_MAX_SUBSCRIPTIONS` | `64` | Default cap on simultaneous subscribers |
| `EVENTS_DROPPED_METRIC` | `"events_dropped_for_subscriber"` | Prometheus counter name |

`EventBus::subscribe(filter, kinds, snapshot_first) -> EventBusResult<SubscriberHandle>`:
1. Validate the filter via `EventFilter::validate()` → `EventBusError::FilterInvalid` on failure.
2. Check `subscribers.load().len() < max_subscriptions` → `SubscriptionCapReached { limit }`.
3. Build a bounded channel, register the new subscriber.

`EventBus::publish(Event) -> PublishReport`:
- Snapshot the `ArcSwap` once, iterate, evaluate `EventFilter::matches(event)`, then check the per-subscriber `kinds` allow-list.
- Per match: increment `matched`. `try_send` → if Ok, `queued += 1`; if `TrySendError::Full`, increment `dropped_since_read` + `lossy.store(true)` + `dropped += 1`; if `Disconnected`, schedule unsubscribe.

`EventBus::unsubscribe(id)` re-snapshots and replaces with the filtered vec under `updates` guard.

## 6. Reflex scheduler

`scheduler.rs` orchestrates ticks across all registered reflexes on a dedicated thread (Windows: `scheduler_windows.rs` raises priority to `TIME_CRITICAL`; portable path: `scheduler_tick.rs`).

### 6.1 `SchedulerConfig`

| Field | Default | Constraint |
|---|---|---|
| `target_interval` | `1 ms` | non-zero |
| `fallback_interval` | `2 ms` | non-zero |
| `late_after` | `target_interval * 2` | — |
| `sample_limit` | `4096` | non-zero |
| `max_ticks` | `None` | tests can bound the loop |
| `force_degraded` | `false` | when true, scheduler reports `degraded=true` on every sample regardless |

### 6.2 Per-tick algorithm (`scheduler_tick::tick`)

For each scheduled tick:

1. Capture `now = Instant::now()`. Record `jitter_us = (actual_interval - target_interval).max(0).as_micros() as u64` (capped at `u64::MAX`).
2. Flag `late = (actual_interval > late_after)`. Flag `degraded = late || config.force_degraded`.
3. Build an `OnEventTickGuard` to enforce `MAX_ON_EVENT_FIRINGS_PER_TICK = 4` across all `OnEvent` reflexes for this tick.
4. For each active reflex (in priority order, ascending):
   - Call its driver (`AimTrackController::step`, `HoldMoveController::step`, `HoldButtonController::step`, `ComboController::step`, or the `OnEvent` resolver). Each step returns an emission set (`Vec<Action>`), state updates, and lifecycle decisions.
   - Apply the conflict resolver (`conflict.rs`): if two reflexes both want to emit to the same exclusive resource and one has lower priority, mark the loser `Starved`. After `STARVATION_AFTER` ticks of consecutive loss, persist a `REFLEX_STARVED` audit row and flip state to `Starved`.
   - Lifetime check (`hold_lifetime.rs`): `Duration { ms }` and `UntilDeadline { ms }` use the registration time; `OneShot` expires after the first fire; `UntilEvent { filter }` watches the inbound stream; expiry persists `REFLEX_LIFETIME_EXPIRED` audit.
   - For successful fires, push the actions into `action_handle` (the shared M2 emitter producer).
5. Push a `TickSample { jitter_us, degraded, late, t: now }` into the bounded sample ring (`sample_limit`). If `late`, persist a `REFLEX_TICK_LATE_KIND` audit row tagged with the offending reflex (if any).
6. Update Prometheus metrics: `reflex_fires_total{kind, reflex_id}`, `reflex_tick_jitter_us` histogram, `reflex_recursion_clamps_total`, `reflex_starved_total`.

`TickSample` + `p99_jitter_us(samples: &[TickSample]) -> u64` is the basis for `synapse-mcp` reporting `degraded_latency` and the `REFERENCE_REFLEX_TICK_JITTER_IDLE_P99_US = 200` budget tested in `crates/synapse-reflex/benches/reflex_tick_jitter_idle.rs`.

### 6.3 Trigger types

`SchedulerTrigger` (declared inside `scheduler.rs`) drives when a reflex's controller is invoked:

| Trigger | Behavior |
|---|---|
| Every-tick (e.g., `AimTrack`, `HoldMove`, `HoldButton`) | Step runs every tick |
| Combo step deadline | `scheduler_combo.rs` fires next combo step at `at_ms` after start |
| Event-bus subscriber (`OnEvent`) | Drains the per-reflex `SubscriberHandle` and runs the `then` actions for each matching event up to the per-tick guard |

### 6.4 ScheduledReflex shape

```rust
pub struct ScheduledReflex {
    pub reflex_id: ReflexId,
    pub trigger: SchedulerTrigger,
    pub then: Vec<Action>,            // canonical action list compiled from ReflexKind
    pub driver: ScheduledReflexDriver,// holds the controller state
    pub priority: u32,
    pub lifetime: ReflexLifetime,
    pub exclusive: bool,
}
```

`ScheduledReflex::on_event(reflex_id, EventFilter, actions)` is a convenience constructor used in `lib.rs` tests.

## 7. Reflex kinds

### 7.1 `AimTrack` (`kinds/aim_track.rs`)

- Constants: `DEFAULT_EMA_ALPHA`, `DEFAULT_MAX_SPEED_PX_PER_TICK`, `TRACK_LOST_AFTER`, `REFLEX_TRACK_LOST_KIND`.
- Maintains a smoothed target position (EMA with `DEFAULT_EMA_ALPHA`) and per-tick step toward it bounded by `max_speed_px_per_tick`.
- Axis lock (`Xy` / `XOnly` / `YOnly`) clamps off-axis deltas to zero.
- `deadzone_px` near the target suppresses output entirely.
- If the target resolver (`AimTrackTarget::Element` looks up via UIA on the action thread) cannot find the element for `TRACK_LOST_AFTER` consecutive ticks, publishes a `REFLEX_TRACK_LOST_KIND` event and records `REFLEX_TRACK_LOST` audit but does not cancel the reflex (lifetime semantics still apply).

### 7.2 `HoldMove` (`kinds/hold_move.rs`)

- Drives a held set of keys through `Action::KeyDown` followed by `Action::KeyUp` at expiry.
- `re_assert: bool` re-issues a `KeyDown` per tick (used for game engines that drop holds during scene loads).
- Phase: `Pressing` / `Holding` / `Releasing`.

### 7.3 `HoldButton` (`kinds/hold_button.rs`)

- Same shape but for `MouseButton` or `PadButton` targets via `ReflexButtonTarget`.

### 7.4 `Combo` (`kinds/combo.rs`)

- Owns the `Vec<ComboStep>` with `at_ms` offsets.
- Phase progression: `NotStarted` → `Running` → `Completed`.
- At each tick the controller compares `now - start_at` against the next step's `at_ms` and emits any due steps in order.
- On the final emission, publishes a `REFLEX_COMBO_COMPLETED_KIND` event. `OneShot` lifetime then marks the reflex `Expired`.

### 7.5 `OnEvent` (`kinds/on_event.rs`)

- Subscribes to the event bus with the reflex's `EventFilter`.
- Drains matched events each tick (subject to `OnEventTickGuard` cap of `MAX_ON_EVENT_FIRINGS_PER_TICK = 4`).
- For each fired event:
  - Publishes a `REFLEX_FIRED_KIND` event on the bus (with `EventSource::Reflex`, `data = { reflex_id, tick_index, trigger_event_id, actions: [...] }`).
  - Pushes the `then` actions into the action emitter.
  - Persists a `reflex_fired` audit row with `steps` populated from the action list.
- Debounce: `debounce_ms` between fires per-reflex via `OnEventState::allows_fire`.
- Recursion guard: if a single tick exceeds `MAX_ON_EVENT_FIRINGS_PER_TICK`, the guard publishes a `REFLEX_RECURSION_LIMIT_KIND` event and persists a `REFLEX_RECURSION_LIMIT` audit row exactly once per tick. Further events for that reflex are dropped this tick.

## 8. Error mapping (`error.rs`)

`ReflexError::code()` returns:

| Variant | Code |
|---|---|
| `CapReached { .. }` | `REFLEX_CAP_REACHED` |
| `KindInvalid { .. }` | `REFLEX_KIND_INVALID` |
| `ParamsInvalid { .. }` | `REFLEX_PARAMS_INVALID` |
| `TargetInvalid { .. }` | `REFLEX_TARGET_INVALID` |
| `FilterInvalid { .. }` | `REFLEX_FILTER_INVALID` |
| `PriorityInvalid { .. }` | `REFLEX_PRIORITY_INVALID` |
| `DisabledByOperator { .. }` | `REFLEX_DISABLED_BY_OPERATOR` |
| `Storage(error)` (forwards `synapse_storage::StorageError`) | uses inner code |
| (additional internal variants — see source) | per-variant mapping |

## 9. Integration with the rest of the daemon

| Edge | Direction | Mechanism |
|---|---|---|
| `SynapseService` → `ReflexRuntime` | sync (mcp tool calls) | `reflex_runtime()` helper lazily opens RocksDB + spawns runtime; `ensure_a11y_event_bridge` plumbs UIA events into the bus |
| `ReflexRuntime` → `synapse-action::ActionHandle` | async producer (mpsc) | `ActionHandle::execute` / `try_execute` push `Action` messages with the standard token-bucket rate limit applied downstream |
| `ReflexRuntime` → `EventBus` | sync `publish` | fire/starved/recursion/tick-late events |
| `EventBus` → SSE | per-subscription bounded channel | `crates/synapse-mcp/src/http/sse.rs::sync_subscription` |
| `ReflexRuntime` → `synapse-storage::Db` | sync `put_batch` + `flush` | every register/cancel/disable/fire writes one `StoredReflexAudit` to `CF_REFLEX_AUDIT` |
| Operator hotkey → `ReflexRuntime::disable_all_by_operator` | callback on hook thread | `crates/synapse-mcp/src/safety.rs::handle_operator_hotkey` |

## 10. Observability constants and metrics

The reflex subsystem feeds these metrics (defined in `crates/synapse-telemetry/src/metrics.rs`):

| Metric | Kind | Labels | Description |
|---|---|---|---|
| `events_published_total` | counter | `source`, `kind` | every `EventBus::publish` |
| `events_dropped_for_subscriber` | counter | `subscription_id` | per-subscriber overflow |
| `reflex_fires_total` | counter | `kind`, `reflex_id` | scheduler fires |
| `reflex_tick_jitter_us` | histogram | — | scheduler tick jitter |
| `reflex_recursion_clamps_total` | counter | — | OnEventTickGuard clamps |
| `reflex_starved_total` | counter | `reflex_id` | starvation events |

## 11. What is NOT covered

- **No public scheduler tick API.** External callers cannot drive ticks manually outside test harnesses (`max_ticks` in `SchedulerConfig` is the only test affordance).
- **No reflex priority migration.** Cancel-then-re-register is the only way to change priority for a live reflex.
- **No cross-process bus.** `EventBus` is in-memory; the only cross-process delivery channel is HTTP SSE.
- **No transactional batching across reflexes.** Each fire writes its own audit row + flushes; concurrent registers serialize via the `Mutex<ReflexRuntime>` held by the M3 state.


---

<a id="file-08"></a>

> Source: `docs/systemspec/08_action_subsystem.md`

# 08 — Action Subsystem (`synapse-action`)

Source files covered:
- `crates/synapse-action/src/lib.rs`
- `crates/synapse-action/src/handle.rs`
- `crates/synapse-action/src/emitter.rs` (+ `emitter/{backends, dispatch, keyboard, lifecycle, rate_limits, routing, state, tests/}`)
- `crates/synapse-action/src/backend/mod.rs` (+ `backend/{software, vigem, recording, unavailable, mouse_coordinates, text_dispatch}`)
- `crates/synapse-action/src/click_timing.rs`
- `crates/synapse-action/src/clipboard.rs`
- `crates/synapse-action/src/curve.rs`
- `crates/synapse-action/src/dynamics.rs`
- `crates/synapse-action/src/error.rs`
- `crates/synapse-action/src/hotkey.rs`
- `crates/synapse-action/src/invoke.rs` (+ `invoke/{dispatch, resolver, tests}`)
- `crates/synapse-action/src/rate_limit.rs`
- `crates/synapse-action/src/safety.rs`
- `crates/synapse-action/src/validation.rs`

## 1. Architecture

The action subsystem is an actor-style emitter with an Tokio mpsc producer (`ActionHandle`) and a backend-dispatching consumer (`ActionEmitter`). Backends implement `ActionBackend::execute(&Action, &mut EmitState)`; the concrete one is chosen by `resolve_backend` based on the action's `backend` field plus the action kind for `Backend::Auto`.

### 1.1 Public re-exports (`lib.rs`)

| Symbol | Source |
|---|---|
| `ActionBackend`, `ResolvedBackend`, `resolve_backend` | `backend::mod` |
| `HardwareBackend` | `backend::hardware` |
| `RecordedInput`, `RecordingBackend` | `backend::recording` |
| `HardwareUnavailableBackend` | `backend::unavailable` |
| `VigemBackend` | `backend::vigem` |
| `DoubleClickTiming`, `cached_double_click_timing`, `initialize_double_click_timing_cache`, `inter_click_delay_ms_for_window` | `click_timing` |
| `ClipboardFormat`, `clear_clipboard`, `read_clipboard_text`, `write_clipboard_text` | `clipboard` |
| `sample_curve` | `curve` |
| `BIGRAMS`, `KeystrokeEvent`, `ModifierMask`, `sample_typing_schedule` | `dynamics` |
| `ActionEmitter`, `ActionEmitterSnapshotHandle`, `ActionSnapshotMessage`, `ActionStateSnapshot`, `Backends`, `EmitState`, `HardwareHidConfig`, `HELD_KEY_MAX_DURATION_MS` | `emitter` |
| `ActionError`, `ActionResult` | `error` |
| `ACTION_QUEUE_CAPACITY`, `ActionHandle`, `ActionMessage`, `RELEASE_ALL_HANDLE` | `handle` |
| `OperatorHotkeyGuard`, `install_operator_hotkey`, `operator_release_epoch`, `operator_release_requested_since` | `hotkey` |
| `CoordinateFallbackPlan`, `ElementClickOutcome`, `click_element_or_fallback`, `invoke_element` | `invoke` |
| `SOFTWARE_RATE_LIMIT_PER_S`, `TokenBucket`, `TokenBucketSnapshot`, `VIGEM_RATE_LIMIT_PER_S` | `rate_limit` |
| `install_panic_hook` | `safety` |
| `MAX_DRAG_DISTANCE_PX`, `validate_action` | `validation` |

## 2. `ActionHandle` (producer)

```rust
pub const ACTION_QUEUE_CAPACITY: usize = 256;
pub type ActionMessage = (Action, tokio::sync::oneshot::Sender<ActionResult<()>>);
pub static RELEASE_ALL_HANDLE: OnceLock<ActionHandle> = OnceLock::new();

pub struct ActionHandle { tx: mpsc::Sender<ActionMessage> }
```

| Method | Behavior |
|---|---|
| `channel()` | Builds `(ActionHandle, Receiver)` with bounded capacity `ACTION_QUEUE_CAPACITY = 256` |
| `execute(action) -> ActionResult<()>` (async) | `validate_action`, then send `(action, ack_tx)`, await `ack_rx`. Closed channel → `ACTION_BACKEND_UNAVAILABLE` |
| `try_execute(action)` | Same but `try_send` (no ack wait); full → `ACTION_QUEUE_FULL`, closed → `ACTION_BACKEND_UNAVAILABLE` |
| `fire_release_all_blocking_with_timeout(timeout)` | Synchronous send of `Action::ReleaseAll`, then busy-polls the ack channel on 1 ms sleeps until `timeout` elapses. Used by the operator hotkey to release inputs from a non-async hook thread. |

`RELEASE_ALL_HANDLE` is a process-global `OnceLock` set on first emitter spawn. `safety::handle_operator_hotkey` reads it (returning a structured "missing_handle" report if unset).

## 3. `ActionEmitter` (consumer actor)

`ActionEmitter::channel()` returns `(ActionHandle, ActionEmitterSnapshotHandle, ActionEmitter)`.

`ActionEmitter::run_with_shutdown_reason(shutdown_cancel, shutdown_reason, connection_closed_cancel)` is the main task. Tokio task spawn site is `crates/synapse-mcp/src/m2.rs::M2State::from_recording_backend_env_with_actor_backend`.

### 3.1 State machine

`EmitState` (`emitter::state` / `state.rs`) owns:

- A `BitSet` of held keys (the M2 source-of-truth for `release_all`/`auto-release-after-`HELD_KEY_MAX_DURATION_MS`) — reflex hold_* must enqueue through `ActionHandle`, not mutate this bitset.
- Held mouse-button set (`MouseButton` discriminant indices).
- Per-pad `GamepadReport` cache (last-emitted neutralizable state).
- Per-key timers for auto-release.

`ActionStateSnapshot` (exposed via `ActionEmitterSnapshotHandle::snapshot().await`) is the public read of `held_keys: Vec<Key>`, `held_buttons: Vec<MouseButton>`, `pad_state: HashMap<PadId, GamepadReport>`, `held_key_timer_count: usize`.

### 3.2 Per-message dispatch (`emitter::dispatch`)

For each `(Action, oneshot::Sender)` pulled from the channel:

1. **Validate.** `validate_action(&action)` (re-checked here even though `ActionHandle::execute` already validates, to defend against `try_execute` callers that bypassed it).
2. **Resolve backend.** `routing.rs` → `backend::resolve_backend(action.backend(), &action)`:
   - `Software` → `ResolvedBackend::Software`
   - `Vigem` → `ResolvedBackend::Vigem`
   - `Hardware` → `ResolvedBackend::Hardware`; the selected backend is `HardwareBackend` only when `synapse-mcp` was started with `--hardware-hid <port|auto>` and the HID connection/IDENTIFY succeeded. Otherwise the hardware slot is `HardwareUnavailableBackend`.
   - `Auto` → `ResolvedBackend::Vigem` for `Pad*` actions, `Software` for everything else
3. **Rate-limit.** `rate_limits.rs` consumes one token from the per-backend `TokenBucket`:
   - `SOFTWARE_RATE_LIMIT_PER_S = 5000`
   - `VIGEM_RATE_LIMIT_PER_S = 1000`
   - Out-of-tokens → `ActionError::RateLimited { retry_after_ms }` (the only variant that carries a hint for clients)
4. **Dispatch.** Backend-specific `execute(&action, &mut EmitState)`:
   - **`SoftwareBackend`** (`backend/software/*`): SendInput-based keyboard/mouse/text, see §4
   - **`VigemBackend`** (`backend/vigem/*`): X360/DS4 controller report via `vigem-client`
   - **`RecordingBackend`** (`backend/recording/*`): appends a `RecordedInput` to an in-memory log (used in tests and via `SYNAPSE_MCP_RECORDING_BACKEND=1`)
   - **`HardwareBackend`** (`backend/hardware/*`): serializes supported key, mouse-relative, pad, combo, and release commands through `synapse-hid-host::HidGateway`
   - **`HardwareUnavailableBackend`** (`backend/unavailable`): fail-closed response when hardware HID is not enabled, returning `ACTION_BACKEND_UNAVAILABLE` with `--hardware-hid <port|auto>` guidance
5. **Auto-release timers.** `emitter::keyboard` enforces `HELD_KEY_MAX_DURATION_MS` per held key — after the limit, the emitter inserts a synthetic `KeyUp` and emits a `STUCK_KEY_AUTO_RELEASED` warn-log + event.
6. **ReleaseAll**: walks `EmitState`, emits a `KeyUp` for each held key, `MouseButton::Up` for each held button, and a `GamepadReport::neutral` for each tracked pad. Reflexes that observe `Action::ReleaseAll` are also expected to expire any held-state controllers.
7. **Ack.** Send `Ok(())` or the `ActionError` back on the oneshot.

### 3.3 Lifecycle (`emitter::lifecycle`)

Loop selects on the mpsc receiver against `shutdown_cancel`/`connection_closed_cancel`. On cancellation:

1. Drain any remaining queued actions OR (in the standard path) flush a synthetic `Action::ReleaseAll` before exit with `shutdown_reason` recorded in tracing.
2. Send the final `ActionStateSnapshot` over the `watch::Sender<Option<ActionStateSnapshot>>` so the stdio loop's `wait_for_m2_emitter_done` can confirm clean drain.

## 4. Software backend (Windows only — `cfg(windows)`)

`backend/software/*` constructs `INPUT` structs and calls `SendInput`:

- **keyboard.rs** — KeyDown/Up/Press emit virtual-key or scancode-based INPUT_KEYBOARD events. `Key.use_scancode = true` toggles between the two.
- **mouse.rs** — MouseMove uses `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK` with normalized coordinates (`mouse_coordinates.rs`). `MouseScroll` uses `MOUSEEVENTF_WHEEL` / `_HWHEEL`.
- **input.rs** — shared INPUT struct preparation/zeroing.
- **text.rs** — `TypeText` lowers to a sampled `KeystrokeEvent` stream via `dynamics::sample_typing_schedule`. `KeystrokeDynamics::Natural` uses `KeystrokeNaturalParams::FAST` (mean 32 ms, stddev 10 ms, bigram-biased; ~190 WPM). `Linear` is constant `ms_per_char`; `Burst` is 0 ms.
- **utils.rs** — scancode/virtual-key conversion helpers.

`software_non_windows.rs` is the compile-stub: every `execute` returns `ACTION_BACKEND_UNAVAILABLE`.

`MouseMove`/`AimAt` traces use `sample_curve(curve, t, duration_ms)` (`curve.rs`) — supports `Instant`, `Linear`, `EaseInOut`, `Bezier { p1, p2 }`, and `Natural` which samples a control-point-jittered Bézier with overshoot probability and micro-corrections from `AimNaturalParams`.

`text_dispatch.rs` chooses between scancode synthesis and clipboard paste for `TypeText` (currently always scancode synthesis in M2; clipboard paste is the planned fallback for Unicode that doesn't fit IME).

## 5. ViGEm backend

`backend/vigem/*` (Windows only, requires the ViGEmBus driver — typically installed via `winget install Nefarius.ViGEmBus`):

- **client.rs** — `vigem_client::Client::connect()`; plug X360 or DS4 pads on demand (`feature = "unstable_ds4"`).
- **pad.rs** — `PadId` ↔ vigem-client target slot.
- **reports.rs** — `GamepadReport` → X360 wire blob `[buttons hi, buttons lo, lt, rt, lx_lo, lx_hi, ly_lo, ly_hi, rx_lo, rx_hi, ry_lo, ry_hi]` (or DS4 variant).
- **state.rs** — per-session pad cache.
- **error.rs** — maps ViGEm-client errors to `ActionError::VigemNotInstalled` / `ActionError::VigemPluginFailed`.

## 6. Recording backend

`RecordingBackend` wraps a `Mutex<Vec<RecordedInput>>`. `RecordedInput` variants mirror the live emit surface (`MouseMove { to, curve, duration_ms }`, `KeyDown`, `KeyUp`, `Press`, `Type`, `Scroll`, `PadReport`, `ReleaseAll`, etc.). Used by the M2 act_* test paths to compare emitted sequences without hitting the OS, and by `synapse-mcp` when `SYNAPSE_MCP_RECORDING_BACKEND=true`.

## 7. UIA Invoke bridge

`invoke.rs` + `invoke/dispatch.rs` + `invoke/resolver.rs`:

- `invoke_element(element_id) -> ActionResult<ElementClickOutcome>` resolves the UIA element via `synapse_a11y::re_resolve(ElementId)` and tries `InvokePattern::Invoke`. Returns `ElementClickOutcome::InvokedPattern` on success.
- `click_element_or_fallback(element_id, coord_plan: CoordinateFallbackPlan)` invokes if possible; otherwise falls back to a `MouseMove` to the element bbox center followed by a button click. Used by `act_click` with `use_invoke_pattern = true`.

## 8. Click timing

Windows reports `GetDoubleClickTime()` (default 500 ms). `click_timing.rs::initialize_double_click_timing_cache()` reads it once and caches `DoubleClickTiming { window_ms, inter_click_delay_ms, source }`. `inter_click_delay_ms_for_window(window_ms)` returns the inter-click delay used for multi-click sequences (`act_click.clicks ∈ 2..=3`). Cache hit available via `cached_double_click_timing()`.

## 9. Operator panic hotkey

`hotkey.rs`:

- `install_operator_hotkey(callback) -> ActionResult<OperatorHotkeyGuard>`: installs a low-level Win32 keyboard hook on a dedicated thread. Detects `Ctrl+Alt+Shift+P` and invokes `callback` once per press (debounced internally).
- Drops the guard → unhooks. The `synapse-mcp` `run_stdio` / `http::serve` retain the guard for the daemon's lifetime.
- Two atomics track epochs so consumers can correlate audit logs: `operator_release_epoch()` (monotonic counter incremented each fire) and `operator_release_requested_since(epoch)` (boolean).

## 10. Clipboard

`clipboard.rs`:

| Function | Behavior |
|---|---|
| `read_clipboard_text(format: ClipboardFormat) -> ActionResult<String>` | Opens the clipboard, fetches `CF_TEXT` or `CF_UNICODETEXT`, returns a `String` |
| `write_clipboard_text(format, text)` | Empties clipboard, allocates global memory, writes the text bytes |
| `clear_clipboard()` | `EmptyClipboard` |

`ClipboardFormat` is `Text` (CF_TEXT, ASCII only) \| `Unicode` (CF_UNICODETEXT). The tool surface (`act_clipboard`) enforces ASCII for `Text` format.

## 11. Curves and dynamics

| Function | Description |
|---|---|
| `sample_curve(curve, t_normalized: f32, duration_ms: u32) -> f32` | Returns a 0..=1 progress fraction. `Instant` → 1. `Linear` → `t`. `EaseInOut` → cubic ease. `Bezier { p1, p2 }` → cubic Bézier with caller-supplied control points. `Natural { params }` → sampled control-point-jittered Bézier with stochastic overshoot/micro-correction. |
| `sample_typing_schedule(text, dynamics: &KeystrokeDynamics) -> Vec<KeystrokeEvent>` | Builds the keystroke sequence + per-key timing. Burst → all keys at `t = 0`. Linear → constant `ms_per_char`. Natural → samples inter-key intervals from `Normal(mean_iki_ms, stddev_ms)`; if `bigram_bias`, looks up `BIGRAMS[(prev, curr)]` for digraph-specific biases. |
| `KeystrokeEvent` | `{ key: Key, at_ms: u32, modifiers: ModifierMask }` |
| `BIGRAMS` | static lookup table of digraph timings (e.g. "th", "in", "er", …) |

## 12. Rate limiting (`rate_limit.rs`)

```rust
pub const SOFTWARE_RATE_LIMIT_PER_S: u32 = 5000;
pub const VIGEM_RATE_LIMIT_PER_S: u32 = 1000;

pub struct TokenBucket {
    capacity: u32,
    tokens: AtomicU32,
    refill_rate_per_s: u32,
    last_refill: AtomicU64,    // nanos since process start
}
```

Algorithm:

1. `take(now_ns)`: compute `elapsed = now_ns - last_refill`, add `(elapsed * refill_rate) / 1_000_000_000` tokens (clamped to `capacity`), CAS `tokens -= 1`.
2. If `tokens == 0`, compute `retry_after_ms = ceil((1 - frac_tokens) / refill_rate * 1000)` and return `Err(ActionError::RateLimited { retry_after_ms })`.

`TokenBucket::for_backend(ResolvedBackend)` sets both capacity and rate from the per-backend constant (so the bucket can absorb up to one second of headroom).

## 13. Errors (`error.rs`)

| Variant | Code | Notes |
|---|---|---|
| `QueueFull { detail }` | `ACTION_QUEUE_FULL` | Bounded mpsc full |
| `RateLimited { detail, retry_after_ms }` | `ACTION_RATE_LIMITED` | Only error variant carrying retry hint |
| `BackendUnavailable { detail }` | `ACTION_BACKEND_UNAVAILABLE` | Emitter channel closed, unsupported feature, non-Windows stub |
| `TargetInvalid { detail }` | `ACTION_TARGET_INVALID` | Invalid `AimTarget`/`MouseTarget` resolution |
| `HoldExceededMax { detail }` | `ACTION_HOLD_EXCEEDED_MAX` | hold_ms > 30000 (per-tool guard) |
| `HidPortDisconnected { detail }` | `ACTION_HID_PORT_DISCONNECTED` | HID gateway disconnected, reconnecting, or timed out after startup |
| `VigemNotInstalled { detail }` | `ACTION_VIGEM_NOT_INSTALLED` | Driver missing |
| `VigemPluginFailed { detail }` | `ACTION_VIGEM_PLUGIN_FAILED` | vigem-client plug error |
| `ElementNotResolved { detail }` | `ACTION_ELEMENT_NOT_RESOLVED` | UIA re_resolve returned None |
| `ForegroundLost { detail }` | `ACTION_FOREGROUND_LOST` | last-observed hwnd ≠ current foreground hwnd at act_type |
| `UnsupportedKey { detail }` | `ACTION_UNSUPPORTED_KEY` | Unknown key name in `act_press.keys` |
| `DragDistanceExceedsLimit { detail }` | `ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT` | distance > `MAX_DRAG_DISTANCE_PX = 4096.0` |
| `StuckKeyAutoReleased { detail }` | `STUCK_KEY_AUTO_RELEASED` | Emitter forced KeyUp after `HELD_KEY_MAX_DURATION_MS` |
| `SafetyReleaseAllFired { detail }` | `SAFETY_RELEASE_ALL_FIRED` | reported when release_all races with another action |
| `SafetyOperatorHotkeyFired { detail }` | `SAFETY_OPERATOR_HOTKEY_FIRED` | reported when an action is enqueued after the hotkey fired this epoch |

## 14. Validation (`validation.rs`)

`validate_action(&Action)`:

- `MouseDrag { from, to, ... }`: distance check via `Point::distance_to` against `MAX_DRAG_DISTANCE_PX = 4096.0` → `ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT`. (Other invariants are enforced at tool-param level or per-backend dispatch.)

## 15. Safety (`safety.rs`)

`install_panic_hook()` (Once-guarded): installs a `std::panic::set_hook` that prints the panic payload + location and best-effort calls `RELEASE_ALL_HANDLE.get()?.try_execute(Action::ReleaseAll)`. Then chains to the previous hook. Used by `synapse-mcp/src/main.rs::run_stdio` before any rmcp service starts, so a panic in tool code still cleans up held inputs.

## 16. Integration with `synapse-mcp/src/m2/*`

Each M2 tool wrapper builds one or more `synapse_core::Action`s and dispatches through the `ActionHandle`:

| Tool | Built actions |
|---|---|
| `act_click` | `MouseMove { to, curve = Natural FAST or chosen, duration_ms = 50 default }` then 1–3 `MouseButton::Press`. For `Element` targets, uses `invoke_element` (UIA Invoke) with coordinate fallback. |
| `act_type` | `TypeText { text, dynamics, backend }`. Pre-call `ensure_act_type_foreground` compares hwnd against last `observe`'s `foreground.hwnd`; mismatch → `ACTION_FOREGROUND_LOST`. Optional `press_enter_after` appends `KeyPress { Key::Named("enter") }`. |
| `act_press` | Single `KeyPress` or `KeyChord` after parsing strings via `m2/press/keys.rs`. |
| `act_aim` | `MouseMove { to, curve = AimCurve::Natural { params: FAST }, duration_ms }`. Style → duration: Snap 50 ms, Flick 35 ms, Natural 150 ms, Track unsupported in M2. |
| `act_drag` | `MouseDrag { from, to, button, curve, duration_ms = 200 default }`. |
| `act_scroll` | `MouseScroll { dy, dx, at, backend }` either once or scheduled into N events at 30 ms (up to 120 steps) for smooth scrolling. |
| `act_pad` | `PadReport { pad: pad_id, report: GamepadReport }`. `hold_ms` schedules a return-to-neutral `PadReport` after the hold. |
| `act_clipboard` | Direct `read_clipboard_text` / `write_clipboard_text` / `clear_clipboard` (no Action enum traversal). |
| `release_all` | `m2/release_all.rs::release_all_with_handles`: snapshot before, `Action::ReleaseAll`, snapshot after with `ensure_drained` (empty held lists, no timers). |

## 17. What is NOT covered

- **Remaining hardware HID gaps.** The live `Backend::Hardware` path is enabled by `--hardware-hid <port|auto>` and maps Synapse keys to USB HID Keyboard/Keypad usage IDs, but modifier/6KRO handling (#395), shifted hardware text, absolute-mouse fallback (#396), and broader supported-use gates remain M4 work.
- **Modifiers on `act_click`.** The schema accepts `Vec<ClickModifier>` but emitting a non-empty list currently returns `ACTION_BACKEND_UNAVAILABLE` with the message "act_click modifiers are not wired in the M2 click schema slice".
- **Element-target aim and drag**. `act_aim` with an `Element` target returns `ACTION_BACKEND_UNAVAILABLE` ("requires the dedicated target resolution issue"); same for `Track` targets. `act_drag` supports `Element` targets via UIA bbox resolution.


---

<a id="file-09"></a>

> Source: `docs/systemspec/09_perception_and_capture.md`

# 09 — Perception, A11y, and Capture (`synapse-perception`, `synapse-a11y`, `synapse-capture`)

Source files covered:
- `crates/synapse-perception/src/lib.rs`
- `crates/synapse-perception/src/error.rs`
- `crates/synapse-perception/src/observe.rs`
- `crates/synapse-perception/src/ocr.rs`
- `crates/synapse-a11y/src/lib.rs`
- `crates/synapse-capture/src/lib.rs`
- `crates/synapse-mcp/src/m1.rs`
- `crates/synapse-mcp/src/m1/{ocr, search, sources}.rs`

## 1. Crate split

| Crate | Role |
|---|---|
| `synapse-capture` | Zero-copy GPU frame capture (`windows-capture` / DXGI). Owns `CaptureBackend`, `CaptureTarget`, `CaptureConfig`, `CapturedFrame`, DPI awareness, screen↔window coordinate helpers. |
| `synapse-a11y` | UIA tree walk + WinEvent hook + Chromium DevTools attach + accessible-event coalescing. |
| `synapse-perception` | Glue layer: assembles `Observation` from a11y + capture + OCR inputs; resolves perception mode (auto/a11y_only/pixel_only/hybrid); exposes OCR provider abstraction. |
| `synapse-mcp/src/m1` | MCP tool wrappers (`observe`, `find`, `read_text`, `set_capture_target`, `set_perception_mode`). |

## 2. `synapse-capture`

### 2.1 Public types (selected)

| Type | Definition / purpose |
|---|---|
| `CAPTURE_CHANNEL_CAPACITY` | `2` — bounded crossbeam channel from capture thread to consumer |
| `FRAMES_DROPPED_METRIC` | `"synapse_capture_frames_dropped_total"` |
| `D3d11Texture` | `windows::Win32::Graphics::Direct3D11::ID3D11Texture2D` on Windows, stub elsewhere |
| `SendablePtr<T>` | `Send + Sync` wrapper around a non-Send GPU handle (`unsafe impl`) so it can travel across threads |
| `DxgiFormat` | `Bgra8` \| `Bgra8Srgb` \| `Rgba8` \| `Rgba8Srgb` \| `Rgba16F` \| `Rgb10A2` \| `Rgb10XrA2` \| `Unknown(u32)` |
| `CapturedFrame` | `{ texture: SendablePtr<D3d11Texture>, width, height, format: DxgiFormat, captured_at: Instant, frame_seq: u64, dirty_region: Option<Rect> }` |
| `CapturedSoftwareBitmap` | `{ region: Rect, bitmap: windows::Graphics::Imaging::SoftwareBitmap }` — used by the WinRT OCR path |
| `CaptureBackend` | `GraphicsCaptureApi` \| `DxgiDuplication` |
| `CaptureBackendPreference` | `Auto` \| `GraphicsCaptureApi` \| `DxgiDuplication` (`from_force_dxgi_value` reads `SYNAPSE_CAPTURE_FORCE_DXGI`) |
| `CaptureTarget` | `Primary` (default) \| `Monitor { monitor_index: u32 }` \| `Window { hwnd: i64 }` |
| `CaptureConfig` | `{ target, min_update_interval_ms: u64 (default 16 → ~60 Hz), cursor_visible: bool (default true), secondary_windows: bool (default true), dirty_region_only: bool (default true), backend_preference: Auto }` |
| `ResolvedCaptureTarget` | `{ target: CaptureTarget, backend: CaptureBackend }` |
| `CaptureError` | `GraphicsApiUnsupported` / `TargetLost` / `TargetInvalid` / `NoDirtyRegions` / `ThreadFailed` with `.code()` → `CAPTURE_*` |
| `CaptureStats` | runtime stats (frames captured, dropped, last frame seq) |
| `CaptureThreadPriority` | `Unknown` (sentinel) \| `Unsupported` \| `TimeCritical` |
| `DpiAwarenessStatus` | result of `init_process_dpi_awareness` |
| `CaptureHandle` | producer / consumer pair for spawned capture loops |
| `CaptureController` | manages capture lifecycle |

### 2.2 Capture loop

`spawn_capture_loop(config) -> Result<CaptureHandle, CaptureError>`:

1. Initialize per-monitor-v2 DPI awareness via `init_process_dpi_awareness()` (idempotent; sets `PROCESS_PER_MONITOR_DPI_AWARE`).
2. Resolve target → `ResolvedCaptureTarget`. Backend `Auto` prefers `GraphicsCaptureApi` on Windows 10 1903+ and falls back to `DxgiDuplication` on older systems or when `SYNAPSE_CAPTURE_FORCE_DXGI` forces it.
3. Spawn a high-priority capture thread (Windows: `THREAD_PRIORITY_TIME_CRITICAL`).
4. Capture loop reads frames from `windows-capture` or DXGI duplication, tags with `frame_seq` (monotonic), optionally clips to `dirty_region`, and `try_send` into a bounded `crossbeam` channel of capacity `CAPTURE_CHANNEL_CAPACITY = 2`. On `Full`, the oldest frame is dropped and the `synapse_capture_frames_dropped_total` counter is incremented.
5. Respects `min_update_interval_ms` (default 16 ms) as the minimum interval between produced frames.

### 2.3 Coordinate helpers

| Function | Purpose |
|---|---|
| `screen_to_window(point: Point, hwnd: i64) -> Result<Point, CaptureError>` | Screen-space (DIPs) to window-relative |
| `window_to_screen(point: Point, hwnd: i64) -> Result<Point, CaptureError>` | Inverse |
| `screen_to_window_with_origin` / `window_to_screen_with_origin` | Const helpers when the origin is already known (used by element-target click) |
| `init_process_dpi_awareness() -> Result<DpiAwarenessStatus, CaptureError>` | Sets `PROCESS_PER_MONITOR_DPI_AWARE_V2` once; idempotent |
| `is_per_monitor_v2_dpi_aware() -> bool` | Read-back |
| `current_thread_priority() -> CaptureThreadPriority` | Used by capture-loop self-check |

### 2.4 WinRT OCR helpers

`captured_frame_region_to_software_bitmap(frame, region)` and `screen_region_to_software_bitmap(region)` produce `CapturedSoftwareBitmap` for the WinRT OCR backend.

## 3. `synapse-a11y`

Single-file 2087 LoC crate on `main` (HEAD `e54ca57`). Wraps `uiautomation` 0.25 and `chromiumoxide` 0.9. M3 carry-over: a `platform/*` module split is queued for M4 Block A.0 (see `docs/impplan/04_m3_reflex_mcp_surface.md`); when that lands, `lib.rs` becomes a 30-LoC re-export surface with logic in `cdp.rs` / `events.rs` / `ids.rs` / `re_resolve.rs` / `snapshot.rs` / `window.rs` / `platform/{non_windows.rs, windows/{common,events,resolve,snapshot,window}.rs}`.

### 3.1 Public surface

| Symbol | Purpose |
|---|---|
| `uiautomation` re-export, `UIElement` re-export | direct UIA access for downstream crates |
| `A11yError` (variants: not-available, element-stale, no-foreground, CDP-unreachable, …) → `.code()` → `A11Y_*` | structured error |
| `AccessibleEvent` (`source`, `at`, `kind: AccessibleEventKind`, `element_id`, `data`) | UIA / WinEvent / CDP normalized event |
| `AccessibleEventKind` | `FocusChanged`, `StructureChanged`, `ValueChanged`, `WindowOpened`, `WindowClosed`, etc. |
| `AccessibleEventSender = UnboundedSender<AccessibleEvent>` | bridge endpoint |
| `WinEventSubscription` (drop = unhook) | result of `subscribe_win_events(sender)` |
| `WinEventHookReadback` (status of the WinEvent hook thread) | observability |
| `ComApartmentKind` | STA / MTA / Uninitialized |
| `current_foreground_context() -> A11yResult<ForegroundContext>` | reads HWND of `GetForegroundWindow`, queries process name/path, monitor index, DPI, fullscreen flag |
| `focused_element() -> A11yResult<UIElement>` | UIA focused element |
| `focused_window() -> A11yResult<UIElement>` | top-level window for the focused element |
| `element_from_point(point) -> A11yResult<UIElement>` | hit-test |
| `snapshot(root, depth) -> A11yResult<AccessibleSubtree>` | bounded UIA tree walk via cache batch (depth ≤ 6) |
| `find_by_name_and_pattern(root, name, pattern)` | quick search for a node by `Name` + supported pattern |
| `re_resolve(&ElementId) -> A11yResult<UIElement>` | re-acquire a UIA element by its `<hwnd_hex>:<runtime_id_hex>` |
| `expand_state_of(&UIElement) -> A11yResult<ExpandState>` | `ExpandCollapsePattern` query |
| `coalesce_events<I>(events, window)` | dedupe within a sliding `Duration` window |
| `debounce_value_changes<I>(events, window)` | collapse rapid `ValueChanged` events on the same element |
| `cdp_capabilities() -> Vec<CdpCapability>`, `is_chromium_family(process_name)`, `probe_chromium_cdp(...)`, `attach_chromiumoxide(endpoint)` | Chromium DevTools Protocol integration |
| `CdpDiagnostics`, `CdpStatus`, `CdpCapability`, `CdpAttachment` | diagnostic types for the CDP path |
| `runtime_id_hex(runtime_id: &[i32]) -> String` | encodes a UIA runtime id as hex |

### 3.2 Event coalescing

`coalesce_events(events, window)` and `debounce_value_changes(events, window)`:
- Iterate events in order.
- For each event, drop preceding events on the same `element_id` + `kind` within `window` (typically 50 ms for value changes, 16 ms for focus changes).
- Used by `m3::a11y_events::A11yEventBridge` to keep the SSE bus signal-to-noise high.

### 3.3 COM apartment management

The crate ensures a per-thread COM apartment (`ComApartmentKind`) before any UIA call. Each public function checks for STA initialization and returns `A11yError::NotAvailable` if the calling thread is `Uninitialized`.

## 4. `synapse-perception`

### 4.1 Public surface

| Symbol | Source |
|---|---|
| `PerceptionError`, `PerceptionResult` | `error.rs` (`.code()` → `OBSERVE_INTERNAL`, `OCR_*`, etc.) |
| `ObservationAssembler` | `observe.rs` |
| `ObservationInput` | `observe.rs` — the input struct containing foreground, focused, elements, entities, hud, audio, recent_events, clipboard_summary, fs_recent, and a `mode_override: Option<PerceptionMode>` |
| `ObserveInclude` | `observe.rs` — per-slot booleans + `max_subtree_depth`, `max_subtree_nodes`, `max_entities` |
| `A11yTreeSummary` | `observe.rs` — counts (`total_nodes`, `enabled_nodes`, `focused_nodes`) used by `auto_mode` |
| `assemble`, `assemble_from_input` | `observe.rs` |
| `auto_mode(input) -> PerceptionMode`, `auto_mode_with_a11y(summary, ...)` | `observe.rs` — resolves Auto based on a11y density (≥10 visible enabled nodes → A11yOnly; known game process → PixelOnly; else Hybrid) |
| `bounded_sensor_latency` | `observe.rs` — helper that clamps measured latency for `ObservationDiagnostics` |
| `is_known_game_process(process_name) -> bool` | hardcoded list of "common game / fullscreen render-only process names |
| `parse_perception_mode(&str) -> PerceptionResult<PerceptionMode>` | string parse used by `set_perception_mode` |
| `OcrProvider`, `TextRegion`, `is_empty_region`, `read_text`, `read_text_with_provider` | `ocr.rs` |
| `read_text_from_software_bitmap` (Windows only) | `ocr.rs` |

### 4.2 `ObservationAssembler::assemble` algorithm

1. Compute the effective `PerceptionMode`: if `input.mode_override.is_some()` use it, else `auto_mode(input)`.
2. For each slot enabled by `ObserveInclude` (defaults: focused, elements, entities, hud, events), include the corresponding fields. Otherwise the slot is left at its default (empty vec / None).
3. Truncate `elements` to `max_subtree_nodes` (default 60, clamp 1..=500) and apply `max_subtree_depth` (clamp ≤ 6). Set `diagnostics.elements_truncated` when truncated.
4. Truncate `entities` to `max_entities` (default 60). Set `diagnostics.entities_truncated`.
5. Compute `diagnostics.assembled_in_ms = started.elapsed().as_secs_f32() * 1000`.
6. Estimate response size: `size_bytes = serde_json::to_vec(&observation).map(|v| v.len() as u32)` and `size_estimate_tokens = size_bytes / 4` (heuristic).
7. Set `diagnostics.sensor_latency_ms` per sensor (bounded via `bounded_sensor_latency`) and `*_status: SensorStatus` (Healthy / DegradedLatency { last_p99_ms } / DegradedSensorFailed { reason_code } / Disabled / Unavailable).
8. Return the `Observation`.

### 4.3 OCR

`OcrProvider` trait:

```rust
pub trait OcrProvider: Send + Sync {
    fn read_text(&self, region: TextRegion, lang_hint: Option<&str>) -> PerceptionResult<OcrResult>;
}
```

`TextRegion` is `{ rect: Rect, source: TextRegionSource }` where source distinguishes screen-coord regions vs an `ElementId` reference. `is_empty_region(rect)` checks `w <= 0 || h <= 0`.

`read_text(region, lang_hint)` picks the default `OcrProvider` (Windows: WinRT OCR via `Media.Ocr`; non-Windows: returns `OCR_BACKEND_UNAVAILABLE`).

`read_text_with_provider(provider, region, lang_hint)` lets callers inject a provider (used in tests with a fixture).

Windows-only `read_text_from_software_bitmap(bitmap: SoftwareBitmap, lang_hint)` is the lower-level entrypoint for code paths that already have a `SoftwareBitmap` in hand.

## 5. `synapse-mcp/src/m1` glue

### 5.1 `M1State`

```rust
pub struct M1State {
    pub capture_config: CaptureConfig,
    pub capture_generation: u64,
    pub perception_mode: PerceptionMode,
    pub synthetic: Option<ObservationInput>,
    pub force_no_perception: bool,
    pub force_observe_internal: bool,
    pub last_observed_foreground: Option<ForegroundContext>,
}
```

`M1State::from_env`:

- Reads `SYNAPSE_MCP_SYNTHETIC_FIXTURE` (case-insensitive "notepad" → synthetic Notepad observation source) — see `m1/sources.rs::synthetic_notepad_input`.
- Reads `SYNAPSE_MCP_FORCE_NO_PERCEPTION` (`1`/`true`) to make every `observe` call return `OBSERVE_NO_PERCEPTION_AVAILABLE`.
- Reads `SYNAPSE_MCP_FORCE_OBSERVE_INTERNAL` to make every `observe` call return `OBSERVE_INTERNAL`.
- Pulls capture backend preference from `SYNAPSE_CAPTURE_FORCE_DXGI` via `CaptureConfig::with_env_backend`.

`last_observed_foreground` is updated after each successful `observe` call (`server.rs::observe`). This is the SoT consulted by `ensure_act_type_foreground` so `act_type` refuses to type into the wrong window.

### 5.2 ObserveParams and slot expansion

`ObserveParams`:

| Field | Default | Behavior |
|---|---|---|
| `include: Vec<ObserveSlot>` | `[]` (treated as defaults) | When empty → defaults: `focused, elements, entities, hud, events`. Non-empty list selects exactly those slots. |
| `depth: Option<u32>` | `None` → `2` | Clamped to `..= 6` |
| `max_elements: Option<usize>` | `None` → `60` | Clamped to `1..=500` |
| `since_event_seq: Option<u64>` | `None` | When set, `recent_events` is filtered to those with `seq > since` |

`observe_include(params)` builds the `ObserveInclude` struct used by `ObservationAssembler::assemble`.

### 5.3 FindParams + `find_in_state`

`FindParams`: `query`, `role`, `name_substring`, `automation_id`, `scope (Elements/Entities/Both)`, `limit (1..=20 default 5)`, `in_window` (filter to a window by `ElementId`).

`find_in_state(state, params)`:

1. Fetch current observation input (`current_input` calls `m1/sources::platform_input`).
2. Iterate elements scoring with `m1/search::element_match` (matches role + name substring + automation id; awards a higher score for exact-role + token coverage).
3. Iterate entities with `m1/search::entity_match` (matches class_label substring).
4. Sort by `score` descending. Truncate to `limit`.

### 5.4 `read_text_in_state`

`m1/ocr.rs::read_text_in_state`:

1. If `params.region` is `Some`, capture that region via `synapse_capture::screen_region_to_software_bitmap` and run WinRT OCR via `synapse_perception::read_text_from_software_bitmap`.
2. If `params.element_id` is `Some`, resolve via `synapse_a11y::re_resolve`, compute its bbox, then proceed as above.
3. Else: read the focused element's bbox.
4. Builds an `OcrResult { full_text, words, confidence, region, lang }`.
5. The `backend` param is currently retained for schema stability but does not branch (always WinRT in this build).

### 5.5 `set_capture_target_in_state`

`set_capture_target_in_state(state, params)`:

1. Compute the previous wire target.
2. Build a new `CaptureConfig`:
   - For `Primary`/`Monitor`/`Window` variants, copy as-is.
   - For `ElementWindow { element_id }`, call `element_id.parts()?.hwnd` and set `target = CaptureTarget::Window { hwnd }`.
3. Apply optional `min_update_interval_ms` (force `>= 1`), `cursor_visible`, `dirty_region_only` overrides.
4. `synapse_capture::resolve_capture_target(&config)` (validates the target — returns `CAPTURE_TARGET_INVALID` for unknown monitors or non-existent HWNDs).
5. Stamp `state.capture_config = config`, `state.capture_generation += 1`.
6. Return `SetCaptureTargetResponse { previous, current, generation, backend: "graphics_capture_api" | "dxgi_duplication" }`.

### 5.6 `set_perception_mode_in_state`

`set_perception_mode_in_state(state, params)`:

1. `parse_perception_mode(&params.mode)` (errors with `PERCEPTION_MODE_INVALID`).
2. Stamp `state.perception_mode = mode`.
3. Return `{ previous, mode, rationale }` where rationale is one of `auto_select_by_foreground_and_a11y_density`, `manual_a11y_only`, `manual_pixel_only`, `manual_hybrid`.

### 5.7 `mcp_error` helper

`mcp_error(code: &'static str, message: impl Into<String>) -> ErrorData`:

```rust
ErrorData::new(
    rmcp::model::ErrorCode(-32099),
    message,
    Some(json!({ "code": code })),
)
```

The fixed JSON-RPC code `-32099` is the rmcp custom-error slot; the structured `code` field carries the Synapse error name.

## 6. Cross-cutting integration

| Edge | Direction | Details |
|---|---|---|
| `observe` tool → `M1State` | sync (lock) | Each call uses `current_input` to build a fresh `ObservationInput` (synthetic, forced-error, or platform-derived). |
| `M1State.last_observed_foreground` → `act_type` | sync read | Comparison happens before any keystroke synthesis. |
| `synapse-a11y` events → SSE bus | via `m3::a11y_events::A11yEventBridge` | The bridge subscribes via `subscribe_win_events`, coalesces, and publishes `Event { source: EventSource::A11yWinEvent / A11yUia, kind: <derived>, data: {element_id, ...} }`. Started on first `reflex_runtime()` call (`SynapseService::reflex_runtime` calls `state.ensure_a11y_event_bridge`). |
| `synapse-capture` channel → consumer | bounded crossbeam | Downstream consumers (perception, OCR) `try_recv` and either run inference / OCR on the texture or discard. |

## 7. What is NOT covered

- **CNN object detection.** `synapse-models` ships the `Detector` trait and ONNX session loader, but `M1State` does not invoke detectors in the current build; `entities: Vec<DetectedEntity>` is populated only by synthetic fixtures.
- **HUD extraction.** `Profile.hud` carries `HudFieldSpec`s but the perception layer does not yet run HUD extractors against live frames — `hud: HudReadings` is empty unless populated synthetically.
- **Audio in `Observation`.** The `audio: AudioContext` field is populated only when an audio runtime is initialized and pushing into the observation source (current build leaves it default).
- **Linux/macOS.** All UIA / WinEvent / WinRT OCR paths are `cfg(windows)`; non-Windows builds return `A11Y_NOT_AVAILABLE` / `OCR_BACKEND_UNAVAILABLE`.


---

<a id="file-10"></a>

> Source: `docs/systemspec/10_audio_and_models.md`

# 10 — Audio Runtime & Model Loader (`synapse-audio`, `synapse-models`)

Source files covered:
- `crates/synapse-audio/src/lib.rs`
- `crates/synapse-audio/src/error.rs`
- `crates/synapse-audio/src/loopback.rs`
- `crates/synapse-audio/src/ring.rs`
- `crates/synapse-audio/src/detectors.rs`
- `crates/synapse-audio/src/direction.rs`
- `crates/synapse-audio/src/stt.rs`
- `crates/synapse-audio/src/stt/window.rs`
- `crates/synapse-models/src/lib.rs`
- `crates/synapse-mcp/src/m3/audio.rs`

## 1. `synapse-audio`

### 1.1 Public surface

```rust
pub const DEFAULT_RING_SECONDS: u32 = 5;
pub const MAX_RING_SECONDS: u32 = 5;

pub type AudioEventSink = Arc<dyn Fn(Event) + Send + Sync + 'static>;

pub struct AudioConfig {
    pub ring_seconds: u32,            // 1..=MAX_RING_SECONDS
    pub start_loopback: bool,
    pub detectors_enabled: bool,      // requires start_loopback
    pub stt_model_path: Option<PathBuf>,
}

pub struct AudioRuntime { ... }
```

Re-exports: `AudioError`, `AudioResult` (error.rs); `LoopbackStatus` (loopback.rs); `AudioFormat`, `AudioRing`, `AudioWindow` (ring.rs); `Transcription`, `WhisperTinyStt` (stt.rs); `DirectionEstimate` (re-exported from `synapse-core`).

### 1.2 `AudioRuntime::spawn(config)` / `spawn_with_event_sink(config, sink)`

1. `validate_config`:
   - `ring_seconds` must be in `1..=MAX_RING_SECONDS = 5`; else `AudioError::LoopbackInitFailed`.
   - `detectors_enabled = true && start_loopback = false` → `AudioError::LoopbackInitFailed` ("audio detectors require loopback startup").
2. Build `AudioRing::new(ring_seconds)` — see §1.4.
3. If `start_loopback`, call `loopback::start_loopback(ring, optional DetectorProcessor)`:
   - Opens the default render endpoint via `wasapi` (WASAPI loopback).
   - Spawns a high-priority capture thread that reads PCM frames, pushes them into the ring, and (if detectors are enabled) feeds them into the `DetectorProcessor` which emits `Event`s to `event_sink`.
   - Returns a `LoopbackHandle` with status tracking.
4. Initialize `WhisperTinyStt::new(stt_model_path)`. The model is not loaded yet — load is deferred to first `transcribe_window` / `transcribe_file` call.
5. Return `AudioRuntime { config, ring, detector_state, loopback, stt }`.

### 1.3 AudioRuntime methods

| Method | Behavior |
|---|---|
| `config() -> &AudioConfig` | borrow config |
| `loopback_started() -> bool` | true iff the WASAPI thread is alive |
| `detectors_started() -> bool` | `config.detectors_enabled && loopback_started()` |
| `ring() -> Arc<AudioRing>` | shares the ring (used by `audio_tail`) |
| `tail_seconds(seconds: f32) -> AudioResult<AudioWindow>` | reads the most-recent N seconds of PCM (seconds outside `1..=ring_seconds` → `LoopbackInitFailed`) |
| `estimate_direction_tail(seconds) -> AudioResult<DirectionEstimate>` | computes azimuth from stereo magnitude+phase over the window |
| `transcribe_tail(seconds, language) -> AudioResult<Transcription>` | runs Whisper-tiny over the ring tail |
| `transcribe_file(path, language) -> AudioResult<Transcription>` | runs Whisper-tiny over a WAV file (used by integration tests against `tests/fixtures/audio/*.wav`) |
| `detector_snapshot() -> DetectorSnapshot` | last detector decision (RMS, VAD, transient, direction) |
| `loopback_status() -> LoopbackStatus` | `{ running, frames_captured, last_error_code: Option<&str> }` |
| `stt_model_loaded() -> bool` | true iff Whisper has loaded successfully |

### 1.4 Ring buffer (`ring.rs`)

`AudioRing`:

- Holds a `Vec<f32>` capable of storing `ring_seconds * DEFAULT_SAMPLE_RATE_HZ * STEREO_CHANNELS` samples (default `5 * 48000 * 2 = 480 000`).
- Lock-free `push_interleaved(samples: &[f32])` updates an `AtomicUsize` write index; readers (`tail_seconds`) snapshot the index and copy out the trailing N frames.
- `set_format(AudioFormat)` updates the negotiated `{ sample_rate_hz, channels }` (default `48000 Hz, 2 ch`).

`AudioFormat`:

| Constant | Value |
|---|---|
| `DEFAULT_SAMPLE_RATE_HZ` | `48_000` |
| `STEREO_CHANNELS` | `2` |

`AudioWindow`:

```rust
pub struct AudioWindow {
    pub samples: Vec<f32>,
    pub format: AudioFormat,
    pub generation: u64,
}
```

`pcm_i16_le()` converts the f32 samples to little-endian s16 PCM bytes for transport across the MCP `audio_tail` response.

### 1.5 Loopback (`loopback.rs`)

- Uses `wasapi 0.23` to open the system default render endpoint in **loopback** mode and a `EventCallback`-driven capture loop.
- The thread writes PCM frames into `AudioRing::push_interleaved` and increments `frames_captured`.
- On WASAPI failure, emits a `tracing::warn` with `code = AUDIO_DEVICE_LOST` (or `AUDIO_LOOPBACK_INIT_FAILED` on setup failure) and records the code into `LoopbackStatus.last_error_code`.

### 1.6 Detectors (`detectors.rs`)

`DetectorProcessor` is an optional consumer that observes each pushed buffer and emits events:

- **RMS** detector emits `EventSource::PerceptionAudio` events of `kind = "audio.rms"` when the running RMS crosses a threshold.
- **VAD** (voice activity) detector flags spoken speech segments.
- **Transient** detector raises `kind = "audio.transient"` on sudden energy spikes.
- All events go through the caller-supplied `AudioEventSink`.

`DetectorSnapshot` captures the current detector state (last RMS dB, last VAD bool, last transient time).

### 1.7 Direction estimate (`direction.rs`)

`estimate_direction(window: &AudioWindow) -> DirectionEstimate`:

1. Stereo decorrelation: compute left-channel and right-channel magnitudes and inter-channel phase difference over the window.
2. Map left-vs-right magnitude ratio to a left-right azimuth in `[-90°, +90°]`.
3. Confidence = `1 - normalized_variance_of_estimates`.

Returns `DirectionEstimate { azimuth_deg: f32, confidence: f32 }` (also a public field on `AudioContext`).

### 1.8 STT (`stt.rs`, `stt/window.rs`)

`WhisperTinyStt`:

- `new(stt_model_path: Option<PathBuf>)` — constructs a lazy-load wrapper. Loading is deferred to first transcribe call.
- `is_loaded() -> bool` — true iff Whisper-tiny has been loaded and validated.
- `transcribe_window(&AudioWindow, language: impl AsRef<str>) -> AudioResult<Transcription>`:
  - Validates language string ("en" only in current build; the M3 audio tool wrapper enforces this at the MCP boundary too).
  - Normalizes the window samples to 16 kHz mono via `stt/window::resample_mono`.
  - If the model file at `stt_model_path` (or the default path) is missing → `AudioError::SttModelNotLoaded` → `AUDIO_STT_MODEL_NOT_LOADED`. Silent input returns `Transcription { text: "", confidence: 0.0, elapsed_ms: 0 }` without invoking ORT (see test `transcribe_maps_silence_without_model_load_and_rejects_language` in `m3/audio.rs`).
  - Otherwise verifies the SHA-256 of the model file against the pinned digest → `MODEL_HASH_MISMATCH` on drift.
  - Loads via `ort` (with `directml` feature in this crate's Cargo.toml) and runs inference.
  - Returns `Transcription { text: String, confidence: f32, elapsed_ms: i64 }`.
- `transcribe_file(path, language)` — same pipeline, but loads the WAV file directly. Used by integration tests against `tests/fixtures/audio/hello_world_5s.wav`, `loud_transient_1s.wav`, `pan_minus60_0_plus60.wav`.

### 1.9 Errors (`error.rs`)

`AudioError::code()` → `AUDIO_DEVICE_LOST` / `AUDIO_LOOPBACK_INIT_FAILED` / `AUDIO_STT_MODEL_NOT_LOADED` / `MODEL_HASH_MISMATCH` / `MODEL_LOAD_FAILED` / `MODEL_BACKEND_UNAVAILABLE`.

## 2. `synapse-models`

### 2.1 Crate features

`crates/synapse-models/Cargo.toml`:

```toml
[features]
default = []
ort = [
    "dep:ort",
    "ort/api-24",
    "ort/copy-dylibs",
    "ort/download-binaries",
    "ort/std",
    "ort/tls-native",
]
cuda = ["ort", "ort/cuda"]
directml = ["ort", "ort/directml"]
```

`synapse-audio` enables `directml`. `synapse-mcp` does not pull in CUDA or DirectML features explicitly; the configured-host install/setup path is responsible for ensuring the ONNX runtime DLL is present. If it is missing during issue work, the agent must acquire or configure it through local reversible workflows where possible and then read the physical DLL/path/source-of-truth directly.

### 2.2 Public surface

| Type | Definition |
|---|---|
| `ModelDescriptor` | `{ id: String, path: PathBuf, sha256: String, input_shape: Vec<usize>, class_map: Vec<String> }`. `yolov10n_general(sha256, class_map)` ctor produces the canonical YOLOv10-nano descriptor with `path = default_model_dir().join("yolov10n_general.onnx")` and `input_shape = vec![1, 3, 640, 640]`. |
| `ModelBackend` | `Cuda` \| `DirectMl` \| `Cpu` (default) |
| `DetectOpts` | `{ confidence_threshold: u16 (default 50), max_detections: usize (default 100) }` |
| `DetectionFrame` | `{ frame_seq: u64, width: u32, height: u32 }`. `validate()` returns `DETECTION_NO_FRAME` for zero dimensions. |
| `Detector` (trait) | `fn infer(&self, frame: DetectionFrame, opts: DetectOpts) -> ModelResult<DetectionBatch>` |
| `ModelError` | thiserror enum with variants `DownloadFailed` / `HashMismatch` / `LoadFailed` / `BackendUnavailable` / `NoFrame` / `InferenceFailed`; `.code()` → `MODEL_*` / `DETECTION_*` |

### 2.3 Model loader behavior

When the `ort` feature is enabled:

1. `Detector::load(descriptor: &ModelDescriptor)` reads the file bytes.
2. Computes SHA-256 (`sha2::Sha256`) and compares against `descriptor.sha256`. Mismatch → `ModelError::HashMismatch` → `MODEL_HASH_MISMATCH`.
3. Constructs an `ort::Session` with the configured `ModelBackend` (`Cpu` if no acceleration feature is enabled).
4. Per-inference: validates the frame, runs the session, decodes the YOLOv10 output tensor into `Vec<Detection>` filtered by `confidence_threshold/100` and capped at `max_detections`.

When `ort` is not enabled, all `Detector::load` paths return `ModelError::BackendUnavailable` → `MODEL_BACKEND_UNAVAILABLE`.

### 2.4 Session IDs

The crate maintains a process-wide `AtomicU64 NEXT_SESSION_ID = 1` used to label model sessions in tracing logs (`crate::lib.rs:15`). This is unrelated to `SessionId` in `synapse-core::types`.

### 2.5 Default model directory

`default_model_dir()` (per source: helper near the bottom of `lib.rs`) is the directory used for the bundled-on-demand model cache. The actual download/cache mechanism is not implemented in the current build — `MODEL_DOWNLOAD_FAILED` is reserved.

## 3. MCP audio tools (`crates/synapse-mcp/src/m3/audio.rs`)

| Tool | Behavior |
|---|---|
| `audio_tail(seconds: u32)` | `0..=MAX_RING_SECONDS = 5`. `0` returns an empty PCM body with `sample_rate = 48_000`, `channels = 2`, `format = "s16le"`. Otherwise pulls `AudioRuntime::tail_seconds(seconds)`, converts to little-endian s16 (`AudioWindow::pcm_i16_le`), and **pads with zeros** if the ring has less data than requested (so the returned `pcm.len() == seconds * sample_rate * channels * 2`). |
| `audio_transcribe(seconds: u32, language: String)` | Same `seconds` bounds. `language` accepts `"en"` or empty (mapped to `"en"`); anything else → `TOOL_PARAMS_INVALID`. Calls `AudioRuntime::transcribe_tail`. Returns `{ text, confidence, latency_ms, model_id: "whisper_tiny_int8" }`. |

Both require permission `READ_AUDIO`, which is only granted by default when `--enable-audio` is set (see [03_configuration.md §4.4](#file-03)).

Lazy init: `M3State::ensure_audio_runtime` (`crates/synapse-mcp/src/m3.rs::364`) builds the `AudioRuntime` on first call with:

```rust
AudioConfig {
    ring_seconds: DEFAULT_RING_SECONDS,
    start_loopback: audio_loopback_enabled()?,  // reads SYNAPSE_AUDIO_LOOPBACK
    detectors_enabled: false,                   // detectors are not wired into the M3 tools yet
    stt_model_path: None,                       // uses synapse-audio's default lookup
}
```

`detectors_enabled = false` because no event sink is plumbed in this build (the SSE bus event sink integration is reserved for later work).

## 4. Performance metrics emitted

| Metric | Kind | Labels | Source |
|---|---|---|---|
| `audio_loopback_underruns_total` | counter | — | `loopback.rs` (incremented on missed-deadline reads) |
| `audio_stt_inferences_total` | counter | `outcome` (success/timeout/failure) | `stt.rs` |
| `audio_stt_latency_ms` | histogram | — | `stt.rs` |

## 5. Test fixtures

The repository ships three WAV fixtures (`tests/fixtures/audio/`):

| File | Purpose |
|---|---|
| `hello_world_5s.wav` | English speech sample used by `audio_transcribe` integration tests |
| `loud_transient_1s.wav` | Tests transient detector and RMS clipping |
| `pan_minus60_0_plus60.wav` | Tests `estimate_direction` azimuth output across the stereo field |

See `tests/fixtures/audio/README.md` for the synthesis recipe.

## 6. What is NOT covered

- **STT models other than Whisper-tiny.** The model id is hard-coded `whisper_tiny_int8` in `m3/audio.rs` and only one language ("en") is accepted.
- **No streaming transcription.** `audio_transcribe` returns a complete `Transcription` after running over the buffered tail; there is no incremental streaming API.
- **Model auto-download.** `MODEL_DOWNLOAD_FAILED` is reserved as an error code but there is no download path; when a workflow requires the ONNX file, the agent acquires or imports it on the configured host through a license-compliant local setup path and verifies `synapse-audio::stt::default_model_path()` plus the expected hash directly.
- **Custom audio devices.** WASAPI loopback always uses the default render endpoint; there is no selector for non-default outputs.
- **YOLO inference pipeline.** `synapse-models::Detector::load` works end-to-end but no `M1State` code path runs detection yet (entities are populated only by synthetic fixtures).


---

<a id="file-11"></a>

> Source: `docs/systemspec/11_profiles_hid_telemetry.md`

# 11 — Profiles, Hardware HID, Telemetry, Test Utilities

Source files covered:
- `crates/synapse-profiles/src/lib.rs`
- `crates/synapse-profiles/src/error.rs`
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

## 1. `synapse-profiles` — TOML profile loader + live reload

### 1.1 Public surface

| Symbol | Source |
|---|---|
| `ProfileError`, `ProfileLoadError` | `error.rs` |
| `LoadedProfile`, `ProfileDefaults`, `ScreenBounds`, `bundled_profiles_dir`, `parse_profile_bytes`, `parse_profile_file`, `parse_profile_file_with_bounds` | `parser.rs` |
| `ForegroundWindow`, `ProfileMatchResolution`, `resolve_active_profile` | `resolver.rs` |
| `ProfileRuntime`, `ProfileStatus` | `watcher.rs` |

`toml_format.rs` is private (holds `RawProfile`, the TOML-shaped intermediate).

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
    pub active: bool,
    pub schema_version: u32,
    pub matches: Vec<ProfileMatch>,
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
3. Profile with the highest specificity (most match fields satisfied) wins; ties broken by load order.
4. Returns `ProfileMatchResolution { matched_profile: Option<ProfileId>, candidates: Vec<...> }`.

`ForegroundWindow` is the input contract:

```rust
pub struct ForegroundWindow {
    pub process_name: String,
    pub window_title: String,
    pub window_class: String,
    pub steam_appid: Option<u32>,
    pub process_args: Vec<String>,
}
```

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
| `profile_activate { profile_id }` | look up the profile; if `use_scope = Unknown` and `--allow-unknown-profile` is not set, return `SAFETY_PROFILE_ACTION_DENIED`; if already active, return `changed = false`; else `runtime.activate(profile_id)`; permission: `WRITE_PROFILE_ACTIVE` |

## 2. `synapse-hid-host`

```text
crates/synapse-hid-host/src/
  discover.rs, error.rs, handshake.rs, lib.rs, pipeline.rs,
  protocol.rs, reconnect.rs, transport.rs
```

Direct dependencies (`Cargo.toml`): `crc16`, `serde`, `serialport`, `synapse-core`, `thiserror`, `tokio`, `tracing`. The crate implements the host-side serial gateway used by `HardwareBackend`: port discovery, `HidGateway::connect`, IDENTIFY parsing/version validation, CRC16 frames, pipelined send, reconnect state, and structured HID errors.

The live driver talks to the RP2040 firmware over USB CDC at 1 Mbaud, with CRC16 frames and a firmware version handshake. Error codes surfaced by the driver include `HID_PORT_NOT_FOUND`, `HID_PORT_OPEN_FAILED`, `HID_PROTOCOL_HANDSHAKE_FAILED`, `HID_FIRMWARE_VERSION_MISMATCH`, `HID_COMMAND_REJECTED`, and `HID_LINK_TIMEOUT`.

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

- Permission gates that consult profile use-scope: [03_configuration.md §4.4](#file-03), [06_mcp_service_and_transports.md §1.5](#file-06).
- Profile schema types: [05_core_types_and_errors.md §5.5](#file-05).
- Metric usage: [07_reflex_runtime.md §10](#file-07), [10_audio_and_models.md §4](#file-10), [04_storage_layer.md §7](#file-04).

## 6. What is NOT covered

- **Physical `synapse-hid-host` runtime FSV.** Source inspection covers the host driver shape; issue closure still requires real Pico/COM-device source-of-truth evidence on the configured host.
- **OTLP export.** `opentelemetry` and `opentelemetry-otlp` are in workspace deps but not wired in `synapse-telemetry::init_tracing` — the file/console layers are the only sinks.
- **Prometheus exporter binding.** `metrics-exporter-prometheus` is referenced in workspace deps but not bound to an HTTP port by `synapse-telemetry`; the `register_m3_metrics` path only describes the metrics so the `metrics` crate global recorder can hold them.
- **Profile-driven action defaults.** `Profile.backends` and `ProfileDefaults` are parsed but not consulted by the M2 emitter wrappers in the current build; tools use their own per-tool defaults (e.g. `act_click.curve = Natural`).
- **Profile activation persistence.** Activating a profile updates in-memory state only; nothing is persisted to `CF_PROFILES` in this build (PRD §7 reserves that CF for future use).


---

<a id="file-12"></a>

> Source: `docs/systemspec/12_milestones_and_roadmap.md`

# 12 — Milestones, Roadmap, and Open Decisions

Source files covered:
- `CHANGELOG.md`
- `README.md`
- `AGENTS.md`
- `docs/impplan/README.md`
- `docs/impplan/00_methodology.md`
- `docs/impplan/01_m0_bootstrap.md`
- `docs/impplan/02_m1_perception_mvp.md`
- `docs/impplan/03_m2_action_mvp.md`
- `docs/impplan/04_m3_reflex_mcp_surface.md`
- `docs/impplan/05_m4_hardware_hid_first_game.md`
- `docs/impplan/06_m5_production_polish.md`
- `docs/impplan/07_cross_cutting.md`
- `docs/computergames/15_roadmap_and_milestones.md`
- `docs/computergames/16_open_questions.md`
- `docs/adr/0001..0007*.md`

## 1. Authority order

Per `docs/impplan/README.md` §"State-tracking", the authority order is:

1. **Git tags + `CHANGELOG.md`** — what shipped.
2. **`main` branch** — what is in code now (impplan is wrong if it disagrees; patch the impplan in the same PR).
3. **GitHub Issues** — every PR-sized task, `[DECISION]`, `[DISCOVERY]`, bug, risk, context (labels: `phase:m{N}`, `area:*`).

## 2. Milestone state (as of 2026-05-26, HEAD `e54ca57`)

| # | Milestone | Tag | Date | Source |
|---|---|---|---|---|
| M0 | Workspace + rmcp stdio + `health` tool | `v0.1.0-m0` | 2026-05-23 | `CHANGELOG.md::v0.1.0-m0` |
| M1 | Perception MVP — capture + UIA + `observe()` + 5 tools | `v0.1.0-m1` | 2026-05-23 | `docs/impplan/README.md` |
| M2 | Action MVP — `synapse-action` + 9 tools + `release_all` | `v0.1.0-m2` | 2026-05-24 | `CHANGELOG.md::v0.1.0-m2` |
| M3 | Reflex + RocksDB + profiles + HTTP/SSE + audio + 15 tools | `v0.1.0-m3` (@ `97019ec`) | 2026-05-25 | `CHANGELOG.md::v0.1.0-m3` + `docs/impplan/04_m3_reflex_mcp_surface.md` |
| **M4** | **RP2040 firmware + `synapse-hid-host` serial driver + Minecraft profile + `act_combo`/`act_run_shell`/`act_launch`** | — | **ACTIVE** | `docs/impplan/05_m4_hardware_hid_first_game.md` |
| M5 | Production polish — installer, overlay, ≥10 profiles, VLM `describe`, soak | — | blocked by M4 | `docs/impplan/06_m5_production_polish.md` |

M3 closed 2026-05-25 (`v0.1.0-m3` @ `97019ec`). What landed on `main`:

- `synapse-storage` — RocksDB open + 11 CFs + per-CF TTL filter + 5 min GC + 4-level disk-pressure responder + JSON codecs (ADR-0001/0002)
- `synapse-reflex` — `EventBus` (bounded crossbeam per subscriber, configurable cap), 1 ms time-critical scheduler (Windows: `THREAD_PRIORITY_TIME_CRITICAL` + MMCSS Pro Audio), 5 reflex kinds (`AimTrack`/`HoldMove`/`HoldButton`/`Combo`/`OnEvent`), recursion guard (ADR-0003), priority resolution (ADR-0004), `CF_REFLEX_AUDIT` persistence
- `synapse-profiles` — TOML parser + `notify`-debounced watcher (200 ms) + match resolver (ADR-0006) + 4 bundled profiles (`notepad`, `vscode`, `chrome`, `terminal`, all Natural defaults)
- `synapse-audio` — WASAPI loopback (5 s ring) + detectors (loud-transient / speech start-end / Silero VAD) + Whisper-tiny-int8 STT + GCC-PHAT stereo direction
- HTTP transport — streamable HTTP + SSE (ADR-0007 per-event notifications); Bearer auth via `subtle::ConstantTimeEq`; Origin/Host loopback allow-list; `Mcp-Session-Id` enforcement
- 15 M3 tools (11 PRD M3 tools + 4 operator-only `storage_*` diagnostics added during M3 — see §3)
- Operator panic hotkey (`Ctrl+Alt+Shift+P`) wired with 50 ms `ReleaseAll` budget
- ADRs landed: 0003 (recursion guard, OQ-022), 0004 (priority, OQ-005), 0005 (multi-monitor capture target, OQ-012), 0006 (profile match precedence, OQ-015), 0007 (per-event notifications, OQ-029)

M3 carry-over open for M4 to address:

- **LoC overrun** — 500-LoC file cap was violated during M3. On `main` (HEAD `e54ca57`): `synapse-a11y/src/lib.rs` (2087), `synapse-capture/src/lib.rs` (1798), `synapse-core/src/types.rs` (1567), `synapse-mcp/src/server.rs` (1335), `synapse-mcp/src/m3/reflex.rs` (1165), `synapse-reflex/src/lib.rs` (986), `synapse-reflex/src/scheduler.rs` (890), `synapse-mcp/src/http/sse.rs` (764), `synapse-mcp/src/m3/replay.rs` (651), `synapse-models/src/lib.rs` (535). M4's Block A.0 splits these before adding hardware HID. Several test files also exceed cap.
- **CHANGELOG M3 entry tool-name drift** — the `v0.1.0-m3` entry names `profile_get`/`profile_set_active`; shipped names are `profile_list`/`profile_activate`. The four `storage_*` diagnostic tools are also missing from the entry. First M4 docs sweep fixes both.

Open M4 work (per `docs/impplan/05_m4_hardware_hid_first_game.md`):

- `firmware/pico-hid/` — standalone RP2040 firmware project excluded from the root Cargo workspace; remaining firmware issues close only with real device evidence.
- `synapse-hid-host` — serial driver with discovery, connect/IDENTIFY, CRC16 framing, pipeline/backpressure, and reconnect paths. `Backend::Hardware` uses `HardwareBackend` when `--hardware-hid <port|auto>` connects successfully, otherwise it fails closed through `HardwareUnavailableBackend`.
- `act_combo`, `act_run_shell`, `act_launch` — three M4 tools that bring the live MCP tool count from 30 → 33.
- `minecraft.java` profile (the first game profile) — fifth bundled profile, validated against a single-player creative world per `15_roadmap_and_milestones.md` §6.
- M3 hold-over items still open: per-subscriber `subscribe.buffer_size` (currently hard-pinned to 4096); persistent writers for `CF_EVENTS`/`CF_OBSERVATIONS`/`CF_SESSIONS`/`CF_TELEMETRY`/`CF_ACTION_LOG`/`CF_PROCESS_HISTORY`/`CF_KV` (only `CF_REFLEX_AUDIT` has a live writer); audio detector → SSE-bus sink integration; HUD extraction pipeline. VLM `describe` and Florence-2 remain M5.

## 3. Tools delivered vs planned

PRD `docs/computergames/05_mcp_tool_surface.md` defines a 30-tool surface cap for the agent-facing tools. Synapse's live build extends this with four operator-only `storage_*` diagnostics added during M3. As of M3 close:

| # | Tool | Milestone | Status | Note |
|---|---|---|---|---|
| 1 | `health` | M0 | live | |
| 2 | `observe` | M1 | live | |
| 3 | `find` | M1 | live | |
| 4 | `read_text` | M1 | live | |
| 5 | `set_capture_target` | M1 | live | |
| 6 | `set_perception_mode` | M1 | live | |
| 7 | `act_click` | M2 | live | modifiers not yet wired |
| 8 | `act_type` | M2 | live | |
| 9 | `act_press` | M2 | live | |
| 10 | `act_aim` | M2 | live | Element / Track targets return `ACTION_BACKEND_UNAVAILABLE` |
| 11 | `act_drag` | M2 | live | |
| 12 | `act_scroll` | M2 | live | |
| 13 | `act_pad` | M2 | live | |
| 14 | `act_clipboard` | M2 | live | |
| 15 | `release_all` | M2 | live | |
| 16 | `subscribe` | M3 | live | `buffer_size` pinned at 4096 |
| 17 | `subscribe_cancel` | M3 | live | |
| 18 | `reflex_register` | M3 | live | |
| 19 | `reflex_cancel` | M3 | live | |
| 20 | `reflex_list` | M3 | live | |
| 21 | `reflex_history` | M3 | live | |
| 22 | `profile_list` | M3 | live | |
| 23 | `profile_activate` | M3 | live | use_scope=unknown requires `--allow-unknown-profile` |
| 24 | `replay_record` | M3 | live | JSONL only |
| 25 | `audio_tail` | M3 | live | |
| 26 | `audio_transcribe` | M3 | live (en only) | |
| 27 | `storage_inspect` | M3 (operator) | live | per-CF row+byte size readback |
| 28 | `storage_put_probe_rows` | M3 (operator) | live | manual storage write/readback support tool |
| 29 | `storage_gc_once` | M3 (operator) | live | synchronous GC pass with before/after sizes |
| 30 | `storage_pressure_sample` | M3 (operator) | live | synthetic disk-pressure trigger |
| — | `read_hud` | (deferred to M4) | not live | HUD extraction pipeline not yet wired |
| — | `act_combo` | M4 | not live | replicated via `reflex_register` |
| — | `act_run_shell` | M4 (gated) | not live | |
| — | `act_launch` | M4 (gated) | not live | |
| — | `describe` | M5 (VLM) | not live | Florence-2 |

Live count in `crates/synapse-mcp/src/server.rs`: **30** (M1: 6, M2: 9, M3: 15 — including 4 operator-only `storage_*` diagnostics; the M3 `m3_tool_stubs()` length-asserts to 15).

## 4. Architecture Decision Records (ADRs)

| File | Title | Decision summary |
|---|---|---|
| `docs/adr/0001-current-rust-and-dependencies.md` | Current Rust + dependencies | Pin to the current installed stable toolchain (`rust-version = "1.95"`); no MSRV downgrade; JSON-only persisted codecs in `synapse-storage` (per RUSTSEC-2025-0141) |
| `docs/adr/0002-rocksdb-primary-storage.md` | RocksDB as primary storage | Chose RocksDB over LMDB/sled for the 11-CF schema; rationale around column-family compaction filters and prefix bloom |
| `docs/adr/0003-reflex-recursion-guard.md` | Reflex recursion guard | OnEvent fires are capped at `MAX_ON_EVENT_FIRINGS_PER_TICK = 4` per tick; overflow emits `REFLEX_RECURSION_LIMIT` audit + bus event exactly once per tick |
| `docs/adr/0004-reflex-priority.md` | Reflex priority semantics | Lower number = higher priority; ties broken by registration order; `MAX_REFLEX_PRIORITY = 1000`, `DEFAULT_REFLEX_PRIORITY = 100` |
| `docs/adr/0005-multi-monitor-capture-target.md` | Multi-monitor capture target | Resolution rules for `Primary`/`Monitor`/`Window`/`ElementWindow` capture targets across multi-monitor configurations |
| `docs/adr/0006-profile-match-precedence.md` | Profile match precedence | When multiple profiles match the current foreground, the most-specific match (most non-`None` fields satisfied) wins; ties broken by load order |
| `docs/adr/0007-per-event-vs-batched-notifications.md` | Per-event vs batched SSE notifications | One Event = one SSE frame; no in-process batching to keep `event-to-subscriber p99 ≤ 50 ms` achievable |

## 5. Operator-level invariants (from `docs/impplan/00_methodology.md`)

These are doctrine — **NEVER violate**:

1. **No backward compatibility (pre-v1).** Schema/API changes break callers; no fallbacks, no shims, no silent error swallowing. Anything that does not work must fail fast with a structured `synapse_core::error_codes::*` code and a tracing log line containing that code.
2. **No mocks gate completion.** OS-bound work-items are not done until a real-OS integration test exercises them against the real SoT (UIA `ValuePattern`, `XInputGetState`, RocksDB key, `GetClipboardData`, `GetCursorPos`, low-level keyboard hook, etc.).
3. **Full-State Verification (FSV) is mandatory and manual.** The agent reads the SoT before, executes the trigger, performs a separate read for "after", exercises ≥3 edge cases (empty/boundary/structurally-invalid), and records actual state. **Scripts, tests, benchmarks, harnesses, GitHub Actions, and CI are supporting evidence only.** They never count as FSV. Do not add `*_fsv` tests, FSV harnesses, or FSV scripts.
4. **Natural-only motion (OQ-004 DECIDED 2026-05-22).** `Natural` curves + `Natural` keystroke dynamics tuned `FAST` are the resolved default of every tool, profile, and reflex. `Instant`/`Burst` exist for explicit opt-in only.
5. **Manual FSV on the configured Windows host is the shipping gate, not CI** (operator decision 2026-05-24, issues #246/#247/#350/#351). Do not dispatch, wait on, or block a tag on GitHub Actions/CI. Do not add `*_fsv` tests.
6. **Missing configured-host prerequisites are agent work, not blockers.** Do not stop at "missing." If the operator could download, install, connect, configure, generate, flash, launch, or inspect it from this computer, the agent must use Synapse/local host control to make it happen and then inspect the physical SoT. Browser downloads, GUI installers, Device Manager checks, package-manager installs, model/file generation, firmware flashing, app launching, and UI inspection are agent-owned work when reversible on this host. Ask only for narrow approval on hard-to-reverse external actions after reversible local work is exhausted.

`AGENTS.md` reinforces these and pins **`[skip ci]` on every agent commit**.

## 6. Per-PR contract (from `docs/impplan/README.md`)

Every PR must satisfy:

```
✓ Compiles release + dev
✓ Clippy zero warnings (workspace + all-targets)
✓ Tests pass (`cargo test --workspace`)
✓ Files ≤ 500 LoC; functions ≤ 30 LoC; cyclomatic ≤ 10
✓ Error variants carry SCREAMING_SNAKE_CASE .code()
✓ Public APIs / CF names are `pub const`
✓ Tracing spans on every non-trivial fn
✓ No mocks gate completion (real captures, real RocksDB, real SendInput, real ViGEm)
✓ Schema change ⇒ wipe-and-rebuild (pre-v1, no shim)
✓ Bench delta ≤ 20% on tracked metrics
✓ Docs cross-refs intact (`scripts/check_docs.ps1`)
✓ Manual issue evidence captures SoT before/readback-after state
```

The 500-LoC file cap is violated in the following places per current code (HEAD `e54ca57`); M4's first PR splits them before adding hardware HID:

- `crates/synapse-mcp/src/server.rs` (1335 LoC) — tool router; exempt by design
- `crates/synapse-core/src/types.rs` (1567 LoC) — type catalog; exempt by design
- `crates/synapse-capture/src/lib.rs` (1798 LoC) — M4 Block A.0 splits
- `crates/synapse-mcp/src/m3/reflex.rs` (1165 LoC) — M4 Block A.0 splits
- `crates/synapse-reflex/src/lib.rs` (986 LoC) — M4 Block A.0 splits
- `crates/synapse-reflex/src/scheduler.rs` (890 LoC) — M4 Block A.0 splits
- `crates/synapse-mcp/src/http/sse.rs` (764 LoC) — M4 Block A.0 splits
- `crates/synapse-mcp/src/m3/replay.rs` (651 LoC) — M4 Block A.0 splits
- `crates/synapse-models/src/lib.rs` (535 LoC) — M4 Block A.0 splits

(`crates/synapse-a11y/src/lib.rs` was 2087 LoC at the start of M3 and is now 30 LoC after the platform/* split landed on `main` — this is the template for the M4 Block A.0 splits above.)

## 7. Performance budgets (binding — from PRD §11)

| Stage | Target p99 |
|---|---|
| Frame capture (zero-copy GPU surface) | ≤ 3 ms |
| Detection inference (small CNN on 5090-class GPU) | ≤ 8 ms |
| UIA tree snapshot for focused window | ≤ 10 ms |
| Full `observe()` response | ≤ 30 ms (`REFERENCE_OBSERVE_WARM_HYBRID_P99_MS`) |
| Event push from underlying frame/UIA event to subscriber | ≤ 50 ms (`REFERENCE_EVENT_TO_SUBSCRIBER_P99_MS`) |
| `act_aim` start-of-motion latency | ≤ 5 ms |
| `act_press` to electrical signal on USB | ≤ 2 ms (software) / ≤ 4 ms (hardware HID) |
| Reflex `on_event` action emission | ≤ 5 ms from event |
| Reflex scheduler tick jitter idle | ≤ 200 µs (`REFERENCE_REFLEX_TICK_JITTER_IDLE_P99_US`) |
| MCP idle-tick CPU usage | ≤ 1% on one core |
| Steady-state VRAM when models loaded | ≤ 2 GB |

These targets are verified via the criterion benches in `crates/*/benches/` and tracked in the bench-delta script (`scripts/check-bench-delta.ps1`, ≤20% regression gate).

## 8. Open questions (PRD `16_open_questions.md`) and their decisions

The PRD's "Open Questions" file enumerates roughly 30 numbered items (OQ-001 … OQ-029). The ones explicitly DECIDED that show up in code:

| OQ | Decision | Code/artifact |
|---|---|---|
| OQ-004 | Natural-only motion defaults (Natural curves + Natural keystroke dynamics tuned `FAST`) | `AimNaturalParams::FAST`, `KeystrokeNaturalParams::FAST` in `synapse-core/src/types.rs` |
| OQ-001 | RocksDB as primary storage | ADR-0002 |
| OQ-005 | Reflex priority semantics | ADR-0004 |
| OQ-012 | Multi-monitor capture target | ADR-0005 |
| OQ-015 | Profile match precedence | ADR-0006 |
| OQ-022 | Reflex recursion guard | ADR-0003 |
| OQ-029 | Per-event vs batched SSE notifications | ADR-0007 |
| OQ-009/010/023/024 | M1 perception closures (max_elements default, CDP auto-attach, element_id stability, token budget) | M1 source |
| operator decisions 2026-05-24 (issues #246/#247/#350/#351) | No GitHub Actions / CI as a shipping gate | `AGENTS.md` |

Open items remaining (PRD §16): OQ-003 (detection model default — YOLOv10n vs RT-DETR-s), OQ-013 (aim_track EMA smoothing), OQ-016 (action coalescing on hardware) closed in M4; OQ-008 (VLM bundling), OQ-014 (Whisper-tiny vs base), OQ-017 (disk-pressure thresholds final), OQ-019 (telemetry split), OQ-020 (`game_screenshot_once` exposure), OQ-030 (GC cadence final) closed in M5; OQ-006/007/021/027/028/026/018 remain v1.x.

## 9. Doctrine documents

| File | What it pins |
|---|---|
| `docs/computergames/README.md` | Project mission, repository layout, performance targets, authoring rules |
| `docs/computergames/00_vision_and_scope.md` | Non-goals, supported contexts |
| `docs/computergames/01_architecture.md` | Process boundaries, thread model, crate dep graph |
| `docs/computergames/02_perception.md` | Capture/A11y/OCR/Audio sensors and the perception mode auto-selector |
| `docs/computergames/03_action.md` | Action emitter design, backends, rate limits, curve/dynamics |
| `docs/computergames/04_reflex_runtime.md` | Reflex semantics, scheduler, conflict resolution |
| `docs/computergames/05_mcp_tool_surface.md` | The 30-tool registry (the contract) |
| `docs/computergames/06_data_schemas.md` | Wire schemas + error code catalog |
| `docs/computergames/07_storage_and_profiles.md` | RocksDB CFs, retention defaults, profile TOML |
| `docs/computergames/08_supported_use_policy.md` | Allowed/disallowed contexts, operator acknowledgments |
| `docs/computergames/09_hardware_hid_gateway.md` | M4 Pi Pico HID firmware + serial protocol + host driver |
| `docs/computergames/10_performance_budget.md` | Per-stage p99 targets + optimization rules |
| `docs/computergames/11_security_and_safety.md` | Threat model, permissions, redaction, kill switches |
| `docs/computergames/12_observability.md` | Logging, tracing, metrics, debug overlay, replay tool |
| `docs/computergames/13_testing_strategy.md` | Unit/integration/E2E, fixtures, manual FSV, perf regression |
| `docs/computergames/14_build_and_packaging.md` | Workspace, deps, profiles, installer, signing |
| `docs/computergames/15_roadmap_and_milestones.md` | M0-M5 phases, scope per milestone, demo criteria |
| `docs/computergames/16_open_questions.md` | Unresolved decisions, ADRs needed |
| `docs/computergames/17_research_appendix.md` | Web research, comparable projects, references |
| `docs/impplan/00_methodology.md` | Dev discipline, FSV protocol, work-item shape |
| `docs/impplan/0{1..6}_m{0..5}_*.md` | Per-milestone work-item ledger |
| `docs/impplan/07_cross_cutting.md` | Perf gates, security, observability, release |
| `docs/dev-host-hygiene.md` | Configured-host hygiene checklist |
| `docs/m1_error_throw_map.md` | M1 error-code throw-site map |
| `docs/AICodingAgentSuperPrompt.md` | Repository agent wake-up prompt |
| `docs/compressionprompt.md` | Doctrine for compressed implementation-plan authoring |

## 10. M3 demo gate (acceptance — passed 2026-05-25)

From `docs/impplan/04_m3_reflex_mcp_surface.md::§2`, validated for the `v0.1.0-m3` tag:

1. Real Win11 box. Notepad open. Claude Desktop configured with `synapse-mcp` over stdio.
2. Agent registers an `on_event` reflex that fires when a `Save As` window appears.
3. Agent observes Notepad, types text, and triggers Save As (Ctrl+S).
4. Reflex fires and emits the configured actions (type filename, press Enter), persists a `reflex_fired` audit row to `CF_REFLEX_AUDIT`, and updates an SSE subscriber if attached.
5. Operator verifies via direct UIA/file-system readback that:
   - The file exists.
   - The audit row is present in `CF_REFLEX_AUDIT`.
   - The reflex priority and lifetime evolved correctly.
6. Operator hotkey `Ctrl+Alt+Shift+P` cleanly disables all reflexes and fires `release_all` within 50 ms.

The M4 demo gate is defined in `docs/impplan/05_m4_hardware_hid_first_game.md` and exercises the RP2040 firmware + `synapse-hid-host` serial driver + Minecraft single-player creative world via `act_press`/`act_aim`/`act_combo` over `Backend::Hardware`.

## 11. What is NOT covered in this doc

- **Detailed per-issue history.** That lives in the GitHub issue tracker (https://github.com/ChrisRoyse/Synapse/issues). The impplan files reference issue numbers but do not duplicate full discussion threads.
- **Operator runbook / install steps.** Those are in `README.md` and `docs/dev-host-hygiene.md`.
- **Future v2 work (Linux / macOS / cross-platform).** Out of scope per PRD §"Out of scope".


---

<a id="file-13"></a>

> Source: `docs/systemspec/13_mcp_tool_reference.md`

# 13 — MCP Tool Reference

Source files covered:
- `crates/synapse-mcp/src/server.rs`
- `crates/synapse-mcp/src/m1.rs` (+ `m1/{ocr, search, sources}.rs`)
- `crates/synapse-mcp/src/m2/{aim, click, clipboard, drag, pad, press, release_all, scroll, type_text}.rs`
- `crates/synapse-mcp/src/m3/{audio, permissions, profile, reflex, replay, subscribe}.rs`
- `crates/synapse-core/src/types.rs`

All 30 tools below are registered on `SynapseService` via `#[tool(description=...)]` in `server.rs`. Tool descriptions are taken verbatim from the source. Every tool returns through `Json<T>` so the response shape exactly matches the deserialized response struct.

Default error response shape (all tools): `ErrorData { code: rmcp::ErrorCode(-32099), message, data: { "code": <SCREAMING_SNAKE_CASE> } }` via `crates/synapse-mcp/src/m1.rs::mcp_error`.

## 1. `health`

**Description:** "Return server health"
**Permissions:** none
**Side effects:** none

| Parameter | Type | Required | Default | Notes |
|---|---|---|---|---|
| (none) | — | — | — | uses an empty input schema (`empty_input_schema()`) |

**Returns:** `synapse_core::Health` (`{ ok, version, build, uptime_s, subsystems: BTreeMap<String, SubsystemHealth> }`). Subsystems: `storage`, `reflex`, `profiles`, `audio`, `http` (see [05_core_types_and_errors.md §5.8](#file-05)).

## 2. `observe`

**Description:** "Returns structured state of the focused window and surrounding context"
**Permissions:** none
**Side effects:** updates `M1State.last_observed_foreground` (used by `act_type`)

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `include` | `Vec<ObserveSlot>` | no | empty (→ defaults: `focused, elements, entities, hud, events`) | one of `focused`/`elements`/`entities`/`hud`/`audio`/`events`/`clipboard`/`fs`/`diagnostics` | Which slots to populate |
| `depth` | `u32` | no | `2` | `0..=6` | UIA tree depth cap |
| `max_elements` | `usize` | no | `60` | `1..=500` | Tree node cap |
| `since_event_seq` | `u64` | no | — | — | When set, `recent_events` filtered to `seq > since` |

**Returns:** `synapse_core::Observation`.
**Errors:** `OBSERVE_NO_PERCEPTION_AVAILABLE` (forced via `SYNAPSE_MCP_FORCE_NO_PERCEPTION`), `OBSERVE_INTERNAL` (forced or assembler error), `A11Y_NO_FOREGROUND`, `CAPTURE_TARGET_LOST`, perception subsystem errors.

## 3. `find`

**Description:** "Search visible accessibility nodes and detected entities"
**Permissions:** none
**Side effects:** none

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `query` | `Option<String>` | no | — | Free-text query |
| `role` | `Option<String>` | no | — | UIA role filter |
| `name_substring` | `Option<String>` | no | — | Name substring filter |
| `automation_id` | `Option<String>` | no | — | UIA automation id |
| `scope` | `Option<FindScope>` | no | `Both` | `Elements` / `Entities` / `Both` |
| `limit` | `Option<usize>` | no | `5` | Clamped `1..=20` |
| `in_window` | `Option<ElementId>` | no | — | Restrict scan to a window |

**Returns:** `FindResponse { results: Vec<FindResult> }` sorted by descending `score`. Each `FindResult` carries `kind: Element|Entity`, identifiers, name/role/automation_id/class_label, `bbox: Rect`, `score: f32`.

## 4. `read_text`

**Description:** "OCR text from a screen region or visible element"
**Permissions:** none
**Side effects:** runs OCR (WinRT)

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `region` | `Option<Rect>` | no | — | Screen-coord region |
| `element_id` | `Option<ElementId>` | no | — | UIA element to OCR; falls back to focused element if neither given |
| `backend` | `OcrBackend` | no | `Auto` | Schema field; currently always WinRT in live code |
| `lang_hint` | `Option<String>` | no | — | BCP-47 language tag (e.g. `en-US`) |

**Returns:** `synapse_core::OcrResult { full_text, words: Vec<OcrWord>, confidence, region, lang }`.
**Errors:** `OCR_NO_TEXT`, `OCR_BACKEND_UNAVAILABLE`, `A11Y_ELEMENT_STALE`, `CAPTURE_TARGET_LOST`.

## 5. `set_capture_target`

**Description:** "Set the active capture target"
**Permissions:** none
**Side effects:** updates `M1State.capture_config`; increments `capture_generation`

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `target` | `CaptureTargetParam` | yes | — | `Primary` \| `Monitor { monitor_index: u32 }` \| `Window { window_hwnd: i64 }` \| `ElementWindow { element_id }` |
| `min_update_interval_ms` | `Option<u64>` | no | — | Forced `>= 1` |
| `cursor_visible` | `Option<bool>` | no | — | |
| `dirty_region_only` | `Option<bool>` | no | — | |

**Returns:** `SetCaptureTargetResponse { previous: CaptureTargetWire, current: CaptureTargetWire, generation: u64, backend: String }` where `backend ∈ {"graphics_capture_api", "dxgi_duplication"}`.
**Errors:** `CAPTURE_TARGET_INVALID` (no monitor, no window, invalid element id).

## 6. `set_perception_mode`

**Description:** "Set the active perception mode"
**Permissions:** none
**Side effects:** updates `M1State.perception_mode`

| Parameter | Type | Required | Default | Valid | Description |
|---|---|---|---|---|---|
| `mode` | `String` | yes | — | `auto`/`a11y_only`/`pixel_only`/`hybrid` | Parsed via `synapse_perception::parse_perception_mode` |

**Returns:** `SetPerceptionModeResponse { previous, mode, rationale }` where `rationale ∈ {"auto_select_by_foreground_and_a11y_density", "manual_a11y_only", "manual_pixel_only", "manual_hybrid"}`.
**Errors:** `PERCEPTION_MODE_INVALID`.

## 7. `act_click`

**Description:** "Click a screen coordinate or UI Automation element"
**Permissions:** `INPUT_MOUSE` (via reflex registration paths; tool itself doesn't gate at server.rs); the action's `backend` adds `INPUT_HARDWARE_HID` if `Hardware` is chosen.
**Side effects:** mouse movement + button click(s); appends to `RecordingBackend` if enabled

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `target` | `ActClickTarget` | yes | — | — | `Element { element_id }` or `Point { x: i32, y: i32 }` |
| `button` | `MouseButton` | no | `Left` | enum | |
| `clicks` | `u8` | no | `1` | `1..=3` | |
| `modifiers` | `Vec<ClickModifier>` | no | `[]` | `Ctrl`/`Shift`/`Alt`/`Super` | Non-empty → `ACTION_BACKEND_UNAVAILABLE` "act_click modifiers are not wired in the M2 click schema slice" |
| `curve` | `ClickCurve` | no | `Natural` | `Natural`/`Instant`/`Linear`/`EaseInOut` | Lowered to `AimCurve::Natural { params: FAST }` etc. |
| `duration_ms` | `u32` | no | `50` | — | Movement duration |
| `backend` | `Backend` | no | `Auto` | enum | |
| `use_invoke_pattern` | `bool` | no | `true` | — | When target is `Element` and the element supports UIA `Invoke`, the invoke pattern is used; coordinate fallback otherwise |

**Returns:** `ActClickResponse { ok: bool, used_invoke_pattern: bool, backend_used: String, double_click_window_ms: u32, inter_click_delay_ms: u32, elapsed_ms: u32 }`.
**Errors:** `TOOL_PARAMS_INVALID` (clicks out of range), `ACTION_BACKEND_UNAVAILABLE` (modifiers), `ACTION_ELEMENT_NOT_RESOLVED`, `ACTION_RATE_LIMITED`.

## 8. `act_type`

**Description:** "Type text through the active keyboard backend"
**Side effects:** keystroke synthesis (foreground check enforced)
**Pre-call check:** `SynapseService::ensure_act_type_foreground` compares `M1State.last_observed_foreground.hwnd` against `synapse_a11y::current_foreground_context().hwnd`. Mismatch → `ACTION_FOREGROUND_LOST` with a structured warn (`M2_ACT_TYPE_FOREGROUND_LOST`).

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `text` | `String` | yes | — | UTF-8; surrogate pairs split via `KeystrokeEvent` lowering |
| `into_element` | `Option<ElementId>` | no | — | If set, the assembler is expected to have focused it first (currently advisory) |
| `dynamics` | `TypeDynamics` | no | `Natural` | `Burst`/`Linear`/`Natural` |
| `linear_ms_per_char` | `u32` | no | — | Only used when `dynamics = Linear` |
| `use_scancodes` | `bool` | no | — | When true, keys emit with `use_scancode = true` |
| `press_enter_after` | `bool` | no | `false` | Appends a `KeyPress { Key::Named("enter") }` |
| `backend` | `TypeBackend` | no | `Auto` | `Software` / `Hardware` / `Auto` |

**Returns:** `ActTypeResponse { ok, chars_typed: u32, elapsed_ms: u32 }`.
**Errors:** `ACTION_FOREGROUND_LOST`, `ACTION_RATE_LIMITED`, `ACTION_BACKEND_UNAVAILABLE`, `ACTION_UNSUPPORTED_KEY` (only when individual chars lower to unsupported keys).

## 9. `act_press`

**Description:** "Press a keyboard key or ordered chord"
**Side effects:** Action::KeyPress (one key) or Action::KeyChord (multiple).

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `keys` | `Vec<String>` | yes | — | `len >= 1` | Names parsed by `m2/press/keys.rs`. Single entry → `KeyPress`; multiple → `KeyChord` |
| `hold_ms` | `u32` | no | `33` | `1..=30000` | |
| `backend` | `PressBackend` | no | `Auto` | `Software`/`Hardware`/`Auto` | |

**Returns:** `ActPressResponse { ok, keys_pressed: u32, elapsed_ms: u32, backend_used: String }`.
**Errors:** `ACTION_UNSUPPORTED_KEY`, `ACTION_RATE_LIMITED`, `ACTION_BACKEND_UNAVAILABLE` (`Hardware` until M4).

## 10. `act_aim`

**Description:** "Move the pointer toward a screen, element, or track target"
**Side effects:** `Action::MouseMove` (or recording of same).

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `target` | `ActAimTarget` | yes | — | `Point { x, y }` \| `Element { element_id }` \| `Track { track_id }` |
| `style` | `AimStyleParam` | no | `Snap` | `Snap` / `Flick` / `Natural` / `Track` |
| `deadline_ms` | `u32` | no | `80` | Effective duration: Snap=50, Flick=35, Natural=150, anything else uses `deadline_ms` |
| `backend` | `AimBackend` | no | `Auto` | `Software`/`Hardware`/`Auto` |

**Returns:** `ActAimResponse { ok, style_used, duration_ms, backend_used, elapsed_ms }`.
**Errors:** `ACTION_BACKEND_UNAVAILABLE` (track style or element target — both return this with detail "requires the dedicated target resolution issue" / "requires the reflex runtime lands at M3"), `ACTION_RATE_LIMITED`.

## 11. `act_drag`

**Description:** "Drag between screen coordinates or element centers"
**Side effects:** `Action::MouseDrag`.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `from` | `ActDragTarget` | yes | — | `Point` or `Element` |
| `to` | `ActDragTarget` | yes | — | `Point` or `Element` |
| `button` | `DragButton` | no | `Left` | `Left`/`Right`/`Middle` |
| `curve` | `DragCurve` | no | `Natural` | `Natural`/`Instant`/`Linear`/`EaseInOut` |
| `duration_ms` | `u32` | no | `200` | |
| `backend` | `DragBackend` | no | `Auto` | `Software`/`Hardware`/`Auto` |

**Returns:** `ActDragResponse { ok, button_used, curve_used, duration_ms_used, elapsed_ms, backend_used, ... }`.
**Errors:** `ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT` (> `MAX_DRAG_DISTANCE_PX = 4096.0`), `ACTION_ELEMENT_NOT_RESOLVED`, `ACTION_RATE_LIMITED`.

## 12. `act_scroll`

**Description:** "Scroll vertically or horizontally at the current pointer or screen point"
**Side effects:** one or more `Action::MouseScroll` events.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `dy` | `i32` | no | `0` | Vertical wheel ticks |
| `dx` | `i32` | no | `0` | Horizontal wheel ticks |
| `at` | `Option<ActScrollPoint { x: i32, y: i32 }>` | no | — | Mouse position when scrolling |
| `smooth` | `bool` | no | `false` | When true, splits into events scheduled every `SMOOTH_SCROLL_INTERVAL_MS = 30 ms`, max `MAX_SMOOTH_SCROLL_STEPS = 120` |

**Returns:** `ActScrollResponse { ok, dy, dx, smooth, scrolled: bool, wheel_event_count: u32, smooth_interval_ms: u32, scheduled_smooth_total_ms: u32, backend_used: String, elapsed_ms: u32 }`. `dy=0,dx=0` is a no-op that returns `scrolled=false`.

## 13. `act_pad`

**Description:** "Apply a virtual gamepad report and optionally return it to neutral"
**Side effects:** `Action::PadReport` via ViGEm.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `pad_id` | `PadId` (u8) | no | `0` | — | ViGEm slot |
| `controller` | `ActPadController` | no | `X360` | `X360`/`Ds4` | |
| `report` | `ActPadReport` | yes | — | — | buttons + axes + triggers |
| `backend` | `PadBackend` | no | `Vigem` | `Vigem`/`Hardware` | |
| `hold_ms` | `Option<u32>` | no | — | `<= 30_000` | If set, schedules a return-to-neutral `PadReport` after the hold |

`ActPadReport`:

| Field | Type | Default | Range |
|---|---|---|---|
| `buttons` | `Vec<ActPadButton>` | `[]` | each ∈ `A`/`B`/`X`/`Y`/`Lb`/`Rb`/`Ls`/`Rs`/`Back`/`Start`/`Up`/`Down`/`Left`/`Right` |
| `thumb_l` | `(f32, f32)` | `(0.0, 0.0)` | each in `[-1.0, 1.0]` |
| `thumb_r` | `(f32, f32)` | `(0.0, 0.0)` | each in `[-1.0, 1.0]` |
| `lt` | `f32` | `0.0` | `[0.0, 1.0]` |
| `rt` | `f32` | `0.0` | `[0.0, 1.0]` |

**Returns:** `ActPadResponse { ok, pad_id, controller, buttons, backend_used, hold_ms, returned_to_neutral: bool, elapsed_ms }`.
**Errors:** `ACTION_VIGEM_NOT_INSTALLED`, `ACTION_VIGEM_PLUGIN_FAILED`, `ACTION_RATE_LIMITED`, `ACTION_HOLD_EXCEEDED_MAX`.

## 14. `act_clipboard`

**Description:** "Read, write, or clear the system clipboard"
**Side effects:** Win32 clipboard read/write/clear.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `verb` | `ActClipboardVerb` | yes | — | `Read`/`Write`/`Clear` |
| `text` | `Option<String>` | required for `Write` | — | Forbidden for `Read`/`Clear` |
| `format` | `ActClipboardFormat` | no | `Unicode` | `Text` (ASCII only) \| `Unicode` |

**Returns:** `ActClipboardResponse { ok, verb, format, written, cleared, text, text_len, elapsed_ms }`.
**Errors:** `TOOL_PARAMS_INVALID` (verb=write without text; verb!=write with text; format=text + non-ASCII).

## 15. `release_all`

**Description:** "Release all held keyboard, mouse, and gamepad input state"
**Side effects:** `Action::ReleaseAll` (KeyUp every held key, MouseButton::Up every held button, neutralize every tracked pad).

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| (none) | — | — | — | Empty params struct |

**Returns:** `ReleaseAllResponse { released_keys: u32, released_buttons: u32, neutralized_pads: u32 }`. The implementation snapshots before, executes `Action::ReleaseAll`, snapshots after, and asserts the held lists drained — `TOOL_INTERNAL_ERROR` if state remains held.

## 16. `subscribe`

**Description:** "Subscribe to filtered event notifications"
**Permissions:** `READ_EVENTS`

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `kinds` | `Vec<String>` | no | `[]` | Allow-list of `Event.kind`s. Empty → all kinds (subject to `filter`) |
| `filter` | `Option<EventFilter>` | no | — | Validated tree (depth ≤ `EVENT_FILTER_MAX_DEPTH = 8`); missing → `EventFilter::All` |
| `snapshot_first` | `bool` | no | `false` | (Reserved; ignored by the live SSE state) |
| `buffer_size` | `u32` | no | `4096` | **Must equal `4096`**; any other value → `TOOL_PARAMS_INVALID` |

**Returns:** `SubscribeResponse { subscription_id: String, started_at: DateTime<Utc> }`. The subscription id is consumed by `GET /events?subscription_id=...` over HTTP (`crates/synapse-mcp/src/http/sse.rs`).
**Errors:** `TOOL_PARAMS_INVALID`, `SUBSCRIPTION_CAP_REACHED`, `REFLEX_FILTER_INVALID`.

## 17. `subscribe_cancel`

**Description:** "Cancel an event subscription"
**Permissions:** `READ_EVENTS`

| Parameter | Type | Required | Description |
|---|---|---|---|
| `subscription_id` | `String` | yes | Trimmed; empty → `TOOL_PARAMS_INVALID` |

**Returns:** `SubscribeCancelResponse { cancelled: bool, reason: SubscribeCancelReason }` (`reason = Ok` on success).
**Errors:** `SUBSCRIPTION_NOT_FOUND`.

## 18. `reflex_register`

**Description:** "Register a reflex"
**Permissions:** `WRITE_REFLEX` plus any input permissions implied by `then` actions (`INPUT_KEYBOARD`/`INPUT_MOUSE`/`INPUT_PAD`/`INPUT_HARDWARE_HID`).
**Side effects:** opens RocksDB on first call; persists a `reflex_registered` audit row; starts the scheduler thread on first reflex.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `kind` | `String` | yes | — | `aim_track` / `hold_move` / `hold_button` / `combo` / `on_event` | Reflex kind |
| `when` | `Option<ReflexWhenParam>` | for `on_event` | — | EventFilter or window-event match | |
| `then` | `ReflexThenParam` | yes | — | Either a `ReflexThen` (Action / Actions / Combo) or `{ steps: Vec<ReflexThenStep { action: String, params: Value }> }` | Action(s) to fire |
| `priority` | `u32` | no | `100` | `0..=1000` | Lower = higher priority. (`DEFAULT_REFLEX_PRIORITY` / `MAX_REFLEX_PRIORITY`) |
| `lifetime` | `ReflexLifetime` | no | `UntilCancelled` | enum | `UntilCancelled` / `OneShot` / `Duration { ms }` / `UntilEvent { filter }` / `UntilDeadline { ms }` |
| `backend` | `Backend` | no | `Auto` | enum | Default backend for the reflex's actions |
| `exclusive` | `bool` | no | `false` | — | If true, conflicts with other exclusive reflexes are resolved by priority |

**Returns:** `ReflexRegisterResponse { reflex_id: String, state: ReflexStatus }`.
**Errors:** `REFLEX_KIND_INVALID`, `REFLEX_PARAMS_INVALID`, `REFLEX_TARGET_INVALID`, `REFLEX_FILTER_INVALID`, `REFLEX_PRIORITY_INVALID`, `REFLEX_CAP_REACHED` (`MAX_SCHEDULED_REFLEXES = 32`).

## 19. `reflex_cancel`

**Description:** "Cancel a reflex"
**Permissions:** `READ_REFLEX`
**Side effects:** persists a `reflex_cancelled` audit row.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `reflex_id` | `String` | yes | Trimmed; empty → `TOOL_PARAMS_INVALID` |

**Returns:** `ReflexCancelResponse { cancelled: bool, reason: ReflexCancelReason }` (reasons: `Ok`/`NotFound`/`AlreadyExpired`).

## 20. `reflex_list`

**Description:** "List registered reflexes"
**Permissions:** `READ_REFLEX`

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `include_expired` | `bool` | no | `false` | When true, reconstructs terminal statuses from `CF_REFLEX_AUDIT` |

**Returns:** `ReflexListResponse { reflexes: Vec<ReflexStatus> }`.

## 21. `reflex_history`

**Description:** "Return persisted reflex audit history"
**Permissions:** `READ_REFLEX`

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `reflex_id` | `Option<String>` | no | — | — | If present, scans `CF_REFLEX_AUDIT` by `<reflex_id>:` prefix |
| `limit` | `u32` | no | `50` | `0..=1000` | Caps the number of audit rows returned |

**Returns:** `ReflexHistoryResponse { events: Vec<StoredReflexAudit> }` newest-first.
**Errors:** `TOOL_PARAMS_INVALID` (limit > 1000).

## 22. `profile_list`

**Description:** "List loaded profiles"
**Permissions:** `READ_PROFILE`

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `include_inactive` | `bool` | no | `true` | When false, only the active profile is returned |

**Returns:** `ProfileListResponse { profiles: Vec<ProfileStatus>, active_profile_id: Option<String> }`. Each `ProfileStatus` carries `id`, `label`, `matches: Vec<ProfileMatchStatus>`, `active: bool`, `schema_version: u32`.

## 23. `profile_activate`

**Description:** "Activate a loaded profile by id"
**Permissions:** `WRITE_PROFILE_ACTIVE`
**Side effects:** updates `ProfileRuntime` active state in memory (no FS write, no `CF_PROFILES` write in current build).

| Parameter | Type | Required | Description |
|---|---|---|---|
| `profile_id` | `String` | yes | Must match a parsed profile id |

**Returns:** `ProfileActivateResponse { profile_id, active_profile_id, previous_active_profile_id, changed: bool }`. `changed=false` if `profile_id` was already active.
**Errors:** `PROFILE_NOT_FOUND`, `SAFETY_PROFILE_ACTION_DENIED` (use_scope=Unknown without `--allow-unknown-profile`).

## 24. `replay_record`

**Description:** "Record observations and/or events to a replay JSONL file"
**Permissions:** `WRITE_REPLAY`
**Side effects:** writes a JSONL file under `%LOCALAPPDATA%/synapse/replays` (or operator-specified absolute path under that root).

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `target` | `String` | no | `"observations"` | `observations` / `events` / `both` |
| `format` | `String` | no | `"jsonl"` | Only `jsonl` accepted |
| `duration_ms` | `u32` | yes | — | `>= 0`; how long to record |
| `path` | `Option<String>` | no | — | Relative paths joined to `replay_root()`; lexical-normalized; must stay under root |

**Returns:** `ReplayRecordResponse { path: String, records_written: u64, bytes: u64 }`.

Recording cadence: observations sampled every `OBSERVATION_SAMPLE_INTERVAL = 250 ms`; events drained every `EVENT_DRAIN_INTERVAL = 20 ms`.

**Errors:** `REPLAY_TARGET_INVALID`, `REPLAY_FORMAT_INVALID`, `SAFETY_PERMISSION_DENIED` (path outside allow-root), `TOOL_PARAMS_INVALID`.

## 25. `audio_tail`

**Description:** "Return the latest loopback audio tail as PCM s16le bytes"
**Permissions:** `READ_AUDIO` (requires `--enable-audio`)
**Side effects:** none (reads the existing ring; loopback must be running or the runtime initialized on demand)

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `seconds` | `u32` | no | `5` | `0..=MAX_RING_SECONDS=5` | `0` returns an empty PCM body |

**Returns:** `AudioTailResponse { pcm: Vec<u8>, sample_rate: u32, channels: u16, format: "s16le" }`. The PCM is **left-padded with zeros** when the ring contains fewer samples than requested, so `pcm.len() == seconds * sample_rate * channels * 2`.

**Errors:** `TOOL_PARAMS_INVALID` (seconds > 5), `AUDIO_LOOPBACK_INIT_FAILED`, `AUDIO_DEVICE_LOST`.

## 26. `audio_transcribe`

**Description:** "Transcribe the latest loopback audio tail with Whisper tiny"
**Permissions:** `READ_AUDIO`
**Side effects:** loads Whisper-tiny on first call (one-shot SHA-256 verification + ORT session bring-up); runs inference.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `seconds` | `u32` | no | `5` | `0..=5` | Window size |
| `language` | `String` | no | `"en"` | `"en"` only (case-insensitive, empty → `"en"`) | Anything else → `TOOL_PARAMS_INVALID` |

**Returns:** `AudioTranscribeResponse { text: String, confidence: f32, latency_ms: u64, model_id: "whisper_tiny_int8" }`.

**Errors:** `TOOL_PARAMS_INVALID`, `AUDIO_STT_MODEL_NOT_LOADED`, `MODEL_HASH_MISMATCH`, `MODEL_LOAD_FAILED`, `MODEL_BACKEND_UNAVAILABLE`.

## 27. `storage_inspect`

**Description:** "Inspect RocksDB column families: row counts and byte sizes"
**Permissions:** none gated at the M3 layer (operator-only tool surface; not in the agent-facing 30-tool PRD list)
**Side effects:** none; reads `Db::cf_sizes` and per-CF scan counts

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `cf` | `Option<String>` | no | — | When set, restricts the report to one CF (must match one of `ALL_COLUMN_FAMILIES`); otherwise all 11 CFs |

**Returns:** `StorageInspectResponse { db_path: String, schema_version: u32, pressure_level: String, cfs: Vec<StorageCfInspectRow { name, row_count, bytes }> }`.
**Errors:** `STORAGE_OPEN_FAILED`, `TOOL_PARAMS_INVALID` (unknown CF name).

## 28. `storage_put_probe_rows`

**Description:** "Insert probe rows into a CF to exercise the write batcher + flush + GC paths"
**Permissions:** none gated at the M3 layer (operator-only)
**Side effects:** writes N synthetic rows into the chosen CF; calls `Db::flush`.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `cf` | `String` | yes | — | one of `ALL_COLUMN_FAMILIES` | Target CF |
| `count` | `u32` | yes | — | `1..=10000` | Number of probe rows |
| `value_bytes` | `Option<u32>` | no | `256` | `1..=65536` | Per-row payload size |

**Returns:** `StoragePutProbeRowsResponse { cf, rows_written, bytes_written, flush_elapsed_ms }`.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_WRITE_FAILED`, `STORAGE_DISK_PRESSURE_LEVEL_1..4` (writes silently dropped at the higher pressure levels).

## 29. `storage_gc_once`

**Description:** "Run one synchronous storage GC pass and return per-CF before/after sizes"
**Permissions:** none gated at the M3 layer (operator-only)
**Side effects:** evicts rows from any CF whose size exceeds its soft cap; emits `cache_evictions_total{cf,reason="soft_cap"}` counter increments.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| (none) | — | — | — | Empty params |

**Returns:** `StorageGcOnceResponse { elapsed_ms, cf_reports: Vec<StorageGcCfReport { cf, before_bytes, after_bytes, rows_evicted, hit_hard_cap: bool }> }`.
**Errors:** `STORAGE_OPEN_FAILED`.

## 30. `storage_pressure_sample`

**Description:** "Apply one synthetic free-byte sample to drive the disk-pressure responder"
**Permissions:** none gated at the M3 layer (operator-only)
**Side effects:** updates `Db::pressure_level()` for subsequent writes; may trigger compaction on selected CFs at higher levels.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `free_bytes` | `u64` | yes | — | Synthetic free-bytes value applied via `Db::run_pressure_check_with_free_bytes_sample` |

**Returns:** `StoragePressureSampleResponse { previous_level: String, current_level: String, frozen_cfs: Vec<String> }`. Levels: `Normal` / `Level1` / `Level2` / `Level3` / `Level4`.
**Errors:** `STORAGE_OPEN_FAILED`.

## Permission mapping reference

For convenience the M3 tool-call gating is summarized here (live source: `crates/synapse-mcp/src/m3/permissions.rs`, plus per-module `required_permissions_*` functions):

| Tool | Required permissions |
|---|---|
| `subscribe`, `subscribe_cancel` | `READ_EVENTS` |
| `reflex_register` | `WRITE_REFLEX` + actions' permissions |
| `reflex_cancel`, `reflex_list`, `reflex_history` | `READ_REFLEX` |
| `profile_list` | `READ_PROFILE` |
| `profile_activate` | `WRITE_PROFILE_ACTIVE` |
| `replay_record` | `WRITE_REPLAY` |
| `audio_tail`, `audio_transcribe` | `READ_AUDIO` |
| `storage_inspect`, `storage_put_probe_rows`, `storage_gc_once`, `storage_pressure_sample` | (operator-only — no M3 permission gate; support manual FSV from the configured host) |

`reflex_register`'s effective permission set is computed by `add_action_permissions` over the compiled `Vec<Action>` (e.g., `Action::PadReport` requires `INPUT_PAD`; any action with `Backend::Hardware` adds `INPUT_HARDWARE_HID`).

M1/M2 tools (`health`, `observe`, `find`, `read_text`, `set_capture_target`, `set_perception_mode`, `act_*`, `release_all`) do not gate at the M3 permission layer because they predate M3; the M3 permission layer applies only to the M3 tool surface. (For reflex-driven action emission, the reflex-register permission check is the gating point.)

## Cross-references

- Type definitions: [05_core_types_and_errors.md](#file-05)
- Service / dispatch: [06_mcp_service_and_transports.md](#file-06)
- Reflex semantics: [07_reflex_runtime.md](#file-07)
- Action emitter contract: [08_action_subsystem.md](#file-08)
- Perception assembly: [09_perception_and_capture.md](#file-09)
- Audio: [10_audio_and_models.md](#file-10)
- Storage CFs: [04_storage_layer.md](#file-04)
- Configuration knobs: [03_configuration.md](#file-03)


---

<a id="file-14"></a>

> Source: `docs/systemspec/14_test_suite.md`

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
| **Benchmarks** | `crates/*/benches/*.rs` (criterion) | Perf-regression detection against §7 budgets in [12_milestones_and_roadmap.md](#file-12) |
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


---

<a id="file-15"></a>

> Source: `docs/systemspec/15_verification_report.md`

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

Per the doctrine in [12_milestones_and_roadmap.md §5](#file-12) ("operator-level invariants"), the **shipping gate is manual FSV on the configured Windows host, not `cargo test`**. Running `cargo test --workspace` is the supporting-evidence test gate for the per-PR contract.

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
| Total Rust source files (excluding tests/benches) | **148** |
| Total Rust integration-test files | 76 |
| Total Rust bench files | 13 |
| MCP tools registered in `server.rs` | **30** (M1: 6, M2: 9, M3: 15 — dispatched via `m3_tool_stubs() len=15`; M3 includes 4 operator-only `storage_*` diagnostics added beyond the PRD agent surface) |
| MCP tools planned by PRD `05_mcp_tool_surface.md` (agent surface cap) | 30 |
| RocksDB column families | **11** (`ALL_COLUMN_FAMILIES.len() == 11`; excludes implicit `default` CF) |
| Stable error-code constants in `synapse_core::error_codes` | **95** |
| Reserved subsystem error enums (mapped to those codes) | 11 (`StorageError`, `ReflexError`, `ActionError`, `ProfileError`, `ProfileLoadError`, `AudioError`, `PerceptionError`, `CaptureError`, `ModelError`, `A11yError`, `TelemetryError` + parse errors `ElementIdParseError`/`EventFilterValidationError`) |
| M3 metric specs declared in `synapse_telemetry::metrics::M3_METRICS` | **19** (12 counters, 5 gauges, 2 histograms) |
| Permissions in M3 grant model | **11** (`READ_EVENTS`, `WRITE_REFLEX`, `READ_REFLEX`, `READ_PROFILE`, `WRITE_PROFILE_ACTIVE`, `WRITE_REPLAY`, `READ_AUDIO`, `INPUT_KEYBOARD`, `INPUT_MOUSE`, `INPUT_PAD`, `INPUT_HARDWARE_HID`) |
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
| `synapse-core` | `src/error_codes.rs` | 112 |
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
| [01_system_overview.md](#file-01) | High-level architecture, tech stack, all 30 live tools |
| [02_source_code_map.md](#file-02) | Per-file tree with descriptions + dep graph + entry-point traces |
| [03_configuration.md](#file-03) | All CLI flags, env vars, validation rules, default constants |
| [04_storage_layer.md](#file-04) | RocksDB CFs, schema sentinel, TTL filter, GC, disk pressure |
| [05_core_types_and_errors.md](#file-05) | `synapse-core` types, error codes, Stored* variants |
| [06_mcp_service_and_transports.md](#file-06) | `SynapseService`, stdio + HTTP routers, Bearer/Origin/Session middleware, SSE bridge |
| [07_reflex_runtime.md](#file-07) | EventBus, scheduler, the 5 reflex kinds, audit persistence |
| [08_action_subsystem.md](#file-08) | Emitter actor, backends, rate limits, hotkey, curves/dynamics, error mapping |
| [09_perception_and_capture.md](#file-09) | Frame capture, UIA + WinEvent, perception assembler, OCR |
| [10_audio_and_models.md](#file-10) | WASAPI loopback, ring + STT (Whisper-tiny), ONNX model loader |
| [11_profiles_hid_telemetry.md](#file-11) | TOML profile loader + watcher, HID stub, tracing + metrics, test utils |
| [12_milestones_and_roadmap.md](#file-12) | Milestone state, ADRs, doctrine, open questions |
| [13_mcp_tool_reference.md](#file-13) | Every tool's params, defaults, ranges, side effects, errors |
| [14_test_suite.md](#file-14) | Test inventory by crate, run commands, fixtures |
| [15_verification_report.md](#file-15) | This file |

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

