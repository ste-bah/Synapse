# 13 — Testing Strategy

## 1. Why testing is hard here

Synapse is hard to test because:

- It touches the real OS (UIA trees, real windows, real input emission)
- It runs at frame rate (5-second fixture setups aren't useful)
- Many bugs are timing-sensitive (reflex correct on idle CPU, races slow capture under load)
- HID output is observable only by another OS process, not the test
- Some components (hardware HID) require physical hardware

We layer tests carefully. Unit tests are cheap and ubiquitous. Integration tests scope to subsystems with OS-layer fakes. E2E exercises run against the configured Windows host for manual FSV and release candidates.

Manual FSV is not an automated test class. The agent must define the source of
truth, read it before, trigger the behavior, read it again, and record the
actual state for the happy path plus at least three edge cases. Automated tests,
benchmarks, scripts, harnesses, GitHub Actions, and CI are supporting evidence
only.

For Synapse behavior, manual FSV also starts by proving the real `synapse-mcp`
runtime is active. The agent reads the process/stdout or socket state,
authenticates when HTTP is used, calls `health`, initializes an MCP session, and
reads `tools/list` before invoking the behavior under review. When an MCP tool
exists, the trigger is the real `tools/call`; a CLI, helper binary, test,
benchmark, or direct database write is supporting evidence only.

For the #536 delta-first reality work, manual FSV must prove both sides of the
contract: the delta stream contains the known change, and a periodic full audit
agrees with or corrects the agent's accumulated assumption by reading the
physical SoT.

"Hard to test" is never an excuse for not testing.
Likewise, a missing local prerequisite is not an excuse to stop. If the
operator could acquire, install, connect, configure, generate, launch, flash, or
inspect it from this computer, the agent must use Synapse/local host control to
make it real, then verify the physical source of truth.

---

## 2. The test pyramid

| Layer | Count | Where | Per-PR? |
|---|---|---|---|
| **Unit** | 1000s | Inside each crate (`#[cfg(test)] mod tests`) | Yes |
| **Integration** | 100s | Workspace-level `tests/integration/` | Yes |
| **Property-based** | 10s of properties | `proptest` in critical crates | Yes |
| **Snapshot** | 10s | `insta` for stable outputs | Yes |
| **Performance regression** | dozens of benches | `criterion` + `critcmp` exported JSON | Local manual gate |
| **End-to-end on real Windows** | ~10 scenarios | Manual configured-host runs driven through `synapse-mcp` | Configured-host FSV |
| **Hardware-in-the-loop** | ~5 scenarios | RP2040 attached to host | Hardware work-items |
| **Profile validation** | Per profile | Auto-generated from `profiles/*.toml` | Yes |

---

## 3. Unit tests

Rules:

- Every non-trivial public function has at least one test
- Every error variant has a test that triggers it
- Every `pub const` (CF names, error codes) is asserted to match its literal in a test
- No `unwrap()` outside test code
- Non-deterministic inputs use a fixed seed

Example contract test:

```rust
#[test]
fn cf_names_match_constants() {
    assert_eq!(synapse_core::cf::CF_EVENTS, "events");
    assert_eq!(synapse_core::cf::CF_OBSERVATIONS, "observations");
    // ... 1 per CF
}

#[test]
fn error_codes_match_constants() {
    use synapse_core::error_codes::*;
    assert_eq!(ACTION_QUEUE_FULL, "ACTION_QUEUE_FULL");
    assert_eq!(OBSERVE_NO_PERCEPTION_AVAILABLE, "OBSERVE_NO_PERCEPTION_AVAILABLE");
    // ... 1 per code
}
```

Drift between code and docs becomes a test failure.

---

## 4. Integration tests

Scoped to a subsystem with the OS layer replaced by a fake. Layered like production but composable.

### 4.1 Capture fakes

`synapse-capture` exposes `MockCaptureSource` emitting fixture frames. Perception, detection, OCR run against this without touching GPU.

```rust
let source = MockCaptureSource::from_dir("tests/fixtures/frames/menu_screen/")?;
let perception = Perception::new(source, /* ... */);
let observation = perception.observe()?;
assert_eq!(observation.entities.len(), 3);
```

### 4.2 UIA fakes

`synapse-a11y` exposes `MockUiaTree` for deterministic tests. Same `UIElement`-like interface reading from JSON fixture:

```json
{
  "root": {
    "name": "Untitled - Notepad",
    "role": "Window",
    "children": [
      {"name": "File", "role": "MenuItem", "patterns": ["Invoke"]},
      ...
    ]
  }
}
```

Real UIA is tested in E2E only.

### 4.3 Action sinks

`synapse-action::backends::software::Backend` is `SendInput`-based in production; tests substitute a `RecordingBackend` that records calls without emitting OS input.

Reflex runtime tests use this exclusively — verifying "given this event stream, this action sequence would have been emitted" without the real OS.

### 4.4 Storage isolation

Every test gets a `tempfile::TempDir`-backed RocksDB via `synapse-test-utils::TestDb`. Wipe-on-drop.

---

## 5. Property-based tests

`proptest` for critical invariants:

- **Event filter evaluator** — round-trip serialization, `And`/`Or` ordering doesn't change result, `Not(Not(x)) == x` for total filters.
- **Aim curves** — generated start/end produce step sequences whose first step starts at start, last ends at end, total ms matches duration within tolerance.
- **Keystroke dynamics** — generated text round-trips, no chars dropped, modifier-state consistent.
- **Coordinate transforms** — `screen_to_window(window_to_screen(p, h), h) == p` for any window.
- **JSON round-trip** — every persistable type round-trips through `serde_json` bytes-identically after canonical serialization.

Critical bug class: action emitter dropping `KeyUp` after `KeyDown` = stuck key. Property test:

```rust
proptest! {
    #[test]
    fn no_stuck_keys(actions in vec(arb_action(), 0..100)) {
        let mut emitter = ActionEmitter::new(RecordingBackend::new());
        for action in &actions {
            emitter.execute(action.clone()).unwrap();
        }
        emitter.flush();
        emitter.release_all();
        assert!(emitter.backend().held_keys().is_empty());
        assert!(emitter.backend().held_buttons().is_empty());
    }
}
```

---

## 6. Snapshot tests

`insta` for stable outputs (tool schemas, observation JSON shape, error response shape):

```rust
#[test]
fn observation_schema_snapshot() {
    let obs = sample_observation();
    insta::assert_json_snapshot!(obs);
}
```

If the schema changes, `cargo insta review` accepts the new snapshot. Reviewers see the diff in PR — schema changes are visible.

---

## 7. Performance regression tests

`criterion` benches for hot paths:

```rust
fn bench_observe_warm(c: &mut Criterion) {
    let setup = warm_synapse();
    c.bench_function("observe_warm", |b| {
        b.iter(|| setup.observe(default_params()))
    });
}
```

Run on the configured Windows host with Criterion named baselines and `critcmp`. Durable baseline exports live under `%LOCALAPPDATA%\synapse\benchmarks\baselines\`; candidate exports live under `.runs\benchmarks\`. `scripts/check-bench-delta.ps1` compares exported `critcmp` JSON and fails if a candidate is missing a tracked benchmark or is more than 20% slower. Per #350, do not commit `bench_results/<sha>/` directories or use GitHub Actions/CI as the benchmark source of truth.

Tracked benches:

| Bench | Target p99 |
|---|---|
| `observe_warm_a11y_only` | ≤ 10 ms |
| `observe_warm_hybrid` | ≤ 30 ms |
| `event_to_subscriber` | ≤ 50 ms |
| `reflex_tick_jitter_idle` | ≤ 200 µs |
| `reflex_tick_jitter_under_load` | ≤ 500 µs |
| `aim_curve_step_calc_natural` | ≤ 1 µs |
| `action_software_press` | ≤ 3 ms |
| `detection_rtdetr_v2_s_coco_640` | ≤ 25 ms DirectML / ≤ 8 ms CUDA |
| `ocr_winrt_120x32` | ≤ 8 ms |
| `serialize_observation_typical` | ≤ 5 ms |

---

## 8. End-to-end manual FSV (real Windows)

Configured Windows 11 host. Each manual FSV run:

1. Reads whether `synapse-mcp` is already running and active; if not, launches
   a repo-owned stdio or loopback HTTP daemon with an issue-local DB/log path.
2. Reads process/socket state, authenticated `health`, initialized MCP session,
   and `tools/list` to prove the required tool is present.
3. Calls the real MCP `tools/call` for the behavior under review with known
   synthetic/manual inputs.
4. Reads the source of truth separately and records before/after state.

Test scenarios:

| Scenario | Verifies |
|---|---|
| `notepad_type_save` | Open Notepad, type text, save file, verify content |
| `vscode_open_file` | Open VS Code with file argument, observe element tree, find file in explorer |
| `chrome_navigate_form_submit` | Open Chrome, navigate URL, fill form, submit, observe response |
| `terminal_run_command` | Open Windows Terminal, type command, observe output |
| `multiwindow_focus_switch` | Open three apps, cycle focus, verify foreground events |
| `clipboard_round_trip` | Write clipboard from agent, read in different app, verify |
| `reflex_aim_track_static` | Track a stationary detected entity, verify aim_track stays on target |
| `reflex_combo_frame_perfect` | Execute a 3-step combo, verify HID emission times |
| `safety_release_all_on_panic` | Hold keys, kill daemon, verify all keys released |
| `disk_pressure_response` | Fill DB to soft cap, verify GC runs and cleanup happens |

Each 5–60 seconds. Total nightly run: ~15 minutes.

### 8.1 Determinism

The OS isn't deterministic. Resilience via:

- Wait on UIA events rather than `sleep`
- Use `find()` to locate elements rather than coordinates
- Assert on event sequences (loose ordering with constraints) rather than exact timestamps

A test that times out fails with the last event log.

### 8.2 Headless game scenarios

Game E2E doesn't require GPU + game on the runner:

- Use `cargo-mommy`-style fake-game test rig: small Bevy app rendering predictable scenes, emitting known windows/HUD layouts.
- E2E targets this fake game first; real-game tests are manual demo scripts the maintainer runs at release.

---

## 9. Hardware-in-the-loop tests

Self-hosted runner has an RP2040 board attached. Rig:

- Pico flashed with Synapse firmware
- A second Pico configured as a **measurement device** that captures HID reports and timestamps them

| Scenario | Asserts |
|---|---|
| `hid_mouse_move_latency` | Round-trip latency p99 ≤ 5 ms |
| `hid_combo_timing` | 3-step combo step intervals within 0.5 ms of scheduled |
| `hid_release_all_on_disconnect` | Host disconnect → watchdog releases everything within 1 s |
| `hid_high_volume` | 10,000 mouse-move commands at full rate, no drops |
| `hid_reflash` | Reset to bootloader, flash, verify new identity |

Run as hardware work-items or release-candidate checks on the configured host.
If the hardware is absent, the work item is not accepted until the operator's
target hardware is present or the issue explicitly scopes it out.

---

## 10. Profile validation tests

Every `profiles/*.toml` auto-validated on PR:

- Parses against `Profile` struct
- All keymap aliases resolve to known key codes
- All HUD region pointers are in-bounds
- All `event_extensions` have valid `EventFilter` syntax
- All model_ids resolve to a known model

A PR breaking a profile fails before merge.

Each bundled profile has a smoke test:

```rust
#[test]
fn profile_minecraft_smoke() {
    let prof = synapse_profiles::load("minecraft.java").unwrap();
    assert_eq!(prof.mode, PerceptionMode::PixelOnly);
    assert!(prof.hud.iter().any(|h| h.name == "hp_hearts"));
    assert!(prof.keymap.contains_key("attack"));
}
```

---

## 11. Fuzz testing

`cargo-fuzz` harnesses for protocol parsers:

- MCP JSON-RPC parser
- HID serial protocol frame parser
- EventFilter parser
- Profile TOML parser

Each target runs locally with `--max-total-time=600` (10 minutes) when parser or protocol changes require it. Crashes are release-blocking; corpus is committed.

---

## 12. Soak tests

`tests/soak/`. Long-running test:

- Spawns Synapse
- Runs synthetic workload (frames at 60 fps, fake agent calls `observe()` at 2 Hz, reflexes register/cancel at 0.1 Hz)
- Runs for 8 hours
- Asserts at end:
  - Memory growth ≤ 50 MB
  - No deadlocks
  - p99 latencies stable (no drift)
  - DB size respects soft caps

Triggered manually or weekly on a dedicated runner.

---

## 13. Replay-driven regression tests

Captured replays become regression fixtures. Workflow:

1. Operator hits a bug
2. Exports replay (`synapse-mcp replay export`)
3. Files bug with the `.zip`
4. Maintainer adds replay as fixture in `tests/replays/<bug_id>/`
5. A test loads the fixture, feeds Synapse the events, asserts the bug is fixed

Builds a regression corpus from real bugs.

---

## 14. Supporting automated-check reference

Manual configured-host FSV is the shipping gate. Do not dispatch or wait on GitHub Actions/CI unless the operator explicitly reverses the no-CI decision. These checks are supporting evidence only and must not be named or treated as FSV:

| Job | OS | Trigger |
|---|---|---|
| `cargo fmt --check` | configured host | changed Rust code |
| `cargo clippy --workspace --all-targets -- -D warnings` | configured host | changed Rust code |
| `cargo test --workspace` | configured host | supporting regression sweep |
| `cargo test --workspace --no-default-features` | configured host | feature-surface changes |
| `cargo build --release --workspace` | configured host | release candidate |
| `cargo deny check` | configured host | dependency changes |
| `cargo audit` | configured host | dependency/security sweep |
| `insta review --check` | configured host | snapshot changes |
| `e2e-real-windows` | configured Windows host | issue-specific manual FSV support |
| `bench-regression` | configured Windows host | local `critcmp` export compare |
| `hardware-in-loop` | self-hosted with Pico | weekly |
| `soak` | self-hosted windows | weekly |
| `fuzz` | configured host | parser/protocol changes, 10min per target |

Do not add new `*_fsv` tests, FSV harnesses, or FSV scripts. Legacy artifacts
with FSV naming are regression artifacts until renamed or removed.

---

## 15. Manual test plan (release gate)

M2+ uses manual FSV on the operator's configured Windows host as the shipping
gate. GitHub Actions/CI, automated scripts, and missing-dependency
portability tests are not release gates. A missing configured-host prerequisite
is an acquisition/setup task, not a reason to mark the issue blocked by itself:
figure out where the thing must come from, where it must physically appear, and
make it happen on this configured host. Treat Synapse/local control as the
operator-equivalent host control surface with full local computer-control
responsibility: if the operator could download, install, connect, configure,
generate, flash, launch, or inspect it from this host, the agent must attempt
those reversible local steps using Synapse plus host OS, shell, browser,
package-manager, and device-management workflows. Do not ask the operator to
download or install something while reversible local acquisition/setup remains
possible. Read the authoritative SoT directly before shipping.
Do not stop at "missing." Synapse gives the agent the same practical local
ability the operator has at this keyboard. If the operator could make the
prerequisite real from this computer, the agent must do the reversible local
work through Synapse and host workflows, then inspect the physical SoT.
Missing local state creates the next action for the agent and must be made
real, not handed back to the operator or treated as a blocker while reversible
host work remains.
Nothing is ever `status:blocked` because a configured-host prerequisite is
absent; the only blockable item is the exact operator-only hard-to-reverse
external action left after every reversible local step is exhausted.
Browser downloads, GUI installers, Device Manager checks, package-manager
installs, model/file generation, firmware flashing, app launching, USB/COM
inspection, and UI inspection count as that reversible local work. They are
agent-owned on the configured host and must not be handed back to the operator
while Synapse/local control can perform them.
Only hard-to-reverse external actions such as spending money, private
credentials, billing, or external account changes require narrow operator
approval, and every reversible local step must be complete before asking.
Before tagging a release, the maintainer runs:

1. **Configured Windows 11 host.** Verify ViGEmBus is installed, Synapse is installed, Claude Desktop is connected, and run "open Notepad, type, save".
2. **Live MCP runtime.** Verify `synapse-mcp` is running from the expected
   binary, authenticated `health` is ok, `tools/list` exposes the required
   tools, and at least one read tool plus one write/gated tool are exercised
   through real `tools/call` with separate SoT readback.
3. **Live game session.** Pick one bundled game profile, play 15 minutes via agent, verify reasonable behavior and no stuck inputs.
4. **Hardware HID flash + smoke.** Flash a Pico, connect, run hardware aim test.
5. **Panic hotkey drill.** Start a long-running reflex, hit `Ctrl+Alt+Shift+P`, verify everything stops within 100 ms.
6. **Disk pressure drill.** Fill a small DB volume, verify pressure transitions, verify operation continues degraded but not broken.

Maintainer signs off with a release-notes entry summarizing what they tested.

---

## 16. Code coverage targets

- `synapse-core`: 95% line coverage. Pure types + small logic; exhaustive.
- `synapse-storage`, `synapse-profiles`, `synapse-reflex`, `synapse-action`: 85%
- `synapse-capture`, `synapse-a11y`, `synapse-audio`, `synapse-perception`: 70% (OS-bound, harder to cover)
- `synapse-models`, `synapse-hid-host`, `synapse-telemetry`: 80%

`tarpaulin` on Linux (where possible) + Windows for OS-bound crates can provide supporting coverage deltas. A >5% drop blocks merge when reviewed locally.

---

## 17. What this doc does NOT cover

- Specific test fixture details → `tests/fixtures/`
- Supporting automation configuration files → `.github/workflows/`
- Hardware test rig wiring → `09_hardware_hid_gateway.md`
- Profile authoring tutorial → community wiki (post-v1)
