use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, anyhow, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    m2::{M2ServiceConfig, recording_backend_enabled},
    safety::hardware_consent::{
        HARDWARE_HID_ACK_PHRASE, HardwareConsentInput, require_hardware_hid_consent,
    },
};

pub const AGREEMENT_VERSION: u32 = 1;
pub const AGREEMENT_PATH_ENV: &str = "SYNAPSE_AGREEMENT_PATH";
const DEFAULT_SUPPORTED_USE_SCOPES: [&str; 2] = ["productivity", "single_player"];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementRecord {
    pub version: u32,
    pub acknowledged_at: String,
    pub hardware_hid: HardwareHidAgreement,
    pub supported_use_scopes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HardwareHidAgreement {
    pub port: String,
    pub ack_phrase_sha256: String,
}

impl AgreementRecord {
    #[must_use]
    pub fn for_hardware_hid_port(port: &str) -> Self {
        Self {
            version: AGREEMENT_VERSION,
            acknowledged_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            hardware_hid: HardwareHidAgreement {
                port: port.to_owned(),
                ack_phrase_sha256: ack_phrase_sha256(),
            },
            supported_use_scopes: DEFAULT_SUPPORTED_USE_SCOPES
                .iter()
                .map(ToString::to_string)
                .collect(),
        }
    }
}

pub fn ensure_hardware_hid_agreement(
    config: &M2ServiceConfig,
    reset_hardware_consent: bool,
) -> anyhow::Result<Option<AgreementRecord>> {
    let Some(port) = config.hardware_hid_readback() else {
        return Ok(None);
    };
    if recording_backend_enabled(config.recording_backend.as_deref()) {
        tracing::info!(
            code = "SAFETY_AGREEMENT_SKIPPED_RECORDING_BACKEND",
            hardware_hid = %port,
            "hardware HID agreement skipped because recording backend disables production HID"
        );
        return Ok(None);
    }
    ensure_hardware_hid_agreement_for_port(&port, reset_hardware_consent).map(Some)
}

pub fn agreement_path() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os(AGREEMENT_PATH_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return Ok(path);
    }
    let appdata = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("%APPDATA% is not set; cannot locate Synapse agreement.json"))?;
    Ok(appdata.join("synapse").join("agreement.json"))
}

pub fn ack_phrase_sha256() -> String {
    let digest = Sha256::digest(HARDWARE_HID_ACK_PHRASE.as_bytes());
    hex_lower(&digest)
}

fn ensure_hardware_hid_agreement_for_port(
    port: &str,
    reset_hardware_consent: bool,
) -> anyhow::Result<AgreementRecord> {
    #[cfg(windows)]
    {
        ensure_hardware_hid_agreement_at_path(
            &agreement_path()?,
            port,
            reset_hardware_consent,
            HardwareConsentInput::Interactive,
        )
    }
    #[cfg(not(windows))]
    {
        let _reset_hardware_consent = reset_hardware_consent;
        require_hardware_hid_consent(port, HardwareConsentInput::Interactive)?;
        Ok(AgreementRecord::for_hardware_hid_port(port))
    }
}

fn ensure_hardware_hid_agreement_at_path(
    path: &Path,
    port: &str,
    reset_hardware_consent: bool,
    consent_input: HardwareConsentInput,
) -> anyhow::Result<AgreementRecord> {
    if port.trim().is_empty() {
        bail!("hardware HID agreement port must not be empty");
    }
    if reset_hardware_consent {
        reset_existing_agreement(path)?;
    }
    let record = match read_existing_agreement(path)? {
        Some(record) => record,
        None => create_agreement(path, port, consent_input)?,
    };
    validate_agreement(&record)?;
    #[cfg(windows)]
    {
        apply_agreement_acl(path)
            .with_context(|| format!("apply Windows ACL to {}", path.display()))?;
        let acl = read_agreement_acl(path)
            .with_context(|| format!("read Windows ACL from {}", path.display()))?;
        if !acl.matches_expected_contract {
            bail!(
                "agreement ACL readback did not match contract: sddl={} expected={}",
                acl.sddl,
                acl.expected_sddl
            );
        }
    }
    Ok(record)
}

fn reset_existing_agreement(path: &Path) -> anyhow::Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            #[cfg(windows)]
            {
                prepare_agreement_for_reset(path)
                    .with_context(|| format!("prepare {} for reset", path.display()))?;
            }
            fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
            Ok(())
        }
        Ok(_) => bail!("agreement path {} is not a file", path.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("read metadata {}", path.display())),
    }
}

fn read_existing_agreement(path: &Path) -> anyhow::Result<Option<AgreementRecord>> {
    match fs::read(path) {
        Ok(bytes) => {
            let record: AgreementRecord = serde_json::from_slice(&bytes)
                .with_context(|| format!("decode {}", path.display()))?;
            validate_agreement(&record)
                .with_context(|| format!("validate existing {}", path.display()))?;
            Ok(Some(record))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn create_agreement(
    path: &Path,
    port: &str,
    consent_input: HardwareConsentInput,
) -> anyhow::Result<AgreementRecord> {
    require_hardware_hid_consent(port, consent_input)?;
    write_agreement(path, port)
}

fn write_agreement(path: &Path, port: &str) -> anyhow::Result<AgreementRecord> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let record = AgreementRecord::for_hardware_hid_port(port);
    let mut bytes = serde_json::to_vec_pretty(&record).context("encode agreement.json")?;
    bytes.push(b'\n');
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(&bytes)
                .with_context(|| format!("write {}", path.display()))?;
            file.flush()
                .with_context(|| format!("flush {}", path.display()))?;
            Ok(record)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            read_existing_agreement(path)?
                .ok_or_else(|| anyhow!("agreement appeared but could not be read"))
        }
        Err(error) => Err(error).with_context(|| format!("create {}", path.display())),
    }
}

fn validate_agreement(record: &AgreementRecord) -> anyhow::Result<()> {
    if record.version != AGREEMENT_VERSION {
        bail!(
            "agreement version {} does not match expected {}",
            record.version,
            AGREEMENT_VERSION
        );
    }
    DateTime::parse_from_rfc3339(&record.acknowledged_at)
        .with_context(|| format!("invalid agreement timestamp {}", record.acknowledged_at))?;
    if record.hardware_hid.port.trim().is_empty() {
        bail!("agreement hardware HID port must not be empty");
    }
    let expected_hash = ack_phrase_sha256();
    if record.hardware_hid.ack_phrase_sha256 != expected_hash {
        bail!("agreement hardware HID acknowledgment phrase hash does not match");
    }
    let expected_scopes: Vec<String> = DEFAULT_SUPPORTED_USE_SCOPES
        .iter()
        .map(ToString::to_string)
        .collect();
    if record.supported_use_scopes != expected_scopes {
        bail!(
            "agreement supported_use_scopes {:?} do not match {:?}",
            record.supported_use_scopes,
            expected_scopes
        );
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgreementAclReadback {
    pub sddl: String,
    pub expected_sddl: String,
    pub matches_expected_contract: bool,
}

#[cfg(windows)]
pub fn read_agreement_acl(path: &Path) -> anyhow::Result<AgreementAclReadback> {
    windows_acl::read_agreement_acl(path)
}

#[cfg(windows)]
fn apply_agreement_acl(path: &Path) -> anyhow::Result<()> {
    windows_acl::apply_agreement_acl(path)
}

#[cfg(windows)]
fn prepare_agreement_for_reset(path: &Path) -> anyhow::Result<()> {
    windows_acl::prepare_agreement_for_reset(path)
}

#[cfg(windows)]
mod windows_acl;

#[cfg(test)]
mod tests;
