# 18 - Luanti / Minetest Game Benchmark

This document is the configured-host source for the local Luanti benchmark
target selected in issue #471. It defines the installed engine/game paths,
world fixture, provenance, and physical sources of truth used by manual FSV.
The step-by-step whole-system runbook lives in
`19_luanti_manual_fsv_runbook.md`.

Automated tests, helper scripts, benchmark harnesses, GitHub Actions, and CI
are not FSV. The agent must trigger the real runtime surface where one exists,
then manually read the physical state named below.

---

## 1. Canonical Paths

All Luanti benchmark state lives under `%LOCALAPPDATA%\synapse\benchmarks\luanti`.

| Item | Canonical location |
|---|---|
| Engine zip | `%LOCALAPPDATA%\synapse\downloads\luanti-5.16.1-win64.zip` |
| Engine root | `%LOCALAPPDATA%\synapse\benchmarks\luanti\engine\5.16.1\luanti-5.16.1-win64` |
| Engine executable | `%LOCALAPPDATA%\synapse\benchmarks\luanti\engine\5.16.1\luanti-5.16.1-win64\bin\luanti.exe` |
| Minetest Game | `%LOCALAPPDATA%\synapse\benchmarks\luanti\engine\5.16.1\luanti-5.16.1-win64\games\minetest_game` |
| Benchmark world | `%LOCALAPPDATA%\synapse\benchmarks\luanti\engine\5.16.1\luanti-5.16.1-win64\worlds\synapse_benchmark_mtg` |
| Benchmark log | `%LOCALAPPDATA%\synapse\benchmarks\luanti\synapse_benchmark_mtg.log` |
| Synapse profile | `crates/synapse-profiles/profiles/luanti.minetest.toml` |

The bundled profile exposes these same paths through `[metadata]` so
`profile_list` can read them at runtime. Runtime action/reflex supported-use
enforcement also reads these paths: the foreground Luanti process command line
must contain the configured `--world`, `--gameid`, and `--logfile` values, and
the latest benchmark log session must prove the same local world/gameid before
dispatch is allowed.

---

## 2. Capability Matrix

The bundled `luanti.minetest` profile is the benchmark capability matrix.
`profile_list` must read the loaded matrix back at runtime instead of relying
on the TOML file alone.

| Capability | Runtime readback |
|---|---|
| Foreground/profile | `observe.foreground.profile_id = luanti.minetest` |
| Pixel capture | `observe.diagnostics.capture_status = healthy` after the foreground-window probe |
| HUD baseline | `observe.hud.by_name.luanti.crosshair_contrast` and `luanti.hotbar_contrast` |
| Entity/target baseline | `observe.entities` contains `luanti_crosshair_region` and `luanti_hotbar_region` markers |
| Software keyboard | `act_press` through `keyboard_default=auto/software` |
| Software mouse | `act_click`, `act_aim`, `act_drag`, and `act_scroll` through `mouse_default=auto/software` |
| Hardware HID parity | profile metadata marks keyboard/mouse hardware HID as the parity target when configured |
| ViGEm pad | profile metadata marks pad support as available when the ViGEm backend is configured |
| Action audit | `storage_inspect.cf_row_counts.CF_ACTION_LOG` plus `cf_row_samples.CF_ACTION_LOG` |
| Profile quality | `profile_quality_refresh(profile_id="luanti.minetest")` writes/reads `CF_PROFILES` key `profile_quality/v1/luanti.minetest` |

The HUD baseline is deliberately a minimal visible-state extractor, not the
full HUD template matcher. It samples raw foreground-window pixels and reports
a 0..1 luma-contrast score over fixed crosshair and hotbar regions. Full
template matching remains tracked separately by #410.

Profile event extensions declare the benchmark outcomes that the profile/audit
loop should learn from as the event-extension evaluator matures: launched,
joined world, observed HUD, moved, dug/placed, inventory opened, and release-all
applied.

Profile quality scoring is local-first. The Luanti benchmark feeds the
profile-registry/audit-data loop only through observed runtime outcomes in
`CF_ACTION_LOG`; `profile_quality_refresh` ignores stale, corrupt, and
non-matching rows for the quality score, records compatibility and denial
counters separately, redacts process paths/window titles from the score
snapshot, and does not export/share anything without a future explicit operator
approval path.

---

## 3. Provenance

| Artifact | Expected value |
|---|---|
| Engine | Luanti 5.16.1 win64 portable release |
| Engine zip SHA256 | `a70fd87e67cc236f250fca90e5cd30211f3e45937b107158b5367d6ee26aabb8` |
| Installed `luanti.exe` SHA256 | `D0D7A1C62FEEA79B7A4C5F9D9124608E4E10D7CF2575E6412BAF0E945ECABD83` |
| Run-in-place marker | `RUN_IN_PLACE=1` in the portable build |
| Game repository | `luanti-org/minetest_game` |
| Game commit | `95991f8dc4c97d3cc7945269bf2d5640c7fe6bc8` |
| Game id list readback | `minetest` |

