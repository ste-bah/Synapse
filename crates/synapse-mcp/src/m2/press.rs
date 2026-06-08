use std::{sync::Arc, time::Instant};

use rmcp::ErrorData;
use synapse_action::{ActionError, ActionHandle, RecordingBackend};
use synapse_core::{Action, Backend, Key, KeyCode, Profile, error_codes};
use tokio_util::sync::CancellationToken;

use crate::m1::mcp_error;

const MAX_HOLD_MS: u32 = 30_000;

mod keys;
mod live;
mod record;
mod schema;
#[cfg(test)]
mod tests;

use schema::press_postcondition_not_requested;
pub use schema::{
    ActKeymapParams, ActKeymapResponse, ActPressParams, ActPressResponse, PressBackend,
};

pub async fn act_press_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    connection_closed_cancel: Option<CancellationToken>,
    params: ActPressParams,
) -> Result<ActPressResponse, ErrorData> {
    let started = Instant::now();
    let keys = keys::normalized_keys(&params.keys)?;
    let key_count = u32::try_from(keys.len()).map_err(|_err| {
        mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_press keys length exceeds u32::MAX",
        )
    })?;
    let backend = params.backend.to_backend();
    validate_hold_ms(params.hold_ms)?;
    let action = press_action(keys.clone(), params.hold_ms, backend);

    if let Some(recording) = recording {
        record::execute_recording(&recording, &action)?;
    } else {
        live::execute_live_press_sequence(
            handle,
            keys,
            params.hold_ms,
            backend,
            connection_closed_cancel,
        )
        .await?;
    }

    Ok(ActPressResponse {
        ok: true,
        keys_pressed: key_count,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        backend_used: backend_used_name(backend).to_owned(),
        backend_tier_used: "foreground".to_owned(),
        required_foreground: true,
        postcondition: press_postcondition_not_requested(),
    })
}

pub async fn act_keymap_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    connection_closed_cancel: Option<CancellationToken>,
    profile: &Profile,
    params: ActKeymapParams,
) -> Result<ActKeymapResponse, ErrorData> {
    let alias = normalized_alias(&params.alias)?;
    let resolved_binding = profile.keymap.get(&alias).cloned().ok_or_else(|| {
        mcp_error(
            error_codes::PROFILE_KEYMAP_INVALID,
            format!(
                "profile {} keymap alias {alias:?} was not found",
                profile.id
            ),
        )
    })?;
    let resolved_keys = split_key_binding(&resolved_binding)?;
    let press = ActPressParams {
        keys: resolved_keys.clone(),
        hold_ms: params.hold_ms,
        backend: params.backend,
        verify_delta: false,
        allow_foreground_change: false,
        expected_foreground_process_regex: None,
        expected_foreground_title_regex: None,
        verify_timeout_ms: crate::m2::default_verify_timeout_ms(),
    };
    let response =
        act_press_with_handle(handle, recording, connection_closed_cancel, press).await?;

    Ok(ActKeymapResponse {
        ok: response.ok,
        alias,
        resolved_binding,
        resolved_keys,
        hold_ms: params.hold_ms,
        keys_pressed: response.keys_pressed,
        elapsed_ms: response.elapsed_ms,
        backend_used: response.backend_used,
        backend_tier_used: response.backend_tier_used,
        required_foreground: response.required_foreground,
    })
}

pub fn action_from_press_params(params: &ActPressParams) -> Result<Action, ErrorData> {
    validate_hold_ms(params.hold_ms)?;
    let keys = keys::normalized_keys(&params.keys)?;
    Ok(press_action(
        keys,
        params.hold_ms,
        params.backend.to_backend(),
    ))
}

fn normalized_alias(alias: &str) -> Result<String, ErrorData> {
    let alias = alias.trim();
    if alias.is_empty() {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_keymap alias must not be empty",
        ));
    }
    Ok(alias.to_ascii_lowercase())
}

fn split_key_binding(binding: &str) -> Result<Vec<String>, ErrorData> {
    let keys = binding
        .split('+')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let normalized = keys::normalized_keys(&keys)?;
    Ok(normalized.iter().map(key_label).collect())
}

fn key_label(key: &Key) -> String {
    match &key.code {
        KeyCode::Named { value } => value.clone(),
        KeyCode::Symbol { value } => value.to_string(),
        KeyCode::HidCode { value } => format!("hid:{value}"),
    }
}

fn validate_hold_ms(hold_ms: u32) -> Result<(), ErrorData> {
    if hold_ms == 0 {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_press hold_ms must be at least 1",
        ));
    }
    if hold_ms > MAX_HOLD_MS {
        return Err(action_error_to_mcp(&ActionError::HoldExceededMax {
            detail: format!("act_press hold_ms {hold_ms} exceeds max {MAX_HOLD_MS}"),
        }));
    }
    Ok(())
}

fn press_action(keys: Vec<synapse_core::Key>, hold_ms: u32, backend: Backend) -> Action {
    if let [key] = keys.as_slice() {
        return Action::KeyPress {
            key: key.clone(),
            hold_ms,
            backend,
        };
    }
    Action::KeyChord {
        keys,
        hold_ms,
        backend,
    }
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    crate::m2::action_error_to_mcp(error)
}

const fn backend_used_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Auto | Backend::Software => "software",
        Backend::Vigem => "vigem",
        Backend::Hardware => "hardware",
    }
}
