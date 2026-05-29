use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use synapse_core::{
    Event, EventSource, ForbiddenRawDataKind, Observation, RealityAudit, RealityBaseline,
    RealityBaselineStatus, RealityDelta, RealityDriftItem, RealityDriftStatus,
    RealitySourceSurface, RealityTargetKind, RealityTargetRef, RedactionSummary, SourceRef,
    error_codes,
};
use synapse_perception::ObservationAssembler;
use synapse_storage::cf;

use super::{
    Json, ObserveParams, Parameters, SynapseService, current_input, observe_include, tool,
    tool_router,
};
use crate::{
    m1::{ObserveSlot, mcp_error},
    m3::permissions::{Permission, RequiredPermissions, required},
};

const REALITY_BASELINE_TOOL: &str = "reality_baseline";
const OBSERVE_DELTA_TOOL: &str = "observe_delta";
const REALITY_AUDIT_TOOL: &str = "reality_audit";
const REALITY_EVENT_KIND: &str = "reality_delta";
const DEFAULT_DEPTH: u32 = 2;
const DEFAULT_MAX_ELEMENTS: usize = 60;
const DEFAULT_MAX_DELTAS: u32 = 64;
const MAX_DEPTH: u32 = 6;
const MAX_ELEMENTS: usize = 500;
const MAX_DELTAS: u32 = 256;
const SCHEMA_VERSION: u32 = 1;
const UNPROFILED_PROFILE_KEY: &str = "unprofiled";

static NEXT_REALITY_EPOCH_SEQ: AtomicU64 = AtomicU64::new(1);
static NEXT_REALITY_AUDIT_SEQ: AtomicU64 = AtomicU64::new(1);
static NEXT_REALITY_EVENT_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealityBaselineParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch_id: Option<String>,
    #[serde(default = "default_false")]
    #[schemars(default = "default_false")]
    pub force_new_epoch: bool,
    #[serde(default = "default_include")]
    #[schemars(default = "default_include")]
    pub include: Vec<ObserveSlot>,
    #[serde(default = "default_depth")]
    #[schemars(default = "default_depth")]
    #[schemars(range(min = 1, max = 6))]
    pub depth: u32,
    #[serde(default = "default_max_elements")]
    #[schemars(default = "default_max_elements")]
    #[schemars(range(min = 1, max = 500))]
    pub max_elements: usize,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObserveDeltaParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_seq: Option<u64>,
    #[serde(default = "default_include")]
    #[schemars(default = "default_include")]
    pub include: Vec<ObserveSlot>,
    #[serde(default = "default_depth")]
    #[schemars(default = "default_depth")]
    #[schemars(range(min = 1, max = 6))]
    pub depth: u32,
    #[serde(default = "default_max_elements")]
    #[schemars(default = "default_max_elements")]
    #[schemars(range(min = 1, max = 500))]
    pub max_elements: usize,
    #[serde(default = "default_max_deltas")]
    #[schemars(default = "default_max_deltas")]
    #[schemars(range(min = 1, max = 256))]
    pub max_deltas: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealityAuditParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assumption_hash: Option<String>,
    #[serde(default = "default_include")]
    #[schemars(default = "default_include")]
    pub include: Vec<ObserveSlot>,
    #[serde(default = "default_depth")]
    #[schemars(default = "default_depth")]
    #[schemars(range(min = 1, max = 6))]
    pub depth: u32,
    #[serde(default = "default_max_elements")]
    #[schemars(default = "default_max_elements")]
    #[schemars(range(min = 1, max = 500))]
    pub max_elements: usize,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealityBaselineResponse {
    pub ok: bool,
    pub created: bool,
    pub profile_key: String,
    pub baseline: RealityBaseline,
    pub baseline_required: bool,
    pub rebase_required: bool,
    pub reason: Option<String>,
    pub head: RealityHeadRow,
    pub readback_rows: Vec<RealityRowReadback>,
    pub size_bytes: u32,
    pub size_estimate_tokens: u32,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObserveDeltaResponse {
    pub ok: bool,
    pub profile_key: Option<String>,
    pub epoch_id: Option<String>,
    pub from_seq: Option<u64>,
    pub to_seq: Option<u64>,
    pub deltas: Vec<RealityDelta>,
    pub cursor: Option<RealityCursor>,
    pub baseline_required: bool,
    pub rebase_required: bool,
    pub reason: Option<String>,
    pub readback_rows: Vec<RealityRowReadback>,
    pub published_sse_events: u32,
    pub size_bytes: u32,
    pub size_estimate_tokens: u32,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealityAuditResponse {
    pub ok: bool,
    pub profile_key: String,
    pub audit: RealityAudit,
    pub baseline_required: bool,
    pub rebase_required: bool,
    pub reason: Option<String>,
    pub row_key: String,
    pub head_key: String,
    pub readback_rows: Vec<RealityRowReadback>,
    pub size_bytes: u32,
    pub size_estimate_tokens: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealityCursor {
    pub epoch_id: String,
    pub since_seq: u64,
    pub head_seq: u64,
    pub has_more: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealityRowReadback {
    pub cf_name: String,
    pub row_key: String,
    pub value_len_bytes: u64,
    pub value_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealityHeadRow {
    pub schema_version: u32,
    pub profile_id: Option<String>,
    pub profile_key: String,
    pub epoch_id: String,
    pub baseline_seq: u64,
    pub head_seq: u64,
    pub compact_state_hash: String,
    pub baseline_row_key: String,
    pub updated_at: DateTime<Utc>,
    pub compact_state: CompactRealityState,
    pub source_refs: Vec<SourceRef>,
    pub size_bytes: u32,
    pub size_estimate_tokens: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactRealityState {
    pub schema_version: u32,
    pub profile_id: Option<String>,
    pub foreground: CompactForeground,
    pub focused: Option<CompactFocused>,
    pub hud: BTreeMap<String, CompactHudReading>,
    pub entities: Vec<CompactEntity>,
    pub events: CompactEventCursor,
    pub diagnostics: CompactDiagnostics,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactForeground {
    pub hwnd: i64,
    pub pid: u32,
    pub process_name: String,
    pub process_path_sha256: Option<String>,
    pub window_title_sha256: Option<String>,
    pub window_bounds: Value,
    pub monitor_index: u32,
    pub profile_id: Option<String>,
    pub is_fullscreen: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactFocused {
    pub element_id: String,
    pub name_sha256: Option<String>,
    pub role: String,
    pub automation_id: Option<String>,
    pub bbox: Value,
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactHudReading {
    pub parsed: Value,
    pub raw_text_sha256: Option<String>,
    pub confidence_milli: u32,
    pub stale_ms: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactEntity {
    pub entity_id: String,
    pub track_id: u64,
    pub class_label: String,
    pub bbox: Value,
    pub confidence_milli: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactEventCursor {
    pub event_count: u32,
    pub latest_non_cursor_kind: Option<String>,
    pub log_path_sha256: Option<String>,
    pub log_start_offset: Option<u64>,
    pub log_next_offset: Option<u64>,
    pub log_file_len_bytes: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactDiagnostics {
    pub a11y_status: String,
    pub capture_status: String,
    pub detection_status: String,
    pub audio_status: String,
    pub elements_truncated: bool,
    pub entities_truncated: bool,
}

#[derive(Clone, Debug)]
struct CapturedReality {
    observation: Observation,
    compact_state: CompactRealityState,
    compact_state_hash: String,
    source_refs: Vec<SourceRef>,
    size_bytes: u32,
    size_estimate_tokens: u32,
}

#[derive(Clone, Debug)]
struct ProfileSelection {
    profile_id: Option<String>,
    profile_key: String,
}

#[derive(Clone, Debug)]
struct RealityChange {
    kind: &'static str,
    path: String,
    target: RealityTargetRef,
    before: Value,
    after: Value,
}

#[tool_router(router = reality_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Capture or read a compact delta-first reality baseline and persist CF_KV reality rows"
    )]
    pub async fn reality_baseline(
        &self,
        params: Parameters<RealityBaselineParams>,
    ) -> Result<Json<RealityBaselineResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = REALITY_BASELINE_TOOL,
            force_new_epoch = params.0.force_new_epoch,
            "tool.invocation kind=reality_baseline"
        );
        self.require_m3_permissions(REALITY_BASELINE_TOOL, &required_reality_write_permissions())?;
        let response = self.capture_or_read_reality_baseline(params.0)?;
        Ok(Json(response))
    }

    #[tool(
        description = "Observe physical reality, persist ordered changes, and return deltas since a cursor"
    )]
    pub async fn observe_delta(
        &self,
        params: Parameters<ObserveDeltaParams>,
    ) -> Result<Json<ObserveDeltaResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = OBSERVE_DELTA_TOOL,
            since_epoch = ?params.0.since_epoch,
            since_seq = ?params.0.since_seq,
            "tool.invocation kind=observe_delta"
        );
        self.require_m3_permissions(OBSERVE_DELTA_TOOL, &required_reality_write_permissions())?;
        let response = self.observe_reality_delta(params.0)?;
        Ok(Json(response))
    }

    #[tool(
        description = "Audit the delta-guided reality assumption against a fresh physical read and persist drift findings"
    )]
    pub async fn reality_audit(
        &self,
        params: Parameters<RealityAuditParams>,
    ) -> Result<Json<RealityAuditResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = REALITY_AUDIT_TOOL,
            epoch_id = ?params.0.epoch_id,
            assumption_hash = ?params.0.assumption_hash,
            "tool.invocation kind=reality_audit"
        );
        self.require_m3_permissions(REALITY_AUDIT_TOOL, &required_reality_write_permissions())?;
        let response = self.audit_reality(params.0)?;
        Ok(Json(response))
    }
}

