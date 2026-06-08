use std::{
    collections::BTreeSet,
    io,
    net::SocketAddr,
    process::ExitCode,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService,
    session::{SessionState, SessionStore, SessionStoreError, local::LocalSessionManager},
};
#[cfg(test)]
use synapse_action::ActionHandle;
use synapse_action::ActionStateSnapshot;
use synapse_core::Health;
use synapse_storage::{Db, cf};
use tokio::{net::TcpListener, sync::watch, task::JoinHandle, time};
use tokio_util::sync::CancellationToken;

use crate::{
    http::auth::{self, HttpAuth},
    http::session,
    http::sse::{self, SseState},
    m2::M2ServiceConfig,
    m3::M3ServiceConfig,
    m4::M4ServiceConfig,
    server::SynapseService,
};

type McpHttpService = StreamableHttpService<SynapseService, LocalSessionManager>;
const STALE_SESSION_INPUT_CLEANUP_INTERVAL: Duration = Duration::from_millis(250);
const M2_EMITTER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct HttpState {
    health_service: Arc<SynapseService>,
    session_manager: Arc<LocalSessionManager>,
    shutdown_cancel: CancellationToken,
    sse_state: SseState,
}

pub(super) async fn serve(
    bind: &str,
    allow_non_loopback: bool,
    m2_config: &M2ServiceConfig,
    m3_config: M3ServiceConfig,
    m4_config: M4ServiceConfig,
) -> anyhow::Result<ExitCode> {
    synapse_action::install_panic_hook();

    // Validate the bind address first — a pure argument check with no side
    // effects. Doing this before acquiring the single-instance lock means a
    // misconfigured non-loopback bind always fails with HTTP_BIND_NON_LOOPBACK_
    // REFUSED (exit 2), even when another daemon already holds the DB lock
    // (which would otherwise short-circuit to exit 3 and mask the real problem).
    let addr = bind
        .parse::<SocketAddr>()
        .with_context(|| format!("parse HTTP bind address {bind}"))?;
    if !addr.ip().is_loopback() && !allow_non_loopback {
        tracing::error!(
            code = synapse_core::error_codes::HTTP_BIND_NON_LOOPBACK_REFUSED,
            bind = %addr,
            "refusing non-loopback HTTP bind without --allow-non-loopback"
        );
        return Ok(ExitCode::from(2));
    }

    // Single-instance guard: at most one daemon may own a given RocksDB path.
    // Acquired before binding the port or opening storage so a duplicate launch
    // fails fast with a clear, holder-naming error instead of a cryptic RocksDB
    // LOCK failure surfacing later inside a tool call.
    let db_path = m3_config
        .db_path
        .clone()
        .unwrap_or_else(crate::m3::default_db_path);
    let _single_instance = match crate::single_instance::SingleInstanceGuard::acquire(&db_path) {
        Ok(guard) => {
            tracing::info!(
                code = "MCP_DAEMON_SINGLE_INSTANCE_ACQUIRED",
                lock_path = %guard.lock_path().display(),
                db_path = %db_path.display(),
                pid = std::process::id(),
                "daemon single-instance lock acquired"
            );
            guard
        }
        Err(crate::single_instance::SingleInstanceError::AlreadyRunning {
            lock_path,
            holder_pid,
        }) => {
            tracing::error!(
                code = "MCP_DAEMON_ALREADY_RUNNING",
                lock_path = %lock_path.display(),
                holder_pid = holder_pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string()),
                db_path = %db_path.display(),
                "refusing to start: another synapse-mcp daemon already owns this DB path"
            );
            return Ok(ExitCode::from(3));
        }
        Err(err @ crate::single_instance::SingleInstanceError::Io { .. }) => {
            return Err(anyhow::Error::new(err)).context("acquire daemon single-instance lock");
        }
    };

    let lifecycle_paths =
        crate::daemon_lifecycle::configure(crate::daemon_lifecycle::DaemonLifecycleConfig {
            mode: "http",
            bind_addr: Some(addr.to_string()),
            db_path: db_path.clone(),
        })
        .context("configure daemon lifecycle ledger")?;
    crate::daemon_lifecycle::install_panic_hook();
    tracing::info!(
        code = "MCP_DAEMON_LIFECYCLE_READY",
        run_current_path = %lifecycle_paths.run_current_path,
        tool_last_path = %lifecycle_paths.tool_last_path,
        tool_events_path = %lifecycle_paths.tool_events_path,
        exit_events_path = %lifecycle_paths.exit_events_path,
        "daemon lifecycle ledger ready"
    );

    if !addr.ip().is_loopback() {
        tracing::warn!(
            code = "MCP_HTTP_NON_LOOPBACK_BIND_ALLOWED",
            bind = %addr,
            "non-loopback HTTP bind allowed by explicit operator flag"
        );
    }
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind HTTP MCP transport to {addr}"))?;
    let local_addr = listener
        .local_addr()
        .context("read HTTP listener address")?;
    let shutdown_cancel = CancellationToken::new();
    let connection_closed_cancel = CancellationToken::new();
    let sse_state = SseState::with_max_subscriptions(m3_config.max_subscriptions);
    let service = http_service(
        shutdown_cancel.clone(),
        connection_closed_cancel.clone(),
        sse_state.clone(),
        m2_config,
        m3_config,
        m4_config,
    )
    .context("initialize shared HTTP service state")?;

    // Eager storage open: validate RocksDB at startup rather than lazily on the
    // first reflex tool call, so a lock/schema fault fails fast with a clear
    // error and the daemon refuses to start half-broken (instead of every tool
    // call failing later). The handle is cached and reused by the reflex
    // runtime, so there is no open-then-reopen race.
    {
        let m3_handle = service.m3_state_handle();
        let open_result = {
            let mut state = m3_handle.lock().map_err(|_poisoned| {
                anyhow::anyhow!("m3 service state lock poisoned during startup storage open")
            })?;
            state.ensure_storage()
        };
        if let Err(error) = open_result {
            let detail = error.to_string();
            if detail.to_lowercase().contains("lock") {
                tracing::error!(
                    code = "STORAGE_LOCK_CONTENDED",
                    db_path = %db_path.display(),
                    detail = %detail,
                    "refusing to start: RocksDB storage lock is held by another process; run `synapse-mcp doctor` to find and stop the holder, or point this daemon at a different --db path"
                );
            } else {
                tracing::error!(
                    code = "STORAGE_OPEN_FAILED",
                    db_path = %db_path.display(),
                    detail = %detail,
                    "refusing to start: storage open failed at daemon startup"
                );
            }
            crate::daemon_lifecycle::record_startup_exit(
                "startup_storage_open_failed",
                serde_json::json!({
                    "db_path": db_path.display().to_string(),
                    "detail": detail,
                }),
            )
            .context("record daemon lifecycle startup storage-open failure")?;
            return Ok(ExitCode::from(4));
        }
        tracing::info!(
            code = "MCP_DAEMON_STORAGE_OPENED",
            db_path = %db_path.display(),
            "daemon storage opened eagerly at startup"
        );
    }

    let _operator_hotkey_guard = crate::safety::install_operator_hotkey(service.m3_state_handle())
        .context("install operator panic hotkey")?;
    let m2_emitter_done = service.m2_emitter_done_receiver();
    let app = router(&shutdown_cancel, local_addr, sse_state, service)
        .context("build HTTP MCP router")?;

    tracing::info!(
        code = "MCP_HTTP_STARTED",
        bind = %local_addr,
        "starting streamable HTTP MCP transport"
    );

    let mut server_task = spawn_server(listener, app, shutdown_cancel.clone());
    let m2_done_after_server_stop = m2_emitter_done.clone();
    let m2_done_after_signal = m2_emitter_done;
    let code = tokio::select! {
        result = &mut server_task => {
            result.context("join HTTP MCP transport")?
                .context("serve HTTP MCP transport")?;
            if shutdown_cancel.is_cancelled() {
                connection_closed_cancel.cancel();
                wait_for_m2_emitter_done(m2_done_after_server_stop, "http_endpoint").await;
            }
            ExitCode::SUCCESS
        }
        signal = wait_for_shutdown_signal("http") => {
            signal?;
            tracing::info!(code = "MCP_SHUTDOWN_GRACEFUL", "HTTP shutdown signal received");
            shutdown_cancel.cancel();
            connection_closed_cancel.cancel();
            wait_for_server_stop(&mut server_task).await?;
            wait_for_m2_emitter_done(m2_done_after_signal, "signal").await;
            ExitCode::SUCCESS
        }
    };
    crate::daemon_lifecycle::record_graceful_exit("http_service_completed")
        .context("record daemon lifecycle graceful HTTP service completion")?;
    Ok(code)
}

