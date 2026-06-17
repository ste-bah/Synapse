//! `target_act` (#1005): a compact, high-level background-first computer-use
//! router.
//!
//! The raw tool surface is large, and model priors make low-level primitive
//! selection brittle and foreground-prone. `target_act` gives agents one
//! intent-named verb that routes to the correct *background-capable*,
//! session-targeted primitive and never to the human OS foreground. It is a thin
//! dispatcher: each verb delegates to the existing tool method, inheriting that
//! tool's target resolution, background routing, action audit (#1006), and
//! lease/foreground guards (#999/#1004) — so a normal (leaseless) session can
//! drive a background target through this router but cannot escalate to the
//! human foreground, which the delegate refuses before any mutation.

use super::{ErrorData, Json, Parameters, SynapseService, tool, tool_router};
use crate::m1::{
    CaptureScreenshotParams, CdpNavigateAction, CdpNavigateTabParams, ObserveParams, mcp_error,
};
use crate::m2::{ActClickParams, ActSetFieldTextParams, default_verify_timeout_ms};
use crate::m4::{ActRunShellExecutionMode, ActRunShellParams};
use rmcp::schemars::JsonSchema;
use rmcp::{RoleServer, service::RequestContext};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use synapse_core::{ElementId, error_codes};

const DEFAULT_TARGET_ACT_SHELL_TIMEOUT_MS: u64 = 30_000;
const TARGET_ACT_STATUS_OK: &str = "ok";
const TARGET_ACT_STATUS_VERIFY_NEEDED: &str = "verify_needed";
const TARGET_ACT_STATUS_REFUSED: &str = "refused";
const TARGET_ACT_STATUS_ERROR: &str = "error";
const TARGET_ACT_KNOWN_VERBS: &str = "read, screenshot, navigate, set_field, click, run_shell";

#[derive(Clone, Debug, JsonSchema)]
#[schemars(transparent)]
pub struct TargetActVerb(String);

impl<'de> Deserialize<'de> for TargetActVerb {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(Self(raw.trim().to_ascii_lowercase()))
    }
}

