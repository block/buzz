use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use tokio_util::sync::CancellationToken;

use super::{
    dispatch_readiness_with_retry, model_readiness_for_schedule, observe_readiness_transition,
    readiness_transition_token, start_model_readiness_observer_with_poll, timer_claim_fast_path,
    trusted_model_readiness_observation, CommandBriefReadinessTransitions, CommandBriefRuntimeSet,
    InstalledCommandBriefRuntime, ReadinessSignalSource, RuntimeConfigIdentity, RuntimeReadiness,
};
use crate::command_brief::audit::{PersistedTerminal, TerminalAuditInput};
use crate::command_brief::orchestrator::{
    BriefAdviserError, BriefAdviserProvider, BriefFuture, BriefPersistence, BriefPersistenceError,
    BriefSourceProvider, CommandBriefOrchestrator, CommandBriefRequest, OrchestratorAdmissionState,
    OrchestratorStartError,
};
use crate::command_brief::provenance::ValidatedSource;
use crate::command_brief::schedule::{
    acquire_due_claim, load_or_create_schedule, mark_claim_started, process_due_schedule,
    ClaimDecision, DeferredReason, ReadinessSnapshot, ScheduleRunOutcome, ScheduleTrigger,
    ScheduledRunPresence, ScheduledRunStarter, ScheduledStartError,
};
use crate::command_brief::scheduler::LocalModelScheduler;
use crate::command_brief::sources::{FrozenSourceContext, SourceCollectionError};
use crate::command_brief::types::{AdviserContribution, AdviserId};
use crate::commands::{LmStudioReadiness, LmStudioReadinessState};

struct UnusedProvider;

struct DropSignal(Arc<AtomicUsize>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct RecordingStarter {
    starts: Mutex<Vec<String>>,
}

impl ScheduledRunStarter for RecordingStarter {
    fn start_scheduled(
        &self,
        run_id: &str,
        _idempotency_key: &str,
        _schedule_id: &str,
        _observed_at: &str,
    ) -> Result<String, ScheduledStartError> {
        self.starts.lock().expect("starts").push(run_id.to_string());
        Ok(run_id.to_string())
    }

    fn presence(&self, _run_id: &str) -> ScheduledRunPresence {
        ScheduledRunPresence::Absent
    }
}

impl BriefSourceProvider for UnusedProvider {
    fn freeze<'a>(
        &'a self,
        _run_id: &'a str,
        _co_request: &'a str,
        _observed_at: &'a str,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<FrozenSourceContext, SourceCollectionError>> {
        Box::pin(async move {
            cancellation.cancelled().await;
            Err(SourceCollectionError::Cancelled)
        })
    }

    fn recheck<'a>(
        &'a self,
        _context: &'a FrozenSourceContext,
        _cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<(), SourceCollectionError>> {
        Box::pin(async { Err(SourceCollectionError::RagUnavailable) })
    }
}

impl BriefAdviserProvider for UnusedProvider {
    fn run_specialist<'a>(
        &'a self,
        _run_id: &'a str,
        _adviser: AdviserId,
        _sources: Vec<ValidatedSource>,
        _cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<AdviserContribution, BriefAdviserError>> {
        Box::pin(async { Err(BriefAdviserError::Failed) })
    }

    fn run_chief_of_staff<'a>(
        &'a self,
        _run_id: &'a str,
        _contributions: Vec<AdviserContribution>,
        _source_ledger: Vec<ValidatedSource>,
        _cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<serde_json::Value, BriefAdviserError>> {
        Box::pin(async { Err(BriefAdviserError::Failed) })
    }
}

impl BriefPersistence for UnusedProvider {
    fn persist_terminal<'a>(
        &'a self,
        _input: TerminalAuditInput,
        _cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<PersistedTerminal, BriefPersistenceError>> {
        Box::pin(async { Err(BriefPersistenceError::Failed) })
    }
}

