# 05 — Core Types and Error Hierarchy (`synapse-core`)

Source files covered:
- `crates/synapse-core/src/lib.rs`
- `crates/synapse-core/src/defaults.rs`
- `crates/synapse-core/src/error_codes.rs`
- `crates/synapse-core/src/filter.rs`
- `crates/synapse-core/src/retention.rs`
- `crates/synapse-core/src/types.rs`

## 1. Crate role

`synapse-core` is the **single dependency-free type/contract crate**. Every other Synapse crate depends on it; it depends on no other Synapse crate. It defines:

- All wire-level structs/enums that travel over MCP (params, responses, observations, events, profiles, reflex registrations, stored persistence variants, health payload).
- The `pub const` error code string set (`SCREAMING_SNAKE_CASE`).
- Retention defaults consumed by `synapse-storage`.
- Reference performance budgets used by tests and tooling.
- The `EventFilter` evaluator (`filter.rs`).

## 2. Constants

| Constant | Value | Used by |
|---|---|---|
| `SCHEMA_VERSION` | `1` | `synapse-storage::Db::open`, `StoredEvent`, `StoredObservation`, `StoredReflexAudit`, `StoredSession` payloads |
| `REFERENCE_OBSERVE_WARM_HYBRID_P99_MS` | `30.0` | Perf budget tests |
| `REFERENCE_REFLEX_TICK_JITTER_IDLE_P99_US` | `200` | Perf budget tests |
| `REFERENCE_EVENT_TO_SUBSCRIBER_P99_MS` | `50.0` | Perf budget tests |
| `EVENT_FILTER_MAX_DEPTH` | `8` | `EventFilter::validate` |

## 3. Error code catalog

All 95 codes are `pub const &'static str` in `crates/synapse-core/src/error_codes.rs`. Mapped from each subsystem's `thiserror` enum's `.code()` method. Categories with line ranges (see [01_system_overview.md §8](01_system_overview.md) for the table).

M3 added the following codes to the M2 baseline: `REFLEX_RECURSION_LIMIT`, `REFLEX_ACTION_PERMISSION_DENIED`, `HTTP_BIND_NON_LOOPBACK_REFUSED`, `HTTP_TOKEN_INVALID`, `HTTP_ORIGIN_REFUSED`, `HTTP_SESSION_INVALID`, `REPLAY_TARGET_INVALID`, `REPLAY_FORMAT_INVALID`, `SUBSCRIPTION_CAP_REACHED`, `STORAGE_DISK_PRESSURE_LEVEL_1..4`, `STORAGE_CF_HARD_CAP_REACHED`, `SAFETY_PERMISSION_DENIED`, `SAFETY_PROFILE_ACTION_DENIED`, `SAFETY_OPERATOR_HOTKEY_FIRED`.

## 4. Retention defaults

`retention.rs` exports `DEFAULTS: [RetentionDefault; 11]` and the `RetentionTtl` enum.

```rust
pub enum RetentionTtl { None, Hours(u64), Days(u64), LruOnly }

pub struct RetentionDefault {
    pub cf: &'static str,
    pub ttl: RetentionTtl,
    pub soft_cap_mb: u64,
    pub hard_cap_mb: u64,
}
```

Full table in [04_storage_layer.md §6](04_storage_layer.md).

## 5. Wire-level types (`types.rs`)

### 5.1 Identity and primitives

| Type | Source | Notes |
|---|---|---|
| `Backend` | enum `Software` \| `Vigem` \| `Hardware` \| `Auto` | All four lowercased on the wire. `Auto` resolves from the active action backend policy: default session = keyboard/mouse/combo/release-all `software`, pad `vigem`; profile `default_backend = "hardware"` makes Auto resolve to `hardware` unless a class default overrides it. |
| `Point` | `{ x: i32, y: i32 }` | screen coords; provides `distance_to(other: Self) -> f64` |
| `Rect` | `{ x: i32, y: i32, w: i32, h: i32 }` | `contains(point: Point)` with exclusive right/bottom edges; non-positive width/height treated as empty |
| `Size` | `{ w: u32, h: u32 }` | |
| `SessionId` / `EntityId` / `ReflexId` / `SubscriptionId` / `ProfileId` | `type ... = String` | UUIDs (v7 for reflex/subscription/session; v4 elsewhere); profile id is a TOML-supplied label |
| `ElementId` | newtype `String`, formatted `<hwnd_hex>:<runtime_id_hex>` | Pattern `^-?0x[0-9a-fA-F]+:[0-9a-fA-F]+$`; `parse()` / `parts()` / `try_from(String)` |
| `ElementIdParts` | `{ hwnd: i64, runtime_id_hex: String }` | |
| `ElementIdParseError` | enum `MissingSeparator` \| `InvalidHwnd` \| `InvalidRuntimeId` | thiserror |

