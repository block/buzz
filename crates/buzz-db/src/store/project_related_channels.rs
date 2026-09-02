//! Collaborative Project related-channel command persistence.

use buzz_core::kind::{KIND_PROJECT, KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT};
use buzz_core::{CommunityId, StoredEvent};
use chrono::Utc;
use nostr::Event;
use serde_json::Value;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::replaceable::{ParameterizedReplacePrecondition, ParameterizedReplaceStatus};
use crate::Db;

/// Requested related-channel state transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectRelatedChannelChange {
    /// Link the channel.
    Add,
    /// Unlink the channel.
    Remove,
}

/// Data the relay must sign while the Project mutation transaction remains open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRelatedChannelsSnapshotPlan {
    /// Canonical `30621:<owner>:<d>` Project coordinate.
    pub project_coordinate: String,
    /// Deterministic snapshot `d` tag.
    pub snapshot_d: String,
    /// Exact live kind:30621 event this projection describes.
    pub project_event_id: String,
    /// Sorted, bounded effective present relations.
    pub entries: Vec<Uuid>,
    /// Whether effective membership exceeded the bounded snapshot projection.
    pub truncated: bool,
    /// Timestamp that will dominate the previous relay snapshot head.
    pub created_at: u64,
}

/// Inputs for one related-channel command application.
pub struct ApplyProjectRelatedChannelCommand<'a> {
    /// Signed kind:47010 command event.
    pub event: &'a Event,
    /// Stable relay identity used to sign the derived snapshot.
    pub relay_pubkey: &'a [u8],
    /// Project owner public key from the command coordinate.
    pub project_owner: &'a [u8],
    /// Project `d` identifier from the command coordinate.
    pub project_d: &'a str,
    /// Related channel targeted by the command.
    pub channel_id: Uuid,
    /// Desired relation transition.
    pub change: ProjectRelatedChannelChange,
}

/// Open Project mutation transaction awaiting one relay-signed snapshot.
#[derive(Debug)]
pub struct PreparedProjectRelatedChannelsMutation {
    db: Db,
    community_id: CommunityId,
    relay_pubkey: Vec<u8>,
    primary_event: StoredEvent,
    snapshot: ProjectRelatedChannelsSnapshotPlan,
    tx: Transaction<'static, Postgres>,
}

impl PreparedProjectRelatedChannelsMutation {
    /// Snapshot data to sign before finalizing this transaction.
    #[must_use]
    pub fn snapshot(&self) -> &ProjectRelatedChannelsSnapshotPlan {
        &self.snapshot
    }

    /// Replace the relay snapshot and commit both mutations atomically.
    pub async fn commit_with_snapshot(
        mut self,
        snapshot_event: &Event,
    ) -> Result<(StoredEvent, StoredEvent)> {
        validate_snapshot_event(snapshot_event, &self.relay_pubkey, &self.snapshot)?;
        let result = self
            .db
            .replace_parameterized_event_in_transaction(
                &mut self.tx,
                self.community_id,
                snapshot_event,
                &self.snapshot.snapshot_d,
                None,
                ParameterizedReplacePrecondition::Unconditional,
            )
            .await?;
        if result.status != ParameterizedReplaceStatus::Inserted {
            return Err(DbError::InvalidData(format!(
                "Project related-channel snapshot replacement was {:?}",
                result.status
            )));
        }
        self.tx.commit().await?;
        Ok((self.primary_event, result.event))
    }
}

