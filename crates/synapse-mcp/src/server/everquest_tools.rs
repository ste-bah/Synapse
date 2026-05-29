use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use rmcp::{ErrorData, model::ErrorCode, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use synapse_action::{ActionBackend, EmitState};
use synapse_core::{Action, Backend, FocusedElement, Key, KeyCode, Rect, error_codes};
use synapse_everquest::{EverQuestLogKind, EverQuestLogTailBatch, tail_log};
use synapse_perception::{PerceptionError, TextRegion, read_text};
use tokio::time::sleep;

use super::{
    Json, Parameters, SynapseService, everquest_log::EVERQUEST_PROFILE_ID, tool, tool_router,
};
use crate::m1::{current_input, mcp_error};

const TOOL: &str = "everquest_loc_probe";
const LOC_COMMAND: &str = "/loc";
const LOC_KEY_HOLD_MS: u32 = 33;
const LOC_INTER_KEY_DELAY: Duration = Duration::from_millis(20);
const MAX_LOC_LOG_BYTES: usize = 64 * 1024;
const MAX_LOC_LOG_EVENTS: usize = 128;
const LOC_POLL_INTERVAL: Duration = Duration::from_millis(100);
const LOC_TIMEOUT: Duration = Duration::from_secs(3);
const CHAT_STATE_ROW_KIND: &str = "everquest.chat_input_state";
const CHAT_INPUT_MIN_VISIBLE_W: i32 = 80;
const CHAT_INPUT_MIN_VISIBLE_H: i32 = 24;
const CHAT_INPUT_LINE_CROP_H: i32 = 100;
const CHAT_INPUT_EMPTY_CONFIDENCE: f32 = 0.82;
const CHAT_INPUT_MIN_TEXT_CONFIDENCE: f32 = 0.60;
const MAIN_CHAT_SECTION: &str = "MainChat";
const CHAT_INPUT_SOURCE_MODE: &str = "everquest_mainchat_layout_ocr_verified_input_crop";

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestLocProbeParams {}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestChatInputStateParams {}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestLocProbeResponse {
    pub ok: bool,
    pub command: String,
    pub coordinate_order: String,
    pub log_path: String,
    pub start_offset: u64,
    pub next_offset: u64,
    pub file_len_bytes: u64,
    pub bytes_read: usize,
    pub event_count: usize,
    pub you_say_count: usize,
    pub location: EverQuestLocProbeLocation,
    pub chat_input_state: EverQuestChatInputState,
    pub elapsed_ms: u32,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestLocProbeLocation {
    pub display_y: f64,
    pub display_x: f64,
    pub display_z: f64,
    pub log_timestamp: String,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestChatInputStateResponse {
    pub ok: bool,
    pub chat_input_state: EverQuestChatInputState,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestChatInputState {
    pub row_kind: String,
    pub visible: bool,
    pub text_present: bool,
    pub confidence: f32,
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denial_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_region: Option<Rect>,
    pub source_mode: String,
    pub text_len_estimate: usize,
    pub word_count: usize,
    pub ocr_status: String,
    pub ocr_confidence: f32,
    pub foreground: EverQuestChatInputForeground,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<EverQuestChatInputLayout>,
    pub source_refs: Vec<EverQuestChatInputSourceRef>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestChatInputForeground {
    pub is_everquest_foreground: bool,
    pub hwnd: i64,
    pub process_name: String,
    pub window_title: String,
    pub window_bounds: Rect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestChatInputLayout {
    pub path: String,
    pub section: String,
    pub line_start: usize,
    pub line_end: usize,
    pub coordinate_mode: String,
    pub full_region: Rect,
    pub clipped_region: Rect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestChatInputSourceRef {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[tool_router(router = everquest_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Read the visible EverQuest chat input pollution state from the foreground window, UI layout file, and OCR crop"
    )]
    pub async fn everquest_chat_input_state(
        &self,
        _params: Parameters<EverQuestChatInputStateParams>,
    ) -> Result<Json<EverQuestChatInputStateResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "everquest_chat_input_state",
            "tool.invocation kind=everquest_chat_input_state"
        );
        let chat_input_state = self.detect_everquest_chat_input_state();
        Ok(Json(EverQuestChatInputStateResponse {
            ok: chat_input_state.allows_text_command(),
            chat_input_state,
        }))
    }

    #[tool(
        description = "Send the literal EverQuest /loc command to the foreground everquest.live window and verify the appended EQ log coordinate line"
    )]
    pub async fn everquest_loc_probe(
        &self,
        _params: Parameters<EverQuestLocProbeParams>,
    ) -> Result<Json<EverQuestLocProbeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TOOL,
            "tool.invocation kind=everquest_loc_probe"
        );

        let request_details = json!({
            "command": LOC_COMMAND,
            "literal_only": true,
            "free_text_allowed": false,
            "coordinate_order": "everquest_display_y_x_z",
        });

        if let Err(error) = self.ensure_supported_use_allows_action(TOOL) {
            self.audit_action_denied_with_details(TOOL, &error, &request_details);
            return Err(error);
        }
        if let Err(error) = self.ensure_active_everquest_profile() {
            self.audit_action_denied_with_details(TOOL, &error, &request_details);
            return Err(error);
        }
        let chat_input_state = match self.ensure_loc_probe_chat_guard() {
            Ok(chat_input_state) => chat_input_state,
            Err(error) => {
                self.audit_action_denied_with_details(TOOL, &error, &request_details);
                return Err(error);
            }
        };

        let active = match self.resolve_active_everquest_log() {
            Ok(active) => active,
            Err(detail) => {
                let error = loc_probe_error("active_log_unavailable", detail, &json!({}));
                self.audit_action_denied_with_details(TOOL, &error, &request_details);
                return Err(error);
            }
        };
        let start_offset = std::fs::metadata(&active.log.path)
            .map_err(|error| {
                loc_probe_error(
                    "log_metadata_unreadable",
                    format!("read active EverQuest log metadata: {error}"),
                    &json!({ "path": active.log.path.display().to_string() }),
                )
            })?
            .len();

        self.audit_action_started_with_details(
            TOOL,
            &json!({
                "request": request_details,
                "log_path": active.log.path.display().to_string(),
                "start_offset": start_offset,
                "chat_input_state": chat_input_state,
            }),
        )?;

        let started = Instant::now();
        let result = async {
            self.execute_literal_loc_command().await?;
            self.read_loc_probe_result(
                &active.log.path,
                start_offset,
                &chat_input_state,
                u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
            )
            .await
        }
        .await;

        self.audit_action_result(TOOL, &result)?;
        result.map(Json)
    }
}

impl SynapseService {
    fn ensure_active_everquest_profile(&self) -> Result<(), ErrorData> {
        let runtime = self.profile_runtime()?;
        let active_profile_id = runtime
            .active_profile_id()
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        if active_profile_id.as_deref() == Some(EVERQUEST_PROFILE_ID) {
            return Ok(());
        }
        Err(loc_probe_error(
            "active_profile_mismatch",
            format!("{TOOL} requires active profile {EVERQUEST_PROFILE_ID}"),
            &json!({
                "active_profile_id": active_profile_id,
                "required_profile_id": EVERQUEST_PROFILE_ID,
            }),
        ))
    }

    fn ensure_loc_probe_chat_guard(&self) -> Result<EverQuestChatInputState, ErrorData> {
        let mut chat_input_state = self.detect_everquest_chat_input_state();
        if !chat_input_state.allows_text_command() {
            return Err(loc_probe_error(
                "chat_input_state_not_safe",
                format!("{TOOL} refused to emit /loc because visible chat input state is not safe"),
                &json!({ "chat_input_state": chat_input_state }),
            ));
        }
        let input = {
            let state = self.m1_state()?;
            current_input(&state, 1)?
        };
        if let Some(reason) = focused_text_entry_pollution_reason(input.focused.as_ref()) {
            "deny_focused_text_entry_not_empty".clone_into(&mut chat_input_state.decision);
            chat_input_state.denial_reason = Some(reason.clone());
            chat_input_state.text_present = true;
            chat_input_state.confidence = 1.0;
            return Err(loc_probe_error(
                "focused_text_entry_not_empty",
                format!("{TOOL} refused to append /loc into an existing focused text entry"),
                &json!({
                    "focused_text_entry_reason": reason,
                    "chat_input_state": chat_input_state,
                }),
            ));
        }
        Ok(chat_input_state)
    }

    async fn execute_literal_loc_command(&self) -> Result<(), ErrorData> {
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        let actions = literal_loc_actions();
        if let Some(recording) = recording {
            let mut emit_state = EmitState::new();
            for action in &actions {
                recording
                    .execute(action, &mut emit_state)
                    .map_err(|error| mcp_error(error.code(), error.to_string()))?;
            }
            return Ok(());
        }
        for action in actions {
            handle
                .execute(action)
                .await
                .map_err(|error| mcp_error(error.code(), error.to_string()))?;
            sleep(LOC_INTER_KEY_DELAY).await;
        }
        Ok(())
    }

    async fn read_loc_probe_result(
        &self,
        log_path: &Path,
        start_offset: u64,
        chat_input_state: &EverQuestChatInputState,
        initial_elapsed_ms: u32,
    ) -> Result<EverQuestLocProbeResponse, ErrorData> {
        let started = Instant::now();
        let last_batch = loop {
            let batch = tail_log(
                log_path,
                start_offset,
                MAX_LOC_LOG_BYTES,
                MAX_LOC_LOG_EVENTS,
            )
            .map_err(|error| {
                loc_probe_error(
                    "log_tail_failed",
                    format!("tail active EverQuest log after /loc: {error}"),
                    &json!({
                        "path": log_path.display().to_string(),
                        "start_offset": start_offset,
                        "chat_input_state": chat_input_state,
                    }),
                )
            })?;
            let you_say_count = you_say_count(&batch);
            if you_say_count > 0 {
                return Err(loc_probe_error(
                    "chat_pollution_detected",
                    format!("{TOOL} detected player say output after /loc dispatch"),
                    &log_batch_detail(&batch, you_say_count, Some(chat_input_state)),
                ));
            }
            if let Some(response) = loc_probe_response_from_batch(
                &batch,
                you_say_count,
                chat_input_state.clone(),
                elapsed_ms_since(initial_elapsed_ms, started),
            ) {
                return Ok(response);
            }
            if started.elapsed() >= LOC_TIMEOUT {
                break batch;
            }
            sleep(LOC_POLL_INTERVAL).await;
        };

        Err(loc_probe_error(
            "location_log_line_absent",
            format!("{TOOL} did not observe a /loc coordinate line before timeout"),
            &log_batch_detail(&last_batch, 0, Some(chat_input_state)),
        ))
    }

    fn detect_everquest_chat_input_state(&self) -> EverQuestChatInputState {
        let foreground = self
            .m1_state()
            .and_then(|state| current_input(&state, 1))
            .map(|input| input.foreground);
        let foreground = match foreground {
            Ok(foreground) => foreground,
            Err(error) => {
                return chat_state_fail_closed(
                    "foreground_unavailable",
                    format!("read foreground state failed: {}", error.message),
                    None,
                    Vec::new(),
                    EverQuestChatInputForeground::unknown(),
                );
            }
        };
        let foreground_ref = EverQuestChatInputForeground {
            is_everquest_foreground: foreground
                .profile_id
                .as_deref()
                .is_some_and(|profile_id| profile_id == EVERQUEST_PROFILE_ID)
                || foreground.process_name.eq_ignore_ascii_case("eqgame.exe"),
            hwnd: foreground.hwnd,
            process_name: foreground.process_name.clone(),
            window_title: foreground.window_title.clone(),
            window_bounds: foreground.window_bounds,
            profile_id: foreground.profile_id.clone(),
        };
        if !foreground_ref.is_everquest_foreground {
            return chat_state_fail_closed(
                "foreground_not_everquest",
                format!(
                    "foreground process {:?} is not EverQuest",
                    foreground.process_name
                ),
                None,
                Vec::new(),
                foreground_ref,
            );
        }

        let active = match self.resolve_active_everquest_log() {
            Ok(active) => active,
            Err(detail) => {
                return chat_state_fail_closed(
                    "active_log_unavailable",
                    detail,
                    None,
                    Vec::new(),
                    foreground_ref,
                );
            }
        };
        let layout = match read_main_chat_layout(
            &active.install_root,
            &active.log.identity.server,
            &foreground_ref,
        ) {
            Ok(layout) => layout,
            Err(detail) => {
                return chat_state_fail_closed(
                    "ui_layout_unavailable",
                    detail,
                    None,
                    Vec::new(),
                    foreground_ref,
                );
            }
        };
        chat_state_from_layout(foreground_ref, &layout)
    }
}

fn focused_text_entry_pollution_reason(focused: Option<&FocusedElement>) -> Option<String> {
    let focused = focused?;
    let role = focused.role.to_ascii_lowercase();
    let name = focused.name.to_ascii_lowercase();
    let is_text_entry = role.contains("edit")
        || role.contains("text")
        || role.contains("document")
        || name.contains("chat")
        || focused.patterns.iter().any(|pattern| {
            matches!(
                pattern,
                synapse_core::UiaPattern::Text | synapse_core::UiaPattern::Value
            )
        });
    if !is_text_entry {
        return None;
    }
    let value_len = focused.value.as_deref().map_or("", str::trim).len();
    let selected_len = focused.selected_text.as_deref().map_or("", str::trim).len();
    (value_len > 0 || selected_len > 0).then(|| {
        format!(
            "focused role={:?} name={:?} value_len={} selected_len={}",
            focused.role, focused.name, value_len, selected_len
        )
    })
}

fn literal_loc_actions() -> Vec<Action> {
    [
        loc_key(KeyCode::Symbol { value: '/' }),
        loc_key(KeyCode::Symbol { value: 'l' }),
        loc_key(KeyCode::Symbol { value: 'o' }),
        loc_key(KeyCode::Symbol { value: 'c' }),
        loc_key(KeyCode::Named {
            value: "enter".to_owned(),
        }),
    ]
    .into_iter()
    .map(|key| Action::KeyPress {
        key,
        hold_ms: LOC_KEY_HOLD_MS,
        backend: Backend::Auto,
    })
    .collect()
}

const fn loc_key(code: KeyCode) -> Key {
    Key {
        code,
        use_scancode: false,
    }
}

fn loc_probe_response_from_batch(
    batch: &EverQuestLogTailBatch,
    you_say_count: usize,
    chat_input_state: EverQuestChatInputState,
    elapsed_ms: u32,
) -> Option<EverQuestLocProbeResponse> {
    let event = batch
        .events
        .iter()
        .find(|event| event.kind == EverQuestLogKind::Location)?;
    let location = event.location.as_ref()?;
    Some(EverQuestLocProbeResponse {
        ok: true,
        command: LOC_COMMAND.to_owned(),
        coordinate_order: "everquest_display_y_x_z".to_owned(),
        log_path: batch.path.display().to_string(),
        start_offset: batch.start_offset,
        next_offset: batch.next_offset,
        file_len_bytes: batch.file_len_bytes,
        bytes_read: batch.bytes_read,
        event_count: batch.events.len(),
        you_say_count,
        location: EverQuestLocProbeLocation {
            display_y: location.display_y,
            display_x: location.display_x,
            display_z: location.display_z,
            log_timestamp: event.timestamp.format("%Y-%m-%dT%H:%M:%S").to_string(),
            summary: event.summary.clone(),
        },
        chat_input_state,
        elapsed_ms,
    })
}

fn elapsed_ms_since(initial_elapsed_ms: u32, started: Instant) -> u32 {
    let poll_elapsed_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    initial_elapsed_ms.saturating_add(poll_elapsed_ms)
}

fn you_say_count(batch: &EverQuestLogTailBatch) -> usize {
    batch
        .events
        .iter()
        .filter(|event| {
            event.kind == EverQuestLogKind::Say
                && event
                    .actor
                    .as_deref()
                    .is_some_and(|actor| actor.eq_ignore_ascii_case("you"))
        })
        .count()
}

fn log_batch_detail(
    batch: &EverQuestLogTailBatch,
    you_say_count: usize,
    chat_input_state: Option<&EverQuestChatInputState>,
) -> Value {
    let mut detail = json!({
        "path": batch.path.display().to_string(),
        "start_offset": batch.start_offset,
        "next_offset": batch.next_offset,
        "file_len_bytes": batch.file_len_bytes,
        "bytes_read": batch.bytes_read,
        "event_count": batch.events.len(),
        "you_say_count": you_say_count,
        "truncated_by_bytes": batch.truncated_by_bytes,
        "truncated_by_events": batch.truncated_by_events,
    });
    if let Some(chat_input_state) = chat_input_state {
        detail["chat_input_state_before_dispatch"] = json!(chat_input_state);
    }
    detail
}

fn loc_probe_error(reason: &'static str, message: impl Into<String>, detail: &Value) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        message.into(),
        Some(json!({
            "code": error_codes::ACTION_TARGET_INVALID,
            "tool": TOOL,
            "reason": reason,
            "detail": detail,
        })),
    )
}

