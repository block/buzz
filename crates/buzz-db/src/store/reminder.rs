//! Event-reminder delivery query, claim, and release persistence.

use buzz_core::kind::KIND_EVENT_REMINDER;
use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::Result;
use crate::Db;

/// A due reminder row returned by [`query_due_reminders`].
#[derive(Debug)]
pub struct DueReminder {
    /// Server-resolved community this reminder row belongs to.
    pub community_id: CommunityId,
    /// Normalized host mapped to that community.
    pub host: String,
    /// The event's raw ID bytes.
    pub id: Vec<u8>,
    /// The event's pubkey bytes.
    pub pubkey: Vec<u8>,
    /// The event's `created_at` timestamp.
    pub created_at: DateTime<Utc>,
    /// The event's kind (always 30300).
    pub kind: i32,
    /// The event's JSONB tags.
    pub tags: serde_json::Value,
    /// The event's encrypted content.
    pub content: String,
    /// The event's signature bytes.
    pub sig: Vec<u8>,
    /// The channel ID (always None for reminders — global events).
    pub channel_id: Option<Uuid>,
}

/// Query due reminders: latest-per-address `kind:30300` rows where
/// `not_before <= now`, `deleted_at IS NULL`, `delivered_at IS NULL`.
///
/// Returns the latest head per `(pubkey, d_tag)` using canonical NIP-16
/// ordering (`created_at DESC, id ASC`).
pub async fn query_due_reminders(
    pool: &PgPool,
    now_secs: i64,
    batch_limit: i64,
) -> Result<Vec<DueReminder>> {
    let kind_i32 = KIND_EVENT_REMINDER as i32;
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT ON (e.community_id, e.pubkey, e.d_tag)
            e.community_id, c.host, e.id, e.pubkey, e.created_at, e.kind, e.tags, e.content, e.sig, e.channel_id
        FROM events AS e
        JOIN communities AS c ON c.id = e.community_id
        WHERE e.kind = $1
          AND e.not_before IS NOT NULL
          AND e.not_before <= $2
          AND e.deleted_at IS NULL
          AND e.delivered_at IS NULL
          AND c.archived_at IS NULL
        ORDER BY e.community_id, e.pubkey, e.d_tag, e.created_at DESC, e.id ASC
        LIMIT $3
        "#,
    )
    .bind(kind_i32)
    .bind(now_secs)
    .bind(batch_limit)
    .fetch_all(pool)
    .await?;

    let results = rows
        .into_iter()
        .map(|row| DueReminder {
            community_id: CommunityId::from_uuid(row.get("community_id")),
            host: row.get("host"),
            id: row.get("id"),
            pubkey: row.get("pubkey"),
            created_at: row.get("created_at"),
            kind: row.get("kind"),
            tags: row.get("tags"),
            content: row.get("content"),
            sig: row.get("sig"),
            channel_id: row.get("channel_id"),
        })
        .collect();

    Ok(results)
}

/// Atomically claim a due reminder for delivery. Returns `Some(id)` if this
/// caller won the claim (set `delivered_at`), or `None` if another pod already
/// claimed it. Mirrors the reaper's `archived_at IS NULL` guard for cross-pod
/// idempotency.
pub async fn claim_due_reminder(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
) -> Result<bool> {
    claim_due_reminder_with_stamp(
        pool,
        community_id,
        event_id,
        event_created_at,
        Utc::now().timestamp(),
    )
    .await
}

