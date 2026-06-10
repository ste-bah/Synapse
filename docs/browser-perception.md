# Browser and Web Perception

Synapse can see ordinary Windows UI through UI Automation, but modern browser
page content is different: Chrome, Edge, Brave, Vivaldi, Opera, and Chromium
usually expose only a shallow browser chrome tree through UIA. For page DOM
nodes, Synapse attaches to the browser's Chrome DevTools Protocol (CDP) endpoint
and reads `Accessibility.getFullAXTree` plus DOM box data.

## When CDP Attaches

`observe` and `find` probe the foreground Chromium-family process on every call.
The probe checks, in order:

1. A CDP port registered by `act_launch` for that browser process.
2. Ports listed in `SYNAPSE_CDP_PORTS`.
3. The conventional default port `9222`.

When the probe finds a reachable endpoint, the async browser path attaches CDP,
pulls the accessibility tree, resolves DOM bounds, and merges the resulting web
nodes into the normal element list. Those nodes are queryable through `find` and
actionable by `act_click`, `act_type`, and `act_stroke`.

When no endpoint is reachable, Synapse does not silently pretend the DOM was
observed. For the user's normal Chrome profile, Synapse next tries the bundled
Chrome debugger extension/native-messaging bridge. If that bridge is not
installed, connected, or allowed by Chrome, the UIA tree is still returned, but
it is the browser shell, not the page DOM.

## Diagnostics

Browser observations carry diagnostics so agents can tell the difference between
"there is no button" and "the browser page was not reachable through CDP."

- `diagnostics.cdp.status = "ok"`: a CDP endpoint is reachable. After the DOM
  attach succeeds, `diagnostics.web_path = "cdp"` and page nodes should appear in
  `elements`. `diagnostics.cdp.checked_ports` and
  `diagnostics.cdp.checked_endpoints` show the loopback probes that led to the
  reachable endpoint.
- `diagnostics.cdp.status = "A11Y_CDP_UNREACHABLE"` with matching
  `reason_code = "A11Y_CDP_UNREACHABLE"`: the foreground process is Chromium
  family, but no probed debug port accepted a connection. The diagnostics
  include the exact localhost ports/endpoints checked and a detail string that
  distinguishes raw CDP HTTP attach from Chrome's newer auto-connect permission
  flow. If the Chrome debugger extension bridge is connected, Synapse can still
  upgrade this observation to `diagnostics.cdp.status = "ok"` and
  `diagnostics.web_path = "cdp"` through `chrome.debugger`.
- `diagnostics.cdp.status = "A11Y_CDP_EXTENSION_UNAVAILABLE"` with matching
  `reason_code = "A11Y_CDP_EXTENSION_UNAVAILABLE"`: raw CDP was unreachable and
  the normal-profile Chrome extension/native host is not connected. The detail
  field names the bundled extension directory, native host name, expected
  extension ID, and registration script. Synapse then attempts OCR over tiled
  browser content. If readable text is found,
  `diagnostics.web_path = "ocr"` and OCR text nodes appear in `elements`; if OCR
  has no usable text or capture is unavailable, `web_path` remains `uia_only`.
- CDP attach errors use the same diagnostics object with
  `diagnostics.cdp.status = "A11Y_CDP_ATTACH_FAILED"`, matching
  `reason_code = "A11Y_CDP_ATTACH_FAILED"`, and a detail string. Synapse may
  still recover readable text through OCR, but the CDP failure remains visible
  in diagnostics.
- Non-browser foreground windows leave `diagnostics.cdp` and `web_path` unset.

The intended strategy ladder is:

1. Raw CDP DOM and accessibility tree for Chromium page content when a real
   loopback debug endpoint exists.
2. Chrome debugger extension/native-messaging CDP for the user's normal Chrome
   profile when raw CDP is unreachable.
3. Non-attach Chrome extension `chrome.tabs` navigation for normal-profile
   background tab open, close, navigate, reload, back, and forward.
