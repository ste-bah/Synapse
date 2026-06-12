//! `episode_segment` MCP tool (#846, epic #830).
//!
//! Runs the deterministic segmentation engine
//! ([`synapse_core::episodes::segment_range`]) over `CF_TIMELINE` rows and
//! persists the resulting episodes in `CF_EPISODES`.
//!
//! Idempotency model: the requested range is snapped OUTWARD to whole local
//! days, and each day is segmented and replaced atomically (delete that
//! day's episode keys + write the new ones in one synchronous flushed
//! batch). Because the engine never lets an episode span local midnight, a
//! day is a closed replacement unit: re-segmenting any day range converges
//! to the same physical rows, byte for byte.
//!
//! Failure policy: disk pressure refusal, undecodable rows, and engine
//! errors are loud and structured. The tool never deletes rows it could not
//! re-derive (a failed day leaves storage untouched for that day).

use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{Days, Local, TimeZone};
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use synapse_core::episodes::{SegmentationConfig, segment_range};
use synapse_core::error_codes;
use synapse_core::types::TimelineRecord;
use synapse_reflex::ReflexRuntime;
use synapse_storage::{
    cf, decode_json, encode_json, episodes as episode_codec, timeline as timeline_codec,
};

use crate::m1::mcp_error;

use super::{
    M3ToolStub,
    permissions::{Permission, RequiredPermissions, required},
};

/// Maximum timeline rows scanned per call; a larger range pauses at a day
/// boundary and returns `next_start_ts_ns`.
pub const MAX_SCAN_ROWS_PER_CALL: usize = 200_000;
/// Maximum local days replaced per call.
pub const MAX_DAYS_PER_CALL: u32 = 92;
/// Chunk size for bounded storage reads inside one call.
const SCAN_CHUNK_ROWS: usize = 4_096;

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EpisodeSegmentParams {
    /// Inclusive lower bound; snapped DOWN to its local midnight. Defaults
    /// to the first timeline row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ts_ns: Option<u64>,
    /// Exclusive upper bound; snapped UP to the next local midnight.
    /// Defaults to the end of the current local day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ts_ns: Option<u64>,
    /// Segment agent-actor rows too (default false: human activity only).
    #[serde(default)]
    pub include_agent_activity: bool,
    /// Compute everything but mutate nothing.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EpisodeSegmentDay {
    /// Local-midnight day start (ns since epoch, UTC clock).
    pub day_start_ns: u64,
    pub day_end_ns: u64,
    /// Decodable timeline rows fed to the engine for this day.
    pub timeline_rows: u64,
    pub episodes_written: u64,
    pub episodes_deleted: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EpisodeSegmentResponse {
    /// Effective day-snapped range this call covered.
    pub range_start_ns: u64,
    pub range_end_ns: u64,
    pub days_processed: u32,
    /// Timeline rows examined (decodable or not).
    pub scanned_rows: u64,
    /// Rows whose key or value failed to decode; details are in daemon logs
    /// under code `TIMELINE_ROW_DECODE_FAILED`. Never segmented, never
    /// silently dropped from this count.
    pub invalid_rows: u64,
    /// Agent-actor rows ignored (include_agent_activity=false).
    pub ignored_agent_rows: u64,
    /// Rows with missing/anomalous payload fields (counted by the engine).
    pub payload_anomalies: u64,
    pub episodes_written: u64,
    pub episodes_deleted: u64,
    pub dry_run: bool,
    /// Per-day breakdown for days that had rows or stale episodes.
    pub days: Vec<EpisodeSegmentDay>,
    /// Present when the scan budget or day cap paused the run; pass back as
    /// `start_ts_ns` to continue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_start_ts_ns: Option<u64>,
    /// `range_complete`, `scan_budget_exhausted`, `day_cap_reached`, or
    /// `empty_timeline`.
    pub stopped_because: String,
}

#[must_use]
pub const fn episode_segment() -> M3ToolStub {
    M3ToolStub::new("episode_segment")
}

#[must_use]
pub fn required_permissions(_params: &EpisodeSegmentParams) -> RequiredPermissions {
    required([Permission::ReadStorage, Permission::WriteStorage])
}

fn invalid(detail: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, detail.into())
}

