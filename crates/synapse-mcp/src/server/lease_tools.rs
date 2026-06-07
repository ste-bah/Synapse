//! MCP tools for the multi-agent input lease (epic #719, issue #733).
//!
//! The lease is the coordination primitive that lets many agents share the
//! single real foreground/cursor without interleaving: an agent acquires it
//! before a leased-foreground action and releases it after. Background tiers
//! (CDP/UIA/PostMessage) never touch it. These tools expose the lease over MCP
//! so an agent (or operator) can explicitly acquire/release/inspect it, and the
//! lease state is also surfaced under `health.subsystems.action`.
//!
//! The lease is keyed by `Mcp-Session-Id`, so every lease tool requires a
//! session. In the shared-daemon HTTP deployment each agent terminal has its
//! own session id; a missing session id is a fail-loud `TOOL_PARAMS_INVALID`.

use super::{
    ErrorData, Json, Parameters, SynapseService, empty_input_schema, mcp_error, tool, tool_router,
};
use rmcp::{RoleServer, model::ErrorCode, service::RequestContext};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use synapse_action::{LeaseOutcome, LeaseStatus, lease};
use synapse_core::error_codes;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ControlLeaseAcquireParams {
    /// Lease lifetime in milliseconds. Clamped to [100, 30000]. The lease is
    /// renewed on every leased action and on a repeat acquire by the holder, so
    /// a short TTL is the safety floor against a crashed holder, not a hard cap
    /// on how long real work can take.
    #[serde(default = "default_lease_ttl_ms")]
    #[schemars(default = "default_lease_ttl_ms", range(min = 100, max = 30000))]
    pub ttl_ms: u64,
}

const fn default_lease_ttl_ms() -> u64 {
    synapse_action::DEFAULT_LEASE_TTL_MS
}

/// Flattened lease snapshot returned by every lease tool. `LeaseStatus` lives in
/// `synapse-action` (no `schemars` dep there), so its fields are flattened here
/// rather than embedded, keeping the action crate schema-free.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ControlLeaseResponse {
    /// One of: `acquired`, `renewed`, `released`, `status`.
    pub outcome: String,
    /// Whether the lease is currently held by anyone.
    pub held: bool,
    /// The session id of the current holder, if any.
    pub owner_session_id: Option<String>,
    /// The session id that issued this tool call.
    pub this_session_id: String,
    /// Whether the calling session is the current holder.
    pub is_owner: bool,
    pub acquired_at_ms_ago: Option<u64>,
    pub renewed_at_ms_ago: Option<u64>,
    pub ttl_ms: Option<u64>,
    pub expires_in_ms: Option<u64>,
}

impl ControlLeaseResponse {
    fn from_status(outcome: &str, this_session_id: String, status: &LeaseStatus) -> Self {
        let is_owner = status.owner_session_id.as_deref() == Some(this_session_id.as_str());
        Self {
            outcome: outcome.to_owned(),
            held: status.held,
            owner_session_id: status.owner_session_id.clone(),
            this_session_id,
            is_owner,
            acquired_at_ms_ago: status.acquired_at_ms_ago,
            renewed_at_ms_ago: status.renewed_at_ms_ago,
            ttl_ms: status.ttl_ms,
            expires_in_ms: status.expires_in_ms,
        }
    }
}

#[tool_router(router = lease_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Acquire (or renew) the process-global input lease for this MCP session. The lease serializes real foreground/cursor/keyboard/clipboard actions across agents; background tiers (CDP/UIA/PostMessage) never need it. Refuse-not-block: if another live session holds it, returns ACTION_FOREGROUND_LEASE_BUSY with the current holder and a retry hint instead of waiting."
    )]
    pub async fn control_lease_acquire(
        &self,
        params: Parameters<ControlLeaseAcquireParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ControlLeaseResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "control_lease_acquire",
            "tool.invocation kind=control_lease_acquire"
        );
        let session_id = require_lease_session_id(&request_context)?;
        acquire_lease_for_session(&session_id, params.0.ttl_ms).map(Json)
    }

    #[tool(
        description = "Release the input lease held by this MCP session. Errors with ACTION_FOREGROUND_LEASE_NOT_HELD if this session is not the current holder.",
        input_schema = empty_input_schema()
    )]
    pub async fn control_lease_release(
        &self,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ControlLeaseResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "control_lease_release",
            "tool.invocation kind=control_lease_release"
        );
        let session_id = require_lease_session_id(&request_context)?;
        release_lease_for_session(&session_id).map(Json)
    }

    #[tool(
        description = "Read the current input lease state (holder, age, TTL, expiry). Never blocks; safe to poll. Reports whether the calling session is the holder.",
        input_schema = empty_input_schema()
    )]
    pub async fn control_lease_status(
        &self,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ControlLeaseResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "control_lease_status",
            "tool.invocation kind=control_lease_status"
        );
        let session_id = require_lease_session_id(&request_context)?;
        Ok(Json(lease_status_for_session(&session_id)))
    }
}