impl TargetActVerb {
    fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetActParams {
    /// The high-level operation to perform on the session target.
    pub verb: TargetActVerb,
    /// `navigate`: destination URL.
    #[serde(default)]
    pub url: Option<String>,
    /// `screenshot`: output file path.
    #[serde(default)]
    pub path: Option<String>,
    /// `set_field`: target element id (from observe/find).
    #[serde(default)]
    pub element_id: Option<String>,
    /// `set_field`: full replacement text (empty clears the field).
    #[serde(default)]
    pub text: Option<String>,
    /// `click`: click count for target element clicks. Defaults to 1; valid range is 1..=3.
    #[serde(default)]
    pub clicks: Option<u8>,
    /// `run_shell`: executable/program name (arguments go in `args`).
    #[serde(default)]
    pub command: Option<String>,
    /// `run_shell`: literal arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// `run_shell`: working directory.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// `run_shell`: inline wait budget (ms). Defaults to 30000.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetActResponse {
    pub verb: String,
    /// True only when the delegated primitive completed and its own postcondition accepted.
    pub ok: bool,
    /// `ok`, `verify_needed`, `refused`, or `error`.
    pub status: String,
    /// The background primitive this verb routed to.
    pub delegated_tool: String,
    pub routing: String,
    /// The delegated tool's full response.
    pub result: Value,
}

#[tool_router(router = background_router_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "High-level background-first computer-use router (#1005/#1033). One verb, routed to the correct background-capable, session-targeted primitive — never the human OS foreground. verb=read observes the target; verb=screenshot captures it; verb=navigate drives the owned browser target (Chrome bridge/CDP); verb=set_field replaces a web/UIA field's text by element id via background tiers; verb=click clicks a target element by semantic/background tiers with verify_delta enabled; verb=run_shell runs a command in the session workspace. Prefer this over raw act_* primitives: it inherits each delegate's target resolution, action audit, and lease/foreground guards, so a normal (leaseless) session can drive a background target but cannot seize the human foreground (the delegate fails closed before input). Mutating delegated failures are returned as ok=false with status=verify_needed/refused/error and the original structured error in result; no optimistic success. Bind a target first with set_target (discover one with window_list)."
    )]
    pub async fn target_act(
        &self,
        params: Parameters<TargetActParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<TargetActResponse>, ErrorData> {
        let params = params.0;
        let verb = params.verb.as_str().to_owned();
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "target_act",
            verb = verb.as_str(),
            "tool.invocation kind=target_act"
        );

        let (delegated_tool, ok, status, result) = match verb.as_str() {
            "read" => {
                let response = self
                    .observe(Parameters(ObserveParams::default()), request_context)
                    .await?;
                (
                    "observe",
                    true,
                    TARGET_ACT_STATUS_OK,
                    target_act_result(&response.0)?,
                )
            }
            "screenshot" => {
                let path = require_param(params.path, "screenshot", "path")?;
                let response = self
                    .capture_screenshot(
                        Parameters(CaptureScreenshotParams {
                            path,
                            region: None,
                            window_hwnd: None,
                            overwrite: true,
                        }),
                        request_context,
                    )
                    .await?;
                (
                    "capture_screenshot",
                    true,
                    TARGET_ACT_STATUS_OK,
                    target_act_result(&response.0)?,
                )
            }
            "navigate" => {
                let url = require_param(params.url, "navigate", "url")?;
                let response = self
                    .cdp_navigate_tab(
                        Parameters(CdpNavigateTabParams {
                            window_hwnd: None,
                            cdp_target_id: None,
                            action: CdpNavigateAction::Navigate,
                            url: Some(url),
                            wait_timeout_ms: None,
                            ignore_cache: None,
                        }),
                        request_context,
                    )
                    .await?;
                (
                    "cdp_navigate_tab",
                    true,
                    TARGET_ACT_STATUS_OK,
                    target_act_result(&response.0)?,
                )
            }
            "set_field" => {
                let element_id = require_param(params.element_id, "set_field", "element_id")?;
                let element_id = ElementId::parse(&element_id).map_err(|error| {
                    mcp_error(
                        error_codes::TOOL_PARAMS_INVALID,
                        format!("target_act verb=set_field element_id is invalid: {error}"),
                    )
                })?;
                let response = self
                    .act_set_field_text(
                        Parameters(ActSetFieldTextParams {
                            element_id,
                            text: params.text.unwrap_or_default(),
                            verify_timeout_ms: default_verify_timeout_ms(),
                        }),
                        request_context,
                    )
                    .await;
                target_act_delegate_response("act_set_field_text", response)?
            }
            "click" => {
                let element_id = require_param(params.element_id, "click", "element_id")?;
                let element_id = ElementId::parse(&element_id).map_err(|error| {
                    mcp_error(
                        error_codes::TOOL_PARAMS_INVALID,
                        format!("target_act verb=click element_id is invalid: {error}"),
                    )
                })?;
                let clicks = target_act_click_count(params.clicks)?;
                let click_params = target_act_click_params(element_id, clicks)?;
                let response = self
                    .act_click(Parameters(click_params), request_context)
                    .await;
                target_act_delegate_response("act_click", response)?
            }
            "run_shell" => {
                let command = require_param(params.command, "run_shell", "command")?;
                let response = self
                    .act_run_shell(
                        Parameters(ActRunShellParams {
                            command,
                            args: params.args,
                            working_dir: params.working_dir,
                            env: BTreeMap::new(),
                            timeout_ms: params
                                .timeout_ms
                                .unwrap_or(DEFAULT_TARGET_ACT_SHELL_TIMEOUT_MS),
                            execution_mode: ActRunShellExecutionMode::Inline,
                            durable_timeout_ms: None,
                            idempotency_key: None,
                        }),
                        request_context,
                    )
                    .await?;
                (
                    "act_run_shell",
                    true,
                    TARGET_ACT_STATUS_OK,
                    target_act_result(&response.0)?,
                )
            }
            other => return Err(target_act_unknown_verb_error(other)),
        };

        Ok(Json(TargetActResponse {
            verb: verb.as_str().to_owned(),
            ok,
            status: status.to_owned(),
            delegated_tool: delegated_tool.to_owned(),
            routing: "background-first; delegated to the session-targeted primitive, which inherits the action audit and lease/foreground guards and refuses the human foreground before input".to_owned(),
            result,
        }))
    }
}

