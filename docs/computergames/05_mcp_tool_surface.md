# 05 — MCP Tool Surface

## 1. Design rules

1. **Tool count cap:** M3 shipped 30 live MCP tools. M4 expands the target
   surface to 33 live MCP tools by adding `act_combo`, `act_run_shell`, and
   `act_launch` per the M4 phase plan. M5 adds the local registry/audit scoring
   tool `profile_quality_refresh` plus the #458 local registry/intelligence
   tool set, #460 adds local audit-export consent/bundle tools, and #462 adds
   six local profile-authoring candidate tools, bringing the live surface to
   50. Any further agent-facing tools require an
   ADR-approved cap change. Overlapping tools merge. Profile and parameter
   knobs are the escape hatches.
2. **One tool, one verb.** No `do_everything(action_kind, ...)` mega-tools.
3. **Structured input, structured output.** Every tool defines a JSON Schema with `additionalProperties: false`. Every response carries explicit fields, no free-form text.
4. **No silent success.** If a tool did not do the work, it returns an MCP error with `code: SCREAMING_SNAKE_CASE`, never `success: true` with a partial result.
5. **All async; all cancellable.** Long-running tools support progress notifications via Streamable HTTP SSE upgrade.
6. **Idempotency tokens where it matters.** `act_run_shell`, `act_launch`, and similar accept an optional `idempotency_key` for safe retries.
7. **Stable identifiers.** `element_id`, `entity_id`, `track_id`, `reflex_id`, `session_id` are returned by tools and accepted unchanged by subsequent calls. Agent never invents these.

The first 30 tools below are the live M3 baseline. M4 adds rows 31-33, and M5
adds rows 34-50 for local profile-registry/audit quality scoring, authoring
candidates, registry row operations, import/export, audit intelligence, and
consented redacted audit export bundles.
Schemas use abbreviated JSON Schema syntax; canonical schema is exported by the
daemon through standard MCP `tools/list`. Until the M4 tools are implemented,
their schemas in this doc are the target contract for #401/#403/#406 and the
future `tools/list` snapshots in #447/#448.

---

## 2. Tool registry summary

| # | Tool | Verb | Side effect |
|---|---|---|---|
| 1 | `observe` | read | none |
| 2 | `find` | read | none |
| 3 | `read_text` | read | none |
| 4 | `audio_tail` | read | none |
| 5 | `audio_transcribe` | read | optional STT inference |
| 6 | `subscribe` | read | opens push stream |
| 7 | `subscribe_cancel` | config | closes push stream |
| 8 | `set_capture_target` | config | reconfigures capture |
| 9 | `set_perception_mode` | config | reconfigures perception |
| 10 | `act_click` | write | mouse click |
| 11 | `act_type` | write | keyboard |
| 12 | `act_press` | write | keyboard |
| 13 | `act_aim` | write | mouse move |
| 14 | `act_drag` | write | mouse drag |
| 15 | `act_scroll` | write | mouse scroll |
| 16 | `act_pad` | write | gamepad |
| 17 | `act_clipboard` | write/read | clipboard |
| 18 | `release_all` | write | releases all held inputs |
| 19 | `reflex_register` | write | adds reflex |
| 20 | `reflex_cancel` | write | removes reflex |
| 21 | `reflex_list` | read | none |
| 22 | `reflex_history` | read | none |
| 23 | `profile_list` | read | none |
| 24 | `profile_activate` | config | loads profile |
| 25 | `health` | read | none |
| 26 | `replay_record` | config | writes replay JSONL |
| 27 | `storage_inspect` | read | none |
| 28 | `storage_put_probe_rows` | write | writes bounded synthetic storage rows |
| 29 | `storage_gc_once` | write | runs one GC pass |
| 30 | `storage_pressure_sample` | write | applies one synthetic pressure sample |
| 31 | `act_combo` | write | schedules a one-shot timed action sequence |
| 32 | `act_run_shell` | write | runs an allowlisted local shell command |
| 33 | `act_launch` | write | launches an allowlisted local process |
| 34 | `profile_authoring_generate` | write/read | proposes a local profile patch from replay/audit evidence |
| 35 | `profile_authoring_list` | read | lists local authoring candidate rows in `CF_PROFILES` |
| 36 | `profile_authoring_inspect` | read | reads one authoring candidate row |
| 37 | `profile_authoring_accept` | write/read | marks a candidate accepted without activating it |
| 38 | `profile_authoring_reject` | write/read | marks a candidate rejected |
| 39 | `profile_authoring_export` | read/write | writes a local candidate export bundle file |
| 40 | `profile_quality_refresh` | write/read | refreshes local profile quality from action audit rows |
| 41 | `profile_registry_search` | read | searches local registry rows in `CF_PROFILES` |
| 42 | `profile_registry_inspect` | read | reads one registry row from `CF_PROFILES` or `CF_KV` |
| 43 | `profile_registry_install` | write/read | validates a package manifest and writes registry rows |
| 44 | `profile_registry_disable` | write/read | marks an installed profile disabled or removed |
| 45 | `profile_registry_export` | read/write | writes a local JSON registry bundle file |
| 46 | `profile_registry_import` | write/read | validates and imports a local JSON registry bundle |
| 47 | `profile_registry_rollback` | write/read | rewrites an installed row to a prior trusted package |
| 48 | `audit_intelligence_query` | read | summarizes profile-linked audit outcomes |
| 49 | `audit_export_consent_set` | write/read | writes local consent state to `CF_KV` and reads it back |
| 50 | `audit_export_bundle` | read/write | writes a local redacted audit bundle after consent verification |

M3 live count: 30 tools. M4 live count: 33 tools. Current M5 live count: 50
tools.

Deferred ideas from earlier drafts (`describe` and `read_hud`) are still not
live M3/M4 agent-facing tools. `act_combo`, `act_run_shell`, and `act_launch`
are the only M4 additions approved by the phase plan.

---

## 3. Tool detail

### 3.1 `observe`

Returns current unified perception observation.

```json
{
  "name": "observe",
  "description": "Returns structured state of the focused window and surrounding context. Replaces screenshots for most use cases.",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "include": {
        "type": "array",
        "items": {"enum": ["focused", "elements", "entities", "hud", "audio", "events", "clipboard", "fs", "diagnostics"]},
        "description": "Which slots to populate. Defaults to ['focused','elements','entities','hud','events']"
      },
      "depth": {"type": "integer", "minimum": 0, "maximum": 6, "default": 2},
      "max_elements": {"type": "integer", "minimum": 1, "maximum": 500, "default": 60},
      "since_event_seq": {"type": "integer", "description": "If set, recent_events filtered to events with seq > this"}
    }
  }
}
```

Returns `Observation` (see `06_data_schemas.md`). Typical size 1–6 KB.

Errors: `OBSERVE_NO_PERCEPTION_AVAILABLE` (all sensors down), `OBSERVE_INTERNAL`.

### 3.2 `find`

Semantic search over visible elements and entities.

```json
{
  "name": "find",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["query"],
    "properties": {
      "query": {"type": "string", "description": "Free-text query, e.g., 'save button' or 'the red enemy'"},
      "scope": {"enum": ["elements", "entities", "both"], "default": "both"},
      "limit": {"type": "integer", "minimum": 1, "maximum": 20, "default": 5},
      "in_window": {"type": "string", "description": "Optional element_id of a window to restrict to"}
    }
  }
}
```

