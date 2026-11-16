//! Atomic community Mention Group and management-policy acceptance.
//!
//! Unused IDs become live at version one; live IDs advance by exact CAS;
//! deleted IDs stay retired. Accepted command receipts survive every later
//! revision, so an uncertain retry never becomes another logical write.

use buzz_core::mention_group::{
    parse_command, parse_policy_command, Command, Manage, Operation, PolicyCommand,
};
use buzz_core::{CommunityId, StoredEvent};
use chrono::{DateTime, Utc};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

use crate::{AdmittedTx, Db, DbError, Result};

/// Result of a group or management-policy command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupOutcome {
    /// Row, receipt and signed state were committed together.
    Applied,
    /// This exact signed command was already committed; no new state is emitted.
    Duplicate,
    /// The expected version does not match a live group or current policy.
    Conflict,
    /// Another live group already owns this name in the community.
    NameTaken,
    /// Current community authority does not permit this command.
    Forbidden,
}

/// Bounded accounting of default-channel joins, retained in the command receipt.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinSummary {
    /// Membership rows newly inserted or reactivated.
    pub joined: usize,
    /// Number of skipped target/channel pairs, keyed by stable public reason.
    pub skipped: BTreeMap<String, usize>,
}

impl JoinSummary {
    pub(crate) fn skip(&mut self, reason: &str, count: usize) {
        if count > 0 {
            *self.skipped.entry(reason.into()).or_default() += count;
        }
    }
}

/// Committed command result; only a fresh acceptance returns a fanout snapshot.
#[derive(Debug)]
pub struct GroupWrite {
    /// Durable outcome.
    pub outcome: GroupOutcome,
    /// Already committed relay-signed state, safe to fan out after returning.
    pub snapshot: Option<StoredEvent>,
    /// Channels whose real membership rows changed in the committed transaction.
    pub joined_channels: Vec<Uuid>,
    /// Newly inserted/reactivated target keys by channel, for postcommit notices.
    pub joined_members: Vec<(Uuid, Vec<u8>)>,
    /// Original join accounting, also returned for a durable identical retry.
    pub joins: JoinSummary,
}

impl GroupWrite {
    fn without_snapshot(outcome: GroupOutcome) -> Self {
        Self {
            outcome,
            snapshot: None,
            joined_channels: Vec::new(),
            joined_members: Vec::new(),
            joins: JoinSummary::default(),
        }
    }
}

impl Db {
    /// Look up durable acceptance before transport freshness checks.
    ///
    /// This read grants no permission to execute an unrecognized command.
    pub async fn mention_group_command_accepted(
        &self,
        community: CommunityId,
        event_id: &[u8],
    ) -> Result<bool> {
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        receipt_exists(&mut conn, community, event_id).await
    }