/// Acquire/renew the lease for `session_id`. Contended → `ACTION_FOREGROUND_LEASE_BUSY`.
/// Split out from the `#[tool]` method so the full outcome logic is unit-testable
/// without constructing an MCP `RequestContext`.
fn acquire_lease_for_session(
    session_id: &str,
    ttl_ms: u64,
) -> Result<ControlLeaseResponse, ErrorData> {
    let ttl = lease::ttl_from_ms(ttl_ms);
    match lease::try_acquire(session_id, ttl) {
        LeaseOutcome::Acquired(status) => {
            tracing::info!(
                code = "INPUT_LEASE_ACQUIRED",
                session_id = %session_id,
                ttl_ms = status.ttl_ms,
                "readback=input_lease outcome=acquired"
            );
            Ok(ControlLeaseResponse::from_status(
                "acquired",
                session_id.to_owned(),
                &status,
            ))
        }
        LeaseOutcome::Renewed(status) => {
            tracing::info!(
                code = "INPUT_LEASE_RENEWED",
                session_id = %session_id,
                ttl_ms = status.ttl_ms,
                "readback=input_lease outcome=renewed"
            );
            Ok(ControlLeaseResponse::from_status(
                "renewed",
                session_id.to_owned(),
                &status,
            ))
        }
        LeaseOutcome::Busy {
            holder,
            retry_after_ms,
        } => {
            tracing::warn!(
                code = error_codes::ACTION_FOREGROUND_LEASE_BUSY,
                session_id = %session_id,
                holder = ?holder.owner_session_id,
                retry_after_ms,
                "readback=input_lease outcome=busy"
            );
            Err(lease_busy_error(session_id, &holder, retry_after_ms))
        }
    }
}

fn release_lease_for_session(session_id: &str) -> Result<ControlLeaseResponse, ErrorData> {
    match lease::release(session_id) {
        Ok(status) => {
            tracing::info!(
                code = "INPUT_LEASE_RELEASED",
                session_id = %session_id,
                "readback=input_lease outcome=released"
            );
            Ok(ControlLeaseResponse::from_status(
                "released",
                session_id.to_owned(),
                &status,
            ))
        }
        Err(error) => Err(lease_not_held_error(session_id, &error)),
    }
}

fn lease_status_for_session(session_id: &str) -> ControlLeaseResponse {
    let status = lease::status();
    ControlLeaseResponse::from_status("status", session_id.to_owned(), &status)
}

/// Resolves the calling session id, failing loud when absent. The lease is
/// per-session, so an unidentified caller cannot meaningfully own it.
fn require_lease_session_id(
    request_context: &RequestContext<RoleServer>,
) -> Result<String, ErrorData> {
    super::context::mcp_session_id_from_request_context(request_context)?.ok_or_else(|| {
        mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "input lease tools require an MCP session id (run the daemon in HTTP mode so each agent has its own Mcp-Session-Id)",
        )
    })
}

fn lease_busy_error(session_id: &str, holder: &LeaseStatus, retry_after_ms: u64) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "input lease is held by session {:?}; retry after {retry_after_ms}ms",
            holder.owner_session_id
        ),
        Some(json!({
            "code": error_codes::ACTION_FOREGROUND_LEASE_BUSY,
            "requesting_session_id": session_id,
            "holder_session_id": holder.owner_session_id,
            "retry_after_ms": retry_after_ms,
            "holder": holder,
        })),
    )
}

