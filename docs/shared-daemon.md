# Shared Synapse daemon (one DB, many agents)

Synapse stores perception/action state in a single RocksDB, which permits
**exactly one writer process**. To let many agents (Claude Code, Codex, Claude
Desktop, WSL clients) use Synapse at once without each opening — and fighting
over — that database, Synapse runs as **one shared daemon** that owns the DB,
and every client connects to it.

> Do **not** run multiple `--mode stdio` servers against the same `--db`. The
> second one fails to open RocksDB (`STORAGE_LOCK_CONTENDED`) and, before this
> model existed, such duplicates leaked and accumulated. Use the daemon + the
> client modes below instead.

## Components

| Mode | Command | Role |
|------|---------|------|
| `http` (daemon) | `synapse-mcp --mode http --bind 127.0.0.1:7700 --db <path>` | The single owner of the DB. Serves all clients over Streamable HTTP. |
| `connect` (bridge) | `synapse-mcp --mode connect --bind 127.0.0.1:7700` | Thin stdio↔HTTP bridge for stdio-only MCP clients. Auto-spawns the daemon if it is down. |
| `doctor` | `synapse-mcp --mode doctor [--kill-stray]` | Enumerate/classify synapse-mcp processes; clean up leaked orphans. |

A bearer token is read from `%APPDATA%\synapse\token.txt` (or the
`SYNAPSE_BEARER_TOKEN` env var). Clients and the bridge authenticate with it.

## How each client connects

- **Claude Code and Codex** speak Streamable HTTP natively — point the
  `synapse` MCP entry at `http://127.0.0.1:7700/mcp` with the bearer token
  (transport `http` / `streamable_http`).
- **Claude Desktop on Windows** is a stdio client — launch `synapse-mcp --mode
  connect --bind 127.0.0.1:7700`. The bridge forwards JSON-RPC to the daemon
  and streams notifications back; it auto-spawns the daemon on first use and
  forwards its `--db` to the spawned daemon.
- **WSL agents must not launch the Windows `.exe` bridge directly.** Direct
  WSL interop makes the Windows parent look like long-lived `wsl.exe`, not the
  real Linux-side MCP client, so the bridge cannot prove client lifetime. WSL
  Codex/Claude clients should use native Streamable HTTP
  `http://127.0.0.1:7700/mcp` with the local bearer token. A direct WSL
  `--mode connect` launch fails closed with `MCP_CONNECT_UNSUPPORTED_PARENT`.

All clients share one daemon, so they observe the same live world state and
receive the same `subscribe` events.

## Single-instance guarantee

The daemon takes an OS advisory lock on `<db>/daemon.lock` at startup (before
binding the port or opening RocksDB) and records its PID in `<db>/daemon.pid`.
A second daemon for the same DB logs `MCP_DAEMON_ALREADY_RUNNING` (naming the
holder PID) and exits **3**. The lock is released automatically if the holder
dies, so a crash never wedges future launches. Racing `connect` bridges
therefore converge on exactly one daemon.

Storage is opened **eagerly at startup**: a lock conflict logs
`STORAGE_LOCK_CONTENDED` and any other open failure logs `STORAGE_OPEN_FAILED`;
both exit **4** instead of failing later inside a tool call.

## Lifecycle / no orphans

- The `connect` bridge exits when its stdin closes (client disconnect) and also
  arms a **parent-death watchdog** (`WaitForSingleObject` on the parent) so it
  can never outlive the client even on an abrupt Windows kill.
- The `connect` bridge refuses direct WSL interop parents (`wsl.exe` /
  `wslhost.exe`) because that parent belongs to the WSL host, not the real
  Linux MCP client. This is a hard failure with structured log code
  `MCP_CONNECT_UNSUPPORTED_PARENT`; use HTTP transport from WSL instead.
- The spawned daemon is created with `bInheritHandles=FALSE`, so it never holds
  a client's stdio pipe open.
- The daemon stays resident across individual client disconnects and does **not**
  `release_all` global input when one client leaves (only on full daemon shutdown).
- Diagnostic bridge, Codex app-server, browser, Cargo/rustc, and test helper
  processes are temporary only when the current operation spawned and recorded
  their exact PIDs. Close only those owner-known PIDs after the check, then
  reread the process/socket table so only the intended daemon and requested
  user-facing apps remain. Never close terminal/IDE/WSL host processes
  globally (`cmd.exe`, `powershell.exe`, `pwsh.exe`, `WindowsTerminal.exe`,
  `OpenConsole.exe`, `conhost.exe`, `wsl.exe`, `wslhost.exe`, `Code.exe`):
  they are operator and agent workspaces.
