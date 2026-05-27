use std::{collections::BTreeMap, path::Path};

use synapse_core::{StoredReflexAudit, error_codes};
use synapse_storage::{
    DiskPressureLevel, GcReport, PressureReport, StorageResult, cf, decode_json,
};

use crate::{ReflexError, ReflexResult, ReflexRuntime};

impl ReflexRuntime {
    /// Returns the storage path backing this runtime.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn storage_path(&self) -> &Path {
        &self.db.path
    }

    /// Returns the storage schema version backing this runtime.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn schema_version(&self) -> u32 {
        self.db.schema_version
    }

    /// Returns the current storage pressure level backing reflex persistence.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn storage_pressure_level(&self) -> DiskPressureLevel {
        self.db.pressure_level()
    }

    /// Returns logical byte sizes for each storage column family.
    ///
    /// # Errors
    ///
    /// Returns a storage error when a column family scan fails.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn storage_cf_sizes(&self) -> StorageResult<BTreeMap<String, u64>> {
        self.db.cf_sizes()
    }

    /// Returns exact row counts for each storage column family.
    ///
    /// # Errors
    ///
    /// Returns a storage error when a column family cannot be scanned.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn storage_cf_row_counts(&self) -> StorageResult<BTreeMap<String, u64>> {
        self.db.cf_row_counts()
    }

    /// Returns the newest rows in one storage column family.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the column family cannot be scanned.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime", cf_name, limit))]
    pub fn storage_cf_tail_rows(
        &self,
        cf_name: &str,
        limit: usize,
    ) -> StorageResult<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut rows = self.db.scan_cf(cf_name)?;
        let keep_from = rows.len().saturating_sub(limit);
        Ok(rows.split_off(keep_from))
    }

    /// Writes a bounded diagnostic batch to storage and flushes it immediately.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the write or flush fails.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime", cf_name))]
    pub fn storage_put_probe_rows(
        &self,
        cf_name: &str,
        rows: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> StorageResult<()> {
        self.db.put_batch(cf_name, rows)?;
        self.db.flush()
    }

    /// Writes action audit rows and flushes them immediately.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the write or flush fails.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime", row_count = rows.len()))]
    pub fn storage_put_action_log_rows(&self, rows: Vec<(Vec<u8>, Vec<u8>)>) -> StorageResult<()> {
        self.db.put_batch(cf::CF_ACTION_LOG, rows)?;
        self.db.flush()
    }

    /// Writes profile-registry rows and flushes them immediately.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the write or flush fails.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime", row_count = rows.len()))]
    pub fn storage_put_profile_rows(&self, rows: Vec<(Vec<u8>, Vec<u8>)>) -> StorageResult<()> {
        self.db.put_batch(cf::CF_PROFILES, rows)?;
        self.db.flush()
    }

    /// Returns one exact profile-registry row by key.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the profile CF cannot be scanned.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime", key_len = key.len()))]
    pub fn storage_profile_row(&self, key: &[u8]) -> StorageResult<Option<Vec<u8>>> {
        Ok(self
            .db
            .scan_cf(cf::CF_PROFILES)?
            .into_iter()
            .find_map(|(row_key, value)| (row_key == key).then_some(value)))
    }

    /// Runs one row-cap storage GC pass for operator diagnostics.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the GC pass fails.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime", cf_name))]
    pub fn storage_run_gc_once_with_row_caps(
        &self,
        cf_name: &'static str,
        soft_cap_rows: u64,
        hard_cap_rows: u64,
    ) -> StorageResult<GcReport> {
        self.db
            .run_gc_once_with_row_caps(cf_name, soft_cap_rows, hard_cap_rows)
    }

    /// Applies one synthetic free-space sample through the production pressure responder.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the pressure check fails.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime", free_bytes))]
    pub fn storage_run_pressure_sample(&self, free_bytes: u64) -> StorageResult<PressureReport> {
        self.db
            .run_pressure_check_with_free_bytes_sample(free_bytes)
    }

    /// Returns the in-process storage pressure transition code history.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the pressure state cannot be read.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn storage_pressure_transition_codes(&self) -> StorageResult<Vec<&'static str>> {
        self.db.pressure_transition_codes()
    }

    /// Counts persisted recursion-guard clamp audit rows.
    ///
    /// # Errors
    ///
    /// Returns a reflex error when audit rows cannot be scanned or decoded.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn recursion_clamps_total(&self) -> ReflexResult<u64> {
        let rows =
            self.db
                .scan_cf(cf::CF_REFLEX_AUDIT)
                .map_err(|error| ReflexError::ParamsInvalid {
                    detail: format!("reflex audit scan failed: {error}"),
                })?;
        let mut total = 0_u64;
        for (_key, value) in rows {
            let audit = decode_json::<StoredReflexAudit>(&value).map_err(|error| {
                ReflexError::ParamsInvalid {
                    detail: format!("reflex audit decode failed: {error}"),
                }
            })?;
            if audit.error_code.as_deref() == Some(error_codes::REFLEX_RECURSION_LIMIT) {
                total = total.saturating_add(1);
            }
        }
        Ok(total)
    }
}
