//! Explicit deployment-global reads for the private deployment-admin plane.
//!
//! This module is the only moderation repository allowed to omit a
//! [`CommunityId`](buzz_core::CommunityId). Keep ordinary moderation reads in
//! [`crate::moderation`] tenant-fenced.

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::error::Result;
use crate::Db;

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

/// The `relay_admin_actions` enforcement record governing a report.
///
/// Populated on the report detail read and enforcement resolve response. Carries
/// the durable state machine so the console can render enforcement progress or
/// terminal outcome without inventing a shape the relay never emits.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminActionDto {
    /// Action row identifier.
    pub id: Uuid,
    /// Client-generated idempotency key.
    pub request_id: Uuid,
    /// Principal who claimed the report.
    pub actor_pubkey: String,
    /// Role of the actor: `"operator"` | `"moderator"`.
    pub actor_role: String,
    /// Enforcement action name: `"delete"` | `"kick"` | `"ban"` | `"timeout"`.
    pub action: String,
    /// State machine: `"pending"` | `"enforcing"` | `"succeeded"` | `"failed"` | `"cancelled"`.
    pub status: String,
    /// Principal who cancelled the action (hex pubkey); null unless `status` is
    /// `"cancelled"`. Attributes the one mutation that would otherwise carry no
    /// actor trail while `BUZZ_AUDIT_ENABLED=false`.
    pub cancelled_by: Option<String>,
    /// Operator reason, if provided.
    pub reason: Option<String>,
    /// Absolute timeout expiry for `timeout` actions; null otherwise. Absolute
    /// (not remaining-seconds) so repeated reads never disagree; the client
    /// computes remaining time.
    pub expires_at: Option<DateTime<Utc>>,
    /// Error from the last failure, if any.
    pub error_message: Option<String>,
    /// Action creation time.
    pub created_at: DateTime<Utc>,
    /// Action last-updated time.
    pub updated_at: DateTime<Utc>,
}

impl AdminActionDto {
    /// Build the wire DTO from a persistence record. Used to embed the
    /// just-cancelled action in the cancel response — the last look at a record
    /// that a subsequent detail read (report back to `open`) no longer surfaces.
    pub fn from_record(record: &crate::relay_admin_actions::AdminActionRecord) -> Self {
        Self {
            id: record.id,
            request_id: record.request_id,
            actor_pubkey: hex::encode(&record.actor_pubkey),
            actor_role: record.actor_role.clone(),
            action: record.action.clone(),
            status: record.state.clone(),
            cancelled_by: record.cancelled_by.as_deref().map(hex::encode),
            reason: record.reason.clone(),
            expires_at: record.timeout_until,
            error_message: record.error_message.clone(),
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
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
    /// Governing enforcement action, when one exists (live or terminal). Null
    /// for reports never enforced via the HTTP admin plane.
    pub active_action: Option<AdminActionDto>,
}

/// Deployment-global product feedback with source-community provenance.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminFeedback {
    /// Feedback row identifier.
    pub id: Uuid,
    /// Source community identifier. `None` once the source community has been
    /// purged: `product_feedback` is deployment-global operator evidence whose
    /// `community_id` is severed to NULL on tenant purge, not cascade-deleted.
    pub community_id: Option<Uuid>,
    /// Source community host. `None` when `community_id` is severed (no row to
    /// join) — the feedback is retained without its origin tenant.
    pub community_host: Option<String>,
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
    /// Operator-managed lifecycle status: `"new"` | `"reviewed"` | `"archived"`.
    pub status: String,
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
               target.deleted_at AS message_deleted_at,
               act.id AS action_id_admin,
               act.request_id AS action_request_id,
               act.actor_pubkey AS action_actor_pubkey,
               act.actor_role AS action_actor_role,
               act.action AS action_name,
               act.state AS action_state,
               act.cancelled_by AS action_cancelled_by,
               act.reason AS action_reason,
               act.timeout_until AS action_timeout_until,
               act.error_message AS action_error_message,
               act.created_at AS action_created_at,
               act.updated_at AS action_updated_at
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
        LEFT JOIN LATERAL (
            SELECT a.id, a.request_id, a.actor_pubkey, a.actor_role, a.action,
                   a.state, a.cancelled_by, a.reason, a.timeout_until, a.error_message,
                   a.created_at, a.updated_at
            FROM relay_admin_actions a
            WHERE a.report_community_id = r.community_id
              AND a.report_id = r.id
              AND a.action IN ('delete', 'kick', 'ban', 'timeout')
              AND (a.id = r.active_action_id OR a.state = 'succeeded')
            ORDER BY a.created_at DESC, a.id DESC
            LIMIT 1
        ) act ON TRUE
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
        let active_action = row_to_action_dto(&row)?;
        Ok(AdminReportDetail {
            report: row_to_report(row)?,
            message,
            active_action,
        })
    })
    .transpose()
}

