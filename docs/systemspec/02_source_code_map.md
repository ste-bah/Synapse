# 02 — Source Code Map

Source files covered:
- `Cargo.toml`
- every `crates/*/Cargo.toml`
- every Rust file under `crates/`
- `scripts/`

## 1. Workspace layout

```
synapse/
├── Cargo.toml                      # Rust workspace root; lints, profiles, shared dep versions
├── Cargo.lock
├── AGENTS.md                       # Repository agent doctrine; manual FSV is the shipping gate
├── CHANGELOG.md                    # Release notes (M0/M2 tagged, M1 changelog entry implicit)
├── LICENSE-MIT / LICENSE-APACHE    # Dual license
├── README.md                       # Project README and MCP-client quickstart
├── deny.toml                       # cargo-deny config (advisory checks)
├── docs/                           # PRD + impplan + ADRs (see §4)
├── scripts/                        # PowerShell + Bash helper scripts (see §3)
├── tests/                          # Repo-level fixtures only (audio WAV samples)
│   └── fixtures/audio/             # Test-shared WAV fixtures
├── target/                         # Cargo build output (gitignored)
└── crates/                         # 15 workspace member crates (see §2)
```

`firmware/pico-hid/` is referenced in `Cargo.toml::exclude` and reserved for the M4 RP2040 firmware. It is not present in the current tree.

## 2. Crate tree (with per-file one-line descriptions)

### 2.1 `crates/synapse-mcp/` — the MCP server binary

