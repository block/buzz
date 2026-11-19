//! Fences a cache's refills against its invalidation.
//!
//! A refill reads, then publishes. If an invalidation for its key runs while
//! it reads, its result may predate the change that the invalidation
//! announced: publishing it would undo the invalidation for the cache's whole
//! lifetime. A fence gives each refill a token, drops the token on
//! invalidation, and lets a refill publish only while its token is current.

use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The token shared by the refills in flight per key, and a lock that orders
/// publication against invalidation. The lock is held only for cache operations, never
/// across a read.
pub(crate) struct LoadFence<K> {
    loads: moka::sync::Cache<K, Arc<()>>,
    lock: Mutex<()>,
}

/// A refill's claim to publish; see [`LoadFence::begin`].
pub(crate) struct Load(Arc<()>);

impl<K> LoadFence<K>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
{
    /// Tokens for up to `max_capacity` keys. A token evicted before its
    /// refill ends only stops that refill from publishing, so the bounds fail
    /// safe.
    pub(crate) fn new(max_capacity: u64) -> Self {
        Self {
            loads: moka::sync::Cache::builder()
                .max_capacity(max_capacity)
                .time_to_live(Duration::from_secs(60))
                .support_invalidation_closures()
                .build(),
            lock: Mutex::new(()),
        }
    }

    /// Starts a refill of `key`. Refills that overlap share one token, so the
    /// first to finish publishes; only an invalidation stops them.
    pub(crate) fn begin(&self, key: &K) -> Load {
        let _lock = self.lock();
        Load(self.loads.get_with(key.clone(), || Arc::new(())))
    }

    /// Runs `publish` if no invalidation of `key` and no other refill's
    /// publication happened since `load` began. Publishing drops the token,
    /// so the refills that shared it leave the published entry alone.
    pub(crate) fn publish(&self, key: &K, load: Load, publish: impl FnOnce()) {
        let _lock = self.lock();
        if self
            .loads
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(&current, &load.0))
        {
            self.loads.invalidate(key);
            publish();
        }
    }

    /// Stops refills of `key` in flight from publishing, then runs `drop`,
    /// which removes the cached entry.
    pub(crate) fn invalidate(&self, key: &K, drop: impl FnOnce()) {
        let _lock = self.lock();
        self.loads.invalidate(key);
        drop();
    }

    /// [`Self::invalidate`] for every key matching `matches`.
    pub(crate) fn invalidate_matching(
        &self,
        matches: impl Fn(&K) -> bool + Send + Sync + 'static,
        drop: impl FnOnce(),
    ) {
        let _lock = self.lock();
        if let Err(error) = self.loads.invalidate_entries_if(move |key, _| matches(key)) {
            // `new` enables invalidation closures. If that ever regresses,
            // stop every refill rather than let a stale one publish.
            tracing::error!(
                ?error,
                "scoped load-fence invalidation unavailable; stopping all refills"
            );
            self.loads.invalidate_all();
        }
        drop();
    }

    /// Whether a refill of `key` is in flight.
    #[cfg(test)]
    pub(crate) fn is_loading(&self, key: &K) -> bool {
        self.loads.contains_key(key)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|e| e.into_inner())
    }
}
