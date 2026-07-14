//! Persistence for deployment-level Buzz product feedback.
//!
//! Feedback retains its source [`CommunityId`] as provenance, but is not a
//! community moderation concern and is never inserted into the events table.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::{error::Result, CommunityId};

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
    /// Source community, retained as provenance only.
    pub community_id: Uuid,
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

/// Insert product feedback, idempotent by signed event id.
pub async fn insert(
    pool: &PgPool,
    community: CommunityId,
    feedback: NewProductFeedback<'_>,
) -> Result<Uuid> {
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
    .fetch_one(pool)
    .await?;

    Ok(row.try_get("id")?)
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
