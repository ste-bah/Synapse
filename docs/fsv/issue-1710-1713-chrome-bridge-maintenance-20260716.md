# Issues #1710-#1713 and #1548 FSV - Chrome Bridge Maintenance Reconnect

Date: 2026-07-16

## Issues Covered

- #1710: Chrome bridge alarm reconnect did not recover after daemon restart.
- #1711: setup binary handoff raced scheduled-task auto-restart and left the installed exe locked.
- #1712: Chrome debugger installer failed in Windows PowerShell on inline `out uint` C#.
- #1713: foreground authority cleanup treated an expired retained session lease as a failed cleanup.
- #1548: Chrome bridge build skew could block normal `browser_tabs` after reload.

## Root Causes

1. The extension persisted the maintenance reconnect pause only in service-worker globals. Manifest V3 service workers are terminated and restarted, so the pause and reconnect plan could be lost or misread after a daemon handoff.
2. The alarm wake path waited for the full maintenance pause to expire. During setup this meant the daemon could be healthy while Chrome stayed disconnected for most of the 12-minute pause.
3. The native register/probe route rejected extension preflights or missing-origin extension fetches before bridge-token auth could run, so an otherwise valid reconnect probe could fail at HTTP/CORS.
4. Setup let the scheduled task/supervisor relaunch the installed daemon while the installer was replacing `synapse-mcp.exe`, causing an executable lock race.
5. `install-synapse-chrome-debugger.ps1` used inline C# `out uint` declarations that Windows PowerShell's hosted compiler rejected.
6. `act operation=foreground` expected a pre-owned session lease to remain owned after cleanup. If the lease naturally expired during the foreground operation, cleanup restored profile state but failed the lease postcondition and could drain the daemon.
7. Health compact output could report `reason=self_entry_missing` even when the Chrome bridge was healthy, making bridge diagnosis misleading.

## Research Used

Chrome's MV3 guidance says extension service workers are ephemeral, globals are lost after shutdown, alarms can wake the worker, important alarms should be recreated, and state should be persisted with `chrome.storage`. Chrome 120 permits 30-second alarms, and Chrome 116+ WebSocket traffic can reset the worker idle timer. The implementation follows those constraints by persisting the maintenance-pause SoT in `chrome.storage.session`, using an alarm as the wake trigger, and probing the daemon after the bounded critical section rather than keeping the worker alive indefinitely.

Sources:

- https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle
- https://developer.chrome.com/docs/extensions/reference/api/alarms
- https://developer.chrome.com/docs/extensions/reference/api/storage
- https://developer.chrome.com/docs/extensions/develop/migrate/to-service-workers
- https://developer.chrome.com/docs/extensions/how-to/web-platform/websockets

## Fix

- Persisted maintenance pause state in `chrome.storage.session` with pause deadline, bounded resume-probe deadline, paused daemon pid, and paused daemon instance id.
- Added `/chrome-debugger/native/reconnect-probe`, strict register-token auth, and exact daemon identity readback.
- Added direct-extension CORS handling for register/probe preflight and actual requests; invalid origins still fail closed.
- Changed alarm wake handling to probe for a replacement daemon after 30 seconds, clear stale websocket/token state only after strict storage readback, then reconnect.
- Made setup pass `resume_probe_after_ms`, suspend the scheduled task during install handoff, drain/adopt old daemon children, and verify the installed exe can be exclusively opened before replace.
- Made setup's Chrome-bridge post-start verifier wait for bounded alarm reconnect and fail with `SYNAPSE_CHROME_BRIDGE_ALARM_RECONNECT_TIMEOUT` instead of hiding failures with UI repair.
- Added deployed-worker register-token readback to the Chrome debugger installer; build id/hash alone is not enough.
- Rewrote the inline C# `out uint` declarations to old syntax accepted by Windows PowerShell.
- Taught foreground cleanup that a retained session lease that is no longer owned in memory is an expired/released state. It now removes the durable CF_SESSIONS lease row, verifies absence, and treats "session does not own lease" as the correct cleanup state.
- Removed the misleading compact-health bridge reason when tab control is actually available.

## Source Of Truth

- Daemon process/socket: Windows process table plus `Get-NetTCPConnection` for `127.0.0.1:7700`.
- Installed binary: `C:\Users\hotra\.cargo\bin\synapse-mcp.exe` SHA-256.
- Scheduled daemon: Windows Scheduled Task `SynapseMcpDaemon`.
- MCP tool surface: real `mcp__synapse.health` and real `mcp__synapse.browser_tabs`.
- Chrome bridge: health `subsystems.chrome_bridge`, deployed unpacked extension worker bytes, worker SHA-256, and injected register-token length/hash.
- Setup handoff: `%LOCALAPPDATA%\synapse\codex-restart-handoffs` and `STATE\RECOVERY_NOTES.md`.
- Foreground authority cleanup: `act operation=lease_status`, `profile operation=status` CF_SESSIONS row, `CF_ACTION_LOG` audit rows, daemon liveness, and shell job stdout/stderr/status artifacts.

