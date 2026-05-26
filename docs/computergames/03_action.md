# 03 — Action Subsystem

## 1. The hands

`synapse-action` is the only crate that emits input to the OS or to a virtual / hardware device. Contract:

> **Anything the agent or the reflex runtime decides to do flows through one mpsc actor that serializes by device and emits at the requested back-end. Nothing else touches the input APIs.**

Serialization is non-negotiable. Prevents stuck modifiers, double-clicks merging, and combo ordering bugs.

---

## 2. Back-ends

Three back-ends ship at v1. Per call, the caller (or active profile) picks one.

| Back-end | Path | Use |
|---|---|---|
| `software` | Win32 `SendInput` via `enigo` crate (or direct `windows-rs` if `enigo` is limiting) | Default. Cheapest. Works for most apps and many single-player games. |
| `vigem` | Virtual Xbox 360 / DualShock 4 controller via `vigem-client` crate | Games that require a gamepad (analog movement, controller-only menus). Many games accept ViGEm even when they reject software input. |
| `hardware` | Serial to RP2040 HID gateway over USB-CDC | Games where ViGEm is detected. Single-player only. See `09_hardware_hid_gateway.md`. |

Selection rules:

1. If caller explicitly names a back-end, use it.
2. Else, if active profile names a default back-end, use it.
3. Else, use `software`.

ViGEm requires ViGEmBus driver installed (one-time, signed). Hardware requires a flashed RP2040 board and `--hardware-hid <port|auto>` argument or `SYNAPSE_HARDWARE_HID` env.

---

## 3. The action types

All actions emit through one `Action` enum (full schema in `06_data_schemas.md`):

```rust
pub enum Action {
    // Keyboard
    KeyPress { key: Key, hold: Duration, backend: Backend },
    KeyDown { key: Key, backend: Backend },
    KeyUp { key: Key, backend: Backend },
    KeyChord { keys: Vec<Key>, hold: Duration, backend: Backend },
    TypeText { text: String, dynamics: KeystrokeDynamics, backend: Backend },

    // Mouse
    MouseMove { to: MouseTarget, curve: AimCurve, duration: Duration, backend: Backend },
    MouseMoveRelative { dx: f32, dy: f32, backend: Backend },
    MouseButton { button: MouseButton, action: ButtonAction, hold: Duration, backend: Backend },
    MouseDrag { from: Point, to: Point, button: MouseButton, curve: AimCurve, duration: Duration, backend: Backend },
    MouseScroll { dy: i32, dx: i32, at: Option<Point>, backend: Backend },

    // Controller
    PadButton { pad: PadId, button: PadButton, action: ButtonAction, hold: Duration },
    PadStick { pad: PadId, stick: Stick, x: f32, y: f32 },      // -1.0 .. 1.0
    PadTrigger { pad: PadId, trigger: Trigger, value: f32 },    // 0.0 .. 1.0
    PadReport { pad: PadId, report: GamepadReport },            // full report

    // High-level intents (compiled internally)
    AimAt { target: AimTarget, style: AimStyle, deadline: Duration, backend: Backend },
    Combo { steps: Vec<ComboStep>, backend: Backend },

    // Lifecycle / safety
    ReleaseAll,
}
```

`KeyChord { ctrl+s }` expresses a hotkey. `Combo` is frame-accurate sequences (fighting-game motions: `↓ → A` within 3 frames).

---

## 4. The serialization actor

```rust
pub struct ActionEmitter {
    rx: mpsc::Receiver<(Action, Sender<Result<()>>)>,
    software: SoftwareBackend,
    vigem: Option<VigemBackend>,
    hardware: Option<HidGateway>,
    held_keys: BitSet,            // for ReleaseAll safety
    held_buttons: BitSet,
    pad_state: HashMap<PadId, GamepadReport>,
}

impl ActionEmitter {
    pub async fn run(mut self, cancel: CancellationToken) {
        loop {
            tokio::select! {
                Some((action, ack)) = self.rx.recv() => {
                    let result = self.execute(action).await;
                    let _ = ack.send(result);
                },
                _ = cancel.cancelled() => {
                    self.release_all();
                    return;
                }
            }
        }
    }
}
```

Public API:

```rust
pub struct ActionHandle {
    tx: mpsc::Sender<(Action, Sender<Result<()>>)>,
}
impl ActionHandle {
    pub async fn execute(&self, action: Action) -> Result<()>;
    pub fn try_execute(&self, action: Action) -> Result<()>;  // for hot paths from reflex runtime; uses try_send
}
```

Bounded channel capacity is 256 actions. Saturation returns `ACTION_QUEUE_FULL` — a sign of agent runaway or reflex misconfiguration.

---

## 5. Software back-end (`SendInput`)

Wraps `enigo` 0.6+ with overrides:

- **Absolute mouse moves are sent as relative deltas in steps,** following an `AimCurve`. Single absolute jump reserved for `MouseMove { curve: Instant, .. }`.
- **`SendInput` is called in batches** when emitting curve steps (e.g., 50 deltas in one `SendInput([..50])` call). Per-call overhead amortizes ~2 µs to ~0.04 µs per delta.
- **Modifier state tracked locally.** `held_keys` ensures `ReleaseAll` releases everything we pressed, even on panic.
- **Unicode text** uses `KEYEVENTF_UNICODE`. Falls back to per-char scan-code when an active game ignores Unicode input (game profile flag).
- **Raw scan codes** can be requested for games reading keyboard via raw input (most FPS games). Profile flag `keyboard.use_scancodes = true`.

Latency p99 (idle Windows 11, RTX 3060, foreground app responsive):

| Action | p99 |
|---|---|
| `KeyPress` (single key, 16ms hold) | ~1 ms |
| `MouseButton` (left click, 16ms hold) | ~1 ms |
| `MouseMove` (absolute, instant curve, 100px) | ~1 ms |
| `MouseMove` (cubic Bezier, 200ms, 60 steps) | 200 ms (intended) |
| `TypeText` (10 chars, default dynamics) | ~50 ms |

---

## 6. Aim curves

`AimCurve` is the parameterized cursor-movement model. Five curves ship at v1.

**Default policy (v1.0+, supersedes `OQ-004`): `Natural` is the default everywhere — productivity and games alike. `Instant` exists in the enum but is never a default; reserved for explicit caller opt-in (test harnesses asserting pixel-perfect positioning). Defaults are tuned `FAST`: total travel 30-60 ms with sub-pixel tremor + small overshoot + 1-step micro-correct. Smooth and natural at productivity speeds.**

| Curve | Shape | Default? |
|---|---|---|
| `Instant` | Single jump to target | Never default; opt-in only via `curve: "instant"` |
| `Linear` | Constant velocity | Only when caller requests explicit lerp |
| `EaseInOut` | Smoothstep velocity | Not default at v1.0+; available as opt-in |
| `Bezier { p1, p2 }` | Cubic Bezier with two control points | Building block for `Natural`; not a top-level default |
| `Natural { tremor, micro_correct }` | Bezier with overshoot + micro-correction + sub-pixel tremor | **Default for all profiles, all action backends, all aim styles** |

Each curve emits `N` steps over the requested `duration`. Default `N = max(8, duration_ms / 4)`. Step spacing:

- `Linear`: uniform
- `EaseInOut`: smoothstep `t' = 3t² - 2t³`
- `Bezier`: cubic Bezier sampling
- `Natural`: Bezier + Gaussian sub-pixel jitter per step + final 1-3 step micro-correction sequence

### Why `Natural` matters

Straight-line cursor jumps are visually jarring and produce brittle recordings. `Natural` gives every coordinate action a smooth path with bounded timing:

- Inter-arrival timing variance
- Acceleration/deceleration instead of constant velocity
- Small overshoot and micro-correction for large moves
- Sub-pixel tremor to avoid perfectly quantized paths

Parameters are profile-tunable; the curve is deterministic given the same seed (seed exposed for replay determinism).

### Curve parameters

```rust
pub struct AimNaturalParams {
    pub control_point_jitter: f32,    // 0..1, stddev as fraction of distance
    pub tremor_stddev_px: f32,        // sub-pixel Gaussian, e.g. 0.3
    pub overshoot_prob: f32,          // 0..1, e.g. 0.4
    pub overshoot_factor_range: (f32, f32),  // e.g. (1.05, 1.20)
    pub micro_correct_steps: u8,      // e.g. 2-3
    pub timing_stddev_ms: f32,        // per-step jitter
}
```

