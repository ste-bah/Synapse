# Manual FSV: Issue #1659 Calyx GC Parity

Date: 2026-07-16

## Source of Truth

- Physical storage SoT: isolated Calyx vault directories under
  `C:\Users\hotra\AppData\Local\Temp\synapse-issue-1659-runs\run-1784212049241`.
- Raw row SoT: Aster `ColumnFamily::Kv` rows scanned by logical Synapse CF
  namespace with a separate raw `SynapseCalyxVault` open after each trigger.
- Logical row SoT: `synapse_storage::Db::scan_cf` readbacks against the same
  Calyx vaults.
- Periodic task SoT: `GcTaskReadback` read after `Db::spawn_gc_task`.
- Evidence file SoT:
  `C:\Users\hotra\AppData\Local\Temp\synapse-issue-1659-runs\run-1784212049241\manual_gc_report.json`,
  SHA256
  `F6394B04E797DCFEB00CD7AE7D0254311CCF0A14B7B8A174994E239072916CA1`.

Separate disk readback after the MCP-triggered run found these physical Calyx
case directories: `row-cap-happy`, `empty-namespace`, `invalid-caps`,
`protected-cf`, `expired-ttl`, `malformed-envelope`, and `periodic-task`.
Each directory contained durable Aster vault files.

## MCP Preconditions

- Real MCP daemon: `synapse-mcp.exe` PID `75204`.
- Executable: `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`.
- Socket SoT: `127.0.0.1:7700` listener owned by PID `75204`.
- `mcp__synapse.health` returned `ok=true`, `tool_count=40`,
  `tool_surface_sha256=e20cb889682709ec22f9b571f043da594ffe1d6c40168566235fe45d4654bb12`.
- The wired client loaded the 40-tool public facade, including `health`,
  `shell`, `storage`, and `profile`.
- Health proved the Calyx runtime is open and CUDA-backed:
  math backend `cuda`, device `NVIDIA GeForce RTX 5090`.

The live daemon storage backend is RocksDB, and the public storage facade does
not expose a parameter for selecting an isolated Calyx backend. The Calyx
behavior trigger was therefore the real wired `mcp__synapse.shell` tool running
an uncommitted one-off manual command outside the repo against isolated Calyx
vaults, followed by separate raw vault readbacks.

## Trigger

MCP tool call:

```text
mcp__synapse.shell run
job_id=019f6b51-3b60-7201-a2b6-1f6163c6c73d
command=cargo
args=run --quiet
working_dir=C:\Users\hotra\AppData\Local\Temp\synapse-issue-1659-manual
CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3
```

Job result: exit code `0`, stdout length `30958`, stderr length `110`
(`REPORT_PATH=...manual_gc_report.json`), completed in `141254 ms`. The MCP
job Source of Truth was `%LOCALAPPDATA%\Synapse\shell-jobs` plus
`%LOCALAPPDATA%\Synapse\shell-sessions`.

## Manual State Readbacks

### Happy Path: Row Cap Evicts Oldest Rows

- Before raw `CF_KV`: `k001`, `k002`, `k003`, `k004`, `k005`.
- Trigger: `Db::run_gc_once_with_row_caps(CF_KV, soft=3, hard=5)`.
- Report: `before_value=5`, `after_value=3`, `evicted_rows=2`,
  `before_estimated_num_keys=5`, `after_estimated_num_keys=3`.
- After raw `CF_KV`: `k003`, `k004`, `k005`.
- Verdict: PASS.

### Edge: Empty Namespace No-Op

- Before raw `CF_EVENTS`: `0` rows.
- Trigger: `Db::run_gc_once_with_row_caps(CF_EVENTS, soft=3, hard=5)`.
- Report: `before_value=0`, `after_value=0`, `evicted_rows=0`.
- After raw `CF_EVENTS`: `0` rows.
- Verdict: PASS.

### Edge: Invalid Caps Fail Closed

- Before raw `CF_KV`: `keep-001`, `keep-002`.
- Trigger: `Db::run_gc_once_with_row_caps(CF_KV, soft=3, hard=2)`.
- Result: `storage write failed in CF_KV: invalid Calyx GC rows cap:
  hard_cap=2 is below soft_cap=3`.
- After raw `CF_KV`: `keep-001`, `keep-002`.
- Verdict: PASS.

### Edge: Protected CF Is Not Evicted

- Before raw `CF_ROUTINE_STATE`: `routine-001`, `routine-002`,
  `routine-003`.
- Trigger: `Db::run_gc_once_with_row_caps(CF_ROUTINE_STATE, soft=1, hard=2)`.
- Report: `before_value=3`, `after_value=3`, `evicted_rows=0`,
  `hard_cap_reached=true`,
  `eviction_skipped_reason=protected_cf_policy_skipped`.
- After raw `CF_ROUTINE_STATE`: all three rows still present.
- Verdict: PASS.

### Edge: Expired TTL Row Is Tombstoned And Purged

- Before logical `CF_EVENTS`: `0` rows.
- Before raw `CF_EVENTS`: one expired envelope, key `expired-001`,
  `expires_at_ms=1`, `written_at_ms=1`, payload `expired-payload`.
- Trigger: `Db::run_gc_once()` default retention budgets.
- `CF_EVENTS` report: `before_value=0`, `after_value=0`,
  `before_estimated_num_keys=1`, `after_estimated_num_keys=0`,
  `evicted_rows=1`.
- After raw `CF_EVENTS`: `0` rows.
- Verdict: PASS.

### Edge: Malformed Envelope Fails Closed

- Before raw `CF_KV`: key `bad-envelope`, value bytes `990102`.
- Trigger: `Db::run_gc_once_with_row_caps(CF_KV, soft=1, hard=2)`.
- Result: `storage write failed in CF_KV: decode Calyx retention envelope:
  Calyx KV value version 153 is unsupported; expected 1 or 2`.
- After raw `CF_KV`: key `bad-envelope`, value bytes `990102` still present.
- Verdict: PASS.

### Periodic Task

- Trigger: `Db::spawn_gc_task()` inside a real Tokio runtime.
- Readback: `running=true`, `last_started_unix_ms=1784212053904`,
  `last_completed_unix_ms=1784212053904`, `last_duration_ms=0`,
  `last_error=null`, `last_unsupported_policy_skips=[]`.
- After raw `CF_KV`: `0` rows.
- Verdict: PASS.

## Public Storage Facade Note

`mcp__synapse.storage operation=gc_once` was attempted against the live daemon
with `CF_MODEL_CACHE` while the row count was `0`. The call failed closed with
`TOOL_PROFILE_POLICY_DENIED` because the current session profile is
`normal_agent` and profile status shows read-only storage grants
(`READ_STORAGE`, no `WRITE_STORAGE`). A separate `mcp__synapse.profile
operation=status` readback proved the source of truth:
`CF_SESSIONS mcp/tool-profile/v1/<session_id>`.

This was not treated as a Calyx GC failure; it proves the live public facade is
maintenance-gated. The isolated Calyx behavior was verified through the real
MCP `shell` trigger above because the public storage facade cannot select an
isolated Calyx backend.

## Structural Checks

These are compile/lint/format checks only, not FSV:

```text
cargo fmt --all --check
cargo check -p synapse-calyx -p synapse-storage -p synapse-mcp
cargo clippy -p synapse-calyx -p synapse-storage -p synapse-mcp --all-targets
```

All completed successfully.
