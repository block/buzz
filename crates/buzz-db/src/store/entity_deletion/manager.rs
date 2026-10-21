use uuid::Uuid;

use crate::Result;

use super::{
    actor, lock_request, safety, DemoInventory, EntityDeletionStore, EntityRequest, Inspection,
};

impl EntityDeletionStore {
    /// Submit a synthetic target, converging on an identical previous submission.
    pub async fn submit(
        &self,
        target: &str,
        requested_by: &str,
        reason: Option<&str>,
    ) -> Result<EntityRequest> {
        DemoInventory::for_target(target)?;
        actor(requested_by)?;
        if reason.is_some_and(|r| r.len() > 4096) {
            return Err(safety("reason exceeds 4096 bytes"));
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO entity_deletion_requests (target, requested_by, reason)
             VALUES ($1, $2, $3) ON CONFLICT (entity_type, target) DO NOTHING",
        )
        .bind(target)
        .bind(requested_by)
        .bind(reason)
        .execute(&mut *tx)
        .await?;
        let request: EntityRequest = sqlx::query_as(
            "SELECT * FROM entity_deletion_requests WHERE entity_type = 'user' AND target = $1",
        )
        .bind(target)
        .fetch_one(&mut *tx)
        .await?;
        if request.requested_by != requested_by || request.reason.as_deref() != reason {
            return Err(safety(format!(
                "target already has request {}; inspect it before resubmission",
                request.id
            )));
        }
        tx.commit().await?;
        Ok(request)
    }

    /// Freeze the fixed non-owned fixture and its plan in one transaction.
    pub async fn prepare_inventory(&self, id: Uuid) -> Result<EntityRequest> {
        let mut tx = self.pool.begin().await?;
        let request = lock_request(&mut tx, id).await?;
        if request.status != "submitted" {
            request.inventory()?;
            tx.commit().await?;
            return Ok(request);
        }
        let inventory = DemoInventory::for_target(&request.target)?;
        let request = sqlx::query_as(
            "UPDATE entity_deletion_requests
             SET inventory_manifest = $2, inventory_digest = $3, inventory_frozen_at = now(),
                 status = 'inventoried', updated_at = now() WHERE id = $1 RETURNING *",
        )
        .bind(id)
        .bind(serde_json::to_value(&inventory)?)
        .bind(inventory.digest()?)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(request)
    }

    /// Approve the frozen inventory and create the first pending stage atomically.
    pub async fn approve(&self, id: Uuid, approved_by: &str) -> Result<EntityRequest> {
        actor(approved_by)?;
        let mut tx = self.pool.begin().await?;
        let request = lock_request(&mut tx, id).await?;
        if request.requested_by == approved_by {
            return Err(safety("approval requires a different operator"));
        }
        let inventory = request.inventory()?;
        let previous: Option<String> = sqlx::query_scalar(
            "SELECT approved_by FROM entity_deletion_approvals WHERE request_id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(previous) = previous {
            if previous != approved_by {
                return Err(safety("approval is already recorded by another operator"));
            }
            tx.commit().await?;
            return Ok(request);
        }
        if request.status != "inventoried" {
            return Err(safety("request must be inventoried before approval"));
        }
        sqlx::query(
            "INSERT INTO entity_deletion_approvals (request_id, inventory_digest, approved_by)
             VALUES ($1, $2, $3)",
        )
        .bind(id)
        .bind(inventory.digest()?)
        .bind(approved_by)
        .execute(&mut *tx)
        .await?;
        super::stages::insert_stage(&mut tx, id, &inventory, 1).await?;
        let request = sqlx::query_as(
            "UPDATE entity_deletion_requests SET status = 'approved', updated_at = now()
             WHERE id = $1 RETURNING *",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(request)
    }

    /// Load a request from the authoritative writer database.
    pub async fn get(&self, id: Uuid) -> Result<EntityRequest> {
        sqlx::query_as("SELECT * FROM entity_deletion_requests WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| crate::DbError::NotFound(format!("entity deletion request {id}")))
    }

    /// List bounded operator-global requests, newest first.
    pub async fn list(&self, limit: u16) -> Result<Vec<EntityRequest>> {
        Ok(sqlx::query_as(
            "SELECT * FROM entity_deletion_requests ORDER BY created_at DESC, id LIMIT $1",
        )
        .bind(i64::from(limit.clamp(1, 1000)))
        .fetch_all(&self.pool)
        .await?)
    }

    /// Inspect a consistent snapshot while serializing with request mutations.
    pub async fn inspect(&self, id: Uuid) -> Result<Inspection> {
        let mut tx = self.pool.begin().await?;
        let request = lock_request(&mut tx, id).await?;
        let approval =
            sqlx::query_as("SELECT * FROM entity_deletion_approvals WHERE request_id = $1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let stage_history = sqlx::query_as(
            "SELECT * FROM entity_deletion_stage_history WHERE request_id = $1 ORDER BY sequence",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Inspection {
            request,
            approval,
            stage_history,
        })
    }
}
