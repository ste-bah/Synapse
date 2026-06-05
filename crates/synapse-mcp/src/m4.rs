use std::{
    borrow::Cow,
    collections::{BTreeMap, HashSet},
    io,
    path::Path,
    process::{Command as StdCommand, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Context;
use rmcp::{
    ErrorData,
    model::ErrorCode,
    schemars::{JsonSchema, Schema, SchemaGenerator, json_schema},
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use synapse_core::{
    Action, Backend, ComboInput, ComboStep, ForegroundContext, Key, error_codes, new_reflex_id,
};
use synapse_reflex::{ComboParams, ReflexRuntime, ScheduledReflex};
use synapse_storage::{decode_json, encode_json};
use tokio::{io::AsyncReadExt, process::Command as TokioCommand};

use crate::{
    m1::mcp_error,
    m2::{ActPressParams, action_from_press_params},
    m3::permissions::{RequiredPermissions, add_action_permissions},
};

const MAX_COMBO_STEPS: usize = 256;
const DEFAULT_SHELL_TIMEOUT_MS: u32 = 30_000;
const MAX_SHELL_TIMEOUT_MS: u32 = 600_000;
const DEFAULT_LAUNCH_TIMEOUT_MS: u32 = 10_000;
const MAX_LAUNCH_TIMEOUT_MS: u32 = 600_000;
#[cfg(windows)]
const SW_SHOWNORMAL: u16 = 1;
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
const LAUNCH_FOREGROUND_STABLE_MS: u64 = 750;
const LAUNCH_FOREGROUND_POLL_MS: u64 = 75;
const LAUNCH_FOREGROUND_MAX_MS: u64 = 3_000;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActComboAction {
    ActPress,
    Retired(String),
}

impl<'de> Deserialize<'de> for ActComboAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "act_press" => Ok(Self::ActPress),
            _ => Ok(Self::Retired(value)),
        }
    }
}

impl JsonSchema for ActComboAction {
    fn schema_name() -> Cow<'static, str> {
        "ActComboAction".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "enum": ["act_press"],
            "description": "Only timed act_press key steps are supported. Use act_stroke for continuous mouse motion/path execution."
        })
    }
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

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
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
    /// Controls CDP debug-port injection for Chromium-family targets (#684).
    /// `None` (default) = auto: inject `--remote-debugging-port=0` + a dedicated
    /// `--user-data-dir` so `observe`/`find` can read the browser's DOM without
    /// manual flags. `Some(false)` = opt out (launch the browser untouched).
    /// `Some(true)` = force injection even if heuristics would skip it.
    /// Ignored for non-Chromium targets.
    #[serde(default)]
    #[schemars(default)]
    pub cdp_debug: Option<bool>,
    /// Opt-in Chromium UIA renderer accessibility fallback (#689).
    /// `Some(true)` adds `--force-renderer-accessibility` for Chromium-family
    /// launches unless the caller already supplied that flag. `Some(false)`
    /// disables the env opt-in. `None` follows `SYNAPSE_FORCE_RENDERER_ACCESSIBILITY`.
    #[serde(default)]
    #[schemars(default)]
    pub force_renderer_accessibility: Option<bool>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActLaunchResponse {
    pub pid: u32,
    pub hwnd: Option<i64>,
    pub matched_title: Option<String>,
    pub launched_at: String,
    pub reason: Option<String>,
    /// CDP debug port opened for a Synapse-launched Chromium browser (#684), if
    /// injection ran. `observe`/`find` use it to attach and read the DOM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdp_debug_port: Option<u16>,
    /// CDP HTTP endpoint (`http://127.0.0.1:<port>`) when a debug port opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdp_endpoint: Option<String>,
    /// Dedicated automation `--user-data-dir` the browser was launched with.
    /// NOT the user's primary profile — logins there do not carry over (Chrome
    /// 136+ refuses remote debugging on the default profile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdp_user_data_dir: Option<String>,
    /// URL observed in the launched browser's CDP target list when Synapse
    /// opened the debug port and the caller supplied a URL argument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdp_verified_url: Option<String>,
    /// Title observed for the verified CDP target URL, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdp_verified_title: Option<String>,
}

pub fn launch_request_details(params: &ActLaunchParams) -> serde_json::Value {
    json!({
        "target": params.target,
        "args": params.args,
        "working_dir": params.working_dir,
        "env_keys": params.env.keys().cloned().collect::<Vec<_>>(),
        "wait_for_window_title_regex": params.wait_for_window_title_regex,
        "timeout_ms": params.timeout_ms,
        "idempotency_key_present": params.idempotency_key.is_some(),
        "cdp_debug": params.cdp_debug,
        "force_renderer_accessibility": params.force_renderer_accessibility,
        "windows_new_console": launch_target_needs_new_console(&params.target),
        "request_sha256": launch_request_sha256(params).ok(),
    })
}

pub fn launch_process_history_row_key(response: &ActLaunchResponse) -> Vec<u8> {
    format!(
        "process_history/v1/act_launch/{}/{}",
        response.launched_at.replace(':', "_"),
        response.pid
    )
    .into_bytes()
}

pub fn launch_process_history_row(
    params: &ActLaunchParams,
    response: &ActLaunchResponse,
) -> Result<Vec<u8>, ErrorData> {
    let row = json!({
        "schema_version": 1,
        "row_kind": "process_start",
        "tool": "act_launch",
        "status": "started",
        "target": params.target,
        "args": params.args,
        "working_dir": params.working_dir,
        "env_keys": params.env.keys().cloned().collect::<Vec<_>>(),
        "wait_for_window_title_regex": params.wait_for_window_title_regex,
        "timeout_ms": params.timeout_ms,
        "idempotency_key_present": params.idempotency_key.is_some(),
        "windows_new_console": launch_target_needs_new_console(&params.target),
        "request_sha256": launch_request_sha256(params).ok(),
        "command_line": launch_command_line(params).ok(),
        "pid": response.pid,
        "hwnd": response.hwnd,
        "matched_title": response.matched_title,
        "launched_at": response.launched_at,
        "reason": response.reason,
        "cdp_debug": params.cdp_debug,
        "force_renderer_accessibility": params.force_renderer_accessibility,
        "cdp_debug_port": response.cdp_debug_port,
        "cdp_endpoint": response.cdp_endpoint,
        "cdp_user_data_dir": response.cdp_user_data_dir,
        "cdp_verified_url": response.cdp_verified_url,
        "cdp_verified_title": response.cdp_verified_title,
    });
    encode_json(&row).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("act_launch process history row encode failed: {error}"),
        )
    })
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

