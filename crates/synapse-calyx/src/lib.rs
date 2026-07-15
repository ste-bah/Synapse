//! Synapse-owned lifecycle wrapper for the embedded Calyx Aster vault.

mod async_vault;

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use calyx_aster::cf::{ColumnFamily, KeyRange};
use calyx_aster::mvcc::{Freshness, Snapshot};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Seq, VaultId};
use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

pub use async_vault::{
    SynapseCalyxAsyncConfig, SynapseCalyxAsyncVault, SynapseCalyxAsyncVaultHandle,
    SynapseCalyxCfWrite, SynapseCalyxReaderLease,
};

pub type SynapseCalyxCfRows = Vec<(Vec<u8>, Vec<u8>)>;

const SYNAPSE_DIR_NAME: &str = "synapse";
const VAULT_DIR_NAME: &str = "vault";
const IDENTITY_FILE_NAME: &str = "vault-identity.json";
const MACHINE_SALT_FILE_NAME: &str = "machine-salt.bin";
const LOCK_FILE_NAME: &str = "vault.lock";
const PID_FILE_NAME: &str = "vault.pid";
const IDENTITY_SCHEMA_VERSION: u32 = 1;
const MACHINE_SALT_BYTES: usize = 32;

const APPDATA_MISSING_REMEDIATION: &str =
    "set APPDATA or configure SYNAPSE_CALYX_VAULT_DIR to an explicit durable directory";
const IDENTITY_REMEDIATION: &str =
    "restore the vault identity files from backup or inspect the exact file named in the error";
const LOCK_REMEDIATION: &str =
    "stop the process holding the vault lock or point Synapse at a different vault directory";
const OPEN_REMEDIATION: &str =
    "inspect the vault directory, recovery report, and Calyx error; repair storage before restart";
const CLOSE_REMEDIATION: &str = "inspect the vault directory and shutdown logs; do not start a successor until the lock and PID readback are clean";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynapseCalyxConfig {
    pub vault_dir: PathBuf,
    pub machine_salt_path: PathBuf,
}

impl SynapseCalyxConfig {
    /// Resolves the default roaming Synapse Calyx paths.
    ///
    /// This deliberately errors when `APPDATA` is absent. A transient temp-dir
    /// fallback would create an unannounced second vault, which is worse than a
    /// startup failure for durable state.
    ///
    /// # Errors
    ///
    /// Returns an error when `APPDATA` is absent.
    pub fn from_default_roaming() -> Result<Self, SynapseCalyxError> {
        let data_dir = roaming_synapse_dir()?;
        Ok(Self {
            vault_dir: data_dir.join(VAULT_DIR_NAME),
            machine_salt_path: data_dir.join(MACHINE_SALT_FILE_NAME),
        })
    }

    #[must_use]
    pub fn from_vault_dir(vault_dir: PathBuf) -> Self {
        let salt_parent = vault_dir
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        Self {
            vault_dir,
            machine_salt_path: salt_parent.join(MACHINE_SALT_FILE_NAME),
        }
    }

    /// Resolves the configured vault directory or the default roaming path.
    ///
    /// # Errors
    ///
    /// Returns an error when no explicit path is supplied and the default
    /// roaming path cannot be resolved, or when the explicit path is empty.
    pub fn from_optional_vault_dir(vault_dir: Option<PathBuf>) -> Result<Self, SynapseCalyxError> {
        match vault_dir {
            Some(path) if path.as_os_str().is_empty() => Err(SynapseCalyxError::new(
                "SYNAPSE_CALYX_VAULT_DIR_EMPTY",
                "configured Calyx vault directory is empty",
                "set SYNAPSE_CALYX_VAULT_DIR to an absolute durable path or unset it for the default APPDATA path",
            )),
            Some(path) => Ok(Self::from_vault_dir(path)),
            None => Self::from_default_roaming(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SynapseCalyxError {
    pub code: &'static str,
    pub message: String,
    pub remediation: &'static str,
}

impl SynapseCalyxError {
    #[must_use]
    pub fn new(code: &'static str, message: impl Into<String>, remediation: &'static str) -> Self {
        Self {
            code,
            message: message.into(),
            remediation,
        }
    }

    #[must_use]
    pub fn with_io(
        code: &'static str,
        action: &str,
        path: &Path,
        error: &std::io::Error,
        remediation: &'static str,
    ) -> Self {
        Self::new(
            code,
            format!("{action} {}: {error}", path.display()),
            remediation,
        )
    }

    #[must_use]
    pub fn from_calyx(action: &str, error: &calyx_core::CalyxError) -> Self {
        Self::new(
            error.code,
            format!("{action}: {}", error.message),
            error.remediation,
        )
    }
}

impl std::fmt::Display for SynapseCalyxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}; remediation={}",
            self.code, self.message, self.remediation
        )
    }
}

