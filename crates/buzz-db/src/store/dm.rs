//! Direct message channel persistence.
//!
//! DMs are channels with channel_type='dm' and visibility='private'.
//! Participant sets are immutable -- adding a member creates a NEW DM.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::channel::ChannelRecord;
use crate::channel_members;
use crate::error::{DbError, Result};
use crate::Db;
use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;

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

/// Outcome of opening a direct-message participant set.
#[derive(Debug, Clone)]
pub struct OpenDmResult {
    /// The existing or newly created DM channel.
    pub channel: ChannelRecord,
    /// Whether this call created the channel.
    pub was_created: bool,
    /// Participants whose missing or soft-removed memberships were restored.
    pub restored_participants: Vec<Vec<u8>>,
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
/// An active participant reopening an existing immutable participant set also
/// restores any missing or soft-removed peer memberships. A removed caller
/// cannot use this path to restore themselves or anyone else.
pub async fn open_dm(
    pool: &PgPool,
    community_id: CommunityId,
    pubkeys: &[&[u8]],
    created_by: &[u8],
) -> Result<OpenDmResult> {
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
        let restored_participants =
            reopen_existing_dm(pool, community_id, existing.id, &all, created_by).await?;
        return Ok(OpenDmResult {
            channel: existing,
            was_created: false,
            restored_participants,
        });
    }

    // Create new DM.
    let channel = create_dm(pool, community_id, &all, created_by).await?;

    Ok(OpenDmResult {
        channel,
        was_created: true,
        restored_participants: Vec::new(),
    })
}

