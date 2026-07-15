//! Async boundary for the synchronous Calyx Aster vault.

use std::collections::BTreeMap;
use std::thread::{self, JoinHandle};

use calyx_aster::cf::{ColumnFamily, KeyRange};
use calyx_aster::mvcc::{Freshness, Snapshot};
use calyx_core::Seq;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};

use crate::{
    OPEN_REMEDIATION, SynapseCalyxCfRows, SynapseCalyxConfig, SynapseCalyxError, SynapseCalyxVault,
    SynapseCalyxVaultCloseReadback, SynapseCalyxVaultStatus,
};

const DEFAULT_QUEUE_CAPACITY: usize = 1024;
const ASYNC_REMEDIATION: &str =
    "inspect the async Calyx vault worker log, queue capacity, and daemon shutdown path";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SynapseCalyxAsyncConfig {
    /// Maximum accepted-but-not-yet-owned commands. Tokio `send().await`
    /// provides backpressure at this bound instead of blocking an executor
    /// worker thread.
    pub queue_capacity: usize,
}

impl Default for SynapseCalyxAsyncConfig {
    fn default() -> Self {
        Self {
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
        }
    }
}

impl SynapseCalyxAsyncConfig {
    fn validate(self) -> Result<(), SynapseCalyxError> {
        if self.queue_capacity == 0 {
            return Err(SynapseCalyxError::new(
                "SYNAPSE_CALYX_ASYNC_QUEUE_CAPACITY_INVALID",
                "async Calyx vault queue_capacity must be greater than zero",
                "set queue_capacity to a positive bounded value sized to the daemon's accepted in-flight storage work",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynapseCalyxCfWrite {
    pub cf: ColumnFamily,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl SynapseCalyxCfWrite {
    #[must_use]
    pub fn new(cf: ColumnFamily, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Self {
        Self {
            cf,
            key: key.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SynapseCalyxReaderLease {
    pub lease_id: u64,
    pub pinned_seq: Seq,
    pub issued_at_ms: u64,
    pub max_age_ms: u64,
    pub expires_at_ms: u64,
    pub derived_content_seq: Seq,
}

impl From<Snapshot> for SynapseCalyxReaderLease {
    fn from(snapshot: Snapshot) -> Self {
        let lease = snapshot.lease();
        Self {
            lease_id: lease.id(),
            pinned_seq: lease.pinned_seq(),
            issued_at_ms: lease.issued_at(),
            max_age_ms: lease.max_age_ms(),
            expires_at_ms: lease.expires_at(),
            derived_content_seq: snapshot.derived_content_seq(),
        }
    }
}

#[derive(Debug)]
pub struct SynapseCalyxAsyncVault {
    handle: SynapseCalyxAsyncVaultHandle,
    worker: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
pub struct SynapseCalyxAsyncVaultHandle {
    sender: mpsc::Sender<VaultCommand>,
    queue_capacity: usize,
}

impl SynapseCalyxAsyncVault {
    /// Opens a durable Calyx vault on Tokio's blocking pool, then moves the
    /// synchronous handle to one dedicated owner thread.
    ///
    /// # Errors
    ///
    /// Returns a structured error when configuration validation, blocking open,
    /// lock acquisition, durable recovery, or worker startup fails.
    pub async fn open(
        config: SynapseCalyxConfig,
        async_config: SynapseCalyxAsyncConfig,
    ) -> Result<Self, SynapseCalyxError> {
        async_config.validate()?;
        tokio::task::spawn_blocking(move || {
            let vault = SynapseCalyxVault::open(config)?;
            Self::from_open_vault(vault, async_config)
        })
        .await
        .map_err(|error| {
            SynapseCalyxError::new(
                "SYNAPSE_CALYX_ASYNC_OPEN_TASK_FAILED",
                format!("join async Calyx vault open task: {error}"),
                OPEN_REMEDIATION,
            )
        })?
    }

    /// Wraps an already-open synchronous vault in the async owner-thread facade.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the bounded queue capacity is invalid
    /// or the worker thread cannot be spawned.
    pub fn from_open_vault(
        vault: SynapseCalyxVault,
        async_config: SynapseCalyxAsyncConfig,
    ) -> Result<Self, SynapseCalyxError> {
        async_config.validate()?;
        let (sender, receiver) = mpsc::channel(async_config.queue_capacity);
        let worker = thread::Builder::new()
            .name("synapse-calyx-vault".to_owned())
            .spawn(move || run_worker(vault, receiver))
            .map_err(|error| {
                SynapseCalyxError::new(
                    "SYNAPSE_CALYX_ASYNC_WORKER_SPAWN_FAILED",
                    format!("spawn async Calyx vault worker: {error}"),
                    ASYNC_REMEDIATION,
                )
            })?;
        Ok(Self {
            handle: SynapseCalyxAsyncVaultHandle {
                sender,
                queue_capacity: async_config.queue_capacity,
            },
            worker: Some(worker),
        })
    }

    #[must_use]
    pub fn handle(&self) -> SynapseCalyxAsyncVaultHandle {
        self.handle.clone()
    }

    #[must_use]
    pub const fn queue_capacity(&self) -> usize {
        self.handle.queue_capacity()
    }

    /// Reads the currently opened vault status through the worker.
    ///
    /// # Errors
    ///
    /// Returns a structured async vault error if the command cannot be
    /// submitted or the worker drops the reply.
    pub async fn status(&self) -> Result<SynapseCalyxVaultStatus, SynapseCalyxError> {
        self.handle.status().await
    }

    /// Writes a raw CF batch through the worker-owned durable vault.
    ///
    /// # Errors
    ///
    /// Returns a structured error if queue submission fails, the worker reply
    /// is dropped, or the underlying Calyx WAL/MVCC commit path rejects the
    /// batch.
    pub async fn write_cf_batch(
        &self,
        rows: Vec<SynapseCalyxCfWrite>,
    ) -> Result<Seq, SynapseCalyxError> {
        self.handle.write_cf_batch(rows).await
    }

    /// Reads one raw CF row at a numeric snapshot through the worker.
    ///
    /// # Errors
    ///
    /// Returns a structured error if command submission/reply handling fails
    /// or the underlying snapshot read fails.
    pub async fn read_cf_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, SynapseCalyxError> {
        self.handle.read_cf_at(snapshot, cf, key).await
    }

    /// Reads one raw CF row through an existing async pinned reader lease.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the lease is missing or expired, command
    /// submission/reply handling fails, or the underlying snapshot read fails.
    pub async fn read_cf_pinned(
        &self,
        lease_id: u64,
        cf: ColumnFamily,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, SynapseCalyxError> {
        self.handle.read_cf_pinned(lease_id, cf, key).await
    }

    /// Scans visible raw CF rows at a numeric snapshot through the worker.
    ///
    /// # Errors
    ///
    /// Returns a structured error if command submission/reply handling fails
    /// or the underlying snapshot scan fails.
    pub async fn scan_cf_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
    ) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
        self.handle.scan_cf_at(snapshot, cf).await
    }

    /// Scans visible raw CF rows through an existing async pinned reader lease.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the lease is missing or expired, command
    /// submission/reply handling fails, or the underlying snapshot scan fails.
    pub async fn scan_cf_pinned(
        &self,
        lease_id: u64,
        cf: ColumnFamily,
    ) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
        self.handle.scan_cf_pinned(lease_id, cf).await
    }

    /// Scans visible raw CF rows in a key range at a numeric snapshot.
    ///
    /// # Errors
    ///
    /// Returns a structured error if command submission/reply handling fails
    /// or the underlying range scan fails.
    pub async fn scan_cf_range_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        range: KeyRange,
    ) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
        self.handle.scan_cf_range_at(snapshot, cf, range).await
    }

    /// Scans visible raw CF rows in a key range through a pinned reader lease.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the lease is missing or expired, command
    /// submission/reply handling fails, or the underlying range scan fails.
    pub async fn scan_cf_range_pinned(
        &self,
        lease_id: u64,
        cf: ColumnFamily,
        range: KeyRange,
    ) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
        self.handle.scan_cf_range_pinned(lease_id, cf, range).await
    }

    /// Pins a bounded reader lease in the worker-owned vault.
    ///
    /// # Errors
    ///
    /// Returns a structured error if command submission/reply handling fails
    /// or the requested lease lifetime is invalid.
    pub async fn pin_reader(
        &self,
        freshness: Freshness,
        max_age_ms: u64,
    ) -> Result<SynapseCalyxReaderLease, SynapseCalyxError> {
        self.handle.pin_reader(freshness, max_age_ms).await
    }

    /// Releases a previously pinned reader lease.
    ///
    /// # Errors
    ///
    /// Returns a structured error if command submission or reply handling
    /// fails.
    pub async fn release_reader(&self, lease_id: u64) -> Result<bool, SynapseCalyxError> {
        self.handle.release_reader(lease_id).await
    }

    /// Flushes the worker-owned durable vault.
    ///
    /// # Errors
    ///
    /// Returns a structured error if command submission/reply handling fails
    /// or the underlying WAL/checkpoint flush fails.
    pub async fn flush(&self) -> Result<(), SynapseCalyxError> {
        self.handle.flush().await
    }

    /// Flushes, closes, and joins the dedicated vault worker.
    ///
    /// # Errors
    ///
    /// Returns a structured error when close cannot be submitted, the worker
    /// drops the reply, the close readback fails, or the worker panics.
    pub async fn close(
        mut self,
        reason: &'static str,
    ) -> Result<SynapseCalyxVaultCloseReadback, SynapseCalyxError> {
        let Some(worker) = self.worker.take() else {
            return Err(SynapseCalyxError::new(
                "SYNAPSE_CALYX_ASYNC_WORKER_MISSING",
                "async Calyx vault close requested after worker owner was already consumed",
                ASYNC_REMEDIATION,
            ));
        };
        let readback = self
            .handle
            .request("close", |reply| VaultCommand::Close { reason, reply })
            .await;
        let join = join_worker(worker, "close").await;
        match (readback, join) {
            (Ok(readback), Ok(())) => Ok(readback),
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
            (Err(close_error), Err(join_error)) => Err(SynapseCalyxError::new(
                "SYNAPSE_CALYX_ASYNC_CLOSE_AND_JOIN_FAILED",
                format!("close_error={close_error}; join_error={join_error}"),
                ASYNC_REMEDIATION,
            )),
        }
    }
}

impl Drop for SynapseCalyxAsyncVault {
    fn drop(&mut self) {
        if self.worker.is_some() {
            tracing::error!(
                code = "SYNAPSE_CALYX_ASYNC_DROPPED_WITHOUT_CLOSE",
                "async Calyx vault owner dropped without explicit close; worker will close after command channel disconnects"
            );
        }
    }
}

impl SynapseCalyxAsyncVaultHandle {
    #[must_use]
    pub const fn queue_capacity(&self) -> usize {
        self.queue_capacity
    }

    /// Reads the currently opened vault status through this handle.
    ///
    /// # Errors
    ///
    /// Returns a structured async vault error if the command cannot be
    /// submitted or the worker drops the reply.
    pub async fn status(&self) -> Result<SynapseCalyxVaultStatus, SynapseCalyxError> {
        self.request("status", |reply| VaultCommand::Status { reply })
            .await
    }

    /// Writes a raw CF batch through the worker-owned durable vault.
    ///
    /// # Errors
    ///
    /// Returns a structured error if queue submission fails, the worker reply
    /// is dropped, or the underlying Calyx WAL/MVCC commit path rejects the
    /// batch.
    pub async fn write_cf_batch(
        &self,
        rows: Vec<SynapseCalyxCfWrite>,
    ) -> Result<Seq, SynapseCalyxError> {
        self.request("write_cf_batch", |reply| VaultCommand::WriteCfBatch {
            rows,
            reply,
        })
        .await
    }

    /// Reads one raw CF row at a numeric snapshot through this handle.
    ///
    /// # Errors
    ///
    /// Returns a structured error if command submission/reply handling fails
    /// or the underlying snapshot read fails.
    pub async fn read_cf_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, SynapseCalyxError> {
        self.request("read_cf_at", |reply| VaultCommand::ReadCfAt {
            snapshot,
            cf,
            key,
            reply,
        })
        .await
    }

    /// Reads one raw CF row through an existing async pinned reader lease.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the lease is missing or expired, command
    /// submission/reply handling fails, or the underlying snapshot read fails.
    pub async fn read_cf_pinned(
        &self,
        lease_id: u64,
        cf: ColumnFamily,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, SynapseCalyxError> {
        self.request("read_cf_pinned", |reply| VaultCommand::ReadCfPinned {
            lease_id,
            cf,
            key,
            reply,
        })
        .await
    }

    /// Scans visible raw CF rows at a numeric snapshot through this handle.
    ///
    /// # Errors
    ///
    /// Returns a structured error if command submission/reply handling fails
    /// or the underlying snapshot scan fails.
    pub async fn scan_cf_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
    ) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
        self.request("scan_cf_at", |reply| VaultCommand::ScanCfAt {
            snapshot,
            cf,
            reply,
        })
        .await
    }

    /// Scans visible raw CF rows through an existing async pinned reader lease.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the lease is missing or expired, command
    /// submission/reply handling fails, or the underlying snapshot scan fails.
    pub async fn scan_cf_pinned(
        &self,
        lease_id: u64,
        cf: ColumnFamily,
    ) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
        self.request("scan_cf_pinned", |reply| VaultCommand::ScanCfPinned {
            lease_id,
            cf,
            reply,
        })
        .await
    }

    /// Scans visible raw CF rows in a key range at a numeric snapshot.
    ///
    /// # Errors
    ///
    /// Returns a structured error if command submission/reply handling fails
    /// or the underlying range scan fails.
    pub async fn scan_cf_range_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        range: KeyRange,
    ) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
        self.request("scan_cf_range_at", |reply| VaultCommand::ScanCfRangeAt {
            snapshot,
            cf,
            range,
            reply,
        })
        .await
    }

    /// Scans visible raw CF rows in a key range through a pinned reader lease.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the lease is missing or expired, command
    /// submission/reply handling fails, or the underlying range scan fails.
    pub async fn scan_cf_range_pinned(
        &self,
        lease_id: u64,
        cf: ColumnFamily,
        range: KeyRange,
    ) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
        self.request("scan_cf_range_pinned", |reply| {
            VaultCommand::ScanCfRangePinned {
                lease_id,
                cf,
                range,
                reply,
            }
        })
        .await
    }

    /// Pins a bounded reader lease in the worker-owned vault.
    ///
    /// # Errors
    ///
    /// Returns a structured error if command submission/reply handling fails
    /// or the requested lease lifetime is invalid.
    pub async fn pin_reader(
        &self,
        freshness: Freshness,
        max_age_ms: u64,
    ) -> Result<SynapseCalyxReaderLease, SynapseCalyxError> {
        self.request("pin_reader", |reply| VaultCommand::PinReader {
            freshness,
            max_age_ms,
            reply,
        })
        .await
    }

    /// Releases a previously pinned reader lease.
    ///
    /// # Errors
    ///
    /// Returns a structured error if command submission or reply handling
    /// fails.
    pub async fn release_reader(&self, lease_id: u64) -> Result<bool, SynapseCalyxError> {
        self.request("release_reader", |reply| VaultCommand::ReleaseReader {
            lease_id,
            reply,
        })
        .await
    }

    /// Flushes the worker-owned durable vault.
    ///
    /// # Errors
    ///
    /// Returns a structured error if command submission/reply handling fails
    /// or the underlying WAL/checkpoint flush fails.
    pub async fn flush(&self) -> Result<(), SynapseCalyxError> {
        self.request("flush", |reply| VaultCommand::Flush { reply })
            .await
    }

    async fn request<T>(
        &self,
        operation: &'static str,
        build: impl FnOnce(oneshot::Sender<Result<T, SynapseCalyxError>>) -> VaultCommand,
    ) -> Result<T, SynapseCalyxError>
    where
        T: Send + 'static,
    {
        let (reply, receive) = oneshot::channel();
        self.sender.send(build(reply)).await.map_err(|error| {
            let message = format!("submit async Calyx vault command {operation}: {error}");
            tracing::error!(
                code = "SYNAPSE_CALYX_ASYNC_QUEUE_CLOSED",
                operation,
                error = %error,
                "async Calyx vault command queue is closed"
            );
            SynapseCalyxError::new(
                "SYNAPSE_CALYX_ASYNC_QUEUE_CLOSED",
                message,
                ASYNC_REMEDIATION,
            )
        })?;
        receive.await.map_err(|error| {
            let message = format!("await async Calyx vault command {operation}: {error}");
            tracing::error!(
                code = "SYNAPSE_CALYX_ASYNC_REPLY_DROPPED",
                operation,
                error = %error,
                "async Calyx vault worker dropped a command reply"
            );
            SynapseCalyxError::new(
                "SYNAPSE_CALYX_ASYNC_REPLY_DROPPED",
                message,
                ASYNC_REMEDIATION,
            )
        })?
    }
}

enum VaultCommand {
    Status {
        reply: oneshot::Sender<Result<SynapseCalyxVaultStatus, SynapseCalyxError>>,
    },
    WriteCfBatch {
        rows: Vec<SynapseCalyxCfWrite>,
        reply: oneshot::Sender<Result<Seq, SynapseCalyxError>>,
    },
    ReadCfAt {
        snapshot: Seq,
        cf: ColumnFamily,
        key: Vec<u8>,
        reply: oneshot::Sender<Result<Option<Vec<u8>>, SynapseCalyxError>>,
    },
    ReadCfPinned {
        lease_id: u64,
        cf: ColumnFamily,
        key: Vec<u8>,
        reply: oneshot::Sender<Result<Option<Vec<u8>>, SynapseCalyxError>>,
    },
    ScanCfAt {
        snapshot: Seq,
        cf: ColumnFamily,
        reply: oneshot::Sender<Result<SynapseCalyxCfRows, SynapseCalyxError>>,
    },
    ScanCfPinned {
        lease_id: u64,
        cf: ColumnFamily,
        reply: oneshot::Sender<Result<SynapseCalyxCfRows, SynapseCalyxError>>,
    },
    ScanCfRangeAt {
        snapshot: Seq,
        cf: ColumnFamily,
        range: KeyRange,
        reply: oneshot::Sender<Result<SynapseCalyxCfRows, SynapseCalyxError>>,
    },
    ScanCfRangePinned {
        lease_id: u64,
        cf: ColumnFamily,
        range: KeyRange,
        reply: oneshot::Sender<Result<SynapseCalyxCfRows, SynapseCalyxError>>,
    },
    PinReader {
        freshness: Freshness,
        max_age_ms: u64,
        reply: oneshot::Sender<Result<SynapseCalyxReaderLease, SynapseCalyxError>>,
    },
    ReleaseReader {
        lease_id: u64,
        reply: oneshot::Sender<Result<bool, SynapseCalyxError>>,
    },
    Flush {
        reply: oneshot::Sender<Result<(), SynapseCalyxError>>,
    },
    Close {
        reason: &'static str,
        reply: oneshot::Sender<Result<SynapseCalyxVaultCloseReadback, SynapseCalyxError>>,
    },
}

fn run_worker(vault: SynapseCalyxVault, mut receiver: mpsc::Receiver<VaultCommand>) {
    let mut vault = Some(vault);
    let mut pinned = BTreeMap::<u64, Snapshot>::new();
    while let Some(command) = receiver.blocking_recv() {
        if let VaultCommand::Close { reason, reply } = command {
            close_worker(&mut vault, &mut pinned, reason, reply);
            break;
        }
        let Some(current) = vault.as_ref() else {
            tracing::error!(
                code = "SYNAPSE_CALYX_ASYNC_COMMAND_AFTER_CLOSE",
                "async Calyx vault worker received a command after closing the vault"
            );
            continue;
        };
        handle_command(current, &mut pinned, command);
    }
    close_after_channel_disconnect(&mut vault, &mut pinned);
}

fn handle_command(
    current: &SynapseCalyxVault,
    pinned: &mut BTreeMap<u64, Snapshot>,
    command: VaultCommand,
) {
    match command {
        VaultCommand::Status { reply } => send_reply("status", reply, Ok(current.status())),
        VaultCommand::WriteCfBatch { rows, reply } => {
            send_reply("write_cf_batch", reply, current.write_cf_batch(rows));
        }
        VaultCommand::ReadCfAt {
            snapshot,
            cf,
            key,
            reply,
        } => send_reply("read_cf_at", reply, current.read_cf_at(snapshot, cf, &key)),
        VaultCommand::ReadCfPinned {
            lease_id,
            cf,
            key,
            reply,
        } => send_reply(
            "read_cf_pinned",
            reply,
            read_pinned_row(current, pinned, lease_id, cf, &key),
        ),
        VaultCommand::ScanCfAt {
            snapshot,
            cf,
            reply,
        } => send_reply("scan_cf_at", reply, current.scan_cf_at(snapshot, cf)),
        VaultCommand::ScanCfPinned {
            lease_id,
            cf,
            reply,
        } => send_reply(
            "scan_cf_pinned",
            reply,
            scan_pinned_rows(current, pinned, lease_id, cf),
        ),
        VaultCommand::ScanCfRangeAt {
            snapshot,
            cf,
            range,
            reply,
        } => send_reply(
            "scan_cf_range_at",
            reply,
            current.scan_cf_range_at(snapshot, cf, &range),
        ),
        VaultCommand::ScanCfRangePinned {
            lease_id,
            cf,
            range,
            reply,
        } => send_reply(
            "scan_cf_range_pinned",
            reply,
            scan_pinned_range(current, pinned, lease_id, cf, &range),
        ),
        VaultCommand::PinReader {
            freshness,
            max_age_ms,
            reply,
        } => send_reply(
            "pin_reader",
            reply,
            pin_reader(current, pinned, freshness, max_age_ms),
        ),
        VaultCommand::ReleaseReader { lease_id, reply } => {
            pinned.remove(&lease_id);
            send_reply(
                "release_reader",
                reply,
                Ok(current.release_reader(lease_id)),
            );
        }
        VaultCommand::Flush { reply } => send_reply("flush", reply, current.flush()),
        VaultCommand::Close { .. } => unreachable!("close handled before worker command dispatch"),
    }
}

fn read_pinned_row(
    vault: &SynapseCalyxVault,
    pinned: &mut BTreeMap<u64, Snapshot>,
    lease_id: u64,
    cf: ColumnFamily,
    key: &[u8],
) -> Result<Option<Vec<u8>>, SynapseCalyxError> {
    let result = pinned_snapshot(pinned, lease_id)
        .and_then(|snapshot| vault.read_cf_snapshot(snapshot, cf, key));
    release_if_expired(vault, pinned, lease_id, &result);
    result
}

fn scan_pinned_rows(
    vault: &SynapseCalyxVault,
    pinned: &mut BTreeMap<u64, Snapshot>,
    lease_id: u64,
    cf: ColumnFamily,
) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
    let result =
        pinned_snapshot(pinned, lease_id).and_then(|snapshot| vault.scan_cf_snapshot(snapshot, cf));
    release_if_expired(vault, pinned, lease_id, &result);
    result
}

