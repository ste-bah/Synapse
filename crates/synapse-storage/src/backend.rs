use std::{collections::BTreeMap, path::Path, str::FromStr, sync::Arc, time::Duration};

use rocksdb::{
    BlockBasedOptions, Cache, ColumnFamilyDescriptor, ColumnFamilyRef, DB, DBCompressionType,
    Direction, IteratorMode, Options, SliceTransform, WriteBatch,
};
use serde::{Deserialize, Serialize};
use synapse_core::{
    error_codes,
    retention::{DEFAULTS, RetentionTtl},
};

use crate::{
    CfEstimateMap, OwnedCfWriteBatch, RawRow, ScanWindow, StorageError, StorageResult, batch, cf,
    compaction, gc, pressure,
};

const MIB: usize = 1024 * 1024;
const DEFAULT_WRITE_BUFFER_BYTES: usize = 64 * MIB;
const MODEL_CACHE_WRITE_BUFFER_BYTES: usize = 256 * MIB;
const BLOCK_CACHE_BYTES: usize = 64 * MIB;
const SCHEMA_VERSION_KEY: &[u8] = b"__schema_version";
const TIMELINE_PERIODIC_COMPACTION_SECONDS: u64 = 86_400;
const STORAGE_WRITES_SHED_TOTAL: &str = "storage_writes_shed_total";
const STORAGE_CF_BYTES: &str = "storage_cf_bytes";
const ESTIMATE_LIVE_DATA_SIZE: &str = "rocksdb.estimate-live-data-size";
const ESTIMATE_NUM_KEYS: &str = "rocksdb.estimate-num-keys";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackendKind {
    #[default]
    RocksDb,
    Calyx,
}

impl StorageBackendKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RocksDb => "rocksdb",
            Self::Calyx => "calyx",
        }
    }

    /// Parses a daemon storage-backend config value.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::BackendInvalidConfig`] when `value` is not one
    /// of the accepted backend names.
    pub fn parse_config(value: &str) -> StorageResult<Self> {
        Self::from_str(value).map_err(|detail| StorageError::BackendInvalidConfig {
            value: value.to_owned(),
            detail,
        })
    }
}

impl FromStr for StorageBackendKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "rocksdb" => Ok(Self::RocksDb),
            "calyx" => Ok(Self::Calyx),
            other => Err(format!(
                "storage_backend must be \"rocksdb\" or \"calyx\"; got {other:?}"
            )),
        }
    }
}

pub trait StorageBackend: Send + Sync {
    fn kind(&self) -> StorageBackendKind;
    fn put_batch(&self, cf_name: &str, rows: Vec<RawRow>) -> StorageResult<()>;
    fn put_batch_pressure_bypass(&self, cf_name: &str, rows: Vec<RawRow>) -> StorageResult<()>;
    fn put_cf_batches_pressure_bypass(&self, batches: Vec<OwnedCfWriteBatch>) -> StorageResult<()>;
    fn get_cf(&self, cf_name: &str, key: &[u8]) -> StorageResult<Option<Vec<u8>>>;
    fn mutate_batch_pressure_bypass(
        &self,
        cf_name: &str,
        deletes: Vec<Vec<u8>>,
        puts: Vec<RawRow>,
    ) -> StorageResult<()>;
    fn delete_batch(&self, cf_name: &str, keys: Vec<Vec<u8>>) -> StorageResult<()>;
    fn flush(&self) -> StorageResult<()>;
    fn run_gc_once(&self) -> StorageResult<gc::GcReport>;
    fn run_gc_once_with_row_caps(
        &self,
        cf_name: &'static str,
        soft_cap_rows: u64,
        hard_cap_rows: u64,
    ) -> StorageResult<gc::GcReport>;
    fn spawn_gc_task(&self) -> StorageResult<gc::GcTask>;
    fn pressure_level(&self) -> pressure::DiskPressureLevel;
    fn pressure_permits_write(&self, cf_name: &str) -> bool;
    fn pressure_transition_codes(&self) -> StorageResult<Vec<&'static str>>;
    fn pressure_probe_readback(&self) -> StorageResult<pressure::PressureProbeReadback>;
    fn cf_sizes(&self) -> StorageResult<BTreeMap<String, u64>>;
    fn cf_live_data_size_estimates(&self) -> StorageResult<CfEstimateMap>;
    fn cf_row_counts(&self) -> StorageResult<BTreeMap<String, u64>>;
    fn cf_estimated_row_counts(&self) -> StorageResult<CfEstimateMap>;
    fn run_pressure_check_once(
        &self,
        storage_path: &Path,
    ) -> StorageResult<pressure::PressureReport>;
    fn run_pressure_check_with_free_bytes_sample(
        &self,
        free_bytes: u64,
    ) -> StorageResult<pressure::PressureReport>;
    fn spawn_pressure_task(&self, storage_path: &Path) -> StorageResult<pressure::PressureTask>;
    fn scan_cf(&self, cf_name: &str) -> StorageResult<Vec<RawRow>>;
    fn scan_cf_prefix(&self, cf_name: &str, prefix: &[u8]) -> StorageResult<Vec<RawRow>>;
    fn scan_cf_prefix_from(
        &self,
        cf_name: &str,
        prefix: &[u8],
        start_key: &[u8],
    ) -> StorageResult<Vec<RawRow>>;
    fn scan_cf_from(
        &self,
        cf_name: &str,
        start_key: &[u8],
        max_rows: usize,
    ) -> StorageResult<ScanWindow>;
    fn scan_cf_tail(&self, cf_name: &str, max_rows: usize) -> StorageResult<Vec<RawRow>>;
    fn compact_cf(&self, cf_name: &str) -> StorageResult<()>;
    fn compact_cf_range(&self, cf_name: &str, start: &[u8], end: &[u8]) -> StorageResult<()>;
}

