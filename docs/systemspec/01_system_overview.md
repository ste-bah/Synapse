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

The repository operates under the doctrine in `AGENTS.md`: manual Full State Verification (FSV) on the configured Windows host is the shipping gate; GitHub Actions, CI, scripts, tests, and benches are supporting evidence only; missing configured-host prerequisites are acquisition/setup work where the agent figures out where the thing must come from, where it must physically appear, uses Synapse/local control as the operator-equivalent host control surface with full local computer-control responsibility and the same practical local ability the operator has at this keyboard to make it happen when reversible local steps exist, treats missing local state as the next agent action that must be made real rather than handed back or treated as a blocker while reversible host work remains, treats browser downloads, GUI installers, Device Manager checks, package-manager installs, model/file generation, firmware flashing, app launching, USB/COM inspection, and UI inspection as agent-owned work on this host, does not stop at "missing" when the operator could do it from this computer, and verifies the real source of truth; and agent commits include `[skip ci]`.

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
| Signatures | `ed25519-dalek` | `2.2.0` |
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

All 51 live tools live in `crates/synapse-mcp/src/server.rs` (declared via `#[tool_router]`). Grouped by milestone:

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
| `storage_inspect` | Return per-CF row counts/byte sizes plus audit-retention policy metadata from RocksDB for the operator-visible CFs | `server.rs::storage_inspect`, `m3/storage.rs` |
| `storage_put_probe_rows` | Insert bounded probe rows into a chosen CF so manual FSV can trigger storage writes, then separately read the RocksDB/log SoT | `server.rs::storage_put_probe_rows`, `m3/storage.rs` |
| `storage_gc_once` | Run one synchronous GC pass; `cf_name="AUDIT_RETENTION"` performs #463 audit retention/dedupe/backfill and writes a `CF_KV` report row | `server.rs::storage_gc_once`, `m3/storage.rs`, `m3/audit_retention.rs` |
| `storage_pressure_sample` | Apply one synthetic free-byte sample to drive the disk-pressure responder | `server.rs::storage_pressure_sample`, `m3/storage.rs` |

### 4.4 M4 — local shell/launch/combo (3 tools)

| Tool | Description | Source |
|---|---|---|
| `act_combo` | Schedule a timed one-shot sequence through the reflex combo scheduler | `server.rs::act_combo`, `m4.rs` |
| `act_run_shell` | Run an allowlisted local shell command | `server.rs::act_run_shell`, `m4.rs` |
| `act_launch` | Launch an allowlisted local process and optionally wait for a window | `server.rs::act_launch`, `m4.rs` |

### 4.5 M5 — profile registry/audit loop (18 tools)

