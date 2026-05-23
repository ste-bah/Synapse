use std::{collections::HashMap, sync::Arc};

use bit_set::BitSet;
use synapse_core::{
    Action, Backend, ButtonAction, ComboInput, GamepadReport, Key, KeyCode, MouseButton, PadId,
    error_codes,
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{self, Duration, Instant},
};
use tokio_util::sync::CancellationToken;

use crate::{
    ACTION_QUEUE_CAPACITY, ActionBackend, ActionError, ActionHandle, ActionMessage, ActionResult,
    HardwareUnavailableBackend, ResolvedBackend, TokenBucket, VigemStateOnlyBackend,
    backend::software::SoftwareBackend, rate_limit::retry_after_ms_for_snapshot, resolve_backend,
};

#[cfg(test)]
use crate::TokenBucketSnapshot;

pub type ActionSnapshotMessage = oneshot::Sender<ActionStateSnapshot>;

pub const HELD_KEY_MAX_DURATION_MS: u64 = 30_000;

#[derive(Clone, Debug, PartialEq)]
pub struct ActionStateSnapshot {
    pub held_keys: Vec<Key>,
    pub held_key_bits: Vec<usize>,
    pub held_key_timer_keys: Vec<Key>,
    pub held_key_timer_count: usize,
    pub held_buttons: Vec<MouseButton>,
    pub held_button_bits: Vec<usize>,
    pub pad_state: HashMap<PadId, GamepadReport>,
}

#[derive(Debug)]
pub struct EmitState {
    pub(crate) held_keys: BitSet,
    pub(crate) held_buttons: BitSet,
    pub(crate) key_indices: HashMap<Key, usize>,
    pub(crate) keys_by_index: Vec<Key>,
    pub(crate) pad_state: HashMap<PadId, GamepadReport>,
}

impl EmitState {
    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "emit_state_new"))]
    pub fn new() -> Self {
        Self {
            held_keys: BitSet::new(),
            held_buttons: BitSet::new(),
            key_indices: HashMap::new(),
            keys_by_index: Vec::new(),
            pad_state: HashMap::new(),
        }
    }

    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "emit_state_snapshot"))]
    pub fn snapshot(&self) -> ActionStateSnapshot {
        ActionStateSnapshot {
            held_keys: self.held_keys(),
            held_key_bits: self.held_keys.iter().collect(),
            held_key_timer_keys: Vec::new(),
            held_key_timer_count: 0,
            held_buttons: self.held_buttons(),
            held_button_bits: self.held_buttons.iter().collect(),
            pad_state: self.pad_state.clone(),
        }
    }

    fn held_keys(&self) -> Vec<Key> {
        self.held_keys
            .iter()
            .filter_map(|index| self.keys_by_index.get(index).cloned())
            .collect()
    }

    fn held_buttons(&self) -> Vec<MouseButton> {
        self.held_buttons
            .iter()
            .filter_map(mouse_button_from_index)
            .collect()
    }

    pub(crate) fn release_all(&mut self) -> (usize, usize, usize) {
        let released_keys = self.held_keys.count();
        let released_buttons = self.held_buttons.count();
        let released_pads = self.pad_state.len();
        self.held_keys.make_empty();
        self.held_buttons.make_empty();
        self.pad_state.clear();
        (released_keys, released_buttons, released_pads)
    }

    pub(crate) fn hold_key(&mut self, key: &Key) {
        let index = self.key_index(key);
        self.held_keys.insert(index);
    }

    pub(crate) fn release_key(&mut self, key: &Key) {
        if let Some(index) = self.key_indices.get(key) {
            self.held_keys.remove(*index);
        }
    }

    pub(crate) fn is_key_held(&self, key: &Key) -> bool {
        self.key_indices
            .get(key)
            .is_some_and(|index| self.held_keys.contains(*index))
    }

    pub(crate) fn apply_mouse_button(&mut self, button: MouseButton, action: ButtonAction) {
        let index = mouse_button_index(button);
        match action {
            ButtonAction::Down => {
                self.held_buttons.insert(index);
            }
            ButtonAction::Up | ButtonAction::Press => {
                self.held_buttons.remove(index);
            }
        }
    }

    fn key_index(&mut self, key: &Key) -> usize {
        if let Some(index) = self.key_indices.get(key) {
            return *index;
        }
        let index = self.keys_by_index.len();
        self.keys_by_index.push(key.clone());
        self.key_indices.insert(key.clone(), index);
        index
    }
}

impl Default for EmitState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct ActionEmitterSnapshotHandle {
    tx: mpsc::Sender<ActionSnapshotMessage>,
}

impl ActionEmitterSnapshotHandle {
    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "snapshot_handle_new"))]
    pub fn new(tx: mpsc::Sender<ActionSnapshotMessage>) -> Self {
        Self { tx }
    }

    /// Reads the emitter's held-state snapshot through the actor task.
    ///
    /// # Errors
    ///
    /// Returns `ACTION_BACKEND_UNAVAILABLE` when the snapshot request or
    /// response channel is closed.
    #[tracing::instrument(skip_all, fields(action_kind = "snapshot"))]
    pub async fn snapshot(&self) -> ActionResult<ActionStateSnapshot> {
        let (snapshot_tx, snapshot_rx) = oneshot::channel();
        self.tx
            .send(snapshot_tx)
            .await
            .map_err(|_err| crate::ActionError::BackendUnavailable {
                detail: "action emitter snapshot channel is closed".to_owned(),
            })?;
        snapshot_rx
            .await
            .map_err(|_err| crate::ActionError::BackendUnavailable {
                detail: "action emitter dropped snapshot response".to_owned(),
            })
    }
}

