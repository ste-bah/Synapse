use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use rmcp::{ErrorData, RoleServer, service::RequestContext};
use serde::Serialize;
use serde_json::{Value, json};

use super::SynapseService;
use crate::m1::mcp_error;

static ACTION_AUDIT_SEQ: AtomicU32 = AtomicU32::new(0);

impl SynapseService {
    pub(super) fn audit_action_started_for_request(
        &self,
        tool: &'static str,
        request_context: &RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let session_id = action_session_id_from_request_context(request_context)?;
        self.write_action_audit_row(tool, "started", None, &json!({}), session_id.as_deref())
    }

    pub(super) fn audit_action_started_with_details(
        &self,
        tool: &'static str,
        details: &Value,
    ) -> Result<(), ErrorData> {
        self.write_action_audit_row(tool, "started", None, details, None)
    }

    pub(super) fn audit_action_started_with_details_for_request(
        &self,
        tool: &'static str,
        details: &Value,
        request_context: &RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let session_id = action_session_id_from_request_context(request_context)?;
        self.write_action_audit_row(tool, "started", None, details, session_id.as_deref())
    }

    pub(super) fn audit_action_started_with_details_for_session(
        &self,
        tool: &'static str,
        details: &Value,
        session_id: &str,
    ) -> Result<(), ErrorData> {
        self.write_action_audit_row(tool, "started", None, details, Some(session_id))
    }

    pub(super) fn audit_action_denied(&self, tool: &'static str, error: &ErrorData) {
        self.audit_action_denied_with_details(tool, error, &json!({}));
    }

    pub(super) fn audit_action_denied_for_request(
        &self,
        tool: &'static str,
        error: &ErrorData,
        request_context: &RequestContext<RoleServer>,
    ) {
        self.audit_action_denied_with_details_for_request(tool, error, &json!({}), request_context);
    }

    pub(super) fn audit_action_denied_with_details(
        &self,
        tool: &'static str,
        error: &ErrorData,
        details: &Value,
    ) {
        if let Err(audit_error) = self.write_action_audit_row(
            tool,
            "denied",
            error_data_code(error),
            &json!({
                "message": error.message.to_string(),
                "data": error.data.clone(),
                "request": details,
            }),
            None,
        ) {
            tracing::warn!(
                code = "ACTION_AUDIT_WRITE_FAILED",
                tool,
                status = "denied",
                audit_error = %audit_error,
                "action audit write failed after denied action"
            );
        }
    }

    pub(super) fn audit_action_denied_with_details_for_request(
        &self,
        tool: &'static str,
        error: &ErrorData,
        details: &Value,
        request_context: &RequestContext<RoleServer>,
    ) {
        let session_id = match action_session_id_from_request_context(request_context) {
            Ok(session_id) => session_id,
            Err(session_error) => {
                tracing::warn!(
                    code = "ACTION_AUDIT_SESSION_ID_READ_FAILED",
                    tool,
                    source_error = %session_error.message,
                    source_error_data = ?session_error.data,
                    "action audit could not read request MCP session id for denied action"
                );
                None
            }
        };
        if let Err(audit_error) = self.write_action_audit_row(
            tool,
            "denied",
            error_data_code(error),
            &json!({
                "message": error.message.to_string(),
                "data": error.data.clone(),
                "request": details,
            }),
            session_id.as_deref(),
        ) {
            tracing::warn!(
                code = "ACTION_AUDIT_WRITE_FAILED",
                tool,
                status = "denied",
                audit_error = %audit_error,
                "action audit write failed after denied action"
            );
        }
    }

    pub(super) fn audit_action_result<T: Serialize>(
        &self,
        tool: &'static str,
        result: &Result<T, ErrorData>,
    ) -> Result<(), ErrorData> {
        match result {
            Ok(response) => self.write_action_audit_row(
                tool,
                "ok",
                None,
                &json!({
                    "response": response,
                }),
                None,
            ),
            Err(error) => self.write_action_audit_row(
                tool,
                "error",
                error_data_code(error),
                &json!({
                    "message": error.message.to_string(),
                    "data": error.data.clone(),
                }),
                None,
            ),
        }
    }

