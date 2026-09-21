//! Deployment-global operator-listener mention registrations and webhook queues.

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, TimeDelta, Utc};
use sqlx::{PgPool, QueryBuilder, Row as _};
use uuid::Uuid;

use crate::error::Result;
use crate::{observability, CommunityId, Db};

/// Maximum attempts for a matcher claim before it is discarded.
pub const MAX_MATCH_ATTEMPTS: i32 = 8;
/// Maximum webhook attempts before a delivery becomes terminally failed.
pub const MAX_DELIVERY_ATTEMPTS: i32 = 9;
/// Maximum time a queued match or delivery remains useful when no worker can
/// route it.
pub const QUEUE_RETENTION: TimeDelta = TimeDelta::minutes(15);

/// A claimed operator-listener webhook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedDelivery {
    /// Durable delivery identifier and claim fence key.
    pub id: Uuid,
    /// Claim fencing token.
    pub claim_id: Uuid,
    /// Configured operator-listener identity.
    pub listener_pubkey: Vec<u8>,
    /// Registered target identity mentioned by the event.
    pub target_pubkey: Vec<u8>,
    /// Community containing the event.
    pub community: CommunityId,
    /// Community host used to address a tenant-scoped query later.
    pub community_host: String,
    /// Mentioning event id.
    pub event_id: Vec<u8>,
    /// Mentioning event kind.
    pub event_kind: i32,
    /// Author-controlled event timestamp.
    pub event_created_at: DateTime<Utc>,
    /// Attempt number, starting at one.
    pub attempt: i32,
}

/// Register target pubkeys for one deployment-global listener.
pub async fn register_pubkeys(
    pool: &PgPool,
    listener_pubkey: &[u8],
    target_pubkeys: &[Vec<u8>],
) -> Result<u64> {
    if target_pubkeys.is_empty() {
        return Ok(0);
    }
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let mut query = QueryBuilder::<sqlx::Postgres>::new(
        "INSERT INTO operator_listener_pubkeys (listener_pubkey, target_pubkey) ",
    );
    query.push_values(target_pubkeys, |mut bind, target| {
        bind.push_bind(listener_pubkey).push_bind(target.as_slice());
    });
    query.push(" ON CONFLICT DO NOTHING");
    Ok(query
        .build()
        .execute(&mut *connection)
        .await?
        .rows_affected())
}

/// Remove target pubkeys for one deployment-global listener.
///
/// Missing registrations are ignored so callers can safely repeat a removal.
pub async fn remove_pubkeys(
    pool: &PgPool,
    listener_pubkey: &[u8],
    target_pubkeys: &[Vec<u8>],
) -> Result<u64> {
    if target_pubkeys.is_empty() {
        return Ok(0);
    }
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let mut query = QueryBuilder::<sqlx::Postgres>::new(
        "DELETE FROM operator_listener_pubkeys WHERE listener_pubkey = ",
    );
    query
        .push_bind(listener_pubkey)
        .push(" AND target_pubkey IN (");
    let mut separated = query.separated(", ");
    for target in target_pubkeys {
        separated.push_bind(target.as_slice());
    }
    separated.push_unseparated(")");
    Ok(query
        .build()
        .execute(&mut *connection)
        .await?
        .rows_affected())
}

/// Exclusively claim due operator-listener match jobs across all communities.
pub async fn claim_match_jobs(
    pool: &PgPool,
    limit: i64,
    lease_until: DateTime<Utc>,
) -> Result<Option<Uuid>> {
    let claim_id = Uuid::new_v4();
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let rows = sqlx::query(
        "WITH candidates AS ( \
             SELECT community_id, event_id \
             FROM operator_listener_match_queue \
             WHERE attempts < $3 \
               AND next_attempt_at <= now() \
               AND (state = 'pending' OR (state = 'matching' AND lease_until < now())) \
             ORDER BY next_attempt_at, created_at \
             FOR UPDATE SKIP LOCKED \
             LIMIT $4 \
         ) \
         UPDATE operator_listener_match_queue q \
         SET state = 'matching', claim_id = $1, lease_until = $2, attempts = q.attempts + 1 \
         FROM candidates c \
         WHERE q.community_id = c.community_id AND q.event_id = c.event_id \
         RETURNING q.event_id",
    )
    .bind(claim_id)
    .bind(lease_until)
    .bind(MAX_MATCH_ATTEMPTS)
    .bind(limit)
    .fetch_all(&mut *connection)
    .await?;
    Ok((!rows.is_empty()).then_some(claim_id))
}