pub struct RocksDbBackend {
    batcher: batch::Batcher,
    inner: Arc<DB>,
    pressure: Arc<pressure::PressureState>,
}

impl RocksDbBackend {
    pub fn open(path: &Path, schema_version: u32) -> StorageResult<Self> {
        let options = db_options();
        // An error here means the database does not exist yet (fresh open);
        // create_missing_column_families covers that path.
        let existing_cfs = DB::list_cf(&Options::default(), path).unwrap_or_default();
        let unknown_cfs: Vec<String> = existing_cfs
            .into_iter()
            .filter(|name| {
                name != rocksdb::DEFAULT_COLUMN_FAMILY_NAME
                    && !cf::ALL_COLUMN_FAMILIES.contains(&name.as_str())
            })
            .collect();
        for name in &unknown_cfs {
            tracing::warn!(
                code = "STORAGE_UNKNOWN_CF_OPENED",
                cf_name = %name,
                backend = StorageBackendKind::RocksDb.as_str(),
                "database holds a column family this binary does not know; \
                 opening it untouched with default options (newer-binary data, \
                 preserved for rollback safety)"
            );
        }
        let descriptors = cf::ALL_COLUMN_FAMILIES
            .into_iter()
            .map(|name| ColumnFamilyDescriptor::new(name, cf_options(name)))
            .chain(
                unknown_cfs
                    .iter()
                    .map(|name| ColumnFamilyDescriptor::new(name, Options::default())),
            );
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
            batcher,
            inner,
            pressure,
        })
    }

    fn cf_handle(&self, cf_name: &str) -> StorageResult<ColumnFamilyRef<'_>> {
        self.inner
            .cf_handle(cf_name)
            .ok_or_else(|| StorageError::ReadFailed {
                cf_name: cf_name.to_owned(),
                detail: "column family handle missing".to_owned(),
            })
    }

    fn cf_property_estimates(&self, property: &str) -> StorageResult<CfEstimateMap> {
        let mut values = BTreeMap::new();
        let mut missing = Vec::new();
        for cf_name in cf::ALL_COLUMN_FAMILIES {
            let handle = self.cf_handle(cf_name)?;
            if let Some(value) = self
                .inner
                .property_int_value_cf(&handle, property)
                .map_err(|source| StorageError::ReadFailed {
                    cf_name: cf_name.to_owned(),
                    detail: source.to_string(),
                })?
            {
                values.insert(cf_name.to_owned(), value);
            } else {
                values.insert(cf_name.to_owned(), 0);
                missing.push(cf_name.to_owned());
            }
        }
        Ok((values, missing))
    }
}

impl StorageBackend for RocksDbBackend {
    fn kind(&self) -> StorageBackendKind {
        StorageBackendKind::RocksDb
    }

