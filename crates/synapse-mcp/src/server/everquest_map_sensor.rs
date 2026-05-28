use chrono::{DateTime, Utc};
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use synapse_core::{Rect, error_codes};
use synapse_everquest::{
    EverQuestMapCoord, EverQuestMapFile, EverQuestZoneEdge, EverQuestZoneGraph,
    EverQuestZoneLandmark, build_zone_graph_from_root, parse_map_file,
};

use super::{
    Json, Parameters, SynapseService,
    everquest_log::EVERQUEST_PROFILE_ID,
    everquest_state::{CURRENT_STATE_ROW_KEY, EverQuestCurrentState, EverQuestStateSource},
    tool, tool_router,
};
use crate::m1::{current_input, mcp_error};

const TOOL: &str = "everquest_map_sensor";
const SCHEMA_VERSION: u32 = 1;
const MAP_SENSOR_PREFIX: &str = "everquest/map_sensor/v1";
const DEFAULT_STALE_AFTER_SECONDS: u64 = 300;
const DEFAULT_MAX_NEAREST_LABELS: usize = 8;
const MAX_NEAREST_LABELS: usize = 16;
const MAX_ID_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 512;
const MAX_SOURCE_REFS: usize = 32;
const MAX_VISIBLE_LABELS: usize = 32;
const MIN_VISIBLE_CONFIDENCE: f32 = 0.50;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapSensorParams {
    pub sensor_id: String,
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    #[serde(default = "default_state_row_key")]
    pub state_row_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_override: Option<EverQuestMapSensorStateOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_map_override: Option<EverQuestVisibleMapOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_zone_short_name: Option<String>,
    #[serde(default = "default_stale_after_seconds")]
    pub stale_after_seconds: u64,
    #[serde(default = "default_max_nearest_labels")]
    pub max_nearest_labels: usize,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapSensorStateOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_short_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EverQuestMapSensorLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub source_refs: Vec<EverQuestMapSensorSourceRef>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestVisibleMapOverride {
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Rect>,
    #[serde(default = "default_full_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub occluded: bool,
    #[serde(default)]
    pub zoom_or_pan_changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_marker_screen: Option<EverQuestScreenPoint>,
    #[serde(default)]
    pub detected_labels: Vec<String>,
    #[serde(default)]
    pub source_refs: Vec<EverQuestMapSensorSourceRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapSensorResponse {
    pub ok: bool,
    pub row_key: String,
    pub stored_value_len_bytes: u64,
    pub sensor: EverQuestMapSensorRow,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapSensorRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub sensor_id: String,
    pub row_key: String,
    pub generated_at: DateTime<Utc>,
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstain_reason: Option<String>,
    pub source_state_row_key: String,
    pub source_mode: String,
    pub foreground: EverQuestMapSensorForeground,
    pub visible_map: EverQuestVisibleMapReadback,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_zone_short_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_location: Option<EverQuestMapSensorLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_source: Option<EverQuestMapSensorSourceFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration: Option<EverQuestMapCalibration>,
    pub nearest_labels: Vec<EverQuestMapSensorLandmark>,
    pub nearest_exits: Vec<EverQuestMapSensorLandmark>,
    pub hazards: Vec<EverQuestMapSensorHazard>,
    pub source_refs: Vec<EverQuestMapSensorSourceRef>,
    pub evidence_boundary: EverQuestMapSensorEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapSensorForeground {
    pub is_everquest_foreground: bool,
    pub hwnd: i64,
    pub process_name: String,
    pub window_title: String,
    pub window_bounds: Rect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestVisibleMapReadback {
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Rect>,
    pub confidence: f32,
    pub occluded: bool,
    pub zoom_or_pan_changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_marker_screen: Option<EverQuestScreenPoint>,
    pub detected_labels: Vec<String>,
    pub source_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestScreenPoint {
    pub x: f64,
    pub y: f64,
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapSensorLocation {
    pub map_x: f64,
    pub map_y: f64,
    pub map_z: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapSensorSourceFile {
    pub zone_short_name: String,
    pub path: String,
    pub len_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified_unix_ms: Option<i64>,
    pub sha256: String,
    pub line_count: usize,
    pub segment_count: usize,
    pub point_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapCalibration {
    pub calibrated: bool,
    pub method: String,
    pub confidence: f32,
    pub anchor_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_bounds: Option<Rect>,
    pub player_location: EverQuestMapSensorLocation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_marker_screen: Option<EverQuestScreenPoint>,
    pub visible_label_matches: Vec<EverQuestMapSensorLandmark>,
    pub transform: EverQuestMapTransform,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapTransform {
    pub transform_kind: String,
    pub map_anchor_x: f64,
    pub map_anchor_y: f64,
    pub map_anchor_z: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_anchor_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_anchor_y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixels_per_map_unit_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixels_per_map_unit_y: Option<f64>,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapSensorLandmark {
    pub label: String,
    pub zone_short_name: String,
    pub map_x: f64,
    pub map_y: f64,
    pub map_z: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_from_current: Option<f64>,
    pub confidence: f32,
    pub source_path: String,
    pub source_line_number: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_zone_short_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapSensorHazard {
    pub code: String,
    pub severity: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapSensorSourceRef {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_number: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMapSensorEvidenceBoundary {
    pub supports_planning: bool,
    pub movement_executed: bool,
    pub manual_fsv_required_for_runtime: bool,
    pub is_fsv: bool,
    pub redacted: bool,
    pub note: String,
}

#[derive(Clone, Debug)]
struct NormalizedParams {
    sensor_id: String,
    profile_id: String,
    state_row_key: String,
    state_override: Option<MapStateOverride>,
    visible_map_override: Option<VisibleMapEvidence>,
    expected_zone_short_name: Option<String>,
    stale_after_seconds: u64,
    max_nearest_labels: usize,
    row_key: String,
}

#[derive(Clone, Debug)]
struct MapStateOverride {
    zone_short_name: Option<String>,
    location: Option<EverQuestMapSensorLocation>,
    generated_at: Option<DateTime<Utc>>,
    confidence: f32,
    source_refs: Vec<EverQuestMapSensorSourceRef>,
}

#[derive(Clone, Debug)]
struct VisibleMapEvidence {
    readback: EverQuestVisibleMapReadback,
    source_refs: Vec<EverQuestMapSensorSourceRef>,
}

#[derive(Clone, Debug)]
struct MapSourceState {
    source_mode: String,
    state_row_key: String,
    generated_at: Option<DateTime<Utc>>,
    zone_short_name: Option<String>,
    zone_confidence: f32,
    location: Option<EverQuestMapSensorLocation>,
    location_confidence: f32,
    source_refs: Vec<EverQuestMapSensorSourceRef>,
}

#[tool_router(router = everquest_map_sensor_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Persist one calibrated EverQuest current-map sensor row from visible map evidence, /loc, and local map files"
    )]
    pub async fn everquest_map_sensor(
        &self,
        params: Parameters<EverQuestMapSensorParams>,
    ) -> Result<Json<EverQuestMapSensorResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TOOL,
            "tool.invocation kind=everquest_map_sensor"
        );
        let normalized = normalize_params(params.0)?;
        let mut input = {
            let state = self.m1_state()?;
            current_input(&state, 2)?
        };
        self.resolve_input_profile_and_hud(&mut input, true);
        let foreground = map_foreground(&input.foreground);
        let source_state = self.map_source_state(&normalized)?;
        let active = self.resolve_active_everquest_log().map_err(|detail| {
            mcp_error(
                error_codes::ACTION_TARGET_INVALID,
                format!("{TOOL} could not resolve active EverQuest install/log: {detail}"),
            )
        })?;
        let graph = build_zone_graph_from_root(&active.install_root).map_err(|error| {
            mcp_error(
                error_codes::STORAGE_READ_FAILED,
                format!("{TOOL} could not build EverQuest zone graph: {error}"),
            )
        })?;
        let row = map_sensor_row(&normalized, foreground, &source_state, &graph)?;
        let (sensor, stored_value_len_bytes) =
            self.persist_map_sensor_json(&normalized.row_key, &row)?;
        Ok(Json(EverQuestMapSensorResponse {
            ok: true,
            row_key: normalized.row_key,
            stored_value_len_bytes,
            sensor,
        }))
    }
}

impl SynapseService {
    fn map_source_state(&self, params: &NormalizedParams) -> Result<MapSourceState, ErrorData> {
        if let Some(override_state) = &params.state_override {
            return Ok(MapSourceState {
                source_mode: "state_override".to_owned(),
                state_row_key: params.state_row_key.clone(),
                generated_at: override_state.generated_at,
                zone_short_name: override_state.zone_short_name.clone(),
                zone_confidence: override_state.confidence,
                location: override_state.location.clone(),
                location_confidence: override_state.confidence,
                source_refs: override_state.source_refs.clone(),
            });
        }
        let stored = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while reading EverQuest current-state row",
                )
            })?;
            runtime
                .storage_kv_row(params.state_row_key.as_bytes())
                .map_err(|error| mcp_error(error.code(), error.to_string()))?
        };
        let Some(stored) = stored else {
            return Ok(MapSourceState {
                source_mode: "current_state_row_missing".to_owned(),
                state_row_key: params.state_row_key.clone(),
                generated_at: None,
                zone_short_name: None,
                zone_confidence: 0.0,
                location: None,
                location_confidence: 0.0,
                source_refs: vec![EverQuestMapSensorSourceRef {
                    kind: "synapse_storage_missing".to_owned(),
                    row_key: Some(params.state_row_key.clone()),
                    path: None,
                    line_number: None,
                    start_offset: None,
                    next_offset: None,
                    summary: Some("current-state row was absent before map sensing".to_owned()),
                }],
            });
        };
        let state =
            decode_json_row::<EverQuestCurrentState>(&stored, "EverQuest current-state row")?;
        Ok(source_state_from_current_row(&params.state_row_key, &state))
    }

    fn persist_map_sensor_json(
        &self,
        key: &str,
        row: &EverQuestMapSensorRow,
    ) -> Result<(EverQuestMapSensorRow, u64), ErrorData> {
        let encoded = serde_json::to_vec(row).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode EverQuest map-sensor row: {error}"),
            )
        })?;
        let stored = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while writing EverQuest map-sensor row",
                )
            })?;
            runtime
                .storage_put_kv_rows(vec![(key.as_bytes().to_vec(), encoded)])
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_WRITE_FAILED,
                        format!("write EverQuest map-sensor row: {error}"),
                    )
                })?;
            runtime
                .storage_kv_row(key.as_bytes())
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        format!("read EverQuest map-sensor row after write: {error}"),
                    )
                })?
                .ok_or_else(|| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        "EverQuest map-sensor row missing after write",
                    )
                })?
        };
        let readback =
            decode_json_row::<EverQuestMapSensorRow>(&stored, "EverQuest map-sensor row")?;
        Ok((readback, len_to_u64(stored.len())))
    }
}

