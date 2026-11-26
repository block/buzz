//! Persistence for deployment-level Buzz product feedback.
//!
//! Feedback retains its source [`CommunityId`] as provenance, but is not a
//! community moderation concern and is never inserted into the events table.

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::{error::Result, CommunityId, Db};

/// Validated fields from an accepted product-feedback event.
#[derive(Debug, Clone)]
pub struct NewProductFeedback<'a> {
    /// Signed feedback event id (32 bytes), used for idempotency.
    pub event_id: &'a [u8],
    /// Authenticated submitter's Nostr pubkey (32 bytes).
    pub submitter_pubkey: &'a [u8],
    /// Optional category from the relay-validated vocabulary.
    pub category: Option<&'a str>,
    /// Required free-text feedback body.
    pub body: &'a str,
    /// Full validated event tags (attachments and diagnostics metadata included).
    pub tags: &'a serde_json::Value,
    /// Timestamp signed into the source event.
    pub event_created_at: DateTime<Utc>,
}

/// Product-feedback row returned to deployment-operator tooling.
#[derive(Debug, Clone, Serialize)]
pub struct ProductFeedbackRecord {
    /// Sidecar row id.
    pub id: Uuid,
    /// Source community, retained as provenance only. `None` once that
    /// community's purge has cleared the provenance.
    pub community_id: Option<Uuid>,
    /// Signed source event id.
    pub event_id: String,
    /// Signed submitter pubkey.
    pub submitter_pubkey: String,
    /// Optional feedback category.
    pub category: Option<String>,
    /// Feedback body.
    pub body: String,
    /// Full source tags, including attachment and diagnostics metadata.
    pub tags: serde_json::Value,
    /// Timestamp signed into the source event.
    pub event_created_at: DateTime<Utc>,
    /// Time accepted by this deployment.
    pub received_at: DateTime<Utc>,
}

/// Insert product feedback, idempotent deployment-wide by signed event id.
///
/// The first accepted submission owns the provenance row. Replaying the exact
/// same signed event through another community returns the same row without
/// changing its source community; callers intentionally receive the same
/// successful acknowledgment in both cases.
///
/// The community must exist, and nothing is written when it does not: this
/// returns [`DbError::NotFound`]. Any lifecycle
/// state qualifies: the table is provenance-only
/// and unfenced (`docs/database.md`), so feedback stays writable while its
/// community is being deleted, and the purge clears its provenance.
///
/// [`DbError::NotFound`]: crate::error::DbError::NotFound
pub async fn insert(
    pool: &PgPool,
    community: CommunityId,
    feedback: NewProductFeedback<'_>,
) -> Result<Uuid> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        r#"
        INSERT INTO product_feedback (
            community_id, event_id, submitter_pubkey, category, body, tags,
            event_created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (event_id) DO UPDATE SET
            event_id = EXCLUDED.event_id
        RETURNING id
        "#,
    )
    .bind(community.as_uuid())
    .bind(feedback.event_id)
    .bind(feedback.submitter_pubkey)
    .bind(feedback.category)
    .bind(feedback.body)
    .bind(feedback.tags)
    .bind(feedback.event_created_at)
    .fetch_one(&mut *tx)
    .await?;
    let id = row.try_get("id")?;
    // Check after the write: the purge locks feedback
    // rows before the community, so taking the community first would deadlock
    // with a replay that overlaps it.
    crate::store::reference_check::require_community(&mut tx, community).await?;
    tx.commit().await?;

    Ok(id)
}

/// List feedback across all communities, newest received first.
pub async fn list(pool: &PgPool, limit: i64) -> Result<Vec<ProductFeedbackRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, community_id, event_id, submitter_pubkey, category, body,
               tags, event_created_at, received_at
        FROM product_feedback
        ORDER BY received_at DESC, id
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(ProductFeedbackRecord {
                id: row.try_get("id")?,
                community_id: row.try_get("community_id")?,
                event_id: hex::encode(row.try_get::<Vec<u8>, _>("event_id")?),
                submitter_pubkey: hex::encode(row.try_get::<Vec<u8>, _>("submitter_pubkey")?),
                category: row.try_get("category")?,
                body: row.try_get("body")?,
                tags: row.try_get("tags")?,
                event_created_at: row.try_get("event_created_at")?,
                received_at: row.try_get("received_at")?,
            })
        })
        .collect()
}