    pub(super) fn audit_action_result_for_request<T: Serialize>(
        &self,
        tool: &'static str,
        result: &Result<T, ErrorData>,
        request_context: &RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let session_id = action_session_id_from_request_context(request_context)?;
        match session_id.as_deref() {
            Some(session_id) => self.audit_action_result_for_session(tool, result, session_id),
            None => self.audit_action_result(tool, result),
        }
    }

    pub(super) fn audit_action_result_for_session<T: Serialize>(
        &self,
        tool: &'static str,
        result: &Result<T, ErrorData>,
        session_id: &str,
    ) -> Result<(), ErrorData> {
        match result {
            Ok(response) => self.write_action_audit_row(
                tool,
                "ok",
                None,
                &json!({
                    "response": response,
                }),
                Some(session_id),
            ),
            Err(error) => self.write_action_audit_row(
                tool,
                "error",
                error_data_code(error),
                &json!({
                    "message": error.message.to_string(),
                    "data": error.data.clone(),
                }),
                Some(session_id),
            ),
        }
    }

    pub(super) fn audit_action_ok_with_details_for_session(
        &self,
        tool: &'static str,
        details: &Value,
        session_id: &str,
    ) -> Result<(), ErrorData> {
        self.write_action_audit_row(tool, "ok", None, details, Some(session_id))
    }

    pub(super) fn audit_action_ok_with_details_for_request(
        &self,
        tool: &'static str,
        details: &Value,
        request_context: &RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let session_id = action_session_id_from_request_context(request_context)?;
        self.write_action_audit_row(tool, "ok", None, details, session_id.as_deref())
    }

    pub(super) fn audit_action_error_with_details_for_request(
        &self,
        tool: &'static str,
        error: &ErrorData,
        details: &Value,
        request_context: &RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let session_id = action_session_id_from_request_context(request_context)?;
        self.write_action_audit_row(
            tool,
            "error",
            error_data_code(error),
            &json!({
                "message": error.message.to_string(),
                "data": error.data.clone(),
                "request": details,
            }),
            session_id.as_deref(),
        )
    }

    pub(super) fn audit_action_error_with_details_for_session(
        &self,
        tool: &'static str,
        error: &ErrorData,
        details: &Value,
        session_id: &str,
    ) -> Result<(), ErrorData> {
        self.write_action_audit_row(
            tool,
            "error",
            error_data_code(error),
            &json!({
                "message": error.message.to_string(),
                "data": error.data.clone(),
                "request": details,
            }),
            Some(session_id),
        )
    }

    pub(super) fn audit_action_result_for_request_best_effort<T: Serialize>(
        &self,
        tool: &'static str,
        result: &Result<T, ErrorData>,
        request_context: &RequestContext<RoleServer>,
    ) {
        if let Err(error) = self.audit_action_result_for_request(tool, result, request_context) {
            tracing::warn!(
                code = "ACTION_AUDIT_WRITE_FAILED",
                tool,
                audit_error = %error,
                "action audit write failed after request-scoped action result"
            );
        }
    }