fn launch_request_sha256(params: &ActLaunchParams) -> Result<String, ErrorData> {
    let payload = json!({
        "target": params.target,
        "args": params.args,
        "working_dir": params.working_dir,
        "env": params.env,
        "wait_for_window_title_regex": params.wait_for_window_title_regex,
        "timeout_ms": params.timeout_ms,
        "cdp_debug": params.cdp_debug,
        "force_renderer_accessibility": params.force_renderer_accessibility,
    });
    let bytes = serde_json::to_vec(&payload).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("act_launch request fingerprint encode failed: {error}"),
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
    // #684: make a CDP debug port reachable for Synapse-launched Chromium so
    // observe/find can read the page DOM without manual flags. Augment the spawn
    // command only (policy already matched the original command above).
    let cdp_launch = chromium_cdp_launch(&params);
    let force_renderer_accessibility = chromium_renderer_accessibility_arg(&params);
    let spawn_params = params_with_chromium_launch_args(
        &params,
        cdp_launch.as_ref(),
        force_renderer_accessibility,
    );
    let pid = spawn_launch_child(&spawn_params)?;
    let cdp = if let Some(launch) = &cdp_launch {
        resolve_launched_cdp_port(pid, launch).await
    } else {
        LaunchedCdp::default()
    };
    let launch_target_name = launch_target_file_name(&params.target);

    let cdp_target =
        verify_launched_chromium_url(&params, cdp_launch.as_ref(), &cdp, params.timeout_ms).await?;
    let window = if let Some(regex) = wait_regex {
        wait_for_launch_window(
            pid,
            &regex,
            params.timeout_ms,
            &excluded_hwnds,
            &launch_target_name,
            &params.args,
        )
        .await?
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
        cdp_debug_port = ?cdp.port,
        cdp_verified_url = ?cdp_target.as_ref().map(|target| target.url.as_str()),
        "readback=act_launch after=process_spawn"
    );
    Ok(ActLaunchResponse {
        pid,
        hwnd: window.hwnd,
        matched_title: window.matched_title,
        launched_at,
        reason: window.reason,
        cdp_debug_port: cdp.port,
        cdp_endpoint: cdp.endpoint,
        cdp_user_data_dir: cdp.user_data_dir,
        cdp_verified_url: cdp_target.as_ref().map(|target| target.url.clone()),
        cdp_verified_title: cdp_target.and_then(|target| target.title),
    })
}

/// Planned CDP-debug augmentation for a Chromium-family launch (#684).
#[derive(Clone, Debug)]
struct ChromiumCdpLaunch {
    /// Dedicated automation profile dir (must be non-default for Chrome 136+).
    user_data_dir: std::path::PathBuf,
    /// Args injected ahead of the caller's args.
    injected_args: Vec<String>,
}

/// Optional Chromium renderer accessibility launch flag (#689).
///
/// Kept independent from CDP injection: callers may opt into the UIA renderer
/// tree even when they opt out of CDP, and CDP users may opt in to improve the
/// non-CDP fallback path.
fn chromium_renderer_accessibility_arg(params: &ActLaunchParams) -> Option<String> {
    if !force_renderer_accessibility_enabled(params) {
        return None;
    }
    if !synapse_a11y::is_chromium_family(&launch_target_file_name(&params.target)) {
        return None;
    }
    let already_configured = params
        .args
        .iter()
        .any(|arg| is_force_renderer_accessibility_arg(arg));
    if already_configured {
        tracing::info!(
            code = "M4_ACT_LAUNCH_RENDERER_A11Y_SKIPPED",
            reason = "caller_supplied_force_renderer_accessibility",
            "act_launch leaving caller-specified renderer accessibility flag untouched"
        );
        return None;
    }
    Some("--force-renderer-accessibility".to_owned())
}

fn is_force_renderer_accessibility_arg(arg: &str) -> bool {
    let lower = arg.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("--force-renderer-accessibility") else {
        return false;
    };
    rest.is_empty() || rest.starts_with('=')
}

fn force_renderer_accessibility_enabled(params: &ActLaunchParams) -> bool {
    match params.force_renderer_accessibility {
        Some(value) => value,
        None => truthy_env("SYNAPSE_FORCE_RENDERER_ACCESSIBILITY"),
    }
}

fn truthy_env(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| truthy_value(&value))
        .unwrap_or(false)
}

fn truthy_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Outcome of opening + discovering a launched browser's CDP port.
#[derive(Clone, Debug, Default)]
struct LaunchedCdp {
    port: Option<u16>,
    endpoint: Option<String>,
    user_data_dir: Option<String>,
}

/// Decides whether to inject CDP debug flags for this launch. Returns `None` for
/// non-Chromium targets, when the caller opted out (`cdp_debug = Some(false)`),
/// or when the caller already specified a debug port / user-data-dir (respect
/// their intent). Otherwise plans an ephemeral port + dedicated profile.
fn chromium_cdp_launch(params: &ActLaunchParams) -> Option<ChromiumCdpLaunch> {
    if params.cdp_debug == Some(false) {
        return None;
    }
    let is_chromium = synapse_a11y::is_chromium_family(&launch_target_file_name(&params.target));
    if !is_chromium && params.cdp_debug != Some(true) {
        return None;
    }
    let already_configured = params.args.iter().any(|arg| {
        let lower = arg.to_ascii_lowercase();
        lower.starts_with("--remote-debugging-port") || lower.starts_with("--user-data-dir")
    });
    if already_configured {
        tracing::info!(
            code = "M4_ACT_LAUNCH_CDP_SKIPPED",
            reason = "caller_supplied_debug_or_profile_flags",
            "act_launch leaving caller-specified CDP/profile flags untouched"
        );
        return None;
    }
    let user_data_dir = cdp_automation_profile_dir();
    let injected_args = vec![
        "--remote-debugging-port=0".to_owned(),
        format!("--user-data-dir={}", user_data_dir.display()),
        "--no-first-run".to_owned(),
        "--no-default-browser-check".to_owned(),
    ];
    Some(ChromiumCdpLaunch {
        user_data_dir,
        injected_args,
    })
}

/// A fresh ActLaunchParams whose injected browser args precede the caller's
/// args (so a positional URL still parses).
fn params_with_chromium_launch_args(
    params: &ActLaunchParams,
    cdp_launch: Option<&ChromiumCdpLaunch>,
    force_renderer_accessibility: Option<String>,
) -> ActLaunchParams {
    let mut spawn_params = params.clone();
    let mut args = cdp_launch
        .map(|launch| launch.injected_args.clone())
        .unwrap_or_default();
    if let Some(arg) = force_renderer_accessibility {
        args.push(arg);
    }
    args.extend(params.args.iter().cloned());
    spawn_params.args = args;
    spawn_params
}

