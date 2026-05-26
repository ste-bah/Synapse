use std::collections::HashMap;

use synapse_core::Key;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::ActionMessage;

mod backends;
mod dispatch;
mod keyboard;
mod lifecycle;
mod rate_limits;
mod routing;
mod state;

#[cfg(test)]
mod tests;

pub use backends::{Backends, HardwareHidConfig};
pub use state::{
    ActionEmitterSnapshotHandle, ActionSnapshotMessage, ActionStateSnapshot, EmitState,
};

use keyboard::HeldKeyAutoRelease;
use rate_limits::BackendRateLimits;

pub const HELD_KEY_MAX_DURATION_MS: u64 = 30_000;

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
