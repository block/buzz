//! Governance persistence. Commands never enter the ordinary event store.
//! A caller commits the signed state and its audit entry in one admitted tx.
use crate::{AdmittedTx, Db, DbError, Result};
use buzz_core::governance::{
    Body, Command, Document, DocumentId, ExpectedHead, GitGrant, GrantAction, RepoCoordinate,
};
use buzz_core::kind::KIND_GOVERNANCE_STATE;
use buzz_core::CommunityId;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use uuid::Uuid;

fn invalid(reason: impl Into<String>) -> DbError {
    DbError::InvalidData(reason.into())
}
fn denied() -> DbError {
    DbError::AccessDenied("restricted: governance authority required".into())
}

/// Prepared accepted state. The caller MUST append the audit entry before commit.
pub struct Prepared {
    /// New or originally accepted signed state.
    pub state: Event,
    /// Historical state replaced by this command.
    pub previous: Option<Event>,
    /// An exact accepted command retry: no write or new audit entry is needed.
    pub replay: bool,
    /// Expiry of the agent's Git grant this decision relied on.
    pub valid_until: Option<i64>,
}

// Authority fences (docs/spec/GitGovernanceLand.tla, protocol "row").
//
//   community  advisory lock: lands and repo-scoped writes share it;
//              floor, trusted checks and protected labels take it alone.
//   repo       its `git_repo_names` row: a land reads it FOR KEY SHARE and
//              swaps the manifest with a plain UPDATE (NO KEY UPDATE), so
//              lands never block each other's reads; an authority write
//              takes FOR UPDATE, so it waits for every land in flight, and
//              bumps `authority_version`.
//
// Lock order, always: community, then repo row, then anything else.

fn community_fence_key(community: CommunityId) -> String {
    format!("buzz_git_authority:{}", community.as_uuid())
}

/// Hold the community's authority shared until the transaction ends.
pub async fn fence_community_shared(tx: &mut AdmittedTx, community: CommunityId) -> Result<()> {
    community_fence(tx, community, true).await
}
/// Hold the community's authority alone until the transaction ends.
pub async fn fence_community_exclusive(tx: &mut AdmittedTx, community: CommunityId) -> Result<()> {
    community_fence(tx, community, false).await
}
async fn community_fence(tx: &mut AdmittedTx, community: CommunityId, shared: bool) -> Result<()> {
    if community != tx.community() {
        return Err(denied());
    }
    let sql = if shared {
        "SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))"
    } else {
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))"
    };
    sqlx::query(sql)
        .bind(community_fence_key(community))
        .execute(tx.conn())
        .await?;
    Ok(())
}
/// Hold the repository's authority shared and return its version. An
/// unknown repository is a denial.
pub async fn fence_repo_shared(tx: &mut AdmittedTx, repo: &RepoCoordinate) -> Result<u64> {
    repo_fence(tx, repo, false).await
}
/// Hold the repository's authority alone, bump its version and return it.
pub async fn fence_repo_exclusive(tx: &mut AdmittedTx, repo: &RepoCoordinate) -> Result<u64> {
    repo_fence(tx, repo, true).await?;
    let community = tx.community();
    let version: i64 = sqlx::query_scalar(
        "UPDATE git_repo_names SET authority_version = authority_version + 1 \
         WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3 \
         RETURNING authority_version",
    )
    .bind(community.as_uuid())
    .bind(&repo.name)
    .bind(&repo.owner)
    .fetch_one(tx.conn())
    .await?;
    u64::try_from(version).map_err(|_| invalid("invalid authority version"))
}
async fn repo_fence(tx: &mut AdmittedTx, repo: &RepoCoordinate, exclusive: bool) -> Result<u64> {
    let community = tx.community();
    let sql = if exclusive {
        "SELECT authority_version FROM git_repo_names \
         WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3 FOR UPDATE"
    } else {
        "SELECT authority_version FROM git_repo_names \
         WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3 FOR KEY SHARE"
    };
    let version: Option<i64> = sqlx::query_scalar(sql)
        .bind(community.as_uuid())
        .bind(&repo.name)
        .bind(&repo.owner)
        .fetch_optional(tx.conn())
        .await?;
    let version = version.ok_or_else(denied)?;
    u64::try_from(version).map_err(|_| invalid("invalid authority version"))
}

/// Lock community before repository; protected labels also serialize with
/// community-wide policy/schema updates. Both versions commit with the audit.
pub async fn fence_authority_write(tx: &mut AdmittedTx, document: &DocumentId) -> Result<()> {
    let community = tx.community();
    match document {
        DocumentId::Floor | DocumentId::Trusted | DocumentId::Labels(_) => {
            fence_community_exclusive(tx, community).await?
        }
        _ => fence_community_shared(tx, community).await?,
    }
    if let Some(repo) = document.repo() {
        fence_repo_exclusive(tx, repo).await?;
    }
    Ok(())
}

