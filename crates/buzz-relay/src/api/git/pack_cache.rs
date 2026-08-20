//! Bounded local cache for immutable Git pack/index pairs.
//!
//! Object storage remains the durable source of truth. Cache entries are
//! content-addressed by the verified pack digest and are published by an
//! atomic directory rename only after both the pack and index are ready.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use tempfile::Builder;

use super::hydrate::{get_verified_limited, install_or_generate_idx, HydrateError};
use super::store::GitStore;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const STALE_SESSION_AGE: Duration = Duration::from_secs(10 * 60);

#[derive(Debug)]
struct CachedPack {
    dir: PathBuf,
    pack_path: PathBuf,
    idx_path: PathBuf,
    pack_bytes: u64,
    total_bytes: u64,
    /// Keeps an over-capacity staging directory alive until it is linked into
    /// the request workspace. Normal cached entries are atomically renamed and
    /// do not need a temporary owner.
    _temporary: Option<tempfile::TempDir>,
}

impl CachedPack {
    fn from_dir(dir: PathBuf, digest: &str) -> Option<Self> {
        let pack_path = dir.join(format!("pack-{digest}.pack"));
        let idx_path = dir.join(format!("pack-{digest}.idx"));
        let pack_bytes = regular_file_len(&pack_path)?;
        let idx_bytes = regular_file_len(&idx_path)?;
        Some(Self {
            dir,
            pack_path,
            idx_path,
            pack_bytes,
            total_bytes: pack_bytes.saturating_add(idx_bytes),
            _temporary: None,
        })
    }
}

struct CacheRecord {
    entry: Arc<CachedPack>,
    last_used: u64,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<String, CacheRecord>,
    total_bytes: u64,
    tick: u64,
}

/// Process-local, byte-bounded cache of immutable pack/index pairs.
pub struct GitPackCache {
    _session_dir: tempfile::TempDir,
    heartbeat_task: Option<tokio::task::JoinHandle<()>>,
    root: PathBuf,
    max_bytes: u64,
    population_semaphore: tokio::sync::Semaphore,
    state: Mutex<CacheState>,
    flights: DashMap<String, Arc<PopulationFlight>>,
}

struct PopulationFlight {
    lock: tokio::sync::Mutex<()>,
    participants: AtomicUsize,
}

struct FlightParticipant<'a> {
    cache: &'a GitPackCache,
    digest: &'a str,
    flight: Arc<PopulationFlight>,
}

struct PopulationPermit<'a> {
    _permit: tokio::sync::SemaphorePermit<'a>,
}

impl Drop for PopulationPermit<'_> {
    fn drop(&mut self) {
        metrics::gauge!("buzz_git_pack_cache_populations_active").decrement(1.0);
    }
}

impl Drop for FlightParticipant<'_> {
    fn drop(&mut self) {
        if self.flight.participants.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.cache.remove_flight(self.digest, &self.flight);
        }
    }
}

impl GitPackCache {
    /// Create an isolated process-lifetime cache beneath `cache_parent`.
    pub fn new(
        cache_parent: &Path,
        max_bytes: u64,
        max_concurrent_populations: usize,
    ) -> Result<Self, String> {
        if max_concurrent_populations == 0 {
            return Err("git pack cache population concurrency must be positive".to_string());
        }
        std::fs::create_dir_all(cache_parent)
            .map_err(|error| format!("create git pack cache {cache_parent:?}: {error}"))?;
        if std::fs::symlink_metadata(cache_parent)
            .map_err(|error| format!("stat git pack cache {cache_parent:?}: {error}"))?
            .file_type()
            .is_symlink()
        {
            return Err(format!(
                "git pack cache path must not be a symlink: {cache_parent:?}"
            ));
        }
        cleanup_stale_sessions(cache_parent);
        let session_dir = Builder::new()
            .prefix("session-")
            .tempdir_in(cache_parent)
            .map_err(|error| format!("create git pack cache session: {error}"))?;
        let root = session_dir.path().to_path_buf();
        let heartbeat_path = root.join(".heartbeat");
        std::fs::write(&heartbeat_path, b"")
            .map_err(|error| format!("create git pack cache heartbeat: {error}"))?;
        let heartbeat_task = tokio::runtime::Handle::try_current().ok().map(|runtime| {
            runtime.spawn(async move {
                let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
                interval.tick().await;
                loop {
                    interval.tick().await;
                    if tokio::fs::write(&heartbeat_path, b"").await.is_err() {
                        break;
                    }
                }
            })
        });
        let cache = Self {
            _session_dir: session_dir,
            heartbeat_task,
            root,
            max_bytes,
            population_semaphore: tokio::sync::Semaphore::new(max_concurrent_populations),
            state: Mutex::new(CacheState::default()),
            flights: DashMap::new(),
        };
        emit_size_metrics(&CacheState::default());
        Ok(cache)
    }

