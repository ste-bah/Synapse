# 09 — Perception, A11y, and Capture (`synapse-perception`, `synapse-a11y`, `synapse-capture`)

Source files covered:
- `crates/synapse-perception/src/lib.rs`
- `crates/synapse-perception/src/error.rs`
- `crates/synapse-perception/src/event_extensions.rs`
- `crates/synapse-perception/src/hud/{mod,anchor,extractor}.rs`
- `crates/synapse-perception/src/observe.rs`
- `crates/synapse-perception/src/ocr.rs`
- `crates/synapse-perception/src/template_match.rs`
- `crates/synapse-a11y/src/lib.rs`
- `crates/synapse-capture/src/lib.rs`
- `crates/synapse-mcp/src/m1.rs`
- `crates/synapse-mcp/src/m1/{ocr, search, sources}.rs`

## 1. Crate split

| Crate | Role |
|---|---|
| `synapse-capture` | Zero-copy GPU frame capture (`windows-capture` / DXGI). Owns `CaptureBackend`, `CaptureTarget`, `CaptureConfig`, `CapturedFrame`, DPI awareness, screen↔window coordinate helpers. |
| `synapse-a11y` | UIA tree walk + WinEvent hook + Chromium DevTools attach + accessible-event coalescing. |
| `synapse-perception` | Glue layer: assembles `Observation` from a11y + capture + OCR inputs; resolves perception mode (auto/a11y_only/pixel_only/hybrid); exposes OCR, HUD anchor, HUD template, and event-extension helpers. |
| `synapse-mcp/src/m1` | MCP tool wrappers (`observe`, `find`, `read_text`, `set_capture_target`, `set_perception_mode`). |

## 2. `synapse-capture`

### 2.1 Public types (selected)

| Type | Definition / purpose |
|---|---|
| `CAPTURE_CHANNEL_CAPACITY` | `2` — bounded crossbeam channel from capture thread to consumer |
| `FRAMES_DROPPED_METRIC` | `"synapse_capture_frames_dropped_total"` |
| `D3d11Texture` | `windows::Win32::Graphics::Direct3D11::ID3D11Texture2D` on Windows, stub elsewhere |
| `SendablePtr<T>` | `Send + Sync` wrapper around a non-Send GPU handle (`unsafe impl`) so it can travel across threads |
| `DxgiFormat` | `Bgra8` \| `Bgra8Srgb` \| `Rgba8` \| `Rgba8Srgb` \| `Rgba16F` \| `Rgb10A2` \| `Rgb10XrA2` \| `Unknown(u32)` |
| `CapturedFrame` | `{ texture: SendablePtr<D3d11Texture>, width, height, format: DxgiFormat, captured_at: Instant, frame_seq: u64, dirty_region: Option<Rect> }` |
| `CapturedSoftwareBitmap` | `{ region: Rect, bitmap: windows::Graphics::Imaging::SoftwareBitmap }` — used by the WinRT OCR path |
| `CaptureBackend` | `GraphicsCaptureApi` \| `DxgiDuplication` |
| `CaptureBackendPreference` | `Auto` \| `GraphicsCaptureApi` \| `DxgiDuplication` (`from_force_dxgi_value` reads `SYNAPSE_CAPTURE_FORCE_DXGI`) |
| `CaptureTarget` | `Primary` (default) \| `Monitor { monitor_index: u32 }` \| `Window { hwnd: i64 }` |
| `CaptureConfig` | `{ target, min_update_interval_ms: u64 (default 16 → ~60 Hz), cursor_visible: bool (default true), secondary_windows: bool (default true), dirty_region_only: bool (default true), backend_preference: Auto }` |
| `ResolvedCaptureTarget` | `{ target: CaptureTarget, backend: CaptureBackend }` |
| `CaptureError` | `GraphicsApiUnsupported` / `TargetLost` / `TargetInvalid` / `NoDirtyRegions` / `ThreadFailed` with `.code()` → `CAPTURE_*` |
| `CaptureStats` | runtime stats (frames captured, dropped, last frame seq) |
| `CaptureThreadPriority` | `Unknown` (sentinel) \| `Unsupported` \| `TimeCritical` |
| `DpiAwarenessStatus` | result of `init_process_dpi_awareness` |
| `CaptureHandle` | producer / consumer pair for spawned capture loops |
| `CaptureController` | manages capture lifecycle |

