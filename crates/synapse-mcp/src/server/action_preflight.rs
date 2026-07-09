use rmcp::ErrorData;
use serde::Serialize;
use serde_json::{Map, Value, json};
use synapse_core::{ForegroundContext, ProfileId};
use synapse_profiles::ProfileRuntime;

use super::SynapseService;

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActionPreflightReadback {
    pub tool: &'static str,
    pub target_profile_id: Option<ProfileId>,
    pub active_profile_id_before: Option<ProfileId>,
    pub applied: bool,
    pub status: &'static str,
    pub before: ForegroundProof,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_count: Option<usize>,
    pub focus_attempted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_hwnd: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<ForegroundProof>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readback_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ForegroundProof {
    pub hwnd: i64,
    pub pid: u32,
    pub process_name: String,
    pub process_path: String,
    pub window_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_minimized: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimized_readback_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_profile_id: Option<ProfileId>,
}

impl SynapseService {
    pub(super) fn preflight_action_foreground(
        &self,
        tool: &'static str,
        runtime: &ProfileRuntime,
        active_profile_id_before: Option<ProfileId>,
        foreground: ForegroundContext,
    ) -> Result<(ForegroundContext, ActionPreflightReadback), ErrorData> {
        let before = foreground_proof(runtime, &foreground);
        Ok((
            foreground,
            not_applicable_preflight(tool, active_profile_id_before, before),
        ))
    }
}

pub(super) fn attach_action_preflight_to_error(
    error: &ErrorData,
    preflight: &ActionPreflightReadback,
) -> ErrorData {
    let preflight = serde_json::to_value(preflight).unwrap_or_else(|serialization_error| {
        json!({
            "serialization_error": serialization_error.to_string(),
        })
    });
    let data = match error.data.clone() {
        Some(Value::Object(mut map)) => {
            map.insert("action_preflight".to_owned(), preflight);
            Value::Object(map)
        }
        Some(other) => {
            let mut map = Map::new();
            map.insert("original_data".to_owned(), other);
            map.insert("action_preflight".to_owned(), preflight);
            Value::Object(map)
        }
        None => {
            let mut map = Map::new();
            map.insert("action_preflight".to_owned(), preflight);
            Value::Object(map)
        }
    };
    ErrorData::new(error.code, error.message.to_string(), Some(data))
}

/// Whether a tool drives the OS foreground / active-target input surface and
/// therefore MUST fail closed when no foreground window exists.
///
/// Daemon robustness (#1061): the action gate reads the current foreground to
/// (a) reevaluate the active profile from the foreground window and (b) verify
/// the surface an input emitter is about to drive. For tools that emit input
/// into a window (every input `act_*` tool) that read is essential and a missing
/// foreground (locked screen, desktop focus, unattended session) must stay
/// fail-closed. But registration / spawn / shell / launch tools never touch the
/// foreground -- the foreground-derived profile reevaluation is irrelevant to
/// them -- so requiring a live foreground window only made the background daemon
/// (epic #717) unusable exactly when the operator is away.
///
/// Fail-closed is the safe default: a tool requires a live foreground unless it
/// is explicitly exempted here. Exempt tools evaluate scope against the active
/// profile with no foreground present instead of erroring `A11Y_NO_FOREGROUND`.
/// Note `act_spawn_agent` is gated under the `act_launch` tool name (see
/// `spawn_agent_journaled`), so exempting `act_launch` covers spawn too.
pub(super) fn tool_requires_live_foreground(tool: &str) -> bool {
    !matches!(
        tool,
        "reflex_register"
            | "act_run_shell"
            | "act_run_shell_start"
            | "act_run_shell_status"
            | "act_run_shell_cancel"
            | "act_launch"
    )
}

/// Preflight readback for the degraded "no foreground window" path taken by
/// non-foreground tools (see [`tool_requires_live_foreground`]). Records that
/// the action gate evaluated scope against the active profile with no
/// foreground present rather than hard-failing `A11Y_NO_FOREGROUND` (#1061).
pub(super) fn no_foreground_preflight(
    tool: &'static str,
    active_profile_id_before: Option<ProfileId>,
) -> ActionPreflightReadback {
    ActionPreflightReadback {
        tool,
        target_profile_id: None,
        active_profile_id_before,
        applied: false,
        status: "no_foreground_scope_evaluated",
        before: ForegroundProof {
            hwnd: 0,
            pid: 0,
            process_name: String::new(),
            process_path: String::new(),
            window_title: String::new(),
            is_minimized: None,
            minimized_readback_error: None,
            observed_profile_id: None,
        },
        candidate_count: None,
        focus_attempted: false,
        focus_hwnd: None,
        focus_error: None,
        after: None,
        readback_error: None,
    }
}

fn not_applicable_preflight(
    tool: &'static str,
    active_profile_id_before: Option<ProfileId>,
    before: ForegroundProof,
) -> ActionPreflightReadback {
    ActionPreflightReadback {
        tool,
        target_profile_id: None,
        active_profile_id_before,
        applied: false,
        status: "not_applicable",
        before: before.clone(),
        candidate_count: None,
        focus_attempted: false,
        focus_hwnd: None,
        focus_error: None,
        after: Some(before),
        readback_error: None,
    }
}

fn foreground_proof(runtime: &ProfileRuntime, foreground: &ForegroundContext) -> ForegroundProof {
    let (is_minimized, minimized_readback_error) =
        match synapse_a11y::is_window_minimized(foreground.hwnd) {
            Ok(is_minimized) => (Some(is_minimized), None),
            Err(error) => (None, Some(error.to_string())),
        };
    let observed_profile_id = runtime
        .resolve_foreground(&synapse_profiles::ForegroundWindow {
            exe: non_empty(&foreground.process_name),
            title: non_empty(&foreground.window_title),
            steam_appid: foreground.steam_appid,
            window_class: None,
        })
        .ok()
        .flatten()
        .map(|resolution| resolution.profile_id);
    ForegroundProof {
        hwnd: foreground.hwnd,
        pid: foreground.pid,
        process_name: foreground.process_name.clone(),
        process_path: foreground.process_path.clone(),
        window_title: foreground.window_title.clone(),
        is_minimized,
        minimized_readback_error,
        observed_profile_id,
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}
