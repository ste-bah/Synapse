# 05 - M4: Hardware HID + First Game Profile (2-3 weeks) - ACTIVE

**Status:** M4 is active. This document is the self-contained M4 plan a fresh
agent should read before touching hardware HID, Minecraft profile, or M4 MCP
tool work.

**Issue state is canonical.** GitHub Issues are the live work queue. This file
summarizes the queue as read on 2026-05-26 after `git pull --ff-only`, but an
agent must re-read the current issue before claiming work. If source, docs, and
issues disagree, inspect the physical source of truth and update the issue.

**Binding doctrine:** issue #351 and `AGENTS.md` control this phase. Manual FSV
is mandatory and cannot be replaced by tests, scripts, benchmarks, CI, GitHub
Actions, harnesses, or return values. Missing tools, drivers, models, files,
services, firmware, account state, or hardware are acquisition/setup work, not a
reason to mark an issue blocked by absence alone. Synapse gives the agent local
computer-control responsibility; treat Synapse/local control as the
operator-equivalent host control surface. If the operator could download,
install, connect, configure, generate, flash, launch, or inspect it from this
host, the agent must attempt those reversible local steps using Synapse plus
normal host workflows before
asking the operator. Then read the authoritative source of truth directly. Ask
only for narrow approval before hard-to-reverse external actions such as
spending money, private credentials, billing, external-account changes, or
irreversible shared-state changes.
Within M4, browser downloads, GUI installers, Device Manager checks,
package-manager installs, model/file generation, firmware flashing, launching
apps, USB/COM inspection, and Synapse-driven UI inspection are agent-owned work
when reversible on this host; they are not reasons to stop and are not operator
errands while the agent can do them locally.
Missing local state creates the next action for the agent, not a blocker while
reversible host work remains. Use Synapse's local computer-control surface to
keep moving until the remaining step is a specific operator-only external
decision.
Nothing is ever `status:blocked` because a configured-host prerequisite is
absent; the only blockable item is the exact operator-only hard-to-reverse
external action left after every reversible local step is exhausted.

**No GitHub Actions or CI gate.** Agent commits pushed during M4 include
`[skip ci]`. Local checks may support regression confidence, but the shipping
gate is configured-host manual FSV with source-of-truth readback.

**PRD authority:** [15_roadmap_and_milestones.md](../computergames/15_roadmap_and_milestones.md)
section 6, [09_hardware_hid_gateway.md](../computergames/09_hardware_hid_gateway.md),
[03_action.md](../computergames/03_action.md),
[05_mcp_tool_surface.md](../computergames/05_mcp_tool_surface.md),
[06_data_schemas.md](../computergames/06_data_schemas.md),
[08_supported_use_policy.md](../computergames/08_supported_use_policy.md),
[10_performance_budget.md](../computergames/10_performance_budget.md),
[11_security_and_safety.md](../computergames/11_security_and_safety.md),
[13_testing_strategy.md](../computergames/13_testing_strategy.md), and
[14_build_and_packaging.md](../computergames/14_build_and_packaging.md).
Implementation doctrine: [00_methodology.md](00_methodology.md) and
[07_cross_cutting.md](07_cross_cutting.md).

---

## 0. Mission

**Ship the first real-device game-control path: RP2040 firmware, the
`synapse-hid-host` serial gateway, `Backend::Hardware`, the three M4 MCP tools,
and a `minecraft.java` single-player profile that can complete the M4 demo on
the configured Windows host with manual source-of-truth evidence.**

Everything in M4 traces back to that sentence:

- A physical Pico must enumerate as Synapse's composite HID + CDC device.
- The host must identify, command, reconnect, and flash the firmware.
- The action subsystem must route real actions to hardware without silently
  falling back to software or ViGEm.
- The MCP tool surface must expose `act_combo`, `act_run_shell`, and
  `act_launch` with explicit safety gates.
- The Minecraft profile must resolve a real Minecraft Java window, read HUD
  state, emit game events, and drive only supported `single_player` use.
- Missing local prerequisites must be made real on this configured host.

---

## 1. Starting State Verified Against `main`

This section was verified by reading repo files and issue state on 2026-05-26.
It is not a substitute for re-reading the active issue before claiming work.

### 1.1 Repository state

- `main` was up to date with `origin/main` after `git pull --ff-only`.
- The working tree was clean before this rewrite.
- M3 is closed and M4 is active. M4 context lives in issue #355.
- M4 Block A.0 carry-over refactors are closed in issues #356-#367.
- Issue #351 forbids CI/GitHub Actions as a gate and forbids automated FSV.
- Issue #353 requires frequent issue journaling, scoped `[skip ci]` commits,
  prompt pushes to `origin/main`, and clean handling of unrelated local files.

### 1.2 Live M3 surface M4 builds on

The live MCP surface remains 30 tools:

```
act_aim, act_click, act_clipboard, act_drag, act_pad, act_press, act_scroll,
act_type, audio_tail, audio_transcribe, find, health, observe, profile_activate,
profile_list, read_text, reflex_cancel, reflex_history, reflex_list,
reflex_register, release_all, replay_record, set_capture_target,
set_perception_mode, storage_gc_once, storage_inspect, storage_pressure_sample,
storage_put_probe_rows, subscribe, subscribe_cancel
```

M4 adds three agent-facing MCP tools: `act_combo`, `act_run_shell`, and
`act_launch`. The `hid identify` and `hid flash` surfaces are CLI subcommands,
not additions to the agent-facing 33-tool count.

### 1.3 Firmware state

