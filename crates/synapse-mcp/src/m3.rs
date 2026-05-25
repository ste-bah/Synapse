pub mod audio;
pub mod profile;
pub mod reflex;
pub mod replay;
pub mod subscribe;
#[cfg(test)]
mod tests;
use anyhow::{Result, bail};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use synapse_action::ActionHandle;
use synapse_core::SCHEMA_VERSION;
use synapse_profiles::{ProfileError, ProfileRuntime, bundled_profiles_dir};
use synapse_reflex::{EventBus, ReflexError, ReflexRuntime};
use synapse_storage::Db;
use tokio_util::sync::CancellationToken;

use crate::http::sse::SseState;

const DB_ENV: &str = "SYNAPSE_DB";
const PROFILE_DIR_ENV: &str = "SYNAPSE_PROFILE_DIR";
const REFLEX_DISABLED_ENV: &str = "SYNAPSE_REFLEX_DISABLED";
const BIND_ENV: &str = "SYNAPSE_BIND";
const BEARER_TOKEN_ENV: &str = "SYNAPSE_BEARER_TOKEN";
const DEFAULT_BIND: &str = "127.0.0.1:7700";
pub type SharedM3State = Arc<Mutex<M3State>>;

#[derive(Clone, Debug)]
pub struct M3State {
    pub db_path: Option<PathBuf>,
    pub profile_dir: Option<PathBuf>,
    pub reflex_disabled: bool,
    pub bind: String,
    pub bearer_token: Option<String>,
    pub shutdown_cancel: CancellationToken,
    pub shutdown_reason: &'static str,
    pub connection_closed_cancel: Option<CancellationToken>,
    pub profile_runtime: Option<Arc<ProfileRuntime>>,
    pub sse_state: SseState,
    pub reflex_runtime: Option<Arc<Mutex<ReflexRuntime>>>,
}

pub fn shared_m3_state_from_env() -> Result<SharedM3State> {
    Ok(Arc::new(Mutex::new(M3State::from_env()?)))
}

pub fn shared_m3_state_from_env_with_shutdown_reason(
    shutdown_cancel: CancellationToken,
    shutdown_reason: &'static str,
    connection_closed_cancel: Option<CancellationToken>,
) -> Result<SharedM3State> {
    Ok(Arc::new(Mutex::new(
        M3State::from_env_with_shutdown_reason(
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
        )?,
    )))
}

pub fn shared_m3_state_from_env_with_shutdown_reason_and_sse_state(
    shutdown_cancel: CancellationToken,
    shutdown_reason: &'static str,
    connection_closed_cancel: Option<CancellationToken>,
    sse_state: SseState,
) -> Result<SharedM3State> {
    Ok(Arc::new(Mutex::new(
        M3State::from_env_with_shutdown_reason_and_sse_state(
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
            sse_state,
        )?,
    )))
}

impl M3State {
    pub fn from_env() -> Result<Self> {
        Self::from_env_with_sse_state(SseState::from_env())
    }

    pub fn from_env_with_sse_state(sse_state: SseState) -> Result<Self> {
        Self::from_env_with_shutdown_reason_and_sse_state(
            CancellationToken::new(),
            "shutdown",
            None,
            sse_state,
        )
    }

    pub fn from_env_with_shutdown_reason(
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: Option<CancellationToken>,
    ) -> Result<Self> {
        Self::from_parts(
            std::env::var_os(DB_ENV).map(PathBuf::from),
            std::env::var_os(PROFILE_DIR_ENV).map(PathBuf::from),
            std::env::var(REFLEX_DISABLED_ENV).ok().as_deref(),
            std::env::var(BEARER_TOKEN_ENV).ok(),
            std::env::var(BIND_ENV).ok(),
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
        )
    }