impl Db {
    /// Sidecar an accepted product-feedback event, idempotent by event id.
    #[datastore_span(name = "insert_product_feedback", system = "postgresql")]
    pub async fn insert_product_feedback(
        &self,
        community: CommunityId,
        feedback: NewProductFeedback<'_>,
    ) -> Result<Uuid> {
        insert(&self.pool, community, feedback).await
    }

    /// List product feedback across the deployment, newest first.
    #[datastore_span(name = "list_product_feedback", system = "postgresql")]
    pub async fn list_product_feedback(&self, limit: i64) -> Result<Vec<ProductFeedbackRecord>> {
        list(&self.pool, limit).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn duplicate_event_keeps_first_community_provenance() {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("BUZZ_TEST_DATABASE_URL or DATABASE_URL");
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect test DB");
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        for (id, host) in [
            (first, format!("feedback-first-{first}.test")),
            (second, format!("feedback-second-{second}.test")),
        ] {
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(id)
                .bind(host)
                .execute(&pool)
                .await
                .expect("insert community");
        }

        let mut event_id = [0_u8; 32];
        event_id[..16].copy_from_slice(Uuid::new_v4().as_bytes());
        event_id[16..].copy_from_slice(Uuid::new_v4().as_bytes());
        let pubkey = [7_u8; 32];
        let tags = serde_json::json!([["category", "bug"]]);
        let feedback = || NewProductFeedback {
            event_id: &event_id,
            submitter_pubkey: &pubkey,
            category: Some("bug"),
            body: "same signed feedback",
            tags: &tags,
            event_created_at: Utc::now(),
        };

        let first_row = insert(&pool, CommunityId::from_uuid(first), feedback())
            .await
            .expect("first insert");
        let duplicate_row = insert(&pool, CommunityId::from_uuid(second), feedback())
            .await
            .expect("duplicate insert");
        assert_eq!(duplicate_row, first_row);

        let rows: Vec<Uuid> =
            sqlx::query_scalar("SELECT community_id FROM product_feedback WHERE event_id = $1")
                .bind(event_id.as_slice())
                .fetch_all(&pool)
                .await
                .expect("read feedback provenance");
        assert_eq!(rows, vec![first]);

        sqlx::query("DELETE FROM product_feedback WHERE event_id = $1")
            .bind(event_id.as_slice())
            .execute(&pool)
            .await
            .expect("delete feedback");
        sqlx::query("DELETE FROM communities WHERE id = ANY($1)")
            .bind(vec![first, second])
            .execute(&pool)
            .await
            .expect("delete communities");
    }

    // -- Reference check (FK retirement row 18) -------------------------------
    //
    // These tests drop `product_feedback -> communities` from the per-test
    // database first, so a pass proves the in-transaction check, not the
    // constraint.

    async fn per_test_pool() -> PgPool {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect test database");
        pool
    }

    async fn new_community(pool: &PgPool) -> CommunityId {
        Db::from_pool(pool.clone())
            .ensure_configured_community(&format!("feedback-{}.example", Uuid::new_v4()))
            .await
            .expect("create community")
            .id
    }

    fn event_id() -> [u8; 32] {
        let mut id = [0_u8; 32];
        id[..16].copy_from_slice(Uuid::new_v4().as_bytes());
        id[16..].copy_from_slice(Uuid::new_v4().as_bytes());
        id
    }

    async fn insert_feedback(
        pool: &PgPool,
        community: CommunityId,
        event_id: &[u8; 32],
    ) -> Result<Uuid> {
        let tags = serde_json::json!([]);
        insert(
            pool,
            community,
            NewProductFeedback {
                event_id,
                submitter_pubkey: &[7_u8; 32],
                category: None,
                body: "feedback",
                tags: &tags,
                event_created_at: Utc::now(),
            },
        )
        .await
    }

    async fn feedback_rows(pool: &PgPool, event_id: &[u8; 32]) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM product_feedback WHERE event_id = $1")
            .bind(event_id.as_slice())
            .fetch_one(pool)
            .await
            .expect("count feedback")
    }

