#![cfg(debug_assertions)]

use std::time::Duration;

use anyhow::bail;
use serde_json::{Value, json};
use synapse_core::error_codes;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn force_panic_during_act_press_fires_release_all_log() -> anyhow::Result<()> {
    let dir = TempDir::new()?;
    let mut client = StdioMcpClient::launch_and_init_with_env(
        Some(dir.path()),
        &[("SYNAPSE_MCP_FORCE_PANIC_DURING_ACT", "1")],
    )
    .await?;
    let pad = client
        .tools_call(
            "act_pad",
            json!({
                "pad_id": 6,
                "report": {
                    "buttons": ["a"],
                    "rt": 0.5
                }
            }),
        )
        .await?;
    let before_logs = read_logs(dir.path())?;
    println!(
        "source_of_truth=daemon_log edge=force_panic before_panic_count:{} held_pad_id:6 pad_response={pad}",
        safety_reason_lines(&before_logs, "panic").len()
    );

    let forced = tokio::time::timeout(
        Duration::from_secs(2),
        client.tools_call("act_press", json!({"keys": ["a"]})),
    )
    .await;
    println!("source_of_truth=mcp_call edge=force_panic after_trigger={forced:?}");
    assert!(
        !matches!(forced, Ok(Ok(_))),
        "forced panic must not return a successful act_press response"
    );

    let (release_line, panic_line) = wait_for_release_and_panic_lines(dir.path()).await?;
    println!("source_of_truth=daemon_log edge=force_panic after_release_line={release_line}");
    println!("source_of_truth=daemon_log edge=force_panic after_panic_line={panic_line}");
    assert!(release_line.contains("\"reason\":\"tool_invocation\""));
    assert!(release_line.contains("\"held_pad_ids\":\"[6]\""));
    assert!(release_line.contains("\"released_pads\":1"));
    assert!(panic_line.contains("\"reason\":\"panic\""));
    assert!(panic_line.contains("\"timeout_ms\":10"));
    // On Windows the panic-hook release_all reaches the live SoftwareBackend
    // and emits SendInput KeyUp/MouseUp for every held key/button, returning
    // `result=ok`. On non-Windows hosts the SoftwareBackend stub fails-closed
    // with ACTION_BACKEND_UNAVAILABLE — the panic hook still logs the attempt
    // but tags `result=error` so the failure is observable rather than silent.
    if cfg!(windows) {
        assert!(panic_line.contains("\"result\":\"ok\""));
    } else {
        assert!(panic_line.contains("\"result\":\"error\""));
        assert!(panic_line.contains(error_codes::ACTION_BACKEND_UNAVAILABLE));
    }
    drop(client);
    Ok(())
}

async fn wait_for_release_and_panic_lines(
    path: &std::path::Path,
) -> anyhow::Result<(String, String)> {
    for _ in 0..30 {
        let logs = read_logs(path)?;
        let release_line = safety_reason_lines(&logs, "tool_invocation")
            .into_iter()
            .find(|line| line.contains("\"held_pad_ids\":\"[6]\""));
        let panic_line = safety_reason_lines(&logs, "panic").into_iter().last();
        if let (Some(release_line), Some(panic_line)) = (release_line, panic_line) {
            return Ok((release_line, panic_line));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let logs = read_logs(path)?;
    bail!("timed out waiting for force panic safety lines; logs={logs}");
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

fn safety_reason_lines(logs: &str, reason: &str) -> Vec<String> {
    logs.lines()
        .filter(|line| {
            parse_log_fields(line).is_some_and(|fields| {
                fields.get("code").and_then(Value::as_str)
                    == Some(error_codes::SAFETY_RELEASE_ALL_FIRED)
                    && fields.get("reason").and_then(Value::as_str) == Some(reason)
            })
        })
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_log_fields(line: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(line).ok()?;
    Some(value.get("fields")?.clone())
}
