use std::fs;

use rmcp::model::ErrorCode;
use serde_json::{Value, json};
use synapse_core::error_codes;

use crate::{
    m3::permissions::{normalize_replay_path, replay_root},
    server::ErrorData,
};

use super::{
    REPLAY_SOT, REPLAY_TOOL,
    errors::{delegate_error, io_error, replay_artifact_corrupt_error},
    types::{
        ReplayArtifactInspectParams, ReplayArtifactInspectResponse, ReplayArtifactLineSummary,
    },
    util::{prefixed_sha256, string_field},
    validation::validate_replay_artifact_params,
};
pub(super) fn inspect_replay_artifact(
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