    fn put_batch(&self, cf_name: &str, rows: Vec<RawRow>) -> StorageResult<()> {
        self.inner
            .cf_handle(cf_name)
            .ok_or_else(|| StorageError::WriteFailed {
                cf_name: cf_name.to_owned(),
                detail: "column family handle missing".to_owned(),
            })?;
        if rows.is_empty() {
            return Ok(());
        }
        if !self.pressure.permits_write(cf_name) {
            let pressure_level = format!("{:?}", self.pressure.level());
            synapse_telemetry::metrics::counter!(
                STORAGE_WRITES_SHED_TOTAL,
                "cf" => cf_name.to_owned()
            )
            .increment(rows.len() as u64);
            tracing::warn!(
                code = error_codes::STORAGE_WRITE_FAILED,
                cf = cf_name,
                pressure_level = ?self.pressure.level(),
                dropped_rows = rows.len(),
                metric_name = STORAGE_WRITES_SHED_TOTAL,
                backend = self.kind().as_str(),
                "storage write dropped under disk pressure"
            );
            return Err(StorageError::WriteShed {
                cf_name: cf_name.to_owned(),
                pressure_level,
                rows: rows.len(),
            });
        }
        self.batcher.put_batch(cf_name, rows)
    }

    fn put_batch_pressure_bypass(&self, cf_name: &str, rows: Vec<RawRow>) -> StorageResult<()> {
        let cf = self
            .inner
            .cf_handle(cf_name)
            .ok_or_else(|| StorageError::WriteFailed {
                cf_name: cf_name.to_owned(),
                detail: "column family handle missing".to_owned(),
            })?;
        if rows.is_empty() {
            return Ok(());
        }
        let mut batch = WriteBatch::default();
        for (key, value) in rows {
            batch.put_cf(&cf, key, value);
        }
        self.inner
            .write(batch)
            .map_err(|source| StorageError::WriteFailed {
                cf_name: cf_name.to_owned(),
                detail: source.to_string(),
            })?;
        self.inner
            .flush_cf(&cf)
            .map_err(|source| StorageError::WriteFailed {
                cf_name: cf_name.to_owned(),
                detail: source.to_string(),
            })
    }

    fn put_cf_batches_pressure_bypass(&self, batches: Vec<OwnedCfWriteBatch>) -> StorageResult<()> {
        if batches.iter().all(|(_cf_name, rows)| rows.is_empty()) {
            return Ok(());
        }
        let mut handles = Vec::with_capacity(batches.len());
        for (cf_name, _rows) in &batches {
            let handle =
                self.inner
                    .cf_handle(cf_name)
                    .ok_or_else(|| StorageError::WriteFailed {
                        cf_name: cf_name.to_owned(),
                        detail: "column family handle missing".to_owned(),
                    })?;
            handles.push(handle);
        }
        let mut batch = WriteBatch::default();
        for ((_cf_name, rows), handle) in batches.iter().zip(&handles) {
            for (key, value) in rows {
                batch.put_cf(handle, key, value);
            }
        }
        self.inner
            .write(batch)
            .map_err(|source| StorageError::WriteFailed {
                cf_name: "<multi-cf>".to_owned(),
                detail: source.to_string(),
            })?;
        for ((cf_name, _rows), handle) in batches.into_iter().zip(handles) {
            self.inner
                .flush_cf(&handle)
                .map_err(|source| StorageError::WriteFailed {
                    cf_name,
                    detail: source.to_string(),
                })?;
        }
        Ok(())
    }

    fn get_cf(&self, cf_name: &str, key: &[u8]) -> StorageResult<Option<Vec<u8>>> {
        let handle = self.cf_handle(cf_name)?;
        self.inner
            .get_cf(&handle, key)
            .map_err(|source| StorageError::ReadFailed {
                cf_name: cf_name.to_owned(),
                detail: source.to_string(),
            })
    }

    fn mutate_batch_pressure_bypass(
        &self,
        cf_name: &str,
        deletes: Vec<Vec<u8>>,
        puts: Vec<RawRow>,
    ) -> StorageResult<()> {
        let cf = self
            .inner
            .cf_handle(cf_name)
            .ok_or_else(|| StorageError::WriteFailed {
                cf_name: cf_name.to_owned(),
                detail: "column family handle missing".to_owned(),
            })?;
        if deletes.is_empty() && puts.is_empty() {
            return Ok(());
        }
        let mut batch = WriteBatch::default();
        for key in deletes {
            batch.delete_cf(&cf, key);
        }
        for (key, value) in puts {
            batch.put_cf(&cf, key, value);
        }
        self.inner
            .write(batch)
            .map_err(|source| StorageError::WriteFailed {
                cf_name: cf_name.to_owned(),
                detail: source.to_string(),
            })?;
        self.inner
            .flush_cf(&cf)
            .map_err(|source| StorageError::WriteFailed {
                cf_name: cf_name.to_owned(),
                detail: source.to_string(),
            })
    }

