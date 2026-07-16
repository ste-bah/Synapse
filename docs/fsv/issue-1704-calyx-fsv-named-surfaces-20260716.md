# Issue #1704 FSV - Calyx FSV-named production surfaces

Date: 2026-07-16
Repo: `C:\code\Synapse`
Issue: https://github.com/ChrisRoyse/Synapse/issues/1704

## Source Of Truth

This is source-tree and public API naming cleanup. It does not change a Synapse
MCP runtime behavior and has no MCP trigger surface.

Source of truth:

- `calyx/crates/calyx-lodestar/Cargo.toml`
- `calyx/crates/calyx-anneal/src/lib.rs`
- `calyx/crates/calyx-anneal/src/integration_fsv.rs`
- `calyx/crates/calyx-anneal/src/integration_fsv/artifact.rs`
- `calyx/crates/calyx-anneal/src/substrate.rs`
- `calyx/crates/calyx-anneal/src/substrate/artifact.rs`
- `calyx/crates/calyx-assay/src/formula_catalog.rs`
- `rg` inventory over `calyx/crates`
- the final Git tree after commit

## Research

I used Exa plus web search against official Rust/Cargo documentation:

- Rust modules enter the crate through parent `mod` declarations or path
  attributes: https://doc.rust-lang.org/reference/items/modules.html
- Rust source files are loaded as modules from the crate source file graph:
  https://doc.rust-lang.org/reference/crates-and-source-files.html
- Cargo features are explicit named entries in the `[features]` table:
  https://doc.rust-lang.org/cargo/reference/features.html

Applied conclusion: `integration_fsv.rs` was a production module because
`calyx-anneal/src/lib.rs` declared and exported it, while `fsv = []` was an
explicit public feature in `calyx-lodestar`.

## Root Cause

The no-test policy cleanup removed automated test targets, but some absorbed
Calyx production names still carried FSV/test-era terminology:

1. `calyx-lodestar` exposed a no-op `fsv` Cargo feature.
2. `calyx-anneal` used `integration_fsv` as the module name for the production
   `AnnealSubstrate` implementation.
3. `calyx-assay` formula coverage serialized `fsv_root` and `test` fields and
   stored FSV/test-like evidence strings.

These were not automated tests, but they contradicted the current doctrine by
presenting production capability and manual evidence as FSV/test surfaces.

## Before State

Issue inventory command:

```text
rg -n "\bmod\s+\w*fsv\b|\bpub\s+mod\s+\w*fsv\b|\w+_fsv\b|^fsv\s*=" calyx/crates -g '!target'
```

Before hits:

```text
calyx/crates/calyx-lodestar/Cargo.toml:11:fsv = []
calyx/crates/calyx-anneal/src/lib.rs:5:mod integration_fsv;
calyx/crates/calyx-anneal/src/lib.rs:57:pub use integration_fsv::{
calyx/crates/calyx-assay/src/formula_catalog.rs:192: "calyx-assay::formula_coverage_fsv"
calyx/crates/calyx-assay/src/formula_catalog.rs:200: "calyx-loom::stage5_fsv"
calyx/crates/calyx-assay/src/formula_catalog.rs:352: "calyx-ward::guard_ph37_fsv"
calyx/crates/calyx-assay/src/formula_catalog.rs:448: "calyx-sextant::causal_gate_fsv"
```

Formula catalog before-state also included:

```text
pub fsv_root: String
pub test: String
FORMULA_COVERAGE_ARTIFACT_KIND = "prd22.formula-coverage.v1"
FORMULA_COVERAGE_SCHEMA_VERSION = 1
FORMULA_COVERAGE_SOT_KEY = "formula_coverage/prd22"
```

## Change

- Removed the unused `fsv = []` Cargo feature from
  `calyx/crates/calyx-lodestar/Cargo.toml`.
- Renamed `calyx-anneal` production module:
  - `integration_fsv.rs` -> `substrate.rs`
  - `integration_fsv/artifact.rs` -> `substrate/artifact.rs`
  - `pub use integration_fsv::{...}` -> `pub use substrate::{...}`
