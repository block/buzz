//! Deployment-wide leader election over PostgreSQL session advisory locks.

use buzz_datastore_tracing::datastore_span;
use sqlx::postgres::PgConnection;
use sqlx::Connection as _;

use crate::error::Result;
use crate::{observability, Db};

/// Owns the detached Postgres session holding a deployment-wide leader advisory lock.
///
/// The connection deliberately does not return to the main pool: session advisory
/// locks must remain bound to this exact physical connection, and each leader-only
/// loop (the usage poller, storage-usage delivery) pings it before working.
pub struct SessionLeader {
    connection: PgConnection,
}

impl SessionLeader {
    /// Returns whether the lock-owning session is still reachable.
    ///
    /// Bounded to 5 seconds — a blackholed connection (no RST) would otherwise
    /// stall the entire poller tick until the OS TCP timeout.
    pub async fn is_live(&mut self) -> bool {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.connection.ping())
            .await
            .is_ok_and(|r| r.is_ok())
    }
}

impl Db {
    /// Try to acquire the detached session advisory lock `lock_key`, making this
    /// replica the leader for the job that key names.
    ///
    /// The returned guard owns the exact connection that acquired the lock. It is
    /// detached from the shared pool so a stable leader neither returns a locked
    /// session to other callers nor permanently consumes a pool slot. Dropping the
    /// guard closes the connection and releases the session-scoped lock.
    #[datastore_span(name = "try_lock_session_leader", system = "postgresql")]
    pub async fn try_lock_session_leader(&self, lock_key: i64) -> Result<Option<SessionLeader>> {
        let mut connection = observability::acquire_writer_with_legacy_metrics(
            &self.pool,
            observability::WriterOperation::Maintenance,
        )
        .await?;
        let acquired = sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)")
            .bind(lock_key)
            .fetch_one(&mut *connection)
            .await?;
        if acquired {
            Ok(Some(SessionLeader {
                connection: connection.detach(),
            }))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::test_support::ScratchDb;

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_session_leader_lock_has_single_owner_and_releases_on_drop() {
        // Use a private scratch database — not the shared TEST_DATABASE_URL.
        // Postgres advisory locks are per-database; hardcoding the production
        // USAGE_METRICS_LOCK_KEY (0x4255_5A5A_4D45_5452) on the shared test DB
        // races any live buzz-relay on the same database (see #3619).
        let admin_url = crate::test_support::database_url();
        let (pool, _scratch) = ScratchDb::migrated(&admin_url, "usage_metrics_lock").await;
        let first = Db::from_pool(pool.clone());
        let second = Db::from_pool(pool.clone());
        // Same key as production (`buzz-relay` USAGE_METRICS_LOCK_KEY) — safe here
        // because the scratch DB is empty of other holders.
        let key = 0x4255_5A5A_4D45_5452;

        let mut leader = first
            .try_lock_session_leader(key)
            .await
            .expect("first lock attempt")
            .expect("first database handle becomes leader");
        assert!(leader.is_live().await, "lock owner remains reachable");
        assert!(
            second
                .try_lock_session_leader(key)
                .await
                .expect("second lock attempt")
                .is_none(),
            "another session cannot become leader while the guard exists"
        );

        drop(leader);
        // Dropping closes the socket; the server releases the session lock only
        // once its backend observes the disconnect, so poll briefly.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let successor = loop {
            let attempt = second
                .try_lock_session_leader(key)
                .await
                .expect("lock attempt after leader drop");
            if attempt.is_some() || tokio::time::Instant::now() >= deadline {
                break attempt;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        assert!(
            successor.is_some(),
            "dropping the detached session releases its advisory lock"
        );
        drop(successor);
    }
}
