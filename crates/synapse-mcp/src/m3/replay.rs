use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use rmcp::{ErrorData, schemars::JsonSchema};
use schemars::{Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use synapse_core::{Event, EventFilter, Observation, error_codes};
use synapse_perception::{ObservationAssembler, ObserveInclude};
use tokio::{
    fs::{self, File},
    io::{AsyncWrite, AsyncWriteExt, BufWriter},
    time::{Instant, sleep},
};

use crate::{
    http::sse::SseState,
    m1::{ObserveParams, SharedM1State, current_input, mcp_error, observe_include},
};

use super::{
    M3ToolStub,
    permissions::{Permission, RequiredPermissions, normalize_replay_path, replay_root, required},
};

const DEFAULT_TARGET: &str = "observations";
const DEFAULT_FORMAT: &str = "jsonl";
const OBSERVATION_SAMPLE_INTERVAL: Duration = Duration::from_millis(250);
const EVENT_DRAIN_INTERVAL: Duration = Duration::from_millis(20);

fn default_target() -> String {
    DEFAULT_TARGET.to_owned()
}

fn default_format() -> String {
    DEFAULT_FORMAT.to_owned()
}

fn replay_target_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "enum": ["observations", "events", "both"],
        "default": DEFAULT_TARGET
    })
}

fn replay_format_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "enum": [DEFAULT_FORMAT],
        "default": DEFAULT_FORMAT
    })
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayRecordParams {
    #[serde(default = "default_target")]
    #[schemars(schema_with = "replay_target_schema")]
    pub target: String,
    #[serde(default = "default_format")]
    #[schemars(schema_with = "replay_format_schema")]
    pub format: String,
    #[schemars(range(min = 0))]
    pub duration_ms: u32,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayRecordResponse {
    pub path: String,
    pub records_written: u64,
    pub observations_skipped: u64,
    pub bytes: u64,
}

#[must_use]
pub const fn replay_record() -> M3ToolStub {
    M3ToolStub::new("replay_record")
}

#[must_use]
pub fn required_permissions(_params: &ReplayRecordParams) -> RequiredPermissions {
    required([Permission::WriteReplay])
}

pub async fn record_replay(
    m1_state: SharedM1State,
    sse_state: SseState,
    params: &ReplayRecordParams,
) -> Result<ReplayRecordResponse, ErrorData> {
    let target = ReplayTarget::parse(&params.target)?;
    let _format = ReplayFormat::parse(&params.format)?;
    let path = replay_path(params.path.as_deref())?;
    create_parent_dir(&path).await?;

    let file = File::create(&path).await.map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("replay_record could not create {}: {error}", path.display()),
        )
    })?;
    let mut writer = BufWriter::new(file);

    let stats = if params.duration_ms > 0 {
        record_window(&mut writer, &m1_state, &sse_state, target, params).await?
    } else {
        RecordWindowStats::default()
    };

    writer.flush().await.map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("replay_record could not flush {}: {error}", path.display()),
        )
    })?;
    drop(writer);

    let bytes = fs::metadata(&path).await.map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!(
                "replay_record could not read metadata for {}: {error}",
                path.display()
            ),
        )
    })?;

    Ok(ReplayRecordResponse {
        path: display_path(&path),
        records_written: stats.records_written,
        observations_skipped: stats.observations_skipped,
        bytes: bytes.len(),
    })
}