    fn delete_batch(&self, cf_name: &str, keys: Vec<Vec<u8>>) -> StorageResult<()> {
        let cf = self
            .inner
            .cf_handle(cf_name)
            .ok_or_else(|| StorageError::WriteFailed {
                cf_name: cf_name.to_owned(),
                detail: "column family handle missing".to_owned(),
            })?;
        if keys.is_empty() {
            return Ok(());
        }
        let mut batch = WriteBatch::default();
        for key in keys {
            batch.delete_cf(&cf, key);
        }
        self.inner
            .write(batch)
            .map_err(|source| StorageError::WriteFailed {
                cf_name: cf_name.to_owned(),
                detail: source.to_string(),
            })?;
        self.inner
            .flush_cf(&cf)
            .map_err(|source| StorageError::WriteFailed {
                cf_name: cf_name.to_owned(),
                detail: source.to_string(),
            })
    }

    fn flush(&self) -> StorageResult<()> {
        self.batcher.flush()
    }

    fn run_gc_once(&self) -> StorageResult<gc::GcReport> {
        gc::run_once(&self.inner, &gc::GcConfig::from_retention_defaults())
    }

    fn run_gc_once_with_row_caps(
        &self,
        cf_name: &'static str,
        soft_cap_rows: u64,
        hard_cap_rows: u64,
    ) -> StorageResult<gc::GcReport> {
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

    fn spawn_gc_task(&self) -> StorageResult<gc::GcTask> {
        gc::spawn(
            Arc::clone(&self.inner),
            gc::GcConfig::from_retention_defaults(),
        )
    }

    fn pressure_level(&self) -> pressure::DiskPressureLevel {
        self.pressure.level()
    }

    fn pressure_permits_write(&self, cf_name: &str) -> bool {
        self.pressure.permits_write(cf_name)
    }

    fn pressure_transition_codes(&self) -> StorageResult<Vec<&'static str>> {
        self.pressure.transition_codes()
    }

    fn pressure_probe_readback(&self) -> StorageResult<pressure::PressureProbeReadback> {
        self.pressure.probe_readback()
    }

    fn cf_sizes(&self) -> StorageResult<BTreeMap<String, u64>> {
        let mut sizes = BTreeMap::new();
        for cf_name in cf::ALL_COLUMN_FAMILIES {
            let mut bytes = 0_u64;
            for (key, value) in self.scan_cf(cf_name)? {
                bytes = bytes.saturating_add(key.len() as u64);
                bytes = bytes.saturating_add(value.len() as u64);
            }
            sizes.insert(cf_name.to_owned(), bytes);
        }
        emit_storage_cf_bytes(&sizes);
        Ok(sizes)
    }

    fn cf_live_data_size_estimates(&self) -> StorageResult<CfEstimateMap> {
        let estimates = self.cf_property_estimates(ESTIMATE_LIVE_DATA_SIZE)?;
        emit_storage_cf_bytes(&estimates.0);
        Ok(estimates)
    }

    fn cf_row_counts(&self) -> StorageResult<BTreeMap<String, u64>> {
        let mut counts = BTreeMap::new();
        for cf_name in cf::ALL_COLUMN_FAMILIES {
            counts.insert(cf_name.to_owned(), self.scan_cf(cf_name)?.len() as u64);
        }
        Ok(counts)
    }

    fn cf_estimated_row_counts(&self) -> StorageResult<CfEstimateMap> {
        self.cf_property_estimates(ESTIMATE_NUM_KEYS)
    }

    fn run_pressure_check_once(
        &self,
        storage_path: &Path,
    ) -> StorageResult<pressure::PressureReport> {
        pressure::run_once(
            &self.inner,
            &self.pressure,
            storage_path,
            &pressure::PressureConfig::default(),
        )
    }

    fn run_pressure_check_with_free_bytes_sample(
        &self,
        free_bytes: u64,
    ) -> StorageResult<pressure::PressureReport> {
        pressure::run_once_with_free_bytes(
            &self.inner,
            &self.pressure,
            &pressure::PressureConfig::default(),
            free_bytes,
        )
    }

    fn spawn_pressure_task(&self, storage_path: &Path) -> StorageResult<pressure::PressureTask> {
        pressure::spawn(
            Arc::clone(&self.inner),
            Arc::clone(&self.pressure),
            storage_path.to_path_buf(),
            pressure::PressureConfig::default(),
        )
    }

