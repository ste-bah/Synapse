# Synapse Chrome Debugger Bridge

This unpacked MV3 extension lets the Synapse daemon inspect and control the
user's normal Chrome profile through Chrome Native Messaging. CDP perception and
element actions use `chrome.debugger`; background tab open/close/navigation use
non-attach `chrome.tabs` APIs so normal navigation does not show Chrome's
debugger warning UI.

Stable extension ID: `leoocgnkjnplbfdbklajepahofecgfbk`

Native host name: `com.synapse.chrome_debugger`

Install the native host registration with:

```powershell
scripts\install-synapse-chrome-debugger.ps1
```

Then load this directory as an unpacked extension from `chrome://extensions`.
The extension keeps one `runtime.connectNative()` port open and sends real CDP
commands only after the daemon asks through the local authenticated bridge.

Background tab commands (`openTab`, `closeTab`, and `navigateTab`) use
`chrome.tabs.create`, `chrome.tabs.remove`, `chrome.tabs.update`,
`chrome.tabs.reload`, `chrome.tabs.goBack`, and `chrome.tabs.goForward`. They do
not call `chrome.debugger.attach`.

Attach-capable commands (`snapshot`, `clickNode`, `typeNode`, and `nodeValue`)
are fail-closed unless the target Chrome process was launched with
`--silent-debugger-extension-api`. Without that switch, Chrome intentionally
shows its "`started debugging this browser`" warning UI when an extension calls
`chrome.debugger.attach`. Synapse checks the target window owner PID and process
command line before attach; if the switch is absent or unreadable, Synapse
returns `A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED` and does not call
`chrome.debugger.attach`.
