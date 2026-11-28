//! The governed Git land: the one transaction in which a push takes effect
//! (`docs/spec/GitGovernanceLand.tla`, protocol "row").
//!
//! ```text
//!   hold      community shared ─► repo row KEY SHARE ─► announcement key ─►
//!             every key a revoking writer takes, shared ─► the authority,
//!             read in this transaction
//!   decide    (the caller's pure evaluator)
//!   swap      UPDATE manifest WHERE it is still the one read
//!             AND clock_timestamp() is before the authority's expiry
//!   receipt   (the caller's audit entry) ─► commit
//! ```
//!
//! No authority write can commit between `hold` and the commit: each one
//! takes, exclusively, a lock `hold` holds shared. Lands share all of them,
//! so they do not wait on each other to hold or decide. They do wait twice:
//! on the row, where a second swap waits for the first to commit and then
//! misses, and on the receipt, whose append takes the community's audit
//! chain exclusively until commit. So a community's lands, refusals and
//! governance writes commit one at a time. The audit lock is always taken
//! last, so it adds no cycle.
//!
//! The repository's `git_repo_names.manifest` is the committed state; the
//! object-store pointer is a cache written after commit.
use crate::{AdmittedTx, Db, Result};
use buzz_core::governance::RepoCoordinate;
use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::Row as _;
use uuid::Uuid;

const KIND_GIT_REPO_ANNOUNCEMENT: i32 = buzz_core::kind::KIND_GIT_REPO_ANNOUNCEMENT as i32;

/// The repository row as a land holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    /// Bumped by every repository-scoped authority write.
    pub version: u64,
    /// The committed manifest digest, if the row has one yet.
    pub manifest: Option<String>,
    /// The bound channel, once pinned.
    pub channel: Option<Uuid>,
}

/// What the relay knows about one key in the bound channel, read under a
/// land's locks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// An unexpired ban names the key.
    pub banned: bool,
    /// The key is a member of the community.
    pub community_member: bool,
    /// Its stored role in the bound channel, if it is an active member.
    pub channel_role: Option<String>,
}

/// The facts of a key and, for an agent with a recorded owner, its owner's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFacts {
    /// The key's own facts.
    pub key: Facts,
    /// Its recorded owner and the owner's facts, if it is an agent with one.
    pub owner: Option<(Vec<u8>, Facts)>,
}

impl Facts {
    /// Admitted on its own: unbanned, and a community member unless the
    /// relay is open.
    pub fn admitted(&self, open: bool) -> bool {
        !self.banned && (open || self.community_member)
    }
}

impl KeyFacts {
    /// Git authority requires the key's admission and, when recorded, its
    /// owner's admission on both the direct and delegated routes.
    pub fn git_admitted(&self, open: bool) -> bool {
        self.admitted(open)
            && self
                .owner
                .as_ref()
                .is_none_or(|(_, owner)| owner.admitted(open))
    }

    /// Admitted to the community: on its own, or, for an unbanned agent,
    /// through its recorded owner's admission, as the relay admits an agent
    /// its owner vouches for. Repository authority is evaluated separately:
    /// the key's own channel role or an owner-capped Git grant.
    pub fn admitted(&self, open: bool) -> bool {
        self.key.admitted(open)
            || (!self.key.banned && self.owner.as_ref().is_some_and(|(_, o)| o.admitted(open)))
    }
}

/// A won swap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swapped {
    /// The database clock when the swap's condition held.
    pub swapped_at: DateTime<Utc>,
    /// The authority version the swap committed under.
    pub version: u64,
}

/// Hold everything that can revoke a push to `repo` until the transaction
/// ends, and read its row. `None` when the repository has no row.
///
/// Lock order matches the writers': community, repository row, then the
/// announcement's replacement key (its rules), the admission key (bans,
/// timeouts, owner links), the channel's membership key
/// (role changes, removals), and the channel row (archive). Call before
/// reading any authority.
pub async fn hold(tx: &mut AdmittedTx, repo: &RepoCoordinate) -> Result<Option<Held>> {
    let community = tx.community();
    crate::governance::fence_community_shared(tx, community).await?;
    let row = sqlx::query(
        "SELECT authority_version, manifest, channel_id FROM git_repo_names \
         WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3 FOR KEY SHARE",
    )
    .bind(community.as_uuid())
    .bind(&repo.name)
    .bind(&repo.owner)
    .fetch_optional(tx.conn())
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let version: i64 = row.try_get("authority_version")?;
    let held = Held {
        version: u64::try_from(version)
            .map_err(|_| crate::DbError::InvalidData("invalid authority version".into()))?,
        manifest: row.try_get("manifest")?,
        channel: row.try_get("channel_id")?,
    };
    hold_announcement(tx, repo).await?;
    crate::relay_invite::lock_admission_shared(tx.conn(), community).await?;
    if let Some(channel) = held.channel {
        hold_channel(tx, channel).await?;
    }
    Ok(Some(held))
}

