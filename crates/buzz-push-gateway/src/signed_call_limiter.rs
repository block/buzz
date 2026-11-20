//! Per-attested-key rate limit on signed installation calls.
//!
//! Attestation carries no stable device id and each enrollment makes a fresh
//! key, so enrollment is not limited per device. The challenge quota caps only
//! outstanding challenges (consuming one deletes it), not the rate of
//! completed enrollments. After enrollment the attested key is the
//! installation's identity, and this limit keeps one installation from
//! flooding the gateway. Counters live in memory, per replica, so a deployment
//! with several replicas allows each key up to that many times the configured
//! calls.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

/// Most keys tracked at once. Past it, the key with the oldest window is
/// dropped, so memory stays bounded however many keys call.
const MAX_TRACKED_KEYS: usize = 100_000;

/// A fixed-window counter per attested key.
pub struct SignedCallLimiter {
    window_seconds: i64,
    max_calls: i64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    windows: HashMap<Vec<u8>, Window>,
    /// Every window start, oldest first. A key whose window restarted leaves
    /// a stale entry behind (its start no longer matches the map), which is
    /// skipped on eviction and dropped by compaction. Evicting from the front
    /// keeps a call on a full map O(1) amortized instead of a scan of it.
    starts: VecDeque<(i64, Vec<u8>)>,
}

#[derive(Clone, Copy)]
struct Window {
    started_at: i64,
    calls: i64,
}

impl State {
    fn is_current(&self, started_at: i64, key: &[u8]) -> bool {
        self.windows
            .get(key)
            .is_some_and(|w| w.started_at == started_at)
    }

    /// Drop the key with the oldest window. Wall-clock steps backwards can
    /// leave the queue slightly out of order, which only makes the choice
    /// approximate.
    fn evict_oldest(&mut self) {
        while let Some((started_at, key)) = self.starts.pop_front() {
            if self.is_current(started_at, &key) {
                self.windows.remove(&key);
                return;
            }
        }
    }

    fn start_window(&mut self, key: &[u8], now: i64) {
        self.starts.push_back((now, key.to_vec()));
        // Restarted windows leave stale entries; drop them once they make up
        // half the queue, so compaction stays amortized O(1) per call.
        if self.starts.len() > 2 * self.windows.len().max(1) {
            let State { windows, starts } = self;
            starts.retain(|(started_at, key)| {
                windows
                    .get(key)
                    .is_some_and(|w| w.started_at == *started_at)
            });
        }
    }
}

impl SignedCallLimiter {
    /// At most `max_calls` calls per key in each `window_seconds` window.
    pub fn new(window_seconds: i64, max_calls: i64) -> Self {
        Self {
            window_seconds,
            max_calls,
            state: Mutex::new(State::default()),
        }
    }

    /// Count one call by `key` at `now`; false when the key is over its limit.
    pub fn admit(&self, key: &[u8], now: i64) -> bool {
        // The state holds only counters, so a poisoned lock is still consistent.
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let restarting = match state.windows.get_mut(key) {
            Some(window) if now.saturating_sub(window.started_at) < self.window_seconds => {
                if window.calls >= self.max_calls {
                    return false;
                }
                window.calls += 1;
                return true;
            }
            Some(_) => true,
            None => false,
        };
        if !restarting && state.windows.len() >= MAX_TRACKED_KEYS {
            state.evict_oldest();
        }
        let admitted = self.max_calls > 0;
        state.windows.insert(
            key.to_vec(),
            Window {
                started_at: now,
                calls: i64::from(admitted),
            },
        );
        state.start_window(key, now);
        admitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_key_has_its_own_window_and_the_map_stays_bounded() {
        let limiter = SignedCallLimiter::new(60, 2);
        assert!(limiter.admit(b"a", 100));
        assert!(limiter.admit(b"a", 101));
        assert!(!limiter.admit(b"a", 159));
        assert!(limiter.admit(b"b", 159), "another key is unaffected");
        assert!(limiter.admit(b"a", 160), "the window rolled over");
        for index in 0..(MAX_TRACKED_KEYS + 10) {
            limiter.admit(&index.to_be_bytes(), 1_000);
        }
        let tracked = limiter.state.lock().unwrap().windows.len();
        assert!(tracked <= MAX_TRACKED_KEYS, "tracked {tracked} keys");
    }

    #[test]
    fn a_full_map_drops_the_key_with_the_oldest_window() {
        let limiter = SignedCallLimiter::new(60, 1);
        assert!(limiter.admit(b"oldest", 100));
        assert!(limiter.admit(b"newer", 101));
        for index in 0..(MAX_TRACKED_KEYS - 2) {
            limiter.admit(&index.to_be_bytes(), 102);
        }
        assert!(limiter.admit(b"new", 103));
        let state = limiter.state.lock().unwrap();
        assert_eq!(state.windows.len(), MAX_TRACKED_KEYS);
        assert!(!state.windows.contains_key(b"oldest".as_slice()));
        assert!(state.windows.contains_key(b"newer".as_slice()));
    }

    #[test]
    fn restarted_windows_do_not_grow_the_queue() {
        let limiter = SignedCallLimiter::new(1, 1);
        for now in 0..10_000 {
            assert!(limiter.admit(b"a", now));
        }
        let state = limiter.state.lock().unwrap();
        assert_eq!(state.windows.len(), 1);
        assert!(state.starts.len() <= 2, "queue has {}", state.starts.len());
    }
}
