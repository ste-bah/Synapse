# 10 — Performance Budget

## 1. Why this is a binding doc

Synapse exists because screenshot-based agents are too slow. If we drift into 100ms-per-`observe()`, the project has failed its core promise. This doc sets numeric budgets per subsystem and per MCP tool, plus the discipline for enforcing them.

Every PR violating a target either fixes the regression or comes with an explicit ADR amendment.

---

## 2. The end-to-end targets

| Target | Budget | Conditions |
|---|---|---|
| `observe()` request → reply | ≤ 30 ms p99 | hybrid mode, 60-element depth-2, fresh UIA cache miss |
| `observe()` steady-state token size | ≤ 6 KB JSON / 1500 tokens p95 | hybrid mode, default `include` |
| Event push: underlying frame/UIA event → SSE notification at client | ≤ 50 ms p99 | local stdio |
| `act_click(element_id)` semantic invoke → element invoked | ≤ 25 ms p99 | UIA Invoke supported |
| `act_click(x,y)` coordinate click → cursor at target | ≤ 60 ms p99 | EaseInOut curve, 80 ms travel |
| `act_press(key)` → key down on the OS | ≤ 3 ms p99 | software backend |
| `act_press(key)` → key down via hardware HID | ≤ 5 ms p99 | including USB poll |
| Reflex `on_event` matched → action emitted | ≤ 5 ms p99 |  |
| Reflex `aim_track` per-tick adjustment | ≤ 2 ms p99 |  |
| Capture loop frame interval | 16 ms (60 fps) | configurable per profile |
| Detection inference (RT-DETRv2-S COCO, 640px) | ≤ 25 ms p99 DirectML / ≤ 8 ms p99 CUDA | Default model per ADR-0010; runs async from `observe()` latest-result lookup |
| OCR WinRT, single small region | ≤ 8 ms p99 |  |
| Health check `health` tool | ≤ 5 ms |  |

`p99` = 99th percentile over rolling 10-minute window of real operation. Cold-start spikes excluded; budget applies post-warm-up.

---

## 3. CPU budgets

| Subsystem | Idle | Active | Notes |
|---|---|---|---|
| Capture (no consumers, 60 Hz target with no dirty regions) | ≤ 0.1% | n/a | Mostly sleeping |
| Capture with consumer attached (60 fps gameplay) | n/a | ≤ 2% | One thread, time-critical priority |
| A11y subscriber (UIA + WinEvent) | ≤ 1% | ≤ 3% during active app switching | COM apartment thread |
| Perception worker | ≤ 0.2% | ≤ 5% with detection running | When detection is on |
| Detection inference | n/a | GPU-bound (CPU ~1% during inference) | |
| Audio loopback | ≤ 0.5% | ≤ 1% | Real-time thread priority |
| Reflex runtime (1 ms tick, dedicated thread) | ≤ 1% | ≤ 2% with active controllers | TIME_CRITICAL priority |
| Action emitter | ≤ 0.05% | ≤ 0.5% | Mostly blocked on channel |
| Storage write batcher | ≤ 0.1% | ≤ 0.5% | Batches every 100ms |
| MCP transport | ≤ 0.1% | ≤ 1% during burst | tokio multi-thread, 4 workers |
| Telemetry | ≤ 0.05% | ≤ 0.2% | OTLP push every 10 s |
| Total ceiling | ≤ 2% | ≤ 15% | Single 8-core host |

Idle = no agent connected, no profile active. Active = an agent running and exercising the system.

---

## 4. Memory and VRAM budgets

| Subsystem | Idle | Active |
|---|---|---|
| Process RSS (no models loaded) | ≤ 80 MB | ≤ 200 MB |
| Process RSS (RT-DETRv2-S COCO + Whisper-tiny loaded) | ≤ 350 MB | ≤ 700 MB |
| GPU VRAM (models resident) | ≤ 500 MB | ≤ 1500 MB |
| Capture textures | ≤ 50 MB | ≤ 100 MB |
| Event bus buffers | ≤ 10 MB | ≤ 50 MB |
| Replay log RAM (write buffers) | ≤ 30 MB | ≤ 100 MB |

