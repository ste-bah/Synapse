use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use rmcp::{
    RoleServer,
    model::ErrorCode,
    schemars::{self, JsonSchema, Schema, SchemaGenerator},
    service::RequestContext,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use synapse_core::error_codes;

use super::{
    ErrorData, Json, Parameters, SynapseService,
    command_audit::{CommandAuditInput, CommandAuditQueryParams, CommandAuditQueryResponse},
    context::mcp_session_id_from_request_context,
    tool, tool_router,
};

use crate::{
    daemon_lifecycle,
    m3::{
        audit_export::{AuditExportBundleParams, AuditExportBundleResponse, export_audit_bundle},
        demo_recording::{
            DemoRecordStartParams, DemoRecordStartResponse, DemoRecordStatusResponse,
            DemoRecordStopParams, DemoRecordStopResponse, demo_record_status_snapshot,
            start_demo_recording, stop_demo_recording,
        },
        permissions::{normalize_replay_path, replay_root},
        profile_registry::{
            AuditIntelligenceQueryParams, AuditIntelligenceQueryResponse, query_audit_intelligence,
        },
        replay::{ReplayRecordParams, ReplayRecordResponse, record_replay},
    },
};

const AUDIT_TOOL: &str = "audit";
const REPLAY_TOOL: &str = "replay";
const AUDIT_SOT: &str =
    "CF_ACTION_LOG + daemon lifecycle JSONL ledgers + profile audit storage rows";
const REPLAY_SOT: &str =
    "Synapse replay JSONL artifacts + CF_KV demo-record row + CF_TIMELINE DemoMarker rows";
const DEFAULT_LIFECYCLE_LIMIT: usize = 20;
const MAX_LIFECYCLE_LIMIT: usize = 100;
const DEFAULT_MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_LINE_BYTES: usize = 512 * 1024;
const DEFAULT_ARTIFACT_MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
const DEFAULT_ARTIFACT_MAX_RECORDS: usize = 5_000;
const MAX_ARTIFACT_RECORDS: usize = 50_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuditOperation {
    CommandQuery,
    LifecycleEvents,
    LifecycleExits,
    ProfileIntelligence,
    ExportBundle,
}

impl AuditOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CommandQuery => "command_query",
            Self::LifecycleEvents => "lifecycle_events",
            Self::LifecycleExits => "lifecycle_exits",
            Self::ProfileIntelligence => "profile_intelligence",
            Self::ExportBundle => "export_bundle",
        }
    }

    fn parse(raw: &str) -> Result<Self, ErrorData> {
        match raw {
            "command_query" => Ok(Self::CommandQuery),
            "lifecycle_events" => Ok(Self::LifecycleEvents),
            "lifecycle_exits" => Ok(Self::LifecycleExits),
            "profile_intelligence" => Ok(Self::ProfileIntelligence),
            "export_bundle" => Ok(Self::ExportBundle),
            other => Err(invalid_operation(
                AUDIT_TOOL,
                other,
                &[
                    "command_query",
                    "lifecycle_events",
                    "lifecycle_exits",
                    "profile_intelligence",
                    "export_bundle",
                ],
                AUDIT_SOT,
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplayOperation {
    Record,
    DemoStatus,
    DemoStart,
    DemoStop,
    ArtifactInspect,
}

impl ReplayOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Record => "record",
            Self::DemoStatus => "demo_status",
            Self::DemoStart => "demo_start",
            Self::DemoStop => "demo_stop",
            Self::ArtifactInspect => "artifact_inspect",
        }
    }

    fn parse(raw: &str) -> Result<Self, ErrorData> {
        match raw {
            "record" => Ok(Self::Record),
            "demo_status" => Ok(Self::DemoStatus),
            "demo_start" => Ok(Self::DemoStart),
            "demo_stop" => Ok(Self::DemoStop),
            "artifact_inspect" => Ok(Self::ArtifactInspect),
            other => Err(invalid_operation(
                REPLAY_TOOL,
                other,
                &[
                    "record",
                    "demo_status",
                    "demo_start",
                    "demo_stop",
                    "artifact_inspect",
                ],
                REPLAY_SOT,
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditParams {
    #[schemars(schema_with = "audit_operation_schema")]
    pub operation: String,
    #[serde(default)]
    pub command_query: Option<AuditCommandQueryParams>,
    #[serde(default)]
    pub lifecycle_events: Option<AuditLifecycleTailParams>,
    #[serde(default)]
    pub lifecycle_exits: Option<AuditLifecycleTailParams>,
    #[serde(default)]
    pub profile_intelligence: Option<AuditIntelligenceQueryParams>,
    #[serde(default)]
    pub export_bundle: Option<AuditExportBundleParams>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayParams {
    #[schemars(schema_with = "replay_operation_schema")]
    pub operation: String,
    #[serde(default)]
    pub record: Option<ReplayRecordParams>,
    #[serde(default)]
    pub demo_status: Option<DemoStatusParams>,
    #[serde(default)]
    pub demo_start: Option<DemoRecordStartParams>,
    #[serde(default)]
    pub demo_stop: Option<DemoRecordStopParams>,
    #[serde(default)]
    pub artifact_inspect: Option<ReplayArtifactInspectParams>,
}

fn audit_operation_schema(_: &mut SchemaGenerator) -> Schema {
    schemars::json_schema!({
        "type": "string",
        "enum": [
            "command_query",
            "lifecycle_events",
            "lifecycle_exits",
            "profile_intelligence",
            "export_bundle"
        ]
    })
}

fn replay_operation_schema(_: &mut SchemaGenerator) -> Schema {
    schemars::json_schema!({
        "type": "string",
        "enum": [
            "record",
            "demo_status",
            "demo_start",
            "demo_stop",
            "artifact_inspect"
        ]
    })
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditCommandQueryParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 250))]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 5000))]
    pub scan_limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_key_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ts_ns: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ts_ns: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_kind: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditLifecycleTailParams {
    #[serde(default = "default_lifecycle_limit")]
    #[schemars(default = "default_lifecycle_limit", range(min = 1, max = 100))]
    pub limit: usize,
    #[serde(default = "default_max_line_bytes")]
    #[schemars(default = "default_max_line_bytes", range(min = 1, max = 524288))]
    pub max_line_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_kind: Option<String>,
}

impl Default for AuditLifecycleTailParams {
    fn default() -> Self {
        Self {
            limit: default_lifecycle_limit(),
            max_line_bytes: default_max_line_bytes(),
            tool: None,
            status: None,
            event_kind: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DemoStatusParams {}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayArtifactInspectParams {
    pub path: String,
    #[serde(default = "default_artifact_max_bytes")]
    #[schemars(
        default = "default_artifact_max_bytes",
        range(min = 1, max = 134217728)
    )]
    pub max_bytes: u64,
    #[serde(default = "default_artifact_max_records")]
    #[schemars(default = "default_artifact_max_records", range(min = 0, max = 50000))]
    pub max_records: usize,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditResponse {
    pub operation: AuditOperation,
    pub source_of_truth: String,
    pub readback_source_of_truth: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_query: Option<AuditCommandQueryResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_events: Option<AuditLifecycleTailResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_exits: Option<AuditLifecycleTailResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_intelligence: Option<AuditIntelligenceQueryResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_bundle: Option<AuditExportBundleResponse>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayResponse {
    pub operation: ReplayOperation,
    pub source_of_truth: String,
    pub readback_source_of_truth: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<ReplayRecordResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub demo_status: Option<DemoRecordStatusResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub demo_start: Option<DemoRecordStartResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub demo_stop: Option<DemoRecordStopResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_readback: Option<ReplayArtifactInspectResponse>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditCommandQueryResponse {
    pub source_of_truth: String,
    pub cf_name: String,
    pub limit: usize,
    pub scan_limit: usize,
    pub scanned_rows: usize,
    pub matched_rows: usize,
    pub returned_count: usize,
    pub corrupt_row_count: usize,
    pub complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_key_hex: Option<String>,
    pub rows: Vec<AuditCommandQueryRowSummary>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditCommandQueryRowSummary {
    pub key_hex: String,
    pub value_len_bytes: u64,
    pub value_sha256: String,
    pub row_kind: String,
    pub audit_id: String,
    pub ts_ns: u64,
    pub phase: Option<String>,
    pub status: Option<String>,
    pub outcome: Option<String>,
    pub tool: String,
    pub verb: Option<String>,
    pub channel: Option<String>,
    pub error_code: Option<String>,
    pub payload_sha256: Option<String>,
    pub payload_truncated: Option<bool>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditLifecycleTailResponse {
    pub path: String,
    pub limit: usize,
    pub max_line_bytes: usize,
    pub total_lines_read: u64,
    pub matched_lines_seen: u64,
    pub returned_count: usize,
    pub rows: Vec<AuditLifecycleRowSummary>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditLifecycleRowSummary {
    pub line_no: u64,
    pub raw_len_bytes: u64,
    pub raw_sha256: String,
    pub schema_version: Option<u64>,
    pub run_id: Option<String>,
    pub pid: Option<u64>,
    pub seq: Option<u64>,
    pub event_kind: Option<String>,
    pub tool: Option<String>,
    pub status: Option<String>,
    pub cause: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub finished_at_unix_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub recorded_at_unix_ms: Option<u64>,
    pub mcp_session_id_present: bool,
    pub mcp_session_id_sha256: Option<String>,
    pub error_code: Option<String>,
    pub panic_present: bool,
    pub detail_code: Option<String>,
    pub in_flight_count: Option<u64>,
    pub last_tool: Option<String>,
    pub last_tool_status: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayArtifactInspectResponse {
    pub path: String,
    pub source_of_truth: String,
    pub exists: bool,
    pub bytes: u64,
    pub sha256: String,
    pub records_read: usize,
    pub max_records: usize,
    pub max_bytes: u64,
    pub empty: bool,
    pub lines: Vec<ReplayArtifactLineSummary>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayArtifactLineSummary {
    pub line_no: u64,
    pub len_bytes: u64,
    pub sha256: String,
    pub target: Option<String>,
    pub record_type: Option<String>,
    pub demo_id_present: bool,
    pub profile_id_present: bool,
}

#[tool_router(router = audit_replay_facade_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Public audit facade for the <=40 MCP surface. operation=command_query reads bounded CF_ACTION_LOG metadata without raw payloads; lifecycle_events/lifecycle_exits read sanitized daemon JSONL ledgers; profile_intelligence summarizes profile-linked audit rows; export_bundle writes a redacted local bundle only with explicit consent."
    )]
    pub async fn audit(
        &self,
        params: Parameters<AuditParams>,
    ) -> Result<Json<AuditResponse>, ErrorData> {
        let operation = validate_audit_params(&params.0)?;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = AUDIT_TOOL,
            operation = operation.as_str(),
            "tool.invocation kind=audit"
        );
        match operation {
            AuditOperation::CommandQuery => {
                let spec = params
                    .0
                    .command_query
                    .ok_or_else(|| missing_spec(AUDIT_TOOL, operation.as_str(), AUDIT_SOT))?;
                let response = self
                    .command_audit_query(spec.clone().into())
                    .map_err(|error| delegate_error(AUDIT_TOOL, operation.as_str(), "CF_ACTION_LOG", AUDIT_SOT, error, "tighten the audit filters or repair CF_ACTION_LOG before retrying command_query"))?;
                let sanitized = summarize_command_query(response)?;
                Ok(Json(audit_response(
                    operation,
                    format!(
                        "CF_ACTION_LOG scanned_rows={} returned_count={}",
                        sanitized.scanned_rows, sanitized.returned_count
                    ),
                    |out| out.command_query = Some(sanitized),
                )))
            }
            AuditOperation::LifecycleEvents => {
                let spec = params.0.lifecycle_events.unwrap_or_default();
                let path = lifecycle_path("tool_events_path")?;
                let response = read_lifecycle_tail(&path, &spec)?;
                Ok(Json(audit_response(
                    operation,
                    format!(
                        "{} lines_read={} returned_count={}",
                        path.display(),
                        response.total_lines_read,
                        response.returned_count
                    ),
                    |out| out.lifecycle_events = Some(response),
                )))
            }
            AuditOperation::LifecycleExits => {
                let spec = params.0.lifecycle_exits.unwrap_or_default();
                let path = lifecycle_path("exit_events_path")?;
                let response = read_lifecycle_tail(&path, &spec)?;
                Ok(Json(audit_response(
                    operation,
                    format!(
                        "{} lines_read={} returned_count={}",
                        path.display(),
                        response.total_lines_read,
                        response.returned_count
                    ),
                    |out| out.lifecycle_exits = Some(response),
                )))
            }
            AuditOperation::ProfileIntelligence => {
                let spec = params
                    .0
                    .profile_intelligence
                    .ok_or_else(|| missing_spec(AUDIT_TOOL, operation.as_str(), AUDIT_SOT))?;
                self.require_m3_permissions(
                    AUDIT_TOOL,
                    &crate::m3::profile_registry::required_permissions_audit(&spec),
                )?;
                let reflex_runtime = self.reflex_runtime().map_err(|error| {
                    delegate_error(
                        AUDIT_TOOL,
                        operation.as_str(),
                        "reflex_runtime",
                        AUDIT_SOT,
                        error,
                        "repair M3 storage initialization before retrying profile_intelligence",
                    )
                })?;
                let response = query_audit_intelligence(&reflex_runtime, &spec).map_err(|error| {
                    delegate_error(
                        AUDIT_TOOL,
                        operation.as_str(),
                        &spec.profile_id,
                        AUDIT_SOT,
                        error,
                        "inspect profile id and audit CF health before retrying profile_intelligence",
                    )
                })?;
                Ok(Json(audit_response(
                    operation,
                    format!(
                        "profile_id={} max_rows={}",
                        response.profile_id, response.max_rows
                    ),
                    |out| out.profile_intelligence = Some(response),
                )))
            }
            AuditOperation::ExportBundle => {
                let spec = params
                    .0
                    .export_bundle
                    .ok_or_else(|| missing_spec(AUDIT_TOOL, operation.as_str(), AUDIT_SOT))?;
                self.require_m3_permissions(
                    AUDIT_TOOL,
                    &crate::m3::audit_export::required_permissions_bundle(&spec),
                )?;
                let reflex_runtime = self.reflex_runtime().map_err(|error| {
                    delegate_error(
                        AUDIT_TOOL,
                        operation.as_str(),
                        "reflex_runtime",
                        AUDIT_SOT,
                        error,
                        "repair M3 storage initialization before retrying export_bundle",
                    )
                })?;
                let response = export_audit_bundle(&reflex_runtime, &spec).map_err(|error| {
                    delegate_error(
                        AUDIT_TOOL,
                        operation.as_str(),
                        &spec.profile_id,
                        AUDIT_SOT,
                        error,
                        "provide explicit enabled strict consent and inspect consent/output-file readbacks",
                    )
                })?;
                Ok(Json(audit_response(
                    operation,
                    format!(
                        "manifest={} rows={} redacted_fields={}",
                        response.manifest_path, response.rows_exported, response.redacted_fields
                    ),
                    |out| out.export_bundle = Some(response),
                )))
            }
        }
    }

    #[tool(
        description = "Public replay facade for the <=40 MCP surface. operation=record writes a replay JSONL file and immediately inspects it; demo_status/demo_start/demo_stop manage explicit UIA demo recording through CF_KV/CF_TIMELINE; artifact_inspect validates replay JSONL bytes and structure without raw payload dumps."
    )]
    pub async fn replay(
        &self,
        params: Parameters<ReplayParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ReplayResponse>, ErrorData> {
        let operation = validate_replay_params(&params.0)?;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = REPLAY_TOOL,
            operation = operation.as_str(),
            "tool.invocation kind=replay"
        );
        match operation {
            ReplayOperation::Record => {
                let spec = params
                    .0
                    .record
                    .ok_or_else(|| missing_spec(REPLAY_TOOL, operation.as_str(), REPLAY_SOT))?;
                self.require_m3_permissions(
                    REPLAY_TOOL,
                    &crate::m3::replay::required_permissions(&spec),
                )?;
                let sse_state = self.sse_state().map_err(|error| {
                    delegate_error(
                        REPLAY_TOOL,
                        operation.as_str(),
                        "sse_state",
                        REPLAY_SOT,
                        error,
                        "repair SSE state initialization before retrying replay record",
                    )
                })?;
                let response = record_replay(self.m1_state.clone(), sse_state, &spec)
                    .await
                    .map_err(|error| {
                        delegate_error(
                            REPLAY_TOOL,
                            operation.as_str(),
                            spec.path.as_deref().unwrap_or("default_replay_path"),
                            REPLAY_SOT,
                            error,
                            "fix replay target/format/path and inspect the replay artifact root",
                        )
                    })?;
                let artifact = inspect_replay_artifact(&ReplayArtifactInspectParams {
                    path: response.path.clone(),
                    max_bytes: DEFAULT_ARTIFACT_MAX_BYTES,
                    max_records: DEFAULT_ARTIFACT_MAX_RECORDS,
                })?;
                Ok(Json(replay_response(
                    operation,
                    format!(
                        "{} bytes={} records_read={}",
                        artifact.path, artifact.bytes, artifact.records_read
                    ),
                    |out| {
                        out.record = Some(response);
                        out.artifact_readback = Some(artifact);
                    },
                )))
            }
            ReplayOperation::DemoStatus => {
                let _spec = params.0.demo_status.unwrap_or_default();
                let status = demo_record_status_snapshot(&self.m3_state).map_err(|error| {
                    delegate_error(
                        REPLAY_TOOL,
                        operation.as_str(),
                        "timeline/demo-record/v1",
                        REPLAY_SOT,
                        error,
                        "inspect CF_KV timeline/demo-record/v1 and retry demo_status",
                    )
                })?;
                Ok(Json(replay_response(
                    operation,
                    format!(
                        "{} armed={} expired_active_row={}",
                        status.source_of_truth, status.armed, status.expired_active_row
                    ),
                    |out| out.demo_status = Some(status),
                )))
            }
            ReplayOperation::DemoStart => {
                let spec = params
                    .0
                    .demo_start
                    .ok_or_else(|| missing_spec(REPLAY_TOOL, operation.as_str(), REPLAY_SOT))?;
                self.require_m3_permissions(
                    REPLAY_TOOL,
                    &crate::m3::demo_recording::required_permissions_start(&spec),
                )?;
                let by_session = mcp_session_id_from_request_context(&request_context)?
                    .unwrap_or_else(|| "stdio".to_owned());
                let command_payload = json!({
                    "profile_id": &spec.profile_id,
                    "duration_ms": spec.duration_ms,
                    "path": &spec.path,
                    "label": &spec.label,
                });
                let command_before = json!({
                    "source_of_truth": REPLAY_SOT,
                    "by_session": &by_session,
                    "operation": "demo_start",
                });
                self.command_audit_intent(CommandAuditInput::mcp(
                    REPLAY_TOOL,
                    "demo_start",
                    Some(by_session.clone()),
                    Some(by_session.clone()),
                    command_payload.clone(),
                    command_before.clone(),
                    Value::Null,
                    "pending",
                ))?;
                let result = start_demo_recording(&self.m3_state, &spec, &by_session);
                match &result {
                    Ok(response) => {
                        self.command_audit_final(CommandAuditInput::mcp(
                            REPLAY_TOOL,
                            "demo_start",
                            Some(by_session.clone()),
                            Some(by_session.clone()),
                            command_payload,
                            command_before,
                            json!({
                                "source_of_truth": REPLAY_SOT,
                                "demo_id": response.demo_id,
                                "replay_path": response.replay_path,
                                "persisted": response.persisted,
                                "marker_row_written": response.marker_row_written,
                            }),
                            "ok",
                        ))?;
                    }
                    Err(error) => {
                        self.command_audit_final(
                            CommandAuditInput::mcp(
                                REPLAY_TOOL,
                                "demo_start",
                                Some(by_session.clone()),
                                Some(by_session.clone()),
                                command_payload,
                                command_before,
                                json!({
                                    "source_of_truth": REPLAY_SOT,
                                    "operation": "demo_start",
                                }),
                                "error",
                            )
                            .with_error(
                                super::command_audit::command_audit_error_from_error_data(error),
                            ),
                        )?;
                    }
                }
                let response = result.map_err(|error| {
                    delegate_error(
                        REPLAY_TOOL,
                        operation.as_str(),
                        &spec.profile_id,
                        REPLAY_SOT,
                        error,
                        "fix demo profile/duration/path and inspect CF_KV/CF_TIMELINE rows",
                    )
                })?;
                Ok(Json(replay_response(
                    operation,
                    format!(
                        "demo_id={} persisted={} marker_row_written={}",
                        response.demo_id, response.persisted, response.marker_row_written
                    ),
                    |out| out.demo_start = Some(response),
                )))
            }
            ReplayOperation::DemoStop => {
                let spec = params
                    .0
                    .demo_stop
                    .ok_or_else(|| missing_spec(REPLAY_TOOL, operation.as_str(), REPLAY_SOT))?;
                self.require_m3_permissions(
                    REPLAY_TOOL,
                    &crate::m3::demo_recording::required_permissions_stop(&spec),
                )?;
                let by_session = mcp_session_id_from_request_context(&request_context)?
                    .unwrap_or_else(|| "stdio".to_owned());
                let command_payload = json!({
                    "demo_id": &spec.demo_id,
                });
                let command_before = json!({
                    "source_of_truth": REPLAY_SOT,
                    "by_session": &by_session,
                    "operation": "demo_stop",
                });
                self.command_audit_intent(CommandAuditInput::mcp(
                    REPLAY_TOOL,
                    "demo_stop",
                    Some(by_session.clone()),
                    Some(by_session.clone()),
                    command_payload.clone(),
                    command_before.clone(),
                    Value::Null,
                    "pending",
                ))?;
                let result = stop_demo_recording(&self.m3_state, &spec, &by_session);
                match &result {
                    Ok(response) => {
                        self.command_audit_final(CommandAuditInput::mcp(
                            REPLAY_TOOL,
                            "demo_stop",
                            Some(by_session.clone()),
                            Some(by_session.clone()),
                            command_payload,
                            command_before,
                            json!({
                                "source_of_truth": REPLAY_SOT,
                                "demo_id": response.demo_id,
                                "replay_path": response.replay_path,
                                "records_written": response.records_written,
                                "bytes": response.bytes,
                            }),
                            "ok",
                        ))?;
                    }
                    Err(error) => {
                        self.command_audit_final(
                            CommandAuditInput::mcp(
                                REPLAY_TOOL,
                                "demo_stop",
                                Some(by_session.clone()),
                                Some(by_session.clone()),
                                command_payload,
                                command_before,
                                json!({
                                    "source_of_truth": REPLAY_SOT,
                                    "operation": "demo_stop",
                                }),
                                "error",
                            )
                            .with_error(
                                super::command_audit::command_audit_error_from_error_data(error),
                            ),
                        )?;
                    }
                }
                let response = result.map_err(|error| {
                    delegate_error(
                        REPLAY_TOOL,
                        operation.as_str(),
                        spec.demo_id.as_deref().unwrap_or("active_demo_recording"),
                        REPLAY_SOT,
                        error,
                        "inspect active demo status and CF_TIMELINE DemoMarker rows before retrying demo_stop",
                    )
                })?;
                let artifact = inspect_replay_artifact(&ReplayArtifactInspectParams {
                    path: response.replay_path.clone(),
                    max_bytes: DEFAULT_ARTIFACT_MAX_BYTES,
                    max_records: DEFAULT_ARTIFACT_MAX_RECORDS,
                })?;
                Ok(Json(replay_response(
                    operation,
                    format!(
                        "{} bytes={} records_read={}",
                        artifact.path, artifact.bytes, artifact.records_read
                    ),
                    |out| {
                        out.demo_stop = Some(response);
                        out.artifact_readback = Some(artifact);
                    },
                )))
            }
            ReplayOperation::ArtifactInspect => {
                let spec = params
                    .0
                    .artifact_inspect
                    .ok_or_else(|| missing_spec(REPLAY_TOOL, operation.as_str(), REPLAY_SOT))?;
                let response = inspect_replay_artifact(&spec)?;
                Ok(Json(replay_response(
                    operation,
                    format!(
                        "{} bytes={} records_read={}",
                        response.path, response.bytes, response.records_read
                    ),
                    |out| out.artifact_readback = Some(response),
                )))
            }
        }
    }
}

impl From<AuditCommandQueryParams> for CommandAuditQueryParams {
    fn from(value: AuditCommandQueryParams) -> Self {
        Self {
            limit: value.limit,
            scan_limit: value.scan_limit,
            start_key_hex: value.start_key_hex,
            start_ts_ns: value.start_ts_ns,
            end_ts_ns: value.end_ts_ns,
            session_id: value.session_id,
            tool: value.tool,
            status: value.status,
            error_code: value.error_code,
            row_kind: value.row_kind,
        }
    }
}

fn audit_response(
    operation: AuditOperation,
    readback: String,
    fill: impl FnOnce(&mut AuditResponse),
) -> AuditResponse {
    let mut response = AuditResponse {
        operation,
        source_of_truth: AUDIT_SOT.to_owned(),
        readback_source_of_truth: readback,
        command_query: None,
        lifecycle_events: None,
        lifecycle_exits: None,
        profile_intelligence: None,
        export_bundle: None,
    };
    fill(&mut response);
    response
}

fn replay_response(
    operation: ReplayOperation,
    readback: String,
    fill: impl FnOnce(&mut ReplayResponse),
) -> ReplayResponse {
    let mut response = ReplayResponse {
        operation,
        source_of_truth: REPLAY_SOT.to_owned(),
        readback_source_of_truth: readback,
        record: None,
        demo_status: None,
        demo_start: None,
        demo_stop: None,
        artifact_readback: None,
    };
    fill(&mut response);
    response
}

fn summarize_command_query(
    response: CommandAuditQueryResponse,
) -> Result<AuditCommandQueryResponse, ErrorData> {
    if response.partial || response.next_start_key_hex.is_some() {
        return Err(ErrorData::new(
            ErrorCode(-32099),
            "audit operation=command_query refused to return a partial CF_ACTION_LOG scan"
                .to_owned(),
            Some(json!({
                "code": error_codes::STORAGE_READ_FAILED,
                "tool": AUDIT_TOOL,
                "operation": "command_query",
                "source_id": response.cf_name,
                "source_of_truth": AUDIT_SOT,
                "scanned_rows": response.scanned_rows,
                "matched_rows": response.matched_rows,
                "returned_count": response.returned_count,
                "limit": response.limit,
                "scan_limit": response.scan_limit,
                "next_start_key_hex": response.next_start_key_hex,
                "remediation": "narrow the time/tool/status filters or rerun with a cursor so the requested page is complete",
            })),
        ));
    }
    if response.corrupt_row_count > 0 {
        return Err(ErrorData::new(
            ErrorCode(-32099),
            "audit operation=command_query found corrupt CF_ACTION_LOG rows".to_owned(),
            Some(json!({
                "code": error_codes::STORAGE_READ_FAILED,
                "tool": AUDIT_TOOL,
                "operation": "command_query",
                "source_id": response.cf_name,
                "source_of_truth": AUDIT_SOT,
                "corrupt_row_count": response.corrupt_row_count,
                "remediation": "inspect and repair the corrupt CF_ACTION_LOG rows before trusting audit output",
            })),
        ));
    }
    Ok(AuditCommandQueryResponse {
        source_of_truth: response.source_of_truth.to_owned(),
        cf_name: response.cf_name.to_owned(),
        limit: response.limit,
        scan_limit: response.scan_limit,
        scanned_rows: response.scanned_rows,
        matched_rows: response.matched_rows,
        returned_count: response.returned_count,
        corrupt_row_count: response.corrupt_row_count,
        complete: response.exhausted,
        start_key_hex: response.start_key_hex,
        rows: response
            .rows
            .into_iter()
            .map(|row| AuditCommandQueryRowSummary {
                key_hex: row.key_hex,
                value_len_bytes: row.value_len_bytes,
                value_sha256: row.value_sha256,
                row_kind: row.row_kind,
                audit_id: row.audit_id,
                ts_ns: row.ts_ns,
                phase: row.phase,
                status: row.status,
                outcome: row.outcome,
                tool: row.tool,
                verb: row.verb,
                channel: row.channel,
                error_code: row.error_code,
                payload_sha256: row.payload_sha256,
                payload_truncated: row.payload_truncated,
            })
            .collect(),
    })
}

fn lifecycle_path(key: &str) -> Result<PathBuf, ErrorData> {
    let diagnostic = daemon_lifecycle::diagnostic_value();
    let path = diagnostic
        .get("paths")
        .and_then(|paths| paths.get(key))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ErrorData::new(
                ErrorCode(-32099),
                format!("audit operation needs daemon lifecycle {key}, but it is unavailable"),
                Some(json!({
                    "code": error_codes::TOOL_INTERNAL_ERROR,
                    "tool": AUDIT_TOOL,
                    "operation": key,
                    "source_id": "daemon_lifecycle::diagnostic_value",
                    "source_of_truth": AUDIT_SOT,
                    "remediation": "repair daemon lifecycle configuration and retry audit lifecycle query",
                })),
            )
        })?;
    Ok(PathBuf::from(path))
}

fn read_lifecycle_tail(
    path: &Path,
    params: &AuditLifecycleTailParams,
) -> Result<AuditLifecycleTailResponse, ErrorData> {
    validate_lifecycle_params(params)?;
    let file = File::open(path).map_err(|error| {
        io_error(
            AUDIT_TOOL,
            "lifecycle_tail",
            &path.display().to_string(),
            AUDIT_SOT,
            error,
            "inspect daemon lifecycle paths and file permissions",
        )
    })?;
    let reader = BufReader::new(file);
    let mut total_lines_read = 0_u64;
    let mut matched_lines_seen = 0_u64;
    let mut rows = VecDeque::with_capacity(params.limit);
    for line in reader.split(b'\n') {
        let mut bytes = line.map_err(|error| {
            io_error(
                AUDIT_TOOL,
                "lifecycle_tail",
                &path.display().to_string(),
                AUDIT_SOT,
                error,
                "inspect daemon lifecycle ledger readability",
            )
        })?;
        total_lines_read = total_lines_read.saturating_add(1);
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        if bytes.is_empty() {
            return Err(lifecycle_corrupt_error(
                path,
                total_lines_read,
                "empty JSONL line",
            ));
        }
        if bytes.len() > params.max_line_bytes {
            return Err(lifecycle_corrupt_error(
                path,
                total_lines_read,
                "line exceeded max_line_bytes",
            ));
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
            lifecycle_corrupt_error(
                path,
                total_lines_read,
                format!("JSON decode failed: {error}"),
            )
        })?;
        if lifecycle_matches(&value, params) {
            matched_lines_seen = matched_lines_seen.saturating_add(1);
            if rows.len() == params.limit {
                rows.pop_front();
            }
            rows.push_back(summarize_lifecycle_row(total_lines_read, &bytes, &value));
        }
    }
    let rows: Vec<_> = rows.into_iter().collect();
    Ok(AuditLifecycleTailResponse {
        path: path.display().to_string(),
        limit: params.limit,
        max_line_bytes: params.max_line_bytes,
        total_lines_read,
        matched_lines_seen,
        returned_count: rows.len(),
        rows,
    })
}

fn lifecycle_matches(value: &Value, params: &AuditLifecycleTailParams) -> bool {
    params
        .tool
        .as_deref()
        .is_none_or(|tool| string_field(value, "tool").as_deref() == Some(tool))
        && params
            .status
            .as_deref()
            .is_none_or(|status| string_field(value, "status").as_deref() == Some(status))
        && params.event_kind.as_deref().is_none_or(|event_kind| {
            string_field(value, "event_kind").as_deref() == Some(event_kind)
        })
}

fn summarize_lifecycle_row(line_no: u64, bytes: &[u8], value: &Value) -> AuditLifecycleRowSummary {
    let last_tool_event = value.get("last_tool_event");
    let mcp_session_id = string_field(value, "mcp_session_id");
    AuditLifecycleRowSummary {
        line_no,
        raw_len_bytes: bytes.len() as u64,
        raw_sha256: prefixed_sha256(bytes),
        schema_version: value.get("schema_version").and_then(Value::as_u64),
        run_id: string_field(value, "run_id"),
        pid: value.get("pid").and_then(Value::as_u64),
        seq: value.get("seq").and_then(Value::as_u64),
        event_kind: string_field(value, "event_kind"),
        tool: string_field(value, "tool"),
        status: string_field(value, "status"),
        cause: string_field(value, "cause"),
        started_at_unix_ms: value.get("started_at_unix_ms").and_then(Value::as_u64),
        finished_at_unix_ms: value.get("finished_at_unix_ms").and_then(Value::as_u64),
        duration_ms: value.get("duration_ms").and_then(Value::as_u64),
        recorded_at_unix_ms: value.get("recorded_at_unix_ms").and_then(Value::as_u64),
        mcp_session_id_present: mcp_session_id.is_some(),
        mcp_session_id_sha256: mcp_session_id.as_deref().map(sha256_text),
        error_code: nested_string(value, &["error", "data", "code"])
            .or_else(|| nested_string(value, &["error", "code"])),
        panic_present: !value.get("panic").is_none_or(Value::is_null),
        detail_code: nested_string(value, &["detail", "code"]),
        in_flight_count: value
            .get("in_flight_tool_events")
            .and_then(Value::as_array)
            .map(|items| items.len() as u64),
        last_tool: last_tool_event.and_then(|event| string_field(event, "tool")),
        last_tool_status: last_tool_event.and_then(|event| string_field(event, "status")),
    }
}

fn inspect_replay_artifact(
    params: &ReplayArtifactInspectParams,
) -> Result<ReplayArtifactInspectResponse, ErrorData> {
    validate_replay_artifact_params(params)?;
    let path =
        normalize_replay_path(&replay_root(), Some(params.path.as_str())).map_err(|error| {
            delegate_error(
                REPLAY_TOOL,
                "artifact_inspect",
                "path",
                REPLAY_SOT,
                error,
                "choose a replay artifact path under the Synapse replay root",
            )
        })?;
    let bytes = fs::read(&path).map_err(|error| {
        io_error(
            REPLAY_TOOL,
            "artifact_inspect",
            &path.display().to_string(),
            REPLAY_SOT,
            error,
            "verify the replay artifact path exists under the Synapse replay root",
        )
    })?;
    if bytes.len() as u64 > params.max_bytes {
        return Err(ErrorData::new(
            ErrorCode(-32099),
            format!(
                "replay operation=artifact_inspect refused {} because it is {} bytes; max_bytes is {}",
                path.display(),
                bytes.len(),
                params.max_bytes
            ),
            Some(json!({
                "code": error_codes::STORAGE_READ_FAILED,
                "tool": REPLAY_TOOL,
                "operation": "artifact_inspect",
                "source_id": path.display().to_string(),
                "source_of_truth": REPLAY_SOT,
                "bytes": bytes.len(),
                "max_bytes": params.max_bytes,
                "remediation": "raise max_bytes within the schema cap or inspect a narrower replay artifact",
            })),
        ));
    }
    let mut lines = Vec::new();
    if !bytes.is_empty() {
        let line_count = bytes.split(|byte| *byte == b'\n').count();
        for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
            let line_no = u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1);
            let mut line = line;
            if line.last() == Some(&b'\r') {
                line = &line[..line.len().saturating_sub(1)];
            }
            if line.is_empty() && index + 1 == line_count && bytes.last() == Some(&b'\n') {
                continue;
            }
            if line.is_empty() {
                return Err(replay_artifact_corrupt_error(
                    &path,
                    line_no,
                    "empty JSONL record",
                ));
            }
            if lines.len() >= params.max_records {
                return Err(ErrorData::new(
                    ErrorCode(-32099),
                    format!(
                        "replay operation=artifact_inspect exceeded max_records={} for {}",
                        params.max_records,
                        path.display()
                    ),
                    Some(json!({
                        "code": error_codes::STORAGE_READ_FAILED,
                        "tool": REPLAY_TOOL,
                        "operation": "artifact_inspect",
                        "source_id": path.display().to_string(),
                        "source_of_truth": REPLAY_SOT,
                        "max_records": params.max_records,
                        "remediation": "raise max_records within the schema cap or inspect a narrower replay artifact",
                    })),
                ));
            }
            let value: Value = serde_json::from_slice(line).map_err(|error| {
                replay_artifact_corrupt_error(
                    &path,
                    line_no,
                    format!("JSON decode failed: {error}"),
                )
            })?;
            lines.push(summarize_replay_line(line_no, line, &value));
        }
    }
    Ok(ReplayArtifactInspectResponse {
        path: path.display().to_string(),
        source_of_truth: "replay JSONL artifact bytes read from disk".to_owned(),
        exists: true,
        bytes: bytes.len() as u64,
        sha256: prefixed_sha256(&bytes),
        records_read: lines.len(),
        max_records: params.max_records,
        max_bytes: params.max_bytes,
        empty: bytes.is_empty(),
        lines,
    })
}

