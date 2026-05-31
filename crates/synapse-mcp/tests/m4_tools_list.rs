use std::collections::BTreeSet;

use anyhow::{Context, ensure};
use serde_json::{Value, json};
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;

const EXPECTED_TOOLS: [&str; 80] = [
    "act_aim",
    "act_click",
    "act_clipboard",
    "act_combo",
    "act_drag",
    "act_keymap",
    "act_launch",
    "act_pad",
    "act_press",
    "act_run_shell",
    "act_scroll",
    "act_type",
    "audio_tail",
    "audio_transcribe",
    "audit_export_bundle",
    "audit_export_consent_set",
    "audit_intelligence_query",
    "everquest_action_prior_record",
    "everquest_action_prior_scorecard",
    "everquest_autocombat",
    "everquest_chat_input_state",
    "everquest_contextgraph_ingest",
    "everquest_contextgraph_search",
    "everquest_current_state",
    "everquest_domain_normalize",
    "everquest_episode_export",
    "everquest_loc_probe",
    "everquest_map_sensor",
    "everquest_memory_consult",
    "everquest_memory_record",
    "everquest_outcome_ingest",
    "everquest_planner_guard",
    "everquest_predictive_model_fit",
    "everquest_predictive_model_predict",
    "everquest_route_plan",
    "everquest_safe_command",
    "everquest_surprise_detect",
    "everquest_survival_readiness",
    "everquest_trajectory_record",
    "everquest_world_model_inspect",
    "everquest_world_model_record",
    "everquest_world_summary",
    "find",
    "health",
    "observe",
    "observe_delta",
    "profile_activate",
    "profile_authoring_accept",
    "profile_authoring_export",
    "profile_authoring_generate",
    "profile_authoring_inspect",
    "profile_authoring_list",
    "profile_authoring_reject",
    "profile_list",
    "profile_quality_refresh",
    "profile_registry_disable",
    "profile_registry_export",
    "profile_registry_import",
    "profile_registry_inspect",
    "profile_registry_install",
    "profile_registry_report",
    "profile_registry_rollback",
    "profile_registry_search",
    "read_text",
    "reality_audit",
    "reality_baseline",
    "reflex_cancel",
    "reflex_history",
    "reflex_list",
    "reflex_register",
    "release_all",
    "replay_record",
    "set_capture_target",
    "set_perception_mode",
    "storage_gc_once",
    "storage_inspect",
    "storage_pressure_sample",
    "storage_put_probe_rows",
    "subscribe",
    "subscribe_cancel",
];

#[tokio::test]
async fn m4_tools_list_snapshot_defaults_and_closed_schemas() -> anyhow::Result<()> {
    let mut client = StdioMcpClient::launch_and_init().await?;
    let response = client.tools_list().await?;
    let tools = response
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;

    let names = sorted_tool_names(tools)?;
    let expected = EXPECTED_TOOLS
        .iter()
        .copied()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(names, expected);
    assert_eq!(names.len(), 80);
    assert_no_duplicate_names(&names)?;

    assert_schema_roots_closed(tools)?;
    let defaults = m4_default_readbacks(tools)?;

    let snapshot = json!({
        "count": names.len(),
        "tools": names,
        "m4_defaults": defaults,
    });
    insta::assert_json_snapshot!("m4_tools_list", snapshot);

    let status = client.shutdown().await?;
    assert!(status.success());
    Ok(())
}