fn router(
    shutdown_cancel: &CancellationToken,
    bind_addr: SocketAddr,
    sse_state: SseState,
    service: SynapseService,
) -> anyhow::Result<Router> {
    let auth = Arc::new(HttpAuth::load(bind_addr).context("load HTTP bearer token")?);
    tracing::info!(
        code = "MCP_HTTP_AUTH_CONFIGURED",
        source = auth.source_label(),
        "HTTP bearer token configured"
    );
    let health_service = Arc::new(service.clone());
    let session_registry = service.session_registry_handle();
    let terminated_sessions = service.terminated_sessions_handle();
    let session_lifecycle = service
        .session_lifecycle_state()
        .map_err(|error| anyhow::anyhow!("initialize session lifecycle state: {error:?}"))?;
    let (mcp_service, session_manager) = streamable_service(shutdown_cancel, service)
        .context("initialize HTTP MCP session state")?;
    let session_request = session::SessionCleanupState::request_state(
        Arc::clone(&session_registry),
        terminated_sessions,
    );
    let session_cleanup =
        session::SessionCleanupState::new(Arc::clone(&session_manager), session_lifecycle.clone());
    let _stale_cleanup_task = spawn_stale_session_input_cleanup(
        Arc::clone(&session_manager),
        session_lifecycle,
        shutdown_cancel.child_token(),
    );
    let state = HttpState {
        health_service,
        session_manager,
        shutdown_cancel: shutdown_cancel.clone(),
        sse_state,
    };
    Ok(Router::new()
        .route("/health", get(health))
        .route("/shutdown", post(shutdown))
        .route("/events", get(events).post(publish_event))
        .route("/events/stats", get(event_stats))
        .nest_service("/mcp", mcp_service)
        .layer(middleware::from_fn_with_state(
            session_request,
            session::require_mcp_session,
        ))
        .layer(middleware::from_fn_with_state(
            session_cleanup,
            session::release_held_inputs_on_delete,
        ))
        .layer(middleware::from_fn_with_state(
            auth,
            auth::require_http_security,
        ))
        .with_state(state))
}

fn streamable_service(
    shutdown_cancel: &CancellationToken,
    service: SynapseService,
) -> anyhow::Result<(McpHttpService, Arc<LocalSessionManager>)> {
    let session_config = session::load_session_config().context("load HTTP session config")?;
    {
        let session_registry = service.session_registry_handle();
        let mut registry = session_registry.lock().map_err(|_poisoned| {
            anyhow::anyhow!("session registry lock poisoned during HTTP session setup")
        })?;
        registry.set_stale_after(session_config.keep_alive);
    }
    let session_store = Arc::new(SynapseMcpSessionStore::new(
        session_store_db(&service)?,
        session_config.keep_alive,
        service.session_registry_handle(),
    ));
    let mut config = StreamableHttpServerConfig::default()
        .with_cancellation_token(shutdown_cancel.child_token());
    config.session_store = Some(session_store);
    let mut session_manager = LocalSessionManager::default();
    session_manager.session_config = session_config;
    let session_manager = Arc::new(session_manager);
    let service = StreamableHttpService::new(
        move || Ok(service.clone()),
        Arc::clone(&session_manager),
        config,
    );
    Ok((service, session_manager))
}

