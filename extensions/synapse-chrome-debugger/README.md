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
`chrome.tabs` readback.

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

To apply the supported Chrome policy remediation for the discovered external
extensions, run:

```powershell
scripts\install-synapse-chrome-debugger.ps1 -ApplyExternalChromeDebuggerPolicy
```

The full Windows setup script exposes the same remediation switch:

```powershell
scripts\synapse-setup.ps1 -ApplyExternalChromeDebuggerPolicy
```

This merges `blocked_permissions=["debugger","nativeMessaging"]` into the
current user's Chrome `ExtensionSettings` policy for the exact external
extension IDs found in the Chrome profile/process SoT. If the current Windows
principal cannot write the policy key, the script fails with
`SYNAPSE_CHROME_POLICY_REMEDIATION_WRITE_FAILED` and names the registry path.
After policy is written, Chrome must reload policy or restart; the verifier
still fails closed until the profile/process SoT shows the external debugger or
native-messaging surface is gone.