fn identity(model: &str, snapshot: &str, config: &str, capacity: u8) -> RuntimeConfigIdentity {
    RuntimeConfigIdentity::new_for_test(
        "owner-pubkey",
        model,
        snapshot,
        config,
        capacity,
        "policy-v1",
    )
}

fn utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("rfc3339")
        .with_timezone(&Utc)
}

#[test]
fn completed_or_started_timer_ticks_take_the_zero_probe_fast_path() {
    for state in ["started", "completed"] {
        let conn = Connection::open_in_memory().expect("memory");
        crate::command_brief::store::migrate_command_brief_store(&conn).expect("migrate");
        let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
        let now = utc("2026-07-25T01:00:00Z");
        let date = now
            .with_timezone(&chrono_tz::Australia::Sydney)
            .date_naive();
        let ClaimDecision::Acquired(claim) =
            acquire_due_claim(&conn, &schedule, date, 1).expect("claim")
        else {
            panic!("claim");
        };
        mark_claim_started(&conn, &claim, claim.run_id(), 2).expect("started");
        if state == "completed" {
            conn.execute(
                "UPDATE command_brief_schedule_claims SET state='completed' WHERE run_id=?1",
                [claim.run_id()],
            )
            .expect("completed");
        }

        let mut expensive_probe_counts = [0_u8; 5];
        for _ in 0..10 {
            let fast = timer_claim_fast_path(&conn, &schedule, now, ScheduleTrigger::Timer, || {
                panic!("started/completed must not inspect admission")
            })
            .expect("fast path");
            if fast.is_none() {
                for counter in &mut expensive_probe_counts {
                    *counter += 1;
                }
            }
            assert_eq!(fast, Some(ScheduleRunOutcome::AlreadyClaimed));
        }
        assert_eq!(
            expensive_probe_counts,
            [0, 0, 0, 0, 0],
            "LM discovery, RAG, Apple config, helper identity, and integrity stay untouched"
        );
        assert_eq!(
            timer_claim_fast_path(&conn, &schedule, now, ScheduleTrigger::Startup, || None)
                .expect("startup"),
            None,
            "startup retains durable crash reconciliation"
        );
    }
}

#[test]
fn actual_knowledge_and_model_observers_recover_once_without_unchanged_dispatch() {
    let conn = Connection::open_in_memory().expect("memory");
    crate::command_brief::store::migrate_command_brief_store(&conn).expect("migrate");
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let now = utc("2026-07-25T01:00:00Z");
    let starter = RecordingStarter::default();
    let mut transitions = CommandBriefReadinessTransitions::default();

    assert!(observe_readiness_transition(
        &mut transitions,
        ReadinessSignalSource::Knowledge,
        "knowledge:rag-unavailable",
    ));
    assert_eq!(
        process_due_schedule(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Startup,
            &ReadinessSnapshot::deferred(
                DeferredReason::LocalStateUnavailable,
                "knowledge:rag-unavailable",
            ),
            &starter,
        )
        .expect("defer RAG"),
        ScheduleRunOutcome::Deferred {
            reason: DeferredReason::LocalStateUnavailable,
        }
    );
    assert!(!observe_readiness_transition(
        &mut transitions,
        ReadinessSignalSource::Knowledge,
        "knowledge:rag-unavailable",
    ));
    assert!(starter.starts.lock().expect("starts").is_empty());

    assert!(observe_readiness_transition(
        &mut transitions,
        ReadinessSignalSource::Knowledge,
        "knowledge:rag-ready",
    ));
    assert!(matches!(
        process_due_schedule(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Readiness,
            &ReadinessSnapshot::ready("knowledge:rag-ready"),
            &starter,
        )
        .expect("RAG transition retry"),
        ScheduleRunOutcome::Started { .. }
    ));
    assert_eq!(starter.starts.lock().expect("starts").len(), 1);

    let mut model_transitions = CommandBriefReadinessTransitions::default();
    assert!(observe_readiness_transition(
        &mut model_transitions,
        ReadinessSignalSource::Model,
        "model:no-loaded-model",
    ));
    assert!(!observe_readiness_transition(
        &mut model_transitions,
        ReadinessSignalSource::Model,
        "model:no-loaded-model",
    ));
    assert!(observe_readiness_transition(
        &mut model_transitions,
        ReadinessSignalSource::Model,
        "model:qwen-ready",
    ));
    assert!(!observe_readiness_transition(
        &mut model_transitions,
        ReadinessSignalSource::Model,
        "model:qwen-ready",
    ));
}