```
crates/synapse-mcp/
├── Cargo.toml                      # Binary crate; depends on every other library crate
└── src/
    ├── main.rs                     # Process entrypoint, clap CLI, telemetry init, stdio/http dispatch
    ├── server.rs                   # SynapseService: ServerHandler + #[tool_router] declaring 69 MCP tools
    ├── server/
    │   ├── action_audit.rs         # CF_ACTION_LOG start/result audit rows with profile/session context
    │   ├── audit_context.rs        # Profile activation/session/event audit context persistence helpers
    │   ├── context.rs              # Shared tool context helpers
    │   ├── everquest_domain.rs     # EverQuest DynamicJEPA domain-pack + typed state/action/outcome transition rows
    │   ├── everquest_guard.rs      # EverQuest planner guard-decision rows
    │   ├── everquest_log.rs        # EverQuest log resolution and compact observation event feed
    │   ├── everquest_map_sensor.rs # EverQuest visible map/current-state/map-file calibration rows
    │   ├── everquest_memory.rs     # EverQuest hazard/safe-area memory and planner consult rows
    │   ├── everquest_outcome.rs    # EverQuest compact outcome log ingestion rows
    │   ├── everquest_route.rs      # EverQuest bounded map/route plan rows
    │   ├── everquest_scorecard.rs  # EverQuest action-prior sample and scorecard rows
    │   ├── everquest_state.rs      # EverQuest current-state row fusion
    │   ├── everquest_surprise.rs   # EverQuest surprise detector MCP router/storage bridge
    │   ├── everquest_surprise/     # Surprise detector model, comparison, validation helpers
    │   │   ├── compare.rs          # Prediction-vs-observation divergence and remediation logic
    │   │   ├── model.rs            # Tool params, response, and compact payload structs
    │   │   └── validation.rs       # Fail-closed params/source-ref normalization
    │   ├── everquest_tools.rs      # EverQuest /loc and chat-input safety tools
    │   ├── everquest_trajectory.rs # EverQuest linked trajectory rows and JSONL provenance export
    │   ├── everquest_world_model.rs # EverQuest approved-prefix world-model tool router/storage methods
    │   ├── everquest_world_model/  # World-model schema, validation, readback helpers, and tests
    │   │   ├── model.rs            # Row/parameter/response structs and constants
    │   │   ├── tests.rs            # World-model unit tests
    │   │   └── validation.rs       # Prefix/key/payload/source-ref validation plus row readback helpers
    │   ├── everquest_world_summary.rs # Compact EverQuest context-injection summary row writer/readback
    │   ├── everquest_world_summary/ # World-summary params, response, validation, and redaction helpers
    │   │   ├── model.rs            # World-summary params, row, response, and provenance structs
    │   │   └── validation.rs       # Summary id/default/source-ref validation and chat redaction
    │   ├── handler.rs              # ServerHandler implementation glue
    │   ├── health.rs               # health subsystem assembly
    │   ├── m1_tools.rs             # M1 tool wrappers
    │   ├── m2_tools.rs             # M2 action tool wrappers
    │   ├── m3_tools.rs             # M3 profile/reflex/storage tool wrappers
    │   ├── m4_tools.rs             # M4 combo/shell/launch tool wrappers
    │   ├── target_policy.rs        # Supported-use target gating and policy evidence
    │   └── tests.rs                # Server-level unit tests
    ├── safety.rs                   # Operator-hotkey handler that disables reflexes + fires release_all
    ├── http/
    │   ├── mod.rs                  # Re-exports http::serve entrypoint
    │   ├── transport.rs            # Axum router, TCP bind, loopback enforcement, /health, /events, /mcp
    │   ├── auth.rs                 # Bearer token (file or env) + constant-time compare + Host/Origin allowlist
    │   ├── session.rs              # Mcp-Session-Id enforcement, idle timeout, initialize-without-session shim
    │   └── sse.rs                  # SseState: subscription map, ring buffer, Last-Event-ID resume, publish/stats
    ├── m1.rs                       # M1State + ObserveParams/FindParams/ReadTextParams/SetCapture/SetPerceptionMode
    ├── m1/
    │   ├── ocr.rs                  # read_text_in_state implementation
    │   ├── search.rs               # element_match + entity_match scoring for `find`
    │   └── sources.rs              # platform_input + synthetic_notepad_input observation source
    ├── m2.rs                       # M2State (emitter actor handle + recording backend + snapshot handle)
    ├── m2/
    │   ├── aim.rs                  # act_aim params, response, MouseMove builder, snap/flick/natural durations
    │   ├── click.rs                # act_click orchestrator (delegates to schema/element/record submodules)
    │   ├── click/element.rs        # UIA invoke-pattern + coordinate-fallback element click
    │   ├── click/record.rs         # RecordingBackend dispatch path for click
    │   ├── click/schema.rs         # ActClickParams/ActClickResponse JSON schemas
    │   ├── click/tests.rs          # In-module click tests
    │   ├── clipboard.rs            # act_clipboard read/write/clear (text vs unicode)
    │   ├── drag.rs                 # act_drag params, MouseDrag builder, drag-distance enforcement
    │   ├── pad.rs                  # act_pad ActPadReport→GamepadReport, ViGEm dispatch, auto-neutral hold
    │   ├── press.rs                # act_press orchestrator (delegates to keys/live/record/schema/tests)
    │   ├── press/keys.rs           # Key name parsing (alpha, modifiers, function keys, scancode toggle)
    │   ├── press/live.rs           # Live ActionHandle dispatch path
    │   ├── press/record.rs         # RecordingBackend dispatch path
    │   ├── press/schema.rs         # ActPressParams/ActPressResponse, default hold_ms=33
    │   ├── press/tests.rs          # In-module press tests
    │   ├── release_all.rs          # release_all_with_handles: snapshot before, execute Action::ReleaseAll, ensure drained
    │   ├── scroll.rs               # act_scroll dy/dx, smooth scheduling (max 120 steps @ 30ms)
    │   └── type_text.rs            # act_type Burst/Linear/Natural dynamics + optional Enter
    ├── m3.rs                       # M3State, M3ServiceConfig, lazy reflex/profile/audio runtime init
    └── m3/
        ├── a11y_events.rs          # Bridges synapse-a11y AccessibleEvent stream into the reflex EventBus
        ├── audio.rs                # audio_tail + audio_transcribe tool implementations
        ├── audit_export.rs         # audit_export_consent_set + audit_export_bundle consent/redaction/export implementation
        ├── permissions.rs          # PermissionGrants, replay path normalization, profile use-scope gate
        ├── profile.rs              # profile_list + profile_activate tool implementations
        ├── profile_authoring.rs    # profile_authoring_* candidate generation/list/inspect/accept/reject/export implementation
        ├── reflex.rs               # reflex_register/cancel/list/history tools + ScheduledReflex construction
        ├── replay.rs               # replay_record: observation + event JSONL writer
        ├── storage.rs              # storage_inspect/_put_probe_rows/_gc_once/_pressure_sample diagnostic tools
        ├── audit_retention.rs      # AUDIT_RETENTION mode for storage_gc_once; report rows in CF_KV
        ├── subscribe.rs            # subscribe + subscribe_cancel tool wrappers around SseState
        └── tests.rs                # M3-level integration scaffolding tests
```