    /// Materialize one verified pack/index pair into a request workspace.
    ///
    /// Concurrent misses for the same digest share one population flight.
    pub async fn materialize_pack(
        &self,
        store: &GitStore,
        object_key: &str,
        digest: &str,
        destination: &Path,
        max_pack_bytes: u64,
    ) -> Result<u64, HydrateError> {
        validate_digest(digest)?;
        if let Some(entry) = self.lookup(digest) {
            metrics::counter!("buzz_git_pack_cache_lookups_total", "result" => "hit").increment(1);
            let pack_bytes = entry.pack_bytes;
            if let Ok(()) = install_entry(&entry, digest, destination).await {
                drop(entry);
                self.prune();
                return Ok(pack_bytes);
            }
            self.invalidate(digest);
        }

        let (flight, joined) = match self.flights.entry(digest.to_string()) {
            Entry::Occupied(entry) => {
                entry.get().participants.fetch_add(1, Ordering::Relaxed);
                (Arc::clone(entry.get()), true)
            }
            Entry::Vacant(entry) => {
                let flight = Arc::new(PopulationFlight {
                    lock: tokio::sync::Mutex::new(()),
                    participants: AtomicUsize::new(1),
                });
                entry.insert(Arc::clone(&flight));
                (flight, false)
            }
        };
        metrics::counter!(
            "buzz_git_pack_cache_lookups_total",
            "result" => if joined { "coalesced" } else { "miss" }
        )
        .increment(1);

        let flight_participant = FlightParticipant {
            cache: self,
            digest,
            flight: Arc::clone(&flight),
        };
        let guard = flight.lock.lock().await;
        let result = if let Some(entry) = self.lookup(digest) {
            install_entry(&entry, digest, destination)
                .await
                .map(|()| entry.pack_bytes)
        } else {
            let started_at = Instant::now();
            let populated = self
                .populate(store, object_key, digest, max_pack_bytes)
                .await;
            let outcome = match &populated {
                Ok(entry) if entry._temporary.is_some() => "bypass",
                Ok(_) => "success",
                Err(_) => "error",
            };
            metrics::histogram!(
                "buzz_git_pack_cache_populate_seconds",
                "outcome" => outcome
            )
            .record(started_at.elapsed().as_secs_f64());
            match populated {
                Ok(entry) => install_entry(&entry, digest, destination)
                    .await
                    .map(|()| entry.pack_bytes),
                Err(error) => Err(error),
            }
        };
        drop(guard);
        drop(flight_participant);
        self.prune();
        result
    }

