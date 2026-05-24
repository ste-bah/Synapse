use super::{M3State, m3_tool_stubs};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[test]
fn m3_state_from_parts_reads_env_shape_with_fsv() -> anyhow::Result<()> {
    let before_reflex = Some("TRUE");
    println!(
        "source_of_truth=m3_state scenario=from_parts before_db=db before_profile=profiles before_reflex={before_reflex:?}"
    );
    let state = M3State::from_parts(
        Some(PathBuf::from("db")),
        Some(PathBuf::from("profiles")),
        before_reflex,
        Some("token".to_owned()),
        Some("127.0.0.1:7701".to_owned()),
        CancellationToken::new(),
        "sigint",
        Some(CancellationToken::new()),
    )?;
    println!(
        "source_of_truth=m3_state scenario=from_parts after_db={:?} after_profile={:?} after_reflex_disabled={} after_bind={} after_token_present={} after_connection_token={}",
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
fn m3_state_rejects_invalid_reflex_disabled_with_fsv() {
    let before_reflex = Some("yes");
    println!("source_of_truth=m3_state scenario=invalid_reflex before={before_reflex:?}");
    let after = M3State::from_parts(
        None,
        None,
        before_reflex,
        None,
        None,
        CancellationToken::new(),
        "shutdown",
        None,
    );
    println!("source_of_truth=m3_state scenario=invalid_reflex after={after:?}");
    assert!(after.is_err());
}

#[test]
fn m3_tool_stub_names_have_fsv() {
    let expected = [
        "subscribe",
        "subscribe_cancel",
        "reflex_register",
        "reflex_cancel",
        "reflex_list",
        "reflex_history",
        "profile_list",
        "profile_activate",
        "replay_record",
        "audio_tail",
        "audio_transcribe",
    ];
    println!("source_of_truth=m3_tool_stubs before=expected:{expected:?}");
    let actual = m3_tool_stubs().map(|stub| stub.name);
    println!("source_of_truth=m3_tool_stubs after=actual:{actual:?}");
    assert_eq!(actual, expected);
}
