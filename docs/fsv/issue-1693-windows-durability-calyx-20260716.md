# Issue #1693 Windows Durability FSV

Date: 2026-07-16

Issue: <https://github.com/ChrisRoyse/Synapse/issues/1693>

## Root Cause

`calyx-aster` had several independent durable-publish implementations. Manifest, SST, ledger-head, query-index, residency, panel-GC, slot-artifact, and base-page-index paths used raw `fs::rename`, local Win32 wrappers, or delete-before-rename. On Windows that left inconsistent replace semantics, no common `MOVEFILE_WRITE_THROUGH`, incomplete long-path handling, and poor sharing-violation diagnostics.

The lock path also used a blocking OS lock and terse lock-file open errors. A file held by another process could either hang too long or fail without the path, attempt count, error kind, and raw OS error needed to repair it.

While testing, the same long-path boundary appeared one layer above storage: `synapse-calyx` wrote identity/salt files with its own temp+rename, and `synapse-profiles` passed a `>260` char profile directory directly to `notify`, causing `PathNotFound` while the directory physically existed.

## Research Basis

Used Exa MCP plus web research against Microsoft documentation:

- `MoveFileExW`: `MOVEFILE_REPLACE_EXISTING` for replacement and `MOVEFILE_WRITE_THROUGH` to wait for copy/delete flush completion. <https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw>
- Windows file replacement/move behavior requires compatible sharing permissions on open destinations. <https://learn.microsoft.com/en-us/windows/win32/fileio/moving-and-replacing-files>
- Extended-length paths require the `\\?\` prefix, with UNC transformed to `\\?\UNC\...`. <https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation>
- Directory handles on Windows require `FILE_FLAG_BACKUP_SEMANTICS` for directory sync/open behavior. <https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew>

Implementation policy from that research: same-directory temp files, fsync temp before publish, atomic create-new or replace modes, parent directory sync, Windows verbatim paths at Win32/watcher boundaries, bounded retries only for transient sharing/lock errors, and fail-closed structured errors with path/kind/raw OS code.

## Changes

- Added shared `calyx-aster::fsync` durable filesystem boundary: atomic create-new/replace writes, durable path publish, directory creation/sync wrappers, Windows `MoveFileExW` with `MOVEFILE_WRITE_THROUGH`, long-path conversion, sharing/lock retry logging, and structured `CALYX_DISK_PRESSURE` details.
- Replaced scattered Aster publish call sites with the shared helper.
- Hardened `file_lock` open and acquire paths with bounded `try_lock`, attempt counts, path, error kind, raw OS error, and no indefinite blocking.
- Exposed a small safe `calyx_aster::durable_fs` wrapper and moved `synapse-calyx` identity/salt writes onto it.
- Added Windows verbatim path conversion for `synapse-profiles` watcher registration.

Related issue filed during audit: #1703 for leftover Calyx test-support residue found outside this issue scope.

## Structural Checks

These are compile/lint/format checks only, not FSV:

- `cargo fmt --manifest-path calyx/Cargo.toml --all`
- `cargo fmt --all`
- `cargo clippy --manifest-path calyx/Cargo.toml --workspace --all-targets`
- `cargo clippy --workspace --all-targets`
- `cargo build -p synapse-mcp`

All passed. The only repeated message was the existing `calyx-forge` build-script notice: `cuda feature not enabled, skipping kernel compilation`.

## Manual FSV

MCP client precondition:

- Real wired MCP client loaded the 40-tool surface, `tool_surface_sha256=7cc1d191749041ce3f0bb1a646d1f2b8553e6155e8a9f917553fd66c4cc257ad`.
- Repo-built daemon used for FSV: `C:\code\Synapse\target\debug\synapse-mcp.exe`.
- Normal-path FSV DB SoT: `C:\Users\hotra\AppData\Local\synapse\fsv\issue-1693\normal\db`.
- Long-path FSV DB SoT: 297-char path under `...\issue-1693\longpath\...\db`.

Before final triggers:

- `workspace list run=fsv-1693-final-20260716 prefix=issue1693/` returned `returned_count=0`.
- Physical scan of the normal-path DB found `FINAL_BEFORE_NO_MATCH`.
- `CURRENT` was `manifest-00000000000000000042.json`.

Happy path:

- Trigger: MCP `workspace.put` key `issue1693/final-happy`, value payload `issue-1693-final-happy-37`.
- MCP readback hash: `sha256:7637f85252cf4d2811491215ddef1e0d3c57fdbabad2ef423d9725b0575f689f`.
- Physical SoT: key/payload found in `cf\kv\00000000000000000043-0000.sst`, `44`, `45`, matching flush SSTs, and `wal\00000000000000000000.wal`.
- After happy+empty, `CURRENT=manifest-00000000000000000048.json`.

Edge 1, empty value:

- Before: physical scan found `EMPTY_EDGE_BEFORE_NO_MATCH`.
- Trigger: MCP `workspace.put` key `issue1693/final-empty`, value `""`.
- MCP `workspace.get` returned `value:""` and hash `sha256:5d6f34c37329d1dbb82ebcb5b460a1da07b6bdb885eaafe6155a900370200bde`.
- Physical SoT: key found in `cf\kv\00000000000000000046-0000.sst`, `47`, `48`, matching flush SSTs, and WAL.

Edge 2, invalid format:

- Trigger: MCP `workspace.put` key containing control char `issue1693/final-invalid\u0001control`.
- Result: real MCP error `TOOL_PARAMS_INVALID`, message `workspace key must not contain control characters`.
- After: `workspace list prefix=issue1693/final-invalid` returned `returned_count=0`; physical scan returned `FINAL_INVALID_AFTER_NO_MATCH`.

Edge 3, locked file:

- Before: `CURRENT=manifest-00000000000000000048.json`; `issue1693/final-lock-after-release` absent.
- Trigger setup: separate helper opened `...\db\wal\.append.lock` with `FileShare.None`, stdout `EXCLUSIVE_READY`.
- MCP `workspace.put` key `issue1693/final-share-blocked` failed closed:
  `open lock file ...\.append.lock attempts=80: kind=Uncategorized raw_os_error=Some(32): The process cannot access the file because it is being used by another process.; source_code=CALYX_DISK_PRESSURE`.
- After failure: physical scan returned `SHARE_BLOCKED_AFTER_NO_MATCH`.
- Release proof: helper stdout `EXCLUSIVE_READY` then `EXCLUSIVE_RELEASED`.
- Post-release trigger: MCP `workspace.put` key `issue1693/final-lock-after-release`, value `issue-1693-final-lock-after-release-persisted`.
- Physical SoT: key/payload found in `cf\kv\00000000000000000052-0000.sst`, `53`, `54`, matching flush SSTs, and WAL.

Crash/recover soak:

- Cycle 1: wrote `issue1693/final-crash-1` = `issue-1693-final-crash-1-ack`, killed exact PID `76092`, restarted PID `74016`, MCP readback found value/hash `sha256:fe522c11107aba164feb4148de80ba3b756977ffc4df79f97f9f6651ccbb0cf2`.
- Cycle 2: wrote `issue1693/final-crash-2` = `issue-1693-final-crash-2-ack`, killed exact PID `74016`, restarted PID `72432`, MCP readback found value/hash `sha256:5ae1b6837445610feb4343ca02c1fe7f86a5ed97f51456eea957565f62ea923c`.
- Cycle 3: wrote `issue1693/final-crash-3` = `issue-1693-final-crash-3-ack`, killed exact PID `72432`, restarted PID `76952`, MCP list returned all 3 values.
- Physical SoT after cycle 3: every crash key/payload found in SST and WAL; examples include `cf\kv\00000000000000000055-0000.sst` for crash-1, `61` for crash-2, `67` for crash-3, and WAL offsets `50189`, `56000`, `61811`.
- Final crash state: `CURRENT=manifest-00000000000000000072.json`, latest manifest `manifest-00000000000000000072.json`, WAL length `66710`.

Long-path edge:

- Initial long-path daemon proved Calyx opened but `health.ok=false` because profiles watcher failed with `PathNotFound` on a 303-char profile path. Root cause fixed by passing a verbatim path to `notify::Watcher::watch`.
- Rebuilt and restarted long-path daemon as PID `20176`.
- Long-path lengths: root `294`, DB `297`, profile `303`, vault `306`.
- MCP `health` after fix: `ok=true`; `profiles.status=ok`; `calyx_vault.status=ok`; `storage_backend=calyx`.
- Trigger: MCP `workspace.put` run `fsv-1693-longpath-20260716`, key `issue1693/final-long-path`, value `issue-1693-final-long-path-persisted`.
- MCP readback hash: `sha256:4c4fc2214ecdddcac7f45f1db80f6f83984568f72e101eea9a9769839a325aa3`.
- Physical SoT under 297-char DB path: key/payload found in `cf\kv\00000000000000000014-0000.sst`, `15`, `16`, matching flush SSTs, and WAL. `CURRENT=manifest-00000000000000000016.json`.

Host cleanup:

- Long-path daemon PID `20176` was shut down and exited.
- Normal scheduled daemon restored: PID `61428`, path `C:\Users\hotra\.cargo\bin\synapse-mcp.exe`, listener owner `61428`, standard DB `C:\Users\hotra\AppData\Local\synapse\db-daemon`.
- Final normal `health`: `ok=true`, `tool_count=40`, `profiles.status=ok`, `storage_backend=rocksdb`.
