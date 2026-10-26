//! Parent-request ownership and stage attempts for deletion workflows.

use std::time::Duration;

use super::requests::REQUEST_COLUMNS;
use super::*;

pub(super) type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

/// Ownership token; this generation fences executor writes, not serving traffic.
#[derive(Debug, Clone)]
pub struct EntityLease {
    /// Claimed parent request.
    pub request_id: Uuid,
    /// Worker identity recorded on the parent.
    pub owner: String,
    /// Incremented whenever a worker claims or reclaims the request.
    pub generation: i64,
}

/// Execution fields stored on the parent, exposed by inspection.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EntityExecution {
    /// Worker that currently owns the request, if any.
    pub lease_owner: Option<String>,
    /// Monotonic claim generation.
    pub lease_generation: i64,
    /// Database-clock deadline for ownership.
    pub lease_expires_at: Option<DateTime<Utc>>,
    /// Most recent claim or heartbeat time.
    pub heartbeat_at: Option<DateTime<Utc>>,
    /// Recorded failures of the current stage; reset on advancement.
    pub retry_count: i32,
    /// Earliest time another run may retry a failed stage.
    pub next_attempt_at: Option<DateTime<Utc>>,
    /// Last recorded stage failure.
    pub last_error: Option<String>,
    /// Time the workflow completed.
    pub completed_at: Option<DateTime<Utc>>,
}

pub(super) async fn read_execution(tx: &mut Tx<'_>, id: Uuid) -> Result<EntityExecution> {
    Ok(sqlx::query_as(
        "SELECT lease_owner, lease_generation, lease_expires_at, heartbeat_at,
                retry_count, next_attempt_at, last_error, completed_at
         FROM entity_deletion_requests WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?)
}

impl EntityDeletionStore {
    /// Claim one approved, due, unleased request; concurrent claimers cannot both win.
    ///
    /// Expired running requests resume their current row with a new generation.
    /// A completed, unapproved, not-yet-due, or already-owned request returns `None`.
    pub async fn claim(
        &self,
        id: Uuid,
        owner: &str,
        lease_duration: Duration,
    ) -> Result<Option<EntityLease>> {
        validate_operator(owner)?;
        let millis = duration_millis(lease_duration)?;
        let mut tx = self.pool.begin().await?;
        let request: Option<EntityRequest> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {REQUEST_COLUMNS}
             FROM entity_deletion_requests
             WHERE id = $1 AND status IN ('approved', 'running', 'retry_wait')
               AND (lease_expires_at IS NULL OR lease_expires_at <= clock_timestamp())
               AND (next_attempt_at IS NULL OR next_attempt_at <= clock_timestamp())
             FOR UPDATE SKIP LOCKED",
        )))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(request) = request else {
            return Ok(None);
        };
        validate_approved_inventory(&mut tx, &request).await?;
        let generation = sqlx::query_scalar(
            "UPDATE entity_deletion_requests
             SET lease_owner = $2,
                 lease_generation = lease_generation + 1,
                 lease_expires_at = clock_timestamp() + $3 * interval '1 millisecond',
                 heartbeat_at = clock_timestamp(),
                 updated_at = clock_timestamp()
             WHERE id = $1 RETURNING lease_generation",
        )
        .bind(id)
        .bind(owner)
        .bind(millis)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(EntityLease {
            request_id: id,
            owner: owner.to_owned(),
            generation,
        }))
    }

    /// Renew an unexpired lease. An expired or superseded worker cannot revive it.
    pub async fn heartbeat(&self, lease: &EntityLease, duration: Duration) -> Result<()> {
        let millis = duration_millis(duration)?;
        let mut tx = self.pool.begin().await?;
        lock_owned(&mut tx, lease).await?;
        sqlx::query(
            "UPDATE entity_deletion_requests
             SET lease_expires_at = clock_timestamp() + $2 * interval '1 millisecond',
                 heartbeat_at = clock_timestamp()
             WHERE id = $1",
        )
        .bind(lease.request_id)
        .bind(millis)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Release this live claim after a pause or shutdown, without advancing its stage.
    ///
    /// Returns false after ownership loss; never clears a successor's lease.
    pub async fn release(&self, lease: &EntityLease) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE entity_deletion_requests SET lease_owner = NULL, lease_expires_at = NULL,
                 updated_at = clock_timestamp()
             WHERE id = $1 AND lease_owner = $2 AND lease_generation = $3
               AND lease_expires_at > clock_timestamp()",
        )
        .bind(lease.request_id)
        .bind(&lease.owner)
        .bind(lease.generation)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }
}

pub(super) async fn lock_owned(tx: &mut Tx<'_>, lease: &EntityLease) -> Result<EntityRequest> {
    // Lock by id first, then check expiry after any wait for that lock. An old
    // snapshot/transaction start time must not extend a worker's authority.
    let request: EntityRequest = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {REQUEST_COLUMNS}
         FROM entity_deletion_requests WHERE id = $1 FOR UPDATE",
    )))
    .bind(lease.request_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| stale_lease(lease))?;
    let owned: bool = sqlx::query_scalar(
        "SELECT COALESCE(status IN ('approved', 'running', 'retry_wait', 'completed') AND lease_owner = $2 AND lease_generation = $3
                AND lease_expires_at > clock_timestamp(), false)
         FROM entity_deletion_requests WHERE id = $1",
    )
    .bind(lease.request_id)
    .bind(&lease.owner)
    .bind(lease.generation)
    .fetch_one(&mut **tx)
    .await?;
    if !owned {
        return Err(stale_lease(lease));
    }
    Ok(request)
}

pub(super) async fn validate_approved_inventory(
    tx: &mut Tx<'_>,
    request: &EntityRequest,
) -> Result<()> {
    let digest = validated_inventory_digest(request)?;
    let approved: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM entity_deletion_approvals
         WHERE request_id = $1 AND inventory_digest = $2)",
    )
    .bind(request.id)
    .bind(digest)
    .fetch_one(&mut **tx)
    .await?;
    if !approved {
        return Err(DbError::DeletionSafety(
            "saved inventory has no approval".into(),
        ));
    }
    Ok(())
}

pub(super) fn duration_millis(duration: Duration) -> Result<i64> {
    if duration.is_zero() || duration > Duration::from_secs(3600) || duration.as_millis() == 0 {
        return Err(DbError::InvalidData(
            "lease/retry duration must be 1ms..1h".into(),
        ));
    }
    Ok(duration.as_millis() as i64)
}

fn stale_lease(lease: &EntityLease) -> DbError {
    // Use the existing deletion lease classifier for executor control flow.
    DbError::AccessDenied(format!(
        "stale deletion lease {} owner {:?} generation {}",
        lease.request_id, lease.owner, lease.generation
    ))
}
