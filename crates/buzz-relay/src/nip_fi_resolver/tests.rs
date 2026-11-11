//! Resolver ordering, staleness, and bounds tests. The store is an in-memory
//! fake that counts every call; the clock is manual.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buzz_auth::{DenialClass, ProvisionBindingPolicy};
use buzz_db::RowCapExceeded;
use futures_util::future::{BoxFuture, FutureExt};

use super::hooks::{BUILD_START, MISS_AFTER_READ, PUBLISH_BEFORE_WRITE, REFRESH_BEFORE_PUBLISH};
use super::*;

const ISS: &str = "https://app.builderlab.xyz/api/goose";
const UNAVAILABLE: Result<(), DenialClass> = Err(DenialClass::AuthorizationUnavailable);

fn policy() -> ProvisionBindingPolicy {
    ProvisionBindingPolicy::parse(
        &serde_json::json!({"host_suffix": "communities.buzz.xyz", "authorized_issuers": [ISS]})
            .to_string(),
        &serde_json::json!([
            {"issuer": ISS, "command_audiences": ["https://communities.buzz.xyz"], "token_class": "nip-fi+jwt"}
        ])
        .to_string(),
    )
    .expect("valid policy")
}

fn host(label: &str) -> String {
    format!("{label}.communities.buzz.xyz")
}

#[derive(Default)]
struct ManualClock(AtomicU64);

impl ManualClock {
    fn set(&self, at: Duration) {
        self.0.store(at.as_millis() as u64, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

/// In-memory bindings.
struct FakeStore {
    hosts: Mutex<Vec<String>>,
    fail_loads: std::sync::atomic::AtomicBool,
    lookup_fails: std::sync::atomic::AtomicBool,
    lookup_panics: std::sync::atomic::AtomicBool,
    loads: AtomicUsize,
    /// Rows the last load pulled from its stream.
    rows_pulled: AtomicUsize,
    lookups: Mutex<Vec<String>>,
    lookup_delay: Mutex<Duration>,
    load_delay: Mutex<Duration>,
}

impl FakeStore {
    fn new(hosts: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            hosts: Mutex::new(hosts.iter().map(|l| host(l)).collect()),
            fail_loads: false.into(),
            lookup_fails: false.into(),
            lookup_panics: false.into(),
            loads: AtomicUsize::new(0),
            rows_pulled: AtomicUsize::new(0),
            lookups: Mutex::new(Vec::new()),
            lookup_delay: Mutex::new(Duration::ZERO),
            load_delay: Mutex::new(Duration::ZERO),
        })
    }
    fn add(&self, label: &str) {
        self.hosts.lock().unwrap().push(host(label));
    }
    fn remove(&self, label: &str) {
        self.hosts.lock().unwrap().retain(|h| h != &host(label));
    }
    fn lookups_for(&self, label: &str) -> usize {
        let h = host(label);
        self.lookups
            .lock()
            .unwrap()
            .iter()
            .filter(|l| **l == h)
            .count()
    }
    fn total_lookups(&self) -> usize {
        self.lookups.lock().unwrap().len()
    }
}

impl BindingSource for FakeStore {
    fn load<'a>(&'a self, _registry: &'a [String], max_rows: usize) -> BoxFuture<'a, SnapshotRows> {
        async move {
            self.loads.fetch_add(1, Ordering::SeqCst);
            // A consistent read: rows as of the read's start, however long
            // it then takes.
            let hosts = self.hosts.lock().unwrap().clone();
            let delay = *self.load_delay.lock().unwrap();
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if self.fail_loads.load(Ordering::SeqCst) {
                return Err("load failed".into());
            }
            // Pulled one row at a time and capped as the real store does.
            self.rows_pulled.store(0, Ordering::SeqCst);
            let mut rows = Vec::new();
            for host in hosts {
                self.rows_pulled.fetch_add(1, Ordering::SeqCst);
                if rows.len() == max_rows {
                    return Ok(Err(RowCapExceeded(max_rows)));
                }
                rows.push((host, ISS.to_owned()));
            }
            Ok(Ok(rows))
        }
        .boxed()
    }

    fn lookup<'a>(
        &'a self,
        host: &'a str,
        _registry: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<String>, String>> {
        async move {
            self.lookups.lock().unwrap().push(host.to_owned());
            let delay = *self.lookup_delay.lock().unwrap();
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            assert!(
                !self.lookup_panics.load(Ordering::SeqCst),
                "lookup panicked"
            );
            if self.lookup_fails.load(Ordering::SeqCst) {
                return Err("pool closed".into());
            }
            let found = self.hosts.lock().unwrap().iter().any(|h| h == host);
            Ok(if found {
                vec![ISS.to_owned()]
            } else {
                Vec::new()
            })
        }
        .boxed()
    }
}

struct Fixture {
    store: Arc<FakeStore>,
    clock: Arc<ManualClock>,
    resolver: Arc<DatabaseResolver>,
}

fn fixture_with(hosts: &[&str], bounds: ResolverBounds) -> Fixture {
    let store = FakeStore::new(hosts);
    let clock = Arc::new(ManualClock::default());
    let resolver = Arc::new(DatabaseResolver::with_clock(
        Arc::clone(&store) as Arc<dyn BindingSource>,
        policy(),
        vec![ISS.to_owned()],
        bounds,
        Arc::clone(&clock) as Arc<dyn Clock>,
    ));
    Fixture {
        store,
        clock,
        resolver,
    }
}

fn fixture(hosts: &[&str]) -> Fixture {
    fixture_with(hosts, ResolverBounds::default())
}

impl Fixture {
    async fn resolve(&self, label: &str) -> Result<(), DenialClass> {
        self.resolver.resolve(&host(label)).await.map(|_| ())
    }
}

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s)
}

