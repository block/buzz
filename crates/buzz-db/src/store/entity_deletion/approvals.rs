//! Approval evidence bound to the saved inventory fingerprint.
use super::requests::REQUEST_COLUMNS;
use super::*;

/// One retained approval of a particular request inventory.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EntityApproval {
    /// Stable identifier; identical approval retries retain it.
    pub id: Uuid,
    /// Parent request, checked and locked by the approval transaction.
    pub request_id: Uuid,
    /// SHA-256 of the inventory this operator approved.
    pub inventory_digest: String,
    /// Operator identifier supplied by the caller; this store does not authenticate it.
    pub approved_by: String,
    /// Time this approval was first recorded.
    pub approved_at: DateTime<Utc>,
}

impl EntityDeletionStore {
    /// Record approval of the saved inventory without creating or executing stages.
    ///
    /// An identical retry is read-only. Another operator adds retained evidence
    /// without restarting the workflow. This store does not authenticate the
    /// operator: today's dummy CLI accepts self-asserted operator names. Callers
    /// enabling real deletion must authenticate and authorize the operator.
    pub async fn approve(
        &self,
        id: Uuid,
        approved_by: &str,
        expected_inventory_digest: &str,
    ) -> Result<EntityRequest> {
        self.approve_validated(id, approved_by, expected_inventory_digest, |_| Ok(()))
            .await
    }

    pub(crate) async fn approve_validated<F>(
        &self,
        id: Uuid,
        approved_by: &str,
        expected_inventory_digest: &str,
        validate: F,
    ) -> Result<EntityRequest>
    where
        F: FnOnce(&EntityRequest) -> Result<()> + Send,
    {
        validate_operator(approved_by)?;
        let mut tx = self.pool.begin().await?;
        let request: EntityRequest = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {REQUEST_COLUMNS}
             FROM entity_deletion_requests WHERE id = $1 FOR UPDATE",
        )))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("entity deletion request {id}")))?;
        validate(&request)?;
        let digest = validated_inventory_digest(&request)?;
        if expected_inventory_digest != digest {
            return Err(DbError::DeletionSafety(
                "approval does not match saved inventory".into(),
            ));
        }
        let has_history: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM entity_deletion_stage_history WHERE request_id = $1)",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if has_history {
            return Err(DbError::DeletionSafety(
                "approval cannot change a started workflow".into(),
            ));
        }
        if request.status == "approved" {
            let has_approval: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM entity_deletion_approvals
                 WHERE request_id = $1 AND inventory_digest = $2)",
            )
            .bind(id)
            .bind(&digest)
            .fetch_one(&mut *tx)
            .await?;
            if !has_approval {
                return Err(DbError::DeletionSafety(
                    "approved request has no approval for its saved inventory".into(),
                ));
            }
            save_approval(&mut tx, id, approved_by, &digest).await?;
            tx.commit().await?;
            return Ok(request);
        }
        if request.status != "inventoried" {
            return Err(DbError::DeletionSafety("request is not inventoried".into()));
        }
        let request = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE entity_deletion_requests
             SET status = 'approved', updated_at = now()
             WHERE id = $1
             RETURNING {REQUEST_COLUMNS}",
        )))
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        save_approval(&mut tx, id, approved_by, &digest).await?;
        tx.commit().await?;
        Ok(request)
    }
}

// The caller holds the parent row lock and has validated inventory and lifecycle.
async fn save_approval(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request_id: Uuid,
    approved_by: &str,
    inventory_digest: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO entity_deletion_approvals (request_id, inventory_digest, approved_by)
         VALUES ($1, $2, $3)
         ON CONFLICT (request_id, inventory_digest, approved_by) DO NOTHING",
    )
    .bind(request_id)
    .bind(inventory_digest)
    .bind(approved_by)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
