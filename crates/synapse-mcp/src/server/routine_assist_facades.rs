use super::{
    ErrorData, Json, Parameters, SynapseService, reality::ObserveDeltaParams,
    reality::ObserveDeltaResponse, reality::RealityAuditParams, reality::RealityAuditResponse,
    reality::RealityBaselineParams, reality::RealityBaselineResponse, tool, tool_router,
    verification::VerificationAuditParams, verification::VerificationAuditResponse,
    verification::VerificationBindParams, verification::VerificationBindResponse,
    verification::VerificationInboxParams, verification::VerificationInboxResponse,
    verification::VerificationPollParams, verification::VerificationPollResponse,
    verification::VerificationSourcesParams, verification::VerificationSourcesResponse,
};

use crate::m3::{
    armed_routines::{ArmedRoutineTickParams, ArmedRoutineTickResponse},
    intent::{IntentCurrentParams, IntentCurrentResponse},
    intent_events::{IntentDetectOutcome, IntentDetectTickParams},
    profile_authoring::{RoutineAutomateParams, RoutineAutomateResponse},
    routines::{
        RoutineFeedbackParams, RoutineFeedbackResponse, RoutineInspectParams,
        RoutineInspectResponse, RoutineLabelExportParams, RoutineLabelExportResponse,
        RoutineListParams, RoutineListResponse, RoutineMineParams, RoutineMineResponse,
        RoutineUpdateParams, RoutineUpdateResponse,
    },
    suggestions::{
        SuggestionAcceptParams, SuggestionAcceptResponse, SuggestionListParams,
        SuggestionListResponse, SuggestionTickParams, SuggestionTickResponse,
    },
};
use rmcp::{RoleServer, model::ErrorCode, schemars::JsonSchema, service::RequestContext};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synapse_core::error_codes;

const ROUTINE_TOOL: &str = "routine";
const ASSIST_TOOL: &str = "assist";
const REALITY_TOOL: &str = "reality";
const VERIFICATION_TOOL: &str = "verification";
const ROUTINE_SOURCE_OF_TRUTH: &str =
    "CF_ROUTINES + CF_ROUTINE_STATE + CF_KV routine automation/armed rows";
const ASSIST_SOURCE_OF_TRUTH: &str =
    "CF_KV suggestion/v1 + intent tracker/events + CF_ROUTINES/CF_ROUTINE_STATE";
const REALITY_SOURCE_OF_TRUTH: &str =
    "CF_KV reality baseline/delta/audit rows + physical observation readback";
const VERIFICATION_SOURCE_OF_TRUTH: &str =
    "CF_KV verification/audit/v1 + verification/binding/v1 + bound Chrome tab readback";

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineOperation {
    Mine,
    List,
    Inspect,
    Update,
    Feedback,
    Label,
    Automate,
    ArmedTick,
}

