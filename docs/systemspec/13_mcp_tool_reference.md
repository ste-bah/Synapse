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
- `crates/synapse-mcp/src/server/everquest_scorecard.rs`
- `crates/synapse-mcp/src/m1.rs` (+ `m1/{ocr, search, sources}.rs`)
- `crates/synapse-mcp/src/m2/{aim, click, clipboard, drag, pad, press, release_all, scroll, type_text}.rs`
- `crates/synapse-mcp/src/m3/{audio, audit_export, permissions, profile, profile_authoring, profile_quality, profile_registry, reflex, replay, subscribe}.rs`
- `crates/synapse-core/src/types.rs`

All 63 live tools are registered on `SynapseService` via `#[tool(description=...)]` in `server.rs`. Tool descriptions are taken verbatim from the source. Every tool returns through `Json<T>` so the response shape exactly matches the deserialized response struct.

Default error response shape (all tools): `ErrorData { code: rmcp::ErrorCode(-32099), message, data: { "code": <SCREAMING_SNAKE_CASE> } }` via `crates/synapse-mcp/src/m1.rs::mcp_error`.

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
**Errors:** `OBSERVE_NO_PERCEPTION_AVAILABLE` (forced via `SYNAPSE_MCP_FORCE_NO_PERCEPTION`), `OBSERVE_INTERNAL` (forced or assembler error), `A11Y_NO_FOREGROUND`, `CAPTURE_TARGET_LOST`, perception subsystem errors.

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
**Permissions:** `INPUT_MOUSE` (via reflex registration paths; tool itself doesn't gate at server.rs); the action's `backend` adds `INPUT_HARDWARE_HID` if `Hardware` is chosen.
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
**Errors:** `ACTION_UNSUPPORTED_KEY`, `ACTION_RATE_LIMITED`, `ACTION_BACKEND_UNAVAILABLE` (`Hardware` until M4).

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

**Description:** "Persist calibrated EverQuest visible-map sensor state from current-state, screenshot/observe evidence, and local map files"
**Side effects:** reads the persisted current-state row by default, reads the current zone's local `maps/*.txt` file, fuses visible-map evidence from a separately inspected observe/screenshot source, writes `CF_KV/everquest/map_sensor/v1/everquest.live/<sensor_id>`, then reads the exact row back before returning.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `sensor_id` | `String` | yes | - | Map-sensor row id |
| `profile_id` | `String` | no | `everquest.live` | EverQuest profile id; other ids fail closed |
| `state_row_key` | `String` | no | `everquest/current_state/v1/everquest.live` | Current-state source row |
| `state_override` | `Option<EverQuestMapSensorStateOverride>` | no | - | Synthetic/manual state input with source refs |
| `visible_map_override` | `Option<EverQuestVisibleMapOverride>` | no | - | Verified visible-map evidence from observe/screenshot readback |
| `expected_zone_short_name` | `Option<String>` | no | - | Optional zone consistency guard |
| `stale_after_seconds` | `u64` | no | `300` | Older current-state rows abstain |
| `max_nearest_labels` | `usize` | no | `8` | Nearest landmark cap; max `16` |

**Returns:** `EverQuestMapSensorResponse { ok, row_key, stored_value_len_bytes, sensor }`. Calibrated rows carry foreground proof, visible map bounds/confidence, current `/loc`, map file SHA-256/mtime/counts, nearest labels/exits, visible label or player-marker anchors, transform confidence, hazards, source refs, and evidence-boundary flags. Hidden maps, occlusion, stale current state, missing `/loc`, non-EQ foreground, zoom/pan changes, low visible confidence, or contradictory zone sources persist abstain rows instead of guessed calibration.
**Errors:** `TOOL_PARAMS_INVALID`, `ACTION_TARGET_INVALID`, `STORAGE_READ_FAILED`, `STORAGE_WRITE_FAILED`, `STORAGE_CORRUPTED`, `TOOL_INTERNAL_ERROR`.

Manual FSV must read the physical screenshot/observe crop, physical EQ log/current-state row, and local map file before the trigger, call the real MCP tool, then separately inspect the persisted `CF_KV` map-sensor row. The tool does not execute movement.

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

**Returns:** `EverQuestRoutePlanResponse { ok, row_key, stored_value_len_bytes, plan }`. Ready plans carry current and target waypoints, map coordinates, distance, nearest labels, source map lines, confidence, guard requirements, and evidence-boundary flags. Floor-route guidance skips already reached local map-line nodes before choosing the next waypoint. Unknown zone, missing `/loc`, absent target, stale state, or conflicting map calibration persist abstain rows instead of guessed movement.
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

## 9j. `everquest_action_prior_record`

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

## 9k. `everquest_action_prior_scorecard`

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
**Permissions:** `WRITE_REFLEX` plus any input permissions implied by `then` actions (`INPUT_KEYBOARD`/`INPUT_MOUSE`/`INPUT_PAD`/`INPUT_HARDWARE_HID`).
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

**Returns:** `ReplayRecordResponse { path: String, records_written: u64, bytes: u64 }`.

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

**Returns:** `StorageInspectResponse { schema_version: u32, pressure_level, pressure_transition_codes, audit_retention_policies, cf_sizes, cf_row_counts, cf_row_samples }`. Each `cf_row_samples` value is a bounded newest-row list with `key_hex`, `value_len_bytes`, `value_utf8_prefix`, and `value_truncated`. `audit_retention_policies` lists the #463 audit classes and strategic prefixes that `storage_gc_once` uses in `AUDIT_RETENTION` mode.
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
**Side effects:** evicts rows from a diagnostic CF whose row count exceeds its soft cap. With `cf_name="AUDIT_RETENTION"`, scans profile-linked audit rows, backfills missing profile linkage, dedupes repeated outcomes, deletes expired/capped rows, preserves unknown-schema and strategic rows, and writes `CF_KV/audit_retention/v1/report/<run_id>`. Audit-retention backfills and report rows use the bounded storage-maintenance write path so Level3/Level4 pressure cannot silently drop the migration/report evidence; ordinary probe or ingestion writes remain pressure-gated.

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

`reflex_register`'s effective permission set is computed by `add_action_permissions` over the compiled `Vec<Action>` (e.g., `Action::PadReport` requires `INPUT_PAD`; any action with `Backend::Hardware` adds `INPUT_HARDWARE_HID`).

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
