use std::{io, path::PathBuf};

use synapse_core::error_codes;
use thiserror::Error;

pub type StorageResult<T> = Result<T, StorageError>;

/// Storage failures with stable Synapse error codes.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage open failed for {path:?}: {source}")]
    OpenFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("storage write failed while encoding {type_name}: {source}")]
    EncodeJson {
        type_name: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("storage read failed while decoding {type_name}: {source}")]
    DecodeJson {
        type_name: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("storage schema mismatch: expected {expected}, actual {actual}")]
    SchemaMismatch { expected: u32, actual: u32 },
}

impl StorageError {
    /// Returns the stable Synapse error code for this storage failure.
    #[tracing::instrument(skip_all, fields(storage_error = ?self))]
    pub fn code(&self) -> &'static str {
        match self {
            Self::OpenFailed { .. } => error_codes::STORAGE_OPEN_FAILED,
            Self::EncodeJson { .. } => error_codes::STORAGE_WRITE_FAILED,
            Self::DecodeJson { .. } => error_codes::STORAGE_READ_FAILED,
            Self::SchemaMismatch { .. } => error_codes::STORAGE_SCHEMA_MISMATCH,
        }
    }
}
