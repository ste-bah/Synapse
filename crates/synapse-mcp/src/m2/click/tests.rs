use std::sync::Arc;

use synapse_action::{ActionBackend, ActionEmitter, RecordingBackend};
use tokio_util::sync::CancellationToken;

use super::{
    act_click_with_handle,
    schema::{
        ActClickParams, ActClickPointTarget, ActClickTarget, default_click_backend,
        default_click_button, default_click_count, default_click_curve, default_click_duration_ms,
        default_use_invoke_pattern,
    },
};

#[tokio::test]
async fn coordinate_click_leaves_actor_held_state_empty() {
    let cancel = CancellationToken::new();
    let backend: Arc<dyn ActionBackend> = Arc::new(RecordingBackend::new());
    let (handle, snapshot_handle, join) =
        ActionEmitter::spawn_with_backend(cancel.clone(), backend);
    let before = match snapshot_handle.snapshot().await {
        Ok(snapshot) => snapshot,
        Err(err) => panic!("before snapshot failed: {err}"),
    };
    println!(
        "source_of_truth=act_click_actor edge=coordinate before=held_buttons:{:?} held_keys:{:?}",
        before.held_buttons, before.held_keys
    );
    let response = match act_click_with_handle(
        handle,
        None,
        ActClickParams {
            target: ActClickTarget::Point(ActClickPointTarget { x: 12, y: 34 }),
            button: default_click_button(),
            clicks: default_click_count(),
            modifiers: Vec::new(),
            curve: default_click_curve(),
            duration_ms: default_click_duration_ms(),
            backend: default_click_backend(),
            use_invoke_pattern: default_use_invoke_pattern(),
        },
    )
    .await
    {
        Ok(response) => response,
        Err(err) => panic!("act_click failed: {err}"),
    };
    let after = match snapshot_handle.snapshot().await {
        Ok(snapshot) => snapshot,
        Err(err) => panic!("after snapshot failed: {err}"),
    };
    println!(
        "source_of_truth=act_click_actor edge=coordinate after=ok:{} backend_used:{} held_buttons:{:?} held_keys:{:?}",
        response.ok, response.backend_used, after.held_buttons, after.held_keys
    );
    assert!(response.ok);
    assert!(!response.used_invoke_pattern);
    assert_eq!(response.backend_used, "software");
    assert!(after.held_buttons.is_empty());
    assert!(after.held_keys.is_empty());
    cancel.cancel();
    let _final_snapshot = match join.await {
        Ok(snapshot) => snapshot,
        Err(err) => panic!("join failed: {err}"),
    };
}
