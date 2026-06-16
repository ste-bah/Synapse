use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use futures_util::StreamExt;
use reqwest::Url;
use rmcp::{
    RoleClient, ServiceExt,
    model::{
        CallToolRequestParams, ClientCapabilities, ClientInfo, Content, Implementation, JsonObject,
        Tool,
    },
    service::{RunningService, ServiceError},
    transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const SYNAPSE_TOOL_CATALOG: &str = "synapse_tool_catalog";
const SYNAPSE_TOOL_CALL: &str = "synapse_tool";

#[derive(Clone, Debug)]
pub(crate) struct LocalAgentCli {
    pub model_name: Option<String>,
    pub task: Option<String>,
    pub task_file: Option<PathBuf>,
    pub mcp_url: String,
    pub spawn_id: Option<String>,
    pub log_dir: Option<PathBuf>,
    pub target_json: Option<String>,
    pub max_turns: u32,
    pub timeout_ms: u64,
    pub context_char_limit: usize,
    pub tool_parse_retry_limit: u32,
    pub no_stream: bool,
    pub allow_non_loopback: bool,
}

type LocalMcpClient = RunningService<RoleClient, ClientInfo>;

#[derive(Debug, Deserialize)]
struct LocalModelListResult {
    rows: Vec<LocalModelRegistryRow>,
}

#[derive(Clone, Debug, Deserialize)]
struct LocalModelRegistryRow {
    name: String,
    base_url: String,
    model_id: String,
    enabled: bool,
    allow_non_loopback: bool,
    api_key_env_var: Option<String>,
    api_shape: String,
    runtime_preset: Option<String>,
    context_length: Option<u64>,
    max_tools: Option<usize>,
    last_probe: Option<LocalModelProbe>,
}

#[derive(Clone, Debug, Deserialize)]
struct LocalModelProbe {
    healthy: bool,
    error_code: Option<String>,
    error_detail: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct Usage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

impl Usage {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            prompt_tokens: value.get("prompt_tokens").and_then(Value::as_u64)?,
            completion_tokens: value.get("completion_tokens").and_then(Value::as_u64)?,
            total_tokens: value.get("total_tokens").and_then(Value::as_u64)?,
        })
    }

    fn add(&mut self, other: &Self) {
        self.prompt_tokens = self.prompt_tokens.saturating_add(other.prompt_tokens);
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(other.completion_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
    }
}

#[derive(Clone, Debug)]
struct OpenAiToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Clone, Debug, Default)]
struct ChatCompletion {
    content: String,
    tool_calls: Vec<OpenAiToolCall>,
    finish_reason: Option<String>,
    usage: Option<Usage>,
    raw_sha256: String,
}

#[derive(Default)]
struct StreamingToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

struct Runner {
    cli: LocalAgentCli,
    spawn_id: String,
    log_dir: PathBuf,
    stdout: File,
    token: String,
    mcp: LocalMcpClient,
    mcp_session_id: String,
    event_url: Url,
    endpoint_url: Url,
    endpoint_api_key: Option<String>,
    registry: LocalModelRegistryRow,
    tool_exposure: ToolExposure,
    tools: Vec<Tool>,
    openai_tools: Vec<Value>,
    messages: Vec<Value>,
    conversation_id: String,
    total_usage: Usage,
    turn_count: u32,
    tool_call_count: u64,
    parse_error_count: u32,
    tool_call_error_count: u32,
    truncated_context_count: u32,
    completed_after_tool: bool,
    successful_workspace_puts: Vec<JsonObject>,
    http: reqwest::Client,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolExposure {
    Direct,
    Routed,
    /// Internalized model: the full tool surface lives in the model's weights,
    /// so NO tool catalog is injected into the request. The model emits tool
    /// calls from a near-empty prompt; the response is routed exactly like
    /// Direct (every real tool is callable by name).
    Internalized,
}

impl ToolExposure {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Routed => "routed",
            Self::Internalized => "internalized",
        }
    }
}

pub(crate) async fn run_from_cli(cli: LocalAgentCli) -> anyhow::Result<ExitCode> {
    let mut runner = Runner::new(cli).await?;
    let result = runner.run_loop().await;
    let exit_code = match result {
        Ok(()) => {
            runner.write_success_status()?;
            ExitCode::SUCCESS
        }
        Err(error) => {
            let detail = format!("{error:#}");
            let code = error_code_from_detail(&detail);
            runner.write_failure(code, &detail).await?;
            ExitCode::from(1)
        }
    };
    let _ = runner.mcp.close().await;
    Ok(exit_code)
}