fn summarize_replay_line(line_no: u64, bytes: &[u8], value: &Value) -> ReplayArtifactLineSummary {
    let record = value.get("record");
    ReplayArtifactLineSummary {
        line_no,
        len_bytes: bytes.len() as u64,
        sha256: prefixed_sha256(bytes),
        target: string_field(value, "target"),
        record_type: record
            .and_then(|record| string_field(record, "type"))
            .or_else(|| string_field(value, "type")),
        demo_id_present: record
            .and_then(|record| record.get("demo_id"))
            .or_else(|| value.get("demo_id"))
            .is_some(),
        profile_id_present: record
            .and_then(|record| record.get("profile_id"))
            .or_else(|| value.get("profile_id"))
            .is_some(),
    }
}

fn validate_audit_params(params: &AuditParams) -> Result<AuditOperation, ErrorData> {
    let operation = AuditOperation::parse(params.operation.as_str())?;
    validate_exact_spec(
        AUDIT_TOOL,
        operation.as_str(),
        &[
            ("command_query", params.command_query.is_some()),
            ("lifecycle_events", params.lifecycle_events.is_some()),
            ("lifecycle_exits", params.lifecycle_exits.is_some()),
            (
                "profile_intelligence",
                params.profile_intelligence.is_some(),
            ),
            ("export_bundle", params.export_bundle.is_some()),
        ],
        AUDIT_SOT,
    )?;
    Ok(operation)
}