#[test]
fn timer_checks_admission_only_for_admission_unavailable_claims() {
    for reason in [
        DeferredReason::IdentityLocked,
        DeferredReason::ModelUnavailable,
        DeferredReason::LocalStateUnavailable,
    ] {
        let conn = Connection::open_in_memory().expect("memory");
        crate::command_brief::store::migrate_command_brief_store(&conn).expect("migrate");
        let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
        let now = utc("2026-07-25T01:00:00Z");
        process_due_schedule(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Startup,
            &ReadinessSnapshot::deferred(reason, "stable:non-admission"),
            &RecordingStarter::default(),
        )
        .expect("deferred");

        for _ in 0..10 {
            assert_eq!(
                timer_claim_fast_path(&conn, &schedule, now, ScheduleTrigger::Timer, || panic!(
                    "non-admission deferral must not inspect runtime admission"
                ),)
                .expect("timer fast path"),
                Some(ScheduleRunOutcome::AlreadyClaimed)
            );
        }
        let retry_count: i64 = conn
            .query_row(
                "SELECT retry_count FROM command_brief_schedule_claims",
                [],
                |row| row.get(0),
            )
            .expect("retry count");
        assert_eq!(retry_count, 0);
    }

    let conn = Connection::open_in_memory().expect("memory");
    crate::command_brief::store::migrate_command_brief_store(&conn).expect("migrate");
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let now = utc("2026-07-25T01:00:00Z");
    process_due_schedule(
        &conn,
        &schedule,
        now,
        ScheduleTrigger::Startup,
        &ReadinessSnapshot::deferred(
            DeferredReason::AdmissionUnavailable,
            "admission:generation-7:64/64",
        ),
        &RecordingStarter::default(),
    )
    .expect("admission deferred");
    assert_eq!(
        timer_claim_fast_path(&conn, &schedule, now, ScheduleTrigger::Timer, || Some(
            "admission:generation-7:64/64".to_string(),
        ))
        .expect("unchanged admission"),
        Some(ScheduleRunOutcome::AlreadyClaimed)
    );
    assert_eq!(
        timer_claim_fast_path(&conn, &schedule, now, ScheduleTrigger::Timer, || Some(
            "admission:generation-7:63/64".to_string(),
        ))
        .expect("changed admission"),
        None
    );
}

