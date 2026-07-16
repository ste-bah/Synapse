use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use calyx_aster::{
    cf::{ColumnFamily, prefix_range},
    mvcc::tombstone_value,
};
use rocksdb::{
    BlockBasedOptions, Cache, ColumnFamilyDescriptor, ColumnFamilyRef, DB, DBCompressionType,
    Direction, IteratorMode, Options, SliceTransform, WriteBatch,
};
use serde::{Deserialize, Serialize};
use synapse_calyx::{
    SynapseCalyxCfWrite, SynapseCalyxConfig, SynapseCalyxError, SynapseCalyxVault,
};
use synapse_core::{
    error_codes,
    retention::{DEFAULTS, RetentionDefault, RetentionTtl},
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
const CALYX_KV_DISC: u8 = 0x03;
const CALYX_KV_VALUE_VERSION_V1: u8 = 0x01;
const CALYX_KV_VALUE_VERSION: u8 = 0x02;
const CALYX_KV_VALUE_V1_HEADER_BYTES: usize = 1 + 8;
const CALYX_KV_VALUE_HEADER_BYTES: usize = 1 + 8 + 8;
const CALYX_KV_NAMESPACE: u64 = 0;
const CALYX_COLLECTION_ID_BASE: u64 = 0x5359_4e43_4600_0000;
const CALYX_METADATA_COLLECTION_ID: u64 = CALYX_COLLECTION_ID_BASE | 0xffff;
const CALYX_UNSUPPORTED_MAINTENANCE_DETAIL: &str = "storage_backend=\"calyx\" supports the byte-preserving Db read/write/scan/delete surface from #1656, but this maintenance API is intentionally unavailable until the Calyx pressure/GC/compaction parity issues #1658/#1659 land";
const MILLIS_PER_HOUR: u64 = 60 * 60 * 1_000;
const MILLIS_PER_DAY: u64 = 24 * MILLIS_PER_HOUR;
const MIB_U64: u64 = 1024 * 1024;

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

pub struct CalyxBackend {
    vault: Mutex<Option<SynapseCalyxVault>>,
    pressure: pressure::PressureState,
}

impl CalyxBackend {
    pub fn open(path: &Path, schema_version: u32) -> StorageResult<Self> {
        let config = SynapseCalyxConfig::from_vault_dir(path.to_path_buf());
        let vault =
            SynapseCalyxVault::open(config).map_err(|source| calyx_open_failed(path, &source))?;
        verify_calyx_schema_version(&vault, path, schema_version)?;
        Ok(Self {
            vault: Mutex::new(Some(vault)),
            pressure: pressure::PressureState::default(),
        })
    }

    #[allow(
        clippy::significant_drop_tightening,
        reason = "the mutex guard intentionally owns the single process-local Calyx vault for the whole storage operation"
    )]
    fn with_vault<T>(
        &self,
        cf_name: &str,
        operation: &'static str,
        write: bool,
        f: impl FnOnce(&SynapseCalyxVault) -> StorageResult<T>,
    ) -> StorageResult<T> {
        let guard = self.vault.lock().map_err(|poisoned| {
            calyx_operation_failed(
                cf_name,
                write,
                format!("{operation}: Calyx vault mutex poisoned: {poisoned}"),
            )
        })?;
        let Some(vault) = guard.as_ref() else {
            return Err(calyx_operation_failed(
                cf_name,
                write,
                format!("{operation}: Calyx vault handle has already been closed"),
            ));
        };
        f(vault)
    }

    fn commit_rows(&self, cf_name: &str, rows: Vec<SynapseCalyxCfWrite>) -> StorageResult<()> {
        if rows.is_empty() {
            return Ok(());
        }
        self.with_vault(cf_name, "commit Calyx KV rows", true, |vault| {
            commit_calyx_rows_to_vault(vault, cf_name, rows)
        })
    }

    fn read_all_rows(&self, cf_name: &str) -> StorageResult<Vec<RawRow>> {
        self.with_vault(cf_name, "scan Calyx KV namespace", false, |vault| {
            read_all_rows_from_vault(vault, cf_name)
        })
    }
}

impl Drop for CalyxBackend {
    fn drop(&mut self) {
        let vault = match self.vault.get_mut() {
            Ok(slot) => slot.take(),
            Err(poisoned) => {
                tracing::error!(
                    code = "STORAGE_CALYX_DROP_LOCK_POISONED",
                    error = %poisoned,
                    "Calyx storage backend mutex poisoned during drop; attempting close anyway"
                );
                poisoned.into_inner().take()
            }
        };
        if let Some(vault) = vault
            && let Err(error) = vault.close("synapse_storage_calyx_backend_drop")
        {
            tracing::error!(
                code = error.code,
                error = %error,
                "Calyx storage backend close failed during drop"
            );
        }
    }
}

impl StorageBackend for CalyxBackend {
    fn kind(&self) -> StorageBackendKind {
        StorageBackendKind::Calyx
    }

    fn put_batch(&self, cf_name: &str, rows: Vec<RawRow>) -> StorageResult<()> {
        calyx_collection_id_for_cf_write(cf_name)?;
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
        self.put_batch_pressure_bypass(cf_name, rows)
    }