### 2.2 Capture loop

`spawn_capture_loop(config) -> Result<CaptureHandle, CaptureError>`:

1. Initialize per-monitor-v2 DPI awareness via `init_process_dpi_awareness()` (idempotent; sets `PROCESS_PER_MONITOR_DPI_AWARE`).
2. Resolve target → `ResolvedCaptureTarget`. Backend `Auto` prefers `GraphicsCaptureApi` on Windows 10 1903+ and falls back to `DxgiDuplication` on older systems or when `SYNAPSE_CAPTURE_FORCE_DXGI` forces it.
3. Spawn a high-priority capture thread (Windows: `THREAD_PRIORITY_TIME_CRITICAL`).
4. Capture loop reads frames from `windows-capture` or DXGI duplication, tags with `frame_seq` (monotonic), optionally clips to `dirty_region`, and `try_send` into a bounded `crossbeam` channel of capacity `CAPTURE_CHANNEL_CAPACITY = 2`. On `Full`, the oldest frame is dropped and the `synapse_capture_frames_dropped_total` counter is incremented.
5. Respects `min_update_interval_ms` (default 16 ms) as the minimum interval between produced frames.

### 2.3 Coordinate helpers

| Function | Purpose |
|---|---|
| `screen_to_window(point: Point, hwnd: i64) -> Result<Point, CaptureError>` | Screen-space (DIPs) to window-relative |
| `window_to_screen(point: Point, hwnd: i64) -> Result<Point, CaptureError>` | Inverse |
| `screen_to_window_with_origin` / `window_to_screen_with_origin` | Const helpers when the origin is already known (used by element-target click) |
| `init_process_dpi_awareness() -> Result<DpiAwarenessStatus, CaptureError>` | Sets `PROCESS_PER_MONITOR_DPI_AWARE_V2` once; idempotent |
| `is_per_monitor_v2_dpi_aware() -> bool` | Read-back |
| `current_thread_priority() -> CaptureThreadPriority` | Used by capture-loop self-check |

### 2.4 WinRT OCR helpers

`captured_frame_region_to_software_bitmap(frame, region)` and `screen_region_to_software_bitmap(region)` produce `CapturedSoftwareBitmap` for the WinRT OCR backend.

## 3. `synapse-a11y`

Single-file 2087 LoC crate on `main` (HEAD `e54ca57`). Wraps `uiautomation` 0.25 and `chromiumoxide` 0.9. M3 carry-over: a `platform/*` module split is queued for M4 Block A.0 (see `docs/impplan/04_m3_reflex_mcp_surface.md`); when that lands, `lib.rs` becomes a 30-LoC re-export surface with logic in `cdp.rs` / `events.rs` / `ids.rs` / `re_resolve.rs` / `snapshot.rs` / `window.rs` / `platform/{non_windows.rs, windows/{common,events,resolve,snapshot,window}.rs}`.

### 3.1 Public surface

