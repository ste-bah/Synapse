use super::{
    ActAimParams, ActAimResponse, ActClickParams, ActClickResponse, ActClipboardParams,
    ActClipboardResponse, ActClipboardVerb, ActDragParams, ActDragResponse, ActPadParams,
    ActPadResponse, ActPressParams, ActPressResponse, ActScrollParams, ActScrollResponse,
    ActTypeParams, ActTypeResponse, ErrorData, Json, Parameters, ReleaseAllParams,
    ReleaseAllResponse, SynapseService, act_aim_with_handle, act_click_with_handle, act_clipboard,
    act_drag_with_handle, act_pad_with_handle, act_press_with_handle, act_scroll_with_handle,
    act_type_with_handle, release_all_with_handles, tool, tool_router,
};

#[tool_router(router = m2_tool_router, vis = "pub(super)")]
impl SynapseService {
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
        if let Err(error) = self.ensure_supported_use_allows_action("act_click") {
            self.audit_action_denied("act_click", &error);
            return Err(error);
        }
        self.audit_action_started("act_click")?;
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        let result = act_click_with_handle(handle, recording, params.0).await;
        self.audit_action_result("act_click", &result)?;
        result.map(Json)
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
        if let Err(error) = self.ensure_supported_use_allows_action("act_type") {
            self.audit_action_denied("act_type", &error);
            return Err(error);
        }
        self.audit_action_started("act_type")?;
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        if let Err(error) = self.ensure_act_type_foreground(recording.as_ref()) {
            let result: Result<ActTypeResponse, ErrorData> = Err(error);
            self.audit_action_result("act_type", &result)?;
            return result.map(Json);
        }
        let result = act_type_with_handle(handle, recording, params.0).await;
        self.audit_action_result("act_type", &result)?;
        result.map(Json)
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
        super::context::maybe_force_panic_during_act("act_press");
        if let Err(error) = self.ensure_supported_use_allows_action("act_press") {
            self.audit_action_denied("act_press", &error);
            return Err(error);
        }
        self.audit_action_started("act_press")?;
        let (handle, recording, connection_closed_cancel) = self.m2_action_context()?;
        let result =
            act_press_with_handle(handle, recording, connection_closed_cancel, params.0).await;
        self.audit_action_result("act_press", &result)?;
        result.map(Json)
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
        if let Err(error) = self.ensure_supported_use_allows_action("act_aim") {
            self.audit_action_denied("act_aim", &error);
            return Err(error);
        }
        self.audit_action_started("act_aim")?;
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        let result = act_aim_with_handle(handle, recording, params.0).await;
        self.audit_action_result("act_aim", &result)?;
        result.map(Json)
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
        if let Err(error) = self.ensure_supported_use_allows_action("act_drag") {
            self.audit_action_denied("act_drag", &error);
            return Err(error);
        }
        self.audit_action_started("act_drag")?;
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        let result = act_drag_with_handle(handle, recording, params.0).await;
        self.audit_action_result("act_drag", &result)?;
        result.map(Json)
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
        if let Err(error) = self.ensure_supported_use_allows_action("act_scroll") {
            self.audit_action_denied("act_scroll", &error);
            return Err(error);
        }
        self.audit_action_started("act_scroll")?;
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        let result = act_scroll_with_handle(handle, recording, params.0).await;
        self.audit_action_result("act_scroll", &result)?;
        result.map(Json)
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
        if let Err(error) = self.ensure_supported_use_allows_action("act_pad") {
            self.audit_action_denied("act_pad", &error);
            return Err(error);
        }
        self.audit_action_started("act_pad")?;
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        let result = act_pad_with_handle(handle, recording, params.0).await;
        self.audit_action_result("act_pad", &result)?;
        result.map(Json)
    }

    #[tool(description = "Read, write, or clear the system clipboard")]
    pub async fn act_clipboard(
        &self,
        params: Parameters<ActClipboardParams>,
    ) -> Result<Json<ActClipboardResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_clipboard",
            "tool.invocation kind=act_clipboard"
        );
        if matches!(
            params.verb,
            ActClipboardVerb::Write | ActClipboardVerb::Clear
        ) && let Err(error) = self.ensure_supported_use_allows_action("act_clipboard")
        {
            self.audit_action_denied("act_clipboard", &error);
            return Err(error);
        }
        act_clipboard(params).await.map(Json)
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
        let result = release_all_with_handles(handle, snapshot_handle, params.0).await;
        self.audit_action_result_best_effort("release_all", &result);
        result.map(Json)
    }
}