/// Hold the announcement's replacement key, which every replacement of it
/// takes exclusively, so its rules cannot change before commit. Deleting the
/// announcement takes no such key, but it only removes rules: a land judged
/// under the stricter ones it removes is still allowed under what is left.
pub(crate) async fn hold_announcement(tx: &mut AdmittedTx, repo: &RepoCoordinate) -> Result<()> {
    let owner = hex::decode(&repo.owner)
        .map_err(|_| crate::DbError::InvalidData("invalid repository owner".into()))?;
    let key = crate::replaceable::event_replacement_lock_key(
        tx.community(),
        KIND_GIT_REPO_ANNOUNCEMENT,
        &owner,
        Some(repo.name.as_bytes()),
    );
    sqlx::query("SELECT pg_advisory_xact_lock_shared($1)")
        .bind(key)
        .execute(tx.conn())
        .await?;
    Ok(())
}

/// Hold the channel's membership key and then the channel row, both shared.
pub(crate) async fn hold_channel(tx: &mut AdmittedTx, channel: Uuid) -> Result<()> {
    let community = tx.community();
    crate::channel_members::acquire_channel_membership_lock_shared(tx.conn(), community, channel)
        .await?;
    sqlx::query("SELECT 1 FROM channels WHERE community_id = $1 AND id = $2 FOR SHARE")
        .bind(community.as_uuid())
        .bind(channel)
        .fetch_optional(tx.conn())
        .await?;
    Ok(())
}

/// Pin the bound channel on a row that has none, then hold it. Only the first
/// pin counts; afterwards a governed rebind is the only way to move it.
pub async fn pin_channel(
    tx: &mut AdmittedTx,
    repo: &RepoCoordinate,
    channel: Uuid,
) -> Result<Option<Uuid>> {
    let community = tx.community();
    let pinned: Option<Option<Uuid>> = sqlx::query_scalar(
        "UPDATE git_repo_names SET channel_id = COALESCE(channel_id, $4) \
         WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3 RETURNING channel_id",
    )
    .bind(community.as_uuid())
    .bind(&repo.name)
    .bind(&repo.owner)
    .bind(channel)
    .fetch_optional(tx.conn())
    .await?;
    let pinned = pinned.flatten();
    if let Some(channel) = pinned {
        hold_channel(tx, channel).await?;
    }
    Ok(pinned)
}

/// The repository's bound channel: its pin, or on a row with none yet,
/// `announced` pinned now. `None` when there is no pin and nothing to pin
/// (or no row).
///
/// The pin is the binding, the only one: the read gate, the hook, the land
/// and governance all read it, and an announcement's tag only ever seeds it.
/// Whoever touches the repository first pins it, so a re-announcement moves
/// nothing once anyone could rely on it. Before that, only the announcer's
/// own announcements compete. The update only fills a NULL; it waits only on
/// a land that is pinning the same row in its own transaction.
pub async fn bind_channel(
    tx: &mut AdmittedTx,
    repo: &RepoCoordinate,
    announced: Option<Uuid>,
) -> Result<Option<Uuid>> {
    let community = tx.community();
    if let Some(channel) = announced {
        sqlx::query(
            "UPDATE git_repo_names SET channel_id = $4 \
             WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3 \
               AND channel_id IS NULL",
        )
        .bind(community.as_uuid())
        .bind(&repo.name)
        .bind(&repo.owner)
        .bind(channel)
        .execute(tx.conn())
        .await?;
    }
    let pinned: Option<Option<Uuid>> = sqlx::query_scalar(
        "SELECT channel_id FROM git_repo_names \
         WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3",
    )
    .bind(community.as_uuid())
    .bind(&repo.name)
    .bind(&repo.owner)
    .fetch_optional(tx.conn())
    .await?;
    Ok(pinned.flatten())
}

/// Move the pin to `channel`: a governed bind, whose command holds the
/// repository's exclusive fence and has checked an admin of both channels.
/// Nothing else moves a pin.
pub async fn rebind(tx: &mut AdmittedTx, repo: &RepoCoordinate, channel: Uuid) -> Result<()> {
    let community = tx.community();
    let moved = sqlx::query(
        "UPDATE git_repo_names SET channel_id = $4 \
         WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3",
    )
    .bind(community.as_uuid())
    .bind(&repo.name)
    .bind(&repo.owner)
    .bind(channel)
    .execute(tx.conn())
    .await?;
    if moved.rows_affected() != 1 {
        return Err(crate::DbError::AccessDenied(
            "restricted: governance authority required".into(),
        ));
    }
    Ok(())
}