/// Expand one claimed matcher batch into webhook rows and finish the claims.
/// The transaction makes matching and queue removal atomic. Delivery claiming
/// rechecks that the source event is still live before sending.
pub async fn match_claimed_jobs(pool: &PgPool, claim_id: Uuid) -> Result<u64> {
    let connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;
    let inserted = sqlx::query(
        "INSERT INTO operator_listener_outbox ( \
             listener_pubkey, target_pubkey, community_id, event_id, event_kind, event_created_at \
         ) \
         SELECT r.listener_pubkey, r.target_pubkey, q.community_id, q.event_id, m.event_kind, m.event_created_at \
         FROM operator_listener_match_queue q \
         JOIN event_mentions m \
           ON m.community_id = q.community_id AND m.event_id = q.event_id \
         JOIN operator_listener_pubkeys r \
           ON r.target_pubkey = decode(m.pubkey_hex, 'hex') \
         WHERE q.claim_id = $1 AND q.state = 'matching' \
         ON CONFLICT DO NOTHING",
    )
    .bind(claim_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    sqlx::query(
        "DELETE FROM operator_listener_match_queue \
         WHERE claim_id = $1 AND state = 'matching'",
    )
    .bind(claim_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(inserted)
}

/// Release one matcher batch after a transient database failure.
pub async fn retry_match_jobs(pool: &PgPool, claim_id: Uuid, next: DateTime<Utc>) -> Result<u64> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    Ok(sqlx::query(
        "UPDATE operator_listener_match_queue \
         SET state = 'pending', claim_id = NULL, lease_until = NULL, next_attempt_at = $2 \
         WHERE claim_id = $1 AND state = 'matching'",
    )
    .bind(claim_id)
    .bind(next)
    .execute(&mut *connection)
    .await?
    .rows_affected())
}

/// Delete matcher rows that have exceeded the queue retention window.
pub async fn reap_exhausted_match_jobs(pool: &PgPool) -> Result<u64> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let stale_before = Utc::now() - QUEUE_RETENTION;
    Ok(
        sqlx::query("DELETE FROM operator_listener_match_queue WHERE created_at < $1")
            .bind(stale_before)
            .execute(&mut *connection)
            .await?
            .rows_affected(),
    )
}

/// Release a delivery that cannot be routed by this pod without consuming a
/// delivery attempt. Another pod may have the matching configuration.
pub async fn release_unroutable_delivery(
    pool: &PgPool,
    id: Uuid,
    claim_id: Uuid,
    next: DateTime<Utc>,
) -> Result<bool> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    Ok(sqlx::query(
        "UPDATE operator_listener_outbox \
         SET state = 'pending', claim_id = NULL, lease_until = NULL, \
             next_attempt_at = $3, attempts = GREATEST(attempts - 1, 0) \
         WHERE id = $1 AND claim_id = $2 AND state = 'sending'",
    )
    .bind(id)
    .bind(claim_id)
    .bind(next)
    .execute(&mut *connection)
    .await?
    .rows_affected()
        == 1)
}

