use super::super::{
    ErrorData, Json, Parameters, SynapseService, reality::ObserveDeltaParams,
    reality::ObserveDeltaResponse, reality::RealityAuditParams, reality::RealityAuditResponse,
    reality::RealityBaselineParams, reality::RealityBaselineResponse,
};
use super::common::{delegate_error, facade_params_error, validate_exact_operation_spec};

use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(super) const REALITY_TOOL: &str = "reality";
const REALITY_SOURCE_OF_TRUTH: &str =
    "CF_KV reality baseline/delta/audit rows + physical observation readback";

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RealityOperation {
    Baseline,
    Delta,
    Audit,
}

impl RealityOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Delta => "delta",
            Self::Audit => "audit",
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealityParams {
    pub operation: RealityOperation,
    #[serde(default)]
    pub baseline: Option<RealityBaselineParams>,
    #[serde(default)]
    pub delta: Option<ObserveDeltaParams>,
    #[serde(default)]
    pub audit: Option<RealityAuditParams>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealityResponse {
    pub operation: RealityOperation,
    pub source_of_truth: String,
    pub readback_source_of_truth: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<RealityBaselineResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<ObserveDeltaResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit: Option<RealityAuditResponse>,
}

pub(super) async fn handle(
    service: &SynapseService,
    params: Parameters<RealityParams>,
) -> Result<Json<RealityResponse>, ErrorData> {
    validate_reality_facade_params(&params.0)?;
    let operation = params.0.operation;
    tracing::info!(
        code = "MCP_TOOL_INVOCATION",
        kind = REALITY_TOOL,
        operation = operation.as_str(),
        "tool.invocation kind=reality"
    );
    match operation {
        RealityOperation::Baseline => {
            let spec = params
                .0
                .baseline
                .ok_or_else(|| missing_reality_spec("baseline"))?;
            let source_id = reality_source_id(spec.profile_id.as_deref(), spec.epoch_id.as_deref());
            let response = service
                    .reality_baseline(Parameters(spec))
                    .await
                    .map_err(|error| {
                        reality_delegate_error(
                            operation,
                            source_id,
                            error,
                            "fix baseline profile/epoch params and inspect CF_KV reality head/baseline rows",
                        )
                    })?
                    .0;
            Ok(Json(reality_response(
                operation,
                format!(
                    "CF_KV reality baseline profile_key={} epoch_id={} created={} rows={}",
                    response.profile_key,
                    response.baseline.epoch_id,
                    response.created,
                    response.readback_rows.len()
                ),
                |out| out.baseline = Some(response),
            )))
        }
        RealityOperation::Delta => {
            let spec = params
                .0
                .delta
                .ok_or_else(|| missing_reality_spec("delta"))?;
            let source_id =
                reality_source_id(spec.profile_id.as_deref(), spec.since_epoch.as_deref());
            let response = service
                .observe_delta(Parameters(spec))
                .await
                .map_err(|error| {
                    reality_delegate_error(
                        operation,
                        source_id,
                        error,
                        "fix delta cursor/profile params and inspect CF_KV reality delta/head rows",
                    )
                })?
                .0;
            Ok(Json(reality_response(
                operation,
                format!(
                    "CF_KV reality delta epoch_id={} from_seq={} to_seq={} deltas={} rows={} baseline_required={} rebase_required={}",
                    response.epoch_id.as_deref().unwrap_or("<none>"),
                    response
                        .from_seq
                        .map_or_else(|| "<none>".to_owned(), |value| value.to_string()),
                    response
                        .to_seq
                        .map_or_else(|| "<none>".to_owned(), |value| value.to_string()),
                    response.deltas.len(),
                    response.readback_rows.len(),
                    response.baseline_required,
                    response.rebase_required
                ),
                |out| out.delta = Some(response),
            )))
        }
        RealityOperation::Audit => {
            let spec = params
                .0
                .audit
                .ok_or_else(|| missing_reality_spec("audit"))?;
            let source_id = reality_source_id(spec.profile_id.as_deref(), spec.epoch_id.as_deref());
            let response = service
                    .reality_audit(Parameters(spec))
                    .await
                    .map_err(|error| {
                        reality_delegate_error(
                            operation,
                            source_id,
                            error,
                            "fix audit profile/assumption params and inspect CF_KV reality audit/head rows",
                        )
                    })?
                    .0;
            Ok(Json(reality_response(
                operation,
                format!(
                    "CF_KV reality audit profile_key={} row_key={} drift_items={} baseline_required={} rebase_required={}",
                    response.profile_key,
                    response.row_key,
                    response.audit.drift_items.len(),
                    response.baseline_required,
                    response.rebase_required
                ),
                |out| out.audit = Some(response),
            )))
        }
    }
}

pub(super) fn validate_reality_facade_params(params: &RealityParams) -> Result<(), ErrorData> {
    validate_exact_operation_spec(
        REALITY_TOOL,
        params.operation.as_str(),
        &[
            ("baseline", params.baseline.is_some()),
            ("delta", params.delta.is_some()),
            ("audit", params.audit.is_some()),
        ],
    )
}

fn missing_reality_spec(operation: &'static str) -> ErrorData {
    facade_params_error(
        REALITY_TOOL,
        operation,
        format!("reality operation={operation} requires a {operation} spec"),
        format!("pass {operation}={{...}} and no other operation spec"),
    )
}

fn reality_delegate_error(
    operation: RealityOperation,
    source_id: impl Into<String>,
    error: ErrorData,
    remediation: &'static str,
) -> ErrorData {
    delegate_error(
        REALITY_TOOL,
        operation.as_str(),
        REALITY_SOURCE_OF_TRUTH,
        source_id,
        error,
        remediation,
    )
}

fn reality_response(
    operation: RealityOperation,
    readback_source_of_truth: String,
    populate: impl FnOnce(&mut RealityResponse),
) -> RealityResponse {
    let mut response = RealityResponse {
        operation,
        source_of_truth: format!(
            "{REALITY_SOURCE_OF_TRUTH} + delegated reality operation={}",
            operation.as_str()
        ),
        readback_source_of_truth,
        baseline: None,
        delta: None,
        audit: None,
    };
    populate(&mut response);
    response
}

fn reality_source_id(profile_id: Option<&str>, epoch_id: Option<&str>) -> String {
    format!(
        "profile:{} epoch:{}",
        profile_id.unwrap_or("<auto>"),
        epoch_id.unwrap_or("<auto>")
    )
}
