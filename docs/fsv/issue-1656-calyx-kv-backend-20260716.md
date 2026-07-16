# Issue #1656 Calyx KV storage backend manual FSV - 2026-07-16

## Scope

Issue #1656 needed `StorageBackendKind::Calyx` to stop failing closed as
unimplemented and instead expose the normal Synapse `Db` storage surface over a
real Calyx AsterVault:

- byte-preserving put/get/delete/mutate/batch behavior for every Synapse CF;
- exact key-ordered scans, prefix scans, and bounded windows;
- schema-version validation in the Calyx store;
- explicit unsupported errors for RocksDB-specific maintenance until Calyx
  pressure/GC/compaction parity lands in #1658/#1659;
- health and storage summaries that truthfully report Calyx maintenance as
  unsupported instead of masking it as healthy RocksDB maintenance.

No tests, benchmarks, scripts, GitHub Actions, or CI were used as FSV.
Structural checks are listed separately and are not FSV.

## Root Cause

The root cause was a backend seam without a Calyx implementation. Synapse had a
generic storage abstraction and `StorageBackendKind::Calyx`, but the Calyx path
returned an unimplemented backend instead of translating Synapse CF rows into
AsterVault rows. The real integration constraints were lower-level than the
enum:

- Synapse expects arbitrary byte keys, including empty keys, and byte-identical
  values per CF.
- Aster's public KV helper is not byte-identical for Synapse because its user KV
  layer rejects empty keys, applies different payload limits, and prefixes
  encoded keys with key length before the user key. That means physical scan
  order has to be restored by decoding and sorting by the original Synapse user
  key.
- RocksDB maintenance APIs do not have Calyx parity yet. Treating them as
  successful would hide missing behavior, so they now fail explicitly or are
  reported as intentionally unsupported in health.

## Research

Research was done after the root problem was identified.

Exa MCP and native web research used primary sources:

- RocksDB Basic Operations:
  https://github.com/facebook/rocksdb/wiki/Basic-Operations
- RocksDB FAQ:
  https://github.com/facebook/rocksdb/wiki/RocksDB-FAQ
- Tokio `spawn_blocking`:
  https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html
- Tokio bounded `mpsc`:
  https://docs.rs/tokio/latest/tokio/sync/mpsc/index.html

Implementation takeaways:

- Use a real atomic write batch for multi-row/multi-CF commits, matching
  RocksDB's WriteBatch model.
- Use MVCC snapshots for scans and then sort decoded Synapse user keys to
  preserve RocksDB byte-lexicographic scan semantics over Calyx's envelope.
- Keep the sync vault as the source of durable truth and fail closed on decode,
  unsupported TTL, oversized keys, unsupported CFs, or closed vault state.
- Do not invent fallback maintenance behavior. Report Calyx maintenance as
  unsupported until the parity issues are implemented.

## Implementation Summary

- `crates/synapse-storage/src/backend.rs`
  - added `CalyxBackend` implementing `StorageBackend`;
  - maps every Synapse CF to a fixed Calyx collection id inside
    `ColumnFamily::Kv`;
  - stores rows as `0x03 | collection_id | namespace | key_len | user_key`;
  - stores values as `0x01 | expires_at_ms=0 | exact_synapse_value`;
  - treats nonzero TTL or malformed encoded bytes as storage corruption;
  - implements put, get, delete, mutate, multi-CF batch, flush, row counts,
    sizes, exact scans, prefix scans, and bounded windows;
  - returns explicit `STORAGE_BACKEND_UNIMPLEMENTED` for Calyx maintenance APIs
    that are not parity-complete yet.
- `crates/synapse-storage/src/lib.rs`
  - opens `CalyxBackend::open(path, schema_version)` when the requested backend
    is `calyx`.
- `crates/synapse-calyx/src/lib.rs`
  - exposes the raw CF read/write/scan/flush/pin operations required by the
    storage backend.
- `crates/synapse-mcp/src/m3.rs`
  - skips RocksDB pressure/GC maintenance tasks for Calyx and records
    `STORAGE_CALYX_MAINTENANCE_UNSUPPORTED`.
