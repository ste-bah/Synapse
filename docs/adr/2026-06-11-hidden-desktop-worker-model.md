# ADR: Hidden Win32 Desktop Worker Model

Status: Accepted
Date: 2026-06-11
Issue: #741
Parent: #721

## Context

Synapse needs many primary agents to use GUI applications concurrently without
stealing the operator's visible foreground, cursor, or active browser tab. The
open hidden-desktop epic proposes per-agent Win32 desktops so each agent has
its own window namespace and focus state.

The relevant Windows constraints are:

- A desktop is a securable object with its own windows, menus, and hooks.
  Window messages are only valid between processes on the same desktop, and
  only one `WinSta0` desktop is the active input desktop at a time.
- `CreateDesktopW` creates a desktop in the current window station. Desktop
  names cannot contain backslashes, and desktop count is bounded by desktop
  heap.
- `SetThreadDesktop` fails after the calling thread has windows or hooks.
- `STARTUPINFO.lpDesktop` connects a newly created process to a named desktop
  before its first GUI thread is initialized.
- UI Automation clients should make UIA calls from a separate MTA thread with
  no owned windows.
- `SwitchDesktop` makes a desktop visible and enables it to receive user input.
  Hidden desktops are therefore not a raw-input surface for the user's current
  input stream.

Sources:

- Microsoft Learn, Desktops:
  https://learn.microsoft.com/en-us/windows/win32/winstation/desktops
- Microsoft Learn, CreateDesktop:
  https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createdesktopa
- Microsoft Learn, SetThreadDesktop:
  https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setthreaddesktop
- Microsoft Learn, Thread Connection to a Desktop:
  https://learn.microsoft.com/en-us/windows/win32/winstation/thread-connection-to-a-desktop
- Microsoft Learn, STARTUPINFOW:
  https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/ns-processthreadsapi-startupinfow
- Microsoft Learn, OpenInputDesktop:
  https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-openinputdesktop
- Microsoft Learn, SwitchDesktop:
  https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-switchdesktop
- Microsoft Learn, SendInput:
  https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput
- Microsoft Learn, UI Automation threading:
  https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-threading
- UFO2 describes the stronger separate-session/PiP direction through Windows
  Remote Desktop loopback rather than ordinary hidden Win32 desktops:
  https://www.microsoft.com/en-us/research/publication/ufo2-the-desktop-agentos/

## Decision

Use a per-hidden-desktop child worker process, not in-process worker threads.

The daemon owns desktop lifecycle and launches a repo-built worker process onto
each hidden desktop with `CreateProcessW` and `STARTUPINFO.lpDesktop =
"WinSta0\\<desktop_name>"`. The worker process starts already connected to the
target desktop, then initializes its own per-desktop UIA MTA worker, window
enumeration, message dispatch, capture, and IPC loop.

The daemon communicates with each worker through a bounded local IPC channel.
Named pipes are the default choice for #742 because they are OS-native,
per-worker addressable, easy to tie to exact worker PIDs, and observable in
process/handle logs. The protocol must include `session_id`, `desktop_id`,
`request_id`, `target_pid`/`hwnd` where applicable, timeout, action tier, and
structured error code.

Hidden-desktop actions are background-tier actions:

- Prefer CDP for browsers.
- Prefer UIA control patterns and ValuePattern for accessible controls.
- Use same-desktop `SendMessage`/`PostMessage` paths for standard Win32 text,
  key, mouse, and scroll messages when UIA is unavailable and the target
  accepts messages.
- Do not route raw `SendInput` to a hidden desktop. A hidden Win32 desktop is
  not the active input desktop, so `SendInput` must fail closed with a precise
  error unless the desktop is explicitly made the input desktop. Switching the
  operator's session desktop is foreground takeover and is not permitted for
  background agents.
- For apps that require real raw input, DirectInput, or an active input
  desktop, use the heavier separate Windows session / RDP loopback design
  tracked by #746 instead of ordinary hidden Win32 desktops.

Capture on hidden desktops must be explicit and worker-local:

- WGC/GDI screen capture can be black or unavailable for hidden desktops.
- `PrintWindow` may be used only as an explicit hidden-desktop capture tier
  under #744, with bounded timeout, target PID/HWND attribution, structured
  failure codes, and no silent fallback. The existing visible-desktop capture
  policy remains correct: do not silently re-enter target windows with
  `PrintWindow` for ordinary target-window capture.

## Architecture

Daemon responsibilities:

- Create a sanitized desktop name: no backslash, bounded length, and globally
  unique by session/worker id.
- Create the desktop with least required desktop rights for the current user.
  Do not enable cross-account hooks.
- Launch `synapse-desktop-worker.exe` with `STARTUPINFO.lpDesktop` and
  hidden/no-console creation flags.
- Attach the worker and all apps it launches to exact job objects for cleanup.
- Persist `desktop_id`, `desktop_name`, `worker_pid`, `created_at`, owning
  session id, worker pipe name, and lifecycle state in storage.
- Record every launch/action/capture in `CF_ACTION_LOG` with
  `backend_tier_used`, `desktop_id`, `worker_pid`, `target_pid`, `hwnd`,
  `required_foreground=false`, and observed-delta status.

Worker responsibilities:

- Start already connected to the requested desktop. Do not call
  `SetThreadDesktop` after runtime initialization.