/// Result of atomically authorizing and applying a related-channel command.
#[derive(Debug)]
pub enum ApplyProjectRelatedChannelOutcome {
    /// A new command event and override committed atomically.
    Applied(Box<PreparedProjectRelatedChannelsMutation>),
    /// This exact signed event was already accepted.
    Replay,
    /// The requested effective state already held; no event was stored.
    Noop,
    /// No live Project exists at the coordinate.
    ProjectNotFound,
    /// The signer lacks Project-management authority.
    Unauthorized,
    /// The target channel cannot be linked.
    InvalidTarget(&'static str),
}

/// Result of preparing an owner-authored Project replacement.
pub enum PrepareProjectReplacementOutcome {
    /// The Project replacement is open and awaits its derived snapshot.
    Applied(Box<PreparedProjectRelatedChannelsMutation>),
    /// Normal NIP-33 ordering treated this write as duplicate or superseded.
    Unchanged,
}

struct ProjectHead {
    event_id: Vec<u8>,
    home_channel_id: Option<Uuid>,
    tags: Value,
}

fn tag_values<'a>(tags: &'a Value, name: &'a str) -> impl Iterator<Item = &'a str> {
    tags.as_array()
        .into_iter()
        .flatten()
        .filter_map(move |tag| {
            let parts = tag.as_array()?;
            if parts.first()?.as_str()? == name {
                parts.get(1)?.as_str()
            } else {
                None
            }
        })
}

async fn live_project_head(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    project_owner: &[u8],
    project_d: &str,
) -> Result<Option<ProjectHead>> {
    let row = sqlx::query(
        "SELECT id, tags FROM events \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 \
           AND deleted_at IS NULL \
         ORDER BY created_at DESC, id ASC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(KIND_PROJECT as i32)
    .bind(project_owner)
    .bind(project_d)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let event_id: Vec<u8> = row.try_get("id")?;
    let tags: Value = row.try_get("tags")?;
    let home_channel_id = tag_values(&tags, "buzz-channel")
        .next()
        .and_then(|value| Uuid::parse_str(value).ok());
    Ok(Some(ProjectHead {
        event_id,
        home_channel_id,
        tags,
    }))
}

async fn override_presence(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    project_owner: &[u8],
    project_d: &str,
    channel_id: Uuid,
) -> Result<Option<bool>> {
    sqlx::query_scalar(
        "SELECT present \
         FROM project_related_channel_overrides \
         WHERE community_id = $1 AND project_owner = $2 AND project_d = $3 AND channel_id = $4",
    )
    .bind(community_id.as_uuid())
    .bind(project_owner)
    .bind(project_d)
    .bind(channel_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(Into::into)
}

fn inherited_presence(head: &ProjectHead, channel_id: Uuid) -> bool {
    tag_values(&head.tags, "buzz-related-channel")
        .filter_map(|value| Uuid::parse_str(value).ok())
        .any(|value| value == channel_id)
}

fn validate_snapshot_event(
    event: &Event,
    relay_pubkey: &[u8],
    plan: &ProjectRelatedChannelsSnapshotPlan,
) -> Result<()> {
    if u32::from(event.kind.as_u16()) != KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT
        || event.pubkey.to_bytes().as_slice() != relay_pubkey
        || !event.content.is_empty()
        || event.created_at.as_secs() != plan.created_at
    {
        return Err(DbError::InvalidData(
            "Project related-channel snapshot envelope does not match its prepared mutation".into(),
        ));
    }
    let mut expected = Vec::with_capacity(plan.entries.len() + 3);
    expected.push(vec!["d".to_owned(), plan.snapshot_d.clone()]);
    expected.push(vec!["a".to_owned(), plan.project_coordinate.clone()]);
    expected.push(vec!["e".to_owned(), plan.project_event_id.clone()]);
    for channel_id in &plan.entries {
        expected.push(vec!["c".to_owned(), channel_id.to_string()]);
    }
    let actual: Vec<Vec<String>> = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect();
    if actual != expected {
        return Err(DbError::InvalidData(
            "Project related-channel snapshot tags do not match prepared effective state".into(),
        ));
    }
    Ok(())
}

/// Remove the relay-derived snapshot while the caller holds the Project
/// coordinate lock and owns the surrounding transaction.
pub(crate) async fn delete_snapshot_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    project_owner: &[u8],
    project_d: &str,
) -> Result<()> {
    let project_coordinate = format!("{KIND_PROJECT}:{}:{project_d}", hex::encode(project_owner));
    let snapshot_d = buzz_core::project_related_channels::project_related_channels_snapshot_d(
        &project_coordinate,
    );
    sqlx::query(
        "UPDATE events SET deleted_at = NOW() \
         WHERE community_id = $1 AND kind = $2 AND d_tag = $3 AND deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT as i32)
    .bind(snapshot_d)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn effective_snapshot_entries(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    project_owner: &[u8],
    project_d: &str,
    head: &ProjectHead,
) -> Result<(Vec<Uuid>, bool)> {
    let cap = buzz_core::project_related_channels::PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP;
    // PostgreSQL overlays all legacy tags with durable overrides before the
    // bounded sort/limit. Rust therefore never allocates a collection sized
    // by an owner-controlled legacy tag list.
    let rows = sqlx::query(
        "WITH legacy AS ( \
           SELECT DISTINCT (tag->>1)::uuid AS channel_id \
           FROM jsonb_array_elements($4::jsonb) AS tag \
           WHERE tag->>0 = 'buzz-related-channel' \
             AND tag->>1 ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' \
             AND (tag->>1)::uuid <> '00000000-0000-0000-0000-000000000000'::uuid \
             AND (tag->>1)::uuid IS DISTINCT FROM $5::uuid \
         ), effective AS ( \
           SELECT legacy.channel_id \
           FROM legacy \
           LEFT JOIN project_related_channel_overrides overrides \
             ON overrides.community_id = $1 AND overrides.project_owner = $2 \
            AND overrides.project_d = $3 AND overrides.channel_id = legacy.channel_id \
           WHERE overrides.channel_id IS NULL OR overrides.present = TRUE \
           UNION ALL \
           SELECT overrides.channel_id \
           FROM project_related_channel_overrides overrides \
           WHERE overrides.community_id = $1 AND overrides.project_owner = $2 \
             AND overrides.project_d = $3 AND overrides.present = TRUE \
             AND overrides.channel_id IS DISTINCT FROM $5::uuid \
             AND NOT EXISTS ( \
               SELECT 1 FROM legacy WHERE legacy.channel_id = overrides.channel_id \
             ) \
         ) \
         SELECT channel_id FROM effective \
         ORDER BY channel_id ASC LIMIT $6",
    )
    .bind(community_id.as_uuid())
    .bind(project_owner)
    .bind(project_d)
    .bind(&head.tags)
    .bind(head.home_channel_id)
    .bind((cap + 1) as i64)
    .fetch_all(&mut **tx)
    .await?;
    let mut entries: Vec<Uuid> = rows
        .into_iter()
        .map(|row| row.try_get("channel_id").map_err(Into::into))
        .collect::<Result<_>>()?;
    let truncated = entries.len() > cap;
    entries.truncate(cap);
    Ok((entries, truncated))
}

async fn snapshot_plan(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    project_owner: &[u8],
    project_d: &str,
    relay_pubkey: &[u8],
    head: &ProjectHead,
) -> Result<ProjectRelatedChannelsSnapshotPlan> {
    let project_coordinate = format!("{KIND_PROJECT}:{}:{project_d}", hex::encode(project_owner));
    let snapshot_d = buzz_core::project_related_channels::project_related_channels_snapshot_d(
        &project_coordinate,
    );
    let latest: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
        "SELECT created_at FROM events \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 \
         ORDER BY created_at DESC, id ASC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT as i32)
    .bind(relay_pubkey)
    .bind(&snapshot_d)
    .fetch_optional(&mut **tx)
    .await?;
    let now = Utc::now().timestamp().max(0) as u64;
    let created_at = latest
        .map(|timestamp| (timestamp.timestamp().max(0) as u64).saturating_add(1))
        .unwrap_or(now)
        .max(now);
    let (entries, truncated) =
        effective_snapshot_entries(tx, community_id, project_owner, project_d, head).await?;
    Ok(ProjectRelatedChannelsSnapshotPlan {
        project_coordinate,
        snapshot_d,
        project_event_id: hex::encode(&head.event_id),
        entries,
        truncated,
        created_at,
    })
}

async fn active_target_membership(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    channel_id: Uuid,
    actor: &[u8],
) -> Result<Option<(bool, bool)>> {
    let row = sqlx::query(
        "SELECT c.archived_at IS NULL AND c.deleted_at IS NULL AS active, \
                c.channel_type = 'dm' AS is_dm, \
                EXISTS(SELECT 1 FROM channel_members cm \
                       WHERE cm.community_id = c.community_id AND cm.channel_id = c.id \
                         AND cm.pubkey = $3 AND cm.removed_at IS NULL) AS member \
         FROM channels c WHERE c.community_id = $1 AND c.id = $2 FOR SHARE OF c",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(actor)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| {
        let active: bool = row.try_get("active")?;
        let is_dm: bool = row.try_get("is_dm")?;
        let member: bool = row.try_get("member")?;
        Ok((active && member, is_dm))
    })
    .transpose()
}

/// Authorize and atomically store one state-changing command plus its override head.
pub async fn apply_project_related_channel_command(
    db: &Db,
    community_id: CommunityId,
    command: ApplyProjectRelatedChannelCommand<'_>,
) -> Result<ApplyProjectRelatedChannelOutcome> {
    let ApplyProjectRelatedChannelCommand {
        event,
        relay_pubkey,
        project_owner,
        project_d,
        channel_id,
        change,
    } = command;
    let mut tx = db.begin_transaction().await?;
    crate::replaceable::acquire_parameterized_event_lock(
        &mut tx,
        community_id,
        KIND_PROJECT as i32,
        project_owner,
        project_d,
    )
    .await?;

    let replay: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM events WHERE community_id = $1 AND id = $2)",
    )
    .bind(community_id.as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .fetch_one(&mut *tx)
    .await?;
    if replay {
        tx.rollback().await?;
        return Ok(ApplyProjectRelatedChannelOutcome::Replay);
    }

    let Some(head) = live_project_head(&mut tx, community_id, project_owner, project_d).await?
    else {
        tx.rollback().await?;
        return Ok(ApplyProjectRelatedChannelOutcome::ProjectNotFound);
    };
    if change == ProjectRelatedChannelChange::Add && head.home_channel_id == Some(channel_id) {
        tx.rollback().await?;
        return Ok(ApplyProjectRelatedChannelOutcome::InvalidTarget(
            "Project home channel cannot also be a related channel",
        ));
    }

    let mut membership_locks = Vec::with_capacity(2);
    if let Some(home) = head.home_channel_id {
        membership_locks.push(home);
    }
    if change == ProjectRelatedChannelChange::Add {
        membership_locks.push(channel_id);
    }
    membership_locks.sort_unstable();
    membership_locks.dedup();
    for locked_channel in membership_locks {
        crate::channel_members::acquire_channel_membership_lock(
            &mut tx,
            community_id,
            locked_channel,
        )
        .await?;
    }

    let actor = event.pubkey.to_bytes();
    let owns_project_agent: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users \
         WHERE community_id = $1 AND pubkey = $2 AND agent_owner_pubkey = $3)",
    )
    .bind(community_id.as_uuid())
    .bind(project_owner)
    .bind(actor.as_slice())
    .fetch_one(&mut *tx)
    .await?;
    let home_authority = match head.home_channel_id {
        Some(home_channel_id) => {
            let home_is_active = sqlx::query_scalar(
                "SELECT archived_at IS NULL AND deleted_at IS NULL FROM channels \
                 WHERE community_id = $1 AND id = $2 FOR SHARE",
            )
            .bind(community_id.as_uuid())
            .bind(home_channel_id)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or(false);
            home_is_active
                && crate::channel_members::has_channel_management_authority_tx(
                    &mut tx,
                    community_id,
                    home_channel_id,
                    actor.as_slice(),
                )
                .await?
        }
        None => false,
    };
    if actor.as_slice() != project_owner && !owns_project_agent && !home_authority {
        tx.rollback().await?;
        return Ok(ApplyProjectRelatedChannelOutcome::Unauthorized);
    }

    let present = override_presence(&mut tx, community_id, project_owner, project_d, channel_id)
        .await?
        .unwrap_or_else(|| inherited_presence(&head, channel_id));

    // Desired-state no-ops are unconditional under the Project lock. No writer
    // can change the already-satisfied end state before this transaction exits.
    if (change == ProjectRelatedChannelChange::Add && present)
        || (change == ProjectRelatedChannelChange::Remove && !present)
    {
        tx.rollback().await?;
        return Ok(ApplyProjectRelatedChannelOutcome::Noop);
    }
    if change == ProjectRelatedChannelChange::Add {
        match active_target_membership(&mut tx, community_id, channel_id, actor.as_slice()).await? {
            Some((_, true)) => {
                tx.rollback().await?;
                return Ok(ApplyProjectRelatedChannelOutcome::InvalidTarget(
                    "DM channels cannot be related to Projects",
                ));
            }
            Some((true, false)) => {}
            Some((false, false)) => {
                tx.rollback().await?;
                return Ok(ApplyProjectRelatedChannelOutcome::InvalidTarget(
                    "target channel must be active and the actor must be a member",
                ));
            }
            None => {
                tx.rollback().await?;
                return Ok(ApplyProjectRelatedChannelOutcome::InvalidTarget(
                    "target channel not found in this community",
                ));
            }
        }
        let (entries, _) =
            effective_snapshot_entries(&mut tx, community_id, project_owner, project_d, &head)
                .await?;
        if entries.len()
            >= buzz_core::project_related_channels::PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP
        {
            tx.rollback().await?;
            return Ok(ApplyProjectRelatedChannelOutcome::InvalidTarget(
                "Project already has 64 related channels",
            ));
        }
    }
    let (stored_event, inserted) =
        crate::event::insert_event_in_transaction(&mut tx, community_id, event, None).await?;
    if !inserted {
        tx.rollback().await?;
        return Ok(ApplyProjectRelatedChannelOutcome::Replay);
    }
    sqlx::query(
        "INSERT INTO project_related_channel_overrides \
             (community_id, project_owner, project_d, channel_id, present) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (community_id, project_owner, project_d, channel_id) DO UPDATE SET \
             present = EXCLUDED.present",
    )
    .bind(community_id.as_uuid())
    .bind(project_owner)
    .bind(project_d)
    .bind(channel_id)
    .bind(change == ProjectRelatedChannelChange::Add)
    .execute(&mut *tx)
    .await?;
    let snapshot = snapshot_plan(
        &mut tx,
        community_id,
        project_owner,
        project_d,
        relay_pubkey,
        &head,
    )
    .await?;
    Ok(ApplyProjectRelatedChannelOutcome::Applied(Box::new(
        PreparedProjectRelatedChannelsMutation {
            db: db.clone(),
            community_id,
            relay_pubkey: relay_pubkey.to_vec(),
            primary_event: stored_event,
            snapshot,
            tx,
        },
    )))
}

/// Prepare an owner-authored Project replacement and its derived snapshot.
pub async fn prepare_project_replacement(
    db: &Db,
    community_id: CommunityId,
    event: &Event,
    project_d: &str,
    relay_pubkey: &[u8],
) -> Result<PrepareProjectReplacementOutcome> {
    let mut tx = db.begin_transaction().await?;
    let result = db
        .replace_parameterized_event_in_transaction(
            &mut tx,
            community_id,
            event,
            project_d,
            None,
            ParameterizedReplacePrecondition::Unconditional,
        )
        .await?;
    if result.status != ParameterizedReplaceStatus::Inserted {
        tx.rollback().await?;
        return Ok(PrepareProjectReplacementOutcome::Unchanged);
    }
    let project_owner = event.pubkey.to_bytes();
    let head = live_project_head(&mut tx, community_id, project_owner.as_slice(), project_d)
        .await?
        .ok_or_else(|| {
            DbError::InvalidData("inserted Project has no live replacement head".into())
        })?;
    if head.event_id.as_slice() != event.id.as_bytes().as_slice() {
        return Err(DbError::InvalidData(
            "inserted Project is not the live replacement head".into(),
        ));
    }
    let snapshot = snapshot_plan(
        &mut tx,
        community_id,
        project_owner.as_slice(),
        project_d,
        relay_pubkey,
        &head,
    )
    .await?;
    Ok(PrepareProjectReplacementOutcome::Applied(Box::new(
        PreparedProjectRelatedChannelsMutation {
            db: db.clone(),
            community_id,
            relay_pubkey: relay_pubkey.to_vec(),
            primary_event: result.event,
            snapshot,
            tx,
        },
    )))
}

#[cfg(test)]
mod postgres_tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::channel::{ChannelRecord, ChannelType, ChannelVisibility};
    use crate::channel_members::MemberRole;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sqlx::PgPool;

