//! NIP-CI conversation import sessions.
//!
//! A session is opened by a kind:42110 proposal and moved by 42111 consent,
//! 42112 withdraw and 42113 complete events. Each control event, its session
//! row change and the relay-signed kind:30623 snapshot commit in one admitted
//! transaction. Imported kind:9 and kind:7 events are inserted under a
//! `FOR UPDATE` lock on the session row, so the acceptance rules and the
//! per-participant counters cannot race.
//!
//! Status is derived lazily: an expired session is only marked `expired` the
//! next time one of these paths reads it, and that transition republishes the
//! snapshot.

use std::collections::BTreeMap;

use buzz_core::import::{
    self, reason, AuthorFacts, ImportCounts, ImportDecision, ImportProposalContent, ImportRange,
    ImportSessionSnapshotContent, ImportSessionStatus, ImportTags, ParticipantState, SessionFacts,
};
use buzz_core::kind::{KIND_IMPORT_SESSION_SNAPSHOT, KIND_REACTION};
use buzz_core::{CommunityId, StoredEvent};
use chrono::{DateTime, Utc};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::event::{insert_event_with_thread_metadata_tx, ThreadMetadataParams};
use crate::{AdmittedTx, Db, DbError, Result};

/// Outcome of an import control event (kinds 42110 to 42113).
#[derive(Debug)]
pub enum ImportControlOutcome {
    /// The event violates a NIP-CI rule; nothing was written.
    Rejected(&'static str),
    /// The event was refused, but the check itself moved the session (for
    /// example to `expired`); the republished snapshot still needs delivery.
    RejectedAfterTransition {
        /// Exact relay rejection reason.
        reason: &'static str,
        /// The republished kind:30623 snapshot.
        snapshot: Box<StoredEvent>,
    },
    /// The exact event was already stored; nothing changed.
    Duplicate,
    /// The event, the session change and the snapshot committed together.
    Applied {
        /// The stored control event.
        stored_event: Box<StoredEvent>,
        /// The republished kind:30623 snapshot.
        snapshot: Box<StoredEvent>,
    },
}

/// Outcome of inserting one imported kind:9 or kind:7 event.
#[derive(Debug)]
pub enum ImportInsertOutcome {
    /// The event violates a NIP-CI acceptance rule. `snapshot` is set when
    /// the check itself moved the session (for example to `expired`).
    Rejected {
        /// Exact relay rejection reason.
        reason: &'static str,
        /// Snapshot republished by a lazy status transition, if any.
        snapshot: Option<Box<StoredEvent>>,
    },
    /// A reaction whose target is absent or soft-deleted.
    TargetMissing,
    /// The event (or the active reaction it represents) already exists.
    Duplicate,
    /// The event and its counter increment committed.
    Inserted {
        /// The stored imported event.
        stored_event: Box<StoredEvent>,
        /// Snapshot republished at a 50-event boundary, if any.
        snapshot: Option<Box<StoredEvent>>,
    },
}

/// Reaction row parameters for an imported kind:7.
#[derive(Debug, Clone, Copy)]
pub struct ImportReaction<'a> {
    /// Target event id bytes.
    pub target_event_id: &'a [u8],
    /// Reacting pubkey bytes.
    pub actor_pubkey: &'a [u8],
    /// Validated emoji content.
    pub emoji: &'a str,
}

/// Read-only view of a session for the relay's pre-insert rule check.
#[derive(Debug, Clone)]
pub struct ImportPreflight {
    /// Destination channel of the session.
    pub channel_id: Uuid,
    /// Declared time range.
    pub range: ImportRange,
    /// Status derived at the time of the read.
    pub status: ImportSessionStatus,
    /// The event author's participant facts, `None` when not a participant.
    pub author: Option<AuthorFacts>,
    /// The exact event is already stored; storage will report a duplicate.
    pub already_stored: bool,
    /// The proposal's declared source type.
    pub source_type: String,
}

struct SessionRow {
    channel_id: Uuid,
    proposer: Vec<u8>,
    manifest: String,
    source_type: String,
    is_dm: bool,
    range: ImportRange,
    expires_at: i64,
    requires_admin: bool,
    admin_consent: Option<ImportDecision>,
    status: ImportSessionStatus,
}

struct ParticipantRow {
    pubkey: Vec<u8>,
    source_user: String,
    declared: ImportCounts,
    consent: Option<ImportDecision>,
    published: ImportCounts,
    completed: bool,
}

fn invalid(what: &str) -> DbError {
    DbError::InvalidData(format!("import session: {what}"))
}

fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| invalid("count out of range"))
}

/// Whether any of `pubkeys` is an agent (a user with an owner).
async fn any_agent(tx: &mut AdmittedTx, pubkeys: &[Vec<u8>]) -> Result<bool> {
    let community = tx.community();
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users \
         WHERE community_id = $1 AND pubkey = ANY($2) AND agent_owner_pubkey IS NOT NULL)",
    )
    .bind(community.as_uuid())
    .bind(pubkeys)
    .fetch_one(tx.conn())
    .await?)
}

/// Whether a current member of `channel_id` is an agent.
async fn channel_has_agent_member(tx: &mut AdmittedTx, channel_id: Uuid) -> Result<bool> {
    let community = tx.community();
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM channel_members m \
         JOIN users u ON u.community_id = m.community_id AND u.pubkey = m.pubkey \
         WHERE m.community_id = $1 AND m.channel_id = $2 AND m.removed_at IS NULL \
           AND u.agent_owner_pubkey IS NOT NULL)",
    )
    .bind(community.as_uuid())
    .bind(channel_id)
    .fetch_one(tx.conn())
    .await?)
}

