//! A FIFO concurrency cap whose limit an operator can change at runtime.
//!
//! The gate is created with a ceiling, its startup limit. [`ConcurrencyGate::set_limit`]
//! moves the limit anywhere in `1..=ceiling` without a restart:
//!
//! - Raising it releases permits to waiting callers at once.
//! - Lowering it takes back permits nobody holds at once. Permits still held
//!   above the new limit are retired as their holders release them, so no
//!   in-flight work is cancelled and no new work is admitted until the count
//!   held falls under the new limit.
//!
//! Every limit change, every permit release and every admission settles
//! against the shrink debt under one lock, so a newer limit always supersedes
//! an older one: there is no background resize to race, and a permit tokio
//! returns from a cancelled waiter is retired rather than admitted.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// A concurrency cap with a runtime-adjustable limit. Clones share one cap.
#[derive(Clone, Debug)]
pub struct ConcurrencyGate {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    permits: Arc<Semaphore>,
    ceiling: usize,
    state: Mutex<LimitState>,
}

#[derive(Debug)]
struct LimitState {
    /// The limit last set: the permits that exist, held or free, once `debt`
    /// is repaid.
    limit: usize,
    /// Permits held above the current limit. Each release retires one of them
    /// instead of returning it to the semaphore.
    debt: usize,
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, LimitState> {
        // The guarded section never panics, but a poisoned lock must not
        // wedge admission for the rest of the process.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl ConcurrencyGate {
    /// A gate whose ceiling and startup limit are `ceiling`, raised to at
    /// least one and clamped to [`Semaphore::MAX_PERMITS`].
    pub fn new(ceiling: usize) -> Self {
        let ceiling = ceiling.clamp(1, Semaphore::MAX_PERMITS);
        Self {
            inner: Arc::new(Inner {
                permits: Arc::new(Semaphore::new(ceiling)),
                ceiling,
                state: Mutex::new(LimitState {
                    limit: ceiling,
                    debt: 0,
                }),
            }),
        }
    }

    /// A gate with no practical limit until one is set.
    pub fn unbounded() -> Self {
        Self::new(Semaphore::MAX_PERMITS)
    }

    /// The highest limit this gate accepts: its startup limit.
    pub fn ceiling(&self) -> usize {
        self.inner.ceiling
    }

    /// The current limit.
    pub fn limit(&self) -> usize {
        self.inner.state().limit
    }

    /// Set the limit to `requested`, clamped to `1..=ceiling`, and return the
    /// limit applied.
    pub fn set_limit(&self, requested: usize) -> usize {
        let target = requested.clamp(1, self.inner.ceiling);
        let mut state = self.inner.state();
        if target < state.limit {
            let excess = state.limit - target;
            let retired = self.inner.permits.forget_permits(excess);
            state.debt += excess - retired;
        } else if target > state.limit {
            let added = target - state.limit;
            let repaid = added.min(state.debt);
            state.debt -= repaid;
            self.inner.permits.add_permits(added - repaid);
        }
        state.limit = target;
        target
    }

    /// Wait up to `wait` for a permit, in arrival order. `None` means the
    /// caller must be shed.
    pub async fn admit(&self, wait: Duration) -> Option<GatePermit> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let permit =
                tokio::time::timeout_at(deadline, Arc::clone(&self.inner.permits).acquire_owned())
                    .await
                    .ok()?
                    .ok()?;
            let mut state = self.inner.state();
            if state.debt == 0 {
                drop(state);
                return Some(GatePermit {
                    permit: Some(permit),
                    inner: Arc::clone(&self.inner),
                });
            }
            // While there is debt every permit should be held, so this one
            // leaked back without a `GatePermit` release: tokio returns a
            // permit it had assigned to a waiter that was then cancelled.
            // Retire it and keep waiting.
            state.debt -= 1;
            permit.forget();
        }
    }

    /// Permits free to admit right now (tests).
    #[cfg(test)]
    pub(crate) fn available(&self) -> usize {
        self.inner.permits.available_permits()
    }
}

/// One admission through a [`ConcurrencyGate`], released on drop.
#[derive(Debug)]
pub struct GatePermit {
    permit: Option<OwnedSemaphorePermit>,
    inner: Arc<Inner>,
}