fn require_param(value: Option<String>, verb: &str, field: &str) -> Result<String, ErrorData> {
    value.filter(|value| !value.is_empty()).ok_or_else(|| {
        mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("target_act verb={verb} requires a non-empty `{field}`"),
        )
    })
}

fn target_act_unknown_verb_error(verb: &str) -> ErrorData {
    mcp_error(
        error_codes::TOOL_PARAMS_INVALID,
        format!("target_act verb must be one of {TARGET_ACT_KNOWN_VERBS}; got {verb:?}"),
    )
}

fn target_act_result<T: Serialize>(value: &T) -> Result<Value, ErrorData> {
    serde_json::to_value(value).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("target_act failed to encode delegated tool result: {error}"),
        )
    })
}

fn target_act_delegate_response<T: Serialize>(
    delegated_tool: &'static str,
    result: Result<Json<T>, ErrorData>,
) -> Result<(&'static str, bool, &'static str, Value), ErrorData> {
    match result {
        Ok(response) => Ok((
            delegated_tool,
            true,
            TARGET_ACT_STATUS_OK,
            target_act_result(&response.0)?,
        )),
        Err(error) => {
            let status = target_act_error_status(&error);
            Ok((
                delegated_tool,
                false,
                status,
                target_act_error_result(delegated_tool, error),
            ))
        }
    }
}

fn target_act_click_count(clicks: Option<u8>) -> Result<u8, ErrorData> {
    let clicks = clicks.unwrap_or(1);
    if !(1..=3).contains(&clicks) {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("target_act verb=click clicks must be in 1..=3, got {clicks}"),
        ));
    }
    Ok(clicks)
}

fn target_act_click_params(element_id: ElementId, clicks: u8) -> Result<ActClickParams, ErrorData> {
    serde_json::from_value(json!({
        "target": {
            "element_id": element_id.to_string()
        },
        "clicks": clicks,
        "verify_delta": true,
        "verify_timeout_ms": default_verify_timeout_ms()
    }))
    .map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("target_act failed to construct act_click params: {error}"),
        )
    })
}

fn target_act_error_result(delegated_tool: &'static str, error: ErrorData) -> Value {
    let code = target_act_error_code(&error)
        .unwrap_or(error_codes::TOOL_INTERNAL_ERROR)
        .to_owned();
    json!({
        "error": {
            "delegated_tool": delegated_tool,
            "code": code,
            "message": error.message.to_string(),
            "data": error.data,
        }
    })
}

fn target_act_error_status(error: &ErrorData) -> &'static str {
    match target_act_error_code(error) {
        Some(
            error_codes::ACTION_NO_OBSERVED_DELTA
            | error_codes::ACTION_FOREGROUND_LOST
            | error_codes::ACTION_POSTCONDITION_FAILED
            | error_codes::ACTION_VERIFY_SURFACE_UNAVAILABLE,
        ) => TARGET_ACT_STATUS_VERIFY_NEEDED,
        Some(
            error_codes::ACTION_ELEMENT_NOT_RESOLVED
            | error_codes::ACTION_ELEMENT_PATTERN_UNSUPPORTED
            | error_codes::ACTION_ELEMENT_VALUE_READ_ONLY
            | error_codes::ACTION_FOREGROUND_LEASE_BUSY
            | error_codes::ACTION_FOREGROUND_LEASE_NOT_HELD
            | error_codes::ACTION_TARGET_INVALID
            | error_codes::A11Y_ELEMENT_STALE
            | error_codes::FOREGROUND_ACTIVATION_REFUSED
            | error_codes::TARGET_CO_OWNED
            | error_codes::TARGET_WINDOW_NOT_FOUND
            | error_codes::TOOL_PARAMS_INVALID
            | error_codes::TRANSIENT_ELEMENT_EXPIRED,
        ) => TARGET_ACT_STATUS_REFUSED,
        _ => TARGET_ACT_STATUS_ERROR,
    }
}

