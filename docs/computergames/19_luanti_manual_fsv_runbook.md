# 19 - Luanti Manual Whole-System FSV Runbook

This runbook defines how an agent manually verifies the Luanti / Minetest Game
benchmark end to end on the configured Windows host. It is not a script, test,
benchmark harness, CI job, or GitHub Action. Do not automate this runbook and
do not call any supporting command output "FSV" unless the physical source of
truth named here has been read before and after the trigger.

Benchmark install, fixture, and provenance are defined in
`18_luanti_benchmark.md`.

---

## 1. Sources Of Truth

Every run must name a run id and read these sources before and after the
triggered action.

| Surface | Source of truth |
|---|---|
| Profile registry state | MCP `profile_list` from the repo-built `synapse-mcp` binary |
| Active/observed profile | MCP `profile_activate`, `health`, and `observe.foreground.profile_id` |
| Launch policy | MCP `act_launch` response data plus Synapse log `M4_ACT_LAUNCH_*` |
| Process/window | Windows process table and foreground/window title from `observe` |
| World identity | `world.mt`, `map_meta.txt`, and fixture file hashes |
| Runtime world state | `map.sqlite`, `auth.sqlite`, `players.sqlite`, `mod_storage.sqlite` |
| Game session | `synapse_benchmark_mtg.log` lines for world path, gameid, player join |
| Action/reflex/audit state | MCP `storage_inspect`, `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`, and tool logs |
| Visual/HUD state | `observe` pixel/a11y payload plus any HUD/perception readback added by #475 |

If a surface is not implemented yet, record the missing physical SoT and keep
or open the implementation issue. Current known follow-ups:

- #475 covers HUD/perception/action capability readback.
- #477 covers supported-use target-policy refusal before action/reflex dispatch.

---

## 2. Preflight

1. Read `AGENTS.md`, #351, #471, and this runbook.
2. Read `git status --short --branch`; the worktree must not hide unrelated
   changes.
3. Read the benchmark profile metadata through `profile_list` and confirm:
   `benchmark_id=luanti.minetest`, `benchmark_world_name=synapse_benchmark_mtg`,
   `benchmark_world_seed=3222739075906153741`, and
   `supported_use.remote_server_allowed=false`.
4. Read the physical install:
   - engine zip SHA256
   - `luanti.exe` SHA256
   - `games\minetest_game` git commit
   - engine-root `--gameid list`
5. Read the canonical world:
   - `world.mt`
   - `map_meta.txt`
   - fixture hashes versus repo fixture hashes

Do not proceed if the benchmark target is missing. Missing local state is
setup work: make it real locally, then read the source of truth again.

---

## 3. Happy Path

### 3.1 Launch

Before trigger:

- No stale `luanti.exe` process, or record and close it.
- Canonical world contains fixture `world.mt` and `map_meta.txt`.
- `synapse_benchmark_mtg.log` is absent or archived for a clean read.

Trigger:

- Use real MCP `act_launch` from the repo-built binary.
- Start `synapse-mcp --mode stdio` with a narrow `--allow-launch` regex that
  matches the exact Luanti command line.
- Call `act_launch` with:
  - target: configured `luanti.exe`
  - working_dir: configured engine root
  - args: `--go`, `--world <canonical world>`, `--gameid minetest`,
    `--name synapsebench`, `--logfile <benchmark log>`
  - wait_for_window_title_regex:
    `^Luanti 5\.16\.[0-9]+ \[(Singleplayer|Multiplayer)\].*`

After trigger:

- Read `act_launch` response `pid`, `hwnd`, `matched_title`, and `reason`.
- Read process table for that PID.
- Read `observe.foreground` and confirm `process_name=luanti.exe`,
  `profile_id=luanti.minetest`, and the Luanti title.
- Read `synapse_benchmark_mtg.log` for `World at`, `Server for
  gameid="minetest"`, and `synapsebench` joining.
- Read regenerated SQLite files in the world directory.

### 3.2 Observe

Trigger MCP `observe` with the Luanti window foreground.

After trigger:

- Read `foreground.profile_id=luanti.minetest`.
- Read diagnostics for `a11y_status` and pixel/capture/perception status.
- If HUD/perception fields are absent, record that state and continue under
  #475 rather than inventing a pass.

### 3.3 Action

Before trigger:

- Read foreground PID/HWND and active/observed profile.
- Read world log tail and any available action/audit CF counts.

Trigger one harmless visible action through real MCP action tools. Preferred
order as capability lands:

1. `act_press` inventory/menu key, then read visible state/HUD/a11y.
2. `act_press` movement/jump key, then read visible state/log/audit.
3. `act_aim` or mouse action, then read visible state/action audit.

For action-tool FSV, keep the MCP transport/session alive until the action
result and the separate after-state readback are complete. Do not send an
action in a one-shot stdio batch that closes stdin immediately after the
request; connection shutdown can trigger `release_all` cleanup and produce an
`ACTION_BACKEND_UNAVAILABLE` result before the physical state can be read.

After trigger:

- Read foreground still belongs to Luanti.
- Read action/audit/log SoT. If there is no physical action/audit SoT yet,
  record that as a current capability gap and link #475/#476.
- Run `release_all` and read no held input state if the runtime exposes it.

### 3.4 Reflex

Trigger only reflexes that are already implemented and scoped to the local
approved benchmark world. After trigger, read `CF_REFLEX_AUDIT`, scheduler
logs, and the Luanti process/window/world state. If no reflex path is available
for Luanti yet, record the absence and do not claim reflex FSV.

---

## 4. Required Edge Cases

Every shipping comment for a benchmark run must include before and after state
for these edges.

| Edge | Trigger | Required after readback |
|---|---|---|
| Failed launch policy | Call `act_launch` without matching `--allow-launch` | `SAFETY_LAUNCH_DENIED_BY_POLICY`, unchanged process table, no Luanti log/session |
| Wrong window/profile match | Focus a non-Luanti or fake-title `luanti.exe` window, then `observe` | no `foreground.profile_id=luanti.minetest`; log `PROFILE_FOREGROUND_UNMATCHED` |
| Supported-use denied | Try action/reflex against remote or unapproved target | fail closed before dispatch, no action/reflex side effect, safety log/audit row (#477 owns implementation) |
| HUD/perception absent | Hide HUD with F1, open inventory/menu, or minimize/unfocus window | `observe`/HUD state records absence or wrong mode; no invented HUD result (#475 owns baseline) |

---

## 5. Evidence Template

Use this shape in the GitHub issue comment:

```text
Run id:
Commit:
No GitHub Actions/CI:

SoT:
- profile:
- process/window:
- world files:
- log:
- storage/audit:

Happy path:
- before:
- trigger:
- after:

Edges:
1. failed launch policy
   before:
   trigger:
   after:
2. wrong window/profile
   before:
   trigger:
   after:
3. supported-use denied
   before:
   trigger:
   after:
4. HUD/perception absent
   before:
   trigger:
   after:

Supporting checks only:
```

Do not compress this to "tests passed." The source of truth bytes and runtime
state are the verdict.
