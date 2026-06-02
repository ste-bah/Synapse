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
- 2026-06-01: #612 code inspection found `hold_move.re_assert` was only schema/docs state and `reflex_cancel` did not release active hold inputs. Patched reassert keydown dispatch and cancel-time hold release action queuing before runtime FSV.
- 2026-06-01: #612 MCP `reflex_register` supporting test timeout root cause was non-aim reflex ticks sampling the aim-track M1 target source before checking controller presence. Patched `step_aim_track` to return before cursor/M1 reads for non-aim slots and added a regression proving `on_event` ticks do not call the aim target source.

# 2026-06-01T04:39:15-05:00 - #612 reassert must not grow crash-recovery ledger per tick

Decision: Keep physical reassert KeyDown dispatch, but suppress duplicate logical crash-recovery ledger writes.

Evidence:
- Real isolated #612 manual evidence on daemon PID `47904` showed `hold_move re_assert=true` correctly kept W physically down after an external KeyUp, proving the reassert behavior itself was needed.
- The separate recovery-ledger SoT read showed many duplicate `key_held` rows for W in `action_recovery.jsonl`, so long-running reassert would create unbounded crash-recovery file growth even though the logical held-input state had not changed.

Outcome:
- `append_recovery_event_at` now replays the current logical ledger and skips appending an event when it would leave recovered state unchanged and the existing file has no ignored trailing bytes.
- Added `recovery_log_skips_duplicate_logical_holds`.
- Supporting checks passed: `cargo fmt`, focused `synapse-action` recovery test, `cargo check -p synapse-action -j 2`, and `cargo build --release -p synapse-mcp -j 2`.

# 2026-06-01T04:51:40-05:00 - #612 reassert dispatch must be periodic, not per-tick

Decision: Rate-limit `hold_move.re_assert` dispatch to a fixed 50 ms interval.

Evidence:
- Fresh real MCP evidence after the duplicate-ledger fix showed W stayed physically down and the recovery ledger stayed bounded at one row, but `reflex_cancel` returned `cancelled=true` while the after-read still showed W down and the ledger present.
- A delayed after-read still showed W down. `release_all` then released one key and removed the ledger, proving the backend could release W and the cancel problem was caused by queued reassert KeyDowns.
- Code readback showed the scheduler dispatched a reassert KeyDown every 1 ms tick while the hold was active.

Outcome:
- `HoldMoveController` now suppresses reassert until 50 ms have elapsed since the initial hold or last reassert.
- Focused regression updated to prove a 1 ms tick is only `Holding` with no queued action, while the next interval tick emits `Reasserted`.
- Supporting checks passed: `cargo fmt`, focused hold_move reassert test, focused recovery-ledger test, `cargo check` for `synapse-reflex` and `synapse-action`, and release build.

# 2026-06-01T05:33:52-05:00 - #612 cancel expired reflexes from persisted terminal audit state

Decision: Make `reflex_cancel` use the same persisted terminal status surface as `reflex_list include_expired=true` before returning `NotFound`.

Evidence:
- Manual #612 edge evidence showed `reflex_cancel` returned `cancelled=false, reason=not_found` for a one-shot combo while `reflex_list include_expired=true` still reported that same reflex as `expired`.
- Code readback showed `cancel` checked only `self.statuses()` from the live scheduler, while listing merges terminal statuses from `CF_REFLEX_AUDIT`.
- A scheduler can drop an already-expired reflex from the live active set while the persisted audit rows remain the user-visible terminal state.

Outcome:
- Added `terminal_status_from_audit(reflex_id)` and used it in `ReflexRuntime::cancel` when the live scheduler snapshot has no status.
- Expired/action-denied historical statuses now return `AlreadyExpired`; historical cancelled statuses retain the existing cancelled outcome.
- Added supporting regression `cancel_expired_reflex_restored_from_audit_reports_already_expired`.
- Fresh repo-built manual MCP rerun under `.runs\612\hold-lifetime-fsv-20260601T0530-cancel-expired` proved the edge: expired combo `019e82be-1a45-7d00-a817-22a9d7248818`, real Inspector `reflex_cancel`, response `already_expired`, OS P false before/after, recovery ledger absent, `reflex_history` lifecycle rows intact, `CF_REFLEX_AUDIT=2`.

- 2026-06-01: Pushed #612 commit `db761fe`, posted RESOLVED evidence, closed #612, refreshed the open queue, and selected #613 `subscribe firehose - 4096 ring, EVENTS_DROPPED, one-per-event, deep filters` as the next event-stream stress issue.

# 2026-06-01T07:09:36-05:00 - #613 harden SSE subscribe firehose behavior

Decision: Treat the #613 runtime findings as real defects and patch the event stream path before closure.

Evidence:
- Code readback showed `EventFilter::Data` validation did not validate JSON Pointer paths or regex patterns, so invalid filters could be accepted and then silently match false.
- Manual SSE readback on a fresh subscription with `Last-Event-ID: 0` exposed that `/events?subscription_id=<id>` created a new All subscription if the requested subscription's ring was still empty.
- Firehose stress needed drop accounting at the SSE ring layer as well as the event-bus queue layer; otherwise ring evictions were visible in stats but did not update the configured dropped-event metric surface.

Outcome:
- Added fail-closed data-filter validation for invalid paths and regex.
- Changed SSE reconnect to reuse an explicitly requested empty subscription when `Last-Event-ID: 0`.
- Added ring-overflow metric/drop accounting for per-subscription SSE ring eviction.
- Manual patched FSV under `.runs\613\subscribe-firehose-fsv-20260601T062230-patched` proved one-per-event delivery, 8-deep filters, 5000-event firehose drops, invalid edges, empty filter All, cancel behavior, cleanup, and strict Inspector tools-list.
- Supporting checks and final release build passed; next action is commit and close #613.

# 2026-06-01T07:16:00-05:00 - #614 reality-delta full-loop work starts after #613 closure

Decision: Take #614 next in the #594 stress campaign and inspect the reality baseline/delta/audit code before changing behavior.

Evidence:
- Live GitHub readback after compaction shows #613 closed and #614 open with no previous comments.
- #614 requires real MCP `reality_baseline`, `observe_delta`, and `reality_audit` triggers plus separate CF_KV/SSE/physical SoT reads for baseline, head, delta, audit, and sensor state.
- Posted #614 START comment at https://github.com/ChrisRoyse/Synapse/issues/614#issuecomment-4592477025.
- Wired `mcp__synapse` client readback succeeded for `health`, `storage_inspect`, `reflex_list`, `reflex_history`, and `observe`.

Outcome:
- Next action is implementation/test inspection for reality baseline/delta/audit, followed by a repo-built isolated daemon and manual MCP FSV.

# 2026-06-01T07:36:00-05:00 - #614 fail closed and reuse the observed-profile baseline

Decision: Patch reality-tool server semantics before runtime FSV so omitted-profile baselines reuse the active profile head and invalid cursor/snapshot params fail closed.

Evidence:
- Code readback showed `reality_baseline` used `UNPROFILED_PROFILE_KEY` for the pre-observe reuse check when `profile_id` was omitted. That misses an existing `reality/head/v1/<observed-profile>` row and can create a new epoch for the active profile.
- Code readback showed `observe_delta.since_epoch` was not passed through `validate_key_segment` before comparison.
- Code readback showed `capture_reality_observation` clamped `depth`/`max_elements`, so a schema-bypassing caller could get accepted behavior for invalid params.

Outcome:
- Added an observed-profile reuse path for omitted-profile `reality_baseline`.
- Added `since_epoch` validation and server-side `depth`/`max_elements` bounds checks.
- Added focused regressions for baseline reuse, malformed epoch rejection, and out-of-range snapshot params; all focused checks passed.

# 2026-06-01T07:40:09-05:00 - #614 profile-change must be rebase guidance

Decision: Treat a known observed-profile switch during `observe_delta` as a rebase response, not a request parameter error.

Evidence:
- #614 explicitly requires a profile-change mid-walk edge.
- Code readback showed the stored head was selected from the requested `profile_id`, but the subsequent live observation used `select_profile(requested, observation)`, which rejects a known observed mismatch before the `profile_changed` response branch can run.
- The first narrow patch made the new edge pass, but the broader reality suite showed synthetic observations with no resolved profile were incorrectly treated as `unprofiled` switches.

Outcome:
- `observe_delta` now compares the stored requested head to the live observed profile only when the observation resolves a profile; unresolved observations retain the requested head profile.
- Added `observe_delta_reports_profile_changed_for_requested_head_mismatch`.
- Supporting checks passed: focused profile-change regression, all 14 reality tests, fmt check, `cargo check -p synapse-mcp -j 2`, schema sanitize tests, and release build.

# 2026-06-01T08:10:00-05:00 - #614 filesystem feed required an FS-watch subrun

Decision: Treat the missing `/fs` delta from the main #614 daemon as setup state, not acceptance failure, and prove the filesystem sensor with a second isolated repo-built daemon configured with `SYNAPSE_FS_WATCH_ROOT`.

Evidence:
- The main daemon was launched without `SYNAPSE_FS_WATCH_ROOT`, so `populate_fs_recent` had no watcher and the main `observe_delta` correctly produced foreground/focus/UIA/HUD/entity/audio/clipboard/diagnostics deltas but no `/fs` delta.
- Code readback showed the filesystem sensor is enabled only from the `SYNAPSE_FS_WATCH_ROOT` environment variable.
- D4 says a missing configured-host prerequisite is acquisition/setup work, not a blocker.

Outcome:
- Launched `.runs\614\fs-watch-fsv-20260601T0805` on `127.0.0.1:7841` with `SYNAPSE_FS_WATCH_ROOT` set to the run watch directory.
- Strict Inspector tools-list returned 80 tools; baseline/head rows were written; real MCP `act_run_shell` created a known file; separate file text/hash readback matched; `observe_delta` returned `/fs` `filesystem_summary_changed`; `storage_inspect` read CF_KV baseline/delta/head rows.

# 2026-06-01T08:19:00-05:00 - #615 follows #614 in the reality-delta campaign

Decision: Take #615 next after closing #614 because it is the next open reality-delta child under #594.

Evidence:
- Live queue after closing #614 lists #594 plus #595-#604 and #615-#634 open.
- #615 is open, has no prior comments, and requires high-fanout `observe_delta` coalescing and snapshot-budget evidence.
- Posted #615 START comment at https://github.com/ChrisRoyse/Synapse/issues/615#issuecomment-4592942496.

Outcome:
- Next action is code/test inspection for UIA high-fanout coalescing and snapshot budget behavior before launching a repo-built isolated daemon for manual MCP FSV.

# 2026-06-01T09:28:31-05:00 - #615 threshold pressure must ignore incidental UIA metadata churn

Decision: Count only structural changes and material coalescing field changes toward the UIA high-fanout threshold; keep incidental focus and parent `children_count` changes out of threshold pressure while retaining full changed-id metadata on emitted aggregate deltas.

Evidence:
- The first manual Show7 boundary run physically created 7 item buttons, but `observe_delta` emitted `uia_structure_changed` because focus movement and parent `children_count` changes inflated `fanout_count` to the threshold.
- Code readback showed `uia_element_fanout` used all `changed_uia_element_ids`, so incidental UIA metadata could turn a low-fanout structural change into a high-fanout coalesced delta.
- #615 requires exact 7 vs 8 threshold behavior, low-fanout per-element rows, and high-fanout coalescing.

Outcome:
- Added `coalescing_uia_element_change_count` and `compact_element_has_coalescing_field_change`.
- Added supporting regressions for incidental changes below threshold, exact threshold coalescing, and mixed structure+field churn.
- Manual MCP FSV with repo-built daemon PID `64500` on `127.0.0.1:7843` proved Show7 per-element, Show8 coalesced, Rename8 coalesced, Mixed8 coalesced, Show80/Clear snapshot-budget rebase with no row growth, empty/no-change, invalid depth, and disappear8 behavior.
- Final checks and release build passed; next action is commit, RESOLVED comment, and close #615.

