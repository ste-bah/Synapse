# Synapse MCP Tool-Surface Audit — issue #1348

**Goal:** shrink the MCP (currently **245 tools**, `crates/synapse-mcp/tests/snapshots/m4_tools_list__m4_tools_list.snap`) while raising real capability, by (1) consolidating where N tools do one job, and (2) deleting/rewiring tools and internal tiers that don't actually change real state.

Every claim cites `file:line` under the repo root. This document is the analysis deliverable for the epic; the consolidations/removals are tracked as child work in the issue.

> Audit method: enumerated every `#[tool(...)]` across `crates/synapse-mcp/src/server/*.rs`, cross-checked the pinned tool list (`m4_tools_list` snapshot) and `docs/multi-agent-capability-matrix.md`, and read the bridge engine (`extensions/synapse-chrome-debugger/service_worker.js`) for the synthetic-vs-real input paths.

---

## 0. Headline findings (load-bearing)

1. **Web `click`/`dblclick`/`press` is synthetic-`dispatchEvent`-only, with no activation postcondition.** A `target_act` click on a *DOM locator* (selector/role/name) routes to `target_act_browser_dom_action(…, "click", …)` (`background_router.rs:608`, `:633`), running in-page `performClick` (`service_worker.js:17161`) firing forged `PointerEvent`/`MouseEvent` (`:17349`, `:17361`) — `isTrusted=false`, so default actions / isTrusted-guarded handlers ignore them, yet the function returns `ok` with only a before/after summary (`:16613`). A **real** CDP mouse path (`cdpInput` → `Input.dispatchMouseEvent`) already exists and is used by `tap`/`hover`/`drag` — but `click` never routes there.
2. **`verb=press` with no key chord silently becomes a synthetic mouse click** (`background_router.rs:711-716` → DOM `"press"` → `performClick`).
3. **`typeActiveElement` uses the plain `.value=` setter + synchronous readback** (`service_worker.js:15555`, `:15467`). Controlled React/Vue/Angular inputs revert this on next render; the sync readback passes before the async revert → false success. The correct native-setter path already exists in `setFieldValue` (`:16002`, comment `:15648`).
4. **Test/FSV scaffolding shipped as live tools:** `storage_put_probe_rows` (`m3_tools.rs:2111`), `storage_pressure_sample` (`m3_tools.rs:2153`), `action_diagnostic_rate_limit_override` (`m2_tools.rs:1644`), `action_diagnostic_queue_full_setup` (`m2_tools.rs:1726`).
5. **EverQuest pack (25 tools) is already correctly gated** behind `SYNAPSE_ENABLE_EVERQUEST` (`server.rs:652-657`) — 0 tools on the default surface. The model to emulate.

---

## 1. Text / keyboard input
Tools: `act_type`, `act_set_value`, `act_set_field_text`, `act_keymap`, `act_press`, `act_combo`, `act_clipboard`, `browser_set_value`, `browser_fill_form`, + router verbs `insert_text/append_text/set_field/type/key/press/set_selection/clear`.

5+ tools all "put text into a field", differing only by lane (web/native/UIA) and replace-vs-append. `act_set_field_text` already unifies web+native+UIA replace; `browser_set_value` is the web subset; `act_set_value` the native subset.

**Canonical:** keep `target_act` (`set_field`/`insert_text`/`append_text`) backed by `act_set_field_text` as the single internal text primitive; demote `act_set_value`/`browser_set_value` to internal tiers; `browser_fill_form` = the multi-field batch; `act_type` keeps only the no-element foreground-typing role.

**Broken/dead:** `typeActiveElement` plain-setter revert (`service_worker.js:15555`); dead duplicate helpers `applyTextToEditable` (`:15597`) and `dispatchSyntheticInputEvent` (`:15633`) — no callers, delete.

