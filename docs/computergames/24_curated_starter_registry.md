# 24 - Curated Starter Registry

## 1. Status and authority

This document closes issue #466's planning gap for the #454
profile-registry / audit-data moat. The starter registry is the local seed set
that tells Synapse which profiles should compound first through real audit
evidence, quality scoring, compatibility metadata, and package provenance.

The starter registry is not a marketing catalog. It is an operator-owned local
work queue with physical Sources of Truth:

- bundled profile TOML files under `crates/synapse-profiles/profiles/`
- package manifests under `docs/computergames/fixtures/`
- registry rows in RocksDB `CF_PROFILES` under `profile_registry/v1/`
- registry head pointers in `CF_KV`
- audit rows in `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`, `CF_EVENTS`,
  `CF_OBSERVATIONS`, and `CF_SESSIONS`
- GitHub per-target issues linked from the seed rows

Manual FSV remains the gate. Scripts, tests, benchmarks, and GitHub Actions/CI
are supporting evidence only and must not be described as FSV.

## 2. Seed set

Seed set id: `starter.v1`

| Priority | Target | Profile id | Status | Issue | Use scope | Minimum manual FSV |
|---|---|---|---|---|---|---|
| P0 | Luanti / Minetest Game benchmark | `luanti.minetest` | Shipped benchmark profile | #471-#476 | `operator_owned_test` | `profile_list`, registry install/search/inspect, `observe`, action audit rows, Luanti process/log/world files, `profile_quality_refresh` |
| P1 | Notepad | `notepad` | Shipped productivity package | #478 | `productivity` | `profile_list`, registry package install/search/inspect, foreground observe, text/action audit row, profile quality refresh, Notepad value readback |
| P1 | Visual Studio Code / VSCodium | `vscode` | Shipped productivity package | #479 | `productivity` | `profile_list`, registry package install/search/inspect, foreground observe, key/text action audit rows, profile quality refresh, VS Code file/command-palette/terminal readback |
| P1 | Windows Terminal / PowerShell | `terminal` | Shipped productivity package | #480 | `productivity` | `profile_list`, registry package install/search/inspect, foreground observe, clipboard paste/enter action audit rows, command output readback, profile quality refresh, terminal settings readback |
| P1 | Chromium-family browsers | `chrome` | Bundled profile, package backlog | #481 | `productivity` | `profile_list`, registry package install, foreground observe, navigation/tab action audit row, profile quality refresh |
| P1 | Minecraft Java | `minecraft.java` | Planned game profile | #482 | `operator_owned_test` / `single_player` only after policy decision | Real game process/window/world/log SoTs, supported-use denial edges, registry package install, action/reflex audit rows |

Luanti remains the whole-system benchmark target because it is installed,
account-free, resettable, and already verified on this configured host. Minecraft
Java remains the M4 first-game demo target, but must not inherit Luanti evidence
as proof of Minecraft behavior; Luanti evidence is only analogue evidence until
Minecraft itself is run and read back.

## 3. Registry row

When a package manifest includes `curated.*` metadata, the real
`profile_registry_install` path writes one additional row:

```text
CF_PROFILES/profile_registry/v1/curated_target/<seed_set_id>/<target_id>
```

Row kind: `curated_profile_target`

Required manifest metadata:

- `curated.seed_set_id`
- `curated.target_id`
- `curated.tier`
- `curated.priority`
- `curated.status`
- `curated.backlog_issue`
- `curated.minimum_manual_fsv`

The installer fails closed if any `curated.*` field is present without the full
required set, or if `curated.target_id` does not match a manifest
compatibility target. The row stores the package/profile ids, use scope, target
kind, app/process/title compatibility fields, safe backend policy, quality
signal, package key, installed key, compatibility key, and provenance.

## 4. Profile metadata

Bundled starter profiles carry lightweight metadata so `profile_list` can expose
their seed status before the registry package exists:

- `registry.family = "profile-registry/audit-data"`
- `registry.curated_seed = "starter.v1"`
- `registry.curated_status`
- `registry.curated_tier`
- `registry.curated_priority`
- `registry.backlog_issue`
- `registry.quality_signal`
- `registry.minimum_manual_fsv`
- `registry.compatibility_target`
- `registry.safe_default_backend_policy`

These profile metadata keys are not sufficient by themselves to ship a package.
They are a profile-directory SoT and backlog pointer. Package manifests and
`profile_registry_install` rows remain the registry SoT.

