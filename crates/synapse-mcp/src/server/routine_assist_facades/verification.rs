use super::super::{
    ErrorData, Json, Parameters, SynapseService, verification::VerificationAuditParams,
    verification::VerificationAuditResponse, verification::VerificationBindParams,
    verification::VerificationBindResponse, verification::VerificationInboxParams,
    verification::VerificationInboxResponse, verification::VerificationPollParams,
    verification::VerificationPollResponse, verification::VerificationSourcesParams,
    verification::VerificationSourcesResponse,
};
use super::common::{delegate_error, facade_params_error, validate_exact_operation_spec};

use rmcp::{RoleServer, schemars::JsonSchema, service::RequestContext};
use serde::{Deserialize, Serialize};

pub(super) const VERIFICATION_TOOL: &str = "verification";
const VERIFICATION_SOURCE_OF_TRUTH: &str =
    "CF_KV verification/audit/v1 + verification/binding/v1 + bound Chrome tab readback";

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOperation {
    Inbox,
    Poll,
    Audit,
    Bind,
    Sources,
}

impl VerificationOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Poll => "poll",
            Self::Audit => "audit",
            Self::Bind => "bind",
            Self::Sources => "sources",
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationParams {
    pub operation: VerificationOperation,
    #[serde(default)]
    pub inbox: Option<VerificationInboxParams>,
    #[serde(default)]
    pub poll: Option<VerificationPollParams>,
    #[serde(default)]
    pub audit: Option<VerificationAuditParams>,
    #[serde(default)]
    pub bind: Option<VerificationBindParams>,
    #[serde(default)]
    pub sources: Option<VerificationSourcesParams>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationResponse {
    pub operation: VerificationOperation,
    pub source_of_truth: String,
    pub readback_source_of_truth: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox: Option<VerificationInboxResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<VerificationPollResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit: Option<VerificationAuditResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<VerificationBindResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<VerificationSourcesResponse>,
}

pub(super) async fn handle(
    service: &SynapseService,
    params: Parameters<VerificationParams>,
    request_context: RequestContext<RoleServer>,
) -> Result<Json<VerificationResponse>, ErrorData> {
    validate_verification_facade_params(&params.0)?;
    let operation = params.0.operation;
    tracing::info!(
        code = "MCP_TOOL_INVOCATION",
        kind = VERIFICATION_TOOL,
        operation = operation.as_str(),
        "tool.invocation kind=verification"
    );
    match operation {
        VerificationOperation::Inbox => {
            let spec = params
                .0
                .inbox
                .ok_or_else(|| missing_verification_spec("inbox"))?;
            let source_id = verification_source_id(spec.source.as_deref());
            let response = service
                    .verification_inbox(Parameters(spec), request_context)
                    .await
                    .map_err(|error| {
                        verification_delegate_error(
                            operation,
                            source_id,
                            error,
                            "bind/select the logged-in verification tab and inspect verification/audit/v1 rows",
                        )
                    })?
                    .0;
            Ok(Json(verification_response(
                operation,
                format!(
                    "CF_KV verification audit source={} codes={} audit_key={}",
                    response.source,
                    response.codes.len(),
                    response.audit_key
                ),
                |out| out.inbox = Some(response),
            )))
        }
        VerificationOperation::Poll => {
            let spec = params
                .0
                .poll
                .ok_or_else(|| missing_verification_spec("poll"))?;
            let source_id = verification_source_id(spec.source.as_deref());
            let response = service
                    .verification_poll(Parameters(spec), request_context)
                    .await
                    .map_err(|error| {
                        verification_delegate_error(
                            operation,
                            source_id,
                            error,
                            "bind/select the logged-in verification tab and inspect verification/audit/v1 rows",
                        )
                    })?
                    .0;
            Ok(Json(verification_response(
                operation,
                format!(
                    "CF_KV verification poll source={} matched={} timed_out={} polls={} audit_key={}",
                    response.source,
                    response.matched,
                    response.timed_out,
                    response.polls,
                    response.audit_key
                ),
                |out| out.poll = Some(response),
            )))
        }
        VerificationOperation::Audit => {
            let spec = params
                .0
                .audit
                .ok_or_else(|| missing_verification_spec("audit"))?;
            let response = service
                .verification_audit(Parameters(spec), request_context)
                .await
                .map_err(|error| {
                    verification_delegate_error(
                        operation,
                        "verification/audit/v1",
                        error,
                        "inspect the verification audit prefix and storage health",
                    )
                })?
                .0;
            Ok(Json(verification_response(
                operation,
                format!("CF_KV verification audit rows={}", response.count),
                |out| out.audit = Some(response),
            )))
        }
        VerificationOperation::Bind => {
            let spec = params
                .0
                .bind
                .ok_or_else(|| missing_verification_spec("bind"))?;
            let source_id = spec.source.clone();
            let response = service
                    .verification_bind(Parameters(spec), request_context)
                    .await
                    .map_err(|error| {
                        verification_delegate_error(
                            operation,
                            source_id,
                            error,
                            "provide a non-empty source and inspect verification/binding/v1 row readback",
                        )
                    })?
                    .0;
            Ok(Json(verification_response(
                operation,
                format!(
                    "CF_KV verification binding source={} enabled={} cf_key={}",
                    response.binding.source, response.binding.enabled, response.cf_key
                ),
                |out| out.bind = Some(response),
            )))
        }
        VerificationOperation::Sources => {
            let spec = params
                .0
                .sources
                .ok_or_else(|| missing_verification_spec("sources"))?;
            let response = service
                .verification_sources(Parameters(spec), request_context)
                .await
                .map_err(|error| {
                    verification_delegate_error(
                        operation,
                        "verification/binding/v1",
                        error,
                        "inspect verification binding prefix and storage health",
                    )
                })?
                .0;
            Ok(Json(verification_response(
                operation,
                format!("CF_KV verification sources count={}", response.count),
                |out| out.sources = Some(response),
            )))
        }
    }
}

pub(super) fn validate_verification_facade_params(
    params: &VerificationParams,
) -> Result<(), ErrorData> {
    validate_exact_operation_spec(
        VERIFICATION_TOOL,
        params.operation.as_str(),
        &[
            ("inbox", params.inbox.is_some()),
            ("poll", params.poll.is_some()),
            ("audit", params.audit.is_some()),
            ("bind", params.bind.is_some()),
            ("sources", params.sources.is_some()),
        ],
    )
}

fn missing_verification_spec(operation: &'static str) -> ErrorData {
    facade_params_error(
        VERIFICATION_TOOL,
        operation,
        format!("verification operation={operation} requires a {operation} spec"),
        format!("pass {operation}={{...}} and no other operation spec"),
    )
}

fn verification_delegate_error(
    operation: VerificationOperation,
    source_id: impl Into<String>,
    error: ErrorData,
    remediation: &'static str,
) -> ErrorData {
    delegate_error(
        VERIFICATION_TOOL,
        operation.as_str(),
        VERIFICATION_SOURCE_OF_TRUTH,
        source_id,
        error,
        remediation,
    )
}

fn verification_response(
    operation: VerificationOperation,
    readback_source_of_truth: String,
    populate: impl FnOnce(&mut VerificationResponse),
) -> VerificationResponse {
    let mut response = VerificationResponse {
        operation,
        source_of_truth: format!(
            "{VERIFICATION_SOURCE_OF_TRUTH} + delegated verification operation={}",
            operation.as_str()
        ),
        readback_source_of_truth,
        inbox: None,
        poll: None,
        audit: None,
        bind: None,
        sources: None,
    };
    populate(&mut response);
    response
}

fn verification_source_id(source: Option<&str>) -> String {
    format!("source:{}", source.unwrap_or("unspecified"))
}
