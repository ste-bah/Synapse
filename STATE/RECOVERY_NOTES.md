# RECOVERY NOTES - Synapse

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
