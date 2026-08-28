//! Writer-pool facade for durable agent workflow persistence.

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::agent_approval::{self, AgentApproval, EnsureAgentApproval};
use crate::agent_artifact;
use crate::agent_settlement;
use crate::agent_snapshot;
use crate::agent_task_recovery;
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

/// Server-provenance candidate for one durable reconciliation pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableRunCandidate {
    /// Owning tenant read from durable state.
    pub community_id: CommunityId,
    /// Active durable workflow run.
    pub run_id: Uuid,
}

impl AgentWorkflowStore {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    /// List a bounded server-provenance page of active durable runs.
    ///
    /// This global internal scan only discovers tenant/run coordinates; every
    /// mutation remains tenant-scoped and CAS-guarded. Runs whose tasks are all
    /// terminal remain candidates until their lifecycle is settled.
    pub async fn list_reconcilable_runs(&self, limit: i64) -> Result<Vec<DurableRunCandidate>> {
        let rows = sqlx::query(
            "WITH candidates AS (                SELECT state.community_id,state.run_id,state.updated_at,                       ROW_NUMBER() OVER (                         PARTITION BY state.community_id                         ORDER BY state.updated_at,state.run_id                       ) AS tenant_rank                FROM workflow_run_state state                JOIN workflow_runs run                  ON run.community_id=state.community_id AND run.id=state.run_id                WHERE run.status IN ('running','waiting_approval')                  AND EXISTS (SELECT 1 FROM workflow_run_tasks task                    WHERE task.community_id=state.community_id AND task.run_id=state.run_id)              )              SELECT community_id,run_id FROM candidates              WHERE tenant_rank <= $2              ORDER BY tenant_rank,updated_at,run_id LIMIT $1",
        )
        .bind(limit.clamp(1, 1_000))
        .bind(100_i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(DurableRunCandidate {
                    community_id: CommunityId::from_uuid(row.try_get("community_id")?),
                    run_id: row.try_get("run_id")?,
                })
            })
            .collect()
    }

    /// Atomically complete an active lifecycle and append its terminal transition.
    pub async fn complete_active_run(
        &self,
        community: CommunityId,
        run_id: Uuid,
        step_count: i32,
        metadata: &Value,
    ) -> Result<Option<AgentRunTransition>> {
        agent_settlement::complete_active_run(&self.pool, community, run_id, step_count, metadata)
            .await
    }

    /// Atomically fail an active lifecycle and append its terminal transition.
    #[allow(clippy::too_many_arguments)]
    pub async fn fail_active_run(
        &self,
        community: CommunityId,
        run_id: Uuid,
        step_count: i32,
        code: &str,
        message: &str,
        metadata: &Value,
    ) -> Result<Option<AgentRunTransition>> {
        agent_settlement::fail_active_run(
            &self.pool, community, run_id, step_count, code, message, metadata,
        )
        .await
    }

    /// Ensure one persistent approval exists for an eligible task.
    pub async fn ensure_approval(
        &self,
        community: CommunityId,
        parameters: EnsureAgentApproval<'_>,
    ) -> Result<Option<AgentApproval>> {
        agent_approval::ensure_approval(&self.pool, community, parameters).await
    }

    /// Fetch the persistent approval bound to a task.
    pub async fn get_approval(
        &self,
        community: CommunityId,
        run_id: Uuid,
        task_id: Uuid,
    ) -> Result<Option<AgentApproval>> {
        agent_approval::get_approval(&self.pool, community, run_id, task_id).await
    }

    /// Expire a still-pending durable approval after its fixed deadline.
    pub async fn expire_approval(
        &self,
        community: CommunityId,
        run_id: Uuid,
        task_id: Uuid,
    ) -> Result<bool> {
        agent_approval::expire_approval(&self.pool, community, run_id, task_id).await
    }

    /// Mark a running workflow lifecycle as waiting for approval.
    pub async fn mark_run_waiting_approval(
        &self,
        community: CommunityId,
        run_id: Uuid,
        step_index: i32,
    ) -> Result<bool> {
        agent_approval::mark_run_waiting(&self.pool, community, run_id, step_index).await
    }

    /// Resume a lifecycle after its durable approval task completes.
    pub async fn mark_run_running_after_approval(
        &self,
        community: CommunityId,
        run_id: Uuid,
    ) -> Result<bool> {
        agent_approval::mark_run_running(&self.pool, community, run_id).await
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

    /// Atomically persist one document manifest and complete its eligible task.
    #[allow(clippy::too_many_arguments)]
    pub async fn complete_ingestion_task(
        &self,
        community: CommunityId,
        task_id: Uuid,
        expected: i64,
        kind: &str,
        artifact_sha256: &[u8],
        manifest_hash: &[u8],
        inline_content: &Value,
        metadata: &Value,
        idempotency_key: &str,
    ) -> Result<Option<(AgentTask, AgentArtifact)>> {
        agent_workflow::complete_ingestion_task(
            &self.pool,
            community,
            task_id,
            expected,
            kind,
            artifact_sha256,
            manifest_hash,
            inline_content,
            metadata,
            idempotency_key,
        )
        .await
    }

    /// Atomically retry or fail a timed-out running agent task.
    pub async fn recover_timed_out_task(
        &self,
        community: CommunityId,
        task_id: Uuid,
        expected_version: i64,
        timeout_secs: i64,
        retry_at: DateTime<Utc>,
    ) -> Result<Option<AgentTask>> {
        agent_task_recovery::recover_timed_out_task(
            &self.pool,
            community,
            task_id,
            expected_version,
            timeout_secs,
            retry_at,
        )
        .await
    }

    /// Defer an eligible coordinator task after a transient adapter failure.
    #[allow(clippy::too_many_arguments)]
    pub async fn defer_ready_task(
        &self,
        community: CommunityId,
        task_id: Uuid,
        expected: i64,
        not_before: DateTime<Utc>,
        code: &str,
        message: &str,
    ) -> Result<Option<AgentTask>> {
        agent_workflow::defer_ready_task(
            &self.pool, community, task_id, expected, not_before, code, message,
        )
        .await
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

    /// Atomically persist a validated artifact and complete its active task.
    pub async fn persist_artifact_and_complete(
        &self,
        community: CommunityId,
        task_id: Uuid,
        expected_version: i64,
        artifact: CreateAgentArtifact<'_>,
    ) -> Result<Option<(AgentTask, AgentArtifact)>> {
        agent_artifact::persist_and_complete(
            &self.pool,
            community,
            task_id,
            expected_version,
            artifact,
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

    /// List the latest checkpoint for every task in a run.
    pub async fn list_latest_checkpoints(
        &self,
        community: CommunityId,
        run_id: Uuid,
    ) -> Result<Vec<AgentCheckpoint>> {
        agent_snapshot::list_latest_checkpoints(&self.pool, community, run_id).await
    }

    /// Read the latest monotonic transition sequence for a run.
    pub async fn latest_transition_sequence(
        &self,
        community: CommunityId,
        run_id: Uuid,
    ) -> Result<i64> {
        agent_snapshot::latest_transition_sequence(&self.pool, community, run_id).await
    }

    /// List durable approvals bound to a run.
    pub async fn list_approvals(
        &self,
        community: CommunityId,
        run_id: Uuid,
    ) -> Result<Vec<AgentApproval>> {
        agent_snapshot::list_approvals(&self.pool, community, run_id).await
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
