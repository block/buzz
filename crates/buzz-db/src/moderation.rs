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
use sqlx::PgPool;
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

/// Insert parameters for a moderation audit row.
#[derive(Debug, Clone)]
pub struct NewAction<'a> {
    /// Acting moderator pubkey bytes.
    pub actor_pubkey: &'a [u8],
    /// `delete_message` | `kick` | `ban` | `unban` | `timeout` | `untimeout`
    /// | `dismiss_report` | `escalate` (DB CHECK-enforced).
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
    /// Action time.
    pub created_at: DateTime<Utc>,
}

/// Insert a new report row. Idempotent on `(community, report_event_id)`:
/// re-ingesting the same signed report is a no-op returning the existing id.
pub async fn insert_report(
    _pool: &PgPool,
    _community: CommunityId,
    _report: NewReport<'_>,
) -> Result<Uuid> {
    todo!("L1 (Max): INSERT ... ON CONFLICT (community_id, report_event_id) DO NOTHING + return id")
}

/// List reports for the moderation queue, newest first.
/// `status = None` lists all; `Some("open")` etc. filters.
pub async fn list_reports(
    _pool: &PgPool,
    _community: CommunityId,
    _status: Option<&str>,
    _limit: i64,
) -> Result<Vec<ReportRecord>> {
    todo!("L1 (Max)")
}

/// Fetch one report by row id.
pub async fn get_report(
    _pool: &PgPool,
    _community: CommunityId,
    _report_id: Uuid,
) -> Result<Option<ReportRecord>> {
    todo!("L1 (Max)")
}

/// Mark a report resolved/dismissed/escalated, linking the audit action.
/// Returns `false` if the report was not found or already closed.
pub async fn resolve_report(
    _pool: &PgPool,
    _community: CommunityId,
    _report_id: Uuid,
    _status: &str,
    _resolved_by: &[u8],
    _action_id: Option<Uuid>,
) -> Result<bool> {
    todo!("L1 (Max)")
}

/// Upsert a ban: sets `banned = true` with optional expiry + reason.
pub async fn ban_member(
    _pool: &PgPool,
    _community: CommunityId,
    _pubkey: &[u8],
    _actor: &[u8],
    _reason: Option<&str>,
    _expires_at: Option<DateTime<Utc>>,
) -> Result<()> {
    todo!("L1 (Max): INSERT ... ON CONFLICT (community_id, pubkey) DO UPDATE")
}

/// Lift a ban. Returns `false` if the member was not banned.
pub async fn unban_member(
    _pool: &PgPool,
    _community: CommunityId,
    _pubkey: &[u8],
    _actor: &[u8],
) -> Result<bool> {
    todo!("L1 (Max)")
}

/// Upsert a timeout: sets `muted_until` + reason.
pub async fn timeout_member(
    _pool: &PgPool,
    _community: CommunityId,
    _pubkey: &[u8],
    _actor: &[u8],
    _muted_until: DateTime<Utc>,
    _reason: Option<&str>,
) -> Result<()> {
    todo!("L1 (Max)")
}

/// Clear a timeout early. Returns `false` if the member was not timed out.
pub async fn untimeout_member(
    _pool: &PgPool,
    _community: CommunityId,
    _pubkey: &[u8],
    _actor: &[u8],
) -> Result<bool> {
    todo!("L1 (Max)")
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
    _pool: &PgPool,
    _community: CommunityId,
    _pubkey: &[u8],
) -> Result<RestrictionState> {
    todo!("L1 (Max): single SELECT evaluating ban expiry + mute window in SQL")
}

/// Fetch the full ban/timeout row (moderation queue / audit views).
pub async fn get_ban(
    _pool: &PgPool,
    _community: CommunityId,
    _pubkey: &[u8],
) -> Result<Option<BanRecord>> {
    todo!("L1 (Max)")
}

/// List currently-restricted members (active ban or timeout) for the queue.
pub async fn list_restricted(_pool: &PgPool, _community: CommunityId) -> Result<Vec<BanRecord>> {
    todo!("L1 (Max)")
}

/// Insert a moderation audit row, returning its id.
pub async fn insert_action(
    _pool: &PgPool,
    _community: CommunityId,
    _action: NewAction<'_>,
) -> Result<Uuid> {
    todo!("L1 (Max)")
}

/// List audit rows, newest first (`buzz moderation audit`).
pub async fn list_actions(
    _pool: &PgPool,
    _community: CommunityId,
    _limit: i64,
) -> Result<Vec<ActionRecord>> {
    todo!("L1 (Max)")
}