| Tool | Description | Source |
|---|---|---|
| `profile_authoring_generate` | Generate a local candidate profile patch from bounded replay/audit evidence and persist/read it in `CF_PROFILES` | `server.rs::profile_authoring_generate`, `m3/profile_authoring.rs` |
| `profile_authoring_list` | List local profile-authoring candidate rows from `CF_PROFILES` | `server.rs::profile_authoring_list`, `m3/profile_authoring.rs` |
| `profile_authoring_inspect` | Read one candidate row from `CF_PROFILES/profile_authoring/v1/candidate/<candidate_id>` | `server.rs::profile_authoring_inspect`, `m3/profile_authoring.rs` |
| `profile_authoring_accept` | Mark a candidate accepted without activating or mutating the active profile | `server.rs::profile_authoring_accept`, `m3/profile_authoring.rs` |
| `profile_authoring_reject` | Mark a candidate rejected with an optional local reason | `server.rs::profile_authoring_reject`, `m3/profile_authoring.rs` |
| `profile_authoring_export` | Export one candidate row to a local JSON bundle file and read the written file back | `server.rs::profile_authoring_export`, `m3/profile_authoring.rs` |
| `profile_quality_refresh` | Refresh a local profile-quality snapshot from real `CF_ACTION_LOG` rows and persist/read it in `CF_PROFILES` | `server.rs::profile_quality_refresh`, `m3/profile_quality.rs` |
| `profile_registry_search` | Search local registry rows under `profile_registry/v1/` in `CF_PROFILES` | `server.rs::profile_registry_search`, `m3/profile_registry.rs` |
| `profile_registry_inspect` | Inspect one registry row in `CF_PROFILES` or registry head row in `CF_KV` | `server.rs::profile_registry_inspect`, `m3/profile_registry.rs` |
| `profile_registry_report` | Report installed registry packages, quarantine/rollback state, quality snapshots, consent/export status, recent audit evidence, and physical SoT pointers | `server.rs::profile_registry_report`, `m3/profile_registry.rs` |
| `profile_registry_install` | Validate a local package manifest/profile TOML, enforce signed trust policy where required, quarantine failed trust packages, write registry rows, and return row keys/readbacks | `server.rs::profile_registry_install`, `m3/profile_registry.rs` |
| `profile_registry_disable` | Mark an installed registry profile disabled or removed and read the stored row back | `server.rs::profile_registry_disable`, `m3/profile_registry.rs` |
| `profile_registry_export` | Export local registry rows or offline contribution bundles with deterministic hashes | `server.rs::profile_registry_export`, `m3/profile_registry.rs` |
| `profile_registry_import` | Import validated registry/contribution bundles into `CF_PROFILES`/`CF_KV`, skipping duplicates and failing closed on conflicts | `server.rs::profile_registry_import`, `m3/profile_registry.rs` |
| `profile_registry_rollback` | Restore an installed profile registry row to a prior trusted/local-validated package and write a rollback row | `server.rs::profile_registry_rollback`, `m3/profile_registry.rs` |
| `audit_intelligence_query` | Summarize profile-linked action/event/reflex/session outcomes and quality snapshots | `server.rs::audit_intelligence_query`, `m3/profile_registry.rs` |
| `audit_export_consent_set` | Write/read local audit-export consent state in `CF_KV/audit_export/v1/consent/<profile_id>` | `server.rs::audit_export_consent_set`, `m3/audit_export.rs` |
| `audit_export_bundle` | Export consented local redacted `CF_ACTION_LOG` rows into manifest/rows/redaction-report files | `server.rs::audit_export_bundle`, `m3/audit_export.rs` |

The profile-registry / audit-data network effect is the P1 strategic moat
tracked by #454 and child issues #455-#470. Profiles encode app/game operating
knowledge; runtime audit rows prove which decisions worked on this host; local
quality and compatibility scoring converts that evidence into better profile
packages; and registry distribution feeds improved profiles back into future
runs.

Physical sources of truth for this loop are profile TOML files, future registry
index/package files, RocksDB `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`, `CF_EVENTS`,
`CF_OBSERVATIONS`, `CF_SESSIONS`, and `CF_PROFILES` rows, consent/export
bundles, and MCP readbacks such as `profile_list`, `profile_quality_refresh`,
`profile_authoring_*`, `profile_registry_*`, `audit_intelligence_query`,
`audit_export_consent_set`, `audit_export_bundle`, and `storage_inspect`.
Manual FSV must trigger the
real runtime path and then read these physical stores directly; GitHub
Actions/CI and automated checks never substitute for FSV.

Registry package installs are treated as an execution-control supply-chain
surface. Signed-required packages are verified against local trust roots before
activation; failed signatures, missing signatures, and unknown signers fail
closed into quarantine rows. Rollbacks restore only prior package rows whose
stored trust state is `trusted` or `local_validated`.

M5 profile-linked audit linkage starts at `profile_activate`: the daemon writes
`CF_SESSIONS` and `CF_EVENTS` rows with `StoredAuditContext`, and action/reflex
paths propagate that same context into `CF_ACTION_LOG` and
`CF_REFLEX_AUDIT`. That keeps every local outcome joinable by
`session_id` + `profile_id` without requiring cloud upload.

Full parameter/return tables: [13_mcp_tool_reference.md](13_mcp_tool_reference.md).

### 4.6 PRD-planned tools NOT live in this build