### 2.2 `crates/synapse-core/` — shared types & error codes

```
crates/synapse-core/
├── Cargo.toml
└── src/
    ├── lib.rs                      # Re-exports public types + SCHEMA_VERSION
    ├── defaults.rs                 # SCHEMA_VERSION=1 + reference-host perf budgets
    ├── error_codes.rs              # 105 SCREAMING_SNAKE_CASE error-code pub const strs
    ├── filter.rs                   # EventFilter and DataPredicate matchers
    ├── retention.rs                # Per-CF TTL + soft/hard cap MB defaults (11 CFs)
    └── types.rs                    # All wire-level types (1567 LoC): Action, Observation, Event, Profile, Reflex*, Stored*, Health, etc.
```

### 2.3 `crates/synapse-storage/` — RocksDB wrapper

```
crates/synapse-storage/
├── Cargo.toml
├── build.rs                        # Build script (likely RocksDB feature gating)
└── src/
    ├── lib.rs                      # Db open/put_batch/flush/scan/compact, batcher spawn, pressure state
    ├── cf.rs                       # 11 column family name pub const + ALL_COLUMN_FAMILIES array
    ├── codecs.rs                   # encode_json/decode_json (only persisted codecs allowed; ADR-0001 / RUSTSEC-2025-0141)
    ├── error.rs                    # StorageError enum (OpenFailed, WriteFailed, ReadFailed, SchemaMismatch, encode/decode)
    ├── batch.rs                    # Background Batcher actor that aggregates put_batch into WriteBatchWithIndex flushes
    ├── batch_tests.rs              # Unit tests for batcher
    ├── compaction.rs               # install_ttl_filter for time-keyed CFs
    ├── compaction_tests.rs         # TTL compaction tests
    ├── gc.rs                       # Periodic GC pass (5-min interval), per-CF soft/hard cap eviction
    ├── gc_tests.rs                 # GC unit tests
    ├── pressure.rs                 # Disk-pressure poller (30-s interval), 4 thresholds (2GB/1GB/500MB/200MB)
    ├── pressure_tests.rs           # Disk-pressure unit tests
    └── open_tests.rs               # Open-path unit tests
```

### 2.4 `crates/synapse-reflex/` — sub-frame runtime

```
crates/synapse-reflex/
├── Cargo.toml
└── src/
    ├── lib.rs                      # ReflexRuntime: register/cancel/disable_all_by_operator + audit persistence
    ├── audit.rs                    # write_audit helper that persists StoredReflexAudit into CF_REFLEX_AUDIT
    ├── bus.rs                      # EventBus, SubscriberHandle, SUBSCRIBER_QUEUE_CAPACITY, drop accounting
    ├── conflict.rs                 # REFLEX_STARVED_KIND, STARVATION_AFTER conflict-resolution constants
    ├── error.rs                    # ReflexError enum with .code() mapping to error_codes::REFLEX_*
    ├── kinds/
    │   ├── mod.rs                  # Re-exports the five reflex-kind controllers
    │   ├── aim_track.rs            # AimTrackController: target, EMA smoothing, deadzone, axis lock, track-lost detection
    │   ├── combo.rs                # ComboController: step list with at_ms scheduling
    │   ├── hold_button.rs          # HoldButtonController: held mouse/pad button
    │   ├── hold_lifetime.rs        # HoldLifetimeContext for HoldMove/HoldButton; REFLEX_LIFETIME_EXPIRED_KIND
    │   ├── hold_move.rs            # HoldMoveController: held key set, re_assert option
    │   └── on_event.rs             # OnEvent: filter-matched action firing with recursion guard (MAX_ON_EVENT_FIRINGS_PER_TICK)
    ├── scheduler.rs                # ReflexScheduler: spawn, validate_reflexes, MAX_REFLEX_PRIORITY/MAX_SCHEDULED_REFLEXES
    ├── scheduler_combo.rs          # Combo-specific tick handling
    ├── scheduler_stateful.rs       # Stateful tick driver (held reflexes + lifetime + conflict resolution)
    ├── scheduler_stats.rs          # TickSample + p99_jitter_us
    ├── scheduler_tick.rs           # Per-tick driver loop
    └── scheduler_windows.rs        # Windows-specific scheduler (TIME_CRITICAL thread + MMCSS Pro Audio)
```

### 2.5 `crates/synapse-action/` — input emission