async fn fence_authority_read(tx: &mut AdmittedTx, document: &DocumentId) -> Result<()> {
    let community = tx.community();
    fence_community_shared(tx, community).await?;
    if let Some(repo) = document.repo() {
        fence_repo_shared(tx, repo).await?;
    }
    Ok(())
}

/// Read the accepted relay-authored head using the caller's transaction.
pub async fn head(tx: &mut AdmittedTx, id: &DocumentId) -> Result<Option<Event>> {
    let community = tx.community();
    let value: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT detail->'state' FROM audit_log WHERE community_id=$1 \
         AND action='governance_changed' AND detail->>'document'=$2 ORDER BY seq DESC LIMIT 1",
    )
    .bind(community.as_uuid())
    .bind(id.to_string())
    .fetch_optional(tx.conn())
    .await?;
    value
        .map(|v| {
            Event::from_json(v.to_string()).map_err(|_| invalid("invalid stored governance head"))
        })
        .transpose()
}

pub(crate) async fn body(tx: &mut AdmittedTx, id: &DocumentId) -> Result<Option<Document>> {
    head(tx, id)
        .await?
        .map(|e| Document::parse(&e.content, id).map_err(invalid))
        .transpose()
}

/// The repository's bound channel: its pin ([`crate::git_land::bind_channel`]),
/// seeded from the announcement only while it has none. The announcement's
/// author never supplies authority after that.
pub async fn repo_channel(tx: &mut AdmittedTx, repo: &RepoCoordinate) -> Result<Uuid> {
    let community = tx.community();
    let tags: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT tags FROM events WHERE community_id=$1 AND kind=30617 AND pubkey=decode($2,'hex') \
         AND d_tag=$3 AND deleted_at IS NULL ORDER BY created_at DESC,id ASC LIMIT 1",
    )
    .bind(community.as_uuid())
    .bind(&repo.owner)
    .bind(&repo.name)
    .fetch_optional(tx.conn())
    .await?;
    let tags: Vec<Vec<String>> = tags
        .map(serde_json::from_value)
        .transpose()?
        .ok_or_else(denied)?;
    // The first `buzz-channel` tag, as the relay's binding resolver reads it.
    let announced = tags
        .iter()
        .find(|t| t.first().is_some_and(|s| s == "buzz-channel"))
        .and_then(|t| t.get(1))
        .and_then(|v| v.parse().ok());
    crate::git_land::bind_channel(tx, repo, announced)
        .await?
        .ok_or_else(denied)
}

/// Whether governance is in force for `repo`: its community has a floor,
/// which applies to every repository, or the repository has a document of
/// its own. Reads of a governed repository are recorded.
pub async fn governed(tx: &mut AdmittedTx, repo: &RepoCoordinate) -> Result<bool> {
    let community = tx.community();
    let documents = [
        DocumentId::Floor,
        DocumentId::Rules(repo.clone()),
        DocumentId::Labels(repo.clone()),
        DocumentId::Bind(repo.clone()),
    ]
    .map(|d| d.to_string());
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM audit_log WHERE community_id=$1 \
         AND action='governance_changed' AND detail->>'document'=ANY($2))",
    )
    .bind(community.as_uuid())
    .bind(&documents[..])
    .fetch_one(tx.conn())
    .await?)
}

