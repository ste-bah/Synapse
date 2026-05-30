mod ocr;
mod search;
mod sources;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use rmcp::{ErrorData, handler::server::common, model::JsonObject, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synapse_capture::{CaptureBackend, CaptureConfig, CaptureTarget, resolve_capture_target};
use synapse_core::{ElementId, ForegroundContext, OcrBackend, PerceptionMode, Rect, error_codes};
use synapse_perception::{ObservationInput, ObserveInclude, parse_perception_mode};

pub use ocr::read_text_in_state;
use search::{element_match, entity_match};
pub use sources::{FsRecentTracker, populate_clipboard_summary, populate_fs_recent};
use sources::{platform_input, synthetic_notepad_input};

pub type SharedM1State = Arc<Mutex<M1State>>;

#[derive(Debug)]
pub struct M1State {
    pub capture_config: CaptureConfig,
    pub capture_generation: u64,
    pub perception_mode: PerceptionMode,
    pub synthetic: Option<ObservationInput>,
    pub force_no_perception: bool,
    pub force_observe_internal: bool,
    pub last_observed_foreground: Option<ForegroundContext>,
    pub everquest_log_cursor: Option<EverQuestLogCursorState>,
    pub everquest_event_seq: u64,
    pub fs_recent_tracker: FsRecentTracker,
}

impl M1State {
    #[must_use]
    pub fn from_env() -> Self {
        let synthetic = match std::env::var("SYNAPSE_MCP_SYNTHETIC_FIXTURE") {
            Ok(value) if value.eq_ignore_ascii_case("notepad") => Some(synthetic_notepad_input()),
            _ => None,
        };
        let force_no_perception = std::env::var("SYNAPSE_MCP_FORCE_NO_PERCEPTION")
            .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
        let force_observe_internal = std::env::var("SYNAPSE_MCP_FORCE_OBSERVE_INTERNAL")
            .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
        Self {
            capture_config: CaptureConfig::default().with_env_backend(),
            capture_generation: 0,
            perception_mode: PerceptionMode::Auto,
            synthetic,
            force_no_perception,
            force_observe_internal,
            last_observed_foreground: None,
            everquest_log_cursor: None,
            everquest_event_seq: 0,
            fs_recent_tracker: FsRecentTracker::from_env(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EverQuestLogCursorState {
    pub path: PathBuf,
    pub offset: u64,
}

impl Default for M1State {
    fn default() -> Self {
        Self::from_env()
    }
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObserveParams {
    #[serde(default)]
    pub include: Vec<ObserveSlot>,
    #[serde(default)]
    pub depth: Option<u32>,
    #[serde(default)]
    pub max_elements: Option<usize>,
    #[serde(default)]
    pub since_event_seq: Option<u64>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObserveSlot {
    Focused,
    Elements,
    Entities,
    Hud,
    Audio,
    Events,
    Clipboard,
    Fs,
    Diagnostics,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindParams {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub name_substring: Option<String>,
    #[serde(default)]
    pub automation_id: Option<String>,
    #[serde(default)]
    pub scope: Option<FindScope>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub in_window: Option<ElementId>,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindScope {
    Elements,
    Entities,
    #[default]
    Both,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindResponse {
    pub results: Vec<FindResult>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindResult {
    pub kind: FindResultKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<ElementId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_label: Option<String>,
    pub bbox: Rect,
    pub score: f32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindResultKind {
    Element,
    Entity,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadTextParams {
    #[serde(default)]
    pub region: Option<Rect>,
    #[serde(default)]
    pub element_id: Option<ElementId>,
    #[serde(default)]
    #[expect(
        dead_code,
        reason = "backend remains part of the M1 request schema; provider selection is not branched here yet"
    )]
    pub backend: OcrBackend,
    #[serde(default)]
    pub lang_hint: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetCaptureTargetParams {
    pub target: CaptureTargetParam,
    #[serde(default)]
    pub min_update_interval_ms: Option<u64>,
    #[serde(default)]
    pub cursor_visible: Option<bool>,
    #[serde(default)]
    pub dirty_region_only: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaptureTargetParam {
    Primary,
    Monitor { monitor_index: u32 },
    Window { window_hwnd: i64 },
    ElementWindow { element_id: ElementId },
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetCaptureTargetResponse {
    pub previous: CaptureTargetWire,
    pub current: CaptureTargetWire,
    pub generation: u64,
    pub backend: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaptureTargetWire {
    Primary,
    Monitor { monitor_index: u32 },
    Window { window_hwnd: i64 },
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPerceptionModeParams {
    pub mode: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPerceptionModeResponse {
    pub previous: PerceptionMode,
    pub mode: PerceptionMode,
    pub rationale: String,
}

pub fn empty_input_schema() -> Arc<JsonObject> {
    common::schema_for_type::<EmptyParams>()
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyParams {}

#[must_use]
pub fn observe_include(params: &ObserveParams) -> ObserveInclude {
    let mut include = if params.include.is_empty() {
        ObserveInclude::default()
    } else {
        ObserveInclude {
            focused: false,
            elements: false,
            entities: false,
            hud: false,
            audio: false,
            events: false,
            clipboard: false,
            fs: false,
            diagnostics: false,
            max_subtree_depth: 2,
            max_subtree_nodes: 60,
            max_entities: 60,
        }
    };
    for slot in &params.include {
        match slot {
            ObserveSlot::Focused => include.focused = true,
            ObserveSlot::Elements => include.elements = true,
            ObserveSlot::Entities => include.entities = true,
            ObserveSlot::Hud => include.hud = true,
            ObserveSlot::Audio => include.audio = true,
            ObserveSlot::Events => include.events = true,
            ObserveSlot::Clipboard => include.clipboard = true,
            ObserveSlot::Fs => include.fs = true,
            ObserveSlot::Diagnostics => include.diagnostics = true,
        }
    }
    include.max_subtree_depth = params.depth.unwrap_or(2).min(6);
    include.max_subtree_nodes = params.max_elements.unwrap_or(60).clamp(1, 500);
    include
}

pub fn current_input(state: &M1State, depth: u32) -> Result<ObservationInput, ErrorData> {
    if state.force_observe_internal {
        return Err(mcp_error(
            error_codes::OBSERVE_INTERNAL,
            "forced observe internal error",
        ));
    }
    if state.force_no_perception {
        return Err(mcp_error(
            error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
            "no perception source is available",
        ));
    }
    if let Some(input) = &state.synthetic {
        let mut input = input.clone();
        if state.perception_mode != PerceptionMode::Auto {
            input.mode_override = Some(state.perception_mode);
        }
        return Ok(input);
    }
    platform_input(depth, state.perception_mode)
}

pub fn find_in_state(state: &M1State, params: &FindParams) -> Result<FindResponse, ErrorData> {
    let input = current_input(state, 2)?;
    let limit = params.limit.unwrap_or(5).clamp(1, 20);
    let mut results = Vec::new();
    if matches!(
        params.scope.unwrap_or_default(),
        FindScope::Elements | FindScope::Both
    ) {
        results.extend(
            input
                .elements
                .iter()
                .filter_map(|node| element_match(node, params)),
        );
    }
    if matches!(
        params.scope.unwrap_or_default(),
        FindScope::Entities | FindScope::Both
    ) {
        results.extend(
            input
                .entities
                .iter()
                .filter_map(|entity| entity_match(entity, params)),
        );
    }
    results.sort_by(|left, right| right.score.total_cmp(&left.score));
    results.truncate(limit);
    Ok(FindResponse { results })
}

pub fn set_capture_target_in_state(
    state: &mut M1State,
    params: SetCaptureTargetParams,
) -> Result<SetCaptureTargetResponse, ErrorData> {
    let previous = capture_target_wire(&state.capture_config.target);
    let mut config = state.capture_config.clone();
    config.target = capture_target_from_param(params.target)?;
    if let Some(interval) = params.min_update_interval_ms {
        config.min_update_interval_ms = interval.max(1);
    }
    if let Some(cursor_visible) = params.cursor_visible {
        config.cursor_visible = cursor_visible;
    }
    if let Some(dirty_region_only) = params.dirty_region_only {
        config.dirty_region_only = dirty_region_only;
    }
    let resolved =
        resolve_capture_target(&config).map_err(|err| mcp_error(err.code(), err.to_string()))?;
    state.capture_config = config;
    state.capture_generation = state.capture_generation.saturating_add(1);
    Ok(SetCaptureTargetResponse {
        previous,
        current: capture_target_wire(&resolved.target),
        generation: state.capture_generation,
        backend: capture_backend_name(resolved.backend).to_owned(),
    })
}

pub fn set_perception_mode_in_state(
    state: &mut M1State,
    params: &SetPerceptionModeParams,
) -> Result<SetPerceptionModeResponse, ErrorData> {
    let previous = state.perception_mode;
    let mode = parse_perception_mode(&params.mode)
        .map_err(|err| mcp_error(err.code(), err.to_string()))?;
    state.perception_mode = mode;
    Ok(SetPerceptionModeResponse {
        previous,
        mode,
        rationale: mode_rationale(mode).to_owned(),
    })
}

pub fn mcp_error(code: &'static str, message: impl Into<String>) -> ErrorData {
    let message = message.into();
    ErrorData::new(
        rmcp::model::ErrorCode(-32099),
        message,
        Some(json!({ "code": code })),
    )
}

fn capture_target_from_param(param: CaptureTargetParam) -> Result<CaptureTarget, ErrorData> {
    match param {
        CaptureTargetParam::Primary => Ok(CaptureTarget::Primary),
        CaptureTargetParam::Monitor { monitor_index } => {
            Ok(CaptureTarget::Monitor { monitor_index })
        }
        CaptureTargetParam::Window { window_hwnd } => {
            Ok(CaptureTarget::Window { hwnd: window_hwnd })
        }
        CaptureTargetParam::ElementWindow { element_id } => element_id
            .parts()
            .map(|parts| CaptureTarget::Window { hwnd: parts.hwnd })
            .map_err(|err| mcp_error(error_codes::CAPTURE_TARGET_INVALID, err.to_string())),
    }
}

const fn capture_target_wire(target: &CaptureTarget) -> CaptureTargetWire {
    match target {
        CaptureTarget::Primary => CaptureTargetWire::Primary,
        CaptureTarget::Monitor { monitor_index } => CaptureTargetWire::Monitor {
            monitor_index: *monitor_index,
        },
        CaptureTarget::Window { hwnd } => CaptureTargetWire::Window { window_hwnd: *hwnd },
    }
}

const fn capture_backend_name(backend: CaptureBackend) -> &'static str {
    match backend {
        CaptureBackend::GraphicsCaptureApi => "graphics_capture_api",
        CaptureBackend::DxgiDuplication => "dxgi_duplication",
    }
}

const fn mode_rationale(mode: PerceptionMode) -> &'static str {
    match mode {
        PerceptionMode::Auto => "auto_select_by_foreground_and_a11y_density",
        PerceptionMode::A11yOnly => "manual_a11y_only",
        PerceptionMode::PixelOnly => "manual_pixel_only",
        PerceptionMode::Hybrid => "manual_hybrid",
    }
}
