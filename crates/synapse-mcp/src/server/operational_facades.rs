use std::{collections::BTreeMap, fs, path::PathBuf};

use rmcp::{RoleServer, model::ErrorCode, schemars::JsonSchema, service::RequestContext};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use synapse_core::error_codes;

use super::{
    ErrorData, Json, Parameters, SynapseService, tool, tool_profiles::ToolProfileKind, tool_router,
};

use crate::m3::{
    hygiene::{
        HygieneFlagsParams, HygieneFlagsResponse, HygieneReportParams, HygieneReportResponse,
        HygieneScanStorageParams, HygieneScanStorageResponse, HygieneScanTextParams,
        HygieneScanTextResponse,
    },
    local_models::{
        LocalModelListParams, LocalModelListResponse, LocalModelProbeParams,
        LocalModelProbeResponse, LocalModelRegisterParams, LocalModelRegisterResponse,
        LocalModelRemoveParams, LocalModelRemoveResponse, LocalModelUpdateParams,
        LocalModelUpdateResponse,
    },
    storage::{
        StorageGcOnceParams, StorageGcOnceResponse, StorageInspectParams, StorageInspectResponse,
        StoragePutProbeRowsParams, StoragePutProbeRowsResponse, StorageSummaryResponse,
    },
};

const STORAGE_TOOL: &str = "storage";
const MODEL_TOOL: &str = "model";
const HYGIENE_TOOL: &str = "hygiene";
const SETUP_TOOL: &str = "setup";
const TELEMETRY_TOOL: &str = "telemetry";
const STORAGE_SOT: &str = "RocksDB CF metadata + exact row readbacks";
const MODEL_SOT: &str = "CF_KV local_model_registry/v1 rows + probe evidence rows";
const HYGIENE_SOT: &str = "CF_KV hygiene/flag/v1 rows + physical source rows";
const SETUP_SOT: &str = "%APPDATA%\\synapse\\token.txt + %LOCALAPPDATA%\\synapse\\db-daemon\\daemon-run-current.json + Codex MCP config";
const TELEMETRY_SOT: &str = "CF_TELEMETRY + CF_AGENT_EVENTS + daemon profile/tool counters";

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageOperation {
    Inspect,
    Summary,
    PutProbeRows,
    GcOnce,
}

