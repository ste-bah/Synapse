use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use chrono::Utc;
use rmcp::{ErrorData, model::ErrorCode, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synapse_core::{ProfileId, SCHEMA_VERSION, error_codes};
use synapse_profiles::{
    ProfileError, ProfilePackageManifest, package_manifest_digest, parse_package_manifest_bytes,
    parse_package_manifest_bytes_with_digest, parse_profile_file,
};
use synapse_reflex::ReflexRuntime;
use synapse_storage::{cf, decode_json, encode_json};

use crate::m1::mcp_error;

use super::{
    M3ToolStub,
    permissions::{Permission, RequiredPermissions, required},
};

const REGISTRY_PREFIX: &str = "profile_registry/v1/";
const SOURCE_PREFIX: &str = "profile_registry/v1/source/";
const PACKAGE_PREFIX: &str = "profile_registry/v1/package/";
const PROFILE_PREFIX: &str = "profile_registry/v1/profile/";
const INSTALLED_PREFIX: &str = "profile_registry/v1/installed/";
const COMPAT_PREFIX: &str = "profile_registry/v1/compat/";
const QUALITY_LINK_PREFIX: &str = "profile_registry/v1/quality_link/";
const HEAD_PREFIX: &str = "profile_registry/v1/head/";
const DEFAULT_SOURCE_ID: &str = "registry.local";
const DEFAULT_SEARCH_LIMIT: u32 = 100;
const MAX_SEARCH_LIMIT: u32 = 1000;
const VALUE_PREFIX_CHARS: usize = 1024;

type EncodedRow = (Vec<u8>, Vec<u8>);
type EncodedRows = Vec<EncodedRow>;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistrySearchParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_kind: Option<String>,
    #[serde(default)]
    pub include_disabled: bool,
    #[serde(default = "default_search_limit")]
    #[schemars(default = "default_search_limit", range(min = 1, max = 1000))]
    pub limit: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistryInspectParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<ProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_profile_id: Option<ProfileId>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistryInstallParams {
    pub manifest_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_manifest_digest: Option<String>,
    #[serde(default = "default_source_id")]
    #[schemars(default = "default_source_id")]
    pub source_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistryDisableParams {
    pub profile_id: ProfileId,
    #[serde(default = "default_disabled_state")]
    #[schemars(default = "default_disabled_state")]
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistryExportParams {
    pub output_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_kind: Option<String>,
    #[serde(default)]
    pub include_disabled: bool,
    #[serde(default = "default_search_limit")]
    #[schemars(default = "default_search_limit", range(min = 1, max = 1000))]
    pub limit: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistryImportParams {
    pub bundle_path: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditIntelligenceQueryParams {
    pub profile_id: ProfileId,
    #[serde(default = "default_search_limit")]
    #[schemars(default = "default_search_limit", range(min = 1, max = 1000))]
    pub max_rows: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistryRowSummary {
    pub cf_name: String,
    pub key: String,
    pub key_hex: String,
    pub row_kind: Option<String>,
    pub row_id: Option<String>,
    pub source_id: Option<String>,
    pub state: Option<String>,
    pub profile_id: Option<ProfileId>,
    pub profile_version: Option<String>,
    pub package_id: Option<String>,
    pub package_version: Option<String>,
    pub updated_at: Option<String>,
    pub value_len_bytes: u64,
    pub value_utf8_prefix: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistryStoredRow {
    pub summary: ProfileRegistryRowSummary,
    pub value: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistrySearchResponse {
    pub cf_name: String,
    pub prefix: String,
    pub query: Option<String>,
    pub row_kind: Option<String>,
    pub include_disabled: bool,
    pub limit: u32,
    pub total_matched: u64,
    pub rows: Vec<ProfileRegistryRowSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistryInspectResponse {
    pub cf_name: String,
    pub row_key: String,
    pub found: bool,
    pub row: Option<ProfileRegistryStoredRow>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistryInstallResponse {
    pub operation: String,
    pub source_id: String,
    pub package_id: String,
    pub package_version: String,
    pub profile_id: ProfileId,
    pub profile_version: String,
    pub manifest_path: String,
    pub manifest_digest: String,
    pub profile_toml_path: String,
    pub wrote_rows: bool,
    pub idempotent: bool,
    pub cf_profile_row_keys: Vec<String>,
    pub cf_kv_row_keys: Vec<String>,
    pub row_summaries: Vec<ProfileRegistryRowSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistryDisableResponse {
    pub profile_id: ProfileId,
    pub row_key: String,
    pub previous_state: Option<String>,
    pub state: String,
    pub wrote_row: bool,
    pub row: ProfileRegistryStoredRow,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistryExportResponse {
    pub output_path: String,
    pub bytes_written: u64,
    pub rows_exported: u64,
    pub rows: Vec<ProfileRegistryRowSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRegistryImportResponse {
    pub bundle_path: String,
    pub rows_read: u64,
    pub cf_profile_rows_written: u64,
    pub cf_kv_rows_written: u64,
    pub rows: Vec<ProfileRegistryRowSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditIntelligenceQueryResponse {
    pub profile_id: ProfileId,
    pub max_rows: u32,
    pub action: AuditBucketSummary,
    pub events: AuditBucketSummary,
    pub reflexes: AuditBucketSummary,
    pub sessions: AuditSessionSummary,
    pub quality_snapshot_key: String,
    pub quality_snapshot: Option<Value>,
    pub learning_candidates: Vec<AuditLearningCandidate>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditBucketSummary {
    pub cf_name: String,
    pub rows_scanned: u64,
    pub matching_rows: u64,
    pub by_status: BTreeMap<String, u64>,
    pub by_kind_or_tool: BTreeMap<String, u64>,
    pub by_error_code: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditSessionSummary {
    pub cf_name: String,
    pub rows_scanned: u64,
    pub matching_rows: u64,
    pub session_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditLearningCandidate {
    pub kind: String,
    pub evidence_count: u64,
    pub rationale: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileRegistryExportBundle {
    schema_version: u32,
    exported_at: String,
    rows: Vec<ProfileRegistryBundleRow>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileRegistryBundleRow {
    cf_name: String,
    key: String,
    value: Value,
}

#[must_use]
pub const fn profile_registry_search() -> M3ToolStub {
    M3ToolStub::new("profile_registry_search")
}

#[must_use]
pub const fn profile_registry_inspect() -> M3ToolStub {
    M3ToolStub::new("profile_registry_inspect")
}

#[must_use]
pub const fn profile_registry_install() -> M3ToolStub {
    M3ToolStub::new("profile_registry_install")
}

#[must_use]
pub const fn profile_registry_disable() -> M3ToolStub {
    M3ToolStub::new("profile_registry_disable")
}

#[must_use]
pub const fn profile_registry_export() -> M3ToolStub {
    M3ToolStub::new("profile_registry_export")
}

#[must_use]
pub const fn profile_registry_import() -> M3ToolStub {
    M3ToolStub::new("profile_registry_import")
}

#[must_use]
pub const fn audit_intelligence_query() -> M3ToolStub {
    M3ToolStub::new("audit_intelligence_query")
}

#[must_use]
pub fn required_permissions_search(_params: &ProfileRegistrySearchParams) -> RequiredPermissions {
    required([Permission::ReadProfile, Permission::ReadStorage])
}

#[must_use]
pub fn required_permissions_inspect(_params: &ProfileRegistryInspectParams) -> RequiredPermissions {
    required([Permission::ReadProfile, Permission::ReadStorage])
}

#[must_use]
pub fn required_permissions_install(_params: &ProfileRegistryInstallParams) -> RequiredPermissions {
    required([
        Permission::ReadProfile,
        Permission::ReadStorage,
        Permission::WriteStorage,
    ])
}

#[must_use]
pub fn required_permissions_disable(_params: &ProfileRegistryDisableParams) -> RequiredPermissions {
    required([
        Permission::ReadProfile,
        Permission::ReadStorage,
        Permission::WriteStorage,
    ])
}

#[must_use]
pub fn required_permissions_export(_params: &ProfileRegistryExportParams) -> RequiredPermissions {
    required([Permission::ReadProfile, Permission::ReadStorage])
}

#[must_use]
pub fn required_permissions_import(_params: &ProfileRegistryImportParams) -> RequiredPermissions {
    required([
        Permission::ReadProfile,
        Permission::ReadStorage,
        Permission::WriteStorage,
    ])
}

#[must_use]
pub fn required_permissions_audit(_params: &AuditIntelligenceQueryParams) -> RequiredPermissions {
    required([Permission::ReadProfile, Permission::ReadStorage])
}

pub fn search_registry(
    reflex_runtime: &Arc<Mutex<ReflexRuntime>>,
    params: &ProfileRegistrySearchParams,
) -> Result<ProfileRegistrySearchResponse, ErrorData> {
    validate_limit(params.limit)?;
    if let Some(kind) = &params.row_kind {
        validate_non_empty("row_kind", kind)?;
    }
    let runtime = lock_runtime(reflex_runtime, "searching profile registry")?;
    let rows = runtime
        .storage_cf_prefix_rows(cf::CF_PROFILES, REGISTRY_PREFIX.as_bytes(), usize::MAX)
        .map_err(storage_error)?;
    drop(runtime);
    let query = normalized_query(params.query.as_deref());
    let mut matched = Vec::new();
    let mut total_matched = 0_u64;
    for (key, value) in rows {
        let summary = row_summary(cf::CF_PROFILES, &key, &value);
        if !row_filter_matches(&summary, &value, query.as_deref(), params) {
            continue;
        }
        total_matched += 1;
        if matched.len() < params.limit as usize {
            matched.push(summary);
        }
    }
    Ok(ProfileRegistrySearchResponse {
        cf_name: cf::CF_PROFILES.to_owned(),
        prefix: REGISTRY_PREFIX.to_owned(),
        query,
        row_kind: params.row_kind.clone(),
        include_disabled: params.include_disabled,
        limit: params.limit,
        total_matched,
        rows: matched,
    })
}

pub fn inspect_registry(
    reflex_runtime: &Arc<Mutex<ReflexRuntime>>,
    params: &ProfileRegistryInspectParams,
) -> Result<ProfileRegistryInspectResponse, ErrorData> {
    let (cf_name, key) = inspect_key(params)?;
    let runtime = lock_runtime(reflex_runtime, "inspecting profile registry")?;
    let value = if cf_name == cf::CF_KV {
        runtime.storage_kv_row(key.as_bytes())
    } else {
        runtime.storage_profile_row(key.as_bytes())
    }
    .map_err(storage_error)?;
    drop(runtime);
    let row = value
        .as_ref()
        .map(|value| stored_row(cf_name, key.as_bytes(), value))
        .transpose()?;
    Ok(ProfileRegistryInspectResponse {
        cf_name: cf_name.to_owned(),
        row_key: key,
        found: row.is_some(),
        row,
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "single MCP operation keeps manifest validation, duplicate handling, row write, and readback together"
)]
pub fn install_registry_package(
    reflex_runtime: &Arc<Mutex<ReflexRuntime>>,
    params: &ProfileRegistryInstallParams,
) -> Result<ProfileRegistryInstallResponse, ErrorData> {
    validate_registry_id("source_id", &params.source_id)?;
    let manifest_path = required_path("manifest_path", &params.manifest_path)?;
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        mcp_error(
            error_codes::PROFILE_PARSE_ERROR,
            format!(
                "profile_registry_install could not read manifest {}: {error}",
                manifest_path.display()
            ),
        )
    })?;
    let manifest_digest = package_manifest_digest(&manifest_bytes);
    let manifest = parse_manifest(&manifest_path, &manifest_bytes, params)?;
    let profile_toml_path = resolve_package_file(&manifest_path, &manifest.files.profile_toml);
    let loaded_profile = parse_profile_file(&profile_toml_path).map_err(profile_error)?;
    if loaded_profile.profile.id != manifest.profile_id {
        return Err(registry_error(
            "profile_toml_id_mismatch",
            format!(
                "manifest profile_id {} does not match profile TOML id {}",
                manifest.profile_id, loaded_profile.profile.id
            ),
        ));
    }
    let package_key = package_key(&manifest.package_id, &manifest.package_version);
    let updated_at = Utc::now().to_rfc3339();
    let runtime = lock_runtime(reflex_runtime, "installing profile registry package")?;
    if let Some(existing) = runtime
        .storage_profile_row(package_key.as_bytes())
        .map_err(storage_error)?
    {
        let existing_value = decode_json::<Value>(&existing).map_err(decode_error)?;
        let existing_digest = existing_value
            .get("manifest_digest")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if existing_digest == manifest_digest {
            let row = row_summary(cf::CF_PROFILES, package_key.as_bytes(), &existing);
            return Ok(ProfileRegistryInstallResponse {
                operation: "install_or_update".to_owned(),
                source_id: params.source_id.clone(),
                package_id: manifest.package_id,
                package_version: manifest.package_version,
                profile_id: manifest.profile_id,
                profile_version: manifest.profile_version,
                manifest_path: manifest_path.display().to_string(),
                manifest_digest,
                profile_toml_path: profile_toml_path.display().to_string(),
                wrote_rows: false,
                idempotent: true,
                cf_profile_row_keys: vec![package_key],
                cf_kv_row_keys: Vec::new(),
                row_summaries: vec![row],
            });
        }
        return Err(registry_error(
            "duplicate_package_version_conflict",
            format!(
                "package {}@{} already exists with manifest_digest {}; new digest is {}",
                manifest.package_id, manifest.package_version, existing_digest, manifest_digest
            ),
        ));
    }

    let quality_row = runtime
        .storage_profile_row(quality_key(&manifest.profile_id).as_bytes())
        .map_err(storage_error)?;
    let mut profile_rows = registry_rows(
        &manifest,
        &manifest_path,
        &manifest_digest,
        &profile_toml_path,
        &params.source_id,
        &updated_at,
        quality_row.as_deref(),
    )?;
    let kv_rows = vec![head_row(
        &manifest,
        &manifest_digest,
        &params.source_id,
        &updated_at,
    )?];
    let profile_row_keys = profile_rows
        .iter()
        .map(|(key, _value)| String::from_utf8_lossy(key).into_owned())
        .collect::<Vec<_>>();
    let kv_row_keys = kv_rows
        .iter()
        .map(|(key, _value)| String::from_utf8_lossy(key).into_owned())
        .collect::<Vec<_>>();
    runtime
        .storage_put_profile_rows(std::mem::take(&mut profile_rows))
        .map_err(storage_error)?;
    runtime
        .storage_put_kv_rows(kv_rows)
        .map_err(storage_error)?;
    let mut summaries = Vec::new();
    for key in &profile_row_keys {
        if let Some(value) = runtime
            .storage_profile_row(key.as_bytes())
            .map_err(storage_error)?
        {
            summaries.push(row_summary(cf::CF_PROFILES, key.as_bytes(), &value));
        }
    }
    for key in &kv_row_keys {
        if let Some(value) = runtime
            .storage_kv_row(key.as_bytes())
            .map_err(storage_error)?
        {
            summaries.push(row_summary(cf::CF_KV, key.as_bytes(), &value));
        }
    }
    drop(runtime);
    Ok(ProfileRegistryInstallResponse {
        operation: "install_or_update".to_owned(),
        source_id: params.source_id.clone(),
        package_id: manifest.package_id,
        package_version: manifest.package_version,
        profile_id: manifest.profile_id,
        profile_version: manifest.profile_version,
        manifest_path: manifest_path.display().to_string(),
        manifest_digest,
        profile_toml_path: profile_toml_path.display().to_string(),
        wrote_rows: true,
        idempotent: false,
        cf_profile_row_keys: profile_row_keys,
        cf_kv_row_keys: kv_row_keys,
        row_summaries: summaries,
    })
}

pub fn disable_registry_profile(
    reflex_runtime: &Arc<Mutex<ReflexRuntime>>,
    params: &ProfileRegistryDisableParams,
) -> Result<ProfileRegistryDisableResponse, ErrorData> {
    validate_disabled_state(&params.state)?;
    let key = installed_key(&params.profile_id);
    let runtime = lock_runtime(reflex_runtime, "disabling profile registry package")?;
    let existing = runtime
        .storage_profile_row(key.as_bytes())
        .map_err(storage_error)?
        .ok_or_else(|| {
            registry_error(
                "installed_profile_missing",
                format!(
                    "installed profile row for {} was not found",
                    params.profile_id
                ),
            )
        })?;
    let mut value = decode_json::<Value>(&existing).map_err(decode_error)?;
    let previous_state = value
        .get("state")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let updated_at = Utc::now().to_rfc3339();
    set_object_field(&mut value, "state", json!(params.state));
    set_object_field(&mut value, "activation_state", json!(params.state));
    set_object_field(&mut value, "updated_at", json!(updated_at));
    set_object_field(&mut value, "disable_reason", json!(params.reason));
    if params.state == "removed" {
        set_object_field(&mut value, "removed_at", json!(updated_at));
    } else {
        set_object_field(&mut value, "disabled_at", json!(updated_at));
    }
    let encoded = encode_json(&value).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("installed registry row encode failed: {error}"),
        )
    })?;
    runtime
        .storage_put_profile_rows(vec![(key.clone().into_bytes(), encoded)])
        .map_err(storage_error)?;
    let stored = runtime
        .storage_profile_row(key.as_bytes())
        .map_err(storage_error)?
        .ok_or_else(|| {
            registry_error(
                "installed_profile_write_missing",
                "installed profile row did not persist",
            )
        })?;
    drop(runtime);
    let row = stored_row(cf::CF_PROFILES, key.as_bytes(), &stored)?;
    Ok(ProfileRegistryDisableResponse {
        profile_id: params.profile_id.clone(),
        row_key: key,
        previous_state,
        state: params.state.clone(),
        wrote_row: true,
        row,
    })
}

pub fn export_registry(
    reflex_runtime: &Arc<Mutex<ReflexRuntime>>,
    params: &ProfileRegistryExportParams,
) -> Result<ProfileRegistryExportResponse, ErrorData> {
    validate_limit(params.limit)?;
    let output_path = required_path("output_path", &params.output_path)?;
    let runtime = lock_runtime(reflex_runtime, "exporting profile registry")?;
    let profile_rows = runtime
        .storage_cf_prefix_rows(cf::CF_PROFILES, REGISTRY_PREFIX.as_bytes(), usize::MAX)
        .map_err(storage_error)?;
    let kv_rows = runtime
        .storage_cf_prefix_rows(cf::CF_KV, REGISTRY_PREFIX.as_bytes(), usize::MAX)
        .map_err(storage_error)?;
    drop(runtime);
    let search_params = ProfileRegistrySearchParams {
        query: params.query.clone(),
        row_kind: params.row_kind.clone(),
        include_disabled: params.include_disabled,
        limit: params.limit,
    };
    let query = normalized_query(params.query.as_deref());
    let mut bundle_rows = Vec::new();
    let mut summaries = Vec::new();
    'rows: for (cf_name, rows) in [(cf::CF_PROFILES, profile_rows), (cf::CF_KV, kv_rows)] {
        for (key, value) in rows {
            let summary = row_summary(cf_name, &key, &value);
            if !row_filter_matches(&summary, &value, query.as_deref(), &search_params) {
                continue;
            }
            if summaries.len() >= params.limit as usize {
                break 'rows;
            }
            let key_string = String::from_utf8_lossy(&key).into_owned();
            let decoded = decode_json::<Value>(&value).map_err(decode_error)?;
            bundle_rows.push(ProfileRegistryBundleRow {
                cf_name: cf_name.to_owned(),
                key: key_string,
                value: decoded,
            });
            summaries.push(summary);
        }
    }
    let bundle = ProfileRegistryExportBundle {
        schema_version: SCHEMA_VERSION,
        exported_at: Utc::now().to_rfc3339(),
        rows: bundle_rows,
    };
    let bytes = serde_json::to_vec_pretty(&bundle).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("profile registry export encode failed: {error}"),
        )
    })?;
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!(
                    "profile registry export could not create {}: {error}",
                    parent.display()
                ),
            )
        })?;
    }
    fs::write(&output_path, &bytes).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!(
                "profile registry export could not write {}: {error}",
                output_path.display()
            ),
        )
    })?;
    Ok(ProfileRegistryExportResponse {
        output_path: output_path.display().to_string(),
        bytes_written: bytes.len() as u64,
        rows_exported: summaries.len() as u64,
        rows: summaries,
    })
}

pub fn import_registry(
    reflex_runtime: &Arc<Mutex<ReflexRuntime>>,
    params: &ProfileRegistryImportParams,
) -> Result<ProfileRegistryImportResponse, ErrorData> {
    let bundle_path = required_path("bundle_path", &params.bundle_path)?;
    let bytes = fs::read(&bundle_path).map_err(|error| {
        mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!(
                "profile_registry_import could not read bundle {}: {error}",
                bundle_path.display()
            ),
        )
    })?;
    let bundle =
        serde_json::from_slice::<ProfileRegistryExportBundle>(&bytes).map_err(|error| {
            mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!("profile registry import bundle decode failed: {error}"),
            )
        })?;
    if bundle.schema_version != SCHEMA_VERSION {
        return Err(registry_error(
            "registry_bundle_schema_unsupported",
            format!(
                "registry bundle schema_version must be {SCHEMA_VERSION}; got {}",
                bundle.schema_version
            ),
        ));
    }
    let mut profile_rows = Vec::new();
    let mut kv_rows = Vec::new();
    let mut summaries = Vec::new();
    for row in bundle.rows {
        validate_bundle_row(&row)?;
        let encoded = encode_json(&row.value).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("profile registry import row encode failed: {error}"),
            )
        })?;
        summaries.push(row_summary(
            row.cf_name.as_str(),
            row.key.as_bytes(),
            &encoded,
        ));
        if row.cf_name == cf::CF_PROFILES {
            profile_rows.push((row.key.into_bytes(), encoded));
        } else {
            kv_rows.push((row.key.into_bytes(), encoded));
        }
    }
    let profile_count = profile_rows.len() as u64;
    let kv_count = kv_rows.len() as u64;
    let runtime = lock_runtime(reflex_runtime, "importing profile registry")?;
    runtime
        .storage_put_profile_rows(profile_rows)
        .map_err(storage_error)?;
    runtime
        .storage_put_kv_rows(kv_rows)
        .map_err(storage_error)?;
    drop(runtime);
    Ok(ProfileRegistryImportResponse {
        bundle_path: bundle_path.display().to_string(),
        rows_read: summaries.len() as u64,
        cf_profile_rows_written: profile_count,
        cf_kv_rows_written: kv_count,
        rows: summaries,
    })
}

