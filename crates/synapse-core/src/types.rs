use std::{borrow::Cow, collections::BTreeMap, fmt, str::FromStr};

use chrono::{DateTime, Utc};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Software,
    Vigem,
    Hardware,
    Auto,
}

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

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PerceptionMode {
    A11yOnly,
    PixelOnly,
    Hybrid,
    Auto,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    #[must_use]
    pub fn distance_to(self, other: Self) -> f64 {
        let dx = f64::from(self.x) - f64::from(other.x);
        let dy = f64::from(self.y) - f64::from(other.y);
        dx.hypot(dy)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    /// Returns true when a point is inside this rectangle.
    ///
    /// The right and bottom edges are exclusive. Non-positive width or height
    /// rectangles are empty.
    #[must_use]
    pub const fn contains(self, point: Point) -> bool {
        if self.w <= 0 || self.h <= 0 {
            return false;
        }

        let right = self.x.saturating_add(self.w);
        let bottom = self.y.saturating_add(self.h);
        point.x >= self.x && point.x < right && point.y >= self.y && point.y < bottom
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Size {
    pub w: u32,
    pub h: u32,
}

pub type SessionId = String;
const ELEMENT_ID_SCHEMA_PATTERN: &str = r"^-?0x[0-9a-fA-F]+:[0-9a-fA-F]+$";

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ElementId(String);

impl ElementId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parses and validates a public UIA element identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when the identifier is not shaped as
    /// `<hwnd_hex>:<runtime_id_hex>`.
    pub fn parse(value: &str) -> Result<Self, ElementIdParseError> {
        value.parse()
    }

    /// Splits a validated element identifier into its HWND and UIA runtime id.
    ///
    /// # Errors
    ///
    /// Returns an error when this value was constructed from a non-canonical
    /// string that cannot be parsed as an element identifier.
    pub fn parts(&self) -> Result<ElementIdParts, ElementIdParseError> {
        parse_element_id_parts(&self.0)
    }
}

impl fmt::Display for ElementId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<ElementId> for String {
    fn from(value: ElementId) -> Self {
        value.0
    }
}

impl From<&ElementId> for String {
    fn from(value: &ElementId) -> Self {
        value.0.clone()
    }
}

impl TryFrom<String> for ElementId {
    type Error = ElementIdParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        parse_element_id_parts(&value)?;
        Ok(Self(value))
    }
}

impl FromStr for ElementId {
    type Err = ElementIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_element_id_parts(value)?;
        Ok(Self(value.to_owned()))
    }
}

impl JsonSchema for ElementId {
    fn schema_name() -> Cow<'static, str> {
        "ElementId".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": ELEMENT_ID_SCHEMA_PATTERN,
        })
    }
}