fn spawn_stale_session_input_cleanup(
    session_manager: Arc<LocalSessionManager>,
    session_lifecycle: crate::server::session_lifecycle::SessionLifecycleState,
    shutdown_cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = time::interval(STALE_SESSION_INPUT_CLEANUP_INTERVAL);
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = shutdown_cancel.cancelled() => {
                    tracing::debug!(
                        code = "MCP_HTTP_SESSION_INPUT_STALE_CLEANUP_STOPPED",
                        "stopping stale HTTP session held-input cleanup"
                    );
                    break;
                }
                _ = interval.tick() => {
                    cleanup_stale_session_resources_once(
                        &session_lifecycle,
                        &session_manager,
                    ).await;
                }
            }
        }
    })
}

async fn cleanup_stale_session_resources_once(
    session_lifecycle: &crate::server::session_lifecycle::SessionLifecycleState,
    session_manager: &LocalSessionManager,
) {
    let active_sessions = active_http_session_ids(session_manager).await;
    session_lifecycle.cleanup_expired_lease_inputs_once().await;
    let stale_sessions = session_lifecycle.stale_session_candidates(&active_sessions);
    for session_id in stale_sessions {
        match session_lifecycle
            .teardown_session(&session_id, "http_stale")
            .await
        {
            Ok(report) => {
                tracing::info!(
                    code = "MCP_HTTP_SESSION_STALE_LIFECYCLE_CLEANUP",
                    session_id = %session_id,
                    active_session_count = active_sessions.len(),
                    report = ?report,
                    "readback=session_lifecycle edge=http_session_gone after_cleanup"
                );
            }
            Err(error) => {
                tracing::error!(
                    code = synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    session_id = %session_id,
                    active_session_count = active_sessions.len(),
                    detail = %error.message,
                    data = ?error.data,
                    "HTTP MCP stale-session lifecycle cleanup failed"
                );
            }
        }
    }
}

#[cfg(test)]
async fn cleanup_stale_session_inputs_once(
    action_handle: &ActionHandle,
    session_manager: &LocalSessionManager,
    cdp_target_owners: &crate::server::SharedCdpTargetOwners,
) {
    let active_sessions = active_http_session_ids(session_manager).await;
    cleanup_expired_lease_inputs_once(action_handle).await;
    cleanup_stale_session_cdp_targets_once(cdp_target_owners, &active_sessions).await;

    let snapshot = match action_handle.session_inputs_snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::error!(
                code = error.code(),
                detail = %error.detail(),
                "HTTP MCP stale-session cleanup could not read held-input ownership"
            );
            return;
        }
    };
    for session in snapshot.sessions {
        if active_sessions.contains(&session.session_id) {
            continue;
        }
        release_stale_session_inputs_and_lease(action_handle, &session.session_id).await;
    }

    cleanup_stale_session_lease_once(action_handle, &active_sessions).await;
}

#[cfg(test)]
async fn cleanup_stale_session_cdp_targets_once(
    cdp_target_owners: &crate::server::SharedCdpTargetOwners,
    active_sessions: &BTreeSet<String>,
) {
    let stale_sessions = match cdp_target_owners.lock() {
        Ok(owners) => owners
            .values()
            .filter_map(|owner| {
                (!active_sessions.contains(&owner.session_id)).then(|| owner.session_id.clone())
            })
            .collect::<BTreeSet<_>>(),
        Err(_error) => {
            tracing::error!(
                code = synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                "HTTP MCP stale-session cleanup could not lock CDP target ownership registry"
            );
            return;
        }
    };
    for session_id in stale_sessions {
        let (owned_before, target_ids) = match cdp_target_owners.lock() {
            Ok(mut owners) => {
                let target_ids = owners
                    .iter()
                    .filter_map(|(target_id, owner)| {
                        (owner.session_id == session_id).then(|| target_id.clone())
                    })
                    .collect::<Vec<_>>();
                for target_id in &target_ids {
                    owners.remove(target_id);
                }
                (target_ids.len(), target_ids)
            }
            Err(_error) => {
                tracing::error!(
                    code = synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    "HTTP MCP test stale-session cleanup could not lock CDP target ownership registry"
                );
                continue;
            }
        };
        tracing::info!(
            code = "MCP_HTTP_SESSION_CDP_TARGET_STALE_CLEANUP",
            session_id = %session_id,
            active_session_count = active_sessions.len(),
            cdp_cleanup_reason = "http_stale",
            cdp_owned_before = owned_before,
            cdp_closed = 0,
            cdp_failed = 0,
            cdp_target_ids = ?target_ids,
            "readback=cdp_target_ownership edge=http_session_gone after_cleanup"
        );
    }
}

async fn active_http_session_ids(session_manager: &LocalSessionManager) -> BTreeSet<String> {
    session_manager
        .sessions
        .read()
        .await
        .keys()
        .map(|session_id| session_id.as_ref().to_owned())
        .collect()
}

#[cfg(test)]
async fn cleanup_expired_lease_inputs_once(action_handle: &ActionHandle) {
    let _lease_status_readback = synapse_action::lease::status();
    let pending = synapse_action::lease::expired_cleanup_snapshot();
    for expired in pending {
        let Some(session_id) = expired.owner_session_id.clone() else {
            continue;
        };
        release_expired_session_inputs_and_lease(action_handle, &session_id, &expired).await;
    }
}

#[cfg(test)]
async fn cleanup_stale_session_lease_once(
    action_handle: &ActionHandle,
    active_sessions: &BTreeSet<String>,
) {
    let status = synapse_action::lease::status();
    let Some(owner_session_id) = status.owner_session_id.clone() else {
        return;
    };
    if owner_session_id == synapse_action::OPERATOR_LEASE_OWNER_SESSION_ID {
        return;
    }
    if active_sessions.contains(&owner_session_id) {
        return;
    }
    let before_ownership = action_handle.session_inputs_snapshot();
    let result = action_handle
        .release_session_inputs_and_lease(&owner_session_id)
        .await;
    let after_ownership = action_handle.session_inputs_snapshot();
    let after_lease = synapse_action::lease::status();
    match result {
        Ok(summary) => {
            tracing::info!(
                code = "MCP_HTTP_SESSION_LEASE_STALE_CLEANUP",
                session_id = %owner_session_id,
                input_lease_released = summary.lease_released,
                expired_lease_cleanup_completed = summary.expired_lease_cleanup_completed,
                before_lease = ?status,
                after_lease = ?after_lease,
                before_ownership = ?before_ownership,
                after_ownership = ?after_ownership,
                active_session_count = active_sessions.len(),
                "readback=input_lease edge=http_session_gone after_cleanup"
            );
        }
        Err(error) => {
            tracing::error!(
                code = error.code(),
                session_id = %owner_session_id,
                detail = %error.detail(),
                before_lease = ?status,
                after_lease = ?after_lease,
                before_ownership = ?before_ownership,
                after_ownership = ?after_ownership,
                active_session_count = active_sessions.len(),
                "HTTP MCP stale-session lease cleanup failed"
            );
        }
    }
}

