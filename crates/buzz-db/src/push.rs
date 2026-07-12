//! Community-scoped NIP-PL lease and durable wake-outbox persistence.
//!
//! Every operation requires a server-resolved [`CommunityId`]. Client-provided
//! origins never select rows in this module.

use buzz_core::CommunityId;
use serde_json::Value;
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::error::Result;

/// Common signed-event ordering fields for a lease replacement.
#[derive(Debug, Clone, Copy)]
pub struct LeaseVersion<'a> {
    /// Signed kind:30350 event id (32 bytes).
    pub source_event_id: &'a [u8],
    /// Signed event `created_at`, in Unix seconds.
    pub source_created_at: i64,
    /// Strictly increasing installation generation.
    pub generation: i64,
    /// Public NIP-40 expiration, in Unix seconds.
    pub expires_at: i64,
}

/// Effective fields for an active APNs lease.
#[derive(Debug, Clone, Copy)]
pub struct ActiveLease<'a> {
    /// Application profile selected from the executor descriptor.
    pub app_profile: &'a str,
    /// SHA-256 of the platform endpoint.
    pub endpoint_hash: &'a [u8],
    /// Opaque endpoint grant issued by the stateless gateway.
    pub endpoint_grant: &'a str,
    /// Highest delivery class this lease permits.
    pub max_class: &'a str,
    /// Validated subscription array stored for matching.
    pub subscriptions: &'a Value,
}

/// Result of applying a lease replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceLeaseOutcome {
    /// The replacement became the effective lease state.
    Accepted,
    /// The signed event did not win NIP-01 addressable-event ordering.
    StaleEvent,
    /// The generation did not exceed the persisted watermark.
    StaleGeneration,
}

/// Result of an idempotent outbox enqueue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueWakeOutcome {
    /// A new durable job was inserted.
    Enqueued(Uuid),
    /// The endpoint/event dedup key already had a durable job.
    Duplicate(Uuid),
    /// No current active, unexpired lease matched the supplied generation.
    InactiveLease,
}

/// Durable wake fields not copied from the effective lease.
#[derive(Debug, Clone, Copy)]
pub struct NewWake<'a> {
    /// Generation observed by the matcher.
    pub lease_generation: i64,
    /// Accepted event id that caused the wake (32 bytes).
    pub event_id: &'a [u8],
    /// Effective wake class.
    pub class: &'a str,
    /// Closed, privacy-safe wake object.
    pub wake: &'a Value,
    /// Delivery deadline, in Unix seconds.
    pub expires_at: i64,
}

/// Create or rotate an active lease if both ordering gates win atomically.
#[allow(clippy::too_many_arguments)]
pub async fn replace_active_lease(
    pool: &PgPool,
    community: CommunityId,
    author: &[u8],
    installation_id: &str,
    version: LeaseVersion<'_>,
    active: ActiveLease<'_>,
) -> Result<ReplaceLeaseOutcome> {
    replace_lease(
        pool,
        community,
        author,
        installation_id,
        version,
        Some(active),
    )
    .await
}

/// Revoke one installation with a higher-generation inactive replacement.
pub async fn revoke_lease(
    pool: &PgPool,
    community: CommunityId,
    author: &[u8],
    installation_id: &str,
    version: LeaseVersion<'_>,
) -> Result<ReplaceLeaseOutcome> {
    replace_lease(pool, community, author, installation_id, version, None).await
}

