//! `CF_AGENT_EVENTS` journal writer (#897).
//!
//! One row per agent lifecycle/telemetry event, keyed `(ts_ns, seq)` through
//! [`synapse_storage::agent_events`]. Writers: HTTP session store (session
//! initialized/restored/deleted), session lifecycle teardown (exited),
//! `act_spawn_agent` (spawn requested/ready/failed), the agent mailbox
//! (message sent/received), and the input-lease tools (acquired/released).
//! Turn/tool-call/token events arrive with the push-telemetry ingress (#899).
//!
//! # Durability contract (#897 acceptance)
//!
//! [`record_agent_event`] rides the storage batcher (`Db::put_batch`,
//! flushed every 100 ms / 64 KiB): a daemon crash loses at most the
//! in-flight batch. [`record_agent_event_durable`] additionally calls
//! `Db::flush()` so terminal lifecycle facts (exited, spawn failure,
//! session deleted) survive an immediate crash.
//!
//! # Failure contract
//!
//! A journal write failure is never swallowed: it logs a structured
//! `AGENT_EVENT_WRITE_FAILED` error with the full event context and is
//! returned to the caller. Tool handlers journal *after* the primary state
//! mutation commits (except inbox drains, which journal *before* deleting
//! rows so a failure can never lose messages); their errors carry
//! `operation_committed` so callers know whether the primary effect stands.

use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use rmcp::model::ErrorCode;
use serde_json::json;
use synapse_core::AgentEventRecord;
use synapse_storage::{
    Db, StorageError, StorageResult, agent_events::agent_event_key, cf, encode_json,
};

use super::ErrorData;

/// Hard cap on one encoded journal row. Agent events are bounded metadata;
/// anything larger indicates a writer leaking content into the journal.
pub(crate) const MAX_AGENT_EVENT_VALUE_BYTES: usize = 16 * 1024;

/// Process-wide tie-breaker for same-nanosecond events. Ordering authority
/// within one clock tick; wraps harmlessly because `ts_ns` dominates the key.
static NEXT_AGENT_EVENT_SEQ: AtomicU32 = AtomicU32::new(0);

/// Physical readback of one persisted journal row.
#[derive(Clone, Debug)]
pub(crate) struct AgentEventWriteReadback {
    pub ts_ns: u64,
    pub seq: u32,
    pub value_len_bytes: usize,
}

/// Current unix time in nanoseconds. A clock before the epoch yields 0,
/// which [`AgentEventRecord::validate`] refuses — the failure surfaces
/// instead of journaling rows the TTL filter could never expire correctly.
pub(crate) fn unix_time_ns_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
        .unwrap_or_default()
}

/// Validates, encodes, and enqueues one event row (batched write path).
///
/// # Errors
///
/// Returns [`StorageError::WriteFailed`] when the record fails validation,
/// exceeds [`MAX_AGENT_EVENT_VALUE_BYTES`], or the storage batcher rejects
/// the write. Every failure is also logged with `AGENT_EVENT_WRITE_FAILED`.
pub(crate) fn record_agent_event(
    db: &Db,
    record: &AgentEventRecord,
) -> StorageResult<AgentEventWriteReadback> {
    let mut readbacks = record_agent_events(db, std::slice::from_ref(record))?;
    readbacks.pop().ok_or_else(|| StorageError::WriteFailed {
        cf_name: cf::CF_AGENT_EVENTS.to_owned(),
        detail: "AGENT_EVENT_WRITE_FAILED: single-record write returned no readback".to_owned(),
    })
}

