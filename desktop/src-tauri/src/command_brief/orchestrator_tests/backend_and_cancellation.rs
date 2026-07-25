use super::*;

#[derive(Clone)]
struct CountingSourceBackend {
    calls: Arc<AtomicUsize>,
}

impl SourceBackend for CountingSourceBackend {
    fn verify_active_rag_snapshot(&self) -> Result<VerifiedRagSnapshot, SourceCollectionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        unreachable!("a pre-cancelled collection must not reach the backend")
    }

    fn memory_conflict_count(&self) -> u64 {
        0
    }

    fn collect_rag(
        &self,
        _snapshot: &VerifiedRagSnapshot,
        _intent: &FixedRetrievalIntent,
        _cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        unreachable!("a pre-cancelled collection must not reach the backend")
    }

    fn collect_memory(
        &self,
        _intent: &FixedRetrievalIntent,
        _cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        unreachable!("a pre-cancelled collection must not reach the backend")
    }

    fn collect_apple(
        &self,
        _request: &AppleInputRequest,
        _cancellation: &CancellationToken,
    ) -> Result<AppleInputResponse, SourceCollectionError> {
        unreachable!("a pre-cancelled collection must not reach the backend")
    }

    fn recheck_rag_snapshot(
        &self,
        _expected: &VerifiedRagSnapshot,
        _cancellation: &CancellationToken,
    ) -> Result<(), SourceCollectionError> {
        unreachable!("a pre-cancelled collection must not reach the backend")
    }
}

#[tokio::test]
async fn pre_cancelled_collection_stops_before_any_source_backend_call() {
    let calls = Arc::new(AtomicUsize::new(0));
    let backend = CountingSourceBackend {
        calls: Arc::clone(&calls),
    };
    let selection = AppleBriefSelection::for_test(json!({
        "schema_version": 1,
        "calendar_ids": ["calendar-command"],
        "reminder_list_ids": ["reminders-command"],
        "note_folder_ids": ["Notes"],
        "file_paths": ["/Users/command/brief.txt"],
        "maximum_records_per_source": 25
    }))
    .expect("valid selection");
    let provider = CollectedSourceProvider::new(backend, selection);
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let result = provider
        .freeze("run-cancelled", "brief request", OBSERVED, cancellation)
        .await;

    assert!(matches!(result, Err(SourceCollectionError::Cancelled)));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[derive(Clone)]
struct ReloadingBackend {
    snapshot: VerifiedRagSnapshot,
}

impl SourceBackend for ReloadingBackend {
    fn verify_active_rag_snapshot(&self) -> Result<VerifiedRagSnapshot, SourceCollectionError> {
        Ok(self.snapshot.clone())
    }

    fn memory_conflict_count(&self) -> u64 {
        0
    }

    fn collect_rag(
        &self,
        snapshot: &VerifiedRagSnapshot,
        intent: &FixedRetrievalIntent,
        _cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        Ok(json!({
            "schema": "rag-evidence-v1",
            "tool_policy": {
                "mode": "read_only",
                "retrieved_content": "untrusted_evidence",
                "instruction_effect": "none"
            },
            "query": intent.query(),
            "snapshot": {"active_snapshot_id": snapshot.snapshot_id()},
            "retrieved_at": OBSERVED,
            "total": 1,
            "results": [{
                "untrusted_evidence": true,
                "source": {
                    "source_id": format!("rag:{:?}", intent.adviser()),
                    "collection": "navy-publications",
                    "document_id": format!("document-{:?}", intent.adviser()),
                    "chunk_id": "chunk-1",
                    "snapshot_id": snapshot.snapshot_id(),
                    "retrieved_at": OBSERVED,
                    "quoted_location": {"page": 1}
                },
                "scores": {"dense": 0.9},
                "quoted_text": "Reloaded snapshot evidence.",
                "metadata": {"title": "Reloaded"}
            }]
        }))
    }

    fn collect_memory(
        &self,
        _intent: &FixedRetrievalIntent,
        _cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        Ok(json!({
            "schema": "memory-evidence-v1",
            "tool_policy": {
                "mode": "read_only",
                "retrieved_content": "untrusted_evidence",
                "instruction_effect": "none"
            },
            "serving_node_id": "node:mac-command",
            "retrieved_at": OBSERVED,
            "total": 0,
            "results": []
        }))
    }

    fn collect_apple(
        &self,
        request: &AppleInputRequest,
        _cancellation: &CancellationToken,
    ) -> Result<AppleInputResponse, SourceCollectionError> {
        Ok(serde_json::from_value(json!({
            "source": request.source_name(),
            "permission": "unavailable",
            "observedAt": OBSERVED,
            "records": [],
            "truncated": false,
            "error": "signed_helper_unavailable"
        }))
        .expect("valid unavailable Apple response"))
    }

    fn recheck_rag_snapshot(
        &self,
        expected: &VerifiedRagSnapshot,
        _cancellation: &CancellationToken,
    ) -> Result<(), SourceCollectionError> {
        expected
            .verify_unchanged(self.snapshot.snapshot_id())
            .map_err(Into::into)
    }
}

#[derive(Clone)]
struct SequencedBackendLoader {
    snapshots: Arc<Mutex<VecDeque<&'static str>>>,
    loads: Arc<AtomicUsize>,
}

impl SequencedBackendLoader {
    fn new(snapshots: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            snapshots: Arc::new(Mutex::new(snapshots.into_iter().collect())),
            loads: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl SourceBackendLoader for SequencedBackendLoader {
    fn load<'a>(
        &'a self,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<Arc<dyn SourceBackend + Send + Sync>, SourceCollectionError>> {
        boxed(async move {
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            self.loads.fetch_add(1, Ordering::SeqCst);
            let snapshot = self
                .snapshots
                .lock()
                .expect("snapshot sequence")
                .pop_front()
                .ok_or(SourceCollectionError::RagUnavailable)?;
            Ok(Arc::new(ReloadingBackend {
                snapshot: VerifiedRagSnapshot::for_test(snapshot, OBSERVED, OBSERVED),
            }) as Arc<dyn SourceBackend + Send + Sync>)
        })
    }
}

fn reloading_sources(loader: SequencedBackendLoader) -> Arc<dyn BriefSourceProvider> {
    let selection = AppleBriefSelection::for_test(json!({
        "schema_version": 1,
        "calendar_ids": ["calendar-command"],
        "reminder_list_ids": ["reminders-command"],
        "note_folder_ids": ["Notes"],
        "file_paths": ["/Users/command/brief.txt"],
        "maximum_records_per_source": 25
    }))
    .expect("valid selection");
    Arc::new(ReloadingSourceProvider::new(Arc::new(loader), selection))
}

#[tokio::test]
async fn production_style_provider_adopts_one_fresh_signed_snapshot_then_rejects_a_second_change() {
    let loader =
        SequencedBackendLoader::new([SNAPSHOT_A, SNAPSHOT_B, SNAPSHOT_B, SNAPSHOT_B, SNAPSHOT_B]);
    let first_orchestrator = orchestrator(
        1,
        reloading_sources(loader.clone()),
        Arc::new(FakeAdviserProvider::default()),
        Arc::new(FakePersistence::default()),
    );

    let run_id = first_orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&first_orchestrator, &run_id).await,
        BriefRunState::Degraded
    );
    let brief = serde_json::to_value(first_orchestrator.result(&run_id).expect("brief").brief())
        .expect("json");
    assert_eq!(brief["snapshotId"], SNAPSHOT_B);
    assert_eq!(loader.loads.load(Ordering::SeqCst), 5);

    let changing_loader =
        SequencedBackendLoader::new([SNAPSHOT_A, SNAPSHOT_B, SNAPSHOT_B, SNAPSHOT_A]);
    let failing = orchestrator(
        1,
        reloading_sources(changing_loader.clone()),
        Arc::new(FakeAdviserProvider::default()),
        Arc::new(FakePersistence::default()),
    );
    let failing_run_id = failing.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&failing, &failing_run_id).await,
        BriefRunState::Failed
    );
    let status = serde_json::to_value(failing.status(&failing_run_id).expect("status"))
        .expect("status json");
    assert_eq!(status["error"], "snapshot_changed");
    assert_eq!(changing_loader.loads.load(Ordering::SeqCst), 4);
}

