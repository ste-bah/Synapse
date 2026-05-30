use chrono::{DateTime, Utc};
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use synapse_core::{ForegroundContext, HudValue, error_codes};
use synapse_everquest::{
    EverQuestLogEvent, EverQuestLogKind, EverQuestLogTailBatch, EverQuestMapCoord,
    EverQuestZoneGraph, build_zone_graph_from_root, tail_log,
};
use synapse_storage::cf;

use super::{
    Json, Parameters, SynapseService,
    everquest_log::{ActiveEverQuestLog, EVERQUEST_PROFILE_ID},
    tool, tool_router,
};
use crate::m1::{current_input, mcp_error};

const TOOL: &str = "everquest_current_state";
pub(super) const CURRENT_STATE_ROW_KEY: &str = "everquest/current_state/v1/everquest.live";
const MAX_STATE_LOG_BYTES: usize = 512 * 1024;
const MAX_STATE_LOG_EVENTS: usize = 65_536;
const MAX_ACTION_ROWS: usize = 8;
const MAX_LANDMARKS: usize = 3;

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestCurrentStateParams {}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestCurrentStateResponse {
    pub ok: bool,
    pub row_key: String,
    pub stored_value_len_bytes: u64,
    pub state: EverQuestCurrentState,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestCurrentState {
    pub schema_version: u32,
    pub profile_id: String,
    pub generated_at: DateTime<Utc>,
    pub character: String,
    pub server: String,
    pub focus: EverQuestFocusState,
    pub log_cursor: EverQuestStateLogCursor,
    pub zone: EverQuestStateField<String>,
    pub zone_short_name: EverQuestStateField<String>,
    pub location: EverQuestStateField<EverQuestStateLocation>,
    pub nearest_landmarks: Vec<EverQuestStateLandmark>,
    pub level: EverQuestStateField<u32>,
    #[serde(default = "default_xp_percent_field")]
    pub xp_percent: EverQuestStateField<f32>,
    pub target: EverQuestStateField<String>,
    pub consider: EverQuestStateField<String>,
    pub latest_actions: Vec<EverQuestStateActionSummary>,
    pub hazards: Vec<EverQuestStateHazard>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestStateField<T> {
    pub value: Option<T>,
    pub confidence: f32,
    pub sources: Vec<EverQuestStateSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestFocusState {
    pub is_everquest_foreground: bool,
    pub confidence: f32,
    pub hwnd: i64,
    pub process_name: String,
    pub process_path: String,
    pub window_title: String,
    pub profile_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestStateLogCursor {
    pub path: String,
    pub start_offset: u64,
    pub next_offset: u64,
    pub file_len_bytes: u64,
    pub bytes_read: usize,
    pub event_count: usize,
    pub truncated_by_bytes: bool,
    pub truncated_by_events: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestStateLocation {
    pub display_y: f64,
    pub display_x: f64,
    pub display_z: f64,
    pub map_x: f64,
    pub map_y: f64,
    pub map_z: f64,
    pub log_timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestStateLandmark {
    pub label: String,
    pub zone_short_name: String,
    pub distance: f64,
    pub map_x: f64,
    pub map_y: f64,
    pub map_z: f64,
    pub source_path: String,
    pub source_line_number: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestStateActionSummary {
    pub profile_id: Option<String>,
    pub tool: Option<String>,
    pub status: Option<String>,
    pub error_code: Option<String>,
    pub ts_ns: Option<u64>,
    pub foreground_process_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestStateHazard {
    pub code: String,
    pub severity: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestStateSource {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[tool_router(router = everquest_state_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Build and persist the compact EverQuest current-state record from live foreground, EQ log, map, HUD, and audit sources"
    )]
    pub async fn everquest_current_state(
        &self,
        _params: Parameters<EverQuestCurrentStateParams>,
    ) -> Result<Json<EverQuestCurrentStateResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TOOL,
            "tool.invocation kind=everquest_current_state"
        );
        let state = self.estimate_everquest_current_state()?;
        let response = self.persist_everquest_current_state(&state)?;
        Ok(Json(response))
    }
}

impl SynapseService {
    fn estimate_everquest_current_state(&self) -> Result<EverQuestCurrentState, ErrorData> {
        let mut input = {
            let state = self.m1_state()?;
            current_input(&state, 2)?
        };
        self.resolve_input_profile_and_hud(&mut input, true);
        let focus = focus_state(&input.foreground);

        let (active, batch, graph) = self.current_state_log_sources()?;
        let latest_actions = self.latest_everquest_action_summaries()?;

        let zone = latest_zone_field(&batch.events, &batch);
        let zone_short_name = zone_short_name_field(zone.value.as_deref(), &graph, &zone);
        let location = latest_location_field(&batch.events, &batch);
        let nearest_landmarks = nearest_landmarks(
            &graph,
            zone_short_name.value.as_deref(),
            location.value.as_ref(),
        );
        let level = level_field(&input.hud);
        let xp_percent = xp_percent_field(&input.hud);
        let target = latest_summary_field(
            &batch.events,
            &batch,
            &[EverQuestLogKind::TargetNpc, EverQuestLogKind::TargetPlayer],
            "no target log event in sampled log window",
        );
        let consider = latest_summary_field(
            &batch.events,
            &batch,
            &[EverQuestLogKind::Consider],
            "no consider log event in sampled log window",
        );
        let mut hazards = hazards_for_state(
            &focus,
            &zone,
            &zone_short_name,
            &location,
            &level,
            &xp_percent,
            &batch,
        );
        if let Some(cursor) = self.m1_state()?.everquest_log_cursor.clone()
            && cursor.path == active.log.path
            && cursor.offset > batch.file_len_bytes
        {
            hazards.push(EverQuestStateHazard {
                code: "stale_log_cursor".to_owned(),
                severity: "warning".to_owned(),
                detail: format!(
                    "M1 cursor offset {} is beyond file length {}",
                    cursor.offset, batch.file_len_bytes
                ),
            });
        }

        Ok(EverQuestCurrentState {
            schema_version: 1,
            profile_id: EVERQUEST_PROFILE_ID.to_owned(),
            generated_at: Utc::now(),
            character: active.log.identity.character,
            server: active.log.identity.server,
            focus,
            log_cursor: EverQuestStateLogCursor {
                path: active.log.path.display().to_string(),
                start_offset: batch.start_offset,
                next_offset: batch.next_offset,
                file_len_bytes: batch.file_len_bytes,
                bytes_read: batch.bytes_read,
                event_count: batch.events.len(),
                truncated_by_bytes: batch.truncated_by_bytes,
                truncated_by_events: batch.truncated_by_events,
            },
            zone,
            zone_short_name,
            location,
            nearest_landmarks,
            level,
            xp_percent,
            target,
            consider,
            latest_actions,
            hazards,
        })
    }

    fn current_state_log_sources(
        &self,
    ) -> Result<
        (
            ActiveEverQuestLog,
            EverQuestLogTailBatch,
            EverQuestZoneGraph,
        ),
        ErrorData,
    > {
        let active = self.resolve_active_everquest_log().map_err(|detail| {
            mcp_error(
                error_codes::ACTION_TARGET_INVALID,
                format!("{TOOL} could not resolve active EverQuest log: {detail}"),
            )
        })?;
        let file_len_bytes = std::fs::metadata(&active.log.path)
            .map_err(|error| {
                mcp_error(
                    error_codes::STORAGE_READ_FAILED,
                    format!("read active EverQuest log metadata: {error}"),
                )
            })?
            .len();
        let start_offset =
            file_len_bytes.saturating_sub(u64::try_from(MAX_STATE_LOG_BYTES).unwrap_or(u64::MAX));
        let batch = tail_log(
            &active.log.path,
            start_offset,
            MAX_STATE_LOG_BYTES,
            MAX_STATE_LOG_EVENTS,
        )
        .map_err(|error| {
            mcp_error(
                error_codes::STORAGE_READ_FAILED,
                format!("{TOOL} could not tail active EverQuest log: {error}"),
            )
        })?;
        let graph = build_zone_graph_from_root(&active.install_root).map_err(|error| {
            mcp_error(
                error_codes::STORAGE_READ_FAILED,
                format!("{TOOL} could not build EverQuest zone graph: {error}"),
            )
        })?;
        Ok((active, batch, graph))
    }

    fn latest_everquest_action_summaries(
        &self,
    ) -> Result<Vec<EverQuestStateActionSummary>, ErrorData> {
        let rows = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while reading action audit",
                )
            })?;
            runtime
                .storage_cf_tail_rows(cf::CF_ACTION_LOG, MAX_ACTION_ROWS)
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        format!("read action audit tail: {error}"),
                    )
                })?
        };
        Ok(rows
            .into_iter()
            .filter_map(|(_key, value)| {
                let value = serde_json::from_slice::<Value>(&value).ok()?;
                if !action_row_matches_everquest(&value) {
                    return None;
                }
                Some(EverQuestStateActionSummary {
                    profile_id: value
                        .get("profile_id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    tool: value
                        .get("tool")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    status: value
                        .get("status")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    error_code: value
                        .get("error_code")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    ts_ns: value.get("ts_ns").and_then(Value::as_u64),
                    foreground_process_name: value
                        .pointer("/foreground/process_name")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                })
            })
            .collect())
    }

    fn persist_everquest_current_state(
        &self,
        state: &EverQuestCurrentState,
    ) -> Result<EverQuestCurrentStateResponse, ErrorData> {
        let encoded = serde_json::to_vec(&state).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode EverQuest current-state row: {error}"),
            )
        })?;
        let stored = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while writing current-state row",
                )
            })?;
            runtime
                .storage_put_kv_rows(vec![(CURRENT_STATE_ROW_KEY.as_bytes().to_vec(), encoded)])
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_WRITE_FAILED,
                        format!("write EverQuest current-state row: {error}"),
                    )
                })?;
            runtime
                .storage_kv_row(CURRENT_STATE_ROW_KEY.as_bytes())
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        format!("read EverQuest current-state row after write: {error}"),
                    )
                })?
                .ok_or_else(|| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        "EverQuest current-state row missing after write",
                    )
                })?
        };
        let readback =
            serde_json::from_slice::<EverQuestCurrentState>(&stored).map_err(|error| {
                mcp_error(
                    error_codes::STORAGE_CORRUPTED,
                    format!("decode EverQuest current-state row after write: {error}"),
                )
            })?;
        Ok(EverQuestCurrentStateResponse {
            ok: true,
            row_key: CURRENT_STATE_ROW_KEY.to_owned(),
            stored_value_len_bytes: stored.len() as u64,
            state: readback,
        })
    }
}