// A lookup that panics still leaves the pending map, so the next admission
// for that host runs a fresh lookup. Mutation: removing the entry only on the
// normal path leaves the host denied by the dead lookup forever.
#[tokio::test]
async fn panicked_lookup_does_not_strand_the_host() {
    let f = fixture(&["a"]);
    f.store.lookup_panics.store(true, Ordering::SeqCst);
    assert!(f.resolve("a").await.is_err());
    f.store.lookup_panics.store(false, Ordering::SeqCst);
    assert_eq!(f.resolve("a").await, Ok(()));
    assert_eq!(f.store.lookups_for("a"), 2);
}

// ── Before the first load, and once the snapshot is stale ──────────────────

// With no snapshot loaded, admissions go to the live lookup; a failed or
// over-cap load publishes nothing.
#[tokio::test]
async fn unloaded_resolver_uses_live_lookups_and_failed_or_over_cap_load_keeps_it_unloaded() {
    let f = fixture_with(
        &["a", "b"],
        ResolverBounds {
            snapshot_max_rows: 1,
            ..ResolverBounds::default()
        },
    );
    assert_eq!(f.resolve("a").await, Ok(()));
    assert_eq!(f.resolve("unknown").await, UNAVAILABLE);
    assert_eq!(f.resolver.refresh().await, Err(LoadFailure::OverCap));
    f.store.fail_loads.store(true, Ordering::SeqCst);
    assert!(matches!(
        f.resolver.refresh().await,
        Err(LoadFailure::Store(_))
    ));
    assert!(f.resolver.published.load().is_none());
    assert_eq!(f.resolve("b").await, Ok(()));
    assert_eq!(f.store.lookups_for("a"), 1);
    assert_eq!(f.store.lookups_for("b"), 1);
}

// While refreshes fail, the last snapshot answers for two minutes after its
// load started; after that every host goes to the live lookup, which sees a
// removal at once and fails closed when the database does. Mutation: serving
// a stale snapshot admits "gone"; denying once stale refuses "a".
#[tokio::test]
async fn stale_snapshot_is_skipped_for_live_lookups() {
    let f = fixture(&["a", "gone"]);
    f.resolver.refresh().await.unwrap();
    f.store.fail_loads.store(true, Ordering::SeqCst);
    f.store.remove("gone");

    f.clock.set(secs(60.0));
    assert!(f.resolver.refresh().await.is_err());
    f.clock.set(secs(119.0));
    assert_eq!(f.resolve("a").await, Ok(()));
    assert_eq!(
        f.resolve("gone").await,
        Ok(()),
        "fresh snapshot still lists it"
    );
    assert_eq!(
        f.store.total_lookups(),
        0,
        "fresh snapshot hosts never look up"
    );

    f.clock.set(secs(121.0));
    assert_eq!(f.resolve("a").await, Ok(()));
    assert_eq!(f.resolve("gone").await, UNAVAILABLE);
    assert_eq!(f.store.lookups_for("a"), 1);
    assert_eq!(f.store.lookups_for("gone"), 1);
    assert_eq!(f.resolver.negative_len(), 1, "the not-found is cached");
    f.store.lookup_fails.store(true, Ordering::SeqCst);
    assert_eq!(f.resolve("a").await, UNAVAILABLE, "database down");

    f.store.fail_loads.store(false, Ordering::SeqCst);
    f.resolver.refresh().await.unwrap();
    assert_eq!(f.resolve("a").await, Ok(()));
    assert_eq!(f.store.lookups_for("a"), 2, "fresh snapshot again");
}

// A lookup answer is judged by its own read, not the snapshot's: the
// snapshot going stale while the lookup runs still admits, and the answer
// expires two minutes after its read started. Mutation: re-checking the
// snapshot's deadline after the await denies; dating the answer from the
// snapshot load expires it at once.
#[tokio::test]
async fn lookup_answer_carries_its_own_deadline_past_a_stale_snapshot() {
    let f = fixture(&["a"]);
    f.resolver.refresh().await.unwrap();
    f.store.add("late");
    f.clock.set(secs(119.9));
    let gate = f.resolver.hooks.arm(MISS_AFTER_READ);
    let pending = tokio::spawn({
        let resolver = Arc::clone(&f.resolver);
        async move { resolver.resolve(&host("late")).await }
    });
    gate.reached().await;
    f.clock.set(secs(120.1));
    gate.release();
    let (_, fresh) = pending.await.unwrap().expect("admitted");
    assert_eq!(fresh.check(), Ok(()));
    f.clock.set(secs(239.8));
    assert_eq!(fresh.check(), Ok(()));
    f.clock.set(secs(239.9));
    assert_eq!(fresh.check(), UNAVAILABLE, "read started at 119.9s");
}

