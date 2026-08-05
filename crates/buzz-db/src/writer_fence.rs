//! Database-backed epoch/lease fencing for relay writers.
//!
//! The lease is authoritative in PostgreSQL. Every writer-pool connection is
//! stamped with the acquired `(resource, epoch, holder)` tuple, while the
//! migration-installed triggers re-check the live row at mutation time. A
//! stale process therefore fails at the database boundary even when it still
//! has an old pooled connection.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sha2::Digest;
use sqlx::postgres::{PgConnection, PgPoolOptions};
use sqlx::{PgPool, Row, Transaction};
use uuid::Uuid;

use crate::{DbError, Result};

/// PostgreSQL session GUC containing the fenced resource name.
pub const RESOURCE_GUC: &str = "buzz.writer_fence_resource";
/// PostgreSQL session GUC containing the holder epoch.
pub const EPOCH_GUC: &str = "buzz.writer_fence_epoch";
/// PostgreSQL session GUC containing the holder identity.
pub const HOLDER_GUC: &str = "buzz.writer_fence_holder";

/// Runtime configuration for the relay writer fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterFenceConfig {
    /// Whether startup must acquire a lease before the relay can serve writes.
    pub required: bool,
    /// The deployment-global resource guarded by this process.
    pub resource: String,
    /// Optional operator-visible holder label. A unique process label is
    /// generated when this is absent.
    pub holder_id: Option<String>,
    /// Lease lifetime in seconds.
    pub lease_seconds: i64,
    /// Renewal cadence in seconds.
    pub renew_interval_seconds: u64,
}

impl Default for WriterFenceConfig {
    fn default() -> Self {
        Self {
            required: false,
            resource: "buzz-relay".to_string(),
            holder_id: None,
            lease_seconds: 30,
            renew_interval_seconds: 10,
        }
    }
}

impl WriterFenceConfig {
    /// Load the fence configuration from the relay environment.
    pub fn from_env() -> std::result::Result<Self, String> {
        let required = std::env::var("BUZZ_WRITER_FENCE_REQUIRED")
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "true" | "1" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        let resource = std::env::var("BUZZ_WRITER_FENCE_RESOURCE")
            .unwrap_or_else(|_| "buzz-relay".to_string());
        let holder_id = std::env::var("BUZZ_WRITER_FENCE_HOLDER_ID")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let lease_seconds = parse_env_i64("BUZZ_WRITER_FENCE_LEASE_SECONDS", 30)?;
        let renew_interval_seconds = parse_env_u64("BUZZ_WRITER_FENCE_RENEW_INTERVAL_SECONDS", 10)?;
        let config = Self {
            required,
            resource,
            holder_id,
            lease_seconds,
            renew_interval_seconds,
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> std::result::Result<(), String> {
        if self.resource.trim().is_empty() || self.resource.len() > 128 {
            return Err("BUZZ_WRITER_FENCE_RESOURCE must be 1..=128 characters".to_string());
        }
        if !(5..=86_400).contains(&self.lease_seconds) {
            return Err("BUZZ_WRITER_FENCE_LEASE_SECONDS must be between 5 and 86400".to_string());
        }
        if self.renew_interval_seconds == 0
            || self.renew_interval_seconds >= self.lease_seconds as u64
        {
            return Err(
                "BUZZ_WRITER_FENCE_RENEW_INTERVAL_SECONDS must be positive and shorter than the lease"
                    .to_string(),
            );
        }
        if self
            .holder_id
            .as_ref()
            .is_some_and(|holder| holder.trim().is_empty() || holder.len() > 128)
        {
            return Err("BUZZ_WRITER_FENCE_HOLDER_ID must be 1..=128 characters".to_string());
        }
        Ok(())
    }

    fn resolved_holder_id(&self) -> String {
        self.holder_id.clone().unwrap_or_else(|| {
            format!(
                "buzz-relay:{}:{}",
                std::process::id(),
                Uuid::new_v4().simple()
            )
        })
    }
}

fn parse_env_i64(name: &str, default: i64) -> std::result::Result<i64, String> {
    std::env::var(name)
        .ok()
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| format!("{name} must be an integer"))
        })
        .unwrap_or(Ok(default))
}

fn parse_env_u64(name: &str, default: u64) -> std::result::Result<u64, String> {
    std::env::var(name)
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| format!("{name} must be an unsigned integer"))
        })
        .unwrap_or(Ok(default))
}

/// The session tuple stamped into every writer-pool connection.
#[derive(Debug, Clone)]
pub struct WriterFenceSession {
    resource: String,
    epoch: i64,
    holder_id: String,
}

