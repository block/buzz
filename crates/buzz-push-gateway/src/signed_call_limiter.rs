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

use std::{collections::HashMap, sync::Mutex};

/// Most keys tracked at once. Past it, the key with the oldest window is
/// dropped, so memory stays bounded however many keys call.
const MAX_TRACKED_KEYS: usize = 100_000;

/// A fixed-window counter per attested key.
pub struct SignedCallLimiter {
    window_seconds: i64,
    max_calls: i64,
    windows: Mutex<HashMap<Vec<u8>, Window>>,
}

#[derive(Clone, Copy)]
struct Window {
    started_at: i64,
    calls: i64,
}

impl SignedCallLimiter {
    /// At most `max_calls` calls per key in each `window_seconds` window.
    pub fn new(window_seconds: i64, max_calls: i64) -> Self {
        Self {
            window_seconds,
            max_calls,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Count one call by `key` at `now`; false when the key is over its limit.
    pub fn admit(&self, key: &[u8], now: i64) -> bool {
        // The map holds only counters, so a poisoned lock is still consistent.
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !windows.contains_key(key) && windows.len() >= MAX_TRACKED_KEYS {
            let window_seconds = self.window_seconds;
            windows.retain(|_, w| now.saturating_sub(w.started_at) < window_seconds);
            if windows.len() >= MAX_TRACKED_KEYS {
                let oldest = windows
                    .iter()
                    .min_by_key(|(_, w)| w.started_at)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    windows.remove(&oldest);
                }
            }
        }
        let window = windows.entry(key.to_vec()).or_insert(Window {
            started_at: now,
            calls: 0,
        });
        if now.saturating_sub(window.started_at) >= self.window_seconds {
            *window = Window {
                started_at: now,
                calls: 0,
            };
        }
        if window.calls >= self.max_calls {
            return false;
        }
        window.calls += 1;
        true
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
        let tracked = limiter.windows.lock().unwrap().len();
        assert!(tracked <= MAX_TRACKED_KEYS, "tracked {tracked} keys");
    }
}
