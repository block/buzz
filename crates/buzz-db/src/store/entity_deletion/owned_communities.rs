//! Journal the submission and subsequent observation of a delegated deletion.
use super::requests::REQUEST_COLUMNS;
use super::stages::{invalid, require_running_stage};
use super::*;

impl EntityDeletionStore {
    /// Record an owned community observation during submission or final review.
    /// Only the current handler's lease may amend its request's history.
    pub async fn save_owned_observation(
        &self,
        lease: &EntityLease,
        current_id: Uuid,
        owned_id: Uuid,
        observation: serde_json::Value,
        complete: bool,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let request = lock_owned(&mut tx, lease).await?;
        validate_approved_inventory(&mut tx, &request).await?;
        let current = require_running_stage(&mut tx, lease, current_id).await?;
        if request.entity_type != "user"
            || (current.id != owned_id && current.stage != "global_cleanup")
        {
            return Err(invalid("unexpected owner of community observation"));
        }
        let updated = sqlx::query(
            "UPDATE entity_deletion_stage_history
             SET status = CASE WHEN $4 THEN 'completed' ELSE 'submitted' END,
                 finished_at = CASE WHEN $4 THEN clock_timestamp() ELSE NULL END,
                 metadata_json = jsonb_set(metadata_json - 'waiting', '{result}', $3),
                 last_error = NULL, updated_at = clock_timestamp()
             WHERE id = $1 AND request_id = $2 AND stage = 'delete_owned_community'
               AND ((is_current AND id = $5 AND status = 'running') OR
                    (NOT is_current AND status IN ('submitted','completed')))",
        )
        .bind(owned_id)
        .bind(request.id)
        .bind(observation)
        .bind(complete)
        .bind(current_id)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(invalid("missing submitted community stage"));
        }
        tx.commit().await?;
        Ok(())
    }

    /// Older executors could finish a real delete while a child was pending.
    /// On explicit rerun restore the final checkpoint, preserving all earlier work.
    pub async fn resume_delegated_review(&self, id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let request: Option<EntityRequest> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {REQUEST_COLUMNS} FROM entity_deletion_requests
             WHERE id = $1 AND entity_type = 'user' AND status IN ('running','completed')
               AND (lease_expires_at IS NULL OR lease_expires_at <= clock_timestamp())
             FOR UPDATE"
        )))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(request) = request else {
            return Ok(());
        };
        let changed = sqlx::query(
            "UPDATE entity_deletion_stage_history s SET is_current = true, status = 'running',
                 finished_at = NULL, metadata_json = metadata_json - 'result' - 'transition',
                 updated_at = clock_timestamp()
             WHERE s.request_id = $1 AND s.stage = 'global_cleanup' AND s.status = 'completed'
               AND (s.is_current OR $2 = 'completed')
               AND s.metadata_json->'result'->>'execution_mode' = 'delete'
               AND NOT ((s.metadata_json->'result') ? 'summary')
               AND EXISTS (SELECT 1 FROM entity_deletion_stage_history o
                    WHERE o.request_id = s.request_id AND o.stage = 'delete_owned_community'
                      AND o.metadata_json->'result'->>'delegated' = 'true')",
        )
        .bind(id)
        .bind(&request.status)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() > 0 {
            validate_approved_inventory(&mut tx, &request).await?;
            sqlx::query(
                "UPDATE entity_deletion_requests SET status = 'running',
                completed_at = NULL, updated_at = clock_timestamp() WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
