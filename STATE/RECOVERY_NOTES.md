# RECOVERY NOTES - Synapse

Resume by:
1. Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, #351, the open issue queue, and `STATE/*`.
2. Treat the old all-clear state as stale. #594 remains the open parent context; #589/#590/#588/#585/#635/#605 are closed with RESOLVED evidence.
3. #605 closed at commit `e0ea7e1` with evidence comment https://github.com/ChrisRoyse/Synapse/issues/605#issuecomment-4587679836.
4. Active issue is #606: `scenario(stress): act_run_shell orchestration - allowlist modes, timeout, 1MB cap, idempotency`.
   - START comment: https://github.com/ChrisRoyse/Synapse/issues/606#issuecomment-4587680954
   - Implementation patch is currently unstaged in `crates/synapse-mcp/src/m4.rs`, `crates/synapse-mcp/src/server.rs`, and `crates/synapse-mcp/src/server/m4_tools.rs`.
   - Manual FSV evidence is captured in:
     - `.runs\606\permissive-20260531T140952` for permissive shell, env containment, output cap, timeout, idempotency, conflict, empty command, default/max timeout.
     - `.runs\606\restrictive-20260531T141636` for allowlisted command and denied command with `SAFETY_SHELL_DENIED_BY_POLICY`.
     - `.runs\606\malformed-20260531T142400` for malformed regex startup fail-closed.
     - `.runs\606\above-max-20260531T142425` for `timeout_ms=600001` rejection and action-log started/error rows.
   - All isolated #606 daemons were stopped; ports `7799`, `7800`, `7801`, and `7802` were verified closed. Preserve wired chat MCP PID `45712`.
5. Final #606 supporting checks and diff review are complete: `cargo fmt --check`, `cargo check -p synapse-mcp`, focused shell idempotency/timeout tests, `cargo clippy -p synapse-mcp --all-targets -- -D warnings`, `cargo build --release -p synapse-mcp`, and `git diff --check` passed.
6. Next #606 step: commit/push with `[skip ci]`, post #606 RESOLVED evidence, close #606, then refresh the live queue and take the next open issue.

Do not use GitHub Actions/CI. Do not create FSV scripts or harnesses. For Synapse behavior FSV, prove the real `synapse-mcp` runtime and client-parity tool list before a real tool call, then read the physical SoT separately.
