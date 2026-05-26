use std::sync::{Arc, Mutex};

use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_core::{StoredReflexAudit, error_codes};
use synapse_reflex::ReflexRuntime;

use crate::m1::mcp_error;

use super::super::permissions::{Permission, RequiredPermissions, required};

const MAX_REFLEX_HISTORY_LIMIT: u32 = 1000;

const fn default_history_limit() -> u32 {
    50
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflexHistoryParams {
    #[serde(default)]
    pub reflex_id: Option<String>,
    #[serde(default = "default_history_limit")]
    #[schemars(default = "default_history_limit", range(min = 0, max = 1000))]
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflexHistoryResponse {
    pub events: Vec<StoredReflexAudit>,
}

#[must_use]
pub fn required_permissions_history(_params: &ReflexHistoryParams) -> RequiredPermissions {
    required([Permission::ReadReflex])
}

pub fn history_reflexes(
    runtime: &Arc<Mutex<ReflexRuntime>>,
    params: &ReflexHistoryParams,
) -> Result<ReflexHistoryResponse, ErrorData> {
    if params.limit > MAX_REFLEX_HISTORY_LIMIT {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("reflex_history limit must be <= {MAX_REFLEX_HISTORY_LIMIT}"),
        ));
    }
    let reflex_id = params.reflex_id.as_deref().map(str::trim);
    if reflex_id.is_some_and(str::is_empty) {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "reflex_history reflex_id must not be empty",
        ));
    }

    let runtime = runtime.lock().map_err(|_err| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "reflex runtime lock poisoned",
        )
    })?;
    let events = runtime
        .history(reflex_id, params.limit as usize)
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    drop(runtime);
    Ok(ReflexHistoryResponse { events })
}
