//! Database-sourced NIP-FI community resolution (`BUZZ_NIP_FI_COMMUNITY_SOURCE=database`).
//!
//! Each pod holds a snapshot of every registry-filtered binding of an active
//! community, loaded at startup and refreshed every 30s, start to start. A
//! host found in a fresh snapshot (loaded under two minutes ago) is answered
//! from memory. Any other host, and every host once the snapshot is stale,
//! goes to a bounded, single-flight database lookup whose positive answer is
//! used for that admission only and whose "not found" is cached for 5s.
//!
//! Every answer carries a [`FreshUntil`] two minutes after the read behind
//! it started (the snapshot load, or the lookup), re-checked after every
//! await, so no admission rests on a read older than that.
//!
//! A removed binding stops admitting new connections once a later refresh
//! publishes: within [`REFRESH_INTERVAL`] plus the refresh timeout (40s by
//! default) while refreshes succeed. While they fail, the last snapshot keeps
//! answering until it goes stale, so the bound is [`STALE_AFTER`] (two
//! minutes) from that snapshot's read; after that every host is looked up
//! live. Open WebSocket sessions are not rechecked when a binding is removed;
//! they end at their own session deadline.
//!
//! `buzz_nip_fi_binding_snapshot_loaded_timestamp_seconds` is written on
//! every refresh attempt with the start of the last good load (zero before
//! the first). Alert on `time() - buzz_nip_fi_binding_snapshot_loaded_timestamp_seconds > 120`
//! together with `absent(buzz_nip_fi_binding_snapshot_loaded_timestamp_seconds)`
//! for a database-source pod, since a pod that never exports it cannot age.
//!
//! The snapshot pointer and the negative cache change under one publication
//! lock. Publishing a snapshot and recording a creation bump a sequence
//! number; a "not found" read while the sequence moved is not cached.
//! Snapshot reads are lock-free.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use buzz_auth::{CommunityBinding, DenialClass, ProvisionBindingPolicy};
use buzz_core::tenant::normalize_host;
use buzz_db::RowCapExceeded;
use futures_util::future::{BoxFuture, FutureExt, Shared};
use tokio::sync::Semaphore;

use crate::config::ConfigError;

#[cfg(test)]
mod tests;

/// Refreshes start this often, measured start to start.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
/// The last good snapshot serves for at most this long after its load.
pub const STALE_AFTER: Duration = Duration::from_secs(120);
/// The first retry delay after a failed refresh; it doubles per failure up
/// to [`REFRESH_INTERVAL`].
pub const RETRY_FIRST: Duration = Duration::from_secs(2);
/// How long a "not found" miss is cached.
pub const NEGATIVE_TTL: Duration = Duration::from_secs(5);

/// Resolver bounds. Every bound fails closed. Only the first four are read
/// from the environment; the lookup queue limits are fixed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolverBounds {
    /// `BUZZ_NIP_FI_RESOLVER_NEGATIVE_CACHE_MAX`, at most 1,000,000.
    pub negative_cache_max: usize,
    /// `BUZZ_NIP_FI_RESOLVER_MAX_INFLIGHT`, kept below the writer pool size
    /// so lookups never take every database connection.
    pub max_inflight: usize,
    /// `BUZZ_NIP_FI_SNAPSHOT_MAX_ROWS`, at most 10,000,000:
    /// community-issuer pairs, not communities.
    pub snapshot_max_rows: usize,
    /// `BUZZ_NIP_FI_SNAPSHOT_REFRESH_TIMEOUT_MS`, shorter than
    /// [`REFRESH_INTERVAL`].
    pub refresh_timeout: Duration,
    /// Hosts with a lookup in flight.
    pub max_pending_hosts: usize,
    /// Callers waiting on those lookups.
    pub max_waiters: usize,
    /// How long a lookup waits for an inflight permit.
    pub permit_wait: Duration,
    /// How long a lookup's database read may take.
    pub lookup_timeout: Duration,
}

impl Default for ResolverBounds {
    fn default() -> Self {
        Self {
            negative_cache_max: 100_000,
            max_inflight: 8,
            snapshot_max_rows: 1_000_000,
            refresh_timeout: Duration::from_secs(10),
            max_pending_hosts: 1_024,
            max_waiters: 4_096,
            permit_wait: Duration::from_millis(50),
            lookup_timeout: Duration::from_millis(500),
        }
    }
}

