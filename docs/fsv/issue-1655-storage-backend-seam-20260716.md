# Manual FSV - Issue #1655 Storage Backend Seam

Date: 2026-07-16

Issue: #1655 `[CALYX] Storage backend seam behind the synapse_storage::Db facade`

## Scope

Change shipped:
- `synapse_storage::Db` is now a stable facade over an internal storage backend trait.
- `StorageBackendKind` supports `rocksdb` (implemented default) and `calyx` (known but fail-closed until #1656).
- MCP daemon config accepts `--storage-backend` / `SYNAPSE_STORAGE_BACKEND`.
- Health and storage facade readbacks expose `storage_backend`.
- Invalid backend config fails closed with `STORAGE_BACKEND_INVALID_CONFIG`.
- Unimplemented Calyx backend fails closed with `STORAGE_BACKEND_UNIMPLEMENTED`.

Supporting checks only, not FSV:
- `cargo fmt --all --check` passed.
- `cargo check -p synapse-mcp` passed.
- `cargo clippy --workspace --all-targets` passed.
- `cargo build --release -p synapse-mcp` passed after stopping the verified old daemon PID that held the release exe.

## Source of Truth

Primary SoTs:
- Running daemon process table and socket: `synapse-mcp.exe` PID/binary path plus `127.0.0.1:7700` listener.
- Installed daemon binary bytes: `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`.
- Repo-built daemon binary bytes: `C:\code\Synapse\target\release\synapse-mcp.exe`.
- Daemon lifecycle file: `%LOCALAPPDATA%\synapse\db-daemon\daemon-run-current.json`.
- RocksDB physical files in `%LOCALAPPDATA%\synapse\db-daemon`: `CURRENT`, `MANIFEST-*`, `OPTIONS-*`, `LOCK`, `daemon.pid`.
- Daemon stderr/log lines carrying `MCP_CLI_PARSED` and `STORAGE_BACKEND_OPENED`.
- Real wired Codex MCP client readbacks: `mcp__synapse.health`, `mcp__synapse.storage`, `mcp__synapse.setup`.

## Before Read

Before installing the patched daemon:
- Wired MCP `health` reported PID `76992`, tool count `40`, and storage status `ok`, but `subsystems.storage` had no `storage_backend` field.
- Wired MCP `storage operation=summary` returned `source_of_truth="RocksDB CF metadata + exact row readbacks"`, `metrics_mode="rocksdb_live_data_size_estimates_estimated_row_counts"`, and no `summary.storage_backend`.
- Process/socket SoT showed PID `76992` at `C:\code\Synapse\target\release\synapse-mcp.exe`, listening on `127.0.0.1:7700`.
- A later configured-host readback showed an auto-restarted installed daemon PID `30428` at `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`; its installed binary hash differed from the patched repo release hash and its MCP health/storage readbacks still lacked `storage_backend`.

## Trigger

Runtime setup:
- Stopped only verified Synapse PID `76992` after confirming process name/path/command line.
- Built patched release binary.
- Stopped only verified installed Synapse PID `30428` after confirming process name/path/command line.
- Replaced `C:\Users\hotra\.cargo\bin\synapse-mcp.exe` with `C:\code\Synapse\target\release\synapse-mcp.exe`.
- Started installed daemon path with:
  - `--mode http`
  - `--bind 127.0.0.1:7700`
  - `--db %LOCALAPPDATA%\synapse\db-daemon`
  - `--profile-dir %USERPROFILE%\.cargo\bin\profiles`
  - `--log-level info`

MCP trigger:
- Used real wired client `mcp__synapse.health detail=compact`.
- Used real wired client `mcp__synapse.storage operation=summary`.
- Used real wired client `mcp__synapse.setup operation=status`.

## After Read

Physical daemon SoT:
- Process table: PID `38764`, name `synapse-mcp.exe`, executable `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`.
- Socket table: `127.0.0.1:7700` state `Listen`, owner PID `38764`.
- Binary hashes matched:
  - `C:\code\Synapse\target\release\synapse-mcp.exe` SHA-256 `9BBF6ACDAAF5ED9DECAF166758D35FF4A4FAF834AF2E7F58CB5D8C7087E46E88`
  - `C:\Users\hotra\.cargo\bin\synapse-mcp.exe` SHA-256 `9BBF6ACDAAF5ED9DECAF166758D35FF4A4FAF834AF2E7F58CB5D8C7087E46E88`
- `%LOCALAPPDATA%\synapse\db-daemon\daemon-run-current.json` read:
  - `pid: 38764`
  - `mode: "http"`
  - `bind_addr: "127.0.0.1:7700"`
  - `db_path: "C:\\Users\\hotra\\AppData\\Local\\synapse\\db-daemon"`
  - `ended_at_unix_ms: null`
- DB file readback:
  - `CURRENT` length `16`, contents `MANIFEST-668816`
  - `daemon.pid` contents `38764`
  - `LOCK`, `MANIFEST-668816`, and `OPTIONS-*` physically present
- Startup log readback contained:
  - `MCP_CLI_PARSED ... storage_backend: "rocksdb"`
  - `STORAGE_BACKEND_OPENED ... backend="rocksdb" schema_version=1`

MCP client readbacks:
- `mcp__synapse.health` returned `pid=38764`, `tool_count=40`, `subsystems.storage.status="ok"`, `subsystems.storage.storage_backend="rocksdb"`, `schema_version=1`.
- `mcp__synapse.storage operation=summary` returned:
  - `source_of_truth="storage backend CF metadata + exact row readbacks"`
  - `readback_source_of_truth="rocksdb summary cf_count=17 pressure=Normal"`
  - `summary.storage_backend="rocksdb"`
  - `summary.schema_version=1`
  - `summary.metrics_mode="rocksdb_live_data_size_estimates_estimated_row_counts"`
- `mcp__synapse.setup operation=status` returned PID `38764` and daemon run file SHA-256 `sha256:334a24399d0b1224f7ca74b5fd2ac607434dd71fc03915ed8245bb590df0b15d`.

## Edge Cases

### Edge 1 - Structurally Invalid Backend String

Synthetic input:
- `--storage-backend bogus`
- isolated DB: `%LOCALAPPDATA%\synapse\fsv-1655-invalid-b179ebc4f51744779926a812dcbd98d9`
- isolated port: `7781`

Before:
- DB path existed: `False`
- listener on `127.0.0.1:7781`: `False`
- matching process count: `0`

Trigger:
- Launched repo-built daemon with invalid backend string.

After:
- exit code: `2`
- output contained `STORAGE_BACKEND_INVALID_CONFIG`
- output contained `storage_backend must be "rocksdb" or "calyx"; got "bogus"`
- DB path existed: `False`
- listener on `127.0.0.1:7781`: `False`
- matching process count: `0`

### Edge 2 - Known But Unimplemented Calyx Backend

Synthetic input:
- `--storage-backend calyx`
- isolated DB: `%LOCALAPPDATA%\synapse\fsv-1655-calyx-fd1b16a6d21e48eaa4f43e17b59efee7`
- isolated shell-job root: `%LOCALAPPDATA%\synapse\fsv-1655-shell-calyx-fd1b16a6d21e48eaa4f43e17b59efee7`
- isolated port: `7782`

Before:
- DB path existed: `False`
- shell-job root existed: `False`
- listener on `127.0.0.1:7782`: `False`
- matching process count: `0`

Trigger:
- Launched repo-built daemon with `calyx`, isolated DB, and isolated shell-job root so startup reached the storage backend gate.

After:
- exit code: `1`
- output contained `STORAGE_BACKEND_UNIMPLEMENTED`
- output contained `storage_backend="calyx" selected, but the Calyx Db backend is not implemented; finish issue #1656 before selecting this backend`
- output contained `STORAGE_OR_CALYX_OPEN_OR_MAINTENANCE_START_FAILED`
- DB path existed with daemon lifecycle files only: `daemon-exit.jsonl`, `daemon-lifecycle.lock`, `daemon-run-current.json`, `daemon-tool-events.jsonl`, `daemon-tool-last.json`, `daemon.lock`
- no RocksDB `CURRENT`, `LOCK`, `MANIFEST-*`, or `OPTIONS-*` files were created for that DB
- listener on `127.0.0.1:7782`: `False`
- matching process count: `0`

### Edge 3 - Empty CLI Value

Synthetic input:
- `--storage-backend` followed by an empty CLI value
- isolated DB: `%LOCALAPPDATA%\synapse\fsv-1655-empty-5ff4ee30443e497d842417ce1480d6b4`
- isolated shell-job root: `%LOCALAPPDATA%\synapse\fsv-1655-shell-empty-5ff4ee30443e497d842417ce1480d6b4`
- isolated port: `7783`

Before:
- DB path existed: `False`
- shell-job root existed: `False`
- listener on `127.0.0.1:7783`: `False`
- matching process count: `0`

Trigger:
- Launched repo-built daemon with an empty CLI value for `--storage-backend`.

After:
- Clap rejected the input before daemon startup: `a value is required for '--storage-backend <rocksdb|calyx>' but none was supplied`
- DB path existed: `False`
- shell-job root existed: `False`
- listener on `127.0.0.1:7783`: `False`
- matching process count: `0`

### Edge 4 - Uppercase RocksDB Boundary

Synthetic input:
- `--storage-backend ROCKSDB`
- isolated DB: `%LOCALAPPDATA%\synapse\fsv-1655-upper-auth-ccab05626aac40afabd507323c42e548`
- isolated shell-job root: `%LOCALAPPDATA%\synapse\fsv-1655-shell-upper-auth-ccab05626aac40afabd507323c42e548`
- isolated port: `7785`

Before:
- DB path existed: `False`
- shell-job root existed: `False`
- listener on `127.0.0.1:7785`: `False`
- matching process count: `0`

Trigger:
- Launched repo-built daemon with uppercase backend value.

After:
- spawned PID: `61204`
- listener on `127.0.0.1:7785`: owner PID `61204`
- authenticated `/health` read returned `health_pid=61204`, `storage_status=ok`, `storage_backend=rocksdb`, `schema_version=1`, and the isolated DB path
- DB files physically present: `CURRENT`, `LOCK`, `MANIFEST-000005`, `OPTIONS-000007`, `daemon.lock`, `daemon.pid`
- `CURRENT` contained `MANIFEST-000005`
- `daemon.pid` contained `61204`
- logs contained `MCP_CLI_PARSED ... storage_backend: "ROCKSDB"`
- logs contained `STORAGE_BACKEND_OPENED ... backend="rocksdb" schema_version=1`
- cleanup stopped only verified PID `61204`
- listener after stop: `False`
- matching process count after stop: `0`

## Result

Issue #1655 acceptance is manually verified against reality:
- The default daemon runs on the implemented `rocksdb` backend.
- The real MCP health and storage surfaces expose `storage_backend`.
- Physical DB/log/process SoTs agree with the MCP readbacks.
- Invalid backend values fail closed before serving.
- `calyx` fails closed with the intended structured unimplemented-backend error and does not create RocksDB files.
- Case-insensitive `ROCKSDB` normalizes to `rocksdb` and opens a real isolated RocksDB directory.
- Temporary daemon probes were stopped by exact verified PID.
- Cleanup readback removed all `%LOCALAPPDATA%\synapse\fsv-1655-*` temp DB/shell roots and `%LOCALAPPDATA%\synapse\logs\fsv-1655-*` probe logs.
- Final process/socket readback showed only the intended main daemon: PID `38764`, executable `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`, listener `127.0.0.1:7700`.
