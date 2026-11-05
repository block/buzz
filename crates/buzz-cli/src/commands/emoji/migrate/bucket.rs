//! Token bucket that paces a rate evenly (no burst) against an injected clock.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;

/// Time source. `now` is a monotonic offset; `sleep` waits that long.
pub trait Clock: Send + Sync {
    fn now(&self) -> Duration;
    fn sleep(&self, d: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// Wall-clock implementation for production.
#[derive(Clone)]
pub struct SystemClock {
    start: std::time::Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    fn sleep(&self, d: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(d))
    }
}

struct State {
    next_free: Duration,
    paused_until: Duration,
}

/// Grants one token every `60 / per_minute` seconds and honors server pauses.
pub struct TokenBucket<C: Clock> {
    clock: C,
    interval: Duration,
    state: Mutex<State>,
}

impl<C: Clock> TokenBucket<C> {
    /// `per_minute` must be at least 1.
    pub fn new(clock: C, per_minute: u32) -> Self {
        let interval = Duration::from_secs_f64(60.0 / f64::from(per_minute.max(1)));
        Self {
            clock,
            interval,
            state: Mutex::new(State {
                next_free: Duration::ZERO,
                paused_until: Duration::ZERO,
            }),
        }
    }

    /// Reserve the next slot and return how long the caller must wait for it.
    fn reserve(&self) -> Duration {
        let now = self.clock.now();
        let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let slot = st.next_free.max(st.paused_until).max(now);
        st.next_free = slot + self.interval;
        slot - now
    }

    /// Wait for the next slot.
    pub async fn acquire(&self) {
        let wait = self.reserve();
        if !wait.is_zero() {
            self.clock.sleep(wait).await;
        }
    }

    /// Hold every slot not yet started until `d` from now (a server `retry in Ns` hint).
    pub fn pause_for(&self, d: Duration) {
        let until = self.clock.now() + d;
        let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        st.paused_until = st.paused_until.max(until);
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// Clock whose `sleep` advances time instead of waiting.
    #[derive(Clone, Default)]
    pub struct FakeClock {
        micros: Arc<AtomicU64>,
    }

    impl FakeClock {
        pub fn advance(&self, d: Duration) {
            self.micros
                .fetch_add(d.as_micros() as u64, Ordering::SeqCst);
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Duration {
            Duration::from_micros(self.micros.load(Ordering::SeqCst))
        }

        fn sleep(&self, d: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.advance(d);
            Box::pin(std::future::ready(()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::FakeClock;
    use super::*;

    #[tokio::test]
    async fn paces_evenly_at_the_configured_rate() {
        let clock = FakeClock::default();
        let bucket = TokenBucket::new(clock.clone(), 30);
        let mut stamps = Vec::new();
        for _ in 0..4 {
            bucket.acquire().await;
            stamps.push(clock.now());
        }
        assert_eq!(stamps[0], Duration::ZERO, "first slot is immediate");
        for pair in stamps.windows(2) {
            assert_eq!(pair[1] - pair[0], Duration::from_secs(2));
        }
    }

    #[tokio::test]
    async fn never_exceeds_the_rate_in_any_minute() {
        let clock = FakeClock::default();
        let bucket = TokenBucket::new(clock.clone(), 240);
        let mut granted = 0;
        while clock.now() < Duration::from_secs(60) {
            bucket.acquire().await;
            if clock.now() < Duration::from_secs(60) {
                granted += 1;
            }
        }
        assert!(granted <= 240, "granted {granted}");
        assert!(granted >= 239, "granted {granted}");
    }

    #[tokio::test]
    async fn idle_time_does_not_bank_a_burst() {
        let clock = FakeClock::default();
        let bucket = TokenBucket::new(clock.clone(), 60);
        bucket.acquire().await;
        clock.advance(Duration::from_secs(600));
        let start = clock.now();
        bucket.acquire().await;
        bucket.acquire().await;
        bucket.acquire().await;
        assert_eq!(clock.now() - start, Duration::from_secs(2));
    }

    #[tokio::test]
    async fn server_pause_delays_the_next_slot() {
        let clock = FakeClock::default();
        let bucket = TokenBucket::new(clock.clone(), 60);
        bucket.acquire().await;
        bucket.pause_for(Duration::from_secs(20));
        let before = clock.now();
        bucket.acquire().await;
        assert_eq!(clock.now() - before, Duration::from_secs(20));
        bucket.pause_for(Duration::from_secs(3));
        bucket.pause_for(Duration::from_secs(1));
        let before = clock.now();
        bucket.acquire().await;
        assert_eq!(
            clock.now() - before,
            Duration::from_secs(3),
            "longest pause wins"
        );
    }
}