impl SynapseService {
    fn capture_or_read_reality_baseline(
        &self,
        params: RealityBaselineParams,
    ) -> Result<RealityBaselineResponse, ErrorData> {
        let existing_profile_key = params
            .profile_id
            .as_deref()
            .map(validate_key_segment)
            .transpose()?
            .unwrap_or_else(|| UNPROFILED_PROFILE_KEY.to_owned());
        if !params.force_new_epoch
            && params.epoch_id.is_none()
            && let Some(head) = self.read_reality_head(&existing_profile_key)?
        {
            let baseline = self.read_baseline_row(&head.baseline_row_key)?;
            let head_readback = self.readback_kv_row(&head_key(&head.profile_key))?;
            let baseline_readback = self.readback_kv_row(&head.baseline_row_key)?;
            return Ok(RealityBaselineResponse {
                ok: true,
                created: false,
                profile_key: head.profile_key.clone(),
                baseline,
                baseline_required: false,
                rebase_required: false,
                reason: Some("existing_baseline_reused".to_owned()),
                size_bytes: head.size_bytes,
                size_estimate_tokens: head.size_estimate_tokens,
                head,
                readback_rows: vec![baseline_readback, head_readback],
            });
        }

        let captured = self.capture_reality_observation(
            &params.include,
            params.depth,
            params.max_elements,
            "reality_baseline",
        )?;
        let profile = select_profile(params.profile_id.as_deref(), &captured.observation)?;
        let epoch_id = params
            .epoch_id
            .as_deref()
            .map(validate_key_segment)
            .transpose()?
            .unwrap_or_else(new_epoch_id);
        let baseline = RealityBaseline {
            epoch_id: epoch_id.clone(),
            baseline_seq: 0,
            generated_at: captured.observation.at,
            profile_id: profile.profile_id.clone(),
            source_surfaces: source_surfaces(&captured.source_refs),
            source_refs: captured.source_refs.clone(),
            compact_state_hash: captured.compact_state_hash.clone(),
            redaction: reality_redaction(),
            size_bytes: captured.size_bytes,
            size_estimate_tokens: captured.size_estimate_tokens,
        };
        let baseline_row_key = baseline_row_key(&profile.profile_key, &epoch_id);
        let head = RealityHeadRow {
            schema_version: SCHEMA_VERSION,
            profile_id: profile.profile_id.clone(),
            profile_key: profile.profile_key.clone(),
            epoch_id: epoch_id.clone(),
            baseline_seq: baseline.baseline_seq,
            head_seq: baseline.baseline_seq,
            compact_state_hash: captured.compact_state_hash,
            baseline_row_key: baseline_row_key.clone(),
            updated_at: Utc::now(),
            compact_state: captured.compact_state,
            source_refs: captured.source_refs,
            size_bytes: baseline.size_bytes,
            size_estimate_tokens: baseline.size_estimate_tokens,
        };
        let baseline_readback =
            self.write_kv_json_readback(&baseline_row_key, &baseline, "reality baseline row")?;
        let head_readback = self.write_kv_json_readback(
            &head_key(&profile.profile_key),
            &head,
            "reality head row",
        )?;
        Ok(RealityBaselineResponse {
            ok: true,
            created: true,
            profile_key: profile.profile_key.clone(),
            baseline,
            baseline_required: false,
            rebase_required: false,
            reason: Some("new_baseline_captured".to_owned()),
            size_bytes: head.size_bytes,
            size_estimate_tokens: head.size_estimate_tokens,
            head,
            readback_rows: vec![baseline_readback, head_readback],
        })
    }

