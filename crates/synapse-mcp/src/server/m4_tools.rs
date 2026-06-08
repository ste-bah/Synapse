use super::{
    ActComboParams, ActComboResponse, ActLaunchParams, ActLaunchResponse,
    ActRunShellCancelResponse, ActRunShellJobIdParams, ActRunShellParams, ActRunShellResponse,
    ActRunShellStartParams, ActRunShellStartResponse, ActRunShellStatusParams,
    ActRunShellStatusResponse, ActSpawnAgentCli, ActSpawnAgentLogPaths, ActSpawnAgentParams,
    ActSpawnAgentResponse, ActSpawnAgentTarget, ErrorData, Json, LaunchWindowState, Parameters,
    RunShellAuthorization, ShellExecutionContext, SynapseService, assign_owned_process_job,
    authorize_run_shell, authorize_run_shell_start, cancel_shell_job, execute_combo, launch,
    launch_process_history_row, launch_process_history_row_key, launch_request_details, mcp_error,
    prepare_run_shell_params_for_context, prepare_run_shell_start_params_for_context,
    required_combo_permissions, run_authorized_shell, run_shell_idempotency_completed_row,
    run_shell_idempotency_replay, run_shell_idempotency_reservation_row,
    run_shell_idempotency_row_key, run_shell_request_details, run_shell_start_request_details,
    shell_execution_context_for_session, shell_job_status, start_authorized_shell_job, tool,
    tool_router, validate_agent_spawn_params,
};

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use rmcp::{RoleServer, model::ErrorCode, service::RequestContext};
use serde_json::json;
use synapse_core::{error_codes, new_reflex_id};

use super::{
    m1_tools::validate_target_window,
    session_registry::{SpawnedAgentRead, unix_time_ms_now},
};

const ACT_SPAWN_AGENT: &str = "act_spawn_agent";
const AGENT_SPAWN_LAUNCH_TARGET: &str = "pwsh.exe";
const AGENT_SPAWN_POLL_INTERVAL_MS: u64 = 250;
const AGENT_SPAWN_LOG_TAIL_BYTES: usize = 8 * 1024;