impl Runner {
    async fn new(cli: LocalAgentCli) -> anyhow::Result<Self> {
        if cli.max_turns == 0 {
            bail!("TOOL_PARAMS_INVALID: --local-agent-max-turns must be > 0");
        }
        if cli.timeout_ms == 0 {
            bail!("TOOL_PARAMS_INVALID: --local-agent-timeout-ms must be > 0");
        }
        if cli.context_char_limit < 1024 {
            bail!("TOOL_PARAMS_INVALID: --local-agent-context-char-limit must be >= 1024");
        }
        let model_name = non_empty(
            cli.model_name.as_deref(),
            "TOOL_PARAMS_INVALID: --local-agent-model is required",
        )?;
        let task = resolve_task(cli.task.as_deref(), cli.task_file.as_ref())?;
        let spawn_id = cli
            .spawn_id
            .clone()
            .unwrap_or_else(|| format!("agent-spawn-local-{}", Uuid::now_v7().simple()));
        validate_spawn_id(&spawn_id)?;
        let log_dir = match cli.log_dir.clone() {
            Some(path) => path,
            None => local_agent_spawn_root_dir()?.join(&spawn_id),
        };
        std::fs::create_dir_all(&log_dir)
            .with_context(|| format!("create local-agent log dir {}", log_dir.display()))?;
        let stdout_path = log_dir.join("stdout.jsonl");
        let stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stdout_path)
            .with_context(|| format!("open {}", stdout_path.display()))?;
        let token = crate::http::load_token_value().context("load Synapse bearer token")?;
        let client_info = ClientInfo::new(
            ClientCapabilities::default(),
            Implementation::new("synapse-local-model-agent", env!("CARGO_PKG_VERSION")),
        );
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(cli.mcp_url.clone())
                .auth_header(token.clone()),
        );
        let mcp = client_info
            .serve(transport)
            .await
            .context("initialize Synapse MCP local-agent session")?;
        let tools = mcp
            .peer()
            .list_all_tools()
            .await
            .context("client-parity tools/list failed")?;
        if tools.is_empty() {
            bail!("MODEL_TOOLS_UNSUPPORTED: Synapse tools/list returned zero tools");
        }
        let _health = call_mcp_tool_json(mcp.peer(), "health", Map::new())
            .await
            .context("call Synapse health tool")?;
        let mcp_session_id = current_mcp_session_id(mcp.peer()).await?;
        let mut list_args = Map::new();
        list_args.insert("name".to_owned(), Value::from(model_name.to_owned()));
        list_args.insert("include_disabled".to_owned(), Value::from(true));
        list_args.insert("limit".to_owned(), Value::from(10));
        let registry_value = call_mcp_tool_json(mcp.peer(), "local_model_list", list_args)
            .await
            .context("call Synapse local_model_list tool")?;
        let registry_result: LocalModelListResult = serde_json::from_value(registry_value)
            .context("decode local_model_list structured content")?;
        let registry = registry_result
            .rows
            .into_iter()
            .find(|row| row.name == model_name)
            .with_context(|| format!("LOCAL_MODEL_NOT_FOUND: registry row {model_name:?}"))?;
        validate_registry_row(&registry)?;
        let endpoint_url = chat_completions_endpoint(&registry, cli.allow_non_loopback)?;
        let endpoint_api_key = match registry.api_key_env_var.as_deref() {
            Some(name) => Some(
                std::env::var(name)
                    .with_context(|| format!("LOCAL_MODEL_API_KEY_MISSING: env {name}"))?,
            ),
            None => None,
        };
        let event_url = agent_event_url(&cli.mcp_url, &spawn_id)?;
        let tool_exposure = resolve_tool_exposure(&registry, tools.len());
        let openai_tools = match tool_exposure {
            ToolExposure::Direct => tools.iter().map(openai_tool_from_mcp).collect::<Vec<_>>(),
            ToolExposure::Routed => routed_harness_tools(),
            // Internalized: surface is in the weights — inject ZERO tools.
            ToolExposure::Internalized => Vec::new(),
        };
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(cli.timeout_ms))
            .build()
            .context("build local-agent HTTP client")?;
        let conversation_id = format!("local-model-{}", Uuid::now_v7().simple());
        bind_requested_target(mcp.peer(), cli.target_json.as_deref()).await?;
        write_json_file(
            log_dir.join("local-model-runner.json"),
            &json!({
                "schema_version": 1,
                "spawn_id": spawn_id,
                "session_id": mcp_session_id.clone(),
                "registry_name": registry.name,
                "model": registry.model_id,
                "mcp_url": cli.mcp_url,
                "endpoint_url": endpoint_url.as_str(),
                "context_length": registry.context_length,
                "registry_max_tools": registry.max_tools,
                "runtime_preset": registry_runtime_preset(&registry),
                "tool_exposure": tool_exposure.as_str(),
                "mcp_tool_count": tools.len(),
                "openai_tool_count": openai_tools.len(),
                "started_at_unix_ms": unix_time_ms_now(),
            }),
        )?;
        let task_started_path = log_dir.join("task-started.json");
        write_json_file(
            task_started_path.clone(),
            &json!({
                "schema_version": 1,
                "spawn_id": spawn_id,
                "cli": "local-model",
                "session_id": mcp_session_id.clone(),
                "status": "started",
                "health_ok": true,
                "target_ok": true,
                "assigned_prompt_present": true,
                "task_started_path": task_started_path.display().to_string(),
                "registry_name": registry.name,
                "model": registry.model_id,
                "conversation_id": conversation_id,
                "tool_count": tools.len(),
                "openai_tool_count": openai_tools.len(),
                "tool_exposure": tool_exposure.as_str(),
                "endpoint_url": endpoint_url.as_str(),
                "started_at_unix_ms": unix_time_ms_now(),
            }),
        )?;
        std::fs::write(log_dir.join("prompt.txt"), &task)
            .with_context(|| format!("write {}", log_dir.join("prompt.txt").display()))?;
        write_json_file(
            log_dir.join("completion-status.json"),
            &json!({
                "schema_version": 1,
                "spawn_id": spawn_id,
                "status": "running",
                "state": "running",
                "started_at_unix_ms": unix_time_ms_now(),
            }),
        )?;
        let mut messages = Vec::new();
        messages.push(json!({
            "role": "system",
            "content": system_prompt(tool_exposure, &tools),
        }));
        messages.push(json!({
            "role": "user",
            "content": task,
        }));
        let mut runner = Self {
            cli,
            spawn_id,
            log_dir,
            stdout,
            token,
            mcp,
            mcp_session_id,
            event_url,
            endpoint_url,
            endpoint_api_key,
            registry,
            tool_exposure,
            tools,
            openai_tools,
            messages,
            conversation_id,
            total_usage: Usage::default(),
            turn_count: 0,
            tool_call_count: 0,
            parse_error_count: 0,
            tool_call_error_count: 0,
            truncated_context_count: 0,
            completed_after_tool: false,
            successful_workspace_puts: Vec::new(),
            http,
        };
        runner.write_line(json!({
            "type": "local.thread.started",
            "session_id": runner.mcp_session_id,
            "conversation_id": runner.conversation_id,
            "model": runner.registry.model_id,
            "registry_name": runner.registry.name,
            "tool_count": runner.tools.len(),
            "openai_tool_count": runner.openai_tools.len(),
            "tool_exposure": runner.tool_exposure.as_str(),
            "runtime_preset": registry_runtime_preset(&runner.registry),
        }))?;
        runner
            .post_event(json!({
                "event": "state_changed",
                "session_id": runner.mcp_session_id,
                "conversation_id": runner.conversation_id,
                "model": runner.registry.model_id,
                "registry_name": runner.registry.name,
                "state_to": "live",
                "reason_code": "local_agent_started",
                "tool_count": runner.tools.len(),
                "openai_tool_count": runner.openai_tools.len(),
                "tool_exposure": runner.tool_exposure.as_str(),
                "runtime_preset": registry_runtime_preset(&runner.registry),
            }))
            .await?;
        Ok(runner)
    }

    async fn run_loop(&mut self) -> anyhow::Result<()> {
        let started = Instant::now();
        let mut used_any_tool = false;
        for turn in 1..=self.cli.max_turns {
            if started.elapsed() > Duration::from_millis(self.cli.timeout_ms) {
                bail!(
                    "MODEL_ENDPOINT_UNREACHABLE: local-agent timeout exceeded before turn {turn}"
                );
            }
            self.turn_count = turn;
            self.write_line(json!({
                "type": "local.turn.started",
                "conversation_id": self.conversation_id,
                "model": self.registry.model_id,
                "turn_index": turn,
            }))?;
            self.post_event(json!({
                "event": "turn_started",
                "session_id": self.mcp_session_id,
                "conversation_id": self.conversation_id,
                "model": self.registry.model_id,
                "registry_name": self.registry.name,
                "turn_index": turn,
            }))
            .await?;
            self.drain_steering_inbox().await?;
            self.truncate_context_if_needed().await?;
            let completion = self.chat_completion().await?;
            self.write_line(json!({
                "type": "local.assistant.message",
                "conversation_id": self.conversation_id,
                "model": self.registry.model_id,
                "turn_index": turn,
                "content": completion.content,
                "finish_reason": completion.finish_reason,
                "raw_response_sha256": completion.raw_sha256,
            }))?;
            self.messages.push(assistant_message(&completion));
            if let Some(usage) = completion.usage.clone() {
                self.total_usage.add(&usage);
                self.write_line(json!({
                    "type": "local.turn.finished",
                    "conversation_id": self.conversation_id,
                    "model": self.registry.model_id,
                    "turn_index": turn,
                    "finish_reason": completion.finish_reason,
                    "usage": usage,
                }))?;
                self.post_event(json!({
                    "event": "turn_finished",
                    "session_id": self.mcp_session_id,
                    "conversation_id": self.conversation_id,
                    "model": self.registry.model_id,
                    "registry_name": self.registry.name,
                    "turn_index": turn,
                    "finish_reason": completion.finish_reason,
                    "usage": usage,
                }))
                .await?;
            }
            if completion.tool_calls.is_empty() {
                // A plain assistant message with no tool call is a legitimate
                // completion: the model answered directly. This is valid on the
                // first turn (a prompt may not need tools) and after tool use
                // alike. Models that are genuinely incapable of tool-calling are
                // rejected at registration-probe time (MODEL_TOOLS_UNSUPPORTED),
                // not here — bailing on a correct direct answer was a false
                // failure that killed otherwise-successful agents. Only a turn
                // that yields neither a tool call nor any content is degenerate.
                let final_message = completion.content.trim();
                if final_message.is_empty() {
                    bail!(
                        "MODEL_EMPTY_COMPLETION: model returned neither a tool call nor message content on turn {turn} (finish_reason={:?}); used_any_tool={used_any_tool}",
                        completion.finish_reason
                    );
                }
                std::fs::write(self.log_dir.join("final-message.txt"), &completion.content)
                    .with_context(|| {
                        format!("write {}", self.log_dir.join("final-message.txt").display())
                    })?;
                self.post_event(json!({
                    "event": "exited",
                    "session_id": self.mcp_session_id,
                    "conversation_id": self.conversation_id,
                    "model": self.registry.model_id,
                    "registry_name": self.registry.name,
                    "end_state": "success",
                    "reason_code": "local_agent_completed",
                    "used_any_tool": used_any_tool,
                    "answered_without_tool_calls": !used_any_tool,
                }))
                .await?;
                return Ok(());
            }
            used_any_tool = true;
            for call in completion.tool_calls {
                self.execute_tool_call(call).await?;
                if self.completed_after_tool {
                    return Ok(());
                }
                self.drain_steering_inbox().await?;
            }
        }
        bail!(
            "LOCAL_AGENT_TURN_LIMIT: local model did not finish within {} turns",
            self.cli.max_turns
        )
    }

    async fn execute_tool_call(&mut self, call: OpenAiToolCall) -> anyhow::Result<()> {
        self.tool_call_count = self.tool_call_count.saturating_add(1);
        let routed = call.name == SYNAPSE_TOOL_CALL;
        let catalog = call.name == SYNAPSE_TOOL_CATALOG;
        self.write_line(json!({
            "type": "local.tool_call.started",
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "turn_index": self.turn_count,
            "tool_name": call.name,
            "tool_call_id": call.id,
            "arguments": call.arguments,
            "tool_exposure": self.tool_exposure.as_str(),
        }))?;
        self.post_event(json!({
            "event": "tool_call_started",
            "session_id": self.mcp_session_id,
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "registry_name": self.registry.name,
            "turn_index": self.turn_count,
            "tool_name": call.name,
            "tool_call_id": call.id,
            "tool_arguments": string_json_or_value(&call.arguments),
            "tool_exposure": self.tool_exposure.as_str(),
        }))
        .await?;

        if catalog {
            let result_value = match self.tool_catalog_value(&call.arguments) {
                Ok(value) => value,
                Err(error) => {
                    self.record_tool_parse_error(&call, error).await?;
                    return Ok(());
                }
            };
            let result_text = bounded_result_text(&result_value);
            self.messages.push(json!({
                "role": "tool",
                "tool_call_id": call.id,
                "content": result_text,
            }));
            self.write_line(json!({
                "type": "local.tool_call.finished",
                "conversation_id": self.conversation_id,
                "model": self.registry.model_id,
                "turn_index": self.turn_count,
                "tool_name": call.name,
                "tool_call_id": call.id,
                "status": "ok",
                "result": result_value,
                "tool_exposure": self.tool_exposure.as_str(),
            }))?;
            self.post_event(json!({
                "event": "tool_call_finished",
                "session_id": self.mcp_session_id,
                "conversation_id": self.conversation_id,
                "model": self.registry.model_id,
                "registry_name": self.registry.name,
                "turn_index": self.turn_count,
                "tool_name": call.name,
                "tool_call_id": call.id,
                "tool_response": result_value,
                "error_code": "",
                "tool_exposure": self.tool_exposure.as_str(),
            }))
            .await?;
            return Ok(());
        }

        let (tool_name, args) = if routed {
            match parse_routed_tool_call(&call.arguments) {
                Ok(parsed) => parsed,
                Err(error) => {
                    self.record_tool_parse_error(&call, error).await?;
                    return Ok(());
                }
            }
        } else {
            match parse_tool_arguments(&call.arguments) {
                Ok(args) => (call.name.clone(), args),
                Err(error) => {
                    self.record_tool_parse_error(&call, error).await?;
                    return Ok(());
                }
            }
        };

        if self.is_duplicate_successful_workspace_put(&tool_name, &args) {
            self.record_duplicate_workspace_put_completion(&call, &tool_name, routed, &args)
                .await?;
            return Ok(());
        }

        if let Some(reason) = model_tool_call_pre_gate_rejection(
            &tool_name,
            &args,
            self.synapse_tool_exists(&tool_name),
        ) {
            self.record_model_tool_call_invalid(&call, &tool_name, routed, &reason)
                .await?;
            bail!("MODEL_TOOL_CALL_INVALID: {tool_name}: {reason}");
        }

        // #1028: gate hazardous tool calls through the shared approval queue
        // before dispatch. Safe (read-only/low-consequence) calls pass through
        // instantly; risky calls block on the in-daemon approval_gate and may be
        // denied (no dispatch) or approved-with-edits (dispatch the operator's
        // edited args). Fail-closed: a gate that cannot answer denies the action.
        let args = match self.gate_tool_call(&tool_name, args, &call.id).await? {
            ToolGate::Allow(effective_args) => effective_args,
            ToolGate::Deny(reason) => {
                self.record_tool_gate_denied(&call, &tool_name, routed, &reason)
                    .await?;
                return Ok(());
            }
        };

        let result = match self
            .mcp
            .peer()
            .call_tool(CallToolRequestParams::new(tool_name.clone()).with_arguments(args.clone()))
            .await
        {
            Ok(result) => result,
            Err(error) => {
                // A single failed tool call MUST NOT crash the agent. The
                // canonical tool-calling reliability pattern is to feed the
                // error back to the model as a tool result so it can
                // self-correct (retry with fixed arguments, pick a different
                // tool, or explain the limitation). Only a genuinely dead
                // transport is terminal. Crashing here previously killed agents
                // on the first recoverable error (e.g. a missing required
                // parameter), tearing down any process the agent had launched.
                let fatal = tool_call_error_is_terminal(&error);
                let detail = format!("SYNAPSE_TOOL_CALL_FAILED: {tool_name}: {error}");
                self.tool_call_error_count = self.tool_call_error_count.saturating_add(1);
                // Structured, actionable feedback for the model (not a bare
                // exception string): names the tool, the failure, and the next
                // step the model should take.
                let model_feedback = json!({
                    "error": "SYNAPSE_TOOL_CALL_FAILED",
                    "tool": tool_name,
                    "message": error.to_string(),
                    "recoverable": !fatal,
                    "suggestion": tool_failure_suggestion(fatal, self.tool_exposure),
                });
                let result_value = json!({ "error": detail });
                self.messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": model_feedback.to_string(),
                }));
                self.write_line(json!({
                    "type": "local.tool_call.finished",
                    "conversation_id": self.conversation_id,
                    "model": self.registry.model_id,
                    "turn_index": self.turn_count,
                    "tool_name": call.name,
                    "routed_tool_name": if routed { Some(tool_name.as_str()) } else { None },
                    "tool_call_id": call.id,
                    "status": "error",
                    "error_code": "SYNAPSE_TOOL_CALL_FAILED",
                    "terminal": fatal,
                    "result": result_value,
                    "tool_exposure": self.tool_exposure.as_str(),
                }))?;
                self.post_event(json!({
                    "event": "tool_call_finished",
                    "session_id": self.mcp_session_id,
                    "conversation_id": self.conversation_id,
                    "model": self.registry.model_id,
                    "registry_name": self.registry.name,
                    "turn_index": self.turn_count,
                    "tool_name": call.name,
                    "routed_tool_name": if routed { Some(tool_name.as_str()) } else { None },
                    "tool_call_id": call.id,
                    "tool_response": result_value,
                    "error_code": "SYNAPSE_TOOL_CALL_FAILED",
                    "tool_exposure": self.tool_exposure.as_str(),
                }))
                .await?;
                if fatal {
                    // Transport is gone: every further call would fail the same
                    // way. Fail loudly with the exact cause.
                    bail!("{detail}");
                }
                // Recoverable: the model now has the error in context and the
                // loop continues. The overall run stays bounded by max_turns.
                return Ok(());
            }
        };
        let is_error = result.is_error.unwrap_or(false);
        let mut result_value = tool_result_value(&result);
        self.fail_if_tool_result_contains_control_shutdown(&tool_name, &result_value)
            .await?;
        if !is_error && tool_name == "workspace_put" {
            let readback = readback_workspace_put(self.mcp.peer(), &args)
                .await
                .context("workspace_put post-write readback")?;
            result_value = attach_workspace_put_readback(result_value, readback);
            self.successful_workspace_puts.push(args.clone());
        }
        let result_text = bounded_result_text(&result_value);
        self.messages.push(json!({
            "role": "tool",
            "tool_call_id": call.id,
            "content": result_text,
        }));
        self.write_line(json!({
            "type": "local.tool_call.finished",
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "turn_index": self.turn_count,
            "tool_name": call.name,
            "routed_tool_name": if routed { Some(tool_name.as_str()) } else { None },
            "tool_call_id": call.id,
            "status": if is_error { "error" } else { "ok" },
            "result": result_value,
            "tool_exposure": self.tool_exposure.as_str(),
        }))?;
        self.post_event(json!({
            "event": "tool_call_finished",
            "session_id": self.mcp_session_id,
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "registry_name": self.registry.name,
            "turn_index": self.turn_count,
            "tool_name": call.name,
            "routed_tool_name": if routed { Some(tool_name.as_str()) } else { None },
            "tool_call_id": call.id,
            "tool_response": result_value,
            "error_code": if is_error { "SYNAPSE_TOOL_ERROR" } else { "" },
            "tool_exposure": self.tool_exposure.as_str(),
        }))
        .await?;
        Ok(())
    }

    fn is_duplicate_successful_workspace_put(&self, tool_name: &str, args: &JsonObject) -> bool {
        tool_name == "workspace_put"
            && self
                .successful_workspace_puts
                .iter()
                .any(|successful| workspace_put_args_match(successful, args))
    }

    fn synapse_tool_exists(&self, tool_name: &str) -> bool {
        self.tools
            .iter()
            .any(|tool| tool.name.as_ref() == tool_name)
    }

    async fn record_duplicate_workspace_put_completion(
        &mut self,
        call: &OpenAiToolCall,
        tool_name: &str,
        routed: bool,
        args: &JsonObject,
    ) -> anyhow::Result<()> {
        let result_value = json!({
            "ok": true,
            "duplicate_suppressed": true,
            "reason": "workspace_put already succeeded and read back in this run",
            "arguments": args,
        });
        let result_text = bounded_result_text(&result_value);
        self.messages.push(json!({
            "role": "tool",
            "tool_call_id": call.id,
            "content": result_text,
        }));
        self.write_line(json!({
            "type": "local.tool_call.finished",
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "turn_index": self.turn_count,
            "tool_name": call.name,
            "routed_tool_name": if routed { Some(tool_name) } else { None },
            "tool_call_id": call.id,
            "status": "ok",
            "result": result_value,
            "tool_exposure": self.tool_exposure.as_str(),
        }))?;
        self.post_event(json!({
            "event": "tool_call_finished",
            "session_id": self.mcp_session_id,
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "registry_name": self.registry.name,
            "turn_index": self.turn_count,
            "tool_name": call.name,
            "routed_tool_name": if routed { Some(tool_name) } else { None },
            "tool_call_id": call.id,
            "tool_response": result_value,
            "error_code": "",
            "tool_exposure": self.tool_exposure.as_str(),
        }))
        .await?;
        let final_message =
            "workspace_put already succeeded and was read back; duplicate suppressed.";
        std::fs::write(self.log_dir.join("final-message.txt"), final_message).with_context(
            || format!("write {}", self.log_dir.join("final-message.txt").display()),
        )?;
        self.write_line(json!({
            "type": "local.agent.completed",
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "reason_code": "duplicate_workspace_put_suppressed",
            "final_message": final_message,
        }))?;
        self.post_event(json!({
            "event": "exited",
            "session_id": self.mcp_session_id,
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "registry_name": self.registry.name,
            "end_state": "success",
            "reason_code": "local_agent_completed",
            "used_any_tool": true,
            "answered_without_tool_calls": false,
            "completion_reason": "duplicate_workspace_put_suppressed",
        }))
        .await?;
        self.completed_after_tool = true;
        Ok(())
    }

    /// #1028: route a hazardous tool call through the shared approval queue
    /// before dispatch. Read-only / low-consequence calls (per the SAME
    /// `permission_policy` the daemon's Claude gate uses) skip the gate entirely;
    /// risky calls block on the in-daemon `approval_gate`, which creates a Pending
    /// `AgentPermission` row attributed to this spawn and returns the operator's
    /// verdict (allow / approve-with-edits / deny). Fail-closed: a gate that
    /// cannot answer denies the action — the risky tool is never dispatched on a
    /// gate failure.
    async fn gate_tool_call(
        &mut self,
        tool_name: &str,
        args: JsonObject,
        tool_use_id: &str,
    ) -> anyhow::Result<ToolGate> {
        let label = gate_tool_label(tool_name);
        if !crate::server::permission_policy::classify(&label, &Value::Object(args.clone()))
            .is_gate()
        {
            return Ok(ToolGate::Allow(args));
        }

        // The agent is now waiting on a human — surface it to the fleet inbox /
        // escalation pipeline via the state machine.
        self.post_event(json!({
            "event": "state_changed",
            "session_id": self.mcp_session_id,
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "registry_name": self.registry.name,
            "state_to": "awaiting_approval",
            "reason_code": "local_tool_gate",
            "tool_name": tool_name,
            "tool_call_id": tool_use_id,
        }))
        .await?;
        self.write_line(json!({
            "type": "local.tool_call.gate_blocking",
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "turn_index": self.turn_count,
            "tool_name": tool_name,
            "tool_call_id": tool_use_id,
        }))?;

        let mut gate_args = Map::new();
        gate_args.insert("tool_name".to_owned(), Value::String(label));
        gate_args.insert("input".to_owned(), Value::Object(args.clone()));
        gate_args.insert(
            "tool_use_id".to_owned(),
            Value::String(tool_use_id.to_owned()),
        );
        gate_args.insert("spawn_id".to_owned(), Value::String(self.spawn_id.clone()));

        let verdict = match self
            .mcp
            .peer()
            .call_tool(CallToolRequestParams::new("approval_gate").with_arguments(gate_args))
            .await
        {
            Ok(result) => parse_gate_verdict(&tool_result_value(&result), &args)?,
            Err(error) => {
                if tool_call_error_is_terminal(&error) {
                    bail!("APPROVAL_GATE_TRANSPORT_DEAD: approval_gate transport lost: {error}");
                }
                tracing::error!(
                    code = "LOCAL_AGENT_APPROVAL_GATE_FAILED",
                    tool = tool_name,
                    spawn_id = %self.spawn_id,
                    detail = %error,
                    "approval_gate call failed; denying the action fail-closed"
                );
                ToolGate::Deny(format!(
                    "approval gate unavailable ({error}); action blocked, not executed"
                ))
            }
        };

        let reason_code = match &verdict {
            ToolGate::Allow(_) => "local_tool_gate_approved",
            ToolGate::Deny(_) => "local_tool_gate_denied",
        };
        self.post_event(json!({
            "event": "state_changed",
            "session_id": self.mcp_session_id,
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "registry_name": self.registry.name,
            "state_to": "live",
            "reason_code": reason_code,
            "tool_name": tool_name,
            "tool_call_id": tool_use_id,
        }))
        .await?;
        Ok(verdict)
    }

    async fn record_model_tool_call_invalid(
        &mut self,
        call: &OpenAiToolCall,
        tool_name: &str,
        routed: bool,
        reason: &str,
    ) -> anyhow::Result<()> {
        self.tool_call_error_count = self.tool_call_error_count.saturating_add(1);
        let detail = format!("MODEL_TOOL_CALL_INVALID: {tool_name}: {reason}");
        let result_value = json!({ "error": detail });
        self.messages.push(json!({
            "role": "tool",
            "tool_call_id": call.id,
            "content": json!({
                "error": "MODEL_TOOL_CALL_INVALID",
                "tool": tool_name,
                "message": reason,
                "recoverable": false,
                "suggestion": "Stop. Do not ask approval for malformed or runner-control tool calls.",
            }).to_string(),
        }));
        self.write_line(json!({
            "type": "local.tool_call.finished",
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "turn_index": self.turn_count,
            "tool_name": call.name,
            "routed_tool_name": if routed { Some(tool_name) } else { None },
            "tool_call_id": call.id,
            "status": "error",
            "error_code": "MODEL_TOOL_CALL_INVALID",
            "terminal": true,
            "result": result_value,
            "tool_exposure": self.tool_exposure.as_str(),
        }))?;
        self.post_event(json!({
            "event": "tool_call_finished",
            "session_id": self.mcp_session_id,
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "registry_name": self.registry.name,
            "turn_index": self.turn_count,
            "tool_name": call.name,
            "routed_tool_name": if routed { Some(tool_name) } else { None },
            "tool_call_id": call.id,
            "tool_response": result_value,
            "error_code": "MODEL_TOOL_CALL_INVALID",
            "tool_exposure": self.tool_exposure.as_str(),
        }))
        .await?;
        Ok(())
    }

    /// Feed an operator denial back to the model as the tool result (so it can
    /// pick another approach), and journal the gate-denied transition. The risky
    /// tool is NOT dispatched.
    async fn record_tool_gate_denied(
        &mut self,
        call: &OpenAiToolCall,
        tool_name: &str,
        routed: bool,
        reason: &str,
    ) -> anyhow::Result<()> {
        let feedback = json!({
            "error": "APPROVAL_DENIED",
            "tool": tool_name,
            "message": reason,
            "recoverable": true,
            "suggestion": "The operator denied this action. Do not retry it; choose a different approach or stop and explain why you cannot proceed.",
        });
        let result_value = json!({ "error": format!("APPROVAL_DENIED: {tool_name}: {reason}") });
        self.messages.push(json!({
            "role": "tool",
            "tool_call_id": call.id,
            "content": feedback.to_string(),
        }));
        self.write_line(json!({
            "type": "local.tool_call.finished",
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "turn_index": self.turn_count,
            "tool_name": call.name,
            "routed_tool_name": if routed { Some(tool_name) } else { None },
            "tool_call_id": call.id,
            "status": "denied",
            "error_code": "APPROVAL_DENIED",
            "result": result_value,
            "tool_exposure": self.tool_exposure.as_str(),
        }))?;
        self.post_event(json!({
            "event": "tool_call_finished",
            "session_id": self.mcp_session_id,
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "registry_name": self.registry.name,
            "turn_index": self.turn_count,
            "tool_name": call.name,
            "routed_tool_name": if routed { Some(tool_name) } else { None },
            "tool_call_id": call.id,
            "tool_response": result_value,
            "error_code": "APPROVAL_DENIED",
            "tool_exposure": self.tool_exposure.as_str(),
        }))
        .await?;
        Ok(())
    }

    async fn record_tool_parse_error(
        &mut self,
        call: &OpenAiToolCall,
        error: anyhow::Error,
    ) -> anyhow::Result<()> {
        self.parse_error_count = self.parse_error_count.saturating_add(1);
        let detail = format!("TOOL_CALL_ARGUMENTS_NOT_JSON: {error}");
        let result_value = json!({ "error": detail });
        self.write_line(json!({
            "type": "local.tool_parse_error",
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "turn_index": self.turn_count,
            "tool_name": call.name,
            "tool_call_id": call.id,
            "error_code": "MODEL_TOOL_ARGUMENTS_INVALID",
            "error_detail": detail,
            "tool_exposure": self.tool_exposure.as_str(),
        }))?;
        self.messages.push(json!({
            "role": "tool",
            "tool_call_id": call.id,
            "content": detail,
        }));
        self.post_event(json!({
            "event": "tool_call_finished",
            "session_id": self.mcp_session_id,
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "registry_name": self.registry.name,
            "turn_index": self.turn_count,
            "tool_name": call.name,
            "tool_call_id": call.id,
            "tool_response": result_value,
            "error_code": "MODEL_TOOL_ARGUMENTS_INVALID",
            "tool_exposure": self.tool_exposure.as_str(),
        }))
        .await?;
        if self.parse_error_count > self.cli.tool_parse_retry_limit {
            bail!(
                "MODEL_TOOLS_UNSUPPORTED: malformed tool-call arguments exceeded retry limit {}",
                self.cli.tool_parse_retry_limit
            );
        }
        Ok(())
    }

    fn tool_catalog_value(&self, raw_arguments: &str) -> anyhow::Result<Value> {
        let args = parse_tool_arguments(raw_arguments)?;
        let name_filter = args
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(50)
            .clamp(1, 200);
        let mut rows = Vec::new();
        for tool in &self.tools {
            let tool_name = tool.name.as_ref();
            let description = tool
                .description
                .as_ref()
                .map(|desc| desc.as_ref())
                .unwrap_or("");
            let matches_name = name_filter.is_none_or(|name| tool_name == name);
            let matches_query = query.as_ref().is_none_or(|query| {
                tool_name.to_ascii_lowercase().contains(query)
                    || description.to_ascii_lowercase().contains(query)
            });
            if matches_name && matches_query {
                rows.push(json!({
                    "name": tool_name,
                    "description": description,
                    "input_schema": Value::Object((*tool.input_schema).clone()),
                }));
                if rows.len() >= limit {
                    break;
                }
            }
        }
        Ok(json!({
            "source_of_truth": "MCP tools/list loaded by local-agent client",
            "tool_count": self.tools.len(),
            "returned_count": rows.len(),
            "limit": limit,
            "tool_exposure": self.tool_exposure.as_str(),
            "call_tool": SYNAPSE_TOOL_CALL,
            "tools": rows,
        }))
    }

    async fn fail_if_tool_result_contains_control_shutdown(
        &mut self,
        tool_name: &str,
        result_value: &Value,
    ) -> anyhow::Result<()> {
        let Some(message) = shutdown_message_from_tool_result(tool_name, result_value) else {
            return Ok(());
        };
        let steering_text = steering_payload_text(&message.payload);
        self.write_line(json!({
            "type": "local.steering.received",
            "session_id": self.mcp_session_id,
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "turn_index": self.turn_count,
            "message_id": message.message_id,
            "kind": message.kind,
            "payload_summary": bounded_text(&steering_text, 2_000),
            "source_tool": tool_name,
            "delivery_path": "tool_result_control_filter",
        }))?;
        self.post_event(json!({
            "event": "state_changed",
            "session_id": self.mcp_session_id,
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "registry_name": self.registry.name,
            "state_to": "live",
            "reason_code": "local_steering_received",
            "message_id": message.message_id,
            "message_kind": message.kind,
            "source_tool": tool_name,
        }))
        .await?;
        bail!(
            "LOCAL_AGENT_INTERRUPTED: control message {} kind={} requested shutdown",
            message.message_id,
            message.kind
        )
    }

    async fn drain_steering_inbox(&mut self) -> anyhow::Result<()> {
        let mut args = Map::new();
        args.insert("drain".to_owned(), Value::Bool(true));
        args.insert("max_messages".to_owned(), Value::from(16));
        let inbox = call_mcp_tool_json(self.mcp.peer(), "agent_inbox", args)
            .await
            .context("drain local-agent steering inbox")?;
        let messages = inbox
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if messages.is_empty() {
            return Ok(());
        }
        for message in messages {
            let message_id = message
                .get("message_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            let kind = message
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("message")
                .trim()
                .to_owned();
            let payload = message.get("payload").cloned().unwrap_or(Value::Null);
            let steering_text = steering_payload_text(&payload);
            let content = format!(
                "Synapse control message {message_id} kind={kind}:\n{}",
                bounded_text(&steering_text, 8_000)
            );
            self.write_line(json!({
                "type": "local.steering.received",
                "session_id": self.mcp_session_id,
                "conversation_id": self.conversation_id,
                "model": self.registry.model_id,
                "turn_index": self.turn_count,
                "message_id": message_id,
                "kind": kind,
                "payload_summary": bounded_text(&steering_text, 2_000),
            }))?;
            self.post_event(json!({
                "event": "state_changed",
                "session_id": self.mcp_session_id,
                "conversation_id": self.conversation_id,
                "model": self.registry.model_id,
                "registry_name": self.registry.name,
                "state_to": "live",
                "reason_code": "local_steering_received",
                "message_id": message_id,
                "message_kind": kind,
            }))
            .await?;
            if steering_requests_shutdown(&kind, &payload) {
                bail!(
                    "LOCAL_AGENT_INTERRUPTED: control message {message_id} kind={kind} requested shutdown"
                );
            }
            self.messages.push(json!({
                "role": "user",
                "content": content,
            }));
        }
        Ok(())
    }

    async fn chat_completion(&self) -> anyhow::Result<ChatCompletion> {
        let stream = should_stream(self.cli.no_stream, self.tool_exposure);
        let mut body = json!({
            "model": self.registry.model_id,
            "messages": self.messages,
            "tools": self.openai_tools,
            "tool_choice": "auto",
            "temperature": 0,
            "stream": stream,
        });
        apply_runtime_preset(&self.registry, &mut body);
        if stream {
            body["stream_options"] = json!({"include_usage": true});
        }
        let mut request = self.http.post(self.endpoint_url.clone()).json(&body);
        if let Some(api_key) = &self.endpoint_api_key {
            request = request.bearer_auth(api_key);
        }
        let response = request
            .send()
            .await
            .map_err(|error| anyhow::anyhow!("MODEL_ENDPOINT_UNREACHABLE: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("MODEL_ENDPOINT_UNREACHABLE: endpoint returned HTTP {status}: {body}");
        }
        if !stream {
            let text = response.text().await.context("read non-stream response")?;
            parse_non_stream_response(&text)
        } else {
            parse_streaming_response(response).await
        }
    }

    async fn truncate_context_if_needed(&mut self) -> anyhow::Result<()> {
        let current = serde_json::to_string(&self.messages)
            .context("serialize local-agent context for length check")?;
        if current.chars().count() <= self.cli.context_char_limit {
            return Ok(());
        }
        let before_chars = current.chars().count();
        while self.messages.len() > 2 {
            let current = serde_json::to_string(&self.messages)?;
            if current.chars().count() <= self.cli.context_char_limit {
                break;
            }
            self.messages.remove(2);
        }
        let after_chars = serde_json::to_string(&self.messages)?.chars().count();
        self.truncated_context_count = self.truncated_context_count.saturating_add(1);
        self.write_line(json!({
            "type": "local.context.truncated",
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "turn_index": self.turn_count,
            "before_chars": before_chars,
            "after_chars": after_chars,
            "limit_chars": self.cli.context_char_limit,
        }))?;
        self.post_event(json!({
            "event": "state_changed",
            "session_id": self.mcp_session_id,
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "registry_name": self.registry.name,
            "state_to": "live",
            "reason_code": "local_context_truncated",
            "before_chars": before_chars,
            "after_chars": after_chars,
            "limit_chars": self.cli.context_char_limit,
        }))
        .await?;
        if after_chars > self.cli.context_char_limit {
            bail!(
                "LOCAL_AGENT_CONTEXT_OVERFLOW: context is {after_chars} chars after truncation, limit {}",
                self.cli.context_char_limit
            );
        }
        Ok(())
    }

    async fn post_event(&self, body: Value) -> anyhow::Result<()> {
        let response = self
            .http
            .post(self.event_url.clone())
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("POST local model event ingress")?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            bail!("AGENT_EVENT_INGRESS_WRITE_FAILED: HTTP {status}: {text}");
        }
        Ok(())
    }

    fn write_line(&mut self, value: Value) -> anyhow::Result<()> {
        serde_json::to_writer(&mut self.stdout, &value).context("write local-agent stdout JSON")?;
        self.stdout
            .write_all(b"\n")
            .context("write local-agent stdout newline")?;
        self.stdout.flush().context("flush local-agent stdout")?;
        Ok(())
    }

    fn write_success_status(&self) -> anyhow::Result<()> {
        write_json_file(
            self.log_dir.join("completion-status.json"),
            &json!({
                "schema_version": 1,
                "spawn_id": self.spawn_id,
                "status": "ok",
                "state": "complete",
                "exit_code": 0,
                "turn_count": self.turn_count,
                "tool_call_count": self.tool_call_count,
                "parse_error_count": self.parse_error_count,
                "tool_call_error_count": self.tool_call_error_count,
                "truncated_context_count": self.truncated_context_count,
                "usage": self.total_usage,
                "completed_at_unix_ms": unix_time_ms_now(),
            }),
        )
    }

    async fn write_failure(&mut self, code: &str, detail: &str) -> anyhow::Result<()> {
        let _ = self.write_line(json!({
            "type": "local.error",
            "conversation_id": self.conversation_id,
            "model": self.registry.model_id,
            "turn_index": self.turn_count,
            "error_code": code,
            "error_detail": detail,
        }));
        let _ = self
            .post_event(json!({
                "event": "exited",
                "session_id": self.mcp_session_id,
                "conversation_id": self.conversation_id,
                "model": self.registry.model_id,
                "registry_name": self.registry.name,
                "end_state": "error",
                "reason_code": code,
                "error_code": code,
            }))
            .await;
        write_json_file(
            self.log_dir.join("completion-status.json"),
            &json!({
                "schema_version": 1,
                "spawn_id": self.spawn_id,
                "status": "failed",
                "state": "dead",
                "exit_code": 1,
                "error_code": code,
                "error_message": detail,
                "turn_count": self.turn_count,
                "tool_call_count": self.tool_call_count,
                "parse_error_count": self.parse_error_count,
                "tool_call_error_count": self.tool_call_error_count,
                "truncated_context_count": self.truncated_context_count,
                "usage": self.total_usage,
                "completed_at_unix_ms": unix_time_ms_now(),
            }),
        )
    }
}