    fn lookup(&self, digest: &str) -> Option<Arc<CachedPack>> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let next_tick = state.tick.saturating_add(1);
        state.tick = next_tick;
        let record = state.entries.get_mut(digest)?;
        if !record.entry.pack_path.is_file() || !record.entry.idx_path.is_file() {
            return None;
        }
        record.last_used = next_tick;
        Some(Arc::clone(&record.entry))
    }

    async fn populate(
        &self,
        store: &GitStore,
        object_key: &str,
        digest: &str,
        max_pack_bytes: u64,
    ) -> Result<Arc<CachedPack>, HydrateError> {
        let wait_started_at = Instant::now();
        let permit = self
            .population_semaphore
            .acquire()
            .await
            .map_err(|_| HydrateError::Hydrate("pack cache population closed".to_string()))?;
        metrics::histogram!("buzz_git_pack_cache_population_wait_seconds")
            .record(wait_started_at.elapsed().as_secs_f64());
        metrics::gauge!("buzz_git_pack_cache_populations_active").increment(1.0);
        let _population_permit = PopulationPermit { _permit: permit };
        let shard = self.root.join(&digest[..2]);
        tokio::fs::create_dir_all(&shard)
            .await
            .map_err(|error| HydrateError::Hydrate(format!("create pack cache shard: {error}")))?;
        let staging = Builder::new()
            .prefix(".staging-")
            .tempdir_in(&shard)
            .map_err(|error| {
                HydrateError::Hydrate(format!("create pack cache staging directory: {error}"))
            })?;
        let pack_path = staging.path().join(format!("pack-{digest}.pack"));
        let bytes = get_verified_limited(store, object_key, digest, max_pack_bytes).await?;
        let pack_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        tokio::fs::write(&pack_path, &bytes)
            .await
            .map_err(|error| HydrateError::Hydrate(format!("write cached pack: {error}")))?;
        install_or_generate_idx(store, staging.path(), digest, &pack_path, max_pack_bytes).await?;
        let idx_path = pack_path.with_extension("idx");
        let idx_bytes = tokio::fs::metadata(&idx_path)
            .await
            .map_err(|error| HydrateError::Hydrate(format!("stat cached idx: {error}")))?
            .len();
        let total_bytes = pack_bytes.saturating_add(idx_bytes);

        if self.max_bytes == 0 || total_bytes > self.max_bytes {
            metrics::counter!("buzz_git_pack_cache_bypasses_total").increment(1);
            return Ok(Arc::new(CachedPack {
                dir: staging.path().to_path_buf(),
                pack_path,
                idx_path,
                pack_bytes,
                total_bytes,
                _temporary: Some(staging),
            }));
        }

        let final_dir = shard.join(digest);
        if let Some(entry) = CachedPack::from_dir(final_dir.clone(), digest) {
            let entry = Arc::new(entry);
            self.insert(digest.to_string(), Arc::clone(&entry));
            return Ok(entry);
        }
        if final_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&final_dir).await;
        }
        if let Err(error) = tokio::fs::rename(staging.path(), &final_dir).await {
            // Another relay process sharing the scratch volume may have won
            // the same content-addressed publication race.
            if let Some(entry) = CachedPack::from_dir(final_dir.clone(), digest) {
                let entry = Arc::new(entry);
                self.insert(digest.to_string(), Arc::clone(&entry));
                return Ok(entry);
            }
            return Err(HydrateError::Hydrate(format!(
                "publish cached pack: {error}"
            )));
        }
        let entry = Arc::new(CachedPack::from_dir(final_dir, digest).ok_or_else(|| {
            HydrateError::Hydrate("published pack cache entry is incomplete".to_string())
        })?);
        self.insert(digest.to_string(), Arc::clone(&entry));
        Ok(entry)
    }

    fn insert(&self, digest: String, entry: Arc<CachedPack>) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.tick = state.tick.saturating_add(1);
        let last_used = state.tick;
        if let Some(previous) = state.entries.remove(&digest) {
            state.total_bytes = state.total_bytes.saturating_sub(previous.entry.total_bytes);
        }
        state.total_bytes = state.total_bytes.saturating_add(entry.total_bytes);
        state
            .entries
            .insert(digest, CacheRecord { entry, last_used });
        emit_size_metrics(&state);
    }

    fn invalidate(&self, digest: &str) {
        let removed = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let removed = state.entries.remove(digest);
            if let Some(record) = &removed {
                state.total_bytes = state.total_bytes.saturating_sub(record.entry.total_bytes);
            }
            emit_size_metrics(&state);
            removed
        };
        if let Some(record) = removed {
            if Arc::strong_count(&record.entry) == 1 {
                let _ = std::fs::remove_dir_all(&record.entry.dir);
            }
        }
    }

    fn prune(&self) {
        let mut removed = Vec::new();
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            while state.total_bytes > self.max_bytes {
                let candidate = state
                    .entries
                    .iter()
                    .filter(|(_, record)| Arc::strong_count(&record.entry) == 1)
                    .min_by_key(|(_, record)| record.last_used)
                    .map(|(digest, _)| digest.clone());
                let Some(digest) = candidate else {
                    break;
                };
                if let Some(record) = state.entries.remove(&digest) {
                    state.total_bytes = state.total_bytes.saturating_sub(record.entry.total_bytes);
                    removed.push(record.entry.dir.clone());
                    metrics::counter!("buzz_git_pack_cache_evictions_total").increment(1);
                }
            }
            emit_size_metrics(&state);
        }
        for dir in removed {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn remove_flight(&self, digest: &str, flight: &Arc<PopulationFlight>) {
        if let Entry::Occupied(entry) = self.flights.entry(digest.to_string()) {
            if Arc::ptr_eq(entry.get(), flight) && flight.participants.load(Ordering::Acquire) == 0
            {
                entry.remove();
            }
        }
    }

    #[cfg(test)]
    fn flight(&self, digest: &str) -> (Arc<PopulationFlight>, bool) {
        match self.flights.entry(digest.to_string()) {
            Entry::Occupied(entry) => {
                entry.get().participants.fetch_add(1, Ordering::Relaxed);
                (Arc::clone(entry.get()), true)
            }
            Entry::Vacant(entry) => {
                let flight = Arc::new(PopulationFlight {
                    lock: tokio::sync::Mutex::new(()),
                    participants: AtomicUsize::new(1),
                });
                entry.insert(Arc::clone(&flight));
                (flight, false)
            }
        }
    }
}

