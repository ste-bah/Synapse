use std::sync::LazyLock;

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use synapse_core::{Observation, OcrResult, PerceptionMode, error_codes};
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;

const SYNTHETIC_ENV: &[(&str, &str)] = &[("SYNAPSE_MCP_SYNTHETIC_FIXTURE", "notepad")];
static MCP_FSV_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[tokio::test]
async fn tools_list_contains_m1_surface_and_closed_schemas() -> anyhow::Result<()> {
    let _guard = MCP_FSV_LOCK.lock().await;
    let mut client = StdioMcpClient::launch_and_init_with_env(None, SYNTHETIC_ENV).await?;
    let resp = client.tools_list().await?;
    let tools = resp
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;
    let mut names = tools
        .iter()
        .map(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .context("tool name missing")
                .map(str::to_owned)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    names.sort();
    let m1_names = [
        "find",
        "health",
        "observe",
        "read_text",
        "set_capture_target",
        "set_perception_mode",
    ];
    println!("source_of_truth=tools_list edge=m1_names after={names:?}");
    for name in m1_names {
        assert!(names.iter().any(|tool_name| tool_name == name));
    }

    let m1_tools = tools
        .iter()
        .filter(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| m1_names.contains(&name))
        })
        .collect::<Vec<_>>();
    for tool in &m1_tools {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        assert_closed_schema(&tool["inputSchema"], &format!("{name}.inputSchema"));
        if let Some(output) = tool.get("outputSchema") {
            assert_closed_schema(output, &format!("{name}.outputSchema"));
        }
    }
    println!(
        "source_of_truth=tools_list edge=schema_closed after=checked_tools:{}",
        m1_tools.len()
    );

    let projection = m1_tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool["name"],
                "description": tool["description"],
                "inputSchema": tool["inputSchema"],
                "outputSchemaRoot": schema_root(tool.get("outputSchema")),
            })
        })
        .collect::<Vec<_>>();
    insta::assert_json_snapshot!("m1_tools_list", projection);
    assert!(client.shutdown().await?.success());
    Ok(())
}

#[tokio::test]
async fn observe_find_and_read_text_fsv() -> anyhow::Result<()> {
    let _guard = MCP_FSV_LOCK.lock().await;
    let mut client = StdioMcpClient::launch_and_init_with_env(None, SYNTHETIC_ENV).await?;

    verify_observe(&mut client).await?;
    verify_observe_latency(&mut client).await?;
    verify_find(&mut client).await?;
    verify_read_text(&mut client).await?;
    assert!(client.shutdown().await?.success());

    verify_no_perception_error().await?;
    verify_observe_internal_error().await?;
    Ok(())
}

#[tokio::test]
async fn capture_target_and_perception_mode_fsv() -> anyhow::Result<()> {
    let _guard = MCP_FSV_LOCK.lock().await;
    let mut client = StdioMcpClient::launch_and_init_with_env(None, SYNTHETIC_ENV).await?;

    println!("source_of_truth=mcp_capture_target edge=monitor before=primary");
    let monitor_resp = client
        .tools_call(
            "set_capture_target",
            json!({"target": {"kind": "monitor", "monitor_index": 2}}),
        )
        .await?;
    println!(
        "source_of_truth=mcp_capture_target edge=monitor after={}",
        monitor_resp["structuredContent"]
    );
    assert_eq!(
        monitor_resp["structuredContent"]["current"],
        json!({"kind": "monitor", "monitor_index": 2})
    );

    println!("source_of_truth=mcp_capture_target edge=primary_readback before=monitor:2");
    let primary_resp = client
        .tools_call("set_capture_target", json!({"target": {"kind": "primary"}}))
        .await?;
    println!(
        "source_of_truth=mcp_capture_target edge=primary_readback after={}",
        primary_resp["structuredContent"]
    );
    assert_eq!(
        primary_resp["structuredContent"]["previous"],
        json!({"kind": "monitor", "monitor_index": 2})
    );

    println!("source_of_truth=mcp_capture_target edge=invalid_hwnd before=hwnd:0");
    let invalid_hwnd = client
        .tools_call_error(
            "set_capture_target",
            json!({"target": {"kind": "window", "window_hwnd": 0}}),
        )
        .await?;
    println!("source_of_truth=mcp_capture_target edge=invalid_hwnd after={invalid_hwnd}");
    assert_eq!(
        error_code(&invalid_hwnd),
        Some(error_codes::CAPTURE_TARGET_INVALID)
    );

    println!("source_of_truth=mcp_perception_mode edge=hybrid before=mode:auto");
    let mode_resp = client
        .tools_call("set_perception_mode", json!({"mode": "hybrid"}))
        .await?;
    println!(
        "source_of_truth=mcp_perception_mode edge=hybrid after={}",
        mode_resp["structuredContent"]
    );
    assert_eq!(mode_resp["structuredContent"]["mode"], "hybrid");
    let observe_resp = client.tools_call("observe", json!({})).await?;
    let observation: Observation = structured(&observe_resp)?;
    println!(
        "source_of_truth=mcp_perception_mode edge=hybrid_readback after=observe_mode:{:?}",
        observation.mode
    );
    assert_eq!(observation.mode, PerceptionMode::Hybrid);
    verify_hybrid_observe_latency(&mut client).await?;

    println!("source_of_truth=mcp_perception_mode edge=invalid before=telepathy");
    let invalid_mode = client
        .tools_call_error("set_perception_mode", json!({"mode": "telepathy"}))
        .await?;
    println!("source_of_truth=mcp_perception_mode edge=invalid after={invalid_mode}");
    assert_eq!(
        error_code(&invalid_mode),
        Some(error_codes::PERCEPTION_MODE_INVALID)
    );

    println!("source_of_truth=mcp_perception_mode edge=invalid_params before=extra:junk");
    let invalid_params = client
        .tools_call_error(
            "set_perception_mode",
            json!({"mode": "hybrid", "junk": "x"}),
        )
        .await?;
    println!("source_of_truth=mcp_perception_mode edge=invalid_params after={invalid_params}");
    assert_eq!(invalid_params["code"], -32099);
    assert_eq!(
        error_code(&invalid_params),
        Some(error_codes::TOOL_PARAMS_INVALID)
    );
    assert!(client.shutdown().await?.success());
    Ok(())
}