- `scripts/synapse-setup.ps1` refuses to stop/restart the shared daemon while
  bridge/stdio children or live peer-owned TCP client connections are present.
  Raw HTTP session-manager entries are logged as `idle_session_map_entries`,
  but they are not physical proof of a live client by themselves because
  Streamable HTTP clients that crash or exit without `DELETE` can leave idle
  session state until the daemon timeout reaps it. Setup fails closed with
  `SYNAPSE_ACTIVE_CLIENTS_PRESENT` only when the process/socket Source of Truth
  proves an attached client; stale server-side sockets without a peer process
  are printed as `stale_tcp_connections`. Use `-ForceRestart` only after a
  deliberate maintenance decision; setup must not silently interrupt another
  agent.
- When setup does restart the daemon, it stops only the verified
  `synapse-mcp.exe` daemon PID. The normal stop path sends an authenticated
  `POST /shutdown` request to the loopback daemon, verifies the responding PID
  is one of the verified Synapse PIDs, then waits for that PID to exit through
  the daemon's graceful cancellation path. It must not use process-tree
  termination such as `taskkill /T`, because apps launched through
  `act_launch` can be daemon children and may contain user state. Child app
  cleanup is separate owner-known work: close only exact PIDs that the current
  verification spawned and recorded, and leave uncertain user-facing windows
  running.
- `scripts/synapse-setup.ps1` also takes an exclusive
  `%LOCALAPPDATA%\synapse\setup-maintenance.lock.json` file handle before
  setup/remove. This serializes multi-agent install/restart work. The lock file
  records the owning PID, command line, lineage, reason, and cleanup policy for
  physical readback; the OS releases the lock when the setup process exits.
  If another setup owns the lock, the script fails closed with
  `SYNAPSE_SETUP_MAINTENANCE_LOCK_HELD` and explicitly forbids clearing it by
  closing terminal windows.
- The daemon's default Streamable HTTP session idle timeout is 5 minutes
  (`SYNAPSE_HTTP_SESSION_IDLE_TIMEOUT_SECS` overrides it). This bounds zombie
  session memory and stale client accounting when a client disappears without
  sending MCP `DELETE`.
- The Windows auto-start daemon task must not launch through `cmd.exe` at any
  point. `scripts/synapse-setup.ps1` writes
  `synapse-daemon-launch-hidden.vbs` and registers
  `wscript.exe //B //Nologo`; the VBS reads `%APPDATA%\synapse\token.txt`,
  sets process-local auth/log environment, and starts `synapse-mcp.exe`
  directly with hidden window style. The legacy
  `synapse-daemon-launch.cmd` file is deleted so no raw bearer token is
  embedded in a command file. Setup fails with
  `SYNAPSE_DAEMON_CMD_ANCESTOR_FORBIDDEN` if the healthy daemon's process
  lineage still contains `cmd.exe`.

## Durable Shell Jobs

`act_run_shell.timeout_ms` is the direct inline wait budget. When it is greater
than the daemon's inline await limit, the request returns a durable job handle
instead of waiting. That handoff does not copy the inline budget into the
durable job lifetime; the persisted job `status.json` records
`"timeout_ms": null`.

`act_run_shell.durable_timeout_ms` is applied only when that request creates a
durable/background job. If the process completes inline, the durable cap is not
part of the execution plan and the command-audit readback reports
`"durable_timeout_policy": "ignored_inline_execution"`.

`act_run_shell_start.timeout_ms` is different: it is an explicit lifetime cap
for that one durable job. Omit it for an unbounded job that exits normally or is
stopped by `act_run_shell_cancel` or session cleanup. Cancellation and timeout
termination are limited to the recorded job-owned PID/process tree from
`status.json`; terminal/IDE/WSL host processes are never broad cleanup targets.

The action subsystem health payload exposes these policies separately:
`run_shell_inline_await_limit_ms`,
`run_shell_durable_default_timeout_ms`, and
`run_shell_durable_max_timeout_ms`. The durable timeout fields are `null` when
there is no default cap and no configured maximum cap.

## Health

`GET http://127.0.0.1:7700/health` (with `Authorization: Bearer <token>`) returns
`{ ok, version, pid, uptime_s, subsystems }`. `subsystems.storage.db_path`
confirms which DB the daemon owns. `subsystems.http.active_sessions` is the raw
Streamable HTTP session-manager count, not by itself proof of a currently
attached process; pair it with process/socket readback for restart decisions.
HTTP MCP session state is persisted in `CF_KV` under `mcp/session/v1/<session>`.
Rows include `stored_at_unix_ms` and expire on the same idle timeout as the
in-memory session manager (`SYNAPSE_HTTP_SESSION_IDLE_TIMEOUT_SECS`, default
300 seconds). Legacy rows without this TTL metadata are deleted on load and are
not accepted as live session proof.

