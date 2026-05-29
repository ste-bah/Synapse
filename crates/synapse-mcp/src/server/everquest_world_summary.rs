mod model;
mod validation;

use chrono::{DateTime, Utc};
use rmcp::ErrorData;
use synapse_core::error_codes;
use synapse_everquest::{
    EverQuestMapCoord, EverQuestZoneEdge, EverQuestZoneGraph, EverQuestZoneLandmark,
    build_zone_graph_from_root,
};

use self::{
    model::{
        EverQuestWorldSummaryEvidenceBoundary, EverQuestWorldSummaryExit,
        EverQuestWorldSummaryFocus, EverQuestWorldSummaryHazard, EverQuestWorldSummaryLandmark,
        EverQuestWorldSummaryLevel, EverQuestWorldSummaryLocation, EverQuestWorldSummaryParams,
        EverQuestWorldSummaryRecovery, EverQuestWorldSummaryRedaction,
        EverQuestWorldSummaryResponse, EverQuestWorldSummaryRow, EverQuestWorldSummarySourceRef,
        EverQuestWorldSummaryTransition, EverQuestWorldSummaryZone, MAX_SOURCE_REFS,
        NormalizedSummaryParams, SCHEMA_VERSION, TOOL,
    },
    validation::{decode_json_row, normalize_params, sanitize_summary},
};
use super::{
    Json, Parameters, SynapseService,
    everquest_state::{EverQuestCurrentState, EverQuestStateActionSummary, EverQuestStateSource},
    tool, tool_router,
};
use crate::m1::mcp_error;

const UNKNOWN_PROCESS: &str = "unknown";

#[derive(Clone, Debug)]
struct SummarySourceState {
    source_mode: String,
    state_row_key: String,
    generated_at: Option<DateTime<Utc>>,
    zone_display_name: Option<String>,
    zone_short_name: Option<String>,
    zone_confidence: f32,
    location: Option<EverQuestWorldSummaryLocation>,
    location_confidence: f32,
    level: Option<u32>,
    level_confidence: f32,
    focus: EverQuestWorldSummaryFocus,
    hazards: Vec<EverQuestWorldSummaryHazard>,
    recent_transitions: Vec<EverQuestWorldSummaryTransition>,
    source_refs: Vec<EverQuestWorldSummarySourceRef>,
    redaction_probe_present: bool,
}

struct GraphContext {
    graph: Option<EverQuestZoneGraph>,
    hazard: Option<EverQuestWorldSummaryHazard>,
    source_ref: Option<EverQuestWorldSummarySourceRef>,
}

#[tool_router(router = everquest_world_summary_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Persist one compact EverQuest world-state summary for context injection with map/log/storage provenance and chat redaction"
    )]
    pub async fn everquest_world_summary(
        &self,
        params: Parameters<EverQuestWorldSummaryParams>,
    ) -> Result<Json<EverQuestWorldSummaryResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TOOL,
            "tool.invocation kind=everquest_world_summary"
        );
        let params = normalize_params(params.0)?;
        let source = self.summary_source_state(&params)?;
        let graph = self.summary_graph_context(&params);
        let row = build_summary_row(&params, &source, graph);
        let (summary, stored_value_len_bytes) =
            self.persist_world_summary_json(&row.row_key, &row)?;
        Ok(Json(EverQuestWorldSummaryResponse {
            ok: true,
            row_key: summary.row_key.clone(),
            stored_value_len_bytes,
            summary,
        }))
    }
}

