# RECOVERY NOTES - Synapse

## Current Resume Point - 2026-06-01T08:10:00-05:00
- #614 implementation and manual MCP FSV evidence are captured. Do not relaunch FSV unless final checks or diff review expose a new defect.
- Current next action: commit with `[skip ci]`, post #614 RESOLVED evidence, close #614, refresh the open issue queue, then continue to the next open issue.
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