fn action_row_matches_everquest(value: &Value) -> bool {
    value
        .get("profile_id")
        .and_then(Value::as_str)
        .is_some_and(|profile| profile == EVERQUEST_PROFILE_ID)
        || value
            .get("active_profile_id")
            .and_then(Value::as_str)
            .is_some_and(|profile| profile == EVERQUEST_PROFILE_ID)
        || value
            .pointer("/foreground/profile_id")
            .and_then(Value::as_str)
            .is_some_and(|profile| profile == EVERQUEST_PROFILE_ID)
        || value
            .pointer("/foreground/process_name")
            .and_then(Value::as_str)
            .is_some_and(|process| process.eq_ignore_ascii_case("eqgame.exe"))
}

fn focus_state(foreground: &ForegroundContext) -> EverQuestFocusState {
    let is_everquest_foreground = foreground.profile_id.as_deref() == Some(EVERQUEST_PROFILE_ID)
        && foreground.process_name.eq_ignore_ascii_case("eqgame.exe");
    EverQuestFocusState {
        is_everquest_foreground,
        confidence: if is_everquest_foreground { 1.0 } else { 0.0 },
        hwnd: foreground.hwnd,
        process_name: foreground.process_name.clone(),
        process_path: foreground.process_path.clone(),
        window_title: foreground.window_title.clone(),
        profile_id: foreground.profile_id.clone(),
    }
}