fn scan_pinned_range(
    vault: &SynapseCalyxVault,
    pinned: &mut BTreeMap<u64, Snapshot>,
    lease_id: u64,
    cf: ColumnFamily,
    range: &KeyRange,
) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
    let result = pinned_snapshot(pinned, lease_id)
        .and_then(|snapshot| vault.scan_cf_range_snapshot(snapshot, cf, range));
    release_if_expired(vault, pinned, lease_id, &result);
    result
}

fn pin_reader(
    vault: &SynapseCalyxVault,
    pinned: &mut BTreeMap<u64, Snapshot>,
    freshness: Freshness,
    max_age_ms: u64,
) -> Result<SynapseCalyxReaderLease, SynapseCalyxError> {
    vault.pin_reader(freshness, max_age_ms).map(|snapshot| {
        let lease = SynapseCalyxReaderLease::from(snapshot);
        pinned.insert(lease.lease_id, snapshot);
        lease
    })
}

fn close_worker(
    vault: &mut Option<SynapseCalyxVault>,
    pinned: &mut BTreeMap<u64, Snapshot>,
    reason: &'static str,
    reply: oneshot::Sender<Result<SynapseCalyxVaultCloseReadback, SynapseCalyxError>>,
) {
    let Some(closing) = vault.take() else {
        send_reply(
            "close",
            reply,
            Err(SynapseCalyxError::new(
                "SYNAPSE_CALYX_ASYNC_WORKER_MISSING_VAULT",
                "async Calyx vault worker had no vault handle to close",
                ASYNC_REMEDIATION,
            )),
        );
        return;
    };
    release_all_pinned(&closing, pinned, "close");
    send_reply("close", reply, closing.close(reason));
}

