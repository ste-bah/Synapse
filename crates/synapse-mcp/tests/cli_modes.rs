use std::{process::Stdio, time::Duration};

use anyhow::Context;
use tempfile::TempDir;
use tokio::process::Command;

#[tokio::test]
async fn http_mode_exits_not_yet_implemented() -> anyhow::Result<()> {
    let dir = TempDir::new()?;
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
            .args(["--mode", "http"])
            .env("SYNAPSE_LOG_DIR", dir.path())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .output(),
    )
    .await
    .context("timed out waiting for http mode")??;

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("NOT_YET_IMPLEMENTED"));
    Ok(())
}

#[tokio::test]
async fn stdio_mode_reaches_transport_path_on_closed_stdin() -> anyhow::Result<()> {
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

    let mut logs = String::new();
    for entry in std::fs::read_dir(dir.path())? {
        let entry = entry?;
        if entry.metadata()?.is_file() {
            logs.push_str(&std::fs::read_to_string(entry.path())?);
        }
    }
    assert!(logs.contains("MCP_STDIO_STARTED"));
    Ok(())
}

#[tokio::test]
async fn invalid_env_mode_exits_with_clap_error() -> anyhow::Result<()> {
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