    /// A community that never existed gets no feedback row. One that is being
    /// deleted still accepts feedback (the table is unfenced), and an active
    /// one accepts it idempotently.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn insert_rejects_missing_community_without_foreign_key() {
        let pool = per_test_pool().await;

        let missing = event_id();
        match insert_feedback(&pool, CommunityId::from_uuid(Uuid::new_v4()), &missing).await {
            Err(crate::error::DbError::NotFound(message)) => {
                assert!(
                    message.contains("community"),
                    "unexpected message: {message}"
                )
            }
            other => panic!("expected NotFound(community), got {other:?}"),
        }
        assert_eq!(feedback_rows(&pool, &missing).await, 0);

        let fenced = new_community(&pool).await;
        let mut tx = pool.begin().await.expect("begin fence");
        sqlx::query(
            "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                    set_config('buzz.deletion_fence_generation', \
                        (SELECT deletion_fence_generation::text FROM communities WHERE id = $2), true)",
        )
        .bind(fenced.as_uuid().to_string())
        .bind(fenced.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("authorize fence transition");
        sqlx::query("UPDATE communities SET deletion_state = 'fenced' WHERE id = $1")
            .bind(fenced.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("fence community");
        tx.commit().await.expect("commit fence");
        let id = event_id();
        insert_feedback(&pool, fenced, &id)
            .await
            .expect("a community being deleted still accepts feedback");
        assert_eq!(feedback_rows(&pool, &id).await, 1);

        let community = new_community(&pool).await;
        let id = event_id();
        let first = insert_feedback(&pool, community, &id)
            .await
            .expect("active community");
        let replay = insert_feedback(&pool, community, &id)
            .await
            .expect("replay");
        assert_eq!(first, replay);
        assert_eq!(feedback_rows(&pool, &id).await, 1);
    }

    /// The check holds `FOR KEY SHARE` on the community row, so a concurrent
    /// delete of that row (which locks it `FOR UPDATE` first) and the insert
    /// are ordered. No foreign key exists, so only the explicit lock can block
    /// the insert behind a community held `FOR UPDATE`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn insert_key_share_locks_community_without_foreign_key() {
        let pool = per_test_pool().await;
        let community = new_community(&pool).await;

        let mut holder = pool.begin().await.expect("begin community lock holder");
        sqlx::query("SELECT 1 FROM communities WHERE id = $1 FOR UPDATE")
            .bind(community.as_uuid())
            .execute(&mut *holder)
            .await
            .expect("lock community");

        let name = format!("feedback-writer-{}", Uuid::new_v4().simple());
        let writer = crate::test_support::named_pool(&name).await;
        let id = event_id();
        let write = tokio::spawn(async move { insert_feedback(&writer, community, &id).await });
        crate::test_support::wait_for_lock_wait(&pool, &name).await;
        holder.commit().await.expect("release community lock");

        write
            .await
            .expect("join writer")
            .expect("insert proceeds once the community lock is released");
        assert_eq!(feedback_rows(&pool, &id).await, 1);
    }

    /// The purge clears provenance with `SET community_id = NULL`; the operator
    /// listing must still return those rows.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn list_returns_rows_with_cleared_provenance() {
        let pool = per_test_pool().await;
        let community = new_community(&pool).await;
        let id = event_id();
        let row = insert_feedback(&pool, community, &id)
            .await
            .expect("insert feedback");
        sqlx::query("UPDATE product_feedback SET community_id = NULL WHERE id = $1")
            .bind(row)
            .execute(&pool)
            .await
            .expect("clear provenance as the purge does");

        let listed = list(&pool, 1_000).await.expect("list feedback");
        let record = listed
            .iter()
            .find(|record| record.id == row)
            .expect("cleared row is listed");
        assert_eq!(record.community_id, None);
    }
}
