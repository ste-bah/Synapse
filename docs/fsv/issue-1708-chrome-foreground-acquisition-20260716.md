# Issue #1708 FSV - Chrome Bridge UI Foreground Acquisition

Date: 2026-07-16

## Root Cause

`scripts/install-synapse-chrome-debugger.ps1` used only `ShowWindowAsync` plus a single
`SetForegroundWindow` call before sending `Ctrl+L` / paste / Enter to Chrome. On this host,
Windows foreground activation rules denied `SetForegroundWindow` while VS Code owned foreground
input, so setup failed with `SYNAPSE_CHROME_NAVIGATION_FOREGROUND_NOT_ACQUIRED`.

The first fix also exposed a PowerShell return-shaping bug: returning a raw
`System.Collections.Generic.List[object]` inside `[pscustomobject]` raised `Argument types do not
match`. The final fix materializes attempts with `ToArray()` and keeps native attach-thread work in
typed C# interop.

## Research Used

- Microsoft `SetForegroundWindow`: Windows restricts which processes may set the foreground window.
  https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow
- Microsoft `AttachThreadInput`: input queues can be temporarily attached and must be detached.
  https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-attachthreadinput
- Microsoft `LockSetForegroundWindow`: ALT/user input can re-enable foreground changes.
  https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-locksetforegroundwindow

## Fix

- Added typed Win32 interop for `GetWindowThreadProcessId`, `GetCurrentThreadId`,
  `AttachThreadInput`, `BringWindowToTop`, and `SetFocus`.
- Added `Wait-SynapseChromeForegroundAcquisition`, which tries:
  1. `ShowWindowAsync` + `SetForegroundWindow`.
  2. Attached input queues + `BringWindowToTop` + `SetForegroundWindow` + `SetFocus`.
  3. ALT unlock + `SetForegroundWindow`.
- Added a separate foreground readback loop before any navigation keystrokes are sent.
- Preserved fail-closed behavior with structured `SYNAPSE_CHROME_FOREGROUND_ACQUISITION_FAILED`
  diagnostics that include stage, target window, and live foreground readback.

## Source Of Truth

- Foreground/UI SoT: `GetForegroundWindow` + UI Automation window title/class/PID.
- Chrome profile SoT: `C:\Users\hotra\AppData\Local\Google\Chrome\User Data\Profile 5\Secure Preferences`.
- Daemon/bridge SoT: process table, `127.0.0.1:7700` socket table, and authenticated
  `http://127.0.0.1:7700/health`.

## Structural Checks

- PowerShell parser: `parse_ok`.
- `git diff --check`: only existing CRLF normalization warning for
  `scripts/install-synapse-chrome-debugger.ps1`.
- `cargo fmt --all --check`: passed.
- `cargo check -p synapse-mcp`: passed.
- `cargo clippy -p synapse-mcp --all-targets`: passed.
- No automated tests or FSV harnesses were added or run.

## Manual FSV

### Before

Daemon before setup retry:

```text
synapse-mcp pid=10504 path=C:\Users\hotra\.cargo\bin\synapse-mcp.exe
listener=127.0.0.1:7700 owner=10504
```

Blocked foreground before the #1708 reproduction:

```json
{
  "foreground_hwnd": 23662538,
  "foreground_pid": 32084,
  "foreground_process": "Code",
  "foreground_title": "FULL_SYSTEM_PLAN.md - poly - Visual Studio Code",
  "chrome_window_hwnd": 1050546,
  "chrome_window_pid": 11008,
  "chrome_window_title": "YouTube - Google Chrome"
}
```

### Trigger

