//! Atomic persistence of validated agent artifact receipts.

use buzz_core::CommunityId;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::agent_workflow::{
    map_artifact, map_task, AgentArtifact, AgentTask, CreateAgentArtifact,
};
use crate::error::{DbError, Result};

/// Persist a validated artifact and complete its active task atomically.
///
/// A replay of the same idempotency key after completion is accepted only when
/// every immutable artifact field matches. A different artifact for a completed
/// task fails closed.
pub async fn persist_and_complete(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
    expected_version: i64,
    artifact: CreateAgentArtifact<'_>,
) -> Result<Option<(AgentTask, AgentArtifact)>> {
    let mut transaction = pool.begin().await?;
    let task_row = sqlx::query(
        "SELECT status,version,run_id FROM workflow_run_tasks          WHERE community_id=$1 AND id=$2 FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(task_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(task_row) = task_row else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let status: String = task_row.try_get("status")?;
    let version: i64 = task_row.try_get("version")?;
    let run_id: Uuid = task_row.try_get("run_id")?;
    if run_id != artifact.run_id || artifact.task_id != Some(task_id) {
        transaction.rollback().await?;
        return Err(DbError::InvalidData(
            "artifact does not belong to its durable task".into(),
        ));
    }
    if status == "completed" {
        let existing = sqlx::query(
            "SELECT id,run_id,task_id,kind,version,content_type,uri,sha256,inline_content,             metadata,created_by,idempotency_key,created_at FROM workflow_run_artifacts              WHERE community_id=$1 AND run_id=$2 AND idempotency_key=$3",
        )
        .bind(community.as_uuid())
        .bind(run_id)
        .bind(artifact.idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?;
        transaction.rollback().await?;
        let Some(existing) = existing else {
            return Err(DbError::InvalidData(
                "completed task has no artifact for receipt replay".into(),
            ));
        };
        assert_artifact_matches(&map_artifact(existing)?, &artifact)?;
        return Ok(None);
    }
    if !matches!(status.as_str(), "running" | "waiting") || version != expected_version {
        transaction.rollback().await?;
        return Ok(None);
    }
    let artifact_row = sqlx::query(
        "INSERT INTO workflow_run_artifacts          (community_id,run_id,task_id,kind,version,content_type,uri,sha256,inline_content,          metadata,created_by,idempotency_key)          VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)          ON CONFLICT (community_id,run_id,idempotency_key) DO UPDATE          SET idempotency_key=EXCLUDED.idempotency_key          RETURNING id,run_id,task_id,kind,version,content_type,uri,sha256,inline_content,          metadata,created_by,idempotency_key,created_at",
    )
    .bind(community.as_uuid())
    .bind(artifact.run_id)
    .bind(artifact.task_id)
    .bind(artifact.kind)
    .bind(artifact.version)
    .bind(artifact.content_type)
    .bind(artifact.uri)
    .bind(artifact.sha256)
    .bind(artifact.inline_content)
    .bind(artifact.metadata)
    .bind(artifact.created_by)
    .bind(artifact.idempotency_key)
    .fetch_one(&mut *transaction)
    .await?;
    let persisted = map_artifact(artifact_row)?;
    assert_artifact_matches(&persisted, &artifact)?;
    let task_row = sqlx::query(
        "UPDATE workflow_run_tasks SET status='completed',completed_at=NOW(),         version=version+1,updated_at=NOW()          WHERE community_id=$1 AND id=$2 AND version=$3            AND status IN ('running','waiting')          RETURNING id,run_id,task_key,phase,agent_pubkey,status,attempt,max_attempts,input,          output_schema,idempotency_key,parent_task_id,depends_on,not_before,started_at,          completed_at,error_code,error_message,version,created_at,updated_at",
    )
    .bind(community.as_uuid())
    .bind(task_id)
    .bind(expected_version)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(task_row) = task_row else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let completed = map_task(task_row)?;
    transaction.commit().await?;
    Ok(Some((completed, persisted)))
}

fn assert_artifact_matches(
    existing: &AgentArtifact,
    expected: &CreateAgentArtifact<'_>,
) -> Result<()> {
    if existing.run_id != expected.run_id
        || existing.task_id != expected.task_id
        || existing.kind != expected.kind
        || existing.version != expected.version
        || existing.content_type != expected.content_type
        || existing.uri.as_deref() != expected.uri
        || existing.sha256 != expected.sha256
        || existing.inline_content.as_ref() != expected.inline_content
        || existing.metadata != *expected.metadata
        || existing.created_by.as_deref() != expected.created_by
        || existing.idempotency_key != expected.idempotency_key
    {
        return Err(DbError::InvalidData(
            "artifact receipt conflicts with persisted artifact".into(),
        ));
    }
    Ok(())
}
