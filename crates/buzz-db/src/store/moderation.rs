//! Community moderation persistence (Phase 1 contract).
//!
//! Backs the NIP-56 report queue (`moderation_reports`), ban/timeout state
//! (`community_bans`), and the moderation audit trail (`moderation_actions`)
//! from `migrations/0006_moderation.sql`.
//!
//! ## Tenant invariant
//! Every function takes a [`CommunityId`] and touches exactly one community's
//! rows. Report/ban targets are resolved by callers under the requesting
//! `TenantContext` **before** they reach this module — no function here may
//! perform a cross-community or global lookup (MOD invariants,
//! `docs/spec/MultiTenantRelay.tla`).
//!
//! Lane ownership: L1 (Max). Signatures below are the contract; changes go
//! through the integration thread.

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::error::Result;
use crate::{CommunityId, Db};

/// What a report points at. Exactly one target class per report row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportTarget {
    /// `e`-tag target: an event that must resolve inside the tenant.
    Event(Vec<u8>),
    /// `p`-only target: a community-local report about a pubkey.
    Pubkey(Vec<u8>),
    /// `x`-tag target: a media blob sha256, resolved via tenant-scoped refs.
    Blob(Vec<u8>),
}

/// Insert parameters for a new report row (from an accepted kind:1984 event).
#[derive(Debug, Clone)]
pub struct NewReport<'a> {
    /// Signed kind:1984 event id (32 bytes) — idempotency key per community.
    pub report_event_id: &'a [u8],
    /// Reporter pubkey bytes. Mod-queue-visible; never revealed to the author.
    pub reporter_pubkey: &'a [u8],
    /// Resolved (in-tenant) target.
    pub target: ReportTarget,
    /// Channel inferred from an in-tenant target event row, when resolvable.
    pub channel_id: Option<Uuid>,
    /// NIP-56 report type (already validated by ingest).
    pub report_type: &'a str,
    /// Reporter's optional free-text note.
    pub note: Option<&'a str>,
}

/// A report row as read back for the moderation queue.
#[derive(Debug, Clone)]
pub struct ReportRecord {
    /// Row id (unique within the community).
    pub id: Uuid,
    /// Signed kind:1984 event id.
    pub report_event_id: Vec<u8>,
    /// Reporter pubkey bytes.
    pub reporter_pubkey: Vec<u8>,
    /// Report target.
    pub target: ReportTarget,
    /// Inferred channel, if the target resolved to one.
    pub channel_id: Option<Uuid>,
    /// NIP-56 report type.
    pub report_type: String,
    /// Reporter's note.
    pub note: Option<String>,
    /// `open` | `resolved` | `dismissed` | `escalated`.
    pub status: String,
    /// Resolving moderator, once resolved.
    pub resolved_by: Option<Vec<u8>>,
    /// Resolution timestamp.
    pub resolved_at: Option<DateTime<Utc>>,
    /// `moderation_actions` row that resolved this report.
    pub action_id: Option<Uuid>,
    /// Report creation time.
    pub created_at: DateTime<Utc>,
}

/// Ban/timeout state for one member in one community.
#[derive(Debug, Clone)]
pub struct BanRecord {
    /// Member pubkey bytes.
    pub pubkey: Vec<u8>,
    /// Whether the member is currently banned (check `ban_expires_at`).
    pub banned: bool,
    /// Ban expiry; `None` while `banned` ⇒ permanent.
    pub ban_expires_at: Option<DateTime<Utc>>,
    /// Moderator-supplied ban reason (private).
    pub ban_reason: Option<String>,
    /// Write-block until this timestamp; `None` or past ⇒ not timed out.
    pub muted_until: Option<DateTime<Utc>>,
    /// Moderator-supplied timeout reason (private).
    pub mute_reason: Option<String>,
    /// Moderator who last modified this row.
    pub actor_pubkey: Vec<u8>,
    /// Last modification time.
    pub updated_at: DateTime<Utc>,
}

/// Audit action values accepted by the `moderation_actions.action` CHECK in
/// migration 0006. Keep this in lockstep with `migrations/0006_moderation.sql`.
pub const MODERATION_ACTION_CHECK_VOCAB: &[&str] = &[
    "delete_message",
    "kick",
    "ban",
    "unban",
    "timeout",
    "untimeout",
    "dismiss_report",
    "escalate",
    "resolve:delete",
    "resolve:kick",
    "resolve:ban",
    "resolve:timeout",
];

