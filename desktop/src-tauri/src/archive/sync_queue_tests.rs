//! Focused crash/retry tests for native archive sync's durable queue.

use super::sync_queue::*;
use super::*;
use nostr::{EventBuilder, JsonUtil, Keys, Kind};
use std::{
    path::Path,
    sync::{Arc, Mutex as StdMutex},
};

struct QueueIo {
    attempts: StdMutex<Vec<Vec<String>>>,
    delivered: StdMutex<Vec<Vec<String>>>,
    failures_remaining: StdMutex<usize>,
}

impl QueueIo {
    fn new(failures: usize) -> Self {
        Self {
            attempts: StdMutex::new(Vec::new()),
            delivered: StdMutex::new(Vec::new()),
            failures_remaining: StdMutex::new(failures),
        }
    }

    fn attempt_count(&self) -> usize {
        self.attempts.lock().unwrap().len()
    }
}

impl ArchiveSyncIo for QueueIo {
    fn list_subscriptions(&self) -> BoxFuture<'_, Result<Vec<SaveSubscription>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn set_subscriptions(&self, _subscriptions: Vec<Subscription>) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn archive(
        &self,
        candidates: Vec<ArchiveCandidate>,
    ) -> BoxFuture<'_, Result<ArchiveBatchResult, String>> {
        Box::pin(async move {
            let ids = candidates
                .iter()
                .map(|candidate| {
                    nostr::Event::from_json(&candidate.raw_event_json)
                        .unwrap()
                        .id
                        .to_hex()
                })
                .collect::<Vec<_>>();
            self.attempts.lock().unwrap().push(ids.clone());
            let mut failures = self.failures_remaining.lock().unwrap();
            if *failures > 0 {
                *failures -= 1;
                return Err("scripted archive failure".into());
            }
            drop(failures);
            self.delivered.lock().unwrap().push(ids);
            Ok(ArchiveBatchResult {
                persisted: 1,
                persisted_agent_metrics: 0,
                dropped: 0,
            })
        })
    }

    fn notify_agent_metrics_changed(&self) {}
}

fn signed_candidate(content: &str) -> DurableCandidate {
    let event = EventBuilder::new(Kind::Custom(9), content)
        .sign_with_keys(&Keys::generate())
        .unwrap();
    DurableCandidate::from_event(
        &event,
        &MatchedScope {
            scope_type: ScopeType::ChannelH,
            scope_value: "channel-a".into(),
        },
    )
}

async fn run_restored_queue(io: Arc<QueueIo>, path: &Path) -> Result<(), String> {
    let (tx, rx) = mpsc::channel(1);
    drop(tx);
    // Drop the sender immediately: the loop reconciles, observes shutdown, and
    // must drain the restored queue before returning.
    run_sync(
        io.as_ref(),
        Arc::new(Notify::new()),
        rx,
        CancellationToken::new(),
        DurableQueue::open(path.to_path_buf())?,
    )
    .await
}

#[tokio::test]
async fn restart_replays_pending_and_commits_ack_tombstone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pending.json");
    let candidate = signed_candidate("restart");
    {
        let mut queue = DurableQueue::open(path.clone()).unwrap();
        assert_eq!(
            queue.enqueue(candidate.clone()).unwrap(),
            EnqueueResult::Accepted
        );
    }

    let io = Arc::new(QueueIo::new(0));
    run_restored_queue(Arc::clone(&io), &path).await.unwrap();
    assert_eq!(io.delivered.lock().unwrap().len(), 1);

    let mut reopened = DurableQueue::open(path).unwrap();
    assert!(reopened.is_empty());
    assert_eq!(
        reopened.enqueue(candidate).unwrap(),
        EnqueueResult::Duplicate
    );
}

#[tokio::test]
async fn fail_once_retries_same_head_without_reordering() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pending.json");
    let first = signed_candidate("first");
    let second = signed_candidate("second");
    let mut queue = DurableQueue::open(path.clone()).unwrap();
    queue.enqueue(first.clone()).unwrap();
    queue.enqueue(second.clone()).unwrap();
    drop(queue);

    let io = Arc::new(QueueIo::new(1));
    run_restored_queue(Arc::clone(&io), &path).await.unwrap();
    assert_eq!(io.attempt_count(), 2);
    let attempts = io.attempts.lock().unwrap();
    assert_eq!(attempts[0], attempts[1]);
    assert_eq!(attempts[1], vec![first.event_id, second.event_id]);
    assert!(DurableQueue::open(path).unwrap().is_empty());
}

