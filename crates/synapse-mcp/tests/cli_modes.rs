use std::{net::TcpListener, process::Stdio, sync::OnceLock, time::Duration};

#[cfg(windows)]
use std::process::Command as StdCommand;

use anyhow::Context;
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    process::{Child, Command},
};

#[tokio::test]
async fn http_mode_serves_health_until_shutdown() -> anyhow::Result<()> {
    let _guard = cli_mode_test_lock().lock().await;
    let dir = TempDir::new()?;
    let bind = free_loopback_bind()?;
    let token = "cli-mode-token";
    let mut child = Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
        .args(["--mode", "http", "--bind", &bind])
        .env("SYNAPSE_LOG_DIR", dir.path())
        .env("SYNAPSE_BEARER_TOKEN", token)
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let response = wait_for_health(&bind, Some(token)).await;
    let missing = read_health_once(&bind, None).await;
    let wrong = read_health_once(&bind, Some("wrong-token")).await;
    stop_child(&mut child).await?;

    let response = response?;
    let missing = missing?;
    let wrong = wrong?;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains(r#""ok":true"#), "{response}");
    assert!(
        missing.starts_with("HTTP/1.1 401 Unauthorized"),
        "{missing}"
    );
    assert!(wrong.starts_with("HTTP/1.1 401 Unauthorized"), "{wrong}");
    Ok(())
}

#[tokio::test]
async fn http_mode_refuses_non_loopback_without_flag() -> anyhow::Result<()> {
    let _guard = cli_mode_test_lock().lock().await;
    let dir = TempDir::new()?;
    let output = Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
        .args(["--mode", "http", "--bind", "0.0.0.0:0"])
        .env("SYNAPSE_LOG_DIR", dir.path())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .await?;

    assert_eq!(output.status.code(), Some(2));
    let logs = read_logs(dir.path())?;
    assert!(logs.contains("HTTP_BIND_NON_LOOPBACK_REFUSED"), "{logs}");
    assert!(logs.contains("0.0.0.0:0"), "{logs}");
    Ok(())
}

#[tokio::test]
async fn http_mode_allows_non_loopback_with_explicit_flag() -> anyhow::Result<()> {
    let _guard = cli_mode_test_lock().lock().await;
    let dir = TempDir::new()?;
    let port = free_loopback_port()?;
    let bind = format!("0.0.0.0:{port}");
    let connect = format!("127.0.0.1:{port}");
    let token = "cli-mode-non-loopback-token";
    let mut child = Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
        .args(["--mode", "http", "--bind", &bind, "--allow-non-loopback"])
        .env("SYNAPSE_LOG_DIR", dir.path())
        .env("SYNAPSE_BEARER_TOKEN", token)
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let response = wait_for_health_with_origin(&connect, Some(token), "http://127.0.0.1").await;
    stop_child(&mut child).await?;

    let response = response?;
    let logs = read_logs(dir.path())?;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        logs.contains("MCP_HTTP_NON_LOOPBACK_BIND_ALLOWED"),
        "{logs}"
    );
    assert!(logs.contains(&bind), "{logs}");
    Ok(())
}

#[tokio::test]
async fn stdio_mode_reaches_transport_path_on_closed_stdin() -> anyhow::Result<()> {
    let _guard = cli_mode_test_lock().lock().await;
    let dir = TempDir::new()?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
        .args(["--mode", "stdio"])
        .env("SYNAPSE_LOG_DIR", dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .context("timed out waiting for stdio closed-stdin exit")??;
    assert!(status.success());

    let logs = read_logs(dir.path())?;
    assert!(logs.contains("MCP_STDIO_STARTED"));
    Ok(())
}

#[tokio::test]
async fn hardware_hid_missing_literal_fails_startup_with_hid_code() -> anyhow::Result<()> {
    let _guard = cli_mode_test_lock().lock().await;
    let dir = TempDir::new()?;
    let agreement_path = dir.path().join("agreement.json");
    let mut child = Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
        .args([
            "--mode",
            "stdio",
            "--hardware-hid",
            "SYNAPSE_MISSING_PORT_393",
        ])
        .env("SYNAPSE_AGREEMENT_PATH", &agreement_path)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(b"I AUTHORIZE HARDWARE INPUT\n")
            .await
            .context("write hardware consent phrase")?;
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().await?;

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("HID_PORT_NOT_FOUND"), "{stderr}");
    assert!(stderr.contains("SYNAPSE_MISSING_PORT_393"), "{stderr}");
    #[cfg(windows)]
    if agreement_path.exists() {
        restore_test_acl_for_cleanup(&agreement_path)?;
    }
    Ok(())
}

