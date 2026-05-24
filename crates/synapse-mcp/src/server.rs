use std::{
    collections::BTreeMap,
    sync::{Arc, MutexGuard},
    time::Instant,
};

use rmcp::{
    ErrorData, ServerHandler,
    handler::server::{
        router::tool::ToolRouter,
        wrapper::{Json, Parameters},
    },
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use synapse_action::{ActionStateSnapshot, RecordingBackend};
use synapse_core::{ForegroundContext, Health, error_codes};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{
    m1::{
        FindParams, FindResponse, M1State, ObserveParams, ReadTextParams, SetCaptureTargetParams,
        SetCaptureTargetResponse, SetPerceptionModeParams, SetPerceptionModeResponse,
        SharedM1State, assemble_observation, empty_input_schema, find_in_state, mcp_error,
        read_text_in_state, set_capture_target_in_state, set_perception_mode_in_state,
    },
    m2::{
        ActAimParams, ActAimResponse, ActClickParams, ActClickResponse, ActClipboardParams,
        ActClipboardResponse, ActDragParams, ActDragResponse, ActPadParams, ActPadResponse,
        ActPressParams, ActPressResponse, ActScrollParams, ActScrollResponse, ActTypeParams,
        ActTypeResponse, ReleaseAllParams, ReleaseAllResponse, SharedM2State, act_aim_with_handle,
        act_click_with_handle, act_clipboard, act_drag_with_handle, act_pad_with_handle,
        act_press_with_handle, act_scroll_with_handle, act_type_with_handle,
        release_all_with_handles, shared_m2_state_from_env,
        shared_m2_state_from_env_with_shutdown_reason,
    },
};

type M2ActionContext = (
    synapse_action::ActionHandle,
    Option<Arc<RecordingBackend>>,
    Option<CancellationToken>,
);

#[derive(Debug, Clone)]
pub struct SynapseService {
    started_at: Instant,
    tool_router: ToolRouter<Self>,
    m1_state: SharedM1State,
    m2_state: SharedM2State,
}

impl SynapseService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            tool_router: Self::tool_router(),
            m1_state: SharedM1State::default(),
            m2_state: shared_m2_state_from_env(),
        }
    }

    #[must_use]
    pub fn with_m2_shutdown_reason(
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: CancellationToken,
    ) -> Self {
        Self {
            started_at: Instant::now(),
            tool_router: Self::tool_router(),
            m1_state: SharedM1State::default(),
            m2_state: shared_m2_state_from_env_with_shutdown_reason(
                shutdown_cancel,
                shutdown_reason,
                Some(connection_closed_cancel),
            ),
        }
    }

    pub fn m2_emitter_done_receiver(&self) -> Option<watch::Receiver<Option<ActionStateSnapshot>>> {
        self.m2_state
            .lock()
            .ok()
            .and_then(|state| state.emitter_done_receiver())
    }

    fn health_payload(&self) -> Health {
        Health {
            ok: true,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            build: option_env!("VERGEN_GIT_SHA").unwrap_or("dev").to_owned(),
            uptime_s: self.started_at.elapsed().as_secs(),
            subsystems: BTreeMap::new(),
        }
    }

    fn m1_state(&self) -> Result<MutexGuard<'_, M1State>, ErrorData> {
        self.m1_state.lock().map_err(|_err| {
            mcp_error(
                synapse_core::error_codes::OBSERVE_INTERNAL,
                "M1 service state lock poisoned",
            )
        })
    }

    fn instructions(&self) -> &'static str {
        if self
            .m2_state
            .lock()
            .is_ok_and(|state| state.recording_enabled())
        {
            "Synapse M1 perception MCP server with M2 action scaffold (recording enabled)"
        } else {
            "Synapse M1 perception MCP server with M2 action scaffold"
        }
    }

    fn m2_action_context(&self) -> Result<M2ActionContext, ErrorData> {
        self.m2_state
            .lock()
            .map(|state| {
                (
                    state.emitter_handle.clone(),
                    state.recording.clone(),
                    state.connection_closed_cancel.clone(),
                )
            })
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::OBSERVE_INTERNAL,
                    "M2 service state lock poisoned",
                )
            })
    }

    fn m2_release_all_context(
        &self,
    ) -> Result<
        (
            synapse_action::ActionHandle,
            synapse_action::ActionEmitterSnapshotHandle,
        ),
        ErrorData,
    > {
        self.m2_state
            .lock()
            .map(|state| (state.emitter_handle.clone(), state.snapshot_handle.clone()))
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::OBSERVE_INTERNAL,
                    "M2 service state lock poisoned",
                )
            })
    }

    fn last_observed_foreground(&self) -> Result<Option<ForegroundContext>, ErrorData> {
        self.m1_state
            .lock()
            .map(|state| state.last_observed_foreground.clone())
            .map_err(|_err| {
                mcp_error(
                    error_codes::OBSERVE_INTERNAL,
                    "M1 service state lock poisoned",
                )
            })
    }

    fn ensure_act_type_foreground(
        &self,
        recording: Option<&Arc<RecordingBackend>>,
    ) -> Result<(), ErrorData> {
        let Some(expected) = self.last_observed_foreground()? else {
            return Ok(());
        };
        let actual = synapse_a11y::current_foreground_context().map_err(|error| {
            mcp_error(
                error_codes::ACTION_FOREGROUND_LOST,
                format!(
                    "act_type could not read current foreground for expected hwnd 0x{:x}: {error}",
                    expected.hwnd
                ),
            )
        })?;
        if actual.hwnd == expected.hwnd {
            return Ok(());
        }

        let recording_event_count_before =
            recording.map_or(0, |recording| recording.events().len());
        let recording_event_count_after = recording.map_or(0, |recording| recording.events().len());
        tracing::warn!(
            code = "M2_ACT_TYPE_FOREGROUND_LOST",
            expected_hwnd = expected.hwnd,
            actual_hwnd = actual.hwnd,
            expected_pid = expected.pid,
            actual_pid = actual.pid,
            expected_title = %expected.window_title,
            actual_title = %actual.window_title,
            recording_event_count_before,
            recording_event_count_after,
            "source_of_truth=foreground edge=lost before_hwnd=0x{:x} after_hwnd=0x{:x} code=ACTION_FOREGROUND_LOST recording_events_before={} recording_events_after={}",
            expected.hwnd,
            actual.hwnd,
            recording_event_count_before,
            recording_event_count_after
        );
        Err(mcp_error(
            error_codes::ACTION_FOREGROUND_LOST,
            format!(
                "act_type expected foreground hwnd 0x{:x} ({}) but current foreground is hwnd 0x{:x} ({})",
                expected.hwnd, expected.window_title, actual.hwnd, actual.window_title
            ),
        ))
    }
}