    static COMMAND_CREATED_AT: AtomicU64 = AtomicU64::new(1_700_000_000);

    async fn make_db() -> (Db, CommunityId) {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB");
        let community_uuid = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(format!(
                "project-related-channel-{}.example",
                community_uuid.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert community");
        (Db::from_pool(pool), CommunityId::from_uuid(community_uuid))
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_project_related_channel_command(
        db: &Db,
        community_id: CommunityId,
        event: &Event,
        relay_pubkey: &[u8],
        project_owner: &[u8],
        project_d: &str,
        channel_id: Uuid,
        change: ProjectRelatedChannelChange,
    ) -> Result<ApplyProjectRelatedChannelOutcome> {
        super::apply_project_related_channel_command(
            db,
            community_id,
            ApplyProjectRelatedChannelCommand {
                event,
                relay_pubkey,
                project_owner,
                project_d,
                channel_id,
                change,
            },
        )
        .await
    }

    fn command_event(
        actor: &Keys,
        project_owner: &[u8],
        project_d: &str,
        channel_id: Uuid,
        operation: &str,
    ) -> Event {
        let tags = vec![
            Tag::parse([
                "a",
                &format!("30621:{}:{project_d}", hex::encode(project_owner)),
            ])
            .expect("a tag"),
            Tag::parse(["op", operation]).expect("op tag"),
            Tag::parse(["d", &channel_id.to_string()]).expect("target channel tag"),
        ];
        EventBuilder::new(Kind::Custom(47010), "")
            .tags(tags)
            .custom_created_at(nostr::Timestamp::from(
                COMMAND_CREATED_AT.fetch_add(1, Ordering::Relaxed),
            ))
            .sign_with_keys(actor)
            .expect("sign command")
    }

    fn snapshot_event(plan: &ProjectRelatedChannelsSnapshotPlan, relay: &Keys) -> Event {
        let mut tags = vec![
            Tag::parse(["d", plan.snapshot_d.as_str()]).expect("snapshot d"),
            Tag::parse(["a", plan.project_coordinate.as_str()]).expect("Project coordinate"),
            Tag::parse(["e", plan.project_event_id.as_str()]).expect("Project head"),
        ];
        for channel_id in &plan.entries {
            tags.push(Tag::parse(["c", channel_id.to_string().as_str()]).expect("snapshot entry"));
        }
        EventBuilder::new(
            Kind::Custom(KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT as u16),
            "",
        )
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(plan.created_at))
        .sign_with_keys(relay)
        .expect("sign snapshot")
    }

    async fn commit_applied(
        outcome: ApplyProjectRelatedChannelOutcome,
        relay: &Keys,
    ) -> (StoredEvent, StoredEvent) {
        let ApplyProjectRelatedChannelOutcome::Applied(prepared) = outcome else {
            panic!("expected applied related-channel mutation");
        };
        let snapshot = snapshot_event(prepared.snapshot(), relay);
        prepared
            .commit_with_snapshot(&snapshot)
            .await
            .expect("commit mutation and snapshot")
    }

    async fn commit_project_replacement(
        db: &Db,
        community: CommunityId,
        project: &Event,
        project_d: &str,
        relay: &Keys,
    ) -> (StoredEvent, StoredEvent) {
        let relay_pubkey = relay.public_key().to_bytes();
        let outcome =
            prepare_project_replacement(db, community, project, project_d, relay_pubkey.as_slice())
                .await
                .expect("prepare Project replacement");
        let PrepareProjectReplacementOutcome::Applied(prepared) = outcome else {
            panic!("expected applied Project replacement");
        };
        let snapshot = snapshot_event(prepared.snapshot(), relay);
        prepared
            .commit_with_snapshot(&snapshot)
            .await
            .expect("commit Project and snapshot")
    }

    async fn live_snapshot(
        db: &Db,
        community: CommunityId,
        relay: &Keys,
        project_coordinate: &str,
    ) -> Option<(Vec<u8>, Value)> {
        let snapshot_d = buzz_core::project_related_channels::project_related_channels_snapshot_d(
            project_coordinate,
        );
        let relay_pubkey = relay.public_key().to_bytes();
        let row = sqlx::query(
            "SELECT id, tags FROM events \
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 \
               AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT as i32)
        .bind(relay_pubkey.as_slice())
        .bind(snapshot_d)
        .fetch_optional(&db.pool)
        .await
        .expect("query live snapshot")?;
        Some((
            row.try_get("id").expect("snapshot id"),
            row.try_get("tags").expect("snapshot tags"),
        ))
    }

    async fn live_snapshot_count(db: &Db, community: CommunityId, project_coordinate: &str) -> i64 {
        let snapshot_d = buzz_core::project_related_channels::project_related_channels_snapshot_d(
            project_coordinate,
        );
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND kind = $2 AND d_tag = $3 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT as i32)
        .bind(snapshot_d)
        .fetch_one(&db.pool)
        .await
        .expect("count live snapshots")
    }

    fn snapshot_entries(tags: &Value) -> Vec<Uuid> {
        tags.as_array()
            .into_iter()
            .flatten()
            .filter_map(|tag| {
                let parts = tag.as_array()?;
                if parts.first()?.as_str()? != "c" || parts.len() != 2 {
                    return None;
                }
                Uuid::parse_str(parts.get(1)?.as_str()?).ok()
            })
            .collect()
    }

    struct ProjectFixture {
        db: Db,
        community: CommunityId,
        project_owner: Keys,
        actor: Keys,
        relay: Keys,
        home: ChannelRecord,
        target: ChannelRecord,
        legacy_target: ChannelRecord,
        project_d: String,
        project: Event,
    }

    async fn make_project_fixture() -> ProjectFixture {
        let (db, community) = make_db().await;
        let project_owner = Keys::generate();
        let actor = Keys::generate();
        let relay = Keys::generate();
        let owner_bytes = project_owner.public_key().to_bytes();
        let actor_bytes = actor.public_key().to_bytes();
        let home = db
            .create_channel(
                community,
                &format!("project-home-{}", Uuid::new_v4().simple()),
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                owner_bytes.as_slice(),
                None,
            )
            .await
            .expect("create home channel");
        let target = db
            .create_channel(
                community,
                &format!("project-target-{}", Uuid::new_v4().simple()),
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                owner_bytes.as_slice(),
                None,
            )
            .await
            .expect("create target channel");
        let legacy_target = db
            .create_channel(
                community,
                &format!("project-legacy-target-{}", Uuid::new_v4().simple()),
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                owner_bytes.as_slice(),
                None,
            )
            .await
            .expect("create legacy target channel");
        for (channel_id, role) in [
            (home.id, MemberRole::Admin),
            (target.id, MemberRole::Member),
            (legacy_target.id, MemberRole::Member),
        ] {
            db.add_member(
                community,
                channel_id,
                actor_bytes.as_slice(),
                role,
                Some(owner_bytes.as_slice()),
            )
            .await
            .expect("add fixture actor");
        }
        let project_d = format!("project-{}", Uuid::new_v4().simple());
        let project = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags(vec![
                Tag::parse(["d", &project_d]).expect("d tag"),
                Tag::parse(["buzz-channel", &home.id.to_string()]).expect("home tag"),
            ])
            .sign_with_keys(&project_owner)
            .expect("sign Project");
        commit_project_replacement(&db, community, &project, &project_d, &relay).await;
        ProjectFixture {
            db,
            community,
            project_owner,
            actor,
            relay,
            home,
            target,
            legacy_target,
            project_d,
            project,
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn command_apply_is_atomic_and_noops_are_not_stored() {
        let ProjectFixture {
            db,
            community,
            project_owner,
            actor,
            relay,
            home,
            target,
            legacy_target: _,
            project_d,
            project,
        } = make_project_fixture().await;
        let owner_bytes = project_owner.public_key().to_bytes();
        let relay_bytes = relay.public_key().to_bytes();
        let project_coordinate = format!(
            "{KIND_PROJECT}:{}:{project_d}",
            hex::encode(owner_bytes.as_slice())
        );
        let project_with_latent_home = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags(vec![
                Tag::parse(["d", &project_d]).expect("d tag"),
                Tag::parse(["buzz-channel", &home.id.to_string()]).expect("home tag"),
                Tag::parse(["buzz-related-channel", &home.id.to_string()])
                    .expect("latent home relation"),
            ])
            .custom_created_at(nostr::Timestamp::from(project.created_at.as_secs() + 1))
            .sign_with_keys(&project_owner)
            .expect("sign Project with latent home relation");
        commit_project_replacement(
            &db,
            community,
            &project_with_latent_home,
            &project_d,
            &relay,
        )
        .await;
        let (initial_snapshot_id, initial_snapshot_tags) =
            live_snapshot(&db, community, &relay, &project_coordinate)
                .await
                .expect("initial Project snapshot");
        assert!(snapshot_entries(&initial_snapshot_tags).is_empty());
        let deleted_snapshot_created_at: i64 = sqlx::query_scalar(
            "UPDATE events \
             SET created_at = NOW() + INTERVAL '1 day', deleted_at = NOW() \
             WHERE community_id = $1 AND id = $2 \
             RETURNING FLOOR(EXTRACT(EPOCH FROM created_at))::bigint",
        )
        .bind(community.as_uuid())
        .bind(initial_snapshot_id)
        .fetch_one(&db.pool)
        .await
        .expect("soft-delete a future-dated prior snapshot");

        let remove_home = command_event(
            &actor,
            owner_bytes.as_slice(),
            &project_d,
            home.id,
            "remove",
        );
        let outcome = apply_project_related_channel_command(
            &db,
            community,
            &remove_home,
            relay_bytes.as_slice(),
            owner_bytes.as_slice(),
            &project_d,
            home.id,
            ProjectRelatedChannelChange::Remove,
        )
        .await
        .expect("remove latent home-channel relation");
        let ApplyProjectRelatedChannelOutcome::Applied(prepared) = outcome else {
            panic!("expected latent home-channel relation removal");
        };
        assert_eq!(
            prepared.snapshot().created_at,
            deleted_snapshot_created_at as u64 + 1,
            "new snapshots must outrank deleted snapshot history"
        );
        let snapshot = snapshot_event(prepared.snapshot(), &relay);
        prepared
            .commit_with_snapshot(&snapshot)
            .await
            .expect("commit latent home-channel removal");
        let add_home = command_event(&actor, owner_bytes.as_slice(), &project_d, home.id, "add");
        assert!(matches!(
            apply_project_related_channel_command(
                &db,
                community,
                &add_home,
                relay_bytes.as_slice(),
                owner_bytes.as_slice(),
                &project_d,
                home.id,
                ProjectRelatedChannelChange::Add,
            )
            .await
            .expect("adding the Project home as related is invalid"),
            ApplyProjectRelatedChannelOutcome::InvalidTarget(
                "Project home channel cannot also be a related channel"
            )
        ));

        let dm = db
            .create_channel(
                community,
                &format!("project-dm-target-{}", Uuid::new_v4().simple()),
                ChannelType::Dm,
                ChannelVisibility::Private,
                None,
                owner_bytes.as_slice(),
                None,
            )
            .await
            .expect("create DM target");
        db.add_member(
            community,
            dm.id,
            actor.public_key().to_bytes().as_slice(),
            MemberRole::Member,
            Some(owner_bytes.as_slice()),
        )
        .await
        .expect("add actor to DM target");
        let add_dm = command_event(&actor, owner_bytes.as_slice(), &project_d, dm.id, "add");
        assert!(matches!(
            apply_project_related_channel_command(
                &db,
                community,
                &add_dm,
                relay_bytes.as_slice(),
                owner_bytes.as_slice(),
                &project_d,
                dm.id,
                ProjectRelatedChannelChange::Add,
            )
            .await
            .expect("reject DM relation"),
            ApplyProjectRelatedChannelOutcome::InvalidTarget(
                "DM channels cannot be related to Projects"
            )
        ));
        let project_with_latent_dm = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags(vec![
                Tag::parse(["d", &project_d]).expect("d tag"),
                Tag::parse(["buzz-channel", &home.id.to_string()]).expect("home tag"),
                Tag::parse(["buzz-related-channel", &dm.id.to_string()])
                    .expect("latent DM relation"),
            ])
            .custom_created_at(nostr::Timestamp::from(project.created_at.as_secs() + 2))
            .sign_with_keys(&project_owner)
            .expect("sign Project with latent DM relation");
        commit_project_replacement(&db, community, &project_with_latent_dm, &project_d, &relay)
            .await;
        let remove_dm = command_event(&actor, owner_bytes.as_slice(), &project_d, dm.id, "remove");
        let outcome = apply_project_related_channel_command(
            &db,
            community,
            &remove_dm,
            relay_bytes.as_slice(),
            owner_bytes.as_slice(),
            &project_d,
            dm.id,
            ProjectRelatedChannelChange::Remove,
        )
        .await
        .expect("remove latent DM relation");
        commit_applied(outcome, &relay).await;

        let add = command_event(&actor, owner_bytes.as_slice(), &project_d, target.id, "add");
        let outcome = apply_project_related_channel_command(
            &db,
            community,
            &add,
            relay_bytes.as_slice(),
            owner_bytes.as_slice(),
            &project_d,
            target.id,
            ProjectRelatedChannelChange::Add,
        )
        .await
        .expect("apply add");
        let ApplyProjectRelatedChannelOutcome::Applied(prepared) = outcome else {
            panic!("expected prepared add");
        };
        let wrong_snapshot = snapshot_event(prepared.snapshot(), &actor);
        assert!(prepared
            .commit_with_snapshot(&wrong_snapshot)
            .await
            .is_err());
        assert!(db
            .get_event_by_id(community, add.id.as_bytes())
            .await
            .expect("query rolled-back add")
            .is_none());
        let outcome = apply_project_related_channel_command(
            &db,
            community,
            &add,
            relay_bytes.as_slice(),
            owner_bytes.as_slice(),
            &project_d,
            target.id,
            ProjectRelatedChannelChange::Add,
        )
        .await
        .expect("retry add after rejected snapshot");
        commit_applied(outcome, &relay).await;
        let (_, add_snapshot_tags) = live_snapshot(&db, community, &relay, &project_coordinate)
            .await
            .expect("add snapshot");
        assert_eq!(snapshot_entries(&add_snapshot_tags), vec![target.id]);
        assert!(matches!(
            apply_project_related_channel_command(
                &db,
                community,
                &add,
                relay_bytes.as_slice(),
                owner_bytes.as_slice(),
                &project_d,
                target.id,
                ProjectRelatedChannelChange::Add,
            )
            .await
            .expect("replay add"),
            ApplyProjectRelatedChannelOutcome::Replay
        ));

        let no_op_add = command_event(&actor, owner_bytes.as_slice(), &project_d, target.id, "add");
        sqlx::query(
            "UPDATE channel_members SET removed_at = NOW() \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
        )
        .bind(community.as_uuid())
        .bind(target.id)
        .bind(actor.public_key().to_bytes().as_slice())
        .execute(&db.pool)
        .await
        .expect("remove actor from already-related target");
        assert!(matches!(
            apply_project_related_channel_command(
                &db,
                community,
                &no_op_add,
                relay_bytes.as_slice(),
                owner_bytes.as_slice(),
                &project_d,
                target.id,
                ProjectRelatedChannelChange::Add,
            )
            .await
            .expect("no-op add after target membership loss"),
            ApplyProjectRelatedChannelOutcome::Noop
        ));
        assert!(
            db.get_event_by_id(community, no_op_add.id.as_bytes())
                .await
                .expect("query no-op event")
                .is_none(),
            "a no-op request must not leave an event without a matching override"
        );

        let remove = command_event(
            &actor,
            owner_bytes.as_slice(),
            &project_d,
            target.id,
            "remove",
        );
        let outcome = apply_project_related_channel_command(
            &db,
            community,
            &remove,
            relay_bytes.as_slice(),
            owner_bytes.as_slice(),
            &project_d,
            target.id,
            ProjectRelatedChannelChange::Remove,
        )
        .await
        .expect("apply remove");
        commit_applied(outcome, &relay).await;
        let (_, remove_snapshot_tags) = live_snapshot(&db, community, &relay, &project_coordinate)
            .await
            .expect("remove snapshot");
        assert!(snapshot_entries(&remove_snapshot_tags).is_empty());

        let no_op_remove = command_event(
            &actor,
            owner_bytes.as_slice(),
            &project_d,
            target.id,
            "remove",
        );
        assert!(matches!(
            apply_project_related_channel_command(
                &db,
                community,
                &no_op_remove,
                relay_bytes.as_slice(),
                owner_bytes.as_slice(),
                &project_d,
                target.id,
                ProjectRelatedChannelChange::Remove,
            )
            .await
            .expect("no-op remove"),
            ApplyProjectRelatedChannelOutcome::Noop
        ));
        assert!(db
            .get_event_by_id(community, no_op_remove.id.as_bytes())
            .await
            .expect("query no-op remove")
            .is_none());

        assert_eq!(
            live_snapshot_count(&db, community, &project_coordinate).await,
            1
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn legacy_overlay_survives_replacement_and_removal_after_archive() {
        let ProjectFixture {
            db,
            community,
            project_owner,
            actor,
            relay,
            home,
            target,
            legacy_target,
            project_d,
            project,
        } = make_project_fixture().await;
        let owner_bytes = project_owner.public_key().to_bytes();
        let actor_bytes = actor.public_key().to_bytes();
        let relay_bytes = relay.public_key().to_bytes();
        let project_coordinate = format!(
            "{KIND_PROJECT}:{}:{project_d}",
            hex::encode(owner_bytes.as_slice())
        );

        let add = command_event(&actor, owner_bytes.as_slice(), &project_d, target.id, "add");
        let outcome = apply_project_related_channel_command(
            &db,
            community,
            &add,
            relay_bytes.as_slice(),
            owner_bytes.as_slice(),
            &project_d,
            target.id,
            ProjectRelatedChannelChange::Add,
        )
        .await
        .expect("add target before Project replacement");
        commit_applied(outcome, &relay).await;

        let remove = command_event(
            &actor,
            owner_bytes.as_slice(),
            &project_d,
            target.id,
            "remove",
        );
        let outcome = apply_project_related_channel_command(
            &db,
            community,
            &remove,
            relay_bytes.as_slice(),
            owner_bytes.as_slice(),
            &project_d,
            target.id,
            ProjectRelatedChannelChange::Remove,
        )
        .await
        .expect("remove target before Project replacement");
        commit_applied(outcome, &relay).await;

        let replacement = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags(vec![
                Tag::parse(["d", &project_d]).expect("d tag"),
                Tag::parse(["buzz-channel", &home.id.to_string()]).expect("home tag"),
                Tag::parse(["buzz-related-channel", &target.id.to_string()])
                    .expect("legacy relation tag"),
                Tag::parse(["buzz-related-channel", &legacy_target.id.to_string()])
                    .expect("second legacy relation tag"),
            ])
            .custom_created_at(nostr::Timestamp::from(project.created_at.as_secs() + 1))
            .sign_with_keys(&project_owner)
            .expect("sign replacement Project");
        commit_project_replacement(&db, community, &replacement, &project_d, &relay).await;
        let (_, replacement_snapshot_tags) =
            live_snapshot(&db, community, &relay, &project_coordinate)
                .await
                .expect("Project replacement snapshot");
        assert_eq!(
            snapshot_entries(&replacement_snapshot_tags),
            vec![legacy_target.id]
        );
        assert_eq!(
            live_snapshot_count(&db, community, &project_coordinate).await,
            1
        );
        let remove_after_replacement = command_event(
            &actor,
            owner_bytes.as_slice(),
            &project_d,
            target.id,
            "remove",
        );
        assert!(matches!(
            apply_project_related_channel_command(
                &db,
                community,
                &remove_after_replacement,
                relay_bytes.as_slice(),
                owner_bytes.as_slice(),
                &project_d,
                target.id,
                ProjectRelatedChannelChange::Remove,
            )
            .await
            .expect("override survives Project replacement"),
            ApplyProjectRelatedChannelOutcome::Noop
        ));

        sqlx::query("UPDATE channels SET archived_at = NOW() WHERE community_id = $1 AND id = $2")
            .bind(community.as_uuid())
            .bind(legacy_target.id)
            .execute(&db.pool)
            .await
            .expect("archive legacy target");
        sqlx::query(
            "UPDATE channel_members SET removed_at = NOW() \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
        )
        .bind(community.as_uuid())
        .bind(legacy_target.id)
        .bind(actor_bytes.as_slice())
        .execute(&db.pool)
        .await
        .expect("remove actor from legacy target");
        let legacy_remove = command_event(
            &actor,
            owner_bytes.as_slice(),
            &project_d,
            legacy_target.id,
            "remove",
        );
        let outcome = apply_project_related_channel_command(
            &db,
            community,
            &legacy_remove,
            relay_bytes.as_slice(),
            owner_bytes.as_slice(),
            &project_d,
            legacy_target.id,
            ProjectRelatedChannelChange::Remove,
        )
        .await
        .expect("remove archived legacy target after membership loss");
        commit_applied(outcome, &relay).await;
        let (_, legacy_remove_snapshot_tags) =
            live_snapshot(&db, community, &relay, &project_coordinate)
                .await
                .expect("legacy remove snapshot");
        assert!(snapshot_entries(&legacy_remove_snapshot_tags).is_empty());

        let second_replacement = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags(vec![
                Tag::parse(["d", &project_d]).expect("d tag"),
                Tag::parse(["buzz-channel", &home.id.to_string()]).expect("home tag"),
                Tag::parse(["buzz-related-channel", &legacy_target.id.to_string()])
                    .expect("reasserted legacy relation tag"),
            ])
            .custom_created_at(nostr::Timestamp::from(project.created_at.as_secs() + 2))
            .sign_with_keys(&project_owner)
            .expect("sign second replacement Project");
        commit_project_replacement(&db, community, &second_replacement, &project_d, &relay).await;
        let remove_reasserted_legacy = command_event(
            &actor,
            owner_bytes.as_slice(),
            &project_d,
            legacy_target.id,
            "remove",
        );
        assert!(matches!(
            apply_project_related_channel_command(
                &db,
                community,
                &remove_reasserted_legacy,
                relay_bytes.as_slice(),
                owner_bytes.as_slice(),
                &project_d,
                legacy_target.id,
                ProjectRelatedChannelChange::Remove,
            )
            .await
            .expect("absent override survives owner reassertion"),
            ApplyProjectRelatedChannelOutcome::Noop
        ));

        sqlx::query("UPDATE channels SET archived_at = NOW() WHERE community_id = $1 AND id = $2")
            .bind(community.as_uuid())
            .bind(home.id)
            .execute(&db.pool)
            .await
            .expect("archive home channel");
        let admin_after_archive =
            command_event(&actor, owner_bytes.as_slice(), &project_d, target.id, "add");
        assert!(matches!(
            apply_project_related_channel_command(
                &db,
                community,
                &admin_after_archive,
                relay_bytes.as_slice(),
                owner_bytes.as_slice(),
                &project_d,
                target.id,
                ProjectRelatedChannelChange::Add,
            )
            .await
            .expect("check archived-home admin"),
            ApplyProjectRelatedChannelOutcome::Unauthorized
        ));

        let owner_after_archive = command_event(
            &project_owner,
            owner_bytes.as_slice(),
            &project_d,
            target.id,
            "add",
        );
        let outcome = apply_project_related_channel_command(
            &db,
            community,
            &owner_after_archive,
            relay_bytes.as_slice(),
            owner_bytes.as_slice(),
            &project_d,
            target.id,
            ProjectRelatedChannelChange::Add,
        )
        .await
        .expect("Project signer remains authorized");
        commit_applied(outcome, &relay).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn command_authorization_uses_project_and_home_channel_rules() {
        use crate::user::{ensure_user, set_agent_owner};

        let (db, community) = make_db().await;
        let relay = Keys::generate();
        let channel_owner = Keys::generate();
        let project_agent = Keys::generate();
        let project_human = Keys::generate();
        let ordinary_member = Keys::generate();
        let admin_agent = Keys::generate();
        let admin_human = Keys::generate();
        for keys in [
            &channel_owner,
            &project_agent,
            &project_human,
            &ordinary_member,
            &admin_agent,
            &admin_human,
        ] {
            ensure_user(&db.pool, community, keys.public_key().to_bytes().as_slice())
                .await
                .expect("ensure authorization-test user");
        }
        let project_agent_bytes = project_agent.public_key().to_bytes();
        let project_human_bytes = project_human.public_key().to_bytes();
        let admin_agent_bytes = admin_agent.public_key().to_bytes();
        let admin_human_bytes = admin_human.public_key().to_bytes();
        set_agent_owner(
            &db.pool,
            community,
            project_agent_bytes.as_slice(),
            project_human_bytes.as_slice(),
        )
        .await
        .expect("register Project agent owner");
        set_agent_owner(
            &db.pool,
            community,
            admin_agent_bytes.as_slice(),
            admin_human_bytes.as_slice(),
        )
        .await
        .expect("register admin agent owner");

        let channel_owner_bytes = channel_owner.public_key().to_bytes();
        let mut channels = Vec::new();
        for label in ["home", "allowed", "admin-human", "nonmember"] {
            channels.push(
                db.create_channel(
                    community,
                    &format!("project-auth-{label}-{}", Uuid::new_v4().simple()),
                    ChannelType::Stream,
                    ChannelVisibility::Open,
                    None,
                    channel_owner_bytes.as_slice(),
                    None,
                )
                .await
                .expect("create authorization-test channel"),
            );
        }
        let [home, allowed_target, admin_human_target, nonmember_target] = channels.as_slice()
        else {
            unreachable!("four authorization-test channels were created")
        };
        let ordinary_bytes = ordinary_member.public_key().to_bytes();
        for (channel_id, pubkey, role) in [
            (home.id, ordinary_bytes.as_slice(), MemberRole::Member),
            (
                allowed_target.id,
                ordinary_bytes.as_slice(),
                MemberRole::Member,
            ),
            (home.id, admin_agent_bytes.as_slice(), MemberRole::Admin),
            (
                admin_human_target.id,
                admin_human_bytes.as_slice(),
                MemberRole::Member,
            ),
            (
                allowed_target.id,
                project_human_bytes.as_slice(),
                MemberRole::Member,
            ),
        ] {
            db.add_member(
                community,
                channel_id,
                pubkey,
                role,
                Some(channel_owner_bytes.as_slice()),
            )
            .await
            .expect("add authorization-test member");
        }

        let project_d = format!("project-auth-{}", Uuid::new_v4().simple());
        let project = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags(vec![
                Tag::parse(["d", &project_d]).expect("d tag"),
                Tag::parse(["buzz-channel", &home.id.to_string()]).expect("home tag"),
            ])
            .sign_with_keys(&project_agent)
            .expect("sign agent-owned Project");
        commit_project_replacement(&db, community, &project, &project_d, &relay).await;
        let relay_bytes = relay.public_key().to_bytes();

        for (actor, target, expected_unauthorized) in [
            (&ordinary_member, allowed_target.id, true),
            (&admin_human, admin_human_target.id, true),
            (&project_human, allowed_target.id, false),
        ] {
            let command = command_event(
                actor,
                project_agent_bytes.as_slice(),
                &project_d,
                target,
                "add",
            );
            let outcome = apply_project_related_channel_command(
                &db,
                community,
                &command,
                relay_bytes.as_slice(),
                project_agent_bytes.as_slice(),
                &project_d,
                target,
                ProjectRelatedChannelChange::Add,
            )
            .await
            .expect("evaluate authorization-test command");
            if expected_unauthorized {
                assert!(matches!(
                    outcome,
                    ApplyProjectRelatedChannelOutcome::Unauthorized
                ));
            } else {
                commit_applied(outcome, &relay).await;
            }
        }

        let nonmember_add = command_event(
            &project_agent,
            project_agent_bytes.as_slice(),
            &project_d,
            nonmember_target.id,
            "add",
        );
        assert!(matches!(
            apply_project_related_channel_command(
                &db,
                community,
                &nonmember_add,
                relay_bytes.as_slice(),
                project_agent_bytes.as_slice(),
                &project_d,
                nonmember_target.id,
                ProjectRelatedChannelChange::Add,
            )
            .await
            .expect("reject nonmember add"),
            ApplyProjectRelatedChannelOutcome::InvalidTarget(
                "target channel must be active and the actor must be a member"
            )
        ));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn deleting_project_by_coordinate_or_event_id_deletes_snapshot_atomically() {
        let (db, community) = make_db().await;
        let project_owner = Keys::generate();
        let relay = Keys::generate();
        let owner_bytes = project_owner.public_key().to_bytes();

        for delete_by_coordinate in [true, false] {
            let project_d = format!("project-delete-{}", Uuid::new_v4().simple());
            let project = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
                .tag(Tag::parse(["d", &project_d]).expect("d tag"))
                .sign_with_keys(&project_owner)
                .expect("sign Project");
            commit_project_replacement(&db, community, &project, &project_d, &relay).await;
            let coordinate = format!(
                "{KIND_PROJECT}:{}:{project_d}",
                hex::encode(owner_bytes.as_slice())
            );
            assert!(live_snapshot(&db, community, &relay, &coordinate)
                .await
                .is_some());

            let deleted = if delete_by_coordinate {
                db.soft_delete_by_coordinate(
                    community,
                    KIND_PROJECT as i32,
                    owner_bytes.as_slice(),
                    &project_d,
                    project.created_at.as_secs() as i64,
                )
                .await
                .expect("delete Project by coordinate")
            } else {
                db.soft_delete_parameterized_event_by_id(community, project.id.as_bytes())
                    .await
                    .expect("delete Project by event id")
            };
            assert!(deleted);
            assert!(live_snapshot(&db, community, &relay, &coordinate)
                .await
                .is_none());
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn command_add_rejects_when_effective_snapshot_is_at_cap() {
        let (db, community) = make_db().await;
        let project_owner = Keys::generate();
        let relay = Keys::generate();
        let owner_bytes = project_owner.public_key().to_bytes();
        let relay_bytes = relay.public_key().to_bytes();
        let target = db
            .create_channel(
                community,
                &format!("project-cap-target-{}", Uuid::new_v4().simple()),
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                owner_bytes.as_slice(),
                None,
            )
            .await
            .expect("create target channel");
        let rejected_target = db
            .create_channel(
                community,
                &format!("project-cap-rejected-{}", Uuid::new_v4().simple()),
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                owner_bytes.as_slice(),
                None,
            )
            .await
            .expect("create rejected target channel");
        let project_d = format!("project-cap-{}", Uuid::new_v4().simple());
        let home_id = Uuid::from_u128(1);
        let mut project_tags = vec![
            Tag::parse(["d", &project_d]).expect("d tag"),
            Tag::parse(["buzz-channel", &home_id.to_string()]).expect("home tag"),
            Tag::parse(["buzz-related-channel", &home_id.to_string()])
                .expect("legacy home relation"),
        ];
        for index in 0..buzz_core::project_related_channels::PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP {
            project_tags.push(
                Tag::parse([
                    "buzz-related-channel",
                    &Uuid::from_u128(index as u128 + 2).to_string(),
                ])
                .expect("legacy related channel"),
            );
        }
        let project = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags(project_tags.clone())
            .sign_with_keys(&project_owner)
            .expect("sign Project");
        let prepared = prepare_project_replacement(
            &db,
            community,
            &project,
            &project_d,
            relay_bytes.as_slice(),
        )
        .await
        .expect("prepare capped Project");
        let PrepareProjectReplacementOutcome::Applied(prepared) = prepared else {
            panic!("expected capped Project replacement");
        };
        assert_eq!(
            prepared.snapshot().entries.len(),
            buzz_core::project_related_channels::PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP
        );
        assert!(!prepared.snapshot().truncated);
        assert!(!prepared.snapshot().entries.contains(&home_id));
        let snapshot = snapshot_event(prepared.snapshot(), &relay);
        prepared
            .commit_with_snapshot(&snapshot)
            .await
            .expect("commit capped Project and snapshot");
        let coordinate = format!(
            "{KIND_PROJECT}:{}:{project_d}",
            hex::encode(owner_bytes.as_slice())
        );
        let (_, capped_snapshot_tags) = live_snapshot(&db, community, &relay, &coordinate)
            .await
            .expect("capped owner snapshot");
        assert_eq!(
            snapshot_entries(&capped_snapshot_tags).len(),
            buzz_core::project_related_channels::PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP
        );

        sqlx::query(
            "INSERT INTO project_related_channel_overrides \
                 (community_id, project_owner, project_d, channel_id, present) \
             VALUES ($1, $2, $3, $4, TRUE)",
        )
        .bind(community.as_uuid())
        .bind(owner_bytes.as_slice())
        .bind(&project_d)
        .bind(target.id)
        .execute(&db.pool)
        .await
        .expect("seed prior command override");
        let replacement = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags(project_tags)
            .custom_created_at(nostr::Timestamp::from(project.created_at.as_secs() + 1))
            .sign_with_keys(&project_owner)
            .expect("sign over-cap Project replacement");
        let prepared = prepare_project_replacement(
            &db,
            community,
            &replacement,
            &project_d,
            relay_bytes.as_slice(),
        )
        .await
        .expect("prepare over-cap Project replacement");
        let PrepareProjectReplacementOutcome::Applied(prepared) = prepared else {
            panic!("expected over-cap Project replacement");
        };
        assert!(prepared.snapshot().truncated);
        assert_eq!(
            prepared.snapshot().entries.len(),
            buzz_core::project_related_channels::PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP
        );
        let snapshot = snapshot_event(prepared.snapshot(), &relay);
        prepared
            .commit_with_snapshot(&snapshot)
            .await
            .expect("commit truncated Project snapshot");

        let add = command_event(
            &project_owner,
            owner_bytes.as_slice(),
            &project_d,
            rejected_target.id,
            "add",
        );
        let outcome = apply_project_related_channel_command(
            &db,
            community,
            &add,
            relay_bytes.as_slice(),
            owner_bytes.as_slice(),
            &project_d,
            rejected_target.id,
            ProjectRelatedChannelChange::Add,
        )
        .await
        .expect("apply over-cap add");
        assert!(matches!(
            outcome,
            ApplyProjectRelatedChannelOutcome::InvalidTarget(
                "Project already has 64 related channels"
            )
        ));
        assert!(db
            .get_event_by_id(community, add.id.as_bytes())
            .await
            .expect("query rejected add")
            .is_none());
    }
}
