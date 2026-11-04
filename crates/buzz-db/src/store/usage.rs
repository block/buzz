//! Leader election and community labels for the relay's usage poller.
//!
//! The relay runs no usage aggregate queries. This module owns the
//! session advisory lock that elects one leader per deployment and the
//! community id → host map that labels per-community series.

use buzz_datastore_tracing::datastore_span;
use sqlx::postgres::PgConnection;
use sqlx::{Connection as _, PgPool};
use uuid::Uuid;

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

/// Mapping from community UUID to host string, used by the poller to resolve
/// Prometheus label values.
#[derive(Debug)]
pub struct CommunityHost {
    /// The UUID of the community.
    pub id: Uuid,
    /// The canonical host string for this community (used as the Prometheus label value).
    pub host: String,
}

/// Fetch every community id → host mapping in one query, including archived,
/// deleting, and tombstoned communities.
///
/// Per-community in-memory gauges and storage attribution label series with
/// this map, so it stays unfiltered: a community keeps its label through every
/// lifecycle state.
pub async fn community_hosts(pool: &PgPool) -> Result<Vec<CommunityHost>> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let rows = sqlx::query_as::<_, (Uuid, String)>("SELECT id, host FROM communities")
        .fetch_all(&mut *connection)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id, host)| CommunityHost { id, host })
        .collect())
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

    /// Return all community id → host mappings, for usage-metrics labels.
    #[datastore_span(name = "usage_community_hosts", system = "postgresql")]
    pub async fn usage_community_hosts(&self) -> Result<Vec<CommunityHost>> {
        community_hosts(&self.pool).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::test_support::ScratchDb;
    use buzz_core::CommunityId;
    use sqlx::PgPool;

    async fn get_pool() -> PgPool {
        PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB")
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_usage_metrics_lock_has_single_owner_and_releases_on_drop() {
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

    async fn make_community(pool: &PgPool) -> (Uuid, CommunityId, String) {
        let id = uuid::Uuid::new_v4();
        let host = format!("usage-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(&host)
            .execute(pool)
            .await
            .expect("insert test community");
        (id, CommunityId::from_uuid(id), host)
    }

    /// community_hosts returns id → host mapping.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_community_hosts_returns_mapping() {
        let pool = get_pool().await;
        let (id, _, host) = make_community(&pool).await;

        let hosts = community_hosts(&pool).await.expect("community_hosts");
        let found = hosts.iter().find(|h| h.id == id);
        assert!(found.is_some(), "inserted community not found");
        assert_eq!(found.unwrap().host, host);
    }

    /// The usage-poller host map is unfiltered: per-community labels and
    /// storage attribution keep covering archived, deleting, and tombstoned
    /// communities.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_usage_community_hosts_include_every_lifecycle_state() {
        let pool = get_pool().await;
        let (active, _, _) = make_community(&pool).await;
        let (archived, _, _) = make_community(&pool).await;
        let (quiescing, _, _) = make_community(&pool).await;
        let (fenced, _, _) = make_community(&pool).await;
        let (tombstone, _, _) = make_community(&pool).await;

        sqlx::query("UPDATE communities SET archived_at = now() WHERE id = $1")
            .bind(archived)
            .execute(&pool)
            .await
            .expect("archive fixture");
        crate::test_support::set_deletion_state(&pool, quiescing, "quiescing").await;
        crate::test_support::set_deletion_state(&pool, fenced, "fenced").await;
        crate::test_support::set_deletion_state(&pool, tombstone, "tombstone").await;

        let metrics_ids: std::collections::HashSet<Uuid> = Db::from_pool(pool.clone())
            .usage_community_hosts()
            .await
            .expect("usage-metrics hosts")
            .iter()
            .map(|h| h.id)
            .collect();
        for (label, id) in [
            ("active", active),
            ("archived", archived),
            ("quiescing", quiescing),
            ("fenced", fenced),
            ("tombstone", tombstone),
        ] {
            assert!(
                metrics_ids.contains(&id),
                "usage-metrics map must include the {label} community"
            );
        }
    }
}