/// Why an active session's destination no longer admits imports: a DM whose
/// member set differs from the participant set, or a DM or private channel
/// that gained an agent member.
async fn destination_drift(
    tx: &mut AdmittedTx,
    session: &SessionRow,
    participants: &[ParticipantRow],
) -> Result<Option<&'static str>> {
    let community = tx.community();
    let (visibility, members): (String, Vec<Vec<u8>>) = sqlx::query_as(
        "SELECT c.visibility::text, \
                COALESCE(array_agg(m.pubkey) FILTER (WHERE m.pubkey IS NOT NULL), '{}') \
         FROM channels c \
         LEFT JOIN channel_members m ON m.community_id = c.community_id \
              AND m.channel_id = c.id AND m.removed_at IS NULL \
         WHERE c.community_id = $1 AND c.id = $2 \
         GROUP BY c.visibility",
    )
    .bind(community.as_uuid())
    .bind(session.channel_id)
    .fetch_one(tx.conn())
    .await?;
    if session.is_dm {
        let mut expected: Vec<&[u8]> = participants.iter().map(|p| p.pubkey.as_slice()).collect();
        let mut actual: Vec<&[u8]> = members.iter().map(Vec::as_slice).collect();
        expected.sort_unstable();
        actual.sort_unstable();
        if expected != actual {
            return Ok(Some(reason::DM_PARTICIPANTS_MISMATCH));
        }
    }
    if (session.is_dm || visibility == "private")
        && channel_has_agent_member(tx, session.channel_id).await?
    {
        return Ok(Some(reason::DESTINATION_HAS_AGENTS));
    }
    Ok(None)
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    hex::decode(value).map_err(|_| invalid("hex value"))
}

fn parse_decision(value: Option<String>) -> Result<Option<ImportDecision>> {
    value
        .map(|v| ImportDecision::parse(&v).ok_or_else(|| invalid("consent value")))
        .transpose()
}

async fn load_session(
    tx: &mut AdmittedTx,
    proposal_id: &[u8],
    lock: bool,
) -> Result<Option<SessionRow>> {
    let community = tx.community();
    read_session(tx.conn(), community, proposal_id, lock).await
}

async fn read_session(
    conn: &mut PgConnection,
    community: CommunityId,
    proposal_id: &[u8],
    lock: bool,
) -> Result<Option<SessionRow>> {
    let sql = if lock {
        "SELECT channel_id, proposer, manifest, source_type, is_dm, range_from, range_to, \
         expires_at, requires_admin, admin_consent, status FROM import_sessions \
         WHERE community_id = $1 AND proposal_id = $2 FOR UPDATE"
    } else {
        "SELECT channel_id, proposer, manifest, source_type, is_dm, range_from, range_to, \
         expires_at, requires_admin, admin_consent, status FROM import_sessions \
         WHERE community_id = $1 AND proposal_id = $2"
    };
    let Some(row) = sqlx::query(sql)
        .bind(community.as_uuid())
        .bind(proposal_id)
        .fetch_optional(conn)
        .await?
    else {
        return Ok(None);
    };
    let status: String = row.try_get("status")?;
    Ok(Some(SessionRow {
        channel_id: row.try_get("channel_id")?,
        proposer: row.try_get("proposer")?,
        manifest: row.try_get("manifest")?,
        source_type: row.try_get("source_type")?,
        is_dm: row.try_get("is_dm")?,
        range: ImportRange {
            from: row.try_get("range_from")?,
            to: row.try_get("range_to")?,
        },
        expires_at: row.try_get("expires_at")?,
        requires_admin: row.try_get("requires_admin")?,
        admin_consent: parse_decision(row.try_get("admin_consent")?)?,
        status: ImportSessionStatus::parse(&status).ok_or_else(|| invalid("status value"))?,
    }))
}

async fn load_participants(tx: &mut AdmittedTx, proposal_id: &[u8]) -> Result<Vec<ParticipantRow>> {
    let community = tx.community();
    read_participants(tx.conn(), community, proposal_id).await
}

async fn read_participants(
    conn: &mut PgConnection,
    community: CommunityId,
    proposal_id: &[u8],
) -> Result<Vec<ParticipantRow>> {
    let rows = sqlx::query(
        "SELECT pubkey, source_user, declared_messages, declared_reactions, consent, \
         published_messages, published_reactions, completed_at IS NOT NULL AS completed \
         FROM import_session_participants \
         WHERE community_id = $1 AND proposal_id = $2 ORDER BY pubkey",
    )
    .bind(community.as_uuid())
    .bind(proposal_id)
    .fetch_all(conn)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(ParticipantRow {
                pubkey: row.try_get("pubkey")?,
                source_user: row.try_get("source_user")?,
                declared: ImportCounts {
                    messages: count(row.try_get("declared_messages")?),
                    reactions: count(row.try_get("declared_reactions")?),
                },
                consent: parse_decision(row.try_get("consent")?)?,
                published: ImportCounts {
                    messages: count(row.try_get("published_messages")?),
                    reactions: count(row.try_get("published_reactions")?),
                },
                completed: row.try_get("completed")?,
            })
        })
        .collect()
}

fn facts(session: &SessionRow, participants: &[ParticipantRow]) -> SessionFacts {
    SessionFacts {
        status: session.status,
        is_dm: session.is_dm,
        requires_admin: session.requires_admin,
        admin_consent: session.admin_consent,
        expires_at: session.expires_at,
        participants: participants
            .iter()
            .map(|p| ParticipantState {
                pubkey: hex::encode(&p.pubkey),
                consent: p.consent,
                completed: p.completed,
            })
            .collect(),
    }
}