async fn current_mcp_session_id(
    peer: &rmcp::service::Peer<rmcp::service::RoleClient>,
) -> anyhow::Result<String> {
    let mut args = Map::new();
    args.insert("drain".to_owned(), Value::Bool(false));
    args.insert("max_messages".to_owned(), Value::from(1));
    let inbox = call_mcp_tool_json(peer, "agent_inbox", args)
        .await
        .context("read local-agent MCP session id through agent_inbox")?;
    let session_id = inbox
        .get("this_session_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "HTTP_SESSION_INVALID: agent_inbox did not return this_session_id for local-agent session"
            )
        })?;
    Ok(session_id.to_owned())
}

async fn bind_requested_target(
    peer: &rmcp::service::Peer<rmcp::service::RoleClient>,
    target_json: Option<&str>,
) -> anyhow::Result<()> {
    let Some(raw_target) = target_json.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };
    let target: Value = serde_json::from_str(raw_target)
        .with_context(|| "TOOL_PARAMS_INVALID: --local-agent-target-json must be JSON")?;
    let mut args = Map::new();
    args.insert("target".to_owned(), target.clone());
    call_mcp_tool_json(peer, "set_target", args)
        .await
        .context("bind local-agent target through set_target")?;
    let readback = call_mcp_tool_json(peer, "get_target", Map::new())
        .await
        .context("read local-agent target through get_target")?;
    let current = readback.get("current").cloned().unwrap_or(Value::Null);
    if current != target {
        bail!("TARGET_BIND_READBACK_MISMATCH: expected {target}, got {current}");
    }
    Ok(())
}

