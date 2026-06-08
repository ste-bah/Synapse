use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use synapse_core::error_codes;
use tokio_util::sync::CancellationToken;

const DRAIN_REASON_CODE: &str = error_codes::DAEMON_RESTARTING;

#[derive(Clone, Debug, Default)]
pub(crate) struct DaemonDrainState {
    inner: Arc<Mutex<Option<DaemonDrainReason>>>,
    cancel: CancellationToken,
}

#[derive(Clone, Debug)]
struct DaemonDrainReason {
    source: &'static str,
    reason_code: &'static str,
    started_at_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DaemonDrainSnapshot {
    pub draining: bool,
    pub reason_code: Option<&'static str>,
    pub source: Option<&'static str>,
    pub started_at_unix_ms: Option<u64>,
    pub state_error: Option<String>,
}

impl DaemonDrainState {
    pub(crate) fn mark_draining(&self, source: &'static str) -> DaemonDrainSnapshot {
        match self.inner.lock() {
            Ok(mut guard) => {
                let reason = guard.get_or_insert_with(|| DaemonDrainReason {
                    source,
                    reason_code: DRAIN_REASON_CODE,
                    started_at_unix_ms: unix_ms_now(),
                });
                tracing::warn!(
                    code = DRAIN_REASON_CODE,
                    source = reason.source,
                    started_at_unix_ms = reason.started_at_unix_ms,
                    "daemon entered drain/restarting state"
                );
                self.cancel.cancel();
                snapshot_from_reason(Some(reason), None)
            }
            Err(_error) => poison_snapshot(),
        }
    }

    pub(crate) fn snapshot(&self) -> DaemonDrainSnapshot {
        match self.inner.lock() {
            Ok(guard) => snapshot_from_reason(guard.as_ref(), None),
            Err(_error) => poison_snapshot(),
        }
    }

    pub(crate) fn token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

fn snapshot_from_reason(
    reason: Option<&DaemonDrainReason>,
    state_error: Option<String>,
) -> DaemonDrainSnapshot {
    DaemonDrainSnapshot {
        draining: reason.is_some() || state_error.is_some(),
        reason_code: reason.map(|reason| reason.reason_code),
        source: reason.map(|reason| reason.source),
        started_at_unix_ms: reason.map(|reason| reason.started_at_unix_ms),
        state_error,
    }
}

fn poison_snapshot() -> DaemonDrainSnapshot {
    snapshot_from_reason(
        Some(&DaemonDrainReason {
            source: "drain_state_poisoned",
            reason_code: DRAIN_REASON_CODE,
            started_at_unix_ms: unix_ms_now(),
        }),
        Some("daemon drain state lock poisoned".to_owned()),
    )
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}
