use std::{fs, io::Write};

use anyhow::bail;
use tempfile::TempDir;

use super::*;

#[test]
fn agreement_schema_round_trip_matches_expected_synthetic_port() -> anyhow::Result<()> {
    let dir = TempDir::new()?;
    let path = dir.path().join("agreement.json");
    let before_exists = path.exists();
    let record = write_agreement(&path, "COM427")?;
    let bytes = fs::read(&path)?;
    let after: AgreementRecord = serde_json::from_slice(&bytes)?;
    println!("readback=agreement_schema edge=happy before_exists={before_exists} after={after:?}");
    assert_eq!(after, record);
    validate_agreement(&after)?;
    Ok(())
}

#[test]
fn agreement_accepts_existing_ack_for_changed_configured_port() -> anyhow::Result<()> {
    let dir = TempDir::new()?;
    let path = dir.path().join("agreement.json");
    let before = write_agreement(&path, "COM427")?;
    let after = ensure_hardware_hid_agreement_at_path(
        &path,
        "COM428",
        false,
        HardwareConsentInput::Provided(HARDWARE_HID_ACK_PHRASE),
    )?;
    println!(
        "readback=agreement_schema edge=changed_configured_port before={before:?} after={after:?}"
    );
    assert_eq!(after.hardware_hid.port, "COM427");
    Ok(())
}

#[test]
fn agreement_create_requires_exact_hardware_consent() -> anyhow::Result<()> {
    let dir = TempDir::new()?;
    let path = dir.path().join("agreement.json");
    let before_exists = path.exists();
    let Err(error) = create_agreement(
        &path,
        "COM426",
        HardwareConsentInput::Provided("I AUTHORIZE HARDWARE INPUT "),
    ) else {
        bail!("expected consent refusal");
    };
    let after_exists = path.exists();
    println!(
        "readback=agreement_schema edge=bad_consent before_exists={before_exists} after_exists={after_exists} after_error={error:#}"
    );
    assert!(
        error
            .downcast_ref::<super::super::hardware_consent::HardwareConsentRefused>()
            .is_some(),
        "{error:#}"
    );
    assert!(!after_exists);

    let record = create_agreement(
        &path,
        "COM426",
        HardwareConsentInput::Provided(HARDWARE_HID_ACK_PHRASE),
    )?;
    let after_bytes = fs::read(&path)?;
    let after: AgreementRecord = serde_json::from_slice(&after_bytes)?;
    println!("readback=agreement_schema edge=good_consent after={after:?}");
    assert_eq!(after, record);
    Ok(())
}

#[test]
fn reset_hardware_consent_rewrites_agreement_for_new_port() -> anyhow::Result<()> {
    let dir = TempDir::new()?;
    let path = dir.path().join("agreement.json");
    let before = write_agreement(&path, "COM426")?;
    let after = ensure_hardware_hid_agreement_at_path(
        &path,
        "COM427",
        true,
        HardwareConsentInput::Provided(HARDWARE_HID_ACK_PHRASE),
    )?;
    println!("readback=agreement_schema edge=reset before={before:?} after={after:?}");
    assert_eq!(after.hardware_hid.port, "COM427");
    Ok(())
}

#[test]
fn agreement_rejects_unknown_schema_field() -> anyhow::Result<()> {
    let dir = TempDir::new()?;
    let path = dir.path().join("agreement.json");
    let before = r#"{"version":1,"acknowledged_at":"2026-05-28T01:02:03Z","hardware_hid":{"port":"COM427","ack_phrase_sha256":"bogus"},"supported_use_scopes":["productivity","single_player"],"extra":true}"#;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(before.as_bytes())?;
    let Err(error) = read_existing_agreement(&path) else {
        bail!("expected unknown agreement field to be rejected");
    };
    println!("readback=agreement_schema edge=unknown_field before={before} after_error={error:#}");
    assert!(error.to_string().contains("decode"));
    Ok(())
}

#[test]
fn agreement_rejects_bad_ack_phrase_hash() -> anyhow::Result<()> {
    let dir = TempDir::new()?;
    let path = dir.path().join("agreement.json");
    let mut before = AgreementRecord::for_hardware_hid_port("COM427");
    before.hardware_hid.ack_phrase_sha256 = "bad".to_owned();
    fs::write(&path, serde_json::to_vec_pretty(&before)?)?;
    let Err(error) = read_existing_agreement(&path) else {
        bail!("expected bad acknowledgment phrase hash to be rejected");
    };
    println!("readback=agreement_schema edge=bad_hash before={before:?} after_error={error:#}");
    assert!(error.to_string().contains("validate existing"));
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_acl_readback_matches_agreement_contract() -> anyhow::Result<()> {
    let dir = TempDir::new()?;
    let path = dir.path().join("agreement.json");
    let before_exists = path.exists();
    let after = ensure_hardware_hid_agreement_at_path(
        &path,
        "COM427",
        false,
        HardwareConsentInput::Provided(HARDWARE_HID_ACK_PHRASE),
    )?;
    let acl = read_agreement_acl(&path)?;
    println!(
        "readback=agreement_acl edge=happy before_exists={before_exists} after={after:?} acl={acl:?}"
    );
    assert!(acl.matches_expected_contract, "{acl:?}");
    super::windows_acl::restore_current_user_full_control_for_test(&path)?;
    fs::remove_file(&path)?;
    Ok(())
}