fn latest_zone_field(
    events: &[EverQuestLogEvent],
    batch: &synapse_everquest::EverQuestLogTailBatch,
) -> EverQuestStateField<String> {
    events
        .iter()
        .rev()
        .find(|event| event.kind == EverQuestLogKind::ZoneEntered)
        .and_then(|event| {
            event
                .zone
                .as_ref()
                .map(|zone| field_some(zone.clone(), 0.95, log_source(batch, event), None))
        })
        .unwrap_or_else(|| field_none("no zone-entered event in sampled log window"))
}

fn zone_short_name_field(
    zone: Option<&str>,
    graph: &EverQuestZoneGraph,
    source_field: &EverQuestStateField<String>,
) -> EverQuestStateField<String> {
    let Some(zone) = zone else {
        return field_none("zone display name is unknown");
    };
    if let Some(short_name) = short_name_for_zone_display(zone) {
        let confidence = if graph.node(short_name).is_some() {
            0.95
        } else {
            0.75
        };
        return field_some(
            short_name.to_owned(),
            confidence,
            source_field
                .sources
                .first()
                .cloned()
                .unwrap_or_else(|| EverQuestStateSource {
                    kind: "zone_alias".to_owned(),
                    path: None,
                    start_offset: None,
                    next_offset: None,
                    log_timestamp: None,
                    summary: None,
                }),
            None,
        );
    }
    field_none(format!("no short-name mapping for zone {zone:?}"))
}