pub fn query_audit_intelligence(
    reflex_runtime: &Arc<Mutex<ReflexRuntime>>,
    params: &AuditIntelligenceQueryParams,
) -> Result<AuditIntelligenceQueryResponse, ErrorData> {
    validate_limit(params.max_rows)?;
    let runtime = lock_runtime(reflex_runtime, "querying audit intelligence")?;
    let action_rows = runtime
        .storage_cf_tail_rows(cf::CF_ACTION_LOG, params.max_rows as usize)
        .map_err(storage_error)?;
    let event_rows = runtime
        .storage_cf_tail_rows(cf::CF_EVENTS, params.max_rows as usize)
        .map_err(storage_error)?;
    let reflex_rows = runtime
        .storage_cf_tail_rows(cf::CF_REFLEX_AUDIT, params.max_rows as usize)
        .map_err(storage_error)?;
    let session_rows = runtime
        .storage_cf_tail_rows(cf::CF_SESSIONS, params.max_rows as usize)
        .map_err(storage_error)?;
    let quality_key = quality_key(&params.profile_id);
    let quality_snapshot = runtime
        .storage_profile_row(quality_key.as_bytes())
        .map_err(storage_error)?
        .map(|value| decode_json::<Value>(&value).map_err(decode_error))
        .transpose()?;
    drop(runtime);
    let action = summarize_bucket(cf::CF_ACTION_LOG, &params.profile_id, action_rows, "tool")?;
    let events = summarize_bucket(cf::CF_EVENTS, &params.profile_id, event_rows, "kind")?;
    let reflexes = summarize_bucket(
        cf::CF_REFLEX_AUDIT,
        &params.profile_id,
        reflex_rows,
        "status",
    )?;
    let sessions = summarize_sessions(&params.profile_id, session_rows)?;
    let learning_candidates =
        learning_candidates(&action, &events, &reflexes, quality_snapshot.is_some());
    Ok(AuditIntelligenceQueryResponse {
        profile_id: params.profile_id.clone(),
        max_rows: params.max_rows,
        action,
        events,
        reflexes,
        sessions,
        quality_snapshot_key: quality_key,
        quality_snapshot,
        learning_candidates,
    })
}

