use std::{fs, path::Path};

use anyhow::Context;
use serde_json::{Value, json};
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn profile_tools_list_activate_and_report_health() -> anyhow::Result<()> {
    let profiles = TempDir::new()?;
    let logs = TempDir::new()?;
    write_profile(
        &profiles.path().join("alpha.toml"),
        "alpha",
        "Alpha",
        "alpha.exe",
    )?;
    write_profile(
        &profiles.path().join("beta.toml"),
        "beta",
        "Beta",
        "beta.exe",
    )?;

    let profile_dir = profiles.path().to_string_lossy().to_string();
    let mut client = StdioMcpClient::launch_and_init_with_env(
        Some(logs.path()),
        &[("SYNAPSE_PROFILE_DIR", profile_dir.as_str())],
    )
    .await?;

    let tools = client.tools_list().await?;
    let tools = tools
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;
    assert!(tools.iter().any(|tool| tool["name"] == "profile_list"));
    assert!(tools.iter().any(|tool| tool["name"] == "profile_activate"));
    assert_profile_tools_schema(tools);

    let response = client.tools_call("profile_list", json!({})).await?;
    let list = structured(&response)?;
    let listed = list["profiles"]
        .as_array()
        .context("profile_list did not return profiles array")?;
    assert_eq!(listed.len(), 2);
    assert_eq!(list["active_profile_id"], Value::Null);
    assert!(listed.iter().any(|profile| profile["id"] == "alpha"));
    assert!(listed.iter().any(|profile| profile["id"] == "beta"));
    assert!(
        listed
            .iter()
            .all(|profile| profile["active"].as_bool() == Some(false))
    );

    let response = client
        .tools_call("profile_activate", json!({"profile_id": "beta"}))
        .await?;
    let activate = structured(&response)?;
    assert_eq!(activate["active_profile_id"], "beta");
    assert_eq!(activate["changed"], true);

    let response = client
        .tools_call("profile_list", json!({"include_inactive": false}))
        .await?;
    let active_only = structured(&response)?;
    let active_profiles = active_only["profiles"]
        .as_array()
        .context("active-only profile_list did not return profiles array")?;
    assert_eq!(active_profiles.len(), 1);
    assert_eq!(active_profiles[0]["id"], "beta");
    assert_eq!(active_profiles[0]["active"], true);

    let response = client
        .tools_call("profile_activate", json!({"profile_id": "beta"}))
        .await?;
    let noop = structured(&response)?;
    assert_eq!(noop["active_profile_id"], "beta");
    assert_eq!(noop["previous_active_profile_id"], "beta");
    assert_eq!(noop["changed"], false);

    let response = client.tools_call("health", json!({})).await?;
    let health = structured(&response)?;
    assert_eq!(
        health["subsystems"]["profiles"]["active_profile_id"],
        "beta"
    );

    let error = client
        .tools_call_error("profile_activate", json!({"profile_id": "missing"}))
        .await?;
    assert_eq!(error["data"]["code"], "PROFILE_NOT_FOUND");

    let response = client.tools_call("health", json!({})).await?;
    let health = structured(&response)?;
    assert_eq!(
        health["subsystems"]["profiles"]["active_profile_id"],
        "beta"
    );

    let status = client.shutdown().await?;
    assert!(status.success());
    let logs_text = read_logs(logs.path())?;
    assert!(logs_text.contains("PROFILE_ACTIVATED"));
    assert!(logs_text.contains("beta"));
    Ok(())
}

fn structured(response: &Value) -> anyhow::Result<Value> {
    if let Some(value) = response.get("structuredContent") {
        return Ok(value.clone());
    }

    let text = response
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|content| content.get("text"))
        .and_then(Value::as_str)
        .context("structured content missing")?;
    serde_json::from_str(text).context("parse text content")
}

fn assert_profile_tools_schema(tools: &[Value]) {
    let mut profile_tool_shapes = tools
        .iter()
        .filter(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.starts_with("profile_"))
        })
        .map(|tool| {
            json!({
                "name": tool.get("name").cloned().unwrap_or(Value::Null),
                "inputSchema": tool.get("inputSchema").cloned().unwrap_or(Value::Null),
                "outputSchema": tool.get("outputSchema").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();
    profile_tool_shapes.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    insta::assert_json_snapshot!("m3_profile_tools", profile_tool_shapes);
}

fn write_profile(path: &Path, id: &str, label: &str, exe: &str) -> anyhow::Result<()> {
    fs::write(
        path,
        format!(
            r#"
id = "{id}"
label = "{label}"
schema_version = 1
use_scope = "productivity"
mouse_curve_default = "natural"
keyboard_dynamics_default = "natural"

[[matches]]
exe = "{exe}"
"#
        ),
    )?;
    Ok(())
}

fn read_logs(path: &Path) -> anyhow::Result<String> {
    let mut logs = String::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.metadata()?.is_file() {
            logs.push_str(&std::fs::read_to_string(entry.path())?);
        }
    }
    Ok(logs)
}