/// Dedicated, non-default automation profile dir. Honors
/// `SYNAPSE_CDP_USER_DATA_DIR` for a stable (login-persisting) profile; otherwise
/// a unique per-launch dir under the OS temp so concurrent browsers never share
/// a profile (Chrome refuses a second debug port on the same profile).
fn cdp_automation_profile_dir() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("SYNAPSE_CDP_USER_DATA_DIR") {
        let dir = std::path::PathBuf::from(dir);
        if !dir.as_os_str().is_empty() {
            return dir;
        }
    }
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let token = format!("{}-{seq}-{nanos:x}", std::process::id());
    std::env::temp_dir()
        .join("synapse-cdp-profiles")
        .join(token)
}

/// Polls the launched browser's `DevToolsActivePort` file (Chrome writes the
/// chosen ephemeral port there) and registers it so observe/find can attach.
/// Fail-loud: logs an error if the port never appears, but does not orphan the
/// already-spawned browser.
async fn resolve_launched_cdp_port(pid: u32, launch: &ChromiumCdpLaunch) -> LaunchedCdp {
    let port_file = launch.user_data_dir.join("DevToolsActivePort");
    let deadline = Instant::now() + Duration::from_secs(15);
    let user_data_dir = Some(launch.user_data_dir.display().to_string());
    loop {
        if let Some(port) = read_devtools_active_port(&port_file) {
            synapse_a11y::register_launched_port(pid, port);
            tracing::info!(
                code = "M4_ACT_LAUNCH_CDP_PORT_OPENED",
                pid,
                port,
                user_data_dir = ?launch.user_data_dir,
                "act_launch opened a CDP debug port for the launched browser"
            );
            return LaunchedCdp {
                port: Some(port),
                endpoint: Some(format!("http://127.0.0.1:{port}")),
                user_data_dir,
            };
        }
        if Instant::now() >= deadline {
            tracing::error!(
                code = error_codes::A11Y_CDP_ATTACH_FAILED,
                pid,
                user_data_dir = ?launch.user_data_dir,
                "act_launch injected CDP flags but DevToolsActivePort never appeared; \
                 the browser launched but its DOM will not be observable via CDP"
            );
            return LaunchedCdp {
                port: None,
                endpoint: None,
                user_data_dir,
            };
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Reads the first line of a `DevToolsActivePort` file as a port number.
fn read_devtools_active_port(path: &Path) -> Option<u16> {
    let contents = std::fs::read_to_string(path).ok()?;
    contents.lines().next()?.trim().parse::<u16>().ok()
}

#[derive(Clone, Debug)]
struct VerifiedCdpTarget {
    url: String,
    title: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct CdpTargetListEntry {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default, rename = "type")]
    target_type: Option<String>,
}

async fn verify_launched_chromium_url(
    params: &ActLaunchParams,
    cdp_launch: Option<&ChromiumCdpLaunch>,
    cdp: &LaunchedCdp,
    timeout_ms: u32,
) -> Result<Option<VerifiedCdpTarget>, ErrorData> {
    let Some(expected_url) = launch_requested_url(&params.args) else {
        return Ok(None);
    };
    if cdp_launch.is_none() {
        return Ok(None);
    }
    let Some(endpoint) = cdp.endpoint.as_deref() else {
        return Err(launch_url_verification_error(
            "cdp_endpoint_missing",
            &expected_url,
            None,
            timeout_ms,
            None,
            &[],
        ));
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(750))
        .build()
        .map_err(|error| {
            launch_tool_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("act_launch failed to build CDP verification client: {error}"),
                json!({
                    "code": error_codes::TOOL_INTERNAL_ERROR,
                    "reason": "cdp_verification_client_build_failed",
                    "expected_url": expected_url,
                    "endpoint": endpoint,
                }),
            )
        })?;
    let started = Instant::now();
    let timeout = Duration::from_millis(u64::from(timeout_ms));
    let mut last_error: Option<String>;
    let mut last_targets = Vec::new();
    loop {
        match fetch_cdp_target_list(&client, endpoint).await {
            Ok(targets) => {
                last_targets = target_summaries(&targets);
                if let Some(target) = targets.iter().find(|target| {
                    target
                        .target_type
                        .as_deref()
                        .is_none_or(|kind| kind == "page")
                        && url_matches(&expected_url, &target.url)
                }) {
                    tracing::info!(
                        code = "M4_ACT_LAUNCH_CDP_URL_VERIFIED",
                        endpoint,
                        expected_url,
                        actual_url = %target.url,
                        title = ?target.title,
                        "act_launch verified requested browser URL in CDP target list"
                    );
                    return Ok(Some(VerifiedCdpTarget {
                        url: target.url.clone(),
                        title: target.title.clone().filter(|title| !title.is_empty()),
                    }));
                }
                last_error = None;
            }
            Err(error) => {
                last_error = Some(error);
            }
        }

        if started.elapsed() >= timeout {
            return Err(launch_url_verification_error(
                "url_not_observed_within_timeout",
                &expected_url,
                Some(endpoint),
                timeout_ms,
                last_error,
                &last_targets,
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn fetch_cdp_target_list(
    client: &reqwest::Client,
    endpoint: &str,
) -> Result<Vec<CdpTargetListEntry>, String> {
    let url = format!("{}/json/list", endpoint.trim_end_matches('/'));
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("GET {url}: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("GET {url}: HTTP {status}"));
    }
    response
        .json::<Vec<CdpTargetListEntry>>()
        .await
        .map_err(|error| format!("decode {url}: {error}"))
}

fn launch_requested_url(args: &[String]) -> Option<String> {
    args.iter().find_map(|arg| {
        if let Some(value) = arg.strip_prefix("--app=")
            && supported_launch_url(value)
        {
            return Some(value.to_owned());
        }
        if arg.starts_with("--") {
            return None;
        }
        supported_launch_url(arg).then(|| arg.to_owned())
    })
}

fn supported_launch_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "file"))
}

fn url_matches(expected: &str, actual: &str) -> bool {
    canonical_launch_url(expected)
        .zip(canonical_launch_url(actual))
        .is_some_and(|(expected, actual)| expected == actual)
}

fn canonical_launch_url(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https" | "file") {
        return None;
    }
    let mut canonical = url.to_string();
    if url.path() == "/" && url.query().is_none() && url.fragment().is_none() {
        canonical = canonical.trim_end_matches('/').to_owned();
    }
    Some(canonical)
}

fn target_summaries(targets: &[CdpTargetListEntry]) -> Vec<serde_json::Value> {
    targets
        .iter()
        .take(5)
        .map(|target| {
            json!({
                "type": target.target_type.as_deref(),
                "title": target.title.as_deref(),
                "url": target.url.as_str(),
            })
        })
        .collect()
}

fn launch_url_verification_error(
    reason: &'static str,
    expected_url: &str,
    endpoint: Option<&str>,
    timeout_ms: u32,
    last_error: Option<String>,
    observed_targets: &[serde_json::Value],
) -> ErrorData {
    launch_tool_error(
        error_codes::ACTION_LAUNCH_URL_NOT_REACHED,
        format!("act_launch did not verify requested browser URL: {reason}"),
        json!({
            "code": error_codes::ACTION_LAUNCH_URL_NOT_REACHED,
            "reason": reason,
            "expected_url": expected_url,
            "endpoint": endpoint,
            "timeout_ms": timeout_ms,
            "last_error": last_error,
            "observed_targets": observed_targets,
        }),
    )
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
            ActComboAction::Retired(ref action) => {
                return Err(mcp_error(
                    error_codes::TOOL_PARAMS_INVALID,
                    retired_combo_action_message(index, action),
                ));
            }
        }
    }
    Ok(out)
}

