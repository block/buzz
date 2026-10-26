//! Shared request reads, inventory fingerprints, and consistent inspection.
use super::*;
use sha2::{Digest, Sha256};

// Shared by SELECT and RETURNING queries that decode EntityRequest.
// This constant is the only SQL interpolation; all request values stay bound.
pub(crate) const REQUEST_COLUMNS: &str =
    "id, entity_type, target, status, requested_by, reason, created_at,
     updated_at, inventory_manifest, inventory_digest";

/// A persisted request whose identity and workflow are selected by intake.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EntityRequest {
    /// Stable request identifier.
    pub id: Uuid,
    /// Workflow discriminator supplied by intake.
    pub entity_type: String,
    /// Opaque identity identifier supplied by intake.
    pub target: String,
    /// Intake, approval, execution (`running`/`retry_wait`), or `completed`.
    pub status: String,
    /// Operator identity supplied at submission.
    pub requested_by: String,
    /// Optional explanation supplied at submission.
    pub reason: Option<String>,
    /// Time of the original submission.
    pub created_at: DateTime<Utc>,
    /// Time of the last change; identical resubmissions do not change it.
    pub updated_at: DateTime<Utc>,
    /// Saved communities in processing order; absent on legacy intake.
    pub inventory_manifest: Option<serde_json::Value>,
    /// SHA-256 of the saved inventory.
    pub inventory_digest: Option<String>,
}

/// A consistent snapshot of a request, its approvals, and reached stages.
#[derive(Debug, Serialize)]
pub struct EntityInspection {
    /// Request fields remain at the top level of CLI JSON.
    #[serde(flatten)]
    pub request: EntityRequest,
    /// Parent ownership, heartbeat, and retry state.
    pub execution: EntityExecution,
    /// Retained approvals, ordered by approval time and identifier.
    pub approvals: Vec<EntityApproval>,
    /// Reached stages, including the current resume position after execution starts.
    pub stage_history: Vec<EntityStage>,
}

impl EntityDeletionStore {
    /// Inspect the parent and its history from one database snapshot.
    pub async fn inspect(&self, id: Uuid) -> Result<EntityInspection> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let request = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {REQUEST_COLUMNS}
             FROM entity_deletion_requests WHERE id = $1",
        )))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("entity deletion request {id}")))?;
        let execution = leases::read_execution(&mut tx, id).await?;
        let approvals = sqlx::query_as(
            "SELECT id, request_id, inventory_digest, approved_by, approved_at
             FROM entity_deletion_approvals WHERE request_id = $1
             ORDER BY approved_at, id",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        let stage_history = sqlx::query_as(
            "SELECT id, request_id, target_community_id, stage, status, is_current,
                    started_at, finished_at, metadata_json, last_error, updated_at
             FROM entity_deletion_stage_history WHERE request_id = $1
             ORDER BY started_at NULLS LAST, id",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(EntityInspection {
            request,
            execution,
            approvals,
            stage_history,
        })
    }

    /// Load a request from the authoritative writer database.
    pub async fn get(&self, id: Uuid) -> Result<EntityRequest> {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {REQUEST_COLUMNS}
             FROM entity_deletion_requests WHERE id = $1",
        )))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("entity deletion request {id}")))
    }

    /// List up to 1,000 requests, newest first.
    pub async fn list(&self, limit: u16) -> Result<Vec<EntityRequest>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {REQUEST_COLUMNS}
             FROM entity_deletion_requests ORDER BY created_at DESC, id LIMIT $1",
        )))
        .bind(i64::from(limit.clamp(1, 1000)))
        .fetch_all(&self.pool)
        .await?)
    }
}
pub(crate) fn validate_operator(operator: &str) -> Result<()> {
    if operator.is_empty() || operator.len() > 256 || operator.trim() != operator {
        return Err(DbError::DeletionSafety(
            "operator identity must be 1..256 bytes without surrounding whitespace".into(),
        ));
    }
    Ok(())
}

/// Compute a stable fingerprint of saved inventory, preserving array order.
pub fn inventory_digest(inventory: &serde_json::Value) -> Result<String> {
    // JSONB does not preserve object-key order. Sorting preserves the digest
    // across a database round trip while retaining the community array order.
    let mut canonical = inventory.clone();
    canonical.sort_all_objects();
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&canonical)?)))
}
pub(super) fn validated_inventory_digest(request: &EntityRequest) -> Result<String> {
    let inventory = request
        .inventory_manifest
        .as_ref()
        .ok_or_else(|| super::stages::invalid("missing inventory"))?;
    let digest = inventory_digest(inventory)?;
    if request.inventory_digest.as_deref() != Some(&digest) {
        return Err(super::stages::invalid("saved inventory digest changed"));
    }
    Ok(digest)
}
