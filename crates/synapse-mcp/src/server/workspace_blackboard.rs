//! Run-scoped shared workspace blackboard tools (#796).
//!
//! The blackboard is the cooperative data plane for primary agents. Durable
//! truth is a `CF_KV` row keyed by run id + structured workspace key; SSE is
//! only the notification path.

use std::{
    fs::File,
    io::Read as _,
    path::Path,
    sync::{
        Arc, LazyLock, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::Utc;
use rmcp::{RoleServer, model::ErrorCode, service::RequestContext};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use synapse_core::{DataPredicate, Event, EventFilter, EventSource, error_codes};
use synapse_reflex::PublishReport;
use synapse_storage::{Db, cf};

use super::{
    ErrorData, Json, Parameters, SynapseService, mcp_error, session_registry::unix_time_ms_now,
    session_tools::validate_session_id, tool, tool_router,
};

const SCHEMA_VERSION: u32 = 1;
const WORKSPACE_PREFIX: &str = "workspace-blackboard/v1";
const WORKSPACE_PUT_EVENT_KIND: &str = "workspace.put";
const DEFAULT_WORKSPACE_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_WORKSPACE_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const DEFAULT_LIST_LIMIT: usize = 100;
const MAX_LIST_LIMIT: usize = 1000;
const MAX_RUN_ID_BYTES: usize = 128;
const MAX_KEY_BYTES: usize = 512;
const MAX_INLINE_VALUE_BYTES: usize = 256 * 1024;
const MAX_ARTIFACT_HANDLE_CHARS: usize = 1024;
const MAX_ARTIFACT_TEXT_CHARS: usize = 512;

static NEXT_WORKSPACE_EVENT_SEQ: AtomicU64 = AtomicU64::new(1);
static WORKSPACE_WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePutParams {
    /// Optional logical run id. Defaults to the current daemon lifecycle run id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Structured key such as "findings/page-1/text" or "artifacts/shot-1".
    pub key: String,
    /// Required when replacing an existing key. Omit to create only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<u64>,
    /// Small JSON value to store inline. Omit when publishing only an artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// Optional large-artifact handle. If `path` is supplied, the file is read
    /// and size/hash are verified before the row is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<WorkspaceArtifactRef>,
    /// Retention in milliseconds. Expired rows are removed on get/list/put.
    #[serde(default = "default_workspace_ttl_ms")]
    #[schemars(
        default = "default_workspace_ttl_ms",
        range(min = 1, max = 604_800_000)
    )]
    pub ttl_ms: u64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceGetParams {
    /// Optional logical run id. Defaults to the current daemon lifecycle run id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub key: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceListParams {
    /// Optional logical run id. Defaults to the current daemon lifecycle run id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Key prefix to return. Empty means the whole run namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default = "default_list_limit")]
    #[schemars(default = "default_list_limit", range(min = 1, max = 1000))]
    pub limit: usize,
    /// Include inline JSON values in entries. Set false for a metadata-only scan.
    #[serde(default = "default_true")]
    #[schemars(default = "default_true")]
    pub include_values: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSubscribeParams {
    /// Optional logical run id. Defaults to the current daemon lifecycle run id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Non-empty key prefix, for example "findings/".
    pub prefix: String,
    #[serde(default = "default_false")]
    #[schemars(default = "default_false")]
    pub snapshot_first: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceArtifactRef {
    pub handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_len: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceEntry {
    pub schema_version: u32,
    pub run_id: String,
    pub key: String,
    pub row_key: String,
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<WorkspaceArtifactRef>,
    pub writer_session_id: String,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    pub ttl_ms: u64,
    pub expires_at_unix_ms: u64,
    pub version: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRowReadback {
    pub cf_name: String,
    pub row_key: String,
    pub value_len_bytes: u64,
    pub value_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceArtifactReadback {
    pub path: String,
    pub exists: bool,
    pub is_file: bool,
    pub bytes_len: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceEventPublishReport {
    pub event_kind: String,
    pub event_seq: u64,
    pub matched: usize,
    pub queued: usize,
    pub dropped: u64,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePutResponse {
    pub ok: bool,
    pub run_id: String,
    pub key: String,
    pub row_key: String,
    pub writer_session_id: String,
    pub version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<u64>,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub expired_rows_deleted_before: usize,
    pub corrupt_rows_skipped_before: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_readback: Option<WorkspaceArtifactReadback>,
    pub storage_readback: WorkspaceRowReadback,
    pub event_publish_report: WorkspaceEventPublishReport,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceGetResponse {
    pub ok: bool,
    pub run_id: String,
    pub key: String,
    pub now_unix_ms: u64,
    pub entry: WorkspaceEntry,
    pub storage_readback: WorkspaceRowReadback,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceListResponse {
    pub ok: bool,
    pub run_id: String,
    pub prefix: String,
    pub values_included: bool,
    pub now_unix_ms: u64,
    pub scanned_rows: usize,
    pub expired_rows_deleted: usize,
    pub corrupt_rows_skipped: Vec<WorkspaceCorruptRow>,
    pub returned_count: usize,
    pub entries: Vec<WorkspaceEntry>,
    pub readback_rows: Vec<WorkspaceRowReadback>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCorruptRow {
    pub row_key: String,
    pub value_len_bytes: u64,
    pub value_sha256: String,
    pub error: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSubscribeResponse {
    pub ok: bool,
    pub subscription_id: String,
    pub run_id: String,
    pub prefix: String,
    pub event_kind: String,
    pub started_at_unix_ms: u64,
}

#[derive(Clone)]
struct DecodedWorkspaceRow {
    key: Vec<u8>,
    encoded: Vec<u8>,
    entry: WorkspaceEntry,
}

#[derive(Default)]
struct WorkspaceCleanupReport {
    expired_keys: Vec<Vec<u8>>,
    corrupt_rows: Vec<WorkspaceCorruptRow>,
}

#[tool_router(router = workspace_blackboard_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Publish one run-scoped blackboard entry into durable CF_KV storage, with optional inline JSON and artifact handle. Creates fail closed if the key already exists unless expected_version matches the current row. If artifact.path is provided, Synapse reads the file and verifies size/hash before accepting the row. The write is accepted only after an exact CF_KV row readback, then a workspace.put SSE event is published."
    )]
    pub async fn workspace_put(
        &self,
        params: Parameters<WorkspacePutParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<WorkspacePutResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "workspace_put",
            "tool.invocation kind=workspace_put"
        );
        let session_id = require_workspace_session_id("workspace_put", &request_context)?;
        self.workspace_put_impl(params.0, &session_id).map(Json)
    }

    #[tool(
        description = "Read one run-scoped blackboard entry by key from durable CF_KV storage. Missing, expired, or corrupt exact rows fail closed with structured error data; the returned entry includes exact row hash readback."
    )]
    pub async fn workspace_get(
        &self,
        params: Parameters<WorkspaceGetParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<WorkspaceGetResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "workspace_get",
            "tool.invocation kind=workspace_get"
        );
        let session_id = require_workspace_session_id("workspace_get", &request_context)?;
        self.workspace_get_impl(params.0, &session_id).map(Json)
    }

    #[tool(
        description = "List run-scoped blackboard entries from durable CF_KV storage. Scans are isolated: one corrupt row is reported and skipped rather than breaking reads of other keys."
    )]
    pub async fn workspace_list(
        &self,
        params: Parameters<WorkspaceListParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<WorkspaceListResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "workspace_list",
            "tool.invocation kind=workspace_list"
        );
        let session_id = require_workspace_session_id("workspace_list", &request_context)?;
        self.workspace_list_impl(params.0, &session_id).map(Json)
    }

    #[tool(
        description = "Create a per-session SSE subscription for workspace.put events in one run and key prefix. The response returns the subscription id; read it through the HTTP SSE events endpoint."
    )]
    pub async fn workspace_subscribe(
        &self,
        params: Parameters<WorkspaceSubscribeParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<WorkspaceSubscribeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "workspace_subscribe",
            "tool.invocation kind=workspace_subscribe"
        );
        let session_id = require_workspace_session_id("workspace_subscribe", &request_context)?;
        self.workspace_subscribe_impl(params.0, &session_id)
            .map(Json)
    }
}

