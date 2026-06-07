use std::time::{Duration, Instant};

use synapse_action::{
    ActionError, OperatorHotkeyGuard, OperatorHotkeyStatus, RELEASE_ALL_HANDLE,
    set_operator_hotkey_status,
};
use synapse_core::error_codes;

use crate::m3::SharedM3State;

pub const DISABLE_OPERATOR_HOTKEY_ENV: &str = "SYNAPSE_MCP_DISABLE_OPERATOR_HOTKEY";
/// When set truthy, a failure to register the operator panic hotkey aborts
/// startup instead of degrading. Off by default so a leaked/duplicate instance
/// holding the global hotkey cannot brick the MCP server (the failure surfaced
/// as JSON-RPC `-32000` to clients and broke the editor-wired stdio child).
pub const REQUIRE_OPERATOR_HOTKEY_ENV: &str = "SYNAPSE_MCP_REQUIRE_OPERATOR_HOTKEY";

/// Operator-facing remediation for an unavailable panic hotkey.
const OPERATOR_HOTKEY_REMEDIATION: &str = "another process already owns Ctrl+Alt+Shift+P (most often a leaked or duplicate synapse-mcp instance); stop the other instance to arm the kill-switch, set SYNAPSE_MCP_DISABLE_OPERATOR_HOTKEY=1 to run intentionally without it, or set SYNAPSE_MCP_REQUIRE_OPERATOR_HOTKEY=1 to make this a hard startup failure";
const OPERATOR_RELEASE_ALL_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Debug)]
struct DisableReport {
    result: &'static str,
    disabled_ids: Vec<String>,
    error_code: Option<&'static str>,
    detail: Option<String>,
}

#[derive(Debug)]
struct ReleaseAllReport {
    result: &'static str,
    error_code: Option<&'static str>,
    detail: Option<String>,
}

pub fn install_operator_hotkey(
    m3_state: SharedM3State,
) -> synapse_action::ActionResult<Option<OperatorHotkeyGuard>> {
    if operator_hotkey_disabled_by_env()? {
        tracing::warn!(
            code = "SAFETY_OPERATOR_HOTKEY_DISABLED",
            env = DISABLE_OPERATOR_HOTKEY_ENV,
            "operator hotkey disabled by explicit environment override"
        );
        set_operator_hotkey_status(OperatorHotkeyStatus::DisabledByEnv);
        return Ok(None);
    }
    match synapse_action::install_operator_hotkey(move || handle_operator_hotkey(&m3_state)) {
        Ok(guard) => {
            set_operator_hotkey_status(OperatorHotkeyStatus::Registered);
            Ok(Some(guard))
        }
        Err(error) => {
            set_operator_hotkey_status(OperatorHotkeyStatus::Unavailable);
            if operator_hotkey_required_by_env()? {
                // Strict mode: caller propagates and startup fails closed.
                return Err(error);
            }
            // Default: do NOT abort the whole MCP server because the global
            // kill-switch could not bind. Log loudly with exact cause and
            // remediation, record degraded status for /health, and continue so
            // the (mostly read-only) tool surface stays usable. Input-emitting
            // tools remain guarded by their own preflight/consent paths.
            tracing::error!(
                code = error_codes::ACTION_BACKEND_UNAVAILABLE,
                component = "operator_hotkey",
                hotkey = "ctrl+alt+shift+p",
                status = OperatorHotkeyStatus::Unavailable.label(),
                error = %error,
                remediation = OPERATOR_HOTKEY_REMEDIATION,
                require_env = REQUIRE_OPERATOR_HOTKEY_ENV,
                disable_env = DISABLE_OPERATOR_HOTKEY_ENV,
                "operator panic hotkey unavailable; continuing in degraded safety mode without the kill-switch"
            );
            Ok(None)
        }
    }
}

fn operator_hotkey_required_by_env() -> synapse_action::ActionResult<bool> {
    parse_bool_env(REQUIRE_OPERATOR_HOTKEY_ENV)
}

fn operator_hotkey_disabled_by_env() -> synapse_action::ActionResult<bool> {
    parse_bool_env(DISABLE_OPERATOR_HOTKEY_ENV)
}

