//! Persistent human approvals bound to durable workflow tasks.

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::workflow::ApprovalStatus;

/// Immutable fields used to create one durable task approval.
pub struct EnsureAgentApproval<'a> {
    /// Owning workflow.
    pub workflow_id: Uuid,
    /// Owning run.
    pub run_id: Uuid,
    /// Gated durable task.
    pub task_id: Uuid,
    /// Stable workflow step id.
    pub step_id: &'a str,
    /// Message shown to the human approver.
    pub request_message: &'a str,
    /// Zero-based workflow step index.
    pub step_index: i32,
    /// Exact authorized approver pubkey as lowercase hex.
    pub approver_spec: &'a str,
    /// Fixed expiration established when the approval is first armed.
    pub expires_at: DateTime<Utc>,
}

/// Approval state required by the durable scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentApproval {
    /// Stored SHA-256 approval reference.
    pub token: Vec<u8>,
    /// Owning workflow.
    pub workflow_id: Uuid,
    /// Owning run.
    pub run_id: Uuid,
    /// Gated task.
    pub task_id: Uuid,
    /// Stable workflow step id.
    pub step_id: String,
    /// Message shown to the human approver.
    pub request_message: String,
    /// Zero-based workflow step index.
    pub step_index: i32,
    /// Exact authorized approver pubkey.
    pub approver_spec: String,
    /// Current decision state.
    pub status: ApprovalStatus,
    /// Pubkey that made the decision.
    pub approver_pubkey: Option<Vec<u8>>,
    /// Fixed expiration.
    pub expires_at: DateTime<Utc>,
}

/// Ensure exactly one approval exists for an eligible durable task.
pub async fn ensure_approval(
    pool: &PgPool,
    community: CommunityId,
    parameters: EnsureAgentApproval<'_>,
) -> Result<Option<AgentApproval>> {
    let raw_token = format!(
        "durable-approval:{}:{}",
        parameters.run_id, parameters.task_id
    );
    let token = Sha256::digest(raw_token.as_bytes()).to_vec();
    let row = sqlx::query(
        r#"
        WITH inserted AS (
          INSERT INTO workflow_approvals
              (community_id,token,workflow_id,run_id,task_id,step_id,request_message,
               step_index,approver_spec,status,expires_at)
          SELECT $1,$2,$3,$4,task.id,$6,$7,$8,$9,'pending',$10
          FROM workflow_run_tasks task
          WHERE task.community_id=$1 AND task.run_id=$4 AND task.id=$5
            AND task.status='pending'
            AND NOT EXISTS (
              SELECT 1 FROM jsonb_array_elements_text(task.depends_on) dependency(task_key)
              LEFT JOIN workflow_run_tasks prerequisite
                ON prerequisite.community_id=task.community_id
               AND prerequisite.run_id=task.run_id
               AND prerequisite.task_key=dependency.task_key
              WHERE prerequisite.id IS NULL OR prerequisite.status<>'completed'
            )
          ON CONFLICT (community_id,run_id,task_id) WHERE task_id IS NOT NULL
          DO NOTHING
          RETURNING token,workflow_id,run_id,task_id,step_id,request_message,
                    step_index,approver_spec,status::text AS status,approver_pubkey,expires_at
        )
        SELECT * FROM inserted
        UNION ALL
        SELECT token,workflow_id,run_id,task_id,step_id,request_message,step_index,
               approver_spec,status::text AS status,approver_pubkey,expires_at
        FROM workflow_approvals
        WHERE community_id=$1 AND run_id=$4 AND task_id=$5
          AND NOT EXISTS (SELECT 1 FROM inserted)
        LIMIT 1
        "#,
    )
    .bind(community.as_uuid())
    .bind(token)
    .bind(parameters.workflow_id)
    .bind(parameters.run_id)
    .bind(parameters.task_id)
    .bind(parameters.step_id)
    .bind(parameters.request_message)
    .bind(parameters.step_index)
    .bind(parameters.approver_spec)
    .bind(parameters.expires_at)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let approval = map_approval(row)?;
    if approval.workflow_id != parameters.workflow_id
        || approval.run_id != parameters.run_id
        || approval.task_id != parameters.task_id
        || approval.step_id != parameters.step_id
        || approval.request_message != parameters.request_message
        || approval.step_index != parameters.step_index
        || approval.approver_spec != parameters.approver_spec
    {
        return Err(DbError::InvalidData(
            "durable approval conflicts with persisted blueprint".into(),
        ));
    }
    Ok(Some(approval))
}

/// Fetch the approval bound to one durable task.
pub async fn get_approval(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    task_id: Uuid,
) -> Result<Option<AgentApproval>> {
    sqlx::query(
        "SELECT token,workflow_id,run_id,task_id,step_id,request_message,step_index,approver_spec,         status::text AS status,approver_pubkey,expires_at FROM workflow_approvals          WHERE community_id=$1 AND run_id=$2 AND task_id=$3",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .bind(task_id)
    .fetch_optional(pool)
    .await?
    .map(map_approval)
    .transpose()
}

/// Atomically expire a pending approval whose fixed deadline elapsed.
pub async fn expire_approval(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    task_id: Uuid,
) -> Result<bool> {
    let affected = sqlx::query(
        "UPDATE workflow_approvals SET status='expired'          WHERE community_id=$1 AND run_id=$2 AND task_id=$3            AND status='pending' AND expires_at<=NOW()",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .bind(task_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected == 1)
}

/// Move a running lifecycle into persistent approval wait without overwriting a terminal run.
pub async fn mark_run_waiting(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    step_index: i32,
) -> Result<bool> {
    let affected = sqlx::query(
        "UPDATE workflow_runs SET status='waiting_approval',current_step=$3          WHERE community_id=$1 AND id=$2 AND status='running'",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .bind(step_index)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected == 1)
}

/// Resume a waiting lifecycle after the durable approval task wins completion.
pub async fn mark_run_running(pool: &PgPool, community: CommunityId, run_id: Uuid) -> Result<bool> {
    let affected = sqlx::query(
        "UPDATE workflow_runs SET status='running'          WHERE community_id=$1 AND id=$2 AND status='waiting_approval'",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected == 1)
}

pub(crate) fn map_approval(row: sqlx::postgres::PgRow) -> Result<AgentApproval> {
    let status = row.try_get::<String, _>("status")?.parse()?;
    let task_id = row
        .try_get::<Option<Uuid>, _>("task_id")?
        .ok_or_else(|| DbError::InvalidData("durable approval is missing task_id".into()))?;
    Ok(AgentApproval {
        token: row.try_get("token")?,
        workflow_id: row.try_get("workflow_id")?,
        run_id: row.try_get("run_id")?,
        task_id,
        step_id: row.try_get("step_id")?,
        request_message: row.try_get("request_message")?,
        step_index: row.try_get("step_index")?,
        approver_spec: row.try_get("approver_spec")?,
        status,
        approver_pubkey: row.try_get("approver_pubkey")?,
        expires_at: row.try_get("expires_at")?,
    })
}
