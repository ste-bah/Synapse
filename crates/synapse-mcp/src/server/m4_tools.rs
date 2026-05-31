use super::{
    ActComboParams, ActComboResponse, ActLaunchParams, ActLaunchResponse, ActRunShellParams,
    ActRunShellResponse, ErrorData, Json, Parameters, RunShellAuthorization, SynapseService,
    authorize_run_shell, execute_combo, launch, mcp_error, required_combo_permissions,
    run_authorized_shell, run_shell_idempotency_completed_row, run_shell_idempotency_replay,
    run_shell_idempotency_reservation_row, run_shell_idempotency_row_key,
    run_shell_request_details, tool, tool_router,
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
        let params = params.0;
        self.audit_action_started_with_details(
            "act_run_shell",
            &run_shell_request_details(&params),
        )?;
        let result = match authorize_run_shell(&self.m4_config, &params) {
            Ok(authorization) => run_shell_with_idempotency(self, params, authorization).await,
            Err(error) => Err(error),
        };
        self.audit_action_result("act_run_shell", &result)?;
        result.map(Json)
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

async fn run_shell_with_idempotency(
    service: &SynapseService,
    params: ActRunShellParams,
    authorization: RunShellAuthorization,
) -> Result<ActRunShellResponse, ErrorData> {
    let Some(row_key) = run_shell_idempotency_row_key(&params)? else {
        return run_authorized_shell(params, &authorization).await;
    };

    let runtime = service.reflex_runtime()?;
    {
        let runtime = runtime.lock().map_err(|_error| {
            mcp_error(
                synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while checking act_run_shell idempotency",
            )
        })?;
        if let Some(existing) = runtime
            .storage_kv_row(&row_key)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?
        {
            drop(runtime);
            return run_shell_idempotency_replay(&params, &existing);
        }
        let reservation = run_shell_idempotency_reservation_row(&params, &authorization)?;
        runtime
            .storage_put_kv_rows(vec![(row_key.clone(), reservation)])
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    }

    let response = run_authorized_shell(params.clone(), &authorization).await?;
    let completed = run_shell_idempotency_completed_row(&params, &authorization, &response)?;
    {
        let runtime = runtime.lock().map_err(|_error| {
            mcp_error(
                synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while recording act_run_shell idempotency",
            )
        })?;
        runtime
            .storage_put_kv_rows(vec![(row_key, completed)])
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    }
    Ok(response)
}
