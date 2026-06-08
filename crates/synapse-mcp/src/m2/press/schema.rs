use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_core::Backend;

use crate::m2::postcondition::{
    ActPostcondition, default_verify_timeout_ms, postcondition_not_requested,
};

const DEFAULT_HOLD_MS: u32 = 33;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActPressParams {
    pub keys: Vec<String>,
    #[serde(default = "default_hold_ms")]
    #[schemars(default = "default_hold_ms", range(min = 1, max = 30000))]
    pub hold_ms: u32,
    #[serde(default = "default_press_backend")]
    #[schemars(default = "default_press_backend")]
    pub backend: PressBackend,
    #[serde(default)]
    #[schemars(default)]
    pub verify_delta: bool,
    #[serde(default)]
    #[schemars(
        default,
        description = "When verify_delta=true, accept a foreground-window identity change as the observed postcondition. Defaults false so unexpected focus loss still fails closed."
    )]
    pub allow_foreground_change: bool,
    #[serde(default)]
    #[schemars(
        default,
        description = "Optional regex that the after-read foreground process name must match when allow_foreground_change=true. Invalid regexes fail before key input is sent."
    )]
    pub expected_foreground_process_regex: Option<String>,
    #[serde(default)]
    #[schemars(
        default,
        description = "Optional regex that the after-read foreground window title must match when allow_foreground_change=true. Invalid regexes fail before key input is sent."
    )]
    pub expected_foreground_title_regex: Option<String>,
    #[serde(default = "default_verify_timeout_ms")]
    #[schemars(default = "default_verify_timeout_ms", range(min = 50, max = 5000))]
    pub verify_timeout_ms: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActKeymapParams {
    pub alias: String,
    #[serde(default = "default_hold_ms")]
    #[schemars(default = "default_hold_ms", range(min = 1, max = 30000))]
    pub hold_ms: u32,
    #[serde(default = "default_press_backend")]
    #[schemars(default = "default_press_backend")]
    pub backend: PressBackend,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PressBackend {
    Software,
    Hardware,
    Auto,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActPressResponse {
    pub ok: bool,
    pub keys_pressed: u32,
    pub elapsed_ms: u32,
    pub backend_used: String,
    pub backend_tier_used: String,
    pub required_foreground: bool,
    pub postcondition: ActPostcondition,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActKeymapResponse {
    pub ok: bool,
    pub alias: String,
    pub resolved_binding: String,
    pub resolved_keys: Vec<String>,
    pub hold_ms: u32,
    pub keys_pressed: u32,
    pub elapsed_ms: u32,
    pub backend_used: String,
    pub backend_tier_used: String,
    pub required_foreground: bool,
}

impl PressBackend {
    pub(in crate::m2::press) const fn to_backend(self) -> Backend {
        match self {
            Self::Software => Backend::Software,
            Self::Hardware => Backend::Hardware,
            Self::Auto => Backend::Auto,
        }
    }
}

pub(in crate::m2::press) const fn default_hold_ms() -> u32 {
    DEFAULT_HOLD_MS
}

pub(in crate::m2::press) const fn default_press_backend() -> PressBackend {
    PressBackend::Auto
}

pub(in crate::m2::press) fn press_postcondition_not_requested() -> ActPostcondition {
    postcondition_not_requested("act_press", "foreground_focused_ui_or_pixels")
}
