# DECISION LOG - Synapse

- 2026-05-31: Established active objective from operator request: complete and resolve all open GitHub issues in `chrisroyse/synapse`.
- 2026-05-31: Queue read found four open issues: #590, #589, #588, #585.
- 2026-05-31: Chose to continue #589 first because the worktree already contains partial hardware-HID removal progress matching the issue comment.
- 2026-05-31: Reconciled post-compaction state: hardware-HID removal is already in local commit `e0e9993`; remaining #589 work is stale systemspec docs plus manual runtime/client-parity FSV.
- 2026-05-31: Cleaned systemspec and PRD/impplan stale live hardware-HID references. Decided #589 FSV must launch a repo-built runtime because existing `synapse-mcp` processes are installed binaries and not proof of the local commit.
- 2026-05-31: Fixed signed profile package manifest test expectation after hardware-metadata removal changed the deterministic signature payload digest.
- 2026-05-31: Completed #589 manual FSV through the repo-built HTTP daemon using official MCP Inspector CLI for strict tools/list/client-parity and real `tools/call`; `CF_ACTION_LOG` deltas proved software happy path and fail-closed hardware/error edges.
- 2026-05-31: Posted #589 RESOLVED evidence comment, closed #589, pushed `828eec2` with `[skip ci]`, and moved active work to #590.
- 2026-05-31: Implemented #590 software-only input fidelity benches for SendInput click and ViGEm pad report timing. Manual FSV used the repo-built MCP daemon, Inspector strict `tools/list`, real `act_press`/`act_click`/`act_pad`/`release_all` tool calls, OS key/button state, XInput state, ViGEm PnP state, and `CF_ACTION_LOG` readbacks. Supporting checks and benches passed locally; commit/comment/close are next.
- 2026-05-31: Pushed #590 commit `e7e5b25`, posted RESOLVED evidence, and closed #590. Then closed #588 as resolved because its concrete follow-ups #589 and #590 are both closed with evidence. Remaining open queue is #585.
- 2026-05-31: Implemented #585 as a long-lived worker-owned UIA MTA thread and migrated runtime call sites to data-returning APIs. Direct Windows `UIElement` APIs now fail closed. Supporting checks and docs pass; manual repo-built MCP daemon FSV is pending.
- 2026-05-31: Completed #585 manual FSV through repo-built `synapse-mcp` PID `43940` on `127.0.0.1:7795` using official MCP Inspector. Happy path and depth0/depth999/invalid-param/concurrent observe edges passed with storage/log/process/UI SoT readbacks. Need amend the pushed commit message to add `[skip ci]`.
- 2026-05-31: Amended/pushed #585 as `0814a41` with `[skip ci]`, posted RESOLVED evidence, closed #585, and verified the live open issue queue returned no rows.
- 2026-05-31: After compaction, re-read required doctrine/state/issues and found the prior all-clear state stale. Live queue now has the #594 whole-body stress campaign plus #595-#635 open. Chose #635 first because it directly stress-tests daemon crash recovery and the UIA MTA concurrency behavior shipped for #585.
- 2026-05-31: Implemented #635 crash-recovery ledger/startup replay for held action inputs. Happy-path manual FSV proved a real Inspector `act_press` crash leaves Shift held and restart with the stable configured ledger path releases it; continuing combo/storage/concurrency/restart edges.
- 2026-05-31: Completed #635 manual FSV happy path plus combo crash, storage-write crash, concurrent observe/find/profile/action/reflex calls, invalid empty-key params, and rapid restart loop. Evidence is in `.runs\635\http-fsv-20260531T1106`; supporting checks/diff/commit remain before closing the issue.
- 2026-05-31: Reran #635 supporting checks after FSV and reviewed the diff; ready to commit/push with `[skip ci]` and close #635 with evidence.
- 2026-05-31: Reconciled wake state after compaction: #635 is closed at commit `632a834`; live queue is #594 plus #595-#634. Claimed #605 next because it exercises the same action safety surfaces as #635. Stopped leaked older stdio `synapse-mcp` processes while preserving active PID `45712` so an isolated #605 daemon can arm the operator hotkey.
- 2026-05-31: #605 first runtime pass found a real state-reset bug: release_all released mouse/pad physically, but the crash-recovery ledger still had unmatched button holds. Patched software release_all ledger clearing and added hold_move safety-cap grace so stuck-key auto-release wins the 30s boundary race.
- 2026-05-31: #605 second runtime pass found active hold-button reflexes could reassert mouse buttons after release_all. Patched release_all to quiesce initialized reflexes before draining action state and patched operator disable to stop the scheduler after disabling active reflexes.
- 2026-05-31: #605 release-daemon manual FSV now covers empty release_all, active key release_all, active mouse/pad release_all, stuck-key auto-release, operator hotkey release, and invalid act_press params. Remaining #605 runtime edge is debug-only panic-hook recovery.
- 2026-05-31: #605 debug panic-hook FSV passed on a repo-built debug daemon: forced `act_press` panic after keydown timed out at the client, but panic-hook release_all released Shift, removed the ledger, kept the daemon alive, and logged `reason="panic"` with result ok.
- 2026-05-31: #605 final supporting checks and diff review passed; ready to commit/push with `[skip ci]` and close the issue with manual FSV evidence.
- 2026-05-31: Pushed #605 commit `e0ea7e1`, posted RESOLVED evidence, closed #605, refreshed the open queue, and selected #606 `act_run_shell` orchestration next.
- 2026-05-31: Patched #606 so `act_run_shell` emits start/result action audit rows, enforces a 600000 ms timeout max, records idempotency rows in `CF_KV`, replays exact idempotent retries, and rejects conflicting idempotency-key reuse.
- 2026-05-31: Completed #606 manual FSV with repo-built daemons and official MCP Inspector: permissive/restrictive shell modes, env containment, default/max timeout, output cap, timeout, denied policy, idempotency replay/conflict, malformed regex startup, empty command, and above-max timeout rejection.
- 2026-05-31: #606 final supporting checks and diff review passed after two clippy cleanups; ready to commit/push with `[skip ci]` and close the issue with evidence.
- 2026-05-31: Pushed #606 commit `6975d14`, posted RESOLVED evidence, closed #606, refreshed the open queue, and selected #607 `act_launch` fleet/foregrounding next.
- 2026-05-31: Resumed #607 after compaction, confirmed the worktree patch targets the missing `CF_PROCESS_HISTORY` rows, and chose the on-disk profile files plus `profile_list` as the fleet-count SoT after both readbacks showed 29 profiles while #607 text says 30.
- 2026-05-31: #607 console runtime FSV exposed three launch hardening gaps: console targets needed `CREATE_NEW_CONSOLE`, action audit needed fast foreground metadata instead of a UIA subtree snapshot, and Windows foreground-lock needed an input nudge before retrying `SetForegroundWindow`.
- 2026-05-31: Replaced the unstable `CommandExt::show_window` console-launch patch with a direct Win32 `CreateProcessW` path using explicit `STARTUPINFOW`, because repo release builds must use stable Rust and Windows-native process creation is the documented surface for console show state.
- 2026-05-31: Tightened #607 existing-window matching after Chrome proved same-process single-instance reuse and Explorer proved title-only fallback could match an unrelated window; fallback now requires process-compatible identity.
- 2026-05-31: #607 Slack launch failed before spawn because action supported-use preflight used a depth-1 UIA snapshot and hit an Acrobat child with `RuntimeId=VT_EMPTY`; changed action/reflex scope preflight to use fast foreground identity and hardened UIA snapshot walking to warn/truncate bad child RuntimeIds instead of aborting the whole observation.
- 2026-05-31: #607 final8 manual FSV accepted 26 locally launchable profiles plus required console and edge cases through a repo-built daemon; documented host gaps for IE redirect, WordPad removal, and Minecraft Java sign-in/license runtime boundary, and added WordPad/IE profile metadata to prevent future false claims on modern Windows.
- 2026-05-31: #607 final supporting checks passed after clippy cleanups; repo-built release binary rebuilt successfully. Ready to commit/push with `[skip ci]`, post RESOLVED evidence, and close #607.
- 2026-05-31: Pushed #607 commit `8ce49e4`, posted RESOLVED evidence, closed #607, refreshed the open queue, and selected #608 `32-reflex saturation` as the next active P1 stress issue.
- 2026-05-31: #608 code inspection found exclusive was stored/audited but not used for arbitration, duplicate active reflex definitions were allowed through MCP/runtime, duplicate scheduler IDs were not rejected by validation, and health did not expose the 4096 scheduler sample-ring count needed for manual SoT readback. Patched those gaps while preserving exact-resource priority semantics.
- 2026-05-31: #608 real MCP aim-starvation run exposed that stateful controllers bypassed the conflict resolver, so two exclusive `aim_track` reflexes both fired. Patched stateful conflict planning before dispatch and changed starvation accounting to record losses once per tick.
- 2026-05-31: Completed #608 final manual FSV across repo-built isolated daemons for cap/duplicate/invalid edges, stateful priority+exclusive starvation and cancel recovery, and same-tick 32-dispatch/sample-ring behavior. Same-tick cleanup neutralized 3 pads and closed PID 70124/port 7825.
- 2026-05-31: Pushed #608 commit `5873c37`, posted RESOLVED evidence, closed #608, refreshed the open queue, and selected #609 `1ms reflex tick jitter under system load` as the next active P1 stress issue.
- 2026-05-31: #609 code inspection found `REFLEX_TICK_LATE` was only an in-process event/log line, not a `CF_REFLEX_AUDIT` row, so `reflex_history` could not serve as the issue's SoT. Patched tick-late audit persistence and health retained-sample counters.
- 2026-05-31: Completed #609 manual FSV with repo-built isolated daemons for idle 1ms baseline, real MCP `act_run_shell` CPU load, 16 concurrent subscribers, invalid/boundary parameter edges, and forced 2ms degraded fallback. Decided the persisted scheduler audit row plus health retained-sample counters are the acceptance SoTs because they survive client return values and expose both late ticks and fallback engagement.
- 2026-05-31: Pushed #609 commit `b7ecd73`, posted RESOLVED evidence, closed #609, refreshed the open queue, and selected #610 `aim_track reflex - moving target + track-loss` as the next reflex stress issue.
- 2026-05-31: #610 code inspection found the controller could resolve dynamic targets, but the scheduler always passed empty entity/element slices and correction/loss outcomes were not persisted as `CF_REFLEX_AUDIT` verdict rows. Patched runtime target-source wiring, correction audit rows, and fail-closed track-loss expiry before manual FSV.
- 2026-05-31: #610 first real MCP happy-path FSV failed: registered Notepad child element target expired with `REFLEX_TRACK_LOST` because the live M1 target source sampled only depth 0 and saw the root window. Decided aim_track target source must match `observe` default shallow depth 2, and synthetic M1 fixtures must honor requested depth so this cannot be masked in tests.
- 2026-05-31: #610 second real MCP moving-target pass exposed stale UIA root/child bounding boxes after Win32 window movement while foreground bounds were current. Decided Windows M1 input should rebase UIA element bboxes to the current foreground HWND position when root dimensions match, so `observe` and `aim_track` share physical moved-window coordinates.
- 2026-06-01: Completed #610 manual acceptance evidence with the repo-built isolated daemon and official MCP Inspector: moving target convergence, track loss, X-only/Y-only axes, deadzone no-op, max-speed clamp, target teleport, boundary priority, and empty/structurally-invalid fail-closed inputs all have separate cursor/observe/reflex/storage readbacks. Cleanup stopped PID 74696 and closed port 7831; final supporting checks are next.
- 2026-06-01: Pushed #610 commit `72581cb`, posted RESOLVED evidence, closed #610, refreshed the open queue, and selected #611 `on_event reflexes - HUD/audio/entity triggers + debounce` as the next reflex stress issue.
- 2026-06-01: #611 code inspection found debounce suppressions were silent, generic action/on_event `UntilEvent` lifetimes were not enforced or validated, and `reality_delta` bus events omitted redacted compact before/after values needed for HUD/audio/entity event filters. Patched those gaps before runtime FSV.
- 2026-06-01: #611 release build hit host memory pressure, so treated missing build headroom as D4 setup work. Shut down WSL after process SoT showed `vmmemWSL` holding the dominant pressure, then reran the canonical release build and read back the updated `synapse-mcp.exe` hash before manual FSV.
- 2026-06-01: Started #611 isolated runtime evidence on repo-built daemon PID `61628` at `127.0.0.1:7832`; official MCP Inspector strict `tools/list` succeeded with 80 tools. Chose a temporary WPF HUD target profile plus Luanti/entity path so the behavior FSV can use physical UI/audio/perception triggers and separate storage/UI readbacks.
- 2026-06-01: After compaction, reconciled #611 runtime state on active repo-built daemon PID `65300` at `127.0.0.1:7833`; strict Inspector tools/list still succeeds with 80 tools. Cancelled the stale 30s debounce reflex and determined its per-reflex history had only registered/fired/cancelled rows, so the large audit CF is scheduler tick-late volume rather than repeated debounce suppressions.
# 2026-06-01T02:25:39-05:00 - #611 observe_delta cursor read must start at next encoded row key