impl ResolverBounds {
    /// Read the tunable bounds from the environment, defaulting unset ones,
    /// and keep `max_inflight` below `db_pool_size` (at least 1).
    pub fn from_env(db_pool_size: u32) -> Result<Self, ConfigError> {
        let d = Self::default();
        let bounded = |name: &str, max: u64, default: usize| -> Result<usize, ConfigError> {
            Ok(crate::nip_fi_config::parse_u64_bounded(name, 1, max)?
                .map_or(default, |v| v as usize))
        };
        let below_interval = REFRESH_INTERVAL.as_millis() as u64 - 1;
        let refresh_timeout = crate::nip_fi_config::parse_u64_bounded(
            "BUZZ_NIP_FI_SNAPSHOT_REFRESH_TIMEOUT_MS",
            1,
            below_interval,
        )?
        .map_or(d.refresh_timeout, Duration::from_millis);
        let max_inflight = bounded(
            "BUZZ_NIP_FI_RESOLVER_MAX_INFLIGHT",
            u32::MAX.into(),
            d.max_inflight,
        )?
        .min(db_pool_size.saturating_sub(1).max(1) as usize);
        Ok(Self {
            negative_cache_max: bounded(
                "BUZZ_NIP_FI_RESOLVER_NEGATIVE_CACHE_MAX",
                1_000_000,
                d.negative_cache_max,
            )?,
            max_inflight,
            snapshot_max_rows: bounded(
                "BUZZ_NIP_FI_SNAPSHOT_MAX_ROWS",
                10_000_000,
                d.snapshot_max_rows,
            )?,
            refresh_timeout,
            ..d
        })
    }
}

/// A snapshot load's `(host, issuer)` rows, or the cap it exceeded; the
/// outer error is a failed read.
pub type SnapshotRows = Result<Result<Vec<(String, String)>, RowCapExceeded>, String>;

/// Where bindings are read from. Implemented by [`buzz_db::Db`]; tests
/// substitute counting and failing sources.
pub trait BindingSource: Send + Sync + 'static {
    /// One consistent registry-filtered read of every active binding, as
    /// `(host, issuer)` rows; more than `max_rows` fails.
    fn load<'a>(&'a self, registry: &'a [String], max_rows: usize) -> BoxFuture<'a, SnapshotRows>;

    /// The registry-filtered issuers of the active community at `host`.
    fn lookup<'a>(
        &'a self,
        host: &'a str,
        registry: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<String>, String>>;
}

impl BindingSource for buzz_db::Db {
    fn load<'a>(&'a self, registry: &'a [String], max_rows: usize) -> BoxFuture<'a, SnapshotRows> {
        async move {
            self.load_nip_fi_binding_snapshot(registry, max_rows)
                .await
                .map_err(|e| e.to_string())
        }
        .boxed()
    }

    fn lookup<'a>(
        &'a self,
        host: &'a str,
        registry: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<String>, String>> {
        async move {
            self.lookup_nip_fi_admission_binding(host, registry)
                .await
                .map_err(|e| e.to_string())
        }
        .boxed()
    }
}

/// Monotonic time since the resolver started; replaceable in tests.
pub trait Clock: Send + Sync + 'static {
    /// Elapsed time on this clock.
    fn now(&self) -> Duration;
}

struct TokioClock(tokio::time::Instant);

impl Clock for TokioClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// The staleness deadline of the read an admission resolved against: two
/// minutes after that snapshot load or lookup started. Admission re-checks
/// it after its own awaits, so a binding resolved just inside the limit
/// cannot be admitted after it. Holds no map.
#[derive(Clone)]
pub struct FreshUntil {
    clock: Arc<dyn Clock>,
    deadline: Duration,
}

impl FreshUntil {
    /// `authorization_unavailable` once the deadline has passed.
    pub fn check(&self) -> Result<(), DenialClass> {
        (self.clock.now() < self.deadline)
            .then_some(())
            .ok_or(DenialClass::AuthorizationUnavailable)
    }
}

/// One published full load.
struct Published {
    map: HashMap<String, Arc<CommunityBinding>>,
    /// Clock time the load started; the staleness deadline counts from here.
    loaded_at: Duration,
}

/// State changed only under the publication lock.
struct Writable {
    negative: lru::LruCache<String, Duration>,
}

