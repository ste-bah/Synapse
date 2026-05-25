use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
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
use synapse_core::{ForegroundContext, Health, SubsystemHealth, error_codes};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{
    http::sse::SseState,
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
    m3::{
        SharedM3State,
        profile::{
            ProfileActivateParams, ProfileActivateResponse, ProfileListParams, ProfileListResponse,
            activate_profile, list_profiles,
        },
        reflex::{ReflexRegisterParams, ReflexRegisterResponse, register_reflex},
        shared_m3_state_from_env, shared_m3_state_from_env_with_shutdown_reason,
        shared_m3_state_from_env_with_shutdown_reason_and_sse_state,
        subscribe::{
            SubscribeCancelParams, SubscribeCancelResponse, SubscribeParams, SubscribeResponse,
            cancel_subscription, subscribe_to_events,
        },
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
    m3_state: SharedM3State,
}

impl SynapseService {
    #[must_use]
    pub fn new() -> Self {
        match Self::try_new() {
            Ok(service) => service,
            Err(error) => panic!("M3 state should initialize from environment: {error:#}"),
        }
    }

    pub fn try_new() -> anyhow::Result<Self> {
        Ok(Self {
            started_at: Instant::now(),
            tool_router: Self::tool_router(),
            m1_state: SharedM1State::default(),
            m2_state: shared_m2_state_from_env(),
            m3_state: shared_m3_state_from_env()?,
        })
    }

    pub fn try_with_m2_shutdown_reason(
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: CancellationToken,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            started_at: Instant::now(),
            tool_router: Self::tool_router(),
            m1_state: SharedM1State::default(),
            m2_state: shared_m2_state_from_env_with_shutdown_reason(
                shutdown_cancel.clone(),
                shutdown_reason,
                Some(connection_closed_cancel.clone()),
            ),
            m3_state: shared_m3_state_from_env_with_shutdown_reason(
                shutdown_cancel,
                shutdown_reason,
                Some(connection_closed_cancel),
            )?,
        })
    }

    pub fn try_with_m2_shutdown_reason_and_sse_state(
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: CancellationToken,
        sse_state: SseState,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            started_at: Instant::now(),
            tool_router: Self::tool_router(),
            m1_state: SharedM1State::default(),
            m2_state: shared_m2_state_from_env_with_shutdown_reason(
                shutdown_cancel.clone(),
                shutdown_reason,
                Some(connection_closed_cancel.clone()),
            ),
            m3_state: shared_m3_state_from_env_with_shutdown_reason_and_sse_state(
                shutdown_cancel,
                shutdown_reason,
                Some(connection_closed_cancel),
                sse_state,
            )?,
        })
    }

    pub fn m2_emitter_done_receiver(&self) -> Option<watch::Receiver<Option<ActionStateSnapshot>>> {
        self.m2_state
            .lock()
            .ok()
            .and_then(|state| state.emitter_done_receiver())
    }

    pub(crate) fn health_payload(&self) -> Health {
        let mut subsystems = BTreeMap::new();
        subsystems.insert("profiles".to_owned(), self.profile_health());
        Health {
            ok: true,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            build: option_env!("VERGEN_GIT_SHA").unwrap_or("dev").to_owned(),
            uptime_s: self.started_at.elapsed().as_secs(),
            subsystems,
        }
    }

    fn profile_health(&self) -> SubsystemHealth {
        match self.m3_state.lock() {
            Ok(state) => state.profile_runtime.as_ref().map_or_else(
                || SubsystemHealth {
                    status: "not_initialized".to_owned(),
                    detail: Some(
                        "profile runtime initializes on first profile tool call".to_owned(),
                    ),
                    active_profile_id: None,
                },
                |runtime| match runtime.active_profile_id() {
                    Ok(active_profile_id) => SubsystemHealth {
                        status: "healthy".to_owned(),
                        detail: Some(format!("profile_dir={}", runtime.profile_dir().display())),
                        active_profile_id,
                    },
                    Err(error) => SubsystemHealth {
                        status: "error".to_owned(),
                        detail: Some(error.to_string()),
                        active_profile_id: None,
                    },
                },
            ),
            Err(_err) => SubsystemHealth {
                status: "error".to_owned(),
                detail: Some("M3 service state lock poisoned".to_owned()),
                active_profile_id: None,
            },
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
        let recording_enabled = self
            .m2_state
            .lock()
            .is_ok_and(|state| state.recording_enabled());
        let m3_stub_count = crate::m3::m3_tool_stubs().len();
        let m3_scaffold_ready = self.m3_state.lock().is_ok_and(|state| {
            let _state_readback = (
                state.db_path.as_ref(),
                state.profile_dir.as_ref(),
                state.reflex_disabled,
                state.bearer_token.as_ref(),
                state.shutdown_cancel.is_cancelled(),
                state.shutdown_reason,
                state
                    .connection_closed_cancel
                    .as_ref()
                    .map(CancellationToken::is_cancelled),
            );
            state.scaffold_ready() && m3_stub_count == 11
        });
        match (recording_enabled, m3_scaffold_ready) {
            (true, true) => {
                "Synapse M1 perception MCP server with M2 action scaffold and M3 scaffold (recording enabled)"
            }
            (false, true) => {
                "Synapse M1 perception MCP server with M2 action scaffold and M3 scaffold"
            }
            (true, false) => {
                "Synapse M1 perception MCP server with M2 action scaffold (recording enabled)"
            }
            (false, false) => "Synapse M1 perception MCP server with M2 action scaffold",
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

    fn profile_runtime(&self) -> Result<Arc<synapse_profiles::ProfileRuntime>, ErrorData> {
        self.m3_state
            .lock()
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    "M3 service state lock poisoned",
                )
            })?
            .ensure_profile_runtime()
            .map_err(|error| mcp_error(error.code(), error.to_string()))
    }

    fn sse_state(&self) -> Result<SseState, ErrorData> {
        self.m3_state
            .lock()
            .map(|state| state.sse_state.clone())
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    "M3 service state lock poisoned",
                )
            })
    }

    fn reflex_runtime(&self) -> Result<Arc<Mutex<synapse_reflex::ReflexRuntime>>, ErrorData> {
        let event_bus = self.sse_state()?.event_bus();
        let (action_handle, _recording, _connection_closed_cancel) = self.m2_action_context()?;
        self.m3_state
            .lock()
            .map_err(|_err| {
                mcp_error(
                    synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    "M3 service state lock poisoned",
                )
            })?
            .ensure_reflex_runtime(action_handle, event_bus)
            .map_err(|error| m3_state_error(&error))
    }

    #[allow(clippy::significant_drop_tightening)]
    fn activate_profile_locked(
        &self,
        params: &ProfileActivateParams,
    ) -> Result<ProfileActivateResponse, ErrorData> {
        // Keep the M3 mutex held so concurrent activations preserve changed=false idempotency.
        let mut state = self.m3_state.lock().map_err(|_err| {
            mcp_error(
                synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                "M3 service state lock poisoned",
            )
        })?;
        let runtime = state
            .ensure_profile_runtime()
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        activate_profile(&runtime, params)
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
            "readback=foreground edge=lost before_hwnd=0x{:x} after_hwnd=0x{:x} code=ACTION_FOREGROUND_LOST recording_events_before={} recording_events_after={}",
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

fn m3_state_error(error: &anyhow::Error) -> ErrorData {
    if let Some(reflex_error) = error.downcast_ref::<synapse_reflex::ReflexError>() {
        return mcp_error(reflex_error.code(), reflex_error.to_string());
    }
    mcp_error(
        synapse_core::error_codes::TOOL_INTERNAL_ERROR,
        error.to_string(),
    )
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

    #[tool(description = "Subscribe to filtered event notifications")]
    pub async fn subscribe(
        &self,
        params: Parameters<SubscribeParams>,
    ) -> Result<Json<SubscribeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "subscribe",
            kinds_count = params.0.kinds.len(),
            snapshot_first = params.0.snapshot_first,
            buffer_size = params.0.buffer_size,
            "tool.invocation kind=subscribe"
        );
        let sse_state = self.sse_state()?;
        subscribe_to_events(&sse_state, &params.0).map(Json)
    }

    #[tool(description = "Cancel an event subscription")]
    pub async fn subscribe_cancel(
        &self,
        params: Parameters<SubscribeCancelParams>,
    ) -> Result<Json<SubscribeCancelResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "subscribe_cancel",
            subscription_id = %params.0.subscription_id,
            "tool.invocation kind=subscribe_cancel"
        );
        let sse_state = self.sse_state()?;
        cancel_subscription(&sse_state, &params.0).map(Json)
    }

    #[tool(description = "Register a reflex")]
    pub async fn reflex_register(
        &self,
        params: Parameters<ReflexRegisterParams>,
    ) -> Result<Json<ReflexRegisterResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "reflex_register",
            reflex_kind = %params.0.kind,
            priority = params.0.priority,
            "tool.invocation kind=reflex_register"
        );
        let runtime = self.reflex_runtime()?;
        register_reflex(&runtime, params.0).map(Json)
    }

    #[tool(description = "List loaded profiles")]
    pub async fn profile_list(
        &self,
        params: Parameters<ProfileListParams>,
    ) -> Result<Json<ProfileListResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_list",
            "tool.invocation kind=profile_list"
        );
        let runtime = self.profile_runtime()?;
        list_profiles(&runtime, &params.0).map(Json)
    }

    #[tool(description = "Activate a loaded profile by id")]
    pub async fn profile_activate(
        &self,
        params: Parameters<ProfileActivateParams>,
    ) -> Result<Json<ProfileActivateResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_activate",
            profile_id = %params.0.profile_id,
            "tool.invocation kind=profile_activate"
        );
        self.activate_profile_locked(&params.0).map(Json)
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
    fn health_payload_reports_profile_subsystem_uninitialized() {
        let service = SynapseService::new();
        let payload = service.health_payload();
        assert!(payload.ok);
        assert_eq!(payload.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(payload.build, "dev");
        assert_eq!(payload.subsystems["profiles"].status, "not_initialized");
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
