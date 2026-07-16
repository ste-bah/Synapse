# Manual FSV: Issue #1660 Storage Inspect/Dump Calyx Parity

Date: 2026-07-16

## Root Cause

The storage inspection and dump paths were still shaped around direct RocksDB
column-family iteration. Calyx stores Synapse logical CF rows inside the Aster
`ColumnFamily::Kv` physical vault with Synapse collection ids, namespaces, and
versioned value envelopes. There was no read-only Calyx vault open path, no
schema-sentinel verification for read-only inspection, no logical CF scan over
the physical vault rows, and no vault-level collection inspector. The dump
examples also emitted raw-ish RocksDB-specific material instead of a shared
metadata-only redacted shape.

## Research

Web research was performed after identifying the root cause.

- OWASP Logging Cheat Sheet:
  `https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html`
  reinforced keeping sensitive payloads out of diagnostic output while retaining
  correlation metadata.
- OWASP ASVS V16:
  `https://asvs.dev/v5.0.0/V16-Security-Logging-and-Error-Handling/`
  reinforced fail-closed error reporting and useful security logging.
- RocksDB read-only/secondary instances:
  `https://github.com/facebook/rocksdb/wiki/Read-only-and-Secondary-instances`
  confirmed read-only DB opens are the correct inspection primitive.
- RocksDB checkpoints:
  `https://github.com/facebook/rocksdb/wiki/Checkpoints`
  confirmed physical snapshot/readback patterns for durable state validation.

Implementation choices from that research:

- Read-only inspection opens existing storage only and fails if the vault or
  schema sentinel is missing.
- Dump and inspect rows expose lengths, SHA-256 hashes, encoding, and omission
  flags only.
- Calyx malformed keys/envelopes fail the inspector instead of being skipped or
  silently rewritten.
- Backend-specific failures keep the storage path, CF name, and operation in
  the error details.

## Source of Truth

- Calyx FSV storage SoT:
  `C:\Users\hotra\AppData\Local\synapse\fsv\issue-1660-calyx-20260716-130949\db`.
- Calyx physical vault SoT: Aster vault `ColumnFamily::Kv`, read by
  `dump_cf --backend calyx` through `SynapseCalyxReadOnlyVault`.
- RocksDB parity SoT:
  `C:\Users\hotra\AppData\Local\synapse\fsv\issue-1660-rocksdb-20260716-134500\db`.
- RocksDB physical CF SoT: `CF_KV`, read by `DB::open_cf_for_read_only`.
- Logical workspace SoT: `CF_KV` rows under run ids
  `issue1660-calyx-fsv-20260716` and
  `issue1660-rocksdb-parity-20260716`.
- MCP daemon/socket SoT: process table plus `127.0.0.1:7700` listener.

## MCP Preconditions

### Calyx FSV Daemon

- Process: `synapse-mcp.exe` PID `47644`.
- Executable: `C:\code\Synapse\target\release\synapse-mcp.exe`.
- Bind: `127.0.0.1:7700`.
- Command included:
  `--storage-backend calyx --db C:\Users\hotra\AppData\Local\synapse\fsv\issue-1660-calyx-20260716-130949\db`.
- `mcp__synapse.health`: `ok=true`, `tool_count=40`,
  `tool_surface_sha256=d4ef68fb707f9ff6e2fd6ef452bc88df0f0d2b0713f692fb5b462369b470dc7c`,
  storage backend `calyx`.
- The wired Codex MCP client loaded and called the public `health`,
  `workspace`, and `storage` tools.

### RocksDB Parity Daemon

- Process: `synapse-mcp.exe` PID `70372`.
- Executable: `C:\code\Synapse\target\release\synapse-mcp.exe`.
- Bind: `127.0.0.1:7700`.
- Command included:
  `--storage-backend rocksdb --db C:\Users\hotra\AppData\Local\synapse\fsv\issue-1660-rocksdb-20260716-134500\db`.
- `mcp__synapse.health`: `ok=true`, `tool_count=40`,
  `tool_surface_sha256=d4ef68fb707f9ff6e2fd6ef452bc88df0f0d2b0713f692fb5b462369b470dc7c`,
  storage backend `rocksdb`.

## Manual State Readbacks

### Initial Calyx State

