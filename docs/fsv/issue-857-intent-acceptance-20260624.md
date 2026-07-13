# Issue 857 Intent Acceptance FSV - 2026-06-24

Issue: https://github.com/ChrisRoyse/Synapse/issues/857

> **Current D1 classification (2026-07-13):** The renamed script referenced
> below is supporting diagnostic automation only; it does not perform or accept
> FSV. Its output was not, and is not now, sufficient by itself for acceptance.
> The historical transcript values remain evidence from the separately observed
> manual run. Current acceptance requires an agent to use the strict production
> MCP client and independently read each physical Source of Truth before and
> after every manual trigger.

This transcript records the manual full-state verification for the intent
acceptance gate. The run used a fresh physical RocksDB store behind a real
HTTP MCP daemon, direct Streamable HTTP MCP calls, a real event subscription,
and physical store readback. The shared daemon was then rebuilt and restarted
from this checkout to verify the same Codex session can reconnect with the
expanded normal tool profile.

## Supporting Diagnostic Script

The retained supporting diagnostic is:

```powershell
.\scripts\diagnostics\issue-857-intent-diagnostic.ps1 `
  -SynapseMcpExe .\target\release\synapse-mcp.exe
```

The script starts an isolated temp daemon and DB, plants a routine library and
live activity, verifies `intent_current`, creates a real bus subscription,
drives `intent_detect_tick` through detected/abandoned/confirmed transitions,
records feedback outcomes, calls `session_end`, stops the daemon, and removes
the temp root. These checks are supporting diagnostics, not FSV acceptance.

## Code Surface

The normal profile now exposes the tools required to verify intent acceptance:

- `intent_current`
- `intent_detect_tick`
- `routine_feedback`
- `subscribe`
- `subscribe_cancel`

`subscribe` and `subscribe_cancel` are read-event tools and are classified as
safe by the MCP permission classifier. `intent_detect_tick` and
`routine_feedback` remain gated mutating/operational tools; they were not added
to the safe list.

## Isolated Daemon Transcript

FSV run:

- Timestamp marker: `issue857-fsv-20260624-144902`
- Temp daemon PID: `29792`
- Temp bind: `127.0.0.1:61411`
- Temp DB:
  `C:\Users\hotra\AppData\Local\Temp\synapse-issue857-fsv-20260624-144902\db`
- Implementation tool count: `216`
- Implementation tool surface SHA256:
  `37cc01612ca64f48d975db2850e08511ec9fc23192d37fa4021157bf14767b4c`
- Normal visible tool count: `173`
- Required tools present: `true`
- MCP session: `bf5f9418-cf36-4e20-a734-cf58f15b3a85`
- Latency budget: `2000` ms

Seeded source of truth:

- Training week Monday: `2026-06-15`
- Live replay day: `2026-06-22`
- Total marker rows inserted: `27`
- Initial marker search matches before final Teams completion rows: `25`
- Final scoped `timeline_search` marker matches: `27`
- Training routine: `outlook.exe -> excel.exe -> teams.exe`
- Non-routine activity: `notepad.exe`

Segmentation readback:

- Initial `episode_segment` episodes written: `18`
- Final live-day re-segmentation episodes written: `4`
- Final `episode_list` count: `19`

Mining readback:

- Routine ID: `rt1-f3f1abcf917c8a37`
- Steps: `outlook.exe`, `excel.exe`, `teams.exe`
- Support days: `5`
- Opportunity days: `5`
- Confidence: `0.5655175352168252`
- Schedule label: `weekdays 09:00+/-10m`

`intent_current` readback:

- Live replay latency: `33` ms
- Live candidates: `1`
- Top candidate routine: `rt1-f3f1abcf917c8a37`
- Matched prefix length: `2`
- Remaining step: `teams.exe`
- Unrelated notepad latency: `29` ms
- Unrelated notepad candidates: `0`
- Stale replay latency: `33` ms
- Stale replay candidates: `0`
- Matched episode ID: `ep1-2a5286a4c20efa45`
- Matched episode timeline refs: `2`

Bus/event readback:

- Subscription ID: `019efb2d-d087-7d91-89b8-090d526e3535`
- Detected tick latency: `27` ms
- Detected transitions: `detected`
- Detected events published: `1`
- Detected matched subscribers: `1`
- Abandoned tick latency: `28` ms
- Abandoned transitions: `abandoned`
- Abandoned events published: `1`
- Abandoned matched subscribers: `1`
- Confirmed tick latency: `29` ms
- Confirmed transitions: `detected`, `confirmed`
- Confirmed events published: `2`
- Confirmed matched subscribers: `2`
- Completed stale transitions: `0`
- Subscription cancelled: `true`

Feedback readback:

- Mined confidence: `0.5655175352168252`
- Abandoned feedback effective confidence:
  `0.5655175352168252`
- Declined feedback effective confidence: `0.0`
- Declined feedback suppressed: `true`
- Declined feedback cooldown remaining seconds: `3600`
- Accepted feedback effective confidence:
  `0.05345905446789677`
- Accepted feedback suppressed: `false`
- Accept count: `1`
- Decline count: `1`
- Abandon count: `1`
- Persisted feedback events: `3`

Physical store readback before cleanup:

- `CF_TIMELINE` rows: `28`
- `CF_EPISODES` rows: `19`
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

- Installed binary:
  `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`
- Installed binary SHA256:
  `E553E16C7569863EBE2BD218D2190926ABC6F0926BB24F826A48F51E0EB83B92`
- Candidate/shared tool count: `173`
- Candidate/shared tool surface SHA256:
  `bbadb49de3fff07adc5aa9d7f88b0c0428769b892aa665d4611d6997419d7375`
- New daemon PID: `17976`
- Chrome bridge after daemon start: `stale=false`, capability
  `pageScreenshot`
- Codex tool-surface snapshot written with tool count `173`

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
- PID: `17976`
- Tool count: `173`
- Tool surface SHA256:
  `bbadb49de3fff07adc5aa9d7f88b0c0428769b892aa665d4611d6997419d7375`
- Chrome bridge status: `ok`
- Chrome bridge active host:
  `chrome-native-0-1782331353723`
- `mcp__synapse.tool_profile_status` profile: `normal_agent`
- Normal visible tool count: `173`
- Normal profile hash:
  `sha256:1f4e49f53cf408ded618b094cf083f6b0a03c5db7338914fa55d6ca5c163632e`
- Visible tools include `intent_current`, `intent_detect_tick`,
  `routine_feedback`, `subscribe`, `subscribe_cancel`, `routine_mine`,
  `routine_update`, `episode_segment`, `episode_get`, and `timeline_search`.

Setup reported the current Codex process schema as stale but nonfatal. The
same-process MCP calls above verified reconnect without opening a new Codex
terminal.

## Verification Commands

- PowerShell parser check for
  `scripts/diagnostics/issue-857-intent-diagnostic.ps1`
- `cargo fmt --check`
- `git diff --check`
- `cargo test -p synapse-mcp --bin synapse-mcp tool_profiles -- --nocapture`
- `cargo test -p synapse-mcp --bin synapse-mcp permission_policy -- --nocapture`
- `cargo test -p synapse-mcp --test m3_intent_current_tool --test m3_intent_events_tool --test m3_subscribe_tool --test m3_tools_list -- --nocapture`

## Acceptance Mapping

- C1 Routine start ranking: PASS. Replayed live routine start produced exactly
  one `intent_current` candidate, top-ranked as
  `rt1-f3f1abcf917c8a37`, with matched prefix length `2` and remaining step
  `teams.exe`.
- C2 Honest negatives and abandonment: PASS. Unrelated `notepad.exe` activity
  and stale routine activity produced zero candidates. The detector then
  published one `abandoned` transition carrying the last-known matched prefix.
- C3 Bus events: PASS. A real event subscription was created before detector
  ticks. Detected, abandoned, and confirmed ticks published events with matched
  subscriber counts `1`, `1`, and `2`; the subscription was then cancelled.
- C4 Feedback-driven confidence movement: PASS. Abandoned feedback preserved
  mined confidence, declined feedback lowered effective confidence to `0.0`
  and set a 3600-second cooldown, and accepted feedback raised effective
  confidence to `0.05345905446789677` while clearing suppression.
- Latency: PASS. `intent_current` and `intent_detect_tick` replay calls were
  all below the stated `2000` ms budget; observed latencies were `27` to
  `33` ms in the FSV transcript.
- Physical Source of Truth: PASS. The script separately read back
  `episode_get`, `episode_list`, `routine_inspect`, `timeline_search`, and
  `storage_inspect` from the physical temp RocksDB store.
- Reconnect guarantee: PASS. The shared daemon was rebuilt, restarted, Chrome
  bridge was auto-verified, and this same Codex process called
  `mcp__synapse.health` and `mcp__synapse.tool_profile_status` successfully
  after the restart.
