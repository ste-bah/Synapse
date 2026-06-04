use super::{
    ActClickParams, ActClickResponse, ActClipboardParams, ActClipboardResponse, ActClipboardVerb,
    ActKeymapParams, ActKeymapResponse, ActPadParams, ActPadResponse, ActPressParams,
    ActPressResponse, ActScrollParams, ActScrollResponse, ActStrokeParams, ActStrokeResponse,
    ActTypeParams, ActTypeResponse, ErrorData, Json, Parameters, ReleaseAllParams,
    ReleaseAllResponse, SynapseService, act_click_with_handle, act_clipboard,
    act_keymap_with_handle, act_pad_with_handle, act_press_with_handle, act_scroll_with_handle,
    act_stroke_with_handle, act_type_with_handle,
    action_preflight::{ActionPreflightReadback, ForegroundProof},
    release_all_with_handles, tool, tool_router, validate_act_stroke_params,
};
use crate::m1::mcp_error;
use crate::m2::{act_stroke_error_details, act_stroke_request_details};
use rmcp::model::ErrorCode;
use serde_json::{Value, json};
use synapse_core::{Backend, ForegroundContext, error_codes};
use tokio_util::sync::CancellationToken;

const ACT_STROKE_FOREGROUND_MONITOR_INTERVAL_MS: u64 = 10;

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

    #[tool(
        description = "Move, aim, or drag to a point/element target or along an explicit spatial path using timed continuous mouse samples; button unset moves/aims and button set drags; motion_model defaults to path and can use wind_mouse for point-to-point line strokes"
    )]
    pub async fn act_stroke(
        &self,
        params: Parameters<ActStrokeParams>,
    ) -> Result<Json<ActStrokeResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_stroke",
            "tool.invocation kind=act_stroke"
        );
        let plan = validate_act_stroke_params(&params)?;
        let stroke_details = act_stroke_request_details(&params, &plan);
        let preflight = match self.ensure_supported_use_allows_action("act_stroke") {
            Ok(preflight) => preflight,
            Err(error) => {
                self.audit_action_denied_with_details(
                    "act_stroke",
                    &error,
                    &json!({
                        "stroke": stroke_details,
                        "failure": act_stroke_error_details(&error),
                    }),
                );
                return Err(error);
            }
        };
        self.audit_action_started_with_details(
            "act_stroke",
            &act_stroke_audit_details(&stroke_details, &preflight),
        )?;
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        let foreground_monitor = recording
            .is_none()
            .then(|| self.start_act_stroke_foreground_monitor(&preflight));
        let result = act_stroke_with_handle(handle, recording, params.clone(), plan.clone()).await;
        let foreground_error = await_act_stroke_foreground_monitor(foreground_monitor).await;
        let result = match foreground_error {
            Some(error) => Err(error),
            None => result,
        };
        match &result {
            Ok(response) => {
                self.audit_action_ok_with_details(
                    "act_stroke",
                    &json!({
                        "response": response,
                        "stroke": stroke_details,
                        "preflight": preflight,
                    }),
                )?;
            }
            Err(error) => {
                let failure_details =
                    act_stroke_failure_audit_details(&stroke_details, &preflight, error);
                log_act_stroke_failure(&failure_details, error);
                self.audit_action_error_with_details("act_stroke", error, &failure_details)?;
            }
        }
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

fn act_stroke_audit_details(stroke_details: &Value, preflight: &ActionPreflightReadback) -> Value {
    json!({
        "stroke": stroke_details,
        "preflight": preflight,
    })
}

fn act_stroke_failure_audit_details(
    stroke_details: &Value,
    preflight: &ActionPreflightReadback,
    error: &ErrorData,
) -> Value {
    json!({
        "stroke": stroke_details,
        "preflight": preflight,
        "failure": act_stroke_error_details(error),
    })
}