impl EverQuestChatInputState {
    const fn allows_text_command(&self) -> bool {
        self.visible && !self.text_present && self.denial_reason.is_none()
    }
}

impl EverQuestChatInputForeground {
    const fn unknown() -> Self {
        Self {
            is_everquest_foreground: false,
            hwnd: 0,
            process_name: String::new(),
            window_title: String::new(),
            window_bounds: Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            },
            profile_id: None,
        }
    }
}

#[derive(Clone, Debug)]
struct MainChatLayoutEvidence {
    path: PathBuf,
    line_start: usize,
    line_end: usize,
    coordinate_mode: String,
    full_region: Rect,
    proof_region: Rect,
    input_region: Rect,
    minimized: bool,
    hidden: bool,
    content_sha256: String,
    proof_status: String,
    proof_word_count: usize,
    proof_score: i32,
}

#[derive(Clone, Debug)]
struct MainChatLayoutCandidate {
    coordinate_mode: String,
    full_region: Rect,
    minimized: bool,
    hidden: bool,
}

#[derive(Clone, Debug)]
struct MainChatLayoutProof {
    status: String,
    word_count: usize,
    score: i32,
}

fn chat_state_from_layout(
    foreground: EverQuestChatInputForeground,
    layout_evidence: &MainChatLayoutEvidence,
) -> EverQuestChatInputState {
    let source_refs = vec![
        EverQuestChatInputSourceRef {
            kind: "everquest_ui_layout_file".to_owned(),
            path: Some(layout_evidence.path.display().to_string()),
            section: Some(MAIN_CHAT_SECTION.to_owned()),
            region: Some(layout_evidence.full_region),
            content_sha256: Some(layout_evidence.content_sha256.clone()),
            note: Some(layout_evidence.coordinate_mode.clone()),
        },
        EverQuestChatInputSourceRef {
            kind: "everquest_mainchat_visible_ocr_proof".to_owned(),
            path: None,
            section: Some(MAIN_CHAT_SECTION.to_owned()),
            region: Some(layout_evidence.proof_region),
            content_sha256: None,
            note: Some(format!(
                "{} words={} score={}",
                layout_evidence.proof_status,
                layout_evidence.proof_word_count,
                layout_evidence.proof_score
            )),
        },
    ];
    if layout_evidence.hidden {
        return chat_state_fail_closed(
            "chat_window_hidden",
            "MainChat section is hidden in the active UI layout".to_owned(),
            Some(chat_layout_readback(layout_evidence)),
            source_refs,
            foreground,
        );
    }
    if layout_evidence.minimized {
        return chat_state_fail_closed(
            "chat_window_minimized",
            "MainChat section is minimized in the active UI layout".to_owned(),
            Some(chat_layout_readback(layout_evidence)),
            source_refs,
            foreground,
        );
    }
    if layout_evidence.proof_region.w < CHAT_INPUT_MIN_VISIBLE_W
        || layout_evidence.proof_region.h < CHAT_INPUT_MIN_VISIBLE_H
    {
        return chat_state_fail_closed(
            "layout_too_small_after_clip",
            format!(
                "MainChat visible proof region is too small: {:?}",
                layout_evidence.proof_region
            ),
            Some(chat_layout_readback(layout_evidence)),
            source_refs,
            foreground,
        );
    }
    let input_region = layout_evidence.input_region;
    let ocr = read_chat_input_ocr(input_region);
    let mut state = EverQuestChatInputState {
        row_kind: CHAT_STATE_ROW_KIND.to_owned(),
        visible: true,
        text_present: ocr.text_present,
        confidence: ocr.confidence,
        decision: ocr.decision,
        denial_reason: ocr.denial_reason,
        source_region: Some(input_region),
        source_mode: CHAT_INPUT_SOURCE_MODE.to_owned(),
        text_len_estimate: ocr.text_len_estimate,
        word_count: ocr.word_count,
        ocr_status: ocr.status,
        ocr_confidence: ocr.ocr_confidence,
        foreground,
        layout: Some(chat_layout_readback(layout_evidence)),
        source_refs,
    };
    state.source_refs.push(EverQuestChatInputSourceRef {
        kind: "everquest_chat_input_ocr_crop".to_owned(),
        path: None,
        section: None,
        region: Some(input_region),
        content_sha256: None,
        note: Some(state.ocr_status.clone()),
    });
    state
}