    /// Compare and atomically commit a normalized group command and signed head.
    ///
    /// `command` must be parsed from `event` by the shared protocol validator.
    /// Every non-applied outcome leaves both canonical state and receipts intact.
    pub async fn accept_mention_group(
        &self,
        community: CommunityId,
        event: &Event,
        command: &Command,
        relay_keys: &Keys,
        max_message_length: usize,
        membership_enforced: bool,
    ) -> Result<GroupWrite> {
        let parsed = parse_command(event).map_err(DbError::InvalidData)?;
        if &parsed != command {
            return Err(DbError::InvalidData(
                "mention group command differs from signed event".into(),
            ));
        }
        let mut tx = self.begin_event_write_transaction(community).await?;
        lock_community(&mut tx).await?;
        if let Some(joins) = receipt_summary(tx.conn(), community, event.id.as_bytes()).await? {
            let mut result = GroupWrite::without_snapshot(GroupOutcome::Duplicate);
            result.joins = joins;
            return Ok(result);
        }
        if !authorize_command(&mut tx, event, false, membership_enforced).await? {
            return Ok(GroupWrite::without_snapshot(GroupOutcome::Forbidden));
        }
        let head = sqlx::query(
            "SELECT name, members, version, deleted_at FROM mention_groups \
             WHERE community_id=$1 AND id=$2 FOR UPDATE",
        )
        .bind(community.as_uuid())
        .bind(command.id)
        .fetch_optional(tx.conn())
        .await?;
        let current = head
            .as_ref()
            .map(|r| r.try_get::<i64, _>("version"))
            .transpose()?;
        let deleted = head
            .as_ref()
            .map(|r| r.try_get::<Option<DateTime<Utc>>, _>("deleted_at"))
            .transpose()?
            .flatten()
            .is_some();
        if deleted
            || current.unwrap_or(0) != command.prev
            || (head.is_none() && matches!(command.operation, Operation::Delete))
        {
            return Ok(GroupWrite::without_snapshot(GroupOutcome::Conflict));
        }
        let version = current
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| DbError::InvalidData("mention group version exhausted".into()))?;
        let (name, members, channels, operation, deleting) = match &command.operation {
            Operation::Put {
                name,
                members,
                channels,
            } => (
                name.clone(),
                members.clone(),
                channels.clone(),
                "put",
                false,
            ),
            Operation::Delete => (
                head.as_ref()
                    .ok_or_else(|| DbError::InvalidData("missing group".into()))?
                    .try_get("name")?,
                Vec::new(),
                Vec::new(),
                "delete",
                true,
            ),
        };
        let mut tags = base_tags(&command.id.to_string(), version, event)?;
        if deleting {
            tags.push(tag(["deleted", "true"])?);
        } else {
            tags.push(tag(["name", name.as_str()])?);
            for member in &members {
                tags.push(tag(["p", member.as_str()])?);
            }
        }
        for channel in &channels {
            tags.push(tag(["channel", &channel.to_string()])?);
        }
        let old_members: Vec<String> = head
            .as_ref()
            .map(|row| row.try_get("members"))
            .transpose()?
            .unwrap_or_default();
        let old_members: HashSet<&str> = old_members.iter().map(String::as_str).collect();
        let added: Vec<Vec<u8>> = members
            .iter()
            .filter(|key| !old_members.contains(key.as_str()))
            .map(|key| {
                hex::decode(key).map_err(|_| DbError::InvalidData("invalid member key".into()))
            })
            .collect::<Result<_>>()?;
        // Build and bound the final signed projection before changing any row.
        let state = build_snapshot(
            &mut tx,
            relay_keys,
            &command.id.to_string(),
            tags,
            max_message_length,
        )
        .await?;
        let (joined_channels, joined_members, joins) =
            crate::channel_members::put_group_default_members_in_transaction(
                &mut tx,
                &channels,
                &added,
                &event.pubkey.to_bytes(),
                membership_enforced,
            )
            .await?;
        let result = sqlx::query(
            "INSERT INTO mention_groups (community_id,id,name,members,channels,version,deleted_at,state_event_id) \
             VALUES ($1,$2,$3,$4,$5,$6,CASE WHEN $7 THEN clock_timestamp() END,$8) \
             ON CONFLICT (community_id,id) DO UPDATE SET name=EXCLUDED.name,members=EXCLUDED.members,\
             channels=EXCLUDED.channels,version=EXCLUDED.version,deleted_at=EXCLUDED.deleted_at,state_event_id=EXCLUDED.state_event_id,\
             updated_at=clock_timestamp()",
        ).bind(community.as_uuid()).bind(command.id).bind(&name).bind(&members).bind(&channels).bind(version)
            .bind(deleting).bind(state.id.as_bytes().as_slice()).execute(tx.conn()).await;
        if let Err(error) = result {
            if matches!(&error, sqlx::Error::Database(db)
                if db.code().as_deref() == Some("23505") && db.constraint() == Some("idx_mention_groups_live_name"))
            {
                // PostgreSQL has aborted this transaction; rollback before reporting a collision.
                tx.rollback().await?;
                return Ok(GroupWrite::without_snapshot(GroupOutcome::NameTaken));
            }
            return Err(error.into());
        }
        let snapshot = persist_snapshot(&mut tx, state).await?;
        record_receipt(
            &mut tx,
            event,
            Some(command.id),
            operation,
            version,
            &snapshot,
            &joins,
        )
        .await?;
        tx.commit().await?;
        Ok(GroupWrite {
            outcome: GroupOutcome::Applied,
            snapshot: Some(snapshot),
            joined_channels,
            joined_members,
            joins,
        })
    }

    /// Compare and atomically commit the community's group-management policy.
    ///
    /// An untouched community has `everyone`, version zero and no published head;
    /// the first policy command uses `prev=0` and publishes settings version one.
    pub async fn accept_mention_group_policy(
        &self,
        community: CommunityId,
        event: &Event,
        command: &PolicyCommand,
        relay_keys: &Keys,
        max_message_length: usize,
        membership_enforced: bool,
    ) -> Result<GroupWrite> {
        let parsed = parse_policy_command(event).map_err(DbError::InvalidData)?;
        if &parsed != command {
            return Err(DbError::InvalidData(
                "mention group policy differs from signed event".into(),
            ));
        }
        let mut tx = self.begin_event_write_transaction(community).await?;
        lock_community(&mut tx).await?;
        if let Some(joins) = receipt_summary(tx.conn(), community, event.id.as_bytes()).await? {
            let mut result = GroupWrite::without_snapshot(GroupOutcome::Duplicate);
            result.joins = joins;
            return Ok(result);
        }
        if !authorize_command(&mut tx, event, true, membership_enforced).await? {
            return Ok(GroupWrite::without_snapshot(GroupOutcome::Forbidden));
        }
        let current: i64 =
            sqlx::query_scalar("SELECT mention_groups_version FROM communities WHERE id=$1")
                .bind(community.as_uuid())
                .fetch_one(tx.conn())
                .await?;
        if current != command.prev {
            return Ok(GroupWrite::without_snapshot(GroupOutcome::Conflict));
        }
        let version = current
            .checked_add(1)
            .ok_or_else(|| DbError::InvalidData("mention group policy version exhausted".into()))?;
        let manage = match command.manage {
            Manage::Admins => "admins",
            Manage::Everyone => "everyone",
        };
        let mut tags = base_tags("settings", version, event)?;
        tags.push(tag(["manage", manage])?);
        let state =
            build_snapshot(&mut tx, relay_keys, "settings", tags, max_message_length).await?;
        sqlx::query("UPDATE communities SET mention_groups_manage=$2,mention_groups_version=$3,mention_groups_state_event_id=$4 WHERE id=$1")
            .bind(community.as_uuid()).bind(manage).bind(version).bind(state.id.as_bytes().as_slice())
            .execute(tx.conn()).await?;
        let snapshot = persist_snapshot(&mut tx, state).await?;
        record_receipt(
            &mut tx,
            event,
            None,
            "policy",
            version,
            &snapshot,
            &JoinSummary::default(),
        )
        .await?;
        tx.commit().await?;
        Ok(GroupWrite {
            outcome: GroupOutcome::Applied,
            snapshot: Some(snapshot),
            joined_channels: Vec::new(),
            joined_members: Vec::new(),
            joins: JoinSummary::default(),
        })
    }
}

