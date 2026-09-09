//! Durable, purpose-built FounderPortal bridge convergence operations.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row as _};

use crate::{error::DbError, Result};

const MAX_BACKOFF_SECONDS: i64 = 3600;

/// Claimed operation leased to one worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimedSyncOperation {
    /// Stable idempotency key.
    pub operation_key: String,
    /// FounderPortal tenant coordinate.
    pub tenant_id: String,
    /// One of the four V1 operation names.
    pub operation_type: String,
    /// Optional actor class for member operations.
    pub actor_type: Option<String>,
    /// Optional actor id for member operations.
    pub actor_id: Option<String>,
    /// Desired-state generation.
    pub desired_version: i64,
    /// Current bounded attempt number.
    pub attempt_count: i32,
    /// Maximum attempts.
    pub max_attempts: i32,
}

fn expected_key(operation: &str, tenant: &str, actor_type: Option<&str>, actor_id: Option<&str>, version: i64) -> Option<String> {
    match (operation, actor_type, actor_id) {
        ("ensure_community" | "archive_community", None, None) => Some(format!("{operation}:{tenant}:{version}")),
        ("ensure_member" | "revoke_member", Some(kind @ ("human" | "agent")), Some(actor)) if !actor.is_empty() => Some(format!("{operation}:{tenant}:{kind}:{actor}:{version}")),
        _ => None,
    }
}

/// Insert duplicate-safely and supersede older non-terminal desired versions.
pub async fn enqueue(
    pool: &PgPool,
    operation: &str,
    tenant_id: &str,
    actor_type: Option<&str>,
    actor_id: Option<&str>,
    desired_version: i64,
) -> Result<String> {
    let key = expected_key(operation, tenant_id, actor_type, actor_id, desired_version)
        .filter(|_| desired_version > 0 && !tenant_id.is_empty() && tenant_id.trim() == tenant_id)
        .ok_or_else(|| DbError::InvalidData("invalid FounderPortal sync operation".into()))?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("{tenant_id}:{operation}:{}:{}", actor_type.unwrap_or(""), actor_id.unwrap_or("")))
        .execute(&mut *tx).await?;
    sqlx::query(
        r#"UPDATE founderportal_bridge.sync_operations SET state='superseded', lease_owner=NULL,
                  lease_expires_at=NULL, updated_at=now()
           WHERE tenant_id=$1 AND operation_type=$2
             AND actor_type IS NOT DISTINCT FROM $3 AND actor_id IS NOT DISTINCT FROM $4
             AND desired_version < $5 AND state IN ('pending','running','retryable_error')"#,
    ).bind(tenant_id).bind(operation).bind(actor_type).bind(actor_id).bind(desired_version)
     .execute(&mut *tx).await?;
    sqlx::query(
        r#"INSERT INTO founderportal_bridge.sync_operations
           (operation_key,tenant_id,operation_type,actor_type,actor_id,desired_version)
           VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (operation_key) DO NOTHING"#,
    ).bind(&key).bind(tenant_id).bind(operation).bind(actor_type).bind(actor_id).bind(desired_version)
     .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(key)
}

fn claimed(row: sqlx::postgres::PgRow) -> Result<ClaimedSyncOperation> {
    Ok(ClaimedSyncOperation {
        operation_key: row.try_get("operation_key")?, tenant_id: row.try_get("tenant_id")?,
        operation_type: row.try_get("operation_type")?, actor_type: row.try_get("actor_type")?,
        actor_id: row.try_get("actor_id")?, desired_version: row.try_get("desired_version")?,
        attempt_count: row.try_get("attempt_count")?, max_attempts: row.try_get("max_attempts")?,
    })
}

/// Claim available work, including work abandoned after lease expiry.
pub async fn claim_next(pool: &PgPool, worker: &str, lease_seconds: i64) -> Result<Option<ClaimedSyncOperation>> {
    if worker.is_empty() || lease_seconds <= 0 || lease_seconds > 3600 {
        return Err(DbError::InvalidData("invalid sync lease".into()));
    }
    let row = sqlx::query(
        r#"WITH candidate AS (
             SELECT operation_key FROM founderportal_bridge.sync_operations
             WHERE ((state IN ('pending','retryable_error') AND available_at <= now())
                    OR (state='running' AND lease_expires_at <= now()))
               AND attempt_count < max_attempts
             ORDER BY available_at, created_at FOR UPDATE SKIP LOCKED LIMIT 1)
           UPDATE founderportal_bridge.sync_operations o
           SET state='running', lease_owner=$1, lease_expires_at=now()+($2 * interval '1 second'),
               attempt_count=attempt_count+1, updated_at=now()
           FROM candidate WHERE o.operation_key=candidate.operation_key
           RETURNING o.operation_key,o.tenant_id,o.operation_type,o.actor_type,o.actor_id,
                     o.desired_version,o.attempt_count,o.max_attempts"#,
    ).bind(worker).bind(lease_seconds).fetch_optional(pool).await?;
    row.map(claimed).transpose()
}

