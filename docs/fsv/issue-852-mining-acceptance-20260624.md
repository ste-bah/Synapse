# Issue 852 Mining Acceptance FSV - 2026-06-24

Issue: https://github.com/ChrisRoyse/Synapse/issues/852

> **Current D1 classification (2026-07-13):** The renamed script referenced
> below is supporting diagnostic automation only; it does not perform or accept
> FSV. Its output was not, and is not now, sufficient by itself for acceptance.
> The historical transcript values remain evidence from the separately observed
> manual run. Current acceptance requires an agent to use the strict production
> MCP client and independently read each physical Source of Truth before and
> after every manual trigger.

This transcript records the manual full-state verification run for the mining
acceptance issue. The planted routine was verified against a fresh physical
RocksDB store through the real HTTP MCP daemon surface, then the shared daemon
was rebuilt and restarted from this checkout to prove the same tools are
available to the live Codex-connected Synapse runtime.

## Supporting Diagnostic Script

The retained supporting diagnostic is:

```powershell
.\scripts\diagnostics\issue-852-mining-diagnostic.ps1 `
  -SynapseMcpExe .\target\release\synapse-mcp.exe
```

The script starts an isolated loopback daemon with a temp profile and temp DB,
uses Streamable HTTP MCP directly, seeds probe rows, verifies the full
timeline->episode->routine->digest chain, calls `session_end`, stops the temp
daemon, and removes the temp root in `finally`.

The historical diagnostic used direct HTTP because that Codex process had stale
generated schemas after tool-surface changes. Under current D1, that caller does
not establish strict production-client schema parity and cannot be an FSV
trigger; a freshly wired production MCP client is required for manual FSV.

## Code Surface

The normal tool profile now exposes the routine lifecycle and label-read tools
needed for the acceptance path:

- `routine_update`
- `routine_label_export`

`routine_label_export` is classified as a safe read-only MCP tool. The mutating
`routine_update` tool remains outside the safe list and is still gated as a
non-destructive mutating MCP call by the permission policy.

## Isolated Daemon Transcript

FSV run:

- Timestamp marker: `issue852-fsv-20260624-135800`
- Temp daemon PID: `15456`
- Temp bind: `127.0.0.1:59459`
- Temp DB: `C:\Users\hotra\AppData\Local\Temp\synapse-issue852-fsv-20260624-135800\db`
- Implementation tool count: `216`
- Implementation tool surface SHA256:
  `37cc01612ca64f48d975db2850e08511ec9fc23192d37fa4021157bf14767b4c`
- Normal visible tool count: `168`
- Required FSV tools present: `true`
- MCP session: `9bf4f901-f449-40ca-9280-e55ee0d5a996`
- Planted week Monday: `2026-06-15`
- Planted range:
  `1781499600000000000` through `1782104400000000000`

Seeded ground truth:

- Timeline rows inserted: `22`
- `timeline_search` marker matches: `22`
- Planted routine apps: `outlook.exe`, `excel.exe`, `teams.exe`
- Noise app: `spotify.exe`

Segmentation readback:

- `episode_segment` episodes written: `16`
- Episodes deleted: `0`
- Days processed: `5`
- Stop reason: `range_complete`
- `episode_list` count: `16`

Mining readback:

- Dry-run routines: `1`
- Dry-run routines written: `0`
- Real routines written: `1`
- Real routines deleted: `0`
- Active days: `5`
- Candidates evaluated: `14`
- Candidates rejected as subpattern: `11`
- Clusters rejected for low support: `2`
- Routine ID: `rt1-f3f1abcf917c8a37`
- Steps: `outlook.exe -> excel.exe -> teams.exe`
- Granularity: `app_document`
- DOW class: `weekdays`
- Schedule label: `weekdays 09:00+/-10m`
- Mean minute of day: `540`
- Tolerance minutes: `10`
- Support days: `5`
- Opportunity days: `5`
- Occurrence count: `5`
- Confidence: `0.5655175352168252`
- Evidence occurrences: `5`

Query, lifecycle, and labeling readback:

- `routine_list` returned: `1`
- `routine_inspect` before update lifecycle: `candidate`
- `routine_update action=confirm` lifecycle after: `confirmed`
- `routine_update action=rename` label after: `Morning report handoff FSV`
- `routine_inspect` after update lifecycle: `confirmed`
- `routine_inspect` after update label: `Morning report handoff FSV`
- Transition count: `3`
- `routine_label_export` current label: `Morning report handoff FSV`
- `routine_label_export` sample count: `3`
- Machine identity:
  `outlook.exe:inbox - outlook -> excel.exe:report.xlsx - excel -> teams.exe:chat - teams`
- Writeback hint names `routine_update`.

Every evidence occurrence returned three episode ids, and `episode_get` read
back every evidence episode with at least one physical timeline ref. The 15
evidence episode durations were five repetitions of:

- `outlook.exe`: `120000` ms
- `excel.exe`: `300000` ms
- `teams.exe`: `120000` ms

Digest readback:

- Routine day date: `2026-06-16`
- Routine day active ms: `540000`
- Routine day episode count: `3`
- Routine day routines touched: `1`
- Routine day matched evidence episodes: `3`
- Week anchor date: `2026-06-19`
- Week active ms: `3000000`
- Week episode count: `16`
- Week routines touched: `1`
- Week matched evidence episodes: `15`
- Week days covered: `7`

Physical store readback before cleanup:

- `CF_TIMELINE` rows: `23`
- `CF_EPISODES` rows: `16`
- `CF_ROUTINES` rows: `1`
- `CF_ROUTINE_STATE` rows: `1`
- `CF_SESSIONS` rows: `1`

The FSV session was explicitly ended. `session_end` reported
`failure_count=0`, `marked_terminated=true`, and `reason=explicit_session_end`.

## Shared Daemon Readback

After the isolated FSV passed, full setup was run from this checkout:

```powershell
.\scripts\synapse-setup.ps1 -SourceDir (Resolve-Path .) -ForceRestart
```

Setup readback:

- Built binary:
  `C:\Users\hotra\AppData\Local\synapse\build-target\synapse-a924ab647587\release\synapse-mcp.exe`
- Installed binary:
  `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`
- Installed binary SHA256:
  `4BCC520FED7A24FD1F04AA64D1C162053434BEFB19CF1A12583490FE52B66B2F`
- Candidate tool count: `168`
- Candidate tool surface SHA256:
  `66b77726e99b527e78bc1b87e8bc6dec9a8f6ca995515a7c8a0c17d6ddeee06d`
- New daemon PID: `30904`
- Chrome bridge status after daemon start: `stale=false`,
  capability `pageScreenshot`
- Codex tool-surface snapshot written with tool count `168`

Chrome bridge setup was auto-verified during handoff:

- Transport: `direct_localhost_websocket`
- Extension ID: `leoocgnkjnplbfdbklajepahofecgfbk`
- Bridge build:
  `synapse-chrome-bridge-2026-06-24-mousedown-click-v3`
- Active profile: `Profile 5`
- Active profile installed: `true`
- Auto-install attempted: `true`
- Auto-install reason:
  `existing_ready_extension_code_reload_deferred_to_daemon_reloadself`
- Popup shield:
  `HKCU:synapse_authored_popup_shield_applied`

Same-process MCP readback from this existing Codex session after daemon restart:

- `mcp__synapse.health`: `ok=true`
- PID: `30904`
- Tool count: `168`
- Tool surface SHA256:
  `66b77726e99b527e78bc1b87e8bc6dec9a8f6ca995515a7c8a0c17d6ddeee06d`
- Chrome bridge status: `ok`
- Chrome bridge active host:
  `chrome-native-0-1782328226379`
- `mcp__synapse.tool_profile_status` profile: `normal_agent`
- Normal visible tool count: `168`
- Normal profile hash:
  `sha256:a828588d5b55b59094dd51f334a2c34658c6b995784ea82ee88813dab8c30a86`
- Visible tools include `routine_mine`, `routine_list`, `routine_inspect`,
  `routine_update`, `routine_label_export`, `episode_segment`, `episode_list`,
  `episode_get`, and `timeline_digest`.

Setup reported the current Codex process schema as stale but nonfatal, and the
same-process MCP calls above verified reconnect without opening a new Codex
terminal.

## Acceptance Mapping

- B1 Seed/replay multi-day timeline: PASS. The FSV inserted 22 marked timeline
  rows for a Monday-Friday planted routine plus realistic noise, and
  `timeline_search` returned all 22 rows by marker.
- B2 Segmentation: PASS. `episode_segment` produced 16 physical episodes across
  5 processed days, and `episode_list` returned 16.
- B3 Mining: PASS. `routine_mine` found exactly one routine with the expected
  `outlook.exe -> excel.exe -> teams.exe` sequence, weekday schedule,
  09:00+/-10m timing, 5/5 support, and honest confidence `0.5655175352168252`.
- B4 Query and lifecycle: PASS. `routine_list`, `routine_inspect`, and
  `routine_update` confirmed and renamed the mined routine with persistent
  transition readback.
- B5 Labeling: PASS. `routine_label_export` returned the renamed label, three
  samples, the machine identity containing all planted apps, and a
  `routine_update` writeback hint.
- B6 Digest accuracy: PASS. `timeline_digest` reconciled with ground truth:
  the single planted routine day reported `540000` active ms and three matched
  routine episodes; the planted week reported `3000000` active ms, 16 total
  episodes including the noise episode, and 15 matched routine evidence
  episodes.
- Reconnect guarantee: PASS. The shared daemon was rebuilt, restarted, Chrome
  bridge was auto-verified, and this same Codex process called
  `mcp__synapse.health` and `mcp__synapse.tool_profile_status` successfully
  after the restart.