ID generators (return `String`): `new_session_id()` (uuid v7), `new_reflex_id()` (v7), `new_subscription_id()` (v7), `element_id(hwnd: i64, runtime_id_hex: &str)`, `entity_id(track: u64)` (returns `"track:{track}"`).

### 5.2 Actions

```rust
pub enum Action {
    KeyPress { key: Key, hold_ms: u32, backend: Backend },
    KeyDown { key: Key, backend: Backend },
    KeyUp { key: Key, backend: Backend },
    KeyChord { keys: Vec<Key>, hold_ms: u32, backend: Backend },
    TypeText { text: String, dynamics: KeystrokeDynamics, backend: Backend },
    MouseMove { to: MouseTarget, curve: AimCurve, duration_ms: u32, backend: Backend },
    MouseMoveRelative { dx: f32, dy: f32, backend: Backend },
    MouseButton { button: MouseButton, action: ButtonAction, hold_ms: u32, backend: Backend },
    MouseDrag { from: Point, to: Point, button: MouseButton, curve: AimCurve, duration_ms: u32, backend: Backend },
    MouseScroll { dy: i32, dx: i32, at: Option<Point>, backend: Backend },
    PadButton { pad: PadId, button: PadButton, action: ButtonAction, hold_ms: u32 },
    PadStick { pad: PadId, stick: Stick, x: f32, y: f32 },
    PadTrigger { pad: PadId, trigger: Trigger, value: f32 },
    PadReport { pad: PadId, report: GamepadReport },
    AimAt { target: AimTarget, style: AimStyle, deadline_ms: u32, backend: Backend },
    Combo { steps: Vec<ComboStep>, backend: Backend },
    ReleaseAll,
}
```

`#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]`.

Supporting types:

| Type | Definition |
|---|---|
| `AimCurve` | `Instant` \| `Linear` \| `EaseInOut` \| `Bezier { p1: (f32, f32), p2: (f32, f32) }` \| `Natural { params: AimNaturalParams }` |
| `AimNaturalParams` | `{ control_point_jitter, tremor_stddev_px, overshoot_prob, overshoot_factor_range: (f32, f32), micro_correct_steps: u8, timing_stddev_ms, seed: Option<u64> }`. `FAST` preset: `(0.08, 0.2, 0.25, (1.02, 1.06), 1, 1.5, None)` — pinned by impplan as the default for every tool/profile/reflex (OQ-004 DECIDED 2026-05-22) |
| `AimStyle` | `Snap` \| `Flick` \| `Natural` \| `Track` |
| `KeystrokeDynamics` | `Burst` \| `Linear { ms_per_char: u32 }` \| `Natural { params: KeystrokeNaturalParams }` |
| `KeystrokeNaturalParams` | `{ mean_iki_ms: f32, stddev_ms: f32, bigram_bias: bool }`. `FAST` preset: `{ mean_iki_ms: 32.0, stddev_ms: 10.0, bigram_bias: true }` (~190 WPM) |
| `Key` | `{ code: KeyCode, use_scancode: bool }` |
| `KeyCode` | `Named { value: String }` \| `Symbol { value: char }` \| `HidCode { value: u8 }` |
| `MouseButton` | `Left` \| `Right` \| `Middle` \| `X1` \| `X2` |
| `ButtonAction` | `Press` \| `Down` \| `Up` |
| `MouseTarget` | `Screen { point }` \| `Element { element_id }` |
| `AimTarget` | `Screen { point }` \| `Element { element_id }` \| `Track { track_id: u64 }` |
| `PadId` | `u8` |
| `GamepadController` | `X360` (default) \| `Ds4` |
| `PadButton` | A/B/X/Y/Lb/Rb/Ls/Rs/Back/Start/Up/Down/Left/Right/Guide |
| `Stick` | `Left` \| `Right` |
| `Trigger` | `Left` \| `Right` |
| `GamepadReport` | `{ controller, buttons: Vec<PadButton>, thumb_l: (f32,f32), thumb_r: (f32,f32), lt: f32 (0..1), rt: f32 (0..1) }` with `neutral(controller)` ctor |
| `ComboStep` | `{ at_ms: u32, input: ComboInput }` |
| `ComboInput` | `KeyDown` / `KeyUp` / `KeyPress { hold_ms: u16 }` / `MouseButton` / `MouseMoveRel { dx: f32, dy: f32 }` / `PadButton` / `PadStick` (all with `key`/`button`/`pad` etc. fields) |