#[tokio::test]
async fn app_owned_model_observer_recovers_without_ui_and_dedupes_unchanged_polls() {
    let directory = tempfile::tempdir().expect("temp");
    let store_path = directory.path().join("brief.db");
    let conn = crate::command_brief::store::open_command_brief_store(&store_path).expect("store");
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let now = utc("2026-07-25T01:00:00Z");
    let unavailable_token = readiness_transition_token(b"model:no-loaded-model");
    process_due_schedule(
        &conn,
        &schedule,
        now,
        ScheduleTrigger::Startup,
        &ReadinessSnapshot::deferred(DeferredReason::ModelUnavailable, &unavailable_token),
        &RecordingStarter::default(),
    )
    .expect("initial model deferral");
    drop(conn);

    let observations = Arc::new(Mutex::new(VecDeque::from([
        "model:no-loaded-model",
        "model:no-loaded-model",
        "model:qwen-ready",
        "model:qwen-ready",
    ])));
    let transitions = Arc::new(Mutex::new(CommandBriefReadinessTransitions::default()));
    let starter = Arc::new(RecordingStarter::default());
    let polls = Arc::new(AtomicUsize::new(0));
    let schedule_dispatches = Arc::new(AtomicUsize::new(0));
    let rag_preflights = Arc::new(AtomicUsize::new(0));

    let observer = start_model_readiness_observer_with_poll(Duration::from_millis(5), {
        let observations = Arc::clone(&observations);
        let transitions = Arc::clone(&transitions);
        let starter = Arc::clone(&starter);
        let polls = Arc::clone(&polls);
        let schedule_dispatches = Arc::clone(&schedule_dispatches);
        let rag_preflights = Arc::clone(&rag_preflights);
        let store_path = store_path.clone();
        move || {
            let observations = Arc::clone(&observations);
            let transitions = Arc::clone(&transitions);
            let starter = Arc::clone(&starter);
            let polls = Arc::clone(&polls);
            let schedule_dispatches = Arc::clone(&schedule_dispatches);
            let rag_preflights = Arc::clone(&rag_preflights);
            let store_path = store_path.clone();
            async move {
                polls.fetch_add(1, Ordering::SeqCst);
                let observation = observations
                    .lock()
                    .expect("observations")
                    .pop_front()
                    .unwrap_or("model:qwen-ready");
                let token = readiness_transition_token(observation.as_bytes());
                let changed = observe_readiness_transition(
                    &mut transitions.lock().expect("transitions"),
                    ReadinessSignalSource::Model,
                    &token,
                );
                if !changed {
                    return;
                }
                schedule_dispatches.fetch_add(1, Ordering::SeqCst);
                let conn = crate::command_brief::store::open_command_brief_store(&store_path)
                    .expect("reopen");
                let schedule =
                    load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
                let readiness = if observation == "model:qwen-ready" {
                    rag_preflights.fetch_add(1, Ordering::SeqCst);
                    ReadinessSnapshot::ready(&token)
                } else {
                    ReadinessSnapshot::deferred(DeferredReason::ModelUnavailable, &token)
                };
                process_due_schedule(
                    &conn,
                    &schedule,
                    now,
                    ScheduleTrigger::Readiness,
                    &readiness,
                    starter.as_ref(),
                )
                .expect("observer dispatch");
            }
        }
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while polls.load(Ordering::SeqCst) < 6 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    observer.stop();
    let stopped_at = polls.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(20)).await;

    assert!(stopped_at >= 4, "observer must poll without a mounted UI");
    assert_eq!(polls.load(Ordering::SeqCst), stopped_at);
    assert_eq!(schedule_dispatches.load(Ordering::SeqCst), 2);
    assert_eq!(rag_preflights.load(Ordering::SeqCst), 1);
    assert_eq!(starter.starts.lock().expect("starts").len(), 1);
    let conn = crate::command_brief::store::open_command_brief_store(&store_path).expect("reopen");
    let (state, retry_count): (String, i64) = conn
        .query_row(
            "SELECT state,retry_count FROM command_brief_schedule_claims",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("claim");
    assert_eq!(state, "started");
    assert_eq!(retry_count, 1);
}

#[tokio::test]
async fn supplied_trusted_model_observation_starts_once_without_a_second_probe() {
    let probes = Arc::new(AtomicUsize::new(1));
    let observation = trusted_model_readiness_observation(Ok(LmStudioReadiness {
        status: LmStudioReadinessState::Ready,
        detail: "ready".to_string(),
        configured_model: Some("qwen-command".to_string()),
        loaded_models: vec!["qwen-command".to_string()],
        security_warnings: vec!["bind exposure unverified".to_string()],
        bind_exposure: "unknown",
    }));
    let selected = model_readiness_for_schedule(Some(observation.clone()), {
        let probes = Arc::clone(&probes);
        move || {
            let probes = Arc::clone(&probes);
            async move {
                probes.fetch_add(1, Ordering::SeqCst);
                Err("second probe must stay unreachable".to_string())
            }
        }
    })
    .await;

    assert_eq!(probes.load(Ordering::SeqCst), 1);
    assert_eq!(selected.transition_token(), observation.transition_token());
    assert_eq!(
        selected
            .readiness()
            .and_then(|readiness| readiness.configured_model.as_deref()),
        Some("qwen-command")
    );

    let conn = Connection::open_in_memory().expect("memory");
    crate::command_brief::store::migrate_command_brief_store(&conn).expect("migrate");
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let starter = RecordingStarter::default();
    assert!(matches!(
        process_due_schedule(
            &conn,
            &schedule,
            utc("2026-07-25T01:00:00Z"),
            ScheduleTrigger::Readiness,
            &ReadinessSnapshot::ready(selected.transition_token()),
            &starter,
        )
        .expect("start from supplied observation"),
        ScheduleRunOutcome::Started { .. }
    ));
    assert_eq!(starter.starts.lock().expect("starts").len(), 1);
}

#[tokio::test]
async fn dispatch_marks_handled_after_success_and_retries_one_infrastructure_failure() {
    let transitions = Arc::new(Mutex::new(CommandBriefReadinessTransitions::default()));
    let token = readiness_transition_token(b"model:qwen-ready");
    let attempts = Arc::new(AtomicUsize::new(0));
    let second_started = Arc::new(tokio::sync::Notify::new());
    let release_second = Arc::new(tokio::sync::Notify::new());
    let dispatch = tokio::spawn(dispatch_readiness_with_retry(
        Arc::clone(&transitions),
        ReadinessSignalSource::Model,
        token.clone(),
        Duration::from_millis(5),
        {
            let attempts = Arc::clone(&attempts);
            let second_started = Arc::clone(&second_started);
            let release_second = Arc::clone(&release_second);
            move || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                let second_started = Arc::clone(&second_started);
                let release_second = Arc::clone(&release_second);
                async move {
                    if attempt == 0 {
                        return Err("dispatch unavailable");
                    }
                    second_started.notify_one();
                    release_second.notified().await;
                    Ok(ScheduleRunOutcome::Started {
                        run_id: "scheduled-test".to_string(),
                    })
                }
            }
        },
    ));

    second_started.notified().await;
    {
        let transitions = transitions.lock().expect("transitions");
        assert!(!transitions.is_handled(ReadinessSignalSource::Model, &token));
        assert!(transitions.is_in_flight(ReadinessSignalSource::Model, &token));
    }
    release_second.notify_one();
    assert!(dispatch.await.expect("join"));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    {
        let transitions = transitions.lock().expect("transitions");
        assert!(transitions.is_handled(ReadinessSignalSource::Model, &token));
        assert!(!transitions.is_in_flight(ReadinessSignalSource::Model, &token));
        assert!(!transitions.is_handled(ReadinessSignalSource::Knowledge, &token));
    }

    let repeated = dispatch_readiness_with_retry(
        Arc::clone(&transitions),
        ReadinessSignalSource::Model,
        token,
        Duration::from_millis(5),
        || async {
            panic!("handled observation must not retry or run heavy probes");
            #[allow(unreachable_code)]
            Ok(ScheduleRunOutcome::NotDue)
        },
    )
    .await;
    assert!(!repeated);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn model_observer_stop_cancels_an_inflight_probe() {
    let started = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let observer = start_model_readiness_observer_with_poll(Duration::from_secs(30), {
        let started = Arc::clone(&started);
        let dropped = Arc::clone(&dropped);
        move || {
            let started = Arc::clone(&started);
            let dropped = Arc::clone(&dropped);
            async move {
                let _drop_signal = DropSignal(dropped);
                started.fetch_add(1, Ordering::SeqCst);
                std::future::pending::<()>().await;
            }
        }
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while started.load(Ordering::SeqCst) == 0 && tokio::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }

    observer.stop();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while dropped.load(Ordering::SeqCst) == 0 && tokio::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }

    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn trusted_runtime_token_changes_only_for_generation_relevant_configuration() {
    let baseline = identity("qwen", "snapshot-a", "apple-a", 1);
    let empty = OrchestratorAdmissionState::Available {
        tracked_nonterminal: 0,
        capacity: 64,
    };
    let full = OrchestratorAdmissionState::Available {
        tracked_nonterminal: 64,
        capacity: 64,
    };
    assert_eq!(baseline, identity("qwen", "snapshot-a", "apple-a", 1));
    assert_ne!(baseline, identity("qwen-next", "snapshot-a", "apple-a", 1));
    assert_ne!(baseline, identity("qwen", "snapshot-b", "apple-a", 1));
    assert_ne!(baseline, identity("qwen", "snapshot-a", "apple-b", 1));
    assert_ne!(baseline, identity("qwen", "snapshot-a", "apple-a", 2));
    assert_eq!(
        RuntimeReadiness::ready(&baseline, 7, empty).transition_token(),
        RuntimeReadiness::ready(&baseline, 7, empty).transition_token()
    );
    assert_ne!(
        RuntimeReadiness::ready(&baseline, 7, full).transition_token(),
        RuntimeReadiness::ready(&baseline, 7, empty).transition_token()
    );
    assert_ne!(
        RuntimeReadiness::ready(&baseline, 8, empty).transition_token(),
        RuntimeReadiness::ready(&baseline, 7, empty).transition_token()
    );
}

#[test]
fn unavailable_to_restored_snapshot_is_one_distinct_local_readiness_transition() {
    let unavailable = RuntimeReadiness::unavailable("rag_unavailable", 0);
    let repeated = RuntimeReadiness::unavailable("rag_unavailable", 0);
    let restored = RuntimeReadiness::ready(
        &identity("qwen", "snapshot-restored", "apple-a", 1),
        1,
        OrchestratorAdmissionState::Available {
            tracked_nonterminal: 0,
            capacity: 64,
        },
    );
    assert_eq!(unavailable.transition_token(), repeated.transition_token());
    assert_ne!(unavailable.transition_token(), restored.transition_token());
}

#[tokio::test]
async fn orchestrator_admission_transition_is_exact_and_scheduler_churn_is_irrelevant() {
    let scheduler = LocalModelScheduler::new(2).expect("scheduler");
    let orchestrator = CommandBriefOrchestrator::new(
        scheduler.clone(),
        Arc::new(UnusedProvider),
        Arc::new(UnusedProvider),
        Arc::new(UnusedProvider),
    );
    let request = || {
        CommandBriefRequest::new("daily-command-brief", "prepare", "2026-07-25T06:00:00Z")
            .expect("request")
    };
    for index in 0..64 {
        orchestrator
            .start_exact(&format!("capacity-{index}"), request())
            .expect("admitted run");
    }
    assert_eq!(
        orchestrator
            .start_exact("capacity-64", request())
            .expect_err("full registry"),
        OrchestratorStartError::AdmissionUnavailable
    );
    let full = orchestrator.admission_state();
    assert_eq!(
        full,
        OrchestratorAdmissionState::Available {
            tracked_nonterminal: 64,
            capacity: 64,
        }
    );
    let identity = identity("qwen", "snapshot-a", "apple-a", 2);
    let full_token = RuntimeReadiness::ready(&identity, 9, full).transition_token;

    let specialist = scheduler.clone();
    let specialist_task = tokio::spawn(async move {
        specialist
            .schedule(
                crate::command_brief::scheduler::SchedulerJobKey::new(
                    "unrelated",
                    AdviserId::Operations,
                )
                .expect("key"),
                CancellationToken::new(),
                |_| async { Ok::<(), ()>(()) },
            )
            .await
    });
    specialist_task.await.expect("join").expect("specialist");
    assert_eq!(
        RuntimeReadiness::ready(&identity, 9, orchestrator.admission_state()).transition_token,
        full_token,
        "specialist scheduler activity is not command-run admission"
    );

    assert!(orchestrator.cancel("capacity-0"));
    for _ in 0..100 {
        if orchestrator.admission_state()
            == (OrchestratorAdmissionState::Available {
                tracked_nonterminal: 63,
                capacity: 64,
            })
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    let one_free = orchestrator.admission_state();
    assert_eq!(
        one_free,
        OrchestratorAdmissionState::Available {
            tracked_nonterminal: 63,
            capacity: 64,
        }
    );
    assert_ne!(
        RuntimeReadiness::ready(&identity, 9, one_free).transition_token,
        full_token,
        "64 to 63 is one distinct admission transition"
    );
}

#[tokio::test]
async fn deferred_full_admission_settlement_causes_one_real_timer_retry() {
    let conn = Connection::open_in_memory().expect("memory");
    crate::command_brief::store::migrate_command_brief_store(&conn).expect("migrate");
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let now = utc("2026-07-25T01:00:00Z");
    let scheduler = LocalModelScheduler::new(2).expect("scheduler");
    let orchestrator = CommandBriefOrchestrator::new(
        scheduler,
        Arc::new(UnusedProvider),
        Arc::new(UnusedProvider),
        Arc::new(UnusedProvider),
    );
    let request = || {
        CommandBriefRequest::new("daily-command-brief", "prepare", "2026-07-25T06:00:00Z")
            .expect("request")
    };
    for index in 0..64 {
        orchestrator
            .start_exact(&format!("admission-{index}"), request())
            .expect("admitted");
    }
    let runtime_identity = identity("qwen", "snapshot-a", "apple-a", 2);
    let full_token = RuntimeReadiness::ready(&runtime_identity, 11, orchestrator.admission_state())
        .transition_token;
    let starter = RecordingStarter::default();
    assert_eq!(
        process_due_schedule(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Startup,
            &ReadinessSnapshot::deferred(DeferredReason::AdmissionUnavailable, &full_token,),
            &starter,
        )
        .expect("full deferred"),
        ScheduleRunOutcome::Deferred {
            reason: DeferredReason::AdmissionUnavailable,
        }
    );
    assert_eq!(
        timer_claim_fast_path(&conn, &schedule, now, ScheduleTrigger::Timer, || Some(
            full_token.clone()
        ),)
        .expect("unchanged full admission"),
        Some(ScheduleRunOutcome::AlreadyClaimed)
    );

    assert!(orchestrator.cancel("admission-0"));
    for _ in 0..100 {
        if orchestrator.admission_state()
            == (OrchestratorAdmissionState::Available {
                tracked_nonterminal: 63,
                capacity: 64,
            })
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    let one_free_token =
        RuntimeReadiness::ready(&runtime_identity, 11, orchestrator.admission_state())
            .transition_token;
    assert_eq!(
        timer_claim_fast_path(&conn, &schedule, now, ScheduleTrigger::Timer, || Some(
            one_free_token.clone()
        ),)
        .expect("admission transition"),
        None
    );
    assert!(matches!(
        process_due_schedule(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Timer,
            &ReadinessSnapshot::ready(&one_free_token),
            &starter,
        )
        .expect("one real retry"),
        ScheduleRunOutcome::Started { .. }
    ));
    let retry_count: i64 = conn
        .query_row(
            "SELECT retry_count FROM command_brief_schedule_claims",
            [],
            |row| row.get(0),
        )
        .expect("retry count");
    assert_eq!(retry_count, 1);
    assert_eq!(starter.starts.lock().expect("starts").len(), 1);
    for _ in 0..10 {
        assert_eq!(
            timer_claim_fast_path(&conn, &schedule, now, ScheduleTrigger::Timer, || panic!(
                "started claim must not inspect admission"
            ),)
            .expect("started fast path"),
            Some(ScheduleRunOutcome::AlreadyClaimed)
        );
    }
    assert_eq!(starter.starts.lock().expect("starts").len(), 1);
}

#[test]
fn poisoned_orchestrator_admission_is_stable_bounded_and_redacted() {
    let scheduler = LocalModelScheduler::new(1).expect("scheduler");
    let orchestrator = CommandBriefOrchestrator::new(
        scheduler,
        Arc::new(UnusedProvider),
        Arc::new(UnusedProvider),
        Arc::new(UnusedProvider),
    );
    orchestrator.poison_admission_lock_for_test("sensitive panic detail");
    assert_eq!(
        orchestrator.admission_state(),
        OrchestratorAdmissionState::Unavailable
    );
    let identity = identity("qwen", "snapshot-a", "apple-a", 1);
    let first =
        RuntimeReadiness::ready(&identity, 7, orchestrator.admission_state()).transition_token;
    let second =
        RuntimeReadiness::ready(&identity, 7, orchestrator.admission_state()).transition_token;
    assert_eq!(first, second);
    assert!(first.len() <= 256);
    assert!(!first.contains("sensitive"));
}

#[tokio::test]
async fn runtime_swap_handles_both_capacities_and_model_change_while_old_runs_finish() {
    let make = |config: RuntimeConfigIdentity, generation| {
        let scheduler = LocalModelScheduler::new(config.capacity).expect("scheduler");
        Arc::new(InstalledCommandBriefRuntime {
            owner_pubkey: "owner-pubkey".to_string(),
            config,
            generation,
            orchestrator: CommandBriefOrchestrator::new(
                scheduler,
                Arc::new(UnusedProvider),
                Arc::new(UnusedProvider),
                Arc::new(UnusedProvider),
            ),
        })
    };
    let request = || {
        CommandBriefRequest::new("daily-command-brief", "prepare", "2026-07-25T06:00:00Z")
            .expect("request")
    };
    let first = make(identity("qwen", "snapshot-a", "apple-a", 1), 1);
    first
        .orchestrator
        .start_exact("runtime-one", request())
        .expect("first active run");
    let mut runtimes = CommandBriefRuntimeSet::default();
    runtimes.install(Arc::clone(&first));
    let second = make(identity("qwen", "snapshot-a", "apple-a", 2), 2);
    runtimes.install(Arc::clone(&second));
    second
        .orchestrator
        .start_exact("runtime-two", request())
        .expect("second active run");
    let third = make(identity("qwen", "snapshot-a", "apple-a", 1), 3);
    runtimes.install(Arc::clone(&third));
    third
        .orchestrator
        .start_exact("runtime-three", request())
        .expect("third active run");
    let fourth = make(identity("qwen-next", "snapshot-a", "apple-a", 1), 4);
    runtimes.install(Arc::clone(&fourth));
    assert_eq!(runtimes.retired.len(), 3);
    assert!(runtimes
        .retired
        .iter()
        .all(|runtime| runtime.orchestrator.has_nonterminal_runs()));
    assert_eq!(runtimes.current.as_ref().expect("current").generation, 4);
    assert_eq!(
        runtimes.current.as_ref().expect("current").config.capacity,
        1
    );
    assert_eq!(first.generation, 1);
    assert_eq!(first.config.capacity, 1);
    assert!(Arc::strong_count(&first) >= 2);
    assert!(first.orchestrator.cancel("runtime-one"));
    assert!(second.orchestrator.cancel("runtime-two"));
    assert!(third.orchestrator.cancel("runtime-three"));
}

mod owner_tests {
    include!("startup_owner_tests.rs");
}