/// A lookup's binding, if any, with the clock time its read started.
type MissOutcome = Result<Option<(Arc<CommunityBinding>, Duration)>, ()>;
type PendingLookup = Shared<BoxFuture<'static, MissOutcome>>;

/// Why a refresh published nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadFailure {
    /// The store failed.
    Store(String),
    /// More rows than `BUZZ_NIP_FI_SNAPSHOT_MAX_ROWS`.
    OverCap,
    /// The whole attempt exceeded `BUZZ_NIP_FI_SNAPSHOT_REFRESH_TIMEOUT_MS`.
    Timeout,
    /// Its read started before the published one's (defensive).
    Superseded,
    /// An earlier attempt's map build is still running; no new load starts.
    BuildOutstanding,
}

impl LoadFailure {
    fn label(&self) -> &'static str {
        match self {
            Self::Store(_) => "store",
            Self::OverCap => "over_cap",
            Self::Timeout => "timeout",
            Self::Superseded => "superseded",
            Self::BuildOutstanding => "build_outstanding",
        }
    }
}

/// The database-sourced community resolver.
pub struct DatabaseResolver {
    source: Arc<dyn BindingSource>,
    policy: ProvisionBindingPolicy,
    registry: Vec<String>,
    bounds: ResolverBounds,
    clock: Arc<dyn Clock>,
    published: ArcSwapOption<Published>,
    publication: Mutex<Writable>,
    sequence: AtomicU64,
    pending: Mutex<HashMap<String, PendingLookup>>,
    waiters: AtomicUsize,
    permits: Arc<Semaphore>,
    /// Set while an attempt (its load or its map build) is alive. A build
    /// abandoned by a timeout keeps it set until the builder thread returns.
    building: Arc<AtomicBool>,
    /// Wall-clock seconds of the last successful load's start, as `f64`
    /// bits; zero before the first.
    loaded_wall_secs: AtomicU64,
    /// A uniform draw in `[0, 1)` for refresh jitter; fixed in tests.
    pub(crate) jitter: fn() -> f64,
    #[cfg(test)]
    pub(crate) hooks: Arc<hooks::Hooks>,
}

impl DatabaseResolver {
    /// A resolver with no snapshot yet: every admission is unavailable until
    /// the first [`DatabaseResolver::refresh`] succeeds.
    pub fn new(
        source: Arc<dyn BindingSource>,
        policy: ProvisionBindingPolicy,
        registry: Vec<String>,
        bounds: ResolverBounds,
    ) -> Self {
        Self::with_clock(
            source,
            policy,
            registry,
            bounds,
            Arc::new(TokioClock(tokio::time::Instant::now())),
        )
    }

    pub(crate) fn with_clock(
        source: Arc<dyn BindingSource>,
        policy: ProvisionBindingPolicy,
        registry: Vec<String>,
        bounds: ResolverBounds,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let negative_cap =
            NonZeroUsize::new(bounds.negative_cache_max).unwrap_or(NonZeroUsize::MIN);
        Self {
            source,
            policy,
            registry,
            permits: Arc::new(Semaphore::new(bounds.max_inflight)),
            bounds,
            clock,
            published: ArcSwapOption::empty(),
            publication: Mutex::new(Writable {
                negative: lru::LruCache::new(negative_cap),
            }),
            sequence: AtomicU64::new(0),
            pending: Mutex::new(HashMap::new()),
            waiters: AtomicUsize::new(0),
            building: Arc::default(),
            loaded_wall_secs: AtomicU64::new(0),
            jitter: rand::random::<f64>,
            #[cfg(test)]
            hooks: Arc::default(),
        }
    }

    /// Resolve the community at `raw_host` for one admission: from a fresh
    /// snapshot when it lists the host, otherwise from a database lookup,
    /// each answer judged by its own read's age.
    pub async fn resolve(
        self: &Arc<Self>,
        raw_host: &str,
    ) -> Result<(Arc<CommunityBinding>, FreshUntil), DenialClass> {
        const UNAVAILABLE: DenialClass = DenialClass::AuthorizationUnavailable;
        let host = normalize_host(raw_host);
        if !self.policy.is_bindable(&host) {
            return Err(UNAVAILABLE);
        }
        if let Some(hit) = self.fresh_snapshot_hit(&host) {
            return Ok(hit);
        }
        if self.negatively_cached(&host) {
            return Err(UNAVAILABLE);
        }
        match self.miss(host).await {
            Ok(Some((binding, read_started))) => Ok((binding, self.fresh_until(read_started))),
            Ok(None) | Err(()) => Err(UNAVAILABLE),
        }
    }

