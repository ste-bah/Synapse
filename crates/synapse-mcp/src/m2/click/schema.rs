use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_core::{AimCurve, AimNaturalParams, Backend, ElementId, MouseButton};

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActClickParams {
    pub target: ActClickTarget,
    #[serde(default = "default_click_button")]
    #[schemars(default = "default_click_button")]
    pub button: MouseButton,
    #[serde(default = "default_click_count")]
    #[schemars(default = "default_click_count", range(min = 1, max = 3))]
    pub clicks: u8,
    #[serde(default)]
    pub modifiers: Vec<ClickModifier>,
    #[serde(default = "default_click_curve")]
    #[schemars(default = "default_click_curve")]
    pub curve: ClickCurve,
    #[serde(default = "default_click_duration_ms")]
    #[schemars(default = "default_click_duration_ms")]
    pub duration_ms: u32,
    #[serde(default = "default_click_backend")]
    #[schemars(default = "default_click_backend")]
    pub backend: Backend,
    #[serde(default = "default_use_invoke_pattern")]
    #[schemars(default = "default_use_invoke_pattern")]
    pub use_invoke_pattern: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(untagged)]
pub enum ActClickTarget {
    Element(ActClickElementTarget),
    Point(ActClickPointTarget),
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActClickElementTarget {
    pub element_id: ElementId,
}

#[derive(Copy, Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActClickPointTarget {
    pub x: i32,
    pub y: i32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ClickModifier {
    Ctrl,
    Shift,
    Alt,
    Super,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClickCurve {
    Natural,
    Instant,
    Linear,
    EaseInOut,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActClickResponse {
    pub ok: bool,
    pub used_invoke_pattern: bool,
    pub backend_used: String,
    pub double_click_window_ms: u32,
    pub inter_click_delay_ms: u32,
    pub elapsed_ms: u32,
}

impl ClickCurve {
    pub(in crate::m2::click) const fn to_aim_curve(self) -> AimCurve {
        match self {
            Self::Natural => AimCurve::Natural {
                params: AimNaturalParams::FAST,
            },
            Self::Instant => AimCurve::Instant,
            Self::Linear => AimCurve::Linear,
            Self::EaseInOut => AimCurve::EaseInOut,
        }
    }
}

pub(in crate::m2::click) const fn default_click_button() -> MouseButton {
    MouseButton::Left
}

pub(in crate::m2::click) const fn default_click_count() -> u8 {
    1
}

pub(in crate::m2::click) const fn default_click_curve() -> ClickCurve {
    ClickCurve::Natural
}

pub(in crate::m2::click) const fn default_click_duration_ms() -> u32 {
    50
}

pub(in crate::m2::click) const fn default_click_backend() -> Backend {
    Backend::Auto
}

pub(in crate::m2::click) const fn default_use_invoke_pattern() -> bool {
    true
}
