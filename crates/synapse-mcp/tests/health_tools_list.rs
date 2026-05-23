use anyhow::Context;
use serde_json::Value;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;

#[tokio::test]
async fn health_appears_in_tools_list_with_schema() -> anyhow::Result<()> {
    let mut client = StdioMcpClient::launch_and_init().await?;
    let resp = client.tools_list().await?;
    let tools = resp
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;
    let health_tool = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::String("health".to_owned())))
        .context("health tool missing")?;

    assert_eq!(health_tool["description"], "Return server health");
    assert_eq!(health_tool["inputSchema"]["type"], "object");
    assert!(
        client
            .raw_received()
            .iter()
            .any(|line| line.contains("\"tools\"") && line.contains("\"health\""))
    );
    let status = client.shutdown().await?;
    assert!(status.success());
    Ok(())
}