impl SynapseService {
    fn workspace_put_impl(
        &self,
        params: WorkspacePutParams,
        writer_session_id: &str,
    ) -> Result<WorkspacePutResponse, ErrorData> {
        self.workspace_put_impl_at(params, writer_session_id, unix_time_ms_now())
    }

    fn workspace_put_impl_at(
        &self,
        params: WorkspacePutParams,
        writer_session_id: &str,
        now_unix_ms: u64,
    ) -> Result<WorkspacePutResponse, ErrorData> {
        validate_session_id(writer_session_id)?;
        let request_params = params.clone();
        let run_id = resolve_workspace_run_id(params.run_id.as_deref())?;
        let key = normalize_workspace_key(&params.key)?;
        let expected_version = params.expected_version;
        validate_workspace_ttl_ms(params.ttl_ms)?;
        validate_inline_value_size(params.value.as_ref())?;
        if params.value.is_none() && params.artifact.is_none() {
            return Err(params_error(
                "workspace_put requires at least one of value or artifact",
            ));
        }
        let (artifact, artifact_readback) = match params.artifact {
            Some(artifact) => {
                let (normalized, readback) = validate_workspace_artifact(artifact)?;
                (Some(normalized), readback)
            }
            None => (None, None),
        };

        let _write_guard = workspace_write_lock()?;
        let db = self.workspace_db()?;
        let cleanup = cleanup_expired_workspace_rows(&db, &run_id, now_unix_ms)?;
        delete_workspace_rows(
            &db,
            cleanup.expired_keys.clone(),
            "delete expired workspace rows",
        )?;

        let row_key = workspace_row_key(&run_id, &key);
        let existing = read_workspace_row_optional(&db, &row_key, now_unix_ms)?;
        let previous_version = existing.as_ref().map(|row| row.entry.version);
        validate_workspace_expected_version(
            expected_version,
            previous_version,
            &run_id,
            &key,
            &row_key,
        )?;
        let (created_at_unix_ms, version) = existing.as_ref().map_or((now_unix_ms, 1), |row| {
            (
                row.entry.created_at_unix_ms,
                row.entry.version.saturating_add(1),
            )
        });
        let command_payload = json!({
            "run_id": &request_params.run_id,
            "resolved_run_id": &run_id,
            "key": &request_params.key,
            "normalized_key": &key,
            "expected_version": request_params.expected_version,
            "value": &request_params.value,
            "artifact": &request_params.artifact,
            "ttl_ms": request_params.ttl_ms,
        });
        let command_before = json!({
            "source_of_truth": cf::CF_KV,
            "row_key": &row_key,
            "had_existing_row": existing.is_some(),
            "previous_version": previous_version,
            "expired_rows_deleted_before": cleanup.expired_keys.len(),
            "corrupt_rows_skipped_before": cleanup.corrupt_rows.len(),
        });
        self.command_audit_intent(super::command_audit::CommandAuditInput::mcp(
            "workspace_put",
            "plan_edit",
            Some(writer_session_id.to_owned()),
            Some(writer_session_id.to_owned()),
            command_payload.clone(),
            command_before.clone(),
            Value::Null,
            "pending",
        ))?;
        let entry = WorkspaceEntry {
            schema_version: SCHEMA_VERSION,
            run_id: run_id.clone(),
            key: key.clone(),
            row_key: row_key.clone(),
            value: params.value,
            artifact,
            writer_session_id: writer_session_id.to_owned(),
            created_at_unix_ms,
            updated_at_unix_ms: now_unix_ms,
            ttl_ms: params.ttl_ms,
            expires_at_unix_ms: now_unix_ms.saturating_add(params.ttl_ms),
            version,
        };
        let encoded = encode_workspace_entry(&entry)?;
        db.put_batch_pressure_bypass(cf::CF_KV, [(row_key.as_bytes().to_vec(), encoded)])
            .map_err(|error| {
                mcp_error(
                    error.code(),
                    format!("write workspace blackboard row {row_key}: {error}"),
                )
            })?;
        let storage_readback = readback_exact_workspace_row(&db, &row_key)?;
        let event_publish_report = match self.publish_workspace_put_event(&entry, &storage_readback)
        {
            Ok(report) => report,
            Err(error) => {
                self.command_audit_final(
                    super::command_audit::CommandAuditInput::mcp(
                        "workspace_put",
                        "plan_edit",
                        Some(writer_session_id.to_owned()),
                        Some(writer_session_id.to_owned()),
                        command_payload,
                        command_before,
                        json!({
                            "source_of_truth": cf::CF_KV,
                            "row_key": &row_key,
                            "version": version,
                            "storage_readback": &storage_readback,
                        }),
                        "error",
                    )
                    .with_error(
                        super::command_audit::command_audit_error_from_error_data(&error),
                    ),
                )?;
                return Err(error);
            }
        };

        tracing::info!(
            code = "WORKSPACE_BLACKBOARD_PUT_COMMITTED",
            run_id,
            key,
            row_key,
            writer_session_id,
            version,
            value_sha256 = %storage_readback.value_sha256,
            event_matched = event_publish_report.matched,
            event_queued = event_publish_report.queued,
            expired_rows_deleted_before = cleanup.expired_keys.len(),
            corrupt_rows_skipped_before = cleanup.corrupt_rows.len(),
            "readback=workspace_blackboard edge=put_committed"
        );

        let response = WorkspacePutResponse {
            ok: true,
            run_id,
            key,
            row_key,
            writer_session_id: writer_session_id.to_owned(),
            version,
            previous_version,
            created_at_unix_ms,
            updated_at_unix_ms: now_unix_ms,
            expires_at_unix_ms: entry.expires_at_unix_ms,
            expired_rows_deleted_before: cleanup.expired_keys.len(),
            corrupt_rows_skipped_before: cleanup.corrupt_rows.len(),
            artifact_readback,
            storage_readback,
            event_publish_report,
        };
        self.command_audit_final(super::command_audit::CommandAuditInput::mcp(
            "workspace_put",
            "plan_edit",
            Some(writer_session_id.to_owned()),
            Some(writer_session_id.to_owned()),
            command_payload,
            command_before,
            json!({
                "source_of_truth": cf::CF_KV,
                "row_key": &response.row_key,
                "version": response.version,
                "previous_version": response.previous_version,
                "storage_readback": &response.storage_readback,
                "event_publish_report": &response.event_publish_report,
            }),
            "ok",
        ))?;
        Ok(response)
    }

