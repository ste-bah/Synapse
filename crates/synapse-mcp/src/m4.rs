use std::{
    collections::BTreeMap,
    io,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Context;
use rmcp::{ErrorData, model::ErrorCode, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synapse_core::{Action, Backend, ComboInput, ComboStep, Key, error_codes, new_reflex_id};
use synapse_reflex::{ComboParams, ReflexRuntime, ScheduledReflex};
use tokio::{io::AsyncReadExt, process::Command};

use crate::{
    m1::mcp_error,
    m2::{ActPressParams, action_from_press_params},
    m3::permissions::{RequiredPermissions, add_action_permissions},
};

const MAX_COMBO_STEPS: usize = 256;
const DEFAULT_SHELL_TIMEOUT_MS: u32 = 30_000;
const DEFAULT_LAUNCH_TIMEOUT_MS: u32 = 10_000;
const ALLOW_SHELL_ENV: &str = "SYNAPSE_ALLOW_SHELL";
const ALLOW_LAUNCH_ENV: &str = "SYNAPSE_ALLOW_LAUNCH";
const SHELL_OUTPUT_CAP_BYTES: usize = 1024 * 1024;
const SHELL_BASE_ENV_KEYS: [&str; 4] = ["PATH", "USERPROFILE", "TEMP", "SystemRoot"];

#[derive(Clone, Debug, Default)]
pub struct M4ServiceConfig {
    allow_shell: Vec<AllowPattern>,
    allow_launch: Vec<AllowPattern>,
}

#[derive(Clone, Debug)]
struct AllowPattern {
    raw: String,
    regex: regex::Regex,
}

impl M4ServiceConfig {
    pub fn from_cli_parts(
        allow_shell: Vec<String>,
        allow_launch: Vec<String>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            allow_shell: compile_allow_patterns(ALLOW_SHELL_ENV, allow_shell)?,
            allow_launch: compile_allow_patterns(ALLOW_LAUNCH_ENV, allow_launch)?,
        })
    }

    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_cli_parts(
            parse_env_list(ALLOW_SHELL_ENV),
            parse_env_list(ALLOW_LAUNCH_ENV),
        )
    }

    #[must_use]
    pub const fn allow_shell_count(&self) -> usize {
        self.allow_shell.len()
    }

    #[must_use]
    pub const fn allow_launch_count(&self) -> usize {
        self.allow_launch.len()
    }

    fn shell_match<'a>(&'a self, command_line: &str) -> Option<&'a str> {
        self.allow_shell
            .iter()
            .find(|pattern| pattern.regex.is_match(command_line))
            .map(|pattern| pattern.raw.as_str())
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActComboParams {
    #[schemars(length(min = 1, max = 256))]
    pub steps: Vec<ActComboStep>,
    #[serde(default = "default_backend")]
    #[schemars(default = "default_backend")]
    pub backend: Backend,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActComboStep {
    pub at_ms: u32,
    pub action: ActComboAction,
    pub params: serde_json::Value,
    pub backend: Option<Backend>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActComboAction {
    ActClick,
    ActType,
    ActPress,
    ActAim,
    ActDrag,
    ActScroll,
    ActPad,
    ActClipboard,
    ReleaseAll,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActComboResponse {
    pub combo_id: String,
    pub scheduled_steps: u32,
    pub backend: Backend,
    pub started_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActRunShellParams {
    pub command: String,
    #[serde(default)]
    #[schemars(default)]
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    #[serde(default)]
    #[schemars(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default = "default_shell_timeout_ms")]
    #[schemars(default = "default_shell_timeout_ms", range(min = 1, max = 600_000))]
    pub timeout_ms: u32,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActRunShellResponse {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u32,
    pub timed_out: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActLaunchParams {
    pub target: String,
    #[serde(default)]
    #[schemars(default)]
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    #[serde(default)]
    #[schemars(default)]
    pub env: BTreeMap<String, String>,
    pub wait_for_window_title_regex: Option<String>,
    #[serde(default = "default_launch_timeout_ms")]
    #[schemars(default = "default_launch_timeout_ms", range(min = 1, max = 600_000))]
    pub timeout_ms: u32,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActLaunchResponse {
    pub pid: u32,
    pub hwnd: Option<i64>,
    pub matched_title: Option<String>,
    pub launched_at: String,
}

pub async fn execute_combo(
    runtime: Arc<Mutex<ReflexRuntime>>,
    params: ActComboParams,
) -> Result<ActComboResponse, ErrorData> {
    validate_combo_params(&params)?;
    let idempotency_present = params.idempotency_key.is_some();
    let started = Instant::now();
    let steps = combo_steps_from_params(&params)?;
    let scheduled_steps = u32::try_from(steps.len()).map_err(|_err| {
        mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_combo expanded steps length exceeds u32::MAX",
        )
    })?;
    let combo_id = new_reflex_id();
    let reflex = ScheduledReflex::combo(combo_id.clone(), ComboParams::new(steps, params.backend));
    let state = runtime
        .lock()
        .map_err(|_err| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned",
            )
        })?
        .register(&reflex)
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    tracing::info!(
        code = "M4_ACT_COMBO_EXECUTED",
        combo_id = %combo_id,
        idempotency_present,
        scheduled_steps,
        backend = ?params.backend,
        state = ?state.state,
        "readback=act_combo after=reflex_runtime_register"
    );
    Ok(ActComboResponse {
        combo_id,
        scheduled_steps,
        backend: params.backend,
        started_at_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

pub fn required_combo_permissions(
    params: &ActComboParams,
) -> Result<RequiredPermissions, ErrorData> {
    validate_combo_params(params)?;
    let action = Action::Combo {
        steps: combo_steps_from_params(params)?,
        backend: params.backend,
    };
    let mut required = RequiredPermissions::new();
    add_action_permissions(&action, &mut required);
    Ok(required)
}

pub async fn run_shell(
    config: &M4ServiceConfig,
    params: ActRunShellParams,
) -> Result<ActRunShellResponse, ErrorData> {
    validate_run_shell_params(&params)?;
    let command_line = shell_command_line(&params);
    let Some(matched_pattern) = config.shell_match(&command_line) else {
        let reason = if config.allow_shell_count() == 0 {
            "no_allow_shell_policy"
        } else {
            "shell_command_not_allowlisted"
        };
        return Err(policy_error(
            error_codes::SAFETY_SHELL_DENIED_BY_POLICY,
            "act_run_shell command is not permitted by --allow-shell policy",
            json!({
                "code": error_codes::SAFETY_SHELL_DENIED_BY_POLICY,
                "command": params.command,
                "args": params.args,
                "command_line": command_line,
                "working_dir": params.working_dir,
                "env_keys": params.env.keys().cloned().collect::<Vec<_>>(),
                "timeout_ms": params.timeout_ms,
                "idempotency_key_present": params.idempotency_key.is_some(),
                "allow_shell_patterns": config.allow_shell_count(),
                "reason": reason,
            }),
        ));
    };
    let matched_pattern = matched_pattern.to_owned();
    let idempotency_present = params.idempotency_key.is_some();
    let result = run_allowlisted_shell(params).await?;
    tracing::info!(
        code = "M4_ACT_RUN_SHELL_EXECUTED",
        command_line = %command_line,
        matched_pattern = %matched_pattern,
        exit_code = ?result.exit_code,
        duration_ms = result.duration_ms,
        timed_out = result.timed_out,
        stdout_truncated = result.stdout_truncated,
        stderr_truncated = result.stderr_truncated,
        idempotency_present,
        "readback=act_run_shell after=process_complete"
    );
    Ok(result)
}

pub async fn launch(params: ActLaunchParams) -> Result<ActLaunchResponse, ErrorData> {
    validate_launch_params(&params)?;
    Err(policy_error(
        error_codes::SAFETY_LAUNCH_DENIED_BY_POLICY,
        "act_launch is disabled until --allow-launch policy is configured",
        json!({
            "code": error_codes::SAFETY_LAUNCH_DENIED_BY_POLICY,
            "target": params.target,
            "args": params.args,
            "working_dir": params.working_dir,
            "env_keys": params.env.keys().cloned().collect::<Vec<_>>(),
            "timeout_ms": params.timeout_ms,
            "idempotency_key_present": params.idempotency_key.is_some(),
            "reason": "no_allow_launch_policy",
        }),
    ))
}

fn validate_combo_params(params: &ActComboParams) -> Result<(), ErrorData> {
    if params.steps.is_empty() {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_combo steps must contain at least one step",
        ));
    }
    if params.steps.len() > MAX_COMBO_STEPS {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!(
                "act_combo steps length {} exceeds max {MAX_COMBO_STEPS}",
                params.steps.len()
            ),
        ));
    }
    let mut previous = 0;
    for (index, step) in params.steps.iter().enumerate() {
        if index > 0 && step.at_ms < previous {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!("act_combo steps[{index}].at_ms must be monotonic"),
            ));
        }
        if let Some(step_backend) = step.backend {
            ensure_combo_step_backend_matches(index, "backend", step_backend, params.backend)?;
        }
        previous = step.at_ms;
    }
    Ok(())
}

fn combo_steps_from_params(params: &ActComboParams) -> Result<Vec<ComboStep>, ErrorData> {
    let mut out = Vec::new();
    for (index, step) in params.steps.iter().enumerate() {
        match step.action {
            ActComboAction::ActPress => {
                let press: ActPressParams =
                    serde_json::from_value(step.params.clone()).map_err(|error| {
                        mcp_error(
                            error_codes::TOOL_PARAMS_INVALID,
                            format!("act_combo steps[{index}].act_press params invalid: {error}"),
                        )
                    })?;
                push_press_combo_steps(&mut out, index, step.at_ms, &press, params.backend)?;
            }
            other => {
                return Err(mcp_error(
                    error_codes::TOOL_PARAMS_INVALID,
                    format!(
                        "act_combo steps[{index}].action {other:?} is not yet combo-lowerable; supported action: act_press"
                    ),
                ));
            }
        }
    }
    Ok(out)
}

fn push_press_combo_steps(
    out: &mut Vec<ComboStep>,
    index: usize,
    at_ms: u32,
    params: &ActPressParams,
    combo_backend: Backend,
) -> Result<(), ErrorData> {
    match action_from_press_params(params)? {
        Action::KeyPress {
            key,
            hold_ms,
            backend,
        } => {
            ensure_combo_step_backend_matches(
                index,
                "act_press params.backend",
                backend,
                combo_backend,
            )?;
            let hold_ms = u16::try_from(hold_ms).map_err(|_err| {
                mcp_error(
                    error_codes::TOOL_PARAMS_INVALID,
                    format!("act_combo steps[{index}].act_press hold_ms exceeds u16::MAX"),
                )
            })?;
            out.push(ComboStep {
                at_ms,
                input: ComboInput::KeyPress { key, hold_ms },
            });
        }
        Action::KeyChord {
            keys,
            hold_ms,
            backend,
        } => {
            ensure_combo_step_backend_matches(
                index,
                "act_press params.backend",
                backend,
                combo_backend,
            )?;
            push_key_chord_combo_steps(out, at_ms, keys, hold_ms);
        }
        other => {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!("act_combo steps[{index}].act_press lowered to unsupported {other:?}"),
            ));
        }
    }
    Ok(())
}

