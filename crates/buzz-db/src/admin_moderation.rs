//! Explicit deployment-global reads for the private deployment-admin plane.
//!
//! This module is the only moderation repository allowed to omit a
//! [`CommunityId`](buzz_core::CommunityId). Keep ordinary moderation reads in
//! [`crate::moderation`] tenant-fenced.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::error::Result;

/// Maximum rows accepted by one admin query.
pub const MAX_PAGE_SIZE: i64 = 200;

fn bounded_limit(limit: i64) -> i64 {
    limit.clamp(1, MAX_PAGE_SIZE)
}

/// Deployment-global moderation report.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminReport {
    /// Report row identifier.
    pub id: Uuid,
    /// Community identifier.
    pub community_id: Uuid,
    /// Community host.
    pub community_host: String,
    /// Signed report event identifier.
    pub report_event_id: String,
    /// Reporter public key.
    pub reporter_pubkey: String,
    /// Target class.
    pub target_kind: String,
    /// Hex target identifier.
    pub target: String,
    /// Optional channel.
    pub channel_id: Option<Uuid>,
    /// NIP-56 report category.
    pub report_type: String,
    /// Private reporter note.
    pub note: Option<String>,
    /// Lifecycle status.
    pub status: String,
    /// Resolving principal pubkey.
    pub resolved_by: Option<String>,
    /// Resolution time.
    pub resolved_at: Option<DateTime<Utc>>,
    /// Linked action.
    pub action_id: Option<Uuid>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

/// Reported message details available only on the admin report detail read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminReportedMessage {
    /// Message author public key.
    pub author_pubkey: String,
    /// Complete message content.
    pub content: String,
    /// Timestamp signed into the message event.
    pub created_at: DateTime<Utc>,
    /// Soft-deletion time, when the message has since been deleted.
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Deployment-global moderation report detail.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminReportDetail {
    /// Report metadata.
    #[serde(flatten)]
    pub report: AdminReport,
    /// Reported message when the report targets a stored event.
    pub message: Option<AdminReportedMessage>,
}

/// Deployment-global product feedback with source-community provenance.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminFeedback {
    /// Feedback row identifier.
    pub id: Uuid,
    /// Source community identifier.
    pub community_id: Uuid,
    /// Source community host.
    pub community_host: String,
    /// Signed feedback event identifier.
    pub event_id: String,
    /// Submitter public key.
    pub submitter_pubkey: String,
    /// Optional feedback category.
    pub category: Option<String>,
    /// Full feedback body.
    pub body: String,
    /// Full source tags, including attachment metadata.
    pub tags: serde_json::Value,
    /// Timestamp signed into the feedback event.
    pub event_created_at: DateTime<Utc>,
    /// Time accepted by this deployment.
    pub received_at: DateTime<Utc>,
}

/// List reports across all communities by stable descending keyset.
#[allow(clippy::too_many_arguments)]
pub async fn list_reports(
    pool: &PgPool,
    community_id: Option<Uuid>,
    status: Option<&str>,
    report_type: Option<&str>,
    target_kind: Option<&str>,
    after: Option<DateTime<Utc>>,
    before: Option<DateTime<Utc>>,
    cursor: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> Result<Vec<AdminReport>> {
    let (cursor_time, cursor_id) = cursor.unzip();
    let rows = sqlx::query(
        r#"
        SELECT r.id, r.community_id, c.host AS community_host,
               r.report_event_id, r.reporter_pubkey, r.target_kind,
               r.target_event_id, r.target_pubkey, r.target_blob_sha256,
               r.channel_id, r.report_type, r.note, r.status, r.resolved_by,
               r.resolved_at, r.action_id, r.created_at
        FROM moderation_reports r
        JOIN communities c ON c.id = r.community_id
        WHERE ($1::uuid IS NULL OR r.community_id = $1)
          AND ($2::text IS NULL OR r.status = $2)
          AND ($3::text IS NULL OR r.report_type = $3)
          AND ($4::text IS NULL OR r.target_kind = $4)
          AND ($5::timestamptz IS NULL OR r.created_at >= $5)
          AND ($6::timestamptz IS NULL OR r.created_at < $6)
          AND ($7::timestamptz IS NULL OR (r.created_at, r.id) < ($7, $8))
        ORDER BY r.created_at DESC, r.id DESC
        LIMIT $9
        "#,
    )
    .bind(community_id)
    .bind(status)
    .bind(report_type)
    .bind(target_kind)
    .bind(after)
    .bind(before)
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(bounded_limit(limit))
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_report).collect()
}

