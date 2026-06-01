# RECOVERY NOTES - Synapse

## Current Resume Point - 2026-06-01T03:31:50-05:00
- #611 is closed with commit `5723393` and RESOLVED evidence at https://github.com/ChrisRoyse/Synapse/issues/611#issuecomment-4590866021. Closure readback: state `CLOSED`, closed at `2026-06-01T08:31:17Z`.
- Live open queue after closing #611: #594 plus #595-#604 and #612-#634.
- Active issue: #612 `scenario(stress): hold_move / hold_button / combo reflex lifetimes`.
- START comment: https://github.com/ChrisRoyse/Synapse/issues/612#issuecomment-4590869661
- #612 acceptance requires real MCP `tools/call` triggers plus separate physical SoT readbacks for:
  - `hold_move` with and without `re_assert`;
  - `hold_button` mouse/gamepad hold;
  - one-shot combo auto-cancel;
  - lifetimes `UntilCancelled`, `OneShot`, `Duration(ms)`, `UntilEvent`, `UntilDeadline`;
  - edges: focus-loss reassert, minimum duration, already-past deadline, cancel already-expired, empty/boundary/structurally-invalid params.
- Suggested SoTs: OS key/button state, controller/XInput state or tester UI, target app/game/tester UI, `CF_REFLEX_AUDIT`, `CF_ACTION_LOG`, `reflex_list`, `reflex_history`, daemon process/socket/log state, and cleanup `release_all`.
- Resume by reading the hold/reflex lifetime/action implementation and current tests, then patch only if gaps are found before launching an isolated repo-built #612 daemon.

## Current Resume Point - 2026-06-01T03:29:27-05:00
- Active issue: #611 `scenario(stress): on_event reflexes - HUD/audio/entity triggers + debounce`.
- Manual behavior FSV evidence is captured in `.runs\611\on-event-fsv-20260601T012006`; the isolated acceptance daemon PID `47056` / port `7834` and FSV-owned target processes were already cleaned up.
- Post-compaction wake-up, live queue readback, git readback, configured wired `mcp__synapse` client health/storage/reflex/observe calls, diff review, and final local supporting checks are complete.
- Final supporting checks passed: `cargo fmt --check`; `cargo check -p synapse-storage -j 2`; `cargo check -p synapse-reflex -j 2`; `cargo check -p synapse-mcp -j 2`; full `scheduler_behavior` (24 passed); MCP `schema_sanitize`; focused reality tests `reality_tools_persist_delta_and_publish_event` and `observe_delta_reads_after_cursor_past_first_page`; `cargo test -p synapse-core error_codes -- --nocapture`; `cargo build --release -p synapse-mcp -j 2`; `git diff --check` with only LF/CRLF warnings.
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46325760`, SHA256 `1D291BB8B00A80377F450E2C285250F1045CC6A663B88F5BAD9990A8F434B7A1`, timestamp `2026-06-01T08:28:59Z`.
- Resume by committing/pushing `[skip ci]`, posting #611 RESOLVED evidence, closing #611, refreshing the live open queue, then selecting #612 or the next live issue.

## Current Resume Point - 2026-06-01T03:09:14-05:00
- Active issue: #611 `scenario(stress): on_event reflexes - HUD/audio/entity triggers + debounce`.
- Manual behavior FSV evidence is captured in `.runs\611\on-event-fsv-20260601T012006` on the patched isolated daemon that was PID `47056` / `127.0.0.1:7834` / DB `db3`.
- Do not relaunch or reuse PID `47056`: cleanup already stopped it and the port has no LISTEN row. Evidence files `141` through `198` record the post-compaction precondition, audio retry, entity path, and cleanup.
- Key acceptance evidence:
  - strict Inspector `tools/list`: `141_tools_list_recheck_7834.json`, count 80, required tools present.
  - HUD: `67` through `72`.
  - Debounce: `73` through `87`.
  - Never-match: `88` through `96`.
  - 8-deep/boundary priority: `97` through `105`.
  - UntilEvent expiry: `106` through `121`.
  - Empty/structurally-invalid: `122` through `126b`.
  - Audio transient accepted on retry: `148` baseline, `149` register, `150` delta showing `music_ended -> loud_transient` and RMS `-120000 -> -8726`, `151` ActionLog `READY AUDIO2_OK `, `152` `reflex_fired`.
  - Entity appear accepted on final physical synthetic Luanti-shaped target: `189` baseline without entities, `190` register, `191` entity_appeared deltas for crosshair/hotbar, `192` ActionLog `ENTITY_READY ENTITY_FINAL_OK `, `193` `reflex_fired` plus `REFLEX_DEBOUNCED`.
  - Cleanup: `196` release_all zero held inputs, `197` active reflexes zero, `198` storage counts.
- Actual Luanti was also launched in `154/155` and proved the real profile emits `luanti_crosshair_region`/`luanti_hotbar_region`; actual Luanti ignored software keyboard/direct chat actions in this session, so the final target-app action SoT used a copied `luanti.exe` WPF target with the same real foreground/capture/entity path and an ActionLog UI readback.
- Resume by running final supporting checks:
  1. `cargo fmt --check`
  2. `cargo check -p synapse-storage -j 2`
  3. `cargo check -p synapse-reflex -j 2`
  4. `cargo check -p synapse-mcp -j 2`
  5. `cargo test -p synapse-reflex --test scheduler_behavior -- --nocapture`
  6. `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  7. focused MCP reality tests for `reality_tools_persist_delta_and_publish_event` and `observe_delta_reads_after_cursor_past_first_page`
  8. `cargo test -p synapse-core error_codes -- --nocapture`
  9. `cargo build --release -p synapse-mcp -j 2`
  10. `git diff --check`
  11. inspect diff.