Returns:

```json
{
  "results": [
    {
      "kind": "element",
      "element_id": "...",
      "name": "Save",
      "role": "Button",
      "automation_id": "btnSave",
      "bbox": {"x": 100, "y": 200, "w": 80, "h": 32},
      "score": 0.93
    },
    {
      "kind": "entity",
      "entity_id": "track:42",
      "class_label": "enemy",
      "bbox": {"x": 820, "y": 340, "w": 60, "h": 130},
      "score": 0.87
    }
  ]
}
```

Implementation: combines string similarity against UIA names/automation IDs and detection class labels with a small bias for foreground-window scope.

### 3.3 `describe` (deferred, not live M3)

Slow-path natural-language description via small VLM. Used for first-orientation on unknown games or when a11y + detection produce sparse results.

```json
{
  "name": "describe",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "region": {"type": "object", "properties": {"x":{}, "y":{}, "w":{}, "h":{}}, "description": "Region in screen coordinates; default full focused window"},
      "detail": {"enum": ["coarse", "standard", "detailed"], "default": "standard"}
    }
  }
}
```

Returns:

```json
{
  "description": "...",
  "model_id": "florence2-base",
  "latency_ms": 230
}
```

Latency 100–500 ms. Use sparingly; default to `observe` + `find` first.

### 3.4 `read_text`

OCR a region.

```json
{
  "name": "read_text",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "region": {"type": "object", "properties": {"x":{}, "y":{}, "w":{}, "h":{}}},
      "element_id": {"type": "string", "description": "Alternative to region: OCR an a11y element's bounding rect"},
      "backend": {"enum": ["winrt", "crnn", "auto"], "default": "auto"},
      "lang_hint": {"type": "string", "description": "BCP-47 language tag, e.g., 'en-US'"}
    }
  }
}
```

Returns:

```json
{
  "full_text": "...",
  "words": [{"text": "Save", "bbox": {}, "confidence": 0.99}],
  "confidence": 0.99,
  "region": {"x": 10, "y": 20, "w": 120, "h": 32},
  "lang": "en"
}
```

Pre-v1 OCR cache/tool payloads using `text` / `language` / `backend` are wipe-and-rebuild;
the M3 response shape does not carry a compatibility shim.

### 3.5 `read_hud` (deferred, not live M3)

Read named HUD regions defined by the active profile.

```json
{
  "name": "read_hud",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "fields": {"type": "array", "items": {"type": "string"}, "description": "Subset of HUD field names; default all"}
    }
  }
}
```

Returns:

```json
{
  "readings": {
    "hp": {"value": 85, "raw_text": "85/100", "confidence": 0.98, "stale_ms": 16},
    "ammo": {"value": 12, "raw_text": "12", "confidence": 0.99, "stale_ms": 16}
  }
}
```

Errors: `HUD_NO_ACTIVE_PROFILE`, `HUD_FIELD_NOT_DEFINED`.

### 3.6 `audio_tail`

Returns summary of recent audio events.

```json
{
  "name": "audio_tail",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "seconds": {"type": "number", "minimum": 0.1, "maximum": 30, "default": 5},
      "include_raw_waveform": {"type": "boolean", "default": false}
    }
  }
}
```

Returns:

```json
{
  "events": [
    {"at": "...", "kind": "loud_transient", "azimuth_deg": 47, "confidence": 0.8}
  ],
  "rms_db": -22.5,
  "vad_speech_pct": 0.0,
  "waveform_b64": null
}
```

### 3.7 `audio_transcribe`

Speech-to-text on the recent audio window.

```json
{
  "name": "audio_transcribe",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "seconds": {"type": "number", "minimum": 0.1, "maximum": 30, "default": 10},
      "lang_hint": {"type": "string", "default": "en"}
    }
  }
}
```

Returns:

```json
{"text": "...", "segments": [{"start_s": 0, "end_s": 1.2, "text": "..."}], "model_id": "whisper-tiny-int8"}
```

### 3.8 `subscribe`

Opens a push stream (SSE) of filtered events. Returns immediately with `subscription_id`; events arrive as MCP notifications.
Per ADR-0007, Synapse emits one notification per event and does not batch at
the EventBus or SSE layer.

```json
{
  "name": "subscribe",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "filter": {"$ref": "#/$defs/EventFilter"},
      "snapshot_first": {"type": "boolean", "default": true, "description": "If true, push current Observation before live events"}
    }
  }
}
```

Returns:

```json
{"subscription_id": "...", "snapshot_observation_seq": 12345}
```

Push events arrive as JSON-RPC notifications with method `synapse/event` and
params containing one `Event` value.

To cancel: `mcp/cancelled` JSON-RPC notification with original request id. Also exposes `subscribe_cancel(subscription_id)` for explicit teardown.

### 3.9 `set_capture_target`

Reconfigures active capture target.

Per ADR-0005, there is one active capture target per session. Monitor targets
are selected explicitly by index; Synapse does not stitch the virtual desktop or
capture multiple monitors concurrently in M3.

```json
{
  "name": "set_capture_target",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "target": {
        "oneOf": [
          {"type": "object", "required": ["kind"], "properties": {"kind": {"const": "primary"}}},
          {"type": "object", "required": ["kind", "monitor_index"], "properties": {"kind": {"const": "monitor"}, "monitor_index": {"type": "integer"}}},
          {"type": "object", "required": ["kind", "window_hwnd"], "properties": {"kind": {"const": "window"}, "window_hwnd": {"type": "integer"}}},
          {"type": "object", "required": ["kind", "element_id"], "properties": {"kind": {"const": "element_window"}, "element_id": {"type": "string"}}}
        ]
      },
      "min_update_interval_ms": {"type": "integer", "default": 16},
      "cursor_visible": {"type": "boolean", "default": true},
      "dirty_region_only": {"type": "boolean", "default": true}
    }
  }
}
```

### 3.10 `set_perception_mode`

```json
{
  "name": "set_perception_mode",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["mode"],
    "properties": {
      "mode": {"enum": ["a11y_only", "pixel_only", "hybrid", "auto"]}
    }
  }
}
```

### 3.11 `act_click`

```json
{
  "name": "act_click",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "target": {
        "oneOf": [
          {"type": "object", "required": ["element_id"], "properties": {"element_id": {"type": "string"}}},
          {"type": "object", "required": ["x","y"], "properties": {"x": {"type": "integer"}, "y": {"type": "integer"}}}
        ]
      },
      "button": {"enum": ["left", "right", "middle", "x1", "x2"], "default": "left"},
      "clicks": {"type": "integer", "minimum": 1, "maximum": 3, "default": 1},
      "modifiers": {"type": "array", "items": {"enum": ["ctrl","shift","alt","super"]}},
      "curve": {"$ref": "#/$defs/AimCurve", "default": "EaseInOut"},
      "duration_ms": {"type": "integer", "default": 80, "description": "Cursor travel time when moving to target"},
      "backend": {"enum": ["software","vigem","hardware","auto"], "default": "auto"},
      "use_invoke_pattern": {"type": "boolean", "default": true, "description": "When target is an element_id with Invoke support, use semantic invoke (no cursor move)"}
    },
    "required": ["target"]
  }
}
```