fn chat_layout_readback(evidence: &MainChatLayoutEvidence) -> EverQuestChatInputLayout {
    EverQuestChatInputLayout {
        path: evidence.path.display().to_string(),
        section: MAIN_CHAT_SECTION.to_owned(),
        line_start: evidence.line_start,
        line_end: evidence.line_end,
        coordinate_mode: evidence.coordinate_mode.clone(),
        full_region: evidence.full_region,
        clipped_region: evidence.proof_region,
        content_sha256: Some(evidence.content_sha256.clone()),
    }
}

struct ChatInputOcrReadback {
    text_present: bool,
    confidence: f32,
    decision: String,
    denial_reason: Option<String>,
    text_len_estimate: usize,
    word_count: usize,
    status: String,
    ocr_confidence: f32,
}

fn read_chat_input_ocr(region: Rect) -> ChatInputOcrReadback {
    match read_text(region) {
        Ok(words) => chat_ocr_from_words(&words),
        Err(PerceptionError::OcrNoText { .. }) => ChatInputOcrReadback {
            text_present: false,
            confidence: CHAT_INPUT_EMPTY_CONFIDENCE,
            decision: "allow_empty_chat_input".to_owned(),
            denial_reason: None,
            text_len_estimate: 0,
            word_count: 0,
            status: "ocr_no_text".to_owned(),
            ocr_confidence: 0.0,
        },
        Err(error) => ChatInputOcrReadback {
            text_present: false,
            confidence: 0.0,
            decision: "deny_ocr_unavailable".to_owned(),
            denial_reason: Some(format!("{}: {error}", error.code())),
            text_len_estimate: 0,
            word_count: 0,
            status: error.code().to_owned(),
            ocr_confidence: 0.0,
        },
    }
}

