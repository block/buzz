//! Explicitly opted-in live acceptance against the installed LM Studio runtime.
//! These tests use the production adviser executor, personas, structured-output
//! parser, and capacity-one scheduler; they do not issue synthetic role prompts.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::lmstudio::{
    AdviserExecutionError, AdviserExecutionResult, AdviserExecutor, SpecialistAdviserRequest,
};
use super::provenance::ValidatedSource;
use super::scheduler::{
    LocalModelScheduler, SchedulerError, SchedulerJobKey, SchedulerLifecycleState,
};
use super::types::{AdviserContribution, AdviserId, SourceLedgerEntry};
use crate::command_services::policy::build_adviser_runtime_catalog;

const SNAPSHOT: &str = "f8bb8f8d2f046a82137f1ebc01f41fb370f3a330992bce8a7a4b6160c3ef3f07";
const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:1234";
const DEFAULT_INSTANCE: &str = "gemma4-26b-official";

type LiveResult =
    Result<AdviserExecutionResult<AdviserContribution>, SchedulerError<AdviserExecutionError>>;

fn live_acceptance_enabled() -> bool {
    std::env::var("BUZZ_LIVE_LMSTUDIO_ACCEPTANCE").as_deref() == Ok("1")
}

fn source() -> ValidatedSource {
    SourceLedgerEntry::parse_for_snapshot(
        json!({
            "classification": "OFFICIAL",
            "ledgerId": "ledger-live-collaboration",
            "sourceKind": "rag",
            "sourceId": "point-live-collaboration",
            "collection": "acceptance",
            "documentId": "departure-readiness",
            "chunkId": "maintenance-confirmation",
            "timestamp": "2026-08-16T00:00:00Z",
            "snapshotId": SNAPSHOT,
            "observedAt": "2026-08-16T00:00:00Z",
            "retrievedAt": "2026-08-16T00:00:00Z",
            "quotedLocation": {
                "location": "acceptance fixture",
                "quote": "Departure is in 48 hours and one maintenance dependency remains unconfirmed."
            }
        }),
        SNAPSHOT,
    )
    .expect("valid live acceptance source")
    .into()
}

async fn wait_until_queued(scheduler: &LocalModelScheduler, run_id: &str, adviser: AdviserId) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if scheduler.lifecycle_history().iter().any(|event| {
                event.key().run_id() == run_id
                    && event.key().adviser() == adviser
                    && event.state() == SchedulerLifecycleState::Queued
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("scheduler did not record queued adviser");
}

async fn run_case(
    executor: Arc<AdviserExecutor>,
    adviser_count: usize,
) -> Vec<AdviserExecutionResult<AdviserContribution>> {
    let advisers = [
        AdviserId::Operations,
        AdviserId::Intelligence,
        AdviserId::Logistics,
    ];
    let scheduler = LocalModelScheduler::sequential();
    let run_id = format!("live-collaboration-{adviser_count}");
    let mut handles: Vec<(AdviserId, JoinHandle<LiveResult>)> = Vec::new();

    for adviser in advisers.into_iter().take(adviser_count) {
        let job_scheduler = scheduler.clone();
        let job_executor = Arc::clone(&executor);
        let job_run_id = run_id.clone();
        let key = SchedulerJobKey::new(&run_id, adviser).expect("valid live job key");
        let request =
            SpecialistAdviserRequest::new(format!("{run_id}:{adviser:?}"), adviser, vec![source()]);
        let handle = tokio::spawn(async move {
            job_scheduler
                .schedule(key, CancellationToken::new(), move |token| async move {
                    job_executor.run_specialist(request, token).await
                })
                .await
        });
        handles.push((adviser, handle));
        wait_until_queued(&scheduler, &job_run_id, adviser).await;
    }

    let mut results = Vec::with_capacity(adviser_count);
    for (expected_adviser, handle) in handles {
        let result = handle
            .await
            .expect("live adviser task joined")
            .expect("live adviser execution passed");
        assert_eq!(result.contribution.adviser(), expected_adviser);
        assert_eq!(result.model_instance_id, DEFAULT_INSTANCE);
        assert_eq!(result.token_counts.reasoning, 0);
        results.push(result);
    }

    let running_order = scheduler
        .lifecycle_history()
        .into_iter()
        .filter(|event| event.state() == SchedulerLifecycleState::Running)
        .map(|event| event.key().adviser())
        .collect::<Vec<_>>();
    assert_eq!(running_order, advisers[..adviser_count]);
    assert_eq!(scheduler.capacity(), 1);
    assert_eq!(scheduler.active_job_count(), 0);
    results
}

#[tokio::test]
#[ignore = "requires BUZZ_LIVE_LMSTUDIO_ACCEPTANCE=1 and the qualified local runtime"]
async fn one_to_three_advisers_use_the_real_capacity_one_product_path() {
    assert!(
        live_acceptance_enabled(),
        "set BUZZ_LIVE_LMSTUDIO_ACCEPTANCE=1 to run this live test"
    );
    let endpoint = std::env::var("BUZZ_LIVE_LMSTUDIO_ENDPOINT")
        .unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string());
    let instance = std::env::var("BUZZ_LIVE_LMSTUDIO_INSTANCE")
        .unwrap_or_else(|_| DEFAULT_INSTANCE.to_string());
    assert_eq!(instance, DEFAULT_INSTANCE);
    let catalog = build_adviser_runtime_catalog(&[], &endpoint, None)
        .expect("build live local-only adviser catalog");
    let executor = Arc::new(
        AdviserExecutor::new(instance, catalog, Duration::from_secs(900))
            .expect("build production adviser executor"),
    );

    for adviser_count in 1..=3 {
        let results = run_case(Arc::clone(&executor), adviser_count).await;
        assert_eq!(results.len(), adviser_count);
        println!(
            "live Command Adviser collaboration: advisers={adviser_count} instance={DEFAULT_INSTANCE} capacity=1 result=pass"
        );
    }
}
