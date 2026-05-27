# 22 - Local Profile Registry Data Model

## 1. Status and authority

This document is the local data-model baseline for issue #455 and the #454
profile-registry / audit-data moat. It decides the physical source of truth for
local registry state before the later package, MCP, sync, inspector, and shared
service work lands.

Decision:

1. Use existing RocksDB `CF_PROFILES` for first-class local registry rows.
2. Use `CF_KV` only for tiny registry head/pointer rows.
3. Do not add a new registry-specific column family for the M5 v0 registry
   model; schema pressure can be revisited after runtime tools prove the access
   pattern needs it.
4. Keep authored profile TOML as authored source. Installed registry state is
   a separate inspectable JSON row set that links profile TOML/package metadata,
   provenance, trust, compatibility, quality, and audit evidence.

Runtime tools now write these rows through Synapse: `profile_registry_install`
validates a local package manifest/profile TOML and writes source, package,
profile, installed, compatibility, optional quality-link, and source-head rows;
`profile_registry_disable` updates installed state; `profile_registry_export`
and `profile_registry_import` move local bundles. Manual FSV must verify them
by reading `CF_PROFILES`/`CF_KV` with `storage_inspect` and registry-specific
readback tools. The fixtures in
`docs/computergames/fixtures/profile_registry_data_model/` are synthetic row
SoTs for this docs/data-model baseline.

## 2. Column-family use

| Column family | Registry role | Why |
|---|---|---|
| `CF_PROFILES` | Primary profile-registry row store | Already reserved for cached profile/registry rows and quality snapshots; inspectable with existing storage readback. |
| `CF_KV` | Small registry head pointers | Bounded key-value extension for active registry index/head metadata. |
| `CF_ACTION_LOG` | Runtime action evidence | Linked from registry quality/audit pointers; not duplicated into registry rows. |
| `CF_REFLEX_AUDIT` | Runtime reflex evidence | Linked from registry quality/audit pointers; not duplicated into registry rows. |
| `CF_EVENTS` / `CF_OBSERVATIONS` / `CF_SESSIONS` | Session and observation evidence | Linked by row ranges/hashes when needed. |

No new column family is introduced by this decision. The registry can still be
manually verified because `CF_PROFILES` row keys are human-readable UTF-8.

## 3. Key namespaces

All registry keys are UTF-8 and versioned under `profile_registry/v1/`.

| Row kind | CF | Key |
|---|---|---|
| Registry source | `CF_PROFILES` | `profile_registry/v1/source/<source_id>` |
| Profile package | `CF_PROFILES` | `profile_registry/v1/package/<package_id>/<package_version>` |
| Profile version | `CF_PROFILES` | `profile_registry/v1/profile/<profile_id>/<profile_version>` |
| Installed profile | `CF_PROFILES` | `profile_registry/v1/installed/<profile_id>` |
| Compatibility target | `CF_PROFILES` | `profile_registry/v1/compat/<target_id>/<profile_id>/<profile_version>` |
| Quality link | `CF_PROFILES` | `profile_registry/v1/quality_link/<profile_id>/<profile_version>` |
| Registry head pointer | `CF_KV` | `profile_registry/v1/head/<source_id>` |

`profile_quality/v1/<profile_id>` remains the existing local quality snapshot
key. Registry quality-link rows point to that snapshot instead of copying it.

## 4. Common row envelope

Every registry row value is JSON with this envelope:

```json
{
  "schema_version": 1,
  "row_kind": "profile_package",
  "row_id": "profile.synthetic.governance@0.1.0",
  "created_at": "2026-05-27T02:00:00Z",
  "updated_at": "2026-05-27T02:00:00Z",
  "source_id": "registry.synthetic.local-fixture",
  "state": "active"
}
```

Required envelope fields:

- `schema_version`
- `row_kind`
- `row_id`
- `created_at`
- `updated_at`
- `source_id`
- `state`

Unknown future schema versions fail closed unless a migration issue defines the
upgrade path. Row writes are all-or-nothing at the batch boundary; duplicate or
invalid package rows must not leave partial companion rows.

## 5. Entity model

### 5.1 Registry source

Links a local source id to optional shared/file-backed source metadata.

Required fields beyond the envelope:

- `source_kind`
- `base_url` or `root_path`
- `auth_mode`
- `trust_policy_id`
- `offline_usable`
- `last_health_status`

### 5.2 Profile package