`docs/computergames/05_mcp_tool_surface.md` defines the tool surface. Synapse's live build now has the M3 baseline, four operator storage diagnostics, M4 `act_combo`/`act_run_shell`/`act_launch`, profile HUD extraction through `observe`, and the M5 local registry/audit tools (`profile_quality_refresh`, `profile_registry_*` including rollback, `audit_intelligence_query`, `audit_export_consent_set`, and `audit_export_bundle`). The following PRD-planned entries remain unimplemented: `describe` (M5 VLM) and standalone `read_hud`.

## 5. Entry points

| Binary | Source | Purpose |
|---|---|---|
| `synapse-mcp` | `crates/synapse-mcp/src/main.rs` (clap CLI, `tokio::main`) | The MCP server; `--mode stdio` (default) or `--mode http` |
| `synapse-overlay` | `crates/synapse-overlay/src/main.rs` | Placeholder binary for the future debug overlay; current source is `fn main() {}` |
| (no other workspace binaries) | — | All other crates are libraries |

Default workspace members for build (`Cargo.toml`): `crates/synapse-mcp`, `crates/synapse-overlay`. All other crates are pulled in via the `[workspace.members]` list.

CLI flags on `synapse-mcp` (parsed in `main.rs::Cli`, full table: [03_configuration.md](03_configuration.md)):

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
--reset-hardware-consent
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
| Profile / config (PRD 8.4) | `PROFILE_NOT_FOUND`, `PROFILE_PARSE_ERROR`, `PROFILE_VERSION_INCOMPATIBLE`, `PROFILE_KEYMAP_INVALID`, `PROFILE_HUD_REGION_INVALID`, `CAPTURE_TARGET_INVALID`, `PERCEPTION_MODE_INVALID`, `PROFILE_TRUST_VERIFICATION_FAILED`, `PROFILE_ROLLBACK_UNAVAILABLE`, `AUDIT_EXPORT_CONSENT_REQUIRED`, `AUDIT_EXPORT_REDACTION_REQUIRED`, `AUDIT_EXPORT_PAYLOAD_TOO_LARGE`, `PROFILE_AUTHORING_INSUFFICIENT_EVIDENCE`, `PROFILE_AUTHORING_CONFLICTING_EVIDENCE`, `PROFILE_AUTHORING_UNSAFE_ESCALATION`, `PROFILE_AUTHORING_CANDIDATE_NOT_FOUND`, `PROFILE_AUTHORING_INVALID_STATE` | lines 56–72 |
| MCP / session (PRD 8.5) | `SESSION_NOT_FOUND`, `SESSION_EXPIRED`, `SUBSCRIPTION_NOT_FOUND`, `SUBSCRIPTION_CAP_REACHED`, `TOOL_NOT_FOUND`, `TOOL_PARAMS_INVALID`, `TOOL_INTERNAL_ERROR`, `HTTP_BIND_NON_LOOPBACK_REFUSED`, `HTTP_TOKEN_INVALID`, `HTTP_ORIGIN_REFUSED`, `HTTP_SESSION_INVALID`, `REPLAY_TARGET_INVALID`, `REPLAY_FORMAT_INVALID` | lines 75–87 |
| Storage (PRD 8.6) | `STORAGE_OPEN_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_READ_FAILED`, `STORAGE_CORRUPTED`, `STORAGE_SCHEMA_MISMATCH`, `STORAGE_DISK_PRESSURE_LEVEL_1..4`, `STORAGE_CF_HARD_CAP_REACHED` | lines 90–99 |
| Models (PRD 8.7) | `MODEL_DOWNLOAD_FAILED`, `MODEL_HASH_MISMATCH`, `MODEL_LOAD_FAILED`, `MODEL_BACKEND_UNAVAILABLE` | lines 102–105 |
| Hardware HID (PRD 8.8) | `HID_PORT_NOT_FOUND`, `HID_PORT_OPEN_FAILED`, `HID_PROTOCOL_HANDSHAKE_FAILED`, `HID_FIRMWARE_VERSION_MISMATCH`, `HID_COMMAND_REJECTED`, `HID_LINK_TIMEOUT` | lines 108–113 |
| Safety (PRD 8.9) | `SAFETY_KILLSWITCH_ACTIVE`, `SAFETY_PROCESS_DENYLISTED`, `SAFETY_SHELL_DENIED_BY_POLICY`, `SAFETY_LAUNCH_DENIED_BY_POLICY`, `SAFETY_SECRET_REDACTED`, `SAFETY_PERMISSION_DENIED`, `SAFETY_PROFILE_ACTION_DENIED` | lines 116–122 |

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
Shared types, schema version, retention defaults, and the canonical error code constants. `crates/synapse-core/src/types.rs` (1567 LoC) defines every wire-level struct/enum: `Action`, `Observation`, `Event`, `EventFilter`, `Profile`, `ReflexRegistration`, `Stored*` persistence variants, `Health`, `SubsystemHealth`, etc. `defaults.rs` pins `SCHEMA_VERSION = 1` and the reference-host performance budgets. `retention.rs` declares the 11-CF retention policy (TTL + soft/hard cap MB) used by storage GC. `filter.rs` evaluates `EventFilter` / `DataPredicate` against an `Event`. See [05_core_types_and_errors.md](05_core_types_and_errors.md).