    fn put_batch_pressure_bypass(&self, cf_name: &str, rows: Vec<RawRow>) -> StorageResult<()> {
        let collection_id = calyx_collection_id_for_cf_write(cf_name)?;
        self.with_vault(cf_name, "write Calyx KV batch", true, |vault| {
            let now_ms = calyx_clock_now_for_write(vault, cf_name)?;
            let rows = rows
                .into_iter()
                .map(|(key, value)| calyx_put_row(cf_name, collection_id, &key, &value, now_ms))
                .collect::<StorageResult<Vec<_>>>()?;
            preflight_calyx_retention_for_cf(vault, cf_name, collection_id, now_ms)?;
            commit_calyx_rows_to_vault(vault, cf_name, rows)?;
            enforce_calyx_retention_for_cf(vault, cf_name, now_ms)
        })
    }

    fn put_cf_batches_pressure_bypass(&self, batches: Vec<OwnedCfWriteBatch>) -> StorageResult<()> {
        if batches.iter().all(|(_cf_name, rows)| rows.is_empty()) {
            return Ok(());
        }
        self.with_vault(
            "<multi-cf>",
            "write Calyx multi-CF KV batch",
            true,
            |vault| {
                let now_ms = calyx_clock_now_for_write(vault, "<multi-cf>")?;
                let mut writes = Vec::new();
                let mut affected_cfs = BTreeSet::new();
                for (cf_name, rows) in batches {
                    let collection_id = calyx_collection_id_for_cf_write(&cf_name)?;
                    if !rows.is_empty() {
                        affected_cfs.insert(cf_name.clone());
                    }
                    for (key, value) in rows {
                        writes.push(calyx_put_row(
                            &cf_name,
                            collection_id,
                            &key,
                            &value,
                            now_ms,
                        )?);
                    }
                }
                for cf_name in &affected_cfs {
                    preflight_calyx_retention_for_cf(
                        vault,
                        cf_name,
                        calyx_collection_id_for_cf_write(cf_name)?,
                        now_ms,
                    )?;
                }
                commit_calyx_rows_to_vault(vault, "<multi-cf>", writes)?;
                for cf_name in affected_cfs {
                    enforce_calyx_retention_for_cf(vault, &cf_name, now_ms)?;
                }
                Ok(())
            },
        )
    }

    fn get_cf(&self, cf_name: &str, key: &[u8]) -> StorageResult<Option<Vec<u8>>> {
        self.with_vault(cf_name, "read Calyx KV row", false, |vault| {
            let collection_id = calyx_collection_id_for_cf_read(cf_name)?;
            let key = encode_calyx_key_for_read(cf_name, collection_id, key)?;
            let snapshot = latest_calyx_seq(vault);
            let value = vault
                .read_cf_at(snapshot, ColumnFamily::Kv, &key)
                .map_err(|source| calyx_read_failed(cf_name, "read Calyx CF row", &source))?;
            let now_ms = calyx_clock_now_for_read(vault, cf_name)?;
            value.map_or(Ok(None), |bytes| {
                decode_calyx_value_for_read(cf_name, &bytes, now_ms)
            })
        })
    }

    fn mutate_batch_pressure_bypass(
        &self,
        cf_name: &str,
        deletes: Vec<Vec<u8>>,
        puts: Vec<RawRow>,
    ) -> StorageResult<()> {
        let collection_id = calyx_collection_id_for_cf_write(cf_name)?;
        self.with_vault(cf_name, "mutate Calyx KV batch", true, |vault| {
            let now_ms = calyx_clock_now_for_write(vault, cf_name)?;
            let put_count = puts.len();
            let mut rows = Vec::with_capacity(deletes.len().saturating_add(put_count));
            for key in deletes {
                rows.push(calyx_delete_row(cf_name, collection_id, &key)?);
            }
            for (key, value) in puts {
                rows.push(calyx_put_row(cf_name, collection_id, &key, &value, now_ms)?);
            }
            if put_count > 0 {
                preflight_calyx_retention_for_cf(vault, cf_name, collection_id, now_ms)?;
            }
            commit_calyx_rows_to_vault(vault, cf_name, rows)?;
            if put_count == 0 {
                return Ok(());
            }
            enforce_calyx_retention_for_cf(vault, cf_name, now_ms)
        })
    }

    fn delete_batch(&self, cf_name: &str, keys: Vec<Vec<u8>>) -> StorageResult<()> {
        let collection_id = calyx_collection_id_for_cf_write(cf_name)?;
        let rows = keys
            .into_iter()
            .map(|key| calyx_delete_row(cf_name, collection_id, &key))
            .collect::<StorageResult<Vec<_>>>()?;
        self.commit_rows(cf_name, rows)
    }

    fn flush(&self) -> StorageResult<()> {
        self.with_vault("<all>", "flush Calyx vault", true, |vault| {
            vault
                .flush()
                .map_err(|source| calyx_write_failed("<all>", "flush Calyx vault", &source))
        })
    }

    fn run_gc_once(&self) -> StorageResult<gc::GcReport> {
        Err(calyx_unsupported_maintenance("run_gc_once"))
    }

    fn run_gc_once_with_row_caps(
        &self,
        _cf_name: &'static str,
        _soft_cap_rows: u64,
        _hard_cap_rows: u64,
    ) -> StorageResult<gc::GcReport> {
        Err(calyx_unsupported_maintenance("run_gc_once_with_row_caps"))
    }

