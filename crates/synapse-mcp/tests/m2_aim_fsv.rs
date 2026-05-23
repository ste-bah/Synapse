use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use synapse_core::error_codes;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn act_aim_schema_defaults_recording_and_edges_fsv() -> anyhow::Result<()> {
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
    assert_act_aim_schema(tools)?;
    call_act_aim_happy_styles_and_edges(&mut client).await?;

    assert!(client.shutdown().await?.success());
    let logs = read_logs(log_dir.path())?;
    assert_recording_log_readbacks(&logs)?;
    Ok(())
}

fn assert_act_aim_schema(tools: &[Value]) -> anyhow::Result<()> {
    let act_aim = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::String("act_aim".to_owned())))
        .context("act_aim tool missing")?;
    let schema = &act_aim["inputSchema"];
    println!(
        "source_of_truth=tools_list tool=act_aim edge=schema before=tool_count:{}",
        tools.len()
    );
    println!(
        "source_of_truth=tools_list tool=act_aim edge=defaults after=style:{} deadline_ms:{} backend:{} additionalProperties:{}",
        schema["properties"]["style"]["default"],
        schema["properties"]["deadline_ms"]["default"],
        schema["properties"]["backend"]["default"],
        schema["additionalProperties"]
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["style"]["default"], "snap");
    assert_eq!(schema["properties"]["deadline_ms"]["default"], 80);
    assert_eq!(schema["properties"]["backend"]["default"], "auto");
    assert_backend_schema(schema);

    let projection = json!({
        "name": act_aim["name"],
        "description": act_aim["description"],
        "inputSchema": act_aim["inputSchema"],
        "outputSchemaRoot": schema_root(act_aim.get("outputSchema")),
    });
    insta::assert_json_snapshot!("m2_act_aim_tool", projection);
    Ok(())
}

fn assert_backend_schema(schema: &Value) {
    let schema_text = schema.to_string();
    assert!(schema_text.contains("software"));
    assert!(schema_text.contains("hardware"));
    assert!(schema_text.contains("auto"));
    assert!(!schema_text.contains("vigem"));
}

async fn call_act_aim_happy_styles_and_edges(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_act_aim edge=snap before=target:(200,200)");
    let snap = client
        .tools_call("act_aim", json!({"target": {"x": 200, "y": 200}}))
        .await?;
    let response: ActAimWireResponse = structured(&snap)?;
    println!(
        "source_of_truth=mcp_act_aim edge=snap after=ok:{} style_used:{} duration_ms:{} backend_used:{} elapsed_ms:{} expected_sequence:mouse_move:screen(200,200):natural_fast:50",
        response.ok,
        response.style_used,
        response.duration_ms,
        response.backend_used,
        response.elapsed_ms
    );
    assert!(response.ok);
    assert_eq!(response.style_used, "snap");
    assert_eq!(response.duration_ms, 50);
    assert_eq!(response.backend_used, "software");

    println!("source_of_truth=mcp_act_aim edge=flick before=target:(201,202) style:flick");
    let flick = client
        .tools_call(
            "act_aim",
            json!({"target": {"x": 201, "y": 202}, "style": "flick"}),
        )
        .await?;
    let response: ActAimWireResponse = structured(&flick)?;
    println!(
        "source_of_truth=mcp_act_aim edge=flick after=ok:{} style_used:{} duration_ms:{} backend_used:{} elapsed_ms:{} expected_sequence:mouse_move:screen(201,202):natural_fast:35",
        response.ok,
        response.style_used,
        response.duration_ms,
        response.backend_used,
        response.elapsed_ms
    );
    assert!(response.ok);
    assert_eq!(response.style_used, "flick");
    assert_eq!(response.duration_ms, 35);

    println!("source_of_truth=mcp_act_aim edge=natural before=target:(203,204) style:natural");
    let natural = client
        .tools_call(
            "act_aim",
            json!({"target": {"x": 203, "y": 204}, "style": "natural"}),
        )
        .await?;
    let response: ActAimWireResponse = structured(&natural)?;
    println!(
        "source_of_truth=mcp_act_aim edge=natural after=ok:{} style_used:{} duration_ms:{} backend_used:{} elapsed_ms:{} expected_sequence:mouse_move:screen(203,204):natural_fast:150",
        response.ok,
        response.style_used,
        response.duration_ms,
        response.backend_used,
        response.elapsed_ms
    );
    assert!(response.ok);
    assert_eq!(response.style_used, "natural");
    assert_eq!(response.duration_ms, 150);

    println!(
        "source_of_truth=mcp_act_aim edge=deadline_override before=target:(205,206) deadline_ms:7"
    );
    let override_deadline = client
        .tools_call(
            "act_aim",
            json!({"target": {"x": 205, "y": 206}, "deadline_ms": 7}),
        )
        .await?;
    let response: ActAimWireResponse = structured(&override_deadline)?;
    println!(
        "source_of_truth=mcp_act_aim edge=deadline_override after=ok:{} style_used:{} duration_ms:{} backend_used:{} elapsed_ms:{} expected_sequence:mouse_move:screen(205,206):natural_fast:7",
        response.ok,
        response.style_used,
        response.duration_ms,
        response.backend_used,
        response.elapsed_ms
    );
    assert!(response.ok);
    assert_eq!(response.duration_ms, 7);

    call_act_aim_error_edges(client).await
}