#[tokio::test]
async fn timed_out_refresh_publishes_nothing_and_keeps_the_deadline() {
    let f = fixture_with(
        &["a"],
        ResolverBounds {
            refresh_timeout: Duration::from_millis(20),
            ..ResolverBounds::default()
        },
    );
    f.resolver.refresh().await.unwrap();
    f.store.remove("a");
    f.store.add("b");
    *f.store.load_delay.lock().unwrap() = Duration::from_millis(200);
    f.clock.set(secs(100.0));
    assert_eq!(f.resolver.refresh().await, Err(LoadFailure::Timeout));
    // Retained snapshot unchanged: "a" still resolves without a lookup.
    assert_eq!(f.resolve("a").await, Ok(()));
    assert_eq!(f.store.lookups_for("a"), 0);
    f.clock.set(secs(121.0));
    assert_eq!(
        f.resolve("a").await,
        UNAVAILABLE,
        "the live lookup sees the removal"
    );
}

// Criterion 3 (publication bound): a deadline that passes during the
// synchronous build/publish tail fails the attempt. The manual clock moves
// while tokio time does not, so only the attempt deadline can catch it.
#[tokio::test]
async fn refresh_whose_deadline_passes_before_publication_publishes_nothing() {
    let f = fixture(&["a"]);
    f.resolver.refresh().await.unwrap();
    f.store.remove("a");
    f.clock.set(secs(30.0));
    let gate = f.resolver.hooks.arm(REFRESH_BEFORE_PUBLISH);
    let refresh = tokio::spawn({
        let resolver = Arc::clone(&f.resolver);
        async move { resolver.refresh().await }
    });
    gate.reached().await;
    f.clock.set(secs(40.0));
    gate.release();
    assert_eq!(refresh.await.unwrap(), Err(LoadFailure::Timeout));
    assert_eq!(f.resolve("a").await, Ok(()), "old map retained");
    f.clock.set(secs(120.0));
    assert_eq!(f.resolve("a").await, UNAVAILABLE, "old deadline retained");
    assert_eq!(f.store.lookups_for("a"), 1);
}

// The deadline is rechecked under the publication lock, immediately before
// the first write: an attempt that crosses it while waiting publishes
// nothing. Mutation: checking before taking the lock (or not at all)
// publishes the late snapshot.
#[tokio::test]
async fn refresh_crossing_its_deadline_while_preparing_publishes_nothing() {
    let f = fixture(&["a"]);
    f.resolver.refresh().await.unwrap();
    f.store.remove("a");
    f.clock.set(secs(30.0));
    let clock = Arc::clone(&f.clock);
    f.resolver
        .hooks
        .on_sync(PUBLISH_BEFORE_WRITE, move || clock.set(secs(40.0)));
    assert_eq!(f.resolver.refresh().await, Err(LoadFailure::Timeout));
    assert_eq!(f.resolve("a").await, Ok(()), "old map retained");
    assert_eq!(f.store.lookups_for("a"), 0);
    f.clock.set(secs(120.0));
    assert_eq!(f.resolve("a").await, UNAVAILABLE, "old deadline retained");
    assert_eq!(f.store.lookups_for("a"), 1);
}

// A map build abandoned by a timeout keeps running on its blocking thread;
// until it returns, no new load starts, so at most one cap-sized build is
// ever outstanding. Once released it stops cooperatively, publishes nothing,
// and the next tick loads normally. Mutation: releasing the slot with the
// attempt (or not holding one) starts a second build while the first lives.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timed_out_build_blocks_new_loads_until_it_returns_and_publishes_nothing() {
    let f = fixture_with(
        &["a"],
        ResolverBounds {
            refresh_timeout: Duration::from_millis(50),
            ..ResolverBounds::default()
        },
    );
    f.resolver.refresh().await.unwrap();
    f.store.remove("a");
    f.store.add("b");

    let outstanding = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(AtomicUsize::new(0));
    let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    f.resolver.hooks.on_sync(BUILD_START, {
        let (outstanding, peak, started, release) = (
            Arc::clone(&outstanding),
            Arc::clone(&peak),
            Arc::clone(&started),
            Arc::clone(&release),
        );
        move || {
            started.fetch_add(1, Ordering::SeqCst);
            let now = outstanding.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            let (open, signal) = &*release;
            // Bounded, so a regression fails the test instead of hanging it.
            let _ = signal
                .wait_timeout_while(open.lock().unwrap(), Duration::from_secs(5), |open| !*open)
                .unwrap();
            outstanding.fetch_sub(1, Ordering::SeqCst);
        }
    });

    assert_eq!(f.resolver.refresh().await, Err(LoadFailure::Timeout));
    let loads = f.store.loads.load(Ordering::SeqCst);
    for _ in 0..3 {
        assert_eq!(
            f.resolver.refresh().await,
            Err(LoadFailure::BuildOutstanding)
        );
    }
    assert_eq!(f.store.loads.load(Ordering::SeqCst), loads, "no new load");
    assert_eq!(started.load(Ordering::SeqCst), 1);

    let (open, signal) = &*release;
    *open.lock().unwrap() = true;
    signal.notify_all();
    for _ in 0..200 {
        if !f.resolver.building.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        !f.resolver.building.load(Ordering::SeqCst),
        "build released"
    );
    assert_eq!(f.resolve("a").await, Ok(()), "nothing published late");
    assert_eq!(f.store.lookups_for("a"), 0);

    f.resolver.refresh().await.unwrap();
    assert_eq!(peak.load(Ordering::SeqCst), 1, "one build at a time");
    assert_eq!(started.load(Ordering::SeqCst), 2);
    assert_eq!(f.resolve("b").await, Ok(()));
    assert_eq!(f.store.lookups_for("b"), 0, "new snapshot published");
}

