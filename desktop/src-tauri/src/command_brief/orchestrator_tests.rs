use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::command_services::apple_inputs::{
    AppleBriefSelection, AppleInputRequest, AppleInputResponse,
};
use crate::command_services::rag::VerifiedRagSnapshot;

use super::orchestrator::{
    BriefAdviserError, BriefAdviserProvider, BriefFuture, BriefPersistence, BriefPersistenceError,
    BriefSourceProvider, CollectedSourceProvider, CommandBriefOrchestrator, CommandBriefRequest,
};
use super::provenance::ValidatedSource;
use super::scheduler::LocalModelScheduler;
use super::sources::{
    FixedRetrievalIntent, FrozenSourceContext, SourceBackend, SourceCollectionError,
    SourceReadError,
};
use super::types::{
    AdviserContribution, AdviserId, BriefRunState, BriefSection, CommandBrief, SourceLedgerEntry,
    SPECIALIST_ADVISERS,
};

const SNAPSHOT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SNAPSHOT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const OBSERVED: &str = "2026-07-25T06:00:00+10:00";

fn boxed<'a, T: Send + 'a>(
    future: impl Future<Output = T> + Send + 'a,
) -> Pin<Box<dyn Future<Output = T> + Send + 'a>> {
    Box::pin(future)
}

fn ledger(snapshot: &str, run_id: &str) -> Vec<SourceLedgerEntry> {
    vec![SourceLedgerEntry::parse_for_snapshot(
        json!({
            "classification": "OFFICIAL",
            "ledgerId": format!("ledger-{run_id}"),
            "sourceId": "rag:source",
            "sourceKind": "rag",
            "collection": "navy-publications",
            "documentId": "document-1",
            "chunkId": "chunk-1",
            "timestamp": OBSERVED,
            "snapshotId": snapshot,
            "quotedLocation": {
                "quote": "\"Verified source text.\"",
                "location": "{\"page\":1}"
            },
            "retrievedAt": OBSERVED,
            "observedAt": OBSERVED
        }),
        snapshot,
    )
    .expect("valid source")]
}

fn source_context(
    run_id: &str,
    snapshot: &str,
    degraded: Vec<BriefSection>,
    limitations: Vec<String>,
) -> FrozenSourceContext {
    FrozenSourceContext::for_orchestrator_test(
        run_id,
        snapshot,
        OBSERVED,
        ledger(snapshot, run_id),
        degraded,
        limitations,
    )
}

fn section_for(adviser: AdviserId) -> BriefSection {
    match adviser {
        AdviserId::Operations => BriefSection::Operations,
        AdviserId::Navigation => BriefSection::Navigation,
        AdviserId::DailyRoutine => BriefSection::DailyRoutine,
        AdviserId::Reporting => BriefSection::Reports,
        AdviserId::Plans => BriefSection::Planning306090,
        AdviserId::ChiefOfStaff => panic!("chief is not a specialist"),
    }
}

fn contribution(adviser: AdviserId, source_id: &str, dissent: &str) -> AdviserContribution {
    AdviserContribution::parse_for_adviser(
        json!({
            "classification": "OFFICIAL",
            "adviser": adviser,
            "section": section_for(adviser),
            "findings": [{
                "classification": "OFFICIAL",
                "text": format!("{adviser:?} supported finding"),
                "sourceIds": [source_id]
            }],
            "confidence": 0.8,
            "limitations": [],
            "dissent": [dissent],
            "proposedActions": []
        }),
        adviser,
        &std::collections::BTreeSet::from([source_id.to_string()]),
    )
    .expect("valid contribution")
}

fn chief_output(contributions: &[AdviserContribution]) -> Value {
    let findings = contributions
        .iter()
        .flat_map(|item| item.findings())
        .cloned()
        .collect::<Vec<_>>();
    let dissent = contributions
        .iter()
        .flat_map(|item| item.dissent().iter().cloned())
        .collect::<Vec<_>>();
    json!({
        "classification": "OFFICIAL",
        "adviser": "chief_of_staff",
        "findings": findings,
        "limitations": [],
        "dissent": dissent
    })
}

