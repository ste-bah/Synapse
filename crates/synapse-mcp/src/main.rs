#![allow(
    clippy::borrow_as_ptr,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::ignored_unit_patterns,
    clippy::implicit_clone,
    clippy::items_after_statements,
    clippy::manual_clamp,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::needless_return,
    clippy::nonminimal_bool,
    clippy::option_if_let_else,
    clippy::question_mark,
    clippy::redundant_closure_for_method_calls,
    clippy::redundant_pub_crate,
    clippy::ref_option,
    clippy::significant_drop_tightening,
    clippy::similar_names,
    clippy::single_match_else,
    clippy::too_many_lines,
    clippy::unnecessary_wraps,
    clippy::unused_async,
    reason = "synapse-mcp keeps pedantic/nursery style lint debt explicit while using clippy -D warnings for behavior-level regressions"
)]
#![cfg_attr(
    test,
    allow(
        clippy::bool_assert_comparison,
        clippy::expect_used,
        clippy::float_cmp,
        clippy::items_after_test_module,
        clippy::manual_let_else,
        clippy::needless_raw_string_hashes,
        clippy::redundant_clone,
        clippy::unreadable_literal,
        clippy::unwrap_used
    )
)]
mod connect;
mod daemon_lifecycle;
mod doctor;
mod http;
mod m1;
mod m2;
mod m3;
mod m4;
mod safety;
mod server;
mod single_instance;
mod stdio_eof;
#[cfg(test)]
mod test_support;

use std::{num::NonZeroUsize, path::PathBuf, process::ExitCode, time::Duration};

use anyhow::Context;
use clap::{ArgAction, Parser, ValueEnum};
use rmcp::ServiceExt;
use synapse_telemetry::{TelemetryConfig, TelemetryGuard, init_tracing};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::filter::LevelFilter;

use crate::server::SynapseService;
use crate::stdio_eof::CancelOnEofRead;

const ALLOW_SHELL_ENV: &str = "SYNAPSE_ALLOW_SHELL";
const ALLOW_LAUNCH_ENV: &str = "SYNAPSE_ALLOW_LAUNCH";

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Mode {
    Stdio,
    Http,
    /// Thin stdio<->HTTP bridge to the shared daemon (for stdio-only clients).
    Connect,
    /// Enumerate/classify synapse-mcp processes; with --kill-stray, clean them.
    Doctor,
}

#[derive(Debug, Parser)]
#[command(name = "synapse-mcp", version, about = "Synapse MCP daemon")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "CLI flags intentionally mirror independent operator startup gates"
)]
struct Cli {
    #[arg(long, value_enum, default_value_t = Mode::Stdio, env = "SYNAPSE_MODE")]
    mode: Mode,
    #[arg(long, default_value = "127.0.0.1:7700", env = "SYNAPSE_BIND")]
    bind: String,
    #[arg(long, env = "SYNAPSE_ALLOW_NON_LOOPBACK")]
    allow_non_loopback: bool,
    #[arg(long, env = "SYNAPSE_DB")]
    db: Option<PathBuf>,
    #[arg(long, env = "SYNAPSE_PROFILE_DIR")]
    profile_dir: Option<PathBuf>,
    #[arg(long, env = "SYNAPSE_LOG_LEVEL", default_value = "info")]
    log_level: String,
    #[arg(long, env = "SYNAPSE_REFLEX_DISABLED")]
    reflex_disabled: bool,
    /// In `--mode doctor`, kill matching stray synapse-mcp processes once a
    /// live lock-holder daemon is identified for the selected DB path.
    #[arg(long)]
    kill_stray: bool,
    #[arg(long, env = "SYNAPSE_ENABLE_AUDIO")]
    enable_audio: bool,
    /// Restrict action dispatch to reviewed profiles. Off by default: Synapse
    /// is general Windows computer-control, so unknown/unprofiled foreground
    /// apps are actionable out of the box. Set to fail closed on unknown scope.
    #[arg(long, env = "SYNAPSE_RESTRICT_UNKNOWN_PROFILE")]
    restrict_unknown_profile: bool,
    #[arg(long, env = "SYNAPSE_MCP_ALLOWED_PERMISSIONS", value_name = "LIST")]
    allowed_permissions: Option<String>,
    #[arg(long, env = "SYNAPSE_REFLEX_FORCE_DEGRADED")]
    reflex_force_degraded: bool,
    #[arg(
        long,
        env = "SYNAPSE_STORAGE_PRESSURE_FREE_BYTES_SAMPLE",
        value_name = "BYTES"
    )]
    storage_pressure_free_bytes_sample: Option<u64>,
    #[arg(
        long,
        env = "SYNAPSE_MAX_SUBSCRIPTIONS",
        default_value_t = synapse_reflex::DEFAULT_MAX_SUBSCRIPTIONS_NONZERO,
        value_name = "COUNT"
    )]
    max_subscriptions: NonZeroUsize,
    #[arg(
        long,
        value_name = "REGEX",
        action = ArgAction::Append,
        help = "Allow act_run_shell command-line regex; repeat for multiple entries. Env: SYNAPSE_ALLOW_SHELL comma-separated"
    )]
    allow_shell: Vec<String>,
    #[arg(
        long,
        value_name = "REGEX",
        action = ArgAction::Append,
        help = "Allow act_launch target regex; repeat for multiple entries. Env: SYNAPSE_ALLOW_LAUNCH comma-separated"
    )]
    allow_launch: Vec<String>,
    #[arg(
        long,
        env = "SYNAPSE_RUN_SHELL_INLINE_AWAIT_LIMIT_MS",
        default_value_t = m4::DEFAULT_RUN_SHELL_INLINE_AWAIT_LIMIT_MS,
        value_name = "MILLISECONDS",
        help = "Inline await budget for act_run_shell before it returns a durable job handle. Set 0 to background every direct shell request."
    )]
    run_shell_inline_await_limit_ms: u64,
}

