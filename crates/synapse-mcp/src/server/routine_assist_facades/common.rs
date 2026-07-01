use super::super::ErrorData;

use rmcp::model::ErrorCode;
use serde_json::{Value, json};
use synapse_core::error_codes;

pub(super) fn validate_exact_operation_spec(
    tool: &'static str,
    operation: &'static str,
    specs: &[(&'static str, bool)],
) -> Result<(), ErrorData> {
    let present = specs
        .iter()
        .filter_map(|(name, is_present)| is_present.then_some(*name))
        .collect::<Vec<_>>();
    if !present.contains(&operation) {
        return Err(facade_params_error(
            tool,
            operation,
            format!("{tool} operation={operation} requires a matching {operation} spec"),
            format!("pass {operation}={{...}} and no other operation spec"),
        ));
    }
    if present.len() != 1 {
        return Err(facade_params_error(
            tool,
            operation,
            format!("{tool} operation={operation} received invalid operation specs {present:?}"),
            format!("pass exactly one operation-specific spec matching {operation}"),
        ));
    }
    Ok(())
}

pub(super) fn facade_params_error(
    tool: &'static str,
    operation: &'static str,
    message: impl Into<String>,
    remediation: impl Into<String>,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        message.into(),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "tool": tool,
            "operation": operation,
            "source_id": "params",
            "source_of_truth": "typed facade params before delegated operation",
            "remediation": remediation.into(),
        })),
    )
}

pub(super) fn delegate_error(
    tool: &'static str,
    operation: &'static str,
    source_of_truth: &'static str,
    source_id: impl Into<String>,
    error: ErrorData,
    remediation: &'static str,
) -> ErrorData {
    let cause_code = error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
        .unwrap_or(error_codes::TOOL_INTERNAL_ERROR)
        .to_owned();
    let cause_data = error.data.clone().unwrap_or(Value::Null);
    ErrorData::new(
        error.code,
        error.message.to_string(),
        Some(json!({
            "code": cause_code,
            "tool": tool,
            "operation": operation,
            "source_of_truth": source_of_truth,
            "source_id": source_id.into(),
            "remediation": remediation,
            "cause": cause_data,
        })),
    )
}
