use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::SessionId;

/// Envelope schema version for [`TimelineRecord`] rows.
pub const TIMELINE_RECORD_VERSION: u32 = 1;

/// One operator-activity timeline row persisted in `CF_TIMELINE`.
///
/// ADR 2026-06-11-timeline-data-model: `ts_ns` must stay a required top-level
/// field because the storage TTL compaction filter extracts it from the JSON
/// bytes; a row without it would never expire by age.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimelineRecord {
    pub record_version: u32,
    pub ts_ns: u64,
    pub kind: TimelineKind,
    pub actor: TimelineActor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub payload: Value,
}

/// Discriminant for timeline row payloads.
///
/// Raw keystroke content is deliberately unrepresentable: interaction rows
/// carry counts and cadence buckets only.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimelineKind {
    FocusChange,
    TitleChange,
    IdleStart,
    IdleEnd,
    SessionStart,
    SessionEnd,
    InteractionSummary,
    Clipboard,
    FileActivity,
    BrowserNav,
    DemoMarker,
}

/// Who produced the activity behind a timeline row.
///
/// Agent-driven activity is recorded with its acting session so episode
/// segmentation and cadence statistics can separate the human from agents.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "actor", rename_all = "snake_case")]
pub enum TimelineActor {
    Human,
    Agent { session_id: SessionId },
}

impl TimelineRecord {
    /// Builds a version-stamped record.
    #[must_use]
    pub const fn new(ts_ns: u64, kind: TimelineKind, actor: TimelineActor) -> Self {
        Self {
            record_version: TIMELINE_RECORD_VERSION,
            ts_ns,
            kind,
            actor,
            app: None,
            payload: Value::Null,
        }
    }
}