**Good (keep):** the PostMessage keyboard tier was hardened to fail loud (#1331/#1332, `m2/press/postmessage.rs`). Residual: the generic `WM_KEYDOWN(+WM_CHAR)` else-branch (`postmessage.rs:89-100`) can still silently no-op for apps reading `GetKeyState` — give it the same fail-loud guard.

## 2. DOM events
`target_act verb=dispatch_event` (honest escape hatch — reports `default_allowed`), `performDomActionInPage` (`service_worker.js:16414`). `performDomActionInPage` verifies `clear`/`check`/`focus`/`select_text`/`select` (throws POSTCONDITION_FAILED) but NOT `click`/`press`/`dblclick` (`:16613`). **Forbid synthetic events as the implementation of `click`/`press`/`type`.**

## 3. Clicking / pointer
Two click lanes diverge by element-id shape: native/UIA → real `act_click` (`background_router.rs:612-631`); web DOM locator → synthetic `performClick` (`:608/:633`). Bring web lane up to real input via `cdpInput`. `browser_drag` (real CDP) vs `browser_drop` (synthetic default) → fold into one drag tool, `mode=mouse|synthetic`, mouse default. **Dead:** `#[allow(dead_code)] act_click_with_handle` (`click.rs:103`).

## 4. Browser navigation / tabs
`browser_tabs` already wraps `cdp_open_tab`/`cdp_close_tab` → demote those to internal; fold `browser_adopt_active_tab` as `operation=adopt`. `cdp_navigate_tab` canonical; keep `cdp_activate_tab`/`cdp_target_info`/`cdp_bridge_reload`.

## 5. Screenshot / perception
`capture_screenshot` (CDP/WGC) and `browser_screenshot` (bridge tiles) overlap on "screenshot a tab" via mutually-exclusive lanes → unify behind one `screenshot` with a `lane` param. `find` ≈ `observe` filtered → fold. Keep `read_text` (OCR), `browser_content` (HTML), `capture_gif`.

## 6. Waiting (strongest single consolidation)
7 tools — `browser_wait_for`, `_function`, `_load_state`, `_url`, `_request`, `_response`, `_selector` (`m1_tools.rs:2688-3109`) — share one poll→satisfied/`BROWSER_WAIT_TIMEOUT` pattern, differing only in predicate. **Collapse into one `browser_wait_for {condition}`.** (`browser_clock` is misfiled here — it's a fake-clock shim, keep separate.)

## 7. Network (7 → ~3)
Read side (`browser_network_requests`/`_request`/`_websockets`) read one shared CDP buffer → merge into `browser_network {mode}`. Fold HAR-replay into `browser_route`; keep `browser_network_har` for record-only. `browser_network_conditions` also reachable via `browser_emulate` → fold.

## 8. Emulation (clearest redundancy)
`browser_emulate` (`browser_emulate.rs:223`) is an explicit union that already subsumes `browser_resize`/`device`/`geolocation`/`locale`/`media` (`browser_emulation.rs:519-751`) + `browser_network_conditions`. **Keep `browser_emulate`; the 6 single-domain tools are fold/delete candidates.** (`browser_resize` viewport ≠ `target_act set_window_bounds` OS bounds — keep distinct.)

## 9. Storage / fields / content
`browser_storage` storageState already includes cookies → fold `browser_cookies` as `scope=cookies`. **Suspect:** `browser_expose_binding operation=remove` is a partial no-op (page-side fn persists) — fix or document.

## 10. Agent / fleet / mailbox
`agent_send` = single-recipient subset of `agent_send_broadcast` (keep as shortcut). `agent_cost_price_put/list/delete` → one verb tool. `agent_interrupt` vs `agent_kill` genuinely distinct (keep).

## 11. Task queue
`task_reconcile` runs implicitly inside list/next/dispatch_once → fold to a flag; keep `task_next` as the explicit dry-run preview of `task_dispatch_once`.

## 12. Timeline / hygiene / scheduler ticks
`timeline_get` ⊂ `timeline_search` → canonical search. `hygiene_flags` ⊂ `hygiene_report` → fold. **Hide scheduler seams** `intent_detect_tick`/`suggestion_tick`/`armed_routine_tick` from the default profile (keep for replay).

## 13. Session / target / lease
`set_target` / `target_claim*` / `control_lease_*` are correctly layered — keep all. **But** `set_capture_target` + `set_perception_mode` have stub descriptions and overlap `set_target` — consolidation/deletion candidates (verify not pre-`set_target` scaffolding).

## 14. Misc
`workspace_subscribe` ⊂ generic `subscribe` (workspace filter) → fold. Naming collision: session `tool_profile_*` vs learned `profile_*`/`profile_authoring_*` — rename one.

## 15. EverQuest
25 tools, gated, 0 on default surface — no action for the shrink goal; long-term extract to its own crate/feature.

---

## (a) Prioritized LOW-RISK first cuts (candidate child issues)
1. Delete/hide FSV/test-harness tools from the live surface — `storage_put_probe_rows`, `storage_pressure_sample`, `action_diagnostic_rate_limit_override`, `action_diagnostic_queue_full_setup`. (-4, zero capability loss)
2. Delete dead code — `act_click_with_handle` (`click.rs:103`), `applyTextToEditable`/`dispatchSyntheticInputEvent` (`service_worker.js`).
3. Collapse 7 `browser_wait_for_*` → one `browser_wait_for {condition}`. (-6)
4. Fold `browser_resize/device/geolocation/locale/media` + `browser_network_conditions` → `browser_emulate`. (-6)
5. Merge network reads → `browser_network {mode}`; fold HAR-replay → `browser_route`. (-3)
6. Make `browser_tabs` the sole tab tool; demote `cdp_open_tab`/`cdp_close_tab`; fold `browser_adopt_active_tab`. (-3)
7. Fold `browser_cookies`→`browser_storage`, `hygiene_flags`→`hygiene_report`, `agent_send`→`agent_send_broadcast`, `workspace_subscribe`→`subscribe`, `agent_cost_price_*`→one tool. (~-5)
8. Hide the three scheduler `*_tick` seams + `set_capture_target`/`set_perception_mode` from the default profile.
9. Fold `browser_drag`→`browser_drop {mode}` (mouse default).
10. Rename `tool_profile_*` vs `profile_*` collision.

Net of items 1-7: roughly **-30 tools** with no real capability loss (245 → ~215).

## (b) HIGH-RISK items (plan + FSV individually)
1. Rewire web `click`/`dblclick`/`press` off synthetic `performClick` onto real CDP `Input.dispatchMouseEvent` + activation postcondition (hot path, needs real-Chrome FSV across guarded sites).
2. Fix/remove `verb=press`→synthetic-mouse-click mapping.
3. Converge field-text tools into `act_set_field_text` tiers (large #882/#1000/#1299 refactor; keep per-tier fail-closed contract).
4. Fix `typeActiveElement` plain-setter revert (native setter / `Input.insertText` + deferred re-read).
5. Unify the two screenshot lanes / two click lanes behind one tool with a `lane` override.
6. Extract the EverQuest crate.

**Repo-wide invariant to enforce (issue principles 3/4):** no synthetic `dispatchEvent` may be the implementation of an input verb, and every mutating verb must assert a real-state postcondition. The codebase already does this for `clear`/`check`/`set_field`/`select`; these cuts bring `click`/`press`/`type` to the same bar.
