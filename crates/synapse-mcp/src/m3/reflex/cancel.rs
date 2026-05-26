use std::sync::{Arc, Mutex};

use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_core::error_codes;
use synapse_reflex::{ReflexCancelOutcome, ReflexRuntime};

use crate::m1::mcp_error;

use super::super::permissions::{Permission, RequiredPermissions, required};

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflexCancelParams {
    pub reflex_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReflexCancelReason {
    Ok,
    NotFound,
    AlreadyExpired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflexCancelResponse {
    pub cancelled: bool,
    pub reason: ReflexCancelReason,
}

#[must_use]
pub fn required_permissions_cancel(_params: &ReflexCancelParams) -> RequiredPermissions {
    required([Permission::ReadReflex])
}

pub fn cancel_reflex(
    runtime: &Arc<Mutex<ReflexRuntime>>,
    params: &ReflexCancelParams,
) -> Result<ReflexCancelResponse, ErrorData> {
    let reflex_id = params.reflex_id.trim();
    if reflex_id.is_empty() {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "reflex_cancel reflex_id must not be empty",
        ));
    }
    let mut runtime = runtime.lock().map_err(|_err| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "reflex runtime lock poisoned",
        )
    })?;
    let outcome = runtime
        .cancel(reflex_id)
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    drop(runtime);
    Ok(match outcome {
        ReflexCancelOutcome::Cancelled { .. } => ReflexCancelResponse {
            cancelled: true,
            reason: ReflexCancelReason::Ok,
        },
        ReflexCancelOutcome::NotFound => ReflexCancelResponse {
            cancelled: false,
            reason: ReflexCancelReason::NotFound,
        },
        ReflexCancelOutcome::AlreadyExpired { .. } => ReflexCancelResponse {
            cancelled: false,
            reason: ReflexCancelReason::AlreadyExpired,
        },
    })
}
