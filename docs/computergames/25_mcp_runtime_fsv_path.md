# 25 - MCP Runtime FSV Path

This runbook defines the reliable Synapse MCP runtime path agents must use when
manual FSV needs a real Synapse tool surface. It is not a script, test,
benchmark harness, CI job, or GitHub Action. Do not automate this runbook and
do not call a return value FSV unless the separate source of truth named here
has been read before and after the trigger.

## 0. Mandatory MCP Runtime Preflight

For Synapse behavior, `synapse-mcp` itself is part of the source-of-truth chain.
Before every manual FSV action cluster, the agent must read and record:

| Surface | Required readback |
|---|---|
| Runtime process | `synapse-mcp.exe` PID, executable path, command line, and start time, or the configured stdio child owned by the MCP client |
| Transport | loopback bind/socket or stdio transport state; unauthenticated HTTP must fail closed |
| Liveness | authenticated `health` response with `ok=true` and relevant subsystem state |
| Session | initialized MCP session id for HTTP, or initialized stdio client state |
| Tool registry | `tools/list` count plus the exact tool name required for the issue |

If any of those reads fail because the daemon is absent, stale, unreachable, or
the direct chat transport is closed, make the runtime real first. Build,
install, or launch the repo-owned `synapse-mcp` process with an issue-local DB
and log directory, then repeat the preflight reads. Missing or closed MCP
runtime state is setup work, not permission to bypass Synapse.

When an MCP tool exists for the behavior under review, the FSV trigger is the
real MCP `tools/call`. Direct CLI calls, helper binaries, unit tests,
benchmarks, scripts, direct RocksDB writes, or code-level invocations may
support diagnosis, but they are not the runtime trigger. After `tools/call`, the
verdict still comes from a separate read of the physical SoT: storage rows,
file bytes, visible UI, local logs, process/window state, device state, or
external record. The tool return value and `health` response prove liveness and
attempt only.

For delta-first reality work (#536), the preflight also records the current
baseline epoch/seq when one exists. The trigger is the real delta or audit MCP
tool once implemented; the after-read must compare the emitted delta/audit row
with the physical UI/log/file/process/storage/device state that actually
changed.

## 1. Direct Chat MCP Boundary

The pre-wired chat MCP tool is a convenience surface owned by the MCP client
process. On this configured host, Codex reads:

```toml
[mcp_servers.synapse]
command = 'C:\Users\hotra\.cargo\bin\synapse-mcp.exe'
args = ["--mode", "stdio"]
```

That client owns the stdio child lifecycle. If the child exits or is killed,
the chat tool may return `Transport closed` and may not respawn until the MCP
client session is restarted. That error is a chat-client transport state, not
proof that the repo-built Synapse runtime is down.

If direct chat MCP reports `Transport closed`, read these physical SoTs:

| Surface | Source of truth |
|---|---|
| Configured binary | `C:\Users\hotra\.cargo\bin\synapse-mcp.exe` size, mtime, SHA256 |
| Running child | `Get-CimInstance Win32_Process -Filter "Name='synapse-mcp.exe'"` |
| Transport log | `%LOCALAPPDATA%\synapse\logs\synapse.log.<date>` |
| Client config | `C:\Users\hotra\.codex\config.toml` |
| Repo runtime | repo-built `synapse-mcp --mode http` or `--mode stdio` process, logs, and DB |

Make missing local state real. After repo changes, install the current runtime
into the configured direct-chat path:

```powershell
cargo install --path crates/synapse-mcp --force
```

If Windows refuses replacement because the old binary is running, first call
`release_all` if the direct tool still responds, then stop only the old
`C:\Users\hotra\.cargo\bin\synapse-mcp.exe` children, rerun the install, and
read the installed file hash. Existing chat sessions can still keep a closed
transport until the MCP client restarts; future sessions will launch the new
binary from the configured path.

## 2. Dedicated Repo Runtime Path

For issue shipping evidence, use a repo-owned runtime process with isolated
state whenever the direct chat child is unavailable or whenever the issue needs
an inspectable process/socket/log/DB source of truth:

```powershell
$env:SYNAPSE_BEARER_TOKEN = 'issue-token'
$env:SYNAPSE_LOG_DIR = '<run-dir>\logs'
$env:SYNAPSE_ALLOW_SHELL = '<exact anchored allowlist when act_run_shell is needed>'
synapse-mcp --mode http --bind 127.0.0.1:<port> --db '<run-dir>\db' --log-level debug
```

Then manually:

1. Read process table, command line, installed binary hash, log file path, DB
   path, and `/health` before the trigger.
2. Initialize `/mcp`, send `notifications/initialized`, read `tools/list`, and
   call the real tool through `tools/call`.
3. Read `/health`, logs, process state, and any DB/file/game SoT after the
   trigger.
4. Stop the temporary daemon before leaving the issue.

Use `/health` as the transport liveness check, but use real MCP `tools/call`
for behavior under review. For storage/profile/audit behavior, the SoT is the
RocksDB column-family readback through `storage_inspect` and, when necessary,
separate file/database inspection.

## 3. Required Edge Audit

Every transport reliability FSV must record before and after state for:

| Edge | Trigger | Expected SoT |
|---|---|---|
| Server absent | Call a known unused loopback port or a closed direct chat child | Connection refused or `Transport closed`; process table shows no listener/child |
| Malformed request | Send malformed JSON or schema-invalid MCP params | Structured HTTP/MCP error; daemon process remains alive |
| Long-running tool | Call an allowlisted long command or long safe action | Duration/readback shows the call completed or timed out as expected; daemon remains alive |
| Runtime panic/early exit | Force a debug panic or terminate a temporary daemon | Log/process state proves the boundary; held input state is neutral or never acquired |

Automated tests, scripts, benchmarks, and GitHub Actions are supporting
evidence only. Manual FSV is the real runtime trigger plus separate source of
truth readback.
