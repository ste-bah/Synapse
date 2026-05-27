# 05 — MCP Tool Surface

## 1. Design rules

1. **Tool count cap:** M3 shipped 30 live MCP tools. M4 expands the target
   surface to 33 live MCP tools by adding `act_combo`, `act_run_shell`, and
   `act_launch` per the M4 phase plan. Any further agent-facing tools require
   an ADR-approved cap change. Overlapping tools merge. Profile and parameter
   knobs are the escape hatches.
2. **One tool, one verb.** No `do_everything(action_kind, ...)` mega-tools.
3. **Structured input, structured output.** Every tool defines a JSON Schema with `additionalProperties: false`. Every response carries explicit fields, no free-form text.
4. **No silent success.** If a tool did not do the work, it returns an MCP error with `code: SCREAMING_SNAKE_CASE`, never `success: true` with a partial result.
5. **All async; all cancellable.** Long-running tools support progress notifications via Streamable HTTP SSE upgrade.
6. **Idempotency tokens where it matters.** `act_run_shell`, `act_launch`, and similar accept an optional `idempotency_key` for safe retries.
7. **Stable identifiers.** `element_id`, `entity_id`, `track_id`, `reflex_id`, `session_id` are returned by tools and accepted unchanged by subsequent calls. Agent never invents these.

The first 30 tools below are the current live M3 contract. M4 adds rows 31-33.
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

M3 live count: 30 tools. M4 target count: 33 tools.

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
logical byte sizes, schema version, and in-process disk-pressure transition
codes from the live RocksDB-backed runtime.

```json
{"name": "storage_inspect", "input_schema": {"type": "object", "additionalProperties": false}}
```

Returns:

```json
{
  "schema_version": 1,
  "pressure_level": {"name": "Normal", "value": 0},
  "pressure_transition_codes": [],
  "cf_row_counts": {"CF_EVENTS": 4},
  "cf_sizes": {"CF_EVENTS": 248}
}
```

### 3.32 `storage_put_probe_rows`

Writes bounded synthetic rows to a small allow-list of diagnostic column
families (`CF_EVENTS`, `CF_OBSERVATIONS`, `CF_SESSIONS`, `CF_KV`) and flushes
them. This exists so manual storage FSV can use known synthetic inputs and then
read the physical storage state through `storage_inspect`.

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
      "value_bytes": {"type": "integer", "minimum": 0, "maximum": 65536}
    }
  }
}
```

### 3.33 `storage_gc_once`

Runs one row-cap GC pass for a diagnostic column family. Manual FSV reads
`storage_inspect` before, calls this trigger, then reads `storage_inspect` and
the daemon log afterward.

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
      "hard_cap_rows": {"type": "integer", "minimum": 1, "maximum": 1000000}
    }
  }
}
```

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

## 5. M4 Default Resolution Rows

The `tools/list` schema must expose the JSON defaults below once these M4 tools
land. Rows that say "required", "omitted", or "inherits" define runtime
resolution behavior rather than a JSON-Schema `default` value. Issue #448 owns
the future default-resolution readback; this table is the PRD source row for the
three M4 tools.

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

All three schemas must serialize as closed top-level JSON objects with
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