#[tokio::test]
async fn repeated_failure_retains_head_for_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pending.json");
    let candidate = signed_candidate("retain");
    let mut queue = DurableQueue::open(path.clone()).unwrap();
    queue.enqueue(candidate).unwrap();
    drop(queue);

    let io = Arc::new(QueueIo::new(usize::MAX));
    run_restored_queue(Arc::clone(&io), &path).await.unwrap();
    assert_eq!(io.attempt_count(), ARCHIVE_RETRY_ATTEMPTS);
    assert_eq!(DurableQueue::open(path).unwrap().len(), 1);
}

#[test]
fn duplicate_pending_event_and_scope_is_written_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pending.json");
    let candidate = signed_candidate("dedupe");
    let mut queue = DurableQueue::open(path.clone()).unwrap();
    assert_eq!(
        queue.enqueue(candidate.clone()).unwrap(),
        EnqueueResult::Accepted
    );
    assert_eq!(queue.enqueue(candidate).unwrap(), EnqueueResult::Duplicate);
    assert_eq!(DurableQueue::open(path).unwrap().len(), 1);
}

#[test]
fn corrupt_pending_state_is_quarantined_and_stays_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pending.json");
    std::fs::write(&path, b"{not-json").unwrap();

    let error = DurableQueue::open(path.clone()).unwrap_err();
    assert!(error.contains("quarantined"));
    assert!(blocked_diagnostic_path(&path).exists());
    assert!(DurableQueue::open(path.clone())
        .unwrap_err()
        .contains("fail-closed"));
    assert!(std::fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".corrupt-")
    }));
}

#[test]
fn queue_bound_rejects_new_event_without_mutating_pending_head() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pending.json");
    let candidate = signed_candidate("capacity");
    let mut queue = DurableQueue::open(path).unwrap();
    queue.fill_to_capacity_for_test(candidate);
    let error = queue.enqueue(signed_candidate("one-too-many")).unwrap_err();
    assert!(error.contains("limit reached"));
    assert_eq!(queue.len(), MAX_DURABLE_QUEUE_ENTRIES);
}

#[tokio::test]
async fn teardown_is_bounded_and_leaves_failed_head_durable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pending.json");
    let mut queue = DurableQueue::open(path.clone()).unwrap();
    queue.enqueue(signed_candidate("teardown")).unwrap();

    let io = Arc::new(QueueIo::new(usize::MAX));
    let (_tx, rx) = mpsc::channel(1);
    let cancel = CancellationToken::new();
    cancel.cancel();
    tokio::time::timeout(
        SHUTDOWN_WAIT_TIMEOUT,
        run_sync(io.as_ref(), Arc::new(Notify::new()), rx, cancel, queue),
    )
    .await
    .expect("teardown exceeded its bound")
    .unwrap();

    assert_eq!(io.attempt_count(), ARCHIVE_RETRY_ATTEMPTS);
    assert_eq!(DurableQueue::open(path).unwrap().len(), 1);
}

#[tokio::test]
async fn lifecycle_stop_waits_until_the_sync_task_finishes() {
    let state = Arc::new(ArchiveSyncState::default());
    let completion = Arc::new(SyncCompletion::default());
    let ownership = state
        .begin(
            (1, 1),
            ("owner".into(), "wss://relay.test".into()),
            CancellationToken::new(),
            Arc::new(Notify::new()),
            Arc::clone(&completion),
        )
        .await
        .expect("start owns sync");
    drop(ownership);

    let stopper = tokio::spawn({
        let state = Arc::clone(&state);
        async move { state.end((1, 1)).await }
    });
    tokio::task::yield_now().await;
    assert!(
        !stopper.is_finished(),
        "stop returned before sync completion"
    );

    completion.finish();
    stopper.await.unwrap().unwrap();
    assert!(state.running.lock().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn lifecycle_stop_timeout_is_explicit_and_keeps_task_visible() {
    let state = ArchiveSyncState::default();
    let completion = Arc::new(SyncCompletion::default());
    let ownership = state
        .begin(
            (1, 1),
            ("owner".into(), "wss://relay.test".into()),
            CancellationToken::new(),
            Arc::new(Notify::new()),
            Arc::clone(&completion),
        )
        .await
        .expect("start owns sync");
    drop(ownership);

    let error = state.end((1, 1)).await.unwrap_err();
    assert!(error.contains("teardown did not finish"));
    assert!(state.running.lock().await.is_some());

    completion.finish();
    state.clear_completed(&completion).await;
    assert!(state.running.lock().await.is_none());

    let restarted = Arc::new(SyncCompletion::default());
    restarted.finish();
    assert!(state
        .begin(
            (1, 2),
            ("owner".into(), "wss://relay.test".into()),
            CancellationToken::new(),
            Arc::new(Notify::new()),
            restarted,
        )
        .await
        .is_some());
}
