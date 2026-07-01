use rmcp::model::ErrorCode;
use serde_json::json;
use synapse_core::error_codes;

use crate::server::{
    ErrorData,
    command_audit::{CommandAuditQueryParams, CommandAuditQueryResponse},
};

use super::{
    AUDIT_SOT, AUDIT_TOOL,
    types::{AuditCommandQueryParams, AuditCommandQueryResponse, AuditCommandQueryRowSummary},
};
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

pub(super) fn summarize_command_query(
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