fn validate_replay_params(params: &ReplayParams) -> Result<ReplayOperation, ErrorData> {
    let operation = ReplayOperation::parse(params.operation.as_str())?;
    validate_exact_spec(
        REPLAY_TOOL,
        operation.as_str(),
        &[
            ("record", params.record.is_some()),
            ("demo_status", params.demo_status.is_some()),
            ("demo_start", params.demo_start.is_some()),
            ("demo_stop", params.demo_stop.is_some()),
            ("artifact_inspect", params.artifact_inspect.is_some()),
        ],
        REPLAY_SOT,
    )?;
    Ok(operation)
}

fn validate_exact_spec(
    tool: &'static str,
    operation: &'static str,
    specs: &[(&'static str, bool)],
    source_of_truth: &'static str,
) -> Result<(), ErrorData> {
    let matching_present = specs
        .iter()
        .any(|(name, present)| *name == operation && *present);
    let extra = specs
        .iter()
        .filter_map(|(name, present)| (*present && *name != operation).then_some(*name))
        .collect::<Vec<_>>();
    if matching_present && extra.is_empty() {
        return Ok(());
    }
    Err(ErrorData::new(
        ErrorCode(-32602),
        format!("{tool} operation={operation} requires exactly one matching operation payload"),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "tool": tool,
            "operation": operation,
            "source_of_truth": source_of_truth,
            "source_id": operation,
            "matching_payload_present": matching_present,
            "extra_payloads": extra,
            "remediation": "pass exactly one payload object whose key matches operation",
        })),
    ))
}

