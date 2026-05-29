# 02 — Perception Subsystem

## 1. What "perception" means

The perception subsystem turns live machine state into a structured `Observation` value the agent reads with low latency and low token cost.

Two paths, simultaneously active:

| Path | Source of truth | When primary |
|---|---|---|
| **A11y path** | Windows UI Automation tree, WinEvent push, Chromium DevTools Protocol | Foreground app exposes a usable accessibility tree (most native apps, browsers, IDEs, Office) |
| **Pixel path** | GPU frame capture + small CNN + targeted OCR + audio loopback | Foreground is a game, canvas-based app, or window where a11y is empty/insufficient |

`observe()` is the unified read. Both paths feed the same `Observation`; agent picks a query, not a path. The subsystem decides which sensors are cheapest and richest for the current foreground.

Issue #536 changes the target context contract from snapshot-first to
delta-first. `observe()` remains the baseline/debug read, but normal agent
context should become a stream of ordered reality deltas: what foreground,
focus, HUD field, entity, log cursor, storage row, action outcome, or world
state changed since the last cursor. Periodic reality audits re-read full
physical sources of truth and compare them against the delta-guided assumption
so drift becomes an explicit rebase event instead of silent context rot.

Output schemas: `06_data_schemas.md`.

---

## 2. Frame capture (`synapse-capture` crate)

### Strategy

Primary API: **Windows Graphics Capture API** via `windows-capture` crate v2.x. Fallback for older Windows / unsupported configs: **DXGI Output Duplication** via raw `windows-rs` bindings.

Both expose captured frames as `ID3D11Texture2D` in GPU memory. **NEVER copy the texture to system memory** unless an explicit caller (OCR, JPEG export, replay log) requires it. Detection inference runs on the texture directly via DirectML / CUDA.

### Capture targets

Per ADR-0005, each session has exactly one active capture target at a time.
The default is the primary monitor. Agents explicitly switch to another monitor
or window with `set_capture_target`; Synapse does not stitch the virtual desktop
or run concurrent per-monitor captures in M3.

| Target | Use |
|---|---|
| Primary monitor | Default; whole-desktop capture |
| Specific monitor by index | Multi-monitor setups; profile-selectable |
| Specific window by HWND | When agent calls `set_capture_target(window=...)`; reduces VRAM and CPU |

Per-target settings:

- `cursor_visible: bool` — render cursor into frame (default true)
- `secondary_windows: bool` — include child/popup windows (default true for window targets)
- `min_update_interval_ms` — frame-rate cap (default 16ms = 60fps; 33ms for 30fps when GPU-bound)
- `dirty_region_only: bool` — when true, perception skips processing if no dirty pixels (default true)

### Frame lifecycle

```rust
pub struct CapturedFrame {
    pub texture: SendablePtr<ID3D11Texture2D>,   // unsafe Send wrapper; consumer must scope correctly
    pub width: u32,
    pub height: u32,
    pub format: DxgiFormat,                      // typically B8G8R8A8_UNORM
    pub captured_at: Instant,
    pub frame_seq: u64,
    pub dirty_region: Option<Rect>,
}
```

Frames flow through bounded channel (capacity 2). If consumer can't keep up, oldest frame is dropped — NEVER block the capture thread. `frames_dropped` metric is incremented.

### Capture thread

One OS thread for the active capture target. Runs at
`THREAD_PRIORITY_TIME_CRITICAL`. Communicates with tokio via
`crossbeam::channel::bounded(2)`.

```rust
loop {
    let frame = graphics_capture.wait_for_next_frame()?;       // blocks until vsync or 16ms tick
    if !frame.has_changes() && config.dirty_region_only {
        continue;
    }
    if tx.try_send(frame).is_err() {
        metrics.frames_dropped.inc();
    }
}
```

### Fallback path

If Graphics Capture API init fails (unsupported Windows build, no DWM), capture crate falls back to DXGI Output Duplication. Same `CapturedFrame` shape; capture rate may degrade to ~30fps on heavy desktops.

### VRAM and lifetime

- Max two textures resident per capture target (current + previous, for dirty diff).
- D3D resources released on shutdown via RAII (`Drop` impl on `CaptureHandle`).
- Capture continues while no one consumes, at min update interval; no VRAM growth.

---

## 3. Accessibility tree (`synapse-a11y` crate)

### Three subsystems

**3a. UIA tree walker.** Wraps `uiautomation` crate. Provides:

```rust
pub fn focused_window() -> Result<UIElement>;
pub fn focused_element() -> Result<UIElement>;
pub fn element_from_point(x: i32, y: i32) -> Result<UIElement>;
pub fn snapshot(element: &UIElement, depth: u32) -> Result<AccessibleSubtree>;
pub fn find(element: &UIElement, query: AccessibleQuery) -> Result<Vec<UIElement>>;
```

`AccessibleSubtree` is a serializable tree of `AccessibleNode`s with name, role/control-type, AutomationId, bounding rect, enabled, focused, pattern support (Invoke, Toggle, Value, Selection, ExpandCollapse, Scroll, Text), and a stable element id (Synapse-assigned; see `06_data_schemas.md`).

**Depth cap.** Default depth 2 around the focused element + 3 for a window snapshot. Cap is profile-overridable. UIA trees can be enormous (50K+ elements in Outlook); NEVER walk the whole tree without bound.

**Cache request batching.** When snapshotting >100 elements, use `IUIAutomationCacheRequest` to fetch all needed properties in a single cross-process COM round-trip. Per-element property fetch is ~1ms; cached batch fetch is ~10ms for hundreds.

**3b. WinEvent hook.** Win32 `SetWinEventHook` with COM apartment model. Subscribes to:

| Event | Fired when |
|---|---|
| `EVENT_SYSTEM_FOREGROUND` | Foreground window changed |
| `EVENT_OBJECT_FOCUS` | Focus moved (within or between windows) |
| `EVENT_OBJECT_VALUECHANGE` | Editable element value changed |
| `EVENT_OBJECT_NAMECHANGE` | Element name (title) changed |
| `EVENT_OBJECT_CREATE`, `EVENT_OBJECT_DESTROY` | Element appeared / disappeared |
| `EVENT_OBJECT_SELECTION`, `EVENT_OBJECT_SELECTIONADD`, `EVENT_OBJECT_SELECTIONREMOVE` | Selection changed |
| `EVENT_SYSTEM_MENUSTART`, `EVENT_SYSTEM_MENUEND` | Menu opened/closed |
| `EVENT_SYSTEM_ALERT` | Alert/error popup |

Hook callbacks run on COM apartment thread. Marshal events into `tokio::sync::mpsc::Sender<AccessibleEvent>` drained by perception.

**3c. CDP client.** Chrome DevTools Protocol via `chromiumoxide`. Activated when:

- Foreground process is a Chromium-based browser (Chrome, Edge, Brave, Electron apps with debug port)
- Browser launched with `--remote-debugging-port=<N>` OR `chrome.devtools_protocol` reachable

Exposes:

- `DOMSnapshot.captureSnapshot` — full DOM + computed styles + bounds
- `Accessibility.getFullAXTree` — semantic AX tree
- `DOM.querySelector` — CSS selector query
- `Page.captureScreenshot` — fallback when DOM is sparse (canvas-heavy pages)
- `Network.requestWillBeSent` — request stream
- `Console.messageAdded` — console messages

CDP overrides UIA for the affected browser tab when available; richer data, no AX-tree-flattening of Chromium's internal hierarchy.

### Event filter

Synapse coalesces high-frequency a11y events:

- Events sharing `(window_id, element_id, kind)` within a 50ms window merged into one `AccessibleEvent` with latest values
- `EVENT_OBJECT_VALUECHANGE` on an actively-typed text field is debounced to one event per 200ms (or on focus loss, whichever first)

---

## 4. Detection / inference (`synapse-perception::detect`)

When pixel-path perception is needed, run a small object detection model per captured frame.

### Model selection (default v1)

| Model | Size | Latency on RTX 3060 (DirectML) | Latency on RTX 5090 (CUDA) | Use |
|---|---|---|---|---|
| RT-DETRv2-S COCO ONNX | ~81 MB | ~25 ms @ 640px | ~5 ms @ 640px | Default per ADR-0010. License-safe general object detection. |
| YOLOv10n / YOLOv8n | ~6 MB | ~6 ms @ 640px | ~2 ms @ 640px | Operator-import only when the checkpoint is license-compliant and SHA-pinned. |
| YOLOv8s | ~22 MB | ~10 ms | ~3 ms | Optional import for higher accuracy when latency budget permits. |
| Florence-2-base (or similar) | ~480 MB | ~120 ms | ~25 ms | Slow loop only, ≤1 Hz, for unknown-game labeling |