    fn spawn_gc_task(&self) -> StorageResult<gc::GcTask> {
        Err(calyx_unsupported_maintenance("spawn_gc_task"))
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
            for (key, value) in self.read_all_rows(cf_name)? {
                bytes = bytes.saturating_add(key.len() as u64);
                bytes = bytes.saturating_add(value.len() as u64);
            }
            sizes.insert(cf_name.to_owned(), bytes);
        }
        emit_storage_cf_bytes(&sizes);
        Ok(sizes)
    }

    fn cf_live_data_size_estimates(&self) -> StorageResult<CfEstimateMap> {
        let sizes = self.cf_sizes()?;
        Ok((sizes, Vec::new()))
    }

    fn cf_row_counts(&self) -> StorageResult<BTreeMap<String, u64>> {
        let mut counts = BTreeMap::new();
        for cf_name in cf::ALL_COLUMN_FAMILIES {
            counts.insert(
                cf_name.to_owned(),
                self.read_all_rows(cf_name)?.len() as u64,
            );
        }
        Ok(counts)
    }

    fn cf_estimated_row_counts(&self) -> StorageResult<CfEstimateMap> {
        let counts = self.cf_row_counts()?;
        Ok((counts, Vec::new()))
    }

    fn run_pressure_check_once(
        &self,
        _storage_path: &Path,
    ) -> StorageResult<pressure::PressureReport> {
        Err(calyx_unsupported_maintenance("run_pressure_check_once"))
    }

    fn run_pressure_check_with_free_bytes_sample(
        &self,
        _free_bytes: u64,
    ) -> StorageResult<pressure::PressureReport> {
        Err(calyx_unsupported_maintenance(
            "run_pressure_check_with_free_bytes_sample",
        ))
    }

    fn spawn_pressure_task(&self, _storage_path: &Path) -> StorageResult<pressure::PressureTask> {
        Err(calyx_unsupported_maintenance("spawn_pressure_task"))
    }

    fn scan_cf(&self, cf_name: &str) -> StorageResult<Vec<RawRow>> {
        self.read_all_rows(cf_name)
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
        let rows = self.read_all_rows(cf_name)?;
        Ok(rows
            .into_iter()
            .filter(|(key, _value)| key.as_slice() >= start_key && key.starts_with(prefix))
            .collect())
    }

    fn scan_cf_from(
        &self,
        cf_name: &str,
        start_key: &[u8],
        max_rows: usize,
    ) -> StorageResult<ScanWindow> {
        let mut rows = self
            .read_all_rows(cf_name)?
            .into_iter()
            .filter(|(key, _value)| key.as_slice() >= start_key);
        let mut window = Vec::new();
        let mut more = false;
        for row in &mut rows {
            if window.len() == max_rows {
                more = true;
                break;
            }
            window.push(row);
        }
        Ok((window, more))
    }

    fn scan_cf_tail(&self, cf_name: &str, max_rows: usize) -> StorageResult<Vec<RawRow>> {
        if max_rows == 0 {
            return Ok(Vec::new());
        }
        let mut rows = self.read_all_rows(cf_name)?;
        if rows.len() > max_rows {
            rows.drain(0..rows.len() - max_rows);
        }
        Ok(rows)
    }

    fn compact_cf(&self, _cf_name: &str) -> StorageResult<()> {
        Err(calyx_unsupported_maintenance("compact_cf"))
    }

    fn compact_cf_range(&self, _cf_name: &str, _start: &[u8], _end: &[u8]) -> StorageResult<()> {
        Err(calyx_unsupported_maintenance("compact_cf_range"))
    }
}

#[allow(clippy::cast_precision_loss)]
fn emit_storage_cf_bytes(sizes: &BTreeMap<String, u64>) {
    for (cf_name, bytes) in sizes {
        synapse_telemetry::metrics::gauge!(STORAGE_CF_BYTES, "cf" => cf_name.clone())
            .set(*bytes as f64);
    }
}

fn verify_calyx_schema_version(
    vault: &SynapseCalyxVault,
    path: &Path,
    schema_version: u32,
) -> StorageResult<()> {
    let key = encode_calyx_key(CALYX_METADATA_COLLECTION_ID, SCHEMA_VERSION_KEY)
        .map_err(|detail| calyx_open_failed_detail(path, detail))?;
    let snapshot = latest_calyx_seq(vault);
    let existing = vault
        .read_cf_at(snapshot, ColumnFamily::Kv, &key)
        .map_err(|source| calyx_open_failed_detail(path, source.to_string()))?;
    match existing {
        None => {
            let row = SynapseCalyxCfWrite::new(
                ColumnFamily::Kv,
                key,
                encode_calyx_value(0, 0, &schema_version.to_be_bytes()),
            );
            vault
                .write_cf_batch(vec![row])
                .map_err(|source| calyx_open_failed_detail(path, source.to_string()))?;
            vault
                .flush()
                .map_err(|source| calyx_open_failed_detail(path, source.to_string()))
        }
        Some(value) => {
            let payload = decode_calyx_value_raw(&value)
                .map_err(|detail| calyx_open_failed_detail(path, detail))?;
            let actual = decode_schema_version(payload.payload);
            if actual == Some(schema_version) {
                Ok(())
            } else {
                Err(StorageError::SchemaMismatch {
                    expected: schema_version,
                    actual: actual.unwrap_or_default(),
                })
            }
        }
    }
}