impl WriterFenceSession {
    /// Apply the tuple to one newly-created PostgreSQL connection.
    pub async fn apply(
        &self,
        connection: &mut PgConnection,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            "SELECT \
                set_config('buzz.writer_fence_resource', $1, false), \
                set_config('buzz.writer_fence_epoch', $2, false), \
                set_config('buzz.writer_fence_holder', $3, false)",
        )
        .bind(&self.resource)
        .bind(self.epoch.to_string())
        .bind(&self.holder_id)
        .execute(connection)
        .await
        .map(|_| ())
    }
}

/// An acquired lease owned by one relay process.
#[derive(Debug, Clone)]
pub struct WriterFenceLease {
    resource: String,
    epoch: i64,
    holder_id: String,
}

impl WriterFenceLease {
    /// Acquire the next epoch in a short-lived control connection.
    pub async fn acquire(database_url: &str, config: &WriterFenceConfig) -> Result<Self> {
        config.validate().map_err(DbError::InvalidData)?;
        let holder_id = config.resolved_holder_id();
        let control_pool = PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await?;

        let row = sqlx::query(
            "SELECT epoch \
             FROM buzz_writer_fence_acquire($1, $2, $3)",
        )
        .bind(&config.resource)
        .bind(&holder_id)
        .bind(config.lease_seconds as i32)
        .fetch_one(&control_pool)
        .await?;
        let epoch: i64 = row.try_get("epoch")?;
        control_pool.close().await;
        Ok(Self {
            resource: config.resource.clone(),
            epoch,
            holder_id,
        })
    }

    /// Return the session tuple for the writer pool.
    pub fn session(&self) -> WriterFenceSession {
        WriterFenceSession {
            resource: self.resource.clone(),
            epoch: self.epoch,
            holder_id: self.holder_id.clone(),
        }
    }
}

/// A transaction that holds the writer-fence row lock for one external effect.
///
/// The caller must keep this permit alive until Redis or HTTP returns, then
/// commit it. Dropping it rolls the transaction back. Epoch acquisition takes
/// `FOR UPDATE` on the same row, so it cannot overtake an effect that already
/// passed this permit.
pub struct WriterFenceEffect {
    transaction: Option<Transaction<'static, sqlx::Postgres>>,
}

impl WriterFenceEffect {
    pub(crate) fn disabled() -> Self {
        Self { transaction: None }
    }

    fn new(transaction: Transaction<'static, sqlx::Postgres>) -> Self {
        Self {
            transaction: Some(transaction),
        }
    }

    /// Commit the effect permit after the external operation has returned.
    ///
    /// If this fails, the external operation may already have happened; callers
    /// must retry with their stable idempotency key (`event_id` or `request_id`).
    pub async fn commit(mut self) -> Result<()> {
        if let Some(transaction) = self.transaction.take() {
            transaction.commit().await?;
        }
        Ok(())
    }
}

/// A live lease with renewal and fail-closed authorization.
#[derive(Debug)]
pub struct WriterFence {
    pool: PgPool,
    lease: WriterFenceLease,
    lease_seconds: i32,
    lost: AtomicBool,
    renew_interval: Duration,
}

