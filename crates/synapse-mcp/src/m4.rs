use std::{
    collections::{BTreeMap, HashSet},
    io,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Context;
use rmcp::{ErrorData, model::ErrorCode, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use synapse_core::{
    Action, Backend, ComboInput, ComboStep, ForegroundContext, Key, error_codes, new_reflex_id,
};
use synapse_reflex::{ComboParams, ReflexRuntime, ScheduledReflex};
use synapse_storage::{decode_json, encode_json};
use tokio::{io::AsyncReadExt, process::Command};

use crate::{
    m1::mcp_error,
    m2::{ActPressParams, action_from_press_params},
    m3::permissions::{RequiredPermissions, add_action_permissions},
};

const MAX_COMBO_STEPS: usize = 256;
const DEFAULT_SHELL_TIMEOUT_MS: u32 = 30_000;
const MAX_SHELL_TIMEOUT_MS: u32 = 600_000;
const DEFAULT_LAUNCH_TIMEOUT_MS: u32 = 10_000;
const MAX_SHELL_IDEMPOTENCY_KEY_BYTES: usize = 256;
const ALLOW_SHELL_ENV: &str = "SYNAPSE_ALLOW_SHELL";
const ALLOW_LAUNCH_ENV: &str = "SYNAPSE_ALLOW_LAUNCH";
/// Unrestricted shell/launch. **On by default**: Synapse is general local
/// computer-control, so any command/target is permitted unless the operator
/// explicitly sets the env to a falsey value (`0`/`false`/`no`/`off`), which
/// restores the per-target allowlist. Every command/target is recorded in
/// `CF_ACTION_LOG` regardless, and the mode is logged loudly at startup.
const ALLOW_SHELL_ANY_ENV: &str = "SYNAPSE_ALLOW_SHELL_ANY";
const ALLOW_LAUNCH_ANY_ENV: &str = "SYNAPSE_ALLOW_LAUNCH_ANY";
/// Sentinel recorded as the matched pattern when permissive mode authorizes a
/// command/target without an allowlist entry.
const ANY_PERMITTED_SENTINEL: &str = "__any_permitted__";
const SHELL_OUTPUT_CAP_BYTES: usize = 1024 * 1024;
const ALLOW_PATTERN_SIZE_LIMIT_BYTES: usize = 256 * 1024;
const PROCESS_BASE_ENV_KEYS: [&str; 4] = ["PATH", "USERPROFILE", "TEMP", "SystemRoot"];
const LAUNCH_WINDOW_POLL_INTERVAL_MS: u64 = 20;
const RUN_SHELL_IDEMPOTENCY_PREFIX: &str = "m4/act_run_shell/idempotency/v1/";
pub const SHELL_PATTERN_TOO_BROAD: &str = "SHELL_PATTERN_TOO_BROAD";
pub const LAUNCH_PATTERN_TOO_BROAD: &str = "LAUNCH_PATTERN_TOO_BROAD";

// All fields are allowlist policy for the two gated tools; the shared `allow_`
// prefix is intentional and reads clearly at call sites.
#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Default)]
pub struct M4ServiceConfig {
    allow_shell: Vec<AllowPattern>,
    allow_launch: Vec<AllowPattern>,
    allow_shell_any: bool,
    allow_launch_any: bool,
}

#[derive(Clone, Debug)]
struct AllowPattern {
    raw: String,
    regex: regex::Regex,
}

#[derive(Debug)]
pub struct BroadAllowPatternError {
    source_name: &'static str,
    tool_name: &'static str,
    code: &'static str,
    raw: String,
    reason: &'static str,
}

impl BroadAllowPatternError {
    #[must_use]
    pub const fn source_name(&self) -> &'static str {
        self.source_name
    }

    #[must_use]
    pub const fn tool_name(&self) -> &'static str {
        self.tool_name
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }

    #[must_use]
    pub fn raw(&self) -> &str {
        &self.raw
    }

    #[must_use]
    pub const fn reason(&self) -> &'static str {
        self.reason
    }
}

impl std::fmt::Display for BroadAllowPatternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} pattern {:?} is too broad for {}: {}",
            self.source_name, self.raw, self.tool_name, self.reason
        )
    }
}

impl std::error::Error for BroadAllowPatternError {}

