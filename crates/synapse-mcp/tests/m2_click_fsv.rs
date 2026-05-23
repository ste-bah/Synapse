use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use synapse_core::error_codes;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

const ELEMENT_ID_PATTERN: &str = r"^-?0x[0-9a-fA-F]+:[0-9a-fA-F]+$";

#[tokio::test]
async fn act_click_schema_defaults_and_edges_fsv() -> anyhow::Result<()> {
    let log_dir = TempDir::new()?;
    let mut client = StdioMcpClient::launch_and_init_with_env(
        Some(log_dir.path()),
        &[("SYNAPSE_MCP_RECORDING_BACKEND", "1")],
    )
    .await?;
    let resp = client.tools_list().await?;
    let tools = resp
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;
    let act_click = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::String("act_click".to_owned())))
        .context("act_click tool missing")?;
    let schema = &act_click["inputSchema"];
    println!(
        "source_of_truth=tools_list tool=act_click edge=schema before=tool_count:{}",
        tools.len()
    );
    println!(
        "source_of_truth=tools_list tool=act_click edge=defaults after=curve:{} duration_ms:{} button:{} clicks:{} use_invoke_pattern:{} backend:{} additionalProperties:{}",
        schema["properties"]["curve"]["default"],
        schema["properties"]["duration_ms"]["default"],
        schema["properties"]["button"]["default"],
        schema["properties"]["clicks"]["default"],
        schema["properties"]["use_invoke_pattern"]["default"],
        schema["properties"]["backend"]["default"],
        schema["additionalProperties"]
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["curve"]["default"], "natural");
    assert_eq!(schema["properties"]["duration_ms"]["default"], 50);
    assert_eq!(schema["properties"]["button"]["default"], "left");
    assert_eq!(schema["properties"]["clicks"]["default"], 1);
    assert_eq!(schema["properties"]["use_invoke_pattern"]["default"], true);
    assert_eq!(schema["properties"]["backend"]["default"], "auto");
    assert_element_id_schema_pattern(schema);

    let projection = json!({
        "name": act_click["name"],
        "description": act_click["description"],
        "inputSchema": act_click["inputSchema"],
        "outputSchemaRoot": schema_root(act_click.get("outputSchema")),
    });
    insta::assert_json_snapshot!("m2_act_click_tool", projection);

    println!("source_of_truth=mcp_act_click edge=happy before=target:(12,34)");
    let happy = client
        .tools_call("act_click", json!({"target": {"x": 12, "y": 34}}))
        .await?;
    let response: ActClickWireResponse = structured(&happy)?;
    println!(
        "source_of_truth=mcp_act_click edge=happy after=ok:{} used_invoke_pattern:{} backend_used:{} elapsed_ms:{}",
        response.ok, response.used_invoke_pattern, response.backend_used, response.elapsed_ms
    );
    assert!(response.ok);
    assert!(!response.used_invoke_pattern);
    assert_eq!(response.backend_used, "software");

    println!("source_of_truth=mcp_act_click edge=extra_property before=junk:true");
    let extra = client
        .tools_call_error(
            "act_click",
            json!({"target": {"x": 12, "y": 34}, "junk": true}),
        )
        .await?;
    println!("source_of_truth=mcp_act_click edge=extra_property after={extra}");
    assert_eq!(error_code(&extra), Some(error_codes::TOOL_PARAMS_INVALID));

    println!("source_of_truth=mcp_act_click edge=clicks_zero before=clicks:0");
    let clicks_zero = client
        .tools_call_error(
            "act_click",
            json!({"target": {"x": 12, "y": 34}, "clicks": 0}),
        )
        .await?;
    println!("source_of_truth=mcp_act_click edge=clicks_zero after={clicks_zero}");
    assert_eq!(
        error_code(&clicks_zero),
        Some(error_codes::TOOL_PARAMS_INVALID)
    );

    assert_malformed_element_id_rejected(&mut client).await?;

    println!("source_of_truth=mcp_act_click edge=modifier_rejected before=modifiers:[ctrl]");
    let modifier = client
        .tools_call_error(
            "act_click",
            json!({"target": {"x": 12, "y": 34}, "modifiers": ["ctrl"]}),
        )
        .await?;
    println!("source_of_truth=mcp_act_click edge=modifier_rejected after={modifier}");
    assert_eq!(
        error_code(&modifier),
        Some(error_codes::ACTION_BACKEND_UNAVAILABLE)
    );

    assert!(client.shutdown().await?.success());
    let logs = read_logs(log_dir.path())?;
    assert_recording_log_readbacks(&logs)?;
    Ok(())
}

