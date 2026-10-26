//! Submit a user request and its dummy inventory in one transaction.
use super::{
    inventory::{demo_inventory, validate_submission},
    UserDeletionStore,
};
use crate::entity_deletion::EntityRequest;
use crate::store::entity_deletion::requests::{inventory_digest, REQUEST_COLUMNS};
use crate::{DbError, Result};
impl UserDeletionStore {
    /// Save a request or return an identical previous submission.
    ///
    /// A conflicting operator or reason returns an error without changing the
    /// existing request. Concurrent identical submissions converge on one row.
    /// Legacy submitted rows acquire their demo inventory on matching resubmission.
    pub async fn submit(
        &self,
        target: &str,
        requested_by: &str,
        reason: Option<&str>,
    ) -> Result<EntityRequest> {
        validate_submission(target, requested_by, reason)?;
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
        let mut request: EntityRequest = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {REQUEST_COLUMNS}
             FROM entity_deletion_requests WHERE entity_type = 'user' AND target = $1 FOR UPDATE",
        )))
        .bind(target)
        .fetch_one(&mut *tx)
        .await?;
        if request.requested_by != requested_by || request.reason.as_deref() != reason {
            return Err(DbError::DeletionSafety(format!(
                "target already has request {}; inspect it before resubmission",
                request.id
            )));
        }
        if request.status == "submitted" {
            let inventory = demo_inventory(target);
            let digest = inventory_digest(&inventory)?;
            request = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "UPDATE entity_deletion_requests
                 SET status = 'inventoried', inventory_manifest = $2, inventory_digest = $3,
                     updated_at = now()
                 WHERE id = $1
                 RETURNING {REQUEST_COLUMNS}",
            )))
            .bind(request.id)
            .bind(inventory)
            .bind(digest)
            .fetch_one(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(request)
    }
}

impl UserDeletionStore {
    /// Validate the user inventory and save approval in the shared approval transaction.
    pub async fn approve(
        &self,
        id: uuid::Uuid,
        approved_by: &str,
        digest: &str,
    ) -> Result<EntityRequest> {
        crate::Db::from_pool(self.pool.clone())
            .entity_deletion_store()
            .approve_validated(id, approved_by, digest, |request| {
                super::UserDeletionInventory::read(request).map(|_| ())
            })
            .await
    }
}