#[cfg(test)]
async fn release_stale_session_inputs_and_lease(action_handle: &ActionHandle, session_id: &str) {
    let before = action_handle.session_inputs_snapshot();
    let before_lease = synapse_action::lease::status();
    let result = action_handle
        .release_session_inputs_and_lease(session_id)
        .await;
    let after = action_handle.session_inputs_snapshot();
    let after_lease = synapse_action::lease::status();
    match result {
        Ok(summary) => {
            tracing::info!(
                code = "MCP_HTTP_SESSION_INPUT_STALE_CLEANUP",
                session_id,
                released_keys = summary.input_summary.released_keys,
                released_buttons = summary.input_summary.released_buttons,
                neutralized_pads = summary.input_summary.neutralized_pads,
                retained_shared_inputs = summary.input_summary.retained_shared_inputs,
                input_lease_released = summary.lease_released,
                expired_lease_cleanup_completed = summary.expired_lease_cleanup_completed,
                before = ?before,
                after = ?after,
                before_lease = ?before_lease,
                after_lease = ?after_lease,
                "readback=session_input_ownership edge=http_session_gone after_cleanup"
            );
        }
        Err(error) => {
            tracing::error!(
                code = error.code(),
                session_id,
                detail = %error.detail(),
                before = ?before,
                after = ?after,
                before_lease = ?before_lease,
                after_lease = ?after_lease,
                "HTTP MCP stale-session cleanup failed while releasing owned inputs"
            );
        }
    }
}

#[cfg(test)]
async fn release_expired_session_inputs_and_lease(
    action_handle: &ActionHandle,
    session_id: &str,
    expired: &synapse_action::LeaseStatus,
) {
    let before = action_handle.session_inputs_snapshot();
    let before_lease = synapse_action::lease::status();
    let result = action_handle
        .release_session_inputs_and_lease(session_id)
        .await;
    let after = action_handle.session_inputs_snapshot();
    let after_lease = synapse_action::lease::status();
    match result {
        Ok(summary) => {
            tracing::warn!(
                code = "MCP_HTTP_SESSION_LEASE_EXPIRED_INPUT_CLEANUP",
                session_id,
                released_keys = summary.input_summary.released_keys,
                released_buttons = summary.input_summary.released_buttons,
                neutralized_pads = summary.input_summary.neutralized_pads,
                retained_shared_inputs = summary.input_summary.retained_shared_inputs,
                input_lease_released = summary.lease_released,
                expired_lease_cleanup_completed = summary.expired_lease_cleanup_completed,
                expired = ?expired,
                before = ?before,
                after = ?after,
                before_lease = ?before_lease,
                after_lease = ?after_lease,
                "readback=session_input_ownership edge=input_lease_expired after_cleanup"
            );
        }
        Err(error) => {
            tracing::error!(
                code = error.code(),
                session_id,
                detail = %error.detail(),
                expired = ?expired,
                before = ?before,
                after = ?after,
                before_lease = ?before_lease,
                after_lease = ?after_lease,
                "HTTP MCP expired-lease cleanup failed while releasing owned inputs"
            );
        }
    }
}

fn session_store_db(service: &SynapseService) -> anyhow::Result<Arc<Db>> {
    let m3_handle = service.m3_state_handle();
    let mut state = m3_handle.lock().map_err(|_poisoned| {
        anyhow::anyhow!("m3 service state lock poisoned during session-store setup")
    })?;
    state
        .ensure_storage()
        .context("open storage for HTTP MCP session store")
}

#[derive(Clone)]
struct SynapseMcpSessionStore {
    db: Arc<Db>,
    ttl: Option<Duration>,
    session_registry: crate::server::session_registry::SharedSessionRegistry,
}