impl std::error::Error for SynapseCalyxError {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SynapseCalyxVaultStatus {
    pub enabled: bool,
    pub phase: String,
    pub open: bool,
    pub vault_dir: Option<PathBuf>,
    pub identity_path: Option<PathBuf>,
    pub machine_salt_path: Option<PathBuf>,
    pub lock_path: Option<PathBuf>,
    pub pid_path: Option<PathBuf>,
    pub vault_id: Option<String>,
    pub latest_seq: Option<u64>,
    pub last_recovered_seq: Option<u64>,
    pub torn_tail: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error: Option<String>,
    pub remediation: Option<String>,
}

impl SynapseCalyxVaultStatus {
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            phase: "disabled".to_owned(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn not_opened(config: Option<&SynapseCalyxConfig>) -> Self {
        let mut status = Self {
            enabled: true,
            phase: "not_opened".to_owned(),
            ..Self::default()
        };
        if let Some(config) = config {
            status.apply_paths(config);
        }
        status
    }

    #[must_use]
    pub fn error(
        config: Option<&SynapseCalyxConfig>,
        phase: &'static str,
        error: &SynapseCalyxError,
    ) -> Self {
        let mut status = Self {
            enabled: true,
            phase: phase.to_owned(),
            last_error_code: Some(error.code.to_owned()),
            last_error: Some(error.message.clone()),
            remediation: Some(error.remediation.to_owned()),
            ..Self::default()
        };
        if let Some(config) = config {
            status.apply_paths(config);
        }
        status
    }

    fn apply_paths(&mut self, config: &SynapseCalyxConfig) {
        self.vault_dir = Some(config.vault_dir.clone());
        self.identity_path = Some(identity_path(&config.vault_dir));
        self.machine_salt_path = Some(config.machine_salt_path.clone());
        self.lock_path = Some(lock_path(&config.vault_dir));
        self.pid_path = Some(pid_path(&config.vault_dir));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SynapseCalyxVaultCloseReadback {
    pub enabled: bool,
    pub reason: &'static str,
    pub closed: bool,
    pub safe_to_unlock: bool,
    pub vault_dir: Option<PathBuf>,
    pub lock_path: Option<PathBuf>,
    pub pid_path: Option<PathBuf>,
    pub pid_sidecar_present_after_close: Option<bool>,
    pub re_lock_probe_succeeded: Option<bool>,
    pub latest_seq: Option<u64>,
}

impl SynapseCalyxVaultCloseReadback {
    #[must_use]
    pub const fn disabled(reason: &'static str) -> Self {
        Self {
            enabled: false,
            reason,
            closed: true,
            safe_to_unlock: true,
            vault_dir: None,
            lock_path: None,
            pid_path: None,
            pid_sidecar_present_after_close: None,
            re_lock_probe_succeeded: None,
            latest_seq: None,
        }
    }

    #[must_use]
    pub fn not_open(reason: &'static str, config: Option<&SynapseCalyxConfig>) -> Self {
        Self {
            enabled: true,
            reason,
            closed: true,
            safe_to_unlock: true,
            vault_dir: config.map(|config| config.vault_dir.clone()),
            lock_path: config.map(|config| lock_path(&config.vault_dir)),
            pid_path: config.map(|config| pid_path(&config.vault_dir)),
            pid_sidecar_present_after_close: None,
            re_lock_probe_succeeded: None,
            latest_seq: None,
        }
    }
}

#[derive(Debug)]
pub struct SynapseCalyxVault {
    config: SynapseCalyxConfig,
    vault: AsterVault,
    lock: VaultLockGuard,
}

impl SynapseCalyxVault {
    /// Opens the configured durable Aster vault after acquiring the Synapse
    /// process lock and loading the stable vault identity.
    ///
    /// # Errors
    ///
    /// Returns an error when directories, identity files, the machine-local
    /// salt, the single-instance lock, or Calyx recovery/open fail.
    pub fn open(config: SynapseCalyxConfig) -> Result<Self, SynapseCalyxError> {
        create_dir_all(&config.vault_dir)?;
        create_parent_dir(&config.machine_salt_path)?;
        let lock = VaultLockGuard::acquire(&config.vault_dir)?;
        let identity = match load_or_create_identity(&config) {
            Ok(identity) => identity,
            Err(error) => return Err(cleanup_open_lock(lock, error)),
        };
        let vault_id = match identity.parse_vault_id() {
            Ok(vault_id) => vault_id,
            Err(error) => return Err(cleanup_open_lock(lock, error)),
        };
        let vault = match AsterVault::open(
            &config.vault_dir,
            vault_id,
            identity.machine_salt,
            VaultOptions::default(),
        ) {
            Ok(vault) => vault,
            Err(error) => {
                return Err(cleanup_open_lock(
                    lock,
                    SynapseCalyxError::from_calyx("open durable Calyx Aster vault", &error),
                ));
            }
        };
        let status = status_from_vault(&config, &vault);
        tracing::info!(
            code = "SYNAPSE_CALYX_VAULT_OPENED",
            vault_dir = %config.vault_dir.display(),
            lock_path = %lock.path.display(),
            pid_path = %lock.pid_path.display(),
            vault_id = status.vault_id.as_deref().unwrap_or(""),
            latest_seq = status.latest_seq,
            last_recovered_seq = status.last_recovered_seq,
            torn_tail = status.torn_tail.as_deref().unwrap_or("none"),
            "opened durable Calyx Aster vault"
        );
        Ok(Self {
            config,
            vault,
            lock,
        })
    }

    #[must_use]
    pub fn status(&self) -> SynapseCalyxVaultStatus {
        status_from_vault(&self.config, &self.vault)
    }

    /// Writes raw CF rows through Aster's durable WAL/MVCC commit path.
    ///
    /// This is synchronous by construction. Tokio callers must use
    /// [`SynapseCalyxAsyncVault`] so the call is owned by the vault worker
    /// thread, not an executor worker.
    ///
    /// # Errors
    ///
    /// Returns a structured Calyx-backed error if admission, WAL append,
    /// MVCC apply, checkpoint staging, or any durability guard fails.
    pub(crate) fn write_cf_batch(
        &self,
        rows: Vec<SynapseCalyxCfWrite>,
    ) -> Result<Seq, SynapseCalyxError> {
        self.vault
            .write_cf_batch(rows.into_iter().map(|row| (row.cf, row.key, row.value)))
            .map_err(|error| SynapseCalyxError::from_calyx("write Calyx CF batch", &error))
    }

    /// Reads one raw CF row at a numeric snapshot.
    ///
    /// # Errors
    ///
    /// Returns a structured Calyx-backed error if the snapshot is stale,
    /// blocked by a read barrier, or unavailable from the opened recovery mode.
    pub(crate) fn read_cf_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, SynapseCalyxError> {
        self.vault
            .read_cf_at(snapshot, cf, key)
            .map_err(|error| SynapseCalyxError::from_calyx("read Calyx CF row", &error))
    }

    /// Reads one raw CF row through an explicit pinned snapshot lease.
    ///
    /// # Errors
    ///
    /// Returns a structured Calyx-backed error if the lease expired, the row is
    /// blocked by a read barrier, or the opened recovery mode cannot serve it.
    pub(crate) fn read_cf_snapshot(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, SynapseCalyxError> {
        self.vault
            .read_cf_snapshot(snapshot, cf, key)
            .map_err(|error| {
                SynapseCalyxError::from_calyx("read Calyx CF row from pinned snapshot", &error)
            })
    }

    /// Scans visible raw CF rows at a numeric snapshot.
    ///
    /// # Errors
    ///
    /// Returns a structured Calyx-backed error if the snapshot is stale,
    /// blocked by a read barrier, or unavailable from the opened recovery mode.
    pub(crate) fn scan_cf_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
    ) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
        self.vault
            .scan_cf_at(snapshot, cf)
            .map_err(|error| SynapseCalyxError::from_calyx("scan Calyx CF", &error))
    }

    /// Scans visible raw CF rows through an explicit pinned snapshot lease.
    ///
    /// # Errors
    ///
    /// Returns a structured Calyx-backed error if the lease expired, any row is
    /// blocked by a read barrier, or the opened recovery mode cannot serve it.
    pub(crate) fn scan_cf_snapshot(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
    ) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
        self.vault.scan_cf_snapshot(snapshot, cf).map_err(|error| {
            SynapseCalyxError::from_calyx("scan Calyx CF from pinned snapshot", &error)
        })
    }

    /// Scans visible raw CF rows in a key range at a numeric snapshot.
    ///
    /// # Errors
    ///
    /// Returns a structured Calyx-backed error if the snapshot is stale,
    /// blocked by a read barrier, or unavailable from the opened recovery mode.
    pub(crate) fn scan_cf_range_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        range: &KeyRange,
    ) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
        self.vault
            .scan_cf_range_at(snapshot, cf, range)
            .map_err(|error| SynapseCalyxError::from_calyx("scan Calyx CF range", &error))
    }