/// Insert parameters for a moderation audit row.
#[derive(Debug, Clone)]
pub struct NewAction<'a> {
    /// Acting moderator pubkey bytes.
    pub actor_pubkey: &'a [u8],
    /// `delete_message` | `kick` | `ban` | `unban` | `timeout` | `untimeout`
    /// | `dismiss_report` | `escalate` | `resolve:*` decision rows (DB CHECK-enforced).
    pub action: &'a str,
    /// Actioned member, when the action targets a pubkey.
    pub target_pubkey: Option<&'a [u8]>,
    /// Actioned event, when the action targets an event.
    pub target_event_id: Option<&'a [u8]>,
    /// Channel context, when known.
    pub channel_id: Option<Uuid>,
    /// Machine-readable rule/reason code.
    pub reason_code: Option<&'a str>,
    /// Sanitized reason, safe for the public tombstone.
    pub public_reason: Option<&'a str>,
    /// Mod-only context; never leaves the audit surface.
    pub private_reason: Option<&'a str>,
    /// NIP-OA matched principal (`self` | `owner`) for ban enforcement audit.
    pub matched_principal: Option<&'a str>,
    /// Deployment authority type. `'community'` for community-moderation paths;
    /// `'relay_operator'`/`'relay_moderator'` for HTTP admin paths.
    pub actor_authority: Option<&'a str>,
}

/// An audit row as read back for `buzz moderation audit`.
#[derive(Debug, Clone)]
pub struct ActionRecord {
    /// Row id.
    pub id: Uuid,
    /// Acting moderator pubkey bytes.
    pub actor_pubkey: Vec<u8>,
    /// Action name.
    pub action: String,
    /// Actioned member.
    pub target_pubkey: Option<Vec<u8>>,
    /// Actioned event.
    pub target_event_id: Option<Vec<u8>>,
    /// Channel context.
    pub channel_id: Option<Uuid>,
    /// Machine-readable rule/reason code.
    pub reason_code: Option<String>,
    /// Sanitized public reason.
    pub public_reason: Option<String>,
    /// Mod-only reason.
    pub private_reason: Option<String>,
    /// NIP-OA principal matched by enforcement, when relevant.
    pub matched_principal: Option<String>,
    /// Deployment authority type for HTTP-initiated actions.
    pub actor_authority: String,
    /// Action time.
    pub created_at: DateTime<Utc>,
}

/// Insert a new report row. Idempotent on `(community, report_event_id)`:
/// re-ingesting the same signed report is a no-op returning the existing id.
///
/// `illegal` reports auto-escalate: they land `status='escalated'` so the
/// platform operator backstop sees them without waiting for a community admin
/// to forward them (the severe class was never the community's to hold). Every
/// other category lands `open` for community triage. Auto-escalation only sets
/// the queue status; it emits no moderator decision, so an auto-escalated
/// report is indistinguishable downstream from an admin-escalated one — reopen
/// and listing key off `status`, never on how the report reached it.
pub async fn insert_report(
    pool: &PgPool,
    community: CommunityId,
    report: NewReport<'_>,
) -> Result<Uuid> {
    let (target_kind, target_event_id, target_pubkey, target_blob_sha256) = match &report.target {
        ReportTarget::Event(id) => ("event", Some(id.as_slice()), None, None),
        ReportTarget::Pubkey(pubkey) => ("pubkey", None, Some(pubkey.as_slice()), None),
        ReportTarget::Blob(sha256) => ("blob", None, None, Some(sha256.as_slice())),
    };

    let initial_status = if report.report_type == "illegal" {
        "escalated"
    } else {
        "open"
    };

    let row = sqlx::query(
        r#"
        INSERT INTO moderation_reports (
            community_id, report_event_id, reporter_pubkey, target_kind,
            target_event_id, target_pubkey, target_blob_sha256, channel_id,
            report_type, note, status
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (community_id, report_event_id) DO UPDATE SET
            report_event_id = EXCLUDED.report_event_id
        RETURNING id
        "#,
    )
    .bind(community.as_uuid())
    .bind(report.report_event_id)
    .bind(report.reporter_pubkey)
    .bind(target_kind)
    .bind(target_event_id)
    .bind(target_pubkey)
    .bind(target_blob_sha256)
    .bind(report.channel_id)
    .bind(report.report_type)
    .bind(report.note)
    .bind(initial_status)
    .fetch_one(pool)
    .await?;

    Ok(row.try_get("id")?)
}

/// List reports for the moderation queue, newest first.
/// `status = None` lists all; `Some("open")` etc. filters.
pub async fn list_reports(
    pool: &PgPool,
    community: CommunityId,
    status: Option<&str>,
    limit: i64,
) -> Result<Vec<ReportRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, report_event_id, reporter_pubkey, target_kind, target_event_id,
               target_pubkey, target_blob_sha256, channel_id, report_type, note,
               status, resolved_by, resolved_at, action_id, created_at
        FROM moderation_reports
        WHERE community_id = $1 AND ($2::text IS NULL OR status = $2)
        ORDER BY created_at DESC
        LIMIT $3
        "#,
    )
    .bind(community.as_uuid())
    .bind(status)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_report).collect()
}

