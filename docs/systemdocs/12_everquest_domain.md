# 12. EverQuest Domain

**Source files covered:**

- `crates/synapse-everquest/Cargo.toml`
- `crates/synapse-everquest/src/lib.rs`
- `crates/synapse-everquest/src/log.rs`
- `crates/synapse-everquest/src/map.rs`
- `crates/synapse-everquest/src/map_inventory.rs`
- `crates/synapse-everquest/src/zone_graph.rs`
- `crates/synapse-mcp/src/server/everquest_tools.rs`
- `crates/synapse-mcp/src/server/everquest_domain.rs`
- `crates/synapse-mcp/src/server/everquest_state.rs`
- `crates/synapse-mcp/src/server/everquest_memory.rs`
- `crates/synapse-mcp/src/server/everquest_outcome.rs`
- `crates/synapse-mcp/src/server/everquest_guard.rs`
- `crates/synapse-mcp/src/server/everquest_autocombat.rs`
- `crates/synapse-mcp/src/server/everquest_route.rs`
- `crates/synapse-mcp/src/server/everquest_map_sensor.rs`
- `crates/synapse-mcp/src/server/everquest_log.rs`
- `crates/synapse-mcp/src/server/everquest_episode_export.rs`
- `crates/synapse-mcp/src/server/everquest_ui_context.rs`
- `crates/synapse-mcp/src/server/everquest_trajectory.rs`
- `crates/synapse-mcp/src/server/everquest_predictive_model.rs`
- `crates/synapse-mcp/src/server/everquest_surprise.rs` (+ `everquest_surprise/{model,compare,validation}.rs`)
- `crates/synapse-mcp/src/server/everquest_scorecard.rs`
- `crates/synapse-mcp/src/server/everquest_world_model.rs` (+ `everquest_world_model/{model,validation}.rs`)
- `crates/synapse-mcp/src/server/everquest_world_summary.rs` (+ `everquest_world_summary/{model,validation}.rs`)
- `crates/synapse-mcp/src/server/everquest_contextgraph.rs`

See [16_api_tools_reference.md](16_api_tools_reference.md) for the consolidated MCP tool reference.

---

## 12.1 Overview

The EverQuest domain is split into two layers:

1. **`synapse-everquest`** — a dependency-light parsing/data crate (deps: `chrono`, `regex`, `schemars`, `serde`, `serde_json`, `sha2`, `thiserror`). It parses EverQuest character logs into compact events/outcomes, parses `.txt` map files, inventories map sets with SHA-256 fingerprints, and builds a static zone graph (nodes, landmarks, segments, zone-transition edges). It performs no I/O against the running game and no MCP work.

2. **MCP integration layer** (`crates/synapse-mcp/src/server/everquest_*`) — a family of MCP tools and supporting modules that build on the crate. They snapshot current state from the live foreground/log/HUD, persist typed `CF_KV` rows (DynamicJEPA transitions, world-model rows, trajectories), gate gameplay actions through a planner guard, run a bounded autocombat loop, plan routes, fit an action-conditioned predictive baseline, detect surprise (predicted vs. observed), score competence, and export/ingest episodes to the ContextGraph long-term-memory system.

The MCP layer is built around a **fail-closed, evidence-bounded** discipline: nearly every row carries an `evidence_boundary` recording that it reads physical state, writes only its own row, does not execute input (except autocombat), and requires manual full-state-verification (FSV) at runtime. Chat bodies and raw target names are redacted by default. The fixed production profile id is `everquest.live`.

---

## 12.2 Log Parsing (`crates/synapse-everquest/src/log.rs`)

### Log file discovery

- `discover_log_files(root)` scans `root/Logs` for files matching `eqlog_<character>_<server>.txt` (`parse_log_file_name` splits on the last `_`). Returns `EverQuestLogFile { path, identity{character,server}, len_bytes }`, sorted by path. Errors fail closed (`InvalidPath` if `Logs` absent, `Io` on metadata failure).

### Line format

Every line is matched by `line_regex`: `^\[(?P<timestamp>[^\]]+)\]\s*(?P<message>.*)$`. The timestamp is parsed with the chrono format `%a %b %d %H:%M:%S %Y` (e.g. `Thu May 28 06:48:10 2026`). Non-matching lines yield `None`; malformed timestamps yield `EverQuestLogError::Timestamp`.

### Two parse surfaces