    /// Scans visible raw CF rows in a key range through an explicit pinned
    /// snapshot lease.
    ///
    /// # Errors
    ///
    /// Returns a structured Calyx-backed error if the lease expired, any row is
    /// blocked by a read barrier, or the opened recovery mode cannot serve it.
    pub(crate) fn scan_cf_range_snapshot(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        range: &KeyRange,
    ) -> Result<SynapseCalyxCfRows, SynapseCalyxError> {
        self.vault
            .scan_cf_range_snapshot(snapshot, cf, range)
            .map_err(|error| {
                SynapseCalyxError::from_calyx("scan Calyx CF range from pinned snapshot", &error)
            })
    }

    /// Pins a bounded reader lease.
    ///
    /// # Errors
    ///
    /// Returns a structured error when `max_age_ms == 0`; zero-length leases
    /// are rejected so callers cannot accidentally create immediately expired
    /// snapshots and then misclassify the follow-on read failure.
    pub(crate) fn pin_reader(
        &self,
        freshness: Freshness,
        max_age_ms: u64,
    ) -> Result<Snapshot, SynapseCalyxError> {
        if max_age_ms == 0 {
            return Err(SynapseCalyxError::new(
                "SYNAPSE_CALYX_READER_LEASE_ZERO",
                "Calyx reader lease max_age_ms must be greater than zero",
                "request a bounded positive lease lifetime; use release_reader when the read is complete",
            ));
        }
        Ok(self.vault.pin_reader(freshness, max_age_ms))
    }

