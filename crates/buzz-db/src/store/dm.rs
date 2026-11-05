//! Direct message channel persistence.
//!
//! DMs are channels with channel_type='dm' and visibility='private'.
//!
//! A DM is identified by its people: at most one DM exists for a set of
//! people, and "message these people" finds it. Agents are add-ons: they join
//! a DM without changing which DM it is (see [`dm_identity_hash`]). A DM can
//! grow in place ([`grow_dm`]); its identity then moves to the grown set.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::channel::ChannelRecord;
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

/// The most people a DM can hold. Agents do not count.
pub const MAX_DM_PEOPLE: usize = 9;

/// The fingerprint that identifies a DM with these `members`, of whom
/// `agents` are agents.
///
/// A DM is identified by its people, so adding or removing an agent never
/// changes which DM it is. A DM with at most one person (someone and their
/// agents) is identified by all its members instead, so each such DM keeps its
/// own identity, as every DM did before agents were told apart.
pub fn dm_identity_hash(members: &[&[u8]], agents: &HashSet<Vec<u8>>) -> [u8; 32] {
    let people: Vec<&[u8]> = members
        .iter()
        .copied()
        .filter(|pk| !agents.contains(*pk))
        .collect();
    if people.len() >= 2 {
        compute_participant_hash(&people)
    } else {
        compute_participant_hash(members)
    }
}

/// The fingerprint a DM of `members` opened by `creator` gets. An agent
/// opening a DM gets one identified by all its members, as before agents were
/// told apart: only people decide which DM their people share.
fn opened_identity_hash(members: &[&[u8]], agents: &HashSet<Vec<u8>>, creator: &[u8]) -> [u8; 32] {
    if agents.contains(creator) {
        compute_participant_hash(members)
    } else {
        dm_identity_hash(members, agents)
    }
}

/// How many of `members` are people rather than agents.
fn count_people(members: &[&[u8]], agents: &HashSet<Vec<u8>>) -> usize {
    members.iter().filter(|pk| !agents.contains(**pk)).count()
}

/// The pubkeys among `candidates` that belong to agents: users with an owner.
async fn agents_among<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    community_id: CommunityId,
    candidates: &[&[u8]],
) -> Result<HashSet<Vec<u8>>> {
    let candidates: Vec<Vec<u8>> = candidates.iter().map(|pk| pk.to_vec()).collect();
    let agents: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT pubkey FROM users \
         WHERE community_id = $1 AND pubkey = ANY($2) AND agent_owner_pubkey IS NOT NULL",
    )
    .bind(community_id.as_uuid())
    .bind(&candidates)
    .fetch_all(executor)
    .await?;
    Ok(agents.into_iter().collect())
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
               purpose, purpose_set_by, purpose_set_at, labels, join_role, unlisted, auto_join
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
/// - `participants` must contain at least 2 entries, and at most
///   [`MAX_DM_PEOPLE`] people; agents do not count (enforced here).
/// - `created_by` must be one of the participants.
/// - The operation is idempotent: the same people (see [`dm_identity_hash`])
///   -> the same channel returned.
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
    for pk in participants {
        if pk.len() != 32 {
            return Err(DbError::InvalidData(format!(
                "pubkey must be 32 bytes, got {}",
                pk.len()
            )));
        }
    }
    let agents = agents_among(pool, community_id, participants).await?;
    if count_people(participants, &agents) > MAX_DM_PEOPLE {
        return Err(DbError::InvalidData(format!(
            "DM supports at most {MAX_DM_PEOPLE} people"
        )));
    }

    let hash = opened_identity_hash(participants, &agents, created_by);

    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;

    // Idempotency check inside the transaction.
    let existing = sqlx::query(
        r#"
        SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
               description, canvas,
               created_by, created_at, updated_at, archived_at, deleted_at,
               nip29_group_id, topic_required, max_members,
               topic, topic_set_by, topic_set_at,
               purpose, purpose_set_by, purpose_set_at, labels, join_role, unlisted, auto_join
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
    .fetch_optional(tx.conn())
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
    .execute(tx.conn())
    .await?;

    // Add all participants as members with role='member'.
    for pk in participants {
        sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
            VALUES ($1, $2, $3, 'member', $4)
            ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET
                sync_modified_at = CASE WHEN channel_members.removed_at IS NOT NULL THEN clock_timestamp() ELSE channel_members.sync_modified_at END,
                removed_at = NULL,
                removed_by = NULL,
                role = EXCLUDED.role
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(id)
        .bind(*pk)
        .bind(created_by)
        .execute(tx.conn())
        .await?;
    }

    let row = sqlx::query(
        r#"
        SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
               description, canvas,
               created_by, created_at, updated_at, archived_at, deleted_at,
               nip29_group_id, topic_required, max_members,
               topic, topic_set_by, topic_set_at,
               purpose, purpose_set_by, purpose_set_at, labels, join_role, unlisted, auto_join
        FROM channels WHERE community_id = $1 AND id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id)
    .fetch_one(tx.conn())
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
/// The DM is found by its people ([`dm_identity_hash`]), so it may hold agents
/// that `pubkeys` does not name, and lack agents it does. Adding those is
/// [`grow_dm`]'s job. A DM made before agents were told apart is still found by
/// all its members. An agent opening a DM finds or makes one identified by all
/// its members. A found DM is returned only to an active member of it:
/// an agent naming its people cannot use this to see or join their DM.
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
    let mut all: Vec<&[u8]> = vec![created_by];
    for pk in pubkeys {
        if !all.contains(pk) {
            all.push(pk);
        }
    }

    let agents = agents_among(pool, community_id, &all).await?;
    if count_people(&all, &agents) > MAX_DM_PEOPLE {
        return Err(DbError::InvalidData(format!(
            "DM supports at most {MAX_DM_PEOPLE} people"
        )));
    }

    let hash = opened_identity_hash(&all, &agents, created_by);
    // DMs made before agents were told apart are identified by all members.
    let legacy = compute_participant_hash(&all);

    // Check for existing DM first (fast path, no transaction).
    let mut existing = find_dm_by_participants(pool, community_id, &hash).await?;
    if existing.is_none() && legacy != hash {
        existing = find_dm_by_participants(pool, community_id, &legacy).await?;
    }
    if let Some(existing) = existing {
        if !crate::channel_members::is_member(pool, community_id, existing.id, created_by).await? {
            return Err(DbError::AccessDenied(
                "a DM with these people already exists".to_string(),
            ));
        }
        // Clear hidden_at for the caller so the DM reappears in their sidebar.
        unhide_dm(pool, community_id, existing.id, created_by).await?;
        return Ok((existing, false));
    }

    // Create new DM.
    let channel = create_dm(pool, community_id, &all, created_by).await?;

    Ok((channel, true))
}