async fn call_mcp_tool_json(
    peer: &rmcp::service::Peer<rmcp::service::RoleClient>,
    name: &str,
    arguments: JsonObject,
) -> anyhow::Result<Value> {
    let result = peer
        .call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments))
        .await
        .with_context(|| format!("MCP tool {name} call failed"))?;
    if result.is_error.unwrap_or(false) {
        bail!(
            "MCP tool {name} returned isError=true: {}",
            tool_result_value(&result)
        );
    }
    Ok(tool_result_value(&result))
}

#[derive(Clone, Debug, PartialEq)]
struct WorkspacePutReadbackPlan {
    arguments: JsonObject,
    expected_value: Value,
}

fn model_tool_call_pre_gate_rejection(
    tool_name: &str,
    args: &JsonObject,
    tool_present: bool,
) -> Option<String> {
    if !tool_present {
        return Some(format!(
            "{tool_name} is not present in Synapse tools/list; local models must emit a real Synapse tool name"
        ));
    }
    match tool_name {
        "agent_send" | "approval_decide" | "approval_gate" => Some(format!(
            "{tool_name} is runner/operator-control; local models must not call it"
        )),
        "workspace_put" => {
            if !args.contains_key("value") && !args.contains_key("artifact") {
                return Some(
                    "workspace_put requires at least one of value or artifact before approval"
                        .to_owned(),
                );
            }
            None
        }
        _ => None,
    }
}

