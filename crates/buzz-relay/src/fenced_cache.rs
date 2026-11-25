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
//!
//! A cache built with a [`Fallback`] also answers a read that fails with a
//! database error from a recent entry its fallback accepts, for up to the
//! fallback's grace past `max_age`. A successful read always wins, and an
//! invalidation drops the entries a fallback would serve.

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

/// Which entries a [`FencedCache`] may serve when a read fails with a database
/// error, and for how long past its `max_age`.
pub(crate) struct Fallback<V> {
    /// How long past `max_age` an accepted entry is still served.
    pub(crate) grace: Duration,
    /// Whether an entry's value may be served on a failed read.
    pub(crate) accepts: fn(&V) -> bool,
    /// The counter of failed reads answered from an entry.
    pub(crate) served: &'static str,
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
    fallback: Option<Fallback<V>>,
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
        counters: (&'static str, &'static str),
    ) -> Self {
        Self::build(capacity, max_age, shared_read_wait, counters, None)
    }

    /// [`Self::new`], also answering a failed read from an entry `fallback`
    /// accepts while it is younger than `max_age` plus the fallback's grace.
    pub(crate) fn with_fallback(
        capacity: u64,
        max_age: Duration,
        shared_read_wait: Duration,
        counters: (&'static str, &'static str),
        fallback: Fallback<V>,
    ) -> Self {
        Self::build(
            capacity,
            max_age,
            shared_read_wait,
            counters,
            Some(fallback),
        )
    }

    fn build(
        capacity: u64,
        max_age: Duration,
        shared_read_wait: Duration,
        (hits, misses): (&'static str, &'static str),
        fallback: Option<Fallback<V>>,
    ) -> Self {
        let kept_for = max_age + fallback.as_ref().map_or(Duration::ZERO, |f| f.grace);
        Self {
            // Readers use an entry only for `max_age` (or a fallback's grace
            // past it) after its read began, which is never later than its
            // insert; the TTL just reclaims memory.
            entries: moka::sync::Cache::builder()
                .max_capacity(capacity)
                .time_to_live(kept_for)
                .support_invalidation_closures()
                .build(),
            reads: SharedReads::new(capacity, shared_read_wait),
            epoch: Mutex::new(0),
            max_age,
            hits,
            misses,
            fallback,
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

    /// `error` unless it is a database error and the fallback accepts `key`'s
    /// entry, which is then served while younger than `max_age` plus grace.
    fn fall_back(&self, key: &K, error: buzz_db::DbError) -> Result<V, buzz_db::DbError> {
        let Some(fallback) = &self.fallback else {
            return Err(error);
        };
        if !matches!(error, buzz_db::DbError::Sqlx(_)) {
            return Err(error);
        }
        let Some(entry) = self.entries.get(key).filter(|entry| {
            entry.read_at.elapsed() < self.max_age + fallback.grace
                && (fallback.accepts)(&entry.value)
        }) else {
            return Err(error);
        };
        metrics::counter!(fallback.served).increment(1);
        tracing::warn!(%error, "read failed; serving the last accepted cached answer");
        Ok(entry.value)
    }

    /// Publishes `read` as `key`'s entry unless an invalidation since
    /// `observed` fenced it.
    fn publish(&self, key: K, observed: u64, read: &buzz_db::ReadAt<V>) {
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
    }

    /// `key`'s value from the cache while it is fresh, else from `read`.
    /// Errors are not cached; a failed read may fall back (see [`Fallback`]).
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
        self.get_fresh(key, read).await
    }

    /// `key`'s value from `read`, never from a fresh entry, published as the
    /// entry unless an invalidation fenced it. A failed read may fall back
    /// (see [`Fallback`]).
    pub(crate) async fn get_fresh<Read, ReadFuture>(
        &self,
        key: K,
        read: Read,
    ) -> Result<V, buzz_db::DbError>
    where
        Read: FnOnce() -> ReadFuture,
        ReadFuture: Future<Output = Result<buzz_db::ReadAt<V>, buzz_db::DbError>>,
    {
        // Observed before the read checks out its connection, so a read that
        // could predate an invalidation always sees a newer epoch.
        let observed = *self.lock();
        match read().await {
            Ok(read) => {
                self.publish(key, observed, &read);
                Ok(read.value)
            }
            Err(error) => self.fall_back(&key, error),
        }
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

    const GRACE: Duration = Duration::from_secs(60);

    /// Accepts `true`, standing for an unrestricted row.
    fn cache_with_fallback() -> FencedCache<u8, bool> {
        FencedCache::with_fallback(
            16,
            MAX_AGE,
            Duration::from_secs(5),
            (
                "test_fenced_cache_hits_total",
                "test_fenced_cache_misses_total",
            ),
            Fallback {
                grace: GRACE,
                accepts: |value| *value,
                served: "test_fenced_cache_fallback_served_total",
            },
        )
    }

    fn pool_timeout() -> Result<buzz_db::ReadAt<bool>, buzz_db::DbError> {
        Err(buzz_db::DbError::Sqlx(sqlx::Error::PoolTimedOut))
    }

    /// A read that fails with a database error, through `get` (past an
    /// expired entry) and `get_fresh` alike, is answered from an accepted
    /// entry younger than the max age plus grace, and only then.
    /// Mutations: serve a rejected value, drop the age check, fall back on a
    /// non-database error, or skip the fallback in either path → RED.
    #[tokio::test]
    async fn a_failed_read_falls_back_only_to_a_recent_accepted_entry() {
        let just_inside = MAX_AGE + GRACE - Duration::from_secs(1);
        let cases = [
            (Some((true, just_inside)), true, Some(true)),
            (Some((true, MAX_AGE + GRACE)), true, None),
            (Some((false, MAX_AGE)), true, None),
            (None, true, None),
            (Some((true, MAX_AGE)), false, None),
        ];
        for (seeded, database_error, served) in cases {
            for fresh in [false, true] {
                let cache = cache_with_fallback();
                if let Some((value, age)) = seeded {
                    cache.seed_read_at(1, value, Instant::now() - age);
                }
                let read = || async move {
                    if database_error {
                        pool_timeout()
                    } else {
                        Err(buzz_db::DbError::AuthEventRejected)
                    }
                };
                let answer = if fresh {
                    cache.get_fresh(1, read).await
                } else {
                    cache.get(1, read).await
                };
                assert_eq!(
                    answer.ok(),
                    served,
                    "seeded {seeded:?}, database error {database_error}, fresh {fresh}",
                );
            }
        }
    }

    /// A successful read wins over an accepted entry, an invalidation drops
    /// the entry a fallback would serve, and a cache without a fallback
    /// still fails closed.
    /// Mutation: fall back before reading, keep entries past invalidation,
    /// or give `new` a fallback → RED.
    #[tokio::test]
    async fn a_successful_read_wins_and_invalidation_drops_the_fallback() {
        let fallback = cache_with_fallback();
        fallback.seed_read_at(1, true, Instant::now() - MAX_AGE);
        let read = fallback.get_fresh(1, || async { read_at(false) }).await;
        assert_eq!(read.ok(), Some(false), "the read's answer is served");
        assert!(
            fallback
                .get_fresh(1, || async { pool_timeout() })
                .await
                .is_err(),
            "the read replaced the accepted entry",
        );

        fallback.seed(1, true);
        fallback.invalidate(&1);
        assert!(fallback
            .get_fresh(1, || async { pool_timeout() })
            .await
            .is_err());

        let strict = cache();
        strict.seed_read_at(1, true, Instant::now() - MAX_AGE);
        assert!(strict.get(1, || async { pool_timeout() }).await.is_err());
    }

    /// `get_fresh` reads past a fresh entry and publishes its read, unless an
    /// invalidation during the read fenced it.
    /// Mutation: serve the fresh entry, skip the publish, or skip its epoch
    /// check → RED.
    #[tokio::test]
    async fn get_fresh_reads_past_the_entry_and_publishes_unless_fenced() {
        let cache = cache_with_fallback();
        cache.seed(1, true);
        let read = cache.get_fresh(1, || async { read_at(false) }).await;
        assert_eq!(read.ok(), Some(false));
        assert_eq!(cache.entry(&1).map(|(value, _)| value), Some(false));

        let read = cache
            .get_fresh(2, || async {
                cache.invalidate(&9);
                read_at(true)
            })
            .await;
        assert_eq!(read.ok(), Some(true), "the caller still gets its read");
        assert!(cache.entry(&2).is_none(), "the fenced read is not cached");
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
