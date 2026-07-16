# Issue #1703 FSV - calyx-aster no-test residue cleanup

Date: 2026-07-16
Repo: `C:\code\Synapse`
Issue: https://github.com/ChrisRoyse/Synapse/issues/1703

## Source Of Truth

This issue changes no runtime Synapse behavior and has no MCP trigger surface.
The source of truth is the repository source tree and Rust module graph:

- `calyx/crates/calyx-aster/src/vault/compaction_tests/`
- `calyx/crates/calyx-aster/src/vault/compaction_tests/support.rs`
- `calyx/crates/calyx-aster/src/stride_fsv.rs`
- `calyx/crates/calyx-aster/src/lib.rs`
- `rg` reachability over `calyx/crates`
- the final Git tree after commit

## Research

I used Exa plus web search against official Rust documentation. The relevant
rule is that Rust source files only enter the crate module graph when a parent
module declares them with `mod` or a path attribute. Official references:

- https://doc.rust-lang.org/reference/items/modules.html
- https://doc.rust-lang.org/reference/crates-and-source-files.html
- https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html

Applied conclusion: an unreferenced directory under `src/vault/` is dead source,
while `pub mod stride_fsv;` in `lib.rs` explicitly exported `stride_fsv.rs`.

## Root Cause

The earlier no-test cleanup removed test functions, test targets, and test
support crates, but it did not catch two source-tree residues:

1. An unreferenced `vault/compaction_tests/support.rs` helper file with
   `assert`, `unwrap`, synthetic data, temp-dir, and readback helpers.
2. A public `stride_fsv` module exporting a self-contained external-command
   allowlist gate that no production caller referenced.

Because these lived in production source paths, they survived the no-test sweep
even though they were not part of the active production module graph.

## Before State

Physical file tree:

```text
Get-ChildItem -Recurse -Force calyx\crates\calyx-aster\src\vault\compaction_tests
support.rs  Length=5622
```

Module and symbol inventory:

```text
calyx/crates/calyx-aster/src/lib.rs:35:pub mod stride_fsv;
calyx/crates/calyx-aster/src/stride_fsv.rs:1://! STRIDE defense FSV proofs for PH61 T06.
calyx/crates/calyx-aster/src/stride_fsv.rs:6:pub const CALYX_EXTERNAL_CMD_NOT_ALLOWED
calyx/crates/calyx-aster/src/stride_fsv.rs:9:pub fn run_external_cmd(...)
```

Reachability check:

```text
rg "mod compaction_tests|compaction_tests::|vault::compaction_tests" calyx/crates
no matches
```

`stride_fsv` dependency check:

```text
rg "calyx_aster::stride_fsv|stride_fsv::|run_external_cmd|CALYX_EXTERNAL_CMD_NOT_ALLOWED" .
only matches inside calyx/crates/calyx-aster/src/stride_fsv.rs
```

## Change

- Removed `pub mod stride_fsv;` from `calyx/crates/calyx-aster/src/lib.rs`.
- Deleted `calyx/crates/calyx-aster/src/stride_fsv.rs`.
- Deleted `calyx/crates/calyx-aster/src/vault/compaction_tests/support.rs`.
- Removed the now-empty `compaction_tests` directory from the working tree.

## Manual Full State Verification

### Happy path - dead compaction support removed

Trigger: delete `vault/compaction_tests/support.rs` and the empty directory.

Expected: no physical `compaction_tests` directory remains.

After read:

```text
if (Test-Path 'calyx\crates\calyx-aster\src\vault\compaction_tests') ...
AFTER_EDGE_DIR_ABSENT
```

### Edge 1 - public FSV module file removed

Trigger: delete `calyx-aster/src/stride_fsv.rs`.

Expected: no physical file remains.

After read:

```text
if (Test-Path 'calyx\crates\calyx-aster\src\stride_fsv.rs') ...
AFTER_EDGE_STRIDE_FILE_ABSENT
```

### Edge 2 - invalid deleted-module export prevented

Trigger: remove `pub mod stride_fsv;` from `lib.rs`.

Expected: module graph no longer references the deleted file and the workspace
still compiles.

After reads:

```text
rg "pub mod stride_fsv|mod stride_fsv|stride_fsv" calyx/crates/calyx-aster/src calyx/crates
FSV_AFTER_NO_STALE_SYMBOLS

cargo check --manifest-path calyx/Cargo.toml --workspace
Finished `dev` profile ... target(s) in 20.54s
```

### Edge 3 - stale call sites or exported symbols absent

Trigger: repository-wide stale-symbol scan.

Expected: no residual references to the deleted support tree or deleted public
symbols.

After read:

```text
rg "pub mod stride_fsv|mod stride_fsv|stride_fsv|compaction_tests|CALYX_EXTERNAL_CMD_NOT_ALLOWED|run_external_cmd" calyx/crates/calyx-aster/src calyx/crates -g '!target'
FSV_AFTER_NO_STALE_SYMBOLS
```

### Edge 4 - Git worktree reflects the intended deletion set

Trigger: inspect Git deletion state before commit.

Expected: the two tracked residue files are deleted; `lib.rs` is modified.

After read:

```text
git status --short
 M calyx/crates/calyx-aster/src/lib.rs
 D calyx/crates/calyx-aster/src/stride_fsv.rs
 D calyx/crates/calyx-aster/src/vault/compaction_tests/support.rs

git ls-files --deleted ...
calyx/crates/calyx-aster/src/stride_fsv.rs
calyx/crates/calyx-aster/src/vault/compaction_tests/support.rs
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

The #1703 investigation found additional FSV-named surfaces outside
`calyx-aster`. They are tracked separately in
https://github.com/ChrisRoyse/Synapse/issues/1704.