#[derive(Clone)]
struct ActiveCancellationBackend {
    rag_calls: Arc<AtomicUsize>,
    memory_calls: Arc<AtomicUsize>,
    apple_calls: Arc<AtomicUsize>,
}

impl SourceBackend for ActiveCancellationBackend {
    fn verify_active_rag_snapshot(&self) -> Result<VerifiedRagSnapshot, SourceCollectionError> {
        Ok(VerifiedRagSnapshot::for_test(
            SNAPSHOT_A, OBSERVED, OBSERVED,
        ))
    }

    fn memory_conflict_count(&self) -> u64 {
        0
    }

    fn collect_rag(
        &self,
        _snapshot: &VerifiedRagSnapshot,
        _intent: &FixedRetrievalIntent,
        cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        self.rag_calls.fetch_add(1, Ordering::SeqCst);
        cancellation.cancel();
        Err(SourceReadError::new("cancelled_in_flight"))
    }

    fn collect_memory(
        &self,
        _intent: &FixedRetrievalIntent,
        _cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        self.memory_calls.fetch_add(1, Ordering::SeqCst);
        Err(SourceReadError::new("unexpected_memory_call"))
    }

    fn collect_apple(
        &self,
        request: &AppleInputRequest,
        _cancellation: &CancellationToken,
    ) -> Result<AppleInputResponse, SourceCollectionError> {
        self.apple_calls.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::from_value(json!({
            "source": request.source_name(),
            "permission": "unavailable",
            "observedAt": OBSERVED,
            "records": [],
            "truncated": false,
            "error": "unexpected_apple_call"
        }))
        .expect("valid unavailable Apple response"))
    }

    fn recheck_rag_snapshot(
        &self,
        _expected: &VerifiedRagSnapshot,
        _cancellation: &CancellationToken,
    ) -> Result<(), SourceCollectionError> {
        Ok(())
    }
}