### 9.2 synapse-mcp (binary + transports)
Owns the `synapse-mcp` binary, the `SynapseService` `ServerHandler` (`crates/synapse-mcp/src/server.rs`), and three milestone-scoped modules:
- **m1**: perception parameter parsing, observation assembly orchestration, OCR `read_text`, find/search across A11y elements + entities, capture-target + perception-mode toggles. Reads the host via `synapse-perception`/`synapse-a11y`/`synapse-capture`.
- **m2**: action tool param/response structs and `act_*_with_handle` orchestrators that build `synapse_core::Action`s and dispatch them through an `ActionHandle`. Holds a `RELEASE_ALL_HANDLE` for the operator hotkey.
- **m3**: event subscription bridge (`SseState`), reflex tool wrappers (kind→`ScheduledReflex`), profile tool wrappers, replay recorder, audio tail/transcribe. Owns `M3State` (db_path, profile_dir, permission grants, lazy reflex/profile/audio runtimes).

Two transports: **stdio** (`run_stdio` in `main.rs`) and **HTTP** (`http::serve`); HTTP requires loopback unless `--allow-non-loopback`, enforces `Bearer` auth with constant-time SHA-256 comparison (`http/auth.rs`), enforces `Mcp-Session-Id` headers on `/mcp` (`http/session.rs`), and exposes the event bus over Server-Sent Events at `/events` (`http/sse.rs`). See [06_mcp_service_and_transports.md](06_mcp_service_and_transports.md).

### 9.3 synapse-storage (RocksDB)
Opens RocksDB with LZ4 base, ZSTD on `CF_OBSERVATIONS`/`CF_SESSIONS`, no-compression on `CF_MODEL_CACHE`, fixed-prefix slice transforms on the time-keyed CFs (`CF_EVENTS`, `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`). Schema version sentinel key `__schema_version` (big-endian `u32`) is checked on open and `STORAGE_SCHEMA_MISMATCH` is returned on mismatch. Periodic background tasks: 5-minute GC (`gc.rs`) and 30-second disk-pressure polling (`pressure.rs`, thresholds 2 GB / 1 GB / 500 MB / 200 MB). Writes are aggregated through a background `Batcher` task (`batch.rs`). JSON-only payload codecs (`codecs.rs`) — binary codecs forbidden by ADR-0001 / RUSTSEC-2025-0141 footnote in source. See [04_storage_layer.md](04_storage_layer.md).

### 9.4 synapse-reflex (sub-frame runtime)
`ReflexRuntime` (`crates/synapse-reflex/src/lib.rs`) owns the scheduler, `EventBus`, and a `Db` handle for audit persistence. Reflex kinds: `AimTrack` (PID-style aim controller with EMA smoothing), `HoldMove` (held key set with optional re-assert), `HoldButton` (held mouse/pad button), `Combo` (timed step list), `OnEvent` (event-filtered action firing with recursion guard, default cap `MAX_ON_EVENT_FIRINGS_PER_TICK`). Scheduler runs at 1 ms tick (default), records `TickSample` with jitter, computes p99, exposes `degraded_latency()` and `recursion_clamps_total()` for `health`. Each register/cancel/disable/fire writes a `StoredReflexAudit` row into `CF_REFLEX_AUDIT`. Subscriber bus is bounded by `SUBSCRIBER_QUEUE_CAPACITY` with drop accounting. See [07_reflex_runtime.md](07_reflex_runtime.md).

