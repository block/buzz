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

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Row as _, Transaction};
use uuid::Uuid;

use crate::error::Result;
use crate::CommunityId;

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
    /// Action time.
    pub created_at: DateTime<Utc>,
}

/// Insert a new report row. Idempotent on `(community, report_event_id)`:
/// re-ingesting the same signed report is a no-op returning the existing id.
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

    let row = sqlx::query(
        r#"
        INSERT INTO moderation_reports (
            community_id, report_event_id, reporter_pubkey, target_kind,
            target_event_id, target_pubkey, target_blob_sha256, channel_id,
            report_type, note
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
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
    .fetch_one(pool)
    .await?;

    Ok(row.try_get("id")?)
}

/// Insert a report inside a caller-owned transaction.
///
/// Protected callers use this variant so the report and its canonical
/// authorization receipt can share one commit boundary.
pub async fn insert_report_tx(
    transaction: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    report: NewReport<'_>,
) -> Result<Uuid> {
    let (target_kind, target_event_id, target_pubkey, target_blob_sha256) = match &report.target {
        ReportTarget::Event(id) => ("event", Some(id.as_slice()), None, None),
        ReportTarget::Pubkey(pubkey) => ("pubkey", None, Some(pubkey.as_slice()), None),
        ReportTarget::Blob(sha256) => ("blob", None, None, Some(sha256.as_slice())),
    };

    let row = sqlx::query(
        r#"
        INSERT INTO moderation_reports (
            community_id, report_event_id, reporter_pubkey, target_kind,
            target_event_id, target_pubkey, target_blob_sha256, channel_id,
            report_type, note
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
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
    .fetch_one(&mut **transaction)
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

/// Lock and fetch a report by signed event id inside a caller-owned
/// transaction.
pub async fn get_report_by_event_tx(
    transaction: &mut Transaction<'_, Postgres>,
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
        FOR UPDATE
        "#,
    )
    .bind(community.as_uuid())
    .bind(report_event_id)
    .fetch_optional(&mut **transaction)
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

/// Resolve a report inside a caller-owned transaction.
pub async fn resolve_report_tx(
    transaction: &mut Transaction<'_, Postgres>,
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
    .execute(&mut **transaction)
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

/// Upsert a ban inside a caller-owned transaction.
pub async fn ban_member_tx(
    transaction: &mut Transaction<'_, Postgres>,
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
    .execute(&mut **transaction)
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

/// Lift a ban inside a caller-owned transaction.
pub async fn unban_member_tx(
    transaction: &mut Transaction<'_, Postgres>,
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
    .execute(&mut **transaction)
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

/// Upsert a timeout inside a caller-owned transaction.
pub async fn timeout_member_tx(
    transaction: &mut Transaction<'_, Postgres>,
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
    .execute(&mut **transaction)
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

/// Clear a timeout inside a caller-owned transaction.
pub async fn untimeout_member_tx(
    transaction: &mut Transaction<'_, Postgres>,
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
    .execute(&mut **transaction)
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
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => Ok(RestrictionState {
            banned: row.try_get("banned")?,
            muted_until: row.try_get("muted_until")?,
        }),
        None => Ok(RestrictionState::default()),
    }
}

/// Fetch and share-lock current restriction state inside a caller-owned
/// transaction.
///
/// The table lock closes the absent-row race: an actor with no restriction row
/// cannot be concurrently banned between this recheck and the protected
/// moderation commit.
pub async fn restriction_state_tx(
    transaction: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    pubkey: &[u8],
) -> Result<RestrictionState> {
    sqlx::query("LOCK TABLE community_bans IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut **transaction)
        .await?;
    let row = sqlx::query(
        r#"
        SELECT
            (banned AND (ban_expires_at IS NULL OR ban_expires_at > now())) AS banned,
            CASE WHEN muted_until > now() THEN muted_until ELSE NULL END AS muted_until
        FROM community_bans
        WHERE community_id = $1 AND pubkey = $2
        FOR SHARE
        "#,
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .fetch_optional(&mut **transaction)
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
            channel_id, reason_code, public_reason, private_reason, matched_principal
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
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
    .fetch_one(pool)
    .await?;

    Ok(row.try_get("id")?)
}

/// Insert a moderation audit row inside a caller-owned transaction.
pub async fn insert_action_tx(
    transaction: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    action: NewAction<'_>,
) -> Result<Uuid> {
    let row = sqlx::query(
        r#"
        INSERT INTO moderation_actions (
            community_id, actor_pubkey, action, target_pubkey, target_event_id,
            channel_id, reason_code, public_reason, private_reason, matched_principal
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
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
    .fetch_one(&mut **transaction)
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
               reason_code, public_reason, private_reason, matched_principal, created_at
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
        created_at: row.try_get("created_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

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
        NewReport {
            report_event_id,
            reporter_pubkey,
            target: ReportTarget::Event(target_event_id.to_vec()),
            channel_id: None,
            report_type: "spam",
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

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn moderation_state_and_audit_rollback_together() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let target = random_32();
        let actor = random_32();
        let mut transaction = pool.begin().await.expect("begin transaction");

        ban_member_tx(
            &mut transaction,
            community,
            &target,
            &actor,
            Some("rollback"),
            None,
        )
        .await
        .expect("stage ban");
        insert_action_tx(
            &mut transaction,
            community,
            NewAction {
                actor_pubkey: &actor,
                action: "ban",
                target_pubkey: Some(&target),
                target_event_id: None,
                channel_id: None,
                reason_code: None,
                public_reason: Some("rollback"),
                private_reason: None,
                matched_principal: None,
            },
        )
        .await
        .expect("stage audit");
        transaction.rollback().await.expect("roll back transaction");

        let restriction = restriction_state(&pool, community, &target)
            .await
            .expect("read restriction");
        assert!(!restriction.banned, "rolled-back ban must not be visible");
        assert!(
            list_actions(&pool, community, 10)
                .await
                .expect("list actions")
                .iter()
                .all(|row| row.target_pubkey.as_deref() != Some(target.as_slice())),
            "rolled-back audit row must not be visible"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn moderation_effect_receipt_claim_and_audit_share_one_rollback_boundary() {
        let pool = setup_pool().await;
        crate::migration::run_migrations(&pool)
            .await
            .expect("apply protected-authority migration");
        let community = make_test_community(&pool).await;
        let actor = random_32();
        let retain_until = Utc::now() + Duration::minutes(5);

        // An application-effect error after state, receipt, and claim staging
        // leaves none of them committed.
        let effect_operation = Uuid::new_v4();
        let effect_request = random_32();
        let effect_claim = random_32();
        let effect_target = random_32();
        let mut effect_tx = pool.begin().await.expect("begin effect failure");
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 11, $4, 1, $5)",
        )
        .bind(community.as_uuid())
        .bind(effect_operation)
        .bind(&effect_request)
        .bind(&actor)
        .bind(random_32())
        .execute(&mut *effect_tx)
        .await
        .expect("stage effect receipt");
        sqlx::query(
            "INSERT INTO authorization_proxy_nonce_claims \
             (authorization_domain, claim_kind, claim_key, retain_until) \
             VALUES ($1, 1, $2, $3)",
        )
        .bind(community.as_uuid())
        .bind(&effect_claim)
        .bind(retain_until)
        .execute(&mut *effect_tx)
        .await
        .expect("stage effect replay claim");
        ban_member_tx(
            &mut effect_tx,
            community,
            &effect_target,
            &actor,
            Some("effect failure"),
            None,
        )
        .await
        .expect("stage effect state");
        let failed_effect = insert_action_tx(
            &mut effect_tx,
            community,
            NewAction {
                actor_pubkey: &actor,
                action: "not-a-moderation-action",
                target_pubkey: Some(&effect_target),
                target_event_id: None,
                channel_id: None,
                reason_code: None,
                public_reason: None,
                private_reason: None,
                matched_principal: None,
            },
        )
        .await;
        assert!(failed_effect.is_err(), "invalid effect must abort");
        effect_tx.rollback().await.expect("rollback failed effect");

        let effect_receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_operation_receipts \
             WHERE community_id = $1 AND operation_id = $2",
        )
        .bind(community.as_uuid())
        .bind(effect_operation)
        .fetch_one(&pool)
        .await
        .expect("count effect receipts");
        let effect_claims: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_proxy_nonce_claims \
             WHERE authorization_domain = $1 AND claim_key = $2",
        )
        .bind(community.as_uuid())
        .bind(&effect_claim)
        .fetch_one(&pool)
        .await
        .expect("count effect claims");
        assert_eq!((effect_receipts, effect_claims), (0, 0));
        assert!(
            !restriction_state(&pool, community, &effect_target)
                .await
                .expect("read effect restriction")
                .banned
        );

        // Exhaust the immutable authorization-audit budget, then prove the
        // failed canonical audit append rolls moderation, receipt, and replay
        // state back together.
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, 1, 4, 4)",
        )
        .bind(community.as_uuid())
        .execute(&pool)
        .await
        .expect("install one-event audit capacity");
        let fill_operation = Uuid::new_v4();
        let fill_request = random_32();
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 11, $4, 1, $5)",
        )
        .bind(community.as_uuid())
        .bind(fill_operation)
        .bind(&fill_request)
        .bind(&actor)
        .bind(random_32())
        .execute(&pool)
        .await
        .expect("insert capacity-fill receipt");
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, actor_kind, \
              actor_fingerprint, operation_id, request_fingerprint, correlation_id, \
              attempt_id, occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 10, 1, 1, 1, $3, $4, $5, $6, $7, \
                     transaction_timestamp(), $8, $9)",
        )
        .bind(community.as_uuid())
        .bind(Uuid::new_v4())
        .bind(&actor)
        .bind(fill_operation)
        .bind(&fill_request)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(vec![1_u8; 4])
        .bind(random_32())
        .execute(&pool)
        .await
        .expect("fill audit capacity");

        let audit_operation = Uuid::new_v4();
        let audit_request = random_32();
        let audit_claim = random_32();
        let audit_target = random_32();
        let mut audit_tx = pool.begin().await.expect("begin exhausted audit effect");
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 11, $4, 1, $5)",
        )
        .bind(community.as_uuid())
        .bind(audit_operation)
        .bind(&audit_request)
        .bind(&actor)
        .bind(random_32())
        .execute(&mut *audit_tx)
        .await
        .expect("stage exhausted receipt");
        sqlx::query(
            "INSERT INTO authorization_proxy_nonce_claims \
             (authorization_domain, claim_kind, claim_key, retain_until) \
             VALUES ($1, 1, $2, $3)",
        )
        .bind(community.as_uuid())
        .bind(&audit_claim)
        .bind(retain_until)
        .execute(&mut *audit_tx)
        .await
        .expect("stage exhausted replay claim");
        ban_member_tx(
            &mut audit_tx,
            community,
            &audit_target,
            &actor,
            Some("audit exhausted"),
            None,
        )
        .await
        .expect("stage exhausted state");
        insert_action_tx(
            &mut audit_tx,
            community,
            NewAction {
                actor_pubkey: &actor,
                action: "ban",
                target_pubkey: Some(&audit_target),
                target_event_id: None,
                channel_id: None,
                reason_code: None,
                public_reason: Some("audit exhausted"),
                private_reason: None,
                matched_principal: None,
            },
        )
        .await
        .expect("stage moderation audit");
        let exhausted = sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, actor_kind, \
              actor_fingerprint, operation_id, request_fingerprint, correlation_id, \
              attempt_id, occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 10, 1, 1, 1, $3, $4, $5, $6, $7, \
                     transaction_timestamp(), $8, $9)",
        )
        .bind(community.as_uuid())
        .bind(Uuid::new_v4())
        .bind(&actor)
        .bind(audit_operation)
        .bind(&audit_request)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(vec![2_u8; 1])
        .bind(random_32())
        .execute(&mut *audit_tx)
        .await;
        assert!(exhausted.is_err(), "audit capacity must fail closed");
        audit_tx
            .rollback()
            .await
            .expect("rollback exhausted effect");
        assert!(
            !restriction_state(&pool, community, &audit_target)
                .await
                .expect("read exhausted restriction")
                .banned
        );
        let exhausted_receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_operation_receipts \
             WHERE community_id = $1 AND operation_id = $2",
        )
        .bind(community.as_uuid())
        .bind(audit_operation)
        .fetch_one(&pool)
        .await
        .expect("count exhausted receipt");
        let exhausted_claims: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_proxy_nonce_claims \
             WHERE authorization_domain = $1 AND claim_key = $2",
        )
        .bind(community.as_uuid())
        .bind(&audit_claim)
        .fetch_one(&pool)
        .await
        .expect("count exhausted claim");
        assert_eq!((exhausted_receipts, exhausted_claims), (0, 0));

        // A deferred commit-time target-link failure has the same rollback
        // semantics; no post-commit action may run for this branch.
        let commit_operation = Uuid::new_v4();
        let commit_request = random_32();
        let commit_claim = random_32();
        let commit_target = random_32();
        let commit_result = random_32();
        let mut commit_tx = pool.begin().await.expect("begin commit failure");
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 11, $4, 1, $5)",
        )
        .bind(community.as_uuid())
        .bind(commit_operation)
        .bind(&commit_request)
        .bind(&actor)
        .bind(&commit_result)
        .execute(&mut *commit_tx)
        .await
        .expect("stage commit-failure receipt");
        sqlx::query(
            "INSERT INTO authorization_proxy_nonce_claims \
             (authorization_domain, claim_kind, claim_key, retain_until) \
             VALUES ($1, 1, $2, $3)",
        )
        .bind(community.as_uuid())
        .bind(&commit_claim)
        .bind(retain_until)
        .execute(&mut *commit_tx)
        .await
        .expect("stage commit-failure claim");
        ban_member_tx(
            &mut commit_tx,
            community,
            &commit_target,
            &actor,
            Some("commit failure"),
            None,
        )
        .await
        .expect("stage commit-failure state");
        insert_action_tx(
            &mut commit_tx,
            community,
            NewAction {
                actor_pubkey: &actor,
                action: "ban",
                target_pubkey: Some(&commit_target),
                target_event_id: None,
                channel_id: None,
                reason_code: None,
                public_reason: Some("commit failure"),
                private_reason: None,
                matched_principal: None,
            },
        )
        .await
        .expect("stage commit-failure moderation audit");
        sqlx::query(
            "INSERT INTO protected_publication_projection_outbox \
             (community_id, operation_id, request_fingerprint, \
              publication_result_digest, object_kind, object_key, application_type, \
              application_version, application_effect_digest, projection_kind, \
              projection_key, staged_object_key, payload_digest) \
             VALUES ($1, $2, $3, $4, 3, $5, decode(repeat('11', 32), 'hex'), \
                     1, decode(repeat('12', 32), 'hex'), 3, $6, $7, $8)",
        )
        .bind(community.as_uuid())
        .bind(commit_operation)
        .bind(&commit_request)
        .bind(&commit_result)
        .bind(random_32())
        .bind("repos/commit-failure/pointer")
        .bind("manifests/commit-failure")
        .bind(random_32())
        .execute(&mut *commit_tx)
        .await
        .expect("stage deferred target-link failure");
        assert!(
            commit_tx.commit().await.is_err(),
            "deferred target link must fail commit"
        );
        assert!(
            !restriction_state(&pool, community, &commit_target)
                .await
                .expect("read commit-failure restriction")
                .banned
        );
        let commit_receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_operation_receipts \
             WHERE community_id = $1 AND operation_id = $2",
        )
        .bind(community.as_uuid())
        .bind(commit_operation)
        .fetch_one(&pool)
        .await
        .expect("count commit-failure receipt");
        let commit_claims: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_proxy_nonce_claims \
             WHERE authorization_domain = $1 AND claim_key = $2",
        )
        .bind(community.as_uuid())
        .bind(&commit_claim)
        .fetch_one(&pool)
        .await
        .expect("count commit-failure claim");
        assert_eq!((commit_receipts, commit_claims), (0, 0));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_report_resolution_commits_one_audit_row() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let report_event_id = random_32();
        let reporter = random_32();
        let target_event_id = random_32();
        let report_id = insert_report(
            &pool,
            community,
            new_report(&report_event_id, &reporter, &target_event_id, None),
        )
        .await
        .expect("insert report");

        let resolve_once = |pool: PgPool, actor: Vec<u8>| {
            let report_event_id = report_event_id.clone();
            let target_event_id = target_event_id.clone();
            async move {
                let mut transaction = pool.begin().await.expect("begin resolver");
                let report = get_report_by_event_tx(&mut transaction, community, &report_event_id)
                    .await
                    .expect("lock report")
                    .expect("report exists");
                if report.status != "open" {
                    transaction.rollback().await.expect("rollback loser");
                    return false;
                }
                let action_id = insert_action_tx(
                    &mut transaction,
                    community,
                    NewAction {
                        actor_pubkey: &actor,
                        action: "dismiss_report",
                        target_pubkey: None,
                        target_event_id: Some(&target_event_id),
                        channel_id: None,
                        reason_code: None,
                        public_reason: None,
                        private_reason: None,
                        matched_principal: None,
                    },
                )
                .await
                .expect("insert resolution audit");
                let resolved = resolve_report_tx(
                    &mut transaction,
                    community,
                    report.id,
                    "dismissed",
                    &actor,
                    Some(action_id),
                )
                .await
                .expect("resolve report");
                transaction.commit().await.expect("commit resolver");
                resolved
            }
        };

        let (first, second) = tokio::join!(
            resolve_once(pool.clone(), random_32()),
            resolve_once(pool.clone(), random_32())
        );
        assert_ne!(first, second, "exactly one concurrent resolver must win");

        let row = get_report(&pool, community, report_id)
            .await
            .expect("read report")
            .expect("report exists");
        let action_id = row.action_id.expect("winner action id");
        let matching = list_actions(&pool, community, 20)
            .await
            .expect("list actions")
            .into_iter()
            .filter(|action| action.id == action_id)
            .count();
        assert_eq!(matching, 1, "one committed resolution has one audit row");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn proxy_nonce_claims_are_atomic_domain_scoped_and_exclusive() {
        let pool = setup_pool().await;
        crate::migration::run_migrations(&pool)
            .await
            .expect("apply nonce-claim migration");
        let domain_a = make_test_community(&pool).await;
        let domain_b = make_test_community(&pool).await;
        let claim_key = random_32();
        let retain_until = Utc::now() + Duration::minutes(5);

        sqlx::query(
            "INSERT INTO authorization_proxy_nonce_claims \
             (authorization_domain, claim_kind, claim_key, retain_until) \
             VALUES ($1, 1, $2, $3)",
        )
        .bind(domain_a.as_uuid())
        .bind(&claim_key)
        .bind(retain_until)
        .execute(&pool)
        .await
        .expect("first claim commits");

        let replay = sqlx::query(
            "INSERT INTO authorization_proxy_nonce_claims \
             (authorization_domain, claim_kind, claim_key, retain_until) \
             VALUES ($1, 1, $2, $3)",
        )
        .bind(domain_a.as_uuid())
        .bind(&claim_key)
        .bind(retain_until)
        .execute(&pool)
        .await;
        assert!(replay.is_err(), "same-domain replay must be rejected");

        sqlx::query(
            "INSERT INTO authorization_proxy_nonce_claims \
             (authorization_domain, claim_kind, claim_key, retain_until) \
             VALUES ($1, 1, $2, $3)",
        )
        .bind(domain_b.as_uuid())
        .bind(&claim_key)
        .bind(retain_until)
        .execute(&pool)
        .await
        .expect("same opaque claim remains isolated across domains");

        let expired_key = random_32();
        let expired = sqlx::query(
            "INSERT INTO authorization_proxy_nonce_claims \
             (authorization_domain, claim_kind, claim_key, retain_until) \
             VALUES ($1, 1, $2, transaction_timestamp())",
        )
        .bind(domain_a.as_uuid())
        .bind(&expired_key)
        .execute(&pool)
        .await;
        assert!(expired.is_err(), "equality at retain_until is expired");

        // Construct an exact equality row transactionally with only the stamp
        // trigger disabled. The table CHECK remains active. This proves the
        // deletion trigger retains through, not merely until, the boundary.
        let equality_key = random_32();
        let mut equality = pool.begin().await.expect("begin equality probe");
        sqlx::query(
            "ALTER TABLE authorization_proxy_nonce_claims \
             DISABLE TRIGGER authorization_proxy_nonce_claims_stamp",
        )
        .execute(&mut *equality)
        .await
        .expect("disable insert stamp in rollback-only probe");
        sqlx::query(
            "INSERT INTO authorization_proxy_nonce_claims \
             (authorization_domain, claim_kind, claim_key, committed_at, retain_until) \
             VALUES ($1, 1, $2, transaction_timestamp() - INTERVAL '1 second', \
                     transaction_timestamp())",
        )
        .bind(domain_a.as_uuid())
        .bind(&equality_key)
        .execute(&mut *equality)
        .await
        .expect("insert exact-boundary probe row");
        sqlx::query(
            "ALTER TABLE authorization_proxy_nonce_claims \
             ENABLE TRIGGER authorization_proxy_nonce_claims_stamp",
        )
        .execute(&mut *equality)
        .await
        .expect("restore insert stamp in probe");
        let equality_delete = sqlx::query(
            "DELETE FROM authorization_proxy_nonce_claims \
             WHERE authorization_domain = $1 AND claim_kind = 1 AND claim_key = $2",
        )
        .bind(domain_a.as_uuid())
        .bind(&equality_key)
        .execute(&mut *equality)
        .await;
        assert!(
            equality_delete.is_err(),
            "deletion at the exclusive retain_until equality must be rejected"
        );
        equality.rollback().await.expect("rollback equality probe");

        let rolled_back_key = random_32();
        let mut transaction = pool.begin().await.expect("begin claim transaction");
        sqlx::query(
            "INSERT INTO authorization_proxy_nonce_claims \
             (authorization_domain, claim_kind, claim_key, retain_until) \
             VALUES ($1, 1, $2, $3)",
        )
        .bind(domain_a.as_uuid())
        .bind(&rolled_back_key)
        .bind(retain_until)
        .execute(&mut *transaction)
        .await
        .expect("stage claim");
        transaction.rollback().await.expect("rollback admission");
        let rolled_back_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_proxy_nonce_claims \
             WHERE authorization_domain = $1 AND claim_kind = 1 AND claim_key = $2",
        )
        .bind(domain_a.as_uuid())
        .bind(&rolled_back_key)
        .fetch_one(&pool)
        .await
        .expect("count rolled-back claim");
        assert_eq!(rolled_back_count, 0, "rollback must not consume the claim");

        let unavailable_key = random_32();
        let mut unavailable = pool.begin().await.expect("begin unavailable probe");
        sqlx::query("SET LOCAL search_path = pg_catalog")
            .execute(&mut *unavailable)
            .await
            .expect("isolate dependency");
        let dependency_failure = sqlx::query(
            "INSERT INTO authorization_proxy_nonce_claims \
             (authorization_domain, claim_kind, claim_key, retain_until) \
             VALUES ($1, 1, $2, $3)",
        )
        .bind(domain_a.as_uuid())
        .bind(&unavailable_key)
        .bind(retain_until)
        .execute(&mut *unavailable)
        .await;
        assert!(
            dependency_failure.is_err(),
            "unreadable claim storage must fail closed"
        );
        unavailable.rollback().await.expect("rollback failed probe");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn protected_projection_outbox_is_target_bound_ordered_and_idempotent() {
        let pool = setup_pool().await;
        crate::migration::run_migrations(&pool)
            .await
            .expect("apply projection-outbox migration");
        let community = make_test_community(&pool).await;
        let object_key = random_32();
        let projection_key = format!("repos/{}/pointer", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, 32, 1048576, 16384)",
        )
        .bind(community.as_uuid())
        .execute(&pool)
        .await
        .expect("install outbox-test audit capacity");

        let seed = stage_publication_admission(&pool, community, &object_key, 1, false)
            .await
            .expect("stage seed admission");
        sqlx::query(
            "INSERT INTO authorization_authority_epochs \
             (community_id, object_kind, object_key, authority_epoch, fence, \
              operation_id, request_fingerprint) \
             VALUES ($1, 3, $2, 7, $3, $4, $5)",
        )
        .bind(community.as_uuid())
        .bind(&object_key)
        .bind(random_32())
        .bind(seed.operation)
        .bind(&seed.request)
        .execute(&pool)
        .await
        .expect("install unchanged same-epoch authority row");

        let first = stage_publication_admission(&pool, community, &object_key, 2, true);
        let second = stage_publication_admission(&pool, community, &object_key, 3, true);
        let (first, second) = tokio::join!(first, second);
        let first = first.expect("first concurrent same-epoch publication");
        let second = second.expect("second concurrent same-epoch publication");

        let first_projection =
            insert_projection(&pool, community, &first, &object_key, &projection_key);
        let second_projection =
            insert_projection(&pool, community, &second, &object_key, &projection_key);
        let (first_projection, second_projection) =
            tokio::join!(first_projection, second_projection);
        first_projection.expect("insert first concurrent same-epoch projection");
        second_projection.expect("insert second concurrent same-epoch projection");

        let rows: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT operation_id, publication_sequence \
             FROM protected_publication_projection_outbox \
             WHERE community_id = $1 AND object_kind = 3 AND object_key = $2 \
               AND projection_kind = 3 AND projection_key = $3 \
             ORDER BY publication_sequence",
        )
        .bind(community.as_uuid())
        .bind(&object_key)
        .bind(&projection_key)
        .fetch_all(&pool)
        .await
        .expect("read concurrent publication order");
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].0, rows[1].0);
        assert!(rows[0].1 < rows[1].1);

        let stale_delivery = sqlx::query(
            "UPDATE protected_publication_projection_outbox \
             SET delivery_state = 2, attempt_count = 1, failure_code = 0 \
             WHERE community_id = $1 AND operation_id = $2 AND projection_kind = 3",
        )
        .bind(community.as_uuid())
        .bind(rows[1].0)
        .execute(&pool)
        .await;
        assert!(
            stale_delivery.is_err(),
            "newer concurrent version is fenced"
        );
        for (operation, _) in &rows {
            sqlx::query(
                "UPDATE protected_publication_projection_outbox \
                 SET delivery_state = 2, attempt_count = 1, failure_code = 0 \
                 WHERE community_id = $1 AND operation_id = $2 AND projection_kind = 3",
            )
            .bind(community.as_uuid())
            .bind(operation)
            .execute(&pool)
            .await
            .expect("deliver concurrent versions in database order");
        }

        let duplicate =
            insert_projection(&pool, community, &first, &object_key, &projection_key).await;
        assert!(
            duplicate.is_err(),
            "exact replay cannot duplicate an outbox effect"
        );

        let mismatch = stage_publication_admission(&pool, community, &object_key, 4, true)
            .await
            .expect("stage mismatch admission");
        let mut mismatch_tx = pool.begin().await.expect("begin mismatch projection");
        sqlx::query(
            "INSERT INTO protected_publication_projection_outbox \
             (community_id, operation_id, request_fingerprint, publication_result_digest, \
              object_kind, object_key, application_type, application_version, \
              application_effect_digest, projection_kind, projection_key, \
              staged_object_key, payload_digest) \
             VALUES ($1, $2, $3, $4, 3, $5, $6, 1, $7, 3, $8, $9, $10)",
        )
        .bind(community.as_uuid())
        .bind(mismatch.operation)
        .bind(&mismatch.request)
        .bind(&mismatch.application_result)
        .bind(&object_key)
        .bind(&mismatch.application_type)
        .bind(random_32())
        .bind(format!("{projection_key}/mismatch"))
        .bind("manifests/mismatch")
        .bind(random_32())
        .execute(&mut *mismatch_tx)
        .await
        .expect("stage deferred effect mismatch");
        assert!(
            mismatch_tx.commit().await.is_err(),
            "application effect mismatch must fail deferred binding"
        );

        let lower = stage_publication_admission(&pool, community, &object_key, 5, false)
            .await
            .expect("stage receipt rejected before an admission result");
        let mut lower_tx = pool.begin().await.expect("begin lower-epoch projection");
        sqlx::query(
            "INSERT INTO protected_publication_projection_outbox \
             (community_id, operation_id, request_fingerprint, publication_result_digest, \
              object_kind, object_key, application_type, application_version, \
              application_effect_digest, projection_kind, projection_key, \
              staged_object_key, payload_digest) \
             VALUES ($1, $2, $3, $4, 3, $5, $6, 1, $7, 3, $8, $9, $10)",
        )
        .bind(community.as_uuid())
        .bind(lower.operation)
        .bind(&lower.request)
        .bind(&lower.application_result)
        .bind(&object_key)
        .bind(&lower.application_type)
        .bind(&lower.application_effect)
        .bind(format!("{projection_key}/lower"))
        .bind("manifests/lower")
        .bind(random_32())
        .execute(&mut *lower_tx)
        .await
        .expect("stage lower-epoch projection without admitted result");
        assert!(
            lower_tx.commit().await.is_err(),
            "a lower-epoch denial without a committed admission result cannot publish"
        );
    }

    struct PublicationAdmissionFixture {
        operation: Uuid,
        request: Vec<u8>,
        application_type: Vec<u8>,
        application_effect: Vec<u8>,
        application_result: Vec<u8>,
        payload: Vec<u8>,
    }

    async fn stage_publication_admission(
        pool: &PgPool,
        community: CommunityId,
        object_key: &[u8],
        marker: u8,
        include_admission_result: bool,
    ) -> std::result::Result<PublicationAdmissionFixture, sqlx::Error> {
        let fixture = PublicationAdmissionFixture {
            operation: Uuid::new_v4(),
            request: vec![marker.saturating_add(10); 32],
            application_type: vec![marker.saturating_add(20); 32],
            application_effect: vec![marker.saturating_add(30); 32],
            application_result: vec![marker.saturating_add(40); 32],
            payload: vec![marker.saturating_add(50); 32],
        };
        let mut transaction = pool.begin().await?;
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 11, $4, 1, $5)",
        )
        .bind(community.as_uuid())
        .bind(fixture.operation)
        .bind(&fixture.request)
        .bind(vec![marker.saturating_add(60); 32])
        .bind(vec![marker.saturating_add(70); 32])
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, actor_kind, \
              actor_fingerprint, operation_id, request_fingerprint, correlation_id, attempt_id, \
              occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 10, 1, 1, 1, $3, $4, $5, $6, $7, \
                     transaction_timestamp(), $8, $9)",
        )
        .bind(community.as_uuid())
        .bind(Uuid::new_v4())
        .bind(vec![marker.saturating_add(60); 32])
        .bind(fixture.operation)
        .bind(&fixture.request)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(vec![marker])
        .bind(vec![marker.saturating_add(80); 32])
        .execute(&mut *transaction)
        .await?;
        if include_admission_result {
            sqlx::query(
                "INSERT INTO authorization_admission_results \
                 (community_id, operation_id, request_fingerprint, semantic_fingerprint, \
                  object_kind, object_key, application_type, application_version, \
                  application_code, application_payload, application_intent_digest, \
                  application_effect_digest, application_result_digest) \
                 VALUES ($1, $2, $3, $4, 3, $5, $6, 1, 1, $7, $8, $9, $10)",
            )
            .bind(community.as_uuid())
            .bind(fixture.operation)
            .bind(&fixture.request)
            .bind(vec![marker.saturating_add(90); 32])
            .bind(object_key)
            .bind(&fixture.application_type)
            .bind(vec![marker])
            .bind(vec![marker.saturating_add(100); 32])
            .bind(&fixture.application_effect)
            .bind(&fixture.application_result)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(fixture)
    }

    async fn insert_projection(
        pool: &PgPool,
        community: CommunityId,
        fixture: &PublicationAdmissionFixture,
        object_key: &[u8],
        projection_key: &str,
    ) -> std::result::Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
        sqlx::query(
            "INSERT INTO protected_publication_projection_outbox \
             (community_id, operation_id, request_fingerprint, publication_result_digest, \
              object_kind, object_key, application_type, application_version, \
              application_effect_digest, projection_kind, projection_key, \
              staged_object_key, payload_digest) \
             VALUES ($1, $2, $3, $4, 3, $5, $6, 1, $7, 3, $8, $9, $10)",
        )
        .bind(community.as_uuid())
        .bind(fixture.operation)
        .bind(&fixture.request)
        .bind(&fixture.application_result)
        .bind(object_key)
        .bind(&fixture.application_type)
        .bind(&fixture.application_effect)
        .bind(projection_key)
        .bind(format!("manifests/{}", fixture.operation))
        .bind(&fixture.payload)
        .execute(pool)
        .await
    }
}