impl M4ServiceConfig {
    pub fn from_cli_parts(
        allow_shell: Vec<String>,
        allow_launch: Vec<String>,
    ) -> anyhow::Result<Self> {
        let allow_shell_any = env_flag_default_true(ALLOW_SHELL_ANY_ENV);
        let allow_launch_any = env_flag_default_true(ALLOW_LAUNCH_ANY_ENV);
        if allow_shell_any {
            tracing::warn!(
                code = "M4_ALLOW_SHELL_ANY_ENABLED",
                env = ALLOW_SHELL_ANY_ENV,
                "act_run_shell permissive mode enabled: ALL shell commands are allowed (allowlist bypassed); every command is still recorded in CF_ACTION_LOG"
            );
        }
        if allow_launch_any {
            tracing::warn!(
                code = "M4_ALLOW_LAUNCH_ANY_ENABLED",
                env = ALLOW_LAUNCH_ANY_ENV,
                "act_launch permissive mode enabled: ALL launch targets are allowed (allowlist bypassed); every launch is still recorded in CF_ACTION_LOG"
            );
        }
        Ok(Self {
            allow_shell: compile_allow_patterns(
                ALLOW_SHELL_ENV,
                allow_shell,
                AllowPatternPolicy::Shell,
            )?,
            allow_launch: compile_allow_patterns(
                ALLOW_LAUNCH_ENV,
                allow_launch,
                AllowPatternPolicy::Launch,
            )?,
            allow_shell_any,
            allow_launch_any,
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

    #[must_use]
    pub const fn allow_shell_any(&self) -> bool {
        self.allow_shell_any
    }

    #[must_use]
    pub const fn allow_launch_any(&self) -> bool {
        self.allow_launch_any
    }

    fn shell_match<'a>(&'a self, command_line: &str) -> Option<&'a str> {
        if self.allow_shell_any {
            return Some(ANY_PERMITTED_SENTINEL);
        }
        self.allow_shell
            .iter()
            .find(|pattern| pattern.regex.is_match(command_line))
            .map(|pattern| pattern.raw.as_str())
    }

    fn launch_match<'a>(&'a self, command_line: &str) -> Option<&'a str> {
        if self.allow_launch_any {
            return Some(ANY_PERMITTED_SENTINEL);
        }
        self.allow_launch
            .iter()
            .find(|pattern| pattern.regex.is_match(command_line))
            .map(|pattern| pattern.raw.as_str())
    }
}

/// Returns `true` unless the env var is explicitly set to a falsey value
/// (`0`/`false`/`no`/`off`). An unset variable means `true`, i.e.
/// permissive-by-default for shell/launch.
fn env_flag_default_true(name: &str) -> bool {
    std::env::var(name).map_or(true, |value| {
        !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
    })
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

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
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

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
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

#[derive(Clone, Debug)]
pub struct RunShellAuthorization {
    command_line: String,
    matched_pattern: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RunShellIdempotencyRow {
    schema_version: u32,
    tool: String,
    idempotency_key_sha256: String,
    request_sha256: String,
    status: String,
    command_line: String,
    matched_pattern: String,
    started_at: String,
    completed_at: Option<String>,
    response: Option<ActRunShellResponse>,
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
    pub reason: Option<String>,
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

#[allow(
    dead_code,
    reason = "kept as the direct M4 execution helper for unit tests and non-server callers"
)]
pub async fn run_shell(
    config: &M4ServiceConfig,
    params: ActRunShellParams,
) -> Result<ActRunShellResponse, ErrorData> {
    let authorization = authorize_run_shell(config, &params)?;
    run_authorized_shell(params, &authorization).await
}

pub fn authorize_run_shell(
    config: &M4ServiceConfig,
    params: &ActRunShellParams,
) -> Result<RunShellAuthorization, ErrorData> {
    validate_run_shell_params(params)?;
    let command_line = shell_command_line(params);
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
    Ok(RunShellAuthorization {
        command_line,
        matched_pattern: matched_pattern.to_owned(),
    })
}

pub async fn run_authorized_shell(
    params: ActRunShellParams,
    authorization: &RunShellAuthorization,
) -> Result<ActRunShellResponse, ErrorData> {
    let idempotency_present = params.idempotency_key.is_some();
    let result = run_allowlisted_shell(params).await?;
    tracing::info!(
        code = "M4_ACT_RUN_SHELL_EXECUTED",
        command_line = %authorization.command_line,
        matched_pattern = %authorization.matched_pattern,
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

pub fn run_shell_request_details(params: &ActRunShellParams) -> serde_json::Value {
    json!({
        "command": params.command,
        "args": params.args,
        "working_dir": params.working_dir,
        "env_keys": params.env.keys().cloned().collect::<Vec<_>>(),
        "timeout_ms": params.timeout_ms,
        "idempotency_key_present": params.idempotency_key.is_some(),
        "request_sha256": run_shell_request_sha256(params).ok(),
    })
}

pub fn run_shell_idempotency_row_key(
    params: &ActRunShellParams,
) -> Result<Option<Vec<u8>>, ErrorData> {
    let Some(key) = &params.idempotency_key else {
        return Ok(None);
    };
    validate_run_shell_idempotency_key(key)?;
    Ok(Some(
        format!(
            "{RUN_SHELL_IDEMPOTENCY_PREFIX}{}",
            sha256_hex(key.as_bytes())
        )
        .into_bytes(),
    ))
}

pub fn run_shell_idempotency_reservation_row(
    params: &ActRunShellParams,
    authorization: &RunShellAuthorization,
) -> Result<Vec<u8>, ErrorData> {
    let key_sha256 = params
        .idempotency_key
        .as_deref()
        .map(|key| sha256_hex(key.as_bytes()))
        .unwrap_or_default();
    let row = RunShellIdempotencyRow {
        schema_version: 1,
        tool: "act_run_shell".to_owned(),
        idempotency_key_sha256: key_sha256,
        request_sha256: run_shell_request_sha256(params)?,
        status: "in_progress".to_owned(),
        command_line: authorization.command_line.clone(),
        matched_pattern: authorization.matched_pattern.clone(),
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        response: None,
    };
    encode_json(&row).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("act_run_shell idempotency reservation encode failed: {error}"),
        )
    })
}