- Trigger/read: `mcp__synapse.workspace operation=list`, run
  `issue1660-calyx-fsv-20260716`, prefix `issue1660/`.
- Before: `returned_count=0`, `scanned_rows=0`.
- `mcp__synapse.storage operation=inspect` returned a Calyx vault report:
  schema `1`, vault id `01KXP1XQF68E28HT3RZ5P2YESA`, `latest_seq=6`,
  `collection_count=5`, `raw_row_count=6`, `live_row_count=6`,
  `expired_row_count=0`.
- `CF_KV/ns/0` before: `raw_row_count=1`, `live_row_count=1`,
  histogram `durable=1`.

### Happy Path: JSON Value Is Stored, Inspected, And Redacted

- Before `CF_KV`: `1` row.
- Trigger: real MCP `workspace.put`, key `issue1660/happy-secret`, value:
  `{"formula":"2+2=4","expected":4,"marker":"ISSUE1660_SECRET_TOKEN_HAPPY_SHOULD_NOT_APPEAR_IN_INSPECT"}`.
- Put readback: version `1`, value bytes `586`,
  `value_sha256=sha256:6fb26881dd1cc8ea32c8405a3b6ea4927a490481f429726bf7060c6bb0220bb5`.
- Separate read: real MCP `workspace.get` returned the exact JSON value.
- After `storage.inspect`: `CF_KV=2`; Calyx `CF_KV/ns/0` had
  `raw_row_count=2`, `live_row_count=2`, `payload_bytes=813`,
  `total_logical_bytes=1005`, histogram `durable=2`.
- After physical Calyx read-only dump: row key hash
  `sha256:ff8739ec6fbc307d809e8e290ea1e66f4588c1b1d70f4a28e9f0c16cfad50350`,
  value hash `sha256:6fb26881dd1cc8ea32c8405a3b6ea4927a490481f429726bf7060c6bb0220bb5`,
  `value_encoding=json`, `key_material_omitted=true`,
  `value_content_omitted=true`.
- Raw vault scan with `rg -a` found the marker string in the vault files; the
  MCP inspect and dump outputs did not contain the marker.
- Verdict: PASS.

### Edge 1: Empty Value

- Before `CF_KV`: `2` rows.
- Trigger: real MCP `workspace.put`, key `issue1660/edge-empty`, value `""`.
- Put readback: value bytes `444`,
  `value_sha256=sha256:e95e3081858d44323649c4c997354e4b8f346c0f824bfca2f475d420354b35b1`.
- Separate read: real MCP `workspace.get` returned the exact empty string.
- After `storage.summary`: `CF_KV=3`.
- After physical Calyx dump: row key hash
  `sha256:7987ac470ed572488afc2021ec5963823d3d077164ab201c6ec502faa22deb4c`,
  value hash `sha256:e95e3081858d44323649c4c997354e4b8f346c0f824bfca2f475d420354b35b1`.
- Raw vault scan found `issue1660/edge-empty` in vault files.
- Verdict: PASS.

### Edge 2: Maximum TTL Boundary

- Before `CF_KV`: `3` rows.
- Trigger: real MCP `workspace.put`, key `issue1660/edge-max-ttl`,
  `ttl_ms=604800000`, value marker `ISSUE1660_MAX_TTL_BOUNDARY_MARKER`.
- Put readback: value bytes `547`,
  `value_sha256=sha256:f34b976af6a654a2b34374942329a1d1d6dd5646a2f5d2d36fc2459c106b62fe`,
  `expires_at_unix_ms=created_at_unix_ms+604800000`.
- Separate read: real MCP `workspace.get` returned `ttl_ms=604800000` and the
  exact marker value.
- After `storage.summary`: `CF_KV=4`.
- After physical Calyx dump: row key hash
  `sha256:28dc27f63fe451df4ce1d699b076edae921f1bd10b7e90dd6691c1fee5960b8f`,
  value hash `sha256:f34b976af6a654a2b34374942329a1d1d6dd5646a2f5d2d36fc2459c106b62fe`.
- Raw vault scan found the max-TTL marker in vault files.
- Verdict: PASS.

### Edge 3: Structurally Invalid Inspect Parameters Fail Closed

- Before `CF_KV`: `4` rows.
- Trigger: real MCP `storage operation=inspect` with invalid
  `inspect={"include_raw":true}`.