| Function | Output | Behavior |
|---|---|---|
| `parse_log_line(line)` | `Option<EverQuestLogEvent>` | Classifies into `EverQuestLogKind`; errors closed on malformed location/zone/timestamp |
| `parse_outcome_line(line)` | `EverQuestCompactOutcome` (infallible) | Classifies into `EverQuestOutcomeKind` with `confidence`, `redacted`, and `diagnostic_code`; never errors — bad input becomes a diagnostic row |
| `tail_log(path, cursor, max_bytes, max_events)` | `EverQuestLogTailBatch` | Seeks to `cursor` (clamped to file len), reads ≤ `max_bytes`, parses ≤ `max_events`, returns `next_offset` and truncation flags |

### `EverQuestLogKind` (events) and `parse_log_line` classification

| Kind | Trigger / pattern |
|---|---|
| `LoggingEnabled` | message starts `Logging to ` |
| `Location` | message starts `Your Location is`; parses 3 comma floats in display order Y, X, Z into `EverQuestLocation{display_y,display_x,display_z}`; non-finite or wrong arity fails closed |
| `ZoneEntered` | message starts `You have entered `; trailing `.` stripped |
| `TargetNpc` / `TargetPlayer` | `Targeted (NPC): ` / `Targeted (Player): ` |
| `TargetCleared` | `You no longer have a target.` |
| `Consider` | regex `^(?P<target>.+) judges you .+ \(Lvl: (?P<level>[0-9]+)\)$` |
| `CastBegins` | `<actor> begins casting Gate.` or regex `^(?P<actor>.+) begins casting (?P<spell>.+)\.$` |
| `CastResult` | `<actor> fades away.` |
| `Say` | regex `^(?P<actor>.+) say(?:s)?, '.*'$` (body not retained) |
| `Tell` | regex `^(?P<actor>.+) tells (?P<channel>[^,]+), '.*'$` (body not retained) |
| `System` | otherwise, if message starts `You ` |
| `Other` | fallthrough |

`EverQuestLogEvent` fields: `timestamp, kind, actor?, target?, channel?, level?, location?, zone?, summary`. Summaries are compacted to `MAX_SUMMARY_CHARS = 160` (whitespace collapsed, truncated with `...`); chat bodies are never copied into the summary.

### `EverQuestOutcomeKind` (outcomes) and `parse_outcome_line` classification

`classify_outcome` runs damage → spell → progress → rest/loot/death classifiers, then falls back to `parse_log_line`.

| OutcomeKind | Pattern / rule | confidence | redacted |
|---|---|---|---|
| `CombatDamageTaken` | dot-damage regex w/ actor `you`, or `<actor> ... YOU for <n> points of damage.` | 0.8 / 0.85 | no |
| `HazardSignal` | dot-damage regex `^(?P<actor>.+) has taken (?P<amount>[0-9]+) damage by (?P<spell>.+)\.$` where actor ≠ you | 0.8 | no |
| `CombatDamageDealt` | `You (hit\|slash\|crush\|pierce\|kick\|bash) <target> for <n> points of damage.` | 0.85 | no |
| `AmbiguousCombat` | message contains `damage`/` hits `/` hit ` but no precise match | 0.25 | yes (`ambiguous_combat`) |
| `SpellHit` | `Your <spell> (hits\|hit) <target> for <n> points of ... damage.` | 0.85 | no |
| `SpellFizzle` | exactly `Your spell fizzles!` | 0.95 | no |
| `SpellResist` | contains `resist` and `spell` | 0.65 | yes (`spell_resist_unparsed`) |
| `XpGain` | `^You gain(?:ed)?(?: party)? experience!+$` | 0.9 | no |
| `LevelUp` | `^You have gained a level! Welcome to level (?P<level>[0-9]+)!$` | 0.95 | no |
| `Death` | `^You have been slain by (?P<actor>.+)!$` | 0.95 | no |
| `Respawn` | starts `LOADING, PLEASE WAIT` or `You regain consciousness` | 0.55 | yes (`respawn_or_loading`) |
| `RestSit` | `You sit down.` / `You begin to meditate.` / `You stand up.` | 0.9 | no |
| `Loot` | starts `You receive ` / `You have looted ` | 0.65 | yes (`loot_name_redacted`) |
| `TargetNpc/Player/Cleared`, `Consider`, `SpellBegins`, `ZoneEntered`, `Location` | mapped from `parse_log_line` event kind | 0.9 | no |
| `ChatRedacted` | Say/Tell fallthrough — body suppressed, summary is channel/actor only | 0.35 | yes |
| `Unknown` | unclassified | 0.1 | yes (`unknown`) |
| `DiagnosticMissingTimestamp` / `DiagnosticMalformedTimestamp` | line did not match line-regex / timestamp did not parse | 0.0 | yes |