impl RoutineOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Mine => "mine",
            Self::List => "list",
            Self::Inspect => "inspect",
            Self::Update => "update",
            Self::Feedback => "feedback",
            Self::Label => "label",
            Self::Automate => "automate",
            Self::ArmedTick => "armed_tick",
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutineParams {
    pub operation: RoutineOperation,
    #[serde(default)]
    pub mine: Option<RoutineMineParams>,
    #[serde(default)]
    pub list: Option<RoutineListParams>,
    #[serde(default)]
    pub inspect: Option<RoutineInspectParams>,
    #[serde(default)]
    pub update: Option<RoutineUpdateParams>,
    #[serde(default)]
    pub feedback: Option<RoutineFeedbackParams>,
    #[serde(default)]
    pub label: Option<RoutineLabelExportParams>,
    #[serde(default)]
    pub automate: Option<RoutineAutomateParams>,
    #[serde(default)]
    pub armed_tick: Option<ArmedRoutineTickParams>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutineResponse {
    pub operation: RoutineOperation,
    pub source_of_truth: String,
    pub readback_source_of_truth: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mine: Option<RoutineMineResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<RoutineListResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspect: Option<RoutineInspectResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<RoutineUpdateResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<RoutineFeedbackResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<RoutineLabelExportResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automate: Option<RoutineAutomateResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armed_tick: Option<ArmedRoutineTickResponse>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistOperation {
    Intent,
    Detect,
    SuggestionTick,
    SuggestionList,
    SuggestionAccept,
}

impl AssistOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Detect => "detect",
            Self::SuggestionTick => "suggestion_tick",
            Self::SuggestionList => "suggestion_list",
            Self::SuggestionAccept => "suggestion_accept",
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssistParams {
    pub operation: AssistOperation,
    #[serde(default)]
    pub intent: Option<IntentCurrentParams>,
    #[serde(default)]
    pub detect: Option<IntentDetectTickParams>,
    #[serde(default)]
    pub suggestion_tick: Option<SuggestionTickParams>,
    #[serde(default)]
    pub suggestion_list: Option<SuggestionListParams>,
    #[serde(default)]
    pub suggestion_accept: Option<SuggestionAcceptParams>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssistResponse {
    pub operation: AssistOperation,
    pub source_of_truth: String,
    pub readback_source_of_truth: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<IntentCurrentResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detect: Option<IntentDetectOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion_tick: Option<SuggestionTickResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion_list: Option<SuggestionListResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion_accept: Option<SuggestionAcceptResponse>,
}

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

#[tool_router(router = routine_assist_facade_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Facade for routine mining, listing, inspection, lifecycle updates, feedback, labels, automation candidate generation, and armed routine ticks in the <=40 public MCP surface. operation is a strict enum; exactly one matching operation spec is accepted. Delegates to the existing routine_* implementation paths and returns CF_ROUTINES/CF_ROUTINE_STATE/CF_KV readback metadata."
    )]
    pub async fn routine(
        &self,
        params: Parameters<RoutineParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<RoutineResponse>, ErrorData> {
        validate_routine_facade_params(&params.0)?;
        let operation = params.0.operation;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = ROUTINE_TOOL,
            operation = operation.as_str(),
            "tool.invocation kind=routine"
        );
        match operation {
            RoutineOperation::Mine => {
                let spec = params.0.mine.ok_or_else(|| missing_routine_spec("mine"))?;
                let source_id = routine_range_source_id(spec.start_ts_ns, spec.end_ts_ns);
                let response = self
                    .routine_mine(Parameters(spec))
                    .await
                    .map_err(|error| {
                        routine_delegate_error(
                            operation,
                            source_id,
                            error,
                            "fix routine mining bounds and inspect CF_EPISODES, CF_ROUTINES, and CF_ROUTINE_STATE",
                        )
                    })?
                    .0;
                Ok(Json(routine_response(
                    operation,
                    format!(
                        "CF_ROUTINES mine routines_written={} routines_deleted={} state_created={} state_updated={} dry_run={}",
                        response.routines_written,
                        response.routines_deleted,
                        response.state_rows_created,
                        response.state_rows_updated,
                        response.dry_run
                    ),
                    |out| out.mine = Some(response),
                )))
            }
            RoutineOperation::List => {
                let spec = params.0.list.ok_or_else(|| missing_routine_spec("list"))?;
                let source_id = routine_list_source_id(&spec);
                let response = self
                    .routine_list(Parameters(spec))
                    .await
                    .map_err(|error| {
                        routine_delegate_error(
                            operation,
                            source_id,
                            error,
                            "narrow routine list filters and inspect CF_ROUTINES plus CF_ROUTINE_STATE rows",
                        )
                    })?
                    .0;
                Ok(Json(routine_response(
                    operation,
                    format!(
                        "CF_ROUTINES list total_mined={} total_state_rows={} matched={} returned={} truncated={}",
                        response.total_mined,
                        response.total_state_rows,
                        response.matched,
                        response.returned,
                        response.truncated
                    ),
                    |out| out.list = Some(response),
                )))
            }
            RoutineOperation::Inspect => {
                let spec = params
                    .0
                    .inspect
                    .ok_or_else(|| missing_routine_spec("inspect"))?;
                let source_id = spec.routine_id.clone();
                let response = self
                    .routine_inspect(Parameters(spec))
                    .await
                    .map_err(|error| {
                        routine_delegate_error(
                            operation,
                            source_id,
                            error,
                            "provide an existing routine_id and inspect CF_ROUTINES plus CF_ROUTINE_STATE",
                        )
                    })?
                    .0;
                Ok(Json(routine_response(
                    operation,
                    format!(
                        "CF_ROUTINES/CF_ROUTINE_STATE routine_id={} mined={} state_row_exists={} armed_present={}",
                        response.routine_id,
                        response.mined,
                        response.state_row_exists,
                        response.armed.is_some()
                    ),
                    |out| out.inspect = Some(response),
                )))
            }
            RoutineOperation::Update => {
                let spec = params
                    .0
                    .update
                    .ok_or_else(|| missing_routine_spec("update"))?;
                let source_id = spec.routine_id.clone();
                let response = self
                    .routine_update(Parameters(spec), request_context)
                    .await
                    .map_err(|error| {
                        routine_delegate_error(
                            operation,
                            source_id,
                            error,
                            "fix routine lifecycle/arming params and inspect CF_ROUTINE_STATE plus armed_routine/v1 rows",
                        )
                    })?
                    .0;
                Ok(Json(routine_response(
                    operation,
                    format!(
                        "CF_ROUTINE_STATE routine_id={} action={:?} lifecycle_before={:?} lifecycle_after={:?} state_row_created={}",
                        response.routine_id,
                        response.action,
                        response.lifecycle_before,
                        response.lifecycle_after,
                        response.state_row_created
                    ),
                    |out| out.update = Some(response),
                )))
            }
            RoutineOperation::Feedback => {
                let spec = params
                    .0
                    .feedback
                    .ok_or_else(|| missing_routine_spec("feedback"))?;
                let source_id = spec.routine_id.clone();
                let response = self
                    .routine_feedback(Parameters(spec), request_context)
                    .await
                    .map_err(|error| {
                        routine_delegate_error(
                            operation,
                            source_id,
                            error,
                            "provide an existing routine_id/outcome and inspect the CF_ROUTINE_STATE feedback history",
                        )
                    })?
                    .0;
                Ok(Json(routine_response(
                    operation,
                    format!(
                        "CF_ROUTINE_STATE feedback routine_id={} outcome={:?} suppressed={}",
                        response.routine_id, response.outcome, response.suppressed
                    ),
                    |out| out.feedback = Some(response),
                )))
            }
            RoutineOperation::Label => {
                let spec = params
                    .0
                    .label
                    .ok_or_else(|| missing_routine_spec("label"))?;
                let source_id = spec.routine_id.clone();
                let response = self
                    .routine_label_export(Parameters(spec))
                    .await
                    .map_err(|error| {
                        routine_delegate_error(
                            operation,
                            source_id,
                            error,
                            "provide an existing routine_id and inspect CF_ROUTINES/CF_ROUTINE_STATE label evidence",
                        )
                    })?
                    .0;
                Ok(Json(routine_response(
                    operation,
                    format!(
                        "CF_ROUTINES label_export routine_id={} samples={} prompt_bytes={}",
                        response.routine_id,
                        response.samples.len(),
                        response.prompt.len()
                    ),
                    |out| out.label = Some(response),
                )))
            }
            RoutineOperation::Automate => {
                let spec = params
                    .0
                    .automate
                    .ok_or_else(|| missing_routine_spec("automate"))?;
                let source_id = spec.routine_id.clone();
                let response = self
                    .routine_automate(Parameters(spec))
                    .await
                    .map_err(|error| {
                        routine_delegate_error(
                            operation,
                            source_id,
                            error,
                            "provide a mined routine_id and inspect profile_authoring/v1 plus routine_automation/v1 rows",
                        )
                    })?
                    .0;
                Ok(Json(routine_response(
                    operation,
                    format!(
                        "CF_KV routine automation routine_id={} candidate_id={} row_key={} wrote_row={}",
                        response.automation.routine_id,
                        response.automation.candidate_id,
                        response.row_key,
                        response.wrote_row
                    ),
                    |out| out.automate = Some(response),
                )))
            }
            RoutineOperation::ArmedTick => {
                let spec = params
                    .0
                    .armed_tick
                    .ok_or_else(|| missing_routine_spec("armed_tick"))?;
                let source_id = spec
                    .routine_id
                    .clone()
                    .unwrap_or_else(|| "routine_id:all".to_owned());
                let response = self
                    .armed_routine_tick(Parameters(spec), request_context)
                    .await
                    .map_err(|error| {
                        routine_delegate_error(
                            operation,
                            source_id,
                            error,
                            "fix armed routine filters and inspect armed_routine/v1 plus armed_routine_run/v1 rows",
                        )
                    })?
                    .0;
                Ok(Json(routine_response(
                    operation,
                    format!(
                        "CF_KV armed routine tick evaluated={} due={} executed={} dry_run={} source_of_truth={}",
                        response.evaluated,
                        response.due,
                        response.executed,
                        response.dry_run,
                        response.source_of_truth
                    ),
                    |out| out.armed_tick = Some(response),
                )))
            }
        }
    }

    #[tool(
        description = "Facade for intent and suggestion assist operations in the <=40 public MCP surface. operation is a strict enum; exactly one matching operation spec is accepted. Delegates to intent_current/intent_detect_tick/suggestion_tick/suggestion_list/suggestion_accept and returns CF_KV suggestion plus intent-tracker readback metadata."
    )]
    pub async fn assist(
        &self,
        params: Parameters<AssistParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<AssistResponse>, ErrorData> {
        validate_assist_facade_params(&params.0)?;
        let operation = params.0.operation;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = ASSIST_TOOL,
            operation = operation.as_str(),
            "tool.invocation kind=assist"
        );
        match operation {
            AssistOperation::Intent => {
                let spec = params
                    .0
                    .intent
                    .ok_or_else(|| missing_assist_spec("intent"))?;
                let source_id = assist_now_source_id(spec.now_ts_ns);
                let response = self
                    .intent_current(Parameters(spec))
                    .await
                    .map_err(|error| {
                        assist_delegate_error(
                            operation,
                            source_id,
                            error,
                            "fix intent filters and inspect CF_EPISODES/CF_ROUTINES/CF_ROUTINE_STATE",
                        )
                    })?
                    .0;
                Ok(Json(assist_response(
                    operation,
                    format!(
                        "intent snapshot now_ts_ns={} candidates={}",
                        response.now.ts_ns,
                        response.candidates.len()
                    ),
                    |out| out.intent = Some(response),
                )))
            }
            AssistOperation::Detect => {
                let spec = params
                    .0
                    .detect
                    .ok_or_else(|| missing_assist_spec("detect"))?;
                let source_id = assist_now_source_id(spec.now_ts_ns);
                let response = self
                    .intent_detect_tick(Parameters(spec))
                    .await
                    .map_err(|error| {
                        assist_delegate_error(
                            operation,
                            source_id,
                            error,
                            "fix detection filters and inspect intent tracker/event delivery plus routine stores",
                        )
                    })?
                    .0;
                Ok(Json(assist_response(
                    operation,
                    format!(
                        "intent detect candidates={} tracked={} events_published={} dropped={}",
                        response.candidates,
                        response.tracked,
                        response.events_published,
                        response.events_dropped
                    ),
                    |out| out.detect = Some(response),
                )))
            }
            AssistOperation::SuggestionTick => {
                let spec = params
                    .0
                    .suggestion_tick
                    .ok_or_else(|| missing_assist_spec("suggestion_tick"))?;
                let source_id = assist_now_source_id(spec.now_ts_ns);
                let response = self
                    .suggestion_tick(Parameters(spec))
                    .await
                    .map_err(|error| {
                        assist_delegate_error(
                            operation,
                            source_id,
                            error,
                            "fix suggestion tick bounds/config and inspect CF_KV suggestion/v1 rows",
                        )
                    })?
                    .0;
                Ok(Json(assist_response(
                    operation,
                    format!(
                        "CF_KV suggestion tick candidates={} created={} expired={} abandoned={} dry_run={}",
                        response.candidates_evaluated,
                        response.created.len(),
                        response.expired.len(),
                        response.abandoned.len(),
                        response.dry_run
                    ),
                    |out| out.suggestion_tick = Some(response),
                )))
            }
            AssistOperation::SuggestionList => {
                let spec = params
                    .0
                    .suggestion_list
                    .ok_or_else(|| missing_assist_spec("suggestion_list"))?;
                let source_id = spec
                    .routine_id
                    .clone()
                    .unwrap_or_else(|| "routine_id:all".to_owned());
                let response = self
                    .suggestion_list(Parameters(spec))
                    .await
                    .map_err(|error| {
                        assist_delegate_error(
                            operation,
                            source_id,
                            error,
                            "narrow suggestion list filters and inspect CF_KV suggestion/v1 rows",
                        )
                    })?
                    .0;
                Ok(Json(assist_response(
                    operation,
                    format!(
                        "CF_KV suggestion list total_rows={} returned={}",
                        response.total_rows, response.returned
                    ),
                    |out| out.suggestion_list = Some(response),
                )))
            }
            AssistOperation::SuggestionAccept => {
                let spec = params
                    .0
                    .suggestion_accept
                    .ok_or_else(|| missing_assist_spec("suggestion_accept"))?;
                let source_id = spec.suggestion_id.clone();
                let response = self
                    .suggestion_accept(Parameters(spec), request_context)
                    .await
                    .map_err(|error| {
                        assist_delegate_error(
                            operation,
                            source_id,
                            error,
                            "provide an existing live suggestion_id and inspect suggestion/plan_execution rows",
                        )
                    })?
                    .0;
                Ok(Json(assist_response(
                    operation,
                    format!(
                        "CF_KV suggestion accept suggestion_id={} execution_id={} dry_run={}",
                        response.suggestion.suggestion_id,
                        response.execution.execution_id,
                        response.execution.dry_run
                    ),
                    |out| out.suggestion_accept = Some(response),
                )))
            }
        }
    }

    #[tool(
        description = "Facade for delta-first reality baseline, delta, and audit operations in the <=40 public MCP surface. operation is a strict enum; exactly one matching operation spec is accepted. Delegates to reality_baseline/observe_delta/reality_audit and returns CF_KV reality row readback metadata."
    )]
    pub async fn reality(
        &self,
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
                let source_id =
                    reality_source_id(spec.profile_id.as_deref(), spec.epoch_id.as_deref());
                let response = self
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
                let response = self
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
                let source_id =
                    reality_source_id(spec.profile_id.as_deref(), spec.epoch_id.as_deref());
                let response = self
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

    #[tool(
        description = "Facade for verification inbox, polling, audit, binding, and source-list operations in the <=40 public MCP surface. operation is a strict enum; exactly one matching operation spec is accepted. Delegates to verification_* implementation paths and returns CF_KV audit/binding readback metadata."
    )]
    pub async fn verification(
        &self,
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
                let response = self
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
                let response = self
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
                let response = self
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
                let response = self
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
                let response = self
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
}