    fn workspace_get_impl(
        &self,
        params: WorkspaceGetParams,
        session_id: &str,
    ) -> Result<WorkspaceGetResponse, ErrorData> {
        self.workspace_get_impl_at(params, session_id, unix_time_ms_now())
    }

    fn workspace_get_impl_at(
        &self,
        params: WorkspaceGetParams,
        session_id: &str,
        now_unix_ms: u64,
    ) -> Result<WorkspaceGetResponse, ErrorData> {
        validate_session_id(session_id)?;
        let run_id = resolve_workspace_run_id(params.run_id.as_deref())?;
        let key = normalize_workspace_key(&params.key)?;
        let db = self.workspace_db()?;
        let row_key = workspace_row_key(&run_id, &key);
        let Some(row) = read_workspace_row_optional(&db, &row_key, now_unix_ms)? else {
            return Err(workspace_missing_error(&run_id, &key, &row_key));
        };
        if row.entry.expires_at_unix_ms <= now_unix_ms {
            delete_workspace_rows(
                &db,
                vec![row.key.clone()],
                "delete expired workspace row on get",
            )?;
            return Err(workspace_expired_error(
                &run_id,
                &key,
                &row_key,
                row.entry.expires_at_unix_ms,
                now_unix_ms,
            ));
        }
        let storage_readback = WorkspaceRowReadback {
            cf_name: cf::CF_KV.to_owned(),
            row_key: row.entry.row_key.clone(),
            value_len_bytes: row.encoded.len() as u64,
            value_sha256: hash_bytes(&row.encoded),
        };
        tracing::info!(
            code = "WORKSPACE_BLACKBOARD_GET_READ",
            run_id,
            key,
            row_key,
            reader_session_id = session_id,
            version = row.entry.version,
            value_sha256 = %storage_readback.value_sha256,
            "readback=workspace_blackboard edge=get_read"
        );
        Ok(WorkspaceGetResponse {
            ok: true,
            run_id,
            key,
            now_unix_ms,
            entry: row.entry,
            storage_readback,
        })
    }

    fn workspace_list_impl(
        &self,
        params: WorkspaceListParams,
        session_id: &str,
    ) -> Result<WorkspaceListResponse, ErrorData> {
        self.workspace_list_impl_at(params, session_id, unix_time_ms_now())
    }

    fn workspace_list_impl_at(
        &self,
        params: WorkspaceListParams,
        session_id: &str,
        now_unix_ms: u64,
    ) -> Result<WorkspaceListResponse, ErrorData> {
        validate_session_id(session_id)?;
        let run_id = resolve_workspace_run_id(params.run_id.as_deref())?;
        let prefix =
            normalize_workspace_prefix(params.prefix.as_deref().unwrap_or_default(), true)?;
        validate_workspace_list_limit(params.limit)?;
        let db = self.workspace_db()?;
        let scan = scan_workspace_run(&db, &run_id, now_unix_ms)?;
        delete_workspace_rows(
            &db,
            scan.expired_keys.clone(),
            "delete expired workspace rows on list",
        )?;

        let mut rows = scan
            .rows
            .into_iter()
            .filter(|row| row.entry.key.starts_with(&prefix))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.entry.key.cmp(&right.entry.key));
        if rows.len() > params.limit {
            rows.truncate(params.limit);
        }
        let readback_rows = rows
            .iter()
            .map(|row| WorkspaceRowReadback {
                cf_name: cf::CF_KV.to_owned(),
                row_key: row.entry.row_key.clone(),
                value_len_bytes: row.encoded.len() as u64,
                value_sha256: hash_bytes(&row.encoded),
            })
            .collect::<Vec<_>>();
        let entries = rows
            .into_iter()
            .map(|mut row| {
                if !params.include_values {
                    row.entry.value = None;
                }
                row.entry
            })
            .collect::<Vec<_>>();

        tracing::info!(
            code = "WORKSPACE_BLACKBOARD_LIST_READ",
            run_id,
            prefix,
            reader_session_id = session_id,
            scanned_rows = scan.scanned_rows,
            expired_rows_deleted = scan.expired_keys.len(),
            corrupt_rows_skipped = scan.corrupt_rows.len(),
            returned_count = entries.len(),
            "readback=workspace_blackboard edge=list_read"
        );

