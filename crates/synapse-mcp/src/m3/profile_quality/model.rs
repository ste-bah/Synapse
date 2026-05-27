use std::collections::BTreeMap;

use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_core::ProfileId;

pub(super) const DEFAULT_MAX_AUDIT_ROWS: u32 = 5_000;
pub(super) const MAX_AUDIT_ROWS: u32 = 50_000;
pub(super) const DEFAULT_STALE_AFTER_NS: u64 = 24 * 60 * 60 * 1_000_000_000;
pub(super) const MAX_STALE_AFTER_NS: u64 = 30 * 24 * 60 * 60 * 1_000_000_000;
pub(super) const STORED_PREFIX_CHARS: usize = 512;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityRefreshParams {
    pub profile_id: ProfileId,
    #[serde(default = "default_max_audit_rows")]
    #[schemars(default = "default_max_audit_rows", range(min = 1, max = 50000))]
    pub max_audit_rows: u32,
    #[serde(default = "default_stale_after_ns")]
    #[schemars(default = "default_stale_after_ns", range(min = 1))]
    pub stale_after_ns: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityRefreshResponse {
    pub profile_id: ProfileId,
    pub cf_name: String,
    pub key_hex: String,
    pub wrote_snapshot: bool,
    pub previous_evidence_hash: Option<String>,
    pub stored_value_len_bytes: u64,
    pub stored_value_utf8_prefix: String,
    pub snapshot: ProfileQualitySnapshot,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualitySnapshot {
    pub schema_version: u32,
    pub profile_id: ProfileId,
    pub profile_label: String,
    pub profile_schema_version: u32,
    pub quality_signal: Option<String>,
    pub generated_at_ns: u64,
    pub evidence_hash: String,
    pub source: ProfileQualitySource,
    pub counts: ProfileQualityCounts,
    pub rates: ProfileQualityRates,
    pub score: ProfileQualityScore,
    pub compatibility: ProfileCompatibilitySummary,
    pub redaction: ProfileQualityRedaction,
    pub contribution: ProfileQualityContribution,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualitySource {
    pub audit_cf_name: String,
    pub profile_cf_name: String,
    pub audit_rows_scanned: u64,
    pub audit_rows_decode_failed: u64,
    pub audit_rows_stale: u64,
    pub audit_rows_future: u64,
    pub audit_rows_other_profile: u64,
    pub audit_rows_profile_relevant: u64,
    pub first_relevant_audit_id: Option<String>,
    pub last_relevant_audit_id: Option<String>,
    pub first_relevant_ts_ns: Option<u64>,
    pub last_relevant_ts_ns: Option<u64>,
    pub max_audit_rows: u32,
    pub stale_after_ns: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityCounts {
    pub started_rows: u64,
    pub ok_rows: u64,
    pub error_rows: u64,
    pub denied_rows: u64,
    pub unknown_status_rows: u64,
    pub quality_eligible_ok_rows: u64,
    pub quality_eligible_error_rows: u64,
    pub backend_unavailable_rows: u64,
    pub release_all_rows: u64,
    pub launch_ok_rows: u64,
    pub launch_error_rows: u64,
    pub tool_counts: BTreeMap<String, u64>,
    pub error_code_counts: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_field_names)]
pub struct ProfileQualityRates {
    pub success_rate: f64,
    pub error_rate: f64,
    pub denied_rate: f64,
    pub backend_unavailable_rate: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityScore {
    pub score_0_100: u32,
    pub confidence_0_1: f64,
    pub wilson_success_lower_95: f64,
    pub sample_size: u64,
    pub method: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileCompatibilitySummary {
    pub foreground_match_rows: u64,
    pub active_profile_only_rows: u64,
    pub profile_mismatch_rows: u64,
    pub target_denied_rows: u64,
    pub observed_process_names: BTreeMap<String, u64>,
    pub observed_backends: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityRedaction {
    pub local_only: bool,
    pub snapshot_redacts_process_path: bool,
    pub snapshot_redacts_window_title: bool,
    pub retained_identifiers: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityContribution {
    pub export_allowed: bool,
    pub operator_consent_required: bool,
    pub future_bundle_shape: String,
}

const fn default_max_audit_rows() -> u32 {
    DEFAULT_MAX_AUDIT_ROWS
}

const fn default_stale_after_ns() -> u64 {
    DEFAULT_STALE_AFTER_NS
}
