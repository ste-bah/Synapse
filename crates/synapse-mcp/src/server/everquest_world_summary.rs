mod model;
mod validation;

use chrono::{DateTime, Utc};
use rmcp::ErrorData;
use synapse_core::{RealityAudit, RealityDelta, RealityDriftStatus, error_codes};
use synapse_everquest::{
    EverQuestMapCoord, EverQuestZoneEdge, EverQuestZoneGraph, EverQuestZoneLandmark,
    build_zone_graph_from_root,
};
use synapse_storage::cf;

use self::{
    model::{
        EverQuestWorldSummaryEvidenceBoundary, EverQuestWorldSummaryExit,
        EverQuestWorldSummaryFocus, EverQuestWorldSummaryHazard, EverQuestWorldSummaryLandmark,
        EverQuestWorldSummaryLevel, EverQuestWorldSummaryLocation, EverQuestWorldSummaryParams,
        EverQuestWorldSummaryRealityContext, EverQuestWorldSummaryRecovery,
        EverQuestWorldSummaryRedaction, EverQuestWorldSummaryResponse, EverQuestWorldSummaryRow,
        EverQuestWorldSummarySourceRef, EverQuestWorldSummaryTransition, EverQuestWorldSummaryZone,
        MAX_SOURCE_REFS, NormalizedSummaryParams, SCHEMA_VERSION, TOOL,
    },
    validation::{decode_json_row, normalize_params, sanitize_summary},
};
use super::{
    Json, Parameters, SynapseService,
    everquest_state::{EverQuestCurrentState, EverQuestStateActionSummary, EverQuestStateSource},
    reality::RealityHeadRow,
    tool, tool_router,
};
use crate::m1::mcp_error;

