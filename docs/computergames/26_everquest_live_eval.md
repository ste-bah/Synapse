# 26 - EverQuest Live Evaluation

This document is the configured-host note for the current live game evaluation
target. It records operator-provided account/character state and the manual FSV
contract for evaluating Synapse against EverQuest.

This is not a script, benchmark harness, CI job, or automated FSV substitute.
Manual FSV must use the real Synapse MCP runtime surface, the visible
EverQuest client, local client logs/config files, Windows process/window state,
and Synapse storage/audit rows as the sources of truth.
Before every EverQuest FSV action cluster, the agent must prove
`synapse-mcp` is running and active: process or stdio child, loopback transport
or client state, authenticated `health`, initialized MCP session, and
`tools/list` containing the EverQuest tool being used. If the daemon is missing
or stale, launch or reinstall the repo-built runtime first. EverQuest behavior
must be triggered through the real MCP tool (`observe`, `act_keymap`,
`everquest_current_state`, `storage_inspect`, etc.) whenever such a tool exists,
then verified by a separate read of the visible client, EQ logs/config, Windows
process/window state, and Synapse storage rows.

## Current Character State

Operator-provided state on 2026-05-28:

| Field | Value |
|---|---|
| Game | EverQuest |
| Server | Frostreaver |
| Character | Thenumberone |
| Race | Dark Elf |
| Class | Wizard |
| Starting level | 1 |
| Starting location | Neriak |
| Evaluation goal | Reach level 2 through live in-game play |

Before claiming progress, the agent must verify the state directly from the
live client using independent readbacks.

## Scope

The EverQuest target uses the existing `operator_owned_test` profile enum for
this configured host only, with explicit `operator_attended_required` metadata.
Actions are allowed only while the foreground process/window belongs to the
operator-authenticated `eqgame.exe` session and the operator-owned character is
visible. The operator is present and prompting the agent; this is not an
unattended loop, bot farm, or background account operation.

The allowed evaluation surface is narrow:

- Observe the visible client and read local EverQuest files/logs.
- Use normal keyboard/mouse input through Synapse action tools.
- Act only from active operator prompts and visible in-game state.
- Play the fresh level 1 character long enough to prove the system can reach
  level 2.
- Record profile/audit evidence through Synapse storage.
- Permit only individually prompted foreground action surfaces for the live
  profile. Broad text entry, clipboard mutation, shell/launch, scheduled combo,
  and reflex registration are denied because they can drive chat,
  social/economy/account, destructive UI, or background automation paths.

The following are out of scope:

- Process memory reads/writes, packet inspection, protocol hooks, DLL
  injection, or graphics injection.
- Chat, trade, auction, economy, group, guild, raid, PvP, or social
  automation.
- Scaled unattended account operation or multi-account operation.
- Credential, billing, subscription, marketplace, or account-management
  actions.
- Destructive UI actions such as deleting characters/items, changing account
  settings, or accepting irreversible dialogs.
- Claiming level-up success from an action return value alone.

## Sources Of Truth

Manual FSV must read these surfaces before and after each trigger:

| SoT | Expected evidence |
|---|---|
| Synapse MCP daemon | `synapse-mcp` PID/command line or stdio child, authenticated `health`, initialized session, and `tools/list` containing the tool used |
| Windows process table | `eqgame.exe` PID and command line from the Daybreak install |
| Foreground window state | title/class/HWND/process path for the active EverQuest window |
| Visible game UI | OCR/screenshot evidence for character/server/zone/level/XP or UI text |
| EverQuest install/log files | relevant same-day logs/config files under the Daybreak install |
| EverQuest map files | base `maps\*.txt`, optional map-set subdirectories such as `maps\Brewall`, and Synapse map-pack archive/manifest cache |
| Synapse profile state | `profile_list`, `profile_activate`, `observe` foreground `profile_id=everquest.live` |
| Synapse action/audit state | `storage_inspect` row counts/samples for `CF_ACTION_LOG`, observations, events, profile quality |

The level-2 acceptance source of truth is visible in-game character state:
the game UI must show the character at level 2 after the run, with separate
process/window/log/audit readback.

## HUD Readback Contract

The current configured-host level source is the visible Inventory character
panel opened with `i`. On 2026-05-28 manual readback, that panel showed
`Thenumberone`, `1 Wizard`, and `NEXT LEVEL 0.000%`.

`everquest.level_text` is intentionally bounded to the Inventory panel instead
of the whole EverQuest window. A whole-window HUD OCR pass can resolve to the
UIA window title `EverQuest`, which is not character state and must not be used
as a level claim. The Inventory crop is deliberately broad enough for the
current windowed placement and the earlier fullscreen placement; the parser
looks for the name/class line such as `1 Wizard`. `everquest.next_level_label`
proves the XP panel is visible; the numeric XP percentage must still be read
from the visible crop/screenshot unless OCR agrees with the crop. If OCR
returns an ambiguous or contradictory XP percentage, the agent must treat the
visible crop as the SoT and record the OCR mismatch on #495/#500.

## Safe Input Aliases

