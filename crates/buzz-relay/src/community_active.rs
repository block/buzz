//! Per-community lifecycle answers ("is this community active?") shared by
//! socket admission and the ephemeral write fence.
//!
//! Every new socket asks this before it can authenticate, so in a reconnect
//! storm one community gets thousands of identical reads at once. An answer
//! is served for [`COMMUNITY_ACTIVE_MAX_AGE`] after the read that produced it
//! checked out its connection, and concurrent misses share one read.
//!
//! A cached "active" must not undo the archive sweep. Socket admission
//! registers the socket before it asks, so a sweep closes every socket that
//! registered before it, and a socket that registers after it must see the
//! community inactive. [`CommunityActiveCache::invalidate`] runs before the
//! sweep, drops the entry and bumps an epoch; a fill whose read began before
//! that bump is not published, so a later socket reads afresh instead of
//! taking a pre-archive answer from the cache or from a shared in-flight read.

use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use buzz_core::CommunityId;

use crate::shared_reads::SharedReads;

/// How long a pod serves a cached community lifecycle answer after the
/// database read that produced it began, at its connection checkout (see
/// [`buzz_db::ReadAt`]). Archive and deletion on this pod drop the entry
/// before closing sockets; a pod that misses that notification admits sockets
/// for at most this long, and lifecycle revalidation closes them.
pub(crate) const COMMUNITY_ACTIVE_MAX_AGE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
struct Entry {
    active: bool,
    read_at: Instant,
}

/// Cached `is_community_active` answers, fenced against invalidation.
pub(crate) struct CommunityActiveCache {
    entries: moka::sync::Cache<CommunityId, Entry>,
    reads: SharedReads<CommunityId>,
    // Serializes invalidation with fill publication. One epoch for every
    // community keeps no per-community tombstones; an unrelated invalidation
    // only skips publishing an in-flight fill.
    epoch: Mutex<u64>,
}

