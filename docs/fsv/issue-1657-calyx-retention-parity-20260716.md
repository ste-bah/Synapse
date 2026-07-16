# Manual FSV: Issue #1657 Calyx Retention Parity

Date: 2026-07-16

## Source of Truth

- Physical storage SoT: isolated Calyx vault directories under
  `C:\Users\hotra\AppData\Local\Temp\synapse1657-manual-data`.
- Raw row SoT: `calyx_aster::ColumnFamily::Kv` rows in each vault namespace,
  decoded from the Synapse Calyx envelope.
- Logical read SoT: `synapse_storage::Db` readbacks against the same Calyx
  vaults.
- Evidence file SoT: `C:\Users\hotra\AppData\Local\Temp\synapse1657-manual-data\report.json`
  with SHA256 `333ED6D722AB66182EEC803E60226BE070F8150FA672BE8B557021BC7683E7BF`.

The evidence file was read back from disk separately after the MCP-triggered
run. The temp data root contained these physical vault directories:
`ttl_happy_vault`, `non_ttl_vault`, `expired_vault`, `malformed_vault`,
`malformed_preflight_vault`, `cap_vault`, and `oversized_key_vault`.

## MCP Preconditions

- Real MCP daemon: `synapse-mcp.exe` PID `75204`.
- Executable: `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`.
- Socket SoT: `127.0.0.1:7700` listener owned by PID `75204`.
- `mcp__synapse.health` returned `ok=true`, `tool_count=40`,
  `tool_surface_sha256=e20cb889682709ec22f9b571f043da594ffe1d6c40168566235fe45d4654bb12`.
- Health also proved the Calyx runtime is open and CUDA-backed:
  vault `C:\Users\hotra\AppData\Roaming\synapse\vault`,
  math backend `cuda`, device `NVIDIA GeForce RTX 5090`.
- `mcp__synapse.storage(operation=inspect)` proved the live public storage
  facade is currently `rocksdb`; there is no public MCP storage operation that
  selects an isolated Calyx backend. The behavior trigger for this FSV was
  therefore the real wired MCP `mcp__synapse.shell` tool running the temp
  Calyx storage exerciser, followed by separate raw vault/logical readbacks.

## Trigger

MCP tool call:

```text
mcp__synapse.shell start
job_id=issue1657-manual-fsv-run7
command=cargo
args=run --quiet
working_dir=C:\Users\hotra\AppData\Local\Temp\synapse1657-manual
SYNAPSE_1657_ROOT=C:\Users\hotra\AppData\Local\Temp\synapse1657-manual-data
```

Job result: exit code `0`, stderr length `0`, stdout length `12433`, completed
in `16382 ms`.

## Manual State Readbacks

### Happy Path: TTL CF_EVENTS

- Before: logical scan count `0`; raw rows `[]`.
- Trigger: write `CF_EVENTS` key `happy-event` value `value-happy`.
- After logical: `happy-event=76616c75652d6861707079`.
- After raw: one row, version `2`, key `happy-event`, payload length `11`,
  payload SHA256 `30420aefc5decb86564262d0ea82ff5ad07bb6c08f99b124c23f31bb39ca49c8`,
  `written_at_ms=1784208498651`, `expires_at_ms=1784294898651`.
- Verdict: `expires_at_ms - written_at_ms = 86400000`, PASS.

### Edge: Non-TTL CF_KV

- Before: logical scan count `0`; raw rows `[]`.
- Trigger: write `CF_KV` key `durable-kv` value `value-kv`.
- After logical: `durable-kv=76616c75652d6b76`.
- After raw: one row, version `2`, key `durable-kv`, `expires_at_ms=0`,
  `written_at_ms=1784208500195`, payload length `8`.
- Verdict: non-TTL row is durable and still timestamped for caps, PASS.

### Edge: Expired Row Invisible and Reclaimed

- Before: raw rows `[]`.
- Setup readback before trigger: raw `expired-event` row existed with
  `expires_at_ms=1784208496811`, `written_at_ms=1784208491811`; logical get was
  `null`, scan count `0`.