fn read_all_rows_from_vault(
    vault: &SynapseCalyxVault,
    cf_name: &str,
) -> StorageResult<Vec<RawRow>> {
    let collection_id = calyx_collection_id_for_cf_read(cf_name)?;
    let range = prefix_range(&calyx_namespace_prefix(collection_id));
    let snapshot = latest_calyx_seq(vault);
    let rows = vault
        .scan_cf_range_at(snapshot, ColumnFamily::Kv, &range)
        .map_err(|source| calyx_read_failed(cf_name, "scan Calyx KV namespace", &source))?;
    let mut decoded = Vec::with_capacity(rows.len());
    let now_ms = calyx_clock_now_for_read(vault, cf_name)?;
    for (key, value) in rows {
        let user_key = decode_calyx_user_key_for_read(cf_name, collection_id, &key)?;
        if let Some(payload) = decode_calyx_value_for_read(cf_name, &value, now_ms)? {
            decoded.push((user_key, payload));
        }
    }
    decoded.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(decoded)
}

fn latest_calyx_seq(vault: &SynapseCalyxVault) -> u64 {
    vault.status().latest_seq.unwrap_or_default()
}

fn commit_calyx_rows_to_vault(
    vault: &SynapseCalyxVault,
    cf_name: &str,
    rows: Vec<SynapseCalyxCfWrite>,
) -> StorageResult<()> {
    if rows.is_empty() {
        return Ok(());
    }
    vault
        .write_cf_batch(rows)
        .map_err(|source| calyx_write_failed(cf_name, "write Calyx CF batch", &source))?;
    vault
        .flush()
        .map_err(|source| calyx_write_failed(cf_name, "flush Calyx CF batch", &source))
}

fn preflight_calyx_retention_for_cf(
    vault: &SynapseCalyxVault,
    cf_name: &str,
    collection_id: u64,
    now_ms: u64,
) -> StorageResult<()> {
    let protected = calyx_cf_protected_from_auto_delete(cf_name);
    collect_calyx_retention_state(vault, cf_name, collection_id, now_ms, protected).map(|_| ())
}

fn calyx_clock_now_for_read(vault: &SynapseCalyxVault, cf_name: &str) -> StorageResult<u64> {
    vault
        .clock_now_ms()
        .map_err(|source| calyx_read_failed(cf_name, "read Calyx vault clock", &source))
}

fn calyx_clock_now_for_write(vault: &SynapseCalyxVault, cf_name: &str) -> StorageResult<u64> {
    vault
        .clock_now_ms()
        .map_err(|source| calyx_write_failed(cf_name, "read Calyx vault clock", &source))
}

fn calyx_collection_id_for_cf(cf_name: &str) -> Option<u64> {
    for (offset, known_cf) in (1_u64..).zip(cf::ALL_COLUMN_FAMILIES) {
        if cf_name == known_cf {
            return Some(CALYX_COLLECTION_ID_BASE | offset);
        }
    }
    None
}

fn calyx_collection_id_for_cf_read(cf_name: &str) -> StorageResult<u64> {
    calyx_collection_id_for_cf(cf_name).ok_or_else(|| StorageError::ReadFailed {
        cf_name: cf_name.to_owned(),
        detail: "column family name is not part of the Synapse storage schema".to_owned(),
    })
}

fn calyx_collection_id_for_cf_write(cf_name: &str) -> StorageResult<u64> {
    calyx_collection_id_for_cf(cf_name).ok_or_else(|| StorageError::WriteFailed {
        cf_name: cf_name.to_owned(),
        detail: "column family name is not part of the Synapse storage schema".to_owned(),
    })
}

#[derive(Debug)]
struct CalyxRetentionLiveEntry {
    full_key: Vec<u8>,
    user_key: Vec<u8>,
    live_bytes: u64,
    written_at_ms: u64,
}

#[derive(Debug)]
struct CalyxRetentionState {
    live_entries: Vec<CalyxRetentionLiveEntry>,
    tombstones: Vec<SynapseCalyxCfWrite>,
    before_live_bytes: u64,
    expired_rows: u64,
}

#[derive(Clone, Copy, Debug)]
struct CalyxRetentionCaps {
    soft_cap_bytes: u64,
    hard_cap_bytes: u64,
    protected: bool,
}

#[derive(Clone, Copy, Debug)]
struct CalyxRetentionOutcome {
    after_live_bytes: u64,
    cap_evicted_rows: u64,
    hard_cap_reached: bool,
}

fn enforce_calyx_retention_for_cf(
    vault: &SynapseCalyxVault,
    cf_name: &str,
    now_ms: u64,
) -> StorageResult<()> {
    let retention = calyx_retention_default_for_write(cf_name)?;
    let (soft_cap_bytes, hard_cap_bytes) = calyx_retention_cap_bytes_for_write(retention)?;
    let collection_id = calyx_collection_id_for_cf_write(cf_name)?;
    let caps = CalyxRetentionCaps {
        soft_cap_bytes,
        hard_cap_bytes,
        protected: calyx_cf_protected_from_auto_delete(cf_name),
    };
    let mut state =
        collect_calyx_retention_state(vault, cf_name, collection_id, now_ms, caps.protected)?;
    let hard_cap_reached = log_calyx_hard_cap_if_reached(
        cf_name,
        state.before_live_bytes,
        caps.soft_cap_bytes,
        caps.hard_cap_bytes,
        caps.protected,
    );
    let (after_live_bytes, cap_evicted_rows) = apply_calyx_cap_eviction(
        cf_name,
        &mut state,
        caps.soft_cap_bytes,
        caps.hard_cap_bytes,
        caps.protected,
    )?;
    let outcome = CalyxRetentionOutcome {
        after_live_bytes,
        cap_evicted_rows,
        hard_cap_reached,
    };

    if !state.tombstones.is_empty() {
        let tombstones = std::mem::take(&mut state.tombstones);
        commit_calyx_rows_to_vault(vault, cf_name, tombstones)?;
    }
    emit_calyx_retention_enforced(cf_name, &state, caps, outcome);
    Ok(())
}

