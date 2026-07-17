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

/// Fetch one report globally by its row id.
pub async fn get_report(pool: &PgPool, report_id: Uuid) -> Result<Option<AdminReport>> {
    let row = sqlx::query(
        r#"
        SELECT r.id, r.community_id, c.host AS community_host,
               r.report_event_id, r.reporter_pubkey, r.target_kind,
               r.target_event_id, r.target_pubkey, r.target_blob_sha256,
               r.channel_id, r.report_type, r.note, r.status, r.resolved_by,
               r.resolved_at, r.action_id, r.created_at
        FROM moderation_reports r
        JOIN communities c ON c.id = r.community_id
        WHERE r.id = $1
        "#,
    )
    .bind(report_id)
    .fetch_optional(pool)
    .await?;
    row.map(row_to_report).transpose()
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
               f.submitter_pubkey, f.category, f.body,
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
               f.submitter_pubkey, f.category, f.body,
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
        event_created_at: row.try_get("event_created_at")?,
        received_at: row.try_get("received_at")?,
    })
}
