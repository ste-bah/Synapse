# Issue 1706 - durable SSH shell jobs

Date: 2026-07-16
Issue: https://github.com/ChrisRoyse/Synapse/issues/1706

## Root Cause

`shell start` rejected every durable SSH request before planning because Synapse had no way to bind a long-lived local `ssh` client to a verified remote cleanup handle while preserving SSH stdout/stderr, host-key/auth behavior, account shell behavior, and stdin semantics. That made any remote command needing more than the inline 110-second client budget impossible to observe through Synapse without replaying the remote operation.

## Fix

Durable SSH is now supported only for direct `ssh` executable invocations whose semantics can be tracked. Unsupported subsets fail before child spawn with typed `ACTION_TARGET_INVALID` reasons and persisted `spawn_refused` job artifacts.

Implemented behavior:

- Direct `ssh` argv is parsed into control args, destination, and remote command.
- Shell-wrapped SSH is refused before spawn with `ssh_durable_tracking_requires_direct_ssh_argv`.
- Interactive SSH is refused before spawn with `ssh_remote_command_required`.
- `-N`, `-f`, `-s`, `-W`, `-O`, `-Q`, unknown unsafe options, forwarding, multiplexing, mutable config dependency, and untrusted SSH executables fail closed before spawn.
- Automatic replay uses hardened OpenSSH args: config isolation (`-F none`), noninteractive auth, strict host-key checking, no forwarding, no multiplexing, no agent/X11, no tty, stdin null, no local commands, and no askpass prompts.
- Remote execution runs through a Python guardian under the remote account shell. The guardian emits `SYNAPSE_REMOTE_PROCESS_V1` and `SYNAPSE_REMOTE_EXIT_V1`, records boot id, pid, pgid, sid, process start time, and an ownership-token hash, and terminates its exact process group on cancellation.
- Remote cleanup metadata is persisted in schema version 4 `remote-cleanup.json`.

## Research Used

- Exa search found the OpenBSD `ssh_config(5)` manual and Linux man-pages for SSH client options.
- OpenBSD `ssh_config(5)` documents that `BatchMode=yes` disables user interaction, `StrictHostKeyChecking=yes` refuses unknown or changed host keys, `StdinNull` prevents reading stdin, `ControlMaster`/`ControlPath` reuse a shared connection, `ForkAfterAuthentication` backgrounds SSH, and `SessionType none` is equivalent to `-N`: https://man.openbsd.org/ssh_config
- Linux `setsid(2)` documents that `setsid()` creates a new session/process group only when the caller is not already a process-group leader, and fails for an existing group leader. This shaped the guardian's fallback to `scope=existing_process_group_leader` only when `pgid == pid`: https://man7.org/linux/man-pages/man2/setsid.2.html
- Linux `pidfd_send_signal(2)` documents stable process identity as the way to avoid PID reuse races. Synapse applies the same principle remotely with boot id + pid + start time + ownership token readback before cleanup: https://man7.org/linux/man-pages/man2/pidfd_send_signal.2.html

## Source Of Truth

- Runtime/client SoT: real `mcp__synapse` client tool surface, `health`, daemon PID, and TCP bind.
- Shell job SoT: `%LOCALAPPDATA%\synapse\shell-jobs\jobs\<job_id>\request.json`, `status.json`, `stdout.log`, `stderr.log`, and `remote-cleanup.json`.
- Local process SoT: Windows `Win32_Process` rows for `synapse-mcp.exe`, `ssh.exe`, and daemon children.
- Remote process SoT: WSL Ubuntu process table and socket table (`ps`, `ss`) for the temporary loopback sshd and remote guardian groups.
- Host-key/auth setup SoT: `C:\Users\hotra\.ssh\known_hosts`, WSL `/home/cabdru/.ssh/authorized_keys`, and temporary sshd config/log paths.

## Runtime Preconditions

Real MCP client surface was used for all triggers.

