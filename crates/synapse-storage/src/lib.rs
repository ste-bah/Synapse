pub mod codecs;
pub mod error;

use std::path::{Path, PathBuf};

use synapse_core::error_codes;

pub use codecs::{decode_json, encode_json};
pub use error::{StorageError, StorageResult};

/// Opened storage handle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Db {
    pub path: PathBuf,
    pub schema_version: u32,
}

impl Db {
    /// Opens the storage scaffold at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::OpenFailed`] when the storage directory cannot
    /// be created.
    #[tracing::instrument(skip_all, fields(storage_path = %path.display(), schema_version))]
    pub fn open(path: &Path, schema_version: u32) -> StorageResult<Self> {
        std::fs::create_dir_all(path).map_err(|source| {
            tracing::warn!(
                code = error_codes::STORAGE_OPEN_FAILED,
                storage_path = %path.display(),
                %source,
                "storage open failed"
            );
            StorageError::OpenFailed {
                path: path.to_path_buf(),
                source,
            }
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            schema_version,
        })
    }
}