- Renamed formula coverage serialized fields:
  - `fsv_root` -> `evidence_root`
  - `test` -> `evidence`
- Bumped formula coverage artifact identity:
  - `prd22.formula-coverage.v1` -> `prd22.formula-coverage.v2`
  - schema version `1` -> `2`
  - SoT key `formula_coverage/prd22` -> `formula_coverage/prd22/v2`
- Replaced FSV/test-like catalog evidence strings with manual evidence names.

## Manual Full State Verification

### Happy path - issue inventory has no FSV-named production surfaces

Trigger: apply the cleanup above.

Expected: the exact #1704 inventory pattern has no matches in `calyx/crates`.

After read:

```text
rg -n "\bmod\s+\w*fsv\b|\bpub\s+mod\s+\w*fsv\b|\w+_fsv\b|^fsv\s*=" calyx/crates -g '!target'
AFTER_1704_FSV_PATTERN_NO_MATCHES
```

### Edge 1 - old production module path is physically gone

Trigger: rename `integration_fsv` to `substrate`.

Expected: old file and directory absent; new production module files present.

After read:

```text
AFTER_1704_OLD_DIR_ABSENT
AFTER_1704_OLD_FILE_ABSENT
AFTER_1704_SUBSTRATE_FILE_PRESENT
AFTER_1704_SUBSTRATE_ARTIFACT_PRESENT
```

### Edge 2 - formula artifact rejects stale FSV/test schema names by construction

Trigger: rename serialized fields and bump schema identity.

Expected: no `fsv_root`, `test:`, `_tests`, or `_fsv` strings remain in
`formula_catalog.rs`; v2 evidence fields are present.

After read:

```text
rg -n "fsv_root|\btest\s*:|_tests\b|_fsv\b|stage[0-9]_fsv" calyx/crates/calyx-assay/src/formula_catalog.rs
AFTER_1704_FORMULA_STALE_NAMES_ABSENT

FORMULA_COVERAGE_ARTIFACT_KIND: "prd22.formula-coverage.v2"
FORMULA_COVERAGE_SCHEMA_VERSION: 2
FORMULA_COVERAGE_SOT_KEY: "formula_coverage/prd22/v2"
pub evidence_root: String
pub evidence: String
```

### Edge 3 - Cargo feature no longer exposes an FSV switch

Trigger: remove `[features] fsv = []`.

Expected: `calyx-lodestar/Cargo.toml` has no `[features]` section and no
`fsv` feature.

After read:

```text
Get-Content -Raw calyx\crates\calyx-lodestar\Cargo.toml
[dependencies]
blake3.workspace = true
...
[lints]
workspace = true
```

### Edge 4 - module graph still compiles after rename

Trigger: compile and lint the nested Calyx workspace.

Expected: production callers still resolve `AnnealSubstrate` and formula
coverage exports through the same public crate-level names.

After read:

```text
cargo check --manifest-path calyx/Cargo.toml --workspace
Finished `dev` profile ... target(s) in 14.55s

cargo clippy --manifest-path calyx/Cargo.toml --workspace --all-targets
Finished `dev` profile ... target(s) in 19.16s
```

## Structural Checks

These are compile/lint checks only, not FSV:

```text
cargo fmt --manifest-path calyx/Cargo.toml --all --check
exit 0

cargo check --manifest-path calyx/Cargo.toml --workspace
exit 0
warning: calyx-forge@0.1.0: cuda feature not enabled, skipping kernel compilation

cargo clippy --manifest-path calyx/Cargo.toml --workspace --all-targets
exit 0
warning: calyx-forge@0.1.0: cuda feature not enabled, skipping kernel compilation
```

## Follow-up Filed

The #1704 investigation also found broader `test`-named production APIs in
Calyx Lodestar. They are outside this issue's FSV-name scope and are tracked in
https://github.com/ChrisRoyse/Synapse/issues/1705.
