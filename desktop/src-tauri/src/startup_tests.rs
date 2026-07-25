use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::{
    CommandBriefRuntimeSet, InstalledCommandBriefRuntime, RuntimeConfigIdentity, RuntimeReadiness,
};
use crate::command_brief::audit::{PersistedTerminal, TerminalAuditInput};
use crate::command_brief::orchestrator::{
    BriefAdviserError, BriefAdviserProvider, BriefFuture, BriefPersistence, BriefPersistenceError,
    BriefSourceProvider, CommandBriefOrchestrator, CommandBriefRequest,
};
use crate::command_brief::provenance::ValidatedSource;
use crate::command_brief::scheduler::LocalModelScheduler;
use crate::command_brief::sources::{FrozenSourceContext, SourceCollectionError};
use crate::command_brief::types::{AdviserContribution, AdviserId};

struct UnusedProvider;

impl BriefSourceProvider for UnusedProvider {
    fn freeze<'a>(
        &'a self,
        _run_id: &'a str,
        _co_request: &'a str,
        _observed_at: &'a str,
        _cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<FrozenSourceContext, SourceCollectionError>> {
        Box::pin(std::future::pending())
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

#[test]
fn trusted_runtime_token_changes_only_for_generation_relevant_configuration() {
    let baseline = identity("qwen", "snapshot-a", "apple-a", 1);
    assert_eq!(baseline, identity("qwen", "snapshot-a", "apple-a", 1));
    assert_ne!(baseline, identity("qwen-next", "snapshot-a", "apple-a", 1));
    assert_ne!(baseline, identity("qwen", "snapshot-b", "apple-a", 1));
    assert_ne!(baseline, identity("qwen", "snapshot-a", "apple-b", 1));
    assert_ne!(baseline, identity("qwen", "snapshot-a", "apple-a", 2));
    assert_eq!(
        RuntimeReadiness::ready(&baseline, 7, 0).transition_token(),
        RuntimeReadiness::ready(&baseline, 7, 0).transition_token()
    );
    assert_ne!(
        RuntimeReadiness::ready(&baseline, 7, 1).transition_token(),
        RuntimeReadiness::ready(&baseline, 7, 0).transition_token()
    );
    assert_ne!(
        RuntimeReadiness::ready(&baseline, 8, 0).transition_token(),
        RuntimeReadiness::ready(&baseline, 7, 0).transition_token()
    );
}

#[test]
fn unavailable_to_restored_snapshot_is_one_distinct_local_readiness_transition() {
    let unavailable = RuntimeReadiness::unavailable("rag_unavailable", 0);
    let repeated = RuntimeReadiness::unavailable("rag_unavailable", 0);
    let restored =
        RuntimeReadiness::ready(&identity("qwen", "snapshot-restored", "apple-a", 1), 1, 0);
    assert_eq!(unavailable.transition_token(), repeated.transition_token());
    assert_ne!(unavailable.transition_token(), restored.transition_token());
}

#[tokio::test]
async fn runtime_swap_handles_both_capacities_and_model_change_while_old_runs_finish() {
    let make = |config: RuntimeConfigIdentity, generation| {
        let scheduler = LocalModelScheduler::new(config.capacity).expect("scheduler");
        Arc::new(InstalledCommandBriefRuntime {
            config,
            generation,
            scheduler: scheduler.clone(),
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
        runtimes
            .current
            .as_ref()
            .expect("current")
            .scheduler
            .capacity(),
        1
    );
    assert_eq!(first.generation, 1);
    assert_eq!(first.scheduler.capacity(), 1);
    assert!(Arc::strong_count(&first) >= 2);
    assert!(first.orchestrator.cancel("runtime-one"));
    assert!(second.orchestrator.cancel("runtime-two"));
    assert!(third.orchestrator.cancel("runtime-three"));
}