#[tool_router(router = m4_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Execute a timed one-shot sequence of act_press key steps only; use act_stroke for continuous mouse motion/path execution"
    )]
    pub async fn act_combo(
        &self,
        params: Parameters<ActComboParams>,
        request_context: RequestContext<RoleServer>,
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
            self.audit_action_denied_for_request("act_combo", &error, &request_context);
            return Err(error);
        }
        self.refresh_reflex_audit_context()?;
        self.audit_action_started_for_request("act_combo", &request_context)?;
        let runtime = self.reflex_runtime()?;
        let result = execute_combo(runtime, params.0).await;
        self.audit_action_result_for_request("act_combo", &result, &request_context)?;
        result.map(Json)
    }

    #[tool(
        description = "Run an allowlisted executable child process. command is an executable path/name only; pass flags and shell snippets in args, using an explicit shell executable when shell syntax is required. Requests with timeout_ms above the inline await budget return immediately as a durable background job with job_id/status/stdout/stderr paths; poll act_run_shell_status and cancel with act_run_shell_cancel."
    )]
    pub async fn act_run_shell(
        &self,
        params: Parameters<ActRunShellParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActRunShellResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_run_shell",
            command = %params.0.command,
            "tool.invocation kind=act_run_shell"
        );
        if let Err(error) = self.ensure_supported_use_allows_action("act_run_shell") {
            self.audit_action_denied_for_request("act_run_shell", &error, &request_context);
            return Err(error);
        }
        let raw_params = params.0;
        let session_id = require_shell_session_id(&request_context)?;
        let shell_context = shell_execution_context_for_session(&session_id)?;
        let params = prepare_run_shell_params_for_context(raw_params, &shell_context)?;
        self.audit_action_started_with_details_for_session(
            "act_run_shell",
            &run_shell_request_details(&params, self.m4_config.run_shell_inline_await_limit_ms()),
            &session_id,
        )?;
        let result = match authorize_run_shell(&self.m4_config, &params) {
            Ok(authorization) => {
                run_shell_with_idempotency(
                    self,
                    params,
                    authorization,
                    self.m4_config.run_shell_inline_await_limit_ms(),
                    Some(&shell_context),
                )
                .await
            }
            Err(error) => Err(error),
        };
        self.audit_action_result_for_session("act_run_shell", &result, &session_id)?;
        result.map(Json)
    }

    #[tool(
        description = "Start an allowlisted executable as a durable background shell job. Returns immediately with a job id plus status/stdout/stderr file paths; use act_run_shell_status to poll and act_run_shell_cancel to terminate by job id."
    )]
    pub async fn act_run_shell_start(
        &self,
        params: Parameters<ActRunShellStartParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActRunShellStartResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_run_shell_start",
            command = %params.0.command,
            "tool.invocation kind=act_run_shell_start"
        );
        if let Err(error) = self.ensure_supported_use_allows_action("act_run_shell") {
            self.audit_action_denied_for_request("act_run_shell_start", &error, &request_context);
            return Err(error);
        }
        let raw_params = params.0;
        let session_id = require_shell_session_id(&request_context)?;
        let shell_context = shell_execution_context_for_session(&session_id)?;
        let params = prepare_run_shell_start_params_for_context(raw_params, &shell_context)?;
        self.audit_action_started_with_details_for_session(
            "act_run_shell_start",
            &run_shell_start_request_details(&params),
            &session_id,
        )?;
        let result = match authorize_run_shell_start(&self.m4_config, &params) {
            Ok(authorization) => {
                start_authorized_shell_job(params, &authorization, Some(&shell_context))
            }
            Err(error) => Err(error),
        };
        self.audit_action_result_for_session("act_run_shell_start", &result, &session_id)?;
        result.map(Json)
    }

    #[tool(
        description = "Read durable background shell job status and tails of persisted stdout/stderr logs by job id. This is a separate source-of-truth readback and does not wait for the process to finish."
    )]
    pub async fn act_run_shell_status(
        &self,
        params: Parameters<ActRunShellStatusParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActRunShellStatusResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_run_shell_status",
            job_id = %params.0.job_id,
            "tool.invocation kind=act_run_shell_status"
        );
        if let Err(error) = self.ensure_supported_use_allows_action("act_run_shell") {
            self.audit_action_denied_for_request("act_run_shell_status", &error, &request_context);
            return Err(error);
        }
        let params = params.0;
        let session_id = require_shell_session_id(&request_context)?;
        self.audit_action_started_with_details_for_session(
            "act_run_shell_status",
            &json!({
                "job_id": &params.job_id,
                "tail_bytes": params.tail_bytes,
                "session_id": &session_id,
            }),
            &session_id,
        )?;
        let result = shell_job_status(&params, Some(&session_id));
        self.audit_action_result_for_session("act_run_shell_status", &result, &session_id)?;
        result.map(Json)
    }

    #[tool(
        description = "Cancel a durable background shell job by exact job id, terminating only the recorded job-owned process tree and returning status/log/process readback."
    )]
    pub async fn act_run_shell_cancel(
        &self,
        params: Parameters<ActRunShellJobIdParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActRunShellCancelResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_run_shell_cancel",
            job_id = %params.0.job_id,
            "tool.invocation kind=act_run_shell_cancel"
        );
        if let Err(error) = self.ensure_supported_use_allows_action("act_run_shell") {
            self.audit_action_denied_for_request("act_run_shell_cancel", &error, &request_context);
            return Err(error);
        }
        let params = params.0;
        let session_id = require_shell_session_id(&request_context)?;
        self.audit_action_started_with_details_for_session(
            "act_run_shell_cancel",
            &json!({
                "job_id": &params.job_id,
                "session_id": &session_id,
            }),
            &session_id,
        )?;
        let result = cancel_shell_job(&params, Some(&session_id));
        self.audit_action_result_for_session("act_run_shell_cancel", &result, &session_id)?;
        result.map(Json)
    }

    #[tool(description = "Launch an allowlisted local process and optionally wait for a window")]
    pub async fn act_launch(
        &self,
        params: Parameters<ActLaunchParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActLaunchResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "act_launch",
            target = %params.0.target,
            "tool.invocation kind=act_launch"
        );
        if let Err(error) = self.ensure_supported_use_allows_action("act_launch") {
            self.audit_action_denied_for_request("act_launch", &error, &request_context);
            return Err(error);
        }
        let params = params.0;
        let session_id = super::context::mcp_session_id_from_request_context(&request_context)?;
        if let Some(session_id) = session_id.as_deref() {
            self.audit_action_started_with_details_for_session(
                "act_launch",
                &launch_request_details(&params),
                session_id,
            )?;
        } else {
            self.audit_action_started_with_details("act_launch", &launch_request_details(&params))?;
        }
        let result = match launch(&self.m4_config, params.clone()).await {
            Ok(response) => {
                let process_job = if session_id.is_some() {
                    match assign_owned_process_job(response.pid, "act_launch", None) {
                        Ok(process_job) => Some(process_job),
                        Err(error) => {
                            let cleanup = crate::m4::terminate_owned_process_tree(response.pid);
                            return Err(launch_lifecycle_tool_error(
                                "act_launch spawned the process but failed to assign a session process job; exact spawned PID cleanup was attempted",
                                json!({
                                    "code": error_codes::TOOL_INTERNAL_ERROR,
                                    "reason": "process_job_assign_failed",
                                    "pid": response.pid,
                                    "source_error": error.message,
                                    "cleanup": cleanup,
                                }),
                            ));
                        }
                    }
                } else {
                    None
                };
                if let Err(error) = record_launch_process_history(self, &params, &response) {
                    let cleanup = crate::m4::terminate_owned_process_tree(response.pid);
                    return Err(launch_lifecycle_tool_error(
                        "act_launch spawned the process but failed to record process history; exact spawned PID cleanup was attempted",
                        json!({
                            "code": error_codes::TOOL_INTERNAL_ERROR,
                            "reason": "process_history_record_failed",
                            "pid": response.pid,
                            "source_error": error.message,
                            "cleanup": cleanup,
                        }),
                    ));
                }
                if let (Some(session_id), Some(process_job)) = (session_id.clone(), process_job) {
                    if let Err(error) = self.register_session_process_resource(
                        super::session_lifecycle::SessionProcessResource::new(
                            session_id,
                            "act_launch",
                            response.pid,
                            None,
                            params.target.clone(),
                            process_job,
                        ),
                    ) {
                        let cleanup = crate::m4::terminate_owned_process_tree(response.pid);
                        return Err(launch_lifecycle_tool_error(
                            "act_launch spawned the process but failed to register the session process resource; exact spawned PID cleanup was attempted",
                            json!({
                                "code": error_codes::TOOL_INTERNAL_ERROR,
                                "reason": "session_process_register_failed",
                                "pid": response.pid,
                                "source_error": error.message,
                                "cleanup": cleanup,
                            }),
                        ));
                    }
                }
                Ok(response)
            }
            Err(error) => Err(error),
        };
        if let Some(session_id) = session_id.as_deref() {
            self.audit_action_result_for_session("act_launch", &result, session_id)?;
        } else {
            self.audit_action_result("act_launch", &result)?;
        }
        result.map(Json)
    }

    #[tool(
        description = "Spawn a fully capable primary Codex or Claude agent as a hidden background process, wire it to the configured Synapse HTTP MCP daemon, require real MCP session registration, optionally bind a per-session target, and return only after session_list readback proves the spawned session exists."
    )]
    pub async fn act_spawn_agent(
        &self,
        params: Parameters<ActSpawnAgentParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActSpawnAgentResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = ACT_SPAWN_AGENT,
            cli = params.0.cli.as_str(),
            "tool.invocation kind=act_spawn_agent"
        );
        if let Err(error) = self.ensure_supported_use_allows_action("act_launch") {
            self.audit_action_denied_for_request(ACT_SPAWN_AGENT, &error, &request_context);
            return Err(error);
        }
        let started_by_session_id =
            super::context::mcp_session_id_from_request_context(&request_context)?;
        let params = params.0;
        self.audit_action_started_with_details_for_request(
            ACT_SPAWN_AGENT,
            &agent_spawn_request_details(&params, started_by_session_id.as_deref()),
            &request_context,
        )?;
        let result = self
            .act_spawn_agent_impl(params, started_by_session_id)
            .await;
        self.audit_action_result_for_request(ACT_SPAWN_AGENT, &result, &request_context)?;
        result.map(Json)
    }
}

