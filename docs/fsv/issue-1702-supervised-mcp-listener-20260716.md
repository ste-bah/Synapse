# Issue #1702 Manual FSV: supervised MCP listener recovery

Date: 2026-07-16

## Source of Truth

- Task Scheduler: `SynapseMcpDaemon`
- Process table: `synapse-mcp.exe`, `wscript.exe`, and the hidden `powershell.exe` supervisor
- Socket table: `127.0.0.1:7700` listener owner PID
- Supervisor files:
  - `C:\Users\hotra\AppData\Local\synapse\logs\daemon-supervisor-current.json`
  - `C:\Users\hotra\AppData\Local\synapse\logs\daemon-supervisor-events.jsonl`
  - `C:\Users\hotra\AppData\Local\synapse\logs\daemon-launcher.log`
- Daemon lifecycle file: `C:\Users\hotra\AppData\Local\synapse\db-daemon\daemon-exit.jsonl`
- Real MCP client calls: `mcp__synapse.health` and `mcp__synapse.shell`

## Root Cause

The installed `SynapseMcpDaemon` task launched a one-shot hidden VBScript which ran `synapse-mcp.exe` synchronously and then exited with the daemon exit code. The task was therefore not a resident supervisor. When the daemon disappeared, recovery depended on Task Scheduler's finite restart settings and opaque task state. The task was observed as `Ready`, not `Running`, while the daemon was live, with `LastTaskResult=3`.

The old launcher also did not write a durable supervisor state file, generation number, child PID, or restart event log. The daemon lifecycle file had multiple `previous_run_unclean` rows, including a shell in-flight case, proving prior daemon processes disappeared without graceful exit.

During installation, `scripts\synapse-setup.ps1 -SkipBuild` also forced the binary handoff path even when the verified candidate was the already-installed binary. That unnecessary daemon drain hit the unrelated Chrome bridge maintenance pause path and failed once with `SYNAPSE_CHROME_BRIDGE_MAINTENANCE_PAUSE_HEALTH_FAILED`. Setup now keeps the live daemon for unchanged `-SkipBuild` installs and lets the new supervisor adopt it.

## Research

Exa MCP and native web research used Microsoft primary documentation:

- Microsoft Task Scheduler `RestartOnFailure`: restart behavior is defined by finite `Count` and `Interval`; both must be set.
  `https://learn.microsoft.com/en-us/windows/win32/taskschd/taskschedulerschema-restartonfailure-settingstype-element`
- Microsoft Task Scheduler `MultipleInstancesPolicy`: `IgnoreNew` does not start a new instance if one is already running.
  `https://learn.microsoft.com/en-us/windows/win32/taskschd/taskschedulerschema-multipleinstancespolicy-settingstype-element`
- Microsoft PowerShell `Start-Process`: `-PassThru` returns a process object; `-WindowStyle Hidden` is supported on Windows.
  `https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/start-process`

Design decision from research: keep Task Scheduler as the bootstrapping/autostart surface with `IgnoreNew`, but make the launched process a resident supervisor that owns child PID tracking, restart policy, fail-loud crash-loop detection, and durable state files.

## Implementation

- `New-HiddenDaemonLauncher` now generates:
  - a small hidden VBScript wrapper, and
  - `synapse-daemon-supervisor.ps1`, a resident supervisor.
- The supervisor:
  - validates token, bind format, listener owner, executable path, and DB path;
  - adopts an already-running expected daemon instead of racing the bind;
  - starts replacement daemon children with `Start-Process -PassThru -WindowStyle Hidden`;
  - logs child PID, generation, exit code, runtime, restart delay, and fatal reasons;
  - restarts nonzero child exits with bounded exponential backoff;
  - fails loudly on crash loops, invalid token, invalid bind, or unexpected listener owner.
- `synapse-setup.ps1 -SkipBuild` now detects when the verified candidate binary is already installed and avoids an unnecessary daemon drain/copy. It registers the supervisor task and lets it adopt the live daemon.

## Manual FSV

### Pre-change state

Before setup repair:

```text
TaskState=Ready
LastRunTime=2026-07-15T20:45:10-05:00
LastTaskResult=3
Action=C:\WINDOWS\System32\wscript.exe
Arguments=//B //Nologo "C:\Users\hotra\AppData\Local\synapse\logs\synapse-daemon-launch-hidden.vbs"

Listener=127.0.0.1:7700 owner_pid=52616
Daemon path=C:\Users\hotra\.cargo\bin\synapse-mcp.exe
Daemon db=C:\Users\hotra\AppData\Local\synapse\db-daemon
```

Old launcher tail showed one-shot behavior:

```text
SYNAPSE_DAEMON_LAUNCH_START ...
SYNAPSE_DAEMON_EXIT exit_code=-1
SYNAPSE_DAEMON_LAUNCH_START ...
SYNAPSE_DAEMON_EXIT exit_code=3
```

