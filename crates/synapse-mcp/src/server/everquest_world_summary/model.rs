use chrono::{DateTime, Utc};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::{everquest_log::EVERQUEST_PROFILE_ID, everquest_state::CURRENT_STATE_ROW_KEY};

pub(super) const TOOL: &str = "everquest_world_summary";
pub(super) const SCHEMA_VERSION: u32 = 1;
pub(super) const ROW_PREFIX: &str = "everquest/world_summary/v1";
pub(super) const DEFAULT_STALE_AFTER_SECONDS: u64 = 300;
pub(super) const DEFAULT_MAX_ITEMS: usize = 5;
pub(super) const MAX_ITEMS: usize = 16;
pub(super) const MAX_TEXT_BYTES: usize = 512;
pub(super) const MAX_SOURCE_REFS: usize = 32;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryParams {
    pub summary_id: String,
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    #[serde(default = "default_state_row_key")]
    pub state_row_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_override: Option<EverQuestWorldSummaryStateOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_root_override: Option<String>,
    #[serde(default = "default_max_items")]
    pub max_exits: usize,
    #[serde(default = "default_max_items")]
    pub max_landmarks: usize,
    #[serde(default = "default_max_items")]
    pub max_transitions: usize,
    #[serde(default = "default_max_items")]
    pub max_hazards: usize,
    #[serde(default = "default_stale_after_seconds")]
    pub stale_after_seconds: u64,
    #[serde(default)]
    pub source_refs: Vec<EverQuestWorldSummarySourceRef>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryStateOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_short_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EverQuestWorldSummaryLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<DateTime<Utc>>,
    #[serde(default = "default_full_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub everquest_foreground: bool,
    #[serde(default)]
    pub hazards: Vec<EverQuestWorldSummaryHazard>,
    #[serde(default)]
    pub source_refs: Vec<EverQuestWorldSummarySourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redaction_probe_text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryResponse {
    pub ok: bool,
    pub row_key: String,
    pub stored_value_len_bytes: u64,
    pub summary: EverQuestWorldSummaryRow,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub summary_id: String,
    pub row_key: String,
    pub generated_at: DateTime<Utc>,
    pub source_state_row_key: String,
    pub source_mode: String,
    pub compact_status: String,
    pub zone: EverQuestWorldSummaryZone,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EverQuestWorldSummaryLocation>,
    pub level_progress: EverQuestWorldSummaryLevel,
    pub focus: EverQuestWorldSummaryFocus,
    pub nearest_exits: Vec<EverQuestWorldSummaryExit>,
    pub nearest_landmarks: Vec<EverQuestWorldSummaryLandmark>,
    pub recent_transitions: Vec<EverQuestWorldSummaryTransition>,
    pub safe_next_probes: Vec<String>,
    pub hazards: Vec<EverQuestWorldSummaryHazard>,
    pub active_blockers: Vec<String>,
    pub source_refs: Vec<EverQuestWorldSummarySourceRef>,
    pub compaction_recovery: EverQuestWorldSummaryRecovery,
    pub redaction: EverQuestWorldSummaryRedaction,
    pub evidence_boundary: EverQuestWorldSummaryEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryZone {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    pub confidence: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryLocation {
    pub map_x: f64,
    pub map_y: f64,
    pub map_z: f64,
    pub confidence: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryLevel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xp_percent: Option<f32>,
    pub confidence: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryFocus {
    pub is_everquest_foreground: bool,
    pub confidence: f32,
    pub process_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryExit {
    pub label: String,
    pub zone_short_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_zone_short_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_from_current: Option<f64>,
    pub confidence: f32,
    pub source_path: String,
    pub source_line_number: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryLandmark {
    pub label: String,
    pub zone_short_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_from_current: Option<f64>,
    pub confidence: f32,
    pub source_path: String,
    pub source_line_number: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryTransition {
    pub transition_kind: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<EverQuestWorldSummarySourceRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryHazard {
    pub code: String,
    pub severity: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummarySourceRef {
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

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryRecovery {
    pub latest_summary_row_key: String,
    pub durable_skill_memory_issue: String,
    pub full_tool_fsv_matrix_issue: String,
    pub world_model_context_issue: String,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryRedaction {
    pub compact_redacted: bool,
    pub raw_chat_body_persisted: bool,
    pub raw_target_names_persisted: bool,
    pub source_summaries_redacted: bool,
    pub redaction_probe_present: bool,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldSummaryEvidenceBoundary {
    pub reads_physical_state: bool,
    pub writes_summary_row_only: bool,
    pub executes_input: bool,
    pub manual_fsv_required_for_runtime: bool,
    pub is_fsv_script: bool,
}

#[derive(Clone, Debug)]
pub(super) struct NormalizedSummaryParams {
    pub(super) summary_id: String,
    pub(super) profile_id: String,
    pub(super) state_row_key: String,
    pub(super) state_override: Option<EverQuestWorldSummaryStateOverride>,
    pub(super) install_root_override: Option<String>,
    pub(super) max_exits: usize,
    pub(super) max_landmarks: usize,
    pub(super) max_transitions: usize,
    pub(super) max_hazards: usize,
    pub(super) stale_after_seconds: u64,
    pub(super) source_refs: Vec<EverQuestWorldSummarySourceRef>,
    pub(super) row_key: String,
}

fn default_profile_id() -> String {
    EVERQUEST_PROFILE_ID.to_owned()
}

fn default_state_row_key() -> String {
    CURRENT_STATE_ROW_KEY.to_owned()
}

const fn default_max_items() -> usize {
    DEFAULT_MAX_ITEMS
}

const fn default_stale_after_seconds() -> u64 {
    DEFAULT_STALE_AFTER_SECONDS
}

const fn default_full_confidence() -> f32 {
    1.0
}