/// Unbanned. Archiving is a visibility hint, not a ban (NIP-IA), so it
/// leaves governance exactly as it leaves pushes.
async fn active(tx: &mut AdmittedTx, key: &[u8]) -> Result<bool> {
    let community = tx.community();
    Ok(sqlx::query_scalar(
        "SELECT NOT EXISTS (SELECT 1 FROM community_bans WHERE community_id=$1 AND pubkey=$2 \
         AND banned AND (ban_expires_at IS NULL OR ban_expires_at>clock_timestamp()))",
    )
    .bind(community.as_uuid())
    .bind(key)
    .fetch_one(tx.conn())
    .await?)
}
async fn owner(tx: &mut AdmittedTx, key: &[u8]) -> Result<Option<Vec<u8>>> {
    let community = tx.community();
    Ok(sqlx::query_scalar::<_, Option<Vec<u8>>>(
        "SELECT agent_owner_pubkey FROM users WHERE community_id=$1 AND pubkey=$2",
    )
    .bind(community.as_uuid())
    .bind(key)
    .fetch_optional(tx.conn())
    .await?
    .flatten())
}
async fn channel_role(tx: &mut AdmittedTx, ch: Uuid, key: &[u8]) -> Result<Option<String>> {
    let community = tx.community();
    Ok(sqlx::query_scalar("SELECT m.role::text FROM channel_members m JOIN channels c ON c.community_id=m.community_id AND c.id=m.channel_id WHERE m.community_id=$1 AND m.channel_id=$2 AND m.pubkey=$3 AND m.removed_at IS NULL AND c.deleted_at IS NULL")
        .bind(community.as_uuid()).bind(ch).bind(key).fetch_optional(tx.conn()).await?)
}
fn reader(role: Option<&str>) -> bool {
    matches!(role, Some("owner" | "admin" | "member" | "guest" | "bot"))
}
fn admin(role: Option<&str>) -> bool {
    matches!(role, Some("owner" | "admin"))
}
async fn community_admin(tx: &mut AdmittedTx, key: &[u8]) -> Result<bool> {
    let community = tx.community();
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM relay_members WHERE community_id=$1 AND pubkey=encode($2,'hex')",
    )
    .bind(community.as_uuid())
    .bind(key)
    .fetch_optional(tx.conn())
    .await?;
    Ok(active(tx, key).await? && admin(role.as_deref()))
}
async fn repo_admin(
    tx: &mut AdmittedTx,
    repo: &RepoCoordinate,
    key: &[u8],
    ch: Uuid,
    grant: Option<&GitGrant>,
) -> Result<bool> {
    if !active(tx, key).await? {
        return Ok(false);
    }
    let own_admin = admin(channel_role(tx, ch, key).await?.as_deref());
    if let Some(owner) = owner(tx, key).await? {
        if !active(tx, &owner).await? {
            return Ok(false);
        }
        return Ok(own_admin
            || (admin(channel_role(tx, ch, &owner).await?.as_deref())
                && live_grant_level(tx, grant, repo, &owner).await? == Some(GrantAction::Manage)));
    }
    Ok(own_admin)
}

/// The level of the grant presented for this repository, if live: signed by
/// the recorded owner, covering the repo, within its window and the floor's
/// lifetime cap. Callers separately cap this level by the owner's channel role.
pub async fn live_grant_level(
    tx: &mut AdmittedTx,
    grant: Option<&GitGrant>,
    repo: &RepoCoordinate,
    owner: &[u8],
) -> Result<Option<GrantAction>> {
    let Some(grant) = grant
        .filter(|g| g.owner.to_bytes()[..] == *owner && g.repo.as_ref().is_none_or(|r| r == repo))
    else {
        return Ok(None);
    };
    let now = clock(tx).await?;
    if grant.after.is_some_and(|f| f >= now) || grant.before.is_some_and(|t| t <= now) {
        return Ok(None);
    }
    let within_cap = match body(tx, &DocumentId::Floor).await?.map(|d| d.body) {
        None => true,
        Some(Body::Floor {
            max_grant_seconds, ..
        }) => grant.window().is_some_and(|w| w <= max_grant_seconds),
        Some(_) => return Err(invalid("invalid floor")),
    };
    Ok(within_cap.then(|| grant.level.clone()))
}

async fn clock(tx: &mut AdmittedTx) -> Result<u64> {
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch from clock_timestamp()))::bigint")
            .fetch_one(tx.conn())
            .await?;
    u64::try_from(now).map_err(|_| invalid("invalid database clock"))
}

/// Check the current authorization under the caller's authority fences.
pub async fn authorize(
    tx: &mut AdmittedTx,
    command: &Command,
    actor: &[u8],
    grant: Option<&GitGrant>,
) -> Result<Option<Uuid>> {
    if command.community != *tx.community().as_uuid() {
        return Err(denied());
    }
    if !active(tx, actor).await? {
        return Err(denied());
    }
    let channel = match command.document.repo() {
        Some(repo) => Some(repo_channel(tx, repo).await?),
        None => None,
    };
    let allowed = match &command.document {
        DocumentId::Floor | DocumentId::Trusted | DocumentId::Labels(_) => {
            let mut allowed = community_admin(tx, actor).await?;
            if let Some(owner) = owner(tx, actor).await? {
                allowed &= community_admin(tx, &owner).await?;
            }
            allowed
        }
        DocumentId::Rules(repo) | DocumentId::Bind(repo) => {
            let current = channel.ok_or_else(denied)?;
            let mut allowed = repo_admin(tx, repo, actor, current, grant).await?;
            if let Body::Bind { channel: next } = command.body.body {
                allowed &= repo_admin(tx, repo, actor, next, grant).await?;
            }
            allowed
        }
    };
    if allowed {
        Ok(channel)
    } else {
        Err(denied())
    }
}