    fn observe_reality_delta(
        &self,
        params: ObserveDeltaParams,
    ) -> Result<ObserveDeltaResponse, ErrorData> {
        let profile_key = params
            .profile_id
            .as_deref()
            .map(validate_key_segment)
            .transpose()?
            .unwrap_or_else(|| UNPROFILED_PROFILE_KEY.to_owned());
        let Some(mut head) = self.read_reality_head(&profile_key)? else {
            return Ok(ObserveDeltaResponse {
                ok: true,
                profile_key: Some(profile_key),
                epoch_id: params.since_epoch,
                from_seq: params.since_seq,
                to_seq: params.since_seq,
                deltas: Vec::new(),
                cursor: None,
                baseline_required: true,
                rebase_required: true,
                reason: Some("missing_baseline".to_owned()),
                readback_rows: Vec::new(),
                published_sse_events: 0,
                size_bytes: 0,
                size_estimate_tokens: 0,
            });
        };
        if let Some(epoch) = params.since_epoch.as_deref()
            && epoch != head.epoch_id
        {
            return Ok(stale_epoch_response(&head, epoch, params.since_seq));
        }
        let since_seq = params.since_seq.unwrap_or(head.baseline_seq);
        if since_seq > head.head_seq {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!(
                    "observe_delta since_seq {since_seq} is ahead of head_seq {}",
                    head.head_seq
                ),
            ));
        }

        let captured = self.capture_reality_observation(
            &params.include,
            params.depth,
            params.max_elements,
            "observe_delta",
        )?;
        let profile = select_profile(params.profile_id.as_deref(), &captured.observation)?;
        if profile.profile_key != head.profile_key {
            return Ok(ObserveDeltaResponse {
                ok: true,
                profile_key: Some(head.profile_key.clone()),
                epoch_id: Some(head.epoch_id.clone()),
                from_seq: Some(since_seq),
                to_seq: Some(head.head_seq),
                deltas: Vec::new(),
                cursor: Some(cursor(&head, head.head_seq, false)),
                baseline_required: false,
                rebase_required: true,
                reason: Some(format!(
                    "profile_changed: head profile {} but observed {}",
                    head.profile_key, profile.profile_key
                )),
                readback_rows: Vec::new(),
                published_sse_events: 0,
                size_bytes: 0,
                size_estimate_tokens: 0,
            });
        }

        let max_deltas = bounded_max_deltas(params.max_deltas)?;
        let mut write_readbacks = Vec::new();
        let mut published_sse_events = 0_u32;
        if head.compact_state_hash != captured.compact_state_hash {
            let changes = reality_changes(&head.compact_state, &captured.compact_state);
            if changes.len() > max_deltas {
                return Ok(ObserveDeltaResponse {
                    ok: true,
                    profile_key: Some(head.profile_key.clone()),
                    epoch_id: Some(head.epoch_id.clone()),
                    from_seq: Some(since_seq),
                    to_seq: Some(head.head_seq),
                    deltas: Vec::new(),
                    cursor: Some(cursor(&head, head.head_seq, true)),
                    baseline_required: false,
                    rebase_required: true,
                    reason: Some(format!(
                        "delta_overflow: {} changes exceed max_deltas {max_deltas}",
                        changes.len()
                    )),
                    readback_rows: Vec::new(),
                    published_sse_events: 0,
                    size_bytes: 0,
                    size_estimate_tokens: 0,
                });
            }
            let mut previous_seq = head.head_seq;
            let mut previous_hash = head.compact_state_hash.clone();
            for change in changes {
                let seq = previous_seq.saturating_add(1);
                let delta = RealityDelta {
                    epoch_id: head.epoch_id.clone(),
                    seq,
                    previous_seq,
                    at: captured.observation.at,
                    source: EventSource::Perception,
                    kind: change.kind.to_owned(),
                    path: change.path,
                    target: change.target,
                    before: change.before,
                    after: change.after,
                    confidence: 0.95,
                    expected_previous_hash: Some(previous_hash.clone()),
                    source_refs: captured.source_refs.clone(),
                    correlations: Vec::new(),
                    conflict: None,
                    redaction: reality_redaction(),
                };
                delta.validate_append_order().map_err(|error| {
                    mcp_error(
                        error_codes::TOOL_INTERNAL_ERROR,
                        format!("generated invalid reality delta: {error}"),
                    )
                })?;
                let row_key = delta_row_key(&head.profile_key, &head.epoch_id, seq);
                let readback =
                    self.write_kv_json_readback(&row_key, &delta, "reality delta row")?;
                write_readbacks.push(readback);
                self.publish_reality_delta(&head.profile_key, &row_key, &delta);
                published_sse_events = published_sse_events.saturating_add(1);
                previous_seq = seq;
                previous_hash = hash_json(&json!({
                    "previous": previous_hash,
                    "delta_row_key": row_key,
                    "after": delta.after,
                }))?;
            }
            head.head_seq = previous_seq;
            head.compact_state_hash = captured.compact_state_hash;
            head.updated_at = Utc::now();
            head.compact_state = captured.compact_state;
            head.source_refs = captured.source_refs;
            head.size_bytes = captured.size_bytes;
            head.size_estimate_tokens = captured.size_estimate_tokens;
            write_readbacks.push(self.write_kv_json_readback(
                &head_key(&head.profile_key),
                &head,
                "reality head row",
            )?);
        }

        let (mut deltas, mut read_readbacks, has_more) =
            self.read_delta_rows_after(&head.profile_key, &head.epoch_id, since_seq, max_deltas)?;
        write_readbacks.append(&mut read_readbacks);
        let to_seq = deltas.last().map_or(since_seq, |delta| delta.seq);
        let (size_bytes, size_estimate_tokens) = json_size_estimate(&deltas)?;
        if !has_more && deltas.is_empty() && since_seq == head.head_seq {
            deltas = Vec::new();
        }
        Ok(ObserveDeltaResponse {
            ok: true,
            profile_key: Some(head.profile_key.clone()),
            epoch_id: Some(head.epoch_id.clone()),
            from_seq: Some(since_seq),
            to_seq: Some(to_seq),
            deltas,
            cursor: Some(cursor(&head, to_seq, has_more)),
            baseline_required: false,
            rebase_required: false,
            reason: if to_seq == since_seq {
                Some("no_changes".to_owned())
            } else {
                Some("deltas_returned".to_owned())
            },
            readback_rows: write_readbacks,
            published_sse_events,
            size_bytes,
            size_estimate_tokens,
        })
    }

    fn audit_reality(&self, params: RealityAuditParams) -> Result<RealityAuditResponse, ErrorData> {
        let captured = self.capture_reality_observation(
            &params.include,
            params.depth,
            params.max_elements,
            "reality_audit",
        )?;
        let profile = select_profile(params.profile_id.as_deref(), &captured.observation)?;
        let head = self.read_reality_head(&profile.profile_key)?;
        if let Some(expected_epoch) = params.epoch_id.as_deref() {
            validate_key_segment(expected_epoch)?;
        }
        let compared_epoch = params
            .epoch_id
            .clone()
            .or_else(|| head.as_ref().map(|row| row.epoch_id.clone()))
            .unwrap_or_else(|| "missing-baseline".to_owned());
        let assumption_hash = params
            .assumption_hash
            .as_deref()
            .map(validate_assumption_hash)
            .transpose()?
            .or_else(|| head.as_ref().map(|row| row.compact_state_hash.clone()))
            .unwrap_or_else(|| "missing-baseline".to_owned());
        let baseline_status = if head.is_none() {
            RealityBaselineStatus::SourceUnavailable
        } else if params.epoch_id.as_deref().is_some_and(|expected| {
            Some(expected) != head.as_ref().map(|row| row.epoch_id.as_str())
        }) {
            RealityBaselineStatus::Stale
        } else {
            RealityBaselineStatus::Current
        };
        let drift_status = if baseline_status == RealityBaselineStatus::SourceUnavailable {
            RealityDriftStatus::SourceUnavailable
        } else if baseline_status == RealityBaselineStatus::Stale {
            RealityDriftStatus::RebaseRequired
        } else if assumption_hash == captured.compact_state_hash {
            RealityDriftStatus::InSync
        } else {
            RealityDriftStatus::RebaseRequired
        };
        let rebase_required = drift_status != RealityDriftStatus::InSync;
        let mut drift_items = Vec::new();
        if assumption_hash != captured.compact_state_hash {
            drift_items.push(RealityDriftItem {
                path: "/compact_state_hash".to_owned(),
                assumed: Value::String(assumption_hash.clone()),
                actual: Value::String(captured.compact_state_hash.clone()),
                severity: drift_status,
                source_refs: captured.source_refs.clone(),
            });
        }
        let audit_id = new_audit_id();
        let audit = RealityAudit {
            audit_id: audit_id.clone(),
            epoch_id: compared_epoch.clone(),
            baseline_seq: head.as_ref().map_or(0, |row| row.baseline_seq),
            compared_seq_start: head.as_ref().map_or(0, |row| row.baseline_seq),
            compared_seq_end: head.as_ref().map_or(0, |row| row.head_seq),
            ran_at: Utc::now(),
            baseline_status,
            assumption_hash,
            actual_hash: captured.compact_state_hash,
            drift_status,
            drift_items,
            physical_source_refs: captured.source_refs,
            rebase_required,
            rebase_reason: rebase_required.then(|| rebase_reason(baseline_status, drift_status)),
            follow_up_refs: Vec::new(),
        };
        let row_key = audit_row_key(&profile.profile_key, &audit_id);
        let audit_readback = self.write_kv_json_readback(&row_key, &audit, "reality audit row")?;
        let mut readback_rows = vec![audit_readback];
        if let Some(head) = &head {
            readback_rows.push(self.readback_kv_row(&head_key(&head.profile_key))?);
        }
        let (size_bytes, size_estimate_tokens) = json_size_estimate(&audit)?;
        let response_head_key = head.as_ref().map_or_else(
            || head_key(&profile.profile_key),
            |row| head_key(&row.profile_key),
        );
        Ok(RealityAuditResponse {
            ok: true,
            profile_key: profile.profile_key.clone(),
            audit,
            baseline_required: head.is_none(),
            rebase_required,
            reason: if rebase_required {
                Some("drift_or_missing_baseline".to_owned())
            } else {
                Some("in_sync".to_owned())
            },
            row_key,
            head_key: response_head_key,
            readback_rows,
            size_bytes,
            size_estimate_tokens,
        })
    }

    fn capture_reality_observation(
        &self,
        include: &[ObserveSlot],
        depth: u32,
        max_elements: usize,
        reason: &'static str,
    ) -> Result<CapturedReality, ErrorData> {
        let depth = depth.clamp(1, MAX_DEPTH);
        let max_elements = max_elements.clamp(1, MAX_ELEMENTS);
        let params = ObserveParams {
            include: include.to_vec(),
            depth: Some(depth),
            max_elements: Some(max_elements),
            since_event_seq: None,
        };
        let state = self.m1_state()?;
        let mut input = current_input(&state, depth)?;
        drop(state);

        let include = observe_include(&params);
        self.resolve_input_profile_and_hud(&mut input, include.hud);
        if include.events {
            self.populate_everquest_log_events(&mut input);
        }
        let observation = ObservationAssembler::new()
            .assemble(include, input)
            .map_err(|err| mcp_error(err.code(), err.to_string()))?;
        let mut state = self.m1_state()?;
        state.last_observed_foreground = Some(observation.foreground.clone());
        drop(state);
        self.persist_observation(&observation, reason)?;

        let compact_state = compact_state(&observation)?;
        let compact_state_hash = hash_json(&compact_state)?;
        let source_refs = source_refs_for_observation(&observation)?;
        let (size_bytes, size_estimate_tokens) = json_size_estimate(&compact_state)?;
        Ok(CapturedReality {
            observation,
            compact_state,
            compact_state_hash,
            source_refs,
            size_bytes,
            size_estimate_tokens,
        })
    }

    fn read_reality_head(&self, profile_key: &str) -> Result<Option<RealityHeadRow>, ErrorData> {
        let key = head_key(profile_key);
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while reading reality head row",
            )
        })?;
        let Some(bytes) = runtime
            .storage_kv_row(key.as_bytes())
            .map_err(|error| mcp_error(error.code(), error.to_string()))?
        else {
            return Ok(None);
        };
        decode_json_row(&bytes, "reality head row").map(Some)
    }

    fn read_baseline_row(&self, row_key: &str) -> Result<RealityBaseline, ErrorData> {
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while reading reality baseline row",
            )
        })?;
        let bytes = runtime
            .storage_kv_row(row_key.as_bytes())
            .map_err(|error| mcp_error(error.code(), error.to_string()))?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::STORAGE_READ_FAILED,
                    format!("reality baseline row missing: {row_key}"),
                )
            })?;
        decode_json_row(&bytes, "reality baseline row")
    }

    fn read_delta_rows_after(
        &self,
        profile_key: &str,
        epoch_id: &str,
        since_seq: u64,
        max_deltas: usize,
    ) -> Result<(Vec<RealityDelta>, Vec<RealityRowReadback>, bool), ErrorData> {
        let prefix = delta_prefix(profile_key, epoch_id);
        let rows = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while reading reality delta rows",
                )
            })?;
            runtime
                .storage_cf_prefix_rows(cf::CF_KV, prefix.as_bytes(), max_deltas + 1)
                .map_err(|error| mcp_error(error.code(), error.to_string()))?
        };
        let mut deltas = Vec::new();
        let mut readbacks = Vec::new();
        for (key, bytes) in rows {
            let row_key = String::from_utf8_lossy(&key).into_owned();
            let delta = decode_json_row::<RealityDelta>(&bytes, "reality delta row")?;
            if delta.seq <= since_seq {
                continue;
            }
            if deltas.len() >= max_deltas {
                return Ok((deltas, readbacks, true));
            }
            readbacks.push(RealityRowReadback {
                cf_name: cf::CF_KV.to_owned(),
                row_key,
                value_len_bytes: bytes.len() as u64,
                value_sha256: hash_bytes(&bytes),
            });
            deltas.push(delta);
        }
        Ok((deltas, readbacks, false))
    }

    fn write_kv_json_readback<T>(
        &self,
        key: &str,
        row: &T,
        label: &str,
    ) -> Result<RealityRowReadback, ErrorData>
    where
        T: Serialize,
    {
        let encoded = synapse_storage::encode_json(row).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode {label}: {error}"),
            )
        })?;
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
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
        let stored = runtime
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
                    format!("{label} missing after write: {key}"),
                )
            })?;
        Ok(RealityRowReadback {
            cf_name: cf::CF_KV.to_owned(),
            row_key: key.to_owned(),
            value_len_bytes: stored.len() as u64,
            value_sha256: hash_bytes(&stored),
        })
    }

    fn readback_kv_row(&self, key: &str) -> Result<RealityRowReadback, ErrorData> {
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while reading reality row",
            )
        })?;
        let stored = runtime
            .storage_kv_row(key.as_bytes())
            .map_err(|error| mcp_error(error.code(), error.to_string()))?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::STORAGE_READ_FAILED,
                    format!("reality row missing: {key}"),
                )
            })?;
        Ok(RealityRowReadback {
            cf_name: cf::CF_KV.to_owned(),
            row_key: key.to_owned(),
            value_len_bytes: stored.len() as u64,
            value_sha256: hash_bytes(&stored),
        })
    }

    fn publish_reality_delta(&self, profile_key: &str, row_key: &str, delta: &RealityDelta) {
        let Ok(sse_state) = self.sse_state() else {
            return;
        };
        let event = Event {
            seq: NEXT_REALITY_EVENT_SEQ.fetch_add(1, Ordering::Relaxed),
            at: delta.at,
            source: EventSource::System,
            kind: REALITY_EVENT_KIND.to_owned(),
            data: json!({
                "profile_key": profile_key,
                "row_key": row_key,
                "epoch_id": delta.epoch_id,
                "seq": delta.seq,
                "previous_seq": delta.previous_seq,
                "delta_kind": delta.kind,
                "path": delta.path,
                "target": delta.target,
                "confidence": delta.confidence,
                "redacted": true,
            }),
            correlations: Vec::new(),
        };
        let report = sse_state.event_bus().publish(event);
        tracing::debug!(
            code = "REALITY_DELTA_EVENT_PUBLISHED",
            matched = report.matched,
            queued = report.queued,
            dropped = report.dropped,
            profile_key,
            row_key,
            "reality_delta event published"
        );
    }
}

