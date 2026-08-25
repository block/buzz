//! Store-level scale evidence for the Google Cloud Storage provider.
//!
//! These arms answer the questions the functional suite cannot: whether the
//! provider's behaviour holds when many writers run at once, whether the
//! compare-and-swap contract still admits exactly one writer per round when
//! that round is repeated dozens of times, and what latency and outcome mix a
//! sustained load actually produces.
//!
//! They cost minutes and thousands of requests, so they are gated twice —
//! `BUZZ_GCS_LIVE=1` like the functional suite, plus `BUZZ_GCS_SCALE=1`:
//!
//! ```bash
//! BUZZ_GCS_LIVE=1 BUZZ_GCS_SCALE=1 \
//! BUZZ_GCS_TEST_BUCKET=my-disposable-bucket \
//!   cargo test -p buzz-object-store --test gcs_scale -- --ignored --nocapture
//! ```
//!
//! Every knob has an environment override so a rehearsal can widen a run
//! without editing the source; the defaults are what a routine run should cost.
//!
//! ## What these arms measure, and what they do not
//!
//! They are *store-level* evidence: pointer objects and blobs through the
//! object-store seam. They are not a repository-shaped corpus — no packs of
//! realistic size, no ref counts, no cold materialisation — and they should not
//! be read as one.
//!
//! Throttling deserves one note. The provider absorbs 429 inside its own
//! bounded policy, so a caller running the production configuration sees almost
//! none; a caller-visible count near zero here is evidence that pacing works,
//! not that the service never throttled. The functional suite's pacing arm
//! drives the same key through a single-attempt client precisely so the 429s
//! become visible and countable.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::FutureExt;

use buzz_object_store::{
    ConditionalWrite, GcsObjectStore, GcsStoreConfig, ObjectStore, ObjectStoreError, Revision,
    WriteCondition,
};

const CONTENT_TYPE: &str = "application/octet-stream";

/// How many pointer creates the distinct-name arm issues at once.
const DEFAULT_DISTINCT_WRITERS: usize = 500;
/// How many hot-pointer contention rounds to run.
const DEFAULT_CONTENTION_ROUNDS: usize = 25;
/// How many writers race for the same pointer in one round.
const DEFAULT_CONTENTION_WIDTH: usize = 3;
/// How long the sustained mixed-load arm runs.
const DEFAULT_MIXED_LOAD_SECONDS: u64 = 120;
/// How many independent workers the mixed-load arm runs.
const DEFAULT_MIXED_LOAD_WORKERS: usize = 8;
/// How long each mixed-load worker waits between operations.
const DEFAULT_MIXED_LOAD_INTERVAL_MS: u64 = 250;

/// Just past Cloud Storage's documented one-write-per-second per-object
/// ceiling, so a contention round measures the conditional-write semantics
/// rather than the rate limiter.
const SAME_KEY_SPACING: Duration = Duration::from_millis(1_100);

/// Read a `usize` knob from the environment.
fn knob(variable: &str, default: usize) -> usize {
    std::env::var(variable)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// Connect to the test bucket, or `None` when this environment has not opted
/// in to the scale arms.
async fn scale_store(test: &str) -> Option<(Arc<GcsObjectStore>, String)> {
    // The relay installs this in `main` before any TLS request. These tests are
    // their own process and must do the same: both ring and aws-lc-rs are in
    // the build graph, so rustls refuses to pick one on its own.
    static PROVIDER: std::sync::Once = std::sync::Once::new();
    PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });

    for (variable, value) in [("BUZZ_GCS_LIVE", "1"), ("BUZZ_GCS_SCALE", "1")] {
        if std::env::var(variable).as_deref() != Ok(value) {
            eprintln!("skipping {test}: set {variable}={value} to run the scale arms");
            return None;
        }
    }
    let bucket = match std::env::var("BUZZ_GCS_TEST_BUCKET") {
        Ok(bucket) if !bucket.is_empty() => bucket,
        _ => panic!("BUZZ_GCS_SCALE=1 requires BUZZ_GCS_TEST_BUCKET"),
    };

    let store = Arc::new(
        GcsObjectStore::connect(&GcsStoreConfig::new(bucket))
            .await
            .expect("connect to the test bucket"),
    );
    Some((store, format!("a4-scale/{}/{test}", uuid::Uuid::new_v4())))
}

