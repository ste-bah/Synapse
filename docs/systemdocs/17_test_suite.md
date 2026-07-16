# 17. Test Suite — REMOVED

**Status: there is no test suite.** As of the operator decision on **2026-07-15**, the
entire automated test surface was deleted from both the root Synapse workspace and the
vendored `calyx/` workspace. This document is retained only as a pointer to that decision.

## What was removed

- Every `#[test]` / `#[tokio::test]` function (~5,900 across both workspaces).
- Every inline `#[cfg(test)] mod tests` module and every external `*_tests.rs` / `tests.rs`
  test-module file.
- Every `tests/` (integration) and `benches/` (benchmark) directory.
- Every `[[test]]` / `[[bench]]` manifest target and all `[dev-dependencies]`.
- The dedicated test-support crates `synapse-test-utils` and `calyx-testkit`.
- The automated FSV *driver* examples (`*_fsv_driver`, `*_fsv_support`) — these violated the
  "FSV is never automated" rule.

## What was deliberately preserved

- `#[cfg(any(windows, test))]` and `#[cfg_attr(test, …)]` **production** seams — they gate real
  code, not tests. (Their `#[cfg(test)]`-only twins were collapsed to unconditional code.)
- Genuine diagnostic/utility examples (`dump_*`, `*_probe`) — product surface, not tests.

## The verification model now

Behavior is verified **only** by manual Full State Verification (FSV) against physical reality —
read the real Source of Truth, trigger through the real MCP tool, read the SoT again. See
[`AGENTS.md`](../../AGENTS.md) directive **D1** for the binding rule and the happy-path + ≥3
edge-case procedure, and [18_verification_report.md](18_verification_report.md) for FSV writeups.

The only automated checks that remain are **structural, not behavioral**: `cargo check` /
`cargo build` (compiles?), `cargo clippy --all-targets`, and `cargo fmt --check`, enforced
locally by `.githooks/pre-push` (D5). They are never FSV and never a shipping verdict on behavior.

**Do not re-introduce automated tests, benchmarks, dev-dependencies, or FSV driver harnesses.**
