# 04 — Storage Layer (RocksDB)

Source files covered:
- `crates/synapse-storage/src/lib.rs`
- `crates/synapse-storage/src/cf.rs`
- `crates/synapse-storage/src/codecs.rs`
- `crates/synapse-storage/src/compaction.rs`
- `crates/synapse-storage/src/batch.rs`
- `crates/synapse-storage/src/gc.rs`
- `crates/synapse-storage/src/pressure.rs`
- `crates/synapse-storage/src/error.rs`
- `crates/synapse-core/src/defaults.rs`
- `crates/synapse-core/src/retention.rs`
- `crates/synapse-core/src/types.rs` (Stored* types)

## 1. Connection management

The single `Db` handle (`crates/synapse-storage/src/lib.rs:33`) owns:

- `path: PathBuf` — root directory passed to `Db::open`
- `schema_version: u32` — pinned at construction (= `synapse_core::SCHEMA_VERSION` = `1`)
- `batcher: batch::Batcher` — background actor consuming write batches
- `inner: Arc<rocksdb::DB>` — the RocksDB handle, shared with GC + pressure tasks
- `pressure: Arc<pressure::PressureState>` — current disk-pressure level (atomic `u8`)

`Db::open(path, schema_version)` (`lib.rs:60`):

1. Builds the base `Options` (`db_options()`):
   - `create_if_missing = true`
   - `create_missing_column_families = true`
   - `max_background_jobs = 2`
   - Default `compression_type = Lz4`
   - `max_open_files = 256`
   - `keep_log_file_num = 8`
   - `write_buffer_size = 64 MiB` (`DEFAULT_WRITE_BUFFER_BYTES`)
   - `max_write_buffer_number = 3`
   - `target_file_size_base = 64 MiB`
   - `level_zero_file_num_compaction_trigger = 4`
   - Block-based table factory with a shared `LruCache` of `64 MiB` (`BLOCK_CACHE_BYTES`)
2. Builds per-CF `Options` via `cf_options(name)`:
   - All CFs: same write buffer + L0 compaction settings as the base
   - **Time-keyed CFs** (`CF_EVENTS`, `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`): `compression = Lz4` (default), `SliceTransform::create_fixed_prefix(8)` for prefix-bloom filters
   - **`CF_MODEL_CACHE`**: `compression = None`, larger write buffer (`MODEL_CACHE_WRITE_BUFFER_BYTES = 256 MiB`) so ONNX blobs spill to L0 less often
   - **`CF_OBSERVATIONS`, `CF_SESSIONS`**: `compression = Zstd` (higher ratio for retained snapshots)
   - All CFs receive the `install_ttl_filter` compaction filter (see §6) and the same shared `LruCache`
3. Opens with `DB::open_cf_descriptors`, wrapping any rocksdb failure into `StorageError::OpenFailed` and emitting a `tracing::warn` with `code = STORAGE_OPEN_FAILED`.
4. Verifies the schema sentinel via `verify_schema_version` (§3).
5. Verifies all CF handles are present; missing → `STORAGE_OPEN_FAILED` with a detail string.
6. Wraps the DB in `Arc`, spawns the `Batcher` task, initializes a fresh `PressureState`, and returns `Db`.

There is **no connection pool** — RocksDB is a single embedded instance per process. Concurrent access goes through the shared `Arc<DB>` (the `multi-threaded-cf` feature is enabled in `Cargo.toml`).

## 2. Codecs

`crates/synapse-storage/src/codecs.rs` (39 LoC) defines the only persisted codecs:

| Function | Signature | Behavior | Error code |
|---|---|---|---|
| `encode_json<T: Serialize>` | `(&T) -> StorageResult<Vec<u8>>` | `serde_json::to_vec` | `STORAGE_WRITE_FAILED` via `StorageError::EncodeJson` |
| `decode_json<T: DeserializeOwned>` | `(&[u8]) -> StorageResult<T>` | `serde_json::from_slice` | `STORAGE_READ_FAILED` via `StorageError::DecodeJson` |