    #[must_use]
    pub(crate) fn release_reader(&self, lease_id: u64) -> bool {
        self.vault.release_reader(lease_id)
    }

    /// Flushes Aster's WAL-backed batcher and pending durable checkpoints.
    ///
    /// # Errors
    ///
    /// Returns a structured Calyx-backed error if the WAL fsync or checkpoint
    /// flush fails.
    pub(crate) fn flush(&self) -> Result<(), SynapseCalyxError> {
        self.vault
            .flush()
            .map_err(|error| SynapseCalyxError::from_calyx("flush Calyx Aster vault", &error))
    }

    /// Flushes and closes the durable vault, then proves the lock can be
    /// reacquired before reporting a safe shutdown readback.
    ///
    /// # Errors
    ///
    /// Returns an error when flush, PID-sidecar cleanup, lock release, or the
    /// re-lock proof fails.
    pub fn close(
        self,
        reason: &'static str,
    ) -> Result<SynapseCalyxVaultCloseReadback, SynapseCalyxError> {
        let Self {
            config,
            vault,
            lock,
        } = self;
        let latest_seq = vault.latest_seq();
        vault.flush().map_err(|error| {
            SynapseCalyxError::from_calyx("flush durable Calyx Aster vault", &error)
        })?;
        tracing::info!(
            code = "SYNAPSE_CALYX_VAULT_FLUSHED",
            reason,
            vault_dir = %config.vault_dir.display(),
            latest_seq,
            "flushed durable Calyx Aster vault before shutdown"
        );
        drop(vault);
        let lock_readback = lock.close(reason)?;
        let readback = SynapseCalyxVaultCloseReadback {
            enabled: true,
            reason,
            closed: true,
            safe_to_unlock: lock_readback.safe_to_unlock,
            vault_dir: Some(config.vault_dir.clone()),
            lock_path: Some(lock_readback.lock_path),
            pid_path: Some(lock_readback.pid_path),
            pid_sidecar_present_after_close: Some(lock_readback.pid_sidecar_present_after_close),
            re_lock_probe_succeeded: Some(lock_readback.re_lock_probe_succeeded),
            latest_seq: Some(latest_seq),
        };
        tracing::info!(
            code = "SYNAPSE_CALYX_VAULT_CLOSED",
            reason,
            vault_dir = %config.vault_dir.display(),
            safe_to_unlock = readback.safe_to_unlock,
            pid_sidecar_present_after_close = readback.pid_sidecar_present_after_close,
            re_lock_probe_succeeded = readback.re_lock_probe_succeeded,
            latest_seq,
            "closed durable Calyx Aster vault"
        );
        Ok(readback)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct VaultIdentityDisk {
    schema_version: u32,
    vault_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VaultIdentity {
    vault_id: String,
    machine_salt: Vec<u8>,
}

impl VaultIdentity {
    fn parse_vault_id(&self) -> Result<VaultId, SynapseCalyxError> {
        VaultId::from_str(&self.vault_id).map_err(|error| {
            SynapseCalyxError::new(
                "SYNAPSE_CALYX_VAULT_ID_INVALID",
                format!("parse vault id {}: {error}", self.vault_id),
                IDENTITY_REMEDIATION,
            )
        })
    }
}

#[derive(Debug)]
struct VaultLockGuard {
    file: File,
    path: PathBuf,
    pid_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VaultLockCloseReadback {
    lock_path: PathBuf,
    pid_path: PathBuf,
    pid_sidecar_present_after_close: bool,
    re_lock_probe_succeeded: bool,
    safe_to_unlock: bool,
}

impl VaultLockGuard {
    fn acquire(vault_dir: &Path) -> Result<Self, SynapseCalyxError> {
        let path = lock_path(vault_dir);
        let pid_path = pid_path(vault_dir);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| {
                SynapseCalyxError::with_io(
                    "SYNAPSE_CALYX_LOCK_OPEN_FAILED",
                    "open Calyx vault lock",
                    &path,
                    &error,
                    LOCK_REMEDIATION,
                )
            })?;
        if let Err(error) = file.try_lock_exclusive() {
            let holder = read_optional_to_string(&pid_path)
                .unwrap_or_else(|read_error| format!("pid sidecar read failed: {read_error}"));
            return Err(SynapseCalyxError::new(
                "SYNAPSE_CALYX_LOCK_HELD",
                format!(
                    "Calyx vault lock {} is held or unavailable: {error}; holder_readback={holder}",
                    path.display()
                ),
                LOCK_REMEDIATION,
            ));
        }
        write_pid_sidecar(&pid_path).inspect_err(|_error| {
            let _ = file.unlock();
        })?;
        Ok(Self {
            file,
            path,
            pid_path,
        })
    }

    fn close(self, reason: &'static str) -> Result<VaultLockCloseReadback, SynapseCalyxError> {
        let path = self.path.clone();
        let pid_path = self.pid_path.clone();
        if let Err(error) = fs::remove_file(&pid_path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::error!(
                code = "SYNAPSE_CALYX_PID_SIDECAR_REMOVE_FAILED",
                reason,
                pid_path = %pid_path.display(),
                error = %error,
                "retaining Calyx vault lock until process exit because PID sidecar removal failed"
            );
            std::mem::forget(self);
            return Err(SynapseCalyxError::with_io(
                "SYNAPSE_CALYX_PID_SIDECAR_REMOVE_FAILED",
                "remove Calyx vault PID sidecar",
                &pid_path,
                &error,
                CLOSE_REMEDIATION,
            ));
        }
        if let Err(error) = self.file.unlock() {
            tracing::error!(
                code = "SYNAPSE_CALYX_LOCK_RELEASE_FAILED",
                reason,
                lock_path = %path.display(),
                error = %error,
                "retaining Calyx vault lock file handle until process exit because unlock failed"
            );
            std::mem::forget(self);
            return Err(SynapseCalyxError::with_io(
                "SYNAPSE_CALYX_LOCK_RELEASE_FAILED",
                "release Calyx vault lock",
                &path,
                &error,
                CLOSE_REMEDIATION,
            ));
        }
        let pid_sidecar_present_after_close = pid_path.exists();
        let re_lock_probe_succeeded = probe_relock(&path)?;
        let readback = VaultLockCloseReadback {
            lock_path: path,
            pid_path,
            pid_sidecar_present_after_close,
            re_lock_probe_succeeded,
            safe_to_unlock: !pid_sidecar_present_after_close && re_lock_probe_succeeded,
        };
        if !readback.safe_to_unlock {
            return Err(SynapseCalyxError::new(
                "SYNAPSE_CALYX_LOCK_CLOSE_READBACK_FAILED",
                format!("readback={readback:?}"),
                CLOSE_REMEDIATION,
            ));
        }
        Ok(readback)
    }
}

fn status_from_vault(config: &SynapseCalyxConfig, vault: &AsterVault) -> SynapseCalyxVaultStatus {
    let recovery_report = vault.recovery_report();
    let mut status = SynapseCalyxVaultStatus {
        enabled: true,
        phase: "open".to_owned(),
        open: true,
        vault_id: Some(vault.vault_id().to_string()),
        latest_seq: Some(vault.latest_seq()),
        last_recovered_seq: Some(recovery_report.last_recovered_seq),
        torn_tail: recovery_report
            .torn_tail
            .as_ref()
            .map(|tail| format!("{tail:?}")),
        ..SynapseCalyxVaultStatus::default()
    };
    status.apply_paths(config);
    status
}

fn cleanup_open_lock(lock: VaultLockGuard, primary: SynapseCalyxError) -> SynapseCalyxError {
    match lock.close("calyx_open_failed") {
        Ok(readback) => {
            tracing::info!(
                code = "SYNAPSE_CALYX_OPEN_FAILURE_LOCK_CLEANED",
                primary_code = primary.code,
                readback = ?readback,
                "closed Calyx vault lock after startup failure"
            );
            primary
        }
        Err(cleanup_error) => SynapseCalyxError::new(
            "SYNAPSE_CALYX_OPEN_FAILURE_LOCK_CLEANUP_FAILED",
            format!("primary={primary}; cleanup={cleanup_error}"),
            CLOSE_REMEDIATION,
        ),
    }
}

fn roaming_synapse_dir() -> Result<PathBuf, SynapseCalyxError> {
    let Some(appdata) = std::env::var_os("APPDATA") else {
        return Err(SynapseCalyxError::new(
            "SYNAPSE_CALYX_APPDATA_MISSING",
            "APPDATA is not set; refusing to create a non-durable fallback vault",
            APPDATA_MISSING_REMEDIATION,
        ));
    };
    Ok(PathBuf::from(appdata).join(SYNAPSE_DIR_NAME))
}

fn create_dir_all(path: &Path) -> Result<(), SynapseCalyxError> {
    fs::create_dir_all(path).map_err(|error| {
        SynapseCalyxError::with_io(
            "SYNAPSE_CALYX_DIR_CREATE_FAILED",
            "create Calyx vault directory",
            path,
            &error,
            OPEN_REMEDIATION,
        )
    })
}

fn create_parent_dir(path: &Path) -> Result<(), SynapseCalyxError> {
    let Some(parent) = path.parent() else {
        return Err(SynapseCalyxError::new(
            "SYNAPSE_CALYX_PARENT_DIR_MISSING",
            format!("path {} has no parent directory", path.display()),
            OPEN_REMEDIATION,
        ));
    };
    fs::create_dir_all(parent).map_err(|error| {
        SynapseCalyxError::with_io(
            "SYNAPSE_CALYX_DIR_CREATE_FAILED",
            "create Calyx parent directory",
            parent,
            &error,
            OPEN_REMEDIATION,
        )
    })
}

fn load_or_create_identity(
    config: &SynapseCalyxConfig,
) -> Result<VaultIdentity, SynapseCalyxError> {
    let identity_path = identity_path(&config.vault_dir);
    if !identity_path.exists() {
        let disk = VaultIdentityDisk {
            schema_version: IDENTITY_SCHEMA_VERSION,
            vault_id: VaultId::from_ulid(Ulid::new()).to_string(),
        };
        write_identity_atomic(&identity_path, &disk)?;
    }
    let vault_id = read_identity(&identity_path)?.vault_id;
    let machine_salt = load_or_create_machine_salt(&config.machine_salt_path)?;
    Ok(VaultIdentity {
        vault_id,
        machine_salt,
    })
}

fn read_identity(path: &Path) -> Result<VaultIdentityDisk, SynapseCalyxError> {
    let raw = fs::read_to_string(path).map_err(|error| {
        SynapseCalyxError::with_io(
            "SYNAPSE_CALYX_IDENTITY_READ_FAILED",
            "read Calyx vault identity",
            path,
            &error,
            IDENTITY_REMEDIATION,
        )
    })?;
    let identity = serde_json::from_str::<VaultIdentityDisk>(&raw).map_err(|error| {
        SynapseCalyxError::new(
            "SYNAPSE_CALYX_IDENTITY_INVALID",
            format!("parse Calyx vault identity {}: {error}", path.display()),
            IDENTITY_REMEDIATION,
        )
    })?;
    if identity.schema_version != IDENTITY_SCHEMA_VERSION {
        return Err(SynapseCalyxError::new(
            "SYNAPSE_CALYX_IDENTITY_SCHEMA_UNSUPPORTED",
            format!(
                "Calyx vault identity {} schema_version={} expected={IDENTITY_SCHEMA_VERSION}",
                path.display(),
                identity.schema_version
            ),
            IDENTITY_REMEDIATION,
        ));
    }
    VaultId::from_str(&identity.vault_id).map_err(|error| {
        SynapseCalyxError::new(
            "SYNAPSE_CALYX_VAULT_ID_INVALID",
            format!(
                "parse vault id {} from {}: {error}",
                identity.vault_id,
                path.display()
            ),
            IDENTITY_REMEDIATION,
        )
    })?;
    Ok(identity)
}

fn write_identity_atomic(
    path: &Path,
    identity: &VaultIdentityDisk,
) -> Result<(), SynapseCalyxError> {
    let temp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let encoded = serde_json::to_vec_pretty(identity).map_err(|error| {
        SynapseCalyxError::new(
            "SYNAPSE_CALYX_IDENTITY_ENCODE_FAILED",
            format!("encode Calyx vault identity {}: {error}", path.display()),
            IDENTITY_REMEDIATION,
        )
    })?;
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| {
                SynapseCalyxError::with_io(
                    "SYNAPSE_CALYX_IDENTITY_WRITE_FAILED",
                    "create staged Calyx vault identity",
                    &temp,
                    &error,
                    IDENTITY_REMEDIATION,
                )
            })?;
        file.write_all(&encoded).map_err(|error| {
            SynapseCalyxError::with_io(
                "SYNAPSE_CALYX_IDENTITY_WRITE_FAILED",
                "write staged Calyx vault identity",
                &temp,
                &error,
                IDENTITY_REMEDIATION,
            )
        })?;
        file.sync_all().map_err(|error| {
            SynapseCalyxError::with_io(
                "SYNAPSE_CALYX_IDENTITY_SYNC_FAILED",
                "sync staged Calyx vault identity",
                &temp,
                &error,
                IDENTITY_REMEDIATION,
            )
        })?;
    }
    fs::rename(&temp, path).map_err(|error| {
        SynapseCalyxError::with_io(
            "SYNAPSE_CALYX_IDENTITY_RENAME_FAILED",
            "publish Calyx vault identity",
            path,
            &error,
            IDENTITY_REMEDIATION,
        )
    })?;
    sync_parent_dir(
        path,
        "Calyx vault identity",
        "SYNAPSE_CALYX_IDENTITY_PARENT_SYNC_FAILED",
        IDENTITY_REMEDIATION,
    )
}

fn load_or_create_machine_salt(path: &Path) -> Result<Vec<u8>, SynapseCalyxError> {
    if path.exists() {
        return read_machine_salt(path);
    }
    let mut bytes = [0_u8; MACHINE_SALT_BYTES];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    write_machine_salt_atomic(path, &bytes)?;
    read_machine_salt(path)
}

fn read_machine_salt(path: &Path) -> Result<Vec<u8>, SynapseCalyxError> {
    let encoded = fs::read_to_string(path).map_err(|error| {
        SynapseCalyxError::with_io(
            "SYNAPSE_CALYX_MACHINE_SALT_READ_FAILED",
            "read Calyx machine-local salt",
            path,
            &error,
            IDENTITY_REMEDIATION,
        )
    })?;
    let bytes = BASE64.decode(encoded.trim()).map_err(|error| {
        SynapseCalyxError::new(
            "SYNAPSE_CALYX_MACHINE_SALT_INVALID",
            format!(
                "decode Calyx machine-local salt {}: {error}",
                path.display()
            ),
            IDENTITY_REMEDIATION,
        )
    })?;
    if bytes.len() != MACHINE_SALT_BYTES {
        return Err(SynapseCalyxError::new(
            "SYNAPSE_CALYX_MACHINE_SALT_INVALID",
            format!(
                "Calyx machine-local salt {} has {} bytes expected {MACHINE_SALT_BYTES}",
                path.display(),
                bytes.len()
            ),
            IDENTITY_REMEDIATION,
        ));
    }
    Ok(bytes)
}

fn write_machine_salt_atomic(
    path: &Path,
    bytes: &[u8; MACHINE_SALT_BYTES],
) -> Result<(), SynapseCalyxError> {
    let temp = path.with_extension(format!("bin.tmp.{}", std::process::id()));
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| {
                SynapseCalyxError::with_io(
                    "SYNAPSE_CALYX_MACHINE_SALT_WRITE_FAILED",
                    "create staged Calyx machine-local salt",
                    &temp,
                    &error,
                    IDENTITY_REMEDIATION,
                )
            })?;
        file.write_all(BASE64.encode(bytes).as_bytes())
            .map_err(|error| {
                SynapseCalyxError::with_io(
                    "SYNAPSE_CALYX_MACHINE_SALT_WRITE_FAILED",
                    "write staged Calyx machine-local salt",
                    &temp,
                    &error,
                    IDENTITY_REMEDIATION,
                )
            })?;
        file.sync_all().map_err(|error| {
            SynapseCalyxError::with_io(
                "SYNAPSE_CALYX_MACHINE_SALT_SYNC_FAILED",
                "sync staged Calyx machine-local salt",
                &temp,
                &error,
                IDENTITY_REMEDIATION,
            )
        })?;
    }
    fs::rename(&temp, path).map_err(|error| {
        SynapseCalyxError::with_io(
            "SYNAPSE_CALYX_MACHINE_SALT_RENAME_FAILED",
            "publish Calyx machine-local salt",
            path,
            &error,
            IDENTITY_REMEDIATION,
        )
    })?;
    sync_parent_dir(
        path,
        "Calyx machine-local salt",
        "SYNAPSE_CALYX_MACHINE_SALT_PARENT_SYNC_FAILED",
        IDENTITY_REMEDIATION,
    )
}

