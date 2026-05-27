use std::{io, path::PathBuf};

use synapse_core::error_codes;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("profile IO failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("profile parse failed for {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error(
        "profile schema version {schema_version} in {path} is incompatible with supported version {supported_version}"
    )]
    VersionIncompatible {
        path: PathBuf,
        schema_version: u32,
        supported_version: u32,
    },
    #[error("profile keymap entry {alias}={binding} in {path} is invalid: {message}")]
    KeymapInvalid {
        path: PathBuf,
        alias: String,
        binding: String,
        message: String,
    },
    #[error("profile HUD region {name} in {path} is invalid: {message}")]
    HudRegionInvalid {
        path: PathBuf,
        name: String,
        message: String,
    },
    #[error("profile {profile_id} was not found")]
    NotFound { profile_id: String },
    #[error("profile watcher failed for {path}: {message}")]
    Watch { path: PathBuf, message: String },
    #[error("profile runtime state lock is poisoned")]
    StatePoisoned,
}

impl ProfileError {
    #[must_use]
    #[tracing::instrument(skip_all, fields(error = %self))]
    pub fn code(&self) -> &'static str {
        match self {
            Self::VersionIncompatible { .. } => error_codes::PROFILE_VERSION_INCOMPATIBLE,
            Self::KeymapInvalid { .. } => error_codes::PROFILE_KEYMAP_INVALID,
            Self::HudRegionInvalid { .. } => error_codes::PROFILE_HUD_REGION_INVALID,
            Self::NotFound { .. } => error_codes::PROFILE_NOT_FOUND,
            Self::Io { .. } | Self::Parse { .. } | Self::Watch { .. } | Self::StatePoisoned => {
                error_codes::PROFILE_PARSE_ERROR
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileLoadError {
    pub path: PathBuf,
    pub code: &'static str,
    pub message: String,
}

impl ProfileLoadError {
    #[must_use]
    #[tracing::instrument(skip_all, fields(code = error.code()))]
    pub fn from_error(error: &ProfileError) -> Self {
        let path = match error {
            ProfileError::Io { path, .. }
            | ProfileError::Parse { path, .. }
            | ProfileError::VersionIncompatible { path, .. }
            | ProfileError::KeymapInvalid { path, .. }
            | ProfileError::HudRegionInvalid { path, .. }
            | ProfileError::Watch { path, .. } => path.clone(),
            ProfileError::NotFound { .. } | ProfileError::StatePoisoned => PathBuf::new(),
        };
        Self {
            path,
            code: error.code(),
            message: error.to_string(),
        }
    }
}