A source-code comment pins the constraint: "ADR-0001 / RUSTSEC-2025-0141 prohibit binary persisted codecs here; storage payloads stay JSON so state-readback bytes remain inspectable." Bincode/postcard/etc. are not used.

## 3. Schema versioning

Schema version is a single big-endian `u32` stored under the key `__schema_version` (`SCHEMA_VERSION_KEY = b"__schema_version"`, `crates/synapse-storage/src/lib.rs:30`).

`verify_schema_version` (`lib.rs:399`):

1. Reads `__schema_version`. If absent (fresh DB), writes the expected value and returns `Ok(())`.
2. If present, decodes as big-endian `u32`. Match → `Ok(())`. Mismatch → `StorageError::SchemaMismatch { expected, actual }` → maps to `STORAGE_SCHEMA_MISMATCH`.

`synapse_core::SCHEMA_VERSION = 1` (`crates/synapse-core/src/defaults.rs`). Pre-v1 doctrine (per `docs/computergames/README.md` "Authoring rules" and `docs/impplan/00_methodology.md`): schema changes wipe-and-rebuild; no migration shims are present in the storage crate.

## 4. Column families

Defined in `crates/synapse-storage/src/cf.rs`. `ALL_COLUMN_FAMILIES` (line 25) is the canonical array of 11 CF names, excluding the implicit RocksDB `default` CF.

| # | CF constant | String value | Purpose | Compression | Prefix extractor |
|---|---|---|---|---|---|
| 1 | `CF_EVENTS` | `"CF_EVENTS"` | Replay event log (M3 reflex bus persistence) | Lz4 | fixed-prefix 8 |
| 2 | `CF_OBSERVATIONS` | `"CF_OBSERVATIONS"` | Observation snapshots retained for replay and debugging | Zstd | — |
| 3 | `CF_PROFILES` | `"CF_PROFILES"` | Cached profile loads plus local profile-registry quality snapshots; on-disk TOML remains authored profile source | Lz4 | — |
| 4 | `CF_MODEL_CACHE` | `"CF_MODEL_CACHE"` | Downloaded ONNX model cache | None | — |
| 5 | `CF_SESSIONS` | `"CF_SESSIONS"` | MCP session continuity records | Zstd | — |
| 6 | `CF_REFLEX_AUDIT` | `"CF_REFLEX_AUDIT"` | Per-reflex audit trail (registered/fired/cancelled/expired/disabled) | Lz4 | fixed-prefix 8 |
| 7 | `CF_OCR_CACHE` | `"CF_OCR_CACHE"` | OCR memoization cache for stable regions | Lz4 | — |
| 8 | `CF_TELEMETRY` | `"CF_TELEMETRY"` | Local metric ring buffer | Lz4 | — |
| 9 | `CF_ACTION_LOG` | `"CF_ACTION_LOG"` | Emitted action log | Lz4 | fixed-prefix 8 |
| 10 | `CF_PROCESS_HISTORY` | `"CF_PROCESS_HISTORY"` | Process start/exit history | Lz4 | — |
| 11 | `CF_KV` | `"CF_KV"` | Generic bounded key-value extension | Lz4 | — |

The implicit RocksDB `default` CF is created automatically but holds only the `__schema_version` sentinel.

### 4.1 Schema (current persisted value types)

These are the `serde_json` payloads written into each CF. Source: `crates/synapse-core/src/types.rs`, plus call sites in `synapse-reflex`, `synapse-mcp`, `synapse-profiles`, `synapse-models`.

