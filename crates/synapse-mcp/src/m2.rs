mod aim;
mod click;
mod clipboard;
mod drag;
mod pad;
mod press;
mod release_all;
mod scroll;
mod type_text;

use std::{
    fmt,
    sync::{Arc, Mutex},
};

use synapse_action::{
    ActionBackend, ActionEmitter, ActionEmitterSnapshotHandle, ActionHandle, ActionStateSnapshot,
    RELEASE_ALL_HANDLE, RecordingBackend, initialize_double_click_timing_cache,
};
use tokio::{sync::watch, task::JoinHandle};
use tokio_util::sync::CancellationToken;

pub use aim::{ActAimParams, ActAimResponse, act_aim_with_handle};
pub use click::{ActClickParams, ActClickResponse, act_click_with_handle};
pub use clipboard::{ActClipboardParams, ActClipboardResponse, act_clipboard};
pub use drag::{ActDragParams, ActDragResponse, act_drag_with_handle};
pub use pad::{ActPadParams, ActPadResponse, act_pad_with_handle};
pub use press::{ActPressParams, ActPressResponse, act_press_with_handle};
pub use release_all::{ReleaseAllParams, ReleaseAllResponse, release_all_with_handles};
pub use scroll::{ActScrollParams, ActScrollResponse, act_scroll_with_handle};
pub use type_text::{ActTypeParams, ActTypeResponse, act_type_with_handle};

const RECORDING_BACKEND_ENV: &str = "SYNAPSE_MCP_RECORDING_BACKEND";

pub type SharedM2State = Arc<Mutex<M2State>>;

pub struct M2State {
    pub emitter_handle: ActionHandle,
    pub snapshot_handle: ActionEmitterSnapshotHandle,
    pub recording: Option<Arc<RecordingBackend>>,
    pub connection_closed_cancel: Option<CancellationToken>,
    retained_emitter: Option<ActionEmitter>,
    emitter_cancel: Option<CancellationToken>,
    emitter_task: Option<JoinHandle<ActionStateSnapshot>>,
    emitter_done: Option<watch::Receiver<Option<ActionStateSnapshot>>>,
}

impl M2State {
    #[must_use]
    pub fn from_env() -> Self {
        let recording_backend = std::env::var(RECORDING_BACKEND_ENV).ok();
        Self::from_recording_backend_env(recording_backend.as_deref())
    }

    #[must_use]
    pub fn from_env_with_shutdown_reason(
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: Option<CancellationToken>,
    ) -> Self {
        let recording_backend = std::env::var(RECORDING_BACKEND_ENV).ok();
        Self::from_recording_backend_env_with_shutdown_tokens(
            recording_backend.as_deref(),
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
        )
    }

    #[must_use]
    pub fn from_recording_backend_env(recording_backend: Option<&str>) -> Self {
        Self::from_recording_backend_env_with_cancel(recording_backend, CancellationToken::new())
    }

    #[must_use]
    pub fn from_recording_backend_env_with_cancel(
        recording_backend: Option<&str>,
        emitter_cancel: CancellationToken,
    ) -> Self {
        Self::from_recording_backend_env_with_shutdown_tokens(
            recording_backend,
            emitter_cancel,
            "shutdown",
            None,
        )
    }

    #[must_use]
    pub fn from_recording_backend_env_with_shutdown_tokens(
        recording_backend: Option<&str>,
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: Option<CancellationToken>,
    ) -> Self {
        Self::from_recording_backend_env_with_actor_backend(
            recording_backend,
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
            None,
        )
    }