fn compact_state(observation: &Observation) -> Result<CompactRealityState, ErrorData> {
    let mut hud = BTreeMap::new();
    for (name, reading) in &observation.hud.by_name {
        hud.insert(
            name.clone(),
            CompactHudReading {
                parsed: serde_json::to_value(&reading.parsed).map_err(encode_value_error)?,
                raw_text_sha256: non_empty_hash(&reading.raw_text),
                confidence_milli: confidence_milli(reading.confidence),
                stale_ms: reading.stale_ms,
            },
        );
    }
    let mut entities = observation
        .entities
        .iter()
        .map(|entity| {
            Ok(CompactEntity {
                entity_id: entity.entity_id.clone(),
                track_id: entity.track_id,
                class_label: entity.class_label.clone(),
                bbox: serde_json::to_value(entity.bbox).map_err(encode_value_error)?,
                confidence_milli: confidence_milli(entity.confidence),
            })
        })
        .collect::<Result<Vec<_>, ErrorData>>()?;
    entities.sort_by(|left, right| {
        left.entity_id
            .cmp(&right.entity_id)
            .then_with(|| left.track_id.cmp(&right.track_id))
            .then_with(|| left.class_label.cmp(&right.class_label))
    });
    Ok(CompactRealityState {
        schema_version: SCHEMA_VERSION,
        profile_id: observation.foreground.profile_id.clone(),
        foreground: CompactForeground {
            hwnd: observation.foreground.hwnd,
            pid: observation.foreground.pid,
            process_name: observation.foreground.process_name.clone(),
            process_path_sha256: non_empty_hash(&observation.foreground.process_path),
            window_title_sha256: non_empty_hash(&observation.foreground.window_title),
            window_bounds: serde_json::to_value(observation.foreground.window_bounds)
                .map_err(encode_value_error)?,
            monitor_index: observation.foreground.monitor_index,
            profile_id: observation.foreground.profile_id.clone(),
            is_fullscreen: observation.foreground.is_fullscreen,
        },
        focused: observation
            .focused
            .as_ref()
            .map(compact_focused)
            .transpose()?,
        hud,
        entities,
        events: compact_events(observation),
        diagnostics: CompactDiagnostics {
            a11y_status: sensor_status_name(&observation.diagnostics.a11y_status),
            capture_status: sensor_status_name(&observation.diagnostics.capture_status),
            detection_status: sensor_status_name(&observation.diagnostics.detection_status),
            audio_status: sensor_status_name(&observation.diagnostics.audio_status),
            elements_truncated: observation.diagnostics.elements_truncated,
            entities_truncated: observation.diagnostics.entities_truncated,
        },
    })
}