        Ok(WorkspaceListResponse {
            ok: true,
            run_id,
            prefix,
            values_included: params.include_values,
            now_unix_ms,
            scanned_rows: scan.scanned_rows,
            expired_rows_deleted: scan.expired_keys.len(),
            corrupt_rows_skipped: scan.corrupt_rows,
            returned_count: entries.len(),
            entries,
            readback_rows,
        })
    }

    fn workspace_subscribe_impl(
        &self,
        params: WorkspaceSubscribeParams,
        session_id: &str,
    ) -> Result<WorkspaceSubscribeResponse, ErrorData> {
        validate_session_id(session_id)?;
        let run_id = resolve_workspace_run_id(params.run_id.as_deref())?;
        let prefix = normalize_workspace_prefix(&params.prefix, false)?;
        let filter = EventFilter::And {
            args: vec![
                EventFilter::Data {
                    path: "/run_id".to_owned(),
                    predicate: DataPredicate::Eq {
                        value: Value::String(run_id.clone()),
                    },
                },
                EventFilter::Data {
                    path: "/key".to_owned(),
                    predicate: DataPredicate::Regex {
                        pattern: format!("^{}", regex::escape(&prefix)),
                    },
                },
            ],
        };
        let started_at_unix_ms = unix_time_ms_now();
        let subscription_id = self
            .sse_state()?
            .subscribe(
                filter,
                vec![WORKSPACE_PUT_EVENT_KIND.to_owned()],
                params.snapshot_first,
                Some(session_id.to_owned()),
            )
            .map_err(|error| mcp_error(error.code(), error.message()))?;
        tracing::info!(
            code = "WORKSPACE_BLACKBOARD_SUBSCRIBE_REGISTERED",
            run_id,
            prefix,
            subscription_id,
            owner_session_id = session_id,
            "readback=workspace_blackboard edge=subscribe_registered"
        );
        Ok(WorkspaceSubscribeResponse {
            ok: true,
            subscription_id,
            run_id,
            prefix,
            event_kind: WORKSPACE_PUT_EVENT_KIND.to_owned(),
            started_at_unix_ms,
        })
    }

    fn workspace_db(&self) -> Result<Arc<Db>, ErrorData> {
        let state = self.m3_state_handle();
        let mut guard = state.lock().map_err(|_error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "M3 service state lock poisoned while opening workspace blackboard storage",
            )
        })?;
        guard
            .ensure_storage()
            .map_err(|error| mcp_error(error.code(), error.to_string()))
    }

    fn publish_workspace_put_event(
        &self,
        entry: &WorkspaceEntry,
        readback: &WorkspaceRowReadback,
    ) -> Result<WorkspaceEventPublishReport, ErrorData> {
        let event_seq = NEXT_WORKSPACE_EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
        let event = Event {
            seq: event_seq,
            at: Utc::now(),
            source: EventSource::System,
            kind: WORKSPACE_PUT_EVENT_KIND.to_owned(),
            data: json!({
                "run_id": entry.run_id,
                "key": entry.key,
                "row_key": entry.row_key,
                "writer_session_id": entry.writer_session_id,
                "version": entry.version,
                "previous_version": if entry.version > 1 { Some(entry.version - 1) } else { None },
                "updated_at_unix_ms": entry.updated_at_unix_ms,
                "expires_at_unix_ms": entry.expires_at_unix_ms,
                "has_value": entry.value.is_some(),
                "artifact_handle": entry.artifact.as_ref().map(|artifact| artifact.handle.clone()),
                "cf_name": readback.cf_name,
                "value_len_bytes": readback.value_len_bytes,
                "value_sha256": readback.value_sha256,
            }),
            correlations: Vec::new(),
        };
        let report = self.sse_state()?.event_bus().publish(event);
        Ok(workspace_publish_report(event_seq, report))
    }
}

struct WorkspaceRunScan {
    scanned_rows: usize,
    expired_keys: Vec<Vec<u8>>,
    corrupt_rows: Vec<WorkspaceCorruptRow>,
    rows: Vec<DecodedWorkspaceRow>,
}

fn scan_workspace_run(
    db: &Db,
    run_id: &str,
    now_unix_ms: u64,
) -> Result<WorkspaceRunScan, ErrorData> {
    let prefix = workspace_run_prefix(run_id);
    let rows = db
        .scan_cf_prefix(cf::CF_KV, prefix.as_bytes())
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    let scanned_rows = rows.len();
    let mut expired_keys = Vec::new();
    let mut corrupt_rows = Vec::new();
    let mut decoded_rows = Vec::new();
    for (key, encoded) in rows {
        match decode_workspace_row(key.clone(), encoded.clone()) {
            Ok(row) if row.entry.expires_at_unix_ms <= now_unix_ms => expired_keys.push(key),
            Ok(row) => decoded_rows.push(row),
            Err(error) => corrupt_rows.push(WorkspaceCorruptRow {
                row_key: String::from_utf8_lossy(&key).to_string(),
                value_len_bytes: encoded.len() as u64,
                value_sha256: hash_bytes(&encoded),
                error,
            }),
        }
    }
    Ok(WorkspaceRunScan {
        scanned_rows,
        expired_keys,
        corrupt_rows,
        rows: decoded_rows,
    })
}

fn cleanup_expired_workspace_rows(
    db: &Db,
    run_id: &str,
    now_unix_ms: u64,
) -> Result<WorkspaceCleanupReport, ErrorData> {
    let scan = scan_workspace_run(db, run_id, now_unix_ms)?;
    Ok(WorkspaceCleanupReport {
        expired_keys: scan.expired_keys,
        corrupt_rows: scan.corrupt_rows,
    })
}

fn read_workspace_row_optional(
    db: &Db,
    row_key: &str,
    _now_unix_ms: u64,
) -> Result<Option<DecodedWorkspaceRow>, ErrorData> {
    let row = db
        .scan_cf_prefix(cf::CF_KV, row_key.as_bytes())
        .map_err(|error| mcp_error(error.code(), error.to_string()))?
        .into_iter()
        .find(|(key, _value)| key == row_key.as_bytes());
    let Some((key, encoded)) = row else {
        return Ok(None);
    };
    decode_workspace_row(key, encoded)
        .map(Some)
        .map_err(|error| workspace_corrupt_error(row_key, error))
}

fn decode_workspace_row(key: Vec<u8>, encoded: Vec<u8>) -> Result<DecodedWorkspaceRow, String> {
    let row_key = String::from_utf8_lossy(&key).to_string();
    let entry: WorkspaceEntry = synapse_storage::decode_json(&encoded)
        .map_err(|error| format!("decode workspace row {row_key}: {error}"))?;
    if entry.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "workspace row {row_key} has schema_version {}, expected {SCHEMA_VERSION}",
            entry.schema_version
        ));
    }
    if entry.row_key != row_key {
        return Err(format!(
            "workspace row key mismatch: stored entry.row_key={} actual={row_key}",
            entry.row_key
        ));
    }
    if entry.run_id.trim().is_empty() || entry.key.trim().is_empty() {
        return Err(format!(
            "workspace row {row_key} has empty run_id or key fields"
        ));
    }
    Ok(DecodedWorkspaceRow {
        key,
        encoded,
        entry,
    })
}

fn readback_exact_workspace_row(db: &Db, row_key: &str) -> Result<WorkspaceRowReadback, ErrorData> {
    let stored = db
        .scan_cf_prefix(cf::CF_KV, row_key.as_bytes())
        .map_err(|error| mcp_error(error.code(), error.to_string()))?
        .into_iter()
        .find_map(|(key, value)| (key == row_key.as_bytes()).then_some(value))
        .ok_or_else(|| {
            mcp_error(
                error_codes::STORAGE_READ_FAILED,
                format!("workspace blackboard row missing after write: {row_key}"),
            )
        })?;
    Ok(WorkspaceRowReadback {
        cf_name: cf::CF_KV.to_owned(),
        row_key: row_key.to_owned(),
        value_len_bytes: stored.len() as u64,
        value_sha256: hash_bytes(&stored),
    })
}