    /// Lower-level constructor that lets callers (notably cross-platform
    /// tests) substitute the actor's `ActionBackend` for one that does not
    /// require the production OS — e.g. `RecordingBackend`. Production code
    /// passes `actor_backend = None` and gets the platform-native backends.
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub fn from_recording_backend_env_with_actor_backend(
        recording_backend: Option<&str>,
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: Option<CancellationToken>,
        actor_backend: Option<Arc<dyn ActionBackend>>,
    ) -> Self {
        let double_click_timing = initialize_double_click_timing_cache();
        tracing::info!(
            code = "M2_DOUBLE_CLICK_TIMING_CACHED",
            window_ms = double_click_timing.window_ms,
            inter_click_delay_ms = double_click_timing.inter_click_delay_ms,
            source = double_click_timing.source,
            "source_of_truth=double_click_timing after_cache_readback"
        );
        let recording =
            recording_backend_enabled(recording_backend).then(|| Arc::new(RecordingBackend::new()));
        let tool_connection_closed_cancel = connection_closed_cancel.clone();
        let build_emitter = || {
            actor_backend
                .as_ref()
                .map_or_else(ActionEmitter::channel, |backend| {
                    ActionEmitter::channel_with_backend(Arc::clone(backend))
                })
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            let (emitter_handle, snapshot_handle, emitter) = build_emitter();
            let _release_handle_result = RELEASE_ALL_HANDLE.set(emitter_handle.clone());
            let (done_tx, done_rx) = watch::channel(None);
            let emitter_task = tokio::spawn(async move {
                let snapshot = emitter
                    .run_with_shutdown_reason(
                        shutdown_cancel,
                        shutdown_reason,
                        connection_closed_cancel,
                    )
                    .await;
                let _send_result = done_tx.send(Some(snapshot.clone()));
                snapshot
            });
            return Self {
                emitter_handle,
                snapshot_handle,
                recording,
                connection_closed_cancel: tool_connection_closed_cancel,
                retained_emitter: None,
                emitter_cancel: None,
                emitter_task: Some(emitter_task),
                emitter_done: Some(done_rx),
            };
        }

        let (emitter_handle, snapshot_handle, emitter) = build_emitter();
        Self {
            emitter_handle,
            snapshot_handle,
            recording,
            connection_closed_cancel: tool_connection_closed_cancel,
            retained_emitter: Some(emitter),
            emitter_cancel: None,
            emitter_task: None,
            emitter_done: None,
        }
    }

    #[must_use]
    pub const fn recording_enabled(&self) -> bool {
        self.recording.is_some()
    }

    #[must_use]
    pub const fn emitter_retained(&self) -> bool {
        self.retained_emitter.is_some()
    }

    #[must_use]
    pub fn emitter_running(&self) -> bool {
        self.emitter_task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    }

    #[must_use]
    pub fn emitter_available(&self) -> bool {
        self.emitter_retained() || self.emitter_running()
    }

    #[must_use]
    pub fn emitter_done_receiver(&self) -> Option<watch::Receiver<Option<ActionStateSnapshot>>> {
        self.emitter_done.clone()
    }
}

impl Default for M2State {
    fn default() -> Self {
        Self::from_env()
    }
}

impl fmt::Debug for M2State {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("M2State")
            .field("emitter_handle", &self.emitter_handle)
            .field("snapshot_handle", &self.snapshot_handle)
            .field("recording", &self.recording_enabled())
            .field(
                "connection_closed_cancel",
                &self.connection_closed_cancel.is_some(),
            )
            .field("retained_emitter", &self.emitter_retained())
            .field("emitter_cancel", &self.emitter_cancel.is_some())
            .field("emitter_task", &self.emitter_running())
            .field("emitter_done", &self.emitter_done.is_some())
            .field("emitter_available", &self.emitter_available())
            .finish()
    }
}

#[must_use]
pub fn shared_m2_state_from_env() -> SharedM2State {
    Arc::new(Mutex::new(M2State::from_env()))
}

#[must_use]
pub fn shared_m2_state_from_env_with_shutdown_reason(
    shutdown_cancel: CancellationToken,
    shutdown_reason: &'static str,
    connection_closed_cancel: Option<CancellationToken>,
) -> SharedM2State {
    Arc::new(Mutex::new(M2State::from_env_with_shutdown_reason(
        shutdown_cancel,
        shutdown_reason,
        connection_closed_cancel,
    )))
}

#[must_use]
pub fn recording_backend_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

#[cfg(test)]
mod tests {
    use synapse_core::{Action, Backend, Key, KeyCode};
    use tokio_util::sync::CancellationToken;

    use super::{M2State, RECORDING_BACKEND_ENV, recording_backend_enabled};

    #[test]
    fn from_env_reads_recording_backend_env() {
        let before = std::env::var(RECORDING_BACKEND_ENV).ok();
        let expected = recording_backend_enabled(before.as_deref());
        let state = M2State::from_env();
        let event_count = state
            .recording
            .as_ref()
            .map_or(0, |recording| recording.events().len());
        println!(
            "source_of_truth=m2_state scenario=from_env before_env={before:?} expected_recording={expected} after_recording_enabled={} emitter_retained={} emitter_available={} recording_event_count={event_count}",
            state.recording_enabled(),
            state.emitter_retained(),
            state.emitter_available()
        );
        assert_eq!(state.recording_enabled(), expected);
        assert!(state.emitter_available());
        assert_eq!(event_count, 0);
    }