4. OCR/capture over tiled browser content when CDP is down or attach fails.
5. Explicit `uia_only` for browser chrome/native UI when neither DOM nor OCR
   produced page content.

## Launching a Browser for DOM Access

Use Synapse `act_launch` for browser sessions that need DOM perception. For a
Chromium-family target, `act_launch` injects a remote-debugging port and a
dedicated automation profile unless the caller already supplied
`--remote-debugging-port` or `--user-data-dir`.

That dedicated profile is intentional. Chrome 136 and newer ignore
`--remote-debugging-port` on the default user profile unless a non-default
`--user-data-dir` is also supplied. A normally launched primary-profile Chrome
can therefore be impossible for Synapse to inspect through CDP.

Chrome 144 and newer also have a user-consented auto-connect flow at
`chrome://inspect/#remote-debugging`. That flow is not the same thing as a raw
`http://127.0.0.1:<port>/json/version` endpoint and cannot be discovered by
adding ports to `SYNAPSE_CDP_PORTS`. It requires a client that speaks Chrome's
auto-connect permission protocol, or a native browser bridge such as the
Synapse Chrome extension path tracked separately.

By default the automation profile is temporary and per launch. Set
`SYNAPSE_CDP_USER_DATA_DIR` when a stable automation profile is needed, for
example to keep a login session across runs. Do not point it at the user's
primary browser profile.

## Already-Running User Browsers

Prefer the already-running authenticated browser when the workflow depends on
that user's cookies, tabs, extensions, or site state. Synapse attaches to that
session when the browser exposes a reachable CDP endpoint; it does not need a
separate automation profile in that case.

Synapse cannot turn a loopback CDP socket on from outside an already-running
Chrome/Edge process that was launched without remote debugging or without the
browser's own local remote-debugging consent. For an existing authenticated
Chrome session, the supported attach path is:

1. Read `observe.diagnostics.cdp`.
2. If it is `ok`, use `diagnostics.cdp.endpoint` and require
   `diagnostics.web_path = "cdp"` after attach before assuming DOM access.
3. If it is `A11Y_CDP_UNREACHABLE`, read
   `diagnostics.cdp.checked_ports`, `checked_endpoints`, and `detail`; those are
   the actual endpoints Synapse checked.
4. If the existing browser has a known remote-debugging port not listed there,
   set `SYNAPSE_CDP_PORTS` to include that port and restart only the Synapse
   daemon, not the browser.
5. If Chrome's own UI supports enabling remote debugging for the running
   browser, treat that as the Chrome 144+ auto-connect permission flow. Do not
   expect it to make `/json/version` reachable through `SYNAPSE_CDP_PORTS`.
6. For the user's normal Chrome profile, install the bundled extension at
   `extensions/synapse-chrome-debugger` and register the native host with
   `scripts\install-synapse-chrome-debugger.ps1`. The stable extension ID is
   `leoocgnkjnplbfdbklajepahofecgfbk`; the native host name is
   `com.synapse.chrome_debugger`.
7. Use the non-attach `chrome.tabs` bridge for normal-profile Chrome tab
   navigation (`cdp_open_tab`, `cdp_close_tab`, and extension-backed
   `cdp_navigate_tab`). This path uses `chrome.tabs.create`,
   `chrome.tabs.remove`, `chrome.tabs.update`, `chrome.tabs.reload`,
   `chrome.tabs.goBack`, and `chrome.tabs.goForward`; it must not call
   `chrome.debugger.attach`.
8. Before using attach-capable Chrome debugger extension commands, read the
   target Chrome process command line and require
   `--silent-debugger-extension-api`. Chrome intentionally shows a
   "`started debugging this browser`" warning UI when an extension calls
   `chrome.debugger.attach` without that switch. Synapse therefore fails closed
   with `A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED` before attach if the switch is
   absent or unreadable.
9. If the current browser session still exposes no endpoint or extension bridge,
   fail closed with
   `web_path = "uia_only"` or `ocr`; do not claim DOM/control readback. Relaunch
   is a user/session decision because it changes the authenticated browser
   process.

