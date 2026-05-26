# 04 — M3: Reflex + MCP Surface (2-3 weeks) — DONE (archival + M4 carry-over)

**Status:** Closed 2026-05-25 by release tag `v0.1.0-m3` (commit `97019ec`).
ADRs landed in this phase: ADR-0003 (reflex recursion guard, OQ-022),
ADR-0004 (reflex priority, OQ-005), ADR-0005 (multi-monitor capture target,
OQ-012), ADR-0006 (profile match precedence, OQ-015), ADR-0007 (per-event
vs batched notifications, OQ-029). M3 ships 30 MCP tools total (6 M1 + 9
M2 + 15 M3 including 4 `storage_*` diagnostics), real RocksDB w/ 11 CFs,
real WASAPI loopback + Whisper-tiny STT, real time-critical reflex
scheduler, streamable HTTP transport w/ SSE + bearer/Origin/session gates,
4 bundled profiles, operator panic hotkey, and the Notepad save-dialog
reflex demo from §2.

**Carry-over for M4** (landed but imperfect after the tag):

- **LoC overrun**: several files exceeded the 500 LoC hard cap during M3 —
  `synapse-a11y/src/lib.rs` 2087, `synapse-capture/src/lib.rs` 1798,
  `synapse-core/src/types.rs` 1567, `synapse-mcp/src/server.rs` 1335,
  `synapse-mcp/src/m3/reflex.rs` 1165, `synapse-reflex/src/lib.rs` 986,
  `synapse-reflex/src/scheduler.rs` 890, `synapse-mcp/src/http/sse.rs` 764,
  `synapse-mcp/src/m3/replay.rs` 651, `synapse-models/src/lib.rs` 535
  (plus several test files). M4's Block A.0 splits these before adding
  hardware HID, mirroring the M2 → M3 Block A.0 pattern.
- **CHANGELOG M3 entry tool names** name `profile_get`/`profile_set_active`
  but the shipped names on `main` are `profile_list`/`profile_activate`;
  the four `storage_*` diagnostic tools are also missing from the entry.
  Fix during the first M4 docs sweep.

