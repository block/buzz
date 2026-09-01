//! HTTP report-resolution enforcement state machine persistence.
//!
//! Backs the `relay_admin_actions` and `relay_admin_outbox` tables from
//! `migrations/0036_relay_admin_actions.sql` and
//! `migrations/0037_relay_admin_action_lease.sql`.
//!
//! This module is the only persistence allowed to write to `relay_admin_actions`;
//! report claim, step advancement, and finalization all go through the
//! functions here.
//!
//! Lane ownership: relay admin API (Duncan).

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::error::Result;
use crate::CommunityId;

/// A row in `relay_admin_actions`.
#[derive(Debug, Clone)]
pub struct AdminActionRecord {
    /// Action UUID.
    pub id: Uuid,
    /// Report this action targets.
    pub report_id: Uuid,
    /// Community the report belongs to.
    pub report_community_id: Uuid,
    /// Client-generated idempotency key.
    pub request_id: Uuid,
    /// Principal who claimed the report.
    pub actor_pubkey: Vec<u8>,
    /// Role of the actor (`"operator"` | `"moderator"`).
    pub actor_role: String,
    /// Enforcement action name.
    pub action: String,
    /// Optional reason provided by the actor.
    pub reason: Option<String>,
    /// Timeout expiration for timeout actions.
    pub timeout_until: Option<DateTime<Utc>>,
    /// State machine: `"pending"` | `"enforcing"` | `"succeeded"` | `"failed"` | `"cancelled"`.
    pub state: String,
    /// Durably committed step: `None` = not started, `"mutation_committed"`, `"artifacts_done"`.
    pub step_marker: Option<String>,
    /// Principal who cancelled this action (32-byte pubkey); `None` until cancelled.
    pub cancelled_by: Option<Vec<u8>>,
    /// Error from the last failure, if any.
    pub error_message: Option<String>,
    /// Row creation time.
    pub created_at: DateTime<Utc>,
    /// Row last-updated time.
    pub updated_at: DateTime<Utc>,
}

/// A row in `relay_admin_outbox`.
#[derive(Debug, Clone)]
pub struct OutboxRecord {
    /// Outbox row UUID.
    pub id: Uuid,
    /// Owning action.
    pub action_id: Uuid,
    /// Delivery task type: `"tombstone"` | `"system_message"` | `"reporter_notice"`.
    pub task_type: String,
    /// Task payload.
    pub payload: serde_json::Value,
    /// Delivery state.
    pub state: String,
    /// Deduplication key.
    pub dedup_key: Option<String>,
    /// Error from the last delivery attempt.
    pub error_message: Option<String>,
    /// Number of delivery attempts made so far.
    pub attempt_count: i32,
    /// Opaque claim token written at claim time. Required by `mark_outbox_delivered`
    /// and `fail_outbox_row` to fence against stale workers.
    pub claim_token: Uuid,
    /// Row creation time. Used as `idempotency_ts` for system-message signing so
    /// that retries produce the same Nostr event ID.
    pub created_at: DateTime<Utc>,
}

/// Result of attempting to claim a report for HTTP enforcement.
#[derive(Debug)]
pub enum ClaimResult {
    /// Successfully claimed. Returns the new action record.
    Claimed(AdminActionRecord),
    /// An existing action with the same `request_id` was found — idempotent retry.
    AlreadyClaimed(AdminActionRecord),
    /// The report is not in `open` status. Returns its current status.
    NotOpen(String),
    /// The report was not found globally.
    NotFound,
}

/// Result of attempting to acquire the action mutation lease.
#[derive(Debug)]
pub enum LeaseResult {
    /// Lease acquired; caller may proceed with the mutation.
    Acquired(Uuid),
    /// Another driver holds a live lease; caller should reload and retry.
    Contended,
    /// Action is not in a leasable state (already succeeded/failed/cancelled).
    NotLeasable,
}

/// A stranded action claimed by the action recovery worker.
#[derive(Debug)]
pub struct StrandedActionClaim {
    /// The claimed action record.
    pub record: AdminActionRecord,
    /// Lease token the worker holds.
    pub lease_token: Uuid,
}

