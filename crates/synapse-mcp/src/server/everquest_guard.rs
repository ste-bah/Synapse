use chrono::{DateTime, Utc};
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use synapse_core::{Rect, error_codes};

use super::{
    Json, Parameters, SynapseService,
    everquest_log::EVERQUEST_PROFILE_ID,
    everquest_state::{CURRENT_STATE_ROW_KEY, EverQuestCurrentState, EverQuestStateSource},
    tool, tool_router,
};
use crate::m1::{current_input, mcp_error};

const TOOL: &str = "everquest_planner_guard";
const SCHEMA_VERSION: u32 = 1;
const GUARD_ROW_PREFIX: &str = "everquest/planner_guard_decision/v1";
const MAX_ID_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 512;
const MIN_STATE_CONFIDENCE: f32 = 0.50;
const SAFE_LEVEL_ONE_TARGET_MAX_LEVEL: u32 = 1;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardParams {
    pub decision_id: String,
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    pub candidate_kind: EverQuestPlannerCandidateKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotbar_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_con_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combat_readiness: Option<EverQuestPlannerGuardCombatReadiness>,
    #[serde(default = "default_state_row_key")]
    pub state_row_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_override: Option<EverQuestPlannerGuardStateOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_input_override: Option<EverQuestPlannerGuardChatInputOverride>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestPlannerCandidateKind {
    LocProbe,
    InventoryRead,
    MapRead,
    TargetConsider,
    BoundedMove,
    SitRest,
    CombatSpell,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardStateOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_short_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    #[serde(default)]
    pub has_location: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consider_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub hazards: Vec<EverQuestPlannerGuardHazard>,
    #[serde(default)]
    pub source_refs: Vec<EverQuestStateSource>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardHazard {
    pub code: String,
    pub severity: String,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardChatInputOverride {
    #[serde(default)]
    pub visible: bool,
    #[serde(default)]
    pub text_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denial_reason: Option<String>,
    #[serde(default)]
    pub decision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardCombatReadiness {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mana_percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_sitting: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rest_state: Option<String>,
    #[serde(default = "default_readiness_confidence")]
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_summary: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardResponse {
    pub ok: bool,
    pub row_key: String,
    pub stored_value_len_bytes: u64,
    pub decision: EverQuestPlannerGuardDecisionRow,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardDecisionRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub decision_id: String,
    pub row_key: String,
    pub generated_at: DateTime<Utc>,
    pub candidate: EverQuestPlannerGuardCandidate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combat_readiness: Option<EverQuestPlannerGuardCombatReadiness>,
    pub decision: String,
    pub selected: bool,
    pub rejected_reasons: Vec<String>,
    pub guard_results: Vec<EverQuestPlannerGuardResult>,
    pub source_state: EverQuestPlannerGuardStateReadback,
    pub foreground: EverQuestPlannerGuardForeground,
    pub chat_input: EverQuestPlannerGuardChatInputReadback,
    pub source_refs: Vec<EverQuestStateSource>,
    pub evidence_boundary: EverQuestPlannerGuardEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardCandidate {
    pub candidate_kind: EverQuestPlannerCandidateKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotbar_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_con_summary: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardResult {
    pub guard: String,
    pub passed: bool,
    pub severity: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardStateReadback {
    pub source_mode: String,
    pub state_row_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_short_name: Option<String>,
    pub zone_confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    pub level_confidence: f32,
    pub has_location: bool,
    pub location_confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consider_summary: Option<String>,
    pub hazards: Vec<EverQuestPlannerGuardHazard>,
    pub source_refs: Vec<EverQuestStateSource>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardForeground {
    pub is_everquest_foreground: bool,
    pub hwnd: i64,
    pub process_name: String,
    pub window_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardChatInputReadback {
    pub visible: bool,
    pub text_present: bool,
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denial_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_region: Option<Rect>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerGuardEvidenceBoundary {
    pub writes_guard_decision_only: bool,
    pub executes_input: bool,
    pub manual_fsv_required_for_runtime: bool,
    pub is_fsv: bool,
    pub redacted: bool,
    pub note: String,
}

#[derive(Clone, Debug)]
struct NormalizedParams {
    decision_id: String,
    profile_id: String,
    candidate: EverQuestPlannerGuardCandidate,
    combat_readiness: Option<EverQuestPlannerGuardCombatReadiness>,
    state_row_key: String,
    state_override: Option<EverQuestPlannerGuardStateReadback>,
    chat_input_override: Option<EverQuestPlannerGuardChatInputReadback>,
    row_key: String,
}

#[tool_router(router = everquest_guard_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Evaluate and persist one EverQuest planner guard decision before bounded foreground gameplay input"
    )]
    pub async fn everquest_planner_guard(
        &self,
        params: Parameters<EverQuestPlannerGuardParams>,
    ) -> Result<Json<EverQuestPlannerGuardResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TOOL,
            "tool.invocation kind=everquest_planner_guard"
        );
        let normalized = normalize_params(params.0)?;
        let foreground = self.read_planner_foreground()?;
        let source_state = self.read_planner_guard_state(&normalized)?;
        let chat_input = self.read_planner_chat_input(&normalized);
        let row = planner_guard_row(&normalized, foreground, &source_state, chat_input);
        let (decision, stored_value_len_bytes) =
            self.persist_planner_guard_json(&normalized.row_key, &row)?;
        Ok(Json(EverQuestPlannerGuardResponse {
            ok: true,
            row_key: normalized.row_key,
            stored_value_len_bytes,
            decision,
        }))
    }
}

impl SynapseService {
    fn read_planner_foreground(&self) -> Result<EverQuestPlannerGuardForeground, ErrorData> {
        let mut input = {
            let state = self.m1_state()?;
            current_input(&state, 1)?
        };
        self.resolve_input_profile_and_hud(&mut input, false);
        Ok(EverQuestPlannerGuardForeground {
            is_everquest_foreground: input
                .foreground
                .profile_id
                .as_deref()
                .is_some_and(|profile_id| profile_id == EVERQUEST_PROFILE_ID)
                || input
                    .foreground
                    .process_name
                    .eq_ignore_ascii_case("eqgame.exe"),
            hwnd: input.foreground.hwnd,
            process_name: input.foreground.process_name,
            window_title: input.foreground.window_title,
            profile_id: input.foreground.profile_id,
        })
    }

    fn read_planner_guard_state(
        &self,
        params: &NormalizedParams,
    ) -> Result<EverQuestPlannerGuardStateReadback, ErrorData> {
        if let Some(override_state) = &params.state_override {
            return Ok(override_state.clone());
        }
        let stored = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_error| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while reading EverQuest planner state row",
                )
            })?;
            runtime
                .storage_kv_row(params.state_row_key.as_bytes())
                .map_err(|error| mcp_error(error.code(), error.to_string()))?
        };
        let Some(stored) = stored else {
            return Ok(EverQuestPlannerGuardStateReadback {
                source_mode: "current_state_row_missing".to_owned(),
                state_row_key: params.state_row_key.clone(),
                generated_at: None,
                zone_short_name: None,
                zone_confidence: 0.0,
                level: None,
                level_confidence: 0.0,
                has_location: false,
                location_confidence: 0.0,
                target_summary: None,
                consider_summary: None,
                hazards: Vec::new(),
                source_refs: Vec::new(),
            });
        };
        let state =
            decode_json_row::<EverQuestCurrentState>(&stored, "EverQuest current-state row")?;
        Ok(state_readback_from_current_row(
            &params.state_row_key,
            &state,
        ))
    }

    fn read_planner_chat_input(
        &self,
        params: &NormalizedParams,
    ) -> EverQuestPlannerGuardChatInputReadback {
        if let Some(override_state) = &params.chat_input_override {
            return override_state.clone();
        }
        let chat = self.detect_everquest_chat_input_state();
        EverQuestPlannerGuardChatInputReadback {
            visible: chat.visible,
            text_present: chat.text_present,
            decision: chat.decision,
            denial_reason: chat.denial_reason,
            source_region: chat.source_region,
        }
    }

    fn persist_planner_guard_json(
        &self,
        key: &str,
        row: &EverQuestPlannerGuardDecisionRow,
    ) -> Result<(EverQuestPlannerGuardDecisionRow, u64), ErrorData> {
        let encoded = serde_json::to_vec(row).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode EverQuest planner guard row: {error}"),
            )
        })?;
        let stored = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_error| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while writing EverQuest planner guard row",
                )
            })?;
            runtime
                .storage_put_kv_rows(vec![(key.as_bytes().to_vec(), encoded)])
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_WRITE_FAILED,
                        format!("write EverQuest planner guard row: {error}"),
                    )
                })?;
            runtime
                .storage_kv_row(key.as_bytes())
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        format!("read EverQuest planner guard row after write: {error}"),
                    )
                })?
                .ok_or_else(|| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        "EverQuest planner guard row missing after write",
                    )
                })?
        };
        let readback = decode_json_row::<EverQuestPlannerGuardDecisionRow>(
            &stored,
            "EverQuest planner guard row",
        )?;
        Ok((readback, len_to_u64(stored.len())))
    }
}