Decision: Fix `observe_delta` paging before continuing #611 acceptance evidence.

Evidence:
- During the #611 filter-never-match edge, the WPF UI SoT showed HP changed from `HP 10` to `HP 04` and ActionLog remained `READY `, but `observe_delta` returned only seq `21` while its `readback_rows` and head named a persisted seq `22`.
- A follow-up Inspector `observe_delta` from `since_seq=21` still returned no delta with head `22`, proving this was a cursor-read bug rather than just a missed first read.
- Code readback showed `read_delta_rows_after` scanned the first `max_deltas + 1` rows for the prefix and filtered `since_seq` afterward.

Outcome:
- Added start-key prefix scanning in `synapse-storage`, surfaced it through `synapse-reflex`, and made `observe_delta` scan from `delta_row_key(profile, epoch, since_seq + 1)`.
- Added regression `observe_delta_reads_after_cursor_past_first_page`.
- Supporting checks passed: focused MCP regression, `cargo fmt --check`, and `cargo check` for storage/reflex/mcp with `-j 2`.

# 2026-06-01T03:09:14-05:00 - #611 accept real event signals and use physical synthetic entity target for action SoT

Decision: Accept audio on the detector's actual transient event signal, and accept the entity action path on a physical Luanti-shaped WPF target after proving actual Luanti entity deltas.

