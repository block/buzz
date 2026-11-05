//! Per-pod admission for replica-served subscription-history reads.
//!
//! History pages are the heaviest routed reads. Unbounded, a burst of them
//! fills the reader pool, every further routed read misses its reader acquire
//! budget, and before this gate those misses fell back to the writer, where
//! they starved the authentication, tenant and moderation lookups that only
//! the writer can serve. The gate holds history reads below the reader pool's
//! size, so a slot stays free for authorization reads, and a history read that
//! cannot get a reader is rejected as retryable instead of moved to the writer
//! (see [`super::Db::route_read`]).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Concurrency cap on history reads holding a reader connection, one per
/// `Db` (shared by its clones, so one per relay process).
#[derive(Clone, Debug)]
pub(crate) struct HistoryReadGate {
    permits: Arc<Semaphore>,
}

impl HistoryReadGate {
    /// How long a history read waits for a permit before it is rejected. The
    /// semaphore is FIFO, so this bounds queueing delay under a burst; a read
    /// that would wait longer is shed and the client retries.
    pub(crate) const PERMIT_WAIT: Duration = Duration::from_secs(1);

    /// A gate for a reader pool of `read_max_connections`.
    pub(crate) fn for_read_pool(read_max_connections: u32) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(Self::cap(read_max_connections) as usize)),
        }
    }

    /// History reads allowed to hold a reader connection at once: the pool
    /// less one fifth (at least one) kept for authorization and bootstrap
    /// reads, and never below one. 25 → 20, 5 → 4, 2 → 1, 1 → 1.
    pub(crate) fn cap(read_max_connections: u32) -> u32 {
        read_max_connections
            .saturating_sub((read_max_connections / 5).max(1))
            .max(1)
    }

    /// Wait up to [`Self::PERMIT_WAIT`] for a permit. `None` means the read
    /// must be rejected.
    pub(crate) async fn admit(&self) -> Option<OwnedSemaphorePermit> {
        tokio::time::timeout(Self::PERMIT_WAIT, self.permits.clone().acquire_owned())
            .await
            .ok()?
            .ok()
    }

    /// Permits not currently held (tests).
    #[cfg(test)]
    pub(crate) fn available(&self) -> usize {
        self.permits.available_permits()
    }
}

/// A proved reader transaction plus the history permit it was admitted
/// under, if any. The permit is released exactly when the transaction (and
/// so its reader connection) is dropped, wherever the transaction travels:
/// a per-call routed read, or a [`super::ReadSession`] the relay keeps for
/// follow-up queries.
pub(crate) struct ReaderTx {
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
    _permit: Option<OwnedSemaphorePermit>,
}

impl ReaderTx {
    pub(crate) fn new(
        tx: sqlx::Transaction<'static, sqlx::Postgres>,
        permit: Option<OwnedSemaphorePermit>,
    ) -> Self {
        Self {
            tx,
            _permit: permit,
        }
    }
}

impl std::ops::Deref for ReaderTx {
    type Target = sqlx::Transaction<'static, sqlx::Postgres>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl std::ops::DerefMut for ReaderTx {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tx
    }
}

#[cfg(test)]
mod tests {
    use super::HistoryReadGate;

    #[test]
    fn cap_leaves_a_fifth_of_the_reader_pool_and_never_reaches_zero() {
        for (pool, cap) in [(25, 20), (10, 8), (5, 4), (4, 3), (2, 1), (1, 1), (0, 1)] {
            assert_eq!(HistoryReadGate::cap(pool), cap, "pool={pool}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn admit_rejects_after_the_bounded_wait_and_reopens_on_release() {
        let gate = HistoryReadGate::for_read_pool(1);
        let held = gate.admit().await.expect("first read is admitted");
        let started = tokio::time::Instant::now();
        assert!(gate.admit().await.is_none(), "a full gate rejects");
        assert_eq!(started.elapsed(), HistoryReadGate::PERMIT_WAIT);
        drop(held);
        assert!(
            gate.admit().await.is_some(),
            "a released permit admits again"
        );
    }
}