| CF | Persisted type | Fields | Key shape (current writer) |
|---|---|---|---|
| `CF_EVENTS` | `StoredEvent` | `schema_version: u32`, `event_id: String`, `ts_ns: u64`, `session_id: Option<String>`, `audit_context: Option<StoredAuditContext>`, `source: EventSource`, `kind: String`, `data: serde_json::Value`, `window_id: Option<i64>`, `element_id: Option<ElementId>`, `redacted: bool`, `redactions: Vec<StoredRedaction>` | profile activation/denial events use `[ts_ns u64 BE][seq u32 BE]` |
| `CF_OBSERVATIONS` | `StoredObservation` | `schema_version`, `observation_id`, `ts_ns`, `session_id`, `mode: PerceptionMode`, `foreground: ForegroundContext`, `focused: Option<FocusedElement>`, `elements: Vec<AccessibleNode>`, `entities: Vec<DetectedEntity>`, `hud: HudReadings`, `audio: AudioContext`, `recent_events: Vec<EventSummary>`, `clipboard_summary: Option<ClipboardSummary>`, `fs_recent: Vec<FsEvent>`, `diagnostics: ObservationDiagnostics`, `reason: String`, `redacted: bool`, `redactions: Vec<StoredRedaction>` | — (no live writer in this build; produced by future M3 replay backends. `replay_record` writes JSONL to disk, not to this CF.) |
| `CF_PROFILES` | cached profile rows plus profile quality snapshots | `Profile { id, label, version, use_scope, matches, mode, capture, detection, ocr, hud, keymap, backends, metadata, event_extensions }`; `ProfileQualitySnapshot` at key `profile_quality/v1/<profile_id>` | `profile_quality_refresh` writes redacted quality snapshots; authored profiles read from TOML and held in `synapse-profiles::ProfileRuntime` memory |
| `CF_MODEL_CACHE` | raw bytes + `ModelDescriptor` | binary ONNX blob behind a JSON-encoded descriptor key | not exercised in current build (no model auto-download yet) |
| `CF_SESSIONS` | `StoredSession` | `schema_version`, `session_id`, `started_at`, `ended_at`, `transport`, `client`, `mode`, `active_profile`, `audit_context: Option<StoredAuditContext>`, `profile_history: Vec<StoredProfileHistoryEntry>`, `redacted`, `redactions` | `session/v1/<session_id>` when a profile activation starts/updates the MCP audit session |
| `CF_REFLEX_AUDIT` | `StoredReflexAudit` | `schema_version`, `audit_id`, `reflex_id`, `ts_ns`, `status: ReflexState`, `event_id: Option<String>`, `audit_context: Option<StoredAuditContext>`, `steps: Vec<StoredReflexStep>`, `error_code: Option<String>`, `details: serde_json::Value`, `redacted`, `redactions` | `format!("{reflex_id}:{audit_id}")` (see §4.2) |
| `CF_OCR_CACHE` | not yet wired | — | — |
| `CF_TELEMETRY` | not yet wired | — | — |
| `CF_ACTION_LOG` | action audit JSON | `schema_version`, `audit_id`, `ts_ns`, `seq`, `session_id`, `profile_id`, `profile_version`, `profile_schema_version`, `audit_context`, `tool`, `status`, `error_code`, `foreground`, `active_profile_id`, `active_profile_schema_version`, `details`, `redacted`, `redactions` | action tools via `server/action_audit.rs`; diagnostic probe writes may create malformed rows for manual corrupt-row checks |
| `CF_PROCESS_HISTORY` | not yet wired | — | — |
| `CF_KV` | generic JSON/bytes rows | Audit export consent rows at `audit_export/v1/consent/<profile_id>` and registry head pointers at `profile_registry/v1/head/<source_id>` | `audit_export_consent_set` and registry tools |

`StoredReflexStep` is `{ index: u32, action: Action, status: String, error_code: Option<String> }`.
`StoredAuditContext` is the shared profile/audit linkage payload:
`{ session_id, profile_id, profile_version, profile_schema_version,
backend_policy, app_context }`. `StoredBackendPolicy` captures default,
keyboard, mouse, and pad backend resolution from the active profile.
`StoredAppContext` captures foreground process/window plus profile metadata
such as benchmark target id, gameid, world path/name, and log path when
available.
`StoredRedaction` is `{ kind: String, offset: u32, len: u32 }`.