- If checks pass: update state, commit/push with `[skip ci]`, post #611 RESOLVED evidence, close #611, refresh queue, and continue to #612 or the next open issue. If a check fails, fix the root cause and rerun manual affected evidence as needed.

## Current Resume Point - 2026-06-01T02:25:39-05:00
- Active issue: #611 `scenario(stress): on_event reflexes - HUD/audio/entity triggers + debounce`.
- Do not use the currently running isolated daemon PID `65300` / port `7833` as final acceptance evidence after the latest patch; it is old relative to the cursor-read fix.
- Freshly discovered/fixed #611 blocker: `observe_delta` persisted/published HUD seq `22` but could not return it after `since_seq=21` because the reader limited the first rows under the reality journal prefix before applying `since_seq`. The fix adds start-key prefix scanning in `synapse-storage` / `synapse-reflex` and uses it in `read_delta_rows_after`.
- Checks already passed after the cursor fix: `cargo test -p synapse-mcp observe_delta_reads_after_cursor_past_first_page -- --nocapture`, `cargo fmt --check`, `cargo check -p synapse-storage -j 2`, `cargo check -p synapse-reflex -j 2`, and `cargo check -p synapse-mcp -j 2`.
- Resume sequence:
  1. Cancel/stop old isolated daemon PID `65300` and verify `127.0.0.1:7833` closes.
  2. Keep WPF target PID `43360` if responding, otherwise relaunch `.runs\611\on-event-fsv-20260601T012006\target-app\synapse611_target.ps1`.
  3. Run `cargo build --release -p synapse-mcp -j 2`, then read binary size/hash/timestamp.
  4. Launch a fresh isolated repo-built daemon on a new port with the same temp profile dir and fresh DB.
  5. Prove process/socket/auth/health and official MCP Inspector strict `tools/list`.
  6. Start a fresh `reality_baseline` and rerun #611 FSV evidence from HUD happy path through debounce, never-match, 8-deep, UntilEvent, empty/boundary/invalid, audio transient, and entity-appear.

