use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use synapse_core::error_codes;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn act_press_schema_defaults_recording_and_edges_fsv() -> anyhow::Result<()> {
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
    assert_act_press_schema(tools)?;
    call_act_press_happy_unordered_and_edges(&mut client).await?;

    assert!(client.shutdown().await?.success());
    let logs = read_logs(log_dir.path())?;
    assert_recording_log_readbacks(&logs)?;
    Ok(())
}

fn assert_act_press_schema(tools: &[Value]) -> anyhow::Result<()> {
    let act_press = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::String("act_press".to_owned())))
        .context("act_press tool missing")?;
    let schema = &act_press["inputSchema"];
    println!(
        "source_of_truth=tools_list tool=act_press edge=schema before=tool_count:{}",
        tools.len()
    );
    println!(
        "source_of_truth=tools_list tool=act_press edge=defaults after=hold_ms:{} minimum:{} maximum:{} backend:{} additionalProperties:{}",
        schema["properties"]["hold_ms"]["default"],
        schema["properties"]["hold_ms"]["minimum"],
        schema["properties"]["hold_ms"]["maximum"],
        schema["properties"]["backend"]["default"],
        schema["additionalProperties"]
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["hold_ms"]["default"], 33);
    assert_eq!(schema["properties"]["hold_ms"]["minimum"], 1);
    assert_eq!(schema["properties"]["hold_ms"]["maximum"], 30000);
    assert_eq!(schema["properties"]["backend"]["default"], "auto");
    assert_backend_schema(schema);

    let projection = json!({
        "name": act_press["name"],
        "description": act_press["description"],
        "inputSchema": act_press["inputSchema"],
        "outputSchemaRoot": schema_root(act_press.get("outputSchema")),
    });
    insta::assert_json_snapshot!("m2_act_press_tool", projection);
    Ok(())
}

fn assert_backend_schema(schema: &Value) {
    let schema_text = schema.to_string();
    assert!(schema_text.contains("software"));
    assert!(schema_text.contains("hardware"));
    assert!(schema_text.contains("auto"));
    assert!(!schema_text.contains("vigem"));
}

async fn call_act_press_happy_unordered_and_edges(
    client: &mut StdioMcpClient,
) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_act_press edge=happy before=keys:[ctrl,shift,s]");
    let happy = client
        .tools_call("act_press", json!({"keys": ["ctrl", "shift", "s"]}))
        .await?;
    let response: ActPressWireResponse = structured(&happy)?;
    println!(
        "source_of_truth=mcp_act_press edge=happy after=ok:{} keys_pressed:{} backend_used:{} elapsed_ms:{} expected_sequence:down:ctrl>down:shift>down:s>delay:33>up:s>up:shift>up:ctrl",
        response.ok, response.keys_pressed, response.backend_used, response.elapsed_ms
    );
    assert!(response.ok);
    assert_eq!(response.keys_pressed, 3);
    assert_eq!(response.backend_used, "software");

    println!(
        "source_of_truth=mcp_act_press edge=unordered_modifiers before=keys:[shift,ctrl,s] hold_ms:7"
    );
    let unordered = client
        .tools_call(
            "act_press",
            json!({"keys": ["shift", "ctrl", "s"], "hold_ms": 7}),
        )
        .await?;
    let response: ActPressWireResponse = structured(&unordered)?;
    println!(
        "source_of_truth=mcp_act_press edge=unordered_modifiers after=ok:{} keys_pressed:{} backend_used:{} elapsed_ms:{} expected_sequence:down:ctrl>down:shift>down:s>delay:7>up:s>up:shift>up:ctrl",
        response.ok, response.keys_pressed, response.backend_used, response.elapsed_ms
    );
    assert!(response.ok);
    assert_eq!(response.keys_pressed, 3);

    call_act_press_error_edges(client).await
}

async fn call_act_press_error_edges(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    assert_error_code(
        client,
        "extra_property",
        "junk:true",
        json!({"keys": ["a"], "junk": true}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "empty_keys",
        "keys:[]",
        json!({"keys": []}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "hold_zero",
        "hold_ms:0",
        json!({"keys": ["a"], "hold_ms": 0}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "hold_exceeded",
        "hold_ms:30001",
        json!({"keys": ["a"], "hold_ms": 30001}),
        error_codes::ACTION_HOLD_EXCEEDED_MAX,
    )
    .await?;
    assert_error_code(
        client,
        "duplicate_alias",
        "keys:[ctrl,control]",
        json!({"keys": ["ctrl", "control"]}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "unsupported_key",
        "keys:[definitely_not_a_key]",
        json!({"keys": ["definitely_not_a_key"]}),
        error_codes::ACTION_UNSUPPORTED_KEY,
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
    println!("source_of_truth=mcp_act_press edge={edge} before={before}");
    let error = client.tools_call_error("act_press", args).await?;
    println!("source_of_truth=mcp_act_press edge={edge} after={error}");
    assert_eq!(error_code(&error), Some(expected_code));
    Ok(())
}

fn assert_recording_log_readbacks(logs: &str) -> anyhow::Result<()> {
    let readbacks = recording_readbacks(logs)?;
    let happy_readback = readbacks
        .iter()
        .find(|readback| {
            readback.event_sequence == "down:ctrl>down:shift>down:s>delay:33>up:s>up:shift>up:ctrl"
                && readback.new_event_count == 7
        })
        .context("happy-path act_press recording readback missing expected chord sequence")?;
    let unordered_readback = readbacks
        .iter()
        .find(|readback| {
            readback.event_sequence == "down:ctrl>down:shift>down:s>delay:7>up:s>up:shift>up:ctrl"
                && readback.new_event_count == 7
        })
        .context(
            "unordered-modifier act_press recording readback missing expected chord sequence",
        )?;
    println!(
        "source_of_truth=recording_log tool=act_press edge=happy after_event_sequence={} new_event_count={}",
        happy_readback.event_sequence, happy_readback.new_event_count
    );
    println!(
        "source_of_truth=recording_log tool=act_press edge=unordered_modifiers after_event_sequence={} new_event_count={}",
        unordered_readback.event_sequence, unordered_readback.new_event_count
    );
    Ok(())
}

#[derive(serde::Deserialize)]
struct ActPressWireResponse {
    ok: bool,
    keys_pressed: u32,
    elapsed_ms: u32,
    backend_used: String,
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
        if fields.get("code").and_then(Value::as_str) != Some("M2_ACT_PRESS_RECORDING_READBACK") {
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
