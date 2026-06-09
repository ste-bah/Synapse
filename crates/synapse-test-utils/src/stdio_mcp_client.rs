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

/// Minimal raw JSON-RPC stdio client for local MCP regression checks.
pub struct StdioMcpClient {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Lines<BufReader<ChildStdout>>,
    stderr_task: Option<tokio::task::JoinHandle<Vec<u8>>>,
    _temp_db_dir: Option<tempfile::TempDir>,
    next_id: u64,
    raw_rx: Vec<String>,
    raw_tx: Vec<String>,
}

impl StdioMcpClient {
    pub async fn launch_and_init() -> anyhow::Result<Self> {
        Self::launch_and_init_with_log_dir(None).await
    }

    pub async fn launch_and_init_with_log_dir(log_dir: Option<&Path>) -> anyhow::Result<Self> {
        Self::launch_with_env(log_dir, &[])?.initialize().await
    }

    pub async fn launch_and_init_with_env(
        log_dir: Option<&Path>,
        envs: &[(&str, &str)],
    ) -> anyhow::Result<Self> {
        Self::launch_with_env(log_dir, envs)?.initialize().await
    }

    async fn initialize(mut self) -> anyhow::Result<Self> {
        let init = self
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
        self.notify("notifications/initialized", serde_json::json!({}))
            .await?;
        Ok(self)
    }

    pub fn launch(log_dir: Option<&Path>) -> anyhow::Result<Self> {
        Self::launch_with_env(log_dir, &[])
    }

    pub fn launch_with_env(log_dir: Option<&Path>, envs: &[(&str, &str)]) -> anyhow::Result<Self> {
        let bin = mcp_binary_path()?;
        let caller_supplied_db = envs
            .iter()
            .any(|(key, _value)| key.eq_ignore_ascii_case("SYNAPSE_DB"));
        let temp_db_dir = if caller_supplied_db {
            None
        } else {
            Some(
                tempfile::Builder::new()
                    .prefix("synapse-stdio-db-")
                    .tempdir()?,
            )
        };
        let mut command = Command::new(bin);
        command
            .args(["--mode", "stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("SYNAPSE_MCP_DISABLE_OPERATOR_HOTKEY", "1")
            .env("SYNAPSE_LOG_LEVEL", "debug");
        if let Some(temp_db_dir) = temp_db_dir.as_ref() {
            command.env("SYNAPSE_DB", temp_db_dir.path().join("db"));
        }
        if let Some(log_dir) = log_dir {
            command.env("SYNAPSE_LOG_DIR", log_dir);
        }
        for (key, value) in envs {
            command.env(key, value);
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
            _temp_db_dir: temp_db_dir,
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

    pub async fn tools_call_error(&mut self, name: &str, args: Value) -> anyhow::Result<Value> {
        self.request_error(
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
            .with_context(|| {
                format!("timed out waiting for JSON-RPC response to {method} id {id}")
            })??;
        if let Some(error) = response.get("error") {
            bail!("JSON-RPC error from {method}: {error}");
        }
        response
            .get("result")
            .cloned()
            .with_context(|| format!("JSON-RPC response missing result: {response}"))
    }

    pub async fn request_error(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
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
            .with_context(|| {
                format!("timed out waiting for JSON-RPC error response to {method} id {id}")
            })??;
        response
            .get("error")
            .cloned()
            .with_context(|| format!("JSON-RPC response missing error: {response}"))
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

    #[cfg(unix)]
    pub async fn send_sigint_and_wait(mut self) -> anyhow::Result<std::process::ExitStatus> {
        let pid = self.child_id().context("child pid missing")?;
        let kill_status = Command::new("kill")
            .args(["-INT", &pid.to_string()])
            .status()
            .await
            .context("send SIGINT to child")?;
        if !kill_status.success() {
            bail!("kill -INT failed with status {kill_status}");
        }

        let mut child = self.child.take().context("child already reaped")?;
        let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .context("timed out waiting for child after SIGINT")?
            .context("wait for child after SIGINT")?;

        if let Some(stderr_task) = self.stderr_task.take() {
            let _stderr = stderr_task.await.context("join stderr reader")?;
        }
        Ok(status)
    }

    #[must_use]
    pub fn raw_received(&self) -> &[String] {
        &self.raw_rx
    }

    #[must_use]
    pub fn child_id(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
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
            for _ in 0..20 {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
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
