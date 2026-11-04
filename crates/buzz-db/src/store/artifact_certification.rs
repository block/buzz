//! Durable proof of the timestamp overlap used by incremental artifact reads.

#[cfg(test)]
use buzz_core::CommunityId;
#[cfg(test)]
use sqlx::PgPool;
use sqlx::{pool::PoolConnection, PgConnection, Postgres};
use std::time::Duration;
use uuid::Uuid;

use crate::{Db, Result};

/// Timestamp overlap shared by writers and incremental readers.
pub const OVERLAP_SECONDS: i64 = 120;
const BATCH: i64 = 1_000;
// Only recovery takes this lock. Serving writes do not use it.
const RECOVERY_LOCK: i64 = 0x42555a5a415243;

struct RecoveryConnection(Option<PoolConnection<Postgres>>);

impl RecoveryConnection {
    fn conn(&mut self) -> Result<&mut PgConnection> {
        self.0
            .as_deref_mut()
            .ok_or_else(|| sqlx::Error::PoolClosed.into())
    }

    fn reuse(mut self) {
        drop(self.0.take());
    }
}

impl Drop for RecoveryConnection {
    fn drop(&mut self) {
        // Cancellation or an uncertain commit must not return this connection.
        if let Some(connection) = self.0.take() {
            drop(connection.detach());
        }
    }
}

#[cfg(test)]
pub(crate) async fn certify(
    pool: &PgPool,
    community: CommunityId,
    operation: Option<Uuid>,
) -> Result<usize> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    let mut connection = RecoveryConnection(Some(
        crate::observability::acquire_writer_until(
            pool,
            crate::observability::WriterOperation::Maintenance,
            deadline,
        )
        .await?,
    ));
    let completed = tokio::time::timeout_at(deadline, async {
        let mut tx = sqlx::Connection::begin(connection.conn()?).await?;
        sqlx::raw_sql(
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED; SET LOCAL statement_timeout='400ms'",
        )
        .execute(&mut *tx)
        .await?;
        match crate::deletion::DeletionStore::new(pool.clone())
            .guard_transaction(&mut tx, community)
            .await
        {
            Ok(()) => (),
            Err(crate::DbError::AccessDenied(_)) => {
                tx.rollback().await?;
                return Ok(0);
            }
            Err(error) => return Err(error),
        }
        let completed = certify_pending(&mut tx, &[*community.as_uuid()], operation).await?;
        tx.commit().await?;
        Ok::<_, crate::DbError>(completed)
    })
    .await
    .map_err(|_| crate::DbError::Sqlx(sqlx::Error::PoolTimedOut))??;
    connection.reuse();
    Ok(completed)
}

async fn certify_pending(
    connection: &mut PgConnection,
    communities: &[Uuid],
    operation: Option<Uuid>,
) -> Result<usize> {
    let completed: i64 = sqlx::query_scalar(
        "WITH pending AS MATERIALIZED ( \
             SELECT p.community_id,p.operation_id,p.earliest_at \
             FROM artifact_sync_pending p \
             WHERE p.community_id=ANY($1) AND ($2::uuid IS NULL OR p.operation_id=$2) \
             ORDER BY p.community_id,p.earliest_at,p.operation_id \
             LIMIT $3 FOR UPDATE OF p SKIP LOCKED \
         ), observed AS MATERIALIZED ( \
             SELECT clock_timestamp() AS now FROM pending HAVING count(*) >= 0 \
         ), resets AS ( \
             UPDATE communities c SET artifact_policy_modified_at=GREATEST( \
                 clock_timestamp(),c.artifact_policy_modified_at + interval '1 microsecond') \
             WHERE c.id IN (SELECT p.community_id FROM pending p CROSS JOIN observed o \
                 WHERE p.earliest_at>o.now OR o.now-p.earliest_at>=make_interval(secs=>$4)) \
             RETURNING c.id \
         ), completed AS ( \
             DELETE FROM artifact_sync_pending p USING pending claimed \
             WHERE p.community_id=claimed.community_id AND p.operation_id=claimed.operation_id \
             RETURNING p.operation_id \
         ) SELECT count(*) FROM completed",
    )
    .bind(communities)
    .bind(operation)
    .bind(BATCH)
    .bind(OVERLAP_SECONDS as f64)
    .fetch_one(connection)
    .await?;
    Ok(completed as usize)
}

impl Db {
    /// Recover one bounded batch across active communities with pending work.
    pub async fn recover_artifact_certifications(&self) -> Result<usize> {
        Ok(self.recover_artifact_certifications_after(None).await?.0)
    }

    /// Recover at most 1,000 records across the next communities after `after`.
    /// The cursor advances even when a community is blocked. A competing worker
    /// leaves the cursor unchanged. `None` marks the end of a sweep.
    pub async fn recover_artifact_certifications_after(
        &self,
        after: Option<Uuid>,
    ) -> Result<(usize, Option<Uuid>)> {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        let mut connection = RecoveryConnection(Some(
            crate::observability::acquire_writer_until(
                &self.pool,
                crate::observability::WriterOperation::Maintenance,
                deadline,
            )
            .await?,
        ));
        let result = tokio::time::timeout_at(deadline, async {
            let mut tx = sqlx::Connection::begin(connection.conn()?).await?;
            sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL READ COMMITTED; SET LOCAL statement_timeout='400ms'")
                .execute(&mut *tx)
                .await?;
            let elected: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
                .bind(RECOVERY_LOCK)
                .fetch_one(&mut *tx)
                .await?;
            if !elected {
                tx.rollback().await?;
                return Ok((0, after));
            }
            let candidates: Vec<Uuid> = sqlx::query_scalar(
                "SELECT DISTINCT community_id FROM ( \
                    SELECT p.community_id FROM artifact_sync_pending p \
                    JOIN communities c ON c.id=p.community_id \
                    WHERE c.deletion_state='active' AND c.deleted_at IS NULL \
                    AND ($1::uuid IS NULL OR p.community_id>$1) \
                    ORDER BY p.community_id,p.earliest_at,p.operation_id LIMIT $2 \
                 ) batch ORDER BY community_id",
            )
            .bind(after)
            .bind(BATCH)
            .fetch_all(&mut *tx)
            .await?;
            if candidates.is_empty() {
                tx.commit().await?;
                return Ok((0, None));
            }
            let next = candidates.last().copied();
            let admitted: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM unnest($1::uuid[]) id \
                 WHERE pg_try_advisory_xact_lock_shared(community_deletion_lock_key(id)) \
                 ORDER BY id",
            )
            .bind(&candidates)
            .fetch_all(&mut *tx)
            .await?;
            // Read lifecycle state in a new snapshot after deletion admission.
            let active: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM communities WHERE id=ANY($1) \
                 AND deletion_state='active' AND deleted_at IS NULL \
                 AND current_setting('transaction_isolation')='read committed' \
                 ORDER BY id FOR NO KEY UPDATE SKIP LOCKED",
            )
            .bind(&admitted)
            .fetch_all(&mut *tx)
            .await?;
            let completed = if active.is_empty() {
                0
            } else {
                certify_pending(&mut tx, &active, None).await?
            };
            tx.commit().await?;
            Ok::<_, crate::DbError>((completed, next))
        })
        .await
        .map_err(|_| crate::DbError::Sqlx(sqlx::Error::PoolTimedOut))??;
        connection.reuse();
        Ok(result)
    }
}

#[cfg(test)]
#[path = "artifact_certification_postgres_tests.rs"]
mod postgres_tests;