impl SynapseMcpSessionStore {
    fn new(
        db: Arc<Db>,
        ttl: Option<Duration>,
        session_registry: crate::server::session_registry::SharedSessionRegistry,
    ) -> Self {
        Self {
            db,
            ttl,
            session_registry,
        }
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct PersistedMcpSessionState {
    stored_at_unix_ms: u64,
    state: SessionState,
}

#[async_trait::async_trait]
impl SessionStore for SynapseMcpSessionStore {
    async fn load(&self, session_id: &str) -> Result<Option<SessionState>, SessionStoreError> {
        let key = mcp_session_store_key(session_id);
        let rows = self
            .db
            .scan_cf_prefix(cf::CF_KV, &key)
            .map_err(session_store_error)?;
        let Some((_key, value)) = rows.into_iter().find(|(row_key, _value)| row_key == &key) else {
            return Ok(None);
        };
        let now_ms = unix_time_ms()?;
        let persisted = match synapse_storage::decode_json::<PersistedMcpSessionState>(&value) {
            Ok(persisted) => persisted,
            Err(wrapper_error) => {
                if synapse_storage::decode_json::<SessionState>(&value).is_ok() {
                    self.db
                        .delete_batch(cf::CF_KV, [key])
                        .map_err(session_store_error)?;
                    tracing::warn!(
                        code = "MCP_HTTP_SESSION_STORE_LEGACY_STALE_DELETE",
                        session_id,
                        detail = %wrapper_error,
                        "deleted legacy MCP HTTP session state without persistent TTL metadata"
                    );
                    return Ok(None);
                }
                tracing::error!(
                    code = "MCP_HTTP_SESSION_STORE_DECODE_FAILED",
                    session_id,
                    detail = %wrapper_error,
                    "failed to decode persisted MCP HTTP session state"
                );
                return Err(session_store_error(wrapper_error));
            }
        };
        if session_store_expired(persisted.stored_at_unix_ms, now_ms, self.ttl) {
            self.db
                .delete_batch(cf::CF_KV, [key])
                .map_err(session_store_error)?;
            tracing::warn!(
                code = "MCP_HTTP_SESSION_STORE_EXPIRED",
                session_id,
                stored_at_unix_ms = persisted.stored_at_unix_ms,
                now_unix_ms = now_ms,
                ttl_ms = self.ttl.map(duration_millis_u64),
                "deleted expired MCP HTTP session state from CF_KV"
            );
            return Ok(None);
        }
        tracing::info!(
            code = "MCP_HTTP_SESSION_STORE_LOAD",
            session_id,
            stored_at_unix_ms = persisted.stored_at_unix_ms,
            "loaded MCP HTTP session state from CF_KV"
        );
        record_registry_initialized(
            &self.session_registry,
            session_id,
            &persisted.state,
            persisted.stored_at_unix_ms,
        )
        .map_err(session_store_error)?;
        Ok(Some(persisted.state))
    }

    async fn store(&self, session_id: &str, state: &SessionState) -> Result<(), SessionStoreError> {
        let key = mcp_session_store_key(session_id);
        let stored_at_unix_ms = unix_time_ms()?;
        let persisted = PersistedMcpSessionState {
            stored_at_unix_ms,
            state: state.clone(),
        };
        let encoded = synapse_storage::encode_json(&persisted).map_err(session_store_error)?;
        self.db
            .put_batch_pressure_bypass(cf::CF_KV, [(key, encoded)])
            .map_err(session_store_error)?;
        tracing::info!(
            code = "MCP_HTTP_SESSION_STORE_WRITE",
            session_id,
            stored_at_unix_ms,
            ttl_ms = self.ttl.map(duration_millis_u64),
            "persisted MCP HTTP session state to CF_KV"
        );
        record_registry_initialized(&self.session_registry, session_id, state, stored_at_unix_ms)
            .map_err(session_store_error)?;
        Ok(())
    }

    async fn delete(&self, session_id: &str) -> Result<(), SessionStoreError> {
        let key = mcp_session_store_key(session_id);
        self.db
            .delete_batch(cf::CF_KV, [key])
            .map_err(session_store_error)?;
        tracing::info!(
            code = "MCP_HTTP_SESSION_STORE_DELETE",
            session_id,
            "deleted MCP HTTP session state from CF_KV"
        );
        record_registry_closed(&self.session_registry, session_id).map_err(session_store_error)?;
        Ok(())
    }
}

fn record_registry_initialized(
    session_registry: &crate::server::session_registry::SharedSessionRegistry,
    session_id: &str,
    state: &SessionState,
    now_unix_ms: u64,
) -> Result<(), synapse_storage::StorageError> {
    let mut registry =
        session_registry
            .lock()
            .map_err(|_error| synapse_storage::StorageError::WriteFailed {
                cf_name: cf::CF_KV.to_owned(),
                detail: "session registry lock poisoned during session store".to_owned(),
            })?;
    registry.record_initialized(session_id, state, "http", now_unix_ms);
    Ok(())
}

fn record_registry_closed(
    session_registry: &crate::server::session_registry::SharedSessionRegistry,
    session_id: &str,
) -> Result<(), synapse_storage::StorageError> {
    let mut registry =
        session_registry
            .lock()
            .map_err(|_error| synapse_storage::StorageError::WriteFailed {
                cf_name: cf::CF_KV.to_owned(),
                detail: "session registry lock poisoned during session delete".to_owned(),
            })?;
    registry.record_closed(
        session_id,
        crate::server::session_registry::unix_time_ms_now(),
    );
    Ok(())
}

fn mcp_session_store_key(session_id: &str) -> Vec<u8> {
    crate::server::session_lifecycle::mcp_session_store_key(session_id)
}

fn session_store_error(error: synapse_storage::StorageError) -> SessionStoreError {
    Box::new(error)
}

fn unix_time_ms() -> Result<u64, SessionStoreError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| -> SessionStoreError { Box::new(error) })?;
    Ok(duration_millis_u64(elapsed))
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn session_store_expired(stored_at_unix_ms: u64, now_unix_ms: u64, ttl: Option<Duration>) -> bool {
    let Some(ttl) = ttl else {
        return false;
    };
    now_unix_ms.saturating_sub(stored_at_unix_ms) > duration_millis_u64(ttl)
}

fn http_service(
    shutdown_cancel: CancellationToken,
    connection_closed_cancel: CancellationToken,
    sse_state: SseState,
    m2_config: &M2ServiceConfig,
    m3_config: M3ServiceConfig,
    m4_config: M4ServiceConfig,
) -> io::Result<SynapseService> {
    SynapseService::try_with_m2_shutdown_reason_and_sse_state_and_m3_config(
        shutdown_cancel,
        "http",
        connection_closed_cancel,
        sse_state,
        m2_config,
        m3_config,
        m4_config,
    )
    .map_err(|error| io::Error::other(format!("{error:#}")))
}

async fn health(State(state): State<HttpState>) -> Json<Health> {
    tracing::info!(
        code = "MCP_HTTP_HEALTH",
        "tool.invocation kind=health transport=http"
    );
    let active_sessions = state.session_manager.sessions.read().await.len();
    Json(
        state
            .health_service
            .health_payload_with_http_sessions(Some(active_sessions)),
    )
}

async fn shutdown(State(state): State<HttpState>, headers: HeaderMap) -> Response {
    let active_sessions = state.session_manager.sessions.read().await.len();
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("<missing>");
    tracing::warn!(
        code = "MCP_HTTP_SHUTDOWN_REQUESTED",
        pid = std::process::id(),
        active_sessions,
        user_agent,
        "authenticated HTTP daemon shutdown requested"
    );
    tracing::info!(
        code = "MCP_SHUTDOWN_GRACEFUL",
        source = "http_shutdown",
        pid = std::process::id(),
        "HTTP shutdown endpoint cancelling daemon shutdown token"
    );
    state.shutdown_cancel.cancel();
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "ok": true,
            "pid": std::process::id(),
            "shutdown": "requested",
            "active_sessions_before_shutdown": active_sessions,
        })),
    )
        .into_response()
}