# 2026-06-01T09:31:17-05:00 - #616 follows #615 in the reality-delta campaign

Decision: Take #616 next after closing #615 because it is the next open reality-delta child under #594.

Evidence:
- `gh issue view 615` read back `state=CLOSED`, `closedAt=2026-06-01T14:30:47Z`.
- Live queue after #615 closure lists #594 plus #595-#604 and #616-#634 open.
- #616 is open, has no prior comments, and requires `reality_audit` drift injection and rebase evidence.
- Posted #616 START comment at https://github.com/ChrisRoyse/Synapse/issues/616#issuecomment-4593554251.

Outcome:
- Next action is code/test inspection for `reality_audit`, followed by repo-built isolated daemon setup and manual MCP FSV.

# 2026-06-01T09:39:21-05:00 - #616 audit drift must be classified from changed fields, not hash inequality alone

Decision: Patch `reality_audit` to compute itemized physical drift from compact-state differences and apply a highest-severity-wins policy.

Evidence:
- Code readback showed `reality_audit` returned only `in_sync`, `rebase_required`, or `source_unavailable`; `minor_drift` and `major_drift` existed in the schema but were unreachable for physical drift.
- #616 requires no-drift, source-unavailable, minor-vs-major boundary, and rebase guidance.
- Exa lookup on drift-classification practice supported comparing concrete changed attributes and letting the maximum severity decide the verdict.

Outcome:
- Added audit drift analysis helpers, source-unavailable diagnostics detection, field-level severity classification, and focused regressions for in-sync, minor/major, source unavailable, and forced assumption mismatch.
- Focused `reality_audit_` test run passed; broader checks and manual MCP FSV are next.

# 2026-06-01T10:12:14-05:00 - #616 major drift should be a same-profile state divergence

Decision: Prove the major-drift boundary with an out-of-band UI-structure change inside a controlled `powershell` WinForms target, not by switching to a different foreground profile.

Evidence:
- `reality_audit` selects the profile from the freshly captured observation; a different-profile foreground switch would audit that profile's head or fail profile validation instead of testing the stored `powershell` head against changed physical state.
- The first attempt to launch a second PowerShell foreground was hosted by Windows Terminal, changing the observed profile surface and making it the wrong discriminator for #616.
- A same-profile WinForms target with a known new control directly exercises `uia_element_appeared`, which the patch classifies as `major_drift`.

Outcome:
- Temporary `.runs\616\issue616_major_target.ps1` was used only as FSV setup.
- Physical before/after readback showed child texts changed from `major-baseline-state|AddMajor|CloseTarget` to `major-baseline-state|AddMajor|CloseTarget|major-new-control`.
- Real Inspector `tools/call reality_audit` returned `drift_status=major_drift`, `rebase_required=true`, and persisted `reality/audit/v1/powershell/audit-01780326554678039600-0000000006`.

# 2026-06-01T10:21:23-05:00 - #617 follows #616 in the stress campaign

Decision: Take #617 next after closing #616 because it is the next open P1 storage child under #594.

Evidence:
- `gh issue view 616` read back `state=CLOSED`, `closedAt=2026-06-01T15:20:44Z`.
- Live queue after #616 closure lists #594 plus #595-#604 and #617-#634 open.
- #617 is open, has no prior comments, and requires storage CF saturation/GC eviction evidence.
- Posted #617 START comment at https://github.com/ChrisRoyse/Synapse/issues/617#issuecomment-4593992720.

Outcome:
- Next action is code/test inspection for storage pressure, probe rows, and GC cap behavior before launching a repo-built isolated daemon for manual MCP FSV.

# 2026-06-01T10:53:00-05:00 - #618 follows #617 in the storage campaign

Decision: Take #618 next after closing #617 because it is the next open storage child under #594.

Evidence:
- `gh issue view 617` read back `state=CLOSED`, `closedAt=2026-06-01T15:52:11Z`.
- Live queue after #617 closure lists #594 plus #595-#604 and #618-#634 open.
- #618 is open, has no prior comments, and requires the storage pressure ladder/write-gating path.
- Posted #618 START comment at https://github.com/ChrisRoyse/Synapse/issues/618#issuecomment-4594238857.

Outcome:
- Next action is code/test inspection for storage pressure levels, transition codes, compaction, and write-gating behavior before launching a repo-built isolated daemon for manual MCP FSV.

# 2026-06-01T11:18:00-05:00 - #618 diagnostic writes must expose pressure refusal explicitly

Decision: Route `storage_put_probe_rows` through pressure policy before non-empty diagnostic writes and return explicit `STORAGE_WRITE_FAILED` when the current pressure level refuses the target CF.

Evidence:
- Code readback showed the storage layer's lower-level `put_batch` suppresses pressure-blocked writes and reports success, which would make the diagnostic MCP surface report only zero rows rather than an explicit refusal.
- #618 specifically requires write-gating proof for CFs outside the original diagnostic allowlist and requires a gated write attempt to return an explicit refusal.
- The original diagnostic probe allowlist covered only five CFs, so real MCP FSV could not attempt writes to `CF_OCR_CACHE`, `CF_TELEMETRY`, `CF_MODEL_CACHE`, `CF_PROCESS_HISTORY`, or `CF_REFLEX_AUDIT`.

Outcome:
- Added storage/reflex pressure-permission read APIs and expanded `storage_put_probe_rows` to all 11 CFs.
- Non-empty blocked diagnostic writes now return MCP error `STORAGE_WRITE_FAILED`; empty `rows=0` remains a no-op.
- Manual MCP FSV with repo-built daemon PID `56980` on `127.0.0.1:7846` proved the full Normal/L1/L2/L3/L4/recovery ladder, strict thresholds, compaction counts, L3/L4 write gating, explicit refusal errors, empty/no-op, invalid CF, and recovery write reopen.

# 2026-06-01T11:29:00-05:00 - #619 follows #618 in the storage campaign

Decision: Take #619 next after closing #618 because it is the next open storage child under #594.

Evidence:
- `gh issue view 618` read back `state=CLOSED`, `closedAt=2026-06-01T16:27:18Z`.
- Live queue after #618 closure lists #594 plus #595-#604 and #619-#634 open.
- #619 is open, has no prior comments, and requires storage GC under concurrent writes evidence.
- Posted #619 START comment at https://github.com/ChrisRoyse/Synapse/issues/619#issuecomment-4594506099.

Outcome:
- Next action is code/test inspection for `storage_gc_once`, probe rows, audit retention, and row cap behavior before launching a repo-built isolated daemon for manual MCP FSV.

# 2026-06-01T11:45:17-05:00 - #619 storage GC needs no code patch so far

Decision: Treat #619 as a runtime-proof issue unless final checks expose a code defect.

Evidence:
- Code inspection found existing row-cap GC, probe-row writes, and audit-retention report persistence paths for the required behavior.
- Manual MCP FSV against repo-built daemon PID `69600` on `127.0.0.1:7847` proved concurrent writes, heavy in-flight write + GC, max-age retention, dedupe/run_id report persistence, at-soft boundary, empty CF, invalid params, and below-hard-cap oscillation with separate `storage_inspect` readbacks.
- The daemon log contains expected `MCP_TOOL_INVOCATION`, `STORAGE_CF_HARD_CAP_REACHED`, `STORAGE_CACHE_EVICTIONS_TOTAL_INCREMENTED`, and intentional invalid-param lines, with no unexpected error/panic/corruption lines.

Outcome:
- No product-code change is currently needed for #619.
- Next action is final supporting checks/release build, then #619 RESOLVED comment and closure if checks stay green.

# 2026-06-01T11:54:30-05:00 - #620 follows #619 in the profile campaign

Decision: Take #620 next after closing #619 because it is the next open H-profile child under #594.

Evidence:
- `gh issue view 619` read back `state=CLOSED`, `closedAt=2026-06-01T16:53:51Z`.
- Live queue after #619 closure lists #594 plus #595-#604 and #620-#634 open.
- #620 is open and requires profile activation/keymap/HUD/capture/mode evidence.
- Posted #620 START comment at https://github.com/ChrisRoyse/Synapse/issues/620#issuecomment-4594697356 and labeled it `status:in-progress`, `agent:codex`.

Outcome:
- Next action is code/profile-definition inspection before launching a repo-built isolated daemon for manual MCP FSV.

# 2026-06-01T12:45:00-05:00 - #620 needs non-mutating M1 mode/capture readback

Decision: Patch `profile_activate` to apply full profile runtime config and expose M1 mode/capture in non-mutating health plus observation diagnostics.

Evidence:
- Code readback showed `profile_activate` only called `apply_backend_resolution_for_profile`; it did not update `M1State.perception_mode` or capture config.
- `observe` can legitimately re-resolve foreground profile state, so using `observe` alone after activating a profile whose app is not foreground can mutate/read a different foreground profile.
- #620 requires proof that activation applies keymap/HUD/capture/mode; mode/capture needed a separate physical readback surface.

Outcome:
- Worktree patch adds M1 `active_capture_config`, `observe.diagnostics.capture_config`, and `health.subsystems.perception`.
- `profile_activate` now applies backend + M1 mode/capture.
- Foreground profile resolution now applies mode/capture into the observation input for matching-profile `observe`.
- Supporting tests/checks passed; manual isolated MCP FSV remains next.

# 2026-06-01T13:04:31-05:00 - #620 HUD live-slot evidence has an explained foreground-control gap

Decision: Accept #620 with HUD specs proven from profile SoT and document the live HUD-slot gap, rather than widening #620 into a host cursor/focus repair issue.

Evidence:
- Profile SoTs (`profile_list` and bundled TOML) expose HUD fields for `everquest.live`, `luanti.minetest`, and `minecraft.java`.
- Live Luanti launched from the configured host and process/window title matched the Luanti profile.
- Foreground focus stayed on PowerShell despite Win32 foreground attempts, `Alt+Tab`, and launching from the foreground shell.
- Both isolated and wired MCP `act_click` failed closed with `ACTION_BACKEND_UNAVAILABLE` because `SetPhysicalCursorPos` returned access denied.
- #620 acceptance allows a documented explained gap, and action/capture focus/cursor behavior is covered by separate action/capture stress children.

Outcome:
- #620 evidence records the HUD spec SoT and foreground-control gap explicitly.
- No extra profile-runtime patch is needed for HUD specs; final checks/commit/issue closure are next.

# 2026-06-01T13:16:11-05:00 - #621 follows #620 in the profile/registry campaign

Decision: Take #621 next after closing #620 because it is the next open H-profile/registry child under #594.

Evidence:
- `gh issue view 620` read back `state=CLOSED`, `closedAt=2026-06-01T18:15:30Z`.
- Live queue after #620 closure lists #594 plus #595-#604 and #621-#634 open.
- #621 is open, has no prior comments, and requires profile registry install/search/export/import/rollback/digest/quarantine evidence.
- Posted #621 START comment at https://github.com/ChrisRoyse/Synapse/issues/621#issuecomment-4595287040 and labeled it `status:in-progress`, `agent:codex`.

Outcome:
- Next action is code/schema/test inspection for profile registry storage, manifest digest, export/import, rollback, disable/inspect, and quarantine paths before launching a repo-built isolated daemon for manual MCP FSV.

# 2026-06-01T13:41:30-05:00 - #621 registry scale passes without product-code patch

Decision: Resolve #621 with manual MCP FSV evidence and no product-code patch.