#[tokio::test]
async fn act_click_default_unset_uses_actor_path_without_recording_log_fsv() -> anyhow::Result<()> {
    let log_dir = TempDir::new()?;
    let mut client = StdioMcpClient::launch_and_init_with_log_dir(Some(log_dir.path())).await?;

    println!("source_of_truth=mcp_act_click edge=env_unset before=recording_env:absent");
    let response = client
        .tools_call("act_click", json!({"target": {"x": 3, "y": 4}}))
        .await?;
    let response: ActClickWireResponse = structured(&response)?;
    assert!(response.ok);
    assert_eq!(response.backend_used, "software");

    assert!(client.shutdown().await?.success());
    let logs = read_logs(log_dir.path())?;
    let readbacks = recording_readbacks(&logs)?;
    println!(
        "source_of_truth=recording_log tool=act_click edge=env_unset after_readback_count={}",
        readbacks.len()
    );
    assert!(readbacks.is_empty());
    Ok(())
}

#[derive(serde::Deserialize)]
struct ActClickWireResponse {
    ok: bool,
    used_invoke_pattern: bool,
    backend_used: String,
    elapsed_ms: u32,
}

fn structured<T: DeserializeOwned>(resp: &Value) -> anyhow::Result<T> {
    serde_json::from_value(resp["structuredContent"].clone()).context("decode structuredContent")
}

fn error_code(error: &Value) -> Option<&str> {
    error
        .get("data")
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
}

fn schema_root(value: Option<&Value>) -> Value {
    let Some(value) = value else {
        return Value::Null;
    };
    json!({
        "title": value.get("title"),
        "type": value.get("type"),
        "required": value.get("required"),
        "additionalProperties": value.get("additionalProperties"),
    })
}

fn assert_element_id_schema_pattern(schema: &Value) {
    println!(
        "source_of_truth=tools_list tool=act_click edge=element_id_schema after_type:{} after_pattern:{}",
        schema["$defs"]["ElementId"]["type"], schema["$defs"]["ElementId"]["pattern"]
    );
    assert_eq!(
        schema["$defs"]["ElementId"]["pattern"],
        Value::String(ELEMENT_ID_PATTERN.to_owned())
    );
}

async fn assert_malformed_element_id_rejected(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    println!(
        "source_of_truth=mcp_act_click edge=malformed_element_id before=element_id:not-a-valid-id"
    );
    let malformed = client
        .tools_call_error(
            "act_click",
            json!({"target": {"element_id": "not-a-valid-id"}}),
        )
        .await?;
    println!("source_of_truth=mcp_act_click edge=malformed_element_id after={malformed}");
    let malformed_code =
        error_code(&malformed).context("malformed element_id error code missing")?;
    assert!(
        [
            error_codes::TOOL_PARAMS_INVALID,
            error_codes::ACTION_TARGET_INVALID
        ]
        .contains(&malformed_code),
        "malformed element_id rejected with unexpected code {malformed_code}"
    );
    assert_eq!(
        malformed_code,
        error_codes::TOOL_PARAMS_INVALID,
        "current rejection layer is MCP parameter deserialization after ElementId parse validation"
    );
    Ok(())
}

fn assert_recording_log_readbacks(logs: &str) -> anyhow::Result<()> {
    let readbacks = recording_readbacks(logs)?;
    let readback = readbacks
        .iter()
        .find(|readback| {
            readback.event_sequence
                == "mouse_move:screen(12,34):natural_fast:50>down:left>delay:0>up:left"
                && readback.new_event_count == 4
        })
        .context("happy-path act_click recording readback missing expected sequence")?;
    let mut events = readback.event_sequence.split('>');
    let first = events.next().unwrap_or("<missing>");
    let last = readback
        .event_sequence
        .rsplit('>')
        .next()
        .unwrap_or("<missing>");
    println!(
        "source_of_truth=recording_log tool=act_click edge=happy after_event_count={} first={} last={} sequence={}",
        readback.new_event_count, first, last, readback.event_sequence
    );
    Ok(())
}

#[derive(Debug)]
struct RecordingReadback {
    event_sequence: String,
    new_event_count: u64,
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

fn recording_readbacks(logs: &str) -> anyhow::Result<Vec<RecordingReadback>> {
    let mut readbacks = Vec::new();
    for line in logs.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line)?;
        let fields = &value["fields"];
        if fields.get("code").and_then(Value::as_str) != Some("M2_ACT_CLICK_RECORDING_READBACK") {
            continue;
        }
        let event_sequence = fields
            .get("event_sequence")
            .and_then(Value::as_str)
            .context("recording readback missing event_sequence")?
            .to_owned();
        let new_event_count = fields
            .get("new_event_count")
            .and_then(Value::as_u64)
            .context("recording readback missing new_event_count")?;
        readbacks.push(RecordingReadback {
            event_sequence,
            new_event_count,
        });
    }
    Ok(readbacks)
}
