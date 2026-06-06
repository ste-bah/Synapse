mod click;
mod clipboard;
mod config;
mod focus_window;
mod pad;
pub(crate) mod postcondition;
mod press;
mod release_all;
mod scroll;
mod set_value;
mod stroke;
mod type_text;

use std::{
    fmt,
    sync::{Arc, Mutex, RwLock},
};

use synapse_action::{
    ActionBackend, ActionEmitter, ActionEmitterSnapshotHandle, ActionHandle, ActionStateSnapshot,
    BackendRateLimitControl, BackendResolutionPolicy, RELEASE_ALL_HANDLE, RecordingBackend,
    initialize_double_click_timing_cache,
};
use tokio::{sync::watch, task::JoinHandle};
use tokio_util::sync::CancellationToken;

pub use click::{ActClickParams, ActClickPostcondition, ActClickResponse, act_click_with_handle};
pub(crate) use click::{
    ActClickTierAttempt, CLICK_REASON_NO_OBSERVED_DELTA, act_click_postmessage_with_params,
    attach_click_tier_attempts, click_params_can_route_background_first, click_tier_failed,
};
#[cfg(test)]
pub use clipboard::ActClipboardFormat;
pub use clipboard::{ActClipboardParams, ActClipboardResponse, ActClipboardVerb, act_clipboard};
pub use config::M2ServiceConfig;
pub use focus_window::{
    ActFocusWindowParams, ActFocusWindowResponse, act_focus_window,
    act_focus_window_request_details,
};
pub use pad::{ActPadParams, ActPadResponse, act_pad_with_handle};
pub use postcondition::default_verify_timeout_ms;
pub use press::action_from_press_params;
pub use press::{
    ActKeymapParams, ActKeymapResponse, ActPressParams, ActPressResponse, PressBackend,
    act_keymap_with_handle, act_press_with_handle,
};
pub use release_all::{ReleaseAllParams, ReleaseAllResponse, release_all_with_handles};
pub use scroll::{ActScrollParams, ActScrollResponse, act_scroll_with_handle};
pub use set_value::{
    ActSetValueParams, ActSetValueResponse, act_set_value, act_set_value_request_details,
};
pub use stroke::{
    ActStrokeParams, ActStrokeResponse, act_stroke_error_details, act_stroke_request_details,
    act_stroke_validation_failure_details, act_stroke_with_handle, validate_act_stroke_params,
};
pub use type_text::action_from_type_params;
pub(crate) use type_text::emitted_text;
pub use type_text::{ActTypeParams, ActTypeResponse, act_type_with_handle};

use config::RECORDING_BACKEND_ENV;

pub type SharedM2State = Arc<Mutex<M2State>>;

pub struct M2State {
    pub emitter_handle: ActionHandle,
    pub snapshot_handle: ActionEmitterSnapshotHandle,
    pub rate_limit_control: BackendRateLimitControl,
    pub recording: Option<Arc<RecordingBackend>>,
    pub connection_closed_cancel: Option<CancellationToken>,
    backend_resolution: Arc<RwLock<BackendResolutionPolicy>>,
    backend_resolution_source: String,
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

    pub fn try_from_env() -> anyhow::Result<Self> {
        Self::try_from_config(&M2ServiceConfig::from_env())
    }