fn record_launch_process_history(
    service: &SynapseService,
    params: &ActLaunchParams,
    response: &ActLaunchResponse,
) -> Result<(), ErrorData> {
    let row = launch_process_history_row(params, response)?;
    let row_key = launch_process_history_row_key(response);
    let runtime = service.reflex_runtime()?;
    let runtime = runtime.lock().map_err(|_error| {
        mcp_error(
            synapse_core::error_codes::TOOL_INTERNAL_ERROR,
            "reflex runtime lock poisoned while recording act_launch process history",
        )
    })?;
    runtime
        .storage_put_process_history_rows(vec![(row_key, row)])
        .map_err(|error| mcp_error(error.code(), error.to_string()))
}

fn require_shell_session_id(
    request_context: &RequestContext<RoleServer>,
) -> Result<String, ErrorData> {
    super::context::mcp_session_id_from_request_context(request_context)?.ok_or_else(|| {
        mcp_error(
            error_codes::HTTP_SESSION_INVALID,
            "act_run_shell tools require an MCP session id (run the daemon in HTTP mode so each agent has its own Mcp-Session-Id)",
        )
    })
}

impl SynapseService {
    async fn act_spawn_agent_impl(
        &self,
        params: ActSpawnAgentParams,
        started_by_session_id: Option<String>,
    ) -> Result<ActSpawnAgentResponse, ErrorData> {
        validate_agent_spawn_params(&params)?;
        validate_spawn_target(&params.target)?;

        let working_dir = resolve_agent_working_dir(params.working_dir.as_deref())?;
        let token = read_synapse_bearer_token()?;
        let before_session_ids = self.current_session_ids()?;
        let spawn_id = format!("agent-spawn-{}", new_reflex_id());
        let launched_at_unix_ms = unix_time_ms_now();
        let files = prepare_agent_spawn_files(&spawn_id, &params, &working_dir)?;
        let script = agent_spawn_powershell_script(&params, &files, &working_dir)?;

        let mut env = BTreeMap::new();
        env.insert("SYNAPSE_BEARER_TOKEN".to_owned(), token);
        env.insert("SYNAPSE_AGENT_SPAWN_ID".to_owned(), spawn_id.clone());
        env.insert(
            "SYNAPSE_AGENT_KIND".to_owned(),
            params.cli.as_str().to_owned(),
        );
        env.insert("SYNAPSE_MCP_URL".to_owned(), params.mcp_url.clone());

        let launch_params = ActLaunchParams {
            target: AGENT_SPAWN_LAUNCH_TARGET.to_owned(),
            args: vec![
                "-NoLogo".to_owned(),
                "-NoProfile".to_owned(),
                "-NonInteractive".to_owned(),
                "-ExecutionPolicy".to_owned(),
                "Bypass".to_owned(),
                "-Command".to_owned(),
                script,
            ],
            working_dir: Some(working_dir.display().to_string()),
            env,
            wait_for_window_title_regex: None,
            timeout_ms: 10_000,
            idempotency_key: None,
            cdp_debug: Some(false),
            force_renderer_accessibility: None,
            windows_console_window_state: Some(LaunchWindowState::Hidden),
        };

        let launch_response = launch(&self.m4_config, launch_params.clone()).await?;
        let process_job = match assign_owned_process_job(
            launch_response.pid,
            ACT_SPAWN_AGENT,
            Some(&spawn_id),
        ) {
            Ok(process_job) => process_job,
            Err(error) => {
                let cleanup = crate::m4::terminate_owned_process_tree(launch_response.pid);
                return Err(agent_spawn_tool_error(
                    error_codes::ACTION_AGENT_SPAWN_FAILED,
                    "act_spawn_agent spawned the wrapper but failed to assign a session process job; exact spawned PID cleanup was attempted",
                    json!({
                        "code": error_codes::ACTION_AGENT_SPAWN_FAILED,
                        "reason": "process_job_assign_failed",
                        "spawn_id": spawn_id,
                        "cli": params.cli.as_str(),
                        "launcher_process_id": launch_response.pid,
                        "log_dir": files.log_dir.display().to_string(),
                        "source_error": error.message,
                        "cleanup": cleanup,
                    }),
                ));
            }
        };
        if let Err(error) = record_launch_process_history(self, &launch_params, &launch_response) {
            let cleanup = crate::m4::terminate_owned_process_tree(launch_response.pid);
            return Err(agent_spawn_tool_error(
                error_codes::ACTION_AGENT_SPAWN_FAILED,
                "act_spawn_agent spawned the wrapper but failed to record process history; exact spawned PID cleanup was attempted",
                json!({
                    "code": error_codes::ACTION_AGENT_SPAWN_FAILED,
                    "reason": "process_history_record_failed",
                    "spawn_id": spawn_id,
                    "cli": params.cli.as_str(),
                    "launcher_process_id": launch_response.pid,
                    "log_dir": files.log_dir.display().to_string(),
                    "source_error": error.message,
                    "cleanup": cleanup,
                }),
            ));
        }

        let matched = match self
            .wait_for_spawned_agent_session(
                &params,
                &before_session_ids,
                launched_at_unix_ms,
                launch_response.pid,
                &files,
            )
            .await
        {
            Ok(matched) => matched,
            Err(error) => {
                let cleanup = crate::m4::terminate_owned_process_tree(launch_response.pid);
                return Err(agent_spawn_tool_error(
                    error_codes::ACTION_AGENT_SPAWN_SESSION_TIMEOUT,
                    "act_spawn_agent did not observe a fully provisioned MCP session before timeout; exact spawned PID cleanup was attempted",
                    json!({
                        "code": error_codes::ACTION_AGENT_SPAWN_SESSION_TIMEOUT,
                        "reason": "session_registry_readback_timeout",
                        "spawn_id": spawn_id,
                        "cli": params.cli.as_str(),
                        "launcher_process_id": launch_response.pid,
                        "mcp_url": params.mcp_url,
                        "wait_timeout_ms": params.wait_timeout_ms,
                        "target": params.target,
                        "log_dir": files.log_dir.display().to_string(),
                        "stdout_tail": tail_file_lossy(&files.stdout_path, AGENT_SPAWN_LOG_TAIL_BYTES),
                        "stderr_tail": tail_file_lossy(&files.stderr_path, AGENT_SPAWN_LOG_TAIL_BYTES),
                        "final_message_tail": tail_file_lossy(&files.final_message_path, AGENT_SPAWN_LOG_TAIL_BYTES),
                        "wait_error": error,
                        "cleanup": cleanup,
                    }),
                ));
            }
        };

        let metadata = SpawnedAgentRead {
            spawn_id: spawn_id.clone(),
            cli: params.cli.as_str().to_owned(),
            launcher_process_id: launch_response.pid,
            agent_process_id: matched.agent_process_id,
            started_by_session_id,
            launched_at_unix_ms,
            launch_target: launch_params.target.clone(),
            log_dir: files.log_dir.display().to_string(),
        };
        if let Err(error) = self.record_spawned_agent_metadata(&matched.session_id, metadata) {
            let cleanup = crate::m4::terminate_owned_process_tree(launch_response.pid);
            return Err(agent_spawn_tool_error(
                error_codes::ACTION_AGENT_SPAWN_FAILED,
                "act_spawn_agent observed the spawned session but failed to record session metadata; exact spawned PID cleanup was attempted",
                json!({
                    "code": error_codes::ACTION_AGENT_SPAWN_FAILED,
                    "reason": "spawned_agent_metadata_record_failed",
                    "spawn_id": spawn_id,
                    "cli": params.cli.as_str(),
                    "launcher_process_id": launch_response.pid,
                    "agent_process_id": matched.agent_process_id,
                    "session_id": matched.session_id,
                    "log_dir": files.log_dir.display().to_string(),
                    "source_error": error.message,
                    "cleanup": cleanup,
                }),
            ));
        }
        if let Err(error) = self.register_session_process_resource(
            super::session_lifecycle::SessionProcessResource::new(
                matched.session_id.clone(),
                ACT_SPAWN_AGENT,
                launch_response.pid,
                Some(spawn_id.clone()),
                launch_params.target.clone(),
                process_job,
            ),
        ) {
            let cleanup = crate::m4::terminate_owned_process_tree(launch_response.pid);
            return Err(agent_spawn_tool_error(
                error_codes::ACTION_AGENT_SPAWN_FAILED,
                "act_spawn_agent observed the spawned session but failed to register the session process resource; exact spawned PID cleanup was attempted",
                json!({
                    "code": error_codes::ACTION_AGENT_SPAWN_FAILED,
                    "reason": "session_process_register_failed",
                    "spawn_id": spawn_id,
                    "cli": params.cli.as_str(),
                    "launcher_process_id": launch_response.pid,
                    "agent_process_id": matched.agent_process_id,
                    "session_id": matched.session_id,
                    "log_dir": files.log_dir.display().to_string(),
                    "source_error": error.message,
                    "cleanup": cleanup,
                }),
            ));
        }

        Ok(ActSpawnAgentResponse {
            spawn_id,
            cli: params.cli,
            launcher_process_id: launch_response.pid,
            agent_process_id: matched.agent_process_id,
            session_id: matched.session_id,
            mcp_url: params.mcp_url,
            working_dir: working_dir.display().to_string(),
            launched_at_unix_ms,
            registered_at_unix_ms: matched.registered_at_unix_ms,
            target: params.target,
            log_paths: files.to_response(),
        })
    }