Evidence:
- Isolated repo-built daemon PID `58848` on `127.0.0.1:7849` passed process/socket/auth/health and official Inspector strict `tools/list` with 80 tools.
- Real Inspector `profile_registry_install` with the expected Notepad manifest digest wrote 6 `CF_PROFILES` rows and 1 `CF_KV` head row; separate storage/report/inspect readbacks matched expected package, installed, and head rows.
- Scale behavior held at 600 imported registry rows and `limit=1000`: search returned all 600 synthetic rows and report scanned 606 registry rows.
- Registry export wrote a deterministic 607-row bundle; re-import skipped 607 duplicates; modified same-key import failed closed with `registry_bundle_conflict`.
- Disable rewrote the installed row; second-version install plus rollback rewrote installed Notepad back to the prior package and wrote a rollback audit row; single-version terminal rollback failed closed.
- Poison and >1000-row contribution bundles wrote quarantine contribution rows only, with separate inspect readbacks showing rejected counts and risk flags.
- Invalid edges (`limit=0`, malformed import JSON, contribution export without `profile_id`) failed closed with unchanged storage.
- Supporting checks and release build passed; final release binary SHA256 `08FEC90BE80C37B940AF9549335F901A8DACE52863FDA9F7990049F0A4A94890`.

Outcome:
- #621 is ready for RESOLVED comment and closure.

# 2026-06-01T13:43:30-05:00 - #622 follows #621 in the authoring/quality campaign

Decision: Take #622 next after closing #621 because it is the next open profile/telemetry child under #594.

Evidence:
- `gh issue view 621` read back `state=CLOSED`, `closedAt=2026-06-01T18:42:45Z`.
- Live queue after #621 closure lists #594 plus #595-#604 and #622-#634 open.
- #622 is open and requires authoring generate/list/inspect/accept/reject/export plus profile quality refresh evidence.
- Posted #622 START comment at https://github.com/ChrisRoyse/Synapse/issues/622#issuecomment-4595477096 and labeled it `status:in-progress`, `agent:codex`.

Outcome:
- Next action is implementation/test inspection before launching a repo-built isolated daemon for #622 manual MCP FSV.

# 2026-06-01T14:28:42-05:00 - #622 authoring and quality loop passes without product-code patch

Decision: Resolve #622 with manual MCP FSV evidence and no product-code patch.

Evidence:
- Isolated repo-built daemon PID `59440` on `127.0.0.1:7850` passed process/socket/auth/health and official Inspector strict `tools/list` with 80 tools.
- Real observe/action/replay/reality triggers produced the required evidence rows/files; separate readbacks showed `CF_ACTION_LOG=2`, `CF_OBSERVATIONS=2`, `CF_EVENTS=3`, `CF_KV=2`, and replay SHA256 `61AB2CC29986048235197AA336CCC34B86F9794445683C72223FE53AE6BABC1F`.
- Authoring generate/list/inspect/accept/reject/export paths persisted expected `CF_PROFILES` candidate rows, accepted/rejected states, rejection reason, and exported accepted bundle SHA256 `D2790BD9118B9DB5790C4B56D382EA3872146688AD7057FA59EA23427AF9E37B`.
- Edges failed closed with unchanged SoT: zero evidence, accept already accepted, reject accepted, missing export, `profile_authoring_list limit=0`, malformed candidate id, and over-max `max_audit_rows=10001`.
- 10000-row boundary was exercised with real `storage_put_probe_rows` and `profile_authoring_generate max_audit_rows=10000`; separate storage/inspect readbacks confirmed `CF_ACTION_LOG=10002` and `issue622.max` scanned/relevant 10000 rows.
- `profile_quality_refresh` persisted `profile_quality/v1/issue622.authoring`; separate `profile_registry_report` readbacks confirmed score/sample/scanned/relevant counts, stale expiry behavior, invalid-param failure, and final non-stale restored score.
- Cleanup stopped the isolated daemon and port `7850`; final supporting checks and release build passed with SHA256 `236992450A49D3177C1FCBF1D06F567C30CC54AA5F217C1F0D59BFDBADF23E01`.

Outcome:
- #622 is ready for RESOLVED comment and closure.

# 2026-06-01T14:31:53-05:00 - #623 follows #622 in the audit/replay campaign

Decision: Take #623 next after closing #622 because it is the next open H-profile/telemetry child under #594.

Evidence:
- `gh issue view 622` read back `state=CLOSED`, `closedAt=2026-06-01T19:31:05Z`.
- Live queue after #622 closure lists #594 plus #595-#604 and #623-#634 open.
- #623 is open and requires audit consent, redacted bundle export, and replay capture evidence.
- Posted #623 START comment at https://github.com/ChrisRoyse/Synapse/issues/623#issuecomment-4595820271 and labeled it `status:in-progress`, `agent:codex`.

Outcome:
- Next action is implementation/test inspection before launching a repo-built isolated daemon for #623 manual MCP FSV.

# 2026-06-01T15:03:43-05:00 - #623 audit/replay evidence passes; docs corrected

Decision: Resolve #623 after final supporting checks with documentation corrections only.

Evidence:
- The audit/export runtime behavior matched the existing implementation: consent is a `CF_KV` row, strict redaction is fail-closed, and exported bundle SoTs are physical files with response/file hash parity.
- The docs were stale for `replay_record`: `docs/computergames/05_mcp_tool_surface.md` still described a `verb=start/stop/status` API and `docs/systemspec/13_mcp_tool_reference.md` omitted `observations_skipped`. The runtime and tests use `duration_ms`, `target`, `format`, and `path`, so the docs were corrected.
- Manual MCP FSV captured audit consent/export, redaction marker removal, max row/byte caps, replay `target=both` with both observation and event JSONL records, zero-duration replay, and invalid replay path/target/format edges.
- A second isolated daemon with `SYNAPSE_HTTP_SSE_MANUAL=1` was necessary to publish deterministic known events into the same EventBus used by `replay_record`; the replay trigger itself remained real Inspector MCP `tools/call`, and JSONL file bytes were the verdict.

Outcome:
- Product code did not require a patch. Final supporting checks, state commit, RESOLVED comment, and closure are next.

# 2026-06-01T15:11:30-05:00 - #623 final checks passed

Decision: Commit #623 docs/state after successful supporting checks.

Evidence:
- `cargo fmt --check`, `git diff --check`, `scripts\check_docs.ps1`, `cargo check -p synapse-mcp -j 2`, focused replay-record test, schema-sanitize test, tools-list test, and release build all passed.
- The docs checker initially exposed a real missing `REFLEX_DEBOUNCED` entry in `docs/computergames/06_data_schemas.md`; the docs entry was added and the checker then passed.
- Final release binary readback SHA256 is `498E3164F4B795E0ABD3A9E7E2AE678810D532F84B35E5381456277C13628476`.

Outcome:
- #623 is ready for commit, RESOLVED comment, and closure.

# 2026-06-01T15:16:27-05:00 - #624 follows #623 in the EverQuest full-loop campaign

Decision: Take #624 next after closing #623 because it is the next open EverQuest full-loop child under #594 and follows the completed profile/audit/replay tool chain work.

Evidence:
- `gh issue view 623` read back `state=CLOSED`, `closedAt=2026-06-01T20:13:12Z`.
- Live queue after #623 closure lists #594 plus #595-#604 and #624-#634 open.
- #624 is open and requires live EverQuest perception, chat/log/map, memory, planner, trajectory, episode export, ContextGraph ingest/search, and world-model readback evidence.
- Posted #624 START comment at https://github.com/ChrisRoyse/Synapse/issues/624#issuecomment-4596141027 and labeled it `status:in-progress`, `agent:codex`.

Outcome:
- Next action is implementation/test/host-SoT inspection before launching a repo-built isolated daemon for #624 manual MCP FSV.

# 2026-06-01T15:16:27-05:00 - #615 fanout windows were temporary UIA fixtures, not product UI

Decision: Treat the `Issue615FanoutTarget` windows/buttons as closed #615 FSV fixture residue only; no product issue is indicated by their existence.

Evidence:
- Live process/window readback found no `Issue615` or fanout window.
- Wired Synapse `find` returned no `Issue615FanoutTarget` or `Show80` elements.
- Fixture source `.runs\615\target\issue615_target.ps1` shows the buttons only mutate an in-window `ItemPanel` (`Clear`, `Show4/7/8/80`, `Rename8`, `Mixed8`) or close the form (`Exit`).
- #615 RESOLVED evidence already recorded the real manual SoT readbacks proving those buttons changed item counts/names during FSV and that target PID `79124` was stopped.

Outcome:
- User can ignore those windows if seen again; the agent should close any leaked fixture window after use and continue the active issue queue.

# 2026-06-01T16:02:28-05:00 - #624 EULA/account gate is fail-closed; continue ContextGraph warm ingest

Decision: Keep the EverQuest EULA/account agreement as an operator-only boundary and continue only read-only/safe #624 setup plus storage/bridge evidence until the operator personally responds to the agreement.

Evidence:
- Fresh foreground and MCP observations show EverQuest is visible at the Daybreak account/legal agreement, not in-world gameplay.
- The #624 patch expands the login/account gate to detect EULA, terms/privacy, I Agree, and I Decline without persisting raw legal/account text.
- Isolated repo-built daemon PID `34624` on `127.0.0.1:7853` passed strict Inspector `tools/list` with 80 tools and all #624 tools present.
- Real MCP `act_keymap inventory` and `everquest_loc_probe` were denied with reason `everquest_login_or_account_gate_visible`; separate storage rows and unchanged EQ log bytes prove no gameplay/chat command was sent.
- ContextGraph local prerequisite was repaired enough that strict direct tool-list works and Synapse's bridge reached `store_memory`; the remaining failure was `Embedding models are still loading`, so the next reversible local action is a warm ingest with `no_warm=false` and a longer timeout.

Outcome:
- Do not click agreement/login controls.
- Rerun `everquest_contextgraph_ingest` warm through the real Synapse MCP daemon, then verify storage and ContextGraph file SoTs before search.

# 2026-06-01T16:24:00-05:00 - #624 safe chain complete; in-world path is operator-gated

Decision: Treat #624's in-world happy path as blocked by an exact operator-only account/legal action, while preserving the completed reversible evidence for the EULA guard, ContextGraph bridge, and safe storage/modeling chain.

Evidence:
- Warm wired `everquest_contextgraph_ingest` and `everquest_contextgraph_search` both succeeded through real Synapse MCP. Search row `everquest/contextgraph_search/v1/everquest.live/issue624-synth-search-wired-warm` read back from active `CF_KV` with one citation to fingerprint `d5d91675-9303-4b0f-bdd6-2f0326abffdb` and export SHA256 `7386a7f8b26cd6fc8e262813eff9167785d13610aaf8e68bbd9fcce3949dc2ef`.
- ContextGraph storage directory changed after search with new SST/log/manifest files; `LOG` SHA256 readback is `FF68150590233C0E101CAD5D071EEC8AD08A81061429B7F95429CD85A9FAB72E`.
- Active Synapse safe-chain tools persisted current state, map sensor, outcome rows, hazard/safe memories, planner consult, planner guard, route plan, world-model transition, and world summary; final `CF_KV=33` and direct DB-byte search found the expected `issue624-*` keys.
- Physical EQ log SoT remained length `2464677`, SHA256 `E563074084A7F5A291AC6FBF77746B993AB086F747C6C111C39503B6BF475368`; physical map SoT `nektulos.txt` line `5974` contains `To_Neriak`.
- Edges failed closed: account/EULA gate action denial, non-EverQuest foreground, visible unsent chat text, structurally invalid planner source ref, absent valid-shaped EQ log path, and EverQuest reality audit profile mismatch.

Outcome:
- Final supporting checks and cleanup passed; release binary SHA256 `31D62B2891F4AA17F7139BF4A5E52276521F7009E7B2C428D6FAFF15CBF5A374`.
- Post #624 BLOCKED evidence: the remaining action is for the operator to personally review/respond to the Daybreak EULA/account agreement and put the character in-world; the agent must not click legal/account/login controls.

