use std::sync::Arc;

use buzz_core_pkg::command_brief::{CommandBriefFailureCode, CommandBriefLifecycleState};

use super::orchestrator_test_support::FakePersistence;
use super::orchestrator_tests::{
    orchestrator, request, wait_terminal, FakeAdviserProvider, FakeSourceProvider,
};
use super::sources::SourceCollectionError;
use super::types::BriefRunState;

#[tokio::test]
async fn source_failure_persists_one_redacted_failed_terminal_without_a_brief() {
    let persistence = Arc::new(FakePersistence::default());
    let orchestrator = orchestrator(
        1,
        Arc::new(FakeSourceProvider::with_freeze_error(
            SourceCollectionError::RagUnavailable,
        )),
        Arc::new(FakeAdviserProvider::default()),
        persistence.clone(),
    );

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Failed
    );
    assert!(orchestrator.result(&run_id).is_none());
    assert!(persistence.values.lock().expect("briefs").is_empty());
    persistence.assert_one_terminal(
        CommandBriefLifecycleState::Failed,
        Some(CommandBriefFailureCode::RagUnavailable),
    );
    let status = serde_json::to_value(orchestrator.status(&run_id).expect("status")).expect("json");
    assert_eq!(status["error"], "rag_unavailable");
}

#[tokio::test]
async fn audit_persistence_failure_surfaces_only_bounded_local_failure_status() {
    let persistence = Arc::new(FakePersistence::failing());
    let orchestrator = orchestrator(
        1,
        Arc::new(FakeSourceProvider::with_freeze_error(
            SourceCollectionError::RagUnavailable,
        )),
        Arc::new(FakeAdviserProvider::default()),
        persistence.clone(),
    );

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Failed
    );
    assert!(persistence.terminals.lock().expect("terminals").is_empty());
    let status = serde_json::to_value(orchestrator.status(&run_id).expect("status")).expect("json");
    assert_eq!(status["error"], "brief_persistence_failed");
}

#[tokio::test]
async fn ui_uses_committed_terminal_state_and_code_not_failed_input() {
    let persistence = Arc::new(FakePersistence::returning_terminal(
        CommandBriefLifecycleState::Cancelled,
        Some(CommandBriefFailureCode::CancellationRequested),
    ));
    let orchestrator = orchestrator(
        1,
        Arc::new(FakeSourceProvider::with_freeze_error(
            SourceCollectionError::RagUnavailable,
        )),
        Arc::new(FakeAdviserProvider::default()),
        persistence,
    );

    let run_id = orchestrator.start(request()).expect("run starts");
    assert_eq!(
        wait_terminal(&orchestrator, &run_id).await,
        BriefRunState::Cancelled
    );
    let status = serde_json::to_value(orchestrator.status(&run_id).expect("status")).expect("json");
    assert_eq!(status["error"], "cancellation_requested");
}