- `crates/synapse-core/src/types/health.rs` and
  `crates/synapse-mcp/src/server/health.rs`
  - report `storage_maintenance_supported=false` and
    `storage.status=maintenance_unsupported` for Calyx without making the whole
    daemon unhealthy.
- `crates/synapse-mcp/src/m3/storage.rs`
  - labels Calyx summary metrics as exact scans rather than RocksDB estimates.

## Source of Truth

Primary SoT:

`C:\Users\hotra\AppData\Local\synapse\fsv\issue-1656-calyx-20260715-2130b\db`

Physical readbacks:

- daemon process and TCP listener;
- bearer token file and current env token match;
- real wired `mcp__synapse.health` and public tool list;
- real MCP `mcp__synapse.workspace` and `mcp__synapse.storage` calls;
- Calyx files under `cf\kv`, `wal`, `MANIFEST`, `CURRENT`,
  `vault-identity.json`, and `vault.pid`;
- raw UTF-8 byte searches in readable Calyx `cf\kv` SST files for known row
  keys and known value markers.

Synthetic run id:

`issue1656-calyx-fsv-20260716`

## MCP Precondition

Before accepting behavior, the real repo-built daemon was running:

- Process SoT: PID `73920`, process name `synapse-mcp.exe`, path
  `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`.
- Command line:
  `"C:\Users\hotra\.cargo\bin\synapse-mcp.exe" --mode http --bind 127.0.0.1:7700 --db C:\Users\hotra\AppData\Local\synapse\fsv\issue-1656-calyx-20260715-2130b\db --storage-backend calyx --profile-dir C:\Users\hotra\.cargo\bin\profiles --log-level info`
- Socket SoT: `127.0.0.1:7700` listener owned by PID `73920`.
- Token SoT: `C:\Users\hotra\AppData\Roaming\synapse\token.txt` length `32`;
  current `SYNAPSE_BEARER_TOKEN` length `32`; values matched.
- Wired client proof: `mcp__synapse.health({"detail":"compact"})` returned
  `ok=true`, `pid=73920`, `tool_count=40`, `tool_surface_sha256=7cc1d191749041ce3f0bb1a646d1f2b8553e6155e8a9f917553fd66c4cc257ad`.
- Health storage readback:
  - `storage_backend=calyx`;
  - `storage.status=maintenance_unsupported`;
  - `storage_maintenance_supported=false`;
  - `storage_maintenance_unsupported_reason="storage backend calyx has the #1656 byte-preserving Db surface; RocksDB-style pressure/GC maintenance is unavailable until #1658/#1659"`.
- Startup log readback:
  - `SYNAPSE_CALYX_VAULT_OPENED`, vault id
    `01KXMC1G7EG1TAW20XX20T11NK`, `latest_seq=0`;
  - `STORAGE_BACKEND_OPENED backend="calyx" schema_version=1`;
  - `STORAGE_CALYX_MAINTENANCE_UNSUPPORTED`.

## Initial State

Before the synthetic row triggers:

- `mcp__synapse.storage(operation="summary")` returned
  `storage_backend=calyx`, `metrics_mode=calyx_exact_scan_sizes_counts`.
- Initial `CF_KV` count before the happy-path write: `2`.
- Physical vault readback showed:
  - DB path as above;
  - Calyx `cf\kv` SST files already present from daemon startup;
  - `wal\00000000000000000000.wal` present;
  - `vault-identity.json` present.

## Happy Path

Trigger:

`mcp__synapse.workspace(operation="put")`

Input:

- run id: `issue1656-calyx-fsv-20260716`
- key: `issue1656/happy`
- value: `{"expected":"calyx-happy-value-1656","n":4,"formula":"2+2=4"}`

Expected:

- one new `CF_KV` row;
- readback value exactly equals the JSON value above;
- physical Calyx SST bytes contain the row key and value markers.

After readback:

- `workspace.put` reported row key
  `workspace-blackboard/v1/run_hex/6973737565313635362d63616c79782d6673762d3230323630373136/key_hex/6973737565313635362f6861707079`,
  `value_len_bytes=488`,
  `value_sha256=sha256:2d9ca6a300765139f24813ff336e2794137c5dd893c155bcb5a62415a853e5d8`.
