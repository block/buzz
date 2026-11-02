//! Durable proof of the timestamp overlap used by incremental artifact reads.

use buzz_core::CommunityId;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{Db, Result};

/// Timestamp overlap shared by writers and incremental readers.
pub const OVERLAP_SECONDS: i64 = 120;
const BATCH: i64 = 100;

pub(crate) async fn certify(
    pool: &PgPool,
    community: CommunityId,
    operation: Option<Uuid>,
) -> Result<usize> {
    let Some(mut tx) = crate::begin_community_sweep_transaction(
        pool,
        community,
        crate::observability::WriterOperation::Maintenance,
    )
    .await?
    else {
        return Ok(0);
    };
    let completed = certify_pending(&mut tx, operation).await?;
    tx.commit().await?;
    Ok(completed)
}

async fn certify_pending(tx: &mut crate::AdmittedTx, operation: Option<Uuid>) -> Result<usize> {
    let community = tx.community();
    sqlx::query("SET LOCAL statement_timeout='2s'")
        .execute(tx.conn())
        .await?;
    let pending: Vec<(Uuid, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT operation_id,earliest_at FROM artifact_sync_pending \
         WHERE community_id=$1 AND ($2::uuid IS NULL OR operation_id=$2) \
         ORDER BY earliest_at,operation_id LIMIT $3 FOR UPDATE SKIP LOCKED",
    )
    .bind(community.as_uuid())
    .bind(operation)
    .bind(BATCH)
    .fetch_all(tx.conn())
    .await?;
    if pending.is_empty() {
        return Ok(0);
    }
    // This clock read follows a read that sees the committed pending records.
    // A delayed recovery makes the proof conservative and requires a reset.
    let now: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(tx.conn())
        .await?;
    if pending.iter().any(|(_, earliest)| {
        *earliest > now || now - *earliest >= chrono::Duration::seconds(OVERLAP_SECONDS)
    }) {
        sqlx::query("UPDATE communities SET artifact_policy_modified_at=GREATEST(clock_timestamp(),artifact_policy_modified_at + interval '1 microsecond') WHERE id=$1")
            .bind(community.as_uuid())
            .execute(tx.conn())
            .await?;
    }
    let operations: Vec<_> = pending.iter().map(|(id, _)| *id).collect();
    sqlx::query("DELETE FROM artifact_sync_pending WHERE community_id=$1 AND operation_id=ANY($2)")
        .bind(community.as_uuid())
        .bind(&operations)
        .execute(tx.conn())
        .await?;
    Ok(pending.len())
}

impl Db {
    /// Recover one bounded batch of committed artifact certifications.
    /// Multiple relay instances can run this method at the same time.
    pub async fn recover_artifact_certifications(&self) -> Result<usize> {
        let communities: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT community_id FROM \
             (SELECT p.community_id FROM artifact_sync_pending p JOIN communities c ON c.id=p.community_id WHERE c.deletion_state='active' AND c.deleted_at IS NULL ORDER BY p.earliest_at,p.community_id,p.operation_id LIMIT $1) p",
        )
        .bind(BATCH)
        .fetch_all(&self.pool)
        .await?;
        let mut completed = 0;
        for id in communities {
            let community = CommunityId::from_uuid(id);
            match certify(&self.pool, community, None).await {
                Ok(count) => completed += count,
                Err(error) => {
                    tracing::warn!(%error, community_id = %community,
                        "artifact certification requires recovery");
                }
            }
        }
        Ok(completed)
    }
}

#[cfg(test)]
#[path = "artifact_certification_postgres_tests.rs"]
mod postgres_tests;