### Setup trigger and readback

Trigger:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts\synapse-setup.ps1 -SkipBuild -SkipClientWiring -ActiveIssue 1702
```

Setup readback:

```text
SkipBuild candidate is already installed; setup will keep the live daemon and let the generated supervisor adopt it.
Task registered and started.
Daemon OK: pid=52616 version=0.1.0 db=C:\Users\hotra\AppData\Local\synapse\db-daemon
```

Separate SoT read after setup:

```text
TaskState=Running
LastRunTime=2026-07-15T21:57:29-05:00
LastTaskResult=267009

wscript pid=71328
supervisor powershell pid=57088 parent=71328
synapse-mcp pid=52616
listener owner=52616
```

Supervisor state:

```json
{
  "state": "adopted_existing",
  "generation": 0,
  "supervisor_pid": 57088,
  "child_pid": 52616,
  "bind": "127.0.0.1:7700",
  "db_path": "C:\\Users\\hotra\\AppData\\Local\\synapse\\db-daemon"
}
```

Supervisor events:

```json
{"event":"supervisor_start","supervisor_pid":57088,"generation":0}
{"event":"adopt_existing","supervisor_pid":57088,"generation":0,"child_pid":52616}
```

### Happy path

Trigger: real MCP `shell.run` with synthetic known output:

```json
{"case":"happy_mcp_shell","input":"2+2","expected":4,"actual":4}
```

Separate SoT read after trigger:

```text
listener owner=52616
supervisor state=adopted_existing
generation=0
child_pid=52616
```

### Edge 1: daemon process disappears

Before trigger:

```text
process pid=52616 path=C:\Users\hotra\.cargo\bin\synapse-mcp.exe
listener owner=52616
```

Trigger: exact verified PID stop:

```powershell
Stop-Process -Id 52616 -Force
```

After trigger:

```text
process pid=73492 parent_pid=57088 path=C:\Users\hotra\.cargo\bin\synapse-mcp.exe
listener owner=73492
supervisor state=running
generation=1
child_pid=73492
```

Supervisor events after trigger:

```json
{"event":"adopted_exit","generation":0,"child_pid":52616}
{"event":"launch_start","generation":1}
{"event":"launch_ok","generation":1,"child_pid":73492}
```

Daemon lifecycle file readback:

```text
daemon-exit.jsonl contains previous_run_unclean for old pid=52616 with new_pid=73492.
```

Real MCP client readback after restart:

```text
mcp__synapse.health ok=true pid=73492 tool_count=40 storage_backend=rocksdb
mcp__synapse.shell output={"case":"post_restart_mcp_shell","input":"3*7","expected":21,"actual":21}
```

### Edge 2: empty shell command payload

Before:

```text
listener owner=73492
supervisor state=running generation=1 child_pid=73492
```

Trigger: real MCP `shell.run` with `powershell.exe -NoProfile -Command ""`.

Result:

```text
exit_code=0 stdout="" stderr=""
```

After:

```text
listener owner=73492
supervisor state=running generation=1 child_pid=73492
```

### Edge 3: invalid shell child command

Before:

```text
listener owner=73492
supervisor state=running generation=1 child_pid=73492
```

Trigger: real MCP `shell.run` with:

```powershell
Write-Error 'synthetic invalid shell payload'; exit 42
```

Result:

```text
exit_code=42
stderr contains "synthetic invalid shell payload"
```

After:

```text
listener owner=73492
supervisor state=running generation=1 child_pid=73492
```

### Edge 4: duplicate demand start while task is running

Before:

```text
task_state=Running
listener owner=73492
process_count=3
processes=57088 powershell supervisor, 71328 wscript wrapper, 73492 synapse-mcp
```

Trigger:

```powershell
Start-ScheduledTask -TaskName SynapseMcpDaemon
```

After:

```text
task_state=Running
listener owner=73492
process_count=3
processes=57088 powershell supervisor, 71328 wscript wrapper, 73492 synapse-mcp
last_task_result=2147946720
```

Interpretation: Task Scheduler recorded the duplicate demand start as refused while `MultipleInstances=IgnoreNew`; the physical process/socket SoT proved no duplicate supervisor or daemon was created and the existing listener remained healthy.

## Final State

```text
Task SynapseMcpDaemon: Running
wscript supervisor wrapper PID: 71328
PowerShell supervisor PID: 57088
synapse-mcp PID: 73492
listener: 127.0.0.1:7700 owner_pid=73492
supervisor state: running generation=1 child_pid=73492
mcp__synapse.health: ok=true pid=73492 tool_count=40
```

Structural checks only:

```text
scripts\synapse-setup.ps1 parse ok
generated supervisor parse ok
setup -SkipBuild -SkipClientWiring completed successfully
```

No automated tests, benchmarks, CI, FSV harnesses, or fallback data were used.