impl Drop for GitPackCache {
    fn drop(&mut self) {
        if let Some(task) = self.heartbeat_task.take() {
            task.abort();
        }
    }
}

async fn install_entry(
    entry: &CachedPack,
    digest: &str,
    destination: &Path,
) -> Result<(), HydrateError> {
    let destination_pack = destination.join(format!("pack-{digest}.pack"));
    let destination_idx = destination.join(format!("pack-{digest}.idx"));
    if tokio::fs::hard_link(&entry.pack_path, &destination_pack)
        .await
        .is_ok()
        && tokio::fs::hard_link(&entry.idx_path, &destination_idx)
            .await
            .is_ok()
    {
        return Ok(());
    }

    let _ = tokio::fs::remove_file(&destination_pack).await;
    let _ = tokio::fs::remove_file(&destination_idx).await;
    tokio::fs::copy(&entry.pack_path, &destination_pack)
        .await
        .map_err(|error| HydrateError::Hydrate(format!("copy cached pack: {error}")))?;
    if let Err(error) = tokio::fs::copy(&entry.idx_path, &destination_idx).await {
        let _ = tokio::fs::remove_file(&destination_pack).await;
        return Err(HydrateError::Hydrate(format!("copy cached idx: {error}")));
    }
    metrics::counter!("buzz_git_pack_cache_copy_fallbacks_total").increment(1);
    Ok(())
}

fn validate_digest(digest: &str) -> Result<(), HydrateError> {
    if digest.len() == 64
        && digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err(HydrateError::Hydrate(
            "pack cache digest is malformed".to_string(),
        ))
    }
}

fn regular_file_len(path: &Path) -> Option<u64> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    metadata.file_type().is_file().then_some(metadata.len())
}

fn emit_size_metrics(state: &CacheState) {
    metrics::gauge!("buzz_git_pack_cache_bytes").set(state.total_bytes as f64);
    metrics::gauge!("buzz_git_pack_cache_entries").set(state.entries.len() as f64);
}

fn cleanup_stale_sessions(cache_parent: &Path) {
    cleanup_sessions_older_than(cache_parent, STALE_SESSION_AGE);
}