`EverQuestCompactOutcome` fields: `timestamp?, timestamp_text?, kind, actor?, target?, spell?, channel?, amount?, level?, zone?, location?, summary, redacted, confidence, diagnostic_code?`.

---

## 12.3 Map, Zones, and Inventory

### Map files (`crates/synapse-everquest/src/map.rs`)

EverQuest map files live in `<root>/maps` (`MAP_DIR_NAME = "maps"`), one `.txt` file per zone; the file stem is the `zone_short_name`. `discover_map_files` returns sorted `EverQuestMapSource { path, zone_short_name, len_bytes, last_modified_unix_ms? }`. `parse_map_file` (default cap `DEFAULT_MAX_MAP_FILE_BYTES = 8 MiB`, overridable via `parse_map_file_with_limit`) fails closed on empty/oversized/malformed/unknown records.

**Record format** — first non-whitespace char is the record type; fields are comma-separated and trimmed:

| Type | Fields | Parsed into |
|---|---|---|
| `L` (line/segment) | 9: `x1,y1,z1,x2,y2,z2,r,g,b` | `EverQuestMapLine { start:Coord, end:Coord, color:{r,g,b u8} }` |
| `P` (point/label) | 8 via `splitn(8,',')`: `x,y,z,r,g,b,layer,label` | `EverQuestMapPoint { location:Coord, color, layer:i32(≥0), label }` |

`splitn(8, ',')` means the 8th field (`label`) keeps any internal commas (verified by test `preserves_point_label_commas`). Coords (`EverQuestMapCoord{x,y,z:f64}`) must be finite; colors `0..=255`; layer non-negative. `EverQuestMapFile` reports `line_count, segment_count, point_count, records`. Unknown record types (e.g. `Q`) raise `UnknownRecord`.

### Map inventory (`crates/synapse-everquest/src/map_inventory.rs`)

`inventory_map_set(install_root, set_name?)` (cap `DEFAULT_MAX_MAP_FILES = 4096`) builds an `EverQuestMapSetInventory` over `maps/` (or a single named subdirectory; `default`/`base`/empty → base maps dir, `EverQuestMapSetKind::BaseMapsDirectory` vs `MapsSubdirectory`). Per file it computes a streaming SHA-256 (`sha256_file`, `sha256:`-prefixed hex) and folds it into an `aggregate_sha256` over `relative_path \0 len \0 file_sha \0`. It tallies zone counts, parseable vs. skipped files, line/segment/point totals, modified-time bounds, and detects:

- **Duplicate zones** — same `zone_short_name` across >1 file (`EverQuestDuplicateZone`).
- **Duplicate labels** — same zone + normalized label (`EverQuestDuplicateLabel`); `normalize_label` lowercases and keeps only ASCII alphanumerics.

Samples are capped at `MAX_SAMPLE_ROWS = 64`; `samples_truncated` flags when limits were exceeded.

### Zone graph (`crates/synapse-everquest/src/zone_graph.rs`)

`build_zone_graph(&[EverQuestMapFile])` (or `build_zone_graph_from_root` which discovers+parses, recording parse failures in `skipped_maps`) produces `EverQuestZoneGraph { nodes, landmarks, edges, segments, unresolved_edge_count, skipped_maps }`:

- **Node** (`EverQuestZoneNode`) — one per map; `zone_short_name`, optional `display_name`, source path/len/mtime.
- **Landmark** (`EverQuestZoneLandmark`) — every `P` record becomes a landmark (label + normalized label + 3D location + layer + source line).
- **Segment** (`EverQuestZoneSegment`) — every `L` record.
- **Edge** (`EverQuestZoneEdge`) — a `P` record whose label begins (case-insensitive) with `to_`, `to `, or `to-` (`transition_target_hint`). The remainder is the target hint; the edge is resolved against available zones:

| Resolution | Rule | confidence |
|---|---|---|
| `ExactZoneShortName` | normalized hint equals a normalized zone short name | 0.95 |
| `Alias` | hint matches a hard-coded alias mapping to an available zone | 0.85 |
| `Unresolved` | no match (`target_zone_short_name = None`, hint preserved) | 0.25 |

