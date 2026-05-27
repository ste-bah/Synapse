use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    AccessibleNode, Action, AudioContext, Backend, ClipboardSummary, DetectedEntity, ElementId,
    EventSource, EventSummary, FocusedElement, ForegroundContext, FsEvent, HudReadings,
    ObservationDiagnostics, PerceptionMode, ProfileId, ReflexId, ReflexState, SessionId,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredRedaction {
    pub kind: String,
    pub offset: u32,
    pub len: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredBackendPolicy {
    pub default: Backend,
    pub keyboard_default: Backend,
    pub mouse_default: Backend,
    pub pad_default: Backend,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredAppContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gameid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredAuditContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<ProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_schema_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_policy: Option<StoredBackendPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_context: Option<StoredAppContext>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredEvent {
    pub schema_version: u32,
    pub event_id: String,
    pub ts_ns: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_context: Option<StoredAuditContext>,
    pub source: EventSource,
    pub kind: String,
    pub data: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<ElementId>,
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redactions: Vec<StoredRedaction>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredObservation {
    pub schema_version: u32,
    pub observation_id: String,
    pub ts_ns: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    pub mode: PerceptionMode,
    pub foreground: ForegroundContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<FocusedElement>,
    #[serde(default)]
    pub elements: Vec<AccessibleNode>,
    #[serde(default)]
    pub entities: Vec<DetectedEntity>,
    #[serde(default)]
    pub hud: HudReadings,
    #[serde(default)]
    pub audio: AudioContext,
    #[serde(default)]
    pub recent_events: Vec<EventSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard_summary: Option<ClipboardSummary>,
    #[serde(default)]
    pub fs_recent: Vec<FsEvent>,
    pub diagnostics: ObservationDiagnostics,
    pub reason: String,
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redactions: Vec<StoredRedaction>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredReflexStep {
    pub index: u32,
    pub action: Action,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredReflexAudit {
    pub schema_version: u32,
    pub audit_id: String,
    pub reflex_id: ReflexId,
    pub ts_ns: u64,
    pub status: ReflexState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_context: Option<StoredAuditContext>,
    #[serde(default)]
    pub steps: Vec<StoredReflexStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default)]
    pub details: serde_json::Value,
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redactions: Vec<StoredRedaction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredProfileHistoryEntry {
    pub profile_id: ProfileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_schema_version: Option<u32>,
    pub activated_at: DateTime<Utc>,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredSession {
    pub schema_version: u32,
    pub session_id: SessionId,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    pub transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    pub mode: PerceptionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<ProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_context: Option<StoredAuditContext>,
    #[serde(default)]
    pub profile_history: Vec<StoredProfileHistoryEntry>,
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redactions: Vec<StoredRedaction>,
}
