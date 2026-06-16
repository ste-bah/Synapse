use std::{
    env,
    path::{Path, PathBuf},
};

use rmcp::{ErrorData, model::ErrorCode};
use serde::Serialize;
use serde_json::{Map, Value, json};
use synapse_core::{ForegroundContext, Profile, ProfileId, error_codes};
use synapse_profiles::ProfileRuntime;

use super::SynapseService;
use super::everquest_ui_context::{
    EverQuestUiContextReadback, deny_login_screen_action, everquest_ui_context_from_input,
};
use crate::m1::{current_input, mcp_error};

const EVERQUEST_PROFILE_ID: &str = "everquest.live";
const KEY_RUNTIME_EVERQUEST_EXE: &str = "runtime.everquest.exe";
const EQGAME_EXE: &str = "eqgame.exe";

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActionPreflightReadback {
    pub tool: &'static str,
    pub target_profile_id: Option<ProfileId>,
    pub active_profile_id_before: Option<ProfileId>,
    pub applied: bool,
    pub status: &'static str,
    pub target: Option<EverQuestFocusTarget>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub everquest_ui_context: Option<EverQuestUiContextReadback>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EverQuestFocusTarget {
    pub profile_id: ProfileId,
    pub process_name: &'static str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_path: Option<String>,
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

struct EverQuestPreflightTargetState {
    profile_id: ProfileId,
    target: EverQuestFocusTarget,
    target_path: Option<PathBuf>,
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
        if active_profile_id_before.as_deref() != Some(EVERQUEST_PROFILE_ID) {
            return Ok((
                foreground,
                not_applicable_preflight(tool, active_profile_id_before, before),
            ));
        }

        let profile = runtime
            .profile(EVERQUEST_PROFILE_ID)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?
            .ok_or_else(|| {
                everquest_preflight_error_without_readback(
                    tool,
                    "everquest_profile_missing",
                    format!("active profile {EVERQUEST_PROFILE_ID} was not found"),
                    active_profile_id_before.clone(),
                    before.clone(),
                    error_codes::ACTION_TARGET_INVALID,
                )
            })?;
        let target_path = everquest_target_exe(&profile);
        let profile_id = profile.id;
        let target = EverQuestPreflightTargetState {
            profile_id: profile_id.clone(),
            target: EverQuestFocusTarget {
                profile_id,
                process_name: EQGAME_EXE,
                process_path: target_path.as_ref().map(|path| path.display().to_string()),
            },
            target_path,
        };

        if everquest_foreground_is_verified(&foreground, &before, target.target_path.as_deref()) {
            let EverQuestPreflightTargetState {
                profile_id, target, ..
            } = target;
            return Ok((
                foreground,
                verified_everquest_preflight(
                    tool,
                    active_profile_id_before,
                    profile_id,
                    target,
                    before,
                ),
            ));
        }

        self.refocus_everquest_foreground(tool, runtime, active_profile_id_before, before, &target)
    }

    fn refocus_everquest_foreground(
        &self,
        tool: &'static str,
        runtime: &ProfileRuntime,
        active_profile_id_before: Option<ProfileId>,
        before: ForegroundProof,
        target: &EverQuestPreflightTargetState,
    ) -> Result<(ForegroundContext, ActionPreflightReadback), ErrorData> {
        let contexts = match synapse_a11y::visible_top_level_window_contexts() {
            Ok(contexts) => contexts,
            Err(error) => {
                return Err(window_enumeration_failed_error(
                    tool,
                    active_profile_id_before,
                    before,
                    target,
                    &error,
                ));
            }
        };
        let candidate_count = contexts.len();
        let Some(candidate) =
            select_everquest_focus_candidate(&contexts, target.target_path.as_deref())
        else {
            return Err(target_window_missing_error(
                tool,
                active_profile_id_before,
                before,
                target,
                candidate_count,
            ));
        };

        let focus_hwnd = candidate.hwnd;
        let focus_error = Some(format!(
            "{}: implicit EverQuest preflight foreground activation refused for hwnd 0x{focus_hwnd:x}; use an explicit lease-held foreground action instead",
            error_codes::FOREGROUND_ACTIVATION_REFUSED
        ));
        tracing::warn!(
            code = error_codes::FOREGROUND_ACTIVATION_REFUSED,
            tool,
            focus_hwnd,
            "EverQuest action preflight refused implicit foreground activation"
        );

        let after_result = self.read_current_action_foreground();
        let after = after_result
            .as_ref()
            .ok()
            .map(|foreground| foreground_proof(runtime, foreground));
        let mut preflight = refocus_attempt_preflight(
            RefocusAttemptPreflightInput {
                tool,
                active_profile_id_before,
                before,
                candidate_count,
                focus_hwnd,
                focus_error,
                after,
                readback_error: after_result.as_ref().err().map(ToString::to_string),
            },
            target,
        );

        let after_foreground = match after_result {
            Ok(after_foreground) => after_foreground,
            Err(error) => {
                preflight.status = "post_focus_readback_failed";
                return Err(attach_action_preflight_to_error(&error, &preflight));
            }
        };
        let after_proof = preflight.after.as_ref();
        if after_proof.is_some_and(|proof| {
            everquest_foreground_is_verified(
                &after_foreground,
                proof,
                target.target_path.as_deref(),
            )
        }) {
            preflight.status = "refocused_and_verified";
            return Ok((after_foreground, preflight));
        }

        preflight.status = post_focus_failure_status(&after_foreground, target);
        Err(everquest_preflight_error(
            error_codes::ACTION_FOREGROUND_LOST,
            "everquest_foreground_not_restored",
            format!(
                "could not restore {EVERQUEST_PROFILE_ID} foreground before {tool}; current foreground is {} ({})",
                after_foreground.process_name, after_foreground.window_title
            ),
            &preflight,
        ))
    }

    fn read_current_action_foreground(&self) -> Result<ForegroundContext, ErrorData> {
        self.current_audit_foreground()
    }

    pub(super) fn ensure_everquest_live_ui_context_for_action(
        &self,
        tool: &'static str,
        mut preflight: ActionPreflightReadback,
    ) -> Result<ActionPreflightReadback, ErrorData> {
        let runtime = self.profile_runtime()?;
        let active_profile_id = runtime
            .active_profile_id()
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        let applies_to_everquest = preflight.target_profile_id.as_deref()
            == Some(EVERQUEST_PROFILE_ID)
            || active_profile_id.as_deref() == Some(EVERQUEST_PROFILE_ID);
        if !applies_to_everquest {
            return Ok(preflight);
        }
        if preflight.target_profile_id.is_none() {
            preflight.target_profile_id = Some(EVERQUEST_PROFILE_ID.to_owned());
        }
        let mut input = {
            let state = self.m1_state()?;
            current_input(&state, 2)?
        };
        self.resolve_input_profile_and_hud(&mut input, true);
        let ui_context = everquest_ui_context_from_input(&input);
        preflight.everquest_ui_context = Some(ui_context.clone());
        if ui_context.login_screen_visible {
            return Err(deny_login_screen_action(tool, &preflight));
        }
        Ok(preflight)
    }
}

fn window_enumeration_failed_error(
    tool: &'static str,
    active_profile_id_before: Option<ProfileId>,
    before: ForegroundProof,
    target: &EverQuestPreflightTargetState,
    error: &synapse_a11y::A11yError,
) -> ErrorData {
    let preflight = ActionPreflightReadback {
        tool,
        target_profile_id: Some(target.profile_id.clone()),
        active_profile_id_before,
        applied: true,
        status: "window_enumeration_failed",
        target: Some(target.target.clone()),
        before,
        candidate_count: None,
        focus_attempted: false,
        focus_hwnd: None,
        focus_error: Some(error.to_string()),
        after: None,
        readback_error: None,
        everquest_ui_context: None,
    };
    everquest_preflight_error(
        error_codes::ACTION_TARGET_INVALID,
        "everquest_window_enumeration_failed",
        format!("could not enumerate visible windows before {tool}: {error}"),
        &preflight,
    )
}

fn target_window_missing_error(
    tool: &'static str,
    active_profile_id_before: Option<ProfileId>,
    before: ForegroundProof,
    target: &EverQuestPreflightTargetState,
    candidate_count: usize,
) -> ErrorData {
    let preflight = ActionPreflightReadback {
        tool,
        target_profile_id: Some(target.profile_id.clone()),
        active_profile_id_before,
        applied: true,
        status: "target_window_missing",
        target: Some(target.target.clone()),
        before,
        candidate_count: Some(candidate_count),
        focus_attempted: false,
        focus_hwnd: None,
        focus_error: None,
        after: None,
        readback_error: None,
        everquest_ui_context: None,
    };
    everquest_preflight_error(
        error_codes::ACTION_TARGET_INVALID,
        "everquest_window_missing",
        format!("could not find a visible {EVERQUEST_PROFILE_ID} window before {tool}"),
        &preflight,
    )
}

struct RefocusAttemptPreflightInput {
    tool: &'static str,
    active_profile_id_before: Option<ProfileId>,
    before: ForegroundProof,
    candidate_count: usize,
    focus_hwnd: i64,
    focus_error: Option<String>,
    after: Option<ForegroundProof>,
    readback_error: Option<String>,
}

fn refocus_attempt_preflight(
    input: RefocusAttemptPreflightInput,
    target: &EverQuestPreflightTargetState,
) -> ActionPreflightReadback {
    ActionPreflightReadback {
        tool: input.tool,
        target_profile_id: Some(target.profile_id.clone()),
        active_profile_id_before: input.active_profile_id_before,
        applied: true,
        status: "refocus_attempted",
        target: Some(target.target.clone()),
        before: input.before,
        candidate_count: Some(input.candidate_count),
        focus_attempted: true,
        focus_hwnd: Some(input.focus_hwnd),
        focus_error: input.focus_error,
        after: input.after,
        readback_error: input.readback_error,
        everquest_ui_context: None,
    }
}

fn post_focus_failure_status(
    after_foreground: &ForegroundContext,
    target: &EverQuestPreflightTargetState,
) -> &'static str {
    if foreground_matches_everquest_target(after_foreground, target.target_path.as_deref()) {
        "post_focus_still_minimized_or_unknown"
    } else {
        "post_focus_readback_mismatch"
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
        target: None,
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
        everquest_ui_context: None,
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
        target: None,
        before: before.clone(),
        candidate_count: None,
        focus_attempted: false,
        focus_hwnd: None,
        focus_error: None,
        after: Some(before),
        readback_error: None,
        everquest_ui_context: None,
    }
}

