use super::{
    ActAimParams, ActAimResponse, ActClickParams, ActClickResponse, ActClipboardParams,
    ActClipboardResponse, ActClipboardVerb, ActDragParams, ActDragResponse, ActKeymapParams,
    ActKeymapResponse, ActPadParams, ActPadResponse, ActPressParams, ActPressResponse,
    ActScrollParams, ActScrollResponse, ActTypeParams, ActTypeResponse, ErrorData, Json,
    Parameters, ReleaseAllParams, ReleaseAllResponse, SynapseService, act_aim_with_handle,
    act_click_with_handle, act_clipboard, act_drag_with_handle, act_keymap_with_handle,
    act_pad_with_handle, act_press_with_handle, act_scroll_with_handle, act_type_with_handle,
    action_preflight::ActionPreflightReadback, release_all_with_handles, tool, tool_router,
};
use crate::m1::mcp_error;
use serde_json::{Value, json};
use synapse_core::{Backend, error_codes};

#[tool_router(router = m2_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(description = "Click a screen coordinate or UI Automation element")]
    pub async fn act_click(
        &self,
        params: Parameters<ActClickParams>,
    ) -> Result<Json<ActClickResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_click",
            "tool.invocation kind=act_click"
        );
        let preflight = match self.ensure_supported_use_allows_action("act_click") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied("act_click", &error);
                return Err(error);
            }
        };
        self.audit_action_started_with_details("act_click", &action_preflight_details(&preflight))?;
        if let Err(error) = ensure_everquest_click_backend(&params, &preflight) {
            let result: Result<ActClickResponse, ErrorData> = Err(error);
            self.audit_action_result("act_click", &result)?;
            return result.map(Json);
        }
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        let result = act_click_with_handle(handle, recording, params).await;
        self.audit_action_result("act_click", &result)?;
        result.map(Json)
    }

    #[tool(description = "Type text through the active keyboard backend")]
    pub async fn act_type(
        &self,
        params: Parameters<ActTypeParams>,
    ) -> Result<Json<ActTypeResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_type",
            "tool.invocation kind=act_type"
        );
        let preflight = match self.ensure_supported_use_allows_action("act_type") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied("act_type", &error);
                return Err(error);
            }
        };
        self.audit_action_started_with_details("act_type", &action_preflight_details(&preflight))?;
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        if params.into_element.is_none()
            && let Err(error) = self.ensure_act_type_foreground(recording.as_ref())
        {
            let result: Result<ActTypeResponse, ErrorData> = Err(error);
            self.audit_action_result("act_type", &result)?;
            return result.map(Json);
        }
        let result = act_type_with_handle(handle, recording, params).await;
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
        let preflight = match self.ensure_supported_use_allows_action("act_press") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied("act_press", &error);
                return Err(error);
            }
        };
        self.audit_action_started_with_details("act_press", &action_preflight_details(&preflight))?;
        let (handle, recording, connection_closed_cancel) = self.m2_action_context()?;
        let result =
            act_press_with_handle(handle, recording, connection_closed_cancel, params.0).await;
        self.audit_action_result("act_press", &result)?;
        result.map(Json)
    }

    #[tool(description = "Press a keyboard alias from the active profile keymap")]
    pub async fn act_keymap(
        &self,
        params: Parameters<ActKeymapParams>,
    ) -> Result<Json<ActKeymapResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_keymap",
            alias = %params.alias,
            "tool.invocation kind=act_keymap"
        );
        let request_details = json!({
            "alias": params.alias.trim(),
            "hold_ms": params.hold_ms,
            "backend": params.backend,
        });
        let preflight = match self.ensure_supported_use_allows_action("act_keymap") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied_with_details("act_keymap", &error, &request_details);
                return Err(error);
            }
        };
        self.audit_action_started_with_details(
            "act_keymap",
            &json!({
                "request": request_details,
                "preflight": preflight,
            }),
        )?;
        let profile = {
            let runtime = self.profile_runtime()?;
            let active_profile_id = runtime
                .active_profile_id()
                .map_err(|error| mcp_error(error.code(), error.to_string()))?
                .ok_or_else(|| {
                    mcp_error(
                        error_codes::PROFILE_NOT_FOUND,
                        "act_keymap requires an active profile",
                    )
                })?;
            runtime
                .profile(&active_profile_id)
                .map_err(|error| mcp_error(error.code(), error.to_string()))?
                .ok_or_else(|| {
                    mcp_error(
                        error_codes::PROFILE_NOT_FOUND,
                        format!("active profile {active_profile_id} was not found"),
                    )
                })?
        };
        let (handle, recording, connection_closed_cancel) = self.m2_action_context()?;
        let result = act_keymap_with_handle(
            handle,
            recording,
            connection_closed_cancel,
            &profile,
            params,
        )
        .await;
        self.audit_action_result("act_keymap", &result)?;
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
        let preflight = match self.ensure_supported_use_allows_action("act_aim") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied("act_aim", &error);
                return Err(error);
            }
        };
        self.audit_action_started_with_details("act_aim", &action_preflight_details(&preflight))?;
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
        let preflight = match self.ensure_supported_use_allows_action("act_drag") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied("act_drag", &error);
                return Err(error);
            }
        };
        self.audit_action_started_with_details("act_drag", &action_preflight_details(&preflight))?;
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
        let preflight = match self.ensure_supported_use_allows_action("act_scroll") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied("act_scroll", &error);
                return Err(error);
            }
        };
        self.audit_action_started_with_details(
            "act_scroll",
            &action_preflight_details(&preflight),
        )?;
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
        let preflight = match self.ensure_supported_use_allows_action("act_pad") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied("act_pad", &error);
                return Err(error);
            }
        };
        self.audit_action_started_with_details("act_pad", &action_preflight_details(&preflight))?;
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
        let request_details = json!({
            "verb": params.verb,
            "format": params.format,
            "text_len": params.text.as_ref().map(|text| text.chars().count()),
        });
        if matches!(
            params.verb,
            ActClipboardVerb::Write | ActClipboardVerb::Clear
        ) && let Err(error) = self.ensure_supported_use_allows_action("act_clipboard")
        {
            self.audit_action_denied_with_details("act_clipboard", &error, &request_details);
            return Err(error);
        }
        self.audit_action_started_with_details("act_clipboard", &request_details)?;
        let result = act_clipboard(params).await;
        match &result {
            Ok(response) => {
                self.audit_action_ok_with_details(
                    "act_clipboard",
                    &clipboard_response_audit_details(response),
                )?;
            }
            Err(error) => {
                self.audit_action_error_with_details("act_clipboard", error, &request_details)?;
            }
        }
        result.map(Json)
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
        let (handle, snapshot_handle, reflex_runtime) = self.m2_release_all_context()?;
        let result =
            release_all_with_handles(handle, snapshot_handle, reflex_runtime, params.0).await;
        self.audit_action_result_best_effort("release_all", &result);
        result.map(Json)
    }
}

fn action_preflight_details(preflight: &ActionPreflightReadback) -> Value {
    json!({
        "preflight": preflight,
    })
}

fn clipboard_response_audit_details(response: &ActClipboardResponse) -> Value {
    json!({
        "response": {
            "ok": response.ok,
            "verb": response.verb,
            "format": response.format,
            "written": response.written,
            "cleared": response.cleared,
            "text_len": response.text_len,
            "text_present": response.text.is_some(),
            "elapsed_ms": response.elapsed_ms,
        },
    })
}

fn ensure_everquest_click_backend(
    params: &ActClickParams,
    preflight: &ActionPreflightReadback,
) -> Result<(), ErrorData> {
    if preflight.target_profile_id.as_deref() == Some("everquest.live")
        && params.backend == Backend::Software
    {
        return Err(mcp_error(
            error_codes::ACTION_BACKEND_UNAVAILABLE,
            "everquest.live software mouse clicks are not FSV-accepted; use backend=hardware through the configured HID path or a keyboard keymap equivalent",
        ));
    }
    Ok(())
}
