use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ElementId, EntityId, EventSummary, PerceptionMode, ProfileId, Rect};

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
    /// On-screen value source-of-truth (UIA `ValuePattern` / `RangeValuePattern`),
    /// e.g. the text in an edit field or a numeric display. `None` when the
    /// element exposes no value. Distinct from `name` (often a static label).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub errors: BTreeMap<String, HudFieldError>,
}

pub type HudField = HudReading;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HudFieldError {
    pub code: String,
    pub detail: String,
}

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
    pub full_text: String,
    pub words: Vec<OcrWord>,
    pub confidence: f32,
    pub region: Rect,
    pub lang: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OcrWord {
    pub text: String,
    pub bbox: Rect,
    pub confidence: f32,
}
