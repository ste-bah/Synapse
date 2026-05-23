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
    assert_act_click_schema(tools)?;
    call_act_click_happy_paths(&mut client).await?;
    call_act_click_error_edges(&mut client).await?;

    assert!(client.shutdown().await?.success());
    let logs = read_logs(log_dir.path())?;
    assert_double_click_timing_cache_readback(&logs)?;
    assert_recording_log_readbacks(&logs)?;
    Ok(())
}

fn assert_act_click_schema(tools: &[Value]) -> anyhow::Result<()> {
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
    Ok(())
}

async fn call_act_click_happy_paths(client: &mut StdioMcpClient) -> anyhow::Result<()> {
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
    assert_timing_response(&response, 1);

    println!("source_of_truth=mcp_act_click edge=clicks_two before=target:(20,30) clicks:2");
    let clicks_two = client
        .tools_call(
            "act_click",
            json!({"target": {"x": 20, "y": 30}, "clicks": 2}),
        )
        .await?;
    let response: ActClickWireResponse = structured(&clicks_two)?;
    println!(
        "source_of_truth=mcp_act_click edge=clicks_two after=ok:{} window_ms:{} inter_click_delay_ms:{} elapsed_ms:{}",
        response.ok,
        response.double_click_window_ms,
        response.inter_click_delay_ms,
        response.elapsed_ms
    );
    assert!(response.ok);
    assert_timing_response(&response, 2);

    println!("source_of_truth=mcp_act_click edge=clicks_three before=target:(0,0) clicks:3");
    let clicks_three = client
        .tools_call(
            "act_click",
            json!({"target": {"x": 0, "y": 0}, "clicks": 3}),
        )
        .await?;
    let response: ActClickWireResponse = structured(&clicks_three)?;
    println!(
        "source_of_truth=mcp_act_click edge=clicks_three after=ok:{} window_ms:{} inter_click_delay_ms:{} elapsed_ms:{}",
        response.ok,
        response.double_click_window_ms,
        response.inter_click_delay_ms,
        response.elapsed_ms
    );
    assert!(response.ok);
    assert_timing_response(&response, 3);
    Ok(())
}

async fn call_act_click_error_edges(client: &mut StdioMcpClient) -> anyhow::Result<()> {
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

    println!("source_of_truth=mcp_act_click edge=clicks_four before=clicks:4");
    let clicks_four = client
        .tools_call_error(
            "act_click",
            json!({"target": {"x": 12, "y": 34}, "clicks": 4}),
        )
        .await?;
    println!("source_of_truth=mcp_act_click edge=clicks_four after={clicks_four}");
    assert_eq!(
        error_code(&clicks_four),
        Some(error_codes::TOOL_PARAMS_INVALID)
    );

    assert_malformed_element_id_rejected(client).await?;

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
    double_click_window_ms: u32,
    inter_click_delay_ms: u32,
    elapsed_ms: u32,
}

fn assert_timing_response(response: &ActClickWireResponse, clicks: u8) {
    assert!(response.double_click_window_ms >= 2);
    assert!(response.inter_click_delay_ms < response.double_click_window_ms);
    let max_total_ms = response.double_click_window_ms * u32::from(clicks);
    assert!(response.elapsed_ms <= max_total_ms);
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
    assert_click_readback(
        &readbacks,
        "happy",
        1,
        2,
        "mouse_move:screen(12,34):natural_fast:50>down:left>delay:0>up:left",
    )?;
    assert_click_readback(
        &readbacks,
        "clicks_two",
        2,
        4,
        "mouse_move:screen(20,30):natural_fast:50>down:left>delay:0>up:left>down:left>delay:0>up:left",
    )?;
    assert_click_readback(
        &readbacks,
        "clicks_three",
        3,
        6,
        "mouse_move:screen(0,0):natural_fast:50>down:left>delay:0>up:left>down:left>delay:0>up:left>down:left>delay:0>up:left",
    )?;
    Ok(())
}