fn parse_manifest(
    path: &Path,
    bytes: &[u8],
    params: &ProfileRegistryInstallParams,
) -> Result<ProfilePackageManifest, ErrorData> {
    params.expected_manifest_digest.as_ref().map_or_else(
        || parse_package_manifest_bytes(path, bytes).map_err(profile_error),
        |expected| {
            parse_package_manifest_bytes_with_digest(path, bytes, expected).map_err(profile_error)
        },
    )
}

fn registry_rows(
    manifest: &ProfilePackageManifest,
    manifest_path: &Path,
    manifest_digest: &str,
    profile_toml_path: &Path,
    source_id: &str,
    updated_at: &str,
    quality_row: Option<&[u8]>,
) -> Result<EncodedRows, ErrorData> {
    let mut rows = vec![
        encoded_row(
            source_key(source_id),
            &source_row(manifest, source_id, updated_at),
        )?,
        encoded_row(
            package_key(&manifest.package_id, &manifest.package_version),
            &package_row(
                manifest,
                manifest_path,
                manifest_digest,
                source_id,
                updated_at,
            ),
        )?,
        encoded_row(
            profile_key(&manifest.profile_id, &manifest.profile_version),
            &profile_row(manifest, profile_toml_path, source_id, updated_at),
        )?,
        encoded_row(
            installed_key(&manifest.profile_id),
            &installed_row(manifest, source_id, updated_at),
        )?,
    ];
    for target in &manifest.targets {
        rows.push(encoded_row(
            compat_key(
                &target.target_id,
                &manifest.profile_id,
                &manifest.profile_version,
            ),
            &compat_row(
                manifest,
                &target.target_id,
                &target.target_kind,
                source_id,
                updated_at,
            ),
        )?);
    }
    if let Some(quality_row) = quality_row {
        let quality = decode_json::<Value>(quality_row).map_err(decode_error)?;
        rows.push(encoded_row(
            quality_link_key(&manifest.profile_id, &manifest.profile_version),
            &quality_link_row(manifest, &quality, source_id, updated_at),
        )?);
    }
    Ok(rows)
}