async fn replace_lease(
    pool: &PgPool,
    community: CommunityId,
    author: &[u8],
    installation_id: &str,
    version: LeaseVersion<'_>,
    active: Option<ActiveLease<'_>>,
) -> Result<ReplaceLeaseOutcome> {
    let (is_active, app_profile, endpoint_hash, endpoint_grant, max_class, subscriptions) =
        match active {
            Some(active) => (
                true,
                Some(active.app_profile),
                Some(active.endpoint_hash),
                Some(active.endpoint_grant),
                Some(active.max_class),
                Some(active.subscriptions),
            ),
            None => (false, None, None, None, None, None),
        };

    // The conflict predicate is the acceptance state machine. Keeping both
    // orderings in the upsert closes the missing-row race: concurrent initial
    // publications cannot bypass a preceding SELECT/row lock.
    let accepted = sqlx::query(
        r#"
        INSERT INTO push_leases (
            community_id, author, installation_id, source_event_id,
            source_created_at, generation, active, app_profile, endpoint_hash,
            endpoint_grant, max_class, subscriptions, expires_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        ON CONFLICT (community_id, author, installation_id) DO UPDATE SET
            source_event_id = EXCLUDED.source_event_id,
            source_created_at = EXCLUDED.source_created_at,
            generation = EXCLUDED.generation,
            active = EXCLUDED.active,
            app_profile = EXCLUDED.app_profile,
            endpoint_hash = EXCLUDED.endpoint_hash,
            endpoint_grant = EXCLUDED.endpoint_grant,
            max_class = EXCLUDED.max_class,
            subscriptions = EXCLUDED.subscriptions,
            expires_at = EXCLUDED.expires_at,
            updated_at = now()
        WHERE (
                EXCLUDED.source_created_at > push_leases.source_created_at
                OR (
                    EXCLUDED.source_created_at = push_leases.source_created_at
                    AND EXCLUDED.source_event_id < push_leases.source_event_id
                )
              )
          AND EXCLUDED.generation > push_leases.generation
        RETURNING generation
        "#,
    )
    .bind(community.as_uuid())
    .bind(author)
    .bind(installation_id)
    .bind(version.source_event_id)
    .bind(version.source_created_at)
    .bind(version.generation)
    .bind(is_active)
    .bind(app_profile)
    .bind(endpoint_hash)
    .bind(endpoint_grant)
    .bind(max_class)
    .bind(subscriptions)
    .bind(version.expires_at)
    .fetch_optional(pool)
    .await?;

    if accepted.is_some() {
        return Ok(ReplaceLeaseOutcome::Accepted);
    }

    let current = sqlx::query(
        "SELECT source_event_id, source_created_at, generation \
         FROM push_leases \
         WHERE community_id = $1 AND author = $2 AND installation_id = $3",
    )
    .bind(community.as_uuid())
    .bind(author)
    .bind(installation_id)
    .fetch_one(pool)
    .await?;
    let current_created_at: i64 = current.try_get("source_created_at")?;
    let current_event_id: Vec<u8> = current.try_get("source_event_id")?;
    let wins_event_order = version.source_created_at > current_created_at
        || (version.source_created_at == current_created_at
            && version.source_event_id < current_event_id.as_slice());
    if !wins_event_order {
        Ok(ReplaceLeaseOutcome::StaleEvent)
    } else {
        Ok(ReplaceLeaseOutcome::StaleGeneration)
    }
}

/// Atomically enqueue at most one job per community, endpoint, and event.
///
/// Endpoint identity and the endpoint grant are copied from the current lease;
/// callers cannot redirect a wake by supplying either value. A generation that
/// lost a replacement race is ineligible in the same statement that inserts.
pub async fn enqueue_wake(
    pool: &PgPool,
    community: CommunityId,
    author: &[u8],
    installation_id: &str,
    wake: NewWake<'_>,
) -> Result<EnqueueWakeOutcome> {
    let mut tx = pool.begin().await?;
    // Serialize against lease replacement. If enqueue wins the lock, a later
    // replacement can leave this durable job queued, but worker revalidation
    // will suppress it; if replacement wins, the generation predicate fails.
    let endpoint_hash = sqlx::query(
        r#"
        SELECT endpoint_hash
        FROM push_leases
        WHERE community_id = $1
          AND author = $2
          AND installation_id = $3
          AND generation = $4
          AND active
          AND expires_at > EXTRACT(EPOCH FROM now())::bigint
        FOR UPDATE
        "#,
    )
    .bind(community.as_uuid())
    .bind(author)
    .bind(installation_id)
    .bind(wake.lease_generation)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(endpoint_hash) = endpoint_hash else {
        return Ok(EnqueueWakeOutcome::InactiveLease);
    };
    let endpoint_hash: Vec<u8> = endpoint_hash.try_get("endpoint_hash")?;

    let inserted = sqlx::query(
        r#"
        INSERT INTO push_wake_outbox (
            community_id, author, installation_id, lease_generation,
            endpoint_hash, event_id, class, wake, expires_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (community_id, endpoint_hash, event_id) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(community.as_uuid())
    .bind(author)
    .bind(installation_id)
    .bind(wake.lease_generation)
    .bind(&endpoint_hash)
    .bind(wake.event_id)
    .bind(wake.class)
    .bind(wake.wake)
    .bind(wake.expires_at)
    .fetch_optional(&mut *tx)
    .await?;

    let outcome = if let Some(row) = inserted {
        EnqueueWakeOutcome::Enqueued(row.try_get("id")?)
    } else {
        // This is a separate statement so READ COMMITTED observes a competing
        // transaction whose unique-key insert completed while ours waited.
        let row = sqlx::query(
            "SELECT id FROM push_wake_outbox \
             WHERE community_id = $1 AND endpoint_hash = $2 AND event_id = $3",
        )
        .bind(community.as_uuid())
        .bind(&endpoint_hash)
        .bind(wake.event_id)
        .fetch_one(&mut *tx)
        .await?;
        EnqueueWakeOutcome::Duplicate(row.try_get("id")?)
    };
    tx.commit().await?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration;
    use std::sync::Arc;
    use tokio::sync::Barrier;

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".into());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test DB");
        migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        pool
    }

    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("push-test-{}.example", id.simple()))
            .execute(pool)
            .await
            .expect("insert community");
        CommunityId::from_uuid(id)
    }

    fn version(event: u8, created_at: i64, generation: i64) -> LeaseVersion<'static> {
        LeaseVersion {
            source_event_id: Box::leak(Box::new([event; 32])),
            source_created_at: created_at,
            generation,
            expires_at: i64::MAX / 2,
        }
    }

    async fn activate(
        pool: &PgPool,
        community: CommunityId,
        author: &[u8],
        installation: &str,
        endpoint: &[u8],
        generation: i64,
    ) {
        assert_eq!(
            replace_active_lease(
                pool,
                community,
                author,
                installation,
                version(generation as u8, generation * 10, generation),
                ActiveLease {
                    app_profile: "ios-production",
                    endpoint_hash: endpoint,
                    endpoint_grant: "opaque-grant",
                    max_class: "default",
                    subscriptions: &serde_json::json!([]),
                },
            )
            .await
            .expect("activate lease"),
            ReplaceLeaseOutcome::Accepted
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn replacement_and_revoke_are_community_scoped_and_dual_ordered() {
        let pool = setup_pool().await;
        let a = make_community(&pool).await;
        let b = make_community(&pool).await;
        let author = [7; 32];
        let endpoint = [8; 32];
        activate(&pool, a, &author, "install", &endpoint, 1).await;
        activate(&pool, b, &author, "install", &endpoint, 1).await;

        assert_eq!(
            revoke_lease(&pool, a, &author, "install", version(2, 20, 2))
                .await
                .expect("revoke A"),
            ReplaceLeaseOutcome::Accepted
        );
        assert_eq!(
            replace_active_lease(
                &pool,
                a,
                &author,
                "install",
                version(3, 15, 99),
                ActiveLease {
                    app_profile: "ios-production",
                    endpoint_hash: &endpoint,
                    endpoint_grant: "grant",
                    max_class: "default",
                    subscriptions: &serde_json::json!([]),
                },
            )
            .await
            .expect("old event loses"),
            ReplaceLeaseOutcome::StaleEvent
        );

        let active: bool = sqlx::query_scalar(
            "SELECT active FROM push_leases \
             WHERE community_id = $1 AND author = $2 AND installation_id = $3",
        )
        .bind(b.as_uuid())
        .bind(author)
        .bind("install")
        .fetch_one(&pool)
        .await
        .expect("read B");
        assert!(active, "revoking A must not touch B");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_enqueue_is_atomic_and_community_scoped() {
        let pool = setup_pool().await;
        let a = make_community(&pool).await;
        let b = make_community(&pool).await;
        let author = [9; 32];
        let endpoint = [10; 32];
        let event = [11; 32];
        activate(&pool, a, &author, "install", &endpoint, 1).await;
        activate(&pool, b, &author, "install", &endpoint, 1).await;

        let barrier = Arc::new(Barrier::new(8));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let pool = pool.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                enqueue_wake(
                    &pool,
                    a,
                    &author,
                    "install",
                    NewWake {
                        lease_generation: 1,
                        event_id: &event,
                        class: "default",
                        wake: &serde_json::json!({"v": 1}),
                        expires_at: i64::MAX / 2,
                    },
                )
                .await
                .expect("enqueue")
            }));
        }
        let mut ids = Vec::new();
        for task in tasks {
            ids.push(match task.await.expect("join") {
                EnqueueWakeOutcome::Enqueued(id) | EnqueueWakeOutcome::Duplicate(id) => id,
                EnqueueWakeOutcome::InactiveLease => panic!("lease unexpectedly inactive"),
            });
        }
        assert!(ids.iter().all(|id| *id == ids[0]));
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM push_wake_outbox \
             WHERE community_id = $1 AND endpoint_hash = $2 AND event_id = $3",
        )
        .bind(a.as_uuid())
        .bind(endpoint)
        .bind(event)
        .fetch_one(&pool)
        .await
        .expect("count A jobs");
        assert_eq!(count, 1);

        assert!(matches!(
            enqueue_wake(
                &pool,
                b,
                &author,
                "install",
                NewWake {
                    lease_generation: 1,
                    event_id: &event,
                    class: "default",
                    wake: &serde_json::json!({"v": 1}),
                    expires_at: i64::MAX / 2,
                },
            )
            .await
            .expect("enqueue B"),
            EnqueueWakeOutcome::Enqueued(_)
        ));
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM push_wake_outbox \
             WHERE endpoint_hash = $1 AND event_id = $2",
        )
        .bind(endpoint)
        .bind(event)
        .fetch_one(&pool)
        .await
        .expect("count all jobs");
        assert_eq!(total, 2, "same dedup key is independent per community");
    }
}