/// Build an [`AdminActionDto`] from the LATERAL-joined `act.*` columns, when a
/// governing action was found. Returns `None` when the join produced no row
/// (all `act.*` columns null).
fn row_to_action_dto(row: &sqlx::postgres::PgRow) -> Result<Option<AdminActionDto>> {
    let Some(id) = row.try_get::<Option<Uuid>, _>("action_id_admin")? else {
        return Ok(None);
    };
    Ok(Some(AdminActionDto {
        id,
        request_id: row.try_get("action_request_id")?,
        actor_pubkey: hex::encode(row.try_get::<Vec<u8>, _>("action_actor_pubkey")?),
        actor_role: row.try_get("action_actor_role")?,
        action: row.try_get("action_name")?,
        status: row.try_get("action_state")?,
        cancelled_by: row
            .try_get::<Option<Vec<u8>>, _>("action_cancelled_by")?
            .map(hex::encode),
        reason: row.try_get("action_reason")?,
        expires_at: row.try_get("action_timeout_until")?,
        error_message: row.try_get("action_error_message")?,
        created_at: row.try_get("action_created_at")?,
        updated_at: row.try_get("action_updated_at")?,
    }))
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
               f.submitter_pubkey, f.category, f.body, f.tags, f.status,
               f.event_created_at, f.received_at
        FROM product_feedback f
        LEFT JOIN communities c ON c.id = f.community_id
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
               f.submitter_pubkey, f.category, f.body, f.tags, f.status,
               f.event_created_at, f.received_at
        FROM product_feedback f
        LEFT JOIN communities c ON c.id = f.community_id
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
        status: row.try_get("status")?,
        event_created_at: row.try_get("event_created_at")?,
        received_at: row.try_get("received_at")?,
    })
}

impl Db {
    /// List reports for the deployment-global read-only admin plane.
    #[allow(clippy::too_many_arguments)]
    #[datastore_span(name = "admin_list_reports", system = "postgresql")]
    pub async fn admin_list_reports(
        &self,
        community_id: Option<Uuid>,
        status: Option<&str>,
        report_type: Option<&str>,
        target_kind: Option<&str>,
        after: Option<DateTime<Utc>>,
        before: Option<DateTime<Utc>>,
        cursor: Option<(DateTime<Utc>, Uuid)>,
        limit: i64,
    ) -> Result<Vec<AdminReport>> {
        list_reports(
            &self.pool,
            community_id,
            status,
            report_type,
            target_kind,
            after,
            before,
            cursor,
            limit,
        )
        .await
    }

    /// Fetch one report for the deployment-global read-only admin plane.
    #[datastore_span(name = "admin_get_report", system = "postgresql")]
    pub async fn admin_get_report(&self, id: Uuid) -> Result<Option<AdminReportDetail>> {
        get_report(&self.pool, id).await
    }

    /// List feedback for the deployment-global read-only admin plane.
    #[datastore_span(name = "admin_list_feedback", system = "postgresql")]
    pub async fn admin_list_feedback(&self, limit: i64) -> Result<Vec<AdminFeedback>> {
        list_feedback(&self.pool, limit).await
    }

