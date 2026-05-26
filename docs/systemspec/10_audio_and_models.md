# 10 — Audio Runtime & Model Loader (`synapse-audio`, `synapse-models`)

Source files covered:
- `crates/synapse-audio/src/lib.rs`
- `crates/synapse-audio/src/error.rs`
- `crates/synapse-audio/src/loopback.rs`
- `crates/synapse-audio/src/ring.rs`
- `crates/synapse-audio/src/detectors.rs`
- `crates/synapse-audio/src/direction.rs`
- `crates/synapse-audio/src/stt.rs`
- `crates/synapse-audio/src/stt/window.rs`
- `crates/synapse-models/src/lib.rs`
- `crates/synapse-mcp/src/m3/audio.rs`

## 1. `synapse-audio`

### 1.1 Public surface

```rust
pub const DEFAULT_RING_SECONDS: u32 = 5;
pub const MAX_RING_SECONDS: u32 = 5;

pub type AudioEventSink = Arc<dyn Fn(Event) + Send + Sync + 'static>;

pub struct AudioConfig {
    pub ring_seconds: u32,            // 1..=MAX_RING_SECONDS
    pub start_loopback: bool,
    pub detectors_enabled: bool,      // requires start_loopback
    pub stt_model_path: Option<PathBuf>,
}

pub struct AudioRuntime { ... }
```

Re-exports: `AudioError`, `AudioResult` (error.rs); `LoopbackStatus` (loopback.rs); `AudioFormat`, `AudioRing`, `AudioWindow` (ring.rs); `Transcription`, `WhisperTinyStt` (stt.rs); `DirectionEstimate` (re-exported from `synapse-core`).

### 1.2 `AudioRuntime::spawn(config)` / `spawn_with_event_sink(config, sink)`

1. `validate_config`:
   - `ring_seconds` must be in `1..=MAX_RING_SECONDS = 5`; else `AudioError::LoopbackInitFailed`.
   - `detectors_enabled = true && start_loopback = false` → `AudioError::LoopbackInitFailed` ("audio detectors require loopback startup").
2. Build `AudioRing::new(ring_seconds)` — see §1.4.
3. If `start_loopback`, call `loopback::start_loopback(ring, optional DetectorProcessor)`:
   - Opens the default render endpoint via `wasapi` (WASAPI loopback).
   - Spawns a high-priority capture thread that reads PCM frames, pushes them into the ring, and (if detectors are enabled) feeds them into the `DetectorProcessor` which emits `Event`s to `event_sink`.
   - Returns a `LoopbackHandle` with status tracking.
4. Initialize `WhisperTinyStt::new(stt_model_path)`. The model is not loaded yet — load is deferred to first `transcribe_window` / `transcribe_file` call.
5. Return `AudioRuntime { config, ring, detector_state, loopback, stt }`.

### 1.3 AudioRuntime methods

| Method | Behavior |
|---|---|
| `config() -> &AudioConfig` | borrow config |
| `loopback_started() -> bool` | true iff the WASAPI thread is alive |
| `detectors_started() -> bool` | `config.detectors_enabled && loopback_started()` |
| `ring() -> Arc<AudioRing>` | shares the ring (used by `audio_tail`) |
| `tail_seconds(seconds: f32) -> AudioResult<AudioWindow>` | reads the most-recent N seconds of PCM (seconds outside `1..=ring_seconds` → `LoopbackInitFailed`) |
| `estimate_direction_tail(seconds) -> AudioResult<DirectionEstimate>` | computes azimuth from stereo magnitude+phase over the window |
| `transcribe_tail(seconds, language) -> AudioResult<Transcription>` | runs Whisper-tiny over the ring tail |
| `transcribe_file(path, language) -> AudioResult<Transcription>` | runs Whisper-tiny over a WAV file (used by integration tests against `tests/fixtures/audio/*.wav`) |
| `detector_snapshot() -> DetectorSnapshot` | last detector decision (RMS, VAD, transient, direction) |
| `loopback_status() -> LoopbackStatus` | `{ running, frames_captured, last_error_code: Option<&str> }` |
| `stt_model_loaded() -> bool` | true iff Whisper has loaded successfully |