#[derive(Default)]
struct FakeSourceProvider {
    snapshots: Mutex<VecDeque<&'static str>>,
    rechecks: Mutex<VecDeque<Result<(), SourceCollectionError>>>,
    freeze_tokens: Mutex<Vec<CancellationToken>>,
    recheck_tokens: Mutex<Vec<CancellationToken>>,
    degraded: Vec<BriefSection>,
    limitations: Vec<String>,
}

impl FakeSourceProvider {
    fn with_snapshots(snapshots: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            snapshots: Mutex::new(snapshots.into_iter().collect()),
            ..Self::default()
        }
    }
}

impl BriefSourceProvider for FakeSourceProvider {
    fn freeze<'a>(
        &'a self,
        run_id: &'a str,
        _co_request: &'a str,
        _observed_at: &'a str,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<FrozenSourceContext, SourceCollectionError>> {
        boxed(async move {
            self.freeze_tokens
                .lock()
                .expect("freeze token lock")
                .push(cancellation.clone());
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            let snapshot = self
                .snapshots
                .lock()
                .expect("snapshot lock")
                .pop_front()
                .unwrap_or(SNAPSHOT_A);
            Ok(source_context(
                run_id,
                snapshot,
                self.degraded.clone(),
                self.limitations.clone(),
            ))
        })
    }

    fn recheck<'a>(
        &'a self,
        _context: &'a FrozenSourceContext,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<(), SourceCollectionError>> {
        boxed(async move {
            self.recheck_tokens
                .lock()
                .expect("recheck token lock")
                .push(cancellation.clone());
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            self.rechecks
                .lock()
                .expect("recheck lock")
                .pop_front()
                .unwrap_or(Ok(()))
        })
    }
}

#[derive(Default)]
struct FakeAdviserProvider {
    calls: Mutex<Vec<String>>,
    snapshots: Mutex<Vec<(AdviserId, String)>>,
    failures: Mutex<BTreeMap<AdviserId, usize>>,
    chief_override: Mutex<Option<Value>>,
    tokens: Mutex<Vec<CancellationToken>>,
}

impl FakeAdviserProvider {
    fn fail_once(&self, adviser: AdviserId) {
        self.failures
            .lock()
            .expect("failure lock")
            .insert(adviser, 1);
    }
}

impl BriefAdviserProvider for FakeAdviserProvider {
    fn run_specialist<'a>(
        &'a self,
        run_id: &'a str,
        adviser: AdviserId,
        sources: Vec<ValidatedSource>,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<AdviserContribution, BriefAdviserError>> {
        boxed(async move {
            self.calls
                .lock()
                .expect("calls lock")
                .push(format!("specialist:{adviser:?}"));
            self.tokens
                .lock()
                .expect("tokens lock")
                .push(cancellation.clone());
            if cancellation.is_cancelled() {
                return Err(BriefAdviserError::Cancelled);
            }
            let snapshot = sources
                .first()
                .map(|source| source.snapshot_id().to_string())
                .ok_or(BriefAdviserError::Failed)?;
            self.snapshots
                .lock()
                .expect("snapshots lock")
                .push((adviser, snapshot));
            let should_fail = self
                .failures
                .lock()
                .expect("failure lock")
                .get_mut(&adviser)
                .is_some_and(|remaining| {
                    if *remaining == 0 {
                        false
                    } else {
                        *remaining -= 1;
                        true
                    }
                });
            if should_fail {
                return Err(BriefAdviserError::Failed);
            }
            let source_id = sources
                .first()
                .map(|source| source.ledger_id())
                .ok_or(BriefAdviserError::Failed)?;
            Ok(contribution(
                adviser,
                source_id,
                &format!("{run_id}:{adviser:?}:dissent"),
            ))
        })
    }