impl SynapseService {
    fn summary_source_state(
        &self,
        params: &NormalizedSummaryParams,
    ) -> Result<SummarySourceState, ErrorData> {
        if let Some(override_state) = &params.state_override {
            let mut source_refs = override_state.source_refs.clone();
            if override_state.redaction_probe_text.is_some() {
                source_refs.push(EverQuestWorldSummarySourceRef {
                    kind: "redaction_probe".to_owned(),
                    row_key: None,
                    path: None,
                    line_number: None,
                    start_offset: None,
                    next_offset: None,
                    summary: Some("[redacted chat summary]".to_owned()),
                });
            }
            source_refs.truncate(MAX_SOURCE_REFS);
            return Ok(SummarySourceState {
                source_mode: "state_override".to_owned(),
                state_row_key: params.state_row_key.clone(),
                generated_at: override_state.generated_at,
                zone_display_name: override_state.zone_display_name.clone(),
                zone_short_name: override_state.zone_short_name.clone(),
                zone_confidence: override_state.confidence,
                location: override_state.location.clone(),
                location_confidence: override_state
                    .location
                    .as_ref()
                    .map_or(0.0, |_| override_state.confidence),
                level: override_state.level,
                level_confidence: override_state
                    .level
                    .map_or(0.0, |_| override_state.confidence),
                focus: EverQuestWorldSummaryFocus {
                    is_everquest_foreground: override_state.everquest_foreground,
                    confidence: override_state.confidence,
                    process_name: if override_state.everquest_foreground {
                        "eqgame.exe".to_owned()
                    } else {
                        UNKNOWN_PROCESS.to_owned()
                    },
                },
                hazards: override_state.hazards.clone(),
                recent_transitions: vec![EverQuestWorldSummaryTransition {
                    transition_kind: "state_override".to_owned(),
                    summary: "synthetic current-state override supplied to world summary"
                        .to_owned(),
                    source_ref: None,
                }],
                source_refs,
                redaction_probe_present: override_state.redaction_probe_text.is_some(),
            });
        }

        let stored = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while reading EverQuest summary source state",
                )
            })?;
            runtime
                .storage_kv_row(params.state_row_key.as_bytes())
                .map_err(|error| mcp_error(error.code(), error.to_string()))?
        };
        let Some(stored) = stored else {
            return Ok(missing_source_state(params));
        };
        let state =
            decode_json_row::<EverQuestCurrentState>(&stored, "EverQuest current-state row")?;
        Ok(source_state_from_current_row(params, &state))
    }

    fn summary_graph_context(&self, params: &NormalizedSummaryParams) -> GraphContext {
        let install_root = if let Some(path) = params.install_root_override.as_deref() {
            std::path::PathBuf::from(path)
        } else {
            match self.resolve_active_everquest_log() {
                Ok(active) => active.install_root,
                Err(error) => {
                    return GraphContext {
                        graph: None,
                        hazard: Some(EverQuestWorldSummaryHazard {
                            code: "map_graph_unavailable".to_owned(),
                            severity: "high".to_owned(),
                            detail: sanitize_summary(&format!(
                                "active EverQuest install/log unavailable: {error}"
                            )),
                        }),
                        source_ref: None,
                    };
                }
            }
        };
        match build_zone_graph_from_root(&install_root) {
            Ok(graph) => GraphContext {
                graph: Some(graph),
                hazard: None,
                source_ref: Some(EverQuestWorldSummarySourceRef {
                    kind: "everquest_map_root".to_owned(),
                    row_key: None,
                    path: Some(install_root.display().to_string()),
                    line_number: None,
                    start_offset: None,
                    next_offset: None,
                    summary: Some(
                        "local EverQuest maps directory used for compact summary".to_owned(),
                    ),
                }),
            },
            Err(error) => GraphContext {
                graph: None,
                hazard: Some(EverQuestWorldSummaryHazard {
                    code: "map_graph_unavailable".to_owned(),
                    severity: "high".to_owned(),
                    detail: sanitize_summary(&format!("build EverQuest map graph: {error}")),
                }),
                source_ref: Some(EverQuestWorldSummarySourceRef {
                    kind: "everquest_map_root".to_owned(),
                    row_key: None,
                    path: Some(install_root.display().to_string()),
                    line_number: None,
                    start_offset: None,
                    next_offset: None,
                    summary: Some(
                        "map graph build failed; summary persisted blockers only".to_owned(),
                    ),
                }),
            },
        }
    }

    fn persist_world_summary_json(
        &self,
        key: &str,
        row: &EverQuestWorldSummaryRow,
    ) -> Result<(EverQuestWorldSummaryRow, u64), ErrorData> {
        let encoded = serde_json::to_vec(row).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode EverQuest world-summary row: {error}"),
            )
        })?;
        let stored = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while writing EverQuest world-summary row",
                )
            })?;
            runtime
                .storage_put_kv_rows(vec![(key.as_bytes().to_vec(), encoded)])
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_WRITE_FAILED,
                        format!("write EverQuest world-summary row: {error}"),
                    )
                })?;
            runtime
                .storage_kv_row(key.as_bytes())
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        format!("read EverQuest world-summary row after write: {error}"),
                    )
                })?
                .ok_or_else(|| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        format!("EverQuest world-summary row missing after write: {key}"),
                    )
                })?
        };
        let readback =
            decode_json_row::<EverQuestWorldSummaryRow>(&stored, "EverQuest world-summary row")?;
        Ok((readback, len_to_u64(stored.len())))
    }
}