fn collect_calyx_retention_state(
    vault: &SynapseCalyxVault,
    cf_name: &str,
    collection_id: u64,
    now_ms: u64,
    protected: bool,
) -> StorageResult<CalyxRetentionState> {
    let range = prefix_range(&calyx_namespace_prefix(collection_id));
    let rows = vault
        .scan_cf_range_at(latest_calyx_seq(vault), ColumnFamily::Kv, &range)
        .map_err(|source| {
            calyx_write_failed(
                cf_name,
                "scan Calyx KV namespace for retention enforcement",
                &source,
            )
        })?;
    let mut state = CalyxRetentionState {
        live_entries: Vec::new(),
        tombstones: Vec::new(),
        before_live_bytes: 0,
        expired_rows: 0,
    };
    for (full_key, value) in rows {
        let user_key = decode_calyx_user_key(collection_id, &full_key).map_err(|detail| {
            calyx_write_failed_detail(
                cf_name,
                format!("decode Calyx retention scan key: {detail}"),
            )
        })?;
        let envelope = decode_calyx_value_raw(&value).map_err(|detail| {
            tracing::error!(
                code = error_codes::STORAGE_WRITE_FAILED,
                cf = cf_name,
                detail,
                "Calyx storage backend rejected malformed KV retention envelope during enforcement"
            );
            calyx_write_failed_detail(
                cf_name,
                format!("decode Calyx retention envelope: {detail}"),
            )
        })?;
        if calyx_value_is_expired(envelope.expires_at_ms, now_ms) {
            if !protected {
                state.tombstones.push(SynapseCalyxCfWrite::new(
                    ColumnFamily::Kv,
                    full_key,
                    tombstone_value(),
                ));
                state.expired_rows = state.expired_rows.saturating_add(1);
            }
            continue;
        }
        let live_bytes = calyx_live_row_bytes(cf_name, &user_key, envelope.payload)?;
        state.before_live_bytes =
            state
                .before_live_bytes
                .checked_add(live_bytes)
                .ok_or_else(|| {
                    calyx_write_failed_detail(
                        cf_name,
                        format!("Calyx retention live-byte accounting overflow in {cf_name}"),
                    )
                })?;
        state.live_entries.push(CalyxRetentionLiveEntry {
            full_key,
            user_key,
            live_bytes,
            written_at_ms: envelope.written_at_ms,
        });
    }
    Ok(state)
}

fn log_calyx_hard_cap_if_reached(
    cf_name: &str,
    before_live_bytes: u64,
    soft_cap_bytes: u64,
    hard_cap_bytes: u64,
    protected: bool,
) -> bool {
    let hard_cap_reached = before_live_bytes >= hard_cap_bytes;
    if hard_cap_reached {
        tracing::warn!(
            code = error_codes::STORAGE_CF_HARD_CAP_REACHED,
            cf = cf_name,
            before_live_bytes,
            soft_cap_bytes,
            hard_cap_bytes,
            protected,
            "Calyx retention hard cap reached"
        );
    }
    hard_cap_reached
}

fn apply_calyx_cap_eviction(
    cf_name: &str,
    state: &mut CalyxRetentionState,
    soft_cap_bytes: u64,
    hard_cap_bytes: u64,
    protected: bool,
) -> StorageResult<(u64, u64)> {
    let before_live_bytes = state.before_live_bytes;
    let mut after_live_bytes = state.before_live_bytes;
    let mut cap_evicted_rows = 0_u64;
    if protected && before_live_bytes > soft_cap_bytes {
        tracing::warn!(
            code = "STORAGE_CALYX_RETENTION_PROTECTED_CAP_SKIPPED",
            cf = cf_name,
            before_live_bytes,
            soft_cap_bytes,
            hard_cap_bytes,
            "Calyx retention skipped cap eviction for protected operator-owned column family"
        );
    } else if before_live_bytes > soft_cap_bytes {
        state.live_entries.sort_by(|left, right| {
            left.written_at_ms
                .cmp(&right.written_at_ms)
                .then_with(|| left.user_key.cmp(&right.user_key))
        });
        for entry in state.live_entries.drain(..) {
            if after_live_bytes <= soft_cap_bytes {
                break;
            }
            after_live_bytes = after_live_bytes.checked_sub(entry.live_bytes).ok_or_else(|| {
                calyx_write_failed_detail(
                    cf_name,
                    format!(
                        "Calyx retention byte accounting underflow while evicting {} bytes from {cf_name}",
                        entry.live_bytes
                    ),
                )
            })?;
            state.tombstones.push(SynapseCalyxCfWrite::new(
                ColumnFamily::Kv,
                entry.full_key,
                tombstone_value(),
            ));
            cap_evicted_rows = cap_evicted_rows.saturating_add(1);
        }
        if before_live_bytes > hard_cap_bytes && after_live_bytes > hard_cap_bytes {
            let detail = format!(
                "Calyx retention could not reduce {cf_name} below hard cap: before_live_bytes={before_live_bytes} after_live_bytes={after_live_bytes} hard_cap_bytes={hard_cap_bytes}"
            );
            tracing::error!(
                code = error_codes::STORAGE_WRITE_FAILED,
                cf = cf_name,
                before_live_bytes,
                after_live_bytes,
                hard_cap_bytes,
                detail,
                "Calyx retention hard cap enforcement failed"
            );
            return Err(calyx_write_failed_detail(cf_name, detail));
        }
    }
    Ok((after_live_bytes, cap_evicted_rows))
}