fn verified_everquest_preflight(
    tool: &'static str,
    active_profile_id_before: Option<ProfileId>,
    profile_id: ProfileId,
    target: EverQuestFocusTarget,
    before: ForegroundProof,
) -> ActionPreflightReadback {
    ActionPreflightReadback {
        tool,
        target_profile_id: Some(profile_id),
        active_profile_id_before,
        applied: true,
        status: "verified_foreground",
        target: Some(target),
        before: before.clone(),
        candidate_count: None,
        focus_attempted: false,
        focus_hwnd: None,
        focus_error: None,
        after: Some(before),
        readback_error: None,
        everquest_ui_context: None,
    }
}

fn everquest_preflight_error_without_readback(
    tool: &'static str,
    reason: &'static str,
    message: String,
    active_profile_id_before: Option<ProfileId>,
    before: ForegroundProof,
    code: &'static str,
) -> ErrorData {
    let preflight = ActionPreflightReadback {
        tool,
        target_profile_id: Some(EVERQUEST_PROFILE_ID.to_owned()),
        active_profile_id_before,
        applied: true,
        status: reason,
        target: None,
        before,
        candidate_count: None,
        focus_attempted: false,
        focus_hwnd: None,
        focus_error: None,
        after: None,
        readback_error: None,
        everquest_ui_context: None,
    };
    everquest_preflight_error(code, reason, message, &preflight)
}

