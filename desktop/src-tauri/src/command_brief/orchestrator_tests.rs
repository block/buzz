use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buzz_core_pkg::command_brief::{CommandBriefFailureCode, CommandBriefLifecycleState};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::command_services::apple_inputs::{
    AppleBriefSelection, AppleInputRequest, AppleInputResponse,
};
use crate::command_services::rag::VerifiedRagSnapshot;
use crate::command_services::trusted_lan::ModelRoutingPreference;

use super::cloud::CloudProvider;
use super::orchestrator::{provider_attempts, ProviderAttempt};
use super::orchestrator::{
    BriefAdviserError, BriefAdviserProvider, BriefFinalizationGate, BriefFuture, BriefPersistence,
    BriefSourceProvider, CollectedSourceProvider, CommandBriefOrchestrator, CommandBriefRequest,
    ReloadingSourceProvider, SourceBackendLoader,
};
use super::orchestrator_test_support::FakePersistence;
use super::provenance::ValidatedSource;
use super::scheduler::LocalModelScheduler;
use super::sources::{
    FixedRetrievalIntent, FrozenSourceContext, SourceBackend, SourceCollectionError,
    SourceReadError,
};
use super::types::{
    AdviserContribution, AdviserId, BriefRunState, BriefSection, SourceLedgerEntry,
    SPECIALIST_ADVISERS,
};

const SNAPSHOT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SNAPSHOT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const OBSERVED: &str = "2026-07-25T06:00:00+10:00";

#[test]
fn routing_preference_defines_one_exact_provider_order() {
    assert_eq!(
        provider_attempts(ModelRoutingPreference::CloudFirst),
        [
            ProviderAttempt::Cloud(CloudProvider::LiteLlm),
            ProviderAttempt::Cloud(CloudProvider::OpenAi),
            ProviderAttempt::Local,
        ]
    );
    assert_eq!(
        provider_attempts(ModelRoutingPreference::LocalFirst),
        [
            ProviderAttempt::Local,
            ProviderAttempt::Cloud(CloudProvider::LiteLlm),
            ProviderAttempt::Cloud(CloudProvider::OpenAi),
        ]
    );
}

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

fn trusted_source_context(run_id: &str) -> FrozenSourceContext {
    FrozenSourceContext::for_orchestrator_trusted_test(
        run_id,
        SNAPSHOT_A,
        OBSERVED,
        ledger(SNAPSHOT_A, run_id),
        Vec::new(),
        Vec::new(),
    )
}

fn section_for(adviser: AdviserId) -> BriefSection {
    match adviser {
        AdviserId::Operations => BriefSection::Operations,
        AdviserId::Intelligence => BriefSection::Intelligence,
        AdviserId::Logistics => BriefSection::Logistics,
        AdviserId::Navigation => BriefSection::Navigation,
        AdviserId::DailyRoutine => BriefSection::DailyRoutine,
        AdviserId::Reporting => BriefSection::Reports,
        AdviserId::Plans => BriefSection::Planning306090,
        AdviserId::ChiefOfStaff => panic!("chief is not a specialist"),
    }
}