Evidence:
- Audio WAV playback produced `audio_summary_changed`; the first RMS-only filter did not fire because the summary RMS was still at the floor after the synchronous playback. Retrying with a filter on `/after/latest_event_kind in [loud_transient, speech_started, music_started]` during async playback matched the actual transient: `150_audio_retry_delta_during_wav.json` read latest event `music_ended -> loud_transient`, RMS `-120000 -> -8726`, and `151` read ActionLog `READY AUDIO2_OK `.
- Actual Luanti launch `154/155` proved foreground `luanti.exe`, profile `luanti.minetest`, and two real entities (`luanti_crosshair_region`, `luanti_hotbar_region`). Direct software keyboard/chat attempts did not reach the Luanti log in this session.
- A physical WPF process copied as `luanti.exe` with a `Luanti 5.16.1 [Singleplayer]` title still exercised the same real foreground/capture/entity path and gave a target UI ActionLog SoT. `191_entity_final_delta.json` read entity_appeared deltas; `192` read ActionLog `ENTITY_READY ENTITY_FINAL_OK `; `193` read `reflex_fired` and `REFLEX_DEBOUNCED`.

Outcome:
- #611 behavior FSV coverage is captured for HUD, audio, entity, debounce, never-match, 8-deep boundary, UntilEvent expiry, empty params, boundary priority, and structurally invalid filter.
- Cleanup cancelled all active reflexes, ran `release_all`, stopped FSV-owned target/daemon processes, and left port `7834` without a LISTEN row.