| Symbol | Purpose |
|---|---|
| `uiautomation` re-export, `UIElement` re-export | direct UIA access for downstream crates |
| `A11yError` (variants: not-available, element-stale, no-foreground, CDP-unreachable, …) → `.code()` → `A11Y_*` | structured error |
| `AccessibleEvent` (`source`, `at`, `kind: AccessibleEventKind`, `element_id`, `data`) | UIA / WinEvent / CDP normalized event |
| `AccessibleEventKind` | `FocusChanged`, `StructureChanged`, `ValueChanged`, `WindowOpened`, `WindowClosed`, etc. |
| `AccessibleEventSender = UnboundedSender<AccessibleEvent>` | bridge endpoint |
| `WinEventSubscription` (drop = unhook) | result of `subscribe_win_events(sender)` |
| `WinEventHookReadback` (status of the WinEvent hook thread) | observability |
| `ComApartmentKind` | STA / MTA / Uninitialized |
| `current_foreground_context() -> A11yResult<ForegroundContext>` | reads HWND of `GetForegroundWindow`, queries process name/path, monitor index, DPI, fullscreen flag |
| `focused_element() -> A11yResult<UIElement>` | UIA focused element |
| `focused_window() -> A11yResult<UIElement>` | top-level window for the focused element |
| `element_from_point(point) -> A11yResult<UIElement>` | hit-test |
| `snapshot(root, depth) -> A11yResult<AccessibleSubtree>` | bounded UIA tree walk via cache batch (depth ≤ 6) |
| `find_by_name_and_pattern(root, name, pattern)` | quick search for a node by `Name` + supported pattern |
| `re_resolve(&ElementId) -> A11yResult<UIElement>` | re-acquire a UIA element by its `<hwnd_hex>:<runtime_id_hex>` |
| `expand_state_of(&UIElement) -> A11yResult<ExpandState>` | `ExpandCollapsePattern` query |
| `coalesce_events<I>(events, window)` | dedupe within a sliding `Duration` window |
| `debounce_value_changes<I>(events, window)` | collapse rapid `ValueChanged` events on the same element |
| `cdp_capabilities() -> Vec<CdpCapability>`, `is_chromium_family(process_name)`, `probe_chromium_cdp(...)`, `attach_chromiumoxide(endpoint)` | Chromium DevTools Protocol integration |
| `CdpDiagnostics`, `CdpStatus`, `CdpCapability`, `CdpAttachment` | diagnostic types for the CDP path |
| `runtime_id_hex(runtime_id: &[i32]) -> String` | encodes a UIA runtime id as hex |

### 3.2 Event coalescing

`coalesce_events(events, window)` and `debounce_value_changes(events, window)`:
- Iterate events in order.
- For each event, drop preceding events on the same `element_id` + `kind` within `window` (typically 50 ms for value changes, 16 ms for focus changes).
- Used by `m3::a11y_events::A11yEventBridge` to keep the SSE bus signal-to-noise high.

### 3.3 COM apartment management

The crate ensures a per-thread COM apartment (`ComApartmentKind`) before any UIA call. Each public function checks for STA initialization and returns `A11yError::NotAvailable` if the calling thread is `Uninitialized`.

## 4. `synapse-perception`

### 4.1 Public surface

| Symbol | Source |
|---|---|
| `PerceptionError`, `PerceptionResult` | `error.rs` (`.code()` → `OBSERVE_INTERNAL`, `OCR_*`, etc.) |
| `ObservationAssembler` | `observe.rs` |
| `ObservationInput` | `observe.rs` — the input struct containing foreground, focused, elements, entities, hud, audio, recent_events, clipboard_summary, fs_recent, and a `mode_override: Option<PerceptionMode>` |
| `ObserveInclude` | `observe.rs` — per-slot booleans + `max_subtree_depth`, `max_subtree_nodes`, `max_entities` |
| `A11yTreeSummary` | `observe.rs` — counts (`total_nodes`, `enabled_nodes`, `focused_nodes`) used by `auto_mode` |
| `assemble`, `assemble_from_input` | `observe.rs` |
| `auto_mode(input) -> PerceptionMode`, `auto_mode_with_a11y(summary, ...)` | `observe.rs` — resolves Auto based on a11y density (≥10 visible enabled nodes → A11yOnly; known game process → PixelOnly; else Hybrid) |
| `bounded_sensor_latency` | `observe.rs` — helper that clamps measured latency for `ObservationDiagnostics` |
| `is_known_game_process(process_name) -> bool` | hardcoded list of "common game / fullscreen render-only process names |
| `parse_perception_mode(&str) -> PerceptionResult<PerceptionMode>` | string parse used by `set_perception_mode` |
| `OcrProvider`, `SystemOcrProvider`, `TextRegion`, `is_empty_region`, `read_text`, `read_text_with_provider` | `ocr.rs` |
| `read_text_from_software_bitmap` (Windows only) | `ocr.rs` |
| `HudAnchor`, `HudAnchorRegion`, `ResolvedHudRegion`, `resolve_anchor_region`, `resolve_hud_region`, `resolve_hud_region_rect` | `hud/anchor.rs` — resolves profile/compact HUD anchors against a foreground window client rect and returns left/top/right/bottom plus `Rect` readback |
| `ExtractionSource`, `FieldExtraction`, `FieldExtractionRequest`, `extract_field`, `parse_hud_text` | `hud/extractor.rs` — applies per-field HUD confidence thresholds, accepts high-confidence template counters, and falls back to OCR plus parser when template confidence is low |
| `HudTemplate`, `TemplateCounterConfig`, `extract_template_counter_from_region`, `extract_template_counter_from_frame` | `template_match.rs` — slotted normalized-correlation HUD counter extraction for hearts/hunger-style icon bars |
| `evaluate_event_extensions`, `validate_event_extension`, `validate_event_extensions` | `event_extensions.rs` — profile `EventExtension` validation and synthetic event derivation using `synapse-core::EventFilter` |

