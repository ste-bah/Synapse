# CURRENT STATE - Synapse

## 2026-06-02T13:48:15-05:00
- Active issue remains #604.
- First isolated FSV daemon/run:
  - run dir `.runs\604\clipboard-fsv-20260602T1328`;
  - daemon PID `9388`, bind `127.0.0.1:7885`, release SHA256 before owner-window fix `007518B8CC722C9E610CADD3CB5B000DEED37E8815E9FA5138A799AAE055700A`;
  - auth health OK, unauth `/health=401`, strict Inspector `tools/list=80` with `act_clipboard`.
- Accepted in the first run:
  - Unicode `act_clipboard write/read` matched separate `Get-Clipboard` UTF-8 hash `AA146817EBD41E7D1325E8C946521DD2A112837FB6F2E3C02542720973F33C3C`;
  - `CF_ACTION_LOG` moved to `4`; storage samples showed `act_clipboard` start/ok rows and redacted response metadata (`text_present`, no raw `text` field).
- Rejected in the first run:
  - CF_TEXT ASCII write returned `ok=true`, but separate Win32 `GetClipboardData(CF_TEXT)` did not match the expected ASCII bytes and MCP `act_clipboard read format=text` returned empty.
  - Root cause: Windows write path used `OpenClipboard(None)` before `EmptyClipboard`; Microsoft documents that if the clipboard owner is `NULL`, `EmptyClipboard` sets owner to `NULL` and later `SetClipboardData` fails/notifies no valid owner. Return value alone was therefore not accepted.
- Additional patch:
  - `crates/synapse-action/src/clipboard.rs` now creates a temporary hidden message-only owner window for write/clear, passes that HWND to `OpenClipboard`, keeps it alive through `CloseClipboard`, destroys it afterward, and verifies `IsClipboardFormatAvailable(format)` immediately after `SetClipboardData`.
- Supporting checks passed after owner-window patch:
  - `cargo fmt`;
  - `cargo check -p synapse-action -j 2`;
  - focused action/MCP clipboard tests.
- New patched release build:
  - `target\release\synapse-mcp.exe`, length `46848512`, SHA256 `3BB80539A49DF75CF6B17DD89D574778DEEE295AC7EB8C005E65D234302F63C5`, `LastWriteTimeUtc=2026-06-02T18:48:07.5196515Z`.
- Cleanup/readback:
  - pre-fix PID `9388` stopped;
  - port `7885` closed.
- Current next:
  1. Launch a clean post-fix isolated daemon, strict Inspector `tools/list`.
  2. Redo #604 manual FSV from clean SoTs, especially CF_TEXT raw bytes and Notepad paste/file bytes.

## 2026-06-02T13:19:31-05:00
- Active issue remains #604 `scenario(stress): act_clipboard round-trip - Text/Unicode, large, non-ASCII reject, contention`.
- Code inspection found two real gaps before runtime FSV:
  - `act_clipboard` success bypassed normal action start/result audit rows, so `CF_ACTION_LOG` could miss successful clipboard side effects.
  - MCP prevalidation rejected non-ASCII `format=text` as `TOOL_PARAMS_INVALID`, while #604 expects the backend limitation to surface as `ACTION_BACKEND_UNAVAILABLE`.
- Patch applied:
  - `crates/synapse-mcp/src/server/m2_tools.rs`: `act_clipboard` now records redacted action audit start/result rows for all calls and includes only verb/format/text length/text-present metadata, never raw clipboard text.
  - `crates/synapse-mcp/src/server/action_audit.rs`: added explicit ok/error audit helpers for redacted/custom details.
  - `crates/synapse-mcp/src/m2/clipboard.rs`: removed non-ASCII CF_TEXT MCP prevalidation so the action backend owns the backend-unavailable failure.
  - `crates/synapse-action/src/clipboard.rs`: supporting regression asserts non-ASCII CF_TEXT fails as `ACTION_BACKEND_UNAVAILABLE`.
  - `crates/synapse-mcp/src/server/context.rs`: supporting regression asserts `act_clipboard` writes two redacted `CF_ACTION_LOG` rows.
- Focused supporting checks passed:
  - `cargo fmt`;
  - `cargo test -p synapse-action cf_text_non_ascii_fails_as_backend_unavailable_before_platform_open -- --nocapture`;
  - `cargo test -p synapse-mcp text_format_non_ascii_reaches_backend_validation -- --nocapture`;
  - `cargo test -p synapse-mcp act_clipboard_records_redacted_action_audit_rows -- --nocapture`.
- Current next:
  1. Run broader supporting checks and release build.
  2. Launch an isolated repo-built daemon for #604.
  3. Perform manual MCP/SoT FSV for Unicode, CF_TEXT ASCII, large payload, clear/empty, CF_TEXT non-ASCII rejection, contention retry, boundary and structurally invalid params, Notepad paste/file bytes, `CF_ACTION_LOG` rows, and cleanup.

## 2026-06-02T13:07:00-05:00
- #603 is closed:
  - commit `6d3c148 fix(mcp): expose gamepad guide button (#603) [skip ci]`;
  - RESOLVED evidence https://github.com/ChrisRoyse/Synapse/issues/603#issuecomment-4605620033;
  - closure readback `state=CLOSED`, `closedAt=2026-06-02T18:06:13Z`;
  - stale `status:in-progress` and `agent:codex` labels removed.
- Git state after #603 close:
  - branch `main`;
  - `git status --short --branch` read `## main...origin/main`;
  - latest commit `6d3c148`.
- Live open queue after #603:
  - #594 parent remains open;
  - #624/#625 remain `status:blocked` on the Daybreak/operator boundary;
  - unblocked children currently open include #604 and #629-#634.
- Active issue is #604 `scenario(stress): act_clipboard round-trip - Text/Unicode, large, non-ASCII reject, contention`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/604#issuecomment-4605626077.
  - Labels/assignee updated with `status:in-progress`, `agent:codex`, and `ChrisRoyse`.
  - Issue goal: prove `act_clipboard` read/write round trips for CF_TEXT and CF_UNICODETEXT, large payload integrity, clear/empty behavior, non-ASCII rejection for CF_TEXT, and contention retry behavior.
  - Planned SoTs: repo-built daemon process/socket/auth/health/strict Inspector `tools/list`; real `act_clipboard` calls; separate Windows clipboard reads; Notepad paste/file-byte readbacks; action/storage audit rows; contention holder process/window state; cleanup clipboard/release state.
- Current next:
  1. Inspect `act_clipboard` MCP params, schema, action backend, Windows clipboard code, size/encoding limits, contention retry, audit logging, and tests.
  2. Patch only if code or FSV exposes a real gap.
  3. Build/launch isolated repo-built daemon and perform manual MCP/SoT FSV for #604.

## 2026-06-02T12:55:00-05:00
- Active issue remains #603 `scenario(stress): ViGEm gamepad full sweep - X360 + DS4 buttons/sticks/triggers`.
- Implementation patch remains scoped to `crates/synapse-mcp/src/m2/pad.rs`:
  - exposes `guide` in `ActPadButton`;
  - maps it to core/backend `PadButton::Guide`;
  - focused tests cover schema exposure, JSON mapping, and full X360/DS4 recording-backend readbacks.
- Manual MCP/SoT evidence is captured under `.runs\603\pad-fsv-20260602T1205`.
  - Repo-built daemon PID `35556`, bind `127.0.0.1:7884`, auth health OK, unauth health `401`, strict Inspector `tools/list=80`; `act_pad` schema included `guide`.
  - ViGEmBus SoT was running via service/PnP/registry readback.
  - X360 full sweep accepted: strict Inspector `act_pad` full report with `a,b,x,y,lb,rb,ls,rs,back,start,up,down,left,right,guide`, sticks/triggers, `hold_ms=30000`; XInput during read `buttons=0xf3ff`, `lt=255`, `rt=128`, `lx=-32768`, `ly=32767`, `rx=32767`, `ry=-32768`; browser Gamepad API showed Xbox buttons including guide pressed; after read neutral.
  - DS4 full sweep accepted: strict Inspector `act_pad` full report, browser Gamepad API showed DS4 face/shoulder/stick/system buttons and guide; PnP DS4 device present; DS4 trigger-only supplement showed buttons 6/7 trigger values `1` and `0.5019608`.
  - Dpad individual sweep accepted for X360 and DS4: each direction had the expected XInput/browser button readback because all-at-once contradictory dpad directions cancel.
  - Concurrent controllers accepted: overlapping X360 A and DS4 B strict calls read back simultaneously on XInput/browser surfaces and returned neutral.
  - Rapid lifecycle accepted: pad_id 2 x360->ds4->x360 sequence succeeded and final browser/PnP readback was neutral.
  - Edge cases accepted: empty/neutral report; max `hold_ms=30000`; over-max `hold_ms=60000` failed closed with `ACTION_HOLD_EXCEEDED_MAX`; structurally invalid `buttons="a"` failed closed and left `CF_ACTION_LOG=48`, XInput neutral.
  - Storage/action audit moved from `CF_ACTION_LOG=6` before the main sweep to `48`, then to `65` after Luanti attempts and cleanup.
  - Luanti real-game attempt was exercised and documented as an explained gap: copied run-local world, enabled joystick settings, added run-local probe mod, and tried X360 left stick/A/dpad/right stick plus DS4 left stick. XInput readbacks proved the virtual reports during holds, Luanti probe logged `enable_joysticks=true` with the requested joystick ids/types, but `get_player_control()` never logged inputs and `players.sqlite` position/yaw stayed unchanged.
  - Public strict Inspector/manual MCP path cannot physically generate 1000 `act_pad` calls/sec without an automated harness; supporting `rate_limit_overshoot` regression remains the rate-limit evidence and the gap will be documented on #603.
  - Cleanup complete: strict Inspector `release_all` returned zero held keys/buttons/pads; XInput slots 0/1 neutral; final `storage_inspect` read `CF_ACTION_LOG=65`; daemon PID `35556` stopped; port `7884` closed; no Luanti process remains.
- Final supporting checks passed:
  - `cargo fmt --check`;
  - `git diff --check` (CRLF warnings only);
  - `cargo test -p synapse-mcp --bin synapse-mcp act_pad_ -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp recording_backend_readback_carries_full_x360_and_ds4_reports -- --nocapture`;
  - `cargo test -p synapse-action backend::vigem::tests --lib -- --nocapture`;
  - `cargo test -p synapse-core gamepad_report_schema_has_closed_object_and_axis_bounds --test action_types -- --nocapture`;
  - `cargo test -p synapse-action --test rate_limit_overshoot vigem_1100_events_limits_exactly_100 -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`;
  - `cargo test -p synapse-mcp --test m3_tools_list -- --nocapture`;
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`;
  - `cargo check -p synapse-action -p synapse-mcp -j 2`;
  - `cargo build --release -p synapse-mcp -j 2`.
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46800896`, SHA256 `68F9285C1860CF55FA291861D94C31E122EF80CF32303B1A73F425011B47ADD6`, `LastWriteTimeUtc=2026-06-02T18:03:05.1344129Z`.
- Tracked diff token scan found no matches for bearer/auth/token markers; diff review completed.
- Current next:
  1. Commit with `[skip ci]`, push, post #603 RESOLVED evidence, close #603, remove stale labels.
  2. Refresh queue and continue #604.

## 2026-06-02T11:48:40-05:00
- Active issue remains #603 `scenario(stress): ViGEm gamepad full sweep - X360 + DS4 buttons/sticks/triggers`.
- Code inspection found one real public-surface gap:
  - core `PadButton` and ViGEm report conversion already supported `Guide`;
  - X360 maps `Guide` to raw `0x0400`; DS4 maps `Guide` to `special=0x01`;
  - MCP `ActPadButton` did not expose `guide`, so `act_pad` could not drive every controller button from the real tool surface.
- Patch applied in `crates/synapse-mcp/src/m2/pad.rs`:
  - adds `ActPadButton::Guide`;
  - maps it to `PadButton::Guide`;
  - adds support checks for schema exposure, JSON mapping to core `Guide`, and recording-backend full X360/DS4 reports carrying `Guide`, all face/shoulder/stick/dpad buttons, stick extremes, and triggers before returning to neutral.
- Focused supporting checks passed:
  - `cargo fmt`;
  - `cargo test -p synapse-mcp --bin synapse-mcp act_pad_ -- --nocapture` (schema + JSON guide mapping);
  - `cargo test -p synapse-mcp --bin synapse-mcp recording_backend_readback_carries_full_x360_and_ds4_reports -- --nocapture`;
  - `cargo test -p synapse-action backend::vigem::tests --lib -- --nocapture`;
  - `cargo test -p synapse-core gamepad_report_schema_has_closed_object_and_axis_bounds --test action_types -- --nocapture`;
  - `cargo test -p synapse-action --test rate_limit_overshoot vigem_1100_events_limits_exactly_100 -- --nocapture`;
  - `git diff --check` (CRLF warning only).
- Current next:
  1. Run broader schema/tool-list/touched-crate checks and release build.
  2. Launch isolated repo-built daemon and perform #603 manual MCP/SoT FSV through strict Inspector against physical controller readback.
  3. Update state again with daemon/run directory and accepted/rejected evidence.

## 2026-06-02T11:42:49-05:00
- Required wake-up context was re-read after compaction:
  - `C:\code\Synapse\docs\AICodingAgentSuperPrompt.md`;
  - `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`;
  - `AGENTS.md`;
  - all `STATE/*` files;
  - #351, #594, #602, #603, live open queue, git status/log/branch.
- #602 is closed:
  - commit `f0f8dc9 fix(mcp): support drag curves and modifiers (#602) [skip ci]`;
  - RESOLVED evidence https://github.com/ChrisRoyse/Synapse/issues/602#issuecomment-4604591472;
  - closure readback `state=CLOSED`, `closedAt=2026-06-02T16:40:00Z`;
  - stale claim labels are removed.
- Git state after #602 close:
  - branch `main`;
  - `git status --short --branch` read `## main...origin/main`;
  - latest commit `f0f8dc9`.
- Live open queue after #602:
  - #594 parent remains open;
  - #624/#625 remain `status:blocked` on the Daybreak/operator boundary;
  - unblocked children currently open include #603, #604, and #629-#634.
- Active issue is #603 `scenario(stress): ViGEm gamepad full sweep - X360 + DS4 buttons/sticks/triggers`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/603#issuecomment-4604616630.
  - Labels/assignee updated with `status:in-progress`, `agent:codex`, and `ChrisRoyse`.
  - Issue goal: prove the real `act_pad` ViGEm backend across X360 and DS4 buttons, sticks, triggers, neutralization, concurrent controllers, lifecycle/rate-limit/fail-closed edges, and real physical game/controller readback.
  - Planned SoTs: repo-built daemon process/socket/auth/health/strict Inspector `tools/list`; real `act_pad` calls; physical controller-observation surface (Windows/gamepad tester and/or browser Gamepad API via strict client-safe workflow); action/storage audit rows; ViGEm/device/process state; cleanup `release_all` and pad neutral state.
- Current next:
  1. Inspect `act_pad` MCP params, schema, validation, ViGEm backend dispatch, button/stick/trigger mapping, neutralization, lifecycle handling, rate limiting, and tests.
  2. Patch only if code or FSV exposes a real gap.
  3. Build/launch isolated repo-built daemon and perform manual MCP/SoT FSV for #603.

## 2026-06-02T11:45:00-05:00
- Active issue #602 `scenario(stress): act_drag boundary + Paint drawing + Explorer drag-drop` has implementation, manual MCP/SoT FSV, runtime cleanup, final supporting checks, release build, and diff review complete. Commit/push and GitHub closeout are next.
- Patch in `crates/synapse-mcp/src/m2/drag.rs`:
  - exposes `DragCurve::Bezier` in `act_drag` and maps it to `AimCurve::Bezier`;
  - adds `modifiers: Vec<DragModifier>` with `ctrl/shift/alt/super`;
  - wraps live drags with `KeyDown` modifiers before `MouseDrag`, a 200 ms post-drop settle before `KeyUp`, and release cleanup on failures;
  - recording-backend tests prove all five curve labels and the modifier sequence `key_down:shift>down:left>mouse_move...>up:left>key_up:shift`.
- Manual MCP/SoT FSV evidence is under `.runs\602\drag-fsv-20260602T1105-clean`.
  - Initial daemon PID `52268`, resumed daemon PID `32428`, patched daemon PID `84276`, final patched-settle daemon PID `61420`, all on `127.0.0.1:7883` with repo release binary; final release binary length `46820864`, SHA256 `8432EEC297778C356BF0B006EABE9D0FA3AA94A6F0ADFF93BC4A3452A1D66826`, timestamp `2026-06-02T16:30:42.4786488Z`.
  - Auth health readbacks passed and unauth health returned `401`; final strict Inspector `tools/list` `64_tools_list_patched2.json` loaded `80` tools and showed `act_drag` curve enum `natural,instant,linear,ease_in_out,bezier` and modifier enum `ctrl,shift,alt,super`.
  - Paint happy path: `paint_curves.png` before length `7107`, SHA256 `F0D46746CD633A789F1F3D9D03B0AAF6B448D09A236991F66F0E2A9DFFC3C9E0`, white control pixels; real MCP `act_drag` calls for natural/instant/linear/ease_in_out/bezier produced five strokes; after length `7737`, SHA256 `1828E65628ED8B2D220C3385A0DA796D8F85DE72AB53A3CA334052FD0C466E23`, sampled stroke pixels black and control pixel white.
  - Explorer happy path: `issue602_dragdrop.txt` before source existed/dest absent/SHA256 `D71F53A60259BF35EFF72D3BB4DEBA460651A30F55B42BEC4C76ED6324F05016`; `find` located row bbox `x=332,y=434,w=1013,h=37`; real MCP `act_drag` from `(838,452)` to destination Items View `(3675,932)` moved file; after source absent, dest exists length `38` with same SHA256; `CF_ACTION_LOG` moved `20 -> 22`.
  - Boundary/cross-monitor: monitor SoT `26_monitor_sot_before_boundary.json` showed 3 monitors and virtual screen `10240x1742`; real MCP middle-button drag from `(5120,500)` to `(9216,500)` returned `distance_px=4096`, crossed DISPLAY1->DISPLAY3, storage `22 -> 24`, and separate input read showed no held buttons/keys.
  - Over-limit: `(5120,500)` to `(9217,500)` failed closed with `action drag distance exceeds limit ... 4097.000 px exceeds max 4096 px`; no held input and file hash unchanged; rejected attempt was audited (`CF_ACTION_LOG 24 -> 26`).
  - Zero-length: middle-button `(6000,600)` to `(6000,600)` returned `distance_px=0`, storage `26 -> 28`, no held input, file hash unchanged.
  - Non-drop-target: `issue602_non_drop.txt` before source-only/SHA256 `30E924C3BFEB69F103959DB1F7CB2B7ADF7F985AA8D56EC0BBE0F0B0A62D639D`; drag from row center to source title/tab area `(1200,216)` left source present, dest absent, hash unchanged; storage `32 -> 34`, inputs neutral.
  - Modifier-held: first Ctrl attempt showed a physical race (file moved), so the patch added the 200 ms settle. Repeated real MCP `act_drag modifiers=["ctrl"]` on `issue602_ctrl_settle.txt` physically copied: source and dest both exist, both length `32`, both SHA256 `F6209DEE7E9B5C536DA81B06D7FEFC76E71FB47E1D993DC956B0CD6C6F26E6C0`; storage `48 -> 50`, Ctrl/Shift/buttons released.
  - Empty params and structurally invalid params failed closed through strict Inspector/MCP deserialization; `CF_ACTION_LOG` stayed `50`, inputs remained neutral, and known file bytes stayed unchanged.
  - Cleanup: strict Inspector `release_all` returned `released_keys=0`, `released_buttons=0`, `neutralized_pads=0`; separate `GetAsyncKeyState` read showed Ctrl/Shift/left/middle/right all false; source/destination Explorer windows closed; final daemon PID `61420` stopped and port `7883` closed.
- Final supporting checks passed:
  - `cargo fmt --check`;
  - `git diff --check` (CRLF warning only);
  - `cargo test -p synapse-mcp --bin synapse-mcp recording_backend_readback_exposes_all_drag_curve_variants -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp recording_backend_readback_holds_drag_modifiers_around_drag -- --nocapture`;
  - `cargo test -p synapse-action --test mouse_drag_validation -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`;
  - `cargo test -p synapse-mcp --test m3_tools_list -- --nocapture`;
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`;
  - `cargo check -p synapse-action -p synapse-mcp -j 2`;
  - `cargo build --release -p synapse-mcp -j 2`.
- Current next:
  1. Scan tracked diff for token leakage, commit with `[skip ci]`, push.
  2. Post #602 RESOLVED evidence, close #602, remove stale labels.
  3. Refresh queue and continue to #603 unless GitHub changed.

## 2026-06-02T10:40:00-05:00
- #601 is closed:
  - commit `aa81266 fix(reflex): persist combo timing audits (#601) [skip ci]`;
  - RESOLVED evidence https://github.com/ChrisRoyse/Synapse/issues/601#issuecomment-4604000389;
  - closure readback `state=CLOSED`, `closedAt=2026-06-02T15:31:27Z`;
  - stale `status:in-progress` and `agent:codex` labels removed.
- Git state after #601 close:
  - branch `main`;
  - `git status --short --branch` read `## main...origin/main`;
  - latest commit `aa81266`.
- Live open queue after #601:
  - #594 parent remains open;
  - #624/#625 remain `status:blocked` on the Daybreak/operator boundary;
  - unblocked children currently open include #602-#604 and #629-#634.
- Active issue is #602 `scenario(stress): act_drag boundary + Paint drawing + Explorer drag-drop`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/602#issuecomment-4604006525.
  - Labels/assignee updated with `status:in-progress`, `agent:codex`, and `ChrisRoyse`.
  - Issue goal: prove `act_drag` curves, 4096px distance boundary, real Paint drawing, and real drag/drop file movement.
  - Planned SoTs: repo-built daemon process/socket/auth/health/strict Inspector `tools/list`; real `act_drag` calls; Paint image/file bytes and/or observe/read_text evidence; filesystem paths for drag/drop movement; storage/action audit rows; OS key/button state; cleanup release state.
  - Required edges: zero-length drag, non-drop target, modifiers held, cross-monitor/DPI feasibility on this host, >4096px fail-closed, empty/boundary/structurally invalid params with before/after state.
- Current next:
  1. Inspect `act_drag` params, validation, action backend dispatch, curve handling, distance limit, audit logging, and tests.
  2. Patch only if the real code path cannot satisfy #602.
  3. Build/launch isolated repo-built daemon and perform manual MCP/SoT FSV.

## 2026-06-02T10:25:00-05:00
- Active issue remains #601 `scenario(stress): act_combo 256-step timed precision - play a song / macro`.
- Implementation patch is unchanged since the #601 audit patch:
  - combo completion rows now persist `details.combo_completion` with due/elapsed/jitter/action details;
  - MCP validation tests cover empty, `>256`, non-monotonic, and unsupported non-`act_press` combo steps.
- Supporting checks already passed before runtime FSV:
  - `cargo fmt`;
  - `cargo test -p synapse-reflex --test combo_behavior -- --nocapture` (7 passed);
  - `cargo test -p synapse-mcp --bin synapse-mcp combo_ -- --nocapture` (8 passed);
  - `cargo fmt --check`;
  - `cargo check -p synapse-reflex -p synapse-mcp -j 2`;
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`;
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`;
  - `cargo test -p synapse-mcp --test m3_reflex_history_tool -- --nocapture`;
  - `cargo build --release -p synapse-mcp -j 2`.
- Accepted manual MCP/SoT FSV evidence:
  - First run `.runs\601\combo-fsv-20260602T1010` proved the repo-built daemon PID `92280`, bind `127.0.0.1:7881`, release SHA256 `669191BA58F581763DB6B389979EF6545ADC458B6AAA9BDEF72DB516FCC51B6D`, strict Inspector `tools/list=80`, and 256-step target-file output of exactly 256 `a` characters.
  - That run's `reflex_history` readback summary is `.runs\601\combo-fsv-20260602T1010\35_256_combo_reflex_summary.json`: combo `019e88df-6cb1-74e0-a4e7-e3d4c180e5c0`, `scheduled_actions=512`, `dispatched_actions=512`, `elapsed_ms=1481`, `max_jitter_ms=205`; the 5 ms cadence intentionally saturated the normal action path but still produced all physical characters.
  - Second run `.runs\601\combo-fsv-20260602T1030-edges` used daemon PID `85268`, bind `127.0.0.1:7882`, same release SHA256, auth health 200/unauth 401, strict Inspector `tools/list=80`, and visible target PID `84500` writing `issue601_combo_target_edges.txt`.
  - Single-step combo `019e88e7-2f61-7640-ac02-23eb621d624a` wrote exactly `z`; storage moved `CF_ACTION_LOG=8 -> 10`, `CF_REFLEX_AUDIT=0 -> 3`; `reflex_history` recorded `scheduled_actions=2`, `dispatched_actions=2`, `max_jitter_ms=0`.
  - Precise 256-step combo `019e88ea-a31d-7921-86eb-4665c3decfce` at 20 ms cadence wrote exactly 256 `b` characters; storage moved `CF_ACTION_LOG=14 -> 16`, `CF_REFLEX_AUDIT=3 -> 6`; `reflex_history` recorded `scheduled_actions=512`, `dispatched_actions=512`, `elapsed_ms=5105`, `max_jitter_ms=0`, and zero `action queue full` log matches.
  - Edges accepted with before/after file + storage readbacks: structurally invalid `steps` object, empty `steps=[]`, non-monotonic `at_ms`, 257-step boundary, and unsupported nested `act_click` action. Each rejected through strict Inspector/MCP with the target file unchanged and `CF_ACTION_LOG`/`CF_REFLEX_AUDIT` counts unchanged.
  - Cleanup: strict Inspector `release_all` returned zero keys/buttons/pads; separate `GetAsyncKeyState` read showed no relevant input down; target PID `84500` stopped; daemon PID `85268` stopped; ports `7881`/`7882` closed; no visible `Issue601*` windows remain.
- Final supporting checks passed after runtime FSV:
  - `cargo fmt --check`;
  - `git diff --check` (line-ending warnings only);
  - `cargo test -p synapse-reflex --test combo_behavior -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp combo_ -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`;
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`;
  - `cargo test -p synapse-mcp --test m3_reflex_history_tool -- --nocapture`;
  - `cargo check -p synapse-reflex -p synapse-mcp -j 2`;
  - `cargo build --release -p synapse-mcp -j 2`.
- Final release build readback after cleanup: `target\release\synapse-mcp.exe`, length `46748160`, SHA256 `F7C089061FE2CF23B5FBEC9D7A12C55FD19A7C38117CEA637A7CA0B02F4919D5`, `LastWriteTimeUtc=2026-06-02T15:28:21Z`. Accepted FSV evidence is tied to the earlier release binary SHA `669191BA58F581763DB6B389979EF6545ADC458B6AAA9BDEF72DB516FCC51B6D` from the same #601 source patch.
- Tracked diff token scan found zero matches for the issue-local bearer token, raw auth header text, or bearer-token env var name; diff review completed.
- Current next:
  1. Commit with `[skip ci]`, push, post #601 RESOLVED evidence, close #601, remove stale labels.
  2. Refresh queue and continue to #602 unless GitHub changed.

## 2026-06-02T09:45:00-05:00
- #600 is closed:
  - commit `5cf6e0b fix(action): fail closed under action queue flood (#600) [skip ci]`;
  - RESOLVED evidence https://github.com/ChrisRoyse/Synapse/issues/600#issuecomment-4603513011;
  - closure readback `state=CLOSED`, `closedAt=2026-06-02T14:39:18Z`;
  - stale `status:in-progress` and `agent:codex` labels removed.
- Git state after #600 close:
  - branch `main`;
  - `git status --short --branch` read `## main...origin/main`;
  - latest commit `5cf6e0b`.
- Live open queue after #600:
  - #594 parent remains open;
  - #624/#625 remain `status:blocked` on the Daybreak/operator boundary;
  - unblocked children currently open include #601-#604 and #629-#634.
- Active issue is #601 `scenario(stress): act_combo 256-step timed precision - play a song / macro`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/601#issuecomment-4603519772.
  - Issue goal: prove `act_combo` schedules accurate, monotonic, <=256-step timed key sequences via the reflex runtime.
  - Required SoTs from issue: strict real MCP precondition; real `act_combo` trigger; action/reflex audit storage rows for requested vs actual dispatch timing and/or physical audio/target output; cleanup release state.
  - Required edges: non-monotonic steps rejected, 257 steps rejected, non-`act_press` combo step rejected with `TOOL_PARAMS_INVALID`, single-step combo, plus empty/structural invalid before/after SoTs.
- Current next:
  1. Inspect `act_combo` MCP params/validation/scheduling implementation and supporting tests.
  2. Decide whether code needs a patch before runtime FSV.
  3. Build/launch isolated repo-built daemon for #601 manual MCP/SoT FSV.

## 2026-06-02T10:05:00-05:00
- Active issue remains #601.
- Code inspection found #601's persistent timing-evidence gap:
  - `act_combo` already validates empty steps, `>256` steps, non-monotonic `at_ms`, backend mismatches, and unsupported non-`act_press` actions as fail-closed paths.
  - The combo controller emitted a transient `reflex_combo_completed` event containing dispatch timing, but `CF_REFLEX_AUDIT` only persisted generic lifetime-expired completion details (`kind`, `reason`, `tick_index`, `fire_count`) and did not persist requested due times, actual dispatch elapsed times, or jitter.
- Patch now applied:
  - `ComboController::completion_audit_details()` builds nested completion details with `scheduled_actions`, `dispatched_actions`, `elapsed_ms`, `max_jitter_ms`, and per-dispatch `due_ms`, `elapsed_ms`, `jitter_ms`, sequence, and action summary.
  - Stateful combo completion now writes those details into the persisted completion row under `details.combo_completion` while preserving the existing lifetime-expired audit shape.
  - M4 combo validation tests now cover empty steps, `257` steps, non-monotonic steps, and unsupported non-`act_press` actions.
- Focused supporting checks passed:
  - `cargo fmt`;
  - `cargo test -p synapse-reflex --test combo_behavior -- --nocapture` (7 passed);
  - `cargo test -p synapse-mcp --bin synapse-mcp combo_ -- --nocapture` (8 passed).
- Current next:
  1. Run broader touched-crate/schema/tool-list checks.
  2. Build release `synapse-mcp`.
  3. Launch an isolated #601 daemon and perform manual strict-client MCP/SoT FSV against `CF_REFLEX_AUDIT`, `CF_ACTION_LOG`, a physical target file/window, and cleanup state.

## 2026-06-02T09:40:00-05:00
- Active issue remains #600 `scenario(stress): SendInput rate-limit + action-queue overflow`; implementation, manual MCP/SoT FSV, cleanup, final supporting checks, release build, and diff review are complete. Commit/push and GitHub RESOLVED closeout are next.
- Patch summary:
  - `ActionHandle::execute` now fails closed with `ACTION_QUEUE_FULL` on saturated normal action queues instead of awaiting capacity.
  - Emitter-created handles carry a priority safety lane for `ReleaseAll` and `KeyUp`.
  - `ActionEmitter::run` polls safety/auto-release/snapshot/shutdown before normal queue work and rejects pending normal actions after `ReleaseAll` with `SAFETY_RELEASE_ALL_FIRED`.
  - MCP `release_all` calls `synapse_action::request_release_interrupt()` before snapshot/readback so an active software hold wakes before the actor drains held state.
- Accepted manual FSV run: `.runs\600\action-flood-fsv-20260602T0904-final`.
  - Repo-built daemon PID `40028`, bind `127.0.0.1:7880`, binary `target\release\synapse-mcp.exe`, length `46730752`, SHA256 `786FF6F6B62AC564F8F0C7A1DC20E8226A6720AB44A6EB6B75064EC8E88081C2`.
  - Process/socket/auth health readbacks passed; unauth health failed with `HTTP_TOKEN_INVALID`; strict MCP Inspector `tools/list` loaded `80` tools with required action/storage tools present and no schema rejection.
  - Happy path: strict Inspector `act_click` focused Notepad, `act_press d/e/f`, and `ctrl+s`; separate file SoT readback was exactly `def`, length `3`, SHA256 `CB8379AC2098AA165029E3938A51DA0BCECFC008FD6795F401178647F96C5B34`; storage moved from all-zero to `CF_ACTION_LOG=10`.
  - Queue flood: long middle-button hold was physically down during flood; 900 strict MCP `act_click` calls produced `ACTION_QUEUE_FULL=317` and `SAFETY_RELEASE_ALL_FIRED=256`; release_all returned while the long hold elapsed `1549 ms` instead of `30000 ms`; OS input after/final all false; storage `CF_ACTION_LOG` grew to `1813`; logs contained `ACTION_QUEUE_FULL=317`, `SAFETY_RELEASE_ALL_FIRED=258`, `M2_RELEASE_ALL_READBACK=1`.
  - Rate-limit documented gap: real public software calls hit `ACTION_QUEUE_FULL`/backend validation before token buckets (`6000 act_aim` calls: `ACTION_QUEUE_FULL=1679`, `ACTION_BACKEND_UNAVAILABLE=4321`, `ACTION_RATE_LIMITED=0`); real ViGEm calls completed below the `1000/s` bucket (`1200 ok`, elapsed `6610 ms`, `ACTION_RATE_LIMITED=0`). Supporting token-bucket regression checks passed for exact/overshoot behavior.
  - Edges accepted: empty `act_press keys=[]` failed closed with `TOOL_PARAMS_INVALID` and file/OS SoTs unchanged; structurally invalid `keys="not-array"` failed deserialization before action audit and file/OS SoTs unchanged; exact 256 and just-under 255 boundary floods produced `SAFETY_RELEASE_ALL_FIRED` without `ACTION_QUEUE_FULL`; `KeyUp`/release safety under a Shift hold returned `released_keys=1` and interrupted the hold at `1766 ms`; ViGEm happy path returned to neutral and cleanup release_all zeroed pads.
  - Cleanup: strict Inspector `release_all` returned zero keys/buttons/pads; daemon PID `40028` stopped and port closed; Notepad PID `60556` closed; final file SoT still exactly `def`.
- Final supporting checks passed:
  - `cargo fmt --check`;
  - `git diff --check` (line-ending warnings only);
  - `cargo test -p synapse-action --test handle_queue -- --nocapture`;
  - `cargo test -p synapse-action --test emitter_state -- --nocapture`;
  - `cargo test -p synapse-action --test rate_limit_overshoot -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp release_all -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp act_press -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`;
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`;
  - `cargo check -p synapse-action -p synapse-reflex -p synapse-mcp -j 2`;
  - `cargo build --release -p synapse-mcp -j 2`.
- Final release build readback after cleanup: `target\release\synapse-mcp.exe`, length `46730752`, SHA256 `8338D75A74663970FE2239119158082D3D03F8F156F7E9B05276813E10BEFEFB`, `LastWriteTimeUtc=2026-06-02T14:34:15.8470672Z`.
- Current next:
  1. Run tracked diff/token scan and final git status.
  2. Commit with `[skip ci]`.
  3. Push, post #600 RESOLVED evidence, close #600, remove stale labels.
  4. Refresh open queue and continue to #601 unless GitHub changed.

## 2026-06-02T07:55:36-05:00
- Active issue remains #600 `scenario(stress): SendInput rate-limit + action-queue overflow`.
- Additional #600 safety defect found before runtime flood acceptance:
  - the first #600 patch gave `ReleaseAll` a priority safety lane and rejected pending normal queue actions, but MCP `release_all` did not advance the release-interrupt epoch;
  - a long in-flight software hold such as `act_click hold_ms=30000` polls that epoch, so release_all could still wait for the current backend hold to finish before draining held state.
- Patch added:
  - `synapse_action::request_release_interrupt()` now exposes the same interrupt epoch increment used by the operator hotkey;
  - `ActionHandle::execute(Action::ReleaseAll)` and `fire_release_all_blocking_with_timeout` both request the interrupt before enqueueing release_all.
- Supporting checks passed after the new patch:
  - `cargo fmt`;
  - `cargo test -p synapse-action --test handle_queue -- --nocapture` (6 passed; includes release_all interrupt epoch readback);
  - `cargo test -p synapse-action --test emitter_state release_all_safety_lane_preempts_saturated_normal_queue -- --nocapture`;
  - `cargo check -p synapse-action`.
- Existing isolated #600 daemon PID `75412` predates this latest patch and must not be used for final acceptance. Next: run broader touched-crate checks, rebuild release `synapse-mcp`, stop/replace the old isolated daemon, then redo strict-client precondition and manual MCP/SoT FSV.

## 2026-06-02T07:30:35-05:00
- Active issue remains #600 `scenario(stress): SendInput rate-limit + action-queue overflow`.
- Code inspection found a real #600 MCP-path gap:
  - `ActionHandle::try_execute` returned `ACTION_QUEUE_FULL` at the 256 bound, but real MCP tools used `ActionHandle::execute`, which awaited bounded `mpsc::Sender::send` capacity instead of failing closed.
  - `release_all` and panic/operator safety release also shared the saturated normal queue; `KeyUp` was rate-limit exempt but still vulnerable to normal queue saturation.
- Patch now applied in `synapse-action`:
  - `ActionHandle::execute` uses non-blocking enqueue for normal actions and returns `ACTION_QUEUE_FULL` at capacity.
  - emitter-created handles carry a priority safety lane for `ReleaseAll` and `KeyUp`.
  - `ActionEmitter::run` polls safety/auto-release/snapshot/shutdown before normal actions.
  - after `ReleaseAll`, the actor rejects pending normal queue items with `SAFETY_RELEASE_ALL_FIRED` so stale queued input cannot resume after a safety release.
- Supporting checks already passed:
  - `cargo fmt`
  - `cargo check -p synapse-action`
  - `cargo test -p synapse-action --test handle_queue -- --nocapture`
  - `cargo test -p synapse-action --test emitter_state release_all_safety_lane_preempts_saturated_normal_queue -- --nocapture`
- Current next:
  1. Run broader touched-crate checks for `synapse-action`, `synapse-reflex`, and `synapse-mcp`.
  2. Build release `synapse-mcp`.
  3. Launch isolated #600 daemon and perform manual strict-client MCP/SoT FSV against action audit rows, Notepad text, OS input state, and logs.

## 2026-06-02T07:20:00-05:00
- #599 is closed:
  - commit `9252e93 fix(mcp): harden audio tail stress path (#599) [skip ci]`;
  - RESOLVED evidence https://github.com/ChrisRoyse/Synapse/issues/599#issuecomment-4602291835;
  - closure readback `state=CLOSED`, `closedAt=2026-06-02T12:17:50Z`;
  - stale `status:in-progress` label removed.
- Git state after #599 close:
  - branch `main`;
  - `git status --short --branch` read `## main...origin/main`.
- Live queue after #599:
  - #594 parent remains open;
  - #624/#625 remain `status:blocked` on the Daybreak/operator boundary;
  - unblocked children currently open include #600-#604 and #629-#634.
- Active issue is #600 `scenario(stress): SendInput rate-limit + action-queue overflow`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/600#issuecomment-4602297629.
  - Issue goal: prove the software backend token bucket (`5000/s`) and `256`-deep action queue fail closed under flood; prove release safety is not starved.
  - Planned SoTs: repo-built daemon process/socket/auth/health/strict Inspector tools-list; `CF_ACTION_LOG` via `storage_inspect`; OS key/button state; Notepad text for actual dispatched keystrokes; logs/process state; cleanup release state.
  - Required edges from issue: exact burst at capacity, sustained just-under-limit, ViGEm 1000/s bucket if host exposes pad backend, interleaved `release_all` during flood, empty/structurally invalid params.
- Current next:
  1. Inspect `synapse-action` software backend rate limiter, queue, release exemption, and action audit logging.
  2. Patch only if code/FSV exposes real gaps.
  3. Build and launch a repo-built isolated daemon for #600 manual MCP/SoT FSV.

## 2026-06-02T06:58:40-05:00
- Active issue remains #599 `scenario(stress): audio stress — loopback music+speech, VAD/azimuth, Whisper transcribe`.
- Implementation patch has manual MCP/SoT FSV accepted; final supporting checks and diff review are complete; commit/closeout are next.
- Accepted post-telemetry run: `.runs\599\audio-fsv-20260602T0647-accepted`.
  - Repo-built daemon PID `76024`, bind `127.0.0.1:7877`, release binary `target\release\synapse-mcp.exe`, length `46730240`, SHA256 `A5B88B6B1048EB64AB9A7E8CEB77979D8FB4EF26112964F3DCB27F634DDBEC09`, timestamp `2026-06-02T11:46:01Z`.
  - Process/socket/auth health readbacks passed; unauth `/health` failed with `HTTP_TOKEN_INVALID`; strict MCP Inspector `tools/list` loaded `80` tools and showed `audio_tail.seconds`/`audio_transcribe.seconds` as `number` with `minimum=0`, `maximum=30`, `default=5`.
  - Initial isolated storage was all zeroes across all CFs.
  - Empty/silence edges: `audio_tail seconds=0` returned `frames=0`, `pcm_count=0`, `rms_db=-120`, `vad=0`; `seconds=0.1` returned a bounded 100ms empty/silent read; muted 1s silence returned `frames=48000`, `pcm_count=192000`, `rms_db=-120`, `vad=0`.
  - Stereo direction: left fixture returned `azimuth=-90`, confidence `0.90016`; center rerun returned `azimuth≈0`, confidence `0.35`; right rerun returned `azimuth=90`, confidence `0.90018`. A parallel center/right attempt was rejected because loopback playback overlapped.
  - Speech/music: `speech_tone_mix_5s` returned `captured=5`, `frames=240000`, RMS `-25.709`, VAD `35.2%`, events `speech_ended,music_ended,music_started,speech_started,loud_transient`; `speech_overlap_5s` returned RMS `-23.706`, VAD `61.6%`, expected speech/transient/music events.
  - Loud transient: `loud_transient_1s` returned one bounded `loud_transient` event in the recent event list.
  - Transcription: first 5s run was rejected because it recognized only `This is Synapse.`; 6s rerun captured the whole 5s fixture and returned exact text `Hello world. This is Synapse.` with confidence `0.86`.
  - Fail-closed edges: `language=es` failed with `audio_transcribe language must be "en"; got "es"`; `seconds=bad` failed deserialization as non-number; `seconds=31` failed with `audio seconds must be between 0 and 30; got 31`; storage stayed all zero after these edges.
  - 30s boundary: `audio_tail seconds=30` over `hello_loop_30s.wav` returned `captured=30`, `frames=1440000`, `pcm_count=5760000`, RMS `-25.416`, VAD `37.6%`, events `speech_started,music_started,loud_transient,speech_ended,music_ended`.
  - Metadata-only readback after all accepted-run triggers: `storage_inspect` still all zero; log and DB byte scans under the product `logs`/`db` directories found no `Hello world. This is Synapse.`, `Hello world`, `CallToolResult`, `response message`, or `pcm` matches. PCM/transcript exist only in issue evidence stdout artifacts.
- Disabled-audio edge accepted on `.runs\599\audio-fsv-20260602T065621-disabled-final3`.
  - Rejected two setup launches first: one inherited the wrong token env name and exited before binding; corrected env was `SYNAPSE_BEARER_TOKEN`.
  - Repo-built daemon PID `44472`, bind `127.0.0.1:7878`, no `--enable-audio`; health audio SoT read `status=disabled`, `detail="audio is disabled; start with --enable-audio"`, `ring_buffer_seconds=30`, `stt_model_loaded=false`.
  - Strict Inspector `tools/list` loaded `80` tools; storage before all zero; `audio_tail seconds=1` failed with `tool audio_tail requires permission READ_AUDIO`; storage after all zero.
- Cleanup:
  - real Inspector `release_all` returned `released_keys=0`, `released_buttons=0`, `neutralized_pads=0` on both issue-local daemons;
  - stopped PIDs `76024` and `44472`;
  - ports `7877`/`7878` are closed and `ffplay_count=0`;
  - restored host audio sessions changed for the run: `pulseaudio|24952|before=True|after=False`, `chrome|34540|before=True|after=False`.
- Final supporting checks passed after the last doc edit:
  - `cargo fmt --check`;
  - `git diff --check` with line-ending warnings only;
  - stale audio docs scan for old `f32`/`u32`/5s wording;
  - `cargo test -p synapse-audio --test ring_detectors -- --nocapture`;
  - `cargo test -p synapse-audio -- --nocapture`;
  - `cargo test -p synapse-telemetry -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp audio_ -- --nocapture`;
  - `cargo test -p synapse-mcp --test m3_audio_tail_tool -- --nocapture`;
  - `cargo test -p synapse-mcp --test m3_audio_transcribe_tool -- --nocapture`;
  - `cargo test -p synapse-mcp --test m3_tools_list -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`;
  - `cargo check -p synapse-audio -p synapse-telemetry -p synapse-mcp -j 2`;
  - `cargo build --release -p synapse-mcp -j 2`.
- Final release build readback after the clean build: `target\release\synapse-mcp.exe`, length `46730240`, SHA256 `5DA77B06F1E100B2E4049B460E56240B782C6162157FDFCA7AEC89E0B8D6A04A`, timestamp `2026-06-02T12:13:43Z`. Note: release binary hashes varied across rebuilds with unchanged source; accepted FSV evidence remains tied to daemon binary SHA `A5B88B6B1048EB64AB9A7E8CEB77979D8FB4EF26112964F3DCB27F634DDBEC09`.
- Diff review completed for audio ring/detectors, M3 audio tool contract/metadata, telemetry payload-safe logging, snapshots/tests, docs, and state notes. Tracked diff token scan found no bearer token values.
- Current next: commit with `[skip ci]`, push, post #599 RESOLVED evidence, close #599, refresh queue.

## 2026-06-02T05:49:18-05:00
- #598 is closed and `main` was clean before #599 work began.
- Active issue is #599 `scenario(stress): audio stress — loopback music+speech, VAD/azimuth, Whisper transcribe`.
  - GitHub readback: issue is open, assigned to `ChrisRoyse`, labels include `status:in-progress`, `agent:codex`, `area:audio`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/599#issuecomment-4601558594.
- Code inspection found real #599 contract gaps:
  - `synapse-audio` hard-coded `DEFAULT_RING_SECONDS = MAX_RING_SECONDS = 5`, but #599 requires `0.1..=30` second windows and a 30s max edge.
  - MCP `audio_tail.seconds` was `u32`, so strict clients could not call `seconds=0.1`.
  - MCP `audio_tail` returned PCM only; RMS/VAD/events/azimuth metadata was available through `observe include audio`, but #599 names `audio_tail` as the tool to prove those values.
- Patch in progress:
  - `crates/synapse-audio/src/lib.rs`: `DEFAULT_RING_SECONDS = MAX_RING_SECONDS = 30`.
  - `crates/synapse-mcp/src/m3/audio.rs`: audio tool `seconds` params are now numeric `f32`; validation is finite `0..=30`; `audio_tail` response includes compact metadata: `requested_seconds`, `captured_seconds`, `frames`, `rms_db`, `vad_speech_pct`, bounded `recent_events`, and optional `direction_estimate`, while preserving zero-second empty PCM without runtime startup.
  - Focused audio tool tests and live systemspec docs are being updated to match the new strict schema.
- Host prerequisite readback:
  - Whisper model exists at `C:\Users\hotra\AppData\Local\synapse\models\whisper-tiny-int8.onnx`.
  - SHA256 readback matched pinned `147AFAC751F89AD8E8F82133464EDC81ECFF9391E98CCDCAE2474384BE68EC86`.
  - ONNX Runtime extensions wheel/unpacked directory exists under `%LOCALAPPDATA%\synapse\models\ort-extensions`.
- Next:
  1. Run fmt/focused audio/schema checks and fix any compile/snapshot issues.
  2. Build release `synapse-mcp`.
  3. Launch an isolated repo-built daemon with audio enabled and copied model/extension SoTs.
  4. Run #599 manual MCP/SoT FSV with real loopback playback and strict Inspector tools/call triggers.

## 2026-06-02T05:36:10-05:00
- Active issue #598 is ready for commit/GitHub closeout.
- Accepted main #598 runtime evidence directory: `.runs\598\detection-fsv-20260602T0513`.
  - Repo-built release daemon PID `28444`, bind `127.0.0.1:7872`, binary `target\release\synapse-mcp.exe`, SHA256 `F8B15ED79B3A5D4D1FF9CE2522189341614589D73348410C4F330A982E170264`.
  - Strict MCP Inspector `tools/list` loaded `80` tools and required `set_perception_mode`, `observe`, `find`, `storage_inspect`, and `release_all`.
  - Initial isolated storage readback was all zeroes; verified model SHA256 in isolated `LOCALAPPDATA` was `583A236AC21C95A7FD94F284FC21485E42355BFEF82C27011BA78FBC09EE87E2`.
  - Deterministic target PID `76292`, title `Issue598DetectionTarget`, displayed COCO image SHA256 `DEA9E7EF97386345F7CFF32F9055DA4982DA5471C48D575146C796AB4563B04E`.
  - Pixel-only still frame: real Inspector `set_perception_mode pixel_only` then `observe` returned `mode=pixel_only`, `detection_status=healthy`, two cats plus remote, confidences `0.9118`, `0.8808`, `0.6176`, and track IDs `1/2/3`; separate storage readback persisted `CF_OBSERVATIONS=1`, `CF_EVENTS=1`.
  - Visual SoT screenshot `06_pixel_still_screenshot.png` SHA256 `8574EB0B4B18281FC47BCDA075F58EA7D90690BDAED48039E7D4E2633BDDB2C0`; manual visual read showed reported bboxes align with the two visible cats and remote.
  - Moving pair: target command set `scene=single`, `vx=180`; observe1 reacquired tracks `4/5/6/7`; observe2 kept cat track IDs `4` and `5` and persisted velocities `[268.11594,16.908213]` and `[267.63287,18.84058]`.
  - Hybrid mode: `set_perception_mode hybrid` then `observe` returned `mode=hybrid`, healthy A11y/capture/detection, cats/sofa/remote with all confidences above `0.637`.
  - `find query=cat scope=both` returned two entity results with cat bboxes matching the visible cats.
  - Empty/black edge: target `scene=black` had empty `draw_rects`; `observe` returned healthy detection with `entity_count=0`.
  - Leave/re-enter edge: after black frame beyond the 3000 ms stale window, restored COCO frame returned new track IDs `18..21` instead of stale IDs.
  - Confidence-floor evidence: default-profile observations had all confidences at/above the 0.5 floor; grid min confidence was `0.5220284`.
  - Structurally invalid edge: real Inspector `set_perception_mode` with missing `mode` failed `missing field mode`; storage counts stayed `CF_EVENTS=8`, `CF_OBSERVATIONS=8`.
- Accepted max-detections cap evidence directory: `.runs\598\detection-cap-fsv-20260602T0523`.
  - First cap daemon launch attempt was rejected after the daemon failed closed on intentionally over-broad `SYNAPSE_ALLOW_SHELL=.*` with `SHELL_PATTERN_TOO_BROAD`.
  - Relaunched repo-built daemon PID `67068`, bind `127.0.0.1:7873`, same release SHA, issue-local `SYNAPSE_PROFILE_DIR`, copied model hash matched.
  - Strict Inspector `tools/list` loaded `80` tools; unauth health returned `401 HTTP_TOKEN_INVALID`; profile_list showed only `issue598.cap`.
  - Issue-local profile matched `Issue598DetectionTarget`, health readback showed `active_profile_id=issue598.cap`, and profile file set `max_detections=2`.
  - Same large grid scene that produced `10` detections on the default profile returned exactly `2` detections on the cap profile; separate storage readback persisted `CF_OBSERVATIONS=1`, `CF_EVENTS=1`, profile source `profile:issue598.cap`.
- Cleanup:
  - real Inspector `release_all` returned zero keys/buttons/pads on both daemons;
  - stopped daemon PIDs `28444` and `67068` plus target PID `76292`;
  - ports `7872` and `7873` closed and no `Issue598DetectionTarget` window remained.
- Final supporting checks passed after accepted FSV:
  - `cargo fmt --check`;
  - `git diff --check` with line-ending warnings only;
  - focused tracker/class-filter/manual-mode tests;
  - `cargo test -p synapse-models --test model_loader -- --nocapture`;
  - `cargo check -p synapse-models -p synapse-mcp -j 2`;
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`;
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`;
  - `cargo build --release -p synapse-mcp -j 2`.
- Final release binary readback:
  - path `C:\code\Synapse\target\release\synapse-mcp.exe`;
  - length `46708736`;
  - SHA256 `32968BB49188230EC41C2DAD5822B6B4E2A9405522DC3D4501719FBA0BEADCE6`;
  - `LastWriteTimeUtc=2026-06-02T10:35:24.2191556Z`.
- Diff review completed for model inference, M1 detection runtime/tracker, MCP observe/find wiring, profile mode persistence, docs, and state notes.
- Next:
  1. Commit with `[skip ci]`.
  2. Post #598 RESOLVED evidence, close #598, remove stale `status:in-progress`.
  3. Refresh open queue and continue to #599 unless GitHub changed.

## 2026-06-02T05:28:20-05:00
- Superseded by the 05:36 final-check update above. Manual MCP/SoT FSV was accepted at this checkpoint; final supporting checks were still pending then.

## 2026-06-02T05:12:31-05:00
- Active issue remains #598.
- First isolated runtime run `.runs\598\detection-fsv-20260602T0458` is rejected as final acceptance evidence but kept as defect/setup evidence:
  - repo-built daemon PID `63716` on `127.0.0.1:7871` passed process/socket/binary, unauth/auth health, and strict Inspector `tools/list=80`;
  - initial isolated storage read was all zeroes;
  - deterministic WinForms target PID `74240`, title `Issue598DetectionTarget`, displayed COCO image `000000039769.jpg` with SHA256 `DEA9E7EF97386345F7CFF32F9055DA4982DA5471C48D575146C796AB4563B04E`;
  - first real `observe` in `pixel_only` failed closed with `DETECTION_MODEL_NOT_LOADED` because isolated `LOCALAPPDATA` had no model; copied the verified RT-DETR model into `.runs\598\...\localappdata\synapse\models\rtdetr_v2_s_coco.onnx` and reread matching SHA256 `583A236AC21C95A7FD94F284FC21485E42355BFEF82C27011BA78FBC09EE87E2`;
  - still pixel-only observe then returned healthy detections: 2 cats, sofa, remote, all confidence >= 0.633;
  - moving pair exposed a real tracker issue: strict Inspector observations were about 1.7s apart, exceeding the old 1500 ms stale-track window, so moving cats reacquired new track IDs and no velocity was produced.
- Patch after rejected run:
  - `STALE_TRACK_MS` increased from 1500 to 3000 ms to tolerate strict-client/inference cadence jitter while still requiring a real loss interval before reacquire;
  - systemspec docs updated from 1500 ms to 3000 ms.
- Cleanup of rejected daemon:
  - real Inspector `release_all` returned exit 0;
  - stopped daemon PID `63716`;
  - port `7871` closed.
- Supporting checks after tracker stale-window patch:
  - `cargo fmt --check`;
  - `cargo test -p synapse-mcp --bin synapse-mcp tracker_ -- --nocapture`;
  - `cargo build --release -p synapse-mcp -j 2`.
- New release binary readback:
  - path `C:\code\Synapse\target\release\synapse-mcp.exe`;
  - length `46708736`;
  - SHA256 `F8B15ED79B3A5D4D1FF9CE2522189341614589D73348410C4F330A982E170264`;
  - `LastWriteTimeUtc=2026-06-02T10:12:23Z`.
- Next:
  1. Launch a fresh post-patch isolated daemon/run, copy/read the isolated model SoT before observing.
  2. Repeat strict Inspector precondition and #598 manual MCP/SoT behavior FSV.

## 2026-06-02T04:56:41-05:00
- Active issue remains #598.
- Additional #598 code/docs since the prior checkpoint:
  - `m1/detection.rs` now honors `ProfileDetection.classes_of_interest` as a case-insensitive filter when the list is non-empty;
  - systemspec docs were updated in `02_source_code_map.md`, `09_perception_and_capture.md`, `10_audio_and_models.md`, and aggregate `SYNAPSE_SYSTEMSPEC.md` so they no longer claim M1 detector invocation is absent.
- Supporting checks now passed after the class-filter/doc patch:
  - `cargo fmt --check`;
  - `cargo test -p synapse-mcp --bin synapse-mcp tracker_ -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp classes_of_interest_filter_is_case_insensitive_and_empty_allows_all -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp runtime_apply -- --nocapture`;
  - `cargo test -p synapse-models --test model_loader -- --nocapture`;
  - `cargo check -p synapse-models -p synapse-mcp -j 2`;
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`;
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`;
  - fixed-string contradiction scans found no remaining systemspec claims that M1 does not invoke detectors or that detection entities are only synthetic;
  - `git diff --check` exited 0 with line-ending warnings only;
  - `cargo build --release -p synapse-mcp -j 2`.
- Release binary readback for upcoming manual #598 FSV:
  - path `C:\code\Synapse\target\release\synapse-mcp.exe`;
  - length `46708736`;
  - SHA256 `696E42B2CA5B590A5605950BC47A37F5E656307F9D950D14B7AACA7E4501AE01`;
  - `LastWriteTimeUtc=2026-06-02T09:56:34Z`.
- Next:
  1. Launch an isolated repo-built daemon from that binary on a fresh port with isolated DB/log/appdata/localappdata/token.
  2. Verify process/socket/binary, unauth/auth `/health`, and strict MCP Inspector `tools/list`.
  3. Create the deterministic moving COCO-object target and run manual MCP/SoT FSV.

## 2026-06-02T04:46:11-05:00
- Wake-up after compaction was rerun:
  - read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #351, #594, #598, live open queue, git status/log;
  - wired configured `mcp__synapse.health`, `storage_inspect`, `observe`, and `find` all returned through the configured client.
- Active issue remains #598 `scenario(stress): detection + entity tracking on fast-moving scene (pixel_only/hybrid)`.
  - #598 is open, assigned to `ChrisRoyse`, labels include `status:in-progress` and `agent:codex`;
  - START comment remains https://github.com/ChrisRoyse/Synapse/issues/598#issuecomment-4600966276.
- Live queue after wake-up:
  - #594 parent remains open;
  - #624/#625 remain `status:blocked` on the Daybreak operator-only boundary;
  - unblocked children still open include #598-#604 and #629-#634.
- Git reconciliation:
  - branch `main`;
  - `HEAD == origin/main == 74dc3b4 docs(state): record issue 598 start [skip ci]`;
  - dirty files are #598-owned detection/runtime patch files: `Cargo.lock`, `crates/synapse-mcp/Cargo.toml`, `crates/synapse-mcp/src/m1.rs`, `crates/synapse-mcp/src/m1/detection.rs`, `crates/synapse-mcp/src/server.rs`, `crates/synapse-mcp/src/server/m1_tools.rs`, `crates/synapse-models/Cargo.toml`, `crates/synapse-models/src/lib.rs`, `crates/synapse-models/src/session.rs`, and `crates/synapse-models/tests/model_loader.rs`.
- #598 root cause found and patched locally:
  - M1 never invoked the detector in `pixel_only`/`hybrid`; `sources.rs` only did capture probing and synthetic Luanti entities;
  - `synapse-models::LoadedModel::infer` returned an empty `DetectionBatch` even when the RT-DETR ONNX model loaded;
  - profile runtime application overwrote explicit `set_perception_mode pixel_only/hybrid` with the active profile mode before `observe`.
- Patch in progress:
  - added real RT-DETR preprocessing/inference/decode in `synapse-models` using captured RGB frame bytes;
  - added M1 detection runtime with fail-closed model/capture/inference errors, default RT-DETR model loading, and entity tracking with stable `track_id` plus `velocity_px_s`;
  - `observe` and `find` now populate detection entities when effective mode is `pixel_only` or `hybrid`;
  - explicit non-auto perception mode now persists across profile refresh; `auto` releases back to profile mode.
- Local model/runtime prerequisite readbacks already completed:
  - model file `C:\Users\hotra\AppData\Local\synapse\models\rtdetr_v2_s_coco.onnx`;
  - length `81057510`;
  - SHA256 `583A236AC21C95A7FD94F284FC21485E42355BFEF82C27011BA78FBC09EE87E2`, matching the registry descriptor;
  - Python `onnxruntime 1.26.0` with `CPUExecutionProvider` can run the model, and a local probe decoded COCO cat/remote/sofa detections from the pinned model.
- Supporting checks already passed after the patch:
  - `cargo fmt`;
  - `cargo check -p synapse-models -j 2`;
  - `cargo check -p synapse-mcp -j 2`;
  - `cargo test -p synapse-models --test model_loader -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp tracker_ -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp runtime_apply -- --nocapture`.
- Current configured MCP baseline is not acceptance evidence for #598:
  - long-lived installed stdio daemon reports `ok=true`, active profile `vscode`, detection `disabled`, and `find Issue615FanoutTarget` returns no results;
  - it is useful only as client sanity until a repo-built isolated daemon is launched from this patch.
- Next:
  1. Run final focused checks including schema sanitize, strict tools-list test, touched-crate checks, docs updates, and release build.
  2. Launch isolated repo-built `synapse-mcp` for #598, verify process/socket/auth/health/strict Inspector `tools/list`.
  3. Create deterministic fast-moving visual target using real COCO-visible objects and read target/frame/window SoTs.
  4. Run manual MCP/SoT FSV for `pixel_only`, `hybrid`, stable track/velocity, `find` by class, leave/re-enter, threshold floor, max cap, black frame, and structurally invalid params.

## 2026-06-02T04:22:34-05:00
- #597 is closed:
  - commit `d64c6a2 fix(mcp): cache read_text OCR by captured pixels (#597) [skip ci]`;
  - RESOLVED evidence https://github.com/ChrisRoyse/Synapse/issues/597#issuecomment-4600960919;
  - closure readback `state=CLOSED`, `closedAt=2026-06-02T09:21:57Z`;
  - stale `status:in-progress` label removed.
- Git state after #597 close:
  - `HEAD == origin/main == d64c6a2`;
  - `git status --short --branch` read `## main...origin/main` before claiming #598.
- Live queue after #597:
  - #594 parent remains open;
  - #624/#625 remain `status:blocked` on the Daybreak operator-only boundary;
  - unblocked children still open include #598-#604 and #629-#634.
- Active issue is now #598 `scenario(stress): detection + entity tracking on fast-moving scene (pixel_only/hybrid)`.
  - START comment https://github.com/ChrisRoyse/Synapse/issues/598#issuecomment-4600966276;
  - assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
  - Goal: prove RT-DETR/entity tracking under a fast moving visible scene in `pixel_only` and `hybrid`, including stable `track_id`, sane `velocity_px_s`, and `find` by entity/class.
  - Planned SoTs: deterministic visible scene/frame evidence, target process/window/frame state, real MCP `observe` entity output, real MCP `find` entity output, isolated storage/log/process state, and cleanup readbacks.
  - Required edges: object leaves/re-enters frame, confidence threshold floor, max detections cap, black/empty frame, structurally invalid params.
- Next:
  1. Inspect detection/entity tracking implementation and model availability paths.
  2. Determine whether a real configured model/runtime exists; if missing, acquire/setup locally per D4.
  3. Build/launch a repo-built isolated daemon for #598, verify process/socket/auth/health/strict Inspector `tools/list`.
  4. Create deterministic fast-moving visual target and run manual MCP/SoT FSV.

## 2026-06-02T04:19:35-05:00
- Active issue #597 has implementation, manual MCP/SoT FSV, cleanup, and final supporting checks complete; commit/RESOLVED posting/closeout are next.
- Patch accepted for #597:
  - `read_text` honors requested backend;
  - `backend=auto` resolves to WinRT and gets a distinct cache key from explicit `winrt`;
  - `backend=crnn` fails closed with `OCR_BACKEND_UNAVAILABLE` until a real provider is wired;
  - zero/non-positive OCR regions fail before capture;
  - element-id requests re-read live UIA bboxes;
  - omitted-target requests fall back to focused element/window bbox;
  - Windows cache path captures BGRA once, hashes the exact pixels, runs OCR from those bytes, writes `CF_OCR_CACHE`, and validates readback.
- Accepted manual #597 FSV directory: `.runs\597\ocr-fsv-20260602T035659`.
- Repo-built isolated daemon evidence:
  - PID `42136`, bind `127.0.0.1:7870`, binary `target\release\synapse-mcp.exe`;
  - process/socket readbacks passed;
  - unauth `/health` returned `401 HTTP_TOKEN_INVALID`;
  - auth `/health` returned `ok=true`;
  - strict MCP Inspector tools/list via `node ...\@modelcontextprotocol\inspector\cli\build\cli.js` returned 80 tools and required `read_text`, `observe`, `find`, `storage_inspect`, `release_all`.
- OCR target SoTs:
  - deterministic visible Windows Forms target PID `3408`, title `Issue597OcrTarget`, HWND `17573350`;
  - source text file `target\issue597_source_text.json` read back known dense/tiny/multilingual/rapid/occlusion strings;
  - isolated MCP `observe` read back foreground `Issue597OcrTarget` and UIA text boxes/bboxes: dense `237,272,1740,225`, tiny `237,519,1740,72`, multilingual `237,632,1740,203`, rapid `237,887,840,120`, occlude element id `0x1fd0a92:0000002a01fd0a92`.
- Manual OCR/cache evidence:
  - dense `backend=winrt`: `CF_OCR_CACHE 0->1`, OCR text `ISSUE597 DENSE BLOCK 2468fn u32) ->`, cache row readback included requested/effective `winrt`, region, bitmap SHA256, result, and `recognition_latency_ms=15`;
  - repeat dense `backend=winrt`: same text, `CF_OCR_CACHE` stayed `1`, log `OCR_CACHE_HIT`;
  - dense `backend=auto`: same text, `CF_OCR_CACHE 1->2`, log row key separates `auto/winrt`;
  - dense `backend=crnn`: failed closed with `OCR_BACKEND_UNAVAILABLE`, `CF_OCR_CACHE` stayed `2`;
  - tiny `winrt` then `auto`: OCR text `ISSUE597 TINY TEXT 7391 SHALL FONT CACHE`, `CF_OCR_CACHE 2->4`;
  - multilingual `winrt` then `auto`: OCR text contained `ISSUE597 MULTILINGUAL 8642`, `Bonjour monde`, `Guten Tag`, `Cafe naive`, `CF_OCR_CACHE 4->6`;
  - rapid BETA: OCR text `ISSUE597 RAPID BETA 2222`, `CF_OCR_CACHE 6->7`; after switching physical pixels to visible ALPHA, WinRT returned `OCR_NO_TEXT` instead of stale BETA and cache stayed unchanged; switching back to BETA hit the existing cache row;
  - occluded element-id: visible element text `ISSUE597 OCCLUDE VISIBLE 13579` wrote row `7->8`; same element id under overlay returned `ISSUE597 COVER PANEL 24680` and wrote row `8->9`;
  - focused-window fallback: omitted target used focused `Issue597OcrTarget` bbox and wrote full-window OCR row `9->10`.
- Manual fail-closed edges:
  - zero-width region failed with non-empty-region error, cache `9->9`;
  - off-screen region failed with `OCR_NO_TEXT`, cache `9->9`;
  - structurally invalid region missing `h` failed deserialization, cache `9->9`.
- Final runtime readback:
  - final isolated `CF_OCR_CACHE=10`, size `16130`, row samples readable;
  - daemon log recorded `OCR_CACHE_MISS_RECORDED`, `OCR_CACHE_HIT`, CRNN unavailable, zero-region, off-screen/no-text errors;
  - Inspector `release_all` returned zero keys/buttons/pads;
  - cleanup removed target PID `3408`, daemon PID `42136`, and port `7870`.
- Final supporting checks passed:
  - `cargo fmt --check`;
  - `git diff --check` with line-ending warnings only;
  - `cargo test -p synapse-mcp --bin synapse-mcp read_text_ -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp ocr_cache_key -- --nocapture`;
  - `cargo test -p synapse-perception small_screen_ocr_regions_are_upscaled_before_recognition -- --nocapture`;
  - `cargo check -p synapse-perception -p synapse-mcp -j 2`;
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`;
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`;
  - `cargo build --release -p synapse-mcp -j 2`.
- Final release binary readback:
  - `target\release\synapse-mcp.exe`;
  - length `46692864`;
  - SHA256 `11C259BD288FC5C71B50CCB6AA025826BD40428E842E93A4D93D4A351B20F674`;
  - `LastWriteTimeUtc=2026-06-02T09:19:26Z`.
- Next:
  1. Final diff review.
  2. Commit #597 patch/state/docs with `[skip ci]`.
  3. Post #597 RESOLVED evidence, close #597, remove stale `status:in-progress`.
  4. Refresh the live issue queue and continue to #598 unless GitHub changed.

## 2026-06-02T03:54:52-05:00
- Required wake-up after compaction was rerun:
  - read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #351, #594, #597, the live open queue, git status/log/branch;
  - wired `mcp__synapse.health`, `storage_inspect`, `observe`, and `find` all returned through the configured Synapse MCP client.
- Live GitHub queue:
  - #597 remains open, assigned to `ChrisRoyse`, labeled `status:in-progress` and `agent:codex`;
  - #594 parent remains open;
  - #624/#625 remain `status:blocked` on the Daybreak operator-only boundary;
  - unblocked children still open include #598-#604 and #629-#634.
- Git reconciliation:
  - branch `main`;
  - `HEAD == origin/main == ce6f048 docs(state): record issue 597 start [skip ci]`;
  - dirty #597-owned files: `crates/synapse-mcp/src/m1.rs`, `crates/synapse-mcp/src/m1/ocr.rs`, `crates/synapse-mcp/src/server.rs`, `crates/synapse-mcp/src/server/m1_tools.rs`, `crates/synapse-perception/src/lib.rs`, `crates/synapse-perception/src/ocr.rs`, and systemspec docs.
- #597 patch in worktree:
  - `read_text` backend selection is now honored instead of silently ignoring `backend`;
  - `backend=crnn` fails closed with `OCR_BACKEND_UNAVAILABLE` until a real CRNN runtime/model is wired;
  - omitted target falls back to focused window bounds;
  - `element_id` targets re-read live UIA bounds and reject non-positive regions before OCR;
  - Windows OCR can run from one captured BGRA bitmap via `read_text_from_bgra_bitmap`;
  - `read_text` captures pixels once, hashes the captured bitmap, and keys `CF_OCR_CACHE` by requested/effective backend, lang hash, region, bitmap dimensions, and bitmap SHA256;
  - cache hits read/validate the persisted row; cache misses write/read back `OcrCacheRow` and log miss/hit events.
- Supporting checks already passed before this compaction:
  - `cargo fmt`;
  - `cargo test -p synapse-mcp --bin synapse-mcp read_text_ -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp ocr_cache_key -- --nocapture`;
  - `cargo test -p synapse-perception small_screen_ocr_regions_are_upscaled_before_recognition -- --nocapture`;
  - `cargo check -p synapse-perception -p synapse-mcp -j 2`;
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`;
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`;
  - `cargo fmt --check`;
  - `cargo build --release -p synapse-mcp -j 2`.
- Release binary readback from the pre-compaction build:
  - `target\release\synapse-mcp.exe`;
  - SHA256 `9C9F0D85D60E5E7E3ED014E7755193EA434BBCEAABA0B71051038372BB3A6AC0`.
- First isolated #597 daemon launch attempt is not accepted:
  - run dir `.runs\597\ocr-fsv-20260602T035217`;
  - intended bind `127.0.0.1:7869`;
  - recorded process PID `77292` had already exited;
  - stdout/stderr capture was unreliable, so use a fresh run directory/port rather than accepting anything from this attempt.
- Current configured MCP baseline:
  - long-lived stdio daemon reports `ok=true`, active profile `vscode`, HTTP disabled, storage path `C:\Users\hotra\AppData\Local\synapse\db`;
  - configured `CF_OCR_CACHE` row count is `0`;
  - foreground is VS Code `how-to-spot-ai-writing.md - Synapse - Visual Studio Code`, bounds `x=5160,y=306,w=1838,h=656`.
- Next:
  1. Read the #597 diff end-to-end and inspect the failed daemon run artifacts.
  2. Launch a fresh issue-local repo-built HTTP daemon from `target\release\synapse-mcp.exe`.
  3. Verify process/binary/socket, unauth/auth health, strict Inspector `tools/list`, and required tools.
  4. Create deterministic visible OCR target and run manual MCP/SoT FSV for dense/tiny/multilingual/backend/cache plus zero-size, off-screen, occluded element, rapidly-changing region, and structurally invalid params.

## 2026-06-02T03:26:47-05:00
- #596 is closed.
  - Commit: `6051fb3 fix(mcp): reject empty element capture targets (#596) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/596#issuecomment-4600229991
  - Closure readback: `state=CLOSED`, `closedAt=2026-06-02T08:25:48Z`.
  - Stale `status:in-progress` label was removed.
  - `git status --short --branch` after close: `## main...origin/main`.
- Live open queue after #596:
  - #594 parent context remains open.
  - #624/#625 remain `status:blocked` on the Daybreak operator-only boundary.
  - Unblocked children still open include #597-#604 and #629-#634.
- Active issue is now #597 `scenario(stress): OCR torture - read_text dense/tiny/multilingual, winrt vs crnn, cache`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/597#issuecomment-4600240790
  - Assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
  - Goal: prove `read_text` through real MCP triggers on dense code/terminal text, tiny text, multilingual text, backend modes `winrt`/`crnn`/`auto`, and stable-region cache behavior.
  - Planned SoTs: target source text bytes/state files, visible UI/window/region geometry, real MCP `read_text` output, isolated `CF_OCR_CACHE` row counts/samples, daemon log bytes, and timing deltas for unchanged-region cache hits.
  - Required edges: zero-size/empty region, off-screen region, occluded/stale element region, rapidly-changing region/cache invalidation, and structurally invalid params.
- Current wired MCP client sanity check:
  - `mcp__synapse.health` ok, active profile `vscode`, action/reflex/storage healthy;
  - `storage_inspect`, `observe`, and `find query="Visual Studio Code"` all returned through the configured Synapse MCP client.
- Next:
  1. Inspect `read_text`, OCR backend, and OCR cache implementation before editing.
  2. Build/launch a repo-built isolated daemon for #597 and verify process/socket/auth/health/strict Inspector `tools/list`.
  3. Create deterministic visible OCR targets with known source bytes and run manual MCP/SoT FSV.

## 2026-06-02T03:21:12-05:00
- Active issue #596 has code, supporting checks, and manual MCP/SoT FSV evidence ready for commit and RESOLVED posting.
- Patch since HEAD:
  - `crates/synapse-mcp/src/m1.rs` rejects `element_window` targets whose re-resolved UIA bounding rectangle is empty/non-positive before converting the element id to a window HWND.
- Accepted manual #596 FSV evidence:
  - main run directory: `.runs\596\capture-target-fsv-20260602T0310-hiddenfix`;
  - DXGI subrun directory: `.runs\596\capture-target-fsv-20260602T0310-hiddenfix-dxgi`;
  - release binary `target\release\synapse-mcp.exe`, length `46603776`, SHA256 `DE9BEFF453DD5A1C45035A3F5836C6453DC1D5E824B6B2A06F9DCD9C286FAA22`, `LastWriteTimeUtc=2026-06-02T08:03:53Z`.
- Main repo-built daemon evidence:
  - PID `47680`, bind `127.0.0.1:7867`, isolated DB/log/appdata/localappdata, bearer auth env;
  - process/socket/binary SoT readbacks passed;
  - unauth `/health` returned 401 `HTTP_TOKEN_INVALID`;
  - auth `/health` returned ok with inactive capture runtime generation 0 and channel capacity 2;
  - strict MCP Inspector `tools/list` returned exit 0, 80 tools, and required tools present.
- Physical SoTs:
  - deterministic WPF target PID `49008`, title `Issue596CaptureTarget`, old HWND `21042450`;
  - per-monitor-v2 Win32 readback: primary DISPLAY2 `5120x2160` at 150% (`dpi=144`), DISPLAY1 `2560x1440`, DISPLAY3 `2560x1440`;
  - target DWM frame `1332x801`, `GetDpiForWindow=144`;
  - visible edge element id `0x1411512:000000070000bf70001b5068`, automation id `Issue596EdgeElement`, bbox `x=455,y=630,w=390,h=96`.
- Main real Inspector `tools/call` trigger/readback results:
  - primary with `min_update_interval_ms=1`: generation 1, health/observe latest frame `5120x2160`, min interval floored to 16 ms, channel len/cap `2/2`, thread priority `time_critical`;
  - monitor index 0: generation 2, health/observe latest frame `5120x2160`, channel `2/2`;
  - window HWND `21042450`: generation 3, health/observe latest frame `1332x801`, matching DWM physical bounds and proving no 150% DPI double-apply;
  - visible `element_window`: generation 4, mapped to window HWND `21042450`, health/observe latest frame `1332x801`, no hang;
  - switch from quiet window to monitor 1: generation 5, health/observe latest frame `2560x1440`, no hang.
- Main edge cases:
  - invalid monitor `9999` failed with `CAPTURE_TARGET_INVALID: Failed to find the specified monitor`, health stayed generation 5 monitor 1;
  - structurally invalid target kind `bogus` failed deserialization, health stayed generation 5 monitor 1;
  - hidden edge target state `edge_visible=false`; separate `find` still resolved the old id with bbox `0x0`; patched `set_capture_target element_window` failed with the non-empty UI rectangle error, health stayed generation 5 monitor 1;
  - closed HWND: target PID absent and `IsWindow(21042450)=false`; `set_capture_target window` failed with `CAPTURE_TARGET_INVALID: HWND is not a live window`, health stayed generation 5 monitor 1.
- Forced-DXGI evidence:
  - second daemon PID `23940`, bind `127.0.0.1:7868`, `SYNAPSE_CAPTURE_FORCE_DXGI=1`, isolated DB/log/appdata/localappdata;
  - unauth/auth health and strict Inspector `tools/list` passed with 80 tools;
  - health before trigger selected backend `dxgi_duplication`;
  - monitor0 trigger produced generation 1, backend/effective backend `dxgi_duplication`, health/observe latest frame `5120x2160`, channel `2/2`;
  - live VS Code HWND `597092` was valid by Win32; forced-DXGI window target failed with `CAPTURE_TARGET_INVALID: DXGI duplication supports monitor targets only`, health stayed generation 1 monitor0 DXGI.
- Storage/log/cleanup:
  - main isolated storage grew from all zeros to `CF_OBSERVATIONS=5`, `CF_EVENTS=5`, `CF_SESSIONS=1`;
  - DXGI isolated storage ended at `CF_OBSERVATIONS=1`, `CF_EVENTS=1`, `CF_SESSIONS=1`;
  - real `release_all` returned zero held keys/buttons/pads on both daemons;
  - stopped daemons PIDs `47680` and `23940`, verified ports `7867`/`7868` closed and target absent.
- Final supporting checks after cleanup:
  - `cargo fmt --check`;
  - `git diff --check` with line-ending warnings only.
- Next:
  1. Commit scoped changes (`crates/synapse-mcp/src/m1.rs` and `STATE/*`) with `[skip ci]`.
  2. Post #596 RESOLVED evidence, close #596, remove stale `status:in-progress` if needed.
  3. Refresh the live issue queue and continue to the next unblocked open issue.

## 2026-06-02T03:04:07-05:00
- Active issue remains #596 `scenario(stress): capture-target thrash - Graphics->DXGI fallback, multi-monitor, DPI`.
- Reconciled post-compaction state:
  - live GitHub still has #596 open/in-progress with only the START comment;
  - branch `main` is at `2784184 docs(state): record issue 596 start`, which already contains the prior #596 capture-controller/WGC patches and also includes a README change from before this checkpoint;
  - current working tree now has one new dirty code file: `crates/synapse-mcp/src/m1.rs`.
- Manual FSV after the WGC stop-control patch exposed another real #596 defect:
  - hidden/collapsed `element_window` target still re-resolved through UIA but returned bbox `{x=0,y=0,w=0,h=0}`;
  - `set_capture_target` accepted that old element id and switched capture to the owning window, violating the issue edge "element_window for an element that disappeared".
- Root cause:
  - `capture_target_from_param(ElementWindow)` called `synapse_a11y::element_bounding_rect` only as a liveness check and ignored the returned rectangle;
  - Microsoft UIA docs say `BoundingRectangle` is physical screen coordinates, defaults to empty, and Empty/NULL is used when an item is not currently displaying UI; accepting non-positive bounds fails open for hidden UI.
- Patch:
  - `crates/synapse-mcp/src/m1.rs` now validates `element_window` re-resolved bbox is non-empty (`w > 0 && h > 0`) before converting the element id to its HWND;
  - empty, zero-height, negative-width, and negative-height bounds fail closed with `CAPTURE_TARGET_INVALID`;
  - added a focused helper regression `element_window_rect_validation_requires_non_empty_bounds`.
- Supporting checks after this patch:
  - `cargo fmt`;
  - `cargo test -p synapse-mcp --bin synapse-mcp element_window_rect_validation_requires_non_empty_bounds -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp capture_interval_floor -- --nocapture`;
  - `cargo test -p synapse-mcp --bin synapse-mcp inactive_capture_runtime_readback -- --nocapture`;
  - `cargo test -p synapse-capture dxgi_backend_rejects_window_targets_before_thread_spawn -- --nocapture`;
  - `cargo test -p synapse-capture switching_capture_target_stops_previous_session -- --nocapture`;
  - `cargo test -p synapse-capture capture_thread_priority_is_recorded -- --nocapture`;
  - `cargo check -p synapse-core -p synapse-perception -p synapse-capture -p synapse-mcp -j 2`;
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`;
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`;
  - `cargo fmt --check`;
  - `git diff --check` with line-ending warning only;
  - `cargo build --release -p synapse-mcp -j 2`.
- Release binary for upcoming manual FSV:
  - `C:\code\Synapse\target\release\synapse-mcp.exe`;
  - length `46603776`;
  - SHA256 `DE9BEFF453DD5A1C45035A3F5836C6453DC1D5E824B6B2A06F9DCD9C286FAA22`;
  - `LastWriteTimeUtc=2026-06-02T08:03:53Z`.
- Cleanup/setup:
  - stopped stale isolated #596 daemon PID `43768` that held `target\release\synapse-mcp.exe`;
  - verified `127.0.0.1:7866` no longer listens.
- Next:
  1. Launch a fresh isolated repo-built #596 daemon with issue-local DB/logs.
  2. Verify process/binary/socket, unauth/auth health, and strict Inspector `tools/list`.
  3. Rerun manual MCP FSV from clean SoTs: primary/min-floor, monitor0, window, element_window, monitor1, invalid monitor, hidden/disappeared element, structurally invalid target, closed HWND, forced DXGI monitor and DXGI-window reject, final storage/log/process cleanup.

## 2026-06-02T02:27:37-05:00
- Active issue remains #596 `scenario(stress): capture-target thrash - Graphics->DXGI fallback, multi-monitor, DPI`.
- Manual FSV after the latest-frame readback patch exposed a real shutdown defect:
  - isolated #596 daemon PID `30420` / bind `127.0.0.1:7865` became unresponsive during `set_capture_target target=element_window`;
  - the tool call timed out and a separate `/health` read also timed out while the process still owned the socket;
  - accepted evidence before the hang already proved primary/monitor/window capture frame dimensions and 16ms interval floor, but the run is not accepted until the hang is fixed and rerun.
- Root cause:
  - `CaptureController::switch_to` called `previous.stop()` synchronously while M1 held its state mutex;
  - Windows Graphics Capture was started with `GraphicsHandler::start(settings)`, so shutdown was only observed from `on_frame_arrived`;
  - a quiet/static window can emit one frame and then no callback, leaving `join()` blocked and wedging all M1 tools behind the mutex.
- Patch added after the hang:
  - `crates/synapse-capture/src/platform/windows/capture.rs` now starts WGC through `GraphicsHandler::start_free_threaded(settings)`;
  - the outer Synapse capture loop polls the shared stop flag and uses `CaptureControl::stop()` to post `WM_QUIT` to the WGC message-loop thread;
  - handler construction sets and records the actual WGC callback thread priority, preserving the `time_critical` readback.
- Supporting checks after the shutdown patch:
  - `cargo fmt`
  - `cargo check -p synapse-capture -p synapse-mcp -j 2`
  - `cargo test -p synapse-capture switching_capture_target_stops_previous_session -- --nocapture`
  - `cargo test -p synapse-capture dxgi_backend_rejects_window_targets_before_thread_spawn -- --nocapture`
  - `cargo test -p synapse-capture capture_thread_priority_is_recorded -- --nocapture`
- Cleanup:
  - stopped the wedged isolated daemon PID `30420`;
  - `Get-NetTCPConnection` on `127.0.0.1:7865` returned no listener.
- Current worktree:
  - #596 product/model/test files dirty, including the WGC stop-control patch;
  - `README.md` remains unrelated/user-owned and must not be staged.
- Next:
  1. Run final supporting checks and rebuild `target\release\synapse-mcp.exe`.
  2. Launch a fresh isolated #596 daemon and redo process/socket/auth/health/strict Inspector `tools/list`.
  3. Rerun manual MCP FSV from clean SoTs for primary, monitor, window, element_window, multi-monitor, DXGI, invalid monitor, disappeared element, closed HWND, min interval floor, and structurally invalid input.

## 2026-06-02T01:31:06-05:00
- Active issue remains #596 `scenario(stress): capture-target thrash - Graphics->DXGI fallback, multi-monitor, DPI`.
- Code inspection found the root cause for the issue's current untestability:
  - `set_capture_target` only resolved the requested target and updated M1 observation metadata.
  - It did not start or switch `synapse-capture::CaptureController`, so the MCP trigger could not prove the 2-frame capture channel, active backend, frame drops, or DXGI fallback.
  - Monitor targets were not prevalidated, and `element_window` trusted the HWND embedded in a possibly stale `ElementId`.
- Patch currently dirty for #596:
  - `synapse-capture`: `CaptureController::switch_to` starts the new handle before stopping the old one; capture stats record effective backend; monitor validation uses the same Windows monitor-index path as capture; forced DXGI rejects window targets before spawning a doomed thread.
  - `synapse-core`/`synapse-perception`: added optional `CaptureRuntimeReadback` to health/observation diagnostics.
  - `synapse-mcp`: `M1State` owns a real `CaptureController`; `set_capture_target` validates, clamps `min_update_interval_ms` to 16ms, switches the controller, and returns runtime readback; `health` and `observe` expose `capture_runtime`; `element_window` re-resolves through the UIA worker before accepting the target.
- Supporting checks passed so far:
  - `cargo test -p synapse-capture dxgi_backend_rejects_window_targets_before_thread_spawn -- --nocapture`
  - `cargo test -p synapse-mcp capture_interval_floor -- --nocapture`
  - `cargo test -p synapse-mcp inactive_capture_runtime_readback -- --nocapture`
  - `cargo check -p synapse-core -p synapse-perception -p synapse-capture -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`
- Web/source context read via Exa:
  - Microsoft Learn Windows Graphics Capture docs describe capture items/frame pools for windows/displays.
  - Microsoft Learn DXGI Desktop Duplication docs describe monitor/output frame acquisition through `AcquireNextFrame`.
  - Microsoft high-DPI docs state Per-Monitor V2 apps see raw pixels and `GetDpiForWindow` returns per-monitor DPI for per-monitor-aware windows.
- Current worktree:
  - #596 code/model/test files dirty.
  - `README.md` remains unrelated/user-owned and must not be staged.
- Next:
  1. Run final supporting checks/release build after any compile fixes.
  2. Launch an isolated repo-built #596 daemon, verify process/socket/auth/health/strict Inspector tools-list.
  3. Run manual MCP FSV for primary/monitor/window/element_window target cycles plus invalid monitor, closed HWND, disappeared element, min interval floor, structurally invalid input, DXGI monitor path, and available DPI/physical-pixel readbacks.

## 2026-06-02T01:09:41-05:00
- #595 is closed:
  - commit `098e8d5 fix(a11y): stream UIA fanout snapshots (#595) [skip ci]` pushed to `origin/main`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/595#issuecomment-4599193156
  - closure readback: `state=CLOSED`, `closedAt=2026-06-02T06:08:59Z`.
  - stale `status:in-progress` label was removed.
  - cleanup readback showed #595 target, CalculatorApp, isolated daemon, and port `7864` absent.
- Git state after #595:
  - `HEAD == origin/main == 098e8d5`.
  - `git status --short --branch` shows only unrelated/user-owned `README.md` dirty.
- Live open queue after #595:
  - #594 parent remains open.
  - #624/#625 remain `status:blocked` on the Daybreak operator-only boundary.
  - #596-#604 and #629-#634 are open/unblocked.
- Active issue is now #596 `scenario(stress): capture-target thrash - Graphics->DXGI fallback, multi-monitor, DPI`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/596#issuecomment-4599199311
  - assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
  - Goal: prove `set_capture_target` switches cleanly under rapid reconfiguration, the 2-frame capture channel, Graphics-Capture->DXGI fallback, and per-monitor DPI behavior through real MCP triggers and separate SoT readbacks.
  - Planned SoTs: Win32 monitor geometry/DPI, foreground/window/UIA bbox/title, cursor position, Synapse `observe` foreground/diagnostics/capture target output, isolated storage rows, and daemon log bytes.
- Next:
  1. Inspect capture target implementation and prior DPI/fallback fixes (#591/#634 context where relevant).
  2. Build/launch repo-built isolated daemon for #596 and verify process/socket/auth/health/strict `tools/list`.
  3. Run manual MCP FSV for target cycles primary/monitor/window/element_window plus invalid monitor, closed HWND, disappeared element/window, min update interval floor, and DXGI fallback where locally forceable.

## 2026-06-02T01:05:39-05:00
- Active issue remains #595, but implementation/checks/manual evidence are ready for commit and RESOLVED posting.
- Required wake-up was re-run after compaction:
  - read both AICodingAgentSuperPrompt files, `AGENTS.md`, `STATE/*`, #351, #595, the live open queue, and git status/log/branch.
  - wired Synapse MCP `health`, `storage_inspect`, `observe`, and `find` loaded through the configured client; the configured long-lived daemon still took ~28s on `find` against the 10k target, so it was not used as #595 acceptance evidence.
- #595 product patch in `crates/synapse-a11y/src/platform/windows/snapshot.rs`:
  - normal Windows UIA snapshot child collection streams through `UITreeWalker` sibling calls with budget/deadline checks before each child.
  - `find_all_build_cache(TreeScope::Children)` is restricted to known UWP app-frame/CoreWindow classes so #582 remains covered.
  - collection now hard-stops on the internal node budget/deadline before collecting more nodes, and logs truncation instead of silently completing.
  - the raw `File`/`Edit`/`View` supplement is limited to Notepad roots so arbitrary high-fanout windows are not scanned by that workaround.
  - added focused helper regressions for budget/deadline classification and Notepad-only raw supplement gating.
- Manual #595 MCP/SoT evidence accepted under `.runs\595\fanout-fsv-20260602T0037`:
  - repo-built isolated daemon PID `64060`, bind `127.0.0.1:7864`, binary `target\release\synapse-mcp.exe`, isolated DB/logs; socket and auth health readbacks passed, unauth `/health` returned 401, strict MCP Inspector `tools/list` succeeded with 80 tools and #595 tools present.
  - target SoT: deterministic WPF/PowerShell `Issue595FanoutTarget` PID `62812`, 10,000 UIA text descendants, state file and independent UIA readbacks for names/automation ids/bboxes.
  - happy `observe depth=6 max_elements=500` through real Inspector `tools/call` returned bounded element count 184 instead of materializing the 10k tree; separate storage readback moved `CF_EVENTS/CF_OBSERVATIONS` 0->1 and daemon log recorded `A11Y_SNAPSHOT_WALK_TRUNCATED reason="deadline"` with snapshot elapsed ~403ms.
  - happy `find query="Issue595 Item 00042"` through real Inspector returned the exact visible item name/automation id/bbox matching independent UIA readback.
  - `reality_baseline` persisted `CF_KV` baseline/head rows for epoch `issue595-fanout-0037`; after physical target rename to `Issue595 Renamed`, `observe_delta profile_id=powershell` returned 8 deltas and persisted `CF_KV/reality/delta/*` rows.
  - edges covered with before/after SoT reads: `max_elements=1`, no-result `find`, depth-0 boundary, max-elements-0 clamp boundary, structurally invalid unknown param rejection with storage unchanged, minimized-window `find window_hwnd`, and Calculator/UWP `ApplicationFrameWindow` smoke for `CalculatorResults`.
  - CLI empty-query encoding was recorded as an Inspector argument-format caveat, not accepted as server empty-query verdict.
- Final supporting checks after the last edit:
  - `cargo fmt --check`
  - `git diff --check` (line-ending warnings only)
  - `cargo test -p synapse-a11y collection_limit_reason -- --nocapture`
  - `cargo test -p synapse-a11y raw_pattern_supplement -- --nocapture`
  - `cargo check -p synapse-a11y -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`
  - `cargo build --release -p synapse-mcp -j 2`
  - final release binary `target\release\synapse-mcp.exe`: length `46485504`, SHA256 `C5415C7A2153613FC5C9BC654C3ADB99A939F83D7BC2A6FA9F7CF206A41DC57A`, `LastWriteTimeUtc=2026-06-02T06:05:23Z`.
- Cleanup:
  - real Inspector `release_all` against isolated daemon returned zero held keys/buttons/pads.
  - stopped #595 target PID `62812`, CalculatorApp PID `29856`, and isolated daemon PID `64060`.
  - readback shows target absent, daemon absent, `127.0.0.1:7864` closed, CalculatorApp absent; `ApplicationFrameHost` PID `18732` remains as Windows Settings and was preserved.
  - `README.md` remains unrelated/user-owned and must not be staged.
- Next:
  1. Review/stage only `crates/synapse-a11y/src/platform/windows/snapshot.rs` and `STATE/*`, excluding `README.md`.
  2. Commit with `[skip ci]`, push, post #595 RESOLVED evidence, close #595, remove stale in-progress label if needed.
  3. Refresh open queue and continue with the next unblocked issue (#596 unless GitHub changed).

## 2026-06-02T00:36:51-05:00
- Required wake-up was re-run after compaction:
  - read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #351, #595, live open queue, and git status/log/branch.
  - wired Synapse MCP `health`, `storage_inspect`, `observe`, and `find` returned through the configured client.
  - live queue still has #595 active/in-progress, #594 parent open, #624/#625 blocked on the operator-only Daybreak boundary, and #596-#604/#629-#634 open/unblocked.
- #595 manual run after the first patch exposed a second root cause:
  - target `Issue595FanoutTarget` PID `62812` is a deterministic WPF 10k-child target with sentinel `Issue595 Item 03500`.
  - independent UIA readback found `text_descendant_count=10009`, sentinel present, and parent path `Window -> Issue595 Fanout Scroll -> TextBlock`.
  - real MCP `observe depth=6 max_elements=500` and `find query=Issue595 Item 03500` against isolated daemon PID `79940` were not accepted as FSV because they took ~26-27s and returned only shallow nodes/no sentinel.
  - daemon logs showed `A11Y_SNAPSHOT_WALK_TRUNCATED reason="deadline" nodes=25` after `FindAllBuildCache(TreeScope::Children)` had already materialized the 10k-child list.
- #595 patch now goes beyond the first budget guard:
  - `crates/synapse-a11y/src/platform/windows/snapshot.rs` normal child traversal now uses `UITreeWalker::get_first_child_build_cache` / `get_next_sibling_build_cache` and checks budget/deadline before each sibling.
  - bulk `find_all_build_cache(TreeScope::Children)` remains only for known UWP app-frame classes (`ApplicationFrameWindow`, `Windows.UI.Core.CoreWindow`, `ApplicationFrameInputSinkWindow`) to preserve the #582 CoreWindow boundary path.
  - the raw menu supplement is now limited to Notepad root windows, so arbitrary high-fanout foreground trees are not scanned three times for `File`/`Edit`/`View`.
- Supporting checks after the streaming-walker patch:
  - `cargo fmt`
  - `cargo test -p synapse-a11y collection_limit_reason -- --nocapture`
  - `cargo check -p synapse-a11y -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo build --release -p synapse-mcp -j 2`
  - release binary readback: `target\release\synapse-mcp.exe`, length `46485504`, SHA256 `9F7663082D2A417E44B053AD95C79B590B50B0409BFCCE421FF1C616196757E7`, `LastWriteTimeUtc=2026-06-02T05:36:42.1557686Z`.
- Cleanup/setup:
  - stale isolated daemon PID `79940` on `127.0.0.1:7863` was stopped and the socket is closed.
  - live `Issue595FanoutTarget` PID `62812` remains available for patched manual FSV; close it during #595 cleanup.
  - `README.md` remains unrelated/user-owned and must not be staged.
- Next:
  1. Launch a fresh isolated repo-built `synapse-mcp` HTTP daemon for #595.
  2. Verify process/socket/auth/health and strict Inspector `tools/list`.
  3. Redo #595 manual MCP FSV through real `observe`, `find`, `reality_baseline`, `observe_delta`, and `storage_inspect`, with separate target UIA/window/state-file/storage/log readbacks.
  4. Cover happy path plus `max_elements=1`, `depth=6`, empty/no-result, structurally invalid, minimized target, and UWP/CoreWindow smoke.

## 2026-06-02T00:18:00-05:00
- Required post-compaction wake-up was re-run:
  - read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #351, #595, live open queue, and git status/log/branch.
  - wired Synapse MCP client returned through real tools: `health`, `storage_inspect`, `observe`, and `find`.
  - live queue still has #595 active/in-progress, #594 parent open, #624/#625 blocked on the Daybreak operator-only boundary, and #596-#604/#629-#634 open/unblocked.
- #595 implementation finding:
  - `observe` clamps response elements through `observe_include` to `max_elements` 1..500 and `depth` <=6.
  - `find` uses `FIND_SNAPSHOT_DEPTH=16` and returns max 20 hits.
  - `observe_delta` uses stricter `bounded_depth`/`bounded_max_elements` for 1..6 and 1..500.
  - Windows UIA snapshots use an internal `SNAPSHOT_NODE_BUDGET=4000` plus `SNAPSHOT_DEADLINE=400ms`.
  - The real boundedness bug was in `collect_nodes`: after enumerating a large flat child list, it only stopped descending and could still collect every sibling, so a 10k-child UIA list could exceed the internal 4000-node budget before response truncation.
- #595 patch currently dirty in `crates/synapse-a11y/src/platform/windows/snapshot.rs`:
  - added `collection_limit_reason`;
  - stop/flag before collecting any node after budget/deadline;
  - skip child enumeration when the current node will exhaust the budget/deadline;
  - break before remaining siblings once budget/deadline is reached, logging `A11Y_SNAPSHOT_WALK_TRUNCATED`;
  - added focused helper tests for budget and deadline boundaries.
- Supporting checks already run after the patch before this state write:
  - `cargo fmt`
  - `cargo test -p synapse-a11y collection_limit_reason -- --nocapture`
  - `cargo check -p synapse-a11y -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo build --release -p synapse-mcp -j 2`
  - release binary readback from that build: `target\release\synapse-mcp.exe`, length `46479360`, SHA256 `291051081606485F341561FABB67AA44A80E4A179DC2D911B42EB4C90B421B0D`, `LastWriteTimeUtc=2026-06-02T05:10:43.48732Z`.
- Web/source research:
  - Microsoft Learn `IUIAutomationElement::FindAll` warns that broad desktop descendant searches can iterate thousands of items and stack overflow; Microsoft UIA caching docs describe caching as bulk prefetching and advise scoping/cache filtering.
  - This supports keeping Synapse snapshots bounded by explicit budgets and shallow child enumeration.
- Issue-local target artifact created for manual #595 FSV:
  - `.runs\595\fanout-fsv-20260602T0018\target\issue595_target.ps1`
  - WinForms target title `Issue595FanoutTarget`;
  - `ListBox` item counts `0/500/4000/10000`, deterministic item names like `Issue595 Item 03500`, rename to `Issue595 Renamed 03500`, state file readback, selection/minimize controls.
  - This is a deterministic physical UIA target, not product code.
- Current dirty state:
  - `README.md` remains unrelated/user-owned; do not stage.
  - `crates/synapse-a11y/src/platform/windows/snapshot.rs` is #595 product patch.
  - `.runs\595\fanout-fsv-20260602T0018\target\issue595_target.ps1` is an untracked/ignored run artifact for manual evidence.
- Next:
  1. Launch `Issue595FanoutTarget` and read target state/window/UIA SoTs.
  2. Launch isolated repo-built `synapse-mcp` HTTP daemon with issue-local DB/logs.
  3. Verify process/socket/auth/health and strict Inspector `tools/list`.
  4. Run manual MCP FSV through real `observe`, `find`, `reality_baseline`, `observe_delta`, and `storage_inspect` calls with separate target state/UIA/storage/log readbacks.
  5. Cover happy path plus `max_elements=1`, `depth=6`, no-result/empty query, minimized window, structurally invalid params, and a UWP/CoreWindow smoke edge where available.

## 2026-06-02T00:03:13-05:00
- #628 is closed:
  - commit `4991efe fix(mcp): harden browser element actions (#628) [skip ci]` pushed to `origin/main`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/628#issuecomment-4598863144
  - closure readback: `state=CLOSED`, `closedAt=2026-06-02T05:02:28Z`.
  - stale `status:in-progress` label was removed from #628 after closure.
  - #628-owned server/Playwright/target Chrome cleanup was verified; unrelated Chrome PID `30964` was preserved.
- Git status after #628 push/close:
  - `## main...origin/main`
  - only `README.md` remains dirty and is unrelated/user-owned.
- Live open queue after #628 closure:
  - #594 parent context remains open.
  - #624/#625 remain `status:blocked` on the Daybreak operator-only boundary.
  - unblocked children include #595-#604 and #629-#634.
- Active issue is now #595 `scenario(stress): UIA fanout storm - observe/find under 10k+ element trees`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/595#issuecomment-4598866903
  - #595 is assigned to `ChrisRoyse` and labeled `status:in-progress`, `agent:codex`.
  - Goal: prove `observe`, `find`, and `observe_delta` stay correct and bounded against high-fanout UIA trees through real Synapse MCP triggers and separate physical SoT readbacks.
  - Initial SoTs: foreground/window/process state, generated high-fanout UI contents, UIA element/name/count/bbox readbacks, Synapse results, storage rows, and daemon logs.
- Next:
  1. Inspect #595-relevant perception/a11y/reality implementations and prior #615 high-fanout fixes.
  2. Choose deterministic high-fanout local targets that satisfy issue intent without external flakiness.
  3. Build/launch a repo-built isolated daemon, verify process/socket/auth/health/strict `tools/list`, then run #595 manual FSV through real MCP tools.

## 2026-06-02T00:00:05-05:00
- #628 `scenario(showcase): browser marathon - Chrome workflow with Playwright MCP as oracle` has complete manual MCP/SoT evidence, final supporting checks, diff review, and cleanup. Remaining #628 actions are commit/push, RESOLVED comment, close issue, remove stale in-progress label if needed, then refresh the queue.
- #628 patch in this worktree:
  - `crates/synapse-mcp/src/m2/scroll.rs`: `act_scroll.at` now uses a Windows targeted HWND wheel-message path when no recording backend is active. It hit-tests the screen point, verifies the window/root rectangle, chunks signed wheel deltas into `WM_MOUSEWHEEL` / `WM_MOUSEHWHEEL`, and logs `M2_ACT_SCROLL_HWND_MESSAGE`. Non-Windows targeted scroll fails closed as backend unavailable.
  - `crates/synapse-a11y/src/re_resolve.rs`, `platform/windows/resolve.rs`, `platform/windows/mod.rs`, and `platform/non_windows.rs`: added re-resolved UIA `ValuePattern` `SetValue` plus before/after readback for element-targeted typing; non-Windows fails closed.
  - `crates/synapse-mcp/src/m2/type_text.rs` and `server/m2_tools.rs`: `act_type into_element` dispatches through live UIA ValuePattern instead of action-only conversion; immediate UIA mismatch is surfaced with `target_text_integrity=uia_value_pattern_dispatch_only_requires_target_readback`; non-targeted typing still uses normal action preflight.
  - `crates/synapse-mcp/src/server/context.rs`: test struct update for existing `FindParams.window_hwnd`.
  - `README.md` is dirty but unrelated/user-owned and must not be staged for #628.
- Manual #628 FSV precondition:
  - isolated repo-built daemon PID `34424`, bind `127.0.0.1:7862`, DB `.runs\628\browser-marathon-fsv-20260601T1915\db12_scroll_hit_test_clean`.
  - strict Inspector artifacts `431_patched12_health_post_compaction7.txt` and `432_patched12_tools_list_post_compaction7.txt` accepted required tools `act_scroll`, `act_type`, `act_click`, `find`, `observe`, `storage_inspect`, `release_all`, and `health`.
  - final release binary after cleanup/build: `target\release\synapse-mcp.exe`, length `46477312`, LastWriteTimeUtc `2026-06-02T04:55:49.4779223Z`, SHA256 `710ADCF581389D984ED613A7DE3034A623055825A8D743B7368CF1F3F6268530`.
- Manual #628 happy-path evidence:
  - targeted scroll seed evidence moved Playwright DOM `scrollY` `0 -> 1278`; isolated `CF_ACTION_LOG` `0 -> 2`; daemon log emitted `M2_ACT_SCROLL_HWND_MESSAGE`.
  - `act_type into_element` wrote exact Playwright DOM `searchValue="vega"`; isolated `CF_ACTION_LOG` `2 -> 4`; UIA immediate readback mismatch was recorded, so Playwright DOM was the accepted browser typing SoT.
  - full browser marathon artifacts `347` through `437` drove search, late-loaded control, modal save, iframe entry/postMessage, form fill, moved target after scroll, and final submit through real Synapse MCP triggers.
  - final browser/server SoT: `437_happy_after_submit_playwright_corrected_post_compaction7.txt` and `435_happy_after_submit_server_post_compaction7.json` show receipt `M-1` with payload `fullName=Casey Happy`, `email=casey.happy@example.test`, `priority=normal`, `notes=Notes happy path via Synapse MCP`, `searchQuery=vega`, `modalCode=MOD-628-HAPPY`, `iframeCode=IFR-628-HAPPY`, `dynamicReady=true`, and `movedClicks=1`.
  - isolated storage after happy submit: `436_happy_after_submit_storage_post_compaction7.txt` shows `CF_ACTION_LOG=38`.
- Manual #628 edge evidence:
  - empty search: before `440`, server `441`, storage `442` (`CF_ACTION_LOG=38`); trigger `444` clicked app search button; after `445` has `searchStatus="Search term required"` and `errors=["empty-search"]`; server `446` unchanged; storage `447` `CF_ACTION_LOG=40`.
  - boundary search: 256-character synthetic query in `448`; before `451`/`452`/`453`; real type `456` and click `459`; after `460` preserves exact length/value and reports 0 results with no errors; server `461` unchanged; storage `462` `CF_ACTION_LOG=44`.
  - structurally invalid element: before `463`/`464`/`465`; trigger `466` attempted `act_click element_id="0x12068a:bad"` and failed closed with `ACTION_ELEMENT_NOT_RESOLVED`; after `467` DOM unchanged, server `468` unchanged, storage `469` `CF_ACTION_LOG=46` with an error row; excerpt `470`.
- Supporting #628 checks passed:
  - `cargo fmt --check`
  - `git diff --check` (line-ending warnings only)
  - `cargo check -p synapse-a11y -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp targeted_wheel_chunks -- --nocapture`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`
  - `cargo build --release -p synapse-mcp -j 2`
- Cleanup:
  - wired `release_all` returned `released_keys=0`, `released_buttons=0`, `neutralized_pads=0`.
  - isolated daemon PID `34424` was stopped earlier; `474_cleanup_daemon_port_readback_post_compaction7.json` shows process/socket absent.
  - #628-owned Node server PID `79412`, Playwright MCP PID `39204`, and target Chrome PID `63396` were stopped; ports `8763`, `8932`, and `9226` read no listeners. Unrelated Chrome PID `30964` remains alive.
- Next:
  1. Stage only #628 code/state files, excluding unrelated `README.md`.
  2. Commit with `[skip ci]`, push, post #628 RESOLVED evidence, close #628, and remove `status:in-progress` if still present.
  3. Refresh open queue and take the next unblocked issue.

## 2026-06-01T23:06:31-05:00
- Active issue remains #628 `scenario(showcase): browser marathon - Chrome workflow with Playwright MCP as oracle`.
- Required wake-up after compaction was re-run again:
  - read `C:\code\Synapse\docs\AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #628 comments, #351 decision/context, live GitHub open queue, and git status/log/branch.
  - wired Synapse MCP tools returned through the real client: `health`, `storage_inspect`, `observe`, and `find`.
  - live queue still has #628 in progress, #624/#625 blocked on Daybreak operator-only state, and #594/#595-#604/#629-#634 open.
- Current #628 runtime was re-read:
  - local server PID `79412`, `127.0.0.1:8763`.
  - Playwright MCP PID `39204`, `::1:8932`.
  - target Chrome PID `63396`, CDP `127.0.0.1:9226`, main HWND `0x12068a` / decimal `1181322`.
  - unrelated user Chrome PID `30964` is still alive; avoid it and do not close it.
  - isolated repo-built Synapse daemon PID `34424`, bind `127.0.0.1:7862`, DB `.runs\628\browser-marathon-fsv-20260601T1915\db12_scroll_hit_test_clean`.
  - fresh process/socket artifacts: `322_runtime_processes_post_compaction2.json` and `323_runtime_sockets_post_compaction2.json`.
  - fresh strict Inspector artifacts: `324_patched12_health_post_compaction2.txt` and `325_patched12_tools_list_post_compaction2.txt`; required #628 tools remain present.
- Manual SoT evidence for `act_type into_element` exactness has been captured:
  - reset/navigate artifacts: `312_type_exact_reset_server.json`, `313_type_exact_playwright_navigate.txt`, and target-window setup `314_type_exact_raise_target_window.json`.
  - before Playwright DOM `315_type_exact_before_playwright.txt`: `searchValue=""`, `activeId=""`, `results=[]`; before isolated storage `316_type_exact_before_storage.txt`: `CF_ACTION_LOG=2`.
  - real Synapse MCP `find` artifact `317_type_exact_find_search_input.txt` found the target search input in HWND `1181322`.
  - real Synapse MCP trigger `318_type_exact_synapse_act_type_search_vega.txt`: `act_type text=vega into_element=<search input>`.
  - after Playwright DOM `319_type_exact_after_playwright.txt`: `searchValue="vega"`, `searchLength=4`, `activeId="searchInput"`, `results=[]`.
  - after isolated storage `320_type_exact_after_storage.txt`: `CF_ACTION_LOG=4`, with started/ok rows for `act_type`.
  - daemon log `321_type_exact_daemon_log_tail.txt` records a UIA immediate readback mismatch (`after_len=0`, expected `4`), so external Playwright DOM is the accepted SoT for exactness.
- Next:
  1. Reset/reload the #628 page again for a clean full happy path.
  2. Use real Synapse MCP `find`, `act_type`, `act_click`, and `act_scroll` against target HWND `1181322`.
  3. Read Playwright DOM, server `/api/state`, and isolated storage before/after each happy-path segment and edge.
  4. If any click or field action returns success without a matching SoT delta, stop, root-cause, fix, rebuild/relaunch, and rerun the manual SoT loop.

## 2026-06-01T23:00:34-05:00
- Active issue remains #628 `scenario(showcase): browser marathon - Chrome workflow with Playwright MCP as oracle`.
- Required wake-up after compaction was re-run:
  - read `C:\code\Synapse\docs\AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #628 comments, #351 decision/context, live GitHub open queue, and git status/log.
  - wired Synapse MCP tools returned through the real client: `health`, `storage_inspect`, `observe`, and `find`.
  - live queue still has #628 in progress, #624/#625 blocked on Daybreak operator-only state, and #594/#595-#604/#629-#634 open.
- Current #628 runtime:
  - local server PID `79412`, `127.0.0.1:8763`, server SoT after reset has `submissions=[]` and `iframeMessages=[]`.
  - Playwright MCP PID `39204`, `::1:8932`, target page `http://127.0.0.1:8763/`.
  - target Chrome PID `63396`, CDP `127.0.0.1:9226`, main HWND `0x12068a`, title `Issue628 Browser Marathon Target - Google Chrome`.
  - unrelated user Chrome PID `30964` is still alive; avoid it and do not close it.
  - isolated repo-built Synapse daemon PID `34424`, bind `127.0.0.1:7862`, DB `.runs\628\browser-marathon-fsv-20260601T1915\db12_scroll_hit_test_clean`.
  - release binary `target\release\synapse-mcp.exe`: length `46477312`, SHA256 `971EAE444FE3E72FA533C7B7FBAA41A97824A5D149C7E263F6D9FB2BBD0FC301`, `LastWriteTimeUtc=2026-06-02T03:52:19.8116278Z`.
  - fresh strict Inspector artifacts: `298_patched12_health_post_compaction.txt` and `299_patched12_tools_list_post_compaction.txt`; required tools `act_scroll`, `act_type`, `act_click`, `find`, `observe`, `storage_inspect`, `release_all`, and `health` are present.
- #628 current patch state:
  - `crates/synapse-mcp/src/m2/scroll.rs` now targets wheel delivery to the actual HWND hit by `WindowFromPoint` when `act_scroll.at` is provided, with chunked signed wheel deltas and structured `M2_ACT_SCROLL_HWND_MESSAGE` logging.
  - `act_type into_element` and related A11Y/UIA value-pattern code is still dirty and must be manually checked against Playwright DOM exactness before acceptance.
  - `README.md` is modified but unrelated/user-owned; do not stage it unless a later issue requires it.
- Fresh manual SoT evidence for the targeted scroll defect:
  - before reset/read artifacts: `300_scroll_seedfix_reset_server.json`, `301_scroll_seedfix_playwright_navigate.txt`, `302_scroll_seedfix_before_playwright.txt`, and `303_scroll_seedfix_before_storage.txt`.
  - before Playwright DOM showed `scrollY=0`, `bodyHeight=3111`, element at viewport point `(846,686)` was `dynamicStatus`, and moved target rect `y=2623.5417`.
  - before isolated storage showed `CF_ACTION_LOG=0`.
  - initial point read showed the coordinate was under VS Code's Electron renderer, so target Chrome HWND `0x12068a` was raised as setup; post-raise read `307_scroll_seedfix_window_from_point_after_raise.json` showed point `856,696` hit `Chrome_RenderWidgetHostHWND` with root `0x12068a`.
  - real Synapse MCP trigger `308_scroll_seedfix_synapse_act_scroll.txt`: `act_scroll` with `dy=-20`, `at={"x":856,"y":696}`, response `backend_used=software_window_message`, `wheel_event_count=1`.
  - after Playwright DOM `309_scroll_seedfix_after_playwright.txt`: `scrollY=1278`, moved target rect `y=1345.5417`.
  - after isolated storage `310_scroll_seedfix_after_storage.txt`: `CF_ACTION_LOG=2`, with started/ok rows for `act_scroll`.
  - daemon log `311_scroll_seedfix_daemon_log_tail.txt`: `M2_ACT_SCROLL_HWND_MESSAGE` targeted `Chrome_RenderWidgetHostHWND` at `screen_x=856`, `screen_y=696`, `delta=-2400`.
- Next:
  1. Reset/reload the #628 page again.
  2. Re-check `act_type into_element` exactness on the target Chrome page using a known value such as `vega` and Playwright DOM readback.
  3. If exact, run the full #628 happy path plus empty/boundary/structurally-invalid edges through real Synapse MCP triggers and separate server/Playwright/storage SoT reads.
  4. If `act_type` contaminates or appends unexpected text, fix it fail-closed and repeat the manual SoT loop.

## 2026-06-01T21:16:00-05:00
- Active issue remains #628 `scenario(showcase): browser marathon - Chrome workflow with Playwright MCP as oracle`.
- User's `Issue615FanoutTarget` concern was rechecked after compaction from persisted state and live reality:
  - no live `Issue615FanoutTarget` window/button surface is currently present;
  - `.runs\615\target\issue615_target.ps1` confirms those buttons are stale #615 WinForms fixture controls: `Show*` repopulates the item panel, `Clear` empties it, `Rename8` renames existing items, `Mixed8` renames/adds, and `Exit` closes the fixture.
- #628 root cause found so far:
  - Chrome UIA `Invoke` returned success for the late-loaded button but Playwright DOM stayed unchanged, so return value was a false positive.
  - Direct coordinate clicks failed because OS cursor APIs cannot currently move the cursor from the foreground PowerShell context; separate readback showed `SetPhysicalCursorPos`/`SetCursorPos` false/no-error and Synapse `act_aim` failed the same way.
  - DPI readback exposed a scaled-coordinate mismatch on the 150% Chrome monitor path, so cursor movement now verifies physical cursor position and fails closed on mismatch instead of claiming success.
  - A diagnostic-only Win32 `PostMessageW` sequence to Chrome's `Chrome_RenderWidgetHostHWND` did trigger the page's late button, proving Chrome can accept a window-message fallback when physical cursor movement is unavailable. This diagnosis is not accepted as FSV until triggered through real Synapse MCP `act_click`.
- Current local #628 patch:
  - `crates/synapse-action/src/backend/software/mouse.rs`: physical cursor movement now has separate `GetPhysicalCursorPos` readback, monitor-DPI compensation, SendInput fallback readback, and explicit `ACTION_BACKEND_UNAVAILABLE` on mismatch.
  - `crates/synapse-mcp/src/m2/click.rs` and `crates/synapse-mcp/src/m2/click/element.rs`: element clicks with `use_invoke_pattern=false` route through the normal coordinate actor path first, then fall back to Windows HWND mouse messages only after the action backend reports unavailable.
- Supporting checks now passed after the latest patch:
  - `cargo fmt`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo check -p synapse-action -j 2`
  - `cargo test -p synapse-mcp direct_coordinate_element_click_uses_move_then_requested_presses -- --nocapture`
  - `cargo test -p synapse-action dpi_compensation -- --nocapture`
  - `cargo test -p synapse-action cursor_readback_tolerance -- --nocapture`
  - `cargo fmt --check`
  - `cargo build --release -p synapse-mcp -j 2`
- Latest release binary readback: `target\release\synapse-mcp.exe`, length `46379008`, SHA256 `42FB209D71E8D2F6967D0F82D1B6A6EE70422B98361489ADCCDCD14F3F4258D1`, `LastWriteTimeUtc=2026-06-02T02:14:58.1557599Z`.
- The old #628 isolated daemon PID `56124` on `127.0.0.1:7857` was released/stopped; port `7857` readback returned no listener.
- Next:
  1. launch a fresh isolated repo-built #628 daemon on a new port with the latest release binary;
  2. verify process/socket/auth/health/strict Inspector `tools/list`;
  3. reset the local browser marathon page/server state;
  4. run manual MCP FSV for same-page Synapse `find`/`act_click`/`act_type`/`act_scroll` triggers with Playwright DOM/server/state readbacks.

## 2026-06-01T19:13:00-05:00
- #627 is closed:
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/627#issuecomment-4597519110
  - closed at `2026-06-02T00:11:22Z`
  - pushed commit `c3b83b2 fix(a11y): handle Office RuntimeId fallback (#627) [skip ci]`
  - `status:in-progress` label was removed from the closed issue after closure.
  - Worktree readback after push was clean at `## main...origin/main`.
- Active issue is now #628 `scenario(showcase): browser marathon - Chrome workflow with Playwright MCP as oracle`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/628#issuecomment-4597523219
  - #628 is assigned to `ChrisRoyse` and labeled `status:in-progress`, `agent:codex`.
  - Goal: drive deterministic Chrome workflow through real Synapse MCP `observe`/`find`/`act_click`/`act_type`/`act_scroll`, and use Playwright as independent DOM oracle.
  - Required edges: dynamic/late-loading elements, element moved between observe/click by scroll/DPI movement, modal dialog, iframe content, plus empty/boundary/structurally-invalid inputs.
  - SoTs: browser DOM/page state via Playwright oracle, Synapse UI/read_text/observe readbacks, any local server/page state, and `CF_ACTION_LOG`.
  - Next: inspect browser/action/profile/Playwright surfaces and verify local Playwright MCP/runtime availability or acquire/setup reversible prerequisites.

## 2026-06-01T18:59:20-05:00
- #627 manual Excel workbook evidence is now complete through the save/readback step.
  - Isolated repo-built MCP daemon remains PID `34556`, bind `127.0.0.1:7855`, binary `target\release\synapse-mcp.exe`, SHA256 `24757F067CBDBE4E5871BDCAB44DF735A47C1788CD53E126D4680B358032B245`.
  - Strict MCP Inspector post-compaction `tools/list` and `health` succeeded; required tools `health`, `observe`, `find`, `act_click`, `act_press`, `act_type`, `act_clipboard`, `read_text`, `storage_inspect`, and `release_all` are present.
  - Classic Excel `Save As` was handled through real MCP `find`/`act_click`/`act_press`/`act_clipboard`; workbook saved to `.runs\627\excel-runtime-check-20260601T1810\issue627-self-driving-spreadsheet.xlsx`.
  - Physical file SoT before save: target absent. After save: target exists, length `22526`, LastWriteTimeUtc `2026-06-01T23:55:42.9399586Z`.
  - Independent shared-read `.xlsx` byte/package readback:
    - SHA256 `D3F696164FE3835A1E7C12C9E7F58821CBC08D52FDB64D7C9553340108AD567E`
    - sheet dimension `A1:M257`
    - expected formula values present: `E2=36`, `E3=27`, `E4=16`, `B5=20`, `C5=26`, `D5=33`, `E5=79`
    - formula error edge present: `G2` formula `1/0`, value `#DIV/0!`
    - large paste edge present: 256 `Bulk*` rows from `J2:J257`, with `J257=Bulk256`, `K257=256`, `L257=512`, `M257=768`
    - chart SoT present: `xl/charts/chart1.xml`, drawing relationship `xl/drawings/drawing1.xml`, chart relationships target `../charts/chart1.xml`, chart formulas reference `Sheet1!$A$2:$A$5` and `Sheet1!$B$2:$E$5`.
- #627 cleanup and supporting checks are complete:
  - real Inspector `release_all` returned zero held keys/buttons/pads.
  - real Inspector `act_press alt+f4` closed Excel; process readback found Excel PID `78020` absent.
  - isolated daemon PID `34556` was stopped; port `127.0.0.1:7855` readback returned no listener.
  - supporting checks passed: `cargo fmt --check`, `cargo check -p synapse-a11y -j 2`, `cargo check -p synapse-mcp -j 2`, `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`, `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`, `cargo build --release -p synapse-mcp -j 2`, and `git diff --check` with line-ending warnings only.
  - final release binary `target\release\synapse-mcp.exe` length `46396416`, LastWriteTimeUtc `2026-06-02T00:09:15.8502522Z`, SHA256 `3FF17F523F900368D486863AA5EED573F8D3616DF2FE87E998330026D5557462`.
- #627 remaining work: post RESOLVED evidence to #627, close it, commit/push with `[skip ci]`, then continue the open queue.

## 2026-06-01T18:35:41-05:00
- Post-compaction wake-up was re-run:
  - read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, live GitHub queue, #351 decision/context, #627 issue/comments, and git status/log/branch.
  - Wired Synapse MCP `health`, `storage_inspect`, and `reflex_list` succeeded; wired `find` still fails on Excel with `cached RuntimeId had unexpected type EMPTY`, matching the active #627 Windows UIA defect being fixed in the local patch.
  - Open queue readback: #627 active/in-progress; #624/#625 blocked on Daybreak operator-only boundary; #594/#595-#604 and #628-#634 remain open.
- User asked about `Issue615FanoutTarget` windows with `Clear`, `Show4`, `Show7`, `Show8`, `Rename8`, `Mixed8`, `Show80`, and `Exit` buttons.
  - OS process/window readback found no visible `Issue615`/fanout top-level windows alive now.
  - `.runs\615\target\issue615_target.ps1` confirms those buttons are temporary #615 WinForms UIA fanout stress-fixture controls: `Show*` populates the item panel, `Clear` empties it, `Rename8` renames existing item buttons, `Mixed8` renames/adds item buttons, and `Exit` closes the fixture.
  - If seen again, treat as stale #615 fixture residue and close it; it is not product UI.
- Active implementation remains #627. Local worktree has the Windows UIA RuntimeId/re-resolution patch in:
  - `crates/synapse-a11y/src/platform/windows/common.rs`
  - `crates/synapse-a11y/src/platform/windows/resolve.rs`
  - `crates/synapse-a11y/src/platform/windows/snapshot.rs`

## 2026-06-01T17:50:00-05:00
- #626 is closed:
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/626#issuecomment-4597095341
  - Closure readback: `state=CLOSED`, `closedAt=2026-06-01T22:44:50Z`.
  - State/evidence commit pushed: `9382bd2 docs(state): record issue 626 evidence [skip ci]`.
  - Worktree readback after push: `## main...origin/main`.
- Active issue is now #627 `scenario(showcase): self-driving spreadsheet - launch Excel, build, verify file`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/627#issuecomment-4597099075
  - #627 is assigned to `ChrisRoyse` and labeled `status:in-progress`, `agent:codex`.
  - Goal: use real Synapse MCP tools to launch Excel, enter known data/formulas, create/verify spreadsheet content and chart, save an issue-local `.xlsx`, then independently read file bytes/worksheet/formula values as SoT.
  - Required edges: formula error cell, large paste/boundary input, undo/redo, save-dialog handling, and empty/boundary/structurally-invalid tool inputs.
  - Local Excel prerequisite exists:
    - `C:\Program Files\Microsoft Office\root\Office16\EXCEL.EXE`
    - length `75917120`, `LastWriteTimeUtc=2026-05-17T00:11:53Z`
    - App Paths registry entries point to the same Office16 executable.
  - Wired Synapse MCP is healthy via stdio: process PID `66040` plus stdio child `70072`, active storage initialized, operator hotkey registered.
  - Next: inspect relevant action/launch/profile/spreadsheet verification surfaces, then run #627 manual FSV with real MCP triggers and separate Excel/file SoT reads.

## 2026-06-01T17:45:00-05:00
- #626 manual evidence is complete; no product-code patch was required.
  - Active issue: `scenario(showcase): autonomous pianist - act_combo song verified by audio_tail`.
  - Evidence directory: `.runs\626\pianist-fsv-20260601T1709`.
  - Deterministic local browser target: `.runs\626\piano-target\index.html` served on `127.0.0.1:8762` during FSV; target was stopped during cleanup.
  - Isolated repo-built MCP daemon:
    - PID `79620`, bind `127.0.0.1:7854`, DB `.runs\626\pianist-fsv-20260601T1709\db`, `--enable-audio`, `SYNAPSE_AUDIO_LOOPBACK=true`.
    - Auth health readback showed `audio loopback running`, active profile `chrome`, and isolated DB.
    - Official MCP Inspector strict `tools/list` after compaction returned `ToolCount=80`; required tools `health`, `act_launch`, `act_combo`, `act_press`, `act_click`, `audio_tail`, `observe`, `read_text`, `storage_inspect`, and `release_all` were present.
  - Happy-path visual/audio evidence:
    - `14_act_combo_happy_ode.json`: `scheduled_steps=15`, `backend=software`.
    - `15_read_text_after_happy.json`: page showed `Audio notes: 15`, `Play count: 15`, `Muted notes: 0`, `Wrong keys: 0`, and melody `E4 E4 F4 G4 G4 F4 E4 D4 C4 C4 D4 E4 E4 D4 D4`.
    - First late `audio_tail` read returned valid all-zero PCM because the 5-second ring had already aged out; repeated with overlapped playback.
    - `19_act_combo_long_ode48.json`: `scheduled_steps=48`; `19_audio_tail_mid_long_ode48.json` reduced readback showed `format=s16le`, `sample_rate=48000`, `channels=2`, `pcm_bytes=960000`, `peak=5809`, `rms_db=-33.3`, and 49 active 50 ms buckets from about `1750..4900 ms`.
    - `20_read_text_after_long_ode48.json`: page showed `Audio notes: 48`, `Play count: 48`, `Muted notes: 0`, `Wrong keys: 0`, and repeated Ode motif.
  - Edge evidence:
    - Empty combo: `21_act_combo_empty_steps.stderr.txt` failed closed with `act_combo steps must contain at least one step`; page/storage readbacks stayed unchanged.
    - Structurally invalid order: `22_act_combo_nonmonotonic.stderr.txt` failed closed with `act_combo steps[1].at_ms must be monotonic`; page/storage readbacks stayed unchanged.
    - Muted/silent: `25_read_text_muted_baseline.json` showed muted clean baseline and `25_audio_tail_muted_baseline.json` had 192000 bytes with zero nonzero samples; `26_act_combo_muted4.json` scheduled 4 steps; `27_read_text_after_muted4.json` showed `Play count: 4`, `Muted notes: 4`, `Audio notes: 0`, melody `C4 D4 E4 F4`; `27_audio_tail_after_muted4.json` stayed zero nonzero samples.
    - Wrong-key recovery: `31_act_press_wrong_x.json` sent unmapped `x`; `32_read_text_after_wrong_x.json` showed `Last event: wrong key x`, `Play count: 0`, `Melody: empty`; `33_act_combo_recovery_c4.json` scheduled a C4 recovery note; `34_read_text_after_recovery_c4.json` showed `Last event: C4 recovered after x`, `Play count: 1`, `Audio notes: 1`, `Melody: C4`.
    - Back-to-back combos: `36_act_combo_backtoback_first.json` and `37_act_combo_backtoback_second.json` each scheduled 3 steps; `38_read_text_after_backtoback.json` showed `Play count: 6`, `Audio notes: 6`, `Wrong keys: 0`, melody `C4 D4 E4 G4 F4 E4`.
    - 256-step boundary/tempo: Inspector CLI hit Windows command-line length for the large payload, so the wired production MCP client was used for this one boundary trigger. Wired daemon PID `66040` plus stdio child `70072` was healthy; `mcp__synapse.act_combo` accepted `scheduled_steps=256`. Separate `mcp__synapse.read_text` showed `Play count: 256`, `Muted notes: 256`, `Audio notes: 0`, `Wrong keys: 0`, `Last note: C4`; wired `storage_inspect` showed `CF_ACTION_LOG=188`, `CF_REFLEX_AUDIT=5`, with action row `scheduled_steps=256` and reflex audit active->expired for combo id `019e8551-9c75-7680-9925-085d07519433`.
  - Cleanup:
    - Isolated Inspector `release_all` returned zero released inputs; wired `mcp__synapse.release_all` returned zero released inputs.
    - Stopped #626-owned Chrome profile children, isolated MCP PID `79620`, and Python server PID `51064`.
    - Port readback for `127.0.0.1:7854` and `127.0.0.1:8762` returned no listeners; Synapse `find` returned no `Issue626PianoTarget` or `Issue615FanoutTarget`.
  - Supporting checks passed:
    - `cargo fmt --check`
    - `cargo test -p synapse-mcp --test m3_audio_tail_tool -- --nocapture`
    - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`
    - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
    - `cargo check -p synapse-mcp -j 2`
    - `cargo build --release -p synapse-mcp -j 2` after an initial timeout was waited out and rerun for a clean exit code
    - `git diff --check`
  - Release binary readback: `target\release\synapse-mcp.exe`, length `46392320`, SHA256 `FC4003D69AA84712112DEBC3534F113B15F89E69046E23D4064D01CFFAECBE4F`, `LastWriteTimeUtc=2026-06-01T22:42:23.6432547Z`.
  - Current worktree readback remains clean: `## main...origin/main`.
  - Next: post #626 RESOLVED evidence, close #626, refresh the queue, and continue to the next open issue.

## 2026-06-01T17:00:00-05:00
- #625 is now posted and labeled blocked:
  - BLOCKED evidence: https://github.com/ChrisRoyse/Synapse/issues/625#issuecomment-4596839011
  - Label readback shows `status:blocked`; `status:in-progress` was removed.
  - State commit pushed: `0c854e8 docs(state): record issue 625 block [skip ci]`.
  - Worktree was clean after push.
- Active issue is now #626 `scenario(showcase): autonomous pianist - act_combo song verified by audio_tail`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/626#issuecomment-4596846733
  - #626 is assigned to `ChrisRoyse` and labeled `status:in-progress`, `agent:codex`.
  - Goal: use real `act_launch`/Chrome navigation, `act_press`/`act_combo`, `audio_tail`, and `observe` to prove a recognizable piano melody with audio and visual readbacks.
  - Required edges from issue body: tempo at combo step limit, silent/muted audio confirms no output, wrong-key recovery, back-to-back combos, plus empty/boundary/structurally invalid inputs.
  - Current wired Synapse MCP `health` reports audio disabled in the stdio runtime, so #626 likely needs an isolated repo-built daemon launched with `--enable-audio` before audio FSV can be accepted.
  - Next: inspect audio/combo/launch/Chrome/profile/observe implementations and tests, then launch an audio-enabled isolated repo-built MCP daemon with strict tools-list/client parity.

## 2026-06-01T16:56:00-05:00
- User's `Issue615FanoutTarget` window/button concern was rechecked after compaction:
  - OS process/window readback found no live `Issue615` or fanout process/window.
  - Wired Synapse MCP `find` returned no `Issue615FanoutTarget`, `Show80`, `Rename8`, `Mixed8`, `Clear`, or `Exit` elements.
  - Foreground is VS Code; no fixture is visible.
  - `.runs\615\target\issue615_target.ps1` shows the buttons are temporary #615 WinForms UIA stress-fixture handlers: `Clear` clears `ItemPanel`, `Show4/7/8/80` populate item buttons, `Rename8` renames existing buttons only, `Mixed8` renames/adds buttons, and `Exit` closes the fixture.
- Active issue #625 has all reversible/safe manual MCP evidence completed; product code did not require a patch.
  - Wired `synapse-mcp` runtime is live through the configured MCP client:
    - `mcp__synapse.health` read `ok=true`, active profile `vscode`, storage path `C:\Users\hotra\AppData\Local\synapse\db`, operator hotkey registered, 29 profiles, and active storage.
    - Process SoT readback found `synapse-mcp.exe` PID `66040` at `C:\Users\hotra\.cargo\bin\synapse-mcp.exe` plus stdio child PID `70072`; the wired client loaded and successfully called the #625 tools.
    - `observe`, `storage_inspect`, `reflex_list`, and `find` all returned through the real wired MCP client after compaction.
  - Physical EQ log SoT stayed unchanged throughout #625:
    - `C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\Logs\eqlog_Thenumberone_frostreaver.txt`
    - length `2464677`, SHA256 `E563074084A7F5A291AC6FBF77746B993AB086F747C6C111C39503B6BF475368`, `LastWriteTimeUtc=2026-05-30T23:26:54Z`.
  - Safe readiness/autocombat gate evidence:
    - `everquest_survival_readiness` persisted `everquest/survival_readiness/v1/everquest.live/latest` with blockers `foreground_not_everquest`, `gameplay_ui_not_proven`, `chat_input_not_safe`, `hud_hp_mana_unavailable`, and `food_drink_absent`.
    - `everquest_current_state` persisted `everquest/current_state/v1/everquest.live`; foreground was VS Code and zone was last-known `nektulos` from the EQ log.
    - `everquest_autocombat issue625-autocombat-deny-vscode` failed closed with `ACTION_TARGET_INVALID active_profile_mismatch`; separate `CF_ACTION_LOG` readback advanced `180 -> 181` and the latest action row records `status=denied`, `tool=everquest_autocombat`, `run_id=issue625-autocombat-deny-vscode`.
  - Synthetic storage/model chain evidence:
    - `everquest_domain_normalize issue625-synth-combat-spell` persisted DynamicJEPA domain/state/action/outcome/transition rows.
    - `everquest_trajectory_record issue625-synth-trajectory` persisted `everquest/trajectory/v1/everquest.live/issue625-synth-trajectory` and exported JSONL SHA256 `FD359802391CF76E9126EEDAEF49CFF29B18CA3669F90AC641CF3A48382A591B`.
    - `everquest_predictive_model_fit issue625-model` persisted model row `everquest/predictive_model/v1/everquest.live/issue625-model`, status `trained`, model hash `286c033af9422dc870e43302c96cf5380c60122fcf7b29122bbcd29ea9b0427c`.
    - `everquest_predictive_model_predict issue625-predict-combat-spell` persisted prediction row `everquest/prediction/v1/everquest.live/issue625-predict-combat-spell`, decision `predict`, selected `combat_spell`, confidence `1.0`, outcome `combat_death`.
  - Surprise evidence:
    - Structurally invalid source ref field `note` failed closed with `TOOL_PARAMS_INVALID`, leaving `CF_KV=41` and no row.
    - Confirmed expected outcome row `everquest/surprise/v1/everquest.live/issue625-surprise-confirmed`: decision `expected_outcome_confirmed`, `surprise_detected=false`, payload SHA256 `4649b69c5f3e64087b0406d6858dd21ddf410cc1b22cd4196f81e22ad84c768b`.
    - Mismatch row `everquest/surprise/v1/everquest.live/issue625-surprise-mismatch`: decision `surprise_detected`, mismatch reasons `zone_short_name_mismatch` and `outcome_kind_mismatch`, payload SHA256 `5d10ba461900ef53689def48d69e5b22d148a722232b5192f14481218a948235`.
    - Missing-prediction row `everquest/surprise/v1/everquest.live/issue625-surprise-missing-prediction`: decision `abstain_missing_prediction`, reason `prediction_missing`, payload SHA256 `e6f09372e381bbb08c44b491efd7dacb40892e60a644d343413182b69c9205d8`.
  - Action-prior and scorecard evidence:
    - `issue625-actionprior-correct` row class `correct_top1`, top1/top3/zone/coord/hazard correctness true, confidence bucket `0.80-1.00`.
    - `issue625-actionprior-low-confidence` row class `correct_top1`, confidence bucket `0.40-0.60`.
    - `issue625-actionprior-abstain` row class `abstained`, `abstained=true`, confidence bucket `0.20-0.40`.
    - `everquest_action_prior_scorecard issue625-scorecard-window` advanced `CF_KV 47 -> 48` and persisted `everquest/action_prior_scorecard/v1/everquest.live/issue625-scorecard-window`.
    - Scorecard metrics readback: `sample_count=3`, `evaluated_count=2`, `abstention_count=1`, `low_confidence_action_count=1`, top1/top3/useful accuracy `1.0`, competence status `low_confidence_action_forced`, `meets_minimum_floor=false`.
    - Duplicate sample IDs edge for `issue625-scorecard-duplicate-invalid` failed closed with `TOOL_PARAMS_INVALID`; separate storage readback stayed `CF_KV=48` and no invalid row bytes were found.
  - Final supporting checks passed:
    - `cargo fmt --check`
    - `cargo test -p synapse-mcp scorecard --bin synapse-mcp -- --nocapture` (4 passed)
    - `cargo test -p synapse-mcp predictive_model --bin synapse-mcp -- --nocapture` (6 passed)
    - `cargo test -p synapse-mcp surprise --bin synapse-mcp -- --nocapture` (4 passed)
    - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture` (3 passed)
    - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture` (1 passed)
    - `cargo check -p synapse-mcp -j 2`
    - `cargo build --release -p synapse-mcp -j 2`
    - `git diff --check`
  - Release binary readback: `target\release\synapse-mcp.exe`, length `46392320`, SHA256 `4AF3EB0E332F6A7AFD5DBBFAD1169EB051371040D5C24CF033662AC3615F78AD`, `LastWriteTimeUtc=2026-06-01T21:55:14Z`.
- #625 should now be marked `status:blocked`, not resolved:
  - Completed all reversible safe work and storage/model evidence.
  - Remaining required live sustained autocombat soak, HUD HP/mana readback, live combat/xp log deltas, death/respawn, out-of-mana, con-classifier, and readiness-blocker-mid-loop cases require the operator to personally review/respond to the Daybreak EULA/account agreement, log in/select character if appropriate, and put `Thenumberone` in a visible in-world state with safe target availability.
  - The agent must not click legal/account/login/character-select/chat controls.

## 2026-06-01T16:31:30-05:00
- #624 is open with `status:blocked` on a specific operator-only action.
  - BLOCKED evidence comment: https://github.com/ChrisRoyse/Synapse/issues/624#issuecomment-4596661903
  - Commit pushed: `9de5ee3 fix(mcp): guard EverQuest account gates (#624) [skip ci]`.
  - Issue labels read back include `status:blocked`; `status:in-progress` was removed.
  - Worktree was clean after the push.
- Active issue is now #625 `scenario(stress): EverQuest autocombat soak + survival/predictive/surprise/scorecard`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/625#issuecomment-4596668371
  - #625 is assigned to `ChrisRoyse` and labeled `status:in-progress`, `agent:codex`.
  - Live foreground readback is VS Code (`profile_id=vscode`), not EverQuest; active wired Synapse MCP `health` is ok.
- #625 acceptance requires real MCP triggers and separate SoT readbacks for:
  - `everquest_survival_readiness`;
  - `everquest_autocombat` sustained/safe target loop;
  - `everquest_predictive_model_fit` / `everquest_predictive_model_predict`;
  - `everquest_surprise_detect`;
  - `everquest_action_prior_record` / `everquest_action_prior_scorecard`;
  - SoTs: EQ log combat/xp bytes, HUD HP/mana readback, `CF_KV/everquest/*`, and `CF_ACTION_LOG`.
- Current working assumption for #625:
  - Live autocombat itself is likely gated by the same Daybreak EULA/account operator-only action as #624.
  - Continue reversible/safe work first: inspect implementations/tests/docs, verify readiness denial, and exercise predictive/surprise/action-prior storage paths with known synthetic rows if available.

## 2026-06-01T16:24:00-05:00
- Required wake-up context was re-read after compaction:
  - `C:\code\Synapse\docs\AICodingAgentSuperPrompt.md`
  - `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`
  - `AGENTS.md`
  - `STATE/ACTIVE_OBJECTIVE.md`, `STATE/CURRENT_STATE.md`, `STATE/RECOVERY_NOTES.md`, `STATE/DECISION_LOG.md` tail, and `STATE/HEARTBEAT.md` tail.
  - Live GitHub open queue, #624 body/comments, and #351 decision/context comments.
  - `git status`, `git log -10`, branch, wired Synapse `health`, `reflex_list`, `storage_inspect`, `observe`, and `find`.
- User's `Issue615FanoutTarget` concern was rechecked again:
  - wired Synapse `find` returned no `Issue615FanoutTarget` or `Show80`/`Rename8`/`Mixed8` elements;
  - foreground is VS Code; no live #615 fixture is visible.
  - The #615 buttons are only the old WinForms UIA stress fixture controls; close any leaked copy if it appears again.
- Active issue remains #624 `scenario(stress): EverQuest full loop - perception->memory->planner->trajectory->ContextGraph`.
  - #624 is open, assigned to `ChrisRoyse`, labeled `status:in-progress`, `agent:codex`.
  - Live open queue: #594 plus #595-#604 and #624-#634.
  - Git readback: branch `main`, HEAD `841679c`, worktree modified in `STATE/*`, `everquest_ui_context.rs`, `everquest.live.toml`, and two EverQuest docs.
- #624 ContextGraph bridge evidence is now complete for the synthetic episode:
  - Warm `everquest_contextgraph_ingest` through the wired Synapse MCP client stored one episode in ContextGraph storage under `.runs\624\contextgraph-bridge-wired-20260601T1610\data\storage`.
  - Ingest row key persisted in active Synapse storage: `everquest/contextgraph_ingest/v1/everquest.live/7386a7f8b26cd6fc8e262813eff9167785d13610aaf8e68bbd9fcce3949dc2ef/issue624-synth-trajectory.issue624-synth-consider`.
  - Ingest fingerprint `d5d91675-9303-4b0f-bdd6-2f0326abffdb`, memory content SHA256 `a0efd146176e2f72f2f7718661c7a20b506cd7ef19753a956c065158869714bd`, ContextGraph audit operation `MemoryCreated` success.
  - Real wired MCP `everquest_contextgraph_search` with `search_id=issue624-synth-search-wired-warm` returned one cited result for the same fingerprint and export SHA256 `7386a7f8b26cd6fc8e262813eff9167785d13610aaf8e68bbd9fcce3949dc2ef`.
  - Separate storage readback: `CF_KV` advanced `20 -> 21` after search and `rg -a` found `everquest/contextgraph_search/v1/everquest.live/issue624-synth-search-wired-warm` with `result_count=1`, `citation_count=1`, and matching citation.
  - Separate ContextGraph storage readback after search shows new SST/log/manifest files, including `000028.sst..000036.sst`, `MANIFEST-000024`, and `LOG` SHA256 `FF68150590233C0E101CAD5D071EEC8AD08A81061429B7F95429CD85A9FAB72E`.
- #624 safe/read-only EverQuest storage chain evidence is complete in active Synapse storage:
  - `everquest_current_state` persisted `everquest/current_state/v1/everquest.live` with warning hazards `non_everquest_foreground`, `ui_context_ambiguous`, `location_unknown`, `level_unknown`, and `xp_percent_unknown`; foreground was VS Code.
  - `everquest_chat_input_state` returned `deny_foreground_not_everquest`.
  - `everquest_map_sensor` with synthetic state override persisted `everquest/map_sensor/v1/everquest.live/issue624-synth-map-sensor` and abstained because EverQuest was not foreground.
  - `everquest_outcome_ingest` read physical log bytes from `C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\Logs\eqlog_Thenumberone_frostreaver.txt`, offsets `2464000..2464677`, and persisted four compact outcome rows.
  - EQ log physical SoT stayed length `2464677`, SHA256 `E563074084A7F5A291AC6FBF77746B993AB086F747C6C111C39503B6BF475368`, `LastWriteTimeUtc=2026-05-30T23:26:54.5595084Z`.
  - `everquest_memory_record` persisted hazard `issue624-feeble-poison-hazard` and safe-area `issue624-neriaka-safe-area`.
  - `everquest_memory_consult issue624-route-consult` scanned two memories and returned `decision=avoid` because the hazard matched the synthetic route location.
  - `everquest_planner_guard issue624-chat-text-deny` persisted `decision=reject` for critical guards `foreground_everquest_live` and `chat_input_safe` with synthetic unsent chat text.
  - `everquest_route_plan issue624-synth-route-plan` read `maps\nektulos.txt` and produced a bounded route to static label `To_Neriak` with `movement_executed=false`.
  - Physical map SoT readback: `nektulos.txt` length `450424`, `To_Neriak` at line `5974`: `P 1001.4131, -1798.6160, 23.2596,  0, 0, 0,  3,  To_Neriak`.
  - `everquest_world_model_record` persisted `everquest/transition/v1/everquest.live/issue624-synth-nektulos-to-neriak-route`; `everquest_world_model_inspect` read it back with payload SHA256 `7e63089f7357168445fc0e7d633945be6693b136fb4fb34e1f4b1aef08e39cd4`.
  - `everquest_world_summary issue624-synth-world-summary` persisted blocked compact state with nearest exits/landmarks from the real map file and a critical `operator_account_gate` hazard.
  - Separate storage readback after the safe chain: `CF_KV=33`; direct DB-byte search found all expected `issue624-*` row keys.
- #624 edge readbacks captured:
  - EULA/account gate: isolated repo-built daemon denied `act_keymap inventory` and `everquest_loc_probe`; EQ log length/hash unchanged.
  - Non-EverQuest foreground: `everquest_current_state` and `everquest_chat_input_state` reported VS Code foreground and denied chat/input safety.
  - Visible unsent chat text: `everquest_planner_guard` with `chat_input_override.text_present=true` rejected before any action.
  - Structurally-invalid planner source ref: `everquest_planner_guard` failed closed on disallowed `row_key` field.
  - Missing/disabled log: `everquest_outcome_ingest` failed closed on absent valid-shaped path `eqlog_Missingissue624_frostreaver.txt` with `STORAGE_READ_FAILED`; `Test-Path` read `false`; `CF_KV` stayed `33`.
  - Delta reality audit for `profile_id=everquest.live` failed closed because observed foreground profile was `vscode`, not `everquest.live`.
- Operator-only boundary remains:
  - EverQuest is blocked at the Daybreak EULA/account agreement. The agent must not click `I AGREE`, `I DECLINE`, login, character select, or account/chat controls.
  - Full in-world `/loc`, map, route, and movement happy path cannot be completed until the operator personally reviews/responds to the agreement and puts the character in-world.
- Cleanup and final supporting checks:
  - Real Inspector `release_all` on isolated daemon `127.0.0.1:7853` returned `released_keys=0`, `released_buttons=0`, `neutralized_pads=0`.
  - Isolated #624 daemon PID `34624` was stopped; process and port `127.0.0.1:7853` readbacks returned no rows.
  - Passed: `cargo fmt --check`; `cargo test -p synapse-mcp ui_context_detects_ --bin synapse-mcp -- --nocapture`; `cargo test -p synapse-profiles bundled_everquest_profile_targets_inventory_panel --test parse_bundled -- --nocapture`; `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`; `cargo test -p synapse-mcp --test m3_tools_list -- --nocapture`; `cargo check -p synapse-mcp -j 2`; `scripts\check_docs.ps1`; `cargo build --release -p synapse-mcp -j 2`; `git diff --check` with line-ending warnings only.
  - Release binary readback: `target\release\synapse-mcp.exe`, length `46392320`, SHA256 `31D62B2891F4AA17F7139BF4A5E52276521F7009E7B2C428D6FAFF15CBF5A374`, `LastWriteTimeUtc=2026-06-01T21:26:54.5016686Z`.
- Next actions:
  - Commit with `[skip ci]`, post #624 evidence, mark #624 blocked on the exact operator-only action, then continue the open issue queue.

## 2026-06-01T16:02:28-05:00
- Required wake-up context was re-read after compaction:
  - `C:\code\Synapse\docs\AICodingAgentSuperPrompt.md`
  - `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`
  - `AGENTS.md`
  - `STATE/ACTIVE_OBJECTIVE.md`, `STATE/CURRENT_STATE.md`, `STATE/RECOVERY_NOTES.md`, `STATE/DECISION_LOG.md` tail, and `STATE/HEARTBEAT.md` tail.
  - Live open GitHub queue, #624 body/comments, and #351 decision/context comments.
  - `git status`, `git log -10`, and branch readback.
- User asked again about `Issue615FanoutTarget` windows/buttons appearing inert.
  - Fresh OS process/window readback found no live `Issue615`/fanout window.
  - Fresh wired Synapse MCP readback found no `Issue615FanoutTarget`, `Show80`, `Rename8`, or `Mixed8` elements.
  - Foreground is EverQuest, not the #615 WinForms target.
  - `.runs\615\target\issue615_target.ps1` confirms `Clear`, `Show4/7/8/80`, `Rename8`, and `Mixed8` only mutate the fixture's in-window `ItemPanel`; `Exit` closes the fixture. It is not Synapse product UI, and a leaked copy may be closed if seen again.
- Git state:
  - branch `main`
  - `HEAD`: `841679c docs(state): record issue 624 start [skip ci]`
  - `git status --short --branch`: `## main...origin/main` with four modified files:
    - `crates/synapse-mcp/src/server/everquest_ui_context.rs`
    - `crates/synapse-profiles/profiles/everquest.live.toml`
    - `docs/computergames/05_mcp_tool_surface.md`
    - `docs/computergames/26_everquest_live_eval.md`
- Active issue remains #624 `scenario(stress): EverQuest full loop - perception->memory->planner->trajectory->ContextGraph`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/624#issuecomment-4596141027
  - #624 is open, assigned to `ChrisRoyse`, and labeled `status:in-progress`, `agent:codex`.
  - Live open queue: #594 plus #595-#604 and #624-#634.
- #624 patch verified in worktree:
  - `everquest.login_screen_text` regex now recognizes EULA, end-user license agreement, terms of service, privacy policy, I Agree, and I Decline as account-gate signals.
  - `everquest_ui_context` denial now names `everquest_login_or_account_gate_visible` and records compact signal names for `eula_agreement`, `terms_or_privacy_policy`, `agree_button`, and `decline_button`.
  - Docs now state the EverQuest login/account gate is not in-world and raw account/legal text must not be persisted.
- #624 isolated repo-built daemon verified:
  - PID `34624`, process `synapse-mcp-runtime.exe`, bind `127.0.0.1:7853`.
  - Binary path `.runs\624\eula-guard-fsv-20260601T2034\bin\synapse-mcp-runtime.exe`, SHA256 `3BA384BF72EC44DC1106235A4809CEDCEBFB056353527FEA57B6D109C14E3AB7`.
  - Official MCP Inspector strict `tools/list` readback found `tool_count=80`, `missing=[]`, and all #624 tools present.
- #624 EULA/account-gate manual MCP evidence verified on disk:
  - Run directory `.runs\624\eula-guard-fsv-20260601T2034`.
  - `observe` saw foreground `eqgame.exe` with `everquest.login_screen_text.parsed="EULA"` and account buttons present.
  - `everquest_survival_readiness` persisted `everquest/survival_readiness/v1/everquest.live/latest` with blockers `login_screen_visible`, `chat_input_not_safe`, `hud_hp_mana_unavailable`, and `food_drink_absent`.
  - `act_keymap inventory` and `everquest_loc_probe` were denied through real MCP tools with `SAFETY_PROFILE_ACTION_DENIED` and reason `everquest_login_or_account_gate_visible`; separate storage readbacks show action-log denial rows.
  - EQ log SoT length stayed `2464677` and SHA256 stayed `E563074084A7F5A291AC6FBF77746B993AB086F747C6C111C39503B6BF475368`, proving no `/loc` or gameplay/chat input was sent while the account gate was visible.
  - `everquest_current_state` persisted `everquest/current_state/v1/everquest.live` with blocker hazard `login_screen_visible`; log-derived world fields are last-known only.
- #624 synthetic storage/episode chain evidence verified on disk:
  - `everquest_domain_normalize` wrote accepted DynamicJEPA state/action/outcome/transition/domain-pack rows for `issue624-synth-consider`.
  - `everquest_trajectory_record` wrote `everquest/trajectory/v1/everquest.live/issue624-synth-trajectory` and JSONL trajectory file SHA256 `793da367ab3d810f92df76ce553fae9052dcf15da7be88c5da37bfafac988db6`.
  - `everquest_episode_export` wrote `C:\Users\hotra\AppData\Local\synapse\everquest\contextgraph_episodes\everquest.live\issue624-synth-export.jsonl`, length `10746`, SHA256 `7386a7f8b26cd6fc8e262813eff9167785d13610aaf8e68bbd9fcce3949dc2ef`, one compact redacted episode, no raw chat persisted.
- ContextGraph prerequisite status:
  - ContextGraph binary was rebuilt in WSL with CUDA 13.2 and direct strict Inspector `tools/list --no-warm` previously passed with 216 tools.
  - Wrapper paths are `.runs\issue529\context-graph-mcp-wsl.cmd` and `.runs\issue529\context-graph-mcp-wsl.sh`.
  - The first #624 `everquest_contextgraph_ingest` with storage under data root initialized ContextGraph but failed closed at `store_memory`: `Embedding models are still loading. Please wait and try again.`
  - Next action is rerun `everquest_contextgraph_ingest` with `no_warm=false`, storage below data root, and a longer timeout so the real bridge waits for model warm-up instead of using the no-warm path.
- EverQuest host state remains an operator-only external/account decision:
  - Foreground EQ is on the Daybreak EULA/account agreement with `I DECLINE` and `I AGREE`.
  - The agent must not click agreement/decline/login controls. Full in-world #624 `/loc`, map, planner, and route happy path cannot proceed until the operator personally reviews/responds to the agreement and puts the character in-world.

## 2026-06-01T15:16:27-05:00
- Required wake-up context was re-read after compaction:
  - `C:\code\Synapse\docs\AICodingAgentSuperPrompt.md`
  - `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`
  - `AGENTS.md`
  - `STATE/ACTIVE_OBJECTIVE.md`, `STATE/CURRENT_STATE.md`, `STATE/RECOVERY_NOTES.md`, `STATE/DECISION_LOG.md` tail, and `STATE/HEARTBEAT.md` tail.
  - GitHub open queue, #351 decision/context issue, decision/context issue lists, #623 closure state, and #624 body/comments.
  - `git status`, `git log -10`, and branch readback.
- User asked about `Issue615FanoutTarget` windows/buttons (`Clear`, `Show4`, `Show7`, `Show8`, `Rename8`, `Mixed8`, `Show80`, `Exit`) appearing inert.
  - Live process/window readback found no `Issue615`/fanout windows.
  - Wired Synapse MCP readback found no `Issue615FanoutTarget` or `Show80` elements; foreground is VS Code.
  - The fixture source is `.runs\615\target\issue615_target.ps1`: it is a temporary WinForms UIA stress target, not product UI. Button behavior is: `Clear` removes item buttons, `Show4/7/8/80` populate item buttons, `Rename8` renames existing item buttons, `Mixed8` renames up to four and adds new buttons through eight, and `Exit` closes the form.
- Git state after wake-up:
  - branch `main`
  - `git status --short --branch`: `## main...origin/main`
  - `HEAD`: `c4c3b14 docs(state): record issue 623 evidence [skip ci]`
- Wired MCP client sanity check passed:
  - `mcp__synapse.health` returned `ok=true`, active profile `vscode`, storage path `C:\Users\hotra\AppData\Local\synapse\db`, reflex runtime initialized, operator hotkey registered.
  - `observe` saw VS Code focused and A11Y/capture healthy.
  - `storage_inspect` and `reflex_list include_expired=true` returned normally.
- #623 is closed.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/623#issuecomment-4596117663
  - Closure readback: state `CLOSED`, closed at `2026-06-01T20:13:12Z`.
  - Evidence/state commit already on `origin/main`: `c4c3b14 docs(state): record issue 623 evidence [skip ci]`.
- Live open queue after #623 closure contains #594 plus #595-#604 and #624-#634.
- Active issue is now #624 `scenario(stress): EverQuest full loop - perception->memory->planner->trajectory->ContextGraph`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/624#issuecomment-4596141027
  - Claimed with `status:in-progress`, `agent:codex`, assigned to `ChrisRoyse`.
  - #624 requires real MCP triggers and separate SoT readbacks across `everquest_chat_input_state`, `everquest_loc_probe`, `everquest_current_state`, `everquest_map_sensor`, `everquest_outcome_ingest`, `everquest_memory_record`, `everquest_memory_consult`, `everquest_planner_guard`, `everquest_route_plan`, `everquest_domain_normalize`, `everquest_trajectory_record`, `everquest_episode_export`, `everquest_contextgraph_ingest/search`, `everquest_world_model_record/inspect`, and `everquest_world_summary`.
  - Required physical SoTs: EQ log bytes/offsets, `UI_*.ini` layout, local `maps/*.txt`, episode JSONL bytes/hash, ContextGraph storage/provenance, and persisted `CF_KV/everquest/*` / world-model rows.
  - Required edges: login/non-EQ foreground denial, visible unsent chat text gate fail, disabled/missing logging, stale state, empty/boundary/structurally invalid params.
  - Next: inspect EverQuest MCP tool implementations, log/map/layout readers, ContextGraph bridge, supporting tests, and host EverQuest runtime state before launching or configuring an isolated repo-built daemon for #624 manual MCP FSV.

## 2026-06-01T15:03:43-05:00
- Active issue #623 `scenario(stress): audit consent + bundle redaction + replay_record` has manual MCP FSV behavior evidence and final supporting checks complete; commit, RESOLVED comment, closure, and queue continuation are next.
- Worktree changes for #623 are documentation corrections only:
  - `docs/computergames/05_mcp_tool_surface.md`: `replay_record` now documents the actual `duration_ms`/`target`/`format`/`path` schema, replay root path rules, response fields, and fail-closed inputs.
  - `docs/systemspec/13_mcp_tool_reference.md`: `ReplayRecordResponse` now includes `observations_skipped`.
  - `docs/computergames/06_data_schemas.md`: supporting docs-check fix adds the missing `REFLEX_DEBOUNCED` error-code entry.
- Audit/export FSV run directory: `.runs\623\audit-replay-fsv-20260601T1445`.
  - Repo-built daemon PID `38756`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, bind `127.0.0.1:7851`, isolated DB `.runs\623\audit-replay-fsv-20260601T1445\db`, isolated token `synapse-623-token`.
  - MCP precondition passed: process/socket/auth readback; unauth `/health=401`; auth `/health=200`; official MCP Inspector strict `tools/list` exited 0 with 80 tools and #623 tools present. Inspector stderr had only `unknown format "uint*"` warnings.
  - Export without consent failed closed with `audit export requires an enabled local consent row`; storage stayed empty and output dir absent.
  - `audit_export_consent_set profile_id=vscode enabled=true redaction_policy=strict` wrote consent row `CF_KV/audit_export/v1/consent/vscode`; separate `storage_inspect` read `CF_KV=1` and row fields `enabled=true`, allowed `["strict"]`, `external_sharing_allowed=false`, note `issue623 manual consent strict local-only`.
  - Real audit activity came from real MCP `observe` and `act_press keys=["shift"] hold_ms=20`; separate storage readback moved `CF_ACTION_LOG=0 -> 2`, `CF_EVENTS=1`, `CF_OBSERVATIONS=1`, `CF_SESSIONS=1`.
  - Synthetic sensitive audit rows were inserted through real MCP `storage_put_probe_rows` into `CF_ACTION_LOG` to make redaction verdicts deterministic. Raw SoT before export contained known markers `ISSUE623_SECRET_TEXT_MARKER`, `ISSUE623_TOKEN_MARKER`, `ISSUE623_SECRET_WINDOW_MARKER`, `ISSUE623_CONTEXT_TITLE_MARKER`, `issue623-user@example.invalid`, and `C:\Users\hotra\secret\issue623`.
  - Happy `audit_export_bundle` wrote `.runs\623\audit-replay-fsv-20260601T1445\exports\happy` with 7 rows scanned/exported and 90 fields redacted.
    - `manifest.json` length 2169, SHA256 `329FD52280770C941008A26E6C44C8352FB89C3108ABEA62090A568142D30CAC`.
    - `rows.json` length 10600, SHA256 `1099D371C32B72CE2326BA751D06BD973F50A1001140232F787199D561F5950C`.
    - `redaction_report.json` length 927, SHA256 `716D862AC76FE5FE30C3273202AD905063A4B4E7B99717D705C8F52417CCAF6B`.
    - Direct file scan found zero raw marker hits. Redaction report read `rows_redacted=7`, `fields_redacted=90`, classes `high_cardinality_id=31`, `path=26`, `text=5`, `timing=7`, `user_identifier=5`, `window_title=16`.
  - Audit/export edges passed: redaction policy `none` failed closed; `max_rows=1` exported exactly one row and response/file hash matched `sha256:35a00c1926d9055460c55bb1d4a05b0eebcfeced918cd80ec673284f22d2723b`; small `max_row_bytes=100` failed closed before writing; empty `output_path` failed closed.
- Replay/event FSV run directory: `.runs\623\replay-events-fsv-20260601T1457`.
  - Repo-built daemon PID `11076`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, bind `127.0.0.1:7852`, isolated DB `.runs\623\replay-events-fsv-20260601T1457\db`, isolated token `synapse-623-events-token`, and `SYNAPSE_HTTP_SSE_MANUAL=1` for manual event publication into the daemon's EventBus.
  - MCP precondition passed: process/socket/auth readback; unauth `/health=401`; auth `/health=200`; official MCP Inspector strict `tools/list` exited 0 with 80 tools including `replay_record`, `storage_inspect`, and `subscribe`.
  - `replay_record target=both format=jsonl duration_ms=3500 path=issue623-both-manual-event-3.jsonl` ran through real Inspector `tools/call` while five known events were POSTed to `/events`. Publish readback showed seq `6231457005..6231457007` matched and queued.
  - Direct replay JSONL SoT readback: `issue623-both-manual-event-3.jsonl`, 23295 bytes, SHA256 `1AE400B7A81EAF3BA99FDA510299EFD8A7CB4A11778F624FC64A24FAF5FE9F31`, 7 lines: 4 `observation` records and 3 `event` records. Event seqs `6231457005`, `6231457006`, `6231457007`, kind `issue623.replay_event`, markers `issue623-event-marker-6231457005..7007`.
  - Replay `duration_ms=0` edge wrote `issue623-empty.jsonl`, 0 bytes, SHA256 `E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855`, response `records_written=0`, `observations_skipped=0`.
  - Replay invalid edges failed closed and wrote no files: `target=bogus`, `format=csv`, empty `path`, and traversal `path=..\issue623-outside.jsonl`. Final replay dir contained only the happy and empty JSONL files; final isolated storage counts stayed all zero.
  - Cleanup: real Inspector `release_all` returned zero held keys/buttons/pads on both #623 daemons; PIDs `38756` and `11076` stopped; ports `7851` and `7852` closed. Log scan found no panic/internal-error matches; only expected operator-hotkey unavailable messages from running isolated daemons alongside the active chat runtime.
- Final supporting checks passed:
  - `cargo fmt --check`
  - `git diff --check` (line-ending warnings only)
  - `scripts\check_docs.ps1`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --test m3_replay_record_tool -- --nocapture`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo test -p synapse-mcp --test m3_tools_list -- --nocapture`
  - `cargo build --release -p synapse-mcp -j 2`
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46406144`, SHA256 `498E3164F4B795E0ABD3A9E7E2AE678810D532F84B35E5381456277C13628476`, `LastWriteTimeUtc=2026-06-01T20:11:10.6731953Z`.
- Current next actions: commit with `[skip ci]`, post #623 RESOLVED evidence, close #623, refresh the queue, and claim the next issue.

## 2026-06-01T14:31:53-05:00
- #622 `scenario(stress): authoring loop - generate/accept/reject/export + quality_refresh` is closed.
  - No product-code patch was required.
  - State/evidence commit: `9c855fc docs(state): record issue 622 evidence [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/622#issuecomment-4595815302
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T19:31:05Z`.
  - Final release binary SHA256 from #622 supporting build: `236992450A49D3177C1FCBF1D06F567C30CC54AA5F217C1F0D59BFDBADF23E01`.
- Active issue is now #623 `scenario(stress): audit consent + bundle redaction + replay_record`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/623#issuecomment-4595820271
  - #623 is assigned to `ChrisRoyse` and labeled `status:in-progress`, `agent:codex`.
  - #623 requires real MCP triggers and separate SoT readbacks for `audit_export_consent_set`, consent `CF_KV` row, refused export without consent, audit activity generation, `audit_export_bundle` output manifest/rows/redaction report files and SHA256 hashes, redaction guarantees/no raw sensitive payloads, max row/byte caps, `replay_record` JSONL counts/records, `duration_ms=0`, `target=both`, and empty/boundary/structurally invalid inputs.
  - Next: inspect audit consent/export, replay recording, storage row formats, redaction policy enforcement, bundle file layout, cap handling, and existing tests before launching an isolated repo-built daemon.

## 2026-06-01T14:28:42-05:00
- Active issue #622 `scenario(stress): authoring loop - generate/accept/reject/export + quality_refresh` has manual MCP FSV and supporting checks complete; no product-code patch was required.
- Manual FSV run directory: `.runs\622\authoring-fsv-20260601T1350`.
  - Repo-built daemon: PID `59440`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, bind `127.0.0.1:7850`, isolated DB `.runs\622\authoring-fsv-20260601T1350\db`, profile dir `.runs\622\authoring-fsv-20260601T1350\profiles`, token `synapse-622-token`.
  - MCP precondition/readback passed: process path matched the repo release binary; socket `127.0.0.1:7850` listened under PID `59440`; authenticated `/health` returned `ok=true` with active profile `issue622.authoring`; official MCP Inspector strict `tools/list` exited 0 with 80 tools and all #622 tools present. Inspector stderr contained only `unknown format "uint*"` schema warnings, not schema rejection.
  - Initial isolated SoT: storage started empty; after profile activation and before the zero-evidence edge, separate storage readback showed `CF_PROFILES=0`, `CF_ACTION_LOG=0`, `CF_OBSERVATIONS=0`, `CF_EVENTS=1`, `CF_KV=0`; profile `issue622.authoring` was active in health/profile_list readbacks.
  - Zero-evidence edge: `profile_authoring_generate candidate_id=issue622.zero max_audit_rows=0 max_replay_rows=0` failed closed with insufficient evidence; separate `profile_authoring_inspect` found no row and storage stayed `CF_PROFILES=0`.
  - Real evidence production: real Inspector `observe`, `act_clipboard write`, `act_press keys=[shift]`, `replay_record`, and `reality_baseline` produced physical evidence; separate readbacks showed `CF_ACTION_LOG=2`, `CF_OBSERVATIONS=2`, `CF_EVENTS=3`, `CF_KV=2`, and replay file SHA256 `61AB2CC29986048235197AA336CCC34B86F9794445683C72223FE53AE6BABC1F`.
  - Happy authoring path: `profile_authoring_generate candidate_id=issue622.accept` scanned 2 action rows and 1 replay row, wrote `CF_PROFILES/profile_authoring/v1/candidate/issue622.accept`, and proposed `matches.add_exe=["powershell.exe"]`; separate inspect/list/storage readbacks confirmed the candidate row.
  - Accept path: `profile_authoring_accept issue622.accept` wrote state `accepted`, `accepted_at_ns=1780340769774384800`, and note `issue622 manual accept`; re-accepting the same candidate returned `wrote_row=false` and separate readbacks showed the row unchanged.
  - Export path: `profile_authoring_export issue622.accept` wrote `.runs\622\authoring-fsv-20260601T1350\exports\issue622.accept.json`, 2883 bytes, SHA256 `D2790BD9118B9DB5790C4B56D382EA3872146688AD7057FA59EA23427AF9E37B`; parsed file readback showed candidate `issue622.accept`, state `accepted`, and `matches.add_exe=["powershell.exe"]`.
  - Reject path: `profile_authoring_generate candidate_id=issue622.reject` then `profile_authoring_reject reason="issue622 reject reason"` wrote state `rejected`; separate inspect/storage readbacks showed `CF_PROFILES=2`, accepted + rejected candidate rows, and the stored rejection reason.
  - Edge coverage: rejecting the accepted candidate failed closed and left it accepted; exporting missing `issue622.missing` failed closed and wrote no file; `profile_authoring_list limit=0` failed closed; malformed candidate id `bad/slash` failed closed; `max_audit_rows=10001` failed closed and no `issue622.overmax` row was written.
  - 10k boundary: real `storage_put_probe_rows` inserted 10000 synthetic `CF_ACTION_LOG` rows (`2 -> 10002`); `profile_authoring_generate candidate_id=issue622.max max_audit_rows=10000` scanned/relevant 10000 rows and wrote a candidate row with `matches.add_exe=["powershell.exe"]`; separate inspect/storage readbacks showed `CF_PROFILES=3` and `CF_ACTION_LOG=10002`.
  - Quality refresh happy path: before report had zero quality snapshots; `profile_quality_refresh profile_id=issue622.authoring max_audit_rows=50000 stale_after_ns=86400000000000` wrote `CF_PROFILES/profile_quality/v1/issue622.authoring`; separate storage/report readbacks showed `CF_PROFILES=4`, one quality snapshot, score `21`, sample size `1`, action rows scanned `10002`, profile-relevant action rows `2`, observation rows `2`, event rows `3`.
  - Quality edges: `stale_after_ns=1` rewrote the persisted quality row with score `0`, sample size `0`, `audit_rows_stale=2`, and `stale_evidence_present=true`; invalid `max_audit_rows=0` and `stale_after_ns=0` failed closed and left storage unchanged. Final non-stale refresh restored the quality row to score `21`, sample size `1`, `audit_rows_stale=0`.
  - Cleanup: real Inspector `release_all` returned zero held keys/buttons/pads; daemon PID `59440` was stopped; port `127.0.0.1:7850` no longer listens. Log scan found no panic/internal-error lines; the only error lines were the discarded first-start broad shell regex config failure and expected operator-hotkey collision with the active chat runtime.
- Supporting checks passed:
  - `cargo fmt --check`
  - `git diff --check`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --test m5_profile_quality_tool -- --nocapture`
  - `cargo test -p synapse-mcp --test m3_replay_record_tool -- --nocapture`
  - `cargo test -p synapse-mcp profile_authoring -- --nocapture` (compiled; no matching tests)
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo test -p synapse-mcp --test m3_tools_list -- --nocapture`
  - `cargo build --release -p synapse-mcp -j 2`
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46406144`, SHA256 `236992450A49D3177C1FCBF1D06F567C30CC54AA5F217C1F0D59BFDBADF23E01`, `LastWriteTimeUtc=2026-06-01T19:28:18Z`.
- Next: commit state update with `[skip ci]`, post #622 RESOLVED evidence, close #622, refresh queue, and take the next open issue unless GitHub changes.

## 2026-06-01T13:43:30-05:00
- #621 `scenario(stress): registry scale - install/search/export/import/rollback, digest, poison quarantine` is closed.
  - No product-code patch was required.
  - State/evidence commit: `f9ab56e docs(state): record issue 621 evidence [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/621#issuecomment-4595473988
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T18:42:45Z`.
  - Final release binary SHA256 from #621 supporting build: `08FEC90BE80C37B940AF9549335F901A8DACE52863FDA9F7990049F0A4A94890`.
- Active issue is now #622 `scenario(stress): authoring loop - generate/accept/reject/export + quality_refresh`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/622#issuecomment-4595477096
  - #622 is assigned to `ChrisRoyse` and labeled `status:in-progress`, `agent:codex`.
  - #622 requires real MCP triggers and separate SoT readbacks for real action/observation/event evidence, `profile_authoring_generate`, candidate list/inspect, accept one candidate, reject another with reason, export bundle with SHA256, `profile_quality_refresh`, zero-evidence generation, accept-already-accepted, stale-after expiry, max authoring rows, and empty/boundary/structurally invalid inputs.
  - Next: inspect profile authoring, quality refresh, replay, and evidence-row implementations/tests before launching an isolated repo-built daemon.

## 2026-06-01T13:41:30-05:00
- Active issue #621 `scenario(stress): registry scale - install/search/export/import/rollback, digest, poison quarantine` has manual MCP FSV complete; no product-code patch was required.
- Manual FSV run directory: `.runs\621\registry-fsv-20260601T1324`.
  - Repo-built daemon: PID `58848`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, bind `127.0.0.1:7849`, isolated DB `.runs\621\registry-fsv-20260601T1324\db`, isolated token file `.runs\621\registry-fsv-20260601T1324\appdata\synapse\token.txt`, token `synapse-621-token`.
  - Precondition/readback passed: process path and command line matched repo release binary; socket `127.0.0.1:7849` listened under PID `58848`; unauth `/health=401`; auth `/health=200 ok=true`; official MCP Inspector `0.21.2` strict `tools/list` exited 0 with 80 tools and all profile registry tools + `storage_inspect` present. Inspector stderr only had `unknown format "uint*"` warnings.
  - Initial SoT: isolated `storage_inspect` read `CF_PROFILES=0`, `CF_KV=0`; `profile_registry_report` scanned 0 registry rows and pointed at the isolated DB path.
  - Happy install with digest: real Inspector `profile_registry_install` on `curated_notepad_package_manifest.toml` with expected digest `sha256:f173036bcc58401a5eff5a539c74cae20b6e714829e84249b67571b19eaa6cd6` wrote 6 `CF_PROFILES` rows and 1 `CF_KV` head row. Separate storage readback: `CF_PROFILES=6`, `CF_KV=1`; separate inspect readbacks confirmed package, installed profile, and head rows with active/local-validated state.
  - Digest mismatch edge: same manifest with wrong digest failed closed with `manifest digest mismatch`; before/after storage stayed `CF_PROFILES=6`, `CF_KV=1`, `CF_ACTION_LOG=0`.
  - Scale import/search/report: synthetic setup bundle with 600 known registry rows was imported through real `profile_registry_import`; storage moved `CF_PROFILES=6 -> 606`, `CF_KV=1`; `profile_registry_search query=issue621.synthetic row_kind=profile_package limit=1000` returned all 600; `profile_registry_report limit=1000` scanned 606 rows and returned 601 package summaries.
  - Export/import round trip: real `profile_registry_export limit=1000` wrote `.runs\621\registry-fsv-20260601T1324\exports\registry-export-after-scale.json`, 607 rows, 489118 bytes, deterministic hash `sha256:e7c953e5a31ee4d5fc60ac6aa1561543fa0510a76b7730d76013204402d3788f`; file SHA256 `DA50F010913B3240CD1A956BC9219721BBD09C098C4B04C8BD5255F00DE0341C`. Re-import skipped 607 duplicates and left storage unchanged.
  - Conflict import edge: same-key modified bundle failed closed with `registry_bundle_conflict`; before/after storage stayed `CF_PROFILES=606`, `CF_KV=1`.
  - Disable/inspect edge: `profile_registry_disable profile_id=notepad state=disabled` rewrote installed row; separate inspect read `state=disabled`, `activation_state=disabled`, `disable_reason=issue621-disable-edge`, counts unchanged.
  - Rollback: synthetic Notepad package `0.2.0`/profile `1.1.0` installed with expected digest `sha256:a1e012640d9ccb6c1d6853a2cde42992abece7577f89f3d0b0c692aaa07c709e`; installed row moved to `0.2.0`; real `profile_registry_rollback profile_id=notepad` rewrote installed row back to `0.1.0`, wrote `profile_registry/v1/rollback/notepad/...`, and storage moved `CF_PROFILES=609 -> 610`. Separate inspect/report confirmed rollback row from `0.2.0` to `0.1.0`.
  - Rollback no-prior edge: single-version terminal install succeeded, then `profile_registry_rollback profile_id=terminal` failed closed with `rollback_prior_package_missing`; before/after storage stayed `CF_PROFILES=615`, `CF_KV=1`.
  - Poison contribution: valid contribution bundle containing `ignore previous instructions` imported through real `profile_registry_import`; only contribution row was written (`CF_PROFILES=615 -> 616`), inspect read `state=quarantined`, `accepted=0`, `rejected=1`, risk flags `metadata_prompt_injection_text` and `low_quality_no_success_evidence`.
  - `>1000` contribution rows edge: valid 1001-row contribution bundle imported through real tool; only contribution row was written (`CF_PROFILES=616 -> 617`), inspect read `state=quarantined`, `accepted=0`, `rejected=1001`, risk flag `contribution_registry_rows_limit_exceeded`.
  - Generic edges: `profile_registry_search limit=0`, malformed import JSON, and contribution export without `profile_id` failed closed; before/after storage stayed `CF_PROFILES=617`, `CF_KV=1`, `CF_ACTION_LOG=0`; missing-profile export wrote no file. An attempted empty-string CLI arg was rejected by Inspector before Synapse and is not counted as product behavior evidence.
  - Final SoT: `storage_inspect` read `CF_PROFILES=617`, `CF_KV=1`, `CF_ACTION_LOG=0`; report scanned 617 rows, packages 603, installed 2, curated 2, rollback 1, heads 1; contribution search with `include_disabled=true` found two quarantined contribution rows. Log readback had 0 panics and only expected fail-closed response errors.
  - Cleanup: real Inspector `release_all` returned zero held state; daemon PID `58848` stopped; port `7849` no longer listens.
- Supporting checks passed:
  - `cargo fmt --check`
  - `git diff --check` (line-ending warning only)
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --test m5_curated_registry_tool -- --nocapture`
  - `cargo test -p synapse-mcp --test m5_registry_report_tool -- --nocapture`
  - `cargo test -p synapse-profiles --test package_manifest -- --nocapture`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo test -p synapse-mcp --test m3_tools_list -- --nocapture`
  - `cargo build --release -p synapse-mcp -j 2`
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46406144`, SHA256 `08FEC90BE80C37B940AF9549335F901A8DACE52863FDA9F7990049F0A4A94890`, `LastWriteTimeUtc=2026-06-01T18:40:29Z`.
- Next: commit state update with `[skip ci]`, post #621 RESOLVED evidence, close #621, refresh queue, and take #622 unless GitHub changes.

## 2026-06-01T13:16:11-05:00
- #620 `scenario(stress): activate all 30 profiles - keymap/HUD/capture/mode apply` is closed.
  - Commit: `6895746 fix(mcp): apply profile runtime config (#620) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/620#issuecomment-4595282935
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T18:15:30Z`.
  - Manual FSV run directory: `.runs\620\profile-fsv-20260601T1238-clean`.
  - Final release binary SHA256: `7940237DE08DB7DF92D7D79944F6DF9FF3120A001AD0FA7C991DD28FCF81F578`.
- Post-close git readback: `main...origin/main`, clean, HEAD `6895746`.
- Active issue is now #621 `scenario(stress): registry scale - install/search/export/import/rollback, digest, poison quarantine`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/621#issuecomment-4595287040
  - #621 is assigned to `ChrisRoyse` and labeled `status:in-progress`, `agent:codex`.
  - Live queue after #620 closure: #594 plus #595-#604 and #621-#634.
  - #621 requires real MCP profile registry triggers and separate storage/profile-registry SoT readbacks for digest enforcement, scale search/report/export/import, rollback rewrite, poison quarantine, and edges: digest mismatch, disable then inspect, conflicting import, rollback with no prior, >1000 contribution rows, empty/boundary/structurally invalid inputs.
  - Next: inspect profile registry manifest/schema/storage implementation and supporting tests before launching a repo-built isolated daemon for manual MCP FSV.

## 2026-06-01T13:04:31-05:00
- Active issue remains #620 `scenario(stress): activate all 30 profiles - keymap/HUD/capture/mode apply`.
- #620 implementation patch is in the worktree and not yet committed. It applies profile runtime config beyond action backend resolution:
  - `profile_activate` now applies M1 perception mode and capture config from the active profile.
  - `M1State` tracks `active_capture_config`; `observe.diagnostics.capture_config` and `health.subsystems.perception` expose separate readback surfaces.
  - foreground-profile `observe` applies the matched profile mode/capture before assembling the observation.
  - regression coverage in `m3_profile_tools` checks activation health and matched-profile observe mode/capture.
- Manual MCP evidence for #620 is captured under `.runs\620\profile-fsv-20260601T1238-clean`.
  - Repo-built daemon: PID `61244`, bind `127.0.0.1:7848`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, isolated DB `.runs\620\profile-fsv-20260601T1238-clean\db`, isolated appdata token file, token `synapse-620-token`.
  - Release binary before FSV: length `46406144`, SHA256 `62323B2C4025438116F18A252DA53883C7F0DD1FCEC8651DA3B91824E6D35F6D`, `LastWriteTimeUtc=2026-06-01T17:37:16Z`.
  - MCP precondition/readback passed: process path/command line matched repo release binary; socket `127.0.0.1:7848` listened under PID `61244`; unauth `/health=401`; auth `/health=200 ok=true`; official MCP Inspector `0.21.2` strict `tools/list` exited 0 with 80 tools and required #620 tools present. Inspector stderr contained only `unknown format "uint*"` schema warnings, not schema rejection.
  - Live profile SoT is 29 bundled TOML profiles, not the stale issue-title count of 30. `crates\synapse-profiles\profiles` file count, `profile_list`, and daemon `health` all read 29.
  - All 29 bundled profiles were activated through official Inspector `profile_activate`, each followed by separate `profile_list` and `health` readbacks. Every readback matched the activated profile id, expected mode, `profile:<id>` capture source, foreground-window capture target, min interval/cursor settings, and exactly one active profile row. Final all-profile readback: active profile `zoom`.
  - Matching foreground `observe` evidence: activated `powershell`; official Inspector `observe` on foreground PowerShell read `mode=a11y_only`, `foreground.profile_id=powershell`, `diagnostics.capture_config.source=profile:powershell`, target `foreground_window`, min interval 50 ms, cursor visible true, and empty HUD fields because the powershell profile defines none.
  - Keymap evidence: activated `powershell`; SoT before `storage_inspect` read `CF_ACTION_LOG=4`; trigger `act_keymap alias=clear backend=software hold_ms=1`; after `storage_inspect` read `CF_ACTION_LOG=6` and final action row preserved alias `clear`, resolved binding `ctrl+l`, resolved keys `["ctrl","l"]`, backend `software`, status `ok`, foreground `powershell.exe`.
  - HUD profile specs: `profile_list`/TOML show HUD fields for `everquest.live`, `luanti.minetest`, and `minecraft.java`. Live Luanti was launched and process/window title read matched the profile, but foreground stayed locked to PowerShell and both isolated/wired `act_click` failed with `SetPhysicalCursorPos ... Access is denied`; HUD-slot live readback was therefore a documented explained gap under #620 acceptance rather than a product-code verdict.
  - Edges covered through real Inspector tools with separate SoT readbacks: unknown profile `missing620` failed closed and active/mode stayed `powershell`; same-profile reactivation returned `changed=false`; activate `acrobat` while PowerShell was foreground caused `act_keymap copy` to fail closed against the foreground `powershell` profile and action log advanced by error rows; empty alias `""` failed closed with `TOOL_PARAMS_INVALID`; unknown alias `__missing620__` failed closed with `PROFILE_KEYMAP_INVALID`; no bundled empty-keymap profile exists.
  - Cleanup: official Inspector `release_all` returned zero held state; Luanti and the FSV-owned Notepad processes were stopped; isolated daemon PID `61244` was stopped; port `127.0.0.1:7848` no longer listens. Existing older Notepad processes were not killed because they predated this run.
- Post-compaction wired MCP client readback passed: `health ok=true`, `storage_inspect` returned live storage rows, `reflex_list`/`reflex_history` returned, and `observe` read foreground PowerShell.
- Final supporting checks passed after #620 FSV:
  - `cargo fmt --check`
  - `git diff --check` (line-ending warnings only)
  - `cargo check -p synapse-core -j 2`
  - `cargo check -p synapse-perception -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --test m3_profile_tools -- --nocapture`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo test -p synapse-core observation_json_shape --test snapshots -- --nocapture`
  - `cargo test -p synapse-perception --test perception_regression -- --nocapture` (7 passed, 2 ignored WinRT-desktop tests)
  - `cargo test -p synapse-mcp --test m3_tools_list -- --nocapture`
  - `cargo test -p synapse-mcp --test m4_tools_list -- --nocapture`
  - `cargo test -p synapse-mcp server::context:: --bin synapse-mcp -- --nocapture`
  - `cargo build --release -p synapse-mcp -j 2`
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46406144`, SHA256 `7940237DE08DB7DF92D7D79944F6DF9FF3120A001AD0FA7C991DD28FCF81F578`, `LastWriteTimeUtc=2026-06-01T18:12:44Z`.
- Diff review completed across code, tests, benches, and state notes.
- Next: commit with `[skip ci]`, post #620 RESOLVED evidence, close #620, refresh queue, and take #621 unless GitHub changes.

## 2026-06-01T12:45:00-05:00
- Active issue remains #620 `scenario(stress): activate all 30 profiles — keymap/HUD/capture/mode apply`.
- User asked about lingering `Issue615FanoutTarget` windows/buttons. Fresh readback found no visible/loaded target:
  - wired `mcp__synapse.find query=Issue615FanoutTarget` returned no results;
  - wired `mcp__synapse.observe` foreground was Explorer taskbar, no #615 window;
  - OS process/window readback found no `Issue615FanoutTarget` title or `issue615*` process.
  - Explanation: those were #615 synthetic UIA fanout target controls used only to mutate a WinForms list for separate UIA readback; they were not product UI.
- #620 implementation patch is in the worktree, not committed:
  - `profile_activate` now applies the active profile as full runtime config, not only action backend resolution.
  - M1 state now tracks `active_capture_config` with target/min interval/cursor/dirty/generation/source.
  - `observe.diagnostics.capture_config` exposes the applied capture config, and `health.subsystems.perception` exposes the non-mutating M1 perception mode/capture readback.
  - Foreground profile resolution now applies profile mode/capture before assembling observation output, so matched-profile `observe` can show profile mode/capture immediately.
  - Supporting regression in `m3_profile_tools` asserts activated profile health readback and matched synthetic notepad observe mode/capture readback.
- Supporting checks passed so far:
  - `cargo check -p synapse-core -j 2`
  - `cargo check -p synapse-perception -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --test m3_profile_tools -- --nocapture`
  - `cargo test -p synapse-core observation_json_shape --test snapshots -- --nocapture`
  - `cargo test -p synapse-perception --test perception_regression -- --nocapture`
  - `cargo test -p synapse-mcp server::context:: --bin synapse-mcp -- --nocapture`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo test -p synapse-core --test types -- --nocapture`
- Live profile count remains 29 bundled TOML profiles, not 30. `crates\synapse-profiles\profiles` file count and wired MCP `health` both read 29. Treat #620 title as stale wording and document this in FSV evidence.
- Next: build release `synapse-mcp`, launch isolated repo-built HTTP daemon for #620, strict Inspector `tools/list`, then manual FSV:
  - activate all 29 bundled profiles and read `profile_list` + `health.subsystems.perception` after each;
  - representative real `observe` mode/capture/HUD readback on a matching foreground profile;
  - `act_keymap` success for a matching foreground profile with `CF_ACTION_LOG` alias/resolved binding readback;
  - required edges: unknown profile id, same-profile reactivation, app-not-running action denial, empty/invalid keymap alias/unknown alias, and no-empty-keymap profile gap if none exist.

## 2026-06-01T11:54:30-05:00
- #619 `scenario(stress): storage_gc_once under concurrent writes` is closed.
  - No product-code patch was required.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/619#issuecomment-4594692386
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T16:53:51Z`.
  - Manual FSV run directory: `.runs\619\gc-concurrent-fsv-20260601T1135`.
  - Final release binary SHA256: `AF801288800BB64E3DA92B95573F2E9787FE7899AA497E264E7023242D03AB60`.
- Active issue is now #620 `scenario(stress): activate all 30 profiles — keymap/HUD/capture/mode apply`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/620#issuecomment-4594697356
  - #620 is assigned to `ChrisRoyse` and labeled `status:in-progress`, `agent:codex`.
  - Live queue after #619 closure: #594 plus #595-#604 and #620-#634.
  - #620 requires real MCP `profile_list`, `profile_activate`, `observe`, `act_keymap`, and storage/action-log readbacks where applicable. Required edges: unknown `profile_id`, same-profile reactivation (`changed=false`), activation while app not running, profile with empty keymap if present, and empty/boundary/structurally invalid inputs.
  - Next: inspect profile runtime/registry code and bundled profile definitions, then launch a repo-built isolated daemon for manual MCP FSV.

## 2026-06-01T11:45:17-05:00
- Active issue #619 `scenario(stress): storage_gc_once under concurrent writes` has manual MCP FSV behavior evidence captured; final supporting checks, RESOLVED comment, close, and queue continuation are next.
- No product-code patch has been required so far; current worktree should contain only `STATE/*` updates and #619 run artifacts under `.runs\619\gc-concurrent-fsv-20260601T1135`.
- Manual FSV run directory: `.runs\619\gc-concurrent-fsv-20260601T1135`.
  - Repo-built daemon PID `69600`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, bind `127.0.0.1:7847`, isolated DB `.runs\619\gc-concurrent-fsv-20260601T1135\db`, token `synapse-619-token`.
  - Process/socket/auth/client-parity readbacks passed after compaction: process path matched the repo release binary; socket listened on `127.0.0.1:7847` owned by PID `69600`; auth `/health ok=true`; official MCP Inspector `0.21.2` strict `tools/list` returned 80 tools with `storage_inspect`, `storage_put_probe_rows`, `storage_gc_once`, and `release_all`.
  - Initial SoT read: isolated storage pressure `Normal`; all 11 CF row counts were 0.
  - Setup correction: the first writer launch attempt failed before MCP because the Inspector header argument was split incorrectly (`Invalid header format: Authorization:`); separate `storage_inspect` confirmed `CF_EVENTS=0`. This is not counted as behavior evidence.
  - Concurrent writers: four real Inspector `storage_put_probe_rows` clients wrote 80 rows each into `CF_EVENTS` with prefixes `issue619-100-a`, `issue619-200-b`, `issue619-300-c`, and `issue619-400-d`. Separate `storage_inspect` read `CF_EVENTS=320` with tail samples `issue619-400-d:77..79`.
  - GC after concurrent writers: real `storage_gc_once cf_name=CF_EVENTS soft_cap_rows=75 hard_cap_rows=120` read `before_rows=320`, `after_rows=75`, `total_evicted_rows=245`, `cache_evictions_total_delta=245`, and `STORAGE_CF_HARD_CAP_REACHED`; separate `storage_inspect` read `CF_EVENTS=75` retaining newest tail samples `issue619-400-d:77..79`.
  - In-flight/heavy writer: real Inspector writer wrote `10000` rows x `2048` bytes with prefix `issue619-900-z`, and a real GC call during that activity read `before_rows=10075`, `after_rows=75`, `total_evicted_rows=10000`, hard cap reached. Separate `storage_inspect` read `CF_EVENTS=75` with newest tail samples `issue619-900-z:9997..9999`.
  - Audit-retention max-age edge: wrote three valid `CF_ACTION_LOG` JSON audit rows at `ts_ns=1000,1010,1020`; `AUDIT_RETENTION` run `issue619-age` with `now_ns=5000 max_age_ns=100` deleted all three as expired, wrote report key `audit_retention/v1/report/issue619-age`, and separate `storage_inspect` read `CF_ACTION_LOG=0`, `CF_KV=1` with the report row.
  - Audit-retention dedupe/run_id edge: wrote three valid duplicate action audit rows at `ts_ns=10000,10010,10020`; `AUDIT_RETENTION` run `issue619-dedupe` with `dedupe_window_ns=100` deleted two duplicates, retained the first row, and wrote report key `audit_retention/v1/report/issue619-dedupe`. Separate `storage_inspect` read `CF_ACTION_LOG=1`, `CF_KV=2`, with the dedupe report including duplicate deletion keys and the expected dedupe key.
  - Boundary edge at soft cap: before `CF_EVENTS=75`; real `storage_gc_once soft_cap_rows=75 hard_cap_rows=120` returned `before_rows=75`, `after_rows=75`, `total_evicted_rows=0`; separate inspect kept `CF_EVENTS=75`.
  - Empty CF edge: before `CF_MODEL_CACHE=0`; real `storage_gc_once cf_name=CF_MODEL_CACHE soft_cap_rows=1 hard_cap_rows=10` returned `before_rows=0`, `after_rows=0`, `total_evicted_rows=0`; separate inspect kept `CF_MODEL_CACHE=0`.
  - Structurally invalid edge: real Inspector `storage_gc_once cf_name=CF_EVENTS soft_cap_rows=0 hard_cap_rows=10` failed closed with MCP error `-32099`, `TOOL_PARAMS_INVALID`, message `storage_gc_once soft_cap_rows must be between 1 and 1000000`; separate inspect showed `CF_EVENTS=75`, `CF_ACTION_LOG=1`, `CF_KV=2`, `CF_MODEL_CACHE=0` unchanged from before.
  - Oscillation below hard cap: wrote 25 rows with prefix `issue619-950-y`, separate inspect read `CF_EVENTS=100` and tail `issue619-950-y:22..24`; real GC `soft=75 hard=120` returned `before_rows=100`, `after_rows=75`, `evicted=25`, no hard-cap code; separate inspect read `CF_EVENTS=75` and retained `issue619-950-y:22..24`.
  - Daemon log readback contains `MCP_TOOL_INVOCATION` lines for each storage trigger, `STORAGE_CF_HARD_CAP_REACHED` for the 320-row and 10075-row cases, `STORAGE_CACHE_EVICTIONS_TOTAL_INCREMENTED` for evictions of 245/10000/25, and the intentional invalid-param `TOOL_PARAMS_INVALID`; no other `ERROR`, panic, corruption, or failed lines were present.
  - Cleanup readback: real Inspector `release_all` returned `released_keys=0`, `released_buttons=0`, `neutralized_pads=0`; stopped daemon PID `69600`; port `127.0.0.1:7847` no longer listens.
- Final supporting checks passed after FSV: `cargo fmt --check`; `git diff --check` (line-ending warnings only); `cargo check -p synapse-storage -j 2`; `cargo check -p synapse-reflex -j 2`; `cargo check -p synapse-mcp -j 2`; `cargo test -p synapse-storage gc_soft_cap_hard_cap_edges_and_metrics -- --nocapture`; `cargo test -p synapse-storage gc_soft_cap_edges_and_restart --test gc_soft_cap -- --nocapture`; `cargo test -p synapse-mcp --test m3_storage_tool -- --nocapture`; `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`; `cargo build --release -p synapse-mcp -j 2`.
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46320128`, SHA256 `AF801288800BB64E3DA92B95573F2E9787FE7899AA497E264E7023242D03AB60`, `LastWriteTimeUtc=2026-06-01T16:52:28Z`.
- Next: post #619 RESOLVED evidence, close #619, refresh live queue, and take the next open child.

## 2026-06-01T11:29:00-05:00
- #618 `scenario(stress): storage pressure ladder - 5 levels + write-gating` is closed.
  - Commit: `c0b24e3 fix(mcp): expose storage pressure gating (#618) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/618#issuecomment-4594501572
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T16:27:18Z`.
  - Manual FSV run directory: `.runs\618\pressure-fsv-20260601T1108-patched`.
  - Final release binary SHA256: `8BCD4B02A37D85C40D15087C8A3B66A8963804CB8A5877CC5A349CE676EFB12B`.
  - Post-close git readback: `main...origin/main`, clean, HEAD `c0b24e3`.
- Active issue is now #619 `scenario(stress): storage_gc_once under concurrent writes`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/619#issuecomment-4594506099
  - Live queue after #618 closure: #594 plus #595-#604 and #619-#634.
  - #619 requires real MCP `storage_put_probe_rows`, `storage_gc_once`, and `storage_inspect` triggers proving GC correctness under concurrent writes, counts oscillating around soft cap without corruption, audit retention report consistency, no lost newest rows, max-age eviction, dedupe window, run_id provenance, GC on empty CF, boundary caps, and structurally invalid params.
  - Next: inspect storage GC/probe-row implementation and tests before launching a repo-built isolated daemon for manual MCP FSV.

## 2026-06-01T11:18:00-05:00
- Active issue #618 `scenario(stress): storage pressure ladder - 5 levels + write-gating` has implementation and manual MCP FSV evidence captured; final supporting checks, diff review, commit, RESOLVED comment, close, and queue continuation are next.
- Patch in worktree:
  - `crates/synapse-storage/src/lib.rs`: exposes `Db::pressure_permits_write`.
  - `crates/synapse-reflex/src/storage.rs`: exposes `ReflexRuntime::storage_pressure_permits_write`.
  - `crates/synapse-mcp/src/m3/storage.rs`: allows diagnostic probe writes across all 11 storage CFs, and returns explicit `STORAGE_WRITE_FAILED` when pressure policy refuses a non-empty diagnostic write.
  - `crates/synapse-mcp/tests/m3_storage_tool.rs`: adds supporting regression coverage for pressure-gated CFs.
- Manual FSV run directory: `.runs\618\pressure-fsv-20260601T1108-patched`.
  - Repo-built daemon PID `56980`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, bind `127.0.0.1:7846`, isolated DB `.runs\618\pressure-fsv-20260601T1108-patched\db`, token `synapse-618-token`.
  - Process/socket/auth/client-parity readbacks passed: process path/command line matched repo release binary and `--db`; socket listened on `127.0.0.1:7846` owned by PID `56980`; unauth `/health=401`; auth `/health ok=true`; official MCP Inspector `0.21.2` strict `tools/list` returned 80 tools including `storage_inspect`, `storage_pressure_sample`, `storage_put_probe_rows`, and `release_all`.
  - Initial/separate `storage_inspect` readback after exact L1 threshold had pressure `Normal`, no transition codes, and all 11 CF row counts at 0.
  - Pressure ladder evidence:
    - exact L1 threshold `free_bytes=2000000000`: stayed `Normal`, no code, no compaction.
    - below L1 `1999999999`: `Normal -> Level1`, emitted `STORAGE_DISK_PRESSURE_LEVEL_1`, no compaction, separate inspect pressure `Level1`.
    - exact L2 threshold `1000000000`: stayed `Level1`, no new code, no compaction, separate inspect pressure `Level1`.
    - below L2 `999999999`: `Level1 -> Level2`, emitted `STORAGE_DISK_PRESSURE_LEVEL_2`, compacted 11 CFs, separate inspect pressure `Level2`.
    - exact L3 threshold `500000000`: stayed `Level2`, no new code, no compaction.
    - below L3 `499999999`: `Level2 -> Level3`, emitted `STORAGE_DISK_PRESSURE_LEVEL_3`, compacted 11 CFs, separate inspect pressure `Level3`.
    - exact L4 threshold `200000000`: stayed `Level3`, no new code, no compaction.
    - below L4 `199999999`: `Level3 -> Level4`, emitted `STORAGE_DISK_PRESSURE_LEVEL_4`, compacted 11 CFs, separate inspect pressure `Level4`.
    - recovery `2500000000`: `Level4 -> Normal`, no code, no compaction, separate inspect pressure `Normal`.
  - Write-gating evidence:
    - `Level2`: `CF_OBSERVATIONS` write accepted, count 0 -> 1 and sample key prefix `issue618-l2-observations`.
    - `Level3`: writes to `CF_OBSERVATIONS`, `CF_OCR_CACHE`, `CF_TELEMETRY`, `CF_MODEL_CACHE`, and `CF_PROCESS_HISTORY` were all refused with `STORAGE_WRITE_FAILED`; separate inspect showed counts unchanged (`CF_OBSERVATIONS=1`, the other four 0). `CF_EVENTS` remained allowed and wrote 0 -> 1.
    - `Level4`: writes to `CF_EVENTS`, `CF_ACTION_LOG`, `CF_KV`, and `CF_OBSERVATIONS` were refused with `STORAGE_WRITE_FAILED`; `CF_REFLEX_AUDIT` and `CF_SESSIONS` each wrote 0 -> 1; separate inspect confirmed only allowed CFs changed.
    - Empty edge under `Level4`: `CF_EVENTS rows=0 value_bytes=0` succeeded as a no-op, leaving `CF_EVENTS=1`.
    - Structurally invalid edge: `cf_name=NOT_A_CF` failed closed with MCP error `-32099` listing allowed CFs; separate inspect left counts unchanged.
    - Recovery edge: after returning `Normal`, `CF_OBSERVATIONS` write accepted again, count 1 -> 2 and sample key prefix `issue618-recovered-observations`.
  - Daemon log readback contained all four `STORAGE_DISK_PRESSURE_LEVEL_*` transition lines and each explicit `STORAGE_WRITE_FAILED` refusal.
  - Cleanup readback: real Inspector `release_all` returned `released_keys=0`, `released_buttons=0`, `neutralized_pads=0`; stopped PID `56980`; port `127.0.0.1:7846` no longer listens.
- Final supporting checks passed after FSV: `cargo fmt --check`; `git diff --check` (line-ending warnings only); `cargo check -p synapse-storage -j 2`; `cargo check -p synapse-reflex -j 2`; `cargo check -p synapse-mcp -j 2`; `cargo test -p synapse-storage pressure -- --nocapture`; `cargo test -p synapse-mcp --test m3_storage_tool -- --nocapture`; `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`; `cargo build --release -p synapse-mcp -j 2`.
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46320128`, SHA256 `8BCD4B02A37D85C40D15087C8A3B66A8963804CB8A5877CC5A349CE676EFB12B`, `LastWriteTimeUtc=2026-06-01T16:25:11.3649649Z`.
- Diff review completed for code/test/state changes.
- Next: commit with `[skip ci]`, post #618 RESOLVED evidence, close #618, refresh queue, and take next open child (#619 unless queue changes).

## 2026-06-01T10:53:00-05:00
- #617 `scenario(stress): storage CF saturation to hard cap + GC eviction` is closed.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/617#issuecomment-4594236079
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T15:52:11Z`.
  - No code patch was required; worktree stayed clean after #617 FSV and final checks.
  - Manual FSV run directory: `.runs\617\storage-fsv-20260601T1024`.
  - Repo-built daemon evidence: PID `73864`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, bind `127.0.0.1:7845`, isolated DB `.runs\617\storage-fsv-20260601T1024\db`, token `synapse-617-token`.
  - Process/socket/auth/client-parity readbacks passed: process path matched repo release binary; socket listened on `127.0.0.1:7845`; unauth `/health=401`; auth `/health ok=true`; official MCP Inspector `0.21.2` strict `tools/list` returned 80 tools including `storage_inspect`, `storage_put_probe_rows`, `storage_gc_once`, `storage_pressure_sample`, and `release_all`.
  - Initial `storage_inspect` read all CF counts/sizes as 0, pressure `Normal`, and 12 audit-retention policies.
  - Happy path: real Inspector `tools/call storage_put_probe_rows` wrote 12 rows x 256 bytes to `CF_EVENTS`, `CF_OBSERVATIONS`, `CF_SESSIONS`, `CF_ACTION_LOG`, and `CF_KV`; separate `storage_inspect` read counts 12 for all five CFs and nonzero sizes.
  - GC path: real Inspector `tools/call storage_gc_once soft_cap_rows=9 hard_cap_rows=20` on each writable CF evicted 3 rows; separate `storage_inspect` read counts 9 for all five CFs and reduced sizes.
  - Edge coverage:
    - hard cap warning continued: `CF_EVENTS` before-read 25 rows, `storage_gc_once soft=10 hard=20` returned `STORAGE_CF_HARD_CAP_REACHED` and evicted 15; after-read 10 rows.
    - invalid `soft_cap_rows > hard_cap_rows` failed closed and left CF counts unchanged.
    - max value size wrote one `65536`-byte row to `CF_OBSERVATIONS`; after-read tail sample had `value_len_bytes=65536`.
    - 128-byte key-prefix boundary wrote one `CF_SESSIONS` row and after-read tail sample contained the long-prefix key.
    - empty/no-op `rows=0 value_bytes=0` left `CF_KV` count/size unchanged.
    - 129-byte prefix failed closed and left `CF_SESSIONS` unchanged.
    - `AUDIT_RETENTION` mode wrote report key `audit_retention/v1/report/issue617-audit-report`; separate `storage_inspect` showed `CF_KV` 9 -> 10.
  - Cleanup readback: real `release_all` returned zero held state; stopped PID `73864`; port `7845` no longer listens.
  - Supporting checks passed: `cargo fmt --check`; `cargo check -p synapse-mcp -j 2`; `cargo test -p synapse-storage gc_soft_cap_hard_cap_edges_and_metrics -- --nocapture`; `cargo test -p synapse-mcp --test m3_storage_tool -- --nocapture`; `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`; `cargo build --release -p synapse-mcp -j 2`; `git diff --check`.
  - Final release binary readback: `target\release\synapse-mcp.exe`, length `46380544`, SHA256 `5E24CC28BB709688209215531A590283A9AA54AF959D9CEC1CDC6A58E5EEC5C5`, `LastWriteTimeUtc=2026-06-01T15:51:19.2177043Z`.
- Active issue is now #618 `scenario(stress): storage pressure ladder — 5 levels + write-gating`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/618#issuecomment-4594238857
  - Live queue after #617 closure: #594 plus #595-#604 and #618-#634.
  - #618 requires real MCP `storage_pressure_sample`, `storage_put_probe_rows`, and `storage_inspect` triggers proving pressure transitions Normal/L1/L2/L3/L4, write gating by pressure level, recovery to Normal, threshold boundaries, explicit refusal errors, and at least three edge cases.
  - Next: inspect storage pressure/write-gating implementation and tests, then launch a repo-built isolated daemon for #618 manual MCP FSV.

## 2026-06-01T10:21:23-05:00
- #616 `scenario(stress): reality drift injection -> reality_audit rebase` is closed.
  - Commit: `79f735f fix(mcp): classify reality audit drift (#616) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/616#issuecomment-4593986844
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T15:20:44Z`.
- Post-close git readback before state update: `main...origin/main`, clean, HEAD `79f735f`.
- Refreshed live open queue now lists #594 plus #595-#604 and #617-#634.
- Active issue is now #617 `scenario(stress): storage CF saturation to hard cap + GC eviction`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/617#issuecomment-4593992720
  - #617 requires real MCP `storage_put_probe_rows`, `storage_inspect`, and `storage_gc_once` triggers proving per-CF row/size growth, pressure/cap readbacks, and eviction under tight soft/hard caps.
  - Required edges: push past hard cap warning/continuation, `storage_gc_once` soft>hard invalid, max value bytes, key-prefix at 128-byte limit, empty/no-op, boundary, and structurally invalid params.
- Next: inspect storage tool implementation and tests before launching an isolated repo-built daemon for manual MCP FSV.

## 2026-06-01T10:12:14-05:00
- Active issue #616 `scenario(stress): reality drift injection -> reality_audit rebase` has implementation and manual MCP FSV evidence captured; final supporting checks/diff review/commit/issue closure are next.
- Patch in `crates/synapse-mcp/src/server/reality.rs`:
  - `reality_audit` now distinguishes missing baseline/source unavailable, stale epoch, caller assumption-hash mismatch, exact in-sync, minor physical drift, and major physical drift.
  - Physical drift is computed by comparing stored head compact state to the freshly captured compact state with itemized `RealityDriftItem` rows and scoped source refs.
  - Highest-severity-wins classification makes title/bounds/focused/diagnostics changes minor unless source unavailable, while UI structure/element appear/disappear and material profile/process changes are major.
- Manual FSV run directory: `.runs\616\audit-fsv-20260601T0945`.
  - Repo-built daemon PID `80292`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, bind `127.0.0.1:7844`, isolated DB `.runs\616\audit-fsv-20260601T0945\db`, token `synapse-616-token`.
  - Process/socket/auth/client-parity readbacks passed: process path matched repo release binary; socket listened on `127.0.0.1:7844`; unauth `/health=401`; auth `/health ok=true`; official MCP Inspector strict `tools/list` returned 80 tools including `reality_baseline`, `observe_delta`, `reality_audit`, `act_launch`, `act_run_shell`, `storage_inspect`, and `release_all`.
  - Source-unavailable/missing baseline edge: before isolated `CF_KV=0`; real Inspector `tools/call reality_audit depth=1 max_elements=5` on Chrome with no baseline returned `baseline_status=source_unavailable`, `drift_status=source_unavailable`, `/baseline` drift item, and wrote `reality/audit/v1/chrome/audit-01780325611288876400-0000000001`; separate `storage_inspect` read `CF_KV=1`, `CF_OBSERVATIONS=1`.
  - Baseline + delta + no-drift: real MCP `act_launch` started target PID `57796`, HWND `0xe1b6a`, title `Issue616DriftTarget Baseline`. Baseline `issue616-loop-20260601T1002` wrote baseline/head rows for profile `powershell`; an out-of-band `SetWindowText` changed title to `Issue616DriftTarget LoopDelta`; real `observe_delta since_seq=0` wrote two delta rows and the head row; real `reality_audit` returned `drift_status=in_sync`, `assumption_hash == actual_hash`, 0 drift items, audit row `reality/audit/v1/powershell/audit-01780326192261774400-0000000002`.
  - Minor drift boundary: baseline `issue616-minor-20260601T1004`, out-of-band title-only change to `Issue616DriftTarget MinorOnly`; real `reality_audit` returned `drift_status=minor_drift`, `rebase_required=true`, two minor drift items for `/foreground/window_title_sha256` and window element name hash, audit row `reality/audit/v1/powershell/audit-01780326236144393800-0000000003`.
  - Audit immediately after rebase: baseline `issue616-rebase-20260601T1005`; immediate `reality_audit` returned `drift_status=in_sync`, `rebase_required=false`, `assumption_hash == actual_hash`, audit row `reality/audit/v1/powershell/audit-01780326257510657800-0000000004`.
  - Major drift boundary: launched temporary target PID `13676`, HWND `0x101530`, title `Issue616MajorTarget Baseline`; baseline `issue616-major-structure-20260601T1009`; out-of-band Win32 `BM_CLICK` on `AddMajor` changed physical child texts from `major-baseline-state|AddMajor|CloseTarget` to `major-baseline-state|AddMajor|CloseTarget|major-new-control`; real `reality_audit` returned `drift_status=major_drift`, `rebase_required=true`, with a major `/elements/...MajorNewControl` drift item, audit row `reality/audit/v1/powershell/audit-01780326554678039600-0000000006`.
  - Stale epoch edge: after baseline `issue616-stale-current-20260601T1010`, real `reality_audit epoch_id=issue616-major-structure-20260601T1009` returned `baseline_status=stale`, `drift_status=rebase_required`, `/epoch_id` item comparing old to current epoch, audit row `reality/audit/v1/powershell/audit-01780326600716104200-0000000007`.
  - Structurally invalid edge: before invalid `CF_KV=18`, `CF_ACTION_LOG=14`; real Inspector `tools/call reality_audit depth=0` failed closed with `MCP error -32099: depth must be between 1 and 6`; after invalid `CF_KV=18`, `CF_ACTION_LOG=14`.
  - Cleanup readback: real `release_all` completed; stopped target PID `13676`; stopped isolated daemon PID `80292`; port `127.0.0.1:7844` no longer listens; no visible `Issue616*` or `Issue615FanoutTarget` windows remain.
- Final supporting checks after FSV passed: `cargo fmt --check`; `cargo check -p synapse-mcp -j 2`; `cargo test -p synapse-mcp server::reality::tests --bin synapse-mcp -- --nocapture` (20 passed); `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture` (3 passed); `cargo build --release -p synapse-mcp -j 2`; `git diff --check` exited 0 with line-ending warnings only.
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46380544`, SHA256 `86D55735BD2FA893E22B16E955D431474147B5F3CE1F616BCBD4EB1E047B201B`, `LastWriteTimeUtc=2026-06-01T15:18:29.1464141Z`.
- Diff review completed for `crates/synapse-mcp/src/server/reality.rs` and `STATE/*`.
- Next: commit with `[skip ci]`, post #616 RESOLVED evidence, close #616, update state, and continue the open queue.

## 2026-06-01T09:39:21-05:00
- Active issue #616 `scenario(stress): reality drift injection -> reality_audit rebase` is in implementation/checkpoint state.
- Code inspection found a real gap in `crates/synapse-mcp/src/server/reality.rs`: `reality_audit` compared only `assumption_hash` to the fresh compact-state hash, so every non-matching physical state collapsed into `rebase_required` and the public `minor_drift` / `major_drift` statuses were never produced.
- Patch in worktree:
  - `reality_audit` now compares the stored head compact state to the fresh captured compact state through `reality_changes`.
  - Drift items are persisted per changed path with before/after values, source refs scoped by change kind, and severity.
  - Stale epoch, missing baseline, caller assumption-hash mismatch, source-unavailable diagnostics, in-sync, minor physical drift, and major physical drift are distinguished.
  - Severity policy is highest-severity-wins, following the drift-classification research pattern from the Exa lookup.
- Focused supporting checks passed:
  - `cargo fmt`
  - `cargo test -p synapse-mcp reality_audit_ --bin synapse-mcp -- --nocapture` (6 passed)
- Next: run broader supporting checks/release build, update state, launch isolated repo-built daemon, and perform #616 manual MCP FSV.

## 2026-06-01T09:31:17-05:00
- #615 `scenario(stress): reality high-fanout delta coalescing + snapshot-budget-exceeded` is closed.
  - Commit: `fad86c9 fix(mcp): harden reality fanout coalescing (#615) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/615#issuecomment-4593549908
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T14:30:47Z`.
- Post-close git readback: `main...origin/main`, clean, HEAD `fad86c9`.
- Refreshed live open queue now lists #594 plus #595-#604 and #616-#634.
- Active issue is now #616 `scenario(stress): reality drift injection -> reality_audit rebase`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/616#issuecomment-4593554251
  - #616 requires real MCP `reality_baseline`, `observe_delta`, and `reality_audit` triggers proving drift verdicts and persisted `CF_KV/reality/audit/*` rows match the actual physical divergence from the assumed epoch/hash.
  - Required edges: no drift / InSync, source unavailable, minor vs major boundary, audit immediately after rebase, empty/no-change, and structurally invalid params.
- Next: inspect `reality_audit` implementation/tests and plan an isolated repo-built daemon FSV run for drift injection.

## 2026-06-01T09:28:31-05:00
- Active issue #615 `scenario(stress): reality high-fanout delta coalescing + snapshot-budget-exceeded` has implementation, manual MCP FSV, cleanup, final supporting checks, and diff review complete; commit/push/comment/close are next.
- Patch in `crates/synapse-mcp/src/server/reality.rs`:
  - `uia_element_fanout` still records all changed UIA ids in aggregate metadata, but high-fanout threshold pressure now counts only structural changes plus material coalescing field changes.
  - Incidental focus and parent `children_count` changes no longer push a 7-appear boundary case over the coalescing threshold.
  - Added regression coverage for incidental changes below threshold, exact threshold coalescing, and mixed appear+field churn.
- Manual FSV run directory: `.runs\615\fanout-fsv-20260601T0844-patched`.
  - Repo-built daemon PID `64500`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, bind `127.0.0.1:7843`, isolated DB `.runs\615\fanout-fsv-20260601T0844-patched\db`, token `synapse-615-patched-token`.
  - Process/socket/auth/client-parity readbacks passed: process path matched repo release binary; socket listened on `127.0.0.1:7843`; auth `/health ok=true`; official MCP Inspector strict `tools/list` returned 80 tools including `reality_baseline`, `observe_delta`, `act_click`, `act_launch`, `storage_inspect`, and `release_all`.
  - Physical target PID `79124`, title `Issue615FanoutTarget`, was launched by real MCP `act_launch`; separate OS UIA reads of the target window were used as the physical SoT for item counts/names after each click.
  - Show7 boundary/low-fanout: before UIA item count 0; real MCP `act_click` Show7; after UIA item count 7 (`Item 0..Item 6`); `observe_delta` returned 7 per-element `uia_element_appeared` deltas, 0 `uia_structure_changed`, source_refs `a11y_uia`.
  - Show8 threshold: before item count 0; real MCP `act_click` Show8; after item count 8; `observe_delta` returned one `uia_structure_changed` with `appeared_count=8`, no per-element appeared rows, capped IDs/hash, source_refs `a11y_uia`; `storage_inspect` showed the `CF_KV` `reality/delta/v1/powershell/issue615-show8-static-.../00000000000000000005` row shape.
  - Rename8 reused-element churn: before `Item 0..7`; real MCP `act_click` Rename8; after `Renamed 0..7`; `observe_delta` returned one `uia_elements_changed`, no per-element name rows, `changed_count=10`, source_refs `a11y_uia`.
  - Mixed appear+field churn: before 4 items; real MCP `act_click` Mixed8; after 8 items (`Mixed Renamed 0..3`, `Mixed New 4..7`); `observe_delta` returned one `uia_structure_changed`, `appeared_count=4`, `changed_count=7`, source_refs `a11y_uia`.
  - Snapshot budget exceeded: before Show80 item count 80 and `CF_KV=62`; real MCP `act_click` Clear; after item count 0 and `CF_KV=62`; `observe_delta` returned `delta_snapshot_budget_exceeded: coalesced delta batch 7770 bytes exceeds compact snapshot 7738 bytes; capture reality_baseline to rebase`, `rebase_required=true`, 0 deltas, 0 rows written.
  - Empty/no-change edge: before item count 0 and `CF_KV=63`; `observe_delta` returned `reason=no_changes`, 0 deltas, 0 rows; after `CF_KV=63`.
  - Structurally invalid edge: `observe_delta depth=0` through strict Inspector returned MCP error `depth must be between 1 and 6`; `CF_KV` and `CF_ACTION_LOG` counts were unchanged.
  - Disappear8 symmetry: before 8 items; real MCP `act_click` Clear; after item count 0; `observe_delta` returned one `uia_structure_changed` with `disappeared_count=8`, capped IDs/hash, source_refs `a11y_uia`.
- Cleanup readback: real MCP `release_all` completed; stopped target PID `79124`; stopped isolated daemon PID `64500`; port `127.0.0.1:7843` no longer listens.
- Final supporting checks after FSV passed:
  - `cargo fmt --check`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp server::reality::tests --bin synapse-mcp -- --nocapture` (17 passed)
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture` (3 passed)
  - `cargo build --release -p synapse-mcp -j 2`
  - `git diff --check` exited 0 with line-ending warnings only.
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46334464`, SHA256 `0EDEBFD08BB324FDCD835727A005C4A161D86C7C6BE5EE34E72FBBA96C8D8894`, `LastWriteTimeUtc=2026-06-01T14:28:17.6122521Z`.
- Diff review completed for `crates/synapse-mcp/src/server/reality.rs` and `STATE/*`.
- Next: commit with `[skip ci]`, post #615 RESOLVED evidence, close #615, refresh the queue, and continue to #616 unless the queue changes.

## 2026-06-01T08:19:00-05:00
- #614 `scenario(stress): reality baseline->delta->audit full loop across all sensors` is closed.
  - Commit: `72918cd fix(mcp): harden reality delta full loop (#614) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/614#issuecomment-4592935089
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T13:16:36Z`.
- Post-close git readback: `main...origin/main`, clean, HEAD `72918cd`.
- Refreshed live open queue now lists #594 plus #595-#604 and #615-#634.
- Active issue is now #615 `scenario(stress): reality high-fanout delta coalescing + snapshot-budget-exceeded`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/615#issuecomment-4592942496
  - #615 requires real MCP `observe_delta` triggers proving high-fanout UIA appear/disappear coalesces into one `uia_structure_changed` row, high-fanout reused-element field changes coalesce into `uia_elements_changed`, oversized coalesced batches return `delta_snapshot_budget_exceeded` rebase guidance with no bloated rows, and CF_KV/source_refs readbacks match.
  - Required edges: exactly 7 vs 8 threshold, mixed appear+field churn, low-fanout per-element, empty/no-change, boundary, and structurally invalid params.
  - Current host note: Windows still reports an orphan `127.0.0.1:7840 LISTENING` row for non-existent PID `82340`; use another port for #615 isolated daemons unless the kernel row clears.
- Next: inspect #615 high-fanout/coalescing code and existing tests before launching an isolated repo-built daemon.

## 2026-06-01T08:10:00-05:00
- Active issue remains #614 `scenario(stress): reality baseline->delta->audit full loop across all sensors`; implementation and manual MCP FSV evidence are captured, final supporting checks/commit/issue closure are next.
- Patch in worktree remains scoped to `crates/synapse-mcp/src/server/reality.rs`:
  - omitted-profile `reality_baseline` observes first and reuses the active observed profile head when no epoch is requested;
  - `observe_delta.since_epoch`, `depth`, and `max_elements` now fail closed server-side;
  - `observe_delta` returns `profile_changed` rebase guidance for a known observed profile switch instead of failing before the edge is represented.
- Main #614 manual FSV run: `.runs\614\reality-loop-fsv-20260601T0741-patched`.
  - Repo-built daemon PID `82340`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, bind `127.0.0.1:7840`, isolated DB `.runs\614\reality-loop-fsv-20260601T0741-patched\db`, token `synapse-614-token`, audio enabled.
  - Process/socket/auth/client parity readbacks passed: process path matched repo release binary; socket listened on `127.0.0.1:7840`; unauth `/health=401`; auth `/health ok=true`, storage DB path isolated, audio loopback running; official MCP Inspector `0.21.2` strict `tools/list` returned 80 tools including `reality_baseline`, `observe_delta`, `reality_audit`, `subscribe`, `act_launch`, `act_run_shell`, `act_clipboard`, `audio_tail`, and `storage_inspect`.
  - Baseline SoT: `reality_baseline` wrote `reality/baseline/v1/unprofiled/issue614-luanti-20260601T0743` and `reality/head/v1/unprofiled`; reuse call returned `created=false`, `reason=existing_baseline_reused`, and CF_KV stayed at 2; forced follow-up baseline `issue614-luanti-profiled-20260601T0746` updated the same unprofiled head because Luanti main-menu title does not match the Luanti gameplay profile regex.
  - Physical triggers: real MCP `act_launch` started Luanti PID `36460`; real `act_clipboard` wrote marker `issue614-delta-20260601T0748`; real `act_run_shell` moved the window, wrote watched marker files, and played a WAV. Separate OS/file reads confirmed Luanti HWND `0x220eaa`, clipboard marker, file text/hash, and window rect.
  - Main `observe_delta` from epoch `issue614-luanti-profiled-20260601T0746`, seq 0 returned 12 deltas and published 12 SSE events: foreground/focus/UIA bounds, HUD `luanti.crosshair_contrast` and `luanti.hotbar_contrast`, entity bbox/confidence, `/audio`, and `/clipboard`. Separate `storage_inspect` read CF_KV 3->15 with delta/head sample keys.
  - SSE SoT: subscription `019e8341-1966-7653-8909-d8d62c6548d0`; `sse-reality-delta-2.out` contains 18 ordered `synapse/event` frames, stream seq 1..18, delta seq 1..18, no lossy frames.
  - Empty/no-change edge: `observe_delta since_seq=12` with same epoch returned `reason=no_changes`, 0 deltas, 0 published SSE events; separate storage read showed CF_KV unchanged at 15.
  - Boundary/diagnostics edge: `observe_delta max_elements=1` returned `/diagnostics` changing `elements_truncated=false` to `true` plus a UIA disappearance; CF_KV advanced 15->17 and samples showed delta seq 13/14 plus head.
  - Cursor/rebase/error edges through strict Inspector:
    - missing baseline: `profile_id=missing614` returned `baseline_required=true`, `rebase_required=true`, `reason=missing_baseline`;
    - stale epoch: old epoch `issue614-luanti-20260601T0743` returned `reason=stale_epoch: requested ..., current issue614-luanti-profiled-20260601T0746`;
    - profile change: real MCP `act_launch notepad.exe` foregrounded Notepad; `observe_delta profile_id=unprofiled since_seq=18` returned `profile_changed: head profile unprofiled but observed notepad`;
    - future seq `999999`, malformed epoch `bad/epoch`, overflow seq `18446744073709551616`, and `depth=0` all failed closed with MCP errors and CF_KV remained 21.
  - `reality_audit` with current epoch wrote audit row `reality/audit/v1/unprofiled/audit-01780319054227975800-0000000001`, returned `baseline_status=current`, `drift_status=rebase_required`, compared seq end 18, and separate storage read showed CF_KV advanced 21->22.
- Filesystem feed subrun: `.runs\614\fs-watch-fsv-20260601T0805`.
  - Repo-built daemon PID `77940`, bind `127.0.0.1:7841`, isolated DB, token `synapse-614fs-token`, `SYNAPSE_FS_WATCH_ROOT=.runs\614\fs-watch-fsv-20260601T0805\watch`.
  - Process/socket/auth/tools-list passed: strict Inspector returned 80 tools. Baseline `issue614-fs-watch-20260601T0806` wrote baseline/head rows with FS count 0.
  - Real MCP `act_run_shell` wrote `issue614-fs-watch-marker.txt`; separate file SoT read confirmed text `issue614-fs-watch-20260601T0806` and SHA256 `F279825DCF260AB5D1B5C3B3F182D95EBE91D677AD3ACFF90612C19842C18053`.
  - `observe_delta` returned one `/fs` `filesystem_summary_changed` delta from `event_count=0` to created file summary; separate `storage_inspect` read CF_KV 2->3 with baseline, delta, and head sample keys.
- Cleanup: cancelled main subscriptions, called real `release_all` on both isolated daemons, stopped curl PID `82448`, main daemon PID `82340`, FS daemon PID `77940`, and Luanti PID `36460`. Port `7841` has only TIME_WAIT rows. Windows still reports an orphan `127.0.0.1:7840 LISTENING` row owned by non-existent PID `82340`; `Get-Process`, `tasklist`, and CIM all show no such process, so future isolated runs should use a new port if the kernel row persists.
- Final supporting checks after FSV passed:
  - `cargo fmt --check`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp server::reality::tests --bin synapse-mcp -- --nocapture` (14 passed)
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture` (3 passed)
  - `cargo build --release -p synapse-mcp -j 2`
  - `git diff --check` exited 0 with line-ending warnings only.
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46350848`, SHA256 `18F213F8799AFA64ACCB31F3C3F07F98D40ADF3E081D3C05B256A8FC957BEED4`, `LastWriteTimeUtc=2026-06-01T13:14:38Z`.
- Diff review completed for `crates/synapse-mcp/src/server/reality.rs` and `STATE/*`.
- Next: commit with `[skip ci]`, post #614 RESOLVED evidence, close #614, refresh the open queue, and continue to the next open issue.

## 2026-06-01T07:40:09-05:00
- Active issue remains #614 `scenario(stress): reality baseline→delta→audit full loop across all sensors`.
- Post-compaction wake-up re-read completed: repo doctrine, external wake-up doctrine, `AGENTS.md`, `STATE/*`, #614, #594, #351, live open queue, and git status/log/branch.
- Live GitHub readback: #614 is still open with only the START comment; parent #594 remains open; #351 is closed and confirms manual FSV/no CI.
- Git readback before the final #614 patch: `main...origin/main`, HEAD `b66b78e`, modified only `crates/synapse-mcp/src/server/reality.rs` and state files.
- Wired configured Synapse MCP client readback succeeded: `health ok=true`, `storage_inspect` read CF counts/retention, `reflex_list include_expired=true` and `reflex_history` read the persisted cancelled reflex, and `observe depth=0` read the live VS Code foreground.
- Additional #614 profile-change edge patch:
  - `observe_delta` now uses the requested profile only to select the stored head; after observing live state it returns `profile_changed` rebase guidance when the observed profile is known and differs.
  - If the live observation cannot resolve a profile, it keeps the requested head profile instead of inventing an `unprofiled` profile switch.
  - Added supporting regression `observe_delta_reports_profile_changed_for_requested_head_mismatch`.
- Supporting checks passed after the final patch:
  - `cargo fmt`
  - `cargo test -p synapse-mcp observe_delta_reports_profile_changed_for_requested_head_mismatch --bin synapse-mcp -- --nocapture`
  - `cargo test -p synapse-mcp server::reality::tests --bin synapse-mcp -- --nocapture` (14 passed)
  - `cargo fmt --check`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
- Stopped stale pre-patch isolated #614 daemon PID `75352`; port `127.0.0.1:7840` had no remaining listener afterward.
- Release build passed after the final patch: `cargo build --release -p synapse-mcp -j 2`; binary `target\release\synapse-mcp.exe`, length `46350848`, SHA256 `319FC6F5942ABF272EDCCA7A1EEF7970EE7AE0C7CB6A11A515F681B74F6854A1`, `LastWriteTimeUtc=2026-06-01T12:39:53Z`.
- Next: launch a fresh isolated #614 daemon from this rebuilt binary and run manual MCP FSV for baseline, delta, SSE, audit, physical sensor changes, and required edge cases.

## 2026-06-01T07:13:00-05:00
- #613 `scenario(stress): subscribe firehose - 4096 ring, EVENTS_DROPPED, one-per-event, deep filters` is closed.
  - Implementation commit: `e95a656 fix(mcp): harden subscribe firehose path (#613) [skip ci]`.
  - State commit: `e792ea0 docs(state): record issue 613 evidence [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/613#issuecomment-4592454588
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T12:12:21Z`.
- Post-close git readback: `main...origin/main`, clean, HEAD `e792ea0`.
- Refreshed live open queue now lists #594 plus #595-#604 and #614-#634.
- Active issue is now #614 `scenario(stress): reality baseline→delta→audit full loop across all sensors`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/614#issuecomment-4592477025
  - Issue requires proving the delta-first reality model end to end across sensor feeds: baseline rows, foreground/focus/UIA/HUD/entity/audio/clipboard/filesystem/diagnostics changes, ordered `observe_delta` cursor walks, `reality_delta` SSE events, `CF_KV/reality/delta/*` rows, and no-change/missing/stale/future cursor edges.
  - Post-compaction wired Synapse MCP client readback succeeded: `health ok=true`, `storage_inspect` returned CF counts and reality retention prefixes, `reflex_list include_expired=true` returned the prior cancelled reflex, `reflex_history limit=5` read terminal rows, and `observe depth=0` returned the live VS Code foreground/focused SoT.
  - Code inspection found and patched three #614 reality-tool robustness gaps in `crates/synapse-mcp/src/server/reality.rs`:
    - `reality_baseline` with omitted `profile_id`, omitted `epoch_id`, and `force_new_epoch=false` checked only `reality/head/v1/unprofiled` before observing, so it could create a fresh epoch for the active observed profile instead of reusing that profile's existing head.
    - `observe_delta.since_epoch` was compared to the head epoch without validating it as a reality key segment.
    - `depth` and `max_elements` had JSON-schema ranges but server-side capture silently clamped bypassed out-of-range values instead of failing closed.
  - Added focused supporting regressions for omitted-profile baseline reuse, invalid `since_epoch`, and out-of-range snapshot params.
  - Focused checks passed: `cargo fmt`; `cargo test -p synapse-mcp reality_baseline_reuses_observed_profile_when_profile_id_is_omitted --bin synapse-mcp -- --nocapture`; `cargo test -p synapse-mcp observe_delta_edges_return_rebase_or_fail_closed --bin synapse-mcp -- --nocapture`; `cargo test -p synapse-mcp reality_tools_reject_out_of_range_snapshot_params --bin synapse-mcp -- --nocapture`.
  - Broader supporting checks passed: `cargo test -p synapse-mcp server::reality::tests --bin synapse-mcp -- --nocapture` (13 tests); `cargo fmt --check`; `cargo check -p synapse-mcp -j 2`; `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`.
  - Release build passed: `cargo build --release -p synapse-mcp -j 2`; binary `target\release\synapse-mcp.exe`, length `46350336`, SHA256 `2C3D7E51AADF23F54B90CE32607813E848CD699327F95BDD4E69F4B6F7164229`, `LastWriteTimeUtc=2026-06-01T12:25:35Z`.
  - Host-sensor setup readback: Luanti binary exists at `%LOCALAPPDATA%\synapse\benchmarks\luanti\engine\5.16.1\luanti-5.16.1-win64\bin\luanti.exe`, EverQuest and Minecraft launcher binaries also exist, but no game process is currently running; Notepad PID `60556` is currently available. Luanti profile has real capture-derived HUD/entity baseline hooks.
  - Next: launch a repo-built isolated #614 daemon with audio enabled and run strict Inspector plus manual MCP FSV.

## 2026-06-01T07:09:36-05:00
- Active issue #613 `scenario(stress): subscribe firehose - 4096 ring, EVENTS_DROPPED, one-per-event, deep filters` has implementation and manual FSV evidence complete; commit/issue closure is next.
- #613 patch in worktree:
  - `EventFilter::validate` now validates `Data` filters, rejects invalid JSON Pointer paths, and rejects invalid regex patterns instead of silently matching false.
  - SSE `/events` reconnect now reuses an explicitly requested empty subscription when `Last-Event-ID: 0`, instead of creating a new unfiltered All subscription.
  - SSE ring overflows now increment `events_dropped_for_subscriber` / `EVENTS_DROPPED_METRIC` when the per-subscription ring drops old buffered events.
  - Supporting regressions cover data-filter validation, strict `subscribe` validation, empty-subscription reconnect, and 5000-event ring overflow/drop accounting.
- Final patched manual MCP FSV evidence is under `.runs\613\subscribe-firehose-fsv-20260601T062230-patched`:
  - Repo-built daemon PID `32356`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, bind `127.0.0.1:7839`, isolated DB `.runs\613\subscribe-firehose-fsv-20260601T062230-patched\db`, watch root `.runs\613\subscribe-firehose-fsv-20260601T062230-patched\watch`.
  - Process/socket/auth/client-parity precondition passed: process path readback matched repo release binary; netstat showed `127.0.0.1:7839 LISTENING`; unauth `/health` returned `401`; auth `/health ok=true`; official MCP Inspector strict `tools/list` returned 80 tools including `health`, `subscribe`, `subscribe_cancel`, `observe_delta`, `reality_baseline`, `act_clipboard`, `act_run_shell`, `storage_inspect`, and `replay_record`.
  - Baseline SoT: `reality_baseline` stored `reality/baseline/v1/notepad/issue613-patched` and `reality/head/v1/notepad` in `CF_KV`.
  - One-per-event happy path: real MCP `act_clipboard`, `act_run_shell`, and `observe_delta` triggers produced physical clipboard/file SoTs matching marker `issue613-patched-oneper-20260601T062403456`; SSE subscription `019e82ec-ebf5-7943-884e-03590d0a05f2` delivered exactly 3 `synapse/event` frames, stream seq `1,2,3`, event seq `1,2,3`, paths `/focused,/clipboard,/fs`, no loss; stats after read `ring_len=3`, `dropped_total=0`.
  - 8-deep filter path: subscription `019e82ee-5d56-72f2-92c0-00e3c4a73063` accepted a depth-8 filter using `kind`, `data in_set`, `exists`, and regex behind nested `not`; `observe_delta` published 4 deltas (`/foreground/window_title_sha256,/focused,/clipboard,/fs`), while SSE delivered only the matching `/clipboard` and `/fs` frames.
  - Slow-consumer/backpressure firehose: subscription `019e82ef-c53f-7e13-ae2c-cfea7dbd3ae8`; manual HTTP event ingress with `SYNAPSE_HTTP_SSE_MANUAL=1` posted 5000 known `issue613.firehose` events. Publish response read `matched=5000 queued=5000 dropped=904`; stats read `ring_len=4096`, `oldest_event_seq=904`, `latest_event_seq=4999`, `dropped_total=904`, `events_dropped_for_subscriber=904`, `lossy_pending=true`; replay read 1 `subscription_started` lossy frame plus 4096 `synapse/event` frames, first event seq `904` lossy true and last event seq `4999`.
  - Edge cases: depth-9 filter, invalid regex `[`, invalid data path `field`, and `buffer_size=4095` all failed through strict Inspector with MCP error and subscriber count unchanged; empty filter All delivered known event `issue613.empty_filter` seq `613000`; subscribe-then-immediate-cancel returned `cancelled=true`, stats endpoint returned 404 before and after publishing a matching event.
  - Cleanup: cancelled active subscriptions through real MCP `subscribe_cancel`, `health` read `sse_subscribers=0`, `release_all` returned zero held inputs, daemon PID `32356` stopped, port `7839` no longer listening.
- Supporting checks passed:
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
  - `cargo build --release -p synapse-mcp -j 2` (rerun passed after one interrupted attempt)
- Final release binary readback: `target\release\synapse-mcp.exe`, length `46359552`, SHA256 `426E96F4CA1C07D92433284FEBD39A161722C256133265AD6472B4E1D51DB67C`, `LastWriteTimeUtc=2026-06-01T12:09:18.7698237Z`.
- Next: commit #613 with `[skip ci]`, post RESOLVED evidence, close #613, refresh live queue, and continue to #614 unless the queue changes.

## 2026-06-01T05:44:31-05:00
- #612 `scenario(stress): hold_move / hold_button / combo reflex lifetimes` is closed.
  - Commit: `db761fe fix(reflex): resolve hold lifetime stress path (#612) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/612#issuecomment-4591828569
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T10:43:56Z`.
- Post-close git readback: `main...origin/main`, clean, HEAD `db761fe`.
- Refreshed live open queue now lists #594 plus #595-#604 and #613-#634.
- Active issue is now #613 `scenario(stress): subscribe firehose - 4096 ring, EVENTS_DROPPED, one-per-event, deep filters`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/613#issuecomment-4591831842
  - Issue requires proving the real event stream under storm conditions: snapshot_first, many event kinds, one notification per event, 4096 ring bound, `EVENTS_DROPPED`, 8-level-deep filters, filter-depth-9 rejection, subscribe/immediate cancel, slow consumer backpressure drops, and empty filter/All.
  - Required SoTs include the real MCP daemon process/socket/auth/session/tools-list, SSE or MCP event delivery state, delivered event counts, event bus drop accounting, storage/event rows, and physical causes such as clipboard changes, filesystem writes, process churn, and UIA changes.
  - Next: inspect subscribe/SSE/event-bus implementation and existing tests before patching or launching an isolated #613 daemon.

## 2026-06-01T05:33:52-05:00
- Active issue remains #612 `scenario(stress): hold_move / hold_button / combo reflex lifetimes`.
- Patched the final #612 defect found in manual cancel-expired evidence:
  - Before patch, `reflex_cancel` returned `cancelled=false, reason=not_found` for an already-expired one-shot combo even though `reflex_list include_expired=true` still read the reflex as `expired`.
  - Root cause: `reflex_cancel` consulted only the live scheduler status snapshot, while `reflex_list include_expired=true` merges terminal statuses from persisted `CF_REFLEX_AUDIT`.
  - Patch: `ReflexRuntime::cancel` now checks the same persisted terminal audit status before returning `NotFound`; expired/action-denied terminal statuses return `AlreadyExpired`, and historical cancelled statuses retain the existing cancelled outcome.
  - Added supporting regression `cancel_expired_reflex_restored_from_audit_reports_already_expired`.
- Supporting checks passed after the patch:
  - `cargo fmt`
  - `cargo test -p synapse-reflex cancel_expired_reflex_restored_from_audit_reports_already_expired --lib -- --nocapture`
  - `cargo check -p synapse-reflex -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture`
  - `cargo build --release -p synapse-mcp -j 2`
- Final broad supporting checks also passed:
  - `cargo fmt --check`
  - `git diff --check` (line-ending warnings only)
  - `cargo check -p synapse-action -j 2`
  - `cargo check -p synapse-reflex -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-action recovery_log_skips_duplicate_logical_holds --lib -- --nocapture`
  - `cargo test -p synapse-reflex cancel_expired_reflex_restored_from_audit_reports_already_expired --lib -- --nocapture`
  - `cargo test -p synapse-reflex --test hold_move_behavior -- --nocapture` (8 passed)
  - `cargo test -p synapse-reflex --test bus_behavior -- --nocapture` (5 passed)
  - `cargo test -p synapse-reflex --test scheduler_behavior -- --nocapture` (26 passed)
  - `cargo test -p synapse-reflex --test combo_behavior -- --nocapture` (7 passed)
  - `cargo test -p synapse-mcp --test m3_reflex_register_tool -- --nocapture` (1 passed)
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture` (3 passed)
  - `cargo build --release -p synapse-mcp -j 2`
- Final release build readback after patch: `target\release\synapse-mcp.exe`, length `46342656`, SHA256 `6898E30AE4FAE8519499B0BB91436E3C0B44D218BE03539EA1D60957C1281BF1`, timestamp `2026-06-01T05:41:10-05:00`.
- Manual MCP FSV rerun for the cancel already-expired edge is captured under `.runs\612\hold-lifetime-fsv-20260601T0530-cancel-expired`:
  - Fresh repo-built daemon PID `53088`, bind `127.0.0.1:7838`, isolated DB `.runs\612\hold-lifetime-fsv-20260601T0530-cancel-expired\db`.
  - Process/socket/auth precondition passed: process path `C:\code\Synapse\target\release\synapse-mcp.exe`; `127.0.0.1:7838 LISTENING`; unauth `/health` returned `401`; auth `/health ok=true`; operator hotkey disabled by env; storage DB path isolated.
  - Official MCP Inspector strict `tools/list` passed with 80 tools and required `health`, `reflex_register`, `reflex_cancel`, `reflex_list`, `reflex_history`, `storage_inspect`, `release_all`, and `act_combo` present.
  - SoT before cancel: OS `GetAsyncKeyState(P)=false`, `action_recovery.jsonl` absent, `reflex_list include_expired=true` showed combo `019e82be-1a45-7d00-a817-22a9d7248818` state `expired`, `fire_count=1`, `last_error_code=REFLEX_LIFETIME_EXPIRED`.
  - Trigger: real MCP Inspector `tools/call reflex_cancel reflex_id=019e82be-1a45-7d00-a817-22a9d7248818`.
  - SoT after cancel: response `cancelled=false, reason=already_expired`; OS P still false; recovery ledger absent; `reflex_list include_expired=true` still shows the reflex `expired`; `reflex_history` rows are `reflex_lifetime_expired` and `reflex_registered`; `storage_inspect` reads `CF_REFLEX_AUDIT=2`, `CF_ACTION_LOG=1`.
  - Cleanup `release_all` returned zero releases; daemon PID `53088` stopped; port `7838` has no LISTEN row, only TIME_WAIT.
- Next: commit/push `[skip ci]`, post #612 RESOLVED evidence, close #612, refresh the open queue, and continue to #613 unless the queue changes.

## 2026-06-01T04:51:40-05:00
- Active issue remains #612 `scenario(stress): hold_move / hold_button / combo reflex lifetimes`.
- Patched an additional #612 defect found during fresh manual reassert evidence:
  - `hold_move re_assert=true` kept W down and the recovery ledger bounded, but the cancel-time KeyUp was hidden behind a flood of per-tick reassert KeyDowns.
  - `reflex_cancel` returned `cancelled=true`; separate physical read after 1s still showed W down and `action_recovery.jsonl` still had one W `key_held` row.
  - `release_all` then released W and removed the ledger, confirming the defect was in the reassert/cancel interaction rather than the action backend.
- Root-cause patch:
  - `HoldMoveController` now rate-limits reassert dispatch to a 50 ms interval instead of every scheduler tick.
  - Focused controller regression now proves early ticks do not enqueue extra KeyDowns and the interval tick does reassert.
- Supporting checks passed after the patch:
  - `cargo fmt`
  - `cargo test -p synapse-reflex --test hold_move_behavior hold_move_reasserts_keydown_while_holding_when_enabled -- --nocapture`
  - `cargo test -p synapse-action recovery_log_skips_duplicate_logical_holds --lib -- --nocapture`
  - `cargo check -p synapse-reflex -j 2`
  - `cargo check -p synapse-action -j 2`
  - `cargo build --release -p synapse-mcp -j 2`
- New release binary readback: `target\release\synapse-mcp.exe`, length `46334976`, SHA256 `8D9AFDC19BD14594C0B42877B6D2DE8F06DB6B7AA354F83709783C4E7701856D`, timestamp `2026-06-01T04:51:32-05:00`.
- The old patched evidence daemon PID `60632` was stopped; port `7836` has no LISTEN row.
- Next: launch another fresh #612 daemon with the reassert-throttled binary, rerun strict Inspector tools-list, and redo behavior FSV from a clean baseline.

## 2026-06-01T04:36:30-05:00
- Active issue remains #612 `scenario(stress): hold_move / hold_button / combo reflex lifetimes`.
- Launched initial isolated #612 daemon PID `47904`, bind `127.0.0.1:7835`, run dir `.runs\612\hold-lifetime-fsv-20260601T042259`, token `synapse-612-token`.
  - Process/socket/auth readbacks passed via `Get-Process`, `netstat`, unauth `/health=401`, auth `/health ok=true`.
  - Official MCP Inspector `0.21.2` strict `tools/list` passed with 80 tools including #612 required tools.
- Manual FSV happy path started:
  - `hold_move` UntilCancelled registered Shift; OS `GetAsyncKeyState` read Shift down and `action_recovery.jsonl` had one `key_held` row.
  - `reflex_cancel` then released Shift; OS read Shift up and recovery ledger absent.
- The `re_assert=true` run exposed a real defect: repeated reassert KeyDowns correctly kept W physically down, but `action_recovery.jsonl` grew with many duplicate `key_held` rows for the same logical key.
- Patch added in `synapse-action` recovery ledger:
  - `append_recovery_event_at` now reads the logical ledger before append and skips no-op hold/release events when they would not change recovered state.
  - Added regression `recovery_log_skips_duplicate_logical_holds`.
- Supporting checks passed after the patch:
  - `cargo fmt`
  - `cargo test -p synapse-action recovery_log_skips_duplicate_logical_holds --lib -- --nocapture`
  - `cargo check -p synapse-action -j 2`
  - `cargo build --release -p synapse-mcp -j 2`
- New release binary readback: `target\release\synapse-mcp.exe`, length `46334976`, SHA256 `1DFD23F3EE49AB5176825096FDFB2D8E59B452454632B65D0C1D5F2D45E2F430`, timestamp `2026-06-01T04:35:53.8023304-05:00`.
- Old #612 daemon PID `47904` was stopped; port `7835` has no LISTEN row, only TIME_WAIT.
- Next: launch a fresh #612 daemon on a new port with the patched binary, re-run strict Inspector tools-list, then redo/continue manual FSV including re_assert with bounded ledger readback.

## 2026-06-01T04:11:07-05:00
- Active issue remains #612 `scenario(stress): hold_move / hold_button / combo reflex lifetimes`.
- Post-compaction wake-up was completed again:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #612, #594, #351, the live open queue, and git status/log/branch.
  - Live open queue remains #594 plus #595-#604 and #612-#634.
  - Configured wired `mcp__synapse` client loaded and executed `health`, `storage_inspect`, `reflex_list include_expired=true`, `reflex_history limit=5`, and `observe depth=0`.
- #612 supporting MCP timeout root cause was found and patched:
  - `step_aim_track` read cursor position and sampled the M1 aim-track target source before proving the current reflex slot actually had an `aim_track` controller.
  - With many `on_event` reflexes, every scheduler tick performed a depth-2 UIA snapshot per non-aim reflex; `reflex_register` waits for the old scheduler to stop on each restart, so repeated MCP registrations slowed until the strict client timed out.
  - Patch now returns from `step_aim_track` before cursor/M1 reads when the slot has no aim-track controller.
  - Added supporting regression `on_event_ticks_do_not_sample_aim_track_target_source`.
  - Removed the temporary diagnostic `logs.keep()` / `eprintln!` from `m3_reflex_register_tool.rs`; retained useful timeout context.
- Supporting checks passed after this patch:
  - `cargo fmt`
  - `cargo test -p synapse-reflex --test scheduler_behavior on_event_ticks_do_not_sample_aim_track_target_source -- --nocapture`
  - `cargo check -p synapse-reflex -j 2`
  - `cargo test -p synapse-mcp --test m3_reflex_register_tool -- --nocapture`
  - `cargo fmt --check`
  - `cargo test -p synapse-reflex --test bus_behavior -- --nocapture` (5 passed)
  - `cargo test -p synapse-reflex --test scheduler_behavior -- --nocapture` (26 passed)
  - `cargo test -p synapse-reflex --test hold_move_behavior -- --nocapture` (8 passed)
  - `cargo test -p synapse-reflex --test combo_behavior -- --nocapture` (7 passed)
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture` (3 passed)
  - `cargo build --release -p synapse-mcp -j 2`
  - `git diff --check` exited 0 with line-ending warnings only.
- Release binary readback: `target\release\synapse-mcp.exe`, length `46333440`, SHA256 `D1029364B92C10FA69690F96AB47DAE9391B1ECE38C0C4FFC0E2CE9C3C86EE20`, timestamp `2026-06-01T09:17:27.5484285Z`.
- Diff review completed for current #612 source/test/state changes. `STATE/RECOVERY_NOTES.md` was rewritten to a single current #612 resume point to avoid stale compaction instructions.
- Next: launch an isolated repo-built daemon for manual MCP FSV.

## 2026-06-01T03:37:29-05:00
- Active issue remains #612 `scenario(stress): hold_move / hold_button / combo reflex lifetimes`.
- Code inspection found two real #612 gaps before runtime evidence:
  - `hold_move` accepted and documented `re_assert`, but no runtime code used it.
  - `reflex_cancel` marked hold reflexes cancelled without queuing the physical key/button release actions, so held inputs could remain down until `release_all` or auto-release.
- Patch in worktree:
  - `HoldMoveController` now emits `HoldMoveOutput::Reasserted` and re-dispatches `KeyDown` for configured keys while holding when `re_assert=true`.
  - Scheduler stateful handling counts reasserted actions as progress without incrementing reflex fire count after the initial hold.
  - `ReflexRuntime::cancel` now queues physical release actions for active `hold_move` and `hold_button` definitions before removing the reflex and writing the cancellation audit.
  - Added focused supporting coverage for reasserted keydown and runtime cancel queuing keyup.
- Supporting checks passed after this patch:
  - `cargo fmt`
  - `cargo check -p synapse-reflex -j 2`
  - `cargo test -p synapse-reflex --test hold_move_behavior -- --nocapture` (8 passed)
  - `cargo check -p synapse-mcp -j 2`
- Next: inspect the diff, run broader supporting checks, build release, then launch an isolated #612 daemon for manual MCP FSV.

## 2026-06-01T03:31:50-05:00
- #611 `scenario(stress): on_event reflexes - HUD/audio/entity triggers + debounce` is closed.
  - Commit: `5723393 fix(reflex): resolve on-event stress path (#611) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/611#issuecomment-4590866021
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T08:31:17Z`.
  - Post-close git readback before state update: `main...origin/main`, clean.
- Refreshed live open queue now lists #594 plus #595-#604 and #612-#634.
- Active issue is now #612 `scenario(stress): hold_move / hold_button / combo reflex lifetimes`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/612#issuecomment-4590869661
  - Issue requires proving continuous hold and one-shot combo reflexes across `UntilCancelled`, `OneShot`, `Duration(ms)`, `UntilEvent`, and `UntilDeadline`.
  - Required paths: `hold_move` with/without `re_assert`, `hold_button` for mouse/gamepad hold, one-shot combo auto-cancel, `reflex_cancel` stopping holds, focus-loss reassert, Duration minimum, already-past deadline, cancel already-expired, and empty/boundary/structurally invalid params.
  - Required SoTs: OS key/button state, controller/XInput or tester state where applicable, target app/game/tester UI, `CF_REFLEX_AUDIT`, `CF_ACTION_LOG`, `reflex_list`, `reflex_history`, daemon process/socket/log state, and cleanup `release_all`.
  - Next: inspect hold/reflex lifetime implementation and existing tests before patching or launching an isolated #612 daemon.

## 2026-06-01T03:29:27-05:00
- Post-compaction wake-up for #611 was completed again:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #611, #594, #351, the live open queue, and git status/log/branch.
  - Live open queue still lists #594 plus #595-#604 and #611-#634; #611 has only the START comment and remains open until evidence is posted.
  - Configured wired `mcp__synapse` client loaded real tools and executed `health`, `storage_inspect`, `reflex_list include_expired=true`, `reflex_history limit=5`, and `observe depth=0 include focused/elements/events/diagnostics max_elements=5` successfully.
- Final #611 supporting checks passed on the current worktree after the manual behavior evidence:
  - `cargo fmt --check`
  - `cargo check -p synapse-storage -j 2`
  - `cargo check -p synapse-reflex -j 2`
  - `cargo check -p synapse-mcp -j 2`
  - `cargo test -p synapse-reflex --test scheduler_behavior -- --nocapture` (24 passed)
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture` (3 passed)
  - `cargo test -p synapse-mcp reality_tools_persist_delta_and_publish_event -- --nocapture`
  - `cargo test -p synapse-mcp observe_delta_reads_after_cursor_past_first_page -- --nocapture`
  - `cargo test -p synapse-core error_codes -- --nocapture` (2 relevant tests passed)
  - `cargo build --release -p synapse-mcp -j 2`
  - `git diff --check` exited 0 with only LF/CRLF warnings.
- Final release binary readback: `C:\code\Synapse\target\release\synapse-mcp.exe`, length `46325760`, SHA256 `1D291BB8B00A80377F450E2C285250F1045CC6A663B88F5BAD9990A8F434B7A1`, timestamp `2026-06-01T08:28:59Z`.
- Diff review completed after compaction for the #611 source changes and state files; changes are scoped to debounced on_event audit/event rows, generic `UntilEvent` expiry/validation, reality_delta event payload/cursor paging, storage prefix scan support, and focused supporting regressions.
- Current next step: commit/push with `[skip ci]`, post #611 RESOLVED evidence, close #611, refresh the open queue, and continue to #612 or the next live issue.

## 2026-06-01T03:09:14-05:00
- #611 manual FSV behavior evidence is complete on the fresh patched isolated daemon that was PID `47056`, bind `127.0.0.1:7834`, DB `.runs\611\on-event-fsv-20260601T012006\db3`, profile dir `.runs\611\on-event-fsv-20260601T012006\profiles`, token `synapse-611-token`, release binary SHA256 `F57B66F004780CFE7EB5F83DB1F24125497A360F91EA92971BCE7A0F11DF29E0`.
- Post-compaction runtime preconditions were re-read: process `47056` existed from `target\release\synapse-mcp.exe`, `127.0.0.1:7834` was listening, unauth `/health` returned `401`, auth `/health` returned `ok=true` with audio loopback running, and official MCP Inspector strict `tools/list` succeeded with 80 tools including `reflex_register`, `observe_delta`, `reality_baseline`, `audio_tail`, `act_type`, and `reflex_cancel`.
- #611 FSV happy/edge evidence captured under `.runs\611\on-event-fsv-20260601T012006`:
  - HUD threshold path: `67_register_hud_patched.json` + `68/69` F9/delta triggered HUD HP `10 -> 4`; `70_observe_after_hud_patched.json` read ActionLog `READY HUD2_OK `; `71_history_hud_patched.json` read `reflex_fired`.
  - Debounce edge: `76_register_debounce_patched.json`, first low `77/78/79/80` produced `READY DEB6_OK ` and `reflex_fired`; second low inside debounce `81-86` left ActionLog unchanged and `86_history_debounce_second_patched.json` read `reflex_debounced` with `REFLEX_DEBOUNCED`, `reason=debounce_window`, `suppressed_count=1`.
  - Never-match edge: `91_register_never_patched.json` filter `/after/parsed <= -1`, `92/93` produced real HUD delta but `94` ActionLog stayed `READY ` and `95` history had only `reflex_registered`.
  - 8-deep/boundary edge: `100_register_deep_patched.json` accepted priority `1000` and depth-8 filter; `101/102` triggered HP low; `103` ActionLog `READY DEEP_OK `; `104` history `reflex_fired`.
  - `UntilEvent` lifetime edge: `109_register_life_patched.json`; `110/111/112/113` first low fired `LIFE_OK`; `114/115/116/117` HP reset expired the reflex with `REFLEX_LIFETIME_EXPIRED`; `118/119/120/121` later low did not add a second action.
  - Empty/invalid edges: `123_empty_on_event_register.stderr.txt` failed with `on_event reflex requires then`; `125_invalid_empty_and_register.stderr.txt` failed with `event filter 'and' must contain at least one argument`; `122/124/126` reflex-list readbacks kept active count `0`.
  - Audio transient path: first RMS-based attempt correctly did not fire; retry used the actual audio transient signal. `149_register_audio_retry.json`, async WAV playback plus real `observe_delta` in `150_audio_retry_delta_during_wav.json` read `audio_summary_changed` with latest event `music_ended -> loud_transient` and RMS `-120000 -> -8726`; `151_observe_audio_retry_after.json` read ActionLog `READY AUDIO2_OK `; `152_history_audio_retry.json` read `reflex_fired`.
  - Entity appear path: real Luanti launch `154/155` proved actual Luanti entities existed. Because actual Luanti ignored software keyboard in this session, a synthetic physical Luanti-shaped WPF target was launched as `luanti.exe` for target-app action SoT. `189_entity_final_baseline_without_entities.json` excluded entities; `190_register_entity_final.json` registered a broad `entity_appeared` on_event reflex with debounce; `191_entity_final_delta.json` read `entity_appeared` for `luanti_crosshair_region` and `luanti_hotbar_region`; `192_observe_entity_final_after.json` read target ActionLog `ENTITY_READY ENTITY_FINAL_OK `; `193_history_entity_final.json` read `reflex_fired`, `reflex_debounced`, and `REFLEX_DEBOUNCED`.
- Cleanup evidence: `194_reflex_list_before_cleanup.json` showed two active reflexes; `195_*` cancelled them; `196_release_all_cleanup.json` returned zero held inputs; `197_reflex_list_after_cancel_cleanup.json` showed `active=0`, `cancelled=9`, `expired=1`; `198_storage_after_cleanup.json` read `CF_ACTION_LOG=67`, `CF_REFLEX_AUDIT=37625`, `CF_EVENTS=52`, `CF_OBSERVATIONS=51`. Stopped FSV-owned WPF/fake Luanti target processes and isolated daemon PID `47056`; `127.0.0.1:7834` has no LISTEN row, only TIME_WAIT.
- Current next step: run final supporting checks, inspect the diff, update this state if any check exposes a defect, then commit/push `[skip ci]`, post #611 RESOLVED evidence, close #611, refresh the open queue, and continue to #612 or the next live issue.

## 2026-06-01T02:25:39-05:00
- #611 manual FSV continued after compaction on isolated daemon PID `65300` / `127.0.0.1:7833`.
- Re-verified process/socket/auth/health and official MCP Inspector strict `tools/list` for the isolated daemon; required tools are still present.
- While running the filter-never-matches edge, `observe_delta` persisted/published HUD seq `22` but returned only seq `21` when called with `since_seq=20` and `max_deltas=20`; a follow-up read from `since_seq=21` still returned no delta even though the head was `22`.
- Root cause patched: `read_delta_rows_after` previously scanned the first `max_deltas + 1` rows under the reality journal prefix and filtered `since_seq` afterward, so later pages disappeared once the prefix had more earlier rows than the page size. Added start-key prefix scanning through `synapse-storage`, `synapse-reflex`, and `observe_delta`, plus regression `observe_delta_reads_after_cursor_past_first_page`.
- Supporting checks passed after the cursor patch:
  - `cargo test -p synapse-mcp observe_delta_reads_after_cursor_past_first_page -- --nocapture`
  - `cargo fmt --check`
  - `cargo check -p synapse-storage -j 2`
  - `cargo check -p synapse-reflex -j 2`
  - `cargo check -p synapse-mcp -j 2`
- Next: stop old isolated daemon PID `65300`, rebuild `target\release\synapse-mcp.exe`, launch a fresh patched isolated daemon/profile on a new port, re-run strict Inspector tools-list, then restart #611 FSV edges from a fresh baseline so the cursor-return evidence is valid.

## 2026-06-01T01:37:37-05:00
- Post-compaction wake-up for #611 was completed again:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #611, #594, #351, the live open queue, and git status/log/branch.
  - Live open queue remains #594 plus #595-#604 and #611-#634; #611 has only the START comment and remains active.
  - Git readback: branch `main`, `main...origin/main`, dirty only #611 source/state changes.
  - Configured wired `mcp__synapse` client loaded and executed `health`, `storage_inspect`, `reflex_list`, `reflex_history`, and `observe`.
- #611 isolated daemon/runtime readback after compaction:
  - Active repo-built daemon PID `65300`, bind `127.0.0.1:7833`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, isolated DB `.runs\611\on-event-fsv-20260601T012006\db2`, profile dir `.runs\611\on-event-fsv-20260601T012006\profiles`.
  - WPF target PID `43360`, title `Synapse611 HUD FSV`, still responding.
  - Process/socket/auth SoTs passed: `127.0.0.1:7833 LISTENING 65300`; unauth `/health` returned 401; authenticated `/health` returned ok with audio loopback running and active profile `synapse611.hud`.
  - Official MCP Inspector strict `tools/list` recheck through `http://127.0.0.1:7833/mcp` listed 80 tools and required `reflex_register`, `observe_delta`, `reality_baseline`, `audio_tail`, `act_press`, and `act_type`; saved to `.runs\611\on-event-fsv-20260601T012006\32_tools_list_recheck_7833.json`.
  - Cancelled stale debounce reflex `019e81e1-aa50-7572-98ad-f25f1e8b03ba` after compaction; `reflex_history` showed exactly registered/fired/cancelled for that reflex, so the large `CF_REFLEX_AUDIT` size is scheduler tick-late audit volume, not repeated debounce rows.
  - Current WPF UI SoT from Inspector `observe`: HP label `HP 04`, ActionLog value `READY DEB_OK `.
- Next: reset/clear the WPF target, run a fresh long-window debounce edge, then continue #611 with never-match, depth-8, UntilEvent, invalid/empty/boundary, audio transient, and entity-appear evidence.

## 2026-06-01T01:25:30-05:00
- #611 isolated runtime FSV precondition is established:
  - Repo-built daemon PID `61628`, bind `127.0.0.1:7832`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, SHA256 `D7F6DD4A11A3A0C7353DDFA1B40BF21F73537B99735645AD5C1F15AC6118C61B`.
  - Run dir `.runs\611\on-event-fsv-20260601T012006`; isolated DB `db`, logs `logs`, APPDATA `appdata`, profiles copied to `profiles` with temporary `synapse611.hud` WPF target profile.
  - Process SoT: PID `61628` running/responding; socket SoT: `127.0.0.1:7832 LISTENING 61628`.
  - HTTP auth SoT: unauthenticated `/health` returned 401; authenticated `/health` returned 200.
  - Official MCP Inspector strict `tools/list` succeeded through `http://127.0.0.1:7832/mcp`, wrote `.runs\611\on-event-fsv-20260601T012006\inspector_tools_list.json`, and listed 80 tools including `reflex_register`, `observe_delta`, `reality_baseline`, `audio_tail`, `act_type`, and `act_launch`.
- Next: launch the temporary WPF HUD target through Inspector `act_launch`, read profile/HUD/action-log SoTs, then register and trigger real `on_event` reflexes for HUD, audio, entity/debounce/lifetime/edge cases with separate `reflex_history`, `storage_inspect`, UI, audio, and reality-row readbacks.

## 2026-06-01T00:47:51-05:00
- Post-compaction wake-up for #611 was completed:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #611, #594, #351, live open queue, and git status/log/branch.
  - Live open queue is #594 plus #595-#604 and #611-#634.
  - Git readback before edits: `main...origin/main`, clean, HEAD `60dcb57 docs(state): record issue 610 closure [skip ci]`.
  - Configured wired `mcp__synapse` client loaded the real tool surface: `health`, `storage_inspect`, `reflex_list`, `reflex_history`, and `observe` succeeded.
- Active issue remains #611 `scenario(stress): on_event reflexes - HUD/audio/entity triggers + debounce`.
- #611 implementation patch is in the worktree:
  - `on_event` debounce suppressions now publish `reflex_debounced` events and persist `CF_REFLEX_AUDIT` rows with `REFLEX_DEBOUNCED`, reason (`same_tick` or `debounce_window`), debounce window, trigger event, and coalesced suppressed count.
  - Generic action/on_event reflex `UntilEvent` lifetimes are validated at scheduler spawn and expire from real bus events before future dispatches, emitting/persisting `REFLEX_LIFETIME_EXPIRED`.
  - `reality_delta` bus events now use `source=perception` and include the redacted compact `before` and `after` values already persisted in the delta row, so real on_event filters can match HUD/audio/entity delta payloads.
  - Supporting regression coverage was added for same-tick debounce, later-window debounce, UntilEvent expiry, invalid lifetime filters, and reality_delta event payloads.
- Supporting checks passed so far:
  - `cargo fmt`
  - `cargo test -p synapse-reflex --test scheduler_behavior on_event_ -- --nocapture` (6 passed)
  - `cargo test -p synapse-reflex --test scheduler_behavior scheduler_rejects_invalid_lifetime_filter -- --nocapture`
  - `cargo test -p synapse-mcp reality_tools_persist_delta_and_publish_event -- --nocapture`
  - `cargo test -p synapse-core error_codes -- --nocapture`
- Next: run broader local supporting checks (`cargo check` / full scheduler behavior / schema sanitize / release build / diff check), then launch an isolated repo-built daemon for manual #611 MCP FSV with official Inspector strict `tools/list`, real `reflex_register` and perception/action tool calls, and separate UI/storage/log/process SoT readbacks.

## 2026-06-01T01:12:03-05:00
- #611 broader supporting checks passed after the patch:
  - `cargo check -p synapse-reflex`
  - `cargo test -p synapse-reflex --test scheduler_behavior -- --nocapture` (24 passed)
  - `cargo check -p synapse-mcp`
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture` (3 passed)
  - `cargo fmt --check`
  - `git diff --check` exited 0 with only LF/CRLF warnings
- `cargo build --release -p synapse-mcp` initially failed twice with `rustc-LLVM ERROR: out of memory`.
  - Host pressure SoT showed `vmmemWSL` at about 18.6 GB working set / 103.5 GB paged memory and memory compression about 32 GB.
  - Per D4, local pressure was treated as setup work: `wsl.exe --shutdown` reduced `vmmemWSL` to about 2.1 GB.
  - The canonical release build was retried and finished after the tool timeout; no `cargo`/`rustc`/`link` processes remained afterward.
  - Fresh release binary readback: `target\release\synapse-mcp.exe`, length `46322176`, SHA256 `D7F6DD4A11A3A0C7353DDFA1B40BF21F73537B99735645AD5C1F15AC6118C61B`, timestamp `2026-06-01T06:10:44Z`.
- Next: run manual #611 FSV with a repo-built isolated daemon, official MCP Inspector strict `tools/list`, and real MCP `tools/call` triggers. Required evidence remains HUD threshold, audio transient, entity appear, debounce coalescing, filter-never-match, 8-deep filter, UntilEvent expiry, empty/boundary/structurally invalid params, separate UI/storage/log/process readbacks, and cleanup.

## 2026-06-01T00:12:25-05:00
- Active issue remains #610 `scenario(stress): aim_track reflex - moving target + track-loss`.
- Post-compaction wake-up was completed again:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #610, #594, #351, the live open issue queue, and git status/log.
  - Live queue remains #594 plus #595-#604 and #610-#634.
  - Configured wired `mcp__synapse` client works through the actual tool surface: `health`, `storage_inspect`, `reflex_list`, `reflex_history`, and `observe` returned without schema-load failure.
- #610 acceptance FSV run is captured in `.runs\610\aim-track-accept-20260531T2351`:
  - Repo-built daemon PID `74696`, bind `127.0.0.1:7831`, binary `target\release\synapse-mcp.exe`, SHA256 `0E1DFB375963D808D4DE5A22A926CA4762DD57DC0AF6CB3A3C585E8FDFE349C7`.
  - Process/socket/auth/health preconditions passed; official MCP Inspector strict `tools/list` loaded 80 tools including `observe`, `reflex_register`, `reflex_list`, `reflex_history`, `storage_inspect`, `reflex_cancel`, and `release_all`.
  - Happy moving-target run: real Inspector `reflex_register` registered Notepad document element `0x261c94:0000002a00261c94`; physical Win32 move changed the document center to `(1665,1107)`; DPI-aware `GetCursorPos` read `(1665,1107)`; `reflex_history` recorded `aim_track_correction` rows against the rebased target.
  - Track-loss run: after hiding Notepad and moving foreground to VS Code, `observe` no longer contained the target; `reflex_list` showed the reflex `expired` with `REFLEX_TRACK_LOST`; `reflex_history` recorded `reflex_track_lost` with `lost_for_ms=641`.
  - Edge matrix completed with real Inspector tool calls and separate SoT readbacks:
    - X-only: cursor `(1000,1000)` to `(1400,1000)`, correction rows had `y=0`.
    - Y-only: cursor `(1000,1000)` to `(1000,1300)`, correction rows had `x=0`.
    - Deadzone larger than error: cursor stayed `(1000,1000)`, reflex `fire_count=0`, and per-reflex history had only `reflex_registered`.
    - Max-speed clamp: raw delta stayed large, but correction details had `smoothed_delta.x=1` and dispatched `mouse_move_relative dx=1, dy=0`; cursor moved slowly from `(1000,1000)` to `(1135,1000)` instead of jumping to `(1800,1000)`.
    - Boundary: `priority=1000` accepted, one-pixel target moved cursor `(1200,1200)` to `(1201,1200)` with one `dx=1` correction.
    - Empty params: `reflex_register kind=aim_track` exited 1 with `aim_track reflex requires target`; total/active reflex counts stayed `6/0`; cursor unchanged.
    - Structurally invalid target: `target={"kind":"element"}` exited 1 with `missing field element_id`; total/active reflex counts stayed `6/0`; cursor unchanged.
    - Target teleport: Notepad document jumped from center `(1110,822)` to `(2100,1302)`; history recorded first large raw delta `(990,480)` and cursor converged to `(2100,1302)`.
  - Cleanup: `release_all` reported zero keys/buttons/pads, `reflex_list` read `total=7 active=0 cancelled=6 expired=1`, storage read `CF_REFLEX_AUDIT=4956`, and PID `74696`/port `7831` are stopped/closed.
- Next: run final supporting checks, inspect the diff, update state, commit/push `[skip ci]`, post #610 RESOLVED evidence, close #610, refresh the queue, then continue to the next open issue.

## 2026-06-01T00:28:35-05:00
- Final #610 supporting checks completed after manual evidence:
  - `cargo fmt --check`
  - `cargo check -p synapse-reflex`
  - `cargo check -p synapse-mcp`
  - `cargo test -p synapse-reflex --test scheduler_behavior -- --nocapture` (21 passed)
  - `cargo test -p synapse-mcp aim_track_target_source_reads_shallow_observe_child_elements -- --nocapture` passed
  - First broad `cargo test -p synapse-mcp rebase_nodes_to_foreground -- --nocapture` hit host paging-file/resource error 1455 while compiling test artifacts, not a test assertion. Cleaned the cached `windows` crate artifact with `cargo clean -p windows` and reran the focused bin test with one build job/incremental off.
  - `cargo test -p synapse-mcp --bin synapse-mcp rebase_nodes_to_foreground -- --nocapture` passed (2 passed)
  - `cargo test -p synapse-mcp --bin synapse-mcp schema_sanitize -- --nocapture` passed (3 passed)
  - `cargo build --release -p synapse-mcp`
  - `git diff --check` exited 0 with only LF/CRLF warnings.
- Final release binary readback: `C:\code\Synapse\target\release\synapse-mcp.exe`, length `46315008`, SHA256 `478BDD601E6CE5CAD19465FE8D43E01BB1837135340B350A4C3E93FC32290F6A`, timestamp `2026-06-01T05:27:02Z`.
- Diff review completed for source/state files; changes are scoped to dynamic aim-track target sourcing, correction/track-loss audit persistence, M1 shallow target sourcing, and stale UIA bbox rebasing.
- Next: commit/push `[skip ci]`, post #610 RESOLVED evidence, close #610, then refresh the queue.

## 2026-06-01T00:30:41-05:00
- #610 `scenario(stress): aim_track reflex - moving target + track-loss` is closed.
  - Commit: `72581cb fix(reflex): resolve dynamic aim tracking (#610) [skip ci]`.
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/610#issuecomment-4589812724
  - Closure readback: issue state `CLOSED`, closed at `2026-06-01T05:30:10Z`.
  - Post-close git readback: `main...origin/main` and clean.
- Active issue is now #611 `scenario(stress): on_event reflexes - HUD/audio/entity triggers + debounce`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/611#issuecomment-4589814953
  - Issue requires proving on_event reflexes fire actions from real perception events with correct debounce.
  - Required paths: HUD threshold event -> action, audio transient -> action, entity-appear -> action, debounce coalescing, filter never matches, 8-deep filter, UntilEvent lifetime expiry, plus empty/boundary/structurally invalid params.
  - Required SoTs: target app/UI state, `reflex_history` / `CF_REFLEX_AUDIT`, `CF_EVENTS` / `CF_OBSERVATIONS`, storage counts, daemon logs, process/socket state, and cleanup state.
  - Next: inspect on_event filter/debounce/lifetime implementation and MCP event/perception surfaces before launching the isolated #611 daemon.
- Current live open queue after closing #610: #594 plus #595-#604 and #611-#634.

## 2026-05-31T23:38:30-05:00
- Active issue remains #610 `scenario(stress): aim_track reflex - moving target + track-loss`.
- Post-compaction wake-up was completed:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #610, #594, #351, decision/context issue list, live open queue, and git status/log/branch.
  - Live queue is #594 plus #595-#604 and #610-#634.
  - Configured wired `mcp__synapse` client works: `health`, `storage_inspect`, `reflex_list`, `reflex_history`, and `observe` returned through the actual tool surface.
- Patched daemon run `.runs\610\aim-track-depth2-20260531T2315` proved:
  - Repo-built daemon PID `51264`, bind `127.0.0.1:7829`, strict Inspector `tools/list` loaded 80 tools including `observe`, `reflex_register`, `reflex_list`, `reflex_history`, `storage_inspect`, and `release_all`.
  - Baseline isolated storage was clean and Notepad child target `0x261c94:0000002a00261c94` was visible at depth 2.
  - Real Inspector `reflex_register` on the child element produced `aim_track_correction` rows and `fire_count>0`, confirming the depth-2 target-source patch fixed the prior immediate track-loss failure.
  - A serialized moving-window read then exposed a new real #610 defect: Win32 `foreground.window_bounds` moved while UIA root/child element bboxes stayed at the old coordinates, so the reflex chased stale child-element center coordinates after target movement.
- Patch after this defect:
  - `crates/synapse-mcp/src/m1/sources.rs`: Windows `platform_input` now rebases UIA element bboxes to current foreground-window position when the root HWND matches and root dimensions match the foreground bounds. This keeps `observe` and `aim_track` target sourcing aligned with the physical window position when UIA returns stale moved-window coordinates.
  - Added focused regression coverage for rebasing stale UIA rects and leaving dimension-mismatched roots unchanged.
- Supporting checks after the bbox patch:
  - `cargo fmt`
  - `cargo check -p synapse-mcp`
  - `cargo test -p synapse-mcp rebase_nodes_to_foreground -- --nocapture`
  - `cargo check -p synapse-reflex`
  - Release build completed after the first command timeout; final binary `C:\code\Synapse\target\release\synapse-mcp.exe`, length `46315008`, SHA256 `0E1DFB375963D808D4DE5A22A926CA4762DD57DC0AF6CB3A3C585E8FDFE349C7`, timestamp `2026-06-01T04:37:08Z`.
- Cleanup before rebuild: cancelled active test reflexes and stopped isolated daemon PID `51264`; port `127.0.0.1:7829` is closed.
- Next: launch a fresh isolated repo-built daemon on a new port, prove process/socket/auth/health/strict Inspector `tools/list`, then rerun #610 manual FSV happy path plus target loss and required edges.

## 2026-05-31T23:05:19-05:00
- Active issue remains #610 `scenario(stress): aim_track reflex - moving target + track-loss`.
- Post-compaction wake-up completed again:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #610, #594, #351, decision/context issue list, live open queue, and git status/log/branch.
  - Live queue is #594 plus #595-#604 and #610-#634.
  - Configured wired `mcp__synapse` client works: `health`, `storage_inspect`, `reflex_list`, `reflex_history`, and `observe` returned through the actual tool surface.
- #610 failed FSV attempt readback from `.runs\610\aim-track-final-20260531T2255`:
  - Daemon PID `60972`, bind `127.0.0.1:7828`, repo-built release binary, strict Inspector `tools/list` and real `tools/call reflex_register` were already proven in that run.
  - Registered target was Notepad text editor element `0x261c94:0000002a00261c94`.
  - `reflex_history` / `CF_REFLEX_AUDIT` showed `REFLEX_TRACK_LOST` after `lost_for_ms=508` with target source `m1_current_input` reporting only element `0x325d2:0000002a000325d2`; cursor stayed at `(20,20)`, `fire_count=0`, and there were zero `aim_track_correction` rows.
  - Root cause: MCP target source sampled M1 at depth `0`, while `observe` default/depth-2 output can select child elements. The live scheduler could not resolve the child element selected by the operator/agent from the prior observation.
- Patch after failure:
  - `crates/synapse-mcp/src/server/context.rs`: `AIM_TRACK_TARGET_SOURCE_DEPTH` is now `2`, matching `observe`'s shallow default; added regression `aim_track_target_source_reads_shallow_observe_child_elements`.
  - `crates/synapse-mcp/src/m1.rs`: synthetic M1 input now respects requested depth, so depth-sensitive target-source bugs are not hidden by synthetic fixtures.
- Supporting checks after this depth patch:
  - `cargo fmt`
  - `cargo test -p synapse-mcp aim_track_target_source_reads_shallow_observe_child_elements -- --nocapture`
  - `cargo check -p synapse-mcp`
  - `cargo check -p synapse-reflex`
- Next: stop stale failed-FSV daemon PID `60972`, rebuild release `synapse-mcp`, launch a fresh isolated daemon on a new port, re-prove process/socket/auth/health/strict Inspector `tools/list`, then manually rerun #610 happy path and required edges with separate cursor/observation/storage/history/log SoT readbacks.

## 2026-05-31T22:50:00-05:00
- Active issue remains #610 `scenario(stress): aim_track reflex - moving target + track-loss`.
- Root-cause patch is in the worktree:
  - `aim_track` now has a dynamic `AimTrackTargetSource` snapshot abstraction.
  - MCP `SynapseService::reflex_runtime()` installs an M1-backed target source so runtime `aim_track` can resolve current entities/elements from real perception input instead of always seeing empty slices.
  - Stateful conflict planning now resolves dynamic `track_id` / `element_id` targets before arbitration.
  - Each dispatched aim correction writes an `aim_track_correction` row to `CF_REFLEX_AUDIT`.
  - Track loss now expires/deactivates the reflex and writes a `reflex_track_lost` audit row instead of repeatedly blocking ticks.
  - Supporting regression `aim_track_uses_dynamic_target_source_and_audits_corrections_and_loss` proves dynamic target snapshots drive corrections, audit rows persist, and target loss expires.
- Supporting checks passed so far:
  - `cargo fmt`
  - `cargo check -p synapse-reflex`
  - `cargo check -p synapse-mcp`
  - `cargo test -p synapse-reflex --test scheduler_behavior aim_track_uses_dynamic_target_source_and_audits_corrections_and_loss -- --nocapture`
  - `cargo test -p synapse-reflex --test scheduler_behavior -- --nocapture` (21 passed)
  - `cargo test -p synapse-mcp schema_sanitize -- --nocapture`
  - `cargo build --release -p synapse-mcp`
- Repo-built FSV binary readback: `C:\code\Synapse\target\release\synapse-mcp.exe`, length `46276608`, SHA256 `D96388B08C3E3EBC605C015BEC035912B7B2938FCA363CD7B85790CBADD44EA3`, timestamp `2026-05-31 22:49:47`.
- Next: launch isolated repo-built daemon for manual #610 FSV, prove process/socket/auth/health/strict Inspector `tools/list`, then trigger real MCP `reflex_register` runs against a scripted moving on-screen marker with separate cursor/observation/storage/log SoT readbacks.

## 2026-05-31T22:07:00-05:00
- #609 `scenario(stress): 1ms reflex tick jitter under system load` is closed.
  - Commit: `b7ecd73 fix(reflex): persist tick-late audit rows (#609) [skip ci]`
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/609#issuecomment-4589227231
  - Closure readback: `gh issue close 609` succeeded; refreshed issue readback shows state `CLOSED`, closed at `2026-06-01T03:06:23Z`.
  - Post-close git readback before state update: `git status --short --branch` was clean and `main...origin/main`.
- Active issue is now #610 `scenario(stress): aim_track reflex - moving target + track-loss`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/610#issuecomment-4589229027
  - Issue requires proving `aim_track` follows a moving target and handles target loss, using real MCP `tools/call` triggers and separate physical SoT readbacks.
  - Required SoTs: cursor position (`GetCursorPos`), target/observation state, `reflex_history` / `CF_REFLEX_AUDIT`, `storage_inspect`, daemon logs, process/socket state, and cleanup state.
  - Required edges: X-only/Y-only axis, deadzone larger than error, target teleport, max-speed clamp, plus empty/boundary/structurally invalid params.
  - Next: inspect existing `aim_track` target resolution, stateful controller, audit/history, and track-loss code paths before launching an isolated repo-built daemon for manual FSV.
- Current live open queue after closing #609: #594 parent plus #595-#604 and #610-#634.

## 2026-05-31T21:53:59-05:00
- Active issue remains #609 `scenario(stress): 1ms reflex tick jitter under system load`.
- #609 implementation is in the worktree:
  - `REFLEX_TICK_LATE` is now persisted to `CF_REFLEX_AUDIT` with `reflex_id="__scheduler__"`, `error_code=REFLEX_TICK_LATE`, tick index, elapsed/jitter/target/late-after/fallback interval, reason, and `degraded`.
  - Health now exposes retained `p99_tick_jitter_us`, `late_tick_count`, and `degraded_tick_count`.
  - Supporting scheduler regression asserts both the event and persisted scheduler audit row.
- Manual #609 FSV evidence was captured with repo-built release binary `C:\code\Synapse\target\release\synapse-mcp.exe`, SHA256 `B8F8593228BF9730BB523467384CA1C55870D96BF4E2DA40374F26D8FCD87DA8`, official MCP Inspector strict `tools/list`, real `tools/call` triggers, and separate SoT readbacks.
  - Baseline daemon PID `72424`, bind `127.0.0.1:7826`, isolated DB `.runs\609\baseline-20260531T2134\db`: unauth `/health` 401, auth health 200, strict tools/list 80 tools. Idle baseline registered quiet on_event reflex `019e8109-d40e-7c20-8eee-c03fb6348cc7`; before `CF_REFLEX_AUDIT=0`; after idle `reflex_history` showed `REFLEX_TICK_LATE` rows persisted with `degraded=false`, `CF_REFLEX_AUDIT=6`, sample ring 4096/4096.
  - Load trigger used real `act_run_shell` to run 16-core PowerShell CPU stress for 20s. The command sampled Windows CPU at 75/66/67/78/71/64/60/11 percent, max 78. Separate after-read: health ok, `late_tick_count=4`, `degraded_tick_count=0`, `reflex_history` count 104 / tick-late 103, latest rows had elapsed_us up to 12678 and jitter_us 11678 with fallback_interval_us 2000, and `CF_REFLEX_AUDIT=104`, `CF_ACTION_LOG=4`.
  - Subscriber competition edge created 16 real `subscribe` tool subscribers for `reflex_tick_late`; health read `sse_subscribers=16`. A second MCP shell load peaked at 75 percent CPU. After-read: `sse_subscribers=16`, p99 jitter 38us, `late_tick_count=33`, `reflex_history` count 486 / tick-late 485, `CF_REFLEX_AUDIT=486`, `CF_ACTION_LOG=6`. All 16 subscriptions were later cancelled through real `subscribe_cancel`, and health read `sse_subscribers=0`.
  - Invalid/boundary edges: `subscribe buffer_size=0` failed closed and kept subscribers at 16; `subscribe_cancel subscription_id=""` reached the tool and failed closed with subscribers still 16; `reflex_history limit=0` returned zero events; `reflex_history limit=1001` failed closed; structurally invalid `reflex_register` target failed deserialization and `reflex_list` remained total 1 / active 1 before cleanup.
  - Forced degraded daemon PID `16712`, bind `127.0.0.1:7827`, isolated DB `.runs\609\degraded-20260531T2254\db`: unauth `/health` 401, auth health ok, strict tools/list 80 tools. Started with `SYNAPSE_REFLEX_FORCE_DEGRADED=1` / `--reflex-force-degraded`. Registered quiet reflex `019e8118-9a9e-77e1-8664-8bf3ff5fac46`; after 6s health read `status=degraded_latency`, sample ring 4096/4096, `degraded_tick_count=4096`, `late_tick_count=546`, p99 jitter 14249us. `reflex_history limit=1000` returned 1000 tick-late rows, latest rows had `degraded=true`, fallback_interval_us 2000, elapsed_us around 14-15ms, and `CF_REFLEX_AUDIT=1992`; daemon log readback had `degraded=true` tick lines and `forced_degraded_config`.
  - Cleanup used real `reflex_cancel` for both registered reflexes; final list readbacks showed both daemons total 1 / active 0 / cancelled 1 and subscribers 0. Final storage readbacks: baseline `CF_REFLEX_AUDIT=3190`, `CF_ACTION_LOG=6`, pressure Normal; degraded `CF_REFLEX_AUDIT=5302`, pressure Normal. PIDs `72424` and `16712` stopped and ports `7826`/`7827` are closed.
- Known setup noise excluded from acceptance: early daemon attempts failed auth due inherited APPDATA token, a background Inspector load wrapper split the auth header, and a first degraded launch rejected `SYNAPSE_ALLOW_SHELL=.*` as `SHELL_PATTERN_TOO_BROAD`. Each was corrected before accepted FSV evidence.
- Final supporting checks passed after FSV: `cargo fmt --check`; `cargo check -p synapse-core`; `cargo check -p synapse-reflex`; focused tick-late scheduler test; full `cargo test -p synapse-reflex --test scheduler_behavior -- --nocapture` (20 passed); `cargo check -p synapse-mcp`; `cargo test -p synapse-mcp schema_sanitize -- --nocapture`; `cargo build --release -p synapse-mcp`; `git diff --check` (line-ending warnings only). The final rebuilt release binary SHA256 is `A245308F45D5A0F1F6354BDF2A99ACD8DF7DA9A44F6FC5CA9115E8240D3C9592`, length `46281216`, timestamp `2026-05-31 22:04:25`.
- Diff review completed for code and state files. Next: commit `[skip ci]`, post #609 RESOLVED evidence, close #609, update state closure, then refresh the issue queue.

## 2026-05-31T21:22:49-05:00
- Active issue is #609 `scenario(stress): 1ms reflex tick jitter under system load`.
- Code inspection found a real #609 evidence-surface gap:
  - The scheduler already emitted `REFLEX_TICK_LATE` on the in-process event bus and logged every tick sample.
  - It did not persist tick-late rows to `CF_REFLEX_AUDIT`, so `reflex_history` could not be the physical SoT requested by the issue body.
  - Health exposed last jitter/sample count/limit from #608, but not retained p99 jitter, late count, or degraded fallback count.
- Current #609 patch in worktree:
  - `scheduler_tick.rs` writes one `CF_REFLEX_AUDIT` row for each late tick with `reflex_id="__scheduler__"`, `error_code=REFLEX_TICK_LATE`, and details for tick index, elapsed/jitter/target/late-after/fallback interval, reason, and degraded flag.
  - `SubsystemHealth` / MCP health now expose `p99_tick_jitter_us`, `late_tick_count`, and `degraded_tick_count` in addition to sample count/limit and last jitter.
  - Existing supporting regression `blocked_dispatch_path_emits_reflex_tick_late` now proves the tick-late event and persisted scheduler audit row shape.
- Supporting checks after this patch:
  - `cargo fmt`
  - `cargo check -p synapse-reflex`
  - `cargo check -p synapse-core`
  - `cargo test -p synapse-reflex --test scheduler_behavior blocked_dispatch_path_emits_reflex_tick_late -- --nocapture`
  - `cargo check -p synapse-mcp`
  - `cargo test -p synapse-mcp schema_sanitize -- --nocapture`
- Next #609 step: run final fmt/checks as needed, build release `synapse-mcp`, launch isolated repo-built daemons for baseline and forced-degraded/load FSV, prove process/socket/auth/health/strict Inspector tools/list, then manually drive idle, load, subscriber, and invalid/boundary edges with real MCP tools plus separate SoT readbacks.

## 2026-05-31T21:15:24-05:00
- #608 `scenario(stress): 32-reflex saturation - priority, exclusive, starvation` is closed.
  - Commit: `5873c37 fix(reflex): arbitrate saturated stateful reflexes (#608) [skip ci]`
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/608#issuecomment-4589052871
  - Closure readback: `gh issue close 608` succeeded; refreshed issue readback shows state `CLOSED`, closed at `2026-06-01T02:14:57Z`.
  - Post-close git readback: `git status --short --branch` is clean and `main...origin/main`.
- Active issue is now #609 `scenario(stress): 1ms reflex tick jitter under system load`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/609#issuecomment-4589054448
  - Issue requires proving 1ms reflex tick behavior or graceful 2ms degraded fallback under host load, with SoT readbacks from `reflex_history`, `CF_REFLEX_AUDIT`, health tick/sample fields, daemon log bytes, process/socket state, and host load counters.
  - Required edges: idle baseline, sudden load spike, sustained CPU/GPU/capture churn, many concurrent subscribers competing, tick late >2ms threshold, plus empty/boundary/structurally invalid params.
  - Next: inspect scheduler tick/degraded-mode paths and existing MCP surfaces, then launch an isolated repo-built daemon for #609 manual FSV.
- Current live open queue after closing #608: #594 parent plus #595-#604 and #609-#634.

## 2026-05-31T21:12:06-05:00
- Active issue remains #608 `scenario(stress): 32-reflex saturation - priority, exclusive, starvation`; implementation and final manual FSV evidence are now captured, with issue comment/closure next.
- Post-compaction wake-up was completed again:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #608, #594, #351, live decision/context issue lists, open issue queue, and git status/log/branch.
  - Live queue is #594 plus #595-#604 and #608-#634; #608 still has only the START comment.
  - Wired configured `mcp__synapse` client works through the actual tool surface: `health`, `storage_inspect`, `reflex_list`, `reflex_history`, and `observe` returned; installed stdio runtime PID remains healthy with DB `C:\Users\hotra\AppData\Local\synapse\db`.
- Final #608 supporting checks passed after the stateful-arbitration patch:
  - `cargo fmt`, `cargo fmt --check`
  - `cargo check -p synapse-reflex`, `cargo check -p synapse-mcp`, `cargo check -p synapse-core`
  - `cargo test -p synapse-reflex --test scheduler_behavior -- --nocapture` (20 passed)
  - Focused duplicate/exclusive/priority/cancel/stateful starvation tests passed sequentially.
  - `cargo test -p synapse-mcp --test m3_reflex_register_tool reflex_register_schema_defaults_and_edges -- --nocapture`
  - `cargo test -p synapse-mcp schema_sanitize -- --nocapture`
  - `cargo build --release -p synapse-mcp` passed; final binary `C:\code\Synapse\target\release\synapse-mcp.exe` SHA256 `A9D2C94C2A05DDEBF01A7A9DAFD2D97BCF09DEF6B1461F1FDAC1EBF593EB24C8`, length `46237184`, timestamp `2026-05-31 20:54:41`.
  - Earlier parallel test attempts hit Windows linker `LNK1104` because multiple scheduler test binaries linked concurrently; affected tests were rerun sequentially and passed.
- Final manual #608 FSV evidence used repo-built isolated HTTP daemons and official MCP Inspector strict `tools/list` / real `tools/call`, with separate SoT reads:
  - Cap/invalid daemon PID `34784`, bind `127.0.0.1:7823`, run `.runs\608\final-fsv-20260531T2058`: unauth health 401; auth health 200; strict Inspector `tools/list` succeeded with 80 tools. Baseline `reflex_list` total/active 0, `CF_REFLEX_AUDIT=0`, `CF_ACTION_LOG=0`.
  - Duplicate active registration failed closed with `REFLEX_PARAMS_INVALID` and left one active row: list total/active 1, `CF_REFLEX_AUDIT=1`, `CF_ACTION_LOG=0`.
  - 32-reflex ceiling accepted 30 `on_event`, one `hold_button`, and one `combo`, including priority boundaries 0 and 1000. After-read: `reflex_list` total/active 32, `CF_REFLEX_AUDIT=32`, `CF_ACTION_LOG=0`, health reflex ok with `sample_count=4096`, `sample_limit=4096`.
  - 33rd registration failed closed with `scheduler reflex cap 32 exceeded by 33`; after-read remained total/active 32, no cap33 row, `CF_REFLEX_AUDIT=32`, `CF_ACTION_LOG=0`.
  - Cleanup `release_all` disabled all 32 and neutralized one pad; after-read total 32, active 0, disabled 32, `CF_REFLEX_AUDIT=64`, `CF_ACTION_LOG=1`, health active 0 and sample ring 4096/4096.
  - Empty params, priority 1001, and structurally invalid aim target all failed before adding rows; after-read remained total 32, active 0, disabled 32, `CF_REFLEX_AUDIT=64`, `CF_ACTION_LOG=1`.
  - Stateful priority/exclusive/starvation daemon PID `59948`, bind `127.0.0.1:7824`, run `.runs\608\final-starvation-20260531T2125`: strict Inspector `tools/list` succeeded. Registered two exclusive `aim_track` reflexes. After wait, winner priority 10 active/fire_count `23619`, loser priority 100 starved/fire_count `0`, loser `last_error_code=REFLEX_STARVED`; history contained one starved row with resource `mouse_cursor`; storage `CF_REFLEX_AUDIT=3`. Cancelling the winner mid-fire let the loser become active and fire (`fire_count=4967`); history then count 4 with active/starved/cancelled rows; storage `CF_REFLEX_AUDIT=4`.
  - Same-tick daemon PID `70124`, bind `127.0.0.1:7825`, run `.runs\608\final-sametick-20260531T2135`: unauth health 401; auth health ok; strict Inspector `tools/list` succeeded. Registered 32 `hold_button` reflexes; after one tick, `reflex_list` total/active/hold_button 32, min/max fire_count 1/1, `CF_REFLEX_AUDIT=32`, health active 32 and sample ring 4096/4096, daemon log readback showed `dispatched_actions=32` on tick_index 0.
  - Same-tick cleanup used real Inspector `release_all`, returned `neutralized_pads=3`, then after-read showed total 32, active 0, disabled 32, hold_button 32, min/max fire_count 1/1, `CF_REFLEX_AUDIT=64`, `CF_ACTION_LOG=1`, health active 0 and sample ring 4096/4096. PID `70124` was stopped and port `7825` is closed.
- Current worktree still contains #608 code/state changes. Next: run final diff/readback checks, commit with `[skip ci]`, push, post #608 RESOLVED evidence, close #608, update state, then refresh the queue and claim the next open issue.

## 2026-05-31T20:38:43-05:00
- Active issue remains #608 `scenario(stress): 32-reflex saturation - priority, exclusive, starvation`.
- Post-compaction wake-up was completed again:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, live #608/#594/#351, decision/context issue lists, open issue queue, and git status/log/branch.
  - Live queue is still #594 plus #595-#604 and #608-#634; #608 has only the START comment.
  - Wired configured `mcp__synapse` client is usable: `health`, `storage_inspect`, `reflex_list`, `reflex_history`, and `observe` all returned through the real client. Installed stdio runtime reports ok with DB `C:\Users\hotra\AppData\Local\synapse\db`, active profile `notepad`, and reflex active_count 0.
- Cleanup after the pre-compaction #608 aim-starvation run:
  - Isolated daemon PID `64832`, bind `127.0.0.1:7822`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, was still alive.
  - `release_all` returned zero held inputs, but `reflex_list` still showed both active `aim_track` reflexes with identical `fire_count=224560`; this confirmed the stateful-controller arbitration gap.
  - Explicit `reflex_cancel` calls cancelled IDs `019e80c9-1987-7b53-bddc-e3363bfc4352` and `019e80c9-440c-7401-b914-43ac088cd555`; after-read showed total 2, active 0, cancelled 2; PID `64832` was stopped and port `7822` is closed.
- Root cause fixed in the #608 worktree:
  - `scheduler_stateful.rs` now creates conflict candidates for active stateful drivers (`aim_track`, `hold_move`, `hold_button`, combo) before dispatch, using their actual reserved resources where possible.
  - `scheduler_tick.rs` now aggregates conflict losers across the tick and records starvation once per tick, preventing a later empty arbitration pass from resetting a stateful loser before it reaches `STARVATION_AFTER`.
  - Added supporting regression `stateful_aim_track_conflicts_by_priority_and_starves_loser`.
- Supporting checks after the stateful patch:
  - `cargo fmt`
  - `cargo check -p synapse-reflex`
  - `cargo test -p synapse-reflex --test scheduler_behavior stateful_aim_track_conflicts_by_priority_and_starves_loser -- --nocapture`
  - `cargo test -p synapse-reflex --test scheduler_behavior lower_priority_number_wins_cursor_conflict_and_starves_loser -- --nocapture`
  - `cargo test -p synapse-reflex --test scheduler_behavior cancelling_winner_allows_starved_loser_to_fire_again -- --nocapture`
  - `cargo test -p synapse-reflex --test scheduler_behavior exclusive_mouse_reflex_blocks_lower_priority_same_device_class -- --nocapture`
  - `cargo test -p synapse-reflex --test scheduler_behavior scheduler_rejects_duplicate_reflex_ids -- --nocapture`
  - `cargo test -p synapse-reflex duplicate_active_reflex_definition_is_rejected -- --nocapture`
  - `cargo check -p synapse-mcp`
  - `cargo check -p synapse-core`
  - `cargo test -p synapse-mcp --test m3_reflex_register_tool reflex_register_schema_defaults_and_edges -- --nocapture`
  - `cargo test -p synapse-mcp schema_sanitize -- --nocapture`
  - Two parallel scheduler test attempts hit Windows linker `LNK1104` because multiple `scheduler_behavior` test binaries linked concurrently; reruns of the affected tests passed sequentially.
- Next #608 action: run `cargo fmt --check`, rebuild release `synapse-mcp`, launch a fresh isolated repo-built daemon on a new port such as `7823`, prove process/socket/auth/health/strict Inspector `tools/list`, then rerun manual #608 FSV on the final binary: 32 cap/33rd fail-closed as needed, stateful priority/exclusive/starvation, cancel recovery, invalid/empty/boundary params, and same-tick/sample-limit readbacks.

## 2026-05-31T19:32:35-05:00
- Post-compaction wake-up completed for active #608:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/ACTIVE_OBJECTIVE.md`, `STATE/CURRENT_STATE.md`, `STATE/RECOVERY_NOTES.md`, and tails of `STATE/DECISION_LOG.md` / `STATE/HEARTBEAT.md`.
  - Re-read live GitHub #608, #594, #351, open issue queue, and decision/context issue lists. #608 is still open with only the START comment; #594 remains the open parent context.
  - Git readback: branch `main`, HEAD `495766d docs(state): record issue 607 closure [skip ci]`, worktree has only #608 patch files in `synapse-core`, `synapse-mcp`, and `synapse-reflex`.
  - Wired configured Synapse MCP client loaded the tool surface and worked: `health`, `profile_list include_inactive=true`, `storage_inspect`, `observe depth=0`, `reflex_list include_expired=true`, and `reflex_history limit=10` all returned. Process SoT shows installed stdio daemon PID `45712` at `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`; stale TCP row remains on `127.0.0.1:7814` owned by absent PID `61024`, so do not reuse that port.
- Current #608 patch in worktree:
  - `synapse-reflex` conflict resolver now preserves exact resource conflicts and adds same-device-class conflicts only when both candidates are `exclusive=true`; combo steps now contribute conflict resources.
  - Duplicate scheduler reflex IDs fail closed in `validate_reflexes`; duplicate active runtime registrations with the same trigger/actions/driver/priority/lifetime/exclusive/debounce fail closed.
  - Reflex health exposes retained scheduler `sample_count` and `sample_limit` for manual tick-ring SoT readback.
  - Supporting regressions added for exclusive mouse starvation, duplicate scheduler IDs, duplicate active runtime registration, and duplicate MCP `reflex_register`.
- Supporting checks passed so far:
  - `cargo fmt`
  - `cargo check -p synapse-reflex`
  - `cargo test -p synapse-reflex --test scheduler_behavior exclusive_mouse_reflex_blocks_lower_priority_same_device_class -- --nocapture`
  - `cargo test -p synapse-reflex --test scheduler_behavior scheduler_rejects_duplicate_reflex_ids -- --nocapture`
  - `cargo test -p synapse-reflex duplicate_active_reflex_definition_is_rejected -- --nocapture`
  - `cargo test -p synapse-mcp --test m3_reflex_register_tool reflex_register_schema_defaults_and_edges -- --nocapture`
  - `cargo check -p synapse-mcp`
  - `cargo check -p synapse-core`
- Next #608 action: run final formatting/schema checks as needed, build repo release `synapse-mcp`, launch an isolated HTTP daemon on a fresh port such as `7816`, prove process/socket/auth/health/strict Inspector `tools/list`, then manually FSV 32-reflex saturation, 33rd fail-closed, duplicate/priority-bound/invalid edges, exclusive starvation, cancel recovery, and same-tick/sample-limit behavior through real MCP `tools/call` plus separate SoT readbacks.

## 2026-05-31T19:17:16-05:00
- #607 `scenario(stress): act_launch fleet - all 30 profiles, foreground incl. console apps` is closed.
  - Commit: `8ce49e4 fix(mcp): harden act_launch foregrounding (#607) [skip ci]`
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/607#issuecomment-4588670440
  - Closure readback: `gh issue close 607` succeeded; refreshed open queue no longer lists #607.
  - Post-close git readback: `git status --short --branch` is clean and `main...origin/main`.
- Active issue is now #608 `scenario(stress): 32-reflex saturation - priority, exclusive, starvation`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/608#issuecomment-4588672100
  - Issue acceptance requires real MCP `tools/call` triggers for reflex saturation/priority/exclusive/starvation behavior and separate SoT readbacks from `reflex_list`, `reflex_history`, `CF_REFLEX_AUDIT`, daemon logs/process state, and physical action state where applicable.
  - Required edges: priority `0` and `1000` bounds, duplicate registration, cancel mid-fire, all 32 firing same tick / sample cap, plus empty/boundary/structurally invalid params.
  - Next: inspect reflex scheduler/runtime/register/list/history/action-dispatch code paths, then launch an isolated repo-built daemon for #608 MCP precondition and manual FSV.
- Current live open queue after closing #607: #594 parent plus #595-#604 and #608-#634.

## 2026-05-31T18:49:37-05:00
- Active issue remains #607 `scenario(stress): act_launch fleet - all 30 profiles, foreground incl. console apps`.
- Post-compaction wake-up was completed again:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #351, live open queue, #607 comments, git status/log/branch.
  - Wired `mcp__synapse` client is usable: `health.ok=true`, installed stdio PID `45712`, profile_count `29`, `profile_list include_inactive=true` loads all bundled profiles, `storage_inspect` works, and `observe depth=0` read foreground `eqgame.exe` with `profile_id=everquest.live`.
  - Live open queue remains #594 plus #595-#604 and #607-#634; #607 is still open with only the START comment.
- Current #607 implementation patch in worktree:
  - `act_launch` now validates timeout max, records start/result audit details, and writes successful spawn rows to `CF_PROCESS_HISTORY` without environment values.
  - Console targets use a Win32 `CreateProcessW` path with `CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP | CREATE_UNICODE_ENVIRONMENT` and `STARTUPINFOW` show-window state.
  - Launch window selection now prefers PID+title, then title, and only accepts existing excluded windows when the process identity is compatible with the requested target or a known shell activation alias.
  - Action audit/action preflight/reflex scope checks now use fast foreground identity instead of depth-1 UIA snapshots when only foreground identity is needed.
  - UIA snapshot child/raw-supplement failures warn/truncate instead of aborting; empty RuntimeId uses a deterministic process-local fallback element id.
  - `cmd.toml` and `powershell.toml` include Windows Terminal/OpenConsole/conhost title-specific matches.
  - New profile metadata added for host-disposition gaps: WordPad removed on modern Windows, IE desktop redirecting to Edge. Minecraft already had launcher/sign-in/operator-boundary metadata.
- #607 manual FSV evidence captured through repo-built isolated daemon PID `61024`, bind `127.0.0.1:7814`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`, isolated DB `.runs\607\launch-fleet-final8-20260531T182322\db`:
  - MCP precondition: unauth `/health` returned 401; auth `/health` ok with `allow_launch_patterns=any`; strict Inspector `tools/list` exited 0 and showed `act_launch`, `observe`, `profile_list`, `storage_inspect`; storage baseline `CF_ACTION_LOG=0`, `CF_PROCESS_HISTORY=0`; `profile_list` showed 29 profiles.
  - Accepted launch/profile readbacks for 26 profiles: `acrobat`, `calculator`, `chrome`, `cmd`, `everquest.live`, `excel`, `explorer`, `firefox`, `luanti.minetest`, `mstsc`, `notepad`, `onenote`, `outlook`, `paint`, `photos`, `powerpoint`, `powershell`, `settings`, `slack`, `snippingtool`, `taskmanager`, `teams`, `terminal`, `vscode`, `word`, `zoom`.
  - Final accepted storage after EverQuest: `CF_ACTION_LOG=60`, `CF_PROCESS_HISTORY=30`; after all edge cases: `CF_ACTION_LOG=74`, `CF_PROCESS_HISTORY=35` on the permissive daemon.
  - Console coverage passed with visible foreground/profile readbacks for cmd (`profile_id=cmd`), PowerShell (`profile_id=powershell`), and Windows Terminal (`profile_id=terminal`).
  - Edge cases passed: already-running/single-instance Chrome and VS Code; wait-title no-match; empty target; structurally invalid wait regex; max timeout boundary `600000`; rapid Notepad relaunch; restrictive policy deny on separate PID `59732` / `127.0.0.1:7815`.
  - Documented host gaps after local setup/readbacks: `iexplore` spawned `iexplore.exe` but foreground became Microsoft Edge (`profile_id=chrome`), WordPad/write binaries and optional capability were absent, and Minecraft Java remains launcher/sign-in/license bounded until operator-owned Java runtime/world-log SoT exists. Luanti analogue passed.
- Cleanup/readback after final FSV:
  - `eqgame.exe` PID `70060` and known FSV-owned heavy app PIDs were stopped to release memory.
  - Agent-created untracked `Logs/` and `eqclient.ini` from the initial EverQuest wrong-working-dir launch were removed after verifying their absolute paths were inside `C:\code\Synapse`.
  - Repo-built daemon PID `59732` is gone and port `7815` is closed. PID `61024` is absent from `Get-Process`, CIM, `tasklist`, and `taskkill`, but Windows still reports a stale TCP LISTEN row for `127.0.0.1:7814` owned by PID `61024`; do not reuse that port.
- Final #607 supporting checks after metadata/clippy fixes:
  - `cargo fmt --check`
  - `cargo check -p synapse-a11y`
  - `cargo check -p synapse-mcp`
  - `cargo check -p synapse-profiles`
  - `cargo test -p synapse-profiles --test parse_bundled -- --nocapture`
  - `cargo clippy -p synapse-mcp --all-targets -- -D warnings`
  - `cargo test -p synapse-mcp launch_ -- --nocapture` (16 matching tests across unit/integration targets passed)
  - `cargo test -p synapse-mcp process_history_has_retention_class -- --nocapture`
  - `cargo build --release -p synapse-mcp` finished successfully; release binary timestamp `2026-05-31 19:13:59`, length `46263808`.
  - `git diff --check` exited 0 with line-ending warnings only.
- Diff review completed. Next: commit/push with `[skip ci]`, post #607 RESOLVED evidence, close #607, refresh queue.

## 2026-05-31T17:56:27-05:00
- Active issue remains #607.
- Post-compaction wake-up was completed again:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #351, live open queue, #607 comments, git status/log/branch.
  - Wired `mcp__synapse` tools verified through the actual configured MCP client: `health.ok=true`, `profile_list include_inactive=true` reports 29 bundled profiles, `storage_inspect` works, and `observe depth=0` resolved the current Chrome foreground.
  - Live open queue remains #594 plus #595-#604 and #607-#634; #607 is the active issue.
- Final7 isolated daemon readback before the latest patch:
  - PID `39520`, bind `127.0.0.1:7813`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`.
  - Strict Inspector `observe` and `storage_inspect` still work; storage counts after partial profile matrix were `CF_ACTION_LOG=35`, `CF_PROCESS_HISTORY=17`.
  - Slack did not leave a `slack.exe` process. The failed Slack `act_launch` happened before process spawn/history recording because action supported-use preflight read a depth-1 UIA snapshot and hit `cached RuntimeId had unexpected type EMPTY` while foreground was Acrobat.
- Root cause / fix added:
  - `ensure_supported_use_allows_action` now reads fast foreground state (`current_audit_foreground`) instead of requiring a depth-1 UIA tree for generic action launch/scope preflight.
  - Reflex action scope checks now use the same fast foreground semantics while preserving synthetic/forced-error behavior.
  - UIA snapshot walking now logs and truncates when a child/raw-supplement node has a bad RuntimeId instead of aborting the whole snapshot; `VT_EMPTY` RuntimeId gets a process-local fallback element id with warning. Microsoft docs confirm RuntimeId's default value is `VT_EMPTY` and its opaque identifier can change/reuse over time.
- Supporting checks after this patch:
  - `cargo fmt --check` passed.
  - `cargo check -p synapse-a11y` passed.
  - `cargo check -p synapse-mcp` passed.
- Next: stop isolated daemon PID `39520` to unlock the release binary, rebuild `cargo build --release -p synapse-mcp`, launch a fresh isolated final daemon, rerun strict Inspector precondition, retry Slack, then continue the remaining profile matrix and #607 edge cases.

## 2026-05-31T17:27:03-05:00
- Active issue remains #607.
- Additional final-binary FSV defect found:
  - Chrome was already running and reused an existing top-level window. Before the latest patch, `act_launch` foregrounded Chrome but returned `reason=no_match_within_timeout` because the existing HWND was excluded from title matching.
  - A first fallback patch accepted excluded windows by title alone, but an Explorer retry with a broad `Synapse` regex falsely matched an unrelated Windows Terminal window. That was treated as a real safety bug.
- Current fix:
  - `select_launch_window` still prefers a new matching PID/window, then a new matching title, but its existing-window fallback now requires the excluded window process to match the launch target or a known Windows console host alias (`wt.exe`/cmd/powershell/pwsh hosted by WindowsTerminal/OpenConsole/conhost).
  - Added supporting tests for preferring new windows, accepting same-process single-instance windows, and rejecting unrelated existing windows with broad title regexes.
- Checks after the latest fix:
  - `cargo fmt --check` passed.
  - `cargo check -p synapse-mcp` passed.
  - `cargo test -p synapse-mcp launch_window_selection -- --nocapture` passed (3 tests).
  - `cargo build --release -p synapse-mcp` passed.
- Latest repo-built daemon evidence before this safety refinement:
  - PID `51896` on `127.0.0.1:7811`, strict Inspector `tools/list` count 80, storage baseline 0/0.
  - Final-binary console checks passed for cmd/powershell/Windows Terminal and Chrome existing-window check passed, but the subsequent Explorer false-match means these need a fresh final daemon again after the process-compatible fallback patch.
- Next: launch fresh final daemon (suggest port `7812`), redo MCP precondition, rerun cmd/powershell/terminal, Chrome existing-window, and the profile matrix with stricter wait regexes.

## 2026-05-31T16:29:23-05:00
- Active issue remains #607 `scenario(stress): act_launch fleet - all 30 profiles, foreground incl. console apps`.
- Post-compaction wake-up was completed again:
  - Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, `STATE/*`, #351, live open queue, and #607 comments.
  - Live queue still shows #594 plus #595-#604 and #607-#634 open; #607 has only the START comment.
  - Wired MCP client readback: `health.ok=true`, `profile_count=29`, `storage_inspect` works and still shows installed-runtime `CF_PROCESS_HISTORY=0`, and `observe depth=0` reads the current foreground `vscode` window.
- Current #607 compile-break fix:
  - Replaced unstable Rust `CommandExt::show_window` usage for console launches with a Windows-only `CreateProcessW` path using `CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP | CREATE_UNICODE_ENVIRONMENT` and `STARTUPINFOW { STARTF_USESHOWWINDOW, wShowWindow=SW_SHOWNORMAL }`.
  - Console launches now build the same narrow environment policy as the std `Command` path without recording env values.
  - Official Microsoft docs consulted for `CreateProcessW`, `STARTUPINFOW`, `SetForegroundWindow`, and WordPad removal in Windows 11 24H2.
- Supporting checks after this fix:
  - `cargo fmt --check` passed.
  - `cargo check -p synapse-mcp` passed.
  - `cargo check -p synapse-a11y` passed.
  - `cargo test -p synapse-mcp launch_console_targets_request_real_console_windows -- --nocapture` passed.
  - `cargo test -p synapse-mcp launch_process_history_row_records_spawn_without_env_values -- --nocapture` passed after rerunning sequentially; the first parallel attempt failed with linker `LNK1104` due concurrent cargo test/link activity, not a code failure.
- Next: build `cargo build --release -p synapse-mcp`, stop stale isolated #607 daemon if present, launch a fresh repo-built isolated daemon on a new port, prove process/socket/auth/health/strict Inspector `tools/list`, then rerun cmd/powershell/Windows Terminal real `act_launch` MCP FSV.

## 2026-05-31T14:47:21-05:00
- #606 `scenario(stress): act_run_shell orchestration - allowlist modes, timeout, 1MB cap, idempotency` is closed.
  - Commit: `6975d14 fix(mcp): audit and dedupe shell orchestration (#606) [skip ci]`
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/606#issuecomment-4587883204
  - Closure readback: `gh issue close 606` succeeded; refreshed open queue no longer lists #606.
  - Post-#606 process cleanup readback: only wired chat MCP PID `45712` remains; isolated ports `7799`, `7800`, `7801`, and `7802` were closed before closure.
- Active issue is now #607 `scenario(stress): act_launch fleet - all 30 profiles, foreground incl. console apps`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/607#issuecomment-4587884557
  - Issue acceptance requires real MCP `tools/call act_launch` triggers, separate foreground/process/profile/storage SoT readbacks, all 30 bundled-profile apps where locally available/acquirable, explicit cmd/powershell/Windows Terminal console foregrounding, and edges for already-running app, wait-title no-match, restrictive-policy deny, rapid relaunch, and invalid/empty params.
  - Next: inspect profile launch definitions and existing `act_launch` foreground/window matching/audit behavior before launching a repo-built isolated daemon for #607 manual FSV.
- Current live open queue after closing #606: #594 parent plus #595-#604 and #607-#634.

## 2026-05-31T15:21:27-05:00
- Resumed #607 after compaction and re-read required wake context:
  - `docs/AICodingAgentSuperPrompt.md`
  - `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`
  - `AGENTS.md`
  - #351 decision/context comments
  - live open queue and #607 comments
  - `git status`, `git log -10`, and current branch
- Git state readback:
  - branch `main`
  - HEAD `a15895f docs(state): record issue 606 closure [skip ci]`
  - worktree modified only in current #607 patch files: `m3/audit_retention.rs`, `m4.rs`, `server.rs`, `server/m4_tools.rs`, and `synapse-reflex/src/storage.rs`.
- Wired chat Synapse MCP client readback:
  - `health.ok=true`, stdio runtime PID reported in health, storage DB `C:\Users\hotra\AppData\Local\synapse\db`, `allow_launch_patterns=any`.
  - `profile_list include_inactive=true` returned 29 bundled profile TOML files from `crates/synapse-profiles/profiles`; #607 says 30, so on-disk profile files plus `profile_list` are the fleet-count SoT.
  - `storage_inspect` shows `CF_PROCESS_HISTORY=0` in the installed runtime, which is the #607 storage gap the patch addresses.
- Current #607 patch in worktree:
  - `act_launch` now audits request details, validates `timeout_ms` against the schema max, records successful spawns in `CF_PROCESS_HISTORY`, and registers `CF_PROCESS_HISTORY` in audit retention.
  - `synapse-reflex` exposes `storage_put_process_history_rows`.
  - Focused supporting checks previously passed before compaction: `cargo check -p synapse-mcp`, `cargo fmt --check`, focused launch/process-history tests, and `cargo build --release -p synapse-mcp`.
- Next: launch an isolated repo-built #607 daemon, prove process/socket/auth/health/strict Inspector `tools/list`, then manually FSV `act_launch` with separate foreground/process/profile/storage SoT readbacks.

## 2026-05-31T16:15:13-05:00
- Active issue remains #607 `scenario(stress): act_launch fleet - all 30 profiles, foreground incl. console apps`.
- Additional #607 defects found through real isolated MCP runtime + official MCP Inspector:
  - Direct console launches were not creating actionable visible console windows. Patched `act_launch` so `cmd.exe`, `powershell.exe`, and `pwsh.exe` request a Windows `CREATE_NEW_CONSOLE` instead of detaching all stdio.
  - `CF_PROCESS_HISTORY` now records whether a launch requested a new console (`windows_new_console`) and the request details omit environment values.
  - Windows 11 console hosting can surface a cmd/powershell console as `WindowsTerminal.exe` with a cmd/powershell title. Patched `cmd.toml` and `powershell.toml` to match title-specific `WindowsTerminal.exe` / `wt.exe` hosts.
  - `act_launch` action audit result rows could stall about 60s after console launches because audit foreground metadata used a depth-1 UIA snapshot. Patched action audit to use fast foreground metadata (`synapse_a11y::current_foreground_context`) while preserving synthetic/error behavior.
  - Foregrounding a newly matched console window can fail under Windows foreground-lock rules. Patched `synapse-a11y::focus_window` to send a Windows-only Alt key activation nudge before the existing `AttachThreadInput` / `SetForegroundWindow` retry.
- Supporting checks passed after these fixes:
  - `cargo fmt --check`
  - `cargo check -p synapse-a11y`
  - `cargo check -p synapse-mcp`
  - `cargo check -p synapse-profiles`
  - `cargo test -p synapse-mcp launch_console_targets_request_real_console_windows -- --nocapture`
  - `cargo test -p synapse-mcp launch_process_history_row_records_spawn_without_env_values -- --nocapture`
  - `cargo test -p synapse-profiles --test parse_bundled -- --nocapture`
  - `cargo build --release -p synapse-mcp`
- Runtime readbacks so far:
  - Fresh isolated daemon PID `49456` on `127.0.0.1:7807` proved strict Inspector `tools/list` count 80 and storage baseline `CF_ACTION_LOG=0`, `CF_PROCESS_HISTORY=0`.
  - After audit fast-path patch, `cmd.exe /k echo synapse-607-cmd-final` returned through Inspector in `1788 ms` instead of timing out; `CF_PROCESS_HISTORY=1` recorded `windows_new_console=true`, pid `39160`, hwnd `11865122`, title `C:\WINDOWS\system32\cmd.exe`.
  - That run exposed profile/foreground mismatch: foreground host was `WindowsTerminal.exe` and resolved to `terminal`, prompting the title-specific cmd/powershell profile patch.
  - Fresh isolated daemon PID `37348` on `127.0.0.1:7808` proved strict Inspector `tools/list` count 80 and storage baseline `CF_ACTION_LOG=0`, `CF_PROCESS_HISTORY=0`.
  - `cmd.exe /k title synapse-607-cmd-title && echo synapse-607-cmd-title` matched a visible hwnd `10551496` and wrote `CF_PROCESS_HISTORY`, but `synapse_a11y::focus_window` returned `SetForegroundWindow returned false`; this prompted the foreground activation nudge patch.
- Next: launch a new isolated repo-built daemon from the latest release binary, repeat MCP precondition, then rerun cmd/powershell/Windows Terminal console FSV. If foreground now holds, proceed through the 29-profile launch matrix and required edge cases.

## 2026-05-31T14:26:17-05:00
- Active issue remains #606 `scenario(stress): act_run_shell orchestration - allowlist modes, timeout, 1MB cap, idempotency`.
- Implementation patch is in the worktree:
  - `crates/synapse-mcp/src/m4.rs`: split shell authorization from execution, added 600000 ms max timeout validation, idempotency key validation, request hashing, and `CF_KV` idempotency row encode/decode/replay helpers.
  - `crates/synapse-mcp/src/server/m4_tools.rs`: `act_run_shell` now writes `CF_ACTION_LOG` start/result audit rows, authorizes before execution, reserves/completes idempotency rows, replays exact retries, and rejects conflicting reuse.
  - `crates/synapse-mcp/src/server.rs`: exports the new M4 helpers into the server tool module.
- Wired Synapse MCP client/tool surface was checked after compaction:
  - `mcp__synapse.health` returned `ok=true`, installed stdio runtime PID `45712`, `allow_shell_patterns=any`.
  - `mcp__synapse.storage_inspect` returned live row counts and samples from `C:\Users\hotra\AppData\Local\synapse\db`.
- #606 manual FSV evidence captured through repo-built `C:\code\Synapse\target\release\synapse-mcp.exe` and official MCP Inspector strict client:
  - Permissive daemon run `.runs\606\permissive-20260531T140952`, PID `37100`, bind `127.0.0.1:7799`, isolated DB/logs, strict `tools/list` count `80`, `act_run_shell` present, schema readback `timeout_ms default=30000 min=1 max=600000`. PID stopped; port closed.
  - Happy permissive shell: before `happy.txt` absent and `CF_ACTION_LOG=2`; trigger wrote `work\happy.txt`; after file bytes `shell-happy-606`, stdout `stdout-606:extra-606`, and storage `CF_ACTION_LOG=4`.
  - Env containment: child process env readback contained `PATH`, `USERPROFILE`, `TEMP`, `SystemRoot`, explicit `SYNAPSE_EXTRA_ENV`, plus PowerShell-created `PathEXT`/`PSMODULEPATH`; broad parent secrets like `APPDATA`/`EXA_API_KEY` were absent.
  - Output cap: >1MB stdout trigger returned `stdout.Length=1048576`, first/last char `x`, `stdout_truncated=true`, `timed_out=false`.
  - Timeout edge: 500 ms trigger returned `timed_out=true`, `duration_ms=529`; after readback `work\timeout-late.txt` absent and `CF_ACTION_LOG` advanced.
  - Idempotency: exact retry with `idempotency_key=issue-606-idem-1` returned `count=1` both times; file `work\idem.txt` stayed `1`; `CF_KV` advanced `0->1` with key hash `03c48bf3f99f7f005e5d9309b3e624abdbfce5687c29e1a0c9e0d2dd369145d9`, request hash `e2901db689891e7af70c7c028d592d76ec3a57c5c2184d891ab66d9c102a2a34`, status `ok`, stored response `count=1`.
  - Idempotency conflict: reused same key with different command failed with `idempotency_key was already used for different parameters`; after readback `work\idem-conflict.txt` absent and `CF_KV` stayed `1`.
  - Empty/structurally invalid command: whitespace command failed with `act_run_shell command must not be empty`; `CF_ACTION_LOG` advanced with `TOOL_PARAMS_INVALID`.
  - Default and max timeout: omitted `timeout_ms` logged `timeout_ms=30000`; explicit `timeout_ms=600000` ran and logged `timeout_ms=600000`.
  - Restrictive daemon run `.runs\606\restrictive-20260531T141636`, PID `49920`, bind `127.0.0.1:7800`, `SYNAPSE_ALLOW_SHELL_ANY=0`, `SYNAPSE_ALLOW_SHELL=^cmd\.exe /c "echo allowlisted-606"$`, strict `tools/list` count `80`; PID stopped and port closed.
  - Restrictive allow/deny: allowed command printed `allowlisted-606`; denied `cmd.exe /c "echo denied-606"` failed with `SAFETY_SHELL_DENIED_BY_POLICY`; `CF_ACTION_LOG` readback `0->2->4`, `CF_KV=0`.
  - Malformed regex startup edge: run `.runs\606\malformed-20260531T142400`, PID `45164`, bind `127.0.0.1:7801`, pattern `^(cmd\.exe /c echo malformed-606$`; process exited `1`, port never opened, stderr showed `regex parse error` / `unclosed group`.
  - Above-max timeout boundary: run `.runs\606\above-max-20260531T142425`, PID `7604`, bind `127.0.0.1:7802`, strict `tools/list` ok; `timeout_ms=600001` failed with `act_run_shell timeout_ms must be 1..=600000`, after storage `CF_ACTION_LOG=2`, `CF_KV=0`, action log had started/error rows with `TOOL_PARAMS_INVALID`; PID stopped and port closed.
- Final #606 supporting checks after the last edit are green:
  - `cargo fmt --check`
  - `cargo check -p synapse-mcp`
  - `cargo test -p synapse-mcp shell_idempotency -- --nocapture`
  - `cargo test -p synapse-mcp shell_rejects_timeout_above_max -- --nocapture`
  - `cargo clippy -p synapse-mcp --all-targets -- -D warnings`
  - `cargo build --release -p synapse-mcp`
  - `git diff --check` exited 0 with line-ending warnings only.
- Diff review completed for `m4.rs`, `server.rs`, `server/m4_tools.rs`, and state notes. Next: commit/push with `[skip ci]`, post #606 RESOLVED evidence, close #606, then refresh the live queue.

## 2026-05-31T13:40:46-05:00
- #605 `scenario(stress): release_all + panic-hook + stuck-key auto-release safety` is closed.
  - Commit: `e0ea7e1 fix(action): harden release_all input recovery (#605) [skip ci]`
  - RESOLVED evidence: https://github.com/ChrisRoyse/Synapse/issues/605#issuecomment-4587679836
  - Closure readback: `gh issue close 605` succeeded; refreshed open queue no longer lists #605.
- Post-#605 cleanup/readback:
  - `git status --short --branch`: `## main...origin/main`
  - remaining live `synapse-mcp.exe`: PID `45712`, installed chat runtime at `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`
  - ports `127.0.0.1:7797` and `127.0.0.1:7798` have no listener
  - OS input SoT readback: Shift/Ctrl/Alt/P/LBUTTON/RBUTTON/MBUTTON all false.
- Active issue is now #606 `scenario(stress): act_run_shell orchestration - allowlist modes, timeout, 1MB cap, idempotency`.
  - START comment: https://github.com/ChrisRoyse/Synapse/issues/606#issuecomment-4587680954
  - Issue acceptance requires real MCP `tools/call` triggers plus separate physical SoT readbacks for shell output/files/process state, environment exposure, timeout/output-cap behavior, deny-policy logs, idempotency, and storage/action log rows.
  - Next: inspect existing `act_run_shell` implementation, policy/env/timeout/logging/idempotency code paths, then patch only if the code/FSV exposes gaps.
- Current live open queue after closing #605: #594 parent plus #595-#604 and #606-#634.

## 2026-05-31T13:18:00-05:00
- Active work remains #605 `scenario(stress): release_all + panic-hook + stuck-key auto-release safety`.
- Required wake-up context was re-read after compaction and reconciled with live GitHub/git state:
  - branch `main`; HEAD `632a834 fix(action): recover held inputs after daemon crash (#635) [skip ci]`
  - live queue still has #594 plus #595-#634 open; #605 active with START comment only.
- Current #605 patched release FSV daemon:
  - PID `50668`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`
  - bind `127.0.0.1:7797`
  - run dir `.runs\605\release-fsv-final2-20260531T130745`
  - DB `.runs\605\release-fsv-final2-20260531T130745\db`
  - recovery ledger `.runs\605\release-fsv-final2-20260531T130745\action_recovery.jsonl`
  - health ok, action `operator_hotkey=registered`, strict Inspector `tools/list` count 80 with #605 tools present.
- #605 manual FSV evidence captured so far on PID `50668` using official MCP Inspector `tools/call` triggers and separate SoT reads:
  - Empty `release_all`: before OS inputs up/recovery ledger absent/storage zero-ish; trigger returned zero releases; after OS up, ledger absent, storage `CF_ACTION_LOG` advanced with release_all ok.
  - Active reflex + pad cleanup after compaction delay: trigger `release_all` returned `released_buttons=2`, `neutralized_pads=1`; after OS buttons up, XInput slot0 neutral, recovery ledger absent, `CF_ACTION_LOG=4`, `CF_REFLEX_AUDIT=6`, disabled hold-button audit rows have `details.reason="release_all"`.
  - Active key cleanup: registered `hold_move` Shift+Ctrl, before OS Shift/Ctrl down and ledger had key_held rows; trigger `release_all` returned `released_keys=2`; after OS all up, ledger absent, `CF_ACTION_LOG=8`, `CF_REFLEX_AUDIT=12`, log `M2_RELEASE_ALL_READBACK` shows `before_held_keys=[shift,ctrl]`, `cancelled_key_timers=2`, disabled reflex id `019e7f3f-ab15-77a3-81d8-99331c4fea83`.
  - Stuck-key auto-release: registered `hold_move` Shift, before OS Shift down and ledger key_held; after 32s OS Shift up, ledger absent, `reflex_list` showed `019e7f40-4621-7522-9204-1590fdd0dced` expired with `REFLEX_LIFETIME_EXPIRED`, log showed `STUCK_KEY_AUTO_RELEASED key=shift`.
  - Operator hotkey: held middle mouse via `hold_button` and XInput A via `act_pad`; before MBUTTON true, XInput A true, ledger had button_held/pad_held; trigger `act_press keys=[ctrl,alt,shift,p]`; after OS all up, XInput neutral, ledger absent, `CF_ACTION_LOG=12`, `CF_REFLEX_AUDIT=16`, log `SAFETY_OPERATOR_HOTKEY_FIRED` with `release_all_result="ok"` and disabled reflex `019e7f41-6785-7221-b2a2-fcc32fa9cdf1` reason `operator_hotkey`.
  - Invalid params edge: before OS all up, ledger absent, `CF_ACTION_LOG=12`; trigger `act_press keys=[]`; after OS all up, ledger absent, `CF_ACTION_LOG=14`; log/audit show `TOOL_PARAMS_INVALID` and message `act_press keys must contain at least one key`.
- #605 panic-hook debug FSV evidence captured on repo-built debug daemon:
  - PID `53320`, binary `C:\code\Synapse\target\debug\synapse-mcp.exe`, bind `127.0.0.1:7798`, run dir `.runs\605\panic-fsv-20260531T132034`, DB `.runs\605\panic-fsv-20260531T132034\db`, ledger `.runs\605\panic-fsv-20260531T132034\action_recovery.jsonl`.
  - Started with `SYNAPSE_MCP_FORCE_PANIC_DURING_ACT=act_press_after_keydown`, `SYNAPSE_MCP_REQUIRE_OPERATOR_HOTKEY=1`, isolated log/recovery env, and repo-built debug binary.
  - Precondition: startup log `ACTION_CRASH_RECOVERY_READBACK ... after=no_stale_inputs`, operator hotkey registered, health ok, strict Inspector `tools/list` count 80 with `act_press`, `release_all`, `storage_inspect`.
  - Before trigger: OS Shift/Ctrl/Alt/P/LBUTTON/RBUTTON/MBUTTON all false, panic ledger absent, storage `CF_ACTION_LOG=0`.
  - Trigger: real Inspector `tools/call act_press keys=["shift"] hold_ms=30000 backend=software`; Inspector exited 1 with `MCP error -32001: Request timed out`, because the request task panicked.
  - After readback: OS all inputs false, ledger absent, process PID `53320` still alive and port `7798` listening, health ok, storage `CF_ACTION_LOG=1`, logs contain `M2_ACT_PRESS_FORCE_PANIC_AFTER_KEYDOWN`, panic hook `SAFETY_RELEASE_ALL_FIRED reason="panic" result="ok"`, emitter release readback `released_keys=1 cancelled_key_timers=1`, and telemetry panic capture for `forced panic during act_press after keydown`.
  - Stopped debug daemon PID `53320`; port `7798` closed.
- Final #605 supporting checks after the last edit:
  - `cargo fmt --check`
  - `cargo check -p synapse-action`
  - `cargo check -p synapse-reflex`
  - `cargo check -p synapse-mcp`
  - `cargo test -p synapse-mcp release_all_disables_reflexes_before_draining_actor_state -- --nocapture`
  - `cargo test -p synapse-reflex hold_move_safety_cap_expires_after_held_key_limit --test hold_move_behavior -- --nocapture`
  - `cargo clippy -p synapse-action -p synapse-reflex -p synapse-mcp --all-targets -- -D warnings`
  - `cargo build --release -p synapse-mcp`
  - `git diff --check` exited 0 with only line-ending warnings.
- Diff review completed for the code and state changes.
- Next: commit/push with `[skip ci]`, post #605 RESOLVED evidence, close #605, then continue the open issue queue.

## 2026-05-31T12:58:00-05:00
- Active work remains #605 `scenario(stress): release_all + panic-hook + stuck-key auto-release safety`.
- Second #605 defect found from manual run: active hold-button reflexes reasserted mouse buttons after `release_all` because the tool drained only M2 actor state and did not quiesce M3 reflex scheduling. Cleaned the stale daemon through real Inspector `reflex_cancel` calls, then old `release_all`; OS key/button SoT readback showed Shift/Ctrl/Alt/P/LBUTTON/RBUTTON/MBUTTON all up and the recovery ledger absent.
- Patched:
  - `crates/synapse-reflex/src/lifecycle.rs`: `disable_all_by_operator` now stops the scheduler after disabling active reflexes so no in-flight tick can reassert held input after the action emitter drains.
  - `crates/synapse-mcp/src/m2/release_all.rs`: `release_all_with_handles` accepts the initialized reflex runtime, disables active reflexes before draining M2 held state, logs disabled reflex ids/result, and still attempts M2 release even if reflex disable reports an error.
  - `crates/synapse-mcp/src/server/context.rs`, `server/m2_tools.rs`, and `server/everquest_autocombat.rs`: release-all context now carries the optional initialized reflex runtime.
  - Added focused regression `release_all_disables_reflexes_before_draining_actor_state`.
- Supporting checks after patch:
  - `cargo fmt`
  - `cargo check -p synapse-reflex`
  - `cargo check -p synapse-mcp`
  - `cargo check -p synapse-action`
  - `cargo test -p synapse-mcp release_all_disables_reflexes_before_draining_actor_state -- --nocapture`
  - `cargo test -p synapse-reflex hold_move_safety_cap_expires_after_held_key_limit --test hold_move_behavior -- --nocapture`
  - `cargo build --release -p synapse-mcp`
- Fresh patched release daemon for #605 manual FSV:
  - PID `52416`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`
  - bind `127.0.0.1:7797`
  - run dir `.runs\605\release-fsv-final-20260531T125355`
  - DB `.runs\605\release-fsv-final-20260531T125355\db`
  - recovery ledger `.runs\605\release-fsv-final-20260531T125355\action_recovery.jsonl`
  - logs `.runs\605\release-fsv-final-20260531T125355\logs`
  - startup log readback: `ACTION_CRASH_RECOVERY_READBACK ... after=no_stale_inputs recovered_keys=0 recovered_buttons=0 recovered_pads=0`, `operator hotkey registered`, `MCP_HTTP_STARTED bind=127.0.0.1:7797`
  - Inspector `health` readback: `ok=true`, `operator_hotkey=registered`, DB path isolated.
  - Strict Inspector `tools/list` readback: 80 tools; `release_all`, `act_press`, `act_pad`, `reflex_register`, `reflex_list`, and `storage_inspect` present.
- Next: run #605 manual behavior FSV on the patched daemon: empty release_all, active reflex + pad release_all, stuck-key auto-release, operator hotkey, invalid params, and debug panic-hook recovery.

## 2026-05-31T12:21:00-05:00
- Active work remains #605 `scenario(stress): release_all + panic-hook + stuck-key auto-release safety`.
- During the first #605 release run, the real MCP path proved `release_all` physically released held mouse buttons and neutralized the ViGEm pad, but separate recovery-ledger SoT readback still showed unmatched `button_held` rows after release. That was treated as a real state-reset defect.
- Code changes now staged in the worktree:
  - `crates/synapse-action/src/backend/software.rs`: `software::release_all` clears crash-recovery key/button ledger entries after successful release.
  - `crates/synapse-reflex/src/kinds/hold_move.rs`: hold-move reflex safety cap now waits 1000 ms beyond `HELD_KEY_MAX_DURATION_MS` so action-emitter stuck-key auto-release can fire deterministically instead of racing the reflex lifetime cap.
  - `crates/synapse-reflex/tests/hold_move_behavior.rs`: updated the supporting safety-cap regression to the new cap.
- Supporting checks passed after the patch:
  - `cargo fmt`
  - `cargo check -p synapse-action`
  - `cargo check -p synapse-reflex`
  - `cargo test -p synapse-reflex hold_move_safety_cap_expires_after_held_key_limit --test hold_move_behavior -- --nocapture`
  - `cargo build --release -p synapse-mcp`
- Old #605 release daemon PID `9276` was stopped; port `127.0.0.1:7797` is closed; OS key/button SoT reads clean.
- Next: launch a fresh post-fix repo-built daemon, strict Inspector `tools/list`, then redo #605 manual FSV from a clean run directory.

## 2026-05-31T11:47:00-05:00
- Required wake-up context was re-read after compaction:
  - `C:\code\Synapse\docs\AICodingAgentSuperPrompt.md`
  - `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`
  - `AGENTS.md`
  - #351 manual-FSV/no-CI decision and context comments
  - live open queue and active issue comments
  - `git status`, `git log -10`, and current branch
- Git state readback:
  - branch `main`
  - `git status --short --branch`: `## main...origin/main`
  - `HEAD`: `632a834 fix(action): recover held inputs after daemon crash (#635) [skip ci]`
- #635 is closed with RESOLVED evidence at https://github.com/ChrisRoyse/Synapse/issues/635#issuecomment-4587371130.
- Live open queue now contains #594 plus #595-#634. Active work is #605 `scenario(stress): release_all + panic-hook + stuck-key auto-release safety`.
- Posted #605 START comment: https://github.com/ChrisRoyse/Synapse/issues/605#issuecomment-4587382776.
- Current wired `mcp__synapse.health` loads and reports `ok=true`; its action subsystem is usable but `operator_hotkey=unavailable` because it started while an older leaked daemon owned Ctrl+Alt+Shift+P.
- Process SoT before cleanup showed five `synapse-mcp.exe` processes. I preserved likely active chat MCP PID `45712` and stopped leaked siblings `50580`, `49028`, `51096`, and `43260`; process readback now shows only PID `45712`.
- #605 code read so far:
  - `release_all` snapshots actor held state before/after and logs `M2_RELEASE_ALL_READBACK`.
  - action panic hook calls `RELEASE_ALL_HANDLE.fire_release_all_blocking_with_timeout` with a 10 ms timeout and logs `SAFETY_RELEASE_ALL_FIRED`.
  - operator hotkey disables reflexes and calls the same release path with a 50 ms budget.
  - key auto-release timers are scheduled on `KeyDown` and emit `STUCK_KEY_AUTO_RELEASED` at `HELD_KEY_MAX_DURATION_MS=30000`.
  - debug-only `SYNAPSE_MCP_FORCE_PANIC_DURING_ACT=act_press_after_keydown` panics after a real `act_press` keydown, giving a real held-key panic trigger.
- Next: build repo runtime, launch an isolated HTTP daemon with `SYNAPSE_MCP_REQUIRE_OPERATOR_HOTKEY=1`, strict Inspector `tools/list`, and run #605 manual FSV against OS key/button state, XInput/ViGEm state, storage samples, process/log bytes, and recovery ledger state.

## Previous 2026-05-31T10:49:28-05:00 Snapshot
- Required wake-up context was re-read after compaction:
  - `C:\code\Synapse\docs\AICodingAgentSuperPrompt.md`
  - `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`
  - `AGENTS.md`
  - #351 manual-FSV/no-CI decision and context issue/decision lists
  - live open queue and active issue comments
  - `git status`, `git log -10`, and current branch
- Git state readback:
  - branch `main`
  - `git status --short --branch`: `## main...origin/main`
  - `HEAD`: `a3f6c43 docs(state): record all issues closed [skip ci]`
- Prior queue #589/#590/#588/#585 is closed with evidence. #585 implementation commit is `0814a41` and evidence is at https://github.com/ChrisRoyse/Synapse/issues/585#issuecomment-4587147620.
- New live open queue now contains 42 issues: #594 parent context plus #595-#635 child stress/showcase issues, all opened after the prior all-clear state.
- #594 mission: prove every live Synapse MCP tool under load with manual FSV, real MCP `tools/call` triggers, strict client-parity `tools/list`, and separate physical SoT readbacks.
- Active first child: #635 `scenario(stress): crash recovery + concurrent-call thread safety (UIA MTA)`.

## Current Queue Snapshot
- #594 remains parent context and should stay open until all children are resolved or explicitly dispositioned.
- P1 children currently open: #595, #596, #600, #603, #605, #606, #607, #608, #609, #613, #614, #616, #617, #621, #624, #633, #634, #635.
- P2/P3 showcase and breadth children are also open: #597-#599, #601-#602, #604, #610-#612, #615, #618-#620, #622-#623, #625-#632.

## Active Work: #635
- Goal: prove daemon crash recovery leaves no inputs held and concurrent tool calls do not trigger UIA cross-thread errors/panics.
- Required runtime FSV shape:
  - prove repo-built `synapse-mcp` process/bind/auth/health/MCP init/strict `tools/list` before behavior acceptance.
  - trigger held-input, concurrent observe/find/action/reflex/profile paths via real MCP `tools/call`.
  - after each trigger read separate SoTs: OS key/button state, XInput/pad state where applicable, process/socket state, `health`, `storage_inspect`/`CF_ACTION_LOG`, daemon log bytes, and UIA worker diagnostics.
  - happy path plus at least 3 edges: crash during combo/reflex, crash during storage write, concurrent observe during `profile_activate`, rapid restart loop, plus invalid/empty/boundary params where applicable.
- Investigation next: inspect existing action held-state/release-all/panic-hook/startup recovery code and MCP concurrency surfaces, then launch a repo-built daemon on an isolated DB for manual evidence.
- Implementation added an action crash-recovery ledger and startup replay path in `synapse-action`/`synapse-mcp`.
- Supporting checks already passed before runtime FSV: `cargo fmt`, `cargo check -p synapse-action`, `cargo check -p synapse-mcp`, focused `synapse-action` recovery tests, `cargo clippy -p synapse-action -p synapse-mcp --all-targets -- -D warnings`, and `cargo build --release -p synapse-mcp`.
- Manual FSV run directory: `.runs\635\http-fsv-20260531T1106`.
- Repo-built daemon evidence:
  - initial PID `46188`, bind `127.0.0.1:7796`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`.
  - strict MCP Inspector `tools/list` succeeded with 80 tools and required #635 tools present.
  - real `tools/call health`, `profile_list`, `profile_activate`, `storage_inspect`, `act_press`, and `release_all` have been used in the run.
- Happy crash-recovery FSV evidence:
  - SoT before: Shift up, recovery ledger absent, `CF_ACTION_LOG=0`.
  - Trigger: Inspector `tools/call act_press` with `keys=["shift"]`, `hold_ms=30000`, `backend=software`; OS Shift became down and `action_recovery.jsonl` contained a `key_held` row.
  - Forced-kill: stopped PID `46188`; socket closed; OS Shift remained down; ledger still contained the held key.
  - Restart with the same configured recovery-file path: new PID `43200`; startup log `ACTION_CRASH_RECOVERY_READBACK ... after=stale_inputs_released recovered_keys=1`; OS Shift up; ledger removed; `release_all` returned zero held inputs.
- Note: one intermediate restart intentionally showed a setup mismatch, looking under `db\action_recovery.jsonl` while the original daemon used the explicit run-root ledger. Acceptance evidence uses the stable configured ledger path and records that path.
- #635 manual FSV status: behavior evidence captured in `.runs\635\http-fsv-20260531T1106`.
  - Happy path: `act_press` long Shift hold killed at PID `46188`; restart with same ledger path released one stale key and removed the ledger.
  - Edge 1: `act_combo` scheduled a long Shift hold; killing PID `43200` left Shift down and ledger populated; restart released one stale key.
  - Edge 2: `storage_put_probe_rows` was killed while its Inspector client was still running; restart reopened RocksDB, `health` was ok, and `storage_inspect` read `CF_KV=0`, `CF_ACTION_LOG=4` with no corruption.
  - Edge 3: concurrent Inspector clients for `observe`, `find`, `profile_activate`, `act_press`, `reflex_register`, and `storage_inspect` all returned `isError=false`; Shift ended up; `CF_ACTION_LOG` advanced 4->6 and `CF_REFLEX_AUDIT` advanced 1->2; daemon log showed one `A11Y_UIA_WORKER_READY` and zero cross-thread/RPC wrong-thread/panic matches.
  - Edge 4: invalid `act_press keys=[]` returned MCP error `act_press keys must contain at least one key`; Shift stayed up; ledger absent; `CF_ACTION_LOG` advanced 6->8 with `TOOL_PARAMS_INVALID`.
  - Edge 5: three explicit rapid restart cycles succeeded on `127.0.0.1:7796`; final PID `42120`, strict Inspector `tools/list` count 80, `health.ok=true`, Shift up, recovery ledger absent.
  - Log readback across 8 stderr files: `CrossThreadOrPanicCount=0`; only non-hotkey error was the intentional invalid-param response.
- Supporting checks after FSV:
  - `cargo fmt --check`
  - `git diff --check` (exit 0; line-ending warnings only)
  - `cargo check -p synapse-action`
  - `cargo check -p synapse-mcp`
  - `cargo test -p synapse-action recovery_log --lib -- --nocapture`
  - `cargo clippy -p synapse-action -p synapse-mcp --all-targets -- -D warnings`
  - `cargo build --release -p synapse-mcp`
- Diff review completed for the action recovery module, keyboard/mouse/ViGEm ledger hooks, MCP startup recovery hook, Cargo metadata, and state notes.
- Current next step: commit with `[skip ci]`, post #635 RESOLVED evidence, close #635, then continue the open queue.

## Standing Rules
- No GitHub Actions/CI dispatch, waits, or CI-gated claims.
- Commits pushed by this agent must include `[skip ci]`.
- Automated checks/benches can support regression confidence only; they are not FSV.
- Missing local prerequisites are acquisition/setup work, not blockers, until only a hard-to-reverse operator-only external action remains.
