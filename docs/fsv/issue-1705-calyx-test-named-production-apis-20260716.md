# Issue 1705 Manual FSV - Calyx Test-Named Production APIs

Date: 2026-07-16

Issue: <https://github.com/ChrisRoyse/Synapse/issues/1705>

## Source Of Truth

The Source of Truth is the Git/source tree for the real Calyx crates:

- `calyx/crates/calyx-lodestar/src`
- `calyx/crates/calyx-oracle/src/super_intel.rs`
- The committed Git tree after the fix

This issue is a source-contract cleanup, not a runtime MCP tool behavior. No
automated tests, benches, FSV harnesses, FSV scripts, mocks, or CI were used.

## Root Cause

The 2026-07-15 no-test cleanup removed automated tests, but production Calyx
APIs still used `test` terminology for real recall evaluation and hypothesis
falsification data. That made production source look like no-test-policy
residue. The risky serialized field was `RecallReport.recall_test_params`: a
plain rename to an optional field could deserialize old artifacts as `None`.

## Research

Research used Exa MCP and web search before the final implementation.

- Rust Reference, module source filenames: <https://doc.rust-lang.org/reference/items/modules.html>
- Rust Reference, crates and source files: <https://doc.rust-lang.org/reference/crates-and-source-files.html>
- Serde field attributes: <https://serde.rs/field-attrs.html>
- Serde container attributes: <https://serde.rs/container-attrs>

Applied guidance:

- Rust module names and file names should mirror the logical module path, so
  `recall_test.rs` became `recall_eval.rs` and `pub mod recall_eval`.
- Serde field names are the serialized contract unless explicitly renamed or
  aliased. Because the operator requested no fallbacks, no aliases were added.
- `serde(deny_unknown_fields)` plus schema/format version bumps make old
  `recall_test_params` and `falsification_test(s)` payloads fail closed instead
  of being silently accepted as missing renamed fields.

## Fix

- Renamed production recall API:
  - `recall_test.rs` -> `recall_eval.rs`
  - `RecallTestParams` -> `RecallEvalParams`
  - `RecallTestReport` -> `RecallEvaluationReport`
  - `kernel_recall_test*` -> `measure_kernel_recall*`
  - `recall_test_params` -> `recall_eval_params`
- Renamed production falsification fields:
  - `falsification_test` -> `falsification_probe`
  - `falsification_tests` -> `falsification_probes`
- Bumped serialized contracts:
  - `KERNEL_ARTIFACT_FORMAT_VERSION` 1 -> 2
  - `HYPOTHESIS_EVALUATION_SCHEMA_VERSION` 1 -> 2
- Added `#[serde(deny_unknown_fields)]` to the renamed serialized payload
  structs that would otherwise be vulnerable to silent old-field acceptance.

## Before State Read

Command:

```text
git grep -n -E "recall_test|RecallTest|kernel_recall_test|recall_test_params|falsification_test|falsification_tests" HEAD -- calyx/crates/calyx-lodestar/src calyx/crates/calyx-oracle/src/super_intel.rs
```

Observed Source of Truth before trigger included these real source entries:

```text
HEAD:calyx/crates/calyx-lodestar/src/lib.rs:33:pub mod recall_test;
HEAD:calyx/crates/calyx-lodestar/src/kernel.rs:38:    pub recall_test_params: Option<RecallTestParams>,
HEAD:calyx/crates/calyx-lodestar/src/recall_test.rs:32:pub struct RecallTestParams {
HEAD:calyx/crates/calyx-lodestar/src/recall_test.rs:148:pub fn kernel_recall_test(
HEAD:calyx/crates/calyx-lodestar/src/hypothesis_evaluation.rs:56:    pub falsification_test: String,
HEAD:calyx/crates/calyx-lodestar/src/hypothesis_evaluation.rs:100:    pub falsification_tests: Vec<String>,
HEAD:calyx/crates/calyx-oracle/src/super_intel.rs:9:    AnnIndex, CorpusReader, KernelIndex, LodestarError, RecallReport, RecallTestParams,
```

Old module path before state:

```text
BEFORE_OLD_MODULE_PRESENT
```

## Trigger

Manual source edit and file rename in the real worktree:

```text
calyx/crates/calyx-lodestar/src/recall_test.rs renamed to calyx/crates/calyx-lodestar/src/recall_eval.rs
```

## After State Read

Command:

```text
rg -n "recall_test|RecallTest|kernel_recall_test|recall_test_params|falsification_test|falsification_tests" calyx/crates/calyx-lodestar/src calyx/crates/calyx-oracle/src/super_intel.rs -g '!target'
```

Observed after state:

```text
exit code 1; no matches
```

Path Source of Truth after trigger:

```text
Test-Path calyx/crates/calyx-lodestar/src/recall_test.rs => False
Test-Path calyx/crates/calyx-lodestar/src/recall_eval.rs => True
```