fn ensure_combo_step_backend_matches(
    index: usize,
    field: &'static str,
    requested: Backend,
    combo_backend: Backend,
) -> Result<(), ErrorData> {
    if requested == Backend::Auto || requested == combo_backend {
        return Ok(());
    }
    Err(mcp_error(
        error_codes::TOOL_PARAMS_INVALID,
        format!(
            "act_combo steps[{index}].{field} differs from top-level backend; per-step backend routing is not lowerable yet"
        ),
    ))
}

fn push_key_chord_combo_steps(out: &mut Vec<ComboStep>, at_ms: u32, keys: Vec<Key>, hold_ms: u32) {
    for key in &keys {
        out.push(ComboStep {
            at_ms,
            input: ComboInput::KeyDown { key: key.clone() },
        });
    }
    let release_at_ms = at_ms.saturating_add(hold_ms);
    for key in keys.into_iter().rev() {
        out.push(ComboStep {
            at_ms: release_at_ms,
            input: ComboInput::KeyUp { key },
        });
    }
}

fn validate_run_shell_params(params: &ActRunShellParams) -> Result<(), ErrorData> {
    if params.command.trim().is_empty() {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_run_shell command must not be empty",
        ));
    }
    Ok(())
}

fn validate_launch_params(params: &ActLaunchParams) -> Result<(), ErrorData> {
    if params.target.trim().is_empty() {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_launch target must not be empty",
        ));
    }
    if let Some(pattern) = &params.wait_for_window_title_regex {
        regex::Regex::new(pattern).map_err(|error| {
            mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!("act_launch wait_for_window_title_regex is invalid: {error}"),
            )
        })?;
    }
    Ok(())
}

