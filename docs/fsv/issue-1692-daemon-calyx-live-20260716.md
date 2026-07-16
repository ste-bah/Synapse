# Issue #1692 daemon Calyx live manual FSV - 2026-07-16

## Scope

Issue #1692 added the async facade that lets Synapse use the synchronous Calyx
`AsterVault` from Tokio without blocking executor workers. The 2026-07-15 FSV
verified the library slice only, because the live daemon did not yet expose a
Calyx storage backend. Issue #1656 has since exposed `--storage-backend calyx`
through the real daemon, so this FSV verifies the remaining acceptance:

- real `synapse-mcp` daemon on the configured Codex endpoint;
- real wired `mcp__synapse` client schema/tool parity;
- Calyx backend writes through MCP `tools/call`, not helper binaries;
- live capture through the same daemon;
- force-kill and restart on the same Calyx vault;
- acknowledged rows recovered byte-for-byte from the physical Calyx Source of
  Truth.

No tests, benchmarks, scripts, CI, GitHub Actions, or automated FSV harnesses
were used. Structural checks were not run because this commit is documentation
only.

## Root Cause

The original root cause was a sync/async impedance mismatch. Synapse is a
Tokio-based daemon, while Calyx `AsterVault` performs synchronous file, WAL, and
vault ownership work. Calling that synchronous API directly from async tasks
would risk blocking runtime workers, would not give bounded admission control
for capture-rate writes, and would make crash/restart ordering hard to audit.

The implemented fix remains the robust path:

- one dedicated owner thread holds the synchronous vault;
- async callers submit commands through a bounded Tokio `mpsc` queue;
- oneshot replies resolve only after the vault commit path has completed;
- unsupported behavior fails explicitly instead of falling back;
- after #1656, the live daemon can route ordinary Synapse CF_KV rows through
  the Calyx backend.

This FSV proves the daemon path now uses that storage surface correctly.

## Research

Research was done after the root problem was identified, using Exa MCP plus
native web search. Primary sources used:

- Tokio `spawn_blocking` docs:
  https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html
- Tokio bounded `mpsc` docs:
  https://docs.rs/tokio/latest/tokio/sync/mpsc/index.html
- Tokio channel tutorial:
  https://tokio.rs/tokio/tutorial/channels
- RocksDB WAL wiki:
  https://github.com/facebook/rocksdb/wiki/Write-Ahead-Log-%28WAL%29
- RocksDB WAL performance wiki:
  https://github.com/facebook/rocksdb/wiki/WAL-Performance
- RocksDB lost buffered write recovery note:
  https://rocksdb.org/blog/2022/10/05/lost-buffered-write-recovery.html

Implementation takeaways:

- A bounded channel is the right admission-control point for async producers.
- Synchronous/blocking storage ownership should be isolated instead of run on
  Tokio executor workers.
- Durable success must be based on separate storage readback, not the write
  return value alone.
- Crash recovery must prove acknowledged rows in the physical WAL/SST Source of
  Truth after process death and restart.

## Source of Truth

Daemon and client precondition SoTs:

- configured endpoint: `http://127.0.0.1:7700/mcp`;
- process table: `synapse-mcp.exe` path and command line;
- socket table: `127.0.0.1:7700` listener owner;
- wired client: `mcp__synapse.health` and validated `tools/list` surface.

Storage SoTs:

- isolated Calyx DB:
  `C:\Users\hotra\AppData\Local\synapse\fsv\issue-1692-calyx-live-20260716-001\db`;
- exact workspace rows in `CF_KV`;
- physical Calyx files under `db\cf\kv` and `db\wal`;
- capture artifact:
  `C:\Users\hotra\AppData\Local\synapse\fsv\issue-1692-calyx-live-20260716-001\capture-before-crash.png`.

Synthetic run id: `issue1692-calyx-live-20260716`.

## MCP Precondition

Normal daemon before the Calyx swap:

- scheduled task `SynapseMcpDaemon`: `Ready` after controlled stop;
- old supervisor PID `57088` and daemon PID `73492` exited after authenticated
  `POST /shutdown`;
