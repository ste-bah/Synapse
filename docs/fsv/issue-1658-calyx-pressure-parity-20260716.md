# Manual FSV: Issue #1658 Calyx Disk-Pressure Parity

Date: 2026-07-16

## Source of Truth

- Physical storage SoT: isolated Calyx vault directories under
  `C:\Users\hotra\AppData\Local\Temp\synapse1658-manual-data`.
- Raw row SoT: `calyx_aster::ColumnFamily::Kv` rows decoded from each Synapse
  Calyx namespace.
- Logical read SoT: `synapse_storage::Db` readbacks against the same live Calyx
  vaults.
- Pressure state SoT: live `Db` pressure state and pressure probe readback
  immediately after each synthetic pressure trigger.
- Evidence file SoT:
  `C:\Users\hotra\AppData\Local\Temp\synapse1658-manual-data\report.json`,
  SHA256
  `80A80E8DAAE5A92E298E51AF76106C859CC5AB33820F55A57A7EAA0BDB35ABBA`.

Separate disk readback after the MCP-triggered run found these physical vault
directories: `normal_vault`, `level3_vault`, `level4_vault`, `recovery_vault`,
`level2_boundary_vault`, `empty_write_vault`, and `unknown_cf_vault`.

## MCP Preconditions

- Real MCP daemon: `synapse-mcp.exe` PID `75204`.
- Executable: `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`.
- Socket SoT: `127.0.0.1:7700` listener owned by PID `75204`.
- `mcp__synapse.health` returned `ok=true`, `tool_count=40`,
  `tool_surface_sha256=e20cb889682709ec22f9b571f043da594ffe1d6c40168566235fe45d4654bb12`.
- The loaded tool surface included `shell`, `storage`, `health`, and the rest
  of the 40-tool public facade, proving client-side schema validation passed.
- Health also proved the Calyx runtime is open and CUDA-backed:
  vault `C:\Users\hotra\AppData\Roaming\synapse\vault`,
  math backend `cuda`, device `NVIDIA GeForce RTX 5090`.

The public 40-tool storage facade exposes inspect/summary/GC operations, not a
synthetic pressure sample operation for an isolated Calyx backend. The behavior
trigger for #1658 was therefore the real wired `mcp__synapse.shell` tool running
an uncommitted temp storage command against isolated Calyx vaults, followed by
separate raw vault and logical state readbacks.

## Trigger

MCP tool call:

```text
mcp__synapse.shell run
job_id=019f6b36-3e17-75c1-8f96-2320d5c811ed
command=cargo
args=run --quiet
working_dir=C:\Users\hotra\AppData\Local\Temp\synapse1658-manual
SYNAPSE_1658_ROOT=C:\Users\hotra\AppData\Local\Temp\synapse1658-manual-data
CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3
```

Job result: exit code `0`, stderr length `0`, stdout length `18553`, completed
in `58740 ms`. The MCP job Source of Truth was
`%LOCALAPPDATA%\Synapse\shell-jobs` plus
`%LOCALAPPDATA%\Synapse\shell-sessions`.

An earlier attempt, job `019f6b35-e7c7-7033-8ab0-5a8093ae7a58`, failed before
storage behavior executed because the long-lived daemon shell environment
inherited stale `CUDA_PATH=/usr/local/cuda-13.3`. Host SoT immediately proved
Windows CUDA v13.3 exists at
`C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3`, `nvcc.exe`
resolves there, and both User and Machine `CUDA_PATH` point there. Follow-up
issue #1709 records that host-shell environment defect.

## Manual State Readbacks

### Happy Path: Normal Allows Timeline Write

- Before: vault did not exist; raw `CF_TIMELINE=[]`.
- Trigger: sample `2500000000` free bytes, then write `CF_TIMELINE` value
  `normal-timeline`.