fn internal(detail: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_INTERNAL_ERROR, detail.into())
}

fn lock_runtime(
    runtime: &Arc<Mutex<ReflexRuntime>>,
) -> Result<MutexGuard<'_, ReflexRuntime>, ErrorData> {
    runtime.lock().map_err(|_err| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "reflex runtime lock poisoned",
        )
    })
}

fn key_after(key: &[u8]) -> Vec<u8> {
    let mut next = key.to_vec();
    next.push(0);
    next
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0F)]));
    }
    output
}

/// Local midnight at or before `ts_ns`, as ns since epoch.
///
/// Uses the host timezone database via `chrono::Local`; DST transitions are
/// resolved to the earliest valid local instant, and an unresolvable local
/// time is a structured error, never a guess.
fn local_day_start(ts_ns: u64) -> Result<u64, ErrorData> {
    let ts = i64::try_from(ts_ns)
        .map_err(|_e| invalid(format!("timestamp {ts_ns} exceeds the representable range")))?;
    let instant = Local
        .timestamp_nanos(ts)
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| internal("midnight is unrepresentable for the local date"))?;
    let midnight = Local
        .from_local_datetime(&instant)
        .earliest()
        .or_else(|| Local.from_local_datetime(&instant).latest())
        .ok_or_else(|| {
            internal(format!(
                "EPISODE_DAY_BOUNDARY_UNRESOLVABLE: no valid local instant for midnight of ts_ns {ts_ns}"
            ))
        })?;
    let nanos = midnight.timestamp_nanos_opt().ok_or_else(|| {
        internal(format!(
            "EPISODE_DAY_BOUNDARY_UNRESOLVABLE: midnight of ts_ns {ts_ns} overflows nanoseconds"
        ))
    })?;
    u64::try_from(nanos).map_err(|_e| {
        invalid(format!(
            "ts_ns {ts_ns} predates the epoch after day snapping"
        ))
    })
}

/// Local midnight strictly after `day_start_ns` (the next local day).
fn next_local_day_start(day_start_ns: u64) -> Result<u64, ErrorData> {
    let ts = i64::try_from(day_start_ns).map_err(|_e| {
        invalid(format!(
            "timestamp {day_start_ns} exceeds the representable range"
        ))
    })?;
    let date = Local
        .timestamp_nanos(ts)
        .date_naive()
        .checked_add_days(Days::new(1))
        .ok_or_else(|| internal("next local day overflows the calendar"))?;
    let instant = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| internal("midnight is unrepresentable for the next local date"))?;
    let midnight = Local
        .from_local_datetime(&instant)
        .earliest()
        .or_else(|| Local.from_local_datetime(&instant).latest())
        .ok_or_else(|| {
            internal(format!(
                "EPISODE_DAY_BOUNDARY_UNRESOLVABLE: no valid local instant for the day after {day_start_ns}"
            ))
        })?;
    let nanos = midnight.timestamp_nanos_opt().ok_or_else(|| {
        internal(format!(
            "EPISODE_DAY_BOUNDARY_UNRESOLVABLE: day after {day_start_ns} overflows nanoseconds"
        ))
    })?;
    u64::try_from(nanos).map_err(|_e| internal("next local day precedes the epoch"))
}

fn now_ts_ns() -> u64 {
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(i64::MAX);
    u64::try_from(nanos).unwrap_or(0)
}

