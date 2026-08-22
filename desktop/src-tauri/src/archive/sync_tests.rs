//! Tests for the native archive sync loop.
//!
//! The loop is driven through the real `run_sync` body with a fake
//! [`ArchiveSyncIo`] and a real event channel, so batching, demultiplexing,
//! reload coalescing, and shutdown flush are exercised as the production task
//! runs them — not as a struct poked directly.

use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag};
use std::sync::Mutex as StdMutex;

// ── Test doubles ─────────────────────────────────────────────────────────────

#[derive(Default)]
struct FakeIo {
    /// Successive results for `list_subscriptions`; the last one repeats so a
    /// reload that outruns the script does not panic.
    listings: StdMutex<Vec<Vec<SaveSubscription>>>,
    applied: StdMutex<Vec<Vec<Subscription>>>,
    batches: StdMutex<Vec<Vec<ArchiveCandidate>>>,
    /// What `archive` returns; drives the notify-on-metrics assertion.
    persisted_agent_metrics: StdMutex<u32>,
    archive_fails: StdMutex<bool>,
    metrics_notifications: StdMutex<u32>,
}

impl FakeIo {
    fn with_listings(listings: Vec<Vec<SaveSubscription>>) -> Self {
        Self {
            listings: StdMutex::new(listings),
            ..Default::default()
        }
    }

    fn applied(&self) -> Vec<Vec<Subscription>> {
        self.applied.lock().unwrap().clone()
    }

    /// Flattened candidates in delivery order, as `(scope_value, event_id)`.
    fn archived(&self) -> Vec<Vec<String>> {
        self.batches
            .lock()
            .unwrap()
            .iter()
            .map(|batch| {
                batch
                    .iter()
                    .map(|c| c.matched_scope.scope_value.clone())
                    .collect()
            })
            .collect()
    }
}

impl ArchiveSyncIo for FakeIo {
    fn list_subscriptions(&self) -> BoxFuture<'_, Result<Vec<SaveSubscription>, String>> {
        Box::pin(async move {
            let mut listings = self.listings.lock().unwrap();
            if listings.is_empty() {
                return Ok(Vec::new());
            }
            if listings.len() == 1 {
                return Ok(listings[0].clone());
            }
            Ok(listings.remove(0))
        })
    }

    fn set_subscriptions(&self, subscriptions: Vec<Subscription>) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.applied.lock().unwrap().push(subscriptions);
        })
    }

    fn archive(
        &self,
        candidates: Vec<ArchiveCandidate>,
    ) -> BoxFuture<'_, Result<ArchiveBatchResult, String>> {
        Box::pin(async move {
            self.batches.lock().unwrap().push(candidates);
            if *self.archive_fails.lock().unwrap() {
                return Err("archive failed".to_string());
            }
            Ok(ArchiveBatchResult {
                persisted: 0,
                persisted_agent_metrics: *self.persisted_agent_metrics.lock().unwrap(),
                dropped: 0,
            })
        })
    }

    fn notify_agent_metrics_changed(&self) {
        *self.metrics_notifications.lock().unwrap() += 1;
    }
}