fn author_facts(participant: &ParticipantRow) -> AuthorFacts {
    AuthorFacts {
        consent: participant.consent,
        completed: participant.completed,
        declared: participant.declared,
        published: participant.published,
    }
}

async fn store_status(
    tx: &mut AdmittedTx,
    proposal_id: &[u8],
    status: ImportSessionStatus,
) -> Result<()> {
    let community = tx.community();
    sqlx::query(
        "UPDATE import_sessions SET status = $3, updated_at = now() \
         WHERE community_id = $1 AND proposal_id = $2",
    )
    .bind(community.as_uuid())
    .bind(proposal_id)
    .bind(status.as_str())
    .execute(tx.conn())
    .await?;
    Ok(())
}

/// Build the snapshot content from the session row and its participants.
fn snapshot_content(
    session: &SessionRow,
    participants: &[ParticipantRow],
    status: ImportSessionStatus,
) -> ImportSessionSnapshotContent {
    let mut required: Vec<String> = participants
        .iter()
        .map(|p| hex::encode(&p.pubkey))
        .collect();
    let mut consents = BTreeMap::new();
    let mut published = BTreeMap::new();
    let mut completed = Vec::new();
    for p in participants {
        let pubkey = hex::encode(&p.pubkey);
        if let Some(decision) = p.consent {
            consents.insert(pubkey.clone(), decision);
        }
        if p.completed {
            completed.push(pubkey.clone());
        }
        published.insert(pubkey, p.published);
    }
    if session.requires_admin {
        required.push(import::ADMIN_CONSENT_SOURCE_USER.to_string());
    }
    if let Some(decision) = session.admin_consent {
        consents.insert(import::ADMIN_CONSENT_SOURCE_USER.to_string(), decision);
    }
    ImportSessionSnapshotContent {
        v: import::IMPORT_PAYLOAD_VERSION,
        status,
        required,
        consents,
        published,
        completed,
        expires_at: session.expires_at,
    }
}

/// The relay clock in unix seconds, the clock snapshot `created_at` uses.
fn relay_now() -> u64 {
    Utc::now().timestamp().max(0) as u64
}

/// `created_at` of the newest stored snapshot for `proposal_hex`, if any.
async fn snapshot_head_at(
    tx: &mut AdmittedTx,
    proposal_hex: &str,
    keys: &Keys,
) -> Result<Option<u64>> {
    let community = tx.community();
    let previous: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT max(created_at) FROM events \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4",
    )
    .bind(community.as_uuid())
    .bind(KIND_IMPORT_SESSION_SNAPSHOT as i32)
    .bind(keys.public_key().to_bytes().as_slice())
    .bind(proposal_hex)
    .fetch_one(tx.conn())
    .await?;
    Ok(previous.map(|at| at.timestamp().max(0) as u64))
}

/// Sign and store the kind:30623 snapshot for `proposal_id` in `tx`.
async fn publish_snapshot(
    db: &Db,
    tx: &mut AdmittedTx,
    proposal_id: &[u8],
    keys: &Keys,
) -> Result<StoredEvent> {
    let session = load_session(tx, proposal_id, false)
        .await?
        .ok_or_else(|| invalid("session vanished"))?;
    let participants = load_participants(tx, proposal_id).await?;
    let content = snapshot_content(&session, &participants, session.status);
    let proposal_hex = hex::encode(proposal_id);
    let channel = session.channel_id.to_string();
    let mut values: Vec<[&str; 2]> =
        vec![["d", &proposal_hex], ["e", &proposal_hex], ["h", &channel]];
    let pubkeys: Vec<String> = participants
        .iter()
        .map(|p| hex::encode(&p.pubkey))
        .collect();
    values.extend(pubkeys.iter().map(|p| ["p", p.as_str()]));
    let tags = values
        .into_iter()
        .map(|v| Tag::parse(v).map_err(|_| invalid("snapshot tag")))
        .collect::<Result<Vec<_>>>()?;

    let created_at = snapshot_head_at(tx, &proposal_hex, keys)
        .await?
        .map(|at| at.saturating_add(1))
        .unwrap_or(0)
        .max(relay_now());

    let event = EventBuilder::new(
        Kind::Custom(KIND_IMPORT_SESSION_SNAPSHOT as u16),
        serde_json::to_string(&content)?,
    )
    .tags(tags)
    .custom_created_at(Timestamp::from(created_at))
    .sign_with_keys(keys)
    .map_err(|_| invalid("snapshot signing"))?;
    let stored = db
        .replace_parameterized_event_in_transaction(
            tx,
            &event,
            &proposal_hex,
            Some(session.channel_id),
            crate::replaceable::ParameterizedReplacePrecondition::Unconditional,
        )
        .await?;
    if !matches!(
        stored.status,
        crate::replaceable::ParameterizedReplaceStatus::Inserted
            | crate::replaceable::ParameterizedReplaceStatus::Duplicate
    ) {
        return Err(invalid("snapshot did not replace current head"));
    }
    Ok(stored.event)
}

/// Derive the status at `now`, persist it when it moved, and report whether
/// it moved.
async fn refresh_status(
    tx: &mut AdmittedTx,
    proposal_id: &[u8],
    session: &mut SessionRow,
    participants: &[ParticipantRow],
    now: i64,
) -> Result<bool> {
    let next = import::derive_status(&facts(session, participants), now);
    if next == session.status {
        return Ok(false);
    }
    store_status(tx, proposal_id, next).await?;
    session.status = next;
    Ok(true)
}

