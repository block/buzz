//! Operator-global workflow state for synthetic, non-owned user deletion demos.
//! Domain handlers must never infer permission to erase data from these records.

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{Db, DbError, Result};

mod lifecycle;
mod manager;
mod model;
mod stages;

pub use model::{
    Approval, DemoCommunity, DemoInventory, EntityClaim, EntityLeaseToken, EntityRequest,
    Inspection, PlannedStage, StageRecord,
};

#[cfg(test)]
mod postgres_tests;

/// PostgreSQL adapter for operator-driven entity workflow demonstrations.
#[derive(Clone)]
pub struct EntityDeletionStore {
    pool: PgPool,
}

impl Db {
    /// Use the writer pool for durable entity deletion workflow state.
    pub fn entity_deletion_store(&self) -> EntityDeletionStore {
        EntityDeletionStore {
            pool: self.pool().clone(),
        }
    }
}

fn safety(message: impl Into<String>) -> DbError {
    DbError::DeletionSafety(message.into())
}

fn stale(id: Uuid) -> DbError {
    DbError::AccessDenied(format!("stale entity deletion lease: {id}"))
}

/// Whether a mutation was rejected because execution ownership is stale.
pub fn is_stale_lease(error: &DbError) -> bool {
    matches!(error, DbError::AccessDenied(message) if message.starts_with("stale entity deletion lease:"))
}

fn actor(value: &str) -> Result<&str> {
    if value.is_empty() || value.len() > 256 || value.trim() != value {
        return Err(safety(
            "operator/executor identity must be 1..256 bytes without surrounding whitespace",
        ));
    }
    Ok(value)
}

async fn lock_request(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<EntityRequest> {
    sqlx::query_as("SELECT * FROM entity_deletion_requests WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("entity deletion request {id}")))
}

async fn lock_owned(
    tx: &mut Transaction<'_, Postgres>,
    token: &EntityLeaseToken,
) -> Result<EntityRequest> {
    // Keep the row lock until the child mutation commits. A concurrent claim
    // cannot change the generation between ownership verification and progress.
    let request = lock_request(tx, token.request_id).await?;
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM entity_deletion_requests r
            JOIN entity_deletion_approvals a
              ON a.request_id = r.id AND a.inventory_digest = r.inventory_digest
            WHERE r.id = $1 AND r.lease_owner = $2 AND r.lease_generation = $3
              AND r.lease_until > clock_timestamp()
              AND r.status = 'running' AND r.blocked_at IS NULL
        )",
    )
    .bind(token.request_id)
    .bind(&token.owner)
    .bind(token.generation)
    .fetch_one(&mut **tx)
    .await?;
    if !valid {
        return Err(stale(token.request_id));
    }
    Ok(request)
}
