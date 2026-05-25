mod batch;
pub mod cf;
pub mod codecs;
pub mod compaction;
pub mod error;
mod gc;
mod pressure;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rocksdb::{
    BlockBasedOptions, Cache, ColumnFamilyDescriptor, ColumnFamilyRef, DB, DBCompressionType,
    Direction, IteratorMode, Options, SliceTransform,
};
use synapse_core::error_codes;

pub use codecs::{decode_json, encode_json};
pub use error::{StorageError, StorageResult};
pub use gc::{GcCfReport, GcReport, GcTask};
pub use pressure::{DiskPressureLevel, PressureReport, PressureTask};

const MIB: usize = 1024 * 1024;
const DEFAULT_WRITE_BUFFER_BYTES: usize = 64 * MIB;
const MODEL_CACHE_WRITE_BUFFER_BYTES: usize = 256 * MIB;
const BLOCK_CACHE_BYTES: usize = 64 * MIB;
const SCHEMA_VERSION_KEY: &[u8] = b"__schema_version";

/// Opened storage handle.
pub struct Db {
    pub path: PathBuf,
    pub schema_version: u32,
    batcher: batch::Batcher,
    inner: Arc<DB>,
    pressure: Arc<pressure::PressureState>,
}

impl fmt::Debug for Db {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Db")
            .field("path", &self.path)
            .field("schema_version", &self.schema_version)
            .finish_non_exhaustive()
    }
}

impl Db {
    /// Opens the `RocksDB` storage at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::OpenFailed`] when `RocksDB` cannot open or
    /// initialize the database, or [`StorageError::SchemaMismatch`] when the
    /// stored schema sentinel differs from `schema_version`.
    #[tracing::instrument(skip_all, fields(storage_path = %path.display(), schema_version))]
    pub fn open(path: &Path, schema_version: u32) -> StorageResult<Self> {
        let options = db_options();
        let descriptors = cf::ALL_COLUMN_FAMILIES
            .into_iter()
            .map(|name| ColumnFamilyDescriptor::new(name, cf_options(name)));
        let inner = DB::open_cf_descriptors(&options, path, descriptors)
            .map_err(|source| open_failed(path, &source))?;
        verify_schema_version(&inner, path, schema_version)?;

        for name in cf::ALL_COLUMN_FAMILIES {
            if inner.cf_handle(name).is_none() {
                return Err(open_failed_detail(
                    path,
                    format!("column family handle missing after open: {name}"),
                ));
            }
        }

        let inner = Arc::new(inner);
        let batcher = batch::Batcher::spawn(Arc::clone(&inner));
        let pressure = Arc::new(pressure::PressureState::default());

        Ok(Self {
            path: path.to_path_buf(),
            schema_version,
            batcher,
            inner,
            pressure,
        })
    }

    /// Enqueues key/value writes for one column family.
    ///
    /// Callers aggregate producer-side and submit batches; per-frame single
    /// writes intentionally are not part of this API.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::WriteFailed`] when the column family is missing,
    /// the background batcher is unavailable, or `RocksDB` rejects the batch.
    #[tracing::instrument(skip_all, fields(cf_name))]
    pub fn put_batch<I, K, V>(&self, cf_name: &str, kvs: I) -> StorageResult<()>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<Vec<u8>>,
        V: Into<Vec<u8>>,
    {
        self.inner
            .cf_handle(cf_name)
            .ok_or_else(|| StorageError::WriteFailed {
                cf_name: cf_name.to_owned(),
                detail: "column family handle missing".to_owned(),
            })?;
        let kvs = kvs
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect::<Vec<_>>();
        if kvs.is_empty() {
            return Ok(());
        }
        if !self.pressure.permits_write(cf_name) {
            tracing::warn!(
                code = error_codes::STORAGE_WRITE_FAILED,
                cf = cf_name,
                pressure_level = ?self.pressure.level(),
                "storage write dropped under disk pressure"
            );
            return Ok(());
        }
        self.batcher.put_batch(cf_name, kvs)
    }

    /// Flushes pending batched writes with synchronous `RocksDB` write options.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::WriteFailed`] when the background batcher is
    /// unavailable or `RocksDB` rejects the flush.
    #[tracing::instrument(skip_all)]
    pub fn flush(&self) -> StorageResult<()> {
        self.batcher.flush()
    }

    /// Runs one storage garbage-collection pass immediately.
    ///
    /// # Errors
    ///
    /// Returns a storage error when `RocksDB` property reads, deletes, flushes,
    /// or compactions fail.
    #[tracing::instrument(skip_all)]
    pub fn run_gc_once(&self) -> StorageResult<GcReport> {
        gc::run_once(&self.inner, &gc::GcConfig::from_retention_defaults())
    }

