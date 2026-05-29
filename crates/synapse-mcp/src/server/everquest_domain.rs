#![allow(clippy::derive_partial_eq_without_eq)]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use synapse_core::error_codes;

use super::{
    Json, Parameters, SynapseService, everquest_log::EVERQUEST_PROFILE_ID, tool, tool_router,
};
use crate::m1::mcp_error;

const TOOL: &str = "everquest_domain_normalize";
const SCHEMA_VERSION: u32 = 1;
const DOMAIN_PACK_ID: &str = "everquest_dynamicjepa_v1";
const DOMAIN_PACK_VERSION: &str = "1.0.0";
const DOMAIN_PACK_ROW_PREFIX: &str = "everquest/dynamicjepa_domain_pack/v1";
const STATE_ROW_PREFIX: &str = "everquest/dynamicjepa_state/v1";
const ACTION_ROW_PREFIX: &str = "everquest/dynamicjepa_action/v1";
const OUTCOME_ROW_PREFIX: &str = "everquest/dynamicjepa_outcome/v1";
const TRANSITION_ROW_PREFIX: &str = "everquest/dynamicjepa_transition/v1";
const MAX_ID_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 512;
const MAX_SOURCE_REFS: usize = 32;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainNormalizeParams {
    pub transition_id: String,
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    pub state: EverQuestDomainStateInput,
    pub action: EverQuestDomainActionInput,
    pub outcome: EverQuestDomainOutcomeInput,
    pub entity: EverQuestDomainEntityInput,
    #[serde(default)]
    pub source_refs: Vec<EverQuestDomainSourceRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainNormalizeResponse {
    pub ok: bool,
    pub profile_id: String,
    pub transition_id: String,
    pub validation_status: EverQuestDomainValidationStatus,
    pub accepted_for_planning: bool,
    pub row_keys: EverQuestDomainRowKeys,
    pub stored_value_len_bytes: EverQuestDomainStoredValueLengths,
    pub transition: EverQuestDynamicJepaTransitionRow,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainValidationStatus {
    Accepted,
    Rejected,
    Denied,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainRowKeys {
    pub domain_pack: String,
    pub state: String,
    pub action: String,
    pub outcome: String,
    pub transition: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainStoredValueLengths {
    pub domain_pack: u64,
    pub state: u64,
    pub action: u64,
    pub outcome: u64,
    pub transition: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDynamicJepaDomainPackRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub domain: EverQuestDomainPackHeader,
    pub state_fields: Vec<EverQuestDomainPackField>,
    pub action_fields: Vec<EverQuestDomainPackField>,
    pub outcome_fields: Vec<EverQuestDomainPackField>,
    pub entity_fields: Vec<EverQuestDomainPackField>,
    pub invariants: Vec<EverQuestDomainInvariantSpec>,
    pub planner_policy: EverQuestDomainPlannerPolicy,
    pub verification_policy: EverQuestDomainVerificationPolicy,
    pub evidence_boundary: EverQuestDomainEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainPackHeader {
    pub id: String,
    pub version: String,
    pub title: String,
    pub schema_version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainPackField {
    pub name: String,
    pub kind: String,
    pub required: bool,
    pub variants: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainInvariantSpec {
    pub id: String,
    pub severity: String,
    pub expression: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainPlannerPolicy {
    pub candidate_actions_kind: String,
    pub candidate_actions: Vec<EverQuestDomainActionKind>,
    pub guard_names: Vec<String>,
    pub surprise_threshold: f32,
    pub minimum_useful_action_prior_accuracy: f32,
    pub stretch_useful_action_prior_accuracy: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainVerificationPolicy {
    pub required_synapse_cfs: Vec<String>,
    pub required_row_prefixes: Vec<String>,
    pub compatible_contextgraph_cfs: Vec<String>,
    pub manual_fsv_required: bool,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainEvidenceBoundary {
    pub raw_chat_body_persisted: bool,
    pub compact_redacted: bool,
    pub source_refs_required: bool,
    pub supports_contextgraph_export: bool,
    pub is_training_script: bool,
    pub manual_fsv_required_for_runtime: bool,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDynamicJepaStateRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub transition_id: String,
    pub state_id: String,
    pub row_key: String,
    pub normalized_at: DateTime<Utc>,
    pub fields: EverQuestDomainStateFields,
    pub field_values: BTreeMap<String, EverQuestDomainFieldValue>,
    pub source_refs: Vec<EverQuestDomainSourceRef>,
    pub evidence_boundary: EverQuestDomainEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDynamicJepaActionRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub transition_id: String,
    pub action_id: String,
    pub row_key: String,
    pub normalized_at: DateTime<Utc>,
    pub fields: EverQuestDomainActionFields,
    pub field_values: BTreeMap<String, EverQuestDomainFieldValue>,
    pub source_refs: Vec<EverQuestDomainSourceRef>,
    pub evidence_boundary: EverQuestDomainEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDynamicJepaOutcomeRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub transition_id: String,
    pub outcome_id: String,
    pub row_key: String,
    pub normalized_at: DateTime<Utc>,
    pub fields: EverQuestDomainOutcomeFields,
    pub field_values: BTreeMap<String, EverQuestDomainFieldValue>,
    pub source_refs: Vec<EverQuestDomainSourceRef>,
    pub evidence_boundary: EverQuestDomainEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDynamicJepaTransitionRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub transition_id: String,
    pub row_key: String,
    pub normalized_at: DateTime<Utc>,
    pub domain_pack_id: String,
    pub domain_pack_version: String,
    pub prior_state_id: String,
    pub action_id: String,
    pub outcome_id: String,
    pub next_state_id: String,
    pub state_row_key: String,
    pub action_row_key: String,
    pub outcome_row_key: String,
    pub entity: EverQuestDomainEntityFields,
    pub validation_status: EverQuestDomainValidationStatus,
    pub accepted_for_planning: bool,
    pub invariant_results: Vec<EverQuestDomainInvariantResult>,
    pub rejection_reasons: Vec<String>,
    pub planner_policy: EverQuestDomainPlannerPolicy,
    pub source_refs: Vec<EverQuestDomainSourceRef>,
    pub evidence_boundary: EverQuestDomainEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainInvariantResult {
    pub invariant_id: String,
    pub passed: bool,
    pub severity: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainStateInput {
    pub zone_short_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_display_name: Option<String>,
    pub location: EverQuestDomainLocation,
    pub heading_bucket: EverQuestDomainHeadingBucket,
    pub level_bucket: EverQuestDomainLevelBucket,
    pub xp_bucket: EverQuestDomainXpBucket,
    pub target_kind: EverQuestDomainTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_level_delta: Option<i16>,
    pub con_bucket: EverQuestDomainConBucket,
    pub hp_bucket: EverQuestDomainResourceBucket,
    pub mana_bucket: EverQuestDomainResourceBucket,
    pub ui_focus_bucket: EverQuestDomainUiFocusBucket,
    pub map_visible: bool,
    pub inventory_visible: bool,
    pub foreground_process_name: String,
    pub foreground_profile_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainStateFields {
    pub zone_short_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_display_name: Option<String>,
    pub x_bucket: String,
    pub y_bucket: String,
    pub z_bucket: String,
    pub coord_bucket: String,
    pub heading_bucket: EverQuestDomainHeadingBucket,
    pub level_bucket: EverQuestDomainLevelBucket,
    pub xp_bucket: EverQuestDomainXpBucket,
    pub target_kind: EverQuestDomainTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_level_delta: Option<i16>,
    pub con_bucket: EverQuestDomainConBucket,
    pub hp_bucket: EverQuestDomainResourceBucket,
    pub mana_bucket: EverQuestDomainResourceBucket,
    pub ui_focus_bucket: EverQuestDomainUiFocusBucket,
    pub map_visible: bool,
    pub inventory_visible: bool,
    pub foreground_process_name: String,
    pub foreground_profile_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainActionInput {
    pub action_kind: EverQuestDomainActionKind,
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub move_duration_bucket: EverQuestDomainDurationBucket,
    pub turn_duration_bucket: EverQuestDomainDurationBucket,
    pub action_origin: EverQuestDomainActionOrigin,
    pub foreground_profile_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainActionFields {
    pub action_kind: EverQuestDomainActionKind,
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub move_duration_bucket: EverQuestDomainDurationBucket,
    pub turn_duration_bucket: EverQuestDomainDurationBucket,
    pub action_origin: EverQuestDomainActionOrigin,
    pub foreground_profile_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainOutcomeInput {
    pub outcome_kind: EverQuestDomainOutcomeKind,
    pub next_zone_short_name: String,
    pub next_location: EverQuestDomainLocation,
    pub target_delta: EverQuestDomainTargetDelta,
    pub con_delta: EverQuestDomainConDelta,
    pub log_event_kind: EverQuestDomainLogEventKind,
    pub damage_delta: EverQuestDomainDamageDelta,
    pub death_delta: EverQuestDomainDeathDelta,
    pub xp_delta: EverQuestDomainXpDelta,
    pub ui_mutation: EverQuestDomainUiMutation,
    pub surprise: bool,
    pub zone_entry_log: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainOutcomeFields {
    pub outcome_kind: EverQuestDomainOutcomeKind,
    pub next_zone_short_name: String,
    pub next_x_bucket: String,
    pub next_y_bucket: String,
    pub next_z_bucket: String,
    pub next_coord_bucket: String,
    pub target_delta: EverQuestDomainTargetDelta,
    pub con_delta: EverQuestDomainConDelta,
    pub log_event_kind: EverQuestDomainLogEventKind,
    pub damage_delta: EverQuestDomainDamageDelta,
    pub death_delta: EverQuestDomainDeathDelta,
    pub xp_delta: EverQuestDomainXpDelta,
    pub ui_mutation: EverQuestDomainUiMutation,
    pub surprise: bool,
    pub zone_entry_log: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainEntityInput {
    pub character_summary: String,
    pub server: String,
    pub trajectory_id: String,
    pub session_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainEntityFields {
    pub character_summary: String,
    pub server: String,
    pub trajectory_id: String,
    pub session_id: String,
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainLocation {
    pub map_x: f64,
    pub map_y: f64,
    pub map_z: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainSourceRef {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestDomainFieldValue {
    pub kind: String,
    pub value: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainHeadingBucket {
    Unknown,
    North,
    Northeast,
    East,
    Southeast,
    South,
    Southwest,
    West,
    Northwest,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainLevelBucket {
    Unknown,
    Level1,
    Level2,
    Level3To5,
    Level6Plus,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainXpBucket {
    Unknown,
    Zero,
    Low,
    Mid,
    NearLevel,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainTargetKind {
    None,
    Npc,
    Player,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainConBucket {
    Unknown,
    SafeLevelOne,
    BlueSafe,
    Even,
    Gamble,
    Dangerous,
    Tombstone,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainResourceBucket {
    Unknown,
    Empty,
    Low,
    Ready,
    Full,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainUiFocusBucket {
    World,
    ChatEmpty,
    ChatText,
    Menu,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainActionKind {
    LocProbe,
    TargetConsider,
    BoundedMove,
    CombatSpell,
    SitRest,
    InventoryRead,
    MapRead,
    DeniedUnsafe,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainDurationBucket {
    None,
    Ms1To250,
    Ms251To500,
    Ms501To1000,
    Over1000,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainActionOrigin {
    SynapseMcp,
    Operator,
    ManualSyntheticEdge,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainOutcomeKind {
    SameZone,
    ZoneChange,
    TargetConsider,
    CombatDamage,
    CombatDeath,
    XpLevel,
    UiMutation,
    Denied,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainTargetDelta {
    NoChange,
    TargetAcquired,
    TargetLost,
    TargetChanged,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainConDelta {
    NoChange,
    SafeToUnsafe,
    UnsafeToSafe,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainLogEventKind {
    None,
    Loc,
    ZoneEntry,
    Consider,
    SpellBegin,
    SpellHit,
    DamageTaken,
    Death,
    XpGain,
    LevelUp,
    UiMutation,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainDamageDelta {
    None,
    DealtLow,
    DealtHigh,
    TakenLow,
    TakenHigh,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainDeathDelta {
    None,
    PlayerDeath,
    TargetDeath,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainXpDelta {
    None,
    Gain,
    LevelUp,
    Loss,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestDomainUiMutation {
    None,
    InventoryOpened,
    InventoryClosed,
    MapVisible,
    MapHidden,
    TargetChanged,
    Unknown,
}

#[derive(Clone, Debug)]
struct NormalizedDomainTransition {
    domain_pack: EverQuestDynamicJepaDomainPackRow,
    state: EverQuestDynamicJepaStateRow,
    action: EverQuestDynamicJepaActionRow,
    outcome: EverQuestDynamicJepaOutcomeRow,
    transition: EverQuestDynamicJepaTransitionRow,
}

#[tool_router(router = everquest_domain_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Normalize one EverQuest DynamicJEPA state/action/outcome transition, persist typed CF_KV rows, and read them back"
    )]
    pub async fn everquest_domain_normalize(
        &self,
        params: Parameters<EverQuestDomainNormalizeParams>,
    ) -> Result<Json<EverQuestDomainNormalizeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TOOL,
            "tool.invocation kind=everquest_domain_normalize"
        );
        let normalized = normalize_domain_transition(params.0)?;
        let row_keys = EverQuestDomainRowKeys {
            domain_pack: domain_pack_row_key(&normalized.domain_pack.profile_id),
            state: normalized.state.row_key.clone(),
            action: normalized.action.row_key.clone(),
            outcome: normalized.outcome.row_key.clone(),
            transition: normalized.transition.row_key.clone(),
        };
        let (domain_pack, domain_pack_len) = self.persist_domain_kv_json(
            &row_keys.domain_pack,
            &normalized.domain_pack,
            "EverQuest DynamicJEPA domain-pack row",
        )?;
        let (state, state_len) = self.persist_domain_kv_json(
            &row_keys.state,
            &normalized.state,
            "EverQuest DynamicJEPA state row",
        )?;
        let (action, action_len) = self.persist_domain_kv_json(
            &row_keys.action,
            &normalized.action,
            "EverQuest DynamicJEPA action row",
        )?;
        let (outcome, outcome_len) = self.persist_domain_kv_json(
            &row_keys.outcome,
            &normalized.outcome,
            "EverQuest DynamicJEPA outcome row",
        )?;
        let (transition, transition_len) = self.persist_domain_kv_json(
            &row_keys.transition,
            &normalized.transition,
            "EverQuest DynamicJEPA transition row",
        )?;
        if domain_pack.domain.id != DOMAIN_PACK_ID
            || state.transition_id != transition.transition_id
            || action.transition_id != transition.transition_id
            || outcome.transition_id != transition.transition_id
        {
            return Err(mcp_error(
                error_codes::STORAGE_CORRUPTED,
                "EverQuest DynamicJEPA readback row linkage mismatch",
            ));
        }
        Ok(Json(EverQuestDomainNormalizeResponse {
            ok: true,
            profile_id: transition.profile_id.clone(),
            transition_id: transition.transition_id.clone(),
            validation_status: transition.validation_status.clone(),
            accepted_for_planning: transition.accepted_for_planning,
            row_keys,
            stored_value_len_bytes: EverQuestDomainStoredValueLengths {
                domain_pack: domain_pack_len,
                state: state_len,
                action: action_len,
                outcome: outcome_len,
                transition: transition_len,
            },
            transition,
        }))
    }
}

impl SynapseService {
    fn persist_domain_kv_json<T>(
        &self,
        key: &str,
        row: &T,
        label: &str,
    ) -> Result<(T, u64), ErrorData>
    where
        T: DeserializeOwned + Serialize,
    {
        let encoded = serde_json::to_vec(row).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode {label}: {error}"),
            )
        })?;
        let stored = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_error| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    format!("reflex runtime lock poisoned while writing {label}"),
                )
            })?;
            runtime
                .storage_put_kv_rows(vec![(key.as_bytes().to_vec(), encoded)])
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_WRITE_FAILED,
                        format!("write {label}: {error}"),
                    )
                })?;
            runtime
                .storage_kv_row(key.as_bytes())
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        format!("read {label} after write: {error}"),
                    )
                })?
                .ok_or_else(|| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        format!("{label} missing after write"),
                    )
                })?
        };
        let readback = serde_json::from_slice::<T>(&stored).map_err(|error| {
            mcp_error(
                error_codes::STORAGE_CORRUPTED,
                format!("decode {label} after write: {error}"),
            )
        })?;
        Ok((readback, len_to_u64(stored.len())))
    }
}

#[allow(clippy::too_many_lines)]
fn normalize_domain_transition(
    params: EverQuestDomainNormalizeParams,
) -> Result<NormalizedDomainTransition, ErrorData> {
    let transition_id = validate_id("transition_id", &params.transition_id)?;
    let profile_id = validate_everquest_profile_id(&params.profile_id)?;
    let source_refs = normalize_source_refs(params.source_refs)?;
    let state_fields = normalize_state(params.state)?;
    let action_fields = normalize_action(params.action)?;
    let outcome_fields = normalize_outcome(params.outcome)?;
    let entity = normalize_entity(&params.entity)?;
    let invariant_results = evaluate_invariants(&state_fields, &action_fields, &outcome_fields);
    let rejection_reasons = invariant_results
        .iter()
        .filter(|result| !result.passed)
        .map(|result| result.invariant_id.clone())
        .collect::<Vec<_>>();
    let validation_status = if action_fields.action_kind == EverQuestDomainActionKind::DeniedUnsafe
        || outcome_fields.outcome_kind == EverQuestDomainOutcomeKind::Denied
    {
        EverQuestDomainValidationStatus::Denied
    } else if rejection_reasons.is_empty() {
        EverQuestDomainValidationStatus::Accepted
    } else {
        EverQuestDomainValidationStatus::Rejected
    };
    let accepted_for_planning = validation_status == EverQuestDomainValidationStatus::Accepted;
    let now = Utc::now();
    let state_id = format!("{transition_id}:state");
    let action_id = format!("{transition_id}:action");
    let outcome_id = format!("{transition_id}:outcome");
    let next_state_id = format!("{transition_id}:next_state");
    let state_row_key = state_row_key(&profile_id, &transition_id);
    let action_row_key = action_row_key(&profile_id, &transition_id);
    let outcome_row_key = outcome_row_key(&profile_id, &transition_id);
    let transition_row_key = transition_row_key(&profile_id, &transition_id);
    let evidence_boundary = evidence_boundary();
    let planner_policy = planner_policy();
    Ok(NormalizedDomainTransition {
        domain_pack: domain_pack(&profile_id),
        state: EverQuestDynamicJepaStateRow {
            schema_version: SCHEMA_VERSION,
            row_kind: "everquest_dynamicjepa_state".to_owned(),
            profile_id: profile_id.clone(),
            transition_id: transition_id.clone(),
            state_id: state_id.clone(),
            row_key: state_row_key.clone(),
            normalized_at: now,
            field_values: state_field_values(&state_fields),
            fields: state_fields,
            source_refs: source_refs.clone(),
            evidence_boundary: evidence_boundary.clone(),
        },
        action: EverQuestDynamicJepaActionRow {
            schema_version: SCHEMA_VERSION,
            row_kind: "everquest_dynamicjepa_action".to_owned(),
            profile_id: profile_id.clone(),
            transition_id: transition_id.clone(),
            action_id: action_id.clone(),
            row_key: action_row_key.clone(),
            normalized_at: now,
            field_values: action_field_values(&action_fields),
            fields: action_fields,
            source_refs: source_refs.clone(),
            evidence_boundary: evidence_boundary.clone(),
        },
        outcome: EverQuestDynamicJepaOutcomeRow {
            schema_version: SCHEMA_VERSION,
            row_kind: "everquest_dynamicjepa_outcome".to_owned(),
            profile_id: profile_id.clone(),
            transition_id: transition_id.clone(),
            outcome_id: outcome_id.clone(),
            row_key: outcome_row_key.clone(),
            normalized_at: now,
            field_values: outcome_field_values(&outcome_fields),
            fields: outcome_fields,
            source_refs: source_refs.clone(),
            evidence_boundary: evidence_boundary.clone(),
        },
        transition: EverQuestDynamicJepaTransitionRow {
            schema_version: SCHEMA_VERSION,
            row_kind: "everquest_dynamicjepa_transition".to_owned(),
            profile_id,
            transition_id,
            row_key: transition_row_key,
            normalized_at: now,
            domain_pack_id: DOMAIN_PACK_ID.to_owned(),
            domain_pack_version: DOMAIN_PACK_VERSION.to_owned(),
            prior_state_id: state_id,
            action_id,
            outcome_id,
            next_state_id,
            state_row_key,
            action_row_key,
            outcome_row_key,
            entity,
            validation_status,
            accepted_for_planning,
            invariant_results,
            rejection_reasons,
            planner_policy,
            source_refs,
            evidence_boundary,
        },
    })
}

fn normalize_state(
    input: EverQuestDomainStateInput,
) -> Result<EverQuestDomainStateFields, ErrorData> {
    let zone_short_name = validate_id("state.zone_short_name", &input.zone_short_name)?;
    let zone_display_name = input
        .zone_display_name
        .map(|value| normalize_required_text("state.zone_display_name", &value))
        .transpose()?;
    validate_location("state.location", &input.location)?;
    let foreground_process_name = normalize_required_text(
        "state.foreground_process_name",
        &input.foreground_process_name,
    )?;
    let foreground_profile_id =
        normalize_required_text("state.foreground_profile_id", &input.foreground_profile_id)?;
    let x_bucket = coord_axis_bucket("x", input.location.map_x);
    let y_bucket = coord_axis_bucket("y", input.location.map_y);
    let z_bucket = coord_axis_bucket("z", input.location.map_z);
    Ok(EverQuestDomainStateFields {
        coord_bucket: format!("{zone_short_name}:{x_bucket}:{y_bucket}:{z_bucket}"),
        zone_short_name,
        zone_display_name,
        x_bucket,
        y_bucket,
        z_bucket,
        heading_bucket: input.heading_bucket,
        level_bucket: input.level_bucket,
        xp_bucket: input.xp_bucket,
        target_kind: input.target_kind,
        target_level_delta: input.target_level_delta,
        con_bucket: input.con_bucket,
        hp_bucket: input.hp_bucket,
        mana_bucket: input.mana_bucket,
        ui_focus_bucket: input.ui_focus_bucket,
        map_visible: input.map_visible,
        inventory_visible: input.inventory_visible,
        foreground_process_name,
        foreground_profile_id,
    })
}

fn normalize_action(
    input: EverQuestDomainActionInput,
) -> Result<EverQuestDomainActionFields, ErrorData> {
    let tool_name = normalize_required_text("action.tool_name", &input.tool_name)?;
    let alias = input
        .alias
        .map(|value| normalize_required_text("action.alias", &value))
        .transpose()?;
    let foreground_profile_id =
        normalize_required_text("action.foreground_profile_id", &input.foreground_profile_id)?;
    Ok(EverQuestDomainActionFields {
        action_kind: input.action_kind,
        tool_name,
        alias,
        move_duration_bucket: input.move_duration_bucket,
        turn_duration_bucket: input.turn_duration_bucket,
        action_origin: input.action_origin,
        foreground_profile_id,
    })
}

fn normalize_outcome(
    input: EverQuestDomainOutcomeInput,
) -> Result<EverQuestDomainOutcomeFields, ErrorData> {
    let next_zone_short_name =
        validate_id("outcome.next_zone_short_name", &input.next_zone_short_name)?;
    validate_location("outcome.next_location", &input.next_location)?;
    let bucket_next_x = coord_axis_bucket("x", input.next_location.map_x);
    let bucket_next_y = coord_axis_bucket("y", input.next_location.map_y);
    let bucket_next_z = coord_axis_bucket("z", input.next_location.map_z);
    Ok(EverQuestDomainOutcomeFields {
        next_coord_bucket: format!(
            "{next_zone_short_name}:{bucket_next_x}:{bucket_next_y}:{bucket_next_z}"
        ),
        outcome_kind: input.outcome_kind,
        next_zone_short_name,
        next_x_bucket: bucket_next_x,
        next_y_bucket: bucket_next_y,
        next_z_bucket: bucket_next_z,
        target_delta: input.target_delta,
        con_delta: input.con_delta,
        log_event_kind: input.log_event_kind,
        damage_delta: input.damage_delta,
        death_delta: input.death_delta,
        xp_delta: input.xp_delta,
        ui_mutation: input.ui_mutation,
        surprise: input.surprise,
        zone_entry_log: input.zone_entry_log,
    })
}

fn normalize_entity(
    input: &EverQuestDomainEntityInput,
) -> Result<EverQuestDomainEntityFields, ErrorData> {
    Ok(EverQuestDomainEntityFields {
        character_summary: normalize_required_text(
            "entity.character_summary",
            &input.character_summary,
        )?,
        server: validate_id("entity.server", &input.server)?,
        trajectory_id: validate_id("entity.trajectory_id", &input.trajectory_id)?,
        session_id: validate_id("entity.session_id", &input.session_id)?,
    })
}

fn evaluate_invariants(
    state: &EverQuestDomainStateFields,
    action: &EverQuestDomainActionFields,
    outcome: &EverQuestDomainOutcomeFields,
) -> Vec<EverQuestDomainInvariantResult> {
    vec![
        invariant_result(
            "zone_entry_log_updates_zone",
            zone_entry_log_updates_zone(state, outcome),
            "zone-entry log outcomes must have log_event_kind=zone_entry, outcome_kind=zone_change, and a changed next zone",
        ),
        invariant_result(
            "movement_requires_everquest_foreground",
            movement_requires_everquest_foreground(state, action),
            "bounded movement/combat candidates require foreground eqgame.exe with everquest.live profile",
        ),
        invariant_result(
            "con_safe_combat_policy",
            con_safe_combat_policy(state, action),
            "combat_spell candidates require safe NPC target/con/readiness buckets",
        ),
        invariant_result(
            "no_chat_social_economy_actions",
            no_chat_social_economy_actions(action),
            "chat/social/economy action labels are never planner-eligible",
        ),
        invariant_result(
            "impossible_zone_transition_rejected",
            impossible_zone_transition_rejected(state, outcome),
            "zone changes require a zone-entry log and cannot be inferred from movement alone",
        ),
    ]
}

fn invariant_result(
    invariant_id: &str,
    passed: bool,
    reason: &str,
) -> EverQuestDomainInvariantResult {
    EverQuestDomainInvariantResult {
        invariant_id: invariant_id.to_owned(),
        passed,
        severity: "fatal".to_owned(),
        reason: reason.to_owned(),
    }
}

fn zone_entry_log_updates_zone(
    state: &EverQuestDomainStateFields,
    outcome: &EverQuestDomainOutcomeFields,
) -> bool {
    !outcome.zone_entry_log
        || (outcome.log_event_kind == EverQuestDomainLogEventKind::ZoneEntry
            && outcome.outcome_kind == EverQuestDomainOutcomeKind::ZoneChange
            && outcome.next_zone_short_name != state.zone_short_name)
}

fn movement_requires_everquest_foreground(
    state: &EverQuestDomainStateFields,
    action: &EverQuestDomainActionFields,
) -> bool {
    if !matches!(
        action.action_kind,
        EverQuestDomainActionKind::BoundedMove
            | EverQuestDomainActionKind::CombatSpell
            | EverQuestDomainActionKind::TargetConsider
            | EverQuestDomainActionKind::SitRest
    ) {
        return true;
    }
    state
        .foreground_process_name
        .eq_ignore_ascii_case("eqgame.exe")
        && state.foreground_profile_id == EVERQUEST_PROFILE_ID
        && action.foreground_profile_id == EVERQUEST_PROFILE_ID
}

fn con_safe_combat_policy(
    state: &EverQuestDomainStateFields,
    action: &EverQuestDomainActionFields,
) -> bool {
    if action.action_kind != EverQuestDomainActionKind::CombatSpell {
        return true;
    }
    state.target_kind == EverQuestDomainTargetKind::Npc
        && matches!(
            state.con_bucket,
            EverQuestDomainConBucket::SafeLevelOne | EverQuestDomainConBucket::BlueSafe
        )
        && state.target_level_delta.is_some_and(|delta| delta <= 0)
        && matches!(
            state.hp_bucket,
            EverQuestDomainResourceBucket::Ready | EverQuestDomainResourceBucket::Full
        )
        && matches!(
            state.mana_bucket,
            EverQuestDomainResourceBucket::Ready | EverQuestDomainResourceBucket::Full
        )
        && matches!(
            state.ui_focus_bucket,
            EverQuestDomainUiFocusBucket::World | EverQuestDomainUiFocusBucket::ChatEmpty
        )
}

fn no_chat_social_economy_actions(action: &EverQuestDomainActionFields) -> bool {
    if action.action_kind == EverQuestDomainActionKind::DeniedUnsafe {
        return true;
    }
    !contains_unsafe_action_text(&action.tool_name)
        && !action
            .alias
            .as_deref()
            .is_some_and(contains_unsafe_action_text)
}

fn impossible_zone_transition_rejected(
    state: &EverQuestDomainStateFields,
    outcome: &EverQuestDomainOutcomeFields,
) -> bool {
    let zone_changed = outcome.next_zone_short_name != state.zone_short_name;
    !zone_changed
        || (outcome.zone_entry_log
            && outcome.log_event_kind == EverQuestDomainLogEventKind::ZoneEntry
            && outcome.outcome_kind == EverQuestDomainOutcomeKind::ZoneChange)
}

fn contains_unsafe_action_text(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [
        "chat", "say", "tell", "reply", "auction", "bazaar", "trade", "bank", "merchant", "vendor",
        "guild", "group", "raid", "pvp", "mail", "parcel",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn domain_pack(profile_id: &str) -> EverQuestDynamicJepaDomainPackRow {
    EverQuestDynamicJepaDomainPackRow {
        schema_version: SCHEMA_VERSION,
        row_kind: "everquest_dynamicjepa_domain_pack".to_owned(),
        profile_id: profile_id.to_owned(),
        domain: EverQuestDomainPackHeader {
            id: DOMAIN_PACK_ID.to_owned(),
            version: DOMAIN_PACK_VERSION.to_owned(),
            title: "EverQuest attended live state-action-outcome transitions".to_owned(),
            schema_version: SCHEMA_VERSION,
        },
        state_fields: vec![
            field("zone_short_name", "string", true, &[]),
            field("x_bucket", "categorical_bucket", true, &[]),
            field("y_bucket", "categorical_bucket", true, &[]),
            field("z_bucket", "categorical_bucket", true, &[]),
            field("heading_bucket", "categorical", true, HEADING_VARIANTS),
            field("level_bucket", "categorical", true, LEVEL_VARIANTS),
            field("xp_bucket", "categorical", true, XP_VARIANTS),
            field("target_kind", "categorical", true, TARGET_KIND_VARIANTS),
            field("target_level_delta", "i16_optional", false, &[]),
            field("con_bucket", "categorical", true, CON_VARIANTS),
            field("hp_bucket", "categorical", true, RESOURCE_VARIANTS),
            field("mana_bucket", "categorical", true, RESOURCE_VARIANTS),
            field("ui_focus_bucket", "categorical", true, UI_FOCUS_VARIANTS),
            field("map_visible", "bool", true, &[]),
            field("inventory_visible", "bool", true, &[]),
        ],
        action_fields: vec![
            field("action_kind", "categorical", true, ACTION_KIND_VARIANTS),
            field("tool_name", "string", true, &[]),
            field("alias", "string_optional", false, &[]),
            field(
                "move_duration_bucket",
                "categorical",
                true,
                DURATION_VARIANTS,
            ),
            field(
                "turn_duration_bucket",
                "categorical",
                true,
                DURATION_VARIANTS,
            ),
            field("action_origin", "categorical", true, ACTION_ORIGIN_VARIANTS),
            field("foreground_profile_id", "string", true, &[]),
        ],
        outcome_fields: vec![
            field("outcome_kind", "categorical", true, OUTCOME_KIND_VARIANTS),
            field("next_zone_short_name", "string", true, &[]),
            field("next_coord_bucket", "categorical_bucket", true, &[]),
            field("target_delta", "categorical", true, TARGET_DELTA_VARIANTS),
            field("con_delta", "categorical", true, CON_DELTA_VARIANTS),
            field("log_event_kind", "categorical", true, LOG_EVENT_VARIANTS),
            field("damage_delta", "categorical", true, DAMAGE_DELTA_VARIANTS),
            field("death_delta", "categorical", true, DEATH_DELTA_VARIANTS),
            field("xp_delta", "categorical", true, XP_DELTA_VARIANTS),
            field("ui_mutation", "categorical", true, UI_MUTATION_VARIANTS),
            field("surprise", "bool", true, &[]),
            field("zone_entry_log", "bool", true, &[]),
        ],
        entity_fields: vec![
            field("character_summary", "string", true, &[]),
            field("server", "string", true, &[]),
            field("trajectory_id", "string", true, &[]),
            field("session_id", "string", true, &[]),
        ],
        invariants: invariant_specs(),
        planner_policy: planner_policy(),
        verification_policy: verification_policy(),
        evidence_boundary: evidence_boundary(),
    }
}

fn invariant_specs() -> Vec<EverQuestDomainInvariantSpec> {
    vec![
        invariant_spec(
            "zone_entry_log_updates_zone",
            "outcome.zone_entry_log -> outcome.log_event_kind == zone_entry AND outcome.outcome_kind == zone_change AND outcome.next_zone_short_name != state.zone_short_name",
        ),
        invariant_spec(
            "movement_requires_everquest_foreground",
            "action in [bounded_move, combat_spell, target_consider, sit_rest] -> foreground eqgame.exe + everquest.live",
        ),
        invariant_spec(
            "con_safe_combat_policy",
            "combat_spell -> target_kind=npc AND con safe AND target_level_delta<=0 AND hp/mana ready AND focus not chat_text/menu",
        ),
        invariant_spec(
            "no_chat_social_economy_actions",
            "planner candidates cannot contain chat/social/economy aliases or tools",
        ),
        invariant_spec(
            "impossible_zone_transition_rejected",
            "next_zone != zone -> zone_entry_log with log_event_kind=zone_entry",
        ),
    ]
}

fn invariant_spec(id: &str, expression: &str) -> EverQuestDomainInvariantSpec {
    EverQuestDomainInvariantSpec {
        id: id.to_owned(),
        severity: "fatal".to_owned(),
        expression: expression.to_owned(),
    }
}

fn planner_policy() -> EverQuestDomainPlannerPolicy {
    EverQuestDomainPlannerPolicy {
        candidate_actions_kind: "enumerated".to_owned(),
        candidate_actions: vec![
            EverQuestDomainActionKind::LocProbe,
            EverQuestDomainActionKind::TargetConsider,
            EverQuestDomainActionKind::BoundedMove,
            EverQuestDomainActionKind::CombatSpell,
            EverQuestDomainActionKind::SitRest,
            EverQuestDomainActionKind::InventoryRead,
            EverQuestDomainActionKind::MapRead,
        ],
        guard_names: vec![
            "everquest_foreground_preflight".to_owned(),
            "everquest_chat_input_state".to_owned(),
            "everquest_planner_guard".to_owned(),
            "everquest_current_state".to_owned(),
            "everquest_route_plan".to_owned(),
            "everquest_safe_combat_guard".to_owned(),
        ],
        surprise_threshold: 0.85,
        minimum_useful_action_prior_accuracy: 0.60,
        stretch_useful_action_prior_accuracy: 0.80,
    }
}

fn verification_policy() -> EverQuestDomainVerificationPolicy {
    EverQuestDomainVerificationPolicy {
        required_synapse_cfs: vec![
            "CF_KV".to_owned(),
            "CF_ACTION_LOG".to_owned(),
            "CF_OBSERVATIONS".to_owned(),
            "CF_EVENTS".to_owned(),
        ],
        required_row_prefixes: vec![
            DOMAIN_PACK_ROW_PREFIX.to_owned(),
            STATE_ROW_PREFIX.to_owned(),
            ACTION_ROW_PREFIX.to_owned(),
            OUTCOME_ROW_PREFIX.to_owned(),
            TRANSITION_ROW_PREFIX.to_owned(),
            "everquest/current_state/v1".to_owned(),
            "everquest/outcome_event/v1".to_owned(),
            "everquest/planner_guard_decision/v1".to_owned(),
        ],
        compatible_contextgraph_cfs: vec![
            "dj_domain_packs".to_owned(),
            "dj_raw_events".to_owned(),
            "dj_normalized_states".to_owned(),
            "dj_actions".to_owned(),
            "dj_outcomes".to_owned(),
            "dj_transitions".to_owned(),
            "dj_trajectories".to_owned(),
            "dj_predictions".to_owned(),
            "dj_plan_traces".to_owned(),
            "dj_guard_decisions".to_owned(),
            "dj_surprise_events".to_owned(),
            "dj_verification_runs".to_owned(),
            "dj_audit_log".to_owned(),
        ],
        manual_fsv_required: true,
    }
}

fn evidence_boundary() -> EverQuestDomainEvidenceBoundary {
    EverQuestDomainEvidenceBoundary {
        raw_chat_body_persisted: false,
        compact_redacted: true,
        source_refs_required: true,
        supports_contextgraph_export: true,
        is_training_script: false,
        manual_fsv_required_for_runtime: true,
        note: "Rows are compact normalized game-state evidence for planning/export; runtime FSV still requires physical EQ log/UI/storage readback."
            .to_owned(),
    }
}

fn field(name: &str, kind: &str, required: bool, variants: &[&str]) -> EverQuestDomainPackField {
    EverQuestDomainPackField {
        name: name.to_owned(),
        kind: kind.to_owned(),
        required,
        variants: variants.iter().map(|value| (*value).to_owned()).collect(),
    }
}

fn state_field_values(
    fields: &EverQuestDomainStateFields,
) -> BTreeMap<String, EverQuestDomainFieldValue> {
    let mut values = BTreeMap::new();
    insert_string(&mut values, "zone_short_name", &fields.zone_short_name);
    insert_string(&mut values, "x_bucket", &fields.x_bucket);
    insert_string(&mut values, "y_bucket", &fields.y_bucket);
    insert_string(&mut values, "z_bucket", &fields.z_bucket);
    insert_string(&mut values, "coord_bucket", &fields.coord_bucket);
    insert_enum(&mut values, "heading_bucket", &fields.heading_bucket);
    insert_enum(&mut values, "level_bucket", &fields.level_bucket);
    insert_enum(&mut values, "xp_bucket", &fields.xp_bucket);
    insert_enum(&mut values, "target_kind", &fields.target_kind);
    if let Some(delta) = fields.target_level_delta {
        insert_i64(&mut values, "target_level_delta", i64::from(delta));
    }
    insert_enum(&mut values, "con_bucket", &fields.con_bucket);
    insert_enum(&mut values, "hp_bucket", &fields.hp_bucket);
    insert_enum(&mut values, "mana_bucket", &fields.mana_bucket);
    insert_enum(&mut values, "ui_focus_bucket", &fields.ui_focus_bucket);
    insert_bool(&mut values, "map_visible", fields.map_visible);
    insert_bool(&mut values, "inventory_visible", fields.inventory_visible);
    values
}

fn action_field_values(
    fields: &EverQuestDomainActionFields,
) -> BTreeMap<String, EverQuestDomainFieldValue> {
    let mut values = BTreeMap::new();
    insert_enum(&mut values, "action_kind", &fields.action_kind);
    insert_string(&mut values, "tool_name", &fields.tool_name);
    if let Some(alias) = &fields.alias {
        insert_string(&mut values, "alias", alias);
    }
    insert_enum(
        &mut values,
        "move_duration_bucket",
        &fields.move_duration_bucket,
    );
    insert_enum(
        &mut values,
        "turn_duration_bucket",
        &fields.turn_duration_bucket,
    );
    insert_enum(&mut values, "action_origin", &fields.action_origin);
    insert_string(
        &mut values,
        "foreground_profile_id",
        &fields.foreground_profile_id,
    );
    values
}

fn outcome_field_values(
    fields: &EverQuestDomainOutcomeFields,
) -> BTreeMap<String, EverQuestDomainFieldValue> {
    let mut values = BTreeMap::new();
    insert_enum(&mut values, "outcome_kind", &fields.outcome_kind);
    insert_string(
        &mut values,
        "next_zone_short_name",
        &fields.next_zone_short_name,
    );
    insert_string(&mut values, "next_coord_bucket", &fields.next_coord_bucket);
    insert_enum(&mut values, "target_delta", &fields.target_delta);
    insert_enum(&mut values, "con_delta", &fields.con_delta);
    insert_enum(&mut values, "log_event_kind", &fields.log_event_kind);
    insert_enum(&mut values, "damage_delta", &fields.damage_delta);
    insert_enum(&mut values, "death_delta", &fields.death_delta);
    insert_enum(&mut values, "xp_delta", &fields.xp_delta);
    insert_enum(&mut values, "ui_mutation", &fields.ui_mutation);
    insert_bool(&mut values, "surprise", fields.surprise);
    insert_bool(&mut values, "zone_entry_log", fields.zone_entry_log);
    values
}

fn insert_enum<T>(values: &mut BTreeMap<String, EverQuestDomainFieldValue>, name: &str, value: &T)
where
    T: Serialize,
{
    let value = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
    values.insert(
        name.to_owned(),
        EverQuestDomainFieldValue {
            kind: "categorical".to_owned(),
            value,
        },
    );
}

fn insert_string(
    values: &mut BTreeMap<String, EverQuestDomainFieldValue>,
    name: &str,
    value: &str,
) {
    values.insert(
        name.to_owned(),
        EverQuestDomainFieldValue {
            kind: "string".to_owned(),
            value: serde_json::Value::String(value.to_owned()),
        },
    );
}

fn insert_bool(values: &mut BTreeMap<String, EverQuestDomainFieldValue>, name: &str, value: bool) {
    values.insert(
        name.to_owned(),
        EverQuestDomainFieldValue {
            kind: "bool".to_owned(),
            value: serde_json::Value::Bool(value),
        },
    );
}

fn insert_i64(values: &mut BTreeMap<String, EverQuestDomainFieldValue>, name: &str, value: i64) {
    values.insert(
        name.to_owned(),
        EverQuestDomainFieldValue {
            kind: "i64".to_owned(),
            value: serde_json::Value::Number(value.into()),
        },
    );
}

#[allow(clippy::cast_possible_truncation)]
fn coord_axis_bucket(axis: &str, value: f64) -> String {
    let start = (value / 50.0).floor() as i64 * 50;
    let end = start + 49;
    format!("{axis}_{start}_{end}")
}

fn validate_location(field: &str, location: &EverQuestDomainLocation) -> Result<(), ErrorData> {
    validate_finite(&format!("{field}.map_x"), location.map_x)?;
    validate_finite(&format!("{field}.map_y"), location.map_y)?;
    validate_finite(&format!("{field}.map_z"), location.map_z)
}

fn validate_finite(field: &str, value: f64) -> Result<(), ErrorData> {
    if !value.is_finite() {
        return Err(params_error(format!("{field} must be finite")));
    }
    Ok(())
}

fn normalize_source_refs(
    source_refs: Vec<EverQuestDomainSourceRef>,
) -> Result<Vec<EverQuestDomainSourceRef>, ErrorData> {
    if source_refs.is_empty() {
        return Err(params_error(
            "source_refs must contain at least one physical SoT reference",
        ));
    }
    if source_refs.len() > MAX_SOURCE_REFS {
        return Err(params_error(format!(
            "source_refs must contain <= {MAX_SOURCE_REFS} refs"
        )));
    }
    source_refs
        .into_iter()
        .map(|source_ref| {
            Ok(EverQuestDomainSourceRef {
                kind: normalize_required_text("source_refs.kind", &source_ref.kind)?,
                row_key: source_ref
                    .row_key
                    .map(|value| normalize_required_text("source_refs.row_key", &value))
                    .transpose()?,
                path: source_ref
                    .path
                    .map(|value| normalize_required_text("source_refs.path", &value))
                    .transpose()?,
                start_offset: source_ref.start_offset,
                next_offset: source_ref.next_offset,
                content_sha256: source_ref
                    .content_sha256
                    .map(|value| normalize_required_text("source_refs.content_sha256", &value))
                    .transpose()?,
                summary: source_ref
                    .summary
                    .map(|value| normalize_required_text("source_refs.summary", &value))
                    .transpose()?,
            })
        })
        .collect()
}

fn validate_id(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = normalize_required_text(field, value)?;
    if value.len() > MAX_ID_BYTES {
        return Err(params_error(format!(
            "{field} must be <= {MAX_ID_BYTES} bytes"
        )));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':'))
    {
        return Err(params_error(format!(
            "{field} must contain only ASCII letters, digits, '.', '_', '-', or ':'"
        )));
    }
    Ok(value)
}

fn validate_everquest_profile_id(value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value != EVERQUEST_PROFILE_ID {
        return Err(params_error(format!(
            "profile_id must be {EVERQUEST_PROFILE_ID:?}; got {value:?}"
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

fn domain_pack_row_key(profile_id: &str) -> String {
    format!("{DOMAIN_PACK_ROW_PREFIX}/{profile_id}/{DOMAIN_PACK_ID}")
}

fn state_row_key(profile_id: &str, transition_id: &str) -> String {
    format!("{STATE_ROW_PREFIX}/{profile_id}/{transition_id}")
}

fn action_row_key(profile_id: &str, transition_id: &str) -> String {
    format!("{ACTION_ROW_PREFIX}/{profile_id}/{transition_id}")
}

fn outcome_row_key(profile_id: &str, transition_id: &str) -> String {
    format!("{OUTCOME_ROW_PREFIX}/{profile_id}/{transition_id}")
}

fn transition_row_key(profile_id: &str, transition_id: &str) -> String {
    format!("{TRANSITION_ROW_PREFIX}/{profile_id}/{transition_id}")
}

fn params_error(message: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, message)
}

fn default_profile_id() -> String {
    EVERQUEST_PROFILE_ID.to_owned()
}

fn len_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

const HEADING_VARIANTS: &[&str] = &[
    "unknown",
    "north",
    "northeast",
    "east",
    "southeast",
    "south",
    "southwest",
    "west",
    "northwest",
];
const LEVEL_VARIANTS: &[&str] = &["unknown", "level1", "level2", "level3_to5", "level6_plus"];
const XP_VARIANTS: &[&str] = &["unknown", "zero", "low", "mid", "near_level"];
const TARGET_KIND_VARIANTS: &[&str] = &["none", "npc", "player", "unknown"];
const CON_VARIANTS: &[&str] = &[
    "unknown",
    "safe_level_one",
    "blue_safe",
    "even",
    "gamble",
    "dangerous",
    "tombstone",
];
const RESOURCE_VARIANTS: &[&str] = &["unknown", "empty", "low", "ready", "full"];
const UI_FOCUS_VARIANTS: &[&str] = &["world", "chat_empty", "chat_text", "menu", "unknown"];
const ACTION_KIND_VARIANTS: &[&str] = &[
    "loc_probe",
    "target_consider",
    "bounded_move",
    "combat_spell",
    "sit_rest",
    "inventory_read",
    "map_read",
    "denied_unsafe",
];
const DURATION_VARIANTS: &[&str] = &[
    "none",
    "ms1_to250",
    "ms251_to500",
    "ms501_to1000",
    "over1000",
    "unknown",
];
const ACTION_ORIGIN_VARIANTS: &[&str] = &["synapse_mcp", "operator", "manual_synthetic_edge"];
const OUTCOME_KIND_VARIANTS: &[&str] = &[
    "same_zone",
    "zone_change",
    "target_consider",
    "combat_damage",
    "combat_death",
    "xp_level",
    "ui_mutation",
    "denied",
    "unknown",
];
const TARGET_DELTA_VARIANTS: &[&str] = &[
    "no_change",
    "target_acquired",
    "target_lost",
    "target_changed",
    "unknown",
];
const CON_DELTA_VARIANTS: &[&str] = &["no_change", "safe_to_unsafe", "unsafe_to_safe", "unknown"];
const LOG_EVENT_VARIANTS: &[&str] = &[
    "none",
    "loc",
    "zone_entry",
    "consider",
    "spell_begin",
    "spell_hit",
    "damage_taken",
    "death",
    "xp_gain",
    "level_up",
    "ui_mutation",
    "unknown",
];
const DAMAGE_DELTA_VARIANTS: &[&str] = &[
    "none",
    "dealt_low",
    "dealt_high",
    "taken_low",
    "taken_high",
    "unknown",
];
const DEATH_DELTA_VARIANTS: &[&str] = &["none", "player_death", "target_death", "unknown"];
const XP_DELTA_VARIANTS: &[&str] = &["none", "gain", "level_up", "loss", "unknown"];
const UI_MUTATION_VARIANTS: &[&str] = &[
    "none",
    "inventory_opened",
    "inventory_closed",
    "map_visible",
    "map_hidden",
    "target_changed",
    "unknown",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_safe_target_consider_transition() {
        let transition = normalize_domain_transition(base_params("safe-consider")).unwrap();
        assert_eq!(
            transition.transition.validation_status,
            EverQuestDomainValidationStatus::Accepted
        );
        assert!(transition.transition.accepted_for_planning);
        assert_eq!(
            transition.transition.row_key,
            "everquest/dynamicjepa_transition/v1/everquest.live/safe-consider"
        );
    }

    #[test]
    fn rejects_missing_source_refs() {
        let mut params = base_params("missing-source");
        params.source_refs.clear();
        let error = normalize_domain_transition(params).unwrap_err();
        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("code")),
            Some(&serde_json::json!(error_codes::TOOL_PARAMS_INVALID))
        );
    }

    #[test]
    fn rejects_impossible_zone_transition_without_log() {
        let mut params = base_params("impossible-zone");
        params.outcome.outcome_kind = EverQuestDomainOutcomeKind::ZoneChange;
        params.outcome.next_zone_short_name = "neriaka".to_owned();
        params.outcome.log_event_kind = EverQuestDomainLogEventKind::None;
        params.outcome.zone_entry_log = false;
        let transition = normalize_domain_transition(params).unwrap();
        assert_eq!(
            transition.transition.validation_status,
            EverQuestDomainValidationStatus::Rejected
        );
        assert!(
            transition
                .transition
                .rejection_reasons
                .contains(&"impossible_zone_transition_rejected".to_owned())
        );
    }

    #[test]
    fn marks_denied_unsafe_action_not_planner_eligible() {
        let mut params = base_params("denied-unsafe");
        params.action.action_kind = EverQuestDomainActionKind::DeniedUnsafe;
        params.action.alias = Some("open_chat".to_owned());
        params.outcome.outcome_kind = EverQuestDomainOutcomeKind::Denied;
        let transition = normalize_domain_transition(params).unwrap();
        assert_eq!(
            transition.transition.validation_status,
            EverQuestDomainValidationStatus::Denied
        );
        assert!(!transition.transition.accepted_for_planning);
    }

    #[test]
    fn rejects_combat_without_safe_con() {
        let mut params = base_params("combat-gamble");
        params.action.action_kind = EverQuestDomainActionKind::CombatSpell;
        params.action.alias = Some("hotbar4".to_owned());
        params.state.target_kind = EverQuestDomainTargetKind::Npc;
        params.state.target_level_delta = Some(1);
        params.state.con_bucket = EverQuestDomainConBucket::Gamble;
        let transition = normalize_domain_transition(params).unwrap();
        assert_eq!(
            transition.transition.validation_status,
            EverQuestDomainValidationStatus::Rejected
        );
        assert!(
            transition
                .transition
                .rejection_reasons
                .contains(&"con_safe_combat_policy".to_owned())
        );
    }

    fn base_params(transition_id: &str) -> EverQuestDomainNormalizeParams {
        EverQuestDomainNormalizeParams {
            transition_id: transition_id.to_owned(),
            profile_id: EVERQUEST_PROFILE_ID.to_owned(),
            state: EverQuestDomainStateInput {
                zone_short_name: "nektulos".to_owned(),
                zone_display_name: Some("Nektulos Forest".to_owned()),
                location: EverQuestDomainLocation {
                    map_x: 963.13,
                    map_y: -1820.25,
                    map_z: 26.59,
                },
                heading_bucket: EverQuestDomainHeadingBucket::Unknown,
                level_bucket: EverQuestDomainLevelBucket::Level1,
                xp_bucket: EverQuestDomainXpBucket::Zero,
                target_kind: EverQuestDomainTargetKind::None,
                target_level_delta: None,
                con_bucket: EverQuestDomainConBucket::Unknown,
                hp_bucket: EverQuestDomainResourceBucket::Ready,
                mana_bucket: EverQuestDomainResourceBucket::Ready,
                ui_focus_bucket: EverQuestDomainUiFocusBucket::World,
                map_visible: true,
                inventory_visible: false,
                foreground_process_name: "eqgame.exe".to_owned(),
                foreground_profile_id: EVERQUEST_PROFILE_ID.to_owned(),
            },
            action: EverQuestDomainActionInput {
                action_kind: EverQuestDomainActionKind::TargetConsider,
                tool_name: "act_keymap".to_owned(),
                alias: Some("consider".to_owned()),
                move_duration_bucket: EverQuestDomainDurationBucket::None,
                turn_duration_bucket: EverQuestDomainDurationBucket::None,
                action_origin: EverQuestDomainActionOrigin::SynapseMcp,
                foreground_profile_id: EVERQUEST_PROFILE_ID.to_owned(),
            },
            outcome: EverQuestDomainOutcomeInput {
                outcome_kind: EverQuestDomainOutcomeKind::TargetConsider,
                next_zone_short_name: "nektulos".to_owned(),
                next_location: EverQuestDomainLocation {
                    map_x: 963.13,
                    map_y: -1820.25,
                    map_z: 26.59,
                },
                target_delta: EverQuestDomainTargetDelta::TargetAcquired,
                con_delta: EverQuestDomainConDelta::Unknown,
                log_event_kind: EverQuestDomainLogEventKind::Consider,
                damage_delta: EverQuestDomainDamageDelta::None,
                death_delta: EverQuestDomainDeathDelta::None,
                xp_delta: EverQuestDomainXpDelta::None,
                ui_mutation: EverQuestDomainUiMutation::TargetChanged,
                surprise: false,
                zone_entry_log: false,
            },
            entity: EverQuestDomainEntityInput {
                character_summary: "level-1 dark elf wizard".to_owned(),
                server: "frostreaver".to_owned(),
                trajectory_id: "issue511-trajectory".to_owned(),
                session_id: "issue511-session".to_owned(),
            },
            source_refs: vec![EverQuestDomainSourceRef {
                kind: "cf_kv_current_state".to_owned(),
                row_key: Some("everquest/current_state/v1/everquest.live".to_owned()),
                path: None,
                start_offset: None,
                next_offset: None,
                content_sha256: None,
                summary: Some("synthetic supporting test source ref".to_owned()),
            }],
        }
    }
}