- Result: failed closed with `TOOL_PARAMS_INVALID`, `unknown_field=include_raw`,
  accepted fields `[]`, source of truth `serde deny_unknown_fields`.
- After `storage.summary`: `CF_KV=4`, `CF_TIMELINE=3` because the failed tool
  call was audited; `CF_KV` size stayed unchanged.
- Verdict: PASS.

### Edge 4: Invalid Dump CF Fails Closed

- Before `CF_KV`: `4` rows.
- Trigger:
  `cargo run --quiet -p synapse-storage --example dump_cf -- --backend calyx <db> CF_DOES_NOT_EXIST`.
- Result: exit code `1`,
  `ReadFailed { cf_name: "CF_DOES_NOT_EXIST", detail: "column family name is not part of the Synapse storage schema" }`.
- After `storage.summary`: `CF_KV=4`, `CF_TIMELINE=3`, sizes unchanged.
- Verdict: PASS.

## Final Calyx State Evidence

Final `mcp__synapse.storage operation=inspect` on Calyx:

- Vault: schema `1`, vault id `01KXP1XQF68E28HT3RZ5P2YESA`, `latest_seq=16`,
  `collection_count=6`, `raw_row_count=16`, `live_row_count=16`,
  `expired_row_count=0`, `payload_bytes=16097`,
  `stored_value_bytes=16369`, `total_logical_bytes=16759`.
- `CF_KV/ns/0`: `raw_row_count=4`, `live_row_count=4`,
  `expired_row_count=0`, histogram `durable=4`, `payload_bytes=1804`,
  `stored_value_bytes=1872`, `total_logical_bytes=2274`,
  `user_key_bytes=470`.
- Public `cf_row_counts`: `CF_KV=4`, `CF_ACTION_LOG=6`, `CF_TIMELINE=3`.
- Public `cf_row_samples.CF_KV`: metadata-only rows with key/value SHA-256,
  lengths, `value_encoding=json`, and omission flags; no raw keys, values, or
  marker strings.

Final `mcp__synapse.workspace operation=list`, prefix `issue1660/`:

- Returned `3` rows and `scanned_rows=3`.
- `issue1660/edge-empty`: value `""`, value bytes `444`,
  `sha256:e95e3081858d44323649c4c997354e4b8f346c0f824bfca2f475d420354b35b1`.
- `issue1660/edge-max-ttl`: marker value present, `ttl_ms=604800000`, value
  bytes `547`,
  `sha256:f34b976af6a654a2b34374942329a1d1d6dd5646a2f5d2d36fc2459c106b62fe`.
- `issue1660/happy-secret`: marker value present, `ttl_ms=86400000`, value
  bytes `586`,
  `sha256:6fb26881dd1cc8ea32c8405a3b6ea4927a490481f429726bf7060c6bb0220bb5`.
- `corrupt_rows_skipped=[]`, `expired_rows_deleted=0`.

## Dump Parity Evidence

Calyx:

```text
dump_cf db_path=...\issue-1660-calyx-20260716-130949\db cf=CF_KV backend=calyx mode=read_only row_count=4
row[1] key_len_bytes=137 key_sha256=sha256:7987ac470ed572488afc2021ec5963823d3d077164ab201c6ec502faa22deb4c key_material_omitted=true value_len_bytes=444 value_sha256=sha256:e95e3081858d44323649c4c997354e4b8f346c0f824bfca2f475d420354b35b1 value_encoding=json value_content_omitted=true redaction_policy=metadata_only_no_raw_keys_or_values_hashes_for_correlation
row[2] key_len_bytes=141 key_sha256=sha256:28dc27f63fe451df4ce1d699b076edae921f1bd10b7e90dd6691c1fee5960b8f key_material_omitted=true value_len_bytes=547 value_sha256=sha256:f34b976af6a654a2b34374942329a1d1d6dd5646a2f5d2d36fc2459c106b62fe value_encoding=json value_content_omitted=true redaction_policy=metadata_only_no_raw_keys_or_values_hashes_for_correlation
row[3] key_len_bytes=141 key_sha256=sha256:ff8739ec6fbc307d809e8e290ea1e66f4588c1b1d70f4a28e9f0c16cfad50350 key_material_omitted=true value_len_bytes=586 value_sha256=sha256:6fb26881dd1cc8ea32c8405a3b6ea4927a490481f429726bf7060c6bb0220bb5 value_encoding=json value_content_omitted=true redaction_policy=metadata_only_no_raw_keys_or_values_hashes_for_correlation
```