async fn lock_community(tx: &mut AdmittedTx) -> Result<()> {
    sqlx::query("SET LOCAL statement_timeout='5s'")
        .execute(tx.conn())
        .await?;
    // The same lock owner transitions use: policy changes and group writes see
    // one order, and compatible FK key-share locks cannot create a lock cycle.
    sqlx::query("SELECT id FROM communities WHERE id=$1 FOR NO KEY UPDATE")
        .bind(tx.community().as_uuid())
        .fetch_one(tx.conn())
        .await?;
    Ok(())
}

// Every mutable membership decision is held through commit. Agent ownership
// itself is immutable; a stored owner only supplies admission under Everyone,
// never an inherited administrator role.
async fn authorize_command(
    tx: &mut AdmittedTx,
    event: &Event,
    policy_change: bool,
    membership_enforced: bool,
) -> Result<bool> {
    let community = tx.community();
    let actor = event.pubkey.to_hex();
    let own_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM relay_members WHERE community_id=$1 AND pubkey=$2 FOR SHARE",
    )
    .bind(community.as_uuid())
    .bind(&actor)
    .fetch_optional(tx.conn())
    .await?;
    let steward = matches!(own_role.as_deref(), Some("owner" | "admin"));
    if steward {
        return Ok(true);
    }
    if policy_change {
        let stewards = sqlx::query(
            "SELECT pubkey FROM relay_members WHERE community_id=$1 AND role IN ('owner','admin') ORDER BY pubkey FOR SHARE",
        ).bind(community.as_uuid()).fetch_all(tx.conn()).await?;
        if !stewards.is_empty() {
            return Ok(false);
        }
    } else {
        let manage: String =
            sqlx::query_scalar("SELECT mention_groups_manage FROM communities WHERE id=$1")
                .bind(community.as_uuid())
                .fetch_one(tx.conn())
                .await?;
        match manage.as_str() {
            "admins" => return Ok(false),
            "everyone" => {}
            _ => {
                return Err(DbError::InvalidData(
                    "unknown mention group management policy".into(),
                ))
            }
        }
    }
    if !membership_enforced || own_role.is_some() {
        return Ok(true);
    }
    let owner: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT agent_owner_pubkey FROM users WHERE community_id=$1 AND pubkey=$2",
    )
    .bind(community.as_uuid())
    .bind(event.pubkey.to_bytes().as_slice())
    .fetch_optional(tx.conn())
    .await?
    .flatten();
    let Some(owner) = owner else {
        return Ok(false);
    };
    let owner_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM relay_members WHERE community_id=$1 AND pubkey=$2 FOR SHARE",
    )
    .bind(community.as_uuid())
    .bind(hex::encode(owner))
    .fetch_optional(tx.conn())
    .await?;
    Ok(owner_role.is_some())
}