The alias table (`zone_aliases`) maps e.g. `neriak_foreign_quarter → neriaka`, `neriak_commons → neriakb`, `neriak_third_gate → neriakc`, `nektulos_forest → nektulos`, `east_commonlands → ecommons`, `west_commonlands → commons`, `lavastorm_mountains → lavastorm`. `display_name_for_zone` provides human names for the same short codes.

**Query helpers** on `EverQuestZoneGraph`: `node`, `exits_for_zone`, `landmarks_for_zone`, `segments_for_zone` (all case-insensitive), and `nearest_landmarks(zone, location, limit)` which sorts landmarks by 3D Euclidean `distance` and truncates.

> Note: the crate's zone graph is a **static descriptive graph** (nodes/edges/landmarks). The cross-zone-line pathfinding and intra-zone shortest-path search live in the MCP `everquest_route` module (§12.6), not in the crate.

---

## 12.4 Current State, Memory, Outcome, Guard — Data Flow

These four modules form the perception → persistence → safety pipeline that precedes any gameplay action.

```
current_state  →  memory_record / memory_consult  →  outcome_ingest  →  planner_guard  →  autocombat
 (snapshot)        (hazard/safe-area ontology)        (event audit)      (safety gate)     (action loop)
```

### `everquest_state.rs` — `everquest_current_state`

Builds and persists the canonical compact snapshot `EverQuestCurrentState` (row `everquest/current_state/v1/everquest.live`) from: foreground/input HUD, an active EQ-log tail (≤ 512 KB / 65536 events), the zone graph, and HUD level/XP. Fields use a generic confidence wrapper `EverQuestStateField<T> { value, confidence(0–1), sources[], note }`. Captures `focus` (foreground/process/hwnd), `log_cursor` (offsets), `zone` (short name + location + 3 nearest landmarks), `level`, `xp_percent`, `target`, `consider`, `latest_actions`, and `hazards` (login screen visible, log truncated, missing fields). EQ display Y/X are converted to map axes (negated). Reads the row back after write for exact-storage confirmation. This is the source-of-truth row consumed by guard, route, world-summary, etc.

### `everquest_memory.rs` — `everquest_memory_record`, `everquest_memory_consult`

A persistent hazard/safe-area ontology used to gate planning.

- **Record** persists `EverQuestWorldMemoryRow` (`memory_type` Hazard|SafeArea, subject, zone, location+radius, severity, confidence, `active_for_planning`, `planning_status`). Validates confidence `[0,1]`, radius `[0,10000]`, ≥1 source ref. Applies a **stale penalty** (if `now - source_generated_at > stale_after_seconds`, confidence is capped to ≥0.25 / flagged) and **conflict downgrade** (conflicting evidence reduces existing confidence by a delta). `active_for_planning = confidence ≥ 0.50 AND fresh AND SupportsMemory`.
- **Consult** scans active memory rows (by hazard/safe-area prefix or explicit keys) against one candidate action, matching on target subject, zone, and location-within-radius. Decision: `abstain_state_unknown` (no target/zone/location), `avoid` (hazard match), `allow_with_safe_memory` (safe-area match), or `allow_no_matching_hazard`. Writes an `EverQuestPlannerConsultRow`.

### `everquest_outcome.rs` — `everquest_outcome_ingest`

Parses active or explicit log bytes (explicit path gated behind `allow_explicit_log_path` for FSV) into compact, redacted, **deduplicated** `EverQuestOutcomeRow`s. Each row's `event_id = <offset hex>-<sha256[:16]>` and carries a byte-exact `content_sha256`. Uses `synapse_everquest::parse_outcome_line` (§12.2). Filters Unknown/malformed unless `persist_unknown=true`; dedups via row-key lookup; reads each persisted row back. Reports `rows_read`, `rows_persisted`, `duplicate_rows`, truncation flags.

### `everquest_guard.rs` — `everquest_planner_guard`

The safety gate evaluated **before** any bounded foreground keystroke. Reads foreground, the current-state row (or an override), and chat-input state, then runs a battery of guards and persists `EverQuestPlannerGuardDecisionRow`. `decision = "select"` only if **all** guards pass; otherwise `"reject"` with `rejected_reasons`. Candidate kinds: `LocProbe, InventoryRead, MapRead, TargetConsider, BoundedMove, SitRest, CombatSpell`.