/// Validates, encodes, and enqueues a batch of event rows in one storage
/// batch. All-or-nothing: any invalid record refuses the whole batch before
/// anything is written.
///
/// # Errors
///
/// Returns [`StorageError::WriteFailed`] under the same conditions as
/// [`record_agent_event`].
pub(crate) fn record_agent_events(
    db: &Db,
    records: &[AgentEventRecord],
) -> StorageResult<Vec<AgentEventWriteReadback>> {
    let mut rows = Vec::with_capacity(records.len());
    let mut readbacks = Vec::with_capacity(records.len());
    for record in records {
        let encoded = validate_and_encode(record).inspect_err(|error| {
            tracing::error!(
                code = "AGENT_EVENT_WRITE_FAILED",
                kind = ?record.kind,
                session_id = ?record.session_id,
                spawn_id = ?record.spawn_id,
                reason_code = ?record.reason_code,
                detail = %error,
                "agent event refused before write"
            );
        })?;
        let seq = NEXT_AGENT_EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
        readbacks.push(AgentEventWriteReadback {
            ts_ns: record.ts_ns,
            seq,
            value_len_bytes: encoded.len(),
        });
        rows.push((agent_event_key(record.ts_ns, seq), encoded));
    }
    if rows.is_empty() {
        return Ok(readbacks);
    }
    db.put_batch(cf::CF_AGENT_EVENTS, rows).inspect_err(|error| {
        tracing::error!(
            code = "AGENT_EVENT_WRITE_FAILED",
            record_count = records.len(),
            first_kind = ?records.first().map(|record| record.kind),
            detail = %error,
            "agent event batch enqueue failed"
        );
    })?;
    for (record, readback) in records.iter().zip(&readbacks) {
        tracing::debug!(
            code = "AGENT_EVENT_RECORDED",
            kind = ?record.kind,
            ts_ns = readback.ts_ns,
            seq = readback.seq,
            session_id = ?record.session_id,
            spawn_id = ?record.spawn_id,
            value_len_bytes = readback.value_len_bytes,
            "readback=CF_AGENT_EVENTS edge=enqueued"
        );
    }
    Ok(readbacks)
}

/// [`record_agent_event`] plus an explicit `Db::flush()` so the row is
/// readable and crash-durable before this returns. Reserved for terminal
/// lifecycle events (exited, killed, spawn failure, session deleted).
///
/// # Errors
///
/// Returns [`StorageError::WriteFailed`] from the write or the flush.
pub(crate) fn record_agent_event_durable(
    db: &Db,
    record: &AgentEventRecord,
) -> StorageResult<AgentEventWriteReadback> {
    let readback = record_agent_event(db, record)?;
    db.flush().inspect_err(|error| {
        tracing::error!(
            code = "AGENT_EVENT_WRITE_FAILED",
            kind = ?record.kind,
            ts_ns = readback.ts_ns,
            seq = readback.seq,
            detail = %error,
            "agent event terminal flush failed"
        );
    })?;
    Ok(readback)
}

fn validate_and_encode(record: &AgentEventRecord) -> StorageResult<Vec<u8>> {
    record
        .validate()
        .map_err(|detail| StorageError::WriteFailed {
            cf_name: cf::CF_AGENT_EVENTS.to_owned(),
            detail,
        })?;
    let encoded = encode_json(record)?;
    if encoded.len() > MAX_AGENT_EVENT_VALUE_BYTES {
        return Err(StorageError::WriteFailed {
            cf_name: cf::CF_AGENT_EVENTS.to_owned(),
            detail: format!(
                "AGENT_EVENT_INVALID: encoded row is {} bytes, cap is {MAX_AGENT_EVENT_VALUE_BYTES}; journal rows are bounded metadata, never content",
                encoded.len()
            ),
        });
    }
    Ok(encoded)
}

/// Maps a registry `agent_kind` onto the OTel `gen_ai.provider.name`
/// well-known values. Unknown kinds stay unattributed rather than guessed.
pub(crate) fn provider_for_agent_kind(agent_kind: &str) -> Option<String> {
    match agent_kind {
        "claude" => Some("anthropic".to_owned()),
        "codex" => Some("openai".to_owned()),
        _ => None,
    }
}