    /// Record that this pod just committed a binding for `host`: drop its
    /// cached "not found" and bump the sequence, so a lookup that read
    /// before the commit cannot cache one, and the creator's first login
    /// looks it up.
    pub fn record_creation(&self, host: &str) {
        let host = normalize_host(host);
        let mut writable = self.lock();
        writable.negative.pop(&host);
        self.bump(&writable);
    }

    /// Run one full load and publish it. A failure publishes nothing and
    /// leaves the staleness deadline where it was.
    pub async fn refresh(&self) -> Result<(), LoadFailure> {
        let started = self.clock.now();
        // The attempt's absolute deadline. The timeout below only fires at an
        // await; the synchronous map build and publication after the read
        // check this instead, so expired work is never published.
        let deadline = started + self.bounds.refresh_timeout;
        let started_wall = std::time::SystemTime::now();
        let timer = std::time::Instant::now();
        // Held by the attempt, then by its builder thread: dropping the
        // attempt on timeout cannot release it while the build still runs.
        let Some(slot) = BuildSlot::claim(&self.building) else {
            metrics::counter!("buzz_nip_fi_binding_snapshot_load_failures_total", "reason" => LoadFailure::BuildOutstanding.label())
                .increment(1);
            return Err(LoadFailure::BuildOutstanding);
        };
        let attempt = async {
            let load = self
                .source
                .load(&self.registry, self.bounds.snapshot_max_rows)
                .await
                .map_err(LoadFailure::Store)?
                .map_err(|_| LoadFailure::OverCap)?;
            let rows = load.len();
            // Up to the row cap of grouping and binding construction: run it
            // off the runtime worker.
            // Set when this attempt is dropped (timeout or cancel); the
            // builder checks it, and the deadline, as it goes.
            let abandoned = Abandoned::default();
            let stop = {
                let flag = Arc::clone(&abandoned.0);
                let clock = Arc::clone(&self.clock);
                move || flag.load(Ordering::Acquire) || clock.now() >= deadline
            };
            #[cfg(test)]
            let test_hooks = Arc::clone(&self.hooks);
            let map = tokio::task::spawn_blocking(move || {
                let _slot = slot;
                #[cfg(test)]
                test_hooks.hit_sync(hooks::BUILD_START);
                build_map(load, stop)
            })
            .await
            .map_err(|e| LoadFailure::Store(format!("map build failed: {e}")))?
            .ok_or(LoadFailure::Timeout)?;
            #[cfg(test)]
            self.hooks.hit(hooks::REFRESH_BEFORE_PUBLISH).await;
            let replaced = self.publish(map, started, deadline)?;
            // Freeing up to the row cap of bindings is real work; keep it
            // off the runtime worker and outside the publication lock.
            if let Some(old) = replaced {
                tokio::task::spawn_blocking(move || drop(old));
            }
            Ok(rows)
        };
        let result = match tokio::time::timeout(self.bounds.refresh_timeout, attempt).await {
            Ok(result) => result,
            Err(_) => Err(LoadFailure::Timeout),
        };
        metrics::histogram!("buzz_nip_fi_binding_snapshot_load_duration_seconds")
            .record(timer.elapsed().as_secs_f64());
        if result.is_ok() {
            if let Ok(since_epoch) = started_wall.duration_since(std::time::UNIX_EPOCH) {
                self.loaded_wall_secs
                    .store(since_epoch.as_secs_f64().to_bits(), Ordering::Relaxed);
            }
        }
        // Rewritten on every attempt, failed ones included, so the series
        // exists from the first attempt and keeps reporting the last good
        // load. A timestamp, not an age: `time() - ...` keeps aging at scrape
        // time even if the refresh task stops; `absent()` catches a pod that
        // never exports it.
        let loaded = f64::from_bits(self.loaded_wall_secs.load(Ordering::Relaxed));
        metrics::gauge!("buzz_nip_fi_binding_snapshot_loaded_timestamp_seconds").set(loaded);
        metrics::gauge!("buzz_nip_fi_binding_snapshot_max_rows")
            .set(self.bounds.snapshot_max_rows as f64);
        match result {
            Ok(rows) => {
                metrics::gauge!("buzz_nip_fi_binding_snapshot_rows").set(rows as f64);
                Ok(())
            }
            Err(failure) => {
                tracing::warn!(
                    reason = failure.label(),
                    "NIP-FI binding snapshot load failed"
                );
                metrics::counter!("buzz_nip_fi_binding_snapshot_load_failures_total", "reason" => failure.label())
                    .increment(1);
                Err(failure)
            }
        }
    }