fn close_after_channel_disconnect(
    vault: &mut Option<SynapseCalyxVault>,
    pinned: &mut BTreeMap<u64, Snapshot>,
) {
    let Some(closing) = vault.take() else {
        return;
    };
    release_all_pinned(&closing, pinned, "async_command_channel_disconnected");
    match closing.close("async_command_channel_disconnected") {
        Ok(readback) => tracing::warn!(
            code = "SYNAPSE_CALYX_ASYNC_CHANNEL_DISCONNECTED_CLOSED",
            safe_to_unlock = readback.safe_to_unlock,
            latest_seq = readback.latest_seq,
            "async Calyx vault command channel disconnected; worker closed vault"
        ),
        Err(error) => tracing::error!(
            code = error.code,
            error = %error,
            "async Calyx vault command channel disconnected and emergency close failed"
        ),
    }
}

fn release_all_pinned(
    vault: &SynapseCalyxVault,
    pinned: &mut BTreeMap<u64, Snapshot>,
    reason: &'static str,
) {
    let count = pinned.len();
    for lease_id in std::mem::take(pinned).into_keys() {
        let released = vault.release_reader(lease_id);
        tracing::debug!(
            code = "SYNAPSE_CALYX_READER_LEASE_RELEASED_ON_SHUTDOWN",
            reason,
            lease_id,
            released,
            "async Calyx vault released pinned reader lease during shutdown"
        );
    }
    if count > 0 {
        tracing::info!(
            code = "SYNAPSE_CALYX_READER_LEASES_RELEASED_ON_SHUTDOWN",
            reason,
            count,
            "async Calyx vault released all pinned reader leases during shutdown"
        );
    }
}

