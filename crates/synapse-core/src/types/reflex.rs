use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    Action, AimCurve, AimTarget, Backend, ComboStep, EventFilter, HumanizeParams, Key, MouseButton,
    PadButton, PadId, PathSpec, ReflexId, StrokeTiming, VelocityProfile,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflexRegistration {
    pub id: ReflexId,
    pub kind: ReflexKind,
    #[serde(default = "default_reflex_priority")]
    #[schemars(default = "default_reflex_priority")]
    pub priority: u32,
    #[serde(default)]
    pub lifetime: ReflexLifetime,
    #[serde(default)]
    pub exclusive: bool,
}

const fn default_reflex_priority() -> u32 {
    100
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReflexKind {
    AimTrack {
        target: AimTarget,
        axis: ReflexAimAxis,
        gain: f32,
        deadzone_px: f32,
        max_speed_px_per_ms: f32,
        curve_per_step: AimCurve,
        backend: Backend,
    },
    HoldMove {
        keys: Vec<Key>,
        backend: Backend,
        #[serde(default)]
        re_assert: bool,
    },
    HoldButton {
        button: ReflexButtonTarget,
        backend: Backend,
    },
    Combo {
        steps: Vec<ComboStep>,
        backend: Backend,
    },
    PathFollow {
        path: PathSpec,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        button: Option<MouseButton>,
        profile: VelocityProfile,
        timing: StrokeTiming,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        humanize: Option<HumanizeParams>,
        backend: Backend,
    },
    OnEvent {
        when: EventFilter,
        then: ReflexThen,
        debounce_ms: u32,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReflexAimAxis {
    Xy,
    XOnly,
    YOnly,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReflexButtonTarget {
    Mouse { button: MouseButton },
    Pad { pad: PadId, button: PadButton },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReflexThen {
    Action {
        action: Action,
    },
    Actions {
        actions: Vec<Action>,
    },
    Combo {
        steps: Vec<ComboStep>,
        backend: Backend,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReflexLifetime {
    #[default]
    UntilCancelled,
    OneShot,
    Duration {
        ms: u32,
    },
    UntilEvent {
        filter: EventFilter,
    },
    UntilDeadline {
        ms: u32,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReflexState {
    Active,
    ActionDenied,
    Paused,
    Cancelled,
    Expired,
    Disabled,
    Starved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflexStatus {
    pub id: ReflexId,
    pub kind_summary: String,
    pub state: ReflexState,
    pub registered_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_at: Option<DateTime<Utc>>,
    pub fire_count: u64,
    pub priority: u32,
    pub lifetime: ReflexLifetime,
    pub exclusive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<String>,
}
