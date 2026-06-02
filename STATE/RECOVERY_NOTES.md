# RECOVERY NOTES - Synapse

## Current Resume Point - 2026-06-02T01:05:39-05:00
- #595 is ready for closeout.
- Product patch:
  - file `crates/synapse-a11y/src/platform/windows/snapshot.rs`;
  - normal UIA child traversal now streams through raw `UITreeWalker` sibling calls with node/deadline checks before each child;
  - bulk `find_all_build_cache(TreeScope::Children)` remains only for UWP app-frame/CoreWindow fallback classes;
  - raw pattern supplement is gated to Notepad root windows;
  - tests cover budget/deadline helper and Notepad-only supplement gating.
- Accepted manual MCP/SoT evidence directory: `.runs\595\fanout-fsv-20260602T0037`.
  - isolated repo-built daemon PID `64060`, bind `127.0.0.1:7864`, strict Inspector `tools/list` 80 tools, and `tools/call` triggers for `health`, `observe`, `find`, `reality_baseline`, `observe_delta`, `storage_inspect`, and `release_all`;
  - deterministic target PID `62812`, title `Issue595FanoutTarget`, 10k item/state/UIA SoT readbacks;
  - happy `observe depth=6 max_elements=500`: element count 184, `CF_EVENTS/CF_OBSERVATIONS` 0->1, daemon elapsed ~403ms with `A11Y_SNAPSHOT_WALK_TRUNCATED reason="deadline"`;
  - happy `find Issue595 Item 00042`: exact name/automation id/bbox matched independent UIA;
  - baseline/rename/delta: `CF_KV` baseline/head rows, target renamed to `Issue595 Renamed`, then 8 reality deltas persisted;
  - edges: `max_elements=1`, no-result `find`, depth-0 boundary, max-elements-0 clamp boundary, structurally invalid unknown param with storage unchanged, minimized-window `find window_hwnd`, and Calculator/UWP `CalculatorResults` smoke.
- Cleanup completed:
  - Inspector `release_all` returned zero held inputs;
  - target PID `62812`, CalculatorApp PID `29856`, daemon PID `64060`, and port `7864` are absent;
  - `ApplicationFrameHost` PID `18732` is now Windows Settings and was preserved.
- Final supporting checks passed:
  - `cargo fmt --check`
  - `git diff --check` with line-ending warnings only
  - `cargo test -p synapse-a11y collection_limit_reason -- --nocapture`
  - `cargo test -p synapse-a11y raw_pattern_supplement -- --nocapture`
  - `cargo check -p synapse-a11y -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`
  - `cargo build --release -p synapse-mcp -j 2`
  - final release SHA256 `C5415C7A2153613FC5C9BC654C3ADB99A939F83D7BC2A6FA9F7CF206A41DC57A`, length `46485504`, timestamp `2026-06-02T06:05:23Z`.
- Important caveats:
  - configured chat MCP loaded but is a long-lived stale daemon and still took ~28s on a wired `find` against the 10k target; do not use that stale daemon as #595 acceptance evidence.
  - Inspector empty-query CLI encoding did not produce a clean empty-string server trigger, so no empty-query verdict is claimed; no-result `find` covers the empty/no-match behavior expected by the issue.
  - `README.md` is unrelated/user-owned; do not stage.
- Exact next actions:
  1. Stage only `crates/synapse-a11y/src/platform/windows/snapshot.rs` plus `STATE/CURRENT_STATE.md`, `STATE/DECISION_LOG.md`, `STATE/HEARTBEAT.md`, and `STATE/RECOVERY_NOTES.md`.
  2. Confirm `README.md` is excluded from the index.
  3. Commit `fix(a11y): stream UIA fanout snapshots (#595) [skip ci]`, push, post RESOLVED evidence to #595, close #595, and remove stale `status:in-progress` if present.
  4. Refresh the live queue and take #596 unless GitHub changed.

## Current Resume Point - 2026-06-02T00:36:51-05:00
- Active issue remains #595:
  - title `scenario(stress): UIA fanout storm - observe/find under 10k+ element trees`;
  - START comment https://github.com/ChrisRoyse/Synapse/issues/595#issuecomment-4598866903;
  - assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
- Wake-up after compaction was completed again:
  - doctrine files, `STATE/*`, #351, #595, live queue, git status/log/branch all read;
  - wired Synapse MCP `health`, `storage_inspect`, `observe`, and `find` returned successfully.
- Important reconciliation:
  - disk state at `00:18` described the first budget guard patch, but the later manual run found a second real defect.
  - real MCP `observe`/`find` against the 10k WPF target took ~26-27s because `FindAllBuildCache(TreeScope::Children)` bulk-materialized 10k children before Synapse could enforce its node/deadline budget. Those results were rejected as FSV evidence.
- Current #595 patch in `crates/synapse-a11y/src/platform/windows/snapshot.rs`:
  - keeps the original `collection_limit_reason` budget/deadline guards;
  - adds a `UITreeWalker` to `SnapshotWalk`;
  - streams normal child enumeration one sibling at a time with budget/deadline checks before each child;
  - limits bulk child enumeration to UWP app-frame/CoreWindow classes to preserve the #582 boundary behavior;
  - limits the raw `File`/`Edit`/`View` supplement to Notepad roots so high-fanout targets are not scanned by that workaround.
- Supporting checks passed after this patch:
  - `cargo fmt`
  - `cargo test -p synapse-a11y collection_limit_reason -- --nocapture`
  - `cargo check -p synapse-a11y -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo build --release -p synapse-mcp -j 2`
  - release binary: length `46485504`, SHA256 `9F7663082D2A417E44B053AD95C79B590B50B0409BFCCE421FF1C616196757E7`, `LastWriteTimeUtc=2026-06-02T05:36:42.1557686Z`.
- Runtime state:
  - stale isolated daemon PID `79940` / port `7863` was stopped.
  - WPF target PID `62812`, title `Issue595FanoutTarget`, remains live and should be reused or replaced for the patched run.
  - `README.md` is unrelated/user-owned; do not stage it.
- Exact next actions:
  1. Start fresh isolated repo-built daemon on a new #595 port (for example `127.0.0.1:7864`) with issue-local DB/log paths and token from `%APPDATA%\synapse\token.txt` without printing the token.
  2. Verify process/socket/binary, unauth/auth health, and strict MCP Inspector `tools/list` for `observe`, `find`, `reality_baseline`, `observe_delta`, `storage_inspect`, and `release_all`.
  3. Read target state/window/UIA SoTs before the trigger.
  4. Trigger real MCP `observe depth=6 max_elements=500`; accept only if the separate after-read shows bounded storage/log state and daemon elapsed time no longer reflects bulk 10k enumeration.
  5. Run `find`, `reality_baseline` + mutate + `observe_delta`, and edges: `max_elements=1`, no-result/empty query, invalid params, minimized target, and UWP/CoreWindow smoke.
  6. Finish with final supporting checks, cleanup target/daemon, diff review, commit `[skip ci]`, RESOLVED comment, close #595, then continue the queue.

## Current Resume Point - 2026-06-02T00:18:00-05:00
- Active issue remains #595:
  - title `scenario(stress): UIA fanout storm - observe/find under 10k+ element trees`;
  - START comment https://github.com/ChrisRoyse/Synapse/issues/595#issuecomment-4598866903;
  - assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
- Required wake-up was re-run after compaction and reconciled:
  - #351 confirms manual FSV only and no GitHub Actions/CI.
  - live queue still has #594 parent, active #595, #624/#625 blocked on the Daybreak operator-only boundary, and #596-#604/#629-#634 open.
  - wired Synapse MCP `health`, `storage_inspect`, `observe`, and `find` loaded through the real configured client.
- Current #595 patch:
  - file: `crates/synapse-a11y/src/platform/windows/snapshot.rs`;
  - root cause: `collect_nodes` could collect all siblings from a large flat `find_all_build_cache(TreeScope::Children)` result even after `SNAPSHOT_NODE_BUDGET=4000`, because the prior guard only stopped descent;
  - fix: `collection_limit_reason` checks budget/deadline before collection, before child enumeration, and before recursing into remaining siblings; sibling collection now breaks at the budget/deadline and logs `A11Y_SNAPSHOT_WALK_TRUNCATED`.
- Supporting checks already passed after patch:
  - `cargo fmt`
  - `cargo test -p synapse-a11y collection_limit_reason -- --nocapture`
  - `cargo check -p synapse-a11y -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo build --release -p synapse-mcp -j 2`
  - latest release binary readback from that build: length `46479360`, SHA256 `291051081606485F341561FABB67AA44A80E4A179DC2D911B42EB4C90B421B0D`, `LastWriteTimeUtc=2026-06-02T05:10:43.48732Z`.
- Issue-local manual target:
  - `.runs\595\fanout-fsv-20260602T0018\target\issue595_target.ps1`;
  - launches visible WinForms `Issue595FanoutTarget`;
  - deterministic `ListBox` counts/prefixes/state file: 0/500/4000/10000 items, sentinel `Issue595 Item 03500`, renamed sentinel `Issue595 Renamed 03500`, `Select3500`, `Minimize`, `Exit`.
- Current worktree:
  - `README.md` dirty and unrelated/user-owned; do not stage.
  - `crates/synapse-a11y/src/platform/windows/snapshot.rs` dirty for #595.
  - state files dirty for #595 resume context.
- Exact next actions:
  1. Launch target with `powershell.exe -STA -File .runs\595\fanout-fsv-20260602T0018\target\issue595_target.ps1 -StatePath .runs\595\fanout-fsv-20260602T0018\target-state.json`; hide only the console helper, leave target window visible.
  2. Read target process/window/state-file SoTs and do a separate UIA/visible read to confirm 10k count/sentinel.
  3. Launch isolated repo-built `synapse-mcp.exe --mode http` on a fresh port with DB `.runs\595\fanout-fsv-20260602T0018\db` and logs `.runs\595\fanout-fsv-20260602T0018\logs`.
  4. Verify daemon process/socket/binary hash, unauth `/health=401`, auth `/health ok=true`, strict Inspector `tools/list` with `observe`, `find`, `observe_delta`, `reality_baseline`, `storage_inspect`, and `release_all`.
  5. Run manual MCP FSV:
     - happy observe depth 6/max 500 against the 10k tree; after-read target state, isolated storage row counts, daemon log `A11Y_SNAPSHOT_WALK_TRUNCATED`, and bounded node counts.
     - `find` sentinel query such as `Issue595 Item 03500` or visible selected sentinel; separate target/UIA readback.
     - baseline then rename target and call `observe_delta`; read CF_KV/head/delta rows plus target state file.
     - edges: empty/no-result find, `max_elements=1`, minimized target, invalid params, UWP/CoreWindow smoke where available.