Hard cap: 2 GB RSS, 2 GB VRAM. Exceeding either is a release blocker.

---

## 5. Latency budget per `observe()` slot

Where the 30 ms goes (hybrid mode, depth 2, ~60 elements):

| Stage | Budget p99 |
|---|---|
| Receive MCP request, parse params, validate | 0.5 ms |
| Fetch foreground context (process name, window bounds) | 1.0 ms (cached) |
| UIA snapshot of focused window (cached `IUIAutomationCacheRequest`) | 10 ms |
| Detection result from latest frame (lookup, no inference here) | 0.5 ms |
| HUD readings (cached) | 1 ms |
| Audio context (cached) | 0.5 ms |
| Recent events drain + filter | 1 ms |
| Clipboard summary | 0.5 ms |
| Filesystem recent events | 0.5 ms |
| Diagnostics population | 0.5 ms |
| Serialize to JSON | 3 ms |
| Send reply | 0.5 ms |
| Total | 19 ms p99 (budget allows 30) |

Headroom 11 ms for spikes and OS scheduling jitter.

---

## 6. Profile-specific perception budgets

Different profiles change the load. A game profile must not blow up productivity-mode performance.

| Profile | Capture FPS | Detection | OCR | Per-frame CPU | Per-frame GPU |
|---|---|---|---|---|---|
| Notepad / VS Code (a11y_only) | 0 (disabled) | none | on demand | ~0 | ~0 |
| Chrome (hybrid) | 5 (poll) | none | on demand | ~1% | ~0 |
| Minecraft (pixel_only) | 60 capture / async detection | RT-DETRv2-S COCO | HUD continuous | ~5% | ~18% |
| FPS demo (pixel_only) | 60 capture / async detection | RT-DETRv2-S COCO | HUD continuous + audio | ~6% | ~20% |

Higher capture FPS (e.g., 144 for high-fps replay analysis) scales CPU/GPU linearly. Documented in profile TOML comments.

---

## 7. Profiling discipline

Per-subsystem latency is instrumented via `tracing::span` and exposed as:

- Histograms in `synapse-telemetry::metrics` (`subsystem_latency_seconds{name}`)
- Per-tool histograms (`mcp_tool_latency_seconds{tool}`)
- Per-event-kind latencies (`event_to_subscriber_latency_seconds{kind}`)
- Reflex tick jitter (`reflex_tick_jitter_seconds`)

Exposed via the `health` MCP tool and Prometheus-format on `/metrics` when `--metrics-bind <addr>` is set.

### 7.1 Local profiling tools

- `cargo-flamegraph` for CPU hot paths
- `tracing-flame` to render spans as flamegraphs
- `nvidia-smi` / `Intel-XPU-SMI` for GPU usage during detection
- Windows Performance Recorder (WPR) + Windows Performance Analyzer (WPA) for kernel-level scheduling jitter

Reference script in `scripts/profile.ps1`:

```powershell
# Runs Synapse under tracing-flame, exercises a fixed scenario,
# emits flamegraph.svg + a CSV of subsystem timings.
.\scripts\profile.ps1 -Scenario "minecraft_combat_30s"
```

Scenarios live in `tests/scenarios/`. Encouraged when investigating a perf regression.

### 7.2 Regression detection

Subset of scenarios runs locally on the configured Windows host before a release candidate. Any p99 metric drifting > 20% triggers a release-blocking finding unless fixed or accepted by ADR. Exported `critcmp` JSON records baseline numbers per release tag outside git for retrospective analysis.

---

## 8. The non-blocking discipline

Hot paths never block on:

- `Mutex` contended by another path (use sharded locks or `crossbeam::SegQueue`)
- File I/O (everything goes through the storage write batcher)
- Network I/O (OTLP export on its own task with bounded queue and drop policy)
- GC pauses from Rust (no `Arc` cloning at >100 Hz without reason)
- Tokio's blocking pool from a `time_critical` thread

If a hot path needs data from a non-hot subsystem, data is published via `tokio::sync::watch` or `crossbeam::ArcSwap` so the hot path does a single atomic load.

---

## 9. The "no surprise allocations" rule

In hot loops we do not allocate per iteration:

- Capture loop: zero allocs per frame. Texture handles pooled.
- Reflex tick: zero allocs per tick. Event matching uses pre-compiled `EventFilter` representation.
- Action emit: at most one allocation (the `Vec<INPUT>` passed to `SendInput`); amortized.
- Detection inference: pre-allocated input/output tensors reused per frame.

Local benchmark runs on the configured Windows host assert zero allocations in hot loops via `dhat` or `tracing-allocations`. Manual FSV reads the benchmark export and allocation evidence directly; GitHub Actions/CI is not the source of truth, and benchmark scripts are supporting evidence only.

---

## 10. Backpressure rules

Every bounded channel has a documented drop policy:

| Channel | Capacity | Drop policy |
|---|---|---|
| Capture frames → perception | 2 | Drop oldest, increment `frames_dropped` |
| A11y events → perception | 1024 | Block briefly (10ms), then drop oldest with `events_dropped{source=a11y}` |
| Perception events → bus | 2048 | Block briefly, then drop oldest |
| Bus → reflex scheduler | 4096 | Drop oldest, log `reflex_event_dropped` |
| Bus → MCP subscriber | 4096 per subscription | Drop oldest, mark subscription as `lossy=true` in next push |
| Action queue → emitter | 256 | Reject submission with `ACTION_QUEUE_FULL` |
| Storage write queue | 4096 | Block briefly, then drop low-priority writes (telemetry only); `STORAGE_BACKPRESSURE` if persistent |

`block briefly` = 10 ms tokio sleep, then check again. Never block longer on a hot path.

---

## 11. Cold-start budget

| Stage | Budget |
|---|---|
| Process startup → MCP server ready to accept connections | ≤ 1.0 s |
| Profile load + cache warm | ≤ 200 ms per profile |
| Model load (RT-DETRv2-S COCO) | ≤ 2.5 s (CUDA), ≤ 5 s (DirectML), ≤ 12 s (CPU) |
| First `observe()` after connect | ≤ 100 ms (depends on first UIA call) |
| First `act_press` | ≤ 5 ms |

Models load **lazily** on first need, not at startup. Keeps startup budget tight; operator's first observation pays model-load cost.

---

## 12. SLA-style targets per MCP tool

Listed in `05_mcp_tool_surface.md`; aggregated here as the contract.

| Tool | p99 latency | p99.9 |
|---|---|---|
| `observe` | 30 ms | 60 ms |
| `find` | 20 ms | 40 ms |
| `describe` | 500 ms (VLM dependent) | 1500 ms |
| `read_text` | 30 ms small region; 100 ms full screen | 200 ms |
| `read_hud` | 15 ms | 30 ms |
| `audio_tail` | 10 ms | 20 ms |
| `audio_transcribe` | 200 ms (5s window) | 500 ms |
| `subscribe` | 5 ms (returns subscription id) | 10 ms |
| `set_capture_target` | 50 ms | 100 ms |
| `set_perception_mode` | 30 ms | 60 ms |
| `act_click` | 60 ms | 120 ms |
| `act_type` | dispatch latency depends on dynamics; `linear_ms_per_char` below 20 fails closed | target text integrity still requires app/file SoT readback |
| `act_press` | 5 ms | 10 ms |
| `act_aim` (non-track) | 100 ms (depends on duration) | duration + 30 ms |
| `act_drag` | duration + 20 ms | duration + 50 ms |
| `act_scroll` | 10 ms | 20 ms |
| `act_pad` | 5 ms (ViGEm) / 10 ms (hardware) | 10/20 ms |
| `act_combo` | scheduled by reflex runtime; combo length + 10 ms | combo + 30 ms |
| `act_clipboard` read/write | 5 ms | 10 ms |
| `act_run_shell` | wall-time dependent; tool overhead ≤ 20 ms | 50 ms |
| `act_launch` | up to `wait_timeout_ms` | dependent |
| `reflex_register` | 5 ms | 10 ms |
| `reflex_cancel` | 5 ms | 10 ms |
| `reflex_list` / `reflex_history` | 10 ms | 30 ms |
| `release_all` | 10 ms | 20 ms |
| `profile_list` | 5 ms | 10 ms |
| `profile_activate` | 100 ms (includes profile load + cache warm) | 200 ms |
| `health` | 5 ms | 10 ms |
| `replay_record` | 5 ms | 10 ms |