**Selectable per profile.** Productivity profile (Notepad) disables detection
entirely. The Minecraft profile pins `rtdetr_v2_s_coco_onnx`. Future
Minecraft-specific fine-tuned detectors can override the profile model id when
they have license-clean artifacts and SHA-pinned registry rows.

### Inference path

```rust
trait Detector: Send + Sync {
    fn infer(&self, frame: &CapturedFrame, opts: DetectOpts) -> Result<DetectionBatch>;
}
```

Backend selection at startup, in priority order:
1. ONNX Runtime CUDA execution provider (NVIDIA only)
2. ONNX Runtime DirectML execution provider (any DX12 GPU)
3. ONNX Runtime CPU (degraded; only for small models in dev)

All via `ort` crate (Rust bindings to ONNX Runtime). One persistent inference session per loaded model. Reuse pinned input buffers.

### DetectionBatch

```rust
pub struct DetectionBatch {
    pub model_id: String,
    pub frame_seq: u64,
    pub inferred_at: Instant,
    pub items: Vec<Detection>,
}

pub struct Detection {
    pub class_label: String,       // from the model's class map
    pub bbox: Rect,                // pixel coords in capture frame
    pub confidence: f32,
    pub track_id: Option<u64>,     // assigned by simple IoU+name tracker
}
```

### Tracking

Lightweight tracker assigns persistent `track_id`s by IoU + class-name matching across consecutive frames. Track lifetime: a track without a detection for 1000ms is retired. Agent references stable `entity_id = track_id` in subsequent commands without re-querying every frame.

### Model cache

Models live under `%LOCALAPPDATA%\synapse\models\`. The default registry entry
declares `rtdetr_v2_s_coco_onnx`, filename `rtdetr_v2_s_coco.onnx`, its
download URL, and SHA-256. The current runtime fails closed if a model is
missing; setup/import work must acquire the artifact through a license-compliant
local path and then verify SHA before load. NEVER load a model without
verification.

For offline use, operator side-loads: `synapse-mcp models import <path>`.

---

## 5. OCR

Two OCR backends ship at v1:

| Backend | Latency p99 (1080p text region) | When to use |
|---|---|---|
| WinRT `Windows.Media.Ocr` | ~30 ms full screen, ~5 ms small region | Default. No external dep, ships with Windows. Excellent for printed text in most languages. |
| Fine-tuned CRNN (ONNX) | ~10 ms small region, ~80 ms full screen | Game HUD numbers (HP / ammo / score) with unusual fonts; trained per-profile. |

WinRT path uses `windows-rs` bindings to `Windows.Media.Ocr.OcrEngine`. Region cropping on GPU texture; only marshal cropped region to system memory.

### Region targeting

Agent calls:

```
read_text(region={x,y,w,h}) -> { text, words: [{text, bbox, confidence}], language }
```

Default: read focused element's bounds (UIA-resolved) or, no a11y, full screen.

### HUD extraction

Per-game profile defines named HUD regions:

```toml
[[hud]]
name = "minecraft.hp_hearts"
region = { kind = "anchored_to_edge", edge = "bottom_left", x_offset = 220, y_offset = -50, w = 180, h = 18 }
extractor = { kind = "template_match", templates = ["hearts/full.png", "hearts/half.png", "hearts/empty.png"] }
parser = { kind = "number" }
confidence_threshold = 0.85
```

Profile-driven extraction runs in `synapse-perception::hud`. Returns:

```rust
pub struct HudReadings {
    pub by_name: BTreeMap<String, HudReading>,   // e.g. "hp" -> 85
    pub errors: BTreeMap<String, HudFieldError>, // field -> fail-closed diagnostic
}
pub struct HudFieldError {
    pub code: String,
    pub detail: String,
}
pub struct HudReading {
    pub raw_text: String,
    pub parsed: HudValue,   // Number | String | Enum
    pub confidence: f32,
    pub stale_ms: u32,      // how old this reading is
}
```

For icon counters such as Minecraft hearts and hunger, `synapse-perception`
exposes a client-rect anchor resolver plus a slotted template extractor. HUD
anchors can be `top_left`, `top_right`, `bottom_left`, `bottom_right`, or
`center`; `none`/absolute regions use screen coordinates directly. The anchor
resolver takes the foreground window client rect, treats the offset as the
top-left point of the crop relative to the selected anchor, and returns both
`Rect { x, y, w, h }` and explicit left/top/right/bottom readback. Example:
client rect `1920x1080` plus `bottom_left`, offsets `(8, -32)`, and size
`180x16` resolves to `(8, 1048, 188, 1064)`.

The template extractor takes a cropped grayscale HUD region plus
full/half/empty templates and scans each slot with zero-mean normalized
cross-correlation. The default status-bar config is 10 slots, full=2, half=1,
empty=0, and max value 20. Each `HudFieldSpec` carries a
`confidence_threshold` defaulting to 0.85. `synapse-perception::hud::extract_field`
runs template counters permissively, accepts readings whose aggregate
confidence meets the field threshold, and falls back to WinRT OCR plus the
profile parser when template confidence is lower. The default
`SystemOcrProvider` uses the real platform OCR path (`Windows.Media.Ocr` on
Windows). If OCR produces no parseable value or stays below the field threshold,
the extractor fails closed with `HUD_EXTRACTION_FAILED`.

The MCP `observe` path runs these profile HUD extractors when the foreground
profile resolves and the request includes `hud`. It resolves profile regions
against the foreground window, captures the crop, loads template assets when
needed, and reports either `hud.by_name[field]` or
`hud.errors[field] = HUD_EXTRACTION_FAILED`.

Synthetic 180x16 regions with three 9x9 templates measure under the HUD budget
locally; real configured-host manual verification must still inspect the
separate game/window/profile/storage sources of truth after an `observe`
trigger.

### OCR cache

`CF_OCR_CACHE` keyed by `sha256(cropped_region_bytes)` → `OcrResult`. TTL 1h. Hit rate on stable HUDs (HP/ammo) is high; rebuilds drop from 30 ms to ~0.1 ms.

---

## 6. Audio (`synapse-audio` crate)

### Loopback capture

Uses the system-default audio render device through WASAPI loopback on Windows. The M3 MCP audio tools require `--enable-audio` / `SYNAPSE_ENABLE_AUDIO=true`; the runtime is initialized lazily when `audio_tail` or `audio_transcribe` needs it. When audio is enabled, loopback capture starts by default unless `SYNAPSE_AUDIO_LOOPBACK=0` / `false`. Buffers the last N seconds in a ring buffer; M3 supports 1-5 seconds and defaults to 5s.

```rust
pub struct AudioConfig {
    pub ring_seconds: u32,
    pub start_loopback: bool,
    pub detectors_enabled: bool,
    pub stt_model_path: Option<PathBuf>,
}