`firmware/pico-hid/` exists in the repo and is excluded from the root Cargo
workspace as a standalone firmware project. Current tree:

```
firmware/pico-hid/
|-- Cargo.toml
|-- Cargo.lock
|-- build.rs
|-- memory.x
|-- src/
|   |-- dispatch.rs
|   |-- hid_descriptors.rs
|   |-- led.rs
|   |-- lib.rs
|   |-- main.rs
|   |-- protocol.rs
|   |-- reports.rs
|   |-- safety.rs
|   |-- serial.rs
|   `-- usb.rs
`-- tests/
    |-- led_patterns.rs
    |-- loopback_dispatch.rs
    |-- protocol_roundtrip.rs
    `-- safety_watchdog.rs
```

The source tree is ahead of the old sketch in this file, but the firmware
issues remain canonical until each issue is manually verified and closed.
Hardware-bound rows require a real Pico and configured-host readback, not only
host-side tests.

### 1.4 Host driver state

`crates/synapse-hid-host` is no longer a stub. Current tree:

```
crates/synapse-hid-host/
|-- Cargo.toml
|-- src/
|   |-- discover.rs
|   |-- error.rs
|   |-- handshake.rs
|   |-- lib.rs
|   |-- pipeline.rs
|   |-- protocol.rs
|   |-- reconnect.rs
|   `-- transport.rs
`-- tests/
    |-- discover.rs
    |-- handshake.rs
    |-- pipeline.rs
    |-- scaffold.rs
    `-- transport.rs
```

The crate contains protocol, handshake, discovery, pipeline, transport, and
reconnect work. The reconnect source checkpoint is committed, but issue #387 is
not done until manual hardware FSV proves unplug/replug behavior on a real COM
port with source-of-truth reads.

### 1.5 Action, profile, and safety state

- `Backend::Hardware` still needs the real `HardwareBackend` route in
  `synapse-action`; until then hardware requests must fail closed instead of
  falling back.
- `synapse-core` already contains the Synapse Pico USB identity constants and
  firmware version constants, but issue state remains canonical for closing the
  corresponding M4 rows.
- `synapse-core::error_codes` already includes the M4 HID and safety strings
  read by this plan. Issue #436 remains open until its own acceptance and manual
  evidence are recorded.
- Existing bundled profiles are under `crates/synapse-profiles/profiles/`:
  `chrome.toml`, `notepad.toml`, `terminal.toml`, and `vscode.toml`.
  `minecraft.java.toml` and its asset bundle are not present yet.
- No `profiles/assets/minecraft.java/` bundle is present yet.

### 1.6 Physical host state

The last hardware readback recorded on #387 found no attached Synapse Pico,
Pico/RP2040 USB serial device, matching registry USB Enum row, or `RPI-RP2`
firmware volume on the configured host. That absence is not a terminal blocker:
the M4 agent must make the hardware path real by performing reversible setup,
procurement research, flashing, driver/device checks, and direct source-of-truth
readback. Purchasing hardware, using private accounts, or other hard-to-reverse
external actions require narrow operator approval after reversible steps are
complete.

---

## 2. Demo Gate

### 2.1 Primary demo

Environment: configured Windows 11 host, Synapse MCP running locally, Minecraft
Java Edition running a local single-player world, supported-use profile active.

Flow:

1. Agent calls `observe()`.
2. Observation includes Minecraft foreground identity, HUD heart/hunger/xp
   values, and visible entities.
3. Agent walks the sequence: find tree, break tree, make planks, make
   workbench.
4. Agent uses `act_press`, `act_aim`, and one or two reflexes such as
   `auto_attack_low_hp`.
5. The run lasts 5 minutes hands-off.

Primary source-of-truth evidence:

- Minecraft UI/frame readback shows HP/hunger/xp state before and after.
- Profile/runtime state shows `minecraft.java` active with
  `use_scope = "single_player"`.
- Reflex audit rows show any reflex registration and fire results.
- Action/backend telemetry shows every emitted action and backend used.
- Real game state on screen shows planks/workbench result after the sequence.

### 2.2 Hardware demo

Same scenario, started with `--hardware-hid auto` and a flashed Synapse Pico
attached. It is not optional for M4 closure; it is the real hardware gate.

Additional source-of-truth evidence:

- `Get-PnpDevice`, `Win32_SerialPort`, registry Enum key, or equivalent direct
  device read shows the Synapse Pico HID and COM device.
- `hid identify` returns firmware identity matching `EXPECTED_FW_MAJOR`.
- External OS input observations show hardware keyboard/mouse/gamepad reports,
  not software fallback.
- Unplug mid-stream causes `ACTION_HID_PORT_DISCONNECTED`; replug resumes via
  reconnect within the acceptance window.

---

## 3. Deliverables

### 3.1 Firmware deliverable

The firmware implements the PRD 09 composite device and serial protocol:

```
firmware/pico-hid/
|-- Cargo.toml                  # standalone thumbv6m-none-eabi firmware crate
|-- memory.x                    # RP2040 linker script
|-- build.rs                    # embeds build/version data and UF2 support
|-- src/
|   |-- main.rs                 # embassy executor and task spawn
|   |-- usb.rs                  # composite HID + CDC ACM descriptor builder
|   |-- hid_descriptors.rs      # mouse, keyboard, gamepad descriptors
|   |-- reports.rs              # HID report structs and neutral states
|   |-- serial.rs               # CDC ACM framed byte stream
|   |-- protocol.rs             # MAGIC/LEN/SEQ/CMD/payload/CRC parser
|   |-- dispatch.rs             # command to report/telemetry dispatch
|   |-- safety.rs               # watchdog and internal RELEASE_ALL
|   `-- led.rs                  # idle/active/watchdog/error LED patterns
`-- tests/
    |-- protocol_roundtrip.rs
    |-- loopback_dispatch.rs
    |-- safety_watchdog.rs
    `-- led_patterns.rs