async fn current_desired(pool: &PgPool, operation: &ClaimedSyncOperation) -> Result<bool> {
    let newest: Option<i64> = sqlx::query_scalar(
        r#"SELECT COALESCE(max(desired_version),0) FROM founderportal_bridge.sync_operations
           WHERE tenant_id=$1 AND operation_type=$2 AND actor_type IS NOT DISTINCT FROM $3
             AND actor_id IS NOT DISTINCT FROM $4 AND state <> 'superseded'"#,
    ).bind(&operation.tenant_id).bind(&operation.operation_type).bind(&operation.actor_type)
     .bind(&operation.actor_id).fetch_one(pool).await?;
    Ok(newest == Some(operation.desired_version))
}

/// Mark success only while the caller still owns the lease and desired version.
pub async fn succeed(pool: &PgPool, operation: &ClaimedSyncOperation, worker: &str) -> Result<bool> {
    if !current_desired(pool, operation).await? {
        sqlx::query("UPDATE founderportal_bridge.sync_operations SET state='superseded',lease_owner=NULL,lease_expires_at=NULL,updated_at=now() WHERE operation_key=$1 AND state='running' AND lease_owner=$2")
            .bind(&operation.operation_key).bind(worker).execute(pool).await?;
        return Ok(false);
    }
    Ok(sqlx::query("UPDATE founderportal_bridge.sync_operations SET state='succeeded',lease_owner=NULL,lease_expires_at=NULL,last_error_code=NULL,updated_at=now() WHERE operation_key=$1 AND state='running' AND lease_owner=$2 AND lease_expires_at>now()")
       .bind(&operation.operation_key).bind(worker).execute(pool).await?.rows_affected()==1)
}

/// Record a retry with bounded exponential backoff, or terminally exhaust it.
pub async fn fail_retryable(pool: &PgPool, operation: &ClaimedSyncOperation, worker: &str, error_code: &str) -> Result<bool> {
    if error_code.is_empty() || error_code.len() > 128 || error_code.trim() != error_code {
        return Err(DbError::InvalidData("invalid sync error code".into()));
    }
    let exhausted = operation.attempt_count >= operation.max_attempts;
    let exponent = operation.attempt_count.saturating_sub(1).min(20) as u32;
    let delay = 2_i64.saturating_pow(exponent).min(MAX_BACKOFF_SECONDS);
    let next: DateTime<Utc> = Utc::now() + Duration::seconds(delay);
    Ok(sqlx::query("UPDATE founderportal_bridge.sync_operations SET state=$3,available_at=$4,lease_owner=NULL,lease_expires_at=NULL,last_error_code=$5,updated_at=now() WHERE operation_key=$1 AND state='running' AND lease_owner=$2")
       .bind(&operation.operation_key).bind(worker).bind(if exhausted{"terminal_error"}else{"retryable_error"})
       .bind(next).bind(error_code).execute(pool).await?.rows_affected()==1)
}