async fn run_allowlisted_shell(
    params: ActRunShellParams,
) -> Result<ActRunShellResponse, ErrorData> {
    let started = Instant::now();
    let mut child = spawn_shell_child(&params)?;
    let (stdout_task, stderr_task) = spawn_capped_readers(&mut child)?;
    let (exit_code, timed_out) = wait_shell_child(&mut child, params.timeout_ms).await?;
    let stdout = join_capped_stream(stdout_task, "stdout").await?;
    let stderr = join_capped_stream(stderr_task, "stderr").await?;
    Ok(ActRunShellResponse {
        exit_code,
        stdout: String::from_utf8_lossy(&stdout.bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
        duration_ms: elapsed_ms_u32(started),
        timed_out,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
    })
}

fn spawn_shell_child(params: &ActRunShellParams) -> Result<tokio::process::Child, ErrorData> {
    let mut command = Command::new(&params.command);
    command.args(&params.args);
    if let Some(working_dir) = &params.working_dir {
        command.current_dir(working_dir);
    }
    command.env_clear();
    for key in SHELL_BASE_ENV_KEYS {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.envs(&params.env);
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    command.spawn().map_err(|error| {
        shell_tool_error(
            error_codes::ACTION_TARGET_INVALID,
            format!("act_run_shell failed to spawn command: {error}"),
            json!({
                "code": error_codes::ACTION_TARGET_INVALID,
                "command": params.command,
                "args": params.args,
                "working_dir": params.working_dir,
                "reason": "spawn_failed",
            }),
        )
    })
}

type CappedStreamTask = tokio::task::JoinHandle<io::Result<CappedOutput>>;

fn spawn_capped_readers(
    child: &mut tokio::process::Child,
) -> Result<(CappedStreamTask, CappedStreamTask), ErrorData> {
    let stdout = child.stdout.take().ok_or_else(|| {
        shell_tool_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "act_run_shell stdout pipe missing after spawn",
            json!({
                "code": error_codes::TOOL_INTERNAL_ERROR,
                "reason": "stdout_pipe_missing",
            }),
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        shell_tool_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "act_run_shell stderr pipe missing after spawn",
            json!({
                "code": error_codes::TOOL_INTERNAL_ERROR,
                "reason": "stderr_pipe_missing",
            }),
        )
    })?;
    let stdout_task = tokio::spawn(read_capped_stream(stdout));
    let stderr_task = tokio::spawn(read_capped_stream(stderr));
    Ok((stdout_task, stderr_task))
}

async fn wait_shell_child(
    child: &mut tokio::process::Child,
    timeout_ms: u32,
) -> Result<(Option<i32>, bool), ErrorData> {
    let wait_result =
        tokio::time::timeout(Duration::from_millis(u64::from(timeout_ms)), child.wait()).await;
    let result = match wait_result {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(error)) => {
            return Err(shell_tool_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("act_run_shell failed while waiting for command: {error}"),
                json!({
                    "code": error_codes::TOOL_INTERNAL_ERROR,
                    "reason": "wait_failed",
                }),
            ));
        }
        Err(_elapsed) => {
            if let Err(error) = child.start_kill() {
                tracing::warn!(
                    code = "M4_ACT_RUN_SHELL_KILL_FAILED",
                    error = %error,
                    "act_run_shell timeout kill request failed"
                );
            }
            let _status = child.wait().await.map_err(|error| {
                shell_tool_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    format!("act_run_shell failed while waiting after timeout kill: {error}"),
                    json!({
                        "code": error_codes::TOOL_INTERNAL_ERROR,
                        "reason": "wait_after_timeout_failed",
                    }),
                )
            })?;
            (None, true)
        }
    };
    Ok(result)
}