## Current Resume Point - 2026-06-02T00:03:13-05:00
- #628 is closed:
  - commit `4991efe fix(mcp): harden browser element actions (#628) [skip ci]`;
  - RESOLVED evidence https://github.com/ChrisRoyse/Synapse/issues/628#issuecomment-4598863144;
  - closure readback `state=CLOSED`, `closedAt=2026-06-02T05:02:28Z`;
  - `status:in-progress` label removed.
- Active issue is #595:
  - title `scenario(stress): UIA fanout storm - observe/find under 10k+ element trees`;
  - START comment https://github.com/ChrisRoyse/Synapse/issues/595#issuecomment-4598866903;
  - assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
- Current worktree after #628:
  - `main` is at `origin/main` commit `4991efe`.
  - only `README.md` is dirty and unrelated/user-owned.
- Exact next actions:
  1. Commit/push this state-only transition with `[skip ci]` while excluding `README.md`.
  2. Inspect #595 code paths: `observe`, `find`, `observe_delta`, UIA snapshot caps/depth, storage observations/events/reality rows, and previous #615 coalescing/fanout fixes.
  3. Create or launch deterministic high-fanout local targets, likely issue-local generated UI fixtures plus Explorer/Chrome/Excel surfaces where practical.
  4. Build/launch an isolated repo-built `synapse-mcp`, verify process/socket/auth/health/strict Inspector `tools/list`.
  5. Run manual #595 FSV: happy path plus `max_elements=1`, `depth=6`, large flat list, no-result/empty query, minimized-window, and structurally invalid params, with before/after physical SoT reads.

## Current Resume Point - 2026-06-02T00:00:05-05:00
- Active issue #628 has complete implementation, manual MCP/SoT evidence, final checks, diff review, and cleanup.
- Final #628 evidence facts to use in the RESOLVED comment:
  - isolated repo-built daemon PID `34424`, bind `127.0.0.1:7862`, DB `.runs\628\browser-marathon-fsv-20260601T1915\db12_scroll_hit_test_clean`;
  - strict Inspector `health`/`tools/list` artifacts `431`/`432` accepted required tools;
  - final release binary SHA256 `710ADCF581389D984ED613A7DE3034A623055825A8D743B7368CF1F3F6268530`, length `46477312`;
  - happy path final SoTs: Playwright artifact `437_happy_after_submit_playwright_corrected_post_compaction7.txt`, server artifact `435_happy_after_submit_server_post_compaction7.json`, storage artifact `436_happy_after_submit_storage_post_compaction7.txt`;
  - final payload receipt `M-1`: `Casey Happy`, `casey.happy@example.test`, `priority=normal`, `Notes happy path via Synapse MCP`, `searchQuery=vega`, `modalCode=MOD-628-HAPPY`, `iframeCode=IFR-628-HAPPY`, `dynamicReady=true`, `movedClicks=1`;
  - edge artifacts: empty search `440`-`447`, 256-char boundary search `448`-`462`, invalid element id `463`-`470`;
  - cleanup: wired `release_all` zero; isolated daemon stopped; #628-owned server/Playwright/Chrome stopped; ports `8763`/`8932`/`9226` closed; unrelated Chrome PID `30964` preserved.
- Current dirty state:
  - #628 code/state files are dirty and should be staged.
  - `README.md` is dirty but unrelated/user-owned and must not be staged for #628.
- Exact next actions:
  1. `git status --short --branch` and stage only #628-owned code/state files.
  2. Verify `git diff --cached --stat` excludes `README.md`.
  3. Commit `fix(mcp): harden browser element actions (#628) [skip ci]` and push.
  4. Post #628 RESOLVED evidence, close #628, remove `status:in-progress` if still present.
  5. Refresh the live issue queue and claim the next open unblocked issue.

## Current Resume Point - 2026-06-01T23:06:31-05:00
- Active issue is #628:
  - title `scenario(showcase): browser marathon - Chrome workflow with Playwright MCP as oracle`
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/628#issuecomment-4597523219
  - assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
- Current runtime:
  - server PID `79412` on `127.0.0.1:8763`;
  - Playwright MCP PID `39204` on `::1:8932`;
  - target Chrome PID `63396`, CDP `127.0.0.1:9226`, HWND `0x12068a` / decimal `1181322`;
  - isolated repo-built `synapse-mcp.exe` PID `34424`, bind `127.0.0.1:7862`, DB `.runs\628\browser-marathon-fsv-20260601T1915\db12_scroll_hit_test_clean`.
- Fresh post-compaction runtime artifacts:
  - `322_runtime_processes_post_compaction2.json`
  - `323_runtime_sockets_post_compaction2.json`
  - `324_patched12_health_post_compaction2.txt`
  - `325_patched12_tools_list_post_compaction2.txt`
- Accepted #628 sub-evidence so far:
  - targeted `act_scroll.at` moved Playwright DOM `scrollY` from `0` to `1278` and isolated `CF_ACTION_LOG` from `0` to `2`.
  - `act_type into_element` on the target Chrome search input wrote exact Playwright DOM value `vega` with length `4` and isolated `CF_ACTION_LOG` moved from `2` to `4`.
  - UIA immediate readback for the `act_type` path still warned `after_len=0`, so the verdict for browser typing is the external Playwright DOM readback, not the tool return or UIA readback.
- Exact next actions:
  1. Reset server and navigate Playwright to `http://127.0.0.1:8763/`.
  2. Raise/check target Chrome HWND `0x12068a` before coordinate-dependent triggers.
  3. Run the full #628 happy path through real Synapse MCP triggers with Playwright/server/storage SoT before/after: search, late-loaded control, modal, iframe, form fill, moved/scroll target, and submit.
  4. Run >=3 edges: empty, boundary, and structurally invalid, with before/after SoT reads.
  5. If any successful tool response lacks the expected SoT delta, fix the root cause before continuing.

## Current Resume Point - 2026-06-01T23:00:34-05:00
- Active issue is #628:
  - title `scenario(showcase): browser marathon - Chrome workflow with Playwright MCP as oracle`
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/628#issuecomment-4597523219
  - assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
- Current runtime:
  - server PID `79412` on `127.0.0.1:8763`;
  - Playwright MCP PID `39204` on `::1:8932`;
  - target Chrome PID `63396`, CDP `127.0.0.1:9226`, HWND `0x12068a`;
  - isolated repo-built `synapse-mcp.exe` PID `34424`, bind `127.0.0.1:7862`, DB `.runs\628\browser-marathon-fsv-20260601T1915\db12_scroll_hit_test_clean`.
- Release binary now in use: `target\release\synapse-mcp.exe`, length `46477312`, SHA256 `971EAE444FE3E72FA533C7B7FBAA41A97824A5D149C7E263F6D9FB2BBD0FC301`.
- Fresh strict Inspector precondition artifacts after compaction:
  - `298_patched12_health_post_compaction.txt`
  - `299_patched12_tools_list_post_compaction.txt`
  - required #628 tools present: `act_scroll`, `act_type`, `act_click`, `find`, `observe`, `storage_inspect`, `release_all`, `health`.
- Targeted scroll fix has passed a manual SoT loop:
  - before Playwright DOM `scrollY=0`, storage `CF_ACTION_LOG=0`;
  - target HWND setup readback showed `WindowFromPoint(856,696)` hit target Chrome root `0x12068a`;
  - real Synapse MCP `tools/call act_scroll dy=-20 at={"x":856,"y":696}` returned `backend_used=software_window_message`;
  - after Playwright DOM `scrollY=1278` and moved target rect shifted from `y=2623.5417` to `y=1345.5417`;
  - storage `CF_ACTION_LOG=2`;
  - daemon stderr contains `M2_ACT_SCROLL_HWND_MESSAGE target_class=Chrome_RenderWidgetHostHWND screen_x=856 screen_y=696 delta=-2400`.
- Exact next actions:
  1. Reset server and navigate Playwright to `http://127.0.0.1:8763/`.
  2. Raise/check target Chrome HWND `0x12068a` before coordinate-dependent triggers.
  3. Test `act_type into_element` on the search field with known text `vega`; read Playwright DOM before/after and isolated `storage_inspect`.
  4. If the value is exact, continue the full happy path and edge FSV. If it appends or contaminates text, fix `act_type` and rebuild/relaunch before proceeding.

## Current Resume Point - 2026-06-01T21:16:00-05:00
- Active issue is #628:
  - title `scenario(showcase): browser marathon - Chrome workflow with Playwright MCP as oracle`
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/628#issuecomment-4597523219
  - assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
- User `Issue615FanoutTarget` concern:
  - treat any `Issue615FanoutTarget` window as stale #615 fixture residue unless a new live window/readback proves otherwise;
  - fixture buttons only mutate the old WinForms item panel or close the fixture, not product behavior.
- #628 worktree patch compiles and release-builds:
  - changed files: `crates/synapse-action/src/backend/software/mouse.rs`, `crates/synapse-mcp/src/m2/click.rs`, `crates/synapse-mcp/src/m2/click/element.rs`.
  - latest release `synapse-mcp.exe`: length `46379008`, SHA256 `42FB209D71E8D2F6967D0F82D1B6A6EE70422B98361489ADCCDCD14F3F4258D1`, `LastWriteTimeUtc=2026-06-02T02:14:58.1557599Z`.
  - checks passed: fmt, fmt-check, `cargo check -p synapse-mcp -j 2`, `cargo check -p synapse-action -j 2`, focused element-coordinate click regression, focused DPI compensation regression, focused cursor-readback tolerance regression, and release build.
