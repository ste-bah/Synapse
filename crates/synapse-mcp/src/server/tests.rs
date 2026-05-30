use super::SynapseService;

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
    assert_eq!(payload.subsystems["hid_host"].status, "disabled");
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
