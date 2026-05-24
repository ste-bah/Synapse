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
use tokio_util::sync::CancellationToken;

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

impl M3State {
    pub fn from_env() -> Result<Self> {
        Self::from_env_with_shutdown_reason(CancellationToken::new(), "shutdown", None)
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
        })
    }

    #[must_use]
    pub const fn scaffold_ready(&self) -> bool {
        !self.bind.is_empty()
    }
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