impl Cli {
    fn m2_config() -> m2::M2ServiceConfig {
        m2::M2ServiceConfig::from_env()
    }

    fn m3_config(&self) -> m3::M3ServiceConfig {
        m3::M3ServiceConfig::from_cli_parts(
            self.db.clone(),
            self.profile_dir.clone(),
            self.reflex_disabled,
            self.bind.clone(),
            self.max_subscriptions,
            self.enable_audio,
            !self.restrict_unknown_profile,
            self.allowed_permissions.clone(),
            self.reflex_force_degraded,
            self.storage_pressure_free_bytes_sample,
        )
    }

    fn m4_config(&self) -> anyhow::Result<m4::M4ServiceConfig> {
        let mut allow_shell = parse_env_list(ALLOW_SHELL_ENV);
        allow_shell.extend(self.allow_shell.clone());
        let mut allow_launch = parse_env_list(ALLOW_LAUNCH_ENV);
        allow_launch.extend(self.allow_launch.clone());
        m4::M4ServiceConfig::from_cli_parts(
            allow_shell,
            allow_launch,
            self.run_shell_inline_await_limit_ms,
        )
    }
}

fn parse_env_list(name: &str) -> Vec<String> {
    std::env::var(name)
        .map(|raw| raw.split(',').map(ToOwned::to_owned).collect())
        .unwrap_or_default()
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(err) => {
            if let Err(lifecycle_error) =
                daemon_lifecycle::record_top_level_error(&format!("{err:#}"))
            {
                eprintln!("synapse-mcp lifecycle error: {lifecycle_error:#}");
            }
            eprintln!("synapse-mcp error: {err:#}");
            ExitCode::from(1)
        }
    }
}

async fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    let telemetry_guard = configure_telemetry(&cli)?;

    // The connect bridge is a thin stdio<->HTTP proxy; it does not initialize
    // perception/action/storage, so return before the daemon-only setup below.
    if matches!(cli.mode, Mode::Connect) {
        let result = connect::run_connect(&cli.bind, cli.db.as_deref()).await;
        drop(telemetry_guard);
        return result;
    }
    if matches!(cli.mode, Mode::Doctor) {
        let code = doctor::run_doctor(cli.kill_stray, cli.db.as_deref());
        drop(telemetry_guard);
        return Ok(code);
    }

    let dpi_awareness = synapse_capture::init_process_dpi_awareness()
        .context("initialize per-monitor DPI awareness")?;
    tracing::info!(?cli, code = "MCP_CLI_PARSED", "synapse-mcp cli parsed");
    tracing::info!(
        ?dpi_awareness,
        code = "CAPTURE_DPI_AWARENESS_INITIALIZED",
        "capture dpi awareness initialized"
    );
    let recovery_file = synapse_action::configure_crash_recovery_file(cli.db.as_deref())
        .context("configure action crash recovery ledger")?;
    let recovery_report = synapse_action::recover_stale_inputs_from_configured_path()
        .context("recover stale action inputs from previous daemon")?;
    tracing::info!(
        code = "ACTION_CRASH_RECOVERY_CONFIGURED",
        recovery_file = %recovery_file.display(),
        recovered_keys = recovery_report.recovered_keys,
        recovered_buttons = recovery_report.recovered_buttons,
        recovered_pads = recovery_report.recovered_pads,
        ignored_trailing_bytes = recovery_report.ignored_trailing_bytes,
        "action crash recovery ledger configured"
    );

    let m2_config = Cli::m2_config();
    let m3_config = cli.m3_config();
    let m4_config = match cli.m4_config() {
        Ok(config) => config,
        Err(error) => {
            if let Some(broad_pattern) = error.downcast_ref::<m4::BroadAllowPatternError>() {
                tracing::error!(
                    event = "CONFIG_INVALID",
                    code = broad_pattern.code(),
                    tool = broad_pattern.tool_name(),
                    source = broad_pattern.source_name(),
                    pattern = broad_pattern.raw(),
                    reason = broad_pattern.reason(),
                    "CONFIG_INVALID code={}",
                    broad_pattern.code()
                );
                eprintln!("synapse-mcp error: {error:#}");
                drop(telemetry_guard);
                return Ok(ExitCode::from(2));
            }
            return Err(error);
        }
    };

    match cli.mode {
        Mode::Stdio => run_stdio(telemetry_guard, &m2_config, m3_config, m4_config).await,
        Mode::Http => {
            let code = http::serve(
                &cli.bind,
                cli.allow_non_loopback,
                &m2_config,
                m3_config,
                m4_config,
            )
            .await?;
            drop(telemetry_guard);
            Ok(code)
        }
        Mode::Connect | Mode::Doctor => {
            unreachable!("connect and doctor modes are handled before daemon setup")
        }
    }
}

