use std::time::{Duration, Instant};

use rmcp::{ErrorData, model::ErrorCode, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synapse_core::{ElementId, error_codes};

use crate::{
    m1::mcp_error,
    m2::postcondition::{ActPostcondition, default_verify_timeout_ms, text_signature},
};

const TOOL: &str = "act_set_value";
const SOURCE_OF_TRUTH: &str = "uia_value_pattern.value";
const TEXT_INTEGRITY_UIA_VALUE_PATTERN: &str = "uia_value_pattern_readback";

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActSetValueParams {
    pub element_id: ElementId,
    pub text: String,
    #[serde(default = "default_verify_timeout_ms")]
    #[schemars(default = "default_verify_timeout_ms", range(min = 50, max = 5000))]
    pub verify_timeout_ms: u32,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActSetValueResponse {
    pub ok: bool,
    pub method: String,
    pub backend_tier_used: String,
    pub required_foreground: bool,
    pub source_of_truth: String,
    pub requested_len: u32,
    pub before_len: u32,
    pub after_len: u32,
    pub requested_sha256: String,
    pub before_sha256: String,
    pub after_sha256: String,
    pub changed: bool,
    pub target_text_integrity: String,
    pub target_readback_required: bool,
    pub postcondition: ActPostcondition,
    pub elapsed_ms: u32,
}

pub async fn act_set_value(params: ActSetValueParams) -> Result<ActSetValueResponse, ErrorData> {
    let started = Instant::now();
    validate_set_value_params(&params)?;
    let requested_len = char_count(&params.text)?;
    let requested_sha256 = text_signature(&params.text);

    let before = synapse_a11y::element_value(&params.element_id).map_err(|error| {
        a11y_error_to_set_value_mcp(&params.element_id, "before_value_read", error, None)
    })?;
    let before_sha256 = text_signature(&before.value);
    let before_len = char_count(&before.value)?;
    let before_readback =
        value_readback_json(&params.element_id, &before.value, before.is_readonly);

    let set_readback = match synapse_a11y::set_element_value(&params.element_id, &params.text) {
        Ok(readback) => readback,
        Err(error) => {
            tokio::time::sleep(Duration::from_millis(u64::from(params.verify_timeout_ms))).await;
            let after = synapse_a11y::element_value(&params.element_id).ok();
            return Err(set_failed_error(
                &params,
                &before,
                &requested_sha256,
                requested_len,
                error,
                after,
            ));
        }
    };

    tokio::time::sleep(Duration::from_millis(u64::from(params.verify_timeout_ms))).await;
    let after = synapse_a11y::element_value(&params.element_id).map_err(|error| {
        a11y_error_to_set_value_mcp(
            &params.element_id,
            "after_value_read",
            error,
            Some(before_readback.clone()),
        )
    })?;
    let after_sha256 = text_signature(&after.value);
    let after_len = char_count(&after.value)?;
    let changed = before.value != after.value;

    if after.value != params.text {
        tracing::error!(
            code = error_codes::ACTION_POSTCONDITION_FAILED,
            tool = TOOL,
            element_id = %params.element_id,
            source_of_truth = SOURCE_OF_TRUTH,
            before_sha256,
            after_sha256,
            requested_sha256,
            before_len,
            after_len,
            requested_len,
            immediate_after_len = set_readback.after_value.chars().count(),
            "act_set_value ValuePattern.SetValue returned, but separate UIA value readback did not equal requested text"
        );
        return Err(postcondition_failed_set_value_error(
            &params,
            &before,
            &after,
            &set_readback,
            "separate UIA ValuePattern readback did not equal requested text after SetValue",
        ));
    }

    tracing::info!(
        code = "M2_ACT_SET_VALUE_READBACK",
        element_id = %params.element_id,
        method = %set_readback.method,
        before_len,
        after_len,
        requested_len,
        changed,
        source_of_truth = SOURCE_OF_TRUTH,
        "readback=act_set_value method=uia_value_pattern before_len={} after_len={} requested_len={} changed={}",
        before_len,
        after_len,
        requested_len,
        changed
    );

    Ok(ActSetValueResponse {
        ok: true,
        backend_tier_used: set_backend_tier(&set_readback).to_owned(),
        required_foreground: false,
        method: set_readback.method,
        source_of_truth: SOURCE_OF_TRUTH.to_owned(),
        requested_len,
        before_len,
        after_len,
        requested_sha256,
        before_sha256: before_sha256.clone(),
        after_sha256: after_sha256.clone(),
        changed,
        target_text_integrity: TEXT_INTEGRITY_UIA_VALUE_PATTERN.to_owned(),
        target_readback_required: false,
        postcondition: postcondition_verified_state(
            before_sha256,
            after_sha256,
            changed,
            if changed {
                "separate UIA ValuePattern readback equals requested text after SetValue"
            } else {
                "separate UIA ValuePattern readback already equaled requested text"
            },
        ),
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

pub fn act_set_value_request_details(params: &ActSetValueParams) -> Value {
    json!({
        "element_id": params.element_id.to_string(),
        "source_of_truth": SOURCE_OF_TRUTH,
        "requested_len": params.text.chars().count(),
        "requested_sha256": text_signature(&params.text),
        "verify_timeout_ms": params.verify_timeout_ms,
        "required_foreground": false,
    })
}

fn set_backend_tier(readback: &synapse_a11y::ElementValueSetReadback) -> &'static str {
    if readback.method == "uia_native_window_text_message" {
        "win32_message"
    } else {
        "uia"
    }
}

fn validate_set_value_params(params: &ActSetValueParams) -> Result<(), ErrorData> {
    if !(50..=5000).contains(&params.verify_timeout_ms) {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!(
                "act_set_value verify_timeout_ms must be in 50..=5000, got {}",
                params.verify_timeout_ms
            ),
        ));
    }
    let _ = char_count(&params.text)?;
    Ok(())
}

fn char_count(text: &str) -> Result<u32, ErrorData> {
    u32::try_from(text.chars().count()).map_err(|_err| {
        mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_set_value text has more than u32::MAX chars",
        )
    })
}

fn postcondition_verified_state(
    before_signature: String,
    after_signature: String,
    changed: bool,
    detail: impl Into<String>,
) -> ActPostcondition {
    ActPostcondition {
        status: "verified_state".to_owned(),
        observed_delta: Some(changed),
        source_of_truth: Some(SOURCE_OF_TRUTH.to_owned()),
        before_signature: Some(before_signature),
        after_signature: Some(after_signature),
        detail: Some(format!("{TOOL} {}", detail.into())),
    }
}

fn a11y_error_to_set_value_mcp(
    element_id: &ElementId,
    operation: &'static str,
    error: synapse_a11y::A11yError,
    prior_readback: Option<Value>,
) -> ErrorData {
    let code = error.code();
    let root_hwnd = element_id.parts().ok().map(|parts| parts.hwnd);
    tracing::error!(
        code,
        tool = TOOL,
        element_id = %element_id,
        root_hwnd,
        operation,
        source_of_truth = SOURCE_OF_TRUTH,
        detail = %error,
        "act_set_value UIA ValuePattern operation failed"
    );
    ErrorData::new(
        ErrorCode(-32099),
        format!("act_set_value {operation} failed for element {element_id}: {error}"),
        Some(json!({
            "code": code,
            "tool": TOOL,
            "operation": operation,
            "source_of_truth": SOURCE_OF_TRUTH,
            "element_id": element_id.to_string(),
            "root_hwnd": root_hwnd,
            "detail": error.to_string(),
            "prior_readback": prior_readback,
        })),
    )
}

fn set_failed_error(
    params: &ActSetValueParams,
    before: &synapse_a11y::ElementValueReadback,
    requested_sha256: &str,
    requested_len: u32,
    error: synapse_a11y::A11yError,
    after: Option<synapse_a11y::ElementValueReadback>,
) -> ErrorData {
    let after_value = after.as_ref().map(|readback| readback.value.as_str());
    let after_sha256 = after_value.map(text_signature);
    let after_len = after_value.map(|value| value.chars().count());
    let before_sha256 = text_signature(&before.value);
    let before_len = before.value.chars().count();
    let final_equals_requested = after_value.is_some_and(|value| value == params.text);
    let code = if final_equals_requested {
        error_codes::ACTION_POSTCONDITION_FAILED
    } else {
        error.code()
    };
    tracing::error!(
        code,
        tool = TOOL,
        element_id = %params.element_id,
        source_of_truth = SOURCE_OF_TRUTH,
        before_sha256,
        after_sha256,
        requested_sha256,
        before_len,
        after_len = after_len.unwrap_or(0),
        requested_len,
        set_error = %error,
        final_equals_requested,
        "act_set_value ValuePattern.SetValue failed; separate UIA value readback captured after failure"
    );
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "act_set_value ValuePattern.SetValue failed for element {}: {error}",
            params.element_id
        ),
        Some(json!({
            "code": code,
            "tool": TOOL,
            "operation": "set_value",
            "source_of_truth": SOURCE_OF_TRUTH,
            "element_id": params.element_id.to_string(),
            "before_len": before_len,
            "after_len": after_len,
            "requested_len": requested_len,
            "before_sha256": before_sha256,
            "after_sha256": after_sha256,
            "requested_sha256": requested_sha256,
            "final_equals_requested": final_equals_requested,
            "set_error_code": error.code(),
            "set_error": error.to_string(),
            "after_read_succeeded": after.is_some(),
        })),
    )
}