fn planner_guard_row(
    params: &NormalizedParams,
    foreground: EverQuestPlannerGuardForeground,
    source_state: &EverQuestPlannerGuardStateReadback,
    chat_input: EverQuestPlannerGuardChatInputReadback,
) -> EverQuestPlannerGuardDecisionRow {
    let guards = planner_guard_results(params, &foreground, source_state, &chat_input);
    let rejected_reasons = guards
        .iter()
        .filter(|guard| !guard.passed)
        .map(|guard| guard.guard.clone())
        .collect::<Vec<_>>();
    let selected = rejected_reasons.is_empty();
    EverQuestPlannerGuardDecisionRow {
        schema_version: SCHEMA_VERSION,
        row_kind: "everquest_planner_guard_decision".to_owned(),
        profile_id: params.profile_id.clone(),
        decision_id: params.decision_id.clone(),
        row_key: params.row_key.clone(),
        generated_at: Utc::now(),
        candidate: params.candidate.clone(),
        combat_readiness: params.combat_readiness.clone(),
        decision: if selected { "select" } else { "reject" }.to_owned(),
        selected,
        rejected_reasons,
        guard_results: guards,
        source_state: source_state.clone(),
        foreground,
        chat_input,
        source_refs: source_state_to_refs(&params.state_row_key, source_state),
        evidence_boundary: EverQuestPlannerGuardEvidenceBoundary {
            writes_guard_decision_only: true,
            executes_input: false,
            manual_fsv_required_for_runtime: true,
            is_fsv: false,
            redacted: true,
            note: "Planner guard rows are decision evidence only; manual FSV still reads the physical game/UI/log/storage SoT before and after any real action."
                .to_owned(),
        },
    }
}