- Important #628 runtime facts:
  - local target/server run directory: `.runs\628\browser-marathon-fsv-20260601T1915`;
  - Node server PID `79412` is still listening on `127.0.0.1:8763`;
  - Chrome CDP PID `77260` is still listening on `127.0.0.1:9226`;
  - Playwright MCP PID `39204` was previously listening on `::1:8932`; recheck before using it;
  - server `state.json` currently has no submissions and no iframe messages.
- Old #628 isolated daemon PID `56124` / port `7857` was released and stopped before the latest release build.
- Exact next actions:
  1. Start a fresh isolated repo-built `synapse-mcp` daemon on a new port, e.g. `127.0.0.1:7858`, with a fresh #628 DB/log path.
  2. Use official MCP Inspector strict `tools/list`, authenticated `health`, and a process/socket/binary readback for the new daemon.
  3. Reset/reload the browser marathon page, making sure Synapse and Playwright observe the same Chrome/page.
  4. Trigger through real Synapse MCP `tools/call` only, then read Playwright DOM/server/page state and Synapse storage/action log separately.
  5. Cover happy path plus late-loaded control, moved/scroll/DPI element, modal, iframe, empty/boundary/structurally-invalid inputs.

## Current Resume Point - 2026-06-01T19:13:00-05:00
- #627 is resolved and closed:
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/627#issuecomment-4597519110
  - closure readback: `state=CLOSED`, `closedAt=2026-06-02T00:11:22Z`
  - commit `c3b83b2 fix(a11y): handle Office RuntimeId fallback (#627) [skip ci]` pushed to `origin/main`.
- Active issue is #628:
  - title `scenario(showcase): browser marathon - Chrome workflow with Playwright MCP as oracle`
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/628#issuecomment-4597523219
  - assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`
- #628 exact next actions:
  1. Inspect Synapse Chrome profiles/action/perception surfaces and any Playwright MCP/client availability in this repo/host.
  2. Pick a deterministic local browser target that can cover search, form fill, submit, dynamic late-loading controls, moved element after scroll, modal dialog, and iframe content without external flakiness.
  3. Launch a repo-built `synapse-mcp` daemon for #628, verify process/socket/auth/strict `tools/list`/`health`, then run manual FSV through real MCP triggers with Playwright DOM SoT readbacks.

## Current Resume Point - 2026-06-01T18:59:20-05:00
- #627 Excel workbook FSV evidence has reached the saved-file SoT readback.
- Evidence directory: `.runs\627\excel-runtime-check-20260601T1810`.
- Isolated repo-built daemon:
  - PID `34556`, bind `127.0.0.1:7855`.
  - Post-compaction process/socket/binary readback: `119_resume_process_socket_readback.json`.
  - Strict Inspector `tools/list`: `120_tools_list_post_compaction.txt`.
  - Health: `121_health_post_compaction.txt`.
- Saved workbook:
  - path `.runs\627\excel-runtime-check-20260601T1810\issue627-self-driving-spreadsheet.xlsx`
  - file after save: `132_file_sot_after_classic_save.json`
  - `.xlsx` package SoT: `133_xlsx_sot_readback.json`
  - chart relationship SoT: `134_chart_sot_readback.json`
  - SHA256 `D3F696164FE3835A1E7C12C9E7F58821CBC08D52FDB64D7C9553340108AD567E`, length `22526`, sheet dimension `A1:M257`.
- Accepted #627 facts:
  - Real MCP Save As sequence used `find`, `act_click`, `act_press`, `act_clipboard`, and `observe`.
  - File SoT before save was absent; after save exists.
  - Workbook values/formulas match expected table, `G2` records `#DIV/0!`, 256 large-paste rows are present, undo/redo restored/reapplied them earlier, invalid empty `act_press` failed closed with workbook unchanged, and chart XML/relationships are present.
- Cleanup and final supporting checks are complete:
  - `135_release_all_before_cleanup.txt`: zero held keys/buttons/pads.
  - `136_act_press_alt_f4_close_excel.txt`: real MCP Alt+F4 close action succeeded.
  - `138_cleanup_process_port_readback.json`: Excel PID `78020` absent, daemon PID `34556` absent, port `7855` closed.
  - Passed: `cargo fmt --check`, `cargo check -p synapse-a11y -j 2`, `cargo check -p synapse-mcp -j 2`, schema sanitize test, M4 tools-list test, release build, and `git diff --check` with line-ending warnings only.
  - final release binary `target\release\synapse-mcp.exe`: length `46396416`, SHA256 `3FF17F523F900368D486863AA5EED573F8D3616DF2FE87E998330026D5557462`, LastWriteTimeUtc `2026-06-02T00:09:15.8502522Z`.
- Exact next actions:
  1. Post #627 RESOLVED evidence and close #627.
  2. Commit/push code + state with `[skip ci]`.
  3. Refresh the open issue queue and continue.

## Current Resume Point - 2026-06-01T18:35:41-05:00
- Required wake-up context has been re-read after compaction.
- User-visible `Issue615FanoutTarget` concern was checked:
  - no visible/live `Issue615` or fanout top-level window is present by OS window enumeration;
  - wired `find` is currently blocked by the Excel `RuntimeId EMPTY` defect being fixed under #627;
  - fixture source shows the #615 buttons only mutate the old WinForms fixture item panel or close the form.
- Continue active #627:
  - keep the current isolated repo-built daemon/Excel run if still alive;
  - finish remaining spreadsheet edges: large paste, undo/redo, structurally invalid input, save-dialog handling;
  - save the workbook, independently parse `.xlsx` bytes/worksheet/formula/chart SoT, run final supporting checks, post #627 evidence, close #627, commit with `[skip ci]`, and continue the open issue queue.

## Current Resume Point - 2026-06-01T17:50:00-05:00
- #626 is closed with RESOLVED evidence:
  - evidence comment: https://github.com/ChrisRoyse/Synapse/issues/626#issuecomment-4597095341
  - closed at `2026-06-01T22:44:50Z`
  - pushed commit `9382bd2 docs(state): record issue 626 evidence [skip ci]`
- Active issue is #627:
  - title `scenario(showcase): self-driving spreadsheet - launch Excel, build, verify file`
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/627#issuecomment-4597099075
  - assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`
- Excel prerequisite readback:
  - `C:\Program Files\Microsoft Office\root\Office16\EXCEL.EXE`
  - length `75917120`, `LastWriteTimeUtc=2026-05-17T00:11:53Z`
  - HKLM App Paths entries point to the same executable.
- Wired Synapse MCP is healthy through the configured client; process readback found `synapse-mcp.exe` PID `66040` plus stdio child PID `70072`.
- Exact next actions:
  1. Inspect `act_launch`, `act_type`, `act_press`, `act_click`, `find`, `observe`, and `read_text` behavior relevant to Excel/Office foregrounding.
  2. Decide the issue-local workbook path and synthetic dataset/formulas/chart with known expected results.
  3. Launch Excel through real MCP, create/save the workbook, and read independent file SoTs from the `.xlsx` package bytes.
  4. Exercise edges: formula error, large paste/boundary, undo/redo, save-dialog handling, and empty/boundary/structurally-invalid tool inputs.

## Current Resume Point - 2026-06-01T17:45:00-05:00
- #626 manual evidence is complete and cleanup/supporting checks passed; no product-code patch was required.
- Evidence directory: `.runs\626\pianist-fsv-20260601T1709`.
- Key accepted behavior:
  - Isolated audio-enabled repo-built daemon PID `79620`, bind `127.0.0.1:7854`, strict Inspector tools-list 80 tools, required #626 tools present.
  - Local Chrome piano target showed `Audio: armed` and clean counters after Arm/Clear.
  - Happy Ode run: `14_act_combo_happy_ode.json` scheduled 15 steps; OCR after showed `Audio notes: 15`, `Play count: 15`, `Wrong keys: 0`, and exact Ode melody.
  - Overlapped audio run: `19_audio_tail_mid_long_ode48.json` showed nonzero loopback PCM (`peak=5809`, `rms_db=-33.3`, 49 active buckets); OCR after showed `Audio notes: 48`, `Play count: 48`.
  - Empty and non-monotonic combos failed closed and left page/storage unchanged.
  - Muted run showed visible `Muted notes: 4` while `audio_tail` stayed all zero.
  - Wrong-key recovery showed `wrong key x` followed by `C4 recovered after x`.
  - Back-to-back combos produced `C4 D4 E4 G4 F4 E4`.
  - 256-step boundary used the wired MCP client because Inspector CLI hit Windows command-line length; `mcp__synapse.act_combo` accepted `scheduled_steps=256`, OCR showed `Play count: 256`, `Muted notes: 256`, `Audio notes: 0`, and wired storage/reflex rows recorded combo active->expired.
- Cleanup:
  - Isolated and wired `release_all` returned zero held inputs.
  - Stopped #626-owned Chrome, Python server, and isolated daemon; ports `7854`/`8762` closed; no `Issue626PianoTarget` or `Issue615FanoutTarget` visible.
- Supporting checks passed:
  - `cargo fmt --check`
  - `cargo test -p synapse-mcp --test m3_audio_tail_tool -- --nocapture`
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo build --release -p synapse-mcp -j 2`
  - `git diff --check`
- Release binary: `target\release\synapse-mcp.exe`, length `46392320`, SHA256 `FC4003D69AA84712112DEBC3534F113B15F89E69046E23D4064D01CFFAECBE4F`.
- Exact next actions:
  1. Post #626 RESOLVED evidence and close #626.
  2. Refresh the open queue.
  3. Take the next open unblocked issue.

## Current Resume Point - 2026-06-01T17:00:00-05:00
- #625 is blocked and state was committed/pushed:
  - BLOCKED evidence: https://github.com/ChrisRoyse/Synapse/issues/625#issuecomment-4596839011
  - commit `0c854e8 docs(state): record issue 625 block [skip ci]`
  - `git status --short --branch` read clean after push.