fn chat_ocr_from_words(words: &[TextRegion]) -> ChatInputOcrReadback {
    let normalized_words = words
        .iter()
        .filter(|word| !word.text.trim().is_empty())
        .collect::<Vec<_>>();
    let word_count = normalized_words.len();
    let text_len_estimate = normalized_words
        .iter()
        .map(|word| word.text.trim().chars().count())
        .sum::<usize>();
    let ocr_confidence = aggregate_word_confidence(&normalized_words);
    if word_count > 0 && ocr_confidence < CHAT_INPUT_MIN_TEXT_CONFIDENCE {
        return ChatInputOcrReadback {
            text_present: false,
            confidence: ocr_confidence,
            decision: "deny_low_ocr_confidence".to_owned(),
            denial_reason: Some(format!(
                "OCR confidence {ocr_confidence:.3} is below minimum {CHAT_INPUT_MIN_TEXT_CONFIDENCE:.3}"
            )),
            text_len_estimate,
            word_count,
            status: "ocr_low_confidence_text".to_owned(),
            ocr_confidence,
        };
    }
    ChatInputOcrReadback {
        text_present: word_count > 0,
        confidence: if word_count > 0 {
            ocr_confidence
        } else {
            CHAT_INPUT_EMPTY_CONFIDENCE
        },
        decision: if word_count > 0 {
            "deny_visible_chat_input_text".to_owned()
        } else {
            "allow_empty_chat_input".to_owned()
        },
        denial_reason: (word_count > 0)
            .then(|| format!("OCR found {word_count} word(s) in the visible chat input crop")),
        text_len_estimate,
        word_count,
        status: "ocr_text".to_owned(),
        ocr_confidence,
    }
}