fn planner_guard_results(
    params: &NormalizedParams,
    foreground: &EverQuestPlannerGuardForeground,
    source_state: &EverQuestPlannerGuardStateReadback,
    chat_input: &EverQuestPlannerGuardChatInputReadback,
) -> Vec<EverQuestPlannerGuardResult> {
    let mut guards = Vec::new();
    push_guard(
        &mut guards,
        "foreground_everquest_live",
        foreground.is_everquest_foreground
            && foreground.profile_id.as_deref() == Some(EVERQUEST_PROFILE_ID),
        "critical",
        if foreground.is_everquest_foreground {
            format!(
                "foreground process={} profile={:?}",
                foreground.process_name, foreground.profile_id
            )
        } else {
            format!(
                "foreground process={} is not EverQuest",
                foreground.process_name
            )
        },
    );
    push_guard(
        &mut guards,
        "chat_input_safe",
        chat_input_allows_guard(chat_input),
        "critical",
        format!(
            "chat decision={} visible={} text_present={}",
            chat_input.decision, chat_input.visible, chat_input.text_present
        ),
    );
    push_guard(
        &mut guards,
        "state_row_available",
        source_state.source_mode != "current_state_row_missing",
        "critical",
        source_state.source_mode.clone(),
    );
    push_guard(
        &mut guards,
        "zone_known",
        source_state.zone_short_name.is_some()
            && source_state.zone_confidence >= MIN_STATE_CONFIDENCE,
        "critical",
        format!(
            "zone={:?} confidence={:.3}",
            source_state.zone_short_name, source_state.zone_confidence
        ),
    );
    push_guard(
        &mut guards,
        "level_known",
        candidate_requires_level(&params.candidate.candidate_kind)
            .then_some(())
            .is_none()
            || source_state.level.is_some()
                && source_state.level_confidence >= MIN_STATE_CONFIDENCE,
        "critical",
        format!(
            "level={:?} confidence={:.3}",
            source_state.level, source_state.level_confidence
        ),
    );
    push_guard(
        &mut guards,
        "location_known_for_movement",
        params.candidate.candidate_kind != EverQuestPlannerCandidateKind::BoundedMove
            || source_state.has_location
                && source_state.location_confidence >= MIN_STATE_CONFIDENCE,
        "critical",
        format!(
            "has_location={} confidence={:.3}",
            source_state.has_location, source_state.location_confidence
        ),
    );
    push_guard(
        &mut guards,
        "no_stop_hazard",
        !has_stop_hazard(source_state, &params.candidate.candidate_kind),
        "critical",
        hazard_reason(source_state),
    );
    add_candidate_specific_guards(&mut guards, params, source_state);
    guards
}

fn add_candidate_specific_guards(
    guards: &mut Vec<EverQuestPlannerGuardResult>,
    params: &NormalizedParams,
    source_state: &EverQuestPlannerGuardStateReadback,
) {
    let candidate = &params.candidate;
    match candidate.candidate_kind {
        EverQuestPlannerCandidateKind::LocProbe
        | EverQuestPlannerCandidateKind::InventoryRead
        | EverQuestPlannerCandidateKind::MapRead
        | EverQuestPlannerCandidateKind::TargetConsider
        | EverQuestPlannerCandidateKind::SitRest => {
            push_guard(
                guards,
                "candidate_is_bounded_probe",
                true,
                "info",
                "candidate does not engage combat or social/economy surfaces",
            );
        }
        EverQuestPlannerCandidateKind::BoundedMove => {
            push_guard(
                guards,
                "bounded_movement_only",
                true,
                "info",
                "movement candidate must be executed as a short probe and re-estimated afterward",
            );
        }
        EverQuestPlannerCandidateKind::CombatSpell => {
            let hotbar_ok = candidate.hotbar_alias.as_deref() == Some("hotbar4");
            push_guard(
                guards,
                "verified_attack_spell",
                hotbar_ok,
                "critical",
                format!(
                    "hotbar_alias={:?}; only hotbar4 Blast of Cold is verified as an attack spell",
                    candidate.hotbar_alias
                ),
            );
            let target_text = candidate
                .target_name
                .as_deref()
                .or(source_state.target_summary.as_deref())
                .unwrap_or_default();
            push_guard(
                guards,
                "target_known",
                !target_text.trim().is_empty(),
                "critical",
                if target_text.trim().is_empty() {
                    "no target name or current-state target summary is available".to_owned()
                } else {
                    "target evidence is present".to_owned()
                },
            );
            push_guard(
                guards,
                "target_is_npc",
                target_is_npc(source_state),
                "critical",
                target_identity_reason(candidate, source_state),
            );
            let level = source_state.level.unwrap_or_default();
            let target_level = candidate.target_level.unwrap_or(u32::MAX);
            push_guard(
                guards,
                "target_level_safe_for_level_one_wizard",
                level == 1 && target_level <= SAFE_LEVEL_ONE_TARGET_MAX_LEVEL,
                "critical",
                format!("character_level={level} target_level={target_level}"),
            );
            let con = candidate
                .target_con_summary
                .as_deref()
                .or(source_state.consider_summary.as_deref())
                .unwrap_or_default();
            let con_known = !con.trim().is_empty();
            push_guard(
                guards,
                "target_con_known",
                con_known,
                "critical",
                if con_known {
                    "consider evidence is present".to_owned()
                } else {
                    "no target consider summary is available".to_owned()
                },
            );
            push_guard(
                guards,
                "target_con_safe",
                con_known && !con_is_unsafe_for_level_one(con),
                "critical",
                if con.is_empty() {
                    "no target consider summary is available".to_owned()
                } else {
                    format!("con_summary={con}")
                },
            );
            add_combat_readiness_guards(guards, params.combat_readiness.as_ref());
        }
    }
}