fn everquest_preflight_error(
    code: &'static str,
    reason: &'static str,
    message: String,
    preflight: &ActionPreflightReadback,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        message,
        Some(json!({
            "code": code,
            "reason": reason,
            "action_preflight": preflight,
        })),
    )
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

fn everquest_target_exe(profile: &Profile) -> Option<PathBuf> {
    profile
        .metadata
        .get(KEY_RUNTIME_EVERQUEST_EXE)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(expand_percent_env)
        .map(PathBuf::from)
}

fn select_everquest_focus_candidate<'a>(
    contexts: &'a [ForegroundContext],
    target_path: Option<&Path>,
) -> Option<&'a ForegroundContext> {
    contexts
        .iter()
        .filter_map(|context| {
            everquest_focus_score(context, target_path).map(|score| (score, context))
        })
        .min_by_key(|(score, context)| (*score, context.hwnd))
        .map(|(_score, context)| context)
}

fn everquest_focus_score(context: &ForegroundContext, target_path: Option<&Path>) -> Option<u8> {
    if let Some(target_path) = target_path {
        return same_path_text(target_path, Path::new(&context.process_path)).then_some(0);
    }
    if context.process_name.eq_ignore_ascii_case(EQGAME_EXE)
        && context
            .window_title
            .to_ascii_lowercase()
            .contains("everquest")
    {
        return Some(1);
    }
    context
        .process_name
        .eq_ignore_ascii_case(EQGAME_EXE)
        .then_some(2)
}