```

Firmware must enumerate as:

- HID boot mouse.
- HID boot keyboard.
- Standard HID/DirectInput gamepad with 14-byte report.
- CDC ACM control/data pair for the command channel.

Canonical identity:

```
VID: 0x2E8A
PID: 0x1F50
Manufacturer: Synapse
Product: Synapse Pico HID
Serial prefix: SYN-PICO-HID
```

### 3.2 Host driver deliverable

`synapse-hid-host` owns:

- `HidGateway::connect(port_name)` at 1 Mbaud setting, 8-N-1, 5 ms timeout.
- `IDENTIFY` handshake and firmware major version validation.
- CRC16/CCITT-FALSE frame encode/decode.
- Up to 16 outstanding unacked frames.
- ACK timeout after 5 ms, retry up to 3 times, then `HID_LINK_TIMEOUT`.
- Auto-detect for `--hardware-hid auto` by enumerating matching USB VID/PID
  serial ports and proving identity.
- Reconnect loop every 500 ms on serial disconnect.
- Immediate fail-fast `ACTION_HID_PORT_DISCONNECTED` while disconnected.
- `hid identify` and `hid flash` support through `synapse-mcp` CLI surfaces.

### 3.3 Action backend deliverable

`synapse-action` adds a real hardware backend and routes only the action
variants supported by the firmware:

| Action variant | Hardware command |
|---|---|
| `MouseMoveRelative` | `MOUSE_MOVE_REL [i16 dx][i16 dy]` |
| `MouseMove` / `AimAt` screen target | `GetPhysicalCursorPos` readback + sampled `MOUSE_MOVE_REL` batch |
| `MouseMove` / `AimAt` element target | UIA bbox center resolution + sampled `MOUSE_MOVE_REL` batch |
| mouse button down/up | `MOUSE_BUTTON [u8 button][u8 down_flag]` |
| wheel | `MOUSE_WHEEL [i8 dy][i8 dx]` |
| key down/up | `KEY_DOWN [u8 hid_code]`, `KEY_UP [u8 hid_code]` |
| modifier state | `KEY_MODS [u8 mods_bitfield]` |
| pad report | `PAD_REPORT [14 raw bytes]` |
| release all | `RELEASE_ALL` |

The backend must never silently downgrade hardware requests to software or
ViGEm. Absolute hardware mouse requests read the current cursor position,
resolve element targets to a screen point when UIA can re-resolve them, sample
the requested curve, chunk every delta to the firmware's `-127..=127` relative
range, and send the resulting `MOUSE_MOVE_REL` stream as a HID pipeline batch.
Unresolved element and track targets still fail closed with no command stream.
Source-of-truth evidence must show which path fired.

### 3.4 MCP and CLI deliverables

M4 adds these agent-facing MCP tools:

- `act_combo`
- `act_run_shell`
- `act_launch`

M4 adds these operator-facing CLI subcommands:

- `synapse-mcp hid identify --port COM7`
- `synapse-mcp hid identify --port auto`
- `synapse-mcp hid flash --port COM7`
- `synapse-mcp hid flash --port auto`

### 3.5 Minecraft profile deliverable

```
crates/synapse-profiles/profiles/minecraft.java.toml
crates/synapse-profiles/profiles/assets/minecraft.java/
|-- hearts/
|   |-- full.png
|   |-- half.png
|   `-- empty.png
`-- hunger/
    |-- full.png
    |-- half.png
    `-- empty.png
```

Profile contents:

- `id = "minecraft.java"`
- `use_scope = "single_player"`
- `mode = "pixel_only"` unless a later issue explicitly broadens it.
- Window match for `javaw.exe` and Minecraft title.
- HUD specs for `hp_hearts`, `hunger`, and `xp`.
- Keymap for default Minecraft Java controls.
- Entity detection model decision from #415.
- Event extensions for `creeper_nearby` and `low_hp`.
- Natural motion defaults only; no `Instant` curves in bundled profile actions.

### 3.6 Policy and docs deliverables

- Hardware procurement and setup docs.
- Hardware troubleshooting docs.
- README firmware build and flash procedure.
- PRD sweeps for tool schemas and data schemas.
- Changelog release notes.
- M4 release tag and bundled `.uf2` asset when all gates pass.

---

## 4. MCP And CLI Schemas

Final schema details are locked by issues #445 and #446. This section states the
minimum M4 contract so implementation issues share the same target.

### 4.1 `act_combo`

Purpose: execute a timed sequence of already-supported actions through the
reflex combo scheduler, preserving backend routing per step.

Input shape:

```json
{
  "steps": [
    {
      "at_ms": 0,
      "action": "act_press",
      "params": { "keys": ["w"], "hold_ms": 100 },
      "backend": "hardware"
    }
  ],
  "backend": "auto",
  "idempotency_key": "optional-client-token"
}
```

Rules:

- `steps` is non-empty.
- `at_ms` values are monotonic and bounded by the combo maximum.
- Each `action` must be an allowed action tool, not `act_run_shell`,
  `act_launch`, subscription tools, storage diagnostics, or profile writes.
- Per-step `backend` overrides the top-level backend.
- The combo compiles to a one-shot `combo` reflex and records audit state.

