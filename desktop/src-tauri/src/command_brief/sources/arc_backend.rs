use std::sync::Arc;

use super::*;

impl<T> SourceBackend for Arc<T>
where
    T: SourceBackend + ?Sized,
{
    fn verify_active_rag_snapshot(&self) -> Result<VerifiedRagSnapshot, SourceCollectionError> {
        (**self).verify_active_rag_snapshot()
    }

    fn memory_conflict_count(&self) -> u64 {
        (**self).memory_conflict_count()
    }

    fn command_team_discussions(&self) -> CommandTeamDiscussionBatch {
        (**self).command_team_discussions()
    }

    fn planning_evidence(&self) -> PlanningEvidenceBatch {
        (**self).planning_evidence()
    }

    fn collect_rag(
        &self,
        snapshot: &VerifiedRagSnapshot,
        intent: &FixedRetrievalIntent,
        query: &str,
        collections: &[String],
        cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        (**self).collect_rag(snapshot, intent, query, collections, cancellation)
    }

    fn collect_memory(
        &self,
        intent: &FixedRetrievalIntent,
        cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        (**self).collect_memory(intent, cancellation)
    }

    fn collect_apple(
        &self,
        request: &AppleInputRequest,
        cancellation: &CancellationToken,
    ) -> Result<AppleInputResponse, SourceCollectionError> {
        (**self).collect_apple(request, cancellation)
    }

    fn recheck_rag_snapshot(
        &self,
        expected: &VerifiedRagSnapshot,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceCollectionError> {
        (**self).recheck_rag_snapshot(expected, cancellation)
    }

    fn post_recheck_limitations(&self) -> Vec<String> {
        (**self).post_recheck_limitations()
    }
}
