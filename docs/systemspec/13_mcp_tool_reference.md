# 13 — MCP Tool Reference

Source files covered:
- `crates/synapse-mcp/src/server.rs`
- `crates/synapse-mcp/src/m1.rs` (+ `m1/{ocr, search, sources}.rs`)
- `crates/synapse-mcp/src/m2/{aim, click, clipboard, drag, pad, press, release_all, scroll, type_text}.rs`
- `crates/synapse-mcp/src/m3/{audio, permissions, profile, profile_quality, profile_registry, reflex, replay, subscribe}.rs`
- `crates/synapse-core/src/types.rs`

All 41 tools below are registered on `SynapseService` via `#[tool(description=...)]` in `server.rs`. Tool descriptions are taken verbatim from the source. Every tool returns through `Json<T>` so the response shape exactly matches the deserialized response struct.

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

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `text` | `String` | yes | — | UTF-8; surrogate pairs split via `KeystrokeEvent` lowering |
| `into_element` | `Option<ElementId>` | no | — | If set, the assembler is expected to have focused it first (currently advisory) |
| `dynamics` | `TypeDynamics` | no | `Natural` | `Burst`/`Linear`/`Natural` |
| `linear_ms_per_char` | `u32` | no | — | Only used when `dynamics = Linear` |
| `use_scancodes` | `bool` | no | — | When true, keys emit with `use_scancode = true` |
| `press_enter_after` | `bool` | no | `false` | Appends a `KeyPress { Key::Named("enter") }` |
| `backend` | `TypeBackend` | no | `Auto` | `Software` / `Hardware` / `Auto` |

**Returns:** `ActTypeResponse { ok, chars_typed: u32, elapsed_ms: u32 }`.
**Errors:** `ACTION_FOREGROUND_LOST`, `ACTION_RATE_LIMITED`, `ACTION_BACKEND_UNAVAILABLE`, `ACTION_UNSUPPORTED_KEY` (only when individual chars lower to unsupported keys).

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

**Returns:** `ProfileListResponse { profiles: Vec<ProfileStatus>, active_profile_id: Option<String> }`. Each `ProfileStatus` carries `id`, `label`, `use_scope`, `mode`, `detection_classes`, `hud_fields`, `keymap_actions`, `backends`, `event_extensions`, `matches: Vec<ProfileMatchStatus>`, `metadata: BTreeMap<String, String>`, `active: bool`, `schema_version: u32`.

## 23. `profile_activate`

**Description:** "Activate a loaded profile by id"
**Permissions:** `WRITE_PROFILE_ACTIVE`
**Side effects:** updates `ProfileRuntime` active state in memory (no FS write, no `CF_PROFILES` write in current build).

| Parameter | Type | Required | Description |
|---|---|---|---|
| `profile_id` | `String` | yes | Must match a parsed profile id |

**Returns:** `ProfileActivateResponse { profile_id, active_profile_id, previous_active_profile_id, changed: bool }`. `changed=false` if `profile_id` was already active.
**Errors:** `PROFILE_NOT_FOUND`, `SAFETY_PROFILE_ACTION_DENIED` (use_scope=Unknown without `--allow-unknown-profile`).

## 23a. `profile_quality_refresh`

**Description:** "Refresh local profile quality scoring from stored action audit rows"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE`
**Side effects:** reads `CF_ACTION_LOG`; writes and immediately reads back
`CF_PROFILES` key `profile_quality/v1/<profile_id>`

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `profile_id` | `String` | yes | — | loaded profile id | Profile whose quality snapshot should be refreshed |
| `max_audit_rows` | `u32` | no | `5000` | `1..=50000` | Newest action audit rows scanned |
| `stale_after_ns` | `u64` | no | `86400000000000` | `1..=2592000000000000` | Rows older than this are counted as stale and ignored for scoring |

**Returns:** `ProfileQualityRefreshResponse { profile_id, cf_name,
key_hex, wrote_snapshot, previous_evidence_hash, stored_value_len_bytes,
stored_value_utf8_prefix, snapshot }`. `snapshot` contains source counters,
ignored corrupt/stale rows, counts/rates, Wilson lower-bound score,
compatibility counters, profile-schema-version recency/mixed-version counters,
redaction policy, and contribution policy.

The score-bearing sample is foreground-profile `ok` vs `error` rows only.
Denied, stale, corrupt, active-profile-only, and profile-mismatch rows are
reported as explainability/compatibility counters and do not invent success
samples. Export is always disabled; contribution requires a future explicit
operator-approved path.

**Errors:** `PROFILE_NOT_FOUND`, `TOOL_PARAMS_INVALID`, `STORAGE_READ_FAILED`,
`STORAGE_WRITE_FAILED`, `TOOL_INTERNAL_ERROR`.

## 23b. `profile_registry_search`

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

## 23c. `profile_registry_inspect`

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

## 23d. `profile_registry_install`

**Description:** "Install or update a local profile registry package manifest"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE`
**Side effects:** validates manifest/profile files; writes `CF_PROFILES`
registry rows and a `CF_KV` source head pointer; reads written rows back

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `manifest_path` | `String` | yes | — | Local package manifest TOML path |
| `expected_manifest_digest` | `Option<String>` | no | — | Optional `sha256:<hex>` digest that must match manifest bytes |
| `source_id` | `String` | no | `registry.local` | Lowercase source id for source/head rows |