/// Yields until `condition` holds, then returns; fails the test if it never
/// does. An unbounded spin turns a broken flush into a HUNG test instead of a
/// failing one — and under a paused clock it also starves tokio's auto-advance,
/// so the deadline that would have masked the bug never even fires.
async fn wait_for(label: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..10_000 {
        if condition() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("timed out waiting for {label}");
}

fn saved(scope_type: &str, scope_value: &str, kinds: &str) -> SaveSubscription {
    SaveSubscription {
        identity_pubkey: "owner".into(),
        relay_url: "wss://relay.test".into(),
        scope_type: scope_type.into(),
        scope_value: scope_value.into(),
        kinds: kinds.into(),
        created_at: 0,
    }
}

fn matched(subscription_id: &str) -> MatchedEvent {
    let event = EventBuilder::new(Kind::Custom(9), "hello")
        .tags([Tag::parse(vec!["h", "channel-a"]).unwrap()])
        .sign_with_keys(&Keys::generate())
        .unwrap();
    MatchedEvent {
        subscription_id: subscription_id.to_string(),
        event: Box::new(event),
    }
}

/// Runs `run_sync` on a task, handing back the controls the tests drive it
/// with. Every test cancels and joins, so a loop that fails to observe
/// cancellation hangs the test rather than passing silently.
fn spawn_sync(
    io: Arc<FakeIo>,
) -> (
    mpsc::Sender<MatchedEvent>,
    Arc<Notify>,
    CancellationToken,
    tokio::task::JoinHandle<()>,
) {
    let (tx, rx) = mpsc::channel(64);
    let reload = Arc::new(Notify::new());
    let cancel = CancellationToken::new();
    let handle = {
        let io = Arc::clone(&io);
        let reload = Arc::clone(&reload);
        let cancel = cancel.clone();
        tokio::spawn(async move { run_sync(io.as_ref(), reload, rx, cancel).await })
    };
    (tx, reload, cancel, handle)
}

async fn stop(cancel: CancellationToken, handle: tokio::task::JoinHandle<()>) {
    cancel.cancel();
    handle.await.expect("sync task panicked");
}

// ── Filter construction ──────────────────────────────────────────────────────

#[test]
fn filters_match_the_renderer_shape_for_every_scope() {
    // Verbatim parity with `buildFilter` in archiveSyncManager.ts: the tag key
    // per scope and `limit: 0` are the contract with the relay, and a wrong
    // tag key silently archives nothing.
    let (planned, scopes) = plan_subscriptions(&[
        saved("channel_h", "channel-a", "[9,40002]"),
        saved("owner_p", "owner-pk", "[24200]"),
        saved("referenced_e", "event-id", "[1]"),
    ]);

    let filters: Vec<_> = planned.iter().map(|s| s.filter.clone()).collect();
    assert_eq!(
        filters,
        vec![
            json!({ "kinds": [9, 40002], "limit": 0, "#h": ["channel-a"] }),
            json!({ "kinds": [24200], "limit": 0, "#p": ["owner-pk"] }),
            json!({ "kinds": [1], "limit": 0, "#e": ["event-id"] }),
        ]
    );
    assert_eq!(scopes.len(), 3);
    let scope = &scopes[&planned[0].id];
    assert_eq!(scope.scope_type, ScopeType::ChannelH);
    assert_eq!(scope.scope_value, "channel-a");
}

#[test]
fn subscription_id_changes_when_kinds_change() {
    // The id doubles as the relay subscription id, so a kinds change MUST
    // produce a different one — otherwise the session sees the same id with a
    // new filter and the old filter can stay live.
    let (before, _) = plan_subscriptions(&[saved("channel_h", "channel-a", "[9]")]);
    let (after, _) = plan_subscriptions(&[saved("channel_h", "channel-a", "[9,40002]")]);
    assert_ne!(before[0].id, after[0].id);
}

#[test]
fn subscription_id_is_stable_across_kind_ordering() {
    // Same set written in a different order is the same subscription; without
    // the sort it would churn the socket on every reload.
    let (a, _) = plan_subscriptions(&[saved("channel_h", "channel-a", "[40002,9]")]);
    let (b, _) = plan_subscriptions(&[saved("channel_h", "channel-a", "[9,40002]")]);
    assert_eq!(a[0].id, b[0].id);
}

#[test]
fn unknown_scope_type_is_skipped_not_guessed() {
    let (planned, scopes) = plan_subscriptions(&[
        saved("wat", "x", "[9]"),
        saved("channel_h", "channel-a", "[9]"),
    ]);
    assert_eq!(planned.len(), 1);
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[&planned[0].id].scope_value, "channel-a");
}

#[test]
fn malformed_kinds_column_yields_a_matchless_filter() {
    // Mirrors the renderer decoder: a row we cannot interpret archives
    // nothing rather than subscribing to everything.
    let (planned, _) = plan_subscriptions(&[saved("channel_h", "channel-a", "not json")]);
    assert_eq!(
        planned[0].filter,
        json!({ "kinds": [], "limit": 0, "#h": ["channel-a"] })
    );
}

#[test]
fn duplicate_rows_produce_one_subscription() {
    let (planned, _) = plan_subscriptions(&[
        saved("channel_h", "channel-a", "[9]"),
        saved("channel_h", "channel-a", "[9]"),
    ]);
    assert_eq!(planned.len(), 1);
}

// ── Loop behavior ────────────────────────────────────────────────────────────

#[tokio::test]
async fn subscribes_to_saved_configs_on_start() {
    let io = Arc::new(FakeIo::with_listings(vec![vec![saved(
        "channel_h",
        "channel-a",
        "[9]",
    )]]));
    let (_tx, _reload, cancel, handle) = spawn_sync(Arc::clone(&io));

    // The first reconcile races the cancel; wait for it to land.
    wait_for("initial subscribe", || !io.applied().is_empty()).await;
    assert_eq!(io.applied()[0].len(), 1);
    stop(cancel, handle).await;
}