fn validate_lifecycle_params(params: &AuditLifecycleTailParams) -> Result<(), ErrorData> {
    if params.limit == 0 || params.limit > MAX_LIFECYCLE_LIMIT {
        return Err(params_error(
            AUDIT_TOOL,
            "lifecycle_tail",
            "limit",
            AUDIT_SOT,
            format!("limit must be 1..={MAX_LIFECYCLE_LIMIT}"),
        ));
    }
    if params.max_line_bytes == 0 || params.max_line_bytes > MAX_LINE_BYTES {
        return Err(params_error(
            AUDIT_TOOL,
            "lifecycle_tail",
            "max_line_bytes",
            AUDIT_SOT,
            format!("max_line_bytes must be 1..={MAX_LINE_BYTES}"),
        ));
    }
    Ok(())
}

fn validate_replay_artifact_params(params: &ReplayArtifactInspectParams) -> Result<(), ErrorData> {
    if params.path.trim().is_empty() {
        return Err(params_error(
            REPLAY_TOOL,
            "artifact_inspect",
            "path",
            REPLAY_SOT,
            "path must not be empty",
        ));
    }
    if params.max_bytes == 0 || params.max_bytes > MAX_ARTIFACT_BYTES {
        return Err(params_error(
            REPLAY_TOOL,
            "artifact_inspect",
            "max_bytes",
            REPLAY_SOT,
            format!("max_bytes must be 1..={MAX_ARTIFACT_BYTES}"),
        ));
    }
    if params.max_records > MAX_ARTIFACT_RECORDS {
        return Err(params_error(
            REPLAY_TOOL,
            "artifact_inspect",
            "max_records",
            REPLAY_SOT,
            format!("max_records must be 0..={MAX_ARTIFACT_RECORDS}"),
        ));
    }
    Ok(())
}