The EverQuest profile owns reviewed keyboard aliases in its `[keymap]` table:
movement (`forward`, `back`, `left`, `right`, `turn_left`, `turn_right`),
targeting/consider (`target_nearest_npc`, `target_self`, `con`, `consider`),
UI recovery (`inventory`, `spellbook`, `menu`, `sit`), and
hotbar slots (`hotbar1` through `hotbar10`). Runtime action work should prefer
`act_keymap` for those aliases so the action audit row records both the
semantic alias and the resolved key/chord. Direct `act_press` remains valid for
explicit edge checks, but the profile-keymap path is the normal EverQuest
action surface for manual FSV.

`open_chat` is not a reviewed live alias. Enter/chat focus caused a real
accidental public-chat side effect during #496 recovery, so the live profile
must fail closed for `act_keymap open_chat`. Chat content entry through
`act_type`, mutating clipboard, or other text-injection paths is not a supported
live EverQuest action surface. If chat focus is accidentally entered, stop,
read the visible UI/log SoT, and do not continue movement/combat while ordinary
keys are being captured by chat. The verified recovery for an already-polluted
chat buffer is bounded Backspace clearing, a single audited Enter only after the
buffer is expected empty, then `inventory` readback; the EQ log `You say` count
and timestamp must be read before and after to prove no chat was submitted.
Do not expose this as `open_chat` or any general chat/recovery alias.

`everquest_loc_probe` and `everquest_safe_command` are the only reviewed live
text-like EverQuest command surfaces. `everquest_loc_probe` takes no command
string and no free-text parameters; it emits only the literal `/loc` key
sequence after foreground/profile/logging preconditions and after #524's
visible chat input pollution gate reads trusted `everquest.chat_input_state`
with `text_present=false`. `everquest_safe_command` is narrower than
`act_type`, but broader than `/loc`: it accepts only enum-selected survival
commands (`sit_on`, `sit_off`, `stand`) and emits the corresponding literal
slash command (`/sit on`, `/sit off`, `/stand`) through the same gate.

The gate reads the active `UI_<character>_<server>_<class>.ini` `[MainChat]`
layout, foreground window bounds, OCR-scored `Windowed`/resolution/scaled
resolution coordinate candidates, and a WinRT OCR crop of the bottom chat input
strip; unsafe states fail closed before key emission and are recorded in
`CF_ACTION_LOG`. `/loc` proves the result by reading the physical EQ log tail
for a new coordinate line. Safe survival commands prove only that no `You say`
pollution was emitted; manual FSV must separately read the visible
Inventory/Player-window HP, mana, and posture state. Unknown parameters,
disabled `Log=0`, non-EQ foreground, visible unsent chat text, untrusted chat
state, missing/malformed location output for `/loc`, or any player-say output
are hard failures.

`everquest_survival_readiness` is the read-only #535 survival SoT. It sends no
gameplay input. It writes
`CF_KV/everquest/survival_readiness/v1/everquest.live/latest` from current
foreground/profile state, `everquest.chat_input_state`, visible HUD HP/mana
text, and the physical EQ log tail. Food/drink absence is detected from the
physical `You are out of food and drink` log signal; positive item counting from
bags/merchant windows is not implied. Merchant/economy/item acquisition remains
outside the supported live surface unless a future issue carries explicit
operator approval and separate manual FSV.

Manual FSV for the chat gate must read the physical UI file, visible OCR crop,
EQ log `You say` count, and `CF_ACTION_LOG` rows before and after. Required
edges are: visible unsent buffer text denies before key emission; invisible or
low-confidence OCR fails closed; layout/foreground disagreement fails closed;
and the post-trigger `You say` detector still catches pollution if the preflight
is bypassed by an unexpected game state.

`everquest_current_state` is the compact world-state bridge for #510. It does
not send gameplay input. It reads foreground/profile/HUD state, the active EQ
log tail, latest zone and `/loc` events, local map landmarks, and recent
EverQuest-linked Synapse action-audit rows, then writes the physical storage row
`CF_KV/everquest/current_state/v1/everquest.live` and reads that row back. This
row is the short-lived state packet for planners and future world-model
injection: zone, map-order location, nearest landmarks, level, target/consider,
latest actions, and hazards with source pointers and confidence. Manual FSV for
the tool still requires an independent post-trigger storage readback; the
returned readback is supporting evidence, not the only verdict.

#517 adds the foreground-stabilization contract for accepted EverQuest action
candidates. Before `act_press`, `act_keymap`, mouse, scroll, or pad input is
emitted under the active `everquest.live` profile, the MCP action path reads
the current foreground, verifies or restores the configured `eqgame.exe`
window, then reads the foreground again. The readback includes whether the HWND
is minimized; a minimized `eqgame.exe` is restored before accepted dispatch,
and remains fail-closed if the post-refocus state is still minimized or
unknown. The `CF_ACTION_LOG` started row stores the `details.preflight` proof
with before/after HWND/process/path/title/minimized state, candidate count,
focus attempt status, and final preflight state. If a non-EverQuest window
remains foreground, if `eqgame.exe` is missing, if minimized state cannot be
proved usable, or if Windows refuses a safe refocus/readback, the action fails
closed and must not be counted as gameplay progress.