Critical guards: `foreground_everquest_live`, `chat_input_safe` (allow-empty-chat, visible, no text, no denial), `state_row_available`, `zone_known` (confidence ≥0.50), conditional `level_known`/`location_known_for_movement` (only required for CombatSpell / BoundedMove respectively), `no_stop_hazard` (rejects critical hazards or codes containing death/aggro/combat/unexpected_zone). For `CombatSpell` it adds: `verified_attack_spell` (hotbar alias must be `hotbar4`), `target_known`, `target_is_npc`, `target_level_safe_for_level_one_wizard` (L1 → target level ≤1), `target_con_safe` (rejects gamble/danger/deadly/threat/red/Lvl 2–6), and combat-readiness guards (health ≥80%, mana ≥30%, must be standing).

---

## 12.5 Autocombat (`everquest_autocombat.rs`) — `everquest_autocombat`

Runs a **bounded, operator-attended, server-side** combat loop for the level-1 wizard: *acquire → consider → melee + nuke-when-mana → confirm kill → recover → re-acquire*. It is the one EQ tool that executes input (via audited keymap actions).

Policy bounds (input-validated): `max_iterations 1–50`, `max_duration 1–600 s`, `hp_floor%`, `mana_floor%`, `target_level_max`, `stop_at_level`, `cast_mana_cost%`, engagement timeout, `hotbar_alias`, roam/chase limits.

Decision helpers:

- `classify_con(summary, target_level_max)` → `Safe | TooHigh | NonNpc | Unknown`. NonNpc on merchant/player/guard; TooHigh on red-con phrases (crazy/kill_you/deadly) or parsed level > cap; Safe when NPC level ≤ cap.
- `classify_fight_signal(log_summaries)` → `Slain | TargetLost | Continue | Idle | Fleeing` from log lines (`has been slain by`, fled/no-target, resist/fizzle/miss/melee verbs, etc.).
- `should_cast_nuke(mana%, cast_cost%)` → true iff mana known and `mana ≥ cost`; unknown mana relies on melee.

**Stop conditions** (checked each iteration, in `ActAutocombatResponse.stop_reason`): operator panic, max duration, foreground lost, login/chat unsafe, HP floor, reached target level (after a kill), ≥3 consecutive no-target (`no_safe_target`), max iterations. Per-iteration it acquires/considers (unless already in a fight), classifies con, asserts melee auto-attack, ticks the fight loop (~900 ms; casts nuke when mana allows; tallies resist/fizzle), then recovers mana by sitting (up to ~45 s) on a kill. Emits `ActAutocombatIteration` rows and a persisted run row with kills, casts, resists/fizzles, level progress, and `stop_reason`.

---

## 12.6 Route Planning (`everquest_route.rs`) — `everquest_route_plan`

Plans (but never executes) a bounded multi-waypoint route from current state to a target landmark label or zone line, persisting `EverQuestRoutePlanRow` with `decision` (`route_ready` / `abstain_*`), waypoints, nearest start/target landmarks, an optional `verified_transition`, `total_distance`, `confidence`, and hazards. Default `max_waypoints = 8` (max 32). Waypoint kinds: `current_state, map_line_guidance, verified_transition_volume, static_zone_label_hint, target_landmark`. The evidence boundary asserts `movement_executed = false`.

### Pathfinding algorithm — **Dijkstra (uniform-cost shortest path)**

For routes needing Z-level navigation (elevation change ≥ 8.0 map units), the module builds a weighted **undirected** graph from the zone's `L` segment endpoints and runs `dijkstra_path(adjacency, start, target)`:

- **Graph construction:** segment endpoints become nodes keyed by `route_node_key` at 4.0-unit precision (deduping near-coincident points). Bidirectional edges connect each segment's two endpoints, weighted by 3D Euclidean distance `sqrt(dx²+dy²+dz²)`. The player start and target are connected to their nearest 8 nodes within a 96.0-unit radius.
- **Search:** classic Dijkstra with an **exhaustive linear scan** for the minimum-distance unvisited node (no binary-heap priority queue), edge relaxation updating `distance_from_start`/`previous`, terminating when the target is popped; the path is reconstructed by backtracking `previous` and reversing. Returns `None` (route abstains) if no connected path exists.
- **Post-processing:** long path edges (>64.0 units) are subdivided by linear interpolation into guidance steps; already-reached front nodes are pruned.