async fn record_window<W>(
    writer: &mut W,
    m1_state: &SharedM1State,
    sse_state: &SseState,
    target: ReplayTarget,
    params: &ReplayRecordParams,
) -> Result<RecordWindowStats, ErrorData>
where
    W: AsyncWrite + Unpin + Send,
{
    let mut stats = RecordWindowStats::default();
    let deadline = Instant::now() + Duration::from_millis(u64::from(params.duration_ms));
    let mut event_subscription = if target.includes_events() {
        Some(
            sse_state
                .event_bus()
                .subscribe(EventFilter::All, Vec::new(), false)
                .map_err(|error| mcp_error(error.code(), error.to_string()))?,
        )
    } else {
        None
    };
    let event_bus = sse_state.event_bus();
    let assembler = ObservationAssembler::new();
    let include = observe_include(&ObserveParams::default());
    let mut next_observation_sample = Instant::now();

    let result = async {
        if target.includes_observations() {
            stats.record_observation(
                write_observation(writer, m1_state, &assembler, include, target).await?,
            );
            next_observation_sample = Instant::now() + OBSERVATION_SAMPLE_INTERVAL;
        }

        while Instant::now() < deadline {
            if let Some(subscription) = &event_subscription {
                stats.records_written = stats
                    .records_written
                    .saturating_add(drain_events(writer, subscription.drain(), target).await?);
            }

            if target.includes_observations() && Instant::now() >= next_observation_sample {
                stats.record_observation(
                    write_observation(writer, m1_state, &assembler, include, target).await?,
                );
                next_observation_sample += OBSERVATION_SAMPLE_INTERVAL;
            }

            sleep(next_sleep(deadline, next_observation_sample, target)).await;
        }

        if let Some(subscription) = &event_subscription {
            stats.records_written = stats
                .records_written
                .saturating_add(drain_events(writer, subscription.drain(), target).await?);
        }

        if target.includes_observations()
            && stats.observations_written == 0
            && stats.observations_skipped > 0
        {
            return Err(mcp_error(
                error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
                format!(
                    "replay_record could not capture any observations; skipped {} unavailable samples",
                    stats.observations_skipped
                ),
            ));
        }

        if stats.observations_skipped > 0 {
            tracing::warn!(
                code = "REPLAY_OBSERVATION_GAPS_SKIPPED",
                observations_skipped = stats.observations_skipped,
                observations_written = stats.observations_written,
                "replay_record skipped transient unavailable observation samples"
            );
        }

        Ok(stats)
    }
    .await;

    if let Some(subscription) = event_subscription.take() {
        event_bus.unsubscribe(subscription.id());
    }

    result
}

async fn write_observation<W>(
    writer: &mut W,
    m1_state: &SharedM1State,
    assembler: &ObservationAssembler,
    include: ObserveInclude,
    target: ReplayTarget,
) -> Result<ObservationWrite, ErrorData>
where
    W: AsyncWrite + Unpin + Send,
{
    let input = {
        let state = m1_state.lock().map_err(|_err| {
            mcp_error(
                error_codes::OBSERVE_INTERNAL,
                "M1 service state lock poisoned",
            )
        })?;
        match current_input(&state, include.max_subtree_depth) {
            Ok(input) => input,
            Err(error) if is_no_perception_error(&error) => {
                return Ok(ObservationWrite::skipped());
            }
            Err(error) => return Err(error),
        }
    };
    let observation = match assembler
        .assemble(include, input)
        .map_err(|error| mcp_error(error.code(), error.to_string()))
    {
        Ok(observation) => observation,
        Err(error) if is_no_perception_error(&error) => return Ok(ObservationWrite::skipped()),
        Err(error) => return Err(error),
    };
    match target {
        ReplayTarget::Observations => write_json_line(writer, &observation).await?,
        ReplayTarget::Both => {
            write_json_line(
                writer,
                &ReplayRecordLine::Observation {
                    record: &observation,
                },
            )
            .await?;
        }
        ReplayTarget::Events => {}
    }
    Ok(ObservationWrite::written(target))
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
struct RecordWindowStats {
    records_written: u64,
    observations_written: u64,
    observations_skipped: u64,
}

impl RecordWindowStats {
    const fn record_observation(&mut self, write: ObservationWrite) {
        self.records_written = self.records_written.saturating_add(write.records_written);
        self.observations_written = self
            .observations_written
            .saturating_add(write.observations_written);
        self.observations_skipped = self
            .observations_skipped
            .saturating_add(write.observations_skipped);
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
struct ObservationWrite {
    records_written: u64,
    observations_written: u64,
    observations_skipped: u64,
}

impl ObservationWrite {
    const fn skipped() -> Self {
        Self {
            records_written: 0,
            observations_written: 0,
            observations_skipped: 1,
        }
    }

    const fn written(target: ReplayTarget) -> Self {
        Self {
            records_written: if target.includes_observations() { 1 } else { 0 },
            observations_written: 1,
            observations_skipped: 0,
        }
    }
}

fn is_no_perception_error(error: &ErrorData) -> bool {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(serde_json::Value::as_str)
        == Some(error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE)
}

async fn drain_events<W>(
    writer: &mut W,
    events: Vec<Event>,
    target: ReplayTarget,
) -> Result<u64, ErrorData>
where
    W: AsyncWrite + Unpin + Send,
{
    let mut records_written = 0_u64;
    for event in events {
        match target {
            ReplayTarget::Events => write_json_line(writer, &event).await?,
            ReplayTarget::Both => {
                write_json_line(writer, &ReplayRecordLine::Event { record: &event }).await?;
            }
            ReplayTarget::Observations => {}
        }
        records_written = records_written.saturating_add(1);
    }
    Ok(records_written)
}

async fn write_json_line<W, T>(writer: &mut W, value: &T) -> Result<(), ErrorData>
where
    W: AsyncWrite + Unpin + Send,
    T: Serialize + Sync + ?Sized,
{
    let line = serde_json::to_vec(value).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("replay_record could not serialize record: {error}"),
        )
    })?;
    writer
        .write_all(&line)
        .await
        .map_err(|error| write_error(&error))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|error| write_error(&error))
}

