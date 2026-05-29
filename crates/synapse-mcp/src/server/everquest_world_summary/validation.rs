use rmcp::ErrorData;
use serde::de::DeserializeOwned;
use synapse_core::error_codes;

use super::model::{
    EverQuestWorldSummaryHazard, EverQuestWorldSummaryParams, EverQuestWorldSummarySourceRef,
    MAX_ITEMS, MAX_SOURCE_REFS, MAX_TEXT_BYTES, NormalizedSummaryParams, ROW_PREFIX,
};
use crate::{m1::mcp_error, server::everquest_log::EVERQUEST_PROFILE_ID};

pub(super) fn normalize_params(
    mut params: EverQuestWorldSummaryParams,
) -> Result<NormalizedSummaryParams, ErrorData> {
    let profile_id = validate_profile_id(&params.profile_id)?;
    let summary_id = validate_id("summary_id", &params.summary_id)?;
    let state_row_key = normalize_required_text("state_row_key", &params.state_row_key)?;
    let install_root_override = params
        .install_root_override
        .take()
        .map(|value| normalize_required_text("install_root_override", &value))
        .transpose()?;
    normalize_count("max_exits", params.max_exits)?;
    normalize_count("max_landmarks", params.max_landmarks)?;
    normalize_count("max_transitions", params.max_transitions)?;
    normalize_count("max_hazards", params.max_hazards)?;
    if params.stale_after_seconds == 0 {
        return Err(params_error("stale_after_seconds must be >= 1"));
    }
    if params.source_refs.len() > MAX_SOURCE_REFS {
        return Err(params_error(format!(
            "source_refs must contain <= {MAX_SOURCE_REFS} items"
        )));
    }
    params.source_refs = normalize_source_refs(params.source_refs)?;
    if let Some(override_state) = params.state_override.as_mut() {
        override_state.zone_display_name = normalize_optional_text(
            "state_override.zone_display_name",
            override_state.zone_display_name.take(),
        )?;
        override_state.zone_short_name = normalize_optional_id(
            "state_override.zone_short_name",
            override_state.zone_short_name.take(),
        )?;
        validate_unit_interval("state_override.confidence", override_state.confidence)?;
        override_state.hazards = normalize_hazards(std::mem::take(&mut override_state.hazards))?;
        override_state.source_refs =
            normalize_source_refs(std::mem::take(&mut override_state.source_refs))?;
        override_state.redaction_probe_text = override_state
            .redaction_probe_text
            .take()
            .map(|value| normalize_probe_text(&value))
            .transpose()?;
    }
    Ok(NormalizedSummaryParams {
        row_key: world_summary_row_key(&profile_id, &summary_id),
        summary_id,
        profile_id,
        state_row_key,
        state_override: params.state_override,
        install_root_override,
        max_exits: params.max_exits,
        max_landmarks: params.max_landmarks,
        max_transitions: params.max_transitions,
        max_hazards: params.max_hazards,
        stale_after_seconds: params.stale_after_seconds,
        source_refs: params.source_refs,
    })
}

pub(super) fn decode_json_row<T>(bytes: &[u8], label: &str) -> Result<T, ErrorData>
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

pub(super) fn sanitize_summary(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.contains("you say,") || lower.contains("tells you,") || lower.contains("raw chat") {
        "[redacted chat summary]".to_owned()
    } else {
        value.trim().chars().take(MAX_TEXT_BYTES).collect()
    }
}

fn normalize_source_refs(
    refs: Vec<EverQuestWorldSummarySourceRef>,
) -> Result<Vec<EverQuestWorldSummarySourceRef>, ErrorData> {
    if refs.len() > MAX_SOURCE_REFS {
        return Err(params_error(format!(
            "source_refs must contain <= {MAX_SOURCE_REFS} items"
        )));
    }
    refs.into_iter()
        .enumerate()
        .map(|(index, source)| normalize_source_ref(&format!("source_refs[{index}]"), source))
        .collect()
}

fn normalize_source_ref(
    field: &str,
    source: EverQuestWorldSummarySourceRef,
) -> Result<EverQuestWorldSummarySourceRef, ErrorData> {
    Ok(EverQuestWorldSummarySourceRef {
        kind: validate_id(&format!("{field}.kind"), &source.kind)?,
        row_key: normalize_optional_text(&format!("{field}.row_key"), source.row_key)?,
        path: normalize_optional_text(&format!("{field}.path"), source.path)?,
        line_number: source.line_number,
        start_offset: source.start_offset,
        next_offset: source.next_offset,
        summary: source
            .summary
            .map(|value| {
                normalize_required_text(&format!("{field}.summary"), &sanitize_summary(&value))
            })
            .transpose()?,
    })
}

fn normalize_hazards(
    hazards: Vec<EverQuestWorldSummaryHazard>,
) -> Result<Vec<EverQuestWorldSummaryHazard>, ErrorData> {
    hazards
        .into_iter()
        .take(MAX_ITEMS)
        .map(|hazard| {
            Ok(EverQuestWorldSummaryHazard {
                code: validate_id("hazard.code", &hazard.code)?,
                severity: validate_id("hazard.severity", &hazard.severity)?,
                detail: normalize_required_text(
                    "hazard.detail",
                    &sanitize_summary(&hazard.detail),
                )?,
            })
        })
        .collect()
}

fn validate_profile_id(value: &str) -> Result<String, ErrorData> {
    let value = normalize_required_text("profile_id", value)?;
    if value != EVERQUEST_PROFILE_ID {
        return Err(params_error(format!(
            "profile_id must be {EVERQUEST_PROFILE_ID:?}"
        )));
    }
    Ok(value)
}

fn validate_id(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = normalize_required_text(field, value)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
    {
        return Err(params_error(format!(
            "{field} may contain only ASCII letters, digits, '.', '_', '-', and '/'"
        )));
    }
    Ok(value)
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

fn normalize_optional_text(
    field: &str,
    value: Option<String>,
) -> Result<Option<String>, ErrorData> {
    value
        .map(|value| normalize_required_text(field, &value))
        .transpose()
}

fn normalize_optional_id(field: &str, value: Option<String>) -> Result<Option<String>, ErrorData> {
    value.map(|value| validate_id(field, &value)).transpose()
}

fn normalize_probe_text(value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error(
            "state_override.redaction_probe_text must not be empty",
        ));
    }
    if value.len() > MAX_TEXT_BYTES {
        return Err(params_error(format!(
            "state_override.redaction_probe_text must be <= {MAX_TEXT_BYTES} bytes"
        )));
    }
    Ok(value.to_owned())
}

fn normalize_count(field: &str, value: usize) -> Result<(), ErrorData> {
    if value > MAX_ITEMS {
        return Err(params_error(format!("{field} must be <= {MAX_ITEMS}")));
    }
    Ok(())
}

fn validate_unit_interval(field: &str, value: f32) -> Result<(), ErrorData> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(params_error(format!("{field} must be between 0.0 and 1.0")));
    }
    Ok(())
}

fn world_summary_row_key(profile_id: &str, summary_id: &str) -> String {
    format!("{ROW_PREFIX}/{profile_id}/{summary_id}")
}

fn params_error(message: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, message.into())
}