/// Run one scale arm, then remove everything it wrote whether it passed or not.
///
/// These arms write hundreds to thousands of objects, so a leak on the failure
/// path is not a rounding error — and a failing arm is the one that gets re-run.
async fn with_prefix<F>(test: &str, body: F)
where
    F: AsyncFnOnce(&Arc<GcsObjectStore>, &str),
{
    let Some((store, prefix)) = scale_store(test).await else {
        return;
    };
    let outcome = AssertUnwindSafe(body(&store, &prefix)).catch_unwind().await;

    let mut token = None;
    let mut keys = Vec::new();
    loop {
        let page = store
            .list_page(&prefix, token, 1000)
            .await
            .expect("list for cleanup");
        keys.extend(page.objects.into_iter().map(|(key, _)| key));
        token = page.next_continuation_token;
        if token.is_none() {
            break;
        }
    }
    let removed = keys.len();
    let cleanup = if keys.is_empty() {
        None
    } else {
        Some(store.delete_objects(&keys).await)
    };

    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
    if let Some(cleanup) = cleanup {
        let cleanup = cleanup.expect("bulk delete for cleanup");
        assert!(
            cleanup.failed.is_empty(),
            "cleanup left objects behind: {:?}",
            cleanup.failed
        );
    }
    let leaked = store
        .list_page(&prefix, None, 10)
        .await
        .expect("verify the namespace is empty");
    assert!(
        leaked.objects.is_empty(),
        "the namespace must be empty after cleanup: {:?}",
        leaked.objects
    );
    eprintln!("{test}: removed {removed} objects, namespace verified empty");
}

/// Latency samples for one class of operation.
#[derive(Default)]
struct Latencies(Vec<Duration>);

impl Latencies {
    fn record(&mut self, elapsed: Duration) {
        self.0.push(elapsed);
    }

    /// The sample at `fraction` through the sorted distribution.
    ///
    /// Nearest-rank, so a small sample reports a real observation rather than
    /// an interpolation between two of them.
    fn percentile(&mut self, fraction: f64) -> Duration {
        if self.0.is_empty() {
            return Duration::ZERO;
        }
        self.0.sort_unstable();
        let rank = ((self.0.len() as f64) * fraction).ceil() as usize;
        self.0[rank.clamp(1, self.0.len()) - 1]
    }

    fn summary(&mut self, label: &str) -> String {
        let count = self.0.len();
        let p50 = self.percentile(0.50);
        let p95 = self.percentile(0.95);
        let p99 = self.percentile(0.99);
        format!("{label}: n={count} p50={p50:?} p95={p95:?} p99={p99:?}")
    }
}

/// How one attempt was answered, folded across a whole run.
#[derive(Default, Debug, PartialEq, Eq)]
struct Outcomes {
    committed: u64,
    conflicts: u64,
    reads: u64,
    lists: u64,
    throttled: u64,
    retryable: u64,
    ambiguous: u64,
    terminal: u64,
}

impl Outcomes {
    /// Fold a provider error into the counter its classification names.
    fn record_error(&mut self, error: &ObjectStoreError) {
        match error {
            ObjectStoreError::Throttled { .. } => self.throttled += 1,
            ObjectStoreError::TransportRetryable { .. } => self.retryable += 1,
            ObjectStoreError::TransportAmbiguous { .. } => self.ambiguous += 1,
            _ => self.terminal += 1,
        }
    }
}