When elevation navigation is not required, the route uses landmark/zone-line guidance steps directly rather than the segment graph.

---

## 12.7 Sensing & Feeds: Map Sensor, Log Feed, UI Context

### `everquest_map_sensor.rs` — `everquest_map_sensor`

Calibrates the visible in-game map window against local map files and persists `EverQuestMapSensorRow`. Extracts visible-map evidence from HUD OCR field `everquest.map_window_text` (UI tokens + zone-title aliases), detects labels by normalized substring match (drops labels <4 chars, caps 32), and selects a calibration method by priority: visible player marker > matched labels > visible bounds alone. Confidence combines OCR confidence + UI-token presence (0.60) + zone OCR match (0.72) + detected labels (0.78), clamped to `[0, 0.95]`. The `EverQuestMapTransform` records a single-point translation anchor from `/loc`; scale/rotation stay unset until a second anchor is verified. `decision`: `calibrated` / `uncalibrated_visible_map` / `abstain_*`.

### `everquest_log.rs` — perception feed (no tool)

Populates the M1 perception `ObservationInput.recent_events` rather than exposing an MCP tool. Discovers the active character log from `eqclient.ini` (`LastCharSel=`, `Log=1`), maintains a cursor (path + offset) in M1 storage, detects rotation when offset > file length, and tail-reads up to ~8 KB / 8 parsed events per cycle. Emits the same event kinds as §12.2 with chat/system bodies redacted to summaries.

### `everquest_ui_context.rs` — UI preflight (no tool)

Returns `EverQuestUiContextReadback` for action preflight. Detects login-screen vs. in-world state from HUD: login keywords in `everquest.login_screen_text` (username/password/login/quick connect/eula) take precedence over in-world signals (presence of level/next-level/map HUD fields). Reports focused text-element summary (role/name/UIA-pattern based) as `value_len`/`selected_len` without persisting content. Status: `login_screen` > `in_world_ui` > `ambiguous_ui`.

---

## 12.8 DynamicJEPA Domain Normalization (`everquest_domain.rs`) — `everquest_domain_normalize`

Normalizes one EverQuest **state/action/outcome transition** into typed DynamicJEPA `CF_KV` rows and reads them back. Input bundles `EverQuestDomainStateInput`, `EverQuestDomainActionInput`, `EverQuestDomainOutcomeInput`, and entity/source-ref metadata; output is `EverQuestDynamicJepaTransitionRow` with `validation_status` (`Accepted | Rejected | Denied`), `accepted_for_planning`, `invariant_results`, and `rejection_reasons`.

Continuous fields are **bucketed** into discrete enums for tractable modeling, e.g. `HeadingBucket` (8 compass dirs), `LevelBucket`, `XpBucket`, `ConBucket` (SafeLevelOne/BlueSafe/Even/Gamble/Dangerous/Tombstone), `ResourceBucket` (Empty/Low/Ready/Full), `UiFocusBucket`, and `ActionKind`/`OutcomeKind`/various `*Delta` enums. `coord_axis_bucket` snaps each X/Y/Z float to a 50-unit bucket string (`floor(v/50)*50` → `"{axis}_{start}_{end}"`).

**Invariants** (`evaluate_invariants`): (1) zone-entry outcomes must have matching `log_event_kind`/`outcome_kind`/zone change; (2) movement actions require EQ foreground (`eqgame.exe` + `everquest.live`); (3) combat spells require a safe NPC target, safe con, `target_level_delta ≤ 0`, ready HP/mana, non-chat focus; (4) chat/social/economy keywords are forbidden in `tool_name`/`alias`; (5) zone transitions without a zone-entry log are rejected. `Denied` if action/outcome is the denied-unsafe kind; `Accepted` if all invariants pass; otherwise `Rejected`.

---

## 12.9 Trajectories, Episodes, ContextGraph Memory

### `everquest_trajectory.rs` — `everquest_trajectory_record`

Persists an **ordered** trajectory (`EverQuestTrajectoryRow`) from linked state/domain-transition/outcome/guard/map-state rows and action/observation/event/log refs. Each `EverQuestTrajectoryTransitionRow` has a strictly increasing `sequence` and monotonic `occurred_at`, links verified row keys (with their byte lengths), and log refs (`path`, `start_offset < next_offset`, `event_kind`, `content_sha256`, `redacted`). Computes a `trajectory_hash` (SHA-256) and optionally exports JSONL. `intent`: `NavigationProbe, TargetConsiderProbe, CombatAttempt, Recovery, LevelUpRun`.

