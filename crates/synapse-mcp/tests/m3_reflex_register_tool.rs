use anyhow::Context;
use serde_json::{Value, json};
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn reflex_register_schema_defaults_and_edges() -> anyhow::Result<()> {
    let logs = TempDir::new()?;
    let db = TempDir::new()?;
    let db_path = db.path().join("db");
    let db_path = db_path.to_string_lossy().into_owned();
    let mut client = StdioMcpClient::launch_and_init_with_env(
        Some(logs.path()),
        &[("SYNAPSE_DB", db_path.as_str())],
    )
    .await?;
    activate_notepad_profile(&mut client).await?;

    let tools = client.tools_list().await?;
    let tools = tools
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;
    let reflex_register_tool = tools
        .iter()
        .find(|tool| tool["name"] == "reflex_register")
        .context("reflex_register tool missing")?;
    assert_reflex_register_schema(reflex_register_tool);

    let response = client
        .tools_call("reflex_register", valid_register_args("support-reflex"))
        .await?;
    let first = structured(&response)?;
    let first_id = first["reflex_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .context("reflex_id missing")?;
    assert_eq!(first["state"]["id"], first_id);
    assert_eq!(first["state"]["priority"], 100);
    assert_eq!(first["state"]["lifetime"]["kind"], "until_cancelled");

    let bad_kind = client
        .tools_call_error(
            "reflex_register",
            json!({
                "kind": "nonsense",
                "when": {"op": "kind", "kind": "support-reflex"},
                "then": {"kind": "action", "action": {"kind": "release_all"}}
            }),
        )
        .await?;
    assert_eq!(bad_kind["data"]["code"], "REFLEX_KIND_INVALID");

    let bad_priority = client
        .tools_call_error(
            "reflex_register",
            json!({
                "kind": "on_event",
                "when": {"op": "kind", "kind": "support-reflex"},
                "then": {"kind": "action", "action": {"kind": "release_all"}},
                "priority": 4_294_967_295_u64
            }),
        )
        .await?;
    assert_eq!(bad_priority["data"]["code"], "REFLEX_PRIORITY_INVALID");

    for index in 1..32 {
        let response = client
            .tools_call(
                "reflex_register",
                valid_register_args(&format!("support-reflex-{index}")),
            )
            .await?;
        let payload = structured(&response)?;
        assert!(
            payload["reflex_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        );
    }
    let capped = client
        .tools_call_error("reflex_register", valid_register_args("support-reflex-cap"))
        .await?;
    assert_eq!(capped["data"]["code"], "REFLEX_CAP_REACHED");

    let status = client.shutdown().await?;
    assert!(status.success());
    Ok(())
}

fn valid_register_args(kind: &str) -> Value {
    json!({
        "kind": "on_event",
        "when": {"op": "kind", "kind": kind},
        "then": {"kind": "action", "action": {"kind": "release_all"}}
    })
}

async fn activate_notepad_profile(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    let response = client
        .tools_call("profile_activate", json!({"profile_id": "notepad"}))
        .await?;
    assert_eq!(structured(&response)?["active_profile_id"], "notepad");
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

fn assert_reflex_register_schema(tool: &Value) {
    let shape = json!({
        "name": tool.get("name").cloned().unwrap_or(Value::Null),
        "inputSchema": tool.get("inputSchema").cloned().unwrap_or(Value::Null),
        "outputSchema": tool.get("outputSchema").cloned().unwrap_or(Value::Null),
    });
    assert_eq!(shape["inputSchema"]["additionalProperties"], false);
    assert_eq!(
        shape["inputSchema"]["properties"]["kind"]["enum"][4],
        "on_event"
    );
    assert_eq!(
        shape["inputSchema"]["properties"]["priority"]["default"],
        100
    );
    assert_eq!(
        shape["inputSchema"]["properties"]["priority"]["maximum"],
        1000
    );
    assert_eq!(
        shape["inputSchema"]["properties"]["lifetime"]["default"]["kind"],
        "until_cancelled"
    );
    assert_eq!(
        shape["inputSchema"]["properties"]["backend"]["default"],
        "auto"
    );
    insta::assert_json_snapshot!("m3_reflex_register_tool", shape);
}