fn write_pid_sidecar(path: &Path) -> Result<(), SynapseCalyxError> {
    let exe = std::env::current_exe().map_or_else(
        |error| format!("current_exe_read_failed:{error}"),
        |path| path.display().to_string(),
    );
    let body = serde_json::json!({
        "schema_version": 1,
        "pid": std::process::id(),
        "exe": exe,
    });
    let encoded = serde_json::to_vec_pretty(&body).map_err(|error| {
        SynapseCalyxError::new(
            "SYNAPSE_CALYX_PID_SIDECAR_ENCODE_FAILED",
            format!("encode Calyx vault PID sidecar {}: {error}", path.display()),
            LOCK_REMEDIATION,
        )
    })?;
    let mut file = File::create(path).map_err(|error| {
        SynapseCalyxError::with_io(
            "SYNAPSE_CALYX_PID_SIDECAR_WRITE_FAILED",
            "create Calyx vault PID sidecar",
            path,
            &error,
            LOCK_REMEDIATION,
        )
    })?;
    file.write_all(&encoded).map_err(|error| {
        SynapseCalyxError::with_io(
            "SYNAPSE_CALYX_PID_SIDECAR_WRITE_FAILED",
            "write Calyx vault PID sidecar",
            path,
            &error,
            LOCK_REMEDIATION,
        )
    })?;
    file.sync_all().map_err(|error| {
        SynapseCalyxError::with_io(
            "SYNAPSE_CALYX_PID_SIDECAR_SYNC_FAILED",
            "sync Calyx vault PID sidecar",
            path,
            &error,
            LOCK_REMEDIATION,
        )
    })?;
    drop(file);
    sync_parent_dir(
        path,
        "Calyx vault PID sidecar",
        "SYNAPSE_CALYX_PID_SIDECAR_PARENT_SYNC_FAILED",
        LOCK_REMEDIATION,
    )
}