/// Reopen an existing immutable DM participant set under the same membership
/// serialization lock used by normal channel membership changes.
async fn reopen_existing_dm(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    participants: &[&[u8]],
    opened_by: &[u8],
) -> Result<Vec<Vec<u8>>> {
    let mut tx = pool.begin().await?;
    channel_members::acquire_channel_membership_lock(&mut tx, community_id, channel_id).await?;

    let opener_is_active: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM channel_members
            WHERE community_id = $1
              AND channel_id = $2
              AND pubkey = $3
              AND removed_at IS NULL
        )
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(opened_by)
    .fetch_one(&mut *tx)
    .await?;

    if !opener_is_active {
        return Err(DbError::AccessDenied(
            "only an active DM participant may restore the participant set".to_string(),
        ));
    }

    let participant_bytes: Vec<Vec<u8>> = participants.iter().map(|pk| pk.to_vec()).collect();
    let rows = sqlx::query(
        r#"
        WITH requested(pubkey) AS (
            SELECT DISTINCT unnest($3::bytea[])
        )
        INSERT INTO channel_members
            (community_id, channel_id, pubkey, role, invited_by)
        SELECT $1, $2, requested.pubkey, 'member', $4
        FROM requested
        ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET
            removed_at = NULL,
            removed_by = NULL,
            hidden_at = NULL,
            role = EXCLUDED.role,
            invited_by = EXCLUDED.invited_by
        WHERE channel_members.removed_at IS NOT NULL
        RETURNING pubkey
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(&participant_bytes)
    .bind(opened_by)
    .fetch_all(&mut *tx)
    .await?;

    // Reopening always resurfaces the DM for its active caller. Active peers'
    // independent hidden preferences remain untouched.
    sqlx::query(
        r#"
        UPDATE channel_members
        SET hidden_at = NULL
        WHERE community_id = $1
          AND channel_id = $2
          AND pubkey = $3
          AND removed_at IS NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(opened_by)
    .execute(&mut *tx)
    .await?;

    let mut restored = rows
        .into_iter()
        .map(|row| row.try_get::<Vec<u8>, _>("pubkey").map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;
    restored.sort_unstable();
    tx.commit().await?;
    Ok(restored)
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
        UPDATE channel_members
        SET hidden_at = NOW()
        WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 AND removed_at IS NULL
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
        UPDATE channel_members
        SET hidden_at = NULL
        WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 AND removed_at IS NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
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

// -- Db API -------------------------------------------------------------------

impl Db {
    /// Find an existing DM by its participant hash.
    #[datastore_span(name = "find_dm_by_participants", system = "postgresql")]
    pub async fn find_dm_by_participants(
        &self,
        community_id: CommunityId,
        participant_hash: &[u8],
    ) -> Result<Option<ChannelRecord>> {
        crate::dm::find_dm_by_participants(&self.pool, community_id, participant_hash).await
    }

    /// Create or return an existing DM channel.
    #[datastore_span(name = "create_dm", system = "postgresql")]
    pub async fn create_dm(
        &self,
        community_id: CommunityId,
        participants: &[&[u8]],
        created_by: &[u8],
    ) -> Result<ChannelRecord> {
        crate::dm::create_dm(&self.pool, community_id, participants, created_by).await
    }

    /// List all DMs for a user.
    #[datastore_span(name = "list_dms_for_user", system = "postgresql")]
    pub async fn list_dms_for_user(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
        limit: u32,
        cursor: Option<Uuid>,
    ) -> Result<Vec<DmRecord>> {
        crate::dm::list_dms_for_user(&self.pool, community_id, pubkey, limit, cursor).await
    }

    /// Open or retrieve a DM for the given participants.
    #[datastore_span(name = "open_dm", system = "postgresql")]
    pub async fn open_dm(
        &self,
        community_id: CommunityId,
        pubkeys: &[&[u8]],
        created_by: &[u8],
    ) -> Result<OpenDmResult> {
        crate::dm::open_dm(&self.pool, community_id, pubkeys, created_by).await
    }

    /// Hide a DM channel for a specific user.
    ///
    /// The DM is not deleted — it can be restored by opening a new DM with
    /// the same participants.
    #[datastore_span(name = "hide_dm", system = "postgresql")]
    pub async fn hide_dm(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) -> Result<()> {
        crate::dm::hide_dm(&self.pool, community_id, channel_id, pubkey).await
    }

    /// Unhide a DM channel for a specific user.
    #[datastore_span(name = "unhide_dm", system = "postgresql")]
    pub async fn unhide_dm(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) -> Result<()> {
        crate::dm::unhide_dm(&self.pool, community_id, channel_id, pubkey).await
    }

    /// List the channel IDs of all DMs the given user currently has hidden.
    #[datastore_span(name = "list_hidden_dms", system = "postgresql")]
    pub async fn list_hidden_dms(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<Vec<Uuid>> {
        crate::dm::list_hidden_dms(&self.pool, community_id, pubkey).await
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    type MembershipRemovalState = (
        Option<DateTime<Utc>>,
        Option<Vec<u8>>,
        Option<DateTime<Utc>>,
    );

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_owned());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrate test database");
        pool
    }

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

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn active_participant_reopening_dm_restores_removed_peer() {
        let pool = setup_pool().await;
        let community_uuid = Uuid::new_v4();
        let community = CommunityId::from_uuid(community_uuid);
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(format!("dm-recovery-{}.example", community_uuid.simple()))
            .execute(&pool)
            .await
            .expect("insert test community");

        let opener = [1u8; 32];
        let peer = [2u8; 32];
        let opened = open_dm(&pool, community, &[&peer], &opener)
            .await
            .expect("create dm");
        assert!(opened.was_created, "fixture must create a fresh DM");
        let channel = opened.channel;

        sqlx::query(
            r#"
            UPDATE channel_members
            SET removed_at = NOW(), removed_by = $4, hidden_at = NOW()
            WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3
            "#,
        )
        .bind(community_uuid)
        .bind(channel.id)
        .bind(peer.as_slice())
        .bind(opener.as_slice())
        .execute(&pool)
        .await
        .expect("soft-remove peer fixture");

        let reopened = open_dm(&pool, community, &[&peer], &opener)
            .await
            .expect("reopen existing dm");
        assert_eq!(
            reopened.channel.id, channel.id,
            "reopen must preserve DM history"
        );
        assert!(
            !reopened.was_created,
            "reopen must not create a replacement DM"
        );
        assert_eq!(reopened.restored_participants, vec![peer.to_vec()]);

        let restored: MembershipRemovalState = sqlx::query_as(
            r#"
                SELECT removed_at, removed_by, hidden_at
                FROM channel_members
                WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3
                "#,
        )
        .bind(community_uuid)
        .bind(channel.id)
        .bind(peer.as_slice())
        .fetch_one(&pool)
        .await
        .expect("read restored peer membership");

        assert_eq!(restored, (None, None, None));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn removed_participant_cannot_reopen_dm_or_restore_peer() {
        let pool = setup_pool().await;
        let community_uuid = Uuid::new_v4();
        let community = CommunityId::from_uuid(community_uuid);
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(format!(
                "dm-recovery-denied-{}.example",
                community_uuid.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert test community");

        let removed_caller = [3u8; 32];
        let removed_peer = [4u8; 32];
        let opened = open_dm(&pool, community, &[&removed_peer], &removed_caller)
            .await
            .expect("create dm");
        assert!(opened.was_created, "fixture must create a fresh DM");
        let channel = opened.channel;

        sqlx::query(
            r#"
            UPDATE channel_members
            SET removed_at = NOW(), removed_by = $4
            WHERE community_id = $1 AND channel_id = $2 AND pubkey = ANY($3)
            "#,
        )
        .bind(community_uuid)
        .bind(channel.id)
        .bind(vec![removed_caller.to_vec(), removed_peer.to_vec()])
        .bind(removed_caller.as_slice())
        .execute(&pool)
        .await
        .expect("soft-remove caller and peer fixtures");

        let error = open_dm(&pool, community, &[&removed_peer], &removed_caller)
            .await
            .expect_err("removed caller must not resurrect the DM participant set");
        assert!(matches!(error, DbError::AccessDenied(_)));

        let active_count: i64 = sqlx::query_scalar(
            r#"
            SELECT count(*)
            FROM channel_members
            WHERE community_id = $1 AND channel_id = $2 AND removed_at IS NULL
            "#,
        )
        .bind(community_uuid)
        .bind(channel.id)
        .fetch_one(&pool)
        .await
        .expect("count active memberships");
        assert_eq!(active_count, 0, "denied reopen must not restore any peer");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reopening_dm_restores_missing_peer_without_unhiding_active_peer() {
        let pool = setup_pool().await;
        let community_uuid = Uuid::new_v4();
        let community = CommunityId::from_uuid(community_uuid);
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(format!(
                "dm-recovery-hidden-{}.example",
                community_uuid.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert test community");

        let opener = [5u8; 32];
        let missing_peer = [6u8; 32];
        let hidden_active_peer = [7u8; 32];
        let opened = open_dm(
            &pool,
            community,
            &[&missing_peer, &hidden_active_peer],
            &opener,
        )
        .await
        .expect("create group dm");
        assert!(opened.was_created, "fixture must create a fresh DM");

        sqlx::query(
            r#"
            DELETE FROM channel_members
            WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3
            "#,
        )
        .bind(community_uuid)
        .bind(opened.channel.id)
        .bind(missing_peer.as_slice())
        .execute(&pool)
        .await
        .expect("delete peer membership fixture");
        sqlx::query(
            r#"
            UPDATE channel_members
            SET hidden_at = NOW()
            WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3
            "#,
        )
        .bind(community_uuid)
        .bind(opened.channel.id)
        .bind(hidden_active_peer.as_slice())
        .execute(&pool)
        .await
        .expect("hide active peer fixture");

        let reopened = open_dm(
            &pool,
            community,
            &[&missing_peer, &hidden_active_peer],
            &opener,
        )
        .await
        .expect("reopen group dm");
        assert_eq!(reopened.restored_participants, vec![missing_peer.to_vec()]);

        let hidden_at: Option<DateTime<Utc>> = sqlx::query_scalar(
            r#"
            SELECT hidden_at
            FROM channel_members
            WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3
            "#,
        )
        .bind(community_uuid)
        .bind(opened.channel.id)
        .bind(hidden_active_peer.as_slice())
        .fetch_one(&pool)
        .await
        .expect("read active peer visibility");
        assert!(
            hidden_at.is_some(),
            "reopening must preserve an active peer's hidden preference"
        );
    }
}