**Returns:** `ProfileRegistryInstallResponse { operation, source_id,
package_id, package_version, profile_id, profile_version, manifest_path,
manifest_digest, profile_toml_path, wrote_rows, idempotent,
cf_profile_row_keys, cf_kv_row_keys, row_summaries }`.

Duplicate package id/version with the same manifest digest is an idempotent
no-op. Duplicate id/version with a different digest fails closed.

## 23e. `profile_registry_disable`

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

## 23f. `profile_registry_export`

**Description:** "Export local profile registry rows to a JSON bundle"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`
**Side effects:** writes a local JSON bundle file

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `output_path` | `String` | yes | — | — | Destination JSON bundle path |
| `query` | `Option<String>` | no | — | — | Same filter as search |
| `row_kind` | `Option<String>` | no | — | — | Same filter as search |
| `include_disabled` | `bool` | no | `false` | — | Include disabled/removed rows |
| `limit` | `u32` | no | `100` | `1..=1000` | Maximum exported rows |

**Returns:** `ProfileRegistryExportResponse { output_path, bytes_written,
rows_exported, rows }`.

## 23g. `profile_registry_import`

**Description:** "Import a local profile registry JSON bundle"
**Permissions:** `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE`
**Side effects:** writes validated `CF_PROFILES` and `CF_KV` registry rows

| Parameter | Type | Required | Description |
|---|---|---|---|
| `bundle_path` | `String` | yes | Local JSON bundle path |

**Returns:** `ProfileRegistryImportResponse { bundle_path, rows_read,
cf_profile_rows_written, cf_kv_rows_written, rows }`.
**Errors:** `TOOL_PARAMS_INVALID` for malformed bundle schema, unsupported CF,
non-registry key, invalid `CF_KV` namespace, or non-object row values.

## 23h. `audit_intelligence_query`

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

**Returns:** `StorageInspectResponse { schema_version: u32, pressure_level, pressure_transition_codes, cf_sizes, cf_row_counts, cf_row_samples }`. Each `cf_row_samples` value is a bounded newest-row list with `key_hex`, `value_len_bytes`, `value_utf8_prefix`, and `value_truncated`.
**Errors:** `STORAGE_OPEN_FAILED`, `TOOL_PARAMS_INVALID` (unknown parameter).

## 28. `storage_put_probe_rows`

**Description:** "Insert probe rows into a CF to exercise the write batcher + flush + GC paths"
**Permissions:** `WRITE_STORAGE` (operator diagnostic)
**Side effects:** writes N synthetic rows into the chosen CF; calls `Db::flush`.

| Parameter | Type | Required | Default | Range | Description |
|---|---|---|---|---|---|
| `cf` | `String` | yes | — | one of `ALL_COLUMN_FAMILIES` | Target CF |
| `count` | `u32` | yes | — | `1..=10000` | Number of probe rows |
| `value_bytes` | `Option<u32>` | no | `256` | `1..=65536` | Per-row payload size |

**Returns:** `StoragePutProbeRowsResponse { cf, rows_written, bytes_written, flush_elapsed_ms }`.
**Errors:** `TOOL_PARAMS_INVALID`, `STORAGE_WRITE_FAILED`, `STORAGE_DISK_PRESSURE_LEVEL_1..4` (writes silently dropped at the higher pressure levels).

## 29. `storage_gc_once`

**Description:** "Run one synchronous storage GC pass and return per-CF before/after sizes"
**Permissions:** `WRITE_STORAGE` (operator diagnostic)
**Side effects:** evicts rows from any CF whose size exceeds its soft cap; emits `cache_evictions_total{cf,reason="soft_cap"}` counter increments.

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| (none) | — | — | — | Empty params |

**Returns:** `StorageGcOnceResponse { elapsed_ms, cf_reports: Vec<StorageGcCfReport { cf, before_bytes, after_bytes, rows_evicted, hit_hard_cap: bool }> }`.
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
| `profile_quality_refresh` | `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE` |
| `profile_registry_search`, `profile_registry_inspect`, `profile_registry_export`, `audit_intelligence_query` | `READ_PROFILE`, `READ_STORAGE` |
| `profile_registry_install`, `profile_registry_disable`, `profile_registry_import` | `READ_PROFILE`, `READ_STORAGE`, `WRITE_STORAGE` |
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