fn encoded_row(key: String, value: &Value) -> Result<EncodedRow, ErrorData> {
    let encoded = encode_json(value).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("profile registry row encode failed: {error}"),
        )
    })?;
    Ok((key.into_bytes(), encoded))
}

fn source_row(manifest: &ProfilePackageManifest, source_id: &str, updated_at: &str) -> Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "row_kind": "registry_source",
        "row_id": source_id,
        "created_at": manifest.created_at,
        "updated_at": updated_at,
        "source_id": source_id,
        "state": "active",
        "source_kind": manifest.source.kind,
        "base_url": manifest.source.uri,
        "root_path": null,
        "auth_mode": "none",
        "trust_policy_id": "local-first",
        "offline_usable": true,
        "last_health_status": "ok",
    })
}

fn package_row(
    manifest: &ProfilePackageManifest,
    manifest_path: &Path,
    manifest_digest: &str,
    source_id: &str,
    updated_at: &str,
) -> Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "row_kind": "profile_package",
        "row_id": format!("{}@{}", manifest.package_id, manifest.package_version),
        "created_at": manifest.created_at,
        "updated_at": updated_at,
        "source_id": source_id,
        "state": "active",
        "package_id": manifest.package_id,
        "package_version": manifest.package_version,
        "manifest_path": manifest_path.display().to_string(),
        "manifest_digest": manifest_digest,
        "package_digest": manifest.hashes.package_sha256,
        "profile_id": manifest.profile_id,
        "profile_version": manifest.profile_version,
        "target_ids": manifest.targets.iter().map(|target| target.target_id.clone()).collect::<Vec<_>>(),
        "license_spdx": manifest.permissions.license_spdx,
        "governance_manifest_key": format!("profile_registry/v1/package/{}/{}", manifest.package_id, manifest.package_version),
        "trust_status": "local_validated",
        "moderation_status": "local_only",
        "revoked": false,
        "profile_versions": [manifest.profile_version.clone()],
        "provenance": {
            "source_kind": manifest.source.kind,
            "source_uri": manifest.source.uri,
            "source_revision": manifest.source.revision,
            "built_by": manifest.source.built_by,
            "generated_by": manifest.source.generated_by,
        },
    })
}