Important: Luanti discovers this game as `minetest`, not `minetest_game`.
`minetest_game` is the source checkout directory, while `minetest` is the
runtime `--gameid`. Run `--gameid list` from the engine root for the canonical
readback; other working directories can make duplicate search paths visible.

---

## 4. Deterministic World Fixture

The repository fixture is:

- `docs/computergames/fixtures/luanti_minetest_world/world.mt`
- `docs/computergames/fixtures/luanti_minetest_world/map_meta.txt`

The fixture pins the world identity, storage backends, mapgen, and seed. It
does not include generated SQLite databases; those are physical runtime SoT
created by Luanti after launch.

Expected fixture values:

| Field | Expected value |
|---|---|
| World name | `synapse_benchmark_mtg` |
| Player name | `synapsebench` |
| `gameid` | `minetest` |
| Mapgen | `v7` |
| Seed | `3222739075906153741` |
| World backend | `sqlite3` |
| Auth backend | `sqlite3` |
| Player backend | `sqlite3` |
| Damage | `true` |
| Creative mode | `false` |

Runtime SoT after launch:

- `world.mt`
- `map_meta.txt`
- `map.sqlite`
- `auth.sqlite`
- `players.sqlite`
- `mod_storage.sqlite`
- Luanti log lines for `World at [...]`, `Server for gameid="minetest"`,
  and `synapsebench` joining.

---

## 5. Launch Contract

Manual launch command, with paths expanded on the configured host:

```powershell
& "$env:LOCALAPPDATA\synapse\benchmarks\luanti\engine\5.16.1\luanti-5.16.1-win64\bin\luanti.exe" `
  --go `
  --world "$env:LOCALAPPDATA\synapse\benchmarks\luanti\engine\5.16.1\luanti-5.16.1-win64\worlds\synapse_benchmark_mtg" `
  --gameid minetest `
  --name synapsebench `
  --logfile "$env:LOCALAPPDATA\synapse\benchmarks\luanti\synapse_benchmark_mtg.log"
```

Expected runtime readback:

- Process name: `luanti.exe`
- Window title: `Luanti 5.16.1 [Singleplayer] ...` or
  `Luanti 5.16.1 [Multiplayer] ...`
- Profile match: `luanti.minetest`
- Foreground process command line includes the canonical `--world`,
  `--gameid minetest`, and `--logfile` values.
- Log join address for the configured local run: loopback
  `::ffff:127.0.0.1`

Luanti's `[Multiplayer]` window title can appear for the local server-backed
single-player run. Do not infer remote-server state from the title alone. Read
the world path/log/session source of truth.

---

## 6. Manual Reset

Use this only as a host setup action. It is not FSV by itself.

1. Close every `luanti.exe` process and read that no Luanti process remains.
2. Move the existing
   `%LOCALAPPDATA%\synapse\benchmarks\luanti\engine\5.16.1\luanti-5.16.1-win64\worlds\synapse_benchmark_mtg`
   directory to an archive name such as `synapse_benchmark_mtg.before-reset`.
3. Create a fresh `synapse_benchmark_mtg` directory.
4. Copy the repo fixture `world.mt` and `map_meta.txt` into it.
5. Launch using the contract above.
6. Read the new `world.mt`, `map_meta.txt`, SQLite files, process/window, and
   Luanti log. The expected seed remains `3222739075906153741`.

If the engine zip, engine root, game checkout, or world fixture is missing,
missing-state doctrine applies: acquire or recreate it locally, then read the
physical SoT.

---

## 7. Edge Cases For #473 FSV

For this benchmark fixture, the manual edge audit must cover:

1. Missing engine zip or engine root: acquire/recreate locally and read the
   executable path plus SHA256.
2. Missing game folder: restore `games\minetest_game`, read the git commit,
   and read `--gameid list` showing `minetest`.
3. Wrong game id spelling: launching with `--gameid minetest_game` on the
   current configured host can still load Minetest Game because the source
   folder name is accepted/resolved. The edge verdict is therefore the
   physical SoT: `world.mt`, `map_meta.txt`, and Luanti logs must still read
   actual runtime `gameid = minetest`; otherwise relaunch with `--gameid
   minetest` and capture the exact error/log.
4. Stale or corrupt world directory: move it aside, copy the fixture
   `world.mt` and `map_meta.txt`, relaunch, and read regenerated SQLite files
   plus the expected log lines.

Record before and after state for each edge in the GitHub issue. The
authoritative verdict is the bytes on disk, the process/window state, and the
runtime MCP readback, not command return values.