/// Atomically claim a due reminder using a caller-supplied delivery stamp.
///
/// The same stamp should be passed to [`release_due_reminder`] if the publish
/// side effect fails, so rollback can compare-and-clear only this pod's claim.
///
/// Scoped by `community_id`: `events` is keyed `(community_id, created_at, id)`,
/// and the same Nostr event id (hence the same `id`/`created_at` pair) is
/// allowed across communities. Without the community predicate a claim for
/// `A/X` would also mark `B/X` delivered. The caller already holds the owning
/// community on the `DueReminder` row.
pub async fn claim_due_reminder_with_stamp(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
    delivery_stamp: i64,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE events
        SET delivered_at = $1
        WHERE community_id = $2 AND created_at = $3 AND id = $4 AND delivered_at IS NULL
        "#,
    )
    .bind(delivery_stamp)
    .bind(community_id.as_uuid())
    .bind(event_created_at)
    .bind(event_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Release a previously claimed reminder when publish fails.
///
/// The `delivery_stamp` must be the exact value written by the claiming pod;
/// that compare-and-clear prevents one pod from rolling back another pod's
/// later claim after a retry/race.
///
/// Scoped by `community_id` for the same reason as the claim: a release for
/// `A/X` must not clear `B/X` even when their `id`/`created_at`/stamp coincide.
pub async fn release_due_reminder(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
    delivery_stamp: i64,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE events
        SET delivered_at = NULL
        WHERE community_id = $1
          AND created_at = $2
          AND id = $3
          AND delivered_at = $4
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(event_created_at)
    .bind(event_id)
    .bind(delivery_stamp)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}

impl Db {
    /// Query due reminders ready for delivery.
    #[datastore_span(name = "query_due_reminders", system = "postgresql")]
    pub async fn query_due_reminders(
        &self,
        now_secs: i64,
        batch_limit: i64,
    ) -> Result<Vec<DueReminder>> {
        crate::reminder::query_due_reminders(&self.pool, now_secs, batch_limit).await
    }

    /// Atomically claim a due reminder for delivery (cross-pod dedup).
    #[datastore_span(name = "claim_due_reminder", system = "postgresql")]
    pub async fn claim_due_reminder(
        &self,
        community_id: CommunityId,
        event_id: &[u8],
        event_created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        crate::reminder::claim_due_reminder(&self.pool, community_id, event_id, event_created_at)
            .await
    }

    /// Atomically claim a due reminder using a caller-supplied delivery stamp.
    #[datastore_span(name = "claim_due_reminder_with_stamp", system = "postgresql")]
    pub async fn claim_due_reminder_with_stamp(
        &self,
        community_id: CommunityId,
        event_id: &[u8],
        event_created_at: chrono::DateTime<chrono::Utc>,
        delivery_stamp: i64,
    ) -> Result<bool> {
        crate::reminder::claim_due_reminder_with_stamp(
            &self.pool,
            community_id,
            event_id,
            event_created_at,
            delivery_stamp,
        )
        .await
    }

    /// Release a claimed due reminder after a publish failure.
    #[datastore_span(name = "release_due_reminder", system = "postgresql")]
    pub async fn release_due_reminder(
        &self,
        community_id: CommunityId,
        event_id: &[u8],
        event_created_at: chrono::DateTime<chrono::Utc>,
        delivery_stamp: i64,
    ) -> Result<bool> {
        crate::reminder::release_due_reminder(
            &self.pool,
            community_id,
            event_id,
            event_created_at,
            delivery_stamp,
        )
        .await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::event::insert_event;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1 -- local test-only credentials

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());

        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    async fn make_test_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        let host = format!("event-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        id
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn query_due_reminders_returns_row_community_and_host_per_tenant() {
        let pool = setup_pool().await;
        let community_a_uuid = make_test_community(&pool).await;
        let community_b_uuid = make_test_community(&pool).await;
        let community_a = CommunityId::from_uuid(community_a_uuid);
        let community_b = CommunityId::from_uuid(community_b_uuid);
        let host_a: String = sqlx::query_scalar("SELECT host FROM communities WHERE id = $1")
            .bind(community_a_uuid)
            .fetch_one(&pool)
            .await
            .expect("load host A");
        let host_b: String = sqlx::query_scalar("SELECT host FROM communities WHERE id = $1")
            .bind(community_b_uuid)
            .fetch_one(&pool)
            .await
            .expect("load host B");

        let not_before = Utc::now().timestamp() - 1;
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();
        let event_a = EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), "a")
            .tags([
                Tag::parse(["d", "due-reminder-scope-a"]).unwrap(),
                Tag::parse(["not_before", &not_before.to_string()]).unwrap(),
            ])
            .sign_with_keys(&keys_a)
            .expect("sign A");
        let event_b = EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), "b")
            .tags([
                Tag::parse(["d", "due-reminder-scope-b"]).unwrap(),
                Tag::parse(["not_before", &not_before.to_string()]).unwrap(),
            ])
            .sign_with_keys(&keys_b)
            .expect("sign B");

        insert_event(&pool, community_a, &event_a, None)
            .await
            .expect("insert A");
        insert_event(&pool, community_b, &event_b, None)
            .await
            .expect("insert B");

        let due = query_due_reminders(&pool, Utc::now().timestamp(), 100)
            .await
            .expect("query due reminders");

        assert!(due.iter().any(|row| {
            row.id == event_a.id.as_bytes() && row.community_id == community_a && row.host == host_a
        }));
        assert!(due.iter().any(|row| {
            row.id == event_b.id.as_bytes() && row.community_id == community_b && row.host == host_b
        }));
    }

    /// Two pods race to claim the same due reminder: exactly one wins. The
    /// scheduler publishes only on a winning claim (`Ok(true)`) and `continue`s
    /// on the loser (`Ok(false)`), so a single winning claim *is* the proof of
    /// exactly one publish side effect across N pods.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_due_reminder_is_won_by_exactly_one_of_two_racing_pods() {
        let pool = setup_pool().await;
        let community = CommunityId::from_uuid(make_test_community(&pool).await);
        let not_before = Utc::now().timestamp() - 1;
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), "due")
            .tags([
                Tag::parse(["d", "due-reminder-claim-race"]).unwrap(),
                Tag::parse(["not_before", &not_before.to_string()]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign reminder");
        insert_event(&pool, community, &event, None)
            .await
            .expect("insert reminder");

        let id = event.id.as_bytes().to_vec();
        let created_at = event.created_at.as_secs() as i64;
        let created_at = chrono::DateTime::from_timestamp(created_at, 0).expect("created_at");

        // Two pods, two distinct per-attempt stamps, same reminder.
        let stamp_p1: i64 = 0x1111_1111_1111_1111;
        let stamp_p2: i64 = 0x2222_2222_2222_2222;
        let won_p1 = claim_due_reminder_with_stamp(&pool, community, &id, created_at, stamp_p1)
            .await
            .expect("p1 claim");
        let won_p2 = claim_due_reminder_with_stamp(&pool, community, &id, created_at, stamp_p2)
            .await
            .expect("p2 claim");

        assert!(
            won_p1 ^ won_p2,
            "exactly one pod must win the claim (p1={won_p1}, p2={won_p2}) — \
         the loser never reaches the publish side effect"
        );
    }

    /// A failed publish releases the claim so the reminder is redeliverable,
    /// and the compare-and-clear stamp guard prevents one pod from rolling back
    /// another pod's claim.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn release_due_reminder_rolls_back_only_the_matching_stamp() {
        let pool = setup_pool().await;
        let community = CommunityId::from_uuid(make_test_community(&pool).await);
        let not_before = Utc::now().timestamp() - 1;
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), "due")
            .tags([
                Tag::parse(["d", "due-reminder-release"]).unwrap(),
                Tag::parse(["not_before", &not_before.to_string()]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign reminder");
        insert_event(&pool, community, &event, None)
            .await
            .expect("insert reminder");

        let id = event.id.as_bytes().to_vec();
        let created_at = event.created_at.as_secs() as i64;
        let created_at = chrono::DateTime::from_timestamp(created_at, 0).expect("created_at");
        let stamp: i64 = 0x3333_3333_3333_3333;

        assert!(
            claim_due_reminder_with_stamp(&pool, community, &id, created_at, stamp)
                .await
                .expect("claim"),
            "first claim wins"
        );

        // A release with the *wrong* stamp must be a no-op (does not clear
        // another pod's claim).
        assert!(
            !release_due_reminder(&pool, community, &id, created_at, stamp ^ 0xFFFF)
                .await
                .expect("wrong-stamp release"),
            "release with a non-matching stamp must not clear the claim"
        );
        assert!(
            !claim_due_reminder_with_stamp(&pool, community, &id, created_at, stamp)
                .await
                .expect("re-claim after no-op release"),
            "reminder must still be claimed after a no-op release"
        );

        // The matching-stamp release rolls the claim back; the reminder is
        // redeliverable and a subsequent claim wins again.
        assert!(
            release_due_reminder(&pool, community, &id, created_at, stamp)
                .await
                .expect("matching-stamp release"),
            "release with the claiming stamp must clear the claim"
        );
        assert!(
            claim_due_reminder_with_stamp(&pool, community, &id, created_at, stamp)
                .await
                .expect("re-claim after release"),
            "released reminder must be reclaimable for retry"
        );
    }

    /// Cross-community confinement: the same Nostr reminder event (identical
    /// `id` and `created_at`) inserted into communities A and B must claim and
    /// release independently. A claim/release for `A/X` must never touch `B/X`.
    ///
    /// This is the primitive the scheduler's exactly-once-publish proof rests
    /// on: `events` is keyed `(community_id, created_at, id)`, so without the
    /// community predicate a claim for A would mark B delivered (suppressing
    /// B's reminder) and a matching-stamp release for A would clear B.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reminder_claim_and_release_are_confined_to_their_community() {
        let pool = setup_pool().await;
        let community_a = CommunityId::from_uuid(make_test_community(&pool).await);
        let community_b = CommunityId::from_uuid(make_test_community(&pool).await);

        // One signed event, inserted into both communities — same id/created_at.
        let not_before = Utc::now().timestamp() - 1;
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), "due")
            .tags([
                Tag::parse(["d", "due-reminder-cross-community"]).unwrap(),
                Tag::parse(["not_before", &not_before.to_string()]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign reminder");
        insert_event(&pool, community_a, &event, None)
            .await
            .expect("insert A/X");
        insert_event(&pool, community_b, &event, None)
            .await
            .expect("insert B/X");

        let id = event.id.as_bytes().to_vec();
        let created_at = event.created_at.as_secs() as i64;
        let created_at = chrono::DateTime::from_timestamp(created_at, 0).expect("created_at");
        let stamp: i64 = 0x4444_4444_4444_4444;

        // Claim A/X. B/X must remain claimable — A's claim did not mark B.
        assert!(
            claim_due_reminder_with_stamp(&pool, community_a, &id, created_at, stamp)
                .await
                .expect("claim A"),
            "A/X claim wins"
        );
        assert!(
            claim_due_reminder_with_stamp(&pool, community_b, &id, created_at, stamp)
                .await
                .expect("claim B"),
            "B/X must still be claimable after A/X is claimed — \
         a claim for A must not mark B delivered"
        );

        // Both are now claimed under the same stamp. A matching-stamp release
        // for A/X must clear only A/X; B/X must stay claimed.
        assert!(
            release_due_reminder(&pool, community_a, &id, created_at, stamp)
                .await
                .expect("release A"),
            "A/X release with the claiming stamp clears A/X"
        );
        assert!(
            !claim_due_reminder_with_stamp(&pool, community_b, &id, created_at, stamp)
                .await
                .expect("re-claim B after A release"),
            "B/X must remain claimed after A/X is released — \
         a release for A must not clear B"
        );
        // And A/X is genuinely redeliverable (the release was real, not a no-op).
        assert!(
            claim_due_reminder_with_stamp(&pool, community_a, &id, created_at, stamp)
                .await
                .expect("re-claim A after release"),
            "A/X must be reclaimable after its own release"
        );
    }
}