#[tokio::test]
async fn hardware_hid_refuses_missing_or_wrong_consent_before_backend_startup() -> anyhow::Result<()>
{
    let _guard = cli_mode_test_lock().lock().await;
    let dir = TempDir::new()?;
    let agreement_path = dir.path().join("agreement.json");
    let output = Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
        .args(["--mode", "stdio", "--hardware-hid", "COM426"])
        .env("SYNAPSE_LOG_DIR", dir.path())
        .env("SYNAPSE_AGREEMENT_PATH", &agreement_path)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .await?;

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("SAFETY_PROFILE_ACTION_DENIED"), "{stderr}");
    assert!(stderr.contains("hardware_consent_refused"), "{stderr}");
    assert!(!agreement_path.exists());
    let logs = read_logs(dir.path())?;
    assert!(logs.contains("hardware_consent_refused"), "{logs}");
    Ok(())
}

#[tokio::test]
async fn help_lists_m4_policy_flags_and_envs() -> anyhow::Result<()> {
    let _guard = cli_mode_test_lock().lock().await;
    let output = Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
        .arg("--help")
        .env_remove("SYNAPSE_ALLOW_SHELL")
        .env_remove("SYNAPSE_ALLOW_LAUNCH")
        .env_remove("SYNAPSE_HARDWARE_HID")
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .await?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("--allow-shell <REGEX>"), "{stdout}");
    assert!(stdout.contains("SYNAPSE_ALLOW_SHELL"), "{stdout}");
    assert!(stdout.contains("--allow-launch <REGEX>"), "{stdout}");
    assert!(stdout.contains("SYNAPSE_ALLOW_LAUNCH"), "{stdout}");
    assert!(stdout.contains("--hardware-hid <PORT_OR_AUTO>"), "{stdout}");
    assert!(stdout.contains("SYNAPSE_HARDWARE_HID"), "{stdout}");
    assert!(stdout.contains("--reset-hardware-consent"), "{stdout}");
    Ok(())
}

#[tokio::test]
async fn http_health_reads_m4_policy_counts_from_repeated_cli_flags() -> anyhow::Result<()> {
    let _guard = cli_mode_test_lock().lock().await;
    let dir = TempDir::new()?;
    let bind = free_loopback_bind()?;
    let token = "cli-mode-m4-policy-cli-token";
    let mut child = Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
        .args([
            "--mode",
            "http",
            "--bind",
            &bind,
            "--allow-shell",
            "^echo$",
            "--allow-shell",
            "^cargo$",
            "--allow-launch",
            "^notepad\\.exe$",
            "--allow-launch",
            "^calc\\.exe$",
        ])
        .env("SYNAPSE_LOG_DIR", dir.path())
        .env("SYNAPSE_BEARER_TOKEN", token)
        .env_remove("SYNAPSE_HARDWARE_HID")
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let response = wait_for_health(&bind, Some(token)).await;
    stop_child(&mut child).await?;

    let response = response?;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        response.contains("allow_shell_patterns=2 allow_launch_patterns=2"),
        "{response}"
    );
    Ok(())
}