- `mcp__synapse.health` returned `ok=true`.
- Daemon PID: `75204`.
- Bind: `127.0.0.1:7700`.
- Tool count: `40`.
- Tool surface SHA-256: `e20cb889682709ec22f9b571f043da594ffe1d6c40168566235fe45d4654bb12`.
- Tool names included `shell`.
- Process readback:
  - `ProcessId=75204`
  - `ExecutablePath=C:\Users\hotra\.cargo\bin\synapse-mcp.exe`
  - `CommandLine="C:\Users\hotra\.cargo\bin\synapse-mcp.exe" --mode http --bind 127.0.0.1:7700 --db C:\Users\hotra\AppData\Local\synapse\db-daemon ...`
- Socket readback showed `127.0.0.1:7700` owned by PID `75204`.

## Temporary SSH Target

Windows OpenSSH Server setup required elevation, so the reversible local prerequisite was a loopback-only WSL OpenSSH server.

- Distro: `Ubuntu-26.04`.
- Port: `127.0.0.1:22222`.
- Listener before FSV: `sshd` PID `1004`.
- Config: `/tmp/synapse-fsv-sshd-1706.conf`.
- Log: `/tmp/synapse-fsv-sshd-1706.log`.
- Host key line added to active `known_hosts` before FSV:
  - `[127.0.0.1]:22222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIG3G0dxqr+pyiZwpuPzuqL8rtNQ6YMoYrOl3uwjFCvCt`
- Direct OpenSSH prerequisite smoke readback returned remote UID `1000` and Python `3.14.4`.

## Happy Path FSV - long-running remote command

Trigger: real `mcp__synapse.shell` with `operation=start`, `command=ssh`, timeout `300000`, direct SSH args, and remote command:

```text
python3 -c 'import os,time; print("SYNAPSE1706_START", os.getpid(), flush=True); time.sleep(115); print("SYNAPSE1706_DONE", os.getpid(), flush=True)'
```

Expected:

- Job starts once.
- Remote output is persisted before and after the 110-second inline ceiling.
- `status` reports running while the remote process is alive.
- Final status is `ok`, exit code `0`, duration greater than 110 seconds.
- Separate SoT reads show no local SSH or remote process group remains after completion.

Actual:

- Job id: `019f6abb-3c8f-7d50-85d5-99584761e970`.
- Initial start returned local SSH PID `48228` and `status=running`.
- Job directory contained `request.json`, `status.json`, `stdout.log`, `stderr.log`, and `remote-cleanup.json`.
- `stderr.log` contained:
  - `SYNAPSE_REMOTE_PROCESS_V1 job_id=019f6abb-3c8f-7d50-85d5-99584761e970 pid=1654 pgid=1654 sid=1654 boot_id=0bbbd7a4-d6c8-4774-8b21-74d81a205624 start_time=28017145 scope=existing_process_group_leader ownership_token_sha256=7ec3...`
- `stdout.log` while running contained `SYNAPSE1706_START 1655`.
- WSL process table while running showed guardian PID/PGID/SID `1654` and payload PID `1655` in PGID `1654`.
- Windows process table while running showed local `ssh.exe` PID `48228`, parent `75204`, with hardened args including `-F none` and `-T`.
- MCP status while running returned:
  - `status=running`
  - `remote_cleanup_status=remote_process_tracked`
  - `remote_process_id=1654`
  - `remote_process_group_id=1654`
  - `remote_liveness_marker:SYNAPSE_REMOTE_LIVENESS_V1:pgid=1654:status=alive`
- MCP status after more than 110 seconds returned:
  - `status=ok`
  - `duration_ms=116110`
  - `exit_code=0`
  - stdout tail contained `SYNAPSE1706_START 1655` and `SYNAPSE1706_DONE 1655`
  - stderr tail contained `SYNAPSE_REMOTE_EXIT_V1 ... exit_code=0`
  - `remote_cleanup_verified=true`
  - `remote_cleanup_status=remote_cleanup_verified`
