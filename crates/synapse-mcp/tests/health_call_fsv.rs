use std::collections::BTreeMap;

use anyhow::Context;
use serde_json::Value;
use synapse_core::Health;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn synthetic_health_call_returns_expected_shape() -> anyhow::Result<()> {
    let dir = TempDir::new()?;
    let mut client = StdioMcpClient::launch_and_init_with_log_dir(Some(dir.path())).await?;

    let resp = client.tools_call("health", serde_json::json!({})).await?;
    let health = extract_health(&resp)?;
    assert!(health.ok);
    assert_eq!(health.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(health.build, "dev");
    assert!(health.subsystems.is_empty());

    let mut previous = health.uptime_s;
    for _ in 0..5 {
        let response = client.tools_call("health", serde_json::json!({})).await?;
        let current = extract_health(&response)?;
        assert!(current.uptime_s >= previous);
        previous = current.uptime_s;
    }

    let extra = client
        .tools_call("health", serde_json::json!({"junk":"x"}))
        .await?;
    assert!(extract_health(&extra)?.ok);

    let err = client
        .tools_call("healt_typo", serde_json::json!({}))
        .await
        .err()
        .context("unknown tool unexpectedly succeeded")?;
    let error_text = err.to_string();
    assert!(
        error_text.contains("Tool not found")
            || error_text.contains("not found")
            || error_text.contains("TOOL_NOT_FOUND")
    );

    let status = client.shutdown().await?;
    assert!(status.success());
    let logs = read_logs(dir.path())?;
    assert!(logs.contains("tool.invocation kind=health"));
    Ok(())
}

fn extract_health(resp: &Value) -> anyhow::Result<Health> {
    if let Some(value) = resp.get("structuredContent") {
        return Ok(serde_json::from_value(value.clone())?);
    }

    let content = resp
        .get("content")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .context("content[0] missing")?;
    if let Some(json) = content.get("json") {
        return Ok(serde_json::from_value(json.clone())?);
    }
    let text = content
        .get("text")
        .and_then(Value::as_str)
        .context("text content missing")?;
    Ok(serde_json::from_str(text)?)
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

#[test]
fn health_empty_subsystems_shape_is_stable() -> anyhow::Result<()> {
    let health = Health {
        ok: true,
        version: "0.1.0".to_owned(),
        build: "dev".to_owned(),
        uptime_s: 0,
        subsystems: BTreeMap::new(),
    };
    assert_eq!(
        serde_json::to_value(health)?,
        serde_json::json!({
            "ok": true,
            "version": "0.1.0",
            "build": "dev",
            "uptime_s": 0,
            "subsystems": {}
        })
    );
    Ok(())
}