impl AudioRuntime {
    pub fn spawn(config: AudioConfig) -> AudioResult<Self>;
    pub fn tail_seconds(&self, seconds: f32) -> AudioResult<AudioWindow>;
    pub fn estimate_direction_tail(&self, seconds: f32) -> AudioResult<DirectionEstimate>;
    pub fn transcribe_tail(&self, seconds: f32, language: impl AsRef<str>) -> AudioResult<Transcription>;
    pub fn detector_snapshot(&self) -> DetectorSnapshot;
    pub fn loopback_status(&self) -> LoopbackStatus;
}

pub fn start_loopback(ring: Arc<AudioRing>, detectors: Option<DetectorProcessor>) -> AudioResult<LoopbackHandle>;
```

`AudioWindow` stores interleaved f32 samples with the active loopback format. The MCP `audio_tail` response converts the requested window to padded `s16le`, reports the sample rate and channel count, and returns an empty PCM buffer for `seconds = 0` without initializing the runtime.

### STT

Whisper-tiny-int8 ONNX is the pinned M3 model family. It runs on demand when the agent calls `audio_transcribe()`. The tool accepts only English (`"en"` or empty/default), transcribes the requested ring window, and returns `{ text, confidence, latency_ms, model_id: "whisper_tiny_int8" }`.

Continuous transcription and per-profile transcription flags are not live M3 behavior; adding them requires a later profile/runtime change.

### Spatial direction estimate

Naive method: compute interleaved L/R energy ratio + GCC-PHAT-style cross-correlation lag. Returns:

```rust
pub struct DirectionEstimate {
    pub azimuth_deg: f32,          // 0 = front, +90 = right, -90 = left
    pub confidence: f32,
}
```

Sufficient for FPS footstep direction at v1. Steam Audio (audionimbus crate) is the v2 upgrade for HRTF-accurate spatial localization.

### Audio events

When `detectors_enabled` is true, the loopback thread runs the current heuristic detector processor and publishes events to the event bus. M3 wires the MCP runtime with detectors disabled by default; detector state is still exposed through `detector_snapshot()` for runtime paths that enable it.

| Event | Trigger heuristic |
|---|---|
| `loud_transient` | RMS exceeds 5× moving average (gunshots, impacts) |
| `speech_started` | RMS crosses the speech start threshold |
| `speech_ended` | RMS stays below the speech end threshold for 500ms |
| `music_started` / `music_ended` | RMS plus crest-factor heuristic |

Events pushed to the event bus.

---

## 7. Event derivation

Both paths emit events:

| Event source | Examples |
|---|---|
| WinEvent | `focus_changed`, `window_opened`, `value_changed`, `selection_changed` |
| UIA structure | `element_appeared`, `element_disappeared` (computed by diffing snapshots) |
| CDP | `dom_mutation`, `navigation_committed`, `network_request_failed` |
| Detection | `entity_appeared` (new track), `entity_disappeared` (track timed out), `entity_class_changed` |
| HUD OCR | `hud_value_changed` (HP went from 100 → 85) |
| Audio | `loud_transient`, `speech_started`, `loud_transient_directional` |
| Filesystem | `file_created`, `file_changed`, `file_deleted` (via `notify` crate) |
| Process | `process_started`, `process_exited`, `socket_opened` |
| Clipboard | `clipboard_changed` |

All events normalized to `synapse-core::Event`:

```rust
pub struct Event {
    pub seq: u64,                    // monotonic across all sources
    pub at: SystemTime,
    pub source: EventSource,
    pub kind: EventKind,
    pub data: serde_json::Value,
    pub correlations: Vec<EventRef>, // e.g., "this hud_value_changed correlates with that entity_class_changed"
}
```

Full schema in `06_data_schemas.md`. Persistence in `CF_EVENTS` (`07_storage_and_profiles.md`).

Profile `event_extensions` are evaluated in `synapse-perception` after a real
event is produced. Each matching extension emits a new
`EventSource::Perception` event with `kind = emits_kind`, a correlation back to
the triggering event sequence, and compact readback data containing the
extension name and triggering event fields. Registration fails closed for
invalid filters or trivially always-true filters so a profile cannot turn every
incoming event into unbounded synthetic traffic.

---

## 8. The unified Observation

`observe()` returns this struct (JSON-serialized):

```rust
pub struct Observation {
    pub at: SystemTime,
    pub mode: PerceptionMode,                   // a11y_only | pixel_only | hybrid
    pub foreground: ForegroundContext,
    pub focused: Option<FocusedElement>,
    pub elements: Vec<AccessibleNode>,          // truncated by depth/N
    pub entities: Vec<DetectedEntity>,          // CNN detections with track ids
    pub hud: HudReadings,                       // game-only
    pub audio: AudioContext,                    // recent events + direction estimate if any
    pub recent_events: Vec<Event>,              // since last observe(), capped
    pub clipboard_summary: Option<ClipboardSummary>,
    pub fs_recent: Vec<FsEvent>,                // last 5 file changes
    pub diagnostics: ObservationDiagnostics,    // perception health (which sensors active, latencies)
}
```

Tokenization budget: aim for `serde_json::to_string(&observation)` ≤ 6 KB (≈ 1500 tokens) under typical conditions. Larger queries require explicit `expand(slot=...)` calls.

---

## 8.1 Delta-first reality model

The target architecture for M4+ is:

1. Capture a `RealityBaseline` from a bounded full `Observation` plus any
   profile-specific physical SoTs such as logs, files, process/window state, or
   storage rows.
2. Emit append-only `RealityDelta` records for changes after that baseline.
   Deltas carry epoch, seq, source, kind, field/path, compact before/after
   values, confidence, and source refs.
3. Let the agent maintain a working assumption by applying deltas instead of
   ingesting a fresh full snapshot on every turn.
4. Periodically run `RealityAudit`: re-read the physical SoTs, compare actual
   state with the assumed baseline+delta state, persist drift findings, and
   force `rebase_required` when the assumption is stale or contradictory.

Full snapshots are still required for initialization, explicit expansion,
debugging, and FSV. Delta payloads are the routine context feed because they are
smaller and more useful: they tell the agent what changed and why it matters.

Tracked work: #536 (context), #537 (schemas), #538 (MCP tools), #539
(perception delta generation), #540 (storage rows), #541 (EverQuest
integration), #542 (manual FSV runbook), and #543 (registry quality signals).

---

## 9. Perception mode auto-selection

```
on foreground change:
    profile = profiles::detect(hwnd)
    if profile.is_some():
        mode = profile.preferred_mode
    else:
        a11y_tree_size = a11y::snapshot(focused_window, depth=1).len()
        if a11y_tree_size > 5:
            mode = a11y_only
        else:
            mode = hybrid
    apply(mode):
        if a11y_only: stop pixel capture; keep UIA + WinEvent
        if pixel_only: stop UIA event handlers; keep capture + audio
        if hybrid: both
