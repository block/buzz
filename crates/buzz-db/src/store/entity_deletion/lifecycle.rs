use std::time::Duration;

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

use super::{
    actor, lock_owned, lock_request, safety, EntityClaim, EntityDeletionStore, EntityLeaseToken,
    EntityRequest,
};

fn seconds(duration: Duration) -> Result<i64> {
    i64::try_from(duration.as_secs())
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| safety("lease/retry duration must be a positive whole number of seconds"))
}

pub(super) async fn heartbeat_row(
    tx: &mut Transaction<'_, Postgres>,
    token: &EntityLeaseToken,
    mode: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO entity_deletion_executor_heartbeats (executor_id, mode, request_id)
         VALUES ($1, $2, $3)
         ON CONFLICT (executor_id) DO UPDATE
         SET mode = EXCLUDED.mode, request_id = EXCLUDED.request_id,
             heartbeat_at = now(), draining = false, stopped_at = NULL",
    )
    .bind(&token.owner)
    .bind(mode)
    .bind(token.request_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl EntityDeletionStore {
    /// Claim one approved due request; expired owners are replaced atomically.
    pub async fn claim(
        &self,
        id: Option<Uuid>,
        owner: &str,
        duration: Duration,
    ) -> Result<Option<EntityClaim>> {
        actor(owner)?;
        let duration = seconds(duration)?;
        let request: Option<EntityRequest> = sqlx::query_as(
            "WITH candidate AS (
                 SELECT r.id FROM entity_deletion_requests r
                 WHERE ($1::uuid IS NULL OR r.id = $1)
                   AND r.status IN ('approved', 'running') AND r.blocked_at IS NULL
                   AND r.next_attempt_at <= clock_timestamp()
                   AND (r.lease_until IS NULL OR r.lease_until <= clock_timestamp())
                   AND EXISTS (SELECT 1 FROM entity_deletion_approvals a
                     WHERE a.request_id = r.id AND a.inventory_digest = r.inventory_digest)
                 ORDER BY r.created_at, r.id FOR UPDATE SKIP LOCKED LIMIT 1
             )
             UPDATE entity_deletion_requests r
             SET status = 'running', lease_owner = $2, lease_generation = lease_generation + 1,
                 lease_until = clock_timestamp() + make_interval(secs => $3),
                 attempts = attempts + 1, updated_at = now()
             FROM candidate WHERE r.id = candidate.id RETURNING r.*",
        )
        .bind(id)
        .bind(owner)
        .bind(duration)
        .fetch_optional(&self.pool)
        .await?;
        Ok(request.map(|request| {
            let lease = EntityLeaseToken {
                request_id: request.id,
                owner: owner.into(),
                generation: request.lease_generation,
            };
            EntityClaim { request, lease }
        }))
    }

    /// Renew parent ownership and save executor liveness in the same transaction.
    pub async fn heartbeat(
        &self,
        token: &EntityLeaseToken,
        mode: &str,
        duration: Duration,
    ) -> Result<()> {
        let duration = seconds(duration)?;
        let mut tx = self.pool.begin().await?;
        lock_owned(&mut tx, token).await?;
        sqlx::query(
            "UPDATE entity_deletion_requests
             SET lease_until = clock_timestamp() + make_interval(secs => $2), updated_at = now()
             WHERE id = $1",
        )
        .bind(token.request_id)
        .bind(duration)
        .execute(&mut *tx)
        .await?;
        heartbeat_row(&mut tx, token, mode).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Release only this generation and record executor shutdown.
    pub async fn stop_executor(&self, token: Option<&EntityLeaseToken>, owner: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        if let Some(token) = token {
            sqlx::query(
                "UPDATE entity_deletion_requests SET lease_owner = NULL, lease_until = NULL,
                     updated_at = now() WHERE id = $1 AND lease_owner = $2 AND lease_generation = $3",
            ).bind(token.request_id).bind(&token.owner).bind(token.generation)
                .execute(&mut *tx).await?;
        }
        sqlx::query(
            "UPDATE entity_deletion_executor_heartbeats SET draining = true, stopped_at = now(),
                 heartbeat_at = now() WHERE executor_id = $1",
        )
        .bind(owner)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Persist failure at the current stage and release ownership for a bounded retry.
    pub async fn record_failure(
        &self,
        token: &EntityLeaseToken,
        stage_id: Uuid,
        error: &str,
        permanent: bool,
        delay: Duration,
    ) -> Result<()> {
        let delay = seconds(delay)?;
        let message: String = error.chars().take(4096).collect();
        let mut tx = self.pool.begin().await?;
        let request = lock_owned(&mut tx, token).await?;
        let affected = sqlx::query(
            "UPDATE entity_deletion_stage_history
             SET status = 'failed', started_at = COALESCE(started_at, now()),
                 last_error = $3, updated_at = now()
             WHERE request_id = $1 AND id = $2 AND is_current",
        )
        .bind(token.request_id)
        .bind(stage_id)
        .bind(&message)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(safety("failed stage is no longer current"));
        }
        let blocked = permanent || request.retry_count >= 4;
        sqlx::query(
            "UPDATE entity_deletion_requests
             SET retry_count = retry_count + 1, last_error = $2, last_error_at = now(),
                 next_attempt_at = clock_timestamp() + make_interval(secs => $3),
                 blocked_at = CASE WHEN $4 THEN now() ELSE NULL END,
                 blocked_reason = CASE WHEN $4 THEN $2 ELSE NULL END,
                 lease_owner = NULL, lease_until = NULL, updated_at = now()
             WHERE id = $1",
        )
        .bind(token.request_id)
        .bind(&message)
        .bind(delay)
        .bind(blocked)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Resume blocked work without changing completed history or the current step.
    pub async fn unblock(
        &self,
        id: Uuid,
        unblocked_by: &str,
        reason: &str,
    ) -> Result<EntityRequest> {
        actor(unblocked_by)?;
        if reason.trim().is_empty() || reason.len() > 4096 {
            return Err(safety(
                "unblock requires a nonempty reason of at most 4096 bytes",
            ));
        }
        let mut tx = self.pool.begin().await?;
        let request = lock_request(&mut tx, id).await?;
        if request.blocked_at.is_none() || request.lease_owner.is_some() {
            return Err(safety("only an unowned blocked request can be unblocked"));
        }
        let affected = sqlx::query(
            "UPDATE entity_deletion_stage_history SET metadata_json = metadata_json || $2,
                 updated_at = now() WHERE request_id = $1 AND is_current",
        )
        .bind(id)
        .bind(serde_json::json!({"last_unblock": {
            "by": unblocked_by, "reason": reason, "at": chrono::Utc::now()
        }}))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(safety("blocked request has no current stage"));
        }
        let request = sqlx::query_as(
            "UPDATE entity_deletion_requests SET blocked_at = NULL, blocked_reason = NULL,
                 retry_count = 0, next_attempt_at = now(), last_error = NULL,
                 last_error_at = NULL, updated_at = now() WHERE id = $1 RETURNING *",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(request)
    }
}