    fn current_session_ids(&self) -> Result<BTreeSet<String>, ErrorData> {
        Ok(self
            .session_list_impl(true)?
            .sessions
            .into_iter()
            .map(|summary| summary.registry.session_id)
            .collect())
    }

    async fn wait_for_spawned_agent_session(
        &self,
        params: &ActSpawnAgentParams,
        before_session_ids: &BTreeSet<String>,
        launched_at_unix_ms: u64,
        launcher_pid: u32,
        files: &AgentSpawnFiles,
    ) -> Result<MatchedSpawnSession, serde_json::Value> {
        let started = Instant::now();
        let timeout = Duration::from_millis(u64::from(params.wait_timeout_ms));
        let mut last_observed = json!({
            "reason": "no_matching_session_observed",
            "sessions": [],
        });
        while started.elapsed() < timeout {
            let list = self.session_list_impl(true).map_err(|error| {
                json!({
                    "reason": "session_list_read_failed",
                    "error": error.message,
                    "data": error.data,
                })
            })?;
            let sessions_json = list
                .sessions
                .iter()
                .map(|summary| {
                    json!({
                        "session_id": summary.registry.session_id,
                        "agent_kind": summary.registry.agent_kind,
                        "client_name": summary.registry.client_name,
                        "lifecycle": summary.registry.lifecycle,
                        "started_at_unix_ms": summary.registry.started_at_unix_ms,
                        "last_action": summary.registry.last_action,
                        "active_target": summary.active_target,
                    })
                })
                .collect::<Vec<_>>();
            last_observed = json!({
                "reason": "candidate_not_ready",
                "sessions": sessions_json,
            });

            for summary in list.sessions {
                if !spawn_session_candidate_matches(
                    &summary,
                    params,
                    before_session_ids,
                    launched_at_unix_ms,
                ) {
                    continue;
                }
                let agent_process_id = discover_agent_process_id(launcher_pid, params.cli);
                return Ok(MatchedSpawnSession {
                    session_id: summary.registry.session_id,
                    registered_at_unix_ms: unix_time_ms_now(),
                    agent_process_id,
                });
            }

            if process_has_exited(launcher_pid) {
                return Err(json!({
                    "reason": "launcher_process_exited_before_registry_match",
                    "launcher_process_id": launcher_pid,
                    "stdout_tail": tail_file_lossy(&files.stdout_path, AGENT_SPAWN_LOG_TAIL_BYTES),
                    "stderr_tail": tail_file_lossy(&files.stderr_path, AGENT_SPAWN_LOG_TAIL_BYTES),
                    "final_message_tail": tail_file_lossy(&files.final_message_path, AGENT_SPAWN_LOG_TAIL_BYTES),
                    "last_observed": last_observed,
                }));
            }
            tokio::time::sleep(Duration::from_millis(AGENT_SPAWN_POLL_INTERVAL_MS)).await;
        }
        Err(last_observed)
    }