- Initialize a per-worker UIA MTA thread with no owned windows.
- Launch target apps onto the same desktop using `STARTUPINFO.lpDesktop`.
- Enumerate windows only on its own desktop.
- Execute CDP/UIA/message/capture actions only for owned desktop targets.
- Fail closed if a target HWND/PID belongs to another desktop or session.
- Return structured errors for target lost, unsupported action tier, timeout,
  worker crash, input-desktop-required, and capture-disabled cases.

## Manual Probe Evidence

Scratch probe: `.runs/issue741-hidden-desktop-probe`, not committed.

Source of Truth:

- Visible desktop foreground HWND and cursor via `GetForegroundWindow` and
  `GetCursorPos`.
- Input desktop via `OpenInputDesktop` and `GetUserObjectInformation`.
- Hidden worker child process report files written after separate readback from
  the child process.
- Process table for exact probe, child worker, and launched Notepad PIDs.

Before read:

- Foreground: `0x9e08b0`
- Cursor: `(9406,905)`
- Input desktop: `Default`
- Pre-existing Notepad PIDs: `31048`, `55716`

Trigger:

- Created hidden desktops `SynapseIssue741Single`,
  `SynapseIssue741Left`, `SynapseIssue741Right`, and
  `SynapseIssue741Notepad`.
- Launched child workers with `STARTUPINFO.lpDesktop`.
- Each child created an EDIT control on its assigned hidden desktop.
- Posted known text to each EDIT control and then attempted `SendInput`.
- Launched Notepad through `STARTUPINFO.lpDesktop`.
  This host's Notepad is the packaged Windows Notepad, so it was useful as a
  process/lifecycle probe but not as the deterministic text Source of Truth.
  The child EDIT controls were the text Source of Truth for input behavior.

After read:

```text
CHILD_EDIT_PARENT desktop=SynapseIssue741Single pid=39496 wait=0x0 remaining_windows=0 report=[CHILD_EDIT pid=39496 thread_desktop=SynapseIssue741Single hwnd=0x4eb0c20 before="" after_post="post-single-" sendinput_sent=0 sendinput_error=5 after_send="post-single-"]
CHILD_EDIT_PARENT desktop=SynapseIssue741Left pid=17716 wait=0x0 remaining_windows=0 report=[CHILD_EDIT pid=17716 thread_desktop=SynapseIssue741Left hwnd=0x830aaa before="" after_post="left-" sendinput_sent=0 sendinput_error=5 after_send="left-"]
CHILD_EDIT_PARENT desktop=SynapseIssue741Right pid=59568 wait=0x0 remaining_windows=0 report=[CHILD_EDIT pid=59568 thread_desktop=SynapseIssue741Right hwnd=0x314c4 before="" after_post="right-" sendinput_sent=0 sendinput_error=5 after_send="right-"]
NOTEPAD desktop=SynapseIssue741Notepad launch_success=true pid=52572 pid_window_count=0 desktop_window_count=33
AFTER foreground=0x9e08b0 cursor=(9406,905) input_desktop=Default
SUMMARY foreground_unchanged=True cursor_unchanged=True
```

Edge readback:

- Invalid desktop name `Synapse\\Invalid` failed:
  `success=False error=161`.
- `SetThreadDesktop` after a thread owned a window failed:
  `success=False error=170`.
- Fresh CLR threads also returned `ERROR_BUSY` when trying to bind with
  `SetThreadDesktop`; this confirms the daemon should not rely on in-process
  thread switching.
- Two hidden desktop child workers wrote independent values:
  `left-` and `right-`; no cross-talk occurred.
- `SendInput` from child workers on hidden desktops returned
  `sendinput_sent=0 sendinput_error=5`, and the EDIT text stayed unchanged
  after the attempted raw input.

Cleanup readback:

- The scratch workers exited with `wait=0x0`.
- The orphaned packaged Notepad child from the probe was stopped by exact PID
  after parent/command-line ownership was proven.
- Post-cleanup process SoT showed only pre-existing Notepad PIDs `31048` and
  `55716`; no `Issue741HiddenDesktopProbe.exe`, `dotnet.exe`, or
  `SynapseIssue741*` child process remained.

## Consequences

Positive:

- Hidden desktop app/window isolation is viable without stealing the operator's
  foreground or cursor.
- Process workers avoid `SetThreadDesktop` brittleness in a long-running Rust
  daemon with Tokio, COM, UIA, hooks, and possible hidden runtime windows.
- Per-worker process boundaries give exact PIDs for cleanup, crash isolation,
  job assignment, logs, and future primary-agent attribution.
- The action router can stay background-first and fail closed when raw input is
  required.

Negative:

- A plain hidden Win32 desktop is not a full raw-input sandbox. It cannot make
  DirectInput/raw-input apps work concurrently with the user's visible desktop.
- `PrintWindow` must be handled as a special hidden-desktop capture tier, not
  as a general fallback for visible desktop capture.
- Worker process lifecycle, IPC, storage rows, and job cleanup are required
  before hidden desktops can be exposed as MCP tools.

## Follow-Up Issues

- #742: add `act_launch` desktop option, worker lifecycle, process history, and
  cleanup.
- #743: route foreground-needing actions to worker-owned background tiers and
  fail closed on raw-input-required cases.
- #744: add explicit hidden-desktop capture/readback path with bounded
  `PrintWindow` and per-worker UIA attach.
- #745: build read-only PiP viewer without granting foreground/input control.
- #746: document the separate Windows session / RDP loopback alternative for
  real raw-input concurrency.