/// The facts the evaluator credits `key` with in `channel`, read under the
/// land's locks. Locks the key's community membership, and an agent's
/// owner's, so neither can be removed before commit.
///
/// The owner's row is locked first: removing a member locks its row and then
/// its agents', so the same order here cannot deadlock with it. Only one level
/// is read: an owner's own owner is never locked. The owner link itself
/// cannot move under the land, which holds admission.
pub async fn key_facts(tx: &mut AdmittedTx, channel: Uuid, key: &[u8]) -> Result<KeyFacts> {
    load_key_facts(tx, channel, key, true).await
}

/// Read-door snapshot of key and owner facts, without roster row locks.
/// Unlike a land, this snapshot does not fence later membership changes.
pub async fn key_facts_for_read(
    tx: &mut AdmittedTx,
    channel: Uuid,
    key: &[u8],
) -> Result<KeyFacts> {
    load_key_facts(tx, channel, key, false).await
}

async fn load_key_facts(
    tx: &mut AdmittedTx,
    channel: Uuid,
    key: &[u8],
    lock_membership: bool,
) -> Result<KeyFacts> {
    let community = tx.community();
    let owner: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT agent_owner_pubkey FROM users WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community.as_uuid())
    .bind(key)
    .fetch_optional(tx.conn())
    .await?
    .flatten();
    let owner = match owner {
        Some(owner) => {
            let facts = facts(tx, channel, &owner, lock_membership).await?;
            Some((owner, facts))
        }
        None => None,
    };
    Ok(KeyFacts {
        key: facts(tx, channel, key, lock_membership).await?,
        owner,
    })
}

