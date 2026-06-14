use super::{SessionTarget, SynapseService, explicit_action_target};

#[test]
fn explicit_action_target_prefers_window_then_cdp_and_rejects_orphan_cdp() {
    // #984B: explicit per-call routing maps to the right SessionTarget so
    // multi-window/multi-agent action routing is deterministic.
    println!("readback=explicit_action_target edge=none input=(None,None)");
    assert_eq!(
        explicit_action_target(None, None).expect("no override is allowed"),
        None,
        "no explicit params => fall back to the session target"
    );

    println!("readback=explicit_action_target edge=window input=(Some(0x1234),None)");
    assert_eq!(
        explicit_action_target(Some(0x1234), None).expect("window override is valid"),
        Some(SessionTarget::Window { hwnd: 0x1234 }),
    );

    println!(
        "readback=explicit_action_target edge=cdp input=(Some(0x1234),Some(\"ABC\")) plus whitespace trim"
    );
    assert_eq!(
        explicit_action_target(Some(0x1234), Some("  ABC123  ")).expect("cdp override is valid"),
        Some(SessionTarget::Cdp {
            window_hwnd: 0x1234,
            cdp_target_id: "ABC123".to_owned(),
        }),
        "cdp_target_id is trimmed and paired with the window hwnd"
    );

    // Whitespace-only cdp_target_id is treated as absent, not as a CDP target.
    println!("readback=explicit_action_target edge=blank_cdp input=(Some(7),Some(\"   \"))");
    assert_eq!(
        explicit_action_target(Some(7), Some("   ")).expect("blank cdp id ignored"),
        Some(SessionTarget::Window { hwnd: 7 }),
    );

    println!("readback=explicit_action_target edge=orphan_cdp input=(None,Some(\"ABC\"))");
    let err = explicit_action_target(None, Some("ABC"))
        .expect_err("cdp_target_id without window_hwnd must be rejected");
    let code = err
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(code, Some(synapse_core::error_codes::TOOL_PARAMS_INVALID));
}

#[test]
fn health_payload_reports_m3_subsystems_initializing_or_disabled() {
    let service = SynapseService::new();
    let payload = service.health_payload();
    assert!(payload.ok);
    assert_eq!(payload.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(payload.build, "dev");
    assert_eq!(payload.subsystems["storage"].status, "initializing");
    assert_eq!(payload.subsystems["reflex"].status, "initializing");
    assert_eq!(payload.subsystems["profiles"].status, "initializing");
    assert!(!payload.subsystems.contains_key("hid_host"));
    assert_eq!(payload.subsystems["action"].status, "ok");
    assert_eq!(payload.subsystems["audio"].status, "disabled");
    assert_eq!(payload.subsystems["http"].status, "disabled");
}

#[test]
fn uptime_uses_monotonic_elapsed() {
    let service = SynapseService::new();
    let first = service.health_payload().uptime_s;
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = service.health_payload().uptime_s;
    assert!(second >= first);
}