/// Fetch one report by row id.
pub async fn get_report(
    pool: &PgPool,
    community: CommunityId,
    report_id: Uuid,
) -> Result<Option<ReportRecord>> {
    let row = sqlx::query(
        r#"
        SELECT id, report_event_id, reporter_pubkey, target_kind, target_event_id,
               target_pubkey, target_blob_sha256, channel_id, report_type, note,
               status, resolved_by, resolved_at, action_id, created_at
        FROM moderation_reports
        WHERE community_id = $1 AND id = $2
        "#,
    )
    .bind(community.as_uuid())
    .bind(report_id)
    .fetch_optional(pool)
    .await?;

    row.map(row_to_report).transpose()
}

/// Fetch one report by signed NIP-56 report event id.
pub async fn get_report_by_event(
    pool: &PgPool,
    community: CommunityId,
    report_event_id: &[u8],
) -> Result<Option<ReportRecord>> {
    let row = sqlx::query(
        r#"
        SELECT id, report_event_id, reporter_pubkey, target_kind, target_event_id,
               target_pubkey, target_blob_sha256, channel_id, report_type, note,
               status, resolved_by, resolved_at, action_id, created_at
        FROM moderation_reports
        WHERE community_id = $1 AND report_event_id = $2
        "#,
    )
    .bind(community.as_uuid())
    .bind(report_event_id)
    .fetch_optional(pool)
    .await?;

    row.map(row_to_report).transpose()
}