### 9.5 synapse-action (input emission)
The `ActionEmitter` actor (mpsc channel, `ACTION_QUEUE_CAPACITY=256`) consumes `Action` messages, applies token-bucket rate limits per backend (`SOFTWARE_RATE_LIMIT_PER_S`, `VIGEM_RATE_LIMIT_PER_S`), tracks held keys/buttons/pad state in a `BitSet`, and dispatches to one of: Software (`enigo`), ViGEm (`vigem-client`), Recording (in-memory buffer, used by tests), `HardwareBackend` (when `--hardware-hid <port|auto>` connects and identifies a Synapse Pico), or `HardwareUnavailable` (fail-closed `ACTION_BACKEND_UNAVAILABLE` when hardware HID is not enabled). Owns the operator panic hotkey (`Ctrl+Alt+Shift+P`) which fires a 50 ms-budgeted `ReleaseAll` (`crates/synapse-mcp/src/safety.rs`). Curve sampling, keystroke-dynamics samplers (`BIGRAMS`, `KeystrokeNaturalParams::FAST`), click-timing caches, clipboard read/write/clear, and per-action validation (`validate_action`, `MAX_DRAG_DISTANCE_PX`) all live here. See [08_action_subsystem.md](08_action_subsystem.md).

### 9.6 synapse-perception (observation assembly)
Aggregates inputs from `synapse-a11y` (UIA tree) and `synapse-capture` (frame sources, OCR) into an `Observation`. Owns the `ObservationAssembler` with per-slot include flags (`ObserveInclude`), auto perception-mode resolution (`auto_mode`, `auto_mode_with_a11y`), and `ObservationInput` (synthetic fixture or live OS data). Provides OCR with WinRT and CRNN backends (`OcrProvider`, `read_text`, `read_text_with_provider`). See [09_perception_and_capture.md](09_perception_and_capture.md).

### 9.7 synapse-a11y (UIA + CDP)
Windows UI Automation tree walker (`uiautomation` crate), WinEvent hook for focus/mutation events (`subscribe_win_events`), Chromium DevTools Protocol attach via `chromiumoxide` for Chromium-family processes. Provides `current_foreground_context`, `focused_element`, `element_from_point`, `snapshot(root, depth)`, `find_by_name_and_pattern`, `re_resolve(ElementId)`, `expand_state_of`. Event utilities `coalesce_events` and `debounce_value_changes` deduplicate noisy UIA streams. See [09_perception_and_capture.md](09_perception_and_capture.md).

### 9.8 synapse-capture (zero-copy GPU capture)
DXGI duplication and Windows.Graphics.Capture backends with a single `CaptureBackendPreference::Auto` resolver. Exposes `D3D11Texture` references via `SendablePtr<T>`, `CapturedFrame { texture, width, height, format, captured_at, frame_seq, dirty_region }`, `screen_region_to_software_bitmap`, DPI-awareness initialization (`init_process_dpi_awareness`, `is_per_monitor_v2_dpi_aware`), and screen↔window coord helpers. Capture loop runs on a high-priority thread and pushes frames into a bounded `crossbeam` channel (`CAPTURE_CHANNEL_CAPACITY=2`). See [09_perception_and_capture.md](09_perception_and_capture.md).

### 9.9 synapse-audio (WASAPI + STT)
`AudioRuntime` (`crates/synapse-audio/src/lib.rs`) holds a ring buffer (default 5 s, max 5 s — `DEFAULT_RING_SECONDS = MAX_RING_SECONDS = 5`), a WASAPI loopback handle (started only when `--enable-audio` or `SYNAPSE_AUDIO_LOOPBACK=1`), optional event-emitting detectors (`detectors::DetectorProcessor`), and a Whisper-tiny STT engine (`WhisperTinyStt`). Provides `tail_seconds`, `transcribe_tail`, ring-format negotiation, direction estimate (azimuth + confidence). See [10_audio_and_models.md](10_audio_and_models.md).

