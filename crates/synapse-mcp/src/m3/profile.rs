use super::M3ToolStub;
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use synapse_core::{ProfileId, error_codes};
use synapse_profiles::{ProfileError, ProfileRuntime};

use crate::m1::mcp_error;

const fn default_include_inactive() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileListParams {
    #[serde(default = "default_include_inactive")]
    #[schemars(default = "default_include_inactive")]
    pub include_inactive: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileActivateParams {
    pub profile_id: ProfileId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileListResponse {
    pub profiles: Vec<ProfileStatus>,
    pub active_profile_id: Option<ProfileId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileStatus {
    pub id: ProfileId,
    pub label: String,
    pub matches: Vec<ProfileMatchStatus>,
    pub active: bool,
    pub schema_version: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileMatchStatus {
    pub exe: Option<String>,
    pub title_regex: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileActivateResponse {
    pub profile_id: ProfileId,
    pub active_profile_id: ProfileId,
    pub previous_active_profile_id: Option<ProfileId>,
    pub changed: bool,
}

#[must_use]
pub const fn profile_list() -> M3ToolStub {
    M3ToolStub::new("profile_list")
}

#[must_use]
pub const fn profile_activate() -> M3ToolStub {
    M3ToolStub::new("profile_activate")
}

pub fn list_profiles(
    runtime: &ProfileRuntime,
    params: &ProfileListParams,
) -> Result<ProfileListResponse, ErrorData> {
    let profiles = runtime
        .list(params.include_inactive)
        .map(|profiles| profiles.into_iter().map(ProfileStatus::from).collect())
        .map_err(|error| profile_error(&error))?;
    let active_profile_id = runtime
        .active_profile_id()
        .map_err(|error| profile_error(&error))?;
    Ok(ProfileListResponse {
        profiles,
        active_profile_id,
    })
}

pub fn activate_profile(
    runtime: &ProfileRuntime,
    params: &ProfileActivateParams,
) -> Result<ProfileActivateResponse, ErrorData> {
    if runtime
        .profile(&params.profile_id)
        .map_err(|error| profile_error(&error))?
        .is_none()
    {
        return Err(mcp_error(
            error_codes::PROFILE_NOT_FOUND,
            format!("profile {} was not found", params.profile_id),
        ));
    }

    let previous_active_profile_id = runtime
        .active_profile_id()
        .map_err(|error| profile_error(&error))?;
    if previous_active_profile_id.as_deref() == Some(params.profile_id.as_str()) {
        return Ok(ProfileActivateResponse {
            profile_id: params.profile_id.clone(),
            active_profile_id: params.profile_id.clone(),
            previous_active_profile_id,
            changed: false,
        });
    }

    runtime
        .activate(&params.profile_id)
        .map_err(|error| profile_error(&error))?;
    let active_profile_id = runtime
        .active_profile_id()
        .map_err(|error| profile_error(&error))?
        .ok_or_else(|| {
            mcp_error(
                error_codes::PROFILE_NOT_FOUND,
                "profile activation did not stick",
            )
        })?;

    Ok(ProfileActivateResponse {
        profile_id: params.profile_id.clone(),
        active_profile_id,
        previous_active_profile_id,
        changed: true,
    })
}

impl From<synapse_profiles::ProfileStatus> for ProfileStatus {
    fn from(value: synapse_profiles::ProfileStatus) -> Self {
        Self {
            id: value.id,
            label: value.label,
            matches: value
                .matches
                .into_iter()
                .map(ProfileMatchStatus::from)
                .collect(),
            active: value.active,
            schema_version: value.schema_version,
        }
    }
}

impl From<synapse_core::ProfileMatch> for ProfileMatchStatus {
    fn from(value: synapse_core::ProfileMatch) -> Self {
        Self {
            exe: value.exe,
            title_regex: value.title_regex,
        }
    }
}

fn profile_error(error: &ProfileError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}