async fn readback_workspace_put(
    peer: &rmcp::service::Peer<rmcp::service::RoleClient>,
    put_args: &JsonObject,
) -> anyhow::Result<Value> {
    let plan = workspace_put_readback_plan(put_args)?;
    let readback = call_mcp_tool_json(peer, "workspace_get", plan.arguments.clone()).await?;
    workspace_put_readback_record(&plan, &readback)
}

fn workspace_put_readback_plan(put_args: &JsonObject) -> anyhow::Result<WorkspacePutReadbackPlan> {
    let key = put_args
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("WORKSPACE_PUT_READBACK_ARGS_INVALID: missing key"))?;
    let expected_value = put_args
        .get("value")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("WORKSPACE_PUT_READBACK_ARGS_INVALID: missing value"))?;
    let mut arguments = Map::new();
    arguments.insert("key".to_owned(), Value::String(key.to_owned()));
    if let Some(run_id) = put_args.get("run_id").and_then(Value::as_str) {
        arguments.insert("run_id".to_owned(), Value::String(run_id.to_owned()));
    }
    Ok(WorkspacePutReadbackPlan {
        arguments,
        expected_value,
    })
}

fn workspace_put_args_match(left: &JsonObject, right: &JsonObject) -> bool {
    left == right
}

fn workspace_put_readback_record(
    plan: &WorkspacePutReadbackPlan,
    readback: &Value,
) -> anyhow::Result<Value> {
    let actual_value = readback
        .get("entry")
        .and_then(|entry| entry.get("value"))
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!("WORKSPACE_PUT_READBACK_INVALID: workspace_get missing entry.value")
        })?;
    if actual_value != plan.expected_value {
        bail!(
            "WORKSPACE_PUT_READBACK_MISMATCH: expected {}, got {}",
            plan.expected_value,
            actual_value
        );
    }
    Ok(json!({
        "tool": "workspace_get",
        "arguments": plan.arguments.clone(),
        "expected_value": plan.expected_value.clone(),
        "actual_value": actual_value,
        "matched": true,
        "storage_readback": readback.get("storage_readback").cloned().unwrap_or(Value::Null),
    }))
}

fn attach_workspace_put_readback(mut result_value: Value, readback: Value) -> Value {
    match &mut result_value {
        Value::Object(map) => {
            map.insert("post_write_readback".to_owned(), readback);
            result_value
        }
        _ => json!({
            "tool_result": result_value,
            "post_write_readback": readback,
        }),
    }
}

fn tool_result_value(result: &rmcp::model::CallToolResult) -> Value {
    if let Some(value) = &result.structured_content {
        return value.clone();
    }
    let text = result
        .content
        .iter()
        .filter_map(content_text)
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    }
}

fn content_text(content: &Content) -> Option<String> {
    content.as_text().map(|text| text.text.clone())
}

fn openai_tool_from_mcp(tool: &Tool) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name.as_ref(),
            "description": tool.description.as_ref().map(|desc| desc.as_ref()).unwrap_or("Synapse MCP tool"),
            "parameters": Value::Object((*tool.input_schema).clone()),
        }
    })
}

fn routed_harness_tools() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": SYNAPSE_TOOL_CATALOG,
                "description": "Read the live Synapse MCP tool catalog loaded by this agent. Use name for an exact tool or query to search names/descriptions.",
                "parameters": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Exact Synapse MCP tool name to inspect."
                        },
                        "query": {
                            "type": "string",
                            "description": "Case-insensitive search across Synapse MCP tool names and descriptions."
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 200,
                            "description": "Maximum catalog rows to return."
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": SYNAPSE_TOOL_CALL,
                "description": "Call any real Synapse MCP tool by name with a JSON object of arguments. The target tool must come from synapse_tool_catalog.",
                "parameters": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Real Synapse MCP tool name to call."
                        },
                        "arguments": {
                            "type": "object",
                            "description": "JSON object passed to that Synapse MCP tool.",
                            "additionalProperties": true
                        }
                    },
                    "required": ["name", "arguments"]
                }
            }
        }),
    ]
}

fn resolve_tool_exposure(row: &LocalModelRegistryRow, tool_count: usize) -> ToolExposure {
    // Internalized models carry the surface in their weights — never inject a
    // catalog, regardless of max_tools. This takes precedence over the routed
    // fallback (which exists only for providers that cap tool count).
    if registry_runtime_preset(row) == "internalized_no_catalog" {
        return ToolExposure::Internalized;
    }
    match row.max_tools {
        Some(max_tools) if tool_count > max_tools => ToolExposure::Routed,
        _ => ToolExposure::Direct,
    }
}

/// Classifies an rmcp `call_tool` failure as terminal (the transport is gone,
/// so every subsequent call would fail identically) versus recoverable (the
/// server responded with an error, timed out, or returned an unexpected shape —
/// the model can be told and can try again). A recoverable error is fed back to
/// the model instead of crashing the agent.
fn tool_call_error_is_terminal(error: &ServiceError) -> bool {
    matches!(
        error,
        ServiceError::TransportClosed
            | ServiceError::TransportSend(_)
            | ServiceError::Cancelled { .. }
    )
}

fn parse_routed_tool_call(raw: &str) -> anyhow::Result<(String, JsonObject)> {
    let mut args = parse_tool_arguments(raw)?;
    let name = args
        .remove("name")
        .and_then(|value| value.as_str().map(str::trim).map(ToOwned::to_owned))
        .filter(|value| !value.is_empty())
        .context("routed tool call requires non-empty name")?;
    let arguments = match args.remove("arguments") {
        Some(Value::Object(map)) => map,
        Some(Value::String(raw)) => {
            let value: Value =
                serde_json::from_str(&raw).context("routed tool arguments string is not JSON")?;
            value
                .as_object()
                .cloned()
                .context("routed tool arguments string must decode to a JSON object")?
        }
        Some(other) => {
            bail!("routed tool arguments must be a JSON object, got {other}");
        }
        None => Map::new(),
    };
    Ok((name, arguments))
}

fn resolve_task(task: Option<&str>, task_file: Option<&PathBuf>) -> anyhow::Result<String> {
    if let Some(task) = task.filter(|value| !value.trim().is_empty()) {
        return Ok(task.to_owned());
    }
    if let Some(path) = task_file {
        let task = std::fs::read_to_string(path)
            .with_context(|| format!("read task file {}", path.display()))?;
        if task.trim().is_empty() {
            bail!("TOOL_PARAMS_INVALID: task file {} is empty", path.display());
        }
        return Ok(task);
    }
    bail!("TOOL_PARAMS_INVALID: --local-agent-task or --local-agent-task-file is required")
}

fn non_empty(value: Option<&str>, detail: &str) -> anyhow::Result<String> {
    value
        .filter(|text| !text.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!(detail.to_owned()))
}

fn validate_spawn_id(spawn_id: &str) -> anyhow::Result<()> {
    if !spawn_id.starts_with("agent-spawn-") {
        bail!("TOOL_PARAMS_INVALID: spawn id must start with agent-spawn-");
    }
    if spawn_id.len() > 128 {
        bail!("TOOL_PARAMS_INVALID: spawn id exceeds 128 chars");
    }
    if !spawn_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        bail!("TOOL_PARAMS_INVALID: spawn id must contain only ASCII alphanumerics and dashes");
    }
    Ok(())
}

fn validate_registry_row(row: &LocalModelRegistryRow) -> anyhow::Result<()> {
    if !row.enabled {
        bail!(
            "LOCAL_MODEL_DISABLED: registry row {:?} is disabled",
            row.name
        );
    }
    if row.api_shape != "open_ai_chat_completions" {
        bail!(
            "LOCAL_MODEL_API_SHAPE_UNSUPPORTED: {:?} is not open_ai_chat_completions",
            row.api_shape
        );
    }
    let Some(probe) = &row.last_probe else {
        bail!(
            "LOCAL_MODEL_UNPROBED: registry row {:?} has no last_probe",
            row.name
        );
    };
    if !probe.healthy {
        bail!(
            "LOCAL_MODEL_UNHEALTHY: {:?}: {} {}",
            row.name,
            probe.error_code.as_deref().unwrap_or("unknown"),
            probe.error_detail.as_deref().unwrap_or("")
        );
    }
    Ok(())
}

fn registry_runtime_preset(row: &LocalModelRegistryRow) -> &str {
    row.runtime_preset
        .as_deref()
        .unwrap_or("open_ai_compatible")
}

fn should_stream(cli_no_stream: bool, tool_exposure: ToolExposure) -> bool {
    !cli_no_stream && tool_exposure != ToolExposure::Internalized
}

fn apply_runtime_preset(row: &LocalModelRegistryRow, body: &mut Value) {
    match registry_runtime_preset(row) {
        "deepseek_v4_flash_non_thinking" => {
            body["thinking"] = json!({ "type": "disabled" });
        }
        "deepseek_v4_reasoning" => {
            body["thinking"] = json!({ "type": "enabled" });
            body["reasoning_effort"] = json!("max");
            if let Some(object) = body.as_object_mut() {
                object.remove("tool_choice");
            }
        }
        "internalized_no_catalog" => {
            // Surface is in the weights: send NO tool catalog and no forced
            // choice. The model emits tool calls from a near-empty prompt; the
            // response is parsed/routed exactly like Direct exposure.
            if let Some(object) = body.as_object_mut() {
                object.remove("tools");
                object.remove("tool_choice");
            }
        }
        _ => {}
    }
}

fn chat_completions_endpoint(
    row: &LocalModelRegistryRow,
    allow_non_loopback: bool,
) -> anyhow::Result<Url> {
    let mut url = Url::parse(&row.base_url)
        .with_context(|| format!("LOCAL_MODEL_ENDPOINT_INVALID: {:?}", row.base_url))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        bail!("LOCAL_MODEL_ENDPOINT_INVALID: endpoint scheme must be http or https");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("LOCAL_MODEL_ENDPOINT_INVALID: endpoint must not contain query or fragment");
    }
    if !(row.allow_non_loopback || allow_non_loopback || is_loopback_url(&url)) {
        bail!(
            "LOCAL_MODEL_ENDPOINT_NON_LOOPBACK: non-loopback endpoints require explicit allowance"
        );
    }
    let path = url.path().trim_end_matches('/');
    let next = if path.ends_with("/chat/completions") {
        path.to_owned()
    } else if path.is_empty() || path == "/" {
        "/v1/chat/completions".to_owned()
    } else {
        format!("{path}/chat/completions")
    };
    url.set_path(&next);
    Ok(url)
}

fn is_loopback_url(url: &Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|addr| addr.is_loopback()),
        None => false,
    }
}

fn agent_event_url(mcp_url: &str, spawn_id: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(mcp_url).context("parse local-agent MCP URL")?;
    url.set_path("/agent-events");
    url.set_query(Some(&format!(
        "spawn_id={spawn_id}&source=local_model_runner"
    )));
    Ok(url)
}

fn parse_tool_arguments(raw: &str) -> anyhow::Result<JsonObject> {
    if raw.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value = serde_json::from_str(raw)?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("tool arguments must be a JSON object"))
}

fn string_json_or_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}

fn assistant_message(completion: &ChatCompletion) -> Value {
    if completion.tool_calls.is_empty() {
        json!({
            "role": "assistant",
            "content": completion.content,
        })
    } else {
        let tool_calls = completion
            .tool_calls
            .iter()
            .map(|call| {
                json!({
                    "id": call.id,
                    "type": "function",
                    "function": {
                        "name": call.name,
                        "arguments": call.arguments,
                    }
                })
            })
            .collect::<Vec<_>>();
        json!({
            "role": "assistant",
            "content": completion.content,
            "tool_calls": tool_calls,
        })
    }
}

fn parse_non_stream_response(text: &str) -> anyhow::Result<ChatCompletion> {
    let value: Value = serde_json::from_str(text)
        .with_context(|| format!("MODEL_RESPONSE_INVALID_JSON: {}", bounded_text(text, 4000)))?;
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| anyhow::anyhow!("MODEL_RESPONSE_INVALID: missing choices[0]"))?;
    let message = choice
        .get("message")
        .ok_or_else(|| anyhow::anyhow!("MODEL_RESPONSE_INVALID: missing choices[0].message"))?;
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| calls.iter().filter_map(parse_tool_call_value).collect())
        .unwrap_or_default();
    Ok(ChatCompletion {
        content,
        tool_calls,
        finish_reason: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        usage: value.get("usage").and_then(Usage::from_value),
        raw_sha256: sha256_hex(text.as_bytes()),
    })
}

