# RECOVERY NOTES - Synapse

Current resume point:
- Active issue: #610 `scenario(stress): aim_track reflex - moving target + track-loss`.
- Acceptance evidence is captured in `.runs\610\aim-track-accept-20260531T2351`.
- Repo-built isolated daemon PID `74696` on `127.0.0.1:7831` was stopped after cleanup; port is closed. Do not try to reuse the daemon.
- Manual evidence completed:
  - MCP preconditions: process/socket/auth/health and official Inspector strict `tools/list` with 80 tools.
  - Happy path: real Inspector `reflex_register` on Notepad document element, Win32 window move, DPI-aware cursor readback at target center, and `reflex_history` correction rows.
  - Track loss: Notepad hidden and foreground switched to VS Code; `observe` lost the element; `reflex_list` expired the reflex with `REFLEX_TRACK_LOST`; `reflex_history` contains `reflex_track_lost`.
  - Edges: X-only, Y-only, deadzone larger than error, max-speed clamp, target teleport, boundary `priority=1000`, empty missing-target params, and structurally invalid element target.
  - Cleanup: `release_all` zero held state; final list read `total=7 active=0 cancelled=6 expired=1`; final storage read `CF_REFLEX_AUDIT=4956`, `CF_ACTION_LOG=1`, `CF_EVENTS=6`, `CF_OBSERVATIONS=6`.
- Next exact steps:
  1. Commit and push with `[skip ci]`.
  2. Post #610 RESOLVED evidence and close the issue.
  3. Refresh the open queue and continue.
