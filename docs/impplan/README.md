# impplan — Synapse Implementation Plan

Operational map from PRD (`docs/computergames/`) → code. Each phase is a binary deliverable with a hard demo gate. Files in this directory are **prescriptive**; PRD is descriptive. Conflict ⇒ PRD wins, file is patched in the same PR.

Doctrine: `docs2/compressionprompt.md` §0-13. Keep verbatim: paths, crate names, error codes, thresholds, deps. Cut meta-framing, restatement, motivation prose — PRD already says it.

---

## Operator-level invariants (NEVER violate)

1. **No backwards compatibility (pre-v1).** Schema/API changes break callers. No fallbacks, no compatibility shims, no silent error swallowing. Anything that does not work must fail fast with a structured `synapse_core::error_codes::*` code and a tracing log line containing that code, so the failure is debuggable.
2. **No mocks gate completion.** Unit fakes are fine for isolation. An OS-bound work-item is **not done** until a real-OS integration test exercises it against the real source of truth (UIA `ValuePattern`, `XInputGetState`, file on disk, RocksDB key, `GetClipboardData`, `GetCursorPos`, low-level keyboard hook, etc.).
3. **Full-State Verification (FSV) is mandatory** for every test from M2 onward. Identify the source of truth, print `before`, execute, print `after` from a separate read, exercise ≥3 edge cases (empty / boundary / structurally invalid), and emit at least one `final_value=` log line. See `00_methodology.md` §5.
4. **Natural-only motion (OQ-004 DECIDED 2026-05-22).** `Natural` curves + `Natural` keystroke dynamics tuned `FAST` (50 ms `Snap` travel, ~190 WPM typing with `mean_iki_ms=32, stddev=10, bigram_bias=true`) are the resolved default of every tool, profile, and reflex. No `Instant` jumps, no `Burst` typing as defaults. `Instant`/`Burst` remain in the enums for explicit caller opt-in only. See `07_cross_cutting.md` §12.
5. **Manual FSV on the configured Windows host is the shipping gate, not CI** (operator decision 2026-05-24, issues #246/#247/#350). Use local checks for supporting evidence. Do not dispatch, wait on, or block a tag on GitHub Actions/CI.

---

## Phase index

| # | File | Phase | PRD demo gate | Status (2026-05-24) |
|---|---|---|---|---|
| 00 | [`00_methodology.md`](00_methodology.md) | Dev discipline (all phases) | n/a | active |
| 01 | [`01_m0_bootstrap.md`](01_m0_bootstrap.md) | M0 — workspace + rmcp stdio + `health` | `15_roadmap_and_milestones.md` §2 | **DONE** — tag `v0.1.0-m0` @ `f04b429` (2026-05-23) |
| 02 | [`02_m1_perception_mvp.md`](02_m1_perception_mvp.md) | M1 — capture + UIA + `observe()` + 5 tools | §3 | **DONE** — tag `v0.1.0-m1` @ `b8ad120` (2026-05-23) |
| 03 | [`03_m2_action_mvp.md`](03_m2_action_mvp.md) | M2 — `synapse-action` + 9 tools + `ReleaseAll` | §4 | **DONE** — tag `v0.1.0-m2` @ `51836fe` (2026-05-24) |
| 04 | [`04_m3_reflex_mcp_surface.md`](04_m3_reflex_mcp_surface.md) | M3 — reflexes + RocksDB + profiles + HTTP/SSE + audio | §5 | **ACTIVE** — start from this file |
| 05 | [`05_m4_hardware_hid_first_game.md`](05_m4_hardware_hid_first_game.md) | M4 — RP2040 firmware + Minecraft profile | §6 | blocked by M3 |
| 06 | [`06_m5_production_polish.md`](06_m5_production_polish.md) | M5 — installer + overlay + 10+ profiles + soak | §7 | blocked by M4 |
| 07 | [`07_cross_cutting.md`](07_cross_cutting.md) | Perf gates, security, observability, release | §10/§11/§12/§14 | active |

Total estimate: ~10w solo remaining from M3 → v1.0.0 (M3 2-3w, M4 2-3w, M5 3-4w). Each phase is merge-blocked by the prior phase's demo gate.

---

## State-tracking — three sources, in order of authority

1. **Git tags and `CHANGELOG.md`** — the final record of what shipped. `v0.1.0-mN` means the demo gate passed and acceptance was signed.
2. **The codebase on `main`** — second authority; the impplan is wrong if it disagrees with `main`. Patch the impplan in the same PR.
3. **GitHub Issues** (https://github.com/ChrisRoyse/Synapse/issues) — every PR-sized task, decision (`[DECISION]`), discovery (`[DISCOVERY]`), bug, risk, and context issue is filed here. As of 2026-05-24 all 244 historical issues are closed; new work opens issues with the `phase:m{N}` and `area:*` labels. **Use `gh issue list --state all` to walk closed M0/M1/M2 issues for landed-decision context** — they record _why_ a path was chosen, often more than the commit message does.

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
- FSV evidence in test stdout: at least one `source_of_truth=<name> ... after_truth=<...>` line and one `final_value=<...>` line per scenario

---

## Per-PR contract (every PR, every phase)

```
✓ Compiles release + dev
✓ Clippy zero warnings (workspace + all-targets)
✓ Tests pass (`cargo test --workspace`)
✓ Files ≤ 500 LoC; functions ≤ 30 LoC; cyclomatic ≤ 10  (M2 carry-over: emitter.rs/vigem.rs/invoke.rs are over cap; split or ADR before M3 builds on top)
✓ Error variants carry SCREAMING_SNAKE_CASE .code()
✓ Public APIs / CF names are `pub const`
✓ Tracing spans on every non-trivial fn
✓ No mocks gate completion (real captures, real RocksDB, real SendInput, real ViGEm)
✓ Schema change ⇒ wipe-and-rebuild (pre-v1, no shim)
✓ Bench delta ≤ 20% on tracked metrics via local exported `critcmp` JSON (`docs/dev-host-hygiene.md` §Benchmark baselines)
✓ Docs cross-refs intact (`scripts/check_docs.ps1`)
✓ FSV evidence in test stdout (see `00_methodology.md` §5)
```

---

## Workspace snapshot (2026-05-24, HEAD = `51836fe`)

| Crate | Path | State | Next phase owner |
|---|---|---|---|
| `synapse-mcp` | `crates/synapse-mcp` | **15 MCP tools live** (6 M1 + 9 M2) over stdio; `--mode http` returns `NOT_YET_IMPLEMENTED` exit 2; tools listed in `tests/snapshots/m2_tools_fsv__m2_tools_list.snap` | M3 adds `subscribe`, `reflex_*`, `profile_*`, `replay_record`, `audio_*` + HTTP transport |
| `synapse-core` | `crates/synapse-core` | full M0-M2 type set + 80 error codes (all `pub const`); `Action` enum + `AimCurve`/`AimNaturalParams::FAST` + `KeystrokeDynamics`/`KeystrokeNaturalParams::FAST` shipped; `ComboStep`/`ComboInput` carried for M3 | extend with `Profile`, `ReflexRegistration`, `Event`, `EventFilter` extensions, `StoredEvent`, etc. |
| `synapse-capture` | `crates/synapse-capture` | windows-capture 2.0 + DXGI fallback + DPI awareness + `screen_to_window`/`window_to_screen` + capture-target resolver | unchanged at M3 |
| `synapse-a11y` | `crates/synapse-a11y` | UIA tree walker + cache batch + WinEvent on COM STA + chromiumoxide CDP attach + `re_resolve` + `expand_state_of` + `coalesce_events`/`debounce_value_changes` | M3 consumes for `subscribe` event source |
| `synapse-perception` | `crates/synapse-perception` | `ObservationAssembler`, WinRT OCR | M3 adds HUD template-match extractor scaffold |
| `synapse-models` | `crates/synapse-models` | ORT 2.0-rc.12 session factory + sha256 verify (488 LoC) | unchanged at M3 |
| `synapse-telemetry` | `crates/synapse-telemetry` | JSON file + console + periodic GC + panic-to-log hook | unchanged at M3 |
| `synapse-test-utils` | `crates/synapse-test-utils` | `StdioMcpClient::launch_and_init_with_env(...)` + Notepad fixture (`launch_notepad`, `wait_for_window_title_regex`) + `notepad_process_ids` | M3 adds RocksDB fixtures, profile-dir scratch, audio test asset loader |
| `synapse-action` | `crates/synapse-action` | **FULL** — emitter actor + bounded mpsc(256) + held `BitSet` + token-bucket rate limit + curve/dynamics samplers + Software/ViGEm/Recording/HardwareUnavailable backends + InvokePattern bridge + click_timing + clipboard + safety panic hook | M3 adds reflex-driven enqueue path; held-state interlock with reflex `hold_*` |
| `synapse-reflex` | `crates/synapse-reflex` | **empty stub** (1 LoC) | M3 — main build target |
| `synapse-storage` | `crates/synapse-storage` | **empty stub** (8 LoC; `pub trait Db {}` declared) | M3 — RocksDB open + 11 CFs |
| `synapse-profiles` | `crates/synapse-profiles` | **empty stub** (1 LoC) | M3 — TOML loader + 4 bundled profiles |
| `synapse-audio` | `crates/synapse-audio` | **empty stub** (1 LoC) | M3 — WASAPI loopback + Whisper-tiny |
| `synapse-hid-host` | `crates/synapse-hid-host` | **empty stub** (1 LoC) | M4 — serial driver |
| `synapse-overlay` | `crates/synapse-overlay` | binary skeleton (`src/main.rs` 3 LoC) | M5 |
| `firmware/pico-hid` | `firmware/` (excluded from workspace per root `Cargo.toml:21`) | not yet created | M4 |

Toolchain: stable Rust 1.95.0 (per ADR-0001), `edition = "2024"`, MSRV `1.95`. Cargo workspace at repo root; `default-members = ["crates/synapse-mcp", "crates/synapse-overlay"]`.

---

## M2 carry-over (must address before or during M3)

These are landed-but-imperfect surfaces from M2. M3 either consumes them as-is or fixes them in a one-shot PR before adding the M3 work on top.

| # | Issue | What | Action for M3 |
|---|---|---|---|
| #244 | UIA TreeWalker hides Win11 packaged Notepad MenuBar | `synapse_a11y::snapshot()` uses `ControlView` which omits `MenuBar` children on Win11 22H2+ packaged Notepad | Patch TreeWalker to `RawView` for menu-bar resolution; M3 `subscribe` events depend on this |
| #239 | `act_aim`/`act_click`/`act_drag`/`act_scroll` coords are physical (DPI-aware) pixels — undocumented | Trips DPI-unaware source-of-truth readers | Document in tool schema description and `03_action.md`; no code change |
| #234 | `SoftwareBackend::mouse_move` uses Enigo `location()` under different DPI than callers | Cursor lands off by DPI scale factor | Patch to read cursor via direct Win32 `GetCursorPos` in DPI-aware mode |
| #233 | `software::type_text` ignores `dynamics`, batches into single `SendInput` | Notepad drops chars past queue depth | Re-thread `dynamics` through `text_dispatch.rs` (partial fix landed in `ea70964`) |
| #231 | held-key auto-release clears actor state but never dispatches backend `KeyUp` | Stuck-key telemetry fires; physical key never released | Auto-release path must call the same backend dispatch as a normal `KeyUp` |
| #243/#260 | bench_results dir bloat (8 per-commit subdirs committed) | Repo grows | Use local `critcmp` exports under `%LOCALAPPDATA%\synapse\benchmarks\baselines\` / `.runs\benchmarks\`; stop committing per-commit baselines |
| #242/#261 | `fsv-*/` ephemeral run dirs leak into the worktree | Untracked clutter | Standardize on `.runs/` + `.gitignore` + `scripts/clean-runs.ps1` |
| #241 | Telemetry log GC runs only at startup | Long-lived daemon exceeds 500 MB cap | Move to `tokio::interval`; already partially landed in `615cd4f` |
| **M2 LoC overrun** | `emitter.rs` 1474, `vigem.rs` 1131, `invoke.rs` 653, `software.rs` 586, `m2/click.rs` 506, `m2/press.rs` 545 — over the 500 LoC hard cap | Split before M3 builds reflex enqueue path on top, or land an ADR amending the rule with measurable justification | First M3 PR: file-split refactor with no behavior change |

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
