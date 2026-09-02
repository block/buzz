//! Transactional persistence for Project lifecycle state and relay projections.

use std::collections::BTreeSet;

use buzz_core::kind::{
    event_kind_u32, KIND_DELETION, KIND_PROJECT, KIND_PROJECT_CHANGE, KIND_PROJECT_STATE,
};
use buzz_core::project_state::{
    project_state_template, ProjectStateProjectionInput, ProjectStateTemplate,
};
use buzz_core::{CommunityId, StoredEvent};
use chrono::{DateTime, Utc};
use nostr::{Event, EventId};
use sqlx::Row;
use uuid::Uuid;

use crate::event::insert_event_in_transaction;
use crate::replaceable::{
    event_replacement_lock_key, ParameterizedReplacePrecondition, ParameterizedReplaceStatus,
};
use crate::{Db, DbError, Result};

const RELATED_CHANNEL_CAP: usize = 64;

/// A validated Project related-channel mutation supplied by the relay parser.
#[derive(Clone, Copy, Debug)]
pub struct ProjectRelatedChannelChange<'a> {
    /// Owner pubkey from the canonical kind:30621 coordinate.
    pub project_owner: &'a [u8],
    /// Verbatim `d` value from the canonical kind:30621 coordinate.
    pub project_d_tag: &'a str,
    /// Relational revision observed by the actor.
    pub expected_revision: i64,
    /// Channels to add to the effective related-channel set.
    pub add: &'a [Uuid],
    /// Channels to remove from the effective related-channel set.
    pub remove: &'a [Uuid],
}

/// Outcome of applying one Project related-channel command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectChangeApplyResult {
    /// The command and mutation committed atomically.
    Applied {
        /// New authoritative revision.
        revision: i64,
    },
    /// This exact command already committed at its original revision.
    Duplicate {
        /// Revision originally produced by the command.
        applied_revision: i64,
    },
    /// No live owner-signed Project exists at the coordinate.
    ProjectNotFound,
    /// The Project is deleted and awaits owner recreation.
    ProjectDeleted,
    /// The actor is neither the Project owner nor a current home-channel owner/admin.
    Forbidden,
    /// The expected revision is stale.
    Conflict {
        /// Current authoritative revision.
        current_revision: i64,
    },
    /// The patch or owner identity is invalid.
    Invalid(String),
}

/// Coherent Project state awaiting a relay-signed kind:30623 projection.
#[derive(Clone, Debug)]
pub struct ProjectStateProjectionCandidate {
    community_id: CommunityId,
    template: ProjectStateTemplate,
    previous_created_at: Option<u64>,
    project_owner: Vec<u8>,
    project_d_tag: String,
    revision: i64,
    identity_event_id: Vec<u8>,
    change_event_id: Vec<u8>,
    observed_projected_revision: i64,
    observed_projection_pubkey: Option<Vec<u8>>,
    projection_pubkey: Vec<u8>,
}

impl ProjectStateProjectionCandidate {
    /// Community containing the Project.
    #[must_use]
    pub const fn community_id(&self) -> CommunityId {
        self.community_id
    }

    /// Unsigned canonical fields the relay must timestamp and sign.
    #[must_use]
    pub const fn template(&self) -> &ProjectStateTemplate {
        &self.template
    }

    /// Timestamp of the current live projection for this relay key, if any.
    #[must_use]
    pub const fn previous_created_at(&self) -> Option<u64> {
        self.previous_created_at
    }
}

/// Outcome of committing a relay-signed Project State projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectStateProjectionCommitResult {
    /// The projection and durable retry marker committed atomically.
    Committed,
    /// Project state or projection ownership changed after the candidate loaded.
    Stale,
}

/// Result category for an owner identity or deletion lifecycle event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectLifecycleStatus {
    /// The event changed authoritative Project state.
    Applied,
    /// The event was newly stored but had no effect on the current Project head.
    NoEffect,
    /// The exact event was already stored.
    Duplicate,
    /// A newer owner identity already dominates the submitted identity.
    Superseded,
}

/// Atomic persistence result for a Project lifecycle event.
#[derive(Clone, Debug)]
pub struct ProjectLifecycleApplyResult {
    /// Stored representation used by relay dispatch when the event was inserted.
    pub event: StoredEvent,
    /// Whether and how authoritative Project state changed.
    pub status: ProjectLifecycleStatus,
}

impl ProjectLifecycleApplyResult {
    /// Whether this call newly persisted the submitted event.
    #[must_use]
    pub fn was_inserted(&self) -> bool {
        matches!(
            self.status,
            ProjectLifecycleStatus::Applied | ProjectLifecycleStatus::NoEffect
        )
    }
}
fn tag_parts(tag: &serde_json::Value) -> Option<Vec<&str>> {
    tag.as_array()?
        .iter()
        .map(serde_json::Value::as_str)
        .collect()
}

fn parse_base_state(
    tags: &serde_json::Value,
) -> std::result::Result<(Option<Uuid>, BTreeSet<Uuid>), String> {
    let mut home = None;
    let mut related = BTreeSet::new();
    for tag in tags.as_array().ok_or("Project tags are not an array")? {
        let Some(parts) = tag_parts(tag) else {
            return Err("Project contains a non-string tag".into());
        };
        match parts.as_slice() {
            ["buzz-channel", value] => {
                let channel = Uuid::parse_str(value)
                    .ok()
                    .filter(|id| id.to_string() == *value);
                home = channel;
            }
            ["buzz-related-channel", value] => {
                let channel = Uuid::parse_str(value)
                    .ok()
                    .filter(|id| id.to_string() == *value)
                    .ok_or("Project contains a non-canonical related channel")?;
                if !related.insert(channel) {
                    return Err("Project contains a duplicate related channel".into());
                }
            }
            ["buzz-related-channel", ..] => {
                return Err("Project contains a malformed related-channel tag".into());
            }
            _ => {}
        }
    }
    if related.len() > RELATED_CHANNEL_CAP {
        return Err("Project contains more than 64 related channels".into());
    }
    if home.is_some_and(|channel| related.contains(&channel)) {
        return Err("Project home channel cannot also be related".into());
    }
    Ok((home, related))
}

fn validate_patch(change: ProjectRelatedChannelChange<'_>) -> Option<String> {
    if change.project_owner.len() != 32 {
        return Some("Project owner must be 32 bytes".into());
    }
    if change.project_d_tag.is_empty() || change.project_d_tag.len() > crate::event::D_TAG_MAX_LEN {
        return Some("Project d tag is empty or too long".into());
    }
    if change.expected_revision < 1 {
        return Some("expected revision must be positive".into());
    }
    if change.add.is_empty() && change.remove.is_empty() {
        return Some("Project change must not be empty".into());
    }
    if change.add.len() > RELATED_CHANNEL_CAP || change.remove.len() > RELATED_CHANNEL_CAP {
        return Some("Project change exceeds the 64-channel patch bound".into());
    }
    let add = change.add.iter().copied().collect::<BTreeSet<_>>();
    let remove = change.remove.iter().copied().collect::<BTreeSet<_>>();
    if add.len() != change.add.len() || remove.len() != change.remove.len() {
        return Some("Project change contains a duplicate channel".into());
    }
    if !add.is_disjoint(&remove) {
        return Some("Project change adds and removes the same channel".into());
    }
    None
}