fn aggregate_word_confidence(words: &[&TextRegion]) -> f32 {
    if words.is_empty() {
        return 0.0;
    }
    let sum = words
        .iter()
        .map(|word| {
            if word.confidence.is_finite() {
                word.confidence.clamp(0.0, 1.0)
            } else {
                0.0
            }
        })
        .sum::<f32>();
    let count = u16::try_from(words.len()).unwrap_or(u16::MAX);
    sum / f32::from(count)
}

fn chat_state_fail_closed(
    reason: &'static str,
    detail: String,
    layout: Option<EverQuestChatInputLayout>,
    source_refs: Vec<EverQuestChatInputSourceRef>,
    foreground: EverQuestChatInputForeground,
) -> EverQuestChatInputState {
    EverQuestChatInputState {
        row_kind: CHAT_STATE_ROW_KIND.to_owned(),
        visible: false,
        text_present: false,
        confidence: 0.0,
        decision: format!("deny_{reason}"),
        denial_reason: Some(detail),
        source_region: None,
        source_mode: CHAT_INPUT_SOURCE_MODE.to_owned(),
        text_len_estimate: 0,
        word_count: 0,
        ocr_status: "not_run".to_owned(),
        ocr_confidence: 0.0,
        foreground,
        layout,
        source_refs,
    }
}

fn read_main_chat_layout(
    install_root: &Path,
    expected_server: &str,
    foreground: &EverQuestChatInputForeground,
) -> Result<MainChatLayoutEvidence, String> {
    let ui_path = choose_ui_layout_file(install_root, expected_server)?;
    let text = fs::read_to_string(&ui_path)
        .map_err(|error| format!("read EverQuest UI layout {}: {error}", ui_path.display()))?;
    let content_sha256 = sha256_hex(text.as_bytes());
    let section = parse_ini_section(&text, MAIN_CHAT_SECTION).ok_or_else(|| {
        format!(
            "{} section [{MAIN_CHAT_SECTION}] is absent",
            ui_path.display()
        )
    })?;
    let candidates = main_chat_layout_candidates(&section.values, foreground);
    if candidates.is_empty() {
        return Err(format!(
            "[{MAIN_CHAT_SECTION}] lacks any x/y/w/h coordinate set"
        ));
    }
    let selected = select_visible_main_chat_layout(&candidates)?;
    Ok(MainChatLayoutEvidence {
        path: ui_path,
        line_start: section.line_start,
        line_end: section.line_end,
        coordinate_mode: selected.candidate.coordinate_mode,
        full_region: selected.candidate.full_region,
        proof_region: selected.candidate.full_region,
        input_region: chat_input_region_from_main_chat(selected.candidate.full_region),
        minimized: selected.candidate.minimized,
        hidden: selected.candidate.hidden,
        content_sha256,
        proof_status: selected.proof.status,
        proof_word_count: selected.proof.word_count,
        proof_score: selected.proof.score,
    })
}

struct SelectedMainChatLayout {
    candidate: MainChatLayoutCandidate,
    proof: MainChatLayoutProof,
}

fn select_visible_main_chat_layout(
    candidates: &[MainChatLayoutCandidate],
) -> Result<SelectedMainChatLayout, String> {
    let mut best: Option<SelectedMainChatLayout> = None;
    let mut summaries = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let proof = read_main_chat_layout_proof(candidate.full_region);
        summaries.push(format!(
            "{}:{:?}:{}:{}",
            candidate.coordinate_mode, candidate.full_region, proof.status, proof.score
        ));
        if proof.score <= 0 {
            continue;
        }
        let replace_best = best
            .as_ref()
            .is_none_or(|current| proof.score > current.proof.score);
        if replace_best {
            best = Some(SelectedMainChatLayout {
                candidate: candidate.clone(),
                proof,
            });
        }
    }
    best.ok_or_else(|| {
        format!(
            "no [{MAIN_CHAT_SECTION}] coordinate candidate produced visible OCR proof; candidates={}",
            summaries.join(", ")
        )
    })
}