### `everquest_episode_export.rs` — `everquest_episode_export`

Exports redacted trajectory/domain rows to ContextGraph-compatible **DynamicJEPA episode JSONL** (one JSON object per line) under `%LOCALAPPDATA%/synapse/everquest/contextgraph_episodes/`. Each `EverQuestContextGraphEpisodeRow` bundles state/action/outcome/transition blocks plus expected-vs-actual readback and a redaction block (`compact_redacted=true`, no raw chat body, no raw target names, session id hashed, all log refs marked redacted). Writes to a temp file, validates line count + first/last episode ids + SHA-256, then promotes atomically. Refuses export if any source row is not compact-redacted.

### `everquest_contextgraph.rs` — `everquest_contextgraph_ingest`, `everquest_contextgraph_search`

Bridges Synapse storage to the external **ContextGraph** long-term-memory process over JSON-RPC 2.0 MCP stdio.

- **Ingest** reads the episode JSONL (SHA-256 verified), builds a compact ≤900-char `memory_content` summary per episode, calls ContextGraph `store_memory`, and retrieves `get_provenance_chain` / `get_audit_trail`. Dedups per `(profile_id, export_sha256, episode_id)`; persists `EverQuestContextGraphIngestRow` with all readbacks. Cap `MAX_EPISODES_PER_INGEST = 64`, default importance 0.78.
- **Search** calls ContextGraph `search_graph` with a tagged query (game/character/server/zone), extracts citations by scanning results for `source_episode_id=` / `source_export_sha256=` markers, and (if `require_provenance`) requires cited provenance. Persists an `EverQuestContextGraphSearchRow`. Defaults: `top_k = 8` (max 25), timeout 120 s.

---

## 12.10 World Model & World Summary

### `everquest_world_model.rs` (+ `model.rs`, `validation.rs`) — `everquest_world_model_record`, `everquest_world_model_inspect`

A generic, size-capped, versioned KV store for compact world-model payloads. `record` persists one `EverQuestWorldModelRow` under prefix `everquest/{kind_prefix}/v1/{profile_id}` with exact readback; `inspect` returns prefix counts, samples, and an optionally-selected row. `EverQuestWorldModelKind`: `Map, ZoneGraph, State, Transition, Trajectory, Planner, Surprise`. `write_mode` `Create`/`Replace` (replace increments `revision`, stores `previous_payload_sha256`). `retention_class` `Strategic`(no TTL)/`Episode`(30 d)/`Scratch`(1 d). Limits: default payload 8 KB / hard max 32 KB, `MAX_SOURCE_REFS = 32`, inspect scan limit 4096. Validation computes the payload SHA-256 and **rejects payloads containing raw chat/message/target keys**.

### `everquest_world_summary.rs` (+ `model.rs`, `validation.rs`) — `everquest_world_summary`

Builds a compact, context-injectable world-state summary (`EverQuestWorldSummaryRow`, prefix `everquest/world_summary/v1/...`) by integrating: the current-state row (or override), the zone graph (nearest exits + landmarks, sorted by 3D distance), and reality-audit context (baseline epoch/seq, newest delta, drift status `in_sync/minor/major/rebase_required`). Collects `active_blockers` (e.g. `unknown_zone`, `unknown_location`, `stale_state`, `map_graph_unavailable`, `reality_audit_*`) and `safe_next_probes` (refresh_state, verify_maps, refocus_everquest, run_loc_probe, route_probes, planner_guard). `compact_status` = `"blocked"` if any blocker is active, else `"ready"`. Chat-like text in summaries is redacted to `[redacted chat summary]`; text fields truncated to 512 bytes.

---

## 12.11 Predictive Model, Surprise Detection, Scorecard

### `everquest_predictive_model.rs` — `everquest_predictive_model_fit`, `everquest_predictive_model_predict`

Fits a **transparent action-conditioned Markov baseline** (`algorithm = "action_conditioned_markov_baseline_v1"`) from verified trajectory/domain rows, then makes calibrated next-outcome predictions with abstention.