fn profile_row(
    manifest: &ProfilePackageManifest,
    profile_toml_path: &Path,
    source_id: &str,
    updated_at: &str,
) -> Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "row_kind": "profile_version",
        "row_id": format!("{}@{}", manifest.profile_id, manifest.profile_version),
        "created_at": manifest.created_at,
        "updated_at": updated_at,
        "source_id": source_id,
        "state": "active",
        "profile_id": manifest.profile_id,
        "profile_version": manifest.profile_version,
        "package_id": manifest.package_id,
        "package_version": manifest.package_version,
        "profile_toml_path": profile_toml_path.display().to_string(),
        "profile_toml_digest": manifest.hashes.profile_toml_sha256,
        "use_scope": manifest.permissions.use_scope,
        "schema_version_supported": true,
    })
}

fn installed_row(manifest: &ProfilePackageManifest, source_id: &str, updated_at: &str) -> Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "row_kind": "installed_profile",
        "row_id": manifest.profile_id,
        "created_at": manifest.created_at,
        "updated_at": updated_at,
        "source_id": source_id,
        "state": "active",
        "profile_id": manifest.profile_id,
        "active_profile_version": manifest.profile_version,
        "installed_package_id": manifest.package_id,
        "installed_package_version": manifest.package_version,
        "installed_at": updated_at,
        "activation_state": "installed",
        "operator_overrides_path": null,
    })
}