fn build_summary_row(
    params: &NormalizedSummaryParams,
    source: &SummarySourceState,
    graph_context: GraphContext,
) -> EverQuestWorldSummaryRow {
    let mut source_refs = source.source_refs.clone();
    source_refs.extend(params.source_refs.clone());
    if let Some(ref source_ref) = graph_context.source_ref {
        source_refs.push(source_ref.clone());
    }
    source_refs.truncate(MAX_SOURCE_REFS);

    let mut hazards = source.hazards.clone();
    if let Some(hazard) = graph_context.hazard {
        hazards.push(hazard);
    }

    let mut active_blockers =
        active_blockers(source, graph_context.graph.as_ref(), &hazards, params);
    let nearest_exits = graph_context.graph.as_ref().map_or_else(Vec::new, |graph| {
        nearest_exits_for_source(graph, source, params.max_exits)
    });
    let nearest_landmarks = graph_context.graph.as_ref().map_or_else(Vec::new, |graph| {
        nearest_landmarks_for_source(graph, source, params.max_landmarks)
    });
    if source.redaction_probe_present {
        hazards.push(EverQuestWorldSummaryHazard {
            code: "redaction_probe_present".to_owned(),
            severity: "info".to_owned(),
            detail: "raw chat-like probe text was observed and redacted before persistence"
                .to_owned(),
        });
    }
    hazards.truncate(params.max_hazards);
    active_blockers.sort();
    active_blockers.dedup();

    let recent_transitions =
        limited_transitions(&source.recent_transitions, params.max_transitions);
    let safe_next_probes = safe_next_probes(&active_blockers, &nearest_exits);
    let compact_status = if active_blockers.is_empty() {
        "ready"
    } else {
        "blocked"
    };

    EverQuestWorldSummaryRow {
        schema_version: SCHEMA_VERSION,
        row_kind: "everquest_world_summary".to_owned(),
        profile_id: params.profile_id.clone(),
        summary_id: params.summary_id.clone(),
        row_key: params.row_key.clone(),
        generated_at: Utc::now(),
        source_state_row_key: source.state_row_key.clone(),
        source_mode: source.source_mode.clone(),
        compact_status: compact_status.to_owned(),
        zone: EverQuestWorldSummaryZone {
            display_name: source.zone_display_name.clone(),
            short_name: source.zone_short_name.clone(),
            confidence: source.zone_confidence,
        },
        location: source.location.clone(),
        level_progress: EverQuestWorldSummaryLevel {
            level: source.level,
            xp_percent: None,
            confidence: source.level_confidence,
        },
        focus: source.focus.clone(),
        nearest_exits,
        nearest_landmarks,
        recent_transitions,
        safe_next_probes,
        hazards,
        active_blockers,
        source_refs,
        compaction_recovery: EverQuestWorldSummaryRecovery {
            latest_summary_row_key: params.row_key.clone(),
            durable_skill_memory_issue: "https://github.com/ChrisRoyse/Synapse/issues/501"
                .to_owned(),
            full_tool_fsv_matrix_issue: "https://github.com/ChrisRoyse/Synapse/issues/500"
                .to_owned(),
            world_model_context_issue: "https://github.com/ChrisRoyse/Synapse/issues/505"
                .to_owned(),
        },
        redaction: EverQuestWorldSummaryRedaction {
            compact_redacted: true,
            raw_chat_body_persisted: false,
            raw_target_names_persisted: false,
            source_summaries_redacted: true,
            redaction_probe_present: source.redaction_probe_present,
        },
        evidence_boundary: EverQuestWorldSummaryEvidenceBoundary {
            reads_physical_state: source.source_mode == "current_state_row"
                || graph_context.source_ref.is_some(),
            writes_summary_row_only: true,
            executes_input: false,
            manual_fsv_required_for_runtime: true,
            is_fsv_script: false,
        },
    }
}