`HudAnchorRegion` accepts `none`, `top_left`, `top_right`, `bottom_left`,
`bottom_center`, `bottom_right`, and `center`. Offsets locate the top-left point
of the HUD crop relative to the selected client-rect anchor. For example, a
`1920x1080` client rect plus `bottom_left`, offsets `(8, -32)`, and size
`180x16` resolves to left/top/right/bottom `(8, 1048, 188, 1064)`.
`none`/absolute regions ignore the window and use screen coordinates directly.

### 4.2 `ObservationAssembler::assemble` algorithm

1. Compute the effective `PerceptionMode`: if `input.mode_override.is_some()` use it, else `auto_mode(input)`.
2. For each slot enabled by `ObserveInclude` (defaults: focused, elements, entities, hud, events), include the corresponding fields. Otherwise the slot is left at its default (empty vec / None).
3. Truncate `elements` to `max_subtree_nodes` (default 60, clamp 1..=500) and apply `max_subtree_depth` (clamp ≤ 6). Set `diagnostics.elements_truncated` when truncated.
4. Truncate `entities` to `max_entities` (default 60). Set `diagnostics.entities_truncated`.
5. Compute `diagnostics.assembled_in_ms = started.elapsed().as_secs_f32() * 1000`.
6. Estimate response size: `size_bytes = serde_json::to_vec(&observation).map(|v| v.len() as u32)` and `size_estimate_tokens = size_bytes / 4` (heuristic).
7. Set `diagnostics.sensor_latency_ms` per sensor (bounded via `bounded_sensor_latency`) and `*_status: SensorStatus` (Healthy / DegradedLatency { last_p99_ms } / DegradedSensorFailed { reason_code } / Disabled / Unavailable).
8. Return the `Observation`.

### 4.3 OCR

`OcrProvider` trait:

```rust
pub trait OcrProvider {
    fn read_text(&self, region: Rect) -> PerceptionResult<Vec<TextRegion>>;
}
```

`TextRegion` is `{ text, bbox: Rect, confidence }`. `is_empty_region(rect)` checks `w <= 0 || h <= 0`.

`read_text(region)` picks the default OCR path. On Windows this captures the
screen region to a `SoftwareBitmap` and calls WinRT `Windows.Media.Ocr`. Small
regions are upscaled before recognition and their word bounding boxes are mapped
back to the requested source region. On WSL/Linux it delegates to the configured
Windows host PowerShell OCR bridge; other targets return
`OCR_BACKEND_UNAVAILABLE`.

`read_text_with_provider(provider, region)` lets callers inject a provider for
deterministic fallback-path verification.

`SystemOcrProvider` implements `OcrProvider` by calling `read_text(region)`,
which is the real WinRT OCR path on Windows.

Windows-only `read_text_from_software_bitmap(region, bitmap)` is the lower-level
entrypoint for code paths that already have a `SoftwareBitmap` in hand.

HUD field extraction uses `FieldExtractionRequest { field, screen_region,
region_image, templates, ocr_provider, stale_ms }`. For `TemplateMatch`, it
runs the slotted template counter with a permissive threshold, compares the
aggregate confidence against `HudFieldSpec.confidence_threshold` (default
0.85), and returns a numeric `HudReading` when accepted. If the template
confidence is below threshold, it calls the supplied OCR provider over the same
screen region, joins recognized words, applies the `HudParser`, and returns
`ExtractionSource::OcrFallback`. If OCR is empty, below threshold, or
unparseable, the field fails closed as `HUD_EXTRACTION_FAILED`.