fn compact_focused(focused: &synapse_core::FocusedElement) -> Result<CompactFocused, ErrorData> {
    Ok(CompactFocused {
        element_id: focused.element_id.to_string(),
        name_sha256: non_empty_hash(&focused.name),
        role: focused.role.clone(),
        automation_id: focused.automation_id.clone(),
        bbox: serde_json::to_value(focused.bbox).map_err(encode_value_error)?,
        enabled: focused.enabled,
    })
}

fn compact_events(observation: &Observation) -> CompactEventCursor {
    let mut latest_non_cursor_kind = None;
    let mut log_path_sha256 = None;
    let mut log_start_offset = None;
    let mut log_next_offset = None;
    let mut log_file_len_bytes = None;
    let mut event_count = 0_u32;
    for event in &observation.recent_events {
        if let Some(cursor) = event.data_excerpt.get("cursor") {
            log_start_offset = cursor
                .get("start_offset")
                .and_then(Value::as_u64)
                .or(log_start_offset);
            log_next_offset = cursor
                .get("next_offset")
                .and_then(Value::as_u64)
                .or(log_next_offset);
            log_file_len_bytes = cursor
                .get("file_len_bytes")
                .and_then(Value::as_u64)
                .or(log_file_len_bytes);
        }
        log_path_sha256 = event
            .data_excerpt
            .get("path")
            .and_then(Value::as_str)
            .and_then(non_empty_hash)
            .or(log_path_sha256);
        if !event.kind.starts_with("everquest.log_cursor") {
            event_count = event_count.saturating_add(1);
            latest_non_cursor_kind = Some(event.kind.clone());
        }
    }
    CompactEventCursor {
        event_count,
        latest_non_cursor_kind,
        log_path_sha256,
        log_start_offset,
        log_next_offset,
        log_file_len_bytes,
    }
}

fn source_refs_for_observation(observation: &Observation) -> Result<Vec<SourceRef>, ErrorData> {
    let mut refs = vec![
        SourceRef {
            surface: RealitySourceSurface::Window,
            path: format!("hwnd:0x{:x}", observation.foreground.hwnd),
            offset: None,
            hash: Some(hash_json(&json!({
                "hwnd": observation.foreground.hwnd,
                "bounds": observation.foreground.window_bounds,
                "profile_id": observation.foreground.profile_id,
            }))?),
            summary: format!(
                "foreground process={} pid={} title_hash={}",
                observation.foreground.process_name,
                observation.foreground.pid,
                non_empty_hash(&observation.foreground.window_title)
                    .unwrap_or_else(|| "empty".to_owned())
            ),
        },
        SourceRef {
            surface: RealitySourceSurface::Process,
            path: format!("pid:{}", observation.foreground.pid),
            offset: None,
            hash: non_empty_hash(&observation.foreground.process_path),
            summary: format!("process {}", observation.foreground.process_name),
        },
    ];
    if let Some(log_ref) = log_source_ref(observation)? {
        refs.push(log_ref);
    }
    if !observation.hud.by_name.is_empty() || !observation.hud.errors.is_empty() {
        refs.push(SourceRef {
            surface: RealitySourceSurface::Hud,
            path: "observation/hud".to_owned(),
            offset: None,
            hash: Some(hash_json(&observation.hud)?),
            summary: format!(
                "hud fields={} errors={}",
                observation.hud.by_name.len(),
                observation.hud.errors.len()
            ),
        });
    }
    refs.push(SourceRef {
        surface: RealitySourceSurface::Storage,
        path: "CF_OBSERVATIONS/latest".to_owned(),
        offset: None,
        hash: None,
        summary: "full observation also persisted through the observation audit path".to_owned(),
    });
    Ok(refs)
}