fn latest_location_field(
    events: &[EverQuestLogEvent],
    batch: &synapse_everquest::EverQuestLogTailBatch,
) -> EverQuestStateField<EverQuestStateLocation> {
    events
        .iter()
        .rev()
        .find(|event| event.kind == EverQuestLogKind::Location)
        .and_then(|event| {
            event.location.as_ref().map(|location| {
                field_some(
                    EverQuestStateLocation {
                        display_y: location.display_y,
                        display_x: location.display_x,
                        display_z: location.display_z,
                        map_x: display_to_map_x(location.display_x),
                        map_y: display_to_map_y(location.display_y),
                        map_z: location.display_z,
                        log_timestamp: event.timestamp.format("%Y-%m-%dT%H:%M:%S").to_string(),
                    },
                    0.98,
                    log_source(batch, event),
                    Some(
                        "EverQuest /loc displays Y, X, Z; local map files use the negated X/Y axes",
                    ),
                )
            })
        })
        .unwrap_or_else(|| field_none("no /loc event in sampled log window"))
}

fn display_to_map_x(display_x: f64) -> f64 {
    -display_x
}

fn display_to_map_y(display_y: f64) -> f64 {
    -display_y
}

fn nearest_landmarks(
    graph: &EverQuestZoneGraph,
    zone_short_name: Option<&str>,
    location: Option<&EverQuestStateLocation>,
) -> Vec<EverQuestStateLandmark> {
    let (Some(zone_short_name), Some(location)) = (zone_short_name, location) else {
        return Vec::new();
    };
    let coord = EverQuestMapCoord {
        x: location.map_x,
        y: location.map_y,
        z: location.map_z,
    };
    graph
        .nearest_landmarks(zone_short_name, &coord, MAX_LANDMARKS)
        .into_iter()
        .map(|nearest| EverQuestStateLandmark {
            label: nearest.landmark.label,
            zone_short_name: nearest.landmark.zone_short_name,
            distance: nearest.distance,
            map_x: nearest.landmark.location.x,
            map_y: nearest.landmark.location.y,
            map_z: nearest.landmark.location.z,
            source_path: nearest.landmark.source_path.display().to_string(),
            source_line_number: nearest.landmark.source_line_number,
        })
        .collect()
}

fn level_field(hud: &synapse_core::HudReadings) -> EverQuestStateField<u32> {
    let Some(reading) = hud.by_name.get("everquest.level_text") else {
        return hud.errors.get("everquest.level_text").map_or_else(
            || field_none("HUD level unavailable"),
            |error| field_none(format!("HUD level unavailable: {}", error.detail)),
        );
    };
    match &reading.parsed {
        HudValue::Number(value) => level_from_number(*value).map_or_else(
            || field_none(format!("HUD level number was not a valid integer: {value}")),
            |level| field_some(level, reading.confidence, hud_source(reading), None),
        ),
        HudValue::Text(text) => text.trim().parse::<u32>().map_or_else(
            |_| field_none(format!("HUD level text was not numeric: {text:?}")),
            |level| field_some(level, reading.confidence, hud_source(reading), None),
        ),
        other => field_none(format!("HUD level parsed to unsupported value: {other:?}")),
    }
}

fn level_from_number(value: f64) -> Option<u32> {
    if value.is_finite() && value >= 0.0 && value <= f64::from(u32::MAX) && value.fract() == 0.0 {
        format!("{value:.0}").parse::<u32>().ok()
    } else {
        None
    }
}

