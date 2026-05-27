use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use super::{
    ActionEmitter, ActionEmitterSnapshotHandle, ActionSnapshotMessage, ActionStateSnapshot,
    BackendRateLimits, Backends, EmitState,
};
use crate::{
    ACTION_QUEUE_CAPACITY, ActionBackend, ActionHandle, ActionMessage, BackendResolutionPolicy,
};

impl ActionEmitter {
    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "new"))]
    pub fn new(
        rx: mpsc::Receiver<ActionMessage>,
        snapshot_rx: mpsc::Receiver<ActionSnapshotMessage>,
    ) -> Self {
        Self::new_with_backends(rx, snapshot_rx, Backends::production())
    }

    #[must_use]
    pub fn new_with_backends(
        rx: mpsc::Receiver<ActionMessage>,
        snapshot_rx: mpsc::Receiver<ActionSnapshotMessage>,
        backends: Backends,
    ) -> Self {
        Self::new_with_backends_and_policy(
            rx,
            snapshot_rx,
            backends,
            Arc::new(RwLock::new(BackendResolutionPolicy::default())),
        )
    }

    #[must_use]
    pub fn new_with_backends_and_policy(
        rx: mpsc::Receiver<ActionMessage>,
        snapshot_rx: mpsc::Receiver<ActionSnapshotMessage>,
        backends: Backends,
        backend_resolution: Arc<RwLock<BackendResolutionPolicy>>,
    ) -> Self {
        let (auto_release_tx, auto_release_rx) = mpsc::channel(ACTION_QUEUE_CAPACITY);
        Self {
            rx,
            snapshot_rx,
            auto_release_tx,
            auto_release_rx,
            state: EmitState::new(),
            backends,
            backend_resolution,
            rate_limits: BackendRateLimits::new(),
            held_key_timers: HashMap::new(),
            held_key_timer_ids: HashMap::new(),
            next_held_key_timer_id: 0,
        }
    }

    #[cfg(test)]
    pub(super) fn with_rate_limits(
        rx: mpsc::Receiver<ActionMessage>,
        snapshot_rx: mpsc::Receiver<ActionSnapshotMessage>,
        backends: Backends,
        rate_limits: BackendRateLimits,
    ) -> Self {
        Self::with_rate_limits_and_policy(
            rx,
            snapshot_rx,
            backends,
            rate_limits,
            Arc::new(RwLock::new(BackendResolutionPolicy::default())),
        )
    }

    #[cfg(test)]
    pub(super) fn with_rate_limits_and_policy(
        rx: mpsc::Receiver<ActionMessage>,
        snapshot_rx: mpsc::Receiver<ActionSnapshotMessage>,
        backends: Backends,
        rate_limits: BackendRateLimits,
        backend_resolution: Arc<RwLock<BackendResolutionPolicy>>,
    ) -> Self {
        let (auto_release_tx, auto_release_rx) = mpsc::channel(ACTION_QUEUE_CAPACITY);
        Self {
            rx,
            snapshot_rx,
            auto_release_tx,
            auto_release_rx,
            state: EmitState::new(),
            backends,
            backend_resolution,
            rate_limits,
            held_key_timers: HashMap::new(),
            held_key_timer_ids: HashMap::new(),
            next_held_key_timer_id: 0,
        }
    }

    #[cfg(test)]
    pub(super) fn channel_with_rate_limits(
        rate_limits: BackendRateLimits,
    ) -> (ActionHandle, ActionEmitterSnapshotHandle, Self) {
        let backends = Backends::all_routed_to(Arc::new(crate::RecordingBackend::new()));
        let (handle, rx) = ActionHandle::channel();
        let (snapshot_tx, snapshot_rx) = mpsc::channel(ACTION_QUEUE_CAPACITY);
        (
            handle,
            ActionEmitterSnapshotHandle::new(snapshot_tx),
            Self::with_rate_limits(rx, snapshot_rx, backends, rate_limits),
        )
    }

    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "channel"))]
    pub fn channel() -> (ActionHandle, ActionEmitterSnapshotHandle, Self) {
        Self::channel_with_backends(Backends::production())
    }

    #[must_use]
    pub fn channel_with_backends(
        backends: Backends,
    ) -> (ActionHandle, ActionEmitterSnapshotHandle, Self) {
        Self::channel_with_backends_and_policy(
            backends,
            Arc::new(RwLock::new(BackendResolutionPolicy::default())),
        )
    }

    #[must_use]
    pub fn channel_with_backends_and_policy(
        backends: Backends,
        backend_resolution: Arc<RwLock<BackendResolutionPolicy>>,
    ) -> (ActionHandle, ActionEmitterSnapshotHandle, Self) {
        let (handle, rx) = ActionHandle::channel();
        let (snapshot_tx, snapshot_rx) = mpsc::channel(ACTION_QUEUE_CAPACITY);
        (
            handle,
            ActionEmitterSnapshotHandle::new(snapshot_tx),
            Self::new_with_backends_and_policy(rx, snapshot_rx, backends, backend_resolution),
        )
    }

    #[must_use]
    pub fn channel_with_backend(
        backend: Arc<dyn ActionBackend>,
    ) -> (ActionHandle, ActionEmitterSnapshotHandle, Self) {
        Self::channel_with_backends(Backends::all_routed_to(backend))
    }

    /// Spawns the action serialization actor on the current Tokio runtime.
    ///
    /// The returned join handle yields the actor's final held-state snapshot
    /// after shutdown release handling.
    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "spawn"))]
    pub fn spawn(
        cancel: CancellationToken,
    ) -> (
        ActionHandle,
        ActionEmitterSnapshotHandle,
        JoinHandle<ActionStateSnapshot>,
    ) {
        let (handle, snapshot_handle, emitter) = Self::channel();
        let join = tokio::spawn(emitter.run(cancel));
        (handle, snapshot_handle, join)
    }

    /// Spawns an actor wired with a single substitute backend across all
    /// resolved kinds. Intended for cross-platform tests that want to observe
    /// actor dispatch without driving real input devices.
    #[must_use]
    pub fn spawn_with_backend(
        cancel: CancellationToken,
        backend: Arc<dyn ActionBackend>,
    ) -> (
        ActionHandle,
        ActionEmitterSnapshotHandle,
        JoinHandle<ActionStateSnapshot>,
    ) {
        let (handle, snapshot_handle, emitter) = Self::channel_with_backend(backend);
        let join = tokio::spawn(emitter.run(cancel));
        (handle, snapshot_handle, join)
    }

    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "pending_len"))]
    pub fn pending_len(&self) -> usize {
        self.rx.len()
    }

    #[tracing::instrument(skip_all, fields(action_kind = "run"))]
    pub async fn run(self, cancel: CancellationToken) -> ActionStateSnapshot {
        self.run_with_connection_closed_cancel(cancel, None).await
    }

    #[tracing::instrument(skip_all, fields(action_kind = "run"))]
    pub async fn run_with_connection_closed_cancel(
        self,
        shutdown_cancel: CancellationToken,
        connection_closed_cancel: Option<CancellationToken>,
    ) -> ActionStateSnapshot {
        self.run_with_shutdown_reason(shutdown_cancel, "shutdown", connection_closed_cancel)
            .await
    }

    #[tracing::instrument(skip_all, fields(action_kind = "run"))]
    pub async fn run_with_shutdown_reason(
        mut self,
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: Option<CancellationToken>,
    ) -> ActionStateSnapshot {
        loop {
            tokio::select! {
                Some((action, ack)) = self.rx.recv() => {
                    let result = self.execute(action).await;
                    let _send_result = ack.send(result);
                },
                Some(snapshot_ack) = self.snapshot_rx.recv() => {
                    let _send_result = snapshot_ack.send(self.snapshot());
                },
                Some(auto_release) = self.auto_release_rx.recv() => {
                    if let Some(emitted_action) = self.auto_release_held_key(&auto_release) {
                        let _release_result = self.execute(emitted_action).await;
                    }
                },
                () = shutdown_cancel.cancelled() => {
                    self.release_all(shutdown_reason).await;
                    return self.snapshot();
                },
                () = connection_closed_cancelled(connection_closed_cancel.as_ref()), if connection_closed_cancel.is_some() => {
                    self.release_all("connection_closed").await;
                    return self.snapshot();
                },
                else => {
                    self.release_all("connection_closed").await;
                    return self.snapshot();
                }
            }
        }
    }

    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "snapshot_state"))]
    pub fn snapshot(&self) -> ActionStateSnapshot {
        let mut snapshot = self.state.snapshot();
        snapshot.held_key_timer_keys = self.held_key_timer_keys();
        snapshot.held_key_timer_count = self.held_key_timers.len();
        snapshot
    }
}
impl Drop for ActionEmitter {
    fn drop(&mut self) {
        self.abort_all_held_key_timers();
    }
}

async fn connection_closed_cancelled(cancel: Option<&CancellationToken>) {
    if let Some(cancel) = cancel {
        cancel.cancelled().await;
    } else {
        std::future::pending::<()>().await;
    }
}