#[derive(Debug)]
struct CappedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn read_capped_stream<R>(mut reader: R) -> io::Result<CappedOutput>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let remaining = SHELL_OUTPUT_CAP_BYTES.saturating_sub(bytes.len());
        let keep = read.min(remaining);
        if keep > 0 {
            bytes.extend_from_slice(&buffer[..keep]);
        }
        if keep < read {
            truncated = true;
        }
    }
    Ok(CappedOutput { bytes, truncated })
}

async fn join_capped_stream(
    task: CappedStreamTask,
    stream_name: &'static str,
) -> Result<CappedOutput, ErrorData> {
    task.await
        .map_err(|error| {
            shell_tool_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("act_run_shell {stream_name} reader task failed: {error}"),
                json!({
                    "code": error_codes::TOOL_INTERNAL_ERROR,
                    "stream": stream_name,
                    "reason": "stream_join_failed",
                }),
            )
        })?
        .map_err(|error| {
            shell_tool_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("act_run_shell {stream_name} read failed: {error}"),
                json!({
                    "code": error_codes::TOOL_INTERNAL_ERROR,
                    "stream": stream_name,
                    "reason": "stream_read_failed",
                }),
            )
        })
}

fn shell_command_line(params: &ActRunShellParams) -> String {
    std::iter::once(&params.command)
        .chain(params.args.iter())
        .map(|part| quote_command_part(part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_command_part(part: &str) -> String {
    if part.is_empty() {
        return "\"\"".to_owned();
    }
    if !part.chars().any(|ch| ch.is_whitespace() || ch == '"') {
        return part.to_owned();
    }
    let escaped = part.replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn elapsed_ms_u32(started: Instant) -> u32 {
    u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX)
}

fn policy_error(code: &'static str, message: &'static str, data: serde_json::Value) -> ErrorData {
    tracing::warn!(code, "M4 policy gate denied tool invocation: {message}");
    ErrorData::new(ErrorCode(-32099), message, Some(data))
}

fn shell_tool_error(
    code: &'static str,
    message: impl Into<String>,
    data: serde_json::Value,
) -> ErrorData {
    let message = message.into();
    tracing::warn!(code, "M4 shell tool error: {message}");
    ErrorData::new(ErrorCode(-32099), message, Some(data))
}

const fn default_backend() -> Backend {
    Backend::Auto
}

const fn default_shell_timeout_ms() -> u32 {
    DEFAULT_SHELL_TIMEOUT_MS
}

const fn default_launch_timeout_ms() -> u32 {
    DEFAULT_LAUNCH_TIMEOUT_MS
}

fn parse_env_list(name: &str) -> Vec<String> {
    std::env::var(name)
        .map(|raw| raw.split(',').map(ToOwned::to_owned).collect())
        .unwrap_or_default()
}

fn compile_allow_patterns(
    source_name: &'static str,
    patterns: Vec<String>,
) -> anyhow::Result<Vec<AllowPattern>> {
    patterns
        .into_iter()
        .map(|raw| {
            let regex = regex::Regex::new(&raw)
                .with_context(|| format!("{source_name} pattern {raw:?} is not valid regex"))?;
            Ok(AllowPattern { raw, regex })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_config_for(params: &ActRunShellParams) -> M4ServiceConfig {
        match M4ServiceConfig::from_cli_parts(
            vec![format!("^{}$", regex::escape(&shell_command_line(params)))],
            Vec::new(),
        ) {
            Ok(config) => config,
            Err(error) => panic!("synthetic shell allowlist should compile: {error:#}"),
        }
    }

    fn shell_params(command: &str, args: Vec<&str>, timeout_ms: u32) -> ActRunShellParams {
        ActRunShellParams {
            command: command.to_owned(),
            args: args.into_iter().map(str::to_owned).collect(),
            working_dir: None,
            env: BTreeMap::new(),
            timeout_ms,
            idempotency_key: None,
        }
    }

    #[test]
    fn combo_rejects_nested_press_backend_mismatch() {
        let params = ActComboParams {
            steps: vec![ActComboStep {
                at_ms: 0,
                action: ActComboAction::ActPress,
                params: json!({
                    "keys": ["f17"],
                    "hold_ms": 5,
                    "backend": "hardware"
                }),
                backend: None,
            }],
            backend: Backend::Software,
            idempotency_key: None,
        };

        let error = match combo_steps_from_params(&params) {
            Ok(steps) => panic!("mismatched backend should reject, got {steps:?}"),
            Err(error) => error,
        };

        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("code"))
                .and_then(|code| code.as_str()),
            Some(error_codes::TOOL_PARAMS_INVALID)
        );
        assert!(
            error
                .message
                .contains("act_press params.backend differs from top-level backend")
        );
    }

    #[test]
    fn combo_allows_nested_press_auto_backend_to_use_top_level_backend() {
        let params = ActComboParams {
            steps: vec![ActComboStep {
                at_ms: 0,
                action: ActComboAction::ActPress,
                params: json!({
                    "keys": ["f18"],
                    "hold_ms": 5,
                    "backend": "auto"
                }),
                backend: None,
            }],
            backend: Backend::Software,
            idempotency_key: None,
        };

        let steps = match combo_steps_from_params(&params) {
            Ok(steps) => steps,
            Err(error) => panic!("auto backend should lower: {error}"),
        };

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].at_ms, 0);
    }

    #[test]
    fn shell_command_line_quotes_empty_and_space_args() {
        let params = shell_params("cmd.exe", vec!["/c", "echo hello", ""], 30_000);

        assert_eq!(
            shell_command_line(&params),
            "cmd.exe /c \"echo hello\" \"\""
        );
    }

    #[tokio::test]
    async fn shell_denies_without_allowlist() {
        let params = shell_params("synthetic-shell-denied", Vec::new(), 30_000);

        let error = match run_shell(&M4ServiceConfig::default(), params).await {
            Ok(response) => panic!("unallowlisted shell should deny, got {response:?}"),
            Err(error) => error,
        };

        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("code"))
                .and_then(|code| code.as_str()),
            Some(error_codes::SAFETY_SHELL_DENIED_BY_POLICY)
        );
        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("reason"))
                .and_then(|reason| reason.as_str()),
            Some("no_allow_shell_policy")
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn shell_allows_cmd_echo_and_captures_stdout() {
        let params = shell_params("cmd.exe", vec!["/c", "echo synapse-m4-shell-ok"], 30_000);
        let config = shell_config_for(&params);

        let response = match run_shell(&config, params).await {
            Ok(response) => response,
            Err(error) => panic!("allowlisted cmd echo should run: {error}"),
        };

        assert_eq!(response.exit_code, Some(0));
        assert_eq!(response.stdout, "synapse-m4-shell-ok\r\n");
        assert_eq!(response.stderr, "");
        assert!(!response.timed_out);
        assert!(!response.stdout_truncated);
        assert!(!response.stderr_truncated);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn shell_caps_stdout_and_marks_truncated() {
        let params = shell_params(
            "powershell.exe",
            vec![
                "-NoProfile",
                "-Command",
                "[Console]::Out.Write(('x'*1048580))",
            ],
            30_000,
        );
        let config = shell_config_for(&params);

        let response = match run_shell(&config, params).await {
            Ok(response) => response,
            Err(error) => panic!("allowlisted large stdout command should run: {error}"),
        };

        assert_eq!(response.exit_code, Some(0));
        assert_eq!(response.stdout.len(), SHELL_OUTPUT_CAP_BYTES);
        assert!(response.stdout.chars().all(|ch| ch == 'x'));
        assert!(response.stdout_truncated);
        assert_eq!(response.stderr, "");
        assert!(!response.stderr_truncated);
        assert!(!response.timed_out);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn shell_timeout_kills_process_and_marks_timed_out() {
        let params = shell_params(
            "powershell.exe",
            vec!["-NoProfile", "-Command", "Start-Sleep -Milliseconds 5000"],
            500,
        );
        let config = shell_config_for(&params);

        let response = match run_shell(&config, params).await {
            Ok(response) => response,
            Err(error) => {
                panic!("allowlisted sleeping command should return timeout response: {error}")
            }
        };

        assert_eq!(response.exit_code, None);
        assert!(response.timed_out);
        assert!(response.duration_ms < 2_000, "{response:?}");
    }
}