async fn verify_observe(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_observe edge=synthetic_notepad before=include:default");
    let observe_resp = client.tools_call("observe", json!({})).await?;
    let observation: Observation = structured(&observe_resp)?;
    let bytes = serde_json::to_vec(&observation)?.len();
    println!(
        "source_of_truth=mcp_observe edge=synthetic_notepad after=bytes:{bytes} process:{} focused_role:{} mode:{:?}",
        observation.foreground.process_name,
        observation
            .focused
            .as_ref()
            .map_or("", |focused| focused.role.as_str()),
        observation.mode
    );
    assert!(bytes <= 6 * 1024);
    assert_eq!(observation.foreground.process_name, "notepad.exe");
    assert_eq!(
        observation
            .focused
            .as_ref()
            .map(|focused| focused.role.as_str()),
        Some("Edit")
    );
    let focused_bbox = observation
        .focused
        .as_ref()
        .map(|focused| focused.bbox)
        .context("focused element missing")?;
    assert!(focused_bbox.w > 0);
    assert!(focused_bbox.h > 0);

    println!("source_of_truth=mcp_observe edge=include_diagnostics before=include:[diagnostics]");
    let filtered_resp = client
        .tools_call("observe", json!({"include": ["diagnostics"]}))
        .await?;
    let filtered: Observation = structured(&filtered_resp)?;
    println!(
        "source_of_truth=mcp_observe edge=include_diagnostics after=focused:{} elements:{} entities:{}",
        filtered.focused.is_some(),
        filtered.elements.len(),
        filtered.entities.len()
    );
    assert!(filtered.focused.is_none());
    assert!(filtered.elements.is_empty());
    assert!(filtered.entities.is_empty());
    Ok(())
}