#[test]
fn build_map_stops_in_either_pass_once_told_to() {
    let rows = vec![(host("a"), ISS.to_owned()), (host("b"), ISS.to_owned())];
    assert!(build_map(rows.clone(), || true).is_none(), "grouping pass");
    let checks = AtomicUsize::new(0);
    let stop_in_second_pass = || checks.fetch_add(1, Ordering::SeqCst) >= rows.len();
    assert!(
        build_map(rows.clone(), stop_in_second_pass).is_none(),
        "binding pass"
    );
    assert_eq!(build_map(rows, || false).map(|m| m.len()), Some(2));
}

// The load stops at cap + 1 rows; a loaded snapshot survives an over-cap
// refresh unchanged.
#[tokio::test]
async fn over_cap_refresh_stops_streaming_and_keeps_the_loaded_snapshot() {
    let f = fixture_with(
        &["a", "b"],
        ResolverBounds {
            snapshot_max_rows: 2,
            ..ResolverBounds::default()
        },
    );
    f.resolver.refresh().await.unwrap();
    for label in ["c", "d", "e", "f"] {
        f.store.add(label);
    }
    f.store.remove("b");
    f.clock.set(secs(60.0));
    assert_eq!(f.resolver.refresh().await, Err(LoadFailure::OverCap));
    assert_eq!(f.store.rows_pulled.load(Ordering::SeqCst), 3, "cap + 1");
    assert_eq!(f.resolve("b").await, Ok(()), "old map retained");
    assert_eq!(f.store.lookups_for("b"), 0);
    f.clock.set(secs(120.0));
    assert_eq!(f.resolve("b").await, UNAVAILABLE, "old deadline retained");
}

#[tokio::test]
async fn failed_miss_lookup_denies_and_stores_no_negative_entry() {
    let f = fixture(&["a"]);
    f.resolver.refresh().await.unwrap();
    f.store.lookup_fails.store(true, Ordering::SeqCst);
    assert_eq!(f.resolve("x").await, UNAVAILABLE);
    assert_eq!(f.resolver.negative_len(), 0);
}

// ── Criterion 14: refresh removes bindings; 40s removal bound ───────────────

#[tokio::test]
async fn refresh_removes_unbound_communities_and_held_positive_miss_is_not_stored() {
    let f = fixture(&["a", "b", "control"]);
    f.resolver.refresh().await.unwrap();
    f.store.remove("a");
    f.store.remove("b");
    assert_eq!(f.resolve("a").await, Ok(()), "before refresh");
    f.resolver.refresh().await.unwrap();
    assert_eq!(f.resolve("a").await, UNAVAILABLE);
    assert_eq!(f.resolve("b").await, UNAVAILABLE);
    assert_eq!(f.resolve("control").await, Ok(()));

    // (c) A miss reads "h" active and is held; "h" is removed and a refresh
    // publishes; the held answer is returned once and never stored.
    f.store.add("h");
    let gate = f.resolver.hooks.arm(MISS_AFTER_READ);
    let pending = tokio::spawn({
        let resolver = Arc::clone(&f.resolver);
        async move { resolver.resolve(&host("h")).await.map(|_| ()) }
    });
    gate.reached().await;
    f.resolver.hooks.disarm(MISS_AFTER_READ);
    f.store.remove("h");
    f.resolver.refresh().await.unwrap();
    gate.release();
    assert_eq!(pending.await.unwrap(), Ok(()));
    assert_eq!(f.resolve("h").await, UNAVAILABLE);
}

#[tokio::test(start_paused = true)]
async fn unbinding_after_a_snapshot_is_enforced_within_forty_seconds() {
    // The production scheduler on the tokio clock: refresh at +30s, its
    // read held for just under the 10s limit, published before +40s.
    let store = FakeStore::new(&["a", "gone"]);
    let mut resolver = DatabaseResolver::new(
        Arc::clone(&store) as Arc<dyn BindingSource>,
        policy(),
        vec![ISS.to_owned()],
        ResolverBounds::default(),
    );
    // No jitter: the worst case, a tick a full interval after the last.
    resolver.jitter = || 0.0;
    let resolver = Arc::new(resolver);
    let resolve = |label: &str| {
        let resolver = Arc::clone(&resolver);
        let host = host(label);
        async move { resolver.resolve(&host).await.map(drop) }
    };
    resolver.refresh().await.unwrap();
    store.remove("gone");
    *store.load_delay.lock().unwrap() = Duration::from_millis(9_900);
    let cancel = tokio_util::sync::CancellationToken::new();
    let refresh_loop = tokio::spawn(Arc::clone(&resolver).run_refresh_loop(cancel.clone()));
    tokio::task::yield_now().await; // the loop arms its schedule at t=0

    // Step the paused clock so the loop wakes on its +30s tick, and let its
    // load start and sleep on the 9.9s read.
    tokio::time::advance(REFRESH_INTERVAL).await;
    while store.loads.load(Ordering::SeqCst) < 2 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_millis(9_800)).await;
    assert_eq!(
        resolve("gone").await,
        Ok(()),
        "before the refresh publishes"
    );
    // +39.9s: the held read returns. The map build runs on a real blocking
    // thread the paused clock cannot see, so wait for the publication itself
    // (bounded in real time); the paused clock stays at +39.9s meanwhile.
    tokio::time::advance(Duration::from_millis(100)).await;
    let waited = std::time::Instant::now();
    while resolver
        .published
        .load()
        .as_ref()
        .is_none_or(|published| published.loaded_at < REFRESH_INTERVAL)
    {
        assert!(
            waited.elapsed() < Duration::from_secs(30),
            "never published"
        );
        tokio::task::yield_now().await;
    }
    assert!(resolver.clock.now() < Duration::from_secs(40));
    assert_eq!(resolve("gone").await, UNAVAILABLE, "removed by +40s");
    assert_eq!(resolve("a").await, Ok(()));
    cancel.cancel();
    refresh_loop.await.unwrap();
}