- **Fit:** reads linked state/action/outcome rows from up to `max_trajectories` (default 64) trajectories. For each `(state_signature, action_kind)` it buckets observed outcomes and picks the majority target; `confidence = winning_count / sample_count`. Builds three scopes: `state_action` (exact state+action), `action_fallback` (action only), `global_fallback` (wildcard). Persists `EverQuestPredictiveModelRow` (entries + a `model_hash`). Defaults: `min_transition_support = 1`, `min_confidence = 0.60`, competence floor 0.60, stretch 0.80.
- **Predict:** evaluates candidate actions against model entries by **scope priority** (state_action rank 0 > action_fallback 1 > global_fallback 2). **Abstains** on stale hash, no verified trajectories, no candidates, no matching entry, `sample_count < min_transition_support`, or `confidence < min_confidence`; otherwise selects the highest-ranked candidate. Persists `EverQuestPredictivePredictionRow` with `decision`, `abstain`, `evaluated_candidates`.

### `everquest_surprise.rs` (+ `compare.rs`, `model.rs`, `validation.rs`) — `everquest_surprise_detect`

Compares a prediction to observed state/log evidence and persists a compact surprise world-model row. Default `threshold = 0.50`, `stale_after_seconds = 300`.

**Surprise-detection algorithm** (`compare.rs`):

1. **Pre-comparison abstentions** (each sets `stop_condition = true`): missing prediction (`abstain_missing_prediction`, divergence 0.0); prediction `confidence < threshold` (`abstain_low_confidence_prediction`); observation stale (`now - observed_at > stale_after_seconds`, `abstain_stale_observation`); observation zone/outcome confidence `< threshold` (`abstain_low_confidence_observation`).
2. **Field comparison:** case-insensitive equality on `zone_short_name` and `outcome_kind` where both predicted and observed are set; tracks `compared_fields` and `mismatch_reasons` (`zone_short_name_mismatch`, `outcome_kind_mismatch`). No comparable fields → `abstain_no_comparable_fields`.
3. **Divergence metric:** `divergence_score = num_mismatches / num_compared_fields` (0.0–1.0).
4. **Decision:** `divergence_score ≥ threshold` → `surprise_detected` (`surprise_detected=true`, `stop_condition=true`); else `expected_outcome_confirmed`.
5. **Remediation:** on stop, emit `["stop_gameplay_actions","reestimate_current_state","repair_world_model_from_physical_sot"]`; else `["continue_supervised_planning"]`.

### `everquest_scorecard.rs` — `everquest_action_prior_record`, `everquest_action_prior_scorecard`

Records per-prediction correctness samples and aggregates them into a **floor-not-ceiling competence scorecard**.

- **Per-sample correctness** (`EverQuestActionPriorCorrectness`): `top1_correct` (predicted `next_action` == actual, case-insensitive), `top3_correct` (actual in predicted top-3), `zone_correct`, `coord_bucket_correct`, `hazard_avoidance_correct` (`hazard_avoidance == !hazard_occurred`). `useful` = non-abstaining, actual known, and ≥1 correctness field true. `overconfident_wrong` = non-abstaining, actual known, not useful, `confidence ≥ 0.80`. `class` ∈ {`correct_top1, correct_top3, correct_context, abstained, unknown_actual, wrong`}.
- **Aggregate metrics:** per-dimension `*_total/_correct/_accuracy` over `evaluated_count` (non-abstaining ∧ actual-known), plus `surprise_rate`, `honest_abstention_count` (abstained ∧ (confidence<0.60 ∨ unknown actual)), `overconfident_wrong_count`, and `supervised_utility_rate = (useful_correct + honest_abstention_count) / sample_count`. Confidence is bucketed into 5 calibration bins (`0.00–0.20 … 0.80–1.00`).
- **Competence decision** against `competence_floor` (0.60) and `stretch_target` (0.80) using `useful_accuracy`. Status ∈ {`no_verified_trajectories, insufficient_samples, low_confidence_action_forced, stretch_target_met, minimum_competence_floor_met, below_minimum_competence_floor, no_evaluated_predictions`}; the floor is enforced as a contract (`minimum_is_floor_not_ceiling = true`), with recommendations to keep optimizing above the floor. Defaults: `min_samples = 3`, `min_confidence_for_action = 0.60`.

---

## 12.12 Cross-References

- MCP tool input/output schemas and the full tool catalogue: see [16_api_tools_reference.md](16_api_tools_reference.md).
- All MCP rows are persisted in the reflex-runtime `CF_KV` column family with byte-exact readback; chat/target redaction and the `evidence_boundary` (reads physical state, writes own row only, manual FSV required at runtime) are uniform across the EverQuest tool family.