RocksDB:

```text
dump_cf db_path=...\issue-1660-rocksdb-20260716-134500\db cf=CF_KV backend=rocksdb mode=read_only row_count=4
row[1] key_len_bytes=147 key_sha256=sha256:4ae5e01bdd2fd41dceb17bf90a1ba50cb24ce52ccd525065ac86af2db1bf9580 key_material_omitted=true value_len_bytes=459 value_sha256=sha256:70dbe2a7346720acf759e322eeeb468ee5b1eae96a058c1219c31876f159b82a value_encoding=json value_content_omitted=true redaction_policy=metadata_only_no_raw_keys_or_values_hashes_for_correlation
row[2] key_len_bytes=151 key_sha256=sha256:9d37f5c9c0addfe1625eba0eb0eb4518b4eddbbcc60b706472c8757b8ab76fb4 key_material_omitted=true value_len_bytes=570 value_sha256=sha256:679f1fa8fe8c0b0548d41632c62c2f8fcb7288a1d219628e4d7aa084ed66dc25 value_encoding=json value_content_omitted=true redaction_policy=metadata_only_no_raw_keys_or_values_hashes_for_correlation
row[3] key_len_bytes=151 key_sha256=sha256:f7aed23598381b9f16249fdc5a9912a2477dcdc386846a27f5e7b68b4c6d9c8b key_material_omitted=true value_len_bytes=591 value_sha256=sha256:d8a54e5afd59c759af325e07e6ddb601660bafa7f801b1a97389cde470507e6a value_encoding=json value_content_omitted=true redaction_policy=metadata_only_no_raw_keys_or_values_hashes_for_correlation
```

Both backends used the same explicit `--backend <rocksdb|calyx> <db_path>
<cf_name>` interface and emitted the same metadata-only output contract:
row count, key length, key SHA-256, key omission flag, value length, value
SHA-256, encoding, value omission flag, and redaction policy. Physical key and
value hashes differ because the backends use different durable row encodings
and timestamps.

Raw RocksDB file scans found both marker strings in the DB files while the
dump/inspect outputs did not emit them. Lock-file access errors occurred while
the live RocksDB daemon owned the DB; the data-file matches were still printed.

## Host Restoration

- Stopped only the owned isolated Calyx PID `47644`.
- Stopped only the owned isolated RocksDB PID `70372`.
- Re-enabled and started scheduled task `SynapseMcpDaemon`.
- Installed the repo-built daemon with
  `scripts\synapse-setup.ps1 -SourceDir C:\code\Synapse -ForceRestart -SkipClientWiring -ActiveIssue 1660`.
- Setup built and installed binary SHA256
  `7A94967D681D77FA499FC944A90DF7285476E990682464ABB8456495F503EEEE`.
- Setup then exited nonzero with
  `SYNAPSE_CHROME_BRIDGE_WAIT_HEALTH_FAILED` while waiting for Chrome bridge
  reconnect health. Immediate separate readback showed the daemon and bridge
  were healthy. Tracked as GitHub issue #1714.
- Final scheduled daemon SoT:
  task `SynapseMcpDaemon=Running`, PID `60560`, executable
  `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`, listener `127.0.0.1:7700`,
  DB `C:\Users\hotra\AppData\Local\synapse\db-daemon`.
- Final `mcp__synapse.health`: `ok=true`, `tool_count=40`,
  `tool_surface_sha256=d4ef68fb707f9ff6e2fd6ef452bc88df0f0d2b0713f692fb5b462369b470dc7c`,
  `chrome_bridge.status=ok`, `host_count=1`, `tab_control_available=true`.

## Structural Checks

These checks prove formatting/lint/compilation only. They are not FSV.

```text
cargo fmt --all --check
cargo check -p synapse-calyx -p synapse-storage -p synapse-reflex -p synapse-mcp
cargo clippy -p synapse-calyx -p synapse-storage -p synapse-reflex -p synapse-mcp --all-targets
cargo build --release -p synapse-mcp
```

All completed successfully. No automated tests or FSV harnesses were created or
run.