/// Concurrent creates across distinct object names: the axis Cloud Storage
/// scales horizontally on.
///
/// The per-object write ceiling applies to one object name. Independent
/// repositories publish to independent pointers, so this is the shape that has
/// to scale — and every writer must commit, because none of them is contending
/// with another.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1, BUZZ_GCS_SCALE=1)"]
async fn concurrent_creates_across_distinct_names_all_commit() {
    let writers = knob("BUZZ_GCS_SCALE_DISTINCT_WRITERS", DEFAULT_DISTINCT_WRITERS);

    with_prefix("distinct-names", async |store, prefix| {
        let started = Instant::now();
        let mut tasks = Vec::with_capacity(writers);
        for i in 0..writers {
            let store = Arc::clone(store);
            let key = format!("{prefix}/pointers/{i:05}");
            tasks.push(tokio::spawn(async move {
                let attempt = Instant::now();
                let outcome = store
                    .put_conditional(
                        &key,
                        format!("writer-{i}").as_bytes(),
                        CONTENT_TYPE,
                        WriteCondition::Absent,
                    )
                    .await;
                (outcome, attempt.elapsed())
            }));
        }

        let mut outcomes = Outcomes::default();
        let mut latencies = Latencies::default();
        let mut revisions = Vec::with_capacity(writers);
        for task in tasks {
            let (outcome, elapsed) = task.await.expect("writer task");
            latencies.record(elapsed);
            match outcome {
                Ok(ConditionalWrite::Committed(revision)) => {
                    outcomes.committed += 1;
                    revisions.push(revision);
                }
                Ok(ConditionalWrite::Conflict) => outcomes.conflicts += 1,
                Err(error) => outcomes.record_error(&error),
            }
        }
        let elapsed = started.elapsed();

        assert_eq!(
            outcomes.committed, writers as u64,
            "every writer addressed its own object name, so every one must commit: {outcomes:?}"
        );
        assert_eq!(
            revisions.len(),
            writers,
            "a commit without a revision leaves the caller nothing to predicate its next write on"
        );

        eprintln!(
            "distinct names: {writers} concurrent creates in {elapsed:?} ({:.0}/sec), {}",
            writers as f64 / elapsed.as_secs_f64(),
            latencies.summary("create")
        );
        eprintln!("distinct names: outcomes {outcomes:?}");
    })
    .await;
}

/// The hot-pointer race, repeated: exactly one winner per round, every round,
/// and the published state is always the winner's.
///
/// One round proves the contract holds; repeating it is what would surface a
/// rare double commit or a lost update. The second assertion is the one that
/// catches a silent lost update — a round can report one winner and still have
/// published a loser's body if the backend committed out of order.
///
/// Rounds are spaced past the published per-object ceiling on purpose: this arm
/// is measuring conditional-write semantics under contention, and a deliberately
/// over-rate round would only measure the rate limiter.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1, BUZZ_GCS_SCALE=1)"]
async fn a_repeated_hot_pointer_race_never_admits_two_winners() {
    let rounds = knob(
        "BUZZ_GCS_SCALE_CONTENTION_ROUNDS",
        DEFAULT_CONTENTION_ROUNDS,
    );
    let width = knob("BUZZ_GCS_SCALE_CONTENTION_WIDTH", DEFAULT_CONTENTION_WIDTH);
    assert!(width >= 2, "a race needs at least two entrants");

    with_prefix("hot-pointer", async |store, prefix| {
        let key = format!("{prefix}/pointers/hot");

        let ConditionalWrite::Committed(mut revision) = store
            .put_conditional(&key, b"round-0", CONTENT_TYPE, WriteCondition::Absent)
            .await
            .expect("create the pointer")
        else {
            panic!("creating an absent pointer must commit");
        };

        let mut outcomes = Outcomes::default();
        let mut latencies = Latencies::default();
        let mut last_round_end = Instant::now();
        let started = Instant::now();

        for round in 1..=rounds {
            // Space same-key rounds past the documented ceiling.
            let since = last_round_end.elapsed();
            if since < SAME_KEY_SPACING {
                tokio::time::sleep(SAME_KEY_SPACING - since).await;
            }

            let mut racers = Vec::with_capacity(width);
            for racer in 0..width {
                let store = Arc::clone(store);
                let key = key.clone();
                let revision = revision.clone();
                let body = format!("round-{round}-racer-{racer}");
                racers.push(tokio::spawn(async move {
                    let attempt = Instant::now();
                    let outcome = store
                        .put_conditional(
                            &key,
                            body.as_bytes(),
                            CONTENT_TYPE,
                            WriteCondition::Matches(revision),
                        )
                        .await;
                    (body, outcome, attempt.elapsed())
                }));
            }

            let mut winners: Vec<(String, Revision)> = Vec::new();
            for racer in racers {
                let (body, outcome, elapsed) = racer.await.expect("racer task");
                latencies.record(elapsed);
                match outcome {
                    Ok(ConditionalWrite::Committed(next)) => {
                        outcomes.committed += 1;
                        winners.push((body, next));
                    }
                    Ok(ConditionalWrite::Conflict) => outcomes.conflicts += 1,
                    Err(error) => {
                        // Throttling is a refusal to evaluate the precondition,
                        // so it is never evidence of a lost race.
                        outcomes.record_error(&error);
                    }
                }
            }
            last_round_end = Instant::now();

            assert_eq!(
                winners.len(),
                1,
                "round {round} admitted {} winners on one revision — two committed writers on the \
                 same precondition is a lost update announcing itself",
                winners.len()
            );
            let (winning_body, winning_revision) = winners.pop().expect("exactly one winner");
            assert_ne!(
                winning_revision, revision,
                "round {round} committed without minting a new revision"
            );

            // The published state must be the winner's, not a loser's: one
            // acknowledged winner over somebody else's bytes is exactly the
            // silent lost update this arm exists to rule out.
            let (published_revision, published) = store
                .get_with_revision(&key)
                .await
                .expect("read the published state")
                .expect("the pointer exists");
            assert_eq!(
                published.as_ref(),
                winning_body.as_bytes(),
                "round {round} published a body no acknowledged winner wrote"
            );
            assert_eq!(
                published_revision, winning_revision,
                "round {round} published a revision the winner did not commit"
            );
            revision = winning_revision;
        }
        let elapsed = started.elapsed();

        assert_eq!(
            outcomes.committed, rounds as u64,
            "exactly one winner per round: {outcomes:?}"
        );

        eprintln!(
            "hot pointer: {rounds} rounds of width {width} in {elapsed:?}, {}",
            latencies.summary("cas")
        );
        eprintln!("hot pointer: outcomes {outcomes:?}");
    })
    .await;
}

