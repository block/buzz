use super::*;
use std::sync::Mutex as StdMutex;

#[derive(Default)]
struct Io {
    reports: StdMutex<Vec<(&'static str, usize)>>,
    calls: StdMutex<Vec<Vec<String>>>,
}
impl ArchiveSyncIo for Io {
    fn list_subscriptions(&self) -> BoxFuture<'_, Result<Vec<SaveSubscription>, String>> {
        Box::pin(async {
            Ok(vec![SaveSubscription {
                identity_pubkey: "owner".into(),
                relay_url: "wss://relay.test".into(),
                scope_type: "channel_h".into(),
                scope_value: "channel".into(),
                kinds: "[9]".into(),
                created_at: 0,
            }])
        })
    }
    fn set_subscriptions(&self, _: Vec<Subscription>) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn archive(
        &self,
        _: Vec<ArchiveCandidate>,
    ) -> BoxFuture<'_, Result<ArchiveBatchResult, String>> {
        unreachable!("runner must use partial-outcome seam")
    }
    fn archive_retry(&self, candidates: Vec<ArchiveCandidate>) -> BoxFuture<'_, SyncOutcome> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(
                candidates
                    .iter()
                    .map(|c| c.raw_event_json.clone())
                    .collect(),
            );
            let retry = candidates
                .into_iter()
                .filter(|c| c.raw_event_json == "ciphertext")
                .collect();
            SyncOutcome {
                committed: ArchiveBatchResult {
                    persisted: 0,
                    persisted_agent_metrics: 0,
                    dropped: 0,
                },
                retry,
            }
        })
    }
    fn notify_agent_metrics_changed(&self) {}
    fn report_degraded(&self, reason: &'static str, count: usize) {
        self.reports.lock().unwrap().push((reason, count));
    }
}
fn input(text: &str) -> ArchiveCandidate {
    ArchiveCandidate {
        raw_event_json: text.into(),
        matched_scope: MatchedScope {
            scope_type: ScopeType::ChannelH,
            scope_value: "channel".into(),
        },
    }
}

#[tokio::test(start_paused = true)]
async fn retries_exhaust_without_processing_or_losing_ciphertext() {
    let io = Io::default();
    let mut b = Buffer::default();
    b.admit(&io, input("ciphertext"));
    tokio::time::advance(FLUSH_DEADLINE).await;
    for attempt in 0..MAX_ATTEMPTS {
        let (batch, prior) = b.take_job().expect("due attempt");
        assert_eq!(prior, attempt);
        let size = batch.iter().map(bytes).sum();
        let outcome = io.archive_retry(batch).await;
        b.complete(&io, 1, size, prior, outcome);
        assert!(b.take_job().is_none(), "no hot retry");
        tokio::time::advance(Duration::from_secs(8)).await;
    }
    assert_eq!(io.calls.lock().unwrap().len(), MAX_ATTEMPTS as usize);
    assert_eq!(
        *io.reports.lock().unwrap(),
        vec![("retry-exhausted-ciphertext-retained", 1)]
    );
    let retained = b.finish();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].raw_event_json, "ciphertext");
}

#[tokio::test(start_paused = true)]
async fn fresh_work_alternates_with_due_retries_and_capacity_is_explicit() {
    let io = Io::default();
    let mut b = Buffer::default();
    for _ in 0..MAX_PARKED {
        b.admit(&io, input("ciphertext"));
    }
    b.admit(&io, input("overflow"));
    assert_eq!(b.count, MAX_PARKED);
    assert_eq!(
        io.reports.lock().unwrap()[0].0,
        "capacity-exceeded-input-not-retained"
    );
    let (batch, _) = b.take_job().unwrap();
    let size = batch.iter().map(bytes).sum();
    let outcome = io.archive_retry(batch).await;
    b.complete(&io, FLUSH_BATCH_SIZE, size, 0, outcome);
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(b.take_job().unwrap().1, 1);
    assert_eq!(
        b.take_job().unwrap().1,
        0,
        "due retry backlog must not starve fresh work"
    );
}

#[tokio::test(start_paused = true)]
async fn oversized_admission_is_not_classified_as_invalid_content() {
    let io = Io::default();
    let mut b = Buffer::default();
    b.admit(&io, input(&"x".repeat(MAX_CANDIDATE_BYTES)));
    assert_eq!(b.count, 0);
    assert_eq!(b.bytes, 0);
    assert_eq!(io.reports.lock().unwrap().len(), 1);
}

async fn wait(mut predicate: impl FnMut() -> bool) {
    for _ in 0..10_000 {
        if predicate() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("runner did not make progress");
}

#[tokio::test(start_paused = true)]
async fn production_loop_accepts_fresh_events_while_ciphertext_retries() {
    let io = Arc::new(Io::default());
    let (tx, rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    let task = {
        let io = io.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            run_sync_pending(
                &*io,
                Arc::new(Notify::new()),
                rx,
                cancel,
                vec![input("ciphertext")],
            )
            .await
        })
    };
    tokio::time::advance(FLUSH_DEADLINE).await;
    // Allow initial reconcile and its first deadline to be established.
    tokio::time::advance(FLUSH_DEADLINE).await;
    let keys = nostr::Keys::generate();
    for _ in 0..FLUSH_BATCH_SIZE {
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "good")
            .sign_with_keys(&keys)
            .unwrap();
        tx.send(MatchedEvent {
            subscription_id: "archive:channel_h:channel:9".into(),
            event: Box::new(event),
        })
        .await
        .unwrap();
    }
    wait(|| {
        io.calls
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .any(|s| s != "ciphertext")
    })
    .await;
    cancel.cancel();
    let retained = task.await.unwrap();
    assert!(retained.iter().any(|c| c.raw_event_json == "ciphertext"));
}

#[tokio::test(start_paused = true)]
async fn expanded_retry_results_cannot_exceed_shared_budget() {
    let io = Io::default();
    let mut b = Buffer::default();
    b.admit(&io, input("ciphertext"));
    let size = bytes(&input("ciphertext"));
    b.fresh.clear(); // model the charged in-flight input
    b.complete(
        &io,
        1,
        size,
        0,
        SyncOutcome::retry((0..MAX_PARKED + 1).map(|_| input("ciphertext")).collect()),
    );
    assert_eq!(b.count, MAX_PARKED);
    assert!(b.bytes <= MAX_PARKED_BYTES);
    assert_eq!(
        io.reports.lock().unwrap()[0].0,
        "capacity-exceeded-input-not-retained"
    );
}