fn log_source_ref(observation: &Observation) -> Result<Option<SourceRef>, ErrorData> {
    for event in &observation.recent_events {
        let Some(path) = event.data_excerpt.get("path").and_then(Value::as_str) else {
            continue;
        };
        let cursor = event.data_excerpt.get("cursor");
        let next_offset = cursor
            .and_then(|value| value.get("next_offset"))
            .and_then(Value::as_u64);
        let start_offset = cursor
            .and_then(|value| value.get("start_offset"))
            .and_then(Value::as_u64);
        return Ok(Some(SourceRef {
            surface: RealitySourceSurface::GameLog,
            path: path.to_owned(),
            offset: next_offset,
            hash: Some(hash_json(&json!({
                "path": path,
                "start_offset": start_offset,
                "next_offset": next_offset,
            }))?),
            summary: format!("game log cursor next_offset={next_offset:?}"),
        }));
    }
    Ok(None)
}

fn reality_changes(
    before: &CompactRealityState,
    after: &CompactRealityState,
) -> Vec<RealityChange> {
    let mut changes = Vec::new();
    push_change(
        &mut changes,
        before.foreground.clone(),
        after.foreground.clone(),
        "foreground_changed",
        "/foreground".to_owned(),
        RealityTargetRef {
            kind: RealityTargetKind::Foreground,
            entity_id: None,
            field: None,
        },
    );
    push_change(
        &mut changes,
        before.focused.clone(),
        after.focused.clone(),
        "focus_changed",
        "/focused".to_owned(),
        RealityTargetRef {
            kind: RealityTargetKind::Focus,
            entity_id: None,
            field: None,
        },
    );
    let hud_keys = before
        .hud
        .keys()
        .chain(after.hud.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for key in hud_keys {
        push_change(
            &mut changes,
            before.hud.get(&key).cloned(),
            after.hud.get(&key).cloned(),
            "hud_field_changed",
            format!("/hud/{}", json_pointer_segment(&key)),
            RealityTargetRef {
                kind: RealityTargetKind::HudField,
                entity_id: None,
                field: Some(key),
            },
        );
    }
    push_change(
        &mut changes,
        before.entities.clone(),
        after.entities.clone(),
        "entity_set_changed",
        "/entities".to_owned(),
        RealityTargetRef {
            kind: RealityTargetKind::Entity,
            entity_id: None,
            field: None,
        },
    );
    push_change(
        &mut changes,
        before.events.clone(),
        after.events.clone(),
        "log_cursor_changed",
        "/events".to_owned(),
        RealityTargetRef {
            kind: RealityTargetKind::LogCursor,
            entity_id: None,
            field: None,
        },
    );
    push_change(
        &mut changes,
        before.diagnostics.clone(),
        after.diagnostics.clone(),
        "diagnostics_changed",
        "/diagnostics".to_owned(),
        RealityTargetRef {
            kind: RealityTargetKind::Other,
            entity_id: None,
            field: Some("diagnostics".to_owned()),
        },
    );
    if changes.is_empty() && before != after {
        push_change(
            &mut changes,
            before.clone(),
            after.clone(),
            "compact_state_changed",
            "/".to_owned(),
            RealityTargetRef {
                kind: RealityTargetKind::Other,
                entity_id: None,
                field: Some("compact_state".to_owned()),
            },
        );
    }
    changes
}

fn push_change<T>(
    changes: &mut Vec<RealityChange>,
    before: T,
    after: T,
    kind: &'static str,
    path: String,
    target: RealityTargetRef,
) where
    T: Eq + Serialize,
{
    if before == after {
        return;
    }
    changes.push(RealityChange {
        kind,
        path,
        target,
        before: serde_json::to_value(before).unwrap_or(Value::Null),
        after: serde_json::to_value(after).unwrap_or(Value::Null),
    });
}

fn stale_epoch_response(
    head: &RealityHeadRow,
    requested_epoch: &str,
    since_seq: Option<u64>,
) -> ObserveDeltaResponse {
    ObserveDeltaResponse {
        ok: true,
        profile_key: Some(head.profile_key.clone()),
        epoch_id: Some(head.epoch_id.clone()),
        from_seq: since_seq,
        to_seq: Some(head.head_seq),
        deltas: Vec::new(),
        cursor: Some(cursor(head, head.head_seq, false)),
        baseline_required: false,
        rebase_required: true,
        reason: Some(format!(
            "stale_epoch: requested {requested_epoch}, current {}",
            head.epoch_id
        )),
        readback_rows: Vec::new(),
        published_sse_events: 0,
        size_bytes: 0,
        size_estimate_tokens: 0,
    }
}

fn cursor(head: &RealityHeadRow, since_seq: u64, has_more: bool) -> RealityCursor {
    RealityCursor {
        epoch_id: head.epoch_id.clone(),
        since_seq,
        head_seq: head.head_seq,
        has_more,
    }
}

fn select_profile(
    requested: Option<&str>,
    observation: &Observation,
) -> Result<ProfileSelection, ErrorData> {
    let requested = requested.map(validate_key_segment).transpose()?;
    let observed = observation.foreground.profile_id.clone();
    if let (Some(requested), Some(observed)) = (requested.as_deref(), observed.as_deref())
        && requested != observed
    {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("profile_id {requested:?} does not match observed profile {observed:?}"),
        ));
    }
    let profile_id = requested.or(observed);
    let profile_key = profile_id
        .as_deref()
        .map_or_else(|| UNPROFILED_PROFILE_KEY.to_owned(), ToOwned::to_owned);
    Ok(ProfileSelection {
        profile_id,
        profile_key,
    })
}

fn validate_key_segment(value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error("reality key segment must not be empty"));
    }
    if value.len() > 128 {
        return Err(params_error("reality key segment must be <= 128 bytes"));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(params_error(
            "reality key segment may contain only ASCII letters, digits, '.', '_', and '-'",
        ));
    }
    Ok(value.to_owned())
}

fn validate_assumption_hash(value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error(
            "assumption_hash must not be empty when provided",
        ));
    }
    if value.len() > 128 {
        return Err(params_error("assumption_hash must be <= 128 bytes"));
    }
    if value.chars().any(char::is_control) {
        return Err(params_error(
            "assumption_hash must not contain control characters",
        ));
    }
    Ok(value.to_owned())
}

