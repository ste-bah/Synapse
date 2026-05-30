# 07 — Cross-cutting concerns

Discipline applied across M0-M5, not owned by any single phase. Pointers to authoritative PRD sections; impplan adds enforcement rules. State-tracking authority: git tags + `CHANGELOG.md` > codebase on `main` > GitHub Issues. **All M0/M1/M2/M3 issues are closed** as of 2026-05-25 (`v0.1.0-m3` tag); M4+ opens new issues with `phase:mN` + `area:*` labels. ADRs landed through M3: 0001 (Rust + deps), 0002 (RocksDB primary), 0003 (reflex recursion guard), 0004 (reflex priority), 0005 (multi-monitor capture target), 0006 (profile match precedence), 0007 (per-event vs batched notifications).

**Operator directives D1–D7 override anything below on conflict.** Canonical: `AGENTS.md` §D1–D4, `00_methodology.md` §"Operator directives" (D5/D6), §12 below (D7). Most relevant here: D1 (manual FSV, no automated substitute), D3 (no GitHub Actions/CI gate; #246/#247/#350/#351), D5 (no pre-v1 back-compat), D6 (no mocks gate completion).

---

## 1. Perf gate workflow (per PR, every phase)

Per `10_performance_budget.md` + `13_testing_strategy.md` §7.

Tracked benches (local/manual perf-regression gate):

| Bench | Target p99 | First defined in |
|---|---|---|
| `observe_warm_a11y_only` | ≤ 10 ms | M1 |
| `observe_warm_hybrid` | ≤ 30 ms | M1 |
| `event_to_subscriber` | ≤ 50 ms | M3 |
| `reflex_tick_jitter_idle` | ≤ 200 µs | M3 |
| `reflex_tick_jitter_under_load` | ≤ 500 µs | M3 |
| `aim_curve_step_calc_natural` | ≤ 1 µs/step | M2 |
| `action_software_press` | ≤ 3 ms | M2 |
| `action_hardware_press` | ≤ 5 ms | M4 |
| `hid_combo_timing` | ≤ 0.5 ms step-interval deviation | M4 |
| `hid_high_volume` | 10k commands ≤ 15 s, zero drops/CRC errors | M4 |
| `detection_rtdetr_v2_s_coco_640` | ≤ 25 ms DirectML / ≤ 8 ms CUDA | M4 default per ADR-0010 |
| `ocr_winrt_120x32` | ≤ 8 ms | M1 |
| `serialize_observation_typical` | ≤ 5 ms | M1 |

Candidate delta > 20% on any tracked bench ⇒ release/merge blocked until either (a) fix, (b) ADR amending the target with measurable justification. The source of truth is exported `critcmp` JSON outside git: durable baselines under `%LOCALAPPDATA%\synapse\benchmarks\baselines\`, candidate runs under `.runs\benchmarks\`, compared with `scripts/check-bench-delta.ps1`. Per #350, do not use GitHub Actions/CI or committed `bench_results/<sha>/` directories for this gate.

Spike check active in production (`10 §15`): any subsystem > 2× p99 for > 5 s ⇒ `synapse-performance-degraded` event + `health.subsystems.X.status="degraded_latency"`.

---

## 2. Security review checklist (per PR touching action / capture / storage / mcp transport)

Per `11_security_and_safety.md`.

```
✓ Loopback-only default unchanged (or ADR for new bind)
✓ Bearer-token auth path unchanged for HTTP routes (or rotation tested)
✓ Origin/Host check intact
✓ No new path bypasses redaction (synapse-core::redact applied to all 8 surfaces in 11 §5.3)
✓ No new permission class without default-deny + explicit `--allow-X` gate
✓ Forbidden capabilities still compile-time disabled (11 §7)
✓ `unsafe` only in synapse-capture / synapse-hid-host / firmware/pico-hid
✓ `cargo deny check` clean for any new dep
✓ `cargo audit` clean
✓ Supported-use gates unchanged (08 §3 / §6) or extended w/ ADR
```

For PRs adding a new MCP tool: declare `required_permissions(params)` returning the `Permission` set; MCP layer checks before dispatch.

---

## 3. Observability gate (per PR)

Per `12_observability.md`.

```
✓ Every non-trivial fn has tracing::instrument or manual span!
✓ Metric labels bounded (no session_id / image_hash as label keys)
✓ Log level appropriate (error/warn/info/debug/trace)
✓ Subsystem status surfaceable via health tool
✓ New error code paths emit the code via tracing + structured fields
✓ Replay log (CF_EVENTS) captures the new event kind if user-visible
```

For PRs adding new event kinds: register in `06 §3.1` catalog + update `synapse-core::Event::kind` validators.

---

## 4. Test discipline (per PR)

Per `13_testing_strategy.md`.

| Layer | Required for | Notes |
|---|---|---|
| Unit | every pub fn w/ non-trivial logic | error variant must be triggered |
| Integration | every subsystem boundary | real OS where possible (capture, RocksDB, UIA on Win) |
| Property | filter eval, aim curves, keystroke, coord transforms, JSON round-trip | `proptest` |
| Snapshot | tool schemas, observation shape, error response shape | `insta` |
| Bench | tracked perf bench list (§1 above) | `criterion`, `critcmp`, local exported JSON delta gate |
| E2E | each milestone's demo scenario | real Notepad, real Minecraft, real RP2040 |
| Fuzz | protocol parsers (MCP JSON-RPC, HID serial, EventFilter, Profile TOML) | `cargo-fuzz` 10 min/target nightly |
| Soak | weekly per `13 §12` | 8 h synthetic workload |

**No mocks gate completion** (PRD authoring rule). Mocks acceptable for fast unit-level isolation; real-OS coverage required for integration + E2E.

---

## 5. Dep + license policy (per PR adding deps)

Per `14_build_and_packaging.md` §14 + `deny.toml`.

Allowed: `MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `MPL-2.0`, `ISC`, `Zlib`, `Unicode-3.0`, `BSL-1.0`, `CC0-1.0`.
Blocked: `GPL-*`, `AGPL-*`, `SSPL-*`, vendored deps w/o SPDX id.

Adding a dep requires:

1. Declare the current compatible version in `[workspace.dependencies]`
2. Justification in PR description (what it replaces / why no smaller alt)
3. License field in dep's manifest matches allowed list
4. Update `THIRD-PARTY-LICENSES.md` via `cargo about`

AGPL ML weights (Ultralytics YOLO trained checkpoints) **never bundled** per `OQ-025`. Operator downloads themselves.

---

## 6. Release process (per tag)

Per `14 §12`. Applies to every `vX.Y.Z` tag (and the M0-M4 archival `v0.1.0-mN` tags).

```
1. Branch release/x.y.z from main
2. Tag vx.y.z on the commit
3. Local release build/signing step produces:
   - synapse-mcp.exe (release profile)
   - synapse-overlay.exe
   - SynapseSetup-x.y.z.msi
   - synapse-portable-x.y.z-windows-x64.zip
   - synapse-pico-hid-x.y.z.uf2
4. Upload to GitHub Releases w/ release notes
5. (v1.0+) cargo publish for library crates
6. (v1.0+) winget manifest PR
```

Each tag requires:

- All acceptance gates of the corresponding phase green
- Manual test plan signed off (`13 §15`) for v1.0.0+
- CHANGELOG entry summarizing changes
- Schema-compat note if any storage shape changed

---

## 7. ADR workflow (when `16_open_questions.md` resolves)

Per `00_methodology.md` §10.

1. Check OQ list for matching entry
2. Create `docs/adr/NNN-<title>.md` with:
   - Context (what changed; what evidence forces a decision)
   - Decision
   - Consequences (PRD diffs, code impact, what new OQs open)
3. Update OQ entry to `## OQ-NNN — <summary> — DECIDED <YYYY-MM-DD>` + ADR link
4. Patch any PRD doc whose claim becomes stale (same PR)
5. PR title: `adr(NNN): <one-line>`

ADRs are append-only once merged. Revisions create new ADRs (`NNN-superseded-by-MMM`).

---

## 8. Documentation hygiene (per PR touching docs)

Per `compressionprompt.md` doctrine (universal):

| Rule | Mechanism |
|---|---|
| Numbers, paths, error codes verbatim | manual review + grep gate for known dupe forms |
| Markdown headings + tables + code fences as primary structure | review pattern |
| Cross-doc references by file path, not restated content | `scripts/check_docs.ps1` resolves all relative Markdown links |
| Defined terms once at top of each doc, used densely below | review pattern |
| One instruction per sentence in normative rules | review pattern (ASD-STE100 §4.12) |
| No emojis unless user-requested | local review / grep |

When PRD §X content moves: leave a one-line `→ see §Y` stub for the link target, don't silently delete.

---

## 9. Open question decision targets — phase mapping

Closes during the phase that hits the decision:

| Phase | Open questions to close |
|---|---|
| M1 | OQ-009 (max_elements default; M5 telem feedback expected); OQ-010 (CDP auto-attach); OQ-024 (token budget enforcement); OQ-023 (element_id stability) |
| M2 | OQ-004 (productivity aim curve default) — partial; final at M5 |
| M3 (closed) | OQ-001 (RocksDB primary per ADR-0002); OQ-005 (reflex priority per ADR-0004); OQ-012 (multi-monitor per ADR-0005); OQ-015 (profile match precedence per ADR-0006); OQ-022 (recursion guard per ADR-0003); OQ-029 (per-event notifications per ADR-0007) |
| M4 | OQ-003 (detection model default — decided in ADR-0010); OQ-013 (aim_track EMA smoothing — decided in ADR-0011); OQ-016 (hardware action coalescing — decided in ADR-0012) |
| M5 | OQ-008 (VLM bundling); OQ-014 (Whisper-tiny vs base); OQ-017 (disk pressure thresholds); OQ-019 (telemetry split); OQ-020 (`game_screenshot_once` exposure); OQ-030 (GC cadence final) |
| v1.x | OQ-006 (per-session permissions); OQ-007 (profile signing); OQ-021 (HRTF audio); OQ-027 (hardware HID 2FA); OQ-028 (migrations vs wipe); OQ-026 (cross-platform start trigger); OQ-018 (replay format final) |

OQs not landing in a phase ⇒ deferred forward with explicit note in `16_open_questions.md`.

---

## 10. Local check authority

Per `13_testing_strategy.md` §14. Repeated for forcing function:

| Check | Host | When |
|---|---|---|
| `cargo fmt --all --check` | configured host | before commit |
| `cargo clippy --workspace --all-targets -- -D warnings` | configured host | before commit when code changes |
| `cargo test --workspace` | configured host | before commit when feasible; focused gate allowed when issue-scoped |
| `cargo test --workspace --no-default-features` | configured host | before commit when feature gates change |
| `cargo build --release --workspace` | configured host | release candidate |
| `bash scripts/check_dep_graph.sh` | local shell with bash | dependency-edge changes |
| `cargo deny check` | configured host | dependency changes |
| `cargo audit` | configured host | dependency changes / release candidate |
| `insta review --check` | configured host | snapshot changes |
| `scripts/check_docs.ps1` | configured host PowerShell | doc changes |
| `e2e-real-windows` | configured Windows host | issue-specific manual evidence / release candidate |
| `bench-regression` | configured Windows host | manual/local exported `critcmp` delta gate |
| `hardware-in-loop` | configured host with Pico | hardware work-items / release candidate |
| `soak` | configured Windows host | release candidate |
| `fuzz` | configured host | parser/protocol changes |

Do not use GitHub Actions/CI as a merge or phase-tag gate unless a later operator decision explicitly reverses #351. Local checks are supporting evidence only; manual configured-host FSV with source-of-truth readback is the required evidence.

---

## 11. Coverage targets (per `13 §16`)

| Crate | Target | Tool |
|---|---|---|
| `synapse-core` | 95% | `tarpaulin` (Linux for pure crates) |
| `synapse-storage`, `synapse-profiles`, `synapse-reflex`, `synapse-action` | 85% | tarpaulin |
| `synapse-capture`, `synapse-a11y`, `synapse-audio`, `synapse-perception` | 70% | OS-bound; Windows tarpaulin where supported |
| `synapse-models`, `synapse-hid-host`, `synapse-telemetry` | 80% | tarpaulin |

> 5% drop on a PR blocks merge.

---

## 12. Natural-only motion invariant (OQ-004 DECIDED 2026-05-22)

> Smooth + natural + very fast. `Natural` curves and `Natural` keystroke dynamics are the default everywhere. No `Instant` jumps, no `Burst` typing as defaults — anywhere.

Authority: `03_action.md` §6 (`AimNaturalParams::FAST` preset) + §7 (`KeystrokeDynamics::Natural::FAST` preset). Resolution: `16_open_questions.md` OQ-004.

Per-PR enforcement:

```
✓ No new bundled profile sets mouse_curve_default ≠ "natural" without ADR
✓ No new bundled profile sets keyboard_dynamics_default ≠ "natural" without ADR
✓ No MCP tool schema default field selects "instant" or "burst" for curve/dynamics
✓ Default-resolution test in synapse-action covers every Action variant — asserts default curve = Natural::FAST, default dynamics = Natural::FAST
✓ Aim-style compilation: Snap→50ms Natural, Flick→35ms Natural, Natural→100-200ms Natural, Track→reflex per-tick (no curve flag accepts Instant as the resolved default)
```

`Instant` and `Burst` remain in their respective enums — usable via explicit caller opt-in (e.g., test harness pixel-asserts, paste of machine tokens). They are never the resolved default of any tool, profile, or reflex parameter.

Travel-time targets (default-resolution paths):

| Path | Default ms | Comes from |
|---|---|---|
| `act_click` cursor travel | 50 | `Natural::FAST` Snap |
| `act_aim style="snap"` | 50 | same |
| `act_aim style="flick"` | 35 | `Natural::FAST` Flick |
| `act_aim style="natural"` (explicit slower mode) | 100-200 | longer travel preset |
| `act_type` per char | ~32 ± 10 (bigram-biased ↓25%) | `Natural::FAST` |
| `act_press` single-key hold | 33 | unchanged (no curve involved) |
| Combo step intra-step | scheduled per `at_ms` | reflex scheduler; no curve |
| Reflex `aim_track` per-tick | 1 ms tick, ≤ 5 px/tick clamp | `Natural`-style sub-pixel tremor optional |

---

## 13. The single-line invariant

> The model is the brain. Synapse is the body. (`00_vision_and_scope.md` §12)

Every PR must preserve this. PRs that add planning, MCTS, GOAP, skill libraries, inner LLM, world model, or learning loops ⇒ rejected without ADR.

---

## 14. M2/M3 lessons — apply at M4+

| Lesson | Source | Apply how |
|---|---|---|
| 500 LoC cap erodes silently — M2 closed at 6 over-cap action files; M3 reintroduced overrun (synapse-a11y/lib.rs 2087, synapse-capture/lib.rs 1798, synapse-core/types.rs 1567, synapse-mcp/server.rs 1335, synapse-mcp/m3/reflex.rs 1165, synapse-reflex/lib.rs 986, synapse-reflex/scheduler.rs 890, synapse-mcp/http/sse.rs 764, synapse-mcp/m3/replay.rs 651, synapse-models/lib.rs 535) | M2 + M3 carry-over | Reviewers enforce at ≤ 450 LoC during code review; M4 Block A.0 splits the M3 over-cap files **before** building hardware HID on top. Repeat the M2 → M3 closure pattern. |
| Telemetry log GC at startup only -> long-lived daemon exceeds 500 MB cap | #241/#262 | Long-running cleanup tasks need an explicit cadence and manual evidence proving mid-uptime cleanup; telemetry GC uses the `synapse-telemetry-gc` worker with `SYNAPSE_LOG_GC_INTERVAL_S` |
| Ephemeral verification run dirs leak into the worktree | #242/#261/#351 | Do not create new FSV scripts, harnesses, or run dirs. If a supporting check writes ad-hoc artifacts, use `.runs/` (gitignored) and `scripts/clean-runs.ps1`; never write into the repo root. |
| `bench_results/<sha>/` was committed per commit (8 dirs removed by #260) | #243/#260 | Use local `critcmp` JSON outside the repo; stop committing per-commit baselines |
| Packaged-Notepad UIA `MenuBar` discovery silently empty under `ControlView` walker | #244 closed by `a414226` | RawView walker is now the default; future a11y work must include a UWP-packaged-app smoke test |
| Coords are physical (DPI-aware) pixels — undocumented; trips DPI-unaware SoT readers | #239 closed by `4eef83c` | Tool schema descriptions + `03_action.md` document coord space; future tools must document coord space explicitly |
| `SoftwareBackend::mouse_move` read cursor via Enigo (DPI-unaware) in a DPI-aware host | #234 closed by `eef654f` | Win32 `GetCursorPos` is now the cursor source of truth in DPI-aware mode; future cross-DPI tests must assert byte-equal end position |
| Backend wiring no-op (#228) went undetected until #219 live manual evidence | M2 carry-over | every backend integration test must dispatch through the real `ActionEmitter` actor with a real backend, not via direct backend `execute` calls; M3 added `eef654f fix: route recording mode through action actor` to enforce this for the recording backend |
| ViGEmBus 1.22.0 installer fails unattended (-536870911 no log) | #229 | M2/M3 explicitly scoped to **operator's configured host** with ViGEmBus pre-installed; do not gate M4+ on unattended driver install |
| `SYNAPSE_MCP_FORCE_PANIC_DURING_ACT` and `SYNAPSE_MCP_FORCE_*` env flags are deterministic trigger hooks for otherwise unreachable code paths | shipped through M1+M2 | M3 added parallel `SYNAPSE_REFLEX_FORCE_DEGRADED`, `SYNAPSE_STORAGE_PRESSURE_FREE_BYTES_SAMPLE`, `SYNAPSE_MAX_SUBSCRIPTIONS` env knobs (visible in `crates/synapse-mcp/src/main.rs`) to drive M3 error paths; M4 adds parallel `SYNAPSE_MCP_FORCE_HID_*` env flags for the hardware-HID error surface |
| M3 introduced its own carry-over: streamable HTTP SSE replay used to mis-track sequence cursors (`86a33ce fix(sse): use stream sequence cursors`); replay observation gaps tolerated under transient perception loss (`97019ec fix: tolerate transient replay perception gaps`) | M3 hotfixes after `f91f729 wire notepad save reflex demo` | Future SSE/replay work asserts the actual emitted sequence cursors against the per-sub ring buffer reader, not the internal counter; replay code paths must tolerate transient empty `Observation` reads without aborting the run |
