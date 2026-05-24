# 12 — Observability

## 1. What "observable" means here

Synapse is a real-time system with real-time failure modes. Three needs:

1. **Live diagnosis.** Identify which subsystem is degraded and why.
2. **Post-hoc replay.** Step through what Synapse saw and did.
3. **Trend detection.** Catch regressions, memory growth, CF growth, dropped frames before outages.

All three built from: `tracing` spans, structured events, metrics histograms, the replay log.

---

## 2. Telemetry primitives

### 2.1 Tracing (`tracing` crate)

Every subsystem instruments with `#[tracing::instrument]` or manual `span!` macros. Each span carries:

- Span name (kebab-case, e.g., `capture.frame`, `perception.detect`, `action.emit`)
- Subsystem tag
- Relevant IDs (`session_id`, `reflex_id`, `seq`)
- Latency on close

Levels: `error` `warn` `info` `debug` `trace`. Default `info`. Override via `--log-level debug` or `RUST_LOG=synapse_capture=debug`.

### 2.2 Metrics (`metrics` crate + a small wrapper)

Three types:

- **Counter:** monotonic (`frames_dropped_total`)
- **Histogram:** latency distributions (`mcp_tool_latency_seconds{tool}`)
- **Gauge:** point-in-time (`cf_size_bytes{cf}`, `active_reflexes`)

Registered through `synapse-telemetry::metrics`. Names lowercase snake_case, `_total` suffix for counters, `_seconds` for time, `_bytes` for size.

### 2.3 Events (the replay log)

Covered in `06_data_schemas.md` and `07_storage_and_profiles.md`. Events are the canonical record of perception/actions/reflexes, flowing through the event bus into `CF_EVENTS`.

---

## 3. Logging

### 3.1 Backends

Two `tracing-subscriber` layers active in production:

| Layer | Purpose | Format |
|---|---|---|
| Console (stderr) | Operator-visible during foreground runs | Pretty (`tracing-subscriber::fmt`) on TTY; JSON otherwise |
| File | Persistent record | JSON Lines |

Operator enables OTLP exporter (`opentelemetry-otlp`) via `--otlp-endpoint http://localhost:4317`. Off by default.

### 3.2 File location and rotation

```
%LOCALAPPDATA%\synapse\logs\
├── synapse.log               # current, JSONL
├── synapse.log.2026-05-21.gz # daily rotation, oldest first to delete
├── synapse.log.2026-05-20.gz
├── ...
└── capture/
    ├── capture.log
    └── capture.log.2026-05-21.gz
```

Rotation (`tracing-appender::rolling::Builder`):

- Daily rollover at local midnight
- Keep 7 daily rotated files
- gzip rotated files
- Total log directory cap: 500 MB (oldest pruned first)
- Per-file size cap: 500 MB (rotate mid-day if reached)

Pruning runs at startup and in the `synapse-telemetry-gc` background worker while the daemon is alive. The default cadence is 6 h and can be overridden with `SYNAPSE_LOG_GC_INTERVAL_S`; `0` disables the periodic worker for short-lived tests.

### 3.3 What's logged at which level

| Level | Examples |
|---|---|
| `error` | Storage corruption, capture FFI failure, panic-hook fires, hardware HID disconnect mid-action |
| `warn` | Backpressure dropping events, model load fallback (DirectML → CPU), profile detection ambiguity, disk pressure level entered |
| `info` | Daemon start/stop, session open/close, profile activation, model loaded, capture target changed, reflex registered/cancelled |
| `debug` | Per-tool request summary, per-event-kind counts, per-frame latencies |
| `trace` | Raw event payloads (redacted), action emission details, COM call results |

Default `info` covers 90% of operator needs. `debug`/`trace` are for issue repro.

### 3.4 Structured fields

Every log line:

```json
{
  "timestamp": "2026-05-22T15:00:00.123Z",
  "level": "info",
  "target": "synapse_perception::detect",
  "fields": {
    "subsystem": "perception",
    "session_id": "...",
    "frame_seq": 12345,
    "detection_count": 7,
    "latency_ms": 4.2,
    "message": "detection complete"
  },
  "span": {"name": "perception.detect", "id": 42}
}
```

Spans embedded so consumers reconstruct the call tree without correlating timestamps.

### 3.5 Redaction passes through logging

Same `synapse-core::redact` patterns from `11_security_and_safety.md` apply. Console + file + OTLP layers redact free-form string fields before emission. Non-negotiable; even `trace` redacts unless `--no-redaction`.

---

## 4. Metrics

### 4.1 Core metric set

Subsystem health:

```
synapse_uptime_seconds (gauge)
synapse_subsystem_status{subsystem,status} (gauge 0/1)
synapse_panics_total (counter)
```

Capture:

```
capture_frames_total{target}
capture_frames_dropped_total{target,reason}
capture_frame_interval_seconds{target} (histogram)
capture_texture_pool_size_bytes
```

Perception:

```
perception_observations_total{mode}
perception_observation_assemble_seconds (histogram)
perception_observation_size_bytes (histogram)
detection_inferences_total{model_id}
detection_inference_seconds{model_id} (histogram)
detection_detections_per_frame (histogram)
ocr_calls_total{backend}
ocr_call_seconds{backend} (histogram)
ocr_cache_hits_total
a11y_events_total{kind}
a11y_snapshot_seconds (histogram)
cdp_connections_active
```

Audio:

```
audio_loopback_frames_total
audio_events_total{kind}
audio_transcriptions_total
audio_transcription_seconds (histogram)
```

Action:

```
action_emitted_total{backend,kind}
action_failed_total{backend,kind,error}
action_emit_seconds{backend} (histogram)
action_queue_depth (gauge)
action_queue_full_total
held_inputs_active (gauge)
```

Reflex:

```
reflex_registered_active (gauge)
reflex_fired_total{kind}
reflex_cancelled_total{reason}
reflex_tick_jitter_seconds (histogram)
reflex_tick_late_total
reflex_starved_total
```

MCP:

```
mcp_sessions_active (gauge)
mcp_requests_total{tool}
mcp_requests_failed_total{tool,error}
mcp_request_seconds{tool} (histogram)
mcp_push_notifications_total{kind}
mcp_subscriptions_active (gauge)
```

Storage:

```
cf_size_bytes{cf} (gauge)
cf_rows{cf} (gauge)
storage_writes_total{cf}
storage_batch_flush_seconds (histogram)
cache_hits_total{cf}
cache_misses_total{cf}
cache_evictions_total{cf,reason}
storage_disk_pressure_level (gauge)
storage_disk_free_bytes (gauge)
```

Hardware HID (when attached):

```
hid_frames_sent_total
hid_frames_acked_total
hid_frames_naked_total{reason}
hid_link_timeouts_total
hid_watchdog_fires_total
hid_round_trip_seconds (histogram)
```

### 4.2 Metric labels

Labels are bounded. Never use unbounded values (session IDs, reflex IDs, image hashes) — cardinality explodes.

Allowed labels: subsystem name, error code (closed set from `06_data_schemas.md` §8), CF name (closed set), backend, model_id (~5 values), tool name (30 tools).

### 4.3 Exposition

| Mechanism | When | Format |
|---|---|---|
| `health` MCP tool | Agent or operator calls | JSON |
| `/metrics` HTTP endpoint | `--metrics-bind <addr>` set | Prometheus text format |
| OTLP push | `--otlp-endpoint <url>` set | OTLP protobuf over gRPC |

Prometheus is the most common operator path. Hook into Grafana/Mimir for charts.

### 4.4 Local ringbuffer fallback

Without OTLP/Prometheus configured, last 6 hours of metrics are kept in `CF_TELEMETRY`. Query via:

```
synapse-mcp metrics dump --since 1h --output csv > metrics.csv
```

For "I noticed something weird, let me look at the last hour" without external infra.

---

## 5. The `health` MCP tool (operator-and-agent view)

Documented in `05_mcp_tool_surface.md` §3.29. Response shape:

```json
{
  "ok": true,
  "subsystems": {
    "capture": {"status": "healthy", "fps": 60, "frames_dropped_60s": 0},
    "a11y": {"status": "healthy", "events_60s": 412},
    "audio": {"status": "healthy", "device": "Speakers (Realtek...)"},
    "perception": {"status": "healthy", "detection_p99_ms": 4.2, "ocr_p99_ms": 7.8},
    "action": {"status": "healthy", "queue_depth": 0, "held_inputs": 0},
    "reflex": {"status": "healthy", "active_count": 2, "tick_jitter_us_p99": 180},
    "storage": {"status": "healthy", "db_size_mb": 234, "disk_pressure": 0},
    "hid": {"status": "disconnected"},
    "models": {"loaded": ["yolov10n", "whisper-tiny"]}
  },
  "retention": {
    "cf_events": {"ttl_hours": 24, "live_mb": 842, "soft_cap_mb": 2048},
    "...": "..."
  },
  "version": "0.1.0",
  "build": "abc123",
  "uptime_s": 1245
}
```

Subsystem `status` values match `06_data_schemas.md::SensorStatus`. Agents poll when sensing something off; operators use as "everything OK?" dashboard.

---

## 6. Debug overlay

Optional in-process overlay rendering telemetry over a transparent always-on-top window:

```
synapse-mcp overlay
```

Shows:

- Real-time frame rate, detection p99, action queue depth
- Active reflexes (name, fired count, last fired)
- Recent events (rolling list)
- Hot keys: `Ctrl+Alt+Shift+L` toggle, `Ctrl+Alt+Shift+P` panic
- Disk pressure level + DB size

