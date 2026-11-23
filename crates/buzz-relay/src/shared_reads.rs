//! Per-key gates that let concurrent misses on a short-lived read cache share
//! one database read.
//!
//! A reconnect storm sends many requests for the same key at once (every
//! socket on one host resolves the same tenant; a desktop client checks its
//! own restriction state on every request). Without a gate each miss issues
//! its own read, and the reads queue for the same pool connections. With one,
//! the first miss reads and the rest wait for it and use its cache entry.

use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, OwnedMutexGuard};

/// One gate per key, held by the caller that is reading that key.
pub(crate) struct SharedReads<K> {
    gates: moka::sync::Cache<K, Arc<Mutex<()>>>,
    wait: Duration,
}

impl<K> SharedReads<K>
where
    K: Hash + Eq + Send + Sync + 'static,
{
    /// Gates for up to `max_capacity` keys, each waited on for at most
    /// `wait`; an idle gate is dropped after a minute.
    ///
    /// `wait` must cover the holder's wait for a pool connection, which is
    /// what grows in a storm, plus its statement: a waiter that gives up
    /// while the holder still queues for a connection only queues behind it
    /// with a duplicate read.
    pub(crate) fn new(max_capacity: u64, wait: Duration) -> Self {
        Self {
            gates: moka::sync::Cache::builder()
                .max_capacity(max_capacity)
                .time_to_idle(Duration::from_secs(60))
                .build(),
            wait,
        }
    }

    /// Takes `key`'s gate, or waits for its holder to finish.
    ///
    /// Returns the guard when no read of `key` is in flight: the caller reads
    /// and holds the guard until its cache entry is written. Otherwise waits
    /// for the holder, at most this gate's `wait`, and returns `None`; the
    /// caller then rechecks its cache and, if the holder failed, was
    /// cancelled or is still reading, reads on its own without the gate. A
    /// read that hangs therefore stalls each waiter for at most `wait`.
    pub(crate) async fn turn(&self, key: K) -> Option<OwnedMutexGuard<()>> {
        let gate = self.gates.get_with(key, || Arc::new(Mutex::new(())));
        match Arc::clone(&gate).try_lock_owned() {
            Ok(guard) => Some(guard),
            Err(_) => {
                drop(tokio::time::timeout(self.wait, gate.lock()).await);
                None
            }
        }
    }

    /// The gate for `key`, if one exists.
    #[cfg(test)]
    pub(crate) fn gate(&self, key: &K) -> Option<Arc<Mutex<()>>> {
        self.gates.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::SharedReads;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn one_caller_reads_while_the_rest_wait_for_it() {
        let reads = Arc::new(SharedReads::<u8>::new(16, Duration::from_secs(5)));
        let leader = reads.turn(1).await;
        assert!(leader.is_some(), "the first caller takes the gate");

        // A second key has its own gate.
        assert!(reads.turn(2).await.is_some());

        let waiter = {
            let reads = Arc::clone(&reads);
            tokio::spawn(async move { reads.turn(1).await })
        };
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished(), "a waiter blocks while the read runs");
        drop(leader);
        assert!(
            waiter.await.expect("join").is_none(),
            "a waiter rechecks the cache instead of becoming a reader"
        );

        // Once the gate is free the next caller reads again.
        assert!(reads.turn(1).await.is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiter_stops_waiting_for_a_hung_read() {
        let reads = SharedReads::<u8>::new(16, Duration::from_secs(4));
        let _hung = reads.turn(1).await;
        let started = tokio::time::Instant::now();
        assert!(reads.turn(1).await.is_none());
        assert_eq!(started.elapsed(), Duration::from_secs(4));
    }
}
