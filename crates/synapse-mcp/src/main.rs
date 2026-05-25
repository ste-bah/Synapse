mod http;
mod m1;
mod m2;
mod m3;
mod safety;
mod server;

use std::{
    io,
    path::PathBuf,
    pin::Pin,
    process::ExitCode,
    task::{Context as TaskContext, Poll},
    time::Duration,
};

use anyhow::Context;
use clap::{Parser, ValueEnum};
use rmcp::ServiceExt;
use synapse_telemetry::{TelemetryConfig, TelemetryGuard, init_tracing};
use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::filter::LevelFilter;

use crate::server::SynapseService;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Mode {
    Stdio,
    Http,
}

#[derive(Debug, Parser)]
#[command(name = "synapse-mcp", version, about = "Synapse MCP daemon")]
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
}

impl Cli {
    fn m3_config(&self) -> m3::M3ServiceConfig {
        m3::M3ServiceConfig::from_cli_parts(
            self.db.clone(),
            self.profile_dir.clone(),
            self.reflex_disabled,
            self.bind.clone(),
        )
    }
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
    let telemetry_guard = configure_telemetry(&cli)?;
    let dpi_awareness = synapse_capture::init_process_dpi_awareness()
        .context("initialize per-monitor DPI awareness")?;
    tracing::info!(?cli, code = "MCP_CLI_PARSED", "synapse-mcp cli parsed");
    tracing::info!(
        ?dpi_awareness,
        code = "CAPTURE_DPI_AWARENESS_INITIALIZED",
        "capture dpi awareness initialized"
    );

    match cli.mode {
        Mode::Stdio => run_stdio(telemetry_guard, cli.m3_config()).await,
        Mode::Http => {
            let code = http::serve(&cli.bind, cli.allow_non_loopback, cli.m3_config()).await?;
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
    m3_config: m3::M3ServiceConfig,
) -> anyhow::Result<ExitCode> {
    tracing::info!(code = "MCP_STDIO_STARTED", "starting stdio MCP transport");
    let rmcp_token = CancellationToken::new();
    let emitter_shutdown_token = CancellationToken::new();
    let emitter_connection_closed_token = CancellationToken::new();
    let service = SynapseService::try_with_m2_shutdown_reason_and_m3_config(
        emitter_shutdown_token.clone(),
        "sigint",
        emitter_connection_closed_token.clone(),
        m3_config,
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