# 2026-06-01T16:31:30-05:00 - #625 follows #624 after operator-gated block

Decision: Take #625 next because #624 is blocked only on an operator-owned Daybreak account/legal action and the broader open-issue queue still has reversible work.

Evidence:
- #624 readback shows `status:blocked` and evidence comment https://github.com/ChrisRoyse/Synapse/issues/624#issuecomment-4596661903.
- `git status --short --branch` read clean after pushing commit `9de5ee3`.
- Live open queue still includes #625 and later issues.
- #625 requires EverQuest readiness/autocombat/predictive/surprise/action-prior evidence.
- Posted #625 START comment at https://github.com/ChrisRoyse/Synapse/issues/625#issuecomment-4596668371 and labeled it `status:in-progress`, `agent:codex`.

Outcome:
- Inspect #625 implementations and complete all safe/reversible evidence before deciding whether live autocombat is blocked by the same operator-only EULA/account action.

# 2026-06-01T16:56:00-05:00 - #625 reversible evidence complete; live soak is operator-gated

Decision: Mark #625 blocked after completing all reversible safe readiness/model/surprise/action-prior evidence, because the remaining sustained live EverQuest autocombat soak depends on an operator-only Daybreak EULA/account/login/in-world action.

Evidence:
- Wired Synapse MCP remained usable after compaction: `health ok=true`, active profile `vscode`, storage initialized, process readback found `synapse-mcp.exe` PID `66040` plus stdio child PID `70072`, and the real tools were called through the configured MCP client.
- `everquest_survival_readiness` persisted blockers for non-EverQuest foreground, gameplay UI not proven, unsafe chat input, missing HUD HP/mana, and missing food/drink.
- `everquest_autocombat issue625-autocombat-deny-vscode` failed closed with `ACTION_TARGET_INVALID active_profile_mismatch`; `CF_ACTION_LOG` advanced and recorded the denial; the EQ log stayed unchanged.
- Synthetic DynamicJEPA/trajectory/model chain persisted rows through `everquest_domain_normalize`, `everquest_trajectory_record`, `everquest_predictive_model_fit`, and `everquest_predictive_model_predict`; model hash `286c033af9422dc870e43302c96cf5380c60122fcf7b29122bbcd29ea9b0427c`.
- Surprise rows covered confirmed outcome, mismatch, missing prediction, and structurally invalid source-ref failure; exact rows and payload hashes were read back from `CF_KV`.
- Action-prior rows covered correct, low-confidence, and abstain samples; scorecard row `everquest/action_prior_scorecard/v1/everquest.live/issue625-scorecard-window` advanced `CF_KV 47 -> 48` and read back `low_confidence_action_forced`.
- Duplicate scorecard sample IDs failed closed with `TOOL_PARAMS_INVALID`; `CF_KV` stayed `48` and no invalid row was found.
- Physical EQ log stayed length `2464677`, SHA256 `E563074084A7F5A291AC6FBF77746B993AB086F747C6C111C39503B6BF475368`.
- Supporting checks passed: fmt, scorecard/predictive/surprise tests, schema sanitize, M4 tools-list, MCP check, release build, and diff check. Release binary SHA256 `4AF3EB0E332F6A7AFD5DBBFAD1169EB051371040D5C24CF033662AC3615F78AD`.

Outcome:
- Posted #625 BLOCKED evidence at https://github.com/ChrisRoyse/Synapse/issues/625#issuecomment-4596839011 and label readback shows `status:blocked`.
- Remaining action is for the operator to personally handle the Daybreak EULA/account/login/character in-world state; the agent must not click legal/account/login/character-select/chat controls.

# 2026-06-01T17:00:00-05:00 - #626 follows #625 as next numbered showcase child

Decision: Take #626 next after blocking #625, because #626 is the next unblocked child in the current numbered #594 campaign sequence and has reversible local/audio/browser work available.

Evidence:
- #625 readback shows `status:blocked` and evidence comment https://github.com/ChrisRoyse/Synapse/issues/625#issuecomment-4596839011.
- `git status --short --branch` read clean after pushing commit `0c854e8`.
- Live queue still contains #626 and later showcase/stress issues plus earlier #595-#604 children.
- #626 requires real `act_launch`, `act_combo`, `audio_tail`, and `observe` SoT evidence.
- Posted #626 START comment at https://github.com/ChrisRoyse/Synapse/issues/626#issuecomment-4596846733 and labeled it `status:in-progress`, `agent:codex`.

Outcome:
- Inspect #626 audio/action/browser paths, then launch an audio-enabled repo-built Synapse MCP runtime for manual FSV.

# 2026-06-01T17:45:00-05:00 - #626 resolved by local Chrome piano evidence

Decision: Resolve #626 with no product-code patch because real MCP evidence proved `act_combo`, browser action, visual readback, and audio loopback behavior for the autonomous pianist scenario.

Evidence:
- Isolated repo-built audio-enabled daemon PID `79620` on `127.0.0.1:7854` passed auth health with loopback running and strict Inspector `tools/list` with 80 tools and all #626 tools present.
- Local Chrome piano target was launched by real MCP `act_launch`; after real `act_click` Arm, OCR showed `Audio: armed`, `Focus: yes`, and zero counters.
- Happy-path `act_combo` scheduled 15 Ode-to-Joy steps; OCR showed 15 audio notes, 15 play count, zero wrong/muted notes, and the expected melody.
- Overlapped 48-step playback plus `audio_tail` read returned nonzero 48 kHz stereo `s16le` PCM: `peak=5809`, `rms_db=-33.3`, with active 50 ms buckets from about 1.75s to 4.9s; OCR showed 48 audio notes.
- Edges passed: empty steps rejected, non-monotonic steps rejected, muted four-note run produced zero PCM and four muted visual notes, wrong-key `x` recovered with C4, back-to-back combos produced the expected six-note melody, and the wired production MCP client accepted the 256-step boundary with storage/reflex readback active->expired.
- Cleanup stopped all #626-owned local processes/ports and both release_all paths returned zero held input.
- Supporting checks passed: fmt, audio-tail test, M4 tools-list test, schema_sanitize, synapse-mcp check, release build, and diff check. Release binary SHA256 `FC4003D69AA84712112DEBC3534F113B15F89E69046E23D4064D01CFFAECBE4F`.

Outcome:
- Post RESOLVED evidence to #626, close it, refresh queue, and continue.

# 2026-06-01T17:50:00-05:00 - #627 follows #626 as next showcase child

Decision: Take #627 next after closing #626, because #627 is the next open unblocked numbered showcase child in the #594 campaign and has reversible local Office work available.

Evidence:
- #626 readback shows `state=CLOSED`, closed at `2026-06-01T22:44:50Z`, with RESOLVED evidence comment https://github.com/ChrisRoyse/Synapse/issues/626#issuecomment-4597095341.
- `git status --short --branch` read clean after pushing commit `9382bd2`.
- Live open queue still contains #627 and later showcase/stress issues plus #595-#604, while #624/#625 remain blocked on the operator-only Daybreak boundary.
- #627 requires real MCP `act_launch`, action entry, visual readback, and saved `.xlsx` file SoT evidence.
- Excel is installed locally at `C:\Program Files\Microsoft Office\root\Office16\EXCEL.EXE`; App Paths registry entries point to it.
- Posted #627 START comment at https://github.com/ChrisRoyse/Synapse/issues/627#issuecomment-4597099075 and labeled it `status:in-progress`, `agent:codex`.

Outcome:
- Inspect Excel/action/file verification surfaces and run #627 manual FSV through real Synapse MCP tools.

# 2026-06-01T18:35:41-05:00 - #615 fanout fixture concern rechecked during #627

Decision: Treat any `Issue615FanoutTarget` window as stale closed-issue fixture residue, not a product UI or active blocker, unless a new live process/window readback proves otherwise.

Evidence:
- OS top-level-window enumeration found no visible `Issue615`, `Fanout`, `Show80`, `Rename8`, or `Mixed8` windows alive.
- `.runs\615\target\issue615_target.ps1` directly defines the button handlers: `Show4/7/8/80` repopulate `ItemPanel`, `Clear` clears it, `Rename8` renames current items, `Mixed8` renames/adds items, and `Exit` closes the form.
- Wired Synapse `find` currently fails on Excel with `cached RuntimeId had unexpected type EMPTY`, which is the active #627 defect already patched locally and still under manual verification.

Outcome:
- User was told the windows are old #615 WinForms UIA stress fixtures; if one appears again, close the leaked fixture and continue #627.

# 2026-06-01T18:59:20-05:00 - #627 saved workbook SoT readback completed

Decision: Accept the #627 Excel workbook behavior as manually verified through the saved `.xlsx` file SoT, after using real MCP tools to complete the save dialog and then independently reading workbook package bytes.

Evidence:
- Post-compaction isolated daemon readback: PID `34556`, bind `127.0.0.1:7855`, repo release binary SHA256 `24757F067CBDBE4E5871BDCAB44DF735A47C1788CD53E126D4680B358032B245`.
- Strict Inspector `tools/list` and `health` succeeded after compaction with all #627 tools present.
- Classic `Save As` field and Save button were discovered through real MCP `find`, filled through `act_click`/`act_press`/`act_clipboard`, and accepted through `act_click`.
- File SoT before save was absent; after save the `.xlsx` exists at `.runs\627\excel-runtime-check-20260601T1810\issue627-self-driving-spreadsheet.xlsx`, length `22526`, SHA256 `D3F696164FE3835A1E7C12C9E7F58821CBC08D52FDB64D7C9553340108AD567E`.
- Workbook package readback shows sheet dimension `A1:M257`, expected formula cached values `36/27/16/20/26/33/79`, `G2` formula `1/0` with `#DIV/0!`, 256 bulk rows through `J257:M257`, and chart/drawing relationships to `xl/charts/chart1.xml`.

Outcome:
- Run final supporting checks and cleanup, then post #627 RESOLVED evidence and close the issue.

# 2026-06-01T19:10:00-05:00 - #627 final checks and cleanup passed

Decision: #627 is ready for RESOLVED posting; the RuntimeId/re-resolution patch, Excel manual FSV, cleanup, and local supporting checks are complete.

Evidence:
- Real Inspector `release_all` returned zero held keys/buttons/pads, real Inspector Alt+F4 closed the saved Excel workbook, and process readback found Excel PID `78020` absent.
- Isolated daemon PID `34556` was stopped and port `127.0.0.1:7855` read back closed.
- Supporting checks passed: `cargo fmt --check`, `cargo check -p synapse-a11y -j 2`, `cargo check -p synapse-mcp -j 2`, schema sanitize test, M4 tools-list test, release build, and `git diff --check` with line-ending warnings only.
- Final release binary SHA256 is `3FF17F523F900368D486863AA5EED573F8D3616DF2FE87E998330026D5557462`, length `46396416`.

Outcome:
- Post #627 RESOLVED evidence, close #627, commit/push with `[skip ci]`, then continue the open queue.

# 2026-06-01T19:13:00-05:00 - #628 follows #627 as next showcase child

Decision: Take #628 next after closing #627, because #628 is the next open unblocked numbered showcase child in the #594 campaign and has reversible local Chrome/Playwright work available.

Evidence:
- #627 readback shows `state=CLOSED`, closed at `2026-06-02T00:11:22Z`, with RESOLVED evidence comment https://github.com/ChrisRoyse/Synapse/issues/627#issuecomment-4597519110.
- `git status --short --branch` read clean after pushing commit `c3b83b2`.
- Live open queue still contains #628 and later showcase/stress issues plus #595-#604, while #624/#625 remain blocked on the operator-only Daybreak boundary.
- #628 requires Synapse real MCP browser action/perception triggers plus Playwright as independent DOM oracle.
- Posted #628 START comment at https://github.com/ChrisRoyse/Synapse/issues/628#issuecomment-4597523219 and labeled it `status:in-progress`, `agent:codex`.