async fn insert_control_event(
    tx: &mut AdmittedTx,
    event: &Event,
    channel_id: Uuid,
) -> Result<Option<StoredEvent>> {
    let (stored, inserted) =
        insert_event_with_thread_metadata_tx(tx, event, Some(channel_id), None).await?;
    if !inserted {
        return Ok(None);
    }
    // Control records are live activity: a proposal mentions its participants
    // and a consent its proposer. Only imported history is quiet.
    crate::insert_mentions_in_transaction(tx, event, Some(channel_id)).await?;
    Ok(Some(stored))
}

/// Which unended-session cap a new proposal would exceed, if any.
///
/// Takes transaction-scoped advisory locks on the channel and the proposer
/// (always in that order) so concurrent proposals cannot both pass the count.
async fn session_cap_reached(
    tx: &mut AdmittedTx,
    channel_id: Uuid,
    proposer: &[u8],
    limits: import::ImportLimits,
    now: i64,
) -> Result<Option<&'static str>> {
    let community = tx.community();
    for key in [
        format!("buzz_import_channel:{}:{channel_id}", community.as_uuid()),
        format!(
            "buzz_import_proposer:{}:{}",
            community.as_uuid(),
            hex::encode(proposer)
        ),
    ] {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(key)
            .execute(tx.conn())
            .await?;
    }
    let (in_channel, by_proposer): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE channel_id = $2), count(*) FILTER (WHERE proposer = $3) \
         FROM import_sessions \
         WHERE community_id = $1 AND (channel_id = $2 OR proposer = $3) \
           AND status IN ('proposed', 'active') AND expires_at > $4",
    )
    .bind(community.as_uuid())
    .bind(channel_id)
    .bind(proposer)
    .bind(now)
    .fetch_one(tx.conn())
    .await?;
    if count(in_channel) >= limits.max_active_sessions_per_channel {
        return Ok(Some(reason::CHANNEL_SESSION_LIMIT));
    }
    if count(by_proposer) >= limits.max_active_sessions_per_proposer {
        return Ok(Some(reason::PROPOSER_SESSION_LIMIT));
    }
    Ok(None)
}

/// Whether `pubkey` is a current owner or admin of `channel_id`.
async fn is_channel_admin(tx: &mut AdmittedTx, channel_id: Uuid, pubkey: &[u8]) -> Result<bool> {
    let community = tx.community();
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 \
           AND removed_at IS NULL AND role IN ('owner', 'admin'))",
    )
    .bind(community.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .fetch_one(tx.conn())
    .await?)
}

