//! `--mode connect`: native stdio<->HTTP bridge to the shared Synapse daemon.
//!
//! Lets a stdio-only MCP client (Claude Desktop, Codex) reach the single shared
//! HTTP daemon instead of spawning its own embedded server (which would contend
//! for the one RocksDB lock). The bridge is a transport-level pump: it forwards
//! raw JSON-RPC between the client's stdio transport and an rmcp
//! Streamable-HTTP client transport pointed at the daemon, so the initialize
//! handshake, `Mcp-Session-Id` sessions, and SSE server->client notifications
//! are all handled by rmcp's client worker. No message interpretation, no
//! external proxy dependency.

use std::{path::Path, process::ExitCode, time::Duration};

use anyhow::Context;
use rmcp::{
    model::ClientJsonRpcMessage,
    transport::{
        Transport,
        async_rw::AsyncRwTransport,
        streamable_http_client::{
            StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
        },
    },
};
use tokio_util::sync::CancellationToken;

use crate::stdio_eof::CancelOnEofRead;

/// How long to wait for a freshly spawned daemon to become healthy.
const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(15);
const DAEMON_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Arm a watchdog that exits the bridge if its parent (the MCP client) dies.
///
/// stdin EOF is the normal shutdown path, but on Windows an abrupt parent death
/// does not always deliver EOF to an inherited stdin (the original orphan
/// failure mode). This watchdog waits on the parent process handle and force
/// exits when it dies, so a bridge can never outlive its client. The shared
/// daemon is intentionally NOT subject to this — it must survive client churn.
fn install_parent_watchdog() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let Some(parent) = parent_process_info() else {
            tracing::error!(
                code = "MCP_CONNECT_PARENT_UNKNOWN",
                "could not determine parent process; refusing bridge without lifecycle owner"
            );
            anyhow::bail!(
                "MCP_CONNECT_PARENT_UNKNOWN: could not determine parent process; refusing bridge without lifecycle owner"
            );
        };
        if parent.is_unsupported_wsl_interop_host() {
            tracing::error!(
                code = "MCP_CONNECT_UNSUPPORTED_PARENT",
                parent_pid = parent.pid,
                parent_name = %parent.name,
                parent_command_line_len = parent.command_line.len(),
                parent_command_line_mentions_wsl = parent.command_line.to_ascii_lowercase().contains("\\wsl.exe"),
                remediation = "configure WSL clients for HTTP MCP transport or launch through a supported wrapper; direct WSL interop cannot prove client lifetime",
                "refusing direct WSL interop bridge parent"
            );
            anyhow::bail!(
                "MCP_CONNECT_UNSUPPORTED_PARENT: parent_pid={} parent_name={} direct WSL interop cannot prove client lifetime; configure WSL clients for HTTP MCP transport or a supported launcher",
                parent.pid,
                parent.name
            );
        };
        verify_parent_watchdog_handle(parent.pid)?;
        let parent_pid = parent.pid;
        std::thread::spawn(move || {
            use windows::Win32::Foundation::CloseHandle;
            use windows::Win32::System::Threading::{
                OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
            };
            // SAFETY: OpenProcess with SYNCHRONIZE, then block on the handle.
            unsafe {
                match OpenProcess(PROCESS_SYNCHRONIZE, false, parent_pid) {
                    Ok(handle) => {
                        WaitForSingleObject(handle, u32::MAX);
                        let _ = CloseHandle(handle);
                    }
                    Err(error) => {
                        tracing::error!(
                            code = "MCP_CONNECT_PARENT_OPEN_FAILED",
                            parent_pid,
                            error = %error,
                            "could not open parent process inside watchdog; bridge shutting down"
                        );
                        std::process::exit(1);
                    }
                }
            }
            tracing::warn!(
                code = "MCP_CONNECT_PARENT_EXITED",
                parent_pid,
                "parent client process exited; bridge shutting down"
            );
            std::process::exit(0);
        });
        tracing::info!(
            code = "MCP_CONNECT_PARENT_WATCHDOG",
            parent_pid,
            "parent-death watchdog armed"
        );
    }
    Ok(())
}

#[cfg(windows)]
#[derive(Debug)]
struct ParentProcessInfo {
    pid: u32,
    name: String,
    command_line: String,
}

