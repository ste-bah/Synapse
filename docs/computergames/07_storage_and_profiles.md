# 07 — Storage and Profiles

## 1. Storage philosophy

Single-machine, single-tenant. Storage exists for:

1. **Replay debugging.** Every event, action, reflex firing persists for deterministic replay.
2. **Caches.** OCR results, downloaded models, profile loads — expensive items live in RocksDB after first access.
3. **Session continuity.** Session-id keyed view lets reconnecting clients resume subscriptions and rediscover reflexes.

Never persisted: captured frames (replay log keeps small diff hashes only), audio waveforms (event metadata only), raw UIA tree snapshots beyond the 1-Hz `CF_OBSERVATIONS` sample, the agent's MCP request payloads beyond minimal trace metadata.

Storage is **wipe-friendly**. Pre-v1 schema changes = wipe and rebuild. Post-v1: schema changes require ADR + tooling.

---

## 2. Backend choice: RocksDB

RocksDB via the `rocksdb` crate. Pinned version with `features = ["multi-threaded-cf"]`.

Why RocksDB: mature on Windows (`bzip2`/`zlib` C dep is unavoidable but stable); column families scope reads/writes precisely; TTL/compaction filters give cheap rolling retention; snapshot reads for replay tool.

M3 removed the unused sled escape valve per ADR-0002. A future fallback backend requires a maintained dependency graph, implemented adapter, and manual source-of-truth verification on the configured Windows host.

---

## 3. Database location

```
default:    %LOCALAPPDATA%\synapse\db\
override:   --db <path>     (CLI flag)
            SYNAPSE_DB_PATH (env)
            config.toml: db.path
```

```
%LOCALAPPDATA%\synapse\
├── db\                            # RocksDB instance
├── models\                        # ONNX model cache (separate from CF_MODEL_CACHE for very large files)
├── profiles\                      # User-installed profiles override bundled
├── replay\                        # Replay session exports (manual)
├── logs\                          # tracing logs
└── config.toml                    # operator-level config
```

---

## 4. Column families

| CF | Key | Value | Encoding | TTL | Soft cap | Hard cap | Notes |
|---|---|---|---|---|---|---|---|
| `CF_EVENTS` | `[at_ns u64 BE][seq u32 BE]` | `StoredEvent` | json | 24h | 2 GB | 4 GB | Append-only ring; profile activation/denial and future replay events |
| `CF_OBSERVATIONS` | `[seq u64 BE]` | `StoredObservation` | json | 6h | 500 MB | 1 GB | 1Hz sample + reason-triggered snapshots |
| `CF_PROFILES` | `[profile_id utf8]` or `[profile_quality/v1/<profile_id>]` | cached TOML/profile-registry rows | raw bytes/json | none | 20 MB | 50 MB | Cached load plus local profile-quality snapshots; authored profile source remains on-disk TOML |
| `CF_MODEL_CACHE` | `[model_sha256 32 bytes]` | model bytes | raw bytes | LRU when full | 1 GB | 2 GB | Downloaded ONNX models, sha-verified |
| `CF_SESSIONS` | `[session/v1/<session_id>]` | `StoredSession` | json | 30d | 50 MB | 100 MB | One row per MCP audit session, including active profile history |
| `CF_REFLEX_AUDIT` | `[reflex_id][audit_id]` | `StoredReflexAudit` | json | 7d | 200 MB | 500 MB | Per-reflex audit with optional profile/session context |
| `CF_OCR_CACHE` | `[image_sha256 32 bytes]` | `OcrResult` | json | 1h | 50 MB | 100 MB | Memoization of OCR on stable regions |
| `CF_TELEMETRY` | `[metric_name utf8][at_ns u64 BE]` | `f64 LE` | raw 8 bytes | 6h | 100 MB | 200 MB | Local metric ringbuffer |
| `CF_ACTION_LOG` | `[at_ns u64 BE][seq u32 BE]` | profile-linked action audit JSON | json | 24h | 200 MB | 500 MB | Every action start/result row, linked to active profile/session when available |
| `CF_PROCESS_HISTORY` | `[at_ns u64 BE][pid u32]` | json | json | 6h | 20 MB | 50 MB | Process started/exited events |
| `CF_KV` | `[utf8]` | bytes | raw | none | 10 MB | 50 MB | Generic key-value extension |