fn compat_row(
    manifest: &ProfilePackageManifest,
    target_id: &str,
    target_kind: &str,
    source_id: &str,
    updated_at: &str,
) -> Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "row_kind": "compatibility_target",
        "row_id": format!("{target_id}:{}@{}", manifest.profile_id, manifest.profile_version),
        "created_at": manifest.created_at,
        "updated_at": updated_at,
        "source_id": source_id,
        "state": "active",
        "target_id": target_id,
        "target_kind": target_kind,
        "profile_id": manifest.profile_id,
        "profile_version": manifest.profile_version,
        "compatibility_status": "declared",
        "source_quality_snapshot_key": quality_key(&manifest.profile_id),
        "evidence_hash": manifest.hashes.package_sha256,
    })
}

fn quality_link_row(
    manifest: &ProfilePackageManifest,
    quality: &Value,
    source_id: &str,
    updated_at: &str,
) -> Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "row_kind": "quality_link",
        "row_id": format!("{}@{}", manifest.profile_id, manifest.profile_version),
        "created_at": manifest.created_at,
        "updated_at": updated_at,
        "source_id": source_id,
        "state": "active",
        "profile_id": manifest.profile_id,
        "profile_version": manifest.profile_version,
        "profile_quality_key": quality_key(&manifest.profile_id),
        "source_cf_ranges": {
            "audit_cf_name": quality.pointer("/source/audit_cf_name").cloned().unwrap_or_else(|| json!(cf::CF_ACTION_LOG)),
            "audit_rows_scanned": quality.pointer("/source/audit_rows_scanned").cloned().unwrap_or_else(|| json!(0)),
        },
        "quality_score": quality.pointer("/score/score_0_100").cloned().unwrap_or_else(|| json!(0)),
        "sample_count": quality.pointer("/score/sample_size").cloned().unwrap_or_else(|| json!(0)),
        "evidence_hash": quality.get("evidence_hash").cloned().unwrap_or_else(|| json!("")),
    })
}

fn head_row(
    manifest: &ProfilePackageManifest,
    manifest_digest: &str,
    source_id: &str,
    updated_at: &str,
) -> Result<EncodedRow, ErrorData> {
    let value = json!({
        "schema_version": SCHEMA_VERSION,
        "row_kind": "registry_head",
        "row_id": source_id,
        "created_at": manifest.created_at,
        "updated_at": updated_at,
        "source_id": source_id,
        "state": "active",
        "package_id": manifest.package_id,
        "package_version": manifest.package_version,
        "package_key": package_key(&manifest.package_id, &manifest.package_version),
        "manifest_digest": manifest_digest,
    });
    encoded_row(head_key(source_id), &value)
}

fn stored_row(
    cf_name: &str,
    key: &[u8],
    value: &[u8],
) -> Result<ProfileRegistryStoredRow, ErrorData> {
    Ok(ProfileRegistryStoredRow {
        summary: row_summary(cf_name, key, value),
        value: decode_json::<Value>(value).map_err(decode_error)?,
    })
}

fn row_summary(cf_name: &str, key: &[u8], value: &[u8]) -> ProfileRegistryRowSummary {
    let decoded = decode_json::<Value>(value).ok();
    ProfileRegistryRowSummary {
        cf_name: cf_name.to_owned(),
        key: String::from_utf8_lossy(key).into_owned(),
        key_hex: hex_encode(key),
        row_kind: decoded
            .as_ref()
            .and_then(|value| string_field(value, "row_kind")),
        row_id: decoded
            .as_ref()
            .and_then(|value| string_field(value, "row_id")),
        source_id: decoded
            .as_ref()
            .and_then(|value| string_field(value, "source_id")),
        state: decoded
            .as_ref()
            .and_then(|value| string_field(value, "state")),
        profile_id: decoded
            .as_ref()
            .and_then(|value| string_field(value, "profile_id")),
        profile_version: decoded.as_ref().and_then(|value| {
            string_field(value, "profile_version")
                .or_else(|| string_field(value, "active_profile_version"))
        }),
        package_id: decoded.as_ref().and_then(|value| {
            string_field(value, "package_id")
                .or_else(|| string_field(value, "installed_package_id"))
        }),
        package_version: decoded.as_ref().and_then(|value| {
            string_field(value, "package_version")
                .or_else(|| string_field(value, "installed_package_version"))
        }),
        updated_at: decoded
            .as_ref()
            .and_then(|value| string_field(value, "updated_at")),
        value_len_bytes: value.len() as u64,
        value_utf8_prefix: utf8_prefix(value, VALUE_PREFIX_CHARS),
    }
}

fn summarize_bucket(
    cf_name: &str,
    profile_id: &str,
    rows: Vec<(Vec<u8>, Vec<u8>)>,
    kind_field: &str,
) -> Result<AuditBucketSummary, ErrorData> {
    let rows_scanned = rows.len() as u64;
    let mut summary = AuditBucketSummary {
        cf_name: cf_name.to_owned(),
        rows_scanned,
        matching_rows: 0,
        by_status: BTreeMap::new(),
        by_kind_or_tool: BTreeMap::new(),
        by_error_code: BTreeMap::new(),
    };
    for (_key, value) in rows {
        let row = decode_json::<Value>(&value).map_err(decode_error)?;
        if !value_mentions_profile(&row, profile_id) {
            continue;
        }
        summary.matching_rows += 1;
        if let Some(status) = string_field(&row, "status") {
            increment(&mut summary.by_status, status);
        }
        if let Some(kind) = string_field(&row, kind_field)
            .or_else(|| string_field(&row, "kind"))
            .or_else(|| string_field(&row, "tool"))
        {
            increment(&mut summary.by_kind_or_tool, kind);
        }
        if let Some(error_code) = string_field(&row, "error_code").or_else(|| {
            row.pointer("/data/error_code")
                .and_then(Value::as_str)
                .map(str::to_owned)
        }) {
            increment(&mut summary.by_error_code, error_code);
        }
    }
    Ok(summary)
}