fn add_combat_readiness_guards(
    guards: &mut Vec<EverQuestPlannerGuardResult>,
    readiness: Option<&EverQuestPlannerGuardCombatReadiness>,
) {
    let Some(readiness) = readiness else {
        push_guard(
            guards,
            "combat_readiness_known",
            false,
            "critical",
            "health, mana, and rest-state evidence are absent",
        );
        return;
    };
    push_guard(
        guards,
        "combat_readiness_known",
        true,
        "critical",
        readiness_reason(readiness),
    );
    push_guard(
        guards,
        "combat_readiness_confident",
        readiness.confidence >= MIN_STATE_CONFIDENCE,
        "critical",
        format!("confidence={:.3}", readiness.confidence),
    );
    push_guard(
        guards,
        "health_known_safe",
        readiness.health_percent.is_some_and(|value| value >= 80),
        "critical",
        readiness.health_percent.map_or_else(
            || "health_percent missing".to_owned(),
            |value| format!("health_percent={value}"),
        ),
    );
    push_guard(
        guards,
        "mana_known_safe",
        readiness.mana_percent.is_some_and(|value| value >= 30),
        "critical",
        readiness.mana_percent.map_or_else(
            || "mana_percent missing".to_owned(),
            |value| format!("mana_percent={value}"),
        ),
    );
    push_guard(
        guards,
        "rest_state_known",
        readiness.is_sitting.is_some(),
        "critical",
        readiness.is_sitting.map_or_else(
            || "rest/casting posture is unknown".to_owned(),
            |is_sitting| format!("is_sitting={is_sitting}"),
        ),
    );
    push_guard(
        guards,
        "standing_to_cast",
        readiness.is_sitting == Some(false),
        "critical",
        readiness.is_sitting.map_or_else(
            || "cannot prove standing for casting".to_owned(),
            |is_sitting| format!("is_sitting={is_sitting}"),
        ),
    );
}

fn push_guard(
    guards: &mut Vec<EverQuestPlannerGuardResult>,
    guard: impl Into<String>,
    passed: bool,
    severity: impl Into<String>,
    reason: impl Into<String>,
) {
    guards.push(EverQuestPlannerGuardResult {
        guard: guard.into(),
        passed,
        severity: severity.into(),
        reason: reason.into(),
    });
}

fn chat_input_allows_guard(chat_input: &EverQuestPlannerGuardChatInputReadback) -> bool {
    chat_input.decision == "allow_empty_chat_input"
        && chat_input.visible
        && !chat_input.text_present
        && chat_input.denial_reason.is_none()
}

const fn candidate_requires_level(candidate: &EverQuestPlannerCandidateKind) -> bool {
    matches!(candidate, EverQuestPlannerCandidateKind::CombatSpell)
}

fn has_stop_hazard(
    source_state: &EverQuestPlannerGuardStateReadback,
    _candidate: &EverQuestPlannerCandidateKind,
) -> bool {
    source_state.hazards.iter().any(|hazard| {
        hazard.severity.eq_ignore_ascii_case("critical")
            || hazard.code.contains("death")
            || hazard.code.contains("aggro")
            || hazard.code.contains("combat")
            || hazard.code.contains("unexpected_zone")
    })
}

fn hazard_reason(source_state: &EverQuestPlannerGuardStateReadback) -> String {
    if source_state.hazards.is_empty() {
        return "no stop hazards in source state".to_owned();
    }
    source_state
        .hazards
        .iter()
        .map(|hazard| format!("{}:{}:{}", hazard.code, hazard.severity, hazard.detail))
        .collect::<Vec<_>>()
        .join("; ")
}

fn con_is_unsafe_for_level_one(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    normalized.contains("gamble")
        || normalized.contains("danger")
        || normalized.contains("deadly")
        || normalized.contains("threat")
        || normalized.contains("red")
        || normalized.contains("lvl: 2")
        || normalized.contains("lvl: 3")
        || normalized.contains("lvl: 4")
        || normalized.contains("lvl: 5")
        || normalized.contains("lvl: 6")
}