impl Db {
    /// Open a session from a validated kind:42110 proposal.
    ///
    /// `content` must already pass [`import::validate_proposal_content`] and
    /// [`import::validate_proposal_tags`]; `expires_at` is the relay-capped
    /// expiry. Inside the transaction this checks that the proposer and every
    /// participant are active members of `channel_id`, that a DM's
    /// participants equal its member set exactly, and whether a channel-admin
    /// consent is required. The proposer's own participant entry starts as
    /// accepted. The channel and the proposer may each hold at most the
    /// `limits` number of unended (proposed or active, unexpired) sessions.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_import_session(
        &self,
        community: CommunityId,
        event: &Event,
        content: &ImportProposalContent,
        channel_id: Uuid,
        expires_at: i64,
        limits: import::ImportLimits,
        keys: &Keys,
    ) -> Result<ImportControlOutcome> {
        let mut tx = crate::begin_community_event_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        // Checked before the session caps so a resent stored proposal is
        // answered as a duplicate even when the channel is at its cap.
        if event_is_stored(tx.conn(), community, event).await? {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Duplicate);
        }
        let proposer = event.pubkey.to_bytes().to_vec();
        let participant_keys = content
            .participants
            .iter()
            .map(|p| decode_hex(&p.pubkey))
            .collect::<Result<Vec<_>>>()?;

        let declared = content
            .participants
            .iter()
            .map(|p| Ok((to_i64(p.messages)?, to_i64(p.reactions)?)))
            .collect::<Result<Vec<_>>>();
        let Ok(declared) = declared else {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::COUNTS_EXCEED_LIMIT));
        };

        let channel: Option<(String, String)> = sqlx::query_as(
            "SELECT channel_type::text, visibility::text FROM channels \
             WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .fetch_optional(tx.conn())
        .await?;
        let Some((channel_type, visibility)) = channel else {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::CHANNEL_MISMATCH));
        };
        let is_dm = channel_type == "dm";

        if any_agent(&mut tx, &participant_keys).await? {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(
                reason::PARTICIPANTS_INCLUDE_AGENTS,
            ));
        }
        if (is_dm || visibility == "private")
            && channel_has_agent_member(&mut tx, channel_id).await?
        {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(
                reason::DESTINATION_HAS_AGENTS,
            ));
        }

        let members = sqlx::query(
            "SELECT pubkey, role::text AS role FROM channel_members \
             WHERE community_id = $1 AND channel_id = $2 AND removed_at IS NULL \
               AND (pubkey = ANY($3) OR pubkey = $4)",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .bind(&participant_keys)
        .bind(&proposer)
        .fetch_all(tx.conn())
        .await?;
        let mut roles: BTreeMap<Vec<u8>, String> = BTreeMap::new();
        for row in members {
            roles.insert(row.try_get("pubkey")?, row.try_get("role")?);
        }
        if !roles.contains_key(&proposer) {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::PROPOSER_NOT_MEMBER));
        }
        if participant_keys.iter().any(|p| !roles.contains_key(p)) {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(
                reason::PARTICIPANT_NOT_MEMBER,
            ));
        }
        let (member_count, admin_count): (i64, i64) = sqlx::query_as(
            "SELECT count(*), count(*) FILTER (WHERE role IN ('owner', 'admin')) \
             FROM channel_members \
             WHERE community_id = $1 AND channel_id = $2 AND removed_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .fetch_one(tx.conn())
        .await?;
        // Participants are distinct and all members, so equal counts mean
        // equal sets.
        if is_dm && usize::try_from(member_count).ok() != Some(participant_keys.len()) {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(
                reason::DM_PARTICIPANTS_MISMATCH,
            ));
        }
        let proposer_is_admin = roles
            .get(&proposer)
            .is_some_and(|role| role == "owner" || role == "admin");
        let requires_admin = !is_dm && admin_count > 0 && !proposer_is_admin;

        if let Some(reason) = session_cap_reached(
            &mut tx,
            channel_id,
            &proposer,
            limits,
            Utc::now().timestamp(),
        )
        .await?
        {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason));
        }

        let Some(stored_event) = insert_control_event(&mut tx, event, channel_id).await? else {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Duplicate);
        };
        let proposal_id = event.id.as_bytes().as_slice();
        sqlx::query(
            "INSERT INTO import_sessions (community_id, proposal_id, channel_id, proposer, \
             manifest, source_type, is_dm, range_from, range_to, expires_at, requires_admin, \
             status) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(community.as_uuid())
        .bind(proposal_id)
        .bind(channel_id)
        .bind(&proposer)
        .bind(&content.manifest)
        .bind(&content.source.source_type)
        .bind(is_dm)
        .bind(content.range.from)
        .bind(content.range.to)
        .bind(expires_at)
        .bind(requires_admin)
        .bind(ImportSessionStatus::Proposed.as_str())
        .execute(tx.conn())
        .await?;
        for ((participant, pubkey), (messages, reactions)) in content
            .participants
            .iter()
            .zip(&participant_keys)
            .zip(declared)
        {
            let consent = (*pubkey == proposer).then_some(ImportDecision::Accept.as_str());
            sqlx::query(
                "INSERT INTO import_session_participants (community_id, proposal_id, pubkey, \
                 source_user, declared_messages, declared_reactions, consent) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(community.as_uuid())
            .bind(proposal_id)
            .bind(pubkey)
            .bind(&participant.source_user)
            .bind(messages)
            .bind(reactions)
            .bind(consent)
            .execute(tx.conn())
            .await?;
        }
        let mut session = load_session(&mut tx, proposal_id, true)
            .await?
            .ok_or_else(|| invalid("session insert"))?;
        let participants = load_participants(&mut tx, proposal_id).await?;
        refresh_status(
            &mut tx,
            proposal_id,
            &mut session,
            &participants,
            Utc::now().timestamp(),
        )
        .await?;
        let snapshot = publish_snapshot(self, &mut tx, proposal_id, keys).await?;
        tx.commit().await?;
        Ok(ImportControlOutcome::Applied {
            stored_event: Box::new(stored_event),
            snapshot: Box::new(snapshot),
        })
    }

    /// Record a kind:42111 consent. `manifest` is the event's `manifest` tag.
    ///
    /// A `source_user` of `"admin"` records a channel-admin consent and
    /// requires the author to be a current owner or admin of the channel.
    /// Otherwise the author must be a participant whose `source_user`
    /// matches. A later consent replaces the earlier one.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_import_consent(
        &self,
        community: CommunityId,
        event: &Event,
        proposal_id: &[u8],
        channel_id: Uuid,
        manifest: &str,
        decision: ImportDecision,
        source_user: &str,
        keys: &Keys,
    ) -> Result<ImportControlOutcome> {
        let mut tx = crate::begin_community_event_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let author = event.pubkey.to_bytes().to_vec();
        let Some(mut session) = load_session(&mut tx, proposal_id, true).await? else {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::PROPOSAL_NOT_FOUND));
        };
        if session.channel_id != channel_id {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::CHANNEL_MISMATCH));
        }
        // Checked before the lazy status refresh so a resend after expiry is
        // still answered as a duplicate.
        if event_is_stored(tx.conn(), community, event).await? {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Duplicate);
        }
        if session.manifest != manifest || !names_only(event, &session.proposer) {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::CONSENT_MISMATCH));
        }
        let mut participants = load_participants(&mut tx, proposal_id).await?;
        let now = Utc::now().timestamp();
        if refresh_status(&mut tx, proposal_id, &mut session, &participants, now).await? {
            // Persist the lazy transition even though this consent is refused.
            let snapshot = publish_snapshot(self, &mut tx, proposal_id, keys).await?;
            tx.commit().await?;
            return Ok(ImportControlOutcome::RejectedAfterTransition {
                reason: reason::SESSION_NOT_ACTIVE,
                snapshot: Box::new(snapshot),
            });
        }
        if session.status.is_terminal() {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::SESSION_NOT_ACTIVE));
        }

        if source_user == import::ADMIN_CONSENT_SOURCE_USER {
            if !is_channel_admin(&mut tx, channel_id, &author).await? {
                tx.rollback().await?;
                return Ok(ImportControlOutcome::Rejected(reason::NOT_PARTICIPANT));
            }
            sqlx::query(
                "UPDATE import_sessions SET admin_consent = $3, updated_at = now() \
                 WHERE community_id = $1 AND proposal_id = $2",
            )
            .bind(community.as_uuid())
            .bind(proposal_id)
            .bind(decision.as_str())
            .execute(tx.conn())
            .await?;
            session.admin_consent = Some(decision);
        } else {
            let Some(participant) = participants.iter_mut().find(|p| p.pubkey == author) else {
                tx.rollback().await?;
                return Ok(ImportControlOutcome::Rejected(reason::NOT_PARTICIPANT));
            };
            if participant.source_user != source_user {
                tx.rollback().await?;
                return Ok(ImportControlOutcome::Rejected(reason::CONSENT_MISMATCH));
            }
            sqlx::query(
                "UPDATE import_session_participants SET consent = $4 \
                 WHERE community_id = $1 AND proposal_id = $2 AND pubkey = $3",
            )
            .bind(community.as_uuid())
            .bind(proposal_id)
            .bind(&author)
            .bind(decision.as_str())
            .execute(tx.conn())
            .await?;
            participant.consent = Some(decision);
        }

        let Some(stored_event) = insert_control_event(&mut tx, event, channel_id).await? else {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Duplicate);
        };
        refresh_status(&mut tx, proposal_id, &mut session, &participants, now).await?;
        let snapshot = publish_snapshot(self, &mut tx, proposal_id, keys).await?;
        tx.commit().await?;
        Ok(ImportControlOutcome::Applied {
            stored_event: Box::new(stored_event),
            snapshot: Box::new(snapshot),
        })
    }

    /// Apply a kind:42112 withdraw from the proposer or a participant.
    pub async fn withdraw_import_session(
        &self,
        community: CommunityId,
        event: &Event,
        proposal_id: &[u8],
        channel_id: Uuid,
        keys: &Keys,
    ) -> Result<ImportControlOutcome> {
        let mut tx = crate::begin_community_event_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let author = event.pubkey.to_bytes().to_vec();
        let Some(mut session) = load_session(&mut tx, proposal_id, true).await? else {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::PROPOSAL_NOT_FOUND));
        };
        if session.channel_id != channel_id {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::CHANNEL_MISMATCH));
        }
        // Checked before the lazy status refresh so a resend after expiry is
        // still answered as a duplicate.
        if event_is_stored(tx.conn(), community, event).await? {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Duplicate);
        }
        let participants = load_participants(&mut tx, proposal_id).await?;
        if session.proposer != author && !participants.iter().any(|p| p.pubkey == author) {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::NOT_PARTICIPANT));
        }
        let now = Utc::now().timestamp();
        if refresh_status(&mut tx, proposal_id, &mut session, &participants, now).await? {
            let snapshot = publish_snapshot(self, &mut tx, proposal_id, keys).await?;
            tx.commit().await?;
            return Ok(ImportControlOutcome::RejectedAfterTransition {
                reason: reason::SESSION_NOT_ACTIVE,
                snapshot: Box::new(snapshot),
            });
        }
        if session.status.is_terminal() {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::SESSION_NOT_ACTIVE));
        }
        let Some(stored_event) = insert_control_event(&mut tx, event, channel_id).await? else {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Duplicate);
        };
        store_status(&mut tx, proposal_id, ImportSessionStatus::Withdrawn).await?;
        let snapshot = publish_snapshot(self, &mut tx, proposal_id, keys).await?;
        tx.commit().await?;
        Ok(ImportControlOutcome::Applied {
            stored_event: Box::new(stored_event),
            snapshot: Box::new(snapshot),
        })
    }

    /// Apply a kind:42113 complete from an accepting participant. The session
    /// becomes `completed` once every accepting participant has completed.
    pub async fn complete_import_participant(
        &self,
        community: CommunityId,
        event: &Event,
        proposal_id: &[u8],
        channel_id: Uuid,
        keys: &Keys,
    ) -> Result<ImportControlOutcome> {
        let mut tx = crate::begin_community_event_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let author = event.pubkey.to_bytes().to_vec();
        let Some(mut session) = load_session(&mut tx, proposal_id, true).await? else {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::PROPOSAL_NOT_FOUND));
        };
        if session.channel_id != channel_id {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::CHANNEL_MISMATCH));
        }
        // Checked before the lazy status refresh so a resend after expiry is
        // still answered as a duplicate.
        if event_is_stored(tx.conn(), community, event).await? {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Duplicate);
        }
        let mut participants = load_participants(&mut tx, proposal_id).await?;
        let now = Utc::now().timestamp();
        if refresh_status(&mut tx, proposal_id, &mut session, &participants, now).await? {
            let snapshot = publish_snapshot(self, &mut tx, proposal_id, keys).await?;
            tx.commit().await?;
            return Ok(ImportControlOutcome::RejectedAfterTransition {
                reason: reason::SESSION_NOT_ACTIVE,
                snapshot: Box::new(snapshot),
            });
        }
        let Some(participant) = participants.iter_mut().find(|p| p.pubkey == author) else {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::NOT_PARTICIPANT));
        };
        if session.status != ImportSessionStatus::Active || participant.completed {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::SESSION_NOT_ACTIVE));
        }
        if participant.consent != Some(ImportDecision::Accept) {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Rejected(reason::NOT_CONSENTING));
        }
        let Some(stored_event) = insert_control_event(&mut tx, event, channel_id).await? else {
            tx.rollback().await?;
            return Ok(ImportControlOutcome::Duplicate);
        };
        sqlx::query(
            "UPDATE import_session_participants SET completed_at = now() \
             WHERE community_id = $1 AND proposal_id = $2 AND pubkey = $3",
        )
        .bind(community.as_uuid())
        .bind(proposal_id)
        .bind(&author)
        .execute(tx.conn())
        .await?;
        participant.completed = true;
        refresh_status(&mut tx, proposal_id, &mut session, &participants, now).await?;
        let snapshot = publish_snapshot(self, &mut tx, proposal_id, keys).await?;
        tx.commit().await?;
        Ok(ImportControlOutcome::Applied {
            stored_event: Box::new(stored_event),
            snapshot: Box::new(snapshot),
        })
    }

    /// Read a session for the relay's pre-insert rule check. Returns `None`
    /// when no session exists for `proposal_id`. Does not write; the
    /// authoritative check runs again in [`Db::insert_import_event`].
    pub async fn import_preflight(
        &self,
        community: CommunityId,
        proposal_id: &[u8],
        event: &Event,
    ) -> Result<Option<ImportPreflight>> {
        let author = event.pubkey.to_bytes();
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let Some(session) = read_session(&mut conn, community, proposal_id, false).await? else {
            return Ok(None);
        };
        let participants = read_participants(&mut conn, community, proposal_id).await?;
        let already_stored = event_is_stored(&mut conn, community, event).await?;
        drop(conn);
        let status = import::derive_status(&facts(&session, &participants), Utc::now().timestamp());
        Ok(Some(ImportPreflight {
            channel_id: session.channel_id,
            range: session.range,
            status,
            author: participants
                .iter()
                .find(|p| p.pubkey == author)
                .map(author_facts),
            already_stored,
            source_type: session.source_type,
        }))
    }

    /// Insert one imported kind:9 or kind:7 event under its session.
    ///
    /// Re-checks rules 2 to 7 of NIP-CI under the session row lock, inserts
    /// the event (and the reaction row for kind:7), and increments the
    /// author's counter in the same transaction. The transaction never
    /// enqueues push, and the mention writers skip import-tagged events.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_import_event(
        &self,
        community: CommunityId,
        event: &Event,
        tags: &ImportTags,
        channel_id: Uuid,
        thread_meta: Option<ThreadMetadataParams<'_>>,
        reaction: Option<ImportReaction<'_>>,
        keys: &Keys,
    ) -> Result<ImportInsertOutcome> {
        let proposal_id = tags.proposal_bytes.to_vec();
        let kind = u32::from(event.kind.as_u16());
        let author = event.pubkey.to_bytes().to_vec();
        let mut tx = crate::begin_community_event_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let reject = |reason| ImportInsertOutcome::Rejected {
            reason,
            snapshot: None,
        };
        let Some(mut session) = load_session(&mut tx, &proposal_id, true).await? else {
            tx.rollback().await?;
            return Ok(reject(reason::PROPOSAL_NOT_FOUND));
        };
        if session.channel_id != channel_id {
            tx.rollback().await?;
            return Ok(reject(reason::CHANNEL_MISMATCH));
        }
        if tags.source_type != session.source_type {
            tx.rollback().await?;
            return Ok(reject(reason::SOURCE_TYPE_MISMATCH));
        }
        // A resend of a stored event is a duplicate even after the author's
        // count is used up or the session has ended, so publishers can retry.
        if event_is_stored(tx.conn(), community, event).await? {
            tx.rollback().await?;
            return Ok(ImportInsertOutcome::Duplicate);
        }
        let participants = load_participants(&mut tx, &proposal_id).await?;
        let now = Utc::now().timestamp();
        if refresh_status(&mut tx, &proposal_id, &mut session, &participants, now).await? {
            let snapshot = publish_snapshot(self, &mut tx, &proposal_id, keys).await?;
            tx.commit().await?;
            return Ok(ImportInsertOutcome::Rejected {
                reason: reason::SESSION_NOT_ACTIVE,
                snapshot: Some(Box::new(snapshot)),
            });
        }
        let author_row = participants.iter().find(|p| p.pubkey == author);
        if let Err(reason) = import::evaluate_import_rules(
            session.status,
            session.range,
            author_row.map(author_facts).as_ref(),
            kind,
            event.created_at.as_secs() as i64,
            now,
        ) {
            tx.rollback().await?;
            return Ok(reject(reason));
        }
        let Some(author_row) = author_row else {
            tx.rollback().await?;
            return Ok(reject(reason::NOT_CONSENTING));
        };
        // An account linked to an owner after the proposal is an agent now.
        if any_agent(&mut tx, std::slice::from_ref(&author)).await? {
            tx.rollback().await?;
            return Ok(reject(reason::PARTICIPANTS_INCLUDE_AGENTS));
        }
        // Membership can change after activation. Membership writes do not
        // take the session lock, so this is checked per event rather than
        // once at activation.
        if let Some(reason) = destination_drift(&mut tx, &session, &participants).await? {
            tx.rollback().await?;
            return Ok(reject(reason));
        }

        if kind == KIND_REACTION {
            let Some(reaction) = reaction else {
                return Err(invalid("imported reaction without target"));
            };
            let target = sqlx::query(
                "SELECT created_at, channel_id, tags FROM events \
                 WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(community.as_uuid())
            .bind(reaction.target_event_id)
            .fetch_optional(tx.conn())
            .await?;
            let Some(target) = target else {
                tx.rollback().await?;
                return Ok(ImportInsertOutcome::TargetMissing);
            };
            let target_tags: serde_json::Value = target.try_get("tags")?;
            let target_channel: Option<Uuid> = target.try_get("channel_id")?;
            if target_channel != Some(channel_id)
                || !carries_import_tag(&target_tags, &tags.proposal_id)
            {
                tx.rollback().await?;
                return Ok(reject(reason::REACTION_TARGET_NOT_IMPORTED));
            }
            let target_created_at: DateTime<Utc> = target.try_get("created_at")?;
            let added = crate::reaction::add_reaction_tx(
                &mut tx,
                reaction.target_event_id,
                target_created_at,
                reaction.actor_pubkey,
                reaction.emoji,
                Some(event.id.as_bytes()),
            )
            .await?;
            // The event itself is new (checked above), so an existing row is
            // another reaction with the same actor, target and emoji.
            if !added {
                tx.rollback().await?;
                return Ok(reject(reason::REACTION_CONFLICT));
            }
        }

        let (stored_event, inserted) =
            insert_event_with_thread_metadata_tx(&mut tx, event, Some(channel_id), thread_meta)
                .await?;
        if !inserted {
            tx.rollback().await?;
            return Ok(ImportInsertOutcome::Duplicate);
        }
        let mut published = author_row.published;
        let column = if kind == KIND_REACTION {
            published.reactions = published.reactions.saturating_add(1);
            "published_reactions"
        } else {
            published.messages = published.messages.saturating_add(1);
            "published_messages"
        };
        let sql = format!(
            "UPDATE import_session_participants SET {column} = {column} + 1 \
             WHERE community_id = $1 AND proposal_id = $2 AND pubkey = $3"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(community.as_uuid())
            .bind(&proposal_id)
            .bind(&author)
            .execute(tx.conn())
            .await?;
        // A boundary republish is skipped while the head snapshot is not yet
        // behind the relay clock, so strictly increasing `created_at` values
        // cannot run ahead of real time when many participants import at
        // once. Control events still republish every time.
        let republish = import::crosses_republish_boundary(published)
            && snapshot_head_at(&mut tx, &hex::encode(&proposal_id), keys)
                .await?
                .is_none_or(|at| at < relay_now());
        let snapshot = if republish {
            Some(Box::new(
                publish_snapshot(self, &mut tx, &proposal_id, keys).await?,
            ))
        } else {
            None
        };
        // Imported history must not extend an ephemeral channel's lifetime.
        tx.skip_ttl_refresh();
        tx.commit().await?;
        Ok(ImportInsertOutcome::Inserted {
            stored_event: Box::new(stored_event),
            snapshot,
        })
    }
}

/// Whether the event's `p` tags are exactly one, naming `pubkey`.
fn names_only(event: &Event, pubkey: &[u8]) -> bool {
    let mut values = event.tags.iter().filter_map(|tag| {
        let slice = tag.as_slice();
        (slice.first().map(String::as_str) == Some("p")).then(|| slice.get(1))
    });
    let expected = hex::encode(pubkey);
    matches!(
        (values.next(), values.next()),
        (Some(Some(value)), None) if *value == expected
    )
}

/// Whether `event` is already stored (soft-deleted rows included).
async fn event_is_stored(
    conn: &mut PgConnection,
    community: CommunityId,
    event: &Event,
) -> Result<bool> {
    let created_at = DateTime::<Utc>::from_timestamp(event.created_at.as_secs() as i64, 0)
        .ok_or_else(|| invalid("event created_at"))?;
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM events \
         WHERE community_id = $1 AND created_at = $2 AND id = $3)",
    )
    .bind(community.as_uuid())
    .bind(created_at)
    .bind(event.id.as_bytes().as_slice())
    .fetch_one(conn)
    .await?)
}