/// Claim due webhook deliveries and recover claims whose visibility timeout expired.
pub async fn claim_deliveries(
    pool: &PgPool,
    limit: i64,
    lease_until: DateTime<Utc>,
) -> Result<Vec<ClaimedDelivery>> {
    let claim_id = Uuid::new_v4();
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let rows = sqlx::query(
        "WITH candidates AS ( \
             SELECT o.id, o.community_id, o.event_id \
             FROM operator_listener_outbox o \
             WHERE o.attempts < $2 \
               AND o.next_attempt_at <= now() \
               AND (o.state = 'pending' OR (o.state = 'sending' AND o.lease_until < now())) \
             ORDER BY o.next_attempt_at, o.created_at, o.id \
             FOR UPDATE OF o SKIP LOCKED \
             LIMIT $3 \
         ) \
         UPDATE operator_listener_outbox o \
         SET state = 'sending', claim_id = $1, lease_until = $4, attempts = o.attempts + 1 \
         FROM candidates c \
         JOIN communities community ON community.id = c.community_id \
         WHERE o.id = c.id \
         RETURNING o.id, o.claim_id, o.listener_pubkey, o.target_pubkey, o.community_id, \
                   community.host, o.event_id, o.event_kind, o.event_created_at, o.attempts",
    )
    .bind(claim_id)
    .bind(MAX_DELIVERY_ATTEMPTS)
    .bind(limit)
    .bind(lease_until)
    .fetch_all(&mut *connection)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(ClaimedDelivery {
                id: row.try_get("id")?,
                claim_id: row.try_get("claim_id")?,
                listener_pubkey: row.try_get("listener_pubkey")?,
                target_pubkey: row.try_get("target_pubkey")?,
                community: CommunityId::from_uuid(row.try_get("community_id")?),
                community_host: row.try_get("host")?,
                event_id: row.try_get("event_id")?,
                event_kind: row.try_get("event_kind")?,
                event_created_at: row.try_get("event_created_at")?,
                attempt: row.try_get("attempts")?,
            })
        })
        .collect()
}

/// Delete one fenced delivery after successful delivery.
pub async fn complete_delivery(pool: &PgPool, id: Uuid, claim_id: Uuid) -> Result<bool> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    Ok(sqlx::query(
        "DELETE FROM operator_listener_outbox \
         WHERE id = $1 AND claim_id = $2 AND state = 'sending'",
    )
    .bind(id)
    .bind(claim_id)
    .execute(&mut *connection)
    .await?
    .rows_affected()
        == 1)
}

/// Retry one fenced delivery after a transient failure.
pub async fn retry_delivery(
    pool: &PgPool,
    id: Uuid,
    claim_id: Uuid,
    next: DateTime<Utc>,
) -> Result<bool> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    Ok(sqlx::query(
        "UPDATE operator_listener_outbox \
         SET state = 'pending', claim_id = NULL, lease_until = NULL, next_attempt_at = $3 \
         WHERE id = $1 AND claim_id = $2 AND state = 'sending' AND attempts < $4",
    )
    .bind(id)
    .bind(claim_id)
    .bind(next)
    .bind(MAX_DELIVERY_ATTEMPTS)
    .execute(&mut *connection)
    .await?
    .rows_affected()
        == 1)
}

/// Delete one fenced delivery after all delivery attempts fail.
pub async fn fail_delivery(pool: &PgPool, id: Uuid, claim_id: Uuid) -> Result<bool> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    Ok(sqlx::query(
        "DELETE FROM operator_listener_outbox \
         WHERE id = $1 AND claim_id = $2 AND state = 'sending'",
    )
    .bind(id)
    .bind(claim_id)
    .execute(&mut *connection)
    .await?
    .rows_affected()
        == 1)
}

/// Delete delivery rows that have exceeded the queue retention window.
pub async fn reap_deliveries(pool: &PgPool) -> Result<u64> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let stale_before = Utc::now() - QUEUE_RETENTION;
    Ok(
        sqlx::query("DELETE FROM operator_listener_outbox WHERE created_at < $1")
            .bind(stale_before)
            .execute(&mut *connection)
            .await?
            .rows_affected(),
    )
}

impl Db {
    /// Register target pubkeys for one configured operator listener.
    #[datastore_span(name = "register_operator_listener_pubkeys", system = "postgresql")]
    pub async fn register_operator_listener_pubkeys(
        &self,
        listener_pubkey: &[u8],
        target_pubkeys: &[Vec<u8>],
    ) -> Result<u64> {
        register_pubkeys(&self.pool, listener_pubkey, target_pubkeys).await
    }

    /// Remove target pubkeys for one configured operator listener.
    ///
    /// Missing registrations are ignored, making this operation idempotent.
    #[datastore_span(name = "remove_operator_listener_pubkeys", system = "postgresql")]
    pub async fn remove_operator_listener_pubkeys(
        &self,
        listener_pubkey: &[u8],
        target_pubkeys: &[Vec<u8>],
    ) -> Result<u64> {
        remove_pubkeys(&self.pool, listener_pubkey, target_pubkeys).await
    }

