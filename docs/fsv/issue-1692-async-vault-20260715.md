# Issue #1692 async Calyx vault facade manual FSV - 2026-07-15

## Scope

Issue #1692 needs a tokio-safe facade over the synchronous Calyx `AsterVault`.
This change adds the async facade in `synapse-calyx` with:

- bounded Tokio `mpsc` command queue for backpressure;
- a dedicated `synapse-calyx-vault` owner thread for the long-lived synchronous vault;
- oneshot replies that resolve only after `AsterVault::write_cf_batch` returns from the WAL-backed commit path;
- async pinned reader leases, explicit release, expired-lease cleanup, and structured errors;
- explicit `close()` that flushes, releases pinned leases, closes the vault, and joins the worker.

The full daemon/live-capture/kill-restart acceptance remains dependent on #1656 because the current <=40 Synapse MCP tool surface has no Calyx row write/read tool. I proved the real Synapse MCP precondition and then FSV-verified this library slice against an isolated physical vault Source of Truth.

## Root cause and design

Root cause: Synapse's daemon is tokio-based, but the absorbed Calyx `AsterVault` API is synchronous and performs real file/WAL work. Calling it directly from async tasks would risk blocking tokio executor workers and would give no bounded admission point for capture-rate writes.

Research used:

- Exa MCP query: official Tokio docs for `spawn_blocking`, bounded `mpsc`, and sync/async bridging.
- Native web query: same primary Tokio docs.

Relevant primary-source conclusions:

- Tokio `spawn_blocking` is for bounded blocking work that eventually finishes; for persistent loops, Tokio recommends a dedicated `thread::spawn` thread. Source: https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html
- Tokio bounded `mpsc` provides backpressure, and sync-side receivers should use `blocking_recv` when bridging sync and async code. Source: https://docs.rs/tokio/latest/tokio/sync/mpsc/index.html
- Tokio file APIs still use ordinary blocking OS file operations behind `spawn_blocking`, and batching file I/O into fewer blocking calls improves performance. Source: https://docs.rs/tokio/latest/tokio/fs/index.html
- Tokio's bridging guidance supports isolating sync/async boundaries explicitly. Source: https://tokio.rs/tokio/topics/bridging

Implementation choice: use `spawn_blocking` only for the bounded open/join operations; move the opened synchronous vault to one dedicated owner thread; communicate with a bounded Tokio `mpsc` queue and oneshot replies. This preserves Aster's existing durable group-commit semantics instead of adding a second batching layer that could weaken ordering.

## MCP precondition

Real Synapse MCP was live before FSV:

- Process SoT: `synapse-mcp.exe`, PID `46828`, path `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`, start `2026-07-15 16:37:51`.
- Socket SoT: `127.0.0.1:7700` had a `Listen` row owned by PID `46828`.
- Codex config SoT: `codex mcp get synapse` reported enabled Streamable HTTP at `http://127.0.0.1:7700/mcp` with bearer token env var `SYNAPSE_BEARER_TOKEN`.
- Wired client schema/tool proof: `mcp__synapse.health({"detail":"compact"})` returned `ok=true`, `pid=46828`, `tool_count=40`, and tool names including `health`; no tools/list schema error occurred.
- Calyx daemon status readback: health reported daemon Calyx vault open at `C:\Users\hotra\AppData\Roaming\synapse\vault`, `latest_seq=0`.

No Calyx row write/read MCP tool exists in the current tool list, so the MCP trigger requirement cannot yet be applied to this library behavior. #1656 is the existing issue that will expose the daemon Calyx KV backend and allow the full daemon-trigger FSV.

## Source of Truth

Post-refactor isolated physical vault directory:

`C:\Users\hotra\AppData\Local\Temp\synapse-issue1692-async-vault-fsv-final`

SoT read methods:

- vault status from a fresh open/close process;
- `wal\00000000000000000000.wal` byte length;
- `cf\kv\*.sst` durable file inventory and raw bytes;
- separate fresh-process read/scan through the new async facade.

Synthetic known row:

- key: `issue1692-final-key`
- value: `issue1692-final-value`
- expected value bytes: `[105, 115, 115, 117, 101, 49, 54, 57, 50, 45, 102, 105, 110, 97, 108, 45, 118, 97, 108, 117, 101]`

The trigger was a temporary uncommitted manual example used only for this run. It was deleted before commit and was not committed as a test, benchmark, script, or FSV harness.

## Manual FSV evidence

Initial SoT:

- Fresh directory `C:\Users\hotra\AppData\Local\Temp\synapse-issue1692-async-vault-fsv-final` existed with `child_count=0`.
- First open/close readback: `latest_seq=0`, `last_recovered_seq=0`, `safe_to_unlock=true`, `pid_sidecar_present_after_close=false`, `re_lock_probe_succeeded=true`.
- Before happy write: WAL segment length `0`; raw search for key/value returned no matches.

Happy path:

- Trigger: write one `ColumnFamily::Kv` row with key/value above through async facade, then flush and close.
- Trigger output: `committed_seq=1`; close readback `safe_to_unlock=true`, `latest_seq=1`.
- Separate after-read:
  - fresh read at snapshot `1`: `read_value=Some([105, 115, 115, 117, 101, 49, 54, 57, 50, 45, 102, 105, 110, 97, 108, 45, 118, 97, 108, 117, 101])`;
  - scan at snapshot `1` returned exactly the key bytes for `issue1692-final-key` and value bytes for `issue1692-final-value`;
  - WAL length became `99`;
  - raw byte search found the key and value in:
    - `wal\00000000000000000000.wal`;
    - `cf\kv\00000000000000000001-0000.sst`;
    - `cf\kv\flush-00000000000000000001-0001.sst`.

Edge 1 - empty batch:

- Before: `latest_seq=1`, `last_recovered_seq=1`, WAL length `99`.
- Trigger: async `write_cf_batch(Vec::new())`, flush, close.
- Trigger output: `before_seq=Some(1) returned_seq=1 after_seq=Some(1)`.
- After SoT read: status still `latest_seq=1`; WAL still `99`; scan at snapshot `1` still returned only the happy-path row.

Edge 2 - missing pinned lease id:

- Before: `latest_seq=1`, WAL length `99`.
- Trigger: `read_cf_pinned(9999999, ColumnFamily::Kv, issue1692-final-key)`.
- Trigger output: structured error `SYNAPSE_CALYX_READER_LEASE_MISSING` with remediation to re-issue a bounded reader lease; close readback `safe_to_unlock=true`, `latest_seq=1`.
- After SoT read: status still `latest_seq=1`; WAL still `99`; scan at snapshot `1` still returned only the happy-path row.

Edge 3 - expired pinned lease:

- Before: `latest_seq=1`, WAL length `99`.
- Trigger: pin reader with `max_age_ms=1`, sleep 10 ms, then `read_cf_pinned` using that lease.
- Trigger output: structured error `CALYX_READER_LEASE_EXPIRED`; close readback `safe_to_unlock=true`, `latest_seq=1`.
- After SoT read: status still `latest_seq=1`; WAL still `99`; fresh read at snapshot `1` still returned the happy-path value.

Edge 4 - invalid queue capacity:

- Before: no PID sidecar; WAL length `99`; status `latest_seq=1`.
- Trigger: async open with `queue_capacity=0`.
- Trigger output: structured error `SYNAPSE_CALYX_ASYNC_QUEUE_CAPACITY_INVALID`; no vault open/worker start.
- After SoT read: no PID sidecar; WAL still `99`; status still `latest_seq=1`; fresh read at snapshot `1` still returned the happy-path value.

Pinned lease happy path:

- Before: `latest_seq=1`, WAL length `99`.
- Trigger: pin `FreshDerived` reader lease for 60,000 ms, read pinned key, release lease, close.
- Trigger output: lease `lease_id=1`, `pinned_seq=1`, `max_age_ms=60000`; pinned read returned the happy-path value bytes; `released=true`.
- After SoT read: status still `latest_seq=1`; WAL still `99`; raw byte search still found the key/value in the WAL and both durable KV SST files.

## Structural checks

Supporting structural checks only; these are not FSV:

- `cargo fmt --package synapse-calyx --check` passed after removing the temporary manual trigger.
- `cargo check -p synapse-calyx` passed.
- `cargo check -p synapse-calyx --examples` passed while the temporary manual trigger existed.
- After removing the temporary manual trigger, `cargo check -p synapse-calyx` passed.
- After removing the temporary manual trigger, `cargo clippy -p synapse-calyx --all-targets` passed with only the existing `calyx-forge` CUDA feature warning.

No tests, benchmarks, GitHub Actions, CI, or automated FSV harnesses were run.

## Result

The async facade slice is manually FSV-verified against physical vault state for happy path, empty input, missing lease, expired lease, invalid capacity, and pinned read/release. The broader #1692 daemon/live-capture/kill-restart acceptance remains open until #1656 lands a real daemon Calyx backend/tool path that can be triggered through MCP and verified directly at the vault SoT.