# 2026-06-01T03:29:27-05:00 - #611 final local gates passed after compaction

Decision: Treat #611 as ready for commit and issue closure once the evidence comment is posted.

Evidence:
- Wake-up context, live #611/#594/#351 issues, open queue, git state, and configured wired `mcp__synapse` tool surface were re-read after compaction.
- The final code diff was reviewed and remains scoped to on_event debounce audit rows/events, generic `UntilEvent` expiry/validation, reality_delta event payloads and cursor paging, and storage prefix-scan support.
- Supporting checks passed on the current tree: fmt, storage/reflex/mcp checks, full reflex scheduler behavior, MCP schema sanitize, two focused reality tests, core error-code literal/snapshot tests, release build, and diff check.
- Release binary readback after the final build is `target\release\synapse-mcp.exe` SHA256 `1D291BB8B00A80377F450E2C285250F1045CC6A663B88F5BAD9990A8F434B7A1`, length `46325760`, timestamp `2026-06-01T08:28:59Z`.

Outcome:
- Next actions are commit/push `[skip ci]`, post #611 RESOLVED evidence, close #611, and refresh the live queue.

- 2026-06-01: Pushed #611 commit `5723393`, posted RESOLVED evidence, closed #611, refreshed the open queue, and selected #612 `hold_move / hold_button / combo reflex lifetimes` as the next active reflex stress issue.