async fn verify_observe_latency(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    let mut elapsed_ms = Vec::with_capacity(100);
    println!("source_of_truth=mcp_observe edge=round_trip_p99 before=samples:100");
    for _ in 0..100 {
        let started = std::time::Instant::now();
        let observe_resp = client.tools_call("observe", json!({})).await?;
        let observation: Observation = structured(&observe_resp)?;
        assert_eq!(observation.foreground.process_name, "notepad.exe");
        elapsed_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    elapsed_ms.sort_by(f64::total_cmp);
    let p99 = elapsed_ms[98];
    println!("source_of_truth=mcp_observe edge=round_trip_p99 after=p99_ms:{p99:.3}");
    assert!(p99 <= 50.0, "observe round-trip p99 was {p99:.3} ms");
    Ok(())
}

async fn verify_hybrid_observe_latency(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    let mut elapsed_ms = Vec::with_capacity(100);
    println!("source_of_truth=mcp_perception_mode edge=hybrid_round_trip_p99 before=samples:100");
    for _ in 0..100 {
        let started = std::time::Instant::now();
        let observe_resp = client.tools_call("observe", json!({})).await?;
        let observation: Observation = structured(&observe_resp)?;
        assert_eq!(observation.mode, PerceptionMode::Hybrid);
        assert_eq!(observation.foreground.process_name, "notepad.exe");
        elapsed_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    elapsed_ms.sort_by(f64::total_cmp);
    let p99 = elapsed_ms[98];
    println!(
        "source_of_truth=mcp_perception_mode edge=hybrid_round_trip_p99 after=p99_ms:{p99:.3}"
    );
    assert!(p99 <= 30.0, "hybrid observe round-trip p99 was {p99:.3} ms");
    Ok(())
}

async fn verify_find(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_find edge=role_edit before=role:Edit");
    let find_resp = client
        .tools_call("find", json!({"role": "Edit", "limit": 5}))
        .await?;
    let results = find_resp["structuredContent"]["results"]
        .as_array()
        .context("find results missing")?;
    println!(
        "source_of_truth=mcp_find edge=role_edit after=count:{} first_role:{}",
        results.len(),
        results[0]["role"].as_str().unwrap_or("")
    );
    assert_eq!(results[0]["role"], "Edit");

    println!("source_of_truth=mcp_find edge=no_match before=query:missing-button");
    let no_match_resp = client
        .tools_call("find", json!({"query": "missing-button", "role": "Button"}))
        .await?;
    let no_match = no_match_resp["structuredContent"]["results"]
        .as_array()
        .context("find no-match results missing")?;
    println!(
        "source_of_truth=mcp_find edge=no_match after=count:{}",
        no_match.len()
    );
    assert!(no_match.is_empty());
    Ok(())
}

async fn verify_read_text(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_read_text edge=synthetic_region before=text:Synapse");
    let ocr_resp = client
        .tools_call(
            "read_text",
            json!({"region": {"x": 5, "y": 7, "w": 256, "h": 64}}),
        )
        .await?;
    let ocr: OcrResult = structured(&ocr_resp)?;
    println!(
        "source_of_truth=mcp_read_text edge=synthetic_region after=text:{} words:{}",
        ocr.text,
        ocr.words.len()
    );
    assert_eq!(ocr.text, "Synapse");
    assert_eq!(ocr.words.len(), 1);

    println!(
        "source_of_truth=mcp_read_text edge=element_id before=element_id:0x1234:0000002a00000001"
    );
    let element_ocr_resp = client
        .tools_call(
            "read_text",
            json!({"element_id": "0x1234:0000002a00000001"}),
        )
        .await?;
    let element_ocr: OcrResult = structured(&element_ocr_resp)?;
    println!(
        "source_of_truth=mcp_read_text edge=element_id after=text:{} words:{}",
        element_ocr.text,
        element_ocr.words.len()
    );
    assert_eq!(element_ocr.text, "Synapse");

    println!("source_of_truth=mcp_read_text edge=missing_target before=params:empty");
    let missing_target = client.tools_call_error("read_text", json!({})).await?;
    println!("source_of_truth=mcp_read_text edge=missing_target after={missing_target}");
    assert_eq!(error_code(&missing_target), Some(error_codes::OCR_NO_TEXT));

    println!("source_of_truth=mcp_read_text edge=empty_region before=w:0");
    let empty = client
        .tools_call_error(
            "read_text",
            json!({"region": {"x": 0, "y": 0, "w": 0, "h": 64}}),
        )
        .await?;
    println!("source_of_truth=mcp_read_text edge=empty_region after={empty}");
    assert_eq!(error_code(&empty), Some(error_codes::OCR_NO_TEXT));
    Ok(())
}

async fn verify_no_perception_error() -> anyhow::Result<()> {
    let mut no_perception =
        StdioMcpClient::launch_and_init_with_env(None, &[("SYNAPSE_MCP_FORCE_NO_PERCEPTION", "1")])
            .await?;
    println!("source_of_truth=mcp_observe edge=no_perception before=forced:true");
    let error = no_perception.tools_call_error("observe", json!({})).await?;
    println!("source_of_truth=mcp_observe edge=no_perception after={error}");
    assert_eq!(
        error_code(&error),
        Some(error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE)
    );
    assert!(no_perception.shutdown().await?.success());
    Ok(())
}

async fn verify_observe_internal_error() -> anyhow::Result<()> {
    let mut internal = StdioMcpClient::launch_and_init_with_env(
        None,
        &[("SYNAPSE_MCP_FORCE_OBSERVE_INTERNAL", "1")],
    )
    .await?;
    println!("source_of_truth=mcp_observe edge=internal_error before=forced:true");
    let error = internal.tools_call_error("observe", json!({})).await?;
    println!("source_of_truth=mcp_observe edge=internal_error after={error}");
    assert_eq!(error_code(&error), Some(error_codes::OBSERVE_INTERNAL));
    assert!(internal.shutdown().await?.success());
    Ok(())
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
        "additionalProperties": value.get("additionalProperties"),
        "required": value.get("required"),
    })
}

fn assert_closed_schema(value: &Value, path: &str) {
    match value {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("object")
                && object.contains_key("properties")
            {
                assert_eq!(
                    object.get("additionalProperties"),
                    Some(&Value::Bool(false)),
                    "object schema at {path} must set additionalProperties:false"
                );
            }
            for (key, child) in object {
                assert_closed_schema(child, &format!("{path}.{key}"));
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                assert_closed_schema(child, &format!("{path}[{index}]"));
            }
        }
        _ => {}
    }
}
