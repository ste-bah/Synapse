# Issue #1707 - Chrome Bridge UI Reload Navigation Verification

Date: 2026-07-16

## Root Cause

`scripts/install-synapse-chrome-debugger.ps1` used `SetForegroundWindow`, `AutomationElement.SetFocus`, and `ValuePattern.SetValue` to put `chrome://extensions/?id=...` in the Chrome address bar, then immediately searched for the extension details Reload button. It did not separately prove that Windows made the selected Chrome window the foreground keyboard target, and it did not prove Chrome reached the extension details URL before looking for `dev-reload-button`.

The observed #1707 failure reported `SYNAPSE_CHROME_BRIDGE_UI_RELOAD_BUTTON_NOT_FOUND` while the latest title was still `Christopher Royse | LinkedIn - Google Chrome`. That means the reload button was a downstream symptom: the setup path had searched the stale page because navigation was never confirmed.

## Research Used

- Exa MCP search and web search both found the Microsoft `SetForegroundWindow` documentation: Windows can deny foreground changes even when a caller appears eligible, and keyboard input goes to the foreground/focused window. Source: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow
- Exa MCP search and web search found Microsoft UI Automation focus documentation: `AutomationElement.SetFocus` does not necessarily bring the element to the foreground. Source: https://learn.microsoft.com/en-us/dotnet/api/system.windows.automation.automationelement.setfocus
- Exa MCP search and web search found Chrome extension lifecycle/runtime references: extension reload is a real runtime lifecycle event, and MV3 service workers reconnect after lifecycle events. Sources: https://developer.chrome.com/docs/extensions/reference/api/runtime and https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle

## Fix

Changed `scripts/install-synapse-chrome-debugger.ps1` to make Chrome UI navigation fail closed and evidence-bearing:

- Replaced the unverified UIA address-bar setter with `Invoke-SynapseChromeAddressBarNavigation`.
- Added foreground readback through `Read-SynapseForegroundWindow`.
- Added Chrome navigation readback through `Read-SynapseChromeNavigationState` and `Read-SynapseChromeAddressBarValue`.
- The helper now:
  - reads target state before input,
  - calls `ShowWindowAsync` / `SetForegroundWindow`,
  - waits until `GetForegroundWindow()` matches the selected Chrome HWND,
  - sends deterministic `Esc`, `Ctrl+L`, clipboard paste, `Enter`,
  - waits until the expected title or exact address-bar URL is observed,
  - throws narrow structured errors if foreground, input, clipboard restore, or navigation confirmation fails.
- The existing-extension reload path now stores `navigation` readback in `synapse_chrome_auto_install`.
- The first-time Load unpacked path uses the same navigation helper before looking for `Load unpacked`.
- If navigation succeeds but the Reload button is still missing, the old reload-button error now includes navigation evidence and accurately means the page was reached but the UI control was absent.

New fail-closed codes:

- `SYNAPSE_CHROME_NAVIGATION_FOREGROUND_NOT_ACQUIRED`
- `SYNAPSE_CHROME_NAVIGATION_KEY_INPUT_FAILED`
- `SYNAPSE_CHROME_NAVIGATION_CLIPBOARD_RESTORE_FAILED`
- `SYNAPSE_CHROME_NAVIGATION_NOT_CONFIRMED`

## Source Of Truth

- Runtime: real `synapse-mcp.exe` process and `127.0.0.1:7700` listener.
- Client parity: Codex loaded the real `mcp__synapse` tool surface; `health` and `browser_debugger` were present through tool discovery.
- Browser state: Chrome top-level window HWND `1050546`, PID `11008`, visible `MainWindowTitle`.
- Profile state: `C:\Users\hotra\AppData\Local\Google\Chrome\User Data\Profile 5\Secure Preferences`, extension row `leoocgnkjnplbfdbklajepahofecgfbk`.
- Bridge runtime state: `/health?detail=compact` `subsystems.chrome_bridge` and active host id inside the detail string.
- Physical log state: `C:\Users\hotra\AppData\Local\synapse\logs\synapse.log.2026-07-16` entries `CHROME_DEBUGGER_NATIVE_HOST_REGISTERED` and `CHROME_DEBUGGER_EXTENSION_HELLO`.
- Installer evidence files:
  - `C:\Users\hotra\AppData\Local\synapse\issue-1707-installer-extension-page.json`
  - `C:\Users\hotra\AppData\Local\synapse\issue-1707-installer-non-extension-page.json`
  - `C:\Users\hotra\AppData\Local\synapse\issue-1707-installer-min-timeout.json`
  - `C:\Users\hotra\AppData\Local\synapse\issue-1707-invalid-id-output.txt`

