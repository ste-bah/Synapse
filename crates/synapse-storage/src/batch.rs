use std::{
    sync::{
        Arc, mpsc,
        mpsc::{RecvTimeoutError, Sender},
    },
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use rocksdb::{DB, WriteBatch, WriteOptions};

use crate::{StorageError, StorageResult};

pub const FLUSH_INTERVAL: Duration = Duration::from_millis(100);
pub const FLUSH_BYTES: usize = 64 * 1024;
const STORAGE_WRITE_BATCH_FLUSHES_TOTAL: &str = "storage_write_batch_flushes_total";

pub struct Batcher {
    sender: Sender<Command>,
    worker: Option<JoinHandle<()>>,
}

impl Batcher {
    pub fn spawn(db: Arc<DB>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || Worker { db, receiver }.run());
        Self {
            sender,
            worker: Some(worker),
        }
    }

    pub fn put_batch(&self, cf_name: &str, kvs: Vec<(Vec<u8>, Vec<u8>)>) -> StorageResult<()> {
        let (reply, wait) = mpsc::sync_channel(1);
        self.sender
            .send(Command::Write {
                cf_name: cf_name.to_owned(),
                kvs,
                reply,
            })
            .map_err(|error| write_failed(cf_name, error.to_string()))?;
        wait.recv()
            .map_err(|error| write_failed(cf_name, error.to_string()))?
    }

    pub fn flush(&self) -> StorageResult<()> {
        let (reply, wait) = mpsc::sync_channel(1);
        self.sender
            .send(Command::Flush { reply })
            .map_err(|error| write_failed("batcher", error.to_string()))?;
        wait.recv()
            .map_err(|error| write_failed("batcher", error.to_string()))?
    }
}

impl Drop for Batcher {
    fn drop(&mut self) {
        let (reply, wait) = mpsc::sync_channel(1);
        let _ = self.sender.send(Command::Shutdown { reply });
        let _ = wait.recv_timeout(FLUSH_INTERVAL.saturating_mul(2));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

enum Command {
    Write {
        cf_name: String,
        kvs: Vec<(Vec<u8>, Vec<u8>)>,
        reply: mpsc::SyncSender<StorageResult<()>>,
    },
    Flush {
        reply: mpsc::SyncSender<StorageResult<()>>,
    },
    Shutdown {
        reply: mpsc::SyncSender<StorageResult<()>>,
    },
}

struct Worker {
    db: Arc<DB>,
    receiver: mpsc::Receiver<Command>,
}

impl Worker {
    fn run(self) {
        let mut pending = PendingBatch::default();
        loop {
            match receive_next(&self.receiver, &pending) {
                Ok(Command::Write {
                    cf_name,
                    kvs,
                    reply,
                }) => {
                    enqueue(&mut pending, &cf_name, kvs);
                    let result = if pending.bytes >= FLUSH_BYTES {
                        flush_pending(&self.db, &mut pending, false, "bytes")
                    } else {
                        Ok(())
                    };
                    let _ = reply.send(result);
                }
                Ok(Command::Flush { reply }) => {
                    let result = flush_pending(&self.db, &mut pending, true, "explicit");
                    let _ = reply.send(result);
                }
                Ok(Command::Shutdown { reply }) => {
                    let result = flush_pending(&self.db, &mut pending, true, "shutdown");
                    let _ = reply.send(result);
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {
                    let _ = flush_pending(&self.db, &mut pending, false, "interval");
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = flush_pending(&self.db, &mut pending, true, "disconnected");
                    break;
                }
            }
        }
    }
}

#[derive(Default)]
struct PendingBatch {
    writes: Vec<PendingWrite>,
    bytes: usize,
    first_write_at: Option<Instant>,
}

struct PendingWrite {
    cf_name: String,
    key: Vec<u8>,
    value: Vec<u8>,
}

fn receive_next(
    receiver: &mpsc::Receiver<Command>,
    pending: &PendingBatch,
) -> Result<Command, RecvTimeoutError> {
    pending.first_write_at.map_or_else(
        || {
            receiver
                .recv()
                .map_err(|_error| RecvTimeoutError::Disconnected)
        },
        |first_write_at| {
            receiver.recv_timeout(
                FLUSH_INTERVAL
                    .checked_sub(first_write_at.elapsed())
                    .unwrap_or(Duration::ZERO),
            )
        },
    )
}

fn enqueue(pending: &mut PendingBatch, cf_name: &str, kvs: Vec<(Vec<u8>, Vec<u8>)>) {
    if kvs.is_empty() {
        return;
    }
    if pending.first_write_at.is_none() {
        pending.first_write_at = Some(Instant::now());
    }
    for (key, value) in kvs {
        pending.bytes = pending
            .bytes
            .saturating_add(key.len())
            .saturating_add(value.len());
        pending.writes.push(PendingWrite {
            cf_name: cf_name.to_owned(),
            key,
            value,
        });
    }
}

fn flush_pending(
    db: &DB,
    pending: &mut PendingBatch,
    sync: bool,
    trigger: &'static str,
) -> StorageResult<()> {
    if pending.writes.is_empty() {
        if sync {
            db.flush_wal(true)
                .map_err(|error| write_failed("batcher", error.to_string()))?;
        }
        return Ok(());
    }

    let mut batch = WriteBatch::default();
    for write in &pending.writes {
        let cf = db.cf_handle(&write.cf_name).ok_or_else(|| {
            write_failed(
                &write.cf_name,
                "column family handle missing while flushing batch".to_owned(),
            )
        })?;
        batch.put_cf(&cf, &write.key, &write.value);
    }

    let mut options = WriteOptions::default();
    options.set_sync(sync);
    db.write_opt(batch, &options)
        .map_err(|error| write_failed("batcher", error.to_string()))?;
    if sync {
        db.flush_wal(true)
            .map_err(|error| write_failed("batcher", error.to_string()))?;
    }

    pending.writes.clear();
    pending.bytes = 0;
    pending.first_write_at = None;
    synapse_telemetry::metrics::counter!(STORAGE_WRITE_BATCH_FLUSHES_TOTAL, "trigger" => trigger)
        .increment(1);
    Ok(())
}

fn write_failed(cf_name: &str, detail: String) -> StorageError {
    StorageError::WriteFailed {
        cf_name: cf_name.to_owned(),
        detail,
    }
}