fn summarize_sessions(
    profile_id: &str,
    rows: Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<AuditSessionSummary, ErrorData> {
    let rows_scanned = rows.len() as u64;
    let mut session_ids = Vec::new();
    let mut matching_rows = 0;
    for (_key, value) in rows {
        let row = decode_json::<Value>(&value).map_err(decode_error)?;
        if !value_mentions_profile(&row, profile_id) {
            continue;
        }
        matching_rows += 1;
        if let Some(session_id) = string_field(&row, "session_id") {
            session_ids.push(session_id);
        }
    }
    session_ids.sort();
    session_ids.dedup();
    Ok(AuditSessionSummary {
        cf_name: cf::CF_SESSIONS.to_owned(),
        rows_scanned,
        matching_rows,
        session_ids,
    })
}

fn learning_candidates(
    action: &AuditBucketSummary,
    events: &AuditBucketSummary,
    reflexes: &AuditBucketSummary,
    has_quality_snapshot: bool,
) -> Vec<AuditLearningCandidate> {
    let mut candidates = Vec::new();
    let action_errors = action.by_error_code.values().sum();
    if action_errors > 0 {
        candidates.push(AuditLearningCandidate {
            kind: "action_error_cluster".to_owned(),
            evidence_count: action_errors,
            rationale:
                "Profile has action error rows; inspect keymap/backend policy for repeat failures."
                    .to_owned(),
        });
    }
    let activation_denied = events
        .by_kind_or_tool
        .get("profile.activation_denied")
        .copied()
        .unwrap_or_default();
    if activation_denied > 0 {
        candidates.push(AuditLearningCandidate {
            kind: "activation_denied".to_owned(),
            evidence_count: activation_denied,
            rationale: "Activation denial rows exist; profile registry should surface missing or disabled package state.".to_owned(),
        });
    }
    let reflex_errors = reflexes.by_error_code.values().sum();
    if reflex_errors > 0 {
        candidates.push(AuditLearningCandidate {
            kind: "reflex_error_cluster".to_owned(),
            evidence_count: reflex_errors,
            rationale: "Reflex audit error rows exist; inspect trigger lifetime and action sequence stability.".to_owned(),
        });
    }
    if !has_quality_snapshot {
        candidates.push(AuditLearningCandidate {
            kind: "quality_snapshot_missing".to_owned(),
            evidence_count: 1,
            rationale: "No profile quality snapshot row exists yet; run profile_quality_refresh after collecting action evidence.".to_owned(),
        });
    }
    candidates
}

fn value_mentions_profile(value: &Value, profile_id: &str) -> bool {
    string_field(value, "profile_id").as_deref() == Some(profile_id)
        || string_field(value, "active_profile_id").as_deref() == Some(profile_id)
        || string_field(value, "active_profile").as_deref() == Some(profile_id)
        || value
            .pointer("/audit_context/profile_id")
            .and_then(Value::as_str)
            == Some(profile_id)
        || value
            .pointer("/foreground/profile_id")
            .and_then(Value::as_str)
            == Some(profile_id)
        || value.pointer("/data/profile_id").and_then(Value::as_str) == Some(profile_id)
        || value
            .get("profile_history")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.get("profile_id").and_then(Value::as_str) == Some(profile_id))
            })
}

fn inspect_key(params: &ProfileRegistryInspectParams) -> Result<(&'static str, String), ErrorData> {
    if let Some(row_key) = params
        .row_key
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        if row_key.starts_with(HEAD_PREFIX) {
            return Ok((cf::CF_KV, row_key.to_owned()));
        }
        if row_key.starts_with(REGISTRY_PREFIX) {
            return Ok((cf::CF_PROFILES, row_key.to_owned()));
        }
        return Err(registry_error(
            "registry_row_key_invalid",
            "row_key must start with profile_registry/v1/",
        ));
    }
    if let Some(source_id) = &params.source_id {
        return Ok((cf::CF_PROFILES, source_key(source_id)));
    }
    if let (Some(package_id), Some(package_version)) = (&params.package_id, &params.package_version)
    {
        return Ok((cf::CF_PROFILES, package_key(package_id, package_version)));
    }
    if let (Some(profile_id), Some(profile_version)) = (&params.profile_id, &params.profile_version)
    {
        return Ok((cf::CF_PROFILES, profile_key(profile_id, profile_version)));
    }
    if let Some(profile_id) = &params.installed_profile_id {
        return Ok((cf::CF_PROFILES, installed_key(profile_id)));
    }
    Err(registry_error(
        "registry_inspect_target_missing",
        "provide row_key, source_id, package_id+package_version, profile_id+profile_version, or installed_profile_id",
    ))
}

fn row_filter_matches(
    summary: &ProfileRegistryRowSummary,
    value: &[u8],
    query: Option<&str>,
    params: &ProfileRegistrySearchParams,
) -> bool {
    if !params.include_disabled && matches!(summary.state.as_deref(), Some("disabled" | "removed"))
    {
        return false;
    }
    if let Some(row_kind) = &params.row_kind
        && summary.row_kind.as_deref() != Some(row_kind.as_str())
    {
        return false;
    }
    query.is_none_or(|query| {
        summary.key.to_ascii_lowercase().contains(query)
            || summary
                .value_utf8_prefix
                .to_ascii_lowercase()
                .contains(query)
            || String::from_utf8_lossy(value)
                .to_ascii_lowercase()
                .contains(query)
    })
}

