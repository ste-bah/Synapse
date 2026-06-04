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
  processes are temporary. Close them after the check and reread the
  process/socket table so only the intended daemon and requested user-facing
  apps remain.

## Health

`GET http://127.0.0.1:7700/health` (with `Authorization: Bearer <token>`) returns
`{ ok, version, pid, uptime_s, subsystems }`. `subsystems.storage.db_path` and
`subsystems.http.active_sessions` confirm which DB the daemon owns and how many
clients are connected.

## Troubleshooting

- **Codex `mcp__synapse` says `Transport closed`** — treat this as configured
  host setup work. Confirm the daemon process and `127.0.0.1:7700` socket, run
  the configured bridge command
  `C:\Users\hotra\.cargo\bin\synapse-mcp.exe --mode connect --bind 127.0.0.1:7700`
  against the repo-built daemon, re-read health and tool discovery, and retry
  the real wired `mcp__synapse` tools. Direct HTTP or standalone stdio probes
  are diagnostics only; do not use them as FSV substitutes. If those host SoTs
  are healthy but the already-running Codex process still has no
  `synapse-mcp --mode connect` child and the live tool remains closed, check
  the installed Codex command surface for a real MCP reload/reconnect action.
  When none exists, start a fresh Codex session so MCP initializes again; do not
  claim the current chat's direct `mcp__synapse` FSV is available until the live
  namespace itself succeeds.
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
  become healthy within the spawn timeout. Check the daemon log under
  `%LOCALAPPDATA%\synapse\logs` for `STORAGE_*` errors.