Built with `egui` + `eframe`. Standalone binary in same workspace (`crates/synapse-overlay/`). Read-only — never emits actions.

---

## 7. Replay tooling

`synapse-mcp replay` CLI:

```bash
synapse-mcp replay list                     # list sessions
synapse-mcp replay show <session_id>        # summary of a session
synapse-mcp replay export <session_id> out.zip
synapse-mcp replay tail <session_id>        # follow live session
synapse-mcp replay search "act_click"       # search by tool/event kind/text
```

`replay show` outputs JSONL of events + actions + observations interleaved by time. Pipeable into `jq`.

`replay export` produces a self-contained `.zip` (Synapse Web Replay format):

- `manifest.json` (session id, time range, Synapse version, agent client)
- `events.jsonl` (full event log)
- `actions.jsonl` (full action log)
- `observations/{seq}.json` (each persisted observation)
- `frames/{seq}.webp` (if `--include-frames` and session had bookmarked frames)

`.zip` is plain — no encryption — and includes redaction per `11_security_and_safety.md` §5.

### 7.1 Replay viewer (future, not v1)

Web-based timeline viewer reading the `.zip`. Planned for v2.

---

## 8. Tracing in production

Production runs go to file + (optionally) OTLP. `info` is verbose enough for 90% of diagnoses:

- Every session open/close → log line
- Every profile activation → log line
- Every action emission → log line (batched if rate > 100/s)
- Every reflex registration → log line
- Every disk pressure transition → log line
- Every model load → log line

`debug` doubles volume; `trace` is development-only.

Volume at info on a 1-hour gameplay session: ~50 MB uncompressed JSONL. After gzip rotation: ~5 MB.

---

## 9. Crash dumps

Panic hook in `synapse-telemetry::panic_handler`:

1. Logs panic message + backtrace at `error`
2. Writes crash dump to `%LOCALAPPDATA%\synapse\crashes\YYYYMMDD-HHMMSS.dump` with panic, version, build hash, last 100 log lines, last 100 events
3. Fires `release_all` via the static handle (see `03_action.md` §11)
4. Re-panics to abort

Retained for 30 days; operator attaches to bug reports.

---

## 10. Performance profiling integration

`tracing-flame` + `pprof` behind feature flags:

```
cargo run --features perf-profiling -- --mode stdio
# Generates flamegraph.svg on Ctrl+C
```

Not default; ~5% overhead when active. CI runs a weekly perf-profiling job producing flamegraphs of standard scenarios.

---

## 11. Specific dashboards (operator templates)

Bundled Grafana dashboards in `dashboards/`:

- `synapse_overview.json` — high-level health + uptime + sessions
- `synapse_perception.json` — capture FPS, detection latency, OCR cache hit rate, a11y event rate
- `synapse_action.json` — action latency by backend + kind, queue depth, error rate
- `synapse_storage.json` — CF sizes, disk pressure, cache hit rates, GC frequency
- `synapse_reflex.json` — active reflex count, tick jitter, fired counts by kind

Operators import once. Updated with major Synapse releases.

---

## 12. What to look at when something is wrong (operator playbook)

**Symptom: actions feel laggy.**
- `action_emit_seconds` p99 by backend
- `action_queue_depth` (high = saturation)
- `reflex_tick_jitter_seconds` (spikes = host overload)

**Symptom: `observe()` is slow.**
- `perception_observation_assemble_seconds` p99
- `a11y_snapshot_seconds` p99 (high = UIA cross-process slowdown)
- `detection_inference_seconds` p99 (high = GPU contention)

**Symptom: events feel stale.**
- `event_to_subscriber_latency_seconds`
- `events_dropped_for_subscriber{}` counter

**Symptom: DB growing.**
- `synapse-mcp db status`
- `cf_size_bytes{cf}` to find the offender
- Disk pressure level + last GC time

**Symptom: hardware HID drops out.**
- `hid_link_timeouts_total`
- `hid_frames_naked_total{reason}`
- Reconnect USB; auto-reconnect should kick in

**Symptom: a reflex misfires.**
- `reflex_history --reflex-id <id>` for fires + filter matches
- `reflex_starved_total{reflex_id}`
- `CF_REFLEX_AUDIT` directly via `ldb`

Every symptom has a metric. If you can't find one, file an issue — that's a doc bug.

---

## 13. What this doc does NOT cover

- Per-tool metric details → `05_mcp_tool_surface.md` and `10_performance_budget.md`
- Storage retention → `07_storage_and_profiles.md` §6
- Replay format details → `07_storage_and_profiles.md` §5 + this doc §7
- Specific Grafana dashboard JSON → `dashboards/`