Before claiming an alias effect, manually read the visible UI/log/storage SoT
before the trigger, call the real MCP `act_keymap` tool while `eqgame.exe` is
foreground, then separately read the visible UI/log/storage state again. The
`CF_ACTION_LOG` row must show `tool=act_keymap`, requested alias, resolved
binding/key list, backend, hold duration, foreground `eqgame.exe`,
`details.preflight.status` (`verified_foreground` or `refocused_and_verified`
for accepted actions), and the allow/deny/error status.

## Log Pipeline

EverQuest already creates local activity logs on this host. Current readback:

| File | Current role |
|---|---|
| `C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\Logs\eqlog_Thenumberone_frostreaver.txt` | Same-day in-game activity log |
| `C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\eqclient.ini` | Client config; `Log=1`; `LastCharSel` may read `Thenumberone` or `0` and must be treated as unknown when numeric |
| `C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\Thenumberone_frostreaver_WIZ.ini` | Character config |
| `C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\UI_Thenumberone_frostreaver_WIZ.ini` | Character UI layout |

The `synapse-everquest` crate owns EverQuest-specific log discovery, parsing,
cursor-based tailing, and token-efficient summaries. It must turn noisy raw log
lines into compact events such as `zone_entered`, `target_npc`, `consider`,
`cast_begins`, `location`, `tell`, `say`, and `system` while suppressing long
chat bodies unless a later operator-approved feature explicitly needs them. Runtime
integration should feed these compact events into `observe`, event streams,
profile quality, and audit storage without dumping full raw logs into the model
context.

The MCP runtime log feed is exposed through `observe(include=["events"])` when
the foreground profile resolves to `everquest.live`. The first observation for
a newly discovered log initializes the cursor at the current end of file and
reports `everquest.log_cursor_initialized` so old log content is not replayed.
Subsequent observations report `everquest.log_cursor` plus compact
`everquest.log.*` summaries for new bytes. Disabled logging (`Log=0`), missing
`Logs` directories, malformed timestamp lines, and cursor offsets beyond the
current file length fail closed as explicit filesystem events or diagnostics;
they are not silent fallbacks. Chat-like events must carry actor/channel/summary
metadata only and mark the body redacted by default.

The current-state path reads the same compact log stream in bounded tails rather
than loading full raw logs into the model context. Its durable row is
`CF_KV/everquest/current_state/v1/everquest.live`; downstream route planners,
world-model context injection, surprise detection, and scorecards should read
that row or derived storage rows instead of rereading unbounded log text.

## Delta Reality For EverQuest

Issue #536 changes the EverQuest context strategy from repeated full-state
summaries to a baseline plus deltas:

- Baseline: foreground `eqgame.exe`, character/server/profile, zone,
  latest `/loc`, HUD level/XP/resources, chat-input state, log cursor, nearest
  map landmarks, and current world-model head rows.
- Deltas: new EQ log lines, log cursor movement, zone entered, `/loc` changed,
  HUD field changed, chat-input state changed, action accepted/denied, target or
  con changed, route/surprise/memory rows written, or profile/audit state
  changed.
- Audit: periodically re-read the visible client, physical EQ log/config/map
  files, Windows process/window state, and Synapse `CF_KV`/audit rows. Compare
  those physical SoTs against the baseline+delta assumption. Persist drift and
  force a new baseline before continuing movement/combat if reality disagrees.

The level-2 run should use delta context between audits once #537-#542 are
implemented. Until then, use existing real MCP tools (`observe`,
`everquest_current_state`, `everquest_world_summary`, `storage_inspect`) plus
manual before/after SoT readback.

## Current-Map Sensor Rows

#525 adds `everquest_map_sensor`, the runtime surface for turning visible map
evidence into compact map-calibration/readback rows. The tool reads the
persisted current-state row, visible-map evidence from an observe/screenshot
readback, and the local `maps/*.txt` file for the current zone, then writes:

- `CF_KV/everquest/map_sensor/v1/everquest.live/<sensor_id>` for calibrated or
  fail-closed current-map sensor state.

Rows contain the foreground EQ window identity, visible map bounds/confidence,
current `/loc`, map file SHA-256/mtime/counts, nearest labels and exits,
visible label or player-marker anchors, transform confidence, hazards, and
source refs. Hidden maps, occlusion, stale current state, missing `/loc`,
non-EQ foreground, zoom/pan changes after calibration, low visible confidence,
or contradictory zone sources persist abstain rows instead of guessed
calibration. Map-sensor rows do not execute movement.

Manual FSV for map sensing reads the physical screenshot/observe crop, physical
EQ log/current-state row, and local map file before the trigger, calls the real
MCP tool, then separately reads the persisted `CF_KV` map-sensor row.

## Map Data Provenance

#520 adds a local map inventory/provenance surface for the configured host:

- `eq-map-inventory --root <everquest-install-root> [--set <name>]`

