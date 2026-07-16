# Manual FSV: Issue #1714 Setup Chrome Bridge Health Timeout

Date: 2026-07-16

## Root Cause

`scripts\synapse-setup.ps1` already had a bounded Chrome bridge reconnect wait
after daemon handoff, but one `/health` read inside that wait used
`Invoke-RestMethod -TimeoutSec 4` and immediately called `Die` on any
exception. During #1660 host restoration, setup installed and started a healthy
daemon but exited nonzero because one health request timed out while Chrome was
reconnecting. A separate readback seconds later showed the bridge was healthy.

The bug was not that setup lacked a deadline. The bug was that a transient
health-read failure inside the deadline was classified as the terminal state.

## Research

- Microsoft PowerShell `Invoke-RestMethod` documentation:
  `https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/invoke-restmethod`
  documents timeout and retry parameters; built-in retry is tied to HTTP status
  failures, so timeout exceptions still need explicit caller handling.
- Microsoft Azure Architecture Center transient fault guidance:
  `https://learn.microsoft.com/en-us/azure/architecture/best-practices/transient-faults`
  recommends retrying transient faults with bounded retry windows and clear
  final failure reporting.
- Microsoft Retry pattern:
  `https://learn.microsoft.com/en-us/azure/architecture/patterns/retry`
  reinforces retrying temporary connectivity failures while preserving a final
  error when the operation does not succeed within policy.

Implementation choices from that research:

- Keep setup fail-closed and deadline-bound.
- Convert transient `/health` read exceptions into logged wait-loop state.
- Preserve exact phase, attempt, timeout, remaining wait, and last error in
  logs.
- Fail only when the bounded reconnect, UI repair, or post-reload deadline
  expires without a healthy readback.

## Source of Truth

- Setup result SoT: the `pwsh` process exit code.
- Daemon process SoT: Windows process table.
- Socket SoT: `Get-NetTCPConnection -LocalAddress 127.0.0.1 -LocalPort 7700`.
- Scheduled task SoT: `Get-ScheduledTask -TaskName SynapseMcpDaemon`.
- Installed binary SoT: `Get-FileHash` on
  `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`.
- Runtime health SoT: real `mcp__synapse.health` and direct setup `/health`
  reads against `127.0.0.1:7700`.

## Code Change

- `Read-SynapseHealthForRestartGuard` now accepts a validated `TimeoutSec` and
  returns `Ok`, `Health`, `Error`, and `TimeoutSec` without throwing on request
  failures.
- The Chrome alarm-reconnect wait now logs failed health reads and keeps waiting
  until the existing deadline; terminal reconnect timeout includes
  `last_health_error`.
- The existing-Chrome UI repair wait uses the same bounded behavior.
- The post-reload health read uses a 45s bounded retry window instead of one
  immediate fatal request.

No fallback path was added; a bridge that never becomes healthy still fails
setup with the last physical health-read error.

## Manual State Readbacks

### Before

- Scheduled task: `SynapseMcpDaemon=Running`.
- Listener: `127.0.0.1:7700` owned by PID `60560`.
- Installed binary hash:
  `7A94967D681D77FA499FC944A90DF7285476E990682464ABB8456495F503EEEE`.
- Pre-fix failure from #1660 restoration:
  `SYNAPSE_CHROME_BRIDGE_WAIT_HEALTH_FAILED bind=127.0.0.1:7700 error=The request was canceled due to the configured HttpClient.Timeout of 4 seconds elapsing`.
- Separate post-failure health read was already healthy:
  `ok=true`, `chrome_bridge.status=ok`, `host_count=1`.

### Happy Path: Real Setup Handoff With Reconnect Wait

Trigger:

```text
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts\synapse-setup.ps1 -SourceDir C:\code\Synapse -ForceRestart -SkipClientWiring -ActiveIssue 1714
```

Observed setup state:

- Candidate daemon preflight passed on `127.0.0.1:60882`, PID `66624`,
  `tool_count=40`, tool surface
  `d4ef68fb707f9ff6e2fd6ef452bc88df0f0d2b0713f692fb5b462369b470dc7c`.
- Setup gracefully stopped old daemon PID `60560`.
- Installed binary hash became:
  `B84E66E9E12021D65D6939E44EE61CC7E262650D26DC667170F333E69B08EB45`.
- New daemon started as PID `73276`.
- The real reconnect wait path was exercised:
  - attempt `1`: `no_active_chrome_bridge_host`, `wait_remaining_ms=104998`.
  - attempt `2`: `no_active_chrome_bridge_host`, `wait_remaining_ms=102489`.
  - attempt `3`: `no_active_chrome_bridge_host`, `wait_remaining_ms=100065`.
  - then: `Chrome bridge OK after daemon start wait: stale=false capability=pageScreenshot`.
- Setup wrote the Codex tool-surface snapshot and exited `0`.

After-state readback:

- Scheduled task: `SynapseMcpDaemon=Running`.
- Listener: `127.0.0.1:7700` owned by PID `73276`.
- Process: `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`, started
  `2026-07-16 13:43:17`.
- Installed binary hash:
  `B84E66E9E12021D65D6939E44EE61CC7E262650D26DC667170F333E69B08EB45`.
- `mcp__synapse.health`: `ok=true`, PID `73276`,
  `chrome_bridge.status=ok`, `host_count=1`, `tab_control_available=true`,
  `tool_count=40`, tool surface
  `d4ef68fb707f9ff6e2fd6ef452bc88df0f0d2b0713f692fb5b462369b470dc7c`.

Verdict: PASS.

## Edge Probes

The exact `Read-SynapseHealthForRestartGuard` function definition was loaded
from `scripts\synapse-setup.ps1` in a one-off PowerShell session. The daemon
state was read before and after the probes to prove no mutation.

Before probes:

```json
{"task":"Running","listen_pid":73276}
```

### Edge 1: Valid Health Read

- Trigger: `Read-SynapseHealthForRestartGuard -Bind 127.0.0.1:7700 -Token <real> -TimeoutSec 4`.
- Result: `ok=true`, `pid=73276`, `timeout_sec=4`, `error=null`.
- Verdict: PASS.

### Edge 2: Invalid Token Fails Closed Without Throwing

- Trigger: same function with `<real-token>-bad`.
- Result: `ok=false`, `timeout_sec=4`,
  `error="Response status code does not indicate success: 401 (Unauthorized)."`.
- Verdict: PASS.

### Edge 3: Unused Port Timeout Is Returned As State

- Trigger: same function with `Bind=127.0.0.1:65534`, `TimeoutSec=1`.
- Result: `ok=false`, `timeout_sec=1`,
  `error="The request was canceled due to the configured HttpClient.Timeout of 1 seconds elapsing."`.
- Verdict: PASS.

### Edge 4: Structurally Invalid Timeout Rejected By Parameter Binding

- Trigger: same function with `TimeoutSec=0`.
- Result: parameter binding rejected the call:
  `Cannot validate argument on parameter 'TimeoutSec'. The 0 argument is less than the minimum allowed range of 1.`
- Verdict: PASS.

After probes:

```json
{"task":"Running","listen_pid":73276}
```

No daemon process, task, or listener state changed during the edge probes.

## Structural Checks

These are structural checks only, not FSV:

```text
[scriptblock]::Create((Get-Content -Raw scripts\synapse-setup.ps1))
git diff --check
```

Both passed. No automated tests or FSV harnesses were created or run.
