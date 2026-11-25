//! Submit a user request and its discovered inventory in one transaction.
use super::{normalize_user_deletion_pubkey, UserDeletionStore};
use crate::entity_deletion::EntityRequest;
use crate::store::entity_deletion::requests::{
    inventory_digest, validate_operator, REQUEST_COLUMNS,
};
use crate::{DbError, Result};
impl UserDeletionStore {
    /// Save a request or return an identical previous submission.
    ///
    /// A conflicting operator or reason returns an error without changing the
    /// existing request. Concurrent identical submissions converge on one row.
    /// Legacy submitted rows acquire their inventory on matching resubmission.
    /// Hex and npub inputs normalize to the same target; saved inventories are not refreshed.
    pub async fn submit(
        &self,
        target: &str,
        requested_by: &str,
        reason: Option<&str>,
    ) -> Result<EntityRequest> {
        self.submit_for_mode(target, requested_by, reason, false)
            .await
    }

    /// Freeze one inventory per subject and mode. Separate progress prevents a
    /// read-only stage from satisfying a later deletion of the same subject.
    pub async fn submit_for_mode(
        &self,
        subject: &str,
        requested_by: &str,
        reason: Option<&str>,
        read_only: bool,
    ) -> Result<EntityRequest> {
        let subject = normalize_user_deletion_pubkey(subject)?;
        let target = if read_only {
            format!("read-only:{subject}")
        } else {
            subject.clone()
        };
        validate_operator(requested_by)?;
        if reason.is_some_and(|r| r.len() > 4096) {
            return Err(DbError::DeletionSafety("reason exceeds 4096 bytes".into()));
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO entity_deletion_requests (target, requested_by, reason)
             VALUES ($1, $2, $3) ON CONFLICT (entity_type, target) DO NOTHING",
        )
        .bind(&target)
        .bind(requested_by)
        .bind(reason)
        .execute(&mut *tx)
        .await?;
        let mut request: EntityRequest = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {REQUEST_COLUMNS}
             FROM entity_deletion_requests WHERE entity_type = 'user' AND target = $1 FOR UPDATE",
        )))
        .bind(&target)
        .fetch_one(&mut *tx)
        .await?;
        if request.requested_by != requested_by || request.reason.as_deref() != reason {
            return Err(DbError::DeletionSafety(format!(
                "target already has request {}; inspect it before resubmission",
                request.id
            )));
        }
        if request.status == "submitted" {
            let inventory = super::discovery::generate_inventory_on(&mut tx, &subject).await?;
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
            // Validate shape and supported scope before committing either the
            // inventory or request. Unsupported execution fails, never filters.
            super::UserDeletionInventory::read(&request)?;
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