    /// Claim a batch of deployment-global operator-listener matcher jobs.
    #[datastore_span(name = "claim_operator_listener_match_jobs", system = "postgresql")]
    pub async fn claim_operator_listener_match_jobs(
        &self,
        limit: i64,
        lease_until: DateTime<Utc>,
    ) -> Result<Option<Uuid>> {
        claim_match_jobs(&self.pool, limit, lease_until).await
    }

    /// Expand and complete one claimed operator-listener matcher batch.
    #[datastore_span(name = "match_operator_listener_jobs", system = "postgresql")]
    pub async fn match_operator_listener_jobs(&self, claim_id: Uuid) -> Result<u64> {
        match_claimed_jobs(&self.pool, claim_id).await
    }

    /// Retry one failed operator-listener matcher batch.
    #[datastore_span(name = "retry_operator_listener_match_jobs", system = "postgresql")]
    pub async fn retry_operator_listener_match_jobs(
        &self,
        claim_id: Uuid,
        next: DateTime<Utc>,
    ) -> Result<u64> {
        retry_match_jobs(&self.pool, claim_id, next).await
    }

    /// Delete exhausted operator-listener matcher jobs.
    #[datastore_span(name = "reap_operator_listener_match_jobs", system = "postgresql")]
    pub async fn reap_operator_listener_match_jobs(&self) -> Result<u64> {
        reap_exhausted_match_jobs(&self.pool).await
    }

    /// Claim due operator-listener webhook deliveries.
    #[datastore_span(name = "claim_operator_listener_deliveries", system = "postgresql")]
    pub async fn claim_operator_listener_deliveries(
        &self,
        limit: i64,
        lease_until: DateTime<Utc>,
    ) -> Result<Vec<ClaimedDelivery>> {
        claim_deliveries(&self.pool, limit, lease_until).await
    }

    /// Mark one operator-listener delivery successful.
    #[datastore_span(name = "complete_operator_listener_delivery", system = "postgresql")]
    pub async fn complete_operator_listener_delivery(
        &self,
        id: Uuid,
        claim_id: Uuid,
    ) -> Result<bool> {
        complete_delivery(&self.pool, id, claim_id).await
    }

    /// Retry one operator-listener delivery.
    #[datastore_span(name = "retry_operator_listener_delivery", system = "postgresql")]
    pub async fn retry_operator_listener_delivery(
        &self,
        id: Uuid,
        claim_id: Uuid,
        next: DateTime<Utc>,
    ) -> Result<bool> {
        retry_delivery(&self.pool, id, claim_id, next).await
    }

    /// Release a delivery claim that this pod cannot route.
    #[datastore_span(name = "release_operator_listener_delivery", system = "postgresql")]
    pub async fn release_operator_listener_delivery(
        &self,
        id: Uuid,
        claim_id: Uuid,
        next: DateTime<Utc>,
    ) -> Result<bool> {
        release_unroutable_delivery(&self.pool, id, claim_id, next).await
    }

    /// Delete one operator-listener delivery after terminal failure.
    #[datastore_span(name = "fail_operator_listener_delivery", system = "postgresql")]
    pub async fn fail_operator_listener_delivery(&self, id: Uuid, claim_id: Uuid) -> Result<bool> {
        fail_delivery(&self.pool, id, claim_id).await
    }