/// A sustained mixed read/write/list load at a bounded rate.
///
/// Each worker owns its own pointer, which is the deployment's real shape:
/// independent repositories publish to independent object names, so a worker's
/// compare-and-swap chain is sequential on its own object and stays inside the
/// per-object ceiling while the fleet as a whole is concurrent.
///
/// This arm is a measurement, not a threshold. It asserts only what must never
/// happen — a writer losing a race it was the only entrant in, or a terminal
/// error — and reports the rest for the record.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1, BUZZ_GCS_SCALE=1)"]
async fn sustained_mixed_load_reports_its_latency_and_outcome_mix() {
    let seconds = knob(
        "BUZZ_GCS_SCALE_MIXED_SECONDS",
        DEFAULT_MIXED_LOAD_SECONDS as usize,
    ) as u64;
    let workers = knob("BUZZ_GCS_SCALE_MIXED_WORKERS", DEFAULT_MIXED_LOAD_WORKERS);
    let interval = Duration::from_millis(knob(
        "BUZZ_GCS_SCALE_MIXED_INTERVAL_MS",
        DEFAULT_MIXED_LOAD_INTERVAL_MS as usize,
    ) as u64);

    with_prefix("mixed-load", async |store, prefix| {
        let deadline = Instant::now() + Duration::from_secs(seconds);

        let mut tasks = Vec::with_capacity(workers);
        for worker in 0..workers {
            let store = Arc::clone(store);
            let pointer = format!("{prefix}/pointers/worker-{worker:03}");
            let blob_prefix = format!("{prefix}/blobs/worker-{worker:03}");
            tasks.push(tokio::spawn(async move {
                let mut outcomes = Outcomes::default();
                let mut latencies: HashMap<&'static str, Latencies> = HashMap::new();

                let ConditionalWrite::Committed(mut revision) = store
                    .put_conditional(&pointer, b"seed", CONTENT_TYPE, WriteCondition::Absent)
                    .await
                    .expect("seed the worker's pointer")
                else {
                    panic!("creating an absent pointer must commit");
                };
                outcomes.committed += 1;

                let mut step = 0u64;
                while Instant::now() < deadline {
                    step += 1;
                    // 4 reads : 2 writes : 1 list. Reads dominate because
                    // serving a repository resolves its pointer per request.
                    let attempt = Instant::now();
                    match step % 7 {
                        0 => {
                            let outcome = store.list_page(&blob_prefix, None, 100).await;
                            latencies
                                .entry("list")
                                .or_default()
                                .record(attempt.elapsed());
                            match outcome {
                                Ok(_) => outcomes.lists += 1,
                                Err(error) => outcomes.record_error(&error),
                            }
                        }
                        1 | 3 => {
                            // An immutable blob write, then a pointer swap
                            // publishing it — the two writes a push performs.
                            let blob = format!("{blob_prefix}/{step:06}");
                            let outcome = store
                                .put_immutable(
                                    &blob,
                                    format!("blob-{step}").as_bytes(),
                                    CONTENT_TYPE,
                                )
                                .await;
                            latencies
                                .entry("write")
                                .or_default()
                                .record(attempt.elapsed());
                            match outcome {
                                Ok(_) => outcomes.committed += 1,
                                Err(error) => outcomes.record_error(&error),
                            }

                            let swap = Instant::now();
                            let outcome = store
                                .put_conditional(
                                    &pointer,
                                    format!("step-{step}").as_bytes(),
                                    CONTENT_TYPE,
                                    WriteCondition::Matches(revision.clone()),
                                )
                                .await;
                            latencies.entry("cas").or_default().record(swap.elapsed());
                            match outcome {
                                Ok(ConditionalWrite::Committed(next)) => {
                                    outcomes.committed += 1;
                                    revision = next;
                                }
                                Ok(ConditionalWrite::Conflict) => outcomes.conflicts += 1,
                                Err(error) => outcomes.record_error(&error),
                            }
                        }
                        _ => {
                            let outcome = store.get_with_revision(&pointer).await;
                            latencies
                                .entry("read")
                                .or_default()
                                .record(attempt.elapsed());
                            match outcome {
                                Ok(Some(_)) => outcomes.reads += 1,
                                Ok(None) => panic!("the worker's own pointer disappeared"),
                                Err(error) => outcomes.record_error(&error),
                            }
                        }
                    }

                    let spent = attempt.elapsed();
                    if spent < interval {
                        tokio::time::sleep(interval - spent).await;
                    }
                }

                (outcomes, latencies)
            }));
        }

        let started = Instant::now();
        let mut totals = Outcomes::default();
        let mut latencies: HashMap<&'static str, Latencies> = HashMap::new();
        for task in tasks {
            let (outcomes, worker_latencies) = task.await.expect("mixed-load worker");
            totals.committed += outcomes.committed;
            totals.conflicts += outcomes.conflicts;
            totals.reads += outcomes.reads;
            totals.lists += outcomes.lists;
            totals.throttled += outcomes.throttled;
            totals.retryable += outcomes.retryable;
            totals.ambiguous += outcomes.ambiguous;
            totals.terminal += outcomes.terminal;
            for (class, samples) in worker_latencies {
                latencies.entry(class).or_default().0.extend(samples.0);
            }
        }
        let elapsed = started.elapsed();

        assert_eq!(
            totals.conflicts, 0,
            "every worker owns its own pointer, so no worker can lose a race: {totals:?}"
        );
        assert_eq!(
            totals.terminal, 0,
            "a terminal error under nominal load is a failure, not a measurement: {totals:?}"
        );

        let operations = totals.committed + totals.reads + totals.lists;
        eprintln!(
            "mixed load: {workers} workers for {elapsed:?}, {operations} operations \
             ({:.1}/sec), outcomes {totals:?}",
            operations as f64 / elapsed.as_secs_f64()
        );
        let mut classes: Vec<&'static str> = latencies.keys().copied().collect();
        classes.sort_unstable();
        for class in classes {
            eprintln!(
                "mixed load: {}",
                latencies.get_mut(class).expect("class").summary(class)
            );
        }
    })
    .await;
}