    fn run_chief_of_staff<'a>(
        &'a self,
        _run_id: &'a str,
        contributions: Vec<AdviserContribution>,
        _source_ledger: Vec<ValidatedSource>,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<Value, BriefAdviserError>> {
        boxed(async move {
            self.calls
                .lock()
                .expect("calls lock")
                .push("chief".to_string());
            self.tokens
                .lock()
                .expect("tokens lock")
                .push(cancellation.clone());
            if cancellation.is_cancelled() {
                return Err(BriefAdviserError::Cancelled);
            }
            Ok(self
                .chief_override
                .lock()
                .expect("chief override lock")
                .clone()
                .unwrap_or_else(|| chief_output(&contributions)))
        })
    }
}

#[derive(Default)]
struct FakePersistence {
    values: Mutex<Vec<Value>>,
    tokens: Mutex<Vec<CancellationToken>>,
    wait_for_cancel: bool,
}

impl BriefPersistence for FakePersistence {
    fn persist<'a>(
        &'a self,
        brief: &'a CommandBrief,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<(), BriefPersistenceError>> {
        boxed(async move {
            self.tokens
                .lock()
                .expect("persistence token lock")
                .push(cancellation.clone());
            if self.wait_for_cancel {
                cancellation.cancelled().await;
            }
            if cancellation.is_cancelled() {
                return Err(BriefPersistenceError::Cancelled);
            }
            self.values
                .lock()
                .expect("persistence values lock")
                .push(serde_json::to_value(brief).expect("serialize brief"));
            Ok(())
        })
    }
}

fn request() -> CommandBriefRequest {
    CommandBriefRequest::new(
        "daily-command-brief",
        "Prepare the Daily Command Brief.",
        OBSERVED,
    )
    .expect("valid request")
}

fn orchestrator(
    capacity: u8,
    sources: Arc<dyn BriefSourceProvider>,
    advisers: Arc<dyn BriefAdviserProvider>,
    persistence: Arc<dyn BriefPersistence>,
) -> CommandBriefOrchestrator {
    CommandBriefOrchestrator::new(
        LocalModelScheduler::new(capacity).expect("scheduler"),
        sources,
        advisers,
        persistence,
    )
}

