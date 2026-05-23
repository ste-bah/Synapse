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
use synapse_action::RecordingBackend;
use synapse_core::Health;

use crate::{
    m1::{
        FindParams, FindResponse, M1State, ObserveParams, ReadTextParams, SetCaptureTargetParams,
        SetCaptureTargetResponse, SetPerceptionModeParams, SetPerceptionModeResponse,
        SharedM1State, assemble_observation, empty_input_schema, find_in_state, mcp_error,
        read_text_in_state, set_capture_target_in_state, set_perception_mode_in_state,
    },
    m2::{
        ActAimParams, ActAimResponse, ActClickParams, ActClickResponse, ActDragParams,
        ActDragResponse, ActPressParams, ActPressResponse, ActTypeParams, ActTypeResponse,
        SharedM2State, act_aim_with_handle, act_click_with_handle, act_drag_with_handle,
        act_press_with_handle, act_type_with_handle, shared_m2_state_from_env,
    },
};

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

    fn m2_action_handle(&self) -> Result<synapse_action::ActionHandle, ErrorData> {
        self.m2_state
            .lock()
            .map(|state| state.emitter_handle.clone())
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::OBSERVE_INTERNAL,
                    "M2 service state lock poisoned",
                )
            })
    }

    fn m2_action_context(
        &self,
    ) -> Result<(synapse_action::ActionHandle, Option<Arc<RecordingBackend>>), ErrorData> {
        self.m2_state
            .lock()
            .map(|state| (state.emitter_handle.clone(), state.recording.clone()))
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::OBSERVE_INTERNAL,
                    "M2 service state lock poisoned",
                )
            })
    }
}

impl Default for SynapseService {
    fn default() -> Self {
        Self::new()
    }
}

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
        let state = self.m1_state()?;
        assemble_observation(&state, &params.0).map(Json)
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
        let handle = self.m2_action_handle()?;
        act_click_with_handle(handle, params.0).await.map(Json)
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
        let (handle, recording) = self.m2_action_context()?;
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
        let (handle, recording) = self.m2_action_context()?;
        act_press_with_handle(handle, recording, params.0)
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
        let (handle, recording) = self.m2_action_context()?;
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
        let (handle, recording) = self.m2_action_context()?;
        act_drag_with_handle(handle, recording, params.0)
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