SERVER-SIDE latencies, measured from request parse to response send. Network and client-side latency are additional but typically ≤ 1 ms on local stdio.

---

## 13. Anti-pattern catalog (what we don't do)

| Anti-pattern | Why it's bad | What we do instead |
|---|---|---|
| Polling for events at high rate | Wastes CPU, adds latency | Push via event bus + SSE |
| Synchronous COM calls on the tokio worker pool | Blocks workers; UIA can take seconds | Dedicated COM apartment thread + channel |
| Copying frame textures to system memory just to check size | 100ms copy per frame | Stay on GPU; query dirty region instead |
| Allocating per-frame buffers | Allocator pressure | Pool textures, reuse JSON serialization buffers |
| `Mutex<HashMap>` for shared state read by hot paths | Lock contention | `arc-swap::ArcSwap` or sharded locks |
| Locking storage on the MCP path | Blocks responses | Storage is a separate task; MCP path queues writes |
| Loading large models at startup | Slow cold start | Lazy load on first use |
| Synchronous file watches | Unnecessary I/O | `notify` crate with event coalescing |

---

## 14. Bench targets

`synapse-test-utils` exposes `Bencher` helpers. Critical benchmarks:

- `bench_observe_warm_p99`: 30 ms target
- `bench_event_to_subscriber_p99`: 50 ms target
- `bench_reflex_tick_jitter`: 200 µs p99 jitter
- `bench_aim_curve_step_calc`: <1 µs per step
- `bench_action_software_press`: 1 ms p99
- `bench_action_hardware_press`: 5 ms p99 (requires HW gateway attached)
- `bench_hid_combo_timing`: ≤0.5 ms scheduled-step deviation (requires HW gateway attached)
- `bench_hid_high_volume`: 10k moves ≤15 s, zero drops/CRC errors (requires HW gateway attached)
- `bench_detection_rtdetr_v2_s_coco_640`: 25 ms p99 DirectML / 8 ms p99 CUDA (GPU dependent)
- `bench_ocr_winrt_120x32`: 8 ms p99

Benches run locally on the configured Windows host with Criterion named baselines and exported `critcmp` JSON. Regression > 20% blocks next release unless fixed or accepted by ADR with measurable justification.

---

## 15. The "spike check" guardrail

Beyond p99, we monitor "stuck above budget" spans. If any subsystem stays above 2× its p99 budget for >5 seconds:

1. Emit `synapse-performance-degraded` event with offending subsystem and current measurements
2. Surface in `health` response under `subsystems.<name>.status = "degraded_latency"`
3. Log `warn` with enough context to reproduce

Catches situations where p99 looks fine but a particular workflow is broken (e.g., a specific game's HUD region causes a 50ms OCR every frame).

---

## 16. What this doc does NOT cover

- Specific code patterns / data structure choices (lives in code review)
- Per-PR benchmarking workflow → `13_testing_strategy.md`
- Tracing/metrics export setup → `12_observability.md`
- Hardware HID specific budgets → `09_hardware_hid_gateway.md` §10
