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
        if let Err(error) = self.ensure_supported_use_allows_action("act_combo") {
            self.audit_action_denied("act_combo", &error);
            return Err(error);
        }
        self.refresh_reflex_audit_context()?;
        self.audit_action_started("act_combo")?;
        let runtime = self.reflex_runtime()?;
        let result = execute_combo(runtime, params.0).await;
        self.audit_action_result("act_combo", &result)?;
        result.map(Json)
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
        if let Err(error) = self.ensure_supported_use_allows_action("act_run_shell") {
            self.audit_action_denied("act_run_shell", &error);
            return Err(error);
        }
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
        if let Err(error) = self.ensure_supported_use_allows_action("act_launch") {
            self.audit_action_denied("act_launch", &error);
            return Err(error);
        }
        self.audit_action_started("act_launch")?;
        let result = launch(&self.m4_config, params.0).await;
        self.audit_action_result("act_launch", &result)?;
        result.map(Json)
    }
}