fn encode_workspace_entry(entry: &WorkspaceEntry) -> Result<Vec<u8>, ErrorData> {
    synapse_storage::encode_json(entry).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_WRITE_FAILED,
            format!("encode workspace blackboard entry: {error}"),
        )
    })
}

fn delete_workspace_rows(
    db: &Db,
    keys: Vec<Vec<u8>>,
    operation: &'static str,
) -> Result<(), ErrorData> {
    if keys.is_empty() {
        return Ok(());
    }
    db.delete_batch(cf::CF_KV, keys)
        .map_err(|error| mcp_error(error.code(), format!("{operation}: {error}")))
}

fn workspace_write_lock() -> Result<MutexGuard<'static, ()>, ErrorData> {
    WORKSPACE_WRITE_LOCK.lock().map_err(|_error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "workspace blackboard write lock poisoned",
        )
    })
}

fn validate_workspace_expected_version(
    expected_version: Option<u64>,
    current_version: Option<u64>,
    run_id: &str,
    key: &str,
    row_key: &str,
) -> Result<(), ErrorData> {
    match (expected_version, current_version) {
        (None, None) => Ok(()),
        (Some(expected), Some(current)) if expected == current => Ok(()),
        (expected, current) => Err(workspace_version_conflict_error(
            run_id, key, row_key, expected, current,
        )),
    }
}

fn resolve_workspace_run_id(raw: Option<&str>) -> Result<String, ErrorData> {
    let value = match raw {
        Some(value) => value.trim().to_owned(),
        None => crate::daemon_lifecycle::current_run_id().ok_or_else(|| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "workspace blackboard requires daemon lifecycle run_id; pass run_id explicitly in tests or start the configured daemon",
            )
        })?,
    };
    validate_workspace_run_id(&value)?;
    Ok(value)
}

fn validate_workspace_run_id(value: &str) -> Result<(), ErrorData> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_RUN_ID_BYTES {
        return Err(params_error(format!(
            "workspace run_id must be non-empty and <= {MAX_RUN_ID_BYTES} bytes"
        )));
    }
    if !trimmed.chars().all(|ch| !ch.is_control()) {
        return Err(params_error(
            "workspace run_id must not contain control characters",
        ));
    }
    Ok(())
}

fn normalize_workspace_key(value: &str) -> Result<String, ErrorData> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_KEY_BYTES {
        return Err(params_error(format!(
            "workspace key must be non-empty and <= {MAX_KEY_BYTES} bytes"
        )));
    }
    if !trimmed.chars().all(|ch| !ch.is_control()) {
        return Err(params_error(
            "workspace key must not contain control characters",
        ));
    }
    Ok(trimmed.to_owned())
}

fn normalize_workspace_prefix(value: &str, allow_empty: bool) -> Result<String, ErrorData> {
    let trimmed = value.trim();
    if !allow_empty && trimmed.is_empty() {
        return Err(params_error("workspace prefix must not be empty"));
    }
    if trimmed.len() > MAX_KEY_BYTES {
        return Err(params_error(format!(
            "workspace prefix must be <= {MAX_KEY_BYTES} bytes"
        )));
    }
    if !trimmed.chars().all(|ch| !ch.is_control()) {
        return Err(params_error(
            "workspace prefix must not contain control characters",
        ));
    }
    Ok(trimmed.to_owned())
}

fn validate_workspace_ttl_ms(ttl_ms: u64) -> Result<(), ErrorData> {
    if ttl_ms == 0 || ttl_ms > MAX_WORKSPACE_TTL_MS {
        return Err(params_error(format!(
            "workspace ttl_ms must be between 1 and {MAX_WORKSPACE_TTL_MS}"
        )));
    }
    Ok(())
}

fn validate_workspace_list_limit(limit: usize) -> Result<(), ErrorData> {
    if limit == 0 || limit > MAX_LIST_LIMIT {
        return Err(params_error(format!(
            "workspace_list limit must be between 1 and {MAX_LIST_LIMIT}"
        )));
    }
    Ok(())
}

fn validate_inline_value_size(value: Option<&Value>) -> Result<(), ErrorData> {
    let Some(value) = value else {
        return Ok(());
    };
    let encoded = synapse_storage::encode_json(value).map_err(|error| {
        mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("workspace_put value must be JSON-encodable: {error}"),
        )
    })?;
    if encoded.len() > MAX_INLINE_VALUE_BYTES {
        return Err(params_error(format!(
            "workspace_put value must encode to <= {MAX_INLINE_VALUE_BYTES} bytes; got {}",
            encoded.len()
        )));
    }
    Ok(())
}

fn validate_workspace_artifact(
    artifact: WorkspaceArtifactRef,
) -> Result<(WorkspaceArtifactRef, Option<WorkspaceArtifactReadback>), ErrorData> {
    let handle = normalize_artifact_text(
        artifact.handle,
        "workspace artifact handle",
        MAX_ARTIFACT_HANDLE_CHARS,
    )?;
    let path = normalize_optional_artifact_text(artifact.path, "workspace artifact path")?;
    let media_type =
        normalize_optional_artifact_text(artifact.media_type, "workspace artifact media_type")?;
    let kind = normalize_optional_artifact_text(artifact.kind, "workspace artifact kind")?;
    let sha256 = artifact.sha256.map(normalize_sha256).transpose()?;
    let bytes_len = artifact.bytes_len;
    let readback = if let Some(path_value) = path.as_deref() {
        let readback = readback_artifact_path(path_value)?;
        if let Some(expected_len) = bytes_len
            && expected_len != readback.bytes_len
        {
            return Err(params_error(format!(
                "workspace artifact bytes_len mismatch for {path_value}: expected {expected_len}, read {}",
                readback.bytes_len
            )));
        }
        if let Some(expected_sha) = sha256.as_deref()
            && expected_sha != readback.sha256
        {
            return Err(params_error(format!(
                "workspace artifact sha256 mismatch for {path_value}: expected {expected_sha}, read {}",
                readback.sha256
            )));
        }
        Some(readback)
    } else {
        None
    };

    Ok((
        WorkspaceArtifactRef {
            handle,
            path,
            media_type,
            kind,
            sha256,
            bytes_len,
        },
        readback,
    ))
}

