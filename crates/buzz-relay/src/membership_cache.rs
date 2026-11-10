//! One active-membership snapshot per community and user, shared by all of
//! their channel checks. Discovery lists have different channel-type rules.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex as PublicationMutex};
use std::time::{Duration, Instant};

use buzz_core::CommunityId;
use buzz_db::{DbError, ReadAt};
use tokio::sync::{Mutex, OwnedMutexGuard};
use uuid::Uuid;

type Key = (CommunityId, Vec<u8>);
/// A loading entry's gate, and the lock this load holds on it.
type HeldGate = (Arc<Mutex<()>>, OwnedMutexGuard<()>);
type Snapshot = ReadAt<Memberships>;

const MAX_AGE: Duration = Duration::from_secs(10);
const CACHE_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_MEMBERSHIPS: usize = 16_384;

/// Users per bulk read in [`MembershipCache::members`]. Each may return up
/// to [`MAX_MEMBERSHIPS`] + 1 rows.
const BULK_USERS: usize = 64;

/// Oversized users retain authoritative point checks, never a truncated set.
enum Memberships {
    Ids(Box<[Uuid]>),
    PointChecks,
}

impl Memberships {
    /// The memberships a bounded read returned. A read past the cap may be
    /// truncated, so it means point checks, never a partial set.
    fn from_read(mut ids: Vec<Uuid>) -> Self {
        if ids.len() > MAX_MEMBERSHIPS {
            return Self::PointChecks;
        }
        ids.sort_unstable();
        ids.dedup();
        Self::Ids(ids.into_boxed_slice())
    }

    /// Whether `channel` is one of them, or `None` when only a point check
    /// can tell.
    fn contains(&self, channel: Uuid) -> Option<bool> {
        match self {
            Self::Ids(ids) => Some(ids.binary_search(&channel).is_ok()),
            Self::PointChecks => None,
        }
    }
}

/// [`MembershipCache::members`]'s answer. A pubkey in neither set is not a
/// member.
#[derive(Debug, Default)]
pub(crate) struct Members {
    pub(crate) members: HashSet<Vec<u8>>,
    /// Pubkeys whose membership could not be read.
    pub(crate) unverified: HashSet<Vec<u8>>,
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
        let memberships = Memberships::from_read(read.value);
        let answer = memberships.contains(channel);
        self.publish(key, &gate, memberships, read.read_at);
        drop(guard);
        match answer {
            Some(member) => Ok(member),
            None => Self::point_check("oversize", point).await,
        }
    }

    /// Which of `pubkeys` are members of `channel`, for one fan-out's
    /// recipients. Fresh sets answer directly. The other users' sets load in
    /// one bulk read per [`BULK_USERS`] users and are published as
    /// [`Self::contains`] publishes its loads. A user whose set another
    /// request is loading is read again here rather than waited for, so a
    /// batch never queues behind single loads. Oversized users share one point
    /// read. Users whose read failed are returned as unverified.
    pub(crate) async fn members<R, P>(
        &self,
        community: CommunityId,
        channel: Uuid,
        pubkeys: impl IntoIterator<Item = Vec<u8>>,
        read: impl Fn(Vec<Vec<u8>>) -> R,
        point: impl FnOnce(Vec<Vec<u8>>) -> P,
    ) -> Members
    where
        R: Future<Output = Result<ReadAt<buzz_db::channel_members::PubkeyChannels>, DbError>>,
        P: Future<Output = Result<Vec<Vec<u8>>, DbError>>,
    {
        let mut out = Members::default();
        let mut points = Vec::new();
        let mut misses = Vec::new();
        for pubkey in pubkeys.into_iter().collect::<HashSet<_>>() {
            let key = (community, pubkey);
            match self
                .entries
                .get(&key)
                .and_then(|entry| Self::answer(&entry, channel))
            {
                Some(Some(true)) => {
                    out.members.insert(key.1);
                }
                Some(Some(false)) => {}
                Some(None) => points.push(key.1),
                None => misses.push(key.1),
            }
        }
        for chunk in misses.chunks(BULK_USERS) {
            // Hold the gates no other load holds, so these reads can publish.
            let mut gates: HashMap<&[u8], HeldGate> = chunk
                .iter()
                .filter_map(|pubkey| {
                    let gate = {
                        let _publication =
                            self.publication.lock().unwrap_or_else(|e| e.into_inner());
                        self.entries
                            .get_with((community, pubkey.clone()), Entry::loading)
                            .gate
                    };
                    let guard = Arc::clone(&gate).try_lock_owned().ok()?;
                    Some((pubkey.as_slice(), (gate, guard)))
                })
                .collect();
            metrics::counter!("buzz_membership_cache_misses_total").increment(chunk.len() as u64);
            let read = match read(chunk.to_vec()).await {
                Ok(read) => read,
                Err(error) => {
                    tracing::warn!(%channel, "bulk membership read failed: {error}");
                    out.unverified.extend(chunk.iter().cloned());
                    continue;
                }
            };
            let mut sets: HashMap<Vec<u8>, Vec<Uuid>> = HashMap::new();
            for (pubkey, id) in read.value {
                sets.entry(pubkey).or_default().push(id);
            }
            for pubkey in chunk {
                let memberships = Memberships::from_read(sets.remove(pubkey).unwrap_or_default());
                match memberships.contains(channel) {
                    Some(true) => {
                        out.members.insert(pubkey.clone());
                    }
                    Some(false) => {}
                    None => points.push(pubkey.clone()),
                }
                if let Some((gate, guard)) = gates.remove(pubkey.as_slice()) {
                    self.publish(
                        (community, pubkey.clone()),
                        &gate,
                        memberships,
                        read.read_at,
                    );
                    drop(guard);
                }
            }
        }
        if !points.is_empty() {
            metrics::counter!("buzz_membership_cache_fallbacks_total", "reason" => "oversize")
                .increment(points.len() as u64);
            match point(points.clone()).await {
                Ok(members) => out.members.extend(members),
                Err(error) => {
                    tracing::warn!(%channel, "membership point read failed: {error}");
                    out.unverified.extend(points);
                }
            }
        }
        out
    }

    /// Publish a completed load, unless an invalidation replaced its entry
    /// while it read: an old loader may answer its own request, but only
    /// publishes while its gate is still current.
    fn publish(&self, key: Key, gate: &Arc<Mutex<()>>, value: Memberships, read_at: Instant) {
        let completed = Entry {
            gate: Arc::clone(gate),
            snapshot: Some(Arc::new(ReadAt { value, read_at })),
        };
        let _publication = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        if self
            .entries
            .get(&key)
            .is_some_and(|current| Arc::ptr_eq(&current.gate, gate))
        {
            self.entries.insert(key, completed);
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