- after shutdown, `127.0.0.1:7700` had no live listener, only ownerless
  `TIME_WAIT` rows.

Isolated Calyx daemon:

- start command:
  `C:\Users\hotra\.cargo\bin\synapse-mcp.exe --mode http --bind 127.0.0.1:7700 --db C:\Users\hotra\AppData\Local\synapse\fsv\issue-1692-calyx-live-20260716-001\db --storage-backend calyx --profile-dir C:\Users\hotra\.cargo\bin\profiles --log-level info`;
- PID: `76976`;
- socket SoT: `127.0.0.1:7700 Listen OwningProcess=76976`;
- wired MCP health:
  - `ok=true`;
  - `pid=76976`;
  - `tool_count=40`;
  - `tool_surface_sha256=7cc1d191749041ce3f0bb1a646d1f2b8553e6155e8a9f917553fd66c4cc257ad`;
  - `storage_backend=calyx`;
  - `db_path=C:\Users\hotra\AppData\Local\synapse\fsv\issue-1692-calyx-live-20260716-001\db`.

Initial run state:

- `workspace.list` for prefix `issue1692/live/` returned `returned_count=0`,
  `scanned_rows=0`.
- `storage.summary` reported `storage_backend=calyx`, `CF_KV=2`.
- `rg -a --fixed-strings issue1692-calyx-live-20260716 db` found no synthetic
  marker before the first trigger.

## Manual FSV Evidence

### Happy Path

Before:

- `workspace.list(run_id, prefix=issue1692/live/, include_values=true)` returned
  no entries.

Trigger:

- MCP tool: `mcp__synapse.workspace`;
- operation: `put`;
- key: `issue1692/live/happy`;
- value:
  `{"marker":"issue1692-calyx-live-20260716-happy","equation":"5*5","expected":25,"actual":25}`.

After, separate SoT reads:

- `workspace.put` exact readback:
  - row key
    `workspace-blackboard/v1/run_hex/6973737565313639322d63616c79782d6c6976652d3230323630373136/key_hex/6973737565313639322f6c6976652f6861707079`;
  - `version=1`;
  - `value_len_bytes=601`;
  - `value_sha256=sha256:b3b0df731e08a2c366890ec8345803734574a4692b19407bbe67808c66e26928`;
  - `event_seq=1`.
- `workspace.get` returned `found=true`, `actual=25`, `expected=25`, and the
  same row hash.
- `workspace.list` returned one row for the prefix with the same hash.
- Physical byte scan found `issue1692-calyx-live-20260716-happy` in both
  `db\cf\kv\*.sst` and `db\wal\00000000000000000000.wal`.

### Edge Case 1 - Empty Value

Expected: the key is absent before the trigger and present afterward with
`value=""`, not with null, an omitted value, or generated substitute content.

Before:

- `workspace.get(absent_ok=true)` for `issue1692/live/empty` returned
  `found=false`, `exact_match_count=0`.
- `storage.summary` after the happy row reported `CF_KV=3`.

Trigger:

- MCP tool: `mcp__synapse.workspace`;
- operation: `put`;
- key: `issue1692/live/empty`;
- value: empty string `""`.

After:

- `workspace.get` returned `found=true`, `value=""`, `version=1`.
- Exact row readback:
  - `value_len_bytes=447`;
  - `value_sha256=sha256:8dff02b2c48352332a27b9cb487158f35ae15bd4e3fc1896dc540353f9a5a14d`.
- `storage.summary` reported `CF_KV=4`.
- Physical `db\cf\kv` scan found `issue1692/live/empty` and JSON
  `"value":""`.

### Edge Case 2 - Maximum Valid TTL Boundary

Expected: the maximum valid workspace TTL, `604800000` ms, is accepted and
stored exactly. The invariant is
`expires_at_unix_ms - created_at_unix_ms = 604800000`.

Before:

- `workspace.get(absent_ok=true)` for `issue1692/live/max-ttl-boundary`
  returned `found=false`, `exact_match_count=0`.
