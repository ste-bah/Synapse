# Synapse Chrome Bridge

This unpacked MV3 extension lets the Synapse daemon inspect and control the
user's normal Chrome profile through a direct localhost WebSocket from the
extension service worker to the Synapse daemon. The normal end-user bridge is
tabs-first: background tab open/close/navigation use `chrome.tabs` APIs and the
extension does not require the `debugger` or `nativeMessaging` permissions.
It uses `chrome.alarms` only to wake the MV3 service worker and reconnect to a
restarted local daemon.

Stable extension ID: `leoocgnkjnplbfdbklajepahofecgfbk`

Install/verify the local bridge registration with:

```powershell
scripts\install-synapse-chrome-debugger.ps1
```

Then load this directory as an unpacked extension from `chrome://extensions`.
The extension registers with the loopback daemon at `http://127.0.0.1:7700`,
then keeps an authenticated WebSocket open at `ws://127.0.0.1:7700` with a 20s
keepalive. A 30s Chrome alarm wakes the service worker after daemon restarts and
attempts direct localhost registration again. Commands execute only after the
daemon asks through the fixed extension origin and daemon-issued bridge token.
The normal bridge does not call `runtime.connectNative()`, so Chrome does not
create a native-host `cmd.exe` wrapper on end-user systems.

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
external profile/process readback is popup-free.

The full Windows setup script applies the supported Chrome policy remediation by
default:

```powershell
scripts\synapse-setup.ps1
```

That default setup path merges `blocked_permissions=["debugger","nativeMessaging"]`
into the current user's Chrome `ExtensionSettings` wildcard `"*"` policy entry,
which blocks current and future extensions from loading with those permissions.
Passing `-ApplyExternalChromeDebuggerPolicy:$false` is diagnostic-only and cannot
certify an end-user host as popup-free.

To apply the same supported Chrome policy remediation from the standalone
bridge verifier, run:

```powershell
scripts\install-synapse-chrome-debugger.ps1 -ApplyExternalChromeDebuggerPolicy
```

Use `-ChromePolicyBlockScope DetectedExtensions` only when the operator
intentionally wants to limit remediation to the currently discovered extension
IDs. If the current Windows principal cannot write the policy key, the script
fails with
`SYNAPSE_CHROME_POLICY_REMEDIATION_WRITE_FAILED` and names the registry path.
After policy is written, Chrome must reload policy or restart; the verifier
still fails closed until the profile/process SoT shows the external debugger or
native-messaging surface is gone.