pub fn run_shell_idempotency_completed_row(
    params: &ActRunShellParams,
    authorization: &RunShellAuthorization,
    response: &ActRunShellResponse,
) -> Result<Vec<u8>, ErrorData> {
    let key_sha256 = params
        .idempotency_key
        .as_deref()
        .map(|key| sha256_hex(key.as_bytes()))
        .unwrap_or_default();
    let now = chrono::Utc::now().to_rfc3339();
    let row = RunShellIdempotencyRow {
        schema_version: 1,
        tool: "act_run_shell".to_owned(),
        idempotency_key_sha256: key_sha256,
        request_sha256: run_shell_request_sha256(params)?,
        status: "ok".to_owned(),
        command_line: authorization.command_line.clone(),
        matched_pattern: authorization.matched_pattern.clone(),
        started_at: now.clone(),
        completed_at: Some(now),
        response: Some(response.clone()),
    };
    encode_json(&row).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("act_run_shell idempotency completion encode failed: {error}"),
        )
    })
}

pub fn run_shell_idempotency_replay(
    params: &ActRunShellParams,
    row_bytes: &[u8],
) -> Result<ActRunShellResponse, ErrorData> {
    let row = decode_json::<RunShellIdempotencyRow>(row_bytes).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("act_run_shell idempotency row decode failed: {error}"),
        )
    })?;
    if row.schema_version != 1 || row.tool != "act_run_shell" {
        return Err(mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "act_run_shell idempotency row has unexpected schema/tool",
        ));
    }
    let expected_key_sha256 = params
        .idempotency_key
        .as_deref()
        .map(|key| sha256_hex(key.as_bytes()))
        .unwrap_or_default();
    if row.idempotency_key_sha256 != expected_key_sha256 {
        return Err(mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "act_run_shell idempotency row key hash mismatch",
        ));
    }
    let request_sha256 = run_shell_request_sha256(params)?;
    if row.request_sha256 != request_sha256 {
        return Err(idempotency_error(
            "act_run_shell idempotency_key was already used for different parameters",
            "idempotency_key_conflict",
            &json!({
                "stored_request_sha256": row.request_sha256,
                "incoming_request_sha256": request_sha256,
            }),
        ));
    }
    match (row.status.as_str(), row.response) {
        ("ok", Some(response)) => {
            tracing::info!(
                code = "M4_ACT_RUN_SHELL_IDEMPOTENT_REPLAY",
                request_sha256 = %request_sha256,
                "readback=act_run_shell after=idempotent_replay"
            );
            Ok(response)
        }
        ("in_progress", _) => Err(idempotency_error(
            "act_run_shell idempotency_key is already in progress",
            "idempotency_in_progress",
            &json!({ "request_sha256": request_sha256 }),
        )),
        (status, _) => Err(mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("act_run_shell idempotency row has unsupported status {status:?}"),
        )),
    }
}

fn run_shell_request_sha256(params: &ActRunShellParams) -> Result<String, ErrorData> {
    let payload = json!({
        "command": params.command,
        "args": params.args,
        "working_dir": params.working_dir,
        "env": params.env,
        "timeout_ms": params.timeout_ms,
    });
    let bytes = serde_json::to_vec(&payload).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("act_run_shell request fingerprint encode failed: {error}"),
        )
    })?;
    Ok(sha256_hex(&bytes))
}