const UNKNOWN_PROCESS: &str = "unknown";
const REALITY_STATUS_READY: &str = "ready";

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
    xp_percent: Option<f32>,
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
        let reality_context = self.summary_reality_context(&params)?;
        let row = build_summary_row(&params, &source, graph, &reality_context);
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
                xp_percent: override_state.xp_percent,
                level_confidence: if override_state.level.is_some()
                    || override_state.xp_percent.is_some()
                {
                    override_state.confidence
                } else {
                    0.0
                },
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

    #[allow(clippy::too_many_lines)]
    fn summary_reality_context(
        &self,
        params: &NormalizedSummaryParams,
    ) -> Result<EverQuestWorldSummaryRealityContext, ErrorData> {
        let profile_key = params.profile_id.clone();
        let head_key = reality_head_key(&profile_key);
        let head_bytes = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while reading EverQuest reality head",
                )
            })?;
            runtime
                .storage_kv_row(head_key.as_bytes())
                .map_err(|error| mcp_error(error.code(), error.to_string()))?
        };
        let Some(head_bytes) = head_bytes else {
            return Ok(missing_reality_context(profile_key, head_key));
        };

        let head = decode_json_row::<RealityHeadRow>(
            &head_bytes,
            "EverQuest reality head row for world summary",
        )?;
        let mut source_refs = vec![EverQuestWorldSummarySourceRef {
            kind: "reality_head".to_owned(),
            row_key: Some(head_key.clone()),
            path: None,
            line_number: None,
            start_offset: None,
            next_offset: None,
            summary: Some(sanitize_summary(&format!(
                "baseline epoch {} head seq {}",
                head.epoch_id, head.head_seq
            ))),
        }];

        let mut newest_delta_seq = None;
        let mut newest_delta_key = None;
        let mut newest_delta_kind = None;
        let mut newest_delta_path = None;
        let mut newest_delta_at = None;
        let mut delta_missing = false;
        if head.head_seq > head.baseline_seq {
            let delta_key = reality_delta_key(&head.profile_key, &head.epoch_id, head.head_seq);
            let delta_bytes = {
                let runtime = self.reflex_runtime()?;
                let runtime = runtime.lock().map_err(|_| {
                    mcp_error(
                        error_codes::TOOL_INTERNAL_ERROR,
                        "reflex runtime lock poisoned while reading newest EverQuest reality delta",
                    )
                })?;
                runtime
                    .storage_kv_row(delta_key.as_bytes())
                    .map_err(|error| mcp_error(error.code(), error.to_string()))?
            };
            if let Some(delta_bytes) = delta_bytes {
                let delta = decode_json_row::<RealityDelta>(
                    &delta_bytes,
                    "newest EverQuest reality delta row",
                )?;
                newest_delta_seq = Some(delta.seq);
                newest_delta_kind = Some(sanitize_summary(&delta.kind));
                newest_delta_path = Some(sanitize_summary(&delta.path));
                newest_delta_at = Some(delta.at);
                newest_delta_key = Some(delta_key.clone());
                source_refs.push(EverQuestWorldSummarySourceRef {
                    kind: "reality_delta".to_owned(),
                    row_key: Some(delta_key),
                    path: None,
                    line_number: None,
                    start_offset: None,
                    next_offset: None,
                    summary: Some(sanitize_summary(&format!(
                        "newest delta seq {} kind {} path {}",
                        delta.seq, delta.kind, delta.path
                    ))),
                });
            } else {
                delta_missing = true;
                newest_delta_seq = Some(head.head_seq);
                newest_delta_key = Some(delta_key.clone());
                source_refs.push(EverQuestWorldSummarySourceRef {
                    kind: "reality_delta_missing".to_owned(),
                    row_key: Some(delta_key),
                    path: None,
                    line_number: None,
                    start_offset: None,
                    next_offset: None,
                    summary: Some(
                        "head seq advanced but newest delta row was absent in storage".to_owned(),
                    ),
                });
            }
        }

        let latest_audit = self.read_latest_reality_audit(&head.profile_key)?;
        let (
            audit_status,
            latest_audit_row_key,
            latest_audit_id,
            latest_audit_ran_at,
            drift_severity,
            drift_item_count,
            audit_rebase_required,
        ) = if let Some((row_key, audit)) = latest_audit {
            let audit_epoch_stale = audit.epoch_id != head.epoch_id;
            let audit_seq_stale = audit.compared_seq_end < head.head_seq;
            let audit_stale = audit_epoch_stale || audit_seq_stale;
            let audit_status = if audit_stale {
                "stale".to_owned()
            } else {
                drift_status_string(audit.drift_status).to_owned()
            };
            let drift_severity = if audit_stale {
                "stale".to_owned()
            } else {
                drift_status_string(audit.drift_status).to_owned()
            };
            let audit_rebase_required = !audit_stale && audit.rebase_required;
            source_refs.push(EverQuestWorldSummarySourceRef {
                kind: "reality_audit".to_owned(),
                row_key: Some(row_key.clone()),
                path: None,
                line_number: None,
                start_offset: None,
                next_offset: None,
                summary: Some(sanitize_summary(&format!(
                    "audit {} epoch {} drift {} rebase_required={} stale={audit_stale}",
                    audit.audit_id,
                    audit.epoch_id,
                    drift_status_string(audit.drift_status),
                    audit.rebase_required
                ))),
            });
            (
                audit_status,
                Some(row_key),
                Some(audit.audit_id),
                Some(audit.ran_at),
                drift_severity,
                len_to_u32(audit.drift_items.len()),
                audit_rebase_required,
            )
        } else {
            (
                "not_run".to_owned(),
                None,
                None,
                None,
                "unknown".to_owned(),
                0,
                false,
            )
        };

        source_refs.truncate(MAX_SOURCE_REFS);
        let status =
            reality_context_status(delta_missing, audit_status.as_str(), audit_rebase_required);
        let safe_next_probe = reality_safe_next_probe(&status).to_owned();
        Ok(EverQuestWorldSummaryRealityContext {
            profile_key: head.profile_key,
            status,
            head_key,
            head_present: true,
            last_baseline_epoch_id: Some(head.epoch_id),
            last_baseline_seq: Some(head.baseline_seq),
            last_head_seq: Some(head.head_seq),
            newest_delta_seq,
            newest_delta_key,
            newest_delta_kind,
            newest_delta_path,
            newest_delta_at,
            audit_status,
            latest_audit_row_key,
            latest_audit_id,
            latest_audit_ran_at,
            drift_severity,
            drift_item_count,
            rebase_required: delta_missing || audit_rebase_required,
            safe_next_probe,
            source_refs,
        })
    }

    fn read_latest_reality_audit(
        &self,
        profile_key: &str,
    ) -> Result<Option<(String, RealityAudit)>, ErrorData> {
        let prefix = reality_audit_prefix(profile_key);
        let rows = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while reading EverQuest reality audits",
                )
            })?;
            runtime
                .storage_cf_prefix_rows(cf::CF_KV, prefix.as_bytes(), usize::MAX)
                .map_err(|error| mcp_error(error.code(), error.to_string()))?
        };
        let Some((key, value)) = rows.into_iter().last() else {
            return Ok(None);
        };
        let row_key = String::from_utf8_lossy(&key).into_owned();
        let audit = decode_json_row::<RealityAudit>(&value, "latest EverQuest reality audit row")?;
        Ok(Some((row_key, audit)))
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