fn postcondition_failed_set_value_error(
    params: &ActSetValueParams,
    before: &synapse_a11y::ElementValueReadback,
    after: &synapse_a11y::ElementValueReadback,
    set_readback: &synapse_a11y::ElementValueSetReadback,
    detail: &'static str,
) -> ErrorData {
    let before_sha256 = text_signature(&before.value);
    let after_sha256 = text_signature(&after.value);
    let requested_sha256 = text_signature(&params.text);
    ErrorData::new(
        ErrorCode(-32099),
        format!("{TOOL} verify_delta Source-of-Truth postcondition failed: {detail}"),
        Some(json!({
            "code": error_codes::ACTION_POSTCONDITION_FAILED,
            "tool": TOOL,
            "source_of_truth": SOURCE_OF_TRUTH,
            "detail": detail,
            "verify_delta": {
                "before_signature": before_sha256,
                "after_signature": after_sha256,
                "readback": {
                    "element_id": params.element_id.to_string(),
                    "method": set_readback.method,
                    "before_len": before.value.chars().count(),
                    "immediate_after_len": set_readback.after_value.chars().count(),
                    "after_len": after.value.chars().count(),
                    "requested_len": params.text.chars().count(),
                    "before_sha256": text_signature(&before.value),
                    "immediate_after_sha256": text_signature(&set_readback.after_value),
                    "after_sha256": text_signature(&after.value),
                    "requested_sha256": requested_sha256,
                    "after_readonly": after.is_readonly,
                }
            }
        })),
    )
}

fn value_readback_json(element_id: &ElementId, value: &str, is_readonly: bool) -> Value {
    json!({
        "element_id": element_id.to_string(),
        "value_len": value.chars().count(),
        "value_sha256": text_signature(value),
        "is_readonly": is_readonly,
    })
}