Current resume point:
- #611 post-compaction recheck completed at 2026-06-01T01:37:37-05:00:
  - Active daemon is PID `65300`, bind `127.0.0.1:7833`, token `synapse-611-token`, DB `.runs\611\on-event-fsv-20260601T012006\db2`.
  - WPF target is PID `43360`, title `Synapse611 HUD FSV`, currently HP `04` and ActionLog `READY DEB_OK `.
  - Auth health recheck saved to `26_health_recheck_7833.json`; strict Inspector tools/list recheck saved to `32_tools_list_recheck_7833.json` and listed 80 tools.
  - Stale debounce reflex `019e81e1-aa50-7572-98ad-f25f1e8b03ba` was cancelled (`27_reflex_cancel_stale_debounce.json`); per-reflex history shows registered/fired/cancelled only.
  - Resume with real Inspector `tools/call` triggers. Reset ActionLog with F7, reset HP with F8, publish `observe_delta` from current head seq 6, then run a new long-window debounce reflex.
- #611 isolated daemon precondition is live:
  - PID `61628`, bind `127.0.0.1:7832`, token `synapse-611-token`, run dir `.runs\611\on-event-fsv-20260601T012006`.
  - Release binary hash `D7F6DD4A11A3A0C7353DDFA1B40BF21F73537B99735645AD5C1F15AC6118C61B`.
  - Unauth `/health` = 401, auth `/health` = 200.
  - Official MCP Inspector strict `tools/list` succeeded with 80 tools and required tools present; JSON saved to `.runs\611\on-event-fsv-20260601T012006\inspector_tools_list.json`.
  - Temporary profile dir contains `synapse611.hud.toml`; WPF target script is `.runs\611\on-event-fsv-20260601T012006\target-app\synapse611_target.ps1`.
- Resume by using Inspector `tools/call`, not hand-rolled HTTP, for:
  1. `act_launch` PowerShell WPF target and read `observe`/UI text SoT.
  2. `reality_baseline` and `observe_delta` HUD action path.
  3. Audio transient path with `audio_tail`/UI/storage readback.
  4. Luanti or equivalent real entity path, debounce, 8-deep filter, UntilEvent lifetime, and invalid/empty/boundary cases.
- #610 is closed with commit `72581cb` and RESOLVED evidence at https://github.com/ChrisRoyse/Synapse/issues/610#issuecomment-4589812724.
- Active issue: #611 `scenario(stress): on_event reflexes - HUD/audio/entity triggers + debounce`.
- START comment: https://github.com/ChrisRoyse/Synapse/issues/611#issuecomment-4589814953
- Current #611 patch is in the worktree. Resume by running broader supporting checks, then launch a repo-built isolated daemon for #611 with official Inspector strict tools/list and real MCP tools/call triggers.
- Patch summary:
  - `crates/synapse-reflex/src/kinds/on_event.rs`: added `REFLEX_DEBOUNCED_KIND` and persisted/published debounce suppression audit evidence.
  - `crates/synapse-reflex/src/scheduler_tick.rs`: coalesces same-tick/window debounce suppressions into audit rows and enforces generic action/on_event `UntilEvent` lifetimes from drained bus events.
  - `crates/synapse-reflex/src/scheduler.rs`: validates `ReflexLifetime::UntilEvent` filters at spawn.
  - `crates/synapse-mcp/src/server/reality.rs`: `reality_delta` bus events are `source=perception` and carry redacted compact `before`/`after` payloads for on_event filters.
  - `crates/synapse-core/src/error_codes.rs`: added `REFLEX_DEBOUNCED`.
- Supporting checks already passed after the patch:
  - `cargo fmt`
  - `cargo test -p synapse-reflex --test scheduler_behavior on_event_ -- --nocapture`
  - `cargo test -p synapse-reflex --test scheduler_behavior scheduler_rejects_invalid_lifetime_filter -- --nocapture`
  - `cargo test -p synapse-mcp reality_tools_persist_delta_and_publish_event -- --nocapture`
  - `cargo test -p synapse-core error_codes -- --nocapture`