## Runtime Precondition Readback

- `mcp__synapse.health(detail=compact)` returned `ok=true`, PID `68704`, `tool_count=40`, and tool names included `health` and `browser_debugger`.
- Process readback: `synapse-mcp.exe` PID `68704`, path `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`, started `2026-07-16T05:10:06.9290196-05:00`.
- Socket readback: `127.0.0.1:7700` listener owned by PID `68704`.
- Chrome bridge before #1707 manual runs: `status=ok`, `extension_stale=false`, `host_count=1`, active host `chrome-native-0-1784197124698`.
- Profile 5 Secure Preferences row existed and pointed at `C:\Users\hotra\AppData\Local\synapse\chrome-extension\synapse-chrome-bridge-2026-07-13-operator-panic-continuity-v3`.

## Happy Path - Already On Extension Details Page

Trigger: real installer path:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -Command "& { $result = & 'C:\code\Synapse\scripts\install-synapse-chrome-debugger.ps1' -SynapseNativeHostExe '$env:USERPROFILE\.cargo\bin\synapse-chrome-native-host.exe' -ReloadExistingExtensionViaUi -AutoInstallTimeoutSeconds 45; $result | ConvertTo-Json -Depth 60 }"
```

Before state:

- Active host before this trigger: `chrome-native-0-1784197735051`.
- Chrome visible page: `Extensions - Synapse Chrome Bridge - Google Chrome`.

Installer readback:

```json
{
  "reason": "existing_ready_extension_ui_reload_invoked",
  "active_profile": "Profile 5",
  "required_foreground": true,
  "navigation_method": "foreground_ctrl_l_clipboard_enter",
  "foreground_before_title": "Welcome - Astrolabe - Visual Studio Code",
  "foreground_acquired_title": "Extensions - Synapse Chrome Bridge - Google Chrome",
  "nav_before_title": "Extensions - Synapse Chrome Bridge - Google Chrome",
  "nav_after_title": "Extensions - Synapse Chrome Bridge - Google Chrome",
  "nav_after_address": "chrome://extensions/?id=leoocgnkjnplbfdbklajepahofecgfbk",
  "reload_button_before": true,
  "enable_toggle_before": "On, extension enabled",
  "reload_button_after": true,
  "enable_toggle_after": "On, extension enabled",
  "profile_after_ready": true,
  "profile_after_manifest_path_matches": true
}
```

After SoT read:

- Log lines `15191/15192` recorded `CHROME_DEBUGGER_NATIVE_HOST_REGISTERED` and `CHROME_DEBUGGER_EXTENSION_HELLO`.
- New active host: `chrome-native-0-1784197778926`.
- `/health?detail=compact` stayed `status=ok`, `extension_stale=false`, `host_count=1`.

## Edge 1 - Start From Non-Extension Page

Before state:

- Real Chrome bridge `browser_tabs list` showed active tab title `Issues · ChrisRoyse/Poly`.
- Process readback showed Chrome HWND `1050546` title `Issues · ChrisRoyse/Poly - Google Chrome`.
- Active host before trigger: `chrome-native-0-1784197778926`.

Trigger: same installer command with `-ReloadExistingExtensionViaUi -AutoInstallTimeoutSeconds 45`.

Installer readback:

```json
{
  "reason": "existing_ready_extension_ui_reload_invoked",
  "navigation_method": "foreground_ctrl_l_clipboard_enter",
  "foreground_before_title": "Welcome - Astrolabe - Visual Studio Code",
  "foreground_acquired_title": "Issues · ChrisRoyse/Poly - Google Chrome",
  "nav_before_title": "Issues · ChrisRoyse/Poly - Google Chrome",
  "nav_after_title": "Extensions - Synapse Chrome Bridge - Google Chrome",
  "nav_after_address": "chrome://extensions/?id=leoocgnkjnplbfdbklajepahofecgfbk",
  "reload_button_before": true,
  "enable_toggle_before": "On, extension enabled",
  "reload_button_after": true,
  "enable_toggle_after": "On, extension enabled",
  "profile_after_ready": true
}
```

After SoT read:

- Log lines `15236/15237` recorded `CHROME_DEBUGGER_NATIVE_HOST_REGISTERED` and `CHROME_DEBUGGER_EXTENSION_HELLO`.
- Active host changed to `chrome-native-0-1784197842936`.
- Chrome visible title after trigger: `Extensions - Synapse Chrome Bridge - Google Chrome`.
- `/health?detail=compact` stayed `status=ok`, `extension_stale=false`, `host_count=1`.

This is the direct regression case for #1707: the stale non-extension page is no longer allowed to become a misleading Reload-button-not-found error.

## Edge 2 - Minimum Timeout Boundary

Before state:

- Active host before trigger: `chrome-native-0-1784197842936`.
- Chrome visible title: `Extensions - Synapse Chrome Bridge - Google Chrome`.

Trigger:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -Command "& { $result = & 'C:\code\Synapse\scripts\install-synapse-chrome-debugger.ps1' -SynapseNativeHostExe '$env:USERPROFILE\.cargo\bin\synapse-chrome-native-host.exe' -ReloadExistingExtensionViaUi -AutoInstallTimeoutSeconds 5; $result | ConvertTo-Json -Depth 60 }"
```

