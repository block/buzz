//! Atomic recovery of timed-out durable agent tasks.

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::agent_workflow::{map_task, AgentTask};
use crate::error::Result;

/// Retry or fail one running task only when its persisted attempt deadline elapsed.
pub async fn recover_timed_out_task(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
    expected_version: i64,
    timeout_secs: i64,
    retry_at: DateTime<Utc>,
) -> Result<Option<AgentTask>> {
    sqlx::query(
        "UPDATE workflow_run_tasks SET          status=CASE WHEN attempt>=max_attempts THEN 'failed' ELSE 'retry_scheduled' END,         not_before=CASE WHEN attempt>=max_attempts THEN NULL ELSE $5 END,         completed_at=CASE WHEN attempt>=max_attempts THEN NOW() ELSE NULL END,         error_code=CASE WHEN attempt>=max_attempts THEN 'agent_timeout_exhausted' ELSE 'agent_timeout' END,         error_message='agent task exceeded its persisted attempt timeout',         version=version+1,updated_at=NOW()          WHERE community_id=$1 AND id=$2 AND version=$3 AND status='running'            AND started_at IS NOT NULL            AND started_at + ($4 * INTERVAL '1 second') <= NOW()          RETURNING id,run_id,task_key,phase,agent_pubkey,status,attempt,max_attempts,input,          output_schema,idempotency_key,parent_task_id,depends_on,not_before,started_at,          completed_at,error_code,error_message,version,created_at,updated_at",
    )
    .bind(community.as_uuid())
    .bind(task_id)
    .bind(expected_version)
    .bind(timeout_secs)
    .bind(retry_at)
    .fetch_optional(pool)
    .await?
    .map(map_task)
    .transpose()
}