async fn replace_related_channels(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    community_id: CommunityId,
    owner: &[u8],
    d_tag: &str,
    related: &BTreeSet<Uuid>,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM project_related_channels WHERE community_id=$1 \
         AND project_owner=$2 AND project_d_tag=$3",
    )
    .bind(community_id.as_uuid())
    .bind(owner)
    .bind(d_tag)
    .execute(&mut **tx)
    .await?;
    for channel in related {
        sqlx::query(
            "INSERT INTO project_related_channels \
             (community_id, project_owner, project_d_tag, channel_id) VALUES ($1,$2,$3,$4)",
        )
        .bind(community_id.as_uuid())
        .bind(owner)
        .bind(d_tag)
        .bind(channel)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Adopt one still-live pre-relational Project identity while the caller holds
/// the Project coordinate lock.
async fn materialize_project_head_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    community_id: CommunityId,
    owner: &[u8],
    d_tag: &str,
    identity_event_id: &[u8],
    related: &BTreeSet<Uuid>,
) -> Result<bool> {
    let live_event_id: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT id FROM events WHERE community_id=$1 AND kind=$2 AND pubkey=$3 \
         AND d_tag=$4 AND deleted_at IS NULL ORDER BY created_at DESC, id ASC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(KIND_PROJECT as i32)
    .bind(owner)
    .bind(d_tag)
    .fetch_optional(&mut **tx)
    .await?;
    if live_event_id.as_deref() != Some(identity_event_id) {
        return Ok(false);
    }
    let inserted = sqlx::query(
        "INSERT INTO project_state_heads \
           (community_id, project_owner, project_d_tag, revision, deleted, identity_event_id, last_event_id) \
         VALUES ($1,$2,$3,1,FALSE,$4,$4) ON CONFLICT DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(owner)
    .bind(d_tag)
    .bind(identity_event_id)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        == 1;
    if inserted {
        replace_related_channels(tx, community_id, owner, d_tag, related).await?;
    }
    Ok(inserted)
}

impl Db {
    /// Atomically accept an owner-signed Project identity and materialize it.
    ///
    /// A newer identity is a full recovery snapshot. Existing relational
    /// revisions advance rather than resetting, including recreation after a
    /// deletion. Duplicate and superseded identities leave state unchanged.
    pub async fn apply_project_identity_event(
        &self,
        community_id: CommunityId,
        event: &Event,
    ) -> Result<ProjectLifecycleApplyResult> {
        if event_kind_u32(event) != KIND_PROJECT {
            return Err(DbError::InvalidData(
                "Project identity persistence requires kind 30621".into(),
            ));
        }
        let d_tag = crate::event::extract_d_tag(event).unwrap_or_default();
        if d_tag.is_empty() || d_tag.len() > crate::event::D_TAG_MAX_LEN {
            return Err(DbError::InvalidData("invalid Project d tag".into()));
        }
        let tags = serde_json::to_value(&event.tags)?;
        let (home_channel, mut related) = parse_base_state(&tags).map_err(DbError::InvalidData)?;
        let owner = event.pubkey.to_bytes();

        let mut tx = self.begin_transaction().await?;
        self.deletion_store()
            .guard_transaction(&mut tx, community_id)
            .await?;
        let coordinate_lock = event_replacement_lock_key(
            community_id,
            KIND_PROJECT as i32,
            owner.as_slice(),
            Some(d_tag.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(coordinate_lock)
            .execute(&mut *tx)
            .await?;
        let current_head = sqlx::query(
            "SELECT revision, deleted, last_event_id FROM project_state_heads \
             WHERE community_id=$1 AND project_owner=$2 AND project_d_tag=$3 FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(owner.as_slice())
        .bind(&d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(head) = current_head
            .as_ref()
            .filter(|head| head.get::<bool, _>("deleted"))
        {
            let tombstone_event_id: Vec<u8> = head.try_get("last_event_id")?;
            let tombstone_created_at: DateTime<Utc> =
                sqlx::query_scalar("SELECT created_at FROM events WHERE community_id=$1 AND id=$2")
                    .bind(community_id.as_uuid())
                    .bind(tombstone_event_id)
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or_else(|| {
                        DbError::InvalidData(
                            "Project tombstone event is missing from history".into(),
                        )
                    })?;
            let identity_created_at =
                DateTime::<Utc>::from_timestamp(event.created_at.as_secs() as i64, 0)
                    .ok_or_else(|| DbError::InvalidTimestamp(event.created_at.as_secs() as i64))?;
            // A tombstone dominates every identity in its second. Recreating a
            // Project requires an unambiguously later owner event.
            if identity_created_at <= tombstone_created_at {
                tx.rollback().await?;
                return Ok(ProjectLifecycleApplyResult {
                    event: StoredEvent::new(event.clone(), None),
                    status: ProjectLifecycleStatus::Superseded,
                });
            }
        }
        let persisted = self
            .replace_parameterized_event_in_transaction(
                &mut tx,
                community_id,
                event,
                &d_tag,
                None,
                ParameterizedReplacePrecondition::Unconditional,
            )
            .await?;
        match persisted.status {
            ParameterizedReplaceStatus::Inserted => {}
            ParameterizedReplaceStatus::Duplicate => {
                tx.rollback().await?;
                return Ok(ProjectLifecycleApplyResult {
                    event: persisted.event,
                    status: ProjectLifecycleStatus::Duplicate,
                });
            }
            _ => {
                tx.rollback().await?;
                return Ok(ProjectLifecycleApplyResult {
                    event: persisted.event,
                    status: ProjectLifecycleStatus::Superseded,
                });
            }
        }

        if current_head.is_some() {
            let preserved = sqlx::query_scalar::<_, Uuid>(
                "SELECT channel_id FROM project_related_channels WHERE community_id=$1 \
                 AND project_owner=$2 AND project_d_tag=$3",
            )
            .bind(community_id.as_uuid())
            .bind(owner.as_slice())
            .bind(&d_tag)
            .fetch_all(&mut *tx)
            .await?;
            related.extend(
                preserved
                    .into_iter()
                    .filter(|channel| Some(*channel) != home_channel),
            );
            if related.len() > RELATED_CHANNEL_CAP {
                return Err(DbError::InvalidData(
                    "owner Project update plus preserved related channels exceeds 64 channels"
                        .into(),
                ));
            }
        }

        let revision = match current_head {
            None => 1,
            Some(head) => head
                .try_get::<i64, _>("revision")?
                .checked_add(1)
                .ok_or_else(|| DbError::InvalidData("Project revision overflow".into()))?,
        };
        sqlx::query(
            "INSERT INTO project_state_heads \
               (community_id, project_owner, project_d_tag, revision, deleted, identity_event_id, last_event_id) \
             VALUES ($1,$2,$3,$4,FALSE,$5,$5) \
             ON CONFLICT (community_id, project_owner, project_d_tag) DO UPDATE SET \
               revision=EXCLUDED.revision, deleted=FALSE, identity_event_id=EXCLUDED.identity_event_id, \
               last_event_id=EXCLUDED.last_event_id, updated_at=transaction_timestamp()",
        )
        .bind(community_id.as_uuid())
        .bind(owner.as_slice())
        .bind(&d_tag)
        .bind(revision)
        .bind(event.id.as_bytes().as_slice())
        .execute(&mut *tx)
        .await?;
        replace_related_channels(&mut tx, community_id, owner.as_slice(), &d_tag, &related).await?;

        tx.commit().await?;
        Ok(ProjectLifecycleApplyResult {
            event: persisted.event,
            status: ProjectLifecycleStatus::Applied,
        })
    }

    /// Atomically store an owner-authorized NIP-09 coordinate deletion and,
    /// when it covers the live identity, advance Project state to a tombstone.
    pub async fn apply_project_deletion_event(
        &self,
        community_id: CommunityId,
        event: &Event,
        project_owner: &[u8],
        project_d_tag: &str,
        expected_identity_event_id: Option<&[u8]>,
    ) -> Result<ProjectLifecycleApplyResult> {
        if event_kind_u32(event) != KIND_DELETION
            || project_owner.len() != 32
            || project_d_tag.is_empty()
            || project_d_tag.len() > crate::event::D_TAG_MAX_LEN
            || expected_identity_event_id.is_some_and(|event_id| event_id.len() != 32)
        {
            return Err(DbError::InvalidData("invalid Project deletion".into()));
        }
        let deletion_created_at =
            DateTime::<Utc>::from_timestamp(event.created_at.as_secs() as i64, 0)
                .ok_or_else(|| DbError::InvalidTimestamp(event.created_at.as_secs() as i64))?;
        let mut tx = self.begin_transaction().await?;
        self.deletion_store()
            .guard_transaction(&mut tx, community_id)
            .await?;
        let lock = event_replacement_lock_key(
            community_id,
            KIND_PROJECT as i32,
            project_owner,
            Some(project_d_tag.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock)
            .execute(&mut *tx)
            .await?;
        let (stored_event, inserted) =
            insert_event_in_transaction(&mut tx, community_id, event, None).await?;
        if !inserted {
            tx.rollback().await?;
            return Ok(ProjectLifecycleApplyResult {
                event: stored_event,
                status: ProjectLifecycleStatus::Duplicate,
            });
        }

        let live = sqlx::query(
            "SELECT id, created_at FROM events WHERE community_id=$1 AND kind=$2 \
             AND pubkey=$3 AND d_tag=$4 AND deleted_at IS NULL \
             ORDER BY created_at DESC, id ASC LIMIT 1 FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(KIND_PROJECT as i32)
        .bind(project_owner)
        .bind(project_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(live) = live else {
            tx.commit().await?;
            return Ok(ProjectLifecycleApplyResult {
                event: stored_event,
                status: ProjectLifecycleStatus::NoEffect,
            });
        };
        let identity_event_id: Vec<u8> = live.try_get("id")?;
        let identity_created_at: DateTime<Utc> = live.try_get("created_at")?;
        if identity_created_at > deletion_created_at
            || expected_identity_event_id
                .is_some_and(|expected| expected != identity_event_id.as_slice())
        {
            tx.commit().await?;
            return Ok(ProjectLifecycleApplyResult {
                event: stored_event,
                status: ProjectLifecycleStatus::NoEffect,
            });
        }

        let head = sqlx::query(
            "SELECT revision, deleted, identity_event_id FROM project_state_heads \
             WHERE community_id=$1 AND project_owner=$2 AND project_d_tag=$3 FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(project_owner)
        .bind(project_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let revision = if let Some(head) = head {
            let materialized: Vec<u8> = head.try_get("identity_event_id")?;
            if head.try_get::<bool, _>("deleted")? || materialized != identity_event_id {
                return Err(DbError::InvalidData(
                    "Project lifecycle state does not match live identity".into(),
                ));
            }
            head.try_get::<i64, _>("revision")?
                .checked_add(1)
                .ok_or_else(|| DbError::InvalidData("Project revision overflow".into()))?
        } else {
            // Materialize the pre-existing identity at revision 1 before applying
            // its first relational lifecycle event.
            2
        };

        sqlx::query(
            "UPDATE events SET deleted_at=transaction_timestamp() WHERE community_id=$1 \
             AND id=$2 AND deleted_at IS NULL AND created_at <= $3",
        )
        .bind(community_id.as_uuid())
        .bind(&identity_event_id)
        .bind(deletion_created_at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO project_state_heads \
               (community_id, project_owner, project_d_tag, revision, deleted, identity_event_id, last_event_id) \
             VALUES ($1,$2,$3,$4,TRUE,$5,$6) \
             ON CONFLICT (community_id, project_owner, project_d_tag) DO UPDATE SET \
               revision=EXCLUDED.revision, deleted=TRUE, last_event_id=EXCLUDED.last_event_id, \
               updated_at=transaction_timestamp()",
        )
        .bind(community_id.as_uuid())
        .bind(project_owner)
        .bind(project_d_tag)
        .bind(revision)
        .bind(&identity_event_id)
        .bind(event.id.as_bytes().as_slice())
        .execute(&mut *tx)
        .await?;
        replace_related_channels(
            &mut tx,
            community_id,
            project_owner,
            project_d_tag,
            &BTreeSet::new(),
        )
        .await?;
        tx.commit().await?;
        Ok(ProjectLifecycleApplyResult {
            event: stored_event,
            status: ProjectLifecycleStatus::Applied,
        })
    }

    /// Load coherent Project states that require publication by `projection_pubkey`.
    ///
    /// A Project is pending when its relational revision has not been projected,
    /// or when a relay-key rotation requires the same revision to be republished.
    /// Every returned candidate is assembled while holding the Project coordinate
    /// lock; [`Self::commit_project_state_projection`] rejects it if state changes
    /// after this method returns.
    pub async fn load_pending_project_state_projections(
        &self,
        projection_pubkey: &[u8],
        limit: i64,
    ) -> Result<Vec<ProjectStateProjectionCandidate>> {
        if projection_pubkey.len() != 32 {
            return Err(DbError::InvalidData(
                "Project projection pubkey must be 32 bytes".into(),
            ));
        }
        if !(1..=1_000).contains(&limit) {
            return Err(DbError::InvalidData(
                "Project projection candidate limit must be between 1 and 1000".into(),
            ));
        }
        self.materialize_brownfield_project_identities(limit)
            .await?;
        let coordinates = sqlx::query(
            "SELECT head.community_id, head.project_owner, head.project_d_tag \
             FROM project_state_heads head JOIN communities community ON community.id=head.community_id \
             WHERE community.deletion_state='active' AND community.deleted_at IS NULL \
               AND (head.projected_revision < head.revision \
                    OR head.projection_pubkey IS DISTINCT FROM $1) \
             ORDER BY head.updated_at, head.community_id, head.project_owner, head.project_d_tag \
             LIMIT $2",
        )
        .bind(projection_pubkey)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut candidates = Vec::with_capacity(coordinates.len());
        for coordinate in coordinates {
            let community_id = CommunityId::from_uuid(coordinate.try_get("community_id")?);
            let project_owner: Vec<u8> = coordinate.try_get("project_owner")?;
            let project_d_tag: String = coordinate.try_get("project_d_tag")?;
            match self
                .load_pending_project_state_projection(
                    community_id,
                    &project_owner,
                    &project_d_tag,
                    projection_pubkey,
                )
                .await
            {
                Ok(Some(candidate)) => candidates.push(candidate),
                Ok(None) => {}
                Err(DbError::InvalidData(message)) => tracing::warn!(
                    %community_id,
                    project_owner = %hex::encode(&project_owner),
                    project_d_tag = %project_d_tag,
                    %message,
                    "skipping invalid Project projection candidate"
                ),
                Err(error) => return Err(error),
            }
        }
        Ok(candidates)
    }

    /// Adopt live Project identities created before relational state existed.
    ///
    /// Each identity is materialized at revision 1 without passing through event
    /// ingest again. Invalid historical identities are isolated so one bad row
    /// cannot prevent valid Projects from being repaired.
    async fn materialize_brownfield_project_identities(&self, limit: i64) -> Result<()> {
        let rows = sqlx::query(
            "SELECT event.community_id, event.id FROM events event \
             JOIN communities community ON community.id=event.community_id \
             LEFT JOIN project_state_heads head ON head.community_id=event.community_id \
               AND head.project_owner=event.pubkey AND head.project_d_tag=event.d_tag \
             WHERE event.kind=$1 AND event.deleted_at IS NULL AND event.d_tag IS NOT NULL \
               AND community.deletion_state='active' AND community.deleted_at IS NULL \
               AND head.community_id IS NULL \
             ORDER BY event.created_at, event.id LIMIT $2",
        )
        .bind(KIND_PROJECT as i32)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        for row in rows {
            let community_id = CommunityId::from_uuid(row.try_get("community_id")?);
            let event_id: Vec<u8> = row.try_get("id")?;
            let stored = match self.get_event_by_id(community_id, &event_id).await {
                Ok(Some(stored)) => stored,
                Ok(None) => continue,
                Err(DbError::InvalidData(message)) => {
                    tracing::warn!(
                        %community_id,
                        event_id = %hex::encode(&event_id),
                        %message,
                        "skipping invalid brownfield Project event"
                    );
                    continue;
                }
                Err(error) => return Err(error),
            };
            match self
                .materialize_brownfield_project_identity(community_id, &stored.event)
                .await
            {
                Ok(_) => {}
                Err(DbError::InvalidData(message)) => tracing::warn!(
                    %community_id,
                    event_id = %stored.event.id,
                    %message,
                    "skipping invalid brownfield Project identity"
                ),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Materialize one still-live pre-relational Project identity at revision 1.
    async fn materialize_brownfield_project_identity(
        &self,
        community_id: CommunityId,
        event: &Event,
    ) -> Result<bool> {
        if event_kind_u32(event) != KIND_PROJECT {
            return Err(DbError::InvalidData(
                "brownfield Project materialization requires kind 30621".into(),
            ));
        }
        let d_tag = crate::event::extract_d_tag(event).unwrap_or_default();
        if d_tag.is_empty() || d_tag.len() > crate::event::D_TAG_MAX_LEN {
            return Err(DbError::InvalidData("invalid Project d tag".into()));
        }
        let tags = serde_json::to_value(&event.tags)?;
        let (_, related) = parse_base_state(&tags).map_err(DbError::InvalidData)?;
        let owner = event.pubkey.to_bytes();

        let mut tx = self.begin_transaction().await?;
        self.deletion_store()
            .guard_transaction(&mut tx, community_id)
            .await?;
        let coordinate_lock = event_replacement_lock_key(
            community_id,
            KIND_PROJECT as i32,
            owner.as_slice(),
            Some(d_tag.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(coordinate_lock)
            .execute(&mut *tx)
            .await?;
        let inserted = materialize_project_head_in_transaction(
            &mut tx,
            community_id,
            owner.as_slice(),
            &d_tag,
            event.id.as_bytes(),
            &related,
        )
        .await?;
        if !inserted {
            tx.rollback().await?;
            return Ok(false);
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Authorize and atomically apply one v1 Project related-channel command.
    ///
    /// Lock order is the community deletion fence, Project coordinate, then
    /// current home-channel membership. Exact accepted-event replays are
    /// detected from immutable event history while holding the coordinate lock.
    pub async fn apply_project_related_channel_change(
        &self,
        community_id: CommunityId,
        event: &Event,
        change: ProjectRelatedChannelChange<'_>,
    ) -> Result<ProjectChangeApplyResult> {
        if event_kind_u32(event) != KIND_PROJECT_CHANGE {
            return Err(DbError::InvalidData(
                "Project change persistence requires kind 47010".into(),
            ));
        }
        if let Some(message) = validate_patch(change) {
            return Ok(ProjectChangeApplyResult::Invalid(message));
        }

        let mut tx = self.begin_transaction().await?;
        self.deletion_store()
            .guard_transaction(&mut tx, community_id)
            .await?;
        let coordinate_lock = event_replacement_lock_key(
            community_id,
            KIND_PROJECT as i32,
            change.project_owner,
            Some(change.project_d_tag.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(coordinate_lock)
            .execute(&mut *tx)
            .await?;

        let duplicate: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2 AND kind=$3)",
        )
        .bind(community_id.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .bind(KIND_PROJECT_CHANGE as i32)
        .fetch_one(&mut *tx)
        .await?;
        if duplicate {
            let applied_revision = change
                .expected_revision
                .checked_add(1)
                .ok_or_else(|| DbError::InvalidData("Project command revision overflow".into()))?;
            return Ok(ProjectChangeApplyResult::Duplicate { applied_revision });
        }

        let head = sqlx::query(
            "SELECT revision, deleted, identity_event_id FROM project_state_heads \
             WHERE community_id=$1 AND project_owner=$2 AND project_d_tag=$3 FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(change.project_owner)
        .bind(change.project_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        if head.as_ref().is_some_and(|row| row.get("deleted")) {
            return Ok(ProjectChangeApplyResult::ProjectDeleted);
        }

        let base = sqlx::query(
            "SELECT id, tags FROM events WHERE community_id=$1 AND kind=$2 AND pubkey=$3 \
             AND d_tag=$4 AND deleted_at IS NULL ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(community_id.as_uuid())
        .bind(KIND_PROJECT as i32)
        .bind(change.project_owner)
        .bind(change.project_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(base) = base else {
            return Ok(ProjectChangeApplyResult::ProjectNotFound);
        };
        let identity_event_id: Vec<u8> = base.try_get("id")?;
        let base_tags: serde_json::Value = base.try_get("tags")?;
        let (home_channel, base_related) = match parse_base_state(&base_tags) {
            Ok(state) => state,
            Err(message) => return Ok(ProjectChangeApplyResult::Invalid(message)),
        };

        let current_revision = if let Some(head) = head {
            let revision: i64 = head.try_get("revision")?;
            let materialized_identity: Vec<u8> = head.try_get("identity_event_id")?;
            if materialized_identity != identity_event_id {
                return Ok(ProjectChangeApplyResult::Conflict {
                    current_revision: revision,
                });
            }
            revision
        } else {
            let inserted = materialize_project_head_in_transaction(
                &mut tx,
                community_id,
                change.project_owner,
                change.project_d_tag,
                &identity_event_id,
                &base_related,
            )
            .await?;
            if !inserted {
                return Ok(ProjectChangeApplyResult::ProjectNotFound);
            }
            1
        };

        let actor = event.pubkey.to_bytes();
        if actor.as_slice() != change.project_owner {
            let Some(home_channel) = home_channel else {
                return Ok(ProjectChangeApplyResult::Forbidden);
            };
            crate::channel_members::acquire_channel_membership_lock(
                &mut tx,
                community_id,
                home_channel,
            )
            .await?;
            let authorized_role: Option<String> = sqlx::query_scalar(
                "SELECT member.role::text FROM channel_members member \
                 JOIN channels channel ON channel.community_id=member.community_id \
                   AND channel.id=member.channel_id \
                 WHERE member.community_id=$1 AND member.channel_id=$2 AND member.pubkey=$3 \
                   AND member.role IN ('owner', 'admin') AND member.removed_at IS NULL \
                   AND channel.archived_at IS NULL AND channel.deleted_at IS NULL \
                 LIMIT 1 FOR SHARE OF channel, member",
            )
            .bind(community_id.as_uuid())
            .bind(home_channel)
            .bind(actor.as_slice())
            .fetch_optional(&mut *tx)
            .await?;
            if authorized_role.is_none() {
                return Ok(ProjectChangeApplyResult::Forbidden);
            }
        }
        if change.expected_revision != current_revision {
            return Ok(ProjectChangeApplyResult::Conflict { current_revision });
        }

        let rows = sqlx::query_scalar::<_, Uuid>(
            "SELECT channel_id FROM project_related_channels WHERE community_id=$1 \
             AND project_owner=$2 AND project_d_tag=$3 ORDER BY channel_id",
        )
        .bind(community_id.as_uuid())
        .bind(change.project_owner)
        .bind(change.project_d_tag)
        .fetch_all(&mut *tx)
        .await?;
        let mut related = rows.into_iter().collect::<BTreeSet<_>>();
        for channel in change.add {
            if Some(*channel) == home_channel || !related.insert(*channel) {
                return Ok(ProjectChangeApplyResult::Invalid(
                    "cannot add the home channel or an already-related channel".into(),
                ));
            }
        }
        for channel in change.remove {
            if !related.remove(channel) {
                return Ok(ProjectChangeApplyResult::Invalid(
                    "cannot remove a channel that is not related".into(),
                ));
            }
        }
        if related.len() > RELATED_CHANNEL_CAP {
            return Ok(ProjectChangeApplyResult::Invalid(
                "effective Project state exceeds 64 related channels".into(),
            ));
        }

        let (_, inserted) = insert_event_in_transaction(&mut tx, community_id, event, None).await?;
        if !inserted {
            return Err(DbError::InvalidData(
                "Project command appeared concurrently outside its coordinate lock".into(),
            ));
        }
        replace_related_channels(
            &mut tx,
            community_id,
            change.project_owner,
            change.project_d_tag,
            &related,
        )
        .await?;
        let revision = current_revision
            .checked_add(1)
            .ok_or_else(|| DbError::InvalidData("Project revision overflow".into()))?;
        let updated = sqlx::query(
            "UPDATE project_state_heads SET revision=$4, last_event_id=$5, \
             updated_at=transaction_timestamp() WHERE community_id=$1 AND project_owner=$2 \
             AND project_d_tag=$3 AND revision=$6",
        )
        .bind(community_id.as_uuid())
        .bind(change.project_owner)
        .bind(change.project_d_tag)
        .bind(revision)
        .bind(event.id.as_bytes().as_slice())
        .bind(current_revision)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated != 1 {
            return Err(DbError::InvalidData(
                "Project CAS failed under coordinate lock".into(),
            ));
        }
        tx.commit().await?;
        Ok(ProjectChangeApplyResult::Applied { revision })
    }

    /// Load one coherent Project state when it requires publication by
    /// `projection_pubkey`.
    ///
    /// The returned candidate is safe to sign outside the transaction because
    /// [`Self::commit_project_state_projection`] revalidates every observed
    /// head field while holding the same Project coordinate lock.
    pub async fn load_pending_project_state_projection(
        &self,
        community_id: CommunityId,
        project_owner: &[u8],
        project_d_tag: &str,
        projection_pubkey: &[u8],
    ) -> Result<Option<ProjectStateProjectionCandidate>> {
        let mut tx = self.begin_transaction().await?;
        let coordinate_lock = event_replacement_lock_key(
            community_id,
            KIND_PROJECT as i32,
            project_owner,
            Some(project_d_tag.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(coordinate_lock)
            .execute(&mut *tx)
            .await?;
        let head = sqlx::query(
            "SELECT revision, projected_revision, projection_pubkey, deleted, \
                    identity_event_id, last_event_id \
             FROM project_state_heads WHERE community_id=$1 AND project_owner=$2 \
               AND project_d_tag=$3 FOR SHARE",
        )
        .bind(community_id.as_uuid())
        .bind(project_owner)
        .bind(project_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(head) = head else {
            return Ok(None);
        };
        let revision: i64 = head.try_get("revision")?;
        let projected_revision: i64 = head.try_get("projected_revision")?;
        let observed_projection_pubkey: Option<Vec<u8>> = head.try_get("projection_pubkey")?;
        if projected_revision == revision
            && observed_projection_pubkey.as_deref() == Some(projection_pubkey)
        {
            return Ok(None);
        }
        let identity_event_id: Vec<u8> = head.try_get("identity_event_id")?;
        let change_event_id: Vec<u8> = head.try_get("last_event_id")?;
        let deleted: bool = head.try_get("deleted")?;
        let identity_row = sqlx::query(
            "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
             FROM events WHERE community_id=$1 AND id=$2 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(community_id.as_uuid())
        .bind(&identity_event_id)
        .fetch_optional(&mut *tx)
        .await?;
        let identity_event = match identity_row {
            Some(row) => {
                crate::event::row_to_stored_event(row)?
                    .ok_or_else(|| DbError::InvalidData("invalid Project identity event".into()))?
                    .event
            }
            None => {
                return Err(DbError::InvalidData(
                    "Project state references a missing identity event".into(),
                ))
            }
        };
        let related_channels = sqlx::query_scalar::<_, Uuid>(
            "SELECT channel_id FROM project_related_channels WHERE community_id=$1 \
               AND project_owner=$2 AND project_d_tag=$3 ORDER BY channel_id",
        )
        .bind(community_id.as_uuid())
        .bind(project_owner)
        .bind(project_d_tag)
        .fetch_all(&mut *tx)
        .await?;
        let change_id = EventId::from_hex(&hex::encode(&change_event_id))
            .map_err(|error| DbError::InvalidData(format!("invalid Project change id: {error}")))?;
        let coordinate = format!("30621:{}:{project_d_tag}", hex::encode(project_owner));
        let template = project_state_template(ProjectStateProjectionInput {
            coordinate: &coordinate,
            revision,
            identity_event: &identity_event,
            change_event_id: &change_id,
            deleted,
            related_channels: &related_channels,
        })
        .map_err(|error| DbError::InvalidData(error.to_string()))?;
        let projection_d_tag = projection_d_tag(&template)?;
        let previous_created_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
            "SELECT created_at FROM events WHERE community_id=$1 AND kind=$2 AND pubkey=$3 \
               AND d_tag=$4 AND deleted_at IS NULL ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(community_id.as_uuid())
        .bind(KIND_PROJECT_STATE as i32)
        .bind(projection_pubkey)
        .bind(projection_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let previous_created_at = previous_created_at
            .map(|value| {
                u64::try_from(value.timestamp()).map_err(|_| {
                    DbError::InvalidData("Project projection has a negative timestamp".into())
                })
            })
            .transpose()?;
        tx.commit().await?;
        Ok(Some(ProjectStateProjectionCandidate {
            community_id,
            template,
            previous_created_at,
            project_owner: project_owner.to_vec(),
            project_d_tag: project_d_tag.to_owned(),
            revision,
            identity_event_id,
            change_event_id,
            observed_projected_revision: projected_revision,
            observed_projection_pubkey,
            projection_pubkey: projection_pubkey.to_vec(),
        }))
    }

    /// Atomically publish a relay-signed projection and advance its retry marker.
    ///
    /// The candidate must be passed back unchanged. The Project coordinate is
    /// locked before the projection replacement coordinate, and every observed
    /// head field is revalidated before the event is stored.
    pub async fn commit_project_state_projection(
        &self,
        candidate: &ProjectStateProjectionCandidate,
        event: &Event,
    ) -> Result<ProjectStateProjectionCommitResult> {
        event.verify().map_err(|error| {
            DbError::InvalidData(format!("invalid signed Project projection: {error}"))
        })?;
        if event_kind_u32(event) != KIND_PROJECT_STATE
            || event.pubkey.to_bytes().as_slice() != candidate.projection_pubkey
            || event.tags.as_slice() != candidate.template.tags
            || event.content != candidate.template.content
        {
            return Err(DbError::InvalidData(
                "signed Project projection does not match its candidate".into(),
            ));
        }
        if candidate
            .previous_created_at
            .is_some_and(|previous| event.created_at.as_secs() <= previous)
        {
            return Err(DbError::InvalidData(
                "Project projection timestamp must advance the live projection".into(),
            ));
        }
        let projection_d_tag = projection_d_tag(&candidate.template)?;
        let mut tx = self.begin_transaction().await?;
        self.deletion_store()
            .guard_transaction(&mut tx, candidate.community_id)
            .await?;
        let coordinate_lock = event_replacement_lock_key(
            candidate.community_id,
            KIND_PROJECT as i32,
            &candidate.project_owner,
            Some(candidate.project_d_tag.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(coordinate_lock)
            .execute(&mut *tx)
            .await?;
        let head = sqlx::query(
            "SELECT revision, projected_revision, projection_pubkey, identity_event_id, \
                    last_event_id FROM project_state_heads WHERE community_id=$1 \
               AND project_owner=$2 AND project_d_tag=$3 FOR UPDATE",
        )
        .bind(candidate.community_id.as_uuid())
        .bind(&candidate.project_owner)
        .bind(&candidate.project_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(head) = head else {
            return Ok(ProjectStateProjectionCommitResult::Stale);
        };
        let projection_pubkey: Option<Vec<u8>> = head.try_get("projection_pubkey")?;
        let matches_candidate = head.try_get::<i64, _>("revision")? == candidate.revision
            && head.try_get::<i64, _>("projected_revision")?
                == candidate.observed_projected_revision
            && projection_pubkey == candidate.observed_projection_pubkey
            && head.try_get::<Vec<u8>, _>("identity_event_id")? == candidate.identity_event_id
            && head.try_get::<Vec<u8>, _>("last_event_id")? == candidate.change_event_id;
        if !matches_candidate {
            return Ok(ProjectStateProjectionCommitResult::Stale);
        }
        let replaced = self
            .replace_parameterized_event_in_transaction(
                &mut tx,
                candidate.community_id,
                event,
                projection_d_tag,
                None,
                ParameterizedReplacePrecondition::Unconditional,
            )
            .await?;
        if replaced.status != ParameterizedReplaceStatus::Inserted {
            tx.rollback().await?;
            return Ok(ProjectStateProjectionCommitResult::Stale);
        }
        sqlx::query(
            "UPDATE project_state_heads SET projected_revision=$4, projection_pubkey=$5, \
               updated_at=transaction_timestamp() WHERE community_id=$1 AND project_owner=$2 \
               AND project_d_tag=$3",
        )
        .bind(candidate.community_id.as_uuid())
        .bind(&candidate.project_owner)
        .bind(&candidate.project_d_tag)
        .bind(candidate.revision)
        .bind(&candidate.projection_pubkey)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(ProjectStateProjectionCommitResult::Committed)
    }
}

fn projection_d_tag(template: &ProjectStateTemplate) -> Result<&str> {
    template
        .tags
        .iter()
        .find_map(|tag| match tag.as_slice() {
            [name, value] if name == "d" => Some(value.as_str()),
            _ => None,
        })
        .ok_or_else(|| DbError::InvalidData("Project projection template has no d tag".into()))
}

#[cfg(test)]
mod postgres_tests {
    use buzz_core::channel::{ChannelType, ChannelVisibility};
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use sqlx::postgres::PgPoolOptions;
    use sqlx::PgPool;

    use super::*;

    fn projection(
        candidate: &ProjectStateProjectionCandidate,
        relay: &Keys,
        created_at: u64,
    ) -> Event {
        EventBuilder::new(
            candidate.template().kind,
            candidate.template().content.clone(),
        )
        .tags(candidate.template().tags.clone())
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(relay)
        .expect("sign projection")
    }

    async fn head(pool: &PgPool, community: CommunityId, owner: &[u8], d_tag: &str) -> (i64, bool) {
        sqlx::query_as(
            "SELECT revision, deleted FROM project_state_heads \
             WHERE community_id=$1 AND project_owner=$2 AND project_d_tag=$3",
        )
        .bind(community.as_uuid())
        .bind(owner)
        .bind(d_tag)
        .fetch_one(pool)
        .await
        .expect("load Project head")
    }

    async fn create_channel(pool: &PgPool, community: CommunityId, channel: Uuid, owner: &Keys) {
        crate::channel::create_channel_with_id(
            pool,
            community,
            channel,
            &format!("project-state-{channel}"),
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            owner.public_key().to_bytes().as_slice(),
            None,
        )
        .await
        .expect("create test channel");
    }

    async fn set_role(
        pool: &PgPool,
        community: CommunityId,
        channel: Uuid,
        user: &Keys,
        role: &str,
    ) {
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by) \
             VALUES ($1,$2,$3,$4::member_role,$3) ON CONFLICT \
             (community_id, channel_id, pubkey) DO UPDATE SET role=EXCLUDED.role, \
             removed_at=NULL, removed_by=NULL",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(user.public_key().to_bytes().as_slice())
        .bind(role)
        .execute(pool)
        .await
        .expect("set channel role");
    }

    fn project_identity(owner: &Keys, d_tag: &str, home: Option<Uuid>, created_at: u64) -> Event {
        let mut tags = vec![Tag::parse(["d", d_tag]).expect("d tag")];
        if let Some(home) = home {
            tags.push(Tag::parse(["buzz-channel", &home.to_string()]).expect("home tag"));
        }
        EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(owner)
            .expect("sign Project identity")
    }

    fn project_change(
        actor: &Keys,
        owner: &Keys,
        d_tag: &str,
        revision: i64,
        add: &[Uuid],
        remove: &[Uuid],
        created_at: u64,
    ) -> Event {
        let coordinate = format!("{KIND_PROJECT}:{}:{d_tag}", owner.public_key().to_hex());
        EventBuilder::new(
            Kind::Custom(KIND_PROJECT_CHANGE as u16),
            serde_json::json!({
                "v": 1,
                "add_related_channels": add,
                "remove_related_channels": remove,
            })
            .to_string(),
        )
        .tags([
            Tag::parse(["a", &coordinate]).expect("coordinate tag"),
            Tag::parse(["r", &revision.to_string()]).expect("revision tag"),
        ])
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(actor)
        .expect("sign Project change")
    }

    fn change<'a>(
        owner: &'a [u8],
        d_tag: &'a str,
        revision: i64,
        add: &'a [Uuid],
        remove: &'a [Uuid],
    ) -> ProjectRelatedChannelChange<'a> {
        ProjectRelatedChannelChange {
            project_owner: owner,
            project_d_tag: d_tag,
            expected_revision: revision,
            add,
            remove,
        }
    }

    async fn event_exists(pool: &PgPool, community: CommunityId, event: &Event) -> bool {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2)")
            .bind(community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .fetch_one(pool)
            .await
            .expect("query event")
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn lifecycle_materializes_brownfield_state_and_preserves_monotonic_revisions() {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&crate::test_support::database_url())
            .await
            .expect("connect test database");
        let db = Db::from_pool(pool.clone());
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("project-state-{}.example", community.as_uuid()))
            .execute(&pool)
            .await
            .expect("insert community");

        let owner = Keys::generate();
        let owner_bytes = owner.public_key().to_bytes();
        let seeded = Uuid::new_v4();
        let identity_time = Utc::now().timestamp() as u64 - 100;
        let malformed_owner = Keys::generate();
        let malformed = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags([
                Tag::parse(["d", "broken"]).expect("d tag"),
                Tag::parse(["buzz-related-channel", "not-a-uuid"]).expect("related tag"),
            ])
            .custom_created_at(Timestamp::from(identity_time - 1))
            .sign_with_keys(&malformed_owner)
            .expect("sign malformed brownfield Project");
        db.replace_parameterized_event(community, &malformed, "broken", None)
            .await
            .expect("store malformed pre-feature Project");
        let base = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags([
                Tag::parse(["d", "shared"]).expect("d tag"),
                Tag::parse(["buzz-related-channel", &seeded.to_string()]).expect("related tag"),
            ])
            .custom_created_at(Timestamp::from(identity_time))
            .sign_with_keys(&owner)
            .expect("sign brownfield Project");
        db.replace_parameterized_event(community, &base, "shared", None)
            .await
            .expect("store pre-feature Project");

        let relay = Keys::generate();
        let relay_bytes = relay.public_key().to_bytes();
        let candidate = db
            .load_pending_project_state_projections(relay_bytes.as_slice(), 10)
            .await
            .expect("materialize brownfield Project")
            .pop()
            .expect("brownfield projection is pending");
        assert_eq!(
            head(&pool, community, owner_bytes.as_slice(), "shared").await,
            (1, false)
        );
        assert!(candidate.template().content.contains(&seeded.to_string()));

        let projected_at = Timestamp::now().as_secs();
        let old_projection = projection(&candidate, &relay, projected_at);
        assert_eq!(
            db.commit_project_state_projection(&candidate, &old_projection)
                .await
                .expect("commit brownfield projection"),
            ProjectStateProjectionCommitResult::Committed
        );
        let invalid_head_owner = Keys::generate();
        sqlx::query(
            "INSERT INTO project_state_heads \
               (community_id, project_owner, project_d_tag, revision, identity_event_id, last_event_id) \
             VALUES ($1,$2,'broken',1,$3,$3)",
        )
        .bind(community.as_uuid())
        .bind(invalid_head_owner.public_key().to_bytes().as_slice())
        .bind(malformed.id.as_bytes().as_slice())
        .execute(&pool)
        .await
        .expect("materialize malformed projection candidate");

        let rotated_relay = Keys::generate();
        let rotated_relay_bytes = rotated_relay.public_key().to_bytes();
        let rotation = db
            .load_pending_project_state_projections(rotated_relay_bytes.as_slice(), 10)
            .await
            .expect("load relay-key rotation")
            .pop()
            .expect("relay-key rotation is pending");
        assert_eq!(rotation.previous_created_at(), None);
        assert_eq!(
            db.commit_project_state_projection(
                &rotation,
                &projection(&rotation, &rotated_relay, projected_at),
            )
            .await
            .expect("commit relay-key rotation"),
            ProjectStateProjectionCommitResult::Committed
        );
        assert!(db
            .load_pending_project_state_projections(rotated_relay_bytes.as_slice(), 10)
            .await
            .expect("check rotated relay is current")
            .is_empty());
        let stale_after_recovery = db
            .load_pending_project_state_projections(relay_bytes.as_slice(), 10)
            .await
            .expect("load rotation back to original relay")
            .pop()
            .expect("original relay is pending again");
        assert_eq!(
            stale_after_recovery.previous_created_at(),
            Some(projected_at)
        );

        let recovered = Uuid::new_v4();
        let recovery = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags([
                Tag::parse(["d", "shared"]).expect("d tag"),
                Tag::parse(["buzz-related-channel", &recovered.to_string()]).expect("related tag"),
            ])
            .custom_created_at(Timestamp::from(identity_time + 1))
            .sign_with_keys(&owner)
            .expect("sign recovery");
        assert_eq!(
            db.apply_project_identity_event(community, &recovery)
                .await
                .expect("apply recovery")
                .status,
            ProjectLifecycleStatus::Applied
        );
        assert_eq!(
            db.commit_project_state_projection(
                &stale_after_recovery,
                &projection(&stale_after_recovery, &relay, projected_at + 1),
            )
            .await
            .expect("reject candidate made stale by recovery"),
            ProjectStateProjectionCommitResult::Stale
        );
        assert_eq!(
            head(&pool, community, owner_bytes.as_slice(), "shared").await,
            (2, false)
        );

        let pending = db
            .load_pending_project_state_projections(relay_bytes.as_slice(), 10)
            .await
            .expect("load recovery projection")
            .pop()
            .expect("recovery projection is pending");
        assert_eq!(pending.previous_created_at(), Some(projected_at));
        assert!(pending.template().content.contains(&recovered.to_string()));
        assert!(pending.template().content.contains(&seeded.to_string()));

        let coordinate = format!("30621:{}:shared", owner.public_key().to_hex());
        let deletion = EventBuilder::new(Kind::EventDeletion, "")
            .tag(Tag::parse(["a", &coordinate]).expect("coordinate tag"))
            .custom_created_at(Timestamp::from(identity_time + 2))
            .sign_with_keys(&owner)
            .expect("sign deletion");
        assert_eq!(
            db.apply_project_deletion_event(
                community,
                &deletion,
                owner_bytes.as_slice(),
                "shared",
                None,
            )
            .await
            .expect("delete Project")
            .status,
            ProjectLifecycleStatus::Applied
        );
        assert_eq!(
            head(&pool, community, owner_bytes.as_slice(), "shared").await,
            (3, true)
        );

        let same_second = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags([Tag::parse(["d", "shared"]).expect("d tag")])
            .custom_created_at(Timestamp::from(identity_time + 2))
            .sign_with_keys(&owner)
            .expect("sign same-second recreation");
        assert_eq!(
            db.apply_project_identity_event(community, &same_second)
                .await
                .expect("reject same-second recreation")
                .status,
            ProjectLifecycleStatus::Superseded
        );

        let recreation = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags([Tag::parse(["d", "shared"]).expect("d tag")])
            .custom_created_at(Timestamp::from(identity_time + 3))
            .sign_with_keys(&owner)
            .expect("sign recreation");
        assert_eq!(
            db.apply_project_identity_event(community, &recreation)
                .await
                .expect("recreate Project")
                .status,
            ProjectLifecycleStatus::Applied
        );
        assert_eq!(
            head(&pool, community, owner_bytes.as_slice(), "shared").await,
            (4, false)
        );

        let exact_deletion = EventBuilder::new(Kind::EventDeletion, "")
            .tag(Tag::event(recreation.id))
            .custom_created_at(Timestamp::from(identity_time + 4))
            .sign_with_keys(&owner)
            .expect("sign exact deletion");
        assert_eq!(
            db.apply_project_deletion_event(
                community,
                &exact_deletion,
                owner_bytes.as_slice(),
                "shared",
                Some(recreation.id.as_bytes()),
            )
            .await
            .expect("delete recreated Project")
            .status,
            ProjectLifecycleStatus::Applied
        );
        assert_eq!(
            head(&pool, community, owner_bytes.as_slice(), "shared").await,
            (5, true)
        );

        pool.close().await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn related_channel_changes_replay_cas_preserve_owner_updates_and_reconcile() {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(&crate::test_support::database_url())
            .await
            .expect("connect test database");
        let db = Db::from_pool(pool.clone());
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("project-change-{}.example", community.as_uuid()))
            .execute(&pool)
            .await
            .expect("insert community");

        let owner = Keys::generate();
        let home_owner = Keys::generate();
        let admin = Keys::generate();
        let home = Uuid::new_v4();
        create_channel(&pool, community, home, &home_owner).await;
        set_role(&pool, community, home, &admin, "admin").await;
        let related = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        for channel in related {
            create_channel(&pool, community, channel, &home_owner).await;
        }
        let owner_bytes = owner.public_key().to_bytes();
        let started_at = Utc::now().timestamp() as u64 - 100;
        db.apply_project_identity_event(
            community,
            &project_identity(&owner, "shared", Some(home), started_at),
        )
        .await
        .expect("apply Project identity");

        let first = project_change(
            &admin,
            &owner,
            "shared",
            1,
            &related[..1],
            &[],
            started_at + 1,
        );
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &first,
                change(owner_bytes.as_slice(), "shared", 1, &related[..1], &[]),
            )
            .await
            .expect("admin change"),
            ProjectChangeApplyResult::Applied { revision: 2 }
        );
        let second = project_change(
            &owner,
            &owner,
            "shared",
            2,
            &related[1..2],
            &[],
            started_at + 2,
        );
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &second,
                change(owner_bytes.as_slice(), "shared", 2, &related[1..2], &[]),
            )
            .await
            .expect("owner change"),
            ProjectChangeApplyResult::Applied { revision: 3 }
        );
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &first,
                change(owner_bytes.as_slice(), "shared", 1, &related[..1], &[]),
            )
            .await
            .expect("replay after later revision"),
            ProjectChangeApplyResult::Duplicate {
                applied_revision: 2
            }
        );
        sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(first.id.as_bytes().as_slice())
            .execute(&pool)
            .await
            .expect("soft-delete command row");
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &first,
                change(owner_bytes.as_slice(), "shared", 1, &related[..1], &[]),
            )
            .await
            .expect("replay soft-deleted command"),
            ProjectChangeApplyResult::Duplicate {
                applied_revision: 2
            }
        );

        let stale = project_change(
            &admin,
            &owner,
            "shared",
            1,
            &related[2..],
            &[],
            started_at + 3,
        );
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &stale,
                change(owner_bytes.as_slice(), "shared", 1, &related[2..], &[]),
            )
            .await
            .expect("reject stale command"),
            ProjectChangeApplyResult::Conflict {
                current_revision: 3
            }
        );
        assert!(!event_exists(&pool, community, &stale).await);
        let resigned = project_change(
            &admin,
            &owner,
            "shared",
            3,
            &related[2..],
            &[],
            started_at + 4,
        );
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &resigned,
                change(owner_bytes.as_slice(), "shared", 3, &related[2..], &[]),
            )
            .await
            .expect("apply re-signed command"),
            ProjectChangeApplyResult::Applied { revision: 4 }
        );

        let replacement = project_identity(&owner, "shared", Some(home), started_at + 10);
        assert_eq!(
            db.apply_project_identity_event(community, &replacement)
                .await
                .expect("replace owner identity")
                .status,
            ProjectLifecycleStatus::Applied
        );
        let mut stale_tags = vec![Tag::parse(["d", "shared"]).expect("d tag")];
        stale_tags.extend((0..62).map(|_| {
            Tag::parse(["buzz-related-channel", &Uuid::new_v4().to_string()]).expect("related tag")
        }));
        let stale_identity = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags(stale_tags)
            .custom_created_at(Timestamp::from(started_at + 9))
            .sign_with_keys(&owner)
            .expect("sign stale identity");
        assert_eq!(
            db.apply_project_identity_event(community, &stale_identity)
                .await
                .expect("stale identity remains superseded despite combined cap")
                .status,
            ProjectLifecycleStatus::Superseded
        );
        let effective: Vec<Uuid> = sqlx::query_scalar(
            "SELECT channel_id FROM project_related_channels WHERE community_id=$1 \
             AND project_owner=$2 AND project_d_tag='shared' ORDER BY channel_id",
        )
        .bind(community.as_uuid())
        .bind(owner_bytes.as_slice())
        .fetch_all(&pool)
        .await
        .expect("load preserved channels");
        assert_eq!(
            effective.into_iter().collect::<BTreeSet<_>>(),
            related.into()
        );

        let relay = Keys::generate();
        let relay_bytes = relay.public_key().to_bytes();
        let candidate = db
            .load_pending_project_state_projections(relay_bytes.as_slice(), 100)
            .await
            .expect("load pending projections")
            .into_iter()
            .find(|candidate| {
                candidate.community_id == community && candidate.project_d_tag == "shared"
            })
            .expect("missed publish remains reconcilable");
        assert_eq!(candidate.revision, 5);
        assert_eq!(
            db.commit_project_state_projection(
                &candidate,
                &projection(&candidate, &relay, Timestamp::now().as_secs()),
            )
            .await
            .expect("commit reconciled projection"),
            ProjectStateProjectionCommitResult::Committed
        );
        assert_eq!(
            head(&pool, community, owner_bytes.as_slice(), "shared").await,
            (5, false)
        );
        pool.close().await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn related_channel_change_authorization_and_concurrent_cas_are_exact() {
        let pool = PgPoolOptions::new()
            .max_connections(12)
            .connect(&crate::test_support::database_url())
            .await
            .expect("connect test database");
        let db = Db::from_pool(pool.clone());
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("project-auth-{}.example", community.as_uuid()))
            .execute(&pool)
            .await
            .expect("insert community");
        let owner = Keys::generate();
        let channel_owner = Keys::generate();
        let admin = Keys::generate();
        let member = Keys::generate();
        let home = Uuid::new_v4();
        create_channel(&pool, community, home, &channel_owner).await;
        set_role(&pool, community, home, &admin, "admin").await;
        set_role(&pool, community, home, &member, "member").await;
        let related = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        for channel in related {
            create_channel(&pool, community, channel, &channel_owner).await;
        }
        let owner_bytes = owner.public_key().to_bytes();
        let now = Utc::now().timestamp() as u64 - 100;
        db.apply_project_identity_event(
            community,
            &project_identity(&owner, "auth", Some(home), now),
        )
        .await
        .expect("apply Project identity");

        for (actor, label) in [(&member, "member"), (&admin, "removed admin")] {
            if label == "removed admin" {
                sqlx::query(
                    "UPDATE channel_members SET removed_at=now() WHERE community_id=$1 \
                     AND channel_id=$2 AND pubkey=$3",
                )
                .bind(community.as_uuid())
                .bind(home)
                .bind(admin.public_key().to_bytes().as_slice())
                .execute(&pool)
                .await
                .expect("remove admin");
            }
            let event = project_change(actor, &owner, "auth", 1, &related[..1], &[], now + 1);
            assert_eq!(
                db.apply_project_related_channel_change(
                    community,
                    &event,
                    change(owner_bytes.as_slice(), "auth", 1, &related[..1], &[]),
                )
                .await
                .unwrap_or_else(|error| panic!("{label} check failed: {error}")),
                ProjectChangeApplyResult::Forbidden
            );
            assert!(!event_exists(&pool, community, &event).await);
        }
        set_role(&pool, community, home, &admin, "admin").await;
        for (disable, restore) in [
            (
                "UPDATE channels SET archived_at=now() WHERE community_id=$1 AND id=$2",
                "UPDATE channels SET archived_at=NULL WHERE community_id=$1 AND id=$2",
            ),
            (
                "UPDATE channels SET deleted_at=now() WHERE community_id=$1 AND id=$2",
                "UPDATE channels SET deleted_at=NULL WHERE community_id=$1 AND id=$2",
            ),
        ] {
            sqlx::query(disable)
                .bind(community.as_uuid())
                .bind(home)
                .execute(&pool)
                .await
                .expect("disable home channel");
            let event = project_change(&admin, &owner, "auth", 1, &related[..1], &[], now + 2);
            assert_eq!(
                db.apply_project_related_channel_change(
                    community,
                    &event,
                    change(owner_bytes.as_slice(), "auth", 1, &related[..1], &[]),
                )
                .await
                .expect("reject inactive home channel"),
                ProjectChangeApplyResult::Forbidden
            );
            sqlx::query(restore)
                .bind(community.as_uuid())
                .bind(home)
                .execute(&pool)
                .await
                .expect("restore home channel");
        }

        let exact = project_change(&admin, &owner, "auth", 1, &related[..1], &[], now + 3);
        let patch = change(owner_bytes.as_slice(), "auth", 1, &related[..1], &[]);
        let (left, right) = tokio::join!(
            db.apply_project_related_channel_change(community, &exact, patch),
            db.apply_project_related_channel_change(community, &exact, patch),
        );
        let outcomes = [
            left.expect("first duplicate race"),
            right.expect("second duplicate race"),
        ];
        assert!(outcomes.contains(&ProjectChangeApplyResult::Applied { revision: 2 }));
        assert!(outcomes.contains(&ProjectChangeApplyResult::Duplicate {
            applied_revision: 2
        }));

        let left_event = project_change(&admin, &owner, "auth", 2, &related[1..2], &[], now + 4);
        let right_event = project_change(&admin, &owner, "auth", 2, &related[2..], &[], now + 5);
        let (left, right) = tokio::join!(
            db.apply_project_related_channel_change(
                community,
                &left_event,
                change(owner_bytes.as_slice(), "auth", 2, &related[1..2], &[]),
            ),
            db.apply_project_related_channel_change(
                community,
                &right_event,
                change(owner_bytes.as_slice(), "auth", 2, &related[2..], &[]),
            ),
        );
        let left = left.expect("first CAS race");
        let right = right.expect("second CAS race");
        let outcomes = [left.clone(), right.clone()];
        assert!(outcomes.contains(&ProjectChangeApplyResult::Applied { revision: 3 }));
        assert!(outcomes.contains(&ProjectChangeApplyResult::Conflict {
            current_revision: 3
        }));
        assert_eq!(
            event_exists(&pool, community, &left_event).await,
            matches!(left, ProjectChangeApplyResult::Applied { .. })
        );
        assert_eq!(
            event_exists(&pool, community, &right_event).await,
            matches!(right, ProjectChangeApplyResult::Applied { .. })
        );
        let stored_channels = sqlx::query_scalar::<_, Uuid>(
            "SELECT channel_id FROM project_related_channels WHERE community_id=$1 \
             AND project_owner=$2 AND project_d_tag='auth' ORDER BY channel_id",
        )
        .bind(community.as_uuid())
        .bind(owner_bytes.as_slice())
        .fetch_all(&pool)
        .await
        .expect("load channels after CAS race");
        assert_eq!(stored_channels.len(), 2);
        assert!(stored_channels.contains(&related[0]));
        assert_eq!(
            stored_channels.contains(&related[1]),
            matches!(left, ProjectChangeApplyResult::Applied { .. })
        );
        assert_eq!(
            stored_channels.contains(&related[2]),
            matches!(right, ProjectChangeApplyResult::Applied { .. })
        );

        let no_home = project_identity(&owner, "no-home", None, now + 6);
        db.apply_project_identity_event(community, &no_home)
            .await
            .expect("apply Project without home");
        let denied = project_change(&admin, &owner, "no-home", 1, &related[..1], &[], now + 7);
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &denied,
                change(owner_bytes.as_slice(), "no-home", 1, &related[..1], &[]),
            )
            .await
            .expect("deny admin without home"),
            ProjectChangeApplyResult::Forbidden
        );
        let allowed = project_change(&owner, &owner, "no-home", 1, &related[..1], &[], now + 8);
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &allowed,
                change(owner_bytes.as_slice(), "no-home", 1, &related[..1], &[]),
            )
            .await
            .expect("owner bypasses home requirement"),
            ProjectChangeApplyResult::Applied { revision: 2 }
        );

        db.apply_project_identity_event(
            community,
            &project_identity(&owner, "deleted", Some(home), now + 9),
        )
        .await
        .expect("apply Project to delete");
        let coordinate = format!("{KIND_PROJECT}:{}:deleted", owner.public_key().to_hex());
        let deletion = EventBuilder::new(Kind::EventDeletion, "")
            .tag(Tag::parse(["a", &coordinate]).expect("coordinate tag"))
            .custom_created_at(Timestamp::from(now + 10))
            .sign_with_keys(&owner)
            .expect("sign Project deletion");
        db.apply_project_deletion_event(
            community,
            &deletion,
            owner_bytes.as_slice(),
            "deleted",
            None,
        )
        .await
        .expect("delete Project");
        let denied = project_change(&admin, &owner, "deleted", 2, &related[1..2], &[], now + 11);
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &denied,
                change(owner_bytes.as_slice(), "deleted", 2, &related[1..2], &[],),
            )
            .await
            .expect("deny deleted Project mutation"),
            ProjectChangeApplyResult::ProjectDeleted
        );
        pool.close().await;
    }
}