#[tokio::test]
async fn reload_signal_resubscribes_with_the_new_set() {
    let io = Arc::new(FakeIo::with_listings(vec![
        vec![saved("channel_h", "channel-a", "[9]")],
        vec![
            saved("channel_h", "channel-a", "[9]"),
            saved("owner_p", "owner-pk", "[24200]"),
        ],
    ]));
    let (_tx, reload, cancel, handle) = spawn_sync(Arc::clone(&io));

    wait_for("initial subscribe", || !io.applied().is_empty()).await;
    reload.notify_one();
    wait_for("resubscribe", || io.applied().len() >= 2).await;

    assert_eq!(io.applied()[1].len(), 2);
    stop(cancel, handle).await;
}

#[tokio::test(start_paused = true)]
async fn flushes_when_the_batch_size_is_reached() {
    // Paused clock: the deadline can never fire, so a flush here is the size
    // bound and nothing else. Without this the test passes on an off-by-one
    // `is_full` — the deadline flushes the same 25 events 2s later and the
    // assertion cannot tell the two apart.
    let io = Arc::new(FakeIo::with_listings(vec![vec![saved(
        "channel_h",
        "channel-a",
        "[9]",
    )]]));
    let (tx, _reload, cancel, handle) = spawn_sync(Arc::clone(&io));
    wait_for("initial subscribe", || !io.applied().is_empty()).await;
    let id = io.applied()[0][0].id.clone();

    for _ in 0..(FLUSH_BATCH_SIZE - 1) {
        tx.send(matched(&id)).await.unwrap();
    }
    // One short of the bound: nothing may flush.
    wait_for("loop to drain the channel", || {
        tx.capacity() == tx.max_capacity()
    })
    .await;
    assert!(
        io.archived().is_empty(),
        "flushed before reaching the batch size"
    );

    tx.send(matched(&id)).await.unwrap();
    wait_for("flush", || !io.archived().is_empty()).await;

    // Exactly one batch of exactly FLUSH_BATCH_SIZE — a flush at the wrong
    // boundary shows up here as a split or an oversized batch.
    let archived = io.archived();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].len(), FLUSH_BATCH_SIZE);
    assert!(archived[0].iter().all(|scope| scope == "channel-a"));
    stop(cancel, handle).await;
}

#[tokio::test(start_paused = true)]
async fn flushes_a_partial_batch_after_the_deadline() {
    let io = Arc::new(FakeIo::with_listings(vec![vec![saved(
        "channel_h",
        "channel-a",
        "[9]",
    )]]));
    let (tx, _reload, cancel, handle) = spawn_sync(Arc::clone(&io));
    wait_for("initial subscribe", || !io.applied().is_empty()).await;
    let id = io.applied()[0][0].id.clone();

    tx.send(matched(&id)).await.unwrap();
    // Just under the deadline: still buffered.
    tokio::time::sleep(FLUSH_DEADLINE - Duration::from_millis(1)).await;
    assert!(io.archived().is_empty(), "flushed before the deadline");

    tokio::time::sleep(Duration::from_millis(2)).await;
    wait_for("flush", || !io.archived().is_empty()).await;
    assert_eq!(io.archived()[0].len(), 1);
    stop(cancel, handle).await;
}

#[tokio::test(start_paused = true)]
async fn a_trickle_cannot_postpone_the_deadline_indefinitely() {
    // The deadline belongs to the OLDEST buffered event. An idle timer reset
    // on each arrival would leave a steady trickle unflushed forever.
    let io = Arc::new(FakeIo::with_listings(vec![vec![saved(
        "channel_h",
        "channel-a",
        "[9]",
    )]]));
    let (tx, _reload, cancel, handle) = spawn_sync(Arc::clone(&io));
    wait_for("initial subscribe", || !io.applied().is_empty()).await;
    let id = io.applied()[0][0].id.clone();

    for _ in 0..4 {
        tx.send(matched(&id)).await.unwrap();
        tokio::time::sleep(FLUSH_DEADLINE / 2).await;
    }
    wait_for("flush", || !io.archived().is_empty()).await;
    assert!(!io.archived().is_empty());
    stop(cancel, handle).await;
}

#[tokio::test]
async fn events_for_an_unknown_subscription_are_dropped() {
    let io = Arc::new(FakeIo::with_listings(vec![vec![saved(
        "channel_h",
        "channel-a",
        "[9]",
    )]]));
    let (tx, _reload, cancel, handle) = spawn_sync(Arc::clone(&io));
    wait_for("initial subscribe", || !io.applied().is_empty()).await;

    // An event from a subscription we already closed: no scope, no archive.
    tx.send(matched("archive:channel_h:gone:[9]"))
        .await
        .unwrap();
    cancel.cancel();
    handle.await.unwrap();

    assert!(
        io.archived().is_empty(),
        "archived an event with no known scope"
    );
}

