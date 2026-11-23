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
        self.submit_mode(target, requested_by, reason, None, None)
            .await
    }

    /// Start an independent rehearsal, or retry submission with the same run ID.
    /// The ID is generated once by the caller, never by a worker invocation.
    /// Without a trusted relay key, discover directly attributed records only.
    pub async fn submit_rehearsal(
        &self,
        target: &str,
        requested_by: &str,
        reason: Option<&str>,
        run_id: uuid::Uuid,
    ) -> Result<EntityRequest> {
        self.submit_rehearsal_with_relay(target, requested_by, reason, run_id, None)
            .await
    }

    /// Include historical content attributed by this explicitly trusted relay.
    /// The attribution key is frozen with the inventory and must match on retry.
    pub async fn submit_rehearsal_with_relay(
        &self,
        target: &str,
        requested_by: &str,
        reason: Option<&str>,
        run_id: uuid::Uuid,
        relay_pubkey: Option<&str>,
    ) -> Result<EntityRequest> {
        let subject = super::normalize_user_deletion_pubkey(target)?;
        let relay_pubkey = relay_pubkey
            .map(super::normalize_user_deletion_pubkey)
            .transpose()?;
        if relay_pubkey.as_deref() == Some(&subject) {
            return Err(DbError::DeletionSafety(
                "the human and trusted relay public keys must differ".into(),
            ));
        }
        self.submit_mode(
            target,
            requested_by,
            reason,
            Some(run_id),
            relay_pubkey.as_deref(),
        )
        .await
    }

    async fn submit_mode(
        &self,
        subject: &str,
        requested_by: &str,
        reason: Option<&str>,
        run_id: Option<uuid::Uuid>,
        relay_pubkey: Option<&str>,
    ) -> Result<EntityRequest> {
        let subject = normalize_target(subject)?;
        let target = match run_id {
            Some(id) => format!("rehearsal:{id}:{subject}"),
            None => subject.clone(),
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
            let mut inventory = if run_id.is_some() {
                super::discovery::generate_rehearsal_inventory_on(&mut tx, &subject, relay_pubkey)
                    .await?
            } else {
                super::discovery::generate_inventory_on(&mut tx, &subject).await?
            };
            if let Some(id) = run_id {
                inventory["execution_mode"] = "rehearsal".into();
                inventory["run_id"] = id.to_string().into();
                inventory["workflow_version"] = 2.into();
                if let Some(key) = relay_pubkey {
                    inventory["discovery_relay_pubkey"] = key.into();
                }
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
        if run_id.is_some()
            && request
                .inventory_manifest
                .as_ref()
                .and_then(|m| m.get("discovery_relay_pubkey"))
                .and_then(serde_json::Value::as_str)
                != relay_pubkey
        {
            return Err(DbError::DeletionSafety(
                "rehearsal discovery relay differs from saved inventory; use a new run ID".into(),
            ));
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