Output shape:

```json
{
  "combo_id": "uuid",
  "scheduled_steps": 1,
  "backend": "hardware",
  "started_at_ms": 0
}
```

### 4.2 `act_run_shell`

Purpose: run a local shell command only when startup policy explicitly allows
the command string.

Input shape:

```json
{
  "command": "cmd.exe",
  "args": ["/c", "echo synapse-m4-shell-ok"],
  "working_dir": "C:\\code\\Synapse",
  "env": { "SYNAPSE_SYNTHETIC": "1" },
  "timeout_ms": 5000,
  "idempotency_key": "optional-client-token"
}
```

Rules:

- Startup config must include `--allow-shell <regex>`.
- Broad patterns such as `.*` are rejected at startup.
- The resolved command line must match an allowlist entry.
- Stdout/stderr are capped; timeout kills the process tree.
- Denied commands return `SAFETY_SHELL_DENIED_BY_POLICY`.

Output shape:

```json
{
  "exit_code": 0,
  "stdout": "synapse-m4-shell-ok\r\n",
  "stderr": "",
  "timed_out": false
}
```

### 4.3 `act_launch`

Purpose: launch an allowed local executable and optionally wait for a matching
window title.

Input shape:

```json
{
  "target": "notepad.exe",
  "args": [],
  "working_dir": "C:\\Windows\\System32",
  "env": {},
  "wait_for_window_title_regex": ".*Notepad.*",
  "timeout_ms": 5000,
  "idempotency_key": "optional-client-token"
}
```

Rules:

- Startup config must include `--allow-launch <regex>`.
- Broad patterns such as `.*` are rejected at startup.
- The resolved target must match an allowlist entry.
- Window wait is optional but must read real window state when requested.
- Denied launches return `SAFETY_LAUNCH_DENIED_BY_POLICY`.

Output shape:

```json
{
  "pid": 1234,
  "hwnd": "0x00123456",
  "matched_title": "Untitled - Notepad"
}
```

### 4.4 `synapse-mcp hid identify`

Command:

```powershell
synapse-mcp hid identify --port COM7
synapse-mcp hid identify --port auto
```

Output fields:

```json
{
  "port": "COM7",
  "fw_major": 0,
  "fw_minor": 1,
  "fw_patch": 0,
  "build_hash": "8-byte-hex",
  "vid": "0x2E8A",
  "pid": "0x1F50",
  "capabilities": ["mouse", "keyboard", "gamepad", "telemetry", "bootloader"]
}
```

### 4.5 `synapse-mcp hid flash`

Command:

```powershell
synapse-mcp hid flash --port COM7
synapse-mcp hid flash --port auto
```

Required steps:

1. Identify firmware mode or prove no firmware mode exists.
2. Enter UF2 bootloader via `RESET_TO_BOOTLOADER` or operator BOOTSEL flow.
3. Wait for the `RPI-RP2` mass-storage volume.
4. Copy bundled `pico-hid-x.y.z.uf2`.
5. Wait for Synapse firmware re-enumeration.
6. Run `IDENTIFY` and print the final identity.

Manual FSV must read the actual USB/volume/device SoT at each phase.

---

## 5. Error Codes

Error code strings are part of the public contract. They must be thrown through
structured MCP error data or CLI structured output, and issue evidence must show
the actual code surfaced on the real trigger path.

### 5.1 Hardware HID

| Code | When it fires |
|---|---|
| `HID_PORT_NOT_FOUND` | Fixed port or auto-detect cannot find a usable Synapse Pico port. |
| `HID_PORT_OPEN_FAILED` | The COM port exists but cannot be opened/configured. |
| `HID_PROTOCOL_HANDSHAKE_FAILED` | `IDENTIFY` response is missing, malformed, wrong command, or invalid length. |
| `HID_FIRMWARE_VERSION_MISMATCH` | Firmware major differs from `EXPECTED_FW_MAJOR`. |
| `HID_COMMAND_REJECTED` | Firmware returns NAK for a sent command. |
| `HID_LINK_TIMEOUT` | ACK not received within retry budget. |
| `ACTION_HID_PORT_DISCONNECTED` | Hardware backend is disconnected or reconnecting. |

### 5.2 Safety and policy

| Code | When it fires |
|---|---|
| `SAFETY_PROFILE_ACTION_DENIED` | Active profile is `unknown` or policy denies writes/actions. |
| `SAFETY_SHELL_DENIED_BY_POLICY` | `act_run_shell` command is not allowlisted or policy invalid. |
| `SAFETY_LAUNCH_DENIED_BY_POLICY` | `act_launch` target is not allowlisted or policy invalid. |
| `SAFETY_OPERATOR_HOTKEY_FIRED` | Operator panic hotkey cancels pending/held input. |
| `SAFETY_RELEASE_ALL_FIRED` | Release-all interlock prevents a conflicting action. |
| `ACTION_QUEUE_FULL` | Hardware/actor backpressure refuses new work. |
| `ACTION_BACKEND_UNAVAILABLE` | Hardware was requested without enablement. |

---

## 6. Work Items

This table is sourced from the M4 GitHub issue queue. A row is not done because
code exists; it is done only when the issue is closed with manual FSV evidence.

### 6.1 Context and carry-over