### 5.3 Perception observation

```rust
pub struct Observation {
    pub seq: u64,
    pub at: DateTime<Utc>,
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

Sub-structs:

| Struct | Fields |
|---|---|
| `PerceptionMode` | `A11yOnly` \| `PixelOnly` \| `Hybrid` \| `Auto` |
| `ForegroundContext` | `hwnd: i64`, `pid: u32`, `process_name`, `process_path`, `window_title`, `window_bounds: Rect`, `monitor_index: u32`, `dpi_scale: f32`, `profile_id: Option<ProfileId>`, `steam_appid: Option<u32>`, `is_fullscreen: bool`, `is_dwm_composed: bool` |
| `FocusedElement` | `element_id`, `name`, `role`, `automation_id: Option<String>`, `bbox: Rect`, `enabled`, `patterns: Vec<UiaPattern>`, `value: Option<String>`, `selected_text: Option<String>` |
| `UiaPattern` | Invoke / Toggle / Value / Selection / ExpandCollapse / Scroll / Text / Window / Transform / RangeValue |
| `AccessibleNode` | `element_id`, `parent: Option<ElementId>`, `name`, `role`, `automation_id: Option<String>`, `bbox: Rect`, `enabled`, `focused`, `patterns: Vec<UiaPattern>`, `children_count: u32`, `depth: u32` |
| `AccessibleSubtree` | `{ root: ElementId, nodes: Vec<AccessibleNode>, max_depth: u32, truncated: bool }` |
| `AccessibleQuery` | `{ role, name_substring, automation_id, scope: AccessibleQueryScope }` |
| `AccessibleQueryScope` | `FocusedSubtree` (default) \| `ForegroundWindow` \| `Global` |
| `DetectedEntity` | `entity_id`, `track_id: u64`, `class_label`, `bbox`, `confidence: f32`, `first_seen_at`, `last_seen_at`, `velocity_px_per_s: Option<(f32, f32)>` |
| `Detection` | `class_label`, `bbox`, `confidence`, `track_id: Option<u64>` |
| `DetectionBatch` | `model_id`, `frame_seq`, `inferred_at`, `items: Vec<Detection>` |
| `HudReadings` | `{ by_name: BTreeMap<String, HudReading> }` |
| `HudReading` | `{ raw_text, parsed: HudValue, confidence, stale_ms }` |
| `HudValue` | untagged `Number(f64)` \| `Text(String)` \| `Enum(String)` \| `Null` |
| `AudioContext` | `rms_db: f32`, `vad_speech_recent: bool`, `recent_events: Vec<AudioEvent>`, `direction_estimate: Option<DirectionEstimate>` |
| `AudioEvent` | `at`, `kind: String`, `azimuth_deg: Option<f32>`, `confidence` |
| `DirectionEstimate` | `azimuth_deg: f32`, `confidence: f32` |
| `ClipboardSummary` | `formats: Vec<String>`, `text_len: Option<u32>`, `text_excerpt: Option<String>`, `redacted: bool` |
| `FsEvent` | `at`, `path`, `kind: FsEventKind` (Created/Modified/Deleted/Renamed), `size_bytes: Option<u64>` |
| `ObservationDiagnostics` | `assembled_in_ms`, `sensor_latency_ms: BTreeMap<String, f32>`, `a11y_enabled`, `pixel_enabled`, `audio_enabled`, `a11y_status: SensorStatus`, `capture_status`, `detection_status`, `audio_status`, `elements_truncated`, `entities_truncated`, `size_bytes`, `size_estimate_tokens` |
| `SensorStatus` | `Healthy` \| `DegradedLatency { last_p99_ms: f32 }` \| `DegradedSensorFailed { reason_code: String }` \| `Disabled` \| `Unavailable` (default) |

### 5.4 OCR

| Type | Fields |
|---|---|
| `OcrBackend` | `Winrt` \| `Crnn` \| `Auto` (default) |
| `OcrResult` | `{ full_text: String, words: Vec<OcrWord>, confidence: f32, region: Rect, lang: String }` |
| `OcrWord` | `{ text: String, bbox: Rect, confidence: f32 }` |

### 5.5 Profiles

```rust
pub struct Profile {
    pub id: ProfileId,
    pub label: String,
    pub version: String,
    pub use_scope: ProfileUseScope,
    pub matches: Vec<ProfileMatch>,
    pub mode: PerceptionMode,
    pub capture: ProfileCapture,
    pub detection: ProfileDetection,
    pub ocr: ProfileOcr,
    pub hud: Vec<HudFieldSpec>,
    pub keymap: BTreeMap<String, String>,
    pub backends: ProfileBackends,
    pub event_extensions: Vec<EventExtension>,
}
```

Supporting types:

| Type | Definition |
|---|---|
| `ProfileMatch` | `{ exe, title_regex, steam_appid, window_class, process_args }` (all `Option`/`Vec`) |
| `ProfileUseScope` | `Productivity` / `SinglePlayer` / `OperatorOwnedTest` / `SanctionedResearch` / `Unknown` |
| `ProfileCapture` | `{ target: ProfileCaptureTarget, min_update_interval_ms: u32, cursor_visible: bool }` |
| `ProfileCaptureTarget` | `ForegroundWindow` \| `PrimaryMonitor` \| `MonitorIndex { index: u32 }` |
| `ProfileDetection` | `{ model_id, classes_of_interest: Vec<String>, confidence_threshold: f32, max_detections: u32 }` |
| `ProfileOcr` | `{ default_backend: OcrBackend, regions: Vec<HudRegion>, parser_config: BTreeMap<String, String> }` |
| `HudFieldSpec` | `{ name, region: HudRegion, extractor: HudExtractor, parser: HudParser }` |
| `HudRegion` | `Absolute { x, y, w, h }` \| `FractionOfWindow { x, y, w, h }` (f32) \| `AnchoredToEdge { edge: WindowEdge, x_offset, y_offset, w, h }` |
| `WindowEdge` | `TopLeft` / `TopRight` / `BottomLeft` / `BottomRight` |
| `HudExtractor` | `WinrtOcr` \| `Crnn { model_id }` \| `TemplateMatch { templates }` \| `ColorRatio { sample_points: Vec<(i32, i32)>, mapping }` |
| `HudParser` | `Number` \| `FractionNumerator` \| `FractionDenominator` \| `Regex { pattern, group }` \| `Enum { mapping }` |
| `ProfileBackends` | `{ default, keyboard_default, mouse_default, pad_default: Backend }`; TOML accepts `default_backend` as an alias for `default` |
| `EventExtension` | `{ name, from_filter: EventFilter, emits_kind }` |

### 5.6 Reflex

| Type | Definition |
|---|---|
| `ReflexRegistration` | `{ id: ReflexId, kind: ReflexKind, priority: u32 (default 100), lifetime: ReflexLifetime, exclusive: bool }` |
| `ReflexKind` | `AimTrack { target: AimTarget, axis: ReflexAimAxis, gain, deadzone_px, max_speed_px_per_ms, curve_per_step: AimCurve, backend }` \| `HoldMove { keys, backend, re_assert }` \| `HoldButton { button: ReflexButtonTarget, backend }` \| `Combo { steps, backend }` \| `OnEvent { when: EventFilter, then: ReflexThen, debounce_ms }` |
| `ReflexAimAxis` | `Xy` \| `XOnly` \| `YOnly` |
| `ReflexButtonTarget` | `Mouse { button }` \| `Pad { pad, button }` |
| `ReflexThen` | `Action { action }` \| `Actions { actions: Vec<Action> }` \| `Combo { steps, backend }` |
| `ReflexLifetime` | `UntilCancelled` (default) \| `OneShot` \| `Duration { ms }` \| `UntilEvent { filter }` \| `UntilDeadline { ms }` |
| `ReflexState` | `Active` \| `Paused` \| `Cancelled` \| `Expired` \| `Disabled` \| `Starved` |
| `ReflexStatus` | `{ id, kind_summary, state, registered_at, last_fired_at: Option, fire_count: u64, priority, lifetime, exclusive, last_error_code: Option }` |

### 5.7 Events and filtering

```rust
pub struct Event {
    pub seq: u64,
    pub at: DateTime<Utc>,
    pub source: EventSource,
    pub kind: String,
    pub data: serde_json::Value,
    pub correlations: Vec<EventRef>,
}
```

| Type | Definition |
|---|---|
| `EventSource` | `A11yUia` / `A11yWinEvent` / `A11yCdp` / `Perception` / `PerceptionDetection` / `PerceptionHud` / `PerceptionAudio` / `Filesystem` / `Process` / `Clipboard` / `ActionEmitter` / `Reflex` / `System` |
| `EventRef` | `{ seq: u64, relation: String }` |
| `EventSummary` | `{ seq, at, source, kind, data_excerpt: serde_json::Value }` (built via `Event::summary()`) |
| `EventFilter` | `{ op: "all" / "none" / "kind" / "source" / "and" / "or" / "not" / "data" }` recursive; serde tag = `"op"`; depth limit `EVENT_FILTER_MAX_DEPTH = 8` |
| `EventFilterValidationError` | `EmptyAnd` \| `EmptyOr` \| `DepthExceeded { depth, max_depth }` |
| `DataPredicate` | `Eq` / `Ne` / `Lt` / `Le` / `Gt` / `Ge` / `Regex { pattern }` / `InSet { values }` / `Exists` |

`EventFilter::matches(&Event) -> bool` delegates to `crate::filter::matches_event_filter` which dispatches per op:
- `All` → true, `None` → false
- `Kind { kind }` → `event.kind == *kind`
- `Source { source }` → `event.source == *source`
- `And { args }` → `args.iter().all(...)`, `Or { args }` → `.any(...)`, `Not { arg }` → negate
- `Data { path, predicate }` → `predicate.matches(event.data.pointer(path))`

`DataPredicate::matches`: `Exists` is `value.is_some()`; comparison ops use `compare_values` (number/string lexicographic); `Regex` compiles per call and runs `is_match`; `InSet` is `values.iter().any(|v| v == actual)`.

Validation (`EventFilter::validate`): rejects empty `And`/`Or` and trees deeper than `EVENT_FILTER_MAX_DEPTH = 8`. Returns `EventFilterValidationError`, mapped by callers to `REFLEX_FILTER_INVALID`/`TOOL_PARAMS_INVALID`.

### 5.8 Health

```rust
pub struct Health {
    pub ok: bool,
    pub version: String,         // env!("CARGO_PKG_VERSION") = "0.1.0"
    pub build: String,           // option_env!("VERGEN_GIT_SHA") or "dev"
    pub uptime_s: u64,           // monotonic via Instant::elapsed
    pub subsystems: BTreeMap<String, SubsystemHealth>,
}
```

`SubsystemHealth` (open-ended; only `Some(_)` fields are serialized):

```rust
pub struct SubsystemHealth {
    pub status: String,
    pub detail: Option<String>,
    pub active_profile_id: Option<ProfileId>,
    pub db_path: Option<String>,
    pub schema_version: Option<u32>,
    pub cf_sizes: Option<BTreeMap<String, u64>>,
    pub active_count: Option<usize>,
    pub last_tick_jitter_us: Option<u64>,
    pub recursion_clamps_total: Option<u64>,
    pub profile_count: Option<usize>,
    pub last_reload_at: Option<String>,
    pub device_name: Option<String>,
    pub ring_buffer_seconds: Option<u32>,
    pub stt_model_loaded: Option<bool>,
    pub bind_addr: Option<String>,
    pub active_sessions: Option<usize>,
    pub sse_subscribers: Option<usize>,
    pub backend_resolution: Option<BTreeMap<String, String>>,
}
```

Subsystem status strings emitted by `synapse-mcp/src/server.rs`:

| Subsystem | Status values |
|---|---|
| `storage` | `initializing` \| `ok` \| `error` \| `disk_pressure_l1..4` |
| `action` | `ok` \| `error` |
| `reflex` | `initializing` \| `ok` \| `degraded_latency` \| `disabled` \| `error` |
| `profiles` | `initializing` \| `ok` \| `error` |
| `audio` | `initializing` \| `ok` \| `disabled` \| `error` |
| `http` | `disabled` (stdio mode) \| `ok` (http mode) \| `error` |

### 5.9 Stored persistence variants

Used as the JSON payload values in RocksDB column families. See [04_storage_layer.md §4.1](04_storage_layer.md) for table.

- `StoredEvent` (CF_EVENTS)
- `StoredObservation` (CF_OBSERVATIONS)
- `StoredReflexAudit` + `StoredReflexStep` (CF_REFLEX_AUDIT)
- `StoredSession` + `StoredProfileHistoryEntry` (CF_SESSIONS)
- `StoredRedaction` reused across the above (`{ kind: String, offset: u32, len: u32 }`)

Every stored type carries `schema_version: u32` so a future migration framework can branch on version. The current code unconditionally writes `synapse_core::SCHEMA_VERSION = 1`.

## 6. Serde conventions

| Rule | Applied via | Why |
|---|---|---|
| `deny_unknown_fields` | most structs | Forward-incompatible fields fail loudly rather than silently |
| `tag = "kind"` on enums | `Action`, `AimCurve`, `KeystrokeDynamics`, `KeyCode`, `MouseTarget`, `AimTarget`, `ComboInput`, `ProfileCaptureTarget`, `HudRegion`, `HudExtractor`, `HudParser`, `ReflexKind`, `ReflexButtonTarget`, `ReflexThen`, `ReflexLifetime` | Discriminated unions encode as `{ "kind": "...", ... }` for JSON ergonomics |
| `tag = "op"` on filter | `EventFilter`, `DataPredicate` | Same idea but the discriminant key is `"op"` |
| `rename_all = "snake_case"` (mostly) | most enums | Wire ergonomics |
| `rename_all = "lowercase"` | `Backend`, `MouseButton`, `ButtonAction`, `PadButton`, `Stick`, `Trigger`, `GamepadController`, `FsEventKind`, some action enums | Single-word variants stay terse |
| JSON schema generation | `schemars 1.2.1` derives `JsonSchema` on every public param/response | Auto-derived MCP `tools/list` schemas |

## 7. JSON Schema generators (non-derive)

- `normalized_axis_pair_schema` (`types.rs:312`): writes a 2-element array with each component in `[-1.0, 1.0]` for `GamepadReport::thumb_l`/`thumb_r`.
- `ElementId::json_schema` (`types.rs:498`): emits `{ "type": "string", "pattern": "^-?0x[0-9a-fA-F]+:[0-9a-fA-F]+$" }`.

## 8. Public utility functions

| Function | Source | Behavior |
|---|---|---|
| `element_id(hwnd: i64, runtime_id_hex: &str) -> ElementId` | `types.rs:555` | Formats hex hwnd (with `-0x` prefix if negative) then `:<runtime_id_hex>` |
| `entity_id(track: u64) -> EntityId` | `types.rs:565` | Returns `"track:{track}"` |
| `new_session_id() -> SessionId` | `types.rs:540` | uuid v7 string |
| `new_reflex_id() -> ReflexId` | `types.rs:545` | uuid v7 string |
| `new_subscription_id() -> SubscriptionId` | `types.rs:550` | uuid v7 string |
| `EventFilter::matches(&Event) -> bool` | `types.rs:1418` | delegates to `crate::filter::matches_event_filter` |
| `EventFilter::depth() -> u32` | `types.rs:1422` | recursive deepest path length |
| `EventFilter::validate()` / `validate_with_max_depth(max)` | `types.rs:1445` / `1455` | Validation of And/Or non-empty + depth bound |
| `DataPredicate::matches(Option<&serde_json::Value>) -> bool` | `types.rs:1517` | dispatches into `crate::filter::matches_data_predicate` |
| `Event::summary() -> EventSummary` | `types.rs:1342` | clones for SSE wire excerpts |
| `Rect::contains(point) -> bool` | `types.rs:405` | inclusive-left, exclusive-right semantics |
| `Point::distance_to(other) -> f64` | `types.rs:383` | `hypot(dx, dy)` |
| `GamepadReport::neutral(controller) -> Self` | `types.rs:294` | zero axes / no buttons / 0 triggers |

## 9. Tests (within crate)

`crates/synapse-core/tests/` contains 10 integration test files (~ each file covers one type family):

- `action_serde_proptest.rs`, `action_snapshots.rs`, `action_types.rs` — Action enum roundtrips
- `error_codes_literal.rs` — Each `error_codes::*` is exactly its name (no typos)
- `event_filter_types.rs` — EventFilter validation + matching
- `ocr_types.rs`, `profile_types.rs`, `reflex_types.rs`, `stored_types.rs`, `types.rs` — schema/roundtrip coverage
- `snapshots.rs` — insta-driven JSON snapshots

## 10. What is NOT covered

- **No runtime behavior.** `synapse-core` is pure data; it has no I/O, no async, no global state.
- **No backward-compatible deserialization.** `deny_unknown_fields` is everywhere; pre-v1 doctrine says schema bumps wipe-and-rebuild.
- **No re-export of subsystem error enums.** `synapse-core` defines only the `pub const` codes; concrete `thiserror` enums live next to their owning crate (see [01_system_overview.md §8](01_system_overview.md)).
