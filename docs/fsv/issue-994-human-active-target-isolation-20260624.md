# Issue #994 FSV: Human OS Foreground Is Not Agent Active Target

Date: 2026-06-24T17:43:32.3755181-05:00
Issue: https://github.com/ChrisRoyse/Synapse/issues/994
Command:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\fsv\issue-994-human-active-target-isolation.ps1
```

## Result

Pass. Real Synapse MCP, real Chrome bridge, and real Win32 foreground/cursor state were used.

Run marker: `issue994-human-active-20260624-174233`

## MCP And Browser State

- MCP daemon health: `ok=true`, PID `8588`, tool count `173`
- Chrome bridge status: `ok`
- Browser HWND: `524970`, title `Synapse Command Center - Google Chrome`
- Observer session: `ed2f30c1-7a60-4a5f-a59e-79b69615f417`
- Session A: `71e68190-61a4-42db-a672-ab8dbc5e6970`
- Session B: `c7643c1d-172e-453f-ae7f-61dd1c6e2251`
- Session A CDP target: `chrome-tab:589708931`
- Session B CDP target: `chrome-tab:589708932`

Both tab opens reported unchanged human OS foreground:

- Session A foreground before/after: `15860480` -> `15860480`
- Session B foreground before/after: `15860480` -> `15860480`

## Human-Active Foreground And Cursor Evidence

The FSV drove real OS foreground and cursor movement while the two session-scoped `target_act` calls were outstanding.

- Baseline foreground: HWND `15860480`, PID `22368`, process `Notepad`, title `Untitled - Notepad`
- Foreground samples: `28`
- Distinct foreground HWNDs observed: `15860480`, `9635466`
- Distinct cursor positions observed: `28`
- Foreground sample 1: `set-field-0`, Notepad, cursor `120,180`
- Foreground sample 2: `set-field-1`, VS Code, cursor `157,209`
- Foreground sample 3: `set-field-2`, Notepad, cursor `194,238`
- Foreground sample 4: `set-field-3`, VS Code, cursor `231,267`
- Final pre-cleanup foreground: HWND `9635466`, PID `19876`, process `Code`, title `Welcome - synapse - Visual Studio Code`

## Target Isolation Evidence

Each session acted only in its own CDP target while the human OS foreground moved independently.

- Session A set field: `status=ok`, `required_foreground=false`
- Session B set field: `status=ok`, `required_foreground=false`
- Session A press button: `status=ok`, `required_foreground=false`
- Session B press button: `status=ok`, `required_foreground=false`
- Session A readback contained only `issue994-human-active-20260624-174233-A`
- Session B readback contained only `issue994-human-active-20260624-174233-B`
- Opposite-session markers were absent from both readbacks.

During the run, Synapse's SoT reported:

- `active_foreground_lane_count=2`
- `claimed_target_lane_count=2`
- `claim_count=2`
- `explicit_real_foreground_lease_count=0`
- `lease_held=false`
- `capacity_exhausted=false`

## Cleanup SoT

The FSV closed both owned tabs and ended both session resources.

- Session A close: `true`
- Session B close: `true`
- Session A cleanup failures: `0`
- Session B cleanup failures: `0`
- Final claimed lanes: `0`
- Final active lanes: `0`
- Final target claims: `0`
- Final lease held: `false`

Independent post-run MCP readback from the live Codex session also reported one live Codex MCP session, `claim_count=0`, `claimed_target_lane_count=0`, `active_foreground_lane_count=0`, and `control_lease_status.held=false`.

## Acceptance Mapping

- Explicit target-source decision: the run used session-owned CDP targets for agent action/readback and Win32 foreground sampling only for human OS foreground evidence.
- Action/perception separation: `cdp_open_tab` reported human foreground before/after separately; `session_list` reported agent logical foreground lanes separately from `human_os_foreground`; all normal target actions reported `required_foreground=false`.
- Human-active FSV: foreground alternated between Notepad and VS Code and cursor moved through 28 unique positions during outstanding two-session target actions.
- No cross-talk: Session A and Session B retained distinct CDP targets and read back only their own markers.
- No break-glass foreground lease: lease was false during target work and after cleanup.