/// Fetch one report globally by its row id, including its event target content.
pub async fn get_report(pool: &PgPool, report_id: Uuid) -> Result<Option<AdminReportDetail>> {
    let row = sqlx::query(
        r#"
        SELECT r.id, r.community_id, c.host AS community_host,
               r.report_event_id, r.reporter_pubkey, r.target_kind,
               r.target_event_id, r.target_pubkey, r.target_blob_sha256,
               r.channel_id, r.report_type, r.note, r.status, r.resolved_by,
               r.resolved_at, r.action_id, r.created_at,
               target.pubkey AS message_author_pubkey,
               target.content AS message_content,
               target.created_at AS message_created_at,
               target.deleted_at AS message_deleted_at
        FROM moderation_reports r
        JOIN communities c ON c.id = r.community_id
        LEFT JOIN LATERAL (
            SELECT e.pubkey, e.content, e.created_at, e.deleted_at
            FROM events e
            WHERE r.target_kind = 'event'
              AND e.community_id = r.community_id
              AND e.id = r.target_event_id
            ORDER BY e.created_at DESC
            LIMIT 1
        ) target ON TRUE
        WHERE r.id = $1
        "#,
    )
    .bind(report_id)
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        let message = row
            .try_get::<Option<Vec<u8>>, _>("message_author_pubkey")?
            .map(|author_pubkey| -> Result<AdminReportedMessage> {
                Ok(AdminReportedMessage {
                    author_pubkey: hex::encode(author_pubkey),
                    content: row.try_get("message_content")?,
                    created_at: row.try_get("message_created_at")?,
                    deleted_at: row.try_get("message_deleted_at")?,
                })
            })
            .transpose()?;
        Ok(AdminReportDetail {
            report: row_to_report(row)?,
            message,
        })
    })
    .transpose()
}

fn row_to_report(row: sqlx::postgres::PgRow) -> Result<AdminReport> {
    let target_kind: String = row.try_get("target_kind")?;
    let target = match target_kind.as_str() {
        "event" => row.try_get::<Vec<u8>, _>("target_event_id")?,
        "pubkey" => row.try_get::<Vec<u8>, _>("target_pubkey")?,
        "blob" => row.try_get::<Vec<u8>, _>("target_blob_sha256")?,
        _ => Vec::new(),
    };
    Ok(AdminReport {
        id: row.try_get("id")?,
        community_id: row.try_get("community_id")?,
        community_host: row.try_get("community_host")?,
        report_event_id: hex::encode(row.try_get::<Vec<u8>, _>("report_event_id")?),
        reporter_pubkey: hex::encode(row.try_get::<Vec<u8>, _>("reporter_pubkey")?),
        target_kind,
        target: hex::encode(target),
        channel_id: row.try_get("channel_id")?,
        report_type: row.try_get("report_type")?,
        note: row.try_get("note")?,
        status: row.try_get("status")?,
        resolved_by: row
            .try_get::<Option<Vec<u8>>, _>("resolved_by")?
            .map(hex::encode),
        resolved_at: row.try_get("resolved_at")?,
        action_id: row.try_get("action_id")?,
        created_at: row.try_get("created_at")?,
    })
}

/// List product feedback across all communities, newest first.
pub async fn list_feedback(pool: &PgPool, limit: i64) -> Result<Vec<AdminFeedback>> {
    let rows = sqlx::query(
        r#"
        SELECT f.id, f.community_id, c.host AS community_host, f.event_id,
               f.submitter_pubkey, f.category, f.body, f.tags,
               f.event_created_at, f.received_at
        FROM product_feedback f
        JOIN communities c ON c.id = f.community_id
        ORDER BY f.received_at DESC, f.id DESC
        LIMIT $1
        "#,
    )
    .bind(bounded_limit(limit))
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_feedback).collect()
}

