//! Lazy agent-pool lifecycle state.
//!
//! Relay connection, subscription, and event buffering live outside this
//! module. This state machine owns only whether a deferred pool has not started,
//! is waking, is ready, or is waiting to retry after a failed wake.

use std::time::Duration;
use tokio::time::Instant;

const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(5);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub(crate) enum PoolLifecycle<P> {
    Listening,
    Waking {
        attempt: u32,
    },
    Ready(P),
    Failed {
        attempt: u32,
        retry_at: Instant,
        error: String,
    },
}

impl<P> PoolLifecycle<P> {
    pub(crate) fn listening() -> Self {
        Self::Listening
    }

    /// Start the first wake, or a due retry, when buffered work exists.
    ///
    /// Returns the attempt token exactly once per transition into `Waking`;
    /// callers attach it to the single pool-initialization task and return it
    /// with the result.
    pub(crate) fn start_wake_if_due(
        &mut self,
        has_pending_work: bool,
        now: Instant,
    ) -> Option<u32> {
        if !has_pending_work {
            return None;
        }

        let next_attempt = match self {
            Self::Listening => Some(1),
            Self::Failed {
                attempt, retry_at, ..
            } if now >= *retry_at => Some(attempt.saturating_add(1)),
            Self::Waking { .. } | Self::Ready(_) | Self::Failed { .. } => None,
        };

        if let Some(attempt) = next_attempt {
            *self = Self::Waking { attempt };
        }
        next_attempt
    }

    pub(crate) fn take_ready(&mut self) -> Option<P> {
        match std::mem::replace(self, Self::Listening) {
            Self::Ready(pool) => Some(pool),
            other => {
                *self = other;
                None
            }
        }
    }

    pub(crate) fn waking_attempt(&self) -> Option<u32> {
        match self {
            Self::Waking { attempt } => Some(*attempt),
            _ => None,
        }
    }

    pub(crate) fn retry_at(&self) -> Option<Instant> {
        match self {
            Self::Failed { retry_at, .. } => Some(*retry_at),
            _ => None,
        }
    }

    pub(crate) fn failed_error(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }

    pub(crate) fn cancel_wake(&mut self, attempt: u32, error: String, now: Instant) -> bool {
        self.complete_wake(attempt, Err(error), now).is_ok()
    }

    /// Complete the matching in-flight wake attempt.
    ///
    /// A failure remains retryable. A result returned outside `Waking`, or from
    /// an older attempt, is rejected: accepting it could replace a newer pool.
    pub(crate) fn complete_wake(
        &mut self,
        completed_attempt: u32,
        result: Result<P, String>,
        now: Instant,
    ) -> Result<(), &'static str> {
        let attempt = match self {
            Self::Waking { attempt } if *attempt == completed_attempt => *attempt,
            Self::Waking { .. } => return Err("wake result attempt did not match Waking attempt"),
            _ => return Err("wake completed while lifecycle was not Waking"),
        };

        *self = match result {
            Ok(pool) => Self::Ready(pool),
            Err(error) => Self::Failed {
                attempt,
                retry_at: now + retry_delay(attempt),
                error,
            },
        };
        Ok(())
    }
}