fn xp_percent_field(hud: &synapse_core::HudReadings) -> EverQuestStateField<f32> {
    let Some(reading) = hud.by_name.get("everquest.next_level_percent") else {
        return hud.errors.get("everquest.next_level_percent").map_or_else(
            || field_none("HUD XP percent unavailable"),
            |error| field_none(format!("HUD XP percent unavailable: {}", error.detail)),
        );
    };
    match &reading.parsed {
        HudValue::Number(value) => xp_percent_from_number(*value).map_or_else(
            || field_none(format!("HUD XP percent was outside 0..=100: {value}")),
            |xp_percent| field_some(xp_percent, reading.confidence, hud_source(reading), None),
        ),
        HudValue::Text(text) => text.trim().parse::<f32>().map_or_else(
            |_| field_none(format!("HUD XP percent text was not numeric: {text:?}")),
            |value| {
                xp_percent_from_number(f64::from(value)).map_or_else(
                    || field_none(format!("HUD XP percent was outside 0..=100: {value}")),
                    |xp_percent| {
                        field_some(xp_percent, reading.confidence, hud_source(reading), None)
                    },
                )
            },
        ),
        other => field_none(format!(
            "HUD XP percent parsed to unsupported value: {other:?}"
        )),
    }
}

fn xp_percent_from_number(value: f64) -> Option<f32> {
    if value.is_finite() && (0.0..=100.0).contains(&value) {
        #[allow(clippy::cast_possible_truncation)]
        Some(value as f32)
    } else {
        None
    }
}

fn default_xp_percent_field() -> EverQuestStateField<f32> {
    field_none("HUD XP percent unavailable in older current-state row")
}

fn hud_source(reading: &synapse_core::HudReading) -> EverQuestStateSource {
    EverQuestStateSource {
        kind: "hud".to_owned(),
        path: None,
        start_offset: None,
        next_offset: None,
        log_timestamp: None,
        summary: Some(reading.raw_text.clone()),
    }
}

fn latest_summary_field(
    events: &[EverQuestLogEvent],
    batch: &synapse_everquest::EverQuestLogTailBatch,
    kinds: &[EverQuestLogKind],
    missing_note: &'static str,
) -> EverQuestStateField<String> {
    events
        .iter()
        .rev()
        .find(|event| kinds.iter().any(|kind| kind == &event.kind))
        .map_or_else(
            || field_none(missing_note),
            |event| field_some(event.summary.clone(), 0.75, log_source(batch, event), None),
        )
}

fn hazards_for_state(
    focus: &EverQuestFocusState,
    zone: &EverQuestStateField<String>,
    zone_short_name: &EverQuestStateField<String>,
    location: &EverQuestStateField<EverQuestStateLocation>,
    level: &EverQuestStateField<u32>,
    xp_percent: &EverQuestStateField<f32>,
    batch: &synapse_everquest::EverQuestLogTailBatch,
) -> Vec<EverQuestStateHazard> {
    let mut hazards = Vec::new();
    if !focus.is_everquest_foreground {
        hazards.push(EverQuestStateHazard {
            code: "non_everquest_foreground".to_owned(),
            severity: "warning".to_owned(),
            detail: format!(
                "foreground is {} title {:?}",
                focus.process_name, focus.window_title
            ),
        });
    }
    if zone.value.is_none() {
        hazards.push(field_hazard("zone_unknown", zone));
    }
    if zone_short_name.value.is_none() {
        hazards.push(field_hazard("zone_short_name_unknown", zone_short_name));
    }
    if location.value.is_none() {
        hazards.push(field_hazard("location_unknown", location));
    }
    if level.value.is_none() {
        hazards.push(field_hazard("level_unknown", level));
    }
    if xp_percent.value.is_none() {
        hazards.push(field_hazard("xp_percent_unknown", xp_percent));
    }
    if batch.truncated_by_bytes || batch.truncated_by_events {
        hazards.push(EverQuestStateHazard {
            code: "log_sample_truncated".to_owned(),
            severity: "warning".to_owned(),
            detail: format!(
                "sample truncated_by_bytes={} truncated_by_events={}",
                batch.truncated_by_bytes, batch.truncated_by_events
            ),
        });
    }
    hazards
}

fn field_hazard<T>(code: &str, field: &EverQuestStateField<T>) -> EverQuestStateHazard {
    EverQuestStateHazard {
        code: code.to_owned(),
        severity: "warning".to_owned(),
        detail: field
            .note
            .clone()
            .unwrap_or_else(|| "field unavailable".to_owned()),
    }
}