fn emit_calyx_retention_enforced(
    cf_name: &str,
    state: &CalyxRetentionState,
    caps: CalyxRetentionCaps,
    outcome: CalyxRetentionOutcome,
) {
    if state.expired_rows > 0 || outcome.cap_evicted_rows > 0 || outcome.hard_cap_reached {
        tracing::info!(
            code = "STORAGE_CALYX_RETENTION_ENFORCED",
            cf = cf_name,
            expired_rows = state.expired_rows,
            cap_evicted_rows = outcome.cap_evicted_rows,
            before_live_bytes = state.before_live_bytes,
            after_live_bytes = outcome.after_live_bytes,
            soft_cap_bytes = caps.soft_cap_bytes,
            hard_cap_bytes = caps.hard_cap_bytes,
            protected = caps.protected,
            "Calyx retention enforcement completed"
        );
    }
    if outcome.cap_evicted_rows > 0 {
        synapse_telemetry::metrics::counter!(
            "cache_evictions_total",
            "cf" => cf_name.to_owned(),
            "reason" => "soft_cap"
        )
        .increment(outcome.cap_evicted_rows);
    }
}

fn calyx_retention_default_for_write(cf_name: &str) -> StorageResult<RetentionDefault> {
    DEFAULTS
        .iter()
        .copied()
        .find(|default| default.cf == cf_name)
        .ok_or_else(|| {
            calyx_write_failed_detail(
                cf_name,
                format!("missing RetentionDefault mapping for Calyx column family {cf_name}"),
            )
        })
}

fn calyx_retention_cap_bytes_for_write(retention: RetentionDefault) -> StorageResult<(u64, u64)> {
    let soft = retention.soft_cap_mb.checked_mul(MIB_U64).ok_or_else(|| {
        calyx_write_failed_detail(
            retention.cf,
            format!(
                "soft cap overflow for {}: {} MiB",
                retention.cf, retention.soft_cap_mb
            ),
        )
    })?;
    let hard = retention.hard_cap_mb.checked_mul(MIB_U64).ok_or_else(|| {
        calyx_write_failed_detail(
            retention.cf,
            format!(
                "hard cap overflow for {}: {} MiB",
                retention.cf, retention.hard_cap_mb
            ),
        )
    })?;
    if hard < soft {
        return Err(calyx_write_failed_detail(
            retention.cf,
            format!(
                "invalid RetentionDefault for {}: hard cap {} bytes is below soft cap {} bytes",
                retention.cf, hard, soft
            ),
        ));
    }
    Ok((soft, hard))
}

fn calyx_expires_at_ms_for_write(cf_name: &str, now_ms: u64) -> StorageResult<u64> {
    let retention = calyx_retention_default_for_write(cf_name)?;
    let Some(ttl_ms) = calyx_ttl_millis_for_write(cf_name, retention.ttl)? else {
        return Ok(0);
    };
    now_ms.checked_add(ttl_ms).ok_or_else(|| {
        calyx_write_failed_detail(
            cf_name,
            format!("Calyx retention expires_at_ms overflow: now_ms={now_ms} ttl_ms={ttl_ms}"),
        )
    })
}

fn calyx_ttl_millis_for_write(cf_name: &str, ttl: RetentionTtl) -> StorageResult<Option<u64>> {
    match ttl {
        RetentionTtl::None | RetentionTtl::LruOnly => Ok(None),
        RetentionTtl::Hours(hours) => {
            if hours == 0 {
                return Err(calyx_write_failed_detail(
                    cf_name,
                    "RetentionDefault Hours(0) is invalid for Calyx TTL".to_owned(),
                ));
            }
            hours.checked_mul(MILLIS_PER_HOUR).map(Some).ok_or_else(|| {
                calyx_write_failed_detail(
                    cf_name,
                    format!("Calyx retention Hours({hours}) overflows milliseconds"),
                )
            })
        }
        RetentionTtl::Days(days) => {
            if days == 0 {
                return Err(calyx_write_failed_detail(
                    cf_name,
                    "RetentionDefault Days(0) is invalid for Calyx TTL".to_owned(),
                ));
            }
            days.checked_mul(MILLIS_PER_DAY).map(Some).ok_or_else(|| {
                calyx_write_failed_detail(
                    cf_name,
                    format!("Calyx retention Days({days}) overflows milliseconds"),
                )
            })
        }
    }
}

fn calyx_live_row_bytes(cf_name: &str, user_key: &[u8], payload: &[u8]) -> StorageResult<u64> {
    let key_bytes = u64::try_from(user_key.len()).map_err(|_error| {
        calyx_write_failed_detail(
            cf_name,
            format!(
                "Calyx retention key length does not fit in u64: {}",
                user_key.len()
            ),
        )
    })?;
    let payload_bytes = u64::try_from(payload.len()).map_err(|_error| {
        calyx_write_failed_detail(
            cf_name,
            format!(
                "Calyx retention payload length does not fit in u64: {}",
                payload.len()
            ),
        )
    })?;
    key_bytes.checked_add(payload_bytes).ok_or_else(|| {
        calyx_write_failed_detail(
            cf_name,
            format!(
                "Calyx retention row byte accounting overflow: key_bytes={key_bytes} payload_bytes={payload_bytes}"
            ),
        )
    })
}