### 9.10 synapse-profiles (TOML loader + live reload)
`ProfileRuntime::spawn` parses every `.toml` file in the profile directory at startup, registers a `notify` recursive watcher with a 200 ms debounce, and refreshes parsed profiles on filesystem events. Bundled profile dir is auto-discovered (`bundled_profiles_dir`); operator override is `SYNAPSE_PROFILE_DIR`. Resolves the active profile against the current foreground window via `resolve_active_profile`. Profiles with `use_scope=unknown` require explicit `--allow-unknown-profile`. See [11_profiles_hid_telemetry.md](11_profiles_hid_telemetry.md) (or section in subsystem deep-dives).

### 9.11 synapse-telemetry (logs + metrics registry)
Owns the process-wide `tracing` subscriber: JSON file appender (daily rolling, lives in `%LOCALAPPDATA%/synapse/logs`), stderr console layer, and a background log-GC thread (default 6-hour interval, 7-day retention, 500 MiB ceiling, all overridable by `SYNAPSE_LOG_GC_INTERVAL_S`). Installs the panic-to-log hook (`install_panic_hook`). The `metrics` module declares 19 M3 metric specs (12 counters, 5 gauges, 2 histograms) with bounded label cardinality (`CARDINALITY_LIMIT = 1000`). Examples: `events_published_total`, `reflex_fires_total`, `reflex_tick_jitter_us`, `storage_disk_pressure_level`, `storage_cf_bytes`, `audio_loopback_underruns_total`, `sse_active_subscribers`. Spawn registers these via `metrics-exporter-prometheus` machinery.

### 9.12 synapse-models (ONNX runtime wrappers)
Holds `ModelDescriptor` (id, path, sha256, input_shape, class_map), `Detector` trait (`infer(frame, opts)`), `DetectionFrame::validate`. Wraps `ort` v2.0.0-rc.12 with optional `cuda` and `directml` features (`directml` is required by `synapse-audio` for the Whisper-tiny inference). Validates SHA-256 before loading; emits `MODEL_HASH_MISMATCH` on drift.

### 9.13 synapse-hid-host
USB-serial driver for the RP2040 HID gateway firmware (`firmware/pico-hid/`, excluded from the root workspace via `Cargo.toml::exclude`). The crate exposes serial discovery, `HidGateway::connect`, IDENTIFY parsing/version validation, CRC16 frame encode/decode, pipelined send, reconnect state, firmware telemetry snapshots, and HID error mapping. `synapse-mcp --hardware-hid <port|auto>` uses this crate to build the live `HardwareBackend`.

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
| M5 — production polish + registry/audit moat | release gate blocked by M4; registry/audit work active | — | Installer, overlay, ≥10 profiles, VLM `describe`, soak, and the #454/#455-#470 profile-registry/audit-data learning loop |

## 11. What is NOT covered

- **Cross-platform support.** All capture, a11y, action, audio, hotkey, and HID paths are `#[cfg(windows)]`. Non-Windows builds compile (stubs return `ACTION_BACKEND_UNAVAILABLE`/equivalent), but no perception or action paths are wired.
- **Inner LLM / planner.** The agent (Claude / Codex / Cursor) is the brain; `synapse-mcp` is the body. There is no GOAP/MCTS/skill library inside this repo.
- **Goal storage.** RocksDB does not persist agent goals or plans; it only stores sensor/reflex/action traces.
- **Process manipulation / packet sniffing.** Out of scope per PRD §"Out of scope". `synapse-hid-host` and `synapse-action` only talk to OS input APIs or USB serial; no game RAM reads.
- **HTTPS / TLS.** The HTTP transport is loopback HTTP only by default; the Origin allow-list rejects non-`http://` Origins (`http/auth.rs::validate_origin`). For non-loopback binds with TLS termination, the operator is expected to front the daemon with a local reverse proxy.
- **Migration shims pre-v1.** Schema changes wipe-and-rebuild (`docs/computergames/README.md` §Authoring rules); there is no migration framework in `synapse-storage`.