    fn record_spawned_agent_metadata(
        &self,
        session_id: &str,
        metadata: SpawnedAgentRead,
    ) -> Result<(), ErrorData> {
        let mut registry = self.session_registry_ref().lock().map_err(|_error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "session registry lock poisoned while recording spawned agent metadata",
            )
        })?;
        registry.record_spawned_agent(session_id, metadata, unix_time_ms_now());
        Ok(())
    }
}

#[derive(Debug)]
struct MatchedSpawnSession {
    session_id: String,
    registered_at_unix_ms: u64,
    agent_process_id: Option<u32>,
}

#[derive(Debug)]
struct AgentSpawnFiles {
    log_dir: PathBuf,
    prompt_path: PathBuf,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    final_message_path: PathBuf,
    debug_path: Option<PathBuf>,
    mcp_config_path: Option<PathBuf>,
}

impl AgentSpawnFiles {
    fn to_response(&self) -> ActSpawnAgentLogPaths {
        ActSpawnAgentLogPaths {
            log_dir: self.log_dir.display().to_string(),
            prompt_path: self.prompt_path.display().to_string(),
            stdout_path: self.stdout_path.display().to_string(),
            stderr_path: self.stderr_path.display().to_string(),
            final_message_path: self.final_message_path.display().to_string(),
            debug_path: self
                .debug_path
                .as_ref()
                .map(|path| path.display().to_string()),
            mcp_config_path: self
                .mcp_config_path
                .as_ref()
                .map(|path| path.display().to_string()),
        }
    }
}

