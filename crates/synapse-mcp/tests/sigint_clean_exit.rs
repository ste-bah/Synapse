#![cfg(unix)]

use std::{process::Stdio, time::Duration};

use anyhow::Context;
use tempfile::TempDir;
use tokio::process::Command;

#[tokio::test]
async fn synthetic_sigint_results_in_exit_0_and_flushed_log() -> anyhow::Result<()> {
    let dir = TempDir::new()?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_synapse-mcp"))
        .args(["--mode", "stdio"])
        .env("SYNAPSE_LOG_LEVEL", "debug")
        .env("SYNAPSE_LOG_DIR", dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    tokio::time::sleep(Duration::from_millis(500)).await;
    let pid = child.id().context("child pid missing")?;
    let kill_status = Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .await?;
    assert!(kill_status.success());

    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .context("timed out waiting for sigint exit")??;
    assert_eq!(status.code(), Some(0));

    let logs = read_logs(dir.path())?;
    assert!(logs.contains("MCP_SHUTDOWN_GRACEFUL"));
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