fn field_some<T>(
    value: T,
    confidence: f32,
    source: EverQuestStateSource,
    note: Option<&str>,
) -> EverQuestStateField<T> {
    EverQuestStateField {
        value: Some(value),
        confidence,
        sources: vec![source],
        note: note.map(ToOwned::to_owned),
    }
}

fn field_none<T>(note: impl Into<String>) -> EverQuestStateField<T> {
    EverQuestStateField {
        value: None,
        confidence: 0.0,
        sources: Vec::new(),
        note: Some(note.into()),
    }
}

fn log_source(
    batch: &synapse_everquest::EverQuestLogTailBatch,
    event: &EverQuestLogEvent,
) -> EverQuestStateSource {
    EverQuestStateSource {
        kind: "everquest_log".to_owned(),
        path: Some(batch.path.display().to_string()),
        start_offset: Some(batch.start_offset),
        next_offset: Some(batch.next_offset),
        log_timestamp: Some(event.timestamp.format("%Y-%m-%dT%H:%M:%S").to_string()),
        summary: Some(event.summary.clone()),
    }
}

fn short_name_for_zone_display(zone: &str) -> Option<&'static str> {
    match zone.trim().to_ascii_lowercase().as_str() {
        "neriak - foreign quarter" => Some("neriaka"),
        "neriak - commons" => Some("neriakb"),
        "neriak - third gate" => Some("neriakc"),
        "nektulos forest" => Some("nektulos"),
        "east commonlands" => Some("ecommons"),
        "west commonlands" => Some("commons"),
        "commonlands" => Some("commonlands"),
        "lavastorm mountains" => Some("lavastorm"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_neriak_foreign_quarter_to_short_name() {
        assert_eq!(
            short_name_for_zone_display("Neriak - Foreign Quarter"),
            Some("neriaka")
        );
    }

    #[test]
    fn location_converts_display_order_to_local_map_axes() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("eqlog_Thenumberone_frostreaver.txt");
        std::fs::write(
            &path,
            "[Thu May 28 14:05:24 2026] Your Location is 50.94, 154.00, 31.19\r\n",
        )?;
        let batch = tail_log(&path, 0, MAX_STATE_LOG_BYTES, MAX_STATE_LOG_EVENTS)?;
        let field = latest_location_field(&batch.events, &batch);
        let location = field
            .value
            .as_ref()
            .unwrap_or_else(|| panic!("expected location"));
        assert_eq!(location.map_x, -154.0);
        assert_eq!(location.map_y, -50.94);
        assert_eq!(location.map_z, 31.19);
        Ok(())
    }

    #[test]
    fn location_uses_latest_loc_after_dense_log_window() -> anyhow::Result<()> {
        use std::fmt::Write as _;

        let dir = tempfile::tempdir()?;
        let path = dir.path().join("eqlog_Thenumberone_frostreaver.txt");
        let mut log = String::new();
        for index in 0..5000 {
            writeln!(
                log,
                "[Thu May 28 14:00:{:02} 2026] Dense filler event {index}",
                index % 60
            )?;
        }
        log.push_str("[Thu May 28 14:05:24 2026] Your Location is 50.94, 154.00, 31.19\r\n");
        log.push_str("[Thu May 28 14:06:24 2026] Your Location is 11.75, 146.88, 31.19\r\n");
        std::fs::write(&path, log)?;

        let batch = tail_log(&path, 0, MAX_STATE_LOG_BYTES, MAX_STATE_LOG_EVENTS)?;
        let field = latest_location_field(&batch.events, &batch);
        let location = field
            .value
            .as_ref()
            .unwrap_or_else(|| panic!("expected latest location"));
        assert_eq!(location.display_y, 11.75);
        assert_eq!(location.display_x, 146.88);
        assert_eq!(location.map_x, -146.88);
        assert_eq!(location.map_y, -11.75);
        assert_eq!(location.log_timestamp, "2026-05-28T14:06:24");
        Ok(())
    }

    #[test]
    fn action_filter_keeps_everquest_rows_only() {
        let everquest = serde_json::json!({
            "profile_id": EVERQUEST_PROFILE_ID,
            "foreground": {"process_name": "eqgame.exe"}
        });
        let notepad = serde_json::json!({
            "profile_id": "notepad",
            "foreground": {"process_name": "notepad.exe"}
        });
        assert!(action_row_matches_everquest(&everquest));
        assert!(!action_row_matches_everquest(&notepad));
    }
}