// Mutation: retrying a failure only on the next full interval, never
// backing off, or jittering a delay up instead of down.
#[test]
fn refresh_delay_backs_off_after_failures_and_jitters_down() {
    let delays: Vec<u64> = (0..7).map(|n| next_refresh_delay(n).as_secs()).collect();
    assert_eq!(delays, [30, 2, 4, 8, 16, 30, 30]);
    assert_eq!(next_refresh_delay(u32::MAX), REFRESH_INTERVAL);
    assert_eq!(jittered(REFRESH_INTERVAL, 0.0), REFRESH_INTERVAL);
    assert_eq!(jittered(REFRESH_INTERVAL, 0.5), secs(28.5));
    assert!(jittered(REFRESH_INTERVAL, 0.999_999) > secs(27.0));
}

// The loop refreshes every REFRESH_INTERVAL and retries a failure after
// RETRY_FIRST. Mutation: a different interval, or no retry before the next
// interval, changes the load count at each step.
#[tokio::test(start_paused = true)]
async fn refresh_loop_runs_every_interval_and_retries_failures_sooner() {
    let store = FakeStore::new(&["a"]);
    let mut resolver = DatabaseResolver::new(
        Arc::clone(&store) as Arc<dyn BindingSource>,
        policy(),
        vec![ISS.to_owned()],
        ResolverBounds::default(),
    );
    resolver.jitter = || 0.0;
    let resolver = Arc::new(resolver);
    let cancel = tokio_util::sync::CancellationToken::new();
    let refresh_loop = tokio::spawn(Arc::clone(&resolver).run_refresh_loop(cancel.clone()));
    let loads_after = |step: Duration| {
        let store = Arc::clone(&store);
        async move {
            tokio::time::advance(step).await;
            for _ in 0..100 {
                tokio::task::yield_now().await;
            }
            store.loads.load(Ordering::SeqCst)
        }
    };
    tokio::task::yield_now().await;
    assert_eq!(loads_after(REFRESH_INTERVAL - secs(0.1)).await, 0);
    store.fail_loads.store(true, Ordering::SeqCst);
    assert_eq!(
        loads_after(secs(0.1)).await,
        1,
        "first tick at the interval"
    );
    assert_eq!(loads_after(RETRY_FIRST).await, 2, "failure retried sooner");
    store.fail_loads.store(false, Ordering::SeqCst);
    assert_eq!(loads_after(RETRY_FIRST * 2).await, 3, "backed off, then ok");
    // The successful load's map build runs on a real blocking thread the
    // paused clock cannot see. Wait for its publication (bounded in real
    // time) before moving the clock, or the next advance passes the
    // refresh's timeout and fails it.
    let succeeded_at = REFRESH_INTERVAL + RETRY_FIRST * 3;
    let waited = std::time::Instant::now();
    while resolver
        .published
        .load()
        .as_ref()
        .is_none_or(|published| published.loaded_at < succeeded_at)
    {
        assert!(
            waited.elapsed() < Duration::from_secs(30),
            "never published"
        );
        tokio::task::yield_now().await;
    }
    assert_eq!(loads_after(REFRESH_INTERVAL - secs(0.1)).await, 3);
    assert_eq!(loads_after(secs(0.1)).await, 4, "back on the interval");
    cancel.cancel();
    refresh_loop.await.unwrap();
}

// Two slow successful refreshes in a row, the binding removed just after the
// first one's read began: the second refresh starts one interval after the
// first start, so the removal is published within the documented 40s.
// Mutation: timing the interval from when a refresh finishes starts the
// second one at +69.9s and publishes the removal at +79.8s.
#[tokio::test(start_paused = true)]
async fn removal_during_a_slow_refresh_is_published_within_forty_seconds() {
    let store = FakeStore::new(&["a", "gone"]);
    *store.load_delay.lock().unwrap() = Duration::from_millis(9_900);
    let mut resolver = DatabaseResolver::new(
        Arc::clone(&store) as Arc<dyn BindingSource>,
        policy(),
        vec![ISS.to_owned()],
        ResolverBounds::default(),
    );
    resolver.jitter = || 0.0;
    let resolver = Arc::new(resolver);
    let cancel = tokio_util::sync::CancellationToken::new();
    let refresh_loop = tokio::spawn(Arc::clone(&resolver).run_refresh_loop(cancel.clone()));
    tokio::task::yield_now().await; // the loop arms its schedule at t=0
    let started = tokio::time::Instant::now();
    let published_after = |at: Duration| {
        let resolver = Arc::clone(&resolver);
        async move {
            // The map build runs on a blocking thread the paused clock
            // cannot see; wait for it in bounded real time.
            let waited = std::time::Instant::now();
            while resolver
                .published
                .load()
                .as_ref()
                .is_none_or(|published| published.loaded_at < at)
            {
                assert!(
                    waited.elapsed() < Duration::from_secs(30),
                    "never published"
                );
                tokio::task::yield_now().await;
            }
        }
    };
    let resolve = |label: &str| {
        let resolver = Arc::clone(&resolver);
        let host = host(label);
        async move { resolver.resolve(&host).await.map(drop) }
    };

    // +30s: the first refresh reads, still seeing "gone"; it is removed now.
    tokio::time::advance(REFRESH_INTERVAL).await;
    while store.loads.load(Ordering::SeqCst) < 1 {
        tokio::task::yield_now().await;
    }
    store.remove("gone");
    let removed = started.elapsed();
    tokio::time::advance(Duration::from_millis(9_900)).await;
    published_after(REFRESH_INTERVAL).await;
    assert_eq!(
        resolve("gone").await,
        Ok(()),
        "first snapshot predates removal"
    );

    // The second refresh starts at +60s and publishes at +69.9s.
    tokio::time::sleep_until(started + REFRESH_INTERVAL * 2 + Duration::from_millis(9_900)).await;
    published_after(REFRESH_INTERVAL * 2).await;
    assert!(resolver.clock.now() - removed < Duration::from_secs(40));
    assert_eq!(resolve("gone").await, UNAVAILABLE, "removed within 40s");
    assert_eq!(resolve("a").await, Ok(()));
    cancel.cancel();
    refresh_loop.await.unwrap();
}