fn missing_source_state(params: &NormalizedSummaryParams) -> SummarySourceState {
    SummarySourceState {
        source_mode: "current_state_row_missing".to_owned(),
        state_row_key: params.state_row_key.clone(),
        generated_at: None,
        zone_display_name: None,
        zone_short_name: None,
        zone_confidence: 0.0,
        location: None,
        location_confidence: 0.0,
        level: None,
        level_confidence: 0.0,
        focus: EverQuestWorldSummaryFocus {
            is_everquest_foreground: false,
            confidence: 0.0,
            process_name: UNKNOWN_PROCESS.to_owned(),
        },
        hazards: vec![EverQuestWorldSummaryHazard {
            code: "current_state_row_missing".to_owned(),
            severity: "high".to_owned(),
            detail: "persisted EverQuest current-state row was absent".to_owned(),
        }],
        recent_transitions: Vec::new(),
        source_refs: vec![EverQuestWorldSummarySourceRef {
            kind: "synapse_storage_missing".to_owned(),
            row_key: Some(params.state_row_key.clone()),
            path: None,
            line_number: None,
            start_offset: None,
            next_offset: None,
            summary: Some("current-state row absent before world-summary build".to_owned()),
        }],
        redaction_probe_present: false,
    }
}

fn source_state_from_current_row(
    params: &NormalizedSummaryParams,
    state: &EverQuestCurrentState,
) -> SummarySourceState {
    let mut source_refs = vec![EverQuestWorldSummarySourceRef {
        kind: "synapse_storage".to_owned(),
        row_key: Some(params.state_row_key.clone()),
        path: None,
        line_number: None,
        start_offset: None,
        next_offset: None,
        summary: Some("persisted EverQuest current-state row".to_owned()),
    }];
    source_refs.push(EverQuestWorldSummarySourceRef {
        kind: "everquest_log_cursor".to_owned(),
        row_key: None,
        path: Some(state.log_cursor.path.clone()),
        line_number: None,
        start_offset: Some(state.log_cursor.start_offset),
        next_offset: Some(state.log_cursor.next_offset),
        summary: Some(format!(
            "log cursor bytes {}..{} with {} compact events",
            state.log_cursor.start_offset,
            state.log_cursor.next_offset,
            state.log_cursor.event_count
        )),
    });
    source_refs.extend(state_source_refs(&state.zone.sources));
    source_refs.extend(state_source_refs(&state.zone_short_name.sources));
    source_refs.extend(state_source_refs(&state.location.sources));
    source_refs.extend(state_source_refs(&state.level.sources));
    source_refs.truncate(MAX_SOURCE_REFS);

    SummarySourceState {
        source_mode: "current_state_row".to_owned(),
        state_row_key: params.state_row_key.clone(),
        generated_at: Some(state.generated_at),
        zone_display_name: state.zone.value.clone(),
        zone_short_name: state.zone_short_name.value.clone(),
        zone_confidence: state.zone_short_name.confidence.max(state.zone.confidence),
        location: state
            .location
            .value
            .as_ref()
            .map(|location| EverQuestWorldSummaryLocation {
                map_x: location.map_x,
                map_y: location.map_y,
                map_z: location.map_z,
                confidence: state.location.confidence,
            }),
        location_confidence: state.location.confidence,
        level: state.level.value,
        level_confidence: state.level.confidence,
        focus: EverQuestWorldSummaryFocus {
            is_everquest_foreground: state.focus.is_everquest_foreground,
            confidence: state.focus.confidence,
            process_name: sanitize_summary(&state.focus.process_name),
        },
        hazards: state
            .hazards
            .iter()
            .map(|hazard| EverQuestWorldSummaryHazard {
                code: hazard.code.clone(),
                severity: hazard.severity.clone(),
                detail: sanitize_summary(&hazard.detail),
            })
            .collect(),
        recent_transitions: transitions_from_current_state(state),
        source_refs,
        redaction_probe_present: false,
    }
}

