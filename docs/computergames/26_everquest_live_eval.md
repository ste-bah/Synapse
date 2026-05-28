# 26 - EverQuest Live Evaluation

This document is the configured-host note for the current live game evaluation
target. It records operator-provided account/character state and the manual FSV
contract for evaluating Synapse against EverQuest.

This is not a script, benchmark harness, CI job, or automated FSV substitute.
Manual FSV must use the real Synapse MCP runtime surface, the visible
EverQuest client, local client logs/config files, Windows process/window state,
and Synapse storage/audit rows as the sources of truth.

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
| Windows process table | `eqgame.exe` PID and command line from the Daybreak install |
| Foreground window state | title/class/HWND/process path for the active EverQuest window |
| Visible game UI | OCR/screenshot evidence for character/server/zone/level/XP or UI text |
| EverQuest install/log files | relevant same-day logs/config files under the Daybreak install |
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

Before claiming an alias effect, manually read the visible UI/log/storage SoT
before the trigger, call the real MCP `act_keymap` tool while `eqgame.exe` is
foreground, then separately read the visible UI/log/storage state again. The
`CF_ACTION_LOG` row must show `tool=act_keymap`, requested alias, resolved
binding/key list, backend, hold duration, foreground `eqgame.exe`, and the
allow/deny/error status.

## Log Pipeline

EverQuest already creates local activity logs on this host. Current readback:

| File | Current role |
|---|---|
| `C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\Logs\eqlog_Thenumberone_frostreaver.txt` | Same-day in-game activity log |
| `C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\eqclient.ini` | Client config; `LastCharSel=Thenumberone`, `Log=1` |
| `C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\Thenumberone_frostreaver_WIZ.ini` | Character config |
| `C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\UI_Thenumberone_frostreaver_WIZ.ini` | Character UI layout |

The `synapse-everquest` crate owns EverQuest-specific log discovery, parsing,
cursor-based tailing, and token-efficient summaries. It must turn noisy raw log
lines into compact events such as `target_npc`, `consider`, `cast_begins`,
`tell`, `say`, and `system` while suppressing long chat bodies unless a later
operator-approved feature explicitly needs them. Runtime integration should feed
these compact events into `observe`, event streams, profile quality, and audit
storage without dumping full raw logs into the model context.

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
| Loss of game foreground | Alt-tab/minimize or verify another window foreground | `observe` no longer reports `everquest.live`; no level/progress claim is made |

No GitHub Actions/CI and no FSV scripts are allowed. Supporting local checks may
be run only as supporting evidence.