// The freshness timestamp is written on every attempt and keeps the last
// good load through failures. Mutation: setting it only on success leaves
// the series absent after a failed first attempt.
#[tokio::test]
async fn freshness_gauge_is_written_on_every_attempt() {
    const GAUGE: &str = "buzz_nip_fi_binding_snapshot_loaded_timestamp_seconds";
    let gauge = |recorder: &metrics_util::debugging::DebuggingRecorder| {
        recorder
            .snapshotter()
            .snapshot()
            .into_vec()
            .into_iter()
            .find(|(key, ..)| key.key().name() == GAUGE)
            .map(|(.., value)| match value {
                metrics_util::debugging::DebugValue::Gauge(v) => v.into_inner(),
                _ => unreachable!(),
            })
    };
    let f = fixture(&["a"]);
    let recorder = metrics_util::debugging::DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    f.store.fail_loads.store(true, Ordering::SeqCst);
    assert!(f.resolver.refresh().await.is_err());
    assert_eq!(
        gauge(&recorder),
        Some(0.0),
        "written by a failed first attempt"
    );
    f.store.fail_loads.store(false, Ordering::SeqCst);
    f.resolver.refresh().await.unwrap();
    let loaded = gauge(&recorder).unwrap();
    assert!(loaded > 0.0);
    f.store.fail_loads.store(true, Ordering::SeqCst);
    assert!(f.resolver.refresh().await.is_err());
    assert_eq!(gauge(&recorder), Some(loaded), "kept through a failure");
}

// ── Creation and the negative cache ────────────────────────────────────────

// A creation on this pod drops the host's cached "not found" (whatever case
// the host arrives in), so the creator's first login looks it up. Mutation:
// not popping the entry denies until the 5s TTL lapses.
#[tokio::test]
async fn record_creation_clears_the_hosts_cached_not_found() {
    let f = fixture(&["a"]);
    f.resolver.refresh().await.unwrap();
    assert_eq!(f.resolve("h").await, UNAVAILABLE);
    f.store.add("h");
    assert_eq!(f.resolve("h").await, UNAVAILABLE, "cached not-found");
    assert_eq!(f.store.lookups_for("h"), 1);
    f.resolver.record_creation(&host("H"));
    assert_eq!(f.resolver.negative_len(), 0);
    assert_eq!(f.resolve("h").await, Ok(()));
    assert_eq!(f.store.lookups_for("h"), 2);
}

// Only a creation or a publication moves the sequence: two unknown hosts
// looked up at once are both cached. Mutation: bumping on a negative store
// leaves the second one uncached.
#[tokio::test]
async fn concurrent_not_found_lookups_are_all_cached() {
    let f = fixture(&["a"]);
    f.resolver.refresh().await.unwrap();
    let gate = f.resolver.hooks.arm(MISS_AFTER_READ);
    let pending: Vec<_> = ["x", "y"]
        .into_iter()
        .map(|label| {
            let resolver = Arc::clone(&f.resolver);
            tokio::spawn(async move { resolver.resolve(&host(label)).await.map(drop) })
        })
        .collect();
    gate.reached().await;
    gate.reached().await;
    gate.release();
    gate.release();
    for task in pending {
        assert_eq!(task.await.unwrap(), UNAVAILABLE);
    }
    assert_eq!(f.resolver.negative_len(), 2);
}

#[tokio::test]
async fn negative_lookup_racing_a_creation_or_swap_stores_nothing() {
    // (d), and criterion 7(f).
    for swap in [false, true] {
        let f = fixture(&["a"]);
        f.resolver.refresh().await.unwrap();
        let gate = f.resolver.hooks.arm(MISS_AFTER_READ);
        let pending = tokio::spawn({
            let resolver = Arc::clone(&f.resolver);
            async move { resolver.resolve(&host("h")).await.map(|_| ()) }
        });
        gate.reached().await;
        f.resolver.hooks.disarm(MISS_AFTER_READ);
        f.store.add("h");
        if swap {
            f.resolver.refresh().await.unwrap();
        } else {
            f.resolver.record_creation(&host("h"));
        }
        gate.release();
        assert_eq!(pending.await.unwrap(), UNAVAILABLE, "answered as read");
        assert_eq!(f.resolver.negative_len(), 0, "swap={swap}");
        assert_eq!(f.resolve("h").await, Ok(()), "swap={swap}");
    }
}