```

Profile-pinned mode wins over heuristics.

---

## 10. Health and degraded modes

| State | Meaning | Recovery |
|---|---|---|
| `healthy` | All requested sensors active and within latency budget | n/a |
| `degraded_latency` | One or more sensors exceeded latency budget recently | Auto-retry; surface in `diagnostics` |
| `degraded_sensor_failed` | A sensor crashed or returned errors persistently | Disable that sensor; surface in `diagnostics`; agent can `observe()` to see what's missing |
| `unavailable` | All sensors failed | Return `OBSERVE_NO_PERCEPTION_AVAILABLE` error from `observe()` |

Sensors recover independently. UIA failure does not stop pixel capture, and vice versa.

---

## 11. CPU / GPU budget

| Subsystem | Steady-state CPU | Steady-state GPU | VRAM |
|---|---|---|---|
| Capture (idle, no consumers) | ~0.1% | ~0% (texture shared with DWM) | ~10 MB |
| Capture (60 fps, consumer attached) | ~2% | ~2% | ~40 MB |
| Detection async loop (RT-DETRv2-S COCO) | ~5% | ~20% on RTX 5090 | ~350 MB |
| UIA event subscriber | ~1% | 0 | ~10 MB |
| Audio loopback + ring buffer | ~0.5% | 0 | ~5 MB |
| OCR (on demand, region) | spike to ~5% for ~5 ms | spike to ~10% | ~50 MB |

Total ceiling at v1: 10% CPU, 25% GPU, 1.5 GB VRAM when fully active. Idle: <2% CPU, <1% GPU, <500 MB VRAM.

---

## 12. Coordinate systems

| System | Origin | Used in |
|---|---|---|
| Screen (virtual desktop) | Top-left of the union of all monitors | Mouse moves, top-level rect math |
| Monitor (per-monitor) | Top-left of one monitor | Per-target capture coords |
| Window client | Top-left of a specific window's client area | A11y bounding rects when window-relative |
| Capture frame | Top-left of captured texture | Detection bboxes |

HUD anchored regions resolve against the foreground window client rect after it
has been translated to screen coordinates. Returned tuples use
left/top/right/bottom with the bottom-right edge exclusive; `Rect` keeps the
same region as `x/y/w/h`.

DPI: all coordinates use **physical pixels** (per-monitor-aware v2 DPI scaling). Synapse calls `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)` at startup.

Transforms exposed:

```rust
pub fn screen_to_window(screen: Point, hwnd: HWND) -> Point;
pub fn window_to_screen(window: Point, hwnd: HWND) -> Point;
pub fn frame_to_screen(frame: Point, target: &CaptureTarget) -> Point;
```

All bounding rects in `Observation` are screen coordinates unless explicitly tagged.

---

## 13. Failure surfaces

| Failure | Error code | Recovery |
|---|---|---|
| Graphics Capture unsupported | `CAPTURE_GRAPHICS_API_UNSUPPORTED` | Fall back to DXGI Output Duplication |
| Capture target HWND no longer valid | `CAPTURE_TARGET_LOST` | Surface to caller; agent picks a new target |
| UIA returned a stale element | `A11Y_ELEMENT_STALE` | Re-snapshot the parent |
| Detection model not loaded | `DETECTION_MODEL_NOT_LOADED` | Surface to caller; either operator imports model or perception runs without detection |
| OCR failed (no text found) | `OCR_NO_TEXT` | Not an error from agent's perspective; observation returns empty text |
| Audio device disconnected | `AUDIO_DEVICE_LOST` | Re-enumerate devices; resume on next default device |
| Per-app profile not found | `PROFILE_NOT_FOUND` | Use hybrid default |

Every error code exported as `pub const` in `synapse-core::error_codes`.

---

## 14. Out of scope for this doc

- Per-tool MCP schemas → `05_mcp_tool_surface.md`
- Storage layout → `07_storage_and_profiles.md`
- Profile TOML format → `07_storage_and_profiles.md`
- Specific dependency versions → `14_build_and_packaging.md`
- Performance budgets per stage → `10_performance_budget.md`
- Event JSON shapes → `06_data_schemas.md`