    fn scan_cf(&self, cf_name: &str) -> StorageResult<Vec<RawRow>> {
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

    fn scan_cf_prefix(&self, cf_name: &str, prefix: &[u8]) -> StorageResult<Vec<RawRow>> {
        self.scan_cf_prefix_from(cf_name, prefix, prefix)
    }

    fn scan_cf_prefix_from(
        &self,
        cf_name: &str,
        prefix: &[u8],
        start_key: &[u8],
    ) -> StorageResult<Vec<RawRow>> {
        let handle = self.cf_handle(cf_name)?;
        let mut rows = Vec::new();
        for item in self
            .inner
            .iterator_cf(&handle, IteratorMode::From(start_key, Direction::Forward))
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

    fn scan_cf_from(
        &self,
        cf_name: &str,
        start_key: &[u8],
        max_rows: usize,
    ) -> StorageResult<ScanWindow> {
        let handle = self.cf_handle(cf_name)?;
        let mut rows = Vec::new();
        let mut more = false;
        for item in self
            .inner
            .iterator_cf(&handle, IteratorMode::From(start_key, Direction::Forward))
        {
            let (key, value) = item.map_err(|source| StorageError::ReadFailed {
                cf_name: cf_name.to_owned(),
                detail: source.to_string(),
            })?;
            if rows.len() == max_rows {
                more = true;
                break;
            }
            rows.push((key.to_vec(), value.to_vec()));
        }
        Ok((rows, more))
    }

    fn scan_cf_tail(&self, cf_name: &str, max_rows: usize) -> StorageResult<Vec<RawRow>> {
        if max_rows == 0 {
            return Ok(Vec::new());
        }
        let handle = self.cf_handle(cf_name)?;
        let mut rows = Vec::new();
        for item in self.inner.iterator_cf(&handle, IteratorMode::End) {
            let (key, value) = item.map_err(|source| StorageError::ReadFailed {
                cf_name: cf_name.to_owned(),
                detail: source.to_string(),
            })?;
            rows.push((key.to_vec(), value.to_vec()));
            if rows.len() >= max_rows {
                break;
            }
        }
        rows.reverse();
        Ok(rows)
    }

    fn compact_cf(&self, cf_name: &str) -> StorageResult<()> {
        let handle = self.cf_handle(cf_name)?;
        self.inner
            .compact_range_cf(&handle, None::<&[u8]>, None::<&[u8]>);
        Ok(())
    }

    fn compact_cf_range(&self, cf_name: &str, start: &[u8], end: &[u8]) -> StorageResult<()> {
        let handle = self.cf_handle(cf_name)?;
        self.inner.compact_range_cf(&handle, Some(start), Some(end));
        Ok(())
    }
}

#[allow(clippy::cast_precision_loss)]
fn emit_storage_cf_bytes(sizes: &BTreeMap<String, u64>) {
    for (cf_name, bytes) in sizes {
        synapse_telemetry::metrics::gauge!(STORAGE_CF_BYTES, "cf" => cf_name.clone())
            .set(*bytes as f64);
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
        cf::CF_TIMELINE | cf::CF_EPISODES | cf::CF_AGENT_EVENTS => {
            options.set_compression_type(DBCompressionType::Zstd);
            options.set_prefix_extractor(SliceTransform::create_fixed_prefix(8));
            options.set_periodic_compaction_seconds(TIMELINE_PERIODIC_COMPACTION_SECONDS);
        }
        cf::CF_AGENT_TRANSCRIPTS => {
            options.set_compression_type(DBCompressionType::Zstd);
            options.set_periodic_compaction_seconds(TIMELINE_PERIODIC_COMPACTION_SECONDS);
        }
        _ => {}
    }

    if cf_has_ttl(name) {
        options.set_periodic_compaction_seconds(TIMELINE_PERIODIC_COMPACTION_SECONDS);
    }

    compaction::install_ttl_filter(&mut options, name);
    apply_block_cache(&mut options);
    options
}

fn cf_has_ttl(name: &'static str) -> bool {
    DEFAULTS
        .iter()
        .find(|default| default.cf == name)
        .is_some_and(|default| {
            matches!(default.ttl, RetentionTtl::Hours(_) | RetentionTtl::Days(_))
        })
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
        backend = StorageBackendKind::RocksDb.as_str(),
        %detail,
        "storage open failed"
    );
    StorageError::OpenFailed {
        path: path.to_path_buf(),
        detail,
    }
}
