//! Fenced persistence of a handler waiting for external work.

use super::leases::{lock_owned, validate_approved_inventory};
use super::stages::require_running_stage;
use super::*;

impl EntityDeletionStore {
    /// Save an external dependency observation without completing or advancing
    /// the stage. Waiting is represented by `metadata_json.waiting`; the existing
    /// stage status remains `running`. It does not schedule a retry or count a
    /// failure. Future workflow routing must explicitly handle this outcome.
    pub async fn save_stage_waiting(
        &self,
        lease: &EntityLease,
        stage_id: Uuid,
        observation: serde_json::Value,
    ) -> Result<()> {
        if !observation.is_object() {
            return Err(DbError::DeletionSafety(
                "external observation must be an object".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let request = lock_owned(&mut tx, lease).await?;
        validate_approved_inventory(&mut tx, &request).await?;
        require_running_stage(&mut tx, lease, stage_id).await?;
        sqlx::query(
            "UPDATE entity_deletion_stage_history
             SET metadata_json = jsonb_set(metadata_json - 'result', '{waiting}',
                     $2 || jsonb_build_object('observed_at', clock_timestamp())),
                 last_error = NULL, updated_at = clock_timestamp()
             WHERE id = $1",
        )
        .bind(stage_id)
        .bind(observation)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE entity_deletion_requests SET last_error = NULL,
                 next_attempt_at = NULL, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(lease.request_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