impl Drop for GatePermit {
    fn drop(&mut self) {
        let Some(permit) = self.permit.take() else {
            return;
        };
        let mut state = self.inner.state();
        if state.debt > 0 {
            state.debt -= 1;
            permit.forget();
        } else {
            // Release under the lock so a concurrent `set_limit` sees either
            // the debt or the returned permit, never neither.
            drop(permit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: Duration = Duration::from_secs(1);

    #[test]
    fn set_limit_clamps_to_one_through_the_ceiling() {
        let gate = ConcurrencyGate::new(4);
        assert_eq!(gate.set_limit(0), 1);
        assert_eq!(gate.set_limit(9), 4);
        assert_eq!(gate.limit(), 4);
        assert_eq!(ConcurrencyGate::new(0).ceiling(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_gate_sheds_after_the_wait_and_reopens_on_release() {
        let gate = ConcurrencyGate::new(1);
        let held = gate.admit(WAIT).await.expect("first caller is admitted");
        let started = tokio::time::Instant::now();
        assert!(gate.admit(WAIT).await.is_none(), "a full gate sheds");
        assert_eq!(started.elapsed(), WAIT);
        drop(held);
        assert!(gate.admit(WAIT).await.is_some(), "a release admits again");
    }

    /// Each row lowers or raises the limit while `held` permits are out, then
    /// releases them all. `admits` is how many more callers get in right after
    /// the change; `settled` is the free count once every holder has left.
    #[tokio::test(start_paused = true)]
    async fn limit_changes_respect_permits_already_held() {
        struct Case {
            name: &'static str,
            held: usize,
            limits: &'static [usize],
            admits: usize,
            settled: usize,
        }
        let cases = [
            Case {
                name: "lower while idle",
                held: 0,
                limits: &[2],
                admits: 2,
                settled: 2,
            },
            Case {
                name: "lower below the count held",
                held: 4,
                limits: &[2],
                admits: 0,
                settled: 2,
            },
            Case {
                name: "lower, then raise before holders leave",
                held: 4,
                limits: &[1, 3],
                admits: 0,
                settled: 3,
            },
            Case {
                name: "lower, then raise past the count held",
                held: 3,
                limits: &[1, 5],
                admits: 2,
                settled: 5,
            },
            Case {
                name: "raise back to the ceiling",
                held: 2,
                limits: &[2, 6],
                admits: 4,
                settled: 6,
            },
        ];
        for case in cases {
            let gate = ConcurrencyGate::new(6);
            let mut held = Vec::new();
            for _ in 0..case.held {
                held.push(gate.admit(WAIT).await.expect("under the ceiling"));
            }
            for &limit in case.limits {
                gate.set_limit(limit);
            }
            let mut admitted = Vec::new();
            while let Some(permit) = gate.admit(WAIT).await {
                admitted.push(permit);
            }
            assert_eq!(admitted.len(), case.admits, "{}: admitted", case.name);
            drop(admitted);
            drop(held);
            assert_eq!(gate.available(), case.settled, "{}: settled", case.name);
            assert_eq!(
                gate.limit(),
                case.settled,
                "{}: limit matches the permits that exist",
                case.name
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn raising_the_limit_wakes_a_waiting_caller() {
        let gate = ConcurrencyGate::new(2);
        gate.set_limit(1);
        let _held = gate.admit(WAIT).await.expect("one permit");
        let waiter = tokio::spawn({
            let gate = gate.clone();
            async move { gate.admit(WAIT).await.is_some() }
        });
        tokio::task::yield_now().await;
        gate.set_limit(2);
        assert!(waiter.await.expect("join"), "the raise admits the waiter");
    }

    /// Tokio hands a released permit to the first waiter before that waiter
    /// runs. If the waiter is cancelled after a shrink, tokio returns the
    /// permit to the semaphore directly, never through `GatePermit::drop`;
    /// the next admission must retire it against the debt instead.
    #[tokio::test(start_paused = true)]
    async fn a_cancelled_waiter_cannot_reopen_a_lowered_gate() {
        let gate = ConcurrencyGate::new(2);
        let first = gate.admit(WAIT).await.expect("first holder");
        let second = gate.admit(WAIT).await.expect("second holder");
        let waiter = tokio::spawn({
            let gate = gate.clone();
            async move { gate.admit(WAIT).await.is_some() }
        });
        tokio::task::yield_now().await; // the waiter queues
        drop(first); // tokio assigns this permit to the queued waiter
        gate.set_limit(1); // nothing free to forget: one permit of debt
        waiter.abort(); // cancelled before it runs again
        assert!(waiter.await.expect_err("aborted").is_cancelled());

        assert!(
            gate.admit(WAIT).await.is_none(),
            "one holder already fills limit 1"
        );
        drop(second);
        let admitted = gate.admit(WAIT).await.expect("the holder left");
        assert!(gate.admit(WAIT).await.is_none(), "limit 1 holds");
        drop(admitted);
        assert_eq!(
            gate.available(),
            1,
            "the permits that exist match the limit"
        );
    }

    #[test]
    fn an_unbounded_gate_can_be_capped() {
        let gate = ConcurrencyGate::unbounded();
        assert_eq!(gate.ceiling(), Semaphore::MAX_PERMITS);
        assert_eq!(gate.set_limit(3), 3);
        assert_eq!(gate.available(), 3);
    }
}