    fn write_action_audit_row(
        &self,
        tool: &'static str,
        status: &'static str,
        error_code: Option<&str>,
        details: &Value,
        action_session_id: Option<&str>,
    ) -> Result<(), ErrorData> {
        let (ts_ns, seq) = next_audit_key_parts();
        let active_profile = self.action_audit_active_profile();
        let mut audit_context = self.current_action_audit_context()?;
        let action_session_id = action_session_id
            .map(str::to_owned)
            .or_else(crate::http::current_mcp_session_id);
        if let Some(action_session_id) = action_session_id {
            audit_context.session_id = Some(action_session_id);
        }
        let session_id = audit_context.session_id.clone();
        let profile_id = audit_context.profile_id.clone();
        let profile_version = audit_context.profile_version.clone();
        let profile_schema_version = audit_context.profile_schema_version;
        let value = json!({
            "schema_version": 1,
            "audit_id": format!("{ts_ns:020}-{seq:010}"),
            "ts_ns": ts_ns,
            "seq": seq,
            "session_id": session_id,
            "profile_id": profile_id,
            "profile_version": profile_version,
            "profile_schema_version": profile_schema_version,
            "audit_context": audit_context,
            "tool": tool,
            "status": status,
            "error_code": error_code,
            "foreground": self.action_audit_foreground(),
            "active_profile_id": active_profile.profile_id,
            "active_profile_schema_version": active_profile.schema_version,
            "redacted": false,
            "redactions": [],
            "details": details,
        });
        let encoded = synapse_storage::encode_json(&value).map_err(|error| {
            mcp_error(
                synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                format!("action audit row encode failed: {error}"),
            )
        })?;
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_error| {
            mcp_error(
                synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while writing action audit",
            )
        })?;
        runtime
            .storage_put_action_log_rows(vec![(action_audit_key(ts_ns, seq), encoded)])
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        drop(runtime);
        tracing::info!(
            code = "ACTION_AUDIT_RECORDED",
            tool,
            status,
            ts_ns,
            seq,
            "action audit row written"
        );
        Ok(())
    }

    fn action_audit_foreground(&self) -> Value {
        match self.current_audit_foreground() {
            Ok(foreground) => {
                let observed_profile = self.action_audit_observed_profile(&foreground);
                json!({
                    "hwnd": foreground.hwnd,
                    "pid": foreground.pid,
                    "process_name": foreground.process_name,
                    "process_path": foreground.process_path,
                    "window_title": foreground.window_title,
                    "profile_id": observed_profile.profile_id,
                    "profile_schema_version": observed_profile.schema_version,
                })
            }
            Err(error) => json!({
                "read_error_code": error_data_code(&error),
                "read_error_message": error.message.to_string(),
            }),
        }
    }

    fn action_audit_observed_profile(
        &self,
        foreground: &synapse_core::ForegroundContext,
    ) -> ActionAuditProfileRef {
        let window = synapse_profiles::ForegroundWindow {
            exe: non_empty(&foreground.process_name),
            title: non_empty(&foreground.window_title),
            steam_appid: foreground.steam_appid,
            window_class: None,
        };
        let profile_id = self
            .profile_runtime()
            .ok()
            .and_then(|runtime| runtime.resolve_foreground(&window).ok().flatten())
            .map(|resolution| resolution.profile_id);
        ActionAuditProfileRef {
            schema_version: profile_id
                .as_deref()
                .and_then(|profile_id| self.action_audit_profile_schema_version(profile_id)),
            profile_id,
        }
    }

    fn action_audit_active_profile(&self) -> ActionAuditProfileRef {
        let profile_id = self
            .profile_runtime()
            .ok()
            .and_then(|runtime| runtime.active_profile_id().ok().flatten());
        ActionAuditProfileRef {
            schema_version: profile_id
                .as_deref()
                .and_then(|profile_id| self.action_audit_profile_schema_version(profile_id)),
            profile_id,
        }
    }

    fn action_audit_profile_schema_version(&self, profile_id: &str) -> Option<u32> {
        self.profile_runtime().ok().and_then(|runtime| {
            runtime
                .list(true)
                .ok()?
                .into_iter()
                .find(|profile| profile.id == profile_id)
                .map(|profile| profile.schema_version)
        })
    }
}

fn action_session_id_from_request_context(
    request_context: &RequestContext<RoleServer>,
) -> Result<Option<String>, ErrorData> {
    super::context::mcp_session_id_from_request_context(request_context)
}

struct ActionAuditProfileRef {
    profile_id: Option<String>,
    schema_version: Option<u32>,
}

fn next_audit_key_parts() -> (u64, u32) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let ts_ns = u64::try_from(nanos).unwrap_or(u64::MAX);
    let seq = ACTION_AUDIT_SEQ.fetch_add(1, Ordering::Relaxed);
    (ts_ns, seq)
}

fn action_audit_key(ts_ns: u64, seq: u32) -> Vec<u8> {
    let mut key = Vec::with_capacity(12);
    key.extend_from_slice(&ts_ns.to_be_bytes());
    key.extend_from_slice(&seq.to_be_bytes());
    key
}

fn error_data_code(error: &ErrorData) -> Option<&str> {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}