    /// Refresh every [`REFRESH_INTERVAL`], start to start, never
    /// overlapping. After a failure the next attempt comes sooner, backing
    /// off from [`RETRY_FIRST`] and doubling up to the interval; every delay
    /// is jittered down by up to a tenth so pods do not refresh in lockstep.
    /// The caller has already done the startup load.
    pub async fn run_refresh_loop(self: Arc<Self>, cancel: tokio_util::sync::CancellationToken) {
        let mut failures = 0u32;
        let mut started = tokio::time::Instant::now();
        loop {
            let delay = jittered(next_refresh_delay(failures), (self.jitter)());
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep_until(started + delay) => {}
            }
            started = tokio::time::Instant::now();
            failures = match self.refresh().await {
                Ok(()) => 0,
                Err(_) => failures.saturating_add(1),
            };
        }
    }

    // ── Internals ───────────────────────────────────────────────────────────

    fn lock(&self) -> std::sync::MutexGuard<'_, Writable> {
        self.publication
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Bump the publication sequence; the caller holds the lock.
    fn bump(&self, _held: &Writable) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
    }

    /// The deadline for an answer whose read started at `read_started`.
    fn fresh_until(&self, read_started: Duration) -> FreshUntil {
        FreshUntil {
            clock: Arc::clone(&self.clock),
            deadline: read_started + STALE_AFTER,
        }
    }

    /// `host` in the published snapshot, unless that snapshot is stale (or
    /// none is loaded): a stale one may still list a removed binding. Clones
    /// only the selected binding.
    fn fresh_snapshot_hit(&self, host: &str) -> Option<(Arc<CommunityBinding>, FreshUntil)> {
        let published = self.published.load();
        let published = published.as_ref()?;
        let fresh = self.fresh_until(published.loaded_at);
        fresh.check().ok()?;
        Some((Arc::clone(published.map.get(host)?), fresh))
    }

    fn negatively_cached(&self, host: &str) -> bool {
        let now = self.clock.now();
        let mut writable = self.lock();
        match writable.negative.get(host) {
            Some(expiry) if *expiry > now => true,
            Some(_) => {
                writable.negative.pop(host);
                false
            }
            None => false,
        }
    }

    fn publish(
        &self,
        map: HashMap<String, Arc<CommunityBinding>>,
        started: Duration,
        deadline: Duration,
    ) -> Result<Option<Arc<Published>>, LoadFailure> {
        let mut writable = self.lock();
        if self
            .published
            .load()
            .as_ref()
            .is_some_and(|published| started < published.loaded_at)
        {
            return Err(LoadFailure::Superseded);
        }
        #[cfg(test)]
        self.hooks.hit_sync(hooks::PUBLISH_BEFORE_WRITE);
        // Checked under the lock, immediately before the first write:
        // waiting for the lock cannot let an expired attempt publish.
        if self.clock.now() >= deadline {
            return Err(LoadFailure::Timeout);
        }
        let replaced = self.published.swap(Some(Arc::new(Published {
            map,
            loaded_at: started,
        })));
        writable.negative.clear();
        metrics::gauge!("buzz_nip_fi_resolver_negative_cache_size").set(0.0);
        self.bump(&writable);
        Ok(replaced)
    }

    /// The bounded, single-flight lookup. A positive answer is returned to
    /// its waiters only; a "not found" is cached only if no creation or
    /// publication happened while it ran.
    async fn miss(self: &Arc<Self>, host: String) -> MissOutcome {
        let lookup = {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            match pending.get(&host) {
                Some(lookup) => lookup.clone(),
                None => {
                    if pending.len() >= self.bounds.max_pending_hosts {
                        reject("pending_hosts");
                        return Err(());
                    }
                    let lookup = self.spawn_lookup(host.clone());
                    pending.insert(host, lookup.clone());
                    lookup
                }
            }
        };
        let Some(_waiter) = WaiterSlot::take(&self.waiters, self.bounds.max_waiters) else {
            reject("waiters");
            return Err(());
        };
        lookup.await
    }

    /// Run one lookup on its own task, so it completes (and leaves the
    /// pending map) even if every waiter goes away.
    fn spawn_lookup(self: &Arc<Self>, host: String) -> PendingLookup {
        let this = Arc::clone(self);
        let task = tokio::spawn(async move {
            // Leaves the pending map even if the lookup panics, so the host
            // is not stuck behind a dead entry.
            struct Leave<'a>(&'a DatabaseResolver, &'a str);
            impl Drop for Leave<'_> {
                fn drop(&mut self) {
                    self.0
                        .pending
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(self.1);
                }
            }
            let _leave = Leave(&this, &host);
            let sequence = this.sequence.load(Ordering::Acquire);
            let outcome = this.lookup_once(&host).await;
            if matches!(outcome, Ok(None)) {
                this.store_negative(&host, sequence);
            }
            outcome
        });
        async move { task.await.unwrap_or(Err(())) }
            .boxed()
            .shared()
    }

    async fn lookup_once(&self, host: &str) -> MissOutcome {
        let permit = match tokio::time::timeout(
            self.bounds.permit_wait,
            Arc::clone(&self.permits).acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            _ => {
                reject("permit");
                return Err(());
            }
        };
        // Every waiter on this lookup shares this deadline origin.
        let read_started = self.clock.now();
        let timer = std::time::Instant::now();
        let read = tokio::time::timeout(
            self.bounds.lookup_timeout,
            self.source.lookup(host, &self.registry),
        )
        .await;
        drop(permit);
        metrics::histogram!("buzz_nip_fi_resolver_lookup_duration_seconds")
            .record(timer.elapsed().as_secs_f64());
        #[cfg(test)]
        self.hooks.hit(hooks::MISS_AFTER_READ).await;
        match read {
            Err(_) => {
                metrics::counter!("buzz_nip_fi_resolver_lookup_timeouts_total").increment(1);
                Err(())
            }
            Ok(Err(_)) => Err(()),
            Ok(Ok(issuers)) if issuers.is_empty() => Ok(None),
            Ok(Ok(issuers)) => Ok(binding_for(host, issuers).map(|b| (b, read_started))),
        }
    }

    fn store_negative(&self, host: &str, sequence: u64) {
        let mut writable = self.lock();
        if self.sequence.load(Ordering::Acquire) != sequence {
            return;
        }
        let expiry = self.clock.now() + NEGATIVE_TTL;
        if let Some((evicted, _)) = writable.negative.push(host.to_owned(), expiry) {
            if evicted != host {
                metrics::counter!("buzz_nip_fi_resolver_negative_evictions_total").increment(1);
            }
        }
        metrics::gauge!("buzz_nip_fi_resolver_negative_cache_size")
            .set(writable.negative.len() as f64);
    }

    #[cfg(test)]
    pub(crate) fn negative_len(&self) -> usize {
        self.lock().negative.len()
    }

    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    #[cfg(test)]
    pub(crate) fn waiters_now(&self) -> usize {
        self.waiters.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn inflight_now(&self) -> usize {
        self.bounds.max_inflight - self.permits.available_permits()
    }
}