fn target_act_error_code(error: &ErrorData) -> Option<&str> {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::schemars::schema_for;

    #[test]
    fn target_act_verb_click_deserializes() {
        let params: TargetActParams = serde_json::from_value(json!({
            "verb": "click",
            "element_id": "0x2a:0000000000000001",
            "clicks": 2
        }))
        .expect("click params should deserialize");

        assert_eq!(params.verb.as_str(), "click");
        assert_eq!(params.clicks, Some(2));
    }

    #[test]
    fn target_act_verb_schema_is_forward_compatible_string() {
        let schema = serde_json::to_value(schema_for!(TargetActParams))
            .unwrap_or_else(|error| panic!("target_act params schema should serialize: {error}"));
        let schema_text = schema.to_string();

        assert!(
            schema_text.contains("\"verb\""),
            "target_act schema must include verb: {schema_text}"
        );
        assert!(
            schema_text.contains("\"type\":\"string\""),
            "target_act verb schema must be an open string: {schema_text}"
        );
        assert!(
            !schema_text.contains("\"enum\""),
            "target_act verb schema must not be a closed enum: {schema_text}"
        );
    }

    #[test]
    fn target_act_unknown_verb_is_runtime_validation_error() {
        let params: TargetActParams = serde_json::from_value(json!({
            "verb": "future_dashboard_action"
        }))
        .expect("future target_act verb should deserialize so clients do not stale on schema");
        let error = target_act_unknown_verb_error(params.verb.as_str());

        assert_eq!(
            target_act_error_code(&error),
            Some(error_codes::TOOL_PARAMS_INVALID)
        );
        assert!(
            error.message.contains("future_dashboard_action"),
            "unknown verb error should name the rejected verb: {}",
            error.message
        );
    }

    #[test]
    fn target_act_click_count_rejects_out_of_range() {
        let error = target_act_click_count(Some(4)).expect_err("clicks=4 should fail");

        assert_eq!(
            target_act_error_code(&error),
            Some(error_codes::TOOL_PARAMS_INVALID)
        );
    }

    #[test]
    fn target_act_errors_classify_verify_needed() {
        for code in [
            error_codes::ACTION_NO_OBSERVED_DELTA,
            error_codes::ACTION_FOREGROUND_LOST,
            error_codes::ACTION_POSTCONDITION_FAILED,
            error_codes::ACTION_VERIFY_SURFACE_UNAVAILABLE,
        ] {
            let error = mcp_error(code, "synthetic delegated postcondition failure");
            assert_eq!(
                target_act_error_status(&error),
                TARGET_ACT_STATUS_VERIFY_NEEDED
            );
        }
    }

    #[test]
    fn target_act_errors_classify_refusal() {
        for code in [
            error_codes::ACTION_ELEMENT_PATTERN_UNSUPPORTED,
            error_codes::ACTION_ELEMENT_VALUE_READ_ONLY,
            error_codes::FOREGROUND_ACTIVATION_REFUSED,
        ] {
            let error = mcp_error(code, "synthetic delegated refusal");
            assert_eq!(target_act_error_status(&error), TARGET_ACT_STATUS_REFUSED);
        }
    }

    #[test]
    fn target_act_error_result_preserves_delegated_data() {
        let error = mcp_error(error_codes::ACTION_POSTCONDITION_FAILED, "mismatch");
        let result = target_act_error_result("act_set_field_text", error);

        assert_eq!(
            result.pointer("/error/code").and_then(Value::as_str),
            Some(error_codes::ACTION_POSTCONDITION_FAILED)
        );
        assert_eq!(
            result
                .pointer("/error/delegated_tool")
                .and_then(Value::as_str),
            Some("act_set_field_text")
        );
        assert_eq!(
            result.pointer("/error/data/code").and_then(Value::as_str),
            Some(error_codes::ACTION_POSTCONDITION_FAILED)
        );
    }
}
