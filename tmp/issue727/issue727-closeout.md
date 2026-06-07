RESOLVED in `3c2e73d` (`fix(mcp): keep element typing background-safe [skip ci]`).

## Root cause

`act_type(into_element)` already had a UIA `ValuePattern.SetValue` route, but the real WinForms edit-control FSV showed that `ValuePattern.SetValue` can still bring the target fixture to the foreground even when the action reports itself as background-safe. A first native-message route using `WM_SETTEXT` stopped the foreground steal, but it did not satisfy the app-level multiline `TextChanged` state path and left bare LF newline state where a native multiline edit expects CRLF.

The fix routes UIA-resolved native edit HWNDs through foreground-safe Win32 text messages after the existing enabled/read-only UIA checks:

- `EM_SETSEL` + `EM_REPLACESEL` under `SendMessageTimeoutW`, so the edit control receives the normal replacement/change path without global foreground input.
- `ES_MULTILINE` detection with LF/CR normalization to CRLF before delivery and readback comparison.
- Password controls never expose raw value text; verification uses `WM_GETTEXTLENGTH` length-only readback.
- `act_type` now returns/reports `backend_tier_used` and `required_foreground` across CDP, Win32-message, UIA, and foreground keyboard paths.
- The MCP tool description now states that `into_element` routing does not require foreground; leased foreground keyboard applies only when no element target is supplied.

Best-practice/source references used:

- Microsoft UI Automation `ValuePattern.SetValue`: https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationvaluepattern-setvalue
- Microsoft UIA Edit control support, including password value rules: https://learn.microsoft.com/en-us/dotnet/framework/ui-automation/ui-automation-support-for-the-edit-control-type
- `SendMessageTimeoutW`: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendmessagetimeoutw
- `EM_SETSEL`: https://learn.microsoft.com/en-us/windows/win32/controls/em-setsel
- `EM_REPLACESEL`: https://learn.microsoft.com/en-us/windows/win32/controls/em-replacesel
- Edit control message/change behavior: https://learn.microsoft.com/en-us/windows/win32/controls/about-edit-controls
- `WM_SETTEXT`: https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-settext

## Supporting checks only

These are regression/build checks, not FSV:

- `cargo fmt --all --check` passed.
- `git diff --check` and `git diff --cached --check` passed.
- `cargo test -p synapse-mcp --bin synapse-mcp type_text -- --nocapture`: 13 passed, 0 failed.
- `cargo test -p synapse-mcp --test health_tools_list -- --nocapture`: 1 passed, 0 failed.
- `cargo check -p synapse-mcp --bin synapse-mcp` passed.
- Release binary was built and installed before the final manual runtime FSV; daemon restarted on PID `51856`.

## Manual FSV evidence

Source of Truth:

- Real repo-built `synapse-mcp.exe` daemon process and socket.
- Real Synapse MCP tool call through a fresh primary Claude agent launched by Synapse `act_launch`, not a subagent.
- Fixture UI state file and native edit HWND `WM_GETTEXT` / `WM_GETTEXTLENGTH` reads after each trigger.
- Foreground sentinel HWND and separate `GetForegroundWindow` reads after each trigger.
- `CF_ACTION_LOG` readback through `storage_inspect`.
- `control_lease_status` readback.

MCP precondition/readback:

- Daemon PID: `51856`.
- Bind: `127.0.0.1:7700`.
- Process path: `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`.
- Command line: `synapse-mcp.exe --mode http --bind 127.0.0.1:7700 --db C:\Users\hotra\AppData\Local\synapse\db-daemon --profile-dir C:\Users\hotra\.cargo\bin\profiles --log-level info`.
- `health` returned `ok` from PID `51856`.
- Fresh primary Claude runner was launched by Synapse, PID `50460`, and loaded the real wired `mcp__synapse` tools.
- Primary Claude Synapse session: `e9168488-14d2-47dd-8145-6b52b59fc619`.
- Fixture launched by Synapse: PID `23456`, HWND `4788518`.
- Foreground sentinel launched by Synapse: PID `49692`, HWND `40635352`.
- Final `control_lease_status`: `held=false`, `owner_session_id=null`.

Element IDs used:

- Normal edit: `0x791100:0000002a00791100`.
- Read-only edit: `0x81166:0000002a00081166`.
- Password edit: `0xae108e:0000002a00ae108e`.
- Multiline edit: `0x11010e0:0000002a011010e0`.

Baseline SoT before triggers:

```json
{
  "normal": "",
  "normal_len": 0,
  "read_only": "LOCKED-727",
  "read_only_len": 10,
  "password_len": 0,
  "multiline": "",
  "multiline_len": 0,
  "foreground_hwnd": 40635352,
  "fixture_is_foreground": false
}
```