fn log_act_stroke_failure(details: &Value, error: &ErrorData) {
    let stroke = details.get("stroke").unwrap_or(&Value::Null);
    let failure = details.get("failure").unwrap_or(&Value::Null);
    let preflight = details.get("preflight").unwrap_or(&Value::Null);
    let error_code = failure
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    let path_id = stroke
        .get("path_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let path_kind = stroke
        .get("path_kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let backend_requested = stroke
        .get("backend_requested")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let backend_resolved = stroke
        .get("backend_resolved")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let point_index = failure.get("point_index").cloned().unwrap_or(Value::Null);
    let queue_rate_state = failure
        .get("queue_rate_state")
        .cloned()
        .unwrap_or_else(|| json!({ "kind": "not_rate_or_queue" }));
    let foreground_proof = preflight
        .get("after")
        .or_else(|| preflight.get("before"))
        .cloned()
        .unwrap_or(Value::Null);

    tracing::error!(
        code = error_code,
        detail = %error.message,
        path_id,
        path_kind,
        backend_requested,
        backend_resolved,
        point_index = ?point_index,
        queue_rate_state = ?queue_rate_state,
        foreground_proof = ?foreground_proof,
        fallback_path_executed = false,
        action_kind = "act_stroke",
        "act_stroke failed without fallback"
    );
}

impl SynapseService {
    fn start_act_stroke_foreground_monitor(
        &self,
        preflight: &ActionPreflightReadback,
    ) -> ActStrokeForegroundMonitor {
        let cancel = CancellationToken::new();
        let expected = preflight
            .after
            .clone()
            .unwrap_or_else(|| preflight.before.clone());
        let task = tokio::spawn(monitor_act_stroke_foreground(
            self.clone(),
            expected,
            cancel.clone(),
        ));
        ActStrokeForegroundMonitor { cancel, task }
    }
}

struct ActStrokeForegroundMonitor {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<Option<ErrorData>>,
}

async fn await_act_stroke_foreground_monitor(
    monitor: Option<ActStrokeForegroundMonitor>,
) -> Option<ErrorData> {
    let Some(monitor) = monitor else {
        return None;
    };
    monitor.cancel.cancel();
    match monitor.task.await {
        Ok(error) => error,
        Err(error) => Some(mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("act_stroke foreground monitor join failed: {error}"),
        )),
    }
}

async fn monitor_act_stroke_foreground(
    service: SynapseService,
    expected: ForegroundProof,
    cancel: CancellationToken,
) -> Option<ErrorData> {
    loop {
        tokio::select! {
            () = cancel.cancelled() => return None,
            () = tokio::time::sleep(std::time::Duration::from_millis(ACT_STROKE_FOREGROUND_MONITOR_INTERVAL_MS)) => {}
        }

        match service.current_audit_foreground() {
            Ok(actual) if actual.hwnd == expected.hwnd => {}
            Ok(actual) => {
                synapse_action::request_release_interrupt();
                tracing::error!(
                    code = error_codes::ACTION_FOREGROUND_LOST,
                    expected_hwnd = expected.hwnd,
                    actual_hwnd = actual.hwnd,
                    expected_pid = expected.pid,
                    actual_pid = actual.pid,
                    expected_process_name = %expected.process_name,
                    actual_process_name = %actual.process_name,
                    expected_window_title = %expected.window_title,
                    actual_window_title = %actual.window_title,
                    action_kind = "act_stroke",
                    "act_stroke foreground lost mid-stroke; release interrupt requested"
                );
                return Some(act_stroke_foreground_lost_error(
                    &expected,
                    Some(&actual),
                    None,
                ));
            }
            Err(error) => {
                synapse_action::request_release_interrupt();
                tracing::error!(
                    code = error_codes::ACTION_FOREGROUND_LOST,
                    expected_hwnd = expected.hwnd,
                    expected_pid = expected.pid,
                    expected_process_name = %expected.process_name,
                    expected_window_title = %expected.window_title,
                    read_error = %error.message,
                    action_kind = "act_stroke",
                    "act_stroke foreground read failed mid-stroke; release interrupt requested"
                );
                return Some(act_stroke_foreground_lost_error(
                    &expected,
                    None,
                    Some(&error),
                ));
            }
        }
    }
}

fn act_stroke_foreground_lost_error(
    expected: &ForegroundProof,
    actual: Option<&ForegroundContext>,
    read_error: Option<&ErrorData>,
) -> ErrorData {
    let detail = match actual {
        Some(actual) => format!(
            "act_stroke expected foreground hwnd 0x{:x} ({}) but current foreground is hwnd 0x{:x} ({})",
            expected.hwnd, expected.window_title, actual.hwnd, actual.window_title
        ),
        None => format!(
            "act_stroke could not read current foreground mid-stroke for expected hwnd 0x{:x} ({})",
            expected.hwnd, expected.window_title
        ),
    };
    ErrorData::new(
        ErrorCode(-32099),
        detail.clone(),
        Some(json!({
            "code": error_codes::ACTION_FOREGROUND_LOST,
            "reason": "act_stroke_foreground_lost_mid_stroke",
            "detail": detail,
            "point_index": Value::Null,
            "queue_rate_state": {
                "kind": "not_rate_or_queue",
            },
            "foreground_expected": expected,
            "foreground_actual": actual.map(foreground_context_details),
            "foreground_read_error": read_error.map(|error| json!({
                "message": error.message.to_string(),
                "data": error.data.clone(),
            })),
        })),
    )
}

fn foreground_context_details(foreground: &ForegroundContext) -> Value {
    json!({
        "hwnd": foreground.hwnd,
        "pid": foreground.pid,
        "process_name": &foreground.process_name,
        "process_path": &foreground.process_path,
        "window_title": &foreground.window_title,
        "monitor_index": foreground.monitor_index,
        "dpi_scale": foreground.dpi_scale,
        "profile_id": &foreground.profile_id,
        "steam_appid": foreground.steam_appid,
        "is_fullscreen": foreground.is_fullscreen,
        "is_dwm_composed": foreground.is_dwm_composed,
        "window_bounds": &foreground.window_bounds,
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

#[cfg(test)]
mod tests {
    use synapse_core::Rect;

    use super::*;

    #[test]
    fn stroke_foreground_lost_error_carries_specific_code_and_readbacks() {
        let expected = foreground_proof(100, 10, "notepad.exe", "before");
        let actual = foreground_context(200, 20, "calc.exe", "after");

        let error = act_stroke_foreground_lost_error(&expected, Some(&actual), None);
        let data = match error.data.as_ref() {
            Some(data) => data,
            None => panic!("foreground lost error should carry structured data"),
        };

        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some(error_codes::ACTION_FOREGROUND_LOST)
        );
        assert_eq!(
            data.pointer("/foreground_expected/hwnd")
                .and_then(Value::as_i64),
            Some(100)
        );
        assert_eq!(
            data.pointer("/foreground_actual/hwnd")
                .and_then(Value::as_i64),
            Some(200)
        );
        assert_eq!(
            data.pointer("/queue_rate_state/kind")
                .and_then(Value::as_str),
            Some("not_rate_or_queue")
        );
    }

    fn foreground_proof(
        hwnd: i64,
        pid: u32,
        process_name: &str,
        window_title: &str,
    ) -> ForegroundProof {
        ForegroundProof {
            hwnd,
            pid,
            process_name: process_name.to_owned(),
            process_path: format!(r"C:\test\{process_name}"),
            window_title: window_title.to_owned(),
            is_minimized: Some(false),
            minimized_readback_error: None,
            observed_profile_id: None,
        }
    }

    fn foreground_context(
        hwnd: i64,
        pid: u32,
        process_name: &str,
        window_title: &str,
    ) -> ForegroundContext {
        ForegroundContext {
            hwnd,
            pid,
            process_name: process_name.to_owned(),
            process_path: format!(r"C:\test\{process_name}"),
            window_title: window_title.to_owned(),
            window_bounds: Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            monitor_index: 0,
            dpi_scale: 1.0,
            profile_id: None,
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
        }
    }
}