fn parse_tool_call_value(value: &Value) -> Option<OpenAiToolCall> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("call_missing_id")
        .to_owned();
    let function = value.get("function")?;
    let name = function.get("name").and_then(Value::as_str)?.to_owned();
    let arguments = match function.get("arguments") {
        Some(Value::String(text)) => text.to_owned(),
        Some(value) => value.to_string(),
        None => "{}".to_owned(),
    };
    Some(OpenAiToolCall {
        id,
        name,
        arguments,
    })
}

async fn parse_streaming_response(response: reqwest::Response) -> anyhow::Result<ChatCompletion> {
    let mut stream = response.bytes_stream();
    let mut pending = String::new();
    let mut raw = Vec::new();
    let mut content = String::new();
    let mut calls: BTreeMap<u64, StreamingToolCall> = BTreeMap::new();
    let mut usage = None;
    let mut finish_reason = None;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read streaming response chunk")?;
        raw.extend_from_slice(&chunk);
        pending.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(index) = pending.find('\n') {
            let mut line = pending[..index].trim_end_matches('\r').to_owned();
            pending = pending[index + 1..].to_owned();
            line = line.trim().to_owned();
            if !line.starts_with("data:") {
                continue;
            }
            let data = line.trim_start_matches("data:").trim();
            if data == "[DONE]" || data.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(data)
                .with_context(|| format!("MODEL_STREAM_CHUNK_INVALID_JSON: {data}"))?;
            if let Some(next_usage) = value.get("usage").and_then(Usage::from_value) {
                usage = Some(next_usage);
            }
            let Some(choice) = value
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
            else {
                continue;
            };
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                finish_reason = Some(reason.to_owned());
            }
            let Some(delta) = choice.get("delta") else {
                continue;
            };
            if let Some(piece) = delta.get("content").and_then(Value::as_str) {
                content.push_str(piece);
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for value in tool_calls {
                    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
                    let entry = calls.entry(index).or_default();
                    if let Some(id) = value.get("id").and_then(Value::as_str) {
                        entry.id = Some(id.to_owned());
                    }
                    if let Some(function) = value.get("function") {
                        if let Some(name) = function.get("name").and_then(Value::as_str) {
                            entry.name = Some(name.to_owned());
                        }
                        match function.get("arguments") {
                            Some(Value::String(part)) => entry.arguments.push_str(part),
                            Some(other) => entry.arguments.push_str(&other.to_string()),
                            None => {}
                        }
                    }
                }
            }
        }
    }
    let tool_calls = calls
        .into_iter()
        .map(|(index, call)| OpenAiToolCall {
            id: call.id.unwrap_or_else(|| format!("call_{index}")),
            name: call.name.unwrap_or_default(),
            arguments: call.arguments,
        })
        .filter(|call| !call.name.is_empty())
        .collect::<Vec<_>>();
    Ok(ChatCompletion {
        content,
        tool_calls,
        finish_reason,
        usage,
        raw_sha256: sha256_hex(&raw),
    })
}

fn bounded_result_text(value: &Value) -> String {
    bounded_text(&value.to_string(), 16_000)
}

#[derive(Clone, Debug, PartialEq)]
struct ShutdownMailboxMessage {
    message_id: String,
    kind: String,
    payload: Value,
}

fn shutdown_message_from_tool_result(
    tool_name: &str,
    result_value: &Value,
) -> Option<ShutdownMailboxMessage> {
    for message in mailbox_messages_from_tool_result(tool_name, result_value) {
        let kind = message
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("message")
            .trim()
            .to_owned();
        let payload = message.get("payload").cloned().unwrap_or(Value::Null);
        if steering_requests_shutdown(&kind, &payload) {
            let message_id = message
                .get("message_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            return Some(ShutdownMailboxMessage {
                message_id,
                kind,
                payload,
            });
        }
    }
    None
}

fn mailbox_messages_from_tool_result<'a>(
    tool_name: &str,
    result_value: &'a Value,
) -> Vec<&'a Value> {
    match tool_name {
        "agent_inbox" => result_value
            .get("messages")
            .and_then(Value::as_array)
            .map(|messages| messages.iter().collect())
            .unwrap_or_default(),
        "agent_wait" => result_value
            .get("inbox")
            .and_then(|inbox| inbox.get("messages"))
            .and_then(Value::as_array)
            .map(|messages| messages.iter().collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_owned()
    } else {
        value.chars().take(max_chars).collect::<String>()
    }
}

fn steering_payload_text(payload: &Value) -> String {
    if let Some(text) = payload.as_str() {
        return text.to_owned();
    }
    for field in [
        "text",
        "message",
        "content",
        "prompt",
        "instruction",
        "instructions",
        "body",
        "command",
    ] {
        if let Some(text) = payload.get(field).and_then(Value::as_str) {
            return text.to_owned();
        }
    }
    payload.to_string()
}

fn steering_requests_shutdown(kind: &str, payload: &Value) -> bool {
    let kind = kind.trim().to_ascii_lowercase();
    if matches!(
        kind.as_str(),
        "kill" | "stop" | "cancel" | "interrupt" | "shutdown"
    ) {
        return true;
    }
    for field in ["command", "action", "control"] {
        if let Some(value) = payload.get(field).and_then(Value::as_str) {
            let value = value.trim().to_ascii_lowercase();
            if matches!(
                value.as_str(),
                "kill" | "stop" | "cancel" | "interrupt" | "shutdown"
            ) {
                return true;
            }
        }
    }
    false
}

fn write_json_file(path: PathBuf, value: &Value) -> anyhow::Result<()> {
    let encoded = serde_json::to_vec_pretty(value).context("serialize local-agent JSON file")?;
    std::fs::write(&path, encoded).with_context(|| format!("write {}", path.display()))
}

fn local_agent_spawn_root_dir() -> anyhow::Result<PathBuf> {
    let local_appdata = std::env::var_os("LOCALAPPDATA")
        .context("LOCAL_MODEL_AGENT_LOG_ROOT_UNAVAILABLE: LOCALAPPDATA is not set")?;
    Ok(PathBuf::from(local_appdata)
        .join("Synapse")
        .join("agent-spawns"))
}

/// Behavioral doctrine prepended to every spawned agent's conversation (the
/// system message). Compressed per `docs/compressionprompt.md`: the load-bearing
/// constraints and the routed meta-tool names are kept verbatim; meta-framing,
/// restatement, and why-prose are cut; imperative present tense; one rule per
/// line; controlled vocabulary ("tool", not "function/MCP tool" interchangeably).
///
/// Tool *awareness* is intentionally NOT injected here: spawned local models
/// carry the Synapse surface in their LoRA'd weights, and API models receive the
/// full tool schemas from the MCP attachment (Direct exposure). The manifest
/// approach is therefore retired; this prompt only governs behavior.
fn system_prompt(tool_exposure: ToolExposure, tools: &[Tool]) -> String {
    const BASE: &str = "Synapse agent. Use attached MCP tools to inspect/change state.\nRules:\n- Never invent tool results.\n- Stored artifact -> read back before success.\n- post_write_readback.matched=true => success; do not repeat write.\n- Summarize after required tools succeed.";
    const INTERNALIZED_BASE: &str = "Synapse agent. Return one tool call and no prose when a tool is needed.\nRules:\n- Use exact argument keys.\n- Never invent tool results.\n- Stored artifact -> read back before success.\n- post_write_readback.matched=true => success; do not repeat write.";
    match tool_exposure {
        // Direct: the model holds every tool schema natively, so it just needs
        // to know they are there and callable by name.
        ToolExposure::Direct => format!(
            "{BASE}\n\nEvery Synapse tool is attached directly; call any by name and read its schema from your tool list."
        ),
        // Routed: a capped provider sees only the two meta-tools, so the
        // indirection mechanism is load-bearing and stated exactly.
        ToolExposure::Routed => {
            let names = tools
                .iter()
                .map(|tool| tool.name.as_ref())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{BASE}\n\nYour provider caps tool count below the full Synapse surface, so tools route through two meta-tools:\n- {catalog} — list/inspect tools (name=<tool> for one, query=<text> to search).\n- {call_tool} {{name, arguments}} — execute any real Synapse tool.\nThis reaches every Synapse tool (perception, action, agent, task, dashboard, local-model). Tools: {names}",
                catalog = SYNAPSE_TOOL_CATALOG,
                call_tool = SYNAPSE_TOOL_CALL,
            )
        }
        // Internalized: the surface is in the weights and the model was trained
        // on bare user turns — keep the prompt to behavioral rules only, with no
        // tool framing, to stay on-distribution and near-empty.
        ToolExposure::Internalized => INTERNALIZED_BASE.to_string(),
    }
}

/// One-line, exposure-aware recovery hint fed back to the model when a tool call
/// fails. Compressed (imperative, no meta) and *correct per exposure*: only
/// Routed sessions actually have `synapse_tool_catalog`, so Direct (API-model)
/// sessions must not be told to use a tool that is not attached.
fn tool_failure_suggestion(fatal: bool, exposure: ToolExposure) -> &'static str {
    if fatal {
        return "Connection to the Synapse tool server was lost; the run is ending.";
    }
    match exposure {
        ToolExposure::Routed => {
            "Tool call failed. Read the message, re-check the tool's schema with synapse_tool_catalog, then retry with corrected arguments or pick a different tool."
        }
        // Direct and Internalized both carry the schemas (attached vs. in
        // weights); neither has the routed catalog meta-tool to consult.
        ToolExposure::Direct | ToolExposure::Internalized => {
            "Tool call failed. Read the message, re-check the tool's input schema, then retry with corrected arguments or pick a different tool."
        }
    }
}

/// Verdict the in-daemon `approval_gate` returned to the harness for a gated
/// tool call (#1028).
enum ToolGate {
    /// Approved (by the operator or by auto-allow). Carries the EFFECTIVE
    /// arguments — the operator's edited args when they approved-with-edits
    /// (#1030), otherwise the original input.
    Allow(JsonObject),
    /// Denied (operator declined, gate timed out, or the gate could not answer).
    /// Carries the reason fed back to the model; the tool is NOT dispatched.
    Deny(String),
}

/// The tool-name form the harness hands the approval policy/gate so a bare
/// Synapse tool name (e.g. `act_type`) classifies as the MCP tool it is
/// (`mcp__synapse__act_type`) — otherwise `permission_policy::classify` would
/// hit its unknown-bare-name branch and gate every call indiscriminately.
fn gate_tool_label(tool_name: &str) -> String {
    format!("mcp__synapse__{tool_name}")
}

/// Parse the `approval_gate` verdict value (`{"behavior":"allow","updatedInput":…}`
/// or `{"behavior":"deny","message":…}`). Fail-closed: an unknown/!object/missing
/// behavior is an error the caller turns into a denial — never a silent allow.
fn parse_gate_verdict(verdict: &Value, fallback: &JsonObject) -> anyhow::Result<ToolGate> {
    match verdict.get("behavior").and_then(Value::as_str) {
        Some("allow") => {
            let args = match verdict.get("updatedInput") {
                Some(Value::Object(map)) => map.clone(),
                None | Some(Value::Null) => fallback.clone(),
                Some(other) => {
                    bail!("approval_gate updatedInput must be a JSON object, got {other}")
                }
            };
            Ok(ToolGate::Allow(args))
        }
        Some("deny") => Ok(ToolGate::Deny(
            verdict
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("denied by operator")
                .to_owned(),
        )),
        other => bail!("approval_gate returned an unknown behavior {other:?}: {verdict}"),
    }
}