fn agent_spawn_request_details(
    params: &ActSpawnAgentParams,
    started_by_session_id: Option<&str>,
) -> serde_json::Value {
    json!({
        "cli": params.cli.as_str(),
        "target": params.target,
        "working_dir": params.working_dir,
        "mcp_url": params.mcp_url,
        "wait_timeout_ms": params.wait_timeout_ms,
        "hold_open_ms": params.hold_open_ms,
        "prompt_present": params.prompt.as_ref().is_some_and(|prompt| !prompt.is_empty()),
        "prompt_bytes": params.prompt.as_ref().map_or(0, String::len),
        "started_by_session_id": started_by_session_id,
        "required_foreground": false,
        "launch_target": AGENT_SPAWN_LAUNCH_TARGET,
        "windows_console_window_state": "hidden",
    })
}

fn validate_spawn_target(target: &Option<ActSpawnAgentTarget>) -> Result<(), ErrorData> {
    match target {
        Some(ActSpawnAgentTarget::Window { window_hwnd })
        | Some(ActSpawnAgentTarget::Cdp { window_hwnd, .. }) => {
            validate_target_window(*window_hwnd)?;
        }
        None => {}
    }
    Ok(())
}

fn resolve_agent_working_dir(working_dir: Option<&str>) -> Result<PathBuf, ErrorData> {
    let path = match working_dir {
        Some(path) => PathBuf::from(path),
        None => std::env::current_dir().map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("act_spawn_agent failed to read daemon current directory: {error}"),
            )
        })?,
    };
    let canonical = fs::canonicalize(&path).map_err(|error| {
        mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!(
                "act_spawn_agent working_dir {:?} could not be resolved: {error}",
                path
            ),
        )
    })?;
    if !canonical.is_dir() {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!(
                "act_spawn_agent working_dir {:?} is not a directory",
                canonical
            ),
        ));
    }
    Ok(canonical)
}

fn read_synapse_bearer_token() -> Result<String, ErrorData> {
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| {
        mcp_error(
            error_codes::STORAGE_READ_FAILED,
            "act_spawn_agent requires APPDATA to locate the Synapse bearer token file",
        )
    })?;
    let token_path = PathBuf::from(appdata).join("synapse").join("token.txt");
    let token = fs::read_to_string(&token_path).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!(
                "act_spawn_agent failed to read Synapse bearer token at {}: {error}",
                token_path.display()
            ),
        )
    })?;
    let token = token.trim().to_owned();
    if token.is_empty() {
        return Err(mcp_error(
            error_codes::HTTP_TOKEN_INVALID,
            format!(
                "act_spawn_agent read an empty Synapse bearer token at {}",
                token_path.display()
            ),
        ));
    }
    Ok(token)
}

fn prepare_agent_spawn_files(
    spawn_id: &str,
    params: &ActSpawnAgentParams,
    working_dir: &Path,
) -> Result<AgentSpawnFiles, ErrorData> {
    let root = agent_spawn_root_dir()?;
    let log_dir = root.join(spawn_id);
    fs::create_dir_all(&log_dir).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_WRITE_FAILED,
            format!(
                "act_spawn_agent failed to create log directory {}: {error}",
                log_dir.display()
            ),
        )
    })?;
    let prompt_path = log_dir.join("prompt.txt");
    let stdout_path = log_dir.join("stdout.jsonl");
    let stderr_path = log_dir.join("stderr.log");
    let final_message_path = log_dir.join("final-message.txt");
    let debug_path =
        (params.cli == ActSpawnAgentCli::Claude).then(|| log_dir.join("claude-debug.log"));
    let mcp_config_path =
        (params.cli == ActSpawnAgentCli::Claude).then(|| log_dir.join("claude-mcp-config.json"));

    let prompt = build_agent_spawn_prompt(spawn_id, params, working_dir)?;
    fs::write(&prompt_path, prompt).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_WRITE_FAILED,
            format!(
                "act_spawn_agent failed to write prompt file {}: {error}",
                prompt_path.display()
            ),
        )
    })?;
    if let Some(config_path) = &mcp_config_path {
        let config = json!({
            "mcpServers": {
                "synapse": {
                    "type": "http",
                    "url": params.mcp_url,
                    "headers": {
                        "Authorization": "Bearer ${SYNAPSE_BEARER_TOKEN}"
                    }
                }
            }
        });
        let encoded = serde_json::to_vec_pretty(&config).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("act_spawn_agent failed to encode Claude MCP config: {error}"),
            )
        })?;
        fs::write(config_path, encoded).map_err(|error| {
            mcp_error(
                error_codes::STORAGE_WRITE_FAILED,
                format!(
                    "act_spawn_agent failed to write Claude MCP config {}: {error}",
                    config_path.display()
                ),
            )
        })?;
    }

    Ok(AgentSpawnFiles {
        log_dir,
        prompt_path,
        stdout_path,
        stderr_path,
        final_message_path,
        debug_path,
        mcp_config_path,
    })
}

fn agent_spawn_root_dir() -> Result<PathBuf, ErrorData> {
    let local_appdata = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        mcp_error(
            error_codes::STORAGE_WRITE_FAILED,
            "act_spawn_agent requires LOCALAPPDATA to create per-agent log directories",
        )
    })?;
    Ok(PathBuf::from(local_appdata)
        .join("Synapse")
        .join("agent-spawns"))
}