### 4.2 Active write paths (current build)

The live build actively writes `CF_REFLEX_AUDIT`, `CF_ACTION_LOG`,
`CF_EVENTS`, `CF_SESSIONS`, profile-quality rows in `CF_PROFILES`, registry
head pointers in `CF_KV`, and audit-export consent rows in `CF_KV`:

| Caller | Trigger | Audit payload | Key format |
|---|---|---|---|
| `SynapseService::profile_activate` | tool `profile_activate` succeeds | `StoredSession` with active profile, profile history, backend policy, app/game context, redaction flags | `session/v1/<session_id>` in `CF_SESSIONS` |
| `SynapseService::profile_activate` | tool `profile_activate` succeeds or fails after dispatch | `StoredEvent` kind `profile.activated` or `profile.activation_denied`, linked to the current audit context when available | `[ts_ns u64 BE][seq u32 BE]` in `CF_EVENTS` |
| `ReflexRuntime::register` (`crates/synapse-reflex/src/lib.rs:146`) | tool `reflex_register` | `details.kind = "reflex_registered"`, `status = Active`, `error_code = None` | `"<reflex_id>:<audit_id>"` (v7 UUID for audit_id) |
| `ReflexRuntime::cancel` (`lib.rs:198`) | tool `reflex_cancel` | `details.kind = "reflex_cancelled"`, `status = Cancelled` | same |
| `ReflexRuntime::disable_all_by_operator` (`lib.rs:245`) | operator panic hotkey (`crates/synapse-mcp/src/safety.rs::handle_operator_hotkey`) | `details.kind = "reflex_disabled_by_operator"`, `status = Disabled`, `error_code = REFLEX_DISABLED_BY_OPERATOR` | same |
| `ReflexScheduler` fire path (in `crates/synapse-reflex/src/scheduler.rs` + `kinds/on_event.rs`) | each reflex fire | `details.kind = "reflex_fired"`, `status = Active`, optional `event_id` and per-step `steps` | same |
| recursion-guard clamp (`kinds/on_event.rs`) | exceeded `MAX_ON_EVENT_FIRINGS_PER_TICK` | `error_code = REFLEX_RECURSION_LIMIT` | same |
| `SynapseService::audit_export_consent_set` | operator enables/disables export consent | JSON row `row_kind = "audit_export_consent"` with profile id, enabled state, redaction policy, allowed policies, and `external_sharing_allowed = false` | `audit_export/v1/consent/<profile_id>` in `CF_KV` |

Reflex writers go through `synapse_reflex::audit::write_audit`
(`crates/synapse-reflex/src/audit.rs`), which is just a thin wrapper around
`Db::put_batch(CF_REFLEX_AUDIT, ...)` followed (by the caller) by `Db::flush()`.
Action tools write profile-linked action-audit rows to `CF_ACTION_LOG` through
`crates/synapse-mcp/src/server/action_audit.rs`; the key is
`[ts_ns u64 BE][seq u32 BE]`, and each write is flushed before the action
result audit returns. Rows carry the MCP audit `session_id`, active
profile id/version/schema, backend policy, app/game context, result/error
codes, and redaction/export flags where available. `profile_quality_refresh`
reads those action rows,
aggregates profile-relevant outcomes, writes a redacted
`ProfileQualitySnapshot` JSON row to `CF_PROFILES` at
`profile_quality/v1/<profile_id>`, and reads that exact row back before
returning. The score uses the Wilson 95% lower bound over foreground-profile
`ok` vs `error` rows; denied, stale, corrupt, and profile-mismatch rows are
explainability/compatibility counters, not invented success samples.

`audit_export_bundle` does not write RocksDB rows. It reads the consent row from
`CF_KV`, reads matching `CF_ACTION_LOG` rows for the requested profile, redacts
sensitive fields, and writes local bundle files (`manifest.json`, `rows.json`,
`redaction_report.json`) under the caller-selected output directory.

