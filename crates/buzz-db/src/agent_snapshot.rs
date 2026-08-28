//! Read-only aggregate queries for durable run snapshots.

use buzz_core::CommunityId;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::agent_approval::{map_approval, AgentApproval};
use crate::agent_workflow::{map_checkpoint, AgentCheckpoint};
use crate::error::Result;

/// List the latest checkpoint for every task in a run.
pub async fn list_latest_checkpoints(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
) -> Result<Vec<AgentCheckpoint>> {
    sqlx::query(
        "SELECT DISTINCT ON (task_id) id,run_id,task_id,sequence,state,artifact_id,created_at          FROM workflow_run_checkpoints WHERE community_id=$1 AND run_id=$2          ORDER BY task_id,sequence DESC",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(map_checkpoint)
    .collect()
}

/// Read the latest monotonic transition sequence for a run.
pub async fn latest_transition_sequence(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
) -> Result<i64> {
    let row = sqlx::query(
        "SELECT COALESCE(MAX(sequence),0)::BIGINT AS sequence          FROM workflow_run_transitions WHERE community_id=$1 AND run_id=$2",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("sequence")?)
}

/// List every durable approval bound to a run.
pub async fn list_approvals(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
) -> Result<Vec<AgentApproval>> {
    sqlx::query(
        "SELECT token,workflow_id,run_id,task_id,step_id,request_message,step_index,         approver_spec,status::text AS status,approver_pubkey,expires_at          FROM workflow_approvals WHERE community_id=$1 AND run_id=$2            AND task_id IS NOT NULL ORDER BY step_index,task_id",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(map_approval)
    .collect()
}