fn active_blockers(
    source: &SummarySourceState,
    graph: Option<&EverQuestZoneGraph>,
    hazards: &[EverQuestWorldSummaryHazard],
    params: &NormalizedSummaryParams,
) -> Vec<String> {
    let mut blockers = Vec::new();
    if source.zone_short_name.is_none() {
        blockers.push("unknown_zone".to_owned());
    }
    if source.location.is_none() {
        blockers.push("unknown_location".to_owned());
    }
    if source.zone_short_name.is_some() && source.zone_confidence < 0.50 {
        blockers.push("low_confidence_zone".to_owned());
    }
    if source.location.is_some() && source.location_confidence < 0.50 {
        blockers.push("low_confidence_location".to_owned());
    }
    if !source.focus.is_everquest_foreground {
        blockers.push("non_everquest_foreground".to_owned());
    }
    if is_stale_source(source.generated_at, params.stale_after_seconds) {
        blockers.push("stale_state".to_owned());
    }
    if hazards
        .iter()
        .any(|hazard| hazard.code == "map_graph_unavailable")
    {
        blockers.push("map_graph_unavailable".to_owned());
    }
    if let (Some(graph), Some(zone)) = (graph, source.zone_short_name.as_deref())
        && graph.node(zone).is_none()
    {
        blockers.push("unknown_zone".to_owned());
    }
    blockers
}

fn nearest_exits_for_source(
    graph: &EverQuestZoneGraph,
    source: &SummarySourceState,
    limit: usize,
) -> Vec<EverQuestWorldSummaryExit> {
    let Some(zone) = source.zone_short_name.as_deref() else {
        return Vec::new();
    };
    let location = source.location.as_ref().map(coord_from_location);
    let mut exits = graph
        .exits_for_zone(zone)
        .into_iter()
        .map(|edge| exit_from_edge(edge, location.as_ref()))
        .collect::<Vec<_>>();
    exits.sort_by(|left, right| {
        left.distance_from_current
            .unwrap_or(f64::MAX)
            .total_cmp(&right.distance_from_current.unwrap_or(f64::MAX))
            .then(left.label.cmp(&right.label))
    });
    exits.truncate(limit);
    exits
}

fn nearest_landmarks_for_source(
    graph: &EverQuestZoneGraph,
    source: &SummarySourceState,
    limit: usize,
) -> Vec<EverQuestWorldSummaryLandmark> {
    let Some(zone) = source.zone_short_name.as_deref() else {
        return Vec::new();
    };
    let Some(location) = source.location.as_ref().map(coord_from_location) else {
        return graph
            .landmarks_for_zone(zone)
            .into_iter()
            .take(limit)
            .map(|landmark| landmark_from_zone_landmark(landmark, None))
            .collect();
    };
    graph
        .nearest_landmarks(zone, &location, limit)
        .into_iter()
        .map(|nearest| landmark_from_zone_landmark(nearest.landmark, Some(nearest.distance)))
        .collect()
}

