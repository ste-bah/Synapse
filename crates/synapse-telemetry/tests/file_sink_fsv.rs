use synapse_telemetry::{TelemetryConfig, TelemetryError, init_tracing};
use tempfile::TempDir;
use tracing::{error, info};

#[test]
fn synthetic_emit_lands_in_jsonl_file() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let cfg = TelemetryConfig::default_with_log_dir(dir.path().to_path_buf());

    let guard = init_tracing(cfg)?;

    info!(field_a = "v_a", field_b = 42, "happy_path_event");
    error!(
        field_c = "oops",
        error.kind = "TELEMETRY_GC_FAILED",
        "edge_case_event"
    );
    info!("plain_message_event");

    drop(guard);

    let logs = read_log_dir(dir.path())?;
    let lines: Vec<&str> = logs.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(lines.len(), 3, "expected 3 events, got {}", lines.len());

    let first: serde_json::Value = serde_json::from_str(lines[0])?;
    assert_eq!(first["fields"]["field_a"], "v_a");
    assert_eq!(first["fields"]["field_b"], 42);
    assert_eq!(first["fields"]["message"], "happy_path_event");
    assert_eq!(first["level"], "INFO");

    let second: serde_json::Value = serde_json::from_str(lines[1])?;
    assert_eq!(second["fields"]["field_c"], "oops");
    assert_eq!(second["fields"]["error.kind"], "TELEMETRY_GC_FAILED");
    assert_eq!(second["level"], "ERROR");

    let third: serde_json::Value = serde_json::from_str(lines[2])?;
    assert_eq!(third["fields"]["message"], "plain_message_event");
    Ok(())
}

#[test]
fn synthetic_unwritable_log_dir_returns_error() {
    let cfg = TelemetryConfig::default_with_log_dir("/proc/cant_write_here".into());
    let res = init_tracing(cfg);
    assert!(matches!(res, Err(TelemetryError::LogDirNotWritable(_))));
}

fn read_log_dir(path: &std::path::Path) -> Result<String, Box<dyn std::error::Error>> {
    let mut contents = String::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.metadata()?.is_file() {
            contents.push_str(&std::fs::read_to_string(entry.path())?);
        }
    }
    assert!(!contents.is_empty(), "no log file content produced");
    Ok(contents)
}