fn read_main_chat_layout_proof(region: Rect) -> MainChatLayoutProof {
    match read_text(region) {
        Ok(words) => main_chat_layout_proof_from_words(&words),
        Err(PerceptionError::OcrNoText { .. }) => MainChatLayoutProof {
            status: "ocr_no_text".to_owned(),
            word_count: 0,
            score: 0,
        },
        Err(error) => MainChatLayoutProof {
            status: error.code().to_owned(),
            word_count: 0,
            score: 0,
        },
    }
}

fn main_chat_layout_proof_from_words(words: &[TextRegion]) -> MainChatLayoutProof {
    let normalized = words
        .iter()
        .filter_map(|word| {
            let trimmed = word.text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_ascii_lowercase())
        })
        .collect::<Vec<_>>();
    let word_count = normalized.len();
    let has_main = normalized.iter().any(|word| word == "main");
    let has_chat = normalized.iter().any(|word| word == "chat");
    let has_chat_traffic = normalized.iter().any(|word| {
        word.contains("tells")
            || word.contains("combat")
            || word.contains("newplayers")
            || word.contains("say")
    });
    let score = i32::from(has_main && has_chat) * 1_000
        + i32::from(has_chat_traffic) * 100
        + i32::try_from(word_count.min(100)).unwrap_or(100);
    MainChatLayoutProof {
        status: "ocr_text".to_owned(),
        word_count,
        score,
    }
}

fn main_chat_layout_candidates(
    values: &BTreeMap<String, String>,
    foreground: &EverQuestChatInputForeground,
) -> Vec<MainChatLayoutCandidate> {
    let suffixes = layout_coordinate_suffixes(values);
    let reference_widths = layout_reference_widths(values, foreground);
    let hidden = values.get("Show").is_some_and(|value| value.trim() == "0");
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for suffix in suffixes {
        let Some(raw) = section_rect(values, &suffix) else {
            continue;
        };
        push_layout_candidate(
            &mut output,
            &mut seen,
            MainChatLayoutCandidate {
                coordinate_mode: suffix.clone(),
                full_region: raw,
                minimized: minimized_for_suffix(values, &suffix),
                hidden,
            },
        );
        let Some((base_w, _base_h)) = parse_resolution_suffix(&suffix) else {
            continue;
        };
        for reference_width in &reference_widths {
            if *reference_width <= 0 || *reference_width == base_w {
                continue;
            }
            let scale = f64::from(*reference_width) / f64::from(base_w);
            push_layout_candidate(
                &mut output,
                &mut seen,
                MainChatLayoutCandidate {
                    coordinate_mode: format!("{suffix}_scaled_to_width_{reference_width}"),
                    full_region: scale_rect(raw, scale),
                    minimized: minimized_for_suffix(values, &suffix),
                    hidden,
                },
            );
        }
    }
    output
}

fn push_layout_candidate(
    output: &mut Vec<MainChatLayoutCandidate>,
    seen: &mut BTreeSet<(i32, i32, i32, i32)>,
    candidate: MainChatLayoutCandidate,
) {
    if candidate.full_region.w < CHAT_INPUT_MIN_VISIBLE_W
        || candidate.full_region.h < CHAT_INPUT_MIN_VISIBLE_H
    {
        return;
    }
    let key = (
        candidate.full_region.x,
        candidate.full_region.y,
        candidate.full_region.w,
        candidate.full_region.h,
    );
    if seen.insert(key) {
        output.push(candidate);
    }
}

fn layout_coordinate_suffixes(values: &BTreeMap<String, String>) -> Vec<String> {
    let mut suffixes = values
        .keys()
        .filter_map(|key| key.strip_prefix("XPos"))
        .filter(|suffix| values.contains_key(&format!("YPos{suffix}")))
        .filter(|suffix| values.contains_key(&format!("Width{suffix}")))
        .filter(|suffix| values.contains_key(&format!("Height{suffix}")))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    suffixes.sort();
    if let Some(position) = suffixes.iter().position(|suffix| suffix == "Windowed") {
        let windowed = suffixes.remove(position);
        suffixes.insert(0, windowed);
    }
    suffixes
}

fn layout_reference_widths(
    values: &BTreeMap<String, String>,
    foreground: &EverQuestChatInputForeground,
) -> BTreeSet<i32> {
    let mut widths = BTreeSet::new();
    widths.extend(
        layout_coordinate_suffixes(values)
            .into_iter()
            .filter_map(|suffix| parse_resolution_suffix(&suffix).map(|(w, _h)| w)),
    );
    widths.extend(screen_reference_widths());
    if foreground.window_bounds.w > 0 {
        widths.insert(foreground.window_bounds.w);
    }
    widths
}

fn screen_reference_widths() -> BTreeSet<i32> {
    let mut widths = BTreeSet::new();
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXSCREEN, SM_CXVIRTUALSCREEN,
        };
        // SAFETY: GetSystemMetrics reads process-local desktop metrics.
        let primary_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        // SAFETY: GetSystemMetrics reads process-local desktop metrics.
        let virtual_width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        if primary_width > 0 {
            widths.insert(primary_width);
        }
        if virtual_width > 0 {
            widths.insert(virtual_width);
        }
    }
    widths
}

fn parse_resolution_suffix(suffix: &str) -> Option<(i32, i32)> {
    let (w, h) = suffix.split_once('x')?;
    Some((parse_i32(w)?, parse_i32(h)?))
}

fn minimized_for_suffix(values: &BTreeMap<String, String>, suffix: &str) -> bool {
    values
        .get(&format!("Minimized{suffix}"))
        .or_else(|| values.get("MinimizedWindowed"))
        .is_some_and(|value| value.trim() == "1")
}