/// The outcome of [`grow_dm`].
#[derive(Debug, Clone)]
pub enum DmGrowth {
    /// The DM now holds everyone asked for. `added` lists those who were not
    /// already active members, in the order asked.
    Grown {
        /// Pubkeys that became members.
        added: Vec<Vec<u8>>,
    },
    /// Another DM, with this ID, already has the grown set of people, so this
    /// DM cannot take its identity. Nothing was written.
    Existing(Uuid),
}

/// Add `additions` to the DM `channel_id` in place, keeping its history.
///
/// Agents join a DM of two or more people without changing which DM it is.
/// Otherwise the DM's identity moves to the grown set ([`dm_identity_hash`]), so
/// opening a DM with the original members afterwards starts a new one. If a DM
/// with the grown set already exists, nothing is written and that DM is
/// returned as [`DmGrowth::Existing`], provided `actor` is in it.
///
/// `actor` must be an active member. The DM must be live and not archived, and
/// may hold at most [`MAX_DM_PEOPLE`] people afterwards. Re-adding members is
/// a no-op, so a retry after a lost commit acknowledgement is safe.
///
/// Runs under the channel's membership locks ([`put_members`]'s), so it is
/// serialized against every other membership write and archive of the DM.
///
/// [`put_members`]: crate::channel_members::put_members
pub async fn grow_dm(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    actor: &[u8],
    additions: &[&[u8]],
) -> Result<DmGrowth> {
    for pk in additions {
        if pk.len() != 32 {
            return Err(DbError::InvalidData(format!(
                "pubkey must be 32 bytes, got {}",
                pk.len()
            )));
        }
    }

    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    crate::channel_members::lock_channel_for_member_put(&mut tx, channel_id).await?;

    let channel = sqlx::query(
        "SELECT channel_type::text AS channel_type, archived_at FROM channels \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(tx.conn())
    .await?
    .ok_or(DbError::ChannelNotFound(channel_id))?;
    let channel_type: String = channel.try_get("channel_type")?;
    if channel_type != "dm" {
        return Err(DbError::InvalidData("channel is not a DM".to_string()));
    }
    if channel
        .try_get::<Option<DateTime<Utc>>, _>("archived_at")?
        .is_some()
    {
        return Err(DbError::AccessDenied("DM is archived".to_string()));
    }

    let members: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT pubkey FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 AND removed_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_all(tx.conn())
    .await?;
    if !members.iter().any(|m| m == actor) {
        return Err(DbError::AccessDenied("not a member of this DM".to_string()));
    }

    let mut added: Vec<&[u8]> = Vec::new();
    for pk in additions {
        if !members.iter().any(|m| m == pk) && !added.contains(pk) {
            added.push(pk);
        }
    }
    if added.is_empty() {
        tx.commit().await?;
        return Ok(DmGrowth::Grown { added: Vec::new() });
    }

    let mut grown: Vec<&[u8]> = members.iter().map(Vec::as_slice).collect();
    grown.extend(&added);
    let agents = agents_among(tx.conn(), community_id, &grown).await?;
    if count_people(&grown, &agents) > MAX_DM_PEOPLE {
        return Err(DbError::InvalidData(format!(
            "DM supports at most {MAX_DM_PEOPLE} people"
        )));
    }

    // People move the DM's identity to the grown set of people. So do agents
    // joining a DM of at most one person, which is identified by all members.
    let hash = dm_identity_hash(&grown, &agents);
    let current: Vec<&[u8]> = members.iter().map(Vec::as_slice).collect();
    if hash != dm_identity_hash(&current, &agents) {
        let existing = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM channels \
             WHERE community_id = $1 AND participant_hash = $2 AND id <> $3 \
               AND deleted_at IS NULL",
        )
        .bind(community_id.as_uuid())
        .bind(hash.as_slice())
        .bind(channel_id)
        .fetch_optional(tx.conn())
        .await?;
        if let Some(existing) = existing {
            tx.rollback().await?;
            return existing_for(pool, community_id, existing, actor).await;
        }
        let moved = sqlx::query(
            "UPDATE channels SET participant_hash = $3, updated_at = NOW() \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(hash.as_slice())
        .execute(tx.conn())
        .await;
        // A DM created or grown into the same people since the check above.
        if let Err(sqlx::Error::Database(e)) = &moved {
            if e.code().as_deref() == Some("23505") {
                tx.rollback().await?;
                return match find_dm_by_participants(pool, community_id, &hash).await? {
                    Some(existing) => existing_for(pool, community_id, existing.id, actor).await,
                    None => Err(DbError::InvalidData(
                        "a DM with these people already exists".to_string(),
                    )),
                };
            }
        }
        moved?;
    }

    for pk in &added {
        sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
            VALUES ($1, $2, $3, 'member', $4)
            ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET
                sync_modified_at = clock_timestamp(),
                removed_at = NULL,
                removed_by = NULL,
                hidden_at = NULL,
                role = EXCLUDED.role
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(*pk)
        .bind(actor)
        .execute(tx.conn())
        .await?;
    }

    tx.commit().await?;
    Ok(DmGrowth::Grown {
        added: added.into_iter().map(<[u8]>::to_vec).collect(),
    })
}

/// Keep `channel_id`'s fingerprint, if it is a DM, true to who is still in it
/// after a member was removed. When the remaining members no longer have that
/// fingerprint (a person left, or the DM was identified by all its members),
/// the DM stops being any group's DM: it keeps its history for those still in
/// it, and opening that group again starts a new DM. Removing an agent from a
/// DM of people changes nothing. The caller holds the channel's membership
/// lock, as every membership writer does, so this cannot race [`grow_dm`].
pub(crate) async fn release_dm_identity(
    conn: &mut sqlx::PgConnection,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<()> {
    let stored: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT participant_hash FROM channels \
         WHERE community_id = $1 AND id = $2 AND channel_type = 'dm'",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    let Some(stored) = stored else {
        return Ok(());
    };
    let members: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT pubkey FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 AND removed_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_all(&mut *conn)
    .await?;
    let members: Vec<&[u8]> = members.iter().map(Vec::as_slice).collect();
    let agents = agents_among(&mut *conn, community_id, &members).await?;
    if dm_identity_hash(&members, &agents).as_slice() == stored.as_slice() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE channels SET participant_hash = NULL \
         WHERE community_id = $1 AND id = $2",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// [`DmGrowth::Existing`] for `actor`, who must be an active member of that
/// DM: growing a DM must not reveal a DM the actor is not in.
async fn existing_for(
    pool: &PgPool,
    community_id: CommunityId,
    existing: Uuid,
    actor: &[u8],
) -> Result<DmGrowth> {
    if crate::channel_members::is_member(pool, community_id, existing, actor).await? {
        Ok(DmGrowth::Existing(existing))
    } else {
        Err(DbError::AccessDenied(
            "a DM with these people already exists".to_string(),
        ))
    }
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
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
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
    .execute(tx.conn())
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::NotFound(format!(
            "no active membership for channel {channel_id}"
        )));
    }

    tx.commit().await?;
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
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
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
    .execute(tx.conn())
    .await?;

    tx.commit().await?;
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
        labels: row.try_get("labels")?,
        join_role: row.try_get("join_role")?,
        unlisted: row.try_get("unlisted")?,
        auto_join: row.try_get("auto_join")?,
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
    ) -> Result<(ChannelRecord, bool)> {
        crate::dm::open_dm(&self.pool, community_id, pubkeys, created_by).await
    }

    /// Add members to a DM in place, keeping its history.
    #[datastore_span(name = "grow_dm", system = "postgresql")]
    pub async fn grow_dm(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        actor: &[u8],
        additions: &[&[u8]],
    ) -> Result<DmGrowth> {
        crate::dm::grow_dm(&self.pool, community_id, channel_id, actor, additions).await
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