#[tokio::test]
async fn buffered_events_flush_on_shutdown() {
    // Ephemeral kind 24200 cannot be re-fetched, so a buffered event dropped
    // at teardown is lost permanently.
    let io = Arc::new(FakeIo::with_listings(vec![vec![saved(
        "owner_p", "owner-pk", "[24200]",
    )]]));
    let (tx, _reload, cancel, handle) = spawn_sync(Arc::clone(&io));
    wait_for("initial subscribe", || !io.applied().is_empty()).await;
    let id = io.applied()[0][0].id.clone();

    tx.send(matched(&id)).await.unwrap();
    // Wait until the loop has actually taken the event off the channel;
    // cancelling first would test a race, not the shutdown flush.
    wait_for("loop to drain the channel", || {
        tx.capacity() == tx.max_capacity()
    })
    .await;
    cancel.cancel();
    handle.await.unwrap();

    let archived = io.archived();
    assert_eq!(archived.len(), 1, "shutdown did not flush the buffer");
    assert_eq!(archived[0], vec!["owner-pk".to_string()]);
}

#[tokio::test]
async fn notifies_agent_metrics_only_when_the_backend_persisted_some() {
    let io = Arc::new(FakeIo::with_listings(vec![vec![saved(
        "owner_p", "owner-pk", "[44200]",
    )]]));
    *io.persisted_agent_metrics.lock().unwrap() = 2;
    let (tx, _reload, cancel, handle) = spawn_sync(Arc::clone(&io));
    wait_for("initial subscribe", || !io.applied().is_empty()).await;
    let id = io.applied()[0][0].id.clone();

    for _ in 0..FLUSH_BATCH_SIZE {
        tx.send(matched(&id)).await.unwrap();
    }
    wait_for("flush", || !io.archived().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(*io.metrics_notifications.lock().unwrap(), 1);
    stop(cancel, handle).await;
}

#[tokio::test]
async fn does_not_notify_when_nothing_was_persisted() {
    let io = Arc::new(FakeIo::with_listings(vec![vec![saved(
        "owner_p", "owner-pk", "[44200]",
    )]]));
    // persisted_agent_metrics stays 0: a duplicate-only batch must not
    // invalidate the renderer's usage queries.
    let (tx, _reload, cancel, handle) = spawn_sync(Arc::clone(&io));
    wait_for("initial subscribe", || !io.applied().is_empty()).await;
    let id = io.applied()[0][0].id.clone();

    for _ in 0..FLUSH_BATCH_SIZE {
        tx.send(matched(&id)).await.unwrap();
    }
    wait_for("flush", || !io.archived().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(*io.metrics_notifications.lock().unwrap(), 0);
    stop(cancel, handle).await;
}

#[tokio::test]
async fn a_failed_archive_call_does_not_notify_or_stop_the_loop() {
    let io = Arc::new(FakeIo::with_listings(vec![vec![saved(
        "channel_h",
        "channel-a",
        "[9]",
    )]]));
    *io.archive_fails.lock().unwrap() = true;
    let (tx, _reload, cancel, handle) = spawn_sync(Arc::clone(&io));
    wait_for("initial subscribe", || !io.applied().is_empty()).await;
    let id = io.applied()[0][0].id.clone();

    for _ in 0..(FLUSH_BATCH_SIZE * 2) {
        tx.send(matched(&id)).await.unwrap();
    }
    wait_for("second flush", || io.archived().len() >= 2).await;
    assert_eq!(*io.metrics_notifications.lock().unwrap(), 0);
    stop(cancel, handle).await;
}

#[tokio::test]
async fn a_failed_listing_leaves_the_previous_subscriptions_live() {
    // A transient SQLite error must not silently stop archiving.
    struct FailingList(Arc<FakeIo>, StdMutex<bool>);
    impl ArchiveSyncIo for FailingList {
        fn list_subscriptions(&self) -> BoxFuture<'_, Result<Vec<SaveSubscription>, String>> {
            Box::pin(async move {
                let mut failed = self.1.lock().unwrap();
                if *failed {
                    return Err("db is busy".into());
                }
                *failed = true;
                Ok(vec![saved("channel_h", "channel-a", "[9]")])
            })
        }
        fn set_subscriptions(&self, subscriptions: Vec<Subscription>) -> BoxFuture<'_, ()> {
            self.0.set_subscriptions(subscriptions)
        }
        fn archive(
            &self,
            candidates: Vec<ArchiveCandidate>,
        ) -> BoxFuture<'_, Result<ArchiveBatchResult, String>> {
            self.0.archive(candidates)
        }
        fn notify_agent_metrics_changed(&self) {
            self.0.notify_agent_metrics_changed();
        }
    }

    let inner = Arc::new(FakeIo::default());
    let io = Arc::new(FailingList(Arc::clone(&inner), StdMutex::new(false)));
    let (tx, rx) = mpsc::channel(8);
    let reload = Arc::new(Notify::new());
    let cancel = CancellationToken::new();
    let handle = {
        let io = Arc::clone(&io);
        let reload = Arc::clone(&reload);
        let cancel = cancel.clone();
        tokio::spawn(async move { run_sync(io.as_ref(), reload, rx, cancel).await })
    };

    wait_for("initial subscribe", || !inner.applied().is_empty()).await;
    let id = inner.applied()[0][0].id.clone();
    reload.notify_one();
    tokio::time::sleep(Duration::from_millis(10)).await;

    // The failed reload applied nothing, and the original scope still
    // demultiplexes — so events keep being archived.
    assert_eq!(inner.applied().len(), 1);
    for _ in 0..FLUSH_BATCH_SIZE {
        tx.send(matched(&id)).await.unwrap();
    }
    wait_for("flush", || !inner.archived().is_empty()).await;
    stop(cancel, handle).await;
}

