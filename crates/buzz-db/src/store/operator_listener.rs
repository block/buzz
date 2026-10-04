//! Deployment-global operator-listener mention registrations and delivery outbox.

use std::collections::HashSet;

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, TimeDelta, Utc};
use nostr::Event;
use sqlx::{PgPool, Postgres, QueryBuilder, Row as _, Transaction};
use uuid::Uuid;

use crate::error::Result;
use crate::{observability, CommunityId, Db};

/// Event kinds whose `p` tags represent user-visible message mentions.
pub const OPERATOR_LISTENER_MENTION_KINDS: [u32; 4] = [9, 40002, 45001, 45003];
/// Maximum notification attempts before a delivery becomes terminally failed.
pub const MAX_DELIVERY_ATTEMPTS: i32 = 9;
/// Maximum time a queued delivery remains useful when no worker can route it.
pub const OUTBOX_RETENTION: TimeDelta = TimeDelta::minutes(15);
/// Registration lifetime before the daily cleanup removes it.
pub const PUBKEY_RETENTION: TimeDelta = TimeDelta::days(30);

async fn acquire_maintenance_writer(
    pool: &PgPool,
) -> sqlx::Result<sqlx::pool::PoolConnection<Postgres>> {
    observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await
}

/// A claimed operator-listener notification.
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
    /// Community host used in the notification payload.
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

/// Return whether an event kind should generate operator-listener notifications.
#[must_use]
pub const fn is_listener_mention_kind(kind: u32) -> bool {
    let mut index = 0;
    while index < OPERATOR_LISTENER_MENTION_KINDS.len() {
        if OPERATOR_LISTENER_MENTION_KINDS[index] == kind {
            return true;
        }
        index += 1;
    }
    false
}

fn event_targets(event: &Event) -> Vec<Vec<u8>> {
    let mut targets = Vec::new();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("p") {
            continue;
        }
        let Some(value) = parts.get(1) else {
            continue;
        };
        let Ok(target) = hex::decode(value) else {
            continue;
        };
        if target.len() == 32 && !targets.iter().any(|known| known == &target) {
            targets.push(target);
        }
    }
    targets
}

/// Insert one outbox row for each registered listener matching the event's
/// `p` tags. The caller owns the transaction and must commit it with the event.
pub(crate) async fn enqueue_mentions_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event: &Event,
) -> Result<u64> {
    let kind = u32::from(event.kind.as_u16());
    if !is_listener_mention_kind(kind) {
        return Ok(0);
    }
    let targets = event_targets(event);
    if targets.is_empty() {
        return Ok(0);
    }
    let event_created_at = DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).ok_or(
        crate::error::DbError::InvalidTimestamp(event.created_at.as_secs() as i64),
    )?;

    let mut query = QueryBuilder::<Postgres>::new(
        "INSERT INTO operator_listener_outbox \
         (listener_pubkey, target_pubkey, community_id, event_id, event_kind, event_created_at) \
         SELECT r.listener_pubkey, r.target_pubkey, ",
    );
    query
        .push_bind(community_id.as_uuid())
        .push(", ")
        .push_bind(event.id.as_bytes().as_slice())
        .push(", ")
        .push_bind(kind as i32)
        .push(", ")
        .push_bind(event_created_at)
        .push(" FROM operator_listener_pubkeys r WHERE r.target_pubkey IN (");
    let mut separated = query.separated(", ");
    for target in &targets {
        separated.push_bind(target.as_slice());
    }
    separated.push_unseparated(") ON CONFLICT DO NOTHING");

    Ok(query.build().execute(&mut **tx).await?.rows_affected())
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
    let mut seen = HashSet::with_capacity(target_pubkeys.len());
    let unique_target_pubkeys: Vec<&[u8]> = target_pubkeys
        .iter()
        .filter_map(|target| seen.insert(target.as_slice()).then_some(target.as_slice()))
        .collect();
    let mut connection = acquire_maintenance_writer(pool).await?;
    let mut query = QueryBuilder::<Postgres>::new(
        "INSERT INTO operator_listener_pubkeys (listener_pubkey, target_pubkey) ",
    );
    query.push_values(unique_target_pubkeys, |mut bind, target| {
        bind.push_bind(listener_pubkey).push_bind(target);
    });
    query.push(
        " ON CONFLICT (listener_pubkey, target_pubkey) DO UPDATE SET created_at = EXCLUDED.created_at",
    );
    Ok(query
        .build()
        .execute(&mut *connection)
        .await?
        .rows_affected())
}