fn bounded_max_deltas(value: u32) -> Result<usize, ErrorData> {
    if value == 0 || value > MAX_DELTAS {
        return Err(params_error(format!(
            "max_deltas must be between 1 and {MAX_DELTAS}"
        )));
    }
    usize::try_from(value).map_err(|_| params_error("max_deltas does not fit usize"))
}

fn source_surfaces(source_refs: &[SourceRef]) -> Vec<RealitySourceSurface> {
    let mut surfaces = Vec::new();
    for source in source_refs {
        if !surfaces.contains(&source.surface) {
            surfaces.push(source.surface);
        }
    }
    surfaces
}

fn reality_redaction() -> RedactionSummary {
    let mut redaction = RedactionSummary::default_private();
    redaction.redacted_fields = vec![
        "foreground.process_path".to_owned(),
        "foreground.window_title".to_owned(),
        "focused.name".to_owned(),
        "hud.raw_text".to_owned(),
        "event.data_excerpt.raw_body".to_owned(),
    ];
    redaction.forbidden_raw_kinds = vec![
        ForbiddenRawDataKind::RawChatBody,
        ForbiddenRawDataKind::RawLogBody,
        ForbiddenRawDataKind::HighCardinalityPrivateData,
        ForbiddenRawDataKind::Secret,
        ForbiddenRawDataKind::Credential,
        ForbiddenRawDataKind::AccountIdentifier,
    ];
    redaction
}

fn rebase_reason(
    baseline_status: RealityBaselineStatus,
    drift_status: RealityDriftStatus,
) -> String {
    match (baseline_status, drift_status) {
        (RealityBaselineStatus::SourceUnavailable, _) => {
            "baseline row is missing; capture reality_baseline before consuming deltas".to_owned()
        }
        (RealityBaselineStatus::Stale, _) => {
            "requested epoch is stale; capture a fresh baseline".to_owned()
        }
        (_, RealityDriftStatus::RebaseRequired) => {
            "assumption hash differs from physical reality; rebase required".to_owned()
        }
        (_, RealityDriftStatus::SourceUnavailable) => {
            "physical source unavailable; rebase required after source recovery".to_owned()
        }
        _ => "drift detected; rebase required".to_owned(),
    }
}

fn sensor_status_name(status: &synapse_core::SensorStatus) -> String {
    match status {
        synapse_core::SensorStatus::Healthy => "healthy".to_owned(),
        synapse_core::SensorStatus::DegradedLatency { .. } => "degraded_latency".to_owned(),
        synapse_core::SensorStatus::DegradedSensorFailed { reason_code } => {
            format!("degraded_sensor_failed:{reason_code}")
        }
        synapse_core::SensorStatus::Disabled => "disabled".to_owned(),
        synapse_core::SensorStatus::Unavailable => "unavailable".to_owned(),
    }
}

fn confidence_milli(value: f32) -> u32 {
    if !value.is_finite() {
        return 0;
    }
    let scaled = (value.clamp(0.0, 1.0) * 1000.0).round();
    if scaled >= 1000.0 {
        1000
    } else if scaled <= 0.0 {
        0
    } else {
        scaled as u32
    }
}

fn non_empty_hash(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| hash_bytes(value.as_bytes()))
}

fn hash_json<T>(value: &T) -> Result<String, ErrorData>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec(value).map_err(encode_value_error)?;
    Ok(hash_bytes(&bytes))
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn json_size_estimate<T>(value: &T) -> Result<(u32, u32), ErrorData>
where
    T: Serialize,
{
    let len = serde_json::to_vec(value).map_err(encode_value_error)?.len();
    let size_bytes = u32::try_from(len).unwrap_or(u32::MAX);
    Ok((size_bytes, size_bytes.div_ceil(4)))
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

fn encode_value_error(error: serde_json::Error) -> ErrorData {
    mcp_error(
        error_codes::TOOL_INTERNAL_ERROR,
        format!("encode reality compact state: {error}"),
    )
}

fn params_error(message: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, message)
}

fn baseline_row_key(profile_key: &str, epoch_id: &str) -> String {
    format!("reality/baseline/v1/{profile_key}/{epoch_id}")
}

fn delta_row_key(profile_key: &str, epoch_id: &str, seq: u64) -> String {
    format!("reality/delta/v1/{profile_key}/{epoch_id}/{seq:020}")
}

fn delta_prefix(profile_key: &str, epoch_id: &str) -> String {
    format!("reality/delta/v1/{profile_key}/{epoch_id}/")
}

fn audit_row_key(profile_key: &str, audit_id: &str) -> String {
    format!("reality/audit/v1/{profile_key}/{audit_id}")
}

fn head_key(profile_key: &str) -> String {
    format!("reality/head/v1/{profile_key}")
}

