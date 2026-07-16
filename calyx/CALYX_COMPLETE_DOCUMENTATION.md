# Calyx Documentation - Current Synapse Snapshot

This file replaces the old generated `CALYX_COMPLETE_DOCUMENTATION.md` bundle.
The previous bundle was an external `C:\code\Calyx-Dev` snapshot. It referenced
source docs and a generator script that are not present in this repository, so
it could not be regenerated from the current Synapse-owned Calyx tree. Keeping a
large unreproducible generated copy made it drift from reality.

## Current Source Of Truth

Use these files instead of historical upstream snapshots:

- `calyx/README.md` - ownership, nested workspace rules, local structural gates,
  CUDA posture, kept/pruned crate list, license.
- `calyx/BUILDING_ON_CALYX.md` - Calyx design doctrine for Synapse work.
- `docs/calyx/INTEGRATION_PLAN.md` - current integration plan and issue graph.
- `docs/calyx/SYNAPSE_ON_CALYX_CAPABILITIES.md` - target capabilities once the
  integration issue graph is closed.
- `docs/fsv/issue-1699-no-test-policy-20260715.md` - manual FSV evidence for
  the zero automated-test policy.
- The physical file tree under `calyx/` - final authority for what crates,
  manifests, paths, and source files exist.

## Current Workspace Inventory

As of 2026-07-15 on Synapse `origin/main`, `calyx/` is a nested Cargo workspace
with 15 Synapse-owned crates:

- `calyx-anneal`
- `calyx-assay`
- `calyx-aster`
- `calyx-core`
- `calyx-forge`
- `calyx-ledger`
- `calyx-lodestar`
- `calyx-loom`
- `calyx-mincut`
- `calyx-oracle`
- `calyx-paths`
- `calyx-registry`
- `calyx-search`
- `calyx-sextant`
- `calyx-ward`

The old upstream applications, servers, generated docs, data/assets, fuzzing
scaffolds, and support-only crates were pruned during absorption. Synapse owns
the remaining crates directly; there is no upstream sync contract.

## Current Verification Policy

The repository carries no automated behavioral test or benchmark surface. For
Calyx changes, use only structural support checks:

```powershell
cargo check --manifest-path calyx\Cargo.toml --workspace
cargo fmt --manifest-path calyx\Cargo.toml --all --check
cargo clippy --manifest-path calyx\Cargo.toml --workspace --all-targets
```

These commands are compile, format, and lint evidence only. They are not Full
State Verification. Behavioral acceptance is manual FSV against the physical
Source of Truth named by the issue being shipped.

## Regeneration Rule

Do not restore a combined generated documentation bundle unless its source
documents and generator live in this repository and the output can be rebuilt
from current Synapse-owned inputs. If a future generated doc is needed, the
generator, source files, and manual FSV evidence must ship together in the same
commit.

## Research Notes

The replacement follows docs-as-code/source-of-truth guidance: documentation
should live with the code it describes, have an auditable source, and avoid
duplicated generated copies that cannot be rebuilt from the current repository.
References consulted while resolving #1700:

- https://docs.aws.amazon.com/wellarchitected/latest/devops-guidance/dl.eac.5-integrate-technical-and-operational-documentation-into-the-development-lifecycle.html
- https://www.writethedocs.org/guide/docs-as-code/
- https://konghq.com/blog/learning-center/what-is-docs-as-code