- Active issue is #626:
  - title `scenario(showcase): autonomous pianist - act_combo song verified by audio_tail`
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/626#issuecomment-4596846733
  - assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
- #626 FSV target:
  - Launch/navigate Chrome to an online or local piano surface.
  - Map notes to keys and trigger a recognizable melody using real `act_press`/`act_combo`.
  - Read separate SoTs: loopback `audio_tail`, visible key highlight through `observe`/UIA/pixels where available, storage/action logs, and browser/process state.
  - Edges: tempo at combo step limit, muted/silent audio, wrong-key recovery, back-to-back combos, empty sequence, boundary/step-limit, structurally invalid params.
- Current known setup:
  - wired stdio MCP runtime is healthy, but `health.subsystems.audio.status=disabled`; #626 likely needs a repo-built isolated daemon launched with `--enable-audio`.
- Exact next actions:
  1. Inspect audio_tail/audio_transcribe, act_combo/act_press, act_launch/Chrome profile, and observe/pixel/UIA implementations/tests.
  2. Identify a deterministic piano target, preferably local HTML if it avoids network flakiness while still using Chrome and real audio output.
  3. Build/launch isolated repo-built `synapse-mcp` with audio enabled, strict Inspector tools-list, and separate process/socket/auth SoT readbacks.
  4. Run #626 manual MCP FSV and patch any defects exposed.

## Current Resume Point - 2026-06-01T16:56:00-05:00
- #625 reversible evidence is complete and GitHub disposition is posted.
- User's `Issue615FanoutTarget` question:
  - No live #615 fixture process/window or wired MCP element is present now.
  - The fixture buttons in `.runs\615\target\issue615_target.ps1` only mutate an in-window `ItemPanel` or close that temporary WinForms target; they are not product UI.
- #625 evidence summary:
  - Wired Synapse MCP health/profile/storage/observe/tool calls were live after compaction; process readback found `synapse-mcp.exe` PID `66040` plus stdio child PID `70072`.
  - EQ log stayed unchanged at length `2464677`, SHA256 `E563074084A7F5A291AC6FBF77746B993AB086F747C6C111C39503B6BF475368`.
  - Readiness persisted blockers for non-EQ foreground/gameplay UI/chat/HUD/food-drink.
  - Autocombat failed closed before gameplay because active profile was `vscode`; `CF_ACTION_LOG` advanced and recorded `ACTION_TARGET_INVALID active_profile_mismatch` for `issue625-autocombat-deny-vscode`.
  - DynamicJEPA, trajectory export, predictive model fit/predict, surprise confirmed/mismatch/missing-prediction, action-prior samples, and scorecard rows were all triggered through real MCP tools and read back from `CF_KV`/file bytes.
  - Scorecard row `everquest/action_prior_scorecard/v1/everquest.live/issue625-scorecard-window` persisted with `sample_count=3`, `evaluated_count=2`, `abstention_count=1`, `low_confidence_action_count=1`, top1/top3/useful accuracy `1.0`, and competence status `low_confidence_action_forced`.
  - Invalid duplicate-scorecard edge failed closed with `TOOL_PARAMS_INVALID` and no `issue625-scorecard-duplicate-invalid` row; `CF_KV` stayed `48`.
  - Supporting checks passed: fmt, focused scorecard/predictive/surprise tests, schema sanitize, M4 tools-list, MCP check, release build, and diff check.
  - Release binary SHA256: `4AF3EB0E332F6A7AFD5DBBFAD1169EB051371040D5C24CF033662AC3615F78AD`.
- #625 disposition:
  - BLOCKED evidence comment: https://github.com/ChrisRoyse/Synapse/issues/625#issuecomment-4596839011
  - Label readback shows `status:blocked`; `status:in-progress` was removed.
  - Next: commit state updates with `[skip ci]`, push, refresh queue, and continue to the next open issue because #625 is blocked only on an operator-only external/legal/account action.
- Exact remaining operator-only action for #625:
  - Operator must personally review/respond to the Daybreak EULA/account agreement, log in/select character if appropriate, and put `Thenumberone` in visible in-world state with safe target availability.
  - Agent must not click legal/account/login/character-select/chat controls.

## Current Resume Point - 2026-06-01T16:31:30-05:00
- #624 has been committed and pushed:
  - commit `9de5ee3 fix(mcp): guard EverQuest account gates (#624) [skip ci]`
  - BLOCKED evidence: https://github.com/ChrisRoyse/Synapse/issues/624#issuecomment-4596661903
  - #624 labels read back include `status:blocked`; worktree was clean after push.
- Active issue is #625:
  - title `scenario(stress): EverQuest autocombat soak + survival/predictive/surprise/scorecard`
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/625#issuecomment-4596668371
  - assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
- Current host state:
  - wired Synapse MCP `health` ok;
  - foreground is VS Code, not EverQuest;
  - #624 proved EverQuest is blocked by the Daybreak EULA/account agreement when focused, and the agent must not click legal/account/login controls.
- Exact next actions for #625:
  1. Inspect autocombat/readiness/predictive/surprise/action-prior implementations and tests.
  2. Read SoTs before triggers: active storage counts, EQ log length/hash/tail, foreground/EQ state, relevant existing trajectory/domain rows.
  3. Trigger real safe MCP calls for readiness and synthetic predictive/surprise/action-prior storage paths where possible.
  4. If `everquest_autocombat` is blocked by account/EULA/in-world requirements, document exact operator-only action and mark #625 blocked after all reversible safe work is complete.

## Current Resume Point - 2026-06-01T16:24:00-05:00
- Wake-up after compaction was completed and reconciled against live GitHub/git/MCP state.
- User's `Issue615FanoutTarget` question was answered and rechecked:
  - no live #615 fixture window or Synapse element is visible now;
  - those windows are old #615 WinForms UIA fanout stress fixtures, not product UI.
- Active issue is #624:
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/624#issuecomment-4596141027
  - open, assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
  - live open queue: #594 plus #595-#604 and #624-#634.
- Current git/worktree:
  - branch `main`, HEAD `841679c docs(state): record issue 624 start [skip ci]`.
  - modified files: `STATE/*`, `crates/synapse-mcp/src/server/everquest_ui_context.rs`, `crates/synapse-profiles/profiles/everquest.live.toml`, `docs/computergames/05_mcp_tool_surface.md`, `docs/computergames/26_everquest_live_eval.md`.
- #624 implementation patch:
  - EverQuest account/login gate now detects EULA, end-user license agreement, terms/privacy, `I AGREE`, and `I DECLINE`.
  - Action denial reason is `everquest_login_or_account_gate_visible`.
  - Docs describe account/legal gates as non-in-world and raw legal/account text as non-persisted.
- #624 evidence already captured:
  - Isolated repo-built daemon PID `34624`, bind `127.0.0.1:7853`, binary SHA256 `3BA384BF72EC44DC1106235A4809CEDCEBFB056353527FEA57B6D109C14E3AB7`, strict Inspector tools-list 80 tools.
  - EULA/account-gate behavior: real MCP `observe`, `everquest_survival_readiness`, `everquest_chat_input_state`, `act_keymap inventory`, `everquest_loc_probe`, and `everquest_current_state`; denied action/log readbacks prove no gameplay/chat input was sent and EQ log bytes stayed SHA256 `E563074084A7F5A291AC6FBF77746B993AB086F747C6C111C39503B6BF475368`.
  - Synthetic domain/trajectory/episode export already exists; episode export SHA256 `7386a7f8b26cd6fc8e262813eff9167785d13610aaf8e68bbd9fcce3949dc2ef`.
  - ContextGraph ingest/search succeeded through real wired Synapse MCP:
    - ingest row `everquest/contextgraph_ingest/v1/everquest.live/7386a7f8b26cd6fc8e262813eff9167785d13610aaf8e68bbd9fcce3949dc2ef/issue624-synth-trajectory.issue624-synth-consider`
    - fingerprint `d5d91675-9303-4b0f-bdd6-2f0326abffdb`
    - search row `everquest/contextgraph_search/v1/everquest.live/issue624-synth-search-wired-warm`
    - search result/citation count `1`, same fingerprint and export hash.
  - Active safe/read-only storage chain persisted current state, map sensor, four outcome rows, hazard memory, safe-area memory, planner consult, planner guard, route plan, world-model transition, and world summary. Final `storage_inspect` read `CF_KV=33`.
  - Physical SoTs: EQ log length `2464677`, SHA256 `E563074084A7F5A291AC6FBF77746B993AB086F747C6C111C39503B6BF475368`; `maps\nektulos.txt` has `To_Neriak` at line `5974`.
- Captured #624 edges:
  - login/account EULA gate denial;
  - non-EverQuest foreground;
  - synthetic visible unsent chat text guard fail;
  - structurally-invalid planner source ref fail-closed;
  - absent valid-shaped EQ log path fail-closed with no `CF_KV` change;
  - reality audit profile mismatch fail-closed.
- Cleanup and final checks:
  - Real Inspector `release_all` on isolated daemon `127.0.0.1:7853` returned zero held inputs.
  - PID `34624` was stopped; process and port `7853` readbacks returned no rows.
  - Passed: `cargo fmt --check`; focused UI-context tests; EverQuest profile parse test; schema sanitize; tools-list; `cargo check -p synapse-mcp -j 2`; `scripts\check_docs.ps1`; release build; `git diff --check`.
  - Release binary SHA256 `31D62B2891F4AA17F7139BF4A5E52276521F7009E7B2C428D6FAFF15CBF5A374`, length `46392320`.
- Exact next steps:
  1. Commit with `[skip ci]`.
  2. Post #624 evidence and mark #624 blocked on the exact operator-only action: operator must personally review/respond to the Daybreak EULA/account agreement and put the character in-world.
  3. Refresh the queue and continue with the next open issue.

## Current Resume Point - 2026-06-01T16:02:28-05:00
- Wake-up after compaction is complete and reconciled against actual disk/GitHub/process/MCP state.
- User's `Issue615FanoutTarget` concern was rechecked:
  - no live Issue615/fanout process or Synapse element exists now;
  - foreground is EverQuest;
  - `.runs\615\target\issue615_target.ps1` confirms the buttons are only a temporary #615 WinForms UIA fixture that mutates an in-window `ItemPanel` or closes itself.
