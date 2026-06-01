# RECOVERY NOTES - Synapse

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