async fn receipt_exists(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
    id: &[u8],
) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM mention_group_commands WHERE community_id=$1 AND event_id=$2)",
    )
    .bind(community.as_uuid())
    .bind(id)
    .fetch_one(conn)
    .await?)
}

async fn receipt_summary(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
    id: &[u8],
) -> Result<Option<JoinSummary>> {
    let summary: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT join_summary FROM mention_group_commands WHERE community_id=$1 AND event_id=$2",
    )
    .bind(community.as_uuid())
    .bind(id)
    .fetch_optional(conn)
    .await?;
    summary
        .map(serde_json::from_value)
        .transpose()
        .map_err(Into::into)
}

fn tag<const N: usize>(values: [&str; N]) -> Result<Tag> {
    Tag::parse(values).map_err(|e| DbError::InvalidData(format!("invalid mention group tag: {e}")))
}

fn base_tags(coordinate: &str, version: i64, command: &Event) -> Result<Vec<Tag>> {
    Ok(vec![
        tag(["-"])?,
        tag(["d", coordinate])?,
        tag(["version", &version.to_string()])?,
        tag(["command", &command.id.to_hex()])?,
    ])
}

async fn build_snapshot(
    tx: &mut AdmittedTx,
    keys: &Keys,
    coordinate: &str,
    tags: Vec<Tag>,
    max_message_length: usize,
) -> Result<Event> {
    let community = tx.community();
    let kind = buzz_core::kind::KIND_MENTION_GROUP_STATE as i32;
    let pubkey = keys.public_key().to_bytes();
    let key = crate::replaceable::event_replacement_lock_key(
        community,
        kind,
        &pubkey,
        Some(coordinate.as_bytes()),
    );
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(tx.conn())
        .await?;
    // Retired heads count: same-second commands must still replace correctly.
    let latest: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT max(created_at) FROM events WHERE community_id=$1 AND kind=$2 AND pubkey=$3 AND d_tag=$4",
    ).bind(community.as_uuid()).bind(kind).bind(pubkey.as_slice()).bind(coordinate)
        .fetch_one(tx.conn()).await?;
    let now = i64::try_from(Timestamp::now().as_secs())
        .map_err(|_| DbError::InvalidTimestamp(i64::MAX))?;
    let seconds = latest
        .map(|value| {
            value
                .timestamp()
                .checked_add(1)
                .ok_or(DbError::InvalidTimestamp(i64::MAX))
        })
        .transpose()?
        .unwrap_or(now)
        .max(now);
    let event = EventBuilder::new(Kind::Custom(kind as u16), "")
        .tags(tags)
        .allow_self_tagging()
        .custom_created_at(Timestamp::from(seconds as u64))
        .sign_with_keys(keys)
        .map_err(|e| DbError::InvalidData(format!("mention group signing failed: {e}")))?;
    if serde_json::to_vec(&serde_json::json!(["EVENT", &event]))?.len() > max_message_length {
        return Err(DbError::InvalidData(
            "mention group state exceeds maximum message length".into(),
        ));
    }
    Ok(event)
}

