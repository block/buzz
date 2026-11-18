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

/// The fingerprint a DM of `members` gets when opened. With `by_people`, the
/// DM of its people ([`dm_identity_hash`]). Otherwise the DM of exactly these
/// members, as before agents were told apart: only callers who opt in decide
/// which DM their people share.
fn opened_identity_hash(members: &[&[u8]], agents: &HashSet<Vec<u8>>, by_people: bool) -> [u8; 32] {
    if by_people {
        dm_identity_hash(members, agents)
    } else {
        compute_participant_hash(members)
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

/// Decides whether the caller may put an agent, given its
/// `channel_add_policy` and owner, into a DM with others. `Err` carries the
/// refusal.
pub type AgentAddPolicy<'a> = &'a (dyn Fn(&str, &[u8]) -> std::result::Result<(), String> + Sync);

/// Refuse unless every agent in `joining`, other than `actor`, may join by
/// `policy`. The refusal names the first agent refused, in the order of
/// `joining`, as `(agent <hex pubkey>)` at its end, so clients can say whose
/// setting blocked it.
///
/// Callers run this in the same transaction, and under the same locks, as the
/// write it guards, so it sees exactly who joins.
async fn check_joining_agents(
    conn: &mut sqlx::PgConnection,
    community_id: CommunityId,
    actor: &[u8],
    joining: &[&[u8]],
    policy: AgentAddPolicy<'_>,
) -> Result<()> {
    let joining: Vec<Vec<u8>> = joining
        .iter()
        .filter(|pk| **pk != actor)
        .map(|pk| pk.to_vec())
        .collect();
    if joining.is_empty() {
        return Ok(());
    }
    let rows = sqlx::query(
        "SELECT pubkey, channel_add_policy::text AS policy, agent_owner_pubkey AS owner \
         FROM users \
         WHERE community_id = $1 AND pubkey = ANY($2) AND agent_owner_pubkey IS NOT NULL \
         ORDER BY array_position($2, pubkey)",
    )
    .bind(community_id.as_uuid())
    .bind(&joining)
    .fetch_all(&mut *conn)
    .await?;
    for row in rows {
        let agent: Vec<u8> = row.try_get("pubkey")?;
        let agent_policy: String = row.try_get("policy")?;
        let owner: Vec<u8> = row.try_get("owner")?;
        policy(&agent_policy, &owner).map_err(|refusal| {
            DbError::AccessDenied(format!("{refusal} (agent {})", hex::encode(&agent)))
        })?;
    }
    Ok(())
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
    find_dm_in(pool, community_id, participant_hash).await
}

/// [`find_dm_by_participants`] on any executor, such as a transaction.
async fn find_dm_in<'e>(
    executor: impl sqlx::PgExecutor<'e>,
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
    .fetch_optional(executor)
    .await?;

    row.map(row_to_channel_record).transpose()
}

/// The existing DM that opening a DM of `participants`, of whom `agents` are
/// agents, finds. With `by_people`, the DM of their people, or one made before
/// agents were told apart, identified by all its members. Without, the DM
/// identified by all of them, which for people alone is their DM, with any
/// agents that joined it: there is one DM per group of people. Failing that,
/// their people's DM when it holds exactly them now, as a DM that agents
/// joined in place does.
async fn find_opened_dm(
    conn: &mut sqlx::PgConnection,
    community_id: CommunityId,
    participants: &[&[u8]],
    agents: &HashSet<Vec<u8>>,
    by_people: bool,
) -> Result<Option<ChannelRecord>> {
    let people = dm_identity_hash(participants, agents);
    let exact = compute_participant_hash(participants);
    let (first, second) = if by_people {
        (people, exact)
    } else {
        (exact, people)
    };
    if let Some(found) = find_dm_in(&mut *conn, community_id, &first).await? {
        return Ok(Some(found));
    }
    if second == first {
        return Ok(None);
    }
    let Some(found) = find_dm_in(&mut *conn, community_id, &second).await? else {
        return Ok(None);
    };
    if by_people {
        return Ok(Some(found));
    }
    let members: HashSet<Vec<u8>> = sqlx::query_scalar(
        "SELECT pubkey FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 AND removed_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(found.id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    let wanted: HashSet<Vec<u8>> = participants.iter().map(|pk| pk.to_vec()).collect();
    Ok((members == wanted).then_some(found))
}

/// Create a new DM channel for the given participant pubkeys, or return the
/// existing one if a DM with the same participant set already exists.
///
/// Rules:
/// - `participants` must contain at least 2 entries, and at most
///   [`MAX_DM_PEOPLE`] people; agents do not count (enforced here).
/// - `created_by` must be one of the participants.
/// - The operation is idempotent: the same identity (see
///   [`opened_identity_hash`] for `by_people`) -> the same channel returned.
/// - A new DM's agents need `policy`'s permission ([`check_joining_agents`]),
///   except in a DM of just `created_by` and one agent: messaging an agent
///   alone is always allowed.
///
/// Returns `(channel, was_created)`; `was_created` is false when the DM
/// already existed, including when a concurrent open made it first.
pub async fn create_dm(
    pool: &PgPool,
    community_id: CommunityId,
    participants: &[&[u8]],
    created_by: &[u8],
    by_people: bool,
    policy: AgentAddPolicy<'_>,
) -> Result<(ChannelRecord, bool)> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let found = find_or_create_dm(
        tx.conn(),
        community_id,
        participants,
        created_by,
        by_people,
        policy,
    )
    .await?;
    tx.commit().await?;
    Ok(found)
}

/// Create or reuse the DM of exactly `participants` without changing who has
/// it hidden, as a workflow sending a DM needs: only the interactive open
/// changes visibility. True only when this call made the DM. A new DM's agents
/// need no permission, as a workflow DMs its owner directly.
pub async fn create_dm_if_missing(
    pool: &PgPool,
    community_id: CommunityId,
    participants: &[&[u8]],
    created_by: &[u8],
) -> Result<(ChannelRecord, bool)> {
    create_dm(
        pool,
        community_id,
        participants,
        created_by,
        false,
        &|_: &str, _: &[u8]| Ok(()),
    )
    .await
}

/// [`create_dm`] on the caller's transaction, so a caller holding a DM's lock
/// makes the new DM without another connection.
async fn find_or_create_dm(
    conn: &mut sqlx::PgConnection,
    community_id: CommunityId,
    participants: &[&[u8]],
    created_by: &[u8],
    by_people: bool,
    policy: AgentAddPolicy<'_>,
) -> Result<(ChannelRecord, bool)> {
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
    let agents = agents_among(&mut *conn, community_id, participants).await?;
    if count_people(participants, &agents) > MAX_DM_PEOPLE {
        return Err(DbError::InvalidData(format!(
            "DM supports at most {MAX_DM_PEOPLE} people"
        )));
    }

    if let Some(existing) =
        find_opened_dm(&mut *conn, community_id, participants, &agents, by_people).await?
    {
        return Ok((existing, false));
    }
    let hash = opened_identity_hash(participants, &agents, by_people);
    // Messaging one agent alone needs no permission: a new direct DM shares
    // no one's history.
    let direct = participants.len() == 2 && participants.contains(&created_by);
    if !direct {
        check_joining_agents(conn, community_id, created_by, participants, policy).await?;
    }

    // Name the DM based on participant count.
    let name = if participants.len() == 2 {
        "DM".to_string()
    } else {
        format!("Group DM ({})", participants.len())
    };

    let id = Uuid::new_v4();

    // A savepoint, so losing to a concurrent open of the same DM leaves the
    // transaction usable.
    let mut insert = sqlx::Connection::begin(&mut *conn).await?;
    let inserted = sqlx::query(
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
    .execute(&mut *insert)
    .await;
    match inserted {
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23505") => {
            insert.rollback().await?;
            return match find_dm_in(&mut *conn, community_id, &hash).await? {
                Some(existing) => Ok((existing, false)),
                None => Err(DbError::InvalidData(
                    "a DM with these members already exists".to_string(),
                )),
            };
        }
        inserted => {
            inserted?;
            insert.commit().await?;
        }
    }

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
        .execute(&mut *conn)
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
    .fetch_one(&mut *conn)
    .await?;

    Ok((row_to_channel_record(row)?, true))
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
/// With `by_people`, the DM is found by its people ([`dm_identity_hash`]), so
/// it may hold agents that `pubkeys` does not name, and lack agents it does.
/// Adding those is [`grow_dm`]'s job. A DM made before agents were told apart
/// is still found by all its members. Without `by_people`, the DM found is
/// the one of exactly these members, agents included, or, when they are all
/// people, their DM, which agents may have joined: there is one DM per group
/// of people. A DM made is one of exactly these members. A found DM is
/// returned only to an active member of it, so naming a DM's people cannot be
/// used to see or join it. A new DM's agents need `policy`'s permission.
///
/// Returns `(channel, was_created)`:
/// - `was_created = true`  -- a new DM was created.
/// - `was_created = false` -- an existing DM was returned.
pub async fn open_dm(
    pool: &PgPool,
    community_id: CommunityId,
    pubkeys: &[&[u8]],
    created_by: &[u8],
    by_people: bool,
    policy: AgentAddPolicy<'_>,
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

    // Check for existing DM first (fast path, no transaction).
    let mut conn = pool.acquire().await?;
    let existing = find_opened_dm(&mut conn, community_id, &all, &agents, by_people).await?;
    drop(conn);
    if let Some(existing) = existing {
        return reopen_dm(pool, community_id, existing, created_by).await;
    }

    let (channel, created) =
        create_dm(pool, community_id, &all, created_by, by_people, policy).await?;
    if created {
        Ok((channel, true))
    } else {
        // A concurrent open made it first.
        reopen_dm(pool, community_id, channel, created_by).await
    }
}

/// Return the existing DM `existing` to `caller`, who must be an active member
/// of it, and show it in their sidebar again.
async fn reopen_dm(
    pool: &PgPool,
    community_id: CommunityId,
    existing: ChannelRecord,
    caller: &[u8],
) -> Result<(ChannelRecord, bool)> {
    if !crate::channel_members::is_member(pool, community_id, existing.id, caller).await? {
        return Err(DbError::AccessDenied(
            "a DM with these people already exists".to_string(),
        ));
    }
    unhide_dm(pool, community_id, existing.id, caller).await?;
    Ok((existing, false))
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
    /// This DM has at most one person, and the additions would bring it to
    /// two or more. Such a DM never gains people in place; the caller starts
    /// a DM of everyone instead. Nothing was written.
    PeopleStartNewDm,
    /// People would join this DM of two or more people, and the caller asked
    /// first: they choose whether those people see its history. Nothing was
    /// written.
    NeedsChoice,
}

/// Add `additions` to the DM `channel_id` in place, keeping its history.
///
/// Agents join a DM of two or more people without changing which DM it is.
/// Otherwise the DM's identity moves to the grown set ([`dm_identity_hash`]), so
/// opening a DM with the original members afterwards starts a new one. If
/// people join and a DM of the grown set already exists, nothing is written and
/// that DM is returned as [`DmGrowth::Existing`], provided `actor` is in it;
/// agents alone join in place, and the DM keeps the identity it has. People never
/// join a DM of at most one person: that is [`DmGrowth::PeopleStartNewDm`].
///
/// The DM's stored fingerprint is brought up to date even when nothing is
/// added, so a DM still identified by all its members (from before agents
/// were told apart) becomes its people's DM. A DM that gave its identity up
/// ([`release_dm_identity`]) takes a new one only when people join. Each agent
/// that joins needs `policy`'s permission ([`check_joining_agents`]).
///
/// With `ask`, people joining a DM of two or more people are not added: that
/// is [`DmGrowth::NeedsChoice`], so the caller can ask whether they see its
/// history. Agents alone still join, and a DM of at most one person still
/// starts a new DM.
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
    policy: AgentAddPolicy<'_>,
    ask: bool,
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

    let members = locked_dm_members(tx.conn(), community_id, channel_id, actor).await?;
    let stored: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT participant_hash FROM channels WHERE community_id = $1 AND id = $2",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_one(tx.conn())
    .await?;

    let mut added: Vec<&[u8]> = Vec::new();
    for pk in additions {
        if !members.iter().any(|m| m == pk) && !added.contains(pk) {
            added.push(pk);
        }
    }

    let mut grown: Vec<&[u8]> = members.iter().map(Vec::as_slice).collect();
    grown.extend(&added);
    let agents = agents_among(tx.conn(), community_id, &grown).await?;
    if count_people(&grown, &agents) > MAX_DM_PEOPLE {
        return Err(DbError::InvalidData(format!(
            "DM supports at most {MAX_DM_PEOPLE} people"
        )));
    }

    // A DM of at most one person and their agents stays theirs: people
    // joining it start a DM of everyone instead.
    let current: Vec<&[u8]> = members.iter().map(Vec::as_slice).collect();
    if count_people(&current, &agents) < 2 && count_people(&grown, &agents) >= 2 {
        tx.rollback().await?;
        return Ok(DmGrowth::PeopleStartNewDm);
    }
    let people_join = count_people(&grown, &agents) != count_people(&current, &agents);
    if ask && people_join {
        tx.rollback().await?;
        return Ok(DmGrowth::NeedsChoice);
    }

    // The DM takes the grown set's identity: its people's, or all its members'
    // when it has at most one person. If another DM has it, people meet there;
    // agents join this DM in place, and it stops being any group's DM, since
    // the identity it had no longer matches who is in it.
    let hash = dm_identity_hash(&grown, &agents);
    let moves = match &stored {
        Some(stored) => stored.as_slice() != hash.as_slice(),
        None => people_join,
    };
    if moves && !claim_dm_identity(tx.conn(), community_id, channel_id, &hash).await? {
        if people_join {
            tx.rollback().await?;
            return match find_dm_by_participants(pool, community_id, &hash).await? {
                Some(existing) => existing_for(pool, community_id, existing.id, actor).await,
                None => Err(DbError::InvalidData(
                    "a DM with these people already exists".to_string(),
                )),
            };
        }
        sqlx::query(
            "UPDATE channels SET participant_hash = NULL, updated_at = NOW() \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .execute(tx.conn())
        .await?;
    }

    // Checked only once they are sure to join this DM, not one of its people
    // have elsewhere.
    check_joining_agents(tx.conn(), community_id, actor, &added, policy).await?;
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

/// Start a DM of the people in the DM `source` plus `additions`, without its
/// history: what adding people to a DM does when they do not join it in place.
/// Agents already in `source` stay behind and can be added later; agents among
/// `additions` join, with `policy`'s permission.
///
/// `source` must be a live, unarchived DM with `actor` as an active member.
/// Its members are read under its membership lock, held until the new DM is
/// made on the same transaction, so the new DM holds exactly who is in
/// `source` now. If a DM of these members already exists, found as
/// [`open_dm`] finds it (by its people with `by_people`, when people join), it
/// is returned to `actor` if they are in it, and shown in their sidebar; named
/// agents it lacks are the caller's to add.
///
/// Returns `(channel, was_created, participants)`.
pub async fn fork_dm(
    pool: &PgPool,
    community_id: CommunityId,
    source: Uuid,
    actor: &[u8],
    additions: &[&[u8]],
    by_people: bool,
    policy: AgentAddPolicy<'_>,
) -> Result<(ChannelRecord, bool, Vec<Vec<u8>>)> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    crate::channel_members::lock_channel_for_member_put(&mut tx, source).await?;
    let source_members = locked_dm_members(tx.conn(), community_id, source, actor).await?;
    let source_refs: Vec<&[u8]> = source_members.iter().map(Vec::as_slice).collect();
    let agents = agents_among(tx.conn(), community_id, &source_refs).await?;
    let added_agents = agents_among(tx.conn(), community_id, additions).await?;
    // Agents alone do not change who the DM is for, so they start a DM of
    // exactly these members, as they always have.
    let people_join = additions
        .iter()
        .any(|pk| !added_agents.contains(*pk) && !source_members.iter().any(|m| m == pk));
    let mut members: Vec<Vec<u8>> = source_members
        .into_iter()
        .filter(|pk| pk.as_slice() == actor || !agents.contains(pk))
        .collect();
    for pk in additions {
        if !members.iter().any(|m| m == pk) {
            members.push(pk.to_vec());
        }
    }
    let refs: Vec<&[u8]> = members.iter().map(Vec::as_slice).collect();
    let (channel, created) = find_or_create_dm(
        tx.conn(),
        community_id,
        &refs,
        actor,
        by_people && people_join,
        policy,
    )
    .await?;
    if !created {
        let shown = sqlx::query(
            "UPDATE channel_members SET hidden_at = NULL \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 AND removed_at IS NULL",
        )
        .bind(community_id.as_uuid())
        .bind(channel.id)
        .bind(actor)
        .execute(tx.conn())
        .await?;
        if shown.rows_affected() == 0 {
            return Err(DbError::AccessDenied(
                "a DM with these people already exists".to_string(),
            ));
        }
    }
    tx.commit().await?;
    Ok((channel, created, members))
}

/// The active members of `channel_id`, read under its membership lock, after
/// checking that it is a live, unarchived DM and that `actor` is one of them.
async fn locked_dm_members(
    conn: &mut sqlx::PgConnection,
    community_id: CommunityId,
    channel_id: Uuid,
    actor: &[u8],
) -> Result<Vec<Vec<u8>>> {
    let channel = sqlx::query(
        "SELECT channel_type::text AS channel_type, archived_at FROM channels \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(&mut *conn)
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
    .fetch_all(&mut *conn)
    .await?;
    if !members.iter().any(|m| m == actor) {
        return Err(DbError::AccessDenied("not a member of this DM".to_string()));
    }
    Ok(members)
}

/// Give `channel_id` the identity `hash` unless another live channel has it.
/// Returns whether it did. Runs under the DM's membership lock.
async fn claim_dm_identity(
    conn: &mut sqlx::PgConnection,
    community_id: CommunityId,
    channel_id: Uuid,
    hash: &[u8],
) -> Result<bool> {
    let taken = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM channels \
         WHERE community_id = $1 AND participant_hash = $2 AND id <> $3 \
           AND deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(hash)
    .bind(channel_id)
    .fetch_optional(&mut *conn)
    .await?;
    if taken.is_some() {
        return Ok(false);
    }
    // A savepoint, so losing the claim to a DM created or grown into the same
    // identity since the check above leaves the transaction usable.
    let mut claim = sqlx::Connection::begin(&mut *conn).await?;
    let moved = sqlx::query(
        "UPDATE channels SET participant_hash = $3, updated_at = NOW() \
         WHERE community_id = $1 AND id = $2",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(hash)
    .execute(&mut *claim)
    .await;
    match moved {
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23505") => {
            claim.rollback().await?;
            Ok(false)
        }
        moved => {
            moved?;
            claim.commit().await?;
            Ok(true)
        }
    }
}

/// Keep `channel_id`'s fingerprint, if it is a DM, true to who is still in it
/// after `removed` left or was removed. When a person left, the DM stops being
/// any group's DM: it keeps its history for those still in it, and opening
/// that group again starts a new DM. When an agent left a DM of two or more
/// people, its people are unchanged, so it stays (or, if it was identified by
/// all its members, becomes) their DM, unless they already have another, in
/// which case it stops being any group's DM. The caller holds the channel's
/// membership lock, as every membership writer does, so this cannot race
/// [`grow_dm`].
pub(crate) async fn release_dm_identity(
    conn: &mut sqlx::PgConnection,
    community_id: CommunityId,
    channel_id: Uuid,
    removed: &[u8],
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
    let mut members: Vec<&[u8]> = members.iter().map(Vec::as_slice).collect();
    members.push(removed);
    let agents = agents_among(&mut *conn, community_id, &members).await?;
    members.pop();
    let hash = dm_identity_hash(&members, &agents);
    if hash.as_slice() == stored.as_slice() {
        return Ok(());
    }
    if agents.contains(removed)
        && count_people(&members, &agents) >= 2
        && claim_dm_identity(&mut *conn, community_id, channel_id, &hash).await?
    {
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
        by_people: bool,
        policy: AgentAddPolicy<'_>,
    ) -> Result<(ChannelRecord, bool)> {
        crate::dm::create_dm(
            &self.pool,
            community_id,
            participants,
            created_by,
            by_people,
            policy,
        )
        .await
    }

    /// Create or reuse a DM without unhiding it; true only for the creator.
    #[datastore_span(name = "create_dm_if_missing", system = "postgresql")]
    pub async fn create_dm_if_missing(
        &self,
        community_id: CommunityId,
        participants: &[&[u8]],
        created_by: &[u8],
    ) -> Result<(ChannelRecord, bool)> {
        crate::dm::create_dm_if_missing(&self.pool, community_id, participants, created_by).await
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
        by_people: bool,
        policy: AgentAddPolicy<'_>,
    ) -> Result<(ChannelRecord, bool)> {
        crate::dm::open_dm(
            &self.pool,
            community_id,
            pubkeys,
            created_by,
            by_people,
            policy,
        )
        .await
    }

    /// Add members to a DM in place, keeping its history.
    #[datastore_span(name = "grow_dm", system = "postgresql")]
    pub async fn grow_dm(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        actor: &[u8],
        additions: &[&[u8]],
        policy: AgentAddPolicy<'_>,
        ask: bool,
    ) -> Result<DmGrowth> {
        crate::dm::grow_dm(
            &self.pool,
            community_id,
            channel_id,
            actor,
            additions,
            policy,
            ask,
        )
        .await
    }

    /// Start a DM of everyone in a DM plus `additions`, without its history,
    /// or return the DM they already have.
    #[datastore_span(name = "fork_dm", system = "postgresql")]
    pub async fn fork_dm(
        &self,
        community_id: CommunityId,
        source: Uuid,
        actor: &[u8],
        additions: &[&[u8]],
        by_people: bool,
        policy: AgentAddPolicy<'_>,
    ) -> Result<(ChannelRecord, bool, Vec<Vec<u8>>)> {
        crate::dm::fork_dm(
            &self.pool,
            community_id,
            source,
            actor,
            additions,
            by_people,
            policy,
        )
        .await
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
#[path = "dm_postgres_tests.rs"]
mod postgres_tests;

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