fn target_is_npc(source_state: &EverQuestPlannerGuardStateReadback) -> bool {
    if let Some(summary) = source_state.target_summary.as_deref() {
        let normalized = summary.to_ascii_lowercase();
        if normalized.contains("target player") {
            return false;
        }
        if normalized.contains("target npc") {
            return true;
        }
    }
    false
}

fn target_identity_reason(
    candidate: &EverQuestPlannerGuardCandidate,
    source_state: &EverQuestPlannerGuardStateReadback,
) -> String {
    if let Some(summary) = source_state.target_summary.as_deref() {
        return format!("target_summary={summary}");
    }
    if candidate.target_name.is_some() {
        return "candidate target name is present but current-state NPC evidence is absent"
            .to_owned();
    }
    "no target identity evidence is available".to_owned()
}

fn readiness_reason(readiness: &EverQuestPlannerGuardCombatReadiness) -> String {
    format!(
        "health={:?} mana={:?} is_sitting={:?} confidence={:.3}",
        readiness.health_percent,
        readiness.mana_percent,
        readiness.is_sitting,
        readiness.confidence
    )
}

fn state_readback_from_current_row(
    state_row_key: &str,
    state: &EverQuestCurrentState,
) -> EverQuestPlannerGuardStateReadback {
    EverQuestPlannerGuardStateReadback {
        source_mode: "current_state_row".to_owned(),
        state_row_key: state_row_key.to_owned(),
        generated_at: Some(state.generated_at),
        zone_short_name: state.zone_short_name.value.clone(),
        zone_confidence: state.zone_short_name.confidence,
        level: state.level.value,
        level_confidence: state.level.confidence,
        has_location: state.location.value.is_some(),
        location_confidence: state.location.confidence,
        target_summary: state.target.value.clone(),
        consider_summary: state.consider.value.clone(),
        hazards: state
            .hazards
            .iter()
            .map(|hazard| EverQuestPlannerGuardHazard {
                code: hazard.code.clone(),
                severity: hazard.severity.clone(),
                detail: hazard.detail.clone(),
            })
            .collect(),
        source_refs: current_state_sources(state),
    }
}

fn current_state_sources(state: &EverQuestCurrentState) -> Vec<EverQuestStateSource> {
    let mut refs = Vec::new();
    refs.extend(state.zone.sources.clone());
    refs.extend(state.zone_short_name.sources.clone());
    refs.extend(state.location.sources.clone());
    refs.extend(state.level.sources.clone());
    refs.extend(state.target.sources.clone());
    refs.extend(state.consider.sources.clone());
    refs
}

fn source_state_to_refs(
    state_row_key: &str,
    state: &EverQuestPlannerGuardStateReadback,
) -> Vec<EverQuestStateSource> {
    let mut refs = state.source_refs.clone();
    refs.insert(
        0,
        EverQuestStateSource {
            kind: state.source_mode.clone(),
            path: None,
            start_offset: None,
            next_offset: None,
            log_timestamp: None,
            summary: Some(format!("state_row_key={state_row_key}")),
        },
    );
    if let Some(zone) = &state.zone_short_name {
        refs.push(EverQuestStateSource {
            kind: "zone_short_name".to_owned(),
            path: None,
            start_offset: None,
            next_offset: None,
            log_timestamp: None,
            summary: Some(zone.clone()),
        });
    }
    refs
}

fn normalize_params(params: EverQuestPlannerGuardParams) -> Result<NormalizedParams, ErrorData> {
    let profile_id = validate_everquest_profile_id(&params.profile_id)?;
    let decision_id = validate_id("decision_id", &params.decision_id)?;
    let state_row_key = normalize_required_text("state_row_key", &params.state_row_key)?;
    let candidate = EverQuestPlannerGuardCandidate {
        candidate_kind: params.candidate_kind,
        candidate_label: params
            .candidate_label
            .map(|value| normalize_required_text("candidate_label", &value))
            .transpose()?,
        hotbar_alias: params
            .hotbar_alias
            .map(|value| validate_id("hotbar_alias", &value))
            .transpose()?,
        target_name: params
            .target_name
            .map(|value| normalize_required_text("target_name", &value))
            .transpose()?,
        target_level: params.target_level,
        target_con_summary: params
            .target_con_summary
            .map(|value| normalize_required_text("target_con_summary", &value))
            .transpose()?,
    };
    let combat_readiness = params
        .combat_readiness
        .map(normalize_combat_readiness)
        .transpose()?;
    let state_override = params
        .state_override
        .map(|value| normalize_state_override(&state_row_key, value))
        .transpose()?;
    let chat_input_override = params
        .chat_input_override
        .map(normalize_chat_override)
        .transpose()?;
    let row_key = guard_row_key(&profile_id, &decision_id);
    Ok(NormalizedParams {
        decision_id,
        profile_id,
        candidate,
        combat_readiness,
        state_row_key,
        state_override,
        chat_input_override,
        row_key,
    })
}