fn retired_combo_action_message(index: usize, action: &str) -> String {
    match action {
        "act_aim" | "act_drag" | "act_stroke" | "mouse_move" | "MouseMove" => format!(
            "act_combo steps[{index}].action {action:?} is not combo-lowerable; act_combo is intentionally limited to timed act_press key steps. Use act_stroke for continuous mouse motion/path execution."
        ),
        _ => format!(
            "act_combo steps[{index}].action {action:?} is not combo-lowerable; supported action: act_press"
        ),
    }
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
    if params.timeout_ms == 0 || params.timeout_ms > MAX_LAUNCH_TIMEOUT_MS {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("act_launch timeout_ms must be 1..={MAX_LAUNCH_TIMEOUT_MS}"),
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

fn spawn_launch_child(params: &ActLaunchParams) -> Result<u32, ErrorData> {
    let needs_new_console = launch_target_needs_new_console(&params.target);
    #[cfg(windows)]
    if needs_new_console {
        return spawn_windows_console_child(params);
    }

    let mut command = StdCommand::new(&params.target);
    command.args(&params.args);
    if let Some(working_dir) = &params.working_dir {
        command.current_dir(working_dir);
    }
    apply_launch_environment(&mut command, params);
    if needs_new_console {
        apply_new_console_creation_flags(&mut command);
    } else {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    }

    let child = command.spawn().map_err(|error| {
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
    })?;
    Ok(child.id())
}

fn apply_launch_environment(command: &mut StdCommand, params: &ActLaunchParams) {
    command.env_clear();
    for key in PROCESS_BASE_ENV_KEYS {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.envs(&params.env);
}

fn launch_target_needs_new_console(target: &str) -> bool {
    let name = launch_target_file_name(target);
    matches!(name.as_str(), "cmd.exe" | "powershell.exe" | "pwsh.exe")
}

fn launch_target_file_name(target: &str) -> String {
    Path::new(target)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(target)
        .to_ascii_lowercase()
}

#[cfg(windows)]
fn spawn_windows_console_child(params: &ActLaunchParams) -> Result<u32, ErrorData> {
    use windows::{
        Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                CREATE_NEW_CONSOLE, CREATE_NEW_PROCESS_GROUP, CREATE_UNICODE_ENVIRONMENT,
                CreateProcessW, PROCESS_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW,
            },
        },
        core::{PCWSTR, PWSTR},
    };

    let command_line = launch_command_line(params)?;
    let mut command_line_wide = wide_null(&command_line);
    let current_dir_wide = params.working_dir.as_ref().map(|dir| wide_null(dir));
    let environment = launch_environment_block(params)?;
    let startup_info_cb = u32::try_from(std::mem::size_of::<STARTUPINFOW>()).map_err(|error| {
        launch_tool_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("act_launch failed to prepare console startup info: {error}"),
            json!({
                "code": error_codes::TOOL_INTERNAL_ERROR,
                "reason": "launch_startup_info_size_overflow",
                "target": params.target,
            }),
        )
    })?;

    let startup_info = STARTUPINFOW {
        cb: startup_info_cb,
        dwFlags: STARTF_USESHOWWINDOW,
        wShowWindow: SW_SHOWNORMAL,
        ..Default::default()
    };

    let mut process_info = PROCESS_INFORMATION::default();
    let current_dir = current_dir_wide
        .as_ref()
        .map_or(PCWSTR::null(), |dir| PCWSTR(dir.as_ptr()));

    let result = unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(command_line_wide.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP | CREATE_UNICODE_ENVIRONMENT,
            Some(environment.as_ptr().cast()),
            current_dir,
            &raw const startup_info,
            &raw mut process_info,
        )
    };

    match result {
        Ok(()) => {
            let pid = process_info.dwProcessId;
            let _ = unsafe { CloseHandle(process_info.hThread) };
            let _ = unsafe { CloseHandle(process_info.hProcess) };
            Ok(pid)
        }
        Err(error) => Err(launch_tool_error(
            error_codes::ACTION_TARGET_INVALID,
            format!("act_launch failed to spawn console target: {error}"),
            json!({
                "code": error_codes::ACTION_TARGET_INVALID,
                "target": params.target,
                "args": params.args,
                "working_dir": params.working_dir,
                "reason": "spawn_failed",
            }),
        )),
    }
}