- Active issue is #624:
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/624#issuecomment-4596141027
  - open, assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
  - live open queue: #594 plus #595-#604 and #624-#634.
- Git readback:
  - branch `main`
  - HEAD `841679c docs(state): record issue 624 start [skip ci]`
  - modified files are the #624 EULA/account-gate patch in `everquest_ui_context.rs`, `everquest.live.toml`, and the two EverQuest docs.
- #624 isolated daemon is still running:
  - PID `34624`, bind `127.0.0.1:7853`
  - binary `.runs\624\eula-guard-fsv-20260601T2034\bin\synapse-mcp-runtime.exe`
  - binary SHA256 `3BA384BF72EC44DC1106235A4809CEDCEBFB056353527FEA57B6D109C14E3AB7`
  - strict Inspector tools-list readback: 80 tools, missing none, all #624 tools present.
- Verified #624 evidence directory:
  - `.runs\624\eula-guard-fsv-20260601T2034`
  - EULA/account-gate guard FSV shows real MCP `observe`, `everquest_survival_readiness`, `everquest_chat_input_state`, `act_keymap`, `everquest_loc_probe`, and `everquest_current_state` calls; separate storage/log readbacks prove actions were denied and EQ log bytes did not change.
  - Synthetic domain/trajectory/episode export rows exist; exported episode file SHA256 is `7386a7f8b26cd6fc8e262813eff9167785d13610aaf8e68bbd9fcce3949dc2ef`.
- ContextGraph next action:
  1. Rerun `everquest_contextgraph_ingest` through the isolated daemon with `no_warm=false`, storage under the data root, wrapper `.runs\issue529\context-graph-mcp-wsl.cmd`, and a long timeout.
  2. If it succeeds, run separate `storage_inspect` and inspect ContextGraph storage directory/file SoTs.
  3. Then run `everquest_contextgraph_search` through real MCP and inspect returned provenance plus persisted search audit rows.
  4. If it fails, inspect the exact child stderr/root cause and continue reversible local setup.
- Operator-only boundary:
  - EQ is visible at the Daybreak EULA/account agreement. Do not click I Agree, I Decline, login, character select, or chat/account fields. Full in-world #624 happy path remains unavailable until the operator personally reviews/responds to the agreement and places the character in-world.

## Current Resume Point - 2026-06-01T15:16:27-05:00
- Wake-up after compaction is complete. Required doctrine/state/GitHub/git/MCP context was re-read and reconciled.
- User's `Issue615FanoutTarget` concern was checked:
  - no live `Issue615`/fanout window/process exists now;
  - wired Synapse `find` found no `Issue615FanoutTarget`/`Show80` elements;
  - fixture source `.runs\615\target\issue615_target.ps1` confirms it was a temporary WinForms UIA stress target; buttons mutate the `ItemPanel` or close the form and are not product UI.
- #623 is closed:
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/623#issuecomment-4596117663
  - Closure readback: `state=CLOSED`, `closedAt=2026-06-01T20:13:12Z`.
  - Evidence/state commit on `origin/main`: `c4c3b14 docs(state): record issue 623 evidence [skip ci]`.
- Active issue is #624:
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/624#issuecomment-4596141027
  - Claimed with `status:in-progress`, `agent:codex`, assigned to `ChrisRoyse`.
  - Live open queue: #594 plus #595-#604 and #624-#634.
- Git readback: branch `main`, `git status --short --branch` is `## main...origin/main`, HEAD `c4c3b14`.
- Wired MCP readback: `health ok=true`, active profile `vscode`, operator hotkey registered, storage initialized, `observe`/`storage_inspect`/`reflex_list` returned normally.
- Exact next actions:
  1. Commit/push this state update with `[skip ci]`.
  2. Inspect #624 EverQuest live-integration tool implementations and supporting tests/docs.
  3. Read current host EverQuest/log/map/layout/ContextGraph SoTs.
  4. Build/launch repo-built `synapse-mcp` for #624 and perform official Inspector strict `tools/list`.
  5. Run #624 manual MCP FSV with separate physical SoT readbacks for each tool path and required edge cases.

## Current Resume Point - 2026-06-01T15:03:43-05:00
- Active issue #623 has behavior FSV evidence and final supporting checks complete. Commit, RESOLVED comment, closure, and queue continuation remain.
- Worktree changes are docs only:
  - `docs/computergames/05_mcp_tool_surface.md`
  - `docs/computergames/06_data_schemas.md`
  - `docs/systemspec/13_mcp_tool_reference.md`
- Evidence directories:
  - Audit/export: `.runs\623\audit-replay-fsv-20260601T1445`
  - Replay/events: `.runs\623\replay-events-fsv-20260601T1457`
- Audit/export evidence:
  - daemon PID `38756`, bind `127.0.0.1:7851`, repo release binary, strict Inspector tools-list 80.
  - consent row `CF_KV/audit_export/v1/consent/vscode` enabled strict local-only.
  - happy export hashes: manifest `329FD52280770C941008A26E6C44C8352FB89C3108ABEA62090A568142D30CAC`, rows `1099D371C32B72CE2326BA751D06BD973F50A1001140232F787199D561F5950C`, report `716D862AC76FE5FE30C3273202AD905063A4B4E7B99717D705C8F52417CCAF6B`.
  - redaction report: 7 rows, 90 fields, zero raw sensitive marker hits.
  - edges: no consent, redaction override, `max_rows=1`, `max_row_bytes=100`, empty output path.
- Replay evidence:
  - daemon PID `11076`, bind `127.0.0.1:7852`, repo release binary, `SYNAPSE_HTTP_SSE_MANUAL=1`, strict Inspector tools-list 80.
  - `replay_record target=both` happy file `issue623-both-manual-event-3.jsonl`: 23295 bytes, SHA256 `1AE400B7A81EAF3BA99FDA510299EFD8A7CB4A11778F624FC64A24FAF5FE9F31`, 7 lines = 4 observations + 3 events, event seqs `6231457005..6231457007`, marker values present.
  - `duration_ms=0` file `issue623-empty.jsonl`: 0 bytes, empty SHA256 `E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855`.
  - invalid target, invalid format, empty path, and traversal path failed closed and wrote no extra files.
  - both isolated daemons were released/stopped; ports `7851` and `7852` are closed.
- Final supporting checks passed: `cargo fmt --check`; `git diff --check`; `scripts\check_docs.ps1`; `cargo check -p synapse-mcp -j 2`; `cargo test -p synapse-mcp --test m3_replay_record_tool -- --nocapture`; `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`; `cargo test -p synapse-mcp --test m3_tools_list -- --nocapture`; `cargo build --release -p synapse-mcp -j 2`.
- Final release binary SHA256 `498E3164F4B795E0ABD3A9E7E2AE678810D532F84B35E5381456277C13628476`, length `46406144`, `LastWriteTimeUtc=2026-06-01T20:11:10.6731953Z`.
- Exact next actions:
  1. Commit docs/state with `[skip ci]`.
  2. Post #623 RESOLVED evidence, close #623, refresh queue, and continue.

## Current Resume Point - 2026-06-01T14:31:53-05:00
- #622 is closed.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/622#issuecomment-4595815302
  - Closure readback: state `CLOSED`, closed at `2026-06-01T19:31:05Z`.
  - State/evidence commit: `9c855fc docs(state): record issue 622 evidence [skip ci]`.
- Active issue is #623 `scenario(stress): audit consent + bundle redaction + replay_record`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/623#issuecomment-4595820271
  - Claimed with `status:in-progress`, `agent:codex`, assigned to `ChrisRoyse`.
  - Current exact next action: inspect `audit_export_consent_set`, `audit_export_bundle`, `replay_record`, redaction policy/report generation, bundle file output, storage row formats, caps, and supporting tests. Then launch a repo-built isolated daemon for manual MCP FSV.

## Current Resume Point - 2026-06-01T14:28:42-05:00
- Active issue #622 has manual MCP FSV, cleanup, and supporting checks complete; no product-code patch was required.
- FSV evidence directory: `.runs\622\authoring-fsv-20260601T1350`.
  - Repo-built daemon was PID `59440`, bind `127.0.0.1:7850`, isolated DB `.runs\622\authoring-fsv-20260601T1350\db`, strict Inspector tools-list count 80.
  - Covered behavior:
    - zero-evidence authoring generate failed closed with no candidate row;
    - real observe/action/replay/reality evidence produced action/observation/event/KV rows and replay SHA256 `61AB2CC29986048235197AA336CCC34B86F9794445683C72223FE53AE6BABC1F`;
    - generate/list/inspect wrote candidate `issue622.accept` proposing `matches.add_exe=["powershell.exe"]`;
    - accept wrote state `accepted` and note, re-accept was idempotent;
    - export wrote 2883-byte accepted bundle SHA256 `D2790BD9118B9DB5790C4B56D382EA3872146688AD7057FA59EA23427AF9E37B`;
    - generate/reject wrote candidate `issue622.reject` with reason `issue622 reject reason`;
    - rejecting accepted, exporting missing candidate, list `limit=0`, malformed candidate id, and over-max `max_audit_rows=10001` all failed closed with storage unchanged;
    - 10000-row boundary used real `storage_put_probe_rows` to grow `CF_ACTION_LOG` `2 -> 10002`, then `profile_authoring_generate issue622.max max_audit_rows=10000` scanned/relevant 10000 rows and wrote a candidate row;
    - `profile_quality_refresh` wrote `profile_quality/v1/issue622.authoring`; separate report readback showed score `21`, sample size `1`, scanned action rows `10002`, relevant action rows `2`, observation rows `2`, event rows `3`;
    - stale edge (`stale_after_ns=1`) persisted stale evidence (`audit_rows_stale=2`, score `0`), invalid quality params failed closed, and a final non-stale refresh restored score `21`.
  - Cleanup completed: `release_all` zero, daemon stopped, port `7850` closed.
