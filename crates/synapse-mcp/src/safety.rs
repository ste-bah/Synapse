use std::time::{Duration, Instant};

use synapse_action::{OperatorHotkeyGuard, RELEASE_ALL_HANDLE};
use synapse_core::error_codes;

use crate::m3::SharedM3State;

pub mod agreement;
pub mod hardware_consent;

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
) -> synapse_action::ActionResult<OperatorHotkeyGuard> {
    synapse_action::install_operator_hotkey(move || handle_operator_hotkey(&m3_state))
}

fn handle_operator_hotkey(m3_state: &SharedM3State) {
    let started = Instant::now();
    let disable_report = disable_reflexes(m3_state);
    let release_all_report = fire_release_all();
    let elapsed = started.elapsed();
    tracing::warn!(
        code = error_codes::SAFETY_OPERATOR_HOTKEY_FIRED,
        hotkey = "ctrl+alt+shift+p",
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