```
crates/synapse-action/
├── Cargo.toml
└── src/
    ├── lib.rs                      # Crate-level re-exports
    ├── click_timing.rs             # DoubleClickTiming + cached_double_click_timing (Win32 system metric)
    ├── clipboard.rs                # OpenClipboard/SetClipboardData/EmptyClipboard text + unicode helpers
    ├── curve.rs                    # sample_curve for AimCurve::Instant/Linear/EaseInOut/Bezier/Natural
    ├── dynamics.rs                 # KeystrokeEvent, ModifierMask, BIGRAMS, sample_typing_schedule
    ├── error.rs                    # ActionError with .code() mapping to error_codes::ACTION_*
    ├── handle.rs                   # ActionHandle (mpsc producer), ACTION_QUEUE_CAPACITY=256, RELEASE_ALL_HANDLE
    ├── hotkey.rs                   # OperatorHotkeyGuard, install_operator_hotkey (Ctrl+Alt+Shift+P low-level hook)
    ├── invoke.rs                   # UIA InvokePattern bridge for element-target clicks
    ├── invoke/dispatch.rs          # invoke_element implementation
    ├── invoke/resolver.rs          # CoordinateFallbackPlan (element bbox center fallback)
    ├── invoke/tests.rs             # InvokePattern unit tests
    ├── rate_limit.rs               # TokenBucket; SOFTWARE_RATE_LIMIT_PER_S, VIGEM_RATE_LIMIT_PER_S
    ├── safety.rs                   # install_panic_hook for action subsystem
    ├── validation.rs               # validate_action, MAX_DRAG_DISTANCE_PX
    ├── emitter.rs                  # ActionEmitter actor entrypoint, ActionStateSnapshot, EmitState
    ├── emitter/
    │   ├── backends.rs             # ActionEmitter::channel_with_backend / spawn_with_backend
    │   ├── dispatch.rs             # Per-action dispatch and held-state mutation
    │   ├── keyboard.rs             # Keyboard hold tracking + auto-release timers
    │   ├── lifecycle.rs            # run / run_with_shutdown_reason main loop
    │   ├── rate_limits.rs          # Per-backend rate-limit application
    │   ├── routing.rs              # Profile-aware Backend::Auto resolution
    │   ├── state.rs                # Snapshot exporter (snapshot_handle)
    │   └── tests/
    │       ├── mod.rs              # Test wiring
    │       ├── auto_release.rs     # Keyboard auto-release timer tests
    │       └── rate_limit.rs       # Token-bucket / rate-limit tests
    └── backend/
        ├── mod.rs                  # ActionBackend trait, BackendResolutionPolicy, ResolvedBackend, resolve_backend
        ├── mouse_coordinates.rs    # Screen→virtual desktop coord conversion
        ├── text_dispatch.rs        # Text-input dispatch (clipboard paste vs synthesized keystrokes)
        ├── hardware.rs             # HardwareBackend public facade
        ├── hardware/keyboard.rs    # Keyboard action helpers
        ├── hardware/keymap.rs      # Synapse key to USB HID Keyboard/Keypad usage mapping
        ├── hardware/keymap_tests.rs # HID usage mapping regression checks
        ├── hardware/mouse.rs       # Relative mouse/button/wheel command encoding
        ├── hardware/pad.rs         # Gamepad report command encoding
        ├── hardware/tests.rs       # HardwareBackend command/state tests
        ├── unavailable.rs          # Fail-closed hardware slot when --hardware-hid is absent
        ├── recording.rs            # RecordingBackend (in-memory event log)
        ├── recording/state.rs      # RecordingBackend internal state
        ├── software.rs             # SoftwareBackend (Windows SendInput)
        ├── software/input.rs       # SendInput-style INPUT struct preparation
        ├── software/keyboard.rs    # Software keyboard down/up + scancode toggle
        ├── software/mouse.rs       # Software mouse move/buttons/wheel
        ├── software/text.rs        # TypeText sampling + scheduling
        ├── software/utils.rs       # Bitmask / coord helpers
        ├── software_non_windows.rs # Compile-stub for non-Windows builds
        ├── vigem.rs                # VigemBackend public façade
        ├── vigem/client.rs         # ViGEm client + pad plug/unplug
        ├── vigem/error.rs          # ViGEm-specific error mapping
        ├── vigem/pad.rs            # PadId allocation
        ├── vigem/reports.rs        # GamepadReport → X360/DS4 report blob conversion
        ├── vigem/state.rs          # ViGEm session/pad state
        └── vigem/tests.rs          # ViGEm unit tests
```