/// One key's facts, optionally locking its community membership for a land.
async fn facts(
    tx: &mut AdmittedTx,
    channel: Uuid,
    key: &[u8],
    lock_membership: bool,
) -> Result<Facts> {
    let community = tx.community();
    let row = sqlx::query(
        "SELECT \
           EXISTS (SELECT 1 FROM community_bans WHERE community_id = $1 AND pubkey = $3 \
                   AND banned AND (ban_expires_at IS NULL OR ban_expires_at > clock_timestamp())) \
             AS banned, \
           (SELECT m.role::text FROM channel_members m JOIN channels c \
              ON c.community_id = m.community_id AND c.id = m.channel_id AND c.deleted_at IS NULL \
            WHERE m.community_id = $1 AND m.channel_id = $2 AND m.pubkey = $3 \
              AND m.removed_at IS NULL) AS channel_role",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(key)
    .fetch_one(tx.conn())
    .await?;
    let membership_query = if lock_membership {
        "SELECT 1 FROM relay_members WHERE community_id = $1 AND pubkey = encode($2, 'hex') FOR KEY SHARE"
    } else {
        "SELECT 1 FROM relay_members WHERE community_id = $1 AND pubkey = encode($2, 'hex')"
    };
    let member: Option<i32> = sqlx::query_scalar(membership_query)
        .bind(community.as_uuid())
        .bind(key)
        .fetch_optional(tx.conn())
        .await?;
    Ok(Facts {
        banned: row.try_get("banned")?,
        community_member: member.is_some(),
        channel_role: row.try_get("channel_role")?,
    })
}

/// Whether the bound channel is archived (or gone), read under the land's
/// lock on its row.
pub async fn channel_closed(tx: &mut AdmittedTx, channel: Uuid) -> Result<bool> {
    let community = tx.community();
    let open: Option<bool> = sqlx::query_scalar(
        "SELECT archived_at IS NULL AND deleted_at IS NULL FROM channels \
         WHERE community_id = $1 AND id = $2",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .fetch_optional(tx.conn())
    .await?;
    Ok(open != Some(true))
}

/// The tags of the repository's current announcement, read in the land.
pub async fn announcement_tags(
    tx: &mut AdmittedTx,
    repo: &RepoCoordinate,
) -> Result<Option<Vec<Vec<String>>>> {
    let community = tx.community();
    let tags: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT tags FROM events WHERE community_id = $1 AND kind = $4 \
         AND pubkey = decode($2, 'hex') AND d_tag = $3 AND deleted_at IS NULL \
         ORDER BY created_at DESC, id ASC LIMIT 1",
    )
    .bind(community.as_uuid())
    .bind(&repo.owner)
    .bind(&repo.name)
    .bind(KIND_GIT_REPO_ANNOUNCEMENT)
    .fetch_optional(tx.conn())
    .await?;
    Ok(tags.map(serde_json::from_value).transpose()?)
}

/// Move the repository from `seen` to `new` if it is still at `seen` and the
/// database clock is before `valid_until`. `None` when the condition fails:
/// a concurrent land won, or the authority expired.
///
/// A second swap of the same row waits for the first to commit, then
/// re-evaluates its condition against the committed value and misses.
pub async fn swap(
    tx: &mut AdmittedTx,
    repo: &RepoCoordinate,
    seen: Option<&str>,
    new: &str,
    valid_until: Option<DateTime<Utc>>,
) -> Result<Option<Swapped>> {
    let community = tx.community();
    let row = sqlx::query(
        "UPDATE git_repo_names SET manifest = $5 \
         WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3 \
           AND manifest IS NOT DISTINCT FROM $4 \
           AND ($6::timestamptz IS NULL OR clock_timestamp() < $6) \
         RETURNING clock_timestamp() AS swapped_at, authority_version",
    )
    .bind(community.as_uuid())
    .bind(&repo.name)
    .bind(&repo.owner)
    .bind(seen)
    .bind(new)
    .bind(valid_until)
    .fetch_optional(tx.conn())
    .await?;
    row.map(|row| {
        let version: i64 = row.try_get("authority_version")?;
        Ok(Swapped {
            swapped_at: row.try_get("swapped_at")?,
            version: u64::try_from(version)
                .map_err(|_| crate::DbError::InvalidData("invalid authority version".into()))?,
        })
    })
    .transpose()
}

impl Db {
    /// The repository's committed manifest digest, read from the writer.
    #[datastore_span(name = "git_repo_manifest", system = "postgresql")]
    pub async fn git_repo_manifest(
        &self,
        community: CommunityId,
        repo: &RepoCoordinate,
    ) -> Result<Option<String>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let manifest: Option<Option<String>> = sqlx::query_scalar(
            "SELECT manifest FROM git_repo_names \
             WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3",
        )
        .bind(community.as_uuid())
        .bind(&repo.name)
        .bind(&repo.owner)
        .fetch_optional(&mut *connection)
        .await?;
        Ok(manifest.flatten())
    }

    /// [`bind_channel`] in its own transaction, for paths outside a land.
    #[datastore_span(name = "git_repo_channel", system = "postgresql")]
    pub async fn git_repo_channel(
        &self,
        community: CommunityId,
        repo: &RepoCoordinate,
        announced: Option<Uuid>,
    ) -> Result<Option<Uuid>> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let channel = bind_channel(&mut tx, repo, announced).await?;
        tx.commit().await?;
        Ok(channel)
    }

    /// Adopt `cache`, a manifest a relay that predates governed lands
    /// published directly on the committed `row`, for an ungoverned
    /// repository: move the row to it. False, moving nothing, if the row
    /// moved meanwhile or governance is in force for the repository.
    /// Governance writes hold the community or the row, so neither can
    /// change between the swap and the check.
    #[datastore_span(name = "git_repo_adopt", system = "postgresql")]
    pub async fn git_repo_adopt(
        &self,
        community: CommunityId,
        repo: &RepoCoordinate,
        row: &str,
        cache: &str,
    ) -> Result<bool> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        crate::governance::fence_community_shared(&mut tx, community).await?;
        if swap(&mut tx, repo, Some(row), cache, None).await?.is_none()
            || crate::governance::governed(&mut tx, repo).await?
        {
            tx.rollback().await?;
            return Ok(false);
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Seed a row that has no committed manifest yet with the object store's,
    /// for a repository created before governed lands. Never moves a row that
    /// has one. Returns the row's manifest afterwards.
    #[datastore_span(name = "git_repo_seed_manifest", system = "postgresql")]
    pub async fn git_repo_seed_manifest(
        &self,
        community: CommunityId,
        repo: &RepoCoordinate,
        digest: &str,
    ) -> Result<Option<String>> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        crate::governance::fence_community_shared(&mut tx, community).await?;
        let manifest: Option<Option<String>> = sqlx::query_scalar(
            "UPDATE git_repo_names SET manifest = COALESCE(manifest, $4) \
             WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3 RETURNING manifest",
        )
        .bind(community.as_uuid())
        .bind(&repo.name)
        .bind(&repo.owner)
        .bind(digest)
        .fetch_optional(tx.conn())
        .await?;
        tx.commit().await?;
        Ok(manifest.flatten())
    }
}

#[cfg(test)]
#[path = "git_land/postgres_tests.rs"]
mod postgres_tests;
