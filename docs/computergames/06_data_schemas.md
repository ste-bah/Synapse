# 06 — Data Schemas

Canonical types live in `synapse-core`. JSON serialization via `serde` (`#[serde(rename_all = "snake_case")]` everywhere). RocksDB stored records use JSON; bincode is excluded by ADR-0001 / RUSTSEC-2025-0141.

This doc is the spec; `synapse-core/src/types/` plus
`synapse-core/src/error_codes.rs` are the implementation source of truth. Drift
between them is a local check failure and release blocker.

---

## 1. Core enums and primitives

### 1.1 Backend selection

```rust
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Software,    // Win32 SendInput
    Vigem,       // Virtual Xbox/DS4 via ViGEm
    Hardware,    // RP2040 HID gateway
    Auto,        // Resolve from active session policy + action class
}
```

`Auto` preserves M2 defaults when no profile override is active: keyboard,
mouse, combo, and release-all choose `software`; pad actions choose `vigem`.
When the active profile declares `[backends] default_backend = "hardware"`,
`Auto` resolves to `hardware` for keyboard, mouse, pad, combo, and release-all
unless a class-specific `keyboard_default`, `mouse_default`, or `pad_default`
overrides that class. The active table is exposed at
`health.subsystems.action.backend_resolution`.

### 1.2 Perception mode

```rust
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerceptionMode {
    A11yOnly,
    PixelOnly,
    Hybrid,
    Auto,
}
```

### 1.3 Geometry

```rust
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct Point { pub x: i32, pub y: i32 }

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct Rect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct Size { pub w: u32, pub h: u32 }
```

All coordinates are physical pixels in the virtual desktop coordinate system unless tagged otherwise (per-window rects carry an explicit field).

### 1.4 IDs

All IDs are `String` (UUID-v7 or composite namespaced) for cross-version stability:

```rust
pub type SessionId = String;     // UUID-v7
pub type ElementId = String;     // "<hwnd>:<uia_runtime_id_hex>"
pub type EntityId = String;      // "track:<u64>"
pub type ReflexId = String;      // UUID-v7
pub type SubscriptionId = String; // UUID-v7
pub type ProfileId = String;     // "namespace.name", e.g., "minecraft.java"
```

`ElementId` is composite: window HWND plus UIA `RuntimeId` hex. Stable across snapshots within a session but NOT across sessions; a re-launched window gets a new RuntimeId.

---

## 2. Observation

Unified perception result returned by `observe()`.

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub seq: u64,
    pub at: chrono::DateTime<chrono::Utc>,
    pub mode: PerceptionMode,
    pub foreground: ForegroundContext,
    pub focused: Option<FocusedElement>,
    pub elements: Vec<AccessibleNode>,
    pub entities: Vec<DetectedEntity>,
    pub hud: HudReadings,
    pub audio: AudioContext,
    pub recent_events: Vec<EventSummary>,
    pub clipboard_summary: Option<ClipboardSummary>,
    pub fs_recent: Vec<FsEvent>,
    pub diagnostics: ObservationDiagnostics,
}
```

### 2.1 ForegroundContext

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForegroundContext {
    pub hwnd: i64,
    pub pid: u32,
    pub process_name: String,         // basename, e.g., "notepad.exe"
    pub process_path: String,         // full path
    pub window_title: String,
    pub window_bounds: Rect,
    pub monitor_index: u32,
    pub dpi_scale: f32,
    pub profile_id: Option<ProfileId>,
    pub steam_appid: Option<u32>,     // resolved from Steam if applicable
    pub is_fullscreen: bool,
    pub is_dwm_composed: bool,
}
```