## 5. Index strategy

RocksDB indexes by key. Prefix-bloom filters are configured (`SliceTransform::create_fixed_prefix(8)`) on the three time-keyed CFs (`CF_EVENTS`, `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`). For audit lookups by reflex id, callers use `Db::scan_cf_prefix(CF_REFLEX_AUDIT, b"<reflex_id>:")` (`crates/synapse-storage/src/lib.rs:302`), which seeks to the prefix and breaks once the iterator leaves the prefix.

There are no secondary indexes maintained by application code. `ReflexRuntime::history` (`crates/synapse-reflex/src/lib.rs:311`) scans either by `reflex_id` prefix or globally, then sorts the deserialized `Vec<StoredReflexAudit>` by `(ts_ns desc, audit_id desc, reflex_id desc)` before applying `limit`.

## 6. TTL compaction filter

`crates/synapse-storage/src/compaction.rs` installs a per-CF compaction filter using each CF's `RetentionTtl` from `synapse_core::retention::DEFAULTS` (`crates/synapse-core/src/retention.rs`):

| CF | TTL | Soft cap (MB) | Hard cap (MB) |
|---|---|---:|---:|
| `CF_EVENTS` | 24 hours | 2048 | 4096 |
| `CF_OBSERVATIONS` | 6 hours | 500 | 1000 |
| `CF_PROFILES` | none | 20 | 50 |
| `CF_MODEL_CACHE` | LRU-only (no TTL) | 1024 | 2048 |
| `CF_SESSIONS` | 30 days | 50 | 100 |
| `CF_REFLEX_AUDIT` | 7 days | 200 | 500 |
| `CF_OCR_CACHE` | 1 hour | 50 | 100 |
| `CF_TELEMETRY` | 6 hours | 100 | 200 |
| `CF_ACTION_LOG` | 24 hours | 200 | 500 |
| `CF_PROCESS_HISTORY` | 6 hours | 20 | 50 |
| `CF_KV` | none | 10 | 50 |

Filter behavior (`ttl_decision`): the compaction filter parses the JSON value (byte-scanning for the literal `"ts_ns"` field, no full JSON parse), reads its `u64`, and compares `now_ns - ts_ns > ttl_ns`. If `ts_ns` cannot be extracted or the value is fresh enough, the row is kept; otherwise removed.

For CFs without a `ts_ns` field (e.g. `CF_PROFILES`, `CF_KV`) the filter is still attached but every row falls into the "kept" branch.

## 7. Garbage collection (soft/hard caps)

`crates/synapse-storage/src/gc.rs`:

- `GC_INTERVAL = 5 minutes` (`Duration::from_mins(5)`).
- `Db::spawn_gc_task` spawns a tokio task that runs `run_once` at every tick.
- `GcConfig::from_retention_defaults` builds a `GcBudget` per CF in **bytes** (each `soft_cap_mb`/`hard_cap_mb` × 1 MiB), with `unit = CapUnit::Bytes`. A test-only `for_row_caps` variant uses `CapUnit::Rows`.
- `run_cf` (`gc.rs:159`) per-CF algorithm:
  1. `collect_keys` walks the CF (`IteratorMode::Start`) to compute current key list and total measured size.
  2. Records `ESTIMATE_NUM_KEYS` and (for byte units) iterates keys to compute total bytes; for row units, uses key count.
  3. If `before_value >= hard_cap`, emits `tracing::warn` with `code = STORAGE_CF_HARD_CAP_REACHED`.
  4. If `before_value > soft_cap`, calls `evict_oldest`: sorts keys lex-asc and `DB::delete_cf` from the oldest until `before_value - sum_of_evicted_bytes <= soft_cap`.
  5. After eviction, re-collects keys for `after_value`, and increments the Prometheus counter `cache_evictions_total{cf=<name>, reason="soft_cap"}` by the evicted row count.
