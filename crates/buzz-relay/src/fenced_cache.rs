//! Short-lived read caches for connect and auth paths, fenced against
//! invalidation.
//!
//! An entry is served for the cache's `max_age` after the read that produced
//! it checked out its connection (see [`buzz_db::ReadAt`]), and concurrent
//! misses for one key share one read through [`SharedReads`].
//!
//! A read that began before an invalidation must not refill the entry the
//! invalidation dropped: it can return the state the invalidating write just
//! replaced. Every invalidation bumps an epoch, and a fill is published only
//! if the epoch it observed before its read is still current. A later caller
//! therefore reads afresh instead of taking a pre-invalidation answer from the
//! cache or from a shared in-flight read.

use std::future::Future;
use std::hash::Hash;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::shared_reads::SharedReads;

#[derive(Clone)]
struct Entry<V> {
    value: V,
    read_at: Instant,
}

/// A bounded read cache whose fills are fenced by invalidation.
pub(crate) struct FencedCache<K, V> {
    entries: moka::sync::Cache<K, Entry<V>>,
    reads: SharedReads<K>,
    // Serializes invalidation with fill publication. One epoch for every key
    // keeps no per-key tombstones; an unrelated invalidation only skips
    // publishing an in-flight fill.
    epoch: Mutex<u64>,
    max_age: Duration,
    hits: &'static str,
    misses: &'static str,
}

