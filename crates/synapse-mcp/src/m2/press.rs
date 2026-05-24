use std::{collections::HashSet, sync::Arc, time::Instant};

use rmcp::ErrorData;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_action::{
    ActionBackend, ActionError, ActionHandle, EmitState, RecordedInput, RecordingBackend,
};
use synapse_core::{Action, Backend, Key, KeyCode, error_codes};
use tokio_util::sync::CancellationToken;

use crate::m1::mcp_error;

const DEFAULT_HOLD_MS: u32 = 33;
const MAX_HOLD_MS: u32 = 30_000;
const MODIFIER_ORDER: [&str; 4] = ["ctrl", "shift", "alt", "super"];

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActPressParams {
    pub keys: Vec<String>,
    #[serde(default = "default_hold_ms")]
    #[schemars(default = "default_hold_ms", range(min = 1, max = 30000))]
    pub hold_ms: u32,
    #[serde(default = "default_press_backend")]
    #[schemars(default = "default_press_backend")]
    pub backend: PressBackend,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PressBackend {
    Software,
    Hardware,
    Auto,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActPressResponse {
    pub ok: bool,
    pub keys_pressed: u32,
    pub elapsed_ms: u32,
    pub backend_used: String,
}

pub async fn act_press_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    connection_closed_cancel: Option<CancellationToken>,
    params: ActPressParams,
) -> Result<ActPressResponse, ErrorData> {
    validate_hold_ms(params.hold_ms)?;
    let started = Instant::now();
    let keys = normalized_keys(&params.keys)?;
    let key_count = u32::try_from(keys.len()).map_err(|_err| {
        mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_press keys length exceeds u32::MAX",
        )
    })?;
    let backend = params.backend.to_backend();
    let action = press_action(keys.clone(), params.hold_ms, backend);

    if let Some(recording) = recording {
        execute_recording(&recording, &action)?;
    } else {
        execute_live_press_sequence(
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
    })
}

impl PressBackend {
    const fn to_backend(self) -> Backend {
        match self {
            Self::Software => Backend::Software,
            Self::Hardware => Backend::Hardware,
            Self::Auto => Backend::Auto,
        }
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

fn normalized_keys(raw_keys: &[String]) -> Result<Vec<Key>, ErrorData> {
    if raw_keys.is_empty() {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_press keys must contain at least one key",
        ));
    }

    let mut seen = HashSet::new();
    let mut names = Vec::with_capacity(raw_keys.len());
    for raw_key in raw_keys {
        let name = canonical_key_name(raw_key)?;
        if !seen.insert(name.clone()) {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!("act_press duplicate key '{name}'"),
            ));
        }
        names.push(name);
    }

    let mut ordered = Vec::with_capacity(names.len());
    for modifier in MODIFIER_ORDER {
        if names.iter().any(|name| name == modifier) {
            ordered.push(key(modifier));
        }
    }
    for name in names
        .iter()
        .filter(|name| !MODIFIER_ORDER.contains(&name.as_str()))
    {
        ordered.push(key(name));
    }
    Ok(ordered)
}

fn canonical_key_name(raw_key: &str) -> Result<String, ErrorData> {
    let lowered = raw_key.trim().to_ascii_lowercase();
    let key = match lowered.as_str() {
        "" => {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                "act_press key names must be non-empty",
            ));
        }
        "control" => "ctrl",
        "escape" => "esc",
        "return" => "enter",
        "arrowup" => "up",
        "arrowdown" => "down",
        "arrowleft" => "left",
        "arrowright" => "right",
        "win" | "windows" | "meta" => "super",
        "pgup" => "pageup",
        "pgdn" => "pagedown",
        other => other,
    };

    if is_allowed_key_name(key) {
        Ok(key.to_owned())
    } else {
        Err(action_error_to_mcp(&ActionError::UnsupportedKey {
            detail: format!("act_press unsupported key '{raw_key}'"),
        }))
    }
}

fn is_allowed_key_name(key: &str) -> bool {
    if key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric() {
        return true;
    }
    if let Some(number) = key
        .strip_prefix('f')
        .and_then(|suffix| suffix.parse::<u8>().ok())
    {
        return (1..=24).contains(&number);
    }
    matches!(
        key,
        "alt"
            | "backspace"
            | "ctrl"
            | "delete"
            | "down"
            | "end"
            | "enter"
            | "esc"
            | "home"
            | "insert"
            | "left"
            | "pagedown"
            | "pageup"
            | "right"
            | "shift"
            | "space"
            | "super"
            | "tab"
            | "up"
    )
}