Outcome:
- Inspect browser/action/Playwright surfaces and prepare deterministic local #628 FSV target.

# 2026-06-01T21:16:00-05:00 - #628 physical cursor failure requires fail-closed readback plus HWND fallback

Decision: Keep the #628 action fix focused on truthful physical cursor readback and a narrow Windows element-click fallback. The normal coordinate actor path remains first choice; the HWND message fallback is attempted only when the action backend reports `ACTION_BACKEND_UNAVAILABLE`.

Evidence:
- Real MCP/OS readbacks showed `SetPhysicalCursorPos`, `SetCursorPos`, and Synapse `act_aim` could not move the cursor from the current foreground/input context; cursor coordinates stayed unchanged despite API calls.
- Playwright DOM readbacks showed Chrome UIA `Invoke` returned success while the late-loaded button state remained unchanged, so invoke success alone is not a verdict.
- DPI readbacks on the Chrome monitor showed scaled coordinate mismatches, so `synapse-action` now reads `GetPhysicalCursorPos` after moves and emits an explicit backend-unavailable error on mismatch.
- A diagnostic-only `PostMessageW` to Chrome's renderer HWND changed Playwright DOM state, proving the target window can accept mouse messages when physical cursor movement is unavailable.
- The latest patch passed fmt, typechecks, focused coordinate/DPI/readback regressions, fmt-check, and release build. Release binary SHA256 is `42FB209D71E8D2F6967D0F82D1B6A6EE70422B98361489ADCCDCD14F3F4258D1`.

Outcome:
- Start a fresh isolated #628 daemon with the latest binary, verify strict MCP client parity, then run real MCP FSV. Diagnostic direct Win32 calls remain investigation evidence only.

# 2026-06-01T23:00:34-05:00 - #628 targeted scroll seed-HWND fix accepted for the scroll sub-behavior

Decision: Accept the targeted `act_scroll.at` HWND-message path as manually verified for the #628 moved-element/scroll sub-behavior, while continuing #628 because typing, click, submit, and edges still need full SoT coverage.

Evidence:
- Fresh repo-built isolated daemon PID `34424`, bind `127.0.0.1:7862`, binary SHA256 `971EAE444FE3E72FA533C7B7FBAA41A97824A5D149C7E263F6D9FB2BBD0FC301`.
- Fresh strict Inspector `tools/list` loaded after compaction and required #628 tools were present.
- Before SoT: Playwright DOM `scrollY=0`; isolated storage `CF_ACTION_LOG=0`.
- Setup readback proved the target point was on Chrome target root `0x12068a` after raising the target window.
- Trigger: real Synapse MCP `tools/call act_scroll dy=-20 at={"x":856,"y":696}`.
- After SoT: Playwright DOM `scrollY=1278`; moved target rect shifted upward; isolated storage `CF_ACTION_LOG=2`; daemon stderr logged `M2_ACT_SCROLL_HWND_MESSAGE` to `Chrome_RenderWidgetHostHWND` at `856,696`.

Outcome:
- Continue #628 with `act_type into_element` exactness before the full browser marathon happy path and edges.

# 2026-06-01T23:06:31-05:00 - #628 browser typing exactness accepted only with external DOM SoT

Decision: Continue #628 with the `act_type into_element` path for browser form fields, but record that the accepted verdict is the independent Playwright DOM readback, not the UIA immediate readback.

Evidence:
- Fresh repo-built isolated daemon PID `34424`, bind `127.0.0.1:7862`, and fresh strict Inspector `tools/list` artifact `325_patched12_tools_list_post_compaction2.txt`.
- Before SoT: Playwright DOM `searchValue=""`, `activeId=""`, and isolated storage `CF_ACTION_LOG=2`.
- Trigger: real Synapse MCP `tools/call act_type text=vega into_element=<search input found under HWND 1181322>`.
- After SoT: Playwright DOM `searchValue="vega"`, `searchLength=4`, `activeId="searchInput"`; isolated storage `CF_ACTION_LOG=4`.
- Daemon log warns `M2_ACT_TYPE_ELEMENT_VALUE_PATTERN_READBACK_MISMATCH` with UIA `after_len=0`, so the UIA readback is not used as the success verdict for Chrome DOM typing.

Outcome:
- Run the full #628 browser happy path and required edges with Playwright/server/storage SoT reads after each real MCP trigger.

# 2026-06-02T00:00:05-05:00 - #628 browser marathon accepted after manual MCP/SoT evidence

Decision: Accept #628 as ready for RESOLVED posting because the real Synapse MCP browser action path now has happy-path and edge evidence against independent Playwright DOM, local server state, and isolated storage SoTs.

Evidence:
- Isolated repo-built daemon PID `34424`, bind `127.0.0.1:7862`, and strict Inspector `tools/list` artifacts `431`/`432` proved the required MCP tools were present before acceptance.
- `act_scroll.at` through real MCP moved Playwright DOM `scrollY` from `0` to `1278`, advanced isolated `CF_ACTION_LOG`, and logged `M2_ACT_SCROLL_HWND_MESSAGE`.
- `act_type into_element` through real MCP wrote exact browser DOM values. UIA immediate readback can mismatch for Chrome, so the tool now reports target readback required and Playwright DOM/server state remains the verdict.
- Happy path completed search, late-loaded control, modal, iframe, form fill, moved target, and submit. Playwright and server SoTs show receipt `M-1` with the expected synthetic payload and `movedClicks=1`; isolated storage reached `CF_ACTION_LOG=38`.
- Edges passed: empty search produced `empty-search` without server mutation, 256-character search preserved the exact boundary value with zero results, and structurally invalid element id failed closed with `ACTION_ELEMENT_NOT_RESOLVED` while DOM/server state stayed unchanged.
- Final supporting checks and release build passed; final `synapse-mcp.exe` SHA256 is `710ADCF581389D984ED613A7DE3034A623055825A8D743B7368CF1F3F6268530`.
- Cleanup released inputs, stopped the isolated daemon and all #628-owned server/Playwright/target Chrome processes, closed ports `8763`/`8932`/`9226`, and preserved unrelated Chrome PID `30964`.

Outcome:
- Commit/push the scoped #628 patch with `[skip ci]`, post RESOLVED evidence, close #628, then refresh the queue.

# 2026-06-02T00:03:13-05:00 - #595 follows #628 as next unblocked child

Decision: Take #595 next after closing #628 because #594 remains the parent context, #624/#625 are still blocked on the Daybreak operator-only boundary, and #595 is the lowest-number open unblocked child in the whole-body stress campaign.

Evidence:
- #628 readback shows `state=CLOSED`, `closedAt=2026-06-02T05:02:28Z`, evidence comment https://github.com/ChrisRoyse/Synapse/issues/628#issuecomment-4598863144, and no remaining `status:in-progress` label.
- `git log -5` shows `4991efe fix(mcp): harden browser element actions (#628) [skip ci]` at `HEAD`, `origin/main`, and `origin/HEAD`.
- `git status --short --branch` after #628 close showed `## main...origin/main` with only unrelated `README.md` dirty.
- Live open queue contains #594, #595-#604, blocked #624/#625, and #629-#634.
- #595 has no prior comments, is open/unassigned, and requires real MCP `observe`/`find`/`observe_delta` triggers plus separate high-fanout UIA/UI/storage/log SoT readbacks.
- Posted #595 START comment at https://github.com/ChrisRoyse/Synapse/issues/595#issuecomment-4598866903 and labeled/assigned it to this agent.

Outcome:
- Begin #595 implementation/investigation with code inspection and deterministic high-fanout local target setup.

# 2026-06-02T00:18:00-05:00 - #595 enforces internal UIA snapshot collection budget

Decision: Fix the #595 boundedness bug at the Windows UIA snapshot collection boundary, not only at the final observe/find response truncation.

Evidence:
- Code inspection showed observe/find/reality response caps already clamp output, but Windows collect_nodes could still collect every sibling from a large flat child enumeration because the old guard only stopped descent.
- Microsoft UIA documentation warns broad descendant searches can traverse thousands of items, so Synapse must enforce its own node/deadline budget.
- Patch in crates/synapse-a11y/src/platform/windows/snapshot.rs adds collection_limit_reason and checks budget/deadline before node collection, before child enumeration, and before recursing into remaining siblings.
- Focused helper tests, synapse-a11y check, synapse-mcp check, and release build passed before this state entry.

Outcome:
- Continue with real MCP manual FSV against an issue-local 10k-item WinForms target and isolated repo-built daemon.

# 2026-06-02T00:36:51-05:00 - #595 must bound UIA child enumeration before bulk materialization

Decision: Replace normal Windows UIA snapshot child collection with streaming raw-tree-walker enumeration, while preserving bulk child enumeration only for known UWP app-frame/CoreWindow nodes.

Evidence:
- Manual #595 run against a 10k WPF UIA target proved the first budget patch was insufficient: real MCP `observe` and `find` took ~26-27s, and daemon logs showed truncation only after `FindAllBuildCache(TreeScope::Children)` had bulk-materialized the large child array.
- Microsoft UIA docs warn broad tree searches can traverse thousands of elements; caching APIs prefetch in bulk, so Synapse must enforce the budget before each sibling, not after a bulk return.
- The patch streams normal child enumeration through `UITreeWalker` sibling calls, checks budget/deadline before each child, and restricts the old bulk path to `ApplicationFrameWindow`, `Windows.UI.Core.CoreWindow`, and `ApplicationFrameInputSinkWindow` to preserve #582.
- The Notepad raw menu supplement is now gated to Notepad roots so it cannot scan arbitrary high-fanout trees.
- Focused helper tests, a11y check, mcp check, and release build passed; release SHA256 is `9F7663082D2A417E44B053AD95C79B590B50B0409BFCCE421FF1C616196757E7`.

Outcome:
- Redo #595 manual MCP FSV on a fresh isolated daemon; reject any result that still shows bulk 10k enumeration latency or misses the known target SoT without an explicit, understood reason.

# 2026-06-02T01:05:39-05:00 - #595 fanout fix accepted on isolated repo-built MCP runtime

Decision: Accept the #595 Windows UIA fanout fix for commit/RESOLVED posting based on the repo-built isolated daemon, strict Inspector client parity, and separate target/storage/log/UIA readbacks.

Evidence:
- Isolated daemon PID `64060`, bind `127.0.0.1:7864`, unauth/auth health, and strict Inspector `tools/list` with 80 tools proved the repo-built runtime and required tools were present before behavior acceptance.
- Deterministic target PID `62812` exposed 10,000 UIA text children with known names/automation ids/bboxes from its state file and independent UIA reads.
- Real Inspector `tools/call observe depth=6 max_elements=500` returned 184 elements, advanced isolated `CF_EVENTS/CF_OBSERVATIONS` from 0 to 1, and daemon logs showed `A11Y_SNAPSHOT_WALK_TRUNCATED reason="deadline"` with ~403ms snapshot elapsed rather than the prior 26s bulk enumeration.
- Real Inspector `tools/call find query="Issue595 Item 00042"` returned the exact visible item name/automation id/bbox matching independent UIA.
- `reality_baseline` persisted baseline/head rows; after target rename, `observe_delta profile_id=powershell` persisted eight `CF_KV/reality/delta/*` rows.
- Edges passed or were explicitly characterized: `max_elements=1`, no-result find, depth-0 and max-elements-0 boundaries, unknown-param structural invalid with storage unchanged, minimized-window find by HWND, and Calculator/UWP CoreWindow fallback smoke.
- Supporting checks passed after the final helper-test edit; release binary SHA256 is `C5415C7A2153613FC5C9BC654C3ADB99A939F83D7BC2A6FA9F7CF206A41DC57A`.
- Cleanup released inputs, stopped target/CalculatorApp/isolated daemon, and verified port `7864` closed.