impl PartialEq<&str> for ElementId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<ElementId> for &str {
    fn eq(&self, other: &ElementId) -> bool {
        *self == other.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ElementIdParts {
    pub hwnd: i64,
    pub runtime_id_hex: String,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ElementIdParseError {
    #[error("element id must be shaped as '<hwnd_hex>:<runtime_id_hex>'")]
    MissingSeparator,
    #[error("element id hwnd must be hex with a 0x prefix")]
    InvalidHwnd,
    #[error("element id runtime id must be non-empty hex")]
    InvalidRuntimeId,
}

pub type EntityId = String;
pub type ReflexId = String;
pub type SubscriptionId = String;
pub type ProfileId = String;

#[must_use]
pub fn new_session_id() -> SessionId {
    uuid::Uuid::now_v7().to_string()
}

#[must_use]
pub fn new_reflex_id() -> ReflexId {
    uuid::Uuid::now_v7().to_string()
}

#[must_use]
pub fn new_subscription_id() -> SubscriptionId {
    uuid::Uuid::now_v7().to_string()
}

#[must_use]
pub fn element_id(hwnd: i64, runtime_id_hex: &str) -> ElementId {
    let hwnd_hex = if hwnd.is_negative() {
        format!("-0x{:x}", hwnd.unsigned_abs())
    } else {
        format!("0x{hwnd:x}")
    };
    ElementId(format!("{hwnd_hex}:{runtime_id_hex}"))
}

#[must_use]
pub fn entity_id(track: u64) -> EntityId {
    format!("track:{track}")
}

fn parse_element_id_parts(value: &str) -> Result<ElementIdParts, ElementIdParseError> {
    let (hwnd_raw, runtime_id_hex) = value
        .split_once(':')
        .ok_or(ElementIdParseError::MissingSeparator)?;
    let hwnd = parse_hwnd_hex(hwnd_raw)?;

    if runtime_id_hex.is_empty() || !runtime_id_hex.chars().all(|item| item.is_ascii_hexdigit()) {
        return Err(ElementIdParseError::InvalidRuntimeId);
    }

    Ok(ElementIdParts {
        hwnd,
        runtime_id_hex: runtime_id_hex.to_owned(),
    })
}

fn parse_hwnd_hex(value: &str) -> Result<i64, ElementIdParseError> {
    if let Some(hex) = value.strip_prefix("0x") {
        return i64::from_str_radix(hex, 16).map_err(|_err| ElementIdParseError::InvalidHwnd);
    }

    if let Some(hex) = value.strip_prefix("-0x") {
        let hwnd = i64::from_str_radix(hex, 16).map_err(|_err| ElementIdParseError::InvalidHwnd)?;
        return Ok(-hwnd);
    }

    Err(ElementIdParseError::InvalidHwnd)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub seq: u64,
    pub at: DateTime<Utc>,
    pub mode: PerceptionMode,
    pub foreground: ForegroundContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<FocusedElement>,
    #[serde(default)]
    pub elements: Vec<AccessibleNode>,
    #[serde(default)]
    pub entities: Vec<DetectedEntity>,
    #[serde(default)]
    pub hud: HudReadings,
    #[serde(default)]
    pub audio: AudioContext,
    #[serde(default)]
    pub recent_events: Vec<EventSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard_summary: Option<ClipboardSummary>,
    #[serde(default)]
    pub fs_recent: Vec<FsEvent>,
    pub diagnostics: ObservationDiagnostics,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForegroundContext {
    pub hwnd: i64,
    pub pid: u32,
    pub process_name: String,
    pub process_path: String,
    pub window_title: String,
    pub window_bounds: Rect,
    pub monitor_index: u32,
    pub dpi_scale: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<ProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_appid: Option<u32>,
    pub is_fullscreen: bool,
    pub is_dwm_composed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FocusedElement {
    pub element_id: ElementId,
    pub name: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_id: Option<String>,
    pub bbox: Rect,
    pub enabled: bool,
    #[serde(default)]
    pub patterns: Vec<UiaPattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_text: Option<String>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UiaPattern {
    Invoke,
    Toggle,
    Value,
    Selection,
    ExpandCollapse,
    Scroll,
    Text,
    Window,
    Transform,
    RangeValue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccessibleNode {
    pub element_id: ElementId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ElementId>,
    pub name: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_id: Option<String>,
    pub bbox: Rect,
    pub enabled: bool,
    pub focused: bool,
    #[serde(default)]
    pub patterns: Vec<UiaPattern>,
    pub children_count: u32,
    pub depth: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccessibleSubtree {
    pub root: ElementId,
    #[serde(default)]
    pub nodes: Vec<AccessibleNode>,
    pub max_depth: u32,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccessibleQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_substring: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_id: Option<String>,
    #[serde(default)]
    pub scope: AccessibleQueryScope,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccessibleQueryScope {
    #[default]
    FocusedSubtree,
    ForegroundWindow,
    Global,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetectedEntity {
    pub entity_id: EntityId,
    pub track_id: u64,
    pub class_label: String,
    pub bbox: Rect,
    pub confidence: f32,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity_px_per_s: Option<(f32, f32)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Detection {
    pub class_label: String,
    pub bbox: Rect,
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetectionBatch {
    pub model_id: String,
    pub frame_seq: u64,
    pub inferred_at: DateTime<Utc>,
    #[serde(default)]
    pub items: Vec<Detection>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HudReadings {
    #[serde(default)]
    pub by_name: BTreeMap<String, HudReading>,
}

pub type HudField = HudReading;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HudReading {
    pub raw_text: String,
    pub parsed: HudValue,
    pub confidence: f32,
    pub stale_ms: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum HudValue {
    Number(f64),
    Text(String),
    Enum(String),
    #[default]
    Null,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioContext {
    pub rms_db: f32,
    pub vad_speech_recent: bool,
    #[serde(default)]
    pub recent_events: Vec<AudioEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction_estimate: Option<DirectionEstimate>,
}

pub type AudioCue = AudioEvent;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioEvent {
    pub at: DateTime<Utc>,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub azimuth_deg: Option<f32>,
    pub confidence: f32,
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DirectionEstimate {
    pub azimuth_deg: f32,
    pub confidence: f32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipboardSummary {
    #[serde(default)]
    pub formats: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_len: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_excerpt: Option<String>,
    pub redacted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FsEvent {
    pub at: DateTime<Utc>,
    pub path: String,
    pub kind: FsEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FsEventKind {
    Created,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct ObservationDiagnostics {
    pub assembled_in_ms: f32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sensor_latency_ms: BTreeMap<String, f32>,
    pub a11y_enabled: bool,
    pub pixel_enabled: bool,
    pub audio_enabled: bool,
    pub a11y_status: SensorStatus,
    pub capture_status: SensorStatus,
    pub detection_status: SensorStatus,
    pub audio_status: SensorStatus,
    pub elements_truncated: bool,
    pub entities_truncated: bool,
    pub size_bytes: u32,
    pub size_estimate_tokens: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SensorStatus {
    Healthy,
    DegradedLatency {
        last_p99_ms: f32,
    },
    DegradedSensorFailed {
        reason_code: String,
    },
    Disabled,
    #[default]
    Unavailable,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OcrBackend {
    Winrt,
    Crnn,
    #[default]
    Auto,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OcrResult {
    pub text: String,
    #[serde(default)]
    pub words: Vec<OcrWord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub backend: OcrBackend,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OcrWord {
    pub text: String,
    pub bbox: Rect,
    pub confidence: f32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub seq: u64,
    pub at: DateTime<Utc>,
    pub source: EventSource,
    pub kind: String,
    pub data: serde_json::Value,
    #[serde(default)]
    pub correlations: Vec<EventRef>,
}

impl Event {
    #[must_use]
    pub fn summary(&self) -> EventSummary {
        EventSummary {
            seq: self.seq,
            at: self.at,
            source: self.source,
            kind: self.kind.clone(),
            data_excerpt: self.data.clone(),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    A11yUia,
    A11yWinEvent,
    A11yCdp,
    Perception,
    PerceptionDetection,
    PerceptionHud,
    PerceptionAudio,
    Filesystem,
    Process,
    Clipboard,
    ActionEmitter,
    Reflex,
    System,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventRef {
    pub seq: u64,
    pub relation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventSummary {
    pub seq: u64,
    pub at: DateTime<Utc>,
    pub source: EventSource,
    pub kind: String,
    pub data_excerpt: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum EventFilter {
    All,
    None,
    Kind {
        kind: String,
    },
    Source {
        source: EventSource,
    },
    And {
        args: Vec<Self>,
    },
    Or {
        args: Vec<Self>,
    },
    Not {
        arg: Box<Self>,
    },
    Data {
        path: String,
        predicate: DataPredicate,
    },
}

impl EventFilter {
    #[must_use]
    pub fn matches(&self, event: &Event) -> bool {
        crate::filter::matches_event_filter(self, event)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DataPredicate {
    Eq { value: serde_json::Value },
    Ne { value: serde_json::Value },
    Lt { value: serde_json::Value },
    Le { value: serde_json::Value },
    Gt { value: serde_json::Value },
    Ge { value: serde_json::Value },
    Regex { pattern: String },
    InSet { values: Vec<serde_json::Value> },
    Exists,
}

impl DataPredicate {
    #[must_use]
    pub fn matches(&self, value: Option<&serde_json::Value>) -> bool {
        crate::filter::matches_data_predicate(self, value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Health {
    pub ok: bool,
    pub version: String,
    pub build: String,
    pub uptime_s: u64,
    pub subsystems: BTreeMap<String, SubsystemHealth>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubsystemHealth {
    pub status: String,
    pub detail: Option<String>,
}