- Returns `GcReport { cf_reports: Vec<GcCfReport> }` (one entry per budget) so callers can readback before/after sizes.

### 7.1 Audit retention runtime path

#463 adds the profile-linked audit retention path on top of the existing
`storage_gc_once` MCP tool rather than adding another live tool. When the
runtime receives `storage_gc_once` with `cf_name = "AUDIT_RETENTION"`, it:

1. Reads `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`, `CF_EVENTS`, `CF_OBSERVATIONS`,
   `CF_SESSIONS`, plus strategic prefixes in `CF_PROFILES` and `CF_KV`.
2. Preserves malformed or unknown-schema rows; they are counted in the report
   and are not deleted.
3. Backfills known schema-v1 rows with top-level `profile_id` and
   `profile_schema_version` when those values already exist in
   `audit_context`, foreground profile state, active profile fields, or session
   state.
4. Dedupes repeated action/reflex/event/observation/session outcomes using
   bounded class-specific keys such as profile id, tool/reflex/kind, status,
   error code, foreground process, and backend.
5. Applies the requested row cap to non-strategic rows and then writes a
   durable report to `CF_KV/audit_retention/v1/report/<run_id>`.

Retention backfills and retention report rows use the storage-maintenance write
path (`Db::put_batch_pressure_bypass`) so Level3/Level4 disk pressure cannot
silently drop the migration/report evidence. Normal ingestion still goes
through `Db::put_batch` and remains pressure-gated.

`storage_inspect` exposes the static `audit_retention_policies` list before
the trigger. Manual FSV then reads the row counts/samples, triggers
`storage_gc_once` in `AUDIT_RETENTION` mode, and separately reads
`storage_inspect` plus the persisted `CF_KV` report row after the trigger.
Strategic profile-quality snapshots and audit-export consent rows are policy
visible and preserved, including under disk pressure.

## 8. Disk pressure monitor

`crates/synapse-storage/src/pressure.rs`:

- `POLL_INTERVAL = 30 s`.
- Pressure thresholds (DB volume free bytes):

| Level | Free-bytes threshold (≤) | Error code |
|---|---|---|
| `Normal` (0) | — | none |
| `Level1` | `2 GB` | `STORAGE_DISK_PRESSURE_LEVEL_1` |
| `Level2` | `1 GB` | `STORAGE_DISK_PRESSURE_LEVEL_2` |
| `Level3` | `500 MB` | `STORAGE_DISK_PRESSURE_LEVEL_3` |
| `Level4` | `200 MB` | `STORAGE_DISK_PRESSURE_LEVEL_4` |

(`GB`/`MB` use decimal: `1_000_000_000` and `1_000_000`.)

- `PressureState` holds the current level as `AtomicU8`; `Db::pressure_level()` reads it.
- `Db::put_batch` consults `pressure.permits_write(cf_name)` before submitting the batch. At higher levels the responder freezes specific CFs; writes are then silently dropped after a `tracing::warn` with `code = STORAGE_WRITE_FAILED`. Bounded storage-maintenance rewrites use `Db::put_batch_pressure_bypass` instead, and the audit retention path reserves that bypass for backfilled rows and report rows.
- The poller may also trigger compaction on selected CFs at higher levels.
- Test-only entrypoint `Db::run_pressure_check_with_free_bytes_sample(free_bytes)` lets the daemon apply a synthetic sample at startup via `--storage-pressure-free-bytes-sample`. (See [03_configuration.md](03_configuration.md).)

## 9. Write path / batching

`crates/synapse-storage/src/batch.rs` implements a `Batcher` actor wrapping `WriteBatchWithIndex`:

1. `Db::put_batch(cf_name, kvs)` validates the CF handle, checks `pressure.permits_write`, materializes the KV pairs as `Vec<(Vec<u8>, Vec<u8>)>`, and forwards to the batcher.
2. The batcher aggregates writes and flushes either on an explicit `Db::flush()` call or when the next batch arrives.
3. `Db::flush()` issues a synchronous flush (`WriteOptions::sync`-style). The current reflex audit pattern is `write_audit(&db, &audit)` followed by `db.flush()`, so each persisted audit is durable before the tool response returns.
4. Empty key sets are no-ops (`if kvs.is_empty() { return Ok(()) }`).

There are no transactions. Reflex audit consistency is achieved by the
single-writer model for `CF_REFLEX_AUDIT`: `ReflexRuntime` writes reflex audit
rows while holding its own `Mutex`. Profile activation/session/event and
action-audit consistency is a local MCP-service contract: the tool writes and
flushes each physical row before returning the corresponding activation or
action result.

## 10. Query helpers

| Function | Source | Behavior |
|---|---|---|
| `Db::cf_sizes()` | `lib.rs:209` | Scans every CF in `ALL_COLUMN_FAMILIES`, sums `key.len + value.len`, returns `BTreeMap<String, u64>`. Used by `synapse_mcp::server::SynapseService::storage_health` to populate `health.subsystems.storage.cf_sizes`. |
| `Db::scan_cf(cf_name)` | `lib.rs:282` | Iterates the CF from the start and returns owned `(key, value)` byte pairs. |
| `Db::scan_cf_prefix(cf_name, prefix)` | `lib.rs:302` | Iterates from `IteratorMode::From(prefix, Direction::Forward)` and breaks once iterator leaves the prefix. |
| `Db::compact_cf(cf_name)` | `lib.rs:331` | Triggers `compact_range_cf(None, None)` over the entire CF; used by the pressure responder. |
| `Db::pressure_level()` | `lib.rs:196` | Returns the cached `DiskPressureLevel`. |
| `Db::run_gc_once()` | `lib.rs:150` | Synchronous one-shot GC pass using retention defaults. |
| `Db::run_pressure_check_once()` | `lib.rs:229` | Synchronous one-shot disk-pressure poll. |

There is no higher-level query API (no SQL, no secondary indexes). All RocksDB operations go through this surface.

## 11. Replay JSONL (alternative persistence)

`replay_record` (`crates/synapse-mcp/src/m3/replay.rs`) writes observation/event records to a **flat JSONL file** under `%LOCALAPPDATA%/synapse/replays`, **not** into the RocksDB CFs. Cadence: observations sampled every 250 ms, events drained every 20 ms, both written via `tokio::io::BufWriter<File>` until the requested `duration_ms` elapses. Paths outside `replay_root()` are rejected with `SAFETY_PERMISSION_DENIED`.

## 12. Virtual tables, extensions, special features

- **None.** Synapse uses stock RocksDB compaction filters and slice transforms; no merge operators, no transactions, no secondary indexes, no virtual CFs. The schema sentinel lives in the default CF; the operator-visible CFs are exactly the 11 listed.
- **Compression**: LZ4 default, ZSTD on `CF_OBSERVATIONS`/`CF_SESSIONS`, none on `CF_MODEL_CACHE`.
- **Block cache**: shared 64 MiB LRU across all CFs via `BlockBasedOptions::set_block_cache`.

## 13. What is NOT covered

- **Migrations.** There is no migration framework; bumping `SCHEMA_VERSION` requires deleting the database directory.
- **Cross-process locking.** A single `synapse-mcp` process owns the directory; running two daemons against the same `--db` path will fail on `Db::open` because RocksDB's exclusive lock kicks in.
- **Backups.** The daemon does not export, snapshot, or back up its own DB; any backup strategy is operator-side (e.g. file-system snapshot while the process is stopped).
- **Encryption-at-rest.** RocksDB is not configured with encryption.
- **`CF_OBSERVATIONS` writer.** The persistence pipeline for observations is
  wired in retention defaults and tested, but no production code path
  currently writes observation snapshots through it. `CF_EVENTS` is now used by
  profile activation/denial audit events.