Default DB size budget **~4 GB** including write-amplification. Soft cap → start aggressive expiry. Hard cap → refuse writes; surface `STORAGE_CF_HARD_CAP_REACHED`.
`CF_OCR_CACHE` is recomputable pre-v1 data: schema changes such as the M3 `OcrResult`
shape are handled by wiping/rebuilding cached OCR entries, not by compatibility shims.

Retention is operator-configurable in `config.toml`:

```toml
[retention.cf_events]
ttl_hours = 24
soft_cap_mb = 2048
hard_cap_mb = 4096

[retention.cf_ocr_cache]
ttl_hours = 1
soft_cap_mb = 50
hard_cap_mb = 100
```

Defaults are research-friendly. Lower for small disks; raise for forensic debugging. `pub const`s for CF names live in `synapse-storage::cf`; a test asserts match with on-disk strings — drift fails the local docs gate.

### 4.1 Key encoding rules

- Time-keyed CFs (`CF_EVENTS`, `CF_OBSERVATIONS`, `CF_REFLEX_AUDIT`, `CF_ACTION_LOG`, `CF_TELEMETRY`, `CF_PROCESS_HISTORY`): `u64` big-endian for natural sort. Add `seq` or `pid` suffix for uniqueness.
- ID-keyed CFs: UTF-8 strings (UUIDs as canonical hex). Inspectable with `ldb`.
- Hash-keyed CFs: raw 32-byte sha256.

### 4.2 TTL implementation

Per-CF compaction filter drops records older than TTL. Implemented in `synapse-storage::compaction`. Runs on every compaction; effective expiry within ~1 hour of nominal TTL.

Filters consult runtime config so TTL changes apply on next compaction without restart. Each filter is a small `Box<dyn CompactionFilter>`; decodes only the timestamp portion for speed. Active retention is exported through the `health` MCP tool and Prometheus endpoint.

### 4.3 Write batches

All writes go through `Db::write_batch(Batch)` to minimize fsync cost. Storage task batches writes from a `mpsc::Receiver<WriteOp>` channel; flush triggers: every 100 ms (idle), every 64 KB accumulated, every explicit `Db::flush()` call (session close, after `act_run_shell`).

---

## 5. Replay log semantics

`CF_EVENTS` is the canonical replay source. Combined with `CF_OBSERVATIONS` it reconstructs any past 24h of session activity.

Replay tool: `synapse-mcp replay --session <id> [--speed 1.0] [--out <dir>]`:

1. Reads `CF_SESSIONS` row for the id.
2. Reads `CF_EVENTS` and `CF_ACTION_LOG` for the session's time range.
3. Reads matching `CF_OBSERVATIONS` snapshots.
4. Produces a JSONL transcript + a Synapse Web Replay (SWR) bundle.
5. Optional `--simulate-actions` replays actions against the live machine.

SWR bundle: single `.zip` containing the JSONL transcript, extracted observation snapshots, and the active profile at the time. Self-contained; shippable for bug reports.

---

## 6. Data lifecycle and cleanup (the contract)

Binding policy for what gets persisted, how long, and how it's removed. The agent rarely cares about data older than minutes; the AI's working memory lives in its context, not Synapse's DB. Synapse stores only what's useful for **debugging, replay, caching, and short rolling history**.

### 6.1 Data classes

| Class | Storage | Retention | Example |
|---|---|---|---|
| **Ephemeral hot** | RAM only | Until consumed or dropped | Captured frames, audio ring buffer, event-bus backlog, in-flight detection results |
| **Short-term durable** | RocksDB with aggressive TTL | hours to days | `CF_EVENTS` (24h), `CF_ACTION_LOG` (24h), `CF_OBSERVATIONS` (6h), `CF_TELEMETRY` (6h) |
| **Cache** | RocksDB with LRU + TTL | until evicted | `CF_OCR_CACHE` (1h), `CF_MODEL_CACHE` (LRU 1GB), `CF_PROFILES` (none, tiny) |
| **Audit / long-lived** | RocksDB, longer TTL | days to weeks | `CF_SESSIONS` (30d), `CF_REFLEX_AUDIT` (7d) |

