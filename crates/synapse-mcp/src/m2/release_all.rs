use std::time::Instant;

use rmcp::ErrorData;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_action::{ActionEmitterSnapshotHandle, ActionError, ActionHandle, ActionStateSnapshot};
use synapse_core::{Action, error_codes};

use crate::m1::mcp_error;

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAllParams {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAllResponse {
    pub released_keys: u32,
    pub released_buttons: u32,
    pub neutralized_pads: u32,
}

pub async fn release_all_with_handles(
    handle: ActionHandle,
    snapshot_handle: ActionEmitterSnapshotHandle,
    _params: ReleaseAllParams,
) -> Result<ReleaseAllResponse, ErrorData> {
    let started = Instant::now();
    let before = snapshot_handle
        .snapshot()
        .await
        .map_err(|error| action_error_to_mcp(&error))?;
    let response = response_from_snapshot(&before)?;

    handle
        .execute(Action::ReleaseAll)
        .await
        .map_err(|error| action_error_to_mcp(&error))?;

    let after = snapshot_handle
        .snapshot()
        .await
        .map_err(|error| action_error_to_mcp(&error))?;
    ensure_drained(&after)?;

    tracing::info!(
        code = "M2_RELEASE_ALL_READBACK",
        kind = "release_all",
        released_keys = response.released_keys,
        released_buttons = response.released_buttons,
        neutralized_pads = response.neutralized_pads,
        before_held_keys = ?before.held_keys,
        before_held_buttons = ?before.held_buttons,
        before_pad_state_len = before.pad_state.len(),
        after_held_keys = ?after.held_keys,
        after_held_buttons = ?after.held_buttons,
        after_pad_state_len = after.pad_state.len(),
        elapsed_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        "source_of_truth=action_emitter_state tool=release_all after_snapshot_readback"
    );

    Ok(response)
}

fn response_from_snapshot(snapshot: &ActionStateSnapshot) -> Result<ReleaseAllResponse, ErrorData> {
    Ok(ReleaseAllResponse {
        released_keys: count_to_u32(snapshot.held_keys.len(), "held_keys")?,
        released_buttons: count_to_u32(snapshot.held_buttons.len(), "held_buttons")?,
        neutralized_pads: count_to_u32(snapshot.pad_state.len(), "pad_state")?,
    })
}

fn ensure_drained(snapshot: &ActionStateSnapshot) -> Result<(), ErrorData> {
    if snapshot.held_keys.is_empty()
        && snapshot.held_buttons.is_empty()
        && snapshot.pad_state.is_empty()
        && snapshot.held_key_timer_count == 0
    {
        return Ok(());
    }

    Err(mcp_error(
        error_codes::TOOL_INTERNAL_ERROR,
        format!("release_all did not drain held state: {snapshot:?}"),
    ))
}

fn count_to_u32(value: usize, field: &str) -> Result<u32, ErrorData> {
    u32::try_from(value).map_err(|_err| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("release_all {field} count exceeds u32::MAX"),
        )
    })
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio_util::sync::CancellationToken;

    use synapse_action::{ActionBackend, ActionEmitter, RecordingBackend};
    use synapse_core::{
        Backend, ButtonAction, GamepadReport, Key, KeyCode, MouseButton, PadButton,
    };

    use super::{ReleaseAllParams, release_all_with_handles};

    #[tokio::test]
    async fn release_all_counts_and_drains_actor_state() {
        let cancel = CancellationToken::new();
        let backend: Arc<dyn ActionBackend> = Arc::new(RecordingBackend::new());
        let (handle, snapshot_handle, join) =
            ActionEmitter::spawn_with_backend(cancel.clone(), backend);
        let keys = [key("ctrl"), key("shift"), key("alt")];
        for key in &keys {
            handle
                .execute(synapse_core::Action::KeyDown {
                    key: key.clone(),
                    backend: Backend::Software,
                })
                .await
                .unwrap_or_else(|error| panic!("prime key should succeed: {error}"));
        }
        handle
            .execute(synapse_core::Action::MouseButton {
                button: MouseButton::Left,
                action: ButtonAction::Down,
                hold_ms: 0,
                backend: Backend::Software,
            })
            .await
            .unwrap_or_else(|error| panic!("prime mouse button should succeed: {error}"));
        let report = GamepadReport {
            buttons: vec![PadButton::A],
            thumb_l: (0.5, -0.5),
            thumb_r: (0.0, 0.0),
            lt: 0.25,
            rt: 0.0,
        };
        handle
            .execute(synapse_core::Action::PadReport { pad: 1, report })
            .await
            .unwrap_or_else(|error| panic!("prime pad should succeed: {error}"));

        let before = snapshot_handle
            .snapshot()
            .await
            .unwrap_or_else(|error| panic!("snapshot before release_all should succeed: {error}"));
        println!(
            "source_of_truth=action_emitter_state tool=release_all edge=happy before={before:?}"
        );
        assert_eq!(before.held_keys.len(), 3);
        assert_eq!(before.held_buttons, vec![MouseButton::Left]);
        assert_eq!(before.pad_state.len(), 1);

        let response =
            release_all_with_handles(handle.clone(), snapshot_handle.clone(), ReleaseAllParams {})
                .await
                .unwrap_or_else(|error| panic!("release_all should succeed: {error}"));
        assert_eq!(response.released_keys, 3);
        assert_eq!(response.released_buttons, 1);
        assert_eq!(response.neutralized_pads, 1);

        let after = snapshot_handle
            .snapshot()
            .await
            .unwrap_or_else(|error| panic!("snapshot after release_all should succeed: {error}"));
        println!(
            "source_of_truth=action_emitter_state tool=release_all edge=happy after={after:?} response={response:?}"
        );
        assert!(after.held_keys.is_empty());
        assert!(after.held_buttons.is_empty());
        assert!(after.pad_state.is_empty());
        assert_eq!(after.held_key_timer_count, 0);

        cancel.cancel();
        let final_snapshot = join
            .await
            .unwrap_or_else(|error| panic!("emitter task should join: {error}"));
        assert!(final_snapshot.held_keys.is_empty());
        assert!(final_snapshot.held_buttons.is_empty());
        assert!(final_snapshot.pad_state.is_empty());
    }

    fn key(value: &str) -> Key {
        Key {
            code: KeyCode::Named {
                value: value.to_owned(),
            },
            use_scancode: false,
        }
    }
}
