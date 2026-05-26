# 00 — Methodology

Discipline applied across M0-M5. PRD authority: `docs/computergames/README.md` §"Authoring rules" + `14_build_and_packaging.md` + `13_testing_strategy.md`. State-tracking authority: git tags + `CHANGELOG.md` (final) > codebase on `main` (operational) > GitHub Issues (https://github.com/ChrisRoyse/Synapse/issues — every PR-sized task, `[DECISION]`, `[DISCOVERY]`, `[BUG]`, `[RISK]`, `[CONTEXT]`). Use `gh issue list --state all --search 'phase:m4'` (etc.) to walk landed-decision history for context the commit message did not carry. **All M0/M1/M2/M3 historical issues are closed as of 2026-05-25** (`v0.1.0-m3` tag); new M4 work opens fresh issues with the same labels.

**Four load-bearing operator directives (NEVER violate):**

1. **No backwards compatibility (pre-v1).** Schema/API changes break callers. No fallbacks, no compatibility shims, no silent error swallowing. Fail fast with a structured `synapse_core::error_codes::*` code and a `tracing` line carrying that code so the failure is debuggable.
2. **No mocks gate completion.** Unit fakes are fine for isolation. An OS-bound work-item is **not done** until a real-OS integration test exercises it and a separate source-of-truth read confirms the side effect landed.
3. **Manual configured-host FSV is the shipping gate, not GitHub Actions** (issues #246/#247/#350/#351, operator decision 2026-05-24). Use local checks for supporting evidence. Do not dispatch, wait on, or block a tag on GitHub Actions/CI. FSV must never be delegated to scripts, automated tests, benchmarks, harnesses, CI jobs, or any other automated substitute.
4. **Missing prerequisites are acquisition work, not a stopping point.** If a local tool, driver, model, device, file, service, account state, installer, hardware surface, or other prerequisite is absent, do not mark the issue blocked for that reason alone. Missing means: figure out where the thing must come from, where it must physically appear, and make it happen on this configured host. Synapse gives the agent local computer control; treat Synapse/local control as the operator-equivalent host control surface. If the operator could download, install, connect, configure, generate, flash, launch, or inspect it from this host, the agent must attempt those reversible local steps using Synapse plus normal OS, shell, browser, package-manager, and device-management workflows. Do not ask the operator to download or install something while reversible local acquisition/setup remains possible. Operationally: do not stop at "missing"; if it can be done from this computer, do it and then inspect the resulting SoT. Browser downloads, GUI installers, Device Manager checks, package-manager installs, model/file generation, firmware flashing, app launching, and UI inspection are agent-owned work on this host. Missing configured-host state is never a blocker by itself. Identify the authoritative SoT where it should appear, perform the setup/acquisition step, then read that SoT directly. Ask only for narrow approval before hard-to-reverse external actions such as spending money, using private credentials, changing billing, modifying an external account, or making an irreversible shared-state change, and complete every reversible local step before asking.

---

## 1. Hard code rules (locally enforced/reviewed)

| Rule | Mechanism |
|---|---|
| `#![forbid(unsafe_code)]` workspace-wide | per-crate override only for `synapse-action` (Win32 SendInput batching), `synapse-capture` (DX FFI), `synapse-hid-host` (serial OS handle), `firmware/pico-hid` |
| File ≤ 500 LoC, function ≤ 30 LoC, cyclomatic ≤ 10 | clippy + local/reviewer check. M2 carry-over (six action/MCP files over cap) is **closed in M3** by the Block A.0 split refactors — all six are now ≤ 112 LoC on `main`. M3 introduced its own LoC overrun (see `README.md` "M3 carry-over"); the largest offenders are `synapse-a11y/src/lib.rs` 2087, `synapse-capture/src/lib.rs` 1798, `synapse-core/src/types.rs` 1567, `synapse-mcp/src/server.rs` 1335, `synapse-mcp/src/m3/reflex.rs` 1165, `synapse-reflex/src/lib.rs` 986, `synapse-reflex/src/scheduler.rs` 890. M4's first PR splits these before hardware HID is built on top. Reviewers must enforce at ≤ 450 LoC during code review to leave 50 LoC of margin. |
| `unwrap()` / `expect()` forbidden outside `#[cfg(test)]` | `#[deny(clippy::unwrap_used, clippy::expect_used)]` |
| `anyhow` forbidden in library crates | manual review + workspace dep gating |
| No `println!` / `eprintln!` | clippy lint + grep gate |
| Public API constants are `pub const`, not magic strings | test asserts every CF name / error code matches its literal |
| Error variants ⇒ `SCREAMING_SNAKE_CASE` code via `thiserror` impl with `.code()` | snapshot test (`13_testing_strategy.md` §3) |
| Schema change pre-v1 ⇒ wipe DB, no migration shim | doc gate; local sample wipe-and-rebuild |
| Files referenced in docs exist | `scripts/check_docs.ps1 -CheckAnchors` |

---

## 2. Crate layout invariants

`synapse-core` ← zero internal deps (type/error/const root). Verified each PR via `cargo tree -p synapse-core --depth 1` showing only crates.io deps.

Acyclic graph: `01_architecture.md` §5. Local dependency-graph checks fail if any new edge introduces a cycle.

Per-crate `Cargo.toml` scaffold from `14_build_and_packaging.md` §17. New crate via `scripts/new-crate.ps1 -Name synapse-<name>`.

---

## 3. Concurrency invariants

| Invariant | Where |
|---|---|
| Action emission serialized per device through one mpsc actor | `synapse-action` |
| Capture, reflex on dedicated OS threads at `THREAD_PRIORITY_TIME_CRITICAL` — never tokio pool | `synapse-capture`, `synapse-reflex` |
| UIA event handlers on COM apartment thread, marshal across channel | `synapse-a11y` |
| Audio loopback thread at MMCSS "Pro Audio" | `synapse-audio` |
| Perception result is single-producer multi-consumer via `tokio::sync::watch` | `synapse-perception` |
| Reflex + MCP tool handlers are the only writers to action channel | `synapse-action` |

Bounded channels everywhere. Drop policy documented per channel (`10_performance_budget.md` §10).

---

## 4. Error handling discipline

Three classes (`01_architecture.md` §9):

| Class | Where | Strategy |
|---|---|---|
| Recoverable (transient OS) | capture/a11y/audio/action | log warn → retry w/ backoff → structured error if persistent |
| User-facing | MCP tool handlers | JSON-RPC error: `code: -32099`, `data.code: SCREAMING_SNAKE_CASE` |
| Fatal | storage corruption, unsafe FFI | `panic!` → panic hook fires `ReleaseAll` → process exits |

`#[derive(thiserror::Error)]` per crate. `.code() -> &'static str` on every variant. Codes stable post-v1; pre-v1 free to rename.

Error code catalog: `06_data_schemas.md` §8. Exported as `pub const` in `synapse-core::error_codes`. Test asserts constants = literal strings.

---

## 5. Test discipline

Test pyramid: `13_testing_strategy.md` §2.

| Per PR | Frequency |
|---|---|
| `cargo fmt --check` | every PR |
| `cargo clippy --workspace --all-targets -- -D warnings` | every PR |
| `cargo test --workspace` | every PR |
| `cargo deny check` | every PR |
| `cargo audit` | every PR + daily cron |
| `insta review --check` | every PR |
| `cargo bench` perf-regression (tracked benches only) | local configured-host run + exported `critcmp` JSON delta ≤ 20% |
| E2E real Windows | nightly self-hosted |
| Hardware-in-loop (RP2040) | weekly self-hosted |
| Fuzz (`cargo-fuzz`) | nightly, 10 min/target |
| Soak (8h) | weekly |

**No mocks gate completion.** Integration tests use real `windows-capture` frames (or `MockCaptureSource` for unit), real RocksDB (`tempfile::TempDir`), real `SendInput` (or `RecordingBackend` for unit). Unit fakes never substitute for integration coverage of the OS-bound path.

**Full-state verification (FSV) — mandatory for every M2+ work item and performed manually by the agent:**

1. **Identify the source of truth** for the work the test is meant to perform (UIA `ValuePattern.value` for a typed string, file bytes on disk for a saved file, `RecordingBackend::events()` for the emitted `INPUT` sequence, `vigem-client` device state for a pad report, the daemon's tracing log line for an audit event, the `held_keys` BitSet for stuck-key state, RocksDB `Db::scan(CF_REFLEX_AUDIT, ..)` for a reflex fire, `GetClipboardData(CF_UNICODETEXT)` for clipboard write, `XInputGetState(0)` for ViGEm pad state, etc.).
2. **Record `before` state**, execute the action, **record `after` state** from a separate read. Issue evidence must name the SoT, edge case, before state, and after state in plain text.
3. **Read back from the source of truth** with a separate operation, distinct from the action under test. Never trust the return value of the operation under test as evidence that the side effect landed.
4. **Three edge cases minimum** per primary path:
   - **Empty / zero input** (`act_type({text:""})`, `audio_tail({seconds:0})`, empty event filter, empty profile dir)
   - **Boundary value** (`act_press({hold_ms:30000})` succeeds vs `30001` rejected; reflex cap 32 vs 33; 4096-event SSE ring boundary)
   - **Structurally invalid** (`schemars` reject → `TOOL_PARAMS_INVALID`; unknown enum variant; out-of-range float)
   Each edge case prints its own `before` / `after` and asserts on the source of truth.
5. **Evidence of success in the issue log.** The resolved comment must include the actual after-read state and final observed result for each scenario. Automated stdout may be attached as supporting evidence, but it is never FSV.
6. **Trigger → outcome reasoning** in the issue evidence: identify the trigger event, the process X inside the daemon, the observable outcome Y, and the source of truth that proves Y. See M2 §8.5 for the canonical example.
7. **Synthetic inputs with known expected outputs.** Pick fixtures whose expected source-of-truth state is unambiguous (`"Hello world."` → exact byte sequence in `Notepad ValuePattern.value`; a 5 s WAV with known transcript → Whisper response within Levenshtein ≤ 10%; 33 reflex registrations → cap rejection on the 33rd). Tests assert byte-equal or count-equal, not "looks ok."
8. **Process-restart durability check** for any test touching persistent storage (RocksDB CFs from M3 onward): write data, drop the daemon, spawn a fresh `StdioMcpClient`, read back, assert data survived.

A change that asserts only on return values is **not done**; review fails the PR. A resolution that omits manual final observed result values in the issue evidence fails review even if automated checks passed.

Missing prerequisite handling is part of this same gate. If a driver, model,
tool, firmware device, config file, or service is required for the configured
host, the agent must create an explicit setup action and verify the real
source of truth where that prerequisite should appear. Missing hardware,
drivers, tools, services, models, and files are work to acquire or configure,
not terminal blockers by themselves. Examples: `rustup target list --installed`
for an embedded target, `Get-PnpDevice` / `HKLM` for an attached hardware
device, `%APPDATA%\synapse\config.toml` for setup output, or the model file path
plus hash for a downloaded model. Do not replace this with absent-dependency
portability testing, scripts, or CI; acquire or configure the real thing and
then inspect it.
Do not stop at "missing." If the operator could make the prerequisite real from
this computer, the agent must do the reversible local work through Synapse and
host workflows, then read the physical SoT.

---

## 6. Performance gates

Targets: `10_performance_budget.md` §2 (end-to-end), §3 (CPU), §4 (memory/VRAM), §12 (per-tool SLA).

Hot loops:

- Capture: zero allocs/frame, texture pool reused
- Reflex tick: zero allocs, pre-compiled `EventFilter`
- Action emit: ≤ 1 alloc (the `Vec<INPUT>` for `SendInput`)
- Detection: pre-allocated tensors

Benchmarks use local Criterion baselines and exported `critcmp` JSON stored outside git (`%LOCALAPPDATA%\synapse\benchmarks\baselines\` for durable baselines, `.runs\benchmarks\` for candidates). `scripts/check-bench-delta.ps1` fails when a candidate export is missing a tracked benchmark or regresses by more than 20%. Per #350, do not use GitHub Actions/CI or committed `bench_results/<sha>/` baselines as the performance source of truth.

Spike check (`10_performance_budget.md` §15): subsystem > 2× p99 for > 5s ⇒ `synapse-performance-degraded` event + `health.subsystems.X.status = "degraded_latency"`.

---

## 7. Security gates

Per `11_security_and_safety.md`:

- stdio mode trusts parent process; no extra auth
- HTTP mode: bearer token in `%APPDATA%\synapse\token.txt`, Origin/Host check, loopback-only default
- Redaction patterns (`synapse-core::redact`, 19 patterns at v1) apply to: `observe()` text, `read_text()`, `audio_transcribe()`, clipboard summaries, `CF_EVENTS` payloads, replay export, tracing logs, OTLP
- Forbidden capabilities (compile-time `#[cfg(feature)]` off): DLL injection, kernel drivers, raw process memory r/w, FS writes outside profile paths, non-loopback by default
- Panic hotkey `Ctrl+Alt+Shift+P` registered via `RegisterHotKey`; fires `ReleaseAll` + reflex disable in ≤ 50 ms
- `cargo deny`: allow only `MIT`, `Apache-2.0`, `BSD-2/3`, `MPL-2.0`, `ISC`, `Zlib`, `Unicode-3.0`, `BSL-1.0`, `CC0-1.0`. Block GPL/AGPL/SSPL.
- Supported-use gates: `08` §6 — explicit operator configuration for hardware HID and other sensitive capabilities

---

## 8. Observability gates

Per `12_observability.md`:

- Every subsystem instrumented with `tracing::instrument` or `span!`
- Metric set per subsystem (`12_observability.md` §4.1) registered through `synapse-telemetry::metrics`
- Labels bounded — no unbounded values (session IDs, image hashes) as label keys
- `tracing-appender::rolling` daily rotation, 7-day keep, gzip rotated, 500 MB dir cap
- `health` MCP tool returns subsystem statuses matching `06_data_schemas.md::SensorStatus`
- Crash dump on panic: `%LOCALAPPDATA%\synapse\crashes\YYYYMMDD-HHMMSS.dump` with backtrace + last 100 log lines + last 100 events

---

## 9. Storage discipline

Per `07_storage_and_profiles.md` §6 (data lifecycle):

- Every new CF declares TTL + soft cap + hard cap in `synapse-core::retention::DEFAULTS`. No "decide later."
- JSON for persisted typed records. `bincode` is disallowed after RUSTSEC-2025-0141 (ADR-0001); a future maintained binary codec requires an explicit ADR and migration plan.
- Per-frame writes forbidden — aggregate, batch every 100 ms or 64 KB
- Three cleanup layers: compaction filter, periodic GC (5 min), disk-pressure responder
- Verify storage pressure and GC through the live `storage_*` MCP diagnostic
  tools on the configured host, with a separate `storage_inspect`/daemon-log
  readback after each trigger. A constrained local DB volume can support
  investigation, but it is not FSV by itself (`07_storage_and_profiles.md`
  §6.3/§7).

---

## 10. ADR / Open Question workflow

When a `16_open_questions.md` decision is forced during a phase:

1. Check OQ list first
2. If matching OQ exists and answer is now clear ⇒ create `docs/adr/NNN-<title>.md` with decision rationale + diff to PRD
3. Update OQ entry to `## OQ-NNN — <summary> — DECIDED <YYYY-MM-DD>` pointing to the ADR
4. Patch any PRD doc whose claim becomes stale
5. PR title: `adr(NNN): <one-line>`

No silent decisions. A code change that resolves an open question without an ADR fails review.

---

## 11. Cross-doc consistency check

Local checks:

- Every `pub const` in `synapse-core::error_codes` listed in `06_data_schemas.md` §8
- Every CF name listed in `07_storage_and_profiles.md` §4
- Every tool name listed in `05_mcp_tool_surface.md` §2 and registered in `synapse-mcp::tools`
- Every error code thrown in code has a `pub const` definition
- Markdown links in `docs/**/*.md` resolve

Run: `scripts/check_docs.ps1` (M0 deliverable).

---

## 12. Dev-environment notes (informational, not contract)

These are facts about the current dev box that help when iterating, not requirements on the shipped product. New contributors are not assumed to have any of this set up.

| Resource | Detail |
|---|---|
| WSL → Windows PulseAudio bridge | Windows host runs `pulseaudio.exe` v1.1 on TCP `127.0.0.1:4713` (mirrored WSL networking). Useful when building Block D of M3 (`04 §Block D`, work-items 16-18) for replaying audio fixtures from WSL or recording `output.monitor` without leaving the Linux shell. Does **not** replace WASAPI loopback — production capture stays direct on Windows. Full snapshot + setup commands in #85. |

---

## 13. Definition of release-ready

Per `15_roadmap_and_milestones.md` §10 — repeated here for forcing function:

1. M0-M5 demo gates all pass
2. Perf budgets met on reference machine (RTX 3060 + 8-core)
3. Local configured-host verification green on the release candidate
4. Soak 8 h clean
5. Manual test plan signed off (`13_testing_strategy.md` §15)
6. PRD docs internally consistent
7. `cargo deny check` clean
8. No `unsafe` outside `synapse-capture` / `synapse-hid-host` / `firmware/pico-hid`
9. No `unwrap()` outside test code
10. Crash dumps land on intentional panics