fn pinned_snapshot(
    pinned: &BTreeMap<u64, Snapshot>,
    lease_id: u64,
) -> Result<Snapshot, SynapseCalyxError> {
    pinned.get(&lease_id).copied().ok_or_else(|| {
        SynapseCalyxError::new(
            "SYNAPSE_CALYX_READER_LEASE_MISSING",
            format!("reader lease {lease_id} is not pinned in the async vault worker"),
            "re-issue a bounded reader lease and ensure reads use the returned lease_id before release or expiry",
        )
    })
}

fn release_if_expired<T>(
    vault: &SynapseCalyxVault,
    pinned: &mut BTreeMap<u64, Snapshot>,
    lease_id: u64,
    result: &Result<T, SynapseCalyxError>,
) {
    if result
        .as_ref()
        .err()
        .is_some_and(|error| error.code == "CALYX_READER_LEASE_EXPIRED")
    {
        pinned.remove(&lease_id);
        let released = vault.release_reader(lease_id);
        tracing::warn!(
            code = "SYNAPSE_CALYX_READER_LEASE_EXPIRED_RELEASED",
            lease_id,
            released,
            "async Calyx vault released an expired reader lease after fail-closed read"
        );
    }
}

fn send_reply<T>(
    operation: &'static str,
    reply: oneshot::Sender<Result<T, SynapseCalyxError>>,
    result: Result<T, SynapseCalyxError>,
) {
    if let Err(error) = &result {
        tracing::error!(
            code = error.code,
            operation,
            error = %error,
            "async Calyx vault command failed"
        );
    }
    if reply.send(result).is_err() {
        tracing::warn!(
            code = "SYNAPSE_CALYX_ASYNC_REPLY_RECEIVER_DROPPED",
            operation,
            "async Calyx vault caller dropped the reply receiver after command submission"
        );
    }
}

async fn join_worker(
    worker: JoinHandle<()>,
    operation: &'static str,
) -> Result<(), SynapseCalyxError> {
    let joined = tokio::task::spawn_blocking(move || worker.join())
        .await
        .map_err(|error| {
            SynapseCalyxError::new(
                "SYNAPSE_CALYX_ASYNC_JOIN_TASK_FAILED",
                format!("join async Calyx vault worker join task for {operation}: {error}"),
                ASYNC_REMEDIATION,
            )
        })?;
    joined.map_err(|_panic| {
        SynapseCalyxError::new(
            "SYNAPSE_CALYX_ASYNC_WORKER_PANICKED",
            format!("async Calyx vault worker panicked during {operation}"),
            ASYNC_REMEDIATION,
        )
    })
}
