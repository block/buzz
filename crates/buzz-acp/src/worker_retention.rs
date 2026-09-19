//! Resource lifetime of one serialized ACP worker (not the relay connection).
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct WorkerRetention {
    sessions_created: u32,
    idle_since: Option<Instant>,
}

impl WorkerRetention {
    pub(crate) fn session_created(&mut self) {
        self.sessions_created = self.sessions_created.saturating_add(1);
    }

    pub(crate) fn mark_idle(&mut self, now: Instant) {
        self.idle_since = Some(now);
    }

    pub(crate) fn expired(&self, now: Instant, ttl: Duration, max_sessions: u32) -> bool {
        // Fresh pool workers have no provider sessions to release. Do not churn.
        self.sessions_created > 0
            && self.idle_since.is_some_and(|idle_since| {
                (max_sessions > 0 && self.sessions_created >= max_sessions)
                    || (!ttl.is_zero() && now.saturating_duration_since(idle_since) >= ttl)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_retained_sessions_even_after_the_registry_forgets_them() {
        let now = Instant::now();
        let mut worker = WorkerRetention::default();
        for _ in 0..4 {
            worker.session_created();
        }
        assert!(!worker.expired(now, Duration::from_secs(300), 4));
        worker.mark_idle(now);
        assert!(worker.expired(now, Duration::from_secs(300), 4));
    }

    #[test]
    fn idle_expiry_resets_after_a_turn_and_never_churns_unused_workers() {
        let now = Instant::now();
        let ttl = Duration::from_secs(300);
        let mut worker = WorkerRetention::default();
        worker.mark_idle(now);
        assert!(!worker.expired(now + ttl, ttl, 4));
        worker.session_created();
        assert!(!worker.expired(now + ttl - Duration::from_secs(1), ttl, 4));
        assert!(worker.expired(now + ttl, ttl, 4));
        worker.mark_idle(now + ttl);
        assert!(!worker.expired(now + ttl, ttl, 4));
        assert!(!worker.expired(now + ttl * 10, Duration::ZERO, 0));
    }
}