- `storage.summary` before trigger reported `CF_KV=4`.

Trigger:

- MCP tool: `mcp__synapse.workspace`;
- operation: `put`;
- key: `issue1692/live/max-ttl-boundary`;
- `ttl_ms=604800000`;
- value marker:
  `issue1692-calyx-live-20260716-max-ttl-boundary`.

After:

- `workspace.put` returned:
  - `created_at_unix_ms=1784171455984`;
  - `expires_at_unix_ms=1784776255984`;
  - delta `604800000`;
  - `value_len_bytes=670`;
  - `value_sha256=sha256:1322b7db91fca5969e2d87c3c52603e96840a096e5d5d724d8447bb7dbb34ced`;
  - `event_seq=3`.
- `workspace.get` returned `ttl_ms=604800000`, the same marker, and the same
  hash.
- Physical `db\cf\kv` scan found the marker and `"ttl_ms":604800000`.

### Edge Case 3 - Expected Absence

Expected: an expected-absence poll reports absence explicitly and does not
create a workspace row.

Before:

- `storage.summary` reported `CF_KV=5` after the three intentional synthetic
  workspace rows. Later aggregate CF_KV counts changed because live MCP calls
  also create session/tool rows in the same CF; exact row readback is the
  verdict for this case.

Trigger:

- MCP tool: `mcp__synapse.workspace`;
- operation: `get`;
- key: `issue1692/live/absent`;
- `absent_ok=true`.

After:

- `workspace.get` returned `found=false`, `exact_match_count=0`.
- `workspace.list(prefix=issue1692/live/, include_values=false)` returned
  exactly the three intentional keys:
  - `issue1692/live/empty`;
  - `issue1692/live/happy`;
  - `issue1692/live/max-ttl-boundary`.
- Physical `db\cf\kv` scan found no `issue1692/live/absent` bytes and no
  absent row-key hex.

### Edge Case 4 - Structurally Invalid Operation Payload

Expected: a mismatched facade payload fails closed before mutation.

Before:

- `workspace.get(absent_ok=true)` for `issue1692/live/invalid-mismatch`
  returned `found=false`, `exact_match_count=0`.

Trigger:

- MCP tool: `mcp__synapse.workspace`;
- declared `operation=put`;
- provided both `put` and `get` operation specs.

Observed error:

- `TOOL_PARAMS_INVALID`;
- message:
  `workspace operation=put received invalid operation specs ["get", "put"]`;
- remediation:
  `pass exactly one operation-specific spec matching put`;
- source of truth:
  `typed workspace facade params before delegated workspace operation`.

After:

- `workspace.get(absent_ok=true)` for `issue1692/live/invalid-mismatch`
  still returned `found=false`, `exact_match_count=0`.
- Physical `db\cf\kv` scan found no
  `issue1692-calyx-live-20260716-invalid-should-not-persist` marker and no
  `issue1692/live/invalid-mismatch` key.

### Live Capture

Before:

- capture artifact path was absent:
  `C:\Users\hotra\AppData\Local\synapse\fsv\issue-1692-calyx-live-20260716-001\capture-before-crash.png`.

Trigger:

- MCP tool: `mcp__synapse.screenshot`;
- operation: `capture`;
- output path above;
- `max_long_edge=800`, `max_pixels=640000`.

After:

- MCP readback:
  - `bytes_written=309580`;
  - `bitmap_sha256=3f2c964d502a034002de7cc513352018744077fa58800a4b93889f776caf9400`;
  - `capture_backend=gdi_screen_region_bgra`;
  - output dimensions `800x556`.
- Separate filesystem read:
  - file length `309580`;
  - `Get-FileHash SHA256` returned
    `3f2c964d502a034002de7cc513352018744077fa58800a4b93889f776caf9400`.

## Crash and Restart Recovery

Pre-crash readback:

- `workspace.list(include_values=true)` returned the three acknowledged rows.
- Hashes before crash:
  - empty:
    `sha256:8dff02b2c48352332a27b9cb487158f35ae15bd4e3fc1896dc540353f9a5a14d`;
  - happy:
    `sha256:b3b0df731e08a2c366890ec8345803734574a4692b19407bbe67808c66e26928`;
  - max TTL:
    `sha256:1322b7db91fca5969e2d87c3c52603e96840a096e5d5d724d8447bb7dbb34ced`.

Crash trigger:

- verified target PID `76976`;
- verified process name `synapse-mcp.exe`;
- verified path `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`;
- verified command line contained the isolated Calyx DB and
  `--storage-backend calyx`;
- force-stopped only PID `76976`.

After crash:

- process table: no `synapse-mcp.exe` PID `76976`;
- socket table: no live `Listen` row on `127.0.0.1:7700`, only ownerless
  `TIME_WAIT` rows.

Restart trigger:

- started the same repo-built binary on the same DB and endpoint;
- new PID: `77764`;
- socket SoT: `127.0.0.1:7700 Listen OwningProcess=77764`;
- MCP health after restart:
  - `ok=true`;
  - `pid=77764`;
  - `storage_backend=calyx`;
  - same isolated DB path;
  - `tool_count=40`;
  - same tool surface hash.

Post-restart readback:

- `workspace.get(issue1692/live/happy)` returned the original marker,
  `actual=25`, `expected=25`, `value_len_bytes=601`, and the same
  `b3b0df...` hash.
- `workspace.get(issue1692/live/empty)` returned `value=""`,
  `value_len_bytes=447`, and the same `8dff02...` hash.
- `workspace.get(issue1692/live/max-ttl-boundary)` returned
  `ttl_ms=604800000`, the original marker, `value_len_bytes=670`, and the same
  `1322b7...` hash.
- `workspace.get(issue1692/live/absent, absent_ok=true)` still returned
  `found=false`, `exact_match_count=0`.
- `workspace.get(issue1692/live/invalid-mismatch, absent_ok=true)` still
  returned `found=false`, `exact_match_count=0`.
- Physical `db\cf\kv` scan after restart:
  - found the happy marker;
  - found the empty key and `"value":""`;
  - found the max-TTL marker and `"ttl_ms":604800000`;
  - did not find the invalid marker;
  - did not find the absent key.

## Host Restoration

The isolated Calyx daemon was stopped through authenticated `POST /shutdown`.

- shutdown response:
  - `ok=true`;
  - `pid=77764`;
  - `shutdown=requested`.
- after shutdown, PID `77764` was gone and `127.0.0.1:7700` had no live
  listener.

The normal scheduled daemon was restarted:

- scheduled task `SynapseMcpDaemon`: `Running`;
- launcher PID `78176` (`wscript.exe`);
- supervisor PID `75216`
  (`C:\Users\hotra\AppData\Local\synapse\logs\synapse-daemon-supervisor.ps1`);
- daemon PID `77508`;
- daemon command line:
  `--mode http --bind 127.0.0.1:7700 --db C:\Users\hotra\AppData\Local\synapse\db-daemon --profile-dir C:\Users\hotra\.cargo\bin\profiles --log-level info`;
- socket SoT:
  `127.0.0.1:7700 Listen OwningProcess=77508`.

Final wired MCP health:

- `ok=true`;
- `pid=77508`;
- `storage_backend=rocksdb`;
- `db_path=C:\Users\hotra\AppData\Local\synapse\db-daemon`;
- `tool_count=40`;
- `tool_surface_sha256=7cc1d191749041ce3f0bb1a646d1f2b8553e6155e8a9f917553fd66c4cc257ad`.

## Verdict

Issue #1692 is accepted at daemon level. The async Calyx vault facade and Calyx
backend survived real MCP-triggered writes, empty values, maximum TTL boundary,
expected absence, invalid facade payloads, live capture, force-kill, restart,
and physical Calyx file readbacks without a workaround or fallback. All
acknowledged workspace rows were present after restart with the same byte
lengths and hashes; invalid and absent rows remained absent.