fn normalize_params(params: EverQuestMapSensorParams) -> Result<NormalizedParams, ErrorData> {
    let profile_id = validate_everquest_profile_id(&params.profile_id)?;
    let sensor_id = validate_id("sensor_id", &params.sensor_id)?;
    let state_row_key = normalize_required_text("state_row_key", &params.state_row_key)?;
    let expected_zone_short_name = params
        .expected_zone_short_name
        .map(|value| validate_id("expected_zone_short_name", &value))
        .transpose()?;
    if params.stale_after_seconds == 0 {
        return Err(params_error("stale_after_seconds must be >= 1"));
    }
    if params.max_nearest_labels == 0 || params.max_nearest_labels > MAX_NEAREST_LABELS {
        return Err(params_error(format!(
            "max_nearest_labels must be between 1 and {MAX_NEAREST_LABELS}"
        )));
    }
    let state_override = params
        .state_override
        .map(normalize_state_override)
        .transpose()?;
    let visible_map_override = params
        .visible_map_override
        .map(normalize_visible_map_override)
        .transpose()?;
    let row_key = map_sensor_row_key(&profile_id, &sensor_id);
    Ok(NormalizedParams {
        sensor_id,
        profile_id,
        state_row_key,
        state_override,
        visible_map_override,
        expected_zone_short_name,
        stale_after_seconds: params.stale_after_seconds,
        max_nearest_labels: params.max_nearest_labels,
        row_key,
    })
}