fn sync_parent_dir(
    path: &Path,
    label: &str,
    code: &'static str,
    remediation: &'static str,
) -> Result<(), SynapseCalyxError> {
    let Some(parent) = path.parent() else {
        return Err(SynapseCalyxError::new(
            code,
            format!("sync {label} parent for {}: no parent", path.display()),
            remediation,
        ));
    };
    sync_dir(parent, label, code, remediation)
}

#[cfg(unix)]
fn sync_dir(
    dir: &Path,
    label: &str,
    code: &'static str,
    remediation: &'static str,
) -> Result<(), SynapseCalyxError> {
    if !dir.is_dir() {
        return Err(SynapseCalyxError::new(
            code,
            format!(
                "sync {label} parent directory {}: not a directory",
                dir.display()
            ),
            remediation,
        ));
    }
    File::open(dir)
        .and_then(|handle| handle.sync_all())
        .map_err(|error| {
            SynapseCalyxError::with_io(
                code,
                "sync Calyx parent directory",
                dir,
                &error,
                remediation,
            )
        })
}

#[cfg(windows)]
fn sync_dir(
    dir: &Path,
    label: &str,
    code: &'static str,
    remediation: &'static str,
) -> Result<(), SynapseCalyxError> {
    use std::os::windows::fs::OpenOptionsExt as _;

    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;

    if !dir.is_dir() {
        return Err(SynapseCalyxError::new(
            code,
            format!(
                "sync {label} parent directory {}: not a directory",
                dir.display()
            ),
            remediation,
        ));
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)
        .and_then(|handle| handle.sync_all())
        .map_err(|error| {
            SynapseCalyxError::with_io(
                code,
                "sync Calyx parent directory",
                dir,
                &error,
                remediation,
            )
        })
}