fn reject(reason: &'static str) {
    metrics::counter!("buzz_nip_fi_resolver_miss_rejections_total", "reason" => reason)
        .increment(1);
}

/// One counted caller waiting on a pending lookup.
struct WaiterSlot<'a>(&'a AtomicUsize);

impl<'a> WaiterSlot<'a> {
    fn take(waiters: &'a AtomicUsize, max: usize) -> Option<Self> {
        waiters
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < max).then_some(n + 1)
            })
            .ok()
            .map(|_| Self(waiters))
    }
}

impl Drop for WaiterSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The delay before the next refresh after `failures` consecutive failures.
fn next_refresh_delay(failures: u32) -> Duration {
    match failures {
        0 => REFRESH_INTERVAL,
        n => RETRY_FIRST
            .saturating_mul(1 << (n - 1).min(16))
            .min(REFRESH_INTERVAL),
    }
}

/// `delay` shortened by up to a tenth, by `unit` in `[0, 1)`.
fn jittered(delay: Duration, unit: f64) -> Duration {
    delay.mul_f64(1.0 - unit.clamp(0.0, 1.0) / 10.0)
}

fn binding_for(host: &str, issuers: Vec<String>) -> Option<Arc<CommunityBinding>> {
    CommunityBinding::new(format!("https://{host}"), issuers)
        .ok()
        .map(Arc::new)
}