fn normalize_state_override(
    override_state: EverQuestMapSensorStateOverride,
) -> Result<MapStateOverride, ErrorData> {
    let zone_short_name = override_state
        .zone_short_name
        .map(|value| validate_id("state_override.zone_short_name", &value))
        .transpose()?;
    let confidence = override_state.confidence.unwrap_or(0.75);
    validate_unit_interval("state_override.confidence", confidence)?;
    let source_refs =
        normalize_source_refs("state_override.source_refs", override_state.source_refs)?;
    if source_refs.is_empty() {
        return Err(params_error(
            "state_override.source_refs must contain at least one physical SoT reference",
        ));
    }
    Ok(MapStateOverride {
        zone_short_name,
        location: override_state.location,
        generated_at: override_state.generated_at,
        confidence,
        source_refs,
    })
}

fn normalize_visible_map_override(
    visible: EverQuestVisibleMapOverride,
) -> Result<VisibleMapEvidence, ErrorData> {
    validate_unit_interval("visible_map_override.confidence", visible.confidence)?;
    if visible.visible && visible.bounds.is_none() {
        return Err(params_error(
            "visible_map_override.bounds is required when visible=true",
        ));
    }
    let visible_revision = visible
        .visible_revision
        .map(|value| normalize_required_text("visible_map_override.visible_revision", &value))
        .transpose()?;
    if visible.detected_labels.len() > MAX_VISIBLE_LABELS {
        return Err(params_error(format!(
            "visible_map_override.detected_labels must contain <= {MAX_VISIBLE_LABELS} labels"
        )));
    }
    let detected_labels = visible
        .detected_labels
        .into_iter()
        .map(|label| normalize_required_text("visible_map_override.detected_labels[]", &label))
        .collect::<Result<Vec<_>, _>>()?;
    let source_refs =
        normalize_source_refs("visible_map_override.source_refs", visible.source_refs)?;
    if source_refs.is_empty() {
        return Err(params_error(
            "visible_map_override.source_refs must contain at least one physical screenshot/observe source",
        ));
    }
    Ok(VisibleMapEvidence {
        readback: EverQuestVisibleMapReadback {
            visible: visible.visible,
            bounds: visible.bounds,
            confidence: visible.confidence,
            occluded: visible.occluded,
            zoom_or_pan_changed: visible.zoom_or_pan_changed,
            visible_revision,
            player_marker_screen: visible.player_marker_screen,
            detected_labels,
            source_mode: "visible_map_override".to_owned(),
            note: None,
        },
        source_refs,
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "sensor row assembly keeps every fail-closed branch in one auditable state transition"
)]
fn map_sensor_row(
    params: &NormalizedParams,
    foreground: EverQuestMapSensorForeground,
    source: &MapSourceState,
    graph: &EverQuestZoneGraph,
) -> Result<EverQuestMapSensorRow, ErrorData> {
    let visible_evidence = params
        .visible_map_override
        .clone()
        .unwrap_or_else(|| auto_visible_map_evidence(&foreground));
    let mut source_refs = source.source_refs.clone();
    source_refs.extend(visible_evidence.source_refs.clone());
    source_refs.truncate(MAX_SOURCE_REFS);
    let mut row = EverQuestMapSensorRow {
        schema_version: SCHEMA_VERSION,
        row_kind: "everquest_current_map_sensor".to_owned(),
        profile_id: params.profile_id.clone(),
        sensor_id: params.sensor_id.clone(),
        row_key: params.row_key.clone(),
        generated_at: Utc::now(),
        decision: "abstain_uninitialized".to_owned(),
        abstain_reason: None,
        source_state_row_key: source.state_row_key.clone(),
        source_mode: source.source_mode.clone(),
        foreground,
        visible_map: visible_evidence.readback,
        current_zone_short_name: source.zone_short_name.clone(),
        current_location: source.location.clone(),
        map_source: None,
        calibration: None,
        nearest_labels: Vec::new(),
        nearest_exits: Vec::new(),
        hazards: Vec::new(),
        source_refs,
        evidence_boundary: evidence_boundary(),
    };

    if source.source_mode == "current_state_row_missing" {
        abstain(
            &mut row,
            "abstain_current_state_missing",
            "current-state row is absent",
        );
        return Ok(row);
    }
    if is_stale_source(source.generated_at, params.stale_after_seconds) {
        row.hazards.push(EverQuestMapSensorHazard {
            code: "stale_current_state".to_owned(),
            severity: "warning".to_owned(),
            detail: format!(
                "current state is older than {} seconds",
                params.stale_after_seconds
            ),
        });
        abstain(
            &mut row,
            "abstain_stale_current_state",
            "current state is stale; refresh /loc/current_state before map sensing",
        );
        return Ok(row);
    }
    if !row.foreground.is_everquest_foreground {
        row.hazards.push(EverQuestMapSensorHazard {
            code: "non_everquest_foreground".to_owned(),
            severity: "warning".to_owned(),
            detail: format!(
                "foreground is {} title {:?}",
                row.foreground.process_name, row.foreground.window_title
            ),
        });
        abstain(
            &mut row,
            "abstain_non_everquest_foreground",
            "EverQuest is not the verified foreground window",
        );
        return Ok(row);
    }
    let Some(zone_short_name) = source.zone_short_name.as_deref() else {
        abstain(
            &mut row,
            "abstain_unknown_current_zone",
            "current zone short name is unknown",
        );
        return Ok(row);
    };
    if let Some(expected) = params.expected_zone_short_name.as_deref()
        && !expected.eq_ignore_ascii_case(zone_short_name)
    {
        row.hazards.push(EverQuestMapSensorHazard {
            code: "zone_source_conflict".to_owned(),
            severity: "high".to_owned(),
            detail: format!(
                "current-state zone {zone_short_name:?} conflicts with expected map zone {expected:?}"
            ),
        });
        abstain(
            &mut row,
            "abstain_contradictory_zone_source",
            "current-state zone and expected visible-map zone conflict",
        );
        return Ok(row);
    }
    let Some(current_location) = source.location.as_ref() else {
        abstain(
            &mut row,
            "abstain_no_current_loc",
            "current /loc map coordinate is unknown",
        );
        return Ok(row);
    };
    let (map_file, map_source) = map_file_for_zone(graph, zone_short_name)?;
    row.map_source = Some(map_source);
    let current_coord = location_to_coord(current_location);
    row.nearest_labels = graph
        .nearest_landmarks(zone_short_name, &current_coord, params.max_nearest_labels)
        .into_iter()
        .map(|nearest| {
            landmark_from_zone_landmark(&nearest.landmark, Some(nearest.distance), 0.80, None)
        })
        .collect();
    row.nearest_exits = nearest_exits(graph, zone_short_name, current_location, 4);

    if !row.visible_map.visible {
        let reason = row
            .visible_map
            .note
            .clone()
            .unwrap_or_else(|| "visible map window evidence is absent".to_owned());
        abstain(&mut row, "abstain_map_not_visible", reason);
        return Ok(row);
    }
    if row.visible_map.occluded {
        row.hazards.push(EverQuestMapSensorHazard {
            code: "map_occluded".to_owned(),
            severity: "warning".to_owned(),
            detail: "visible map window is marked occluded".to_owned(),
        });
        abstain(
            &mut row,
            "abstain_map_occluded",
            "visible map window is occluded; calibration would be unsafe",
        );
        return Ok(row);
    }
    if row.visible_map.zoom_or_pan_changed {
        row.hazards.push(EverQuestMapSensorHazard {
            code: "map_zoom_or_pan_changed".to_owned(),
            severity: "warning".to_owned(),
            detail: "visible map zoom/pan revision changed after calibration".to_owned(),
        });
        abstain(
            &mut row,
            "abstain_recalibration_required",
            "visible map zoom or pan changed; recalibration is required",
        );
        return Ok(row);
    }
    if row.visible_map.confidence < MIN_VISIBLE_CONFIDENCE {
        let confidence = row.visible_map.confidence;
        abstain(
            &mut row,
            "abstain_low_visible_confidence",
            format!(
                "visible map confidence {confidence:.3} is below threshold {MIN_VISIBLE_CONFIDENCE:.2}"
            ),
        );
        return Ok(row);
    }

    let visible_label_matches = visible_label_matches(
        &map_file,
        zone_short_name,
        current_location,
        &row.visible_map.detected_labels,
    );
    let anchor_count =
        visible_label_matches.len() + usize::from(row.visible_map.player_marker_screen.is_some());
    let confidence =
        (row.visible_map.confidence * source.zone_confidence * source.location_confidence)
            .clamp(0.0, 1.0);
    row.calibration = Some(EverQuestMapCalibration {
        calibrated: anchor_count > 0 && confidence >= MIN_VISIBLE_CONFIDENCE,
        method: if row.visible_map.player_marker_screen.is_some() {
            "loc_plus_visible_player_marker".to_owned()
        } else if visible_label_matches.is_empty() {
            "loc_plus_visible_bounds_without_label_anchor".to_owned()
        } else {
            "loc_plus_visible_map_labels".to_owned()
        },
        confidence,
        anchor_count,
        map_bounds: row.visible_map.bounds,
        player_location: current_location.clone(),
        player_marker_screen: row.visible_map.player_marker_screen.clone(),
        visible_label_matches,
        transform: EverQuestMapTransform {
            transform_kind: if anchor_count > 0 {
                "translation_anchor_only".to_owned()
            } else {
                "uncalibrated_loc_anchor_only".to_owned()
            },
            map_anchor_x: current_location.map_x,
            map_anchor_y: current_location.map_y,
            map_anchor_z: current_location.map_z,
            screen_anchor_x: row.visible_map.player_marker_screen.as_ref().map(|point| point.x),
            screen_anchor_y: row.visible_map.player_marker_screen.as_ref().map(|point| point.y),
            pixels_per_map_unit_x: None,
            pixels_per_map_unit_y: None,
            note: "One /loc point establishes the player map anchor; scale/rotation remain unset until a second visible label or marker anchor is verified."
                .to_owned(),
        },
    });
    if let Some(calibration) = &row.calibration
        && calibration.calibrated
    {
        "calibrated".clone_into(&mut row.decision);
    } else {
        "uncalibrated_visible_map".clone_into(&mut row.decision);
        row.hazards.push(EverQuestMapSensorHazard {
            code: "insufficient_visible_anchors".to_owned(),
            severity: "warning".to_owned(),
            detail: "visible map was present but no visible label/player marker anchor matched the local map"
                .to_owned(),
        });
    }
    Ok(row)
}

fn map_file_for_zone(
    graph: &EverQuestZoneGraph,
    zone_short_name: &str,
) -> Result<(EverQuestMapFile, EverQuestMapSensorSourceFile), ErrorData> {
    let node = graph.node(zone_short_name).ok_or_else(|| {
        mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!("EverQuest map graph has no node for zone {zone_short_name:?}"),
        )
    })?;
    let map_file = parse_map_file(&node.source_path).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!(
                "parse EverQuest map file {}: {error}",
                node.source_path.display()
            ),
        )
    })?;
    let source = source_file_from_map(&map_file)?;
    Ok((map_file, source))
}