Full setup trigger:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\synapse-setup.ps1 -SourceDir C:\code\Synapse -ActiveIssue 1708 -ForceRestart
```

Focused UI repair trigger:

```powershell
.\scripts\install-synapse-chrome-debugger.ps1 -ReloadExistingExtensionViaUi -AutoInstallTimeoutSeconds 45
```

### After

Full setup reached Chrome repair success and then failed only at the existing Codex current-process
schema-stale guard:

```text
Chrome bridge UI repair completed reason=existing_ready_extension_ui_reload_invoked active_profile=Profile 5 chrome_window_pid=11008 chrome_window_hwnd=1050546
Chrome bridge OK after existing-Chrome UI repair: reason=existing_ready_extension_ui_reload_invoked stale=false capability=pageScreenshot
FATAL: SYNAPSE_CODEX_CURRENT_PROCESS_SCHEMA_STALE codex_pid=27984 start_tool_surface_sha256=7cc1d191... current_tool_surface_sha256=e20cb889...
handoff=C:\Users\hotra\AppData\Local\synapse\codex-restart-handoffs\codex-restart-handoff-27984-20260716T112418072Z.json
```

Physical readback after setup:

```text
synapse-mcp pid=30284 path=C:\Users\hotra\.cargo\bin\synapse-mcp.exe
listener=127.0.0.1:7700 owner=30284
health.ok=true
health.subsystems.chrome_bridge.status=ok
extension_id=leoocgnkjnplbfdbklajepahofecgfbk
extension_build_id=synapse-chrome-bridge-2026-07-13-operator-panic-continuity-v3
host_count=1
```

## Edge Cases

### 1. Non-Chrome Foreground

Before:

```json
{
  "foreground_hwnd": 23662538,
  "foreground_pid": 32084,
  "foreground_process": "Code",
  "foreground_title": "FULL_SYSTEM_PLAN.md - poly - Visual Studio Code"
}
```

After repair trigger:

```json
{
  "ok": true,
  "changed": true,
  "reason": "existing_ready_extension_ui_reload_invoked",
  "acquisition_method": "show_window_async_set_foreground",
  "navigation_after": "chrome://extensions/?id=leoocgnkjnplbfdbklajepahofecgfbk",
  "foreground_after": {
    "process": "chrome",
    "title": "Extensions - Synapse Chrome Bridge - Google Chrome",
    "hwnd": 1050546,
    "pid": 11008
  }
}
```

### 2. Invalid Extension Id

Trigger:

```powershell
.\scripts\install-synapse-chrome-debugger.ps1 -ExtensionId 'INVALID!!!' -ReloadExistingExtensionViaUi -AutoInstallTimeoutSeconds 10
```

Readback:

```json
{
  "status": "expected_failure",
  "error": "SYNAPSE_CHROME_EXTENSION_ID_INVALID extension_id=INVALID!!! remediation=Chrome extension IDs are 32 lowercase characters in the range a-p; refusing to inspect profiles with an ambiguous extension identity",
  "before_synapse_extension_path": "C:\\Users\\hotra\\AppData\\Local\\synapse\\chrome-extension\\synapse-chrome-bridge-2026-07-13-operator-panic-continuity-v3",
  "after_synapse_extension_path": "C:\\Users\\hotra\\AppData\\Local\\synapse\\chrome-extension\\synapse-chrome-bridge-2026-07-13-operator-panic-continuity-v3",
  "path_unchanged": true
}
```

### 3. Minimum Timeout Boundary

Trigger:

```powershell
.\scripts\install-synapse-chrome-debugger.ps1 -ReloadExistingExtensionViaUi -AutoInstallTimeoutSeconds 5
```

Readback:

```json
{
  "ok": true,
  "changed": true,
  "reason": "existing_ready_extension_ui_reload_invoked",
  "acquisition_method": "show_window_async_set_foreground",
  "address_after": "chrome://extensions/?id=leoocgnkjnplbfdbklajepahofecgfbk",
  "ui_after_enabled": true,
  "profile_path": "C:\\Users\\hotra\\AppData\\Local\\synapse\\chrome-extension\\synapse-chrome-bridge-2026-07-13-operator-panic-continuity-v3"
}
```

## Verdict

#1708 is fixed. The Chrome UI repair no longer fails on foreground acquisition, refuses to send
navigation keys until Chrome is physically foreground, and leaves enough staged diagnostics to root
cause any future foreground failure. The only remaining full setup failure in this Codex process is
the documented `SYNAPSE_CODEX_CURRENT_PROCESS_SCHEMA_STALE` restart guard, not the Chrome foreground
bug.
