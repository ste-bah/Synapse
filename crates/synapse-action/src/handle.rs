use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

use synapse_core::Action;
use tokio::sync::{mpsc, oneshot};

use crate::{ActionError, ActionResult, validate_action};

pub const ACTION_QUEUE_CAPACITY: usize = 256;

pub type ActionMessage = (Action, oneshot::Sender<ActionResult<()>>);

pub static RELEASE_ALL_HANDLE: OnceLock<ActionHandle> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct ActionHandle {
    tx: mpsc::Sender<ActionMessage>,
}

impl ActionHandle {
    #[must_use]
    pub const fn new(tx: mpsc::Sender<ActionMessage>) -> Self {
        Self { tx }
    }

    #[must_use]
    pub fn channel() -> (Self, mpsc::Receiver<ActionMessage>) {
        let (tx, rx) = mpsc::channel(ACTION_QUEUE_CAPACITY);
        (Self::new(tx), rx)
    }

    /// Enqueues an action and waits for the emitter acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns `ACTION_BACKEND_UNAVAILABLE` when the emitter channel or
    /// acknowledgement path is closed, or the emitter's own `ActionError`.
    pub async fn execute(&self, action: Action) -> ActionResult<()> {
        validate_action(&action)?;
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send((action, ack_tx))
            .await
            .map_err(|_err| ActionError::BackendUnavailable {
                detail: "action emitter channel is closed".to_owned(),
            })?;
        ack_rx
            .await
            .map_err(|_err| ActionError::BackendUnavailable {
                detail: "action emitter dropped acknowledgement".to_owned(),
            })?
    }

    /// Attempts to enqueue an action without waiting for emitter completion.
    ///
    /// # Errors
    ///
    /// Returns `ACTION_QUEUE_FULL` when the bounded queue is saturated, or
    /// `ACTION_BACKEND_UNAVAILABLE` when the emitter channel is closed.
    pub fn try_execute(&self, action: Action) -> ActionResult<()> {
        validate_action(&action)?;
        let (ack_tx, _ack_rx) = oneshot::channel();
        self.tx.try_send((action, ack_tx)).map_err(map_try_send)?;
        Ok(())
    }

    /// Enqueues `ReleaseAll` and synchronously waits for its acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns `ACTION_QUEUE_FULL` if the bounded queue is saturated, or
    /// `ACTION_BACKEND_UNAVAILABLE` if the acknowledgement closes or times out.
    pub fn fire_release_all_blocking_with_timeout(&self, timeout: Duration) -> ActionResult<()> {
        let (ack_tx, mut ack_rx) = oneshot::channel();
        self.tx
            .try_send((Action::ReleaseAll, ack_tx))
            .map_err(map_try_send)?;

        let deadline = Instant::now() + timeout;
        loop {
            match ack_rx.try_recv() {
                Ok(result) => return result,
                Err(oneshot::error::TryRecvError::Closed) => {
                    return Err(ActionError::BackendUnavailable {
                        detail: "release_all acknowledgement channel closed".to_owned(),
                    });
                }
                Err(oneshot::error::TryRecvError::Empty) if Instant::now() >= deadline => {
                    return Err(ActionError::BackendUnavailable {
                        detail: format!("release_all acknowledgement timed out after {timeout:?}"),
                    });
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}

fn map_try_send(error: mpsc::error::TrySendError<ActionMessage>) -> ActionError {
    match error {
        mpsc::error::TrySendError::Full(_message) => ActionError::QueueFull {
            detail: format!("action queue capacity {ACTION_QUEUE_CAPACITY} is full"),
        },
        mpsc::error::TrySendError::Closed(_message) => ActionError::BackendUnavailable {
            detail: "action emitter channel is closed".to_owned(),
        },
    }
}