// The provisioning handler drops the cached "not found" only for a creation
// that bound issuers. Mutation: skipping the resolver call in the handler, or
// calling it for an unbound creation, fails one of the two halves.
#[tokio::test]
async fn provisioning_a_bound_community_clears_its_cached_not_found() {
    use crate::handlers::community_provisioning::record_binding_creation;
    let f = fixture(&["a"]);
    f.resolver.refresh().await.unwrap();
    assert_eq!(f.resolve("h").await, UNAVAILABLE);
    f.store.add("h");
    record_binding_creation(Some(&f.resolver), &host("h"), &[]);
    assert_eq!(
        f.resolve("h").await,
        UNAVAILABLE,
        "unbound creation keeps it"
    );
    record_binding_creation(Some(&f.resolver), &host("h"), &[ISS.to_owned()]);
    assert_eq!(f.resolve("h").await, Ok(()));
    assert_eq!(f.store.lookups_for("h"), 2);
}

// One lookup admits at most `max_waiters` admissions; the next is rejected
// rather than queued. Mutation: ignoring the cap lets the third wait.
#[tokio::test(start_paused = true)]
async fn waiters_beyond_the_cap_are_rejected() {
    let f = fixture_with(
        &["a"],
        ResolverBounds {
            max_waiters: 2,
            ..ResolverBounds::default()
        },
    );
    *f.store.lookup_delay.lock().unwrap() = secs(0.5);
    let waiters: Vec<_> = (0..2)
        .map(|_| {
            let resolver = Arc::clone(&f.resolver);
            tokio::spawn(async move { resolver.resolve(&host("a")).await.map(drop) })
        })
        .collect();
    tokio::time::sleep(secs(0.1)).await;
    assert_eq!(f.resolver.waiters_now(), 2);
    assert_eq!(f.resolve("a").await, UNAVAILABLE);
    for w in waiters {
        assert_eq!(w.await.unwrap(), Ok(()));
    }
    assert_eq!(f.store.lookups_for("a"), 1);
}

// ── Criterion 9: miss-path bounds under a flood ─────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn flood_of_unknown_hosts_respects_every_cap_and_spares_snapshot_hosts() {
    let bounds = ResolverBounds {
        max_inflight: 4,
        max_pending_hosts: 8,
        max_waiters: 16,
        negative_cache_max: 32,
        permit_wait: Duration::from_millis(5),
        lookup_timeout: Duration::from_millis(500),
        ..ResolverBounds::default()
    };
    let f = fixture_with(&["a", "b"], bounds.clone());
    f.resolver.refresh().await.unwrap();
    *f.store.lookup_delay.lock().unwrap() = Duration::from_millis(20);

    let peak = Arc::new(Mutex::new((0usize, 0usize, 0usize, 0usize)));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let monitor = tokio::spawn({
        let (resolver, peak, stop) = (
            Arc::clone(&f.resolver),
            Arc::clone(&peak),
            Arc::clone(&stop),
        );
        async move {
            while !stop.load(Ordering::SeqCst) {
                {
                    let mut p = peak.lock().unwrap();
                    p.0 = p.0.max(resolver.inflight_now());
                    p.1 = p.1.max(resolver.pending_len());
                    p.2 = p.2.max(resolver.waiters_now());
                    p.3 = p.3.max(resolver.negative_len());
                }
                tokio::task::yield_now().await;
            }
        }
    });
    let flood: Vec<_> = (0..400)
        .map(|i| {
            let resolver = Arc::clone(&f.resolver);
            tokio::spawn(async move { resolver.resolve(&host(&format!("rand{i}"))).await.is_err() })
        })
        .collect();
    for _ in 0..50 {
        assert_eq!(f.resolve("a").await, Ok(()));
        assert_eq!(f.resolve("b").await, Ok(()));
    }
    for task in flood {
        assert!(task.await.unwrap(), "every made-up host is denied");
    }
    stop.store(true, Ordering::SeqCst);
    monitor.await.unwrap();
    let p = *peak.lock().unwrap();
    assert!(p.0 <= bounds.max_inflight, "inflight {}", p.0);
    assert!(p.1 <= bounds.max_pending_hosts, "pending {}", p.1);
    assert!(p.2 <= bounds.max_waiters, "waiters {}", p.2);
    assert!(p.3 <= bounds.negative_cache_max, "negative {}", p.3);
    assert_eq!(f.store.lookups_for("a") + f.store.lookups_for("b"), 0);
}

// ── Criterion 17: map lifetime ──────────────────────────────────────────────

#[tokio::test]
async fn old_snapshot_maps_are_dropped_while_admissions_keep_their_bindings() {
    let f = fixture(&["a"]);
    f.resolver.refresh().await.unwrap();
    let mut held = Vec::new();
    let mut weak_maps = Vec::new();
    for _ in 0..10 {
        held.push(f.resolver.resolve(&host("a")).await.unwrap());
        weak_maps.push(Arc::downgrade(
            f.resolver.published.load_full().as_ref().unwrap(),
        ));
        f.resolver.refresh().await.unwrap();
    }
    // Replaced maps are freed on a blocking thread; wait for it (bounded).
    let waited = std::time::Instant::now();
    while weak_maps.iter().any(|w| w.strong_count() > 0) {
        assert!(
            waited.elapsed() < Duration::from_secs(30),
            "only the published map is alive"
        );
        tokio::task::yield_now().await;
    }
    assert!(held
        .iter()
        .all(|b| b.0.expected_aud() == "https://a.communities.buzz.xyz"));
}