/// Atomically resolve a report without enforcement (decision-only).
///
/// Inserts the decision audit row AND CASes the report status `open → terminal`
/// in a single transaction. If the report is not in `open` status, the whole
/// transaction rolls back — no orphan audit row.
///
/// Returns `true` if the report was successfully closed, `false` if the CAS
/// failed (report not open or wrong community).
#[allow(clippy::too_many_arguments)]
pub async fn resolve_report_decision_atomic(
    pool: &PgPool,
    community_id: CommunityId,
    report_id: Uuid,
    terminal_status: &str,
    audit_action: &str,
    actor_pubkey: &[u8],
    actor_authority: &str,
    target_pubkey: Option<&[u8]>,
    target_event_id: Option<&[u8]>,
    channel_id: Option<Uuid>,
    reason: Option<&str>,
) -> Result<bool> {
    let mut tx = pool.begin().await?;

    // CAS: open → terminal. The update count tells us whether the report was open.
    let updated = sqlx::query(
        r#"
        UPDATE moderation_reports
        SET status = $3, resolved_by = $4, resolved_at = now(), active_action_id = NULL
        WHERE community_id = $1 AND id = $2 AND status = 'open'
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(report_id)
    .bind(terminal_status)
    .bind(actor_pubkey)
    .execute(&mut *tx)
    .await?;

    if updated.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }

    // Insert the decision audit row in the same transaction.
    sqlx::query(
        r#"
        INSERT INTO moderation_actions (
            community_id, actor_pubkey, action, target_pubkey, target_event_id,
            channel_id, public_reason, actor_authority
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(actor_pubkey)
    .bind(audit_action)
    .bind(target_pubkey)
    .bind(target_event_id)
    .bind(channel_id)
    .bind(reason)
    .bind(actor_authority)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

/// Claim a report for HTTP enforcement via a single-transaction CAS.
///
/// - If `status = 'open'`: sets `status = 'processing'`, `active_action_id = new_action.id`,
///   inserts the decision audit row, and inserts the action record (state=`pending`).
///   Returns `ClaimResult::Claimed`. **No outbox rows are inserted here** — they are
///   created atomically in `finalize_success` after the mutation succeeds.
///
/// - If `status = 'processing'` and an action with the same `(community_id, report_id, request_id)`
///   already exists: idempotent retry — returns `ClaimResult::AlreadyClaimed` with the
///   existing action record.
///
/// - If `status = 'processing'` with a different `request_id`, or any other status:
///   returns `ClaimResult::NotOpen(status)`.
///
/// Decision audit row is written in the same transaction with the given `actor_authority`.
#[allow(clippy::too_many_arguments)]
pub async fn claim_report(
    pool: &PgPool,
    community_id: CommunityId,
    report_id: Uuid,
    request_id: Uuid,
    actor_pubkey: &[u8],
    actor_role: &str,
    action: &str,
    reason: Option<&str>,
    timeout_until: Option<DateTime<Utc>>,
    audit_action: &str,
    actor_authority: &str,
    target_pubkey: Option<&[u8]>,
    target_event_id: Option<&[u8]>,
    channel_id: Option<Uuid>,
) -> Result<ClaimResult> {
    let mut tx = pool.begin().await?;

    // Lock the report row to serialize concurrent claims on the same report.
    let report_row = sqlx::query(
        r#"
        SELECT id, status, active_action_id
        FROM moderation_reports
        WHERE community_id = $1 AND id = $2
        FOR UPDATE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(report_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(report_row) = report_row else {
        return Ok(ClaimResult::NotFound);
    };

    let status: String = report_row.try_get("status")?;

    // Idempotent retry: if this exact request_id already claimed, return existing.
    if status == "processing" {
        let existing = sqlx::query(
            r#"
            SELECT id, report_id, report_community_id, request_id, actor_pubkey, actor_role,
                   action, reason, timeout_until, state, step_marker, cancelled_by, error_message,
                   created_at, updated_at
            FROM relay_admin_actions
            WHERE report_community_id = $1 AND report_id = $2 AND request_id = $3
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(report_id)
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(row) = existing {
            tx.rollback().await?;
            return Ok(ClaimResult::AlreadyClaimed(row_to_action(row)?));
        }
        // Different request_id against processing report → conflict.
        return Ok(ClaimResult::NotOpen(status));
    }

    if status != "open" {
        return Ok(ClaimResult::NotOpen(status));
    }

    // Insert the action record first to get its ID.
    let action_row = sqlx::query(
        r#"
        INSERT INTO relay_admin_actions (
            report_id, report_community_id, request_id, actor_pubkey, actor_role,
            action, reason, timeout_until, state
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending')
        RETURNING id, report_id, report_community_id, request_id, actor_pubkey, actor_role,
                  action, reason, timeout_until, state, step_marker, cancelled_by, error_message,
                  created_at, updated_at
        "#,
    )
    .bind(report_id)
    .bind(community_id.as_uuid())
    .bind(request_id)
    .bind(actor_pubkey)
    .bind(actor_role)
    .bind(action)
    .bind(reason)
    .bind(timeout_until)
    .fetch_one(&mut *tx)
    .await?;

    let action_id: Uuid = action_row.try_get("id")?;

    // Insert the decision audit row in the same transaction.
    sqlx::query(
        r#"
        INSERT INTO moderation_actions (
            community_id, actor_pubkey, action, target_pubkey, target_event_id,
            channel_id, public_reason, actor_authority
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(actor_pubkey)
    .bind(audit_action)
    .bind(target_pubkey)
    .bind(target_event_id)
    .bind(channel_id)
    .bind(reason)
    .bind(actor_authority)
    .execute(&mut *tx)
    .await?;

    // CAS: set report to processing with active_action_id.
    let updated = sqlx::query(
        r#"
        UPDATE moderation_reports
        SET status = 'processing', active_action_id = $3
        WHERE community_id = $1 AND id = $2 AND status = 'open'
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(report_id)
    .bind(action_id)
    .execute(&mut *tx)
    .await?;

    if updated.rows_affected() == 0 {
        // Shouldn't happen since we locked the row above, but be defensive.
        tx.rollback().await?;
        return Ok(ClaimResult::NotOpen("concurrent_update".to_string()));
    }

    tx.commit().await?;

    Ok(ClaimResult::Claimed(row_to_action(action_row)?))
}

/// Acquire the action mutation lease. Only one driver (HTTP request or action
/// worker) may hold the lease at a time; a second concurrent driver reloads
/// and retries rather than running the mutation twice.
///
/// - `state IN ('pending', 'enforcing')` AND lease expired or unset → lease granted.
/// - Lease held by another driver → `Contended`.
/// - Action already in terminal state → `NotLeasable`.
pub async fn acquire_action_lease(
    pool: &PgPool,
    action_id: Uuid,
    lease_until: DateTime<Utc>,
) -> Result<LeaseResult> {
    let token = Uuid::new_v4();
    let result = sqlx::query(
        r#"
        UPDATE relay_admin_actions
        SET action_lease_token = $2, action_lease_expires_at = $3, updated_at = now()
        WHERE id = $1
          AND state IN ('pending', 'enforcing')
          AND (action_lease_expires_at IS NULL OR action_lease_expires_at < now())
        "#,
    )
    .bind(action_id)
    .bind(token)
    .bind(lease_until)
    .execute(pool)
    .await?;

    if result.rows_affected() > 0 {
        return Ok(LeaseResult::Acquired(token));
    }

    // Check why the lease failed: is the action in a terminal state or contended?
    let row = sqlx::query("SELECT state FROM relay_admin_actions WHERE id = $1")
        .bind(action_id)
        .fetch_optional(pool)
        .await?;

    match row {
        None => Ok(LeaseResult::NotLeasable),
        Some(r) => {
            let state: String = r.try_get("state")?;
            if matches!(state.as_str(), "succeeded" | "failed" | "cancelled") {
                Ok(LeaseResult::NotLeasable)
            } else {
                Ok(LeaseResult::Contended)
            }
        }
    }
}

/// Release the action mutation lease (clears token and expiry).
/// Only releases if the caller still holds the given token.
pub async fn release_action_lease(pool: &PgPool, action_id: Uuid, lease_token: Uuid) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE relay_admin_actions
        SET action_lease_token = NULL, action_lease_expires_at = NULL, updated_at = now()
        WHERE id = $1 AND action_lease_token = $2
        "#,
    )
    .bind(action_id)
    .bind(lease_token)
    .execute(pool)
    .await?;
    Ok(())
}

/// Advance the action to 'enforcing' state. Returns false if the action was
/// not in 'pending' state (e.g. concurrent worker picked it up).
pub async fn begin_enforcing(pool: &PgPool, action_id: Uuid) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE relay_admin_actions
        SET state = 'enforcing', updated_at = now()
        WHERE id = $1 AND state = 'pending'
        "#,
    )
    .bind(action_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Atomically execute a ban mutation and commit the step marker in one
/// transaction, fenced by `action_id` AND the caller's `lease_token`.
///
/// Performs:
/// 1. `UPSERT` into `community_bans` for the target pubkey.
/// 2. `UPDATE relay_admin_actions SET step_marker = 'mutation_committed'` where
///    `id = action_id AND action_lease_token = lease_token AND state = 'enforcing'
///    AND step_marker IS NULL`.
///
/// The lease token is a real DB fence: if the caller's token no longer matches
/// the row (because the lease expired and another pod reclaimed the action), the
/// marker UPDATE affects zero rows and the transaction rolls back — the domain
/// mutation never commits.
///
/// Returns `true` if the step marker was successfully committed (i.e. this
/// driver owns the action and the mutation landed). Returns `false` if the
/// action was already marked or the lease was lost (idempotent re-drive or
/// stale worker: caller must stop or reload).
pub async fn execute_ban_with_marker(
    pool: &PgPool,
    action_id: Uuid,
    lease_token: Uuid,
    community_id: CommunityId,
    target_pubkey: &[u8],
    actor_pubkey: &[u8],
    reason: Option<&str>,
) -> Result<bool> {
    let mut tx = pool.begin().await?;

    // Verify lease ownership first — abort without touching domain rows if the
    // lease is already gone. This prevents the commit entirely on a stale worker.
    let owned: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM relay_admin_actions
            WHERE id = $1
              AND action_lease_token = $2
              AND action_lease_expires_at > now()
              AND state = 'enforcing'
        )
        "#,
    )
    .bind(action_id)
    .bind(lease_token)
    .fetch_one(&mut *tx)
    .await?;

    if !owned {
        tx.rollback().await?;
        return Ok(false);
    }

    sqlx::query(
        r#"
        INSERT INTO community_bans (community_id, pubkey, banned, actor_pubkey, ban_reason)
        VALUES ($1, $2, TRUE, $3, $4)
        ON CONFLICT (community_id, pubkey)
        DO UPDATE SET banned = TRUE, actor_pubkey = EXCLUDED.actor_pubkey,
                      ban_reason = EXCLUDED.ban_reason, updated_at = now()
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(target_pubkey)
    .bind(actor_pubkey)
    .bind(reason)
    .execute(&mut *tx)
    .await?;

    let marker = sqlx::query(
        r#"
        UPDATE relay_admin_actions
        SET step_marker = 'mutation_committed', updated_at = now()
        WHERE id = $1
          AND action_lease_token = $2
          AND action_lease_expires_at > now()
          AND state = 'enforcing'
          AND step_marker IS NULL
        "#,
    )
    .bind(action_id)
    .bind(lease_token)
    .execute(&mut *tx)
    .await?;

    if marker.rows_affected() == 0 {
        // Lease was lost between the ownership check and the UPDATE (race), or
        // step_marker was already set by another driver. Roll back domain change.
        tx.rollback().await?;
        return Ok(false);
    }

    tx.commit().await?;
    Ok(true)
}

/// Atomically execute a timeout mutation and commit the step marker in one
/// transaction, fenced by `action_id` AND the caller's `lease_token`.
#[allow(clippy::too_many_arguments)]
pub async fn execute_timeout_with_marker(
    pool: &PgPool,
    action_id: Uuid,
    lease_token: Uuid,
    community_id: CommunityId,
    target_pubkey: &[u8],
    actor_pubkey: &[u8],
    until: DateTime<Utc>,
    reason: Option<&str>,
) -> Result<bool> {
    let mut tx = pool.begin().await?;

    let owned: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM relay_admin_actions
            WHERE id = $1
              AND action_lease_token = $2
              AND action_lease_expires_at > now()
              AND state = 'enforcing'
        )
        "#,
    )
    .bind(action_id)
    .bind(lease_token)
    .fetch_one(&mut *tx)
    .await?;

    if !owned {
        tx.rollback().await?;
        return Ok(false);
    }

    sqlx::query(
        r#"
        INSERT INTO community_bans (community_id, pubkey, banned, muted_until, actor_pubkey, mute_reason)
        VALUES ($1, $2, FALSE, $3, $4, $5)
        ON CONFLICT (community_id, pubkey)
        DO UPDATE SET muted_until = EXCLUDED.muted_until,
                      actor_pubkey = EXCLUDED.actor_pubkey,
                      mute_reason = EXCLUDED.mute_reason,
                      updated_at = now()
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(target_pubkey)
    .bind(until)
    .bind(actor_pubkey)
    .bind(reason)
    .execute(&mut *tx)
    .await?;

    let marker = sqlx::query(
        r#"
        UPDATE relay_admin_actions
        SET step_marker = 'mutation_committed', updated_at = now()
        WHERE id = $1
          AND action_lease_token = $2
          AND action_lease_expires_at > now()
          AND state = 'enforcing'
          AND step_marker IS NULL
        "#,
    )
    .bind(action_id)
    .bind(lease_token)
    .execute(&mut *tx)
    .await?;

    if marker.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }

    tx.commit().await?;
    Ok(true)
}

/// Result of the kick-with-marker atomic operation.
pub enum KickWithMarkerResult {
    /// Member was present and removed; step marker committed.
    Removed,
    /// Member was already absent before this action; step marker NOT committed
    /// so the caller can record a pre-provenance failure.
    AlreadyGone,
    /// The action ownership fence rejected the marker (action already marked).
    AlreadyMarked,
}

/// Atomically execute a kick mutation and commit the step marker in one
/// transaction, fenced by `action_id` AND the caller's `lease_token`.
///
/// The kick step marker is committed only if the member was present. If the
/// member was already gone (`UPDATE … rows_affected = 0`), the step marker is
/// NOT written so that `run_enforcement_mutation` can distinguish this action's
/// own prior removal from pre-existing absence.
pub async fn execute_kick_with_marker(
    pool: &PgPool,
    action_id: Uuid,
    lease_token: Uuid,
    community_id: CommunityId,
    channel_id: Uuid,
    target_pubkey: &[u8],
    actor_pubkey: &[u8],
) -> Result<KickWithMarkerResult> {
    let mut tx = pool.begin().await?;

    let owned: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM relay_admin_actions
            WHERE id = $1
              AND action_lease_token = $2
              AND action_lease_expires_at > now()
              AND state = 'enforcing'
        )
        "#,
    )
    .bind(action_id)
    .bind(lease_token)
    .fetch_one(&mut *tx)
    .await?;

    if !owned {
        tx.rollback().await?;
        // Return AlreadyMarked so callers follow the same "skip mutation, go to finalize"
        // path as when another driver already committed the marker.
        return Ok(KickWithMarkerResult::AlreadyMarked);
    }

    let kick = sqlx::query(
        r#"
        UPDATE channel_members
        SET removed_at = NOW(), removed_by = $1
        WHERE community_id = $2 AND channel_id = $3 AND pubkey = $4 AND removed_at IS NULL
        "#,
    )
    .bind(actor_pubkey)
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(target_pubkey)
    .execute(&mut *tx)
    .await?;

    if kick.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(KickWithMarkerResult::AlreadyGone);
    }

    let marker = sqlx::query(
        r#"
        UPDATE relay_admin_actions
        SET step_marker = 'mutation_committed', updated_at = now()
        WHERE id = $1
          AND action_lease_token = $2
          AND action_lease_expires_at > now()
          AND state = 'enforcing'
          AND step_marker IS NULL
        "#,
    )
    .bind(action_id)
    .bind(lease_token)
    .execute(&mut *tx)
    .await?;

    if marker.rows_affected() == 0 {
        // Lease lost after kick but before marker commit — roll back the kick too.
        tx.rollback().await?;
        return Ok(KickWithMarkerResult::AlreadyMarked);
    }

    tx.commit().await?;
    Ok(KickWithMarkerResult::Removed)
}

/// Atomically execute a soft-delete mutation and commit the step marker in one
/// transaction, fenced by `action_id` AND the caller's `lease_token`.
///
/// The delete is idempotent: if the event is already deleted the marker is still
/// committed (soft-delete is already-done = success).
pub async fn execute_delete_with_marker(
    pool: &PgPool,
    action_id: Uuid,
    lease_token: Uuid,
    community_id: CommunityId,
    target_event_id: &[u8],
    parent_event_id: Option<&[u8]>,
    _root_event_id: Option<&[u8]>,
) -> Result<bool> {
    let mut tx = pool.begin().await?;

    let owned: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM relay_admin_actions
            WHERE id = $1
              AND action_lease_token = $2
              AND action_lease_expires_at > now()
              AND state = 'enforcing'
        )
        "#,
    )
    .bind(action_id)
    .bind(lease_token)
    .fetch_one(&mut *tx)
    .await?;

    if !owned {
        tx.rollback().await?;
        return Ok(false);
    }

    // Soft-delete the event and update thread metadata (idempotent: already-deleted is a no-op).
    sqlx::query(
        r#"
        UPDATE events
        SET deleted_at = now()
        WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(target_event_id)
    .execute(&mut *tx)
    .await?;

    // Update thread metadata if parent is known.
    if let Some(parent) = parent_event_id {
        sqlx::query(
            r#"
            UPDATE events
            SET reply_count = GREATEST(reply_count - 1, 0)
            WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(parent)
        .execute(&mut *tx)
        .await?;
    }

    let marker = sqlx::query(
        r#"
        UPDATE relay_admin_actions
        SET step_marker = 'mutation_committed', updated_at = now()
        WHERE id = $1
          AND action_lease_token = $2
          AND action_lease_expires_at > now()
          AND state = 'enforcing'
          AND step_marker IS NULL
        "#,
    )
    .bind(action_id)
    .bind(lease_token)
    .execute(&mut *tx)
    .await?;

    if marker.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }

    tx.commit().await?;
    Ok(true)
}

/// Commit the core mutation step and advance the step_marker to
/// 'mutation_committed' in one transaction. This is the idempotency point:
/// a crash after this returns true on re-drive; re-drive skips the mutation
/// and resumes from artifact delivery.
///
/// Returns false if the action was not found or not in the expected state.
pub async fn commit_mutation_step(pool: &PgPool, action_id: Uuid) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE relay_admin_actions
        SET step_marker = 'mutation_committed', updated_at = now()
        WHERE id = $1 AND state = 'enforcing' AND step_marker IS NULL
        "#,
    )
    .bind(action_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Atomically finalize the enforcement: action → succeeded, report → terminal status,
/// and enqueue outbox delivery rows.
///
/// Requires that:
/// - The action is in `enforcing` state WITH `step_marker = 'mutation_committed'`.
/// - The report's `active_action_id` matches this action (prevents wrong-action finalization).
///
/// On success, outbox rows for tombstone/system_message/reporter_notice are inserted
/// in the same transaction — delivery rows are created only after the mutation has
/// durably committed, preventing pre-success artifact delivery.
///
/// Returns false if either fence fails (ownership lost or wrong step).
#[allow(clippy::too_many_arguments)]
pub async fn finalize_success(
    pool: &PgPool,
    action_id: Uuid,
    community_id: CommunityId,
    report_id: Uuid,
    terminal_status: &str,
    actor_pubkey: &[u8],
    action_name: &str,
    target_pubkey: Option<&[u8]>,
    target_event_id: Option<&[u8]>,
    channel_id: Option<Uuid>,
    reason: Option<&str>,
    timeout_until: Option<DateTime<Utc>>,
) -> Result<bool> {
    let mut tx = pool.begin().await?;

    // Require step_marker = 'mutation_committed' to prevent premature finalization.
    let updated_action = sqlx::query(
        r#"
        UPDATE relay_admin_actions
        SET state = 'succeeded', step_marker = 'artifacts_done', updated_at = now()
        WHERE id = $1 AND state = 'enforcing' AND step_marker = 'mutation_committed'
        "#,
    )
    .bind(action_id)
    .execute(&mut *tx)
    .await?;

    if updated_action.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }

    // Transition report to terminal status. Requires active_action_id = this action,
    // which prevents a stale or wrong action from closing the report.
    let updated_report = sqlx::query(
        r#"
        UPDATE moderation_reports
        SET status = $3, resolved_by = $4, resolved_at = now(),
            active_action_id = NULL
        WHERE community_id = $1 AND id = $2
          AND status = 'processing'
          AND active_action_id = $5
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(report_id)
    .bind(terminal_status)
    .bind(actor_pubkey)
    .bind(action_id)
    .execute(&mut *tx)
    .await?;

    if updated_report.rows_affected() == 0 {
        // The report CAS failed: either the report moved to a different state
        // or active_action_id no longer matches. Roll back the action update too.
        tx.rollback().await?;
        return Ok(false);
    }

    // Enqueue outbox delivery rows in the same finalization transaction.
    // This is the authoritative creation point: delivery rows exist iff and only
    // iff enforcement succeeded, preventing tombstone/notice delivery on failed actions.
    let community_str = community_id.as_uuid().to_string();
    let action_str = action_id.to_string();

    if action_name == "delete" {
        if let (Some(target_eid), Some(ch)) = (target_event_id, channel_id) {
            let payload = serde_json::json!({
                "community_id": community_str,
                "channel_id": ch.to_string(),
                "target_event_id": hex::encode(target_eid),
                "action_id": action_str,
                "actor": hex::encode(actor_pubkey),
                "reason_code": reason.unwrap_or(""),
            });
            sqlx::query(
                r#"
                INSERT INTO relay_admin_outbox (action_id, task_type, payload, dedup_key)
                VALUES ($1, 'tombstone', $2, $3)
                ON CONFLICT (dedup_key) DO NOTHING
                "#,
            )
            .bind(action_id)
            .bind(payload)
            .bind(format!("tombstone:{action_str}"))
            .execute(&mut *tx)
            .await?;
        }
    }

    if action_name == "kick" {
        if let (Some(target_pk), Some(ch)) = (target_pubkey, channel_id) {
            let payload = serde_json::json!({
                "community_id": community_str,
                "channel_id": ch.to_string(),
                "target": hex::encode(target_pk),
                "action_id": action_str,
            });
            sqlx::query(
                r#"
                INSERT INTO relay_admin_outbox (action_id, task_type, payload, dedup_key)
                VALUES ($1, 'system_message', $2, $3)
                ON CONFLICT (dedup_key) DO NOTHING
                "#,
            )
            .bind(action_id)
            .bind(payload)
            .bind(format!("system_message:{action_str}"))
            .execute(&mut *tx)
            .await?;
        }
    }

    // Always enqueue a reporter notice. Payload carries action_id; the worker
    // looks up report_id → reporter_pubkey at delivery time.
    let notice_payload = serde_json::json!({
        "action_id": action_str,
        "community_id": community_str,
    });
    sqlx::query(
        r#"
        INSERT INTO relay_admin_outbox (action_id, task_type, payload, dedup_key)
        VALUES ($1, 'reporter_notice', $2, $3)
        ON CONFLICT (dedup_key) DO NOTHING
        "#,
    )
    .bind(action_id)
    .bind(notice_payload)
    .bind(format!("reporter_notice:{action_str}"))
    .execute(&mut *tx)
    .await?;

    // Enqueue a recipient-specific notice to the actioned user so the restricted
    // party hears the truth (VISION_MODERATION: "Reasons travel … to the
    // restricted user"). Mapped onto the existing notice variants:
    //   delete/kick → ContentActioned (action taken on their content/presence)
    //   ban/timeout → Restriction (terms of the restriction)
    // Best-effort like the reporter notice: enqueued in this same transaction,
    // delivered asynchronously; a delivery failure never undoes enforcement. When
    // the target pubkey is absent (a purged event with no derivable author on a
    // `delete`), there is no one to notify, so the notice is simply skipped.
    let affected_notice: Option<(&str, Option<&str>)> = match action_name {
        "delete" | "kick" => Some(("content_actioned", None)),
        "ban" => Some(("restriction", Some("ban"))),
        "timeout" => Some(("restriction", Some("timeout"))),
        _ => None,
    };
    if let (Some((notice_kind, restriction_kind)), Some(recipient)) =
        (affected_notice, target_pubkey)
    {
        let mut affected_payload = serde_json::json!({
            "community_id": community_str,
            "recipient": hex::encode(recipient),
            "notice_kind": notice_kind,
            "public_reason": reason.unwrap_or(""),
        });
        if let Some(rk) = restriction_kind {
            affected_payload["restriction_kind"] = serde_json::Value::String(rk.to_string());
        }
        // A `timeout` carries its expiry so the notice can tell the user "until
        // <when>". A `ban` is indefinite (no expiry); a `delete`/`kick` is not a
        // restriction. `timeout_until` is authoritative from the action row.
        if restriction_kind == Some("timeout") {
            if let Some(until) = timeout_until {
                affected_payload["timeout_until"] = serde_json::Value::String(until.to_rfc3339());
            }
        }
        sqlx::query(
            r#"
            INSERT INTO relay_admin_outbox (action_id, task_type, payload, dedup_key)
            VALUES ($1, 'affected_user_notice', $2, $3)
            ON CONFLICT (dedup_key) DO NOTHING
            "#,
        )
        .bind(action_id)
        .bind(affected_payload)
        .bind(format!("affected_user_notice:{action_str}"))
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(true)
}

/// Record a failure on an action, fenced by the caller's lease token. The report
/// remains 'processing' with active_action_id set. Only legal before
/// 'mutation_committed' step marker (post-mutation failures are delivery states,
/// not enforcement failures — handled separately).
///
/// The lease fence (`action_lease_token` match AND unexpired lease) prevents a
/// stale worker — one whose lease already expired and whose action was reclaimed
/// by a new owner — from marking the reclaimed action `failed`. Without it, that
/// late write races the new owner's fenced mutation: the mutation rolls back on
/// the ownership check and the report is stranded in `processing` with no live
/// action to drive it. Returns `true` iff a row was updated; `false` means the
/// lease was lost (log and stop — do not treat as a terminal failure).
pub async fn record_failure(
    pool: &PgPool,
    action_id: Uuid,
    lease_token: Uuid,
    error: &str,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE relay_admin_actions
        SET state = 'failed', error_message = $2, updated_at = now()
        WHERE id = $1 AND state = 'enforcing' AND step_marker IS NULL
          AND action_lease_token = $3
          AND action_lease_expires_at > now()
        "#,
    )
    .bind(action_id)
    .bind(error)
    .bind(lease_token)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Cancel a failed action (pre-mutation only) and return its report to 'open'.
///
/// This is one atomic, ownership-fenced transition: the action is cancelled
/// only if it is `failed`/pre-mutation AND belongs to the path `report_id` +
/// `community_id`, and the report is reopened only if it is still `processing`
/// and still points at this exact action. Both updates must each affect
/// exactly one row; any mismatch rolls the whole transaction back and returns
/// `false`.
///
/// Returns `false` (→ 409 at the HTTP layer, no state change) when the action
/// is not `failed`, has a `step_marker` (post-mutation cancel is forbidden),
/// does not belong to the path report/community (cross-report cancel), or the
/// report moved underneath the cancel.
pub async fn cancel_action(
    pool: &PgPool,
    action_id: Uuid,
    community_id: CommunityId,
    report_id: Uuid,
    cancelled_by: &[u8],
) -> Result<bool> {
    let mut tx = pool.begin().await?;

    // Cancel only a pre-mutation `failed` action that BELONGS to the path
    // report and community. Fencing on report_id + report_community_id is what
    // blocks cross-report cancellation: `/reports/A/cancel {actionId:B}` matches
    // zero rows because B's report_id is not A. `cancelled_by` attributes the
    // transition — the one mutation that would otherwise carry no actor trail.
    let updated = sqlx::query(
        r#"
        UPDATE relay_admin_actions
        SET state = 'cancelled', cancelled_by = $4, updated_at = now()
        WHERE id = $1
          AND report_id = $2
          AND report_community_id = $3
          AND state = 'failed'
          AND step_marker IS NULL
        "#,
    )
    .bind(action_id)
    .bind(report_id)
    .bind(community_id.as_uuid())
    .bind(cancelled_by)
    .execute(&mut *tx)
    .await?;

    if updated.rows_affected() != 1 {
        tx.rollback().await?;
        return Ok(false);
    }

    // Return the report to `open`, fenced on it still being `processing` and
    // still pointing at this exact action. Must affect exactly one row — any
    // mismatch means the report moved underneath us, so roll back the action
    // cancel too. This is what makes the handler's `"status":"open"` legitimate.
    let reopened = sqlx::query(
        r#"
        UPDATE moderation_reports
        SET status = 'open', active_action_id = NULL
        WHERE community_id = $1
          AND id = $2
          AND status = 'processing'
          AND active_action_id = $3
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(report_id)
    .bind(action_id)
    .execute(&mut *tx)
    .await?;

    if reopened.rows_affected() != 1 {
        tx.rollback().await?;
        return Ok(false);
    }

    tx.commit().await?;
    Ok(true)
}

/// Result of attempting to reopen a terminal report.
#[derive(Debug)]
pub enum ReopenResult {
    /// Report was terminal and is now `open`; a `reopen` audit row was inserted.
    Reopened,
    /// This exact `request_id` already reopened the report — idempotent replay.
    /// No state changed; the earlier reopen stands.
    AlreadyReopened,
    /// The report is not in a terminal state. Carries its current status.
    NotReopenable(String),
    /// The report was not found globally.
    NotFound,
}

/// Reopen a terminal report (`resolved | dismissed | escalated` → `open`) in a
/// single transaction, recording a durable `reopen` audit row.
///
/// The audit row is written to `relay_admin_actions` with `action = 'reopen'`
/// and `state = 'succeeded'`: `succeeded` keeps the stranded-action recovery
/// worker (which claims `state IN ('pending','enforcing')`) from ever driving
/// it, and the `action` value keeps it out of the enforcement DTO join (which
/// filters `action IN ('delete','kick','ban','timeout')`).
///
/// Idempotency is keyed on `request_id`: a replay after the report has been
/// reopened (and possibly re-resolved) returns [`ReopenResult::AlreadyReopened`]
/// without mutating, so a client network retry never re-reopens a
/// freshly-resolved report.
pub async fn reopen_report(
    pool: &PgPool,
    community_id: CommunityId,
    report_id: Uuid,
    request_id: Uuid,
    actor_pubkey: &[u8],
    actor_role: &str,
    reason: Option<&str>,
) -> Result<ReopenResult> {
    let mut tx = pool.begin().await?;

    // Lock the report row to serialize concurrent reopen/resolve on it.
    let report_row = sqlx::query(
        r#"
        SELECT status
        FROM moderation_reports
        WHERE community_id = $1 AND id = $2
        FOR UPDATE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(report_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(report_row) = report_row else {
        return Ok(ReopenResult::NotFound);
    };

    // Idempotent replay: this request_id already reopened the report. Checked
    // before the terminal-status gate so a retry after a re-resolve still
    // returns success rather than a spurious NotReopenable.
    let existing = sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT id FROM relay_admin_actions
        WHERE report_community_id = $1 AND report_id = $2
          AND request_id = $3 AND action = 'reopen'
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(report_id)
    .bind(request_id)
    .fetch_optional(&mut *tx)
    .await?;

    if existing.is_some() {
        tx.rollback().await?;
        return Ok(ReopenResult::AlreadyReopened);
    }

    let status: String = report_row.try_get("status")?;
    if !matches!(status.as_str(), "resolved" | "dismissed" | "escalated") {
        tx.rollback().await?;
        return Ok(ReopenResult::NotReopenable(status));
    }

    // Return the report to the queue. Clear the resolution stamp so an open
    // report never carries a stale resolver/timestamp; active_action_id is
    // already NULL on a terminal report but clear it defensively.
    sqlx::query(
        r#"
        UPDATE moderation_reports
        SET status = 'open', resolved_by = NULL, resolved_at = NULL,
            active_action_id = NULL
        WHERE community_id = $1 AND id = $2
          AND status IN ('resolved', 'dismissed', 'escalated')
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(report_id)
    .execute(&mut *tx)
    .await?;

    // Durable audit row. Inserted as 'succeeded' so the recovery worker never
    // claims it; 'reopen' keeps it out of the enforcement DTO join.
    sqlx::query(
        r#"
        INSERT INTO relay_admin_actions (
            report_id, report_community_id, request_id, actor_pubkey, actor_role,
            action, reason, state
        ) VALUES ($1, $2, $3, $4, $5, 'reopen', $6, 'succeeded')
        "#,
    )
    .bind(report_id)
    .bind(community_id.as_uuid())
    .bind(request_id)
    .bind(actor_pubkey)
    .bind(actor_role)
    .bind(reason)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(ReopenResult::Reopened)
}

/// Fetch an action record by ID.
pub async fn get_action(pool: &PgPool, action_id: Uuid) -> Result<Option<AdminActionRecord>> {
    let row = sqlx::query(
        r#"
        SELECT id, report_id, report_community_id, request_id, actor_pubkey, actor_role,
               action, reason, timeout_until, state, step_marker, cancelled_by, error_message,
               created_at, updated_at
        FROM relay_admin_actions WHERE id = $1
        "#,
    )
    .bind(action_id)
    .fetch_optional(pool)
    .await?;
    row.map(row_to_action).transpose()
}

/// Fetch an action record by report + request_id (idempotency lookup).
pub async fn get_action_by_request(
    pool: &PgPool,
    community_id: CommunityId,
    report_id: Uuid,
    request_id: Uuid,
) -> Result<Option<AdminActionRecord>> {
    let row = sqlx::query(
        r#"
        SELECT id, report_id, report_community_id, request_id, actor_pubkey, actor_role,
               action, reason, timeout_until, state, step_marker, cancelled_by, error_message,
               created_at, updated_at
        FROM relay_admin_actions
        WHERE report_community_id = $1 AND report_id = $2 AND request_id = $3
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(report_id)
    .bind(request_id)
    .fetch_optional(pool)
    .await?;
    row.map(row_to_action).transpose()
}

/// Insert an outbox command for artifact/notice delivery.
/// `dedup_key` prevents re-creating an artifact that was already delivered.
/// The INSERT is ON CONFLICT DO NOTHING so re-inserting on re-drive is a no-op.
pub async fn enqueue_outbox(
    pool: &PgPool,
    action_id: Uuid,
    task_type: &str,
    payload: serde_json::Value,
    dedup_key: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO relay_admin_outbox (action_id, task_type, payload, dedup_key)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (dedup_key) DO NOTHING
        "#,
    )
    .bind(action_id)
    .bind(task_type)
    .bind(payload)
    .bind(dedup_key)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark an outbox record as delivered, fenced by the claim token.
///
/// The update only succeeds if the caller still holds the claim token written
/// at claim time. Returns `true` if the row was updated, `false` if ownership
/// was already lost (lease expired and another worker reclaimed it).
pub async fn mark_outbox_delivered(
    pool: &PgPool,
    outbox_id: Uuid,
    claim_token: Uuid,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE relay_admin_outbox
        SET state = 'delivered', updated_at = now()
        WHERE id = $1
          AND outbox_claim_token = $2
          AND state = 'pending'
        "#,
    )
    .bind(outbox_id)
    .bind(claim_token)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Maximum number of delivery attempts before an outbox row is permanently failed.
pub const OUTBOX_MAX_ATTEMPTS: i32 = 5;

/// Record a delivery failure for an outbox row, fenced by the claim token.
///
/// Uses a single atomic `UPDATE … SET attempt_count = attempt_count + 1` — no
/// read-then-write, so concurrent updates cannot lose an increment. If the
/// incremented count reaches `OUTBOX_MAX_ATTEMPTS`, the row transitions to
/// terminal `failed`; otherwise it stays `pending` with exponential backoff.
///
/// Returns `true` if the row was updated (ownership still held), `false` if
/// the claim token no longer matches (ownership lost — stale worker must stop).
pub async fn fail_outbox_row(
    pool: &PgPool,
    outbox_id: Uuid,
    claim_token: Uuid,
    error: &str,
) -> Result<bool> {
    // One statement: increment attempt_count and derive backoff/terminal state.
    // The CASE expression mirrors the Rust logic that was previously read-then-write.
    let result = sqlx::query(
        r#"
        UPDATE relay_admin_outbox
        SET
            attempt_count  = attempt_count + 1,
            error_message  = $3,
            state          = CASE WHEN attempt_count + 1 >= $4 THEN 'failed' ELSE 'pending' END,
            retry_after    = CASE WHEN attempt_count + 1 >= $4 THEN NULL
                                  ELSE now() + (LEAST(POWER(2, attempt_count), 300) * INTERVAL '1 second')
                             END,
            held_by        = NULL,
            lease_expires_at = NULL,
            outbox_claim_token = NULL,
            updated_at     = now()
        WHERE id = $1
          AND outbox_claim_token = $2
          AND state = 'pending'
        "#,
    )
    .bind(outbox_id)
    .bind(claim_token)
    .bind(error)
    .bind(OUTBOX_MAX_ATTEMPTS)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Claim a batch of pending outbox rows using DB-level leases.
///
/// Atomically sets `held_by`, `lease_expires_at`, and a fresh `outbox_claim_token`
/// on up to `batch_size` rows whose lease is expired or unset AND whose
/// `retry_after` is past (or null), returning them for processing. The claim token
/// is the fencing token required by `mark_outbox_delivered` and `fail_outbox_row`.
pub async fn claim_pending_outbox_batch(
    pool: &PgPool,
    worker_id: &str,
    lease_until: DateTime<Utc>,
    batch_size: i64,
) -> Result<Vec<OutboxRecord>> {
    // Generate one fresh claim token per row via a VALUES list, same approach as
    // claim_stranded_action_batch.  Step 1: find candidates (SKIP LOCKED).
    let candidate_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"
        SELECT id FROM relay_admin_outbox
        WHERE state = 'pending'
          AND (lease_expires_at IS NULL OR lease_expires_at < now())
          AND (retry_after IS NULL OR retry_after <= now())
        -- NULLS FIRST is carried by this ORDER BY, not by the supporting index
        -- (idx_relay_admin_outbox_pending is plain ascending so the desired-state
        -- schema can match it via pgschema). Never-retried rows (retry_after IS
        -- NULL) are claimed before rescheduled ones; Postgres applies this
        -- ordering to the small pending candidate set regardless of index shape.
        ORDER BY retry_after NULLS FIRST, created_at ASC
        LIMIT $1
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(batch_size)
    .fetch_all(pool)
    .await?;

    if candidate_ids.is_empty() {
        return Ok(vec![]);
    }

    // Step 2: assign a unique token to each row via individual UPDATE statements.
    // Dynamic SQL (format!-built VALUES list) is rejected by the SqlSafeStr trait,
    // so we iterate. The FOR UPDATE SKIP LOCKED in step 1 ends with that SELECT
    // statement — the locks are not retained here. The UPDATE's own WHERE clause
    // (state = 'pending' AND lease_expires_at < now()) re-verifies ownership;
    // a concurrent pod that wins the race for the same row gets zero rows updated
    // and we skip it below.
    let mut records = Vec::with_capacity(candidate_ids.len());
    for id in candidate_ids {
        let token = Uuid::new_v4();
        let row = sqlx::query(
            r#"
            UPDATE relay_admin_outbox
            SET held_by = $2, lease_expires_at = $3,
                outbox_claim_token = $4, updated_at = now()
            WHERE id = $1
              AND state = 'pending'
              AND (lease_expires_at IS NULL OR lease_expires_at < now())
            RETURNING id, action_id, task_type, payload, state,
                      dedup_key, error_message, attempt_count, created_at,
                      outbox_claim_token
            "#,
        )
        .bind(id)
        .bind(worker_id)
        .bind(lease_until)
        .bind(token)
        .fetch_optional(pool)
        .await?;

        if let Some(row) = row {
            records.push(row_to_outbox_claimed(row)?);
        }
        // If the row was not found (race: another pod reclaimed between step 1 and 2),
        // skip it — no claim issued for that row.
    }
    Ok(records)
}

/// Fetch pending outbox records for a given action.
pub async fn list_pending_outbox(pool: &PgPool, action_id: Uuid) -> Result<Vec<OutboxRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, action_id, task_type, payload, state, dedup_key, error_message, attempt_count, created_at
        FROM relay_admin_outbox
        WHERE action_id = $1 AND state = 'pending'
        ORDER BY created_at ASC
        "#,
    )
    .bind(action_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_outbox).collect()
}

/// Claim a batch of stranded `relay_admin_actions` for the action recovery worker.
///
/// Claims `state IN ('pending', 'enforcing')` rows whose action lease has expired
/// or was never set. Each claimed row receives its own unique lease token so that
/// per-row lease fencing in `execute_*_with_marker` works correctly: all batch
/// items share the same expiry window, but each gets an independent token that
/// cannot be reused across rows.
pub async fn claim_stranded_action_batch(
    pool: &PgPool,
    _worker_id: &str,
    lease_until: DateTime<Utc>,
    batch_size: i64,
) -> Result<Vec<StrandedActionClaim>> {
    // Step 1: find candidate IDs (SKIP LOCKED prevents double-claim across pods).
    let candidate_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"
        SELECT id FROM relay_admin_actions
        WHERE state IN ('pending', 'enforcing')
          AND (action_lease_expires_at IS NULL OR action_lease_expires_at < now())
        ORDER BY created_at ASC
        LIMIT $1
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(batch_size)
    .fetch_all(pool)
    .await?;

    if candidate_ids.is_empty() {
        return Ok(vec![]);
    }

    // Step 2: assign a unique token to each row via individual UPDATE statements.
    // We cannot use a shared VALUES-list without dynamic SQL, so we iterate.
    // The FOR UPDATE SKIP LOCKED in step 1 ends with that SELECT statement —
    // the locks are not retained here. The UPDATE's own WHERE clause
    // (state IN ('pending','enforcing') AND lease_expires_at < now()) re-verifies
    // ownership; a concurrent pod that wins the same row gets zero rows updated
    // and we skip it below.
    let mut claims = Vec::with_capacity(candidate_ids.len());
    for id in candidate_ids {
        let token = Uuid::new_v4();
        let row = sqlx::query(
            r#"
            UPDATE relay_admin_actions
            SET action_lease_token = $2, action_lease_expires_at = $3, updated_at = now()
            WHERE id = $1
              AND state IN ('pending', 'enforcing')
              AND (action_lease_expires_at IS NULL OR action_lease_expires_at < now())
            RETURNING id, report_id, report_community_id, request_id, actor_pubkey,
                      actor_role, action, reason, timeout_until, state, step_marker,
                      cancelled_by, error_message, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(token)
        .bind(lease_until)
        .fetch_optional(pool)
        .await?;

        if let Some(row) = row {
            let record = row_to_action(row)?;
            claims.push(StrandedActionClaim {
                record,
                lease_token: token,
            });
        }
        // If the row was not found (race: another pod reclaimed between step 1 and 2),
        // skip it — no claim issued for that row.
    }
    Ok(claims)
}

/// New deployment-authority kick primitive: removes a member from a channel
/// without requiring the caller to be an active tenant owner/admin.
/// - `Ok(KickResult::Removed)` — member was active and is now removed.
/// - `Ok(KickResult::AlreadyGone)` — member was already absent before this action.
///   (The enforcement mutation landed; the member was simply not there.)
/// - `Err(_)` — unexpected DB error.
///
/// Never blanket-converts "not found" to success — callers must distinguish
/// `AlreadyGone` (expected idempotency) from `Removed` (new removal).
#[derive(Debug, PartialEq, Eq)]
pub enum KickResult {
    /// Member was present and is now removed.
    Removed,
    /// Member was not present (already removed or never joined).
    AlreadyGone,
}

/// Remove a member using deployment authority (no tenant owner/admin check).
///
/// Returns `KickResult::Removed` if the member was present,
/// `KickResult::AlreadyGone` if already absent.
pub async fn deploy_kick_member(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    target_pubkey: &[u8],
    actor_pubkey: &[u8],
) -> Result<KickResult> {
    // Use a direct UPDATE to avoid the tenant ownership check in channel::remove_member.
    // This is the deployment-authority primitive: no actor role check.
    let result = sqlx::query(
        r#"
        UPDATE channel_members
        SET removed_at = NOW(), removed_by = $1
        WHERE community_id = $2 AND channel_id = $3 AND pubkey = $4 AND removed_at IS NULL
        "#,
    )
    .bind(actor_pubkey)
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(target_pubkey)
    .execute(pool)
    .await?;

    if result.rows_affected() > 0 {
        Ok(KickResult::Removed)
    } else {
        Ok(KickResult::AlreadyGone)
    }
}

/// Update product_feedback status.
pub async fn update_feedback_status(pool: &PgPool, id: Uuid, status: &str) -> Result<bool> {
    let result = sqlx::query("UPDATE product_feedback SET status = $2 WHERE id = $1")
        .bind(id)
        .bind(status)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

fn row_to_action(row: sqlx::postgres::PgRow) -> Result<AdminActionRecord> {
    Ok(AdminActionRecord {
        id: row.try_get("id")?,
        report_id: row.try_get("report_id")?,
        report_community_id: row.try_get("report_community_id")?,
        request_id: row.try_get("request_id")?,
        actor_pubkey: row.try_get("actor_pubkey")?,
        actor_role: row.try_get("actor_role")?,
        action: row.try_get("action")?,
        reason: row.try_get("reason")?,
        timeout_until: row.try_get("timeout_until")?,
        state: row.try_get("state")?,
        step_marker: row.try_get("step_marker")?,
        cancelled_by: row.try_get("cancelled_by")?,
        error_message: row.try_get("error_message")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn row_to_outbox(row: sqlx::postgres::PgRow) -> Result<OutboxRecord> {
    Ok(OutboxRecord {
        id: row.try_get("id")?,
        action_id: row.try_get("action_id")?,
        task_type: row.try_get("task_type")?,
        payload: row.try_get("payload")?,
        state: row.try_get("state")?,
        dedup_key: row.try_get("dedup_key")?,
        error_message: row.try_get("error_message")?,
        attempt_count: row.try_get("attempt_count").unwrap_or(0),
        // For non-claim queries (e.g. list_pending_outbox), there is no claim token.
        claim_token: Uuid::nil(),
        created_at: row.try_get("created_at").unwrap_or_else(|_| Utc::now()),
    })
}

/// Decode a row returned by `claim_pending_outbox_batch` — includes the claim token.
fn row_to_outbox_claimed(row: sqlx::postgres::PgRow) -> Result<OutboxRecord> {
    Ok(OutboxRecord {
        id: row.try_get("id")?,
        action_id: row.try_get("action_id")?,
        task_type: row.try_get("task_type")?,
        payload: row.try_get("payload")?,
        state: row.try_get("state")?,
        dedup_key: row.try_get("dedup_key")?,
        error_message: row.try_get("error_message")?,
        attempt_count: row.try_get("attempt_count").unwrap_or(0),
        claim_token: row.try_get("outbox_claim_token")?,
        created_at: row.try_get("created_at").unwrap_or_else(|_| Utc::now()),
    })
}

impl crate::Db {
    /// Atomic decision-only report closure: CAS open→terminal + audit row in one transaction.
    #[allow(clippy::too_many_arguments)]
    #[datastore_span(name = "resolve_report_decision_atomic", system = "postgresql")]
    pub async fn resolve_report_decision_atomic(
        &self,
        community_id: CommunityId,
        report_id: uuid::Uuid,
        terminal_status: &str,
        audit_action: &str,
        actor_pubkey: &[u8],
        actor_authority: &str,
        target_pubkey: Option<&[u8]>,
        target_event_id: Option<&[u8]>,
        channel_id: Option<uuid::Uuid>,
        reason: Option<&str>,
    ) -> Result<bool> {
        resolve_report_decision_atomic(
            &self.pool,
            community_id,
            report_id,
            terminal_status,
            audit_action,
            actor_pubkey,
            actor_authority,
            target_pubkey,
            target_event_id,
            channel_id,
            reason,
        )
        .await
    }

    /// Attempt to claim a report for HTTP enforcement (CAS open → processing).
    #[allow(clippy::too_many_arguments)]
    #[datastore_span(name = "claim_report_for_enforcement", system = "postgresql")]
    pub async fn claim_report_for_enforcement(
        &self,
        community_id: CommunityId,
        report_id: uuid::Uuid,
        request_id: uuid::Uuid,
        actor_pubkey: &[u8],
        actor_role: &str,
        action: &str,
        reason: Option<&str>,
        timeout_until: Option<chrono::DateTime<chrono::Utc>>,
        audit_action: &str,
        actor_authority: &str,
        target_pubkey: Option<&[u8]>,
        target_event_id: Option<&[u8]>,
        channel_id: Option<uuid::Uuid>,
    ) -> Result<ClaimResult> {
        claim_report(
            &self.pool,
            community_id,
            report_id,
            request_id,
            actor_pubkey,
            actor_role,
            action,
            reason,
            timeout_until,
            audit_action,
            actor_authority,
            target_pubkey,
            target_event_id,
            channel_id,
        )
        .await
    }

    /// Advance an action from 'pending' to 'enforcing'.
    #[datastore_span(name = "begin_enforcing_action", system = "postgresql")]
    pub async fn begin_enforcing_action(&self, action_id: uuid::Uuid) -> Result<bool> {
        begin_enforcing(&self.pool, action_id).await
    }

    /// Commit the core mutation step (advance step_marker to 'mutation_committed').
    #[datastore_span(name = "commit_action_mutation_step", system = "postgresql")]
    pub async fn commit_action_mutation_step(&self, action_id: uuid::Uuid) -> Result<bool> {
        commit_mutation_step(&self.pool, action_id).await
    }

    /// Finalize enforcement: action → succeeded, report → terminal status,
    /// and enqueue outbox delivery rows atomically.
    #[allow(clippy::too_many_arguments)]
    #[datastore_span(name = "finalize_action_success", system = "postgresql")]
    pub async fn finalize_action_success(
        &self,
        action_id: uuid::Uuid,
        community_id: CommunityId,
        report_id: uuid::Uuid,
        terminal_status: &str,
        actor_pubkey: &[u8],
        action_name: &str,
        target_pubkey: Option<&[u8]>,
        target_event_id: Option<&[u8]>,
        channel_id: Option<uuid::Uuid>,
        reason: Option<&str>,
        timeout_until: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<bool> {
        finalize_success(
            &self.pool,
            action_id,
            community_id,
            report_id,
            terminal_status,
            actor_pubkey,
            action_name,
            target_pubkey,
            target_event_id,
            channel_id,
            reason,
            timeout_until,
        )
        .await
    }

    /// Atomically execute a ban mutation and commit the step marker.
    /// Returns `true` if this driver committed the marker, `false` if already marked or lease lost.
    #[datastore_span(name = "execute_ban_with_marker", system = "postgresql")]
    pub async fn execute_ban_with_marker(
        &self,
        action_id: uuid::Uuid,
        lease_token: uuid::Uuid,
        community_id: CommunityId,
        target_pubkey: &[u8],
        actor_pubkey: &[u8],
        reason: Option<&str>,
    ) -> Result<bool> {
        execute_ban_with_marker(
            &self.pool,
            action_id,
            lease_token,
            community_id,
            target_pubkey,
            actor_pubkey,
            reason,
        )
        .await
    }

    /// Atomically execute a timeout mutation and commit the step marker.
    /// Returns `true` if this driver committed the marker, `false` if already marked or lease lost.
    #[allow(clippy::too_many_arguments)]
    #[datastore_span(name = "execute_timeout_with_marker", system = "postgresql")]
    pub async fn execute_timeout_with_marker(
        &self,
        action_id: uuid::Uuid,
        lease_token: uuid::Uuid,
        community_id: CommunityId,
        target_pubkey: &[u8],
        actor_pubkey: &[u8],
        until: chrono::DateTime<chrono::Utc>,
        reason: Option<&str>,
    ) -> Result<bool> {
        execute_timeout_with_marker(
            &self.pool,
            action_id,
            lease_token,
            community_id,
            target_pubkey,
            actor_pubkey,
            until,
            reason,
        )
        .await
    }

    /// Atomically execute a kick mutation and commit the step marker.
    /// Returns `Removed` (member was present), `AlreadyGone` (absent before this action),
    /// or `AlreadyMarked` (marker already committed by another driver or lease lost).
    #[datastore_span(name = "execute_kick_with_marker", system = "postgresql")]
    pub async fn execute_kick_with_marker(
        &self,
        action_id: uuid::Uuid,
        lease_token: uuid::Uuid,
        community_id: CommunityId,
        channel_id: uuid::Uuid,
        target_pubkey: &[u8],
        actor_pubkey: &[u8],
    ) -> Result<KickWithMarkerResult> {
        execute_kick_with_marker(
            &self.pool,
            action_id,
            lease_token,
            community_id,
            channel_id,
            target_pubkey,
            actor_pubkey,
        )
        .await
    }

    /// Atomically execute a soft-delete mutation and commit the step marker.
    /// Returns `true` if this driver committed the marker, `false` if already marked or lease lost.
    #[datastore_span(name = "execute_delete_with_marker", system = "postgresql")]
    pub async fn execute_delete_with_marker(
        &self,
        action_id: uuid::Uuid,
        lease_token: uuid::Uuid,
        community_id: CommunityId,
        target_event_id: &[u8],
        parent_event_id: Option<&[u8]>,
        root_event_id: Option<&[u8]>,
    ) -> Result<bool> {
        execute_delete_with_marker(
            &self.pool,
            action_id,
            lease_token,
            community_id,
            target_event_id,
            parent_event_id,
            root_event_id,
        )
        .await
    }

    /// Acquire the action mutation lease (prevents concurrent double-mutation).
    #[datastore_span(name = "acquire_admin_action_lease", system = "postgresql")]
    pub async fn acquire_admin_action_lease(
        &self,
        action_id: uuid::Uuid,
        lease_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<LeaseResult> {
        acquire_action_lease(&self.pool, action_id, lease_until).await
    }

    /// Release the action mutation lease. No-op if caller no longer holds the token.
    #[datastore_span(name = "release_admin_action_lease", system = "postgresql")]
    pub async fn release_admin_action_lease(
        &self,
        action_id: uuid::Uuid,
        lease_token: uuid::Uuid,
    ) -> Result<()> {
        release_action_lease(&self.pool, action_id, lease_token).await
    }

    /// Claim a batch of stranded `relay_admin_actions` for the action recovery worker.
    #[datastore_span(name = "claim_stranded_admin_action_batch", system = "postgresql")]
    pub async fn claim_stranded_admin_action_batch(
        &self,
        worker_id: &str,
        lease_until: chrono::DateTime<chrono::Utc>,
        batch_size: i64,
    ) -> Result<Vec<StrandedActionClaim>> {
        claim_stranded_action_batch(&self.pool, worker_id, lease_until, batch_size).await
    }

    /// Record a pre-mutation enforcement failure (keeps report in 'processing').
    #[datastore_span(name = "record_action_failure", system = "postgresql")]
    pub async fn record_action_failure(
        &self,
        action_id: uuid::Uuid,
        lease_token: uuid::Uuid,
        error: &str,
    ) -> Result<bool> {
        record_failure(&self.pool, action_id, lease_token, error).await
    }

    /// Cancel a pre-mutation failed action (returns report to 'open'),
    /// attributing the cancel to `cancelled_by`.
    #[datastore_span(name = "cancel_admin_action", system = "postgresql")]
    pub async fn cancel_admin_action(
        &self,
        action_id: uuid::Uuid,
        community_id: CommunityId,
        report_id: uuid::Uuid,
        cancelled_by: &[u8],
    ) -> Result<bool> {
        cancel_action(&self.pool, action_id, community_id, report_id, cancelled_by).await
    }

    /// Reopen a terminal report (resolved|dismissed|escalated → open) with a
    /// durable `reopen` audit row, keyed idempotent on `request_id`.
    #[datastore_span(name = "reopen_report", system = "postgresql")]
    pub async fn reopen_report(
        &self,
        community_id: CommunityId,
        report_id: uuid::Uuid,
        request_id: uuid::Uuid,
        actor_pubkey: &[u8],
        actor_role: &str,
        reason: Option<&str>,
    ) -> Result<ReopenResult> {
        reopen_report(
            &self.pool,
            community_id,
            report_id,
            request_id,
            actor_pubkey,
            actor_role,
            reason,
        )
        .await
    }

    /// Fetch an action record by ID.
    #[datastore_span(name = "get_admin_action", system = "postgresql")]
    pub async fn get_admin_action(
        &self,
        action_id: uuid::Uuid,
    ) -> Result<Option<AdminActionRecord>> {
        get_action(&self.pool, action_id).await
    }

    /// Enqueue an outbox artifact/notice delivery command.
    #[datastore_span(name = "enqueue_admin_outbox", system = "postgresql")]
    pub async fn enqueue_admin_outbox(
        &self,
        action_id: uuid::Uuid,
        task_type: &str,
        payload: serde_json::Value,
        dedup_key: &str,
    ) -> Result<()> {
        enqueue_outbox(&self.pool, action_id, task_type, payload, dedup_key).await
    }

    /// Mark an outbox record as delivered, fenced by the claim token.
    /// Returns `true` if updated, `false` if ownership was already lost.
    #[datastore_span(name = "mark_admin_outbox_delivered", system = "postgresql")]
    pub async fn mark_admin_outbox_delivered(
        &self,
        outbox_id: uuid::Uuid,
        claim_token: uuid::Uuid,
    ) -> Result<bool> {
        mark_outbox_delivered(&self.pool, outbox_id, claim_token).await
    }

    /// Mark an outbox record as failed, fenced by the claim token.
    /// Returns `true` if updated, `false` if ownership was already lost.
    #[datastore_span(name = "fail_admin_outbox_row", system = "postgresql")]
    pub async fn fail_admin_outbox_row(
        &self,
        outbox_id: uuid::Uuid,
        claim_token: uuid::Uuid,
        error: &str,
    ) -> Result<bool> {
        fail_outbox_row(&self.pool, outbox_id, claim_token, error).await
    }

    /// Claim a batch of pending outbox rows for the given worker pod.
    #[datastore_span(name = "claim_pending_admin_outbox_batch", system = "postgresql")]
    pub async fn claim_pending_admin_outbox_batch(
        &self,
        worker_id: &str,
        lease_until: chrono::DateTime<chrono::Utc>,
        batch_size: i64,
    ) -> Result<Vec<OutboxRecord>> {
        claim_pending_outbox_batch(&self.pool, worker_id, lease_until, batch_size).await
    }

    /// List pending outbox records for an action.
    #[datastore_span(name = "list_pending_admin_outbox", system = "postgresql")]
    pub async fn list_pending_admin_outbox(
        &self,
        action_id: uuid::Uuid,
    ) -> Result<Vec<OutboxRecord>> {
        list_pending_outbox(&self.pool, action_id).await
    }

    /// Deployment-authority kick: remove a member without requiring tenant owner/admin actor.
    #[datastore_span(name = "deploy_kick_member", system = "postgresql")]
    pub async fn deploy_kick_member(
        &self,
        community_id: CommunityId,
        channel_id: uuid::Uuid,
        target_pubkey: &[u8],
        actor_pubkey: &[u8],
    ) -> Result<KickResult> {
        deploy_kick_member(
            &self.pool,
            community_id,
            channel_id,
            target_pubkey,
            actor_pubkey,
        )
        .await
    }

    /// Update product_feedback status (operator-managed lifecycle).
    #[datastore_span(name = "update_feedback_status", system = "postgresql")]
    pub async fn update_feedback_status(&self, id: uuid::Uuid, status: &str) -> Result<bool> {
        update_feedback_status(&self.pool, id, status).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;

    use sqlx::PgPool;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    async fn setup_pool() -> PgPool {
        let url =
            std::env::var("BUZZ_TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.to_string());
        PgPool::connect(&url).await.expect("connect to test DB")
    }

    async fn make_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        let host = format!("admin-action-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert community");
        id
    }

    async fn make_report(pool: &PgPool, community_id: Uuid) -> Uuid {
        let reporter = vec![0u8; 32];
        let target = vec![1u8; 32];
        // report_event_id requires exactly 32 bytes (Nostr event ID).
        // Use the two UUID halves concatenated to produce a unique 32-byte value.
        let uid = Uuid::new_v4();
        let event_id: Vec<u8> = uid
            .as_bytes()
            .iter()
            .chain(uid.as_bytes().iter())
            .copied()
            .collect();
        let row = sqlx::query(
            r#"
            INSERT INTO moderation_reports (
                community_id, report_event_id, reporter_pubkey, target_kind,
                target_pubkey, report_type
            ) VALUES ($1, $2, $3, 'pubkey', $4, 'harassment')
            RETURNING id
            "#,
        )
        .bind(community_id)
        .bind(event_id)
        .bind(&reporter)
        .bind(&target)
        .fetch_one(pool)
        .await
        .expect("insert report");
        row.try_get("id").expect("id")
    }

    fn actor() -> Vec<u8> {
        vec![2u8; 32]
    }

    // Helper: perform a full claim call.
    async fn do_claim(
        pool: &PgPool,
        community_id: Uuid,
        report_id: Uuid,
        request_id: Uuid,
    ) -> ClaimResult {
        let actor = actor();
        let target = vec![1u8; 32];
        claim_report(
            pool,
            CommunityId::from_uuid(community_id),
            report_id,
            request_id,
            &actor,
            "operator",
            "ban",
            Some("test reason"),
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("claim_report")
    }

    // Helper: call finalize_success with the new full signature for a ban action.
    async fn do_finalize(
        pool: &PgPool,
        action_id: Uuid,
        community_id: Uuid,
        report_id: Uuid,
    ) -> bool {
        let actor = actor();
        let target = vec![1u8; 32];
        finalize_success(
            pool,
            action_id,
            CommunityId::from_uuid(community_id),
            report_id,
            "resolved",
            &actor,
            "ban",
            Some(&target),
            None,
            None,
            Some("test reason"),
            None,
        )
        .await
        .expect("finalize_success")
    }

    // ── Racing moderators ─────────────────────────────────────────────────────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn racing_moderators_exactly_one_claim_one_conflict() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        // Two concurrent claims with different request_ids.
        let req_a = Uuid::new_v4();
        let req_b = Uuid::new_v4();

        let (result_a, result_b) = tokio::join!(
            do_claim(&pool, community_id, report_id, req_a),
            do_claim(&pool, community_id, report_id, req_b),
        );

        // Exactly one should succeed; the other gets NotOpen.
        let (claimed, conflicted) = match (&result_a, &result_b) {
            (ClaimResult::Claimed(_), ClaimResult::NotOpen(_)) => (result_a, result_b),
            (ClaimResult::NotOpen(_), ClaimResult::Claimed(_)) => (result_b, result_a),
            other => panic!("expected one claim + one conflict, got: {other:?}"),
        };

        let action_id = match claimed {
            ClaimResult::Claimed(ref a) => a.id,
            _ => unreachable!(),
        };
        _ = action_id;
        _ = conflicted;

        // No orphan audit rows: exactly one moderation_actions row for this report.
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM moderation_actions WHERE community_id = $1")
                .bind(community_id)
                .fetch_one(&pool)
                .await
                .expect("count audit rows");
        assert_eq!(count, 1, "expected exactly one audit row");
    }

    // ── Same request_id idempotent retry ──────────────────────────────────────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn same_request_id_retry_returns_same_action_id() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        let request_id = Uuid::new_v4();

        let first = do_claim(&pool, community_id, report_id, request_id).await;
        let first_action = match first {
            ClaimResult::Claimed(a) => a,
            other => panic!("expected Claimed, got {other:?}"),
        };

        // Retry with the same request_id.
        let second = do_claim(&pool, community_id, report_id, request_id).await;
        let second_action = match second {
            ClaimResult::AlreadyClaimed(a) => a,
            other => panic!("expected AlreadyClaimed on retry, got {other:?}"),
        };

        assert_eq!(
            first_action.id, second_action.id,
            "idempotent retry must return the same action id"
        );
    }

    // ── Different request_id against processing report → conflict ─────────────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn different_request_id_against_processing_report_returns_conflict() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        // First claim succeeds.
        let _first = do_claim(&pool, community_id, report_id, Uuid::new_v4()).await;

        // Second claim with a different request_id must fail.
        let second = do_claim(&pool, community_id, report_id, Uuid::new_v4()).await;
        assert!(
            matches!(second, ClaimResult::NotOpen(_)),
            "expected NotOpen for different request_id against processing report"
        );
    }

    // ── Mutation + step_marker atomicity: cancel rejected post-marker ─────────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn cancel_after_mutation_committed_is_rejected() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        let request_id = Uuid::new_v4();
        let claimed = match do_claim(&pool, community_id, report_id, request_id).await {
            ClaimResult::Claimed(a) => a,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let action_id = claimed.id;

        // Advance to enforcing.
        let advanced = begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");
        assert!(advanced);

        // Commit the mutation step marker.
        let committed = commit_mutation_step(&pool, action_id)
            .await
            .expect("commit_mutation_step");
        assert!(committed);

        // Attempt to cancel — must fail because step_marker is set.
        let cancelled = cancel_action(
            &pool,
            action_id,
            CommunityId::from_uuid(community_id),
            report_id,
            &[0_u8; 32],
        )
        .await
        .expect("cancel_action");
        assert!(
            !cancelled,
            "cancel after mutation_committed must be rejected"
        );
    }

    // ── Crash re-drive: step_marker skips the mutation, finalize succeeds ─────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn crash_redrive_with_mutation_committed_skips_to_finalization() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        let request_id = Uuid::new_v4();
        let claimed = match do_claim(&pool, community_id, report_id, request_id).await {
            ClaimResult::Claimed(a) => a,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let action_id = claimed.id;

        // Simulate: process advanced, mutation committed, crash before finalization.
        let _ = begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");
        let _ = commit_mutation_step(&pool, action_id)
            .await
            .expect("commit_mutation_step");

        // Re-load the record (simulates crash recovery).
        let reloaded = get_action(&pool, action_id)
            .await
            .expect("get_action")
            .expect("action exists");

        // step_marker is set — re-drive should skip mutation and go to finalize.
        assert_eq!(reloaded.step_marker.as_deref(), Some("mutation_committed"));

        // Finalize succeeds (proves re-drive transitions from persisted marker).
        let finalized = do_finalize(&pool, action_id, community_id, report_id).await;
        assert!(
            finalized,
            "finalize_success must succeed from mutation_committed state"
        );

        // Report must be resolved.
        let row: Option<String> =
            sqlx::query_scalar("SELECT status FROM moderation_reports WHERE id = $1")
                .bind(report_id)
                .fetch_optional(&pool)
                .await
                .expect("fetch report");
        assert_eq!(row.as_deref(), Some("resolved"));

        // Outbox rows must exist in the finalization transaction (success-gated delivery).
        let outbox_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM relay_admin_outbox WHERE action_id = $1")
                .bind(action_id)
                .fetch_one(&pool)
                .await
                .expect("count outbox");
        assert!(outbox_count > 0, "finalize_success must create outbox rows");
    }

    // ── Decision-only atomicity: no orphan audit row on concurrent close ───────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn decision_only_concurrent_close_no_orphan_audit() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;
        let actor = actor();
        let target = vec![1u8; 32];
        let cid = CommunityId::from_uuid(community_id);

        // First close succeeds.
        let first = resolve_report_decision_atomic(
            &pool,
            cid,
            report_id,
            "dismissed",
            "dismiss_report",
            &actor,
            "relay_operator",
            Some(&target),
            None,
            None,
            None,
        )
        .await
        .expect("first close");
        assert!(first, "first close must succeed");

        // Concurrent close on already-closed report must fail.
        let second = resolve_report_decision_atomic(
            &pool,
            cid,
            report_id,
            "dismissed",
            "dismiss_report",
            &actor,
            "relay_operator",
            Some(&target),
            None,
            None,
            None,
        )
        .await
        .expect("second close");
        assert!(!second, "second close on non-open report must fail");

        // Exactly one audit row.
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM moderation_actions WHERE community_id = $1")
                .bind(community_id)
                .fetch_one(&pool)
                .await
                .expect("count audit rows");
        assert_eq!(count, 1, "no orphan audit row on concurrent close");
    }

    // ── Outbox rows created in finalize_success, NOT at claim time ────────────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn outbox_rows_created_at_finalize_not_at_claim() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        let request_id = Uuid::new_v4();
        let claimed = match do_claim(&pool, community_id, report_id, request_id).await {
            ClaimResult::Claimed(a) => a,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let action_id = claimed.id;

        // Immediately after claim: NO outbox rows — delivery is success-gated.
        let rows_at_claim = list_pending_outbox(&pool, action_id)
            .await
            .expect("list_pending_outbox at claim");
        assert!(
            rows_at_claim.is_empty(),
            "claim must NOT insert outbox rows (success-gated); got: {rows_at_claim:?}"
        );

        // Advance to enforcing + commit marker.
        let _ = begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");
        let _ = commit_mutation_step(&pool, action_id)
            .await
            .expect("commit_mutation_step");

        // After finalize_success: outbox rows must exist.
        let finalized = do_finalize(&pool, action_id, community_id, report_id).await;
        assert!(finalized, "finalize_success must succeed");

        let rows_after = list_pending_outbox(&pool, action_id)
            .await
            .expect("list_pending_outbox after finalize");
        assert!(
            rows_after.iter().any(|r| r.task_type == "reporter_notice"),
            "finalize_success must create reporter_notice outbox row; got: {rows_after:?}"
        );
    }

    // ── Affected-user notice: actioned user hears the truth ───────────────────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn finalize_enqueues_affected_user_notice_for_restriction() {
        // A `ban` must enqueue an `affected_user_notice` addressed to the target
        // pubkey, carrying the operator-authored public reason and restriction
        // kind — so the
        // restricted user is told what happened (VISION_MODERATION).
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        let claimed = match do_claim(&pool, community_id, report_id, Uuid::new_v4()).await {
            ClaimResult::Claimed(a) => a,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let action_id = claimed.id;
        let _ = begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");
        let _ = commit_mutation_step(&pool, action_id)
            .await
            .expect("commit_mutation_step");

        // do_finalize uses action "ban", target [1u8; 32], reason "test reason".
        assert!(do_finalize(&pool, action_id, community_id, report_id).await);

        let rows = list_pending_outbox(&pool, action_id)
            .await
            .expect("list_pending_outbox");
        let notice = rows
            .iter()
            .find(|r| r.task_type == "affected_user_notice")
            .expect("finalize_success must enqueue an affected_user_notice for ban");
        assert_eq!(
            notice.payload["recipient"].as_str(),
            Some(hex::encode([1u8; 32]).as_str()),
            "notice must be addressed to the actioned target pubkey"
        );
        assert_eq!(notice.payload["notice_kind"].as_str(), Some("restriction"));
        assert_eq!(notice.payload["restriction_kind"].as_str(), Some("ban"));
        assert_eq!(
            notice.payload["public_reason"].as_str(),
            Some("test reason")
        );
        // A ban is indefinite: no timeout_until in the payload.
        assert!(
            notice.payload.get("timeout_until").is_none(),
            "ban notice must not carry a timeout_until; got: {:?}",
            notice.payload
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn finalize_carries_timeout_until_for_timeout_notice() {
        // A `timeout` must enqueue an `affected_user_notice` carrying the
        // authoritative expiry so the restricted user is told "for how long"
        // (VISION_MODERATION: "what restriction was applied, why, and for how long").
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        let claimed = match do_claim(&pool, community_id, report_id, Uuid::new_v4()).await {
            ClaimResult::Claimed(a) => a,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let action_id = claimed.id;
        let _ = begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");
        let _ = commit_mutation_step(&pool, action_id)
            .await
            .expect("commit_mutation_step");

        let target = vec![1u8; 32];
        let until = chrono::DateTime::parse_from_rfc3339("2026-09-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let finalized = finalize_success(
            &pool,
            action_id,
            CommunityId::from_uuid(community_id),
            report_id,
            "resolved",
            &actor(),
            "timeout",
            Some(&target),
            None,
            None,
            Some("Cool off."),
            Some(until),
        )
        .await
        .expect("finalize_success");
        assert!(finalized);

        let rows = list_pending_outbox(&pool, action_id)
            .await
            .expect("list_pending_outbox");
        let notice = rows
            .iter()
            .find(|r| r.task_type == "affected_user_notice")
            .expect("timeout must enqueue an affected_user_notice");
        assert_eq!(notice.payload["notice_kind"].as_str(), Some("restriction"));
        assert_eq!(notice.payload["restriction_kind"].as_str(), Some("timeout"));
        assert_eq!(
            notice.payload["timeout_until"].as_str(),
            Some(until.to_rfc3339().as_str()),
            "timeout notice payload must carry the authoritative expiry"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn finalize_maps_delete_and_kick_to_content_actioned_notice() {
        // The four-action mapping: delete/kick → content_actioned. (ban/timeout →
        // restriction are covered by the restriction tests above.) Both `delete`
        // and `kick` are finalized against a target pubkey so the affected user is
        // notified; removing EITHER mapping arm fails this test.
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;

        // Finalize one action per verb on its own report (the affected_user_notice
        // dedup_key is per-action) and assert each enqueues a content_actioned
        // notice with no restriction fields.
        for verb in ["delete", "kick"] {
            let report_id = make_report(&pool, community_id).await;
            let claimed = match do_claim(&pool, community_id, report_id, Uuid::new_v4()).await {
                ClaimResult::Claimed(a) => a,
                other => panic!("expected Claimed, got {other:?}"),
            };
            let action_id = claimed.id;
            let _ = begin_enforcing(&pool, action_id)
                .await
                .expect("begin_enforcing");
            let _ = commit_mutation_step(&pool, action_id)
                .await
                .expect("commit_mutation_step");

            let target = vec![1u8; 32];
            let channel_id = Uuid::new_v4();
            let finalized = finalize_success(
                &pool,
                action_id,
                CommunityId::from_uuid(community_id),
                report_id,
                "resolved",
                &actor(),
                verb,
                Some(&target),
                None,
                Some(channel_id),
                Some("Off-topic."),
                None,
            )
            .await
            .expect("finalize_success");
            assert!(finalized);

            let rows = list_pending_outbox(&pool, action_id)
                .await
                .expect("list_pending_outbox");
            let notice = rows
                .iter()
                .find(|r| r.task_type == "affected_user_notice")
                .unwrap_or_else(|| panic!("{verb} must enqueue an affected_user_notice"));
            assert_eq!(
                notice.payload["notice_kind"].as_str(),
                Some("content_actioned"),
                "{verb} maps to content_actioned"
            );
            assert!(
                notice.payload.get("restriction_kind").is_none(),
                "content_actioned notice carries no restriction_kind"
            );
            assert!(
                notice.payload.get("timeout_until").is_none(),
                "content_actioned notice carries no timeout_until"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn finalize_skips_affected_user_notice_when_no_target_pubkey() {
        // A `delete` with no derivable author (purged event) has no one to
        // notify: no affected_user_notice row is enqueued, but the reporter
        // notice still is.
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        let claimed = match do_claim(&pool, community_id, report_id, Uuid::new_v4()).await {
            ClaimResult::Claimed(a) => a,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let action_id = claimed.id;
        let _ = begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");
        let _ = commit_mutation_step(&pool, action_id)
            .await
            .expect("commit_mutation_step");

        // Finalize a `delete` with target_pubkey = None.
        let finalized = finalize_success(
            &pool,
            action_id,
            CommunityId::from_uuid(community_id),
            report_id,
            "resolved",
            &actor(),
            "delete",
            None,
            None,
            None,
            Some("test reason"),
            None,
        )
        .await
        .expect("finalize_success");
        assert!(finalized);

        let rows = list_pending_outbox(&pool, action_id)
            .await
            .expect("list_pending_outbox");
        assert!(
            rows.iter().any(|r| r.task_type == "reporter_notice"),
            "reporter notice must still be enqueued"
        );
        assert!(
            !rows.iter().any(|r| r.task_type == "affected_user_notice"),
            "no affected_user_notice when there is no target pubkey; got: {rows:?}"
        );
    }

    // ── record_failure lease fence: stale worker cannot strand the report ─────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn record_failure_is_a_no_op_after_lease_reclaim() {
        // Worker A leases the action, its lease expires, worker B reclaims it,
        // then A's late failure write must be a no-op (0 rows) rather than
        // marking the reclaimed action `failed` and stranding the report.
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        let claimed = match do_claim(&pool, community_id, report_id, Uuid::new_v4()).await {
            ClaimResult::Claimed(a) => a,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let action_id = claimed.id;
        let _ = begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");

        // Worker A leases with an ALREADY-EXPIRED expiry (simulates lease loss).
        let expired = chrono::Utc::now() - chrono::Duration::seconds(1);
        let token_a = match acquire_action_lease(&pool, action_id, expired)
            .await
            .expect("acquire A")
        {
            LeaseResult::Acquired(t) => t,
            other => panic!("expected Acquired, got {other:?}"),
        };

        // Worker B reclaims (A's lease is expired, so this succeeds).
        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(60);
        let token_b = match acquire_action_lease(&pool, action_id, lease_until)
            .await
            .expect("acquire B")
        {
            LeaseResult::Acquired(t) => t,
            other => panic!("expected Acquired for B, got {other:?}"),
        };
        assert_ne!(token_a, token_b);

        // A's late failure write must be a no-op.
        let a_wrote = record_failure(&pool, action_id, token_a, "A late failure")
            .await
            .expect("record_failure A");
        assert!(!a_wrote, "stale worker A must not record the failure");

        let rec = get_action(&pool, action_id)
            .await
            .expect("get_action")
            .expect("action exists");
        assert_eq!(
            rec.state, "enforcing",
            "action must remain enforcing (owned by B), not failed"
        );

        // B, holding the live lease, can record a failure.
        let b_wrote = record_failure(&pool, action_id, token_b, "B failure")
            .await
            .expect("record_failure B");
        assert!(b_wrote, "live owner B must be able to record the failure");
        let rec = get_action(&pool, action_id)
            .await
            .expect("get_action")
            .expect("action exists");
        assert_eq!(rec.state, "failed");
    }

    // ── Finalize fences: requires step_marker + active_action_id ─────────────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn finalize_without_step_marker_is_rejected() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        let request_id = Uuid::new_v4();
        let claimed = match do_claim(&pool, community_id, report_id, request_id).await {
            ClaimResult::Claimed(a) => a,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let action_id = claimed.id;

        let _ = begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");

        // Attempt finalize WITHOUT committing step_marker — must fail.
        let finalized = do_finalize(&pool, action_id, community_id, report_id).await;
        assert!(
            !finalized,
            "finalize_success must be rejected when step_marker is NULL"
        );
    }

    // ── Action lease: concurrent drivers cannot both run mutation branch ──────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn action_lease_prevents_concurrent_mutation() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        let claimed = match do_claim(&pool, community_id, report_id, Uuid::new_v4()).await {
            ClaimResult::Claimed(a) => a,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let action_id = claimed.id;

        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(30);

        // First driver acquires the lease.
        let first = acquire_action_lease(&pool, action_id, lease_until)
            .await
            .expect("acquire_action_lease first");
        assert!(
            matches!(first, LeaseResult::Acquired(_)),
            "first acquire must succeed"
        );

        // Second concurrent driver must be blocked.
        let second = acquire_action_lease(&pool, action_id, lease_until)
            .await
            .expect("acquire_action_lease second");
        assert!(
            matches!(second, LeaseResult::Contended),
            "second acquire while lease active must return Contended"
        );

        // Release the lease.
        if let LeaseResult::Acquired(token) = first {
            release_action_lease(&pool, action_id, token)
                .await
                .expect("release_action_lease");
        }

        // After release, lease can be acquired again.
        let third = acquire_action_lease(&pool, action_id, lease_until)
            .await
            .expect("acquire_action_lease third");
        assert!(
            matches!(third, LeaseResult::Acquired(_)),
            "acquire after release must succeed"
        );
    }

    // ── execute_ban_with_marker: atomic mutation + step marker ─────────────────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn execute_ban_with_marker_is_atomic() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        let claimed = match do_claim(&pool, community_id, report_id, Uuid::new_v4()).await {
            ClaimResult::Claimed(a) => a,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let action_id = claimed.id;

        // Advance to enforcing.
        let _ = begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");

        let target = vec![1u8; 32];
        let actork = actor();
        let cid = CommunityId::from_uuid(community_id);

        // Acquire action lease (required by execute_ban_with_marker).
        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(60);
        let lease_token = match acquire_action_lease(&pool, action_id, lease_until)
            .await
            .expect("acquire_action_lease")
        {
            LeaseResult::Acquired(t) => t,
            other => panic!("expected Acquired, got {other:?}"),
        };

        // Execute ban + step_marker in one transaction.
        let committed =
            execute_ban_with_marker(&pool, action_id, lease_token, cid, &target, &actork, None)
                .await
                .expect("execute_ban_with_marker");
        assert!(committed, "execute_ban_with_marker must return true");

        // step_marker must now be 'mutation_committed'.
        let rec = get_action(&pool, action_id)
            .await
            .expect("get_action")
            .expect("action exists");
        assert_eq!(rec.step_marker.as_deref(), Some("mutation_committed"));

        // Re-execution with same token: step_marker already set → returns false (idempotent).
        let second =
            execute_ban_with_marker(&pool, action_id, lease_token, cid, &target, &actork, None)
                .await
                .expect("second execute_ban_with_marker");
        assert!(
            !second,
            "second execute_ban_with_marker must return false (already marked)"
        );
    }

    // ── execute_kick_with_marker: provenance — Removed vs AlreadyGone ─────────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn execute_kick_with_marker_tracks_provenance() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let actork = actor();
        let target = vec![3u8; 32];
        let cid = CommunityId::from_uuid(community_id);

        // Create a channel and add the target as a member.
        let channel_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by)
            VALUES ($1, $2, 'test-kick', 'stream', 'open', $3)
            "#,
        )
        .bind(channel_id)
        .bind(community_id)
        .bind(&actork)
        .execute(&pool)
        .await
        .expect("create channel");
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role) VALUES ($1, $2, $3, 'member')",
        )
        .bind(community_id)
        .bind(channel_id)
        .bind(&target)
        .execute(&pool)
        .await
        .expect("add member");

        // Set up a kick action.
        let target_event = {
            let uid = Uuid::new_v4();
            uid.as_bytes()
                .iter()
                .chain(uid.as_bytes().iter())
                .copied()
                .collect::<Vec<u8>>()
        };
        let reporter = vec![0u8; 32];
        let report_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO moderation_reports (
                community_id, report_event_id, reporter_pubkey, target_kind,
                target_pubkey, channel_id, report_type
            ) VALUES ($1, $2, $3, 'pubkey', $4, $5, 'harassment')
            RETURNING id
            "#,
        )
        .bind(community_id)
        .bind(target_event.as_slice())
        .bind(&reporter)
        .bind(&target)
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("insert report");

        let action_id = match claim_report(
            &pool,
            cid,
            report_id,
            Uuid::new_v4(),
            &actork,
            "operator",
            "kick",
            None,
            None,
            "resolve:kick",
            "relay_operator",
            Some(&target),
            None,
            Some(channel_id),
        )
        .await
        .expect("claim")
        {
            ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };

        let _ = begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");

        // Acquire action lease for action_id (required by execute_kick_with_marker).
        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(60);
        let lease_token1 = match acquire_action_lease(&pool, action_id, lease_until)
            .await
            .expect("acquire lease1")
        {
            LeaseResult::Acquired(t) => t,
            other => panic!("expected Acquired for action1, got {other:?}"),
        };

        // First kick: member is present → Removed + step_marker committed.
        let r1 = execute_kick_with_marker(
            &pool,
            action_id,
            lease_token1,
            cid,
            channel_id,
            &target,
            &actork,
        )
        .await
        .expect("first kick");
        assert!(
            matches!(r1, KickWithMarkerResult::Removed),
            "first kick must be Removed"
        );

        // step_marker must now be set.
        let rec = get_action(&pool, action_id)
            .await
            .expect("get_action")
            .expect("action exists");
        assert_eq!(rec.step_marker.as_deref(), Some("mutation_committed"));

        // Second kick action (new report, new action for AlreadyGone test).
        let report_id2: Uuid = {
            let uid2 = Uuid::new_v4();
            let eid2: Vec<u8> = uid2
                .as_bytes()
                .iter()
                .chain(uid2.as_bytes().iter())
                .copied()
                .collect();
            sqlx::query_scalar(
                r#"
                INSERT INTO moderation_reports (
                    community_id, report_event_id, reporter_pubkey, target_kind,
                    target_pubkey, channel_id, report_type
                ) VALUES ($1, $2, $3, 'pubkey', $4, $5, 'harassment')
                RETURNING id
                "#,
            )
            .bind(community_id)
            .bind(eid2.as_slice())
            .bind(&reporter)
            .bind(&target)
            .bind(channel_id)
            .fetch_one(&pool)
            .await
            .expect("insert report2")
        };

        let action_id2 = match claim_report(
            &pool,
            cid,
            report_id2,
            Uuid::new_v4(),
            &actork,
            "operator",
            "kick",
            None,
            None,
            "resolve:kick",
            "relay_operator",
            Some(&target),
            None,
            Some(channel_id),
        )
        .await
        .expect("claim2")
        {
            ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let _ = begin_enforcing(&pool, action_id2)
            .await
            .expect("begin_enforcing2");

        // Acquire action lease for action_id2.
        let lease_token2 = match acquire_action_lease(&pool, action_id2, lease_until)
            .await
            .expect("acquire lease2")
        {
            LeaseResult::Acquired(t) => t,
            other => panic!("expected Acquired for action2, got {other:?}"),
        };

        // Target already gone (removed by action 1) → AlreadyGone, step marker NOT committed.
        let r2 = execute_kick_with_marker(
            &pool,
            action_id2,
            lease_token2,
            cid,
            channel_id,
            &target,
            &actork,
        )
        .await
        .expect("second kick");
        assert!(
            matches!(r2, KickWithMarkerResult::AlreadyGone),
            "kick of absent target must return AlreadyGone"
        );

        // Step marker for action2 must NOT be set.
        let rec2 = get_action(&pool, action_id2)
            .await
            .expect("get_action2")
            .expect("action2 exists");
        assert!(
            rec2.step_marker.is_none(),
            "AlreadyGone kick must not commit step_marker; got: {:?}",
            rec2.step_marker
        );
    }

    // ── Stranded action recovery: claim_stranded_action_batch ─────────────────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_stranded_action_batch_claims_pending_actions() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;

        // Create a pending action (no lease = stranded).
        let claimed = match do_claim(&pool, community_id, report_id, Uuid::new_v4()).await {
            ClaimResult::Claimed(a) => a,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let action_id = claimed.id;

        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(30);

        // Action recovery worker claims the stranded action.
        let batch = claim_stranded_action_batch(&pool, "test-worker", lease_until, 1000)
            .await
            .expect("claim_stranded_action_batch");

        let found = batch.iter().any(|c| c.record.id == action_id);
        assert!(found, "stranded action must appear in recovery batch");

        // After claiming, the same worker must not see it again (already leased).
        let batch2 = claim_stranded_action_batch(&pool, "test-worker-2", lease_until, 10)
            .await
            .expect("claim_stranded_action_batch second");
        assert!(
            !batch2.iter().any(|c| c.record.id == action_id),
            "leased action must not appear in second recovery batch"
        );
    }

    // ── Deploy kick member distinguishes Removed vs AlreadyGone ──────────────

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn deploy_kick_member_removed_vs_already_gone() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let actor = actor();
        let target = vec![3u8; 32];

        // Create a channel and add the target as a member.
        let channel_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by)
            VALUES ($1, $2, 'test', 'stream', 'open', $3)
            "#,
        )
        .bind(channel_id)
        .bind(community_id)
        .bind(&actor)
        .execute(&pool)
        .await
        .expect("create channel");
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role) VALUES ($1, $2, $3, 'member')",
        )
        .bind(community_id)
        .bind(channel_id)
        .bind(&target)
        .execute(&pool)
        .await
        .expect("add member");

        // First kick: member is present → Removed.
        let r1 = deploy_kick_member(
            &pool,
            CommunityId::from_uuid(community_id),
            channel_id,
            &target,
            &actor,
        )
        .await
        .expect("first kick");
        assert_eq!(r1, KickResult::Removed, "first kick must return Removed");

        // Second kick: member is gone → AlreadyGone.
        let r2 = deploy_kick_member(
            &pool,
            CommunityId::from_uuid(community_id),
            channel_id,
            &target,
            &actor,
        )
        .await
        .expect("second kick");
        assert_eq!(
            r2,
            KickResult::AlreadyGone,
            "second kick must return AlreadyGone"
        );
    }

    // ── Reopen: terminal → open CAS + durable audit row ───────────────────────

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

    async fn report_status(pool: &PgPool, community_id: Uuid, report_id: Uuid) -> String {
        sqlx::query_scalar(
            "SELECT status FROM moderation_reports WHERE community_id = $1 AND id = $2",
        )
        .bind(community_id)
        .bind(report_id)
        .fetch_one(pool)
        .await
        .expect("read report status")
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reopen_terminal_report_returns_open_and_records_succeeded_audit_row() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let cid = CommunityId::from_uuid(community_id);

        for terminal in ["resolved", "dismissed", "escalated"] {
            let report_id = make_report(&pool, community_id).await;
            set_report_status(&pool, community_id, report_id, terminal).await;

            let request_id = Uuid::new_v4();
            let result = reopen_report(
                &pool,
                cid,
                report_id,
                request_id,
                &actor(),
                "operator",
                Some("re-triage"),
            )
            .await
            .expect("reopen_report");
            assert!(
                matches!(result, ReopenResult::Reopened),
                "reopen of {terminal} must return Reopened, got {result:?}"
            );
            assert_eq!(
                report_status(&pool, community_id, report_id).await,
                "open",
                "report must be open after reopen from {terminal}"
            );

            // The audit row is state='succeeded' and action='reopen' so the
            // recovery worker never claims it and the DTO join never surfaces it.
            let row = sqlx::query(
                "SELECT action, state FROM relay_admin_actions WHERE report_id = $1 AND request_id = $2",
            )
            .bind(report_id)
            .bind(request_id)
            .fetch_one(&pool)
            .await
            .expect("reopen audit row exists");
            assert_eq!(row.try_get::<String, _>("action").unwrap(), "reopen");
            assert_eq!(row.try_get::<String, _>("state").unwrap(), "succeeded");
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reopen_open_report_is_rejected_as_not_reopenable() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await; // starts 'open'

        let result = reopen_report(
            &pool,
            CommunityId::from_uuid(community_id),
            report_id,
            Uuid::new_v4(),
            &actor(),
            "moderator",
            None,
        )
        .await
        .expect("reopen_report");
        assert!(
            matches!(result, ReopenResult::NotReopenable(ref s) if s == "open"),
            "reopen of an open report must return NotReopenable(open), got {result:?}"
        );

        // No audit row written on a rejected reopen.
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM relay_admin_actions WHERE report_id = $1")
                .bind(report_id)
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(count, 0, "rejected reopen must not write an audit row");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reopen_processing_report_is_rejected_as_not_reopenable() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let report_id = make_report(&pool, community_id).await;
        // A live enforcement claim moves the report to 'processing'.
        let _ = do_claim(&pool, community_id, report_id, Uuid::new_v4()).await;

        let result = reopen_report(
            &pool,
            CommunityId::from_uuid(community_id),
            report_id,
            Uuid::new_v4(),
            &actor(),
            "operator",
            None,
        )
        .await
        .expect("reopen_report");
        assert!(
            matches!(result, ReopenResult::NotReopenable(ref s) if s == "processing"),
            "reopen of a processing report must return NotReopenable(processing), got {result:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reopen_is_idempotent_on_request_id_even_after_reresolve() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let cid = CommunityId::from_uuid(community_id);
        let report_id = make_report(&pool, community_id).await;
        set_report_status(&pool, community_id, report_id, "resolved").await;

        let request_id = Uuid::new_v4();
        let first = reopen_report(
            &pool,
            cid,
            report_id,
            request_id,
            &actor(),
            "operator",
            None,
        )
        .await
        .expect("first reopen");
        assert!(matches!(first, ReopenResult::Reopened));

        // The report gets re-resolved (a fresh terminal cycle) before the client's
        // network retry of the SAME reopen request lands.
        set_report_status(&pool, community_id, report_id, "resolved").await;

        let replay = reopen_report(
            &pool,
            cid,
            report_id,
            request_id,
            &actor(),
            "operator",
            None,
        )
        .await
        .expect("reopen replay");
        assert!(
            matches!(replay, ReopenResult::AlreadyReopened),
            "same request_id replay must return AlreadyReopened, got {replay:?}"
        );

        // The replay must NOT have re-reopened the freshly re-resolved report.
        assert_eq!(
            report_status(&pool, community_id, report_id).await,
            "resolved",
            "idempotent replay must not re-reopen a re-resolved report"
        );

        // Exactly one reopen audit row exists for this request_id.
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM relay_admin_actions WHERE report_id = $1 AND request_id = $2 AND action = 'reopen'",
        )
        .bind(report_id)
        .bind(request_id)
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(
            count, 1,
            "idempotent replay must not write a second audit row"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reopen_missing_report_returns_not_found() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;

        let result = reopen_report(
            &pool,
            CommunityId::from_uuid(community_id),
            Uuid::new_v4(), // no such report
            Uuid::new_v4(),
            &actor(),
            "operator",
            None,
        )
        .await
        .expect("reopen_report");
        assert!(
            matches!(result, ReopenResult::NotFound),
            "reopen of a missing report must return NotFound, got {result:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reopen_audit_row_is_never_claimed_by_recovery_worker() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let cid = CommunityId::from_uuid(community_id);
        let report_id = make_report(&pool, community_id).await;
        set_report_status(&pool, community_id, report_id, "dismissed").await;

        let request_id = Uuid::new_v4();
        reopen_report(
            &pool,
            cid,
            report_id,
            request_id,
            &actor(),
            "operator",
            None,
        )
        .await
        .expect("reopen");

        // The stranded-action recovery worker claims state IN ('pending','enforcing').
        // A 'succeeded' reopen row must never appear in its batch — otherwise the
        // worker would try to drive an enforcement mutation for a reopen.
        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(30);
        let batch = claim_stranded_action_batch(&pool, "recovery-worker", lease_until, 1000)
            .await
            .expect("claim batch");
        let reopen_action_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM relay_admin_actions WHERE report_id = $1 AND request_id = $2",
        )
        .bind(report_id)
        .bind(request_id)
        .fetch_one(&pool)
        .await
        .expect("reopen action id");
        assert!(
            !batch.iter().any(|c| c.record.id == reopen_action_id),
            "reopen audit row (state=succeeded) must never be claimed by the recovery worker"
        );
    }

    /// An `illegal` report reaches `escalated` at ingest with no resolver, while
    /// an admin `escalate` reaches it through a decision that stamps one. The
    /// reopen path keys only on `status`, so both must reopen identically —
    /// nothing downstream may special-case how a report became escalated.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn auto_escalated_report_reopens_like_an_admin_escalated_one() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let cid = CommunityId::from_uuid(community_id);

        // Auto-escalated at ingest: illegal category, no resolver stamped.
        let uid = Uuid::new_v4();
        let event_id: Vec<u8> = uid
            .as_bytes()
            .iter()
            .chain(uid.as_bytes().iter())
            .copied()
            .collect();
        let auto_id = crate::moderation::insert_report(
            &pool,
            cid,
            crate::moderation::NewReport {
                report_event_id: &event_id,
                reporter_pubkey: &[0u8; 32],
                target: crate::moderation::ReportTarget::Pubkey(vec![7u8; 32]),
                channel_id: None,
                report_type: "illegal",
                note: None,
            },
        )
        .await
        .expect("insert illegal report");
        assert_eq!(
            report_status(&pool, community_id, auto_id).await,
            "escalated",
            "illegal report must ingest as escalated"
        );

        let result = reopen_report(
            &pool,
            cid,
            auto_id,
            Uuid::new_v4(),
            &actor(),
            "operator",
            Some("re-triage auto-escalated"),
        )
        .await
        .expect("reopen_report");
        assert!(
            matches!(result, ReopenResult::Reopened),
            "auto-escalated report must reopen exactly like an admin-escalated one, got {result:?}"
        );
        assert_eq!(
            report_status(&pool, community_id, auto_id).await,
            "open",
            "auto-escalated report must return to open after reopen"
        );
    }
}