fn source_file_from_map(
    map_file: &EverQuestMapFile,
) -> Result<EverQuestMapSensorSourceFile, ErrorData> {
    let bytes = std::fs::read(&map_file.source.path).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!(
                "read EverQuest map file {}: {error}",
                map_file.source.path.display()
            ),
        )
    })?;
    Ok(EverQuestMapSensorSourceFile {
        zone_short_name: map_file.source.zone_short_name.clone(),
        path: map_file.source.path.display().to_string(),
        len_bytes: map_file.source.len_bytes,
        last_modified_unix_ms: map_file.source.last_modified_unix_ms,
        sha256: sha256_hex(&bytes),
        line_count: map_file.line_count,
        segment_count: map_file.segment_count,
        point_count: map_file.point_count,
    })
}

fn visible_label_matches(
    map_file: &EverQuestMapFile,
    zone_short_name: &str,
    current_location: &EverQuestMapSensorLocation,
    labels: &[String],
) -> Vec<EverQuestMapSensorLandmark> {
    labels
        .iter()
        .filter_map(|label| {
            let normalized = normalize_label(label);
            map_file.records.iter().find_map(|record| {
                let synapse_everquest::EverQuestMapRecord::Point(point) = record else {
                    return None;
                };
                if normalize_label(&point.label) == normalized {
                    Some(EverQuestMapSensorLandmark {
                        label: point.label.clone(),
                        zone_short_name: zone_short_name.to_owned(),
                        map_x: point.location.x,
                        map_y: point.location.y,
                        map_z: point.location.z,
                        distance_from_current: Some(distance(
                            &location_to_coord(current_location),
                            &point.location,
                        )),
                        confidence: 0.85,
                        source_path: point.source_path.display().to_string(),
                        source_line_number: point.source_line_number,
                        target_zone_short_name: None,
                    })
                } else {
                    None
                }
            })
        })
        .collect()
}