fn parse_bool_env(name: &str) -> synapse_action::ActionResult<bool> {
    let Some(raw) = std::env::var_os(name) else {
        return Ok(false);
    };
    let value = raw.to_string_lossy();
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "" | "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ActionError::BackendUnavailable {
            detail: format!("{name} must be one of 1/true/yes/on or 0/false/no/off"),
        }),
    }
}

fn handle_operator_hotkey(m3_state: &SharedM3State) {
    let started = Instant::now();
    let preempted_lease = synapse_action::force_preempt_input_lease("operator_hotkey");
    let disable_report = disable_reflexes(m3_state);
    let release_all_report = fire_release_all();
    let elapsed = started.elapsed();
    tracing::warn!(
        code = error_codes::SAFETY_OPERATOR_HOTKEY_FIRED,
        hotkey = "ctrl+alt+shift+p",
        input_lease_preempted = preempted_lease.is_some(),
        input_lease_prior_owner = ?preempted_lease
            .as_ref()
            .and_then(|status| status.owner_session_id.clone()),
        input_lease_operator_owner = synapse_action::OPERATOR_LEASE_OWNER_SESSION_ID,
        input_lease_operator_ttl_ms = synapse_action::OPERATOR_PREEMPT_LEASE_TTL_MS,
        disabled_reflexes = disable_report.disabled_ids.len(),
        disabled_reflex_ids = ?disable_report.disabled_ids,
        reflex_result = disable_report.result,
        reflex_error_code = ?disable_report.error_code,
        reflex_detail = ?disable_report.detail,
        release_all_result = release_all_report.result,
        release_all_error_code = ?release_all_report.error_code,
        release_all_detail = ?release_all_report.detail,
        elapsed_ms = elapsed.as_millis(),
        within_budget = elapsed <= OPERATOR_RELEASE_ALL_TIMEOUT,
        "operator hotkey fired release_all and disabled reflexes"
    );
}

fn disable_reflexes(m3_state: &SharedM3State) -> DisableReport {
    let runtime = match m3_state.lock() {
        Ok(state) => state.reflex_runtime.clone(),
        Err(_err) => {
            return DisableReport {
                result: "error",
                disabled_ids: Vec::new(),
                error_code: Some(error_codes::TOOL_INTERNAL_ERROR),
                detail: Some("M3 service state lock poisoned".to_owned()),
            };
        }
    };
    let Some(runtime) = runtime else {
        return DisableReport {
            result: "not_initialized",
            disabled_ids: Vec::new(),
            error_code: None,
            detail: None,
        };
    };
    let mut runtime = match runtime.lock() {
        Ok(runtime) => runtime,
        Err(_err) => {
            return DisableReport {
                result: "error",
                disabled_ids: Vec::new(),
                error_code: Some(error_codes::TOOL_INTERNAL_ERROR),
                detail: Some("reflex runtime lock poisoned".to_owned()),
            };
        }
    };
    match runtime.disable_all_by_operator() {
        Ok(disabled) => DisableReport {
            result: "ok",
            disabled_ids: disabled.into_iter().map(|status| status.id).collect(),
            error_code: None,
            detail: None,
        },
        Err(error) => DisableReport {
            result: "error",
            disabled_ids: Vec::new(),
            error_code: Some(error.code()),
            detail: Some(error.to_string()),
        },
    }
}

fn fire_release_all() -> ReleaseAllReport {
    let Some(handle) = RELEASE_ALL_HANDLE.get() else {
        return ReleaseAllReport {
            result: "missing_handle",
            error_code: Some(error_codes::ACTION_BACKEND_UNAVAILABLE),
            detail: Some("RELEASE_ALL_HANDLE is not initialized".to_owned()),
        };
    };
    match handle.fire_release_all_blocking_with_timeout(OPERATOR_RELEASE_ALL_TIMEOUT) {
        Ok(()) => ReleaseAllReport {
            result: "ok",
            error_code: None,
            detail: None,
        },
        Err(error) => ReleaseAllReport {
            result: "error",
            error_code: Some(error.code()),
            detail: Some(error.to_string()),
        },
    }
}