fn normalize_artifact_text(
    value: String,
    field: &'static str,
    max_chars: usize,
) -> Result<String, ErrorData> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(params_error(format!("{field} must not be empty")));
    }
    if trimmed.chars().count() > max_chars {
        return Err(params_error(format!(
            "{field} must be at most {max_chars} Unicode scalar values"
        )));
    }
    if !trimmed.chars().all(|ch| !ch.is_control()) {
        return Err(params_error(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(trimmed.to_owned())
}

fn normalize_optional_artifact_text(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<String>, ErrorData> {
    value
        .map(|value| normalize_artifact_text(value, field, MAX_ARTIFACT_TEXT_CHARS))
        .transpose()
}

fn normalize_sha256(value: String) -> Result<String, ErrorData> {
    let trimmed = value.trim().to_ascii_lowercase();
    let hex = trimmed.strip_prefix("sha256:").unwrap_or(&trimmed);
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(params_error(
            "workspace artifact sha256 must be a 64-char hex digest or sha256:<hex>",
        ));
    }
    Ok(format!("sha256:{hex}"))
}

fn readback_artifact_path(path_value: &str) -> Result<WorkspaceArtifactReadback, ErrorData> {
    let path = Path::new(path_value);
    let metadata = path.metadata().map_err(|error| {
        params_error(format!(
            "workspace artifact path {} is not readable: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(params_error(format!(
            "workspace artifact path {} must be a file",
            path.display()
        )));
    }
    let sha256 = sha256_file(path)?;
    Ok(WorkspaceArtifactReadback {
        path: path.display().to_string(),
        exists: true,
        is_file: true,
        bytes_len: metadata.len(),
        sha256,
    })
}

fn sha256_file(path: &Path) -> Result<String, ErrorData> {
    let mut file = File::open(path).map_err(|error| {
        params_error(format!(
            "workspace artifact path {} could not be opened for hashing: {error}",
            path.display()
        ))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            params_error(format!(
                "workspace artifact path {} could not be read for hashing: {error}",
                path.display()
            ))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{}", hex_bytes(&hasher.finalize())))
}

fn workspace_publish_report(event_seq: u64, report: PublishReport) -> WorkspaceEventPublishReport {
    WorkspaceEventPublishReport {
        event_kind: WORKSPACE_PUT_EVENT_KIND.to_owned(),
        event_seq,
        matched: report.matched,
        queued: report.queued,
        dropped: report.dropped,
    }
}

fn workspace_missing_error(run_id: &str, key: &str, row_key: &str) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!("workspace blackboard key {key:?} was not found for run {run_id:?}"),
        Some(json!({
            "code": error_codes::STORAGE_READ_FAILED,
            "run_id": run_id,
            "key": key,
            "row_key": row_key,
            "source_of_truth": "CF_KV workspace-blackboard exact row",
        })),
    )
}

fn workspace_expired_error(
    run_id: &str,
    key: &str,
    row_key: &str,
    expires_at_unix_ms: u64,
    now_unix_ms: u64,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!("workspace blackboard key {key:?} for run {run_id:?} expired and was deleted"),
        Some(json!({
            "code": error_codes::STORAGE_READ_FAILED,
            "detail_code": "WORKSPACE_ROW_EXPIRED",
            "run_id": run_id,
            "key": key,
            "row_key": row_key,
            "expires_at_unix_ms": expires_at_unix_ms,
            "now_unix_ms": now_unix_ms,
            "source_of_truth": "CF_KV workspace-blackboard exact row",
        })),
    )
}

fn workspace_version_conflict_error(
    run_id: &str,
    key: &str,
    row_key: &str,
    expected_version: Option<u64>,
    current_version: Option<u64>,
) -> ErrorData {
    let detail = match (expected_version, current_version) {
        (None, Some(current)) => {
            format!(
                "key already exists at version {current}; read it first and retry with expected_version={current}"
            )
        }
        (Some(expected), None) => {
            format!("expected_version={expected} was supplied, but the key does not exist")
        }
        (Some(expected), Some(current)) => {
            format!("expected_version={expected} did not match current version {current}")
        }
        (None, None) => "no version conflict".to_owned(),
    };
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "workspace blackboard key {key:?} for run {run_id:?} version precondition failed: {detail}"
        ),
        Some(json!({
            "code": error_codes::STORAGE_WRITE_FAILED,
            "detail_code": "WORKSPACE_VERSION_CONFLICT",
            "run_id": run_id,
            "key": key,
            "row_key": row_key,
            "expected_version": expected_version,
            "current_version": current_version,
            "source_of_truth": "CF_KV workspace-blackboard exact row",
        })),
    )
}

fn workspace_corrupt_error(row_key: &str, detail: String) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!("workspace blackboard row {row_key} is corrupt: {detail}"),
        Some(json!({
            "code": error_codes::STORAGE_CORRUPTED,
            "row_key": row_key,
            "detail": detail,
            "source_of_truth": "CF_KV workspace-blackboard exact row",
        })),
    )
}

fn params_error(message: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, message.into())
}

fn require_workspace_session_id(
    tool_name: &str,
    request_context: &RequestContext<RoleServer>,
) -> Result<String, ErrorData> {
    super::context::mcp_session_id_from_request_context(request_context)?.ok_or_else(|| {
        mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!(
                "{tool_name} requires an MCP session id (run the daemon in HTTP mode so each agent has its own Mcp-Session-Id)"
            ),
        )
    })
}

fn workspace_run_prefix(run_id: &str) -> String {
    format!(
        "{WORKSPACE_PREFIX}/run_hex/{}/key_hex/",
        hex_bytes(run_id.as_bytes())
    )
}

fn workspace_row_key(run_id: &str, key: &str) -> String {
    format!(
        "{}{}",
        workspace_run_prefix(run_id),
        hex_bytes(key.as_bytes())
    )
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex_bytes(&digest))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

const fn default_workspace_ttl_ms() -> u64 {
    DEFAULT_WORKSPACE_TTL_MS
}

const fn default_list_limit() -> usize {
    DEFAULT_LIST_LIMIT
}

const fn default_true() -> bool {
    true
}

