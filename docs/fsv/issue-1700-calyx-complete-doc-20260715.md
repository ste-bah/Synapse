# Issue 1700 Manual FSV - Calyx complete documentation replacement

Date: 2026-07-15

## Source of Truth

- `calyx/CALYX_COMPLETE_DOCUMENTATION.md`
- Physical `calyx/` file tree and nested workspace crate directories
- `calyx/README.md`
- `docs/calyx/INTEGRATION_PLAN.md`
- `docs/calyx/SYNAPSE_ON_CALYX_CAPABILITIES.md`
- GitHub issue #1700

This issue is documentation-only. No Synapse MCP behavior was changed, so no MCP
tool call is the acceptance trigger for the fix. The verdict is the committed
file bytes and the separate file-tree readback.

## Root Cause

`calyx/CALYX_COMPLETE_DOCUMENTATION.md` was an unreproducible external snapshot
from `C:\code\Calyx-Dev`. It claimed to be generated from numbered source docs
by a generator script, but the current Synapse repo contains neither that
generator nor those numbered source docs. After #1699 removed the automated test
surface, the generated blob still advertised deleted support crates, command
paths, and historical test inventory. Keeping it would preserve a second,
stale source of truth.

Best-practice research used Exa MCP and native web search. The useful principle
was docs-as-code/source-of-truth discipline: documentation should live with the
code it describes and generated documentation should have auditable source
inputs in the repository.

References consulted:

- https://docs.aws.amazon.com/wellarchitected/latest/devops-guidance/dl.eac.5-integrate-technical-and-operational-documentation-into-the-development-lifecycle.html
- https://www.writethedocs.org/guide/docs-as-code/
- https://konghq.com/blog/learning-center/what-is-docs-as-code

## Before Read

Before the trigger, the file was a 12,943-line generated/historical bundle. The
stale-reference scan over the file found 139 hits for old generated-source,
testing, support-crate, and command references:

```text
calyx-fsv
calyx-testkit
cargo test
#[test]
#[tokio::test]
tests/
benches/
dev-dependencies
21_test_suite
22_verification_report
combine_docs.py
cargo-nextest
nextest
proptest
criterion
```

The file-tree source readback also showed no local regeneration surface:

```json
{
  "generator_or_numbered_source_docs": 0
}
```

## Trigger

Replaced `calyx/CALYX_COMPLETE_DOCUMENTATION.md` with a concise current-state
snapshot that:

- Retires the external generated bundle.
- Names the current Source of Truth files.
- Lists the actual current 15 Calyx crates from the physical tree.
- States the current structural-check-only verification policy.
- Adds a regeneration rule: do not restore a combined generated bundle unless
  the generator, source files, and manual FSV evidence ship with it.

## After Read

Separate post-trigger file and tree readback:

```json
{
  "stale_reference_hits": 0,
  "generator_or_numbered_source_docs": 0,
  "current_doc_lines": 64
}
```

Actual current Calyx crate inventory:

```text
calyx-anneal
calyx-assay
calyx-aster
calyx-core
calyx-forge
calyx-ledger
calyx-lodestar
calyx-loom
calyx-mincut
calyx-oracle
calyx-paths
calyx-registry
calyx-search
calyx-sextant
calyx-ward
```

Current Calyx no-test policy readback:

```json
{
  "calyx_test_or_bench_paths": 0,
  "calyx_rust_test_attrs": 0,
  "calyx_manifest_test_or_dev_hits": 0,
  "calyx_support_crate_paths": 0
}
```

## Manual Edge Audit

Happy path - stale generated bundle replaced:

- Before: 12,943-line external generated snapshot with 139 stale references.
- Trigger: replaced with current 64-line Synapse-owned summary.
- After: stale-reference scan reads `0`; file points to current Synapse docs and
  physical tree Source of Truth.

Edge 1 - missing generator/source docs:

- Before: file claimed generated provenance, but `calyx/` contained no generator
  script and no numbered source docs.
- Trigger: removed generated-bundle claim and documented that unreproducible
  generated docs must not be restored.
- After: local generator/source-doc inventory still reads `0`, and the current
  file no longer presents itself as regenerated output.

Edge 2 - crate-count drift:

- Before: old bundle advertised old upstream crate counts and deleted crates.
- Trigger: read actual directories under `calyx/crates`.
- After: file lists exactly the 15 directories present in the physical tree.

Edge 3 - no-test policy drift:

- Before: old bundle referenced deleted test paths, support crates, and command
  paths.
- Trigger: replaced those sections with structural-check-only policy.
- After: stale-reference scan reads `0`, and the actual Calyx tree reads zero
  for automated test paths, test attributes, manifest test/dev sections, and
  removed support-crate paths.

## Supporting Structural Check

```text
git diff --check: PASS
```

No tests were run or created.

## Result

`calyx/CALYX_COMPLETE_DOCUMENTATION.md` no longer contradicts the current
Synapse-owned Calyx tree or the no-test operator policy. It now acts as a
pointer to current documentation and physical file-tree Source of Truth rather
than an unreproducible stale generated artifact.