> The rest of this file is preserved for onboarding so a fresh agent can see
> how M3 was structured (the M3 file is the template for M4+ phase docs:
> verbatim crate inventory, default-resolution table, FSV contract, manual
> happy-path + edge-case plan, Occam's-razor recap, trigger→outcome map).
> Every claim about the codebase below was verified against `main` at tag time.
> If a discrepancy now exists, the **codebase wins** — file a follow-up issue
> with the `phase:m3` + `area:docs` labels and patch this file in the same PR.

> **No backwards compatibility:** pre-v1 schema/API changes break callers.
> **No mocks gate completion:** OS-bound paths must be exercised against the
> real OS resource and verified by source-of-truth read-back
> (`00_methodology.md` §5). **Natural-only motion** (OQ-004 DECIDED 2026-05-22 —
> `07_cross_cutting.md` §12) applies to every bundled profile this phase added.

PRD authority: `docs/computergames/04_reflex_runtime.md` (Reflex subsystem),
`docs/computergames/07_storage_and_profiles.md` (CFs, TTLs, profile TOML),
`docs/computergames/02_perception.md` §6 (audio), `docs/computergames/01_architecture.md` §2 (HTTP transport),
`docs/computergames/05_mcp_tool_surface.md` §3.5-3.8 + §3.22-3.28 + §3.30 (M3 tools) + §5-7 (HTTP/SSE),
`docs/computergames/06_data_schemas.md` (event, filter, profile, reflex schemas),
`docs/computergames/10_performance_budget.md` §2 + §12 (latency budgets),
`docs/computergames/15_roadmap_and_milestones.md` §5 (M3 roadmap entry),
`docs/computergames/16_open_questions.md` OQ-001/005/010/012/015/022/023/024/029 (M3 decision targets).
Doctrine: `docs/impplan/00_methodology.md` + `07_cross_cutting.md`.

---

## 0. Mission in one sentence (Occam's razor)

**Fill out the four empty M3 crates (`synapse-reflex`, `synapse-storage`,
`synapse-profiles`, `synapse-audio`) and add the M3 MCP surface to
`synapse-mcp` so an agent can subscribe to events over SSE, register reflexes
that fire without further round-trips, load TOML profiles for the foreground
app, read transcribed audio, and manually inspect/trigger storage GC and disk
pressure through live MCP tools — backed by RocksDB with per-CF TTL/GC and a
loopback-only HTTP transport with bearer auth.** Everything else in this
document is a consequence of that sentence plus the global invariants. If a
contributor finds themselves designing something that doesn't trace back to
it, the design is wrong.

---

## 1. Where you are starting from (verified against `main` 2026-05-24)

### 1.1 Crate inventory and current state

```
crates/
├── synapse-mcp/             15 tools live (6 M1 + 9 M2) over stdio;
│                            `--mode http` returns NOT_YET_IMPLEMENTED exit 2.
│                            Tools listed verbatim in §1.3.
├── synapse-core/            full M0-M2 types + 80 pub-const error codes;
│                            ComboStep/ComboInput already carried.
├── synapse-action/          FULL — emitter actor + held BitSet + token
│                            bucket + curve/dynamics + Software/ViGEm/
│                            Recording/HardwareUnavailable backends +
│                            InvokePattern bridge + click_timing +
│                            clipboard + safety panic hook.
├── synapse-test-utils/      StdioMcpClient + Notepad fixture +
│                            wait_for_window_title_regex.
├── synapse-a11y/            UIA + WinEvent (COM STA) + CDP attach +
│                            re_resolve + expand_state_of + event
│                            coalesce/debounce helpers.
├── synapse-capture/         windows-capture 2.0 + DXGI fallback + DPI +
│                            screen↔window coord transforms.
├── synapse-perception/      Observation assembler + WinRT OCR.
├── synapse-models/          ORT 2.0-rc.12 session factory + sha256 verify
│                            (488 LoC; M1-stable surface).
├── synapse-telemetry/       JSON file + console + periodic GC + panic-to-log.
├── synapse-reflex/          EMPTY STUB (1 LoC). M3 main build target.
├── synapse-storage/         EMPTY STUB (8 LoC; `pub trait Db {}`). M3.
├── synapse-profiles/        EMPTY STUB (1 LoC). M3.
├── synapse-audio/           EMPTY STUB (1 LoC). M3.
├── synapse-hid-host/        EMPTY STUB. M4.
└── synapse-overlay/         binary skeleton (3 LoC). M5.
```

Workspace root: `Cargo.toml`. `default-members = ["crates/synapse-mcp", "crates/synapse-overlay"]`. `exclude = ["firmware/pico-hid"]`.

### 1.2 Already-pinned deps you will turn on at M3 (in `[workspace.dependencies]` at the repo root)

```
rocksdb = "0.24.0"  (features: lz4, zstd, multi-threaded-cf)
wasapi = "0.23.0"
notify = "9.0.0-rc.4"
axum = "0.8.9"
hyper = "1.9.0"
tower = "0.5.3"
rmcp = "1.7.0"  (already has `transport-streamable-http-server` feature on)
arc-swap = "1.9.1"  (already used by synapse-action; reuse for event bus)
crossbeam = "0.8.4"  (bounded channels for per-subscriber queues)
metrics = "0.24.6" + metrics-exporter-prometheus = "0.18.3"
opentelemetry = "0.32.0" + opentelemetry-otlp = "0.32.0"
ort = "2.0.0-rc.12"  (already used; reuse for Whisper-tiny + Silero VAD)
sha2 = "0.11.0"  (already used; verify model downloads)
```

No new top-level dep additions required at M3 — every crate listed already lives in `[workspace.dependencies]`.

### 1.3 Shipped MCP tools (the 15 you must NOT regress)

Per the #352 manual tool-surface readback (sorted):

```
act_aim, act_click, act_clipboard, act_drag, act_pad, act_press, act_scroll,
act_type, find, health, observe, read_text, release_all, set_capture_target,
set_perception_mode
```

Retained supporting checks may assert this exact list under neutral names. M3
adds 15 more -> final list at end of M3 is 30 live tools:
`subscribe`, `subscribe_cancel`, `reflex_register`, `reflex_cancel`,
`reflex_list`, `reflex_history`, `profile_list`, `profile_activate`,
`replay_record`, `audio_tail`, `audio_transcribe`, `storage_inspect`,
`storage_put_probe_rows`, `storage_gc_once`, and
`storage_pressure_sample`. The four `storage_*` tools are the operator-facing
manual-FSV surface for storage GC and disk-pressure evidence; they are not
scripts or harnesses. Update the workspace tools snapshot in the work-item that
adds each tool.

### 1.4 Already-wired entry points you will reuse

| Asset | Path | Use |
|---|---|---|
| `SynapseService::new()` + `tool_router` macro | `crates/synapse-mcp/src/server.rs:46` | M3 adds 15 `#[tool(...)]` methods next to the 15 existing |
| `mcp_error(code, msg) -> ErrorData` | `crates/synapse-mcp/src/m1.rs:369` | Throw every M3 error code through this helper (`{"code":-32099,"message":..,"data":{"code":"<NAME>"}}`) |
| `init_process_dpi_awareness` | `crates/synapse-mcp/src/main.rs:62` | Already called once; do NOT re-call |
| `install_panic_hook` | `crates/synapse-action/src/safety.rs:13` | Already installed by `main.rs:103` — reuse, do not add a second hook |
| `synapse_action::ActionHandle::execute(action)` | `crates/synapse-action/src/handle.rs:42` | Reflex `then` clauses dispatch through this — same channel as MCP tool dispatch (the `held_keys` BitSet stays the single source of truth) |
| `synapse_a11y::subscribe_win_events(sender)` | `crates/synapse-a11y/src/lib.rs:432` | M3 event bus subscribes here for the UIA event source |
| `synapse_a11y::coalesce_events(iter, 50ms)` + `debounce_value_changes(iter, 200ms)` | `crates/synapse-a11y/src/lib.rs:165 / 196` | Already implements the 02 §3 coalescing rules |
| `M2State::from_env_with_shutdown_reason(...)` | `crates/synapse-mcp/src/m2.rs:47-67` | M3 wraps with an `M3State` next to it (mirror the M1/M2 pattern) |
| `synapse_test_utils::stdio_mcp_client::StdioMcpClient` | `crates/synapse-test-utils/src/stdio_mcp_client.rs` | Every E2E spawns the daemon through this |

### 1.5 OS reality (same as M2)

- Production target: configured Windows 11 host (DX11-capable GPU; ViGEmBus working).
- Dev box: WSL2 on Win11. `rocksdb`/`wasapi` build on Linux + Windows; `windows`-feature deps stay Win-only.
- **Manual configured-host FSV is the M3 shipping gate** (issues #246/#247/#350/#351). Use local checks for supporting evidence; do not dispatch or wait on GitHub Actions/CI. Do not create automated FSV scripts, harnesses, or `*_fsv` tests.

### 1.6 Things that are NOT done at M3 (deferred ≥ M4)

- Hardware HID backend → `Backend::Hardware` requests still resolve to `HardwareUnavailableBackend` and surface `ACTION_BACKEND_UNAVAILABLE`. Do not change this.
- `act_combo` standalone MCP tool → M4. `ComboStep`/`ComboInput` types exist in `synapse-core`; M3 internal combos work via `reflex_register(kind: combo, ...)`.
- `act_run_shell`, `act_launch` → M4 (gated).
- Hardware HID gateway, RP2040 firmware → M4.
- Setup wizard, tray icon, installer, debug overlay, VLM `describe`, Florence-2 → M5.
- M2 carry-over bugs (#244, #239, #234, #233, #231): land **first** as a refactor PR (block A.0 below) so M3 work is not built on stale paths.

---

## 2. Demo gate (must pass to close M3)

Real Win11 box. Notepad open. Claude Desktop configured with `synapse-mcp` as MCP stdio server. The operator opens a fresh chat:

```
1. agent → reflex_register({
        kind: "on_event",
        when: { kind: "element-appeared",
                match: { window_title_regex: "^Save As$" } },
        then: { steps: [
          { action: "act_type", params: { text: "m3-demo.txt" } },
          { action: "act_press", params: { keys: ["enter"] } } ] },
        lifetime: { kind: "one_shot" } })
   → response: { reflex_id: "<uuid>" }
2. agent → act_press({ keys: ["ctrl","s"] })
3. (the reflex fires automatically when Notepad's Save As dialog appears)
4. agent → reflex_history({ reflex_id, limit: 1 })
   → response shows one `reflex-fired` audit row with the dialog appearance
     event_id and the two action steps marked completed.
```

**Source-of-truth verification (mandatory before closing the demo):**

1. `fs::read_to_string("%USERPROFILE%\Desktop\m3-demo.txt")` returns exactly the prior `act_type` payload (LF or CRLF per Notepad's Save As line ending).
2. RocksDB `CF_REFLEX_AUDIT` contains the audit row (read it back via a `Db::scan(CF_REFLEX_AUDIT, ..)` test helper).
3. The daemon's `synapse.log.<date>` JSONL contains at least one `code=REFLEX_FIRED` line with the matching `reflex_id`.
4. After the demo, the agent calls `reflex_cancel({ reflex_id })` (idempotent — already auto-expired by `lifetime: one_shot`); response `{ cancelled: false, reason: "already_expired" }`.

**Failure modes that block the gate:** dialog appeared but reflex did not fire (event filter or coalescing bug), reflex fired but actions raced focus loss (`ACTION_FOREGROUND_LOST`), file written to wrong directory, audit row missing, RocksDB write batcher dropped the row on flush boundary, daemon exited 0 with no `REFLEX_FIRED` log line.

---

## 3. Deliverables

### 3.1 New crate file layout

```
crates/synapse-reflex/
├── Cargo.toml                  (add deps; see §1.2)
├── src/
│   ├── lib.rs                  (≤ 200 LoC: ReflexRuntime::spawn + re-exports)
│   ├── bus.rs                  (≤ 400 LoC: EventBus broadcast — ArcSwap<Vec<Subscriber>>)
│   ├── scheduler.rs            (≤ 500 LoC: 1ms tick thread @ THREAD_PRIORITY_TIME_CRITICAL via CreateWaitableTimerEx)
│   ├── kinds/
│   │   ├── mod.rs              (≤ 80 LoC: ReflexKind dispatch trait)
│   │   ├── aim_track.rs        (≤ 400 LoC: delta + gain + deadzone + max_speed + EMA α=0.7 per OQ-013)
│   │   ├── hold_move.rs        (≤ 250 LoC: KeyDown on register, KeyUp on lifetime end)
│   │   ├── hold_button.rs      (≤ 200 LoC: same pattern for mouse / pad buttons)
│   │   ├── combo.rs            (≤ 350 LoC: timed step sequence; consumes ComboStep)
│   │   └── on_event.rs         (≤ 400 LoC: EventFilter eval + debounce + recursion guard OQ-022)
│   ├── conflict.rs             (≤ 300 LoC: priority + newer-wins + starvation logging)
│   ├── audit.rs                (≤ 200 LoC: CF_REFLEX_AUDIT writes via Db handle)
│   └── error.rs                (≤ 250 LoC: ReflexError + .code() table)
├── benches/
│   ├── reflex_tick_jitter_idle.rs
│   ├── reflex_tick_jitter_under_load.rs
│   ├── event_to_subscriber.rs
│   └── reflex_combo_step_interval.rs
└── tests/
    ├── bus_drop_oldest_proptest.rs
    ├── filter_eval_proptest.rs
    ├── on_event_behavior.rs
    ├── aim_track_controller.rs
    ├── hold_move_behavior.rs
    ├── combo_schedule.rs
    └── recursion_guard.rs

crates/synapse-storage/
├── Cargo.toml
├── src/
│   ├── lib.rs                  (≤ 200 LoC: Db open + CF handles)
│   ├── cf.rs                   (≤ 150 LoC: pub const CF_* names, validated against 07 §4)
│   ├── compaction.rs           (≤ 300 LoC: per-CF compaction filter w/ TTL)
│   ├── batch.rs                (≤ 250 LoC: 100ms / 64KB / explicit-flush write batcher)
│   ├── gc.rs                   (≤ 350 LoC: 5min GC task — DeleteRange + compact per soft cap)
│   ├── pressure.rs             (≤ 400 LoC: 4-level disk-pressure responder per 07 §6.3)
│   ├── codecs.rs               (≤ 200 LoC: JSON wrappers; bincode forbidden — ADR-0001 / RUSTSEC-2025-0141)
│   └── error.rs                (≤ 200 LoC: StorageError + .code())
└── tests/
    ├── open_all_cfs.rs
    ├── compaction_ttl_proptest.rs
    ├── batch_throughput.rs
    ├── gc_soft_cap.rs
    └── disk_pressure_4_levels.rs

crates/synapse-profiles/
├── Cargo.toml
├── src/
│   ├── lib.rs                  (≤ 200 LoC: ProfileRuntime::spawn + re-exports)
│   ├── parser.rs               (≤ 350 LoC: toml → Profile + version compat)
│   ├── watcher.rs              (≤ 300 LoC: notify watcher; debounced 200ms)
│   ├── resolver.rs             (≤ 250 LoC: precedence + match-by-exe/title/steam_appid)
│   └── error.rs                (≤ 150 LoC: ProfileError + .code())
├── profiles/                   (bundled TOMLs ship in repo)
│   ├── notepad.toml
│   ├── vscode.toml
│   ├── chrome.toml
│   └── terminal.toml
└── tests/
    ├── parse_bundled.rs
    ├── hot_reload.rs
    └── natural_defaults_smoke.rs   (asserts every bundled mouse_curve_default/keyboard_dynamics_default == "natural")

crates/synapse-audio/
├── Cargo.toml
├── src/
│   ├── lib.rs                  (≤ 200 LoC: AudioRuntime::spawn + re-exports)
│   ├── loopback.rs             (≤ 400 LoC: WASAPI loopback ring 5s)
│   ├── detectors.rs            (≤ 350 LoC: loud_transient / speech_start_end / music_start_end / Silero VAD)
│   ├── stt.rs                  (≤ 300 LoC: Whisper-tiny-int8 ONNX lazy load + transcribe)
│   ├── direction.rs            (≤ 250 LoC: L/R energy + GCC-PHAT lag)
│   └── error.rs                (≤ 150 LoC: AudioError + .code())
└── tests/
    ├── loopback_ring.rs
    ├── vad.rs
    ├── whisper_known_clip.rs
    └── direction_pan.rs

crates/synapse-mcp/src/
├── m1.rs / m1/                  (existing)
├── m2.rs / m2/                  (existing)
├── m3.rs                       (NEW: tool param/response types + shared helpers)
└── m3/                         (NEW)
    ├── subscribe.rs            (subscribe + subscribe_cancel)
    ├── reflex.rs               (reflex_register + reflex_cancel + reflex_list + reflex_history)
    ├── profile.rs              (profile_list + profile_activate)
    ├── replay.rs               (replay_record)
    └── audio.rs                (audio_tail + audio_transcribe)

crates/synapse-mcp/src/http/    (NEW: axum HTTP transport + SSE)
├── mod.rs
├── auth.rs                     (bearer token + Origin/Host check)
├── transport.rs                (axum routes for rmcp::transport::streamable-http-server)
├── sse.rs                      (SSE push with Last-Event-ID resume; 4096-event ring per sub)
└── session.rs                  (Mcp-Session-Id header lifecycle)
```

**Hard caps stay 500 LoC / 30 LoC fn / cyclomatic ≤ 10.** Split early. The M2 carry-over showed what happens otherwise (emitter.rs at 1474 LoC).

### 3.2 New `synapse-core` types (extend `crates/synapse-core/src/types.rs` and re-export from `lib.rs`)

Every struct: `#[derive(Clone, Debug, Eq|PartialEq, Serialize, Deserialize, JsonSchema)]` with `#[serde(deny_unknown_fields)]`. Every enum on the wire: `#[serde(tag = "kind", rename_all = "snake_case")]`. Match `06_data_schemas.md` byte-for-byte; PRD wins on conflict.

```
Profile, ProfileMatch, ProfileCapture, ProfileDetection, ProfileOcr,
HudFieldSpec, HudExtractor, HudParser, HudRegion, WindowEdge,
ProfileBackends, EventExtension
ReflexRegistration, ReflexKind, ReflexLifetime, ReflexState, ReflexStatus
StoredEvent, StoredObservation, StoredReflexAudit, StoredSession
OcrResult (extension; word-level), OcrWord
```

`Event` / `EventFilter` / `DataPredicate` / `EventSource` already exist in `synapse-core` since M1; extend `EventFilter` with the M3 `match` predicates listed in `06 §3.2` (Kind/Source/And/Or/Not/Data with DataPredicate full set). Do **not** reshape the existing variants — extend additively or land a wipe-and-rebuild PR (no migration shim; pre-v1 directive).

### 3.3 Channel + lifetime invariants (hard contract)

- **EventBus**: `ArcSwap<Vec<Subscriber>>`. Per-subscriber bounded `crossbeam::channel` 4096 events, drop-oldest. Active subscriptions are capped at 64 per session by default; the HTTP/stdio MCP service can lower or raise the cap with `--max-subscriptions` / `SYNAPSE_MAX_SUBSCRIPTIONS`, and exceeding it returns `SUBSCRIPTION_CAP_REACHED`. Slow consumer increments `events_dropped_for_subscriber{subscription_id}` metric; subscription marked `lossy=true` on next push so SSE clients see the gap.
- **Reflex scheduler tick**: 1 ms via `CreateWaitableTimerEx` + `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` + MMCSS "Pro Audio". On MMCSS-unavailable boxes (for example Linux) the bench gate stays disabled; production target is Win11. Fallback `tokio::time::interval(2ms)` is logged as `degraded` + spike check fires.
- **Reflex cap 32 per session** (`REFLEX_CAP_REACHED`). Recursion guard ≤ 4 firings/tick (OQ-022, `REFLEX_RECURSION_LIMIT` — declare the new code in this PR and add to `synapse-core::error_codes`).
- **Storage write batch flushes** every 100 ms or 64 KB or on explicit `Db::flush()`. Per-CF compaction filter runs in RocksDB's compaction thread; GC task runs in a dedicated `tokio::task` every 5 min checking soft caps and calling `DeleteRange` + manual `compact_range`. Disk-pressure responder polls disk every 30 s and emits `STORAGE_DISK_PRESSURE_LEVEL_N` on transitions.
- **HTTP transport**: `axum` server bound to `127.0.0.1:7700` by default (`--bind`); refuses non-loopback by default with `HTTP_BIND_NON_LOOPBACK_REFUSED` (new code; declare it). Bearer token loaded from `%APPDATA%\synapse\token.txt` (M5 wizard generates; M3 falls back to `SYNAPSE_BEARER_TOKEN` env var for dev). `Origin` and `Host` headers checked against loopback list.
- **SSE**: 4096-event ring per subscription; `Last-Event-ID` header on reconnect replays from buffer. Deeper outage ⇒ subscription marked `lossy=true` in next push and re-sent `subscription_started` with `lossy: true`.
- **`SCHEMA_VERSION = 1`** (already `pub const` in `synapse-core::defaults`). Any storage shape change at M3 bumps `SCHEMA_VERSION` and triggers wipe-and-rebuild (no migration shim).

### 3.4 Tracing instrumentation

Every public `fn` in M3 crates carries `#[tracing::instrument(skip_all, fields(...))]` with the same convention as M1/M2 (`code = "..."` per error path; `readback=<name>` for state transitions). Every error-code emit also logs `tracing::warn!(code = "<CODE>", ...)`. Manual FSV issue evidence records source-of-truth values; tests and stdout greps are supporting regression evidence only.

---

## 4. MCP tool schemas (defaults — every test asserts these)

| Tool | Field | Default | Source |
|---|---|---|---|
| `subscribe` | `kinds` | `[]` (subscribe-all) | `05 §3.5` |
| `subscribe` | `snapshot_first` | `false` | `05 §3.5` |
| `subscribe` | `buffer_size` | `4096` | `04 §bus` |
| `subscribe_cancel` | `subscription_id` | required | `05 §3.5` |
| `reflex_register` | `priority` | `100` | `05 §3.7` |
| `reflex_register` | `lifetime` | `{ kind: "until_cancelled" }` | `05 §3.7` |
| `reflex_register` | `backend` (for then.steps) | `"auto"` | inherits M2 |
| `reflex_register` | curve/dynamics in then.steps | `"natural"` | OQ-004 |
| `reflex_cancel` | `reflex_id` | required | `05 §3.7` |
| `reflex_list` | `include_expired` | `false` | `05 §3.7` |
| `reflex_history` | `limit` | `50` (cap 1000) | `05 §3.7` |
| `profile_list` | `include_inactive` | `true` | `05 §3.27` |
| `profile_activate` | `profile_id` | required | `05 §3.27` |
| `replay_record` | `format` | `"jsonl"` | `05 §3.28` |
| `replay_record` | `target` | `"observations"` | `05 §3.28` |
| `audio_tail` | `seconds` | `5` (cap 5 — matches loopback ring) | `05 §3.30` |
| `audio_transcribe` | `seconds` | `5` | `05 §3.30` |
| `audio_transcribe` | `language` | `"en"` | `05 §3.30` |
| `storage_inspect` | — | no params | `05 §3.31` |
| `storage_put_probe_rows` | `cf_name`, `key_prefix`, `rows`, `value_bytes` | required | `05 §3.32` |
| `storage_gc_once` | `cf_name`, `soft_cap_rows`, `hard_cap_rows` | required | `05 §3.33` |
| `storage_pressure_sample` | `free_bytes` | required | `05 §3.34` |

All schemas serialize with `additionalProperties: false` and the `insta` snapshots at `tests/snapshots/m3_*.snap` enforce every default. The M3 work-item that adds each tool also updates the workspace tools snapshot.

---

## 5. Error codes (every code must be `pub const` in `synapse-core::error_codes` AND throwable in a test)

These are already declared (verified in `crates/synapse-core/src/error_codes.rs`):

```
REFLEX_CAP_REACHED               REFLEX_KIND_INVALID
REFLEX_PARAMS_INVALID            REFLEX_TARGET_INVALID
REFLEX_FILTER_INVALID            REFLEX_PRIORITY_INVALID
REFLEX_TICK_LATE                 REFLEX_TRACK_LOST
REFLEX_STARVED                   REFLEX_DISABLED_BY_OPERATOR
REFLEX_LIFETIME_EXPIRED          PROFILE_NOT_FOUND
PROFILE_PARSE_ERROR              PROFILE_VERSION_INCOMPATIBLE
PROFILE_KEYMAP_INVALID           PROFILE_HUD_REGION_INVALID
SESSION_NOT_FOUND                SESSION_EXPIRED
SUBSCRIPTION_NOT_FOUND           SUBSCRIPTION_CAP_REACHED
TOOL_NOT_FOUND                   TOOL_PARAMS_INVALID
TOOL_INTERNAL_ERROR              STORAGE_OPEN_FAILED
STORAGE_WRITE_FAILED             STORAGE_READ_FAILED
STORAGE_CORRUPTED                STORAGE_SCHEMA_MISMATCH
HUD_NO_ACTIVE_PROFILE            HUD_FIELD_NOT_DEFINED
HUD_EXTRACTION_FAILED            AUDIO_DEVICE_LOST
AUDIO_LOOPBACK_INIT_FAILED       AUDIO_STT_MODEL_NOT_LOADED
```

New M3 codes to **add** in this phase (declare in `synapse-core::error_codes`, list in `06 §8`, throw in a test, and reflect in the M3 catalog grep gate):

```
REFLEX_RECURSION_LIMIT           (OQ-022 — recursion guard)
HTTP_BIND_NON_LOOPBACK_REFUSED   (loopback-only default)
HTTP_TOKEN_INVALID               (bearer auth)
HTTP_ORIGIN_REFUSED              (Origin/Host check)
HTTP_SESSION_INVALID             (Mcp-Session-Id lifecycle)
STORAGE_DISK_PRESSURE_LEVEL_1..4 (4 codes — already declared in 06 §8 but not yet const; declare and emit per pressure-level transition)
REPLAY_TARGET_INVALID            (replay_record param check)
REPLAY_FORMAT_INVALID            (replay_record param check)
SAFETY_PERMISSION_DENIED         (per-tool M3 permission refusal)
SAFETY_PROFILE_ACTION_DENIED     (unknown-scope profile activation refusal)
REFLEX_ACTION_PERMISSION_DENIED  (reflex firing permission suppression)
```

Throw helper stays `mcp_error(code, msg)` from `crates/synapse-mcp/src/m1.rs:369`.

---

## 6. Work-items (PR-sized, ordered)

### Block A.0 — M2 carry-over (do FIRST; no behavior beyond what M2 promised)

| # | Title | Acceptance |
|---|---|---|
| 0a | `refactor(action): split emitter.rs / vigem.rs / invoke.rs / software.rs to ≤ 500 LoC` | All files under cap; `cargo test --workspace` green; no public API change; insta snapshots unchanged |
| 0b | `refactor(mcp): split m2/click.rs (506) and m2/press.rs (545) to ≤ 500 LoC` | same |
| 0c | `fix(a11y): widen TreeWalker to RawView for packaged Notepad MenuBar (#244)` | `synapse_a11y::snapshot()` returns the menu-bar children on Win11 22H2+ packaged Notepad; manual #352 Notepad/File-menu evidence covers the invoke path |
| 0d | `fix(action): SoftwareBackend mouse_move uses Win32 GetCursorPos in DPI-aware mode (#234)` | proptest: `(cursor_after - cursor_before) == (dx, dy)` within ±1 px across 100 random DPI scales |
| 0e | `fix(action): thread dynamics through text_dispatch.rs (#233)` | Notepad receives every char of a 1000-char paste; recording backend events count == 2 × text length under `Natural::FAST` |
| 0f | `fix(action): held-key auto-release calls backend KeyUp (#231)` | external `WH_KEYBOARD_LL` hook observes `KeyUp(a)` within 50 ms of timer expiry; existing `STUCK_KEY_AUTO_RELEASED` log line unchanged |
| 0g | `docs(action): document DPI-aware physical-pixel coordinate convention (#239)` | `05_mcp_tool_surface.md` + every M2 tool schema description updated |

### Block A — storage (work-items 1-6)

| # | Title | Throws | Acceptance (manual SoT evidence required) |
|---|---|---|---|
| 1 | `feat(storage): cf.rs — pub const CF_* names matching 07 §4` | — | unit test asserts the 11 CF names equal their string literals (mirrors `error_codes_literal.rs`); doc-grep gate added to `scripts/check_docs.ps1` |
| 2 | `feat(storage): Db::open(tempdir) w/ all 11 CFs + tuning per 07 §12` | `STORAGE_OPEN_FAILED`, `STORAGE_SCHEMA_MISMATCH` | supporting unit check opens db and lists CF handles; manual boundary evidence opens against existing db with `SCHEMA_VERSION` mismatch -> `STORAGE_SCHEMA_MISMATCH` + wipe-and-rebuild succeeds on retry |
| 3 | `feat(storage): per-CF compaction filter w/ TTL from runtime config` | — | proptest inserts N records w/ timestamps spanning > TTL and calls `compact_range`; manual SoT readback scans CF and confirms old rows gone, fresh rows present |
| 4 | `feat(storage): write batcher (100ms / 64KB / flush)` | `STORAGE_WRITE_FAILED` | bench: 10k events writes <= 200 ms wall; manual SoT readback after each scenario uses `Db::scan(CF, ..)` and confirms byte-equal payloads |
| 5 | `feat(storage): GC task @ 5 min w/ soft-cap DeleteRange + compact` | — | scenario: use `storage_put_probe_rows` to fill `CF_EVENTS` to 2x soft cap; trigger `storage_gc_once`; manual SoT readback uses `storage_inspect` and daemon log to confirm row count dropped below soft cap and `cache_evictions_total{cf,reason}` incremented |
| 6 | `feat(storage): disk-pressure responder 4 levels (07 §6.3)` | `STORAGE_DISK_PRESSURE_LEVEL_1..4`, `STORAGE_CF_HARD_CAP_REACHED` | trigger `storage_pressure_sample` with synthetic free-byte values for L1-L4; manual evidence records emitted transition codes via `storage_inspect` and daemon log; L4 write gate is verified by `storage_put_probe_rows` before/after row counts |

### Block B — event bus + reflex runtime (work-items 7-13)

| # | Title | Throws | Acceptance (manual SoT evidence required) |
|---|---|---|---|
| 7 | `feat(reflex): EventBus + drop-oldest subscriber backpressure (4096 buf)` | `SUBSCRIPTION_CAP_REACHED` | per-subscriber proptest: 5000 events pushed; manual SoT readback reads subscriber queue, confirms <= 4096, and confirms `events_dropped_for_subscriber` matches the overflow count |
| 8 | `feat(reflex): scheduler thread @ TIME_CRITICAL + 1ms CreateWaitableTimerEx + MMCSS` | `REFLEX_TICK_LATE` | bench `reflex_tick_jitter_idle` p99 <= 200 us; bench `reflex_tick_jitter_under_load` p99 <= 500 us; manual evidence records tick log elapsed-since-last-tick measurement |
| 9 | `feat(reflex): aim_track controller (delta + gain + deadzone + max_speed + EMA alpha=0.7)` | `REFLEX_TRACK_LOST` | E2E vs a static `DetectedEntity` synthetic source: 60 ticks; manual SoT readback reads cursor (`GetCursorPos`) and confirms within +/-deadzone of target |
| 10 | `feat(reflex): hold_move + hold_button (KeyDown register / KeyUp lifetime end)` | `REFLEX_LIFETIME_EXPIRED` | E2E: hold `w` for 2 s via `UntilEvent` synthetic; lifetime fires; manual SoT readback of `RecordingBackend::events()` shows `KeyUp(w)` exactly once; `held_keys` BitSet empty after |
| 11 | `feat(reflex): combo (timed step sequence; consumes ComboStep)` | — | bench `reflex_combo_step_interval` step intervals within 500 us of scheduled; manual SoT readback confirms dispatched action sequence matches the `ComboStep` payload byte-for-byte |
| 12 | `feat(reflex): on_event w/ EventFilter eval + debounce + recursion guard (OQ-022)` | `REFLEX_RECURSION_LIMIT`, `REFLEX_FILTER_INVALID` | proptest filter eval: `Not(Not(x)) == x` for total filters; manual SoT readback for synthetic event stream firing 5x in one tick shows audit has 4 firings + 1 `REFLEX_RECURSION_LIMIT` row |
| 13 | `feat(reflex): conflict resolution (priority + newer-wins + starvation log)` | `REFLEX_STARVED` | two contending aim_tracks: lower numeric priority wins; manual SoT readback of loser's `reflex_list` row shows `status: "starved"` after 2 s; audit log shows `REFLEX_STARVED` |

### Block C — profiles (work-items 14-17)

| # | Title | Throws | Acceptance (manual SoT evidence required) |
|---|---|---|---|
| 14 | `feat(profiles): TOML loader -> Profile + version compat` | `PROFILE_PARSE_ERROR`, `PROFILE_VERSION_INCOMPATIBLE` | supporting unit checks parse every bundled profile + 3 synthetic invalid TOMLs (missing required field / bad regex / future schema_version); manual evidence records exact error code per invalid case; boundary: empty profile dir -> `profile_list` returns `[]` |
| 15 | `feat(profiles): notify watcher + match resolver (debounced 200ms)` | `PROFILE_HUD_REGION_INVALID`, `PROFILE_KEYMAP_INVALID` | E2E: write `profiles/scratch.toml`; manual SoT readback shows `profile_list` contains it within 1 tick; edit readback shows in-memory profile replaced; delete readback shows removal from list; profile resolution follows the one-active-capture-target rule in ADR-0005 |
| 16 | `feat(profiles): bundled notepad / vscode / chrome / terminal w/ Natural defaults` | — | smoke test asserts every bundled `mouse_curve_default == "natural"` AND `keyboard_dynamics_default == "natural"`; E2E launches each app, `profile_list` shows the active match |
| 17 | `feat(mcp): profile_list + profile_activate tools` | `PROFILE_NOT_FOUND` | snapshot at `tests/snapshots/m3_profile_tools.snap`; manual SoT readback after `profile_activate({id: "vscode"})` confirms next `health.subsystems.profiles.active_profile_id` equals `vscode` |

### Block D — audio (work-items 18-20)

| # | Title | Throws | Acceptance (manual SoT evidence required) |
|---|---|---|---|
| 18 | `feat(audio): WASAPI loopback ring 5s + detectors` | `AUDIO_DEVICE_LOST`, `AUDIO_LOOPBACK_INIT_FAILED` | playback known test asset; manual SoT readback confirms events emitted (`loud_transient`, `speech_started/ended`), RMS metric flows, and `audio_tail(seconds=2)` returns the last 2 s of PCM; loopback target remains independent of visual capture target per ADR-0005 |
| 19 | `feat(audio): Whisper-tiny-int8 STT (lazy load + sha256 verify)` | `AUDIO_STT_MODEL_NOT_LOADED`, `MODEL_HASH_MISMATCH` | known 5 s clip with ground-truth transcript; bench p99 <= 200 ms; manual SoT readback confirms `audio_transcribe` returns the ground-truth string +/-10% Levenshtein; missing model -> `AUDIO_STT_MODEL_NOT_LOADED` |
| 20 | `feat(audio): direction estimate (L/R energy + GCC-PHAT)` | — | 3 stereo test clips at -60deg/0deg/+60deg azimuth; manual SoT readback confirms estimate within +/-15deg per clip |

> Dev-loop note: WSL → Windows PulseAudio bridge on `tcp:127.0.0.1:4713` (mirrored networking) is available for fixture playback / capture (issue #85). Production path stays WASAPI direct — PulseAudio is **not** in the shipped surface.

### Block E — MCP HTTP + SSE + new tools (work-items 21-24)

| # | Title | Throws | Acceptance (manual SoT evidence required) |
|---|---|---|---|
| 21 | `feat(mcp): axum HTTP + Mcp-Session-Id + bearer auth + Origin/Host check` | `HTTP_BIND_NON_LOOPBACK_REFUSED`, `HTTP_TOKEN_INVALID`, `HTTP_ORIGIN_REFUSED`, `HTTP_SESSION_INVALID` | `curl` matrix: no token -> 401; bad Origin -> 403; missing/expired session id -> 404; non-loopback bind without `--allow-non-loopback` -> process exits with `HTTP_BIND_NON_LOOPBACK_REFUSED`; manual SoT readback reads daemon log for each refusal |
| 22 | `feat(mcp): SSE push notifications w/ Last-Event-ID resume` | — | reconnect test: drop SSE mid-stream; reconnect with `Last-Event-ID: <seq>`; manual SoT readback confirms server replays the exact missed events byte-equal; buffer overflow -> next push carries `lossy: true` and `subscription_started` is re-sent |
| 23 | `feat(mcp): subscribe + subscribe_cancel + reflex_register + reflex_cancel + reflex_list + reflex_history` | `SUBSCRIPTION_NOT_FOUND`, `REFLEX_*` | tools/list snapshot updated; manual E2E evidence registers on_event for `value-changed`, fires it, observes `reflex-fired` audit + `reflex_history` row, cancels it, then observes `reflex_list` no longer shows it |
| 24 | `feat(mcp): replay_record + audio_tail + audio_transcribe` | `REPLAY_TARGET_INVALID`, `REPLAY_FORMAT_INVALID`, `AUDIO_*` | tools/list snapshot updated; manual SoT readback confirms `replay_record({target:"observations", duration_ms:1000})` writes a JSONL file at response `path`; direct file read confirms every line parses as `Observation` |
| 24a | `feat(mcp): storage_inspect + storage_put_probe_rows + storage_gc_once + storage_pressure_sample` | `TOOL_PARAMS_INVALID`, `SAFETY_PERMISSION_DENIED`, storage `STORAGE_*` | tools/list snapshot updated; manual E5/E6 evidence uses only live MCP calls plus separate `storage_inspect`/log readbacks for storage row counts, pressure transition codes, and write-gate behavior |

### Block F — safety + demo (work-items 25-26)

| # | Title | Throws | Acceptance (manual SoT evidence required) |
|---|---|---|---|
| 25 | `feat(safety): panic hotkey RegisterHotKey(Ctrl+Alt+Shift+P) -> reflex_disable_all + ReleaseAll within 50ms` | `SAFETY_OPERATOR_HOTKEY_FIRED` | E2E: register 3 reflexes; press hotkey via `keybd_event` test injection; manual SoT readback confirms all 3 reflexes status -> `disabled`, `RELEASE_ALL_HANDLE` fired, `GetAsyncKeyState` for every previously-held key returns 0, and daemon log carries `SAFETY_OPERATOR_HOTKEY_FIRED` |
| 26 | `test(e2e): notepad save-dialog reflex demo (M3 demo gate)` | — | full §2 demo via stdio AND via HTTP w/ token; manual evidence records all 4 source-of-truth reads; manual sign-off pasted into PR |

Total: 6 carry-over PRs + 26 M3 PRs plus storage diagnostic MCP surface = 33
PR-sized work items. Order matters: A.0 (carry-over) → A (storage; reflex
needs `Db` to write audit) → B (reflex runtime) → C (profiles) → D (audio) →
E (HTTP + tools wire up) → F (safety + demo).

---

## 7. Manual Full-State Verification — the M3 contract

Every M3 work item follows this manual evidence template. Automated tests under `crates/synapse-{reflex,storage,profiles,audio}/tests/**` and `crates/synapse-mcp/tests/m3_*.rs` are supporting regression checks only and must not claim to perform FSV.

### 7.1 Source-of-truth table (M3)

| Action under test | Source of truth | How to read it |
|---|---|---|
| `subscribe` then event push | per-subscription queue + SSE stream bytes | drain channel; for SSE: `reqwest` GET with `Accept: text/event-stream` and read framed events |
| `reflex_register` | `Db::scan(CF_REFLEX_AUDIT, prefix=reflex_id)` returns the registration audit row | direct CF scan via `synapse-storage::Db` |
| `reflex_register` then fire | audit row + `RecordingBackend::events()` for action steps + tracing log `code=REFLEX_FIRED` | three reads, all required |
| `reflex_cancel` | `reflex_list` no longer shows the reflex; audit row `status=cancelled` | two reads |
| `profile_activate` | `health.subsystems.profiles.active_profile_id` updated; `tracing` log `code=PROFILE_ACTIVATED` | two reads |
| `replay_record` | the file at the response's `path` exists, is non-empty, and each line `serde_json::from_str::<Observation>()` succeeds | direct disk read |
| `audio_transcribe` | response `text` equals the ground-truth transcript within Levenshtein ≤ 10% of length | text compare |
| `audio_tail` | the returned PCM byte length equals `seconds * sample_rate * channels * 2` (i16) | byte-length math |
| `Db::put_batch` | `Db::get(cf, key)` returns the value after flush | round-trip |
| Storage GC | `storage_inspect` row counts/bytes + daemon log line for `cache_evictions_total` | read before, trigger `storage_gc_once`, read after |
| Disk-pressure transition | `storage_pressure_sample` response + `storage_inspect.pressure_transition_codes` + daemon log | one synthetic free-byte sample per transition, with a separate read after each |
| Reflex recursion guard | per-tick firing count never exceeds 4; one `REFLEX_RECURSION_LIMIT` audit row per exceeded tick | audit scan |
| HTTP refusal (auth/origin/loopback) | HTTP status code matches expected (401/403/404/exit code 2); daemon log carries the matching code | two reads |

### 7.2 The required manual issue-evidence pattern

For each scenario, the issue comment must name the SoT, show its before value, name the trigger, show the separate readback after the trigger, and record the final observed result. Missing the readback or final observed result means the change is not accepted, even if supporting checks pass.

### 7.3 Boundary & edge-case audit — ≥ 3 per primary path

Minimum cases per primary path:

1. **Empty / zero**: `subscribe({kinds: []})` returns valid id; `reflex_list({include_expired: false})` on empty session returns `[]`; `audio_tail({seconds: 0})` returns 0-byte PCM (or `TOOL_PARAMS_INVALID` if schema rejects).
2. **Boundary**: `reflex_register` 32nd reflex succeeds, 33rd returns `REFLEX_CAP_REACHED`; `audio_tail({seconds: 5})` succeeds, `seconds: 6` returns `TOOL_PARAMS_INVALID`; SSE `Last-Event-ID` resume across exactly 4096-event buffer boundary.
3. **Structurally invalid**: `reflex_register({kind: "nonsense"})` → `REFLEX_KIND_INVALID`; `profile_activate({profile_id: "does-not-exist"})` → `PROFILE_NOT_FOUND`; `audio_transcribe({language: "xx"})` → `TOOL_PARAMS_INVALID`.

Storage gets a 4th class: **process-restart durability** — write data, restart
the daemon, read back, and confirm data persists. The disk-pressure scenario
gets a 5th: **all 4 levels must transition deterministically** with a manual
SoT read between each transition. For M3 storage edges, the trigger and SoT
readback are the live `storage_*` MCP tools plus daemon log inspection; do not
replace this with a script, harness, or automated helper.

### 7.4 Trigger → outcome reasoning (doc-comment on every test fn)

```rust
/// Trigger: caller invokes `reflex_register({kind:"on_event", when:..., then:[act_type, act_press(enter)]})`.
/// X (process): m3::reflex::register_in_state → ReflexRuntime::register →
///   audit write to CF_REFLEX_AUDIT → event bus subscribes a new filter →
///   on next matching event scheduler.tick fires then.steps via ActionHandle.
/// Y (outcome, observable): file saved to disk at expected path; audit row
///   appears in CF_REFLEX_AUDIT; tracing log line `code=REFLEX_FIRED` emitted.
/// Sources of truth (4): file bytes, RocksDB CF scan, RecordingBackend
///   events, daemon JSONL log.
```

Trigger = the tool call. X = the process inside the daemon. Y = the observable outcome. The test asserts on **every** source of truth, not just Y's most convenient one.

---

## 8. Manual happy-path + edge-case test plan (run on real Win11 box before tagging `v0.1.0-m3`)

### Happy paths

| # | Steps | Source of truth | Expected |
|---|---|---|---|
| H1 | `subscribe({kinds:["foreground-changed"]})` then alt-tab between Notepad ↔ Calc 5 times | response stream + per-subscription queue length | exactly 10 `foreground-changed` events with the right hwnd ordering |
| H2 | `reflex_register(on_event, when=value-changed of Notepad editor, then=act_press(["ctrl","z"]))`; type 5 chars | UIA `ValuePattern.value` after each char | after each char appended, undo fires; editor value reverts to prior; final value `""` |
| H3 | `reflex_register(aim_track, target=<detected entity stub>, gain=0.5, deadzone=2)`; wait 1s | `GetCursorPos` polled 60 Hz | cursor settles within ±2 px of synthetic entity center within 200 ms |
| H4 | `reflex_register(hold_move, key="w", lifetime={kind:"duration", ms:1500})`; wait 2s | external `WH_KEYBOARD_LL` hook | `KeyDown(w)` at t≈0, `KeyUp(w)` at t≈1500 ms; no further events |
| H5 | `reflex_register(combo, steps=[act_press("e"), at_ms:200 act_press("space")])`; trigger | recording backend events + `RecordingBackend::events()` | exactly 4 events (down/up × 2) at 200 ms intervals ±10 ms |
| H6 | `profile_activate({profile_id:"vscode"})`, open VS Code | `health.subsystems.profiles.active_profile_id` | equals `vscode`; keymap aliases resolvable via `find` |
| H7 | `replay_record({target:"observations", duration_ms:1000})` while moving mouse | file at returned path | non-empty JSONL; each line is a valid `Observation` |
| H8 | `audio_transcribe({seconds:5})` while playing 5 s known clip | response `text` | matches transcript (Levenshtein ≤ 10%) |
| H9 | HTTP transport: `curl -H "Authorization: Bearer $TOKEN" -H "Origin: http://127.0.0.1" http://127.0.0.1:7700/initialize` | response 200 + `Mcp-Session-Id` header | header present; subsequent calls round-trip |
| H10 | SSE stream resume after drop: subscribe; receive 100 events; drop; reconnect with `Last-Event-ID: 50` | replayed event seq numbers | 51..100 inclusive, no gaps |

### Edge cases

| # | Steps | Source of truth | Expected |
|---|---|---|---|
| E1 | `reflex_register` 33 reflexes back-to-back | response | 1-32 succeed; 33rd returns `REFLEX_CAP_REACHED` |
| E2 | `reflex_register(on_event, when=<filter that matches itself>)` to trigger recursion | audit log | per-tick firings clamp to 4; one `REFLEX_RECURSION_LIMIT` audit row per over-tick |
| E3 | `profile_activate({profile_id:"does-not-exist"})` | response | `PROFILE_NOT_FOUND` |
| E4 | Edit `profiles/vscode.toml` to add a deliberate parse error; save | daemon log + `profile_list` | `PROFILE_PARSE_ERROR`; previous valid profile remains active |
| E5 | `storage_put_probe_rows(CF_EVENTS)` to 2× row soft cap; trigger `storage_gc_once` | `storage_inspect` row counts/bytes + daemon log | drops below soft cap; `cache_evictions_total{cf=CF_EVENTS}` incremented |
| E6 | `storage_pressure_sample` with free-byte values for L1→L2→L3→L4; attempt one non-essential and one essential write at L4 | `storage_inspect` transition codes + row counts + daemon log | each transition emits `STORAGE_DISK_PRESSURE_LEVEL_N`; level 4 disables non-essential writes while preserving session/reflex-audit class writes |
| E7 | HTTP: `curl` with bad bearer token | response code | 401; daemon log `code=HTTP_TOKEN_INVALID` |
| E8 | HTTP: `curl -H "Origin: http://evil.example"` from loopback | response code | 403; daemon log `code=HTTP_ORIGIN_REFUSED` |
| E9 | Start daemon with `--bind 0.0.0.0:7700` (without `--allow-non-loopback`) | process exit code + last log line | exits 2; last log line `code=HTTP_BIND_NON_LOOPBACK_REFUSED` |
| E10 | `audio_transcribe` with Whisper-tiny model absent | response | `AUDIO_STT_MODEL_NOT_LOADED` |
| E11 | Press `Ctrl+Alt+Shift+P` (operator hotkey) while 3 reflexes active and `act_press(hold_ms=10000)` running | external keyboard hook + reflex_list + daemon log | within 50 ms: every held key released; all reflexes status=`disabled`; log carries `SAFETY_OPERATOR_HOTKEY_FIRED` |
| E12 | SSE stream: producer drops 5000 events in 100 ms (slow consumer) | next push frame metadata | one frame carries `lossy: true`; `subscription_started` re-sent; `events_dropped_for_subscriber` metric increments by exactly the overflow count |

For each row the operator pastes both the structured response and the source-of-truth read-back into the PR description. **No row is "ok by inspection."**

---

## 9. Synthetic-input fixtures (the test contract)

Pick inputs whose expected outputs are unambiguous; tests assert on them exactly, not on fuzzy matches.

| Synthetic input | Subsystem | Expected source-of-truth state |
|---|---|---|
| `Event { kind: "value-changed", source: "uia", data: {window_id: 0x42, element_id: "0x42:0x00", new_value: "x"} }` × 100 | event bus | exactly 100 entries in per-subscriber queue; coalesce window discards duplicates within 50 ms (assert via `coalesce_events`) |
| Reflex `on_event` filter `Kind("value-changed") AND Data.window_id == 0x42` | filter eval | matches the above 100; rejects a synthetic event with `window_id: 0x43` |
| 5000 events in 100 ms to a 4096-buffer subscriber | bus drop-oldest | queue len 4096; dropped count exactly 904; `events_dropped_for_subscriber` metric == 904 |
| 33 `reflex_register` calls | reflex cap | calls 1-32 OK; 33rd `REFLEX_CAP_REACHED` |
| Combo `[act_press("a") at_ms:0, act_press("b") at_ms:100]` | combo scheduler | recording events: `[KeyDown(a)@0, KeyUp(a)@33, KeyDown(b)@100±5, KeyUp(b)@133±5]` |
| RocksDB: 10 000 PUT (`CF_EVENTS`, key=ts_le, val=jsonb) | write batcher | wall ≤ 200 ms; `Db::scan(CF_EVENTS, all)` returns 10 000 rows; sum(bytes) == expected |
| Profile TOML w/ `schema_version = 999` | parser | `PROFILE_VERSION_INCOMPATIBLE` |
| Profile TOML missing `[matches]` | parser | `PROFILE_PARSE_ERROR` |
| Profile TOML w/ `mouse_curve_default = "instant"` | natural-defaults gate | smoke test fails locally |
| Whisper-tiny test clip: `tests/fixtures/audio/hello_world_5s.wav` (transcript "Hello world. This is Synapse.") | STT | response `text` within Levenshtein ≤ 4 chars |
| WASAPI loopback while playing `tests/fixtures/audio/loud_transient_1s.wav` | detectors | one `loud_transient` event with rms > -6 dBFS |
| Stereo `tests/fixtures/audio/pan_minus60_0_plus60.wav` | direction | three direction estimates: -60° ±15°, 0° ±15°, +60° ±15° |
| HTTP `POST /initialize` no `Authorization` | auth | 401 + log `HTTP_TOKEN_INVALID` |
| HTTP `POST /initialize` `Origin: http://attacker.example` from loopback | origin check | 403 + log `HTTP_ORIGIN_REFUSED` |

---

## 10. Acceptance gates (block M4)

```
✓ M3 demo passes (Notepad save-dialog reflex; §2) via stdio AND HTTP w/ token
✓ Manual H1-H10 happy paths all green; operator pastes source-of-truth read-back in PR
✓ Manual E1-E12 edge cases all match expected outcome
✓ Bench reflex_tick_jitter_idle p99 ≤ 200 µs (07 §1)
✓ Bench reflex_tick_jitter_under_load p99 ≤ 500 µs
✓ Bench event_to_subscriber p99 ≤ 50 ms
✓ Bench observe_warm_hybrid p99 still ≤ 30 ms (no regression from M1)
✓ Bench action_software_press p99 still ≤ 3 ms (no regression from M2)
✓ Disk-pressure scenario passes through all 4 levels deterministically
✓ Profile hot-reload picks up edits in ≤ 1 tick (200 ms debounce)
✓ All bundled profiles satisfy Natural-defaults invariant (07 §12)
✓ HTTP transport: bearer auth + Host/Origin + SSE resume + Mcp-Session-Id work end-to-end
✓ Every M3 error code (declared + new) thrown ≥ 1× in a test that asserts data.code
✓ tools/list snapshot updated to 30 tools (15 prior + 15 M3); `additionalProperties:false` on every schema
✓ No mocks gate completion — real RocksDB on real disk, real WASAPI on real device, real Notepad in E2E
✓ Local supporting checks green; manual configured-host FSV is the shipping gate (issues #246/#247/#350/#351)
✓ Manual evidence: issue resolution records source-of-truth before/after state plus final observed result values per scenario
✓ Soak (1 h) clean: no memory growth > 50 MB, no deadlocks, no held-key leaks after `release_all`
✓ scripts/check_docs.ps1 green
✓ CHANGELOG.md updated with M3 entry; tag v0.1.0-m3 cut
```

---

## 11. Risks (`15 §9` + extras)

| Risk | Mitigation |
|---|---|
| Time-critical thread jitter on Windows | `CreateWaitableTimerEx` w/ `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` + MMCSS Pro Audio; fallback to `tokio::time::interval(2ms)` logs `degraded` and fires spike check |
| RocksDB Windows hiccups (`OQ-001`) | M3 uses RocksDB only per ADR-0002; alternate backend requires a fresh implementation issue |
| Hot-reload vs. active reflexes | Reflex params snapshot at registration; profile alias resolution happens at register-time; subsequent profile changes don't retroactively break running reflexes; missing alias on fire ⇒ `REFLEX_PARAMS_INVALID` |
| HTTP/SSE reconnect semantics | `Last-Event-ID` header on reconnect; buffer 4096/sub; deeper outage ⇒ subscription marked `lossy=true` in next push |
| Whisper-tiny accuracy weaker than expected (`OQ-014`) | Operator opt-in upgrade to `whisper-base` via separate `models import` flow; bundled-default decision deferred to M5 |
| Multi-monitor profile match (`OQ-012`, ADR-0005) | one active capture target per session; agent picks via `set_capture_target`; monitor changes do not implicitly switch active profile |
| EventFilter eval blow-up on large filters | filter depth limited to 8 (configurable); deeper trees rejected at registration with `REFLEX_FILTER_INVALID` |
| Reflex starvation under conflicting priorities | logged via `REFLEX_STARVED` after 2 s of contended ticks; status reflected in `reflex_list` |
| M2 carry-over (#244, #234, #233, #231) | Block A.0 fixes them **first**; M3 work does not build on the buggy paths |
| LoC cap re-violations | enforce hard split at 450 LoC (50 LoC of margin) during code review; the M2 carry-over PRs (A.0a/A.0b) prove the discipline |

---

## 12. Out of scope at M3 (deferred ≥ M4)

- Hardware HID backend (`Backend::Hardware` keeps surfacing `ACTION_BACKEND_UNAVAILABLE`)
- `act_combo` standalone tool (M4; M3 combos register via `reflex_register(kind: combo, ...)`)
- `act_run_shell`, `act_launch` (M4, gated via `--allow-shell` / `--allow-launch`)
- RP2040 firmware (M4)
- Minecraft profile + HUD template-match runtime (M4)
- VLM `describe` (M5, Florence-2; downloaded on first call)
- Debug overlay (M5)
- Installer / MSI / setup wizard (M5)
- Profile signing / marketplace (v2; `OQ-007`)
- Permission system beyond loopback + bearer + Origin (v1.x; `OQ-006`)

---

## 13. Definition of Done

Closed 2026-05-25 by `v0.1.0-m3` (commit `97019ec`). The §2 demo gate, the §10 acceptance gates, the §8 manual happy-path + edge-case plan, the CHANGELOG entry, and the tag are all on `main`. M4 starts against this state without waiting on a self-hosted runner; manual configured-host FSV remains the shipping evidence (see `00_methodology.md` §5).

Open next: `05_m4_hardware_hid_first_game.md` (active phase).

---

## Appendix A — Trigger → outcome map (audit framework)

When debugging, identify the row, read both columns, the bug is in X:

| Trigger | Process X | Outcome Y (observable) | Source of truth |
|---|---|---|---|
| `tools/call subscribe` | EventBus inserts a Subscriber; SSE thread starts pushing | Subscriber receives queued events | per-sub queue drain + SSE stream bytes |
| `tools/call reflex_register` | ReflexRuntime adds to active set; audit write to CF_REFLEX_AUDIT | Reflex fires on matching event | audit row + RecordingBackend events + tracing log |
| `tools/call reflex_cancel` | ReflexRuntime removes from active set; audit row `status=cancelled` | Reflex stops firing | `reflex_list` excludes the id; audit row |
| `tools/call profile_activate` | ProfileRuntime replaces active profile; emits `profile-activated` event | `health.subsystems.profiles.active_profile_id` updates | health tool + tracing log |
| `tools/call replay_record` | spawn task reading event/observation bus for N ms; write JSONL file | file at `path` contains the records | `fs::read_to_string(path)` |
| `tools/call audio_transcribe` | grab last N s from loopback ring; feed to Whisper; return text | response carries text | response field; Levenshtein vs ground truth |
| HTTP request | axum router → auth/origin check → rmcp dispatch | response status / body / Mcp-Session-Id | HTTP response |
| SSE drop + resume | reconnect with Last-Event-ID; server replays from ring | client receives missed events | event sequence numbers contiguous |
| Operator hotkey | RegisterHotKey delivers WM_HOTKEY → fire ReleaseAll + disable all reflexes | every held key released; reflexes disabled | external keyboard hook + reflex_list + log |
| Disk-pressure transition | pressure responder polls df every 30 s → emit level event | `STORAGE_DISK_PRESSURE_LEVEL_N` emitted; non-essential writes paused at L4 | df + event drain |

## Appendix B — Where to look when something breaks (root-cause-first)

| Symptom | First file to read |
|---|---|
| New tool missing from `tools/list` | `crates/synapse-mcp/src/server.rs` `#[tool_router]` block |
| Reflex never fires | `crates/synapse-reflex/src/kinds/on_event.rs` filter eval + `bus.rs` subscriber path |
| Audit row missing after fire | `crates/synapse-reflex/src/audit.rs` write batch flush (must be ≤ 100 ms or explicit) |
| Subscriber drops events | `crates/synapse-reflex/src/bus.rs` per-sub bounded channel (4096) — confirm slow-consumer policy |
| HTTP 401 spurious | `crates/synapse-mcp/src/http/auth.rs` token compare (constant-time) + `%APPDATA%\synapse\token.txt` perms |
| SSE replay missing events | `crates/synapse-mcp/src/http/sse.rs` ring buffer + `Last-Event-ID` parse |
| `cargo deny check` fails on new dep | `deny.toml` SPDX allowlist — every M3 dep is already in `[workspace.dependencies]`; no new SPDX exposure |
| Held key not released after panic | `crates/synapse-action/src/safety.rs` — confirm `RELEASE_ALL_HANDLE` was set before reflex started |
| WASAPI loopback init fails | check `wasapi::initialize_mta()` order; `synapse-audio::loopback` must run on a dedicated thread |
| RocksDB open error on Win | check pinned `rocksdb` crate and `multi-threaded-cf`; file a new backend issue only if this configured host needs it |

## Appendix C — Occam's razor recap

The single simplest description of M3: **fill out four empty crates, add the 15
M3 MCP tools, add an HTTP transport with SSE, and write to a real on-disk
RocksDB.** Every other clause traces back to that sentence plus the global
invariants (no backcompat, no mocks gate completion, Natural-only motion,
manual FSV is the shipping gate). If a design doesn't trace back, it's wrong.
