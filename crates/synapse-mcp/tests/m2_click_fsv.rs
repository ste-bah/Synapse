use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use synapse_core::error_codes;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;

#[tokio::test]
async fn act_click_schema_defaults_and_edges_fsv() -> anyhow::Result<()> {
    let mut client = StdioMcpClient::launch_and_init().await?;
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