#[cfg(windows)]
impl ParentProcessInfo {
    fn is_unsupported_wsl_interop_host(&self) -> bool {
        self.name.eq_ignore_ascii_case("wsl.exe")
            || self.name.eq_ignore_ascii_case("wslhost.exe")
            || self.command_line.to_ascii_lowercase().contains("\\wsl.exe")
    }
}

#[cfg(windows)]
fn parent_process_info() -> Option<ParentProcessInfo> {
    use sysinfo::{ProcessesToUpdate, System, get_current_pid};
    let current = get_current_pid().ok()?;
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let parent_pid = system.process(current)?.parent()?;
    let parent = system.process(parent_pid)?;
    Some(ParentProcessInfo {
        pid: parent_pid.as_u32(),
        name: parent.name().to_string_lossy().into_owned(),
        command_line: parent
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" "),
    })
}

#[cfg(windows)]
fn verify_parent_watchdog_handle(parent_pid: u32) -> anyhow::Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};

    // SAFETY: OpenProcess is called with synchronize-only access. The handle is
    // closed immediately; the watchdog thread opens its own handle after spawn.
    unsafe {
        match OpenProcess(PROCESS_SYNCHRONIZE, false, parent_pid) {
            Ok(handle) => {
                let _ = CloseHandle(handle);
                Ok(())
            }
            Err(error) => {
                tracing::error!(
                    code = "MCP_CONNECT_PARENT_OPEN_FAILED",
                    parent_pid,
                    error = %error,
                    "could not open parent process; refusing bridge without watchdog"
                );
                anyhow::bail!(
                    "MCP_CONNECT_PARENT_OPEN_FAILED: could not open parent pid {parent_pid}: {error}"
                );
            }
        }
    }
}

/// Probe the daemon `/health` endpoint. Returns true only on a 2xx response.
async fn probe_health(bind: &str, token: &str) -> bool {
    let url = format!("http://{bind}/health");
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build()
    else {
        return false;
    };
    match client.get(&url).bearer_auth(token).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// Spawn the shared daemon detached (its own stdio = null so it never writes to
/// the bridge's MCP stdout, and it outlives the bridge). The T1 single-instance
/// guard ensures that if several bridges race to spawn, only one daemon wins.
#[cfg(not(windows))]
fn spawn_detached_daemon(bind: &str, db: Option<&Path>) -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("resolve current executable path")?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["--mode", "http", "--bind", bind]);
    if let Some(db) = db {
        cmd.arg("--db").arg(db);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn().context("spawn shared daemon process")?;
    Ok(())
}

/// Spawn the daemon on Windows with `bInheritHandles = FALSE` via
/// `CreateProcessW`. This is critical: `std::process::Command` spawns with
/// handle inheritance enabled, which would leak the stdio pipe handles
/// connecting an MCP client to this bridge into the long-lived daemon — keeping
/// those pipes open so the client could never detect the bridge exiting. With
/// inheritance disabled the detached daemon shares none of our handles.
#[cfg(windows)]
fn spawn_detached_daemon(bind: &str, db: Option<&Path>) -> anyhow::Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        CREATE_NO_WINDOW, CreateProcessW, DETACHED_PROCESS, PROCESS_INFORMATION, STARTUPINFOW,
    };
    use windows::core::{PCWSTR, PWSTR};

    let exe = std::env::current_exe().context("resolve current executable path")?;
    let mut command_line = String::new();
    command_line.push('"');
    command_line.push_str(&exe.to_string_lossy());
    command_line.push_str("\" --mode http --bind ");
    command_line.push_str(bind);
    if let Some(db) = db {
        command_line.push_str(" --db \"");
        command_line.push_str(&db.to_string_lossy());
        command_line.push('"');
    }
    let mut command_line_w: Vec<u16> = command_line
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let startup_info = STARTUPINFOW {
        cb: u32::try_from(core::mem::size_of::<STARTUPINFOW>()).unwrap_or(0),
        ..Default::default()
    };
    let mut process_info = PROCESS_INFORMATION::default();

    // SAFETY: command_line_w is a writable, NUL-terminated UTF-16 buffer kept
    // alive across the call; all optional pointers are null; bInheritHandles is
    // false so the daemon inherits none of this process's handles.
    let result = unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(command_line_w.as_mut_ptr())),
            None,
            None,
            false,
            DETACHED_PROCESS | CREATE_NO_WINDOW,
            None,
            PCWSTR::null(),
            &startup_info,
            &mut process_info,
        )
    };
    result.context("CreateProcessW for shared daemon")?;

    // SAFETY: handles from a successful CreateProcessW; we do not need them.
    unsafe {
        let _ = CloseHandle(process_info.hProcess);
        let _ = CloseHandle(process_info.hThread);
    }
    Ok(())
}

