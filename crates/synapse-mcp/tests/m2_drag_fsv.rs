use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use synapse_core::error_codes;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn act_drag_schema_defaults_recording_and_edges_fsv() -> anyhow::Result<()> {
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
    assert_act_drag_schema(tools)?;
    call_act_drag_happy_and_edges(&mut client).await?;

    assert!(client.shutdown().await?.success());
    let logs = read_logs(log_dir.path())?;
    assert_recording_log_readbacks(&logs)?;
    Ok(())
}

fn assert_act_drag_schema(tools: &[Value]) -> anyhow::Result<()> {
    let act_drag = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::String("act_drag".to_owned())))
        .context("act_drag tool missing")?;
    let schema = &act_drag["inputSchema"];
    println!(
        "source_of_truth=tools_list tool=act_drag edge=schema before=tool_count:{}",
        tools.len()
    );
    println!(
        "source_of_truth=tools_list tool=act_drag edge=defaults after=curve:{} duration_ms:{} button:{} backend:{} additionalProperties:{}",
        schema["properties"]["curve"]["default"],
        schema["properties"]["duration_ms"]["default"],
        schema["properties"]["button"]["default"],
        schema["properties"]["backend"]["default"],
        schema["additionalProperties"]
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["curve"]["default"], "natural");
    assert_eq!(schema["properties"]["duration_ms"]["default"], 200);
    assert_eq!(schema["properties"]["button"]["default"], "left");
    assert_eq!(schema["properties"]["backend"]["default"], "auto");
    assert_eq!(schema["required"], json!(["from", "to"]));
    assert_drag_target_schema_accepts_points_and_elements(schema);
    assert_drag_button_and_backend_schema(schema);

    let projection = json!({
        "name": act_drag["name"],
        "description": act_drag["description"],
        "inputSchema": act_drag["inputSchema"],
        "outputSchemaRoot": schema_root(act_drag.get("outputSchema")),
    });
    insta::assert_json_snapshot!("m2_act_drag_tool", projection);
    Ok(())
}

fn assert_drag_target_schema_accepts_points_and_elements(schema: &Value) {
    let schema_text = schema.to_string();
    assert!(schema_text.contains("\"from\""));
    assert!(schema_text.contains("\"to\""));
    assert!(schema_text.contains("\"x\""));
    assert!(schema_text.contains("\"y\""));
    assert!(schema_text.contains("\"element_id\""));
    assert!(schema_text.contains("\"ActDragPointTarget\""));
    assert!(schema_text.contains("\"ActDragElementTarget\""));
}

fn assert_drag_button_and_backend_schema(schema: &Value) {
    let schema_text = schema.to_string();
    assert!(schema_text.contains("\"left\""));
    assert!(schema_text.contains("\"right\""));
    assert!(schema_text.contains("\"middle\""));
    assert!(schema_text.contains("\"software\""));
    assert!(schema_text.contains("\"hardware\""));
    assert!(schema_text.contains("\"auto\""));
    assert!(!schema_text.contains("\"vigem\""));
}

