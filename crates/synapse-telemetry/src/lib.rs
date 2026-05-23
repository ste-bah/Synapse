use std::{
    env,
    fs::{self, File},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use thiserror::Error;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    EnvFilter, Layer, Registry, filter::LevelFilter, fmt, layer::SubscriberExt,
    registry::LookupSpan, util::SubscriberInitExt,
};

const DEFAULT_MAX_DIR_BYTES: u64 = 500 * 1024 * 1024;
const DEFAULT_KEEP_DAYS: u32 = 7;

#[derive(Clone, Debug)]
pub struct TelemetryConfig {
    pub log_dir: Option<PathBuf>,
    pub file_level: LevelFilter,
    pub console_level: LevelFilter,
    pub max_dir_bytes: u64,
    pub keep_days: u32,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            log_dir: None,
            file_level: LevelFilter::INFO,
            console_level: LevelFilter::INFO,
            max_dir_bytes: DEFAULT_MAX_DIR_BYTES,
            keep_days: DEFAULT_KEEP_DAYS,
        }
    }
}

impl TelemetryConfig {
    #[must_use]
    pub fn default_with_log_dir(log_dir: PathBuf) -> Self {
        Self {
            log_dir: Some(log_dir),
            ..Self::default()
        }
    }
}

#[derive(Debug, Error)]
pub enum TelemetryError {
    #[error("TELEMETRY_LOG_DIR_NOT_WRITABLE: {0}")]
    LogDirNotWritable(PathBuf),
    #[error("TELEMETRY_SUBSCRIBER_INIT_FAILED: {0}")]
    SubscriberInit(String),
    #[error("TELEMETRY_GC_FAILED: {0}")]
    Gc(String),
}

impl TelemetryError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::LogDirNotWritable(_) => "TELEMETRY_LOG_DIR_NOT_WRITABLE",
            Self::SubscriberInit(_) => "TELEMETRY_SUBSCRIBER_INIT_FAILED",
            Self::Gc(_) => "TELEMETRY_GC_FAILED",
        }
    }
}

#[derive(Debug)]
pub struct TelemetryGuard {
    _file_guard: WorkerGuard,
}

#[must_use]
pub fn json_layer<S>() -> impl Layer<S> + Send + Sync + 'static
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
{
    fmt::layer()
        .json()
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_current_span(true)
        .with_span_list(true)
}

#[must_use]
pub fn console_layer<S>() -> impl Layer<S> + Send + Sync + 'static
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
{
    fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_thread_ids(false)
        .with_ansi(false)
}

/// Initialize the process-wide tracing subscriber.
///
/// # Errors
///
/// Returns a [`TelemetryError`] when the log directory cannot be created,
/// opened, or when another global tracing subscriber is already installed.
pub fn init() -> Result<TelemetryGuard, TelemetryError> {
    init_tracing(TelemetryConfig::default())
}

/// Initialize the process-wide tracing subscriber with explicit configuration.
///
/// # Errors
///
/// Returns a [`TelemetryError`] when the log directory cannot be created,
/// opened, garbage collection fails, or when another global tracing subscriber
/// is already installed.
pub fn init_tracing(cfg: TelemetryConfig) -> Result<TelemetryGuard, TelemetryError> {
    let log_dir = cfg.log_dir.unwrap_or_else(default_log_dir);
    prepare_log_dir(&log_dir)?;
    run_log_gc(&log_dir, cfg.keep_days, cfg.max_dir_bytes).map_err(TelemetryError::Gc)?;

    let file_appender = tracing_appender::rolling::daily(&log_dir, "synapse.log");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);
    let file_layer = fmt::layer()
        .json()
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_current_span(true)
        .with_span_list(true)
        .with_writer(file_writer)
        .with_filter(cfg.file_level);

    let env_filter = EnvFilter::builder()
        .with_default_directive(cfg.console_level.into())
        .from_env_lossy();
    let console_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_thread_ids(false)
        .with_ansi(false)
        .with_filter(env_filter);

    Registry::default()
        .with(file_layer)
        .with(console_layer)
        .try_init()
        .map_err(|err| TelemetryError::SubscriberInit(err.to_string()))?;

    Ok(TelemetryGuard {
        _file_guard: file_guard,
    })
}

#[must_use]
pub fn default_log_dir() -> PathBuf {
    if cfg!(windows) {
        return env::var_os("LOCALAPPDATA")
            .map_or_else(|| PathBuf::from("."), PathBuf::from)
            .join("synapse")
            .join("logs");
    }

    env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from(".synapse-state"))
        .join("synapse")
        .join("logs")
}

fn prepare_log_dir(log_dir: &Path) -> Result<(), TelemetryError> {
    fs::create_dir_all(log_dir)
        .map_err(|_| TelemetryError::LogDirNotWritable(log_dir.to_path_buf()))?;
    let probe = log_dir.join(".synapse-write-probe");
    File::create(&probe).map_err(|_| TelemetryError::LogDirNotWritable(log_dir.to_path_buf()))?;
    fs::remove_file(probe).map_err(|_| TelemetryError::LogDirNotWritable(log_dir.to_path_buf()))
}

fn run_log_gc(log_dir: &Path, keep_days: u32, max_dir_bytes: u64) -> Result<(), String> {
    let keep = Duration::from_secs(u64::from(keep_days) * 24 * 60 * 60);
    let now = SystemTime::now();
    let mut entries = Vec::new();

    for entry in fs::read_dir(log_dir).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let metadata = entry.metadata().map_err(|err| err.to_string())?;
        if !metadata.is_file() {
            continue;
        }

        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if now.duration_since(modified).unwrap_or_default() > keep {
            fs::remove_file(entry.path()).map_err(|err| err.to_string())?;
            continue;
        }

        entries.push((entry.path(), modified, metadata.len()));
    }

    let mut total: u64 = entries.iter().map(|(_, _, len)| *len).sum();
    if total <= max_dir_bytes {
        return Ok(());
    }

    entries.sort_by_key(|(_, modified, _)| *modified);
    for (path, _, len) in entries {
        fs::remove_file(path).map_err(|err| err.to_string())?;
        total = total.saturating_sub(len);
        if total <= max_dir_bytes {
            break;
        }
    }

    Ok(())
}