#[cfg(not(any(unix, windows)))]
fn sync_dir(
    dir: &Path,
    label: &str,
    code: &'static str,
    remediation: &'static str,
) -> Result<(), SynapseCalyxError> {
    if !dir.is_dir() {
        return Err(SynapseCalyxError::new(
            code,
            format!(
                "sync {label} parent directory {}: not a directory",
                dir.display()
            ),
            remediation,
        ));
    }
    Err(SynapseCalyxError::new(
        code,
        format!(
            "sync {label} parent directory {}: unsupported platform",
            dir.display()
        ),
        remediation,
    ))
}

fn probe_relock(path: &Path) -> Result<bool, SynapseCalyxError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            SynapseCalyxError::with_io(
                "SYNAPSE_CALYX_LOCK_PROBE_OPEN_FAILED",
                "open Calyx vault lock for release probe",
                path,
                &error,
                CLOSE_REMEDIATION,
            )
        })?;
    match file.try_lock_exclusive() {
        Ok(()) => {
            file.unlock().map_err(|error| {
                SynapseCalyxError::with_io(
                    "SYNAPSE_CALYX_LOCK_PROBE_RELEASE_FAILED",
                    "release Calyx vault lock probe",
                    path,
                    &error,
                    CLOSE_REMEDIATION,
                )
            })?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(SynapseCalyxError::with_io(
            "SYNAPSE_CALYX_LOCK_PROBE_FAILED",
            "probe Calyx vault lock release",
            path,
            &error,
            CLOSE_REMEDIATION,
        )),
    }
}

fn read_optional_to_string(path: &Path) -> std::io::Result<String> {
    let mut raw = String::new();
    let mut file = File::open(path)?;
    file.read_to_string(&mut raw)?;
    Ok(raw)
}

fn identity_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(IDENTITY_FILE_NAME)
}

fn lock_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(LOCK_FILE_NAME)
}

fn pid_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(PID_FILE_NAME)
}
