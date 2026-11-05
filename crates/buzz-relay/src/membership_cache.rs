//! One active-membership snapshot per community and user, shared by all of
//! their channel checks. Discovery lists have different channel-type rules.

use std::future::Future;
use std::sync::{Arc, Mutex as PublicationMutex};
use std::time::Duration;

use buzz_core::CommunityId;
use buzz_db::{DbError, ReadAt};
use tokio::sync::Mutex;
use uuid::Uuid;

type Key = (CommunityId, Vec<u8>);
type Snapshot = ReadAt<Memberships>;

const MAX_AGE: Duration = Duration::from_secs(10);
const CACHE_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_MEMBERSHIPS: usize = 16_384;

/// Oversized users retain authoritative point checks, never a truncated set.
enum Memberships {
    Ids(Box<[Uuid]>),
    PointChecks,
}

/// The snapshot is immutable. Replacing this value through Moka updates its
/// weight; the shared gate coordinates loads without holding their payload.
#[derive(Clone)]
struct Entry {
    gate: Arc<Mutex<()>>,
    snapshot: Option<Arc<Snapshot>>,
}

impl Entry {
    fn loading() -> Self {
        Self {
            gate: Arc::new(Mutex::new(())),
            snapshot: None,
        }
    }

    fn weight(key: &Key, entry: &Self) -> u32 {
        let payload = match entry.snapshot.as_deref().map(|s| &s.value) {
            Some(Memberships::Ids(ids)) => std::mem::size_of_val(ids.as_ref()),
            _ => 0,
        };
        // Account for our key, value, gate and snapshot allocations, plus Arc
        // counters and an estimated Moka bookkeeping allowance. This is an
        // eviction weight, not an allocator/RSS measurement. Empty entries and
        // fallback markers have a nonzero weight too.
        let overhead = std::mem::size_of::<Key>()
            + key.1.capacity()
            + std::mem::size_of::<Self>()
            + std::mem::size_of::<Mutex<()>>()
            + std::mem::size_of::<Snapshot>()
            + 4 * std::mem::size_of::<usize>()
            + 128;
        u32::try_from(payload.saturating_add(overhead)).unwrap_or(u32::MAX)
    }

    fn fresh(&self) -> Option<&Memberships> {
        self.snapshot
            .as_deref()
            .filter(|s| s.read_at.elapsed() < MAX_AGE)
            .map(|s| &s.value)
    }
}

/// Moka owns weighted admission, eviction and expiration. Publication and
/// invalidation share a short synchronous lock: an old loader may answer its
/// original request, but can only publish while its gate is still current.
/// No database work or async waiting happens under the publication lock.
pub(crate) struct MembershipCache {
    entries: moka::sync::Cache<Key, Entry>,
    publication: PublicationMutex<()>,
    wait: Duration,
}

impl MembershipCache {
    pub(crate) fn new(wait: Duration) -> Self {
        Self::with_capacity(wait, CACHE_BYTES)
    }

    fn with_capacity(wait: Duration, bytes: u64) -> Self {
        Self {
            entries: moka::sync::Cache::builder()
                .max_capacity(bytes)
                .weigher(Entry::weight)
                .time_to_live(MAX_AGE)
                .support_invalidation_closures()
                .build(),
            publication: PublicationMutex::new(()),
            wait,
        }
    }