fn retry_delay(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(63);
    let multiplier = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
    Duration::from_secs(
        INITIAL_RETRY_DELAY
            .as_secs()
            .saturating_mul(multiplier)
            .min(MAX_RETRY_DELAY.as_secs()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn first_pending_event_starts_exactly_one_wake() {
        let now = Instant::now();
        let mut lifecycle = PoolLifecycle::<()>::listening();

        assert_eq!(lifecycle.start_wake_if_due(false, now), None);
        assert_eq!(lifecycle.start_wake_if_due(true, now), Some(1));
        assert_eq!(lifecycle.start_wake_if_due(true, now), None);
        assert!(matches!(lifecycle, PoolLifecycle::Waking { attempt: 1 }));
    }

    #[tokio::test(start_paused = true)]
    async fn failure_retries_only_when_work_exists_and_deadline_is_due() {
        let now = Instant::now();
        let mut lifecycle = PoolLifecycle::<()>::listening();
        assert_eq!(lifecycle.start_wake_if_due(true, now), Some(1));
        lifecycle
            .complete_wake(1, Err("provider unavailable".into()), now)
            .unwrap();

        assert_eq!(
            lifecycle.start_wake_if_due(true, now + Duration::from_secs(4)),
            None
        );
        assert_eq!(
            lifecycle.start_wake_if_due(false, now + Duration::from_secs(5)),
            None
        );
        assert_eq!(
            lifecycle.start_wake_if_due(true, now + Duration::from_secs(5)),
            Some(2)
        );
        assert!(matches!(lifecycle, PoolLifecycle::Waking { attempt: 2 }));
    }

    #[tokio::test(start_paused = true)]
    async fn retry_backoff_doubles_and_caps_at_five_minutes() {
        let mut now = Instant::now();
        let mut lifecycle = PoolLifecycle::<()>::listening();

        for attempt in 1..=9 {
            assert_eq!(lifecycle.start_wake_if_due(true, now), Some(attempt));
            assert!(matches!(
                lifecycle,
                PoolLifecycle::Waking { attempt: actual } if actual == attempt
            ));
            lifecycle
                .complete_wake(attempt, Err("no brain".into()), now)
                .unwrap();

            let expected = retry_delay(attempt);
            let retry_at = match &lifecycle {
                PoolLifecycle::Failed { retry_at, .. } => *retry_at,
                _ => panic!("failure must enter Failed"),
            };
            assert_eq!(retry_at, now + expected);
            assert!(expected <= MAX_RETRY_DELAY);
            now = retry_at;
        }

        assert_eq!(retry_delay(7), MAX_RETRY_DELAY);
        assert_eq!(retry_delay(u32::MAX), MAX_RETRY_DELAY);
    }

    #[tokio::test(start_paused = true)]
    async fn successful_retry_consumes_pool_and_stops_future_wakes() {
        let now = Instant::now();
        let mut lifecycle = PoolLifecycle::listening();
        assert_eq!(lifecycle.start_wake_if_due(true, now), Some(1));
        lifecycle
            .complete_wake(1, Err("first attempt failed".into()), now)
            .unwrap();

        let retry_at = match &lifecycle {
            PoolLifecycle::Failed { retry_at, .. } => *retry_at,
            _ => panic!("expected Failed"),
        };
        assert_eq!(lifecycle.start_wake_if_due(true, retry_at), Some(2));
        lifecycle.complete_wake(2, Ok("pool"), retry_at).unwrap();

        assert!(matches!(lifecycle, PoolLifecycle::Ready("pool")));
        assert_eq!(
            lifecycle.start_wake_if_due(true, retry_at + Duration::from_secs(600)),
            None
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stale_or_duplicate_wake_result_is_rejected() {
        let now = Instant::now();
        let mut lifecycle = PoolLifecycle::<()>::listening();
        assert_eq!(
            lifecycle.complete_wake(1, Ok(()), now),
            Err("wake completed while lifecycle was not Waking")
        );

        assert_eq!(lifecycle.start_wake_if_due(true, now), Some(1));
        lifecycle.complete_wake(1, Ok(()), now).unwrap();
        assert_eq!(
            lifecycle.complete_wake(1, Ok(()), now),
            Err("wake completed while lifecycle was not Waking")
        );
        assert!(matches!(lifecycle, PoolLifecycle::Ready(())));
    }

    #[tokio::test(start_paused = true)]
    async fn stale_attempt_result_cannot_replace_current_wake() {
        let now = Instant::now();
        let mut lifecycle = PoolLifecycle::<&str>::listening();
        assert_eq!(lifecycle.start_wake_if_due(true, now), Some(1));
        lifecycle
            .complete_wake(1, Err("attempt one failed".into()), now)
            .unwrap();

        let retry_at = match &lifecycle {
            PoolLifecycle::Failed { retry_at, .. } => *retry_at,
            _ => panic!("expected Failed"),
        };
        assert_eq!(lifecycle.start_wake_if_due(true, retry_at), Some(2));
        assert_eq!(
            lifecycle.complete_wake(1, Ok("stale pool"), retry_at),
            Err("wake result attempt did not match Waking attempt")
        );
        assert!(matches!(lifecycle, PoolLifecycle::Waking { attempt: 2 }));
        lifecycle
            .complete_wake(2, Ok("current pool"), retry_at)
            .unwrap();
        assert!(matches!(lifecycle, PoolLifecycle::Ready("current pool")));
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_wake_enters_failed_and_can_retry() {
        let now = Instant::now();
        let mut lifecycle = PoolLifecycle::<()>::listening();
        assert_eq!(lifecycle.start_wake_if_due(true, now), Some(1));
        assert_eq!(lifecycle.waking_attempt(), Some(1));
        assert!(lifecycle.cancel_wake(1, "task panicked".into(), now));
        assert_eq!(lifecycle.failed_error(), Some("task panicked"));
        assert_eq!(
            lifecycle.start_wake_if_due(true, now + Duration::from_secs(5)),
            Some(2)
        );
    }

    #[test]
    fn take_ready_transfers_pool_exactly_once() {
        let now = Instant::now();
        let mut lifecycle = PoolLifecycle::listening();
        assert_eq!(lifecycle.start_wake_if_due(true, now), Some(1));
        lifecycle.complete_wake(1, Ok("pool"), now).unwrap();
        assert_eq!(lifecycle.take_ready(), Some("pool"));
        assert_eq!(lifecycle.take_ready(), None);
    }

    #[test]
    fn failed_state_preserves_attempt_deadline_and_error() {
        let now = Instant::now();
        let mut lifecycle = PoolLifecycle::<()>::listening();
        assert_eq!(lifecycle.start_wake_if_due(true, now), Some(1));
        lifecycle.complete_wake(1, Err("boom".into()), now).unwrap();

        match lifecycle {
            PoolLifecycle::Failed {
                attempt,
                retry_at,
                error,
            } => {
                assert_eq!(attempt, 1);
                assert_eq!(retry_at, now + Duration::from_secs(5));
                assert_eq!(error, "boom");
            }
            _ => panic!("expected Failed"),
        }
    }
}