### 1.4 Ring buffer (`ring.rs`)

`AudioRing`:

- Holds a `Vec<f32>` capable of storing `ring_seconds * DEFAULT_SAMPLE_RATE_HZ * STEREO_CHANNELS` samples (default `5 * 48000 * 2 = 480 000`).
- Lock-free `push_interleaved(samples: &[f32])` updates an `AtomicUsize` write index; readers (`tail_seconds`) snapshot the index and copy out the trailing N frames.
- `set_format(AudioFormat)` updates the negotiated `{ sample_rate_hz, channels }` (default `48000 Hz, 2 ch`).

`AudioFormat`:

| Constant | Value |
|---|---|
| `DEFAULT_SAMPLE_RATE_HZ` | `48_000` |
| `STEREO_CHANNELS` | `2` |

`AudioWindow`:

```rust
pub struct AudioWindow {
    pub samples: Vec<f32>,
    pub format: AudioFormat,
    pub generation: u64,
}
```

`pcm_i16_le()` converts the f32 samples to little-endian s16 PCM bytes for transport across the MCP `audio_tail` response.

### 1.5 Loopback (`loopback.rs`)

- Uses `wasapi 0.23` to open the system default render endpoint in **loopback** mode and a `EventCallback`-driven capture loop.
- The thread writes PCM frames into `AudioRing::push_interleaved` and increments `frames_captured`.
- On WASAPI failure, emits a `tracing::warn` with `code = AUDIO_DEVICE_LOST` (or `AUDIO_LOOPBACK_INIT_FAILED` on setup failure) and records the code into `LoopbackStatus.last_error_code`.

### 1.6 Detectors (`detectors.rs`)

`DetectorProcessor` is an optional consumer that observes each pushed buffer and emits events:

- **RMS** detector emits `EventSource::PerceptionAudio` events of `kind = "audio.rms"` when the running RMS crosses a threshold.
- **VAD** (voice activity) detector flags spoken speech segments.
- **Transient** detector raises `kind = "audio.transient"` on sudden energy spikes.
- All events go through the caller-supplied `AudioEventSink`.

`DetectorSnapshot` captures the current detector state (last RMS dB, last VAD bool, last transient time).

### 1.7 Direction estimate (`direction.rs`)

`estimate_direction(window: &AudioWindow) -> DirectionEstimate`:

1. Stereo decorrelation: compute left-channel and right-channel magnitudes and inter-channel phase difference over the window.
2. Map left-vs-right magnitude ratio to a left-right azimuth in `[-90°, +90°]`.
3. Confidence = `1 - normalized_variance_of_estimates`.

Returns `DirectionEstimate { azimuth_deg: f32, confidence: f32 }` (also a public field on `AudioContext`).

### 1.8 STT (`stt.rs`, `stt/window.rs`)

`WhisperTinyStt`:

- `new(stt_model_path: Option<PathBuf>)` — constructs a lazy-load wrapper. Loading is deferred to first transcribe call.
- `is_loaded() -> bool` — true iff Whisper-tiny has been loaded and validated.
- `transcribe_window(&AudioWindow, language: impl AsRef<str>) -> AudioResult<Transcription>`:
  - Validates language string ("en" only in current build; the M3 audio tool wrapper enforces this at the MCP boundary too).
  - Normalizes the window samples to 16 kHz mono via `stt/window::resample_mono`.
  - If the model file at `stt_model_path` (or the default path) is missing → `AudioError::SttModelNotLoaded` → `AUDIO_STT_MODEL_NOT_LOADED`. Silent input returns `Transcription { text: "", confidence: 0.0, elapsed_ms: 0 }` without invoking ORT (see test `transcribe_maps_silence_without_model_load_and_rejects_language` in `m3/audio.rs`).
  - Otherwise verifies the SHA-256 of the model file against the pinned digest → `MODEL_HASH_MISMATCH` on drift.
  - Loads via `ort` (with `directml` feature in this crate's Cargo.toml) and runs inference.
  - Returns `Transcription { text: String, confidence: f32, elapsed_ms: i64 }`.
- `transcribe_file(path, language)` — same pipeline, but loads the WAV file directly. Used by integration tests against `tests/fixtures/audio/hello_world_5s.wav`, `loud_transient_1s.wav`, `pan_minus60_0_plus60.wav`.

### 1.9 Errors (`error.rs`)

`AudioError::code()` → `AUDIO_DEVICE_LOST` / `AUDIO_LOOPBACK_INIT_FAILED` / `AUDIO_STT_MODEL_NOT_LOADED` / `MODEL_HASH_MISMATCH` / `MODEL_LOAD_FAILED` / `MODEL_BACKEND_UNAVAILABLE`.

## 2. `synapse-models`

### 2.1 Crate features

`crates/synapse-models/Cargo.toml`:

```toml
[features]
default = []
ort = [
    "dep:ort",
    "ort/api-24",
    "ort/copy-dylibs",
    "ort/download-binaries",
    "ort/std",
    "ort/tls-native",
]
cuda = ["ort", "ort/cuda"]
directml = ["ort", "ort/directml"]
```

`synapse-audio` enables `directml`. `synapse-mcp` does not pull in CUDA or DirectML features explicitly; the configured-host install/setup path is responsible for ensuring the ONNX runtime DLL is present. If it is missing during issue work, the agent must acquire or configure it through local reversible workflows where possible and then read the physical DLL/path/source-of-truth directly.

### 2.2 Public surface

| Type | Definition |
|---|---|
| `ModelDescriptor` | `{ id: String, path: PathBuf, sha256: String, input_shape: Vec<usize>, class_map: Vec<String> }`. `yolov10n_general(sha256, class_map)` ctor produces the canonical YOLOv10-nano descriptor with `path = default_model_dir().join("yolov10n_general.onnx")` and `input_shape = vec![1, 3, 640, 640]`. |
| `ModelBackend` | `Cuda` \| `DirectMl` \| `Cpu` (default) |
| `DetectOpts` | `{ confidence_threshold: u16 (default 50), max_detections: usize (default 100) }` |
| `DetectionFrame` | `{ frame_seq: u64, width: u32, height: u32 }`. `validate()` returns `DETECTION_NO_FRAME` for zero dimensions. |
| `Detector` (trait) | `fn infer(&self, frame: DetectionFrame, opts: DetectOpts) -> ModelResult<DetectionBatch>` |
| `ModelError` | thiserror enum with variants `DownloadFailed` / `HashMismatch` / `LoadFailed` / `BackendUnavailable` / `NoFrame` / `InferenceFailed`; `.code()` → `MODEL_*` / `DETECTION_*` |

### 2.3 Model loader behavior

When the `ort` feature is enabled:

1. `Detector::load(descriptor: &ModelDescriptor)` reads the file bytes.
2. Computes SHA-256 (`sha2::Sha256`) and compares against `descriptor.sha256`. Mismatch → `ModelError::HashMismatch` → `MODEL_HASH_MISMATCH`.
3. Constructs an `ort::Session` with the configured `ModelBackend` (`Cpu` if no acceleration feature is enabled).
4. Per-inference: validates the frame, runs the session, decodes the YOLOv10 output tensor into `Vec<Detection>` filtered by `confidence_threshold/100` and capped at `max_detections`.

When `ort` is not enabled, all `Detector::load` paths return `ModelError::BackendUnavailable` → `MODEL_BACKEND_UNAVAILABLE`.

### 2.4 Session IDs

The crate maintains a process-wide `AtomicU64 NEXT_SESSION_ID = 1` used to label model sessions in tracing logs (`crate::lib.rs:15`). This is unrelated to `SessionId` in `synapse-core::types`.

### 2.5 Default model directory

`default_model_dir()` (per source: helper near the bottom of `lib.rs`) is the directory used for the bundled-on-demand model cache. The actual download/cache mechanism is not implemented in the current build — `MODEL_DOWNLOAD_FAILED` is reserved.

## 3. MCP audio tools (`crates/synapse-mcp/src/m3/audio.rs`)

| Tool | Behavior |
|---|---|
| `audio_tail(seconds: u32)` | `0..=MAX_RING_SECONDS = 5`. `0` returns an empty PCM body with `sample_rate = 48_000`, `channels = 2`, `format = "s16le"`. Otherwise pulls `AudioRuntime::tail_seconds(seconds)`, converts to little-endian s16 (`AudioWindow::pcm_i16_le`), and **pads with zeros** if the ring has less data than requested (so the returned `pcm.len() == seconds * sample_rate * channels * 2`). |
| `audio_transcribe(seconds: u32, language: String)` | Same `seconds` bounds. `language` accepts `"en"` or empty (mapped to `"en"`); anything else → `TOOL_PARAMS_INVALID`. Calls `AudioRuntime::transcribe_tail`. Returns `{ text, confidence, latency_ms, model_id: "whisper_tiny_int8" }`. |

Both require permission `READ_AUDIO`, which is only granted by default when `--enable-audio` is set (see [03_configuration.md §4.4](03_configuration.md)).

Lazy init: `M3State::ensure_audio_runtime` (`crates/synapse-mcp/src/m3.rs::364`) builds the `AudioRuntime` on first call with:

```rust
AudioConfig {
    ring_seconds: DEFAULT_RING_SECONDS,
    start_loopback: audio_loopback_enabled()?,  // reads SYNAPSE_AUDIO_LOOPBACK
    detectors_enabled: false,                   // detectors are not wired into the M3 tools yet
    stt_model_path: None,                       // uses synapse-audio's default lookup
}
```

`detectors_enabled = false` because no event sink is plumbed in this build (the SSE bus event sink integration is reserved for later work).

## 4. Performance metrics emitted

| Metric | Kind | Labels | Source |
|---|---|---|---|
| `audio_loopback_underruns_total` | counter | — | `loopback.rs` (incremented on missed-deadline reads) |
| `audio_stt_inferences_total` | counter | `outcome` (success/timeout/failure) | `stt.rs` |
| `audio_stt_latency_ms` | histogram | — | `stt.rs` |

## 5. Test fixtures

The repository ships three WAV fixtures (`tests/fixtures/audio/`):

| File | Purpose |
|---|---|
| `hello_world_5s.wav` | English speech sample used by `audio_transcribe` integration tests |
| `loud_transient_1s.wav` | Tests transient detector and RMS clipping |
| `pan_minus60_0_plus60.wav` | Tests `estimate_direction` azimuth output across the stereo field |

See `tests/fixtures/audio/README.md` for the synthesis recipe.

## 6. What is NOT covered

- **STT models other than Whisper-tiny.** The model id is hard-coded `whisper_tiny_int8` in `m3/audio.rs` and only one language ("en") is accepted.
- **No streaming transcription.** `audio_transcribe` returns a complete `Transcription` after running over the buffered tail; there is no incremental streaming API.
- **Model auto-download.** `MODEL_DOWNLOAD_FAILED` is reserved as an error code but there is no download path; when a workflow requires the ONNX file, the agent acquires or imports it on the configured host through a license-compliant local setup path and verifies `synapse-audio::stt::default_model_path()` plus the expected hash directly.
- **Custom audio devices.** WASAPI loopback always uses the default render endpoint; there is no selector for non-default outputs.
- **YOLO inference pipeline.** `synapse-models::Detector::load` works end-to-end but no `M1State` code path runs detection yet (entities are populated only by synthetic fixtures).