- Trigger: write `CF_EVENTS` key `sweeper` value `new-live`.
- After logical: `expired-event=null`, `sweeper=6e65772d6c697665`, scan keys
  `["sweeper"]`.
- After raw: only `sweeper` remained; `expired-event` was absent.
- Verdict: expired data is invisible before cleanup and reclaimed on write,
  PASS.

### Edge: Malformed Envelope Fails Closed on Read

- Before: logical scan count `0`; raw rows `[]`.
- Trigger: raw insert unsupported envelope version `99`, then read
  `CF_EVENTS/bad-envelope` through `Db::get_cf`.
- Trigger result: `storage read failed in CF_EVENTS: Calyx KV value version 99 is unsupported; expected 1 or 2`.
- After raw: one diagnostic row remained, key `bad-envelope`, version `99`,
  payload SHA256 `b1f6db13d4e7c8f78f812aa0bc0dee11f5e901682d9190ee0622480fa3e126dc`.
- Verdict: malformed data errors and is not hidden or rewritten, PASS.

### Edge: Malformed Envelope Blocks Partial Write

- Before: raw rows `[]`.
- Setup readback before trigger: raw `bad-envelope` row existed with version
  `99`.
- Trigger: attempt normal write `CF_EVENTS/new-after-corruption=should-not-write`.
- Trigger result: `storage write failed in CF_EVENTS: decode Calyx retention envelope: Calyx KV value version 99 is unsupported; expected 1 or 2`.
- After raw: only `bad-envelope` remained; `new-after-corruption` was absent.
- Verdict: existing corruption fails the write before committing new rows, PASS.

### Edge: Soft-Cap Eviction

- Before: raw rows `[]`.
- Trigger: write 12 `CF_KV` rows `cap-000..cap-011`, each with a 1 MiB payload.
- After logical keys:
  `cap-003`, `cap-004`, `cap-005`, `cap-006`, `cap-007`, `cap-008`,
  `cap-009`, `cap-010`, `cap-011`.
- After raw: 9 rows, each version `2`, each `expires_at_ms=0`, each payload
  length `1048576`.
- Actual live bytes: `9437247`; soft cap bytes: `10485760`.
- Verdict: oldest rows `cap-000..cap-002` were removed and live bytes are under
  soft cap, PASS.

### Edge: Oversized Key Fails Closed

- Before: raw rows `[]`.
- Trigger: write `CF_KV` with key length `65536`.
- Trigger result: `storage write failed in CF_KV: Calyx Synapse KV envelope supports keys up to 65535 bytes; got 65536`.
- After raw: `[]`.
- Verdict: invalid input errors before any row is written, PASS.

## Structural Checks

These are compile/lint checks only, not FSV:

- `cargo fmt --all --check` PASS.
- `cargo check -p synapse-calyx -p synapse-storage` PASS.
- `cargo clippy -p synapse-calyx -p synapse-storage --all-targets` PASS.

No automated tests, benches, FSV scripts, or FSV harnesses were added or run.

## Process Hygiene

- Durable MCP job `issue1657-manual-fsv-run7` completed and the job-owned
  process tree was no longer running.
- Explicit process readback found no remaining `cargo.exe`/manual exerciser
  PIDs from the run.
- The intended long-lived daemon remained `synapse-mcp.exe` PID `75204` on
  `127.0.0.1:7700`.

## Research Used

- Redis `EXPIRE` documentation: passive expiry on access plus active cleanup.
  https://redis.io/docs/latest/commands/expire/
- RocksDB TTL wiki: TTL data can remain until compaction and reads may see
  expired records unless the caller enforces expiry.
  https://github.com/facebook/rocksdb/wiki/Time-to-Live
- Caffeine eviction wiki: separate time expiry and size eviction, with
  maintenance driven during writes and reads.
  https://github.com/ben-manes/caffeine/wiki/Eviction