fn chat_input_region_from_main_chat(region: Rect) -> Rect {
    let x_inset = region.w / 20;
    let y_offset = region.h.saturating_add(region.h.saturating_mul(17) / 20);
    Rect {
        x: region.x.saturating_sub(x_inset),
        y: region.y.saturating_add(y_offset),
        w: region.w.max(CHAT_INPUT_MIN_VISIBLE_W),
        h: CHAT_INPUT_LINE_CROP_H,
    }
}

#[allow(clippy::cast_possible_truncation)]
fn scale_rect(region: Rect, scale: f64) -> Rect {
    Rect {
        x: scale_i32(region.x, scale),
        y: scale_i32(region.y, scale),
        w: scale_i32(region.w, scale).max(0),
        h: scale_i32(region.h, scale).max(0),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn scale_i32(value: i32, scale: f64) -> i32 {
    if !scale.is_finite() || scale <= 0.0 {
        return value;
    }
    (f64::from(value) * scale)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

fn choose_ui_layout_file(install_root: &Path, expected_server: &str) -> Result<PathBuf, String> {
    let server_marker = format!("_{}_", expected_server.to_ascii_lowercase());
    let mut candidates = fs::read_dir(install_root)
        .map_err(|error| {
            format!(
                "read EverQuest install root {}: {error}",
                install_root.display()
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    let normalized = name.to_ascii_lowercase();
                    normalized.starts_with("ui_")
                        && normalized.contains(&server_marker)
                        && Path::new(name)
                            .extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("ini"))
                })
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| modified_time(path));
    candidates.pop().ok_or_else(|| {
        format!(
            "no EverQuest UI layout file matched server {expected_server:?} under {}",
            install_root.display()
        )
    })
}

struct IniSection {
    line_start: usize,
    line_end: usize,
    values: BTreeMap<String, String>,
}

fn parse_ini_section(text: &str, section_name: &str) -> Option<IniSection> {
    let mut in_section = false;
    let mut line_start = 0usize;
    let mut values = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index.saturating_add(1);
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_section {
                return Some(IniSection {
                    line_start,
                    line_end: line_number.saturating_sub(1),
                    values,
                });
            }
            in_section = &trimmed[1..trimmed.len().saturating_sub(1)] == section_name;
            if in_section {
                line_start = line_number;
            }
            continue;
        }
        if in_section {
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            values.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    in_section.then(|| IniSection {
        line_start,
        line_end: text.lines().count(),
        values,
    })
}

fn section_rect(values: &BTreeMap<String, String>, suffix: &str) -> Option<Rect> {
    Some(Rect {
        x: parse_i32(values.get(&format!("XPos{suffix}"))?)?,
        y: parse_i32(values.get(&format!("YPos{suffix}"))?)?,
        w: parse_i32(values.get(&format!("Width{suffix}"))?)?,
        h: parse_i32(values.get(&format!("Height{suffix}"))?)?,
    })
}

fn parse_i32(value: &str) -> Option<i32> {
    value.trim().parse().ok()
}

fn modified_time(path: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len().saturating_mul(2));
    for byte in digest {
        output.push(hex_digit(byte >> 4));
        output.push(hex_digit(byte & 0x0f));
    }
    output
}

fn hex_digit(value: u8) -> char {
    char::from(b"0123456789abcdef"[usize::from(value)])
}

#[cfg(test)]
mod tests {
    use synapse_core::{Rect, UiaPattern, element_id};

    use super::*;

    #[test]
    fn loc_batch_response_uses_structured_location_and_counts_chat() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("eqlog_Thenumberone_frostreaver.txt");
        std::fs::write(
            &path,
            "[Thu May 28 11:00:00 2026] Your Location is -1.25, 2.50, 3.75\r\n",
        )?;
        let batch = tail_log(&path, 0, MAX_LOC_LOG_BYTES, MAX_LOC_LOG_EVENTS)?;

        let response =
            loc_probe_response_from_batch(&batch, you_say_count(&batch), empty_chat_state(), 7)
                .unwrap_or_else(|| panic!("expected loc response"));

        assert_eq!(response.location.display_y, -1.25);
        assert_eq!(response.location.display_x, 2.5);
        assert_eq!(response.location.display_z, 3.75);
        assert_eq!(response.you_say_count, 0);
        assert!(response.chat_input_state.allows_text_command());
        assert_eq!(response.elapsed_ms, 7);
        Ok(())
    }

    #[test]
    fn chat_guard_denies_nonempty_focused_text_entry() {
        let focused = FocusedElement {
            element_id: element_id(7, "cafe"),
            name: "Chat Input".to_owned(),
            role: "Edit".to_owned(),
            automation_id: None,
            bbox: Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 20,
            },
            enabled: true,
            patterns: vec![UiaPattern::Text, UiaPattern::Value],
            value: Some("partial text".to_owned()),
            selected_text: None,
        };

        let reason = focused_text_entry_pollution_reason(Some(&focused))
            .unwrap_or_else(|| panic!("expected focused text pollution reason"));

        assert!(reason.contains("value_len=12"));
    }

    #[test]
    fn chat_guard_allows_empty_focused_text_entry_for_literal_command() {
        let focused = FocusedElement {
            element_id: element_id(7, "cafe"),
            name: "Chat Input".to_owned(),
            role: "Edit".to_owned(),
            automation_id: None,
            bbox: Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 20,
            },
            enabled: true,
            patterns: vec![UiaPattern::Text, UiaPattern::Value],
            value: Some("   ".to_owned()),
            selected_text: None,
        };

        assert!(focused_text_entry_pollution_reason(Some(&focused)).is_none());
    }

    #[test]
    fn literal_loc_actions_are_fixed_keypress_sequence() {
        let actions = literal_loc_actions();
        let keys = actions
            .iter()
            .map(|action| match action {
                Action::KeyPress { key, hold_ms, .. } => {
                    assert_eq!(*hold_ms, LOC_KEY_HOLD_MS);
                    match &key.code {
                        KeyCode::Symbol { value } => value.to_string(),
                        KeyCode::Named { value } => value.clone(),
                        KeyCode::HidCode { value } => value.to_string(),
                    }
                }
                other => panic!("unexpected /loc action: {other:?}"),
            })
            .collect::<Vec<_>>();

        assert_eq!(keys, ["/", "l", "o", "c", "enter"]);
    }

    #[test]
    fn chat_ocr_allows_empty_region_readback() {
        let readback = read_chat_input_ocr(Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        });

        assert!(!readback.text_present);
        assert_eq!(readback.decision, "allow_empty_chat_input");
        assert_eq!(readback.status, "ocr_no_text");
    }

    #[test]
    fn chat_ocr_denies_visible_text_without_persisting_body() {
        let words = vec![TextRegion {
            text: "synthetic-buffer".to_owned(),
            bbox: Rect {
                x: 10,
                y: 20,
                w: 120,
                h: 18,
            },
            confidence: 0.93,
        }];

        let readback = chat_ocr_from_words(&words);

        assert!(readback.text_present);
        assert_eq!(readback.decision, "deny_visible_chat_input_text");
        assert_eq!(readback.word_count, 1);
        assert_eq!(readback.text_len_estimate, "synthetic-buffer".len());
    }

    #[test]
    fn chat_hidden_layout_fails_closed() {
        let foreground = EverQuestChatInputForeground {
            is_everquest_foreground: true,
            hwnd: 42,
            process_name: "eqgame.exe".to_owned(),
            window_title: "EverQuest".to_owned(),
            window_bounds: Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            profile_id: Some(EVERQUEST_PROFILE_ID.to_owned()),
        };
        let layout = MainChatLayoutEvidence {
            path: PathBuf::from("UI_Thenumberone_frostreaver_WIZ.ini"),
            line_start: 10,
            line_end: 20,
            coordinate_mode: "Windowed".to_owned(),
            full_region: Rect {
                x: 90,
                y: 70,
                w: 200,
                h: 100,
            },
            proof_region: Rect {
                x: 90,
                y: 70,
                w: 200,
                h: 100,
            },
            input_region: Rect {
                x: 150,
                y: 50,
                w: 120,
                h: 48,
            },
            minimized: false,
            hidden: true,
            content_sha256: "00".repeat(32),
            proof_status: "ocr_text".to_owned(),
            proof_word_count: 2,
            proof_score: 1002,
        };

        let state = chat_state_from_layout(foreground, &layout);

        assert!(!state.allows_text_command());
        assert_eq!(state.decision, "deny_chat_window_hidden");
        assert!(state.source_refs.iter().any(|source| {
            source.kind == "everquest_ui_layout_file"
                && source.section.as_deref() == Some(MAIN_CHAT_SECTION)
        }));
    }

    #[test]
    fn chat_layout_candidates_include_scaled_resolution_modes() {
        let mut values = BTreeMap::new();
        values.insert("Show".to_owned(), "1".to_owned());
        values.insert("XPos2560x1369".to_owned(), "1186".to_owned());
        values.insert("YPos2560x1369".to_owned(), "1156".to_owned());
        values.insert("Width2560x1369".to_owned(), "515".to_owned());
        values.insert("Height2560x1369".to_owned(), "198".to_owned());
        values.insert("Minimized2560x1369".to_owned(), "0".to_owned());
        values.insert("XPos3413x1369".to_owned(), "1193".to_owned());
        values.insert("YPos3413x1369".to_owned(), "664".to_owned());
        values.insert("Width3413x1369".to_owned(), "515".to_owned());
        values.insert("Height3413x1369".to_owned(), "198".to_owned());
        values.insert("Minimized3413x1369".to_owned(), "0".to_owned());
        let foreground = EverQuestChatInputForeground {
            is_everquest_foreground: true,
            hwnd: 42,
            process_name: "eqgame.exe".to_owned(),
            window_title: "EverQuest".to_owned(),
            window_bounds: Rect {
                x: 0,
                y: 35,
                w: 5144,
                h: 2112,
            },
            profile_id: Some(EVERQUEST_PROFILE_ID.to_owned()),
        };

        let candidates = main_chat_layout_candidates(&values, &foreground);

        assert!(candidates.iter().any(|candidate| {
            candidate.coordinate_mode == "2560x1369_scaled_to_width_3413"
                && candidate.full_region.x == 1581
                && candidate.full_region.y == 1541
                && candidate.full_region.w == 687
                && candidate.full_region.h == 264
        }));
    }

    fn empty_chat_state() -> EverQuestChatInputState {
        EverQuestChatInputState {
            row_kind: CHAT_STATE_ROW_KIND.to_owned(),
            visible: true,
            text_present: false,
            confidence: CHAT_INPUT_EMPTY_CONFIDENCE,
            decision: "allow_empty_chat_input".to_owned(),
            denial_reason: None,
            source_region: Some(Rect {
                x: 1,
                y: 2,
                w: 100,
                h: 30,
            }),
            source_mode: "unit_test".to_owned(),
            text_len_estimate: 0,
            word_count: 0,
            ocr_status: "ocr_no_text".to_owned(),
            ocr_confidence: 0.0,
            foreground: EverQuestChatInputForeground {
                is_everquest_foreground: true,
                hwnd: 1,
                process_name: "eqgame.exe".to_owned(),
                window_title: "EverQuest".to_owned(),
                window_bounds: Rect {
                    x: 0,
                    y: 0,
                    w: 1024,
                    h: 768,
                },
                profile_id: Some(EVERQUEST_PROFILE_ID.to_owned()),
            },
            layout: None,
            source_refs: Vec::new(),
        }
    }
}