// ── Lifecycle ownership ──────────────────────────────────────────────────────
//
// The renderer fires `start_archive_sync` and `stop_archive_sync` without
// awaiting them, and Tauri commands may complete in any order. These tests
// drive `ArchiveSyncState` through the same `claim_start`/`claim_stop`/
// `install`/`stop` seam the commands use, applying the halves in a chosen
// order — which is the one thing the mounted-hook test cannot do, because its
// mock `invoke` resolves immediately and can only observe call order.

/// Runs the ownership half of `start_archive_sync` under `lease`, returning the
/// installed task's cancel token. `None` means the start did not take
/// ownership. Mirrors the command minus the relay session and spawned loop,
/// which the ordering invariant does not involve.
///
/// The ownership token is dropped before returning, so these tests apply the
/// halves sequentially as before. The test that needs it held across an
/// acquisition calls [`ArchiveSyncState::begin`] directly.
///
/// The token is the task's identity: two starts produce distinct tokens, so a
/// test can name WHICH task survived an interleaving rather than only that one
/// did. "Something is running" is satisfiable by the stale start's task.
async fn start_half(
    state: &ArchiveSyncState,
    mark: (u64, u64),
    scope: (&str, &str),
) -> Option<CancellationToken> {
    let cancel = CancellationToken::new();
    state
        .begin(
            mark,
            (scope.0.to_string(), scope.1.to_string()),
            cancel.clone(),
            Arc::new(Notify::new()),
        )
        .await
        .is_some()
        .then_some(cancel)
}

/// Runs `stop_archive_sync` under `mark`.
async fn stop_half(state: &ArchiveSyncState, mark: (u64, u64)) {
    state.end(mark).await;
}

async fn is_running(state: &ArchiveSyncState) -> bool {
    state.running.lock().await.is_some()
}

/// Whether the installed task is the one `cancel` belongs to.
///
/// `CancellationToken` is not `PartialEq`, so identity is checked through the
/// shared state the clones observe: cancelling the candidate must cancel the
/// installed task if and only if they are the same instance. The token is
/// consumed by the check, so callers assert identity last.
async fn running_is(state: &ArchiveSyncState, cancel: &CancellationToken) -> bool {
    let installed = match state.running.lock().await.as_ref() {
        Some(running) => running.cancel.clone(),
        None => return false,
    };
    cancel.cancel();
    installed.is_cancelled()
}

const SCOPE: (&str, &str) = ("owner-pubkey", "wss://relay.example");