Outcome:
- Commit the scoped code/state changes with `[skip ci]`, post #595 RESOLVED evidence, close #595, then continue with the next unblocked issue.

# 2026-06-02T01:09:41-05:00 - #596 follows #595 as next unblocked child

Decision: Take #596 after closing #595 because #596 is now the lowest-number open unblocked child under the #594 stress campaign; #624/#625 remain blocked on the operator-only Daybreak boundary.

Evidence:
- #595 closure readback shows `state=CLOSED`, `closedAt=2026-06-02T06:08:59Z`, RESOLVED evidence comment `4599193156`, and no `status:in-progress` label.
- `git log -1` shows `098e8d5 fix(a11y): stream UIA fanout snapshots (#595) [skip ci]` at `HEAD`, `origin/main`, and `origin/HEAD`.
- `git status --short --branch` after #595 close shows only unrelated `README.md` dirty.
- Live open queue shows #594 parent, #596-#604, blocked #624/#625, and #629-#634.
- #596 body requires real MCP `set_capture_target`/`observe` triggers plus separate physical capture/window/monitor/DPI SoT readbacks.
- Posted #596 START comment at https://github.com/ChrisRoyse/Synapse/issues/596#issuecomment-4599199311 and labeled/assigned it to this agent.

Outcome:
- Begin #596 with code inspection of capture-target, fallback, and DPI paths before any implementation or FSV setup.

# 2026-06-02T01:31:06-05:00 - #596 set_capture_target must drive the real capture controller

Decision: Fix #596 by wiring M1 `set_capture_target` to `synapse-capture::CaptureController` and exposing runtime capture readback in `set_capture_target`, `health`, and `observe`, rather than accepting config-only metadata updates.

Evidence:
- Code inspection showed `set_capture_target_in_state` only called `resolve_capture_target`, incremented a generation counter, and updated `active_capture_config`.
- `M1State` did not own a `CaptureController`, so no real capture thread, 2-frame channel, frame/drop counters, thread priority, or effective backend could be observed through the MCP tool.
- `resolve_capture_target` accepted arbitrary monitor indices; failures would occur only inside the background capture path.
- `element_window` only parsed the HWND from the element id; a disappeared element could still be accepted if the window lived.
- Exa/Microsoft docs confirm Windows Graphics Capture and DXGI Desktop Duplication are frame-producing APIs, DXGI duplication is output/monitor oriented, and Per-Monitor V2 DPI awareness exposes raw pixels for DPI-sensitive comparisons.

Outcome:
- Patch M1/capture/core/perception to provide real controller switching and runtime readbacks.
- Keep manual FSV acceptance on a fresh repo-built MCP daemon; supporting tests/checks remain regression evidence only.

# 2026-06-02T02:27:37-05:00 - #596 WGC shutdown must use an external stop control

Decision: Replace Synapse's WGC `GraphicsHandler::start(settings)` path with `start_free_threaded(settings)` plus `CaptureControl::stop()` for Windows Graphics Capture target switches.

Evidence:
- The #596 isolated daemon PID `30420` timed out during `set_capture_target target=element_window`; a separate `/health` read also timed out while PID `30420` still owned `127.0.0.1:7865`.
- Code inspection showed `CaptureController::switch_to` joined the previous capture thread synchronously while M1 held its state mutex.
- The local `windows-capture` crate shows `start(settings)` runs a message loop on the current thread and only lets user code call `InternalCaptureControl::stop()` from `on_frame_arrived`.
- For quiet/static WGC window targets, another frame callback may not arrive after Synapse sets its stop flag, so the join can block indefinitely.
- The same crate's `start_free_threaded()` returns `CaptureControl`, whose `stop()` sets the halt flag, posts `WM_QUIT` to the capture thread, and joins the message-loop thread.
- Focused supporting checks after the patch passed for capture target switching, DXGI-window rejection, capture thread priority, and capture/MCP crate typechecking.

Outcome:
- Use the crate-provided external WGC stop mechanism and record the actual callback thread priority.
- Restart #596 FSV from a fresh daemon; the prior run remains defect evidence, not acceptance evidence.

# 2026-06-02T03:04:07-05:00 - #596 element_window requires a non-empty live UIA rectangle

Decision: Fail closed when an `element_window` target re-resolves to an empty/non-positive UIA bounding rectangle, instead of accepting the owning HWND.

Evidence:
- The post-WGC #596 manual run hid the deterministic edge button and a separate `find` read still returned the old element id with bbox `{x=0,y=0,w=0,h=0}`.
- The real `set_capture_target target=element_window` call accepted that stale-looking element and switched capture to the window, leaving capture generation advanced and violating the disappeared-element edge.
- Code inspection showed `capture_target_from_param(ElementWindow)` used `element_bounding_rect` only as a liveness check and discarded the returned rectangle.
- Microsoft UIA documentation says BoundingRectangle uses physical screen coordinates and defaults to Empty / is NULL when an item is not currently displaying UI, so zero/non-positive bounds are not a valid capture target surface.
- The patch validates `w > 0 && h > 0` before converting the element id to its HWND and returns `CAPTURE_TARGET_INVALID` for empty bounds.
- Focused supporting checks, capture regressions, schema/tool-list checks, touched-crate check, and release build passed; release SHA256 is `DE9BEFF453DD5A1C45035A3F5836C6453DC1D5E824B6B2A06F9DCD9C286FAA22`.

Outcome:
- Rerun #596 manual MCP FSV from a fresh daemon on the patched release binary; prior hidden-element acceptance remains defect evidence, not acceptance evidence.

# 2026-06-02T03:21:12-05:00 - #596 capture target fix accepted after manual MCP FSV

Decision: Accept the #596 capture-target implementation and hidden-element hardening for commit/RESOLVED posting.

Evidence:
- Repo-built main daemon PID `47680` on `127.0.0.1:7867` and forced-DXGI daemon PID `23940` on `127.0.0.1:7868` both passed process/socket/auth health and strict Inspector `tools/list` with 80 tools.
- Physical Win32 SoT readbacks showed primary DISPLAY2 `5120x2160` at 150% DPI, secondary monitors `2560x1440`, and WPF target DWM frame `1332x801` at `GetDpiForWindow=144`.
- Real Inspector `set_capture_target` calls plus separate health/observe readbacks proved primary min-floor `5120x2160`, monitor0 `5120x2160`, window `1332x801`, visible `element_window` `1332x801`, and quiet-window-to-monitor1 switch `2560x1440`.
- Edge calls failed closed and left capture state unchanged for invalid monitor, structural invalid target kind, hidden element bbox `0x0`, and closed HWND.
- Forced-DXGI run proved monitor0 `dxgi_duplication` `5120x2160` and rejected live window HWND with `DXGI duplication supports monitor targets only`.
- Storage readbacks showed expected observation/event rows in isolated DBs; `release_all` returned zero held state; cleanup verified daemons, ports, and target absent.

Outcome:
- Commit scoped patch/state with `[skip ci]`, post #596 RESOLVED evidence, close #596, then continue the queue.

# 2026-06-02T03:26:47-05:00 - #597 follows #596 as next unblocked child

Decision: Take #597 after closing #596 because #597 is the lowest-number open unblocked child under the #594 stress campaign; #624/#625 remain blocked on the Daybreak operator-only boundary.

Evidence:
- #596 closure readback shows `state=CLOSED`, `closedAt=2026-06-02T08:25:48Z`, evidence comment https://github.com/ChrisRoyse/Synapse/issues/596#issuecomment-4600229991, and no remaining `status:in-progress` label.
- `git log -3` shows `6051fb3 fix(mcp): reject empty element capture targets (#596) [skip ci]` at `HEAD`, `origin/main`, and `origin/HEAD`.
- `git status --short --branch` after #596 close showed `## main...origin/main`.
- Live open queue contains #594, #597-#604, blocked #624/#625, and #629-#634.
- #597 has no prior comments, was open/unassigned, and requires real MCP `read_text` triggers plus separate source text/UI/storage/cache/log SoT readbacks.
- Posted #597 START comment at https://github.com/ChrisRoyse/Synapse/issues/597#issuecomment-4600240790 and labeled/assigned it to this agent.

Outcome:
- Begin #597 implementation/investigation with code inspection of OCR backend and cache paths before any FSV setup.

# 2026-06-02T03:54:52-05:00 - #597 post-compaction resume reconciled to dirty OCR/cache patch

Decision: Continue #597 from the existing OCR/cache patch and rerun isolated runtime setup from a fresh daemon/run directory, rather than treating the first failed daemon launch as FSV evidence.

Evidence:
- Required wake-up files, #351, #594, #597, live queue, and git status/log/branch were reread.
- Git shows `HEAD == origin/main == ce6f048` with dirty #597-owned OCR/MCP/docs files.
- Wired configured Synapse MCP client still loads `health`, `storage_inspect`, `observe`, and `find`; configured `CF_OCR_CACHE` row count is `0`.
- The first #597 isolated launch attempt under `.runs\597\ocr-fsv-20260602T035217` recorded PID `77292` already exited and did not provide reliable stdout/stderr capture.
- Supporting checks and release build were already run before compaction; the release binary hash is `9C9F0D85D60E5E7E3ED014E7755193EA434BBCEAABA0B71051038372BB3A6AC0`.

Outcome:
- Record current state, then inspect diff/run artifacts and launch a fresh repo-built HTTP daemon for real Inspector `tools/list` and `tools/call read_text` manual SoT FSV.

# 2026-06-02T04:19:35-05:00 - #597 OCR backend/cache fix accepted after manual MCP FSV

Decision: Accept the #597 `read_text` backend/cache implementation for commit and issue closeout.

Evidence:
- Fresh repo-built daemon PID `42136` on `127.0.0.1:7870` passed process/socket/auth health and strict Inspector `tools/list` with 80 tools.
- Deterministic target PID `3408` / `Issue597OcrTarget` had source text bytes read from `target\issue597_source_text.json` and physical UIA bboxes read through isolated `observe`.
- Dense WinRT OCR wrote `CF_OCR_CACHE` row `0->1`, repeat same pixels stayed `1` and daemon log recorded `OCR_CACHE_HIT`; row readback includes requested/effective backend, lang, region, bitmap SHA256, result, latency, and word count.
- `backend=auto` wrote a separate `auto/winrt` row; `backend=crnn` failed closed with `OCR_BACKEND_UNAVAILABLE` and no cache growth.
- Tiny and multilingual regions both OCRed through `winrt` and `auto`; cache grew to `6`.
- Rapid changed pixels did not return stale cached BETA after switching to ALPHA; occluded element-id read returned the overlay text and wrote a different bitmap-hash row.
- Zero-size, off-screen, and structurally invalid region edges all failed closed and left cache count unchanged.
- Focused-window fallback wrote the final full-window row; final isolated `CF_OCR_CACHE=10`.
- Cleanup verified `release_all` zero, target absent, daemon absent, and port `7870` closed.
- Final supporting checks and release build passed; final release SHA256 is `11C259BD288FC5C71B50CCB6AA025826BD40428E842E93A4D93D4A351B20F674`.

Outcome:
- Commit/push with `[skip ci]`, post #597 RESOLVED evidence, close #597, refresh the queue, then continue to the next unblocked issue.

# 2026-06-02T04:22:34-05:00 - #598 follows #597 as next unblocked child

Decision: Take #598 after closing #597 because it is the lowest-number open unblocked child under #594; #624/#625 remain blocked on the Daybreak operator-only boundary.

