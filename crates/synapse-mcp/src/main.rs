mod cli;
mod http;
mod m1;
mod m2;
mod m3;
mod m4;
mod safety;
mod server;

use std::{
    io,
    num::NonZeroUsize,
    path::PathBuf,
    pin::Pin,
    process::ExitCode,
    task::{Context as TaskContext, Poll},
    time::Duration,
};

use anyhow::Context;
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use rmcp::ServiceExt;
use synapse_telemetry::{TelemetryConfig, TelemetryGuard, init_tracing};
use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::filter::LevelFilter;

use crate::server::SynapseService;

const ALLOW_SHELL_ENV: &str = "SYNAPSE_ALLOW_SHELL";
const ALLOW_LAUNCH_ENV: &str = "SYNAPSE_ALLOW_LAUNCH";

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Mode {
    Stdio,
    Http,
}

#[derive(Debug, Parser)]
#[command(name = "synapse-mcp", version, about = "Synapse MCP daemon")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "CLI flags intentionally mirror independent operator startup gates"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
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
    #[arg(long, env = "SYNAPSE_ENABLE_AUDIO")]
    enable_audio: bool,
    #[arg(long, env = "SYNAPSE_ALLOW_UNKNOWN_PROFILE")]
    allow_unknown_profile: bool,
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
    #[arg(long, env = "SYNAPSE_HARDWARE_HID", value_name = "PORT_OR_AUTO")]
    hardware_hid: Option<String>,
    #[arg(long)]
    reset_hardware_consent: bool,
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
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    Hid(cli::hid::HidCli),
}

impl CliCommand {
    fn run(self) -> anyhow::Result<ExitCode> {
        match self {
            Self::Hid(command) => command.run(),
        }
    }
}

impl Cli {
    fn m2_config(&self) -> m2::M2ServiceConfig {
        m2::M2ServiceConfig::from_cli_parts(self.hardware_hid.clone())
    }

    fn m3_config(&self) -> m3::M3ServiceConfig {
        m3::M3ServiceConfig::from_cli_parts(
            self.db.clone(),
            self.profile_dir.clone(),
            self.reflex_disabled,
            self.bind.clone(),
            self.max_subscriptions,
            self.enable_audio,
            self.allow_unknown_profile,
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
        m4::M4ServiceConfig::from_cli_parts(allow_shell, allow_launch)
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
            eprintln!("synapse-mcp error: {err:#}");
            ExitCode::from(1)
        }
    }
}

async fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();
    if let Some(command) = cli.command {
        return command.run();
    }

    let telemetry_guard = configure_telemetry(&cli)?;
    let dpi_awareness = synapse_capture::init_process_dpi_awareness()
        .context("initialize per-monitor DPI awareness")?;
    tracing::info!(?cli, code = "MCP_CLI_PARSED", "synapse-mcp cli parsed");
    tracing::info!(
        ?dpi_awareness,
        code = "CAPTURE_DPI_AWARENESS_INITIALIZED",
        "capture dpi awareness initialized"
    );

    let m2_config = cli.m2_config();
    if let Err(error) =
        safety::agreement::ensure_hardware_hid_agreement(&m2_config, cli.reset_hardware_consent)
    {
        if error
            .downcast_ref::<safety::hardware_consent::HardwareConsentRefused>()
            .is_some()
        {
            tracing::error!(
                code = safety::hardware_consent::HardwareConsentRefused::code(),
                reason = safety::hardware_consent::HardwareConsentRefused::reason(),
                "SAFETY_PROFILE_ACTION_DENIED reason=hardware_consent_refused"
            );
            eprintln!(
                "synapse-mcp error: {} reason={}",
                safety::hardware_consent::HardwareConsentRefused::code(),
                safety::hardware_consent::HardwareConsentRefused::reason()
            );
            drop(telemetry_guard);
            return Ok(ExitCode::from(2));
        }
        return Err(error).context("ensure hardware HID safety agreement");
    }
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
    );
    let start = service.serve_with_ct((stdin, stdout), rmcp_token.clone());
    tokio::pin!(start);
    let service = tokio::select! {
        service = &mut start => match service {
            Ok(service) => service,
            Err(err) if err.to_string().contains("connection closed") => {
                tracing::info!(code = "MCP_STDIO_CLOSED_BEFORE_INIT", "stdio closed before init");
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
            drop(telemetry_guard);
            std::process::exit(0);
        }
    };

    drop(telemetry_guard);
    Ok(code)
}

struct CancelOnEofRead<R> {
    inner: R,
    connection_closed_cancel: CancellationToken,
    service_cancel: CancellationToken,
    eof_seen: bool,
}

impl<R> CancelOnEofRead<R> {
    const fn new(
        inner: R,
        connection_closed_cancel: CancellationToken,
        service_cancel: CancellationToken,
    ) -> Self {
        Self {
            inner,
            connection_closed_cancel,
            service_cancel,
            eof_seen: false,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for CancelOnEofRead<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before_len = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(&result, Poll::Ready(Ok(())))
            && buf.filled().len() == before_len
            && !self.eof_seen
        {
            self.eof_seen = true;
            self.connection_closed_cancel.cancel();
            self.service_cancel.cancel();
            tracing::info!(
                code = "MCP_STDIO_EOF_CONNECTION_CLOSED",
                "readback=stdio edge=connection_closed after=eof"
            );
        }
        result
    }
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
