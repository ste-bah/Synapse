# RECOVERY NOTES - Synapse

Resume by:
1. Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, #351, the open issue queue, and `STATE/*`.
2. Treat the old all-clear state as stale. #594 remains the open parent context; #589/#590/#588/#585/#635/#605/#606 are closed with RESOLVED evidence.
3. #606 closed at commit `6975d14` with evidence comment https://github.com/ChrisRoyse/Synapse/issues/606#issuecomment-4587883204.
4. Active issue is #607: `scenario(stress): act_launch fleet - all 30 profiles, foreground incl. console apps`.
   - START comment: https://github.com/ChrisRoyse/Synapse/issues/607#issuecomment-4587884557
   - Issue body requires proving `act_launch` starts/foregrounds every bundled-profile app, with `observe` resolving the app profile; explicitly cover cmd/powershell/Windows Terminal; SoT is foreground HWND/process plus `CF_PROCESS_HISTORY`/`CF_ACTION_LOG`.
   - Edges: app already running, wait-title never matches, launch denied by restrictive policy, rapid relaunch, invalid/empty params.
5. Next #607 step: inspect existing profile launch definitions and `act_launch` code paths, then build/launch an isolated repo-built daemon with strict Inspector `tools/list` before runtime FSV.

Do not use GitHub Actions/CI. Do not create FSV scripts or harnesses. For Synapse behavior FSV, prove the real `synapse-mcp` runtime and client-parity tool list before a real tool call, then read the physical SoT separately.