const fn default_false() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, path::Path};

    use serde_json::json;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{m2::M2ServiceConfig, m3::M3ServiceConfig, m4::M4ServiceConfig};

    fn service_with_db(path: &Path) -> anyhow::Result<SynapseService> {
        SynapseService::try_with_m2_shutdown_reason_and_m3_config(
            CancellationToken::new(),
            "test",
            CancellationToken::new(),
            &M2ServiceConfig::default(),
            M3ServiceConfig::from_cli_parts(
                Some(path.join("db")),
                Some(path.to_path_buf()),
                false,
                "127.0.0.1:0".to_owned(),
                NonZeroUsize::new(8)
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

    fn error_code(error: &rmcp::ErrorData) -> Option<&str> {
        error
            .data
            .as_ref()
            .and_then(|data| data.get("code"))
            .and_then(Value::as_str)
    }

    fn error_detail_code(error: &rmcp::ErrorData) -> Option<&str> {
        error
            .data
            .as_ref()
            .and_then(|data| data.get("detail_code"))
            .and_then(Value::as_str)
    }

    #[test]
    fn put_records_command_audit_rows_readable_by_snapshot() -> anyhow::Result<()> {
        let temp = TempDir::new()?;
        let service = service_with_db(temp.path())?;

        let put = service.workspace_put_impl_at(
            WorkspacePutParams {
                run_id: Some("run-audit".to_owned()),
                key: "plans/next-step".to_owned(),
                expected_version: None,
                value: Some(json!({"known": "audit-happy"})),
                artifact: None,
                ttl_ms: 60_000,
            },
            "session-audit",
            9_000,
        )?;
        assert_eq!(put.version, 1);

        let snapshot = service.command_audit_snapshot()?;
        assert!(
            snapshot.rows.iter().any(|row| {
                row.tool == "workspace_put"
                    && row.verb == "plan_edit"
                    && row.phase == "intent"
                    && row.actor_session_id.as_deref() == Some("session-audit")
            }),
            "workspace_put intent row should be projected from CF_ACTION_LOG"
        );
        assert!(
            snapshot.rows.iter().any(|row| {
                row.tool == "workspace_put"
                    && row.verb == "plan_edit"
                    && row.phase == "final"
                    && row.outcome == "ok"
                    && row.actor_session_id.as_deref() == Some("session-audit")
            }),
            "workspace_put final row should be projected from CF_ACTION_LOG"
        );
        Ok(())
    }

    #[test]
    fn put_fails_before_storage_write_when_command_audit_intent_fails() -> anyhow::Result<()> {
        let temp = TempDir::new()?;
        let service = service_with_db(temp.path())?;

        crate::server::command_audit::set_command_audit_force_fail_for_tests(true);
        let result = service.workspace_put_impl_at(
            WorkspacePutParams {
                run_id: Some("run-audit-fail".to_owned()),
                key: "plans/blocked".to_owned(),
                expected_version: None,
                value: Some(json!({"must_not_write": true})),
                artifact: None,
                ttl_ms: 60_000,
            },
            "session-audit",
            9_100,
        );
        crate::server::command_audit::set_command_audit_force_fail_for_tests(false);

        let error = match result {
            Ok(response) => anyhow::bail!("workspace_put unexpectedly succeeded: {response:?}"),
            Err(error) => error,
        };
        assert_eq!(error_code(&error), Some(error_codes::TOOL_INTERNAL_ERROR));
        assert!(error.message.contains("command audit forced failure"));

        let db = service.workspace_db()?;
        assert!(
            db.scan_cf_prefix(cf::CF_KV, workspace_run_prefix("run-audit-fail").as_bytes())?
                .is_empty(),
            "workspace row must not exist when command audit intent write fails"
        );
        Ok(())
    }

    #[test]
    fn put_and_get_round_trip_value_and_artifact_file_readback() -> anyhow::Result<()> {
        let temp = TempDir::new()?;
        let artifact_path = temp.path().join("artifact.txt");
        std::fs::write(&artifact_path, b"issue796-artifact-bytes")?;
        let artifact_hash = sha256_file(&artifact_path)
            .map_err(|error| anyhow::anyhow!("artifact hash failed: {error:?}"))?;
        let service = service_with_db(temp.path())?;

        let put = service.workspace_put_impl_at(
            WorkspacePutParams {
                run_id: Some("run-796".to_owned()),
                key: "findings/k1".to_owned(),
                expected_version: None,
                value: Some(json!({"known": 4, "marker": "ISSUE796"})),
                artifact: Some(WorkspaceArtifactRef {
                    handle: "artifact://issue796/screenshot".to_owned(),
                    path: Some(artifact_path.display().to_string()),
                    media_type: Some("text/plain".to_owned()),
                    kind: Some("screenshot".to_owned()),
                    sha256: Some(artifact_hash.clone()),
                    bytes_len: Some(23),
                }),
                ttl_ms: 60_000,
            },
            "session-a",
            10_000,
        )?;
        assert_eq!(put.run_id, "run-796");
        assert_eq!(put.version, 1);
        assert_eq!(
            put.artifact_readback
                .as_ref()
                .map(|row| row.sha256.as_str()),
            Some(artifact_hash.as_str())
        );

        let get = service.workspace_get_impl_at(
            WorkspaceGetParams {
                run_id: Some("run-796".to_owned()),
                key: "findings/k1".to_owned(),
            },
            "session-b",
            10_001,
        )?;
        assert_eq!(
            get.entry.value,
            Some(json!({"known": 4, "marker": "ISSUE796"}))
        );
        assert_eq!(
            get.entry
                .artifact
                .as_ref()
                .map(|artifact| artifact.handle.as_str()),
            Some("artifact://issue796/screenshot")
        );
        assert_eq!(get.storage_readback, put.storage_readback);
        Ok(())
    }

    #[test]
    fn list_skips_corrupt_row_without_breaking_good_reader() -> anyhow::Result<()> {
        let temp = TempDir::new()?;
        let service = service_with_db(temp.path())?;
        service.workspace_put_impl_at(
            WorkspacePutParams {
                run_id: Some("run-corrupt".to_owned()),
                key: "findings/good".to_owned(),
                expected_version: None,
                value: Some(json!({"ok": true})),
                artifact: None,
                ttl_ms: 60_000,
            },
            "session-a",
            20_000,
        )?;
        let db = service.workspace_db()?;
        let corrupt_key = workspace_row_key("run-corrupt", "findings/bad");
        db.put_batch_pressure_bypass(
            cf::CF_KV,
            [(corrupt_key.as_bytes().to_vec(), b"{not-json".to_vec())],
        )?;

        let list = service.workspace_list_impl_at(
            WorkspaceListParams {
                run_id: Some("run-corrupt".to_owned()),
                prefix: Some("findings/".to_owned()),
                limit: 10,
                include_values: true,
            },
            "session-b",
            20_001,
        )?;
        assert_eq!(list.scanned_rows, 2);
        assert_eq!(list.returned_count, 1);
        assert_eq!(list.entries[0].key, "findings/good");
        assert_eq!(list.corrupt_rows_skipped.len(), 1);
        assert_eq!(list.corrupt_rows_skipped[0].row_key, corrupt_key);
        Ok(())
    }

    #[test]
    fn subscribe_filter_matches_later_put_event() -> anyhow::Result<()> {
        let temp = TempDir::new()?;
        let service = service_with_db(temp.path())?;
        let subscription = service.workspace_subscribe_impl(
            WorkspaceSubscribeParams {
                run_id: Some("run-sub".to_owned()),
                prefix: "findings/".to_owned(),
                snapshot_first: false,
            },
            "session-b",
        )?;
        assert_eq!(service.sse_state()?.active_subscription_count(), 1);

        let put = service.workspace_put_impl_at(
            WorkspacePutParams {
                run_id: Some("run-sub".to_owned()),
                key: "findings/x".to_owned(),
                expected_version: None,
                value: Some(json!({"event": "expected"})),
                artifact: None,
                ttl_ms: 60_000,
            },
            "session-a",
            30_000,
        )?;
        assert_eq!(subscription.event_kind, WORKSPACE_PUT_EVENT_KIND);
        assert_eq!(put.event_publish_report.matched, 1);
        assert_eq!(put.event_publish_report.queued, 1);
        Ok(())
    }

    #[test]
    fn expired_rows_are_deleted_on_get_and_list() -> anyhow::Result<()> {
        let temp = TempDir::new()?;
        let service = service_with_db(temp.path())?;
        service.workspace_put_impl_at(
            WorkspacePutParams {
                run_id: Some("run-ttl".to_owned()),
                key: "findings/old".to_owned(),
                expected_version: None,
                value: Some(json!({"ttl": 1})),
                artifact: None,
                ttl_ms: 1,
            },
            "session-a",
            40_000,
        )?;

        let error = match service.workspace_get_impl_at(
            WorkspaceGetParams {
                run_id: Some("run-ttl".to_owned()),
                key: "findings/old".to_owned(),
            },
            "session-b",
            40_002,
        ) {
            Ok(response) => anyhow::bail!("expired get unexpectedly succeeded: {response:?}"),
            Err(error) => error,
        };
        assert_eq!(error_code(&error), Some(error_codes::STORAGE_READ_FAILED));
        let db = service.workspace_db()?;
        assert!(
            db.scan_cf_prefix(cf::CF_KV, workspace_run_prefix("run-ttl").as_bytes())?
                .is_empty()
        );

        service.workspace_put_impl_at(
            WorkspacePutParams {
                run_id: Some("run-ttl".to_owned()),
                key: "findings/old-list".to_owned(),
                expected_version: None,
                value: Some(json!({"ttl": 1})),
                artifact: None,
                ttl_ms: 1,
            },
            "session-a",
            50_000,
        )?;
        let list = service.workspace_list_impl_at(
            WorkspaceListParams {
                run_id: Some("run-ttl".to_owned()),
                prefix: Some("findings/".to_owned()),
                limit: 10,
                include_values: true,
            },
            "session-b",
            50_002,
        )?;
        assert_eq!(list.expired_rows_deleted, 1);
        assert_eq!(list.returned_count, 0);
        assert!(
            db.scan_cf_prefix(cf::CF_KV, workspace_run_prefix("run-ttl").as_bytes())?
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn put_existing_key_requires_expected_version() -> anyhow::Result<()> {
        let temp = TempDir::new()?;
        let service = service_with_db(temp.path())?;
        let first = service.workspace_put_impl_at(
            WorkspacePutParams {
                run_id: Some("run-cas".to_owned()),
                key: "findings/shared".to_owned(),
                expected_version: None,
                value: Some(json!({"marker": "first"})),
                artifact: None,
                ttl_ms: 60_000,
            },
            "session-a",
            70_000,
        )?;
        assert_eq!(first.version, 1);
        assert_eq!(first.previous_version, None);

        let conflict = match service.workspace_put_impl_at(
            WorkspacePutParams {
                run_id: Some("run-cas".to_owned()),
                key: "findings/shared".to_owned(),
                expected_version: None,
                value: Some(json!({"marker": "blind-overwrite"})),
                artifact: None,
                ttl_ms: 60_000,
            },
            "session-b",
            70_001,
        ) {
            Ok(response) => anyhow::bail!("blind overwrite unexpectedly succeeded: {response:?}"),
            Err(error) => error,
        };
        assert_eq!(
            error_code(&conflict),
            Some(error_codes::STORAGE_WRITE_FAILED)
        );
        assert_eq!(
            error_detail_code(&conflict),
            Some("WORKSPACE_VERSION_CONFLICT")
        );

        let still_first = service.workspace_get_impl_at(
            WorkspaceGetParams {
                run_id: Some("run-cas".to_owned()),
                key: "findings/shared".to_owned(),
            },
            "session-c",
            70_002,
        )?;
        assert_eq!(still_first.entry.value, Some(json!({"marker": "first"})));
        assert_eq!(still_first.entry.version, 1);

        let second = service.workspace_put_impl_at(
            WorkspacePutParams {
                run_id: Some("run-cas".to_owned()),
                key: "findings/shared".to_owned(),
                expected_version: Some(1),
                value: Some(json!({"marker": "second"})),
                artifact: None,
                ttl_ms: 60_000,
            },
            "session-b",
            70_003,
        )?;
        assert_eq!(second.version, 2);
        assert_eq!(second.previous_version, Some(1));

        let updated = service.workspace_get_impl_at(
            WorkspaceGetParams {
                run_id: Some("run-cas".to_owned()),
                key: "findings/shared".to_owned(),
            },
            "session-c",
            70_004,
        )?;
        assert_eq!(updated.entry.value, Some(json!({"marker": "second"})));
        assert_eq!(updated.entry.version, 2);
        Ok(())
    }

    #[test]
    fn parameter_edges_fail_closed() -> anyhow::Result<()> {
        assert!(normalize_workspace_key("").is_err());
        assert!(normalize_workspace_key("bad\nkey").is_err());
        assert!(validate_workspace_ttl_ms(0).is_err());
        assert!(validate_workspace_ttl_ms(MAX_WORKSPACE_TTL_MS + 1).is_err());
        assert!(validate_workspace_list_limit(0).is_err());
        assert!(validate_workspace_list_limit(MAX_LIST_LIMIT + 1).is_err());
        assert!(
            validate_inline_value_size(Some(&json!({"blob": "x".repeat(MAX_INLINE_VALUE_BYTES)})))
                .is_err()
        );
        assert!(normalize_sha256("not-a-digest".to_owned()).is_err());

        let temp = TempDir::new()?;
        let service = service_with_db(temp.path())?;
        let error = match service.workspace_put_impl_at(
            WorkspacePutParams {
                run_id: Some("run-edge".to_owned()),
                key: "findings/no-payload".to_owned(),
                expected_version: None,
                value: None,
                artifact: None,
                ttl_ms: 60_000,
            },
            "session-a",
            60_000,
        ) {
            Ok(response) => anyhow::bail!("payload-less put unexpectedly succeeded: {response:?}"),
            Err(error) => error,
        };
        assert_eq!(error_code(&error), Some(error_codes::TOOL_PARAMS_INVALID));
        Ok(())
    }
}