fn invalid_operation(
    tool: &'static str,
    operation: &str,
    allowed: &[&'static str],
    source_of_truth: &'static str,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32602),
        format!("{tool} operation={operation} is invalid"),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "tool": tool,
            "operation": operation,
            "source_id": "operation",
            "source_of_truth": source_of_truth,
            "allowed_operations": allowed,
            "remediation": "set operation to one of the allowed values and pass exactly the matching payload object",
        })),
    )
}

fn missing_spec(
    tool: &'static str,
    operation: &'static str,
    source_of_truth: &'static str,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32602),
        format!("{tool} operation={operation} missing operation payload"),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "tool": tool,
            "operation": operation,
            "source_id": operation,
            "source_of_truth": source_of_truth,
            "remediation": "pass the payload object matching operation",
        })),
    )
}

fn params_error(
    tool: &'static str,
    operation: &'static str,
    source_id: &'static str,
    source_of_truth: &'static str,
    message: impl Into<String>,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32602),
        format!(
            "{tool} operation={operation} invalid {source_id}: {}",
            message.into()
        ),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "tool": tool,
            "operation": operation,
            "source_id": source_id,
            "source_of_truth": source_of_truth,
            "remediation": "fix the parameter value and retry",
        })),
    )
}

fn delegate_error(
    tool: &'static str,
    operation: &'static str,
    source_id: &str,
    source_of_truth: &'static str,
    error: ErrorData,
    remediation: &'static str,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "{tool} operation={operation} failed for {source_id}: {}",
            error.message
        ),
        Some(json!({
            "code": error_code_from(&error),
            "tool": tool,
            "operation": operation,
            "source_id": source_id,
            "source_of_truth": source_of_truth,
            "remediation": remediation,
            "cause": {
                "message": error.message.to_string(),
                "data": error.data,
            },
        })),
    )
}