fn contribution_with_limitations(
    adviser: AdviserId,
    source_id: &str,
    dissent: &str,
    limitations: Vec<String>,
) -> AdviserContribution {
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
            "limitations": limitations,
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
pub(super) struct FakeSourceProvider {
    snapshots: Mutex<VecDeque<&'static str>>,
    rechecks: Mutex<VecDeque<Result<(), SourceCollectionError>>>,
    freeze_tokens: Mutex<Vec<CancellationToken>>,
    recheck_tokens: Mutex<Vec<CancellationToken>>,
    pub(super) freeze_error: Mutex<Option<SourceCollectionError>>,
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

    pub(super) fn with_freeze_error(error: SourceCollectionError) -> Self {
        Self {
            freeze_error: Mutex::new(Some(error)),
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
            if let Some(error) = self.freeze_error.lock().expect("freeze error").clone() {
                return Err(error);
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
pub(super) struct FakeAdviserProvider {
    calls: Mutex<Vec<String>>,
    snapshots: Mutex<Vec<(AdviserId, String)>>,
    failures: Mutex<BTreeMap<AdviserId, usize>>,
    chief_override: Mutex<Option<Value>>,
    chief_limitations: Mutex<Option<Vec<String>>>,
    specialist_limitations: Mutex<BTreeMap<AdviserId, Vec<String>>>,
    chief_failures: AtomicUsize,
    tokens: Mutex<Vec<CancellationToken>>,
}

impl FakeAdviserProvider {
    fn fail_once(&self, adviser: AdviserId) {
        self.failures
            .lock()
            .expect("failure lock")
            .insert(adviser, 1);
    }

    fn set_specialist_limitations(&self, adviser: AdviserId, limitations: Vec<String>) {
        self.specialist_limitations
            .lock()
            .expect("specialist limitations lock")
            .insert(adviser, limitations);
    }

    fn fail_chief_once(&self) {
        self.chief_failures.store(1, Ordering::SeqCst);
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
            let limitations = self
                .specialist_limitations
                .lock()
                .expect("specialist limitations lock")
                .get(&adviser)
                .cloned()
                .unwrap_or_default();
            Ok(contribution_with_limitations(
                adviser,
                source_id,
                &format!("{run_id}:{adviser:?}:dissent"),
                limitations,
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
            if self
                .chief_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(BriefAdviserError::Failed);
            }
            let mut output = self
                .chief_override
                .lock()
                .expect("chief override lock")
                .clone()
                .unwrap_or_else(|| chief_output(&contributions));
            if let Some(limitations) = self
                .chief_limitations
                .lock()
                .expect("chief limitations lock")
                .clone()
            {
                output["limitations"] = json!(limitations);
            }
            Ok(output)
        })
    }
}

struct TrustedRecheckFailureProvider;

impl BriefSourceProvider for TrustedRecheckFailureProvider {
    fn freeze<'a>(
        &'a self,
        run_id: &'a str,
        _co_request: &'a str,
        _observed_at: &'a str,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<FrozenSourceContext, SourceCollectionError>> {
        boxed(async move {
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            Ok(trusted_source_context(run_id))
        })
    }

    fn recheck<'a>(
        &'a self,
        _context: &'a FrozenSourceContext,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<(), SourceCollectionError>> {
        boxed(async move {
            if cancellation.is_cancelled() {
                Err(SourceCollectionError::Cancelled)
            } else {
                Err(SourceCollectionError::RagUnavailable)
            }
        })
    }
}

pub(super) fn request() -> CommandBriefRequest {
    CommandBriefRequest::new(
        "daily-command-brief",
        "Prepare the Daily Command Brief.",
        OBSERVED,
    )
    .expect("valid request")
}

pub(super) fn orchestrator(
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

pub(super) async fn wait_terminal(
    orchestrator: &CommandBriefOrchestrator,
    run_id: &str,
) -> BriefRunState {
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
async fn exact_seven_specialists_share_snapshot_then_tool_free_chief_builds_eleven_sections() {
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
    assert_eq!(calls.len(), 8);
    assert_eq!(calls.last().map(String::as_str), Some("chief"));
    for adviser in SPECIALIST_ADVISERS {
        assert!(calls.contains(&format!("specialist:{adviser:?}")));
    }
    let snapshots = advisers.snapshots.lock().expect("snapshots lock").clone();
    assert_eq!(snapshots.len(), 7);
    assert!(snapshots.iter().all(|(_, snapshot)| snapshot == SNAPSHOT_A));

    let brief = orchestrator.result(&run_id).expect("completed brief");
    let value = serde_json::to_value(brief.brief()).expect("serialize brief");
    assert_eq!(value["classification"], "OFFICIAL");
    assert_eq!(value["runId"], run_id);
    assert_eq!(value["snapshotId"], SNAPSHOT_A);
    assert_eq!(value["sections"].as_object().map(|map| map.len()), Some(11));
    assert_eq!(
        value["contributions"]
            .as_array()
            .expect("contributions")
            .iter()
            .map(|item| item["adviser"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("operations"),
            json!("intelligence"),
            json!("logistics"),
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
    let orchestrator = orchestrator(1, sources, advisers.clone(), persistence.clone());

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Degraded
    );
    let value = serde_json::to_value(
        orchestrator
            .result(&run_id)
            .expect("degraded brief")
            .brief(),
    )
    .expect("json");
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
    persistence.assert_one_terminal(CommandBriefLifecycleState::Degraded, None);
}

#[tokio::test(flavor = "current_thread")]
async fn failed_chief_model_preserves_specialists_in_a_degraded_brief() {
    let sources = Arc::new(FakeSourceProvider::with_snapshots([SNAPSHOT_A]));
    let advisers = Arc::new(FakeAdviserProvider::default());
    advisers.fail_chief_once();
    let persistence = Arc::new(FakePersistence::default());
    let orchestrator = orchestrator(1, sources, advisers, persistence.clone());

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Degraded
    );
    let brief =
        serde_json::to_value(orchestrator.result(&run_id).expect("brief").brief()).expect("json");
    assert_eq!(
        brief["contributions"]
            .as_array()
            .expect("contributions")
            .len(),
        7
    );
    assert!(!brief["sections"]["today"]
        .as_array()
        .expect("today")
        .is_empty());
    assert!(brief["missingInformation"]
        .as_array()
        .expect("limitations")
        .contains(&json!(
            "Chief of Staff model consolidation was unavailable; the brief was consolidated deterministically from validated specialist advice."
        )));
    persistence.assert_one_terminal(CommandBriefLifecycleState::Degraded, None);
}

#[tokio::test(flavor = "current_thread")]
async fn trusted_lan_recheck_failure_keeps_already_collected_evidence() {
    let persistence = Arc::new(FakePersistence::default());
    let orchestrator = orchestrator(
        1,
        Arc::new(TrustedRecheckFailureProvider),
        Arc::new(FakeAdviserProvider::default()),
        persistence.clone(),
    );

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Degraded
    );
    let brief =
        serde_json::to_value(orchestrator.result(&run_id).expect("brief").brief()).expect("json");
    assert_eq!(brief["sourceLedger"].as_array().expect("ledger").len(), 1);
    assert!(brief["missingInformation"]
        .as_array()
        .expect("limitations")
        .contains(&json!(
            "Trusted-LAN source recheck was unavailable after evidence collection; the brief retains the cited evidence already collected."
        )));
    persistence.assert_one_terminal(CommandBriefLifecycleState::Degraded, None);
}

#[tokio::test(flavor = "current_thread")]
async fn unsupported_chief_addition_is_discarded_and_uses_safe_fallback() {
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
        BriefRunState::Degraded
    );
    let brief =
        serde_json::to_value(orchestrator.result(&run_id).expect("brief").brief()).expect("json");
    assert!(!brief.to_string().contains("Unsupported new claim"));
    assert_eq!(
        brief["contributions"]
            .as_array()
            .expect("contributions")
            .len(),
        7
    );
    persistence.assert_one_terminal(CommandBriefLifecycleState::Degraded, None);
    let status = serde_json::to_value(orchestrator.status(&run_id).expect("status")).expect("json");
    assert_eq!(status["state"], "degraded");
    assert!(!status.to_string().contains("Unsupported new claim"));
}

#[tokio::test]
async fn unsupported_chief_limitation_is_discarded_and_uses_safe_fallback() {
    let sources = Arc::new(FakeSourceProvider::with_snapshots([SNAPSHOT_A]));
    let advisers = Arc::new(FakeAdviserProvider::default());
    *advisers
        .chief_limitations
        .lock()
        .expect("chief limitations") = Some(vec!["Invented source gap.".to_string()]);
    let persistence = Arc::new(FakePersistence::default());
    let orchestrator = orchestrator(1, sources, advisers, persistence.clone());

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Degraded
    );
    let brief =
        serde_json::to_value(orchestrator.result(&run_id).expect("brief").brief()).expect("json");
    assert!(!brief.to_string().contains("Invented source gap"));
    persistence.assert_one_terminal(CommandBriefLifecycleState::Degraded, None);
    let status = serde_json::to_value(orchestrator.status(&run_id).expect("status")).expect("json");
    assert_eq!(status["state"], "degraded");
    assert!(!status.to_string().contains("Invented source gap"));
}

#[tokio::test]
async fn exact_trusted_source_and_specialist_limitations_are_valid_chief_subset() {
    let sources = Arc::new(FakeSourceProvider {
        snapshots: Mutex::new(VecDeque::from([SNAPSHOT_A])),
        limitations: vec!["Trusted source gap.".to_string()],
        ..FakeSourceProvider::default()
    });
    let advisers = Arc::new(FakeAdviserProvider::default());
    advisers.fail_once(AdviserId::Navigation);
    *advisers
        .chief_limitations
        .lock()
        .expect("chief limitations") = Some(vec![
        "Trusted source gap.".to_string(),
        "Navigation adviser output was unavailable.".to_string(),
    ]);
    let persistence = Arc::new(FakePersistence::default());
    let orchestrator = orchestrator(1, sources, advisers, persistence.clone());

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Degraded
    );
    let brief =
        serde_json::to_value(orchestrator.result(&run_id).expect("brief").brief()).expect("json");
    assert_eq!(
        brief["missingInformation"],
        json!([
            "Navigation adviser output was unavailable.",
            "Trusted source gap."
        ])
    );
    assert_eq!(persistence.values.lock().expect("persistence").len(), 1);
}

#[tokio::test]
async fn missing_information_prioritizes_failure_and_specialist_before_64_source_limitations() {
    let source_limitations = (0..64)
        .map(|index| format!("Source gap {index:02}."))
        .collect::<Vec<_>>();
    let sources = Arc::new(FakeSourceProvider {
        snapshots: Mutex::new(VecDeque::from([SNAPSHOT_A])),
        limitations: source_limitations,
        ..FakeSourceProvider::default()
    });
    let advisers = Arc::new(FakeAdviserProvider::default());
    advisers.fail_once(AdviserId::Navigation);
    advisers.set_specialist_limitations(
        AdviserId::Operations,
        vec!["Operations specialist gap.".to_string()],
    );
    let orchestrator = orchestrator(1, sources, advisers, Arc::new(FakePersistence::default()));

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Degraded
    );
    let brief =
        serde_json::to_value(orchestrator.result(&run_id).expect("brief").brief()).expect("json");
    let missing = brief["missingInformation"]
        .as_array()
        .expect("missing information");
    assert_eq!(missing.len(), 64);
    assert_eq!(
        missing.first(),
        Some(&json!("Navigation adviser output was unavailable."))
    );
    assert_eq!(missing.get(1), Some(&json!("Operations specialist gap.")));
    assert_eq!(
        missing.last(),
        Some(&json!(
            "3 additional trusted limitations omitted after the canonical limit."
        ))
    );
    assert!(missing.contains(&json!("Source gap 60.")));
    assert!(!missing.contains(&json!("Source gap 61.")));
    assert!(!missing.contains(&json!("Source gap 62.")));
    assert!(!missing.contains(&json!("Source gap 63.")));
}