| Issue | State | Work item |
|---|---|---|
| #355 | open | M4 context: Hardware HID + first game profile. |
| #356 | closed | A.0-1 split `synapse-a11y` over-cap file. |
| #357 | closed | A.0-2 split `synapse-capture` over-cap file. |
| #358 | closed | A.0-3 split `synapse-core` types. |
| #359 | closed | A.0-4 split `synapse-mcp/src/server.rs`. |
| #360 | closed | A.0-5 split `synapse-mcp/src/m3/reflex.rs`. |
| #361 | closed | A.0-6 split `synapse-reflex/src/lib.rs`. |
| #362 | closed | A.0-7 split `synapse-reflex/src/scheduler.rs`. |
| #363 | closed | A.0-8 split `synapse-mcp/src/http/sse.rs` and sparse-seq fix. |
| #364 | closed | A.0-9 split `synapse-mcp/src/m3/replay.rs`. |
| #365 | closed | A.0-10 split `synapse-models/src/lib.rs`. |
| #366 | closed | A.0-11 fix M3 changelog tool names. |
| #367 | closed | A.0-12 split over-cap test files. |

### 6.2 Block A - firmware

| Issue | State | Work item |
|---|---|---|
| #368 | open | A-01 create `firmware/pico-hid` Cargo project, memory map, embassy init, LED hello world. |
| #369 | open | A-02 ADR: USB VID/PID assignment for Synapse Pico HID. |
| #370 | open | A-03 USB CDC ACM serial channel and 10k-byte loopback echo. |
| #371 | open | A-04 HID composite descriptor: boot mouse, boot keyboard, HID gamepad. |
| #372 | closed | A-05 ADR: HID gamepad vs raw XInput emulation and ViGEm interplay. |
| #373 | closed | A-06 protocol parser: MAGIC/LEN/SEQ/CMD/payload/CRC16. |
| #374 | open | A-07 command dispatcher for mouse, keyboard, pad, release, watchdog, identify, telemetry. |
| #375 | open | A-08 watchdog default 1000 ms, internal RELEASE_ALL, telemetry counter. |
| #376 | open | A-09 LED status patterns. |
| #377 | open | A-10 telemetry counters. |
| #378 | open | A-11 elf2uf2 build pipeline and release firmware path. |
| #379 | open | A-12 `EXPECTED_FW_MAJOR` constant and version drift handshake. |
| #380 | closed | A-13 host-side protocol roundtrip tests. |
| #381 | open | A-14 loopback firmware build for off-target debugging. |

### 6.3 Block B - host driver

| Issue | State | Work item |
|---|---|---|
| #382 | closed | B-01 scaffold `synapse-hid-host`. |
| #383 | open | B-02 `HidGateway::connect(port_name)`, 1 Mbaud, 5 ms timeout. |
| #384 | open | B-03 `IDENTIFY` handshake and version check. |
| #385 | open | B-04 pipelined send, 16 outstanding, 5 ms ACK, 3 retries. |
| #386 | open | B-05 auto-detect via `--hardware-hid auto`. |
| #387 | open | B-06 reconnect loop every 500 ms on serial disconnect. |
| #388 | open | B-07 host-side CRC16/CCITT-FALSE encode/decode. |
| #389 | open | B-08 frame reassembly across CDC ACM packet boundaries. |
| #390 | open | B-09 send-buffer backpressure to `ACTION_QUEUE_FULL`. |
| #391 | open | B-10 in-process loopback fixture for supporting checks. |

### 6.4 Block C - action hardware backend

| Issue | State | Work item |
|---|---|---|
| #392 | closed | C-01 `HardwareBackend` struct in `synapse-action`. |
| #393 | closed | C-02 replace `HardwareUnavailableBackend` when `--hardware-hid` is set. |
| #394 | closed | C-03 Synapse key to HID usage-code mapping. |
| #395 | closed | C-04 HID boot keyboard 6KRO and modifier handling. |
| #396 | closed | C-05 hardware mouse relative-only rule and absolute fallback. |
| #397 | closed | C-06 hardware `ReleaseAll` to firmware `RELEASE_ALL`. |
| #398 | closed | C-07 held-state interlock between hardware and software backends. |
| #399 | open | C-08 `Backend::Auto` resolution for hardware-capable hosts. |

### 6.5 Block D - combo, shell, launch

| Issue | State | Work item |
|---|---|---|
| #400 | open | D-01 `Action::Combo` lifetime semantics. |
| #401 | open | D-02 `act_combo` MCP tool. |
| #402 | open | D-03 `act_combo` schema snapshot and default-resolution row. |
| #403 | open | D-04 `act_run_shell` allowlist parsing and dispatch. |
| #404 | open | D-05 `act_run_shell` stdout/stderr caps and timeout. |
| #405 | open | D-06 `act_run_shell` broad-pattern rejection. |
| #406 | open | D-07 `act_launch` with working dir, env, and window wait. |
| #407 | open | D-08 `act_launch` broad-pattern rejection. |
| #408 | open | D-09 CLI args: `--allow-shell`, `--allow-launch`, `--hardware-hid`. |
| #409 | open | D-10 shell/launch safety error codes. |

### 6.6 Block E - perception HUD and event extensions

| Issue | State | Work item |
|---|---|---|
| #410 | open | E-01 HUD template-match extractor. |
| #411 | open | E-02 `event_extensions` evaluator. |
| #412 | open | E-03 HUD region anchor system. |
| #413 | open | E-04 HUD confidence threshold and OCR fallback. |
| #414 | open | E-05 `creeper_nearby` synthetic event extension end-to-end. |

### 6.7 Block F - Minecraft profile