/// First decodable timeline codec key timestamp, if any.
fn first_timeline_ts(
    runtime: &MutexGuard<'_, ReflexRuntime>,
    scanned_rows: &mut u64,
    invalid_rows: &mut u64,
) -> Result<Option<u64>, ErrorData> {
    let mut start: Vec<u8> = Vec::new();
    loop {
        if usize::try_from(*scanned_rows).unwrap_or(usize::MAX) >= MAX_SCAN_ROWS_PER_CALL {
            return Err(internal(
                "EPISODE_SCAN_BUDGET_EXHAUSTED while locating the first timeline row; \
                 pass an explicit start_ts_ns",
            ));
        }
        let (rows, more) = runtime
            .storage_cf_rows_from(cf::CF_TIMELINE, &start, SCAN_CHUNK_ROWS)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        if rows.is_empty() {
            return Ok(None);
        }
        for (key, _value) in &rows {
            *scanned_rows += 1;
            if let Ok((ts_ns, _seq)) = timeline_codec::decode_timeline_key(key) {
                return Ok(Some(ts_ns));
            }
            *invalid_rows += 1;
            tracing::warn!(
                code = "TIMELINE_ROW_DECODE_FAILED",
                key_hex = %hex_encode(key),
                "episode_segment skipped a non-codec CF_TIMELINE key while locating the first row"
            );
        }
        if !more {
            return Ok(None);
        }
        let last = rows.last().map(|(key, _value)| key.clone());
        let Some(last) = last else { return Ok(None) };
        start = key_after(&last);
    }
}

/// Collects and decodes the timeline rows of one local day, in key order.
fn day_timeline_rows(
    runtime: &MutexGuard<'_, ReflexRuntime>,
    day_start_ns: u64,
    day_end_ns: u64,
    scanned_rows: &mut u64,
    invalid_rows: &mut u64,
) -> Result<(Vec<TimelineRecord>, Option<u64>), ErrorData> {
    let mut records = Vec::new();
    let mut next_populated_ts: Option<u64> = None;
    let mut start = timeline_codec::timeline_scan_start(day_start_ns);
    'scan: loop {
        let (rows, more) = runtime
            .storage_cf_rows_from(cf::CF_TIMELINE, &start, SCAN_CHUNK_ROWS)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        if rows.is_empty() {
            break;
        }
        for (key, value) in &rows {
            *scanned_rows += 1;
            match timeline_codec::decode_timeline_key(key) {
                Ok((ts_ns, _seq)) => {
                    if ts_ns >= day_end_ns {
                        next_populated_ts = Some(ts_ns);
                        break 'scan;
                    }
                    match decode_json::<TimelineRecord>(value) {
                        Ok(record) => records.push(record),
                        Err(error) => {
                            *invalid_rows += 1;
                            tracing::warn!(
                                code = "TIMELINE_ROW_DECODE_FAILED",
                                key_hex = %hex_encode(key),
                                %error,
                                "episode_segment skipped an undecodable CF_TIMELINE row"
                            );
                        }
                    }
                }
                Err(error) => {
                    *invalid_rows += 1;
                    tracing::warn!(
                        code = "TIMELINE_ROW_DECODE_FAILED",
                        key_hex = %hex_encode(key),
                        %error,
                        "episode_segment skipped a non-codec CF_TIMELINE key"
                    );
                }
            }
        }
        if !more {
            break;
        }
        let last = rows.last().map(|(key, _value)| key.clone());
        let Some(last) = last else { break };
        start = key_after(&last);
    }
    Ok((records, next_populated_ts))
}