fn nearest_exits(
    graph: &EverQuestZoneGraph,
    zone_short_name: &str,
    current_location: &EverQuestMapSensorLocation,
    max_items: usize,
) -> Vec<EverQuestMapSensorLandmark> {
    let mut exits = graph.exits_for_zone(zone_short_name);
    exits.sort_by(|left, right| {
        distance(&left.location, &location_to_coord(current_location)).total_cmp(&distance(
            &right.location,
            &location_to_coord(current_location),
        ))
    });
    exits
        .into_iter()
        .take(max_items)
        .map(|edge| landmark_from_edge(&edge, current_location))
        .collect()
}

fn source_state_from_current_row(row_key: &str, state: &EverQuestCurrentState) -> MapSourceState {
    let location = state
        .location
        .value
        .as_ref()
        .map(|location| EverQuestMapSensorLocation {
            map_x: location.map_x,
            map_y: location.map_y,
            map_z: location.map_z,
        });
    let mut source_refs = vec![EverQuestMapSensorSourceRef {
        kind: "synapse_storage".to_owned(),
        row_key: Some(row_key.to_owned()),
        path: None,
        line_number: None,
        start_offset: None,
        next_offset: None,
        summary: Some("persisted EverQuest current-state row".to_owned()),
    }];
    source_refs.extend(state_source_refs(&state.zone_short_name.sources));
    source_refs.extend(state_source_refs(&state.location.sources));
    source_refs.truncate(MAX_SOURCE_REFS);
    MapSourceState {
        source_mode: "current_state_row".to_owned(),
        state_row_key: row_key.to_owned(),
        generated_at: Some(state.generated_at),
        zone_short_name: state.zone_short_name.value.clone(),
        zone_confidence: state.zone_short_name.confidence,
        location,
        location_confidence: state.location.confidence,
        source_refs,
    }
}

fn state_source_refs(sources: &[EverQuestStateSource]) -> Vec<EverQuestMapSensorSourceRef> {
    sources
        .iter()
        .map(|source| EverQuestMapSensorSourceRef {
            kind: source.kind.clone(),
            row_key: None,
            path: source.path.clone(),
            line_number: None,
            start_offset: source.start_offset,
            next_offset: source.next_offset,
            summary: source.summary.clone(),
        })
        .collect()
}

fn map_foreground(foreground: &synapse_core::ForegroundContext) -> EverQuestMapSensorForeground {
    let is_everquest_foreground = foreground.profile_id.as_deref() == Some(EVERQUEST_PROFILE_ID)
        && foreground.process_name.eq_ignore_ascii_case("eqgame.exe");
    EverQuestMapSensorForeground {
        is_everquest_foreground,
        hwnd: foreground.hwnd,
        process_name: foreground.process_name.clone(),
        window_title: foreground.window_title.clone(),
        window_bounds: foreground.window_bounds,
        profile_id: foreground.profile_id.clone(),
    }
}

fn auto_visible_map_evidence(foreground: &EverQuestMapSensorForeground) -> VisibleMapEvidence {
    VisibleMapEvidence {
        readback: EverQuestVisibleMapReadback {
            visible: false,
            bounds: None,
            confidence: 0.0,
            occluded: false,
            zoom_or_pan_changed: false,
            visible_revision: None,
            player_marker_screen: None,
            detected_labels: Vec::new(),
            source_mode: "foreground_auto_fail_closed".to_owned(),
            note: Some(
                "automatic map-window detector has no verified visible-map evidence; provide visible_map_override from a physical observe/screenshot readback"
                    .to_owned(),
            ),
        },
        source_refs: vec![EverQuestMapSensorSourceRef {
            kind: "foreground_observation".to_owned(),
            row_key: None,
            path: None,
            line_number: None,
            start_offset: None,
            next_offset: None,
            summary: Some(format!(
                "foreground hwnd={} process={} title={:?}",
                foreground.hwnd, foreground.process_name, foreground.window_title
            )),
        }],
    }
}