This is a real local CLI for map-pack state, not an FSV substitute. It reads
physical map files, computes per-file and aggregate SHA-256 hashes, parses each
`.txt` map with `synapse-everquest`, reports skipped/corrupt files and duplicate
label samples, and can write a JSON provenance manifest. Manual FSV still reads
the map directory, archive, and manifest bytes independently before and after
any acquisition or install action.

Current host readback on 2026-05-29:

| Surface | Readback |
|---|---|
| Base EQ maps | `C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\maps`; 121 top-level `.txt` files; aggregate SHA-256 `sha256:3d5eed00f56f089f6f7aaf6f77893a24dcc3e16e08d0dbf85fa3ebf425769412` |
| Base parse status | 120 parseable maps; `kael.txt` skipped because line 4050 has an empty point label |
| Community source page | Brewall EQ Maps: `https://www.eqmaps.info/eq-map-files/` |
| Preserved archive | `%APPDATA%\Synapse\everquest\map_packs\archives\brewall-20240109.zip`; SHA-256 `sha256:9b8d17e3e1058ceb7276b1d1e6fcee46b9930ba8bf68450bcb572d9641b5305d` |
| Installed map set | `C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\maps\Brewall`; 1,707 `.txt` files; base top-level count remained 121 |
| Brewall inventory | 1,686 parseable maps, 21 skipped maps, aggregate SHA-256 `sha256:068125aabd59fe8472ca35fc891f5e639ab075c722464beb0bc3c1997bab6434` |
| Manifest SoT | `%APPDATA%\Synapse\everquest\map_packs\manifests\brewall-20240109.json`; records source URLs, archive path/hash, file counts, skipped files, duplicate label samples, `base_maps_overwritten=false`, and rollback target |
| Rollback | Delete `maps\Brewall` to remove the community map set; do not delete or overwrite base `maps\*.txt` |

Community map packs must be installed in a separate map-set directory or a
Synapse cache unless an issue explicitly approves replacing base files. Before
any replacement-style workflow, first preserve a backup plus manifest of the
original files, then read both backup and target directories directly. The map
window can select a custom map set, but world-model code must record which set
was used before treating labels or route hints as SoT.

Manual FSV for map data uses these edges: unavailable download/source URL,
corrupt archive or corrupt map file, duplicate/stale labels, and permission or
write-target failure. Each edge must print the source directory/archive/manifest
state before and after the trigger and verify that base maps were not silently
modified.

## Compact Outcome Rows

#526 adds `everquest_outcome_ingest`, the runtime storage surface for compact,
redacted EverQuest log outcomes. The tool reads bounded log bytes from the
active `everquest.live` log or an explicitly approved
`eqlog_<character>_<server>.txt` file, then writes deterministic rows under
`CF_KV/everquest/outcome_event/v1/everquest.live/<offset>-<hash>`.

Each row stores source path, byte offsets, line index in the read window,
timestamp text, parsed timestamp when available, source-line SHA-256, compact
outcome kind, confidence, diagnostic code, and redaction evidence. The taxonomy
covers combat damage dealt/taken, spell begin/hit/fizzle/resist, XP/level,
death/respawn, loot, rest/sit, target/consider, zone/location, hazard signals,
chat-redacted lines, ambiguous combat, unknowns, and malformed/missing timestamp
diagnostics. Raw chat bodies are never persisted.

These rows feed trajectories, surprise detection, hazard/safe-area memory,
planner guards, and scorecards without replaying stale raw log text into model
context. Manual FSV still reads physical log bytes before the trigger, invokes
the real MCP tool, and separately reads the persisted `CF_KV` rows afterward.

## Hazard And Safe-Area Memory Rows

#528 adds durable memory rows that let the planner avoid repeating known
EverQuest failures while still allowing supervised progress when safe evidence
exists. The memory writer stores:

- `CF_KV/everquest/hazard_memory/v1/everquest.live/<memory_id>` for high-risk
  targets, death locations, unexpected zone transitions, aggro/damage clusters,
  and low-confidence focus/location zones.
- `CF_KV/everquest/safe_area_memory/v1/everquest.live/<memory_id>` for safe
  recovery areas, successful rest/recovery, low-risk con evidence, or
  confirmed level-appropriate route outcomes.
- `CF_KV/everquest/planner_consult/v1/everquest.live/<candidate_id>` for the
  planner's readback decision after consulting active hazard/safe rows.

Each memory row includes schema version, memory type/kind, subject, optional
zone/location/radius, confidence, source state key/time, compact source refs,
duplicate marker, stale-source status, conflict downgrade state, and redaction
evidence. Stale source state caps confidence and disables planner use until
refreshed. Conflicting later evidence lowers confidence instead of deleting the
older hazard, preserving the learning trail.

Planner consult rows match active memories by target, zone, or location radius.
An active matching hazard returns `avoid`; a matching safe area with no active
hazard returns `allow_with_safe_memory`; unknown candidate state returns
`abstain_state_unknown`. These rows are action-prior inputs, not autonomous
gameplay. Manual FSV still reads physical EQ logs/UI/storage before and after
the runtime trigger, and raw chat bodies must not be persisted.