/// Wren's schedule: `start2` reaches the backend before the delayed `start1`,
/// then the old effect's cleanup runs.
///
/// A backend-issued generation fails here — `start1` arrives last, so it mints
/// the newest token and hands it to the stalest caller, whose `stop` then
/// legitimately cancels the task the new effect depends on. The lease is
/// allocated in the renderer in effect order, so `start1` is stale on arrival.
#[tokio::test]
async fn a_start_that_arrives_after_a_newer_one_cannot_supersede_it() {
    let state = ArchiveSyncState::default();

    // Effect 2 wins the race to the backend.
    let start2 = start_half(&state, (1, 2), SCOPE)
        .await
        .expect("start2 installs");
    // Effect 1's delayed start lands second and must not take ownership.
    assert!(
        start_half(&state, (1, 1), SCOPE).await.is_none(),
        "a start older than the newest lease must not install"
    );
    // Effect 1's cleanup, holding lease 1.
    stop_half(&state, (1, 1)).await;

    assert!(
        !start2.is_cancelled(),
        "start2's task must not have been cancelled by lease 1's stop"
    );
    // Identity, not survival: a lease mutant that keeps the WRONG task alive
    // would satisfy "something is running", so name the instance.
    assert!(
        running_is(&state, &start2).await,
        "the surviving task must be start2's instance — stale cleanup cancelled \
         the newest start, the exact stranding this lease prevents"
    );
}

/// The other half of the invariant: a start delayed past its own cleanup must
/// not resurrect sync after the renderer gate closed.
#[tokio::test]
async fn a_start_that_arrives_after_its_own_stop_cannot_resurrect_sync() {
    let state = ArchiveSyncState::default();

    stop_half(&state, (1, 3)).await;
    assert!(
        start_half(&state, (1, 3), SCOPE).await.is_none(),
        "a start whose own stop already ran must not install"
    );

    assert!(
        !is_running(&state).await,
        "sync was resurrected after its owner stopped"
    );
}

/// The ordinary sequence still works: each remount's start takes ownership and
/// its own cleanup stops it.
#[tokio::test]
async fn ordered_start_and_stop_still_take_effect() {
    let state = ArchiveSyncState::default();

    let first = start_half(&state, (1, 1), SCOPE)
        .await
        .expect("first start");
    assert!(is_running(&state).await, "sync must be running after start");

    stop_half(&state, (1, 1)).await;
    assert!(!is_running(&state).await, "its own stop must take effect");
    assert!(first.is_cancelled(), "the stopped task must be cancelled");

    let second = start_half(&state, (1, 2), SCOPE)
        .await
        .expect("newer start");
    assert!(
        running_is(&state, &second).await,
        "the newer start's own task must be the installed one"
    );

    stop_half(&state, (1, 2)).await;
    assert!(!is_running(&state).await, "the newer stop must take effect");
}

/// A same-scope remount that reaches the backend in order is still a no-op at
/// the socket, so the lease does not undo the idempotence the port relies on.
#[tokio::test]
async fn a_same_scope_restart_does_not_churn_the_running_task() {
    let state = ArchiveSyncState::default();

    let first = start_half(&state, (1, 1), SCOPE)
        .await
        .expect("first start");
    assert!(
        start_half(&state, (1, 2), SCOPE).await.is_none(),
        "a newer start for the same scope must not reinstall"
    );
    assert!(
        !first.is_cancelled(),
        "the original task must not be torn down"
    );
    assert!(
        running_is(&state, &first).await,
        "the original task must still be the installed one"
    );
}

/// An identity or relay change must replace the task rather than leaving the
/// old scope's socket live.
#[tokio::test]
async fn a_scope_change_replaces_the_running_task() {
    let state = ArchiveSyncState::default();

    let first = start_half(&state, (1, 1), SCOPE)
        .await
        .expect("first start");
    let second = start_half(&state, (1, 2), ("other-pubkey", SCOPE.1))
        .await
        .expect("a different scope must install");

    assert!(
        first.is_cancelled(),
        "the replaced task must be cancelled, not leaked"
    );
    assert!(
        running_is(&state, &second).await,
        "the new scope's task must be the installed one"
    );
}

/// A stop must hold its lease guard across the cancellation, not just across
/// the check.
///
/// The other lifecycle tests apply the two halves sequentially, so they cannot
/// see this: they pass against an `end` that releases `latest_lease` before
/// taking `running`. That version leaves a window — a stop clears its lease
/// check, yields, a newer start installs its task, and the resuming stop
/// cancels it. Stale cleanup strands the newest owner, which is the exact
/// failure the lease exists to prevent.
///
/// Rather than race it (unreliable either way), this observes the invariant
/// directly: hold `running` so a concurrent `end` must park after its lease
/// check, then ask whether `latest_lease` is still held. Held means the stop
/// and a competing start are mutually exclusive over the whole operation.
#[tokio::test]
async fn a_stop_holds_its_lease_guard_across_the_cancellation() {
    let state = Arc::new(ArchiveSyncState::default());

    start_half(&state, (1, 1), SCOPE)
        .await
        .expect("first start");

    // Block the second half of `end` by owning the lock it must acquire.
    let running_guard = state.running.lock().await;

    let stopper = tokio::spawn({
        let state = Arc::clone(&state);
        async move { state.end((1, 2)).await }
    });

    // Let the stop run until it blocks on `running`.
    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // The parked stop is past its lease check. If it still holds the lease
    // guard, no concurrent start can install a task for it to cancel.
    let lease_held = state.latest.try_lock().is_err();

    drop(running_guard);
    stopper.await.expect("stop task");

    assert!(
        lease_held,
        "a stop parked mid-cancellation released its lease guard: a newer start \
         can install a task in that window, which this stop then cancels — \
         stale cleanup stranding the newest owner"
    );
}