fn exit_from_edge(
    edge: EverQuestZoneEdge,
    location: Option<&EverQuestMapCoord>,
) -> EverQuestWorldSummaryExit {
    EverQuestWorldSummaryExit {
        label: sanitize_summary(&edge.label),
        zone_short_name: edge.source_zone_short_name,
        target_zone_short_name: edge.target_zone_short_name,
        target_display_name: edge
            .target_display_name
            .map(|value| sanitize_summary(&value)),
        distance_from_current: location.map(|location| distance(location, &edge.location)),
        confidence: edge.confidence,
        source_path: edge.source_path.display().to_string(),
        source_line_number: edge.source_line_number,
    }
}

fn landmark_from_zone_landmark(
    landmark: EverQuestZoneLandmark,
    distance_from_current: Option<f64>,
) -> EverQuestWorldSummaryLandmark {
    EverQuestWorldSummaryLandmark {
        label: sanitize_summary(&landmark.label),
        zone_short_name: landmark.zone_short_name,
        distance_from_current,
        confidence: 0.85,
        source_path: landmark.source_path.display().to_string(),
        source_line_number: landmark.source_line_number,
    }
}

fn transitions_from_current_state(
    state: &EverQuestCurrentState,
) -> Vec<EverQuestWorldSummaryTransition> {
    let mut transitions = Vec::new();
    if let Some(zone) = &state.zone_short_name.value {
        transitions.push(EverQuestWorldSummaryTransition {
            transition_kind: "zone".to_owned(),
            summary: sanitize_summary(&format!("current zone short-name {zone}")),
            source_ref: state
                .zone_short_name
                .sources
                .first()
                .map(source_ref_from_state_source),
        });
    }
    if let Some(location) = &state.location.value {
        transitions.push(EverQuestWorldSummaryTransition {
            transition_kind: "location".to_owned(),
            summary: format!(
                "current map location x={:.2} y={:.2} z={:.2}",
                location.map_x, location.map_y, location.map_z
            ),
            source_ref: state
                .location
                .sources
                .first()
                .map(source_ref_from_state_source),
        });
    }
    transitions.extend(
        state
            .latest_actions
            .iter()
            .rev()
            .map(transition_from_action_summary),
    );
    transitions
}

fn transition_from_action_summary(
    action: &EverQuestStateActionSummary,
) -> EverQuestWorldSummaryTransition {
    let tool = action.tool.as_deref().unwrap_or("unknown_tool");
    let status = action.status.as_deref().unwrap_or("unknown_status");
    let error = action.error_code.as_deref().unwrap_or("none");
    EverQuestWorldSummaryTransition {
        transition_kind: "action_audit".to_owned(),
        summary: sanitize_summary(&format!("tool={tool} status={status} error={error}")),
        source_ref: None,
    }
}

fn limited_transitions(
    transitions: &[EverQuestWorldSummaryTransition],
    limit: usize,
) -> Vec<EverQuestWorldSummaryTransition> {
    transitions
        .iter()
        .take(limit)
        .map(|transition| EverQuestWorldSummaryTransition {
            transition_kind: sanitize_summary(&transition.transition_kind),
            summary: sanitize_summary(&transition.summary),
            source_ref: transition.source_ref.clone(),
        })
        .collect()
}

