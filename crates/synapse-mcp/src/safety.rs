use std::time::{Duration, Instant};

use serde::Serialize;
use synapse_action::{
    ActionError, OperatorHotkeyGuard, OperatorHotkeyStatus, RELEASE_ALL_HANDLE,
    set_operator_hotkey_status,
};
use synapse_core::error_codes;
use tokio::runtime::Handle;

use crate::m3::SharedM3State;
use crate::server::SynapseService;

pub const DISABLE_OPERATOR_HOTKEY_ENV: &str = "SYNAPSE_MCP_DISABLE_OPERATOR_HOTKEY";
/// When set truthy, a failure to register the operator panic hotkey aborts
/// startup instead of degrading. Defaults to true because the MCP daemon exposes
/// input-emitting tools; set this false only with an explicit operator decision
/// to run degraded, or set `SYNAPSE_MCP_DISABLE_OPERATOR_HOTKEY=1` to skip
/// registration intentionally.
pub const REQUIRE_OPERATOR_HOTKEY_ENV: &str = "SYNAPSE_MCP_REQUIRE_OPERATOR_HOTKEY";

/// Operator-facing remediation for an unavailable panic hotkey.
const OPERATOR_HOTKEY_REMEDIATION: &str = "the daemon-owned operator hotkey could not be armed; stop duplicate synapse-mcp instances or conflicting hook owners, set SYNAPSE_OPERATOR_HOTKEY / SYNAPSE_MCP_OPERATOR_HOTKEY to another Ctrl+Alt+Shift+<A-Z|0-9> chord, set SYNAPSE_MCP_DISABLE_OPERATOR_HOTKEY=1 to run intentionally without it, or set SYNAPSE_MCP_REQUIRE_OPERATOR_HOTKEY=0 only for an explicit degraded run";
const OPERATOR_RELEASE_ALL_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DisableReport {
    pub(crate) result: &'static str,
    pub(crate) disabled_ids: Vec<String>,
    pub(crate) error_code: Option<&'static str>,
    pub(crate) detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseAllReport {
    pub(crate) result: &'static str,
    pub(crate) error_code: Option<&'static str>,
    pub(crate) detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperatorHotkeyImmediateReport {
    pub hotkey: &'static str,
    pub lease_before: synapse_action::LeaseStatus,
    pub preempted_lease: Option<synapse_action::LeaseStatus>,
    pub lease_after_preempt: synapse_action::LeaseStatus,
    pub disable_report: DisableReport,
    pub release_all_report: ReleaseAllReport,
    pub elapsed_ms: u128,
    pub within_budget: bool,
}

pub fn install_operator_hotkey(
    service: SynapseService,
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
    let m3_state = service.m3_state_handle();
    let runtime = Handle::current();
    match synapse_action::install_operator_hotkey(move || {
        handle_operator_hotkey(&service, &m3_state, &runtime);
    }) {
        Ok(guard) => {
            set_operator_hotkey_status(OperatorHotkeyStatus::Registered);
            Ok(Some(guard))
        }
        Err(error) => {
            set_operator_hotkey_status(OperatorHotkeyStatus::Unavailable);
            if operator_hotkey_required_by_env()? {
                // Default strict mode: caller propagates and startup fails closed.
                let detail = format!("{}; {OPERATOR_HOTKEY_REMEDIATION}", error.detail());
                return Err(error.with_detail(detail));
            }
            // Explicit degraded mode: do NOT abort the whole MCP server because
            // the operator chose to run without a bound global kill-switch.
            // Log loudly with exact cause/remediation and record status for
            // /health so the risk is visible.
            tracing::error!(
                code = error_codes::ACTION_BACKEND_UNAVAILABLE,
                component = "operator_hotkey",
                hotkey = synapse_action::hotkey::DEFAULT_OPERATOR_HOTKEY,
                status = OperatorHotkeyStatus::Unavailable.label(),
                error = %error,
                remediation = OPERATOR_HOTKEY_REMEDIATION,
                require_env = REQUIRE_OPERATOR_HOTKEY_ENV,
                disable_env = DISABLE_OPERATOR_HOTKEY_ENV,
                "operator panic hotkey unavailable; continuing only because degraded hotkey mode was explicitly allowed"
            );
            Ok(None)
        }
    }
}

fn operator_hotkey_required_by_env() -> synapse_action::ActionResult<bool> {
    parse_bool_env(REQUIRE_OPERATOR_HOTKEY_ENV, true)
}

fn operator_hotkey_disabled_by_env() -> synapse_action::ActionResult<bool> {
    parse_bool_env(DISABLE_OPERATOR_HOTKEY_ENV, false)
}

fn parse_bool_env(name: &str, default: bool) -> synapse_action::ActionResult<bool> {
    let raw = std::env::var_os(name);
    parse_bool_value(
        name,
        raw.as_ref().map(|value| value.to_string_lossy()),
        default,
    )
}

fn parse_bool_value(
    name: &str,
    value: Option<std::borrow::Cow<'_, str>>,
    default: bool,
) -> synapse_action::ActionResult<bool> {
    let Some(value) = value else {
        return Ok(default);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "" | "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ActionError::BackendUnavailable {
            detail: format!("{name} must be one of 1/true/yes/on or 0/false/no/off"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_hotkey_required_defaults_to_fail_closed() {
        let required = parse_bool_value(REQUIRE_OPERATOR_HOTKEY_ENV, None, true)
            .expect("missing require env should parse");

        assert!(required);
    }

    #[test]
    fn operator_hotkey_required_can_be_explicitly_relaxed() {
        let required = parse_bool_value(
            REQUIRE_OPERATOR_HOTKEY_ENV,
            Some(std::borrow::Cow::Borrowed("0")),
            true,
        )
        .expect("false require env should parse");

        assert!(!required);
    }

    #[test]
    fn operator_hotkey_disabled_defaults_to_false() {
        let disabled = parse_bool_value(DISABLE_OPERATOR_HOTKEY_ENV, None, false)
            .expect("missing disable env should parse");

        assert!(!disabled);
    }
}

fn handle_operator_hotkey(service: &SynapseService, m3_state: &SharedM3State, runtime: &Handle) {
    let started = Instant::now();
    let lease_before = synapse_action::lease::status();
    let preempted_lease = synapse_action::force_preempt_input_lease("operator_hotkey");
    let disable_report = disable_reflexes(m3_state);
    let release_all_report = fire_release_all();
    let elapsed = started.elapsed();
    let lease_after_preempt = synapse_action::lease::status();
    let immediate = OperatorHotkeyImmediateReport {
        hotkey: synapse_action::hotkey::DEFAULT_OPERATOR_HOTKEY,
        lease_before,
        preempted_lease: preempted_lease.clone(),
        lease_after_preempt,
        disable_report: disable_report.clone(),
        release_all_report: release_all_report.clone(),
        elapsed_ms: elapsed.as_millis(),
        within_budget: elapsed <= OPERATOR_RELEASE_ALL_TIMEOUT,
    };
    tracing::warn!(
        code = error_codes::SAFETY_OPERATOR_HOTKEY_FIRED,
        hotkey = immediate.hotkey,
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
        elapsed_ms = immediate.elapsed_ms,
        within_budget = immediate.within_budget,
        "operator hotkey fired release_all, disabled reflexes, and queued K2 fleet kill"
    );
    let service = service.clone();
    let _operator_panic_task = runtime.spawn(async move {
        if let Err(error) = service.operator_panic_kill_all(immediate).await {
            tracing::error!(
                code = error_codes::SAFETY_OPERATOR_HOTKEY_FIRED,
                error_code = error
                    .data
                    .as_ref()
                    .and_then(|data| data.get("code"))
                    .and_then(serde_json::Value::as_str),
                detail = %error.message,
                "operator hotkey K2 fleet kill task failed"
            );
        }
    });
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