## Happy Path FSV

### Maintenance Restart And Alarm Reconnect

Before:

```text
installed_hash=591BAFE13E1669AF3179DA2909910C64B73D3A855572A73B89862DD3EFB8FC7E
daemon_pid=76476
listener=127.0.0.1:7700 owner=76476
scheduled_task=SynapseMcpDaemon state=4
chrome_bridge.status=ok tab_control_available=true host_count=1 stale=false
```

Trigger:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\synapse-setup.ps1 -SourceDir C:\code\Synapse -ForceRestart
```

Observed setup state:

```text
candidate_hash=1EB9479183CF15DAC8C907AA7B436C984E2B3E735FE1655205FB9C18F4CCF403
maintenance_pause_ack host=chrome-native-0-1784221878951 paused_pid=76476 paused_instance=2554fcfc-fff2-408d-8898-7cdfb83be9c1
pause_ms=720000 resume_probe_after_ms=30000 resume_probe_after_unix_ms=1784222876054
exclusive-open verified installed path before replace
new_daemon_pid=28212
Chrome bridge OK after daemon start wait: stale=false capability=pageScreenshot
expected final setup guard=SYNAPSE_CODEX_CURRENT_PROCESS_SCHEMA_STALE for stale live Codex PID 27984
```

After:

```text
installed_hash=1EB9479183CF15DAC8C907AA7B436C984E2B3E735FE1655205FB9C18F4CCF403
daemon_pid=28212
listener=127.0.0.1:7700 owner=28212
scheduled_task=SynapseMcpDaemon state=4
health.ok=true tool_count=40 tool_surface_sha256=f457205fa2fb76db06c1157808fd54f417ace660f3398df74f80bb3c21383ffe
chrome_bridge.status=ok tab_control_available=true host_count=1 queued=0 pending=0 stale=false
deployed_worker_sha256=A59EE9FDAF1508CDB3FDD3D8F2965C1E0260C3CFCE4647A6E5D691D723C4B729
bridge_register_token_length=64
bridge_register_token_sha256=c4e3db1041c4c2e3d0858f86f6823969f0c981208591eb7f5f63d72d490120c9
```

Real MCP trigger/readback:

```text
mcp__synapse.health detail=compact -> ok=true pid=28212 chrome_bridge.status=ok
mcp__synapse.browser_tabs operation=list -> source_of_truth="chrome.tabs.query via normal Synapse Chrome bridge", returned 14 tabs
```

### Windows PowerShell Installer Compile Path

Trigger:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\install-synapse-chrome-debugger.ps1 -ReloadExistingExtensionViaUi -AutoInstallTimeoutSeconds 90
```

After:

```text
exit_code=0
synapse_chrome_auto_install.attempted=true
synapse_chrome_auto_install.changed=true
reason=existing_ready_extension_ui_reload_invoked
required_foreground=true
chrome_window_hwnd=1050546
chrome_window_pid=11008
no C# compiler error
post_reload_health.chrome_bridge.status=ok
post_reload_worker_sha256=A59EE9FDAF1508CDB3FDD3D8F2965C1E0260C3CFCE4647A6E5D691D723C4B729
```

### Foreground Retained-Lease Cleanup

Before:

```text
mcp__synapse.health -> ok=true pid=28212 daemon_drain.status=ok chrome_bridge.status=ok
mcp__synapse.act operation=lease_status -> held=false is_owner=false owner_session_id=null
```

Trigger:

```json
{"operation":"lease_acquire","ttl_ms":30000}
```

Readback before foreground:

```json
{"held":true,"is_owner":true,"owner_session_id":"855ea493-3e75-4e4b-a384-b1f3e13aebf6","ttl_ms":30000}
```

Foreground trigger:

```json
{
  "operation": "foreground",
  "reason": "issue-1713 manual FSV retained 30s pre-lease renewed to 1s then expires during foreground cleanup",
  "ttl_ms": 1000,
  "action": {
    "verb": "run_shell",
    "command": "powershell.exe",
    "args": ["-NoProfile", "-Command", "Start-Sleep -Milliseconds 1500; [Console]::Out.Write('issue1713-retained-renewed-expired-ok')"],
    "working_dir": "C:\\code\\Synapse",
    "timeout_ms": 5000
  }
}
```

After:

```text
foreground.acquired_lease=false
foreground.released_lease=false
foreground.profile_restored=true
foreground.lease_cleanup_verified=true
foreground.session_holds_lease_after=false
shell_job=019f6c01-2530-7ad2-9b21-d88789f0cd7b
stdout=issue1713-retained-renewed-expired-ok
stderr_len_bytes=0
exit_code=0
mcp__synapse.act operation=lease_status -> held=false is_owner=false owner_session_id=null
mcp__synapse.profile operation=status -> profile=normal_agent CF_SESSIONS key_hex=6d63702f746f6f6c2d70726f66696c652f76312f38353565613439332d336537352d346534622d613338342d623166336531336165626636
CF_ACTION_LOG act foreground final row outcome=ok value_sha256=sha256:15fcbd92f7ca3ed86cc78b7cc1bece276c6fe3b1270bda3fc721d864342ea8ca
daemon_pid=28212 still live, listener=127.0.0.1:7700 owner=28212, daemon_drain.status=ok
```

## Edge Cases

### 1. Valid Extension CORS Preflight With Exact Origin

Before:

```text
daemon_pid=28212 chrome_bridge.status=ok tab_control_available=true host_count=1
```

Trigger:

```text
OPTIONS /chrome-debugger/native/reconnect-probe
Origin: chrome-extension://leoocgnkjnplbfdbklajepahofecgfbk
Access-Control-Request-Method: GET
Access-Control-Request-Headers: x-synapse-bridge-register-token
```

After:

```text
status=204
access-control-allow-origin=chrome-extension://leoocgnkjnplbfdbklajepahofecgfbk
access-control-allow-methods=GET, POST, OPTIONS
access-control-allow-headers=content-type, x-synapse-bridge-token, x-synapse-bridge-register-token
chrome_bridge.status=ok tab_control_available=true host_count=1
```

### 2. Missing Origin Preflight And Invalid Origin Rejection

Before:

```text
daemon_pid=28212 chrome_bridge.status=ok tab_control_available=true host_count=1
```

Trigger A:

```text
OPTIONS /chrome-debugger/native/reconnect-probe
Access-Control-Request-Method: GET
```

After A:

```text
status=204
allow-origin header present
chrome_bridge.status=ok
```

Trigger B:

```text
OPTIONS /chrome-debugger/native/reconnect-probe
Origin: https://evil.invalid
Access-Control-Request-Method: GET
```

After B:

```text
status=403
body=HTTP_ORIGIN_REFUSED
no access-control-allow-origin header
chrome_bridge.status=ok
```

### 3. Register Token And Maintenance Parameter Validation

Before:

```text
daemon_pid=28212 chrome_bridge.status=ok tab_control_available=true host_count=1
```

Missing-token trigger:

```text
GET /chrome-debugger/native/reconnect-probe
```

After:

```text
status=401
body=HTTP_TOKEN_INVALID
chrome_bridge.status=ok
```

Valid-token trigger:

```text
GET /chrome-debugger/native/reconnect-probe
X-Synapse-Bridge-Register-Token: <64-byte deployed token>
```

After:

```json
{
  "ok": true,
  "daemon_pid": 28212,
  "daemon_instance_id": "ebcc81a5-7022-42e5-9c1e-80bf0c3904d8",
  "bridge_protocol_version": 1,
  "expected_extension_id": "leoocgnkjnplbfdbklajepahofecgfbk",
  "expected_extension_build_id": "synapse-chrome-bridge-2026-07-16-maintenance-alarm-resume-v1"
}
```

Invalid maintenance trigger:

```json
{"reason":"edge_invalid_resume","pause_ms":1000,"resume_probe_after_ms":5000}
```

After:

```json
{
  "ok": false,
  "code": "TOOL_PARAMS_INVALID",
  "detail": "chrome bridge maintenance reconnect resume_probe_after_ms must be 1000..=1000, got 5000; remediation=set the resume probe after the shutdown/socket-drain critical section but before the bounded pause expires"
}
```

## Structural Checks

- `node --check extensions\synapse-chrome-debugger\service_worker.js`: passed.
- PowerShell parser for `scripts\install-synapse-chrome-debugger.ps1`: passed.
- PowerShell parser for `scripts\synapse-setup.ps1`: passed.
- `cargo fmt --all --check`: passed.
- `cargo check`: passed.
- `cargo clippy --workspace --all-targets`: passed.
- `git diff --check`: passed, with Git CRLF normalization warnings only for existing script/worker line endings.
- No automated tests, benchmarks, FSV scripts, FSV harnesses, or GitHub Actions were added or run.

## Verdict

#1710, #1711, #1712, #1713, and #1548 are fixed and manually FSV-verified against the real daemon, real Chrome bridge, physical process/socket/file Sources of Truth, CF_ACTION_LOG/profile readbacks, and deployed extension bytes. The only setup failure observed after the daemon handoff is the intentional `SYNAPSE_CODEX_CURRENT_PROCESS_SCHEMA_STALE` guard for the already-running Codex PID 27984; the restart handoff was written under `%LOCALAPPDATA%\synapse\codex-restart-handoffs`.