fn foreground_matches_everquest_target(
    foreground: &ForegroundContext,
    target_path: Option<&Path>,
) -> bool {
    if let Some(target_path) = target_path {
        return same_path_text(target_path, Path::new(&foreground.process_path));
    }
    foreground.process_name.eq_ignore_ascii_case(EQGAME_EXE)
        && foreground
            .window_title
            .to_ascii_lowercase()
            .contains("everquest")
}

fn everquest_foreground_is_verified(
    foreground: &ForegroundContext,
    proof: &ForegroundProof,
    target_path: Option<&Path>,
) -> bool {
    foreground_matches_everquest_target(foreground, target_path)
        && proof.is_minimized == Some(false)
}

fn same_path_text(left: &Path, right: &Path) -> bool {
    normalize_path_text(left) == normalize_path_text(right)
}

fn normalize_path_text(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn expand_percent_env(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('%') else {
            out.push('%');
            out.push_str(after_start);
            return out;
        };
        let name = &after_start[..end];
        if name.is_empty() {
            out.push_str("%%");
        } else if let Ok(value) = env::var(name) {
            out.push_str(&value);
        } else {
            out.push('%');
            out.push_str(name);
            out.push('%');
        }
        rest = &after_start[end + 1..];
    }
    out.push_str(rest);
    out
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use synapse_core::Rect;

    use super::*;

    #[test]
    fn everquest_candidate_prefers_exact_configured_path() {
        let expected = PathBuf::from(
            r"C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\eqgame.exe",
        );
        let wrong = foreground(
            200,
            "eqgame.exe",
            r"C:\Other\EverQuest\eqgame.exe",
            "EverQuest",
        );
        let right = foreground(
            100,
            "eqgame.exe",
            &expected.display().to_string(),
            "EverQuest",
        );
        let contexts = vec![wrong, right];

        let selected = select_everquest_focus_candidate(&contexts, Some(&expected))
            .expect("exact-path EverQuest candidate should be selected");

        assert_eq!(selected.hwnd, 100);
    }

    #[test]
    fn everquest_candidate_refuses_wrong_path_when_configured() {
        let expected = PathBuf::from(
            r"C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\eqgame.exe",
        );
        let contexts = vec![foreground(
            200,
            "eqgame.exe",
            r"C:\Other\EverQuest\eqgame.exe",
            "EverQuest",
        )];

        assert!(select_everquest_focus_candidate(&contexts, Some(&expected)).is_none());
    }

    #[test]
    fn everquest_candidate_can_fallback_to_process_and_title_without_configured_path() {
        let contexts = vec![
            foreground(300, "notepad.exe", r"C:\Windows\notepad.exe", "Notes"),
            foreground(100, "eqgame.exe", r"C:\EverQuest\eqgame.exe", "EverQuest"),
        ];

        let selected = select_everquest_focus_candidate(&contexts, None)
            .expect("eqgame title candidate should be selected");

        assert_eq!(selected.hwnd, 100);
    }

    #[test]
    fn everquest_foreground_verification_requires_not_minimized() {
        let expected = PathBuf::from(
            r"C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest\eqgame.exe",
        );
        let foreground = foreground(
            100,
            "eqgame.exe",
            &expected.display().to_string(),
            "EverQuest",
        );

        assert!(everquest_foreground_is_verified(
            &foreground,
            &proof_for(&foreground, Some(false)),
            Some(&expected)
        ));
        assert!(!everquest_foreground_is_verified(
            &foreground,
            &proof_for(&foreground, Some(true)),
            Some(&expected)
        ));
        assert!(!everquest_foreground_is_verified(
            &foreground,
            &proof_for(&foreground, None),
            Some(&expected)
        ));
    }

    fn foreground(
        hwnd: i64,
        process_name: &str,
        process_path: &str,
        window_title: &str,
    ) -> ForegroundContext {
        ForegroundContext {
            hwnd,
            pid: u32::try_from(hwnd).unwrap_or_default(),
            process_name: process_name.to_owned(),
            process_path: process_path.to_owned(),
            window_title: window_title.to_owned(),
            window_bounds: Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            monitor_index: 0,
            dpi_scale: 1.0,
            profile_id: None,
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
        }
    }

    fn proof_for(foreground: &ForegroundContext, is_minimized: Option<bool>) -> ForegroundProof {
        ForegroundProof {
            hwnd: foreground.hwnd,
            pid: foreground.pid,
            process_name: foreground.process_name.clone(),
            process_path: foreground.process_path.clone(),
            window_title: foreground.window_title.clone(),
            is_minimized,
            minimized_readback_error: is_minimized.is_none().then(|| "unknown".to_owned()),
            observed_profile_id: None,
        }
    }
}