/// Mark a report resolved/dismissed/escalated, linking the audit action.
/// Returns `false` if the report was not found or already closed.
pub async fn resolve_report(
    pool: &PgPool,
    community: CommunityId,
    report_id: Uuid,
    status: &str,
    resolved_by: &[u8],
    action_id: Option<Uuid>,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE moderation_reports
        SET status = $3, resolved_by = $4, resolved_at = now(), action_id = $5
        WHERE community_id = $1 AND id = $2 AND status = 'open'
        "#,
    )
    .bind(community.as_uuid())
    .bind(report_id)
    .bind(status)
    .bind(resolved_by)
    .bind(action_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Upsert a ban: sets `banned = true` with optional expiry + reason.
pub async fn ban_member(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &[u8],
    actor: &[u8],
    reason: Option<&str>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO community_bans (
            community_id, pubkey, banned, ban_expires_at, ban_reason, actor_pubkey
        ) VALUES ($1, $2, true, $3, $4, $5)
        ON CONFLICT (community_id, pubkey) DO UPDATE SET
            banned = true,
            ban_expires_at = EXCLUDED.ban_expires_at,
            ban_reason = EXCLUDED.ban_reason,
            actor_pubkey = EXCLUDED.actor_pubkey,
            updated_at = now()
        "#,
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(expires_at)
    .bind(reason)
    .bind(actor)
    .execute(pool)
    .await?;

    Ok(())
}

/// Lift a ban. Returns `false` if the member was not banned.
pub async fn unban_member(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &[u8],
    actor: &[u8],
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE community_bans
        SET banned = false, ban_expires_at = NULL, ban_reason = NULL,
            actor_pubkey = $3, updated_at = now()
        WHERE community_id = $1 AND pubkey = $2 AND banned = true
        "#,
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(actor)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Upsert a timeout: sets `muted_until` + reason.
pub async fn timeout_member(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &[u8],
    actor: &[u8],
    muted_until: DateTime<Utc>,
    reason: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO community_bans (
            community_id, pubkey, muted_until, mute_reason, actor_pubkey
        ) VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (community_id, pubkey) DO UPDATE SET
            muted_until = EXCLUDED.muted_until,
            mute_reason = EXCLUDED.mute_reason,
            actor_pubkey = EXCLUDED.actor_pubkey,
            updated_at = now()
        "#,
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(muted_until)
    .bind(reason)
    .bind(actor)
    .execute(pool)
    .await?;

    Ok(())
}

/// Clear a timeout early. Returns `false` if the member was not timed out.
pub async fn untimeout_member(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &[u8],
    actor: &[u8],
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE community_bans
        SET muted_until = NULL, mute_reason = NULL,
            actor_pubkey = $3, updated_at = now()
        WHERE community_id = $1 AND pubkey = $2 AND muted_until > now()
        "#,
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(actor)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Restriction snapshot consumed by the auth-seam gate (L4) and write gates.
///
/// One cheap read per check: `banned` already accounts for expiry;
/// `muted_until` is returned raw so the caller can render the timestamp in
/// the `restricted:` message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestrictionState {
    /// Currently banned (row exists, `banned`, unexpired).
    pub banned: bool,
    /// Active timeout expiry, if in the future.
    pub muted_until: Option<DateTime<Utc>>,
}

/// Fetch the current restriction state for a pubkey in one community.
/// Missing row ⇒ `RestrictionState::default()` (unrestricted).
pub async fn restriction_state(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &[u8],
) -> Result<RestrictionState> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let row = sqlx::query(
        r#"
        SELECT
            (banned AND (ban_expires_at IS NULL OR ban_expires_at > now())) AS banned,
            CASE WHEN muted_until > now() THEN muted_until ELSE NULL END AS muted_until
        FROM community_bans
        WHERE community_id = $1 AND pubkey = $2
        "#,
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .fetch_optional(&mut *connection)
    .await?;

    match row {
        Some(row) => Ok(RestrictionState {
            banned: row.try_get("banned")?,
            muted_until: row.try_get("muted_until")?,
        }),
        None => Ok(RestrictionState::default()),
    }
}

/// Fetch the full ban/timeout row (moderation queue / audit views).
pub async fn get_ban(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &[u8],
) -> Result<Option<BanRecord>> {
    let row = sqlx::query(
        r#"
        SELECT pubkey,
               (banned AND (ban_expires_at IS NULL OR ban_expires_at > now())) AS banned,
               ban_expires_at, ban_reason, muted_until,
               mute_reason, actor_pubkey, updated_at
        FROM community_bans
        WHERE community_id = $1 AND pubkey = $2
        "#,
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .fetch_optional(pool)
    .await?;

    row.map(row_to_ban).transpose()
}

/// List currently-restricted members (active ban or timeout) for the queue.
pub async fn list_restricted(pool: &PgPool, community: CommunityId) -> Result<Vec<BanRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT pubkey,
               (banned AND (ban_expires_at IS NULL OR ban_expires_at > now())) AS banned,
               ban_expires_at, ban_reason, muted_until,
               mute_reason, actor_pubkey, updated_at
        FROM community_bans
        WHERE community_id = $1
          AND (
              (banned AND (ban_expires_at IS NULL OR ban_expires_at > now()))
              OR muted_until > now()
          )
        ORDER BY updated_at DESC
        "#,
    )
    .bind(community.as_uuid())
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_ban).collect()
}