/// Remove target pubkeys for one deployment-global listener.
pub async fn remove_pubkeys(
    pool: &PgPool,
    listener_pubkey: &[u8],
    target_pubkeys: &[Vec<u8>],
) -> Result<u64> {
    if target_pubkeys.is_empty() {
        return Ok(0);
    }
    let mut connection = acquire_maintenance_writer(pool).await?;
    let mut query = QueryBuilder::<Postgres>::new(
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

/// Delete registrations older than the operator-listener retention period.
pub async fn delete_expired_pubkeys(pool: &PgPool) -> Result<u64> {
    let mut connection = acquire_maintenance_writer(pool).await?;
    let cutoff = Utc::now() - PUBKEY_RETENTION;
    Ok(
        sqlx::query("DELETE FROM operator_listener_pubkeys WHERE created_at < $1")
            .bind(cutoff)
            .execute(&mut *connection)
            .await?
            .rows_affected(),
    )
}

/// Claim due notification deliveries and recover claims whose visibility timeout expired.
pub async fn claim_deliveries(
    pool: &PgPool,
    limit: i64,
    lease_until: DateTime<Utc>,
) -> Result<Vec<ClaimedDelivery>> {
    let claim_id = Uuid::new_v4();
    let cutoff = Utc::now() - OUTBOX_RETENTION;
    let mut connection = acquire_maintenance_writer(pool).await?;
    let rows = sqlx::query(
        "WITH candidates AS ( \
             SELECT o.id, o.community_id \
             FROM operator_listener_outbox o \
             WHERE o.attempts < $2 \
               AND o.next_attempt_at <= now() \
               AND o.created_at >= $4 \
               AND (o.state = 'pending' OR (o.state = 'sending' AND o.lease_until < now())) \
             ORDER BY o.next_attempt_at, o.created_at, o.id \
             FOR UPDATE OF o SKIP LOCKED \
             LIMIT $3 \
         ) \
         UPDATE operator_listener_outbox o \
         SET state = 'sending', claim_id = $1, lease_until = $5, attempts = o.attempts + 1 \
         FROM candidates c \
         JOIN communities community ON community.id = c.community_id \
         WHERE o.id = c.id \
         RETURNING o.id, o.claim_id, o.listener_pubkey, o.target_pubkey, o.community_id, \
                   community.host, o.event_id, o.event_kind, o.event_created_at, o.attempts",
    )
    .bind(claim_id)
    .bind(MAX_DELIVERY_ATTEMPTS)
    .bind(limit)
    .bind(cutoff)
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

/// Release a delivery claim that this pod cannot route.
pub async fn release_unroutable_delivery(
    pool: &PgPool,
    id: Uuid,
    claim_id: Uuid,
    next: DateTime<Utc>,
) -> Result<bool> {
    let mut connection = acquire_maintenance_writer(pool).await?;
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

/// Mark one fenced delivery successful.
pub async fn complete_delivery(pool: &PgPool, id: Uuid, claim_id: Uuid) -> Result<bool> {
    delete_claimed_delivery(pool, id, claim_id).await
}

/// Retry one fenced delivery after a transient failure.
pub async fn retry_delivery(
    pool: &PgPool,
    id: Uuid,
    claim_id: Uuid,
    next: DateTime<Utc>,
) -> Result<bool> {
    let mut connection = acquire_maintenance_writer(pool).await?;
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
    delete_claimed_delivery(pool, id, claim_id).await
}

async fn delete_claimed_delivery(pool: &PgPool, id: Uuid, claim_id: Uuid) -> Result<bool> {
    let mut connection = acquire_maintenance_writer(pool).await?;
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

/// Delete delivery rows that have exceeded the outbox retention window.
pub async fn reap_deliveries(pool: &PgPool) -> Result<u64> {
    let mut connection = acquire_maintenance_writer(pool).await?;
    let cutoff = Utc::now() - OUTBOX_RETENTION;
    Ok(
        sqlx::query("DELETE FROM operator_listener_outbox WHERE created_at < $1")
            .bind(cutoff)
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
    #[datastore_span(name = "remove_operator_listener_pubkeys", system = "postgresql")]
    pub async fn remove_operator_listener_pubkeys(
        &self,
        listener_pubkey: &[u8],
        target_pubkeys: &[Vec<u8>],
    ) -> Result<u64> {
        remove_pubkeys(&self.pool, listener_pubkey, target_pubkeys).await
    }

    /// Claim due operator-listener deliveries.
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

    /// Reap stale deliveries.
    #[datastore_span(name = "reap_operator_listener_deliveries", system = "postgresql")]
    pub async fn reap_operator_listener_deliveries(&self) -> Result<u64> {
        reap_deliveries(&self.pool).await
    }

    /// Delete operator-listener registrations older than thirty days.
    #[datastore_span(
        name = "delete_expired_operator_listener_pubkeys",
        system = "postgresql"
    )]
    pub async fn delete_expired_operator_listener_pubkeys(&self) -> Result<u64> {
        delete_expired_pubkeys(&self.pool).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_message_kinds_are_listener_mentions() {
        for kind in OPERATOR_LISTENER_MENTION_KINDS {
            assert!(is_listener_mention_kind(kind));
        }
        assert!(!is_listener_mention_kind(1));
        assert!(!is_listener_mention_kind(40003));
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sqlx::PgPool;

    async fn setup_pool() -> PgPool {
        PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB")
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn event_insert_enqueues_registered_listener_in_same_transaction() {
        let pool = setup_pool().await;
        let community_id = Uuid::new_v4();
        let listener = Keys::generate();
        let target = Keys::generate();
        let target_pubkey = target.public_key().to_bytes().to_vec();

        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!(
                "operator-listener-test-{}.example",
                community_id.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert test community");
        register_pubkeys(
            &pool,
            listener.public_key().as_bytes(),
            std::slice::from_ref(&target_pubkey),
        )
        .await
        .expect("register listener target");

        let event = EventBuilder::new(Kind::Custom(9), "mention")
            .tag(Tag::public_key(target.public_key()))
            .sign_with_keys(&Keys::generate())
            .expect("sign test event");
        let (_, inserted) =
            crate::event::insert_event(&pool, CommunityId::from_uuid(community_id), &event, None)
                .await
                .expect("insert event");
        assert!(inserted);

        let outbox_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM operator_listener_outbox WHERE community_id = $1 AND event_id = $2",
        )
        .bind(community_id)
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("count listener deliveries");
        assert_eq!(outbox_count, 1);

        sqlx::query("DELETE FROM operator_listener_outbox WHERE community_id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete test outbox rows");
        sqlx::query("DELETE FROM operator_listener_pubkeys WHERE listener_pubkey = $1")
            .bind(listener.public_key().as_bytes().as_slice())
            .execute(&pool)
            .await
            .expect("delete test registration");
        sqlx::query("DELETE FROM event_mentions WHERE community_id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete test mentions");
        sqlx::query("DELETE FROM events WHERE community_id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete test event");
        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete test community");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn thread_metadata_insert_fans_out_once_to_matching_listeners() {
        let pool = setup_pool().await;
        let community_id = Uuid::new_v4();
        let first_listener = Keys::generate();
        let second_listener = Keys::generate();
        let unrelated_listener = Keys::generate();
        let target = Keys::generate();
        let unrelated_target = Keys::generate();
        let target_pubkey = target.public_key().to_bytes().to_vec();
        let unrelated_pubkey = unrelated_target.public_key().to_bytes().to_vec();

        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!(
                "operator-listener-thread-test-{}.example",
                community_id.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert test community");
        for listener in [&first_listener, &second_listener] {
            register_pubkeys(
                &pool,
                listener.public_key().as_bytes(),
                std::slice::from_ref(&target_pubkey),
            )
            .await
            .expect("register matching listener target");
        }
        register_pubkeys(
            &pool,
            unrelated_listener.public_key().as_bytes(),
            std::slice::from_ref(&unrelated_pubkey),
        )
        .await
        .expect("register unrelated listener target");

        let event = EventBuilder::new(Kind::Custom(9), "thread mention")
            .tags([
                Tag::public_key(target.public_key()),
                Tag::public_key(target.public_key()),
            ])
            .sign_with_keys(&Keys::generate())
            .expect("sign test event");
        let community = CommunityId::from_uuid(community_id);
        let (_, inserted) =
            crate::event::insert_event_with_thread_metadata(&pool, community, &event, None, None)
                .await
                .expect("insert event through thread-metadata path");
        assert!(inserted);

        let delivered_listeners: Vec<Vec<u8>> = sqlx::query_scalar(
            "SELECT listener_pubkey FROM operator_listener_outbox WHERE community_id = $1 AND event_id = $2 ORDER BY listener_pubkey",
        )
        .bind(community_id)
        .bind(event.id.as_bytes().as_slice())
        .fetch_all(&pool)
        .await
        .expect("read listener fanout");
        let mut expected_listeners = vec![
            first_listener.public_key().to_bytes().to_vec(),
            second_listener.public_key().to_bytes().to_vec(),
        ];
        expected_listeners.sort();
        assert_eq!(delivered_listeners, expected_listeners);

        let (_, duplicate_inserted) =
            crate::event::insert_event_with_thread_metadata(&pool, community, &event, None, None)
                .await
                .expect("retry duplicate event through thread-metadata path");
        assert!(!duplicate_inserted);
        let outbox_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM operator_listener_outbox WHERE community_id = $1 AND event_id = $2",
        )
        .bind(community_id)
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("count listener deliveries after duplicate event");
        assert_eq!(outbox_count, 2);

        sqlx::query("DELETE FROM operator_listener_outbox WHERE community_id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete test outbox rows");
        sqlx::query("DELETE FROM operator_listener_pubkeys WHERE listener_pubkey IN ($1, $2, $3)")
            .bind(first_listener.public_key().as_bytes().as_slice())
            .bind(second_listener.public_key().as_bytes().as_slice())
            .bind(unrelated_listener.public_key().as_bytes().as_slice())
            .execute(&pool)
            .await
            .expect("delete test registrations");
        sqlx::query("DELETE FROM events WHERE community_id = $1 AND id = $2")
            .bind(community_id)
            .bind(event.id.as_bytes().as_slice())
            .execute(&pool)
            .await
            .expect("delete test event");
        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete test community");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn event_and_listener_outbox_roll_back_together() {
        let pool = setup_pool().await;
        let community_id = Uuid::new_v4();
        let listener = Keys::generate();
        let target = Keys::generate();
        let target_pubkey = target.public_key().to_bytes().to_vec();

        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!(
                "operator-listener-rollback-test-{}.example",
                community_id.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert test community");
        register_pubkeys(
            &pool,
            listener.public_key().as_bytes(),
            std::slice::from_ref(&target_pubkey),
        )
        .await
        .expect("register listener target");
        let event = EventBuilder::new(Kind::Custom(9), "mention")
            .tag(Tag::public_key(target.public_key()))
            .sign_with_keys(&Keys::generate())
            .expect("sign test event");

        let mut tx = pool.begin().await.expect("begin event transaction");
        let (_, inserted) = crate::event::insert_event_in_transaction(
            &mut tx,
            CommunityId::from_uuid(community_id),
            &event,
            None,
        )
        .await
        .expect("insert event and enqueue mention");
        assert!(inserted);
        tx.rollback().await.expect("roll back event transaction");

        let event_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE community_id = $1 AND id = $2")
                .bind(community_id)
                .bind(event.id.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count rolled-back event");
        let outbox_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM operator_listener_outbox WHERE community_id = $1 AND event_id = $2",
        )
        .bind(community_id)
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("count rolled-back delivery");
        assert_eq!(event_count, 0);
        assert_eq!(outbox_count, 0);

        sqlx::query("DELETE FROM operator_listener_pubkeys WHERE listener_pubkey = $1")
            .bind(listener.public_key().as_bytes().as_slice())
            .execute(&pool)
            .await
            .expect("delete test registration");
        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete test community");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn repeated_registration_renews_created_at() {
        let pool = setup_pool().await;
        let listener = Keys::generate();
        let target = Keys::generate();
        let listener_pubkey = listener.public_key().as_bytes().to_vec();
        let target_pubkey = target.public_key().as_bytes().to_vec();

        register_pubkeys(
            &pool,
            &listener_pubkey,
            std::slice::from_ref(&target_pubkey),
        )
        .await
        .expect("register listener target");

        let expired_at = Utc::now() - PUBKEY_RETENTION - TimeDelta::hours(1);
        sqlx::query(
            "UPDATE operator_listener_pubkeys SET created_at = $3 \
             WHERE listener_pubkey = $1 AND target_pubkey = $2",
        )
        .bind(&listener_pubkey)
        .bind(&target_pubkey)
        .bind(expired_at)
        .execute(&pool)
        .await
        .expect("age test registration");

        register_pubkeys(
            &pool,
            &listener_pubkey,
            std::slice::from_ref(&target_pubkey),
        )
        .await
        .expect("renew listener target");

        let renewed_at: DateTime<Utc> = sqlx::query_scalar(
            "SELECT created_at FROM operator_listener_pubkeys \
             WHERE listener_pubkey = $1 AND target_pubkey = $2",
        )
        .bind(&listener_pubkey)
        .bind(&target_pubkey)
        .fetch_one(&pool)
        .await
        .expect("read renewed registration");
        assert!(renewed_at > expired_at);

        sqlx::query(
            "DELETE FROM operator_listener_pubkeys \
             WHERE listener_pubkey = $1 AND target_pubkey = $2",
        )
        .bind(&listener_pubkey)
        .bind(&target_pubkey)
        .execute(&pool)
        .await
        .expect("delete test registration");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_deliveries_skips_rows_past_retention() {
        let pool = setup_pool().await;
        let community_id = Uuid::new_v4();
        let listener = Keys::generate();
        let target = Keys::generate();
        let stale_id = Uuid::new_v4();
        let fresh_id = Uuid::new_v4();
        let stale_at = Utc::now() - OUTBOX_RETENTION - TimeDelta::seconds(1);
        let event_created_at = Utc::now();

        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!(
                "operator-listener-claim-test-{}.example",
                community_id.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert test community");
        for (id, event_marker, created_at) in [
            (stale_id, 1_u8, stale_at),
            (fresh_id, 2_u8, event_created_at),
        ] {
            sqlx::query(
                "INSERT INTO operator_listener_outbox \
                 (id, listener_pubkey, target_pubkey, community_id, event_id, event_kind, \
                  event_created_at, created_at) \
                 VALUES ($1, $2, $3, $4, $5, 9, $6, $7)",
            )
            .bind(id)
            .bind(listener.public_key().as_bytes().as_slice())
            .bind(target.public_key().as_bytes().as_slice())
            .bind(community_id)
            .bind(vec![event_marker; 32])
            .bind(event_created_at)
            .bind(created_at)
            .execute(&pool)
            .await
            .expect("insert test delivery");
        }

        let claimed = claim_deliveries(&pool, 1, Utc::now() + TimeDelta::seconds(30))
            .await
            .expect("claim deliveries");
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, fresh_id);

        sqlx::query("DELETE FROM operator_listener_outbox WHERE community_id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete test deliveries");
        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete test community");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn expired_claim_is_recovered_and_old_claim_token_is_fenced() {
        let pool = setup_pool().await;
        let community_id = Uuid::new_v4();
        let listener = Keys::generate();
        let target = Keys::generate();
        let delivery_id = Uuid::new_v4();

        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!(
                "operator-listener-lease-test-{}.example",
                community_id.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert test community");
        sqlx::query(
            "INSERT INTO operator_listener_outbox \
             (id, listener_pubkey, target_pubkey, community_id, event_id, event_kind, event_created_at) \
             VALUES ($1, $2, $3, $4, $5, 9, $6)",
        )
        .bind(delivery_id)
        .bind(listener.public_key().as_bytes().as_slice())
        .bind(target.public_key().as_bytes().as_slice())
        .bind(community_id)
        .bind(vec![0x33_u8; 32])
        .bind(Utc::now())
        .execute(&pool)
        .await
        .expect("insert test delivery");

        let first = claim_deliveries(&pool, 1, Utc::now() + TimeDelta::seconds(30))
            .await
            .expect("claim delivery");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].id, delivery_id);
        assert!(
            claim_deliveries(&pool, 1, Utc::now() + TimeDelta::seconds(30))
                .await
                .expect("don't claim active lease")
                .is_empty()
        );

        sqlx::query("UPDATE operator_listener_outbox SET lease_until = $2 WHERE id = $1")
            .bind(delivery_id)
            .bind(Utc::now() - TimeDelta::seconds(1))
            .execute(&pool)
            .await
            .expect("expire first lease");
        let recovered = claim_deliveries(&pool, 1, Utc::now() + TimeDelta::seconds(30))
            .await
            .expect("recover expired claim");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].id, delivery_id);
        assert_ne!(recovered[0].claim_id, first[0].claim_id);
        assert_eq!(recovered[0].attempt, first[0].attempt + 1);
        assert!(!complete_delivery(&pool, delivery_id, first[0].claim_id)
            .await
            .expect("old claim cannot complete delivery"));
        assert!(!retry_delivery(
            &pool,
            delivery_id,
            first[0].claim_id,
            Utc::now() + TimeDelta::seconds(10),
        )
        .await
        .expect("old claim cannot retry delivery"));
        assert!(!release_unroutable_delivery(
            &pool,
            delivery_id,
            first[0].claim_id,
            Utc::now() + TimeDelta::seconds(10),
        )
        .await
        .expect("old claim cannot release delivery"));
        assert!(!fail_delivery(&pool, delivery_id, first[0].claim_id)
            .await
            .expect("old claim cannot delete delivery"));
        assert!(complete_delivery(&pool, delivery_id, recovered[0].claim_id)
            .await
            .expect("current claim completes delivery"));

        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete test community");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reapers_delete_expired_rows_and_preserve_recent_rows() {
        let pool = setup_pool().await;
        let listener = Keys::generate();
        let old_target = Keys::generate();
        let recent_target = Keys::generate();
        let listener_pubkey = listener.public_key().as_bytes().to_vec();
        let old_target_pubkey = old_target.public_key().as_bytes().to_vec();
        let recent_target_pubkey = recent_target.public_key().as_bytes().to_vec();

        register_pubkeys(
            &pool,
            &listener_pubkey,
            &[old_target_pubkey.clone(), recent_target_pubkey.clone()],
        )
        .await
        .expect("register test targets");
        let expired_at = Utc::now() - PUBKEY_RETENTION - TimeDelta::seconds(1);
        sqlx::query(
            "UPDATE operator_listener_pubkeys SET created_at = $3 \
             WHERE listener_pubkey = $1 AND target_pubkey = $2",
        )
        .bind(&listener_pubkey)
        .bind(&old_target_pubkey)
        .bind(expired_at)
        .execute(&pool)
        .await
        .expect("age test registration");

        let community_id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!(
                "operator-listener-reaper-test-{}.example",
                community_id.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert test community");
        let old_delivery = Uuid::new_v4();
        let recent_delivery = Uuid::new_v4();
        for (id, marker, created_at) in [
            (
                old_delivery,
                0x44_u8,
                Utc::now() - OUTBOX_RETENTION - TimeDelta::seconds(1),
            ),
            (recent_delivery, 0x55_u8, Utc::now()),
        ] {
            sqlx::query(
                "INSERT INTO operator_listener_outbox \
                 (id, listener_pubkey, target_pubkey, community_id, event_id, event_kind, \
                  event_created_at, created_at) \
                 VALUES ($1, $2, $3, $4, $5, 9, $6, $7)",
            )
            .bind(id)
            .bind(&listener_pubkey)
            .bind(&recent_target_pubkey)
            .bind(community_id)
            .bind(vec![marker; 32])
            .bind(Utc::now())
            .bind(created_at)
            .execute(&pool)
            .await
            .expect("insert test delivery");
        }

        assert!(
            delete_expired_pubkeys(&pool)
                .await
                .expect("reap expired registrations")
                >= 1
        );
        assert!(
            reap_deliveries(&pool)
                .await
                .expect("reap expired deliveries")
                >= 1
        );

        let registration_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM operator_listener_pubkeys \
             WHERE listener_pubkey = $1 AND target_pubkey = $2",
        )
        .bind(&listener_pubkey)
        .bind(&recent_target_pubkey)
        .fetch_one(&pool)
        .await
        .expect("count recent registration");
        let old_delivery_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM operator_listener_outbox WHERE id = $1")
                .bind(old_delivery)
                .fetch_one(&pool)
                .await
                .expect("count expired delivery");
        let recent_delivery_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM operator_listener_outbox WHERE id = $1")
                .bind(recent_delivery)
                .fetch_one(&pool)
                .await
                .expect("count recent delivery");
        assert_eq!(registration_count, 1);
        assert_eq!(old_delivery_count, 0);
        assert_eq!(recent_delivery_count, 1);

        sqlx::query("DELETE FROM operator_listener_outbox WHERE community_id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete test deliveries");
        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete test community");
        sqlx::query("DELETE FROM operator_listener_pubkeys WHERE listener_pubkey = $1")
            .bind(&listener_pubkey)
            .execute(&pool)
            .await
            .expect("delete test registrations");
    }
}