/// Authorize a current-document read; errors reveal no repository existence.
/// Channel membership and an owner-capped live grant are independent routes.
pub async fn authorize_read(
    tx: &mut AdmittedTx,
    id: &DocumentId,
    actor: &[u8],
    grant: Option<&GitGrant>,
) -> Result<()> {
    if !active(tx, actor).await? {
        return Err(denied());
    }
    if let Some(repo) = id.repo() {
        let ch = repo_channel(tx, repo).await?;
        let own_read = reader(channel_role(tx, ch, actor).await?.as_deref());
        let allowed = if let Some(owner) = owner(tx, actor).await? {
            active(tx, &owner).await?
                && (own_read
                    || (reader(channel_role(tx, ch, &owner).await?.as_deref())
                        && live_grant_level(tx, grant, repo, &owner).await?.is_some()))
        } else {
            own_read
        };
        if !allowed {
            return Err(denied());
        }
    } else {
        // A community document governs every key with a place in the
        // community: a relay member, or a role in any live channel of it
        // other than a DM, archived or not, guest included. An agent's owner
        // must be unbanned.
        let community = tx.community();
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM relay_members WHERE community_id=$1 AND pubkey=encode($2,'hex')) \
             OR EXISTS(SELECT 1 FROM channel_members m JOIN channels c ON c.community_id=m.community_id \
             AND c.id=m.channel_id WHERE m.community_id=$1 AND m.pubkey=$2 AND m.removed_at IS NULL \
             AND c.deleted_at IS NULL AND c.channel_type<>'dm')",
        )
        .bind(community.as_uuid())
        .bind(actor)
        .fetch_one(tx.conn())
        .await?;
        if !member {
            return Err(denied());
        }
        if let Some(owner) = owner(tx, actor).await? {
            if !active(tx, &owner).await? {
                return Err(denied());
            }
        }
    }
    Ok(())
}

async fn validate_against_floor(tx: &mut AdmittedTx, command: &Command) -> Result<()> {
    let Body::Labels { values } = &command.body.body else {
        return Ok(());
    };
    let floor = body(tx, &DocumentId::Floor)
        .await?
        .ok_or_else(|| invalid("unavailable: missing floor"))?;
    let Body::Floor { labels, .. } = floor.body else {
        return Err(invalid("invalid floor"));
    };
    if values
        .iter()
        .any(|(key, value)| !labels.get(key).is_some_and(|d| d.values.contains(value)))
    {
        return Err(invalid("invalid protected label value"));
    }
    Ok(())
}