fn validate_bundle_row(row: &ProfileRegistryBundleRow) -> Result<(), ErrorData> {
    if row.cf_name != cf::CF_PROFILES && row.cf_name != cf::CF_KV {
        return Err(registry_error(
            "registry_bundle_cf_invalid",
            format!(
                "bundle row cf_name must be CF_PROFILES or CF_KV; got {}",
                row.cf_name
            ),
        ));
    }
    if !row.key.starts_with(REGISTRY_PREFIX) {
        return Err(registry_error(
            "registry_bundle_key_invalid",
            "bundle row key must start with profile_registry/v1/",
        ));
    }
    if row.cf_name == cf::CF_KV && !row.key.starts_with(HEAD_PREFIX) {
        return Err(registry_error(
            "registry_bundle_kv_key_invalid",
            "CF_KV registry import rows must use profile_registry/v1/head/",
        ));
    }
    if !row.value.is_object() {
        return Err(registry_error(
            "registry_bundle_value_invalid",
            "bundle row value must be a JSON object",
        ));
    }
    let schema_version = row
        .value
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if schema_version != u64::from(SCHEMA_VERSION) {
        return Err(registry_error(
            "registry_bundle_row_schema_invalid",
            format!("bundle row schema_version must be {SCHEMA_VERSION}; got {schema_version}"),
        ));
    }
    Ok(())
}

fn resolve_package_file(manifest_path: &Path, package_path: &str) -> PathBuf {
    let raw = PathBuf::from(package_path);
    if raw.is_absolute() || raw.exists() {
        return raw;
    }
    manifest_path
        .parent()
        .map_or_else(|| raw.clone(), |parent| parent.join(&raw))
}

fn required_path(field: &str, value: &str) -> Result<PathBuf, ErrorData> {
    validate_non_empty(field, value)?;
    Ok(PathBuf::from(value))
}

fn validate_limit(limit: u32) -> Result<(), ErrorData> {
    if (1..=MAX_SEARCH_LIMIT).contains(&limit) {
        return Ok(());
    }
    Err(mcp_error(
        error_codes::TOOL_PARAMS_INVALID,
        format!("limit must be 1..={MAX_SEARCH_LIMIT}; got {limit}"),
    ))
}

fn validate_non_empty(field: &str, value: &str) -> Result<(), ErrorData> {
    if !value.trim().is_empty() {
        return Ok(());
    }
    Err(mcp_error(
        error_codes::TOOL_PARAMS_INVALID,
        format!("{field} must not be empty"),
    ))
}

fn validate_registry_id(field: &str, value: &str) -> Result<(), ErrorData> {
    validate_non_empty(field, value)?;
    if value.chars().all(|item| {
        item.is_ascii_lowercase() || item.is_ascii_digit() || matches!(item, '.' | '-' | '_')
    }) {
        return Ok(());
    }
    Err(mcp_error(
        error_codes::TOOL_PARAMS_INVALID,
        format!("{field} must use lowercase ascii letters, digits, '.', '-', or '_'"),
    ))
}

fn validate_disabled_state(value: &str) -> Result<(), ErrorData> {
    if matches!(value, "disabled" | "removed") {
        return Ok(());
    }
    Err(mcp_error(
        error_codes::TOOL_PARAMS_INVALID,
        format!("profile_registry_disable state must be disabled or removed; got {value:?}"),
    ))
}

fn normalized_query(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
}

fn lock_runtime<'a>(
    reflex_runtime: &'a Arc<Mutex<ReflexRuntime>>,
    context: &str,
) -> Result<MutexGuard<'a, ReflexRuntime>, ErrorData> {
    reflex_runtime.lock().map_err(|_error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("reflex runtime lock poisoned while {context}"),
        )
    })
}

fn set_object_field(value: &mut Value, field: &str, next: Value) {
    if let Value::Object(object) = value {
        object.insert(field.to_owned(), next);
    }
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn increment(counts: &mut BTreeMap<String, u64>, key: String) {
    *counts.entry(key).or_default() += 1;
}

fn source_key(source_id: &str) -> String {
    format!("{SOURCE_PREFIX}{source_id}")
}

fn package_key(package_id: &str, package_version: &str) -> String {
    format!("{PACKAGE_PREFIX}{package_id}/{package_version}")
}

fn profile_key(profile_id: &str, profile_version: &str) -> String {
    format!("{PROFILE_PREFIX}{profile_id}/{profile_version}")
}

fn installed_key(profile_id: &str) -> String {
    format!("{INSTALLED_PREFIX}{profile_id}")
}

fn compat_key(target_id: &str, profile_id: &str, profile_version: &str) -> String {
    format!("{COMPAT_PREFIX}{target_id}/{profile_id}/{profile_version}")
}

fn quality_link_key(profile_id: &str, profile_version: &str) -> String {
    format!("{QUALITY_LINK_PREFIX}{profile_id}/{profile_version}")
}

fn head_key(source_id: &str) -> String {
    format!("{HEAD_PREFIX}{source_id}")
}

fn quality_key(profile_id: &str) -> String {
    format!("profile_quality/v1/{profile_id}")
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn utf8_prefix(bytes: &[u8], max_chars: usize) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .take(max_chars)
        .collect()
}

const fn default_search_limit() -> u32 {
    DEFAULT_SEARCH_LIMIT
}

fn default_source_id() -> String {
    DEFAULT_SOURCE_ID.to_owned()
}

fn default_disabled_state() -> String {
    "disabled".to_owned()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err receives owned errors and this adapter preserves the simple call sites"
)]
fn storage_error(error: synapse_storage::StorageError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err receives owned errors and this adapter preserves the simple call sites"
)]
fn profile_error(error: ProfileError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err receives owned errors and this adapter preserves the simple call sites"
)]
fn decode_error(error: synapse_storage::StorageError) -> ErrorData {
    mcp_error(
        error_codes::STORAGE_CORRUPTED,
        format!("profile registry row decode failed: {error}"),
    )
}

fn registry_error(reason: &'static str, message: impl Into<String>) -> ErrorData {
    let message = message.into();
    ErrorData::new(
        ErrorCode(-32099),
        message,
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "reason": reason,
        })),
    )
}