- Separate `workspace.get` returned `found=true` and the exact value:
  `{"expected":"calyx-happy-value-1656","formula":"2+2=4","n":4}`.
- Separate `storage.summary` readback changed `CF_KV` from `2` to `3`.
- Separate physical byte scan found `issue1656/happy`,
  `calyx-happy-value-1656`, `2+2=4`, and the exact row key in Calyx
  `cf\kv` SST files, including:
  - `cf\kv\00000000000000000032-0000.sst`
  - `cf\kv\00000000000000000033-0000.sst`
  - `cf\kv\00000000000000000034-0000.sst`
  - their matching `flush-*` SST files.

## Edge 1 - Empty Value

Before:

- `CF_KV=3`.

Trigger:

`mcp__synapse.workspace(operation="put")`

Input:

- key: `issue1656/edge/empty-value`
- value: empty JSON string `""`

Expected:

- one new `CF_KV` row;
- readback value is exactly the empty string;
- physical row key exists in Calyx SST bytes.

After readback:

- `workspace.put` returned `value_len_bytes=462`,
  `value_sha256=sha256:4bd4e7a587c74bec8b5b0d4bef00baeefe8c0a910e36d7449519d9a5f70ac387`.
- Separate `workspace.get` returned `found=true`, `value=""`, `version=1`.
- Separate `storage.summary` changed `CF_KV` from `3` to `4`.
- Separate physical byte scan found `issue1656/edge/empty-value` and its exact
  row key in Calyx `cf\kv` SST files:
  - `00000000000000000047-0000.sst`
  - `00000000000000000048-0000.sst`
  - `00000000000000000049-0000.sst`
  - matching `flush-*` SST files.

## Edge 2 - Bounded Larger Payload

Before:

- `CF_KV=4`.

Trigger:

`mcp__synapse.workspace(operation="put")`

Input:

- key: `issue1656/edge/boundary-long-value`
- value: object with markers `calyx-boundary-start-1656`,
  `calyx-boundary-end-1656`, and a 32-repeat synthetic payload string.

Expected:

- one new `CF_KV` row;
- returned value sha remains stable across put/get;
- physical SST bytes contain both boundary markers.

After readback:

- `workspace.put` returned `value_len_bytes=1206`,
  `value_sha256=sha256:473a781544092b9d45b2b5f40fd440dc2dd368e92c88c6d25b46aee4173ca33c`.
- Separate `workspace.get` returned `found=true`, the same sha, and the value
  containing `marker_start`, `marker_end`, and `repeat_count=32`.
- Separate `storage.summary` changed `CF_KV` from `4` to `5`.
- Separate physical byte scan found:
  - `issue1656/edge/boundary-long-value`;
  - `calyx-boundary-start-1656`;
  - `calyx-boundary-end-1656`;
  - the exact row key.
- Matches appeared in Calyx `cf\kv` SST files:
  - `00000000000000000080-0000.sst`
  - `00000000000000000081-0000.sst`
  - `00000000000000000082-0000.sst`
  - matching `flush-*` SST files.

## Edge 3 - Expected Absent Key

Before:

- `CF_KV=5`.

Trigger:

`mcp__synapse.workspace(operation="get", absent_ok=true)`

Input:

- key: `issue1656/edge/absent`

Expected:

- no row is created;
- exact row readback reports no match;
- `CF_KV` count stays unchanged;
- physical `cf\kv` files do not contain the absent row key.

After readback:

- `workspace.get` returned
  `exact_match_count=0`, `exists=false`, `found=false`.
- Separate `storage.summary` kept `CF_KV=5`.
- Separate physical scan of `db\cf\kv` returned `MatchCount=0` for both
  `issue1656/edge/absent` and its exact workspace row key.

## Edge 4 - Invalid Facade Payload

Before:

- `CF_KV=5`.

Trigger:

`mcp__synapse.workspace(operation="put")` with only a mismatched `get={...}`
payload.

Expected:

- fail closed before delegated storage mutation;
- typed error explains the bad route;
- no row appears in `CF_KV`.

After readback:

- MCP error:
  `TOOL_PARAMS_INVALID`, message
  `workspace operation=put requires a matching put spec`, remediation
  `pass put={...} and no other operation spec`.
- Separate `storage.summary` kept `CF_KV=5`.
- Separate physical scan of `db\cf\kv` returned `MatchCount=0` for
  `issue1656/edge/invalid-mismatch` and its exact workspace row key.

## Edge 5 - Mutating Maintenance Denied Outside Maintenance Profile

Before:

- `CF_KV=5`.
- Health reported Calyx storage maintenance unsupported.

Trigger:

`mcp__synapse.storage(operation="gc_once", cf_name="CF_KV", soft_cap_rows=1, hard_cap_rows=1)`

Expected:

- fail closed before mutation under the normal agent profile;
- no CF_KV row change;
- error carries the physical SoT and remediation.

After readback:

- MCP error:
  `TOOL_PROFILE_POLICY_DENIED`, message
  `storage operation=gc_once is not allowed for profile normal_agent`.
- Error data included:
  `source_of_truth="storage backend CF metadata + exact row readbacks"` and
  remediation to switch to an explicit maintenance profile with operator intent.
- Separate `storage.summary` kept `CF_KV=5`.
- Daemon log recorded the structured error at lines 438-439.

The backend-specific maintenance state was also separately verified through
health and daemon logs:

- health: `storage.status=maintenance_unsupported`;
- health: `storage_maintenance_supported=false`;
- log: `STORAGE_CALYX_MAINTENANCE_UNSUPPORTED` with the #1658/#1659 reason.

## Final After-State

Final `workspace.list` for run id `issue1656-calyx-fsv-20260716` and prefix
`issue1656/` returned exactly the three expected stored rows:

1. `issue1656/edge/boundary-long-value`
   - `value_len_bytes=1206`
   - `value_sha256=sha256:473a781544092b9d45b2b5f40fd440dc2dd368e92c88c6d25b46aee4173ca33c`
2. `issue1656/edge/empty-value`
   - `value_len_bytes=462`
   - `value_sha256=sha256:4bd4e7a587c74bec8b5b0d4bef00baeefe8c0a910e36d7449519d9a5f70ac387`
3. `issue1656/happy`
   - `value_len_bytes=488`
   - `value_sha256=sha256:2d9ca6a300765139f24813ff336e2794137c5dd893c155bcb5a62415a853e5d8`

Final physical vault snapshot:

- `vault.pid` contained PID `73920`, executable
  `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`, `schema_version=1`.
- `vault-identity.json` contained vault id `01KXMC1G7EG1TAW20XX20T11NK`.
- `CURRENT` pointed at `manifest-00000000000000000103.json`.
- readable file count: `529`.
- readable total bytes: `1156520`.
- `cf\kv` file count: `206`.
- WAL length: `300239`.
- physical marker counts in `cf\kv` files:
  - `calyx-happy-value-1656`: `6` matching files;
  - `issue1656/edge/empty-value`: `6` matching files;
  - `calyx-boundary-start-1656`: `6` matching files;
  - `calyx-boundary-end-1656`: `6` matching files.

## Structural Checks

Supporting structural checks only; these are not FSV:

- `cargo fmt --all`
- `cargo fmt --all --check`
- `cargo check -p synapse-storage`
- `cargo check -p synapse-mcp`
- `cargo clippy --workspace --all-targets`
- `cargo build --release -p synapse-mcp` only to ship/run the repo-built
  daemon used for MCP FSV

No `cargo test`, benchmarks, GitHub Actions, CI, test harness, or FSV script was
run or created.

## Result

#1656 is FSV-accepted for the byte-preserving Calyx implementation of the
Synapse `Db` storage API over AsterVault. The real repo-built daemon opened the
Calyx backend, the real wired MCP client triggered storage writes through the
public workspace facade, and separate readbacks proved the expected rows and
markers existed in the physical Calyx vault files. Unsupported maintenance
remains explicit and observable instead of hidden by a fallback.