/// Group rows into bindings, or `None` as soon as `stop` reports the
/// attempt abandoned or expired; checked in both passes.
fn build_map(
    rows: Vec<(String, String)>,
    stop: impl Fn() -> bool,
) -> Option<HashMap<String, Arc<CommunityBinding>>> {
    let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
    for (host, issuer) in rows {
        if stop() {
            return None;
        }
        grouped
            .entry(normalize_host(&host))
            .or_default()
            .push(issuer);
    }
    let mut map = HashMap::with_capacity(grouped.len());
    for (host, issuers) in grouped {
        if stop() {
            return None;
        }
        if let Some(binding) = binding_for(&host, issuers) {
            map.insert(host, binding);
        }
    }
    Some(map)
}

/// The resolver's single in-flight attempt; released on drop, wherever the
/// owner (the attempt or its builder thread) ends, including a panic.
struct BuildSlot(Arc<AtomicBool>);

impl BuildSlot {
    fn claim(building: &Arc<AtomicBool>) -> Option<Self> {
        building
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(Arc::clone(building)))
    }
}

impl Drop for BuildSlot {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Raised when the awaiting attempt is dropped, telling its builder to stop.
#[derive(Default)]
struct Abandoned(Arc<AtomicBool>);

impl Drop for Abandoned {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[cfg(test)]
pub(crate) mod hooks {
    //! Per-resolver pause points for ordering tests.
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tokio::sync::Semaphore;

    pub(crate) const REFRESH_BEFORE_PUBLISH: &str = "refresh.before_publish";
    pub(crate) const MISS_AFTER_READ: &str = "miss.after_read";
    /// Synchronous, under the publication lock: see [`Hooks::on_sync`].
    pub(crate) const PUBLISH_BEFORE_WRITE: &str = "publish.before_write";
    /// Synchronous, on the builder thread before the map build.
    pub(crate) const BUILD_START: &str = "build.start";

    /// An armed pause: production signals `reached`, then waits for `release`.
    pub(crate) struct Gate {
        reached: Semaphore,
        release: Semaphore,
    }

    impl Gate {
        pub(crate) async fn reached(&self) {
            self.reached.acquire().await.expect("open").forget();
        }

        pub(crate) fn release(&self) {
            self.release.add_permits(1);
        }
    }

    #[derive(Default)]
    pub(crate) struct Hooks(
        Mutex<HashMap<&'static str, Arc<Gate>>>,
        Mutex<HashMap<&'static str, SyncAction>>,
    );

    type SyncAction = Arc<dyn Fn() + Send + Sync>;

    impl Hooks {
        /// Pause every later hit of `point` until released, once per release.
        pub(crate) fn arm(&self, point: &'static str) -> Arc<Gate> {
            let gate = Arc::new(Gate {
                reached: Semaphore::new(0),
                release: Semaphore::new(0),
            });
            self.0.lock().unwrap().insert(point, Arc::clone(&gate));
            gate
        }

        pub(crate) fn disarm(&self, point: &'static str) {
            self.0.lock().unwrap().remove(point);
        }

        /// Run `action` at every later hit of the synchronous `point`.
        pub(crate) fn on_sync(
            &self,
            point: &'static str,
            action: impl Fn() + Send + Sync + 'static,
        ) {
            self.1.lock().unwrap().insert(point, Arc::new(action));
        }

        pub(crate) fn hit_sync(&self, point: &'static str) {
            let action = self.1.lock().unwrap().get(point).cloned();
            if let Some(action) = action {
                action();
            }
        }

        pub(crate) async fn hit(&self, point: &'static str) {
            let gate = self.0.lock().unwrap().get(point).cloned();
            if let Some(gate) = gate {
                gate.reached.add_permits(1);
                gate.release.acquire().await.expect("open").forget();
            }
        }
    }
}