/// Whether stored event tags contain `["import", proposal_id]`.
fn carries_import_tag(tags: &serde_json::Value, proposal_id: &str) -> bool {
    tags.as_array().is_some_and(|tags| {
        tags.iter().any(|tag| {
            tag.as_array().is_some_and(|tag| {
                tag.first().and_then(|v| v.as_str()) == Some(import::IMPORT_TAG)
                    && tag.get(1).and_then(|v| v.as_str()) == Some(proposal_id)
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_tag_detection_on_stored_tags() {
        let id = "a".repeat(64);
        let tags = serde_json::json!([["h", "x"], ["import", id], ["source", "t", "1"]]);
        assert!(carries_import_tag(&tags, &id));
        assert!(!carries_import_tag(&tags, &"b".repeat(64)));
        assert!(!carries_import_tag(&serde_json::json!([["h", "x"]]), &id));
        assert!(!carries_import_tag(&serde_json::json!("import"), &id));
    }

    #[test]
    fn snapshot_lists_admin_requirement_and_counts() {
        let session = SessionRow {
            channel_id: Uuid::nil(),
            proposer: vec![1; 32],
            manifest: "m".into(),
            source_type: "test".into(),
            is_dm: false,
            range: ImportRange { from: 0, to: 1 },
            expires_at: 99,
            requires_admin: true,
            admin_consent: Some(ImportDecision::Accept),
            status: ImportSessionStatus::Active,
        };
        let participants = vec![
            ParticipantRow {
                pubkey: vec![1; 32],
                source_user: "s".into(),
                declared: ImportCounts {
                    messages: 2,
                    reactions: 0,
                },
                consent: Some(ImportDecision::Accept),
                published: ImportCounts {
                    messages: 2,
                    reactions: 0,
                },
                completed: true,
            },
            ParticipantRow {
                pubkey: vec![2; 32],
                source_user: "s".into(),
                declared: ImportCounts::default(),
                consent: None,
                published: ImportCounts::default(),
                completed: false,
            },
        ];
        let content = snapshot_content(&session, &participants, ImportSessionStatus::Active);
        let first = hex::encode([1u8; 32]);
        let second = hex::encode([2u8; 32]);
        assert_eq!(
            content.required,
            vec![first.clone(), second.clone(), "admin".into()]
        );
        assert_eq!(content.consents.get(&first), Some(&ImportDecision::Accept));
        assert_eq!(content.consents.get(&second), None);
        assert_eq!(content.consents.get("admin"), Some(&ImportDecision::Accept));
        assert_eq!(content.published[&first].messages, 2);
        assert_eq!(content.completed, vec![first]);
        assert_eq!(content.expires_at, 99);
    }
}

#[cfg(test)]
mod postgres_tests;