async fn persist_snapshot(tx: &mut AdmittedTx, event: Event) -> Result<StoredEvent> {
    let community = tx.community();
    let kind = buzz_core::kind::KIND_MENTION_GROUP_STATE as i32;
    let pubkey = event.pubkey.to_bytes();
    let coordinate = crate::event::extract_d_tag(&event)
        .ok_or_else(|| DbError::InvalidData("mention group state missing coordinate".into()))?;
    let seconds = i64::try_from(event.created_at.as_secs())
        .map_err(|_| DbError::InvalidTimestamp(i64::MAX))?;
    let created_at =
        DateTime::from_timestamp(seconds, 0).ok_or(DbError::InvalidTimestamp(seconds))?;
    let received_at = Utc::now();
    sqlx::query(superseded_with_mentions_sql!(
        "UPDATE events SET deleted_at=NOW() WHERE community_id=$1 AND kind=$2 AND pubkey=$3 AND d_tag=$4 AND deleted_at IS NULL"
    )).bind(community.as_uuid()).bind(kind).bind(pubkey.as_slice()).bind(&coordinate).execute(tx.conn()).await?;
    sqlx::query(
        "INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig,received_at,channel_id,d_tag) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,NULL,$10)",
    ).bind(community.as_uuid()).bind(event.id.as_bytes().as_slice()).bind(pubkey.as_slice())
        .bind(created_at).bind(kind).bind(serde_json::to_value(&event.tags)?).bind(&event.content)
        .bind(event.sig.serialize().as_slice()).bind(received_at).bind(&coordinate).execute(tx.conn()).await?;
    crate::insert_mentions_in_transaction(tx, &event, None).await?;
    crate::store::event_follow_up::after_admitted_insert(
        tx,
        event.id.as_bytes(),
        created_at,
        kind,
        None,
    )
    .await?;
    Ok(StoredEvent::with_received_at(
        event,
        received_at,
        None,
        true,
    ))
}

async fn record_receipt(
    tx: &mut AdmittedTx,
    event: &Event,
    group: Option<Uuid>,
    operation: &str,
    version: i64,
    snapshot: &StoredEvent,
    joins: &JoinSummary,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO mention_group_commands (community_id,event_id,signer,group_id,operation,version,state_event_id,command,join_summary) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    ).bind(tx.community().as_uuid()).bind(event.id.as_bytes().as_slice()).bind(event.pubkey.to_hex())
        .bind(group).bind(operation).bind(version).bind(snapshot.event.id.as_bytes().as_slice())
        .bind(serde_json::to_value(event)?).bind(serde_json::to_value(joins)?).execute(tx.conn()).await?;
    Ok(())
}