async fn events(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<sse::EventsQuery>,
) -> Response {
    state.sse_state.open(&headers, query)
}

async fn publish_event(
    State(state): State<HttpState>,
    Json(request): Json<sse::PublishRequest>,
) -> Response {
    state.sse_state.publish(request)
}

async fn event_stats(
    State(state): State<HttpState>,
    Query(query): Query<sse::StatsQuery>,
) -> Response {
    state.sse_state.stats(&query)
}

fn spawn_server(
    listener: TcpListener,
    app: Router,
    shutdown_cancel: CancellationToken,
) -> JoinHandle<io::Result<()>> {
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown_cancel.cancelled_owned().await })
            .await
    })
}

async fn wait_for_server_stop(server_task: &mut JoinHandle<io::Result<()>>) -> anyhow::Result<()> {
    match tokio::time::timeout(Duration::from_secs(2), &mut *server_task).await {
        Ok(result) => {
            result
                .context("join stopped HTTP MCP transport")?
                .context("stop HTTP MCP transport")?;
        }
        Err(_elapsed) => {
            server_task.abort();
            tracing::warn!(
                code = "MCP_HTTP_SHUTDOWN_TIMEOUT",
                "HTTP transport did not stop within shutdown timeout"
            );
        }
    }
    Ok(())
}

async fn wait_for_m2_emitter_done(
    done: Option<watch::Receiver<Option<ActionStateSnapshot>>>,
    source: &'static str,
) {
    let Some(mut done) = done else {
        tracing::warn!(
            code = "MCP_M2_EMITTER_SHUTDOWN_UNOBSERVED",
            source,
            "M2 emitter final snapshot receiver was unavailable during HTTP shutdown"
        );
        return;
    };

    let result = time::timeout(M2_EMITTER_SHUTDOWN_TIMEOUT, async {
        loop {
            if done.borrow().is_some() {
                break;
            }
            if done.changed().await.is_err() {
                break;
            }
        }
    })
    .await;

    match (result, done.borrow().as_ref()) {
        (Ok(()), Some(snapshot)) => {
            tracing::info!(
                code = "MCP_M2_EMITTER_SHUTDOWN_DONE",
                source,
                held_keys = snapshot.held_keys.len(),
                held_buttons = snapshot.held_buttons.len(),
                held_pads = snapshot.pad_state.len(),
                held_key_timer_count = snapshot.held_key_timer_count,
                "readback=action_emitter_state edge=http_shutdown after_emitter_done"
            );
        }
        (Ok(()), None) => {
            tracing::warn!(
                code = "MCP_M2_EMITTER_SHUTDOWN_UNOBSERVED",
                source,
                "M2 emitter ended without publishing a final snapshot during HTTP shutdown"
            );
        }
        (Err(_elapsed), _) => {
            tracing::error!(
                code = "MCP_M2_EMITTER_SHUTDOWN_TIMEOUT",
                source,
                timeout_ms = M2_EMITTER_SHUTDOWN_TIMEOUT.as_millis(),
                "M2 emitter did not publish final shutdown snapshot before HTTP daemon exit"
            );
        }
    }
}

