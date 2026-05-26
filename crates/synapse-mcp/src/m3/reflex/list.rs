use std::sync::{Arc, Mutex};

use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_core::{ReflexStatus, error_codes};
use synapse_reflex::ReflexRuntime;

use crate::m1::mcp_error;

use super::super::permissions::{Permission, RequiredPermissions, required};

const fn default_include_expired() -> bool {
    false
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflexListParams {
    #[serde(default = "default_include_expired")]
    #[schemars(default = "default_include_expired")]
    pub include_expired: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflexListResponse {
    pub reflexes: Vec<ReflexStatus>,
}

#[must_use]
pub fn required_permissions_list(_params: &ReflexListParams) -> RequiredPermissions {
    required([Permission::ReadReflex])
}

pub fn list_reflexes(
    runtime: &Arc<Mutex<ReflexRuntime>>,
    params: &ReflexListParams,
) -> Result<ReflexListResponse, ErrorData> {
    let runtime = runtime.lock().map_err(|_err| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "reflex runtime lock poisoned",
        )
    })?;
    let reflexes = runtime
        .list(params.include_expired)
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    drop(runtime);
    Ok(ReflexListResponse { reflexes })
}