impl StorageOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::Summary => "summary",
            Self::PutProbeRows => "put_probe_rows",
            Self::GcOnce => "gc_once",
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageParams {
    pub operation: StorageOperation,
    #[serde(default)]
    pub inspect: Option<StorageInspectParams>,
    #[serde(default)]
    pub summary: Option<StorageInspectParams>,
    #[serde(default)]
    pub put_probe_rows: Option<StoragePutProbeRowsParams>,
    #[serde(default)]
    pub gc_once: Option<StorageGcOnceParams>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageResponse {
    pub operation: StorageOperation,
    pub source_of_truth: String,
    pub readback_source_of_truth: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspect: Option<StorageInspectResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<StorageSummaryResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub put_probe_rows: Option<StoragePutProbeRowsResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gc_once: Option<StorageGcOnceResponse>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelOperation {
    List,
    Status,
    Probe,
    Register,
    Update,
    Remove,
}

impl ModelOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Status => "status",
            Self::Probe => "probe",
            Self::Register => "register",
            Self::Update => "update",
            Self::Remove => "remove",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelStatusParams {
    #[serde(default)]
    pub include_disabled: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelParams {
    pub operation: ModelOperation,
    #[serde(default)]
    pub list: Option<LocalModelListParams>,
    #[serde(default)]
    pub status: Option<ModelStatusParams>,
    #[serde(default)]
    pub probe: Option<LocalModelProbeParams>,
    #[serde(default)]
    pub register: Option<LocalModelRegisterParams>,
    #[serde(default)]
    pub update: Option<LocalModelUpdateParams>,
    #[serde(default)]
    pub remove: Option<LocalModelRemoveParams>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelStatusResponse {
    pub source_of_truth: &'static str,
    pub scanned_rows: usize,
    pub visible_rows: usize,
    pub corrupt_rows: usize,
    pub enabled_rows: usize,
    pub disabled_rows: usize,
    pub probed_rows: usize,
    pub healthy_rows: usize,
    pub unhealthy_rows: usize,
    pub rows_with_api_key_secret: usize,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelResponse {
    pub operation: ModelOperation,
    pub source_of_truth: String,
    pub readback_source_of_truth: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<LocalModelListResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ModelStatusResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<LocalModelProbeResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register: Option<LocalModelRegisterResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<LocalModelUpdateResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove: Option<LocalModelRemoveResponse>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HygieneOperation {
    ScanText,
    ScanStorage,
    Flags,
    Report,
}

impl HygieneOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ScanText => "scan_text",
            Self::ScanStorage => "scan_storage",
            Self::Flags => "flags",
            Self::Report => "report",
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HygieneParams {
    pub operation: HygieneOperation,
    #[serde(default)]
    pub scan_text: Option<HygieneScanTextParams>,
    #[serde(default)]
    pub scan_storage: Option<HygieneScanStorageParams>,
    #[serde(default)]
    pub flags: Option<HygieneFlagsParams>,
    #[serde(default)]
    pub report: Option<HygieneReportParams>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HygieneResponse {
    pub operation: HygieneOperation,
    pub source_of_truth: String,
    pub readback_source_of_truth: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_text: Option<HygieneScanTextResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_storage: Option<HygieneScanStorageResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<HygieneFlagsResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<HygieneReportResponse>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupOperation {
    Status,
    Doctor,
    Repair,
}

impl SetupOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Doctor => "doctor",
            Self::Repair => "repair",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupStatusParams {}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupRepairParams {
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupParams {
    pub operation: SetupOperation,
    #[serde(default)]
    pub status: Option<SetupStatusParams>,
    #[serde(default)]
    pub doctor: Option<SetupStatusParams>,
    #[serde(default)]
    pub repair: Option<SetupRepairParams>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileReadback {
    pub path: String,
    pub exists: bool,
    pub len_bytes: Option<u64>,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupStatusResponse {
    pub source_of_truth: &'static str,
    pub pid: u32,
    pub bind: String,
    pub token_file: FileReadback,
    pub daemon_run_file: FileReadback,
    pub codex_config_file: FileReadback,
    pub token_env_present: bool,
    pub token_env_len_bytes: Option<usize>,
    pub codex_mcp_config_mentions_synapse: bool,
    pub codex_mcp_config_mentions_bearer_env: bool,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupResponse {
    pub operation: SetupOperation,
    pub source_of_truth: String,
    pub readback_source_of_truth: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<SetupStatusResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doctor: Option<SetupStatusResponse>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryOperation {
    Status,
}

impl TelemetryOperation {
    const fn as_str(self) -> &'static str {
        "status"
    }
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TelemetryStatusParams {}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TelemetryParams {
    pub operation: TelemetryOperation,
    #[serde(default)]
    pub status: Option<TelemetryStatusParams>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentEventIngressStats {
    pub accepted_total: u64,
    pub rejected_unknown_spawn_total: u64,
    pub rejected_malformed_total: u64,
    pub rejected_storage_total: u64,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolSurfaceTelemetry {
    pub source_of_truth: &'static str,
    pub profile: String,
    pub profile_label: String,
    pub profile_source: String,
    pub visible_tool_count: usize,
    pub visible_public_tool_count: usize,
    pub implementation_tool_count: usize,
    pub hidden_implementation_tool_count: usize,
    pub public_tool_count: usize,
    pub max_public_tool_count: usize,
    pub over_public_tool_limit_by: usize,
    pub profile_gated_public_tool_count: usize,
    pub registered_public_tool_count: usize,
    pub missing_public_tool_count: usize,
    pub denied_break_glass_tool_count: usize,
    pub hidden_tool_route_count: usize,
    pub last_tool_surface_sha256: String,
    pub visible_tool_sha256: String,
    pub public_tool_sha256: String,
    pub facade_contract_sha256: String,
    pub facade_contract_tool_count: usize,
    pub facade_contract_operation_count: usize,
    pub facade_contract_mutating_operation_count: usize,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TelemetryStatusResponse {
    pub source_of_truth: &'static str,
    pub tool_surface: ToolSurfaceTelemetry,
    pub storage_summary: StorageSummaryResponse,
    pub agent_event_ingress: AgentEventIngressStats,
    pub cf_row_counts: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TelemetryResponse {
    pub operation: TelemetryOperation,
    pub source_of_truth: String,
    pub readback_source_of_truth: String,
    pub status: TelemetryStatusResponse,
}

#[tool_router(router = operational_facade_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Public storage facade for the <=40 MCP surface. operation=inspect/summary are read-only RocksDB CF reports; operation=put_probe_rows/gc_once are maintenance-gated and return separate CF row-count/readback evidence. Unknown operations and mismatched operation payloads fail closed."
    )]
    pub async fn storage(
        &self,
        params: Parameters<StorageParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<StorageResponse>, ErrorData> {
        validate_storage_params(&params.0)?;
        let operation = params.0.operation;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = STORAGE_TOOL,
            operation = operation.as_str(),
            "tool.invocation kind=storage"
        );
        match operation {
            StorageOperation::Inspect => {
                let spec = params.0.inspect.unwrap_or_default();
                self.require_m3_permissions(
                    STORAGE_TOOL,
                    &crate::m3::storage::required_permissions_inspect(&spec),
                )?;
                let runtime = self.reflex_runtime().map_err(|error| {
                    facade_delegate_error(
                        STORAGE_TOOL,
                        operation.as_str(),
                        "reflex_runtime",
                        STORAGE_SOT,
                        error,
                        "repair storage/reflex initialization and retry storage operation=inspect",
                    )
                })?;
                let response =
                    crate::m3::storage::inspect_storage(&runtime, &spec).map_err(|error| {
                        facade_delegate_error(
                            STORAGE_TOOL,
                            operation.as_str(),
                            "storage_inspect",
                            STORAGE_SOT,
                            error,
                            "inspect storage health and CF metadata before retrying",
                        )
                    })?;
                Ok(Json(storage_response(
                    operation,
                    format!(
                        "RocksDB CF rows={} pressure={}",
                        response.cf_row_counts.len(),
                        response.pressure_level.name
                    ),
                    |out| out.inspect = Some(response),
                )))
            }
            StorageOperation::Summary => {
                let spec = params.0.summary.unwrap_or_default();
                self.require_m3_permissions(
                    STORAGE_TOOL,
                    &crate::m3::storage::required_permissions_inspect(&spec),
                )?;
                let response = self.storage_summary_snapshot().map_err(|error| {
                    facade_delegate_error(
                        STORAGE_TOOL,
                        operation.as_str(),
                        "storage_summary",
                        STORAGE_SOT,
                        error,
                        "repair storage initialization and read RocksDB CF metadata again",
                    )
                })?;
                Ok(Json(storage_response(
                    operation,
                    format!(
                        "RocksDB summary cf_count={} pressure={}",
                        response.cf_row_counts.len(),
                        response.pressure_level.name
                    ),
                    |out| out.summary = Some(response),
                )))
            }
            StorageOperation::PutProbeRows => {
                let spec = params
                    .0
                    .put_probe_rows
                    .ok_or_else(|| missing_spec(STORAGE_TOOL, "put_probe_rows"))?;
                require_maintenance_profile(
                    self,
                    &request_context,
                    STORAGE_TOOL,
                    operation.as_str(),
                    &spec.cf_name,
                    STORAGE_SOT,
                )?;
                self.require_m3_permissions(
                    STORAGE_TOOL,
                    &crate::m3::storage::required_permissions_put(&spec),
                )?;
                let runtime = self.reflex_runtime()?;
                let response = crate::m3::storage::put_probe_rows(&runtime, &spec).map_err(
                    |error| {
                        facade_delegate_error(
                            STORAGE_TOOL,
                            operation.as_str(),
                            &spec.cf_name,
                            STORAGE_SOT,
                            error,
                            "fix cf_name/key/value limits and inspect CF row counts before retrying",
                        )
                    },
                )?;
                Ok(Json(storage_response(
                    operation,
                    format!(
                        "{} before_rows={} after_rows={}",
                        response.cf_name, response.before_rows, response.after_rows
                    ),
                    |out| out.put_probe_rows = Some(response),
                )))
            }
            StorageOperation::GcOnce => {
                let spec = params
                    .0
                    .gc_once
                    .ok_or_else(|| missing_spec(STORAGE_TOOL, "gc_once"))?;
                require_maintenance_profile(
                    self,
                    &request_context,
                    STORAGE_TOOL,
                    operation.as_str(),
                    &spec.cf_name,
                    STORAGE_SOT,
                )?;
                self.require_m3_permissions(
                    STORAGE_TOOL,
                    &crate::m3::storage::required_permissions_gc(&spec),
                )?;
                let runtime = self.reflex_runtime()?;
                let response =
                    crate::m3::storage::run_storage_gc_once(&runtime, &spec).map_err(|error| {
                        facade_delegate_error(
                            STORAGE_TOOL,
                            operation.as_str(),
                            &spec.cf_name,
                            STORAGE_SOT,
                            error,
                            "fix row caps / CF name and inspect CF row counts before retrying",
                        )
                    })?;
                Ok(Json(storage_response(
                    operation,
                    format!(
                        "{} before_rows={} after_rows={} evicted={}",
                        response.cf_name,
                        response.before_rows,
                        response.after_rows,
                        response.total_evicted_rows
                    ),
                    |out| out.gc_once = Some(response),
                )))
            }
        }
    }

    #[tool(
        description = "Public local-model facade for the <=40 MCP surface. operation=list/status read CF_KV registry rows; operation=probe writes real endpoint probe evidence; register/update/remove are maintenance-gated and require physical CF_KV readback."
    )]
    pub async fn model(
        &self,
        params: Parameters<ModelParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ModelResponse>, ErrorData> {
        validate_model_params(&params.0)?;
        let operation = params.0.operation;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = MODEL_TOOL,
            operation = operation.as_str(),
            "tool.invocation kind=model"
        );
        let by_session = session_or_stdio(&request_context)?;
        let db = self.m3_storage().map_err(|error| {
            facade_delegate_error(
                MODEL_TOOL,
                operation.as_str(),
                "m3_storage",
                MODEL_SOT,
                error,
                "repair M3 storage and retry the model registry operation",
            )
        })?;
        match operation {
            ModelOperation::List => {
                let spec = params.0.list.unwrap_or(LocalModelListParams {
                    name: None,
                    include_disabled: true,
                    limit: 100,
                });
                self.require_m3_permissions(
                    MODEL_TOOL,
                    &crate::m3::local_models::required_permissions_list(&spec),
                )?;
                let response =
                    crate::m3::local_models::list_local_models(&db, &spec).map_err(|error| {
                        facade_delegate_error(
                            MODEL_TOOL,
                            operation.as_str(),
                            spec.name.as_deref().unwrap_or("registry"),
                            MODEL_SOT,
                            error,
                            "inspect CF_KV local model registry rows and corrupt row diagnostics",
                        )
                    })?;
                Ok(Json(model_response(
                    operation,
                    format!(
                        "registry rows={} corrupt_rows={}",
                        response.rows.len(),
                        response.corrupt_rows.len()
                    ),
                    |out| out.list = Some(response),
                )))
            }
            ModelOperation::Status => {
                let spec = params.0.status.unwrap_or_default();
                let list_params = LocalModelListParams {
                    name: None,
                    include_disabled: spec.include_disabled,
                    limit: 1000,
                };
                self.require_m3_permissions(
                    MODEL_TOOL,
                    &crate::m3::local_models::required_permissions_list(&list_params),
                )?;
                let list = crate::m3::local_models::list_local_models(&db, &list_params).map_err(
                    |error| {
                        facade_delegate_error(
                            MODEL_TOOL,
                            operation.as_str(),
                            "registry_status",
                            MODEL_SOT,
                            error,
                            "inspect CF_KV local model registry rows and storage health",
                        )
                    },
                )?;
                let status = model_status(&list);
                Ok(Json(model_response(
                    operation,
                    format!(
                        "registry visible_rows={} healthy_rows={} corrupt_rows={}",
                        status.visible_rows, status.healthy_rows, status.corrupt_rows
                    ),
                    |out| out.status = Some(status),
                )))
            }
            ModelOperation::Probe => {
                let spec = params
                    .0
                    .probe
                    .ok_or_else(|| missing_spec(MODEL_TOOL, "probe"))?;
                self.require_m3_permissions(
                    MODEL_TOOL,
                    &crate::m3::local_models::required_permissions_probe(&spec),
                )?;
                let response =
                    crate::m3::local_models::probe_local_model(&db, &spec, &by_session)
                        .await
                        .map_err(|error| {
                            facade_delegate_error(
                                MODEL_TOOL,
                                operation.as_str(),
                                &spec.name,
                                MODEL_SOT,
                                error,
                                "repair the real backend endpoint/socket/credentials and retry model operation=probe",
                            )
                        })?;
                Ok(Json(model_response(
                    operation,
                    format!(
                        "{} probe_status={} healthy={}",
                        response.row.name, response.probe.status, response.probe.healthy
                    ),
                    |out| out.probe = Some(response),
                )))
            }
            ModelOperation::Register => {
                let spec = params
                    .0
                    .register
                    .ok_or_else(|| missing_spec(MODEL_TOOL, "register"))?;
                require_maintenance_profile(
                    self,
                    &request_context,
                    MODEL_TOOL,
                    operation.as_str(),
                    &spec.name,
                    MODEL_SOT,
                )?;
                self.require_m3_permissions(
                    MODEL_TOOL,
                    &crate::m3::local_models::required_permissions_register(&spec),
                )?;
                let response =
                    crate::m3::local_models::register_local_model(&db, spec, &by_session)
                        .await
                        .map_err(|error| {
                            facade_delegate_error(
                                MODEL_TOOL,
                                operation.as_str(),
                                "register",
                                MODEL_SOT,
                                error,
                                "fix endpoint/model/key settings until the real structured tool-call probe passes",
                            )
                        })?;
                Ok(Json(model_response(
                    operation,
                    format!("{} row_key={}", response.row.name, response.row.row_key),
                    |out| out.register = Some(response),
                )))
            }
            ModelOperation::Update => {
                let spec = params
                    .0
                    .update
                    .ok_or_else(|| missing_spec(MODEL_TOOL, "update"))?;
                require_maintenance_profile(
                    self,
                    &request_context,
                    MODEL_TOOL,
                    operation.as_str(),
                    &spec.name,
                    MODEL_SOT,
                )?;
                self.require_m3_permissions(
                    MODEL_TOOL,
                    &crate::m3::local_models::required_permissions_update(&spec),
                )?;
                let response = crate::m3::local_models::update_local_model(&db, spec, &by_session)
                    .await
                    .map_err(|error| {
                        facade_delegate_error(
                            MODEL_TOOL,
                            operation.as_str(),
                            "update",
                            MODEL_SOT,
                            error,
                            "fix endpoint/model/key settings until the real structured tool-call probe passes",
                        )
                    })?;
                Ok(Json(model_response(
                    operation,
                    format!("{} row_key={}", response.row.name, response.row.row_key),
                    |out| out.update = Some(response),
                )))
            }
            ModelOperation::Remove => {
                let spec = params
                    .0
                    .remove
                    .ok_or_else(|| missing_spec(MODEL_TOOL, "remove"))?;
                require_maintenance_profile(
                    self,
                    &request_context,
                    MODEL_TOOL,
                    operation.as_str(),
                    &spec.name,
                    MODEL_SOT,
                )?;
                self.require_m3_permissions(
                    MODEL_TOOL,
                    &crate::m3::local_models::required_permissions_remove(&spec),
                )?;
                let response =
                    crate::m3::local_models::remove_local_model(&db, &spec).map_err(|error| {
                        facade_delegate_error(
                            MODEL_TOOL,
                            operation.as_str(),
                            &spec.name,
                            MODEL_SOT,
                            error,
                            "inspect the exact registry row and retry only if removal is intended",
                        )
                    })?;
                Ok(Json(model_response(
                    operation,
                    format!(
                        "{} after_row_present={}",
                        response.removed_row.name, response.after_row_present
                    ),
                    |out| out.remove = Some(response),
                )))
            }
        }
    }

    #[tool(
        description = "Public prompt-injection hygiene facade for the <=40 MCP surface. Read operations flags/report are normal-profile visible. scan_text without persistence is read-only; scan_text persist=true and scan_storage write CF_KV flag rows and are maintenance-gated with readback."
    )]
    pub async fn hygiene(
        &self,
        params: Parameters<HygieneParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<HygieneResponse>, ErrorData> {
        validate_hygiene_params(&params.0)?;
        let operation = params.0.operation;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = HYGIENE_TOOL,
            operation = operation.as_str(),
            "tool.invocation kind=hygiene"
        );
        let runtime = self.reflex_runtime().map_err(|error| {
            facade_delegate_error(
                HYGIENE_TOOL,
                operation.as_str(),
                "reflex_runtime",
                HYGIENE_SOT,
                error,
                "repair storage/reflex initialization and retry the hygiene operation",
            )
        })?;
        match operation {
            HygieneOperation::ScanText => {
                let spec = params
                    .0
                    .scan_text
                    .ok_or_else(|| missing_spec(HYGIENE_TOOL, "scan_text"))?;
                if spec.persist {
                    require_maintenance_profile(
                        self,
                        &request_context,
                        HYGIENE_TOOL,
                        operation.as_str(),
                        spec.source_cf.as_deref().unwrap_or("source_cf_missing"),
                        HYGIENE_SOT,
                    )?;
                }
                self.require_m3_permissions(
                    HYGIENE_TOOL,
                    &crate::m3::hygiene::required_permissions_scan_text(&spec),
                )?;
                let response = crate::m3::hygiene::scan_text_tool(&runtime, &spec).map_err(
                    |error| {
                        facade_delegate_error(
                            HYGIENE_TOOL,
                            operation.as_str(),
                            spec.source_key_hex.as_deref().unwrap_or("text_only"),
                            HYGIENE_SOT,
                            error,
                            "fix text/source row identity and inspect hygiene flags before retrying",
                        )
                    },
                )?;
                Ok(Json(hygiene_response(
                    operation,
                    format!(
                        "matches={} flags_written={}",
                        response.matches.len(),
                        response.flags_written
                    ),
                    |out| out.scan_text = Some(response),
                )))
            }
            HygieneOperation::ScanStorage => {
                let spec = params
                    .0
                    .scan_storage
                    .ok_or_else(|| missing_spec(HYGIENE_TOOL, "scan_storage"))?;
                require_maintenance_profile(
                    self,
                    &request_context,
                    HYGIENE_TOOL,
                    operation.as_str(),
                    "storage_scan",
                    HYGIENE_SOT,
                )?;
                self.require_m3_permissions(
                    HYGIENE_TOOL,
                    &crate::m3::hygiene::required_permissions_scan_storage(&spec),
                )?;
                let response =
                    crate::m3::hygiene::scan_storage(&runtime, &spec).map_err(|error| {
                        facade_delegate_error(
                            HYGIENE_TOOL,
                            operation.as_str(),
                            "storage_scan",
                            HYGIENE_SOT,
                            error,
                            "fix source_cfs/cursor and inspect CF_KV hygiene flag rows",
                        )
                    })?;
                Ok(Json(hygiene_response(
                    operation,
                    format!(
                        "scanned_rows={} flags_written={}",
                        response.scanned_rows, response.flags_written
                    ),
                    |out| out.scan_storage = Some(response),
                )))
            }
            HygieneOperation::Flags => {
                let spec = params.0.flags.unwrap_or_default();
                self.require_m3_permissions(
                    HYGIENE_TOOL,
                    &crate::m3::hygiene::required_permissions_flags(&spec),
                )?;
                let response =
                    crate::m3::hygiene::query_flags(&runtime, &spec).map_err(|error| {
                        facade_delegate_error(
                            HYGIENE_TOOL,
                            operation.as_str(),
                            spec.source_key_hex.as_deref().unwrap_or("flag_prefix"),
                            HYGIENE_SOT,
                            error,
                            "inspect CF_KV hygiene/flag/v1 rows and cursor format",
                        )
                    })?;
                Ok(Json(hygiene_response(
                    operation,
                    format!(
                        "flags={} scanned_rows={}",
                        response.flags.len(),
                        response.scanned_rows
                    ),
                    |out| out.flags = Some(response),
                )))
            }
            HygieneOperation::Report => {
                let spec = params.0.report.unwrap_or_default();
                self.require_m3_permissions(
                    HYGIENE_TOOL,
                    &crate::m3::hygiene::required_permissions_report(&spec),
                )?;
                let response = crate::m3::hygiene::report(&runtime, &spec).map_err(|error| {
                    facade_delegate_error(
                        HYGIENE_TOOL,
                        operation.as_str(),
                        spec.source_key_hex.as_deref().unwrap_or("report"),
                        HYGIENE_SOT,
                        error,
                        "inspect hygiene report joins and CF_KV flag/taint rows",
                    )
                })?;
                Ok(Json(hygiene_response(
                    operation,
                    format!(
                        "flags_total={} impacted_routines={}",
                        response.summary.flags_total, response.summary.impacted_routine_count
                    ),
                    |out| out.report = Some(response),
                )))
            }
        }
    }

    #[tool(
        description = "Public setup facade for the <=40 MCP surface. operation=status/doctor read host setup Source-of-Truth files, daemon pid/bind, and Codex MCP config. operation=repair is maintenance-gated and refuses normal-agent self-repair instead of silently mutating the running daemon."
    )]
    pub async fn setup(
        &self,
        params: Parameters<SetupParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<SetupResponse>, ErrorData> {
        validate_setup_params(&params.0)?;
        let operation = params.0.operation;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = SETUP_TOOL,
            operation = operation.as_str(),
            "tool.invocation kind=setup"
        );
        match operation {
            SetupOperation::Status | SetupOperation::Doctor => {
                let status = setup_status(self).map_err(|error| {
                    facade_delegate_error(
                        SETUP_TOOL,
                        operation.as_str(),
                        "setup_status",
                        SETUP_SOT,
                        error,
                        "repair the exact unreadable setup file/env prerequisite and retry setup status",
                    )
                })?;
                Ok(Json(setup_response(
                    operation,
                    "setup status physical files read".to_owned(),
                    |out| {
                        if operation == SetupOperation::Status {
                            out.status = Some(status);
                        } else {
                            out.doctor = Some(status);
                        }
                    },
                )))
            }
            SetupOperation::Repair => {
                let spec = params
                    .0
                    .repair
                    .ok_or_else(|| missing_spec(SETUP_TOOL, "repair"))?;
                if spec.reason.trim().is_empty() {
                    return Err(missing_spec(SETUP_TOOL, "repair.reason"));
                }
                require_maintenance_profile(
                    self,
                    &request_context,
                    SETUP_TOOL,
                    operation.as_str(),
                    "synapse_setup_repair",
                    SETUP_SOT,
                )?;
                Err(facade_policy_error(
                    SETUP_TOOL,
                    operation.as_str(),
                    "synapse_setup_repair",
                    ToolProfileKind::BreakGlass,
                    SETUP_SOT,
                    "run scripts\\synapse-setup.ps1 from an external maintenance process so daemon replacement has a separate process/socket SoT; in-process self-repair is refused",
                ))
            }
        }
    }

    #[tool(
        description = "Public telemetry facade for the <=40 MCP surface. operation=status returns active profile, visible/public/implementation/hidden/profile-gated tool counts, tool-surface hashes, storage CF counters, and agent-event ingress counters from physical SoTs."
    )]
    pub async fn telemetry(
        &self,
        params: Parameters<TelemetryParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<TelemetryResponse>, ErrorData> {
        validate_telemetry_params(&params.0)?;
        let operation = params.0.operation;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TELEMETRY_TOOL,
            operation = operation.as_str(),
            "tool.invocation kind=telemetry"
        );
        let session_id = super::context::mcp_session_id_from_request_context(&request_context)?;
        let snapshot = self
            .tool_profile_snapshot(session_id.as_deref())
            .map_err(|error| {
                facade_delegate_error(
                    TELEMETRY_TOOL,
                    operation.as_str(),
                    "tool_profile_snapshot",
                    TELEMETRY_SOT,
                    error,
                    "inspect CF_SESSIONS tool profile rows and retry telemetry status",
                )
            })?;
        let storage_summary = self.storage_summary_snapshot().map_err(|error| {
            facade_delegate_error(
                TELEMETRY_TOOL,
                operation.as_str(),
                "storage_summary",
                TELEMETRY_SOT,
                error,
                "repair storage initialization and retry telemetry status",
            )
        })?;
        let ingress = agent_ingress_stats();
        let cf_row_counts = storage_summary.cf_row_counts.clone();
        let status = TelemetryStatusResponse {
            source_of_truth: TELEMETRY_SOT,
            tool_surface: tool_surface_telemetry(&snapshot),
            storage_summary,
            agent_event_ingress: ingress,
            cf_row_counts,
        };
        Ok(Json(TelemetryResponse {
            operation,
            source_of_truth: TELEMETRY_SOT.to_owned(),
            readback_source_of_truth:
                "tool profile snapshot + RocksDB storage summary + ingress counters".to_owned(),
            status,
        }))
    }
}

fn storage_response(
    operation: StorageOperation,
    readback: String,
    fill: impl FnOnce(&mut StorageResponse),
) -> StorageResponse {
    let mut response = StorageResponse {
        operation,
        source_of_truth: STORAGE_SOT.to_owned(),
        readback_source_of_truth: readback,
        inspect: None,
        summary: None,
        put_probe_rows: None,
        gc_once: None,
    };
    fill(&mut response);
    response
}

fn model_response(
    operation: ModelOperation,
    readback: String,
    fill: impl FnOnce(&mut ModelResponse),
) -> ModelResponse {
    let mut response = ModelResponse {
        operation,
        source_of_truth: MODEL_SOT.to_owned(),
        readback_source_of_truth: readback,
        list: None,
        status: None,
        probe: None,
        register: None,
        update: None,
        remove: None,
    };
    fill(&mut response);
    response
}

fn hygiene_response(
    operation: HygieneOperation,
    readback: String,
    fill: impl FnOnce(&mut HygieneResponse),
) -> HygieneResponse {
    let mut response = HygieneResponse {
        operation,
        source_of_truth: HYGIENE_SOT.to_owned(),
        readback_source_of_truth: readback,
        scan_text: None,
        scan_storage: None,
        flags: None,
        report: None,
    };
    fill(&mut response);
    response
}

fn setup_response(
    operation: SetupOperation,
    readback: String,
    fill: impl FnOnce(&mut SetupResponse),
) -> SetupResponse {
    let mut response = SetupResponse {
        operation,
        source_of_truth: SETUP_SOT.to_owned(),
        readback_source_of_truth: readback,
        status: None,
        doctor: None,
    };
    fill(&mut response);
    response
}

fn model_status(list: &LocalModelListResponse) -> ModelStatusResponse {
    let enabled_rows = list.rows.iter().filter(|row| row.enabled).count();
    let probed_rows = list
        .rows
        .iter()
        .filter(|row| row.last_probe.is_some())
        .count();
    let healthy_rows = list
        .rows
        .iter()
        .filter(|row| row.last_probe.as_ref().is_some_and(|probe| probe.healthy))
        .count();
    ModelStatusResponse {
        source_of_truth: "CF_KV prefix local_model_registry/v1/model/name_hex/",
        scanned_rows: list.scanned_rows,
        visible_rows: list.rows.len(),
        corrupt_rows: list.corrupt_rows.len(),
        enabled_rows,
        disabled_rows: list.rows.len().saturating_sub(enabled_rows),
        probed_rows,
        healthy_rows,
        unhealthy_rows: probed_rows.saturating_sub(healthy_rows),
        rows_with_api_key_secret: list
            .rows
            .iter()
            .filter(|row| row.has_api_key_secret)
            .count(),
    }
}

fn session_or_stdio(request_context: &RequestContext<RoleServer>) -> Result<String, ErrorData> {
    Ok(
        super::context::mcp_session_id_from_request_context(request_context)?
            .unwrap_or_else(|| "stdio".to_owned()),
    )
}

fn require_maintenance_profile(
    service: &SynapseService,
    request_context: &RequestContext<RoleServer>,
    tool: &'static str,
    operation: &'static str,
    source_id: &str,
    source_of_truth: &'static str,
) -> Result<(), ErrorData> {
    let session_id = super::context::mcp_session_id_from_request_context(request_context)?;
    let snapshot = service.tool_profile_snapshot(session_id.as_deref())?;
    if matches!(
        snapshot.profile,
        ToolProfileKind::BreakGlass | ToolProfileKind::FullCapability
    ) {
        return Ok(());
    }
    Err(facade_policy_error(
        tool,
        operation,
        source_id,
        snapshot.profile,
        source_of_truth,
        "switch to an explicit maintenance profile with operator intent before running this mutating operation; normal_agent may use the read-only operation first",
    ))
}

fn setup_status(service: &SynapseService) -> Result<SetupStatusResponse, ErrorData> {
    let bind = service.m3_bind_addr()?;
    let token_file = file_readback(appdata_path(["synapse", "token.txt"]));
    let daemon_run_file = file_readback(localappdata_path([
        "synapse",
        "db-daemon",
        "daemon-run-current.json",
    ]));
    let codex_config_file = file_readback(userprofile_path([".codex", "config.toml"]));
    let codex_text = fs::read_to_string(codex_config_file.path.as_str()).unwrap_or_default();
    let token_env = std::env::var("SYNAPSE_BEARER_TOKEN").ok();
    Ok(SetupStatusResponse {
        source_of_truth: SETUP_SOT,
        pid: std::process::id(),
        bind,
        token_file,
        daemon_run_file,
        codex_config_file,
        token_env_present: token_env.is_some(),
        token_env_len_bytes: token_env.as_ref().map(|value| value.len()),
        codex_mcp_config_mentions_synapse: codex_text.contains("[mcp_servers.synapse]")
            || codex_text.contains("synapse"),
        codex_mcp_config_mentions_bearer_env: codex_text.contains("SYNAPSE_BEARER_TOKEN"),
    })
}

fn file_readback(path: PathBuf) -> FileReadback {
    match fs::read(&path) {
        Ok(bytes) => FileReadback {
            path: path.display().to_string(),
            exists: true,
            len_bytes: Some(bytes.len() as u64),
            sha256: Some(format!("sha256:{}", sha256_hex(&bytes))),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileReadback {
            path: path.display().to_string(),
            exists: false,
            len_bytes: None,
            sha256: None,
        },
        Err(error) => FileReadback {
            path: path.display().to_string(),
            exists: false,
            len_bytes: Some(error.raw_os_error().unwrap_or_default() as u64),
            sha256: None,
        },
    }
}

fn appdata_path<const N: usize>(parts: [&str; N]) -> PathBuf {
    env_path("APPDATA", "C:\\Users\\Default\\AppData\\Roaming", parts)
}

fn localappdata_path<const N: usize>(parts: [&str; N]) -> PathBuf {
    env_path("LOCALAPPDATA", "C:\\Users\\Default\\AppData\\Local", parts)
}

fn userprofile_path<const N: usize>(parts: [&str; N]) -> PathBuf {
    env_path("USERPROFILE", "C:\\Users\\Default", parts)
}

fn env_path<const N: usize>(name: &str, fallback: &str, parts: [&str; N]) -> PathBuf {
    let mut path = PathBuf::from(std::env::var(name).unwrap_or_else(|_| fallback.to_owned()));
    for part in parts {
        path.push(part);
    }
    path
}

fn agent_ingress_stats() -> AgentEventIngressStats {
    let value = super::agent_event_ingress::ingress_stats();
    AgentEventIngressStats {
        accepted_total: stat_u64(&value, "accepted_total"),
        rejected_unknown_spawn_total: stat_u64(&value, "rejected_unknown_spawn_total"),
        rejected_malformed_total: stat_u64(&value, "rejected_malformed_total"),
        rejected_storage_total: stat_u64(&value, "rejected_storage_total"),
    }
}

fn tool_surface_telemetry(
    snapshot: &super::tool_profiles::ToolProfileSnapshot,
) -> ToolSurfaceTelemetry {
    let visible_public_count = count_visible_public_tools(
        &snapshot.public_tool_registry.public_tool_names,
        &snapshot.visible_tool_names,
    );
    ToolSurfaceTelemetry {
        source_of_truth: snapshot.source_of_truth,
        profile: snapshot.profile.as_str().to_owned(),
        profile_label: snapshot.profile_label.to_owned(),
        profile_source: snapshot.source.clone(),
        visible_tool_count: snapshot.visible_tool_count,
        visible_public_tool_count: visible_public_count,
        implementation_tool_count: snapshot.implementation_tool_count,
        hidden_implementation_tool_count: snapshot
            .implementation_tool_count
            .saturating_sub(snapshot.visible_tool_count),
        public_tool_count: snapshot.public_tool_registry.public_tool_count,
        max_public_tool_count: snapshot.public_tool_registry.max_public_tool_count,
        over_public_tool_limit_by: snapshot.public_tool_registry.over_limit_by,
        profile_gated_public_tool_count: snapshot
            .public_tool_registry
            .public_tool_count
            .saturating_sub(visible_public_count),
        registered_public_tool_count: snapshot.public_tool_registry.registered_tools_present.len(),
        missing_public_tool_count: snapshot.public_tool_registry.registered_tools_missing.len(),
        denied_break_glass_tool_count: snapshot.denied_break_glass_tools.len(),
        hidden_tool_route_count: snapshot.hidden_tool_routes.len(),
        last_tool_surface_sha256: snapshot.visible_tool_sha256.clone(),
        visible_tool_sha256: snapshot.visible_tool_sha256.clone(),
        public_tool_sha256: snapshot.public_tool_registry.public_tool_sha256.clone(),
        facade_contract_sha256: snapshot.facade_contract.facade_contract_sha256.clone(),
        facade_contract_tool_count: snapshot.facade_contract.contract_tool_count,
        facade_contract_operation_count: snapshot.facade_contract.operation_count,
        facade_contract_mutating_operation_count: snapshot.facade_contract.mutating_operation_count,
    }
}

fn count_visible_public_tools(
    public_tool_names: &[String],
    visible_tool_names: &[String],
) -> usize {
    visible_tool_names
        .iter()
        .filter(|visible| public_tool_names.iter().any(|public| public == *visible))
        .count()
}

fn stat_u64(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

fn validate_storage_params(params: &StorageParams) -> Result<(), ErrorData> {
    validate_exact_spec(
        STORAGE_TOOL,
        params.operation.as_str(),
        &[
            ("inspect", params.inspect.is_some()),
            ("summary", params.summary.is_some()),
            ("put_probe_rows", params.put_probe_rows.is_some()),
            ("gc_once", params.gc_once.is_some()),
        ],
    )
}

fn validate_model_params(params: &ModelParams) -> Result<(), ErrorData> {
    validate_exact_spec(
        MODEL_TOOL,
        params.operation.as_str(),
        &[
            ("list", params.list.is_some()),
            ("status", params.status.is_some()),
            ("probe", params.probe.is_some()),
            ("register", params.register.is_some()),
            ("update", params.update.is_some()),
            ("remove", params.remove.is_some()),
        ],
    )
}

fn validate_hygiene_params(params: &HygieneParams) -> Result<(), ErrorData> {
    validate_exact_spec(
        HYGIENE_TOOL,
        params.operation.as_str(),
        &[
            ("scan_text", params.scan_text.is_some()),
            ("scan_storage", params.scan_storage.is_some()),
            ("flags", params.flags.is_some()),
            ("report", params.report.is_some()),
        ],
    )
}

fn validate_setup_params(params: &SetupParams) -> Result<(), ErrorData> {
    validate_exact_spec(
        SETUP_TOOL,
        params.operation.as_str(),
        &[
            ("status", params.status.is_some()),
            ("doctor", params.doctor.is_some()),
            ("repair", params.repair.is_some()),
        ],
    )
}

fn validate_telemetry_params(params: &TelemetryParams) -> Result<(), ErrorData> {
    validate_exact_spec(
        TELEMETRY_TOOL,
        params.operation.as_str(),
        &[("status", params.status.is_some())],
    )
}

fn validate_exact_spec(
    tool: &'static str,
    operation: &'static str,
    specs: &[(&'static str, bool)],
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
        ErrorCode(-32099),
        format!("{tool} operation={operation} requires exactly one matching operation payload"),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "tool": tool,
            "operation": operation,
            "source_of_truth": "MCP request parameters",
            "source_id": operation,
            "matching_payload_present": matching_present,
            "extra_payloads": extra,
            "remediation": "pass exactly one payload object whose key matches operation",
        })),
    ))
}

fn missing_spec(tool: &'static str, operation: &'static str) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!("{tool} operation={operation} missing operation payload"),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "tool": tool,
            "operation": operation,
            "source_of_truth": "MCP request parameters",
            "source_id": operation,
            "remediation": "pass the payload object matching operation",
        })),
    )
}