Installer and after-state readback:

```json
{
  "before_host_id": "chrome-native-0-1784197842936",
  "after_host_id": "chrome-native-0-1784197891762",
  "host_changed": true,
  "reason": "existing_ready_extension_ui_reload_invoked",
  "navigation_method": "foreground_ctrl_l_clipboard_enter",
  "nav_before_title": "Extensions - Synapse Chrome Bridge - Google Chrome",
  "nav_after_title": "Extensions - Synapse Chrome Bridge - Google Chrome",
  "nav_after_address": "chrome://extensions/?id=leoocgnkjnplbfdbklajepahofecgfbk",
  "reload_button_before": true,
  "reload_button_after": true,
  "profile_after_ready": true
}
```

After SoT read:

- Log lines `15260/15261` recorded `CHROME_DEBUGGER_NATIVE_HOST_REGISTERED` and `CHROME_DEBUGGER_EXTENSION_HELLO`.
- Active host changed to `chrome-native-0-1784197891762`.
- Chrome visible title stayed `Extensions - Synapse Chrome Bridge - Google Chrome`.

## Edge 3 - Structurally Invalid Extension ID

Before state:

- Active host before trigger: `chrome-native-0-1784197891762`.
- Profile 5 manifest path: `C:\Users\hotra\AppData\Local\synapse\chrome-extension\synapse-chrome-bridge-2026-07-13-operator-panic-continuity-v3`.
- `CHROME_DEBUGGER_EXTENSION_HELLO` count in `synapse.log.2026-07-16`: `22`.

Trigger:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts\install-synapse-chrome-debugger.ps1 -ExtensionId bad-id -SynapseNativeHostExe "$env:USERPROFILE\.cargo\bin\synapse-chrome-native-host.exe" -ReloadExistingExtensionViaUi -AutoInstallTimeoutSeconds 45
```

After readback:

```json
{
  "exit_code": 1,
  "output": "SYNAPSE_CHROME_EXTENSION_ID_INVALID extension_id=bad-id remediation=Chrome extension IDs are 32 lowercase characters in the range a-p; refusing to inspect profiles with an ambiguous extension identity",
  "before_host_id": "chrome-native-0-1784197891762",
  "after_host_id": "chrome-native-0-1784197891762",
  "host_unchanged": true,
  "before_manifest_path": "C:\\Users\\hotra\\AppData\\Local\\synapse\\chrome-extension\\synapse-chrome-bridge-2026-07-13-operator-panic-continuity-v3",
  "after_manifest_path": "C:\\Users\\hotra\\AppData\\Local\\synapse\\chrome-extension\\synapse-chrome-bridge-2026-07-13-operator-panic-continuity-v3",
  "profile_row_unchanged": true,
  "before_hello_count": 22,
  "after_hello_count": 22,
  "no_new_hello": true
}
```

This proves the invalid input fails closed before the profile row or live bridge host changes.

## Structural Checks

- PowerShell parser:
  - `[System.Management.Automation.Language.Parser]::ParseFile(...)`
  - Result: `parse_ok`
- Diff whitespace:
  - `git diff --check`
  - Result: exit code `0`

No automated tests, benches, FSV scripts, FSV harnesses, GitHub Actions, or CI were created or run.

## Final Runtime State

- Daemon PID `68704` remains live.
- `127.0.0.1:7700` remains owned by PID `68704`.
- Chrome bridge health remains `status=ok`, `extension_stale=false`, `host_count=1`.
- Active host after all successful reload triggers: `chrome-native-0-1784197891762`.
- Chrome visible title: `Extensions - Synapse Chrome Bridge - Google Chrome`.
- Profile 5 Secure Preferences row still points at the stable Synapse Chrome Bridge directory.