### 2.2 FocusedElement

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FocusedElement {
    pub element_id: ElementId,
    pub name: String,
    pub role: String,                 // UIA ControlType, e.g., "Button"
    pub automation_id: Option<String>,
    pub bbox: Rect,
    pub enabled: bool,
    pub patterns: Vec<UiaPattern>,
    pub value: Option<String>,        // if ValuePattern supported
    pub selected_text: Option<String>,// if TextPattern supports it
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiaPattern { Invoke, Toggle, Value, Selection, ExpandCollapse, Scroll, Text, Window, Transform, RangeValue }
```

### 2.3 AccessibleNode

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessibleNode {
    pub element_id: ElementId,
    pub parent: Option<ElementId>,
    pub name: String,
    pub role: String,
    pub automation_id: Option<String>,
    pub bbox: Rect,
    pub enabled: bool,
    pub focused: bool,
    pub patterns: Vec<UiaPattern>,
    pub children_count: u32,          // not the children themselves
    pub depth: u32,
}
```

Tree is flattened — depth + parent enables reconstruction. Children are not nested to keep JSON small.

### 2.4 DetectedEntity

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectedEntity {
    pub entity_id: EntityId,
    pub track_id: u64,
    pub class_label: String,
    pub bbox: Rect,
    pub confidence: f32,
    pub first_seen_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
    pub velocity_px_per_s: Option<(f32, f32)>,
}
```

### 2.5 HudReadings

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HudReadings {
    pub by_name: std::collections::BTreeMap<String, HudReading>,
    pub errors: std::collections::BTreeMap<String, HudFieldError>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HudFieldError {
    pub code: String,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HudReading {
    pub raw_text: String,
    pub parsed: HudValue,
    pub confidence: f32,
    pub stale_ms: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HudValue {
    Number(f64),
    Text(String),
    Enum(String),
    Null,
}
```

### 2.6 AudioContext

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioContext {
    pub rms_db: f32,
    pub vad_speech_recent: bool,
    pub recent_events: Vec<AudioEvent>,
    pub direction_estimate: Option<DirectionEstimate>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioEvent {
    pub at: chrono::DateTime<chrono::Utc>,
    pub kind: String,                 // "loud_transient" | "speech_started" | ...
    pub azimuth_deg: Option<f32>,
    pub confidence: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectionEstimate { pub azimuth_deg: f32, pub confidence: f32 }
```

### 2.7 ClipboardSummary

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClipboardSummary {
    pub formats: Vec<String>,         // "text/plain", "image/png", ...
    pub text_len: Option<u32>,
    pub text_excerpt: Option<String>, // first ~120 chars, redacted if sensitive pattern matched
    pub redacted: bool,
}
```

### 2.8 FsEvent

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FsEvent {
    pub at: chrono::DateTime<chrono::Utc>,
    pub path: String,
    pub kind: FsEventKind,
    pub size_bytes: Option<u64>,
}
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FsEventKind { Created, Modified, Deleted, Renamed }
```

### 2.9 ObservationDiagnostics

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObservationDiagnostics {
    pub assembled_in_ms: f32,
    pub sensor_latency_ms: std::collections::BTreeMap<String, f32>, // bounded keys: a11y/capture/detection/ocr/audio
    pub a11y_enabled: bool,
    pub pixel_enabled: bool,
    pub audio_enabled: bool,
    pub a11y_status: SensorStatus,
    pub capture_status: SensorStatus,
    pub detection_status: SensorStatus,
    pub audio_status: SensorStatus,
    pub elements_truncated: bool,
    pub entities_truncated: bool,
    pub size_bytes: u32,
    pub size_estimate_tokens: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorStatus {
    Healthy,
    DegradedLatency { last_p99_ms: f32 },
    DegradedSensorFailed { reason_code: String },
    Disabled,
    Unavailable,
}
```

---

### 2.10 Reality baseline, deltas, and audits (target)

Issue #536 introduces the target delta-first reality contract. These schemas
are the intended shape for #537; they are not live until implemented in
`synapse-core` and exposed through `tools/list`.

```rust
pub struct SourceRef {
    pub kind: String,                 // "window" | "log" | "file" | "rocksdb" | ...
    pub path: String,
    pub offset: Option<u64>,
    pub hash: Option<String>,
    pub summary: String,
}

pub struct RedactionSummary {
    pub policy: String,
    pub raw_private_fields_omitted: bool,
    pub redacted_fields: Vec<String>,
}

pub struct RealityBaseline {
    pub epoch_id: String,
    pub baseline_seq: u64,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub profile_id: Option<ProfileId>,
    pub source_refs: Vec<SourceRef>,
    pub compact_state_hash: String,
    pub redaction: RedactionSummary,
    pub size_bytes: u32,
    pub size_estimate_tokens: u32,
}

pub struct RealityDelta {
    pub epoch_id: String,
    pub seq: u64,
    pub previous_seq: u64,
    pub at: chrono::DateTime<chrono::Utc>,
    pub source: EventSource,
    pub kind: String,
    pub path: String,
    pub before: serde_json::Value,
    pub after: serde_json::Value,
    pub confidence: f32,
    pub expected_previous_hash: Option<String>,
    pub source_refs: Vec<SourceRef>,
    pub correlations: Vec<EventRef>,
    pub redaction: RedactionSummary,
}

pub struct RealityAudit {
    pub audit_id: String,
    pub epoch_id: String,
    pub compared_seq_start: u64,
    pub compared_seq_end: u64,
    pub ran_at: chrono::DateTime<chrono::Utc>,
    pub assumption_hash: String,
    pub actual_hash: String,
    pub drift_status: RealityDriftStatus,
    pub drift_items: Vec<RealityDriftItem>,
    pub physical_source_refs: Vec<SourceRef>,
    pub rebase_required: bool,
}

pub struct RealityDriftItem {
    pub path: String,
    pub assumed: serde_json::Value,
    pub actual: serde_json::Value,
    pub severity: RealityDriftStatus,
    pub source_refs: Vec<SourceRef>,
}

#[serde(rename_all = "snake_case")]
pub enum RealityDriftStatus {
    InSync,
    MinorDrift,
    MajorDrift,
    RebaseRequired,
    SourceUnavailable,
}
```

Deltas are append-only within an epoch. If retention, source loss, invalid
cursors, or conflicting evidence makes the assumption untrustworthy, the system
returns `rebase_required` instead of guessing.

Physical storage target:

- `CF_KV/reality/baseline/v1/<profile>/<epoch>`
- `CF_KV/reality/delta/v1/<profile>/<epoch>/<seq>`
- `CF_KV/reality/audit/v1/<profile>/<audit_id>`
- `CF_KV/reality/head/v1/<profile>` for the current epoch/seq/hash pointer

Full observations remain the source for baseline and explicit audit/debug
expansion. Routine context should prefer `RealityDelta` batches because they
carry the change in reality plus source refs, not the unchanged surroundings.

---

## 3. Events

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,                     // monotonic across all sources
    pub at: chrono::DateTime<chrono::Utc>,
    pub source: EventSource,
    pub kind: String,                 // kebab-case, e.g., "entity-appeared"
    pub data: serde_json::Value,      // shape varies by kind
    pub correlations: Vec<EventRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    A11yUia,
    A11yWinEvent,
    A11yCdp,
    Perception,
    PerceptionDetection,
    PerceptionHud,
    PerceptionAudio,
    Filesystem,
    Process,
    Clipboard,
    ActionEmitter,
    Reflex,
    System,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventRef {
    pub seq: u64,
    pub relation: String,
}
```

`EventSummary` is the trimmed form in `Observation.recent_events`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventSummary {
    pub seq: u64,
    pub at: chrono::DateTime<chrono::Utc>,
    pub source: EventSource,
    pub kind: String,
    pub data_excerpt: serde_json::Value,    // capped to small size
}
```

### 3.1 Event kinds

| Kind | Source | `data` shape |
|---|---|---|
| `foreground-changed` | A11yWinEvent | `{ from_hwnd, to_hwnd, from_process, to_process }` |
| `focus-changed` | A11yWinEvent | `{ from_element_id, to_element_id }` |
| `element-appeared` | A11yUia | `{ element_id, parent, role, name }` |
| `element-disappeared` | A11yUia | `{ element_id }` |
| `value-changed` | A11yUia | `{ element_id, old, new }` |
| `name-changed` | A11yUia | `{ element_id, old, new }` |
| `selection-changed` | A11yUia | `{ container_id, selected_ids }` |
| `dom-mutation` | A11yCdp | `{ frame_id, mutation_summary }` |
| `navigation-committed` | A11yCdp | `{ frame_id, url, title }` |
| `entity-appeared` | PerceptionDetection | `{ entity_id, track_id, class_label, bbox, confidence }` |
| `entity-disappeared` | PerceptionDetection | `{ entity_id, track_id }` |
| `entity-class-changed` | PerceptionDetection | `{ entity_id, old, new }` |
| `hud-value-changed` | PerceptionHud | `{ field, old, new, confidence }` |
| `loud-transient` | PerceptionAudio | `{ azimuth_deg, rms_db, confidence }` |
| `speech-started` / `speech-ended` | PerceptionAudio | `{}` / `{ duration_ms }` |
| `file-created` / `file-changed` / `file-deleted` | Filesystem | `{ path, size_bytes? }` |
| `process-started` / `process-exited` | Process | `{ pid, name, cmdline?, exit_code? }` |
| `clipboard-changed` | Clipboard | `{ formats, text_len, redacted }` |
| `action-completed` | ActionEmitter | `{ action_kind, success, error_code?, duration_us }` |
| `reflex-registered` / `reflex-fired` / `reflex-cancelled` / `reflex-expired` | Reflex | `{ reflex_id, kind, ... }` |
| `system-shutdown` / `system-resume` | System | `{}` |

This is the v1 catalog. Additions go through ADR.

### 3.2 EventFilter

Mini-language used by `subscribe()` and `reflex_register(on_event)`.

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum EventFilter {
    All,
    None,
    Kind { kind: String },
    Source { source: EventSource },
    And { args: Vec<EventFilter> },
    Or  { args: Vec<EventFilter> },
    Not { arg: Box<EventFilter> },
    Data { path: String, predicate: DataPredicate },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DataPredicate {
    Eq { value: serde_json::Value },
    Ne { value: serde_json::Value },
    Lt { value: serde_json::Value },
    Le { value: serde_json::Value },
    Gt { value: serde_json::Value },
    Ge { value: serde_json::Value },
    Regex { pattern: String },
    InSet { values: Vec<serde_json::Value> },
    Exists,
}
```

`path` is JSON-Pointer style: `/track_id`, `/field`, `/bbox/x`. Evaluator in `synapse-core::filter`.
Validation lives on `EventFilter::validate()`: `And` and `Or` require at least one
argument, and filter trees deeper than `EVENT_FILTER_MAX_DEPTH` (`8`) are
rejected before registration/subscription.

Example: "low HP event":

```json
{
  "op": "and",
  "args": [
    {"op": "kind", "kind": "hud-value-changed"},
    {"op": "data", "path": "/field", "predicate": {"op": "eq", "value": "hp"}},
    {"op": "data", "path": "/new", "predicate": {"op": "lt", "value": 20}}
  ]
}
```

---

## 4. Action types

Full `Action` enum referenced in `03_action.md`. Each variant carries a `backend` field where applicable.

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    KeyPress { key: Key, hold: Duration, backend: Backend },
    KeyDown  { key: Key, backend: Backend },
    KeyUp    { key: Key, backend: Backend },
    KeyChord { keys: Vec<Key>, hold: Duration, backend: Backend },
    TypeText { text: String, dynamics: KeystrokeDynamics, backend: Backend },

    MouseMove { to: MouseTarget, curve: AimCurve, duration: Duration, backend: Backend },
    MouseMoveRelative { dx: f32, dy: f32, backend: Backend },
    MouseButton { button: MouseButton, action: ButtonAction, hold: Duration, backend: Backend },
    MouseDrag { from: Point, to: Point, button: MouseButton, curve: AimCurve, duration: Duration, backend: Backend },
    MouseScroll { dy: i32, dx: i32, at: Option<Point>, backend: Backend },

    PadButton { pad: PadId, button: PadButton, action: ButtonAction, hold: Duration },
    PadStick  { pad: PadId, stick: Stick, x: f32, y: f32 },
    PadTrigger{ pad: PadId, trigger: Trigger, value: f32 },
    PadReport { pad: PadId, report: GamepadReport },

    AimAt    { target: AimTarget, style: AimStyle, deadline: Duration, backend: Backend },
    Combo    { steps: Vec<ComboStep>, backend: Backend },

    ReleaseAll,
}
```

### 4.1 Sub-types

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MouseTarget {
    Screen(Point),
    Element(ElementId),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AimTarget {
    Screen(Point),
    Element(ElementId),
    Track(u64),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AimCurve {
    Instant,
    Linear,
    EaseInOut,
    Bezier { p1: (f32, f32), p2: (f32, f32) },
    Natural { params: AimNaturalParams },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AimNaturalParams {
    pub control_point_jitter: f32,
    pub tremor_stddev_px: f32,
    pub overshoot_prob: f32,
    pub overshoot_factor_range: (f32, f32),
    pub micro_correct_steps: u8,
    pub timing_stddev_ms: f32,
    pub seed: Option<u64>,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AimStyle { Snap, Flick, Natural, Track }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KeystrokeDynamics {
    Burst,
    Linear { ms_per_char: u32 },
    Natural { mean_iki_ms: f32, stddev_ms: f32, bigram_bias: bool },
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton { Left, Right, Middle, X1, X2 }

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonAction { Press, Down, Up }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Key {
    pub code: KeyCode,                // see vocab below
    pub use_scancode: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum KeyCode {
    Named(String),                    // "a", "f1", "ctrl", "enter"
    Symbol(char),                     // single char
    HidCode(u8),                      // direct HID usage code (hardware backend only)
}

pub type PadId = u8;

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PadButton { A,B,X,Y, Lb,Rb, Ls,Rs, Back,Start, Up,Down,Left,Right, Guide }

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stick { Left, Right }

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trigger { Left, Right }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GamepadReport {
    pub buttons: Vec<PadButton>,
    pub thumb_l: (f32, f32),          // -1.0..1.0
    pub thumb_r: (f32, f32),
    pub lt: f32,                      // 0.0..1.0
    pub rt: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComboStep {
    pub at_ms: u32,
    pub input: ComboInput,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComboInput {
    KeyDown { key: Key },
    KeyUp   { key: Key },
    KeyPress{ key: Key, hold_ms: u16 },
    MouseButton { button: MouseButton, action: ButtonAction },
    MouseMoveRel { dx: f32, dy: f32 },
    PadButton { pad: PadId, button: PadButton, action: ButtonAction },
    PadStick  { pad: PadId, stick: Stick, x: f32, y: f32 },
}
```

`ComboInput` is the complete M4 combo payload surface. It is intentionally
smaller than the full `Action` enum: shell, launch, storage, profile writes,
subscriptions, and nested combo tools are not valid combo inputs.

| JSON `kind` | Payload fields | Effect |
|---|---|---|
| `key_down` | `key` | press and hold one key |
| `key_up` | `key` | release one key |
| `key_press` | `key`, `hold_ms` | down, wait, up |
| `mouse_button` | `button`, `action` | mouse button press/down/up |
| `mouse_move_rel` | `dx`, `dy` | relative mouse movement |
| `pad_button` | `pad`, `button`, `action` | gamepad button press/down/up |
| `pad_stick` | `pad`, `stick`, `x`, `y` | gamepad stick position |

`ComboStep.at_ms` is relative to combo start and must be monotonic in the
validated step list. Runtime backend routing is carried by the enclosing
`Action::Combo`/`ReflexThen::Combo` backend, with M4 `act_combo` optionally
providing per-step backend hints at the MCP layer before lowering to this core
shape.

### 4.2 Key name vocabulary

Named keys (case-insensitive on input, normalized lowercase internally):

- Letters: `a` .. `z`
- Digits: `0` .. `9`
- Function: `f1` .. `f24`
- Arrows: `up`, `down`, `left`, `right`
- Modifiers: `ctrl`, `lctrl`, `rctrl`, `shift`, `lshift`, `rshift`, `alt`, `lalt`, `ralt`, `super`, `lsuper`, `rsuper`
- Whitespace / punctuation: `space`, `tab`, `enter`, `backspace`, `delete`, `escape`/`esc`, `home`, `end`, `pageup`, `pagedown`, `insert`
- Symbols: `minus`, `equals`, `lbracket`, `rbracket`, `backslash`, `semicolon`, `apostrophe`, `comma`, `period`, `slash`, `grave`/`backtick`
- Lock keys: `capslock`, `numlock`, `scrolllock`
- Numpad: `np_0`..`np_9`, `np_plus`, `np_minus`, `np_star`, `np_slash`, `np_dot`, `np_enter`
- Media: `volup`, `voldown`, `mute`, `play`, `next`, `prev`, `stop`
- Mouse pseudo-keys: `lmb`, `rmb`, `mmb`, `mbx1`, `mbx2`

Profile keymap aliases extend this set per-app (e.g., Minecraft profile maps `attack` → `lmb`, `place` → `rmb`, `inventory` → `e`).

---

## 5. Reflex types

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReflexRegistration {
    pub id: ReflexId,
    pub kind: ReflexKind,
    pub priority: u32,
    pub lifetime: ReflexLifetime,
    pub exclusive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReflexKind {
    AimTrack {
        target: AimTarget,
        axis: ReflexAimAxis,
        gain: f32,
        deadzone_px: f32,
        max_speed_px_per_ms: f32,
        curve_per_step: AimCurve,
        backend: Backend,
    },
    HoldMove {
        keys: Vec<Key>,
        backend: Backend,
        re_assert: bool,
    },
    HoldButton {
        button: ReflexButtonTarget,
        backend: Backend,
    },
    Combo {
        steps: Vec<ComboStep>,
        backend: Backend,
    },
    OnEvent {
        when: EventFilter,
        then: ReflexThen,
        debounce_ms: u32,
    },
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflexAimAxis { Xy, XOnly, YOnly }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReflexButtonTarget {
    Mouse { button: MouseButton },
    Pad { pad: PadId, button: PadButton },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReflexThen {
    Action { action: Action },
    Actions { actions: Vec<Action> },
    Combo { steps: Vec<ComboStep>, backend: Backend },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReflexLifetime {
    UntilCancelled,
    OneShot,
    Duration { ms: u32 },
    UntilEvent { filter: EventFilter },
    UntilDeadline { ms: u32 },
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflexState {
    Active,
    Paused,
    Cancelled,
    Expired,
    Disabled,
    Starved,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReflexStatus {
    pub id: ReflexId,
    pub kind_summary: String,
    pub state: ReflexState,
    pub registered_at: chrono::DateTime<chrono::Utc>,
    pub last_fired_at: Option<chrono::DateTime<chrono::Utc>>,
    pub fire_count: u64,
    pub priority: u32,
    pub lifetime: ReflexLifetime,
    pub exclusive: bool,
    pub last_error_code: Option<String>,
}
```

---

## 6. Profile schema

Stored as TOML under `profiles/<id>.toml`. Loaded into:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub id: ProfileId,
    pub label: String,
    pub version: String,                       // semver of this profile
    pub use_scope: ProfileUseScope,
    pub matches: Vec<ProfileMatch>,
    pub mode: PerceptionMode,
    pub capture: ProfileCapture,
    pub detection: ProfileDetection,
    pub ocr: ProfileOcr,
    pub hud: Vec<HudFieldSpec>,
    pub keymap: std::collections::BTreeMap<String, String>,   // alias -> key name
    pub backends: ProfileBackends,
    pub metadata: std::collections::BTreeMap<String, String>, // registry/provenance/policy hints
    pub event_extensions: Vec<EventExtension>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileMatch {
    pub exe: Option<String>,                   // basename or full path regex
    pub title_regex: Option<String>,
    pub steam_appid: Option<u32>,
    pub window_class: Option<String>,
    pub process_args: Vec<String>,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileUseScope {
    Productivity,
    SinglePlayer,
    OperatorOwnedTest,
    SanctionedResearch,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileCapture {
    pub target: ProfileCaptureTarget,
    pub min_update_interval_ms: u32,
    pub cursor_visible: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProfileCaptureTarget {
    ForegroundWindow,
    PrimaryMonitor,
    MonitorIndex { index: u32 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileDetection {
    pub model_id: Option<String>,              // None = disable detection
    pub classes_of_interest: Vec<String>,
    pub confidence_threshold: f32,
    pub max_detections: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileOcr {
    pub default_backend: OcrBackend,
    pub regions: Vec<HudRegion>,
    pub parser_config: std::collections::BTreeMap<String, String>,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrBackend { Winrt, Crnn, Auto }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HudFieldSpec {
    pub name: String,
    pub region: HudRegion,
    pub extractor: HudExtractor,
    pub parser: HudParser,
    pub confidence_threshold: f32,              // default 0.85; template fallback threshold
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HudRegion {
    Absolute { x: i32, y: i32, w: i32, h: i32 },
    FractionOfWindow { x: f32, y: f32, w: f32, h: f32 },
    AnchoredToEdge { edge: WindowEdge, x_offset: i32, y_offset: i32, w: i32, h: i32 },
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowEdge { TopLeft, TopRight, BottomLeft, BottomCenter, BottomRight, Center }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HudExtractor {
    WinrtOcr,
    Crnn { model_id: String },
    TemplateMatch { templates: Vec<String> },   // sha or asset reference
    ColorRatio { sample_points: Vec<(i32, i32)>, mapping: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HudParser {
    Number,
    BoundedInteger { min: u32, max: u32, default_on_no_text: Option<u32> },
    FractionNumerator,                          // "85/100" -> 85
    FractionDenominator,
    Regex { pattern: String, group: u32 },
    Enum { mapping: std::collections::BTreeMap<String, String> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileBackends {
    pub default: Backend,
    pub keyboard_default: Backend,
    pub mouse_default: Backend,
    pub pad_default: Backend,
}

// TOML accepts both `default` and `default_backend`; the canonical field is
// `default`. Omitted fields parse as Auto.

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventExtension {
    pub name: String,
    pub from_filter: EventFilter,
    pub emits_kind: String,
}
```

HUD text fields use the configured crop as the source of truth. On Windows the
live MCP `observe` path first accepts bounded UIA text whose element rectangle
intersects and stays close to that crop, then falls back to WinRT OCR. Very small
OCR crops are upscaled before recognition. `BoundedInteger` rejects non-integer
or out-of-range text and may declare an in-range `default_on_no_text` for
intentional empty HUD states such as Minecraft XP level `0`.

Serialized `use_scope` values and policy behavior:

| TOML/JSON value | Intended scope | Default action posture |
|---|---|---|
| `productivity` | Operator productivity apps such as editors, browsers, terminals, chat, and file managers. | Actions allowed according to normal session/tool permissions. |
| `single_player` | Local single-player games such as Minecraft Java local worlds. | Game actions allowed; hardware HID still requires explicit enablement. |
| `operator_owned_test` | Local QA fixtures, private test servers, simulators, and replay harnesses owned by the operator. | Actions allowed when the profile declares the test boundary. |
| `sanctioned_research` | University, tournament, or research rigs where automation is explicitly authorized. | Actions allowed with explicit profile metadata and operator setup. |
| `unknown` | Unreviewed apps/games or profiles without a supported-use declaration. | Observation allowed; write/action tools refuse with `SAFETY_PROFILE_ACTION_DENIED` unless the operator explicitly permits the reviewed override path. |

Bundled profiles must set `use_scope`; the loader rejects unknown enum strings.
Profiles may also carry a string-valued `[metadata]` table for profile-registry
provenance, benchmark IDs, supported-use declarations, launch hints, and local
quality/audit signal names. Metadata is exposed through `profile_list` so FSV
can read the loaded registry state directly.

Profile TOML examples in `07_storage_and_profiles.md`.

---

## 7. Storage records (RocksDB values)

RocksDB stored records use JSON so the on-disk source of truth stays inspectable and avoids bincode's RUSTSEC-2025-0141 risk. Choice documented in `07_storage_and_profiles.md`.

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredRedaction {
    pub kind: String,
    pub offset: u32,
    pub len: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredEvent {
    pub schema_version: u32,
    pub event_id: String,
    pub ts_ns: u64,
    pub session_id: Option<SessionId>,
    pub source: EventSource,
    pub kind: String,
    pub data: serde_json::Value,
    pub window_id: Option<i64>,
    pub element_id: Option<ElementId>,
    pub redacted: bool,
    pub redactions: Vec<StoredRedaction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredObservation {
    pub schema_version: u32,
    pub observation_id: String,
    pub ts_ns: u64,
    pub session_id: Option<SessionId>,
    pub mode: PerceptionMode,
    pub foreground: ForegroundContext,
    pub focused: Option<FocusedElement>,
    pub elements: Vec<AccessibleNode>,
    pub entities: Vec<DetectedEntity>,
    pub hud: HudReadings,
    pub audio: AudioContext,
    pub recent_events: Vec<EventSummary>,
    pub clipboard_summary: Option<ClipboardSummary>,
    pub fs_recent: Vec<FsEvent>,
    pub diagnostics: ObservationDiagnostics,
    pub reason: String,                         // "1hz_sample" | "before_action" | "user_requested"
    pub redacted: bool,
    pub redactions: Vec<StoredRedaction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredReflexStep {
    pub index: u32,
    pub action: Action,
    pub status: String,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredReflexAudit {
    pub schema_version: u32,
    pub audit_id: String,
    pub reflex_id: ReflexId,
    pub ts_ns: u64,
    pub status: ReflexState,
    pub event_id: Option<String>,
    pub steps: Vec<StoredReflexStep>,
    pub error_code: Option<String>,
    pub details: serde_json::Value,
    pub redacted: bool,
    pub redactions: Vec<StoredRedaction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredProfileHistoryEntry {
    pub profile_id: ProfileId,
    pub activated_at: chrono::DateTime<chrono::Utc>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredSession {
    pub schema_version: u32,
    pub session_id: SessionId,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
    pub transport: String,                      // "stdio" | "http"
    pub client: Option<String>,                 // e.g., "claude-desktop/0.4.2"
    pub mode: PerceptionMode,
    pub active_profile: Option<ProfileId>,
    pub profile_history: Vec<StoredProfileHistoryEntry>,
    pub redacted: bool,
    pub redactions: Vec<StoredRedaction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OcrResult {
    pub full_text: String,
    pub words: Vec<OcrWord>,
    pub confidence: f32,
    pub region: Rect,
    pub lang: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OcrWord {
    pub text: String,
    pub bbox: Rect,
    pub confidence: f32,
}
```

M3 changes `OcrResult` from the pre-v1 `text` / `language` / `backend` output to the
shape above. Cached OCR payloads are wipe-and-rebuild data; do not add a migration shim
for the pre-v1 shape.

---

## 8. Error codes (full catalog)

Stable identifiers. Adding a code is a release-note item; renaming is forbidden until v2.

### 8.1 Perception

```
OBSERVE_NO_PERCEPTION_AVAILABLE
OBSERVE_INTERNAL
CAPTURE_GRAPHICS_API_UNSUPPORTED
CAPTURE_TARGET_LOST
CAPTURE_NO_DIRTY_REGIONS
A11Y_NOT_AVAILABLE
A11Y_ELEMENT_STALE
A11Y_NO_FOREGROUND
A11Y_CDP_UNREACHABLE
DETECTION_MODEL_NOT_LOADED
DETECTION_MODEL_INFER_FAILED
DETECTION_NO_FRAME
OCR_NO_TEXT
OCR_BACKEND_UNAVAILABLE
HUD_NO_ACTIVE_PROFILE
HUD_FIELD_NOT_DEFINED
HUD_EXTRACTION_FAILED
AUDIO_DEVICE_LOST
AUDIO_LOOPBACK_INIT_FAILED
AUDIO_STT_MODEL_NOT_LOADED
```

### 8.2 Action

```
ACTION_QUEUE_FULL
ACTION_RATE_LIMITED
ACTION_BACKEND_UNAVAILABLE
ACTION_TARGET_INVALID
ACTION_HOLD_EXCEEDED_MAX
ACTION_HID_PORT_DISCONNECTED
ACTION_VIGEM_NOT_INSTALLED
ACTION_VIGEM_PLUGIN_FAILED
ACTION_ELEMENT_NOT_RESOLVED
ACTION_FOREGROUND_LOST
ACTION_UNSUPPORTED_KEY
ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT
STUCK_KEY_AUTO_RELEASED
SAFETY_RELEASE_ALL_FIRED
SAFETY_OPERATOR_HOTKEY_FIRED
```

M4 action-path codes:

| Code | Trigger path |
|---|---|
| `ACTION_QUEUE_FULL` | Action queue or hardware emitter backpressure rejects a new action. |
| `ACTION_BACKEND_UNAVAILABLE` | Requested backend is disabled, unavailable, or missing required host setup. |
| `ACTION_TARGET_INVALID` | Launch/window target, action target, or hardware route target cannot be resolved. |
| `ACTION_HID_PORT_DISCONNECTED` | `Backend::Hardware` is disconnected, reconnecting, or failed after serial loss. |
| `SAFETY_RELEASE_ALL_FIRED` | Release-all interlock cancels pending or held inputs. |
| `SAFETY_OPERATOR_HOTKEY_FIRED` | Operator panic hotkey cancels pending or held inputs. |

### 8.3 Reflex

```
REFLEX_CAP_REACHED
REFLEX_KIND_INVALID
REFLEX_PARAMS_INVALID
REFLEX_TARGET_INVALID
REFLEX_FILTER_INVALID
REFLEX_PRIORITY_INVALID
REFLEX_TICK_LATE
REFLEX_TRACK_LOST
REFLEX_STARVED
REFLEX_DISABLED_BY_OPERATOR
REFLEX_LIFETIME_EXPIRED
REFLEX_RECURSION_LIMIT
REFLEX_ACTION_PERMISSION_DENIED
```

M4 reflex-path codes:

| Code | Trigger path |
|---|---|
| `REFLEX_ACTION_PERMISSION_DENIED` | Combo or event-triggered reflex suppresses an action because the active profile or session policy denies it. |

### 8.4 Profile & config

```
PROFILE_NOT_FOUND
PROFILE_PARSE_ERROR
PROFILE_VERSION_INCOMPATIBLE
PROFILE_KEYMAP_INVALID
PROFILE_HUD_REGION_INVALID
CAPTURE_TARGET_INVALID
PERCEPTION_MODE_INVALID
PROFILE_TRUST_VERIFICATION_FAILED
PROFILE_ROLLBACK_UNAVAILABLE
AUDIT_EXPORT_CONSENT_REQUIRED
AUDIT_EXPORT_REDACTION_REQUIRED
AUDIT_EXPORT_PAYLOAD_TOO_LARGE
PROFILE_AUTHORING_INSUFFICIENT_EVIDENCE
PROFILE_AUTHORING_CONFLICTING_EVIDENCE
PROFILE_AUTHORING_UNSAFE_ESCALATION
PROFILE_AUTHORING_CANDIDATE_NOT_FOUND
PROFILE_AUTHORING_INVALID_STATE
```

M5 profile-registry trust/rollback, audit-export, and profile-authoring codes:

| Code | Trigger path |
|---|---|
| `PROFILE_TRUST_VERIFICATION_FAILED` | `profile_registry_install` rejects a package because signed trust is required and the signature is missing, invalid, or not rooted in a trusted signer. The failed package is written only to a quarantine row. |
| `PROFILE_ROLLBACK_UNAVAILABLE` | `profile_registry_rollback` cannot find or validate a prior trusted/local-validated package target, so the installed row is left unchanged. |
| `AUDIT_EXPORT_CONSENT_REQUIRED` | `audit_export_bundle` cannot find an enabled local consent row in `CF_KV/audit_export/v1/consent/<profile_id>`, finds a disabled/invalid row, or detects a non-local sharing flag. |
| `AUDIT_EXPORT_REDACTION_REQUIRED` | `audit_export_consent_set` or `audit_export_bundle` receives a missing, unsupported, or non-consented redaction policy. |
| `AUDIT_EXPORT_PAYLOAD_TOO_LARGE` | `audit_export_bundle` finds a matching `CF_ACTION_LOG` row larger than `max_row_bytes` and aborts before writing bundle files. |
| `PROFILE_AUTHORING_INSUFFICIENT_EVIDENCE` | `profile_authoring_generate` finds no relevant audit/replay evidence or the relevant evidence produces an empty patch; no candidate row is written. |
| `PROFILE_AUTHORING_CONFLICTING_EVIDENCE` | `profile_authoring_generate` sees contradictory hints for the same keymap, backend, HUD field, metadata, or use-scope field; no candidate row is written. |
| `PROFILE_AUTHORING_UNSAFE_ESCALATION` | `profile_authoring_generate` sees evidence that would escalate local policy, such as hardware/ViGEm defaults, sanctioned-research scope, remote-server enablement, shell enablement, or hardware-input metadata; no candidate row is written. |
| `PROFILE_AUTHORING_CANDIDATE_NOT_FOUND` | `profile_authoring_accept`, `profile_authoring_reject`, or `profile_authoring_export` cannot read the requested `CF_PROFILES/profile_authoring/v1/candidate/<candidate_id>` row. |
| `PROFILE_AUTHORING_INVALID_STATE` | `profile_authoring_accept` or `profile_authoring_reject` is requested from an incompatible candidate state. |

### 8.5 MCP & session

```
SESSION_NOT_FOUND
SESSION_EXPIRED
SUBSCRIPTION_NOT_FOUND
SUBSCRIPTION_CAP_REACHED
TOOL_NOT_FOUND
TOOL_PARAMS_INVALID
TOOL_INTERNAL_ERROR
HTTP_BIND_NON_LOOPBACK_REFUSED
HTTP_TOKEN_INVALID
HTTP_ORIGIN_REFUSED
HTTP_SESSION_INVALID
REPLAY_TARGET_INVALID
REPLAY_FORMAT_INVALID
```

M4 MCP/session-path codes:

| Code | Trigger path |
|---|---|
| `TOOL_PARAMS_INVALID` | Invalid `act_combo`, `act_run_shell`, `act_launch`, `hid identify`, or `hid flash` parameters after schema/default resolution. |

### 8.6 Storage

```
STORAGE_OPEN_FAILED
STORAGE_WRITE_FAILED
STORAGE_READ_FAILED
STORAGE_CORRUPTED
STORAGE_SCHEMA_MISMATCH
STORAGE_DISK_PRESSURE_LEVEL_1
STORAGE_DISK_PRESSURE_LEVEL_2
STORAGE_DISK_PRESSURE_LEVEL_3
STORAGE_DISK_PRESSURE_LEVEL_4
STORAGE_CF_HARD_CAP_REACHED
```

### 8.7 Models

```
MODEL_DOWNLOAD_FAILED
MODEL_HASH_MISMATCH
MODEL_LOAD_FAILED
MODEL_BACKEND_UNAVAILABLE
```

### 8.8 Hardware HID

```
HID_PORT_NOT_FOUND
HID_PORT_OPEN_FAILED
HID_PROTOCOL_HANDSHAKE_FAILED
HID_FIRMWARE_VERSION_MISMATCH
HID_COMMAND_REJECTED
HID_LINK_TIMEOUT
```

M4 hardware-HID path codes:

| Code | Trigger path |
|---|---|
| `HID_PORT_NOT_FOUND` | Auto-discovery or configured serial path cannot find a candidate RP2040 CDC ACM port. |
| `HID_PORT_OPEN_FAILED` | Candidate serial port exists but cannot be opened with the requested access/settings. |
| `HID_PROTOCOL_HANDSHAKE_FAILED` | Host and firmware fail the initial protocol handshake. |
| `HID_FIRMWARE_VERSION_MISMATCH` | Firmware reports an incompatible protocol or firmware version. |
| `HID_COMMAND_REJECTED` | Firmware receives a syntactically valid command but rejects it by status code. |
| `HID_LINK_TIMEOUT` | Host does not receive the expected firmware response before the command deadline. |

### 8.9 Safety

```
SAFETY_KILLSWITCH_ACTIVE
SAFETY_PROCESS_DENYLISTED
SAFETY_SHELL_DENIED_BY_POLICY
SAFETY_LAUNCH_DENIED_BY_POLICY
SAFETY_SECRET_REDACTED
SAFETY_PERMISSION_DENIED
SAFETY_PROFILE_ACTION_DENIED
```

M4 safety-policy path codes:

| Code | Trigger path |
|---|---|
| `SAFETY_SHELL_DENIED_BY_POLICY` | `act_run_shell` is refused by the active allowlist, supported-use scope, or operator permission policy. |
| `SAFETY_LAUNCH_DENIED_BY_POLICY` | `act_launch` is refused by the active allowlist, supported-use scope, or operator permission policy. |
| `SAFETY_PERMISSION_DENIED` | A generic M4 tool/action request lacks the session or operator permission required for the requested side effect. |
| `SAFETY_PROFILE_ACTION_DENIED` | Active `Profile.use_scope` blocks write/action tools for an unsupported or unknown target profile. |

All codes exported as `pub const NAME: &str = "NAME";` in `synapse-core::error_codes`. Tests assert constants match their literal string.

---

## 9. Versioning

`SCHEMA_VERSION` constant in `synapse-core`. Every persisted record carries this version. Reading a record with mismatched version returns `STORAGE_SCHEMA_MISMATCH`; operator wipes the DB and restarts.

Pre-v1: bump major freely. Post-v1: schema changes require ADR + migration plan or DB wipe with release-notes warning.

---

## 10. Out of scope

- Storage layout, CF list, key encoding → `07_storage_and_profiles.md`
- Profile TOML examples → `07_storage_and_profiles.md`
- HID protocol record format → `09_hardware_hid_gateway.md`
- Tool API surface that consumes these types → `05_mcp_tool_surface.md`