fn facade_policy_error(
    tool: &'static str,
    operation: &'static str,
    source_id: &str,
    profile: ToolProfileKind,
    source_of_truth: &'static str,
    remediation: &'static str,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "{tool} operation={operation} is not allowed for profile {}",
            profile.as_str()
        ),
        Some(json!({
            "code": error_codes::TOOL_PROFILE_POLICY_DENIED,
            "tool": tool,
            "operation": operation,
            "source_id": source_id,
            "profile": profile.as_str(),
            "source_of_truth": source_of_truth,
            "remediation": remediation,
        })),
    )
}

fn facade_delegate_error(
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

fn error_code_from(error: &ErrorData) -> String {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
        .unwrap_or(error_codes::TOOL_INTERNAL_ERROR)
        .to_owned()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_requires_matching_storage_payload_only() {
        let ok = StorageParams {
            operation: StorageOperation::Inspect,
            inspect: Some(StorageInspectParams::default()),
            summary: None,
            put_probe_rows: None,
            gc_once: None,
        };
        validate_storage_params(&ok).expect("matching payload accepted");

        let missing = StorageParams {
            operation: StorageOperation::Inspect,
            inspect: None,
            summary: None,
            put_probe_rows: None,
            gc_once: None,
        };
        validate_storage_params(&missing).expect_err("missing payload rejected");

        let extra = StorageParams {
            operation: StorageOperation::Inspect,
            inspect: Some(StorageInspectParams::default()),
            summary: Some(StorageInspectParams::default()),
            put_probe_rows: None,
            gc_once: None,
        };
        validate_storage_params(&extra).expect_err("extra payload rejected");
    }

    #[test]
    fn telemetry_operation_stays_strict() {
        let ok = TelemetryParams {
            operation: TelemetryOperation::Status,
            status: Some(TelemetryStatusParams::default()),
        };
        validate_telemetry_params(&ok).expect("status payload accepted");
        let missing = TelemetryParams {
            operation: TelemetryOperation::Status,
            status: None,
        };
        validate_telemetry_params(&missing).expect_err("status payload required");
    }

    #[test]
    fn model_status_counts_probe_health() {
        let list = LocalModelListResponse {
            schema_version: synapse_core::SCHEMA_VERSION,
            source_of_truth: "test".to_owned(),
            scanned_rows: 2,
            rows: vec![
                crate::m3::local_models::LocalModelRegistryRow {
                    schema_version: synapse_core::SCHEMA_VERSION,
                    row_key: "r1".to_owned(),
                    name: "a".to_owned(),
                    base_url: "http://127.0.0.1:1/v1".to_owned(),
                    model_id: "m".to_owned(),
                    api_shape: crate::m3::local_models::LocalModelApiShape::default(),
                    runtime_preset: crate::m3::local_models::LocalModelRuntimePreset::default(),
                    context_length: None,
                    max_tools: None,
                    notes: None,
                    enabled: true,
                    allow_non_loopback: false,
                    api_key_env_var: None,
                    created_at_unix_ms: 1,
                    updated_at_unix_ms: 1,
                    created_by_session: "test".to_owned(),
                    updated_by_session: "test".to_owned(),
                    last_probe: Some(crate::m3::local_models::LocalModelProbeReport {
                        schema_version: synapse_core::SCHEMA_VERSION,
                        observed_at_unix_ms: 1,
                        endpoint_url: "http://127.0.0.1:1/v1/chat/completions".to_owned(),
                        healthy: true,
                        status: "ok".to_owned(),
                        latency_ms: 1,
                        tokens_per_second: Some(1.0),
                        prompt_tokens: Some(1),
                        completion_tokens: Some(1),
                        total_tokens: Some(2),
                        error_code: None,
                        error_phase: None,
                        error_kind: None,
                        error_detail: None,
                        raw_response_sha256: Some("sha256:x".to_owned()),
                        raw_response_excerpt: None,
                        raw_response_truncated: false,
                    }),
                    has_api_key_secret: false,
                },
                crate::m3::local_models::LocalModelRegistryRow {
                    schema_version: synapse_core::SCHEMA_VERSION,
                    row_key: "r2".to_owned(),
                    name: "b".to_owned(),
                    base_url: "http://127.0.0.1:2/v1".to_owned(),
                    model_id: "m".to_owned(),
                    api_shape: crate::m3::local_models::LocalModelApiShape::default(),
                    runtime_preset: crate::m3::local_models::LocalModelRuntimePreset::default(),
                    context_length: None,
                    max_tools: None,
                    notes: None,
                    enabled: false,
                    allow_non_loopback: false,
                    api_key_env_var: None,
                    created_at_unix_ms: 1,
                    updated_at_unix_ms: 1,
                    created_by_session: "test".to_owned(),
                    updated_by_session: "test".to_owned(),
                    last_probe: None,
                    has_api_key_secret: true,
                },
            ],
            corrupt_rows: Vec::new(),
        };
        let status = model_status(&list);
        assert_eq!(status.visible_rows, 2);
        assert_eq!(status.enabled_rows, 1);
        assert_eq!(status.disabled_rows, 1);
        assert_eq!(status.probed_rows, 1);
        assert_eq!(status.healthy_rows, 1);
        assert_eq!(status.rows_with_api_key_secret, 1);
    }
}
