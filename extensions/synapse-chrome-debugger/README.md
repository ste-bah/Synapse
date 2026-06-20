# Synapse Chrome Bridge

This unpacked MV3 extension lets the Synapse daemon inspect and control the
user's normal Chrome profile through a direct localhost WebSocket from the
extension service worker to the Synapse daemon. The normal end-user bridge is
tabs-first: background tab open/close/navigation use `chrome.tabs` APIs and the
extension does not require `debugger` or `nativeMessaging`. Page-scoped
evaluation uses `chrome.scripting.executeScript`; screenshot or deep CDP work
must use raw CDP from a dedicated Synapse-launched automation profile started
with `--silent-debugger-extension-api`, or fail closed before touching the
normal browser. It requires `chrome.alarms` so Chrome can wake the MV3
service worker after the daemon restarts or the worker is suspended, and
`chrome.management` so the bridge can disable external debugger/nativeMessaging
extensions that would otherwise create Chrome warning surfaces. If the daemon
is unavailable or the live Chrome profile is unsafe, the failure is
logged with the exact daemon error code and the bridge retries with bounded
backoff while remaining fail-closed to browser commands.

Stable extension ID: `leoocgnkjnplbfdbklajepahofecgfbk`

The service worker checks `chrome.runtime.id` against that stable ID before it
contacts the daemon. If Chrome loads the unpacked directory under any other ID,
the bridge stays dormant until the extension is reloaded correctly.

The bridge reports its extension version, protocol version, build ID, build
SHA-256, and command capabilities in the daemon `hello` readback. The daemon
uses that identity as the Source of Truth for extension/runtime skew. A loaded
worker that does not advertise a command needed by the daemon fails closed with
`CHROME_BRIDGE_EXTENSION_STALE` before the command is queued to Chrome.

Future bridge updates reload through the background MCP tool
`cdp_bridge_reload`. That tool asks the loaded extension to run
`chrome.runtime.reload()`, then waits for a new authenticated bridge host
registration in daemon health. It does not open `chrome://extensions`, does not
activate Chrome, and does not use coordinates. If the currently loaded worker
predates the `reloadSelf` capability, Synapse cannot make that old worker run
new code; the correct behavior is a visible stale-worker error, not foreground
automation.
If daemon health reports `synapse_chrome_bridge_profile_installation
installed=false`, Chrome has no loaded extension host to receive `reloadSelf`;
load this directory as an unpacked extension in the already-open profile before
retrying bridge health.

Install/verify the local bridge registration with:

```powershell
scripts\install-synapse-chrome-debugger.ps1
```

The verifier registers the bridge, removes stale Synapse-authored external
`debugger`/`nativeMessaging` blockers from earlier builds, then applies the
current reversible HKCU Chrome `ExtensionSettings` popup shield for external
extensions or native hosts that request `debugger`/`nativeMessaging`. It also
preserves a self-shield for the stable Synapse extension ID. The current bridge
does not request those permissions; an old loaded bridge build that still does
is blocked by Chrome policy instead of being able to show the
`started debugging this browser` banner. The shield is identified by Synapse's
`blocked_install_message` marker and can be removed with the maintenance command
below. If the HKCU Chrome policy key is ACL-locked, the verifier reports
`SYNAPSE_CHROME_POLICY_POPUP_SHIELD_WRITE_DENIED` with ACL readback. That policy
shield failure is not ignored: the loaded bridge uses `chrome.management` to
disable enabled external `debugger`/`nativeMessaging` extensions, and normal
commands fail closed if that suppression does not complete.

Then load this directory as an unpacked extension from `chrome://extensions`.
The extension registers with the loopback daemon at `http://127.0.0.1:7700`,
then keeps an authenticated WebSocket open at `ws://127.0.0.1:7700` with a 20s
keepalive. Commands execute only after the daemon asks through the fixed
extension origin and daemon-issued bridge token. If registration, message post,
or WebSocket keepalive fails, the bridge closes the stale token, logs the code
and reconnect delay, and re-registers with bounded WebSocket reconnect. While
disconnected it keeps a 30s `chrome.alarms` wake registered so a suspended MV3
worker can re-register with the daemon without foreground Chrome automation.
The normal bridge does not call `runtime.connectNative()`, so Chrome does not
create a native-host `cmd.exe` wrapper on end-user systems.
The verifier also removes stale Synapse native-host registration from every
Chrome Windows lookup hive (`HKCU`/`HKLM`, 32-bit and 64-bit views) and returns
the before/after registry readback. If any Synapse native-host key remains, the
verifier fails closed with the exact hive/path/ACL evidence instead of
certifying the host.
The verifier also reads Chrome profile permissions for the live Synapse
extension ID and fails closed if an older load still has unexpected
`debugger` or `nativeMessaging` active. Granted-only stale permissions are
reported separately because Chromium can retain removed permissions in the
granted set after an update; they are profile debt, not proof that the running
bridge can call the API.

Registration is command-surface scoped. The daemon accepts the direct bridge
registration for the expected Synapse extension so the worker can report its
`chrome.management` suppression readback. If enabled external
`debugger`/`nativeMessaging` hazards remain, normal bridge commands fail closed
with `A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED` before queueing any browser work.