| Issue | State | Work item |
|---|---|---|
| #415 | open | F-01 ADR: default detection model, YOLOv10n vs RT-DETR-s. |
| #416 | open | F-02 `minecraft.java.toml` matches, detection, HUD, keymap, events. |
| #417 | open | F-03 heart HUD templates. |
| #418 | open | F-04 hunger HUD templates. |
| #419 | open | F-05 XP HUD region. |
| #420 | open | F-06 ADR: `aim_track` EMA smoothing alpha. |
| #421 | open | F-07 ADR: hardware-backend coalescing for small moves. |
| #422 | open | F-08 Minecraft 5-minute hands-off demo. |

### 6.8 Block G - supported-use gates

| Issue | State | Work item |
|---|---|---|
| #423 | open | G-01 `Profile.use_scope` field and schema version bump. |
| #424 | open | G-02 scope-aware action gating. |
| #425 | open | G-03 re-evaluation on foreground change and scope transition. |
| #426 | open | G-04 hardware-HID explicit enablement and first-use prompt. |
| #427 | open | G-05 `agreement.json` schema and Windows ACL. |
| #428 | open | G-06 unknown-scope refusal matrix. |
| #429 | open | G-07 reflex action-permission suppression. |

### 6.9 Block H - performance, fuzz, and manual evidence plan

| Issue | State | Work item |
|---|---|---|
| #430 | open | H-01 `action_hardware_press` p99 <= 5 ms. |
| #431 | open | H-02 `hid_combo_timing` step interval within 0.5 ms. |
| #432 | open | H-03 `hid_high_volume`, 10k mouse moves no drops. |
| #433 | open | H-04 HID protocol parser fuzz target as supporting evidence. |
| #434 | open | H-05 one-hour hardware reconnect soak, unplug/replug x100. |
| #435 | open | H-06 M4 manual happy-path and edge-case test plan. |
| #436 | open | H-07 HID error codes in `synapse-core::error_codes`. |
| #437 | open | H-08 PRD 09 stale-claim sweep. |

### 6.10 Block I - CLI, release assets, hardware docs

| Issue | State | Work item |
|---|---|---|
| #438 | open | I-01 `hid identify` subcommand. |
| #439 | open | I-02 `hid flash` subcommand. |
| #440 | open | I-03 bundled `pico-hid-x.y.z.uf2`. |
| #441 | open | I-04 hardware procurement docs. |
| #442 | open | I-05 hardware troubleshooting docs. |

### 6.11 Block J - release and docs

| Issue | State | Work item |
|---|---|---|
| #443 | open | J-01 M4 changelog release notes. |
| #444 | open | J-02 rewrite this M4 implementation plan. |
| #445 | open | J-03 PRD 05 tool surface patch. |
| #446 | open | J-04 PRD 06 data schema patch. |
| #447 | open | J-05 M4 tools-list snapshot, 33 tools sorted. |
| #448 | open | J-06 M4 default-resolution values. |
| #449 | open | J-07 tag `v0.1.0-m4` and publish release assets. |
| #450 | open | J-08 firmware build prereqs and Pico flash procedure in README. |

---

## 7. Manual FSV Contract

Manual FSV is not optional. It is the only shipping gate.

For every changed behavior:

1. Define the Source of Truth: file bytes, DB CF/key, queue, device list, COM
   port, registry Enum key, USB volume, HID report observer, process/window
   state, log line, or other physical state.
2. Read the SoT before the trigger.
3. Trigger the real runtime surface manually with synthetic input whose expected
   output is known. Use MCP tools, CLI commands, daemon endpoints, firmware,
   OS controls, or Synapse computer-control surfaces where those are the real
   path.
4. Read the SoT again with a separate operation.
5. Record before/after state for the happy path and at least three edge cases.
6. If the output should be persisted, enumerated, displayed, or emitted, inspect
   the persisted/enumerated/displayed/emitted artifact directly.
7. If any error appears, stop, identify root cause from first principles, fix
   the cause, update supporting tests if needed, then repeat manual FSV.

Strict prohibitions:

- Do not call a script, test, fuzz target, benchmark, CI run, or GitHub Action
  "FSV".
- Do not add `*_fsv` tests, FSV harnesses, or FSV scripts.
- Do not use a fake runtime path when Synapse exposes the real MCP, CLI,
  daemon, firmware, or OS path.
- Do not close a hardware-bound issue without physical host/device SoT readback.
- Do not mark missing hardware/tooling/model state as blocked for absence alone.
- Do not ask the operator to download, install, or connect something while a
  reversible local acquisition/setup path remains available through Synapse or
  host workflows.
- Do not leave a Pico, cable, COM port, driver, Rust target, UF2, model, asset,
  profile, installer, or app state "missing" when a reversible local path exists
  to acquire, connect, generate, flash, launch, or inspect it from this host.

Supporting evidence allowed:

- `cargo test`, `cargo clippy`, `cargo fmt`, `cargo check`, fuzz targets,
  benches, and doc checks, as long as they are labeled supporting evidence only.
- Manual `gh`, filesystem, registry, PnP, volume, log, DB, and UI reads that the
  agent inspects directly.

---

## 8. Happy Path And Edge-Case Tables

Issue #435 owns the final detailed M4 manual test matrix. The rows below are
the minimum phase contract each implementation issue should align with.

### 8.1 Firmware and physical USB

