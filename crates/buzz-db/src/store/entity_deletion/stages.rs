use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

use super::{
    lock_owned, safety, DemoInventory, EntityDeletionStore, EntityLeaseToken, EntityRequest,
    StageRecord,
};

pub(super) async fn insert_stage(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    inventory: &DemoInventory,
    sequence: i32,
) -> Result<()> {
    let index = usize::try_from(sequence - 1).map_err(|_| safety("invalid stage sequence"))?;
    let stage = inventory
        .stages
        .get(index)
        .ok_or_else(|| safety("stage is outside the approved plan"))?;
    sqlx::query(
        "INSERT INTO entity_deletion_stage_history (request_id, sequence, target_community_id, stage)
         VALUES ($1, $2, $3, $4)",
    ).bind(id).bind(sequence).bind(stage.target_community_id).bind(&stage.stage)
        .execute(&mut **tx).await?;
    Ok(())
}

async fn current(
    tx: &mut Transaction<'_, Postgres>,
    request: &EntityRequest,
) -> Result<StageRecord> {
    let stage: StageRecord = sqlx::query_as(
        "SELECT * FROM entity_deletion_stage_history WHERE request_id = $1 AND is_current FOR UPDATE",
    ).bind(request.id).fetch_optional(&mut **tx).await?
        .ok_or_else(|| safety("runnable request has no current stage"))?;
    let inventory = request.inventory()?;
    let index =
        usize::try_from(stage.sequence - 1).map_err(|_| safety("invalid stage sequence"))?;
    let planned = inventory
        .stages
        .get(index)
        .ok_or_else(|| safety("stage is outside the approved plan"))?;
    if stage.stage != planned.stage || stage.target_community_id != planned.target_community_id {
        return Err(safety("current stage differs from the approved plan"));
    }
    Ok(stage)
}

impl EntityDeletionStore {
    /// Read the current stage for dispatch; starting it still validates ownership.
    pub async fn current_stage(&self, id: Uuid) -> Result<StageRecord> {
        sqlx::query_as(
            "SELECT * FROM entity_deletion_stage_history WHERE request_id = $1 AND is_current",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| safety("runnable request has no current stage"))
    }

    /// Start or resume the current stage under the parent ownership lock.
    pub async fn start_stage(&self, token: &EntityLeaseToken) -> Result<StageRecord> {
        let mut tx = self.pool.begin().await?;
        let request = lock_owned(&mut tx, token).await?;
        let stage = current(&mut tx, &request).await?;
        let stage = sqlx::query_as(
            "UPDATE entity_deletion_stage_history
             SET status = 'running', started_at = COALESCE(started_at, clock_timestamp()), updated_at = now()
             WHERE id = $1 RETURNING *",
        ).bind(stage.id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(stage)
    }

    /// Commit a stage result and its successor (or final completion) atomically.
    pub async fn complete_stage(
        &self,
        token: &EntityLeaseToken,
        stage_id: Uuid,
        metadata: &Value,
    ) -> Result<EntityRequest> {
        if !metadata.is_object() || serde_json::to_vec(metadata)?.len() > 16_384 {
            return Err(safety(
                "stage metadata must be an object of at most 16384 bytes",
            ));
        }
        let mut tx = self.pool.begin().await?;
        let request = lock_owned(&mut tx, token).await?;
        let stage = current(&mut tx, &request).await?;
        if stage.id != stage_id || stage.status != "running" {
            return Err(safety("only the current running stage can complete"));
        }
        let inventory = request.inventory()?;
        let completed: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM entity_deletion_stage_history
             WHERE request_id = $1 AND sequence < $2 AND status = 'completed'",
        )
        .bind(request.id)
        .bind(stage.sequence)
        .fetch_one(&mut *tx)
        .await?;
        if completed != i64::from(stage.sequence - 1) {
            return Err(safety("earlier stages have not all completed"));
        }
        sqlx::query(
            "UPDATE entity_deletion_stage_history SET status = 'completed', is_current = false,
                 finished_at = clock_timestamp(), metadata_json = metadata_json || $2, updated_at = now()
             WHERE id = $1",
        ).bind(stage.id).bind(metadata).execute(&mut *tx).await?;
        let finished = usize::try_from(stage.sequence).ok() == Some(inventory.stages.len());
        if !finished {
            insert_stage(&mut tx, request.id, &inventory, stage.sequence + 1).await?;
        }
        let request = sqlx::query_as(
            "UPDATE entity_deletion_requests
             SET status = CASE WHEN $2 THEN 'demo_completed' ELSE status END,
                 completed_at = CASE WHEN $2 THEN clock_timestamp() ELSE NULL END,
                 lease_owner = CASE WHEN $2 THEN NULL ELSE lease_owner END,
                 lease_until = CASE WHEN $2 THEN NULL ELSE lease_until END,
                 retry_count = 0, last_error = NULL, last_error_at = NULL, updated_at = now()
             WHERE id = $1 RETURNING *",
        )
        .bind(request.id)
        .bind(finished)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(request)
    }
}