    /// Runs one row-count-scaled GC pass for deterministic regression tests.
    ///
    /// This avoids writing gigabytes to hit production byte caps.
    ///
    /// # Errors
    ///
    /// Returns a storage error when `RocksDB` property reads, deletes, flushes,
    /// or compactions fail.
    #[doc(hidden)]
    #[tracing::instrument(skip_all)]
    pub fn run_gc_once_with_row_caps(
        &self,
        cf_name: &'static str,
        soft_cap_rows: u64,
        hard_cap_rows: u64,
    ) -> StorageResult<GcReport> {
        gc::run_once(
            &self.inner,
            &gc::GcConfig::for_row_caps(
                Duration::from_mins(5),
                cf_name,
                soft_cap_rows,
                hard_cap_rows,
            ),
        )
    }

    /// Spawns the periodic storage garbage-collection task.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::WriteFailed`] when no Tokio runtime is active.
    #[tracing::instrument(skip_all)]
    pub fn spawn_gc_task(&self) -> StorageResult<GcTask> {
        gc::spawn(
            Arc::clone(&self.inner),
            gc::GcConfig::from_retention_defaults(),
        )
    }

    /// Returns the current DB-volume disk-pressure level.
    #[must_use]
    pub fn pressure_level(&self) -> DiskPressureLevel {
        self.pressure.level()
    }

    /// Returns approximate logical bytes currently stored in each Synapse column family.
    ///
    /// This scans row keys and values so health reports reflect persisted data
    /// even when `RocksDB` has not compacted files yet.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::ReadFailed`] when a column family cannot be scanned.
    #[tracing::instrument(skip_all)]
    pub fn cf_sizes(&self) -> StorageResult<BTreeMap<String, u64>> {
        let mut sizes = BTreeMap::new();
        for cf_name in cf::ALL_COLUMN_FAMILIES {
            let mut bytes = 0_u64;
            for (key, value) in self.scan_cf(cf_name)? {
                bytes = bytes.saturating_add(key.len() as u64);
                bytes = bytes.saturating_add(value.len() as u64);
            }
            sizes.insert(cf_name.to_owned(), bytes);
        }
        Ok(sizes)
    }

    /// Runs one disk-pressure check immediately.
    ///
    /// # Errors
    ///
    /// Returns a storage error when disk free-space probing or pressure-triggered
    /// compaction fails.
    #[tracing::instrument(skip_all)]
    pub fn run_pressure_check_once(&self) -> StorageResult<PressureReport> {
        pressure::run_once(
            &self.inner,
            &self.pressure,
            &self.path,
            &pressure::PressureConfig::default(),
        )
    }

    /// Applies one synthetic free-byte sample for deterministic regression tests.
    ///
    /// This uses the production thresholds and responder actions while avoiding
    /// host-volume manipulation.
    ///
    /// # Errors
    ///
    /// Returns a storage error when pressure-triggered compaction fails.
    #[doc(hidden)]
    #[tracing::instrument(skip_all)]
    pub fn run_pressure_check_with_free_bytes_sample(
        &self,
        free_bytes: u64,
    ) -> StorageResult<PressureReport> {
        pressure::run_once_with_free_bytes(
            &self.inner,
            &self.pressure,
            &pressure::PressureConfig::default(),
            free_bytes,
        )
    }

    /// Spawns the periodic DB-volume disk-pressure task.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::WriteFailed`] when no Tokio runtime is active.
    #[tracing::instrument(skip_all)]
    pub fn spawn_pressure_task(&self) -> StorageResult<PressureTask> {
        pressure::spawn(
            Arc::clone(&self.inner),
            Arc::clone(&self.pressure),
            self.path.clone(),
            pressure::PressureConfig::default(),
        )
    }

    /// Scans a column family into owned key/value bytes.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::ReadFailed`] when the column family is missing
    /// or `RocksDB` iteration fails.
    #[tracing::instrument(skip_all, fields(cf_name))]
    pub fn scan_cf(&self, cf_name: &str) -> StorageResult<Vec<(Vec<u8>, Vec<u8>)>> {
        let handle = self.cf_handle(cf_name)?;
        let mut rows = Vec::new();
        for item in self.inner.iterator_cf(&handle, IteratorMode::Start) {
            let (key, value) = item.map_err(|source| StorageError::ReadFailed {
                cf_name: cf_name.to_owned(),
                detail: source.to_string(),
            })?;
            rows.push((key.to_vec(), value.to_vec()));
        }
        Ok(rows)
    }