/// Existing `CF_EPISODES` keys with start timestamps in `[start_ns, end_ns)`.
fn existing_episode_keys(
    runtime: &MutexGuard<'_, ReflexRuntime>,
    start_ns: u64,
    end_ns: u64,
) -> Result<Vec<Vec<u8>>, ErrorData> {
    let mut keys = Vec::new();
    let mut start = episode_codec::episode_scan_start(start_ns);
    'scan: loop {
        let (rows, more) = runtime
            .storage_cf_rows_from(cf::CF_EPISODES, &start, SCAN_CHUNK_ROWS)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        if rows.is_empty() {
            break;
        }
        for (key, _value) in &rows {
            match episode_codec::decode_episode_key(key) {
                Ok((ts_ns, _ordinal)) => {
                    if ts_ns >= end_ns {
                        break 'scan;
                    }
                    keys.push(key.clone());
                }
                Err(error) => {
                    // A malformed derived-state key is corruption we own:
                    // refuse to replace around it rather than strand it.
                    return Err(mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        format!(
                            "EPISODE_KEY_INVALID in CF_EPISODES at {}: {error}; refusing to \
                             replace a range containing keys this codec cannot account for",
                            hex_encode(key)
                        ),
                    ));
                }
            }
        }
        if !more {
            break;
        }
        let last = rows.last().map(|(key, _value)| key.clone());
        let Some(last) = last else { break };
        start = key_after(&last);
    }
    Ok(keys)
}

