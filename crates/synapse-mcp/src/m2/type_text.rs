use std::{sync::Arc, time::Instant};

use rmcp::ErrorData;
use rmcp::model::ErrorCode;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use synapse_action::{
    ActionBackend, ActionError, ActionHandle, EmitState, RecordedInput, RecordingBackend,
};
use synapse_core::{Action, Backend, ElementId, KeystrokeDynamics, KeystrokeNaturalParams};

use crate::m1::mcp_error;
use crate::m2::postcondition::{
    ActPostcondition, default_verify_timeout_ms, no_observed_delta_error,
    postcondition_failed_error, postcondition_not_requested, postcondition_observed_delta,
    text_signature,
};

const MIN_SAFE_LINEAR_MS_PER_CHAR: u32 = 20;
const TEXT_INTEGRITY_DISPATCH_ONLY: &str = "dispatch_only_requires_target_readback";
const TEXT_INTEGRITY_UIA_VALUE_PATTERN: &str = "uia_value_pattern_readback";
const TEXT_INTEGRITY_UIA_VALUE_PATTERN_DISPATCH_ONLY: &str =
    "uia_value_pattern_dispatch_only_requires_target_readback";
/// Text was inserted into a web input via CDP `Input.insertText` after focusing
/// the DOM node. Verify via `observe`/`find` (the node's `value`) — this path
/// dispatches into the renderer, so a follow-up readback is required (#686).
#[cfg(windows)]
const TEXT_INTEGRITY_CDP_INSERT_TEXT: &str =
    "cdp_insert_text_dispatch_only_requires_target_readback";

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActTypeParams {
    pub text: String,
    #[serde(default)]
    pub into_element: Option<ElementId>,
    #[serde(default = "default_type_dynamics")]
    #[schemars(default = "default_type_dynamics")]
    pub dynamics: TypeDynamics,
    #[serde(default = "default_linear_ms_per_char")]
    #[schemars(default = "default_linear_ms_per_char", range(min = 20))]
    pub linear_ms_per_char: u32,
    #[serde(default = "default_use_scancodes")]
    #[schemars(default = "default_use_scancodes")]
    pub use_scancodes: bool,
    #[serde(default = "default_press_enter_after")]
    #[schemars(default = "default_press_enter_after")]
    pub press_enter_after: bool,
    #[serde(default = "default_type_backend")]
    #[schemars(default = "default_type_backend")]
    pub backend: TypeBackend,
    #[serde(default = "default_verify_delta")]
    #[schemars(default = "default_verify_delta")]
    pub verify_delta: bool,
    #[serde(default = "default_verify_timeout_ms")]
    #[schemars(default = "default_verify_timeout_ms", range(min = 50, max = 5000))]
    pub verify_timeout_ms: u32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TypeDynamics {
    Burst,
    Linear,
    Natural,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TypeBackend {
    Software,
    Hardware,
    Auto,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActTypeResponse {
    pub ok: bool,
    pub chars_typed: u32,
    pub elapsed_ms: u32,
    pub target_text_integrity: String,
    pub target_readback_required: bool,
    pub minimum_linear_ms_per_char: u32,
    pub postcondition: ActPostcondition,
}

/// Routes `act_type into_element=<web element id>` through CDP (#686): resolve
/// the browser endpoint from the element's window, focus the DOM node, and
/// insert the text. Fail-loud if the endpoint/node is gone.
#[cfg(windows)]
async fn cdp_type_into_element(
    element_id: &ElementId,
    backend_node_id: i64,
    emitted: &str,
    chars_typed: u32,
    started: Instant,
    verify_delta: bool,
    verify_timeout_ms: u32,
) -> Result<ActTypeResponse, ErrorData> {
    use synapse_core::error_codes;

    let hwnd = element_id
        .parts()
        .map_err(|err| {
            mcp_error(
                error_codes::ACTION_ELEMENT_NOT_RESOLVED,
                format!("web element id is malformed: {err}"),
            )
        })?
        .hwnd;
    let endpoint = synapse_a11y::endpoint_for_window(hwnd).ok_or_else(|| {
        mcp_error(
            error_codes::A11Y_CDP_UNREACHABLE,
            format!(
                "no reachable CDP endpoint for web element {element_id} (browser closed or debug port gone)"
            ),
        )
    })?;
    let title_hint = synapse_a11y::foreground_context(hwnd)
        .map(|context| context.window_title)
        .unwrap_or_default();
    let target_id_hint = synapse_a11y::cdp_target_from_element_id(element_id);
    let before = if verify_delta {
        Some(
            synapse_a11y::cdp_node_value(
                &endpoint,
                &title_hint,
                target_id_hint.as_deref(),
                backend_node_id,
            )
            .await
            .map_err(|err| mcp_error(err.code(), err.to_string()))?,
        )
    } else {
        None
    };
    synapse_a11y::cdp_type_node(
        &endpoint,
        &title_hint,
        target_id_hint.as_deref(),
        backend_node_id,
        emitted,
    )
    .await
    .map_err(|err| mcp_error(err.code(), err.to_string()))?;
    let postcondition = if let Some(before) = before {
        tokio::time::sleep(std::time::Duration::from_millis(u64::from(
            verify_timeout_ms,
        )))
        .await;
        let after = synapse_a11y::cdp_node_value(
            &endpoint,
            &title_hint,
            target_id_hint.as_deref(),
            backend_node_id,
        )
        .await
        .map_err(|err| mcp_error(err.code(), err.to_string()))?;
        verify_cdp_type_delta(verify_timeout_ms, emitted, before, after)?
    } else {
        postcondition_not_requested("act_type", "cdp_node.value")
    };
    tracing::info!(
        code = "M2_ACT_TYPE_CDP_INSERT_TEXT",
        element_id = %element_id,
        chars_typed,
        "readback=act_type_into_element method=cdp_insert_text chars_typed={chars_typed}"
    );
    Ok(ActTypeResponse {
        ok: true,
        chars_typed,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        target_text_integrity: TEXT_INTEGRITY_CDP_INSERT_TEXT.to_owned(),
        target_readback_required: !verify_delta,
        minimum_linear_ms_per_char: MIN_SAFE_LINEAR_MS_PER_CHAR,
        postcondition,
    })
}

pub async fn act_type_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActTypeParams,
) -> Result<ActTypeResponse, ErrorData> {
    let started = Instant::now();
    validate_type_params(&params)?;
    let emitted = emitted_text(&params);
    let chars_typed = char_count(&emitted)?;

    if let Some(element_id) = &params.into_element {
        // #686: a web element id (cdcd sentinel) routes through CDP focus+insert.
        #[cfg(windows)]
        if let Some(backend) = synapse_a11y::cdp_backend_from_element_id(element_id) {
            return cdp_type_into_element(
                element_id,
                backend,
                &emitted,
                chars_typed,
                started,
                params.verify_delta,
                params.verify_timeout_ms,
            )
            .await;
        }
        let readback = if params.verify_delta {
            verified_set_element_value(element_id, &emitted, params.verify_timeout_ms).await?
        } else {
            synapse_a11y::set_element_value(element_id, &emitted).map_err(a11y_error_to_mcp)?
        };
        let readback_matches = readback.after_value == emitted;
        if !readback_matches {
            tracing::warn!(
                code = "M2_ACT_TYPE_ELEMENT_VALUE_PATTERN_READBACK_MISMATCH",
                element_id = %element_id,
                method = %readback.method,
                before_len = readback.before_value.chars().count(),
                after_len = readback.after_value.chars().count(),
                expected_len = emitted.chars().count(),
                chars_typed,
                "act_type into_element ValuePattern SetValue returned success but immediate UIA value readback did not match; target SoT readback is required"
            );
        }
        tracing::info!(
            code = "M2_ACT_TYPE_ELEMENT_VALUE_PATTERN_READBACK",
            element_id = %element_id,
            method = %readback.method,
            before_len = readback.before_value.chars().count(),
            after_len = readback.after_value.chars().count(),
            chars_typed,
            "readback=act_type_into_element before_len={} after_len={} chars_typed={} method={}",
            readback.before_value.chars().count(),
            readback.after_value.chars().count(),
            chars_typed,
            readback.method
        );
        return Ok(ActTypeResponse {
            ok: true,
            chars_typed,
            elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
            target_text_integrity: if readback_matches {
                TEXT_INTEGRITY_UIA_VALUE_PATTERN
            } else {
                TEXT_INTEGRITY_UIA_VALUE_PATTERN_DISPATCH_ONLY
            }
            .to_owned(),
            target_readback_required: !params.verify_delta || !readback_matches,
            minimum_linear_ms_per_char: MIN_SAFE_LINEAR_MS_PER_CHAR,
            postcondition: if params.verify_delta {
                verify_uia_type_delta(params.verify_timeout_ms, &emitted, &readback)?
            } else {
                postcondition_not_requested("act_type", "uia_value_pattern.value")
            },
        });
    }

    let action = action_from_type_params(&params)?;

    if let Some(recording) = recording {
        execute_recording(&recording, &action)?;
    } else {
        handle
            .execute(action)
            .await
            .map_err(|error| action_error_to_mcp(&error))?;
    }

    Ok(ActTypeResponse {
        ok: true,
        chars_typed,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        target_text_integrity: TEXT_INTEGRITY_DISPATCH_ONLY.to_owned(),
        target_readback_required: true,
        minimum_linear_ms_per_char: MIN_SAFE_LINEAR_MS_PER_CHAR,
        postcondition: postcondition_not_requested("act_type", "foreground_focused_ui_or_pixels"),
    })
}

pub fn action_from_type_params(params: &ActTypeParams) -> Result<Action, ErrorData> {
    validate_type_params(params)?;
    if let Some(element_id) = &params.into_element {
        return Err(action_error_to_mcp(&ActionError::BackendUnavailable {
            detail: format!(
                "act_type into_element target {element_id} requires live UIA ValuePattern dispatch, not action-only conversion"
            ),
        }));
    }
    Ok(Action::TypeText {
        text: emitted_text(params),
        dynamics: params
            .dynamics
            .to_keystroke_dynamics(params.linear_ms_per_char),
        backend: params.backend.to_backend(),
    })
}

async fn verified_set_element_value(
    element_id: &ElementId,
    emitted: &str,
    verify_timeout_ms: u32,
) -> Result<synapse_a11y::ElementValueSetReadback, ErrorData> {
    let before = synapse_a11y::element_value(element_id).map_err(a11y_error_to_mcp)?;
    match synapse_a11y::set_element_value(element_id, emitted) {
        Ok(readback) => Ok(readback),
        Err(error) => {
            tokio::time::sleep(std::time::Duration::from_millis(u64::from(
                verify_timeout_ms,
            )))
            .await;
            let after = synapse_a11y::element_value(element_id).map_err(a11y_error_to_mcp)?;
            let before_signature = text_signature(&before.value);
            let after_signature = text_signature(&after.value);
            if before.value == after.value {
                return Err(no_observed_delta_error(
                    "act_type",
                    "uia_value_pattern.value",
                    verify_timeout_ms,
                    before_signature,
                    after_signature,
                    json!({
                        "element_id": element_id.to_string(),
                        "before_len": before.value.chars().count(),
                        "after_len": after.value.chars().count(),
                        "before_readonly": before.is_readonly,
                        "after_readonly": after.is_readonly,
                        "set_error": error.to_string(),
                    }),
                ));
            }
            Err(postcondition_failed_error(
                "act_type",
                "uia_value_pattern.value",
                format!("ValuePattern SetValue failed but value changed: {error}"),
                before_signature,
                after_signature,
                json!({
                    "element_id": element_id.to_string(),
                    "expected_len": emitted.chars().count(),
                    "before_len": before.value.chars().count(),
                    "after_len": after.value.chars().count(),
                    "set_error": error.to_string(),
                }),
            ))
        }
    }
}

fn verify_uia_type_delta(
    verify_timeout_ms: u32,
    emitted: &str,
    readback: &synapse_a11y::ElementValueSetReadback,
) -> Result<ActPostcondition, ErrorData> {
    let before_signature = text_signature(&readback.before_value);
    let after_signature = text_signature(&readback.after_value);
    if readback.before_value == readback.after_value {
        return Err(no_observed_delta_error(
            "act_type",
            "uia_value_pattern.value",
            verify_timeout_ms,
            before_signature,
            after_signature,
            json!({
                "method": readback.method,
                "before_len": readback.before_value.chars().count(),
                "after_len": readback.after_value.chars().count(),
                "expected_len": emitted.chars().count(),
            }),
        ));
    }
    if readback.after_value != emitted {
        return Err(postcondition_failed_error(
            "act_type",
            "uia_value_pattern.value",
            "UIA ValuePattern value changed but does not equal requested text",
            before_signature,
            after_signature,
            json!({
                "method": readback.method,
                "before_len": readback.before_value.chars().count(),
                "after_len": readback.after_value.chars().count(),
                "expected_len": emitted.chars().count(),
            }),
        ));
    }
    Ok(postcondition_observed_delta(
        "act_type",
        "uia_value_pattern.value",
        before_signature,
        after_signature,
        "observed target value equal requested text",
    ))
}

fn verify_cdp_type_delta(
    verify_timeout_ms: u32,
    emitted: &str,
    before: String,
    after: String,
) -> Result<ActPostcondition, ErrorData> {
    let before_signature = text_signature(&before);
    let after_signature = text_signature(&after);
    if before == after {
        return Err(no_observed_delta_error(
            "act_type",
            "cdp_node.value",
            verify_timeout_ms,
            before_signature,
            after_signature,
            json!({
                "before_len": before.chars().count(),
                "after_len": after.chars().count(),
                "expected_insert_len": emitted.chars().count(),
            }),
        ));
    }
    if !after.contains(emitted) {
        return Err(postcondition_failed_error(
            "act_type",
            "cdp_node.value",
            "CDP node value changed but does not contain requested inserted text",
            before_signature,
            after_signature,
            json!({
                "before_len": before.chars().count(),
                "after_len": after.chars().count(),
                "expected_insert_len": emitted.chars().count(),
            }),
        ));
    }
    Ok(postcondition_observed_delta(
        "act_type",
        "cdp_node.value",
        before_signature,
        after_signature,
        "observed target value containing requested inserted text",
    ))
}

impl TypeDynamics {
    const fn to_keystroke_dynamics(self, linear_ms_per_char: u32) -> KeystrokeDynamics {
        match self {
            Self::Burst => KeystrokeDynamics::Burst,
            Self::Linear => KeystrokeDynamics::Linear {
                ms_per_char: linear_ms_per_char,
            },
            Self::Natural => KeystrokeDynamics::Natural {
                params: KeystrokeNaturalParams::FAST,
            },
        }
    }
}

impl TypeBackend {
    const fn to_backend(self) -> Backend {
        match self {
            Self::Software => Backend::Software,
            Self::Hardware => Backend::Hardware,
            Self::Auto => Backend::Auto,
        }
    }
}

fn validate_type_params(params: &ActTypeParams) -> Result<(), ErrorData> {
    if params.use_scancodes {
        return Err(action_error_to_mcp(&ActionError::BackendUnavailable {
            detail: "act_type use_scancodes=true is not wired for the M2 unicode typing path"
                .to_owned(),
        }));
    }
    if params.dynamics == TypeDynamics::Linear
        && params.linear_ms_per_char < MIN_SAFE_LINEAR_MS_PER_CHAR
    {
        return Err(type_params_error(
            params.linear_ms_per_char,
            format!(
                "act_type linear_ms_per_char {} is below the text-integrity minimum {}; use slower pacing and verify target text via UI/file readback",
                params.linear_ms_per_char, MIN_SAFE_LINEAR_MS_PER_CHAR
            ),
        ));
    }
    Ok(())
}

pub(crate) fn emitted_text(params: &ActTypeParams) -> String {
    if params.press_enter_after {
        let mut text = params.text.clone();
        text.push('\n');
        text
    } else {
        params.text.clone()
    }
}

fn char_count(text: &str) -> Result<u32, ErrorData> {
    u32::try_from(text.chars().count()).map_err(|_err| {
        mcp_error(
            synapse_core::error_codes::TOOL_PARAMS_INVALID,
            "act_type text has more than u32::MAX chars",
        )
    })
}

fn execute_recording(recording: &RecordingBackend, action: &Action) -> Result<(), ErrorData> {
    let before_events = recording.events();
    let before_event_count = before_events.len();
    let mut emit_state = EmitState::new();
    recording
        .execute(action, &mut emit_state)
        .map_err(|error| action_error_to_mcp(&error))?;
    let after_events = recording.events();
    let new_events = &after_events[before_event_count..];
    let recorded_ikis = recorded_ikis(new_events);
    tracing::info!(
        code = "M2_ACT_TYPE_RECORDING_READBACK",
        kind = "act_type",
        before_event_count,
        after_event_count = after_events.len(),
        new_event_count = new_events.len(),
        ?recorded_ikis,
        ?new_events,
        "readback=recording_backend tool=act_type after_events_readback"
    );
    Ok(())
}

fn recorded_ikis(events: &[RecordedInput]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|event| match event {
            RecordedInput::DelayMs { ms } => Some(*ms),
            _ => None,
        })
        .collect()
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

fn a11y_error_to_mcp(error: synapse_a11y::A11yError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

fn type_params_error(requested_linear_ms_per_char: u32, message: impl Into<String>) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        message.into(),
        Some(json!({
            "code": synapse_core::error_codes::TOOL_PARAMS_INVALID,
            "reason": "linear_ms_per_char_below_text_integrity_minimum",
            "requested_linear_ms_per_char": requested_linear_ms_per_char,
            "minimum_linear_ms_per_char": MIN_SAFE_LINEAR_MS_PER_CHAR,
            "target_text_integrity": TEXT_INTEGRITY_DISPATCH_ONLY,
            "target_readback_required": true,
        })),
    )
}

const fn default_type_dynamics() -> TypeDynamics {
    TypeDynamics::Natural
}

const fn default_linear_ms_per_char() -> u32 {
    30
}

const fn default_type_backend() -> TypeBackend {
    TypeBackend::Auto
}

const fn default_verify_delta() -> bool {
    true
}

const fn default_use_scancodes() -> bool {
    false
}

const fn default_press_enter_after() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use synapse_action::{ActionEmitter, RecordedInput, sample_typing_schedule};
    use synapse_core::{ElementId, KeystrokeNaturalParams};

    use super::{
        ActTypeParams, MIN_SAFE_LINEAR_MS_PER_CHAR, TEXT_INTEGRITY_DISPATCH_ONLY, TypeBackend,
        TypeDynamics, act_type_with_handle, action_from_type_params, default_linear_ms_per_char,
        default_press_enter_after, default_type_backend, default_type_dynamics,
        default_use_scancodes, default_verify_delta, default_verify_timeout_ms, recorded_ikis,
    };

    #[tokio::test]
    async fn recording_backend_readback_uses_natural_fast_ikis() {
        let (handle, _snapshot_handle, _emitter) = ActionEmitter::channel();
        let recording = Arc::new(synapse_action::RecordingBackend::new());
        let text = "Hello world.";
        let params = ActTypeParams {
            text: text.to_owned(),
            into_element: None,
            dynamics: default_type_dynamics(),
            linear_ms_per_char: default_linear_ms_per_char(),
            use_scancodes: false,
            press_enter_after: false,
            backend: default_type_backend(),
            verify_delta: false,
            verify_timeout_ms: default_verify_timeout_ms(),
        };
        let before = recording.events();
        println!("readback=act_type_recording edge=natural_fast before={before:?}");

        let response = act_type_with_handle(handle, Some(Arc::clone(&recording)), params)
            .await
            .unwrap_or_else(|error| panic!("act_type recording should succeed: {error}"));
        let after = recording.events();
        let actual_ikis = recorded_ikis(&after);
        let expected_ikis: Vec<u32> = sample_typing_schedule(
            text,
            &TypeDynamics::Natural.to_keystroke_dynamics(default_linear_ms_per_char()),
            None,
        )
        .into_iter()
        .filter_map(|event| (event.iki_ms_before > 0).then_some(event.iki_ms_before))
        .collect();
        println!(
            "readback=act_type_recording edge=natural_fast after={after:?} expected_ikis={expected_ikis:?} actual_ikis={actual_ikis:?} chars_typed={}",
            response.chars_typed
        );

        assert!(response.ok);
        assert_eq!(response.chars_typed, 12);
        assert_eq!(response.target_text_integrity, TEXT_INTEGRITY_DISPATCH_ONLY);
        assert!(response.target_readback_required);
        assert_eq!(
            response.minimum_linear_ms_per_char,
            MIN_SAFE_LINEAR_MS_PER_CHAR
        );
        assert_eq!(actual_ikis, expected_ikis);
        assert_eq!(
            TypeDynamics::Natural.to_keystroke_dynamics(default_linear_ms_per_char()),
            synapse_core::KeystrokeDynamics::Natural {
                params: KeystrokeNaturalParams::FAST
            }
        );
    }

    #[test]
    fn defaults_are_issue_required_values() {
        assert_eq!(default_type_dynamics(), TypeDynamics::Natural);
        assert_eq!(default_linear_ms_per_char(), 30);
        assert_eq!(default_type_backend(), TypeBackend::Auto);
        assert!(default_verify_delta());
        assert!(!default_use_scancodes());
        assert!(!default_press_enter_after());
    }

    #[test]
    fn recorded_ikis_only_reads_delay_events() {
        let before = vec![
            RecordedInput::DelayMs { ms: 17 },
            RecordedInput::DelayMs { ms: 0 },
        ];
        let after = recorded_ikis(&before);
        println!("readback=act_type_recording edge=iki_readback before={before:?} after={after:?}");
        assert_eq!(after, [17, 0]);
    }

    #[test]
    fn linear_typing_below_safe_minimum_fails_closed() {
        let params = ActTypeParams {
            text: "unsafe".to_owned(),
            into_element: None,
            dynamics: TypeDynamics::Linear,
            linear_ms_per_char: MIN_SAFE_LINEAR_MS_PER_CHAR - 1,
            use_scancodes: false,
            press_enter_after: false,
            backend: TypeBackend::Software,
            verify_delta: false,
            verify_timeout_ms: default_verify_timeout_ms(),
        };

        let error = match action_from_type_params(&params) {
            Ok(action) => panic!("low linear pacing dispatched unexpectedly: {action:?}"),
            Err(error) => error,
        };
        let Some(data) = error.data else {
            panic!("low linear pacing error had no structured data");
        };

        assert_eq!(data["code"], synapse_core::error_codes::TOOL_PARAMS_INVALID);
        assert_eq!(
            data["reason"],
            "linear_ms_per_char_below_text_integrity_minimum"
        );
        assert_eq!(
            data["minimum_linear_ms_per_char"],
            MIN_SAFE_LINEAR_MS_PER_CHAR
        );
        assert_eq!(
            data["requested_linear_ms_per_char"],
            MIN_SAFE_LINEAR_MS_PER_CHAR - 1
        );
        assert_eq!(data["target_readback_required"], true);
        assert_eq!(data["target_text_integrity"], TEXT_INTEGRITY_DISPATCH_ONLY);
    }

    #[test]
    fn linear_typing_at_safe_minimum_is_allowed() {
        let params = ActTypeParams {
            text: "safe".to_owned(),
            into_element: None,
            dynamics: TypeDynamics::Linear,
            linear_ms_per_char: MIN_SAFE_LINEAR_MS_PER_CHAR,
            use_scancodes: false,
            press_enter_after: false,
            backend: TypeBackend::Software,
            verify_delta: false,
            verify_timeout_ms: default_verify_timeout_ms(),
        };

        let action = match action_from_type_params(&params) {
            Ok(action) => action,
            Err(error) => panic!("linear pacing at safe minimum failed unexpectedly: {error}"),
        };
        assert_eq!(
            action,
            synapse_core::Action::TypeText {
                text: "safe".to_owned(),
                dynamics: synapse_core::KeystrokeDynamics::Linear {
                    ms_per_char: MIN_SAFE_LINEAR_MS_PER_CHAR,
                },
                backend: synapse_core::Backend::Software,
            }
        );
    }

    #[test]
    fn action_only_conversion_rejects_into_element() {
        let params = ActTypeParams {
            text: "targeted".to_owned(),
            into_element: Some(
                ElementId::parse("0x1000:0000002a00000001")
                    .expect("synthetic element id should parse"),
            ),
            dynamics: TypeDynamics::Linear,
            linear_ms_per_char: MIN_SAFE_LINEAR_MS_PER_CHAR,
            use_scancodes: false,
            press_enter_after: false,
            backend: TypeBackend::Software,
            verify_delta: false,
            verify_timeout_ms: default_verify_timeout_ms(),
        };

        let error = match action_from_type_params(&params) {
            Ok(action) => panic!("element-targeted act_type converted unexpectedly: {action:?}"),
            Err(error) => error,
        };
        assert!(
            error
                .message
                .contains("requires live UIA ValuePattern dispatch")
        );
    }
}