- Supporting checks passed: fmt, diff check, MCP check, profile quality tool test, replay record tool test, schema sanitize, m3 tools-list, release build; `cargo test -p synapse-mcp profile_authoring -- --nocapture` compiled but had no matching tests.
- Final release binary SHA256: `236992450A49D3177C1FCBF1D06F567C30CC54AA5F217C1F0D59BFDBADF23E01`.
- Exact next actions:
  1. Commit state update with `[skip ci]`.
  2. Post #622 RESOLVED evidence and close #622.
  3. Refresh live queue.
  4. Take the next open issue unless GitHub state changes.

## Current Resume Point - 2026-06-01T13:43:30-05:00
- #621 is closed.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/621#issuecomment-4595473988
  - Closure readback: state `CLOSED`, closed at `2026-06-01T18:42:45Z`.
  - State/evidence commit: `f9ab56e docs(state): record issue 621 evidence [skip ci]`.
- Active issue is #622 `scenario(stress): authoring loop - generate/accept/reject/export + quality_refresh`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/622#issuecomment-4595477096
  - Claimed with `status:in-progress`, `agent:codex`, assigned to `ChrisRoyse`.
  - Current exact next action: inspect `profile_authoring`, `profile_quality_refresh`, `replay_record`, evidence scanning/storage row formats, and supporting tests. Then launch a repo-built isolated daemon for manual MCP FSV.

## Current Resume Point - 2026-06-01T13:41:30-05:00
- Active issue #621 has manual MCP FSV and supporting checks complete; no product-code patch was required.
- FSV evidence directory: `.runs\621\registry-fsv-20260601T1324`.
  - Repo-built daemon was PID `58848`, bind `127.0.0.1:7849`, isolated DB `.runs\621\registry-fsv-20260601T1324\db`, official Inspector strict `tools/list` count 80.
  - Covered install with expected digest, digest mismatch, scale 600-row registry import/search/report at `limit=1000`, deterministic export and duplicate import, conflicting import, disable+inspect, second-version install and rollback, rollback with no prior, poison contribution quarantine, >1000 contribution quarantine, malformed import, missing-profile contribution export, and invalid limit.
  - Final storage readback: `CF_PROFILES=617`, `CF_KV=1`, `CF_ACTION_LOG=0`; report scanned 617 registry rows and contribution search found two quarantined contribution rows.
  - Cleanup completed: `release_all` zero, daemon stopped, port `7849` closed.
- Supporting checks passed: fmt, diff check, MCP check, curated registry test, registry report test, package manifest test, schema sanitize, m3 tools-list, and release build.
- Final release binary SHA256: `08FEC90BE80C37B940AF9549335F901A8DACE52863FDA9F7990049F0A4A94890`.
- Exact next actions:
  1. Commit state update with `[skip ci]`.
  2. Post #621 RESOLVED evidence and close #621.
  3. Refresh live queue.
  4. Take #622 next unless GitHub state changes.