fn normalize_combat_readiness(
    readiness: EverQuestPlannerGuardCombatReadiness,
) -> Result<EverQuestPlannerGuardCombatReadiness, ErrorData> {
    if readiness.health_percent.is_some_and(|value| value > 100) {
        return Err(params_error(
            "combat_readiness.health_percent must be <= 100",
        ));
    }
    if readiness.mana_percent.is_some_and(|value| value > 100) {
        return Err(params_error("combat_readiness.mana_percent must be <= 100"));
    }
    validate_unit_interval("combat_readiness.confidence", readiness.confidence)?;
    let rest_state = readiness
        .rest_state
        .map(|value| normalize_required_text("combat_readiness.rest_state", &value))
        .transpose()?;
    let source_summary = readiness
        .source_summary
        .map(|value| normalize_required_text("combat_readiness.source_summary", &value))
        .transpose()?;
    Ok(EverQuestPlannerGuardCombatReadiness {
        health_percent: readiness.health_percent,
        mana_percent: readiness.mana_percent,
        is_sitting: readiness.is_sitting,
        rest_state,
        confidence: readiness.confidence,
        source_summary,
    })
}

fn normalize_state_override(
    state_row_key: &str,
    override_state: EverQuestPlannerGuardStateOverride,
) -> Result<EverQuestPlannerGuardStateReadback, ErrorData> {
    let zone_short_name = override_state
        .zone_short_name
        .map(|value| validate_id("state_override.zone_short_name", &value))
        .transpose()?;
    let confidence = override_state.confidence.unwrap_or(0.75);
    validate_unit_interval("state_override.confidence", confidence)?;
    let target_summary = override_state
        .target_summary
        .map(|value| normalize_required_text("state_override.target_summary", &value))
        .transpose()?;
    let consider_summary = override_state
        .consider_summary
        .map(|value| normalize_required_text("state_override.consider_summary", &value))
        .transpose()?;
    Ok(EverQuestPlannerGuardStateReadback {
        source_mode: "state_override".to_owned(),
        state_row_key: state_row_key.to_owned(),
        generated_at: None,
        zone_short_name,
        zone_confidence: confidence,
        level: override_state.level,
        level_confidence: if override_state.level.is_some() {
            confidence
        } else {
            0.0
        },
        has_location: override_state.has_location,
        location_confidence: if override_state.has_location {
            confidence
        } else {
            0.0
        },
        target_summary,
        consider_summary,
        hazards: override_state.hazards,
        source_refs: override_state.source_refs,
    })
}

fn normalize_chat_override(
    override_state: EverQuestPlannerGuardChatInputOverride,
) -> Result<EverQuestPlannerGuardChatInputReadback, ErrorData> {
    let decision = if override_state.decision.trim().is_empty() {
        if override_state.visible && !override_state.text_present {
            "allow_empty_chat_input".to_owned()
        } else {
            "deny_chat_input_override".to_owned()
        }
    } else {
        normalize_required_text("chat_input_override.decision", &override_state.decision)?
    };
    Ok(EverQuestPlannerGuardChatInputReadback {
        visible: override_state.visible,
        text_present: override_state.text_present,
        decision,
        denial_reason: override_state.denial_reason,
        source_region: None,
    })
}

fn validate_everquest_profile_id(value: &str) -> Result<String, ErrorData> {
    let value = validate_id("profile_id", value)?;
    if value != EVERQUEST_PROFILE_ID {
        return Err(params_error(format!(
            "profile_id must be {EVERQUEST_PROFILE_ID:?}"
        )));
    }
    Ok(value)
}

fn validate_id(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error(format!("{field} must not be empty")));
    }
    if value.len() > MAX_ID_BYTES {
        return Err(params_error(format!("{field} is too long")));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
    {
        return Err(params_error(format!(
            "{field} must contain only ASCII letters, digits, '.', '_', '-' or ':'"
        )));
    }
    Ok(value.to_owned())
}

fn normalize_required_text(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error(format!("{field} must not be empty")));
    }
    if value.len() > MAX_TEXT_BYTES {
        return Err(params_error(format!("{field} is too long")));
    }
    Ok(value.to_owned())
}

fn validate_unit_interval(field: &str, value: f32) -> Result<(), ErrorData> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(params_error(format!("{field} must be between 0.0 and 1.0")));
    }
    Ok(())
}

fn guard_row_key(profile_id: &str, decision_id: &str) -> String {
    format!("{GUARD_ROW_PREFIX}/{profile_id}/{decision_id}")
}

fn default_profile_id() -> String {
    EVERQUEST_PROFILE_ID.to_owned()
}

fn default_state_row_key() -> String {
    CURRENT_STATE_ROW_KEY.to_owned()
}

const fn default_readiness_confidence() -> f32 {
    0.0
}

fn params_error(message: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, message.into())
}

fn decode_json_row<T>(bytes: &[u8], label: &str) -> Result<T, ErrorData>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("decode {label}: {error}"),
        )
    })
}