#[cfg(windows)]
fn launch_environment_block(params: &ActLaunchParams) -> Result<Vec<u16>, ErrorData> {
    let mut env: BTreeMap<String, (String, String)> = BTreeMap::new();
    for key in PROCESS_BASE_ENV_KEYS {
        if let Some(value) = std::env::var_os(key) {
            env.insert(
                key.to_ascii_uppercase(),
                (key.to_owned(), value.to_string_lossy().into_owned()),
            );
        }
    }
    for (key, value) in &params.env {
        validate_launch_environment_entry(key, value)?;
        env.insert(key.to_ascii_uppercase(), (key.clone(), value.clone()));
    }

    let mut block = Vec::new();
    for (_sort_key, (key, value)) in env {
        block.extend(format!("{key}={value}").encode_utf16());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

#[cfg(windows)]
fn validate_launch_environment_entry(key: &str, value: &str) -> Result<(), ErrorData> {
    if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_launch env entries must have non-empty keys without '=' or NUL and values without NUL",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
const fn apply_new_console_creation_flags(_command: &mut StdCommand) {}

#[cfg(not(windows))]
const fn apply_new_console_creation_flags(_command: &mut StdCommand) {}

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
}

async fn wait_for_launch_window(
    pid: u32,
    title_regex: &regex::Regex,
    timeout_ms: u32,
    excluded_hwnds: &HashSet<i64>,
    launch_target_name: &str,
    launch_args: &[String],
) -> Result<WindowWaitResult, ErrorData> {
    let started = Instant::now();
    let timeout = Duration::from_millis(u64::from(timeout_ms));
    let mut last_error: Option<String>;
    let mut last_windows = Vec::new();
    loop {
        match synapse_a11y::visible_top_level_window_contexts() {
            Ok(contexts) => {
                last_windows = window_context_summaries(&contexts);
                if let Some(context) = select_launch_window(
                    &contexts,
                    pid,
                    title_regex,
                    excluded_hwnds,
                    launch_target_name,
                    launch_args,
                ) {
                    let foreground = focus_launch_window(context.hwnd).await?;
                    return Ok(WindowWaitResult::matched(foreground));
                }
                last_error = None;
            }
            Err(error) if error.code() == error_codes::A11Y_NOT_AVAILABLE => {
                tracing::error!(
                    code = error.code(),
                    error = %error,
                    "act_launch window readback unavailable"
                );
                return Err(launch_window_error(
                    "window_readback_unavailable",
                    pid,
                    title_regex.as_str(),
                    timeout_ms,
                    Some(error.to_string()),
                    &last_windows,
                ));
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
            return Err(launch_window_error(
                "no_match_within_timeout",
                pid,
                title_regex.as_str(),
                timeout_ms,
                last_error,
                &last_windows,
            ));
        }
        tokio::time::sleep(Duration::from_millis(LAUNCH_WINDOW_POLL_INTERVAL_MS)).await;
    }
}

async fn focus_launch_window(hwnd: i64) -> Result<ForegroundContext, ErrorData> {
    let mut last_error = None;
    let started = Instant::now();
    let deadline = started + Duration::from_millis(LAUNCH_FOREGROUND_MAX_MS);
    let stable_for = Duration::from_millis(LAUNCH_FOREGROUND_STABLE_MS);
    let mut stable_since: Option<Instant> = None;
    let mut focus_attempts = 0usize;
    let mut last_matching_context: Option<ForegroundContext> = None;

    loop {
        match synapse_a11y::current_foreground_context() {
            Ok(context) if context.hwnd == hwnd => {
                let now = Instant::now();
                let stable_since = *stable_since.get_or_insert(now);
                last_matching_context = Some(context.clone());
                if now.duration_since(stable_since) >= stable_for {
                    tracing::info!(
                        code = "M4_ACT_LAUNCH_FOCUSED",
                        hwnd,
                        attempts = focus_attempts,
                        stable_ms = LAUNCH_FOREGROUND_STABLE_MS,
                        pid = context.pid,
                        title = %context.window_title,
                        "act_launch foregrounded the matched window and verified it stayed foreground"
                    );
                    return Ok(context);
                }
            }
            Ok(context) => {
                stable_since = None;
                last_error = Some(format!(
                    "foreground readback hwnd 0x{:x} pid {} title {:?}, expected hwnd 0x{hwnd:x}",
                    context.hwnd, context.pid, context.window_title
                ));
            }
            Err(error) => {
                stable_since = None;
                last_error = Some(format!("foreground readback failed: {error}"));
            }
        }

        if Instant::now() >= deadline {
            break;
        }

        focus_attempts += 1;
        if let Err(error) = synapse_a11y::focus_window(hwnd) {
            stable_since = None;
            last_error = Some(error.to_string());
        }
        tokio::time::sleep(Duration::from_millis(LAUNCH_FOREGROUND_POLL_MS)).await;
    }
    tracing::error!(
        code = "M4_ACT_LAUNCH_FOCUS_FAILED",
        hwnd,
        error = ?last_error,
        attempts = focus_attempts,
        stable_ms = LAUNCH_FOREGROUND_STABLE_MS,
        last_matching_title = ?last_matching_context.as_ref().map(|context| context.window_title.as_str()),
        "act_launch matched the launched window but could not keep it foreground after retries"
    );
    Err(launch_tool_error(
        error_codes::ACTION_LAUNCH_FOREGROUND_FAILED,
        "act_launch matched the launched window but could not keep it foreground after retries",
        json!({
            "code": error_codes::ACTION_LAUNCH_FOREGROUND_FAILED,
            "reason": "foreground_not_stable",
            "hwnd": hwnd,
            "attempts": focus_attempts,
            "required_stable_ms": LAUNCH_FOREGROUND_STABLE_MS,
            "max_wait_ms": LAUNCH_FOREGROUND_MAX_MS,
            "last_error": last_error,
            "last_matching_context": last_matching_context.map(|context| json!({
                "hwnd": context.hwnd,
                "pid": context.pid,
                "process_name": context.process_name,
                "title": context.window_title,
            })),
        }),
    ))
}

fn window_context_summaries(contexts: &[ForegroundContext]) -> Vec<serde_json::Value> {
    contexts
        .iter()
        .take(12)
        .map(|context| {
            json!({
                "hwnd": context.hwnd,
                "pid": context.pid,
                "process_name": context.process_name,
                "title": context.window_title,
            })
        })
        .collect()
}

fn launch_window_error(
    reason: &'static str,
    pid: u32,
    title_regex: &str,
    timeout_ms: u32,
    last_error: Option<String>,
    observed_windows: &[serde_json::Value],
) -> ErrorData {
    launch_tool_error(
        error_codes::ACTION_LAUNCH_WINDOW_NOT_FOUND,
        format!("act_launch did not verify requested launch window: {reason}"),
        json!({
            "code": error_codes::ACTION_LAUNCH_WINDOW_NOT_FOUND,
            "reason": reason,
            "pid": pid,
            "title_regex": title_regex,
            "timeout_ms": timeout_ms,
            "last_error": last_error,
            "observed_windows": observed_windows,
        }),
    )
}

fn select_launch_window<'a>(
    contexts: &'a [ForegroundContext],
    pid: u32,
    title_regex: &regex::Regex,
    excluded_hwnds: &HashSet<i64>,
    launch_target_name: &str,
    launch_args: &[String],
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
        .or_else(|| {
            contexts.iter().find(|context| {
                excluded_hwnds.contains(&context.hwnd)
                    && launch_target_matches_existing_window(
                        launch_target_name,
                        launch_args,
                        context,
                    )
                    && title_regex.is_match(&context.window_title)
            })
        })
}

fn launch_target_matches_existing_window(
    target_name: &str,
    launch_args: &[String],
    context: &ForegroundContext,
) -> bool {
    let target_name = target_name.to_ascii_lowercase();
    let process_name = context.process_name.to_ascii_lowercase();
    target_name == process_name
        || launch_target_matches_shell_activation(&target_name, launch_args, &process_name)
        || matches!(
            (target_name.as_str(), process_name.as_str()),
            ("wt.exe", "windowsterminal.exe")
                | (
                    "calc.exe",
                    "calculatorapp.exe" | "calculator.exe" | "applicationframehost.exe",
                )
                | (
                    "cmd.exe" | "powershell.exe" | "pwsh.exe",
                    "windowsterminal.exe" | "openconsole.exe" | "conhost.exe",
                )
        )
}

fn launch_target_matches_shell_activation(
    target_name: &str,
    launch_args: &[String],
    process_name: &str,
) -> bool {
    if target_name != "explorer.exe" {
        return false;
    }
    let args = launch_args
        .iter()
        .map(|arg| arg.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "ms-settings:") {
        return matches!(
            process_name,
            "systemsettings.exe" | "applicationframehost.exe"
        );
    }
    if args
        .iter()
        .any(|arg| arg.contains("microsoft.windows.photos"))
    {
        return matches!(
            process_name,
            "photos.exe" | "microsoft.photos.exe" | "applicationframehost.exe"
        );
    }
    false
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
    let mut command = TokioCommand::new(&params.command);
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

    fn foreground_for_launch_selection(
        hwnd: i64,
        pid: u32,
        process_name: &str,
        window_title: &str,
    ) -> ForegroundContext {
        ForegroundContext {
            hwnd,
            pid,
            process_name: process_name.to_owned(),
            process_path: format!(r"C:\Synthetic\{process_name}"),
            window_title: window_title.to_owned(),
            window_bounds: synapse_core::Rect {
                x: 0,
                y: 0,
                w: 640,
                h: 480,
            },
            monitor_index: 0,
            dpi_scale: 1.0,
            profile_id: None,
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
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
            cdp_debug: None,
            force_renderer_accessibility: None,
        }
    }

    #[test]
    fn chromium_cdp_launch_injects_ephemeral_port_and_dedicated_profile() {
        let params = launch_params("chrome.exe", vec!["https://example.com"], 10_000);
        let launch = chromium_cdp_launch(&params).expect("chrome.exe should get CDP injection");
        println!(
            "readback=cdp_launch edge=chrome before=args:{:?} after=injected:{:?} udd:{:?}",
            params.args, launch.injected_args, launch.user_data_dir
        );
        assert!(
            launch
                .injected_args
                .iter()
                .any(|arg| arg == "--remote-debugging-port=0")
        );
        assert!(
            launch
                .injected_args
                .iter()
                .any(|arg| arg.starts_with("--user-data-dir="))
        );
        // The dedicated profile must be non-default (Chrome 136+ requirement).
        assert!(
            launch
                .user_data_dir
                .to_string_lossy()
                .contains("synapse-cdp-profiles")
        );

        let spawn_params = params_with_chromium_launch_args(&params, Some(&launch), None);
        // Injected flags precede the caller's URL so the positional arg parses.
        assert_eq!(
            spawn_params.args.first().map(String::as_str),
            Some("--remote-debugging-port=0")
        );
        assert_eq!(
            spawn_params.args.last().map(String::as_str),
            Some("https://example.com")
        );
    }

    #[test]
    fn chromium_cdp_launch_respects_opt_out_and_non_chromium() {
        let mut opted_out = launch_params("chrome.exe", vec![], 10_000);
        opted_out.cdp_debug = Some(false);
        println!("readback=cdp_launch edge=opt_out before=cdp_debug:Some(false)");
        assert!(chromium_cdp_launch(&opted_out).is_none());

        let notepad = launch_params("notepad.exe", vec![], 10_000);
        println!("readback=cdp_launch edge=non_chromium before=target:notepad.exe");
        assert!(chromium_cdp_launch(&notepad).is_none());
    }

    #[test]
    fn chromium_cdp_launch_defers_to_caller_supplied_flags() {
        let with_port = launch_params("msedge.exe", vec!["--remote-debugging-port=9222"], 10_000);
        println!(
            "readback=cdp_launch edge=caller_port before=args:{:?}",
            with_port.args
        );
        assert!(chromium_cdp_launch(&with_port).is_none());

        let with_profile = launch_params("chrome.exe", vec!["--user-data-dir=C:\\my"], 10_000);
        assert!(chromium_cdp_launch(&with_profile).is_none());
    }

    #[test]
    fn chromium_renderer_accessibility_is_opt_in_and_chromium_only() {
        let mut params = launch_params("chrome.exe", vec!["https://example.com"], 10_000);
        println!(
            "readback=renderer_a11y edge=default before=force_renderer_accessibility:{:?}",
            params.force_renderer_accessibility
        );
        assert!(chromium_renderer_accessibility_arg(&params).is_none());

        params.force_renderer_accessibility = Some(true);
        let arg = chromium_renderer_accessibility_arg(&params);
        println!(
            "readback=renderer_a11y edge=opt_in before=args:{:?} after=arg:{arg:?}",
            params.args
        );
        assert_eq!(arg.as_deref(), Some("--force-renderer-accessibility"));

        let launch = chromium_cdp_launch(&params).expect("chrome should still get CDP launch");
        let spawn_params = params_with_chromium_launch_args(&params, Some(&launch), arg);
        assert!(
            spawn_params
                .args
                .iter()
                .any(|arg| arg == "--force-renderer-accessibility")
        );
        assert_eq!(
            spawn_params.args.last().map(String::as_str),
            Some("https://example.com")
        );

        let mut notepad = launch_params("notepad.exe", vec![], 10_000);
        notepad.force_renderer_accessibility = Some(true);
        assert!(chromium_renderer_accessibility_arg(&notepad).is_none());
    }

    #[test]
    fn chromium_renderer_accessibility_respects_caller_flag_and_truthy_env_values() {
        let mut caller = launch_params(
            "msedge.exe",
            vec!["--force-renderer-accessibility", "https://example.com"],
            10_000,
        );
        caller.force_renderer_accessibility = Some(true);
        assert!(
            chromium_renderer_accessibility_arg(&caller).is_none(),
            "caller-supplied flag must not be duplicated"
        );

        caller.args[0] = "--force-renderer-accessibility=complete".to_owned();
        assert!(
            chromium_renderer_accessibility_arg(&caller).is_none(),
            "caller-supplied valued flag must not be duplicated"
        );

        for value in ["1", "true", "yes", "on", " TRUE "] {
            assert!(truthy_value(value), "{value:?} should enable env opt-in");
        }
        for value in ["", "0", "false", "off", "no", "maybe"] {
            assert!(
                !truthy_value(value),
                "{value:?} should not enable env opt-in"
            );
        }
    }

    #[test]
    fn read_devtools_active_port_parses_first_line() {
        let dir = std::env::temp_dir().join(format!(
            "synapse-cdp-test-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let port_file = dir.join("DevToolsActivePort");
        std::fs::write(&port_file, "51234\n/devtools/browser/abc-123\n").expect("write port file");
        let port = read_devtools_active_port(&port_file);
        println!("readback=devtools_active_port before=file:{port_file:?} after=port:{port:?}");
        assert_eq!(port, Some(51234));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn launch_requested_url_detects_browser_page_args() {
        let args = vec![
            "--new-window".to_owned(),
            "http://localhost:5173/polis".to_owned(),
        ];
        let url = launch_requested_url(&args);
        println!(
            "readback=act_launch_url edge=wsl_localhost_arg before=args:{args:?} after={url:?}"
        );
        assert_eq!(url.as_deref(), Some("http://localhost:5173/polis"));

        let app_args = vec!["--app=https://example.test/app".to_owned()];
        assert_eq!(
            launch_requested_url(&app_args).as_deref(),
            Some("https://example.test/app")
        );

        let non_url_args = vec!["--new-window".to_owned(), "not-a-url".to_owned()];
        assert!(launch_requested_url(&non_url_args).is_none());
    }

    #[tokio::test]
    async fn launch_url_verification_skips_when_synapse_did_not_open_cdp() {
        let mut opted_out = launch_params("chrome.exe", vec!["http://localhost:5173"], 10);
        opted_out.cdp_debug = Some(false);
        let result =
            verify_launched_chromium_url(&opted_out, None, &LaunchedCdp::default(), 10).await;
        println!(
            "readback=act_launch_url edge=cdp_opt_out before=cdp_launch:None after={result:?}"
        );
        assert!(matches!(result, Ok(None)));

        let non_chromium = launch_params("notepad.exe", vec!["http://localhost:5173"], 10);
        let result =
            verify_launched_chromium_url(&non_chromium, None, &LaunchedCdp::default(), 10).await;
        println!(
            "readback=act_launch_url edge=non_chromium before=cdp_launch:None after={result:?}"
        );
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn launch_url_matching_normalizes_root_trailing_slash() {
        assert!(url_matches(
            "http://localhost:5173",
            "http://localhost:5173/"
        ));
        assert!(url_matches(
            "https://example.test/path?q=1",
            "https://example.test/path?q=1"
        ));
        assert!(!url_matches(
            "http://localhost:5173/expected",
            "http://localhost:5173/other"
        ));
    }

    fn combo_press_step(at_ms: u32, key: &str) -> ActComboStep {
        ActComboStep {
            at_ms,
            action: ActComboAction::ActPress,
            params: json!({
                "keys": [key],
                "hold_ms": 1,
                "backend": "software",
            }),
            backend: None,
        }
    }

    fn combo_params(steps: Vec<ActComboStep>) -> ActComboParams {
        ActComboParams {
            steps,
            backend: Backend::Software,
            idempotency_key: None,
        }
    }

    fn assert_tool_params_invalid(error: &ErrorData) {
        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("code"))
                .and_then(|code| code.as_str()),
            Some(error_codes::TOOL_PARAMS_INVALID)
        );
    }

    #[test]
    fn combo_rejects_empty_steps() {
        let error = match validate_combo_params(&combo_params(Vec::new())) {
            Ok(()) => panic!("empty combo should reject"),
            Err(error) => error,
        };

        assert_tool_params_invalid(&error);
        assert!(
            error
                .message
                .contains("steps must contain at least one step")
        );
    }

    #[test]
    fn combo_rejects_more_than_256_steps() {
        let steps = (0..=MAX_COMBO_STEPS)
            .map(|index| combo_press_step(u32::try_from(index).unwrap_or(u32::MAX), "f13"))
            .collect::<Vec<_>>();
        let error = match validate_combo_params(&combo_params(steps)) {
            Ok(()) => panic!("257-step combo should reject"),
            Err(error) => error,
        };

        assert_tool_params_invalid(&error);
        assert!(error.message.contains("exceeds max 256"));
    }

    #[test]
    fn combo_rejects_non_monotonic_steps() {
        let error = match validate_combo_params(&combo_params(vec![
            combo_press_step(10, "f14"),
            combo_press_step(9, "f15"),
        ])) {
            Ok(()) => panic!("non-monotonic combo should reject"),
            Err(error) => error,
        };

        assert_tool_params_invalid(&error);
        assert!(error.message.contains("at_ms must be monotonic"));
    }

    #[test]
    fn combo_rejects_motion_action_with_act_stroke_pointer() {
        let params = combo_params(vec![ActComboStep {
            at_ms: 0,
            action: ActComboAction::Retired("act_drag".to_owned()),
            params: json!({"path": [{"x": 0, "y": 0}, {"x": 10, "y": 0}]}),
            backend: None,
        }]);
        let error = match combo_steps_from_params(&params) {
            Ok(steps) => panic!("motion combo action should reject, got {steps:?}"),
            Err(error) => error,
        };

        assert_tool_params_invalid(&error);
        assert!(error.message.contains("act_drag"));
        assert!(error.message.contains("not combo-lowerable"));
        assert!(error.message.contains("Use act_stroke"));
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
    fn launch_window_selection_prefers_new_matching_window() {
        let contexts = vec![
            foreground_for_launch_selection(10, 100, "chrome.exe", "Google Chrome"),
            foreground_for_launch_selection(11, 200, "chrome.exe", "Google Chrome"),
        ];
        let excluded = HashSet::from([10]);
        let title_regex = regex::Regex::new("Chrome|Chromium").expect("synthetic regex compiles");

        let selected =
            select_launch_window(&contexts, 999, &title_regex, &excluded, "chrome.exe", &[])
                .expect("new matching window should be selected");

        assert_eq!(selected.hwnd, 11);
    }

    #[test]
    fn launch_window_selection_accepts_existing_single_instance_window() {
        let contexts = vec![foreground_for_launch_selection(
            10,
            100,
            "chrome.exe",
            "Google Chrome",
        )];
        let excluded = HashSet::from([10]);
        let title_regex = regex::Regex::new("Chrome|Chromium").expect("synthetic regex compiles");

        let selected =
            select_launch_window(&contexts, 999, &title_regex, &excluded, "chrome.exe", &[])
                .expect("existing single-instance matching window should be selected");

        assert_eq!(selected.hwnd, 10);
    }

    #[test]
    fn launch_window_selection_rejects_unrelated_existing_window() {
        let contexts = vec![foreground_for_launch_selection(
            10,
            100,
            "WindowsTerminal.exe",
            "Synapse - Windows Terminal",
        )];
        let excluded = HashSet::from([10]);
        let title_regex = regex::Regex::new("Synapse|Explorer").expect("synthetic regex compiles");

        let selected =
            select_launch_window(&contexts, 999, &title_regex, &excluded, "explorer.exe", &[]);

        assert!(
            selected.is_none(),
            "unrelated existing windows must not satisfy broad launch title regexes"
        );
    }

    #[test]
    fn launch_window_selection_accepts_known_shell_activation_window() {
        let contexts = vec![foreground_for_launch_selection(
            10,
            100,
            "ApplicationFrameHost.exe",
            "Settings",
        )];
        let excluded = HashSet::from([10]);
        let title_regex =
            regex::Regex::new("^(Settings|Control Panel)$").expect("synthetic regex compiles");
        let launch_args = vec!["ms-settings:".to_owned()];

        let selected = select_launch_window(
            &contexts,
            999,
            &title_regex,
            &excluded,
            "explorer.exe",
            &launch_args,
        )
        .expect("known shell-activated app window should be accepted");

        assert_eq!(selected.hwnd, 10);
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
    async fn launch_wait_fails_when_window_does_not_match() {
        let mut params = launch_params("cmd.exe", vec!["/c", "exit 0"], 50);
        params.wait_for_window_title_regex = Some("^SynapseLaunchNoSuchWindow$".to_owned());
        let config = launch_config_for(&params);

        let error = match launch(&config, params).await {
            Ok(response) => panic!("window verification should fail closed: {response:?}"),
            Err(error) => error,
        };

        println!(
            "readback=act_launch_window_wait edge=no_match before=regex:^SynapseLaunchNoSuchWindow$ after=error:{error}"
        );
        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("code"))
                .and_then(|code| code.as_str()),
            Some(error_codes::ACTION_LAUNCH_WINDOW_NOT_FOUND)
        );
        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("reason"))
                .and_then(|reason| reason.as_str()),
            Some("no_match_within_timeout")
        );
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
    fn launch_rejects_timeout_outside_schema_bounds() {
        for timeout_ms in [0, MAX_LAUNCH_TIMEOUT_MS + 1] {
            let params = launch_params("notepad.exe", Vec::new(), timeout_ms);

            let error = match validate_launch_params(&params) {
                Ok(()) => panic!("timeout {timeout_ms} should reject"),
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
    }

    #[test]
    fn launch_process_history_row_records_spawn_without_env_values() {
        let mut params = launch_params("notepad.exe", vec!["C:\\tmp\\launch.txt"], 10_000);
        params.env.insert(
            "SYNAPSE_LAUNCH_SECRET".to_owned(),
            "do-not-store".to_owned(),
        );
        let response = ActLaunchResponse {
            pid: 1234,
            hwnd: Some(5678),
            matched_title: Some("launch.txt - Notepad".to_owned()),
            launched_at: "2026-05-31T20:00:00Z".to_owned(),
            reason: None,
            cdp_debug_port: None,
            cdp_endpoint: None,
            cdp_user_data_dir: None,
            cdp_verified_url: None,
            cdp_verified_title: None,
        };

        let row = launch_process_history_row(&params, &response)
            .unwrap_or_else(|error| panic!("process history row should encode: {error}"));
        let value: serde_json::Value = serde_json::from_slice(&row)
            .unwrap_or_else(|error| panic!("process history row should decode: {error}"));

        assert_eq!(value["tool"], "act_launch");
        assert_eq!(value["pid"], 1234);
        assert_eq!(value["hwnd"], 5678);
        assert_eq!(value["matched_title"], "launch.txt - Notepad");
        assert_eq!(value["env_keys"], json!(["SYNAPSE_LAUNCH_SECRET"]));
        assert_eq!(value["cdp_debug"], serde_json::Value::Null);
        assert_eq!(value["cdp_debug_port"], serde_json::Value::Null);
        assert_eq!(value["cdp_endpoint"], serde_json::Value::Null);
        assert_eq!(value["cdp_user_data_dir"], serde_json::Value::Null);
        assert_eq!(value["cdp_verified_url"], serde_json::Value::Null);
        assert_eq!(value["cdp_verified_title"], serde_json::Value::Null);
        assert!(!String::from_utf8_lossy(&row).contains("do-not-store"));
        assert!(
            String::from_utf8_lossy(&launch_process_history_row_key(&response)).contains("1234")
        );
    }

    #[test]
    fn launch_process_history_row_records_cdp_launch_fields() {
        let mut params = launch_params("chrome.exe", vec!["https://example.test"], 10_000);
        params.cdp_debug = Some(true);
        let response = ActLaunchResponse {
            pid: 2222,
            hwnd: Some(3333),
            matched_title: Some("Synthetic CDP Page - Google Chrome".to_owned()),
            launched_at: "2026-06-03T23:00:00Z".to_owned(),
            reason: None,
            cdp_debug_port: Some(45678),
            cdp_endpoint: Some("http://127.0.0.1:45678".to_owned()),
            cdp_user_data_dir: Some("C:\\Temp\\synapse-cdp-profiles\\synthetic".to_owned()),
            cdp_verified_url: Some("https://example.test/".to_owned()),
            cdp_verified_title: Some("Synthetic CDP Page".to_owned()),
        };

        let row = launch_process_history_row(&params, &response)
            .unwrap_or_else(|error| panic!("process history row should encode: {error}"));
        let value: serde_json::Value = serde_json::from_slice(&row)
            .unwrap_or_else(|error| panic!("process history row should decode: {error}"));

        println!(
            "readback=act_launch_history_cdp before=port:{:?} after=row_port:{} endpoint:{}",
            response.cdp_debug_port, value["cdp_debug_port"], value["cdp_endpoint"]
        );
        assert_eq!(value["cdp_debug"], true);
        assert_eq!(value["cdp_debug_port"], 45678);
        assert_eq!(value["cdp_endpoint"], "http://127.0.0.1:45678");
        assert_eq!(
            value["cdp_user_data_dir"],
            "C:\\Temp\\synapse-cdp-profiles\\synthetic"
        );
        assert_eq!(value["cdp_verified_url"], "https://example.test/");
        assert_eq!(value["cdp_verified_title"], "Synthetic CDP Page");
    }

    #[test]
    fn launch_console_targets_request_real_console_windows() {
        for target in [
            "cmd.exe",
            "C:\\Windows\\System32\\cmd.exe",
            "powershell.exe",
            "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
        ] {
            assert!(
                launch_target_needs_new_console(target),
                "{target} should request CREATE_NEW_CONSOLE on Windows"
            );
        }

        for target in ["notepad.exe", "wt.exe", "WindowsTerminal.exe"] {
            assert!(
                !launch_target_needs_new_console(target),
                "{target} should use normal GUI launch stdio handling"
            );
        }
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