/// Fetch one feedback submission globally by its row id.
pub async fn get_feedback(pool: &PgPool, id: Uuid) -> Result<Option<AdminFeedback>> {
    let row = sqlx::query(
        r#"
        SELECT f.id, f.community_id, c.host AS community_host, f.event_id,
               f.submitter_pubkey, f.category, f.body, f.tags,
               f.event_created_at, f.received_at
        FROM product_feedback f
        JOIN communities c ON c.id = f.community_id
        WHERE f.id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.map(row_to_feedback).transpose()
}

fn row_to_feedback(row: sqlx::postgres::PgRow) -> Result<AdminFeedback> {
    Ok(AdminFeedback {
        id: row.try_get("id")?,
        community_id: row.try_get("community_id")?,
        community_host: row.try_get("community_host")?,
        event_id: hex::encode(row.try_get::<Vec<u8>, _>("event_id")?),
        submitter_pubkey: hex::encode(row.try_get::<Vec<u8>, _>("submitter_pubkey")?),
        category: row.try_get("category")?,
        body: row.try_get("body")?,
        tags: row.try_get("tags")?,
        event_created_at: row.try_get("event_created_at")?,
        received_at: row.try_get("received_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    async fn insert_community(pool: &PgPool, label: &str) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("admin-report-{label}-{}.example", id.simple()))
            .execute(pool)
            .await
            .expect("insert community");
        id
    }

    async fn insert_event(
        pool: &PgPool,
        community_id: Uuid,
        event_id: &[u8],
        author: &[u8],
        content: &str,
        deleted_at: Option<DateTime<Utc>>,
    ) {
        sqlx::query(
            r#"
            INSERT INTO events (
                community_id, id, pubkey, created_at, kind, tags, content, sig, deleted_at
            ) VALUES ($1, $2, $3, $4, 9, '[]'::jsonb, $5, $6, $7)
            "#,
        )
        .bind(community_id)
        .bind(event_id)
        .bind(author)
        .bind(Utc::now())
        .bind(content)
        .bind(vec![3_u8; 64])
        .bind(deleted_at)
        .execute(pool)
        .await
        .expect("insert event");
    }

    async fn insert_event_report(
        pool: &PgPool,
        community_id: Uuid,
        target_event_id: &[u8],
    ) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO moderation_reports (
                community_id, id, report_event_id, reporter_pubkey,
                target_kind, target_event_id, report_type
            ) VALUES ($1, $2, $3, $4, 'event', $5, 'spam')
            "#,
        )
        .bind(community_id)
        .bind(id)
        .bind(Uuid::new_v4().as_bytes().repeat(2))
        .bind(vec![4_u8; 32])
        .bind(target_event_id)
        .execute(pool)
        .await
        .expect("insert report");
        id
    }

    async fn insert_pubkey_report(pool: &PgPool, community_id: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO moderation_reports (
                community_id, id, report_event_id, reporter_pubkey,
                target_kind, target_pubkey, report_type
            ) VALUES ($1, $2, $3, $4, 'pubkey', $5, 'spam')
            "#,
        )
        .bind(community_id)
        .bind(id)
        .bind(Uuid::new_v4().as_bytes().repeat(2))
        .bind(vec![4_u8; 32])
        .bind(vec![7_u8; 32])
        .execute(pool)
        .await
        .expect("insert report");
        id
    }

    async fn delete_report_fixture(pool: &PgPool, community_id: Uuid) {
        sqlx::query("DELETE FROM moderation_reports WHERE community_id = $1")
            .bind(community_id)
            .execute(pool)
            .await
            .expect("delete report fixture");
        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community_id)
            .execute(pool)
            .await
            .expect("delete community fixture");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn report_detail_reads_only_the_same_community_target_and_includes_deleted_content() {
        let pool = setup_pool().await;
        let report_community = insert_community(&pool, "reported").await;
        let other_community = insert_community(&pool, "other").await;
        let event_id = vec![1_u8; 32];
        let deleted_at = Utc::now();
        insert_event(
            &pool,
            report_community,
            &event_id,
            &[5_u8; 32],
            "reported message",
            Some(deleted_at),
        )
        .await;
        insert_event(
            &pool,
            other_community,
            &event_id,
            &[6_u8; 32],
            "wrong tenant message",
            None,
        )
        .await;
        let report_id = insert_event_report(&pool, report_community, &event_id).await;

        let detail = get_report(&pool, report_id)
            .await
            .expect("query report")
            .expect("report exists");
        let message = detail.message.expect("reported message exists");
        assert_eq!(message.content, "reported message");
        assert_eq!(message.author_pubkey, hex::encode([5_u8; 32]));
        assert!(message.deleted_at.is_some());

        sqlx::query("DELETE FROM moderation_reports WHERE community_id = $1")
            .bind(report_community)
            .execute(&pool)
            .await
            .expect("delete report fixture");
        sqlx::query("DELETE FROM events WHERE community_id = ANY($1)")
            .bind(vec![report_community, other_community])
            .execute(&pool)
            .await
            .expect("delete event fixtures");
        sqlx::query("DELETE FROM communities WHERE id = ANY($1)")
            .bind(vec![report_community, other_community])
            .execute(&pool)
            .await
            .expect("delete community fixtures");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn report_detail_has_no_message_for_non_event_target() {
        let pool = setup_pool().await;
        let community_id = insert_community(&pool, "pubkey-target").await;
        let report_id = insert_pubkey_report(&pool, community_id).await;

        let detail = get_report(&pool, report_id)
            .await
            .expect("query report")
            .expect("report exists");
        assert_eq!(detail.report.target_kind, "pubkey");
        assert!(detail.message.is_none());

        delete_report_fixture(&pool, community_id).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn report_detail_has_no_message_when_event_row_is_missing() {
        let pool = setup_pool().await;
        let community_id = insert_community(&pool, "missing-event").await;
        let missing_event_id = vec![8_u8; 32];
        let report_id = insert_event_report(&pool, community_id, &missing_event_id).await;

        let detail = get_report(&pool, report_id)
            .await
            .expect("query report")
            .expect("report exists");
        assert_eq!(detail.report.target_kind, "event");
        assert_eq!(detail.report.target, hex::encode(missing_event_id));
        assert!(detail.message.is_none());

        delete_report_fixture(&pool, community_id).await;
    }
}