Evidence:
- #597 readback shows `state=CLOSED`, `closedAt=2026-06-02T09:21:57Z`, RESOLVED comment https://github.com/ChrisRoyse/Synapse/issues/597#issuecomment-4600960919, and no remaining `status:in-progress` label.
- `git log -5` shows `d64c6a2` at `HEAD`, `origin/main`, and `origin/HEAD`; `git status --short --branch` read `## main...origin/main`.
- Live open queue contains #594, #598-#604, blocked #624/#625, and #629-#634.
- #598 was open/unassigned before claim and requires real MCP detection/entity tracking FSV.
- Posted #598 START comment at https://github.com/ChrisRoyse/Synapse/issues/598#issuecomment-4600966276 and labeled/assigned it to this agent.

Outcome:
- Begin #598 with code inspection of detection/entity tracking and runtime/model setup before any FSV setup.

# 2026-06-02T04:46:11-05:00 - #598 root cause requires real detector invocation, not fixture entities

Decision: Patch M1 to invoke the real RT-DETR runtime in `pixel_only`/`hybrid` and patch `synapse-models` to decode the pinned ONNX model output, rather than accepting synthetic entities or an empty inference batch.

Evidence:
- Existing M1 source collection only performed capture probing and synthetic Luanti entities; no detector session was called for normal `pixel_only`/`hybrid` observations.
- `LoadedModel::infer` validated frames but returned an empty `DetectionBatch`, so a present model file still produced no entities.
- The pinned local model exists at `C:\Users\hotra\AppData\Local\synapse\models\rtdetr_v2_s_coco.onnx` with SHA256 `583A236AC21C95A7FD94F284FC21485E42355BFEF82C27011BA78FBC09EE87E2`, matching the registry descriptor.
- Local ONNX Runtime probing showed the model's `logits` and normalized `pred_boxes` decode correctly into COCO cats/remote/sofa detections using sigmoid class scores and normalized cx/cy/w/h boxes.
- `set_perception_mode pixel_only/hybrid` was being overwritten by profile runtime application before `observe`, so explicit manual mode needed to persist until `auto` clears it.

Outcome:
- Continue with the local #598 patch, update docs, run final checks/release build, then accept only after isolated repo-built MCP Inspector FSV with separate physical SoT readbacks.

# 2026-06-02T04:56:41-05:00 - #598 profile detection config must be honored

Decision: Honor `ProfileDetection.classes_of_interest` in the live M1 detection path and update the systemspec to describe current detector behavior.

Evidence:
- The profile schema exposes `classes_of_interest`, `confidence_threshold`, and `max_detections` as detection controls.
- The #598 patch already applied threshold and max cap, but leaving `classes_of_interest` unused would make the profile readback misleading once live detection is wired.
- Systemspec files still claimed `M1State` did not invoke detectors and `entities` were only synthetic, which contradicted the patch.

Outcome:
- Added case-insensitive class filtering for non-empty `classes_of_interest`.
- Updated systemspec source map, M1 perception section, model-loader section, and aggregate systemspec.
- Supporting checks and release build passed; continue to isolated manual MCP/SoT FSV.

# 2026-06-02T05:12:31-05:00 - #598 tracker stale window must tolerate strict-client cadence

Decision: Increase detection track stale retention from 1500 ms to 3000 ms.

Evidence:
- Post-patch isolated daemon PID `63716` passed strict Inspector preconditions.
- After copying the verified RT-DETR model into isolated `LOCALAPPDATA`, still pixel-only observe returned healthy COCO detections.
- Two moving observations through the strict Inspector arrived about 1.7 seconds apart; the old 1500 ms prune window deleted active tracks before the second observation.
- The visible object persisted and moved across the target state SoT, but cats reacquired new track IDs and had no `velocity_px_per_s`, failing #598's stable-track requirement.

Outcome:
- `STALE_TRACK_MS` is now 3000 ms; docs updated.
- Rebuild release binary SHA256 `F8B15ED79B3A5D4D1FF9CE2522189341614589D73348410C4F330A982E170264`.
- Rerun isolated manual FSV from a fresh post-patch daemon.

# 2026-06-02T05:28:20-05:00 - #598 detector/tracker behavior accepted by manual MCP/SoT FSV

Decision: Accept the #598 implementation and runtime behavior after main and cap-edge isolated repo-built MCP runs.

Evidence:
- Main run `.runs\598\detection-fsv-20260602T0513`: repo-built daemon PID `28444`, bind `127.0.0.1:7872`, release SHA256 `F8B15ED79B3A5D4D1FF9CE2522189341614589D73348410C4F330A982E170264`, strict Inspector `tools/list=80`, initial storage all zeroes.
- Pixel-only still frame returned healthy detections for two cats and remote; storage persisted `CF_OBSERVATIONS=1`, `CF_EVENTS=1`; screenshot readback visually matched reported bboxes.
- Moving target kept cat track IDs `4` and `5` across consecutive strict Inspector observes and produced non-null velocity vectors.
- Hybrid mode returned healthy detection with cats/sofa/remote; `find cat` returned two entity results with matching visible bboxes.
- Edges covered: black frame empty/healthy, leave/re-enter reacquired new track IDs after the 3000 ms stale window, default confidence floor all >= 0.5, and missing `mode` failed closed with unchanged storage counts.
- Cap run `.runs\598\detection-cap-fsv-20260602T0523`: issue-local profile `issue598.cap` set `max_detections=2`; health showed it active; same large grid that produced 10 detections under default profile returned exactly 2 and persisted that observation.
- Cleanup: `release_all` zero on both daemons; ports `7872`/`7873` closed; target window absent.

Outcome:
- Proceed to final supporting checks, diff review, commit, RESOLVED evidence, and #598 closeout.

# 2026-06-02T05:36:10-05:00 - #598 final checks and diff review complete

Decision: Proceed to commit and close #598.

Evidence:
- Final supporting checks passed: fmt, diff check, focused M1/model tests, touched-crate check, schema sanitize, strict tool-list regression, and release build.
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46708736`, SHA256 `32968BB49188230EC41C2DAD5822B6B4E2A9405522DC3D4501719FBA0BEADCE6`, timestamp `2026-06-02T10:35:24.2191556Z`.
- Diff review completed across `synapse-models`, `synapse-mcp` M1/server wiring, systemspec docs, and state notes.

Outcome:
- Commit with `[skip ci]`, push, post #598 RESOLVED evidence, close #598, refresh queue.

# 2026-06-02T05:49:18-05:00 - #599 audio tool contract must support the issue stress window directly

Decision: Patch the M3 audio tool contract before FSV instead of treating #599 as a pure verification issue.

Evidence:
- #599 requires `audio_tail` over `0.1..=30` second windows and names RMS/VAD/events/azimuth as acceptance values.
- Current code exposed `seconds: u32` and `MAX_RING_SECONDS = 5`, so a strict MCP client could not call the 100 ms window and the runtime could not satisfy the 30 s boundary.
- Current `audio_tail` returned PCM only; compact audio metadata was reachable through `observe include audio`, but not through the named tool surface.
- Host Whisper prerequisite exists and matched the pinned SHA256; missing model is not the current blocker.

Outcome:
- Extend the ring/tool max to 30 seconds.
- Accept numeric seconds at the MCP boundary.
- Add compact metadata to `audio_tail` while preserving raw PCM as non-persisted tool response data.
- Continue with focused checks, release build, and real loopback manual MCP/SoT FSV.

# 2026-06-02T06:58:40-05:00 - #599 audio behavior accepted after post-telemetry manual MCP/SoT FSV

Decision: Accept the #599 implementation after the post-telemetry isolated daemon run and cleanup.

Evidence:
- Accepted daemon `.runs\599\audio-fsv-20260602T0647-accepted`: PID `76024`, bind `127.0.0.1:7877`, release SHA256 `A5B88B6B1048EB64AB9A7E8CEB77979D8FB4EF26112964F3DCB27F634DDBEC09`, strict Inspector `tools/list=80`.
- `audio_tail` covered zero, 100ms, silence, left/center/right panning, speech+music, overlapping speech, loud transient, and 30s boundary using known WAV fixture hashes and real loopback playback.
- `audio_transcribe` exact English happy path was the 6s rerun: `Hello world. This is Synapse.` with confidence `0.86`; the first 5s partial result was rejected.
- Edges failed closed: unsupported non-English language, nonnumeric seconds, seconds over 30, and audio-disabled daemon.
- Final storage readbacks stayed all zero; product log/DB byte scans found no transcript, `CallToolResult`, response payload, or `pcm` persistence. Host audio mutes restored to original unmuted state.

Outcome:
- Proceed to final supporting checks, diff review, commit, RESOLVED comment, close #599, then continue the live queue.

# 2026-06-02T07:20:00-05:00 - #600 starts after #599 closure

Decision: Claim #600 as the next unblocked #594 child after closing #599.

Evidence:
- #599 readback showed `state=CLOSED`, `closedAt=2026-06-02T12:17:50Z`, and no `status:in-progress` label.
- `git status --short --branch` read `## main...origin/main`.
- Live open queue shows #600 as the next unblocked child before #601-#604 and #629-#634; #624/#625 remain blocked.
- #600 body requires real MCP action flood FSV for `ACTION_RATE_LIMITED`, `ACTION_QUEUE_FULL`, release safety exemption, and physical/action-log readbacks.

Outcome:
- Posted #600 START comment and labeled/assigned the issue.
- Inspect action limiter/queue code next.

# 2026-06-02T07:30:35-05:00 - #600 requires real MCP execute-path queue hardening

Decision: Patch the action handle/emitter before runtime FSV because the low-level queue tests did not cover the real MCP enqueue path.

Evidence:
- #600 requires real `act_press`/`act_click` MCP floods to surface `ACTION_QUEUE_FULL` once the 256-deep action queue is saturated.
- `ActionHandle::try_execute` already returned `ACTION_QUEUE_FULL`, but MCP tools call `ActionHandle::execute`.
- The old `execute` used async `mpsc::Sender::send`, which waits for bounded-channel capacity instead of failing closed.
- `ReleaseAll` and panic/operator safety release used the same saturated normal queue; `KeyUp` was rate-limit exempt but not queue-priority exempt.
- Tokio docs confirm bounded `send` waits for channel capacity while `try_send` returns immediately when the buffer is full; `select!` with `biased` polling can enforce an explicit priority order.

Outcome:
- Normal `execute` now uses non-blocking queue insertion and returns `ACTION_QUEUE_FULL`.
- Emitter-backed handles carry a priority safety lane for `ReleaseAll`/`KeyUp`.
- The actor polls safety/auto-release/snapshot/shutdown before normal actions and rejects pending normal actions after `ReleaseAll`.
- Focused `synapse-action` compile and queue/preemption regressions passed; broader MCP/runtime FSV remains next.

# 2026-06-02T07:55:36-05:00 - #600 release_all must interrupt active software holds

Decision: Have `ReleaseAll` enqueue request the release-interrupt epoch before it waits for actor execution.

Evidence:
- The #600 queue fix made `ReleaseAll` bypass the saturated normal queue, but the actor can still be inside a blocking backend call.
- Software key/mouse hold sleeps poll `operator_release_requested_since(epoch)`, but only the operator hotkey advanced that epoch.
- MCP `release_all` and panic-hook `fire_release_all_blocking_with_timeout` therefore had a priority lane but did not wake the current long hold.
- Focused handle test readback showed `execute(Action::ReleaseAll)` now changes the epoch before actor ack, and the safety-lane preemption regression still passes.

Outcome:
- Added `synapse_action::request_release_interrupt()`.
- `ActionHandle::execute(Action::ReleaseAll)` and blocking release fire now call it before enqueueing.
- Rebuild and fresh isolated MCP FSV are required because the old #600 daemon predates this patch.

# 2026-06-02T09:40:00-05:00 - #600 accepted with queue-flood fix and documented rate-limit gap

Decision: Accept the #600 implementation after the final isolated repo-built daemon run, with the token-bucket runtime evidence recorded as a documented host-throughput gap rather than a forced claim.