fn lease_not_held_error(session_id: &str, error: &synapse_action::LeaseError) -> ErrorData {
    let holder = match error {
        synapse_action::LeaseError::NotHeld { holder, .. } => holder.clone(),
    };
    ErrorData::new(
        ErrorCode(-32099),
        error.to_string(),
        Some(json!({
            "code": error.code(),
            "requesting_session_id": session_id,
            "holder_session_id": holder,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::{acquire_lease_for_session, lease_status_for_session, release_lease_for_session};
    use std::sync::{Mutex, MutexGuard, PoisonError};
    use synapse_action::lease;
    use synapse_core::error_codes;

    // The lease is process-global, so these tests cannot run concurrently with
    // each other. Serialize them on a module-local mutex and reset the lease
    // under the guard so no test observes another's holder.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        let guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let _prior = lease::force_clear("lease_tools_test_reset");
        guard
    }

    fn reset() {
        let _prior = lease::force_clear("lease_tools_test_reset");
    }

    fn error_code(error: &rmcp::ErrorData) -> Option<String> {
        error
            .data
            .as_ref()
            .and_then(|data| data.get("code"))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    }

    #[test]
    fn acquire_then_status_then_release_round_trip() -> anyhow::Result<()> {
        let _serial = serial();
        let session = "fsv-tool-acquire";
        let acquired = acquire_lease_for_session(session, 5_000)
            .map_err(|error| anyhow::anyhow!("acquire failed: {error:?}"))?;
        assert_eq!(acquired.outcome, "acquired");
        assert!(acquired.held);
        assert!(acquired.is_owner);
        assert_eq!(acquired.owner_session_id.as_deref(), Some(session));

        // Source of truth: a separate status read reflects the holder.
        let status = lease_status_for_session(session);
        assert!(status.held);
        assert!(status.is_owner);
        assert_eq!(status.owner_session_id.as_deref(), Some(session));
        println!(
            "readback=input_lease step=after_acquire held={} owner={:?} expires_in_ms={:?}",
            status.held, status.owner_session_id, status.expires_in_ms
        );

        let released = release_lease_for_session(session)
            .map_err(|error| anyhow::anyhow!("release failed: {error:?}"))?;
        assert_eq!(released.outcome, "released");
        assert!(!released.held);

        let after = lease_status_for_session(session);
        assert!(!after.held);
        assert_eq!(after.owner_session_id, None);
        println!(
            "readback=input_lease step=after_release held={} owner={:?}",
            after.held, after.owner_session_id
        );
        reset();
        Ok(())
    }

    #[test]
    fn second_session_is_refused_busy_with_holder() -> anyhow::Result<()> {
        let _serial = serial();
        let owner = "fsv-tool-busy-owner";
        let contender = "fsv-tool-busy-contender";
        let _held = acquire_lease_for_session(owner, 5_000)
            .map_err(|error| anyhow::anyhow!("owner acquire failed: {error:?}"))?;

        let error = match acquire_lease_for_session(contender, 5_000) {
            Ok(response) => anyhow::bail!("contender unexpectedly acquired: {response:?}"),
            Err(error) => error,
        };
        assert_eq!(
            error_code(&error).as_deref(),
            Some(error_codes::ACTION_FOREGROUND_LEASE_BUSY)
        );
        let holder = error
            .data
            .as_ref()
            .and_then(|data| data.get("holder_session_id"))
            .and_then(serde_json::Value::as_str);
        assert_eq!(holder, Some(owner));

        // Source of truth: owner still holds; contender did not block.
        let status = lease_status_for_session(owner);
        assert_eq!(status.owner_session_id.as_deref(), Some(owner));
        println!(
            "readback=input_lease step=busy requesting={contender} holder={:?}",
            status.owner_session_id
        );
        reset();
        Ok(())
    }

    #[test]
    fn non_owner_release_errors_not_held() -> anyhow::Result<()> {
        let _serial = serial();
        let owner = "fsv-tool-nonowner-owner";
        let intruder = "fsv-tool-nonowner-intruder";
        let _held = acquire_lease_for_session(owner, 5_000)
            .map_err(|error| anyhow::anyhow!("owner acquire failed: {error:?}"))?;

        let error = match release_lease_for_session(intruder) {
            Ok(response) => anyhow::bail!("intruder unexpectedly released: {response:?}"),
            Err(error) => error,
        };
        assert_eq!(
            error_code(&error).as_deref(),
            Some(error_codes::ACTION_FOREGROUND_LEASE_NOT_HELD)
        );
        // Owner's lease survives the intruder's failed release.
        assert!(lease_status_for_session(owner).is_owner);
        reset();
        Ok(())
    }

    #[test]
    fn repeat_acquire_by_owner_renews() -> anyhow::Result<()> {
        let _serial = serial();
        let session = "fsv-tool-renew";
        let _first = acquire_lease_for_session(session, 5_000)
            .map_err(|error| anyhow::anyhow!("first acquire failed: {error:?}"))?;
        let second = acquire_lease_for_session(session, 5_000)
            .map_err(|error| anyhow::anyhow!("renew failed: {error:?}"))?;
        assert_eq!(second.outcome, "renewed");
        assert!(second.is_owner);
        reset();
        Ok(())
    }
}