#[allow(clippy::too_many_lines)]
fn build_summary_row(
    params: &NormalizedSummaryParams,
    source: &SummarySourceState,
    graph_context: GraphContext,
    reality_context: &EverQuestWorldSummaryRealityContext,
) -> EverQuestWorldSummaryRow {
    let mut source_refs = source.source_refs.clone();
    source_refs.extend(params.source_refs.clone());
    source_refs.extend(reality_context.source_refs.clone());
    if let Some(ref source_ref) = graph_context.source_ref {
        source_refs.push(source_ref.clone());
    }
    source_refs.truncate(MAX_SOURCE_REFS);

    let mut hazards = source.hazards.clone();
    hazards.extend(reality_hazards(reality_context));
    if let Some(hazard) = graph_context.hazard {
        hazards.push(hazard);
    }

    let mut active_blockers =
        active_blockers(source, graph_context.graph.as_ref(), &hazards, params);
    active_blockers.extend(reality_blockers(reality_context));
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

    let mut recent_transitions =
        limited_transitions(&source.recent_transitions, params.max_transitions);
    if let Some(transition) = reality_transition(reality_context) {
        recent_transitions.insert(0, transition);
        recent_transitions.truncate(params.max_transitions);
    }
    let mut safe_next_probes = Vec::new();
    if reality_context.status != REALITY_STATUS_READY {
        safe_next_probes.push(reality_context.safe_next_probe.clone());
    }
    safe_next_probes.extend(base_safe_next_probes(&active_blockers, &nearest_exits));
    safe_next_probes.sort();
    safe_next_probes.dedup();
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
            xp_percent: source.xp_percent,
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
        reality_context: reality_context.clone(),
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
        xp_percent: None,
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
    source_refs.extend(state_source_refs(&state.xp_percent.sources));
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
        xp_percent: state.xp_percent.value,
        level_confidence: state.level.confidence.max(state.xp_percent.confidence),
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

fn base_safe_next_probes(
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

fn missing_reality_context(
    profile_key: String,
    head_key: String,
) -> EverQuestWorldSummaryRealityContext {
    EverQuestWorldSummaryRealityContext {
        profile_key,
        status: "baseline_missing".to_owned(),
        head_key: head_key.clone(),
        head_present: false,
        last_baseline_epoch_id: None,
        last_baseline_seq: None,
        last_head_seq: None,
        newest_delta_seq: None,
        newest_delta_key: None,
        newest_delta_kind: None,
        newest_delta_path: None,
        newest_delta_at: None,
        audit_status: "not_run".to_owned(),
        latest_audit_row_key: None,
        latest_audit_id: None,
        latest_audit_ran_at: None,
        drift_severity: "unknown".to_owned(),
        drift_item_count: 0,
        rebase_required: true,
        safe_next_probe: "capture_reality_baseline".to_owned(),
        source_refs: vec![EverQuestWorldSummarySourceRef {
            kind: "reality_head_missing".to_owned(),
            row_key: Some(head_key),
            path: None,
            line_number: None,
            start_offset: None,
            next_offset: None,
            summary: Some("reality head row absent before world-summary build".to_owned()),
        }],
    }
}

fn reality_context_status(
    delta_missing: bool,
    audit_status: &str,
    audit_rebase_required: bool,
) -> String {
    if delta_missing {
        "delta_row_missing".to_owned()
    } else if audit_status == "not_run" {
        "audit_missing".to_owned()
    } else if audit_status == "stale" {
        "audit_stale".to_owned()
    } else if audit_rebase_required {
        "rebase_required".to_owned()
    } else if audit_status == "in_sync" {
        REALITY_STATUS_READY.to_owned()
    } else {
        "drift_detected".to_owned()
    }
}

fn reality_safe_next_probe(status: &str) -> &'static str {
    match status {
        REALITY_STATUS_READY => "continue_with_delta_context",
        "audit_missing" | "audit_stale" => "run_reality_audit_before_movement",
        "delta_row_missing" => "run_reality_audit_then_capture_reality_baseline",
        "rebase_required" | "drift_detected" => "stop_repair_then_capture_reality_baseline",
        "baseline_missing" => "capture_reality_baseline",
        _ => "refresh_reality_baseline_and_audit",
    }
}

fn reality_hazards(
    context: &EverQuestWorldSummaryRealityContext,
) -> Vec<EverQuestWorldSummaryHazard> {
    match context.status.as_str() {
        REALITY_STATUS_READY => Vec::new(),
        "audit_missing" => vec![EverQuestWorldSummaryHazard {
            code: "reality_audit_missing".to_owned(),
            severity: "warning".to_owned(),
            detail: "delta-guided EverQuest state has no persisted reality audit yet".to_owned(),
        }],
        "audit_stale" => vec![EverQuestWorldSummaryHazard {
            code: "reality_audit_stale".to_owned(),
            severity: "warning".to_owned(),
            detail: "latest reality audit does not cover the current baseline/head seq".to_owned(),
        }],
        "baseline_missing" => vec![EverQuestWorldSummaryHazard {
            code: "reality_baseline_missing".to_owned(),
            severity: "high".to_owned(),
            detail: "reality head row is absent; capture baseline before movement or combat"
                .to_owned(),
        }],
        "delta_row_missing" => vec![EverQuestWorldSummaryHazard {
            code: "reality_delta_missing".to_owned(),
            severity: "high".to_owned(),
            detail: "reality head seq points at a delta row that is absent from storage".to_owned(),
        }],
        "rebase_required" | "drift_detected" => vec![EverQuestWorldSummaryHazard {
            code: "reality_drift_rebase_required".to_owned(),
            severity: "high".to_owned(),
            detail: format!(
                "latest reality audit status {} with {} drift item(s)",
                context.audit_status, context.drift_item_count
            ),
        }],
        _ => vec![EverQuestWorldSummaryHazard {
            code: "reality_context_unready".to_owned(),
            severity: "warning".to_owned(),
            detail: format!("reality context status {}", context.status),
        }],
    }
}

fn reality_blockers(context: &EverQuestWorldSummaryRealityContext) -> Vec<String> {
    match context.status.as_str() {
        REALITY_STATUS_READY => Vec::new(),
        "audit_missing" => vec!["reality_audit_missing".to_owned()],
        "audit_stale" => vec!["reality_audit_stale".to_owned()],
        "baseline_missing" => vec!["reality_baseline_missing".to_owned()],
        "delta_row_missing" => vec!["reality_delta_missing".to_owned()],
        "rebase_required" | "drift_detected" => vec!["reality_drift_rebase_required".to_owned()],
        _ => vec!["reality_context_unready".to_owned()],
    }
}

fn reality_transition(
    context: &EverQuestWorldSummaryRealityContext,
) -> Option<EverQuestWorldSummaryTransition> {
    if let Some(delta_key) = &context.newest_delta_key {
        return Some(EverQuestWorldSummaryTransition {
            transition_kind: "reality_delta".to_owned(),
            summary: sanitize_summary(&format!(
                "delta seq {:?} kind {:?} path {:?}; audit {}",
                context.newest_delta_seq,
                context.newest_delta_kind,
                context.newest_delta_path,
                context.audit_status
            )),
            source_ref: Some(EverQuestWorldSummarySourceRef {
                kind: "reality_delta".to_owned(),
                row_key: Some(delta_key.clone()),
                path: None,
                line_number: None,
                start_offset: None,
                next_offset: None,
                summary: Some("newest persisted delta used by summary".to_owned()),
            }),
        });
    }
    context
        .latest_audit_row_key
        .as_ref()
        .map(|row_key| EverQuestWorldSummaryTransition {
            transition_kind: "reality_audit".to_owned(),
            summary: sanitize_summary(&format!(
                "audit status {} drift {}",
                context.audit_status, context.drift_severity
            )),
            source_ref: Some(EverQuestWorldSummarySourceRef {
                kind: "reality_audit".to_owned(),
                row_key: Some(row_key.clone()),
                path: None,
                line_number: None,
                start_offset: None,
                next_offset: None,
                summary: Some("latest persisted audit used by summary".to_owned()),
            }),
        })
}

const fn drift_status_string(status: RealityDriftStatus) -> &'static str {
    match status {
        RealityDriftStatus::InSync => "in_sync",
        RealityDriftStatus::MinorDrift => "minor_drift",
        RealityDriftStatus::MajorDrift => "major_drift",
        RealityDriftStatus::RebaseRequired => "rebase_required",
        RealityDriftStatus::SourceUnavailable => "source_unavailable",
    }
}

