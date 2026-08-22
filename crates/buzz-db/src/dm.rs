//! Direct message channel persistence.
//!
//! DMs are channels with channel_type='dm' and visibility='private'.
//! Participant sets are immutable -- adding a member creates a NEW DM.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::channel::ChannelRecord;
use crate::error::{DbError, Result};
use buzz_core::CommunityId;

// -- Public structs -----------------------------------------------------------

/// A DM conversation with its participant list.
#[derive(Debug, Clone)]
pub struct DmRecord {
    /// The underlying channel ID.
    pub channel_id: Uuid,
    /// All active participants in this DM.
    pub participants: Vec<DmParticipant>,
    /// When the last message was sent (approximated by channel updated_at).
    pub last_message_at: Option<DateTime<Utc>>,
    /// When the DM was created.
    pub created_at: DateTime<Utc>,
}

/// A single participant in a DM.
#[derive(Debug, Clone)]
pub struct DmParticipant {
    /// Compressed public key bytes.
    pub pubkey: Vec<u8>,
    /// Optional display name from the users table.
    pub display_name: Option<String>,
    /// Member role string (always "member" for DMs).
    pub role: String,
}

/// One bounded retry-queue claim for a relay-authored DM visibility snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedDmVisibilityViewer {
    /// Community containing the viewer's DM memberships.
    pub community_id: CommunityId,
    /// Canonical host used to construct the tenant context for fan-out.
    pub community_host: String,
    /// Viewer whose kind:30622 projection needs rebuilding.
    pub viewer: Vec<u8>,
    /// Claim fence used to release a failed publication without touching a
    /// newer mutation that reset the queue row.
    pub claim_id: Uuid,
}

// -- Pure helpers -------------------------------------------------------------

/// Compute a stable SHA-256 fingerprint for a set of participant pubkeys.
///
/// Pubkeys are sorted lexicographically before hashing so that the same set
/// of participants always produces the same hash regardless of input order.
/// No separator is used because all pubkeys are fixed-width 32-byte values.
pub fn compute_participant_hash(pubkeys: &[&[u8]]) -> [u8; 32] {
    let mut sorted: Vec<&[u8]> = pubkeys.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut hasher = Sha256::new();
    for pk in sorted {
        hasher.update(pk);
    }
    hasher.finalize().into()
}

// -- DB functions -------------------------------------------------------------