fn calyx_cf_protected_from_auto_delete(cf_name: &str) -> bool {
    cf_name == cf::CF_ROUTINE_STATE
}

fn calyx_put_row(
    cf_name: &str,
    collection_id: u64,
    key: &[u8],
    value: &[u8],
    now_ms: u64,
) -> StorageResult<SynapseCalyxCfWrite> {
    let expires_at_ms = calyx_expires_at_ms_for_write(cf_name, now_ms)?;
    Ok(SynapseCalyxCfWrite::new(
        ColumnFamily::Kv,
        encode_calyx_key_for_write(cf_name, collection_id, key)?,
        encode_calyx_value(expires_at_ms, now_ms, value),
    ))
}

fn calyx_delete_row(
    cf_name: &str,
    collection_id: u64,
    key: &[u8],
) -> StorageResult<SynapseCalyxCfWrite> {
    Ok(SynapseCalyxCfWrite::new(
        ColumnFamily::Kv,
        encode_calyx_key_for_write(cf_name, collection_id, key)?,
        tombstone_value(),
    ))
}

fn encode_calyx_key_for_read(
    cf_name: &str,
    collection_id: u64,
    user_key: &[u8],
) -> StorageResult<Vec<u8>> {
    encode_calyx_key(collection_id, user_key).map_err(|detail| StorageError::ReadFailed {
        cf_name: cf_name.to_owned(),
        detail,
    })
}

fn encode_calyx_key_for_write(
    cf_name: &str,
    collection_id: u64,
    user_key: &[u8],
) -> StorageResult<Vec<u8>> {
    encode_calyx_key(collection_id, user_key).map_err(|detail| StorageError::WriteFailed {
        cf_name: cf_name.to_owned(),
        detail,
    })
}

fn encode_calyx_key(collection_id: u64, user_key: &[u8]) -> Result<Vec<u8>, String> {
    let user_key_len = u16::try_from(user_key.len()).map_err(|_error| {
        format!(
            "Calyx Synapse KV envelope supports keys up to {} bytes; got {}",
            u16::MAX,
            user_key.len()
        )
    })?;
    let mut key = calyx_namespace_prefix(collection_id);
    key.extend_from_slice(&user_key_len.to_be_bytes());
    key.extend_from_slice(user_key);
    Ok(key)
}

fn calyx_namespace_prefix(collection_id: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(1 + 8 + 8);
    key.push(CALYX_KV_DISC);
    key.extend_from_slice(&collection_id.to_be_bytes());
    key.extend_from_slice(&CALYX_KV_NAMESPACE.to_be_bytes());
    key
}

fn decode_calyx_user_key_for_read(
    cf_name: &str,
    collection_id: u64,
    full_key: &[u8],
) -> StorageResult<Vec<u8>> {
    decode_calyx_user_key(collection_id, full_key).map_err(|detail| StorageError::ReadFailed {
        cf_name: cf_name.to_owned(),
        detail,
    })
}

fn decode_calyx_user_key(collection_id: u64, full_key: &[u8]) -> Result<Vec<u8>, String> {
    let prefix = calyx_namespace_prefix(collection_id);
    let Some(rest) = full_key.strip_prefix(prefix.as_slice()) else {
        return Err("Calyx KV scan returned a key outside the requested namespace".to_owned());
    };
    let Some(len_bytes) = rest.get(0..2) else {
        return Err("Calyx KV key is missing its user-key length prefix".to_owned());
    };
    let len = usize::from(u16::from_be_bytes([len_bytes[0], len_bytes[1]]));
    let Some(user_key) = rest.get(2..2 + len) else {
        return Err("Calyx KV key length prefix exceeds the stored key".to_owned());
    };
    if rest.len() != 2 + len {
        return Err("Calyx KV key has trailing bytes after the user key".to_owned());
    }
    Ok(user_key.to_vec())
}

fn encode_calyx_value(expires_at_ms: u64, written_at_ms: u64, payload: &[u8]) -> Vec<u8> {
    let mut value = Vec::with_capacity(CALYX_KV_VALUE_HEADER_BYTES + payload.len());
    value.push(CALYX_KV_VALUE_VERSION);
    value.extend_from_slice(&expires_at_ms.to_be_bytes());
    value.extend_from_slice(&written_at_ms.to_be_bytes());
    value.extend_from_slice(payload);
    value
}

fn decode_calyx_value_for_read(
    cf_name: &str,
    value: &[u8],
    now_ms: u64,
) -> StorageResult<Option<Vec<u8>>> {
    let envelope = decode_calyx_value_raw(value).map_err(|detail| {
        tracing::error!(
            code = error_codes::STORAGE_READ_FAILED,
            cf = cf_name,
            detail,
            "Calyx storage backend rejected malformed KV retention envelope"
        );
        StorageError::ReadFailed {
            cf_name: cf_name.to_owned(),
            detail,
        }
    })?;
    if calyx_value_is_expired(envelope.expires_at_ms, now_ms) {
        return Ok(None);
    }
    Ok(Some(envelope.payload.to_vec()))
}

#[derive(Debug)]
struct CalyxValueEnvelope<'a> {
    expires_at_ms: u64,
    written_at_ms: u64,
    payload: &'a [u8],
}