async fn wait_terminal(orchestrator: &CommandBriefOrchestrator, run_id: &str) -> BriefRunState {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let value =
                serde_json::to_value(orchestrator.status(run_id).expect("run status must exist"))
                    .expect("serialize status");
            let state: BriefRunState =
                serde_json::from_value(value["state"].clone()).expect("state");
            if matches!(
                state,
                BriefRunState::Completed
                    | BriefRunState::Degraded
                    | BriefRunState::Cancelled
                    | BriefRunState::Failed
            ) {
                return state;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("run reached terminal state")
}

#[tokio::test(flavor = "current_thread")]
async fn exact_five_specialists_share_snapshot_then_tool_free_chief_builds_nine_sections() {
    let sources = Arc::new(FakeSourceProvider::with_snapshots([SNAPSHOT_A]));
    let advisers = Arc::new(FakeAdviserProvider::default());
    let persistence = Arc::new(FakePersistence::default());
    let orchestrator = orchestrator(2, sources.clone(), advisers.clone(), persistence.clone());

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Completed
    );

    let calls = advisers.calls.lock().expect("calls lock").clone();
    assert_eq!(calls.len(), 6);
    assert_eq!(calls.last().map(String::as_str), Some("chief"));
    for adviser in SPECIALIST_ADVISERS {
        assert!(calls.contains(&format!("specialist:{adviser:?}")));
    }
    let snapshots = advisers.snapshots.lock().expect("snapshots lock").clone();
    assert_eq!(snapshots.len(), 5);
    assert!(snapshots.iter().all(|(_, snapshot)| snapshot == SNAPSHOT_A));

    let brief = orchestrator.result(&run_id).expect("completed brief");
    let value = serde_json::to_value(&brief).expect("serialize brief");
    assert_eq!(value["classification"], "OFFICIAL");
    assert_eq!(value["runId"], run_id);
    assert_eq!(value["snapshotId"], SNAPSHOT_A);
    assert_eq!(value["sections"].as_object().map(|map| map.len()), Some(9));
    assert_eq!(
        value["contributions"]
            .as_array()
            .expect("contributions")
            .iter()
            .map(|item| item["adviser"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("operations"),
            json!("navigation"),
            json!("daily_routine"),
            json!("reporting"),
            json!("plans"),
        ]
    );
    let expected_dissent = SPECIALIST_ADVISERS
        .into_iter()
        .map(|adviser| json!(format!("{run_id}:{adviser:?}:dissent")))
        .collect::<Vec<_>>();
    assert_eq!(
        value["dissent"].as_array().expect("dissent"),
        expected_dissent.as_slice()
    );
    assert_eq!(
        persistence.values.lock().expect("persistence lock").len(),
        1
    );
    assert!(sources
        .freeze_tokens
        .lock()
        .expect("freeze tokens")
        .iter()
        .all(|token| !token.is_cancelled()));
    assert!(advisers
        .tokens
        .lock()
        .expect("adviser tokens")
        .iter()
        .all(|token| !token.is_cancelled()));
}

#[tokio::test(flavor = "current_thread")]
async fn failed_adviser_becomes_visible_limitation_only_degradation_without_retry() {
    let sources = Arc::new(FakeSourceProvider::with_snapshots([SNAPSHOT_A]));
    let advisers = Arc::new(FakeAdviserProvider::default());
    advisers.fail_once(AdviserId::Navigation);
    let persistence = Arc::new(FakePersistence::default());
    let orchestrator = orchestrator(1, sources, advisers.clone(), persistence);

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Degraded
    );
    let value =
        serde_json::to_value(orchestrator.result(&run_id).expect("degraded brief")).expect("json");
    assert!(value["degradedSections"]
        .as_array()
        .expect("degraded")
        .contains(&json!("navigation")));
    assert!(value["missingInformation"]
        .as_array()
        .expect("missing")
        .contains(&json!("Navigation adviser output was unavailable.")));
    let navigation = value["contributions"]
        .as_array()
        .expect("contributions")
        .iter()
        .find(|item| item["adviser"] == "navigation")
        .expect("navigation placeholder");
    assert_eq!(navigation["findings"], json!([]));
    assert_eq!(
        navigation["limitations"],
        json!(["Navigation adviser output was unavailable."])
    );
    assert_eq!(
        advisers
            .calls
            .lock()
            .expect("calls")
            .iter()
            .filter(|call| call.as_str() == "specialist:Navigation")
            .count(),
        1
    );
}

#[tokio::test(flavor = "current_thread")]
async fn unsupported_chief_addition_fails_before_persistence() {
    let sources = Arc::new(FakeSourceProvider::with_snapshots([SNAPSHOT_A]));
    let advisers = Arc::new(FakeAdviserProvider::default());
    *advisers.chief_override.lock().expect("chief override lock") = Some(json!({
        "classification": "OFFICIAL",
        "adviser": "chief_of_staff",
        "findings": [{
            "classification": "OFFICIAL",
            "text": "Unsupported new claim",
            "sourceIds": ["ledger-will-be-rebound"]
        }],
        "limitations": [],
        "dissent": []
    }));
    let persistence = Arc::new(FakePersistence::default());
    let orchestrator = orchestrator(1, sources, advisers, persistence.clone());

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Failed
    );
    assert!(orchestrator.result(&run_id).is_none());
    assert!(persistence.values.lock().expect("persistence").is_empty());
    let status = serde_json::to_value(orchestrator.status(&run_id).expect("status")).expect("json");
    assert_eq!(status["error"], "chief_of_staff_output_rejected");
    assert!(!status.to_string().contains("Unsupported new claim"));
}