- Trigger result: pressure level `Normal`, `gc_advised=false`, write `ok=true`.
- After pressure: level `Normal`; all checked CF permits were `true`.
- After logical: `CF_TIMELINE=6e6f726d616c2d74696d656c696e65`.
- After raw: one `CF_TIMELINE` row, version `2`, payload length `15`, payload
  SHA256 `sha256:732fe913fe36accc38266a3374f1262510ae2796ace7baff68807278d0edd58e`.
- Verdict: PASS.

### Edge: Level3 Sheds Timeline But Allows Agent Events

- Before: vault did not exist; raw `CF_TIMELINE=[]`, `CF_AGENT_EVENTS=[]`.
- Trigger: sample `400000000` free bytes, then write `CF_TIMELINE` and
  `CF_AGENT_EVENTS`.
- Trigger result: pressure level `Level3`, emitted
  `STORAGE_DISK_PRESSURE_LEVEL_3`, `gc_advised=true`, compaction attempted
  across all 17 Synapse CF names.
- Timeline write result: `WriteShed`,
  `storage write shed in CF_TIMELINE under disk pressure Level3: 1 rows`.
- Agent-events write result: `ok=true`.
- After pressure: level `Level3`; permits
  `CF_TIMELINE=false`, `CF_AGENT_EVENTS=true`, `CF_KV=true`,
  `CF_SESSIONS=true`; transition history contained
  `STORAGE_DISK_PRESSURE_LEVEL_3`.
- After logical/raw: `CF_TIMELINE` absent with `0` raw rows;
  `CF_AGENT_EVENTS=6167656e742d6576656e742d6f6b` with one raw row.
- Verdict: PASS.

### Edge: Level4 Sheds KV But Allows Sessions

- Before: vault did not exist; raw `CF_KV=[]`, `CF_SESSIONS=[]`.
- Trigger: sample `100000000` free bytes, then write `CF_KV` and
  `CF_SESSIONS`.
- Trigger result: pressure level `Level4`, emitted
  `STORAGE_DISK_PRESSURE_LEVEL_4`, `gc_advised=true`, compaction attempted
  across all 17 Synapse CF names.
- KV write result: `WriteShed`,
  `storage write shed in CF_KV under disk pressure Level4: 1 rows`.
- Sessions write result: `ok=true`.
- After pressure: level `Level4`; permits
  `CF_KV=false`, `CF_TIMELINE=false`, `CF_AGENT_EVENTS=false`,
  `CF_SESSIONS=true`; transition history contained
  `STORAGE_DISK_PRESSURE_LEVEL_4`.
- After logical/raw: `CF_KV` absent with `0` raw rows;
  `CF_SESSIONS=73657373696f6e2d6f6b` with one raw row.
- Verdict: PASS.

### Edge: Recovery To Normal Reopens Timeline

- Before: vault did not exist; raw `CF_TIMELINE=[]`.
- Trigger: sample `100000000`, attempt failed `CF_TIMELINE` write, sample
  `2500000000`, then write `CF_TIMELINE=recovered-ok`.
- Trigger result: first sample level `Level4`, emitted
  `STORAGE_DISK_PRESSURE_LEVEL_4`; failed write returned `WriteShed`. Recovery
  sample level was `Normal`, `gc_advised=false`; recovered write `ok=true`.
- After pressure: level `Normal`; all checked CF permits `true`; transition
  history still contained `STORAGE_DISK_PRESSURE_LEVEL_4`.
- After logical/raw: one `CF_TIMELINE` row with payload
  `7265636f76657265642d6f6b`.
- Verdict: PASS.

### Edge: Level2 Boundary Allows Writes And Advises GC

- Before: vault did not exist; raw `CF_TIMELINE=[]`.
- Trigger: sample exact boundary `500000000` free bytes, then write
  `CF_TIMELINE=level2-ok`.
- Trigger result: pressure level `Level2`, emitted
  `STORAGE_DISK_PRESSURE_LEVEL_2`, `gc_advised=true`, compaction attempted
  across all 17 Synapse CF names, write `ok=true`.
- After pressure: level `Level2`; all checked CF permits `true`; transition
  history contained `STORAGE_DISK_PRESSURE_LEVEL_2`.