fn reality_head_key(profile_key: &str) -> String {
    format!("reality/head/v1/{profile_key}")
}

fn reality_delta_key(profile_key: &str, epoch_id: &str, seq: u64) -> String {
    format!("reality/delta/v1/{profile_key}/{epoch_id}/{seq:020}")
}

fn reality_audit_prefix(profile_key: &str) -> String {
    format!("reality/audit/v1/{profile_key}/")
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

fn len_to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
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
            &ready_reality_context(),
        );

        assert_eq!(row.compact_status, "ready");
        assert_eq!(row.zone.short_name.as_deref(), Some("neriaka"));
        assert_eq!(row.level_progress.level, Some(1));
        assert_eq!(row.level_progress.xp_percent, Some(0.0));
        assert_eq!(row.nearest_exits[0].label, "to_Nektulos_Forest");
        assert_eq!(row.reality_context.audit_status, "in_sync");
        assert_eq!(row.reality_context.newest_delta_seq, Some(2));
        assert!(
            row.safe_next_probes
                .contains(&"planner_guard_before_any_movement".to_owned())
        );
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
            &ready_reality_context(),
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
            &ready_reality_context(),
        );

        assert_eq!(row.compact_status, "blocked");
        assert!(
            row.active_blockers
                .contains(&"map_graph_unavailable".to_owned())
        );
        assert!(row.nearest_landmarks.is_empty());
    }

    #[test]
    fn missing_reality_audit_blocks_blind_movement() {
        let params = params("missing-reality-audit");
        let source = source_state(Some("neriaka"), false);
        let reality = EverQuestWorldSummaryRealityContext {
            status: "audit_missing".to_owned(),
            audit_status: "not_run".to_owned(),
            latest_audit_row_key: None,
            latest_audit_id: None,
            latest_audit_ran_at: None,
            drift_severity: "unknown".to_owned(),
            drift_item_count: 0,
            rebase_required: false,
            safe_next_probe: "run_reality_audit_before_movement".to_owned(),
            ..ready_reality_context()
        };
        let row = build_summary_row(
            &params,
            &source,
            GraphContext {
                graph: Some(graph()),
                hazard: None,
                source_ref: None,
            },
            &reality,
        );

        assert_eq!(row.compact_status, "blocked");
        assert!(
            row.active_blockers
                .contains(&"reality_audit_missing".to_owned())
        );
        assert!(
            row.safe_next_probes
                .contains(&"run_reality_audit_before_movement".to_owned())
        );
    }

    #[test]
    fn stale_reality_audit_blocks_until_fresh_audit() {
        assert_eq!(reality_context_status(false, "stale", false), "audit_stale");
        let reality = EverQuestWorldSummaryRealityContext {
            status: "audit_stale".to_owned(),
            audit_status: "stale".to_owned(),
            drift_severity: "stale".to_owned(),
            safe_next_probe: "run_reality_audit_before_movement".to_owned(),
            ..ready_reality_context()
        };
        let blockers = reality_blockers(&reality);

        assert!(blockers.contains(&"reality_audit_stale".to_owned()));
        assert_eq!(
            reality_safe_next_probe(&reality.status),
            "run_reality_audit_before_movement"
        );
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
            xp_percent: Some(0.0),
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

    fn ready_reality_context() -> EverQuestWorldSummaryRealityContext {
        EverQuestWorldSummaryRealityContext {
            profile_key: "everquest.live".to_owned(),
            status: REALITY_STATUS_READY.to_owned(),
            head_key: "reality/head/v1/everquest.live".to_owned(),
            head_present: true,
            last_baseline_epoch_id: Some("issue-541-test".to_owned()),
            last_baseline_seq: Some(0),
            last_head_seq: Some(2),
            newest_delta_seq: Some(2),
            newest_delta_key: Some(
                "reality/delta/v1/everquest.live/issue-541-test/00000000000000000002".to_owned(),
            ),
            newest_delta_kind: Some("log_cursor_changed".to_owned()),
            newest_delta_path: Some("/events".to_owned()),
            newest_delta_at: Some(Utc::now()),
            audit_status: "in_sync".to_owned(),
            latest_audit_row_key: Some(
                "reality/audit/v1/everquest.live/audit-issue-541-test".to_owned(),
            ),
            latest_audit_id: Some("audit-issue-541-test".to_owned()),
            latest_audit_ran_at: Some(Utc::now()),
            drift_severity: "in_sync".to_owned(),
            drift_item_count: 0,
            rebase_required: false,
            safe_next_probe: "continue_with_delta_context".to_owned(),
            source_refs: vec![
                EverQuestWorldSummarySourceRef {
                    kind: "reality_head".to_owned(),
                    row_key: Some("reality/head/v1/everquest.live".to_owned()),
                    path: None,
                    line_number: None,
                    start_offset: None,
                    next_offset: None,
                    summary: Some("baseline epoch issue-541-test head seq 2".to_owned()),
                },
                EverQuestWorldSummarySourceRef {
                    kind: "reality_delta".to_owned(),
                    row_key: Some(
                        "reality/delta/v1/everquest.live/issue-541-test/00000000000000000002"
                            .to_owned(),
                    ),
                    path: None,
                    line_number: None,
                    start_offset: None,
                    next_offset: None,
                    summary: Some("newest delta seq 2".to_owned()),
                },
            ],
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