- Separate physical read after completion:
  - `status.json`: `status=ok`, `exit_code=0`, `duration_ms=116110`, `timed_out=false`, `remote_cleanup_verified=true`.
  - `remote-cleanup.json`: `schema_version=4`, `transport=ssh`, trusted command `\\?\C:\Program Files\Git\usr\bin\ssh.exe`, hardened effective args present, ownership token present.
  - Local PID `48228` absent.
  - WSL remote process group `1654` absent.

## Cancel FSV - exact remote cleanup

Trigger: real `mcp__synapse.shell` with `operation=start`, direct SSH args, timeout `600000`, and remote command:

```text
python3 -c 'import os,time; print("SYNAPSE1706_CANCEL_START", os.getpid(), flush=True); time.sleep(300); print("SYNAPSE1706_CANCEL_DONE", os.getpid(), flush=True)'
```

Then trigger: real `mcp__synapse.shell` with `operation=cancel`.

Expected:

- Job starts once and exposes remote process identity.
- Cancel terminates the exact local SSH process and the tracked remote process group.
- `CANCEL_DONE` never appears.

Actual:

- Job id: `019f6abe-451a-7792-932c-a82a2eef7b5a`.
- Local SSH PID: `21244`.
- Running status readback:
  - `status=running`
  - remote PID/PGID/SID `1826`
  - payload PID `1827`
  - remote liveness marker alive
  - stdout tail `SYNAPSE1706_CANCEL_START 1827`
- Cancel returned:
  - `before_status=running`
  - `status.status=cancelled`
  - `cancel_requested=true`
  - `exit_code=1`
  - `duration_ms=23816`
  - `remaining_process_ids=[]`
  - `termination_status=local_ssh_client_terminated_remote_cleanup_verified`
  - `remote_cleanup_verified=true`
  - `remote_cleanup_status=remote_cleanup_verified`
  - cleanup message reported remote pid `1826`, process group `1826`, status `terminated`.
- Separate physical read after cancel:
  - `status.json`: `status=cancelled`, `cancel_requested=true`, `remote_cleanup_verified=true`.
  - `stdout.log`: contains `SYNAPSE1706_CANCEL_START 1827`; does not contain `SYNAPSE1706_CANCEL_DONE`.
  - `stderr.log`: contains process marker for pid/pgid/sid `1826`.
  - Local PID `21244` absent.
  - WSL process-group readback showed no group `1826` members and no `SYNAPSE1706_CANCEL` payload command.

## Edge 1 - interactive SSH refused

Before:

- No live `ssh.exe` rows.
- Newest job before trigger: `019f6abe-451a-7792-932c-a82a2eef7b5a`.

Trigger: real `mcp__synapse.shell` with direct SSH args ending at destination `127.0.0.1`, no remote command.

Expected:

- Fail before spawn.
- Typed reason `ssh_remote_command_required`.
- No local SSH process or remote command.
- Persisted artifact accurately records refusal.

Actual:

- Tool error reason: `ssh_remote_command_required`.
- Requested job id: `019f6ac1-c229-7210-851f-106682d5c44f`.
- Separate job readback:
  - directory exists with `request.json` and `status.json` only.
  - `status.json`: `status=spawn_refused`, `pid=null`, `error_code=ACTION_TARGET_INVALID`, `timed_out=false`, `duration_ms=24`.
  - `remote_process_scope.remote_cleanup_required=false`.
  - `remote_process_scope.remote_cleanup_status=remote_process_never_started_or_untracked_pre_marker`.
  - `remote_cleanup_message=SSH tracking preflight refused the prepared plan before any child process was created`.
  - `detection_evidence` includes `direct_command_ssh:ssh` and `remote_tracking_pre_marker_terminal:tracking_preflight_refused_before_spawn`.
  - `stdout.log`, `stderr.log`, and `remote-cleanup.json` absent.
- Separate process readback:
  - no `ssh.exe`.
  - no WSL `SYNAPSE1706` remote process.

## Edge 2 - shell-wrapped SSH refused