The live MCP `observe` path resolves the foreground profile before assembling
the response. When the request includes the `hud` slot, `observe` loads that
profile's `HudFieldSpec`s, resolves each region against the foreground window
bounds, captures the cropped screen region, and dispatches the configured
extractor. `ColorRatio` fields read the live pixels directly; `TemplateMatch`
fields load profile template assets and fall back to platform OCR over the same
crop; `WinrtOcr` fields use the live HUD text provider directly. On Windows the
live HUD text provider first accepts a UIA element name only when the element
bounds intersect and stay close to the HUD crop, then falls back to platform
OCR. This keeps small text such as Minecraft XP readable without accepting the
whole window title as HUD text or leaking UIA labels into template-only HUD
fields. Per-field failures are reported under `Observation.hud.errors[field_name]` with
`HUD_EXTRACTION_FAILED` instead of silently inventing readings.

## 5. `synapse-mcp/src/m1` glue

### 5.1 `M1State`

```rust
pub struct M1State {
    pub capture_config: CaptureConfig,
    pub capture_generation: u64,
    pub perception_mode: PerceptionMode,
    pub synthetic: Option<ObservationInput>,
    pub force_no_perception: bool,
    pub force_observe_internal: bool,
    pub last_observed_foreground: Option<ForegroundContext>,
}
```

`M1State::from_env`:

- Reads `SYNAPSE_MCP_SYNTHETIC_FIXTURE` (case-insensitive "notepad" → synthetic Notepad observation source) — see `m1/sources.rs::synthetic_notepad_input`.
- Reads `SYNAPSE_MCP_FORCE_NO_PERCEPTION` (`1`/`true`) to make every `observe` call return `OBSERVE_NO_PERCEPTION_AVAILABLE`.
- Reads `SYNAPSE_MCP_FORCE_OBSERVE_INTERNAL` to make every `observe` call return `OBSERVE_INTERNAL`.
- Pulls capture backend preference from `SYNAPSE_CAPTURE_FORCE_DXGI` via `CaptureConfig::with_env_backend`.

`last_observed_foreground` is updated after each successful `observe` call (`server.rs::observe`). This is the SoT consulted by `ensure_act_type_foreground` so `act_type` refuses to type into the wrong window.

### 5.1a Delta-first reality target (#536)

The current `observe` implementation builds full `Observation` values. The
target architecture in #536 keeps that path for baseline/debug/full-audit reads
but adds a delta-first context flow:

- `RealityBaseline` establishes epoch, seq, compact state hash, and physical
  source refs from a bounded full observation plus profile-specific SoTs.
- `RealityDelta` records ordered foreground/focus/HUD/entity/log/action/storage
  changes after that baseline.
- `RealityAudit` periodically re-reads physical SoTs and compares actual state
  to the baseline+delta assumption; drift produces explicit rebase guidance.

Until #537-#542 land, these schemas/tools are planned surfaces. Existing live
FSV must still use the current MCP tools and separate physical SoT readback.

### 5.2 ObserveParams and slot expansion

`ObserveParams`:

| Field | Default | Behavior |
|---|---|---|
| `include: Vec<ObserveSlot>` | `[]` (treated as defaults) | When empty → defaults: `focused, elements, entities, hud, events`. Non-empty list selects exactly those slots. |
| `depth: Option<u32>` | `None` → `2` | Clamped to `..= 6` |
| `max_elements: Option<usize>` | `None` → `60` | Clamped to `1..=500` |
| `since_event_seq: Option<u64>` | `None` | When set, `recent_events` is filtered to those with `seq > since` |

`observe_include(params)` builds the `ObserveInclude` struct used by `ObservationAssembler::assemble`.

### 5.3 FindParams + `find_in_state`

`FindParams`: `query`, `role`, `name_substring`, `automation_id`, `scope (Elements/Entities/Both)`, `limit (1..=20 default 5)`, `in_window` (filter to a window by `ElementId`).

`find_in_state(state, params)`:

1. Fetch current observation input (`current_input` calls `m1/sources::platform_input`).
2. Iterate elements scoring with `m1/search::element_match` (matches role + name substring + automation id; awards a higher score for exact-role + token coverage).
3. Iterate entities with `m1/search::entity_match` (matches class_label substring).
4. Sort by `score` descending. Truncate to `limit`.