| Case | Known trigger | Expected outcome | SoT read before/after |
|---|---|---|---|
| Happy | Flash bundled UF2 to Pico. | Device reboots as Synapse Pico HID + CDC ACM. | `Get-PnpDevice`, `Win32_SerialPort`, USB Enum registry, `RPI-RP2` volume disappearance/reappearance. |
| Edge 1 | Invalid CRC frame. | Firmware NAKs, increments `crc_errors`, does not emit HID report. | Telemetry before/after plus external HID observer. |
| Edge 2 | No host command for >1000 ms while key held. | Watchdog fires, all inputs released. | Telemetry `watchdog_fires` and external key/button state. |
| Edge 3 | Unsupported command ID. | Firmware NAKs with unknown-command reason, keeps link alive. | Telemetry and next valid PING/PONG readback. |
| Edge 4 | Bootloader flash flow. | `RPI-RP2` appears, UF2 copied, firmware re-enumerates. | Volume list, file copy target, post-flash `IDENTIFY`. |

### 8.2 Host driver

| Case | Known trigger | Expected outcome | SoT read before/after |
|---|---|---|---|
| Happy | `hid identify --port COMx` on Synapse Pico. | Identity fields match core constants and firmware build. | CLI output plus independent COM/PnP read. |
| Edge 1 | `--port COM_DOES_NOT_EXIST`. | `HID_PORT_NOT_FOUND`. | Port list before/after and CLI structured error. |
| Edge 2 | Firmware major mismatch fixture or older firmware. | `HID_FIRMWARE_VERSION_MISMATCH`. | Identify response payload and surfaced error code. |
| Edge 3 | Unplug during send. | Next hardware action fails fast with `ACTION_HID_PORT_DISCONNECTED`. | PnP removal, driver snapshot, action error, reconnect state. |
| Edge 4 | Replug. | Auto-resume within 1 second. | PnP add, identify success, action success after reconnect. |

### 8.3 Action hardware backend

| Case | Known trigger | Expected outcome | SoT read before/after |
|---|---|---|---|
| Happy | `act_press` with backend `hardware` and key `w`. | Real keyboard HID down/up observed. | External low-level hook/UI target state plus action telemetry. |
| Edge 1 | Hardware backend requested with no enablement. | Refuse with explicit backend-unavailable/disconnected code. | Startup config and MCP error data. |
| Edge 2 | Absolute mouse move requested on hardware. | Documented relative split/fallback or structured rejection. | Action plan output and actual pointer/HID readback. |
| Edge 3 | `release_all` while key/button/pad held. | Firmware neutralizes all reports. | HID observer and held-state snapshot before/after. |
| Edge 4 | Queue/backpressure saturation. | `ACTION_QUEUE_FULL`, no hidden queue growth. | Actor queue metric/snapshot and error response. |

### 8.4 MCP `act_combo`, `act_run_shell`, and `act_launch`

| Case | Known trigger | Expected outcome | SoT read before/after |
|---|---|---|---|
| Happy combo | Three-step combo at 0/50/100 ms. | Audit shows one-shot combo fired in order. | Reflex audit DB/log and target UI/HID state. |
| Happy shell | Allowlisted `cmd /c echo synapse-m4-shell-ok`. | Exit 0 and capped stdout exact string. | Process exit, stdout bytes, log entry. |
| Happy launch | Allowlisted Notepad launch with title wait. | PID/HWND returned and window exists. | Process table and UIA/window enumeration. |
| Edge 1 | Shell command outside allowlist. | `SAFETY_SHELL_DENIED_BY_POLICY`. | Startup allowlist and MCP error data. |
| Edge 2 | Launch target outside allowlist. | `SAFETY_LAUNCH_DENIED_BY_POLICY`. | Startup allowlist and MCP error data. |
| Edge 3 | Broad pattern `.*` at startup. | Startup refuses configuration. | CLI stderr/exit and no server listening. |
| Edge 4 | Combo empty or non-monotonic steps. | `TOOL_PARAMS_INVALID` or schema-specific error. | MCP error and no audit/action side effect. |

### 8.5 Minecraft profile and supported-use

| Case | Known trigger | Expected outcome | SoT read before/after |
|---|---|---|---|
| Happy | Minecraft Java local world foreground. | Profile resolves, HUD fields populate, actions allowed. | Profile runtime state, observation JSON, game UI. |
| Edge 1 | Unknown app/profile active. | Observation allowed, write/action denied. | Active profile state and `SAFETY_PROFILE_ACTION_DENIED`. |
| Edge 2 | HUD frame with zero hearts/empty hunger. | Extractor returns expected boundary values. | Captured frame asset and observation fields. |
| Edge 3 | Entity event `creeper` bbox width 100. | `creeper-imminent` emitted. | Event bus/log/audit readback. |
| Edge 4 | Foreground changes from Minecraft to unknown app. | Scope transition suppresses reflex/action emission. | Foreground state, profile state, suppression log/error. |

---

## 9. Synthetic Fixtures

Synthetic fixtures must have known inputs and expected outputs before they are
used. They support manual verification; they do not replace real runtime FSV.

