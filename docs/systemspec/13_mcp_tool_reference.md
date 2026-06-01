# 13 — MCP Tool Reference

Source files covered:
- `crates/synapse-mcp/src/server.rs`
- `crates/synapse-mcp/src/server/everquest_tools.rs`
- `crates/synapse-mcp/src/server/everquest_log.rs`
- `crates/synapse-mcp/src/server/everquest_state.rs`
- `crates/synapse-mcp/src/server/everquest_map_sensor.rs`
- `crates/synapse-mcp/src/server/everquest_memory.rs`
- `crates/synapse-mcp/src/server/everquest_outcome.rs`
- `crates/synapse-mcp/src/server/everquest_guard.rs`
- `crates/synapse-mcp/src/server/everquest_route.rs`
- `crates/synapse-mcp/src/server/everquest_domain.rs`
- `crates/synapse-mcp/src/server/everquest_trajectory.rs`
- `crates/synapse-mcp/src/server/everquest_contextgraph.rs`
- `crates/synapse-mcp/src/server/everquest_world_model.rs`
- `crates/synapse-mcp/src/server/everquest_world_model/{model,validation}.rs`
- `crates/synapse-mcp/src/server/everquest_surprise.rs`
- `crates/synapse-mcp/src/server/everquest_surprise/{model,compare,validation}.rs`
- `crates/synapse-mcp/src/server/everquest_world_summary.rs`
- `crates/synapse-mcp/src/server/everquest_world_summary/{model,validation}.rs`
- `crates/synapse-mcp/src/server/everquest_predictive_model.rs`
- `crates/synapse-mcp/src/server/everquest_scorecard.rs`
- `crates/synapse-mcp/src/server/reality.rs`
- `crates/synapse-mcp/src/m1.rs` (+ `m1/{ocr, search, sources}.rs`)
- `crates/synapse-mcp/src/m2/{aim, click, clipboard, drag, pad, press, release_all, scroll, type_text}.rs`
- `crates/synapse-mcp/src/m3/{audio, audit_export, permissions, profile, profile_authoring, profile_quality, profile_registry, reflex, replay, subscribe}.rs`
- `crates/synapse-core/src/types.rs`

All 80 live tools are registered on `SynapseService` via
`#[tool(description=...)]` in `server.rs` and routed submodules. Tool
descriptions are taken verbatim from the source. Every tool returns through
`Json<T>` so the response shape exactly matches the deserialized response
struct.

Default error response shape (all tools): `ErrorData { code: rmcp::ErrorCode(-32099), message, data: { "code": <SCREAMING_SNAKE_CASE> } }` via `crates/synapse-mcp/src/m1.rs::mcp_error`.

Manual FSV of any tool starts by proving the live runtime: read the
`synapse-mcp` process/stdout child or loopback socket, authenticate when HTTP
is used, call `health`, initialize the MCP session, and confirm the target tool
appears in `tools/list`. The behavior trigger is the real MCP `tools/call`;
afterward the agent reads the separate physical source of truth the tool should
have changed or observed. Tool returns and health responses are liveness and
attempt evidence only.

Issue #536 defines the delta-first reality architecture and #538 exposes the
live `reality_baseline`, `observe_delta`, and `reality_audit` tools. Their
contract is: baseline establishes epoch/hash/source refs, delta returns ordered
changes since a cursor, and audit re-reads physical SoTs to detect drift and
force rebase.

## 1. `health`

**Description:** "Return server health"
**Permissions:** none
**Side effects:** none

| Parameter | Type | Required | Default | Notes |
|---|---|---|---|---|
| (none) | — | — | — | uses an empty input schema (`empty_input_schema()`) |

**Returns:** `synapse_core::Health` (`{ ok, version, build, uptime_s, subsystems: BTreeMap<String, SubsystemHealth> }`). Subsystems: `storage`, `action`, `reflex`, `profiles`, `audio`, `http` (see [05_core_types_and_errors.md §5.8](05_core_types_and_errors.md)). `subsystems.action.backend_resolution` reports `source`, configured defaults, and resolved `keyboard_auto`, `mouse_auto`, `pad_auto`, and `release_all_auto`.

## 2. `observe`

**Description:** "Returns structured state of the focused window and surrounding context"
**Permissions:** none
**Side effects:** updates `M1State.last_observed_foreground` (used by `act_type`)

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `include` | `Vec<ObserveSlot>` | no | empty (→ defaults: `focused, elements, entities, hud, events`) | one of `focused`/`elements`/`entities`/`hud`/`audio`/`events`/`clipboard`/`fs`/`diagnostics` | Which slots to populate |
| `depth` | `u32` | no | `2` | `0..=6` | UIA tree depth cap |
| `max_elements` | `usize` | no | `60` | `1..=500` | Tree node cap |
| `since_event_seq` | `u64` | no | — | — | When set, `recent_events` filtered to `seq > since` |

**Returns:** `synapse_core::Observation`.
When `include` contains `audio` and the daemon was started with
`--enable-audio` / `SYNAPSE_ENABLE_AUDIO=true`, the live path initializes the
approved loopback runtime if needed and returns a bounded one-second
`AudioContext` summary: RMS, VAD flag, at most five detector events, and an
optional direction estimate. Loopback starts the lightweight detector processor
by default, so VAD/events come from live runtime state. Raw PCM and transcript
text are not persisted into observation or reality rows.
When `include` contains `clipboard`, the live path samples the system clipboard
into a redacted `ClipboardSummary` containing format names, optional text
length, and hash-only excerpt metadata. Raw clipboard text must not be persisted
by `observe`, `reality_baseline`, or `observe_delta`.
When `include` contains `fs`, the live path drains the bounded non-recursive
watcher configured by `SYNAPSE_FS_WATCH_ROOT`. `fs_recent` contains at most five
events with hashed path tokens, event kind, and optional file size; raw watched
paths must not be persisted by observation or reality rows.
**Errors:** `OBSERVE_NO_PERCEPTION_AVAILABLE` (forced via `SYNAPSE_MCP_FORCE_NO_PERCEPTION`), `OBSERVE_INTERNAL` (forced or assembler error), `A11Y_NO_FOREGROUND`, `CAPTURE_TARGET_LOST`, perception subsystem errors.

## 2a. `reality_baseline`

**Description:** "Capture or read a compact delta-first reality baseline and persist CF_KV reality rows"
**Permissions:** `READ_STORAGE`, `WRITE_STORAGE`, `READ_EVENTS`
**Side effects:** captures/reads the current perception input, persists
`CF_KV/reality/baseline/v1/<profile>/<epoch>` and
`CF_KV/reality/head/v1/<profile>`, then reads those rows back.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `profile_id` | `Option<String>` | no | observed profile or `unprofiled` | key segment | Expected profile key |
| `epoch_id` | `Option<String>` | no | generated | key segment | Operator-provided epoch id |
| `force_new_epoch` | `bool` | no | `false` | — | Capture a new baseline even when a head exists |
| `include` | `Vec<ObserveSlot>` | no | empty | see `observe` | Slots used to build the compact baseline |
| `depth` | `u32` | no | `2` | `1..=6` | Observation depth |
| `max_elements` | `usize` | no | `60` | `1..=500` | Observation element cap |

**Returns:** `RealityBaselineResponse { ok, created, profile_key,
baseline, baseline_required=false, rebase_required=false, reason, head,
readback_rows, size_bytes, size_estimate_tokens }`.

## 2b. `observe_delta`

**Description:** "Return ordered compact reality deltas since a cursor, persist changed rows, and publish reality_delta events"
**Permissions:** `READ_STORAGE`, `WRITE_STORAGE`, `READ_EVENTS`
**Side effects:** captures current reality, compares it to
`CF_KV/reality/head/v1/<profile>`, writes any
`CF_KV/reality/delta/v1/<profile>/<epoch>/<seq>` rows, updates the head row,
and publishes one `reality_delta` SSE event for each persisted delta.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `profile_id` | `Option<String>` | no | observed profile or `unprofiled` | key segment | Expected profile key |
| `since_epoch` | `Option<String>` | no | current head epoch | key segment | Cursor epoch |
| `since_seq` | `Option<u64>` | no | baseline seq | — | Return deltas after this sequence |
| `include` | `Vec<ObserveSlot>` | no | empty | see `observe` | Slots used for comparison |
| `depth` | `u32` | no | `2` | `1..=6` | Observation depth |
| `max_elements` | `usize` | no | `60` | `1..=500` | Observation element cap |
| `max_deltas` | `u32` | no | `64` | `1..=256` | Max deltas returned/written before rebase |

**Returns:** `ObserveDeltaResponse { ok, profile_key, epoch_id, from_seq,
to_seq, deltas, cursor, baseline_required, rebase_required, reason,
readback_rows, published_sse_events, size_bytes, size_estimate_tokens }`.
Missing baseline, stale epoch, profile change, and overflow return explicit
rebase guidance; future `since_seq` fails closed with `TOOL_PARAMS_INVALID`.
Generated deltas use stable field-level JSON-pointer paths within the active
epoch for foreground/focus, UIA elements, HUD values/errors, entity fields,
audio, log/runtime action outcomes, clipboard, filesystem, and diagnostics.
Delta `source_refs` are scoped to the changed physical surface instead of
repeating the whole observation ref set on every row.
High-fanout UIA element changes coalesce at eight or more affected elements:
appeared/disappeared fanout becomes one bounded `uia_structure_changed` delta,
and reused-element field fanout becomes one bounded `uia_elements_changed`
delta. Both use `/elements` and carry appeared/disappeared/changed counts, up
to 32 changed IDs per side, truncation flags, and compact hashes for the full
changed element sets.
If the coalesced batch is still larger than the compact snapshot budget,
`observe_delta` returns `delta_snapshot_budget_exceeded` rebase guidance before
writing delta rows. Low-fanout UIA changes remain individual element or field
deltas.
Because the head comparison is against the latest compact state, rapid repeated
changes coalesce into the final before/after pair persisted for that path.

## 2c. `reality_audit`

**Description:** "Re-read physical reality, compare against an assumed compact state hash, and persist drift audit rows"
**Permissions:** `READ_STORAGE`, `WRITE_STORAGE`, `READ_EVENTS`
**Side effects:** captures a fresh compact physical reality read, compares it
to the caller's `epoch_id`/`assumption_hash` or the current head row, writes
`CF_KV/reality/audit/v1/<profile>/<audit_id>`, and reads the audit/head rows
back.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `profile_id` | `Option<String>` | no | observed profile or `unprofiled` | key segment | Expected profile key |
| `epoch_id` | `Option<String>` | no | current head epoch | key segment | Assumed epoch |
| `assumption_hash` | `Option<String>` | no | current head hash | `sha256:*` | Agent's assumed compact state hash |
| `include` | `Vec<ObserveSlot>` | no | empty | see `observe` | Slots used for physical audit |
| `depth` | `u32` | no | `2` | `1..=6` | Observation depth |
| `max_elements` | `usize` | no | `60` | `1..=500` | Observation element cap |

**Returns:** `RealityAuditResponse { ok, profile_key, audit,
baseline_required, rebase_required, reason, row_key, head_key, readback_rows,
size_bytes, size_estimate_tokens }`.

## 3. `find`

**Description:** "Search visible accessibility nodes and detected entities"
**Permissions:** none
**Side effects:** none

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `query` | `Option<String>` | no | — | Free-text query |
| `role` | `Option<String>` | no | — | UIA role filter |
| `name_substring` | `Option<String>` | no | — | Name substring filter |
| `automation_id` | `Option<String>` | no | — | UIA automation id |
| `scope` | `Option<FindScope>` | no | `Both` | `Elements` / `Entities` / `Both` |
| `limit` | `Option<usize>` | no | `5` | Clamped `1..=20` |
| `in_window` | `Option<ElementId>` | no | — | Restrict scan to a window |

**Returns:** `FindResponse { results: Vec<FindResult> }` sorted by descending `score`. Each `FindResult` carries `kind: Element|Entity`, identifiers, name/role/automation_id/class_label, `bbox: Rect`, `score: f32`.

## 4. `read_text`

**Description:** "OCR text from a screen region or visible element"
**Permissions:** none
**Side effects:** runs OCR (WinRT)

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `region` | `Option<Rect>` | no | — | Screen-coord region |
| `element_id` | `Option<ElementId>` | no | — | UIA element to OCR; falls back to focused element if neither given |
| `backend` | `OcrBackend` | no | `Auto` | Schema field; currently always WinRT in live code |
| `lang_hint` | `Option<String>` | no | — | BCP-47 language tag (e.g. `en-US`) |

**Returns:** `synapse_core::OcrResult { full_text, words: Vec<OcrWord>, confidence, region, lang }`.
**Errors:** `OCR_NO_TEXT`, `OCR_BACKEND_UNAVAILABLE`, `A11Y_ELEMENT_STALE`, `CAPTURE_TARGET_LOST`.

## 5. `set_capture_target`

**Description:** "Set the active capture target"
**Permissions:** none
**Side effects:** updates `M1State.capture_config`; increments `capture_generation`

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `target` | `CaptureTargetParam` | yes | — | `Primary` \| `Monitor { monitor_index: u32 }` \| `Window { window_hwnd: i64 }` \| `ElementWindow { element_id }` |
| `min_update_interval_ms` | `Option<u64>` | no | — | Forced `>= 1` |
| `cursor_visible` | `Option<bool>` | no | — | |
| `dirty_region_only` | `Option<bool>` | no | — | |