Happy path trigger:

- Trigger: real `mcp__synapse__act_type` into normal edit with text `FSV727 primary normal`.
- Expected: normal edit value changes to that exact string; foreground remains sentinel.
- Response fields: `backend_tier_used=win32_message`, `required_foreground=false`, `target_readback_required=false`, `target_text_integrity=win32_text_message_readback`.
- Separate SoT after read:

```json
{
  "normal": "FSV727 primary normal",
  "normal_len": 21,
  "foreground_hwnd": 40635352,
  "fixture_is_foreground": false
}
```

Edge 1, empty input:

- Trigger: real `act_type` into normal edit with empty text.
- Expected: normal edit becomes empty; foreground remains sentinel.
- Response fields: `backend_tier_used=win32_message`, `required_foreground=false`, `target_text_integrity=win32_text_message_readback`.
- Separate SoT after read:

```json
{
  "normal": "",
  "normal_len": 0,
  "foreground_hwnd": 40635352,
  "fixture_is_foreground": false
}
```

Edge 2, read-only edit:

- Trigger: real `act_type` into read-only edit with text `SHOULD-NOT-WRITE-727`.
- Expected: fail closed and no state mutation.
- Actual error: `-32099`, `act_type verify_delta observed no Source-of-Truth state change within 250 ms`.
- Separate SoT after read:

```json
{
  "read_only": "LOCKED-727",
  "read_only_len": 10,
  "foreground_hwnd": 40635352,
  "fixture_is_foreground": false
}
```

Edge 3, password edit:

- Trigger: real `act_type` into password edit with text `p@ss727`.
- Expected: no raw password readback; length-only SoT becomes 7; foreground remains sentinel.
- Response fields: `backend_tier_used=win32_message`, `required_foreground=false`, `target_text_integrity=win32_text_message_password_length_readback`, `source=win32_window_text.password_length`, `before_signature=password_len:0`, `after_signature=password_len:7`.
- Separate SoT after read:

```json
{
  "password_len": 7,
  "raw_password_in_state_file": false,
  "foreground_hwnd": 40635352,
  "fixture_is_foreground": false
}
```

Edge 4, multiline newline handling:

- Trigger: real `act_type` into multiline edit with text containing a single LF: `LineA-727\nLineB-727`.
- Expected: native multiline edit stores CRLF form and fixture state follows the control's change notification path; foreground remains sentinel.
- Response fields: `backend_tier_used=win32_message`, `required_foreground=false`, `target_text_integrity=win32_text_message_readback`.
- Separate SoT after file read and native `WM_GETTEXT`:

```json
{
  "multiline": "LineA-727\r\nLineB-727",
  "multiline_len": 20,
  "native_wm_gettext": "LineA-727\r\nLineB-727",
  "native_wm_gettext_len": 20,
  "foreground_hwnd": 40635352,
  "fixture_is_foreground": false
}
```

Final state file read:

```json
{
  "normal": "",
  "normal_len": 0,
  "read_only": "LOCKED-727",
  "read_only_len": 10,
  "password_len": 7,
  "multiline": "LineA-727\r\nLineB-727",
  "multiline_len": 20,
  "updated_at": "2026-06-07T09:38:46.6019449Z"
}
```

Audit/storage readback:

- `storage_inspect` on `CF_ACTION_LOG` read `row_count=2135`.
- Password `act_type` ok row: `backend_tier_used=win32_message`, `required_foreground=false`, `target_text_integrity=win32_text_message_password_length_readback`, `source=win32_window_text.password_length`, foreground sentinel HWND `40635352`.
- Multiline `act_type` ok row: `backend_tier_used=win32_message`, `chars_typed=19`, `required_foreground=false`, `target_readback_required=false`, `target_text_integrity=win32_text_message_readback`, postcondition `source=win32_window_text`, `observed_delta=true`, foreground sentinel HWND `40635352`.
- `CF_PROCESS_HISTORY` rows showed fixture PID `23456`, sentinel PID `49692`, and hidden primary runner PID `50460` launched through Synapse.

Cleanup/readback:

- Exact owned temporary fixture PID `23456` and sentinel PID `49692` were stopped after evidence was captured.
- No global terminal/IDE/WSL cleanup was performed.
- Final daemon SoT remains PID `51856` listening on `127.0.0.1:7700`.
- Setup lock readback: absent.

Note: the already-running Codex process still had stale current-process Synapse tool metadata after daemon reinstall (`SYNAPSE_CODEX_CURRENT_PROCESS_ENV_STALE`); final runtime FSV therefore used a fresh Synapse-launched primary Claude process with the real wired Synapse MCP client against daemon PID `51856`, per D4.
