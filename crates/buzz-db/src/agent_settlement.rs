//! Atomic terminal settlement for durable workflow runs.

use buzz_core::CommunityId;
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::agent_workflow::{map_transition, AgentRunTransition};
use crate::error::Result;

/// Atomically complete an active run and append its terminal ledger transition.
pub async fn complete_active_run(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    step_count: i32,
    metadata: &Value,
) -> Result<Option<AgentRunTransition>> {
    settle_active_run(
        pool,
        community,
        run_id,
        step_count,
        "completed",
        None,
        None,
        "all durable tasks completed",
        metadata,
    )
    .await
}

/// Atomically fail an active run and append its terminal ledger transition.
#[allow(clippy::too_many_arguments)]
pub async fn fail_active_run(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    step_count: i32,
    code: &str,
    message: &str,
    metadata: &Value,
) -> Result<Option<AgentRunTransition>> {
    settle_active_run(
        pool,
        community,
        run_id,
        step_count,
        "failed",
        Some(code),
        Some(message),
        "one or more durable tasks reached a terminal failure",
        metadata,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn settle_active_run(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    step_count: i32,
    terminal_status: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
    reason: &str,
    metadata: &Value,
) -> Result<Option<AgentRunTransition>> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text || ':' || $2::text, 0))")
        .bind(community.as_uuid())
        .bind(run_id)
        .execute(&mut *transaction)
        .await?;
    let row = sqlx::query(
        "SELECT status::text AS status FROM workflow_runs          WHERE community_id=$1 AND id=$2 FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(row) = row else {
        transaction.commit().await?;
        return Ok(None);
    };
    let previous_status: String = row.try_get("status")?;
    if !matches!(previous_status.as_str(), "running" | "waiting_approval") {
        transaction.commit().await?;
        return Ok(None);
    }
    sqlx::query(
        "UPDATE workflow_runs SET status=$4::run_status,current_step=$3,completed_at=NOW(),         error_code=$5,error_message=$6 WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .bind(step_count)
    .bind(terminal_status)
    .bind(error_code)
    .bind(error_message)
    .execute(&mut *transaction)
    .await?;
    let row = sqlx::query(
        "INSERT INTO workflow_run_transitions          (community_id,run_id,sequence,from_phase,to_phase,from_status,to_status,reason,metadata)          SELECT $1,$2,COALESCE(MAX(sequence),-1)+1,NULL,$3,$4,$3,$5,$6          FROM workflow_run_transitions WHERE community_id=$1 AND run_id=$2          RETURNING id,run_id,sequence,from_phase,to_phase,from_status,to_status,reason,          actor_pubkey,metadata,created_at",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .bind(terminal_status)
    .bind(previous_status)
    .bind(reason)
    .bind(metadata)
    .fetch_one(&mut *transaction)
    .await?;
    let transition = map_transition(row)?;
    transaction.commit().await?;
    Ok(Some(transition))
}