| Fixture | Known input | Expected output |
|---|---|---|
| Protocol PING | Host frame `MAGIC=0x5A`, nonce `0x12345678`, valid CRC. | Firmware `ACK` and `PONG` with same nonce. |
| Protocol CRC error | Same frame with one CRC bit flipped. | `NAK_CRC_INVALID`, telemetry `crc_errors + 1`, no HID report. |
| Identify payload | `fw_major=EXPECTED_FW_MAJOR`, VID/PID `0x2E8A:0x1F50`. | Parsed identity accepted. |
| Version mismatch | `fw_major=EXPECTED_FW_MAJOR + 1`. | `HID_FIRMWARE_VERSION_MISMATCH`. |
| Combo input | steps at `0`, `50`, `100` ms. | Three ordered audit/action records. |
| Shell allowlist | `cmd.exe /c echo synapse-m4-shell-ok`. | Exit 0, stdout exact text. |
| Shell denied | `powershell.exe -NoProfile -Command Get-Process` without allowlist. | `SAFETY_SHELL_DENIED_BY_POLICY`. |
| Launch allowlist | `notepad.exe`, title regex `.*Notepad.*`. | PID/HWND plus matching UI window. |
| Minecraft HUD image | Hand-made frame with 8.5 hearts and 6.5 hunger. | `hp_hearts=17`, `hunger=13` when expressed in half-units. |
| Creeper event | `class=creeper`, `bbox.w=100`. | `creeper-imminent` event. |
| Unknown profile | active profile `use_scope=unknown`. | action/write tools refuse with `SAFETY_PROFILE_ACTION_DENIED`. |
| Hardware absence | no matching VID/PID COM device. | `HID_PORT_NOT_FOUND` or hardware disabled code, not a silent fallback. |

---

## 10. Acceptance Gates

M4 blocks M5 until all gates are satisfied and issue evidence is posted:

1. Minecraft primary 5-minute hands-off demo passes on the configured host.
2. Hardware HID demo passes with `--hardware-hid auto` and a real Synapse Pico.
3. Firmware enumerates as boot mouse, boot keyboard, HID gamepad, and CDC ACM.
4. `hid identify` verifies firmware identity and version.
5. `hid flash` reflashes a Pico and re-verifies identity.
6. Hardware watchdog releases all held inputs within 1 second of host silence.
7. Reconnect loop survives unplug/replug and resumes within 1 second.
8. `action_hardware_press` p99 <= 5 ms on the configured host.
9. `hid_combo_timing` step interval is within 0.5 ms of scheduled.
10. `hid_high_volume` sends 10k mouse moves with no drops.
11. All M4 error codes are reachable through real trigger paths.
12. Hardware HID is refused without explicit enablement and works with it.
13. `act_run_shell` and `act_launch` require allowlists and reject broad `.*`.
14. `use_scope=unknown` denies every action/write surface.
15. The live MCP tool list is 33 sorted tools after M4 additions.
16. PRD docs, README, hardware docs, changelog, and this plan are internally
    consistent.
17. Supporting local checks are green, including `scripts/check_docs.ps1`.
18. No GitHub Actions/CI was used as a gate, and every commit contains
    `[skip ci]`.

---

## 11. Risks

| Risk | Mitigation |
|---|---|
| Pico hardware is absent on the host. | Treat as setup/acquisition work. Complete driver/toolchain checks and procurement research, then ask only for narrow purchase/credential approval if needed. |
| Firmware bugs are hard to diagnose. | Keep loopback build, serial telemetry, LED patterns, and host-side parser checks; verify real hardware state manually before closure. |
| Existing docs have stale "stub/absent" claims. | Track and fix in #437/#445/#446/#450; this file records current source state. |
| Gamepad compatibility differs by game. | Hardware pad is standard HID/DirectInput; use ViGEm for XInput-only paths per ADR #372. |
| Hardware HID latency under load. | Pipeline to 16 outstanding frames, measure p99 on host, and apply coalescing decision from #421. |
| Minecraft HUD varies by resource pack, scale, biome, and lighting. | Use anchored regions, confidence thresholds, asset fixtures, and documented fallback OCR where needed. |
| Supported-use gates regress under foreground changes. | Re-evaluate profile scope on foreground change and manually verify unknown-scope suppression. |
| Shell/launch tools expand local agency. | Require narrow allowlists, reject broad regex, cap output/time, and log every execution. |
| FSV language drifts back toward scripts. | Keep #351/AGENTS.md binding; supporting checks must be named supporting checks only. |

---

## 12. Out Of Scope For M4

- Multiple game profiles beyond `minecraft.java`.
- VLM `describe` and Florence-2 model packaging.
- Debug overlay.
- Installer/MSI/tray wizard.
- Public marketplace/profile signing.
- Kernel drivers, DLL injection, process-memory tools, graphics injection, or
  third-party device identity spoofing.
- XInput emulation in firmware; use ViGEm for XInput.
- Cross-platform hardware HID validation outside the configured host.
- CI/GitHub Actions as acceptance evidence.

---

## 13. Definition Of Done

M4 is done only when:

1. Every M4 issue in section 6 is closed or explicitly superseded by a linked
   issue with operator-approved scope.
2. The primary Minecraft demo and hardware HID demo pass on the configured
   Windows host.
3. Manual FSV evidence exists in issue comments for each shipped behavior:
   source of truth, before read, trigger, after read, happy path, and at least
   three edge cases.
4. Physical hardware behavior is verified against direct host/device SoT:
   PnP/COM/registry/volume state, firmware identity, HID observations, and
   telemetry.
5. Missing prerequisites were made real where reversible, with only narrow
   operator approvals requested for hard-to-reverse external actions.
6. Supporting local checks are green and clearly labeled as supporting evidence.
7. Docs and PRD refs resolve, including README firmware instructions,
   hardware docs, PRD 05/06/09 updates, changelog, and this file.
8. `scripts/check_docs.ps1` is green as supporting doc-link evidence.
9. `CHANGELOG.md` contains the M4 release entry.
10. Release assets include the bundled `pico-hid-x.y.z.uf2`.
11. `v0.1.0-m4` is tagged and pushed only after all manual gates are complete.
12. All commits include `[skip ci]`, and no GitHub Actions/CI status is used as
    a shipping gate.

Open next after M4 closure: [06_m5_production_polish.md](06_m5_production_polish.md).
