use super::{AppendAck, Wal};
use calyx_core::{CalyxError, Clock, Result};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

type BatchReply = Sender<Result<BatchResponse>>;

enum BatchOp {
    Append(Vec<u8>),
    Flush,
    TipSeq,
}

enum BatchResponse {
    Ack(AppendAck),
    Flush,
    TipSeq(u64),
}

struct BatchRequest {
    op: BatchOp,
    respond: BatchReply,
}

/// Fsync-backed group commit wrapper around `Wal`.
#[derive(Debug)]
pub struct GroupCommitBatcher {
    sender: Sender<BatchRequest>,
    _thread: JoinHandle<()>,
}

impl GroupCommitBatcher {
    pub fn new(wal: Wal, group_commit_window: Duration, clock: Arc<dyn Clock>) -> Result<Self> {
        validate_window(group_commit_window)?;
        let (sender, receiver) = mpsc::channel();
        let wal = Arc::new(Mutex::new(wal));
        let thread = thread::spawn(move || run_batcher(wal, receiver, group_commit_window, clock));
        Ok(Self {
            sender,
            _thread: thread,
        })
    }

    pub fn submit(&self, payload: Vec<u8>) -> Result<AppendAck> {
        let (respond, receive) = mpsc::channel();
        self.sender
            .send(BatchRequest {
                op: BatchOp::Append(payload),
                respond,
            })
            .map_err(|_| CalyxError::disk_pressure("group commit batcher is closed"))?;
        match receive
            .recv()
            .map_err(|_| CalyxError::disk_pressure("group commit response channel closed"))?
        {
            Ok(BatchResponse::Ack(ack)) => Ok(ack),
            Ok(_) => Err(CalyxError::disk_pressure("missing WAL ack")),
            Err(error) => Err(error),
        }
    }

    pub fn flush_sync(&self) -> Result<()> {
        let (respond, receive) = mpsc::channel();
        self.sender
            .send(BatchRequest {
                op: BatchOp::Flush,
                respond,
            })
            .map_err(|_| CalyxError::disk_pressure("group commit batcher is closed"))?;
        match receive
            .recv()
            .map_err(|_| CalyxError::disk_pressure("group commit flush channel closed"))?
        {
            Ok(BatchResponse::Flush) => Ok(()),
            Ok(_) => Err(CalyxError::disk_pressure("missing WAL flush ack")),
            Err(error) => Err(error),
        }
    }

    pub fn tip_seq(&self) -> Result<u64> {
        let (respond, receive) = mpsc::channel();
        self.sender
            .send(BatchRequest {
                op: BatchOp::TipSeq,
                respond,
            })
            .map_err(|_| CalyxError::disk_pressure("group commit batcher is closed"))?;
        match receive
            .recv()
            .map_err(|_| CalyxError::disk_pressure("group commit tip channel closed"))?
        {
            Ok(BatchResponse::TipSeq(seq)) => Ok(seq),
            Ok(_) => Err(CalyxError::disk_pressure("missing WAL tip ack")),
            Err(error) => Err(error),
        }
    }
}

pub(super) fn validate_window(window: Duration) -> Result<()> {
    if window > super::DEFAULT_GROUP_COMMIT_WINDOW {
        return Err(CalyxError::disk_pressure(
            "group_commit_window exceeds 2 ms limit",
        ));
    }
    Ok(())
}

fn run_batcher(
    wal: Arc<Mutex<Wal>>,
    receiver: Receiver<BatchRequest>,
    group_commit_window: Duration,
    _clock: Arc<dyn Clock>,
) {
    while let Ok(first) = receiver.recv() {
        if !matches!(first.op, BatchOp::Append(_)) {
            flush_requests(&wal, vec![first]);
            continue;
        }
        let mut requests = vec![first];
        match receiver.try_recv() {
            Ok(request) => requests.push(request),
            Err(TryRecvError::Empty) => {
                flush_requests(&wal, requests);
                continue;
            }
            Err(TryRecvError::Disconnected) => break,
        }
        let deadline = std::time::Instant::now() + group_commit_window;
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                break;
            }
            match receiver.recv_timeout(deadline.saturating_duration_since(now)) {
                Ok(request) => requests.push(request),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        flush_requests(&wal, requests);
    }
}

fn flush_requests(wal: &Mutex<Wal>, requests: Vec<BatchRequest>) {
    let payloads: Vec<_> = requests
        .iter()
        .filter_map(|request| match &request.op {
            BatchOp::Append(payload) => Some(payload.as_slice()),
            BatchOp::Flush | BatchOp::TipSeq => None,
        })
        .collect();
    let wants_tip = requests
        .iter()
        .any(|request| matches!(request.op, BatchOp::TipSeq));
    let (acks, tip_seq) = {
        let mut wal = wal.lock().expect("group commit WAL lock poisoned");
        let acks = if payloads.is_empty() {
            Vec::new()
        } else {
            match wal.append_batch(&payloads) {
                Ok(acks) => acks,
                Err(error) => return send_error(&requests, error),
            }
        };
        let tip_seq = if wants_tip {
            match wal.durable_tip_seq() {
                Ok(seq) => Some(seq),
                Err(error) => return send_error(&requests, error),
            }
        } else {
            None
        };
        (acks, tip_seq)
    };
    let mut acks = acks.into_iter();
    for request in requests {
        let response = match request.op {
            BatchOp::Append(_) => acks
                .next()
                .map(BatchResponse::Ack)
                .ok_or_else(|| CalyxError::disk_pressure("missing WAL ack")),
            BatchOp::Flush => Ok(BatchResponse::Flush),
            BatchOp::TipSeq => tip_seq
                .map(BatchResponse::TipSeq)
                .ok_or_else(|| CalyxError::disk_pressure("missing WAL tip")),
        };
        let _ = request.respond.send(response);
    }
}

fn send_error(requests: &[BatchRequest], error: CalyxError) {
    for request in requests {
        let _ = request.respond.send(Err(error.clone()));
    }
}
