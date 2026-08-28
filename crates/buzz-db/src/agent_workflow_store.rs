//! Writer-pool facade for durable agent workflow persistence.

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::agent_workflow::{
    self, AgentArtifact, AgentCheckpoint, AgentRunState, AgentRunTransition, AgentTask,
    CreateAgentArtifact, CreateAgentTask, EnsureAgentRunState,
};
use crate::error::Result;

/// Tenant-scoped durable agent-workflow persistence facade.
///
/// Constructed by [`crate::Db::agent_workflow_store`]; the underlying writer
/// pool remains private to `buzz-db`.
#[derive(Clone, Debug)]
pub struct AgentWorkflowStore {
    pool: PgPool,
}

impl AgentWorkflowStore {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Ensure durable state exists for a run.
    pub async fn ensure_run_state(
        &self,
        community: CommunityId,
        parameters: EnsureAgentRunState<'_>,
    ) -> Result<AgentRunState> {
        agent_workflow::ensure_run_state(&self.pool, community, parameters).await
    }

    /// Fetch durable state for a run.
    pub async fn get_run_state(
        &self,
        community: CommunityId,
        run_id: Uuid,
    ) -> Result<Option<AgentRunState>> {
        agent_workflow::get_run_state(&self.pool, community, run_id).await
    }

    /// Compare-and-swap run phase metadata.
    pub async fn cas_run_state(
        &self,
        community: CommunityId,
        run_id: Uuid,
        expected: i64,
        phase: &str,
        metadata: &Value,
        deadline: Option<DateTime<Utc>>,
    ) -> Result<Option<AgentRunState>> {
        agent_workflow::cas_run_state(
            &self.pool, community, run_id, expected, phase, metadata, deadline,
        )
        .await
    }

    /// Idempotently create a durable task.
    pub async fn create_task(
        &self,
        community: CommunityId,
        parameters: CreateAgentTask<'_>,
    ) -> Result<AgentTask> {
        agent_workflow::create_task(&self.pool, community, parameters).await
    }

    /// List tasks for a run.
    pub async fn list_tasks(
        &self,
        community: CommunityId,
        run_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<AgentTask>> {
        agent_workflow::list_tasks(&self.pool, community, run_id, limit).await
    }

    /// Fetch one task by id.
    pub async fn get_task(
        &self,
        community: CommunityId,
        task_id: Uuid,
    ) -> Result<Option<AgentTask>> {
        agent_workflow::get_task(&self.pool, community, task_id).await
    }

    /// Atomically claim an eligible task.
    pub async fn claim_task(
        &self,
        community: CommunityId,
        task_id: Uuid,
        expected: i64,
        agent: &[u8],
    ) -> Result<Option<AgentTask>> {
        agent_workflow::claim_task(&self.pool, community, task_id, expected, agent).await
    }

    /// Atomically block an eligible unsupported coordinator task.
    #[allow(clippy::too_many_arguments)]
    pub async fn block_ready_task(
        &self,
        community: CommunityId,
        task_id: Uuid,
        expected: i64,
        code: &str,
        message: &str,
    ) -> Result<Option<AgentTask>> {
        agent_workflow::block_ready_task(&self.pool, community, task_id, expected, code, message)
            .await
    }

    /// Atomically complete an eligible coordinator task.
    pub async fn complete_ready_task(
        &self,
        community: CommunityId,
        task_id: Uuid,
        expected: i64,
    ) -> Result<Option<AgentTask>> {
        agent_workflow::complete_ready_task(&self.pool, community, task_id, expected).await
    }

    /// Complete a task using optimistic locking.
    pub async fn complete_task(
        &self,
        community: CommunityId,
        task_id: Uuid,
        expected: i64,
    ) -> Result<Option<AgentTask>> {
        agent_workflow::complete_task(&self.pool, community, task_id, expected).await
    }

    /// Schedule a retry using optimistic locking.
    #[allow(clippy::too_many_arguments)]
    pub async fn schedule_retry(
        &self,
        community: CommunityId,
        task_id: Uuid,
        expected: i64,
        not_before: DateTime<Utc>,
        code: &str,
        message: &str,
    ) -> Result<Option<AgentTask>> {
        agent_workflow::schedule_retry(
            &self.pool, community, task_id, expected, not_before, code, message,
        )
        .await
    }

    /// Fail or block a task using optimistic locking.
    #[allow(clippy::too_many_arguments)]
    pub async fn fail_task(
        &self,
        community: CommunityId,
        task_id: Uuid,
        expected: i64,
        blocked: bool,
        code: &str,
        message: &str,
    ) -> Result<Option<AgentTask>> {
        agent_workflow::fail_task(
            &self.pool, community, task_id, expected, blocked, code, message,
        )
        .await
    }

    /// Idempotently store a versioned artifact.
    pub async fn create_artifact(
        &self,
        community: CommunityId,
        parameters: CreateAgentArtifact<'_>,
    ) -> Result<AgentArtifact> {
        agent_workflow::create_artifact(&self.pool, community, parameters).await
    }

    /// List artifacts for a run.
    pub async fn list_artifacts(
        &self,
        community: CommunityId,
        run_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<AgentArtifact>> {
        agent_workflow::list_artifacts(&self.pool, community, run_id, limit).await
    }

    /// Append an idempotent monotonic checkpoint.
    pub async fn append_checkpoint(
        &self,
        community: CommunityId,
        run_id: Uuid,
        task_id: Uuid,
        sequence: i64,
        state: &Value,
        artifact_id: Option<Uuid>,
    ) -> Result<AgentCheckpoint> {
        agent_workflow::append_checkpoint(
            &self.pool,
            community,
            run_id,
            task_id,
            sequence,
            state,
            artifact_id,
        )
        .await
    }

    /// Fetch the latest checkpoint for a task.
    pub async fn latest_checkpoint(
        &self,
        community: CommunityId,
        run_id: Uuid,
        task_id: Uuid,
    ) -> Result<Option<AgentCheckpoint>> {
        agent_workflow::latest_checkpoint(&self.pool, community, run_id, task_id).await
    }

    /// Append an idempotent run transition.
    #[allow(clippy::too_many_arguments)]
    pub async fn append_transition(
        &self,
        community: CommunityId,
        run_id: Uuid,
        sequence: i64,
        from_phase: Option<&str>,
        to_phase: &str,
        from_status: Option<&str>,
        to_status: &str,
        reason: Option<&str>,
        actor: Option<&[u8]>,
        metadata: &Value,
    ) -> Result<AgentRunTransition> {
        agent_workflow::append_transition(
            &self.pool,
            community,
            run_id,
            sequence,
            from_phase,
            to_phase,
            from_status,
            to_status,
            reason,
            actor,
            metadata,
        )
        .await
    }

    /// Append the next monotonic run transition.
    #[allow(clippy::too_many_arguments)]
    pub async fn append_next_transition(
        &self,
        community: CommunityId,
        run_id: Uuid,
        from_phase: Option<&str>,
        to_phase: &str,
        from_status: Option<&str>,
        to_status: &str,
        reason: Option<&str>,
        actor: Option<&[u8]>,
        metadata: &Value,
    ) -> Result<AgentRunTransition> {
        agent_workflow::append_next_transition(
            &self.pool,
            community,
            run_id,
            from_phase,
            to_phase,
            from_status,
            to_status,
            reason,
            actor,
            metadata,
        )
        .await
    }

    /// List transitions for a run.
    pub async fn list_transitions(
        &self,
        community: CommunityId,
        run_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<AgentRunTransition>> {
        agent_workflow::list_transitions(&self.pool, community, run_id, limit).await
    }
}