#[tokio::test]
async fn missing_information_boundary_is_exact_and_omission_summary_is_not_silent() {
    for (source_count, expected_omitted) in [(62, None), (63, Some(2usize))] {
        let sources = Arc::new(FakeSourceProvider {
            snapshots: Mutex::new(VecDeque::from([SNAPSHOT_A])),
            limitations: (0..source_count)
                .map(|index| format!("Source gap {index:02}."))
                .collect(),
            ..FakeSourceProvider::default()
        });
        let advisers = Arc::new(FakeAdviserProvider::default());
        advisers.fail_once(AdviserId::Navigation);
        advisers.set_specialist_limitations(
            AdviserId::Operations,
            vec!["Operations specialist gap.".to_string()],
        );
        let orchestrator = orchestrator(1, sources, advisers, Arc::new(FakePersistence::default()));

        let run_id = orchestrator.start(request()).expect("run starts");
        assert_eq!(
            wait_terminal(&orchestrator, &run_id).await,
            BriefRunState::Degraded
        );
        let brief = serde_json::to_value(orchestrator.result(&run_id).expect("brief").brief())
            .expect("json");
        let missing = brief["missingInformation"]
            .as_array()
            .expect("missing information");
        assert_eq!(missing.len(), 64);
        assert_eq!(
            missing.first(),
            Some(&json!("Navigation adviser output was unavailable."))
        );
        assert_eq!(missing.get(1), Some(&json!("Operations specialist gap.")));
        match expected_omitted {
            None => assert_eq!(missing.last(), Some(&json!("Source gap 61."))),
            Some(omitted) => assert_eq!(
                missing.last(),
                Some(&json!(format!(
                    "{omitted} additional trusted limitations omitted after the canonical limit."
                )))
            ),
        }
    }
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
    assert_eq!(snapshots.len(), 14);
    assert!(snapshots[..7]
        .iter()
        .all(|(_, snapshot)| snapshot == SNAPSHOT_A));
    assert!(snapshots[7..]
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
        14
    );
    let status = serde_json::to_value(failing.status(&failing_run).expect("status")).expect("json");
    assert_eq!(status["error"], "snapshot_changed");
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_reaches_persistence_and_status_history_is_bounded_metadata_only() {
    let sources = Arc::new(FakeSourceProvider::with_snapshots([SNAPSHOT_A]));
    let advisers = Arc::new(FakeAdviserProvider::default());
    let persistence = Arc::new(FakePersistence::waiting_for_cancel());
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
    persistence.assert_one_terminal(
        CommandBriefLifecycleState::Cancelled,
        Some(CommandBriefFailureCode::CancellationRequested),
    );
}

#[path = "orchestrator_tests/backend_and_cancellation.rs"]
mod backend_and_cancellation;