#[tokio::test]
async fn active_source_cancellation_stops_later_reads_and_never_reaches_persistence() {
    let rag_calls = Arc::new(AtomicUsize::new(0));
    let memory_calls = Arc::new(AtomicUsize::new(0));
    let apple_calls = Arc::new(AtomicUsize::new(0));
    let backend = ActiveCancellationBackend {
        rag_calls: Arc::clone(&rag_calls),
        memory_calls: Arc::clone(&memory_calls),
        apple_calls: Arc::clone(&apple_calls),
    };
    let selection = AppleBriefSelection::for_test(json!({
        "schema_version": 1,
        "calendar_ids": ["calendar-command"],
        "reminder_list_ids": ["reminders-command"],
        "note_folder_ids": ["Notes"],
        "file_paths": ["/Users/command/brief.txt"],
        "maximum_records_per_source": 25
    }))
    .expect("valid selection");
    let sources = Arc::new(CollectedSourceProvider::new(backend, selection));
    let persistence = Arc::new(FakePersistence::default());
    let orchestrator = orchestrator(
        1,
        sources,
        Arc::new(FakeAdviserProvider::default()),
        persistence.clone(),
    );

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Cancelled
    );
    assert_eq!(rag_calls.load(Ordering::SeqCst), 1);
    assert_eq!(memory_calls.load(Ordering::SeqCst), 0);
    assert_eq!(apple_calls.load(Ordering::SeqCst), 0);
    assert!(persistence.values.lock().expect("persistence").is_empty());
    persistence.assert_one_terminal(
        CommandBriefLifecycleState::Cancelled,
        Some(CommandBriefFailureCode::CancellationRequested),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_observation_is_atomic_at_capacity_and_terminal_cancel_is_false() {
    let orchestrator = orchestrator(
        2,
        Arc::new(FakeSourceProvider::with_snapshots(std::iter::repeat_n(
            SNAPSHOT_A, 65,
        ))),
        Arc::new(FakeAdviserProvider::default()),
        Arc::new(FakePersistence::default()),
    );
    let mut run_ids = Vec::new();
    for _ in 0..65 {
        let run_id = orchestrator.start(request()).expect("run starts");
        assert_eq!(
            wait_terminal(&orchestrator, &run_id).await,
            BriefRunState::Completed
        );
        run_ids.push(run_id);
    }

    let mut retained = 0;
    for run_id in &run_ids {
        if let Some((status, result)) = orchestrator.status_and_result(run_id) {
            retained += 1;
            let status = serde_json::to_value(status).expect("status json");
            assert_eq!(status["state"], "completed");
            assert!(result.is_some());
        }
        assert!(!orchestrator.cancel(run_id));
    }
    assert_eq!(retained, 64);
}

struct BarrierFinalizationGate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl BarrierFinalizationGate {
    fn new() -> Self {
        Self {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        }
    }
}

impl BriefFinalizationGate for BarrierFinalizationGate {
    fn wait<'a>(&'a self) -> BriefFuture<'a, ()> {
        boxed(async move {
            self.entered.notify_one();
            self.release.notified().await;
        })
    }
}

fn orchestrator_with_finalization_gate(
    gate: Arc<dyn BriefFinalizationGate>,
    persistence: Arc<dyn BriefPersistence>,
) -> CommandBriefOrchestrator {
    CommandBriefOrchestrator::new_with_finalization_gate(
        LocalModelScheduler::new(1).expect("scheduler"),
        Arc::new(FakeSourceProvider::with_snapshots([SNAPSHOT_A])),
        Arc::new(FakeAdviserProvider::default()),
        persistence,
        gate,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_after_persistence_commit_is_rejected_and_result_is_installed() {
    let cancellation_gate = Arc::new(BarrierFinalizationGate::new());
    let cancellation_persistence = Arc::new(FakePersistence::default());
    let cancelled = orchestrator_with_finalization_gate(
        cancellation_gate.clone(),
        cancellation_persistence.clone(),
    );
    let cancelled_run = cancelled.start(request()).expect("run starts");
    cancellation_gate.entered.notified().await;
    assert_eq!(
        cancellation_persistence
            .values
            .lock()
            .expect("persistence")
            .len(),
        1
    );
    assert!(!cancelled.cancel(&cancelled_run));
    cancellation_gate.release.notify_one();
    assert_eq!(
        wait_terminal(&cancelled, &cancelled_run).await,
        BriefRunState::Completed
    );
    assert!(cancelled.result(&cancelled_run).is_some());

    let completion_gate = Arc::new(BarrierFinalizationGate::new());
    let completed = orchestrator_with_finalization_gate(
        completion_gate.clone(),
        Arc::new(FakePersistence::default()),
    );
    let completed_run = completed.start(request()).expect("run starts");
    completion_gate.entered.notified().await;
    completion_gate.release.notify_one();
    assert_eq!(
        wait_terminal(&completed, &completed_run).await,
        BriefRunState::Completed
    );
    assert!(completed.result(&completed_run).is_some());
    assert!(!completed.cancel(&completed_run));
}