async fn call_act_aim_error_edges(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    assert_error_code(
        client,
        "extra_property",
        "junk:true",
        json!({"target": {"x": 200, "y": 200}, "junk": true}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "track_style",
        "target:track_id=42 style:track",
        json!({"target": {"track_id": 42}, "style": "track"}),
        error_codes::ACTION_BACKEND_UNAVAILABLE,
    )
    .await?;
    assert_error_code(
        client,
        "element_target",
        "element_id:0x1:2a",
        json!({"target": {"element_id": "0x1:2a"}}),
        error_codes::ACTION_BACKEND_UNAVAILABLE,
    )
    .await?;
    assert_error_code(
        client,
        "invalid_style",
        "style:telepathy",
        json!({"target": {"x": 200, "y": 200}, "style": "telepathy"}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await
}

async fn assert_error_code(
    client: &mut StdioMcpClient,
    edge: &str,
    before: &str,
    args: Value,
    expected_code: &'static str,
) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_act_aim edge={edge} before={before}");
    let error = client.tools_call_error("act_aim", args).await?;
    println!("source_of_truth=mcp_act_aim edge={edge} after={error}");
    assert_eq!(error_code(&error), Some(expected_code));
    Ok(())
}

fn assert_recording_log_readbacks(logs: &str) -> anyhow::Result<()> {
    let readbacks = recording_readbacks(logs)?;
    assert_readback(
        &readbacks,
        "snap",
        "mouse_move:screen(200,200):natural_fast:50",
    )?;
    assert_readback(
        &readbacks,
        "flick",
        "mouse_move:screen(201,202):natural_fast:35",
    )?;
    assert_readback(
        &readbacks,
        "natural",
        "mouse_move:screen(203,204):natural_fast:150",
    )?;
    assert_readback(
        &readbacks,
        "deadline_override",
        "mouse_move:screen(205,206):natural_fast:7",
    )
}

fn assert_readback(
    readbacks: &[RecordingReadback],
    edge: &str,
    expected_sequence: &str,
) -> anyhow::Result<()> {
    let readback = readbacks
        .iter()
        .find(|readback| {
            readback.event_sequence == expected_sequence && readback.new_event_count == 1
        })
        .with_context(|| format!("{edge} act_aim recording readback missing expected sequence"))?;
    println!(
        "source_of_truth=recording_log tool=act_aim edge={edge} after_event_sequence={} new_event_count={}",
        readback.event_sequence, readback.new_event_count
    );
    Ok(())
}

#[derive(serde::Deserialize)]
struct ActAimWireResponse {
    ok: bool,
    style_used: String,
    duration_ms: u32,
    backend_used: String,
    elapsed_ms: u32,
}

#[derive(Debug)]
struct RecordingReadback {
    event_sequence: String,
    new_event_count: u64,
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
        if fields.get("code").and_then(Value::as_str) != Some("M2_ACT_AIM_RECORDING_READBACK") {
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
