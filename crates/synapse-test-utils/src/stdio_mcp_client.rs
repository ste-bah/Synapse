use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, bail};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Minimal raw JSON-RPC stdio client for M0 MCP full-state verification.
pub struct StdioMcpClient {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Lines<BufReader<ChildStdout>>,
    stderr_task: Option<tokio::task::JoinHandle<Vec<u8>>>,
    next_id: u64,
    raw_rx: Vec<String>,
    raw_tx: Vec<String>,
}

impl StdioMcpClient {
    pub async fn launch_and_init() -> anyhow::Result<Self> {
        Self::launch_and_init_with_log_dir(None).await
    }

    pub async fn launch_and_init_with_log_dir(log_dir: Option<&Path>) -> anyhow::Result<Self> {
        let mut client = Self::launch(log_dir)?;
        let init = client
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "synapse-test-utils",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
            .await?;
        if init["serverInfo"]["name"] != "synapse-mcp" {
            bail!("unexpected initialize response: {init}");
        }
        client
            .notify("notifications/initialized", serde_json::json!({}))
            .await?;
        Ok(client)
    }

    pub fn launch(log_dir: Option<&Path>) -> anyhow::Result<Self> {
        let bin = mcp_binary_path()?;
        let mut command = Command::new(bin);
        command
            .args(["--mode", "stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("SYNAPSE_LOG_LEVEL", "debug");
        if let Some(log_dir) = log_dir {
            command.env("SYNAPSE_LOG_DIR", log_dir);
        }

        let mut child = command.spawn().context("spawn synapse-mcp")?;
        let stdin = child.stdin.take().context("child stdin missing")?;
        let stdout = child.stdout.take().context("child stdout missing")?;
        let stderr = child.stderr.take().context("child stderr missing")?;
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout: BufReader::new(stdout).lines(),
            stderr_task: Some(tokio::spawn(read_stderr(stderr))),
            next_id: 0,
            raw_rx: Vec::new(),
            raw_tx: Vec::new(),
        })
    }

    pub async fn tools_list(&mut self) -> anyhow::Result<Value> {
        self.request("tools/list", serde_json::json!({})).await
    }

    pub async fn tools_call(&mut self, name: &str, args: Value) -> anyhow::Result<Value> {
        self.request(
            "tools/call",
            serde_json::json!({
                "name": name,
                "arguments": args,
            }),
        )
        .await
    }

    pub async fn request(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.next_id = self.next_id.saturating_add(1);
        let id = self.next_id;
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_message(&request).await?;

        let response = tokio::time::timeout(REQUEST_TIMEOUT, self.read_response(id))
            .await
            .context("timed out waiting for JSON-RPC response")??;
        if let Some(error) = response.get("error") {
            bail!("JSON-RPC error from {method}: {error}");
        }
        response
            .get("result")
            .cloned()
            .with_context(|| format!("JSON-RPC response missing result: {response}"))
    }

    pub async fn notify(&mut self, method: &str, params: Value) -> anyhow::Result<()> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_message(&notification).await
    }

    pub async fn shutdown(mut self) -> anyhow::Result<std::process::ExitStatus> {
        drop(self.stdin.take());
        let mut child = self.child.take().context("child already reaped")?;
        let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .context("timed out waiting for child shutdown")?
            .context("wait for child")?;

        if let Some(stderr_task) = self.stderr_task.take() {
            let _stderr = stderr_task.await.context("join stderr reader")?;
        }
        Ok(status)
    }

    #[must_use]
    pub fn raw_received(&self) -> &[String] {
        &self.raw_rx
    }

    async fn write_message(&mut self, value: &Value) -> anyhow::Result<()> {
        let line = serde_json::to_string(value)?;
        self.raw_tx.push(line.clone());
        let stdin = self.stdin.as_mut().context("child stdin closed")?;
        stdin.write_all(line.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn read_response(&mut self, id: u64) -> anyhow::Result<Value> {
        while let Some(line) = self.stdout.next_line().await? {
            self.raw_rx.push(line.clone());
            let value: Value = serde_json::from_str(&line)
                .with_context(|| format!("parse JSON-RPC line: {line}"))?;
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(value);
            }
        }
        bail!("MCP_TRANSPORT_CLOSED before response id {id}");
    }
}

impl Drop for StdioMcpClient {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
    }
}

async fn read_stderr(mut stderr: ChildStderr) -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = stderr.read_to_end(&mut buf).await;
    buf
}

fn mcp_binary_path() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("SYNAPSE_MCP_BIN") {
        return Ok(PathBuf::from(path));
    }
    std::env::var_os("CARGO_BIN_EXE_synapse-mcp")
        .map(PathBuf::from)
        .context("CARGO_BIN_EXE_synapse-mcp is unset; run from synapse-mcp integration tests or set SYNAPSE_MCP_BIN")
}
