use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::Utc;
use rmcp::ErrorData;
use serde_json::{Value, json};
use synapse_core::{
    EventSource, Profile, ProfileBackends, ProfileId, SCHEMA_VERSION, SessionId, StoredAppContext,
    StoredAuditContext, StoredBackendPolicy, StoredEvent, StoredProfileHistoryEntry, StoredSession,
    error_codes, new_session_id,
};

use super::SynapseService;
use crate::{
    m1::{current_input, mcp_error},
    m3::AuditSessionState,
};

static EVENT_AUDIT_SEQ: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Debug)]
struct ProfileAuditInfo {
    profile: Profile,
    schema_version: u32,
}

impl SynapseService {
    pub(super) fn persist_profile_activation_success(
        &self,
        profile_id: &ProfileId,
        changed: bool,
    ) -> Result<(), ErrorData> {
        let session_id = self.ensure_audit_session_started()?;
        let profile_info = self.profile_audit_info(profile_id)?;
        let audit_context = self.audit_context_for_profile(&profile_info, Some(session_id.clone()));
        let activated_at = Utc::now();
        let mut state = self.m3_state.lock().map_err(|_err| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "M3 service state lock poisoned while updating audit session",
            )
        })?;
        let (session_started_at, profile_history) = {
            let audit_session = state.audit_session.as_mut().ok_or_else(|| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "audit session disappeared while persisting profile activation",
                )
            })?;
            let should_append = changed
                || audit_session
                    .profile_history
                    .last()
                    .is_none_or(|last| last.profile_id != *profile_id);
            if should_append {
                audit_session
                    .profile_history
                    .push(StoredProfileHistoryEntry {
                        profile_id: profile_id.clone(),
                        profile_version: Some(profile_info.profile.version.clone()),
                        profile_schema_version: Some(profile_info.schema_version),
                        activated_at,
                        reason: "profile_activate".to_owned(),
                    });
            }
            (
                audit_session.started_at,
                audit_session.profile_history.clone(),
            )
        };
        drop(state);
        let session = StoredSession {
            schema_version: SCHEMA_VERSION,
            session_id: session_id.clone(),
            started_at: session_started_at,
            ended_at: None,
            transport: "mcp".to_owned(),
            client: Some("synapse-mcp".to_owned()),
            mode: profile_info.profile.mode,
            active_profile: Some(profile_id.clone()),
            audit_context: Some(audit_context.clone()),
            profile_history,
            redacted: false,
            redactions: Vec::new(),
        };

        self.write_session_row(&session)?;
        self.write_event_row(&StoredEvent {
            schema_version: SCHEMA_VERSION,
            event_id: format!("profile-activation-{}", event_id_suffix()),
            ts_ns: now_ts_ns(),
            session_id: Some(session_id),
            audit_context: Some(audit_context.clone()),
            source: EventSource::System,
            kind: "profile.activated".to_owned(),
            data: json!({
                "profile_id": profile_id,
                "profile_version": profile_info.profile.version,
                "profile_schema_version": profile_info.schema_version,
                "changed": changed,
                "reason": "profile_activate",
            }),
            window_id: None,
            element_id: None,
            redacted: false,
            redactions: Vec::new(),
        })?;
        self.set_reflex_runtime_audit_context(Some(audit_context))?;
        Ok(())
    }

    pub(super) fn persist_profile_activation_denied(
        &self,
        profile_id: &ProfileId,
        error: &ErrorData,
    ) {
        let audit_context = self.audit_context_for_denied_profile(profile_id);
        let event = StoredEvent {
            schema_version: SCHEMA_VERSION,
            event_id: format!("profile-activation-denied-{}", event_id_suffix()),
            ts_ns: now_ts_ns(),
            session_id: audit_context.session_id.clone(),
            audit_context: Some(audit_context),
            source: EventSource::System,
            kind: "profile.activation_denied".to_owned(),
            data: json!({
                "profile_id": profile_id,
                "error_code": error_data_code(error),
                "message": error.message.to_string(),
                "data": error.data.clone(),
            }),
            window_id: None,
            element_id: None,
            redacted: false,
            redactions: Vec::new(),
        };
        if let Err(write_error) = self.write_event_row(&event) {
            tracing::warn!(
                code = "PROFILE_ACTIVATION_AUDIT_WRITE_FAILED",
                profile_id = %profile_id,
                audit_error = %write_error,
                "profile activation denial audit write failed"
            );
        }
    }

    pub(super) fn current_action_audit_context(&self) -> Result<StoredAuditContext, ErrorData> {
        let session_id = self.current_audit_session_id()?;
        let profile_id = self
            .profile_runtime()
            .ok()
            .and_then(|runtime| runtime.active_profile_id().ok().flatten());
        match profile_id {
            Some(profile_id) => {
                let info = self.profile_audit_info(&profile_id)?;
                Ok(self.audit_context_for_profile(&info, session_id))
            }
            None => Ok(StoredAuditContext {
                session_id,
                profile_id: None,
                profile_version: None,
                profile_schema_version: None,
                backend_policy: None,
                app_context: self.current_app_context(None),
            }),
        }
    }

    pub(super) fn refresh_reflex_audit_context(&self) -> Result<(), ErrorData> {
        let context = self.current_action_audit_context()?;
        self.set_reflex_runtime_audit_context(Some(context))
    }

    fn ensure_audit_session_started(&self) -> Result<SessionId, ErrorData> {
        let mut state = self.m3_state.lock().map_err(|_err| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "M3 service state lock poisoned while starting audit session",
            )
        })?;
        if state.audit_session.is_none() {
            state.audit_session = Some(AuditSessionState {
                session_id: new_session_id(),
                started_at: Utc::now(),
                profile_history: Vec::new(),
            });
        }
        state
            .audit_session
            .as_ref()
            .map(|session| session.session_id.clone())
            .ok_or_else(|| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "audit session did not initialize",
                )
            })
    }

    fn current_audit_session_id(&self) -> Result<Option<SessionId>, ErrorData> {
        self.m3_state
            .lock()
            .map(|state| {
                state
                    .audit_session
                    .as_ref()
                    .map(|session| session.session_id.clone())
            })
            .map_err(|_err| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "M3 service state lock poisoned while reading audit session",
                )
            })
    }

    fn profile_audit_info(&self, profile_id: &str) -> Result<ProfileAuditInfo, ErrorData> {
        let runtime = self.profile_runtime()?;
        let profile = runtime
            .profile(profile_id)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::PROFILE_NOT_FOUND,
                    format!("profile {profile_id} was not found"),
                )
            })?;
        let schema_version = runtime
            .list(true)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?
            .into_iter()
            .find(|status| status.id == profile_id)
            .map_or(SCHEMA_VERSION, |status| status.schema_version);
        Ok(ProfileAuditInfo {
            profile,
            schema_version,
        })
    }

    fn audit_context_for_profile(
        &self,
        info: &ProfileAuditInfo,
        session_id: Option<SessionId>,
    ) -> StoredAuditContext {
        StoredAuditContext {
            session_id,
            profile_id: Some(info.profile.id.clone()),
            profile_version: Some(info.profile.version.clone()),
            profile_schema_version: Some(info.schema_version),
            backend_policy: Some(backend_policy(info.profile.backends)),
            app_context: self.current_app_context(Some(&info.profile)),
        }
    }

    fn audit_context_for_denied_profile(&self, profile_id: &ProfileId) -> StoredAuditContext {
        StoredAuditContext {
            session_id: self.current_audit_session_id().ok().flatten(),
            profile_id: Some(profile_id.clone()),
            profile_version: None,
            profile_schema_version: None,
            backend_policy: None,
            app_context: self.current_app_context(None),
        }
    }

    fn current_app_context(&self, profile: Option<&Profile>) -> Option<StoredAppContext> {
        let foreground = self
            .m1_state()
            .ok()
            .and_then(|state| current_input(&state, 1).ok())
            .map(|input| input.foreground);
        let metadata = profile.map(|profile| &profile.metadata);
        let has_metadata = metadata.is_some();
        let has_foreground = foreground.is_some();
        if !has_metadata && !has_foreground {
            return None;
        }
        Some(StoredAppContext {
            process_name: foreground.as_ref().map(|value| value.process_name.clone()),
            process_path: foreground.as_ref().map(|value| value.process_path.clone()),
            window_title: foreground.as_ref().map(|value| value.window_title.clone()),
            target_id: metadata.and_then(|value| {
                value
                    .get("benchmark_id")
                    .or_else(|| value.get("registry.family"))
                    .cloned()
            }),
            gameid: metadata.and_then(|value| value.get("benchmark_world_gameid").cloned()),
            world_name: metadata.and_then(|value| value.get("benchmark_world_name").cloned()),
            world_path: metadata.and_then(|value| value.get("launch.world").cloned()),
            log_path: metadata.and_then(|value| value.get("launch.logfile").cloned()),
        })
    }

    fn write_session_row(&self, session: &StoredSession) -> Result<(), ErrorData> {
        let encoded = synapse_storage::encode_json(session).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("session audit row encode failed: {error}"),
            )
        })?;
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while writing session audit",
            )
        })?;
        runtime
            .storage_put_session_rows(vec![(session_key(&session.session_id), encoded)])
            .map_err(|error| mcp_error(error.code(), error.to_string()))
    }

    fn write_event_row(&self, event: &StoredEvent) -> Result<(), ErrorData> {
        let encoded = synapse_storage::encode_json(event).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("event audit row encode failed: {error}"),
            )
        })?;
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while writing event audit",
            )
        })?;
        runtime
            .storage_put_event_rows(vec![(event_key(event.ts_ns), encoded)])
            .map_err(|error| mcp_error(error.code(), error.to_string()))
    }

    fn set_reflex_runtime_audit_context(
        &self,
        context: Option<StoredAuditContext>,
    ) -> Result<(), ErrorData> {
        let runtime = self.reflex_runtime()?;
        runtime
            .lock()
            .map_err(|_error| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while setting audit context",
                )
            })?
            .set_audit_context(context);
        Ok(())
    }
}

const fn backend_policy(backends: ProfileBackends) -> StoredBackendPolicy {
    StoredBackendPolicy {
        default: backends.default,
        keyboard_default: backends.keyboard_default,
        mouse_default: backends.mouse_default,
        pad_default: backends.pad_default,
    }
}

fn session_key(session_id: &str) -> Vec<u8> {
    format!("session/v1/{session_id}").into_bytes()
}

fn event_key(ts_ns: u64) -> Vec<u8> {
    let seq = EVENT_AUDIT_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut key = Vec::with_capacity(12);
    key.extend_from_slice(&ts_ns.to_be_bytes());
    key.extend_from_slice(&seq.to_be_bytes());
    key
}

fn event_id_suffix() -> String {
    format!(
        "{}-{}",
        now_ts_ns(),
        EVENT_AUDIT_SEQ.load(Ordering::Relaxed)
    )
}

fn now_ts_ns() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    u64::try_from(nanos).unwrap_or(u64::MAX)
}

fn error_data_code(error: &ErrorData) -> Option<&str> {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
}