fn decode_calyx_value_raw(value: &[u8]) -> Result<CalyxValueEnvelope<'_>, String> {
    let Some(version) = value.first().copied() else {
        return Err(
            "Calyx KV value is empty and missing its retention envelope version".to_owned(),
        );
    };
    match version {
        CALYX_KV_VALUE_VERSION_V1 => decode_calyx_value_v1(value),
        CALYX_KV_VALUE_VERSION => decode_calyx_value_v2(value),
        other => Err(format!(
            "Calyx KV value version {other} is unsupported; expected {CALYX_KV_VALUE_VERSION_V1} or {CALYX_KV_VALUE_VERSION}"
        )),
    }
}

fn decode_calyx_value_v1(value: &[u8]) -> Result<CalyxValueEnvelope<'_>, String> {
    if value.len() < CALYX_KV_VALUE_V1_HEADER_BYTES {
        return Err(format!(
            "Calyx KV v1 value is shorter than its {CALYX_KV_VALUE_V1_HEADER_BYTES} byte header"
        ));
    }
    let mut expires_at_bytes = [0_u8; 8];
    expires_at_bytes.copy_from_slice(&value[1..9]);
    Ok(CalyxValueEnvelope {
        expires_at_ms: u64::from_be_bytes(expires_at_bytes),
        written_at_ms: 0,
        payload: &value[CALYX_KV_VALUE_V1_HEADER_BYTES..],
    })
}

fn decode_calyx_value_v2(value: &[u8]) -> Result<CalyxValueEnvelope<'_>, String> {
    if value.len() < CALYX_KV_VALUE_HEADER_BYTES {
        return Err(format!(
            "Calyx KV v2 value is shorter than its {CALYX_KV_VALUE_HEADER_BYTES} byte header"
        ));
    }
    let mut expires_at_bytes = [0_u8; 8];
    expires_at_bytes.copy_from_slice(&value[1..9]);
    let mut written_at_bytes = [0_u8; 8];
    written_at_bytes.copy_from_slice(&value[9..17]);
    Ok(CalyxValueEnvelope {
        expires_at_ms: u64::from_be_bytes(expires_at_bytes),
        written_at_ms: u64::from_be_bytes(written_at_bytes),
        payload: &value[CALYX_KV_VALUE_HEADER_BYTES..],
    })
}

const fn calyx_value_is_expired(expires_at_ms: u64, now_ms: u64) -> bool {
    expires_at_ms != 0 && now_ms >= expires_at_ms
}

fn calyx_read_failed(
    cf_name: &str,
    action: &'static str,
    source: &SynapseCalyxError,
) -> StorageError {
    tracing::error!(
        code = source.code,
        source_code = source.source_code.unwrap_or("none"),
        remediation = source.remediation,
        cf_name,
        action,
        error = %source,
        "Calyx storage backend read failed"
    );
    StorageError::ReadFailed {
        cf_name: cf_name.to_owned(),
        detail: format!("{action}: {source}"),
    }
}

fn calyx_write_failed(
    cf_name: &str,
    action: &'static str,
    source: &SynapseCalyxError,
) -> StorageError {
    tracing::error!(
        code = source.code,
        source_code = source.source_code.unwrap_or("none"),
        remediation = source.remediation,
        cf_name,
        action,
        error = %source,
        "Calyx storage backend write failed"
    );
    StorageError::WriteFailed {
        cf_name: cf_name.to_owned(),
        detail: format!("{action}: {source}"),
    }
}

fn calyx_write_failed_detail(cf_name: &str, detail: impl Into<String>) -> StorageError {
    let detail = detail.into();
    tracing::error!(
        code = error_codes::STORAGE_WRITE_FAILED,
        cf_name,
        detail,
        "Calyx storage backend write failed"
    );
    StorageError::WriteFailed {
        cf_name: cf_name.to_owned(),
        detail,
    }
}

fn calyx_operation_failed(cf_name: &str, write: bool, detail: String) -> StorageError {
    if write {
        StorageError::WriteFailed {
            cf_name: cf_name.to_owned(),
            detail,
        }
    } else {
        StorageError::ReadFailed {
            cf_name: cf_name.to_owned(),
            detail,
        }
    }
}

fn calyx_unsupported_maintenance(operation: &'static str) -> StorageError {
    tracing::error!(
        code = error_codes::STORAGE_BACKEND_UNIMPLEMENTED,
        backend = StorageBackendKind::Calyx.as_str(),
        operation,
        detail = CALYX_UNSUPPORTED_MAINTENANCE_DETAIL,
        "Calyx storage backend maintenance API is unavailable"
    );
    StorageError::BackendUnavailable {
        backend: StorageBackendKind::Calyx.as_str().to_owned(),
        detail: format!("{operation}: {CALYX_UNSUPPORTED_MAINTENANCE_DETAIL}"),
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
    open_failed_detail_with_backend(path, StorageBackendKind::RocksDb, detail)
}

fn calyx_open_failed(path: &Path, source: &SynapseCalyxError) -> StorageError {
    calyx_open_failed_detail(path, source.to_string())
}

fn calyx_open_failed_detail(path: &Path, detail: String) -> StorageError {
    open_failed_detail_with_backend(path, StorageBackendKind::Calyx, detail)
}

fn open_failed_detail_with_backend(
    path: &Path,
    backend: StorageBackendKind,
    detail: String,
) -> StorageError {
    tracing::warn!(
        code = error_codes::STORAGE_OPEN_FAILED,
        storage_path = %path.display(),
        backend = backend.as_str(),
        %detail,
        "storage open failed"
    );
    StorageError::OpenFailed {
        path: path.to_path_buf(),
        detail,
    }
}