Do not treat a fresh automation profile as equivalent to the user's primary
profile when cookies/session state matter. Also do not silently fall back to
coordinate-only web automation and claim DOM access; `web_path = uia_only` means
the DOM was not read.

## Optional UIA Renderer Accessibility

CDP is the preferred browser DOM path. When you specifically need the Chromium
renderer accessibility tree through UIA, opt in to
`--force-renderer-accessibility` for a Synapse-launched browser:

- Per launch: set `force_renderer_accessibility = true` on `act_launch`.
- Per host/session: set `SYNAPSE_FORCE_RENDERER_ACCESSIBILITY` to `1`, `true`,
  `yes`, or `on`.
- Per launch override: set `force_renderer_accessibility = false` to ignore the
  environment opt-in for that call.

This has a cost: Chromium builds and maintains a fuller renderer accessibility
tree. Keep it off by default and enable it only for browser sessions where the
UIA fallback matters.

## Agent Workflow

For browser work in an existing authenticated Chrome session, prefer this loop:

1. Bring the existing Chrome tab/window to foreground using ordinary user-level
   navigation; do not launch a second Chrome when session state matters.
2. Call `observe` with `include = ["focused", "elements", "diagnostics"]`.
3. Require `diagnostics.cdp.status = "ok"` and `diagnostics.web_path = "cdp"`
   before assuming page DOM nodes are present.
4. Call `find` with a role/name query, such as `role = "button"` and
   `name_substring = "Apply"`.
5. Use the returned CDP-backed `element_id` with `act_click`, `act_type`, or
   `act_stroke`. For fragile browser controls, set `act_click.verify_delta =
   true` so Synapse fails closed with `ACTION_NO_OBSERVED_DELTA` when no
   focused/UI/pixel state change is observed.
6. Read the separate source of truth after the action: page text/DOM state,
   visible UI state, downloaded file bytes, server-side record, or the Synapse
   audit row that should have changed.

For a disposable automation browser where login/session state does not matter,
`act_launch` remains the simplest way to create a dedicated CDP-enabled profile.

If `observe` reports `A11Y_CDP_UNREACHABLE` and `web_path = "ocr"`, text can be
searched but DOM-only controls still need a CDP-backed browser launch. If
`web_path = "uia_only"`, relaunch through `act_launch` or provide a real debug
port through `SYNAPSE_CDP_PORTS`. Do not keep retrying `find` against the
collapsed UIA tree and treat missing page buttons as absent.

## Recovering Truncated Observations

Large browser, Electron, and IDE trees can exceed the default element budget.
When `diagnostics.elements_truncated = true`, read
`diagnostics.elements_page`:

- `total`: element count available after the requested depth filter.
- `offset`: first element index returned in this response.
- `limit`: maximum elements requested for this response.
- `next_offset`: pass this as `element_offset` on the next `observe` call to
  fetch the next page. If it is absent, this page is the end of the current
  result set.

To expand one UIA subtree instead of paging the whole foreground tree, call
`observe` with `subtree_root = "<element_id>"` and a larger `depth`. This
re-snapshots that element as the root. Use this for native/UIA trees; CDP-backed
web nodes should generally be recovered by paging or by a targeted `find`.

Example sequence:

```text
act_launch(target="chrome.exe", args=["https://example.test/form"], wait_for_window_title_regex="Example")
observe(include=["focused","elements","diagnostics"], max_elements=200)
find(role="button", name_substring="Apply", limit=5)
act_click(target={ element_id="<cdp-backed element id from find>" })
observe(include=["focused","elements","diagnostics"], max_elements=200)
```

For UIA-only browser content, `act_click` element targets prefer semantic UIA
patterns (`InvokePattern`, then `TogglePattern`) before coordinate fallback.
Only set `use_invoke_pattern = false` when the operator explicitly wants a raw
coordinate click and the post-click foreground/readback will be verified.

The final `observe` is not the verdict by itself. Full State Verification still
requires a separate read of the real outcome produced by the web action.