## Local Route-Plan Rows

#527 adds a bounded local route planner for map-backed movement planning before
any movement key is pressed. The planner reads the persisted current-state row
from `CF_KV/everquest/current_state/v1/everquest.live`, builds the local map
graph from the configured EverQuest install, resolves a target label or zone
line in the current zone, and writes:

- `CF_KV/everquest/route_plan/v1/everquest.live/<plan_id>` for the compact
  route plan or abstain decision.

Ready route rows contain the source state key, target label/zone, current and
target map coordinates, nearest labels, distance, route confidence, source map
line provenance, and guard requirements such as foreground verification,
world-focus verification, `/loc` readback before each step, bounded step probes,
and re-planning after zone changes or surprise. Unknown zone, missing `/loc`,
absent target labels, stale state, or conflicting map calibration persist
abstain rows instead of guessing movement.

Manual FSV for route planning reads the physical map file line and current
state before the trigger, calls the real MCP tool, then separately reads the
route-plan row through storage readback. Route rows do not execute movement.

## Planner Guard-Decision Rows

#514 adds the bounded candidate guard surface that must be read before live
movement or combat inputs. The tool writes:

- `CF_KV/everquest/planner_guard_decision/v1/everquest.live/<decision_id>` for
  one selected or rejected candidate action.

`everquest_planner_guard` reads live foreground/profile state, visible
chat-input pollution state, and the persisted current-state row before
evaluating a candidate. Selected candidates require `eqgame.exe` foreground,
active `everquest.live`, empty visible chat input, current-state availability,
known zone, and candidate-specific guards. Combat candidates are stricter:
only the verified `hotbar4` Blast of Cold attack spell can be selected, the
level-1 wizard target must be level-1-safe, and gamble/high-risk con text
rejects the candidate. #518 also requires explicit health, mana, and casting
posture readiness evidence before any `combat_spell` candidate can be
selected; missing or low-confidence readiness rejects before input.

The guard tool never executes input. It produces the planner verdict and
source refs only. Manual FSV for any later action still reads physical SoT
before the trigger, reads the persisted guard-decision row, executes the real
bounded foreground action manually, and then separately reads the EQ UI/log and
Synapse storage/audit SoTs afterward.

## DynamicJEPA Domain Rows

#511 adds `everquest_domain_normalize`, the runtime surface that turns one
observed EverQuest state/action/outcome cluster into ContextGraph-compatible
typed rows. The tool writes and reads back:

- `CF_KV/everquest/dynamicjepa_domain_pack/v1/everquest.live/everquest_dynamicjepa_v1`
- `CF_KV/everquest/dynamicjepa_state/v1/everquest.live/<transition_id>`
- `CF_KV/everquest/dynamicjepa_action/v1/everquest.live/<transition_id>`
- `CF_KV/everquest/dynamicjepa_outcome/v1/everquest.live/<transition_id>`
- `CF_KV/everquest/dynamicjepa_transition/v1/everquest.live/<transition_id>`

The domain pack defines the state, action, outcome, and entity fields used by
trajectory/export work: zone, local coordinate buckets, heading, level/xp,
target/con/resource/UI focus, action kind/tool/alias/duration/origin, next
zone/coord, log event kind, damage/death/xp/UI deltas, surprise, character,
server, trajectory, and session. It also records planner candidates, guard
names, surprise threshold, and the ContextGraph-compatible CF names.

Fatal invariants are persisted with every transition: zone-entry log must
update zone, movement/combat requires live EverQuest foreground, combat must
pass con-safe level-1 target/resource evidence, chat/social/economy actions are
never planner-eligible, and zone changes cannot be inferred without a
zone-entry log. Missing required fields and invalid categorical variants fail
before storage mutation; denied unsafe actions persist as denied rows.

Manual FSV must read physical EQ UI/log/action/storage state before the
trigger, call the real MCP tool with a known action/log/observe cluster, and
separately inspect all five durable `CF_KV` rows afterward. This is a domain
pack/storage surface, not a training script or FSV substitute.

## Trajectory Rows

#512 adds `everquest_trajectory_record`, the runtime surface that links real
Synapse audit rows and EQ log byte ranges into ordered, token-efficient
trajectory evidence. The tool writes:

- `CF_KV/everquest/trajectory/v1/everquest.live/<trajectory_id>`
- A local JSONL provenance artifact under the Synapse EverQuest trajectory
  export directory when `export_jsonl=true`.

Each transition must point at existing physical evidence: the durable
current-state row, one or more `CF_ACTION_LOG`, `CF_OBSERVATIONS`, and
`CF_EVENTS` rows, plus bounded EQ log offsets with optional byte-range hashes.
Optional links can include DynamicJEPA domain-transition rows, outcome rows,
planner guard rows, and map-state rows. Missing refs, duplicate transition
ids, out-of-order timestamps, bad log offsets, and log hash mismatches fail
closed before new storage mutation. Duplicate trajectory ids return the
already-stored row without rewriting it.