// ── Realm epochs ─────────────────────────────────────────────────────────────
//
// A lease counter lives and dies with its JS realm, but this state lives for
// the whole Tauri process. A renderer reload (useReloadShortcut,
// RootErrorBoundary, useCommunityInit) restarts the counter at zero, so
// ordering on the lease alone makes every post-reload call look stale. The
// epoch is minted here so successive realms can be ordered by an authority
// that outlives them.

/// After a renderer reload, the new realm's first start must take ownership
/// even though its lease counter restarted below what the backend has seen.
///
/// This is the regression for the reload boundary: seed the backend as if a
/// realm had already run, then have a fresh realm announce and start from
/// lease 1. Ordering on the lease alone rejects it forever — and because the
/// RootErrorBoundary reload is the recovery path for a renderer crash, that
/// would make crash recovery the thing that permanently kills archive sync.
#[tokio::test]
async fn a_realm_that_reloaded_owns_sync_despite_restarting_its_lease() {
    let state = ArchiveSyncState::default();

    // Realm 1 ran and got as far as lease 2 (StrictMode alone reaches this).
    let first_epoch = state.announce().await;
    start_half(&state, (first_epoch, 1), SCOPE)
        .await
        .expect("realm 1 start");
    stop_half(&state, (first_epoch, 2)).await;

    // The realm is destroyed by reload; the JS lease counter restarts at 1.
    let second_epoch = state.announce().await;
    assert!(
        second_epoch > first_epoch,
        "each announcing realm must outrank the last"
    );
    let reloaded = start_half(&state, (second_epoch, 1), SCOPE)
        .await
        .expect("the first start after a reload must own sync");

    assert!(
        running_is(&state, &reloaded).await,
        "the post-reload realm's task must be the installed one — ordering on \
         the lease alone leaves sync permanently absent after any reload"
    );
}

/// A call from a realm that has already been superseded must lose from the
/// moment the new realm ANNOUNCES — not merely once the new realm has managed
/// to land a lifecycle call of its own.
///
/// This is the stale-cleanup invariant replayed one level up: the dead realm's
/// in-flight cleanup arrives after the new realm has taken over, and a lease
/// comparison alone would let its larger counter win.
///
/// The old calls are sent BEFORE any epoch-2 lifecycle call deliberately.
/// That gap is reachable in production and can be arbitrarily long: the new
/// realm awaits its announcement, then waits on observer reconciliation before
/// it may issue a start at all. A version that mints the epoch without
/// publishing it passes any schedule where the new realm calls first, because
/// the new call — not the announcement — is what advanced the mark.
#[tokio::test]
async fn a_delayed_call_from_a_superseded_realm_cannot_supersede_the_new_one() {
    let state = ArchiveSyncState::default();

    // The old realm ran and owns the installed task.
    let old_epoch = state.announce().await;
    let stale = start_half(&state, (old_epoch, 1), SCOPE)
        .await
        .expect("the old realm installs");

    // The new realm announces. It has issued no lifecycle call yet.
    let new_epoch = state.announce().await;

    // The dead realm's delayed start and cleanup, both with high leases, land
    // in the gap between the new realm's announcement and its first call.
    //
    // The delayed start carries a DIFFERENT scope on purpose. Under `SCOPE` it
    // would hit the same-scope remount no-op and return `None` whatever the
    // mark said, so the assertion would hold against a backend that had stopped
    // comparing marks entirely — a pass for a benign reason is not a pass.
    assert!(
        start_half(&state, (old_epoch, 99), ("stale-realm-pubkey", SCOPE.1))
            .await
            .is_none(),
        "a superseded realm's start must not install, whatever its lease — \
         announcing is what supersedes it, not the new realm's first call"
    );
    stop_half(&state, (old_epoch, 99)).await;
    assert!(
        !stale.is_cancelled(),
        "a superseded realm's cleanup must not cancel a task it no longer owns"
    );

    // And the announcement must not have locked the new realm out of its own
    // start: publishing a mark too high is blocker 3 rebuilt from the far side.
    // A different scope so the start installs rather than taking the
    // same-scope no-op path, which would report `None` for a benign reason and
    // blur what this assertion is for.
    let current = start_half(&state, (new_epoch, 1), ("other-pubkey", SCOPE.1))
        .await
        .expect("the announcing realm's own first start must still install");
    assert!(
        running_is(&state, &current).await,
        "the surviving task must be the new realm's instance"
    );
}

