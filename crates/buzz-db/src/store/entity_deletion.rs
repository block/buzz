//! Operator-global intake for synthetic user-deletion requests.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{Db, DbError, Result};

/// A submitted demonstration request; no cleanup is authorized by this record.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EntityRequest {
    /// Stable request identifier.
    pub id: Uuid,
    /// Currently always `user`.
    pub entity_type: String,
    /// Synthetic `demo:` label, not a resolved account identity.
    pub target: String,
    /// Currently always `submitted`.
    pub status: String,
    /// Operator identity supplied at submission.
    pub requested_by: String,
    /// Optional explanation supplied at submission.
    pub reason: Option<String>,
    /// Time of the original submission.
    pub created_at: DateTime<Utc>,
    /// Time of the last change; identical resubmissions do not change it.
    pub updated_at: DateTime<Utc>,
}

/// Writer-database access to synthetic deletion requests.
#[derive(Clone)]
pub struct EntityDeletionStore {
    pool: PgPool,
}

impl Db {
    /// Use the writer pool for operator-global request intake and inspection.
    pub fn entity_deletion_store(&self) -> EntityDeletionStore {
        EntityDeletionStore {
            pool: self.pool().clone(),
        }
    }
}

impl EntityDeletionStore {
    /// Save a request or return an identical previous submission.
    ///
    /// A conflicting operator or reason returns an error without changing the
    /// existing request. Concurrent identical submissions converge on one row.
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
        let request: EntityRequest = sqlx::query_as(
            "SELECT * FROM entity_deletion_requests WHERE entity_type = 'user' AND target = $1",
        )
        .bind(target)
        .fetch_one(&mut *tx)
        .await?;
        if request.requested_by != requested_by || request.reason.as_deref() != reason {
            return Err(DbError::DeletionSafety(format!(
                "target already has request {}; inspect it before resubmission",
                request.id
            )));
        }
        tx.commit().await?;
        Ok(request)
    }

    /// Load a request from the authoritative writer database.
    pub async fn get(&self, id: Uuid) -> Result<EntityRequest> {
        sqlx::query_as("SELECT * FROM entity_deletion_requests WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("entity deletion request {id}")))
    }

    /// List up to 1,000 requests, newest first.
    pub async fn list(&self, limit: u16) -> Result<Vec<EntityRequest>> {
        Ok(sqlx::query_as(
            "SELECT * FROM entity_deletion_requests ORDER BY created_at DESC, id LIMIT $1",
        )
        .bind(i64::from(limit.clamp(1, 1000)))
        .fetch_all(&self.pool)
        .await?)
    }
}

fn validate_submission(target: &str, requested_by: &str, reason: Option<&str>) -> Result<()> {
    let slug = target.strip_prefix("demo:").unwrap_or_default();
    if slug.is_empty()
        || slug.len() > 64
        || !slug.as_bytes()[0].is_ascii_alphanumeric()
        || !slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        return Err(DbError::DeletionSafety(
            "target must be demo:<1..64 lowercase letters, digits, underscores or hyphens>".into(),
        ));
    }
    if requested_by.is_empty() || requested_by.len() > 256 || requested_by.trim() != requested_by {
        return Err(DbError::DeletionSafety(
            "operator identity must be 1..256 bytes without surrounding whitespace".into(),
        ));
    }
    if reason.is_some_and(|r| r.len() > 4096) {
        return Err(DbError::DeletionSafety("reason exceeds 4096 bytes".into()));
    }
    Ok(())
}

#[cfg(test)]
#[path = "entity_deletion_postgres_tests.rs"]
mod postgres_tests;
