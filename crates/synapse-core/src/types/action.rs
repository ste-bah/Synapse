use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Backend, ElementId, PathSpec, Point};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    KeyPress {
        key: Key,
        hold_ms: u32,
        backend: Backend,
    },
    KeyDown {
        key: Key,
        backend: Backend,
    },
    KeyUp {
        key: Key,
        backend: Backend,
    },
    KeyChord {
        keys: Vec<Key>,
        hold_ms: u32,
        backend: Backend,
    },
    TypeText {
        text: String,
        dynamics: KeystrokeDynamics,
        backend: Backend,
    },
    MouseMove {
        to: MouseTarget,
        curve: AimCurve,
        duration_ms: u32,
        backend: Backend,
    },
    MouseMoveRelative {
        dx: f32,
        dy: f32,
        backend: Backend,
    },
    MouseButton {
        button: MouseButton,
        action: ButtonAction,
        hold_ms: u32,
        backend: Backend,
    },
    MouseDrag {
        from: Point,
        to: Point,
        button: MouseButton,
        curve: AimCurve,
        duration_ms: u32,
        backend: Backend,
    },
    MouseStroke {
        path: PathSpec,
        button: Option<MouseButton>,
        profile: VelocityProfile,
        timing: StrokeTiming,
        #[serde(default = "default_stroke_motion_model")]
        motion_model: StrokeMotionModel,
        humanize: Option<HumanizeParams>,
        backend: Backend,
    },
    MouseScroll {
        dy: i32,
        dx: i32,
        at: Option<Point>,
        backend: Backend,
    },
    PadButton {
        pad: PadId,
        button: PadButton,
        action: ButtonAction,
        hold_ms: u32,
    },
    PadStick {
        pad: PadId,
        stick: Stick,
        x: f32,
        y: f32,
    },
    PadTrigger {
        pad: PadId,
        trigger: Trigger,
        value: f32,
    },
    PadReport {
        pad: PadId,
        report: GamepadReport,
    },
    AimAt {
        target: AimTarget,
        style: AimStyle,
        deadline_ms: u32,
        backend: Backend,
    },
    Combo {
        steps: Vec<ComboStep>,
        backend: Backend,
    },
    ReleaseAll,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AimCurve {
    Instant,
    Linear,
    EaseInOut,
    Bezier { p1: (f32, f32), p2: (f32, f32) },
    Natural { params: AimNaturalParams },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VelocityProfile {
    Constant,
    Linear,
    EaseInOut,
    MinimumJerk,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StrokeTiming {
    DurationMs { duration_ms: u32 },
    SpeedPxPerSec { px_per_sec: f64 },
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StrokeMotionModel {
    Path,
    WindMouse {
        gravity: f64,
        wind: f64,
        max_step: f64,
        damped_distance: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed: Option<u64>,
    },
}

const fn default_stroke_motion_model() -> StrokeMotionModel {
    StrokeMotionModel::Path
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HumanizeParams {
    pub tremor_base_stddev_px: f32,
    pub tremor_velocity_scale: f32,
    pub overshoot_prob: f32,
    pub overshoot_factor_range: (f32, f32),
    pub micro_pause_prob: f32,
    pub micro_pause_ms_range: (u32, u32),
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AimNaturalParams {
    pub control_point_jitter: f32,
    pub tremor_stddev_px: f32,
    pub overshoot_prob: f32,
    pub overshoot_factor_range: (f32, f32),
    pub micro_correct_steps: u8,
    pub timing_stddev_ms: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl AimNaturalParams {
    pub const FAST: Self = Self {
        control_point_jitter: 0.08,
        tremor_stddev_px: 0.2,
        overshoot_prob: 0.25,
        overshoot_factor_range: (1.02, 1.06),
        micro_correct_steps: 1,
        timing_stddev_ms: 1.5,
        seed: None,
    };
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AimStyle {
    Snap,
    Flick,
    Natural,
    Track,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeystrokeDynamics {
    Burst,
    Linear { ms_per_char: u32 },
    Natural { params: KeystrokeNaturalParams },
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeystrokeNaturalParams {
    pub mean_iki_ms: f32,
    pub stddev_ms: f32,
    pub bigram_bias: bool,
}

impl KeystrokeNaturalParams {
    pub const FAST: Self = Self {
        mean_iki_ms: 32.0,
        stddev_ms: 10.0,
        bigram_bias: true,
    };
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub code: KeyCode,
    pub use_scancode: bool,
}

#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeyCode {
    Named { value: String },
    Symbol { value: char },
    HidCode { value: u8 },
}

#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ButtonAction {
    Press,
    Down,
    Up,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MouseTarget {
    Screen { point: Point },
    Element { element_id: ElementId },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AimTarget {
    Screen { point: Point },
    Element { element_id: ElementId },
    Track { track_id: u64 },
}

pub type PadId = u8;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GamepadController {
    X360,
    Ds4,
}

const fn default_gamepad_controller() -> GamepadController {
    GamepadController::X360
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PadButton {
    A,
    B,
    X,
    Y,
    Lb,
    Rb,
    Ls,
    Rs,
    Back,
    Start,
    Up,
    Down,
    Left,
    Right,
    Guide,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Stick {
    Left,
    Right,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GamepadReport {
    #[serde(default = "default_gamepad_controller")]
    #[schemars(default = "default_gamepad_controller")]
    pub controller: GamepadController,
    #[serde(default)]
    pub buttons: Vec<PadButton>,
    #[schemars(schema_with = "normalized_axis_pair_schema")]
    pub thumb_l: (f32, f32),
    #[schemars(schema_with = "normalized_axis_pair_schema")]
    pub thumb_r: (f32, f32),
    #[schemars(range(min = 0.0, max = 1.0))]
    pub lt: f32,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub rt: f32,
}

impl GamepadReport {
    #[must_use]
    pub const fn neutral(controller: GamepadController) -> Self {
        Self {
            controller,
            buttons: Vec::new(),
            thumb_l: (0.0, 0.0),
            thumb_r: (0.0, 0.0),
            lt: 0.0,
            rt: 0.0,
        }
    }
}

impl Default for GamepadReport {
    fn default() -> Self {
        Self::neutral(GamepadController::X360)
    }
}

fn normalized_axis_pair_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "array",
        "prefixItems": [
            {"type": "number", "minimum": -1.0, "maximum": 1.0},
            {"type": "number", "minimum": -1.0, "maximum": 1.0}
        ],
        "minItems": 2,
        "maxItems": 2
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComboStep {
    pub at_ms: u32,
    pub input: ComboInput,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComboInput {
    KeyDown {
        key: Key,
    },
    KeyUp {
        key: Key,
    },
    KeyPress {
        key: Key,
        hold_ms: u16,
    },
    MouseButton {
        button: MouseButton,
        action: ButtonAction,
    },
    MouseMoveRel {
        dx: f32,
        dy: f32,
    },
    PadButton {
        pad: PadId,
        button: PadButton,
        action: ButtonAction,
    },
    PadStick {
        pad: PadId,
        stick: Stick,
        x: f32,
        y: f32,
    },
}