- Final supporting checks already completed:
  - `cargo fmt --check`
  - `cargo check -p synapse-reflex`
  - `cargo check -p synapse-mcp`
  - `cargo test -p synapse-reflex --test scheduler_behavior -- --nocapture` (21 passed)
  - `cargo test -p synapse-mcp aim_track_target_source_reads_shallow_observe_child_elements -- --nocapture`
  - `cargo test -p synapse-mcp --bin synapse-mcp rebase_nodes_to_foreground -- --nocapture` after cleaning the stale `windows` artifact from a host paging-file failure
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo build --release -p synapse-mcp`
  - `git diff --check` (only LF/CRLF warnings)
- Final release binary: `target\release\synapse-mcp.exe`, SHA256 `478BDD601E6CE5CAD19465FE8D43E01BB1837135340B350A4C3E93FC32290F6A`, length `46315008`, timestamp `2026-06-01T05:27:02Z`.

Resume by:
1. Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, #351, the open issue queue, and `STATE/*`.
2. Treat the old all-clear state as stale. #594 remains the open parent context; #589/#590/#588/#585/#635/#605/#606/#607/#608 are closed with RESOLVED evidence.
3. #606 closed at commit `6975d14` with evidence comment https://github.com/ChrisRoyse/Synapse/issues/606#issuecomment-4587883204.
4. #607 is closed with commit `8ce49e4` and RESOLVED evidence https://github.com/ChrisRoyse/Synapse/issues/607#issuecomment-4588670440.
5. #608 is closed with commit `5873c37` and RESOLVED evidence https://github.com/ChrisRoyse/Synapse/issues/608#issuecomment-4589052871.
6. #609 is closed with commit `b7ecd73` and RESOLVED evidence https://github.com/ChrisRoyse/Synapse/issues/609#issuecomment-4589227231.
7. Active issue is #610: `scenario(stress): aim_track reflex - moving target + track-loss`.
   - START comment: https://github.com/ChrisRoyse/Synapse/issues/610#issuecomment-4589229027
   - Issue body requires proving `aim_track` follows a moving target and handles target loss. SoTs include cursor position (`GetCursorPos`), target/observation state, `reflex_history` / `CF_REFLEX_AUDIT`, `storage_inspect`, daemon logs, process/socket state, and cleanup state.
   - Edges: X-only / Y-only axis, deadzone larger than error, target teleport, max_speed clamp, plus empty/boundary/structurally invalid params.
8. Current #610 implementation checkpoint:
   - `aim_track` has a dynamic target-source abstraction, MCP installs an M1-backed source, dynamic targets participate in stateful conflict planning, correction rows are persisted to `CF_REFLEX_AUDIT`, and track loss expires/deactivates the reflex with a `REFLEX_TRACK_LOST` audit row.
   - Checks passed before the bbox patch: `cargo fmt`, `cargo check -p synapse-reflex`, `cargo check -p synapse-mcp`, focused dynamic target/loss regression, full `scheduler_behavior` (21 passed), MCP `schema_sanitize`, and `cargo build --release -p synapse-mcp`.
   - Latest repo-built binary after bbox patch: `target\release\synapse-mcp.exe`, SHA256 `0E1DFB375963D808D4DE5A22A926CA4762DD57DC0AF6CB3A3C585E8FDFE349C7`, length `46315008`, timestamp `2026-06-01T04:37:08Z`.
9. The first #610 manual FSV attempt in `.runs\610\aim-track-final-20260531T2255` failed and must not be used as acceptance:
   - Repo-built daemon PID `60972` on `127.0.0.1:7828` had passed process/socket/auth/health/strict Inspector `tools/list` preconditions.
   - Real `reflex_register` registered Notepad text editor target `0x261c94:0000002a00261c94`, but after movement cursor stayed `(20,20)`, `fire_count=0`, and `CF_REFLEX_AUDIT` had zero `aim_track_correction` rows.
   - `reflex_history` showed `REFLEX_TRACK_LOST` at `lost_for_ms=508`; target context proved the live source saw only root window element `0x325d2:0000002a000325d2`.
   - Root cause: `AIM_TRACK_TARGET_SOURCE_DEPTH` was `0`, so scheduler ticks could not resolve child elements selected from normal `observe` output.
10. Patch added after that failure:
   - `crates/synapse-mcp/src/server/context.rs`: set aim-track target source depth to `2` and added `aim_track_target_source_reads_shallow_observe_child_elements`.
   - `crates/synapse-mcp/src/m1.rs`: synthetic input respects requested depth so tests can expose depth mismatches.
   - Checks passed: `cargo fmt`; focused MCP regression; `cargo check -p synapse-mcp`; `cargo check -p synapse-reflex`.
11. Second #610 manual FSV attempt in `.runs\610\aim-track-depth2-20260531T2315` exposed a moved-window bbox bug and must not be used as acceptance:
   - Repo-built daemon PID `51264` on `127.0.0.1:7829` passed process/socket/auth/health/strict Inspector `tools/list` preconditions.
   - Real `reflex_register` against Notepad child element `0x261c94:0000002a00261c94` produced `aim_track_correction` rows and `fire_count>0`, proving the depth-2 target-source patch fixed the prior immediate target-loss problem.
   - Moving the Notepad window exposed inconsistent SoT: Win32 foreground bounds moved to the new coordinates, but UIA root/child bboxes stayed at the old coordinates; `reflex_history` showed the new reflex chasing stale target center `(1170,807)`.
12. Patch added after the moved-window defect:
   - `crates/synapse-mcp/src/m1/sources.rs`: Windows `platform_input` rebases UIA element bboxes by the foreground-window delta when the root HWND matches and root dimensions match. This keeps `observe` and `aim_track` child-element target resolution aligned with current physical window position.
   - Added tests `rebase_nodes_to_foreground_shifts_stale_uia_rects_when_root_size_matches` and `rebase_nodes_to_foreground_leaves_different_sized_roots_unchanged`.
   - Checks passed: `cargo fmt`; `cargo check -p synapse-mcp`; `cargo test -p synapse-mcp rebase_nodes_to_foreground -- --nocapture`; `cargo check -p synapse-reflex`; release build completed with binary SHA256 `0E1DFB375963D808D4DE5A22A926CA4762DD57DC0AF6CB3A3C585E8FDFE349C7`.
13. Next #610 step: launch an isolated repo-built daemon on a fresh port, prove process/socket/auth/health/strict Inspector `tools/list`, then rerun manual FSV with real MCP `tools/call` triggers against the moving Notepad child target and separate cursor/observation/storage/history/log SoT readbacks. Required edges still pending: track loss, X-only/Y-only axis, deadzone larger than error, target teleport, max-speed clamp, empty/boundary/structurally invalid params.

Closed #609 reference notes:

- Issue #609: `scenario(stress): 1ms reflex tick jitter under system load`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/609#issuecomment-4589054448
  - Commit: `b7ecd73 fix(reflex): persist tick-late audit rows (#609) [skip ci]`
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/609#issuecomment-4589227231
  - Patch: `REFLEX_TICK_LATE` persists to `CF_REFLEX_AUDIT` as `reflex_id="__scheduler__"` with elapsed/jitter/target/late-after/fallback/reason/degraded details; health exposes p99 jitter, late retained-sample count, and degraded retained-sample count; supporting scheduler test asserts the persisted row.
  - Manual FSV captured idle baseline, real `act_run_shell` CPU load, 16 subscribers, invalid/boundary edges, and forced degraded fallback through repo-built daemons and strict Inspector tools/list. Cleanup stopped PIDs `72424`/`16712` and closed ports `7826`/`7827`.
  - Final checks passed after FSV: `cargo fmt --check`, `cargo check -p synapse-core`, `cargo check -p synapse-reflex`, focused tick-late scheduler test, full `scheduler_behavior` (20 passed), `cargo check -p synapse-mcp`, `cargo test -p synapse-mcp schema_sanitize -- --nocapture`, release build, and `git diff --check`.

Closed #608 reference notes:

- Issue #608: `scenario(stress): 32-reflex saturation - priority, exclusive, starvation`.
   - START comment: https://github.com/ChrisRoyse/Synapse/issues/608#issuecomment-4588672100
   - Issue body requires registering 32 concurrent reflexes, 33rd fail-closed, priority/exclusive arbitration, starvation detection after `STARVATION_AFTER`, and SoT readbacks from `reflex_list`, `reflex_history`, and `CF_REFLEX_AUDIT`.
   - Edges: priority `0` and `1000` bounds, duplicate registration, cancel mid-fire, all 32 firing same tick / sample cap, empty/boundary/structurally invalid params.
- Current #608 implementation status:
   - Patch is in the worktree for `synapse-reflex` exclusive same-device-class arbitration, combo conflict resources, duplicate reflex-id validation, duplicate active runtime registration rejection, and health `sample_count`/`sample_limit` readback.
   - Additional root-cause patch after the latest compaction: stateful controllers now participate in conflict arbitration before dispatch, and starvation losses are aggregated once per tick. This fixes the observed real-runtime defect where two exclusive `aim_track` reflexes both fired continuously.
   - Supporting checks already passed: `cargo fmt`, `cargo fmt --check`, `cargo check -p synapse-reflex`, `cargo check -p synapse-mcp`, `cargo check -p synapse-core`, full `cargo test -p synapse-reflex --test scheduler_behavior -- --nocapture`, focused stateful/exclusive/priority/cancel/duplicate tests, MCP `reflex_register_schema_defaults_and_edges`, schema sanitize tests, and `cargo build --release -p synapse-mcp`. Parallel scheduler test attempts that hit `LNK1104` were rerun sequentially and passed.
- Final #608 manual FSV status:
   - Cap/invalid daemon PID `34784` on `127.0.0.1:7823` is stopped/closed. Evidence: strict Inspector `tools/list` 80; duplicate fail-closed; 32 accepted; 33rd rejected; priority 0/1000 accepted; empty/invalid edges rejected; final cleanup active 0, disabled 32, `CF_REFLEX_AUDIT=64`, `CF_ACTION_LOG=1`.
   - Starvation daemon PID `59948` on `127.0.0.1:7824` is stopped/closed. Evidence: two exclusive `aim_track` reflexes; winner fired, loser starved with `REFLEX_STARVED` and `mouse_cursor`; canceling winner let loser fire.
   - Same-tick daemon PID `70124` on `127.0.0.1:7825` was cleaned after compaction. Evidence: unauth health 401; strict Inspector `tools/list`; 32 hold_button reflexes all fired exactly once on tick 0; daemon log `dispatched_actions=32`; sample ring 4096/4096; cleanup `release_all` neutralized 3 pads; after-read active 0, disabled 32, `CF_REFLEX_AUDIT=64`, `CF_ACTION_LOG=1`; PID absent and port closed.
- #608 was committed as `5873c37`, evidence was posted, issue was closed, and `main` was pushed with `[skip ci]`.

Closed #607 reference notes:

Current #607 resume point as of 2026-05-31T16:15:13-05:00:
- The latest patch also touches `crates/synapse-a11y/src/platform/windows/window.rs` plus `cmd.toml`/`powershell.toml`.
- Root causes fixed in code:
  - console targets now request `CREATE_NEW_CONSOLE`;
  - action audit reads fast foreground metadata instead of a depth-1 UIA subtree snapshot;
  - cmd/powershell profiles include title-specific `WindowsTerminal.exe`/`wt.exe` matches;
  - `focus_window` sends a Windows-only Alt activation nudge before retrying `SetForegroundWindow` under foreground-lock rules.
- Supporting checks already passed for these changes and `cargo build --release -p synapse-mcp` completed after the foreground-lock patch.
- Last isolated daemon PID `37348` on `127.0.0.1:7808` was stopped before the rebuild.
- Resume by starting a fresh isolated HTTP daemon on a new port, then prove auth/health/strict Inspector `tools/list` and storage baseline. Rerun:
  - cmd: `target=cmd.exe`, args `["/k","title synapse-607-cmd-title2 && echo synapse-607-cmd-title2"]`, wait `(?i).*synapse-607-cmd-title2.*`.
  - powershell: `target=powershell.exe`, args `["-NoExit","-Command","$host.UI.RawUI.WindowTitle='synapse-607-powershell-title'; Write-Output 'synapse-607-powershell-title'"]`, wait `(?i).*synapse-607-powershell-title.*`.
  - terminal: `target=wt.exe` with a unique title, wait for that title.
- Required verdict for each: Inspector trigger exits 0 without timeout, `CF_PROCESS_HISTORY` increments with hwnd/title/pid, action audit ok row foreground profile resolves to cmd/powershell/terminal as appropriate, and a separate foreground/window/process SoT read agrees.

Update 2026-05-31T16:29:23-05:00:
- The unstable `CommandExt::show_window` patch was replaced with a Windows-only `CreateProcessW` console spawn path. `cargo fmt --check`, `cargo check -p synapse-mcp`, `cargo check -p synapse-a11y`, and focused launch/process-history tests are green.
- Resume by running `cargo build --release -p synapse-mcp`, stopping stale isolated daemon PID `37952` if still present, and launching a new isolated repo-built daemon on a fresh port (suggest `7810` or later). Then redo the MCP precondition and console FSV.
- The first parallel `cargo test` attempt for `launch_process_history_row_records_spawn_without_env_values` hit `LNK1104` during concurrent linking; rerunning that test sequentially passed.

Update 2026-05-31T17:27:03-05:00:
- Latest code also fixes the existing-window fallback discovered during Chrome/Explorer launch FSV. Existing excluded windows only satisfy `wait_for_window_title_regex` when the window process is compatible with the requested launch target or a known console-host alias.
- Supporting checks are green: `cargo fmt --check`, `cargo check -p synapse-mcp`, `cargo test -p synapse-mcp launch_window_selection -- --nocapture`, and `cargo build --release -p synapse-mcp`.
- Previous final daemon PID `51896` was stopped before the rebuild. Resume by launching a new isolated daemon on a fresh port (suggest `7812`) and rerunning the MCP precondition. Do not reuse the prior `7811` evidence for closure except as defect-discovery history.

Update 2026-05-31T17:56:27-05:00:
- Wake-up context and live queue were re-read again after compaction. Wired `mcp__synapse` health/profile_list/storage_inspect/observe all work; live profile fleet SoT remains 29 profiles.
- Final7 daemon PID `39520` on `127.0.0.1:7813` is still alive but must be stopped before rebuilding because it locks `target\release\synapse-mcp.exe`.
- Slack failure root cause: `act_launch` did not spawn Slack. It failed during supported-use/action preflight while foreground was Acrobat, because that preflight read a depth-1 UIA snapshot and encountered a child `RuntimeId` value of `VT_EMPTY` (`cached RuntimeId had unexpected type EMPTY`). Storage after the failed trigger read `CF_ACTION_LOG=35`, `CF_PROCESS_HISTORY=17`; `Get-Process slack` found no process.
- Patch now in worktree:
  - action launch/scope preflight uses fast foreground readback instead of a UIA tree when only foreground identity is needed;
  - reflex action scope checks use the same fast foreground behavior;
  - UIA snapshot child/raw-supplement node failures mark the tree truncated and log warnings instead of aborting; empty RuntimeId gets a process-local fallback element id.
- Checks after that patch passed: `cargo fmt --check`, `cargo check -p synapse-a11y`, `cargo check -p synapse-mcp`.
- Resume by stopping PID `39520`, rebuilding release, starting a fresh isolated daemon on a fresh port (suggest `7814`), proving process/socket/auth/health/strict Inspector `tools/list`, then retry Slack and continue the #607 matrix.

Do not use GitHub Actions/CI. Do not create FSV scripts or harnesses. For Synapse behavior FSV, prove the real `synapse-mcp` runtime and client-parity tool list before a real tool call, then read the physical SoT separately.

Update 2026-05-31T18:49:37-05:00:
- Post-compaction wake-up has been completed again. Wired `mcp__synapse` health/profile_list/storage_inspect/observe works through the configured client; live queue still has #607 open plus #594/#595-#604/#608-#634.
- #607 final8 manual FSV evidence is already captured on repo-built isolated daemon PID `61024`, bind `127.0.0.1:7814`, DB `.runs\607\launch-fleet-final8-20260531T182322\db`. Strict Inspector `tools/list` succeeded, storage baseline was `CF_ACTION_LOG=0`, `CF_PROCESS_HISTORY=0`, and `profile_list` showed 29 profiles.
- Accepted #607 profile launches: `acrobat`, `calculator`, `chrome`, `cmd`, `everquest.live`, `excel`, `explorer`, `firefox`, `luanti.minetest`, `mstsc`, `notepad`, `onenote`, `outlook`, `paint`, `photos`, `powerpoint`, `powershell`, `settings`, `slack`, `snippingtool`, `taskmanager`, `teams`, `terminal`, `vscode`, `word`, `zoom`. Console foreground/profile readback passed for cmd, PowerShell, and Windows Terminal.
- Host gaps after reversible local work: `iexplore` redirects foreground to Edge (`profile_id=chrome`), WordPad/write binaries are absent on this modern Windows host, and Minecraft Java remains bounded by Microsoft sign-in/license/runtime/world-log SoT. Luanti analogue passed. WordPad/IE profile metadata was patched with evidence-policy/configured-host status strings using Microsoft removed-features docs.
- Edge cases captured: already-running Chrome/VS Code; wait-title no-match; empty target; structurally invalid regex; max timeout `600000`; rapid Notepad relaunch; restrictive policy deny on daemon PID `59732`, bind `127.0.0.1:7815`.
- Current cleanup/finalization steps:
  1. Done: stopped repo-built daemons where possible and stopped `eqgame.exe` plus FSV-owned heavy apps to release memory. PID `59732` and port `7815` are gone; PID `61024` is absent from process/CIM/tasklist/taskkill, but Windows still reports a stale TCP LISTEN row on `127.0.0.1:7814`, so do not reuse that port.
  2. Done: removed agent-created untracked EverQuest artifacts `Logs/` and `eqclient.ini` after verifying they were under `C:\code\Synapse`.
  3. Done: final checks passed: `cargo fmt --check`, `cargo check -p synapse-a11y`, `cargo check -p synapse-mcp`, `cargo check -p synapse-profiles`, bundled profile parse test, `cargo clippy -p synapse-mcp --all-targets -- -D warnings`, `cargo test -p synapse-mcp launch_ -- --nocapture`, `cargo test -p synapse-mcp process_history_has_retention_class -- --nocapture`, `cargo build --release -p synapse-mcp`, and `git diff --check` (line-ending warnings only).
  4. Next: commit with `[skip ci]`, post #607 RESOLVED evidence, close #607, then refresh queue.