async fn call_act_drag_happy_and_edges(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_act_drag edge=default_happy before=from:(100,100) to:(300,300)");
    let happy = client
        .tools_call(
            "act_drag",
            json!({"from": {"x": 100, "y": 100}, "to": {"x": 300, "y": 300}}),
        )
        .await?;
    let response: ActDragWireResponse = structured(&happy)?;
    println!(
        "source_of_truth=mcp_act_drag edge=default_happy after=ok:{} button_used:{} curve_used:{} duration_ms:{} backend_used:{} distance_px:{:.3} elapsed_ms:{} expected_sequence:down:left>mouse_move:screen(300,300):natural_fast:200>up:left",
        response.ok,
        response.button_used,
        response.curve_used,
        response.duration_ms,
        response.backend_used,
        response.distance_px,
        response.elapsed_ms
    );
    assert!(response.ok);
    assert_eq!(response.button_used, "left");
    assert_eq!(response.curve_used, "natural");
    assert_eq!(response.duration_ms, 200);
    assert_eq!(response.backend_used, "software");
    assert!((response.distance_px - 282.842_712).abs() < 0.001);

    println!(
        "source_of_truth=mcp_act_drag edge=right_linear before=from:(-10,5) to:(20,-25) button:right curve:linear duration_ms:75"
    );
    let explicit = client
        .tools_call(
            "act_drag",
            json!({
                "from": {"x": -10, "y": 5},
                "to": {"x": 20, "y": -25},
                "button": "right",
                "curve": "linear",
                "duration_ms": 75
            }),
        )
        .await?;
    let response: ActDragWireResponse = structured(&explicit)?;
    println!(
        "source_of_truth=mcp_act_drag edge=right_linear after=ok:{} button_used:{} curve_used:{} duration_ms:{} backend_used:{} distance_px:{:.3} elapsed_ms:{} expected_sequence:down:right>mouse_move:screen(20,-25):linear:75>up:right",
        response.ok,
        response.button_used,
        response.curve_used,
        response.duration_ms,
        response.backend_used,
        response.distance_px,
        response.elapsed_ms
    );
    assert!(response.ok);
    assert_eq!(response.button_used, "right");
    assert_eq!(response.curve_used, "linear");
    assert_eq!(response.duration_ms, 75);
    assert!((response.distance_px - 42.426_407).abs() < 0.001);

    call_act_drag_error_edges(client).await
}

async fn call_act_drag_error_edges(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    assert_error_code(
        client,
        "over_limit",
        "from:(0,0) to:(5000,5000)",
        json!({"from": {"x": 0, "y": 0}, "to": {"x": 5000, "y": 5000}}),
        error_codes::ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT,
    )
    .await?;
    assert_error_code(
        client,
        "element_target_non_windows",
        "from:element_id=0x1:2a to:(1,1)",
        json!({"from": {"element_id": "0x1:2a"}, "to": {"x": 1, "y": 1}}),
        error_codes::ACTION_BACKEND_UNAVAILABLE,
    )
    .await?;
    assert_error_code(
        client,
        "extra_property",
        "junk:true",
        json!({"from": {"x": 0, "y": 0}, "to": {"x": 1, "y": 1}, "junk": true}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "invalid_curve",
        "curve:telepathy",
        json!({"from": {"x": 0, "y": 0}, "to": {"x": 1, "y": 1}, "curve": "telepathy"}),
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
    println!("source_of_truth=mcp_act_drag edge={edge} before={before}");
    let error = client.tools_call_error("act_drag", args).await?;
    println!("source_of_truth=mcp_act_drag edge={edge} after={error}");
    assert_eq!(error_code(&error), Some(expected_code));
    Ok(())
}

fn assert_recording_log_readbacks(logs: &str) -> anyhow::Result<()> {
    let readbacks = recording_readbacks(logs)?;
    assert_readback(
        &readbacks,
        "default_happy",
        "down:left>mouse_move:screen(300,300):natural_fast:200>up:left",
    )?;
    assert_readback(
        &readbacks,
        "right_linear",
        "down:right>mouse_move:screen(20,-25):linear:75>up:right",
    )?;
    println!(
        "source_of_truth=recording_log tool=act_drag edge=failed_edges after_readback_count={} expected_successful_readbacks=2",
        readbacks.len()
    );
    assert_eq!(readbacks.len(), 2);
    Ok(())
}

fn assert_readback(
    readbacks: &[RecordingReadback],
    edge: &str,
    expected_sequence: &str,
) -> anyhow::Result<()> {
    let readback = readbacks
        .iter()
        .find(|readback| {
            readback.event_sequence == expected_sequence && readback.new_event_count == 3
        })
        .with_context(|| format!("{edge} act_drag recording readback missing expected sequence"))?;
    println!(
        "source_of_truth=recording_log tool=act_drag edge={edge} after_event_sequence={} new_event_count={}",
        readback.event_sequence, readback.new_event_count
    );
    Ok(())
}

#[derive(serde::Deserialize)]
struct ActDragWireResponse {
    ok: bool,
    button_used: String,
    curve_used: String,
    duration_ms: u32,
    distance_px: f64,
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
        if fields.get("code").and_then(Value::as_str) != Some("M2_ACT_DRAG_RECORDING_READBACK") {
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