- After logical/raw: one `CF_TIMELINE` row with payload
  `6c6576656c322d6f6b`.
- Verdict: PASS.

### Edge: Empty Write Under Level4 Is A No-Op

- Before: vault did not exist; raw `CF_KV=[]`.
- Trigger: sample `100000000` free bytes, then empty `put_batch(CF_KV, [])`.
- Trigger result: pressure level `Level4`, emitted
  `STORAGE_DISK_PRESSURE_LEVEL_4`, empty write `ok=true`.
- After pressure: level `Level4`; `CF_KV` permit was `false`.
- After logical/raw: `CF_KV` row count `0`, raw `CF_KV=[]`.
- Verdict: PASS.

### Edge: Unknown CF Fails Closed Without Write

- Before: vault did not exist; raw `CF_KV=[]`.
- Trigger: sample normal free bytes, then write `CF_NOT_REAL`.
- Trigger result: `WriteFailed`,
  `storage write failed in CF_NOT_REAL: column family name is not part of the Synapse storage schema`.
- After pressure: level `Normal`; transition history empty.
- After logical/raw: `CF_KV` row count `0`, raw `CF_KV=[]`.
- Verdict: PASS.

## Report Readback Summary

Separate report parse after the run:

```text
happy_normal_allows_timeline_write: PASS, after_level=Normal, raw CF_TIMELINE=1
edge_level3_sheds_timeline_but_allows_agent_events: PASS, after_level=Level3, raw CF_AGENT_EVENTS=1;CF_TIMELINE=0
edge_level4_sheds_kv_but_allows_sessions: PASS, after_level=Level4, raw CF_KV=0;CF_SESSIONS=1
edge_recovery_to_normal_reopens_timeline: PASS, after_level=Normal, raw CF_TIMELINE=1
edge_level2_boundary_allows_writes_and_advises_gc: PASS, after_level=Level2, raw CF_TIMELINE=1
edge_empty_write_noop_under_level4: PASS, after_level=Level4, raw CF_KV=0
edge_unknown_cf_fails_closed_without_write: PASS, after_level=Normal, raw CF_KV=0
```

Physical vault directory readback:

```text
normal_vault: 20 files, 6035 bytes
level3_vault: 21 files, 6203 bytes
level4_vault: 21 files, 6201 bytes
recovery_vault: 21 files, 6197 bytes
level2_boundary_vault: 21 files, 6188 bytes
empty_write_vault: 16 files, 4106 bytes
unknown_cf_vault: 15 files, 3935 bytes
```

## Structural Checks

These are compile/lint checks only, not FSV:

- `cargo fmt --all --check` PASS.
- `cargo check -p synapse-calyx -p synapse-storage -p synapse-mcp` PASS.
- `cargo clippy -p synapse-calyx -p synapse-storage -p synapse-mcp --all-targets` PASS.

No automated tests, benches, FSV scripts, FSV harnesses, GitHub Actions, or CI
were added or run.

## Process Hygiene

- Durable MCP job `019f6b36-3e17-75c1-8f96-2320d5c811ed` completed and the
  job-owned process tree was no longer running.
- Exact owned PIDs from the failed and successful jobs were checked after
  completion; no rows remained for PIDs `6768`, `70968`, `78792`, `73480`, or
  `79832`.
- The intended long-lived daemon remained `synapse-mcp.exe` PID `75204` on
  `127.0.0.1:7700`.

## Research Used

- RocksDB write stalls: writes must slow/stop when flush or compaction cannot
  keep up, with logs and non-blocking failure support for callers that cannot
  wait indefinitely.
  https://github.com/facebook/rocksdb/wiki/Write-Stalls
- CockroachDB admission control: priority queues for storage IO protect
  critical work when stateful nodes are hot.
  https://www.cockroachlabs.com/docs/stable/admission-control
- Kubernetes node-pressure eviction: pressure signals, thresholds, reclaim
  before shedding, and priority ordering.
  https://kubernetes.io/docs/concepts/scheduling-eviction/node-pressure-eviction/