/// Insert a moderation audit row, returning its id.
pub async fn insert_action(
    pool: &PgPool,
    community: CommunityId,
    action: NewAction<'_>,
) -> Result<Uuid> {
    let row = sqlx::query(
        r#"
        INSERT INTO moderation_actions (
            community_id, actor_pubkey, action, target_pubkey, target_event_id,
            channel_id, reason_code, public_reason, private_reason, matched_principal,
            actor_authority
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING id
        "#,
    )
    .bind(community.as_uuid())
    .bind(action.actor_pubkey)
    .bind(action.action)
    .bind(action.target_pubkey)
    .bind(action.target_event_id)
    .bind(action.channel_id)
    .bind(action.reason_code)
    .bind(action.public_reason)
    .bind(action.private_reason)
    .bind(action.matched_principal)
    .bind(action.actor_authority.unwrap_or("community"))
    .fetch_one(pool)
    .await?;

    Ok(row.try_get("id")?)
}

/// List audit rows, newest first (`buzz moderation audit`).
pub async fn list_actions(
    pool: &PgPool,
    community: CommunityId,
    limit: i64,
) -> Result<Vec<ActionRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, actor_pubkey, action, target_pubkey, target_event_id, channel_id,
               reason_code, public_reason, private_reason, matched_principal,
               actor_authority, created_at
        FROM moderation_actions
        WHERE community_id = $1
        ORDER BY created_at DESC
        LIMIT $2
        "#,
    )
    .bind(community.as_uuid())
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_action).collect()
}

fn row_to_report(row: sqlx::postgres::PgRow) -> Result<ReportRecord> {
    let target_kind: String = row.try_get("target_kind")?;
    let target = match target_kind.as_str() {
        "event" => ReportTarget::Event(row.try_get("target_event_id")?),
        "pubkey" => ReportTarget::Pubkey(row.try_get("target_pubkey")?),
        "blob" => ReportTarget::Blob(row.try_get("target_blob_sha256")?),
        other => {
            return Err(crate::error::DbError::InvalidData(format!(
                "invalid report target_kind: {other}"
            )))
        }
    };

    Ok(ReportRecord {
        id: row.try_get("id")?,
        report_event_id: row.try_get("report_event_id")?,
        reporter_pubkey: row.try_get("reporter_pubkey")?,
        target,
        channel_id: row.try_get("channel_id")?,
        report_type: row.try_get("report_type")?,
        note: row.try_get("note")?,
        status: row.try_get("status")?,
        resolved_by: row.try_get("resolved_by")?,
        resolved_at: row.try_get("resolved_at")?,
        action_id: row.try_get("action_id")?,
        created_at: row.try_get("created_at")?,
    })
}