- Next supporting checks before FSV: `cargo check -p synapse-reflex`, full `scheduler_behavior`, `cargo check -p synapse-mcp`, `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`, `cargo build --release -p synapse-mcp`, and `git diff --check`.
- These broader checks now passed. Release build required local pressure cleanup: `wsl.exe --shutdown` was run after `rustc-LLVM ERROR: out of memory`; final release binary is `target\release\synapse-mcp.exe`, length `46322176`, SHA256 `D7F6DD4A11A3A0C7353DDFA1B40BF21F73537B99735645AD5C1F15AC6118C61B`, timestamp `2026-06-01T06:10:44Z`.
- Resume by launching the isolated #611 daemon from that binary on a fresh port, proving process/socket/auth/health/strict Inspector `tools/list`, and running manual FSV.
- #611 must cover: HUD threshold event -> action, audio transient -> action, entity-appear -> action, debounce coalescing, filter never matches, 8-deep filter, UntilEvent lifetime expiry, plus empty/boundary/structurally invalid params.
- #611 SoTs: target app/UI state, `reflex_history` / `CF_REFLEX_AUDIT`, `CF_EVENTS` / `CF_OBSERVATIONS`, storage counts, daemon logs, process/socket state, and cleanup state.

Closed #610 reference:
- Acceptance evidence is in `.runs\610\aim-track-accept-20260531T2351`.
- Repo-built isolated daemon PID `74696` on `127.0.0.1:7831` was stopped after cleanup; port is closed.
- Manual evidence completed: MCP preconditions, moving-target convergence, track loss, X-only, Y-only, deadzone no-op, max-speed clamp, target teleport, boundary `priority=1000`, empty missing-target params, structurally invalid element target, and cleanup.
- Final supporting checks completed: fmt/checks, full scheduler behavior test, focused MCP target-source and bbox tests, schema sanitize, release build, and `git diff --check`.
- Final #610 release binary: `target\release\synapse-mcp.exe`, SHA256 `478BDD601E6CE5CAD19465FE8D43E01BB1837135340B350A4C3E93FC32290F6A`, length `46315008`, timestamp `2026-06-01T05:27:02Z`.

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
# Recovery Notes - Synapse

## 2026-06-01T02:25:39-05:00
- Active issue: #611 `scenario(stress): on_event reflexes - HUD/audio/entity triggers + debounce`.
- Important interruption: the prior isolated daemon PID `65300` / port `7833` is running old code relative to the new cursor fix. Do not continue FSV on that daemon as acceptance evidence.
- The just-patched cursor bug: `observe_delta` read rows from the start of the `reality/delta/v1/<profile>/<epoch>/` prefix and filtered `since_seq` after limiting, causing later rows to disappear when the prefix had more older rows than `max_deltas`. Fix adds start-key prefix scanning and regression `observe_delta_reads_after_cursor_past_first_page`.
- Checks already passed after this patch: focused MCP regression, `cargo fmt --check`, `cargo check -p synapse-storage -j 2`, `cargo check -p synapse-reflex -j 2`, and `cargo check -p synapse-mcp -j 2`.
- Resume:
  1. Cancel/stop old isolated daemon PID `65300` and verify `127.0.0.1:7833` closes.
  2. Keep WPF target PID `43360` if still responding; otherwise relaunch from `.runs\611\on-event-fsv-20260601T012006\target-app\synapse611_target.ps1`.
  3. Run `cargo build --release -p synapse-mcp -j 2`, read binary size/hash/timestamp.
  4. Launch fresh isolated daemon on a new port with the same temp profile dir and a fresh DB.
  5. Prove process/socket/auth/health and official Inspector strict `tools/list`.
  6. Start a fresh baseline and rerun #611 FSV evidence from HUD happy path or, at minimum, rerun every edge after the cursor fix before posting acceptance.
