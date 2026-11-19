//! Per-pod admission for replica-served subscription-history reads.
//!
//! History pages are the heaviest routed reads. Unbounded, a burst of them
//! fills the reader pool, every further routed read misses its reader acquire
//! budget, and before this gate those misses fell back to the writer, where
//! they starved the authentication, tenant and moderation lookups that only
//! the writer can serve. The gate holds running history transactions below the
//! reader pool's size, and a history read that cannot get a reader is rejected
//! as retryable instead of moved to the writer (see [`super::Db::route_read`]).
//!
//! The cap bounds history transactions, not reader connections. A permit is
//! released when its transaction is dropped, while sqlx still returns the
//! connection asynchronously through the pool's release check. Released
//! connections can therefore briefly hold the slots the cap leaves free for
//! authorization reads: usually one round trip, at most the release check's
//! timeout when reader sockets stall. Authorization reads that miss a reader
//! in that window fall back to the writer as before; history reads on the
//! saturated pool are rejected, not moved to the writer.

use std::time::Duration;

use crate::concurrency_gate::{ConcurrencyGate, GatePermit};

/// How long a history read waits for a permit before it is rejected. The
/// gate is FIFO, so this bounds queueing delay under a burst; a read that
/// would wait longer is shed and the client retries.
pub(crate) const PERMIT_WAIT: Duration = Duration::from_secs(1);

/// History transactions allowed at once: the pool less one fifth (at least
/// one) left to authorization and bootstrap reads, and never below one.
/// 25 → 20, 5 → 4, 2 → 1, 1 → 1. This is the gate's ceiling: an operator may
/// lower the limit at runtime but never raise it past the share that keeps
/// readers free for authorization.
pub(crate) fn cap(read_max_connections: u32) -> u32 {
    read_max_connections
        .saturating_sub((read_max_connections / 5).max(1))
        .max(1)
}

/// The history-read gate for a reader pool of `read_max_connections`, one per
/// `Db` (shared by its clones, so one per relay process).
pub(crate) fn gate_for_read_pool(read_max_connections: u32) -> ConcurrencyGate {
    ConcurrencyGate::new(cap(read_max_connections) as usize)
}

/// A proved reader transaction plus the history permit it was admitted
/// under, if any. The permit is released when the transaction is dropped,
/// wherever the transaction travels: a per-call routed read, or a
/// [`super::ReadSession`] the relay keeps for follow-up queries. The reader
/// connection itself returns to the pool asynchronously after that (see the
/// module docs).
pub(crate) struct ReaderTx {
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
    _permit: Option<GatePermit>,
}

impl ReaderTx {
    pub(crate) fn new(
        tx: sqlx::Transaction<'static, sqlx::Postgres>,
        permit: Option<GatePermit>,
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
    use super::cap;

    #[test]
    fn cap_leaves_a_fifth_of_the_reader_pool_and_never_reaches_zero() {
        for (pool, expected) in [(25, 20), (10, 8), (5, 4), (4, 3), (2, 1), (1, 1), (0, 1)] {
            assert_eq!(cap(pool), expected, "pool={pool}");
        }
    }
}