Represents an installable package version.
The manifest bytes are defined by
[`23_profile_package_manifest.md`](23_profile_package_manifest.md); this row
stores the manifest/package digests and registry state after validation.

Required fields beyond the envelope:

- `package_id`
- `package_version`
- `manifest_digest`
- `package_digest`
- `license_spdx`
- `governance_manifest_key`
- `trust_status`
- `moderation_status`
- `revoked`
- `profile_versions`
- `provenance`

### 5.3 Profile version

Links a package to a runtime profile id/version.

Required fields beyond the envelope:

- `profile_id`
- `profile_version`
- `package_id`
- `package_version`
- `profile_toml_path`
- `profile_toml_digest`
- `use_scope`
- `schema_version_supported`

### 5.4 Installed profile

Tracks local install/activation state without rewriting authored profile TOML.

Required fields beyond the envelope:

- `profile_id`
- `active_profile_version`
- `installed_package_id`
- `installed_package_version`
- `installed_at`
- `activation_state`
- `operator_overrides_path`

### 5.5 Compatibility target

Stores compatibility by target, not just by profile id.

Required fields beyond the envelope:

- `target_id`
- `target_kind`
- `profile_id`
- `profile_version`
- `compatibility_status`
- `source_quality_snapshot_key`
- `evidence_hash`

### 5.6 Quality link

Points registry rows to audit-derived quality snapshots.

Required fields beyond the envelope:

- `profile_id`
- `profile_version`
- `profile_quality_key`
- `source_cf_ranges`
- `quality_score`
- `sample_count`
- `evidence_hash`

## 6. Install/register transaction shape

A successful local package registration writes these rows through the real MCP
`profile_registry_install` path:

1. `profile_registry/v1/source/<source_id>` if missing or changed.
2. `profile_registry/v1/package/<package_id>/<package_version>`.
3. One or more `profile_registry/v1/profile/<profile_id>/<profile_version>`.
4. `profile_registry/v1/installed/<profile_id>` when the package is installed.
5. One or more compatibility rows.
6. One quality-link row when audit-derived quality already exists.
7. `CF_KV` head pointer for the source/index only after the rows above succeed.

If validation fails, none of the companion rows are written. If a duplicate
package id/version exists with the same digest, the operation is idempotent and
does not rewrite the row. If it exists with a different digest, the operation
fails with a duplicate-conflict result.

## 7. Validation rules

| Case | Outcome |
|---|---|
| Duplicate package id/version with same digest | Idempotent no-op; row hash unchanged. |
| Duplicate package id/version with different digest | Reject with `duplicate_package_version_conflict`; no companion rows change. |
| Corrupt manifest | Reject with `manifest_decode_failed`; no rows written. |
| Incompatible schema version | Reject with `registry_schema_version_unsupported`; no rows written. |
| Missing governance metadata | Reject per `20_profile_registry_governance.md`. |
| Revoked package | Reject install/activation; tombstone can still be written. |

## 8. Manual FSV contract

Runtime FSV for this model must:

1. Read `CF_PROFILES` and `CF_KV` before the trigger and show whether the
   synthetic package key exists.
2. Trigger the real package registration path.
3. Separately read `CF_PROFILES`/`CF_KV` after the trigger and print the exact
   source/package/profile/installed/compatibility/quality-link rows.
4. Exercise duplicate id/version, corrupt manifest, and incompatible schema
   version edges and prove no silent partial install.

For this docs baseline, the synthetic fixture row files remain the physical SoT
for expected row shape. Runtime acceptance must additionally prove the same row
classes exist in RocksDB after the MCP trigger.

## 9. Fixture row index

| Fixture | Purpose |
|---|---|
| `cf_profiles_source_row.json` | Registry source row in `CF_PROFILES`. |
| `cf_profiles_package_row.json` | Package row with provenance, trust, compatibility, and governance pointer. |
| `cf_profiles_profile_version_row.json` | Runtime profile version row. |
| `cf_profiles_installed_row.json` | Local installed profile row. |
| `cf_profiles_compatibility_row.json` | Compatibility target row. |
| `cf_profiles_quality_link_row.json` | Link from registry package to `profile_quality/v1/<profile_id>`. |
| `cf_kv_registry_head_row.json` | Source/index head pointer in `CF_KV`. |
| `edge_duplicate_id_version.toml` | Duplicate id/version conflict edge. |
| `edge_corrupt_manifest.toml` | Corrupt manifest edge. |
| `edge_incompatible_schema_version.toml` | Unsupported schema edge. |