/// Snapshot of the three production backends the actor dispatches through.
///
/// Resolved per-action via [`resolve_backend`]. The actor itself stays the
/// single serialization point — backends never see concurrent calls.
#[derive(Clone)]
pub struct Backends {
    software: Arc<dyn ActionBackend>,
    vigem: Arc<dyn ActionBackend>,
    hardware: Arc<dyn ActionBackend>,
}

impl Backends {
    #[must_use]
    pub fn production() -> Self {
        Self {
            software: Arc::new(SoftwareBackend::new()),
            vigem: Arc::new(VigemStateOnlyBackend::new()),
            hardware: Arc::new(HardwareUnavailableBackend::new()),
        }
    }

    #[must_use]
    pub fn all_routed_to(backend: Arc<dyn ActionBackend>) -> Self {
        Self {
            software: Arc::clone(&backend),
            vigem: Arc::clone(&backend),
            hardware: backend,
        }
    }

    fn pick(&self, resolved: ResolvedBackend) -> Arc<dyn ActionBackend> {
        match resolved {
            ResolvedBackend::Software => Arc::clone(&self.software),
            ResolvedBackend::Vigem => Arc::clone(&self.vigem),
            ResolvedBackend::Hardware => Arc::clone(&self.hardware),
        }
    }
}

impl std::fmt::Debug for Backends {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backends").finish_non_exhaustive()
    }
}

pub struct ActionEmitter {
    rx: mpsc::Receiver<ActionMessage>,
    snapshot_rx: mpsc::Receiver<ActionSnapshotMessage>,
    auto_release_tx: mpsc::Sender<HeldKeyAutoRelease>,
    auto_release_rx: mpsc::Receiver<HeldKeyAutoRelease>,
    state: EmitState,
    backends: Backends,
    rate_limits: BackendRateLimits,
    held_key_timers: HashMap<Key, JoinHandle<()>>,
    held_key_timer_ids: HashMap<Key, u64>,
    next_held_key_timer_id: u64,
}

#[derive(Debug)]
struct HeldKeyAutoRelease {
    key: Key,
    timer_id: u64,
}

#[cfg(test)]
#[derive(Debug)]
struct BackendRateLimitSnapshot {
    software: TokenBucketSnapshot,
    vigem: TokenBucketSnapshot,
    hardware: TokenBucketSnapshot,
}

struct BackendRateLimits {
    software: TokenBucket,
    vigem: TokenBucket,
    hardware: TokenBucket,
}

impl BackendRateLimits {
    fn new() -> Self {
        Self {
            software: TokenBucket::for_backend(ResolvedBackend::Software),
            vigem: TokenBucket::for_backend(ResolvedBackend::Vigem),
            hardware: TokenBucket::for_backend(ResolvedBackend::Hardware),
        }
    }

    #[cfg(test)]
    const fn with_buckets(
        software: TokenBucket,
        vigem: TokenBucket,
        hardware: TokenBucket,
    ) -> Self {
        Self {
            software,
            vigem,
            hardware,
        }
    }

    const fn bucket(&self, backend: ResolvedBackend) -> &TokenBucket {
        match backend {
            ResolvedBackend::Software => &self.software,
            ResolvedBackend::Vigem => &self.vigem,
            ResolvedBackend::Hardware => &self.hardware,
        }
    }