Trigger: real `mcp__synapse.shell` with `command=powershell.exe`, args `-NoProfile -Command "ssh ... 127.0.0.1 hostname"`.

Expected:

- Fail before spawning the wrapper or SSH.
- Typed reason `ssh_durable_tracking_requires_direct_ssh_argv`.
- Persisted artifact accurately records refusal.

Actual:

- Tool error reason: `ssh_durable_tracking_requires_direct_ssh_argv`.
- Requested job id: `019f6ac2-8b87-7c93-964b-cf39813274fa`.
- Separate job readback:
  - directory exists with `request.json` and `status.json` only.
  - `status.json`: `status=spawn_refused`, `pid=null`, `command=powershell.exe`, `error_code=ACTION_TARGET_INVALID`, `duration_ms=20`.
  - `remote_cleanup_status=remote_process_never_started_or_untracked_pre_marker`.
  - `remote_cleanup_message=SSH tracking preflight refused the prepared plan before any child process was created`.
  - `detection_evidence` includes `shell_wrapped_ssh:powershell:ssh`, `tracking_preflight_refused_before_spawn`, and `no_child_process_created`.
- Separate process readback:
  - no daemon-owned `powershell.exe` wrapper process.
  - no `ssh.exe`.
  - no WSL `SYNAPSE1706` remote process.

## Edge 3 - unsupported `-N` mode refused

Trigger: real `mcp__synapse.shell` with direct SSH args including `-N`, destination, and dummy payload `hostname`.

Expected:

- Fail before spawn because `-N` requests no remote command/session execution semantics.
- Typed reason `ssh_no_remote_command_flag`.
- No local or remote SSH work starts.

Actual:

- Tool error reason: `ssh_no_remote_command_flag`.
- Requested job id: `019f6ac3-2c6f-73f3-9645-d52814b11f39`.
- Separate job readback:
  - directory exists with `request.json` and `status.json` only.
  - `status.json`: `status=spawn_refused`, `pid=null`, `error_code=ACTION_TARGET_INVALID`, `duration_ms=11`.
  - `remote_cleanup_status=remote_process_never_started_or_untracked_pre_marker`.
  - `detection_evidence` includes `direct_command_ssh:ssh`, `remote_tracking_unsupported:ssh_no_remote_command_flag`, and `tracking_preflight_refused_before_spawn`.
- Separate process readback:
  - no `ssh.exe`.
  - no WSL job id or `SYNAPSE1706` remote process.

## Structural Checks

These are compile/lint checks only, not FSV.

- `cargo fmt --all --check`: passed.
- `cargo check -p synapse-mcp`: passed.
- `cargo clippy -p synapse-mcp --all-targets`: passed.

## Cleanup Readback

Temporary FSV host state was cleaned up after verification.

- Verified temporary `sshd` process before cleanup:
  - PID `1004`
  - executable `/usr/sbin/sshd`
  - command line used `/tmp/synapse-fsv-sshd-1706.conf` and `/tmp/synapse-fsv-sshd-1706.log`.
- Killed exact PID `1004`.
- After cleanup:
  - no `127.0.0.1:22222` listener in WSL `ss`.
  - no `/tmp/synapse-fsv-sshd-1706.conf`, `.log`, or `.pid` files.
  - no `/home/cabdru/.ssh/authorized_keys`.
  - active `C:\Users\hotra\.ssh\known_hosts` has no `[127.0.0.1]:22222` line and length `1033` bytes.
  - no live Windows `ssh.exe`.

## Result

Issue #1706 acceptance is satisfied on the real configured host through the real `mcp__synapse.shell` facade:

- Durable direct SSH runs past the 110-second inline ceiling without replay.
- `shell status` reads persisted stdout/stderr, lifecycle, exit, and remote identity.
- `shell cancel` terminates the exact local SSH client and tracked remote process group.
- Unsupported SSH subsets fail closed before spawn with typed diagnostics and physical refusal artifacts.
- Manual Source-of-Truth readbacks prove the expected state before and after the happy path, cancel path, and three edge cases.