### `AimNaturalParams::FAST` — the default preset

```rust
AimNaturalParams::FAST = AimNaturalParams {
    control_point_jitter: 0.08,            // 8% of distance — subtle path curvature
    tremor_stddev_px: 0.2,                 // sub-pixel; invisible at 1080p
    overshoot_prob: 0.25,                  // overshoots ~1 in 4 motions
    overshoot_factor_range: (1.02, 1.06),  // 2-6% past target; small
    micro_correct_steps: 1,                // single-step settle
    timing_stddev_ms: 1.5,                 // tight per-step jitter
    seed: None,                            // non-deterministic; tests pin seed
}
```

Travel times under `FAST`:

| Aim style | Total ms (default) |
|---|---|
| `Snap` (button click in UI) | 50 ms |
| `Flick` (rapid target acquisition) | 35 ms |
| `Natural` (explicit slower mode for demo/recording) | 100-200 ms |
| `Track` (continuous via reflex; per-tick) | n/a (1 ms ticks, ≤ 5 px/tick) |

Profiles can override `FAST` with their own params (e.g., a sim-racing profile may want longer travel for analog feel). Agents rarely touch these.

---

## 7. Keystroke dynamics

Typing has the same pacing problem. `KeystrokeDynamics`:

```rust
pub enum KeystrokeDynamics {
    Burst,                       // Send all chars in one SendInput
    Linear { ms_per_char: u32 }, // Equal spacing
    Natural { mean_iki_ms: f32, stddev_ms: f32, bigram_bias: bool },
}
```

`Natural` samples inter-keystroke interval (IKI) from Gaussian; if `bigram_bias`, common bigrams ("th", "he", "in") get reduced IKI for smoother text entry.

**Default policy (v1.0+): `Natural` is the default everywhere — productivity, games, chat. `Burst` exists in the enum but is never a default; reserved for explicit caller opt-in (e.g., pasting machine-generated tokens where pacing is irrelevant).**

### `KeystrokeDynamics::Natural::FAST` — the default preset

```rust
KeystrokeDynamics::Natural {
    mean_iki_ms: 32.0,      // ~190 WPM equivalent — faster than typical human, smooth
    stddev_ms: 10.0,        // realistic variance
    bigram_bias: true,      // common bigrams ~25% faster than mean
}
```

10-char string under `FAST`: ~320 ms total (vs ~30 ms `Burst`). Looks human; well under perceptible wait for productivity. Long strings (≥ 200 chars) auto-bias toward bigram-faster regions to keep total time bounded.

---

## 8. ViGEm back-end

Wraps `vigem-client` 0.1+.

```rust
pub struct VigemBackend {
    client: vigem_client::Client,
    pads: HashMap<PadId, VigemPad>,
}
enum VigemPad {
    X360(vigem_client::Xbox360Wired),
    Ds4(vigem_client::DualShock4Wired),
}
```

- One `Client` per process (singleton). Connection opened lazily on first `PadButton` / `PadStick` / `PadTrigger` / `PadReport`.
- Pads plugged in on first reference; unplugged on shutdown (RAII; also via panic hook).
- `wait_for_ready` called on plug-in to avoid `TargetNotReady` errors.
- `GamepadReport` updated by accumulating partial commands (a `PadButton(A, down)` flips a bit in cached report and re-sends).

### Pad state model

```rust
pub struct GamepadReport {
    pub buttons: PadButtons,           // bitflags
    pub thumb_lx: i16,                 // -32768..32767
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
    pub left_trigger: u8,              // 0..255
    pub right_trigger: u8,
}
```

Compatible with both X360 and DS4 reports. Back-end translates fields per pad type.

### Pad analog handling

`PadStick { x, y }` accepts -1.0..1.0; multiplied by 32767 internally. `PadTrigger { value }` accepts 0.0..1.0; multiplied by 255.

Smoothing: by default, stick deltas >0.5 in 16ms snap immediately (game-driven smoothing handles overshoot). For racing/sim profiles, `AnalogCurve::Smooth { tau_ms }` interpolates stick movement over time.