fn build_agent_spawn_prompt(
    spawn_id: &str,
    params: &ActSpawnAgentParams,
    working_dir: &Path,
) -> Result<String, ErrorData> {
    let target_instruction = match &params.target {
        Some(target) => {
            let target_json = serde_json::to_string(target).map_err(|error| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    format!("act_spawn_agent failed to encode target prompt JSON: {error}"),
                )
            })?;
            format!(
                "3. Call the Synapse MCP set_target tool with exactly this JSON as the target value: {target_json}\n4. Call the Synapse MCP get_target tool and verify its current target exactly matches that JSON."
            )
        }
        None => {
            "3. Call the Synapse MCP get_target tool and report the returned session_id/current target.".to_owned()
        }
    };
    let assigned_prompt = params.prompt.as_deref().unwrap_or("").trim();
    let assigned_block = if assigned_prompt.is_empty() {
        "No additional task was provided; perform only the provisioning checks above.".to_owned()
    } else {
        format!(
            "After the provisioning checks pass, perform this assigned task:\n{assigned_prompt}"
        )
    };
    let hold_instruction = if params.hold_open_ms == 0 {
        "Do not add an artificial hold-open sleep after completing the provisioning checks and assigned task.".to_owned()
    } else {
        format!(
            "After the provisioning checks and assigned task, keep this primary process alive for at least {} ms using a normal local shell sleep command, then finish.",
            params.hold_open_ms
        )
    };
    Ok(format!(
        "You are a primary {cli} agent spawned by Synapse act_spawn_agent.\n\
Spawn ID: {spawn_id}\n\
Working directory: {working_dir}\n\
Mandatory provisioning checks:\n\
1. Use the real Synapse MCP health tool through your normal configured MCP client. Do not use curl, direct HTTP, helper scripts, or local storage writes as a substitute.\n\
2. Use the real Synapse MCP session_list tool through your normal configured MCP client.\n\
{target_instruction}\n\
5. If any Synapse MCP tool is missing or fails, stop and report the exact tool/error.\n\
6. In your final response, include one compact JSON object containing spawn_id, health_ok, session_id, target_ok, and any error.\n\
\n\
{assigned_block}\n\
\n\
{hold_instruction}\n",
        cli = params.cli.as_str(),
        spawn_id = spawn_id,
        working_dir = working_dir.display(),
        target_instruction = target_instruction,
        assigned_block = assigned_block,
        hold_instruction = hold_instruction,
    ))
}

fn agent_spawn_powershell_script(
    params: &ActSpawnAgentParams,
    files: &AgentSpawnFiles,
    working_dir: &Path,
) -> Result<String, ErrorData> {
    let prompt_path = ps_single_quoted_path(&files.prompt_path);
    let stdout_path = ps_single_quoted_path(&files.stdout_path);
    let stderr_path = ps_single_quoted_path(&files.stderr_path);
    let final_message_path = ps_single_quoted_path(&files.final_message_path);
    let working_dir = ps_single_quoted_path(working_dir);
    let script = match params.cli {
        ActSpawnAgentCli::Codex => {
            let mcp_url_config = format!(
                "mcp_servers.synapse.url={}",
                toml_string_literal(&params.mcp_url)
            );
            format!(
                "$ErrorActionPreference = 'Stop'\n\
Set-Location -LiteralPath {working_dir}\n\
$prompt = Get-Content -Raw -LiteralPath {prompt_path}\n\
$codexArgs = @('exec','-C',{working_dir},'-s','danger-full-access','--json','-o',{final_message_path},'-c',{mcp_url_config},'-c','mcp_servers.synapse.bearer_token_env_var=\"SYNAPSE_BEARER_TOKEN\"','-')\n\
$prompt | & codex @codexArgs 1> {stdout_path} 2> {stderr_path}\n\
exit $LASTEXITCODE\n",
                working_dir = working_dir,
                prompt_path = prompt_path,
                final_message_path = final_message_path,
                mcp_url_config = ps_single_quote(&mcp_url_config),
                stdout_path = stdout_path,
                stderr_path = stderr_path,
            )
        }
        ActSpawnAgentCli::Claude => {
            let Some(debug_path) = files.debug_path.as_ref() else {
                return Err(mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "act_spawn_agent internal error: missing Claude debug path",
                ));
            };
            let Some(mcp_config_path) = files.mcp_config_path.as_ref() else {
                return Err(mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "act_spawn_agent internal error: missing Claude MCP config path",
                ));
            };
            let debug_path = ps_single_quoted_path(debug_path);
            let mcp_config_path = ps_single_quoted_path(mcp_config_path);
            format!(
                "$ErrorActionPreference = 'Stop'\n\
Set-Location -LiteralPath {working_dir}\n\
$prompt = Get-Content -Raw -LiteralPath {prompt_path}\n\
$claudeArgs = @('-p','--verbose','--output-format','stream-json','--input-format','text','--permission-mode','bypassPermissions','--mcp-config',{mcp_config_path},'--strict-mcp-config','--add-dir',{working_dir},'--debug-file',{debug_path})\n\
$prompt | & claude @claudeArgs 1> {stdout_path} 2> {stderr_path}\n\
exit $LASTEXITCODE\n",
                working_dir = working_dir,
                prompt_path = prompt_path,
                mcp_config_path = mcp_config_path,
                debug_path = debug_path,
                stdout_path = stdout_path,
                stderr_path = stderr_path,
            )
        }
    };
    Ok(script)
}