#[tokio::test(flavor = "current_thread")]
async fn one_snapshot_change_restarts_whole_run_second_change_fails() {
    let sources = Arc::new(FakeSourceProvider::with_snapshots([SNAPSHOT_A, SNAPSHOT_B]));
    sources.rechecks.lock().expect("rechecks").extend([
        Err(SourceCollectionError::SnapshotChanged),
        Ok(()),
        Ok(()),
    ]);
    let advisers = Arc::new(FakeAdviserProvider::default());
    let persistence = Arc::new(FakePersistence::default());
    let first_orchestrator = orchestrator(1, sources.clone(), advisers.clone(), persistence);

    let run_id = first_orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&first_orchestrator, &run_id).await,
        BriefRunState::Completed
    );
    let snapshots = advisers.snapshots.lock().expect("snapshots").clone();
    assert_eq!(snapshots.len(), 10);
    assert!(snapshots[..5]
        .iter()
        .all(|(_, snapshot)| snapshot == SNAPSHOT_A));
    assert!(snapshots[5..]
        .iter()
        .all(|(_, snapshot)| snapshot == SNAPSHOT_B));

    let failing_sources = Arc::new(FakeSourceProvider::with_snapshots([SNAPSHOT_A, SNAPSHOT_B]));
    failing_sources.rechecks.lock().expect("rechecks").extend([
        Err(SourceCollectionError::SnapshotChanged),
        Err(SourceCollectionError::SnapshotChanged),
    ]);
    let failing_advisers = Arc::new(FakeAdviserProvider::default());
    let failing = orchestrator(
        1,
        failing_sources,
        failing_advisers.clone(),
        Arc::new(FakePersistence::default()),
    );
    let failing_run = failing.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&failing, &failing_run).await,
        BriefRunState::Failed
    );
    assert_eq!(
        failing_advisers.snapshots.lock().expect("snapshots").len(),
        10
    );
    let status = serde_json::to_value(failing.status(&failing_run).expect("status")).expect("json");
    assert_eq!(status["error"], "snapshot_changed");
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_reaches_persistence_and_status_history_is_bounded_metadata_only() {
    let sources = Arc::new(FakeSourceProvider::with_snapshots([SNAPSHOT_A]));
    let advisers = Arc::new(FakeAdviserProvider::default());
    let persistence = Arc::new(FakePersistence {
        wait_for_cancel: true,
        ..FakePersistence::default()
    });
    let orchestrator = orchestrator(1, sources, advisers, persistence.clone());
    let run_id = orchestrator.start(request()).expect("run starts");

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if !persistence.tokens.lock().expect("tokens").is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("persistence reached");
    assert!(orchestrator.cancel(&run_id));
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Cancelled
    );
    assert!(persistence
        .tokens
        .lock()
        .expect("tokens")
        .iter()
        .all(CancellationToken::is_cancelled));
    assert!(orchestrator.result(&run_id).is_none());
    let history = orchestrator.history(&run_id);
    assert!(!history.is_empty());
    assert!(history.len() <= 32);
    let history_json = serde_json::to_string(&history).expect("serialize history");
    assert!(!history_json.contains("reasoning"));
    assert!(!history_json.contains("Verified source text"));
}

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
    ) -> Result<Value, SourceReadError> {
        unreachable!("a pre-cancelled collection must not reach the backend")
    }

    fn collect_memory(&self, _intent: &FixedRetrievalIntent) -> Result<Value, SourceReadError> {
        unreachable!("a pre-cancelled collection must not reach the backend")
    }

    fn collect_apple(&self, _request: &AppleInputRequest) -> AppleInputResponse {
        unreachable!("a pre-cancelled collection must not reach the backend")
    }

    fn recheck_rag_snapshot(
        &self,
        _expected: &VerifiedRagSnapshot,
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