## Current Resume Point - 2026-06-01T13:16:11-05:00
- #620 is closed.
  - Commit: `6895746 fix(mcp): apply profile runtime config (#620) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/620#issuecomment-4595282935
  - Closure readback: state `CLOSED`, closed at `2026-06-01T18:15:30Z`.
  - Worktree was clean after push/closure before this state-only update.
- Active issue is #621 `scenario(stress): registry scale - install/search/export/import/rollback, digest, poison quarantine`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/621#issuecomment-4595287040
  - Claimed with `status:in-progress`, `agent:codex`, assigned to `ChrisRoyse`.
  - Current exact next action: inspect profile registry implementation, manifest/schema files, storage row formats, and supporting tests. Then build release and run an isolated repo-built daemon for manual MCP FSV using official Inspector strict `tools/list`.

## Current Resume Point - 2026-06-01T13:04:31-05:00
- Active issue is #620.
- #620 implementation is patched locally; manual MCP behavior evidence, final supporting checks, release build, and diff review are complete. Commit, RESOLVED comment, closure, and queue continuation are next.
- Manual FSV run directory: `.runs\620\profile-fsv-20260601T1238-clean`.
  - Repo-built daemon PID `61244`, bind `127.0.0.1:7848`, isolated DB, isolated appdata/token, strict official Inspector tools-list count 80.
  - All 29 bundled profiles were activated through real Inspector `profile_activate`; each had separate `profile_list` + `health.subsystems.perception` readbacks matching active profile id, mode, capture target/source, min interval, and cursor setting. #620 title says 30, but live SoT has 29 profiles.
  - Matching foreground PowerShell `observe` read `mode=a11y_only` and `diagnostics.capture_config.source=profile:powershell`.
  - `act_keymap alias=clear` wrote `CF_ACTION_LOG` rows preserving alias `clear`, binding `ctrl+l`, keys `ctrl,l`, backend `software`, and foreground `powershell.exe`.
  - HUD specs are present for EverQuest/Luanti/Minecraft profiles. Live Luanti launched and matched the profile process/window, but host foreground remained PowerShell and mouse focus failed with access denied, so HUD-slot live readback is a documented explained gap under the issue acceptance.
  - Edges covered: unknown profile, same-profile reactivation, app-not-running/foreground mismatch, empty alias, unknown alias, and no bundled empty-keymap profile.
  - Cleanup completed: release_all zero, FSV-owned Luanti/Notepad processes stopped, daemon stopped, port `7848` closed.
- Exact next actions:
  1. Commit with `[skip ci]`.
  2. Post #620 RESOLVED evidence and close #620.
  3. Refresh the live queue.
  4. Take #621 next unless GitHub state changes.

## Current Resume Point - 2026-06-01T12:45:00-05:00
- Active issue is #620.
- The user's `Issue615FanoutTarget` concern has been checked: no #615 target windows/processes are currently visible/running. They were only the #615 synthetic UIA target and should not be considered product UI.
- Worktree has an uncommitted #620 patch:
  - M1 `active_capture_config`;
  - `observe.diagnostics.capture_config`;
  - `health.subsystems.perception` mode/capture readback;
  - `profile_activate` and foreground profile resolution apply profile mode/capture as well as action backend resolution;
  - `m3_profile_tools` regression checks activation health and observe mode/capture for a synthetic matching profile.
- Supporting checks already passed: core/perception/MCP checks, `m3_profile_tools`, core snapshots observation shape, perception regression, MCP context tests, schema sanitize, and core types.
- Remaining exact next actions:
  1. Run final local supporting checks as needed and `cargo build --release -p synapse-mcp -j 2`.
  2. Launch an isolated repo-built HTTP daemon for #620 with `SYNAPSE_DB` under `.runs\620\...`.
  3. Prove process/socket/auth/health/strict Inspector `tools/list`.
  4. Manually FSV all 29 bundled profiles through real Inspector `profile_activate` calls with separate `profile_list` and `health.subsystems.perception` readbacks.
  5. Manually FSV representative matching-profile `observe`, HUD, `act_keymap` + `CF_ACTION_LOG`, and required edge cases.
  6. Post RESOLVED evidence to #620 only after FSV and final checks pass.

## Current Resume Point - 2026-06-01T11:54:30-05:00
- #619 is closed.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/619#issuecomment-4594692386
  - Closure readback: state `CLOSED`, closed at `2026-06-01T16:53:51Z`.
  - No product-code patch was required; final supporting checks/release build passed.
  - Final release binary SHA256: `AF801288800BB64E3DA92B95573F2E9787FE7899AA497E264E7023242D03AB60`.
- Active issue is #620 `scenario(stress): activate all 30 profiles — keymap/HUD/capture/mode apply`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/620#issuecomment-4594697356
  - Claimed with `status:in-progress` and `agent:codex`.
  - Current next action: inspect profile runtime/registry code and bundled profile definitions, then launch a repo-built isolated daemon for #620 manual MCP FSV.

## Current Resume Point - 2026-06-01T11:45:17-05:00
- Active issue #619 has manual MCP FSV behavior evidence captured and the isolated daemon has been cleaned up.
- Run directory: `.runs\619\gc-concurrent-fsv-20260601T1135`.
  - Repo-built daemon was PID `69600`, bind `127.0.0.1:7847`, isolated DB `.runs\619\gc-concurrent-fsv-20260601T1135\db`, strict Inspector tools-list count 80.
  - Covered behavior:
    - concurrent four-writer fan-in to `CF_EVENTS` (0 -> 320) followed by GC to soft cap 75 while retaining newest tail rows;
    - in-flight/heavy writer of 10000 rows x 2048 bytes followed by GC to 75 while retaining newest `issue619-900-z:9997..9999`;
    - audit-retention max-age report `audit_retention/v1/report/issue619-age`;
    - audit-retention dedupe/run_id report `audit_retention/v1/report/issue619-dedupe`;
    - soft-cap boundary no-op at 75 rows;
    - empty `CF_MODEL_CACHE` GC no-op;
    - invalid `soft_cap_rows=0` failed closed with `TOOL_PARAMS_INVALID` and unchanged storage;
    - 75 -> 100 -> 75 oscillation below hard cap with newest `issue619-950-y:22..24` retained.
  - Cleanup completed: real `release_all` returned zero held state; PID `69600` stopped; port `7847` closed.
- Final supporting checks passed: fmt, diff check, `cargo check` for storage/reflex/MCP, focused storage GC tests, MCP storage tool test, schema sanitize test, and release build.
- Final release binary readback: length `46320128`, SHA256 `AF801288800BB64E3DA92B95573F2E9787FE7899AA497E264E7023242D03AB60`, timestamp `2026-06-01T16:52:28Z`.
- Current next action: post #619 RESOLVED evidence, close #619, refresh the open queue, and take the next open child.

## Current Resume Point - 2026-06-01T11:29:00-05:00
- #618 is closed.
  - Commit: `c0b24e3 fix(mcp): expose storage pressure gating (#618) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/618#issuecomment-4594501572
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T16:27:18Z`.
  - Worktree clean after push.
- Active issue is #619 `scenario(stress): storage_gc_once under concurrent writes`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/619#issuecomment-4594506099
  - Live queue after #618 closure: #594 plus #595-#604 and #619-#634.
  - Current next action: inspect storage GC/probe-row/audit-retention implementation and tests, then launch a repo-built isolated daemon for #619 manual MCP FSV.

## Current Resume Point - 2026-06-01T11:18:00-05:00
- Active issue #618 `scenario(stress): storage pressure ladder - 5 levels + write-gating` is patched and manual MCP FSV evidence is captured.
- Worktree patch:
  - `crates/synapse-storage/src/lib.rs`: `Db::pressure_permits_write`.
  - `crates/synapse-reflex/src/storage.rs`: `ReflexRuntime::storage_pressure_permits_write`.
  - `crates/synapse-mcp/src/m3/storage.rs`: diagnostic probe writes cover all 11 CFs and fail explicitly with `STORAGE_WRITE_FAILED` under pressure refusal.
  - `crates/synapse-mcp/tests/m3_storage_tool.rs`: supporting pressure-gating regression.
- Manual FSV evidence is under `.runs\618\pressure-fsv-20260601T1108-patched`.
  - Repo-built daemon PID `56980`, bind `127.0.0.1:7846`, isolated DB, token `synapse-618-token`, strict Inspector `tools/list` count 80.
  - Initial SoT read: pressure `Normal`, all CF counts 0.
  - Ladder: exact thresholds stayed at the prior level; below thresholds entered L1/L2/L3/L4 with codes `STORAGE_DISK_PRESSURE_LEVEL_1..4`; L2/L3/L4 transitions compacted all 11 CFs; recovery to `Normal` emitted no new code and compacted 0.
  - Gating: L2 accepted `CF_OBSERVATIONS`; L3 refused `CF_OBSERVATIONS`, `CF_OCR_CACHE`, `CF_TELEMETRY`, `CF_MODEL_CACHE`, `CF_PROCESS_HISTORY` and allowed `CF_EVENTS`; L4 refused `CF_EVENTS`, `CF_ACTION_LOG`, `CF_KV`, `CF_OBSERVATIONS` and allowed only `CF_REFLEX_AUDIT`/`CF_SESSIONS`; empty `rows=0` to blocked `CF_EVENTS` no-opped; invalid `cf_name=NOT_A_CF` failed closed; recovery allowed `CF_OBSERVATIONS` again.
  - Separate `storage_inspect` readbacks confirmed the counts: final `Normal`, `CF_EVENTS=1`, `CF_OBSERVATIONS=2`, `CF_REFLEX_AUDIT=1`, `CF_SESSIONS=1`, blocked CFs unchanged.
  - Cleanup: `release_all` zeroed state, daemon PID `56980` stopped, port `7846` closed.
- Final supporting checks passed: `cargo fmt --check`; `git diff --check`; `cargo check -p synapse-storage -j 2`; `cargo check -p synapse-reflex -j 2`; `cargo check -p synapse-mcp -j 2`; `cargo test -p synapse-storage pressure -- --nocapture`; `cargo test -p synapse-mcp --test m3_storage_tool -- --nocapture`; `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`; `cargo build --release -p synapse-mcp -j 2`.
- Final release binary readback: length `46320128`, SHA256 `8BCD4B02A37D85C40D15087C8A3B66A8963804CB8A5877CC5A349CE676EFB12B`, timestamp `2026-06-01T16:25:11.3649649Z`; diff review completed.
- Next exact action: commit with `[skip ci]`, post RESOLVED evidence to #618, close #618, then refresh queue and take #619 unless GitHub changed.

## Current Resume Point - 2026-06-01T10:53:00-05:00
- #617 is closed.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/617#issuecomment-4594236079
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T15:52:11Z`.
  - No code patch was required; worktree stayed clean after FSV.
  - Manual FSV run directory: `.runs\617\storage-fsv-20260601T1024`.
  - Repo-built daemon PID `73864`, bind `127.0.0.1:7845`, isolated DB, strict Inspector tools-list 80 tools.
  - Covered: per-CF write growth on `CF_EVENTS`, `CF_OBSERVATIONS`, `CF_SESSIONS`, `CF_ACTION_LOG`, `CF_KV`; per-CF row-cap GC 12 -> 9; hard-cap warning/continue 25 -> 10; invalid soft>hard; max value bytes; 128-byte prefix; empty rows=0; 129-byte invalid prefix; `AUDIT_RETENTION` report row.
  - Cleanup stopped daemon PID `73864`; port `7845` closed.
  - Supporting checks passed: fmt, `cargo check -p synapse-mcp -j 2`, focused storage GC regression, `m3_storage_tool`, schema sanitize, release build, and `git diff --check`.
- Active issue is #618 `scenario(stress): storage pressure ladder — 5 levels + write-gating`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/618#issuecomment-4594238857
  - Live queue after #617 closure: #594 plus #595-#604 and #618-#634.
  - Current next action: inspect pressure-level/write-gating code and tests, then launch a repo-built isolated daemon for #618 manual MCP FSV.

## Current Resume Point - 2026-06-01T10:21:23-05:00
- #616 is closed.
  - Commit: `79f735f fix(mcp): classify reality audit drift (#616) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/616#issuecomment-4593986844
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T15:20:44Z`.
- Active issue is #617 `scenario(stress): storage CF saturation to hard cap + GC eviction`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/617#issuecomment-4593992720
  - Live queue after #616 closure: #594 plus #595-#604 and #617-#634.
  - #617 requires manual MCP FSV of `storage_put_probe_rows`, `storage_inspect`, and `storage_gc_once` against isolated CF row/size SoT under storage pressure and eviction edges.
- Current next action: inspect storage tool/cap/GC implementation and supporting tests, then launch a repo-built isolated daemon for #617 FSV.

## Current Resume Point - 2026-06-01T10:12:14-05:00
- Active issue #616 has implementation, manual MCP FSV evidence, cleanup, final supporting checks, and diff review complete; commit, RESOLVED comment, close, and queue continuation are next.
- Patch in `crates/synapse-mcp/src/server/reality.rs` makes `reality_audit` classify concrete compact-state drift instead of treating every physical mismatch as generic `rebase_required`.
- FSV evidence is in `.runs\616\audit-fsv-20260601T0945`:
  - repo-built daemon PID `80292`, bind `127.0.0.1:7844`, isolated DB, strict Inspector `tools/list` with 80 tools;
  - source unavailable/missing baseline wrote `reality/audit/v1/chrome/audit-01780325611288876400-0000000001`;
  - baseline+delta+no-drift wrote two delta rows and audit `...0000000002` with `in_sync`;
  - minor title drift wrote audit `...0000000003` with `minor_drift`;
  - immediate rebase audit wrote audit `...0000000004` with `in_sync`;
  - major UI-structure drift wrote audit `...0000000006` with `major_drift`;
  - stale epoch wrote audit `...0000000007` with `rebase_required`;
  - invalid `depth=0` failed closed and left `CF_KV=18`, `CF_ACTION_LOG=14` unchanged.
- Cleanup is done: release_all called, target PID `13676` stopped, daemon PID `80292` stopped, port `7844` closed, no visible `Issue616*` or `Issue615FanoutTarget` windows remain.
- Final supporting checks passed: `cargo fmt --check`; `cargo check -p synapse-mcp -j 2`; full reality tests (20 passed); schema sanitize tests (3 passed); release build; `git diff --check` with line-ending warnings only.
- Final release binary readback: length `46380544`, SHA256 `86D55735BD2FA893E22B16E955D431474147B5F3CE1F616BCBD4EB1E047B201B`, timestamp `2026-06-01T15:18:29.1464141Z`.
- Next exact commands: commit with `[skip ci]`, post RESOLVED evidence to #616, close it, refresh queue.

## Current Resume Point - 2026-06-01T09:39:21-05:00
- Active issue #616 is patched locally but not yet fully verified or committed.
  - Patch in `crates/synapse-mcp/src/server/reality.rs`: `reality_audit` now itemizes drift by comparing the stored head compact state to the fresh captured compact state, classifies field-level severity, distinguishes source unavailable/stale/mismatch cases, and persists changed paths in `RealityDriftItem`.
  - Focused checks passed: `cargo fmt`; `cargo test -p synapse-mcp reality_audit_ --bin synapse-mcp -- --nocapture` (6 passed).
  - Next action: run broader supporting checks/release build, then launch an isolated repo-built daemon for #616 manual MCP FSV.

## #615 Closure Resume Point - 2026-06-01T09:31:17-05:00
- #615 is closed.
  - Commit: `fad86c9 fix(mcp): harden reality fanout coalescing (#615) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/615#issuecomment-4593549908
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T14:30:47Z`.
- Active issue is #616 `scenario(stress): reality drift injection -> reality_audit rebase`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/616#issuecomment-4593554251
  - Live queue after #615 closure: #594 plus #595-#604 and #616-#634.
  - #616 requires `reality_audit` drift verdicts to match physical divergence from the assumed baseline/head epoch/hash and to persist `CF_KV/reality/audit/*` rows through real MCP triggers.
- Current next action: inspect `reality_audit` implementation/tests and plan an isolated repo-built daemon FSV run for drift injection, no-drift, source-unavailable, minor/major boundary, audit-after-rebase, empty/no-change, and invalid params.

## #615 Closure Checkpoint
- Patch: `crates/synapse-mcp/src/server/reality.rs` excludes incidental focus and parent `children_count` changes from the UIA high-fanout coalescing threshold while preserving changed-id metadata on emitted aggregate deltas.
- Manual MCP FSV evidence is under `.runs\615\fanout-fsv-20260601T0844-patched`.
  - Repo-built daemon PID `64500`, bind `127.0.0.1:7843`, strict Inspector `tools/list` returned 80 tools.
  - Physical target PID `79124`, title `Issue615FanoutTarget`, separate OS UIA reads confirmed item counts/names after real MCP `act_click` triggers.
  - Covered Show7 per-element, Show8 coalesced, Rename8 `uia_elements_changed`, Mixed8 coalesced, Show80/Clear snapshot-budget rebase with no `CF_KV` row growth, empty/no-change, invalid `depth=0`, and disappear8.
  - Cleanup stopped PID `79124`, stopped PID `64500`, and port `7843` no longer listens.
- Final supporting checks passed: `cargo fmt --check`; `cargo check -p synapse-mcp -j 2`; `cargo test -p synapse-mcp server::reality::tests --bin synapse-mcp -- --nocapture` (17 passed); schema sanitize tests (3 passed); release build; `git diff --check`.
- Release binary readback: length `46334464`, SHA256 `0EDEBFD08BB324FDCD835727A005C4A161D86C7C6BE5EE34E72FBBA96C8D8894`, timestamp `2026-06-01T14:28:17.6122521Z`.

## #614 Closure Checkpoint
- Final supporting checks passed after FSV: `cargo fmt --check`; `cargo check -p synapse-mcp -j 2`; `cargo test -p synapse-mcp server::reality::tests --bin synapse-mcp -- --nocapture` (14 passed); `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture` (3 passed); `cargo build --release -p synapse-mcp -j 2`; `git diff --check` with line-ending warnings only.
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46350848`, SHA256 `18F213F8799AFA64ACCB31F3C3F07F98D40ADF3E081D3C05B256A8FC957BEED4`, `LastWriteTimeUtc=2026-06-01T13:14:38Z`.
- Main evidence run: `.runs\614\reality-loop-fsv-20260601T0741-patched`.
  - Repo-built daemon PID `82340`, bind `127.0.0.1:7840`, strict Inspector tools-list 80 tools, baseline/head rows, 18 total SSE `reality_delta` frames, audit row `reality/audit/v1/unprofiled/audit-01780319054227975800-0000000001`, and cursor/error edges captured.
  - Cleanup stopped PID `82340`, curl PID `82448`, and Luanti PID `36460`; an orphan TCP LISTEN row for `127.0.0.1:7840` still references non-existent PID `82340`, so use a different port if needed.
- FS-watch evidence run: `.runs\614\fs-watch-fsv-20260601T0805`.
  - Repo-built daemon PID `77940`, bind `127.0.0.1:7841`, `SYNAPSE_FS_WATCH_ROOT` set to the run `watch` directory, strict Inspector tools-list 80 tools.
  - Real MCP `act_run_shell` wrote `issue614-fs-watch-marker.txt`; separate file read matched text/hash; `observe_delta` returned `/fs` created-file delta; `storage_inspect` read CF_KV baseline/delta/head rows.
  - Cleanup stopped PID `77940`; port `7841` has only TIME_WAIT rows.

## #614 Patch Checkpoint
- Current worktree change: `crates/synapse-mcp/src/server/reality.rs`.
- Patched:
  - omitted-profile `reality_baseline` now observes first, selects the active profile, and reuses that profile's existing head/baseline when `epoch_id` is omitted and `force_new_epoch=false`;
  - `observe_delta.since_epoch` now validates through the same reality key-segment validation used for baseline/audit epochs before stale-epoch comparison;
  - `capture_reality_observation` now fails closed on `depth=0`, `depth>6`, `max_elements=0`, or `max_elements>500` instead of clamping bypassed invalid values.
  - `observe_delta` now returns `profile_changed` rebase guidance for a known observed profile switch instead of failing requested-profile validation before the edge can be represented.
- Focused checks passed:
  - `cargo fmt`
  - `cargo test -p synapse-mcp reality_baseline_reuses_observed_profile_when_profile_id_is_omitted --bin synapse-mcp -- --nocapture`
  - `cargo test -p synapse-mcp observe_delta_edges_return_rebase_or_fail_closed --bin synapse-mcp -- --nocapture`
  - `cargo test -p synapse-mcp reality_tools_reject_out_of_range_snapshot_params --bin synapse-mcp -- --nocapture`
  - `cargo test -p synapse-mcp observe_delta_reports_profile_changed_for_requested_head_mismatch --bin synapse-mcp -- --nocapture`
- Broader supporting checks passed:
  - `cargo test -p synapse-mcp server::reality::tests --bin synapse-mcp -- --nocapture` (14 passed)
  - `cargo fmt --check`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo build --release -p synapse-mcp -j 2`
- Stopped stale pre-patch isolated #614 daemon PID `75352`.
- Release binary: `target\release\synapse-mcp.exe`, length `46350848`, SHA256 `319FC6F5942ABF272EDCCA7A1EEF7970EE7AE0C7CB6A11A515F681B74F6854A1`, timestamp `2026-06-01T12:39:53Z`.
- Host-sensor plan: use Luanti (`%LOCALAPPDATA%\synapse\benchmarks\luanti\engine\5.16.1\luanti-5.16.1-win64\bin\luanti.exe`) for real foreground/UIA/HUD/entity/diagnostics deltas, Notepad/clipboard/filesystem tools for additional physical changes, and `--enable-audio` plus a local sound trigger for audio if loopback initializes.
- Next: launch isolated #614 daemon, strict Inspector `tools/list`, then manual FSV.

## #614 Manual FSV Evidence
- Main run directory: `.runs\614\reality-loop-fsv-20260601T0741-patched`.
- FS subrun directory: `.runs\614\fs-watch-fsv-20260601T0805`.
- Covered feeds:
  - baseline/head: CF_KV baseline/head rows for `issue614-luanti-20260601T0743` and `issue614-luanti-profiled-20260601T0746`;
  - foreground/focus/UIA/HUD/entity/audio/clipboard: 12-delta main cursor walk, CF_KV 3->15, SSE stream seq 1..12;
  - diagnostics: boundary `max_elements=1` changed `elements_truncated=false` to `true`;
  - filesystem: FS-watch subrun `/fs` created-file delta, CF_KV 2->3, file SHA256 readback;
  - audit: `reality_audit` persisted audit row and returned drift/rebase guidance.
- Edge cases:
  - empty/no-change: `since_seq=12` returned `no_changes`, no CF_KV delta rows;
  - missing baseline: `profile_id=missing614` returned `missing_baseline`;
  - stale epoch: old epoch returned `stale_epoch`;
  - profile change: Notepad foreground returned `profile_changed: head profile unprofiled but observed notepad`;
  - future/overflow/malformed/structural invalids: future seq, overflow seq, malformed epoch, and `depth=0` all failed closed.

## #614 Scope
- Goal: prove the delta-first reality model end to end across every sensor feed.
- Required runtime evidence:
  - real repo-built `synapse-mcp` daemon process/bind/auth/health/session/strict Inspector `tools/list`;
  - real MCP `tools/call` triggers for `reality_baseline`, `observe_delta`, `reality_audit`, and event-producing tools;
  - separate SoT readbacks for `CF_KV/reality/baseline/*`, `reality/head/*`, `reality/delta/*`, `reality/audit/*`, `reality_delta` SSE frames, and physical foreground/focus/UIA/HUD/entity/audio/clipboard/filesystem/diagnostics changes where available;
  - happy path plus missing baseline, stale epoch, profile-change mid-walk, future/overflow `since_seq`, empty/no-change, boundary, and structurally invalid params.

## #613 Manual FSV Evidence
- Final run directory: `.runs\613\subscribe-firehose-fsv-20260601T062230-patched`
- Daemon: PID `32356`, bind `127.0.0.1:7839`, repo release binary, isolated DB/watch/log dirs.
- Precondition: process/socket/auth readback passed; unauth `/health=401`; auth `/health ok=true`; official MCP Inspector strict `tools/list` returned 80 tools with #613 tools present.
- One-per-event: subscription `019e82ec-ebf5-7943-884e-03590d0a05f2` delivered exactly 3 frames for `/focused,/clipboard,/fs`; stream/event seqs `1,2,3`; no drops; file and clipboard physical SoTs matched marker `issue613-patched-oneper-20260601T062403456`.
- 8-deep filter: subscription `019e82ee-5d56-72f2-92c0-00e3c4a73063` accepted regex/in_set/exists filter at max depth and delivered only `/clipboard` and `/fs` from four published reality deltas.
- Firehose/backpressure: subscription `019e82ef-c53f-7e13-ae2c-cfea7dbd3ae8`; 5000 known events posted; stats read `ring_len=4096`, `oldest_event_seq=904`, `latest_event_seq=4999`, `dropped_total=904`, `events_dropped_for_subscriber=904`, `lossy_pending=true`; replay had 1 lossy preface and 4096 event frames.
- Edges: depth 9, invalid regex, invalid data path, and bad buffer size rejected through strict Inspector; empty filter All delivered event seq `613000`; subscribe/immediate cancel produced `cancelled=true`, stats 404 after cancel and after matching publish.
- Cleanup: subscriptions cancelled, `sse_subscribers=0`, `release_all` zero, daemon stopped, port `7839` closed.

## Final Supporting Checks
- `cargo fmt --check`
- `git diff --check` (line-ending warnings only)
- `cargo check -p synapse-core -j 2`
- `cargo check -p synapse-reflex -j 2`
- `cargo check -p synapse-mcp -j 2`
- `cargo test -p synapse-core event_filter_validation_edges_have_readback --test event_filter_types -- --nocapture`
- `cargo test -p synapse-mcp last_event_id_zero_reuses_empty_existing_subscription --bin synapse-mcp -- --nocapture`
- `cargo test -p synapse-mcp ring_overflow_reports_drop_metric_and_lossy_frame --bin synapse-mcp -- --nocapture`
- `cargo test -p synapse-mcp --test m3_subscribe_tool -- --nocapture`
- `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
- `cargo test -p synapse-reflex --test bus_behavior -- --nocapture`
- `cargo build --release -p synapse-mcp -j 2`
- Release binary: `target\release\synapse-mcp.exe`, length `46359552`, SHA256 `426E96F4CA1C07D92433284FEBD39A161722C256133265AD6472B4E1D51DB67C`, timestamp `2026-06-01T12:09:18.7698237Z`.

## Standing Rules
- Re-read `C:\code\Synapse\docs\AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, and `STATE/*` after compaction.
- GitHub Issues are the only coordination surface.
- No GitHub Actions/CI dispatch, waits, or CI-gated claims.
- Commits pushed by this agent must include `[skip ci]`.
- Automated checks/benches are supporting regression evidence only; they are not FSV.
- Missing local prerequisites are acquisition/setup work, not blockers, unless only a specific operator-only hard-to-reverse external action remains.