fn ps_single_quoted_path(path: &Path) -> String {
    ps_single_quote(&path.display().to_string())
}

fn ps_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn toml_string_literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

fn spawn_session_candidate_matches(
    summary: &super::session_tools::SessionSummary,
    params: &ActSpawnAgentParams,
    before_session_ids: &BTreeSet<String>,
    launched_at_unix_ms: u64,
) -> bool {
    if before_session_ids.contains(&summary.registry.session_id) {
        return false;
    }
    if summary.registry.started_at_unix_ms + 2_000 < launched_at_unix_ms {
        return false;
    }
    if !summary_matches_cli(summary, params.cli) {
        return false;
    }
    if let Some(expected) = &params.target {
        matches_target_wire(summary.active_target.as_ref(), expected)
    } else {
        summary
            .registry
            .last_action
            .as_deref()
            .is_some_and(|action| action.starts_with("tools/call:"))
    }
}

fn summary_matches_cli(
    summary: &super::session_tools::SessionSummary,
    cli: ActSpawnAgentCli,
) -> bool {
    let cli = cli.as_str();
    if summary.registry.agent_kind == cli {
        return true;
    }
    summary
        .registry
        .client_name
        .as_deref()
        .is_some_and(|name| name.to_ascii_lowercase().contains(cli))
}

fn matches_target_wire(wire: Option<&super::TargetWire>, expected: &ActSpawnAgentTarget) -> bool {
    match (wire, expected) {
        (
            Some(super::TargetWire::Window {
                window_hwnd: actual,
            }),
            ActSpawnAgentTarget::Window {
                window_hwnd: expected,
            },
        ) => actual == expected,
        (
            Some(super::TargetWire::Cdp {
                window_hwnd: actual_hwnd,
                cdp_target_id: actual_target,
            }),
            ActSpawnAgentTarget::Cdp {
                window_hwnd: expected_hwnd,
                cdp_target_id: expected_target,
            },
        ) => actual_hwnd == expected_hwnd && actual_target == expected_target,
        _ => false,
    }
}

fn tail_file_lossy(path: &Path, limit_bytes: usize) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let start = bytes.len().saturating_sub(limit_bytes);
    Some(String::from_utf8_lossy(&bytes[start..]).into_owned())
}

fn process_has_exited(pid: u32) -> bool {
    !process_exists(pid)
}

fn process_exists(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessesToUpdate, System};

    let mut system = System::new();
    let pid = Pid::from_u32(pid);
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), false);
    system.process(pid).is_some()
}

fn discover_agent_process_id(launcher_pid: u32, cli: ActSpawnAgentCli) -> Option<u32> {
    use sysinfo::{ProcessesToUpdate, System};

    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let descendants = descendant_process_ids(&system, launcher_pid);
    let cli = cli.as_str();
    descendants
        .iter()
        .copied()
        .find(|pid| {
            let Some(process) = system.process(sysinfo::Pid::from_u32(*pid)) else {
                return false;
            };
            let name = process.name().to_string_lossy().to_ascii_lowercase();
            let cmd = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase();
            name.contains(cli) || cmd.contains(cli)
        })
        .or_else(|| descendants.first().copied())
}

fn descendant_process_ids(system: &sysinfo::System, root_pid: u32) -> Vec<u32> {
    let mut descendants = Vec::new();
    let mut stack = vec![root_pid];
    while let Some(parent) = stack.pop() {
        for (pid, process) in system.processes() {
            if process.parent().map(|value| value.as_u32()) == Some(parent) {
                let child = pid.as_u32();
                descendants.push(child);
                stack.push(child);
            }
        }
    }
    descendants
}

fn agent_spawn_tool_error(
    code: &'static str,
    message: &'static str,
    data: serde_json::Value,
) -> ErrorData {
    tracing::warn!(code, "M4 agent spawn tool error: {message}");
    ErrorData::new(ErrorCode(-32099), message, Some(data))
}

fn launch_lifecycle_tool_error(message: &'static str, data: serde_json::Value) -> ErrorData {
    tracing::warn!(
        code = error_codes::TOOL_INTERNAL_ERROR,
        "M4 launch lifecycle tool error: {message}"
    );
    ErrorData::new(ErrorCode(-32099), message, Some(data))
}

async fn run_shell_with_idempotency(
    service: &SynapseService,
    params: ActRunShellParams,
    authorization: RunShellAuthorization,
    inline_await_limit_ms: u64,
    context: Option<&ShellExecutionContext>,
) -> Result<ActRunShellResponse, ErrorData> {
    let session_id = context.map(ShellExecutionContext::session_id);
    let Some(row_key) = run_shell_idempotency_row_key(&params, session_id)? else {
        return run_authorized_shell(params, &authorization, inline_await_limit_ms, context).await;
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
            return run_shell_idempotency_replay(&params, &existing, session_id);
        }
        let reservation =
            run_shell_idempotency_reservation_row(&params, &authorization, session_id)?;
        runtime
            .storage_put_kv_rows(vec![(row_key.clone(), reservation)])
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    }

    let response = run_authorized_shell(
        params.clone(),
        &authorization,
        inline_await_limit_ms,
        context,
    )
    .await?;
    let completed =
        run_shell_idempotency_completed_row(&params, &authorization, &response, session_id)?;
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
