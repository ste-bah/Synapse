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
baseline epoch/seq when one exists. The triggers are the real
`reality_baseline`, `observe_delta`, and `reality_audit` MCP tools. The
after-read must compare the emitted baseline/delta/audit row with the physical
UI/log/file/process/storage/device state that actually changed.

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

## 3. Delta-Reality Manual FSV Runbook (#542)

This section is the manual runbook for #536 reality streams. It is not a
script, harness, benchmark, or CI job. Agents execute each step deliberately,
record before/after state, and inspect the separate physical SoT named for that
step.

### 3.1 Source Of Truth Table

| Surface | Physical SoT to read before and after |
|---|---|
| Baseline row | `CF_KV/reality/baseline/v1/<profile>/<epoch>`, `CF_KV/reality/head/v1/<profile>`, and DB WAL/SST bytes containing the row key, epoch, baseline seq, head seq, compact hash, source refs, and redaction summary |
| Delta row | `CF_KV/reality/delta/v1/<profile>/<epoch>/<seq>`, the head row's `head_seq`, and the physical source ref that changed: UI element, EQ log offset, process/window state, clipboard/fs/event row, or action audit row |
| Audit row | `CF_KV/reality/audit/v1/<profile>/<audit_id>`, drift items, `compared_seq_end`, `drift_status`, `rebase_required`, and physical source refs for the audit's re-read |
| Storage summary | real MCP `storage_inspect` row counts/samples for `CF_KV`, `CF_OBSERVATIONS`, `CF_EVENTS`, `CF_ACTION_LOG`, and `CF_SESSIONS`; use separate DB/WAL file reads when the exact row key must be proven |
| Physical reality | process table/window foreground, local log file length/tail/offsets, game map files, UI/HUD readback, chat-input state row, action audit rows, device state, and any external file/database/device the tool claims to observe |

The MCP return value is the attempt. The SoT table above is the verdict.

### 3.2 Happy Path

1. Read runtime preflight: process/socket or stdio child, unauth fail-closed,
   authenticated `/health`, initialized MCP session, and `tools/list` containing
   `reality_baseline`, `observe_delta`, `reality_audit`, and any consumer tool
   being verified.
2. Read the physical before-state: foreground process/window, relevant file
   length/tail, current storage row counts, and any domain SoT such as EQ map or
   current-state rows.
3. Trigger `tools/call reality_baseline` with a synthetic epoch id whose
   expected profile and physical source refs are known.
4. Separately read `CF_KV`/DB bytes for the baseline row and head row. Confirm
   the epoch, baseline seq, head seq, compact hash, source refs, and redaction
   summary.
5. Create or observe one known safe change. For EverQuest this can be a log
   cursor/runtime event movement while the operator is attending the game; for
   non-game surfaces use a reversible local UI/file/process/action change.
6. Trigger `tools/call observe_delta` from the known epoch/seq.
7. Separately read the delta row, updated head row, and the physical source ref
   named by the delta. Confirm kind/path/seq and the expected before/after
   value class without storing raw private bodies.
8. Trigger `tools/call reality_audit` with the current compact state hash or no
   forced mismatch.
9. Separately read the audit row. Expected no-drift state is
   `drift_status=in_sync`, `rebase_required=false`, `compared_seq_end` covering
   the head seq, and physical source refs matching the re-read surfaces.
10. Trigger the consumer tool, such as `everquest_world_summary`, then read its
    persisted row. Expected consumer rows must carry baseline epoch/seq, newest
    delta seq, audit status, drift severity, source refs, and safe next probe.

### 3.3 Required Delta Edges

| Edge | Manual trigger | Expected SoT |
|---|---|---|
| No change | Capture a baseline, avoid any relevant physical change, call `observe_delta` from the head seq | No new delta row or an explicit no-change reason; head seq does not advance; physical SoT read shows unchanged source state |
| Invalid cursor / stale epoch | Call `observe_delta` with a future seq, unknown epoch, or epoch that is no longer the head | Structured MCP error or rebase guidance; no blind delta consumption; head row remains authoritative |
| Dropped or overflowed delta | Drive a change set that would exceed the compact delta budget, or inspect a head that points at an absent newest delta row | `rebase_required` or `delta_snapshot_budget_exceeded`; consumer rows block with `run_reality_audit_then_capture_reality_baseline` or equivalent repair probe |
| Deliberate drift | Call `reality_audit` with a synthetic wrong `assumption_hash` after a known baseline/delta sequence | Audit row has drift item(s), `drift_status` not `in_sync`, `rebase_required=true` when severity requires it; consumers block movement/combat until repair/rebase |

### 3.4 EverQuest-Specific Readback

EverQuest FSV must use the same runtime path above and additionally read:

- EQ process/window: `eqgame.exe` PID, foreground window title/hash, and profile
  `everquest.live` in the baseline/audit source refs.
- EQ log file: `Logs/eqlog_<character>_<server>.txt` length before/after, tail
  lines, and cursor offsets in `game_log` source refs. Confirm delta paths such
  as `/events/log_cursor` or `/events/runtime` against the actual file length.
- Zone/location: persisted `everquest_current_state` or synthetic override row,
  local map file path and line for known landmarks/exits, and consumer row
  source refs.
- HUD level/XP: UI/HUD fields or explicit HUD extraction errors in the
  observation/audit source refs; never treat missing HUD as proof of level.
- Chat input: `everquest_chat_input_state` or equivalent row before/after when
  movement/action tests depend on key focus. If chat is focused, movement keys
  type into chat and must not be treated as movement.
- Action audit deltas: `CF_ACTION_LOG` rows for approved action tools and any
  resulting `CF_EVENTS`/`CF_OBSERVATIONS` rows. Confirm the expected action
  result at the game/log/UI SoT, not just the action return.

### 3.5 Evidence To Post

Issue evidence must include the daemon PID/bind or stdio child, MCP session id,
tool names, exact row keys, physical file/process/window paths, expected output,
actual SoT readback, and at least the happy path plus the four edge cases above.
Supporting checks can be listed after the manual evidence, but they must not be
called FSV.

## 4. Transport Required Edge Audit

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