Coordinate targets (`x`, `y`) are physical pixels (DPI-aware), matching UI Automation bounding boxes and per-monitor-DPI-aware `GetCursorPos`; see [03_action.md §13](03_action.md#13-click-on-element-semantics).

### 3.12 `act_type`

```json
{
  "name": "act_type",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["text"],
    "properties": {
      "text": {"type": "string"},
      "into_element": {"type": "string", "description": "Optional element_id; focuses + clears + types"},
      "dynamics": {"enum": ["burst","linear","natural"], "default": "natural"},
      "linear_ms_per_char": {"type": "integer", "default": 30},
      "use_scancodes": {"type": "boolean", "default": false},
      "press_enter_after": {"type": "boolean", "default": false},
      "backend": {"enum": ["software","hardware","auto"], "default": "auto"}
    }
  }
}
```

### 3.13 `act_press`

```json
{
  "name": "act_press",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["keys"],
    "properties": {
      "keys": {"type": "array", "items": {"type": "string"}, "description": "Key names; multiple = chord (e.g., ['ctrl','s'])"},
      "hold_ms": {"type": "integer", "minimum": 1, "maximum": 30000, "default": 33},
      "backend": {"enum": ["software","hardware","auto"], "default": "auto"}
    }
  }
}
```

Key name vocabulary: standard symbolic names (`a`..`z`, `0`..`9`, `f1`..`f24`, `up`, `down`, `enter`, `space`, `tab`, `esc`, `ctrl`, `shift`, `alt`, `super`, `lmb`, `rmb`, `mmb`, etc.). Per-game profile may extend (e.g., `medkit` → bound to whatever key is configured in that game).

### 3.14 `act_aim`

```json
{
  "name": "act_aim",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["target"],
    "properties": {
      "target": {
        "oneOf": [
          {"type": "object", "required": ["x","y"], "properties": {"x":{"type":"integer"}, "y":{"type":"integer"}}},
          {"type": "object", "required": ["element_id"], "properties": {"element_id":{"type":"string"}}},
          {"type": "object", "required": ["track_id"], "properties": {"track_id":{"type":"integer"}}}
        ]
      },
      "style": {"enum": ["snap","flick","natural","track"], "default": "snap"},
      "deadline_ms": {"type": "integer", "default": 80},
      "backend": {"enum": ["software","hardware","auto"], "default": "auto"}
    }
  }
}
```

`style: "track"` registers an aim_track reflex and returns its `reflex_id` instead of completing immediately. Cancel via `reflex_cancel`.
Screen-point targets (`x`, `y`) are physical pixels (DPI-aware), matching UI Automation bounding boxes and per-monitor-DPI-aware `GetCursorPos`; see [03_action.md §13](03_action.md#13-click-on-element-semantics).

### 3.15 `act_drag`

```json
{
  "name": "act_drag",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["from","to"],
    "properties": {
      "from": {"type": "object", "properties": {"x":{}, "y":{}, "element_id":{}}},
      "to":   {"type": "object", "properties": {"x":{}, "y":{}, "element_id":{}}},
      "button": {"enum": ["left","right","middle"], "default": "left"},
      "curve": {"$ref": "#/$defs/AimCurve", "default": "EaseInOut"},
      "duration_ms": {"type": "integer", "default": 200},
      "backend": {"enum": ["software","hardware","auto"], "default": "auto"}
    }
  }
}
```

Coordinate `from` / `to` endpoints are physical pixels (DPI-aware), matching UI Automation bounding boxes and per-monitor-DPI-aware `GetCursorPos`; see [03_action.md §13](03_action.md#13-click-on-element-semantics).

### 3.16 `act_scroll`

```json
{
  "name": "act_scroll",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "dy": {"type": "integer", "default": 0, "description": "Positive = down, negative = up; units are wheel ticks"},
      "dx": {"type": "integer", "default": 0},
      "at": {"type": "object", "properties": {"x":{}, "y":{}}, "description": "Optional cursor target before scrolling"},
      "smooth": {"type": "boolean", "default": false, "description": "Split into multiple small wheel events for animation"}
    }
  }
}
```

The optional `at` cursor target is in physical pixels (DPI-aware), matching UI Automation bounding boxes and per-monitor-DPI-aware `GetCursorPos`; see [03_action.md §13](03_action.md#13-click-on-element-semantics).

### 3.17 `act_pad`

Drives a virtual or hardware gamepad.

```json
{
  "name": "act_pad",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["pad_id"],
    "properties": {
      "pad_id": {"type": "integer", "default": 0},
      "report": {
        "type": "object",
        "additionalProperties": false,
        "properties": {
          "buttons": {"type": "array", "items": {"enum": ["a","b","x","y","lb","rb","ls","rs","back","start","up","down","left","right"]}},
          "thumb_l": {"type": "array", "items": {"type":"number"}, "minItems":2, "maxItems":2},
          "thumb_r": {"type": "array", "items": {"type":"number"}, "minItems":2, "maxItems":2},
          "lt": {"type": "number", "minimum": 0, "maximum": 1},
          "rt": {"type": "number", "minimum": 0, "maximum": 1}
        }
      },
      "backend": {"enum": ["vigem","hardware"], "default": "vigem"},
      "hold_ms": {"type": "integer", "description": "If set, applies report for this duration then returns to neutral"}
    }
  }
}
```

### 3.18 `act_combo` (M4)

Schedules a one-shot timed combo through the reflex scheduler. The tool is an
M4 addition and must appear in `tools/list` only when its real runtime path is
implemented.

```json
{
  "name": "act_combo",
  "description": "Execute a timed one-shot sequence of already-supported action tools through the reflex combo scheduler.",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["steps"],
    "properties": {
      "steps": {
        "type": "array",
        "minItems": 1,
        "maxItems": 256,
        "items": {
          "type": "object",
          "additionalProperties": false,
          "required": ["at_ms", "action", "params"],
          "properties": {
            "at_ms": {"type": "integer", "minimum": 0},
            "action": {
              "enum": ["act_click", "act_type", "act_press", "act_aim", "act_drag", "act_scroll", "act_pad", "act_clipboard", "release_all"]
            },
            "params": {"type": "object", "description": "Validated against the selected action tool's schema before scheduling"},
            "backend": {"enum": ["software", "hardware", "vigem", "auto"]}
          }
        }
      },
      "backend": {"enum": ["software", "hardware", "vigem", "auto"], "default": "auto"},
      "idempotency_key": {"type": "string"}
    }
  }
}
```

Returns:

```json
{
  "combo_id": "uuid",
  "scheduled_steps": 3,
  "backend": "auto",
  "started_at_ms": 0
}
```

Required permissions:

- `WRITE_REFLEX`
- `INPUT_KEYBOARD`, `INPUT_MOUSE`, `INPUT_PAD`, and/or `INPUT_HARDWARE_HID`
  according to the nested step actions and chosen backend.

Rules:

- `at_ms` values must be monotonic.
- `act_run_shell`, `act_launch`, subscription tools, storage diagnostics, and
  profile writes are not valid combo steps.
- Step `params` must pass the selected action tool's own schema before the
  combo is scheduled.
- Unknown-scope profile gates still apply; denied action emission returns
  `SAFETY_PROFILE_ACTION_DENIED`.

Errors: `TOOL_PARAMS_INVALID`, `SAFETY_PERMISSION_DENIED`,
`SAFETY_PROFILE_ACTION_DENIED`, `ACTION_BACKEND_UNAVAILABLE`,
`ACTION_HID_PORT_DISCONNECTED`, `ACTION_QUEUE_FULL`,
`REFLEX_ACTION_PERMISSION_DENIED`.

### 3.19 `act_clipboard`

```json
{
  "name": "act_clipboard",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "verb": {"enum": ["read","write","clear"]},
      "text": {"type": "string", "description": "Required for write"},
      "format": {"enum": ["text","unicode"], "default": "unicode"}
    },
    "required": ["verb"]
  }
}
```

### 3.20 `act_run_shell` (M4)

Runs an allowlisted local shell command. Disabled unless `--allow-shell
<regex>` was passed at startup. Broad allowlists such as `.*` are rejected at
startup per `11_security_and_safety.md`; accepted shell patterns must be
full-command-line anchored and must not match empty input.

```json
{
  "name": "act_run_shell",
  "description": "Run a local shell command only when the startup allowlist permits the resolved command line.",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["command"],
    "properties": {
      "command": {"type": "string", "minLength": 1},
      "args": {"type": "array", "items": {"type": "string"}, "default": []},
      "working_dir": {"type": "string"},
      "env": {"type": "object", "additionalProperties": {"type": "string"}, "default": {}},
      "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 600000, "default": 30000},
      "idempotency_key": {"type": "string"}
    }
  }
}
```

Returns:

```json
{
  "exit_code": 0,
  "stdout": "...",
  "stderr": "...",
  "duration_ms": 152,
  "timed_out": false,
  "stdout_truncated": false,
  "stderr_truncated": false
}
```

Required policy gate: startup `--allow-shell <regex>` must match the resolved
command line. M4 may add a dedicated permission enum later; until then the
allowlist is the required permission surface.

Rules:

- `command + args` are resolved into a command line before allowlist matching.
- Shell allowlist patterns are validated at startup and rejected if empty,
  unanchored, matching empty, or using catch-all any-character repetition such
  as `.*` / `.+`.
- `env` defaults to `{}` and extends a restricted child environment containing
  only `PATH`, `USERPROFILE`, `TEMP`, `SystemRoot`, plus any variables the child
  interpreter synthesizes after launch.
- `working_dir` defaults to the daemon's current directory if omitted.
- Stdout/stderr are capped at 1 MiB each and report truncation flags.
- Timeout kills the subprocess and returns `timed_out: true` with captured
  output up to the cap.

Errors: `TOOL_PARAMS_INVALID`, `SAFETY_SHELL_DENIED_BY_POLICY`,
`SAFETY_PERMISSION_DENIED`, `ACTION_TARGET_INVALID`.

### 3.21 `act_launch` (M4)

Launches an allowlisted local process and optionally waits for a window title.
Disabled unless `--allow-launch <regex>` was passed at startup. Broad allowlists
such as `.*` are rejected at startup per `11_security_and_safety.md`.

```json
{
  "name": "act_launch",
  "description": "Launch an allowlisted local executable and optionally wait for a matching window title.",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["target"],
    "properties": {
      "target": {"type": "string", "description": "Executable name (e.g., 'notepad.exe') or Steam appid (e.g., 'steam://run/440')"},
      "args": {"type": "array", "items": {"type": "string"}, "default": []},
      "working_dir": {"type": "string"},
      "env": {"type": "object", "additionalProperties": {"type": "string"}, "default": {}},
      "wait_for_window_title_regex": {"type": "string"},
      "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 600000, "default": 10000},
      "idempotency_key": {"type": "string"}
    }
  }
}
```

Returns:

```json
{
  "pid": 12345,
  "hwnd": 67890,
  "matched_title": "Untitled - Notepad",
  "launched_at": "...",
  "reason": null
}
```

If a window wait was requested but no matching title appears before the
timeout, the process launch can still succeed with:

```json
{
  "pid": 12345,
  "hwnd": null,
  "matched_title": null,
  "launched_at": "...",
  "reason": "no_match_within_timeout"
}
```

Required policy gate: startup `--allow-launch <regex>` must match the resolved
command line made from `target` plus `args` using the same quoting rules as
`act_run_shell`. M4 may add a dedicated permission enum later; until then the
allowlist is the required permission surface.

Rules:

- `args` defaults to `[]`.
- `env` defaults to `{}` and extends a restricted child environment containing
  only `PATH`, `USERPROFILE`, `TEMP`, `SystemRoot`.
- `working_dir` defaults to the daemon's current directory if omitted.
- On Windows, path-like `target` values are resolved through Win32
  `GetLongPathNameW` before allowlist matching. Bare executable names such as
  `javaw.exe` and URI targets such as `steam://run/...` match as provided.
- `wait_for_window_title_regex` is optional. When present, the tool reads real
  window state until `timeout_ms` expires.
- `reason` is `null` on full process/window success, `no_match_within_timeout`
  when the launch succeeds but the optional title wait times out, and
  `window_readback_unavailable` when the host window-readback layer is
  unavailable.

Errors: `TOOL_PARAMS_INVALID`, `SAFETY_LAUNCH_DENIED_BY_POLICY`,
`SAFETY_PERMISSION_DENIED`, `ACTION_TARGET_INVALID`.

### 3.22 `reflex_register`

```json
{
  "name": "reflex_register",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["kind","params"],
    "properties": {
      "kind": {"enum": ["aim_track","hold_move","hold_button","combo","on_event"]},
      "params": {"type": "object"},
      "priority": {"type": "integer", "minimum": 0, "maximum": 1000, "default": 100},
      "lifetime": {"$ref": "#/$defs/ReflexLifetime"},
      "exclusive": {"type": "boolean", "default": false}
    }
  }
}
```

Returns `{ "reflex_id": "..." }`.

### 3.23 `reflex_cancel`

```json
{
  "name": "reflex_cancel",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["reflex_id"],
    "properties": {"reflex_id": {"type": "string"}}
  }
}
```

### 3.24 `reflex_list`

Returns all currently-active reflexes for this session with state.

### 3.25 `reflex_history`

Returns past reflex events (fires, cancellations, lifetime expiries, conflicts) within a time window.

```json
{
  "name": "reflex_history",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "since_ms_ago": {"type": "integer", "default": 60000},
      "reflex_id": {"type": "string"},
      "kinds": {"type": "array", "items": {"type": "string"}}
    }
  }
}
```

### 3.26 `release_all`

```json
{"name": "release_all", "input_schema": {"type": "object", "additionalProperties": false}}
```

Returns count of released inputs by class.

### 3.27 `profile_list`

```json
{"name": "profile_list", "input_schema": {"type": "object", "additionalProperties": false, "properties": {"filter": {"type":"string"}}}}
```

Returns:

```json
{
  "profiles": [
    {"id": "minecraft.java", "label": "Minecraft Java Edition", "match": {"exe": "javaw.exe", "title_regex": "Minecraft"}},
    {"id": "vscode", "label": "Visual Studio Code", "match": {"exe": "Code.exe"}}
  ],
  "active": "vscode"
}
```

### 3.28 `profile_activate`

```json
{
  "name": "profile_activate",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["profile_id"],
    "properties": {"profile_id": {"type": "string"}}
  }
}
```

### 3.28a `profile_authoring_generate`

Generates a local candidate profile patch from real replay/audit evidence and
writes it as a separate candidate row. The physical SoT is `CF_PROFILES` key
`profile_authoring/v1/candidate/<candidate_id>`. The candidate is never an
active profile; approval only changes candidate state.

```json
{
  "name": "profile_authoring_generate",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["profile_id"],
    "properties": {
      "profile_id": {"type": "string"},
      "replay_path": {"type": "string"},
      "max_audit_rows": {"type": "integer", "minimum": 0, "maximum": 10000, "default": 500},
      "max_replay_rows": {"type": "integer", "minimum": 0, "maximum": 10000, "default": 500},
      "candidate_id": {"type": "string"}
    }
  }
}
```

Evidence can add matches, HUD fields, keymaps, backend defaults, detection
classes, reflex combo proposals, use-scope hints, and safety metadata. The
stored row contains source CF names, replay path, source audit keys/ids,
evidence hash, evidence summary, expected improvement, generated timestamp,
candidate state, patch, and a safety review. It fails closed with
`PROFILE_AUTHORING_INSUFFICIENT_EVIDENCE`,
`PROFILE_AUTHORING_CONFLICTING_EVIDENCE`, or
`PROFILE_AUTHORING_UNSAFE_ESCALATION` before writing a candidate row.

### 3.28b `profile_authoring_list`

Lists local profile-authoring candidates from `CF_PROFILES` without activating
or mutating them.

```json
{
  "name": "profile_authoring_list",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "profile_id": {"type": "string"},
      "state": {"enum": ["candidate", "accepted", "rejected"]},
      "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
    }
  }
}
```

Returns the `CF_PROFILES` prefix, filters, total matched count, and bounded
candidate summaries with row keys, state, evidence hash, expected improvement,
and stored value size.

### 3.28c `profile_authoring_inspect`

Reads one local candidate row by candidate id.

```json
{
  "name": "profile_authoring_inspect",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["candidate_id"],
    "properties": {"candidate_id": {"type": "string"}}
  }
}
```

Returns `found=false` for a missing row. Found rows return the full stored
candidate plus the summary used by `profile_authoring_list`.

### 3.28d `profile_authoring_accept`

Marks a candidate row accepted and reads it back. It does not activate,
install, or overwrite any active/bundled profile.

```json
{
  "name": "profile_authoring_accept",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["candidate_id"],
    "properties": {
      "candidate_id": {"type": "string"},
      "operator_note": {"type": "string"}
    }
  }
}
```

Accepting from `candidate` writes `state="accepted"`, `accepted_at_ns`, and the
optional note. Re-accepting an already accepted row is idempotent. Any other
state fails closed with `PROFILE_AUTHORING_INVALID_STATE`.

### 3.28e `profile_authoring_reject`

Marks a candidate row rejected and reads it back.

```json
{
  "name": "profile_authoring_reject",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["candidate_id"],
    "properties": {
      "candidate_id": {"type": "string"},
      "reason": {"type": "string"}
    }
  }
}
```

Rejecting from `candidate` writes `state="rejected"`, `rejected_at_ns`, and the
optional reason. Re-rejecting an already rejected row is idempotent. Any other
state fails closed with `PROFILE_AUTHORING_INVALID_STATE`.

### 3.28f `profile_authoring_export`

Writes one candidate as a local JSON export file after reading the candidate
from `CF_PROFILES`.

```json
{
  "name": "profile_authoring_export",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["candidate_id", "output_path"],
    "properties": {
      "candidate_id": {"type": "string"},
      "output_path": {"type": "string"}
    }
  }
}
```

The bundle contains the schema version, export timestamp, source CF/key, and
the full candidate row. The tool parses the written file before returning so a
manual FSV run can separately inspect both the candidate row and the export
file bytes.

### 3.28g `profile_quality_refresh`

Refreshes the local profile-registry quality snapshot for one profile from real
stored action audit rows. This is a local-only read/aggregate/write/readback
surface: it scans `CF_ACTION_LOG`, writes the redacted snapshot to
`CF_PROFILES` at `profile_quality/v1/<profile_id>`, then reads that exact row
back before returning.

```json
{
  "name": "profile_quality_refresh",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["profile_id"],
    "properties": {
      "profile_id": {"type": "string"},
      "max_audit_rows": {"type": "integer", "minimum": 1, "maximum": 50000, "default": 5000},
      "stale_after_ns": {"type": "integer", "minimum": 1, "default": 86400000000000}
    }
  }
}
```

Returns the `CF_PROFILES` key, whether a new snapshot was written, stored value
length/prefix, and an explainable snapshot containing source row counts,
ignored corrupt/stale rows, quality counts/rates, Wilson lower-bound score,
compatibility counters, profile-schema-version recency/mixed-version counters,
redaction policy, and contribution policy. Export is always `false`; sharing
requires a future explicit operator-approved path.

### 3.28h `profile_registry_search`

Searches local registry rows under `profile_registry/v1/` in `CF_PROFILES`.
This is the operator-facing list/search readback for source/package/profile/
installed/compatibility/quality-link rows.

```json
{
  "name": "profile_registry_search",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "query": {"type": "string"},
      "row_kind": {"type": "string"},
      "include_disabled": {"type": "boolean", "default": false},
      "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
    }
  }
}
```

Returns `cf_name`, `prefix`, filters, `total_matched`, and row summaries with
UTF-8 keys, hex keys, row kind/id, state, profile/package ids, update time, and
bounded value prefix.

### 3.28i `profile_registry_inspect`

Reads one registry row by exact key or derived id. `profile_registry/v1/head/*`
keys read `CF_KV`; all other `profile_registry/v1/*` keys read `CF_PROFILES`.

```json
{
  "name": "profile_registry_inspect",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "row_key": {"type": "string"},
      "source_id": {"type": "string"},
      "package_id": {"type": "string"},
      "package_version": {"type": "string"},
      "profile_id": {"type": "string"},
      "profile_version": {"type": "string"},
      "installed_profile_id": {"type": "string"}
    }
  }
}
```

Returns `cf_name`, `row_key`, `found`, and when found the full decoded JSON row
plus the same row summary used by search.

### 3.28j `profile_registry_install`

Validates a local profile package manifest, verifies signed package trust when
policy requires it, parses the referenced profile TOML, checks
manifest/profile id agreement, writes local registry rows to `CF_PROFILES`,
writes the source head pointer to `CF_KV`, and reads the written rows back
before returning. If signed trust verification fails, the tool writes a
`profile_package_quarantine` row and returns `PROFILE_TRUST_VERIFICATION_FAILED`
without writing package/profile/installed/head rows.

```json
{
  "name": "profile_registry_install",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["manifest_path"],
    "properties": {
      "manifest_path": {"type": "string"},
      "expected_manifest_digest": {"type": "string"},
      "source_id": {"type": "string", "default": "registry.local"},
      "trust_policy": {"enum": ["local_first", "signed_required"], "default": "local_first"}
    }
  }
}
```

Duplicate package id/version with the same manifest digest is idempotent.
Duplicate id/version with a different digest fails closed with no companion-row
rewrite. The response returns `manifest_digest`, profile TOML path, `wrote_rows`,
`idempotent`, trust/signature status, signer/trust-root readback, row keys, and
row summaries.

### 3.28k `profile_registry_disable`

Marks an installed registry row disabled or removed in `CF_PROFILES` and reads
the updated row back.

```json
{
  "name": "profile_registry_disable",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["profile_id"],
    "properties": {
      "profile_id": {"type": "string"},
      "state": {"enum": ["disabled", "removed"], "default": "disabled"},
      "reason": {"type": "string"}
    }
  }
}
```

Returns previous/current state, the row key, and the decoded stored row.

### 3.28l `profile_registry_export`

Exports local registry rows from `CF_PROFILES` and `CF_KV` into a JSON bundle on
disk. The bundle is a local file artifact and is not a consent/share path.

```json
{
  "name": "profile_registry_export",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["output_path"],
    "properties": {
      "output_path": {"type": "string"},
      "query": {"type": "string"},
      "row_kind": {"type": "string"},
      "include_disabled": {"type": "boolean", "default": false},
      "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
    }
  }
}
```

Returns output path, bytes written, exported row count, and row summaries.

### 3.28m `profile_registry_import`

Imports a local JSON registry bundle after validating schema version, supported
CF names, `profile_registry/v1/` key namespace, and object-valued rows.

```json
{
  "name": "profile_registry_import",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["bundle_path"],
    "properties": {"bundle_path": {"type": "string"}}
  }
}
```

Returns read row count, per-CF write counts, and summaries for the imported
rows.

### 3.28n `profile_registry_rollback`

Rewrites `profile_registry/v1/installed/<profile_id>` to a prior active package
whose package row is `trusted` or `local_validated`, and writes a
`profile_registry_rollback` row under `profile_registry/v1/rollback/`.

```json
{
  "name": "profile_registry_rollback",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["profile_id"],
    "properties": {
      "profile_id": {"type": "string"},
      "target_package_id": {"type": "string"},
      "target_package_version": {"type": "string"},
      "reason": {"type": "string"}
    }
  }
}
```

If no target is supplied, the tool selects the newest prior known-good package
for the profile. It fails closed with `PROFILE_ROLLBACK_UNAVAILABLE` if no
known-good target exists, the target is current, revoked, quarantined, or not
trusted/local-validated. The response includes the previous and rolled-back
package id/version plus readback of both the installed and rollback rows. The
installed row readback must carry the rolled-back package's trust/signature
metadata, not stale metadata from the package being replaced.

### 3.28o `audit_intelligence_query`

Summarizes profile-linked outcomes across the audit SoTs now populated by
profile activation, action, and reflex paths.

```json
{
  "name": "audit_intelligence_query",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["profile_id"],
    "properties": {
      "profile_id": {"type": "string"},
      "max_rows": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
    }
  }
}
```

Reads newest rows from `CF_ACTION_LOG`, `CF_EVENTS`, `CF_REFLEX_AUDIT`, and
`CF_SESSIONS`, reads `CF_PROFILES` quality snapshot
`profile_quality/v1/<profile_id>` when present, and returns bucket counts by
status/tool/kind/error code plus learning candidates.

### 3.28p `audit_export_consent_set`

Writes the local consent state required before any audit export bundle can be
created. The physical SoT is `CF_KV` key
`audit_export/v1/consent/<profile_id>`. The tool writes the row and immediately
reads that same key back before returning.

```json
{
  "name": "audit_export_consent_set",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["profile_id", "enabled"],
    "properties": {
      "profile_id": {"type": "string"},
      "enabled": {"type": "boolean"},
      "redaction_policy": {"type": "string", "default": "strict"},
      "operator_note": {"type": "string"}
    }
  }
}
```

The stored row includes `row_kind="audit_export_consent"`, enabled/disabled
state, selected policy, allowed policies, `external_sharing_allowed=false`, and
operator note. Unsupported policies fail closed with
`AUDIT_EXPORT_REDACTION_REQUIRED`.

### 3.28q `audit_export_bundle`

Creates a local, redacted audit export bundle only after consent and redaction
policy verification. The trigger reads `CF_KV` consent state and newest
`CF_ACTION_LOG` rows for the requested profile, redacts sensitive fields, and
writes an operator-visible directory containing:

- `manifest.json`
- `rows.json`
- `redaction_report.json`

```json
{
  "name": "audit_export_bundle",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["profile_id", "output_path"],
    "properties": {
      "profile_id": {"type": "string"},
      "output_path": {"type": "string"},
      "redaction_policy": {"type": "string"},
      "max_rows": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100},
      "max_row_bytes": {"type": "integer", "minimum": 1, "maximum": 524288, "default": 65536}
    }
  }
}
```

`redaction_policy` is runtime-required even though it is optional in the schema
so callers must consciously select the policy consented by the operator. The
only current policy is `strict`. Strict redaction removes window titles, paths,
command lines, exact timing fields, OCR/text/clipboard/transcript fields,
screenshots/images/pixels, user identifiers, and high-cardinality IDs while
retaining bounded signals such as profile id/version/schema, process name,
tool, status, error code, and backend.

The manifest records schema version, bundle kind, source CF, consent key and
hash, redaction policy, row/file hashes, row counts, and
`external_sharing_allowed=false`. The redaction report records counts by
redaction class and the fail-closed rules. Missing/disabled consent returns
`AUDIT_EXPORT_CONSENT_REQUIRED`; missing/unsupported policy returns
`AUDIT_EXPORT_REDACTION_REQUIRED`; a matching row larger than `max_row_bytes`
returns `AUDIT_EXPORT_PAYLOAD_TOO_LARGE` before bundle files are written.

### 3.29 `health`

```json
{"name": "health", "input_schema": {"type": "object", "additionalProperties": false}}
```

Returns:

```json
{
  "ok": true,
  "subsystems": {
    "storage": {"status": "ok", "db_path": "...", "schema_version": 7, "cf_sizes": {"CF_REFLEX_AUDIT": 4096}},
    "reflex": {"status": "ok", "active_count": 2, "last_tick_jitter_us": 180, "recursion_clamps_total": 0},
    "profiles": {"status": "ok", "active_profile_id": "notepad", "profile_count": 4, "last_reload_at": "1779723537765"},
    "audio": {"status": "disabled", "ring_buffer_seconds": 5, "stt_model_loaded": false},
    "http": {"status": "ok", "bind_addr": "127.0.0.1:7700", "active_sessions": 1, "sse_subscribers": 0}
  },
  "version": "0.1.0",
  "uptime_s": 1245
}
```

M3 subsystem status strings are `initializing`, `ok`, `degraded_latency`,
`disk_pressure_l1`..`disk_pressure_l4`, `disabled`, or `error`.

### 3.30 `replay_record`

```json
{
  "name": "replay_record",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["verb"],
    "properties": {
      "verb": {"enum": ["start","stop","status"]},
      "session_id": {"type": "string"}
    }
  }
}
```

### 3.31 `storage_inspect`

Operator-facing storage readback for manual FSV. This reads exact row counts,
logical byte sizes, bounded newest-row samples, schema version, and in-process
disk-pressure transition codes from the live RocksDB-backed runtime.

```json
{"name": "storage_inspect", "input_schema": {"type": "object", "additionalProperties": false}}
```

Returns:

```json
{
  "schema_version": 1,
  "pressure_level": {"name": "Normal", "value": 0},
  "pressure_transition_codes": [],
  "audit_retention_policies": [
    {
      "audit_class": "actions",
      "cf_name": "CF_ACTION_LOG",
      "key_prefix": null,
      "ttl": "24h",
      "ttl_ns": 86400000000000,
      "dedupe_key_fields": ["profile_id", "tool", "status", "error_code"],
      "pressure_preserve": false,
      "strategic": false
    }
  ],
  "cf_row_counts": {"CF_EVENTS": 4},
  "cf_sizes": {"CF_EVENTS": 248},
  "cf_row_samples": {
    "CF_ACTION_LOG": [
      {
        "key_hex": "1870e8a94f2b000000000001",
        "value_len_bytes": 512,
        "value_utf8_prefix": "{\"tool\":\"act_press\",\"status\":\"ok\"",
        "value_truncated": true
      }
    ]
  }
}
```

`cf_row_samples` is a bounded newest-row readback for manual FSV. It is not an
automation substitute; agents still define the Source of Truth, trigger the
real runtime surface, then read and record the physical row data they expect to
change.

### 3.32 `storage_put_probe_rows`

Writes bounded synthetic rows to a small allow-list of diagnostic column
families (`CF_EVENTS`, `CF_OBSERVATIONS`, `CF_SESSIONS`, `CF_ACTION_LOG`,
`CF_KV`) and flushes them. This exists so manual storage FSV can use known
synthetic inputs and then read the physical storage state through
`storage_inspect`; `CF_ACTION_LOG` probe rows are deliberately useful for
corrupt-audit-row edge checks because profile quality scoring ignores malformed
rows for score changes.

```json
{
  "name": "storage_put_probe_rows",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["cf_name", "key_prefix", "rows", "value_bytes"],
    "properties": {
      "cf_name": {"type": "string"},
      "key_prefix": {"type": "string", "maxLength": 128},
      "rows": {"type": "integer", "minimum": 0, "maximum": 10000},
      "value_bytes": {"type": "integer", "minimum": 0, "maximum": 65536},
      "value_json": {"type": "object"},
      "ts_ns_start": {"type": "integer"},
      "ts_ns_step": {"type": "integer"}
    }
  }
}
```

When `value_json` is supplied, the tool writes that JSON object as the row
value, adding deterministic `probe_id`, `seq`, and optional `ts_ns`/`audit_id`
from `ts_ns_start` + `ts_ns_step` only when those fields are absent. This keeps
manual storage/audit FSV on the real MCP runtime surface while still using
known synthetic input rows.

### 3.33 `storage_gc_once`

Runs one row-cap GC pass for a diagnostic column family. When `cf_name` is
`AUDIT_RETENTION`, the same tool runs the M5 audit-data retention path without
adding another MCP tool: it classifies profile-linked audit rows, preserves
unknown-schema rows, backfills missing top-level `profile_id` /
`profile_schema_version` from existing M3/M4 audit context, dedupes repeated
outcomes, applies row caps, and writes a durable report row to
`CF_KV/audit_retention/v1/report/<run_id>`. Manual FSV reads `storage_inspect`
before, calls this trigger, then reads `storage_inspect` and the report row
afterward.
Retention backfills and report rows use the storage-maintenance write path, so
Level3/Level4 disk pressure cannot silently drop the migration/report evidence;
ordinary probe and ingestion writes remain pressure-gated.

```json
{
  "name": "storage_gc_once",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["cf_name", "soft_cap_rows", "hard_cap_rows"],
    "properties": {
      "cf_name": {"type": "string"},
      "soft_cap_rows": {"type": "integer", "minimum": 1, "maximum": 1000000},
      "hard_cap_rows": {"type": "integer", "minimum": 1, "maximum": 1000000},
      "run_id": {"type": "string"},
      "now_ns": {"type": "integer"},
      "max_age_ns": {"type": "integer"},
      "dedupe_window_ns": {"type": "integer"},
      "profile_id": {"type": "string"}
    }
  }
}
```

`run_id`, `now_ns`, `max_age_ns`, `dedupe_window_ns`, and `profile_id` are
valid only with `cf_name="AUDIT_RETENTION"`. The response then includes
`audit_retention_report_key` and an `audit_retention` report with before/after
row counts, per-CF scanned/deleted/backfilled/unknown-schema counts,
`dedupe_keys`, and the readback report state that was actually persisted to
`CF_KV`.

### 3.34 `storage_pressure_sample`

Applies one synthetic free-byte sample through the production disk-pressure
responder. Manual FSV must separately read `storage_inspect` and logs after each
sample to confirm the pressure level and emitted transition code.

```json
{
  "name": "storage_pressure_sample",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["free_bytes"],
    "properties": {"free_bytes": {"type": "integer", "minimum": 0}}
  }
}
```

---

## 4. Errors

All errors follow JSON-RPC `error` shape:

```json
{
  "jsonrpc": "2.0",
  "id": <request_id>,
  "error": {
    "code": -32099,
    "message": "Human-readable summary",
    "data": {
      "code": "ACTION_QUEUE_FULL",
      "tool": "act_click",
      "details": {"queue_depth": 256, "limit": 256},
      "retry_after_ms": 50
    }
  }
}
```

`code: SCREAMING_SNAKE_CASE` in `data.code` is the stable identifier; numeric JSON-RPC `code` is always `-32099` (server-defined error) or `-32602` (invalid params).

Full error code catalog in `06_data_schemas.md` §Error codes.

---

## 5. M4/M5 Default Resolution Rows

The `tools/list` schema must expose the JSON defaults below once these M4 tools
land and for M5 tools whose safety behavior depends on caller-visible defaults.
Rows that say "required", "omitted", or "inherits" define runtime resolution
behavior rather than a JSON-Schema `default` value. Issue #448 owns the M4
default-resolution readback; current `tools/list` snapshots also pin the M5
profile-authoring and audit-export defaults below.

| Tool | Field | Default | Source |
|---|---|---|---|
| `act_combo` | `steps` | required; no default | M4 plan #444 |
| `act_combo` | `steps[].backend` | inherits top-level `backend` | M4 plan #444 |
| `act_combo` | `backend` | `"auto"` | M4 plan #444 |
| `act_combo` | `idempotency_key` | omitted | M4 plan #444 |
| `act_run_shell` | `command` | required; no default | M4 plan #444 |
| `act_run_shell` | `args` | `[]` | M4 plan #444 |
| `act_run_shell` | `working_dir` | daemon current directory | security policy #11 |
| `act_run_shell` | `env` | `{}` | M4 plan #444 |
| `act_run_shell` | `timeout_ms` | `30000` | PRD 05 + performance budget |
| `act_run_shell` | `idempotency_key` | omitted | PRD 05 design rule 6 |
| `act_launch` | `target` | required; no default | M4 plan #444 |
| `act_launch` | `args` | `[]` | M4 plan #444 |
| `act_launch` | `working_dir` | target resolver default | security policy #11 |
| `act_launch` | `env` | `{}` | M4 plan #444 |
| `act_launch` | `wait_for_window_title_regex` | omitted | M4 plan #444 |
| `act_launch` | `timeout_ms` | `10000` | PRD 05 |
| `act_launch` | `idempotency_key` | omitted | PRD 05 design rule 6 |
| `profile_authoring_generate` | `profile_id` | required; no default | M5 issue #462 |
| `profile_authoring_generate` | `replay_path` | omitted | M5 issue #462 |
| `profile_authoring_generate` | `max_audit_rows` | `500` | M5 issue #462 |
| `profile_authoring_generate` | `max_replay_rows` | `500` | M5 issue #462 |
| `profile_authoring_generate` | `candidate_id` | omitted; generated from evidence hash | M5 issue #462 |
| `profile_authoring_list` | `profile_id` | omitted | M5 issue #462 |
| `profile_authoring_list` | `state` | omitted | M5 issue #462 |
| `profile_authoring_list` | `limit` | `100` | M5 issue #462 |
| `profile_authoring_inspect` | `candidate_id` | required; no default | M5 issue #462 |
| `profile_authoring_accept` | `candidate_id` | required; no default | M5 issue #462 |
| `profile_authoring_accept` | `operator_note` | omitted | M5 issue #462 |
| `profile_authoring_reject` | `candidate_id` | required; no default | M5 issue #462 |
| `profile_authoring_reject` | `reason` | omitted | M5 issue #462 |
| `profile_authoring_export` | `candidate_id` | required; no default | M5 issue #462 |
| `profile_authoring_export` | `output_path` | required; no default | M5 issue #462 |
| `audit_export_consent_set` | `profile_id` | required; no default | M5 issue #460 |
| `audit_export_consent_set` | `enabled` | required; no default | M5 issue #460 |
| `audit_export_consent_set` | `redaction_policy` | `"strict"` | M5 issue #460 |
| `audit_export_consent_set` | `operator_note` | omitted | M5 issue #460 |
| `audit_export_bundle` | `profile_id` | required; no default | M5 issue #460 |
| `audit_export_bundle` | `output_path` | required; no default | M5 issue #460 |
| `audit_export_bundle` | `redaction_policy` | runtime-required; omitted by schema | M5 issue #460 |
| `audit_export_bundle` | `max_rows` | `100` | M5 issue #460 |
| `audit_export_bundle` | `max_row_bytes` | `65536` | M5 issue #460 |

All listed schemas must serialize as closed top-level JSON objects with
`additionalProperties: false`. `act_combo.steps[]` also serializes as a closed
object. The nested `act_combo.steps[].params` object is validated against the
selected action tool schema before scheduling; it is not accepted as unchecked
free-form input.

---

## 6. Transports

| Transport | When | Capabilities |
|---|---|---|
| stdio | Local agent client (Claude Desktop, Codex CLI) | Tool calls + push notifications via the same stream |
| Streamable HTTP | Remote / multi-agent | Tool calls (POST JSON), push events (SSE upgrade on GET), session via `Mcp-Session-Id` header |

Server binary selects via `--mode {stdio|http}`. `--bind 127.0.0.1:7700` for HTTP; default bind is localhost-only.

For long-running tools (`describe` with VLM inference, `audio_transcribe`), server upgrades to SSE and emits progress notifications:

```json
{"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":"...","progress":50,"total":100,"message":"Inference 50% done"}}
```

Final response arrives as a normal JSON-RPC reply when complete.

---

## 7. Session model

`initialize` creates a session. Server returns `Mcp-Session-Id` (HTTP) or implicitly tracks the stdio process. Per-session state:

- Active capture target
- Active perception mode
- Active profile
- Open subscriptions
- Registered reflexes
- Allowed back-ends (`vigem`, `hardware`)
- Allow-listed shell patterns

Closing the session (or process exit) cancels all subscriptions and reflexes, releases all held inputs (via `release_all`), and persists session metadata to `CF_SESSIONS`.

`delete` request to the MCP endpoint with session id explicitly tears down the session.

---

## 8. Push event format

On subscription fire, server emits one notification/SSE frame per event:

```json
{
  "jsonrpc": "2.0",
  "method": "synapse/event",
  "params": {
    "subscription_id": "...",
    "stream_seq": 1,
    "lossy": false,
    "event": {
      "seq": 123456,
      "at": "2026-05-22T15:00:00Z",
      "source": "perception.detection",
      "kind": "entity_appeared",
      "data": {"track_id": 42, "class_label": "enemy", "bbox": {"x":820,"y":340,"w":60,"h":130}, "confidence": 0.87}
    }
  }
}
```

Agent's MCP client must implement notification handling (most do natively).
`stream_seq` is the per-subscription SSE resume sequence. `event.seq` remains
the domain event sequence.

For resumability over HTTP, each SSE event carries `id: <stream_seq>`. On
reconnect, client sends `Last-Event-ID: <stream_seq>` and server replays
buffered events from there. Buffer depth: 4096 events per subscription. If a
gap or overflow is detected, Synapse sends a `subscription_started` frame with
`lossy: true` before continuing event delivery.

---

## 9. Tool examples (end-to-end)

### Example A — open Notepad, type a paragraph, save

```
→ profile_activate(profile_id="notepad")
→ act_launch(target="notepad.exe", wait_for_window_title_regex="Untitled")
← {pid:..., hwnd:...}
→ observe(include=["focused","elements"])
← Observation with the editor element_id
→ act_type(into_element="<editor_element_id>", text="Hello world.\nThis is Synapse.")
→ act_press(keys=["ctrl","s"])
→ observe(include=["focused","elements"])           # waits implicitly until UIA settles
← Observation with the Save As dialog visible
→ find(query="filename edit field", scope=elements, limit=1)
← {results: [{element_id: "...", role: "Edit"}]}
→ act_type(into_element="<filename_edit_id>", text="C:\\tmp\\demo.txt", press_enter_after=true)
```

8 tool calls, ~2 KB token cost end-to-end.

### Example B — track and shoot in a game

```
→ profile_activate(profile_id="some_fps_singleplayer")
→ observe(include=["entities","hud"])
← entities: [{entity_id: "track:42", class_label: "enemy", ...}]
→ act_aim(target={track_id:42}, style="track")
← {reflex_id: "<r1>"}
→ reflex_register(
    kind="on_event",
    params={
      when:{kind:"detection",match:{track_id:42,inside_crosshair:true}},
      then:{action:"act_press", args:{keys:["lmb"], hold_ms:50}},
      debounce_ms:200
    },
    lifetime={UntilEvent:{kind:"entity_disappeared",track_id:42}}
)
← {reflex_id: "<r2>"}
(... agent waits, reflex runtime handles per-frame ...)
→ subscribe(filter={kind:"entity_disappeared",track_id:42})
← {subscription_id: "..."}
... entity_disappeared notification arrives ...
→ reflex_cancel(reflex_id="<r1>")
```

5 tool calls + 1 push event = under 1 KB tokens, with frame-accurate execution between calls.

---

## 10. Out of scope

- Internal `Action` enum and back-end selection → `03_action.md`
- Reflex semantics in detail → `04_reflex_runtime.md`
- Observation struct fields → `06_data_schemas.md`
- Profile TOML fields → `07_storage_and_profiles.md`
- HTTP transport details → `01_architecture.md` and `rmcp` reference docs