impl CommunityActiveCache {
    /// A cache whose concurrent misses wait at most `shared_read_wait` for a
    /// shared read (see [`SharedReads::new`]).
    pub(crate) fn new(shared_read_wait: Duration) -> Self {
        Self {
            // Readers use an entry only for `COMMUNITY_ACTIVE_MAX_AGE`; the
            // TTL just reclaims memory.
            entries: moka::sync::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(10))
                .build(),
            reads: SharedReads::new(10_000, shared_read_wait),
            epoch: Mutex::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, u64> {
        self.epoch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn fresh(&self, community: CommunityId) -> Option<bool> {
        self.entries
            .get(&community)
            .filter(|entry| entry.read_at.elapsed() < COMMUNITY_ACTIVE_MAX_AGE)
            .map(|entry| entry.active)
    }

    /// `community`'s answer from the cache while it is fresh, else from
    /// `read`. Errors are not cached.
    ///
    /// The first concurrent miss reads; the rest wait for it and use its
    /// entry. A waiter whose leader failed, was cancelled, was fenced by an
    /// invalidation or is still reading after the gate's wait reads on its
    /// own.
    pub(crate) async fn get<Read, ReadFuture>(
        &self,
        community: CommunityId,
        read: Read,
    ) -> Result<bool, buzz_db::DbError>
    where
        Read: FnOnce() -> ReadFuture,
        ReadFuture: Future<Output = Result<buzz_db::ReadAt<bool>, buzz_db::DbError>>,
    {
        if let Some(active) = self.fresh(community) {
            metrics::counter!("buzz_community_active_cache_hits_total").increment(1);
            return Ok(active);
        }
        let _turn = self.reads.turn(community).await;
        if let Some(active) = self.fresh(community) {
            metrics::counter!("buzz_community_active_cache_hits_total").increment(1);
            return Ok(active);
        }
        metrics::counter!("buzz_community_active_cache_misses_total").increment(1);
        // Observed before the read checks out its connection, so a read that
        // could predate an invalidation always sees a newer epoch.
        let observed = *self.lock();
        let read = read().await?;
        let epoch = self.lock();
        if *epoch == observed {
            self.entries.insert(
                community,
                Entry {
                    active: read.value,
                    read_at: read.read_at,
                },
            );
        }
        Ok(read.value)
    }

    /// Drops `community`'s entry and fences every fill begun before this.
    /// Runs before an archive or deletion closes the community's sockets.
    pub(crate) fn invalidate(&self, community: CommunityId) {
        let mut epoch = self.lock();
        *epoch = epoch.wrapping_add(1);
        self.entries.invalidate(&community);
    }

    /// Seeds a fresh entry without touching the database.
    #[cfg(test)]
    pub(crate) fn seed(&self, community: CommunityId, active: bool) {
        self.entries.insert(
            community,
            Entry {
                active,
                read_at: Instant::now(),
            },
        );
    }

    /// The cached answer and its read time, fresh or not.
    #[cfg(test)]
    pub(crate) fn entry(&self, community: CommunityId) -> Option<(bool, Instant)> {
        self.entries
            .get(&community)
            .map(|entry| (entry.active, entry.read_at))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    fn read_at(value: bool) -> Result<buzz_db::ReadAt<bool>, buzz_db::DbError> {
        Ok(buzz_db::ReadAt {
            value,
            read_at: Instant::now(),
        })
    }

    #[tokio::test]
    async fn concurrent_misses_share_one_read_and_expire_after_max_age() {
        let cache = Arc::new(CommunityActiveCache::new(Duration::from_secs(5)));
        let community = CommunityId::from_uuid(uuid::Uuid::new_v4());
        let reads = Arc::new(AtomicUsize::new(0));
        let (release, released) = tokio::sync::watch::channel(false);

        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let reads = Arc::clone(&reads);
                let mut released = released.clone();
                tokio::spawn(async move {
                    cache
                        .get(community, || async move {
                            reads.fetch_add(1, Ordering::SeqCst);
                            let _ = released.wait_for(|go| *go).await;
                            read_at(true)
                        })
                        .await
                })
            })
            .collect();
        // Let every task reach the gate before the leader's read finishes.
        tokio::time::sleep(Duration::from_millis(50)).await;
        release.send_replace(true);
        for task in tasks {
            assert!(task.await.expect("task").expect("read"));
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1, "misses must share a read");

        // Past the max age the entry is stale and the next caller reads.
        let (active, _) = cache.entry(community).expect("entry");
        assert!(active);
        cache.entries.insert(
            community,
            Entry {
                active: true,
                read_at: Instant::now() - COMMUNITY_ACTIVE_MAX_AGE,
            },
        );
        let refreshed = cache
            .get(community, || async {
                reads.fetch_add(1, Ordering::SeqCst);
                read_at(false)
            })
            .await
            .expect("read");
        assert!(!refreshed, "an expired entry must not be served");
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn invalidation_fences_an_in_flight_fill_and_its_waiters() {
        let cache = Arc::new(CommunityActiveCache::new(Duration::from_secs(5)));
        let community = CommunityId::from_uuid(uuid::Uuid::new_v4());
        let (release, mut released) = tokio::sync::watch::channel(false);

        // A leader reads "active" from a snapshot that predates the archive.
        let leader = {
            let cache = Arc::clone(&cache);
            tokio::spawn(async move {
                cache
                    .get(community, || async move {
                        let _ = released.wait_for(|go| *go).await;
                        read_at(true)
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        // A socket that registers after the sweep joins the in-flight read.
        let fresh_reads = Arc::new(AtomicUsize::new(0));
        let waiter = {
            let cache = Arc::clone(&cache);
            let fresh_reads = Arc::clone(&fresh_reads);
            tokio::spawn(async move {
                cache
                    .get(community, || async move {
                        fresh_reads.fetch_add(1, Ordering::SeqCst);
                        read_at(false)
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        cache.invalidate(community);
        release.send_replace(true);

        assert!(leader.await.expect("leader").expect("read"));
        assert!(
            !waiter.await.expect("waiter").expect("read"),
            "a post-invalidation caller must not take a pre-invalidation answer",
        );
        assert_eq!(fresh_reads.load(Ordering::SeqCst), 1);
        assert_eq!(
            cache.entry(community).map(|(active, _)| active),
            Some(false),
            "only the post-invalidation read may fill the cache",
        );
    }

    #[tokio::test]
    async fn errors_are_not_cached() {
        let cache = CommunityActiveCache::new(Duration::from_secs(5));
        let community = CommunityId::from_uuid(uuid::Uuid::new_v4());
        let failed = cache
            .get(community, || async {
                Err(buzz_db::DbError::AuthEventRejected)
            })
            .await;
        assert!(failed.is_err());
        assert!(cache.entry(community).is_none());
    }
}