## Troubleshooting

- **Codex `mcp__synapse` says `Transport closed`** — treat this as configured
  host setup work. Codex should be Streamable HTTP, not stdio: confirm the
  daemon process and `127.0.0.1:7700` socket, confirm
  `SYNAPSE_BEARER_TOKEN` matches `%APPDATA%\synapse\token.txt`, and read
  `codex mcp get synapse`. If it drifted, repair it with
  `codex mcp add synapse --url http://127.0.0.1:7700/mcp --bearer-token-env-var SYNAPSE_BEARER_TOKEN`.
  Re-run `scripts/synapse-setup.ps1` if the standard Windows Codex launchers do
  not contain the Synapse token loader; the setup script patches those launchers
  so future Codex processes load the canonical token before MCP initialization.
  Then re-read health and tool discovery and retry the real wired
  `mcp__synapse` tools. Direct HTTP probes are diagnostics only; do not use
  them as FSV substitutes. If the already-running Codex process was launched
  before `SYNAPSE_BEARER_TOKEN` existed or changed, Windows cannot update that
  process environment after the fact; setup reports
  `SYNAPSE_CODEX_CURRENT_PROCESS_ENV_STALE` and the current chat cannot claim
  direct `mcp__synapse` FSV until a fresh Codex process initializes with the
  token loader.
- **Codex exposes stale `mcp__synapse` tool schemas after setup/restart** —
  setup snapshots the daemon's sanitized `tools/list` at
  `%APPDATA%\synapse\codex-tool-surface.json` and each patched Codex launcher
  copies that file to an immutable per-process start snapshot under
  `%LOCALAPPDATA%\synapse\codex-start-snapshots`. If setup later sees the live
  daemon schema hash differ from `SYNAPSE_TOOL_SURFACE_HASH_AT_CODEX_START`, it
  fails with `SYNAPSE_CODEX_CURRENT_PROCESS_SCHEMA_STALE` and reports
  added/removed/schema-changed tools. Restart Codex through the patched launcher
  so the real wired `mcp__synapse` client loads the new schema. Direct HTTP or
  stdio probes remain diagnostics only and are not D1/client-parity FSV.
- **WSL Codex/Claude leaves `synapse-mcp --mode connect` children under
  `wsl.exe`** — this is a configuration error. Reconfigure the WSL client to
  HTTP transport with bearer auth. The bridge now refuses direct WSL interop
  with `MCP_CONNECT_UNSUPPORTED_PARENT` so this stale-child class cannot recur
  silently.
- **`RegisterHotKey ... Hot key is already registered` / "leaked or duplicate
  synapse-mcp instance"** — more than one synapse-mcp is running. Run
  `synapse-mcp --mode doctor` to list them; `--mode doctor --kill-stray` removes
  matching bridge/stdio/orphan/unknown processes only after it identifies the
  live lock-holder daemon for the selected `--db`.
- **`STORAGE_LOCK_CONTENDED` on daemon start** — another process holds the
  RocksDB lock for that `--db`. `doctor` will name the holder PID; stop it or
  point the daemon at a different `--db`.
- **Bridge exits immediately / `MCP_DAEMON_SPAWN_FAILED`** — the daemon did not
  become healthy within the spawn timeout. Check
  `%LOCALAPPDATA%\synapse\logs\daemon-launcher.log` and the rotated
  `%LOCALAPPDATA%\synapse\logs\synapse.log.*` telemetry files for launch,
  `STORAGE_*`, or bind errors.
- **Setup/update says `SYNAPSE_ACTIVE_CLIENTS_PRESENT`** — setup saw a live
  daemon with bridge/stdio children or a peer-owned loopback client socket.
  Close only the exact owner-known helper PID named in the process/socket SoT,
  rerun setup, and read the process/socket SoT again. Do not close terminal
  windows or broad shell process groups. `idle_session_map_entries` without
  live TCP peers means stale or idle HTTP session state is present but did not
  prove an attached client. If a restart is truly required while clients are
  attached, rerun with `-ForceRestart` only after coordinating a maintenance
  window; this flag exists to make interruption explicit.
- **Setup killed a launched app/window** — this is a setup bug. Setup is only
  allowed to stop verified `synapse-mcp.exe` daemon PIDs, never their child
  process tree. Inspect the process stop log, patch setup before rerunning it,
  and verify that an exact-PID daemon restart leaves a synthetic daemon child
  process alive.
- **Setup/update says `SYNAPSE_SETUP_MAINTENANCE_LOCK_HELD`** — another setup
  or remove operation is already running. Read the lock file owner fields, wait
  for that process to exit, or inspect that exact PID. Do not close terminal
  windows to clear this condition.