/// Ensure a shared daemon is reachable at `bind`: probe, and if absent spawn one
/// (guarded) and wait until it is healthy. Errors (no fallback) if it never
/// comes up within [`DAEMON_READY_TIMEOUT`].
async fn ensure_daemon_running(bind: &str, db: Option<&Path>, token: &str) -> anyhow::Result<()> {
    if probe_health(bind, token).await {
        tracing::info!(
            code = "MCP_CONNECT_DAEMON_PRESENT",
            bind = %bind,
            "shared daemon already running"
        );
        return Ok(());
    }
    tracing::info!(
        code = "MCP_CONNECT_DAEMON_SPAWNING",
        bind = %bind,
        "no daemon detected; spawning shared daemon"
    );
    spawn_detached_daemon(bind, db).context("spawn shared daemon")?;

    let max_attempts = (DAEMON_READY_TIMEOUT.as_millis() / DAEMON_POLL_INTERVAL.as_millis()) as u32;
    for attempt in 1..=max_attempts {
        tokio::time::sleep(DAEMON_POLL_INTERVAL).await;
        if probe_health(bind, token).await {
            tracing::info!(
                code = "MCP_CONNECT_DAEMON_READY",
                bind = %bind,
                attempts = attempt,
                "spawned daemon is healthy"
            );
            return Ok(());
        }
    }
    anyhow::bail!(
        "MCP_DAEMON_SPAWN_FAILED: shared daemon at {bind} did not become healthy within {}s after spawn",
        DAEMON_READY_TIMEOUT.as_secs()
    );
}

fn new_daemon_transport(uri: &str, token: &str) -> StreamableHttpClientTransport<reqwest::Client> {
    let config =
        StreamableHttpClientTransportConfig::with_uri(uri.to_owned()).auth_header(token.to_owned());
    StreamableHttpClientTransport::from_config(config)
}

async fn open_daemon_transport(
    bind: &str,
    uri: &str,
    db: Option<&Path>,
    token: &str,
) -> anyhow::Result<StreamableHttpClientTransport<reqwest::Client>> {
    ensure_daemon_running(bind, db, token)
        .await
        .context("ensure shared daemon is running")?;
    Ok(new_daemon_transport(uri, token))
}

async fn reconnect_daemon_transport(
    bind: &str,
    uri: &str,
    db: Option<&Path>,
    token: &str,
    saved_initialize: Option<&ClientJsonRpcMessage>,
    saved_initialized: Option<&ClientJsonRpcMessage>,
) -> anyhow::Result<StreamableHttpClientTransport<reqwest::Client>> {
    let Some(initialize_message) = saved_initialize.cloned() else {
        anyhow::bail!("MCP_CONNECT_RECONNECT_NO_INITIALIZE: cannot replay bridge handshake");
    };
    let Some(initialized_message) = saved_initialized.cloned() else {
        anyhow::bail!("MCP_CONNECT_RECONNECT_NO_INITIALIZED: cannot replay bridge handshake");
    };

    let mut daemon = open_daemon_transport(bind, uri, db, token).await?;
    daemon
        .send(initialize_message)
        .await
        .context("replay initialize to daemon after reconnect")?;
    let Some(_initialize_response) = tokio::time::timeout(DAEMON_READY_TIMEOUT, daemon.receive())
        .await
        .context("wait for replayed initialize response after daemon reconnect")?
    else {
        anyhow::bail!(
            "MCP_CONNECT_RECONNECT_INIT_EOF: daemon closed before replayed initialize response"
        );
    };
    daemon
        .send(initialized_message)
        .await
        .context("replay initialized notification to daemon after reconnect")?;
    tracing::info!(
        code = "MCP_CONNECT_DAEMON_RECONNECTED",
        bind = %bind,
        "reconnected daemon transport and replayed MCP handshake"
    );
    Ok(daemon)
}