fn new_epoch_id() -> String {
    format!(
        "epoch-{:020}-{:010}",
        now_ts_ns(),
        NEXT_REALITY_EPOCH_SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

fn new_audit_id() -> String {
    format!(
        "audit-{:020}-{:010}",
        now_ts_ns(),
        NEXT_REALITY_AUDIT_SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

fn now_ts_ns() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    u64::try_from(nanos).unwrap_or(u64::MAX)
}

fn json_pointer_segment(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn required_reality_write_permissions() -> RequiredPermissions {
    required([
        Permission::ReadStorage,
        Permission::WriteStorage,
        Permission::ReadEvents,
    ])
}

const fn default_false() -> bool {
    false
}

const fn default_depth() -> u32 {
    DEFAULT_DEPTH
}

const fn default_max_elements() -> usize {
    DEFAULT_MAX_ELEMENTS
}

const fn default_max_deltas() -> u32 {
    DEFAULT_MAX_DELTAS
}

const fn default_include() -> Vec<ObserveSlot> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, path::Path};

    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{m1::M1State, m2::M2ServiceConfig, m3::M3ServiceConfig, m4::M4ServiceConfig};
    use synapse_core::{ForegroundContext, HudReadings, Rect, SensorStatus};
    use synapse_perception::ObservationInput;
    use synapse_reflex::DEFAULT_MAX_SUBSCRIPTIONS_NONZERO;

    #[tokio::test]
    async fn reality_tools_persist_delta_and_publish_event() -> anyhow::Result<()> {
        let temp = TempDir::new()?;
        let service = service_with_db(temp.path())?;
        install_synthetic_input(&service, synthetic_input("Window A"))?;
        let subscription = service.sse_state()?.event_bus().subscribe(
            synapse_core::EventFilter::All,
            vec![REALITY_EVENT_KIND.to_owned()],
            false,
        )?;

        let baseline = service
            .reality_baseline(Parameters(RealityBaselineParams {
                profile_id: Some("synthetic".to_owned()),
                epoch_id: Some("test-epoch".to_owned()),
                force_new_epoch: true,
                include: Vec::new(),
                depth: DEFAULT_DEPTH,
                max_elements: DEFAULT_MAX_ELEMENTS,
            }))
            .await?;
        assert!(baseline.0.created);
        assert_eq!(baseline.0.baseline.epoch_id, "test-epoch");
        assert_eq!(baseline.0.head.head_seq, 0);

        install_synthetic_input(&service, synthetic_input("Window B"))?;
        let deltas = service
            .observe_delta(Parameters(ObserveDeltaParams {
                profile_id: Some("synthetic".to_owned()),
                since_epoch: Some("test-epoch".to_owned()),
                since_seq: Some(0),
                include: Vec::new(),
                depth: DEFAULT_DEPTH,
                max_elements: DEFAULT_MAX_ELEMENTS,
                max_deltas: DEFAULT_MAX_DELTAS,
            }))
            .await?;
        assert!(!deltas.0.deltas.is_empty());
        assert!(
            deltas
                .0
                .deltas
                .iter()
                .any(|delta| delta.kind == "foreground_changed")
        );
        assert!(deltas.0.published_sse_events > 0);
        let events = subscription.drain();
        assert!(events.iter().any(|event| event.kind == REALITY_EVENT_KIND));
        Ok(())
    }

    #[tokio::test]
    async fn observe_delta_edges_return_rebase_or_fail_closed() -> anyhow::Result<()> {
        let temp = TempDir::new()?;
        let service = service_with_db(temp.path())?;
        let missing = service
            .observe_delta(Parameters(ObserveDeltaParams {
                profile_id: Some("synthetic".to_owned()),
                since_epoch: None,
                since_seq: None,
                include: Vec::new(),
                depth: DEFAULT_DEPTH,
                max_elements: DEFAULT_MAX_ELEMENTS,
                max_deltas: DEFAULT_MAX_DELTAS,
            }))
            .await?;
        assert!(missing.0.baseline_required);

        install_synthetic_input(&service, synthetic_input("Window A"))?;
        let baseline = service
            .reality_baseline(Parameters(RealityBaselineParams {
                profile_id: Some("synthetic".to_owned()),
                epoch_id: Some("edge-epoch".to_owned()),
                force_new_epoch: true,
                include: Vec::new(),
                depth: DEFAULT_DEPTH,
                max_elements: DEFAULT_MAX_ELEMENTS,
            }))
            .await?;
        let empty = service
            .observe_delta(Parameters(ObserveDeltaParams {
                profile_id: Some("synthetic".to_owned()),
                since_epoch: Some("edge-epoch".to_owned()),
                since_seq: Some(baseline.0.head.head_seq),
                include: Vec::new(),
                depth: DEFAULT_DEPTH,
                max_elements: DEFAULT_MAX_ELEMENTS,
                max_deltas: DEFAULT_MAX_DELTAS,
            }))
            .await?;
        assert!(empty.0.deltas.is_empty());
        assert_eq!(empty.0.reason.as_deref(), Some("no_changes"));

        let stale = service
            .observe_delta(Parameters(ObserveDeltaParams {
                profile_id: Some("synthetic".to_owned()),
                since_epoch: Some("stale-epoch".to_owned()),
                since_seq: Some(0),
                include: Vec::new(),
                depth: DEFAULT_DEPTH,
                max_elements: DEFAULT_MAX_ELEMENTS,
                max_deltas: DEFAULT_MAX_DELTAS,
            }))
            .await?;
        assert!(stale.0.rebase_required);
        assert!(
            stale
                .0
                .reason
                .as_deref()
                .is_some_and(|value| value.starts_with("stale_epoch"))
        );

        let invalid = service
            .observe_delta(Parameters(ObserveDeltaParams {
                profile_id: Some("synthetic".to_owned()),
                since_epoch: Some("edge-epoch".to_owned()),
                since_seq: Some(999),
                include: Vec::new(),
                depth: DEFAULT_DEPTH,
                max_elements: DEFAULT_MAX_ELEMENTS,
                max_deltas: DEFAULT_MAX_DELTAS,
            }))
            .await;
        assert!(invalid.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn reality_audit_persists_forced_drift() -> anyhow::Result<()> {
        let temp = TempDir::new()?;
        let service = service_with_db(temp.path())?;
        install_synthetic_input(&service, synthetic_input("Window A"))?;
        service
            .reality_baseline(Parameters(RealityBaselineParams {
                profile_id: Some("synthetic".to_owned()),
                epoch_id: Some("audit-epoch".to_owned()),
                force_new_epoch: true,
                include: Vec::new(),
                depth: DEFAULT_DEPTH,
                max_elements: DEFAULT_MAX_ELEMENTS,
            }))
            .await?;
        let audit = service
            .reality_audit(Parameters(RealityAuditParams {
                profile_id: Some("synthetic".to_owned()),
                epoch_id: Some("audit-epoch".to_owned()),
                assumption_hash: Some("sha256:wrong".to_owned()),
                include: Vec::new(),
                depth: DEFAULT_DEPTH,
                max_elements: DEFAULT_MAX_ELEMENTS,
            }))
            .await?;
        assert!(audit.0.rebase_required);
        assert_eq!(
            audit.0.audit.drift_status,
            RealityDriftStatus::RebaseRequired
        );
        assert!(!audit.0.readback_rows.is_empty());
        Ok(())
    }

    fn service_with_db(path: &Path) -> anyhow::Result<SynapseService> {
        SynapseService::try_with_m2_shutdown_reason_and_m3_config(
            CancellationToken::new(),
            "test",
            CancellationToken::new(),
            &M2ServiceConfig::default(),
            M3ServiceConfig::from_cli_parts(
                Some(path.join("db")),
                None,
                false,
                "127.0.0.1:0".to_owned(),
                NonZeroUsize::new(DEFAULT_MAX_SUBSCRIPTIONS_NONZERO.get())
                    .ok_or_else(|| anyhow::anyhow!("max subscriptions must be nonzero"))?,
                false,
                true,
                None,
                false,
                None,
            ),
            M4ServiceConfig::default(),
        )
    }

    fn install_synthetic_input(
        service: &SynapseService,
        input: ObservationInput,
    ) -> anyhow::Result<()> {
        let mut state = service
            .m1_state
            .lock()
            .map_err(|_| anyhow::anyhow!("M1 state lock poisoned"))?;
        *state = M1State {
            synthetic: Some(input),
            ..M1State::default()
        };
        Ok(())
    }

    fn synthetic_input(window_title: &str) -> ObservationInput {
        let mut input = ObservationInput::new(ForegroundContext {
            hwnd: 0x1234,
            pid: 1234,
            process_name: "synthetic.exe".to_owned(),
            process_path: "C:\\Synthetic\\synthetic.exe".to_owned(),
            window_title: window_title.to_owned(),
            window_bounds: Rect {
                x: 1,
                y: 2,
                w: 800,
                h: 600,
            },
            monitor_index: 0,
            dpi_scale: 1.0,
            profile_id: None,
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
        });
        input.hud = HudReadings::default();
        input.a11y_status = SensorStatus::Healthy;
        input.capture_status = SensorStatus::Healthy;
        input.detection_status = SensorStatus::Disabled;
        input.audio_status = SensorStatus::Disabled;
        input
    }
}
