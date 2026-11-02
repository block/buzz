//! Durable proof of the timestamp overlap used by incremental artifact reads.

use buzz_core::CommunityId;
use sqlx::PgPool;
use std::time::Duration;
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
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    tokio::time::timeout_at(deadline, async {
        let mut connection = crate::observability::acquire_writer_until(
            pool,
            crate::observability::WriterOperation::Maintenance,
            deadline,
        )
        .await?;
        // A cancelled commit has uncertain protocol state. Never reuse its connection.
        connection.close_on_drop();
        sqlx::query("SET statement_timeout='400ms'")
            .execute(&mut *connection)
            .await?;
        let tx = sqlx::Transaction::begin(connection, None).await?;
        let mut tx = match crate::AdmittedTx::admit(
            tx,
            &crate::deletion::DeletionStore::new(pool.clone()),
            community,
        )
        .await
        {
            Ok(tx) => tx,
            Err(crate::DbError::AccessDenied(_)) => return Ok(0),
            Err(error) => return Err(error),
        };
        let completed = certify_pending(&mut tx, operation).await?;
        tx.commit().await?;
        Ok(completed)
    })
    .await
    .map_err(|_| crate::DbError::Sqlx(sqlx::Error::PoolTimedOut))?
}

async fn certify_pending(tx: &mut crate::AdmittedTx, operation: Option<Uuid>) -> Result<usize> {
    let community = tx.community();
    sqlx::query("SET LOCAL statement_timeout='400ms'")
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
    /// Recover one bounded batch in the first active community with pending work.
    pub async fn recover_artifact_certifications(&self) -> Result<usize> {
        Ok(self.recover_artifact_certifications_after(None).await?.0)
    }

    /// Recover at most 100 records in the next community after `after`.
    /// The returned UUID advances the sweep even when that community is blocked.
    /// `None` means that the sweep reaches its end and can start again.
    pub async fn recover_artifact_certifications_after(
        &self,
        after: Option<Uuid>,
    ) -> Result<(usize, Option<Uuid>)> {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        let mut connection = crate::observability::acquire_writer_until(
            &self.pool,
            crate::observability::WriterOperation::Maintenance,
            deadline,
        )
        .await?;
        let next = tokio::time::timeout_at(deadline, async {
            let mut tx = sqlx::Connection::begin(&mut *connection).await?;
            sqlx::query("SET LOCAL statement_timeout='400ms'")
                .execute(&mut *tx)
                .await?;
            let next = sqlx::query_scalar::<_, Uuid>(
                "SELECT DISTINCT p.community_id FROM artifact_sync_pending p \
                 JOIN communities c ON c.id=p.community_id \
                 WHERE c.deletion_state='active' AND c.deleted_at IS NULL \
                 AND ($1::uuid IS NULL OR p.community_id>$1) \
                 ORDER BY p.community_id LIMIT 1",
            )
            .bind(after)
            .fetch_optional(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok::<_, sqlx::Error>(next)
        })
        .await;
        let next = match next {
            Ok(result) => result?,
            Err(_) => {
                drop(connection.detach());
                return Err(sqlx::Error::PoolTimedOut.into());
            }
        };
        drop(connection);
        let Some(id) = next else {
            return Ok((0, None));
        };
        let community = CommunityId::from_uuid(id);
        match certify(&self.pool, community, None).await {
            Ok(count) => Ok((count, Some(id))),
            Err(error) => {
                tracing::warn!(%error, community_id = %community,
                    "artifact certification requires recovery");
                Ok((0, Some(id)))
            }
        }
    }
}

#[cfg(test)]
#[path = "artifact_certification_postgres_tests.rs"]
mod postgres_tests;