fn landmark_from_zone_landmark(
    landmark: &EverQuestZoneLandmark,
    distance_from_current: Option<f64>,
    confidence: f32,
    target_zone_short_name: Option<String>,
) -> EverQuestMapSensorLandmark {
    EverQuestMapSensorLandmark {
        label: landmark.label.clone(),
        zone_short_name: landmark.zone_short_name.clone(),
        map_x: landmark.location.x,
        map_y: landmark.location.y,
        map_z: landmark.location.z,
        distance_from_current,
        confidence,
        source_path: landmark.source_path.display().to_string(),
        source_line_number: landmark.source_line_number,
        target_zone_short_name,
    }
}

fn landmark_from_edge(
    edge: &EverQuestZoneEdge,
    current_location: &EverQuestMapSensorLocation,
) -> EverQuestMapSensorLandmark {
    EverQuestMapSensorLandmark {
        label: edge.label.clone(),
        zone_short_name: edge.source_zone_short_name.clone(),
        map_x: edge.location.x,
        map_y: edge.location.y,
        map_z: edge.location.z,
        distance_from_current: Some(distance(
            &location_to_coord(current_location),
            &edge.location,
        )),
        confidence: edge.confidence,
        source_path: edge.source_path.display().to_string(),
        source_line_number: edge.source_line_number,
        target_zone_short_name: edge.target_zone_short_name.clone(),
    }
}

fn abstain(
    row: &mut EverQuestMapSensorRow,
    decision: impl Into<String>,
    reason: impl Into<String>,
) {
    row.decision = decision.into();
    row.abstain_reason = Some(reason.into());
    row.calibration = None;
}

fn evidence_boundary() -> EverQuestMapSensorEvidenceBoundary {
    EverQuestMapSensorEvidenceBoundary {
        supports_planning: true,
        movement_executed: false,
        manual_fsv_required_for_runtime: true,
        is_fsv: false,
        redacted: true,
        note: "Map-sensor rows are compact planning evidence only; movement and leveling still require separate attended action FSV and UI/log/storage readback."
            .to_owned(),
    }
}

const fn location_to_coord(location: &EverQuestMapSensorLocation) -> EverQuestMapCoord {
    EverQuestMapCoord {
        x: location.map_x,
        y: location.map_y,
        z: location.map_z,
    }
}

fn distance(left: &EverQuestMapCoord, right: &EverQuestMapCoord) -> f64 {
    let dx = left.x - right.x;
    let dy = left.y - right.y;
    let dz = left.z - right.z;
    dx.mul_add(dx, dy.mul_add(dy, dz * dz)).sqrt()
}

fn is_stale_source(source_generated_at: Option<DateTime<Utc>>, stale_after_seconds: u64) -> bool {
    source_generated_at.is_some_and(|generated_at| {
        let age = Utc::now().signed_duration_since(generated_at);
        age.num_seconds() > i64::try_from(stale_after_seconds).unwrap_or(i64::MAX)
    })
}

fn normalize_source_refs(
    field: &str,
    refs: Vec<EverQuestMapSensorSourceRef>,
) -> Result<Vec<EverQuestMapSensorSourceRef>, ErrorData> {
    if refs.len() > MAX_SOURCE_REFS {
        return Err(params_error(format!(
            "{field} must contain <= {MAX_SOURCE_REFS} refs"
        )));
    }
    refs.into_iter()
        .enumerate()
        .map(|(index, source)| normalize_source_ref(&format!("{field}[{index}]"), source))
        .collect()
}

fn normalize_source_ref(
    field: &str,
    source: EverQuestMapSensorSourceRef,
) -> Result<EverQuestMapSensorSourceRef, ErrorData> {
    Ok(EverQuestMapSensorSourceRef {
        kind: normalize_required_text(&format!("{field}.kind"), &source.kind)?,
        row_key: source
            .row_key
            .map(|value| normalize_required_text(&format!("{field}.row_key"), &value))
            .transpose()?,
        path: source
            .path
            .map(|value| normalize_required_text(&format!("{field}.path"), &value))
            .transpose()?,
        line_number: source.line_number,
        start_offset: source.start_offset,
        next_offset: source.next_offset,
        summary: source
            .summary
            .map(|value| normalize_required_text(&format!("{field}.summary"), &value))
            .transpose()?,
    })
}

fn validate_everquest_profile_id(value: &str) -> Result<String, ErrorData> {
    let profile_id = normalize_required_text("profile_id", value)?;
    if profile_id != EVERQUEST_PROFILE_ID {
        return Err(params_error(format!(
            "profile_id must be {EVERQUEST_PROFILE_ID:?}; got {profile_id:?}"
        )));
    }
    Ok(profile_id)
}

fn validate_id(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error(format!("{field} must not be empty")));
    }
    if value.len() > MAX_ID_BYTES {
        return Err(params_error(format!(
            "{field} must be <= {MAX_ID_BYTES} bytes"
        )));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(params_error(format!(
            "{field} may contain only ASCII letters, digits, '.', '_', and '-'"
        )));
    }
    Ok(value.to_owned())
}

fn normalize_required_text(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error(format!(
            "{field} must not be empty when present"
        )));
    }
    if value.len() > MAX_TEXT_BYTES {
        return Err(params_error(format!(
            "{field} must be <= {MAX_TEXT_BYTES} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(params_error(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(value.to_owned())
}

fn validate_unit_interval(field: &str, value: f32) -> Result<(), ErrorData> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(params_error(format!(
            "{field} must be a finite value between 0.0 and 1.0"
        )));
    }
    Ok(())
}

fn decode_json_row<T>(bytes: &[u8], label: &str) -> Result<T, ErrorData>
where
    T: DeserializeOwned,
{
    serde_json::from_slice::<T>(bytes).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("decode {label}: {error}"),
        )
    })
}