#[cfg(windows)]
async fn wait_for_shutdown_signal(phase: &'static str) -> anyhow::Result<()> {
    let mut ctrl_break = tokio::signal::windows::ctrl_break()
        .with_context(|| format!("register ctrl-break handler {phase}"))?;
    tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            signal.with_context(|| format!("wait for ctrl-c {phase}"))?;
        }
        received = ctrl_break.recv() => {
            if received.is_none() {
                anyhow::bail!("ctrl-break stream ended while waiting for shutdown signal {phase}");
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
async fn wait_for_shutdown_signal(phase: &'static str) -> anyhow::Result<()> {
    tokio::signal::ctrl_c()
        .await
        .with_context(|| format!("wait for ctrl-c {phase}"))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use crate::test_support;
    use anyhow::Context as _;
    use rmcp::model::{ClientCapabilities, Implementation, InitializeRequestParams};
    use rmcp::transport::streamable_http_server::session::SessionManager as _;
    use synapse_action::{ActionBackend, ActionEmitter, RecordingBackend};
    use synapse_core::{Action, Backend, Key, KeyCode, SCHEMA_VERSION};

    use super::*;

    const TEST_RESET_REASON: &str = "http_transport_lease_test_reset";

    fn test_session_state(name: &str) -> SessionState {
        SessionState::new(InitializeRequestParams::new(
            ClientCapabilities::default(),
            Implementation::new(name, "0.0.0-test"),
        ))
    }

    fn test_store_error(error: SessionStoreError) -> anyhow::Error {
        anyhow::anyhow!("{error}")
    }

    fn empty_cdp_target_owners() -> crate::server::SharedCdpTargetOwners {
        Arc::new(Mutex::new(HashMap::new()))
    }

    fn empty_session_registry() -> crate::server::session_registry::SharedSessionRegistry {
        Arc::new(Mutex::new(
            crate::server::session_registry::SessionRegistry::default(),
        ))
    }

    #[tokio::test]
    async fn synapse_mcp_session_store_round_trips_exact_keys_and_deletes() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Arc::new(Db::open(&temp.path().join("db"), SCHEMA_VERSION)?);
        let store = SynapseMcpSessionStore::new(
            Arc::clone(&db),
            Some(Duration::from_mins(5)),
            empty_session_registry(),
        );

        assert!(
            store
                .load("codex-session")
                .await
                .map_err(test_store_error)?
                .is_none(),
            "unknown session should not load"
        );

        let state = test_session_state("codex-test");
        let neighboring_state = test_session_state("codex-test-neighbor");
        store
            .store("codex-session", &state)
            .await
            .map_err(test_store_error)?;
        let stored_rows = db.scan_cf_prefix(cf::CF_KV, &mcp_session_store_key("codex-session"))?;
        let stored_row = stored_rows
            .iter()
            .find(|(key, _value)| key == &mcp_session_store_key("codex-session"))
            .context("stored row should exist in CF_KV")?;
        let persisted = synapse_storage::decode_json::<PersistedMcpSessionState>(&stored_row.1)?;
        assert_eq!(persisted.state.initialize_params, state.initialize_params);

        store
            .store("codex-session-extra", &neighboring_state)
            .await
            .map_err(test_store_error)?;

        let loaded = store
            .load("codex-session")
            .await
            .map_err(test_store_error)?
            .context("stored session should load")?;
        assert_eq!(loaded.initialize_params, state.initialize_params);

        store
            .delete("codex-session")
            .await
            .map_err(test_store_error)?;
        assert!(
            store
                .load("codex-session")
                .await
                .map_err(test_store_error)?
                .is_none(),
            "deleted session should not load"
        );
        assert!(
            store
                .load("codex-session-extra")
                .await
                .map_err(test_store_error)?
                .is_some(),
            "deleting one session should not delete a prefix-sharing neighbor"
        );

        Ok(())
    }

    #[tokio::test]
    async fn synapse_mcp_session_store_deletes_expired_rows() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Arc::new(Db::open(&temp.path().join("db"), SCHEMA_VERSION)?);
        let store = SynapseMcpSessionStore::new(
            Arc::clone(&db),
            Some(Duration::from_millis(1)),
            empty_session_registry(),
        );
        let key = mcp_session_store_key("expired-session");

        store
            .store("expired-session", &test_session_state("expired-test"))
            .await
            .map_err(test_store_error)?;
        assert!(
            db.scan_cf_prefix(cf::CF_KV, &key)?
                .into_iter()
                .any(|(row_key, _value)| row_key == key),
            "stored row should physically exist before expiry"
        );

        tokio::time::sleep(Duration::from_millis(10)).await;

        assert!(
            store
                .load("expired-session")
                .await
                .map_err(test_store_error)?
                .is_none(),
            "expired session should not load"
        );
        assert!(
            !db.scan_cf_prefix(cf::CF_KV, &key)?
                .into_iter()
                .any(|(row_key, _value)| row_key == key),
            "expired session row should be deleted from CF_KV"
        );

        Ok(())
    }

    #[tokio::test]
    async fn synapse_mcp_session_store_deletes_legacy_rows_without_ttl() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Arc::new(Db::open(&temp.path().join("db"), SCHEMA_VERSION)?);
        let store = SynapseMcpSessionStore::new(
            Arc::clone(&db),
            Some(Duration::from_mins(5)),
            empty_session_registry(),
        );
        let key = mcp_session_store_key("legacy-session");
        let legacy_state = test_session_state("legacy-test");
        let legacy_encoded = synapse_storage::encode_json(&legacy_state)?;
        db.put_batch_pressure_bypass(cf::CF_KV, [(key.clone(), legacy_encoded)])?;

        assert!(
            db.scan_cf_prefix(cf::CF_KV, &key)?
                .into_iter()
                .any(|(row_key, _value)| row_key == key),
            "legacy row should physically exist before load"
        );

        assert!(
            store
                .load("legacy-session")
                .await
                .map_err(test_store_error)?
                .is_none(),
            "legacy row without persistent TTL metadata should not load"
        );
        assert!(
            !db.scan_cf_prefix(cf::CF_KV, &key)?
                .into_iter()
                .any(|(row_key, _value)| row_key == key),
            "legacy session row should be deleted from CF_KV"
        );

        Ok(())
    }

    #[tokio::test]
    async fn stale_session_cleanup_releases_absent_inputs_only() -> anyhow::Result<()> {
        let _serial = test_support::lease_serial(TEST_RESET_REASON);
        let cancel = CancellationToken::new();
        let backend: Arc<dyn ActionBackend> = Arc::new(RecordingBackend::new());
        let (handle, snapshot_handle, join) =
            ActionEmitter::spawn_with_backend(cancel.clone(), backend);
        let session_manager = Arc::new(LocalSessionManager::default());
        let (active_session_id, _active_transport) = session_manager
            .create_session()
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        let active_session_text = active_session_id.as_ref().to_owned();
        let stale_session_id = "stale-session".to_owned();
        let _prior = synapse_action::lease::force_clear("http_stale_inputs_test_reset");
        let _held = synapse_action::lease::try_acquire(&stale_session_id, Duration::from_secs(30));

        handle
            .with_session_id(Some(stale_session_id.clone()))
            .execute(Action::KeyDown {
                key: test_key("ctrl"),
                backend: Backend::Software,
            })
            .await?;
        handle
            .with_session_id(Some(active_session_text.clone()))
            .execute(Action::KeyDown {
                key: test_key("shift"),
                backend: Backend::Software,
            })
            .await?;

        let before_state = snapshot_handle.snapshot().await?;
        let before_ownership = handle.session_inputs_snapshot()?;
        let before_lease = synapse_action::lease::status();
        println!(
            "readback=http_session_cleanup edge=stale_owner before_state={before_state:?} before_ownership={before_ownership:?} before_lease={before_lease:?} active_session_id={active_session_text}"
        );
        assert_eq!(
            before_lease.owner_session_id.as_deref(),
            Some(stale_session_id.as_str())
        );

        let cdp_target_owners = empty_cdp_target_owners();
        cleanup_stale_session_inputs_once(&handle, &session_manager, &cdp_target_owners).await;

        let after_state = snapshot_handle.snapshot().await?;
        let after_ownership = handle.session_inputs_snapshot()?;
        let after_lease = synapse_action::lease::status();
        println!(
            "readback=http_session_cleanup edge=stale_owner after_state={after_state:?} after_ownership={after_ownership:?} after_lease={after_lease:?}"
        );

        assert_eq!(after_state.held_keys, vec![test_key("shift")]);
        assert!(!after_lease.held);
        assert!(
            after_ownership
                .sessions
                .iter()
                .any(|session| session.session_id == active_session_text),
            "active session ownership should be retained"
        );
        assert!(
            !after_ownership
                .sessions
                .iter()
                .any(|session| session.session_id == stale_session_id),
            "stale session ownership should be removed"
        );

        session_manager
            .close_session(&active_session_id)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        handle.execute(Action::ReleaseAll).await?;
        cancel.cancel();
        let final_snapshot = join.await?;
        assert!(final_snapshot.held_keys.is_empty());
        let _prior = synapse_action::lease::force_clear("http_stale_inputs_test_reset");

        Ok(())
    }

    #[tokio::test]
    async fn stale_session_cleanup_releases_absent_lease_without_inputs() -> anyhow::Result<()> {
        let _serial = test_support::lease_serial(TEST_RESET_REASON);
        let cancel = CancellationToken::new();
        let backend: Arc<dyn ActionBackend> = Arc::new(RecordingBackend::new());
        let (handle, snapshot_handle, join) =
            ActionEmitter::spawn_with_backend(cancel.clone(), backend);
        let session_manager = LocalSessionManager::default();
        let stale_session_id = "stale-lease-session";
        let _prior = synapse_action::lease::force_clear("http_stale_lease_test_reset");
        let _held = synapse_action::lease::try_acquire(stale_session_id, Duration::from_secs(30));

        let before_state = snapshot_handle.snapshot().await?;
        let before_lease = synapse_action::lease::status();
        println!(
            "readback=http_session_cleanup edge=stale_lease before_state={before_state:?} before_lease={before_lease:?}"
        );

        let cdp_target_owners = empty_cdp_target_owners();
        cleanup_stale_session_inputs_once(&handle, &session_manager, &cdp_target_owners).await;

        let after_state = snapshot_handle.snapshot().await?;
        let after_lease = synapse_action::lease::status();
        println!(
            "readback=http_session_cleanup edge=stale_lease after_state={after_state:?} after_lease={after_lease:?}"
        );
        assert!(!after_lease.held);
        assert_eq!(after_lease.owner_session_id, None);

        cancel.cancel();
        let final_snapshot = join.await?;
        assert!(final_snapshot.held_keys.is_empty());
        let _prior = synapse_action::lease::force_clear("http_stale_lease_test_reset");

        Ok(())
    }

    #[tokio::test]
    async fn expired_lease_cleanup_releases_held_input_before_reacquire() -> anyhow::Result<()> {
        let _serial = test_support::lease_serial(TEST_RESET_REASON);
        let cancel = CancellationToken::new();
        let backend: Arc<dyn ActionBackend> = Arc::new(RecordingBackend::new());
        let (handle, snapshot_handle, join) =
            ActionEmitter::spawn_with_backend(cancel.clone(), backend);
        let session_manager = LocalSessionManager::default();
        let expired_session_id = "expired-lease-session";
        let contender_session_id = "expired-lease-contender";
        let _prior = synapse_action::lease::force_clear("http_expired_lease_test_reset");

        handle
            .with_session_id(Some(expired_session_id.to_owned()))
            .execute(Action::KeyDown {
                key: test_key("ctrl"),
                backend: Backend::Software,
            })
            .await?;
        let _held = synapse_action::lease::try_acquire(
            expired_session_id,
            Duration::from_millis(synapse_action::MIN_LEASE_TTL_MS),
        );
        tokio::time::sleep(Duration::from_millis(synapse_action::MIN_LEASE_TTL_MS + 50)).await;

        let before_state = snapshot_handle.snapshot().await?;
        let before_ownership = handle.session_inputs_snapshot()?;
        let before_lease = synapse_action::lease::status();
        let before_pending = synapse_action::lease::expired_cleanup_snapshot();
        println!(
            "readback=http_session_cleanup edge=expired_lease before_state={before_state:?} before_ownership={before_ownership:?} before_lease={before_lease:?} before_pending={before_pending:?}"
        );
        assert_eq!(before_state.held_keys, vec![test_key("ctrl")]);
        assert!(!before_lease.held);
        assert_eq!(before_pending.len(), 1);
        match synapse_action::lease::try_acquire(contender_session_id, Duration::from_secs(30)) {
            synapse_action::LeaseOutcome::CleanupPending { expired, .. } => {
                assert_eq!(
                    expired.owner_session_id.as_deref(),
                    Some(expired_session_id)
                );
            }
            other => anyhow::bail!("contender should be refused pending cleanup, got {other:?}"),
        }

        let cdp_target_owners = empty_cdp_target_owners();
        cleanup_stale_session_inputs_once(&handle, &session_manager, &cdp_target_owners).await;

        let after_state = snapshot_handle.snapshot().await?;
        let after_ownership = handle.session_inputs_snapshot()?;
        let after_pending = synapse_action::lease::expired_cleanup_snapshot();
        let acquire_after_cleanup =
            synapse_action::lease::try_acquire(contender_session_id, Duration::from_secs(30));
        let after_lease = synapse_action::lease::status();
        println!(
            "readback=http_session_cleanup edge=expired_lease after_state={after_state:?} after_ownership={after_ownership:?} after_pending={after_pending:?} acquire_after_cleanup={acquire_after_cleanup:?} after_lease={after_lease:?}"
        );
        assert!(after_state.held_keys.is_empty());
        assert!(after_ownership.sessions.is_empty());
        assert!(after_pending.is_empty());
        assert!(matches!(
            acquire_after_cleanup,
            synapse_action::LeaseOutcome::Acquired(_)
        ));
        assert_eq!(
            after_lease.owner_session_id.as_deref(),
            Some(contender_session_id)
        );

        let _released = synapse_action::lease::release_if_owner(contender_session_id);
        cancel.cancel();
        let final_snapshot = join.await?;
        assert!(final_snapshot.held_keys.is_empty());
        let _prior = synapse_action::lease::force_clear("http_expired_lease_test_reset");

        Ok(())
    }

    fn test_key(value: &str) -> Key {
        Key {
            code: KeyCode::Named {
                value: value.to_owned(),
            },
            use_scancode: false,
        }
    }
}
