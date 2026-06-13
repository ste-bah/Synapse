use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
use std::cell::Cell;

use rmcp::ErrorData;
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use synapse_storage::cf;

use super::SynapseService;
use crate::m1::mcp_error;

const COMMAND_AUDIT_ROW_KIND: &str = "command_audit";
const COMMAND_AUDIT_SCHEMA_VERSION: u32 = 1;
const COMMAND_AUDIT_PAYLOAD_MAX_BYTES: usize = 8192;
const COMMAND_AUDIT_SNAPSHOT_SCAN_LIMIT: usize = 1000;
const COMMAND_AUDIT_SNAPSHOT_ROW_LIMIT: usize = 100;

static COMMAND_AUDIT_SEQ: AtomicU32 = AtomicU32::new(0);

#[cfg(test)]
thread_local! {
    static COMMAND_AUDIT_FORCE_FAIL: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Debug)]
pub(super) struct CommandAuditInput {
    pub tool: &'static str,
    pub verb: &'static str,
    pub channel: &'static str,
    pub actor_session_id: Option<String>,
    pub target_session_id: Option<String>,
    pub target: Option<Value>,
    pub payload: Value,
    pub before: Value,
    pub after: Value,
    pub outcome: &'static str,
    pub error: Option<CommandAuditError>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct CommandAuditError {
    pub code: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CommandAuditRowReadback {
    pub cf_name: &'static str,
    pub key_hex: String,
    pub value_len_bytes: u64,
    pub value_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CommandAuditSnapshot {
    pub source_of_truth: &'static str,
    pub scanned_rows: usize,
    pub row_count: usize,
    pub rows: Vec<CommandAuditSnapshotRow>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CommandAuditSnapshotRow {
    pub key_hex: String,
    pub audit_id: String,
    pub ts_ns: u64,
    pub phase: String,
    pub actor_session_id: Option<String>,
    pub tool: String,
    pub verb: String,
    pub channel: String,
    pub target_session_id: Option<String>,
    pub payload_sha256: Option<String>,
    pub payload_bounded: Option<Value>,
    pub payload_truncated: bool,
    pub target: Option<Value>,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub outcome: String,
    pub error_code: Option<String>,
    pub source_of_truth: Option<Value>,
}

impl CommandAuditInput {
    pub(super) fn mcp(
        tool: &'static str,
        verb: &'static str,
        actor_session_id: Option<String>,
        target_session_id: Option<String>,
        payload: Value,
        before: Value,
        after: Value,
        outcome: &'static str,
    ) -> Self {
        Self {
            tool,
            verb,
            channel: "mcp",
            actor_session_id,
            target_session_id,
            target: None,
            payload,
            before,
            after,
            outcome,
            error: None,
        }
    }

    pub(super) fn with_target(mut self, target: Value) -> Self {
        self.target = Some(target);
        self
    }

    pub(super) fn with_error(mut self, error: CommandAuditError) -> Self {
        self.error = Some(error);
        self
    }

    pub(super) fn with_channel(mut self, channel: &'static str) -> Self {
        self.channel = channel;
        self
    }
}

impl SynapseService {
    pub(super) fn command_audit_intent(
        &self,
        input: CommandAuditInput,
    ) -> Result<CommandAuditRowReadback, ErrorData> {
        self.write_command_audit_row("intent", input)
    }

    pub(super) fn command_audit_final(
        &self,
        input: CommandAuditInput,
    ) -> Result<CommandAuditRowReadback, ErrorData> {
        self.write_command_audit_row("final", input)
    }

    pub(crate) fn command_audit_snapshot(&self) -> Result<CommandAuditSnapshot, ErrorData> {
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_error| {
            command_audit_internal_error("reflex runtime lock poisoned while reading command audit")
        })?;
        let rows = runtime
            .storage_cf_tail_rows(cf::CF_ACTION_LOG, COMMAND_AUDIT_SNAPSHOT_SCAN_LIMIT)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        let scanned_rows = rows.len();
        let mut parsed = Vec::new();
        for (key, value) in rows.into_iter().rev() {
            let Ok(row) = synapse_storage::decode_json::<Value>(&value) else {
                continue;
            };
            if row.get("row_kind").and_then(Value::as_str) != Some(COMMAND_AUDIT_ROW_KIND) {
                continue;
            }
            parsed.push(command_audit_snapshot_row(&key, &row));
            if parsed.len() >= COMMAND_AUDIT_SNAPSHOT_ROW_LIMIT {
                break;
            }
        }
        Ok(CommandAuditSnapshot {
            source_of_truth: cf::CF_ACTION_LOG,
            scanned_rows,
            row_count: parsed.len(),
            rows: parsed,
        })
    }

    fn write_command_audit_row(
        &self,
        phase: &'static str,
        input: CommandAuditInput,
    ) -> Result<CommandAuditRowReadback, ErrorData> {
        #[cfg(test)]
        if COMMAND_AUDIT_FORCE_FAIL.with(Cell::get) {
            return Err(command_audit_internal_error(
                "command audit forced failure for test",
            ));
        }

        let (ts_ns, seq) = next_command_audit_key_parts();
        let key = command_audit_key(ts_ns, seq);
        let key_hex = hex_encode(&key);
        let tool = input.tool;
        let verb = input.verb;
        let channel = input.channel;
        let outcome = input.outcome;
        let mut audit_context = self.current_action_audit_context()?;
        let actor_session_id = input
            .actor_session_id
            .clone()
            .or_else(crate::http::current_mcp_session_id)
            .or_else(|| audit_context.session_id.clone());
        audit_context.session_id = actor_session_id.clone();
        let payload_record = bounded_redacted_payload(&input.payload);
        let value = json!({
            "schema_version": COMMAND_AUDIT_SCHEMA_VERSION,
            "row_kind": COMMAND_AUDIT_ROW_KIND,
            "audit_id": format!("{ts_ns:020}-{seq:010}"),
            "ts_ns": ts_ns,
            "seq": seq,
            "phase": phase,
            "actor": {
                "channel": channel,
                "tool": tool,
                "session_id": actor_session_id,
                "profile_id": audit_context.profile_id.clone(),
                "profile_version": audit_context.profile_version.clone(),
                "profile_schema_version": audit_context.profile_schema_version,
            },
            "audit_context": audit_context,
            "tool": tool,
            "verb": verb,
            "channel": channel,
            "target_session_id": input.target_session_id.clone(),
            "target": input.target.clone(),
            "payload_sha256": payload_record.sha256,
            "payload_bytes": payload_record.bytes,
            "payload_bounded": payload_record.value,
            "payload_truncated": payload_record.truncated,
            "payload_hash_scope": "redacted_payload",
            "redacted": payload_record.redacted,
            "redactions": payload_record.redactions,
            "before": input.before.clone(),
            "after": input.after.clone(),
            "outcome": outcome,
            "error_code": input.error.as_ref().and_then(|error| error.code.clone()),
            "error": input.error.clone(),
            "source_of_truth": {
                "cf_name": cf::CF_ACTION_LOG,
                "row_kind": COMMAND_AUDIT_ROW_KIND,
                "retention": "24h",
                "key_hex": key_hex,
            },
        });
        let encoded = synapse_storage::encode_json(&value).map_err(|error| {
            command_audit_internal_error(format!("command audit row encode failed: {error}"))
        })?;
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_error| {
            command_audit_internal_error("reflex runtime lock poisoned while writing command audit")
        })?;
        runtime
            .storage_put_action_log_rows(vec![(key.clone(), encoded.clone())])
            .map_err(|error| {
                command_audit_internal_error(format!("command audit write failed: {error}"))
            })?;
        let (readback_rows, _has_more) = runtime
            .storage_cf_rows_from(cf::CF_ACTION_LOG, &key, 1)
            .map_err(|error| {
            command_audit_internal_error(format!("command audit readback failed: {error}"))
        })?;
        let Some((read_key, read_value)) = readback_rows.first() else {
            return Err(command_audit_internal_error(format!(
                "command audit readback missing row key_hex={key_hex}"
            )));
        };
        if read_key != &key || read_value != &encoded {
            return Err(command_audit_internal_error(format!(
                "command audit readback mismatch key_hex={key_hex}"
            )));
        }
        let readback = CommandAuditRowReadback {
            cf_name: cf::CF_ACTION_LOG,
            key_hex,
            value_len_bytes: read_value.len() as u64,
            value_sha256: sha256_hex(read_value),
        };
        tracing::info!(
            code = "COMMAND_AUDIT_RECORDED",
            tool,
            verb,
            phase,
            outcome,
            key_hex = %readback.key_hex,
            "command audit row written and read back"
        );
        Ok(readback)
    }
}

pub(super) fn command_audit_error_from_error_data(error: &ErrorData) -> CommandAuditError {
    CommandAuditError {
        code: error_data_code(error).map(str::to_owned),
        message: error.message.to_string(),
        data: error.data.clone(),
    }
}

#[cfg(test)]
pub(crate) fn set_command_audit_force_fail_for_tests(enabled: bool) {
    COMMAND_AUDIT_FORCE_FAIL.with(|force_fail| force_fail.set(enabled));
}

#[derive(Debug)]
struct BoundedPayload {
    value: Value,
    sha256: String,
    bytes: usize,
    truncated: bool,
    redacted: bool,
    redactions: Vec<String>,
}

fn bounded_redacted_payload(payload: &Value) -> BoundedPayload {
    let mut redactions = Vec::new();
    let value = redact_value(payload, "$", &mut redactions);
    let encoded = serde_json::to_vec(&value).unwrap_or_else(|_error| b"null".to_vec());
    let bytes = encoded.len();
    let sha256 = sha256_hex(&encoded);
    let truncated = bytes > COMMAND_AUDIT_PAYLOAD_MAX_BYTES;
    let value = if truncated {
        let prefix =
            String::from_utf8_lossy(&encoded[..COMMAND_AUDIT_PAYLOAD_MAX_BYTES]).to_string();
        json!({
            "truncated_utf8_prefix": prefix,
            "omitted_bytes": bytes.saturating_sub(COMMAND_AUDIT_PAYLOAD_MAX_BYTES),
        })
    } else {
        value
    };
    BoundedPayload {
        value,
        sha256,
        bytes,
        truncated,
        redacted: !redactions.is_empty(),
        redactions,
    }
}

fn redact_value(value: &Value, path: &str, redactions: &mut Vec<String>) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                let child_path = format!("{path}.{key}");
                if sensitive_key(key) {
                    redactions.push(child_path);
                    out.insert(key.clone(), Value::String("[REDACTED]".to_owned()));
                } else {
                    out.insert(key.clone(), redact_value(value, &child_path, redactions));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| redact_value(item, &format!("{path}[{index}]"), redactions))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "token",
        "password",
        "secret",
        "api_key",
        "apikey",
        "authorization",
        "bearer",
        "cookie",
        "credential",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn command_audit_snapshot_row(key: &[u8], row: &Value) -> CommandAuditSnapshotRow {
    let actor = row.get("actor").and_then(Value::as_object);
    CommandAuditSnapshotRow {
        key_hex: hex_encode(key),
        audit_id: string_field(row, "audit_id"),
        ts_ns: row.get("ts_ns").and_then(Value::as_u64).unwrap_or_default(),
        phase: string_field(row, "phase"),
        actor_session_id: actor
            .and_then(|actor| actor.get("session_id"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        tool: string_field(row, "tool"),
        verb: string_field(row, "verb"),
        channel: string_field(row, "channel"),
        target_session_id: row
            .get("target_session_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        payload_sha256: row
            .get("payload_sha256")
            .and_then(Value::as_str)
            .map(str::to_owned),
        payload_bounded: row.get("payload_bounded").cloned(),
        payload_truncated: row
            .get("payload_truncated")
            .and_then(Value::as_bool)
            .unwrap_or_default(),
        target: row.get("target").cloned(),
        before: row.get("before").cloned(),
        after: row.get("after").cloned(),
        outcome: string_field(row, "outcome"),
        error_code: row
            .get("error_code")
            .and_then(Value::as_str)
            .map(str::to_owned),
        source_of_truth: row.get("source_of_truth").cloned(),
    }
}

fn string_field(row: &Value, field: &str) -> String {
    row.get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn command_audit_internal_error(message: impl ToString) -> ErrorData {
    mcp_error(
        synapse_core::error_codes::TOOL_INTERNAL_ERROR,
        message.to_string(),
    )
}

fn next_command_audit_key_parts() -> (u64, u32) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let ts_ns = u64::try_from(nanos).unwrap_or(u64::MAX);
    let seq = COMMAND_AUDIT_SEQ.fetch_add(1, Ordering::Relaxed);
    (ts_ns, seq)
}

fn command_audit_key(ts_ns: u64, seq: u32) -> Vec<u8> {
    let mut key = Vec::with_capacity(12);
    key.extend_from_slice(&ts_ns.to_be_bytes());
    key.extend_from_slice(&seq.to_be_bytes());
    key
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex_encode(digest.as_ref()))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

fn error_data_code(error: &ErrorData) -> Option<&str> {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_audit_redacts_sensitive_payload_fields() {
        let payload = json!({
            "token": "raw-token",
            "nested": {
                "api_key": "raw-key",
                "safe": "kept",
            },
            "items": [
                {"cookie": "raw-cookie"},
                {"name": "kept"}
            ]
        });
        let bounded = bounded_redacted_payload(&payload);
        assert!(bounded.redacted);
        assert!(bounded.redactions.contains(&"$.token".to_owned()));
        assert!(bounded.redactions.contains(&"$.nested.api_key".to_owned()));
        assert!(bounded.redactions.contains(&"$.items[0].cookie".to_owned()));
        assert_eq!(bounded.value["token"], "[REDACTED]");
        assert_eq!(bounded.value["nested"]["safe"], "kept");
        assert!(bounded.sha256.starts_with("sha256:"));
    }

    #[test]
    fn command_audit_bounds_large_payload() {
        let payload = json!({"body": "A".repeat(COMMAND_AUDIT_PAYLOAD_MAX_BYTES + 128)});
        let bounded = bounded_redacted_payload(&payload);
        assert!(bounded.truncated);
        assert!(bounded.bytes > COMMAND_AUDIT_PAYLOAD_MAX_BYTES);
        assert!(
            bounded
                .value
                .get("omitted_bytes")
                .and_then(Value::as_u64)
                .is_some_and(|omitted| omitted > 0)
        );
    }
}