impl Db {
    /// Prepare a command under an already-admitted and authority-fenced tx.
    /// No state is published until the caller appends the private audit entry with `command`, `previous`
    /// and `state`, then commits; any error must drop/rollback the transaction.
    /// `grant` is the Git grant the command's signer presented, verified.
    pub async fn prepare_governance_command(
        &self,
        tx: &mut AdmittedTx,
        event: &Event,
        grant: Option<&GitGrant>,
        keys: &Keys,
    ) -> Result<Prepared> {
        let command = Command::parse(event).map_err(invalid)?;
        let channel = authorize(tx, &command, event.pubkey.as_bytes(), grant).await?;
        let community = tx.community();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 39010))")
            .bind(format!("{}:{}", community, command.document))
            .execute(tx.conn())
            .await?;
        let replay:Option<serde_json::Value>=sqlx::query_scalar("SELECT detail->'state' FROM audit_log WHERE community_id=$1 AND action='governance_changed' AND object_id=$2 ORDER BY seq LIMIT 1")
            .bind(community.as_uuid()).bind(event.id.to_hex()).fetch_optional(tx.conn()).await?;
        if let Some(state) = replay {
            let state = Event::from_json(state.to_string())
                .map_err(|_| invalid("invalid recorded governance outcome"))?;
            return Ok(Prepared {
                state,
                previous: None,
                replay: true,
                valid_until: None,
            });
        }
        let now = clock(tx).await?;
        if now.abs_diff(event.created_at.as_secs()) > 120 {
            return Err(invalid("invalid: stale governance command"));
        }
        let old = head(tx, &command.document).await?;
        let matches = match (&command.expected, &old) {
            (ExpectedHead::Absent, None) => true,
            (ExpectedHead::Event(expected), Some(old)) => expected == &old.id.to_hex(),
            _ => false,
        };
        if !matches {
            return Err(invalid("conflict: governance head changed"));
        }
        if old.is_none() {
            let initialized:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_log WHERE community_id=$1 AND action='governance_changed' AND detail->>'document'=$2)")
                .bind(community.as_uuid()).bind(command.document.to_string()).fetch_one(tx.conn()).await?;
            if initialized {
                return Err(invalid(
                    "unavailable: initialized governance head is missing",
                ));
            }
        }
        validate_against_floor(tx, &command).await?;
        // Only a command that needs delegated authority depends on the
        // grant's expiry. A member's own admin role survives an expired grant.
        // Rebinding needs authority in both the old and the new channel.
        let mut needs_grant = false;
        if matches!(command.document, DocumentId::Rules(_) | DocumentId::Bind(_)) {
            let current = channel.ok_or_else(denied)?;
            needs_grant = !admin(
                channel_role(tx, current, event.pubkey.as_bytes())
                    .await?
                    .as_deref(),
            );
            if let Body::Bind { channel: next } = command.body.body {
                needs_grant |= !admin(
                    channel_role(tx, next, event.pubkey.as_bytes())
                        .await?
                        .as_deref(),
                );
            }
        }
        let valid_until = grant
            .filter(|_| needs_grant)
            .and_then(|g| g.before)
            .map(|before| i64::try_from(before).map_err(|_| invalid("invalid grant expiry")))
            .transpose()?;
        // A bind moves the pin, the one binding every Git path reads, in the
        // transaction that records it, under the repository's exclusive fence.
        if let (DocumentId::Bind(repo), Body::Bind { channel }) =
            (&command.document, &command.body.body)
        {
            crate::git_land::rebind(tx, repo, *channel).await?;
        }
        let previous_time = old.as_ref().map(|e| e.created_at.as_secs()).unwrap_or(0);
        let next = now.max(
            previous_time
                .checked_add(1)
                .ok_or_else(|| invalid("head timestamp exhausted"))?,
        );
        let revision = old
            .as_ref()
            .map(|e| {
                e.tags
                    .iter()
                    .find_map(|t| {
                        let s = t.as_slice();
                        (s.first().map(String::as_str) == Some("revision"))
                            .then(|| s.get(1))
                            .flatten()
                    })
                    .and_then(|v| v.parse::<u64>().ok())
                    .ok_or_else(|| invalid("missing governance revision"))
            })
            .transpose()?
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| invalid("revision exhausted"))?;
        let tag = |name: &str, value: String| {
            Tag::parse([name.to_string(), value]).map_err(|e| invalid(e.to_string()))
        };
        let mut tags = vec![
            tag("community", community.to_string())?,
            tag("d", command.document.to_string())?,
            tag("prev", command.expected.to_string())?,
            tag("command", event.id.to_hex())?,
            tag("revision", revision.to_string())?,
        ];
        if let Some(repo) = command.document.repo() {
            tags.push(tag("a", repo.to_string())?);
        }
        if let Some(ch) = channel {
            tags.push(tag("h", ch.to_string())?);
        }
        let state = EventBuilder::new(
            Kind::Custom(KIND_GOVERNANCE_STATE as u16),
            serde_json::to_string(&command.body)?,
        )
        .tags(tags)
        .custom_created_at(Timestamp::from(next))
        .sign_with_keys(keys)
        .map_err(|e| invalid(e.to_string()))?;
        Ok(Prepared {
            state,
            previous: old,
            replay: false,
            valid_until,
        })
    }

    /// [`live_grant_level`] in its own transaction, for gates outside a land.
    pub async fn git_live_grant_level(
        &self,
        community: CommunityId,
        grant: Option<&GitGrant>,
        repo: &RepoCoordinate,
        owner: &[u8],
    ) -> Result<Option<GrantAction>> {
        let mut tx = self.begin_event_write_transaction(community).await?;
        let level = live_grant_level(&mut tx, grant, repo, owner).await?;
        tx.rollback().await?;
        Ok(level)
    }

    /// Read a signed governance head with current repository authorization.
    pub async fn governance_head(
        &self,
        community: CommunityId,
        id: &DocumentId,
        actor: &[u8],
        grant: Option<&GitGrant>,
    ) -> Result<Option<Event>> {
        let mut tx = self.begin_event_write_transaction(community).await?;
        fence_authority_read(&mut tx, id).await?;
        authorize_read(&mut tx, id, actor, grant).await?;
        let result = head(&mut tx, id).await?;
        // Commit: resolving the repository may have pinned its channel.
        tx.commit().await?;
        Ok(result)
    }
}

#[cfg(test)]
mod postgres_tests;