**Nothing is persisted forever.** Every CF either has a TTL or is a bounded cache. The two non-TTL CFs (`CF_PROFILES`, `CF_KV`) are tiny by design.

### 6.2 Why these retentions

| CF | Retention | Why |
|---|---|---|
| `CF_EVENTS` | 24h | Replay debugging mostly on "last session." Older = forensic; export. |
| `CF_OBSERVATIONS` | 6h | 1 Hz × 6h = 21,600 snapshots; reconstructs any recent session. |
| `CF_ACTION_LOG` | 24h | Same as events. |
| `CF_REFLEX_AUDIT` | 7d | Reflex debugging crosses sessions. |
| `CF_SESSIONS` | 30d | Small; long-term usage analysis. |
| `CF_OCR_CACHE` | 1h | Recomputable. High-churn region. |
| `CF_MODEL_CACHE` | LRU only | Models expire by disuse, not age. |
| `CF_TELEMETRY` | 6h | Local fallback. Push to OTLP/Prometheus for long-term. |
| `CF_PROCESS_HISTORY` | 6h | Useful for "what ran an hour ago when X happened." |

### 6.3 Three layers of cleanup

Three independent mechanisms; none block the hot path.

**Layer 1 — RocksDB compaction filters.** Per-CF filter drops expired rows during natural compaction. Cheapest; handles ~95% of expirations.

**Layer 2 — Periodic GC task.** `storage_gc` tokio task every 5 minutes:

1. For each CF with a soft cap, query `db.property_int_value("rocksdb.estimate-live-data-size")`.
2. If estimated size > soft cap, run `DeleteRange` against the oldest 25% of keys.
3. Request compaction over that range so disk reclaims promptly.
4. Update `cache_evictions_total{cf}` and `cf_size_bytes{cf}`.

Bounded work per tick; no single tick exceeds 100 ms CPU. If soft cap is grossly exceeded (>2×), tighter intervals (1 min) until back under.

**Layer 3 — Disk-pressure responder.** `storage_disk_pressure` task wakes when DB-volume free disk drops below 2 GB:

1. **Level 1 (free < 2 GB):** Tighten all TTLs to 50% of nominal (events 24h → 12h). Emit `STORAGE_DISK_PRESSURE_LEVEL_1`.
2. **Level 2 (free < 1 GB):** Drop cache CFs entirely (`CF_OCR_CACHE`, `CF_TELEMETRY`, `CF_PROCESS_HISTORY`). Re-tighten TTLs to 25%. `STORAGE_DISK_PRESSURE_LEVEL_2`.
3. **Level 3 (free < 500 MB):** Halt new writes to non-essential CFs (telemetry, OCR cache, model cache for new downloads). Surface `STORAGE_WRITE_FAILED`. `STORAGE_DISK_PRESSURE_LEVEL_3`.
4. **Level 4 (free < 200 MB):** Refuse new MCP sessions. Existing sessions get a one-line warning event. Action emission continues (don't strand held inputs).

State transitions debounce — level change requires free-space change to persist for 30 seconds.

### 6.4 Session-end cleanup

On MCP session end (clean close, transport drop, process shutdown):

1. Cancel all reflexes registered by that session (each logged to `CF_REFLEX_AUDIT`).
2. Close all open subscriptions.
3. Emit `release_all` for any held inputs.
4. Write `closed_at` to the session's `CF_SESSIONS` row.
5. Schedule session's `CF_OBSERVATIONS` snapshots for short-half-life cleanup (most expire 6h after capture; session-end-marked get 2h half-life).

No per-session DB-wipe needed; TTL mechanism handles cleanup.

### 6.5 Application-level retention overrides

Agent can mark data for shorter or longer retention:

```
observe(include=["focused","elements"], retain_hint="ephemeral")     # do not write CF_OBSERVATIONS row
observe(include=["focused","elements"], retain_hint="bookmark")      # extend this snapshot to 30d retention
```

Default `retain_hint` is `"standard"` (CF's TTL). Bookmarked observations live in a `bookmark:` sub-prefix of `CF_OBSERVATIONS` with its own 30-day TTL. Cap 100 bookmarks per session.

### 6.6 Cache management

| Cache | Eviction trigger | Size cap |
|---|---|---|
| `CF_OCR_CACHE` | TTL 1h + LRU when > 50 MB | 50 MB |
| `CF_MODEL_CACHE` | LRU when > 1 GB; ONNX never used in last 30d evicted first | 1 GB |
| `CF_PROFILES` | None (tiny) | Few MB |
| `CF_TELEMETRY` | TTL 6h + LRU when > 200 MB | 200 MB hard cap |

LRU bookkeeping piggybacks on the row's `last_read_at_ns` updated on read. Updates batched per 10 reads to avoid write amplification.

Eviction runs in the storage GC task. Tracked via `cache_evictions_total{cf, reason}` (`reason ∈ {ttl, lru, soft_cap, disk_pressure}`), `cache_hit_ratio{cf}` (rolling 1h), `cf_size_bytes{cf}`.

### 6.7 RocksDB-level space reclamation

Deleting a key writes only a tombstone; disk reclaims on compaction. Synapse triggers compaction explicitly after aggressive deletions: GC layer-2 work, `DeleteRange` operations, disk-pressure level-2 cleanup, `synapse-mcp db compact`.

Scheduled background compaction runs nightly (configurable: `[storage] nightly_compaction_hour = 3`).

### 6.8 Operator-facing visibility

```bash
$ synapse-mcp db status
DB path:      C:\Users\alice\AppData\Local\synapse\db
Total size:   1.42 GB on disk (live: 1.05 GB, garbage: 0.37 GB)
Disk free:    87.4 GB on volume C:

Column family       Live MB   TTL    Soft  Hard   Status
CF_EVENTS             842.1   24h   2048  4096   OK
CF_OBSERVATIONS       104.5   6h     500  1000   OK
CF_ACTION_LOG          18.2   24h    200   500   OK
CF_REFLEX_AUDIT         3.1   7d     200   500   OK
CF_SESSIONS             0.4   30d     50   100   OK
CF_OCR_CACHE           47.8   1h      50   100   warning (95% of soft)
CF_TELEMETRY           38.9   6h     100   200   OK
CF_MODEL_CACHE        612.3   LRU   1024  2048   OK
CF_PROCESS_HISTORY      0.3   6h      20    50   OK
CF_PROFILES             0.1   none    20    50   OK
CF_KV                   0.0   none    10    50   OK

Pressure level:  0 (healthy)
Last compaction: 2026-05-22 03:00 UTC (4h 22m ago)
Last GC:         2026-05-22 07:18 UTC (3m 42s ago)
```

```bash
$ synapse-mcp db gc --aggressive       # immediate full GC pass
$ synapse-mcp db compact               # force compaction
$ synapse-mcp db trim --cf CF_EVENTS --keep-hours 6
$ synapse-mcp db wipe --yes            # nuclear option
```

### 6.9 What is NEVER persisted

Never written to disk regardless of TTL:

- **Captured frame pixels.** Only metadata + dirty-region SHA hashes for replay matching.
- **Audio waveforms.** Only event metadata + summarized direction estimates.
- **Full UIA snapshots beyond the 1 Hz observation samples.** Live tree state stays in RAM.
- **Raw model intermediate tensors.** Only inference results.
- **Free-form clipboard content beyond a 120-char redacted excerpt.**
- **HTTP/WS transport message bodies.** Only trace metadata.

RAM-only; live as long as the immediate consumer needs them.

### 6.10 Log file rotation

`tracing` JSON logs in `%LOCALAPPDATA%\synapse\logs\synapse.log` rotate via `tracing-appender::rolling`: daily, keep 7 days, compress to `.log.gz` after rotation, total log dir cap 500 MB (oldest beyond pruned). Logs at `info` level. Per-subsystem logs follow the same policy.

### 6.11 Replay export retention

`%LOCALAPPDATA%\synapse\replay\` holds operator-exported SWR bundles via `synapse-mcp replay export`. Synapse never writes here automatically. Operator-managed; size surfaced in `db status`; never auto-deleted. Folders > 5 GB get a warning in `db status`.

### 6.12 Best practices summary

For developers extending Synapse:

1. **Default to ephemeral.** RAM-only unless explicit replay/debug reason.
2. **Pick a TTL upfront.** Every new CF declares TTL + soft cap + hard cap in `synapse-core::retention::DEFAULTS`.
3. **Don't write per-frame.** Aggregate. 60 fps = 5,184,000 events per day if per frame. Batch.
4. **Use write batches.** Many small writes → one batch flush every 100 ms.
5. **JSON for persisted typed records.** Bincode is excluded by ADR-0001 / RUSTSEC-2025-0141; prefer inspectable JSON until a maintained binary codec is explicitly selected.
6. **Don't store what you can recompute cheaply.** Detection results, OCR results, panel materializations.
7. **For long-term retention, push to external storage** (OTLP for metrics, replay export for events).
8. **Compaction filter, then GC task, then disk pressure.** Layer cleanup.
9. **Surface CF size in `health` and Prometheus.** Operators need to see what's growing.
10. **Verify storage through live MCP readbacks.** Manual configured-host FSV
    uses `storage_inspect`, `storage_put_probe_rows`, `storage_gc_once`, and
    `storage_pressure_sample` against the running daemon, then reads row counts,
    bounded row samples, pressure transition codes, and daemon logs after each
    trigger. A constrained DB target or low-volume setup may support
    investigation, but it is not a substitute for the live MCP trigger plus
    separate source-of-truth readback.

---

## 7. Operator-visible operations

| Command | Effect |
|---|---|
| `synapse-mcp db status` | DB path, total size, per-CF row count + bytes |
| `synapse-mcp db wipe` | Drops everything; confirms `--yes` flag |
| `synapse-mcp db backup <out>` | Hot backup via RocksDB checkpoint API |
| `synapse-mcp db restore <in>` | Stops daemon, restores from backup, restarts |
| `synapse-mcp db compact` | Force compaction across all CFs |
| `synapse-mcp models list` | Inventory of cached models |
| `synapse-mcp models import <path>` | Side-load a model file |
| `synapse-mcp models gc` | Drop unreferenced models |

The live M3 daemon also exposes storage diagnostic MCP tools used for manual
FSV:

| MCP tool | Effect |
|---|---|
| `storage_inspect` | Reads schema version, pressure level, transition codes, per-CF row counts, logical bytes, and bounded newest-row samples |
| `storage_put_probe_rows` | Writes bounded synthetic rows to `CF_EVENTS`, `CF_OBSERVATIONS`, `CF_SESSIONS`, `CF_ACTION_LOG`, or `CF_KV` and flushes |
| `storage_gc_once` | Runs one row-cap GC pass for a diagnostic CF |
| `storage_pressure_sample` | Applies one synthetic free-byte sample through the production disk-pressure responder |

These tools are operator diagnostics for direct state verification. They are not
FSV automation, and their responses must be followed by separate SoT reads when
used as acceptance evidence.

Real action tools write profile-linked JSON rows to `CF_ACTION_LOG`. For
manual action FSV, read `storage_inspect.cf_row_counts.CF_ACTION_LOG` before
and after the action, then read
`storage_inspect.cf_row_samples.CF_ACTION_LOG` and record the actual sampled
JSON row. Rows include the MCP audit session id, active profile id/version/
schema, foreground profile/schema, backend policy, app/game context, status,
error code, and redaction fields when profile resolution is available. A count
delta alone is not enough evidence.

M5 profile-linked audit rows extend that contract across `CF_SESSIONS`,
`CF_EVENTS`, `CF_ACTION_LOG`, and `CF_REFLEX_AUDIT`. A successful
`profile_activate` starts or updates the local MCP audit session, writes the
current `StoredSession`, and writes a `profile.activated` `StoredEvent`.
Action audit rows and reflex audit rows then carry the same `audit_context`
payload where the active profile is known: session id, profile id/version/
schema, backend policy, foreground app/game context, redaction flag, and
redaction spans. Profile-denied paths write `profile.activation_denied` events
with the attempted profile id and current session context when one exists.

For manual FSV of this linkage, trigger the real MCP tool (`profile_activate`,
an action tool such as `act_press`/`act_combo`, and a reflex path), then read
`storage_inspect.cf_row_counts` and the 4096-character `cf_row_samples` for
`CF_SESSIONS`, `CF_EVENTS`, `CF_ACTION_LOG`, and `CF_REFLEX_AUDIT`. The row
samples must show the same `session_id`/`profile_id` chain and the expected
redaction fields; return values alone do not prove the row exists.

`profile_quality_refresh` is the first profile-registry/audit-data scoring
surface. It reads real `CF_ACTION_LOG` rows, ignores stale/corrupt/non-matching
audit rows for scoring, writes a redacted JSON snapshot to
`CF_PROFILES` under `profile_quality/v1/<profile_id>`, and reads that exact row
back before returning. The snapshot stores counts, rates, a Wilson 95% lower
bound score for foreground-profile `ok` vs `error` outcomes, compatibility
signals, profile-schema-version recency/mixed-version counters, source row
range, evidence hash, and a local-only contribution policy. It never exports or
shares data; contribution bundles require a future
operator-approved path and the governance metadata defined in
[`20_profile_registry_governance.md`](20_profile_registry_governance.md):
license SPDX expression, attribution, provenance, revocation state, redaction
policy, and operator consent id.

The M5 runtime registry surface now includes `profile_registry_search`,
`profile_registry_inspect`, `profile_registry_install`,
`profile_registry_disable`, `profile_registry_export`,
`profile_registry_import`, and `audit_intelligence_query`. These tools operate
on the physical `CF_PROFILES` / `CF_KV` row namespaces below and return exact
row keys or bundle paths so manual FSV can trigger the real MCP tool and then
separately read the stored RocksDB rows or filesystem bundle.

### 7.1 Local profile registry row namespaces

The local profile registry uses existing `CF_PROFILES` JSON rows for M5 v0; no
new RocksDB column family is introduced until runtime access patterns prove one
is needed. `CF_KV` is reserved only for tiny registry head/pointer rows.

| Row kind | CF | Key |
|---|---|---|
| Registry source | `CF_PROFILES` | `profile_registry/v1/source/<source_id>` |
| Profile package | `CF_PROFILES` | `profile_registry/v1/package/<package_id>/<package_version>` |
| Profile version | `CF_PROFILES` | `profile_registry/v1/profile/<profile_id>/<profile_version>` |
| Installed profile | `CF_PROFILES` | `profile_registry/v1/installed/<profile_id>` |
| Compatibility target | `CF_PROFILES` | `profile_registry/v1/compat/<target_id>/<profile_id>/<profile_version>` |
| Quality link | `CF_PROFILES` | `profile_registry/v1/quality_link/<profile_id>/<profile_version>` |
| Registry head pointer | `CF_KV` | `profile_registry/v1/head/<source_id>` |

The data model and synthetic row fixtures are defined in
[`22_profile_registry_data_model.md`](22_profile_registry_data_model.md).
Profile package manifest bytes and fail-closed parser validation are defined in
[`23_profile_package_manifest.md`](23_profile_package_manifest.md). Successful
runtime install tools must write the manifest path/digest into the package row
and then read `CF_PROFILES` separately during manual FSV.

---

## 8. Profile system

Profiles drive per-app and per-game behavior. Synapse ships bundled profiles; operators can add more.

### 8.1 Profile directories (precedence high → low)

1. **`--profile-dir <path>`** (CLI override)
2. **`%APPDATA%\synapse\profiles\`** (user-installed)
3. **`profiles/`** beside the executable (bundled)

Files are `<id>.toml`. ID is basename without extension and matches the `id` field. Mismatch → `PROFILE_PARSE_ERROR`.

### 8.2 TOML schema

Concrete TOML mapped to the `Profile` Rust struct from `06_data_schemas.md`. Productivity example:

```toml
# profiles/vscode.toml
id = "vscode"
label = "Visual Studio Code"
version = "1.0.0"
use_scope = "productivity"

[[matches]]
exe = "Code.exe"

[[matches]]
exe = "VSCode.exe"

mode = "a11y_only"

[capture]
target = { kind = "foreground_window" }
min_update_interval_ms = 100
cursor_visible = true

[detection]
model_id = "none"             # disable detection for this app
classes_of_interest = []
confidence_threshold = 0.0
max_detections = 0

[ocr]
default_backend = "winrt"

# no HUD fields for VS Code
hud = []

[keymap]
save = "ctrl+s"
quick_open = "ctrl+p"
command_palette = "ctrl+shift+p"

[backends]
default = "software"
keyboard_default = "software"
mouse_default = "software"
pad_default = "vigem"

[metadata]
benchmark_id = "minecraft.java"
"supported_use.local_world_only" = "true"
"supported_use.remote_server_allowed" = "false"
```

Game example:

```toml
# profiles/minecraft.java.toml
id = "minecraft.java"
label = "Minecraft Java Edition"
version = "1.0.0"
use_scope = "single_player"

[[matches]]
exe = "javaw.exe"
title_regex = "Minecraft\\* [0-9]"

mode = "pixel_only"

[capture]
target = { kind = "foreground_window" }
min_update_interval_ms = 16
cursor_visible = true

[detection]
model_id = "yolov10n_general"
classes_of_interest = ["player", "zombie", "skeleton", "creeper", "villager"]
confidence_threshold = 0.45
max_detections = 32

[ocr]
default_backend = "winrt"

[[hud]]
name = "hp_hearts"
extractor = { kind = "template_match", templates = ["hearts/full.png", "hearts/half.png", "hearts/empty.png"] }
parser = { kind = "number" }
region = { kind = "anchored_to_edge", edge = "bottom_left", x_offset = 220, y_offset = -50, w = 180, h = 18 }

[[hud]]
name = "hunger"
extractor = { kind = "template_match", templates = ["hunger/full.png", "hunger/half.png", "hunger/empty.png"] }
parser = { kind = "number" }
region = { kind = "anchored_to_edge", edge = "bottom_right", x_offset = -400, y_offset = -50, w = 180, h = 18 }

[keymap]
forward = "w"
back = "s"
left = "a"
right = "d"
jump = "space"
sneak = "shift"
sprint = "ctrl"
attack = "lmb"
place = "rmb"
inventory = "e"
drop = "q"
chat = "t"
hotbar1 = "1"
hotbar2 = "2"
hotbar3 = "3"

[backends]
default = "software"
keyboard_default = "software"
mouse_default = "software"
pad_default = "vigem"

[[event_extensions]]
name = "creeper_nearby"
from_filter = { op = "and", args = [
    { op = "kind", kind = "entity-appeared" },
    { op = "data", path = "/class_label", predicate = { op = "eq", value = "creeper" } },
    { op = "data", path = "/bbox/w", predicate = { op = "gt", value = 80 } }
] }
emits_kind = "creeper-imminent"
```

`event_extensions` rewrites/derives custom events the agent can subscribe to or use in `on_event` reflexes. Evaluated by perception.

### 8.3 Match precedence

On foreground window change, profile detection follows ADR-0006. Each profile
may define multiple `[[matches]]`; entries are ORed together, but every
declared supported field inside one entry must match the foreground state. A
compound Luanti entry with `exe = "luanti.exe"` and a Luanti title regex, for
example, does not match a different `luanti.exe` window title. The resolver
then ranks a matching entry by its strongest matched field. Across profiles,
precedence is:

1. `exe`
2. `title_regex`
3. `steam_appid`
4. `window_class`

If two profiles match at the same rank, the newer profile file mtime wins. Any
remaining exact tie is broken deterministically by source path, profile id, and
loaded index. Agent/operator override is explicit through
`profile_activate(profile_id=...)`.

`process_args` is parsed in the profile schema but is not a runtime foreground
match signal until Synapse has a process-argument source of truth.

### 8.4 Bundled profiles at v1

| Profile | Use |
|---|---|
| `notepad` | Windows Notepad (smoke-test productivity app) |
| `vscode` | Visual Studio Code |
| `chrome` | Google Chrome (CDP-enabled when remote debugging port present) |
| `terminal` | Windows Terminal / PowerShell window |
| `luanti.minetest` | Local Luanti / Minetest Game benchmark world (`operator_owned_test`) |
| `file_explorer` | Windows File Explorer |
| `slack` | Slack desktop |
| `discord` | Discord desktop |
| `minecraft.java` | Minecraft Java Edition (single-player) |
| `factorio` | Factorio (mod-friendly automation profile) |
| `<one FPS>` | TBD — a single-player FPS for the M3 demo (likely a free game) |

Profiles with `use_scope = "unknown"` ship with minimal action defaults until reviewed. Observation can work before a keymap exists, but write/action behavior should be added only when the intended environment is documented.

### 8.5 Profile hot reload

Synapse watches the profile directory via `notify` crate. File changes trigger re-parse and replace in memory. Existing observation streams switch profiles on next event tick. Reflexes don't auto-restart; if a reflex depends on a removed keymap alias, it fails with `REFLEX_PARAMS_INVALID` on next firing.

### 8.6 Versioning

`version` is semver. Loader rejects profiles whose major > Synapse-supported major. Minor is informational. Expected workflow: community-contributed profiles in a separate repo, installed via `synapse-mcp profiles install <repo>/<name>`.

### 8.7 Profile signing (post-v1)

Profiles are TOML data; no scripts in v1. Post-v1: optional profile signing via a community-key model so the operator can choose to load only signed profiles. Tracked in `16_open_questions.md`.

---

## 9. Migrations

Pre-v1: none. DB wipe on schema change is acceptable. Manual FSV wipe-and-rebuild scenarios use a sample data set and then read the rebuilt database state directly.

Post-v1: migrations live in `synapse-storage::migrations` with explicit `from -> to` functions. Idempotent and resumable. Migration failure halts the daemon with `STORAGE_SCHEMA_MISMATCH`; operator runs `synapse-mcp db migrate` manually.

---

## 10. Backups

`synapse-mcp db backup <out>` uses RocksDB's `CheckpointBuilder` for a hot, consistent snapshot. Output is a directory the operator can tar/zip. Restore: `synapse-mcp db restore <in>` stops the daemon (via shared lock file), replaces the DB dir, starts the daemon. Backup size ≈ live DB size; with default retention typically 100–500 MB.

---

## 11. Disk pressure response

If DB directory's free disk drops below 1 GB:

1. Log `STORAGE_DISK_LOW`; emit `system-disk-low` event.
2. Aggressively expire `CF_OCR_CACHE`, `CF_TELEMETRY`, `CF_OBSERVATIONS`.
3. If still below 500 MB, halt writes to `CF_EVENTS` (replay log); surface `STORAGE_WRITE_FAILED`.
4. Below 200 MB: refuse new MCP sessions; existing sessions get a one-line warning.

Agent can poll `health` for disk pressure state.

---

## 12. RocksDB tuning (initial)

```rust
let mut opts = Options::default();
opts.create_if_missing(true);
opts.create_missing_column_families(true);
opts.set_max_background_jobs(2);
opts.set_compression_type(DBCompressionType::Lz4);
opts.set_max_open_files(256);
opts.set_keep_log_file_num(8);
opts.set_write_buffer_size(64 * 1024 * 1024);   // 64 MB memtable
opts.set_max_write_buffer_number(3);
opts.set_target_file_size_base(64 * 1024 * 1024);
opts.set_level_zero_file_num_compaction_trigger(4);
```

Per-CF overrides: `CF_EVENTS`/`CF_ACTION_LOG`/`CF_REFLEX_AUDIT` use `LZ4` + prefix extractor on time prefix for range scans; `CF_MODEL_CACHE` uses `None` compression (already compressed binary) + 256 MB write buffer; `CF_OBSERVATIONS`/`CF_SESSIONS` use `Zstd` (smaller, fewer writes). Tuning in `synapse-storage::tuning`.

---

## 13. What this doc does NOT cover

- Alternate storage backends → future ADR only; M3 ships RocksDB
- Replay tool UI → none at v1; CLI-only output
- Operator config file schema → `14_build_and_packaging.md`
- Per-CF compaction filter implementation → code only