pub async fn launch(
    config: &M4ServiceConfig,
    params: ActLaunchParams,
) -> Result<ActLaunchResponse, ErrorData> {
    validate_launch_params(&params)?;
    let command_line = launch_command_line(&params)?;
    let Some(matched_pattern) = config.launch_match(&command_line) else {
        let reason = if config.allow_launch_count() == 0 {
            "no_allow_launch_policy"
        } else {
            "launch_command_not_allowlisted"
        };
        return Err(policy_error(
            error_codes::SAFETY_LAUNCH_DENIED_BY_POLICY,
            "act_launch target is not permitted by --allow-launch policy",
            json!({
                "code": error_codes::SAFETY_LAUNCH_DENIED_BY_POLICY,
                "target": params.target,
                "args": params.args,
                "command_line": command_line,
                "working_dir": params.working_dir,
                "env_keys": params.env.keys().cloned().collect::<Vec<_>>(),
                "timeout_ms": params.timeout_ms,
                "idempotency_key_present": params.idempotency_key.is_some(),
                "allow_launch_patterns": config.allow_launch_count(),
                "reason": reason,
            }),
        ));
    };
    let matched_pattern = matched_pattern.to_owned();
    let wait_regex = params
        .wait_for_window_title_regex
        .as_ref()
        .map(|pattern| {
            regex::Regex::new(pattern).map_err(|error| {
                mcp_error(
                    error_codes::TOOL_PARAMS_INVALID,
                    format!("act_launch wait_for_window_title_regex is invalid: {error}"),
                )
            })
        })
        .transpose()?;
    let excluded_hwnds = if wait_regex.is_some() {
        snapshot_visible_window_hwnds()
    } else {
        HashSet::new()
    };
    let child = spawn_launch_child(&params)?;
    let pid = child.id().ok_or_else(|| {
        launch_tool_error(
            error_codes::ACTION_TARGET_INVALID,
            "act_launch spawned process without a process id",
            json!({
                "code": error_codes::ACTION_TARGET_INVALID,
                "target": params.target,
                "args": params.args,
                "reason": "spawned_process_missing_pid",
            }),
        )
    })?;
    drop(child);

    let window = if let Some(regex) = wait_regex {
        wait_for_launch_window(pid, &regex, params.timeout_ms, &excluded_hwnds).await
    } else {
        WindowWaitResult::not_requested()
    };
    let launched_at = chrono::Utc::now().to_rfc3339();
    tracing::info!(
        code = "M4_ACT_LAUNCH_EXECUTED",
        command_line = %command_line,
        matched_pattern = %matched_pattern,
        pid,
        hwnd = ?window.hwnd,
        matched_title = ?window.matched_title,
        reason = ?window.reason,
        wait_requested = params.wait_for_window_title_regex.is_some(),
        idempotency_present = params.idempotency_key.is_some(),
        "readback=act_launch after=process_spawn"
    );
    Ok(ActLaunchResponse {
        pid,
        hwnd: window.hwnd,
        matched_title: window.matched_title,
        launched_at,
        reason: window.reason,
    })
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
    if params.timeout_ms == 0 || params.timeout_ms > MAX_SHELL_TIMEOUT_MS {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("act_run_shell timeout_ms must be 1..={MAX_SHELL_TIMEOUT_MS}"),
        ));
    }
    if let Some(key) = &params.idempotency_key {
        validate_run_shell_idempotency_key(key)?;
    }
    Ok(())
}

fn validate_run_shell_idempotency_key(key: &str) -> Result<(), ErrorData> {
    if key.trim().is_empty() {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_run_shell idempotency_key must not be empty",
        ));
    }
    if key.len() > MAX_SHELL_IDEMPOTENCY_KEY_BYTES {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!(
                "act_run_shell idempotency_key must be <= {MAX_SHELL_IDEMPOTENCY_KEY_BYTES} bytes"
            ),
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

fn spawn_launch_child(params: &ActLaunchParams) -> Result<tokio::process::Child, ErrorData> {
    let mut command = Command::new(&params.target);
    command.args(&params.args);
    if let Some(working_dir) = &params.working_dir {
        command.current_dir(working_dir);
    }
    command.env_clear();
    for key in PROCESS_BASE_ENV_KEYS {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.envs(&params.env);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    command.spawn().map_err(|error| {
        launch_tool_error(
            error_codes::ACTION_TARGET_INVALID,
            format!("act_launch failed to spawn target: {error}"),
            json!({
                "code": error_codes::ACTION_TARGET_INVALID,
                "target": params.target,
                "args": params.args,
                "working_dir": params.working_dir,
                "reason": "spawn_failed",
            }),
        )
    })
}

#[derive(Debug)]
struct WindowWaitResult {
    hwnd: Option<i64>,
    matched_title: Option<String>,
    reason: Option<String>,
}

impl WindowWaitResult {
    const fn not_requested() -> Self {
        Self {
            hwnd: None,
            matched_title: None,
            reason: None,
        }
    }

    fn matched(context: ForegroundContext) -> Self {
        Self {
            hwnd: Some(context.hwnd),
            matched_title: Some(context.window_title),
            reason: None,
        }
    }

    fn no_match() -> Self {
        Self {
            hwnd: None,
            matched_title: None,
            reason: Some("no_match_within_timeout".to_owned()),
        }
    }

    fn unavailable() -> Self {
        Self {
            hwnd: None,
            matched_title: None,
            reason: Some("window_readback_unavailable".to_owned()),
        }
    }
}

async fn wait_for_launch_window(
    pid: u32,
    title_regex: &regex::Regex,
    timeout_ms: u32,
    excluded_hwnds: &HashSet<i64>,
) -> WindowWaitResult {
    let started = Instant::now();
    let timeout = Duration::from_millis(u64::from(timeout_ms));
    let mut last_error: Option<String>;
    loop {
        match synapse_a11y::visible_top_level_window_contexts() {
            Ok(contexts) => {
                if let Some(context) =
                    select_launch_window(&contexts, pid, title_regex, excluded_hwnds)
                {
                    // Best-effort foreground so the launched window is
                    // immediately actionable/observable. Without this, a newly
                    // launched window can open behind the caller's foreground
                    // (Windows foreground-stealing prevention), leaving observe
                    // pointed at the previous window.
                    if let Err(error) = synapse_a11y::focus_window(context.hwnd) {
                        tracing::warn!(
                            code = "M4_ACT_LAUNCH_FOCUS_FAILED",
                            hwnd = context.hwnd,
                            error = %error,
                            "act_launch matched the launched window but could not foreground it"
                        );
                    }
                    return WindowWaitResult::matched(context.clone());
                }
                last_error = None;
            }
            Err(error) if error.code() == error_codes::A11Y_NOT_AVAILABLE => {
                tracing::warn!(
                    code = error.code(),
                    error = %error,
                    "act_launch window readback unavailable"
                );
                return WindowWaitResult::unavailable();
            }
            Err(error) => {
                last_error = Some(error.to_string());
            }
        }

        if started.elapsed() >= timeout {
            tracing::warn!(
                code = "M4_ACT_LAUNCH_WINDOW_WAIT_TIMEOUT",
                pid,
                title_regex = title_regex.as_str(),
                ?excluded_hwnds,
                last_error,
                "act_launch window title wait timed out"
            );
            return WindowWaitResult::no_match();
        }
        tokio::time::sleep(Duration::from_millis(LAUNCH_WINDOW_POLL_INTERVAL_MS)).await;
    }
}

fn select_launch_window<'a>(
    contexts: &'a [ForegroundContext],
    pid: u32,
    title_regex: &regex::Regex,
    excluded_hwnds: &HashSet<i64>,
) -> Option<&'a ForegroundContext> {
    contexts
        .iter()
        .find(|context| {
            context.pid == pid
                && !excluded_hwnds.contains(&context.hwnd)
                && title_regex.is_match(&context.window_title)
        })
        .or_else(|| {
            contexts.iter().find(|context| {
                !excluded_hwnds.contains(&context.hwnd)
                    && title_regex.is_match(&context.window_title)
            })
        })
}

