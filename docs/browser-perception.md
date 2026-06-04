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
observed. The UIA tree is still returned, but it is the browser shell, not the
page DOM.

## Diagnostics

Browser observations carry diagnostics so agents can tell the difference between
"there is no button" and "the browser page was not reachable through CDP."

- `diagnostics.cdp.status = "ok"`: a CDP endpoint is reachable. After the DOM
  attach succeeds, `diagnostics.web_path = "cdp"` and page nodes should appear in
  `elements`.
- `diagnostics.cdp.status = "unreachable"` with
  `reason_code = "A11Y_CDP_UNREACHABLE"`: the foreground process is Chromium
  family, but no probed debug port accepted a connection. Synapse then attempts
  OCR over tiled browser content. If readable text is found,
  `diagnostics.web_path = "ocr"` and OCR text nodes appear in `elements`; if OCR
  has no usable text or capture is unavailable, `web_path` remains `uia_only`.
- CDP attach errors use the same diagnostics object with a non-`ok` status and
  detail. Synapse may still recover readable text through OCR, but the CDP
  failure remains visible in diagnostics.
- Non-browser foreground windows leave `diagnostics.cdp` and `web_path` unset.

The intended strategy ladder is:

1. CDP DOM and accessibility tree for Chromium page content.
2. OCR/capture over tiled browser content when CDP is down or attach fails.
3. Explicit `uia_only` for browser chrome/native UI when neither DOM nor OCR
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

By default the automation profile is temporary and per launch. Set
`SYNAPSE_CDP_USER_DATA_DIR` when a stable automation profile is needed, for
example to keep a login session across runs. Do not point it at the user's
primary browser profile.

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

For browser work, prefer this loop:

1. Call `act_launch` with a Chromium-family target and the page URL as an arg.
2. Read the process/window state and the returned `cdp_debug_port` /
   `cdp_endpoint`.
3. Call `observe` with `include = ["focused", "elements", "diagnostics"]`.
4. Require `diagnostics.cdp.status = "ok"` and `diagnostics.web_path = "cdp"`
   before assuming page DOM nodes are present.
5. Call `find` with a role/name query, such as `role = "button"` and
   `name_substring = "Apply"`.
6. Use the returned CDP-backed `element_id` with `act_click`, `act_type`, or
   `act_stroke`.
7. Read the separate source of truth after the action: page text/DOM state,
   visible UI state, downloaded file bytes, server-side record, or the Synapse
   audit row that should have changed.

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

The final `observe` is not the verdict by itself. Full State Verification still
requires a separate read of the real outcome produced by the web action.
