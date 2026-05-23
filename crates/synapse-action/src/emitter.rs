use std::collections::HashMap;

use bit_set::BitSet;
use synapse_core::{
    Action, ButtonAction, ComboInput, GamepadReport, Key, MouseButton, PadButton, PadId, Stick,
    Trigger, error_codes,
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{ACTION_QUEUE_CAPACITY, ActionHandle, ActionMessage, ActionResult};

pub type ActionSnapshotMessage = oneshot::Sender<ActionStateSnapshot>;

#[derive(Clone, Debug, PartialEq)]
pub struct ActionStateSnapshot {
    pub held_keys: Vec<Key>,
    pub held_key_bits: Vec<usize>,
    pub held_buttons: Vec<MouseButton>,
    pub held_button_bits: Vec<usize>,
    pub pad_state: HashMap<PadId, GamepadReport>,
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

pub struct ActionEmitter {
    rx: mpsc::Receiver<ActionMessage>,
    snapshot_rx: mpsc::Receiver<ActionSnapshotMessage>,
    held_keys: BitSet,
    held_buttons: BitSet,
    key_indices: HashMap<Key, usize>,
    keys_by_index: Vec<Key>,
    pad_state: HashMap<PadId, GamepadReport>,
}

impl ActionEmitter {
    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "new"))]
    pub fn new(
        rx: mpsc::Receiver<ActionMessage>,
        snapshot_rx: mpsc::Receiver<ActionSnapshotMessage>,
    ) -> Self {
        Self {
            rx,
            snapshot_rx,
            held_keys: BitSet::new(),
            held_buttons: BitSet::new(),
            key_indices: HashMap::new(),
            keys_by_index: Vec::new(),
            pad_state: HashMap::new(),
        }
    }

    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "channel"))]
    pub fn channel() -> (ActionHandle, ActionEmitterSnapshotHandle, Self) {
        let (handle, rx) = ActionHandle::channel();
        let (snapshot_tx, snapshot_rx) = mpsc::channel(ACTION_QUEUE_CAPACITY);
        (
            handle,
            ActionEmitterSnapshotHandle::new(snapshot_tx),
            Self::new(rx, snapshot_rx),
        )
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

    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "pending_len"))]
    pub fn pending_len(&self) -> usize {
        self.rx.len()
    }

    #[tracing::instrument(skip_all, fields(action_kind = "run"))]
    pub async fn run(mut self, cancel: CancellationToken) -> ActionStateSnapshot {
        loop {
            tokio::select! {
                Some((action, ack)) = self.rx.recv() => {
                    let result = self.execute(action).await;
                    let _send_result = ack.send(result);
                },
                Some(snapshot_ack) = self.snapshot_rx.recv() => {
                    let _send_result = snapshot_ack.send(self.snapshot());
                },
                () = cancel.cancelled() => {
                    self.release_all().await;
                    return self.snapshot();
                },
                else => {
                    self.release_all().await;
                    return self.snapshot();
                }
            }
        }
    }

    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "snapshot_state"))]
    pub fn snapshot(&self) -> ActionStateSnapshot {
        ActionStateSnapshot {
            held_keys: self.held_keys(),
            held_key_bits: self.held_keys.iter().collect(),
            held_buttons: self.held_buttons(),
            held_button_bits: self.held_buttons.iter().collect(),
            pad_state: self.pad_state.clone(),
        }
    }

    #[tracing::instrument(skip_all, fields(action_kind = %action_kind(&action)))]
    async fn execute(&mut self, action: Action) -> ActionResult<()> {
        match action {
            Action::KeyPress { key, .. } => {
                self.hold_key(&key);
                self.release_key(&key);
            }
            Action::KeyDown { key, .. } => self.hold_key(&key),
            Action::KeyUp { key, .. } => self.release_key(&key),
            Action::KeyChord { keys, .. } => self.apply_key_chord(&keys),
            Action::TypeText { .. }
            | Action::MouseMove { .. }
            | Action::MouseMoveRelative { .. }
            | Action::MouseDrag { .. }
            | Action::MouseScroll { .. }
            | Action::AimAt { .. } => {}
            Action::MouseButton { button, action, .. } => self.apply_mouse_button(button, action),
            Action::PadButton {
                pad,
                button,
                action,
                ..
            } => self.apply_pad_button(pad, button, action),
            Action::PadStick { pad, stick, x, y } => self.apply_pad_stick(pad, stick, x, y),
            Action::PadTrigger {
                pad,
                trigger,
                value,
            } => self.apply_pad_trigger(pad, trigger, value),
            Action::PadReport { pad, report } => self.apply_pad_report(pad, report),
            Action::Combo { steps, .. } => self.apply_combo(steps),
            Action::ReleaseAll => self.release_all().await,
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(action_kind = "release_all"))]
    async fn release_all(&mut self) {
        let released_keys = self.held_keys.count();
        let released_buttons = self.held_buttons.count();
        let released_pads = self.pad_state.len();
        self.held_keys.make_empty();
        self.held_buttons.make_empty();
        self.pad_state.clear();
        tracing::warn!(
            code = error_codes::SAFETY_RELEASE_ALL_FIRED,
            released_keys,
            released_buttons,
            released_pads,
            "release_all drained action emitter held state"
        );
    }

    fn apply_key_chord(&mut self, keys: &[Key]) {
        for key in keys {
            self.hold_key(key);
        }
        for key in keys {
            self.release_key(key);
        }
    }

    fn apply_combo(&mut self, steps: Vec<synapse_core::ComboStep>) {
        for step in steps {
            match step.input {
                ComboInput::KeyDown { key } => self.hold_key(&key),
                ComboInput::KeyUp { key } | ComboInput::KeyPress { key, .. } => {
                    self.release_key(&key);
                }
                ComboInput::MouseButton { button, action } => {
                    self.apply_mouse_button(button, action);
                }
                ComboInput::MouseMoveRel { .. } => {}
                ComboInput::PadButton {
                    pad,
                    button,
                    action,
                } => self.apply_pad_button(pad, button, action),
                ComboInput::PadStick { pad, stick, x, y } => {
                    self.apply_pad_stick(pad, stick, x, y);
                }
            }
        }
    }

    fn hold_key(&mut self, key: &Key) {
        let index = self.key_index(key);
        self.held_keys.insert(index);
    }

    fn release_key(&mut self, key: &Key) {
        if let Some(index) = self.key_indices.get(key) {
            self.held_keys.remove(*index);
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

    fn held_keys(&self) -> Vec<Key> {
        self.held_keys
            .iter()
            .filter_map(|index| self.keys_by_index.get(index).cloned())
            .collect()
    }

    fn apply_mouse_button(&mut self, button: MouseButton, action: ButtonAction) {
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

    fn held_buttons(&self) -> Vec<MouseButton> {
        self.held_buttons
            .iter()
            .filter_map(mouse_button_from_index)
            .collect()
    }

    fn apply_pad_button(&mut self, pad: PadId, button: PadButton, action: ButtonAction) {
        let should_remove = {
            let report = self
                .pad_state
                .entry(pad)
                .or_insert_with(neutral_gamepad_report);
            match action {
                ButtonAction::Down => push_unique(&mut report.buttons, button),
                ButtonAction::Up | ButtonAction::Press => {
                    report.buttons.retain(|held| *held != button);
                }
            }
            is_neutral_report(report)
        };

        if should_remove {
            self.pad_state.remove(&pad);
        }
    }

    fn apply_pad_stick(&mut self, pad: PadId, stick: Stick, x: f32, y: f32) {
        let should_remove = {
            let report = self
                .pad_state
                .entry(pad)
                .or_insert_with(neutral_gamepad_report);
            match stick {
                Stick::Left => report.thumb_l = (x, y),
                Stick::Right => report.thumb_r = (x, y),
            }
            is_neutral_report(report)
        };

        if should_remove {
            self.pad_state.remove(&pad);
        }
    }

    fn apply_pad_trigger(&mut self, pad: PadId, trigger: Trigger, value: f32) {
        let should_remove = {
            let report = self
                .pad_state
                .entry(pad)
                .or_insert_with(neutral_gamepad_report);
            match trigger {
                Trigger::Left => report.lt = value,
                Trigger::Right => report.rt = value,
            }
            is_neutral_report(report)
        };

        if should_remove {
            self.pad_state.remove(&pad);
        }
    }

    fn apply_pad_report(&mut self, pad: PadId, report: GamepadReport) {
        if is_neutral_report(&report) {
            self.pad_state.remove(&pad);
        } else {
            self.pad_state.insert(pad, report);
        }
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

const fn neutral_gamepad_report() -> GamepadReport {
    GamepadReport {
        buttons: Vec::new(),
        thumb_l: (0.0, 0.0),
        thumb_r: (0.0, 0.0),
        lt: 0.0,
        rt: 0.0,
    }
}

fn is_neutral_report(report: &GamepadReport) -> bool {
    report.buttons.is_empty()
        && report.thumb_l == (0.0, 0.0)
        && report.thumb_r == (0.0, 0.0)
        && report.lt == 0.0
        && report.rt == 0.0
}

fn push_unique(buttons: &mut Vec<PadButton>, button: PadButton) {
    if !buttons.contains(&button) {
        buttons.push(button);
    }
}
