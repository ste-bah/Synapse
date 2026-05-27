use super::{M3ServiceConfig, M3State, m3_tool_stubs};
use crate::http::sse::SseState;
use std::path::PathBuf;
use synapse_reflex::DEFAULT_MAX_SUBSCRIPTIONS_NONZERO;
use tokio_util::sync::CancellationToken;

#[test]
fn m3_state_from_config_reads_cli_shape() -> anyhow::Result<()> {
    println!(
        "readback=m3_state scenario=from_config before_db=db before_profile=profiles before_reflex=true"
    );
    let state = M3State::from_config_with_shutdown_reason_and_sse_state(
        M3ServiceConfig {
            db_path: Some(PathBuf::from("db")),
            profile_dir: Some(PathBuf::from("profiles")),
            reflex_disabled: true,
            bind: "127.0.0.1:7701".to_owned(),
            bearer_token: Some("token".to_owned()),
            max_subscriptions: DEFAULT_MAX_SUBSCRIPTIONS_NONZERO,
            enable_audio: false,
            allow_unknown_profile: false,
            allowed_permissions: None,
            reflex_force_degraded: false,
            storage_pressure_free_bytes_sample: None,
        },
        CancellationToken::new(),
        "sigint",
        Some(CancellationToken::new()),
        SseState::from_env(),
    )?;
    println!(
        "readback=m3_state scenario=from_config after_db={:?} after_profile={:?} after_reflex_disabled={} after_bind={} after_token_present={} after_connection_token={}",
        state.db_path,
        state.profile_dir,
        state.reflex_disabled,
        state.bind,
        state.bearer_token.is_some(),
        state.connection_closed_cancel.is_some()
    );
    assert!(state.reflex_disabled);
    assert!(state.scaffold_ready());
    Ok(())
}

#[test]
fn m3_state_rejects_invalid_reflex_disabled() {
    let before_reflex = Some("yes");
    println!("readback=m3_state scenario=invalid_reflex before={before_reflex:?}");
    let after = M3State::from_parts_with_sse_state(
        None,
        None,
        before_reflex,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        CancellationToken::new(),
        "shutdown",
        None,
        SseState::from_env(),
    );
    println!("readback=m3_state scenario=invalid_reflex after={after:?}");
    assert!(after.is_err());
}

#[test]
fn m3_tool_stub_names_are_stable() {
    let expected = [
        "subscribe",
        "subscribe_cancel",
        "reflex_register",
        "reflex_cancel",
        "reflex_list",
        "reflex_history",
        "profile_list",
        "profile_activate",
        "profile_quality_refresh",
        "profile_registry_search",
        "profile_registry_inspect",
        "profile_registry_install",
        "profile_registry_disable",
        "profile_registry_export",
        "profile_registry_import",
        "audit_intelligence_query",
        "replay_record",
        "audio_tail",
        "audio_transcribe",
        "storage_inspect",
        "storage_put_probe_rows",
        "storage_gc_once",
        "storage_pressure_sample",
    ];
    println!("readback=m3_tool_stubs before=expected:{expected:?}");
    let actual = m3_tool_stubs().map(|stub| stub.name);
    println!("readback=m3_tool_stubs after=actual:{actual:?}");
    assert_eq!(actual, expected);
}