    /// Fetch one feedback submission for the deployment-global admin plane.
    #[datastore_span(name = "admin_get_feedback", system = "postgresql")]
    pub async fn admin_get_feedback(&self, id: Uuid) -> Result<Option<AdminFeedback>> {
        get_feedback(&self.pool, id).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

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
        // relay_admin_actions FK-references (community_id, report_id), so clear
        // any enforcement/audit rows before the reports they point at. A no-op
        // for tests that never insert actions.
        sqlx::query("DELETE FROM relay_admin_actions WHERE report_community_id = $1")
            .bind(community_id)
            .execute(pool)
            .await
            .expect("delete admin action fixture");
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

    // ── activeAction LATERAL join ─────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    async fn insert_admin_action(
        pool: &PgPool,
        id: Uuid,
        community_id: Uuid,
        report_id: Uuid,
        action: &str,
        state: &str,
        created_at: DateTime<Utc>,
    ) {
        sqlx::query(
            r#"
            INSERT INTO relay_admin_actions (
                id, report_id, report_community_id, request_id, actor_pubkey,
                actor_role, action, state, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, 'operator', $6, $7, $8, $8)
            "#,
        )
        .bind(id)
        .bind(report_id)
        .bind(community_id)
        .bind(Uuid::new_v4())
        .bind(vec![2_u8; 32])
        .bind(action)
        .bind(state)
        .bind(created_at)
        .execute(pool)
        .await
        .expect("insert admin action");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn report_detail_surfaces_succeeded_enforcement_on_a_dismissed_reopened_report() {
        // Enforcement succeeded, report was later reopened and re-triaged to
        // `dismissed`. active_action_id is NULL, but the succeeded enforcement
        // row still matches `a.state='succeeded'` — the activeAction must surface
        // that DTO: a later dismissal does not un-happen the executed ban.
        let pool = setup_pool().await;
        let community_id = insert_community(&pool, "dismissed-after-enforce").await;
        let report_id = insert_pubkey_report(&pool, community_id).await;
        let action_id = Uuid::new_v4();
        insert_admin_action(
            &pool,
            action_id,
            community_id,
            report_id,
            "ban",
            "succeeded",
            Utc::now(),
        )
        .await;
        set_report_status(&pool, community_id, report_id, "dismissed").await;

        let detail = get_report(&pool, report_id)
            .await
            .expect("query report")
            .expect("report exists");
        assert_eq!(detail.report.status, "dismissed");
        let action = detail
            .active_action
            .expect("succeeded enforcement DTO survives dismissal");
        assert_eq!(action.id, action_id);
        assert_eq!(action.action, "ban");
        assert_eq!(action.status, "succeeded");

        delete_report_fixture(&pool, community_id).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn report_detail_active_action_breaks_equal_timestamp_ties_by_id_desc() {
        // Two succeeded enforcement rows (possible across reopen cycles) sharing
        // an identical created_at: the `a.id DESC` tiebreaker must pick the
        // greater id deterministically, never leave the choice to row order.
        let pool = setup_pool().await;
        let community_id = insert_community(&pool, "equal-ts-tiebreak").await;
        let report_id = insert_pubkey_report(&pool, community_id).await;
        let ts = Utc::now();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        insert_admin_action(&pool, id_a, community_id, report_id, "ban", "succeeded", ts).await;
        insert_admin_action(
            &pool,
            id_b,
            community_id,
            report_id,
            "kick",
            "succeeded",
            ts,
        )
        .await;
        set_report_status(&pool, community_id, report_id, "resolved").await;

        let detail = get_report(&pool, report_id)
            .await
            .expect("query report")
            .expect("report exists");
        let action = detail.active_action.expect("an action surfaces");
        assert_eq!(
            action.id,
            id_a.max(id_b),
            "equal timestamps must resolve to the greater id via a.id DESC"
        );

        delete_report_fixture(&pool, community_id).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn report_detail_active_action_excludes_reopen_audit_rows() {
        // A `reopen` audit row is written state='succeeded'. The enforcement DTO
        // join filters `action IN (delete,kick,ban,timeout)`, so a report whose
        // only relay_admin_actions row is a reopen audit must surface no action.
        let pool = setup_pool().await;
        let community_id = insert_community(&pool, "reopen-audit-excluded").await;
        let report_id = insert_pubkey_report(&pool, community_id).await;
        insert_admin_action(
            &pool,
            Uuid::new_v4(),
            community_id,
            report_id,
            "reopen",
            "succeeded",
            Utc::now(),
        )
        .await;

        let detail = get_report(&pool, report_id)
            .await
            .expect("query report")
            .expect("report exists");
        assert!(
            detail.active_action.is_none(),
            "reopen audit row must not surface as an enforcement action"
        );

        delete_report_fixture(&pool, community_id).await;
    }

    async fn set_report_status(pool: &PgPool, community_id: Uuid, report_id: Uuid, status: &str) {
        sqlx::query(
            "UPDATE moderation_reports SET status = $3 WHERE community_id = $1 AND id = $2",
        )
        .bind(community_id)
        .bind(report_id)
        .bind(status)
        .execute(pool)
        .await
        .expect("set report status");
    }

    // ── Feedback severed-provenance survival ──────────────────────────────────

    async fn insert_feedback(pool: &PgPool, community_id: Uuid, status: &str) -> Uuid {
        let id = Uuid::new_v4();
        let event_id: Vec<u8> = id
            .as_bytes()
            .iter()
            .chain(id.as_bytes().iter())
            .copied()
            .collect();
        sqlx::query(
            r#"
            INSERT INTO product_feedback (
                id, community_id, event_id, submitter_pubkey, category, body,
                tags, status, event_created_at, received_at
            ) VALUES ($1, $2, $3, $4, 'bug', 'reproduces on launch', '[]'::jsonb,
                      $5, now(), now())
            "#,
        )
        .bind(id)
        .bind(community_id)
        .bind(event_id)
        .bind(vec![9_u8; 32])
        .bind(status)
        .execute(pool)
        .await
        .expect("insert feedback");
        id
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn feedback_survives_community_purge_with_null_provenance_in_list_and_detail() {
        // purge_postgres severs tenant provenance without deleting the row:
        // `UPDATE product_feedback SET community_id = NULL`. The LEFT JOIN must
        // keep the row visible in both list and detail reads with null
        // community fields and its operator-managed status intact.
        let pool = setup_pool().await;
        let community_id = insert_community(&pool, "severed-feedback").await;
        let feedback_id = insert_feedback(&pool, community_id, "reviewed").await;

        // Sever provenance exactly as the community purge transaction does.
        sqlx::query("UPDATE product_feedback SET community_id = NULL WHERE community_id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("sever provenance");

        let detail = get_feedback(&pool, feedback_id)
            .await
            .expect("query feedback")
            .expect("severed feedback still readable in detail");
        assert_eq!(detail.id, feedback_id);
        assert!(
            detail.community_id.is_none(),
            "community_id severed to None"
        );
        assert!(
            detail.community_host.is_none(),
            "community_host has no row to join"
        );
        assert_eq!(detail.status, "reviewed", "operator status is retained");

        let listed = list_feedback(&pool, MAX_PAGE_SIZE)
            .await
            .expect("list feedback");
        let row = listed
            .iter()
            .find(|f| f.id == feedback_id)
            .expect("severed feedback still appears in the list read");
        assert!(row.community_id.is_none());
        assert!(row.community_host.is_none());
        assert_eq!(row.status, "reviewed");

        sqlx::query("DELETE FROM product_feedback WHERE id = $1")
            .bind(feedback_id)
            .execute(&pool)
            .await
            .expect("delete feedback fixture");
        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete community fixture");
    }

    // ── Wire contract: nullable action fields are required-nullable ───────────

    #[test]
    fn action_dto_emits_nullable_fields_as_json_null_not_absent() {
        // The desktop console types must be required-nullable, not optional:
        // `reason`, `expiresAt`, `errorMessage` are plain `Option<T>` with no
        // `skip_serializing_if`, so serde always emits the key (null when None).
        // This test pins that contract so the seam can't silently drift.
        let dto = AdminActionDto {
            id: Uuid::nil(),
            request_id: Uuid::nil(),
            actor_pubkey: hex::encode([0_u8; 32]),
            actor_role: "operator".to_string(),
            action: "ban".to_string(),
            status: "succeeded".to_string(),
            cancelled_by: None,
            reason: None,
            expires_at: None,
            error_message: None,
            created_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            updated_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
        };
        let value = serde_json::to_value(&dto).expect("serialize dto");
        let obj = value.as_object().expect("dto serializes to an object");
        for key in ["reason", "expiresAt", "errorMessage", "cancelledBy"] {
            assert_eq!(
                obj.get(key),
                Some(&serde_json::Value::Null),
                "{key} must be present and null, never absent"
            );
        }
        // Field names are camelCase on the wire.
        for key in [
            "requestId",
            "actorPubkey",
            "actorRole",
            "expiresAt",
            "errorMessage",
            "createdAt",
            "updatedAt",
        ] {
            assert!(obj.contains_key(key), "missing camelCase key {key}");
        }
    }
}