pub fn segment_episodes(
    runtime: &Arc<Mutex<ReflexRuntime>>,
    params: &EpisodeSegmentParams,
) -> Result<EpisodeSegmentResponse, ErrorData> {
    if let (Some(start), Some(end)) = (params.start_ts_ns, params.end_ts_ns)
        && start >= end
    {
        return Err(invalid(format!(
            "episode_segment start_ts_ns {start} must be < end_ts_ns {end}"
        )));
    }

    let config = SegmentationConfig {
        include_agent_activity: params.include_agent_activity,
        ..SegmentationConfig::default()
    };

    let runtime = lock_runtime(runtime)?;
    if !params.dry_run && !runtime.storage_pressure_permits_write(cf::CF_EPISODES) {
        return Err(mcp_error(
            error_codes::STORAGE_WRITE_FAILED,
            format!(
                "episode_segment refused under disk pressure: cf_name={} pressure_level={:?}; \
                 nothing was deleted or written",
                cf::CF_EPISODES,
                runtime.storage_pressure_level()
            ),
        ));
    }

    let mut scanned_rows = 0_u64;
    let mut invalid_rows = 0_u64;

    // Effective range: explicit bounds, else first row → end of today.
    let range_start = match params.start_ts_ns {
        Some(start) => start,
        None => match first_timeline_ts(&runtime, &mut scanned_rows, &mut invalid_rows)? {
            Some(ts_ns) => ts_ns,
            None => {
                return Ok(EpisodeSegmentResponse {
                    range_start_ns: 0,
                    range_end_ns: 0,
                    days_processed: 0,
                    scanned_rows,
                    invalid_rows,
                    ignored_agent_rows: 0,
                    payload_anomalies: 0,
                    episodes_written: 0,
                    episodes_deleted: 0,
                    dry_run: params.dry_run,
                    days: Vec::new(),
                    next_start_ts_ns: None,
                    stopped_because: "empty_timeline".to_owned(),
                });
            }
        },
    };
    let range_end = params.end_ts_ns.unwrap_or_else(now_ts_ns);
    if range_start >= range_end {
        return Err(invalid(format!(
            "episode_segment effective range is empty: start {range_start} >= end {range_end}"
        )));
    }
    let range_start_snapped = local_day_start(range_start)?;
    let range_end_snapped = next_local_day_start(local_day_start(range_end.saturating_sub(1))?)?;

    let mut response = EpisodeSegmentResponse {
        range_start_ns: range_start_snapped,
        range_end_ns: range_end_snapped,
        days_processed: 0,
        scanned_rows: 0,
        invalid_rows: 0,
        ignored_agent_rows: 0,
        payload_anomalies: 0,
        episodes_written: 0,
        episodes_deleted: 0,
        dry_run: params.dry_run,
        days: Vec::new(),
        next_start_ts_ns: None,
        stopped_because: "range_complete".to_owned(),
    };

    let mut day_start = range_start_snapped;
    while day_start < range_end_snapped {
        if response.days_processed >= MAX_DAYS_PER_CALL {
            response.next_start_ts_ns = Some(day_start);
            response.stopped_because = "day_cap_reached".to_owned();
            break;
        }
        if usize::try_from(scanned_rows).unwrap_or(usize::MAX) >= MAX_SCAN_ROWS_PER_CALL {
            response.next_start_ts_ns = Some(day_start);
            response.stopped_because = "scan_budget_exhausted".to_owned();
            break;
        }
        let day_end = next_local_day_start(day_start)?;
        let end_is_day_boundary = day_end < range_end_snapped;

        let (records, next_populated_ts) = day_timeline_rows(
            &runtime,
            day_start,
            day_end,
            &mut scanned_rows,
            &mut invalid_rows,
        )?;
        let timeline_rows = u64::try_from(records.len()).unwrap_or(u64::MAX);

        let segmentation =
            segment_range(&records, day_start, day_end, end_is_day_boundary, &config).map_err(
                |error| {
                    internal(format!(
                        "episode_segment engine failed for day starting {day_start}: {error}"
                    ))
                },
            )?;
        response.ignored_agent_rows += segmentation.ignored_agent_rows;
        response.payload_anomalies += segmentation.payload_anomalies;

        // Replacement unit: this day. Collect stale keys, build new rows,
        // and swap them in one atomic flushed batch.
        let stale_keys = existing_episode_keys(&runtime, day_start, day_end)?;
        let mut new_rows: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(segmentation.episodes.len());
        for (ordinal, episode) in segmentation.episodes.iter().enumerate() {
            let ordinal = u32::try_from(ordinal).map_err(|_e| {
                internal(format!(
                    "episode ordinal {ordinal} overflows u32 for day starting {day_start}"
                ))
            })?;
            let key = episode_codec::episode_key(episode.start_ts_ns, ordinal);
            let value =
                encode_json(episode).map_err(|error| mcp_error(error.code(), error.to_string()))?;
            new_rows.push((key, value));
        }
        let deleted = u64::try_from(stale_keys.len()).unwrap_or(u64::MAX);
        let written = u64::try_from(new_rows.len()).unwrap_or(u64::MAX);
        if !params.dry_run && (deleted > 0 || written > 0) {
            runtime
                .storage_replace_rows(cf::CF_EPISODES, stale_keys, new_rows)
                .map_err(|error| {
                    mcp_error(
                        error.code(),
                        format!(
                            "episode_segment failed to replace day starting {day_start}: {error}; \
                             that day's previous episodes are unchanged"
                        ),
                    )
                })?;
        }
        response.episodes_deleted += deleted;
        response.episodes_written += written;
        response.days_processed += 1;
        if timeline_rows > 0 || deleted > 0 || written > 0 {
            response.days.push(EpisodeSegmentDay {
                day_start_ns: day_start,
                day_end_ns: day_end,
                timeline_rows,
                episodes_written: written,
                episodes_deleted: deleted,
            });
        }
        tracing::info!(
            code = "EPISODE_DAY_SEGMENTED",
            day_start_ns = day_start,
            day_end_ns = day_end,
            timeline_rows,
            episodes_written = written,
            episodes_deleted = deleted,
            dry_run = params.dry_run,
            "episode_segment replaced one local day"
        );

        // Skip empty stretches fast, but never past days holding stale
        // episodes: jump to the next populated timeline day (clamped to the
        // range) unless a skipped day still holds episode rows to clean.
        let target_day = match next_populated_ts {
            Some(ts_ns) => local_day_start(ts_ns)?.clamp(day_end, range_end_snapped),
            None => range_end_snapped,
        };
        day_start = if target_day > day_end {
            let stale_between = existing_episode_keys(&runtime, day_end, target_day)?;
            if stale_between.is_empty() {
                target_day
            } else {
                day_end
            }
        } else {
            day_end
        };
    }

    response.scanned_rows = scanned_rows;
    response.invalid_rows = invalid_rows;
    Ok(response)
}