    #[cfg(test)]
    fn snapshot(&self) -> BackendRateLimitSnapshot {
        BackendRateLimitSnapshot {
            software: self.software.snapshot(),
            vigem: self.vigem.snapshot(),
            hardware: self.hardware.snapshot(),
        }
    }
}

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
        let (auto_release_tx, auto_release_rx) = mpsc::channel(ACTION_QUEUE_CAPACITY);
        Self {
            rx,
            snapshot_rx,
            auto_release_tx,
            auto_release_rx,
            state: EmitState::new(),
            backends,
            rate_limits: BackendRateLimits::new(),
            held_key_timers: HashMap::new(),
            held_key_timer_ids: HashMap::new(),
            next_held_key_timer_id: 0,
        }
    }

    #[cfg(test)]
    fn with_rate_limits(
        rx: mpsc::Receiver<ActionMessage>,
        snapshot_rx: mpsc::Receiver<ActionSnapshotMessage>,
        backends: Backends,
        rate_limits: BackendRateLimits,
    ) -> Self {
        let (auto_release_tx, auto_release_rx) = mpsc::channel(ACTION_QUEUE_CAPACITY);
        Self {
            rx,
            snapshot_rx,
            auto_release_tx,
            auto_release_rx,
            state: EmitState::new(),
            backends,
            rate_limits,
            held_key_timers: HashMap::new(),
            held_key_timer_ids: HashMap::new(),
            next_held_key_timer_id: 0,
        }
    }

    #[cfg(test)]
    fn channel_with_rate_limits(
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
        let (handle, rx) = ActionHandle::channel();
        let (snapshot_tx, snapshot_rx) = mpsc::channel(ACTION_QUEUE_CAPACITY);
        (
            handle,
            ActionEmitterSnapshotHandle::new(snapshot_tx),
            Self::new_with_backends(rx, snapshot_rx, backends),
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
                    let _emitted_action = self.auto_release_held_key(&auto_release);
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

    #[tracing::instrument(skip_all, fields(action_kind = %action_kind(&action)))]
    async fn execute(&mut self, action: Action) -> ActionResult<()> {
        crate::validate_action(&action)?;

        if matches!(action, Action::ReleaseAll) {
            return self.do_release_all("tool_invocation").await;
        }

        if action_consumes_rate_limit(&action) {
            let backend = resolved_backend_for_action(&action)?;
            self.consume_rate_limit(backend)?;
        }

        self.cancel_timers_for_release_actions(&action);

        let resolved = resolved_backend_for_action(&action)?;
        let backend = self.backends.pick(resolved);
        let result = self.dispatch_via_backend(backend, action.clone()).await;

        if result.is_ok() {
            self.schedule_timers_for_held_keys(&action);
        }

        result
    }

    async fn dispatch_via_backend(
        &mut self,
        backend: Arc<dyn ActionBackend>,
        action: Action,
    ) -> ActionResult<()> {
        let mut state = std::mem::take(&mut self.state);
        let task = tokio::task::spawn_blocking(move || {
            let result = backend.execute(&action, &mut state);
            (result, state)
        });
        match task.await {
            Ok((result, state)) => {
                self.state = state;
                result
            }
            Err(join_error) => {
                // The blocking task panicked or was cancelled; we lost the
                // moved EmitState. Fail-closed: surface the cause and reset
                // held-state to empty so the next action starts clean.
                self.state = EmitState::new();
                Err(ActionError::BackendUnavailable {
                    detail: format!(
                        "code={} backend.execute spawn_blocking join failed: {join_error}",
                        error_codes::ACTION_BACKEND_UNAVAILABLE
                    ),
                })
            }
        }
    }

    /// Cancels in-flight auto-release timers for any key the action is about
    /// to release. Done before backend dispatch so a timer cannot fire mid-call
    /// and enqueue a duplicate `KeyUp` against a key the backend has already
    /// released.
    fn cancel_timers_for_release_actions(&mut self, action: &Action) {
        match action {
            Action::KeyPress { key, .. } | Action::KeyUp { key, .. } => {
                self.cancel_held_key_timer(key);
            }
            Action::KeyChord { keys, .. } => {
                for key in keys {
                    self.cancel_held_key_timer(key);
                }
            }
            Action::Combo { steps, .. } => {
                for step in steps {
                    if let ComboInput::KeyUp { key } | ComboInput::KeyPress { key, .. } =
                        &step.input
                    {
                        self.cancel_held_key_timer(key);
                    }
                }
            }
            _ => {}
        }
    }

    /// Schedules safety auto-release timers for any key the action just put
    /// into a held-down state. Called only when the backend dispatch
    /// succeeded — if the backend errored the key is never in `held_keys`
    /// and there is nothing to time out.
    fn schedule_timers_for_held_keys(&mut self, action: &Action) {
        match action {
            Action::KeyDown { key, .. } => {
                self.schedule_held_key_auto_release(key.clone());
            }
            Action::Combo { steps, .. } => {
                for step in steps {
                    if let ComboInput::KeyDown { key } = &step.input {
                        self.schedule_held_key_auto_release(key.clone());
                    }
                }
            }
            _ => {}
        }
    }

    fn consume_rate_limit(&self, backend: ResolvedBackend) -> ActionResult<()> {
        let bucket = self.rate_limits.bucket(backend);
        if bucket.try_consume(1) {
            return Ok(());
        }

        let snapshot = bucket.snapshot();
        let retry_after_ms = retry_after_ms_for_snapshot(snapshot, 1);
        Err(ActionError::RateLimited {
            detail: format!(
                "backend={} retry_after_ms={} requested_tokens=1 available_tokens={} refill_rate_per_s={}",
                backend.as_str(),
                retry_after_ms,
                snapshot.tokens,
                snapshot.refill_rate_per_s
            ),
            retry_after_ms,
        })
    }

    #[tracing::instrument(skip_all, fields(action_kind = "release_all"))]
    async fn release_all(&mut self, reason: &'static str) {
        let _release_result = self.do_release_all(reason).await;
    }

    /// Drives a `ReleaseAll` through the resolved backend (so the production
    /// path actually emits SendInput KeyUps for every held key/button), aborts
    /// in-flight auto-release timers, and logs the drained snapshot. The
    /// blocking backend work runs on `spawn_blocking` so the runtime stays
    /// responsive.
    async fn do_release_all(&mut self, reason: &'static str) -> ActionResult<()> {
        let before = self.snapshot();
        let mut held_pad_ids: Vec<_> = before.pad_state.keys().copied().collect();
        held_pad_ids.sort_unstable();
        let released_keys = before.held_keys.len();
        let released_buttons = before.held_buttons.len();
        let released_pads = before.pad_state.len();

        let cancelled_key_timers = self.abort_all_held_key_timers();

        let resolved = resolved_backend_for_action(&Action::ReleaseAll)?;
        let backend = self.backends.pick(resolved);
        let result = self.dispatch_via_backend(backend, Action::ReleaseAll).await;

        tracing::warn!(
            code = error_codes::SAFETY_RELEASE_ALL_FIRED,
            reason,
            held_keys = ?before.held_keys,
            held_key_bits = ?before.held_key_bits,
            held_key_timer_keys = ?before.held_key_timer_keys,
            held_key_timer_count = before.held_key_timer_count,
            held_buttons = ?before.held_buttons,
            held_button_bits = ?before.held_button_bits,
            held_pad_ids = ?held_pad_ids,
            released_keys,
            released_buttons,
            released_pads,
            cancelled_key_timers,
            backend_ok = result.is_ok(),
            "release_all drained action emitter held state"
        );

        // If the backend failed mid-release, clear the held bitmaps anyway —
        // the actor's state must reflect what the safety path attempted, so
        // the next ReleaseAll snapshot does not loop on the same keys.
        if result.is_err() {
            self.state.release_all();
        }
        result
    }

    fn schedule_held_key_auto_release(&mut self, key: Key) {
        self.cancel_held_key_timer(&key);

        let timer_id = self.next_held_key_timer_id;
        self.next_held_key_timer_id = self.next_held_key_timer_id.wrapping_add(1);
        let deadline = Instant::now() + Duration::from_millis(HELD_KEY_MAX_DURATION_MS);
        let tx = self.auto_release_tx.clone();
        let timer_key = key.clone();
        let handle = tokio::spawn(async move {
            time::sleep_until(deadline).await;
            let _send_result = tx
                .send(HeldKeyAutoRelease {
                    key: timer_key,
                    timer_id,
                })
                .await;
        });

        self.held_key_timer_ids.insert(key.clone(), timer_id);
        self.held_key_timers.insert(key, handle);
    }

    fn cancel_held_key_timer(&mut self, key: &Key) -> bool {
        self.held_key_timer_ids.remove(key);
        self.held_key_timers.remove(key).is_some_and(|handle| {
            handle.abort();
            true
        })
    }

    fn abort_all_held_key_timers(&mut self) -> usize {
        let cancelled = self.held_key_timers.len();
        for (_key, handle) in self.held_key_timers.drain() {
            handle.abort();
        }
        self.held_key_timer_ids.clear();
        cancelled
    }

    fn auto_release_held_key(&mut self, auto_release: &HeldKeyAutoRelease) -> Option<Action> {
        if self
            .held_key_timer_ids
            .get(&auto_release.key)
            .is_none_or(|timer_id| *timer_id != auto_release.timer_id)
        {
            return None;
        }

        self.held_key_timer_ids.remove(&auto_release.key);
        self.held_key_timers.remove(&auto_release.key);
        if !self.state.is_key_held(&auto_release.key) {
            return None;
        }

        self.state.release_key(&auto_release.key);
        tracing::warn!(
            code = %error_codes::STUCK_KEY_AUTO_RELEASED,
            held_ms = HELD_KEY_MAX_DURATION_MS,
            key = %key_log_label(&auto_release.key),
            key_debug = ?auto_release.key,
            "stuck key auto-released"
        );
        Some(Action::KeyUp {
            key: auto_release.key.clone(),
            backend: Backend::Auto,
        })
    }

    fn held_key_timer_keys(&self) -> Vec<Key> {
        let mut keys: Vec<_> = self.held_key_timers.keys().cloned().collect();
        keys.sort_by_key(|key| format!("{key:?}"));
        keys
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

fn resolved_backend_for_action(action: &Action) -> ActionResult<ResolvedBackend> {
    resolve_backend(requested_backend(action), action)
}

const fn action_consumes_rate_limit(action: &Action) -> bool {
    !matches!(action, Action::ReleaseAll | Action::KeyUp { .. })
}

const fn requested_backend(action: &Action) -> Backend {
    match action {
        Action::KeyPress { backend, .. }
        | Action::KeyDown { backend, .. }
        | Action::KeyUp { backend, .. }
        | Action::KeyChord { backend, .. }
        | Action::TypeText { backend, .. }
        | Action::MouseMove { backend, .. }
        | Action::MouseMoveRelative { backend, .. }
        | Action::MouseButton { backend, .. }
        | Action::MouseDrag { backend, .. }
        | Action::MouseScroll { backend, .. }
        | Action::AimAt { backend, .. }
        | Action::Combo { backend, .. } => *backend,
        Action::PadButton { .. }
        | Action::PadStick { .. }
        | Action::PadTrigger { .. }
        | Action::PadReport { .. }
        | Action::ReleaseAll => Backend::Auto,
    }
}

const fn action_kind(action: &Action) -> &'static str {
    match action {
        Action::KeyPress { .. } => "key_press",
        Action::KeyDown { .. } => "key_down",
        Action::KeyUp { .. } => "key_up",
        Action::KeyChord { .. } => "key_chord",
        Action::TypeText { .. } => "type_text",
        Action::MouseMove { .. } => "mouse_move",
        Action::MouseMoveRelative { .. } => "mouse_move_relative",
        Action::MouseButton { .. } => "mouse_button",
        Action::MouseDrag { .. } => "mouse_drag",
        Action::MouseScroll { .. } => "mouse_scroll",
        Action::PadButton { .. } => "pad_button",
        Action::PadStick { .. } => "pad_stick",
        Action::PadTrigger { .. } => "pad_trigger",
        Action::PadReport { .. } => "pad_report",
        Action::AimAt { .. } => "aim_at",
        Action::Combo { .. } => "combo",
        Action::ReleaseAll => "release_all",
    }
}

fn key_log_label(key: &Key) -> String {
    match &key.code {
        KeyCode::Named { value } => value.clone(),
        KeyCode::Symbol { value } => value.to_string(),
        KeyCode::HidCode { value } => format!("hid:{value}"),
    }
}

const fn mouse_button_index(button: MouseButton) -> usize {
    match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::X1 => 3,
        MouseButton::X2 => 4,
    }
}

const fn mouse_button_from_index(index: usize) -> Option<MouseButton> {
    match index {
        0 => Some(MouseButton::Left),
        1 => Some(MouseButton::Right),
        2 => Some(MouseButton::Middle),
        3 => Some(MouseButton::X1),
        4 => Some(MouseButton::X2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };

    use crate::{ActionBackend, RecordedInput, RecordingBackend};
    use synapse_core::{KeyCode, PadButton};
    use tracing_subscriber::fmt::writer::MakeWriter;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn rate_limited_error_carries_code_and_retry_after_ms_without_state_mutation() {
        let (_handle, _snapshot_handle, mut emitter) =
            ActionEmitter::channel_with_rate_limits(one_token_limits());
        let first_key = key_named("first");
        let second_key = key_named("second");
        let before_state = emitter.snapshot();
        let before_limits = emitter.rate_limits.snapshot();

        let first_result = emitter
            .execute(Action::KeyDown {
                key: first_key.clone(),
                backend: Backend::Software,
            })
            .await;
        assert!(
            first_result.is_ok(),
            "first token should be available: {first_result:?}"
        );
        let after_first_state = emitter.snapshot();
        let after_first_limits = emitter.rate_limits.snapshot();
        let after = emitter
            .execute(Action::KeyDown {
                key: second_key,
                backend: Backend::Software,
            })
            .await;
        let after_limited_state = emitter.snapshot();
        let after_limited_limits = emitter.rate_limits.snapshot();

        let Err(error) = after else {
            panic!("second software action should be rate limited");
        };
        assert_eq!(error.code(), error_codes::ACTION_RATE_LIMITED);
        assert_eq!(error.retry_after_ms(), Some(1));
        assert!(error.detail().contains("retry_after_ms=1"));
        assert_eq!(before_limits.hardware.tokens, 1);
        assert_eq!(after_first_state.held_keys, vec![first_key.clone()]);
        assert_eq!(after_limited_state.held_keys, vec![first_key]);
        assert_eq!(after_first_limits.software.tokens, 0);
        assert_eq!(after_limited_limits.software.tokens, 0);
        println!(
            "source_of_truth=action_emitter_rate_limit edge=software_over_cap before_state={before_state:?} before_limits={before_limits:?} after_first_state={after_first_state:?} after_first_limits={after_first_limits:?} after_limited_state={after_limited_state:?} after_limited_limits={after_limited_limits:?} data.code={} data.retry_after_ms={:?} detail={}",
            error.code(),
            error.retry_after_ms(),
            error.detail()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn software_rate_limit_does_not_consume_vigem_bucket() {
        let (_handle, _snapshot_handle, mut emitter) =
            ActionEmitter::channel_with_rate_limits(one_token_limits());
        let before = emitter.rate_limits.snapshot();

        let software_result = emitter
            .execute(Action::KeyPress {
                key: key_named("software"),
                hold_ms: 0,
                backend: Backend::Software,
            })
            .await;
        assert!(
            software_result.is_ok(),
            "software token should be available: {software_result:?}"
        );
        let after_software = emitter.rate_limits.snapshot();
        let report = gamepad_report(PadButton::A);
        let vigem_result = emitter
            .execute(Action::PadReport {
                pad: 1,
                report: report.clone(),
            })
            .await;
        assert!(
            vigem_result.is_ok(),
            "vigem token should be independent from software: {vigem_result:?}"
        );
        let after_vigem = emitter.rate_limits.snapshot();
        let after_vigem_state = emitter.snapshot();
        let after = emitter
            .execute(Action::PadReport {
                pad: 1,
                report: gamepad_report(PadButton::B),
            })
            .await;
        let after_limited_state = emitter.snapshot();

        let Err(error) = after else {
            panic!("second vigem action should be rate limited");
        };
        assert_eq!(error.code(), error_codes::ACTION_RATE_LIMITED);
        assert_eq!(error.retry_after_ms(), Some(1));
        assert_eq!(after_software.software.tokens, 0);
        assert_eq!(after_software.vigem.tokens, 1);
        assert_eq!(after_software.hardware.tokens, 1);
        assert_eq!(after_vigem.vigem.tokens, 0);
        assert_eq!(after_vigem_state.pad_state.get(&1), Some(&report));
        assert_eq!(after_limited_state.pad_state.get(&1), Some(&report));
        println!(
            "source_of_truth=action_emitter_rate_limit edge=backend_separation before={before:?} after_software={after_software:?} after_vigem={after_vigem:?} after_vigem_state={after_vigem_state:?} after_limited_state={after_limited_state:?} data.code={} data.retry_after_ms={:?}",
            error.code(),
            error.retry_after_ms()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn release_all_bypasses_empty_buckets_and_drains_state() {
        let (_handle, _snapshot_handle, mut emitter) =
            ActionEmitter::channel_with_rate_limits(empty_limits());
        let key = key_named("stuck");
        emitter.state.hold_key(&key);
        let before_state = emitter.snapshot();
        let before_limits = emitter.rate_limits.snapshot();

        let release_result = emitter.execute(Action::ReleaseAll).await;
        assert!(
            release_result.is_ok(),
            "ReleaseAll must not be rate limited: {release_result:?}"
        );
        let after_state = emitter.snapshot();
        let after_limits = emitter.rate_limits.snapshot();

        assert_eq!(before_state.held_keys, vec![key]);
        assert!(after_state.held_keys.is_empty());
        assert_eq!(before_limits.software.tokens, 0);
        assert_eq!(after_limits.software.tokens, 0);
        println!(
            "source_of_truth=action_emitter_rate_limit edge=release_all_bypass before_state={before_state:?} before_limits={before_limits:?} after_state={after_state:?} after_limits={after_limits:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn key_down_timer_auto_releases_held_key_and_clears_hashmap() {
        let (_handle, _snapshot_handle, mut emitter) =
            ActionEmitter::channel_with_rate_limits(generous_limits());
        let key = key_named("auto-release-happy");
        let before = emitter.snapshot();

        let key_down_result = emitter
            .execute(Action::KeyDown {
                key: key.clone(),
                backend: Backend::Software,
            })
            .await;
        assert!(
            key_down_result.is_ok(),
            "KeyDown should consume available token: {key_down_result:?}"
        );
        let after_key_down = emitter.snapshot();

        tokio::task::yield_now().await;
        time::advance(Duration::from_millis(HELD_KEY_MAX_DURATION_MS)).await;
        tokio::task::yield_now().await;
        let auto_release = read_pending_auto_release(&mut emitter);
        let emitted_action = emitter.auto_release_held_key(&auto_release);
        let after_auto_release = emitter.snapshot();

        assert!(before.held_keys.is_empty());
        assert_eq!(after_key_down.held_keys, vec![key.clone()]);
        assert_eq!(after_key_down.held_key_timer_keys, vec![key.clone()]);
        assert_eq!(after_key_down.held_key_timer_count, 1);
        assert_auto_key_up(emitted_action.as_ref(), &key);
        assert!(after_auto_release.held_keys.is_empty());
        assert!(after_auto_release.held_key_timer_keys.is_empty());
        assert_eq!(after_auto_release.held_key_timer_count, 0);
        println!(
            "source_of_truth=held_keys_bitset_and_timer_hashmap edge=happy_auto_release before={before:?} after_key_down={after_key_down:?} after_auto_release={after_auto_release:?} data.code={}",
            error_codes::STUCK_KEY_AUTO_RELEASED
        );
    }

    #[tokio::test(start_paused = true, flavor = "current_thread")]
    async fn stuck_key_auto_release_tracing_event_and_recording_keyup_are_observable() {
        let trace_buffer = SharedTraceBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(trace_buffer.clone())
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_level(false)
            .finish();

        let (_handle, _snapshot_handle, mut emitter) =
            ActionEmitter::channel_with_rate_limits(generous_limits());
        let recording_backend = RecordingBackend::new();
        let mut recording_state = EmitState::new();
        let key = key_named("a");
        let key_down = Action::KeyDown {
            key: key.clone(),
            backend: Backend::Software,
        };
        recording_backend
            .execute(&key_down, &mut recording_state)
            .unwrap_or_else(|error| panic!("recording keydown should succeed: {error}"));
        let before_empty = emitter.snapshot();

        let key_down_result = emitter.execute(key_down).await;
        assert!(
            key_down_result.is_ok(),
            "KeyDown should set held state for trace test: {key_down_result:?}"
        );
        let before_auto_release = emitter.snapshot();

        tokio::task::yield_now().await;
        time::advance(Duration::from_millis(HELD_KEY_MAX_DURATION_MS)).await;
        tokio::task::yield_now().await;
        let auto_release = read_pending_auto_release(&mut emitter);
        let emitted_action = tracing::subscriber::with_default(subscriber, || {
            emitter
                .auto_release_held_key(&auto_release)
                .unwrap_or_else(|| panic!("auto-release should emit KeyUp action"))
        });
        recording_backend
            .execute(&emitted_action, &mut recording_state)
            .unwrap_or_else(|error| panic!("recording auto KeyUp should succeed: {error}"));
        let after_auto_release = emitter.snapshot();

        let log_output = trace_buffer.text();
        let log_line = find_log_line(&log_output, error_codes::STUCK_KEY_AUTO_RELEASED);
        let recording_events = recording_backend.events();
        let expected_events = vec![
            RecordedInput::KeyDown { key: key.clone() },
            RecordedInput::KeyUp { key: key.clone() },
        ];

        assert!(before_empty.held_keys.is_empty());
        assert_eq!(before_auto_release.held_keys, vec![key.clone()]);
        assert_eq!(before_auto_release.held_key_timer_count, 1);
        assert!(after_auto_release.held_keys.is_empty());
        assert_eq!(after_auto_release.held_key_timer_count, 0);
        assert_eq!(recording_events, expected_events);
        assert_auto_key_up(Some(&emitted_action), &key);
        assert!(log_line.contains("code=STUCK_KEY_AUTO_RELEASED"));
        assert!(log_line.contains("held_ms=30000"));
        assert!(log_line.contains("key=a"));
        println!(
            "source_of_truth=stuck_key edge=auto_release before=held:{:?} after=held:{:?} log_line={} recording_events={recording_events:?}",
            held_key_labels(&before_auto_release),
            held_key_labels(&after_auto_release),
            log_line
        );
    }

    #[tokio::test(start_paused = true)]
    async fn actor_loop_processes_auto_release_timer_message() {
        let (handle, snapshot_handle, emitter) =
            ActionEmitter::channel_with_rate_limits(generous_limits());
        let cancel = CancellationToken::new();
        let join = tokio::spawn(emitter.run(cancel.clone()));
        let key = key_named("actor-auto-release");
        let before = snapshot_or_panic(&snapshot_handle).await;

        let key_down_result = handle
            .execute(Action::KeyDown {
                key: key.clone(),
                backend: Backend::Software,
            })
            .await;
        assert!(
            key_down_result.is_ok(),
            "actor KeyDown should be accepted: {key_down_result:?}"
        );
        let after_key_down = snapshot_or_panic(&snapshot_handle).await;

        tokio::task::yield_now().await;
        time::advance(Duration::from_millis(HELD_KEY_MAX_DURATION_MS)).await;
        tokio::task::yield_now().await;
        let after_auto_release = snapshot_until_empty(&snapshot_handle).await;

        cancel.cancel();
        let after_cancel = join_actor_or_panic(join).await;

        assert!(before.held_keys.is_empty());
        assert_eq!(after_key_down.held_keys, vec![key]);
        assert_eq!(after_key_down.held_key_timer_count, 1);
        assert!(after_auto_release.held_keys.is_empty());
        assert_eq!(after_auto_release.held_key_timer_count, 0);
        assert!(after_cancel.held_keys.is_empty());
        assert_eq!(after_cancel.held_key_timer_count, 0);
        println!(
            "source_of_truth=actor_snapshot_held_keys_bitset_and_timer_hashmap edge=actor_loop_auto_release before={before:?} after_key_down={after_key_down:?} after_auto_release={after_auto_release:?} after_cancel={after_cancel:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn key_up_cancels_timer_before_releasing_even_when_buckets_are_empty() {
        let (_handle, _snapshot_handle, mut emitter) =
            ActionEmitter::channel_with_rate_limits(empty_limits());
        let key = key_named("manual-key-up-cancel");
        emitter.state.hold_key(&key);
        emitter.schedule_held_key_auto_release(key.clone());
        let before = emitter.snapshot();
        let before_limits = emitter.rate_limits.snapshot();

        let key_up_result = emitter
            .execute(Action::KeyUp {
                key: key.clone(),
                backend: Backend::Software,
            })
            .await;
        assert!(
            key_up_result.is_ok(),
            "KeyUp must bypass rate limits to cancel the safety timer: {key_up_result:?}"
        );
        let after = emitter.snapshot();
        let after_limits = emitter.rate_limits.snapshot();

        time::advance(Duration::from_millis(HELD_KEY_MAX_DURATION_MS + 1)).await;
        tokio::task::yield_now().await;
        assert_no_pending_auto_release(&mut emitter);

        assert_eq!(before.held_keys, vec![key]);
        assert_eq!(before.held_key_timer_count, 1);
        assert!(after.held_keys.is_empty());
        assert_eq!(after.held_key_timer_count, 0);
        assert_eq!(before_limits.software.tokens, 0);
        assert_eq!(after_limits.software.tokens, 0);
        println!(
            "source_of_truth=held_keys_bitset_and_timer_hashmap edge=keyup_cancel_empty_bucket before={before:?} before_limits={before_limits:?} after={after:?} after_limits={after_limits:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn repeated_key_down_replaces_timer_without_old_timer_release() {
        let (_handle, _snapshot_handle, mut emitter) =
            ActionEmitter::channel_with_rate_limits(generous_limits());
        let key = key_named("repeat-reset");
        let before = emitter.snapshot();

        let first_result = emitter
            .execute(Action::KeyDown {
                key: key.clone(),
                backend: Backend::Software,
            })
            .await;
        assert!(
            first_result.is_ok(),
            "first KeyDown should be accepted: {first_result:?}"
        );
        let after_first = emitter.snapshot();
        let first_timer_id = current_timer_id(&emitter, &key);

        tokio::task::yield_now().await;
        time::advance(Duration::from_millis(HELD_KEY_MAX_DURATION_MS - 1_000)).await;
        let second_result = emitter
            .execute(Action::KeyDown {
                key: key.clone(),
                backend: Backend::Software,
            })
            .await;
        assert!(
            second_result.is_ok(),
            "second KeyDown should reset the timer: {second_result:?}"
        );
        let after_second = emitter.snapshot();
        let second_timer_id = current_timer_id(&emitter, &key);

        time::advance(Duration::from_secs(2)).await;
        tokio::task::yield_now().await;
        let after_old_deadline = emitter.snapshot();
        assert_no_pending_auto_release(&mut emitter);

        time::advance(Duration::from_millis(HELD_KEY_MAX_DURATION_MS - 2_000)).await;
        tokio::task::yield_now().await;
        let auto_release = read_pending_auto_release(&mut emitter);
        let emitted_action = emitter.auto_release_held_key(&auto_release);
        let after_new_deadline = emitter.snapshot();

        assert_ne!(first_timer_id, second_timer_id);
        assert_eq!(after_first.held_key_timer_count, 1);
        assert_eq!(after_second.held_key_timer_count, 1);
        assert_eq!(after_old_deadline.held_keys, vec![key.clone()]);
        assert_eq!(after_old_deadline.held_key_timer_count, 1);
        assert_auto_key_up(emitted_action.as_ref(), &key);
        assert!(after_new_deadline.held_keys.is_empty());
        assert_eq!(after_new_deadline.held_key_timer_count, 0);
        println!(
            "source_of_truth=held_keys_bitset_and_timer_hashmap edge=repeated_keydown_reset before={before:?} after_first={after_first:?} first_timer_id={first_timer_id} after_second={after_second:?} second_timer_id={second_timer_id} after_old_deadline={after_old_deadline:?} after_new_deadline={after_new_deadline:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn release_all_aborts_held_key_timer_hashmap() {
        let (_handle, _snapshot_handle, mut emitter) =
            ActionEmitter::channel_with_rate_limits(generous_limits());
        let key = key_named("release-all-abort");
        let key_down_result = emitter
            .execute(Action::KeyDown {
                key: key.clone(),
                backend: Backend::Software,
            })
            .await;
        assert!(
            key_down_result.is_ok(),
            "KeyDown should set up the timer: {key_down_result:?}"
        );
        let before_release_all = emitter.snapshot();

        let release_result = emitter.execute(Action::ReleaseAll).await;
        assert!(
            release_result.is_ok(),
            "ReleaseAll must abort timers without rate limiting: {release_result:?}"
        );
        let after_release_all = emitter.snapshot();

        time::advance(Duration::from_millis(HELD_KEY_MAX_DURATION_MS + 1)).await;
        tokio::task::yield_now().await;
        assert_no_pending_auto_release(&mut emitter);

        assert_eq!(before_release_all.held_keys, vec![key]);
        assert_eq!(before_release_all.held_key_timer_count, 1);
        assert!(after_release_all.held_keys.is_empty());
        assert_eq!(after_release_all.held_key_timer_count, 0);
        println!(
            "source_of_truth=held_keys_bitset_and_timer_hashmap edge=release_all_abort before={before_release_all:?} after={after_release_all:?}"
        );
    }

    fn one_token_limits() -> BackendRateLimits {
        BackendRateLimits::with_buckets(
            TokenBucket::new(1, 5_000),
            TokenBucket::new(1, 1_000),
            TokenBucket::new(1, 5_000),
        )
    }

    fn empty_limits() -> BackendRateLimits {
        BackendRateLimits::with_buckets(
            TokenBucket::new(0, 0),
            TokenBucket::new(0, 0),
            TokenBucket::new(0, 0),
        )
    }

    fn generous_limits() -> BackendRateLimits {
        BackendRateLimits::with_buckets(
            TokenBucket::new(10, 5_000),
            TokenBucket::new(10, 1_000),
            TokenBucket::new(10, 5_000),
        )
    }

    fn read_pending_auto_release(emitter: &mut ActionEmitter) -> HeldKeyAutoRelease {
        match emitter.auto_release_rx.try_recv() {
            Ok(auto_release) => auto_release,
            Err(error) => panic!("expected fired auto-release timer message, got {error:?}"),
        }
    }

    fn assert_no_pending_auto_release(emitter: &mut ActionEmitter) {
        match emitter.auto_release_rx.try_recv() {
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            other => panic!("expected no auto-release timer message, got {other:?}"),
        }
    }

    fn current_timer_id(emitter: &ActionEmitter, key: &Key) -> u64 {
        emitter.held_key_timer_ids.get(key).map_or_else(
            || panic!("expected held key timer id for {key:?}"),
            |timer_id| *timer_id,
        )
    }

    fn assert_auto_key_up(action: Option<&Action>, expected_key: &Key) {
        match action {
            Some(Action::KeyUp { key, backend }) => {
                assert_eq!(key, expected_key);
                assert_eq!(*backend, Backend::Auto);
            }
            other => panic!("expected emitted auto KeyUp for {expected_key:?}, got {other:?}"),
        }
    }

    fn held_key_labels(snapshot: &ActionStateSnapshot) -> Vec<String> {
        snapshot.held_keys.iter().map(key_log_label).collect()
    }

    fn find_log_line(log_output: &str, needle: &str) -> String {
        log_output
            .lines()
            .find(|line| line.contains(needle))
            .map_or_else(
                || panic!("expected log output to contain {needle}, got {log_output:?}"),
                ToOwned::to_owned,
            )
    }

    #[derive(Clone, Default)]
    struct SharedTraceBuffer {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedTraceBuffer {
        fn text(&self) -> String {
            let bytes = match self.bytes.lock() {
                Ok(guard) => guard.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            };
            String::from_utf8_lossy(&bytes).into_owned()
        }
    }

    impl<'a> MakeWriter<'a> for SharedTraceBuffer {
        type Writer = SharedTraceBufferWriter;

        fn make_writer(&'a self) -> Self::Writer {
            SharedTraceBufferWriter {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }

    struct SharedTraceBufferWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for SharedTraceBufferWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            match self.bytes.lock() {
                Ok(mut guard) => guard.extend_from_slice(buf),
                Err(poisoned) => poisoned.into_inner().extend_from_slice(buf),
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    async fn snapshot_until_empty(
        snapshot_handle: &ActionEmitterSnapshotHandle,
    ) -> ActionStateSnapshot {
        let mut last_snapshot = snapshot_or_panic(snapshot_handle).await;
        for _attempt in 0..8 {
            if last_snapshot.held_keys.is_empty() && last_snapshot.held_key_timer_count == 0 {
                return last_snapshot;
            }
            tokio::task::yield_now().await;
            last_snapshot = snapshot_or_panic(snapshot_handle).await;
        }
        panic!("expected actor auto-release to drain held key state, last={last_snapshot:?}");
    }

    async fn snapshot_or_panic(
        snapshot_handle: &ActionEmitterSnapshotHandle,
    ) -> ActionStateSnapshot {
        match snapshot_handle.snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => panic!("snapshot should succeed: {error:?}"),
        }
    }

    async fn join_actor_or_panic(join: JoinHandle<ActionStateSnapshot>) -> ActionStateSnapshot {
        match join.await {
            Ok(snapshot) => snapshot,
            Err(error) => panic!("actor join should succeed: {error:?}"),
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

    fn gamepad_report(button: PadButton) -> GamepadReport {
        GamepadReport {
            buttons: vec![button],
            thumb_l: (0.0, 0.0),
            thumb_r: (0.0, 0.0),
            lt: 0.0,
            rt: 0.0,
        }
    }
}
