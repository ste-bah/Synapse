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
    ActionEmitter, ActionEmitterSnapshotHandle, ActionHandle, ActionStateSnapshot, RecordingBackend,
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub use aim::{ActAimParams, ActAimResponse, act_aim_with_handle};
pub use click::{ActClickParams, ActClickResponse, act_click_with_handle};
pub use press::{ActPressParams, ActPressResponse, act_press_with_handle};
pub use type_text::{ActTypeParams, ActTypeResponse, act_type_with_handle};

const RECORDING_BACKEND_ENV: &str = "SYNAPSE_MCP_RECORDING_BACKEND";

pub type SharedM2State = Arc<Mutex<M2State>>;

pub struct M2State {
    pub emitter_handle: ActionHandle,
    pub snapshot_handle: ActionEmitterSnapshotHandle,
    pub recording: Option<Arc<RecordingBackend>>,
    retained_emitter: Option<ActionEmitter>,
    emitter_cancel: Option<CancellationToken>,
    emitter_task: Option<JoinHandle<ActionStateSnapshot>>,
}

impl M2State {
    #[must_use]
    pub fn from_env() -> Self {
        let recording_backend = std::env::var(RECORDING_BACKEND_ENV).ok();
        Self::from_recording_backend_env(recording_backend.as_deref())
    }

    #[must_use]
    pub fn from_recording_backend_env(recording_backend: Option<&str>) -> Self {
        let recording =
            recording_backend_enabled(recording_backend).then(|| Arc::new(RecordingBackend::new()));
        if tokio::runtime::Handle::try_current().is_ok() {
            let emitter_cancel = CancellationToken::new();
            let (emitter_handle, snapshot_handle, emitter_task) =
                ActionEmitter::spawn(emitter_cancel.clone());
            return Self {
                emitter_handle,
                snapshot_handle,
                recording,
                retained_emitter: None,
                emitter_cancel: Some(emitter_cancel),
                emitter_task: Some(emitter_task),
            };
        }

        let (emitter_handle, snapshot_handle, emitter) = ActionEmitter::channel();
        Self {
            emitter_handle,
            snapshot_handle,
            recording,
            retained_emitter: Some(emitter),
            emitter_cancel: None,
            emitter_task: None,
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
            .field("retained_emitter", &self.emitter_retained())
            .field("emitter_cancel", &self.emitter_cancel.is_some())
            .field("emitter_task", &self.emitter_running())
            .field("emitter_available", &self.emitter_available())
            .finish()
    }
}

impl Drop for M2State {
    fn drop(&mut self) {
        if let Some(cancel) = &self.emitter_cancel {
            cancel.cancel();
        }
    }
}

#[must_use]
pub fn shared_m2_state_from_env() -> SharedM2State {
    Arc::new(Mutex::new(M2State::from_env()))
}

#[must_use]
pub fn recording_backend_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

#[cfg(test)]
mod tests {
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
}