Background tab commands (`openTab`, `closeTab`, `navigateTab`, `activateTab`,
`targetInfoPageText`, `pageVitals`, `domAction`, `setFieldValue`, and
`typeActiveElement`) use `chrome.windows.getAll`,
`chrome.tabs.create`, `chrome.tabs.remove`, `chrome.tabs.update`,
`chrome.tabs.reload`, `chrome.tabs.goBack`, `chrome.tabs.goForward`, and
`chrome.scripting.executeScript`. When the daemon gives the normal bridge an OS
HWND hint, the extension cannot see HWNDs directly, so it maps the hint through
the daemon's passive window bounds/title readback before using `windowId`. If
that mapping does not identify exactly one Chrome window, `openTab` and
target-scoped readback fail before accepting the tab/window pair. It never
creates a helper Chrome window and never treats the most-recently-focused Chrome
window as a substitute for the requested HWND. `capturePageScreenshot` is not a
normal-bridge capability; the daemon
refuses it before queueing any Chrome command because Chrome's debugger infobar
changes viewport/layout and breaks coordinate truth. `evaluateScript` is also
not a normal-bridge capability: arbitrary string evaluation needs raw CDP
Runtime.evaluate, while `chrome.scripting.executeScript` can safely provide only
typed, precompiled DOM helpers under normal page/extension CSP. Use
`targetInfoPageText`, `pageVitals`, `domAction`, `setFieldValue`, and
`typeActiveElement` for popup-free normal-profile read/action work, and use raw
CDP in a dedicated silent automation profile for arbitrary JavaScript eval.
`pageVitals` and `targetInfoPageText` read the page Performance Timeline for LCP
plus document visibility state. No command calls `chrome.debugger.getTargets` or
`chrome.debugger.attach`; target IDs returned by this path are synthetic
`chrome-tab:<tabId>` IDs backed by `chrome.tabs` readback. The daemon refuses
attach-capable debugger commands before queueing them. External
`debugger`/`nativeMessaging` surfaces remain visible in health/diagnostics for
operator attribution and policy shielding, and they must be suppressed by
policy or `chrome.management` before popup-free normal-profile commands run.

The lifecycle command `reloadSelf` is limited to self-reload. It validates the
expected extension ID and expected build ID, acknowledges the request to the
daemon, then schedules `chrome.runtime.reload()`. The daemon accepts the reload
only after a separate post-reconnect host readback reports the expected build
and the full required capability set.

Attach-capable DOM commands (`snapshot`, `clickNode`, `typeNode`, and
`nodeValue`) are unavailable in the normal end-user install. The normal service
worker rejects them immediately. The bridge contains no `chrome.debugger` use;
DOM attach and debugger-backed screenshots require raw CDP on a dedicated
Synapse-launched automation profile.

The install verifier also reads whether the live Chrome profile contains an
external extension or native-messaging wrapper process with `debugger` or
`nativeMessaging`. Those are separate browser surfaces that can produce an
end-user popup/window even though Synapse's bridge uses only tabs plus
scripting. By default, setup writes a Synapse-marked HKCU `ExtensionSettings`
`blocked_permissions` shield for those hazards. Use
`-PreserveExternalDebuggerExtensions` only as an explicit emergency opt-out;
deep CDP work still belongs in a dedicated Synapse-launched automation profile
started with `--silent-debugger-extension-api`. If the policy key is ACL-locked,
setup reports the denied write with ACL evidence instead of assuming the shield
exists. Granted-only stale Synapse rows remain advisory when the loaded bridge is
current and `debuggerApiAvailable=false`; active/manifest self hazards still fail
closed. External hazards rely on the loaded bridge's `chrome.management`
suppression readback. If Chrome rejects that suppression, normal-profile commands
fail closed with exact extension IDs.

Runtime Chrome observations follow the same rule. If raw CDP is unavailable and
Synapse refuses a normal-profile attach-capable command, the diagnostic detail
includes any external Chrome `debugger` or `nativeMessaging` profile/process
surface found at that moment. Health/setup also report visible automation Chrome
processes whose flags are known to show layout-shifting browser banners, such as
headed Playwright MCP launches with `--disable-blink-features=AutomationControlled`
or remote debugging without `--silent-debugger-extension-api`. A remaining
end-user debugger/native-host/banner popup is therefore attributed to a concrete
extension or process instead of being reported as an ambiguous Synapse bridge
failure. Background normal-profile tab and typed DOM commands require those
warnings to be cleared or suppressed before they run. Use raw CDP on a dedicated
Synapse-launched automation profile started with
`--silent-debugger-extension-api` only for attach-capable CDP work.

The full Windows setup script runs the same verifier and applies the same
reversible HKCU popup shield by default:

```powershell
scripts\synapse-setup.ps1
```

Setup registers the bridge, removes stale Synapse-authored external blockers
from prior builds, preserves the Synapse extension ID self-shield, and tries to
write current Synapse-authored `blocked_permissions` entries for detected
external debugger/nativeMessaging hazards. Those entries are reversible through
the maintenance command below and are the supported first way to suppress
popups from other extensions/native hosts; the runtime `chrome.management`
fallback is the second way, and fail-closed is the only remaining behavior when
both are unavailable.

To heal an affected machine without a full setup run, invoke the standalone
verifier's maintenance entry point:

```powershell
scripts\install-synapse-chrome-debugger.ps1 -RemoveExternalDebuggerPolicyOnly
```

That removes only Synapse-authored external blockers (matched by the
`blocked_install_message` marker) from HKCU and HKLM, preserves the Synapse
extension ID self-shield, and reports a per-hive result; admin- or user-authored
`ExtensionSettings` entries are preserved.
Popup-free background automation is achieved on Synapse's own side: the bundled
bridge is tabs-first over localhost WebSocket with no `debugger` or
`nativeMessaging` permission, helper Chrome windows are never created, and
deeper DOM/action CDP runs in a dedicated Synapse-launched automation profile
started with `--silent-debugger-extension-api`.