fn row_to_ban(row: sqlx::postgres::PgRow) -> Result<BanRecord> {
    Ok(BanRecord {
        pubkey: row.try_get("pubkey")?,
        banned: row.try_get("banned")?,
        ban_expires_at: row.try_get("ban_expires_at")?,
        ban_reason: row.try_get("ban_reason")?,
        muted_until: row.try_get("muted_until")?,
        mute_reason: row.try_get("mute_reason")?,
        actor_pubkey: row.try_get("actor_pubkey")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn row_to_action(row: sqlx::postgres::PgRow) -> Result<ActionRecord> {
    Ok(ActionRecord {
        id: row.try_get("id")?,
        actor_pubkey: row.try_get("actor_pubkey")?,
        action: row.try_get("action")?,
        target_pubkey: row.try_get("target_pubkey")?,
        target_event_id: row.try_get("target_event_id")?,
        channel_id: row.try_get("channel_id")?,
        reason_code: row.try_get("reason_code")?,
        public_reason: row.try_get("public_reason")?,
        private_reason: row.try_get("private_reason")?,
        matched_principal: row.try_get("matched_principal")?,
        actor_authority: row.try_get("actor_authority")?,
        created_at: row.try_get("created_at")?,
    })
}

impl Db {
    /// Insert a tenant-scoped NIP-56 report row, idempotent by report event id.
    #[datastore_span(name = "insert_moderation_report", system = "postgresql")]
    pub async fn insert_moderation_report(
        &self,
        community: CommunityId,
        report: NewReport<'_>,
    ) -> Result<Uuid> {
        insert_report(&self.pool, community, report).await
    }

    /// List moderation reports for a community, newest first.
    #[datastore_span(name = "list_moderation_reports", system = "postgresql")]
    pub async fn list_moderation_reports(
        &self,
        community: CommunityId,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ReportRecord>> {
        list_reports(&self.pool, community, status, limit).await
    }

    /// Fetch one moderation report by row id.
    #[datastore_span(name = "get_moderation_report", system = "postgresql")]
    pub async fn get_moderation_report(
        &self,
        community: CommunityId,
        report_id: Uuid,
    ) -> Result<Option<ReportRecord>> {
        get_report(&self.pool, community, report_id).await
    }

    /// Fetch one moderation report by signed NIP-56 report event id.
    #[datastore_span(name = "get_moderation_report_by_event", system = "postgresql")]
    pub async fn get_moderation_report_by_event(
        &self,
        community: CommunityId,
        report_event_id: &[u8],
    ) -> Result<Option<ReportRecord>> {
        get_report_by_event(&self.pool, community, report_event_id).await
    }

    /// Resolve, dismiss, or escalate an open moderation report.
    #[datastore_span(name = "resolve_moderation_report", system = "postgresql")]
    pub async fn resolve_moderation_report(
        &self,
        community: CommunityId,
        report_id: Uuid,
        status: &str,
        resolved_by: &[u8],
        action_id: Option<Uuid>,
    ) -> Result<bool> {
        resolve_report(
            &self.pool,
            community,
            report_id,
            status,
            resolved_by,
            action_id,
        )
        .await
    }

    /// Upsert a community ban for a member pubkey.
    #[datastore_span(name = "ban_community_member", system = "postgresql")]
    pub async fn ban_community_member(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        actor: &[u8],
        reason: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        ban_member(&self.pool, community, pubkey, actor, reason, expires_at).await
    }

    /// Lift a community ban for a member pubkey.
    #[datastore_span(name = "unban_community_member", system = "postgresql")]
    pub async fn unban_community_member(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        actor: &[u8],
    ) -> Result<bool> {
        unban_member(&self.pool, community, pubkey, actor).await
    }

    /// Upsert a community timeout/write-block for a member pubkey.
    #[datastore_span(name = "timeout_community_member", system = "postgresql")]
    pub async fn timeout_community_member(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        actor: &[u8],
        muted_until: DateTime<Utc>,
        reason: Option<&str>,
    ) -> Result<()> {
        timeout_member(&self.pool, community, pubkey, actor, muted_until, reason).await
    }

    /// Clear a community timeout/write-block for a member pubkey.
    #[datastore_span(name = "untimeout_community_member", system = "postgresql")]
    pub async fn untimeout_community_member(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        actor: &[u8],
    ) -> Result<bool> {
        untimeout_member(&self.pool, community, pubkey, actor).await
    }

    /// Fetch the active ban/timeout restriction state for enforcement hot paths.
    #[datastore_span(name = "moderation_restriction_state", system = "postgresql")]
    pub async fn moderation_restriction_state(
        &self,
        community: CommunityId,
        pubkey: &[u8],
    ) -> Result<RestrictionState> {
        restriction_state(&self.pool, community, pubkey).await
    }

    /// Fetch the full ban/timeout row for a member pubkey.
    #[datastore_span(name = "get_community_ban", system = "postgresql")]
    pub async fn get_community_ban(
        &self,
        community: CommunityId,
        pubkey: &[u8],
    ) -> Result<Option<BanRecord>> {
        get_ban(&self.pool, community, pubkey).await
    }

    /// List currently restricted members in a community.
    #[datastore_span(name = "list_community_restrictions", system = "postgresql")]
    pub async fn list_community_restrictions(
        &self,
        community: CommunityId,
    ) -> Result<Vec<BanRecord>> {
        list_restricted(&self.pool, community).await
    }

    /// Insert a moderation audit action row.
    #[datastore_span(name = "insert_moderation_action", system = "postgresql")]
    pub async fn insert_moderation_action(
        &self,
        community: CommunityId,
        action: NewAction<'_>,
    ) -> Result<Uuid> {
        insert_action(&self.pool, community, action).await
    }

    /// List moderation audit action rows, newest first.
    #[datastore_span(name = "list_moderation_actions", system = "postgresql")]
    pub async fn list_moderation_actions(
        &self,
        community: CommunityId,
        limit: i64,
    ) -> Result<Vec<ActionRecord>> {
        list_actions(&self.pool, community, limit).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use chrono::Duration;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    async fn make_test_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        let host = format!("moderation-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    fn random_32() -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32);
        bytes.extend_from_slice(Uuid::new_v4().as_bytes());
        bytes.extend_from_slice(Uuid::new_v4().as_bytes());
        bytes
    }

    fn new_report<'a>(
        report_event_id: &'a [u8],
        reporter_pubkey: &'a [u8],
        target_event_id: &'a [u8],
        note: Option<&'a str>,
    ) -> NewReport<'a> {
        new_report_typed(
            report_event_id,
            reporter_pubkey,
            target_event_id,
            "spam",
            note,
        )
    }

    fn new_report_typed<'a>(
        report_event_id: &'a [u8],
        reporter_pubkey: &'a [u8],
        target_event_id: &'a [u8],
        report_type: &'a str,
        note: Option<&'a str>,
    ) -> NewReport<'a> {
        NewReport {
            report_event_id,
            reporter_pubkey,
            target: ReportTarget::Event(target_event_id.to_vec()),
            channel_id: None,
            report_type,
            note,
        }
    }

    /// Community moderation restrictions are tenant-scoped. This guards the same
    /// mutation class as the TLA⁺ tenant-fence invariant: a ban in community A
    /// must not restrict the same pubkey in community B, through either the hot
    /// `restriction_state` read or the queue-facing `list_restricted` read.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn restrictions_are_confined_to_their_community() {
        let pool = setup_pool().await;
        let community_a = make_test_community(&pool).await;
        let community_b = make_test_community(&pool).await;
        let pubkey = random_32();
        let actor = random_32();

        ban_member(
            &pool,
            community_a,
            &pubkey,
            &actor,
            Some("tenant fence test"),
            None,
        )
        .await
        .expect("ban in community A");

        let state_a = restriction_state(&pool, community_a, &pubkey)
            .await
            .expect("restriction_state A");
        assert!(state_a.banned, "pubkey must be banned in community A");

        let state_b = restriction_state(&pool, community_b, &pubkey)
            .await
            .expect("restriction_state B");
        assert!(
            !state_b.banned && state_b.muted_until.is_none(),
            "ban in A must not restrict the same pubkey in community B"
        );

        let restricted_a = list_restricted(&pool, community_a)
            .await
            .expect("list restricted A");
        assert!(
            restricted_a.iter().any(|row| row.pubkey == pubkey),
            "community A restricted list must include the banned pubkey"
        );

        let restricted_b = list_restricted(&pool, community_b)
            .await
            .expect("list restricted B");
        assert!(
            restricted_b.iter().all(|row| row.pubkey != pubkey),
            "community B restricted list must not include community A's ban"
        );
    }

    /// Ban expiry is evaluated in SQL, while a live timeout on the same row keeps
    /// the member restricted for writes. This protects the one-row/two-restriction
    /// shape used by L4's auth and ingest gates.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn expired_ban_does_not_hide_active_timeout() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let pubkey = random_32();
        let actor = random_32();

        ban_member(
            &pool,
            community,
            &pubkey,
            &actor,
            Some("expired ban"),
            Some(Utc::now() - Duration::hours(1)),
        )
        .await
        .expect("insert expired ban");
        timeout_member(
            &pool,
            community,
            &pubkey,
            &actor,
            Utc::now() + Duration::hours(1),
            Some("active timeout"),
        )
        .await
        .expect("insert active timeout");

        let state = restriction_state(&pool, community, &pubkey)
            .await
            .expect("restriction_state");
        assert!(!state.banned, "expired ban must evaluate inactive");
        assert!(
            state.muted_until.is_some(),
            "active timeout must survive an expired ban on the same row"
        );

        let ban = get_ban(&pool, community, &pubkey)
            .await
            .expect("get ban")
            .expect("restriction row exists");
        assert!(
            !ban.banned,
            "get_ban must also evaluate expired ban inactive"
        );
        assert!(ban.muted_until.is_some(), "get_ban must preserve timeout");

        let restricted = list_restricted(&pool, community)
            .await
            .expect("list restricted");
        let listed = restricted
            .iter()
            .find(|row| row.pubkey == pubkey)
            .expect("timeout-only row remains listed");
        assert!(
            !listed.banned,
            "list_restricted reports expired ban inactive"
        );
        assert!(
            listed.muted_until.is_some(),
            "list_restricted preserves timeout"
        );
    }

    /// Re-ingesting the same signed report is idempotent by event id and must not
    /// reopen or otherwise reset a report that a moderator already resolved.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn report_reingest_returns_same_id_and_preserves_resolution() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let report_event_id = random_32();
        let reporter = random_32();
        let target_event_id = random_32();
        let resolver = random_32();

        let report = new_report(&report_event_id, &reporter, &target_event_id, Some("first"));
        let first_id = insert_report(&pool, community, report)
            .await
            .expect("insert report");

        assert!(
            resolve_report(&pool, community, first_id, "resolved", &resolver, None)
                .await
                .expect("resolve report"),
            "first resolve should close the report"
        );

        let duplicate = new_report(&report_event_id, &reporter, &target_event_id, Some("retry"));
        let second_id = insert_report(&pool, community, duplicate)
            .await
            .expect("re-ingest report");
        assert_eq!(first_id, second_id, "re-ingest must return the same row id");

        let row = get_report(&pool, community, first_id)
            .await
            .expect("get report")
            .expect("report exists");
        let row_by_event = get_report_by_event(&pool, community, &report_event_id)
            .await
            .expect("get report by event id")
            .expect("report exists by event id");
        assert_eq!(
            row_by_event.id, first_id,
            "report event id lookup must return the same row"
        );
        assert_eq!(
            row.status, "resolved",
            "re-ingest must not reopen the report"
        );
        assert!(
            row.resolved_at.is_some(),
            "resolution timestamp is preserved"
        );
        assert_eq!(
            row.resolved_by.as_deref(),
            Some(resolver.as_slice()),
            "resolving moderator is preserved"
        );
    }

    /// `resolve_report` is a guarded transition out of `open`; a second resolve
    /// on a closed report must be a no-op and return `false`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn resolve_report_returns_false_after_report_is_closed() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let report_event_id = random_32();
        let reporter = random_32();
        let target_event_id = random_32();
        let resolver = random_32();

        let report_id = insert_report(
            &pool,
            community,
            new_report(&report_event_id, &reporter, &target_event_id, None),
        )
        .await
        .expect("insert report");

        assert!(
            resolve_report(&pool, community, report_id, "dismissed", &resolver, None)
                .await
                .expect("first resolve"),
            "first resolve should update the open report"
        );
        assert!(
            !resolve_report(&pool, community, report_id, "resolved", &resolver, None)
                .await
                .expect("second resolve"),
            "second resolve should return false once the report is closed"
        );
    }

    /// `illegal` reports are the severe class the vision doc says was never the
    /// community's to hold: they auto-escalate to the platform backstop at
    /// ingestion rather than waiting for a community admin to forward them.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn illegal_report_auto_escalates_at_ingest() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let report_event_id = random_32();
        let reporter = random_32();
        let target_event_id = random_32();

        let report_id = insert_report(
            &pool,
            community,
            new_report_typed(
                &report_event_id,
                &reporter,
                &target_event_id,
                "illegal",
                Some("illegal content"),
            ),
        )
        .await
        .expect("insert illegal report");

        let row = get_report(&pool, community, report_id)
            .await
            .expect("get report")
            .expect("report exists");
        assert_eq!(
            row.status, "escalated",
            "an illegal report must land escalated"
        );
        // Auto-escalation is a queue-status decision, not a moderator action: no
        // resolver is stamped, so downstream reads cannot infer a human forwarded it.
        assert!(
            row.resolved_by.is_none() && row.resolved_at.is_none(),
            "auto-escalation must not stamp a resolver"
        );
    }

    /// Every non-`illegal` category still lands `open` for community triage; the
    /// auto-escalation branch must not widen to the ordinary report flow.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn non_illegal_report_lands_open_at_ingest() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;

        for report_type in ["spam", "nudity", "malware", "profanity", "other"] {
            let report_event_id = random_32();
            let reporter = random_32();
            let target_event_id = random_32();

            let report_id = insert_report(
                &pool,
                community,
                new_report_typed(
                    &report_event_id,
                    &reporter,
                    &target_event_id,
                    report_type,
                    None,
                ),
            )
            .await
            .expect("insert report");

            let row = get_report(&pool, community, report_id)
                .await
                .expect("get report")
                .expect("report exists");
            assert_eq!(
                row.status, "open",
                "a {report_type} report must land open, not escalated"
            );
        }
    }
}