/// Find an existing DM by its participant hash.
///
/// Returns `None` if no matching DM exists or if it has been deleted.
pub async fn find_dm_by_participants(
    pool: &PgPool,
    community_id: CommunityId,
    participant_hash: &[u8],
) -> Result<Option<ChannelRecord>> {
    let row = sqlx::query(
        r#"
        SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
               description, canvas,
               created_by, created_at, updated_at, archived_at, deleted_at,
               nip29_group_id, topic_required, max_members,
               topic, topic_set_by, topic_set_at,
               purpose, purpose_set_by, purpose_set_at
        FROM channels
        WHERE community_id = $1
          AND participant_hash = $2
          AND channel_type = 'dm'
          AND deleted_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(participant_hash)
    .fetch_optional(pool)
    .await?;

    row.map(row_to_channel_record).transpose()
}

/// Create a new DM channel for the given participant pubkeys, or return the
/// existing one if a DM with the same participant set already exists.
///
/// Rules:
/// - `participants` must contain 2-9 entries (enforced here).
/// - `created_by` must be one of the participants.
/// - The operation is idempotent: same participant set -> same channel returned.
pub async fn create_dm(
    pool: &PgPool,
    community_id: CommunityId,
    participants: &[&[u8]],
    created_by: &[u8],
) -> Result<ChannelRecord> {
    if participants.len() < 2 {
        return Err(DbError::InvalidData(
            "DM requires at least 2 participants".to_string(),
        ));
    }
    if participants.len() > 9 {
        return Err(DbError::InvalidData(
            "DM supports at most 9 participants".to_string(),
        ));
    }
    for pk in participants {
        if pk.len() != 32 {
            return Err(DbError::InvalidData(format!(
                "pubkey must be 32 bytes, got {}",
                pk.len()
            )));
        }
    }

    let hash = compute_participant_hash(participants);

    let mut tx = pool.begin().await?;

    // Idempotency check inside the transaction.
    let existing = sqlx::query(
        r#"
        SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
               description, canvas,
               created_by, created_at, updated_at, archived_at, deleted_at,
               nip29_group_id, topic_required, max_members,
               topic, topic_set_by, topic_set_at,
               purpose, purpose_set_by, purpose_set_at
        FROM channels
        WHERE community_id = $1
          AND participant_hash = $2
          AND channel_type = 'dm'
          AND deleted_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(hash.as_slice())
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(row) = existing {
        tx.commit().await?;
        return row_to_channel_record(row);
    }

    // Name the DM based on participant count.
    let name = if participants.len() == 2 {
        "DM".to_string()
    } else {
        format!("Group DM ({})", participants.len())
    };

    let id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO channels
            (id, community_id, name, channel_type, visibility, created_by, participant_hash)
        VALUES ($1, $2, $3, 'dm', 'private', $4, $5)
        "#,
    )
    .bind(id)
    .bind(community_id.as_uuid())
    .bind(&name)
    .bind(created_by)
    .bind(hash.as_slice())
    .execute(&mut *tx)
    .await?;

    // Add all participants as members with role='member'.
    for pk in participants {
        sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
            VALUES ($1, $2, $3, 'member', $4)
            ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET
                removed_at = NULL,
                removed_by = NULL,
                role = EXCLUDED.role
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(id)
        .bind(*pk)
        .bind(created_by)
        .execute(&mut *tx)
        .await?;
    }

    let row = sqlx::query(
        r#"
        SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
               description, canvas,
               created_by, created_at, updated_at, archived_at, deleted_at,
               nip29_group_id, topic_required, max_members,
               topic, topic_set_by, topic_set_at,
               purpose, purpose_set_by, purpose_set_at
        FROM channels WHERE community_id = $1 AND id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;

    let record = row_to_channel_record(row)?;
    tx.commit().await?;
    Ok(record)
}

/// List all DM conversations for a given user, ordered by most recent activity.
///
/// Includes participant details for each DM. Supports cursor-based pagination
/// using `updated_at` ordering.
pub async fn list_dms_for_user(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
    limit: u32,
    cursor: Option<Uuid>,
) -> Result<Vec<DmRecord>> {
    let limit = limit.min(200) as i64;

    // Resolve cursor to a timestamp for keyset pagination.
    let cursor_ts: Option<DateTime<Utc>> = if let Some(cid) = cursor {
        let row =
            sqlx::query("SELECT updated_at FROM channels WHERE community_id = $1 AND id = $2")
                .bind(community_id.as_uuid())
                .bind(cid)
                .fetch_optional(pool)
                .await?;
        row.map(|r| r.try_get::<DateTime<Utc>, _>("updated_at"))
            .transpose()?
    } else {
        None
    };

    // Fetch DM channel IDs where this user is an active member.
    let channel_rows = if let Some(ts) = cursor_ts {
        sqlx::query(
            r#"
            SELECT c.id, c.created_at, c.updated_at
            FROM channels c
            JOIN channel_members cm
                ON c.community_id = cm.community_id
               AND c.id = cm.channel_id
               AND cm.pubkey = $2
               AND cm.removed_at IS NULL
               AND cm.hidden_at IS NULL
            WHERE c.community_id = $1
              AND c.channel_type = 'dm'
              AND c.deleted_at IS NULL
              AND c.updated_at < $3
            ORDER BY c.updated_at DESC
            LIMIT $4
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(pubkey)
        .bind(ts)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT c.id, c.created_at, c.updated_at
            FROM channels c
            JOIN channel_members cm
                ON c.community_id = cm.community_id
               AND c.id = cm.channel_id
               AND cm.pubkey = $2
               AND cm.removed_at IS NULL
               AND cm.hidden_at IS NULL
            WHERE c.community_id = $1
              AND c.channel_type = 'dm'
              AND c.deleted_at IS NULL
            ORDER BY c.updated_at DESC
            LIMIT $3
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(pubkey)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };

    let mut results = Vec::with_capacity(channel_rows.len());

    for row in channel_rows {
        let channel_id: Uuid = row.try_get("id")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

        // Fetch participants for this DM.
        let member_rows = sqlx::query(
            r#"
            SELECT cm.pubkey, cm.role::text AS role, u.display_name
            FROM channel_members cm
            LEFT JOIN users u
              ON u.community_id = cm.community_id
             AND u.pubkey = cm.pubkey
            WHERE cm.community_id = $1
              AND cm.channel_id = $2
              AND cm.removed_at IS NULL
            ORDER BY cm.joined_at ASC
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_all(pool)
        .await?;

        let participants: Vec<DmParticipant> = member_rows
            .into_iter()
            .map(|r| -> Result<DmParticipant> {
                Ok(DmParticipant {
                    pubkey: r.try_get("pubkey")?,
                    display_name: r.try_get("display_name")?,
                    role: r.try_get("role")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        results.push(DmRecord {
            channel_id,
            participants,
            last_message_at: Some(updated_at),
            created_at,
        });
    }

    Ok(results)
}

/// Open or retrieve a DM for the given set of participants.
///
/// `created_by` is automatically added to `pubkeys` if not already present,
/// ensuring the caller is always a participant in their own DM.
///
/// Returns `(channel, was_created)`:
/// - `was_created = true`  -- a new DM was created.
/// - `was_created = false` -- an existing DM was returned.
pub async fn open_dm(
    pool: &PgPool,
    community_id: CommunityId,
    pubkeys: &[&[u8]],
    created_by: &[u8],
) -> Result<(ChannelRecord, bool)> {
    // Merge created_by into the participant set (dedup handled by compute_participant_hash).
    let mut all: Vec<&[u8]> = pubkeys.to_vec();
    if !all.contains(&created_by) {
        all.push(created_by);
    }

    // Enforce max before hitting the DB.
    if all.len() > 9 {
        return Err(DbError::InvalidData(
            "DM supports at most 9 participants".to_string(),
        ));
    }

    let hash = compute_participant_hash(&all);

    // Check for existing DM first (fast path, no transaction).
    if let Some(existing) = find_dm_by_participants(pool, community_id, &hash).await? {
        // Clear hidden_at for the caller so the DM reappears in their sidebar.
        unhide_dm(pool, community_id, existing.id, created_by).await?;
        return Ok((existing, false));
    }

    // Create new DM.
    let channel = create_dm(pool, community_id, &all, created_by).await?;

    Ok((channel, true))
}

// -- Hide / unhide ------------------------------------------------------------

/// Hide a DM for a specific user by setting `hidden_at = NOW()`.
///
/// The DM is not deleted — it can be restored by opening a new DM with the
/// same participants (which clears `hidden_at`). Returns an error if the user
/// is not an active member of the channel.
pub async fn hide_dm(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<()> {
    let result = sqlx::query(
        r#"
        WITH changed AS (
            UPDATE channel_members
            SET hidden_at = NOW()
            WHERE community_id = $1
              AND channel_id = $2
              AND pubkey = $3
              AND removed_at IS NULL
            RETURNING pubkey
        )
        INSERT INTO dm_visibility_dirty_viewers (community_id, viewer)
        SELECT $1, pubkey FROM changed
        ON CONFLICT (community_id, viewer) DO UPDATE SET
            generation = dm_visibility_dirty_viewers.generation + 1,
            state = 'pending',
            dirty_at = NOW(),
            next_attempt_at = NOW(),
            claim_id = NULL,
            lease_until = NULL,
            attempts = 0
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::NotFound(format!(
            "no active membership for channel {channel_id}"
        )));
    }

    Ok(())
}

/// Unhide a DM for a specific user by clearing `hidden_at`.
///
/// This is called automatically when a user re-opens a DM via [`open_dm`].
/// It is a no-op if the membership is not currently hidden.
pub async fn unhide_dm(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<()> {
    sqlx::query(
        r#"
        WITH changed AS (
            UPDATE channel_members
            SET hidden_at = NULL
            WHERE community_id = $1
              AND channel_id = $2
              AND pubkey = $3
              AND removed_at IS NULL
            RETURNING pubkey
        )
        INSERT INTO dm_visibility_dirty_viewers (community_id, viewer)
        SELECT $1, pubkey FROM changed
        ON CONFLICT (community_id, viewer) DO UPDATE SET
            generation = dm_visibility_dirty_viewers.generation + 1,
            state = 'pending',
            dirty_at = NOW(),
            next_attempt_at = NOW(),
            claim_id = NULL,
            lease_until = NULL,
            attempts = 0
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .execute(pool)
    .await?;

    Ok(())
}

/// Clear the hidden state for every active recipient of a DM message.
///
/// The sender is deliberately excluded: receiving new activity resurfaces a
/// conversation, while sending from another surface must not silently rewrite
/// the sender's own sidebar preference. Returns only the pubkeys whose hidden
/// state changed so callers can publish targeted visibility snapshots.
pub async fn unhide_dm_recipients(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    sender_pubkey: &[u8],
) -> Result<Vec<Vec<u8>>> {
    let mut tx = pool.begin().await?;
    let viewers = unhide_dm_recipients_tx(&mut tx, community_id, channel_id, sender_pubkey).await?;
    tx.commit().await?;
    Ok(viewers)
}

/// Transactional form of [`unhide_dm_recipients`] used when accepted-message
/// persistence and the canonical visibility mutation must commit together.
pub(crate) async fn unhide_dm_recipients_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    channel_id: Uuid,
    sender_pubkey: &[u8],
) -> Result<Vec<Vec<u8>>> {
    let rows = sqlx::query(
        r#"
        WITH changed AS (
            UPDATE channel_members cm
            SET hidden_at = NULL
            FROM channels c
            WHERE cm.community_id = $1
              AND cm.channel_id = $2
              AND cm.pubkey != $3
              AND cm.removed_at IS NULL
              AND cm.hidden_at IS NOT NULL
              AND c.community_id = cm.community_id
              AND c.id = cm.channel_id
              AND c.channel_type = 'dm'
              AND c.deleted_at IS NULL
            RETURNING cm.pubkey
        )
        INSERT INTO dm_visibility_dirty_viewers (community_id, viewer)
        SELECT $1, pubkey FROM changed
        ON CONFLICT (community_id, viewer) DO UPDATE SET
            generation = dm_visibility_dirty_viewers.generation + 1,
            state = 'pending',
            dirty_at = NOW(),
            next_attempt_at = NOW(),
            claim_id = NULL,
            lease_until = NULL,
            attempts = 0
        RETURNING viewer
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(sender_pubkey)
    .fetch_all(&mut **tx)
    .await?;

    rows.into_iter()
        .map(|row| row.try_get::<Vec<u8>, _>("viewer").map_err(Into::into))
        .collect()
}

/// Claim at most `limit` indexed dirty-viewer rows across active communities.
/// Expired claims are recovered by the same bounded query.
pub async fn claim_dm_visibility_dirty_viewers(
    pool: &PgPool,
    limit: i64,
    lease_seconds: i64,
) -> Result<Vec<ClaimedDmVisibilityViewer>> {
    let claim_id = Uuid::new_v4();
    let limit = limit.clamp(1, 1_000);
    let fresh_limit = (limit + 1) / 2;
    let retry_limit = (limit - fresh_limit + 1) / 2;
    let recovery_limit = limit - fresh_limit - retry_limit;
    let rows = sqlx::query(
        r#"
        WITH fresh_candidates AS (
            SELECT q.community_id, q.viewer
            FROM dm_visibility_dirty_viewers q
            JOIN communities c ON c.id = q.community_id
            WHERE c.archived_at IS NULL
              AND c.deleted_at IS NULL
              AND c.deletion_state = 'active'
              AND community_write_allowed(q.community_id)
              AND q.state = 'pending'
              AND q.attempts = 0
              AND q.next_attempt_at <= NOW()
            ORDER BY q.dirty_at, q.community_id, q.viewer
            FOR UPDATE OF q SKIP LOCKED
            LIMIT $1
        ), retry_candidates AS (
            SELECT q.community_id, q.viewer
            FROM dm_visibility_dirty_viewers q
            JOIN communities c ON c.id = q.community_id
            WHERE c.archived_at IS NULL
              AND c.deleted_at IS NULL
              AND c.deletion_state = 'active'
              AND community_write_allowed(q.community_id)
              AND q.state = 'pending'
              AND q.attempts > 0
              AND q.next_attempt_at <= NOW()
            ORDER BY q.next_attempt_at, q.dirty_at, q.community_id, q.viewer
            FOR UPDATE OF q SKIP LOCKED
            LIMIT $2
        ), recovery_candidates AS (
            SELECT q.community_id, q.viewer
            FROM dm_visibility_dirty_viewers q
            JOIN communities c ON c.id = q.community_id
            WHERE c.archived_at IS NULL
              AND c.deleted_at IS NULL
              AND c.deletion_state = 'active'
              AND community_write_allowed(q.community_id)
              AND q.state = 'publishing'
              AND q.lease_until <= NOW()
            ORDER BY q.lease_until, q.community_id, q.viewer
            FOR UPDATE OF q SKIP LOCKED
            LIMIT $3
        ), overflow_fresh AS (
            SELECT q.community_id, q.viewer
            FROM dm_visibility_dirty_viewers q
            JOIN communities c ON c.id = q.community_id
            WHERE c.archived_at IS NULL
              AND c.deleted_at IS NULL
              AND c.deletion_state = 'active'
              AND community_write_allowed(q.community_id)
              AND q.state = 'pending'
              AND q.attempts = 0
              AND q.next_attempt_at <= NOW()
              AND NOT EXISTS (
                  SELECT 1 FROM fresh_candidates reserved
                  WHERE reserved.community_id = q.community_id
                    AND reserved.viewer = q.viewer
              )
            ORDER BY q.dirty_at, q.community_id, q.viewer
            FOR UPDATE OF q SKIP LOCKED
            LIMIT GREATEST(
                $4 - (SELECT COUNT(*) FROM fresh_candidates)
                   - (SELECT COUNT(*) FROM retry_candidates)
                   - (SELECT COUNT(*) FROM recovery_candidates),
                0
            )
        ), overflow_retry AS (
            SELECT q.community_id, q.viewer
            FROM dm_visibility_dirty_viewers q
            JOIN communities c ON c.id = q.community_id
            WHERE c.archived_at IS NULL
              AND c.deleted_at IS NULL
              AND c.deletion_state = 'active'
              AND community_write_allowed(q.community_id)
              AND q.state = 'pending'
              AND q.attempts > 0
              AND q.next_attempt_at <= NOW()
              AND NOT EXISTS (
                  SELECT 1 FROM retry_candidates reserved
                  WHERE reserved.community_id = q.community_id
                    AND reserved.viewer = q.viewer
              )
            ORDER BY q.next_attempt_at, q.dirty_at, q.community_id, q.viewer
            FOR UPDATE OF q SKIP LOCKED
            LIMIT GREATEST(
                $4 - (SELECT COUNT(*) FROM fresh_candidates)
                   - (SELECT COUNT(*) FROM retry_candidates)
                   - (SELECT COUNT(*) FROM recovery_candidates)
                   - (SELECT COUNT(*) FROM overflow_fresh),
                0
            )
        ), overflow_recovery AS (
            SELECT q.community_id, q.viewer
            FROM dm_visibility_dirty_viewers q
            JOIN communities c ON c.id = q.community_id
            WHERE c.archived_at IS NULL
              AND c.deleted_at IS NULL
              AND c.deletion_state = 'active'
              AND community_write_allowed(q.community_id)
              AND q.state = 'publishing'
              AND q.lease_until <= NOW()
              AND NOT EXISTS (
                  SELECT 1 FROM recovery_candidates reserved
                  WHERE reserved.community_id = q.community_id
                    AND reserved.viewer = q.viewer
              )
            ORDER BY q.lease_until, q.community_id, q.viewer
            FOR UPDATE OF q SKIP LOCKED
            LIMIT GREATEST(
                $4 - (SELECT COUNT(*) FROM fresh_candidates)
                   - (SELECT COUNT(*) FROM retry_candidates)
                   - (SELECT COUNT(*) FROM recovery_candidates)
                   - (SELECT COUNT(*) FROM overflow_fresh)
                   - (SELECT COUNT(*) FROM overflow_retry),
                0
            )
        ), candidates AS (
            SELECT community_id, viewer FROM fresh_candidates
            UNION ALL
            SELECT community_id, viewer FROM retry_candidates
            UNION ALL
            SELECT community_id, viewer FROM recovery_candidates
            UNION ALL
            SELECT community_id, viewer FROM overflow_fresh
            UNION ALL
            SELECT community_id, viewer FROM overflow_retry
            UNION ALL
            SELECT community_id, viewer FROM overflow_recovery
        )
        UPDATE dm_visibility_dirty_viewers q
        SET state = 'publishing',
            claim_id = $5,
            lease_until = NOW() + make_interval(secs => $6::int),
            attempts = q.attempts + 1
        FROM candidates candidate, communities c
        WHERE q.community_id = candidate.community_id
          AND q.viewer = candidate.viewer
          AND c.id = q.community_id
        RETURNING q.community_id, c.host, q.viewer
        "#,
    )
    .bind(fresh_limit)
    .bind(retry_limit)
    .bind(recovery_limit)
    .bind(limit)
    .bind(claim_id)
    .bind(lease_seconds.clamp(1, 3_600) as i32)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(ClaimedDmVisibilityViewer {
                community_id: CommunityId::from_uuid(row.try_get("community_id")?),
                community_host: row.try_get("host")?,
                viewer: row.try_get("viewer")?,
                claim_id,
            })
        })
        .collect()
}

/// Release a failed queue claim for a bounded retry. A mutation that occurred
/// after the claim resets `claim_id`, so this cannot overwrite newer work.
pub async fn retry_dm_visibility_dirty_viewer(
    pool: &PgPool,
    community_id: CommunityId,
    viewer: &[u8],
    claim_id: Uuid,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE dm_visibility_dirty_viewers
        SET state = 'pending',
            next_attempt_at = NOW() + make_interval(
                secs => LEAST(
                    300,
                    5 * POWER(2, LEAST(GREATEST(attempts - 1, 0), 6))::INTEGER
                )
            ),
            claim_id = NULL,
            lease_until = NULL
        WHERE community_id = $1
          AND viewer = $2
          AND state = 'publishing'
          AND claim_id = $3
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(viewer)
    .bind(claim_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Return the channel IDs of all DMs the given user currently has hidden
/// (`hidden_at IS NOT NULL`) while still being an active member. Used to build
/// the relay-signed NIP-DV visibility snapshot.
pub async fn list_hidden_dms(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Vec<Uuid>> {
    let rows = sqlx::query(
        r#"
        SELECT cm.channel_id
        FROM channel_members cm
        JOIN channels c
          ON c.community_id = cm.community_id
         AND c.id = cm.channel_id
        WHERE cm.community_id = $1
          AND cm.pubkey = $2
          AND cm.removed_at IS NULL
          AND cm.hidden_at IS NOT NULL
          AND c.channel_type = 'dm'
          AND c.deleted_at IS NULL
        ORDER BY cm.channel_id
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| r.try_get::<Uuid, _>("channel_id").map_err(Into::into))
        .collect()
}

// -- Row mapping --------------------------------------------------------------

fn row_to_channel_record(row: sqlx::postgres::PgRow) -> Result<ChannelRecord> {
    let id: Uuid = row.try_get("id")?;
    let topic_required: bool = row.try_get("topic_required")?;

    Ok(ChannelRecord {
        id,
        name: row.try_get("name")?,
        channel_type: row.try_get("channel_type")?,
        visibility: row.try_get("visibility")?,
        description: row.try_get("description")?,
        canvas: row.try_get("canvas")?,
        created_by: row.try_get("created_by")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        archived_at: row.try_get("archived_at")?,
        deleted_at: row.try_get("deleted_at")?,
        nip29_group_id: row.try_get("nip29_group_id")?,
        topic_required,
        max_members: row.try_get("max_members")?,
        topic: row.try_get("topic").unwrap_or(None),
        topic_set_by: row.try_get("topic_set_by").unwrap_or(None),
        topic_set_at: row.try_get("topic_set_at").unwrap_or(None),
        purpose: row.try_get("purpose").unwrap_or(None),
        purpose_set_by: row.try_get("purpose_set_by").unwrap_or(None),
        purpose_set_at: row.try_get("purpose_set_at").unwrap_or(None),
        ttl_seconds: row.try_get("ttl_seconds").unwrap_or(None),
        ttl_deadline: row.try_get("ttl_deadline").unwrap_or(None),
    })
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn participant_hash_is_order_independent() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let h1 = compute_participant_hash(&[&a, &b]);
        let h2 = compute_participant_hash(&[&b, &a]);
        assert_eq!(h1, h2, "hash must be the same regardless of input order");
    }

    #[test]
    fn participant_hash_deduplicates() {
        let a = [1u8; 32];
        let h1 = compute_participant_hash(&[&a, &a]);
        let h2 = compute_participant_hash(&[&a]);
        assert_eq!(h1, h2, "duplicate pubkeys should be deduped before hashing");
    }

    #[test]
    fn participant_hash_differs_for_different_sets() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let c = [3u8; 32];
        let h_ab = compute_participant_hash(&[&a, &b]);
        let h_ac = compute_participant_hash(&[&a, &c]);
        assert_ne!(h_ab, h_ac);
    }

    #[test]
    fn participant_hash_returns_32_bytes() {
        let a = [0u8; 32];
        let b = [255u8; 32];
        let h = compute_participant_hash(&[&a, &b]);
        assert_eq!(h.len(), 32);
    }
}