/// Maps a journal write failure into a tool error that states whether the
/// primary operation already committed before the journal refused.
pub(crate) fn agent_event_tool_error(
    tool: &'static str,
    error: &StorageError,
    operation_committed: bool,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "{tool} could not journal its agent event to CF_AGENT_EVENTS: {error}{}",
            if operation_committed {
                " (the underlying operation already committed; storage needs attention before its effects are auditable)"
            } else {
                ""
            }
        ),
        Some(json!({
            "code": error.code(),
            "reason": "agent_event_journal_write_failed",
            "tool": tool,
            "operation_committed": operation_committed,
        })),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use synapse_core::{AgentEventKind, GenAiOperationName};
    use synapse_storage::decode_json;

    use super::*;

    fn open_temp_db() -> (tempfile::TempDir, Db) {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = Db::open(&temp.path().join("db"), synapse_core::SCHEMA_VERSION)
            .expect("temp DB must open");
        (temp, db)
    }

    fn event(session_id: &str, kind: AgentEventKind) -> AgentEventRecord {
        let mut record = AgentEventRecord::new(unix_time_ns_now(), kind);
        record.session_id = Some(session_id.to_owned());
        record
    }

    #[test]
    fn batched_write_lands_physical_rows_after_flush() {
        let (_temp, db) = open_temp_db();
        let before = db
            .scan_cf(cf::CF_AGENT_EVENTS)
            .expect("scan before must work");
        assert!(before.is_empty(), "fresh CF must start empty");

        let mut record = event("journal-test-session", AgentEventKind::MessageSent);
        record.attributes.operation_name = Some(GenAiOperationName::InvokeAgent);
        let readback = record_agent_event(&db, &record).expect("write must enqueue");
        db.flush().expect("flush must succeed");

        let rows = db.scan_cf(cf::CF_AGENT_EVENTS).expect("scan after");
        assert_eq!(rows.len(), 1, "exactly the written row must exist");
        let (key, value) = &rows[0];
        let (ts_ns, seq) =
            synapse_storage::agent_events::decode_agent_event_key(key).expect("key must decode");
        assert_eq!(ts_ns, readback.ts_ns);
        assert_eq!(seq, readback.seq);
        let decoded: AgentEventRecord = decode_json(value).expect("row must decode");
        assert_eq!(decoded, record);
        let raw: Value = serde_json::from_slice(value).expect("row must be JSON");
        assert_eq!(
            raw["attributes"]["gen_ai.operation.name"], "invoke_agent",
            "OTel attribute names must be stored verbatim: {raw}"
        );
    }

    #[test]
    fn durable_write_is_readable_without_extra_flush() {
        let (_temp, db) = open_temp_db();
        let mut record = event("durable-session", AgentEventKind::Exited);
        record.reason_code = Some("test_teardown".to_owned());
        record.end_state = Some(synapse_core::AgentEndState::Indeterminate);
        record_agent_event_durable(&db, &record).expect("durable write");

        let rows = db.scan_cf(cf::CF_AGENT_EVENTS).expect("scan");
        assert_eq!(rows.len(), 1, "durable row must be readable immediately");
    }

    #[test]
    fn invalid_record_is_refused_and_nothing_is_written() {
        let (_temp, db) = open_temp_db();
        let anonymous = AgentEventRecord::new(unix_time_ns_now(), AgentEventKind::Exited);
        let error = record_agent_event(&db, &anonymous).expect_err("anonymous must refuse");
        assert!(
            error.to_string().contains("AGENT_EVENT_INVALID"),
            "structured detail expected: {error}"
        );
        db.flush().expect("flush");
        let rows = db.scan_cf(cf::CF_AGENT_EVENTS).expect("scan");
        assert!(rows.is_empty(), "refused write must leave no rows");
    }

    #[test]
    fn oversized_payload_is_refused_with_byte_counts() {
        let (_temp, db) = open_temp_db();
        let mut record = event("oversize-session", AgentEventKind::MessageSent);
        record.payload = serde_json::json!({
            "blob": "x".repeat(MAX_AGENT_EVENT_VALUE_BYTES)
        });
        let error = record_agent_event(&db, &record).expect_err("oversize must refuse");
        assert!(error.to_string().contains("cap is"), "{error}");
        db.flush().expect("flush");
        assert!(
            db.scan_cf(cf::CF_AGENT_EVENTS).expect("scan").is_empty(),
            "refused oversize write must leave no rows"
        );
    }

    #[test]
    fn batch_with_one_invalid_record_writes_nothing() {
        let (_temp, db) = open_temp_db();
        let good = event("batch-session", AgentEventKind::MessageReceived);
        let anonymous = AgentEventRecord::new(unix_time_ns_now(), AgentEventKind::MessageReceived);
        let error =
            record_agent_events(&db, &[good, anonymous]).expect_err("mixed batch must refuse");
        assert!(error.to_string().contains("AGENT_EVENT_INVALID"), "{error}");
        db.flush().expect("flush");
        assert!(
            db.scan_cf(cf::CF_AGENT_EVENTS).expect("scan").is_empty(),
            "all-or-nothing: no row from a refused batch"
        );
    }

    #[test]
    fn same_tick_events_keep_distinct_ordered_keys() {
        let (_temp, db) = open_temp_db();
        let mut first = event("tick-session", AgentEventKind::LeaseAcquired);
        first.ts_ns = 42;
        let mut second = event("tick-session", AgentEventKind::LeaseReleased);
        second.ts_ns = 42;
        let readbacks =
            record_agent_events(&db, &[first, second]).expect("same-tick batch must write");
        db.flush().expect("flush");
        let rows = db.scan_cf(cf::CF_AGENT_EVENTS).expect("scan");
        assert_eq!(rows.len(), 2, "both same-tick rows must persist");
        assert!(
            readbacks[0].seq < readbacks[1].seq,
            "sequence must strictly increase within a tick: {readbacks:?}"
        );
        assert!(rows[0].0 < rows[1].0, "keys must iterate in seq order");
    }
}