    pub fn try_from_config(config: &M2ServiceConfig) -> anyhow::Result<Self> {
        Self::try_from_config_with_shutdown_tokens(
            config,
            CancellationToken::new(),
            "shutdown",
            None,
        )
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "keeps the fallible constructor contract aligned with try_from_env/try_from_config"
    )]
    pub fn try_from_config_with_shutdown_tokens(
        config: &M2ServiceConfig,
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: Option<CancellationToken>,
    ) -> anyhow::Result<Self> {
        Ok(Self::from_recording_backend_env_with_configured_backends(
            config.recording_backend.as_deref(),
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
            None,
            None,
        ))
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
        Self::from_recording_backend_env_with_configured_backends(
            recording_backend,
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
            actor_backend,
            None,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    fn from_recording_backend_env_with_configured_backends(
        recording_backend: Option<&str>,
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: Option<CancellationToken>,
        actor_backend: Option<Arc<dyn ActionBackend>>,
        action_backends: Option<synapse_action::Backends>,
    ) -> Self {
        let double_click_timing = initialize_double_click_timing_cache();
        tracing::info!(
            code = "M2_DOUBLE_CLICK_TIMING_CACHED",
            window_ms = double_click_timing.window_ms,
            inter_click_delay_ms = double_click_timing.inter_click_delay_ms,
            source = double_click_timing.source,
            "readback=double_click_timing after_cache_readback"
        );
        let recording =
            recording_backend_enabled(recording_backend).then(|| Arc::new(RecordingBackend::new()));
        let actor_backend = actor_backend.or_else(|| {
            recording
                .as_ref()
                .map(|recording| Arc::clone(recording) as Arc<dyn ActionBackend>)
        });
        let backend_resolution = Arc::new(RwLock::new(BackendResolutionPolicy::default()));
        let tool_connection_closed_cancel = connection_closed_cancel.clone();
        let (emitter_handle, snapshot_handle, emitter) = actor_backend.map_or_else(
            || {
                action_backends.map_or_else(
                    || {
                        ActionEmitter::channel_with_backends_and_policy(
                            synapse_action::Backends::production(),
                            Arc::clone(&backend_resolution),
                        )
                    },
                    |backends| {
                        ActionEmitter::channel_with_backends_and_policy(
                            backends,
                            Arc::clone(&backend_resolution),
                        )
                    },
                )
            },
            |backend| {
                ActionEmitter::channel_with_backends_and_policy(
                    synapse_action::Backends::all_routed_to(backend),
                    Arc::clone(&backend_resolution),
                )
            },
        );
        let rate_limit_control = emitter.rate_limit_control();
        if tokio::runtime::Handle::try_current().is_ok() {
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
                rate_limit_control,
                recording,
                connection_closed_cancel: tool_connection_closed_cancel,
                backend_resolution,
                backend_resolution_source: "global_default".to_owned(),
                retained_emitter: None,
                emitter_cancel: None,
                emitter_task: Some(emitter_task),
                emitter_done: Some(done_rx),
            };
        }

        Self {
            emitter_handle,
            snapshot_handle,
            rate_limit_control,
            recording,
            connection_closed_cancel: tool_connection_closed_cancel,
            backend_resolution,
            backend_resolution_source: "global_default".to_owned(),
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

    #[must_use]
    pub fn backend_resolution_source(&self) -> &str {
        &self.backend_resolution_source
    }

    pub fn backend_resolution_readback(&self) -> Result<(String, BackendResolutionPolicy), String> {
        self.backend_resolution
            .read()
            .map(|policy| (self.backend_resolution_source.clone(), *policy))
            .map_err(|_err| "backend resolution policy lock poisoned".to_owned())
    }

    pub fn set_backend_resolution(
        &mut self,
        source: String,
        policy: BackendResolutionPolicy,
    ) -> Result<(), String> {
        let mut guard = self
            .backend_resolution
            .write()
            .map_err(|_err| "backend resolution policy lock poisoned".to_owned())?;
        *guard = policy;
        drop(guard);
        self.backend_resolution_source = source;
        Ok(())
    }
}

impl Default for M2State {
    fn default() -> Self {
        Self::from_env()
    }
}

impl fmt::Debug for M2State {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let backend_resolution = self.backend_resolution_readback().ok();
        formatter
            .debug_struct("M2State")
            .field("emitter_handle", &self.emitter_handle)
            .field("snapshot_handle", &self.snapshot_handle)
            .field(
                "rate_limit_control",
                &self.rate_limit_control.try_snapshot().ok(),
            )
            .field("recording", &self.recording_enabled())
            .field(
                "connection_closed_cancel",
                &self.connection_closed_cancel.is_some(),
            )
            .field("backend_resolution", &backend_resolution)
            .field(
                "backend_resolution_source",
                &self.backend_resolution_source(),
            )
            .field("retained_emitter", &self.emitter_retained())
            .field("emitter_cancel", &self.emitter_cancel.is_some())
            .field("emitter_task", &self.emitter_running())
            .field("emitter_done", &self.emitter_done.is_some())
            .field("emitter_available", &self.emitter_available())
            .finish()
    }
}

pub fn shared_m2_state_from_env() -> anyhow::Result<SharedM2State> {
    Ok(Arc::new(Mutex::new(M2State::try_from_env()?)))
}

pub fn shared_m2_state_from_config_with_shutdown_reason(
    config: &M2ServiceConfig,
    shutdown_cancel: CancellationToken,
    shutdown_reason: &'static str,
    connection_closed_cancel: Option<CancellationToken>,
) -> anyhow::Result<SharedM2State> {
    Ok(Arc::new(Mutex::new(
        M2State::try_from_config_with_shutdown_tokens(
            config,
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
        )?,
    )))
}
#[must_use]
pub fn recording_backend_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

#[cfg(test)]
mod tests;
