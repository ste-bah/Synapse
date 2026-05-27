use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use rmcp::ErrorData;
use serde_json::Value;
use synapse_core::error_codes;
use synapse_profiles::{ProfileError, ProfileRuntime, ProfileStatus};
use synapse_reflex::ReflexRuntime;
use synapse_storage::{cf, decode_json, encode_json};

use crate::m1::mcp_error;

use super::{
    M3ToolStub,
    permissions::{Permission, RequiredPermissions, required},
};

mod aggregate;
mod model;

use aggregate::build_snapshot;
use model::{MAX_AUDIT_ROWS, MAX_STALE_AFTER_NS, STORED_PREFIX_CHARS};
pub use model::{
    ProfileCompatibilitySummary, ProfileQualityContribution, ProfileQualityCounts,
    ProfileQualityRates, ProfileQualityRedaction, ProfileQualityRefreshParams,
    ProfileQualityRefreshResponse, ProfileQualityScore, ProfileQualitySnapshot,
    ProfileQualitySource,
};

#[must_use]
pub const fn profile_quality_refresh() -> M3ToolStub {
    M3ToolStub::new("profile_quality_refresh")
}

#[must_use]
pub fn required_permissions_refresh(_params: &ProfileQualityRefreshParams) -> RequiredPermissions {
    required([
        Permission::ReadProfile,
        Permission::ReadStorage,
        Permission::WriteStorage,
    ])
}

pub fn refresh_profile_quality(
    profile_runtime: &ProfileRuntime,
    reflex_runtime: &Arc<Mutex<ReflexRuntime>>,
    params: &ProfileQualityRefreshParams,
) -> Result<ProfileQualityRefreshResponse, ErrorData> {
    validate_params(params)?;
    let profile = find_profile(profile_runtime, &params.profile_id)?;
    let key = quality_key(&params.profile_id);
    let runtime = reflex_runtime.lock().map_err(|_error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "reflex runtime lock poisoned while refreshing profile quality",
        )
    })?;
    let previous_hash = read_existing_hash(&runtime, &key)?;
    let rows = runtime
        .storage_cf_tail_rows(cf::CF_ACTION_LOG, params.max_audit_rows as usize)
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    let computed_snapshot = build_snapshot(&profile, rows, params, now_ns());
    let wrote_snapshot = previous_hash.as_deref() != Some(computed_snapshot.evidence_hash.as_str());
    if wrote_snapshot {
        let encoded = encode_json(&computed_snapshot).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("profile quality snapshot encode failed: {error}"),
            )
        })?;
        runtime
            .storage_put_profile_rows(vec![(key.clone(), encoded)])
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    }
    let stored = runtime
        .storage_profile_row(&key)
        .map_err(|error| mcp_error(error.code(), error.to_string()))?
        .ok_or_else(|| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "profile quality snapshot write did not persist",
            )
        })?;
    drop(runtime);
    let snapshot = decode_json::<ProfileQualitySnapshot>(&stored).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("stored profile quality snapshot decode failed after write: {error}"),
        )
    })?;

    Ok(ProfileQualityRefreshResponse {
        profile_id: params.profile_id.clone(),
        cf_name: cf::CF_PROFILES.to_owned(),
        key_hex: hex_encode(&key),
        wrote_snapshot,
        previous_evidence_hash: previous_hash,
        stored_value_len_bytes: stored.len() as u64,
        stored_value_utf8_prefix: utf8_prefix(&stored, STORED_PREFIX_CHARS),
        snapshot,
    })
}

fn validate_params(params: &ProfileQualityRefreshParams) -> Result<(), ErrorData> {
    if params.max_audit_rows == 0 || params.max_audit_rows > MAX_AUDIT_ROWS {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("profile_quality_refresh max_audit_rows must be 1..={MAX_AUDIT_ROWS}"),
        ));
    }
    if params.stale_after_ns == 0 || params.stale_after_ns > MAX_STALE_AFTER_NS {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("profile_quality_refresh stale_after_ns must be 1..={MAX_STALE_AFTER_NS}"),
        ));
    }
    Ok(())
}

fn find_profile(runtime: &ProfileRuntime, profile_id: &str) -> Result<ProfileStatus, ErrorData> {
    runtime
        .list(true)
        .map_err(|error| profile_error(&error))?
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| {
            mcp_error(
                error_codes::PROFILE_NOT_FOUND,
                format!("profile {profile_id} was not found"),
            )
        })
}

fn read_existing_hash(runtime: &ReflexRuntime, key: &[u8]) -> Result<Option<String>, ErrorData> {
    let Some(value) = runtime
        .storage_profile_row(key)
        .map_err(|error| mcp_error(error.code(), error.to_string()))?
    else {
        return Ok(None);
    };
    let existing = decode_json::<Value>(&value).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("stored profile quality snapshot decode failed: {error}"),
        )
    })?;
    Ok(existing
        .get("evidence_hash")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

fn quality_key(profile_id: &str) -> Vec<u8> {
    format!("profile_quality/v1/{profile_id}").into_bytes()
}

fn now_ns() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    u64::try_from(nanos).unwrap_or(u64::MAX)
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

fn profile_error(error: &ProfileError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}
