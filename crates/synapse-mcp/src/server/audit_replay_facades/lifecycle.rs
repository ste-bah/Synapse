use std::{
    collections::VecDeque,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use rmcp::model::ErrorCode;
use serde_json::{Value, json};
use synapse_core::error_codes;

use crate::{daemon_lifecycle, server::ErrorData};

use super::{
    AUDIT_SOT, AUDIT_TOOL,
    errors::{io_error, lifecycle_corrupt_error},
    types::{AuditLifecycleRowSummary, AuditLifecycleTailParams, AuditLifecycleTailResponse},
    util::{nested_string, prefixed_sha256, sha256_text, string_field},
    validation::validate_lifecycle_params,
};
pub(super) fn lifecycle_path(key: &str) -> Result<PathBuf, ErrorData> {
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

pub(super) fn read_lifecycle_tail(
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