impl WriterFence {
    /// Start renewal for an already-acquired lease.
    pub fn start(pool: PgPool, lease: WriterFenceLease, config: &WriterFenceConfig) -> Arc<Self> {
        let fence = Arc::new(Self {
            pool,
            lease,
            lease_seconds: config.lease_seconds as i32,
            lost: AtomicBool::new(false),
            renew_interval: Duration::from_secs(config.renew_interval_seconds),
        });
        let renewer = Arc::clone(&fence);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(renewer.renew_interval).await;
                if let Err(error) = renewer.renew_once().await {
                    renewer.lost.store(true, Ordering::Release);
                    tracing::error!(error = %error, "writer-fence lease renewal failed; fencing this process");
                    break;
                }
            }
        });
        fence
    }

    /// Return whether this process still owns the lease locally.
    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    /// Begin an external effect while holding a shared lock on the live fence.
    ///
    /// The returned transaction must remain open until the external operation
    /// completes. The database acquisition path takes an incompatible row lock
    /// when replacing the epoch, which gives the effect a database-backed
    /// linearization point instead of a check-then-send race.
    pub async fn begin_effect(&self, effect_key: &str) -> Result<WriterFenceEffect> {
        if self.is_lost() {
            return Err(DbError::WriterFence("writer lease lost".to_string()));
        }
        if effect_key.trim().is_empty() || effect_key.len() > 256 {
            return Err(DbError::InvalidData(
                "writer-fence effect key must be 1..=256 characters".to_string(),
            ));
        }

        let mut transaction = self.pool.begin().await?;
        let permitted: bool =
            sqlx::query_scalar("SELECT buzz_writer_fence_begin_effect($1, $2, $3, $4)")
                .bind(&self.lease.resource)
                .bind(self.lease.epoch)
                .bind(&self.lease.holder_id)
                .bind(effect_key)
                .fetch_one(&mut *transaction)
                .await?;
        if !permitted {
            return Err(DbError::WriterFence(
                "writer-fence external effect denied".to_string(),
            ));
        }
        Ok(WriterFenceEffect::new(transaction))
    }

    /// Revalidate the current epoch/holder/expiry against PostgreSQL.
    pub async fn assert_current(&self) -> Result<()> {
        if self.is_lost() {
            return Err(DbError::WriterFence("writer lease lost".to_string()));
        }
        let row = sqlx::query(
            "SELECT epoch, holder_id, mode, lease_until \
             FROM buzz_writer_fence_state($1)",
        )
        .bind(&self.lease.resource)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Err(DbError::WriterFence("writer lease missing".to_string()));
        };
        let epoch: i64 = row.try_get("epoch")?;
        let holder_id: String = row.try_get("holder_id")?;
        let mode: String = row.try_get("mode")?;
        let lease_until: DateTime<Utc> = row.try_get("lease_until")?;
        if epoch != self.lease.epoch {
            return Err(DbError::WriterFence("writer epoch stale".to_string()));
        }
        if holder_id != self.lease.holder_id {
            return Err(DbError::WriterFence("writer holder mismatch".to_string()));
        }
        if mode != "active" || lease_until <= Utc::now() {
            return Err(DbError::WriterFence(
                "writer lease expired or fenced".to_string(),
            ));
        }
        Ok(())
    }

    async fn renew_once(&self) -> Result<()> {
        let result = sqlx::query("SELECT buzz_writer_fence_renew($1, $2, $3, $4) AS renewed")
            .bind(&self.lease.resource)
            .bind(self.lease.epoch)
            .bind(&self.lease.holder_id)
            .bind(self.lease_seconds)
            .fetch_one(&self.pool)
            .await?;
        let renewed: bool = result.try_get("renewed")?;
        if !renewed {
            return Err(DbError::WriterFence(
                "writer lease renewal rejected".to_string(),
            ));
        }
        Ok(())
    }

    /// Return a sanitized state snapshot suitable for an operator receipt.
    pub async fn sanitized_state(&self) -> Result<WriterFenceState> {
        let row = sqlx::query(
            "SELECT epoch, holder_id, mode, lease_until, updated_at \
             FROM buzz_writer_fence_state($1)",
        )
        .bind(&self.lease.resource)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| DbError::WriterFence("writer lease missing".to_string()))?;
        let holder_id: String = row.try_get("holder_id")?;
        Ok(WriterFenceState {
            resource: self.lease.resource.clone(),
            epoch: row.try_get("epoch")?,
            mode: row.try_get("mode")?,
            lease_until: row.try_get("lease_until")?,
            updated_at: row.try_get("updated_at")?,
            holder_sha256: hex::encode(sha2::Sha256::digest(holder_id.as_bytes())),
        })
    }
}

/// Sanitized operator-facing lease state.
#[derive(Debug, Clone)]
pub struct WriterFenceState {
    /// Guarded resource.
    pub resource: String,
    /// Current monotonically increasing epoch.
    pub epoch: i64,
    /// Current lifecycle mode.
    pub mode: String,
    /// Current lease expiry.
    pub lease_until: DateTime<Utc>,
    /// Last control-plane update.
    pub updated_at: DateTime<Utc>,
    /// SHA-256 of the holder identity; the raw holder is never exposed.
    pub holder_sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_configuration_is_fail_open_only_for_unconfigured_dev_instances() {
        let config = WriterFenceConfig::default();
        assert!(!config.required);
        assert_eq!(config.resource, "buzz-relay");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn required_configuration_rejects_a_renewal_interval_that_can_outlive_the_lease() {
        let config = WriterFenceConfig {
            required: true,
            lease_seconds: 10,
            renew_interval_seconds: 10,
            ..WriterFenceConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn generated_holder_id_is_nonempty_and_bounded() {
        let holder = WriterFenceConfig::default().resolved_holder_id();
        assert!(!holder.is_empty());
        assert!(holder.len() <= 128);
    }
}