Trajectory rows are the bridge from attended gameplay evidence to later
ContextGraph/DynamicJEPA exports and model scorecards. They are not scripts,
training jobs, or FSV substitutes. Manual FSV must still read the physical
storage rows and export file before and after the real MCP trigger.

## ContextGraph Episode Export

#521 adds `everquest_episode_export`, the runtime bridge from Synapse's
trajectory/domain rows into ContextGraph-compatible DynamicJEPA episode JSONL.
The tool reads existing `CF_KV` trajectory and DynamicJEPA state/action/outcome
rows, refuses incomplete or unredacted sources, writes a local JSONL artifact,
and reads the final artifact bytes back before reporting success.

Each exported line carries `source_of_truth`, `state`, `action`, `outcome`,
`transition`, `expected_persisted_delta`, and `actual_readback` blocks. Source
refs include the Synapse row keys and hashes, current-state row key, linked
action/observation/event/log refs, issue refs, and ContextGraph-compatible CF
names. Raw chat bodies, raw target names, and raw session ids are excluded; the
session id appears only as a SHA-256 hash inside the transition entity block.

Manual FSV for this bridge must call the real MCP tool, then separately inspect
the physical `CF_KV` source rows and the JSONL file on disk. The export is a
planning/model evidence surface and does not prove movement, combat, or level
progress by itself.

## World-Model Prefix Rows

#513 adds durable, compact EverQuest world-model storage prefixes and readback
surfaces without bloating generic observation rows. The runtime writer stores
only approved `CF_KV` keys:

- `everquest/map/v1/everquest.live/<row_id>`
- `everquest/zone_graph/v1/everquest.live/<row_id>`
- `everquest/state/v1/everquest.live/<row_id>`
- `everquest/transition/v1/everquest.live/<row_id>`
- `everquest/trajectory/v1/everquest.live/<row_id>`
- `everquest/planner/v1/everquest.live/<row_id>`
- `everquest/surprise/v1/everquest.live/<row_id>`

Every row carries schema version, profile id, world-model kind, row key,
created/updated timestamps, revision, payload hash/length, compact source refs,
redaction flags, retention class, caps, and an evidence boundary saying manual
runtime FSV is still required. Strategic rows have no TTL and are pressure
preserved; episode rows use 30-day TTL; scratch rows use 24-hour TTL.

`everquest_world_model_inspect` is the compact readback surface for counts,
selected keys, and redacted samples from those prefixes. It is meant for
planners, ContextGraph/DynamicJEPA export, surprise detection, and manual FSV
evidence. Raw chat/message payloads and raw target-name style data must be
rejected before storage.

Learned physical zone-transition volumes live under
`everquest/transition/v1/everquest.live/<row_id>`. Planner-eligible payloads
must be compact/redacted and include `verification_status="verified_zone_entry"`,
`from_zone_short_name`, `to_zone_short_name`, optional `label`, complete
`pre_zone_location` and `post_zone_location` coordinates (`map_x/map_y/map_z`),
optional `action_cluster`, and `confidence >= 0.70`. The source refs must point
at the physical EQ log zone-entry line, pre/post `/loc` evidence, and any guard,
route, action, or current-state rows used to reconstruct the crossing.
`everquest_route_plan` prefers these verified transition rows over static
zone-line labels; static labels remain approach hints only until the log and
current-state SoT prove the crossing.

Manual FSV for these rows reads `CF_KV` before the trigger, calls the real MCP
tool with known synthetic world-model data, then separately reads the selected
key, prefix counts, and storage/WAL state afterward. These rows are not
movement, combat, level-progress, or gameplay-success proof by themselves.

## Surprise Detector Rows

#515 adds `everquest_surprise_detect`, a compact runtime detector that compares
predicted route/action outcomes against observed EQ log, UI/current-state, or
outcome evidence. It writes compatible world-model rows at
`CF_KV/everquest/surprise/v1/everquest.live/<surprise_id>`.

The row records the prediction, observed state/log summary, compared fields,
divergence score, threshold, mismatch reasons, remediation steps, and
`stop_condition`. Unexpected zone entries, action outcomes, or state changes
therefore become first-class repair evidence instead of blind movement
continuing after the world model is wrong.

Missing prediction, stale log/current-state evidence, false OCR-style zone
evidence, and low-confidence state all fail closed as stop/repair rows. The
tool executes no input. Manual FSV must read the physical EQ log/current-state
storage before the trigger, call the real MCP tool, then separately read
`everquest_world_model_inspect`, `storage_inspect`, and DB/WAL bytes afterward.

## World Summary Context Rows

#516 adds `everquest_world_summary`, the compact context-injection surface that
agents should keep in context after compaction instead of dumping raw EQ logs or
full map files. It writes
`CF_KV/everquest/world_summary/v1/everquest.live/<summary_id>` with bounded
current zone/position confidence, nearest exits and landmarks, recent
transitions, safe next probes, level state, focus state, hazards, blockers,
source refs, and compaction recovery links to #501, #500, and #505.