fn configure_telemetry(cli: &Cli) -> anyhow::Result<TelemetryGuard> {
    let level = cli
        .log_level
        .parse::<LevelFilter>()
        .with_context(|| format!("invalid log level {}", cli.log_level))?;
    let log_dir = std::env::var_os("SYNAPSE_LOG_DIR").map(PathBuf::from);
    init_tracing(TelemetryConfig {
        log_dir,
        file_level: level,
        console_level: level,
        ..TelemetryConfig::default()
    })
    .context("initialize telemetry")
}

async fn run_stdio(
    telemetry_guard: TelemetryGuard,
    m2_config: &m2::M2ServiceConfig,
    m3_config: m3::M3ServiceConfig,
    m4_config: m4::M4ServiceConfig,
) -> anyhow::Result<ExitCode> {
    tracing::info!(code = "MCP_STDIO_STARTED", "starting stdio MCP transport");

    // Single-instance guard (epic #717 / single-daemon invariant): an embedded
    // stdio daemon is a FULL daemon: it opens RocksDB and owns its own
    // process-global input lease + per-session registries. Without this guard a
    // stray or misconfigured stdio launch would run a SECOND parallel daemon
    // whose lease/state cannot coordinate with the canonical HTTP daemon, which
    // silently breaks multi-agent isolation. The HTTP path already acquires this
    // lock before binding the port; the stdio path must obey the same rule.
    // Fail loud, naming the current holder, and point the operator at --mode
    // connect (the supported way for a stdio-only client to reach the shared
    // daemon) instead of crashing later on a cryptic RocksDB LOCK error.
    let db_path = m3_config
        .db_path
        .clone()
        .unwrap_or_else(crate::m3::default_db_path);
    let _single_instance = match crate::single_instance::SingleInstanceGuard::acquire(&db_path) {
        Ok(guard) => {
            tracing::info!(
                code = "MCP_DAEMON_SINGLE_INSTANCE_ACQUIRED",
                lock_path = %guard.lock_path().display(),
                db_path = %db_path.display(),
                pid = std::process::id(),
                mode = "stdio",
                "daemon single-instance lock acquired"
            );
            guard
        }
        Err(crate::single_instance::SingleInstanceError::AlreadyRunning {
            lock_path,
            holder_pid,
        }) => {
            let holder = holder_pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string());
            tracing::error!(
                code = "MCP_DAEMON_ALREADY_RUNNING",
                lock_path = %lock_path.display(),
                holder_pid = %holder,
                db_path = %db_path.display(),
                mode = "stdio",
                "refusing to start: another synapse-mcp daemon already owns this DB path"
            );
            eprintln!(
                "synapse-mcp error: another synapse-mcp daemon already owns {} (holder pid {holder}); use --mode connect to reach the shared daemon instead of starting a second one",
                db_path.display()
            );
            drop(telemetry_guard);
            return Ok(ExitCode::from(3));
        }
        Err(err @ crate::single_instance::SingleInstanceError::Io { .. }) => {
            return Err(anyhow::Error::new(err)).context("acquire daemon single-instance lock");
        }
    };

    let lifecycle_paths = daemon_lifecycle::configure(daemon_lifecycle::DaemonLifecycleConfig {
        mode: "stdio",
        bind_addr: None,
        db_path: db_path.clone(),
    })
    .context("configure daemon lifecycle ledger")?;
    daemon_lifecycle::install_panic_hook();
    tracing::info!(
        code = "MCP_DAEMON_LIFECYCLE_READY",
        run_current_path = %lifecycle_paths.run_current_path,
        tool_last_path = %lifecycle_paths.tool_last_path,
        tool_events_path = %lifecycle_paths.tool_events_path,
        exit_events_path = %lifecycle_paths.exit_events_path,
        "daemon lifecycle ledger ready"
    );

    let rmcp_token = CancellationToken::new();
    let emitter_shutdown_token = CancellationToken::new();
    let emitter_connection_closed_token = CancellationToken::new();
    let service = SynapseService::try_with_m2_shutdown_reason_and_m3_config(
        emitter_shutdown_token.clone(),
        "sigint",
        emitter_connection_closed_token.clone(),
        m2_config,
        m3_config,
        m4_config,
    )
    .context("initialize Synapse service state")?;
    synapse_action::install_panic_hook();
    let _operator_hotkey_guard = safety::install_operator_hotkey(service.m3_state_handle())
        .context("install operator panic hotkey")?;
    let m2_emitter_done = service.m2_emitter_done_receiver();
    let (stdin, stdout) = rmcp::transport::stdio();
    let stdin = CancelOnEofRead::new(
        stdin,
        emitter_connection_closed_token.clone(),
        rmcp_token.clone(),
        "MCP_STDIO_EOF_CONNECTION_CLOSED",
        "stdio",
    );
    let start = service.serve_with_ct((stdin, stdout), rmcp_token.clone());
    tokio::pin!(start);
    let service = tokio::select! {
        service = &mut start => match service {
            Ok(service) => service,
            Err(err) if err.to_string().contains("connection closed") => {
                tracing::info!(code = "MCP_STDIO_CLOSED_BEFORE_INIT", "stdio closed before init");
                daemon_lifecycle::record_graceful_exit("stdio_closed_before_init")
                    .context("record daemon lifecycle graceful stdio close before init")?;
                drop(telemetry_guard);
                return Ok(ExitCode::SUCCESS);
            }
            Err(err) => return Err(err).context("start rmcp stdio service"),
        },
        signal = wait_for_shutdown_signal("during startup") => {
            signal?;
            rmcp_token.cancel();
            emitter_shutdown_token.cancel();
            tracing::info!(code = "MCP_SHUTDOWN_GRACEFUL", "shutdown signal received before init");
            daemon_lifecycle::record_graceful_exit("stdio_signal_before_init")
                .context("record daemon lifecycle graceful stdio shutdown before init")?;
            drop(telemetry_guard);
            std::process::exit(0);
        }
    };
    let shutdown = service.cancellation_token();
    let mut wait_task = tokio::spawn(async move { service.waiting().await });

    let code = tokio::select! {
        wait = &mut wait_task => {
            wait.context("join rmcp service")??;
            emitter_connection_closed_token.cancel();
            wait_for_m2_emitter_done(m2_emitter_done).await;
            ExitCode::SUCCESS
        }
        signal = wait_for_shutdown_signal("after init") => {
            signal?;
            tracing::info!(code = "MCP_SHUTDOWN_GRACEFUL", "shutdown signal received");
            emitter_shutdown_token.cancel();
            shutdown.cancel();
            wait_for_m2_emitter_done(m2_emitter_done).await;
            wait_task.abort();
            daemon_lifecycle::record_graceful_exit("stdio_signal_after_init")
                .context("record daemon lifecycle graceful stdio shutdown after init")?;
            drop(telemetry_guard);
            std::process::exit(0);
        }
    };

    daemon_lifecycle::record_graceful_exit("stdio_service_completed")
        .context("record daemon lifecycle graceful stdio service completion")?;
    drop(telemetry_guard);
    Ok(code)
}

#[cfg(windows)]
async fn wait_for_shutdown_signal(phase: &'static str) -> anyhow::Result<()> {
    let mut ctrl_break = tokio::signal::windows::ctrl_break()
        .with_context(|| format!("register ctrl-break handler {phase}"))?;
    tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            signal.with_context(|| format!("wait for ctrl-c {phase}"))?;
        }
        received = ctrl_break.recv() => {
            if received.is_none() {
                anyhow::bail!("ctrl-break stream ended while waiting for shutdown signal {phase}");
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
async fn wait_for_shutdown_signal(phase: &'static str) -> anyhow::Result<()> {
    tokio::signal::ctrl_c()
        .await
        .with_context(|| format!("wait for ctrl-c {phase}"))
}

async fn wait_for_m2_emitter_done(
    done: Option<tokio::sync::watch::Receiver<Option<synapse_action::ActionStateSnapshot>>>,
) {
    let Some(mut done) = done else {
        return;
    };
    let _wait_result = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if done.borrow().is_some() {
                break;
            }
            if done.changed().await.is_err() {
                break;
            }
        }
    })
    .await;
}