fn cleanup_sessions_older_than(cache_parent: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(cache_parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("session-")
            || !entry.file_type().is_ok_and(|file_type| file_type.is_dir())
        {
            continue;
        }
        let heartbeat = entry.path().join(".heartbeat");
        let modified = std::fs::symlink_metadata(&heartbeat)
            .or_else(|_| entry.metadata())
            .and_then(|metadata| metadata.modified());
        let stale = modified
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > max_age);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(character: char) -> String {
        std::iter::repeat_n(character, 64).collect()
    }

    fn write_entry(root: &Path, digest: &str, pack_bytes: usize, idx_bytes: usize) -> PathBuf {
        let dir = root.join(&digest[..2]).join(digest);
        std::fs::create_dir_all(&dir).expect("entry dir");
        std::fs::write(dir.join(format!("pack-{digest}.pack")), vec![1; pack_bytes]).expect("pack");
        std::fs::write(dir.join(format!("pack-{digest}.idx")), vec![2; idx_bytes]).expect("idx");
        dir
    }

    #[test]
    fn entries_are_bounded_by_total_bytes() {
        let scratch = tempfile::TempDir::new().expect("scratch");
        let cache = GitPackCache::new(scratch.path(), 8, 2).expect("cache");
        let first = digest('a');
        let second = digest('b');
        let first_dir = write_entry(&cache.root, &first, 6, 2);
        let second_dir = write_entry(&cache.root, &second, 6, 2);
        cache.insert(
            first.clone(),
            Arc::new(CachedPack::from_dir(first_dir, &first).expect("first entry")),
        );
        cache.insert(
            second.clone(),
            Arc::new(CachedPack::from_dir(second_dir, &second).expect("second entry")),
        );
        cache.prune();

        let state = cache.state.lock().expect("state");
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.total_bytes, 8);
    }

    #[test]
    fn cache_sessions_are_process_isolated() {
        let parent = tempfile::TempDir::new().expect("parent");
        let first = GitPackCache::new(parent.path(), 8, 2).expect("first");
        let second = GitPackCache::new(parent.path(), 8, 2).expect("second");

        assert_ne!(first.root, second.root);
        assert!(first.root.starts_with(parent.path()));
        assert!(second.root.starts_with(parent.path()));
    }

    #[test]
    fn abandoned_sessions_are_removed_after_grace_period() {
        let parent = tempfile::TempDir::new().expect("parent");
        let abandoned = parent.path().join("session-abandoned");
        std::fs::create_dir(&abandoned).expect("abandoned");
        std::fs::write(abandoned.join(".heartbeat"), b"").expect("heartbeat");
        std::thread::sleep(Duration::from_millis(2));

        cleanup_sessions_older_than(parent.path(), Duration::ZERO);

        assert!(!abandoned.exists());
    }

    #[cfg(unix)]
    #[test]
    fn cache_parent_must_not_be_a_symlink() {
        let parent = tempfile::TempDir::new().expect("parent");
        let target = parent.path().join("target");
        let link = parent.path().join("link");
        std::fs::create_dir(&target).expect("target");
        std::os::unix::fs::symlink(&target, &link).expect("link");

        assert!(GitPackCache::new(&link, 8, 2).is_err());
    }

    #[test]
    fn cache_digest_cannot_escape_cache_root() {
        for digest in ["../pack", "/absolute", "g", &"a".repeat(63)] {
            assert!(validate_digest(digest).is_err(), "{digest:?}");
        }
        assert!(validate_digest(&"a".repeat(64)).is_ok());
    }

    #[tokio::test]
    async fn concurrent_digest_requests_share_one_flight() {
        let scratch = tempfile::TempDir::new().expect("scratch");
        let cache = GitPackCache::new(scratch.path(), 1024, 2).expect("cache");
        let digest = digest('c');
        let (leader, joined) = cache.flight(&digest);
        assert!(!joined);
        let leader_participant = FlightParticipant {
            cache: &cache,
            digest: &digest,
            flight: Arc::clone(&leader),
        };
        let leader_guard = leader.lock.lock().await;

        let (waiter, joined) = cache.flight(&digest);
        assert!(joined);
        assert!(Arc::ptr_eq(&leader, &waiter));
        let waiter_participant = FlightParticipant {
            cache: &cache,
            digest: &digest,
            flight: Arc::clone(&waiter),
        };
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), waiter.lock.lock())
                .await
                .is_err()
        );

        drop(leader_guard);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), waiter.lock.lock())
                .await
                .is_ok()
        );
        drop(waiter_participant);
        assert!(cache.flights.contains_key(&digest));
        drop(leader_participant);
        assert!(!cache.flights.contains_key(&digest));
    }

    #[tokio::test]
    async fn cancelled_waiter_keeps_active_flight_registered() {
        let scratch = tempfile::TempDir::new().expect("scratch");
        let cache = GitPackCache::new(scratch.path(), 1024, 2).expect("cache");
        let digest = digest('e');
        let (leader, _) = cache.flight(&digest);
        let leader_participant = FlightParticipant {
            cache: &cache,
            digest: &digest,
            flight: Arc::clone(&leader),
        };
        let leader_guard = leader.lock.lock().await;
        let (waiter, joined) = cache.flight(&digest);
        assert!(joined);
        let waiter_participant = FlightParticipant {
            cache: &cache,
            digest: &digest,
            flight: waiter,
        };

        drop(waiter_participant);
        assert!(cache.flights.contains_key(&digest));

        drop(leader_guard);
        drop(leader_participant);
        assert!(!cache.flights.contains_key(&digest));
    }

    #[tokio::test]
    async fn cached_pair_is_linked_into_request_workspace() {
        let scratch = tempfile::TempDir::new().expect("scratch");
        let cache = GitPackCache::new(scratch.path(), 1024, 2).expect("cache");
        let digest = digest('d');
        let source = write_entry(&cache.root, &digest, 3, 2);
        cache.insert(
            digest.clone(),
            Arc::new(CachedPack::from_dir(source.clone(), &digest).expect("entry")),
        );
        let entry = cache.lookup(&digest).expect("entry");
        let destination = tempfile::TempDir::new().expect("destination");

        install_entry(&entry, &digest, destination.path())
            .await
            .expect("install");

        assert_eq!(
            std::fs::read(destination.path().join(format!("pack-{digest}.pack")))
                .expect("installed pack"),
            vec![1; 3]
        );
        assert!(source.is_dir());
    }
}