fn len_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_selects_safe_loc_probe() {
        let params = normalized_for(
            "safe-loc",
            EverQuestPlannerCandidateKind::LocProbe,
            Some("allow_empty_chat_input"),
        );
        let row = planner_guard_row(
            &params,
            foreground(true),
            &state(Some("neriaka"), Some(1), true, Vec::new()),
            chat(false),
        );

        assert!(row.selected);
        assert_eq!(row.decision, "select");
        assert!(row.rejected_reasons.is_empty());
    }

    #[test]
    fn guard_rejects_chat_input_text() {
        let params = normalized_for(
            "chat-text",
            EverQuestPlannerCandidateKind::InventoryRead,
            Some("deny_visible_chat_input_text"),
        );
        let row = planner_guard_row(
            &params,
            foreground(true),
            &state(Some("neriaka"), Some(1), true, Vec::new()),
            EverQuestPlannerGuardChatInputReadback {
                visible: true,
                text_present: true,
                decision: "deny_visible_chat_input_text".to_owned(),
                denial_reason: Some("visible text in chat input".to_owned()),
                source_region: None,
            },
        );

        assert!(!row.selected);
        assert!(row.rejected_reasons.contains(&"chat_input_safe".to_owned()));
    }

    #[test]
    fn guard_rejects_chat_deny_decision_without_text() {
        let params = normalized_for(
            "chat-deny",
            EverQuestPlannerCandidateKind::InventoryRead,
            Some("deny_ambiguous_chat_state"),
        );
        let row = planner_guard_row(
            &params,
            foreground(true),
            &state(Some("neriaka"), Some(1), true, Vec::new()),
            EverQuestPlannerGuardChatInputReadback {
                visible: true,
                text_present: false,
                decision: "deny_ambiguous_chat_state".to_owned(),
                denial_reason: None,
                source_region: None,
            },
        );

        assert!(!row.selected);
        assert!(row.rejected_reasons.contains(&"chat_input_safe".to_owned()));
    }

    #[test]
    fn guard_rejects_level_two_gamble_combat() {
        let mut params = normalized_for(
            "unsafe-combat",
            EverQuestPlannerCandidateKind::CombatSpell,
            Some("allow_empty_chat_input"),
        );
        params.candidate.hotbar_alias = Some("hotbar4".to_owned());
        params.candidate.target_level = Some(2);
        params.candidate.target_con_summary =
            Some("looks like quite a gamble. (Lvl: 2)".to_owned());
        let row = planner_guard_row(
            &params,
            foreground(true),
            &state(Some("nektulos"), Some(1), true, Vec::new()),
            chat(false),
        );

        assert!(!row.selected);
        assert!(
            row.rejected_reasons
                .contains(&"target_level_safe_for_level_one_wizard".to_owned())
        );
        assert!(row.rejected_reasons.contains(&"target_con_safe".to_owned()));
    }

    #[test]
    fn guard_rejects_unknown_zone_for_movement() {
        let params = normalized_for(
            "unknown-zone",
            EverQuestPlannerCandidateKind::BoundedMove,
            Some("allow_empty_chat_input"),
        );
        let row = planner_guard_row(
            &params,
            foreground(true),
            &state(None, Some(1), true, Vec::new()),
            chat(false),
        );

        assert!(!row.selected);
        assert!(row.rejected_reasons.contains(&"zone_known".to_owned()));
    }

    #[test]
    fn guard_rejects_stop_hazard_for_loc_probe() {
        let params = normalized_for(
            "aggro-loc",
            EverQuestPlannerCandidateKind::LocProbe,
            Some("allow_empty_chat_input"),
        );
        let row = planner_guard_row(
            &params,
            foreground(true),
            &state(
                Some("neriaka"),
                Some(1),
                true,
                vec![EverQuestPlannerGuardHazard {
                    code: "aggro_detected".to_owned(),
                    severity: "critical".to_owned(),
                    detail: "synthetic hostile state".to_owned(),
                }],
            ),
            chat(false),
        );

        assert!(!row.selected);
        assert!(row.rejected_reasons.contains(&"no_stop_hazard".to_owned()));
    }

    fn normalized_for(
        decision_id: &str,
        candidate_kind: EverQuestPlannerCandidateKind,
        _chat_decision: Option<&str>,
    ) -> NormalizedParams {
        NormalizedParams {
            decision_id: decision_id.to_owned(),
            profile_id: EVERQUEST_PROFILE_ID.to_owned(),
            candidate: EverQuestPlannerGuardCandidate {
                candidate_kind,
                candidate_label: None,
                hotbar_alias: None,
                target_name: None,
                target_level: None,
                target_con_summary: None,
            },
            combat_readiness: None,
            state_row_key: CURRENT_STATE_ROW_KEY.to_owned(),
            state_override: None,
            chat_input_override: None,
            row_key: guard_row_key(EVERQUEST_PROFILE_ID, decision_id),
        }
    }

    fn foreground(ok: bool) -> EverQuestPlannerGuardForeground {
        EverQuestPlannerGuardForeground {
            is_everquest_foreground: ok,
            hwnd: if ok { 1 } else { 2 },
            process_name: if ok { "eqgame.exe" } else { "notepad.exe" }.to_owned(),
            window_title: if ok {
                "EverQuest"
            } else {
                "Untitled - Notepad"
            }
            .to_owned(),
            profile_id: ok.then(|| EVERQUEST_PROFILE_ID.to_owned()),
        }
    }

    fn state(
        zone: Option<&str>,
        level: Option<u32>,
        has_location: bool,
        hazards: Vec<EverQuestPlannerGuardHazard>,
    ) -> EverQuestPlannerGuardStateReadback {
        EverQuestPlannerGuardStateReadback {
            source_mode: "state_override".to_owned(),
            state_row_key: CURRENT_STATE_ROW_KEY.to_owned(),
            generated_at: None,
            zone_short_name: zone.map(str::to_owned),
            zone_confidence: zone.map_or(0.0, |_| 0.9),
            level,
            level_confidence: level.map_or(0.0, |_| 0.9),
            has_location,
            location_confidence: if has_location { 0.9 } else { 0.0 },
            target_summary: None,
            consider_summary: None,
            hazards,
            source_refs: Vec::new(),
        }
    }

    fn chat(text_present: bool) -> EverQuestPlannerGuardChatInputReadback {
        EverQuestPlannerGuardChatInputReadback {
            visible: true,
            text_present,
            decision: if text_present {
                "deny_visible_chat_input_text"
            } else {
                "allow_empty_chat_input"
            }
            .to_owned(),
            denial_reason: None,
            source_region: None,
        }
    }

    fn ready_for_combat() -> EverQuestPlannerGuardCombatReadiness {
        EverQuestPlannerGuardCombatReadiness {
            health_percent: Some(100),
            mana_percent: Some(80),
            is_sitting: Some(false),
            rest_state: Some("standing".to_owned()),
            confidence: 0.9,
            source_summary: Some("synthetic visible health/mana/rest readback".to_owned()),
        }
    }

    #[test]
    fn guard_selects_level_one_safe_combat_with_readiness() {
        let mut params = normalized_for(
            "safe-combat",
            EverQuestPlannerCandidateKind::CombatSpell,
            Some("allow_empty_chat_input"),
        );
        params.candidate.hotbar_alias = Some("hotbar4".to_owned());
        params.candidate.target_name = Some("synthetic level-one npc".to_owned());
        params.candidate.target_level = Some(1);
        params.candidate.target_con_summary = Some("looks like an even fight. (Lvl: 1)".to_owned());
        params.combat_readiness = Some(ready_for_combat());
        let mut source_state = state(Some("nektulos"), Some(1), true, Vec::new());
        source_state.target_summary = Some("target npc synthetic level-one npc".to_owned());
        let row = planner_guard_row(&params, foreground(true), &source_state, chat(false));

        assert!(row.selected);
        assert_eq!(row.decision, "select");
        assert!(row.combat_readiness.is_some());
    }

    #[test]
    fn guard_rejects_combat_without_readiness() {
        let mut params = normalized_for(
            "unknown-readiness-combat",
            EverQuestPlannerCandidateKind::CombatSpell,
            Some("allow_empty_chat_input"),
        );
        params.candidate.hotbar_alias = Some("hotbar4".to_owned());
        params.candidate.target_name = Some("synthetic level-one npc".to_owned());
        params.candidate.target_level = Some(1);
        params.candidate.target_con_summary = Some("looks like an even fight. (Lvl: 1)".to_owned());
        let mut source_state = state(Some("nektulos"), Some(1), true, Vec::new());
        source_state.target_summary = Some("target npc synthetic level-one npc".to_owned());
        let row = planner_guard_row(&params, foreground(true), &source_state, chat(false));

        assert!(!row.selected);
        assert!(
            row.rejected_reasons
                .contains(&"combat_readiness_known".to_owned())
        );
    }

    #[test]
    fn guard_rejects_player_target_combat() {
        let mut params = normalized_for(
            "player-target-combat",
            EverQuestPlannerCandidateKind::CombatSpell,
            Some("allow_empty_chat_input"),
        );
        params.candidate.hotbar_alias = Some("hotbar4".to_owned());
        params.candidate.target_name = Some("synthetic player".to_owned());
        params.candidate.target_level = Some(1);
        params.candidate.target_con_summary = Some("looks like an even fight. (Lvl: 1)".to_owned());
        params.combat_readiness = Some(ready_for_combat());
        let mut source_state = state(Some("nektulos"), Some(1), true, Vec::new());
        source_state.target_summary = Some("target player synthetic player".to_owned());
        let row = planner_guard_row(&params, foreground(true), &source_state, chat(false));

        assert!(!row.selected);
        assert!(row.rejected_reasons.contains(&"target_is_npc".to_owned()));
    }

    #[test]
    fn guard_rejects_no_target_combat() {
        let mut params = normalized_for(
            "no-target-combat",
            EverQuestPlannerCandidateKind::CombatSpell,
            Some("allow_empty_chat_input"),
        );
        params.candidate.hotbar_alias = Some("hotbar4".to_owned());
        params.candidate.target_level = Some(1);
        params.candidate.target_con_summary = Some("looks like an even fight. (Lvl: 1)".to_owned());
        params.combat_readiness = Some(ready_for_combat());
        let row = planner_guard_row(
            &params,
            foreground(true),
            &state(Some("nektulos"), Some(1), true, Vec::new()),
            chat(false),
        );

        assert!(!row.selected);
        assert!(row.rejected_reasons.contains(&"target_known".to_owned()));
    }
}