/// Run the stdio<->HTTP bridge against the daemon listening at `bind`
/// (`host:port`). Exits 0 when the client closes stdin; daemon stream loss is
/// repaired by reopening the HTTP transport and replaying the MCP handshake.
pub async fn run_connect(bind: &str, db: Option<&Path>) -> anyhow::Result<ExitCode> {
    let uri = format!("http://{bind}/mcp");
    let token = crate::http::load_token_value().context("load daemon bearer token for bridge")?;
    tracing::info!(
        code = "MCP_CONNECT_STARTING",
        daemon_uri = %uri,
        "starting stdio<->http bridge to shared daemon"
    );

    // Arm the parent-death watchdog before anything else so the bridge can
    // never outlive the client that launched it.
    install_parent_watchdog()?;

    // Ensure exactly one shared daemon is up (spawn it if needed) before bridging.
    let mut daemon = open_daemon_transport(bind, &uri, db, &token).await?;

    let client_closed_token = CancellationToken::new();
    let (stdin, stdout) = rmcp::transport::stdio();
    let stdin = CancelOnEofRead::new(
        stdin,
        client_closed_token.clone(),
        client_closed_token.clone(),
        "MCP_CONNECT_EOF_CONNECTION_CLOSED",
        "connect",
    );
    let mut client = AsyncRwTransport::new_server(stdin, stdout);
    let mut client_message_count = 0usize;
    let mut saved_initialize: Option<ClientJsonRpcMessage> = None;
    let mut saved_initialized: Option<ClientJsonRpcMessage> = None;

    loop {
        tokio::select! {
            _ = client_closed_token.cancelled() => {
                tracing::info!(
                    code = "MCP_CONNECT_STDIN_EOF",
                    "client stdin EOF guard cancelled bridge; shutting down bridge"
                );
                break;
            }
            from_client = client.receive() => {
                match from_client {
                    Some(message) => {
                        let message_index = client_message_count;
                        client_message_count = client_message_count.saturating_add(1);
                        match message_index {
                            0 => saved_initialize = Some(message.clone()),
                            1 => saved_initialized = Some(message.clone()),
                            _ => {}
                        }

                        if let Err(error) = daemon.send(message.clone()).await {
                            tracing::warn!(
                                code = "MCP_CONNECT_CLIENT_SEND_FAILED",
                                error = %error,
                                "client->daemon send failed; attempting daemon reconnect"
                            );
                            if message_index == 0 {
                                daemon = open_daemon_transport(bind, &uri, db, &token).await?;
                                daemon
                                    .send(message)
                                    .await
                                    .context("forward initial client->daemon message after reconnect")?;
                            } else {
                                daemon = reconnect_daemon_transport(
                                    bind,
                                    &uri,
                                    db,
                                    &token,
                                    saved_initialize.as_ref(),
                                    saved_initialized.as_ref(),
                                )
                                .await
                                .context("reconnect daemon after client->daemon send failure")?;
                                if message_index != 1 {
                                    daemon
                                        .send(message)
                                        .await
                                        .context("forward client->daemon message after reconnect")?;
                                }
                            }
                        }
                    }
                    None => {
                        tracing::info!(
                            code = "MCP_CONNECT_STDIN_EOF",
                            "client closed stdin; shutting down bridge"
                        );
                        break;
                    }
                }
            }
            from_daemon = daemon.receive() => {
                match from_daemon {
                    Some(message) => client
                        .send(message)
                        .await
                        .context("forward daemon->client message")?,
                    None => {
                        tracing::warn!(
                            code = "MCP_CONNECT_DAEMON_CLOSED",
                            "daemon stream closed; attempting reconnect"
                        );
                        daemon = reconnect_daemon_transport(
                            bind,
                            &uri,
                            db,
                            &token,
                            saved_initialize.as_ref(),
                            saved_initialized.as_ref(),
                        )
                        .await
                        .context("reconnect daemon after stream close")?;
                    }
                }
            }
        }
    }

    // Bound shutdown: close() can block (e.g. HTTP session-delete, or a daemon
    // transport whose worker never initialized). Never let cleanup hang the
    // bridge — any lingering rmcp worker task is aborted when the runtime drops
    // on return.
    let _ = tokio::time::timeout(Duration::from_secs(3), daemon.close()).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), client.close()).await;
    Ok(ExitCode::SUCCESS)
}