### 5.4 `read_text_in_state`

`m1/ocr.rs::read_text_in_state`:

1. If `params.region` is `Some`, capture that region via `synapse_capture::screen_region_to_software_bitmap` and run WinRT OCR via `synapse_perception::read_text_from_software_bitmap`.
2. If `params.element_id` is `Some`, resolve via `synapse_a11y::re_resolve`, compute its bbox, then proceed as above.
3. Else: read the focused element's bbox.
4. Builds an `OcrResult { full_text, words, confidence, region, lang }`.
5. The `backend` param is currently retained for schema stability but does not branch (always WinRT in this build).

### 5.5 `set_capture_target_in_state`

`set_capture_target_in_state(state, params)`:

1. Compute the previous wire target.
2. Build a new `CaptureConfig`:
   - For `Primary`/`Monitor`/`Window` variants, copy as-is.
   - For `ElementWindow { element_id }`, call `element_id.parts()?.hwnd` and set `target = CaptureTarget::Window { hwnd }`.
3. Apply optional `min_update_interval_ms` (force `>= 1`), `cursor_visible`, `dirty_region_only` overrides.
4. `synapse_capture::resolve_capture_target(&config)` (validates the target — returns `CAPTURE_TARGET_INVALID` for unknown monitors or non-existent HWNDs).
5. Stamp `state.capture_config = config`, `state.capture_generation += 1`.
6. Return `SetCaptureTargetResponse { previous, current, generation, backend: "graphics_capture_api" | "dxgi_duplication" }`.

### 5.6 `set_perception_mode_in_state`

`set_perception_mode_in_state(state, params)`:

1. `parse_perception_mode(&params.mode)` (errors with `PERCEPTION_MODE_INVALID`).
2. Stamp `state.perception_mode = mode`.
3. Return `{ previous, mode, rationale }` where rationale is one of `auto_select_by_foreground_and_a11y_density`, `manual_a11y_only`, `manual_pixel_only`, `manual_hybrid`.

### 5.7 `mcp_error` helper

`mcp_error(code: &'static str, message: impl Into<String>) -> ErrorData`:

```rust
ErrorData::new(
    rmcp::model::ErrorCode(-32099),
    message,
    Some(json!({ "code": code })),
)
```

The fixed JSON-RPC code `-32099` is the rmcp custom-error slot; the structured `code` field carries the Synapse error name.

## 6. Cross-cutting integration

| Edge | Direction | Details |
|---|---|---|
| `observe` tool → `M1State` | sync (lock) | Each call uses `current_input` to build a fresh `ObservationInput` (synthetic, forced-error, or platform-derived). |
| `M1State.last_observed_foreground` → `act_type` | sync read | Comparison happens before any keystroke synthesis. |
| `synapse-a11y` events → SSE bus | via `m3::a11y_events::A11yEventBridge` | The bridge subscribes via `subscribe_win_events`, coalesces, and publishes `Event { source: EventSource::A11yWinEvent / A11yUia, kind: <derived>, data: {element_id, ...} }`. Started on first `reflex_runtime()` call (`SynapseService::reflex_runtime` calls `state.ensure_a11y_event_bridge`). |
| `synapse-capture` channel → consumer | bounded crossbeam | Downstream consumers (perception, OCR) `try_recv` and either run inference / OCR on the texture or discard. |

## 7. What is NOT covered

- **CNN object detection.** `synapse-models` ships the `Detector` trait and ONNX session loader, but `M1State` does not invoke detectors in the current build; `entities: Vec<DetectedEntity>` is populated only by synthetic fixtures.
- **Event extension runtime wiring.** `synapse-perception` exposes the
  `event_extensions` evaluator and validator, but the current `observe()` path
  does not yet automatically feed live detection/HUD events through profile
  extensions and publish them. #414 owns the creeper-nearby runtime path.
- **Audio in `Observation`.** The `audio: AudioContext` field is populated only when an audio runtime is initialized and pushing into the observation source (current build leaves it default).
- **Linux/macOS.** All UIA / WinEvent / WinRT OCR paths are `cfg(windows)`; non-Windows builds return `A11Y_NOT_AVAILABLE` / `OCR_BACKEND_UNAVAILABLE`.
