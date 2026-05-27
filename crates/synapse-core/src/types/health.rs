use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ProfileId;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Health {
    pub ok: bool,
    pub version: String,
    pub build: String,
    pub uptime_s: u64,
    pub subsystems: BTreeMap<String, SubsystemHealth>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubsystemHealth {
    pub status: String,
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_profile_id: Option<ProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cf_sizes: Option<BTreeMap<String, u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tick_jitter_us: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recursion_clamps_total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reload_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ring_buffer_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stt_model_loaded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_addr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_sessions: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sse_subscribers: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_resolution: Option<BTreeMap<String, String>>,
}
