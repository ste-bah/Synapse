use synapse_core::{Action, Key, KeyCode, error_codes};
use tokio::time::{self, Duration, Instant};

use super::{ActionEmitter, HELD_KEY_MAX_DURATION_MS};
use crate::ResolvedBackend;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct HeldKeyTimerKey {
    pub(super) key: Key,
    pub(super) backend: ResolvedBackend,
}

impl HeldKeyTimerKey {
    pub(super) const fn new(key: Key, backend: ResolvedBackend) -> Self {
        Self { key, backend }
    }
}

#[derive(Debug)]
pub(super) struct HeldKeyAutoRelease {
    timer_key: HeldKeyTimerKey,
    timer_id: u64,
}

impl ActionEmitter {
    pub(super) fn schedule_held_key_auto_release(&mut self, key: Key, backend: ResolvedBackend) {
        let timer_key = HeldKeyTimerKey::new(key, backend);
        self.cancel_held_key_timer(&timer_key);

        let timer_id = self.next_held_key_timer_id;
        self.next_held_key_timer_id = self.next_held_key_timer_id.wrapping_add(1);
        let deadline = Instant::now() + Duration::from_millis(HELD_KEY_MAX_DURATION_MS);
        let tx = self.auto_release_tx.clone();
        let auto_release_key = timer_key.clone();
        let handle = tokio::spawn(async move {
            time::sleep_until(deadline).await;
            let _send_result = tx
                .send(HeldKeyAutoRelease {
                    timer_key: auto_release_key,
                    timer_id,
                })
                .await;
        });

        self.held_key_timer_ids.insert(timer_key.clone(), timer_id);
        self.held_key_timers.insert(timer_key, handle);
    }

    pub(super) fn cancel_held_key_timer(&mut self, timer_key: &HeldKeyTimerKey) -> bool {
        self.held_key_timer_ids.remove(timer_key);
        self.held_key_timers
            .remove(timer_key)
            .is_some_and(|handle| {
                handle.abort();
                true
            })
    }

    pub(super) fn abort_all_held_key_timers(&mut self) -> usize {
        let cancelled = self.held_key_timers.len();
        for (_key, handle) in self.held_key_timers.drain() {
            handle.abort();
        }
        self.held_key_timer_ids.clear();
        cancelled
    }

    pub(super) fn auto_release_held_key(
        &mut self,
        auto_release: &HeldKeyAutoRelease,
    ) -> Option<Action> {
        if self
            .held_key_timer_ids
            .get(&auto_release.timer_key)
            .is_none_or(|timer_id| *timer_id != auto_release.timer_id)
        {
            return None;
        }

        self.held_key_timer_ids.remove(&auto_release.timer_key);
        self.held_key_timers.remove(&auto_release.timer_key);
        if !self
            .state
            .is_key_held_for_backend(&auto_release.timer_key.key, auto_release.timer_key.backend)
        {
            return None;
        }

        tracing::warn!(
            code = %error_codes::STUCK_KEY_AUTO_RELEASED,
            backend = auto_release.timer_key.backend.as_str(),
            held_ms = HELD_KEY_MAX_DURATION_MS,
            key = %key_log_label(&auto_release.timer_key.key),
            key_debug = ?auto_release.timer_key.key,
            "stuck key auto-released"
        );
        Some(Action::KeyUp {
            key: auto_release.timer_key.key.clone(),
            backend: auto_release.timer_key.backend.to_backend(),
        })
    }

    pub(super) fn held_key_timer_keys(&self) -> Vec<Key> {
        let mut keys: Vec<_> = self
            .held_key_timers
            .keys()
            .map(|timer_key| timer_key.key.clone())
            .collect();
        keys.sort_by_key(|key| format!("{key:?}"));
        keys
    }
}
pub(super) fn key_log_label(key: &Key) -> String {
    match &key.code {
        KeyCode::Named { value } => value.clone(),
        KeyCode::Symbol { value } => value.to_string(),
        KeyCode::HidCode { value } => format!("hid:{value}"),
    }
}