    /// Scans a column family from a key prefix into owned key/value bytes.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::ReadFailed`] when the column family is missing
    /// or `RocksDB` iteration fails.
    #[tracing::instrument(skip_all, fields(cf_name, prefix_len = prefix.len()))]
    pub fn scan_cf_prefix(
        &self,
        cf_name: &str,
        prefix: &[u8],
    ) -> StorageResult<Vec<(Vec<u8>, Vec<u8>)>> {
        let handle = self.cf_handle(cf_name)?;
        let mut rows = Vec::new();
        for item in self
            .inner
            .iterator_cf(&handle, IteratorMode::From(prefix, Direction::Forward))
        {
            let (key, value) = item.map_err(|source| StorageError::ReadFailed {
                cf_name: cf_name.to_owned(),
                detail: source.to_string(),
            })?;
            if !key.starts_with(prefix) {
                break;
            }
            rows.push((key.to_vec(), value.to_vec()));
        }
        Ok(rows)
    }

    /// Compacts a whole column family.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::ReadFailed`] when the column family is missing.
    #[tracing::instrument(skip_all, fields(cf_name))]
    pub fn compact_cf(&self, cf_name: &str) -> StorageResult<()> {
        let handle = self.cf_handle(cf_name)?;
        self.inner
            .compact_range_cf(&handle, None::<&[u8]>, None::<&[u8]>);
        Ok(())
    }

    fn cf_handle(&self, cf_name: &str) -> StorageResult<ColumnFamilyRef<'_>> {
        self.inner
            .cf_handle(cf_name)
            .ok_or_else(|| StorageError::ReadFailed {
                cf_name: cf_name.to_owned(),
                detail: "column family handle missing".to_owned(),
            })
    }
}

fn db_options() -> Options {
    let mut options = Options::default();
    options.create_if_missing(true);
    options.create_missing_column_families(true);
    options.set_max_background_jobs(2);
    options.set_compression_type(DBCompressionType::Lz4);
    options.set_max_open_files(256);
    options.set_keep_log_file_num(8);
    options.set_write_buffer_size(DEFAULT_WRITE_BUFFER_BYTES);
    options.set_max_write_buffer_number(3);
    options.set_target_file_size_base(DEFAULT_WRITE_BUFFER_BYTES as u64);
    options.set_level_zero_file_num_compaction_trigger(4);
    apply_block_cache(&mut options);
    options
}

fn cf_options(name: &'static str) -> Options {
    let mut options = Options::default();
    options.set_write_buffer_size(DEFAULT_WRITE_BUFFER_BYTES);
    options.set_max_write_buffer_number(3);
    options.set_target_file_size_base(DEFAULT_WRITE_BUFFER_BYTES as u64);
    options.set_level_zero_file_num_compaction_trigger(4);
    options.set_compression_type(DBCompressionType::Lz4);

    match name {
        cf::CF_EVENTS | cf::CF_ACTION_LOG | cf::CF_REFLEX_AUDIT => {
            options.set_compression_type(DBCompressionType::Lz4);
            options.set_prefix_extractor(SliceTransform::create_fixed_prefix(8));
        }
        cf::CF_MODEL_CACHE => {
            options.set_compression_type(DBCompressionType::None);
            options.set_write_buffer_size(MODEL_CACHE_WRITE_BUFFER_BYTES);
        }
        cf::CF_OBSERVATIONS | cf::CF_SESSIONS => {
            options.set_compression_type(DBCompressionType::Zstd);
        }
        _ => {}
    }

    compaction::install_ttl_filter(&mut options, name);
    apply_block_cache(&mut options);
    options
}

fn apply_block_cache(options: &mut Options) {
    let cache = Cache::new_lru_cache(BLOCK_CACHE_BYTES);
    let mut block_options = BlockBasedOptions::default();
    block_options.set_block_cache(&cache);
    options.set_block_based_table_factory(&block_options);
}

fn verify_schema_version(db: &DB, path: &Path, schema_version: u32) -> StorageResult<()> {
    db.get(SCHEMA_VERSION_KEY)
        .map_err(|source| open_failed(path, &source))?
        .map_or_else(
            || {
                db.put(SCHEMA_VERSION_KEY, schema_version.to_be_bytes())
                    .map_err(|source| open_failed(path, &source))
            },
            |value| {
                let actual = decode_schema_version(&value);
                if actual == Some(schema_version) {
                    Ok(())
                } else {
                    Err(StorageError::SchemaMismatch {
                        expected: schema_version,
                        actual: actual.unwrap_or_default(),
                    })
                }
            },
        )
}

fn decode_schema_version(value: &[u8]) -> Option<u32> {
    let bytes: [u8; 4] = value.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

fn open_failed(path: &Path, source: &rocksdb::Error) -> StorageError {
    open_failed_detail(path, source.to_string())
}

fn open_failed_detail(path: &Path, detail: String) -> StorageError {
    tracing::warn!(
        code = error_codes::STORAGE_OPEN_FAILED,
        storage_path = %path.display(),
        %detail,
        "storage open failed"
    );
    StorageError::OpenFailed {
        path: path.to_path_buf(),
        detail,
    }
}

#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod compaction_tests;
#[cfg(test)]
mod gc_tests;
#[cfg(test)]
mod open_tests;
#[cfg(test)]
mod pressure_tests;