#[tokio::test]
async fn http_health_reads_m4_policy_counts_from_comma_env() -> anyhow::Result<()> {
    let _guard = cli_mode_test_lock().lock().await;
    let dir = TempDir::new()?;
    let bind = free_loopback_bind()?;
    let token = "cli-mode-m4-policy-env-token";
    let mut child = Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
        .args(["--mode", "http", "--bind", &bind])
        .env("SYNAPSE_LOG_DIR", dir.path())
        .env("SYNAPSE_BEARER_TOKEN", token)
        .env("SYNAPSE_ALLOW_SHELL", "^echo$,^cargo$")
        .env("SYNAPSE_ALLOW_LAUNCH", "^notepad\\.exe$,^calc\\.exe$")
        .env_remove("SYNAPSE_HARDWARE_HID")
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let response = wait_for_health(&bind, Some(token)).await;
    stop_child(&mut child).await?;

    let response = response?;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        response.contains("allow_shell_patterns=2 allow_launch_patterns=2"),
        "{response}"
    );
    Ok(())
}

#[tokio::test]
async fn invalid_env_mode_exits_with_clap_error() -> anyhow::Result<()> {
    let _guard = cli_mode_test_lock().lock().await;
    let output = Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
        .env("SYNAPSE_MODE", "garbage")
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .await?;

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("invalid value") || stderr.contains("garbage"));
    Ok(())
}

#[tokio::test]
async fn invalid_max_subscriptions_env_exits_with_clap_error() -> anyhow::Result<()> {
    let _guard = cli_mode_test_lock().lock().await;
    let output = Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
        .env("SYNAPSE_MAX_SUBSCRIPTIONS", "0")
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .await?;

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.contains("max-subscriptions") || stderr.contains("SYNAPSE_MAX_SUBSCRIPTIONS"),
        "{stderr}"
    );
    assert!(stderr.contains('0'), "{stderr}");
    Ok(())
}

fn cli_mode_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn free_loopback_bind() -> anyhow::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    drop(listener);
    Ok(addr.to_string())
}

fn free_loopback_port() -> anyhow::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

async fn wait_for_health(bind: &str, token: Option<&str>) -> anyhow::Result<String> {
    wait_for_health_inner(bind, token, None).await
}

async fn wait_for_health_with_origin(
    bind: &str,
    token: Option<&str>,
    origin: &str,
) -> anyhow::Result<String> {
    wait_for_health_inner(bind, token, Some(origin)).await
}

async fn wait_for_health_inner(
    bind: &str,
    token: Option<&str>,
    origin: Option<&str>,
) -> anyhow::Result<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match read_health_once_inner(bind, token, origin).await {
            Ok(response) => return Ok(response),
            Err(error) if tokio::time::Instant::now() < deadline => {
                let _last_error = error;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => return Err(error).context("read HTTP health from child"),
        }
    }
}

async fn read_health_once(bind: &str, token: Option<&str>) -> anyhow::Result<String> {
    read_health_once_inner(bind, token, None).await
}

async fn read_health_once_inner(
    bind: &str,
    token: Option<&str>,
    origin: Option<&str>,
) -> anyhow::Result<String> {
    let mut stream = TcpStream::connect(bind).await?;
    let auth = token.map_or(String::new(), |token| {
        format!("Authorization: Bearer {token}\r\n")
    });
    let origin = origin.map_or(String::new(), |origin| format!("Origin: {origin}\r\n"));
    let request =
        format!("GET /health HTTP/1.1\r\nHost: {bind}\r\n{auth}{origin}Connection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    String::from_utf8(response).context("decode HTTP health response")
}

async fn stop_child(child: &mut Child) -> anyhow::Result<()> {
    child.start_kill().context("stop http-mode child")?;
    tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .context("timed out waiting for http-mode child shutdown")??;
    Ok(())
}

fn read_logs(path: &std::path::Path) -> anyhow::Result<String> {
    let mut logs = String::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.metadata()?.is_file() {
            logs.push_str(&std::fs::read_to_string(entry.path())?);
        }
    }
    Ok(logs)
}

#[cfg(windows)]
fn restore_test_acl_for_cleanup(path: &std::path::Path) -> anyhow::Result<()> {
    let principal = format!(
        r"{}\{}:F",
        std::env::var("USERDOMAIN")?,
        std::env::var("USERNAME")?
    );
    let status = StdCommand::new("icacls")
        .arg(path)
        .args(["/grant", &principal])
        .status()?;
    assert!(status.success(), "icacls cleanup grant failed: {status}");
    Ok(())
}