Evidence:
- Final daemon `.runs\600\action-flood-fsv-20260602T0904-final`: PID `40028`, bind `127.0.0.1:7880`, release SHA256 `786FF6F6B62AC564F8F0C7A1DC20E8226A6720AB44A6EB6B75064EC8E88081C2`, strict Inspector `tools/list=80`.
- Real MCP happy path wrote exactly `def` to the Notepad file SoT and separately read `CF_ACTION_LOG=10`.
- Queue flood produced `ACTION_QUEUE_FULL=317`, `SAFETY_RELEASE_ALL_FIRED=256`, log readbacks for both codes, and no stuck OS input after release/cleanup.
- MCP release_all interrupted active software holds: middle-button hold elapsed `1549 ms` and Shift hold elapsed `1766 ms` rather than the requested `30000 ms`.
- Empty, invalid-structure, exact 256, just-under 255, ViGEm happy, and KeyUp/release safety edges all had before/after file/storage/OS/process readbacks.
- The real public MCP paths on this host did not reach `ACTION_RATE_LIMITED`: software flood attempts hit `ACTION_QUEUE_FULL`/backend validation first, while ViGEm calls completed below `1000/s`; supporting token-bucket tests passed exact software/ViGEm overshoot cases.
- Cleanup readbacks showed release_all zero, daemon stopped, port closed, Notepad closed, and file unchanged.

Outcome:
- Proceed to tracked diff/token scan, `[skip ci]` commit, push, #600 RESOLVED comment/closeout, then refresh queue for #601.

# 2026-06-02T09:45:00-05:00 - #601 starts after #600 closure

Decision: Claim #601 as the next unblocked #594 child after closing #600.

Evidence:
- #600 readback showed `state=CLOSED`, `closedAt=2026-06-02T14:39:18Z`, and stale labels removed.
- `git status --short --branch` read `## main...origin/main`.
- Live open queue shows #601 as the next unblocked child before #602-#604 and #629-#634; #624/#625 remain blocked.
- #601 requires real MCP `act_combo` timing FSV for monotonic <=256-step sequences, jitter readback, and fail-closed edge validation.

Outcome:
- Posted #601 START comment and labeled/assigned the issue.
- Inspect combo/reflex scheduling code next.

# 2026-06-02T10:05:00-05:00 - #601 needs persisted combo timing readback

Decision: Patch combo completion auditing before runtime FSV so #601 can verify requested vs actual timing from persistent storage.

Evidence:
- `act_combo` already lowered only `act_press` steps and rejected unsupported actions, empty lists, `>256` steps, non-monotonic `at_ms`, and backend mismatches.
- The combo controller emitted transient `reflex_combo_completed` event data with dispatches, but the persistent `CF_REFLEX_AUDIT` completion row only contained generic lifetime completion fields.
- #601 requires step dispatch times vs requested timing as a separate SoT read; relying on a tool return or transient stream would be weaker than a persisted audit row.

Outcome:
- Persist nested combo completion details in `CF_REFLEX_AUDIT` with due/elapsed/jitter/action details per primitive dispatch.
- Add focused validation regressions for #601 fail-closed edges.
- Focused combo/reflex checks pass; broader checks and manual MCP/SoT FSV remain next.

# 2026-06-02T10:25:00-05:00 - #601 runtime FSV accepted

Decision: Accept #601 runtime behavior after a second precise 256-step run proved timing readback without queue saturation, and after fail-closed edges left physical/storage SoTs unchanged.

Evidence:
- Repo-built release SHA256 `669191BA58F581763DB6B389979EF6545ADC458B6AAA9BDEF72DB516FCC51B6D` was exercised through strict MCP Inspector against isolated daemons on `127.0.0.1:7881` and `127.0.0.1:7882`; both loaded `tools/list=80`.
- The first 256-step stress run wrote exactly 256 `a` characters and persisted `512/512` primitive dispatches; its 5 ms cadence showed `max_jitter_ms=205`, which is recorded as saturation evidence rather than the precision proof.
- The accepted precision run used 256 steps at 20 ms cadence and wrote exactly 256 `b` characters; `reflex_history` persisted `scheduled_actions=512`, `dispatched_actions=512`, `elapsed_ms=5105`, `max_jitter_ms=0`, with first and last due/elapsed pairs matching exactly.
- Single-step combo wrote exactly `z` and persisted `scheduled_actions=2`, `dispatched_actions=2`, `max_jitter_ms=0`.
- Structurally invalid `steps` object, empty steps, non-monotonic steps, 257-step boundary, and unsupported `act_click` step all rejected through strict Inspector/MCP and left the target file plus `CF_ACTION_LOG`/`CF_REFLEX_AUDIT` counts unchanged.
- Cleanup readback showed release_all zero, no relevant OS key/button down, no `Issue601*` windows, and issue-local ports closed.

Outcome:
- Runtime FSV complete for #601. Final supporting checks, diff/token scan, commit, push, RESOLVED comment, and closeout remain.

# 2026-06-02T10:35:00-05:00 - #601 final checks passed

Decision: Proceed to #601 commit/closeout.

Evidence:
- Final checks passed: `cargo fmt --check`, `git diff --check`, combo behavior tests, MCP combo tests, schema sanitizer tests, M4 tool-list test, reflex-history tool test, touched-crate check, and release build.
- Release build readback: `target\release\synapse-mcp.exe`, length `46748160`, SHA256 `F7C089061FE2CF23B5FBEC9D7A12C55FD19A7C38117CEA637A7CA0B02F4919D5`, timestamp `2026-06-02T15:28:21Z`.
- Tracked diff token scan found zero matches for the issue-local bearer token, raw auth header text, or bearer-token env var name.
- Diff review found the code changes scoped to combo completion audit persistence and fail-closed validation coverage.

Outcome:
- Commit with `[skip ci]`, push, post #601 RESOLVED evidence, close #601, remove stale labels, refresh queue, and continue to #602.

# 2026-06-02T10:40:00-05:00 - #602 starts after #601 closure

Decision: Claim #602 as the next unblocked #594 child after closing #601.

Evidence:
- #601 GitHub readback showed `state=CLOSED`, `closedAt=2026-06-02T15:31:27Z`, and stale claim labels removed.
- `git status --short --branch` read `## main...origin/main`.
- Live open queue shows #602 as the next unblocked child before #603/#604 and #629-#634; #624/#625 remain blocked.
- #602 requires real MCP `act_drag` FSV for Paint drawing, Explorer/file drag-drop, 4096px boundary, and fail-closed/physical edge cases.

Outcome:
- Posted #602 START comment and labeled/assigned the issue.
- Inspect drag implementation next.

# 2026-06-02T11:45:00-05:00 - #602 act_drag patch and FSV accepted

Decision: Patch `act_drag` rather than only run the existing surface, because #602 exposed two real capability gaps: MCP schema lacked `bezier` despite the core `AimCurve::Bezier`, and `act_drag` had no direct modifier-held path.

Evidence:
- Strict Inspector `tools/list` before patch showed `act_drag` loaded but only exposed `natural,instant,linear,ease_in_out`.
- Recording tests now show all five curves, including `bezier`, and modifier order `key_down:shift>down:left>mouse_move...>up:left>key_up:shift`.
- Live Ctrl-modified Explorer drag initially moved instead of copied, proving key release happened too soon for Explorer drop processing; after adding a 200 ms post-drop modifier settle, repeated Ctrl-drag physically copied with source+dest hashes both `F6209DEE7E9B5C536DA81B06D7FEFC76E71FB47E1D993DC956B0CD6C6F26E6C0`.
- Manual FSV run `.runs\602\drag-fsv-20260602T1105-clean` also accepted Paint curve strokes, Explorer move, exact 4096px cross-monitor boundary, 4097px fail-closed, zero-length, non-drop, empty params, structurally invalid params, storage/input/file SoT readbacks, and cleanup.
- Final strict tools-list on PID `61420` showed `act_drag` curve enum `natural,instant,linear,ease_in_out,bezier` and modifier enum `ctrl,shift,alt,super`.

Outcome:
- Final release binary SHA256 `8432EEC297778C356BF0B006EABE9D0FA3AA94A6F0ADFF93BC4A3452A1D66826`.
- Final supporting checks passed; only `git diff --check` CRLF warning remains.
- Next: token scan, commit/push with `[skip ci]`, post #602 RESOLVED evidence, close #602, refresh queue for #603.

# 2026-06-02T11:42:49-05:00 - #603 starts after #602 closure

Decision: Claim #603 as the next unblocked #594 child after verifying #602 closed and `main` is clean.

Evidence:
- #602 GitHub readback showed `state=CLOSED`, `closedAt=2026-06-02T16:40:00Z`, RESOLVED evidence comment, and stale claim labels removed.
- `git status --short --branch` read `## main...origin/main`; `HEAD` and `origin/main` are `f0f8dc9`.
- Live open queue shows #603 as the next unblocked child before #604 and #629-#634; #624/#625 remain blocked.
- #603 requires real MCP `act_pad` FSV for ViGEm X360 + DS4 buttons, sticks, triggers, controller observation, concurrent controllers, lifecycle/rate-limit/fail-closed edges, and cleanup neutralization.

Outcome:
- Posted #603 START comment and labeled/assigned the issue.
- Inspect `act_pad`/ViGEm implementation next.

# 2026-06-02T11:48:40-05:00 - #603 patch exposes gamepad guide button

Decision: Patch the MCP `act_pad` public surface before runtime FSV because it could not drive the full button set.

Evidence:
- Core `PadButton` includes `Guide`, and `synapse-action` ViGEm conversion already maps it for both controllers (`x360` raw `0x0400`, DS4 `special=0x01`).
- `ActPadButton` exposed only face/shoulder/stick/dpad/start/back buttons, so strict `tools/list` clients could not call `act_pad` with `guide`.
- Focused checks now show the `ActPadButton` schema includes `guide`, JSON `"guide"` maps to core `PadButton::Guide`, and recording-backend X360/DS4 full reports carry `Guide` plus sticks/triggers and return to neutral.

Outcome:
- `crates/synapse-mcp/src/m2/pad.rs` patched.
- Broader checks and real MCP/SoT FSV remain next.

# 2026-06-02T12:55:00-05:00 - #603 runtime FSV accepted with Luanti gap

Decision: Accept #603 controller-surface FSV for X360/DS4 through strict Inspector and document Luanti as an explained external-game gap rather than a Synapse pad failure.

Evidence:
- Isolated repo-built daemon PID `35556` on `127.0.0.1:7884` passed auth health and strict Inspector `tools/list=80`; `act_pad` schema included `guide`.
- Real MCP `act_pad` calls drove X360 and DS4 full sweeps. Separate XInput/browser/PnP readbacks proved buttons, guide, sticks, triggers, dpad directions, concurrent controllers, lifecycle churn, and neutral return.
- Edge readbacks covered empty/neutral, max hold, over-max fail-closed, structurally invalid params, storage unchanged on invalid, and final cleanup neutral state.
- Luanti was launched from the installed runtime against a run-local copied world with joystick settings enabled and a probe mod reading `get_player_control()` plus `players.sqlite`. XInput showed held virtual reports, but Luanti logged no controls and persisted no position/yaw delta for X360 id0 or DS4 id1.
- Public manual strict-Inspector path cannot produce 1000 calls/sec without an automated harness; supporting rate-limit regression remains non-FSV evidence and the limitation will be stated on the issue.

Outcome:
- Cleanup completed: `release_all` zero, XInput neutral, `CF_ACTION_LOG=65`, daemon stopped, port `7884` closed, no Luanti process remains.
- Final supporting checks passed, release binary SHA256 is `68F9285C1860CF55FA291861D94C31E122EF80CF32303B1A73F425011B47ADD6`, and tracked diff token scan found no matches.
- Commit and GitHub closeout are next.