fn safe_next_probes(
    active_blockers: &[String],
    nearest_exits: &[EverQuestWorldSummaryExit],
) -> Vec<String> {
    if active_blockers
        .iter()
        .any(|blocker| blocker == "stale_state")
    {
        return vec!["refresh_everquest_current_state".to_owned()];
    }
    if active_blockers
        .iter()
        .any(|blocker| blocker == "map_graph_unavailable")
    {
        return vec!["verify_local_maps_and_run_map_sensor".to_owned()];
    }
    if active_blockers
        .iter()
        .any(|blocker| blocker == "non_everquest_foreground")
    {
        return vec!["refocus_everquest_foreground".to_owned()];
    }
    if active_blockers
        .iter()
        .any(|blocker| blocker == "unknown_zone" || blocker == "unknown_location")
    {
        return vec!["run_everquest_loc_probe_then_current_state".to_owned()];
    }
    let mut probes = nearest_exits
        .iter()
        .take(2)
        .map(|exit| format!("route_probe:{}", exit.label))
        .collect::<Vec<_>>();
    probes.push("planner_guard_before_any_movement".to_owned());
    probes
}

fn state_source_refs(sources: &[EverQuestStateSource]) -> Vec<EverQuestWorldSummarySourceRef> {
    sources
        .iter()
        .map(source_ref_from_state_source)
        .collect::<Vec<_>>()
}

fn source_ref_from_state_source(source: &EverQuestStateSource) -> EverQuestWorldSummarySourceRef {
    EverQuestWorldSummarySourceRef {
        kind: sanitize_summary(&source.kind),
        row_key: None,
        path: source.path.clone(),
        line_number: None,
        start_offset: source.start_offset,
        next_offset: source.next_offset,
        summary: source
            .summary
            .as_ref()
            .map(|summary| sanitize_summary(summary)),
    }
}