impl<K, V> FencedCache<K, V>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Up to `capacity` entries, each served for `max_age` after its read's
    /// connection checkout. Concurrent misses wait at most `shared_read_wait`
    /// for a shared read (see [`SharedReads::new`]). `hits` and `misses` name
    /// the counters this cache reports.
    pub(crate) fn new(
        capacity: u64,
        max_age: Duration,
        shared_read_wait: Duration,
        (hits, misses): (&'static str, &'static str),
    ) -> Self {
        Self {
            // Readers use an entry only for `max_age` after its read began,
            // which is never later than its insert; the TTL just reclaims
            // memory.
            entries: moka::sync::Cache::builder()
                .max_capacity(capacity)
                .time_to_live(max_age)
                .support_invalidation_closures()
                .build(),
            reads: SharedReads::new(capacity, shared_read_wait),
            epoch: Mutex::new(0),
            max_age,
            hits,
            misses,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, u64> {
        self.epoch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn fresh(&self, key: &K) -> Option<V> {
        self.entries
            .get(key)
            .filter(|entry| entry.read_at.elapsed() < self.max_age)
            .map(|entry| entry.value)
    }

    /// `key`'s value from the cache while it is fresh, else from `read`.
    /// Errors are not cached.
    ///
    /// The first concurrent miss reads; the rest wait for it and use its
    /// entry. A waiter whose leader failed, was cancelled, was fenced by an
    /// invalidation or is still reading after the gate's wait reads on its
    /// own.
    pub(crate) async fn get<Read, ReadFuture>(
        &self,
        key: K,
        read: Read,
    ) -> Result<V, buzz_db::DbError>
    where
        Read: FnOnce() -> ReadFuture,
        ReadFuture: Future<Output = Result<buzz_db::ReadAt<V>, buzz_db::DbError>>,
    {
        if let Some(value) = self.fresh(&key) {
            metrics::counter!(self.hits).increment(1);
            return Ok(value);
        }
        let _turn = self.reads.turn(key.clone()).await;
        if let Some(value) = self.fresh(&key) {
            metrics::counter!(self.hits).increment(1);
            return Ok(value);
        }
        metrics::counter!(self.misses).increment(1);
        // Observed before the read checks out its connection, so a read that
        // could predate an invalidation always sees a newer epoch.
        let observed = *self.lock();
        let read = read().await?;
        let epoch = self.lock();
        if *epoch == observed {
            self.entries.insert(
                key,
                Entry {
                    value: read.value.clone(),
                    read_at: read.read_at,
                },
            );
        }
        Ok(read.value)
    }

    /// Drops `key`'s entry and fences every fill begun before this.
    pub(crate) fn invalidate(&self, key: &K) {
        let mut epoch = self.lock();
        *epoch = epoch.wrapping_add(1);
        self.entries.invalidate(key);
    }

    /// Drops every entry whose key matches `predicate` and fences every fill
    /// begun before this.
    pub(crate) fn invalidate_if(&self, predicate: impl Fn(&K) -> bool + Send + Sync + 'static) {
        let mut epoch = self.lock();
        *epoch = epoch.wrapping_add(1);
        if let Err(error) = self
            .entries
            .invalidate_entries_if(move |key, _| predicate(key))
        {
            // Invalidation closures are enabled at construction. If that ever
            // regresses, over-invalidate rather than serve stale state.
            tracing::error!(
                ?error,
                "scoped cache invalidation unavailable; dropping all entries"
            );
            self.entries.invalidate_all();
        }
    }

    /// Seeds a fresh entry without touching the database.
    #[cfg(test)]
    pub(crate) fn seed(&self, key: K, value: V) {
        self.seed_read_at(key, value, Instant::now());
    }

    /// Seeds an entry whose read checked out its connection at `read_at`.
    #[cfg(test)]
    pub(crate) fn seed_read_at(&self, key: K, value: V, read_at: Instant) {
        self.entries.insert(key, Entry { value, read_at });
    }

    /// The cached value and its read time, fresh or not.
    #[cfg(test)]
    pub(crate) fn entry(&self, key: &K) -> Option<(V, Instant)> {
        self.entries
            .get(key)
            .map(|entry| (entry.value, entry.read_at))
    }

    /// The shared-read gates, so tests can hold or count a key's gate.
    #[cfg(test)]
    pub(crate) fn reads(&self) -> &SharedReads<K> {
        &self.reads
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    const MAX_AGE: Duration = Duration::from_secs(10);

    fn cache() -> Arc<FencedCache<u8, bool>> {
        Arc::new(FencedCache::new(
            16,
            MAX_AGE,
            Duration::from_secs(5),
            (
                "test_fenced_cache_hits_total",
                "test_fenced_cache_misses_total",
            ),
        ))
    }

    fn read_at(value: bool) -> Result<buzz_db::ReadAt<bool>, buzz_db::DbError> {
        Ok(buzz_db::ReadAt {
            value,
            read_at: Instant::now(),
        })
    }

    /// Tasks holding or waiting on `key`'s gate: its leader and every waiter
    /// queued behind it (the cache's own clone and this probe's are
    /// excluded).
    fn gate_holders(cache: &FencedCache<u8, bool>, key: u8) -> usize {
        cache
            .reads
            .gate(&key)
            .map_or(0, |gate| Arc::strong_count(&gate) - 2)
    }

    /// Waits, at most 5 s, until `ready` holds.
    async fn until(ready: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("readiness");
    }

    #[tokio::test]
    async fn concurrent_misses_share_one_read_and_expire_after_max_age() {
        let cache = cache();
        let reads = Arc::new(AtomicUsize::new(0));
        let (release, released) = tokio::sync::watch::channel(false);

        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let reads = Arc::clone(&reads);
                let mut released = released.clone();
                tokio::spawn(async move {
                    cache
                        .get(1, || async move {
                            reads.fetch_add(1, Ordering::SeqCst);
                            let _ = released.wait_for(|go| *go).await;
                            read_at(true)
                        })
                        .await
                })
            })
            .collect();
        // The leader is in its read and the other 15 wait behind its gate.
        until(|| reads.load(Ordering::SeqCst) == 1 && gate_holders(&cache, 1) == 16).await;
        release.send_replace(true);
        for task in tasks {
            assert!(task.await.expect("task").expect("read"));
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1, "misses must share a read");

        // Just inside the max age the entry is served without a read.
        cache.seed_read_at(1, true, Instant::now() - (MAX_AGE - Duration::from_secs(1)));
        assert!(cache
            .get(1, || async { panic!("a fresh entry must be served") })
            .await
            .expect("hit"));

        // Past the max age the entry is stale and the next caller reads.
        cache.seed_read_at(1, true, Instant::now() - MAX_AGE);
        let refreshed = cache
            .get(1, || async {
                reads.fetch_add(1, Ordering::SeqCst);
                read_at(false)
            })
            .await
            .expect("read");
        assert!(!refreshed, "an expired entry must not be served");
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    /// Mutation: drop the epoch check from `get` → RED.
    #[tokio::test]
    async fn invalidation_fences_an_in_flight_fill_and_its_waiters() {
        let cache = cache();
        let (release, mut released) = tokio::sync::watch::channel(false);
        let leader_reading = Arc::new(AtomicUsize::new(0));

        // A leader reads `true` from a snapshot that predates the write.
        let leader = {
            let cache = Arc::clone(&cache);
            let leader_reading = Arc::clone(&leader_reading);
            tokio::spawn(async move {
                cache
                    .get(1, || async move {
                        leader_reading.fetch_add(1, Ordering::SeqCst);
                        let _ = released.wait_for(|go| *go).await;
                        read_at(true)
                    })
                    .await
            })
        };
        // In its read, so it observed the epoch before the invalidation.
        until(|| leader_reading.load(Ordering::SeqCst) == 1).await;
        // A caller that arrives after the write joins the in-flight read.
        let fresh_reads = Arc::new(AtomicUsize::new(0));
        let waiter = {
            let cache = Arc::clone(&cache);
            let fresh_reads = Arc::clone(&fresh_reads);
            tokio::spawn(async move {
                cache
                    .get(1, || async move {
                        fresh_reads.fetch_add(1, Ordering::SeqCst);
                        read_at(false)
                    })
                    .await
            })
        };
        // Queued behind the leader's gate before the invalidation.
        until(|| gate_holders(&cache, 1) == 2).await;
        cache.invalidate(&1);
        release.send_replace(true);

        assert!(leader.await.expect("leader").expect("read"));
        assert!(
            !waiter.await.expect("waiter").expect("read"),
            "a post-invalidation caller must not take a pre-invalidation answer",
        );
        assert_eq!(fresh_reads.load(Ordering::SeqCst), 1);
        assert_eq!(
            cache.entry(&1).map(|(value, _)| value),
            Some(false),
            "only the post-invalidation read may fill the cache",
        );
    }

    /// A scoped invalidation drops only matching keys and also fences fills.
    /// Mutation: bump no epoch in `invalidate_if` → RED.
    #[tokio::test]
    async fn scoped_invalidation_drops_matches_and_fences_fills() {
        let cache = cache();
        cache.seed(1, true);
        cache.seed(2, true);
        cache.invalidate_if(|key| *key == 1);
        cache.entries.run_pending_tasks();
        assert!(cache.entry(&1).is_none(), "the matching key is dropped");
        assert!(cache.entry(&2).is_some(), "other keys stay");

        // A fill that began before a scoped invalidation is not published.
        let filled = cache
            .get(3, || async {
                cache.invalidate_if(|key| *key == 9);
                read_at(true)
            })
            .await
            .expect("read");
        assert!(filled, "the caller still gets its own read");
        assert!(cache.entry(&3).is_none(), "the fenced fill is not cached");
    }

    #[tokio::test]
    async fn errors_are_not_cached() {
        let cache = cache();
        let failed = cache
            .get(1, || async { Err(buzz_db::DbError::AuthEventRejected) })
            .await;
        assert!(failed.is_err());
        assert!(cache.entry(&1).is_none());
    }
}