    pub fn from_env_with_shutdown_reason_and_sse_state(
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: Option<CancellationToken>,
        sse_state: SseState,
    ) -> Result<Self> {
        Self::from_parts_with_sse_state(
            std::env::var_os(DB_ENV).map(PathBuf::from),
            std::env::var_os(PROFILE_DIR_ENV).map(PathBuf::from),
            std::env::var(REFLEX_DISABLED_ENV).ok().as_deref(),
            std::env::var(BEARER_TOKEN_ENV).ok(),
            std::env::var(BIND_ENV).ok(),
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
            sse_state,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        db_path: Option<PathBuf>,
        profile_dir: Option<PathBuf>,
        reflex_disabled: Option<&str>,
        bearer_token: Option<String>,
        bind: Option<String>,
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: Option<CancellationToken>,
    ) -> Result<Self> {
        Self::from_parts_with_sse_state(
            db_path,
            profile_dir,
            reflex_disabled,
            bearer_token,
            bind,
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
            SseState::from_env(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_parts_with_sse_state(
        db_path: Option<PathBuf>,
        profile_dir: Option<PathBuf>,
        reflex_disabled: Option<&str>,
        bearer_token: Option<String>,
        bind: Option<String>,
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: Option<CancellationToken>,
        sse_state: SseState,
    ) -> Result<Self> {
        Ok(Self {
            db_path,
            profile_dir,
            reflex_disabled: parse_bool_env(REFLEX_DISABLED_ENV, reflex_disabled)?,
            bind: bind
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_BIND.to_owned()),
            bearer_token: bearer_token.filter(|value| !value.is_empty()),
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
            profile_runtime: None,
            sse_state,
            reflex_runtime: None,
        })
    }

    #[must_use]
    pub const fn scaffold_ready(&self) -> bool {
        !self.bind.is_empty()
    }

    pub fn ensure_profile_runtime(
        &mut self,
    ) -> std::result::Result<Arc<ProfileRuntime>, ProfileError> {
        if let Some(runtime) = &self.profile_runtime {
            return Ok(Arc::clone(runtime));
        }

        let profile_dir = self
            .profile_dir
            .clone()
            .unwrap_or_else(bundled_profiles_dir);
        let runtime = Arc::new(ProfileRuntime::spawn(profile_dir)?);
        self.profile_runtime = Some(Arc::clone(&runtime));
        Ok(runtime)
    }

    pub fn ensure_reflex_runtime(
        &mut self,
        action_handle: ActionHandle,
        event_bus: EventBus,
    ) -> Result<Arc<Mutex<ReflexRuntime>>> {
        if let Some(runtime) = &self.reflex_runtime {
            return Ok(Arc::clone(runtime));
        }
        if self.reflex_disabled {
            bail!(ReflexError::DisabledByOperator {
                detail: "SYNAPSE_REFLEX_DISABLED is set".to_owned(),
            });
        }

        let db_path = self.db_path.clone().unwrap_or_else(default_db_path);
        let db = Arc::new(Db::open(&db_path, SCHEMA_VERSION)?);
        let runtime = Arc::new(Mutex::new(ReflexRuntime::spawn(
            db,
            action_handle,
            event_bus,
        )?));
        self.reflex_runtime = Some(Arc::clone(&runtime));
        Ok(runtime)
    }
}

fn default_db_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("synapse")
        .join("db")
}
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct M3ToolStub {
    pub name: &'static str,
}

impl M3ToolStub {
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }
}

#[must_use]
pub const fn m3_tool_stubs() -> [M3ToolStub; 11] {
    [
        subscribe::subscribe(),
        subscribe::subscribe_cancel(),
        reflex::reflex_register(),
        reflex::reflex_cancel(),
        reflex::reflex_list(),
        reflex::reflex_history(),
        profile::profile_list(),
        profile::profile_activate(),
        replay::replay_record(),
        audio::audio_tail(),
        audio::audio_transcribe(),
    ]
}

fn parse_bool_env(name: &str, value: Option<&str>) -> Result<bool> {
    match value {
        None | Some("0") => Ok(false),
        Some("1") => Ok(true),
        Some(value) if value.eq_ignore_ascii_case("true") => Ok(true),
        Some(value) if value.eq_ignore_ascii_case("false") => Ok(false),
        Some(value) => bail!("{name} must be one of 1, 0, true, or false; got {value:?}"),
    }
}
