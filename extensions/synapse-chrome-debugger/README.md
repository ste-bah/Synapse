# Synapse Chrome Bridge

This unpacked MV3 extension lets the Synapse daemon inspect and control the
user's normal Chrome profile through a direct localhost WebSocket from the
extension service worker to the Synapse daemon. The normal end-user bridge is
tabs-first: background tab open/close/navigation use `chrome.tabs` APIs and the
extension does not require the `debugger` or `nativeMessaging` permissions.
It also does not require `chrome.alarms` or any recurring wakeup permission. If
the daemon is unavailable or the live Chrome profile is unsafe, the failure is
logged with the exact daemon error code and the bridge retries with bounded
backoff while remaining fail-closed to browser commands.

Stable extension ID: `leoocgnkjnplbfdbklajepahofecgfbk`

The service worker checks `chrome.runtime.id` against that stable ID before it
contacts the daemon. If Chrome loads the unpacked directory under any other ID,
the bridge stays dormant until the extension is reloaded correctly.

Install/verify the local bridge registration with:

```powershell
scripts\install-synapse-chrome-debugger.ps1
```

That verifier applies the same Chrome `ExtensionSettings` policy remediation as
the full setup path by default, blocking `debugger` and `nativeMessaging`
permissions so external extensions cannot surface the Chrome
"started debugging this browser" banner during Synapse background work.

Then load this directory as an unpacked extension from `chrome://extensions`.
The extension registers with the loopback daemon at `http://127.0.0.1:7700`,
then keeps an authenticated WebSocket open at `ws://127.0.0.1:7700` with a 20s
keepalive. Commands execute only after the daemon asks through the fixed
extension origin and daemon-issued bridge token. If registration, message post,
or WebSocket keepalive fails, the bridge closes the stale token, logs the code
and reconnect delay, and re-registers with bounded WebSocket reconnect. While
disconnected it performs only a low-frequency `chrome.runtime.getPlatformInfo()`
heartbeat to keep the MV3 worker available for reconnect; it does not request
`chrome.alarms`.
The normal bridge does not call `runtime.connectNative()`, so Chrome does not
create a native-host `cmd.exe` wrapper on end-user systems.
The verifier also removes stale Synapse native-host registration from every
Chrome Windows lookup hive (`HKCU`/`HKLM`, 32-bit and 64-bit views) and returns
the before/after registry readback. If any Synapse native-host key remains, the
verifier fails closed with the exact hive/path/ACL evidence instead of
certifying the host.
The verifier also reads Chrome profile permissions for the live Synapse
extension ID and fails closed if an older load still has `alarms`, `debugger`,
or `nativeMessaging` active.

Registration is also fail-closed. If the daemon sees any live Chrome
profile/process Source of Truth that is not popup-free, it refuses the direct
bridge registration with `A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED` before
accepting a Chrome-hosted command channel. The service worker treats that exact
error as an unsafe-profile condition, logs it with
`A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED`, and retries with the same bounded
backoff. This keeps the failure visible while preventing any browser command
from queueing on an unsafe end-user Chrome profile.

Background tab commands (`openTab`, `closeTab`, and `navigateTab`) use
`chrome.tabs.create`, `chrome.tabs.remove`, `chrome.tabs.update`,
`chrome.tabs.reload`, `chrome.tabs.goBack`, and `chrome.tabs.goForward`. They do
not call `chrome.debugger.getTargets` or `chrome.debugger.attach`; target IDs
returned by this path are synthetic `chrome-tab:<tabId>` IDs backed by
`chrome.tabs` readback. The daemon refuses these normal-profile commands before
queueing them whenever the live Chrome profile/process Source of Truth still
contains any external `debugger` or `nativeMessaging` surface, because even a tab
event can wake another extension's debugger/native-host popup on an unsafe host.

Attach-capable commands (`snapshot`, `clickNode`, `typeNode`, and `nodeValue`)
are unavailable in the normal end-user install. The normal service worker
rejects them immediately and contains no `chrome.debugger` API calls, so a
stale daemon command or stale permission grant cannot surface Chrome's
"started debugging this browser" warning from the Synapse bridge. DOM attach
requires raw CDP on a dedicated Synapse-launched automation profile.

The install verifier also fails closed when the live Chrome profile contains an
active external extension with the `debugger` permission, or when Chrome has a
live external native-messaging wrapper process. Those are separate browser
surfaces that can produce the same end-user popup/window even though Synapse's
bridge is tabs-only. The verifier names the extension ID, profile, and process
SoT so the host can remove the external surface or apply a Chrome
`ExtensionSettings.blocked_permissions` policy before treating the system as
popup-free.

Runtime Chrome observations follow the same rule. If raw CDP is unavailable and
Synapse refuses a normal-profile attach-capable command, the diagnostic detail
includes any external Chrome `debugger` or `nativeMessaging` profile/process
surface found at that moment. A remaining end-user debugger/native-host popup is
therefore attributed to a concrete extension or process instead of being
reported as an ambiguous Synapse bridge failure. Background normal-profile tab
commands follow the same runtime guard; they are available only after the
external profile/process readback is popup-free. If registration itself is
refused, normal-profile tab commands are unavailable; use raw CDP on a dedicated
Synapse-launched automation profile until policy/profile readback is clean.

The full Windows setup script applies the supported Chrome policy remediation by
default:

```powershell
scripts\synapse-setup.ps1
```

That default setup path uses `-ChromePolicyHive Auto`: it tries HKCU first, then
HKLM, and accepts the setup only after a separate policy readback proves that
`blocked_permissions=["debugger","nativeMessaging"]` was merged into the Chrome
`ExtensionSettings` wildcard `"*"` policy entry. This blocks current and future
extensions from loading with those permissions.
Before attempting a policy write, the verifier reads HKCU/HKLM and accepts an
already-compliant policy as `existing_policy=true`; repeated setup runs should
not launch any helper when the policy Source of Truth is already correct.
If the current process cannot write either hive, setup fails closed with
per-hive ACL/readback evidence. Passing `-AutoElevateChromePolicy:$true`
explicitly permits a one-time elevated PowerShell helper that writes the same
supported policy to HKLM and returns a JSON evidence file; that opt-in can show
UAC and is not used by background end-user setup.
Passing `-ApplyExternalChromeDebuggerPolicy:$false` is diagnostic-only and cannot
certify an end-user host as popup-free.

The standalone bridge verifier applies the same supported Chrome policy
remediation by default:

```powershell
scripts\install-synapse-chrome-debugger.ps1
```

Use `-ChromePolicyBlockScope DetectedExtensions` only when the operator
intentionally wants to limit remediation to the currently discovered extension
IDs. If no allowed hive can persist the policy, the script fails with
`SYNAPSE_CHROME_POLICY_REMEDIATION_WRITE_FAILED_ALL_HIVES` and includes the
per-hive registry path, ACL/readback failure, elevated-helper evidence path when
attempted, and remediation.
After policy is written, Chrome must reload policy or restart; the verifier
still fails closed until the profile/process SoT shows the external debugger or
native-messaging surface is gone. When the registry policy is correct but the
running Chrome profile/process SoT has not consumed it yet, the verifier reports
`SYNAPSE_CHROME_POLICY_PENDING_CHROME_RELOAD` with the policy, profile, and
process readback instead of the generic external-surface error.
Passing `-ApplyExternalChromeDebuggerPolicy:$false` is diagnostic-only and
cannot certify the host as popup-free.