fn normalize_label(label: &str) -> String {
    label
        .chars()
        .flat_map(char::to_lowercase)
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

fn map_sensor_row_key(profile_id: &str, sensor_id: &str) -> String {
    format!("{MAP_SENSOR_PREFIX}/{profile_id}/{sensor_id}")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(&digest)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn default_profile_id() -> String {
    EVERQUEST_PROFILE_ID.to_owned()
}

fn default_state_row_key() -> String {
    CURRENT_STATE_ROW_KEY.to_owned()
}

const fn default_stale_after_seconds() -> u64 {
    DEFAULT_STALE_AFTER_SECONDS
}

const fn default_max_nearest_labels() -> usize {
    DEFAULT_MAX_NEAREST_LABELS
}

const fn default_full_confidence() -> f32 {
    1.0
}

fn len_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn params_error(message: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, message.into())
}

#[cfg(test)]
mod tests {
    use synapse_everquest::{
        EverQuestZoneEdge, EverQuestZoneEdgeResolution, EverQuestZoneLandmark, EverQuestZoneNode,
    };

    use super::*;

    #[test]
    fn calibrated_visible_map_with_label_anchor() -> Result<(), ErrorData> {
        let params = params("happy");
        let source = source_state(Some("neriaka"), Some(location(154.0, 50.94, 31.19)), 0.95);
        let foreground = foreground(true);
        let (_temp, graph) = graph_fixture();

        let row = map_sensor_row(&params, foreground, &source, &graph)?;

        assert_eq!(row.decision, "calibrated");
        let calibration = row.calibration.expect("calibration");
        assert!(calibration.calibrated);
        assert_eq!(calibration.anchor_count, 2);
        assert_eq!(
            calibration.visible_label_matches[0].label,
            "to_Nektulos_Forest"
        );
        assert_eq!(
            row.nearest_exits[0].target_zone_short_name.as_deref(),
            Some("nektulos")
        );
        Ok(())
    }

    #[test]
    fn hidden_map_abstains_after_map_readback() -> Result<(), ErrorData> {
        let mut params = params("hidden");
        params.visible_map_override = Some(VisibleMapEvidence {
            readback: EverQuestVisibleMapReadback {
                visible: false,
                bounds: None,
                confidence: 0.0,
                occluded: false,
                zoom_or_pan_changed: false,
                visible_revision: None,
                player_marker_screen: None,
                detected_labels: Vec::new(),
                source_mode: "visible_map_override".to_owned(),
                note: Some("synthetic hidden map".to_owned()),
            },
            source_refs: refs("manual_hidden"),
        });
        let (_temp, graph) = graph_fixture();
        let row = map_sensor_row(
            &params,
            foreground(true),
            &source_state(Some("neriaka"), Some(location(154.0, 50.94, 31.19)), 0.95),
            &graph,
        )?;

        assert_eq!(row.decision, "abstain_map_not_visible");
        assert_eq!(row.nearest_labels.len(), 2);
        assert!(row.calibration.is_none());
        Ok(())
    }

    #[test]
    fn occluded_map_abstains_fail_closed() -> Result<(), ErrorData> {
        let mut params = params("occluded");
        if let Some(visible) = params.visible_map_override.as_mut() {
            visible.readback.occluded = true;
        }
        let (_temp, graph) = graph_fixture();
        let row = map_sensor_row(
            &params,
            foreground(true),
            &source_state(Some("neriaka"), Some(location(154.0, 50.94, 31.19)), 0.95),
            &graph,
        )?;

        assert_eq!(row.decision, "abstain_map_occluded");
        assert_eq!(row.hazards[0].code, "map_occluded");
        Ok(())
    }

    #[test]
    fn zone_conflict_abstains_before_calibration() -> Result<(), ErrorData> {
        let mut params = params("zone-conflict");
        params.expected_zone_short_name = Some("nektulos".to_owned());
        let (_temp, graph) = graph_fixture();
        let row = map_sensor_row(
            &params,
            foreground(true),
            &source_state(Some("neriaka"), Some(location(154.0, 50.94, 31.19)), 0.95),
            &graph,
        )?;

        assert_eq!(row.decision, "abstain_contradictory_zone_source");
        assert_eq!(row.hazards[0].code, "zone_source_conflict");
        assert!(row.map_source.is_none());
        Ok(())
    }

    #[test]
    fn zoom_or_pan_change_requires_recalibration() -> Result<(), ErrorData> {
        let mut params = params("zoom-change");
        if let Some(visible) = params.visible_map_override.as_mut() {
            visible.readback.zoom_or_pan_changed = true;
        }
        let (_temp, graph) = graph_fixture();
        let row = map_sensor_row(
            &params,
            foreground(true),
            &source_state(Some("neriaka"), Some(location(154.0, 50.94, 31.19)), 0.95),
            &graph,
        )?;

        assert_eq!(row.decision, "abstain_recalibration_required");
        assert_eq!(row.hazards[0].code, "map_zoom_or_pan_changed");
        Ok(())
    }

    fn params(sensor_id: &str) -> NormalizedParams {
        NormalizedParams {
            sensor_id: sensor_id.to_owned(),
            profile_id: EVERQUEST_PROFILE_ID.to_owned(),
            state_row_key: CURRENT_STATE_ROW_KEY.to_owned(),
            state_override: None,
            visible_map_override: Some(VisibleMapEvidence {
                readback: EverQuestVisibleMapReadback {
                    visible: true,
                    bounds: Some(Rect {
                        x: 100,
                        y: 100,
                        w: 600,
                        h: 480,
                    }),
                    confidence: 0.90,
                    occluded: false,
                    zoom_or_pan_changed: false,
                    visible_revision: Some("rev-a".to_owned()),
                    player_marker_screen: Some(EverQuestScreenPoint { x: 320.0, y: 260.0 }),
                    detected_labels: vec!["to_Nektulos_Forest".to_owned()],
                    source_mode: "visible_map_override".to_owned(),
                    note: None,
                },
                source_refs: refs("manual_visible_map"),
            }),
            expected_zone_short_name: Some("neriaka".to_owned()),
            stale_after_seconds: DEFAULT_STALE_AFTER_SECONDS,
            max_nearest_labels: DEFAULT_MAX_NEAREST_LABELS,
            row_key: map_sensor_row_key(EVERQUEST_PROFILE_ID, sensor_id),
        }
    }

    fn source_state(
        zone_short_name: Option<&str>,
        location: Option<EverQuestMapSensorLocation>,
        confidence: f32,
    ) -> MapSourceState {
        MapSourceState {
            source_mode: "state_override".to_owned(),
            state_row_key: CURRENT_STATE_ROW_KEY.to_owned(),
            generated_at: Some(Utc::now()),
            zone_short_name: zone_short_name.map(str::to_owned),
            zone_confidence: if zone_short_name.is_some() {
                confidence
            } else {
                0.0
            },
            location,
            location_confidence: confidence,
            source_refs: refs("unit_test_state"),
        }
    }

    fn refs(kind: &str) -> Vec<EverQuestMapSensorSourceRef> {
        vec![EverQuestMapSensorSourceRef {
            kind: kind.to_owned(),
            row_key: Some("synthetic-row".to_owned()),
            path: None,
            line_number: None,
            start_offset: None,
            next_offset: None,
            summary: Some("synthetic source".to_owned()),
        }]
    }

    fn location(map_x: f64, map_y: f64, map_z: f64) -> EverQuestMapSensorLocation {
        EverQuestMapSensorLocation {
            map_x,
            map_y,
            map_z,
        }
    }

    fn foreground(is_everquest_foreground: bool) -> EverQuestMapSensorForeground {
        EverQuestMapSensorForeground {
            is_everquest_foreground,
            hwnd: 42,
            process_name: if is_everquest_foreground {
                "eqgame.exe".to_owned()
            } else {
                "notepad.exe".to_owned()
            },
            window_title: if is_everquest_foreground {
                "EverQuest".to_owned()
            } else {
                "Untitled - Notepad".to_owned()
            },
            window_bounds: Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
            profile_id: if is_everquest_foreground {
                Some(EVERQUEST_PROFILE_ID.to_owned())
            } else {
                None
            },
        }
    }

    fn graph_fixture() -> (tempfile::TempDir, EverQuestZoneGraph) {
        let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        let neriaka = temp.path().join("neriaka.txt");
        let nektulos = temp.path().join("nektulos.txt");
        std::fs::write(
            &neriaka,
            "P 160, 55, 31, 0, 0, 0, 3, Priest_of_Discord\nP -155.1781, -20.6847, 28.6260, 0, 0, 0, 3, to_Nektulos_Forest\n",
        )
        .unwrap_or_else(|error| panic!("write neriaka fixture: {error}"));
        std::fs::write(&nektulos, "P 1, 2, 3, 0, 0, 0, 3, To_Neriak\n")
            .unwrap_or_else(|error| panic!("write nektulos fixture: {error}"));
        let graph = EverQuestZoneGraph {
            nodes: vec![
                EverQuestZoneNode {
                    zone_short_name: "neriaka".to_owned(),
                    display_name: Some("Neriak - Foreign Quarter".to_owned()),
                    source_path: neriaka.clone(),
                    len_bytes: 42,
                    last_modified_unix_ms: Some(1_000),
                },
                EverQuestZoneNode {
                    zone_short_name: "nektulos".to_owned(),
                    display_name: Some("Nektulos Forest".to_owned()),
                    source_path: nektulos,
                    len_bytes: 42,
                    last_modified_unix_ms: Some(1_000),
                },
            ],
            landmarks: vec![
                EverQuestZoneLandmark {
                    zone_short_name: "neriaka".to_owned(),
                    label: "Priest_of_Discord".to_owned(),
                    normalized_label: normalize_label("Priest_of_Discord"),
                    location: EverQuestMapCoord {
                        x: 160.0,
                        y: 55.0,
                        z: 31.0,
                    },
                    layer: 3,
                    source_path: neriaka.clone(),
                    source_line_number: 1,
                },
                EverQuestZoneLandmark {
                    zone_short_name: "neriaka".to_owned(),
                    label: "to_Nektulos_Forest".to_owned(),
                    normalized_label: normalize_label("to_Nektulos_Forest"),
                    location: EverQuestMapCoord {
                        x: -155.1781,
                        y: -20.6847,
                        z: 28.6260,
                    },
                    layer: 3,
                    source_path: neriaka.clone(),
                    source_line_number: 2,
                },
            ],
            edges: vec![EverQuestZoneEdge {
                source_zone_short_name: "neriaka".to_owned(),
                target_zone_short_name: Some("nektulos".to_owned()),
                target_display_name: Some("Nektulos Forest".to_owned()),
                target_hint: "Nektulos_Forest".to_owned(),
                normalized_target_hint: normalize_label("Nektulos_Forest"),
                label: "to_Nektulos_Forest".to_owned(),
                location: EverQuestMapCoord {
                    x: -155.1781,
                    y: -20.6847,
                    z: 28.6260,
                },
                confidence: 0.85,
                resolution: EverQuestZoneEdgeResolution::Alias,
                source_path: neriaka,
                source_line_number: 2,
            }],
            unresolved_edge_count: 0,
            skipped_maps: Vec::new(),
        };
        (temp, graph)
    }

    #[test]
    fn source_file_hashes_real_map_file() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("neriaka.txt");
        std::fs::write(
            &path,
            "L 1, 2, 3, 4, 5, 6, 0, 0, 0\nP -155.1781, -20.6847, 28.6260, 0, 0, 0, 3, to_Nektulos_Forest\n",
        )?;
        let map = parse_map_file(&path)?;
        let source = source_file_from_map(&map)?;

        assert_eq!(source.zone_short_name, "neriaka");
        assert_eq!(source.line_count, 2);
        assert_eq!(source.point_count, 1);
        assert_eq!(source.sha256.len(), 64);
        Ok(())
    }
}