/// Epochs are handed out strictly increasing, so an announcement can never tie
/// with or fall behind one already given out.
#[tokio::test]
async fn announced_epochs_strictly_increase() {
    let state = ArchiveSyncState::default();

    let mut previous = 0;
    for _ in 0..5 {
        let epoch = state.announce().await;
        assert!(
            epoch > previous,
            "epoch {epoch} did not outrank its predecessor {previous}"
        );
        previous = epoch;
    }
}

// ── Session acquisition ──────────────────────────────────────────────────────
//
// Ordering alone is not enough once a start has to acquire the shared relay
// session: `ensure_session` shuts down a different scope's socket and
// `attach_archive` replaces the archive sender, both destructively on entry.
// A start that checked its mark, yielded, and acquired afterwards would already
// have torn down the newer owner's session by the time it discovered it lost.
//
// Two things close that, and only one of them is testable here. That a
// superseded start cannot call `archive_session` at all is the token's job and
// is enforced by the compiler, not by a test — `ArchiveOwnership` is
// un-constructible outside this module, so the bypass does not compile. What
// this test pins is the property the token's usefulness rests on: while a
// winner holds it, no other start can claim.

/// While a start holds its ownership token, a newer start cannot claim — so the
/// window in which the shared session is acquired is exclusive.
///
/// This is the mutant that motivated the design and the one a test has to
/// catch: keeping the token but releasing the guards inside `begin`. That
/// compiles, keeps every other lifecycle test green, and restores exactly the
/// race — B claims, yields into `archive_session`, C claims and installs its
/// own session, then B's acquisition shuts C's socket down and attaches the
/// archive stream to a task whose token is already cancelled.
///
/// The competing start uses a DIFFERENT scope on purpose: under `SCOPE` it
/// would take the same-scope remount no-op and report `None` whatever the locks
/// did, so the assertion would hold for a benign reason.
#[tokio::test]
async fn a_newer_start_cannot_claim_while_the_owner_holds_its_token() {
    let state = Arc::new(ArchiveSyncState::default());

    let first = CancellationToken::new();
    let ownership = state
        .begin(
            (1, 1),
            (SCOPE.0.to_string(), SCOPE.1.to_string()),
            first.clone(),
            Arc::new(Notify::new()),
        )
        .await
        .expect("the first start claims ownership");

    let second = CancellationToken::new();
    let claimed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let contender = tokio::spawn({
        let state = Arc::clone(&state);
        let claimed = Arc::clone(&claimed);
        let second = second.clone();
        async move {
            let won = state
                .begin(
                    (1, 2),
                    ("other-pubkey".to_string(), SCOPE.1.to_string()),
                    second,
                    Arc::new(Notify::new()),
                )
                .await
                .is_some();
            claimed.store(won, std::sync::atomic::Ordering::SeqCst);
        }
    });

    // Give the contender every chance to claim while the owner still holds the
    // token. Yield first so it is polled at all, then a real sleep so a slow
    // scheduler cannot make this pass by never running it.
    tokio::task::yield_now().await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        !claimed.load(std::sync::atomic::Ordering::SeqCst),
        "a newer start claimed while the owner still held its token: the owner's \
         session acquisition is no longer exclusive, so a superseded start can \
         shut down the newer scope's socket"
    );
    assert!(
        !first.is_cancelled(),
        "the owner's task was cancelled while it still held ownership"
    );

    // Releasing the token is what lets the newer start through — and it must
    // then win, or this test would also pass against a `begin` that deadlocked.
    drop(ownership);
    contender.await.expect("contender task");
    assert!(
        claimed.load(std::sync::atomic::Ordering::SeqCst),
        "the newer start never claimed after the owner released its token"
    );
    assert!(
        first.is_cancelled(),
        "the superseded task must be cancelled once the newer start installs"
    );
    assert!(
        running_is(&state, &second).await,
        "the newer start's task must be the installed one"
    );
}