New contract readback:

```text
calyx/crates/calyx-lodestar/src/kernel_health.rs:15:pub const KERNEL_ARTIFACT_FORMAT_VERSION: u32 = 2;
calyx/crates/calyx-lodestar/src/kernel.rs:31:#[serde(deny_unknown_fields)]
calyx/crates/calyx-lodestar/src/kernel.rs:39:    pub recall_eval_params: Option<RecallEvalParams>,
calyx/crates/calyx-lodestar/src/hypothesis_evaluation.rs:8:pub const HYPOTHESIS_EVALUATION_SCHEMA_VERSION: u32 = 2;
calyx/crates/calyx-lodestar/src/hypothesis_evaluation.rs:48:#[serde(deny_unknown_fields)]
calyx/crates/calyx-lodestar/src/hypothesis_evaluation.rs:57:    pub falsification_probe: String,
calyx/crates/calyx-lodestar/src/hypothesis_evaluation.rs:83:#[serde(deny_unknown_fields)]
calyx/crates/calyx-lodestar/src/hypothesis_evaluation.rs:102:    pub falsification_probes: Vec<String>,
```

## Boundary And Edge Case Audit

### Happy Path - Production API Inventory

Before: `HEAD` exported `pub mod recall_test`, `RecallTestParams`, and
`kernel_recall_test*`.

After: the real source tree exports `pub mod recall_eval`, `RecallEvalParams`,
`RecallEvaluationReport`, and `measure_kernel_recall*`.

Observed after read:

```text
calyx/crates/calyx-lodestar/src/lib.rs:33:pub mod recall_eval;
calyx/crates/calyx-lodestar/src/lib.rs:168:    RecallEvalParams, RecallEvaluationReport, RecallQuery, RecallSupportReport,
calyx/crates/calyx-lodestar/src/lib.rs:170:    measure_kernel_recall, measure_kernel_recall_with_clock,
```

### Edge 1 - Empty Old-Name Inventory

Before: stale-name inventory over `HEAD` returned production matches in
`lib.rs`, `kernel.rs`, `recall_test.rs`, `summarize.rs`, `vault_kernel.rs`,
`aster_bridge.rs`, `hypothesis_evaluation.rs`, and `calyx-oracle`.

After: the same inventory over the working Source of Truth returned no matches.

```text
exit code 1; no matches
```

### Edge 2 - Boundary Module Path

Before:

```text
git cat-file -e HEAD:calyx/crates/calyx-lodestar/src/recall_test.rs => BEFORE_OLD_MODULE_PRESENT
```

After:

```text
Test-Path calyx/crates/calyx-lodestar/src/recall_test.rs => False
Test-Path calyx/crates/calyx-lodestar/src/recall_eval.rs => True
```

### Edge 3 - Structurally Invalid Old Serialized Fields

Before: `RecallReport` had optional `recall_test_params`, which could be
dangerous if renamed without a strict deserialization boundary.

After: old kernel artifacts are rejected by `KERNEL_ARTIFACT_FORMAT_VERSION = 2`;
old or extra recall payload fields are rejected by `#[serde(deny_unknown_fields)]`;
old hypothesis evaluation payload shape is versioned as
`HYPOTHESIS_EVALUATION_SCHEMA_VERSION = 2`.

Observed after read:

```text
calyx/crates/calyx-lodestar/src/kernel_health.rs:59:    if snapshot.format_version != KERNEL_ARTIFACT_FORMAT_VERSION {
calyx/crates/calyx-lodestar/src/kernel.rs:31:#[serde(deny_unknown_fields)]
calyx/crates/calyx-lodestar/src/hypothesis_evaluation.rs:48:#[serde(deny_unknown_fields)]
calyx/crates/calyx-lodestar/src/hypothesis_evaluation.rs:83:#[serde(deny_unknown_fields)]
```

## Structural Checks

These are compile/lint checks only, not FSV.

```text
cargo fmt --manifest-path calyx/Cargo.toml --all --check
PASS

cargo check --manifest-path calyx/Cargo.toml --workspace
PASS
warning: calyx-forge@0.1.0: cuda feature not enabled, skipping kernel compilation

cargo clippy --manifest-path calyx/Cargo.toml --workspace --all-targets
PASS
warning: calyx-forge@0.1.0: cuda feature not enabled, skipping kernel compilation
```

## Evidence Of Success

The actual data residing in the system after execution is the working source
tree with:

- No stale production `recall_test` / `RecallTest` / `kernel_recall_test` /
  `recall_test_params` / `falsification_test(s)` symbols under the issue scope.
- New recall evaluation API and module physically present.
- Kernel artifact and hypothesis evaluation serialized contracts bumped.
- Strict Serde unknown-field rejection on the renamed payloads.