/// Record a non-retryable classified failure.
pub async fn fail_terminal(pool: &PgPool, operation_key: &str, worker: &str, error_code: &str) -> Result<bool> {
    Ok(sqlx::query("UPDATE founderportal_bridge.sync_operations SET state='terminal_error',lease_owner=NULL,lease_expires_at=NULL,last_error_code=$3,updated_at=now() WHERE operation_key=$1 AND state='running' AND lease_owner=$2")
       .bind(operation_key).bind(worker).bind(error_code).execute(pool).await?.rows_affected()==1)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> PgPool {
        PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect isolated PostgreSQL test database")
    }

    async fn state(pool: &PgPool, key: &str) -> String {
        sqlx::query_scalar(
            "SELECT state FROM founderportal_bridge.sync_operations WHERE operation_key=$1",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .expect("read operation state")
    }

    #[test]
    fn operation_keys_are_exact_and_scoped() {
        assert_eq!(expected_key("ensure_community","t",None,None,2).as_deref(),Some("ensure_community:t:2"));
        assert_eq!(expected_key("revoke_member","t",Some("human"),Some("u"),3).as_deref(),Some("revoke_member:t:human:u:3"));
        assert!(expected_key("ensure_member","t",None,None,1).is_none());
    }

    #[tokio::test]
        async fn overlapping_delivery_is_duplicate_safe_and_claim_is_exclusive() {
        let pool = pool().await;
        let tenant = format!("sync-duplicate-{}", uuid::Uuid::new_v4());
        let (a, b) = tokio::join!(
            enqueue(&pool, "ensure_community", &tenant, None, None, 1),
            enqueue(&pool, "ensure_community", &tenant, None, None, 1),
        );
        let key = a.expect("first enqueue");
        assert_eq!(b.expect("duplicate enqueue"), key);
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM founderportal_bridge.sync_operations WHERE operation_key=$1",
        )
        .bind(&key).fetch_one(&pool).await.expect("count");
        assert_eq!(count, 1);

        let (first, second) = tokio::join!(
            claim_next(&pool, "worker-a", 60),
            claim_next(&pool, "worker-b", 60),
        );
        let claims = [first.expect("claim a"), second.expect("claim b")];
        assert_eq!(claims.iter().filter(|claim| claim.as_ref().is_some_and(|op| op.operation_key == key)).count(), 1);
    }

    #[tokio::test]
        async fn newer_version_supersedes_running_work_and_fences_stale_success() {
        let pool = pool().await;
        let tenant = format!("sync-supersede-{}", uuid::Uuid::new_v4());
        let old_key = enqueue(&pool, "ensure_member", &tenant, Some("human"), Some("user"), 1)
            .await.expect("enqueue v1");
        let old = claim_next(&pool, "lost-worker", 60).await.expect("claim").expect("v1 claim");
        assert_eq!(old.operation_key, old_key);
        let new_key = enqueue(&pool, "ensure_member", &tenant, Some("human"), Some("user"), 2)
            .await.expect("enqueue v2");
        assert_eq!(state(&pool, &old_key).await, "superseded");
        assert!(!succeed(&pool, &old, "lost-worker").await.expect("stale success fence"));
        let new = claim_next(&pool, "worker-new", 60).await.expect("claim").expect("v2 claim");
        assert_eq!(new.operation_key, new_key);
        assert!(succeed(&pool, &new, "worker-new").await.expect("current success"));
        assert_eq!(state(&pool, &new_key).await, "succeeded");
    }

    #[tokio::test]
        async fn expired_lease_is_reclaimed_and_old_owner_cannot_complete() {
        let pool = pool().await;
        let tenant = format!("sync-lease-{}", uuid::Uuid::new_v4());
        let key = enqueue(&pool, "archive_community", &tenant, None, None, 1).await.expect("enqueue");
        let old = claim_next(&pool, "lost-worker", 60).await.expect("claim").expect("operation");
        sqlx::query("UPDATE founderportal_bridge.sync_operations SET lease_expires_at=now()-interval '1 second' WHERE operation_key=$1")
            .bind(&key).execute(&pool).await.expect("expire lease");
        let reclaimed = claim_next(&pool, "replacement", 60).await.expect("reclaim").expect("operation");
        assert_eq!(reclaimed.operation_key, key);
        assert_eq!(reclaimed.attempt_count, old.attempt_count + 1);
        assert!(!succeed(&pool, &old, "lost-worker").await.expect("old owner fenced"));
        assert!(succeed(&pool, &reclaimed, "replacement").await.expect("new owner succeeds"));
    }

    #[tokio::test]
        async fn retry_is_delayed_bounded_and_exhaustion_is_terminal() {
        let pool = pool().await;
        let tenant = format!("sync-retry-{}", uuid::Uuid::new_v4());
        let key = enqueue(&pool, "revoke_member", &tenant, Some("agent"), Some("bot"), 1)
            .await.expect("enqueue");
        sqlx::query("UPDATE founderportal_bridge.sync_operations SET max_attempts=2 WHERE operation_key=$1")
            .bind(&key).execute(&pool).await.expect("bound attempts");
        let first = claim_next(&pool, "worker", 60).await.expect("claim").expect("first attempt");
        assert!(fail_retryable(&pool, &first, "worker", "redis_unavailable").await.expect("retry"));
        assert_eq!(state(&pool, &key).await, "retryable_error");
        assert!(claim_next(&pool, "other", 60).await.expect("delayed claim").is_none());
        sqlx::query("UPDATE founderportal_bridge.sync_operations SET available_at=now()-interval '1 second' WHERE operation_key=$1")
            .bind(&key).execute(&pool).await.expect("advance clock");
        let last = claim_next(&pool, "worker", 60).await.expect("claim").expect("last attempt");
        assert_eq!(last.attempt_count, 2);
        assert!(fail_retryable(&pool, &last, "worker", "redis_unavailable").await.expect("exhaust"));
        assert_eq!(state(&pool, &key).await, "terminal_error");
        assert!(claim_next(&pool, "other", 60).await.expect("terminal claim").is_none());
    }

    #[tokio::test]
        async fn actor_and_tenant_coordinates_do_not_cross_supersede() {
        let pool = pool().await;
        let suffix = uuid::Uuid::new_v4();
        let tenant_a = format!("tenant-a-{suffix}");
        let tenant_b = format!("tenant-b-{suffix}");
        let a_user = enqueue(&pool, "ensure_member", &tenant_a, Some("human"), Some("same-user"), 1).await.expect("a user");
        let b_user = enqueue(&pool, "ensure_member", &tenant_b, Some("human"), Some("same-user"), 1).await.expect("b user");
        let a_agent = enqueue(&pool, "ensure_member", &tenant_a, Some("agent"), Some("same-user"), 1).await.expect("a agent");
        enqueue(&pool, "ensure_member", &tenant_a, Some("human"), Some("same-user"), 2).await.expect("a user v2");
        assert_eq!(state(&pool, &a_user).await, "superseded");
        assert_eq!(state(&pool, &b_user).await, "pending");
        assert_eq!(state(&pool, &a_agent).await, "pending");
    }
}