fn press_action(keys: Vec<Key>, hold_ms: u32, backend: Backend) -> Action {
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

async fn execute_live_press_sequence(
    handle: ActionHandle,
    keys: Vec<Key>,
    hold_ms: u32,
    backend: Backend,
    connection_closed_cancel: Option<CancellationToken>,
) -> Result<(), ErrorData> {
    let mut pressed = Vec::with_capacity(keys.len());
    for key in &keys {
        if let Err(error) = handle
            .execute(Action::KeyDown {
                key: key.clone(),
                backend,
            })
            .await
        {
            release_pressed_keys(&handle, &pressed, backend).await;
            return Err(action_error_to_mcp(&error));
        }
        pressed.push(key.clone());
        maybe_force_panic_after_keydown();
    }

    let connection_closed = if let Some(cancel) = connection_closed_cancel {
        tokio::select! {
            () = tokio::time::sleep(std::time::Duration::from_millis(u64::from(hold_ms))) => false,
            () = cancel.cancelled() => true,
        }
    } else {
        tokio::time::sleep(std::time::Duration::from_millis(u64::from(hold_ms))).await;
        false
    };

    let mut first_error = None;
    for key in pressed.iter().rev() {
        if let Err(error) = handle
            .execute(Action::KeyUp {
                key: key.clone(),
                backend,
            })
            .await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }

    if let Some(error) = first_error {
        return Err(action_error_to_mcp(&error));
    }
    if connection_closed {
        tracing::warn!(
            code = "M2_ACT_PRESS_CONNECTION_CLOSED_RELEASE",
            released_keys = pressed.len(),
            "source_of_truth=connection_closed edge=act_press after=pressed_keys_released"
        );
    }
    Ok(())
}

#[cfg(debug_assertions)]
fn maybe_force_panic_after_keydown() {
    if std::env::var("SYNAPSE_MCP_FORCE_PANIC_DURING_ACT").as_deref()
        == Ok("act_press_after_keydown")
    {
        tracing::warn!(
            code = "M2_ACT_PRESS_FORCE_PANIC_AFTER_KEYDOWN",
            "source_of_truth=act_press edge=force_panic after=keydown"
        );
        tokio::task::block_in_place(|| panic!("forced panic during act_press after keydown"));
    }
}

#[cfg(not(debug_assertions))]
const fn maybe_force_panic_after_keydown() {}

async fn release_pressed_keys(handle: &ActionHandle, pressed: &[Key], backend: Backend) {
    for key in pressed.iter().rev() {
        let _ = handle
            .execute(Action::KeyUp {
                key: key.clone(),
                backend,
            })
            .await;
    }
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
    let event_sequence = event_sequence(new_events);
    tracing::info!(
        code = "M2_ACT_PRESS_RECORDING_READBACK",
        kind = "act_press",
        before_event_count,
        after_event_count = after_events.len(),
        new_event_count = new_events.len(),
        event_sequence,
        ?new_events,
        "source_of_truth=recording_backend tool=act_press after_events_readback"
    );
    Ok(())
}

fn event_sequence(events: &[RecordedInput]) -> String {
    events.iter().map(event_label).collect::<Vec<_>>().join(">")
}

fn event_label(event: &RecordedInput) -> String {
    match event {
        RecordedInput::KeyDown { key } => format!("down:{}", key_label(key)),
        RecordedInput::KeyUp { key } => format!("up:{}", key_label(key)),
        RecordedInput::DelayMs { ms } => format!("delay:{ms}"),
        other => format!("{other:?}"),
    }
}

fn key_label(key: &Key) -> String {
    match &key.code {
        KeyCode::Named { value } => value.clone(),
        KeyCode::Symbol { value } => value.to_string(),
        KeyCode::HidCode { value } => format!("hid:{value}"),
    }
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

fn key(value: &str) -> Key {
    Key {
        code: KeyCode::Named {
            value: value.to_owned(),
        },
        use_scancode: false,
    }
}

const fn default_hold_ms() -> u32 {
    DEFAULT_HOLD_MS
}

const fn default_press_backend() -> PressBackend {
    PressBackend::Auto
}

const fn backend_used_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Auto | Backend::Software => "software",
        Backend::Vigem => "vigem",
        Backend::Hardware => "hardware",
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::{
        ActPressParams, PressBackend, act_press_with_handle, default_hold_ms,
        default_press_backend, event_sequence, execute_live_press_sequence, key, normalized_keys,
    };
    use synapse_action::{ActionEmitter, RecordedInput, RecordingBackend};
    use synapse_core::{Action, Backend};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn recording_backend_readback_orders_chord_and_default_hold() {
        let (handle, _snapshot_handle, _emitter) = ActionEmitter::channel();
        let recording = Arc::new(RecordingBackend::new());
        let params = ActPressParams {
            keys: vec!["shift".to_owned(), "ctrl".to_owned(), "s".to_owned()],
            hold_ms: default_hold_ms(),
            backend: default_press_backend(),
        };
        let before = recording.events();
        println!("source_of_truth=act_press_recording edge=ordered_chord before={before:?}");

        let response = act_press_with_handle(handle, Some(Arc::clone(&recording)), None, params)
            .await
            .unwrap_or_else(|error| panic!("act_press recording should succeed: {error}"));
        let after = recording.events();
        let sequence = event_sequence(&after);
        println!(
            "source_of_truth=act_press_recording edge=ordered_chord after={after:?} sequence={sequence} keys_pressed={}",
            response.keys_pressed
        );

        assert!(response.ok);
        assert_eq!(response.keys_pressed, 3);
        assert_eq!(
            sequence,
            "down:ctrl>down:shift>down:s>delay:33>up:s>up:shift>up:ctrl"
        );
    }

    #[tokio::test]
    async fn live_press_sequence_leaves_actor_available_for_release_all_mid_hold() {
        let cancel = CancellationToken::new();
        let recording = Arc::new(RecordingBackend::new());
        let (handle, snapshot_handle, join) =
            ActionEmitter::spawn_with_backend(cancel.clone(), recording.clone());
        let keys = vec![key("a")];
        let started_events = recording.events();
        println!(
            "source_of_truth=act_press_live_sequence edge=mid_hold_release before_events={started_events:?}"
        );

        let press = tokio::spawn(execute_live_press_sequence(
            handle.clone(),
            keys,
            50,
            Backend::Software,
            None,
        ));
        let before_release = wait_for_held_key(&snapshot_handle, "a").await;
        println!(
            "source_of_truth=act_press_live_sequence edge=mid_hold_release before_release={before_release:?}"
        );

        handle
            .execute(Action::ReleaseAll)
            .await
            .unwrap_or_else(|error| panic!("release_all should execute during hold: {error}"));
        let after_release = snapshot_handle
            .snapshot()
            .await
            .unwrap_or_else(|error| panic!("snapshot after release_all should succeed: {error}"));
        println!(
            "source_of_truth=act_press_live_sequence edge=mid_hold_release after_release={after_release:?}"
        );
        assert!(after_release.held_keys.is_empty());

        press
            .await
            .unwrap_or_else(|error| panic!("press task should join: {error}"))
            .unwrap_or_else(|error| {
                panic!("press task should tolerate prior release_all: {error}")
            });
        let final_events = recording.events();
        println!(
            "source_of_truth=act_press_live_sequence edge=mid_hold_release after_events={final_events:?}"
        );
        assert!(
            final_events
                .iter()
                .any(|event| matches!(event, RecordedInput::ReleaseAll { .. }))
        );

        cancel.cancel();
        let final_snapshot = join
            .await
            .unwrap_or_else(|error| panic!("emitter should join: {error}"));
        assert!(final_snapshot.held_keys.is_empty());
    }

    #[test]
    fn defaults_are_issue_required_values() {
        assert_eq!(default_hold_ms(), 33);
        assert_eq!(default_press_backend(), PressBackend::Auto);
    }

    #[test]
    fn normalized_keys_are_modifier_ordered() {
        let before = vec!["super".to_owned(), "s".to_owned(), "ctrl".to_owned()];
        println!("source_of_truth=act_press_keys edge=modifier_order before={before:?}");
        let after = normalized_keys(&before)
            .unwrap_or_else(|error| panic!("keys should normalize: {error}"));
        let labels = after
            .iter()
            .map(|key| match &key.code {
                synapse_core::KeyCode::Named { value } => value.as_str(),
                _ => "",
            })
            .collect::<Vec<_>>();
        println!("source_of_truth=act_press_keys edge=modifier_order after={labels:?}");
        assert_eq!(labels, ["ctrl", "super", "s"]);
    }

    #[test]
    fn event_sequence_reads_recording_events() {
        let before = vec![
            RecordedInput::KeyDown { key: key("ctrl") },
            RecordedInput::DelayMs { ms: 33 },
            RecordedInput::KeyUp { key: key("ctrl") },
        ];
        let after = event_sequence(&before);
        println!(
            "source_of_truth=act_press_recording edge=event_sequence before={before:?} after={after}"
        );
        assert_eq!(after, "down:ctrl>delay:33>up:ctrl");
    }

    async fn wait_for_held_key(
        snapshot_handle: &synapse_action::ActionEmitterSnapshotHandle,
        key_name: &str,
    ) -> synapse_action::ActionStateSnapshot {
        for _ in 0..50 {
            let snapshot = snapshot_handle
                .snapshot()
                .await
                .unwrap_or_else(|error| panic!("snapshot should succeed: {error}"));
            if snapshot.held_keys.iter().any(|key| match &key.code {
                synapse_core::KeyCode::Named { value } => value == key_name,
                _ => false,
            }) {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("timed out waiting for held key {key_name}");
    }
}