### 2.6 `crates/synapse-perception/` — observation assembler & OCR

```
crates/synapse-perception/
├── Cargo.toml
└── src/
    ├── lib.rs                      # Crate-level re-exports
    ├── error.rs                    # PerceptionError with .code() mapping to OBSERVE_* / OCR_*
    ├── event_extensions.rs         # Profile event_extension validation and derived event emission
    ├── hud/
    │   ├── mod.rs                  # HUD module exports
    │   ├── anchor.rs               # Client-rect HUD anchor resolver to LTRB/Rect
    │   └── extractor.rs            # HUD field extractor: template threshold plus OCR/parser fallback
    ├── observe.rs                  # ObservationAssembler, ObservationInput, ObserveInclude, auto_mode, A11yTreeSummary
    ├── ocr.rs                      # OcrProvider, TextRegion, read_text/read_text_with_provider, WinRT vs CRNN
    └── template_match.rs           # Slotted template HUD counter extraction
```

### 2.7 `crates/synapse-a11y/` — UIA + WinEvent + CDP

```
crates/synapse-a11y/
├── Cargo.toml
└── src/
    └── lib.rs                      # 2087 LoC on main (HEAD `e54ca57`): UIA wrapper, AccessibleEvent, snapshot, find, foreground context, CDP attach
```

