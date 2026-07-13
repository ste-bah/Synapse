# Issue #1220 FSV: 50 Foreground-Equivalent Lanes With Human-Active Foreground

Date: 2026-06-24T18:35:12.7442179-05:00
Issue: https://github.com/ChrisRoyse/Synapse/issues/1220

> **Current D1 classification (2026-07-13):** The renamed script referenced
> below is supporting diagnostic automation only; it does not perform or accept
> FSV. Its output was not, and is not now, sufficient by itself for acceptance.
> The historical transcript values remain evidence from the separately observed
> manual run. Current acceptance requires an agent to use the strict production
> MCP client and independently read each physical Source of Truth before and
> after every manual trigger.

Command:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\diagnostics\issue-1220-foreground-lane-concurrency-diagnostic.ps1 -Count 50 -BatchSize 10
```

## Result

Pass. The run used real Synapse MCP HTTP sessions and tools against the live daemon and the already-open authenticated Chrome profile. Per the operator's current direction that there are no other active repo agents, this proof used 50 direct primary MCP sessions opened by this Codex run, not spawned subagents and not a paid remote worker swarm.

Run marker: `issue1220-lanes-20260624-182420`

## Live Substrate

- MCP daemon: `ok=true`, PID `8588`, tool count `173`
- Chrome bridge: `ok`
- Browser HWND: `524970`
- Observer session: `e02c4937-a0cc-41a5-bf2a-96da3f4ed2a4`
- Opened lane sessions: `50`
- First lane target: session `084daa88-55dd-46fa-952e-adc417cbba48`, target `chrome-tab:589709177`
- Fifth lane target: session `423a2906-2f99-430a-aa46-20d32e442973`, target `chrome-tab:589709181`

At the 50-lane capacity readback:

- `active_foreground_lane_count=50`
- `claimed_target_lane_count=50`
- `target_claim_status.claim_count=50`
- `capacity_exhausted=false`
- `control_lease_status.held=false`
- Mid-run claim renewal readback: `claim_count=50`

## Human-Active Foreground Evidence

The run drove real Win32 foreground and cursor changes while lane actions were outstanding.

- Foreground samples: `60`
- Distinct foreground HWNDs: `15860480` and `9635466`
- Distinct cursor positions: `60`
- Sample foregrounds alternated between Notepad (`Untitled - Notepad`) and VS Code (`Welcome - synapse - Visual Studio Code`)
- First samples:
  - `set-field-0`: Notepad, cursor `120,180`
  - `set-field-1`: VS Code, cursor `157,209`
  - `set-field-2`: Notepad, cursor `194,238`
  - `set-field-3`: VS Code, cursor `231,267`

## Action Mix

All routine lane work was target-owned and foreground-equivalent. The script asserts routine action responses did not require real OS foreground.

- `target_act set_field`: `50`
- `target_act press`: `50`
- `target_act read`: `50`
- `browser_evaluate`: `5`
- `cdp_navigate_tab` plus same-target evaluate: `4`
- `target_act screenshot`: `3`, all `required_foreground=false`
- `target_act run_shell`: `4`, all `required_foreground=false`

Artifact readback:

- `manual-fsv-tmp\issue-1220-20260624-182420\lane-01.png`: `161243` bytes
- `manual-fsv-tmp\issue-1220-20260624-182420\lane-18.png`: `162915` bytes
- `manual-fsv-tmp\issue-1220-20260624-182420\lane-35.png`: `163396` bytes
- Shell files for lanes `1`, `14`, `27`, and `40` each read back their own marker and no foreground requirement.

## Edge Coverage

- Concurrent lane acquisition pressure: 50 direct MCP sessions opened 50 distinct owned Chrome tab targets and held 50 live claims.
- Cross-target action: lane 1 attempted explicit evaluation against lane 2's target and received a tool error; `cross_target_denied=true`.
- Tool-profile refresh: lane 3 switched to `browser_control`, `tool_profile_status` reported `profile_preserves_capability=true`, then returned to `normal_agent`.
- Real foreground lease edge: normal `target_act focus_window` without lease was denied; lane 5 acquired a real foreground lease, switched to `break_glass`, `target_act focus_window` returned `status=ok`, and the lease was released.
- Disconnected client edge: lane 50 was closed through MCP session DELETE. Readback dropped to `claim_count=49` and `active_foreground_lane_count=49` before final cleanup.

## Cleanup SoT

The run closed/ended the remaining 49 lane sessions after the deliberate disconnect edge.

- Cleaned sessions: `49`
- Cleanup failures: `0`
- Final `active_foreground_lane_count=0`
- Final `claimed_target_lane_count=0`
- Final `target_claim_status.claim_count=0`
- Final `control_lease_status.held=false`

Independent post-run readback from the live Codex MCP session matched the cleanup: one live Codex MCP session, `target_session_count=0`, active lanes `0`, claimed lanes `0`, claims `0`, and no lease.

## Acceptance Mapping

- 50 foreground-equivalent lanes: passed with 50 real primary MCP sessions and 50 owned Chrome tab targets.
- Human active during run: passed with 60 Win32 foreground/cursor samples while target actions were outstanding.
- Mixed click/type/read/screenshot/browser/shell work: passed through `target_act`, `browser_evaluate`, `cdp_navigate_tab`, screenshots, and shell file readback.
- Separate SoTs: `session_list`, `target_claim_status`, `control_lease_status`, Win32 foreground/cursor, target DOM/readback, screenshot files, and shell files all matched.
- No cross-talk: each lane read back its own marker; sampled other-lane markers were absent; explicit cross-target evaluation was denied.
- Break-glass real foreground remains serialized: normal focus was denied without lease; leased break-glass focus succeeded and final lease state returned false.