**Returns:** `SetCaptureTargetResponse { previous: CaptureTargetWire, current: CaptureTargetWire, generation: u64, backend: String }` where `backend ∈ {"graphics_capture_api", "dxgi_duplication"}`.
**Errors:** `CAPTURE_TARGET_INVALID` (no monitor, no window, invalid element id).

## 6. `set_perception_mode`

**Description:** "Set the active perception mode"
**Permissions:** none
**Side effects:** updates `M1State.perception_mode`

| Parameter | Type | Required | Default | Valid | Description |
|---|---|---|---|---|---|
| `mode` | `String` | yes | — | `auto`/`a11y_only`/`pixel_only`/`hybrid` | Parsed via `synapse_perception::parse_perception_mode` |

**Returns:** `SetPerceptionModeResponse { previous, mode, rationale }` where `rationale ∈ {"auto_select_by_foreground_and_a11y_density", "manual_a11y_only", "manual_pixel_only", "manual_hybrid"}`.
**Errors:** `PERCEPTION_MODE_INVALID`.

## 7. `act_click`

**Description:** "Click a screen coordinate or UI Automation element"
**Permissions:** `INPUT_MOUSE` (via reflex registration paths; tool itself doesn't gate at server.rs).
**Side effects:** mouse movement + button click(s); appends to `RecordingBackend` if enabled

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `target` | `ActClickTarget` | yes | — | — | `Element { element_id }` or `Point { x: i32, y: i32 }` |
| `button` | `MouseButton` | no | `Left` | enum | |
| `clicks` | `u8` | no | `1` | `1..=3` | |
| `modifiers` | `Vec<ClickModifier>` | no | `[]` | `Ctrl`/`Shift`/`Alt`/`Super` | Non-empty → `ACTION_BACKEND_UNAVAILABLE` "act_click modifiers are not wired in the M2 click schema slice" |
| `curve` | `ClickCurve` | no | `Natural` | `Natural`/`Instant`/`Linear`/`EaseInOut` | Lowered to `AimCurve::Natural { params: FAST }` etc. |
| `duration_ms` | `u32` | no | `50` | — | Movement duration |
| `backend` | `Backend` | no | `Auto` | enum | |
| `use_invoke_pattern` | `bool` | no | `true` | — | When target is `Element` and the element supports UIA `Invoke`, the invoke pattern is used; coordinate fallback otherwise |

**Returns:** `ActClickResponse { ok: bool, used_invoke_pattern: bool, backend_used: String, double_click_window_ms: u32, inter_click_delay_ms: u32, elapsed_ms: u32 }`.
**Errors:** `TOOL_PARAMS_INVALID` (clicks out of range), `ACTION_BACKEND_UNAVAILABLE` (modifiers), `ACTION_ELEMENT_NOT_RESOLVED`, `ACTION_RATE_LIMITED`.

## 8. `act_type`

**Description:** "Type text through the active keyboard backend"
**Side effects:** keystroke synthesis (foreground check enforced)
**Pre-call check:** `SynapseService::ensure_act_type_foreground` compares `M1State.last_observed_foreground.hwnd` against `synapse_a11y::current_foreground_context().hwnd`. Mismatch → `ACTION_FOREGROUND_LOST` with a structured warn (`M2_ACT_TYPE_FOREGROUND_LOST`).

All foreground-gated action tools call the supported-use/action preflight
before accepted dispatch. For `everquest.live`, preflight verifies or restores
the configured `eqgame.exe` foreground window and stores an `action_preflight`
object as `details.preflight` in `CF_ACTION_LOG` started rows with before/after
foreground proof, including minimized/iconic state. A minimized EverQuest HWND
is restored before accepted dispatch and is not considered verified until the
post-refocus readback reports `is_minimized=false`. Denied rows that fail before
dispatch include the same object as `error.data.action_preflight` when the
preflight ran.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `text` | `String` | yes | — | — | UTF-8; surrogate pairs split via `KeystrokeEvent` lowering |
| `into_element` | `Option<ElementId>` | no | — | — | If set, the assembler is expected to have focused it first (currently advisory) |
| `dynamics` | `TypeDynamics` | no | `Natural` | `Burst`/`Linear`/`Natural` | |
| `linear_ms_per_char` | `u32` | no | `30` | `>=20` | Only used when `dynamics = Linear`; lower values fail closed with `TOOL_PARAMS_INVALID` |
| `use_scancodes` | `bool` | no | — | — | When true, keys emit with `use_scancode = true` |
| `press_enter_after` | `bool` | no | `false` | — | Appends a `KeyPress { Key::Named("enter") }` |
| `backend` | `TypeBackend` | no | `Auto` | `Software`/`Hardware`/`Auto` | |

**Returns:** `ActTypeResponse { ok, chars_typed: u32, elapsed_ms: u32, target_text_integrity: "dispatch_only_requires_target_readback", target_readback_required: true, minimum_linear_ms_per_char: 20 }`. `ok=true` means Synapse dispatched text events; target text integrity must still be verified by reading the application/file source of truth.
**Errors:** `ACTION_FOREGROUND_LOST`, `ACTION_RATE_LIMITED`, `ACTION_BACKEND_UNAVAILABLE`, `ACTION_UNSUPPORTED_KEY` (only when individual chars lower to unsupported keys), `TOOL_PARAMS_INVALID` with reason `linear_ms_per_char_below_text_integrity_minimum`.

## 9. `act_press`

**Description:** "Press a keyboard key or ordered chord"
**Side effects:** Action::KeyPress (one key) or Action::KeyChord (multiple).

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `keys` | `Vec<String>` | yes | — | `len >= 1` | Names parsed by `m2/press/keys.rs`. Single entry → `KeyPress`; multiple → `KeyChord` |
| `hold_ms` | `u32` | no | `33` | `1..=30000` | |
| `backend` | `PressBackend` | no | `Auto` | `Software`/`Hardware`/`Auto` | |

**Returns:** `ActPressResponse { ok, keys_pressed: u32, elapsed_ms: u32, backend_used: String }`.
**Errors:** `ACTION_UNSUPPORTED_KEY`, `ACTION_RATE_LIMITED`, `ACTION_BACKEND_UNAVAILABLE` (including the retired `Hardware` backend token).

## 9a. `act_keymap`

**Description:** "Press a keyboard alias from the active profile keymap"
**Side effects:** Resolves the active profile `[keymap]` alias, then emits the same keyboard action path as `act_press`.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `alias` | `String` | yes | — | non-empty after trim | Lowercased active-profile keymap alias, for example `inventory`, `target_nearest_npc`, `con`, `menu`, or `hotbar1` |
| `hold_ms` | `u32` | no | `33` | `1..=30000` | Passed to the lowered `act_press` request |
| `backend` | `PressBackend` | no | `Auto` | `Software`/`Hardware`/`Auto` | |

**Returns:** `ActKeymapResponse { ok, alias, resolved_binding, resolved_keys, hold_ms, keys_pressed, elapsed_ms, backend_used }`.
**Errors:** `PROFILE_NOT_FOUND`, `PROFILE_KEYMAP_INVALID`, `TOOL_PARAMS_INVALID`, `ACTION_UNSUPPORTED_KEY`, `ACTION_HOLD_EXCEEDED_MAX`, `ACTION_RATE_LIMITED`, `ACTION_BACKEND_UNAVAILABLE`, and supported-use foreground/policy denial errors. Action audit rows keep the requested alias plus result/error details so FSV can read the stored intent and resolved key/chord. For `everquest.live`, the started row must also carry `details.preflight.status` of `verified_foreground` or `refocused_and_verified` before an emitted action can be treated as an accepted foreground action.

## 9b. `everquest_loc_probe`

**Description:** "Send the literal EverQuest /loc command to the foreground everquest.live window and verify the appended EQ log coordinate line"
**Side effects:** Emits only the fixed `/`, `l`, `o`, `c`, `enter` keyboard sequence when `eqgame.exe` is foreground under `everquest.live` and the visible chat input state is trusted with `text_present=false`, then reads the physical EQ log tail.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| (none) | `{}` | no | `{}` | `deny_unknown_fields`; any parameter is `TOOL_PARAMS_INVALID` |

**Returns:** `EverQuestLocProbeResponse { ok, command, coordinate_order, log_path, start_offset, next_offset, file_len_bytes, bytes_read, event_count, you_say_count, location, chat_input_state, elapsed_ms }`, where `location` carries `display_y`, `display_x`, `display_z`, `log_timestamp`, and `summary`. `chat_input_state` is the pre-dispatch `everquest.chat_input_state` readback used to prove no visible unsent chat text was present before key emission.
**Errors:** `SAFETY_PROFILE_ACTION_DENIED`, `ACTION_TARGET_INVALID` with reasons such as `active_profile_mismatch`, `chat_input_state_not_safe`, `focused_text_entry_not_empty`, `active_log_unavailable`, `log_tail_failed`, `chat_pollution_detected`, or `location_log_line_absent`.

Manual FSV must read the physical EQ log byte offset, location count, and `You say` count before and after the trigger, read the visible chat input OCR crop and `UI_<character>_<server>_<class>.ini` `[MainChat]` section before key emission, then read `CF_ACTION_LOG` through `storage_inspect` or an audit readback for the started/ok or denied rows. Automated tests are only supporting evidence.

## 9b1. `everquest_chat_input_state`

**Description:** "Read the visible EverQuest chat input pollution state from the foreground window, UI layout file, and OCR crop"
**Side effects:** none. Reads the foreground EverQuest window, active character UI layout file, every `[MainChat]` coordinate mode (`Windowed`, resolution-specific, and scaled resolution-specific candidates), visible OCR proof for the selected layout, and a WinRT OCR crop of the bottom chat input strip.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| (none) | `{}` | no | `{}` | `deny_unknown_fields`; any parameter is `TOOL_PARAMS_INVALID` |

**Returns:** `EverQuestChatInputStateResponse { ok, chat_input_state }`. `chat_input_state` has row kind `everquest.chat_input_state` and compact fields: `visible`, `text_present`, `confidence`, `decision`, optional `denial_reason`, `source_region`, `source_mode`, `text_len_estimate`, `word_count`, `ocr_status`, `ocr_confidence`, foreground proof, `[MainChat]` layout proof with file SHA-256/line range, and source refs. It does not persist or return raw chat text.
**Errors:** only structural MCP/schema errors. Unsafe or untrusted chat state returns `ok=false` with `decision`/`denial_reason` so text-like tools can fail closed before emitting keys.

Manual FSV must read the physical UI layout file and visible OCR crop before calling the real MCP tool, then separately inspect the same crop/layout state. Edge reads must include visible unsent text, missing/low-confidence OCR or invisible region, and layout/foreground disagreement.

## 9c. `everquest_current_state`

**Description:** "Estimate and persist compact EverQuest current state from foreground, logs, map files, HUD, and action audit"
**Side effects:** Reads foreground/profile/HUD state, tails the active EverQuest log, reads local map files and newest EverQuest-linked action-audit rows, writes `CF_KV/everquest/current_state/v1/everquest.live`, then reads that row back before returning.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| (none) | `{}` | no | `{}` | `deny_unknown_fields`; any parameter is `TOOL_PARAMS_INVALID` |

**Returns:** `EverQuestCurrentStateResponse { ok, row_key, stored_value_len_bytes, state }`. `state` includes schema version, profile id, generation time, foreground focus, character/server, log cursor, zone and zone short name, map-order location, nearest local-map landmarks, visible level, target/consider, newest action summaries, and explicit hazards. Each inferred field is confidence-scored and carries source pointers such as EQ log path/offset, map file path, HUD field name, or action audit tail.
**Errors:** `ACTION_TARGET_INVALID` for unavailable active log state or unresolved runtime inputs, `STORAGE_WRITE_FAILED` / `STORAGE_READ_FAILED` for the durable current-state row, and `TOOL_PARAMS_INVALID` for unknown parameters.

Manual FSV must read the EQ log/config/map files and foreground state before the trigger, call the real MCP tool, then independently read `CF_KV/everquest/current_state/v1/everquest.live` through storage readback. The tool's internal row readback is supporting evidence, not the separate manual source-of-truth read required for shipping.

## 9d. `everquest_map_sensor`

**Description:** "Persist calibrated EverQuest visible-map sensor state from current-state, bounded HUD/screenshot/observe evidence, and local map files"
**Side effects:** reads the persisted current-state row by default, reads the current zone's local `maps/*.txt` file, fuses visible-map evidence from bounded `everquest.map_window_text` HUD OCR or a separately inspected observe/screenshot source, writes `CF_KV/everquest/map_sensor/v1/everquest.live/<sensor_id>`, then reads the exact row back before returning.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `sensor_id` | `String` | yes | - | Map-sensor row id |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `state_row_key` | `String` | no | `everquest/current_state/v1/everquest.live` | Current-state source row |
| `state_override` | `Option<EverQuestMapSensorStateOverride>` | no | - | Synthetic/manual state input with source refs |
| `visible_map_override` | `Option<EverQuestVisibleMapOverride>` | no | - | Optional explicit verified visible-map evidence from observe/screenshot readback |
| `expected_zone_short_name` | `Option<String>` | no | - | Optional zone consistency guard |
| `stale_after_seconds` | `u64` | no | `300` | Older current-state rows abstain |
| `max_nearest_labels` | `usize` | no | `8` | Nearest landmark cap; max `16` |

**Returns:** `EverQuestMapSensorResponse { ok, row_key, stored_value_len_bytes, sensor }`. Calibrated rows carry foreground proof, visible map bounds/confidence, compact readable map UI summary, current `/loc`, map file SHA-256/mtime/counts, nearest labels/exits, visible label or player-marker anchors, transform confidence, hazards, source refs, and evidence-boundary flags. Hidden maps, occlusion, stale current state, missing `/loc`, non-EQ foreground, zoom/pan changes, low visible confidence, or contradictory zone sources persist abstain rows instead of guessed calibration.
**Errors:** `TOOL_PARAMS_INVALID`, `ACTION_TARGET_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

Manual FSV must read the physical screenshot/observe crop, physical EQ log/current-state row, and local map file before the trigger, call the real MCP tool, then separately inspect the persisted `CF_OBSERVATIONS`, `CF_EVENTS`, and `CF_KV` map-sensor rows. Event rows include bounded HUD field-name/count metadata, not raw OCR text. The tool does not execute movement.

## 9e. `everquest_outcome_ingest`

**Description:** "Parse active or explicit EverQuest log bytes into compact redacted outcome rows and persist them in CF_KV with offset/hash readback"
**Side effects:** reads bounded bytes from the active EverQuest log, or an explicit approved `eqlog_<character>_<server>.txt` path, writes deterministic `CF_KV/everquest/outcome_event/v1/everquest.live/<offset>-<hash>` rows, then reads those rows back before returning.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `start_offset` | `Option<u64>` | no | recent bounded tail | Source byte offset |
| `max_bytes` | `usize` | no | `65536` | Bounded read size; max `524288` |
| `max_events` | `usize` | no | `64` | Bounded row count; max `256` |
| `log_path` | `Option<String>` | no | active EQ log | Explicit physical log path |
| `allow_explicit_log_path` | `bool` | no | `false` | Must be true when `log_path` is set |
| `persist_unknown` | `bool` | no | `true` | Keeps unknown/diagnostic rows instead of silently dropping them |

**Returns:** `EverQuestOutcomeIngestResponse { ok, row_prefix, source, rows_read, rows_persisted, duplicate_rows, skipped_unknown_rows, truncated_by_bytes, truncated_by_events, rows }`. Each row includes source path, byte offsets, line index in the read window, timestamp text, parsed timestamp when present, SHA-256 of the source line, compact outcome kind, confidence, diagnostic code, and redaction evidence. Raw chat bodies are never persisted.
**Errors:** `TOOL_PARAMS_INVALID`, `ACTION_TARGET_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

Manual FSV must read the physical log bytes before the trigger, call the real MCP tool, then separately inspect durable `CF_KV` rows afterward for offsets, hashes, outcome kinds, duplicate markers, and redaction flags.

## 9f. `everquest_memory_record`

**Description:** "Persist one compact EverQuest hazard or safe-area memory row with source refs, stale/conflict handling, and exact CF_KV readback"
**Side effects:** validates compact/redacted source refs, writes either `CF_KV/everquest/hazard_memory/v1/everquest.live/<memory_id>` or `CF_KV/everquest/safe_area_memory/v1/everquest.live/<memory_id>`, then reads the exact row back before returning.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `memory_id` | `String` | yes | - | ASCII id, used in the row key |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `memory_type` | `EverQuestMemoryType` | yes | - | `hazard` or `safe_area` |
| `memory_kind` | `String` | yes | - | High-risk target, death location, safe recovery area, etc. |
| `subject` | `String` | yes | - | Compact target/area/route label |
| `zone_short_name` | `Option<String>` | no | - | Zone short name such as `nektulos` or `neriaka` |
| `location` | `Option<EverQuestMemoryLocation>` | no | - | Map-order X/Y/Z coordinate |
| `radius` | `Option<f64>` | no | - | Match radius for planner consult |
| `severity` | `Option<String>` | no | type default | Hazard defaults to `high`; safe area defaults to `supportive` |
| `confidence` | `f32` | yes | - | `0.0..=1.0` |
| `evidence_relation` | `EverQuestMemoryEvidenceRelation` | no | `supports_memory` | `conflicts_with_memory` downgrades an existing row |
| `conflict_confidence_delta` | `f32` | no | `0.35` | Amount to subtract from existing confidence on conflicting evidence |
| `source_state_row_key` | `Option<String>` | no | - | Current-state or trajectory row key used as source |
| `source_state_generated_at` | `Option<DateTime<Utc>>` | no | - | Time used for stale-source detection |
| `stale_after_seconds` | `u64` | no | `3600` | Older source state disables planning use and caps confidence |
| `source_refs` | `Vec<EverQuestMemorySourceRef>` | yes at runtime | `[]` by schema | Runtime requires at least one physical SoT ref |
| `redacted_note` | `Option<String>` | no | - | Short redacted operator/agent note |

**Returns:** `EverQuestMemoryRecordResponse { ok, row_key, duplicate_of_prior_row, stored_value_len_bytes, memory }`. The memory row includes stale-source status, active-for-planning status, duplicate marker, prior confidence, conflict count, source refs, and redaction/evidence-boundary flags.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

Manual FSV must read the physical EQ log/UI/storage evidence first, call the real tool with known source refs, then separately inspect the `CF_KV` row. Closed schemas reject attempted raw chat payload fields; storage must remain unchanged for that edge.

## 9g. `everquest_memory_consult`

**Description:** "Consult persisted EverQuest hazard and safe-area memories for one candidate action, write a compact planner consult row, and read it back"
**Side effects:** reads named memory rows or scans hazard/safe prefixes, matches active rows by target/zone/location radius, writes `CF_KV/everquest/planner_consult/v1/everquest.live/<candidate_id>`, then reads that exact decision row back.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `candidate_id` | `String` | yes | - | Planner candidate id |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `candidate_kind` | `String` | yes | - | Movement, combat, rest, probe, etc. |
| `target` | `Option<String>` | no | - | Candidate target label |
| `zone_short_name` | `Option<String>` | no | - | Candidate zone |
| `location` | `Option<EverQuestMemoryLocation>` | no | - | Candidate location |
| `memory_row_keys` | `Vec<String>` | no | `[]` | Empty scans memory prefixes |
| `max_memory_rows` | `usize` | no | `128` | Prefix scan cap; max `512` |

**Returns:** `EverQuestMemoryConsultResponse { ok, row_key, stored_value_len_bytes, consult }`. The consult decision is `avoid`, `allow_with_safe_memory`, `allow_no_matching_hazard`, or `abstain_state_unknown`, with matched hazard/safe rows and match reasons.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

## 9h. `everquest_route_plan`

**Description:** "Plan and persist one bounded EverQuest route from current state to a local map landmark or zone line without executing movement"
**Side effects:** reads the persisted current-state row by default, builds the local map/zone graph from the configured EverQuest install root, writes `CF_KV/everquest/route_plan/v1/everquest.live/<plan_id>`, then reads the exact route-plan row back.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `plan_id` | `String` | yes | - | Route-plan id used in the row key |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `target_label` | `Option<String>` | no | - | Map label such as `to_Nektulos_Forest` |
| `target_zone_short_name` | `Option<String>` | no | - | Target zone short name such as `nektulos`; at least one target field is required |
| `state_row_key` | `String` | no | `everquest/current_state/v1/everquest.live` | Current-state source row to read |
| `state_override` | `Option<EverQuestRouteStateOverride>` | no | - | Synthetic/manual edge input with source refs |
| `map_calibration` | `Option<EverQuestRouteMapCalibration>` | no | - | Optional map-window calibration used to detect conflicts |
| `stale_after_seconds` | `u64` | no | `300` | Older current-state rows abstain |
| `max_waypoints` | `usize` | no | `8` | Waypoint cap; allowed `2..=32` |

**Returns:** `EverQuestRoutePlanResponse { ok, row_key, stored_value_len_bytes, plan }`. Ready plans carry current and target waypoints, map coordinates, distance, nearest labels, source map lines, confidence, guard requirements, and evidence-boundary flags. Floor-route guidance skips already reached local map-line nodes before choosing the next waypoint. Before falling back to static map labels, the tool scans compact `everquest/transition/v1/everquest.live/*` world-model rows and prefers planner-eligible verified transition volumes. Eligible rows require `verification_status="verified_zone_entry"`, matching from/to zones, complete `pre_zone_location` and `post_zone_location`, compact redaction, and confidence at least `0.70`; they appear as `verified_transition_volume` waypoints and as `plan.verified_transition`. Static zone labels without a verified row become `static_zone_label_hint` waypoints with a `static_zone_label_unverified` hazard. If the current zone already matches the requested target zone, the tool persists `abstain_already_in_target_zone` to retire stale pre-crossing plans. Unknown zone, missing `/loc`, absent target, stale state, or conflicting map calibration persist abstain rows instead of guessed movement.
**Errors:** `TOOL_PARAMS_INVALID`, `ACTION_TARGET_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

Manual FSV must read the physical map/current-state SoT before the trigger, call the real MCP tool, then separately inspect the persisted `CF_KV` route-plan row. This tool does not execute movement.

## 9i. `everquest_planner_guard`

**Description:** "Evaluate and persist one EverQuest planner guard decision before bounded foreground gameplay input"
**Side effects:** reads live foreground/profile state, visible chat-input state, and the persisted current-state row by default, writes `CF_KV/everquest/planner_guard_decision/v1/everquest.live/<decision_id>`, then reads that exact row back. It does not execute input.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `decision_id` | `String` | yes | - | Guard decision id used in the row key |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `candidate_kind` | `EverQuestPlannerCandidateKind` | yes | - | One of `loc_probe`, `inventory_read`, `map_read`, `target_consider`, `bounded_move`, `sit_rest`, or `combat_spell` |
| `candidate_label` | `Option<String>` | no | - | Human-readable bounded candidate label |
| `hotbar_alias` | `Option<String>` | no | - | Required for combat spell selection; only `hotbar4` is currently verified |
| `target_name` | `Option<String>` | no | - | Optional target name |
| `target_level` | `Option<u32>` | no | - | Target level for combat safety checks |
| `target_con_summary` | `Option<String>` | no | - | Compact consider/con summary for combat safety checks |
| `combat_readiness` | `Option<EverQuestPlannerGuardCombatReadiness>` | no | - | Explicit health, mana, standing/rest, confidence, and source-summary evidence required for `combat_spell` selection |
| `state_row_key` | `String` | no | `everquest/current_state/v1/everquest.live` | Current-state source row to read |
| `state_override` | `Option<EverQuestPlannerGuardStateOverride>` | no | - | Synthetic/manual edge input with source refs |
| `chat_input_override` | `Option<EverQuestPlannerGuardChatInputOverride>` | no | - | Synthetic/manual edge input; live FSV should use the real detector |

**Returns:** `EverQuestPlannerGuardResponse { ok, row_key, stored_value_len_bytes, decision }`. The decision row is `select` only when foreground/profile, chat-input, current-state, known-zone, no-stop-hazard, and candidate-specific guards pass. Rejections persist every failed guard and reason. `combat_spell` requires verified `hotbar4`, a known NPC target, level-1-safe target level, non-gamble con text, and explicit health/mana/rest readiness evidence.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

Manual FSV must read the physical foreground/chat/UI/log/storage SoT before the trigger, call the real MCP tool, then separately inspect the persisted `CF_KV` guard row. Any later movement/combat action still needs its own before/after physical SoT readback.

## 9j. `everquest_domain_normalize`

**Description:** "Normalize one EverQuest DynamicJEPA state/action/outcome transition, persist typed CF_KV rows, and read them back"
**Side effects:** validates one compact state/action/outcome/entity cluster plus source refs, writes the domain-pack row and typed state/action/outcome/transition rows in `CF_KV`, then reads each exact row back before returning.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `transition_id` | `String` | yes | - | ASCII id shared by typed rows |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `state` | `EverQuestDomainStateInput` | yes | - | Zone, local map coordinate, heading/level/xp/target/con/resource/UI/foreground buckets |
| `action` | `EverQuestDomainActionInput` | yes | - | Action kind, tool/alias, duration buckets, origin, foreground profile |
| `outcome` | `EverQuestDomainOutcomeInput` | yes | - | Next zone/coord buckets, target/con/log/damage/death/xp/UI deltas, surprise, zone-entry flag |
| `entity` | `EverQuestDomainEntityInput` | yes | - | Character summary, server, trajectory id, session id |
| `source_refs` | `Vec<EverQuestDomainSourceRef>` | yes at runtime | `[]` by schema | Runtime requires at least one physical SoT ref |

**Returns:** `EverQuestDomainNormalizeResponse { ok, profile_id, transition_id, validation_status, accepted_for_planning, row_keys, stored_value_len_bytes, transition }`. Row keys include `everquest/dynamicjepa_domain_pack/v1`, `everquest/dynamicjepa_state/v1`, `everquest/dynamicjepa_action/v1`, `everquest/dynamicjepa_outcome/v1`, and `everquest/dynamicjepa_transition/v1`. Accepted rows are planner-eligible; rejected rows preserve failed invariant ids; denied unsafe rows are persisted but never planner-eligible.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_WRITE_FAILED`, `STORAGE_READ_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

The persisted domain pack names explicit state/action/outcome/entity fields, enumerated planner candidates, guard names, surprise threshold, Synapse verification row prefixes, and ContextGraph-compatible DynamicJEPA CF names. Fatal invariants cover zone-entry log updates, EQ foreground for movement/combat, con-safe combat, no chat/social/economy actions, and impossible zone transitions.

Manual FSV must read physical EQ UI/log/action/storage state before the trigger, call this real MCP tool with a known action/log/observe cluster, then separately inspect the five durable `CF_KV` rows afterward. The tool is not a training script and does not replace runtime FSV for gameplay behavior.

## 9k. `everquest_trajectory_record`

**Description:** "Persist one ordered EverQuest trajectory from linked action, observation, event, log, state, and outcome evidence with JSONL provenance readback"
**Side effects:** verifies linked source rows/log byte ranges, writes `CF_KV/everquest/trajectory/v1/everquest.live/<trajectory_id>`, writes a local JSONL provenance artifact by default, then reads the persisted row back.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `trajectory_id` | `String` | yes | - | ASCII id used in the row key and JSONL file name |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `intent` | `EverQuestTrajectoryIntent` | yes | - | `navigation_probe`, `target_consider_probe`, `combat_attempt`, `recovery`, or `level_up_run` |
| `session_id` | `String` | yes | - | ASCII trajectory session id |
| `transitions` | `Vec<EverQuestTrajectoryTransitionInput>` | yes | - | 1..=32 strictly ordered transition records |
| `source_refs` | `Vec<EverQuestTrajectorySourceRef>` | yes at runtime | `[]` by schema | Top-level provenance refs; runtime requires at least one |
| `export_jsonl` | `bool` | no | `true` | Write a local JSONL provenance artifact |

Each transition requires `transition_id`, `sequence`, `occurred_at`, a `state_row_key` in `CF_KV`, at least one source row in each of `CF_ACTION_LOG`, `CF_OBSERVATIONS`, and `CF_EVENTS`, and at least one bounded EQ log ref. Optional `domain_transition_row_key`, `outcome_row_key`, `guard_row_key`, and `map_state_row_key` point to existing `CF_KV` rows.

**Returns:** `EverQuestTrajectoryRecordResponse { ok, row_key, duplicate_of_prior_row, stored_value_len_bytes, summary, trajectory }`. Rows include linked source row key hex, value lengths, compact summaries, trajectory hash, export artifact metadata, and redaction/evidence-boundary flags.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

The tool rejects missing linked rows, duplicate transition ids, non-increasing sequences, out-of-order timestamps, empty required ref lists, invalid log offsets, and log hash mismatches before writing new storage. If the trajectory row already exists, it returns the stored row with `duplicate_of_prior_row=true` without rewriting storage or the export artifact. Raw chat bodies and raw target names are not persisted.

Manual FSV must read `CF_KV`, source CF counts/samples, EQ log bytes, and the JSONL file path before the trigger, call the real MCP tool with known linked source refs, then separately inspect the persisted trajectory row and export artifact afterward.

## 9l. `everquest_episode_export`

**Description:** "Export redacted EverQuest trajectory/domain rows to ContextGraph-compatible DynamicJEPA episode JSONL with final artifact readback"
**Side effects:** reads existing `CF_KV` trajectory/current-state/DynamicJEPA state-action-outcome rows, writes a local JSONL artifact under the EverQuest ContextGraph episode export root, then reads the final file bytes back.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `export_id` | `String` | yes | - | ASCII id used for the default JSONL file name |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `trajectory_row_keys` | `Vec<String>` | yes | - | 1..=32 existing `CF_KV/everquest/trajectory/v1/...` rows |
| `issue_refs` | `Vec<String>` | no | `[]` | GitHub issue/evidence refs carried into each episode |
| `output_path` | `Option<String>` | no | `<export_id>.jsonl` | Relative path below the local episode export root |
| `overwrite` | `bool` | no | `false` | Existing output files fail closed unless explicitly replaced |

**Returns:** `EverQuestEpisodeExportResponse { ok, export_id, profile_id, output_path, line_count, bytes, sha256, episode_count, source_row_count, readback, episodes }`. Each JSONL row includes ContextGraph-compatible `source_of_truth`, `state`, `action`, `outcome`, `transition`, `expected_persisted_delta`, and `actual_readback` blocks.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

The tool refuses zero-row exports, missing trajectory/domain/current-state rows, invalid row schemas, linkage mismatches, unredacted log refs, raw chat/target redaction flags, duplicate trajectory keys, duplicate generated episode ids, absolute output paths, non-JSONL output paths, and accidental overwrite. Raw session ids are not exported; the transition entity carries only `session_id_sha256`.

Manual FSV must read the source `CF_KV` rows and output path before the trigger, call the real MCP tool, then separately inspect the source rows and final JSONL file bytes afterward. The export is not a training script and does not replace physical EQ UI/log/action FSV.

## 9m. `everquest_contextgraph_ingest` / `everquest_contextgraph_search`

**Description:** Ingest redacted EverQuest episode JSONL into ContextGraph through its real MCP stdio tool surface, and query those memories with source episode/hash provenance.
**Side effects:** `everquest_contextgraph_ingest` reads a #521 JSONL artifact, verifies the caller-supplied SHA-256, launches `context-graph-mcp --transport stdio`, calls ContextGraph `store_memory`, `get_provenance_chain`, and `get_audit_trail`, then writes `CF_KV/everquest/contextgraph_ingest/v1/everquest.live/<export_sha>/<episode_id>`. `everquest_contextgraph_search` calls ContextGraph `search_graph`, requires `source_episode_id=` and `source_export_sha256=` citations by default, then writes `CF_KV/everquest/contextgraph_search/v1/everquest.live/<search_id>`.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `ingest_id` | `String` | ingest only | - | ASCII ingest id used for session/provenance |
| `export_path` | `String` | ingest only | - | Absolute path to a #521 episode JSONL artifact |
| `expected_export_sha256` | `String` | ingest only | - | Required SHA-256 for fail-closed artifact validation |
| `search_id` | `String` | search only | - | ASCII id used in the search audit row key |
| `query` | `String` | search only | - | Search query; EverQuest tags are appended by the tool |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `contextgraph_storage_path` | `String` | yes | - | Absolute ContextGraph RocksDB path used as explicit SoT |
| `contextgraph_data_root` | `Option<String>` | no | - | Optional explicit ContextGraph data root |
| `contextgraph_command` | `String` | no | `context-graph-mcp` | Command or absolute path for the ContextGraph MCP binary |
| `no_warm` | `bool` | no | `false` | Adds `--no-warm` for protocol/debug startup; store/search may still fail if models are unavailable |
| `timeout_ms` | `u64` | no | `120000` | Per JSON-RPC request timeout |
| `importance` | `f64` | ingest only | `0.78` | ContextGraph memory importance |
| `top_k` | `u32` | search only | `8` | Search result cap, max 25 |
| `min_similarity` | `f64` | search only | `0.0` | ContextGraph search threshold |
| `require_provenance` | `bool` | search only | `true` | Reject search results that do not cite source episode/hash markers |

**Returns:** `EverQuestContextGraphIngestResponse { ok, ingest_id, profile_id, export_path, export_sha256, export_line_count, contextgraph_command, contextgraph_storage_path, stored_count, duplicate_count, rows }` and `EverQuestContextGraphSearchResponse { ok, search_id, profile_id, query, contextgraph_command, contextgraph_storage_path, result_count, citation_count, citations, contextgraph_search_readback, synapse_readback }`.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_SCHEMA_MISMATCH`, `STORAGE_CORRUPTED`, `MODEL_BACKEND_UNAVAILABLE`, `TOOL_INTERNAL_ERROR`.

The ingest tool refuses empty/unreadable exports, hash mismatch, malformed JSONL, wrong schema/profile/record kind, incompatible ContextGraph metadata, unsafe redaction flags, and private chat/session/target payload markers before any ContextGraph mutation. Stored ContextGraph content is a bounded retrieval summary, not the full episode JSON, so the real sparse embedder stays under its token ceiling. Duplicate same-hash episode rows read the existing Synapse bridge row and do not store a second ContextGraph memory. ContextGraph JSON-RPC errors and tool-level `isError=true` are fail-closed.

ContextGraph is retrieval/long-term memory only. Manual FSV must read the JSONL artifact, ContextGraph storage/audit/search SoT, and Synapse `CF_KV` bridge/search rows before and after the real MCP trigger. It does not replace physical EQ UI/log/action/storage verification for gameplay progress.

## 9n. `everquest_world_model_record`

**Description:** "Persist one compact EverQuest world-model row under an approved CF_KV prefix with exact readback"
**Side effects:** validates one compact world-model payload, writes `CF_KV` under an approved EverQuest world-model prefix, then reads the exact row back before returning.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `row_kind` | `EverQuestWorldModelKind` | yes | - | `map`, `zone_graph`, `state`, `transition`, `trajectory`, `planner`, or `surprise` |
| `row_id` | `String` | yes | - | ASCII id used in the row key |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `payload` | `serde_json::Value` | yes | - | Nonempty JSON object, compact/redacted, max 8192 bytes by default |
| `source_refs` | `Vec<EverQuestWorldModelSourceRef>` | yes | - | 1..=32 compact source provenance refs |
| `write_mode` | `EverQuestWorldModelWriteMode` | no | `create` | `create` rejects existing keys; `replace` requires an existing key |
| `retention_class` | `EverQuestWorldModelRetentionClass` | no | `strategic` | `strategic`, `episode`, or `scratch` |
| `compact_redacted` | `bool` | no | `true` | Marks the payload as compact/redacted |
| `max_payload_bytes` | `usize` | no | `8192` | Per-call payload cap, hard maximum 32768 |

Approved key prefixes are `everquest/map/v1/everquest.live/`, `everquest/zone_graph/v1/everquest.live/`, `everquest/state/v1/everquest.live/`, `everquest/transition/v1/everquest.live/`, `everquest/trajectory/v1/everquest.live/`, `everquest/planner/v1/everquest.live/`, and `everquest/surprise/v1/everquest.live/`.

Planner-eligible learned transition volumes use `row_kind=transition` with a compact payload containing `verification_status="verified_zone_entry"`, `from_zone_short_name`, `to_zone_short_name`, optional `label`, complete `pre_zone_location` and `post_zone_location` objects with `map_x/map_y/map_z`, optional `action_cluster`, and `confidence >= 0.70`. Source refs must point at the physical EQ log zone-entry line, pre/post `/loc` evidence, and any storage/action rows used to reconstruct the crossing. Rows missing post-zone `/loc`, pointing at an unexpected destination, or below confidence remain storage evidence only and are not planner-eligible transition volumes.

**Returns:** `EverQuestWorldModelRecordResponse { ok, row_key, stored_value_len_bytes, updated_existing, row }`. The row includes schema version, timestamps, revision, previous payload hash on replace, payload hash/length, source refs, redaction flags, retention class/TTL, caps, and an evidence boundary that manual runtime FSV is still required.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_WRITE_FAILED`, `STORAGE_READ_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

The tool rejects invalid profile ids, invalid row ids, non-object or empty payloads, oversized payloads, missing source refs, malformed hashes, duplicate create writes, replace-without-existing-row, and payloads that appear to contain raw chat/message bodies.

## 9o. `everquest_world_model_inspect`

**Description:** "Inspect approved EverQuest world-model CF_KV prefixes, selected keys, counts, and redacted samples"
**Side effects:** none beyond reading `CF_KV`.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `row_kind` | `Option<EverQuestWorldModelKind>` | no | - | Restrict counts/samples to one approved kind |
| `row_key` | `Option<String>` | no | - | Selected key readback; must start with an approved prefix |
| `sample_limit` | `usize` | no | `8` | 1..=64 samples per scanned kind |
| `include_payload` | `bool` | no | `false` | Include compact payloads in samples when explicitly requested |

**Returns:** `EverQuestWorldModelInspectResponse { ok, profile_id, cf_name, counts, samples, selected }`. Counts are bounded by the scan cap; samples include row key, value length, revision, payload hash, and redaction state.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

Manual FSV for both #513 tools must read `CF_KV` before the trigger, call the real MCP tool with known synthetic world-model data, then separately read selected keys, prefix counts, and storage/WAL state afterward. These tools are storage/readback surfaces, not FSV scripts and not gameplay-progress proof.

## 9p. `everquest_surprise_detect`

**Description:** "Compare predicted EverQuest outcome with observed state/log evidence and persist a compact surprise world-model row"
**Side effects:** reads the persisted current-state row by default or a provided observed override, compares it to a compact prediction, writes `CF_KV/everquest/surprise/v1/everquest.live/<surprise_id>` through the approved world-model row path, then reads that exact row back. It does not execute input.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `surprise_id` | `String` | yes | - | ASCII id used in the surprise row key |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `prediction` | `Option<EverQuestSurprisePrediction>` | no | - | Expected zone/outcome/action plus confidence and source refs |
| `observed_state_row_key` | `String` | no | `everquest/current_state/v1/everquest.live` | Current-state row to read when no observed override is supplied |
| `observed_override` | `Option<EverQuestSurpriseObservedOverride>` | no | - | Manual/log edge input with observed zone/outcome/confidence/source refs |
| `threshold` | `f32` | no | `0.50` | Divergence and confidence threshold |
| `stale_after_seconds` | `u64` | no | `300` | Older observed state/log evidence becomes a stop/repair row |
| `source_refs` | `Vec<EverQuestWorldModelSourceRef>` | no | `[]` | Additional compact provenance refs |

**Returns:** `EverQuestSurpriseDetectResponse { ok, row_key, stored_value_len_bytes, decision, surprise_detected, stop_condition, world_model }`. Decisions include `surprise_detected`, `expected_outcome_confirmed`, `abstain_missing_prediction`, `abstain_stale_observation`, `abstain_low_confidence_observation`, and other fail-closed stop/repair states.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

Manual FSV must read physical EQ log/current-state/storage before the trigger, call this real MCP tool with known expected/observed inputs, then separately inspect `everquest_world_model_inspect`, `storage_inspect`, and DB/WAL bytes afterward. The row is repair evidence only, not gameplay progress proof.

## 9q. `everquest_world_summary`

**Description:** "Persist one compact EverQuest world-state summary for context injection with map/log/storage provenance and chat redaction"
**Side effects:** reads the persisted current-state row by default or a provided synthetic override, builds bounded map context from local EQ map files, writes `CF_KV/everquest/world_summary/v1/everquest.live/<summary_id>`, then reads that exact row back. It does not execute input.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `summary_id` | `String` | yes | - | ASCII id used in the world-summary row key |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `state_row_key` | `String` | no | `everquest/current_state/v1/everquest.live` | Current-state row to read when no state override is supplied |
| `state_override` | `Option<EverQuestWorldSummaryStateOverride>` | no | - | Synthetic/manual-FSV state with explicit source refs |
| `install_root_override` | `Option<String>` | no | - | Alternate EQ install root for map-missing and synthetic edges |
| `max_exits` | `usize` | no | `5` | Bounded nearest exits, max 16 |
| `max_landmarks` | `usize` | no | `5` | Bounded nearest landmarks, max 16 |
| `max_transitions` | `usize` | no | `5` | Bounded transition summaries, max 16 |
| `max_hazards` | `usize` | no | `5` | Bounded hazards, max 16 |
| `stale_after_seconds` | `u64` | no | `300` | Older source state becomes a blocker |
| `source_refs` | `Vec<EverQuestWorldSummarySourceRef>` | no | `[]` | Additional compact provenance refs |

**Returns:** `EverQuestWorldSummaryResponse { ok, row_key, stored_value_len_bytes, summary }`. The summary contains zone/position confidence, level, focus, nearest exits/landmarks, recent transitions, safe next probes, hazards, active blockers, source refs, compaction recovery issue links, redaction flags, and evidence-boundary flags.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

Manual FSV must read physical EQ map/log/current-state/storage before the trigger, call this real MCP tool with known expected outputs, then separately inspect `storage_inspect` and DB/WAL bytes for the exact summary key. The row is compact context evidence only and not movement, combat, or level-progress proof.

## 9r. `everquest_predictive_model_fit`

**Description:** "Fit a transparent EverQuest action-conditioned predictive baseline from verified trajectory/domain rows with exact CF_KV readback"
**Side effects:** reads #512 trajectory rows and linked #511 DynamicJEPA rows, writes `CF_KV/everquest/predictive_model/v1/everquest.live/<model_id>`, computes a stable model hash, then reads that exact row back.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `model_id` | `String` | yes | - | ASCII id used in the model row key |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `trajectory_row_keys` | `Vec<String>` | no | `[]` | Empty scans `everquest/trajectory/v1/<profile>/` |
| `max_trajectories` | `u32` | no | `64` | Scan cap; allowed `1..=128` |
| `min_transition_support` | `u32` | no | `1` | Minimum samples required for prediction use |
| `min_confidence` | `f32` | no | `0.60` | Minimum useful supervised confidence |
| `source_refs` | `Vec<EverQuestPredictiveSourceRef>` | no | `[]` | Compact provenance refs |
| `limitations` | `Vec<String>` | no | `[]` | Known model limitations |

**Returns:** `EverQuestPredictiveModelFitResponse { ok, row_key, stored_value_len_bytes, model }`. The model row records status (`trained`, `no_verified_trajectories`, or `insufficient_transition_support`), training counts, conflict counts, source trajectory/transition keys, state-action entries, action fallbacks, global fallback, confidence thresholds, model hash, and evidence-boundary flags.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

## 9s. `everquest_predictive_model_predict`

**Description:** "Persist one EverQuest predictive-model next-outcome prediction row with calibrated abstention and exact CF_KV readback"
**Side effects:** reads the model row and DynamicJEPA state row, ranks candidate actions, writes `CF_KV/everquest/prediction/v1/everquest.live/<prediction_id>`, then reads that exact row back.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `prediction_id` | `String` | yes | - | ASCII id used in the prediction row key |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `model_id` | `String` | yes | - | Model row id to read |
| `state_row_key` | `String` | yes | - | Existing DynamicJEPA state row key |
| `candidate_actions` | `Vec<EverQuestPredictiveCandidateAction>` | yes | - | Candidate actions, max 16 |
| `expected_model_hash` | `Option<String>` | no | - | Mismatch persists `abstain_stale_model_hash` |
| `min_transition_support` | `u32` | no | `1` | Minimum selected-entry sample count |
| `min_confidence` | `f32` | no | `0.60` | Below this, prediction abstains |
| `source_refs` | `Vec<EverQuestPredictiveSourceRef>` | no | `[]` | Compact provenance refs |
| `limitations` | `Vec<String>` | no | `[]` | Known prediction limits |

**Returns:** `EverQuestPredictiveModelPredictResponse { ok, row_key, stored_value_len_bytes, prediction }`. Decisions include `predict`, `abstain_stale_model_hash`, `abstain_no_verified_trajectories`, `abstain_no_candidate_actions`, `abstain_no_matching_model_entry`, `abstain_insufficient_transition_support`, and `abstain_uncertain_prediction`.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

Manual FSV for both #522 tools must read trajectory/domain/state/model/prediction `CF_KV` rows before the trigger, call the real MCP tool with known inputs, then separately inspect the durable rows afterward. The happy path must compare one prediction to a later observed outcome through the real action-prior sample surface; edges must include no data, conflicting data, stale artifact hash, and uncertainty above threshold.

## 9t. `everquest_action_prior_record`

**Description:** "Persist one EverQuest action-prior prediction/outcome sample with computed correctness and exact CF_KV readback"
**Side effects:** validates a redacted prediction/outcome sample, computes correctness, writes `CF_KV/everquest/action_prior_eval/v1/everquest.live/<sample_id>`, then reads that exact row back before returning.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `sample_id` | `String` | yes | - | ASCII id, used in the row key |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `prediction_id` | `String` | yes | - | Prediction row/model id |
| `actual_outcome_id` | `Option<String>` | no | - | Optional observed outcome id |
| `prediction` | `EverQuestActionPriorPrediction` | yes | - | Predicted next action/top-3/zone/coord/hazard/confidence/abstention |
| `actual` | `EverQuestActionPriorActual` | yes | - | Observed next action/zone/coord/hazard/surprise |
| `source_episode_ids` | `Vec<String>` | no | `[]` | Source episode ids |
| `source_refs` | `Vec<EverQuestActionPriorSourceRef>` | no | `[]` | Redacted storage/log/source pointers |
| `limitations` | `Vec<String>` | no | `[]` | Known limits for this sample |

**Returns:** `EverQuestActionPriorRecordResponse { ok, row_key, stored_value_len_bytes, sample }`. `sample.correctness.class` is one of `correct_top1`, `correct_top3`, `correct_context`, `wrong`, `abstained`, or `unknown_actual`; it also carries calibration bucket, useful flag, overconfident-wrong flag, and the evidence boundary that scorecards are not FSV.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_WRITE_FAILED`, `STORAGE_READ_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

## 9u. `everquest_action_prior_scorecard`

**Description:** "Aggregate persisted EverQuest action-prior samples into a floor-not-ceiling competence scorecard with exact CF_KV readback"
**Side effects:** reads named eval rows from `CF_KV`, writes `CF_KV/everquest/action_prior_scorecard/v1/everquest.live/<window_id>`, then reads that exact row back before returning.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `window_id` | `String` | yes | - | Scorecard window id |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `sample_ids` | `Vec<String>` | no | `[]` | Eval sample ids to aggregate |
| `min_samples` | `u32` | no | `3` | Tiny windows report insufficient evidence |
| `min_confidence_for_action` | `f32` | no | `0.60` | Below this, action forcing is counted as low-confidence |
| `competence_floor` | `f32` | no | `0.60` | Minimum useful supervised floor |
| `stretch_target` | `f32` | no | `0.80` | Stretch target; must be >= floor |
| `limitations` | `Vec<String>` | no | `[]` | Known scorecard limits |

**Returns:** `EverQuestActionPriorScorecardResponse { ok, row_key, stored_value_len_bytes, scorecard }`. The scorecard includes sample-record-time window bounds, aggregate source episode ids, top-1/top-3, zone, coord-bucket, hazard-avoidance, useful-accuracy, abstention, surprise, low-confidence-action, overconfident-wrong, and calibration-bucket metrics. It records `minimum_is_floor_not_ceiling=true`; 60-80% is the minimum semi-competence threshold, not the optimization ceiling. A non-abstaining action below `min_confidence_for_action` records `low_confidence_action_forced` and does not meet the competence floor.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

Manual FSV for both tools must read storage state before the trigger, call the real MCP tool with known synthetic prediction/outcome data, then separately inspect the durable `CF_KV` rows afterward. Scorecards support planning quality only and never replace runtime FSV against game UI/log/process/storage SoT.

## 9v. `everquest_autocombat`

**Description:** "Run a bounded, operator-attended, server-side EverQuest combat loop for the level-1 wizard (acquire -> consider -> melee + nuke-when-mana -> confirm kill -> recover -> re-acquire)"
**Side effects:** requires the active `everquest.live` profile, chat-input safety, active EQ log, and supported-use foreground preflight; emits audited `act_keymap` actions for target/consider, melee auto-attack, and bounded nuke casts; persists an `everquest_autocombat_run` row under `CF_KV/everquest/autocombat/v1/everquest.live/<run_id>`.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `max_iterations` | `u32` | no | `8` | Combat-loop iteration cap |
| `max_duration_s` | `u32` | no | `120` | Wall-clock cap |
| `hp_floor_percent` | `u32` | no | `50` | Stop when observed HP falls below this floor |
| `mana_floor_percent` | `u32` | no | `30` | Recovery/engagement floor |
| `target_level_max` | `u32` | no | `2` | Highest safe target level |
| `stop_at_level` | `u32` | no | `2` | Stop when the character reaches this level |
| `cast_mana_cost_percent` | `u32` | no | `70` | Minimum mana threshold for the configured nuke |
| `engagement_timeout_s` | `u32` | no | `30` | Per-target fight timeout |
| `hotbar_alias` | `String` | no | `hotbar4` | Active-profile keymap alias for the nuke |
| `max_roam_steps` | `u32` | no | `6` | Bounded roam/search scaffold |
| `max_chase_s` | `u32` | no | `12` | Bounded chase scaffold |
| `idempotency_key` | `Option<String>` | no | generated | Run id seed |

**Returns:** `ActAutocombatResponse { ok, iterations, kills, casts, casts_resisted, casts_fizzled, started_level, final_level, final_xp_percent, stop_reason, run_row_key, looting_note, per_iteration }`.
**Errors:** supported-use/profile/foreground/chat-input/action-preflight errors, `ACTION_TARGET_INVALID` for missing active log/runtime inputs, storage write/read errors, and `ACTION_BACKEND_UNAVAILABLE` from any failed keymap/action dispatch.

Manual FSV must read the EQ foreground/chat-input/log/HUD/action-log/storage SoTs before the trigger, call the real MCP tool, then separately inspect the EQ log offsets/outcomes, `CF_ACTION_LOG`, and the persisted autocombat run row afterward. The tool is a gameplay loop surface, not a substitute for physical level/XP/log verification.

## 10. `act_aim`

**Description:** "Move the pointer toward a screen, element, or track target"
**Side effects:** `Action::MouseMove` (or recording of same).

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `target` | `ActAimTarget` | yes | — | `Point { x, y }` \| `Element { element_id }` \| `Track { track_id }` |
| `style` | `AimStyleParam` | no | `Snap` | `Snap` / `Flick` / `Natural` / `Track` |
| `deadline_ms` | `u32` | no | `80` | Effective duration: Snap=50, Flick=35, Natural=150, anything else uses `deadline_ms` |
| `backend` | `AimBackend` | no | `Auto` | `Software`/`Hardware`/`Auto` |

**Returns:** `ActAimResponse { ok, style_used, duration_ms, backend_used, elapsed_ms }`.
**Errors:** `ACTION_BACKEND_UNAVAILABLE` (track style or element target — both return this with detail "requires the dedicated target resolution issue" / "requires the reflex runtime lands at M3"), `ACTION_RATE_LIMITED`.

## 11. `act_drag`

**Description:** "Drag between screen coordinates or element centers"
**Side effects:** `Action::MouseDrag`.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `from` | `ActDragTarget` | yes | — | `Point` or `Element` |
| `to` | `ActDragTarget` | yes | — | `Point` or `Element` |
| `button` | `DragButton` | no | `Left` | `Left`/`Right`/`Middle` |
| `curve` | `DragCurve` | no | `Natural` | `Natural`/`Instant`/`Linear`/`EaseInOut` |
| `duration_ms` | `u32` | no | `200` | |
| `backend` | `DragBackend` | no | `Auto` | `Software`/`Hardware`/`Auto` |

**Returns:** `ActDragResponse { ok, button_used, curve_used, duration_ms_used, elapsed_ms, backend_used, ... }`.
**Errors:** `ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT` (> `MAX_DRAG_DISTANCE_PX = 4096.0`), `ACTION_ELEMENT_NOT_RESOLVED`, `ACTION_RATE_LIMITED`.

## 12. `act_scroll`

**Description:** "Scroll vertically or horizontally at the current pointer or screen point"
**Side effects:** one or more `Action::MouseScroll` events.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `dy` | `i32` | no | `0` | Vertical wheel ticks |
| `dx` | `i32` | no | `0` | Horizontal wheel ticks |
| `at` | `Option<ActScrollPoint { x: i32, y: i32 }>` | no | — | Mouse position when scrolling |
| `smooth` | `bool` | no | `false` | When true, splits into events scheduled every `SMOOTH_SCROLL_INTERVAL_MS = 30 ms`, max `MAX_SMOOTH_SCROLL_STEPS = 120` |

**Returns:** `ActScrollResponse { ok, dy, dx, smooth, scrolled: bool, wheel_event_count: u32, smooth_interval_ms: u32, scheduled_smooth_total_ms: u32, backend_used: String, elapsed_ms: u32 }`. `dy=0,dx=0` is a no-op that returns `scrolled=false`.

## 13. `act_pad`

**Description:** "Apply a virtual gamepad report and optionally return it to neutral"
**Side effects:** `Action::PadReport` via ViGEm.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `pad_id` | `PadId` (u8) | no | `0` | — | ViGEm slot |
| `controller` | `ActPadController` | no | `X360` | `X360`/`Ds4` | |
| `report` | `ActPadReport` | yes | — | — | buttons + axes + triggers |
| `backend` | `PadBackend` | no | `Vigem` | `Vigem`/`Hardware` | |
| `hold_ms` | `Option<u32>` | no | — | `<= 30_000` | If set, schedules a return-to-neutral `PadReport` after the hold |

`ActPadReport`:

| Field | Type | Default | Range |
|---|---|---|---|
| `buttons` | `Vec<ActPadButton>` | `[]` | each ∈ `A`/`B`/`X`/`Y`/`Lb`/`Rb`/`Ls`/`Rs`/`Back`/`Start`/`Up`/`Down`/`Left`/`Right` |
| `thumb_l` | `(f32, f32)` | `(0.0, 0.0)` | each in `[-1.0, 1.0]` |
| `thumb_r` | `(f32, f32)` | `(0.0, 0.0)` | each in `[-1.0, 1.0]` |
| `lt` | `f32` | `0.0` | `[0.0, 1.0]` |
| `rt` | `f32` | `0.0` | `[0.0, 1.0]` |

**Returns:** `ActPadResponse { ok, pad_id, controller, buttons, backend_used, hold_ms, returned_to_neutral: bool, elapsed_ms }`.
**Errors:** `ACTION_VIGEM_NOT_INSTALLED`, `ACTION_VIGEM_PLUGIN_FAILED`, `ACTION_RATE_LIMITED`, `ACTION_HOLD_EXCEEDED_MAX`.

## 14. `act_clipboard`

**Description:** "Read, write, or clear the system clipboard"
**Side effects:** Win32 clipboard read/write/clear.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `verb` | `ActClipboardVerb` | yes | — | `Read`/`Write`/`Clear` |
| `text` | `Option<String>` | required for `Write` | — | Forbidden for `Read`/`Clear` |
| `format` | `ActClipboardFormat` | no | `Unicode` | `Text` (ASCII only) \| `Unicode` |

**Returns:** `ActClipboardResponse { ok, verb, format, written, cleared, text, text_len, elapsed_ms }`.
**Errors:** `TOOL_PARAMS_INVALID` (verb=write without text; verb!=write with text; format=text + non-ASCII).

## 15. `release_all`

**Description:** "Release all held keyboard, mouse, and gamepad input state"
**Side effects:** `Action::ReleaseAll` (KeyUp every held key, MouseButton::Up every held button, neutralize every tracked pad).

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| (none) | — | — | — | Empty params struct |

**Returns:** `ReleaseAllResponse { released_keys: u32, released_buttons: u32, neutralized_pads: u32 }`. The implementation snapshots before, executes `Action::ReleaseAll`, snapshots after, and asserts the held lists drained — `TOOL_INTERNAL_ERROR` if state remains held.

## 16. `subscribe`

**Description:** "Subscribe to filtered event notifications"
**Permissions:** `READ_EVENTS`

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `kinds` | `Vec<String>` | no | `[]` | Allow-list of `Event.kind`s. Empty → all kinds (subject to `filter`) |
| `filter` | `Option<EventFilter>` | no | — | Validated tree (depth ≤ `EVENT_FILTER_MAX_DEPTH = 8`); missing → `EventFilter::All` |
| `snapshot_first` | `bool` | no | `false` | (Reserved; ignored by the live SSE state) |
| `buffer_size` | `u32` | no | `4096` | **Must equal `4096`**; any other value → `TOOL_PARAMS_INVALID` |

**Returns:** `SubscribeResponse { subscription_id: String, started_at: DateTime<Utc> }`. The subscription id is consumed by `GET /events?subscription_id=...` over HTTP (`crates/synapse-mcp/src/http/sse.rs`).
**Errors:** `TOOL_PARAMS_INVALID`, `SUBSCRIPTION_CAP_REACHED`, `REFLEX_FILTER_INVALID`.

## 17. `subscribe_cancel`

**Description:** "Cancel an event subscription"
**Permissions:** `READ_EVENTS`

| Parameter | Type | Required | Description |
|---|---|---|---|
| `subscription_id` | `String` | yes | Trimmed; empty → `TOOL_PARAMS_INVALID` |

**Returns:** `SubscribeCancelResponse { cancelled: bool, reason: SubscribeCancelReason }` (`reason = Ok` on success).
**Errors:** `SUBSCRIPTION_NOT_FOUND`.

## 18. `reflex_register`

**Description:** "Register a reflex"
**Permissions:** `WRITE_REFLEX` plus any input permissions implied by `then` actions (`INPUT_KEYBOARD`/`INPUT_MOUSE`/`INPUT_PAD`).
**Side effects:** opens RocksDB on first call; persists a `reflex_registered` audit row; starts the scheduler thread on first reflex.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `kind` | `String` | yes | — | `aim_track` / `hold_move` / `hold_button` / `combo` / `on_event` | Reflex kind |
| `when` | `Option<ReflexWhenParam>` | for `on_event` | — | EventFilter or window-event match | |
| `then` | `ReflexThenParam` | yes | — | Either a `ReflexThen` (Action / Actions / Combo) or `{ steps: Vec<ReflexThenStep { action: String, params: Value }> }` | Action(s) to fire |
| `priority` | `u32` | no | `100` | `0..=1000` | Lower = higher priority. (`DEFAULT_REFLEX_PRIORITY` / `MAX_REFLEX_PRIORITY`) |
| `lifetime` | `ReflexLifetime` | no | `UntilCancelled` | enum | `UntilCancelled` / `OneShot` / `Duration { ms }` / `UntilEvent { filter }` / `UntilDeadline { ms }` |
| `backend` | `Backend` | no | `Auto` | enum | Default backend for the reflex's actions |
| `exclusive` | `bool` | no | `false` | — | If true, conflicts with other exclusive reflexes are resolved by priority |

**Returns:** `ReflexRegisterResponse { reflex_id: String, state: ReflexStatus }`.
**Errors:** `REFLEX_KIND_INVALID`, `REFLEX_PARAMS_INVALID`, `REFLEX_TARGET_INVALID`, `REFLEX_FILTER_INVALID`, `REFLEX_PRIORITY_INVALID`, `REFLEX_CAP_REACHED` (`MAX_SCHEDULED_REFLEXES = 32`).

## 19. `reflex_cancel`

**Description:** "Cancel a reflex"
**Permissions:** `READ_REFLEX`
**Side effects:** persists a `reflex_cancelled` audit row.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `reflex_id` | `String` | yes | Trimmed; empty → `TOOL_PARAMS_INVALID` |

**Returns:** `ReflexCancelResponse { cancelled: bool, reason: ReflexCancelReason }` (reasons: `Ok`/`NotFound`/`AlreadyExpired`).

## 20. `reflex_list`

**Description:** "List registered reflexes"
**Permissions:** `READ_REFLEX`

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `include_expired` | `bool` | no | `false` | When true, reconstructs terminal statuses from `CF_REFLEX_AUDIT` |

**Returns:** `ReflexListResponse { reflexes: Vec<ReflexStatus> }`.

## 21. `reflex_history`

**Description:** "Return persisted reflex audit history"
**Permissions:** `READ_REFLEX`

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `reflex_id` | `Option<String>` | no | — | — | If present, scans `CF_REFLEX_AUDIT` by `<reflex_id>:` prefix |
| `limit` | `u32` | no | `50` | `0..=1000` | Caps the number of audit rows returned |

**Returns:** `ReflexHistoryResponse { events: Vec<StoredReflexAudit> }` newest-first.
**Errors:** `TOOL_PARAMS_INVALID` (limit > 1000).

## 22. `profile_list`

**Description:** "List loaded profiles"
**Permissions:** `READ_PROFILE`

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `include_inactive` | `bool` | no | `true` | When false, only the active profile is returned |

**Returns:** `ProfileListResponse { profiles: Vec<ProfileStatus>, active_profile_id: Option<String> }`. Each `ProfileStatus` carries `id`, `label`, `use_scope`, `mode`, `detection_model_id: Option<String>`, `detection_classes`, `hud_fields`, `keymap_actions`, `backends`, `event_extensions`, `matches: Vec<ProfileMatchStatus>`, `metadata: BTreeMap<String, String>`, `active: bool`, `schema_version: u32`.

## 23. `profile_activate`

**Description:** "Activate a loaded profile by id"
**Permissions:** `WRITE_PROFILE_ACTIVE`
**Side effects:** updates `ProfileRuntime` active state in memory (no FS write, no `CF_PROFILES` write in current build).

| Parameter | Type | Required | Description |
|---|---|---|---|
| `profile_id` | `String` | yes | Must match a parsed profile id |

**Returns:** `ProfileActivateResponse { profile_id, active_profile_id, previous_active_profile_id, changed: bool }`. `changed=false` if `profile_id` was already active.
**Errors:** `PROFILE_NOT_FOUND`, `SAFETY_PROFILE_ACTION_DENIED` (use_scope=Unknown without `--allow-unknown-profile`).

## 23a. `profile_authoring_generate`

**Description:** "Generate a local profile authoring candidate from replay/audit evidence"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE`
**Side effects:** reads loaded profile state, optional replay JSONL, and
`CF_ACTION_LOG`; writes and immediately reads back `CF_PROFILES` key
`profile_authoring/v1/candidate/<candidate_id>`

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `profile_id` | `String` | yes | — | loaded profile id | Profile the candidate patch targets |
| `replay_path` | `Option<String>` | no | — | under replay root | Optional replay JSONL evidence path |
| `max_audit_rows` | `u32` | no | `500` | `0..=10000` | Newest action audit rows scanned |
| `max_replay_rows` | `u32` | no | `500` | `0..=10000` | Replay rows scanned |
| `candidate_id` | `Option<String>` | no | derived | non-empty, path-safe | Optional deterministic candidate id |

**Returns:** `ProfileAuthoringGenerateResponse { cf_name, row_key,
wrote_row, active_profile_id, candidate, summary }`. The candidate row stores
source audit/replay counts and row ids, an evidence hash, evidence summary,
expected improvement strings, a declarative patch, and a safety review.
Candidates are stored separately from active profiles.

**Errors:** `PROFILE_NOT_FOUND`, `PROFILE_AUTHORING_INSUFFICIENT_EVIDENCE`,
`PROFILE_AUTHORING_CONFLICTING_EVIDENCE`,
`PROFILE_AUTHORING_UNSAFE_ESCALATION`, `TOOL_PARAMS_INVALID`,
`STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `TOOL_INTERNAL_ERROR`.

## 23b. `profile_authoring_list`

**Description:** "List local profile authoring candidates"
**Permissions:** `READ_STORAGE`
**Side effects:** none; scans `CF_PROFILES` keys under
`profile_authoring/v1/candidate/`

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `profile_id` | `Option<String>` | no | — | loaded or stored profile id | Optional profile filter |
| `state` | `Option<String>` | no | — | non-empty | Optional state filter (`candidate`, `accepted`, `rejected`) |
| `limit` | `u32` | no | `100` | `1..=1000` | Maximum returned summaries |

**Returns:** `ProfileAuthoringListResponse { cf_name, prefix, profile_id,
state, limit, total_matched, candidates }`.

## 23c. `profile_authoring_inspect`

**Description:** "Inspect a local profile authoring candidate"
**Permissions:** `READ_STORAGE`
**Side effects:** none; reads one `CF_PROFILES` candidate row

| Parameter | Type | Required | Description |
|---|---|---|---|
| `candidate_id` | `String` | yes | Candidate id under `profile_authoring/v1/candidate/` |

**Returns:** `ProfileAuthoringInspectResponse { cf_name, row_key, found,
candidate, summary }`. Missing candidates return `found=false`.

## 23d. `profile_authoring_accept`

**Description:** "Accept a local profile authoring candidate without activating it"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE`
**Side effects:** rewrites one `CF_PROFILES` candidate row with
`state="accepted"` and reads it back. It does not mutate bundled profile TOML or
activate the profile.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `candidate_id` | `String` | yes | — | Candidate id to accept |
| `operator_note` | `Option<String>` | no | — | Optional local note stored on the row |

**Returns:** `ProfileAuthoringAcceptResponse { cf_name, row_key,
candidate_id, profile_id, previous_state, state, wrote_row, activated,
active_profile_id, candidate }` with `activated=false`.
**Errors:** `PROFILE_AUTHORING_CANDIDATE_NOT_FOUND`,
`PROFILE_AUTHORING_INVALID_STATE`, storage read/write errors.

## 23e. `profile_authoring_reject`

**Description:** "Reject a local profile authoring candidate"
**Permissions:** `READ_STORAGE`, `WRITE_STORAGE`
**Side effects:** rewrites one `CF_PROFILES` candidate row with
`state="rejected"` and reads it back.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `candidate_id` | `String` | yes | — | Candidate id to reject |
| `reason` | `Option<String>` | no | — | Optional local rejection reason |

**Returns:** `ProfileAuthoringRejectResponse { cf_name, row_key,
candidate_id, profile_id, previous_state, state, wrote_row, candidate }`.
**Errors:** `PROFILE_AUTHORING_CANDIDATE_NOT_FOUND`,
`PROFILE_AUTHORING_INVALID_STATE`, storage read/write errors.

## 23f. `profile_authoring_export`

**Description:** "Export a local profile authoring candidate to JSON"
**Permissions:** `READ_STORAGE`
**Side effects:** reads one `CF_PROFILES` candidate row, writes a local JSON
bundle file, then parses the written file before returning.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `candidate_id` | `String` | yes | Candidate id to export |
| `output_path` | `String` | yes | Destination JSON file path |

**Returns:** `ProfileAuthoringExportResponse { output_path, bytes_written,
cf_name, row_key, candidate_id, profile_id, state }`. The exported JSON file
contains the full candidate row plus schema/version/CF metadata.
**Errors:** `PROFILE_AUTHORING_CANDIDATE_NOT_FOUND`, `TOOL_PARAMS_INVALID`,
`STORAGE_READ_FAILED`, `TOOL_INTERNAL_ERROR`.

## 23g. `profile_quality_refresh`

**Description:** "Refresh local profile quality scoring from stored action, observation, and event rows"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE`
**Side effects:** reads `CF_ACTION_LOG`, `CF_OBSERVATIONS`, and `CF_EVENTS`;
writes and immediately reads back `CF_PROFILES` key
`profile_quality/v1/<profile_id>`

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `profile_id` | `String` | yes | — | loaded profile id | Profile whose quality snapshot should be refreshed |
| `max_audit_rows` | `u32` | no | `5000` | `1..=50000` | Newest action, observation, and event rows scanned per CF |
| `stale_after_ns` | `u64` | no | `86400000000000` | `1..=2592000000000000` | Rows older than this are counted as stale and ignored for scoring |
| `manual_fsv_evidence_ref` | `String` | no | — | non-empty, <=512 chars | Issue comment or evidence id for manual SoT readback |

**Returns:** `ProfileQualityRefreshResponse { profile_id, cf_name,
key_hex, wrote_snapshot, previous_evidence_hash, stored_value_len_bytes,
stored_value_utf8_prefix, snapshot }`. `snapshot` contains source counters,
ignored corrupt/stale rows, counts/rates, Wilson lower-bound score,
compatibility counters, profile-schema-version recency/mixed-version counters,
runtime observation/event evidence, compact event-kind/log-kind counters,
manual FSV evidence ref, redaction policy, and contribution policy.

The score-bearing sample is foreground-profile `ok` vs `error` rows only.
Denied, stale, corrupt, active-profile-only, and profile-mismatch rows are
reported as explainability/compatibility counters and do not invent success
samples. Export is always disabled; contribution requires a future explicit
operator-approved path. The snapshot keeps bounded identifiers/counts only and
must not store raw chat bodies, process paths, full window titles, private
session tickets, or raw log lines by default.

**Errors:** `PROFILE_NOT_FOUND`, `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`,
`STORAGE_WRITE_FAILED`, `TOOL_INTERNAL_ERROR`.

## 23h. `profile_registry_search`

**Description:** "Search local profile registry rows"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`
**Side effects:** none; scans `CF_PROFILES` rows whose keys start with `profile_registry/v1/`

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `query` | `Option<String>` | no | — | — | Case-insensitive key/value text filter |
| `row_kind` | `Option<String>` | no | — | non-empty when present | Filters decoded row envelope kind |
| `include_disabled` | `bool` | no | `false` | — | Includes `state=disabled` / `state=removed` rows |
| `limit` | `u32` | no | `100` | `1..=1000` | Maximum returned summaries |

**Returns:** `ProfileRegistrySearchResponse { cf_name, prefix, query,
row_kind, include_disabled, limit, total_matched, rows }`. Rows are
`ProfileRegistryRowSummary` values with UTF-8 key, key hex, row kind/id,
profile/package ids, state, update time, value length, and bounded value prefix.

## 23i. `profile_registry_inspect`

**Description:** "Inspect one local profile registry row by key or id"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`
**Side effects:** none; reads `CF_PROFILES` or `CF_KV`

| Parameter | Type | Required | Description |
|---|---|---|---|
| `row_key` | `Option<String>` | no | Exact `profile_registry/v1/*` key; `head/*` keys read `CF_KV`, others read `CF_PROFILES` |
| `source_id` | `Option<String>` | no | Builds `profile_registry/v1/source/<source_id>` |
| `package_id` + `package_version` | `Option<String>` | no | Builds package row key |
| `profile_id` + `profile_version` | `Option<String>` | no | Builds profile version row key |
| `installed_profile_id` | `Option<String>` | no | Builds installed row key |

**Returns:** `ProfileRegistryInspectResponse { cf_name, row_key, found, row }`
where `row` includes the decoded JSON value when found.
**Errors:** `TOOL_PARAMS_INVALID`, storage read/decode errors.

## 23j. `profile_registry_report`

**Description:** "Report local profile registry, quality, audit, and consent state"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`
**Side effects:** none; reads `CF_PROFILES`, `CF_KV`, and `CF_ACTION_LOG`

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `profile_id` | `Option<String>` | no | — | — | Optional profile filter |
| `limit` | `u32` | no | `100` | `1..=1000` | Maximum rows returned per report section |
| `max_audit_rows` | `u32` | no | `100` | `1..=1000` | `CF_ACTION_LOG` tail rows scanned |

**Returns:** `ProfileRegistryReportResponse` with storage path, per-CF row
counts, physical SoT pointers, registry heads, installed profiles, package
rows, curated starter targets, quarantine rows, rollback rows, profile quality
snapshots including stale-evidence counts, audit-export consent/export
readiness, recent audit bucket counts, and an explicit control list for
install/update/rollback/import/export/quality/consent/export-bundle actions.
**Errors:** `TOOL_PARAMS_INVALID`, storage read/decode errors.

## 23k. `profile_registry_install`

**Description:** "Install or update a local profile registry package manifest"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE`
**Side effects:** validates manifest/profile files; enforces signed package
trust where required; writes `CF_PROFILES` registry rows and a `CF_KV` source
head pointer; reads written rows back. Failed trust verification writes only a
`profile_package_quarantine` row in `CF_PROFILES`. Manifests with complete
`curated.*` metadata also write a `curated_profile_target` row under
`profile_registry/v1/curated_target/<seed_set_id>/<target_id>`; partial
curated metadata or a missing matching compatibility target fails closed before
companion rows are written.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `manifest_path` | `String` | yes | — | Local package manifest TOML path |
| `expected_manifest_digest` | `Option<String>` | no | — | Optional `sha256:<hex>` digest that must match manifest bytes |
| `source_id` | `String` | no | `registry.local` | Lowercase source id for source/head rows |
| `trust_policy` | `String` | no | `local_first` | `local_first` permits local unsigned packages after parser validation; `signed_required` requires a trusted Ed25519 signature |

**Returns:** `ProfileRegistryInstallResponse { operation, source_id,
package_id, package_version, profile_id, profile_version, manifest_path,
manifest_digest, profile_toml_path, wrote_rows, idempotent, trust_status,
signature_status, signer_id, trust_root_key, signature_payload_digest,
cf_profile_row_keys, cf_kv_row_keys, row_summaries }`.

Duplicate package id/version with the same manifest digest is an idempotent
no-op. Duplicate id/version with a different digest fails closed.
Signed-required packages with missing, invalid, or unknown-signer signatures
fail closed with `PROFILE_TRUST_VERIFICATION_FAILED`; package/profile/installed
rows are not activated, and the response error data carries the quarantine row
key and readback.

## 23l. `profile_registry_disable`

**Description:** "Disable or remove an installed local profile registry row"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE`
**Side effects:** updates one `CF_PROFILES` installed-profile row and reads it back

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `profile_id` | `String` | yes | — | Installed profile id |
| `state` | `String` | no | `disabled` | `disabled` or `removed` |
| `reason` | `Option<String>` | no | — | Optional local reason stored on the row |

**Returns:** `ProfileRegistryDisableResponse { profile_id, row_key,
previous_state, state, wrote_row, row }`.

## 23m. `profile_registry_export`

**Description:** "Export local profile registry rows to a JSON bundle"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`
**Side effects:** writes a local JSON bundle file; contribution mode includes
redacted audit evidence and quality summaries

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `output_path` | `String` | yes | — | — | Destination JSON bundle path |
| `bundle_kind` | `String` | no | `registry` | `registry` / `contribution` | Export plain registry rows or an offline contribution bundle |
| `profile_id` | `Option<String>` | no | — | — | Required for `bundle_kind=contribution` |
| `query` | `Option<String>` | no | — | — | Same filter as search |
| `row_kind` | `Option<String>` | no | — | — | Same filter as search |
| `include_disabled` | `bool` | no | `false` | — | Include disabled/removed rows |
| `include_audit_evidence` | `bool` | no | `true` | — | Include redacted action-audit summaries in contribution bundles |
| `include_quality_summary` | `bool` | no | `true` | — | Include `profile_quality/v1/<profile_id>` summary if present |
| `max_audit_rows` | `u32` | no | `100` | `1..=1000` | Tail rows scanned for contribution evidence |
| `limit` | `u32` | no | `100` | `1..=1000` | Maximum exported rows |

**Returns:** `ProfileRegistryExportResponse { output_path, bundle_kind,
bytes_written, rows_exported, audit_evidence_rows, quality_summary_rows,
deterministic_bundle_sha256, registry_rows_sha256, audit_evidence_sha256,
quality_summary_sha256, rows }`.
Contribution exports strip path-like registry metadata fields from the shared
bundle rows before hashing and writing the JSON bundle.

## 23n. `profile_registry_import`

**Description:** "Import a local profile registry JSON bundle"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE`
**Side effects:** writes validated `CF_PROFILES` and `CF_KV` registry rows;
contribution imports stage a `profile_contribution_bundle` row

| Parameter | Type | Required | Description |
|---|---|---|---|
| `bundle_path` | `String` | yes | Local JSON bundle path |

**Returns:** `ProfileRegistryImportResponse { bundle_path, bundle_kind,
rows_read, cf_profile_rows_written, cf_kv_rows_written, duplicate_rows,
contribution_row_key, deterministic_bundle_sha256, rows }`.
Duplicate rows are byte-identical rows, plus contribution rows with the same
deterministic content even if the exact bundle-file hash differs.
Contribution imports run local abuse review before active-row writes. Hostile
contribution bundles write only a quarantined contribution row with risk
reason codes; staged contributions carry `rank_eligible=false`,
`quality_weight=0`, and `external_quality_claims_trusted=false` until local
success evidence exists on this host.
**Errors:** `TOOL_PARAMS_INVALID` for malformed bundle schema, unsupported CF,
non-registry key, invalid `CF_KV` namespace, non-object row values, hash
mismatch, or same-key/different-value local conflicts.

## 23o. `profile_registry_rollback`

**Description:** "Rollback an installed profile registry row to a prior trusted package"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE`
**Side effects:** validates a prior package row, rewrites
`CF_PROFILES/profile_registry/v1/installed/<profile_id>`, writes a rollback row
under `CF_PROFILES/profile_registry/v1/rollback/<profile_id>/<timestamp>`, and
reads both rows back.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `profile_id` | `String` | yes | Installed profile id to restore |
| `target_package_id` | `Option<String>` | no | Optional explicit package id; must be paired with `target_package_version` |
| `target_package_version` | `Option<String>` | no | Optional explicit package version; must be paired with `target_package_id` |
| `reason` | `Option<String>` | no | Local rollback reason stored on the rollback row |

If no explicit target is supplied, the tool selects the newest prior package
version for the installed profile. The target package row must be active,
unrevoked, match the profile id, and carry `trust_status = "trusted"` or
`"local_validated"`.

**Returns:** `ProfileRegistryRollbackResponse { profile_id, installed_row_key,
rollback_row_key, previous_package_id, previous_package_version,
rolled_back_package_id, rolled_back_package_version, installed_row,
rollback_row }`. The installed row readback carries the rolled-back package's
trust/signature metadata, not stale metadata from the package being replaced.
**Errors:** `TOOL_PARAMS_INVALID`, `PROFILE_ROLLBACK_UNAVAILABLE`,
`STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`.

## 23p. `audit_intelligence_query`

**Description:** "Summarize profile-linked audit outcomes for registry intelligence"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`
**Side effects:** none; reads audit/storage rows

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `profile_id` | `String` | yes | — | loaded or registry profile id | Profile id to match across stored audit contexts |
| `max_rows` | `u32` | no | `100` | `1..=1000` | Newest rows scanned per audit CF |

**Returns:** `AuditIntelligenceQueryResponse { profile_id, max_rows, action,
events, reflexes, sessions, quality_snapshot_key, quality_snapshot,
learning_candidates }`. Buckets summarize matches by status, tool/kind, and
error code across `CF_ACTION_LOG`, `CF_EVENTS`, `CF_REFLEX_AUDIT`, and
`CF_SESSIONS`; the quality snapshot is read from
`CF_PROFILES/profile_quality/v1/<profile_id>` when present.

## 23q. `audit_export_consent_set`

**Description:** "Set local consent state for redacted audit export bundles"
**Permissions:** `READ_STORAGE`, `WRITE_STORAGE`
**Side effects:** writes `CF_KV/audit_export/v1/consent/<profile_id>` and reads it back

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `profile_id` | `String` | yes | — | Profile whose audit rows may be locally exported after consent |
| `enabled` | `bool` | yes | — | Enables or disables local export consent |
| `redaction_policy` | `String` | no | `strict` | Only `strict` is live |
| `operator_note` | `Option<String>` | no | — | Optional local note stored on the consent row |

**Returns:** `AuditExportConsentSetResponse { profile_id, consent_key,
enabled, redaction_policy, wrote_row, consent_row }`. The stored row includes
`row_kind = "audit_export_consent"`, `allowed_redaction_policies`, and
`external_sharing_allowed = false`.
**Errors:** `AUDIT_EXPORT_REDACTION_REQUIRED`, `STORAGE_READ_FAILED`,
`STORAGE_WRITE_FAILED`, `TOOL_INTERNAL_ERROR`.

## 23r. `audit_export_bundle`

**Description:** "Export a local redacted audit bundle after consent verification"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`
**Side effects:** reads `CF_KV` consent and `CF_ACTION_LOG`; writes local
`manifest.json`, `rows.json`, and `redaction_report.json` files under
`output_path`

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `profile_id` | `String` | yes | — | Profile id matched against action audit rows |
| `output_path` | `String` | yes | — | — | Local directory for the bundle files |
| `redaction_policy` | `Option<String>` | runtime-required | — | `strict` | Must be explicitly selected and consented |
| `max_rows` | `u32` | no | `100` | `1..=1000` | Newest action-log rows scanned |
| `max_row_bytes` | `u64` | no | `65536` | `1..=524288` | Matching row size ceiling before abort |

**Returns:** `AuditExportBundleResponse { profile_id, output_dir,
manifest_path, rows_path, redaction_report_path, consent_key,
redaction_policy, rows_scanned, rows_exported, redacted_fields,
manifest_sha256, rows_sha256, redaction_report_sha256, consent_row }`.

Strict redaction removes window titles, paths, command lines, exact timing
fields, OCR/text/clipboard/transcript fields, screenshots/images/pixels, user
identifiers, and high-cardinality IDs while retaining bounded profile/outcome
signals.
**Errors:** `AUDIT_EXPORT_CONSENT_REQUIRED`,
`AUDIT_EXPORT_REDACTION_REQUIRED`, `AUDIT_EXPORT_PAYLOAD_TOO_LARGE`,
`TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_CORRUPTED`,
`TOOL_INTERNAL_ERROR`.

## 24. `replay_record`

**Description:** "Record observations and/or events to a replay JSONL file"
**Permissions:** `WRITE_REPLAY`
**Side effects:** writes a JSONL file under `%LOCALAPPDATA%/synapse/replays` (or operator-specified absolute path under that root).

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `target` | `String` | no | `"observations"` | `observations` / `events` / `both` |
| `format` | `String` | no | `"jsonl"` | Only `jsonl` accepted |
| `duration_ms` | `u32` | yes | — | `>= 0`; how long to record |
| `path` | `Option<String>` | no | — | Relative paths joined to `replay_root()`; lexical-normalized; must stay under root |

**Returns:** `ReplayRecordResponse { path: String, records_written: u64, observations_skipped: u64, bytes: u64 }`.

Recording cadence: observations sampled every `OBSERVATION_SAMPLE_INTERVAL = 250 ms`; events drained every `EVENT_DRAIN_INTERVAL = 20 ms`.

**Errors:** `REPLAY_TARGET_INVALID`, `REPLAY_FORMAT_INVALID`, `SAFETY_PERMISSION_DENIED` (path outside allow-root), `TOOL_PARAMS_INVALID`.

## 25. `audio_tail`

**Description:** "Return the latest loopback audio tail as PCM s16le bytes"
**Permissions:** `READ_AUDIO` (requires `--enable-audio`)
**Side effects:** none (reads the existing ring; loopback must be running or the runtime initialized on demand)

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `seconds` | `u32` | no | `5` | `0..=MAX_RING_SECONDS=5` | `0` returns an empty PCM body |

**Returns:** `AudioTailResponse { pcm: Vec<u8>, sample_rate: u32, channels: u16, format: "s16le" }`. The PCM is **left-padded with zeros** when the ring contains fewer samples than requested, so `pcm.len() == seconds * sample_rate * channels * 2`.

**Errors:** `TOOL_PARAMS_INVALID` (seconds > 5), `AUDIO_LOOPBACK_INIT_FAILED`, `AUDIO_DEVICE_LOST`.

## 26. `audio_transcribe`

**Description:** "Transcribe the latest loopback audio tail with Whisper tiny"
**Permissions:** `READ_AUDIO`
**Side effects:** loads Whisper-tiny on first call (one-shot SHA-256 verification + ORT session bring-up); runs inference.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `seconds` | `u32` | no | `5` | `0..=5` | Window size |
| `language` | `String` | no | `"en"` | `"en"` only (case-insensitive, empty → `"en"`) | Anything else → `TOOL_PARAMS_INVALID` |

**Returns:** `AudioTranscribeResponse { text: String, confidence: f32, latency_ms: u64, model_id: "whisper_tiny_int8" }`.

**Errors:** `TOOL_PARAMS_INVALID`, `AUDIO_STT_MODEL_NOT_LOADED`, `MODEL_HASH_MISMATCH`, `MODEL_LOAD_FAILED`, `MODEL_BACKEND_UNAVAILABLE`.

## 27. `storage_inspect`

**Description:** "Inspect RocksDB column families: row counts, byte sizes, and bounded newest-row samples"
**Permissions:** `READ_STORAGE` (operator diagnostic)
**Side effects:** none; reads `Db::cf_sizes`, per-CF scan counts, and bounded newest-row samples

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| none | `{}` | no | `{}` | Always reports every operator-visible RocksDB column family. |

**Returns:** `StorageInspectResponse { schema_version: u32, pressure_level, pressure_transition_codes, audit_retention_policies, cf_sizes, cf_row_counts, cf_row_samples }`. Each `cf_row_samples` value is a bounded newest-row list with `key_hex`, `value_len_bytes`, `value_utf8_prefix`, and `value_truncated`. `audit_retention_policies` lists the #463 audit classes plus the M4 reality storage classes that `storage_gc_once` uses in `AUDIT_RETENTION` mode: strategic `reality_baselines`, `reality_heads`, and `reality_audits`, plus capped `reality_delta_journal`.
**Errors:** `STORAGE_OPEN_FAILED`, `TOOL_PARAMS_INVALID` (unknown parameter).

## 28. `storage_put_probe_rows`

**Description:** "Insert probe rows into a CF to exercise the write batcher + flush + GC paths"
**Permissions:** `WRITE_STORAGE` (operator diagnostic)
**Side effects:** writes N synthetic rows into the chosen CF; calls `Db::flush`.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `cf_name` | `String` | yes | — | `CF_EVENTS`, `CF_OBSERVATIONS`, `CF_SESSIONS`, `CF_ACTION_LOG`, or `CF_KV` | Target CF |
| `key_prefix` | `String` | yes | — | non-empty, <= 128 bytes | Prefix used in generated row keys |
| `rows` | `u32` | yes | — | `0..=10000` | Number of probe rows |
| `value_bytes` | `u32` | yes | — | `0..=65536` | Per-row byte filler size when `value_json` is absent |
| `value_json` | `Option<object>` | no | — | JSON object | When present, writes this JSON object as the row value instead of byte filler |
| `ts_ns_start` / `ts_ns_step` | `Option<u64>` | no | — | any `u64` | Deterministic timestamp generation for JSON probe rows |

**Returns:** `StoragePutProbeRowsResponse { cf_name, key_prefix, requested_rows, value_bytes, before_rows, after_rows, rows_added, after_cf_size_bytes, pressure_level }`.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_WRITE_FAILED`, `STORAGE_DISK_PRESSURE_LEVEL_1..4` (writes silently dropped at the higher pressure levels).

## 29. `storage_gc_once`

**Description:** "Run one synchronous storage GC pass and return per-CF before/after row counts"
**Permissions:** `WRITE_STORAGE` (operator diagnostic)
**Side effects:** evicts rows from a diagnostic CF whose row count exceeds its soft cap. With `cf_name="AUDIT_RETENTION"`, scans profile-linked audit rows, backfills missing profile linkage, dedupes repeated outcomes, deletes expired/capped rows, preserves unknown-schema and strategic rows, and writes `CF_KV/audit_retention/v1/report/<run_id>`. The M4 reality namespace is policy-visible: baseline, head, and audit rows are strategic preserve classes, while `CF_KV/reality/delta/v1/` is capped as the high-frequency delta journal. Audit-retention backfills and report rows use the bounded storage-maintenance write path so Level3/Level4 pressure cannot silently drop the migration/report evidence; ordinary probe or ingestion writes remain pressure-gated.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `cf_name` | `String` | yes | — | Diagnostic CF name or `AUDIT_RETENTION` |
| `soft_cap_rows` | `u64` | yes | — | Row soft cap; for `AUDIT_RETENTION`, per-CF non-strategic row cap |
| `hard_cap_rows` | `u64` | yes | — | Row hard cap; must be >= soft cap |
| `run_id` | `Option<String>` | no | generated | `AUDIT_RETENTION` report id |
| `now_ns` | `Option<u64>` | no | system clock | Synthetic/manual-FSV clock for expiry decisions |
| `max_age_ns` | `Option<u64>` | no | policy TTL | Override age threshold for all non-strategic audit classes |
| `dedupe_window_ns` | `Option<u64>` | no | `1_000_000_000` | Window for repeated-outcome dedupe |
| `profile_id` | `Option<String>` | no | all profiles | Limit retention decisions to one profile id |

**Returns:** `StorageGcOnceResponse { cf_name, before_rows, after_rows, total_evicted_rows, cache_evictions_total_delta, cf_reports, audit_retention_report_key?, audit_retention? }`. `audit_retention` is present only for `cf_name="AUDIT_RETENTION"` and is read back from the persisted `CF_KV` report row before return.
**Errors:** `STORAGE_OPEN_FAILED`.

## 30. `storage_pressure_sample`

**Description:** "Apply one synthetic free-byte sample to drive the disk-pressure responder"
**Permissions:** `WRITE_STORAGE` (operator diagnostic)
**Side effects:** updates `Db::pressure_level()` for subsequent writes; may trigger compaction on selected CFs at higher levels.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `free_bytes` | `u64` | yes | — | Synthetic free-bytes value applied via `Db::run_pressure_check_with_free_bytes_sample` |

**Returns:** `StoragePressureSampleResponse { previous_level: String, current_level: String, frozen_cfs: Vec<String> }`. Levels: `Normal` / `Level1` / `Level2` / `Level3` / `Level4`.
**Errors:** `STORAGE_OPEN_FAILED`.

## Permission mapping reference

For convenience the M3 tool-call gating is summarized here (live source: `crates/synapse-mcp/src/m3/permissions.rs`, plus per-module `required_permissions_*` functions):

| Tool | Required permissions |
|---|---|
| `subscribe`, `subscribe_cancel` | `READ_EVENTS` |
| `reflex_register` | `WRITE_REFLEX` + actions' permissions |
| `reflex_cancel`, `reflex_list`, `reflex_history` | `READ_REFLEX` |
| `profile_list` | `READ_PROFILE` |
| `profile_activate` | `WRITE_PROFILE_ACTIVE` |
| `replay_record` | `WRITE_REPLAY` |
| `audio_tail`, `audio_transcribe` | `READ_AUDIO` |
| `profile_authoring_generate`, `profile_authoring_accept` | `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE` |
| `profile_authoring_list`, `profile_authoring_inspect`, `profile_authoring_export` | `READ_STORAGE` |
| `profile_authoring_reject` | `READ_STORAGE`, `WRITE_STORAGE` |
| `profile_quality_refresh` | `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE` |
| `profile_registry_search`, `profile_registry_inspect`, `profile_registry_report`, `profile_registry_export`, `audit_intelligence_query`, `audit_export_bundle` | `READ_PROFILE`, `READ_STORAGE` |
| `profile_registry_install`, `profile_registry_disable`, `profile_registry_import`, `profile_registry_rollback` | `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE` |
| `audit_export_consent_set` | `READ_STORAGE`, `WRITE_STORAGE` |
| `storage_inspect` | `READ_STORAGE` |
| `storage_put_probe_rows`, `storage_gc_once`, `storage_pressure_sample` | `WRITE_STORAGE` |

`reflex_register`'s effective permission set is computed by `add_action_permissions` over the compiled `Vec<Action>` (e.g., `Action::PadReport` requires `INPUT_PAD`; keyboard and mouse actions require the matching input permissions). The retired `hardware` backend token does not add a separate permission; it fails closed during action dispatch.

M1/M2 tools (`health`, `observe`, `find`, `read_text`, `set_capture_target`, `set_perception_mode`, `act_*`, `release_all`) do not gate at the M3 permission layer because they predate M3; the M3 permission layer applies only to the M3 tool surface. (For reflex-driven action emission, the reflex-register permission check is the gating point.)

## Cross-references

- Type definitions: [05_core_types_and_errors.md](05_core_types_and_errors.md)
- Service / dispatch: [06_mcp_service_and_transports.md](06_mcp_service_and_transports.md)
- Reflex semantics: [07_reflex_runtime.md](07_reflex_runtime.md)
- Action emitter contract: [08_action_subsystem.md](08_action_subsystem.md)
- Perception assembly: [09_perception_and_capture.md](09_perception_and_capture.md)
- Audio: [10_audio_and_models.md](10_audio_and_models.md)
- Storage CFs: [04_storage_layer.md](04_storage_layer.md)
- Configuration knobs: [03_configuration.md](03_configuration.md)