    /// Shares a bounded full-set read; the database pool controls query concurrency.
    /// Gate timeouts use the authoritative point query, never a second full load.
    /// Errors are not cached; cancellation releases the gate.
    pub(crate) async fn contains(
        &self,
        key: Key,
        channel: Uuid,
        read: impl Future<Output = Result<ReadAt<Vec<Uuid>>, DbError>>,
        point: impl Future<Output = Result<bool, DbError>>,
    ) -> Result<bool, DbError> {
        let entry = {
            // Fast hits need only Moka's own synchronization.
            if let Some(entry) = self.entries.get(&key) {
                entry
            } else {
                let _publication = self.publication.lock().unwrap_or_else(|e| e.into_inner());
                self.entries.get_with(key.clone(), Entry::loading)
            }
        };
        if let Some(answer) = Self::answer(&entry, channel) {
            return match answer {
                Some(member) => Ok(member),
                None => Self::point_check("oversize", point).await,
            };
        }
        let gate = entry.gate.clone();
        drop(entry);
        let Ok(guard) = tokio::time::timeout(self.wait, gate.lock()).await else {
            return Self::point_check("wait_timeout", point).await;
        };
        // A preceding loader may have replaced the loading entry while this
        // request waited. Re-read the completed value instead of mutating it.
        if let Some(entry) = self.entries.get(&key) {
            if let Some(answer) = Self::answer(&entry, channel) {
                drop(guard);
                return match answer {
                    Some(member) => Ok(member),
                    None => Self::point_check("oversize", point).await,
                };
            }
        }
        metrics::counter!("buzz_membership_cache_misses_total").increment(1);
        let read = read.await?;
        let memberships = if read.value.len() > MAX_MEMBERSHIPS {
            // Do not retain the query buffer while a point check waits.
            drop(read.value);
            Memberships::PointChecks
        } else {
            let mut ids = read.value;
            ids.sort_unstable();
            ids.dedup();
            Memberships::Ids(ids.into_boxed_slice())
        };
        let answer = match &memberships {
            Memberships::Ids(ids) => Some(ids.binary_search(&channel).is_ok()),
            Memberships::PointChecks => None,
        };
        let completed = Entry {
            gate: gate.clone(),
            snapshot: Some(Arc::new(ReadAt {
                value: memberships,
                read_at: read.read_at,
            })),
        };
        {
            let _publication = self.publication.lock().unwrap_or_else(|e| e.into_inner());
            if self
                .entries
                .get(&key)
                .is_some_and(|current| Arc::ptr_eq(&current.gate, &gate))
            {
                self.entries.insert(key, completed);
            }
        }
        drop(guard);
        match answer {
            Some(member) => Ok(member),
            None => Self::point_check("oversize", point).await,
        }
    }

    // Outer None is a cache miss; inner None is the cached point-check strategy.
    fn answer(entry: &Entry, channel: Uuid) -> Option<Option<bool>> {
        match entry.fresh()? {
            Memberships::Ids(ids) => {
                metrics::counter!("buzz_membership_cache_hits_total").increment(1);
                Some(Some(ids.binary_search(&channel).is_ok()))
            }
            Memberships::PointChecks => Some(None),
        }
    }

    async fn point_check(
        reason: &'static str,
        point: impl Future<Output = Result<bool, DbError>>,
    ) -> Result<bool, DbError> {
        metrics::counter!("buzz_membership_cache_fallbacks_total", "reason" => reason).increment(1);
        point.await
    }

    pub(crate) fn invalidate(&self, community: CommunityId, pubkey: &[u8]) {
        let _publication = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        self.entries.invalidate(&(community, pubkey.to_vec()));
    }

    pub(crate) fn invalidate_community(&self, community: CommunityId) {
        let _publication = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = self
            .entries
            .invalidate_entries_if(move |(entry_community, _), _| *entry_community == community)
        {
            tracing::error!(
                ?error,
                "membership invalidation unavailable; dropping all entries"
            );
            self.entries.invalidate_all();
        }
    }

    /// Seed one membership in a test's synthetic snapshot.
    #[cfg(test)]
    pub(crate) fn seed_member(&self, key: (CommunityId, Uuid, Vec<u8>), member: bool) {
        let (community, channel, pubkey) = key;
        let key = (community, pubkey);
        let mut ids = self
            .entries
            .get(&key)
            .and_then(|e| match e.fresh() {
                Some(Memberships::Ids(ids)) => Some(ids.to_vec()),
                _ => None,
            })
            .unwrap_or_default();
        ids.retain(|id| *id != channel);
        if member {
            ids.push(channel);
        }
        ids.sort_unstable();
        self.entries.insert(
            key,
            Entry {
                gate: Arc::new(Mutex::new(())),
                snapshot: Some(Arc::new(ReadAt {
                    value: Memberships::Ids(ids.into_boxed_slice()),
                    read_at: std::time::Instant::now(),
                })),
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn cached_member(&self, key: &(CommunityId, Uuid, Vec<u8>)) -> Option<bool> {
        Self::answer(&self.entries.get(&(key.0, key.2.clone()))?, key.1).flatten()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod postgres_tests;