fn snapshot_visible_window_hwnds() -> HashSet<i64> {
    match synapse_a11y::visible_top_level_window_contexts() {
        Ok(contexts) => contexts.into_iter().map(|context| context.hwnd).collect(),
        Err(error) => {
            tracing::warn!(
                code = error.code(),
                error = %error,
                "act_launch could not snapshot pre-existing windows"
            );
            HashSet::new()
        }
    }
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
    for key in PROCESS_BASE_ENV_KEYS {
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

fn launch_command_line(params: &ActLaunchParams) -> Result<String, ErrorData> {
    let target = resolve_launch_target_for_policy(&params.target)?;
    Ok(std::iter::once(&target)
        .chain(params.args.iter())
        .map(|part| quote_command_part(part))
        .collect::<Vec<_>>()
        .join(" "))
}

#[cfg(windows)]
fn resolve_launch_target_for_policy(target: &str) -> Result<String, ErrorData> {
    if !is_path_like_launch_target(target) {
        return Ok(target.to_owned());
    }

    win32_long_path_name(target).map_err(|error| {
        launch_tool_error(
            error_codes::ACTION_TARGET_INVALID,
            format!("act_launch target path could not be resolved with GetLongPathNameW: {error}"),
            json!({
                "code": error_codes::ACTION_TARGET_INVALID,
                "target": target,
                "reason": "launch_target_path_resolution_failed",
            }),
        )
    })
}

#[cfg(not(windows))]
fn resolve_launch_target_for_policy(target: &str) -> Result<String, ErrorData> {
    Ok(target.to_owned())
}

#[cfg(windows)]
fn is_path_like_launch_target(target: &str) -> bool {
    if target.contains("://") {
        return false;
    }
    target.contains('\\')
        || target.contains('/')
        || target
            .as_bytes()
            .get(1)
            .is_some_and(|second| *second == b':')
}

#[cfg(windows)]
fn win32_long_path_name(target: &str) -> anyhow::Result<String> {
    use std::{
        ffi::{OsStr, OsString},
        os::windows::ffi::{OsStrExt, OsStringExt},
    };
    use windows::{Win32::Storage::FileSystem::GetLongPathNameW, core::PCWSTR};

    let wide_target = OsStr::new(target)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    // SAFETY: `wide_target` is explicitly NUL-terminated and remains alive for the call.
    let required = unsafe { GetLongPathNameW(PCWSTR(wide_target.as_ptr()), None) };
    if required == 0 {
        return Err(anyhow::Error::new(windows::core::Error::from_thread()))
            .with_context(|| format!("resolve launch target {target:?}"));
    }

    let mut buffer = vec![0; required as usize + 1];
    // SAFETY: the buffer is writable for its full length and the input pointer is valid.
    let written = unsafe { GetLongPathNameW(PCWSTR(wide_target.as_ptr()), Some(&mut buffer)) };
    if written == 0 {
        return Err(anyhow::Error::new(windows::core::Error::from_thread()))
            .with_context(|| format!("resolve launch target {target:?}"));
    }
    if written as usize >= buffer.len() {
        anyhow::bail!(
            "GetLongPathNameW returned {} characters for a {} character buffer",
            written,
            buffer.len()
        );
    }

    buffer.truncate(written as usize);
    Ok(OsString::from_wide(&buffer).to_string_lossy().into_owned())
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

fn idempotency_error(
    message: &'static str,
    reason: &'static str,
    details: &serde_json::Value,
) -> ErrorData {
    let mut data = json!({
        "code": error_codes::TOOL_PARAMS_INVALID,
        "reason": reason,
    });
    if let (Some(target), Some(source)) = (data.as_object_mut(), details.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
    ErrorData::new(ErrorCode(-32099), message, Some(data))
}

fn launch_tool_error(
    code: &'static str,
    message: impl Into<String>,
    data: serde_json::Value,
) -> ErrorData {
    let message = message.into();
    tracing::warn!(code, "M4 launch tool error: {message}");
    ErrorData::new(ErrorCode(-32099), message, Some(data))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
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

#[derive(Copy, Clone)]
enum AllowPatternPolicy {
    Shell,
    Launch,
}

impl AllowPatternPolicy {
    const fn tool_name(self) -> &'static str {
        match self {
            Self::Shell => "act_run_shell",
            Self::Launch => "act_launch",
        }
    }

    const fn broad_code(self) -> &'static str {
        match self {
            Self::Shell => SHELL_PATTERN_TOO_BROAD,
            Self::Launch => LAUNCH_PATTERN_TOO_BROAD,
        }
    }

    const fn unanchored_reason(self) -> &'static str {
        match self {
            Self::Shell => "shell_pattern_must_match_full_command_line",
            Self::Launch => "launch_pattern_must_match_full_command_line",
        }
    }
}

fn compile_allow_patterns(
    source_name: &'static str,
    patterns: Vec<String>,
    policy: AllowPatternPolicy,
) -> anyhow::Result<Vec<AllowPattern>> {
    patterns
        .into_iter()
        .map(|raw| {
            validate_allow_pattern_source(source_name, &raw, policy)?;
            let regex = regex::RegexBuilder::new(&raw)
                .size_limit(ALLOW_PATTERN_SIZE_LIMIT_BYTES)
                .build()
                .with_context(|| {
                    format!("{source_name} pattern {raw:?} is not valid regex or exceeds the compiled-size limit")
                })?;
            validate_compiled_allow_pattern(source_name, &raw, &regex, policy)?;
            Ok(AllowPattern { raw, regex })
        })
        .collect()
}

fn validate_allow_pattern_source(
    source_name: &'static str,
    raw: &str,
    policy: AllowPatternPolicy,
) -> Result<(), BroadAllowPatternError> {
    if raw.trim().is_empty() {
        return Err(broad_allow_pattern(
            source_name,
            raw,
            "empty_pattern",
            policy,
        ));
    }
    if contains_unbounded_dot_repetition(raw) || contains_any_character_class_repetition(raw) {
        return Err(broad_allow_pattern(
            source_name,
            raw,
            "unbounded_any_character_repetition",
            policy,
        ));
    }
    if !has_full_command_anchors(raw) {
        return Err(broad_allow_pattern(
            source_name,
            raw,
            policy.unanchored_reason(),
            policy,
        ));
    }
    Ok(())
}

fn validate_compiled_allow_pattern(
    source_name: &'static str,
    raw: &str,
    regex: &regex::Regex,
    policy: AllowPatternPolicy,
) -> Result<(), BroadAllowPatternError> {
    if regex.is_match("") {
        return Err(broad_allow_pattern(
            source_name,
            raw,
            "matches_empty",
            policy,
        ));
    }
    if BROAD_COMMAND_PROBES
        .iter()
        .all(|probe| regex.is_match(probe))
    {
        return Err(broad_allow_pattern(
            source_name,
            raw,
            "matches_arbitrary_command_lines",
            policy,
        ));
    }
    Ok(())
}

const BROAD_COMMAND_PROBES: [&str; 4] = [
    "cmd.exe /c \"echo synapse-broad-probe\"",
    "powershell.exe -NoProfile -Command Get-Process",
    "notepad.exe",
    "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe -EncodedCommand AAAA",
];

fn broad_allow_pattern(
    source_name: &'static str,
    raw: &str,
    reason: &'static str,
    policy: AllowPatternPolicy,
) -> BroadAllowPatternError {
    BroadAllowPatternError {
        source_name,
        tool_name: policy.tool_name(),
        code: policy.broad_code(),
        raw: raw.to_owned(),
        reason,
    }
}

fn has_full_command_anchors(raw: &str) -> bool {
    let pattern = strip_leading_global_flags(raw.trim());
    pattern.starts_with('^') && (pattern.ends_with('$') || pattern.ends_with("\\z"))
}

fn strip_leading_global_flags(pattern: &str) -> &str {
    let Some(rest) = pattern.strip_prefix("(?") else {
        return pattern;
    };
    let Some(close_index) = rest.find(')') else {
        return pattern;
    };
    let flags = &rest[..close_index];
    if flags.is_empty()
        || flags
            .chars()
            .any(|ch| !matches!(ch, 'i' | 'm' | 's' | 'R' | 'U' | 'u' | 'x' | '-'))
    {
        return pattern;
    }
    &rest[(close_index + 1)..]
}

fn contains_unbounded_dot_repetition(pattern: &str) -> bool {
    let mut escaped = false;
    let mut in_class = false;
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '.' if !in_class => {
                let Some(next) = chars.peek().copied() else {
                    continue;
                };
                if matches!(next, '*' | '+') {
                    return true;
                }
                if next == '{' && following_counted_repetition_is_unbounded(chars.clone()) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn following_counted_repetition_is_unbounded<I>(mut chars: I) -> bool
where
    I: Iterator<Item = char>,
{
    if chars.next() != Some('{') {
        return false;
    }
    let mut body = String::new();
    for ch in chars {
        if ch == '}' {
            return body.trim_end().ends_with(',');
        }
        body.push(ch);
    }
    false
}

fn contains_any_character_class_repetition(pattern: &str) -> bool {
    let compact = pattern
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    [
        r"[\s\S]*", r"[\S\s]*", r"[\d\D]*", r"[\D\d]*", r"[\w\W]*", r"[\W\w]*", r"[\s\S]+",
        r"[\S\s]+", r"[\d\D]+", r"[\D\d]+", r"[\w\W]+", r"[\W\w]+",
    ]
    .iter()
    .any(|needle| compact.contains(needle))
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

    fn launch_config_for(params: &ActLaunchParams) -> M4ServiceConfig {
        let command_line = launch_command_line(params)
            .unwrap_or_else(|error| panic!("synthetic launch command line should build: {error}"));
        match M4ServiceConfig::from_cli_parts(
            Vec::new(),
            vec![format!("^{}$", regex::escape(&command_line))],
        ) {
            Ok(config) => config,
            Err(error) => panic!("synthetic launch allowlist should compile: {error:#}"),
        }
    }

    fn launch_params(target: &str, args: Vec<&str>, timeout_ms: u32) -> ActLaunchParams {
        ActLaunchParams {
            target: target.to_owned(),
            args: args.into_iter().map(str::to_owned).collect(),
            working_dir: None,
            env: BTreeMap::new(),
            wait_for_window_title_regex: None,
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

    #[test]
    fn launch_command_line_quotes_empty_and_space_args() {
        let params = launch_params("notepad.exe", vec!["C:\\tmp\\hello world.txt", ""], 10_000);

        assert_eq!(
            launch_command_line(&params).unwrap_or_else(|error| {
                panic!("synthetic launch command line should build: {error}")
            }),
            "notepad.exe \"C:\\tmp\\hello world.txt\" \"\""
        );
    }

    #[cfg(windows)]
    #[test]
    fn launch_command_line_uses_win32_long_path_for_existing_path_target() {
        let dir = tempfile::TempDir::new()
            .unwrap_or_else(|error| panic!("create temp launch path dir: {error}"));
        let target_path = dir.path().join("synapse launch target.exe");
        std::fs::write(&target_path, b"synthetic")
            .unwrap_or_else(|error| panic!("write temp target: {error}"));
        let params = launch_params(&target_path.display().to_string(), vec!["--flag"], 10_000);

        let command_line = launch_command_line(&params).unwrap_or_else(|error| {
            panic!("existing path-like launch target should resolve: {error}")
        });

        assert!(
            command_line.contains("synapse launch target.exe\" --flag"),
            "{command_line}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn launch_command_line_rejects_unresolvable_path_target() {
        let dir = tempfile::TempDir::new()
            .unwrap_or_else(|error| panic!("create temp launch path dir: {error}"));
        let target_path = dir.path().join("missing-launch-target.exe");
        let params = launch_params(&target_path.display().to_string(), Vec::new(), 10_000);

        let error = match launch_command_line(&params) {
            Ok(command_line) => panic!("missing path should not resolve, got {command_line}"),
            Err(error) => error,
        };

        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("reason"))
                .and_then(|reason| reason.as_str()),
            Some("launch_target_path_resolution_failed")
        );
    }

    #[test]
    fn shell_allowlist_accepts_narrow_startup_patterns() {
        let config = M4ServiceConfig::from_cli_parts(
            vec![
                r"^git \w+$".to_owned(),
                r"^echo .{0,100}$".to_owned(),
                r"^cargo (build|test)( --[\w-]+)*$".to_owned(),
            ],
            Vec::new(),
        );

        assert!(
            config.is_ok(),
            "narrow allow-shell examples should compile: {config:?}"
        );
    }

    #[test]
    fn shell_allowlist_rejects_broad_startup_patterns() {
        let cases = [
            ("", "empty_pattern"),
            (".*", "unbounded_any_character_repetition"),
            ("^.+$", "unbounded_any_character_repetition"),
            ("^$", "matches_empty"),
            ("git status", "shell_pattern_must_match_full_command_line"),
            (r"^[\s\S]*$", "unbounded_any_character_repetition"),
        ];

        for (pattern, reason) in cases {
            let error = match M4ServiceConfig::from_cli_parts(vec![pattern.to_owned()], Vec::new())
            {
                Ok(config) => panic!("pattern {pattern:?} should reject, got {config:?}"),
                Err(error) => error,
            };
            let Some(broad) = error.downcast_ref::<BroadAllowPatternError>() else {
                panic!("pattern {pattern:?} returned unexpected error: {error:#}");
            };
            assert_eq!(broad.reason(), reason);
        }
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

    #[tokio::test]
    async fn launch_denies_without_allowlist() {
        let params = launch_params("synthetic-launch-denied", Vec::new(), 10_000);

        let error = match launch(&M4ServiceConfig::default(), params).await {
            Ok(response) => panic!("unallowlisted launch should deny, got {response:?}"),
            Err(error) => error,
        };

        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("code"))
                .and_then(|code| code.as_str()),
            Some(error_codes::SAFETY_LAUNCH_DENIED_BY_POLICY)
        );
        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("reason"))
                .and_then(|reason| reason.as_str()),
            Some("no_allow_launch_policy")
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn launch_applies_working_dir_and_env() {
        let dir = match tempfile::TempDir::new() {
            Ok(dir) => dir,
            Err(error) => panic!("create temp launch dir: {error}"),
        };
        let output_path = dir.path().join("launch-env.txt");
        let mut params = launch_params(
            "cmd.exe",
            vec!["/c", "echo %SYNAPSE_LAUNCH_ENV%>launch-env.txt"],
            10_000,
        );
        params.working_dir = Some(dir.path().display().to_string());
        params.env.insert(
            "SYNAPSE_LAUNCH_ENV".to_owned(),
            "synapse-launch-ok".to_owned(),
        );
        let config = launch_config_for(&params);

        let response = match launch(&config, params).await {
            Ok(response) => response,
            Err(error) => panic!("allowlisted cmd launch should spawn: {error}"),
        };

        assert!(response.pid > 0);
        assert_eq!(response.hwnd, None);
        assert_eq!(response.matched_title, None);
        assert_eq!(response.reason, None);
        let text = read_text_file_with_retry(&output_path).await;
        assert_eq!(text.trim(), "synapse-launch-ok");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn launch_wait_returns_reason_when_window_does_not_match() {
        let mut params = launch_params("cmd.exe", vec!["/c", "exit 0"], 50);
        params.wait_for_window_title_regex = Some("^SynapseLaunchNoSuchWindow$".to_owned());
        let config = launch_config_for(&params);

        let response = match launch(&config, params).await {
            Ok(response) => response,
            Err(error) => panic!("allowlisted cmd launch should spawn: {error}"),
        };

        assert!(response.pid > 0);
        assert_eq!(response.hwnd, None);
        assert_eq!(response.matched_title, None);
        assert_eq!(response.reason.as_deref(), Some("no_match_within_timeout"));
    }

    #[cfg(windows)]
    async fn read_text_file_with_retry(path: &std::path::Path) -> String {
        for _ in 0..100 {
            match std::fs::read_to_string(path) {
                Ok(text) => return text,
                Err(_error) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
        panic!(
            "file {} was not created by launched process",
            path.display()
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

    #[test]
    fn shell_rejects_timeout_above_max() {
        let params = shell_params(
            "cmd.exe",
            vec!["/c", "echo too-long"],
            MAX_SHELL_TIMEOUT_MS + 1,
        );

        let error = match authorize_run_shell(&shell_config_for(&params), &params) {
            Ok(_authorization) => panic!("timeout above max should reject"),
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
        assert!(error.message.contains("timeout_ms must be"));
    }

    #[test]
    fn shell_idempotency_replays_matching_completed_row() {
        let mut params = shell_params("cmd.exe", vec!["/c", "echo replay"], 30_000);
        params.idempotency_key = Some("issue-606-replay".to_owned());
        let authorization = authorize_run_shell(&shell_config_for(&params), &params)
            .unwrap_or_else(|error| panic!("authorized shell params: {error}"));
        let response = ActRunShellResponse {
            exit_code: Some(0),
            stdout: "replay\r\n".to_owned(),
            stderr: String::new(),
            duration_ms: 12,
            timed_out: false,
            stdout_truncated: false,
            stderr_truncated: false,
        };
        let row = run_shell_idempotency_completed_row(&params, &authorization, &response)
            .unwrap_or_else(|error| panic!("completed idempotency row should encode: {error}"));

        let replay = run_shell_idempotency_replay(&params, &row)
            .unwrap_or_else(|error| panic!("matching idempotency row should replay: {error}"));

        assert_eq!(replay.stdout, "replay\r\n");
        assert_eq!(replay.exit_code, Some(0));
    }

    #[test]
    fn shell_idempotency_rejects_conflicting_request_reuse() {
        let mut first = shell_params("cmd.exe", vec!["/c", "echo first"], 30_000);
        first.idempotency_key = Some("issue-606-conflict".to_owned());
        let authorization = authorize_run_shell(&shell_config_for(&first), &first)
            .unwrap_or_else(|error| panic!("first shell params should authorize: {error}"));
        let response = ActRunShellResponse {
            exit_code: Some(0),
            stdout: "first\r\n".to_owned(),
            stderr: String::new(),
            duration_ms: 10,
            timed_out: false,
            stdout_truncated: false,
            stderr_truncated: false,
        };
        let row = run_shell_idempotency_completed_row(&first, &authorization, &response)
            .unwrap_or_else(|error| panic!("completed idempotency row should encode: {error}"));
        let mut second = shell_params("cmd.exe", vec!["/c", "echo second"], 30_000);
        second.idempotency_key = first.idempotency_key.clone();

        let error = match run_shell_idempotency_replay(&second, &row) {
            Ok(replay) => panic!("conflicting idempotency reuse should reject, got {replay:?}"),
            Err(error) => error,
        };

        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("reason"))
                .and_then(|reason| reason.as_str()),
            Some("idempotency_key_conflict")
        );
    }
}
