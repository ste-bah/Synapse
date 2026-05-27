# impplan — Synapse Implementation Plan

Operational map from PRD (`docs/computergames/`) → code. Each phase is a binary deliverable with a hard demo gate. Files in this directory are **prescriptive**; PRD is descriptive. Conflict ⇒ PRD wins, file is patched in the same PR.

Doctrine: `docs2/compressionprompt.md` §0-13. Keep verbatim: paths, crate names, error codes, thresholds, deps. Cut meta-framing, restatement, motivation prose — PRD already says it.

---

## Operator-level invariants (NEVER violate)

1. **No backwards compatibility (pre-v1).** Schema/API changes break callers. No fallbacks, no compatibility shims, no silent error swallowing. Anything that does not work must fail fast with a structured `synapse_core::error_codes::*` code and a tracing log line containing that code, so the failure is debuggable.
2. **No mocks gate completion.** Unit fakes are fine for isolation. An OS-bound work-item is **not done** until a real-OS integration test exercises it against the real source of truth (UIA `ValuePattern`, `XInputGetState`, file on disk, RocksDB key, `GetClipboardData`, `GetCursorPos`, low-level keyboard hook, etc.).
3. **Full-State Verification (FSV) is mandatory and manual.** The agent identifies the source of truth, reads `before`, executes the trigger, performs a separate read for `after`, exercises ≥3 edge cases (empty / boundary / structurally invalid), and records the actual state. Scripts, tests, benchmarks, harnesses, GitHub Actions, and CI are supporting evidence only; never call them FSV. See `00_methodology.md` §5.
4. **Natural-only motion (OQ-004 DECIDED 2026-05-22).** `Natural` curves + `Natural` keystroke dynamics tuned `FAST` (50 ms `Snap` travel, ~190 WPM typing with `mean_iki_ms=32, stddev=10, bigram_bias=true`) are the resolved default of every tool, profile, and reflex. No `Instant` jumps, no `Burst` typing as defaults. `Instant`/`Burst` remain in the enums for explicit caller opt-in only. See `07_cross_cutting.md` §12.
5. **Manual FSV on the configured Windows host is the shipping gate, not CI** (operator decision 2026-05-24, issues #246/#247/#350/#351). Use local checks for supporting evidence. Do not dispatch, wait on, or block a tag on GitHub Actions/CI. Do not add `*_fsv` tests, FSV harnesses, or FSV scripts.
6. **Missing configured-host prerequisites are work, not blockers.** Do not stop at "missing." Synapse gives the agent full local computer-control responsibility for this host. If the operator could download, install, connect, configure, generate, flash, launch, or inspect it from this computer, the agent must make it happen through Synapse/local host workflows and then inspect the physical SoT. Missing local state creates the next action for the agent, not a blocker while reversible host work remains. Nothing is ever `status:blocked` because a configured-host prerequisite is absent; the only blockable item is the exact operator-only hard-to-reverse external action left after every reversible local step is exhausted. Browser downloads, GUI installers, Device Manager checks, package-manager installs, model/file generation, firmware flashing, app launching, USB/COM inspection, and UI inspection are agent-owned work when reversible on this host. Ask only for narrow approval on hard-to-reverse external actions after every reversible local step is complete.

---

## Phase index

| # | File | Phase | PRD demo gate | Status (2026-05-25) |
|---|---|---|---|---|
| 00 | [`00_methodology.md`](00_methodology.md) | Dev discipline (all phases) | n/a | active |
| 01 | [`01_m0_bootstrap.md`](01_m0_bootstrap.md) | M0 — workspace + rmcp stdio + `health` | `15_roadmap_and_milestones.md` §2 | **DONE** — tag `v0.1.0-m0` @ `f04b429` (2026-05-23) |
| 02 | [`02_m1_perception_mvp.md`](02_m1_perception_mvp.md) | M1 — capture + UIA + `observe()` + 5 tools | §3 | **DONE** — tag `v0.1.0-m1` @ `b8ad120` (2026-05-23) |
| 03 | [`03_m2_action_mvp.md`](03_m2_action_mvp.md) | M2 — `synapse-action` + 9 tools + `ReleaseAll` | §4 | **DONE** — tag `v0.1.0-m2` @ `51836fe` (2026-05-24) |
| 04 | [`04_m3_reflex_mcp_surface.md`](04_m3_reflex_mcp_surface.md) | M3 — reflexes + RocksDB + profiles + HTTP/SSE + audio + 15 tools | §5 | **DONE** — tag `v0.1.0-m3` @ `97019ec` (2026-05-25) |
| 05 | [`05_m4_hardware_hid_first_game.md`](05_m4_hardware_hid_first_game.md) | M4 — RP2040 firmware + `synapse-hid-host` + Minecraft profile + `act_combo`/`act_run_shell`/`act_launch` | §6 | **ACTIVE** — start from this file |
| 06 | [`06_m5_production_polish.md`](06_m5_production_polish.md) | M5 — installer + overlay + 10+ profiles + VLM `describe` + soak | §7 | blocked by M4 |
| 07 | [`07_cross_cutting.md`](07_cross_cutting.md) | Perf gates, security, observability, release | §10/§11/§12/§14 | active |

Total estimate: ~5-7w solo remaining from M4 → v1.0.0 (M4 2-3w, M5 3-4w). Each phase is merge-blocked by the prior phase's demo gate.

---

## State-tracking — three sources, in order of authority

1. **Git tags and `CHANGELOG.md`** — the final record of what shipped. `v0.1.0-mN` means the demo gate passed and acceptance was signed. Current tags: `v0.1.0-m0`, `v0.1.0-m1`, `v0.1.0-m2`, `v0.1.0-m3`.
2. **The codebase on `main`** — second authority; the impplan is wrong if it disagrees with `main`. Patch the impplan in the same PR.
3. **GitHub Issues** (https://github.com/ChrisRoyse/Synapse/issues) — every PR-sized task, decision (`[DECISION]`), discovery (`[DISCOVERY]`), bug, risk, and context issue is filed here. M0/M1/M2/M3 historical issues are closed; new work opens issues with the `phase:m{N}` and `area:*` labels. **Use `gh issue list --state all` to walk closed M0-M3 issues for landed-decision context** — they record _why_ a path was chosen, often more than the commit message does.
4. **ADRs in `docs/adr/`** — append-only architectural decisions. Current set: ADR-0001 (Rust + deps), ADR-0002 (RocksDB primary), ADR-0003 (reflex recursion guard), ADR-0004 (reflex priority), ADR-0005 (multi-monitor capture target), ADR-0006 (profile match precedence), ADR-0007 (per-event vs batched notifications).

---

## How to use

1. Read PRD top-to-bottom once: `docs/computergames/README.md` → `00` → `01` → ... → `17`.
2. Open the impplan file for the current phase (M3 = `04_m3_reflex_mcp_surface.md` as of 2026-05-24).
3. Walk **Work-items** in order. Each is one merge-sized PR.
4. Block merge on **Acceptance gates** before opening the next phase.
5. **Open Questions** (`docs/computergames/16_open_questions.md`) hit during the phase → create `docs/adr/NNN-*.md` with decision rationale + PRD patch, or defer with note. Do not silently decide.

A work-item is "done" iff:

- `cargo build --release --workspace` compiles
- `cargo clippy --workspace --all-targets -- -D warnings` clean
- `cargo test --workspace` green locally on the configured host unless a focused issue-specific gate is documented
- The work-item's specific acceptance bullet passes
- Tracing instrumented; every error variant carries an `error_codes::*` code
- No `unwrap()` / `expect()` outside `#[cfg(test)]`, no `unsafe` outside the allowed crates (`synapse-capture`, `synapse-hid-host`, `firmware/pico-hid`)
- Manual FSV evidence in the issue: SoT name, before read, trigger, after read, happy path, ≥3 edge cases, and actual state values. Test stdout is supporting evidence only.

---

## Per-PR contract (every PR, every phase)

```
✓ Compiles release + dev
✓ Clippy zero warnings (workspace + all-targets)
✓ Tests pass (`cargo test --workspace`)
✓ Files ≤ 500 LoC; functions ≤ 30 LoC; cyclomatic ≤ 10  (M3 carry-over: several files in synapse-a11y/synapse-capture/synapse-core/synapse-mcp/synapse-reflex are over cap — see "M3 carry-over" table below; split before building M4 hardware paths on top, or land an ADR amending the rule per file)
✓ Error variants carry SCREAMING_SNAKE_CASE .code()
✓ Public APIs / CF names are `pub const`
✓ Tracing spans on every non-trivial fn
✓ No mocks gate completion (real captures, real RocksDB, real SendInput, real ViGEm, real WASAPI, real serial port at M4)
✓ Schema change ⇒ wipe-and-rebuild (pre-v1, no shim)
✓ Bench delta ≤ 20% on tracked metrics via local exported `critcmp` JSON (`docs/dev-host-hygiene.md` §Benchmark baselines)
✓ Docs cross-refs intact (`scripts/check_docs.ps1`)
✓ Manual issue evidence captures SoT before/readback-after state (see `00_methodology.md` §5)
```

---

## Workspace snapshot (2026-05-25, HEAD = `6ed52e4`, tag `v0.1.0-m3` @ `97019ec`)

| Crate | Path | State | Next phase owner |
|---|---|---|---|
| `synapse-mcp` | `crates/synapse-mcp` | **30 MCP tools live** (6 M1 + 9 M2 + 15 M3: `subscribe`/`subscribe_cancel`/`reflex_register`/`reflex_cancel`/`reflex_list`/`reflex_history`/`profile_list`/`profile_activate`/`replay_record`/`audio_tail`/`audio_transcribe`/`storage_inspect`/`storage_put_probe_rows`/`storage_gc_once`/`storage_pressure_sample`); streamable HTTP + SSE transport live on loopback with bearer auth + Origin/Host check + `Mcp-Session-Id` lifecycle; M3 permission gating wired; M3 subsystem health surfaced; M3 metrics registered | M4 adds `act_combo`, `act_run_shell`, `act_launch` |
| `synapse-core` | `crates/synapse-core` | full M0-M3 type set + all M3 error codes (`pub const`) including `REFLEX_RECURSION_LIMIT`, `HTTP_*` (bind/token/origin/session), `STORAGE_DISK_PRESSURE_LEVEL_1..4`, `REPLAY_*`, `SAFETY_PERMISSION_DENIED`, `SAFETY_PROFILE_ACTION_DENIED`, `REFLEX_ACTION_PERMISSION_DENIED`; `Action` enum + `AimCurve`/`AimNaturalParams::FAST` + `KeystrokeDynamics`/`KeystrokeNaturalParams::FAST`; `Profile`/`ReflexRegistration`/`StoredEvent`/`StoredObservation`/`StoredReflexAudit`/`StoredSession`/`OcrResult`/`EventFilter` extensions; event-filter validator | M4 extends with `Profile.use_scope`, `ComboInput` plumbing through MCP layer |
| `synapse-capture` | `crates/synapse-capture` | windows-capture 2.0 + DXGI fallback + DPI awareness + `screen_to_window`/`window_to_screen` + capture-target resolver | unchanged at M4 |
| `synapse-a11y` | `crates/synapse-a11y` | UIA tree walker + cache batch + WinEvent on COM STA + chromiumoxide CDP attach + `re_resolve` + `expand_state_of` + `coalesce_events`/`debounce_value_changes` + packaged-Notepad RawView fix (#244 closed) | M3 wired `subscribe_win_events` into the reflex bus; M4 unchanged |
| `synapse-perception` | `crates/synapse-perception` | `ObservationAssembler`, WinRT OCR | M4 adds HUD template-match extractor + `event_extensions` evaluator |
| `synapse-models` | `crates/synapse-models` | ORT 2.0-rc.12 session factory + sha256 verify | unchanged at M4 |
| `synapse-telemetry` | `crates/synapse-telemetry` | JSON file + console + periodic GC + panic-to-log hook + M3 metric registry hookups | unchanged at M4 |
| `synapse-test-utils` | `crates/synapse-test-utils` | `StdioMcpClient::launch_and_init_with_env(...)` + Notepad fixture (`launch_notepad`, `wait_for_window_title_regex`) + `notepad_process_ids` + M3 RocksDB scratch helpers + profile scratch + audio test asset loaders | M4 adds Pico/serial-port fixture |
| `synapse-action` | `crates/synapse-action` | **FULL** — emitter actor (split per A.0a refactor) + bounded mpsc(256) + held `BitSet` + token-bucket rate limit + curve/dynamics samplers + Software/ViGEm/Recording/Hardware/HardwareUnavailable backends + InvokePattern bridge + click_timing + clipboard (with open-contention retry) + safety panic hook + operator panic hotkey (`Ctrl+Alt+Shift+P`) + auto-release backend KeyUp (#231 closed) + DPI-aware `GetCursorPos` (#234 closed) + dynamics threaded through `text_dispatch` (#233 closed) + recording mode routed through the emitter actor | Hardware backend routes to `synapse-hid-host` only when `--hardware-hid <port|auto>` is configured and connected; otherwise it fails closed |
| `synapse-reflex` | `crates/synapse-reflex` | **FULL** — `EventBus` (drop-oldest 4096/sub, subscription cap configurable via `--max-subscriptions`/`SYNAPSE_MAX_SUBSCRIPTIONS`), 1 ms scheduler thread at `THREAD_PRIORITY_TIME_CRITICAL` + MMCSS Pro Audio on Windows with tokio fallback, `aim_track`/`hold_move`/`hold_button`/`combo`/`on_event` kinds, recursion guard (≤4/tick), conflict resolution (priority + newer-wins + starvation log), `CF_REFLEX_AUDIT` writes via `synapse-storage` | M4 reuses scheduler for `act_combo` (compiles to a `combo` reflex) |
| `synapse-storage` | `crates/synapse-storage` | **FULL** — RocksDB open w/ 11 CFs (per `07_storage_and_profiles.md` §4), `pub const CF_*` names, per-CF TTL compaction filter, 100 ms / 64 KB / explicit-flush write batcher, 5-min row-cap GC task, 4-level disk-pressure responder | unchanged at M4 |
| `synapse-profiles` | `crates/synapse-profiles` | **FULL** — TOML parser + `notify`-based watcher (debounced 200 ms) + match resolver (precedence per ADR-0006) + 4 bundled profiles (`notepad`, `vscode`, `chrome`, `terminal` — all Natural defaults) | M4 adds `minecraft.java` profile + `use_scope` field |
| `synapse-audio` | `crates/synapse-audio` | **FULL** — WASAPI loopback 5 s ring + detectors (loud transient / speech start-end / Silero VAD) + Whisper-tiny-int8 STT (lazy load + sha256) + GCC-PHAT stereo direction | unchanged at M4 |
| `synapse-hid-host` | `crates/synapse-hid-host` | **PARTIAL** — serial discovery, connect/IDENTIFY, CRC16 framing, pipeline/backpressure, reconnect state | M4 — remaining hardware-bound issues require real Pico/COM-device manual FSV |
| `synapse-overlay` | `crates/synapse-overlay` | binary skeleton (`src/main.rs` 3 LoC) | M5 |
| `firmware/pico-hid` | `firmware/` (excluded from workspace per root `Cargo.toml:21`) | standalone RP2040 firmware project | M4 — remaining firmware-bound issues require real Pico manual FSV |

Toolchain: stable Rust 1.95.0 (per ADR-0001), `edition = "2024"`, MSRV `1.95`. Cargo workspace at repo root; `default-members = ["crates/synapse-mcp", "crates/synapse-overlay"]`.

---

## M2 carry-over (closed in M3)

All M2 carry-over items from the prior revision are resolved on `main`. The original table is preserved here for archival; each row links to the M3 commit/PR that closed it.

| # | Issue | What | Closed in M3 by |
|---|---|---|---|
| #244 | UIA TreeWalker hides Win11 packaged Notepad MenuBar | `a414226 fix(a11y): expose packaged Notepad menu in snapshot` — RawView walker |
| #239 | DPI-aware physical-pixel coordinates undocumented | `4eef83c docs(action): document physical pixel tool coordinates` + tool schema descriptions |
| #234 | `SoftwareBackend::mouse_move` DPI mismatch | `eef654f`/mouse-coordinates split — Win32 `GetCursorPos` in DPI-aware mode |
| #233 | `software::type_text` ignored `dynamics` | dynamics re-threaded through `text_dispatch.rs` in the `synapse-action` split refactor |
| #231 | held-key auto-release never dispatched backend KeyUp | `87a051d test(action): add auto-release keyboard hook FSV` (legacy test-name artifact; supporting regression evidence only) plus the emitter split that calls the same backend dispatch as normal KeyUp |
| #243/#260 | `bench_results/<sha>/` committed bloat | `4b6eb80 chore(bench): move baselines off-tree` — local `critcmp` exports under `%LOCALAPPDATA%\synapse\benchmarks\baselines\` / `.runs\benchmarks\` |
| #242/#261 | Ephemeral verification run dirs in worktree | `81bb8ab chore(test): add run artifact cleanup` — `.runs/` standardized via `.gitignore` + `scripts/clean-runs.ps1` |
| #241/#262 | Telemetry log GC at startup only | `c430ccc test(telemetry): cover periodic log size GC` — periodic GC worker + size-cap manual evidence |
| **M2 LoC overrun** | `emitter.rs` 1474, `vigem.rs` 1131, `invoke.rs` 653, `software.rs` 586, `m2/click.rs` 506, `m2/press.rs` 545 | Split refactors `e5cde51`/`54acbf1`/`0fdf800`/`678b1c8`/`a7a8aff`/`d9c21a5`. All six files now ≤ 112 LoC. |

---

## M3 carry-over (must address before or during M4)

These are landed-but-imperfect surfaces from M3. M4 either consumes them as-is or fixes them in a one-shot PR before building hardware HID on top.

| # | What | Notes for M4 |
|---|---|---|
| **M3 LoC overrun** | The 500 LoC hard cap regressed in several places during M3. Current offenders (verified `wc -l` on `main`): `synapse-a11y/src/lib.rs` 2087, `synapse-capture/src/lib.rs` 1798, `synapse-core/src/types.rs` 1567, `synapse-mcp/src/server.rs` 1335, `synapse-mcp/src/m3/reflex.rs` 1165, `synapse-reflex/src/lib.rs` 986, `synapse-reflex/src/scheduler.rs` 890, `synapse-mcp/src/http/sse.rs` 764, `synapse-mcp/src/m3/replay.rs` 651, `synapse-models/src/lib.rs` 535. Test files (`crates/synapse-core/tests/stored_types.rs` 1012, `crates/synapse-core/tests/profile_types.rs` 740, etc.) also over cap. M4 first PRs split these as a no-behavior-change refactor or land per-file ADRs amending the rule with measurable justification — repeat the M2 → M3 pattern from Block A.0. |
| CHANGELOG M3 entry tool names | The `v0.1.0-m3` CHANGELOG entry lists `profile_get`/`profile_set_active`; the shipped tool names on `main` are `profile_list`/`profile_activate` (verified in `crates/synapse-mcp/src/server.rs`). Also missing from the M3 entry: the four `storage_*` diagnostic tools (`storage_inspect`, `storage_put_probe_rows`, `storage_gc_once`, `storage_pressure_sample`). Fix as part of the first M4 docs sweep. |

---

## Cross-references

| Concern | Authority |
|---|---|
| Crate boundaries, threading, channels | `docs/computergames/01_architecture.md` |
| Tool schemas, error response shape, transports | `docs/computergames/05_mcp_tool_surface.md`, `06_data_schemas.md` §8 |
| Storage CFs, TTLs, GC layers, profile TOML | `docs/computergames/07_storage_and_profiles.md` |
| Supported-use policy + permission gates | `docs/computergames/08_*.md` |
| Latency budgets per stage / per tool | `docs/computergames/10_performance_budget.md` §2/§12 |
| Permissions, redaction, kill switches | `docs/computergames/11_security_and_safety.md` |
| Tracing, metrics, OTLP, dashboards | `docs/computergames/12_observability.md` |
| Test pyramid, fakes, fuzz, soak | `docs/computergames/13_testing_strategy.md` |
| Workspace deps + profiles + features | `docs/computergames/14_build_and_packaging.md` |
| Risks per phase | `docs/computergames/15_roadmap_and_milestones.md` §9 |
| Open decisions | `docs/computergames/16_open_questions.md` |
| ADRs | `docs/adr/NNN-*.md` (0001 = current Rust + deps; new ones land in `04`/`05`/`06`) |

---

## Out of scope for impplan

- ADR contents (lives in `docs/adr/NNN-*.md`, created when an OQ resolves)
- Issue tracker / sprint board (GitHub Issues is authoritative)
- User-facing guide (`USER_GUIDE.md`, lands at M5)
- Release notes (per-tag `CHANGELOG.md`, not per-plan)