// ── Pre-filter ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn non_suffix_reserved_and_multi_label_hosts_never_reach_the_store() {
    let f = fixture(&["a"]);
    f.resolver.refresh().await.unwrap();
    for raw in [
        "communities.buzz.xyz",
        "a.b.communities.buzz.xyz",
        "evil.example",
        "a.communities.buzz.xyz:8443",
    ] {
        assert!(f.resolver.resolve(raw).await.is_err(), "{raw}");
    }
    assert_eq!(f.store.total_lookups(), 0);
}

/// Unset or blank variables take the defaults, a set value overrides only
/// its own bound, out-of-range or non-numeric values fail startup, and
/// `max_inflight` stays below the writer pool. Mutation: dropping the clamp
/// lets pool 4 run 8 lookups at once.
#[test]
fn resolver_bounds_from_env_defaults_overrides_clamps_and_rejects() {
    const VARS: [&str; 4] = [
        "BUZZ_NIP_FI_RESOLVER_NEGATIVE_CACHE_MAX",
        "BUZZ_NIP_FI_RESOLVER_MAX_INFLIGHT",
        "BUZZ_NIP_FI_SNAPSHOT_MAX_ROWS",
        "BUZZ_NIP_FI_SNAPSHOT_REFRESH_TIMEOUT_MS",
    ];
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            for v in VARS {
                std::env::remove_var(v);
            }
        }
    }
    let _lock = crate::nip_fi_config::NIP_FI_ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _clear = Clear;
    for v in VARS {
        std::env::remove_var(v);
    }

    assert_eq!(
        ResolverBounds::from_env(50).unwrap(),
        ResolverBounds::default()
    );
    assert_eq!(ResolverBounds::from_env(4).unwrap().max_inflight, 3);
    assert_eq!(ResolverBounds::from_env(1).unwrap().max_inflight, 1);

    std::env::set_var("BUZZ_NIP_FI_SNAPSHOT_MAX_ROWS", "250");
    std::env::set_var("BUZZ_NIP_FI_SNAPSHOT_REFRESH_TIMEOUT_MS", "29999");
    std::env::set_var("BUZZ_NIP_FI_RESOLVER_MAX_INFLIGHT", " ");
    let bounds = ResolverBounds::from_env(50).unwrap();
    assert_eq!(bounds.snapshot_max_rows, 250);
    assert_eq!(bounds.refresh_timeout, Duration::from_millis(29_999));
    assert_eq!(bounds.max_inflight, ResolverBounds::default().max_inflight);
    std::env::set_var("BUZZ_NIP_FI_RESOLVER_MAX_INFLIGHT", "40");
    assert_eq!(ResolverBounds::from_env(50).unwrap().max_inflight, 40);
    assert_eq!(ResolverBounds::from_env(10).unwrap().max_inflight, 9);

    for (var, bad) in [
        ("BUZZ_NIP_FI_SNAPSHOT_REFRESH_TIMEOUT_MS", "30000"),
        ("BUZZ_NIP_FI_RESOLVER_MAX_INFLIGHT", "0"),
        ("BUZZ_NIP_FI_SNAPSHOT_MAX_ROWS", "lots"),
        ("BUZZ_NIP_FI_SNAPSHOT_MAX_ROWS", "10000001"),
        ("BUZZ_NIP_FI_RESOLVER_NEGATIVE_CACHE_MAX", "1000001"),
    ] {
        let prior = std::env::var(var).ok();
        std::env::set_var(var, bad);
        assert!(
            ResolverBounds::from_env(50).is_err(),
            "{var}={bad} must fail startup"
        );
        match prior {
            Some(p) => std::env::set_var(var, p),
            None => std::env::remove_var(var),
        }
    }
}

mod router_tests;

// A database-source community is labelled `bound` in shadow, never by its
// host, whose set is unbounded. Mutation: labelling every resolved community
// by its URI, as the `env` source is, names the host.
#[tokio::test]
async fn database_source_shadow_label_never_names_the_host() {
    let f = fixture(&["a"]);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(axum::http::header::HOST, host("a").parse().unwrap());
    let community = crate::nip_fi_core::Communities::Database(&f.resolver)
        .resolve(&headers)
        .await
        .expect("bound");
    assert_eq!(
        community.shadow_label(),
        metrics::SharedString::from("bound")
    );
}

// Every step of one request reuses the router guard's resolution, a failed
// one included, so shadow resolves the Host once. Mutation: reusing only a
// successful resolution sends the denied host back to the live lookup.
#[tokio::test]
async fn admission_reuses_the_guards_resolution_without_a_lookup() {
    let f = fixture(&["a"]);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(axum::http::header::HOST, host("a").parse().unwrap());
    let communities = crate::nip_fi_core::Communities::Database(&f.resolver);
    let denied = Err(DenialClass::AuthorizationUnavailable);
    let reused = crate::nip_fi_http::with_resolved_community(denied, async {
        crate::nip_fi_http::resolve_for_admission(communities, &headers).await
    })
    .await;
    assert_eq!(reused.map(|_| ()), UNAVAILABLE);
    assert_eq!(f.store.lookups_for("a"), 0);
}
