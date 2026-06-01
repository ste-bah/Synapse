# RECOVERY NOTES - Synapse

## Current Resume Point - 2026-06-01T05:41:10-05:00
- Active issue: #612 `scenario(stress): hold_move / hold_button / combo reflex lifetimes`.
- START comment: https://github.com/ChrisRoyse/Synapse/issues/612#issuecomment-4590869661
- Required wake-up has been completed after the latest compaction: doctrine files, `STATE/*`, #612/#594/#351, live open queue, git status/log/branch, and configured wired `mcp__synapse` health/storage/reflex/observe calls.
- Worktree contains #612 patches for:
  - `hold_move.re_assert` dispatch plus 50 ms reassert throttling.
  - cancel-time physical release actions for active `hold_move`/`hold_button`.
  - logical no-op suppression in the action crash-recovery ledger.
  - EventBus subscriber drop cleanup.
  - tick-late audit/event coalescing for repeated identical late runs.
  - aim-track scheduler guard so non-aim reflex ticks do not sample M1 target sources.
  - `reflex_cancel` historical terminal-status lookup from `CF_REFLEX_AUDIT` before returning `NotFound`.
- Latest release binary after the cancel-expired patch:
  - `target\release\synapse-mcp.exe`
  - length `46342656`
  - SHA256 `6898E30AE4FAE8519499B0BB91436E3C0B44D218BE03539EA1D60957C1281BF1`
  - timestamp `2026-06-01T05:41:10-05:00`
- Manual #612 behavior evidence already captured:
  - Full hold/lifetime/edge evidence from `.runs\612\hold-lifetime-fsv-20260601T045248-throttle`: hold_move UntilCancelled, re_assert under external key-up/focus-loss proxy, hold_button mouse, hold_button XInput pad, one-shot combo expiry, Duration 1ms, deadline already past, UntilEvent via real `observe_delta`, empty keys, boundary priority, and structurally invalid button.
  - Fresh patched cancel-expired evidence from `.runs\612\hold-lifetime-fsv-20260601T0530-cancel-expired`: repo-built daemon PID `53088`, bind `127.0.0.1:7838`, strict Inspector `tools/list` 80 tools, expired combo `019e82be-1a45-7d00-a817-22a9d7248818`, `reflex_cancel` response `cancelled=false, reason=already_expired`, OS P false before/after, recovery ledger absent before/after, `CF_REFLEX_AUDIT=2`, daemon stopped afterward.
- Supporting checks already passed after the latest patch:
  - `cargo fmt`
  - `cargo test -p synapse-reflex cancel_expired_reflex_restored_from_audit_reports_already_expired --lib -- --nocapture`
  - `cargo check -p synapse-reflex -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo build --release -p synapse-mcp -j 2`
- Final broad supporting checks passed:
  - `cargo fmt --check`
  - `git diff --check` (line-ending warnings only)
  - `cargo check -p synapse-action -j 2`
  - `cargo check -p synapse-reflex -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-action recovery_log_skips_duplicate_logical_holds --lib -- --nocapture`
  - `cargo test -p synapse-reflex cancel_expired_reflex_restored_from_audit_reports_already_expired --lib -- --nocapture`
  - `cargo test -p synapse-reflex --test hold_move_behavior -- --nocapture`
  - `cargo test -p synapse-reflex --test bus_behavior -- --nocapture`
  - `cargo test -p synapse-reflex --test scheduler_behavior -- --nocapture`
  - `cargo test -p synapse-reflex --test combo_behavior -- --nocapture`
  - `cargo test -p synapse-mcp --test m3_reflex_register_tool -- --nocapture`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo build --release -p synapse-mcp -j 2`
- All FSV-owned #612 daemons are stopped. Only the configured chat stdio daemon PID `45712` remains.

## Next Steps
1. Commit/push with `[skip ci]`.
2. Post #612 RESOLVED evidence and close #612.
3. Refresh the open queue and continue to the next issue, likely #613.

## Standing Rules
- No GitHub Actions/CI dispatch, waits, or CI-gated claims.
- Commits pushed by this agent must include `[skip ci]`.
- Automated checks/benches are supporting regression evidence only; they are not FSV.
- Missing local prerequisites are acquisition/setup work, not blockers, unless only a specific operator-only hard-to-reverse external action remains.