impl Default for SynapseService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(debug_assertions)]
fn maybe_force_panic_during_act(tool: &'static str) {
    if std::env::var("SYNAPSE_MCP_FORCE_PANIC_DURING_ACT").as_deref() == Ok("1") {
        tokio::task::block_in_place(|| panic!("forced panic during {tool}"));
    }
}

#[cfg(not(debug_assertions))]
fn maybe_force_panic_during_act(_tool: &'static str) {}

#[tool_router(router = tool_router)]
impl SynapseService {
    #[tool(description = "Return server health", input_schema = empty_input_schema())]
    pub async fn health(&self) -> Json<Health> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "health",
            "tool.invocation kind=health"
        );
        Json(self.health_payload())
    }

    #[tool(description = "Returns structured state of the focused window and surrounding context")]
    pub async fn observe(
        &self,
        params: Parameters<ObserveParams>,
    ) -> Result<Json<synapse_core::Observation>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "observe",
            "tool.invocation kind=observe"
        );
        let mut state = self.m1_state()?;
        let observation = assemble_observation(&state, &params.0)?;
        state.last_observed_foreground = Some(observation.foreground.clone());
        drop(state);
        Ok(Json(observation))
    }

    #[tool(description = "Search visible accessibility nodes and detected entities")]
    pub async fn find(
        &self,
        params: Parameters<FindParams>,
    ) -> Result<Json<FindResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "find",
            "tool.invocation kind=find"
        );
        let state = self.m1_state()?;
        find_in_state(&state, &params.0).map(Json)
    }

    #[tool(description = "OCR text from a screen region or visible element")]
    pub async fn read_text(
        &self,
        params: Parameters<ReadTextParams>,
    ) -> Result<Json<synapse_core::OcrResult>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "read_text",
            "tool.invocation kind=read_text"
        );
        let state = self.m1_state()?;
        read_text_in_state(&state, params.0).map(Json)
    }

    #[tool(description = "Set the active capture target")]
    pub async fn set_capture_target(
        &self,
        params: Parameters<SetCaptureTargetParams>,
    ) -> Result<Json<SetCaptureTargetResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "set_capture_target",
            "tool.invocation kind=set_capture_target"
        );
        let mut state = self.m1_state()?;
        set_capture_target_in_state(&mut state, params.0).map(Json)
    }

    #[tool(description = "Set the active perception mode")]
    pub async fn set_perception_mode(
        &self,
        params: Parameters<SetPerceptionModeParams>,
    ) -> Result<Json<SetPerceptionModeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "set_perception_mode",
            "tool.invocation kind=set_perception_mode"
        );
        let mut state = self.m1_state()?;
        set_perception_mode_in_state(&mut state, &params.0).map(Json)
    }

    #[tool(description = "Click a screen coordinate or UI Automation element")]
    pub async fn act_click(
        &self,
        params: Parameters<ActClickParams>,
    ) -> Result<Json<ActClickResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_click",
            "tool.invocation kind=act_click"
        );
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        act_click_with_handle(handle, recording, params.0)
            .await
            .map(Json)
    }

    #[tool(description = "Type text through the active keyboard backend")]
    pub async fn act_type(
        &self,
        params: Parameters<ActTypeParams>,
    ) -> Result<Json<ActTypeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_type",
            "tool.invocation kind=act_type"
        );
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        self.ensure_act_type_foreground(recording.as_ref())?;
        act_type_with_handle(handle, recording, params.0)
            .await
            .map(Json)
    }

    #[tool(description = "Press a keyboard key or ordered chord")]
    pub async fn act_press(
        &self,
        params: Parameters<ActPressParams>,
    ) -> Result<Json<ActPressResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_press",
            "tool.invocation kind=act_press"
        );
        maybe_force_panic_during_act("act_press");
        let (handle, recording, connection_closed_cancel) = self.m2_action_context()?;
        act_press_with_handle(handle, recording, connection_closed_cancel, params.0)
            .await
            .map(Json)
    }

    #[tool(description = "Move the pointer toward a screen, element, or track target")]
    pub async fn act_aim(
        &self,
        params: Parameters<ActAimParams>,
    ) -> Result<Json<ActAimResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_aim",
            "tool.invocation kind=act_aim"
        );
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        act_aim_with_handle(handle, recording, params.0)
            .await
            .map(Json)
    }

    #[tool(description = "Drag between screen coordinates or element centers")]
    pub async fn act_drag(
        &self,
        params: Parameters<ActDragParams>,
    ) -> Result<Json<ActDragResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_drag",
            "tool.invocation kind=act_drag"
        );
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        act_drag_with_handle(handle, recording, params.0)
            .await
            .map(Json)
    }

    #[tool(
        description = "Scroll vertically or horizontally at the current pointer or screen point"
    )]
    pub async fn act_scroll(
        &self,
        params: Parameters<ActScrollParams>,
    ) -> Result<Json<ActScrollResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_scroll",
            "tool.invocation kind=act_scroll"
        );
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        act_scroll_with_handle(handle, recording, params.0)
            .await
            .map(Json)
    }

    #[tool(description = "Apply a virtual gamepad report and optionally return it to neutral")]
    pub async fn act_pad(
        &self,
        params: Parameters<ActPadParams>,
    ) -> Result<Json<ActPadResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_pad",
            "tool.invocation kind=act_pad"
        );
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        act_pad_with_handle(handle, recording, params.0)
            .await
            .map(Json)
    }

    #[tool(description = "Read, write, or clear the system clipboard")]
    pub async fn act_clipboard(
        &self,
        params: Parameters<ActClipboardParams>,
    ) -> Result<Json<ActClipboardResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_clipboard",
            "tool.invocation kind=act_clipboard"
        );
        act_clipboard(params.0).await.map(Json)
    }

    #[tool(description = "Release all held keyboard, mouse, and gamepad input state")]
    pub async fn release_all(
        &self,
        params: Parameters<ReleaseAllParams>,
    ) -> Result<Json<ReleaseAllResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "release_all",
            "tool.invocation kind=release_all"
        );
        let (handle, snapshot_handle) = self.m2_release_all_context()?;
        release_all_with_handles(handle, snapshot_handle, params.0)
            .await
            .map(Json)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SynapseService {
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResult, ErrorData> {
        let tool_name = request.name.to_string();
        let context = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        match self.tool_router.call(context).await {
            Ok(result) => Ok(result),
            Err(error) if error.data.is_none() && error.message == "tool not found" => {
                Err(mcp_error(
                    synapse_core::error_codes::TOOL_NOT_FOUND,
                    format!("tool not found: {tool_name}"),
                ))
            }
            Err(error)
                if error.data.is_none() && error.code == rmcp::model::ErrorCode::INVALID_PARAMS =>
            {
                Err(mcp_error(
                    synapse_core::error_codes::TOOL_PARAMS_INVALID,
                    error.message.to_string(),
                ))
            }
            Err(error) => Err(error),
        }
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "synapse-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(self.instructions())
    }
}

#[cfg(test)]
mod tests {
    use super::SynapseService;

    #[test]
    fn health_payload_is_m0_hardcoded() {
        let service = SynapseService::new();
        let payload = service.health_payload();
        assert!(payload.ok);
        assert_eq!(payload.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(payload.build, "dev");
        assert!(payload.subsystems.is_empty());
    }

    #[test]
    fn uptime_uses_monotonic_elapsed() {
        let service = SynapseService::new();
        let first = service.health_payload().uptime_s;
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = service.health_payload().uptime_s;
        assert!(second >= first);
    }
}
