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
| Supported-use target policy | Foreground process command line plus `world.mt` plus latest Luanti log session |
| Action/reflex/audit state | MCP `storage_inspect`, `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`, `cf_row_samples`, and tool logs |
| Profile quality state | MCP `profile_quality_refresh` plus `storage_inspect.cf_row_samples.CF_PROFILES` key `profile_quality/v1/luanti.minetest` |
| Visual/HUD state | `observe` pixel/a11y payload, `luanti.crosshair_contrast`, and `luanti.hotbar_contrast` |

If a surface is not implemented yet, record the missing physical SoT and keep
or open the implementation issue.

---

## 2. Preflight

1. Read `AGENTS.md`, #351, #471, and this runbook.
2. Read `git status --short --branch`; the worktree must not hide unrelated
   changes.
3. Read the benchmark profile metadata through `profile_list` and confirm:
   `benchmark_id=luanti.minetest`, `benchmark_world_name=synapse_benchmark_mtg`,
   `benchmark_world_seed=3222739075906153741`, and
   `supported_use.remote_server_allowed=false`.
   Also confirm `hud_fields` contains `luanti.crosshair_contrast` and
   `luanti.hotbar_contrast`, `event_extensions` contains the Luanti benchmark
   outcome names, and `metadata.capability.audit.action_log` names
   `CF_ACTION_LOG`.
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
- Read diagnostics for `a11y_status`, pixel/capture/perception status, and
  capture latency.
- Read `hud.by_name.luanti.crosshair_contrast` and
  `hud.by_name.luanti.hotbar_contrast`; record the numeric contrast values and
  region strings.
- Read `entities` for `luanti_crosshair_region` and `luanti_hotbar_region`.

### 3.3 Action

Before trigger:

- Read foreground PID/HWND and active/observed profile.
- Read foreground process command line and confirm it contains the configured
  `--world`, `--gameid`, and `--logfile` values.
- Read `world.mt` for `world_name=synapse_benchmark_mtg` and `gameid=minetest`.
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
- Read supported-use policy log data: allowed runs must include
  `SAFETY_PROFILE_TARGET_ALLOWED` with the foreground PID, command line, world
  path, world name, gameid, and logfile path.
- Read action/audit/log SoT. If there is no physical action/audit SoT yet,
  record that as a current capability gap and link #476.
- Read `storage_inspect.cf_row_counts.CF_ACTION_LOG` before and after the
  action. Then read `storage_inspect.cf_row_samples.CF_ACTION_LOG` and record
  the sampled JSON row showing the actual action audit data that resides in
  RocksDB.
- Run `release_all` and read no held input state if the runtime exposes it.

### 3.4 Reflex

Trigger only reflexes that are already implemented and scoped to the local
approved benchmark world. After trigger, read `CF_REFLEX_AUDIT`, scheduler
logs, and the Luanti process/window/world state. If no reflex path is available
for Luanti yet, record the absence and do not claim reflex FSV.

### 3.5 Profile Quality

Before trigger:

- Read `storage_inspect.cf_row_counts.CF_PROFILES` and
  `cf_row_samples.CF_PROFILES` for `profile_quality/v1/luanti.minetest`.
- Read `storage_inspect.cf_row_counts.CF_ACTION_LOG` and
  `cf_row_samples.CF_ACTION_LOG`.
- Read `profile_list` for `luanti.minetest` metadata
  `registry.quality_signal`.

Trigger:

- Call real MCP `profile_quality_refresh` with
  `profile_id="luanti.minetest"` after the benchmark action rows exist.

After trigger:

- Read the returned `snapshot.source`, `snapshot.counts`, `snapshot.rates`,
  `snapshot.score`, `snapshot.compatibility`, `snapshot.redaction`, and
  `snapshot.contribution`.
- Separately call `storage_inspect` and read `CF_PROFILES` row count/sample to
  prove the physical profile-quality row exists.
- Confirm score-bearing fields changed only from observed Luanti foreground
  outcomes. Denied target, failed launch, stale, corrupt, and profile-mismatch
  rows may change source/compatibility counters, but must not be counted as
  foreground `ok`/`error` score samples unless the foreground profile is
  actually `luanti.minetest`.

---

## 4. Required Edge Cases

Every shipping comment for a benchmark run must include before and after state
for these edges.

| Edge | Trigger | Required after readback |
|---|---|---|
| Failed launch policy | Call `act_launch` without matching `--allow-launch` | `SAFETY_LAUNCH_DENIED_BY_POLICY`, unchanged process table, no Luanti log/session |
| Wrong window/profile match | Focus a non-Luanti or fake-title `luanti.exe` window, then `observe` | no `foreground.profile_id=luanti.minetest`; log `PROFILE_FOREGROUND_UNMATCHED` |
| Supported-use denied | Try action/reflex against remote or unapproved target | `SAFETY_PROFILE_ACTION_DENIED` before dispatch, no action/reflex side effect, error data names the reason and physical SoT |
| HUD hidden | Press F1, then `observe` | crosshair/hotbar contrast values change or degrade; record before/after HUD readings and restore F1 |
| Inventory/menu open | Press `i` or `esc`, then `observe` | foreground remains Luanti; HUD/entity readings reflect menu/world-view change; restore with `esc` |
| Unfocused/minimized | Focus another window or minimize Luanti, then `observe` | foreground/profile changes or capture reports degraded/disabled; no invented Luanti HUD pass |
| Backend unavailable or denied | Request unavailable backend or denied supported-use target | action response/log names the denial/unavailable reason; `CF_ACTION_LOG` sample records the attempted/failed action when dispatch path reached audit |
| Profile-quality failed launch | Trigger denied or failed `act_launch`, then `profile_quality_refresh` | `CF_ACTION_LOG` records launch failure; `CF_PROFILES` quality snapshot records launch/error compatibility counters without inventing foreground success |
| Profile-quality stale/corrupt audit | Use stale cutoff or diagnostic corrupt `CF_ACTION_LOG` probe row, then `profile_quality_refresh` | source counters show stale/corrupt rows ignored; score sample counts do not increase |

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
4. HUD hidden / menu / unfocused / backend denied
   before:
   trigger:
   after:

Supporting checks only:
```

Do not compress this to "tests passed." The source of truth bytes and runtime
state are the verdict.
