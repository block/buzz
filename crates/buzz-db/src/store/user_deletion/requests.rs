//! Submit a user request and its discovered inventory in one transaction.
use super::{discovery::normalize_target, UserDeletionStore};
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
        let target = normalize_target(target)?;
        self.submit_mode(&target, requested_by, reason, false).await
    }

    /// Save an independently approved print rehearsal for a real public key.
    /// Its opaque request target cannot be accepted by destructive handlers.
    pub async fn submit_rehearsal(
        &self,
        pubkey: &str,
        requested_by: &str,
        reason: Option<&str>,
    ) -> Result<EntityRequest> {
        let target = super::normalize_user_deletion_pubkey(pubkey)?;
        self.submit_mode(&target, requested_by, reason, true).await
    }

    async fn submit_mode(
        &self,
        subject: &str,
        requested_by: &str,
        reason: Option<&str>,
        rehearsal: bool,
    ) -> Result<EntityRequest> {
        let target = if rehearsal {
            format!("rehearsal:{subject}")
        } else {
            subject.to_owned()
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
            let mut inventory = super::discovery::generate_inventory_on(&mut tx, subject).await?;
            if rehearsal {
                inventory["execution_mode"] = "rehearsal".into();
            }
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
            // Use the approval validator before committing either the request
            // or inventory, so unsupported scopes leave no new request behind.
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