The default path reads the persisted `everquest_current_state` row and local EQ
map graph. `state_override` exists for synthetic manual FSV with known inputs,
but every source ref still has to point at a physical SoT such as an EQ map
line, log cursor, storage row, or issue comment. The row executes no input and
persists no raw chat bodies. Unknown zone, missing map graph, stale state,
non-EQ foreground, and low-confidence zone/location state persist explicit
blockers and safe next probes instead of allowing blind movement.

Manual FSV for this row reads physical map/log/current-state/storage before the
trigger, calls the real MCP tool, then separately reads `storage_inspect` and
DB/WAL bytes for the exact summary key. The required happy path is the current
Neriak context reporting level 1, `neriaka`, and the `to_Nektulos_Forest`
candidate without raw chat bodies. Required edges are unknown zone, map missing,
stale state, and chat redaction.

## Predictive Model Rows

#522 adds the transparent local predictive-model bridge from verified
EverQuest trajectories into a calibrated action prior. `everquest_predictive_model_fit`
reads #512 trajectory rows plus linked #511 DynamicJEPA state/action/outcome
rows, fits an action-conditioned Markov baseline, writes
`CF_KV/everquest/predictive_model/v1/everquest.live/<model_id>`, and reads the
model row back with a stable hash. `everquest_predictive_model_predict` reads
that model plus a current DynamicJEPA state row, ranks candidate actions, writes
`CF_KV/everquest/prediction/v1/everquest.live/<prediction_id>`, and reads the
prediction row back.

The first model is intentionally transparent: exact state-action entries first,
then action fallback, then global fallback. Confidence is winning-count divided
by sample-count for the bucket. The row preserves source trajectory keys,
source transition keys, conflict counts, thresholds, limitations, and an
evidence boundary saying the model supports attended planning only. The 0.60
minimum confidence is the useful supervised floor, not an optimization ceiling.

Manual FSV for this bridge must read `CF_KV` before the trigger, call the real
MCP tools with known trajectory/state/candidate inputs, then separately inspect
the persisted model and prediction rows. The happy path must compare one
prediction to a later observed outcome by recording a real action-prior sample.
Required edges are no verified data, conflicting outcomes, stale model hash, and
uncertainty above threshold.

## Action-Prior Scorecard Rows

#531 adds the runtime storage surface for measuring whether the EverQuest world
model is becoming useful during supervised play. The sample tool writes
`CF_KV/everquest/action_prior_eval/v1/everquest.live/<sample_id>` and the
scorecard tool writes
`CF_KV/everquest/action_prior_scorecard/v1/everquest.live/<window_id>`.

Each eval sample stores a redacted prediction, the actual observed outcome,
source refs, limitations, confidence, abstention flag, and computed correctness
class. The scorecard aggregates named samples into top-1, top-3, zone,
coordinate-bucket, hazard-avoidance, useful-accuracy, abstention, surprise,
overconfident-wrong, low-confidence-action, and calibration-bucket metrics. It
also stores sample-record-time window bounds and the aggregate source episode
ids from the eval rows.
The default minimum competence floor is `0.60`; the default stretch target is
`0.80`. These numbers are a floor, not a ceiling. The scorecard must continue
reporting the best verified performance the system can honestly support as the
profile registry, action audit, map memories, route planner, and ContextGraph
exports accumulate better trajectories.

Scorecards are planning-quality evidence only. They do not prove runtime game
state, route success, level progress, or action safety. Manual FSV still reads
the physical source of truth before and after the real trigger: visible game
state, EQ logs, Windows foreground/process state, and Synapse storage rows.
Empty or tiny windows must produce `no_verified_trajectories` or
`insufficient_samples`, and low-confidence states should abstain instead of
forcing input. A non-abstaining action below `min_confidence_for_action`
records `low_confidence_action_forced` and does not meet the competence floor.

## Profile Registry / Audit Moat Rows

`profile_quality_refresh` is the durable profile-registry/audit-data bridge for
EverQuest. The physical row is
`CF_PROFILES/profile_quality/v1/everquest.live`. It aggregates bounded action
audit rows, observation rows, and event rows into source counts, app identity,
event-kind counts, EverQuest log-kind counts, quality score inputs, and an
optional `manual_fsv_evidence_ref` pointing to the GitHub issue comment that
contains manual source-of-truth readback.

For EverQuest, the row must link:

- `profile_id=everquest.live`
- app identity such as `eqgame.exe` and `registry.compatibility_target`
- compact event kinds such as `perception.observed` and `everquest.log.*`
- manual FSV evidence comment/id for the runtime verification
- bounded outcome metadata only

It must not store/export raw player chat bodies, private session tickets, full
window titles, process paths, or full raw log lines by default.

## GitHub Issue Map

GitHub issues remain the canonical coordination state:

| Issue | Scope |
|---|---|
| #491 | Context decision: EverQuest is the active live game target |
| #492 | `everquest.live` profile and docs |
| #493 | `synapse-everquest` log parser/tailer crate |
| #494 | MCP/runtime compact log event feed |
| #495 | EverQuest HUD/level/XP readback contract |
| #496 | Supervised level-1-to-level-2 manual FSV scenario |
| #497 | Operator-attended live MMO supported-use gate |
| #498 | Profile-registry/audit-data moat rows |
| #499 | Safe input aliases and foreground action audit |
| #500 | Full-tool manual FSV coverage matrix against EverQuest |
| #501 | Gameplay learning, hotkey/focus rules, and durable skill memory |
| #505 | EverQuest world model / DynamicJEPA navigation architecture |
| #508 | Literal `/loc` probe with EQ log readback |
| #510 | Current-state estimator fusing logs, `/loc`, map, HUD, and action audit |
| #511 | EverQuest DynamicJEPA state/action/outcome domain pack |
| #512 | Linked trajectory rows from Synapse audit rows plus EQ log events |
| #513 | Durable world-model row prefixes and compact readback tools |
| #514 | Planner guard-decision rows for bounded EverQuest candidates |
| #515 | Surprise detector for unexpected zone/action outcomes |
| #516 | Compact world-summary context injection from map/log/storage SoTs |
| #517 | Stabilize EverQuest foreground before accepted action candidates |
| #518 | Safe target/combat model for level-1 wizard leveling |
| #519 | Manual FSV route from Neriak Foreign Quarter to Nektulos safe area |
| #520 | Map data acquisition/provenance and optional community map pack workflow |
| #521 | ContextGraph-compatible DynamicJEPA episode JSONL export |
| #522 | Tiny local predictive EverQuest world model after verified trajectories |
| #525 | Calibrated map-window sensor from visible map, `/loc`, and map files |
| #526 | Compact outcome log taxonomy for combat/spell/XP/death/hazard learning |
| #527 | Local route planner from current state to map landmarks/zone lines |
| #528 | Hazard and safe-area memory rows for planning |
| #529 | ContextGraph ingestion/retrieval for exported EQ memories |
| #531 | 60-80% useful action-prior scorecard with honest abstention |
| #533 | Learned verified zone-transition volumes from physical crossings |
| #504 | Remove unsafe `open_chat` alias and verify chat-focus denial |

## Gameplay Learning Memory

EverQuest gameplay knowledge is durable project state. Store learned controls,
focus rules, recovery steps, routes, combat loops, and UI source-of-truth
findings in #501 as compact issue comments. Do not rely on the agent's current
context window to remember how to play after compaction.

Initial learned rules:

- `i` opens the Inventory character panel used for level/XP readback.
- `Enter` can focus or submit chat input and is not a safe recovery primitive;
  when chat input is focused, ordinary keypresses type into chat instead of
  moving or triggering hotbar actions.
- If chat focus is already polluted with typed keys, clear it with bounded
  Backspace presses, read the EQ log `You say` count before and after, and use
  Enter only after the buffer is expected empty; never expose this as a profile
  alias or normal gameplay primitive.
- Restore world focus and read visible UI state before movement or combat
  inputs.
- Use the in-game Options/keybind UI as the authoritative control list when a
  binding is uncertain.
- Summarize public chat bodies in issues; do not preserve unnecessary raw chat.

## World Model And Competence Threshold

The world-model goal is practical supervised competence, not a claim of full
game intelligence. The first useful threshold is a calibrated action prior that
is right roughly 60-80% of the time on manually verified local EverQuest
trajectories, with honest abstention when state is sparse or contradictory.
That threshold is a floor, not a ceiling: keep optimizing above it as the
profile registry, action audit, log taxonomy, map rows, and ContextGraph-style
trajectory exports accumulate more evidence.

Manual FSV remains the runtime shipping gate. Model scorecards and prediction
accuracy are planning-quality evidence only; any real action, route, level,
zone, storage row, or profile claim still needs before/trigger/after physical
SoT readback.

## Manual FSV Plan

Happy path:

1. Read the EverQuest process/window/log/UI state before action.
2. Activate or match `everquest.live` through the real MCP runtime.
3. Use `observe` and `read_text` to identify zone, level, and UI state.
4. Use only foreground keyboard/mouse actions through Synapse to progress the
   level 1 Dark Elf Wizard.
5. After each meaningful action cluster, separately read the visible UI,
   logs/config files, and Synapse storage rows.
6. Stop when level 2 is visible and record the actual source-of-truth values.

Required edges:

| Edge | Before/after SoT | Expected outcome |
|---|---|---|
| Unsupported foreground | Focus a non-EverQuest window, then attempt an EverQuest action | Profile/action gate refuses or the action is not sent to EverQuest; no game state changes |
| Menu/chat focused | Encounter or simulate pre-existing UI/menu/chat focus, then avoid movement/combat input until the state is read | Readback identifies non-world/action context; agent clears chat text without submitting it, proves the EQ log `You say` count did not change, and restores visible gameplay UI before action |
| Invalid or empty action | Send an invalid/empty action request through MCP | Tool rejects input and action row/game state do not show unintended input |
| Loss of game foreground | Alt-tab/minimize or verify another window foreground | Action preflight either restores and proves `eqgame.exe` before dispatch or fails closed with no gameplay-progress claim |

No GitHub Actions/CI and no FSV scripts are allowed. Supporting local checks may
be run only as supporting evidence.