fn validate_routine_facade_params(params: &RoutineParams) -> Result<(), ErrorData> {
    validate_exact_operation_spec(
        ROUTINE_TOOL,
        params.operation.as_str(),
        &[
            ("mine", params.mine.is_some()),
            ("list", params.list.is_some()),
            ("inspect", params.inspect.is_some()),
            ("update", params.update.is_some()),
            ("feedback", params.feedback.is_some()),
            ("label", params.label.is_some()),
            ("automate", params.automate.is_some()),
            ("armed_tick", params.armed_tick.is_some()),
        ],
    )
}

fn validate_assist_facade_params(params: &AssistParams) -> Result<(), ErrorData> {
    validate_exact_operation_spec(
        ASSIST_TOOL,
        params.operation.as_str(),
        &[
            ("intent", params.intent.is_some()),
            ("detect", params.detect.is_some()),
            ("suggestion_tick", params.suggestion_tick.is_some()),
            ("suggestion_list", params.suggestion_list.is_some()),
            ("suggestion_accept", params.suggestion_accept.is_some()),
        ],
    )
}

fn validate_reality_facade_params(params: &RealityParams) -> Result<(), ErrorData> {
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

fn validate_verification_facade_params(params: &VerificationParams) -> Result<(), ErrorData> {
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

fn validate_exact_operation_spec(
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

fn missing_routine_spec(operation: &'static str) -> ErrorData {
    facade_params_error(
        ROUTINE_TOOL,
        operation,
        format!("routine operation={operation} requires a {operation} spec"),
        format!("pass {operation}={{...}} and no other operation spec"),
    )
}

fn missing_assist_spec(operation: &'static str) -> ErrorData {
    facade_params_error(
        ASSIST_TOOL,
        operation,
        format!("assist operation={operation} requires a {operation} spec"),
        format!("pass {operation}={{...}} and no other operation spec"),
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

fn missing_verification_spec(operation: &'static str) -> ErrorData {
    facade_params_error(
        VERIFICATION_TOOL,
        operation,
        format!("verification operation={operation} requires a {operation} spec"),
        format!("pass {operation}={{...}} and no other operation spec"),
    )
}

fn facade_params_error(
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

fn routine_delegate_error(
    operation: RoutineOperation,
    source_id: impl Into<String>,
    error: ErrorData,
    remediation: &'static str,
) -> ErrorData {
    delegate_error(
        ROUTINE_TOOL,
        operation.as_str(),
        ROUTINE_SOURCE_OF_TRUTH,
        source_id,
        error,
        remediation,
    )
}

fn assist_delegate_error(
    operation: AssistOperation,
    source_id: impl Into<String>,
    error: ErrorData,
    remediation: &'static str,
) -> ErrorData {
    delegate_error(
        ASSIST_TOOL,
        operation.as_str(),
        ASSIST_SOURCE_OF_TRUTH,
        source_id,
        error,
        remediation,
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

fn delegate_error(
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

fn routine_response(
    operation: RoutineOperation,
    readback_source_of_truth: String,
    populate: impl FnOnce(&mut RoutineResponse),
) -> RoutineResponse {
    let mut response = RoutineResponse {
        operation,
        source_of_truth: format!(
            "{ROUTINE_SOURCE_OF_TRUTH} + delegated routine operation={}",
            operation.as_str()
        ),
        readback_source_of_truth,
        mine: None,
        list: None,
        inspect: None,
        update: None,
        feedback: None,
        label: None,
        automate: None,
        armed_tick: None,
    };
    populate(&mut response);
    response
}

fn assist_response(
    operation: AssistOperation,
    readback_source_of_truth: String,
    populate: impl FnOnce(&mut AssistResponse),
) -> AssistResponse {
    let mut response = AssistResponse {
        operation,
        source_of_truth: format!(
            "{ASSIST_SOURCE_OF_TRUTH} + delegated assist operation={}",
            operation.as_str()
        ),
        readback_source_of_truth,
        intent: None,
        detect: None,
        suggestion_tick: None,
        suggestion_list: None,
        suggestion_accept: None,
    };
    populate(&mut response);
    response
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

fn routine_range_source_id(start_ts_ns: Option<u64>, end_ts_ns: Option<u64>) -> String {
    format!(
        "range:{}..{}",
        start_ts_ns
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unbounded".to_owned()),
        end_ts_ns
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unbounded".to_owned())
    )
}

fn routine_list_source_id(params: &RoutineListParams) -> String {
    format!(
        "routine_list:lifecycle={} app={} limit={}",
        params
            .lifecycle
            .as_ref()
            .map_or("default".to_owned(), |items| items.len().to_string()),
        params.app.as_deref().unwrap_or("<any>"),
        params
            .limit
            .map_or_else(|| "default".to_owned(), |value| value.to_string())
    )
}

fn assist_now_source_id(now_ts_ns: Option<u64>) -> String {
    format!(
        "now_ts_ns:{}",
        now_ts_ns
            .map(|value| value.to_string())
            .unwrap_or_else(|| "now".to_owned())
    )
}

fn reality_source_id(profile_id: Option<&str>, epoch_id: Option<&str>) -> String {
    format!(
        "profile:{} epoch:{}",
        profile_id.unwrap_or("<auto>"),
        epoch_id.unwrap_or("<auto>")
    )
}

fn verification_source_id(source: Option<&str>) -> String {
    format!("source:{}", source.unwrap_or("unspecified"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_routine_params(operation: RoutineOperation) -> RoutineParams {
        RoutineParams {
            operation,
            mine: None,
            list: None,
            inspect: None,
            update: None,
            feedback: None,
            label: None,
            automate: None,
            armed_tick: None,
        }
    }

    fn empty_assist_params(operation: AssistOperation) -> AssistParams {
        AssistParams {
            operation,
            intent: None,
            detect: None,
            suggestion_tick: None,
            suggestion_list: None,
            suggestion_accept: None,
        }
    }

    fn empty_reality_params(operation: RealityOperation) -> RealityParams {
        RealityParams {
            operation,
            baseline: None,
            delta: None,
            audit: None,
        }
    }

    fn empty_verification_params(operation: VerificationOperation) -> VerificationParams {
        VerificationParams {
            operation,
            inbox: None,
            poll: None,
            audit: None,
            bind: None,
            sources: None,
        }
    }

    #[test]
    fn routine_facade_params_require_exact_matching_spec() {
        let missing =
            validate_routine_facade_params(&empty_routine_params(RoutineOperation::Inspect))
                .expect_err("missing inspect spec should fail");
        assert!(
            missing
                .message
                .to_string()
                .contains("requires a matching inspect spec"),
            "{missing:?}"
        );

        let mut extra = empty_routine_params(RoutineOperation::List);
        extra.list = Some(RoutineListParams::default());
        extra.mine = Some(RoutineMineParams::default());
        let error =
            validate_routine_facade_params(&extra).expect_err("multiple routine specs should fail");
        assert!(
            error
                .message
                .to_string()
                .contains("received invalid operation specs"),
            "{error:?}"
        );
    }

    #[test]
    fn assist_facade_params_require_exact_matching_spec() {
        let missing =
            validate_assist_facade_params(&empty_assist_params(AssistOperation::SuggestionList))
                .expect_err("missing suggestion_list spec should fail");
        assert!(
            missing
                .message
                .to_string()
                .contains("requires a matching suggestion_list spec"),
            "{missing:?}"
        );

        let mut valid = empty_assist_params(AssistOperation::SuggestionList);
        valid.suggestion_list = Some(SuggestionListParams::default());
        validate_assist_facade_params(&valid).expect("matching suggestion_list spec should pass");
    }

    #[test]
    fn reality_facade_params_require_exact_matching_spec() {
        let missing =
            validate_reality_facade_params(&empty_reality_params(RealityOperation::Audit))
                .expect_err("missing audit spec should fail");
        assert!(
            missing
                .message
                .to_string()
                .contains("requires a matching audit spec"),
            "{missing:?}"
        );

        let mut extra = empty_reality_params(RealityOperation::Baseline);
        extra.baseline = Some(RealityBaselineParams {
            profile_id: None,
            epoch_id: None,
            force_new_epoch: false,
            include: Vec::new(),
            depth: 1,
            max_elements: 1,
        });
        extra.audit = Some(RealityAuditParams {
            profile_id: None,
            epoch_id: None,
            assumption_hash: None,
            include: Vec::new(),
            depth: 1,
            max_elements: 1,
        });
        let error =
            validate_reality_facade_params(&extra).expect_err("multiple reality specs should fail");
        assert!(
            error
                .message
                .to_string()
                .contains("received invalid operation specs"),
            "{error:?}"
        );
    }

    #[test]
    fn verification_facade_params_require_exact_matching_spec() {
        let missing = validate_verification_facade_params(&empty_verification_params(
            VerificationOperation::Sources,
        ))
        .expect_err("missing sources spec should fail");
        assert!(
            missing
                .message
                .to_string()
                .contains("requires a matching sources spec"),
            "{missing:?}"
        );

        let mut valid = empty_verification_params(VerificationOperation::Audit);
        valid.audit = Some(VerificationAuditParams { max: None });
        validate_verification_facade_params(&valid).expect("matching audit spec should pass");
    }
}