fn sorted_tool_names(tools: &[Value]) -> anyhow::Result<Vec<String>> {
    let mut names = tools
        .iter()
        .map(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .context("tool name missing")
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

fn assert_no_duplicate_names(names: &[String]) -> anyhow::Result<()> {
    let mut seen = BTreeSet::new();
    for name in names {
        ensure!(seen.insert(name), "duplicate tool name: {name}");
    }
    Ok(())
}

fn assert_schema_roots_closed(tools: &[Value]) -> anyhow::Result<()> {
    for tool in tools {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .context("tool name missing")?;
        for schema_key in ["inputSchema", "outputSchema"] {
            let Some(schema) = tool.get(schema_key) else {
                continue;
            };
            ensure!(
                schema.get("type") == Some(&json!("object")),
                "{name}.{schema_key}.type must be object"
            );
            ensure!(
                schema.get("additionalProperties") == Some(&json!(false)),
                "{name}.{schema_key}.additionalProperties must be false"
            );
        }
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "schema readback table is intentionally kept as one ordered assertion"
)]
fn m4_default_readbacks(tools: &[Value]) -> anyhow::Result<Vec<Value>> {
    let mut readbacks = Vec::new();
    read_default(
        &mut readbacks,
        tools,
        "act_keymap",
        "inputSchema.properties.hold_ms.default",
        &json!(33),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "act_keymap",
        "inputSchema.properties.backend.default",
        &json!("auto"),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "act_click",
        "inputSchema.properties.hold_ms.default",
        &json!(120),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "act_combo",
        "inputSchema.properties.backend.default",
        &json!("auto"),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "act_run_shell",
        "inputSchema.properties.args.default",
        &json!([]),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "act_run_shell",
        "inputSchema.properties.env.default",
        &json!({}),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "act_run_shell",
        "inputSchema.properties.timeout_ms.default",
        &json!(30000),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "act_launch",
        "inputSchema.properties.args.default",
        &json!([]),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "act_launch",
        "inputSchema.properties.env.default",
        &json!({}),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "act_launch",
        "inputSchema.properties.timeout_ms.default",
        &json!(10000),
    )?;
    read_required(&mut readbacks, tools, "act_combo", "steps")?;
    read_required(&mut readbacks, tools, "act_keymap", "alias")?;
    read_required(&mut readbacks, tools, "act_run_shell", "command")?;
    read_required(&mut readbacks, tools, "act_launch", "target")?;
    read_default(
        &mut readbacks,
        tools,
        "everquest_world_summary",
        "inputSchema.properties.profile_id.default",
        &json!("everquest.live"),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "everquest_world_summary",
        "inputSchema.properties.state_row_key.default",
        &json!("everquest/current_state/v1/everquest.live"),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "everquest_world_summary",
        "inputSchema.properties.max_exits.default",
        &json!(5),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "everquest_world_summary",
        "inputSchema.properties.max_landmarks.default",
        &json!(5),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "everquest_world_summary",
        "inputSchema.properties.max_transitions.default",
        &json!(5),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "everquest_world_summary",
        "inputSchema.properties.max_hazards.default",
        &json!(5),
    )?;
    read_default(
        &mut readbacks,
        tools,
        "everquest_world_summary",
        "inputSchema.properties.stale_after_seconds.default",
        &json!(300),
    )?;
    read_required(
        &mut readbacks,
        tools,
        "everquest_world_summary",
        "summary_id",
    )?;
    Ok(readbacks)
}

fn read_default(
    readbacks: &mut Vec<Value>,
    tools: &[Value],
    tool_name: &str,
    path: &str,
    expected: &Value,
) -> anyhow::Result<()> {
    let actual = value_at(tool_by_name(tools, tool_name)?, path)?.clone();
    assert_eq!(&actual, expected, "{tool_name}.{path}");
    readbacks.push(json!({
        "tool": tool_name,
        "path": path,
        "actual": actual,
    }));
    Ok(())
}

fn read_required(
    readbacks: &mut Vec<Value>,
    tools: &[Value],
    tool_name: &str,
    field: &str,
) -> anyhow::Result<()> {
    let required = value_at(tool_by_name(tools, tool_name)?, "inputSchema.required")?;
    let required = required
        .as_array()
        .with_context(|| format!("{tool_name}.inputSchema.required must be an array"))?;
    ensure!(
        required.iter().any(|value| value.as_str() == Some(field)),
        "{tool_name}.inputSchema.required must contain {field}"
    );
    readbacks.push(json!({
        "tool": tool_name,
        "path": "inputSchema.required",
        "actual_contains": field,
    }));
    Ok(())
}

fn tool_by_name<'a>(tools: &'a [Value], name: &str) -> anyhow::Result<&'a Value> {
    tools
        .iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
        .with_context(|| format!("tool missing from tools/list: {name}"))
}

fn value_at<'a>(value: &'a Value, path: &str) -> anyhow::Result<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current
            .get(segment)
            .with_context(|| format!("missing path {path}"))?;
    }
    Ok(current)
}
