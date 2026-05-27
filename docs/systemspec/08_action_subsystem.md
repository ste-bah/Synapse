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

The action subsystem is an actor-style emitter with an Tokio mpsc producer (`ActionHandle`) and a backend-dispatching consumer (`ActionEmitter`). Backends implement `ActionBackend::execute(&Action, &mut EmitState)`; the concrete one is chosen by `resolve_backend_with_policy` based on the action's `backend` field plus the active `BackendResolutionPolicy` for `Backend::Auto`.

### 1.1 Public re-exports (`lib.rs`)

| Symbol | Source |
|---|---|
| `ActionBackend`, `BackendResolutionPolicy`, `ResolvedBackend`, `resolve_backend`, `resolve_backend_with_policy` | `backend::mod` |
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

- A global union `BitSet` of held keys plus `held_key_backends: HashMap<usize, BTreeSet<ResolvedBackend>>`, which records which concrete backend owns each held key. Reflex `hold_*` flows must enqueue through `ActionHandle`, not mutate this state directly.
- A global union held mouse-button set plus `held_button_backends: HashMap<usize, BTreeSet<ResolvedBackend>>`.
- Per-pad `GamepadReport` cache (last-emitted neutralizable state).
- Per-key auto-release timers keyed by `(Key, ResolvedBackend)`.

`ActionStateSnapshot` (exposed via `ActionEmitterSnapshotHandle::snapshot().await`) is the public read of `held_keys: Vec<Key>`, `held_buttons: Vec<MouseButton>`, `pad_state: HashMap<PadId, GamepadReport>`, `held_key_timer_count: usize`, plus `held_keys_by_backend` and `held_buttons_by_backend` for backend-owned held-state readback.

### 3.2 Per-message dispatch (`emitter::dispatch`)

For each `(Action, oneshot::Sender)` pulled from the channel:

1. **Validate.** `validate_action(&action)` (re-checked here even though `ActionHandle::execute` already validates, to defend against `try_execute` callers that bypassed it).
2. **Resolve backend.** `routing.rs` reads the emitter's active `BackendResolutionPolicy` and calls `backend::resolve_backend_with_policy(action.backend(), &action, policy)`:
   - `Software` → `ResolvedBackend::Software`
   - `Vigem` → `ResolvedBackend::Vigem`
   - `Hardware` → `ResolvedBackend::Hardware`; the selected backend is `HardwareBackend` only when `synapse-mcp` was started with `--hardware-hid <port|auto>` and the HID connection/IDENTIFY succeeded. Otherwise the hardware slot is `HardwareUnavailableBackend`.
   - `Auto` with the global default policy → `ResolvedBackend::Vigem` for `Pad*` actions and `Software` for keyboard, mouse, combo, and release-all.
   - `Auto` after activating a profile with `[backends] default_backend = "hardware"` → `ResolvedBackend::Hardware` for keyboard, mouse, pad, combo, and release-all unless `keyboard_default`, `mouse_default`, or `pad_default` is explicitly set for that class.
   - The active policy and resolved table are readable at `health.subsystems.action.backend_resolution`.
3. **Rate-limit.** `rate_limits.rs` consumes one token from the per-backend `TokenBucket`:
   - `SOFTWARE_RATE_LIMIT_PER_S = 5000`
   - `VIGEM_RATE_LIMIT_PER_S = 1000`
   - Out-of-tokens → `ActionError::RateLimited { retry_after_ms }` (the only variant that carries a hint for clients)
4. **Dispatch.** Backend-specific `execute(&action, &mut EmitState)`:
   - **`SoftwareBackend`** (`backend/software/*`): SendInput-based keyboard/mouse/text, see §4
   - **`VigemBackend`** (`backend/vigem/*`): X360/DS4 controller report via `vigem-client`
   - **`RecordingBackend`** (`backend/recording/*`): appends a `RecordedInput` to an in-memory log (used in tests and via `SYNAPSE_MCP_RECORDING_BACKEND=1`)
   - **`HardwareBackend`** (`backend/hardware/*`): serializes supported key, mouse-relative, absolute-mouse-to-relative batch, pad, combo, and release commands through `synapse-hid-host::HidGateway`
   - **`HardwareUnavailableBackend`** (`backend/unavailable`): fail-closed response when hardware HID is not enabled, returning `ACTION_BACKEND_UNAVAILABLE` with `--hardware-hid <port|auto>` guidance
5. **Auto-release timers.** `emitter::keyboard` enforces `HELD_KEY_MAX_DURATION_MS` per `(Key, ResolvedBackend)` held key. After the limit, the emitter inserts a synthetic `KeyUp` for the backend that owns the hold and emits a `STUCK_KEY_AUTO_RELEASED` warn-log + event with `backend=<software|vigem|hardware>`.
6. **ReleaseAll**: reads the full `EmitState` union, aborts held-key timers, and dispatches `Action::ReleaseAll` through every backend that owns releasable state. Each backend sees only its own `held_keys`/`held_buttons` snapshot while executing, so software release cannot claim hardware-held inputs and hardware release cannot claim software-held inputs. ViGEm pad state is released through the ViGEm backend when pads are held. When a real hardware HID backend was configured at startup, the emitter also dispatches `Action::ReleaseAll` to `HardwareBackend`, which sends one firmware `RELEASE_ALL (0x40)` command and clears the hardware host mirror. The safety log records `code=SAFETY_RELEASE_ALL_FIRED`, `backend="hardware"`, `primary_backend`, `release_backends`, and `hardware_release_ok` for this path. Reflexes that observe `Action::ReleaseAll` are also expected to expire any held-state controllers.
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

- **Remaining hardware HID gaps.** The live `Backend::Hardware` path is enabled by `--hardware-hid <port|auto>` and maps Synapse keys to USB HID Keyboard/Keypad usage IDs with boot-report modifier byte handling, 6KRO limit enforcement, shifted US-layout text, and absolute mouse fallback by converting screen or resolved-element targets into batched `MOUSE_MOVE_REL` commands. Broader supported-use gates remain M4 work.
- **Modifiers on `act_click`.** The schema accepts `Vec<ClickModifier>` but emitting a non-empty list currently returns `ACTION_BACKEND_UNAVAILABLE` with the message "act_click modifiers are not wired in the M2 click schema slice".
- **Element-target aim and drag**. `act_aim` with an `Element` target returns `ACTION_BACKEND_UNAVAILABLE` ("requires the dedicated target resolution issue"); same for `Track` targets. `act_drag` supports `Element` targets via UIA bbox resolution.