const fn coord_from_location(location: &EverQuestWorldSummaryLocation) -> EverQuestMapCoord {
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

fn len_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use synapse_everquest::{
        EverQuestZoneEdgeResolution, EverQuestZoneNode, EverQuestZoneSkippedMap,
    };

    use super::*;

    #[test]
    fn ready_summary_keeps_nearest_exit_and_redacts_probe() {
        let params = params("happy");
        let source = source_state(Some("neriaka"), true);
        let row = build_summary_row(
            &params,
            &source,
            GraphContext {
                graph: Some(graph()),
                hazard: None,
                source_ref: None,
            },
        );

        assert_eq!(row.compact_status, "ready");
        assert_eq!(row.zone.short_name.as_deref(), Some("neriaka"));
        assert_eq!(row.level_progress.level, Some(1));
        assert_eq!(row.nearest_exits[0].label, "to_Nektulos_Forest");
        assert!(!serde_json::to_string(&row).unwrap().contains("you say"));
        assert!(row.redaction.redaction_probe_present);
    }

    #[test]
    fn unknown_zone_is_blocked_but_persistable() {
        let params = params("unknown");
        let source = source_state(Some("unknown_zone"), false);
        let row = build_summary_row(
            &params,
            &source,
            GraphContext {
                graph: Some(graph()),
                hazard: None,
                source_ref: None,
            },
        );

        assert_eq!(row.compact_status, "blocked");
        assert!(row.active_blockers.contains(&"unknown_zone".to_owned()));
        assert!(row.nearest_exits.is_empty());
    }

    #[test]
    fn missing_map_context_records_blocker() {
        let params = params("missing-map");
        let source = source_state(Some("neriaka"), false);
        let row = build_summary_row(
            &params,
            &source,
            GraphContext {
                graph: None,
                hazard: Some(EverQuestWorldSummaryHazard {
                    code: "map_graph_unavailable".to_owned(),
                    severity: "high".to_owned(),
                    detail: "maps directory absent".to_owned(),
                }),
                source_ref: None,
            },
        );

        assert_eq!(row.compact_status, "blocked");
        assert!(
            row.active_blockers
                .contains(&"map_graph_unavailable".to_owned())
        );
        assert!(row.nearest_landmarks.is_empty());
    }

    fn params(summary_id: &str) -> NormalizedSummaryParams {
        NormalizedSummaryParams {
            summary_id: summary_id.to_owned(),
            profile_id: "everquest.live".to_owned(),
            state_row_key: "everquest/current_state/v1/everquest.live".to_owned(),
            state_override: None,
            install_root_override: None,
            max_exits: 5,
            max_landmarks: 5,
            max_transitions: 5,
            max_hazards: 5,
            stale_after_seconds: 300,
            source_refs: Vec::new(),
            row_key: format!("everquest/world_summary/v1/everquest.live/{summary_id}"),
        }
    }

    fn source_state(zone_short_name: Option<&str>, redaction_probe: bool) -> SummarySourceState {
        SummarySourceState {
            source_mode: "state_override".to_owned(),
            state_row_key: "everquest/current_state/v1/everquest.live".to_owned(),
            generated_at: Some(Utc::now()),
            zone_display_name: Some("Neriak - Foreign Quarter".to_owned()),
            zone_short_name: zone_short_name.map(str::to_owned),
            zone_confidence: 0.95,
            location: Some(EverQuestWorldSummaryLocation {
                map_x: 62.1,
                map_y: 23.21,
                map_z: 3.19,
                confidence: 0.95,
            }),
            location_confidence: 0.95,
            level: Some(1),
            level_confidence: 0.95,
            focus: EverQuestWorldSummaryFocus {
                is_everquest_foreground: true,
                confidence: 1.0,
                process_name: "eqgame.exe".to_owned(),
            },
            hazards: Vec::new(),
            recent_transitions: vec![EverQuestWorldSummaryTransition {
                transition_kind: "redaction_probe".to_owned(),
                summary: sanitize_summary("you say, synthetic secret"),
                source_ref: None,
            }],
            source_refs: Vec::new(),
            redaction_probe_present: redaction_probe,
        }
    }

    fn graph() -> EverQuestZoneGraph {
        EverQuestZoneGraph {
            nodes: vec![
                EverQuestZoneNode {
                    zone_short_name: "neriaka".to_owned(),
                    display_name: Some("Neriak - Foreign Quarter".to_owned()),
                    source_path: PathBuf::from("neriaka.txt"),
                    len_bytes: 1,
                    last_modified_unix_ms: None,
                },
                EverQuestZoneNode {
                    zone_short_name: "nektulos".to_owned(),
                    display_name: Some("Nektulos Forest".to_owned()),
                    source_path: PathBuf::from("nektulos.txt"),
                    len_bytes: 1,
                    last_modified_unix_ms: None,
                },
            ],
            landmarks: vec![EverQuestZoneLandmark {
                zone_short_name: "neriaka".to_owned(),
                label: "to_Nektulos_Forest".to_owned(),
                normalized_label: "tonektulosforest".to_owned(),
                location: EverQuestMapCoord {
                    x: -155.1781,
                    y: -20.6847,
                    z: 28.6260,
                },
                layer: 3,
                source_path: PathBuf::from("neriaka.txt"),
                source_line_number: 2983,
            }],
            edges: vec![EverQuestZoneEdge {
                source_zone_short_name: "neriaka".to_owned(),
                target_zone_short_name: Some("nektulos".to_owned()),
                target_display_name: Some("Nektulos Forest".to_owned()),
                target_hint: "Nektulos_Forest".to_owned(),
                normalized_target_hint: "nektulosforest".to_owned(),
                label: "to_Nektulos_Forest".to_owned(),
                location: EverQuestMapCoord {
                    x: -155.1781,
                    y: -20.6847,
                    z: 28.6260,
                },
                confidence: 0.85,
                resolution: EverQuestZoneEdgeResolution::Alias,
                source_path: PathBuf::from("neriaka.txt"),
                source_line_number: 2983,
            }],
            segments: Vec::new(),
            unresolved_edge_count: 0,
            skipped_maps: Vec::<EverQuestZoneSkippedMap>::new(),
        }
    }
}
