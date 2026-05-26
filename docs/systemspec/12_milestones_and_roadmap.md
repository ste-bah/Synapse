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
6. **Missing configured-host prerequisites are agent work, not blockers.** Do not stop at "missing." If the operator could download, install, connect, configure, generate, flash, launch, or inspect it from this computer, the agent must use Synapse/local host control to make it happen and then inspect the physical SoT. Ask only for narrow approval on hard-to-reverse external actions after reversible local work is exhausted.

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
