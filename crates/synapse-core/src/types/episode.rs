use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::timeline::TimelineActor;

/// Envelope schema version for [`EpisodeRecord`] rows.
pub const EPISODE_RECORD_VERSION: u32 = 1;

/// Why an episode opened or closed (#846).
///
/// Recorded on both edges so re-segmentation regressions are diagnosable from
/// the persisted rows alone: a boundary that moves between runs names the
/// heuristic responsible.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeBoundary {
    /// Foreground moved to a different process.
    AppSwitch,
    /// Same app, different document (today: browser navigation to a
    /// different host).
    DocumentSwitch,
    /// An `idle_start` row closed the span (backdated to last input) or an
    /// `idle_end` row opened it.
    IdleGap,
    /// No timeline evidence for longer than the configured silent-gap
    /// threshold (recorder dead, machine asleep, or agent-only activity
    /// while human rows are excluded).
    SilentGap,
    /// A `session_start`/`session_end` recorder boundary row.
    SessionBoundary,
    /// Split at local midnight so every episode is contained in one local
    /// day — the invariant that makes day-aligned re-segmentation
    /// idempotent.
    DayBoundary,
    /// The episode ran into the edge of the segmented range.
    RangeEdge,
}

/// One derived episode row persisted in `CF_EPISODES` (#846).
///
/// Episodes are a pure, deterministic function of `CF_TIMELINE` rows: same
/// input rows + same config ⇒ byte-identical episodes, including
/// `episode_id`. `ts_ns` duplicates `start_ts_ns` because the storage TTL
/// compaction filter extracts a top-level `ts_ns` from the JSON bytes
/// (ADR 2026-06-11-timeline-data-model); a row without it would never expire
/// by age.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EpisodeRecord {
    pub record_version: u32,
    /// TTL contract field; always equals `start_ts_ns`.
    pub ts_ns: u64,
    /// Stable deterministic id: `ep1-` + first 16 hex chars of
    /// SHA-256 over `start_ts_ns|actor|app|document`.
    pub episode_id: String,
    pub start_ts_ns: u64,
    pub end_ts_ns: u64,
    pub actor: TimelineActor,
    /// Process executable name, lowercased as recorded by the timeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Document identity. Browser episodes use URL host from `browser_nav`;
    /// non-browser episodes use normalized foreground window title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<String>,
    /// Representative (most recent) full URL for browser episodes,
    /// query/fragment included as recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_first: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_last: Option<String>,
    /// Distinct foreground titles seen inside the episode.
    pub distinct_title_count: u32,
    /// Timeline evidence rows that fed this episode.
    pub row_count: u64,
    /// Aggregated from `interaction_summary` buckets inside the span.
    pub keystroke_count: u64,
    pub click_count: u64,
    /// Sub-threshold foreign focus spans absorbed into this episode
    /// (rapid alt-tab noise), and the time they consumed.
    pub interruption_count: u32,
    pub interrupted_ms: u64,
    pub started_because: EpisodeBoundary,
    pub ended_because: EpisodeBoundary,
}

impl EpisodeRecord {
    /// Episode duration in milliseconds.
    #[must_use]
    pub const fn duration_ms(&self) -> u64 {
        self.end_ts_ns.saturating_sub(self.start_ts_ns) / 1_000_000
    }
}