fn assert_double_click_timing_cache_readback(logs: &str) -> anyhow::Result<()> {
    let readback = json_log_fields(logs)?
        .into_iter()
        .find(|fields| {
            fields.get("code").and_then(Value::as_str) == Some("M2_DOUBLE_CLICK_TIMING_CACHED")
        })
        .context("double-click timing cache readback missing")?;
    let window_ms = readback
        .get("window_ms")
        .and_then(Value::as_u64)
        .context("double-click timing readback missing window_ms")?;
    let inter_click_delay_ms = readback
        .get("inter_click_delay_ms")
        .and_then(Value::as_u64)
        .context("double-click timing readback missing inter_click_delay_ms")?;
    let source = readback
        .get("source")
        .and_then(Value::as_str)
        .context("double-click timing readback missing source")?;
    println!(
        "source_of_truth=daemon_log edge=double_click_cache after_window_ms={window_ms} after_inter_click_delay_ms={inter_click_delay_ms} source={source}"
    );
    assert!(inter_click_delay_ms < window_ms);
    Ok(())
}

fn assert_click_readback(
    readbacks: &[RecordingReadback],
    edge: &str,
    click_count: u64,
    button_event_count: u64,
    expected_sequence: &str,
) -> anyhow::Result<()> {
    let readback = readbacks
        .iter()
        .find(|readback| {
            readback.event_sequence == expected_sequence
                && readback.click_count == click_count
                && readback.button_event_count == button_event_count
        })
        .with_context(|| {
            format!("{edge} act_click recording readback missing expected sequence")
        })?;
    let first = readback
        .event_sequence
        .split('>')
        .next()
        .unwrap_or("<missing>");
    let last = readback
        .event_sequence
        .rsplit('>')
        .next()
        .unwrap_or("<missing>");
    println!(
        "source_of_truth=recording_log tool=act_click edge={edge} after_click_count={} button_events={} new_event_count={} window_ms={} inter_click_delay_ms={} scheduled_total_ms={} first={} last={} sequence={}",
        readback.click_count,
        readback.button_event_count,
        readback.new_event_count,
        readback.double_click_window_ms,
        readback.inter_click_delay_ms,
        readback.scheduled_inter_click_total_ms,
        first,
        last,
        readback.event_sequence
    );
    assert!(readback.inter_click_delay_ms < readback.double_click_window_ms);
    assert_eq!(readback.button_event_count, click_count * 2);
    assert!(
        readback.scheduled_inter_click_total_ms
            <= readback.double_click_window_ms * readback.click_count
    );
    Ok(())
}

#[derive(Debug)]
struct RecordingReadback {
    event_sequence: String,
    new_event_count: u64,
    click_count: u64,
    button_event_count: u64,
    double_click_window_ms: u64,
    inter_click_delay_ms: u64,
    scheduled_inter_click_total_ms: u64,
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
    for fields in json_log_fields(logs)? {
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
        let click_count = fields
            .get("click_count")
            .and_then(Value::as_u64)
            .context("recording readback missing click_count")?;
        let button_event_count = fields
            .get("button_event_count")
            .and_then(Value::as_u64)
            .context("recording readback missing button_event_count")?;
        let double_click_window_ms = fields
            .get("double_click_window_ms")
            .and_then(Value::as_u64)
            .context("recording readback missing double_click_window_ms")?;
        let inter_click_delay_ms = fields
            .get("inter_click_delay_ms")
            .and_then(Value::as_u64)
            .context("recording readback missing inter_click_delay_ms")?;
        let scheduled_inter_click_total_ms = fields
            .get("scheduled_inter_click_total_ms")
            .and_then(Value::as_u64)
            .context("recording readback missing scheduled_inter_click_total_ms")?;
        readbacks.push(RecordingReadback {
            event_sequence,
            new_event_count,
            click_count,
            button_event_count,
            double_click_window_ms,
            inter_click_delay_ms,
            scheduled_inter_click_total_ms,
        });
    }
    Ok(readbacks)
}

fn json_log_fields(logs: &str) -> anyhow::Result<Vec<Value>> {
    let mut fields = Vec::new();
    for line in logs.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line)?;
        fields.push(value["fields"].clone());
    }
    Ok(fields)
}