    /// Reap missing-source and exhausted operator-listener deliveries.
    #[datastore_span(name = "reap_operator_listener_deliveries", system = "postgresql")]
    pub async fn reap_operator_listener_deliveries(&self) -> Result<u64> {
        reap_deliveries(&self.pool).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::migration;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    async fn setup_pool() -> PgPool {
        let database_url = crate::test_support::database_url();
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test DB");
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            migration::run_migrations(&pool)
                .await
                .expect("run migrations");
        }
        pool
    }

    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("operator-listener-test-{}.example", id.simple()))
            .execute(pool)
            .await
            .expect("insert community");
        CommunityId::from_uuid(id)
    }

    struct MentionFixture {
        community: CommunityId,
        target_pubkey: Vec<u8>,
    }

    async fn insert_mention_event(
        pool: &PgPool,
        community: CommunityId,
        target_pubkey: &nostr::PublicKey,
        kind: u32,
    ) -> (nostr::Event, DateTime<Utc>) {
        let event = EventBuilder::new(Kind::Custom(kind as u16), "operator-listener test")
            .tag(Tag::parse(["p", target_pubkey.to_hex().as_str()]).expect("p tag"))
            .sign_with_keys(&Keys::generate())
            .expect("sign event");
        let event_created_at = DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
            .expect("event timestamp");
        let (_, inserted) = crate::event::insert_event(pool, community, &event, None)
            .await
            .expect("insert event");
        assert!(inserted);
        crate::insert_mentions(pool, community, &event, None)
            .await
            .expect("index mention");
        (event, event_created_at)
    }

    async fn create_matched_delivery(pool: &PgPool) -> MentionFixture {
        let community = make_community(pool).await;
        let listener = Keys::generate();
        let target = Keys::generate();
        register_pubkeys(
            pool,
            listener.public_key().as_bytes(),
            &[target.public_key().to_bytes().to_vec()],
        )
        .await
        .expect("register target");

        insert_mention_event(pool, community, &target.public_key(), 9).await;
        let claim_id = claim_match_jobs(pool, 10, Utc::now() + TimeDelta::seconds(30))
            .await
            .expect("claim match jobs")
            .expect("mention job");
        assert_eq!(
            match_claimed_jobs(pool, claim_id).await.expect("match job"),
            1
        );

        MentionFixture {
            community,
            target_pubkey: target.public_key().to_bytes().to_vec(),
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn mention_trigger_matches_registered_target_across_communities() {
        let pool = setup_pool().await;
        let community_a = make_community(&pool).await;
        let community_b = make_community(&pool).await;
        let listener = Keys::generate();
        let target = Keys::generate();
        register_pubkeys(
            &pool,
            listener.public_key().as_bytes(),
            &[target.public_key().to_bytes().to_vec()],
        )
        .await
        .expect("register target");

        let (event_a, created_at_a) =
            insert_mention_event(&pool, community_a, &target.public_key(), 9).await;
        let (event_b, created_at_b) =
            insert_mention_event(&pool, community_b, &target.public_key(), 9).await;

        let claim_id = claim_match_jobs(&pool, 10, Utc::now() + TimeDelta::seconds(30))
            .await
            .expect("claim match jobs")
            .expect("mention job");
        assert_eq!(
            match_claimed_jobs(&pool, claim_id)
                .await
                .expect("match job"),
            2
        );

        let deliveries = claim_deliveries(&pool, 10, Utc::now() + TimeDelta::seconds(30))
            .await
            .expect("claim deliveries");
        assert_eq!(deliveries.len(), 2);
        assert!(deliveries.iter().any(|delivery| {
            delivery.community == community_a
                && delivery.event_id == event_a.id.as_bytes()
                && delivery.event_created_at == created_at_a
        }));
        assert!(deliveries.iter().any(|delivery| {
            delivery.community == community_b
                && delivery.event_id == event_b.id.as_bytes()
                && delivery.event_created_at == created_at_b
        }));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn delivery_retries_then_completes() {
        let pool = setup_pool().await;
        let fixture = create_matched_delivery(&pool).await;
        let first = claim_deliveries(&pool, 10, Utc::now() + TimeDelta::seconds(30))
            .await
            .expect("claim delivery");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].target_pubkey, fixture.target_pubkey);
        assert!(release_unroutable_delivery(
            &pool,
            first[0].id,
            first[0].claim_id,
            Utc::now() - TimeDelta::seconds(1),
        )
        .await
        .expect("release unroutable delivery"));

        let rerouted = claim_deliveries(&pool, 10, Utc::now() + TimeDelta::seconds(30))
            .await
            .expect("reclaim unroutable delivery");
        assert_eq!(rerouted.len(), 1);
        assert_eq!(rerouted[0].attempt, 1);
        assert!(retry_delivery(
            &pool,
            rerouted[0].id,
            rerouted[0].claim_id,
            Utc::now() - TimeDelta::seconds(1),
        )
        .await
        .expect("retry delivery"));

        let second = claim_deliveries(&pool, 10, Utc::now() + TimeDelta::seconds(30))
            .await
            .expect("reclaim delivery");
        assert_eq!(second.len(), 1);
        assert!(complete_delivery(&pool, second[0].id, second[0].claim_id)
            .await
            .expect("complete delivery"));
        assert!(
            claim_deliveries(&pool, 10, Utc::now() + TimeDelta::seconds(30))
                .await
                .expect("claim after successful delivery")
                .is_empty()
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn terminal_delivery_failure_is_removed() {
        let pool = setup_pool().await;
        let fixture = create_matched_delivery(&pool).await;
        let deliveries = claim_deliveries(&pool, 10, Utc::now() + TimeDelta::seconds(30))
            .await
            .expect("claim delivery");
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].community, fixture.community);
        assert!(
            fail_delivery(&pool, deliveries[0].id, deliveries[0].claim_id)
                .await
                .expect("delete terminal failure")
        );
        assert!(
            claim_deliveries(&pool, 10, Utc::now() + TimeDelta::seconds(30))
                .await
                .expect("claim after terminal failure")
                .is_empty()
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stale_match_and_delivery_rows_are_reaped() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let listener = Keys::generate();
        let target = Keys::generate();
        let event_id = [0_u8; 32];
        let stale_at = Utc::now() - QUEUE_RETENTION - TimeDelta::seconds(1);

        sqlx::query(
            "INSERT INTO operator_listener_match_queue (community_id, event_id, created_at) \
             VALUES ($1, $2, $3)",
        )
        .bind(community.as_uuid())
        .bind(event_id.as_slice())
        .bind(stale_at)
        .execute(&pool)
        .await
        .expect("insert stale match fixture");
        assert_eq!(
            reap_exhausted_match_jobs(&pool)
                .await
                .expect("reap stale match"),
            1
        );

        sqlx::query(
            "INSERT INTO operator_listener_outbox \
             (listener_pubkey, target_pubkey, community_id, event_id, event_kind, event_created_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(listener.public_key().to_bytes().as_slice())
        .bind(target.public_key().to_bytes().as_slice())
        .bind(community.as_uuid())
        .bind(event_id.as_slice())
        .bind(9_i32)
        .bind(stale_at)
        .bind(stale_at)
        .execute(&pool)
        .await
        .expect("insert stale delivery fixture");
        assert_eq!(
            reap_deliveries(&pool).await.expect("reap stale delivery"),
            1
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn non_message_mentions_are_not_enqueued() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let listener = Keys::generate();
        let target = Keys::generate();
        register_pubkeys(
            &pool,
            listener.public_key().as_bytes(),
            &[target.public_key().to_bytes().to_vec()],
        )
        .await
        .expect("register target");
        insert_mention_event(&pool, community, &target.public_key(), 1).await;

        assert!(
            claim_match_jobs(&pool, 10, Utc::now() + TimeDelta::seconds(30))
                .await
                .expect("claim non-message event")
                .is_none()
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn remove_pubkeys_is_idempotent() {
        let pool = setup_pool().await;
        let listener = Keys::generate();
        let target_bytes = Keys::generate().public_key().to_bytes().to_vec();
        let missing_bytes = Keys::generate().public_key().to_bytes().to_vec();
        register_pubkeys(
            &pool,
            listener.public_key().as_bytes(),
            std::slice::from_ref(&target_bytes),
        )
        .await
        .expect("register target");
        assert_eq!(
            remove_pubkeys(
                &pool,
                listener.public_key().as_bytes(),
                &[target_bytes.clone(), missing_bytes.clone()],
            )
            .await
            .expect("remove target pubkeys"),
            1
        );
        assert_eq!(
            remove_pubkeys(
                &pool,
                listener.public_key().as_bytes(),
                &[target_bytes, missing_bytes],
            )
            .await
            .expect("repeat removal"),
            0
        );
    }
}