---

## 9. Hardware HID back-end

When `--hardware-hid <port|auto>` is set and `synapse-hid-host` connects and completes `IDENTIFY`, the hardware back-end routes to `HardwareBackend`. Without explicit hardware HID enablement, `Backend::Hardware` fails closed through `HardwareUnavailableBackend` with `ACTION_BACKEND_UNAVAILABLE`; it never silently downgrades to software or ViGEm.

The live hardware route talks to an RP2040 board running our firmware (`firmware/pico-hid/`) through the serial-protocol driver in `synapse-hid-host`.

The board enumerates as generic HID composite device (mouse + keyboard + gamepad). PC sees a real USB peripheral. No `SendInput`, no virtual driver, no signal interception possible.

Routing:

```rust
match action {
    Action::MouseMoveRelative { dx, dy, backend: Backend::Hardware } => {
        hid_gateway.send_mouse_delta(dx, dy)?;
    }
    Action::KeyPress { key, hold, backend: Backend::Hardware } => {
        hid_gateway.send_key(hid_code_for(key), hold)?;
    }
    /* ... */
}
```

Protocol, latency, and firmware design: `09_hardware_hid_gateway.md`. Host driver: `synapse-hid-host`.

---

## 10. High-level intents

`AimAt` and `Combo` are NOT transmitted to OS directly — compiled at the actor into primitive action sequences.

### AimAt compilation

```rust
Action::AimAt { target: ScreenPoint(820, 340), style: Snap, deadline: 60ms, backend: software }
↓
[
  MouseMove { to: (820, 340), curve: EaseInOut, duration: 60ms, backend: software },
]

Action::AimAt { target: EntityTrack(track_id), style: Track, deadline: 0ms, backend: software }
↓
  registers an aim-track reflex; emits MouseMoveRelative on each frame following the track
```

`AimStyle`:

- `Snap` — fast, EaseInOut, ~50ms default
- `Flick` — very fast Bezier, ~30ms default, with overshoot
- `Natural` — Bezier with all human-modeling params, 100-300ms
- `Track` — registers a reflex; not a one-shot

### Combo compilation

```rust
Action::Combo {
    steps: vec![
        ComboStep { input: Down(↓), at_ms: 0 },
        ComboStep { input: Down(→), at_ms: 16 },
        ComboStep { input: Press(A), at_ms: 33 },
    ],
    backend: hardware,
}
↓
schedules each step on the reflex runtime's tick wheel at the exact ms offset
```

Combo execution runs on reflex runtime thread for frame-accurate timing.

---

## 11. The release-all safety net

Three layers ensure no stuck inputs:

1. **Per-action timeout.** `KeyDown` without paired `KeyUp` within `held_key_max_duration_ms` (default 30s) emits an automatic `KeyUp` and logs `STUCK_KEY_AUTO_RELEASED`.
2. **Shutdown handler.** `ReleaseAll` sent on SIGINT / SIGTERM and on tokio cancellation token's cancellation.
3. **Panic hook.** Process-wide panic hook (`std::panic::set_hook`) calls static `RELEASE_ALL_HANDLE: OnceCell<ActionHandle>` to fire `ReleaseAll` even on unhandled panic before the process dies.

`ReleaseAll` does:

- All tracked held keys → `KeyUp` via active back-end
- All tracked held mouse buttons → up
- All ViGEm pads → neutral report (no buttons, sticks centered, triggers 0)
- All hardware HID inputs → neutral

Runs in ≤ 10 ms.

---

## 12. Authorization layer

Not every action is allowed by default. MCP handler applies:

| Action class | Default | Override |
|---|---|---|
| Mouse / keyboard / pad | allowed | — |
| Hardware HID | requires `--hardware-hid <port|auto>` flag/env and successful HID connection | per-call `backend: hardware` |
| Launch process | gated behind `--allow-launch <exe>` allowlist | profile may extend |
| Run shell | gated behind `--allow-shell <pattern>` allowlist | profile may extend |
| Clipboard write of sensitive content | per-call `confirm_sensitive: true` | env var disables prompt |

See `11_security_and_safety.md` for the full permission model.

---

## 13. Click-on-element semantics

Common agent pattern: "click the Save button." Two resolution layers:

1. **A11y-targeted click.** Caller passes an `element_id` from a recent `observe()` result. Actor:
   - Re-resolves element via UIA (may have moved).
   - Calls `IUIAutomationInvokePattern::Invoke` if element supports Invoke — semantic click, no cursor movement.
   - Falls back to clicking center of element's bounding rect with chosen curve.
2. **Coordinate click.** Caller passes raw `(x, y)`. Pure pixel click.

Semantic invoke is faster and more reliable than coordinate click for productivity apps; default when `element_id` is provided.

For games (no a11y), only coordinate clicks are possible.

> **Coordinate space.** All `(x, y)` parameters across `act_aim`, `act_click`, `act_drag`, and `act_scroll` are interpreted as **physical (DPI-aware) pixels** — the units `GetCursorPos` returns from a per-monitor-DPI-aware process and the units UI Automation bboxes use. Source-of-truth readers that aren't DPI-aware (e.g. PowerShell 5.1 `[System.Windows.Forms.Cursor]::Position` by default) will report logical coords and disagree with synapse by the monitor scale factor. See `docs/dev-host-hygiene.md#coordinates` for the verifier recipe.

---

## 14. Drag, scroll, multi-click

- **Drag.** `MouseDrag { from, to, ... }` = `MouseButton(down) + MouseMove(curve) + MouseButton(up)`. Curve drives in-flight motion.
- **Scroll.** `MouseScroll { dy, dx }` uses `SendInput` with `MOUSEEVENTF_WHEEL` / `MOUSEEVENTF_HWHEEL`. Optional `at` point first moves cursor.
- **Double-click.** Two `MouseButton` actions within `GetDoubleClickTime()` (default 500ms). Actor injects the gap; agent doesn't manage it.
- **Triple-click.** Three within the same time window. For text-selection ops on plain edit fields.

---

## 15. Input rate limiting

Per back-end caps prevent OS or virtual device overwhelm:

| Back-end | Per-second cap |
|---|---|
| Software | 5000 events/s |
| ViGEm | 1000 reports/s (Xbox 360 USB poll rate ~1ms anyway) |
| Hardware HID | depends on USB poll rate; default 1000 events/s |

Saturation returns `ACTION_RATE_LIMITED` and re-queues with a small backoff.

---

## 16. Determinism and replay

Every action through the actor is persisted to `CF_EVENTS` with:

- Originating call site (MCP tool / reflex id)
- Actual back-end used
- Exact parameters
- Success/failure result
- Completion timestamp

`synapse-mcp replay <session_id>` replays actions deterministically (same seeds, same curves) against the same back-end. Used for debug and regression testing.

---

## 17. Error codes

```rust
pub const ACTION_QUEUE_FULL: &str = "ACTION_QUEUE_FULL";
pub const ACTION_RATE_LIMITED: &str = "ACTION_RATE_LIMITED";
pub const ACTION_BACKEND_UNAVAILABLE: &str = "ACTION_BACKEND_UNAVAILABLE";
pub const ACTION_TARGET_INVALID: &str = "ACTION_TARGET_INVALID";
pub const ACTION_HOLD_EXCEEDED_MAX: &str = "ACTION_HOLD_EXCEEDED_MAX";
pub const ACTION_HID_PORT_DISCONNECTED: &str = "ACTION_HID_PORT_DISCONNECTED";
pub const ACTION_VIGEM_NOT_INSTALLED: &str = "ACTION_VIGEM_NOT_INSTALLED";
pub const ACTION_ELEMENT_NOT_RESOLVED: &str = "ACTION_ELEMENT_NOT_RESOLVED";
pub const STUCK_KEY_AUTO_RELEASED: &str = "STUCK_KEY_AUTO_RELEASED";
pub const SAFETY_RELEASE_ALL_FIRED: &str = "SAFETY_RELEASE_ALL_FIRED";
```

---

## 18. Out of scope for this doc

- Reflex bindings (aim_track, on_event) → `04_reflex_runtime.md`
- MCP tool surface wrapping these actions → `05_mcp_tool_surface.md`
- Hardware HID firmware design → `09_hardware_hid_gateway.md`
- Supported-use policy and permission gates → `08`
