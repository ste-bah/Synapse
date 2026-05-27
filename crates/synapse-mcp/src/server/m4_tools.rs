use super::{
    ActComboParams, ActComboResponse, ActLaunchParams, ActLaunchResponse, ActRunShellParams,
    ActRunShellResponse, ErrorData, Json, Parameters, SynapseService, execute_combo, launch,
    required_combo_permissions, run_shell, tool, tool_router,
};

#[tool_router(router = m4_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(description = "Execute a timed one-shot sequence of already-supported action tools")]
    pub async fn act_combo(
        &self,
        params: Parameters<ActComboParams>,
    ) -> Result<Json<ActComboResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_combo",
            step_count = params.0.steps.len(),
            "tool.invocation kind=act_combo"
        );
        let required = required_combo_permissions(&params.0)?;
        self.require_m3_permissions("act_combo", &required)?;
        let runtime = self.reflex_runtime()?;
        execute_combo(runtime, params.0).await.map(Json)
    }

    #[tool(description = "Run a local shell command only when startup policy permits it")]
    pub async fn act_run_shell(
        &self,
        params: Parameters<ActRunShellParams>,
    ) -> Result<Json<ActRunShellResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_run_shell",
            command = %params.0.command,
            "tool.invocation kind=act_run_shell"
        );
        run_shell(&self.m4_config, params.0).await.map(Json)
    }

    #[tool(description = "Launch an allowlisted local process and optionally wait for a window")]
    pub async fn act_launch(
        &self,
        params: Parameters<ActLaunchParams>,
    ) -> Result<Json<ActLaunchResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_launch",
            target = %params.0.target,
            "tool.invocation kind=act_launch"
        );
        launch(params.0).await.map(Json)
    }
}