(Single-file lib as-shipped at `v0.1.0-m3`. A platform/* module split is in-progress as an M4 Block A.0 carry-over per `docs/impplan/04_m3_reflex_mcp_surface.md` — when it lands, `lib.rs` becomes a 30-LoC re-export surface with logic in `cdp.rs`, `events.rs`, `ids.rs`, `re_resolve.rs`, `snapshot.rs`, `window.rs`, `platform/non_windows.rs`, and `platform/windows/{common,events,resolve,snapshot,window}.rs`. Update this section in the same PR that lands the split.)

### 2.8 `crates/synapse-capture/` — frame capture

```
crates/synapse-capture/
├── Cargo.toml
└── src/
    └── lib.rs                      # 1798 LoC: DXGI + Windows.Graphics.Capture loops, DPI awareness, coord helpers, channel
```

### 2.9 `crates/synapse-audio/` — WASAPI loopback + STT

```
crates/synapse-audio/
├── Cargo.toml
└── src/
    ├── lib.rs                      # AudioRuntime, AudioConfig, DEFAULT_RING_SECONDS=5, AudioEventSink
    ├── error.rs                    # AudioError with .code() mapping to AUDIO_*
    ├── detectors.rs                # DetectorProcessor: VAD/transient/RMS detectors that emit Events
    ├── direction.rs                # Azimuth estimate from stereo magnitude/phase
    ├── loopback.rs                 # WASAPI loopback start, LoopbackHandle, LoopbackStatus
    ├── ring.rs                     # AudioRing (lock-free push), AudioFormat, AudioWindow, DEFAULT_SAMPLE_RATE_HZ, STEREO_CHANNELS
    ├── stt.rs                      # WhisperTinyStt + Transcription
    └── stt/window.rs               # STT window normalization
```

### 2.10 `crates/synapse-profiles/` — TOML profile loader

```
crates/synapse-profiles/
├── Cargo.toml
└── src/
    ├── lib.rs                      # Crate-level re-exports
    ├── error.rs                    # ProfileError + ProfileLoadError with .code() mapping to PROFILE_*
    ├── package/
    │   ├── mod.rs                  # Public package-manifest parse/validate entrypoints
    │   ├── digest.rs               # SHA-256 digest helper and digest comparison
    │   ├── types.rs                # ProfilePackageManifest and nested manifest data types
    │   └── validation.rs           # Fail-closed package metadata validation rules
    ├── parser.rs                   # parse_profile_file, LoadedProfile, ProfileDefaults, ScreenBounds, bundled_profiles_dir
    ├── resolver.rs                 # ForegroundWindow + resolve_active_profile (regex matching against exe/title/steam_appid)
    ├── toml_format.rs              # RawProfile TOML schema (private)
    └── watcher.rs                  # ProfileRuntime (notify watcher, 200ms debounce, ProfileStatus)
```

### 2.11 `crates/synapse-hid-host/`

```
crates/synapse-hid-host/
├── Cargo.toml
└── src/
    ├── discover.rs                 # Synapse Pico serial-port discovery / auto-detect
    ├── error.rs                    # HidError + code mapping
    ├── handshake.rs                # IDENTIFY parsing and expected-version checks
    ├── lib.rs                      # Public exports
    ├── pipeline.rs                 # ACK/NAK pipeline, retries, backpressure
    ├── protocol.rs                 # CRC16 frame encoding/parsing
    ├── reconnect.rs                # Reconnect state machine and snapshots
    ├── telemetry.rs                # GET_TELEMETRY request + snapshot parsing
    └── transport.rs                # Serialport-backed HidGateway
```

### 2.12 `crates/synapse-models/` — ONNX runtime wrapper

```
crates/synapse-models/
├── Cargo.toml                      # features = default, ort, cuda, directml
└── src/
    └── lib.rs                      # ModelDescriptor, Detector trait, DetectionFrame, ort feature-gated session loader
```

### 2.13 `crates/synapse-telemetry/` — tracing + metrics

```
crates/synapse-telemetry/
├── Cargo.toml
└── src/
    ├── lib.rs                      # TelemetryConfig, init_tracing, GcWorker, install_panic_hook, default_log_dir
    └── metrics.rs                  # M3_METRICS array (19 specs), describe_metric, CARDINALITY_LIMIT=1000
```

### 2.14 `crates/synapse-test-utils/` — shared test rig

```
crates/synapse-test-utils/
├── Cargo.toml
└── src/
    ├── lib.rs                      # Re-exports
    ├── fixtures.rs                 # launch_notepad, wait_for_window_title_regex, notepad_process_ids
    └── stdio_mcp_client.rs         # StdioMcpClient: launches synapse-mcp, drives initialize + tools/call
```

### 2.15 `crates/synapse-overlay/` — M5 placeholder

```
crates/synapse-overlay/
├── Cargo.toml
└── src/
    └── main.rs                     # 1 LoC stub; M5 debug overlay target (default-member alongside synapse-mcp)
```

## 3. Helper scripts

| Path | Description |
|---|---|
| `scripts/check-bench-delta.ps1` | Compares two `critcmp` JSON outputs and fails if any tracked benchmark regressed beyond 20% |
| `scripts/check_dep_graph.sh` | Validates the crate dependency graph against the architecture document |
| `scripts/check_docs.ps1` | Validates cross-references in `docs/` |
| `scripts/check_docs_smoke.ps1` | Smoke variant of the docs link check |
| `scripts/clean-runs.ps1` | Cleans local test artifacts (replays, logs, db) |
| `scripts/new-crate.ps1` | Scaffolds a new workspace member crate |

## 4. Documentation tree

| Path | Description |
|---|---|
| `docs/computergames/` | Product Requirements Document (PRD) — 24 numbered files covering architecture, perception, action, reflex, MCP surface, schemas, storage, supported use, hardware HID, perf budget, security, observability, testing, build, roadmap, open questions, research appendix, Luanti benchmark/runbook, profile-registry governance, optional registry protocol, local registry data model, and profile package manifests |
| `docs/impplan/` | Implementation plan — methodology + per-milestone work-item ledger (M0 through M5) + cross-cutting concerns |
| `docs/adr/0001-current-rust-and-dependencies.md` | ADR: pinned to current stable Rust, no MSRV downgrade |
| `docs/adr/0002-rocksdb-primary-storage.md` | ADR: RocksDB chosen over LMDB/sled for primary storage |
| `docs/adr/0003-reflex-recursion-guard.md` | ADR: on-event recursion guard design |
| `docs/adr/0004-reflex-priority.md` | ADR: reflex scheduler priority semantics |
| `docs/adr/0005-multi-monitor-capture-target.md` | ADR: multi-monitor capture target resolution rules |
| `docs/adr/0006-profile-match-precedence.md` | ADR: profile match precedence when multiple profiles match the foreground |
| `docs/adr/0007-per-event-vs-batched-notifications.md` | ADR: per-event notifications over the SSE bus rather than batching |
| `docs/AICodingAgentSuperPrompt.md` | Agent prompt that AGENTS.md references for wake-up context |
| `docs/compressionprompt.md` | Doctrine for compressed implementation-plan authoring |
| `docs/dev-host-hygiene.md` | Configured-host hygiene checklist (toolchain, drivers) |
| `docs/m1_error_throw_map.md` | M1 error code throw-site map |

## 5. Module dependency graph (build-order)

```
synapse-core            (no synapse-* deps; standalone shared types)
  ↑
  ├── synapse-telemetry     (logging + metrics; no synapse-* deps)
  ├── synapse-models        (synapse-core for DetectionBatch)
  ├── synapse-storage       (synapse-core: retention, error_codes)
  ├── synapse-profiles      (synapse-core)
  ├── synapse-a11y          (synapse-core)
  ├── synapse-capture       (synapse-core, synapse-telemetry)
  ├── synapse-audio         (synapse-core, synapse-models{directml})
  ├── synapse-action        (synapse-core; cfg(windows): synapse-a11y, synapse-capture)
  ├── synapse-perception    (synapse-core; cfg(windows): synapse-a11y, synapse-capture, synapse-models)
  ├── synapse-reflex        (synapse-core, synapse-storage, synapse-action)
  └── synapse-hid-host      (synapse-core; serialport + crc16 HID gateway)

synapse-mcp (binary)
  ↑
  └── depends on every library crate above
        +  synapse-test-utils (dev-dep only)

synapse-overlay (binary)
  ↑
  └── no synapse-* deps yet (1 LoC stub)

synapse-test-utils
  ↑
  └── synapse-mcp (for the StdioMcpClient launch path) — dev-only
```

## 6. Entry point traces

### 6.1 `synapse-mcp --mode stdio`

```
main.rs::main
  └─ run()
     ├─ Cli::parse() → Cli (clap)
     ├─ configure_telemetry()  → synapse_telemetry::init_tracing(TelemetryConfig{...})
     ├─ synapse_capture::init_process_dpi_awareness()
     └─ run_stdio(telemetry_guard, cli.m3_config())
        ├─ SynapseService::try_with_m2_shutdown_reason_and_m3_config(...)
        │  ├─ SharedM1State::default()                       (m1.rs::M1State::from_env)
        │  ├─ shared_m2_state_from_env_with_shutdown_reason  (m2.rs)
        │  │  └─ M2State::from_recording_backend_env_with_actor_backend
        │  │     ├─ synapse_action::initialize_double_click_timing_cache
        │  │     └─ synapse_action::ActionEmitter::channel  → spawns emitter actor task
        │  └─ shared_m3_state_from_config_with_shutdown_reason_and_sse_state
        │     └─ M3State::from_parts_with_sse_state  → SseState::with_max_subscriptions
        ├─ synapse_action::install_panic_hook()
        ├─ safety::install_operator_hotkey()  → low-level keyboard hook on Win32
        ├─ rmcp::transport::stdio() → (stdin, stdout) wrapped in CancelOnEofRead
        └─ service.serve_with_ct(...)  → rmcp service loop (rmcp 1.7.0)
            └─ on each tools/call dispatches to tool_router (#[tool_router] in server.rs)
```

### 6.2 `synapse-mcp --mode http`

```
main.rs::main
  └─ http::serve(bind, allow_non_loopback, m3_config)
     └─ transport.rs::serve
        ├─ SocketAddr parsing + loopback enforcement (HTTP_BIND_NON_LOOPBACK_REFUSED)
        ├─ TcpListener::bind
        ├─ SseState::with_max_subscriptions
        ├─ http_service()  → SynapseService::try_with_m2_shutdown_reason_and_sse_state_and_m3_config
        ├─ safety::install_operator_hotkey
        ├─ router(): axum::Router
        │   ├─ GET  /health        → transport.rs::health
        │   ├─ GET  /events        → SseState::open (SSE)
        │   ├─ POST /events        → SseState::publish  (only when SYNAPSE_HTTP_SSE_MANUAL=1)
        │   ├─ GET  /events/stats  → SseState::stats    (only when SYNAPSE_HTTP_SSE_MANUAL=1)
        │   ├─ /mcp/*              → rmcp StreamableHttpService (LocalSessionManager)
        │   ├─ layer: require_mcp_session    (http/session.rs)
        │   └─ layer: require_http_security  (http/auth.rs — Origin/Host + Bearer)
        ├─ axum::serve with graceful_shutdown(shutdown_cancel)
        └─ on SIGINT / Ctrl-Break: cancels shutdown_cancel and connection_closed_cancel
```

### 6.3 Tool invocation: e.g. `tools/call name=reflex_register`

```
rmcp dispatches → SynapseService::call_tool
  └─ tool_router.call(context)
     └─ SynapseService::reflex_register (server.rs)
        ├─ require_m3_permissions("reflex_register", required_permissions_register(params))
        │   └─ m3::permissions::PermissionGrants::first_missing → authorization_error if denied
        ├─ self.reflex_runtime()
        │   ├─ M3State::ensure_reflex_runtime
        │   │   ├─ Db::open(db_path, SCHEMA_VERSION)  → schema sentinel verify
        │   │   ├─ optional run_pressure_check_with_free_bytes_sample
        │   │   └─ ReflexRuntime::spawn_with_config (action_handle, event_bus, scheduler_config)
        │   └─ M3State::ensure_a11y_event_bridge (event_bus)  → A11yEventBridge::start
        └─ register_reflex(&runtime, params)
            ├─ scheduled_reflex_from_params  → builds ScheduledReflex
            ├─ ReflexRuntime::register
            │   ├─ ReflexScheduler::spawn_with_audit_db   → spawns scheduler thread
            │   ├─ write_registration_audit               → encodes JSON into CF_REFLEX_AUDIT
            │   └─ Db::flush
            └─ returns ReflexRegisterResponse{ reflex_id, state }
```

### 6.4 Tool invocation: `tools/call name=observe`

```
SynapseService::observe
  └─ self.m1_state()  (Mutex<M1State>)
     └─ assemble_observation(&state, &params)
        ├─ current_input(state, depth)
        │   ├─ if state.synthetic.is_some()      → returns synthetic fixture
        │   ├─ if state.force_no_perception      → OBSERVE_NO_PERCEPTION_AVAILABLE
        │   ├─ if state.force_observe_internal   → OBSERVE_INTERNAL
        │   └─ otherwise: m1/sources.rs::platform_input(depth, perception_mode)
        │       ├─ synapse_a11y::current_foreground_context
        │       ├─ synapse_a11y::focused_element (optional)
        │       ├─ synapse_a11y::snapshot(root, depth)  (A11y tree)
        │       └─ assembles ObservationInput { foreground, elements, entities, ... }
        └─ ObservationAssembler::new().assemble(ObserveInclude::default(), input)
           └─ returns synapse_core::Observation
```

## 7. Build / package configuration

| File | Purpose |
|---|---|
| `Cargo.toml` (workspace) | Declares 15 workspace members (`crates/synapse-*`), default-members `[synapse-mcp, synapse-overlay]`, `exclude = ["firmware/pico-hid"]`. Workspace package metadata: `version = "0.1.0"`, `edition = 2024`, `rust-version = "1.95"`, `license = "MIT OR Apache-2.0"`, `repository = "https://github.com/ChrisRoyse/Synapse"`. |
| `Cargo.toml [workspace.dependencies]` | Pins all third-party deps (38 entries) at the workspace level; child crates use `<dep>.workspace = true` |
| `Cargo.toml [workspace.lints.rust]` | `unsafe_code = forbid`, `unused = warn` |
| `Cargo.toml [workspace.lints.clippy]` | `all = deny`, `pedantic/nursery = warn`, `unwrap_used = deny`, `expect_used = deny` |
| `Cargo.toml [profile.dev]` | `opt-level=0`, line-only debug, incremental |
| `Cargo.toml [profile.release]` | `opt-level=3`, `lto="thin"`, `codegen-units=16`, `strip=true`, `panic="abort"` |
| `Cargo.toml [profile.release-max]` | inherits release, `lto="fat"`, `codegen-units=1` |
| `Cargo.toml [profile.bench]` | inherits release with line-only debug |
| Per-crate `Cargo.toml` | All workspace members override `[lints.rust]::unsafe_code` to `allow` only in: `synapse-action`, `synapse-a11y`, `synapse-capture`, `synapse-audio`, `synapse-hid-host` (anywhere doing FFI). Default-binary in `synapse-mcp/Cargo.toml`: `[[bin]] name = "synapse-mcp" path = "src/main.rs"`. Bench harness flags (`harness = false`) declare each `criterion` bench. |
| `deny.toml` | cargo-deny configuration (advisory/licensing checks). Repo policy bans GitHub Actions as a shipping gate (`AGENTS.md`), so this is used locally only. |

The release binary path on Windows is `target/release/synapse-mcp.exe` (linux/wsl variant `target/release/synapse-mcp`).

## 8. Test files (integration tests; per-crate `tests/` directories)

See [14_test_suite.md](14_test_suite.md) for the full inventory and per-file test counts. Summary: 76 `tests/*.rs` files plus 13 `benches/*.rs` files across the workspace.