fn error_code_from_detail(detail: &str) -> &str {
    for code in [
        "MODEL_ENDPOINT_UNREACHABLE",
        "MODEL_TOOLS_UNSUPPORTED",
        "MODEL_EMPTY_COMPLETION",
        "MODEL_TOOL_CALL_INVALID",
        "LOCAL_AGENT_CONTEXT_OVERFLOW",
        "LOCAL_AGENT_INTERRUPTED",
        "LOCAL_AGENT_TURN_LIMIT",
        "AGENT_EVENT_INGRESS_WRITE_FAILED",
        "LOCAL_MODEL_UNHEALTHY",
        "LOCAL_MODEL_DISABLED",
        "LOCAL_MODEL_UNPROBED",
        "LOCAL_MODEL_ENDPOINT_NON_LOOPBACK",
        "SYNAPSE_TOOL_CALL_FAILED",
        "TOOL_PARAMS_INVALID",
    ] {
        if detail.contains(code) {
            return code;
        }
    }
    "LOCAL_MODEL_AGENT_FAILED"
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    out
}

fn unix_time_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use reqwest::header;

    use super::*;

    #[test]
    fn agent_wait_tool_result_kill_is_runner_control() {
        let result = json!({
            "ok": true,
            "timed_out": false,
            "inbox": {
                "messages": [{
                    "message_id": "agentmsg-1",
                    "kind": "kill",
                    "payload": { "reason": "operator acceptance" }
                }]
            }
        });

        assert_eq!(
            shutdown_message_from_tool_result("agent_wait", &result),
            Some(ShutdownMailboxMessage {
                message_id: "agentmsg-1".to_owned(),
                kind: "kill".to_owned(),
                payload: json!({ "reason": "operator acceptance" }),
            })
        );
    }

    #[test]
    fn agent_wait_tool_result_normal_steer_stays_model_visible() {
        let result = json!({
            "ok": true,
            "timed_out": false,
            "inbox": {
                "messages": [{
                    "message_id": "agentmsg-2",
                    "kind": "steer",
                    "payload": { "instruction": "write the marker row" }
                }]
            }
        });

        assert_eq!(
            shutdown_message_from_tool_result("agent_wait", &result),
            None
        );
    }

    #[test]
    fn tool_call_server_error_is_recoverable_not_terminal() {
        // The paint-death case: gemma called capture_screenshot without the
        // required `path`, so the server answered with a JSON-RPC error
        // (-32099 / invalid params). That is an McpError — the transport is
        // fine and the model can be told and retry. It MUST NOT be terminal.
        let server_error = ServiceError::McpError(rmcp::model::ErrorData::invalid_params(
            "missing field `path`",
            None,
        ));
        assert!(
            !tool_call_error_is_terminal(&server_error),
            "a server-side invalid-params error is recoverable; the agent must feed it back and continue"
        );

        // Timeouts and unexpected responses are also recoverable retries.
        assert!(!tool_call_error_is_terminal(&ServiceError::Timeout {
            timeout: std::time::Duration::from_millis(1),
        }));
        assert!(!tool_call_error_is_terminal(
            &ServiceError::UnexpectedResponse
        ));
    }

    #[test]
    fn tool_call_transport_loss_is_terminal() {
        // A dead pipe cannot recover; every further call would fail identically.
        assert!(tool_call_error_is_terminal(&ServiceError::TransportClosed));
        assert!(tool_call_error_is_terminal(&ServiceError::Cancelled {
            reason: Some("peer gone".to_owned()),
        }));
    }

    #[test]
    fn routed_tool_call_parses_real_tool_name_and_arguments() -> anyhow::Result<()> {
        let (name, args) = parse_routed_tool_call(
            r#"{"name":"workspace_put","arguments":{"run_id":"issue985","key":"ok","value":{"actual":true}}}"#,
        )?;
        assert_eq!(name, "workspace_put");
        assert_eq!(args["run_id"], "issue985");
        assert_eq!(args["value"]["actual"], true);
        Ok(())
    }

    #[test]
    fn assistant_tool_call_message_keeps_string_content_for_chat_templates() {
        let completion = ChatCompletion {
            content: String::new(),
            tool_calls: vec![OpenAiToolCall {
                id: "call-1".to_owned(),
                name: "workspace_get".to_owned(),
                arguments: r#"{"key":"probe"}"#.to_owned(),
            }],
            ..ChatCompletion::default()
        };
        let message = assistant_message(&completion);
        assert_eq!(message["role"], "assistant");
        assert_eq!(message["content"], "");
        assert!(message["content"].is_string());
        assert_eq!(
            message["tool_calls"][0]["function"]["name"],
            "workspace_get"
        );
    }

    #[test]
    fn workspace_put_readback_plan_uses_exact_key_and_run_id() -> anyhow::Result<()> {
        let put_args: JsonObject = serde_json::from_value(json!({
            "run_id": "issue1053",
            "key": "known-key",
            "value": {"expected": 4, "actual": 4},
        }))?;
        let plan = workspace_put_readback_plan(&put_args)?;
        assert_eq!(plan.arguments["run_id"], "issue1053");
        assert_eq!(plan.arguments["key"], "known-key");
        assert_eq!(plan.expected_value["actual"], 4);
        Ok(())
    }

    #[test]
    fn workspace_put_readback_record_fails_on_value_mismatch() -> anyhow::Result<()> {
        let put_args: JsonObject = serde_json::from_value(json!({
            "run_id": "issue1053",
            "key": "known-key",
            "value": {"expected": 4, "actual": 4},
        }))?;
        let plan = workspace_put_readback_plan(&put_args)?;
        let mismatch = json!({
            "entry": {
                "value": {"expected": 4, "actual": 5},
            },
            "storage_readback": {
                "value_sha256": "sha256:synthetic",
            },
        });
        let error = workspace_put_readback_record(&plan, &mismatch)
            .expect_err("mismatched readback must fail");
        assert!(
            error
                .to_string()
                .contains("WORKSPACE_PUT_READBACK_MISMATCH")
        );
        Ok(())
    }

    #[test]
    fn workspace_put_readback_record_preserves_storage_hash() -> anyhow::Result<()> {
        let put_args: JsonObject = serde_json::from_value(json!({
            "run_id": "issue1053",
            "key": "known-key",
            "value": {"expected": 4, "actual": 4},
        }))?;
        let plan = workspace_put_readback_plan(&put_args)?;
        let readback = json!({
            "entry": {
                "value": {"expected": 4, "actual": 4},
            },
            "storage_readback": {
                "value_sha256": "sha256:synthetic",
            },
        });
        let record = workspace_put_readback_record(&plan, &readback)?;
        assert_eq!(record["matched"], true);
        assert_eq!(record["actual_value"]["actual"], 4);
        assert_eq!(
            record["storage_readback"]["value_sha256"],
            "sha256:synthetic"
        );
        Ok(())
    }

    #[test]
    fn workspace_put_args_match_only_exact_duplicates() -> anyhow::Result<()> {
        let first: JsonObject = serde_json::from_value(json!({
            "run_id": "issue1054",
            "key": "known-key",
            "value": {"expected": "done", "actual": "done"},
        }))?;
        let same: JsonObject = serde_json::from_value(json!({
            "key": "known-key",
            "value": {"actual": "done", "expected": "done"},
            "run_id": "issue1054",
        }))?;
        let different: JsonObject = serde_json::from_value(json!({
            "run_id": "issue1054",
            "key": "known-key",
            "value": {"expected": "done", "actual": "changed"},
        }))?;
        assert!(workspace_put_args_match(&first, &same));
        assert!(!workspace_put_args_match(&first, &different));
        Ok(())
    }

    #[test]
    fn model_tool_call_pre_gate_rejects_workspace_put_without_value_or_artifact()
    -> anyhow::Result<()> {
        let args: JsonObject = serde_json::from_value(json!({
            "run_id": "issue1055",
            "key": "missing-value",
        }))?;
        let reason = model_tool_call_pre_gate_rejection("workspace_put", &args, true)
            .expect("workspace_put without value/artifact must fail before approval");
        assert!(reason.contains("requires at least one of value or artifact"));

        let with_value: JsonObject = serde_json::from_value(json!({
            "run_id": "issue1055",
            "key": "ok",
            "value": null,
        }))?;
        assert!(model_tool_call_pre_gate_rejection("workspace_put", &with_value, true).is_none());
        Ok(())
    }

    #[test]
    fn model_tool_call_pre_gate_rejects_unknown_tool_before_approval() -> anyhow::Result<()> {
        let args: JsonObject = serde_json::from_value(json!({
            "malformed_output": true,
        }))?;
        let reason = model_tool_call_pre_gate_rejection("agent_retry", &args, false)
            .expect("invented tool names must fail before approval");
        assert!(reason.contains("not present in Synapse tools/list"));
        assert!(reason.contains("real Synapse tool name"));
        Ok(())
    }

    #[test]
    fn model_tool_call_pre_gate_rejects_local_model_approval_control() -> anyhow::Result<()> {
        let args: JsonObject = serde_json::from_value(json!({
            "approval_id": "apr1-test",
            "decision": "decline",
        }))?;
        let reason = model_tool_call_pre_gate_rejection("approval_decide", &args, true)
            .expect("local models must not self-decide approval rows");
        assert!(reason.contains("runner/operator-control"));
        assert!(model_tool_call_pre_gate_rejection("approval_gate", &args, true).is_some());
        assert!(model_tool_call_pre_gate_rejection("agent_send", &args, true).is_some());
        assert!(model_tool_call_pre_gate_rejection("workspace_get", &args, true).is_none());
        Ok(())
    }

    #[test]
    fn tool_exposure_routes_when_provider_tool_cap_is_lower_than_synapse_surface() {
        let mut row = test_local_agent_row();
        row.max_tools = Some(128);
        assert_eq!(resolve_tool_exposure(&row, 141), ToolExposure::Routed);
        assert_eq!(resolve_tool_exposure(&row, 128), ToolExposure::Direct);
    }

    #[test]
    fn internalized_preset_never_injects_a_catalog_regardless_of_max_tools() {
        // An internalized model carries the surface in its weights; the harness
        // must inject ZERO tools even when the surface exceeds any cap.
        let mut row = test_local_agent_row();
        row.runtime_preset = Some("internalized_no_catalog".to_owned());
        row.max_tools = Some(16);
        assert_eq!(resolve_tool_exposure(&row, 141), ToolExposure::Internalized);
        assert_eq!(resolve_tool_exposure(&row, 1), ToolExposure::Internalized);
        // The behavioral prompt carries no tool framing (no catalog meta-tools).
        let prompt = system_prompt(ToolExposure::Internalized, &[]);
        assert!(!prompt.contains(SYNAPSE_TOOL_CATALOG));
        assert!(!prompt.contains("attached MCP tools"));
        assert!(!prompt.contains("attached directly"));
        assert!(prompt.contains("one tool call and no prose"));
        assert!(prompt.contains("exact argument keys"));
        assert!(prompt.contains("post_write_readback.matched=true"));
        // Internalized serving endpoints are validated through non-streaming
        // probes and may return JSON even when asked to stream; keep the runner
        // on the same non-streaming path so tool_calls are parsed.
        assert!(!should_stream(false, ToolExposure::Internalized));
        assert!(!should_stream(true, ToolExposure::Internalized));
        assert!(should_stream(false, ToolExposure::Direct));
        assert!(!should_stream(true, ToolExposure::Direct));
        // The request body for an internalized session must drop tools+choice.
        let mut body = json!({"tools": [], "tool_choice": "auto", "model": "x"});
        apply_runtime_preset(&row, &mut body);
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn deepseek_runtime_presets_shape_chat_body() {
        let mut row = test_local_agent_row();
        row.runtime_preset = Some("deepseek_v4_flash_non_thinking".to_owned());
        let mut flash = json!({"tool_choice": "auto"});
        apply_runtime_preset(&row, &mut flash);
        assert_eq!(flash["thinking"]["type"], "disabled");
        assert!(flash.get("tool_choice").is_some());

        row.runtime_preset = Some("deepseek_v4_reasoning".to_owned());
        let mut reasoning = json!({"tool_choice": "auto"});
        apply_runtime_preset(&row, &mut reasoning);
        assert_eq!(reasoning["thinking"]["type"], "enabled");
        assert_eq!(reasoning["reasoning_effort"], "max");
        assert!(reasoning.get("tool_choice").is_none());
    }

    fn prompt_test_tools() -> Vec<Tool> {
        ["observe", "act_type", "agent_send"]
            .iter()
            .map(|name| {
                serde_json::from_value(json!({
                    "name": name,
                    "description": "synthetic",
                    "inputSchema": {"type": "object", "properties": {}},
                }))
                .expect("synthetic Tool")
            })
            .collect()
    }

    /// Round-trip proxy for compressionprompt.md §5.5: every load-bearing
    /// constraint from the original verbose prompt must survive the compression,
    /// and the result must actually be shorter (compression, not bloat). Also
    /// pins the Direct/Routed correctness invariant: a Direct (API-model) session
    /// is never told about `synapse_tool_catalog`, which it does not have.
    #[test]
    fn system_prompt_keeps_load_bearing_constraints_and_compresses() {
        // The exact verbose prompts this change replaced (origin/main 3537242).
        const OLD_BASE: &str = "You are a local Synapse agent. Use the provided MCP tools to inspect and change state. Never invent tool results. When a task asks for a stored artifact, call the relevant Synapse tool and then read it back. Finish with a concise summary only after the needed tool calls have succeeded.";
        let old_direct = format!(
            "{OLD_BASE}\n\nAll Synapse MCP tools from this session's strict tools/list are attached directly as model tools."
        );
        let old_routed = format!(
            "{OLD_BASE}\n\nThis provider has a lower function-count cap than the live Synapse tool surface, so tools are exposed through a routed harness. Call synapse_tool_catalog to inspect exact tool schemas and call synapse_tool with a real Synapse tool name plus arguments to execute it. The routed harness can call every real Synapse MCP tool loaded by this session, including file, shell, browser/perception, agent, dashboard, and local-model tools. Live tool names: observe, act_type, agent_send"
        );

        let tools = prompt_test_tools();
        let direct = system_prompt(ToolExposure::Direct, &tools);
        let routed = system_prompt(ToolExposure::Routed, &tools);

        // Load-bearing behavioral constraints survive verbatim in both modes.
        for prompt in [&direct, &routed] {
            assert!(prompt.contains("Synapse agent"), "identity");
            assert!(prompt.contains("inspect/change state"), "capability");
            assert!(
                prompt.contains("Never invent tool results"),
                "no-fabrication"
            );
            assert!(prompt.contains("read back"), "read-back verification");
            assert!(
                prompt.contains("post_write_readback.matched=true"),
                "driver-readback terminal success"
            );
            assert!(
                prompt.contains("Summarize after required tools succeed"),
                "summary-gating"
            );
        }
        // Direct mode must NOT advertise the routed-only meta-tools (it has the
        // real schemas attached; synapse_tool_catalog is not in its tool list).
        assert!(!direct.contains(SYNAPSE_TOOL_CATALOG));
        assert!(!direct.contains(SYNAPSE_TOOL_CALL));
        // Routed mode keeps the indirection mechanism and the live names.
        assert!(routed.contains(SYNAPSE_TOOL_CATALOG));
        assert!(routed.contains(SYNAPSE_TOOL_CALL));
        for tool in &tools {
            assert!(routed.contains(tool.name.as_ref()));
        }

        // Compression: char count is the tiktoken-absent proxy (~4 chars/token).
        println!(
            "readback=prompt_compress direct old={} new={} routed old={} new={}",
            old_direct.len(),
            direct.len(),
            old_routed.len(),
            routed.len()
        );
        assert!(
            direct.len() < old_direct.len(),
            "Direct prompt must be shorter than the verbose original"
        );
        assert!(
            routed.len() < old_routed.len(),
            "Routed prompt must be shorter than the verbose original"
        );
    }

    #[test]
    fn tool_failure_suggestion_is_exposure_aware() {
        // Direct sessions have no synapse_tool_catalog — must not be told to use it.
        let direct = tool_failure_suggestion(false, ToolExposure::Direct);
        assert!(!direct.contains("synapse_tool_catalog"));
        assert!(direct.contains("input schema"));
        // Routed sessions do have it — the hint names it.
        let routed = tool_failure_suggestion(false, ToolExposure::Routed);
        assert!(routed.contains("synapse_tool_catalog"));
        // A dead transport is terminal regardless of exposure.
        assert!(tool_failure_suggestion(true, ToolExposure::Direct).contains("run is ending"));
        assert!(tool_failure_suggestion(true, ToolExposure::Routed).contains("run is ending"));
    }

    #[test]
    fn gate_label_makes_bare_synapse_tools_classify_as_mcp_tools() {
        use crate::server::permission_policy::classify;
        // A hazardous action tool must gate; a read-only one must auto-allow —
        // proving the harness shares the daemon's single classification authority.
        assert_eq!(
            gate_tool_label("act_run_shell"),
            "mcp__synapse__act_run_shell"
        );
        assert!(
            classify(&gate_tool_label("act_run_shell"), &json!({})).is_gate(),
            "act_run_shell must be gated"
        );
        assert!(
            classify(&gate_tool_label("act_type"), &json!({})).is_gate(),
            "act_type (typing into apps) must be gated"
        );
        assert!(
            !classify(&gate_tool_label("observe"), &json!({})).is_gate(),
            "observe is read-only and must auto-allow"
        );
        assert!(
            !classify(&gate_tool_label("read_text"), &json!({})).is_gate(),
            "read_text is read-only and must auto-allow"
        );
    }

    #[test]
    fn parse_gate_verdict_allow_prefers_operator_edited_args() {
        let fallback: JsonObject =
            serde_json::from_value(json!({"command": "original"})).expect("fallback object");
        // Approve-with-edits (#1030): the edited args win.
        let edited = json!({"behavior": "allow", "updatedInput": {"command": "edited"}});
        match parse_gate_verdict(&edited, &fallback).expect("allow") {
            ToolGate::Allow(args) => assert_eq!(args["command"], "edited"),
            ToolGate::Deny(reason) => panic!("expected allow, got deny: {reason}"),
        }
        // Plain allow (no updatedInput) falls back to the original input.
        let plain = json!({"behavior": "allow"});
        match parse_gate_verdict(&plain, &fallback).expect("allow") {
            ToolGate::Allow(args) => assert_eq!(args["command"], "original"),
            ToolGate::Deny(reason) => panic!("expected allow, got deny: {reason}"),
        }
    }

    #[test]
    fn parse_gate_verdict_deny_carries_reason() {
        let fallback = JsonObject::new();
        let deny = json!({"behavior": "deny", "message": "not safe right now"});
        match parse_gate_verdict(&deny, &fallback).expect("deny parses") {
            ToolGate::Deny(reason) => assert_eq!(reason, "not safe right now"),
            ToolGate::Allow(_) => panic!("expected deny"),
        }
    }

    #[test]
    fn parse_gate_verdict_is_fail_closed_on_garbage() {
        let fallback = JsonObject::new();
        // Unknown behavior, missing behavior, and a non-object updatedInput must
        // all error (the caller turns the error into a denial) — never allow.
        assert!(parse_gate_verdict(&json!({"behavior": "maybe"}), &fallback).is_err());
        assert!(parse_gate_verdict(&json!({}), &fallback).is_err());
        assert!(
            parse_gate_verdict(
                &json!({"behavior": "allow", "updatedInput": "not-an-object"}),
                &fallback
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn local_model_endpoint_env_probe_requires_real_tool_call() -> anyhow::Result<()> {
        let base_url = match std::env::var("SYNAPSE_LOCAL_AGENT_ITEST_BASE_URL") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => {
                eprintln!(
                    "SKIP_LOCAL_MODEL_ENDPOINT_ITEST: set SYNAPSE_LOCAL_AGENT_ITEST_BASE_URL and SYNAPSE_LOCAL_AGENT_ITEST_MODEL to run against a real OpenAI-compatible local endpoint"
                );
                return Ok(());
            }
        };
        let model_id = match std::env::var("SYNAPSE_LOCAL_AGENT_ITEST_MODEL") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => {
                eprintln!(
                    "SKIP_LOCAL_MODEL_ENDPOINT_ITEST: SYNAPSE_LOCAL_AGENT_ITEST_MODEL is absent"
                );
                return Ok(());
            }
        };
        let allow_non_loopback = std::env::var("SYNAPSE_LOCAL_AGENT_ITEST_ALLOW_NON_LOOPBACK")
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);
        let row = LocalModelRegistryRow {
            name: "env-local-agent-itest".to_owned(),
            base_url,
            model_id: model_id.clone(),
            enabled: true,
            allow_non_loopback,
            api_key_env_var: None,
            api_shape: "open_ai_chat_completions".to_owned(),
            runtime_preset: None,
            context_length: None,
            max_tools: None,
            last_probe: Some(LocalModelProbe {
                healthy: true,
                error_code: None,
                error_detail: None,
            }),
        };
        validate_registry_row(&row)?;
        let endpoint = chat_completions_endpoint(&row, allow_non_loopback)?;
        let nonce = format!("local-agent-itest-{}", Uuid::now_v7().simple());
        let request_body = json!({
            "model": model_id,
            "messages": [
                {
                    "role": "system",
                    "content": "Return no prose. Call the requested tool exactly once."
                },
                {
                    "role": "user",
                    "content": format!("Call synapse_probe with nonce {nonce:?}.")
                }
            ],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "synapse_probe",
                        "description": "Echo the provided nonce to prove structured tool calling works.",
                        "parameters": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "nonce": { "type": "string" }
                            },
                            "required": ["nonce"]
                        }
                    }
                }
            ],
            "tool_choice": {
                "type": "function",
                "function": { "name": "synapse_probe" }
            },
            "stream": false,
            "temperature": 0,
            "max_tokens": 128
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;
        let mut request = client
            .post(endpoint.clone())
            .header(header::ACCEPT, "application/json")
            .json(&request_body);
        if let Ok(token) = std::env::var("SYNAPSE_LOCAL_AGENT_ITEST_API_KEY") {
            if !token.trim().is_empty() {
                request = request.bearer_auth(token);
            }
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("real local endpoint {endpoint} request failed"))?;
        let status = response.status();
        let body = response.text().await.context("read endpoint response")?;
        assert!(
            status.is_success(),
            "real local endpoint returned HTTP {status}: {}",
            bounded_text(&body, 4000)
        );
        let completion = parse_non_stream_response(&body)?;
        let call = completion
            .tool_calls
            .iter()
            .find(|call| call.name == "synapse_probe")
            .context("real endpoint did not return a synapse_probe tool call")?;
        let args: Value = serde_json::from_str(&call.arguments)
            .context("real endpoint returned malformed tool arguments")?;
        assert_eq!(
            args.get("nonce").and_then(Value::as_str),
            Some(nonce.as_str()),
            "real endpoint tool call nonce must match"
        );
        eprintln!(
            "LOCAL_MODEL_ENDPOINT_ITEST_OK endpoint={} model={} prompt_tokens={:?} completion_tokens={:?}",
            endpoint,
            row.model_id,
            completion.usage.as_ref().map(|usage| usage.prompt_tokens),
            completion
                .usage
                .as_ref()
                .map(|usage| usage.completion_tokens)
        );
        Ok(())
    }

    fn test_local_agent_row() -> LocalModelRegistryRow {
        LocalModelRegistryRow {
            name: "deepseek-flash".to_owned(),
            base_url: "https://api.deepseek.com".to_owned(),
            model_id: "deepseek-v4-flash".to_owned(),
            enabled: true,
            allow_non_loopback: true,
            api_key_env_var: Some("DEEPSEEK_API_KEY".to_owned()),
            api_shape: "open_ai_chat_completions".to_owned(),
            runtime_preset: Some("deepseek_v4_flash_non_thinking".to_owned()),
            context_length: Some(1_000_000),
            max_tools: Some(128),
            last_probe: Some(LocalModelProbe {
                healthy: true,
                error_code: None,
                error_detail: None,
            }),
        }
    }
}
