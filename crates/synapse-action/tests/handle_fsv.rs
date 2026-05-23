use std::time::Duration;

use synapse_action::{ACTION_QUEUE_CAPACITY, ActionError, ActionHandle};
use synapse_core::Action;

#[test]
fn try_execute_reports_queue_full_at_capacity_boundary() {
    let (handle, rx) = ActionHandle::channel();
    assert_eq!(rx.len(), 0);
    println!(
        "source_of_truth=action_queue edge=capacity before=queued:{}",
        rx.len()
    );

    for _index in 0..ACTION_QUEUE_CAPACITY {
        handle
            .try_execute(Action::ReleaseAll)
            .unwrap_or_else(|err| panic!("queue should accept capacity-sized burst: {err}"));
    }

    assert_eq!(rx.len(), ACTION_QUEUE_CAPACITY);
    let error = match handle.try_execute(Action::ReleaseAll) {
        Ok(()) => panic!("257th queued action should hit bounded mpsc capacity"),
        Err(error) => error,
    };
    assert_eq!(error.code(), synapse_core::error_codes::ACTION_QUEUE_FULL);
    println!(
        "source_of_truth=action_queue edge=capacity after=queued:{} final_value={}",
        rx.len(),
        error.code()
    );
}

#[test]
fn try_execute_reports_closed_channel_without_mutating_queue() {
    let (handle, rx) = ActionHandle::channel();
    println!(
        "source_of_truth=action_queue edge=closed before=queued:{}",
        rx.len()
    );
    drop(rx);

    let error = match handle.try_execute(Action::ReleaseAll) {
        Ok(()) => panic!("closed receiver should reject enqueue"),
        Err(error) => error,
    };
    assert!(matches!(error, ActionError::BackendUnavailable { .. }));
    println!(
        "source_of_truth=action_queue edge=closed after=receiver_dropped final_value={}",
        error.code()
    );
}

#[test]
fn release_all_blocking_timeout_enqueues_then_reports_timeout() {
    let (handle, rx) = ActionHandle::channel();
    println!(
        "source_of_truth=action_queue edge=release_timeout before=queued:{}",
        rx.len()
    );

    let error = match handle.fire_release_all_blocking_with_timeout(Duration::ZERO) {
        Ok(()) => panic!("no actor can ack release_all, so this must time out"),
        Err(error) => error,
    };
    assert!(matches!(error, ActionError::BackendUnavailable { .. }));
    assert_eq!(rx.len(), 1);
    println!(
        "source_of_truth=action_queue edge=release_timeout after=queued:{} final_value={}",
        rx.len(),
        error.detail()
    );
}

#[tokio::test]
async fn execute_waits_for_actor_ack_happy_path() {
    let (handle, mut rx) = ActionHandle::channel();
    println!(
        "source_of_truth=action_queue edge=execute_ack before=queued:{}",
        rx.len()
    );

    let actor = tokio::spawn(async move {
        let Some((action, ack)) = rx.recv().await else {
            panic!("handle should enqueue one action");
        };
        assert!(matches!(action, Action::ReleaseAll));
        ack.send(Ok(()))
            .unwrap_or_else(|err| panic!("test receiver should still wait for ack: {err:?}"));
        rx.len()
    });

    handle
        .execute(Action::ReleaseAll)
        .await
        .unwrap_or_else(|err| panic!("actor ack should make execute succeed: {err}"));
    let final_len = actor
        .await
        .unwrap_or_else(|err| panic!("actor task should complete: {err}"));
    assert_eq!(final_len, 0);
    println!("source_of_truth=action_queue edge=execute_ack after=queued:{final_len}");
}