fn write_error(error: &std::io::Error) -> ErrorData {
    mcp_error(
        error_codes::TOOL_INTERNAL_ERROR,
        format!("replay_record could not write JSONL record: {error}"),
    )
}

async fn create_parent_dir(path: &Path) -> Result<(), ErrorData> {
    let parent = path.parent().filter(|value| !value.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent).await.map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!(
                    "replay_record could not create parent directory {}: {error}",
                    parent.display()
                ),
            )
        })?;
    }
    Ok(())
}

fn replay_path(path: Option<&str>) -> Result<PathBuf, ErrorData> {
    normalize_replay_path(&replay_root(), path)
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn next_sleep(
    deadline: Instant,
    next_observation_sample: Instant,
    target: ReplayTarget,
) -> Duration {
    let now = Instant::now();
    if now >= deadline {
        return Duration::ZERO;
    }
    let until_deadline = deadline.saturating_duration_since(now);
    let base = until_deadline.min(EVENT_DRAIN_INTERVAL);
    if target.includes_observations() {
        base.min(next_observation_sample.saturating_duration_since(now))
    } else {
        base
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ReplayTarget {
    Observations,
    Events,
    Both,
}

impl ReplayTarget {
    fn parse(value: &str) -> Result<Self, ErrorData> {
        match value.trim() {
            "observations" => Ok(Self::Observations),
            "events" => Ok(Self::Events),
            "both" => Ok(Self::Both),
            other => Err(mcp_error(
                error_codes::REPLAY_TARGET_INVALID,
                format!(
                    "replay_record target must be one of observations, events, or both; got {other:?}"
                ),
            )),
        }
    }

    const fn includes_observations(self) -> bool {
        matches!(self, Self::Observations | Self::Both)
    }

    const fn includes_events(self) -> bool {
        matches!(self, Self::Events | Self::Both)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ReplayFormat {
    Jsonl,
}

impl ReplayFormat {
    fn parse(value: &str) -> Result<Self, ErrorData> {
        match value.trim() {
            DEFAULT_FORMAT => Ok(Self::Jsonl),
            other => Err(mcp_error(
                error_codes::REPLAY_FORMAT_INVALID,
                format!("replay_record format must be jsonl; got {other:?}"),
            )),
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "target", rename_all = "snake_case")]
enum ReplayRecordLine<'a> {
    Observation { record: &'a Observation },
    Event { record: &'a Event },
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use serde_json::json;
    use synapse_core::{EventSource, ForegroundContext, Observation, Rect, SensorStatus};
    use synapse_perception::ObservationInput;

    use crate::m1::M1State;

    use super::*;

    #[tokio::test]
    async fn events_target_records_published_bus_events() -> anyhow::Result<()> {
        let path = replay_test_path("events");
        let _ = std::fs::remove_file(&path);
        let sse_state = SseState::from_env();
        let publisher = sse_state.event_bus();
        let params = ReplayRecordParams {
            target: "events".to_owned(),
            format: "jsonl".to_owned(),
            duration_ms: 250,
            path: Some(path.display().to_string()),
        };
        let m1_state = Arc::new(Mutex::new(M1State::default()));
        let event = Event {
            seq: 324_001,
            at: Utc::now(),
            source: EventSource::System,
            kind: "support.replay_record".to_owned(),
            data: json!({"known": "event-target"}),
            correlations: Vec::new(),
        };

        let (response, report) =
            tokio::join!(record_replay(m1_state, sse_state, &params), async move {
                sleep(Duration::from_millis(50)).await;
                publisher.publish(event)
            });
        let response =
            response.map_err(|error| anyhow::anyhow!("record_replay failed: {error:?}"))?;
        assert_eq!(report.matched, 1);
        assert_eq!(report.queued, 1);
        assert_eq!(response.records_written, 1);

        let replay_text = std::fs::read_to_string(&path)?;
        let events = replay_text
            .lines()
            .map(serde_json::from_str::<Event>)
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].seq, 324_001);
        assert_eq!(events[0].data["known"], "event-target");
        std::fs::remove_file(&path)?;
        Ok(())
    }

    #[tokio::test]
    async fn observations_target_skips_transient_no_perception_and_continues() -> anyhow::Result<()>
    {
        let path = replay_test_path("transient-observations");
        let _ = std::fs::remove_file(&path);
        let sse_state = SseState::from_env();
        let params = ReplayRecordParams {
            target: "observations".to_owned(),
            format: "jsonl".to_owned(),
            duration_ms: 600,
            path: Some(path.display().to_string()),
        };
        let state = M1State {
            synthetic: Some(observation_input()),
            force_no_perception: true,
            ..M1State::default()
        };
        let m1_state = Arc::new(Mutex::new(state));
        let toggled = Arc::clone(&m1_state);
        let restore_perception = tokio::spawn(async move {
            sleep(Duration::from_millis(40)).await;
            toggled
                .lock()
                .map_err(|_| anyhow::anyhow!("test M1 state lock poisoned"))?
                .force_no_perception = false;
            anyhow::Ok(())
        });

        let response = record_replay(m1_state, sse_state, &params)
            .await
            .map_err(|error| anyhow::anyhow!("record_replay failed: {error:?}"))?;
        restore_perception.await??;

        assert!(response.records_written >= 1);
        assert!(response.observations_skipped >= 1);

        let replay_text = std::fs::read_to_string(&path)?;
        let observations = replay_text
            .lines()
            .map(serde_json::from_str::<Observation>)
            .collect::<Result<Vec<_>, _>>()?;
        assert!(!observations.is_empty());
        assert_eq!(observations[0].foreground.process_name, "notepad.exe");
        std::fs::remove_file(&path)?;
        Ok(())
    }

    #[tokio::test]
    async fn observations_target_errors_when_every_sample_is_unavailable() -> anyhow::Result<()> {
        let path = replay_test_path("unavailable-observations");
        let _ = std::fs::remove_file(&path);
        let sse_state = SseState::from_env();
        let params = ReplayRecordParams {
            target: "observations".to_owned(),
            format: "jsonl".to_owned(),
            duration_ms: 60,
            path: Some(path.display().to_string()),
        };
        let state = M1State {
            synthetic: Some(observation_input()),
            force_no_perception: true,
            ..M1State::default()
        };
        let m1_state = Arc::new(Mutex::new(state));

        let error = match record_replay(m1_state, sse_state, &params).await {
            Ok(response) => {
                anyhow::bail!("sustained no-perception replay unexpectedly succeeded: {response:?}")
            }
            Err(error) => error,
        };
        assert_eq!(
            error_data_code(&error),
            Some(error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE)
        );
        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    fn observation_input() -> ObservationInput {
        let mut input = ObservationInput::new(ForegroundContext {
            hwnd: 100,
            pid: 200,
            process_name: "notepad.exe".to_owned(),
            process_path: "C:\\Windows\\System32\\notepad.exe".to_owned(),
            window_title: "transient.txt - Notepad".to_owned(),
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
        });
        input.a11y_status = SensorStatus::Healthy;
        input
    }

    fn error_data_code(error: &ErrorData) -> Option<&str> {
        error.data.as_ref()?.get("code")?.as_str()
    }

    fn replay_test_path(prefix: &str) -> PathBuf {
        replay_root().join(format!(
            "{prefix}-{}.jsonl",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }
}