fn io_error(
    tool: &'static str,
    operation: &'static str,
    source_id: &str,
    source_of_truth: &'static str,
    error: std::io::Error,
    remediation: &'static str,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!("{tool} operation={operation} could not read {source_id}: {error}"),
        Some(json!({
            "code": error_codes::STORAGE_READ_FAILED,
            "tool": tool,
            "operation": operation,
            "source_id": source_id,
            "source_of_truth": source_of_truth,
            "remediation": remediation,
            "io_error_kind": format!("{:?}", error.kind()),
        })),
    )
}

fn lifecycle_corrupt_error(path: &Path, line_no: u64, reason: impl Into<String>) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "audit operation=lifecycle_tail found corrupt daemon lifecycle row {}:{}",
            path.display(),
            line_no
        ),
        Some(json!({
            "code": error_codes::STORAGE_READ_FAILED,
            "tool": AUDIT_TOOL,
            "operation": "lifecycle_tail",
            "source_id": path.display().to_string(),
            "source_of_truth": AUDIT_SOT,
            "line_no": line_no,
            "reason": reason.into(),
            "remediation": "inspect the daemon lifecycle JSONL file and repair or rotate the corrupt ledger before trusting audit output",
        })),
    )
}

fn replay_artifact_corrupt_error(
    path: &Path,
    line_no: u64,
    reason: impl Into<String>,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "replay operation=artifact_inspect found corrupt replay JSONL row {}:{}",
            path.display(),
            line_no
        ),
        Some(json!({
            "code": error_codes::STORAGE_READ_FAILED,
            "tool": REPLAY_TOOL,
            "operation": "artifact_inspect",
            "source_id": path.display().to_string(),
            "source_of_truth": REPLAY_SOT,
            "line_no": line_no,
            "reason": reason.into(),
            "remediation": "recreate the replay artifact from source rows or inspect the corrupt JSONL bytes",
        })),
    )
}

fn error_code_from(error: &ErrorData) -> String {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
        .unwrap_or(error_codes::TOOL_INTERNAL_ERROR)
        .to_owned()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn nested_string(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn sha256_text(value: &str) -> String {
    prefixed_sha256(value.as_bytes())
}

fn prefixed_sha256(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

const fn default_lifecycle_limit() -> usize {
    DEFAULT_LIFECYCLE_LIMIT
}

const fn default_max_line_bytes() -> usize {
    DEFAULT_MAX_LINE_BYTES
}

const fn default_artifact_max_bytes() -> u64 {
    DEFAULT_ARTIFACT_MAX_BYTES
}

const fn default_artifact_max_records() -> usize {
    DEFAULT_ARTIFACT_MAX_RECORDS
}