## 5. Follow-up issue rule

Every selected seed target needs a GitHub issue before implementation starts.
The issue must include:

- parent links to #466 and #454
- target app/game and runtime profile id
- use scope and supported-use boundaries
- compatibility metadata required before install
- manual FSV happy path plus duplicate package, unknown use scope, missing
  compatibility metadata, and profile mismatch edges
- explicit statement that GitHub Actions/CI are not a shipping gate

New target ideas that are not yet selected should not be hidden in docs. Open a
`type:discovery` or per-target issue first, then update this table once the
target is selected.

## 6. Manual FSV contract

For a curated seed install:

1. Read the profile directory and registry SoT before the trigger:
   `profile_list`, `profile_registry_search(row_kind="curated_profile_target")`,
   `storage_inspect`, and the manifest/profile TOML files.
2. Trigger the real runtime path:
   `profile_registry_install` on a curated package manifest.
3. Read the SoT again with separate operations:
   `profile_registry_search`, `profile_registry_inspect` for the curated row,
   package/profile/installed/compatibility rows, and `storage_inspect`
   `CF_PROFILES` / `CF_KV` counts.
4. Manually exercise at least:
   duplicate same package, unknown use scope, missing compatibility metadata,
   and missing/partial curated metadata.
5. Print before and after state for each edge, including actual row counts and
   row keys.

For app/game-specific profile completion, the per-target issue adds the real
process/window/log/world/file/audit SoTs for that app or game.

## 7. Fixtures

Fixture directory:
`docs/computergames/fixtures/curated_starter_registry/`

| Fixture | Purpose |
|---|---|
| `curated_luanti_package_manifest.toml` | Valid curated package manifest that writes a `curated_profile_target` row for `luanti.minetest`. |
| `cf_profiles_curated_luanti_row.json` | Static expected row shape for docs/data-model review; runtime FSV must still inspect RocksDB. |
| `curated_notepad_package_manifest.toml` | Valid curated package manifest that writes a `curated_profile_target` row for `notepad.windows`. |
| `cf_profiles_curated_notepad_row.json` | Static expected row shape for the Notepad starter package; runtime FSV must still inspect RocksDB. |
| `curated_vscode_package_manifest.toml` | Valid curated package manifest that writes a `curated_profile_target` row for `vscode.windows` and compatibility rows for VS Code/VSCodium. |
| `cf_profiles_curated_vscode_row.json` | Static expected row shape for the VS Code starter package; runtime FSV must still inspect RocksDB. |
| `curated_terminal_package_manifest.toml` | Valid curated package manifest that writes a `curated_profile_target` row for `terminal.windows`. |
| `cf_profiles_curated_terminal_row.json` | Static expected row shape for the Windows Terminal starter package; runtime FSV must still inspect RocksDB. |
| `edge_terminal_unknown_use_scope_manifest.toml` | Invalid Windows Terminal package: installable curated package cannot use `use_scope = "unknown"`. |
| `edge_terminal_missing_compatibility_manifest.toml` | Invalid Windows Terminal package: curated target cannot ship without a compatibility target. |
| `edge_terminal_profile_mismatch_manifest.toml` | Invalid Windows Terminal package: manifest `profile_id` must match the authored profile TOML id. |
| `edge_vscode_unknown_use_scope_manifest.toml` | Invalid VS Code package: installable curated package cannot use `use_scope = "unknown"`. |
| `edge_vscode_missing_compatibility_manifest.toml` | Invalid VS Code package: curated target cannot ship without a compatibility target. |
| `edge_vscode_profile_mismatch_manifest.toml` | Invalid VS Code package: manifest `profile_id` must match the authored profile TOML id. |
| `edge_notepad_unknown_use_scope_manifest.toml` | Invalid Notepad package: installable curated package cannot use `use_scope = "unknown"`. |
| `edge_notepad_missing_compatibility_manifest.toml` | Invalid Notepad package: curated target cannot ship without a compatibility target. |
| `edge_notepad_profile_mismatch_manifest.toml` | Invalid Notepad package: manifest `profile_id` must match the authored profile TOML id. |
| `edge_unknown_use_scope_manifest.toml` | Invalid package: installable curated package cannot use `use_scope = "unknown"`. |
| `edge_missing_compatibility_manifest.toml` | Invalid package: curated target cannot ship without a compatibility target. |