    #[tokio::test]
    async fn m2_state_spawns_running_emitter_inside_runtime() {
        let before = Some("true");
        let state = M2State::from_recording_backend_env(before);
        let snapshot = match state.snapshot_handle.snapshot().await {
            Ok(snapshot) => snapshot,
            Err(err) => panic!("snapshot failed: {err}"),
        };
        println!(
            "source_of_truth=m2_state scenario=runtime_actor before_env={before:?} after_recording_enabled={} emitter_retained={} emitter_running={} held_keys={:?} held_key_timer_count={} held_buttons={:?} pad_state_len={}",
            state.recording_enabled(),
            state.emitter_retained(),
            state.emitter_running(),
            snapshot.held_keys,
            snapshot.held_key_timer_count,
            snapshot.held_buttons,
            snapshot.pad_state.len()
        );
        assert!(state.recording_enabled());
        assert!(!state.emitter_retained());
        assert!(state.emitter_running());
        assert!(snapshot.held_keys.is_empty());
        assert_eq!(snapshot.held_key_timer_count, 0);
        assert!(snapshot.held_buttons.is_empty());
        assert!(snapshot.pad_state.is_empty());
    }

    #[tokio::test]
    async fn m2_state_uses_injected_cancel_token_to_release_all_on_shutdown() {
        let before = Some("false");
        let cancel = CancellationToken::new();
        let substitute: std::sync::Arc<dyn synapse_action::ActionBackend> =
            std::sync::Arc::new(synapse_action::RecordingBackend::new());
        let mut state = M2State::from_recording_backend_env_with_actor_backend(
            before,
            cancel.clone(),
            "shutdown",
            None,
            Some(substitute),
        );
        let key = key_named("m2-cancel-token");
        println!(
            "source_of_truth=m2_state scenario=injected_cancel before_cancelled:{} emitter_running:{}",
            cancel.is_cancelled(),
            state.emitter_running()
        );

        state
            .emitter_handle
            .execute(Action::KeyDown {
                key: key.clone(),
                backend: Backend::Software,
            })
            .await
            .unwrap_or_else(|error| panic!("KeyDown should reach emitter before cancel: {error}"));
        let before_cancel = state
            .snapshot_handle
            .snapshot()
            .await
            .unwrap_or_else(|error| panic!("snapshot before cancel should succeed: {error}"));
        assert_eq!(before_cancel.held_keys, vec![key.clone()]);
        let done = state
            .emitter_done_receiver()
            .unwrap_or_else(|| panic!("runtime M2 state should expose emitter done receiver"));

        cancel.cancel();
        let join = state
            .emitter_task
            .take()
            .unwrap_or_else(|| panic!("runtime M2 state should retain emitter join handle"));
        let after_cancel = join
            .await
            .unwrap_or_else(|error| panic!("emitter join should complete after cancel: {error}"));
        let done_snapshot = done
            .borrow()
            .clone()
            .unwrap_or_else(|| panic!("emitter done receiver should contain final snapshot"));

        println!(
            "source_of_truth=m2_state scenario=injected_cancel after_cancelled:{} before_held_keys:{:?} after_held_keys:{:?} done_held_keys:{:?} after_timer_count:{} after_buttons:{:?} after_pad_state_len:{}",
            cancel.is_cancelled(),
            before_cancel.held_keys,
            after_cancel.held_keys,
            done_snapshot.held_keys,
            after_cancel.held_key_timer_count,
            after_cancel.held_buttons,
            after_cancel.pad_state.len()
        );
        assert!(cancel.is_cancelled());
        assert!(after_cancel.held_keys.is_empty());
        assert_eq!(done_snapshot, after_cancel);
        assert_eq!(after_cancel.held_key_timer_count, 0);
        assert!(after_cancel.held_buttons.is_empty());
        assert!(after_cancel.pad_state.is_empty());
    }

    #[test]
    fn recording_backend_env_parser_handles_happy_path_and_edges() {
        let cases = [
            ("happy_one", Some("1"), true),
            ("happy_true_uppercase", Some("TRUE"), true),
            ("edge_absent", None, false),
            ("edge_empty", Some(""), false),
            ("edge_false", Some("false"), false),
            ("edge_whitespace", Some(" true "), false),
            ("edge_invalid", Some("record"), false),
        ];
        for (name, before, expected) in cases {
            let state = M2State::from_recording_backend_env(before);
            let event_count = state
                .recording
                .as_ref()
                .map_or(0, |recording| recording.events().len());
            println!(
                "source_of_truth=m2_state scenario={name} before_env={before:?} expected_recording={expected} after_recording_enabled={} emitter_retained={} emitter_available={} recording_event_count={event_count}",
                state.recording_enabled(),
                state.emitter_retained(),
                state.emitter_available()
            );
            assert_eq!(state.recording_enabled(), expected);
            assert!(state.emitter_available());
            assert_eq!(event_count, 0);
        }
    }

    fn key_named(name: &str) -> Key {
        Key {
            code: KeyCode::Named {
                value: name.to_owned(),
            },
            use_scancode: false,
        }
    }
}
