//! PostgreSQL-authoritative visibility for protected object-store content.

use buzz_core::CommunityId;
use serde_json::Value;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::protected_visibility::{
    require_protected_object_authority, ProtectedObjectAuthorityState, ProtectedObjectSurface,
};
use crate::{Db, DbError, Result};

/// Exact database policy state evaluated by the Git pre-receive hook.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitPolicyCommitFence {
    /// Exact active kind-30617 event evaluated by the hook.
    pub announcement_id: String,
    /// Channel bound by that announcement, when present.
    pub channel_id: Option<Uuid>,
    /// Exact database relationship used to derive the evaluated role.
    pub grant: GitPolicyGrant,
}

/// Database relationship that granted the evaluated push role.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GitPolicyGrant {
    /// The authenticated pusher is the repository announcement author.
    RepoOwner,
    /// The authenticated pusher owns the managed-agent repository key.
    ManagedAgentOwner,
    /// The authenticated pusher held this exact active channel role.
    ChannelMember {
        /// Role text as stored in PostgreSQL and evaluated by the hook.
        role: String,
    },
}

/// Current Git publication selected by PostgreSQL.
#[derive(Clone, PartialEq, Eq)]
pub struct GitPublication {
    /// Repository owner key encoded by the existing Git namespace.
    pub owner_pubkey: String,
    /// Verified immutable manifest digest.
    pub manifest_sha256: String,
    /// Monotonic compare-and-set version.
    pub publication_version: u64,
}

impl std::fmt::Debug for GitPublication {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GitPublication")
            .field("owner_pubkey", &"[redacted]")
            .field("manifest_sha256", &"[redacted]")
            .field("publication_version", &self.publication_version)
            .finish()
    }
}

/// Expected parent for one PostgreSQL Git publication CAS.
#[derive(Clone, PartialEq, Eq)]
pub struct ExpectedGitPublication {
    /// Monotonic parent version.
    pub publication_version: u64,
    /// Exact parent manifest digest.
    pub manifest_sha256: String,
}

/// Outcome of a PostgreSQL Git publication CAS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitPublicationOutcome {
    /// The new manifest became authoritative at this version.
    Published(GitPublication),
    /// The authoritative parent did not match the supplied expectation.
    Conflict,
}

/// Inputs for one PostgreSQL-authoritative Git publication.
pub struct GitPublicationRequest<'a> {
    /// Community whose repository namespace is being changed.
    pub community_id: CommunityId,
    /// Stable repository identifier.
    pub repo_id: &'a str,
    /// Repository owner key encoded by the existing namespace.
    pub owner_pubkey: &'a str,
    /// Required parent publication, or no parent for initial publication.
    pub expected: Option<&'a ExpectedGitPublication>,
    /// Digest of the immutable manifest being published.
    pub manifest_sha256: &'a str,
    /// Authenticated pusher key evaluated by the policy fence.
    pub pusher_pubkey: &'a [u8],
    /// Exact policy state evaluated before the transaction began.
    pub policy: &'a GitPolicyCommitFence,
}

/// Canonical media publication metadata selected by PostgreSQL.
#[derive(Clone, PartialEq, Eq)]
pub struct MediaPublication {
    /// Content digest.
    pub sha256: String,
    /// Immutable object-store key.
    pub object_key: String,
    /// Canonical path extension.
    pub extension: String,
    /// Canonical MIME type.
    pub mime_type: String,
    /// Immutable object size.
    pub object_size: u64,
    /// Bounded provider-neutral metadata.
    pub metadata: Value,
    /// Optional immutable thumbnail key.
    pub thumbnail_key: Option<String>,
    /// Monotonic publication version.
    pub publication_version: u64,
}

impl std::fmt::Debug for MediaPublication {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MediaPublication")
            .field("sha256", &"[redacted]")
            .field("object_key", &"[redacted]")
            .field("extension", &self.extension)
            .field("mime_type", &self.mime_type)
            .field("object_size", &self.object_size)
            .field("metadata", &"[redacted]")
            .field("thumbnail_key", &"[redacted]")
            .field("publication_version", &self.publication_version)
            .finish()
    }
}

fn positive_version(value: i64) -> Result<u64> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| DbError::InvalidData("publication version is invalid".into()))
}

fn nonnegative_size(value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| DbError::InvalidData("publication object size is invalid".into()))
}

fn validate_digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .chars()
            .all(|character| matches!(character, '0'..='9' | 'a'..='f'))
    {
        return Err(DbError::InvalidData("publication digest is invalid".into()));
    }
    Ok(())
}

impl Db {
    /// Read the active PostgreSQL-authoritative Git publication.
    pub async fn git_publication(
        &self,
        community_id: CommunityId,
        repo_id: &str,
        owner_pubkey: &str,
    ) -> Result<Option<GitPublication>> {
        let row = sqlx::query(
            "SELECT owner_pubkey, manifest_sha256, publication_version \
             FROM git_repo_publications \
             WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3 \
               AND state = 'active'",
        )
        .bind(community_id.as_uuid())
        .bind(repo_id)
        .bind(owner_pubkey)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(GitPublication {
                owner_pubkey: row.try_get("owner_pubkey")?,
                manifest_sha256: row.try_get("manifest_sha256")?,
                publication_version: positive_version(row.try_get("publication_version")?)?,
            })
        })
        .transpose()
    }

    /// Read the active PostgreSQL-authoritative media publication.
    pub async fn media_publication(
        &self,
        community_id: CommunityId,
        sha256: &str,
    ) -> Result<Option<MediaPublication>> {
        validate_digest(sha256)?;
        let row = sqlx::query(
            "SELECT sha256, object_key, extension, mime_type, object_size, metadata, \
                    thumbnail_key, publication_version \
             FROM media_publications \
             WHERE community_id = $1 AND sha256 = $2 AND state = 'active'",
        )
        .bind(community_id.as_uuid())
        .bind(sha256)
        .fetch_optional(&self.pool)
        .await?;
        row.map(media_publication_from_row).transpose()
    }
}

/// Compare and publish one Git manifest inside the caller-owned transaction.
pub async fn compare_and_publish_git(
    transaction: &mut Transaction<'_, Postgres>,
    request: GitPublicationRequest<'_>,
) -> Result<GitPublicationOutcome> {
    let GitPublicationRequest {
        community_id,
        repo_id,
        owner_pubkey,
        expected,
        manifest_sha256,
        pusher_pubkey,
        policy,
    } = request;
    validate_digest(manifest_sha256)?;
    require_protected_object_authority(
        transaction,
        community_id,
        ProtectedObjectSurface::Git,
        ProtectedObjectAuthorityState::PostgreSql,
    )
    .await?;
    if repo_id.is_empty() || owner_pubkey.is_empty() {
        return Err(DbError::InvalidData(
            "Git publication identity is invalid".into(),
        ));
    }
    validate_git_policy_fence(
        transaction,
        community_id,
        repo_id,
        owner_pubkey,
        pusher_pubkey,
        policy,
    )
    .await?;
    let registered_owner: Option<String> = sqlx::query_scalar(
        "SELECT owner_pubkey FROM git_repo_names \
         WHERE community_id = $1 AND repo_id = $2 FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(repo_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if registered_owner.as_deref() != Some(owner_pubkey) {
        return Err(DbError::InvalidData(
            "Git publication does not match the registered owner".into(),
        ));
    }
    let current = sqlx::query(
        "SELECT owner_pubkey, manifest_sha256, publication_version, state \
         FROM git_repo_publications \
         WHERE community_id = $1 AND repo_id = $2 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(repo_id)
    .fetch_optional(&mut **transaction)
    .await?;

    match (current, expected) {
        (None, None) => {
            sqlx::query(
                "INSERT INTO git_repo_publications \
                 (community_id, repo_id, owner_pubkey, manifest_sha256, publication_version) \
                 VALUES ($1, $2, $3, $4, 1)",
            )
            .bind(community_id.as_uuid())
            .bind(repo_id)
            .bind(owner_pubkey)
            .bind(manifest_sha256)
            .execute(&mut **transaction)
            .await?;
            Ok(GitPublicationOutcome::Published(GitPublication {
                owner_pubkey: owner_pubkey.to_owned(),
                manifest_sha256: manifest_sha256.to_owned(),
                publication_version: 1,
            }))
        }
        (Some(row), Some(expected)) => {
            let current_owner: String = row.try_get("owner_pubkey")?;
            let current_digest: String = row.try_get("manifest_sha256")?;
            let current_version = positive_version(row.try_get("publication_version")?)?;
            let state: String = row.try_get("state")?;
            if state != "active"
                || current_owner != owner_pubkey
                || current_version != expected.publication_version
                || current_digest != expected.manifest_sha256
            {
                return Ok(GitPublicationOutcome::Conflict);
            }
            let next = current_version
                .checked_add(1)
                .ok_or_else(|| DbError::InvalidData("Git publication version exhausted".into()))?;
            let next_i64 = i64::try_from(next)
                .map_err(|_| DbError::InvalidData("Git publication version exhausted".into()))?;
            sqlx::query(
                "UPDATE git_repo_publications \
                 SET manifest_sha256 = $3, publication_version = $4, \
                     updated_at = clock_timestamp() \
                 WHERE community_id = $1 AND repo_id = $2",
            )
            .bind(community_id.as_uuid())
            .bind(repo_id)
            .bind(manifest_sha256)
            .bind(next_i64)
            .execute(&mut **transaction)
            .await?;
            Ok(GitPublicationOutcome::Published(GitPublication {
                owner_pubkey: owner_pubkey.to_owned(),
                manifest_sha256: manifest_sha256.to_owned(),
                publication_version: next,
            }))
        }
        _ => Ok(GitPublicationOutcome::Conflict),
    }
}

async fn validate_git_policy_fence(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    repo_id: &str,
    owner_pubkey: &str,
    pusher_pubkey: &[u8],
    policy: &GitPolicyCommitFence,
) -> Result<()> {
    let owner = hex::decode(owner_pubkey)
        .map_err(|_| DbError::InvalidData("Git policy owner is invalid".into()))?;
    let announcement = hex::decode(&policy.announcement_id)
        .map_err(|_| DbError::InvalidData("Git policy announcement is invalid".into()))?;
    if owner.len() != 32 || pusher_pubkey.len() != 32 || announcement.len() != 32 {
        return Err(DbError::InvalidData(
            "Git policy identity is invalid".into(),
        ));
    }

    let current: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM events \
         WHERE community_id = $1 AND id = $2 AND kind = 30617 \
           AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(&announcement)
    .bind(&owner)
    .bind(repo_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if current.is_none() {
        return Err(DbError::InvalidData(
            "Git policy announcement changed before publication".into(),
        ));
    }

    if let Some(channel_id) = policy.channel_id {
        let channel_active: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM channels \
             WHERE community_id = $1 AND id = $2 \
               AND archived_at IS NULL AND deleted_at IS NULL FOR SHARE",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_optional(&mut **transaction)
        .await?;
        if channel_active.is_none() {
            return Err(DbError::InvalidData(
                "Git policy channel changed before publication".into(),
            ));
        }
    }

    let granted = match &policy.grant {
        GitPolicyGrant::RepoOwner => pusher_pubkey == owner.as_slice(),
        GitPolicyGrant::ManagedAgentOwner => {
            let row: Option<i32> = sqlx::query_scalar(
                "SELECT 1 FROM users \
                 WHERE community_id = $1 AND pubkey = $2 \
                   AND agent_owner_pubkey = $3 AND deactivated_at IS NULL FOR SHARE",
            )
            .bind(community_id.as_uuid())
            .bind(&owner)
            .bind(pusher_pubkey)
            .fetch_optional(&mut **transaction)
            .await?;
            row.is_some()
        }
        GitPolicyGrant::ChannelMember { role } => {
            let Some(channel_id) = policy.channel_id else {
                return Err(DbError::InvalidData("Git policy channel is missing".into()));
            };
            let current_role: Option<String> = sqlx::query_scalar(
                "SELECT role::text FROM channel_members \
                 WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 \
                   AND removed_at IS NULL FOR SHARE",
            )
            .bind(community_id.as_uuid())
            .bind(channel_id)
            .bind(pusher_pubkey)
            .fetch_optional(&mut **transaction)
            .await?;
            current_role.as_deref() == Some(role.as_str())
        }
    };
    if !granted {
        return Err(DbError::InvalidData(
            "Git policy grant changed before publication".into(),
        ));
    }
    Ok(())
}

/// Publish immutable media metadata inside the caller-owned transaction.
///
/// Republication of identical bytes/metadata is idempotent. A conflicting row
/// for the same content digest fails closed rather than silently changing what
/// an existing URL means.
pub async fn publish_media(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    publication: &MediaPublication,
) -> Result<MediaPublication> {
    validate_digest(&publication.sha256)?;
    require_protected_object_authority(
        transaction,
        community_id,
        ProtectedObjectSurface::Media,
        ProtectedObjectAuthorityState::PostgreSql,
    )
    .await?;
    let size = i64::try_from(publication.object_size)
        .map_err(|_| DbError::InvalidData("media publication size is invalid".into()))?;
    sqlx::query(
        "INSERT INTO media_publications \
         (community_id, sha256, object_key, extension, mime_type, object_size, \
          metadata, thumbnail_key, publication_version, state) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, 'active') \
         ON CONFLICT (community_id, sha256) DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(&publication.sha256)
    .bind(&publication.object_key)
    .bind(&publication.extension)
    .bind(&publication.mime_type)
    .bind(size)
    .bind(&publication.metadata)
    .bind(&publication.thumbnail_key)
    .execute(&mut **transaction)
    .await?;

    let row = sqlx::query(
        "SELECT sha256, object_key, extension, mime_type, object_size, metadata, \
                thumbnail_key, publication_version \
         FROM media_publications \
         WHERE community_id = $1 AND sha256 = $2 AND state = 'active' FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(&publication.sha256)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| DbError::InvalidData("media publication is unavailable".into()))?;
    let stored = media_publication_from_row(row)?;
    if stored.object_key != publication.object_key
        || stored.extension != publication.extension
        || stored.mime_type != publication.mime_type
        || stored.object_size != publication.object_size
        || stored.metadata != publication.metadata
        || stored.thumbnail_key != publication.thumbnail_key
    {
        return Err(DbError::InvalidData(
            "media publication conflicts with existing content metadata".into(),
        ));
    }
    Ok(stored)
}

/// Idempotently import one legacy Git publication while visibility remains fenced.
pub async fn import_git_publication(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    repo_id: &str,
    owner_pubkey: &str,
    manifest_sha256: &str,
) -> Result<()> {
    validate_digest(manifest_sha256)?;
    require_protected_object_authority(
        transaction,
        community_id,
        ProtectedObjectSurface::Git,
        ProtectedObjectAuthorityState::Importing,
    )
    .await?;
    sqlx::query(
        "INSERT INTO git_repo_publications \
         (community_id, repo_id, owner_pubkey, manifest_sha256, publication_version, state) \
         VALUES ($1, $2, $3, $4, 1, 'active') \
         ON CONFLICT (community_id, repo_id) DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(repo_id)
    .bind(owner_pubkey)
    .bind(manifest_sha256)
    .execute(&mut **transaction)
    .await?;
    let stored = sqlx::query(
        "SELECT owner_pubkey, manifest_sha256, publication_version, state \
         FROM git_repo_publications WHERE community_id = $1 AND repo_id = $2 FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(repo_id)
    .fetch_one(&mut **transaction)
    .await?;
    let stored_owner: String = stored.try_get("owner_pubkey")?;
    let stored_digest: String = stored.try_get("manifest_sha256")?;
    let stored_version = positive_version(stored.try_get("publication_version")?)?;
    let stored_state: String = stored.try_get("state")?;
    if stored_owner != owner_pubkey
        || stored_digest != manifest_sha256
        || stored_version != 1
        || stored_state != "active"
    {
        return Err(DbError::InvalidData(
            "Git import conflicts with an existing publication".into(),
        ));
    }
    Ok(())
}

/// Idempotently import one legacy media publication while visibility remains fenced.
pub async fn import_media_publication(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    publication: &MediaPublication,
) -> Result<()> {
    require_protected_object_authority(
        transaction,
        community_id,
        ProtectedObjectSurface::Media,
        ProtectedObjectAuthorityState::Importing,
    )
    .await?;
    // `publish_media` requires the final PostgreSQL state, so reproduce its
    // exact idempotent row contract under the import-state lock.
    validate_digest(&publication.sha256)?;
    let size = i64::try_from(publication.object_size)
        .map_err(|_| DbError::InvalidData("media publication size is invalid".into()))?;
    sqlx::query(
        "INSERT INTO media_publications \
         (community_id, sha256, object_key, extension, mime_type, object_size, \
          metadata, thumbnail_key, publication_version, state) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, 'active') \
         ON CONFLICT (community_id, sha256) DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(&publication.sha256)
    .bind(&publication.object_key)
    .bind(&publication.extension)
    .bind(&publication.mime_type)
    .bind(size)
    .bind(&publication.metadata)
    .bind(&publication.thumbnail_key)
    .execute(&mut **transaction)
    .await?;
    let row = sqlx::query(
        "SELECT sha256, object_key, extension, mime_type, object_size, metadata, \
                thumbnail_key, publication_version \
         FROM media_publications \
         WHERE community_id = $1 AND sha256 = $2 AND state = 'active' FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(&publication.sha256)
    .fetch_one(&mut **transaction)
    .await?;
    let stored = media_publication_from_row(row)?;
    if stored.object_key != publication.object_key
        || stored.extension != publication.extension
        || stored.mime_type != publication.mime_type
        || stored.object_size != publication.object_size
        || stored.metadata != publication.metadata
        || stored.thumbnail_key != publication.thumbnail_key
        || stored.publication_version != 1
    {
        return Err(DbError::InvalidData(
            "media import conflicts with an existing publication".into(),
        ));
    }
    Ok(())
}

/// List exact Git repository reservations for one migration domain.
pub async fn list_git_repo_reservations(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
) -> Result<Vec<(String, String, String)>> {
    require_protected_object_authority(
        transaction,
        community_id,
        ProtectedObjectSurface::Git,
        ProtectedObjectAuthorityState::Importing,
    )
    .await?;
    let rows = sqlx::query(
        "SELECT repo_id, owner_pubkey, publication_origin FROM git_repo_names \
         WHERE community_id = $1 ORDER BY repo_id FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .fetch_all(&mut **transaction)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok((
                row.try_get("repo_id")?,
                row.try_get("owner_pubkey")?,
                row.try_get("publication_origin")?,
            ))
        })
        .collect()
}

/// List the exact imported Git inventory for parity validation.
pub async fn list_git_publications(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
) -> Result<Vec<(String, String, String)>> {
    let rows = sqlx::query(
        "SELECT repo_id, owner_pubkey, manifest_sha256 FROM git_repo_publications \
         WHERE community_id = $1 AND state = 'active' ORDER BY repo_id FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .fetch_all(&mut **transaction)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok((
                row.try_get("repo_id")?,
                row.try_get("owner_pubkey")?,
                row.try_get("manifest_sha256")?,
            ))
        })
        .collect()
}

/// List the exact imported media inventory for parity validation.
pub async fn list_media_publications(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
) -> Result<Vec<MediaPublication>> {
    let rows = sqlx::query(
        "SELECT sha256, object_key, extension, mime_type, object_size, metadata, \
                thumbnail_key, publication_version FROM media_publications \
         WHERE community_id = $1 AND state = 'active' ORDER BY sha256 FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .fetch_all(&mut **transaction)
    .await?;
    rows.into_iter().map(media_publication_from_row).collect()
}

fn media_publication_from_row(row: sqlx::postgres::PgRow) -> Result<MediaPublication> {
    Ok(MediaPublication {
        sha256: row.try_get("sha256")?,
        object_key: row.try_get("object_key")?,
        extension: row.try_get("extension")?,
        mime_type: row.try_get("mime_type")?,
        object_size: nonnegative_size(row.try_get("object_size")?)?,
        metadata: row.try_get("metadata")?,
        thumbnail_key: row.try_get("thumbnail_key")?,
        publication_version: positive_version(row.try_get("publication_version")?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup() -> (Db, CommunityId) {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let pool = PgPool::connect(&database_url).await.expect("test database");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("migrated test database");
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("publication-{}.example", id.simple()))
            .execute(&pool)
            .await
            .expect("community");
        for surface in ["git", "media"] {
            sqlx::query(
                "INSERT INTO protected_object_authority \
                 (community_id, surface, state, generation, imported_objects, \
                  inventory_sha256, started_at, completed_at) \
                 VALUES ($1, $2, 'postgresql', 2, 0, $3, \
                         clock_timestamp(), clock_timestamp())",
            )
            .bind(id)
            .bind(surface)
            .bind("0".repeat(64))
            .execute(&pool)
            .await
            .expect("protected object authority");
        }
        (Db::from_pool(pool), CommunityId::from_uuid(id))
    }

    fn digest(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn git_publication_is_owner_bound_and_compare_and_set() {
        let (db, community) = setup().await;
        let repo = format!("repo-{}", Uuid::new_v4().simple());
        let owner = digest(1);
        let owner_bytes = hex::decode(&owner).expect("owner bytes");
        let announcement_id = digest(8);
        sqlx::query(
            "INSERT INTO events \
             (community_id, id, pubkey, created_at, kind, tags, content, sig, d_tag) \
             VALUES ($1, $2, $3, clock_timestamp(), 30617, $4, '', $5, $6)",
        )
        .bind(community.as_uuid())
        .bind(hex::decode(&announcement_id).expect("announcement bytes"))
        .bind(&owner_bytes)
        .bind(serde_json::json!([["d", repo]]))
        .bind(vec![0_u8; 64])
        .bind(&repo)
        .execute(&db.pool)
        .await
        .expect("announcement");
        sqlx::query(
            "INSERT INTO git_repo_names (community_id, repo_id, owner_pubkey) \
             VALUES ($1, $2, $3)",
        )
        .bind(community.as_uuid())
        .bind(&repo)
        .bind(&owner)
        .execute(&db.pool)
        .await
        .expect("repo reservation");
        let policy = GitPolicyCommitFence {
            announcement_id,
            channel_id: None,
            grant: GitPolicyGrant::RepoOwner,
        };

        let mut transaction = db.begin_transaction().await.expect("transaction");
        let first = compare_and_publish_git(
            &mut transaction,
            GitPublicationRequest {
                community_id: community,
                repo_id: &repo,
                owner_pubkey: &owner,
                expected: None,
                manifest_sha256: &digest(2),
                pusher_pubkey: &owner_bytes,
                policy: &policy,
            },
        )
        .await
        .expect("first publication");
        transaction.commit().await.expect("commit");
        let GitPublicationOutcome::Published(first) = first else {
            panic!("first publication must win")
        };
        assert_eq!(first.publication_version, 1);
        assert!(db
            .git_publication(community, &repo, &digest(9))
            .await
            .expect("wrong-owner read")
            .is_none());

        let mut stale = db.begin_transaction().await.expect("stale transaction");
        assert_eq!(
            compare_and_publish_git(
                &mut stale,
                GitPublicationRequest {
                    community_id: community,
                    repo_id: &repo,
                    owner_pubkey: &owner,
                    expected: None,
                    manifest_sha256: &digest(3),
                    pusher_pubkey: &owner_bytes,
                    policy: &policy,
                },
            )
            .await
            .expect("stale compare"),
            GitPublicationOutcome::Conflict
        );
        stale.rollback().await.expect("rollback stale compare");

        let mut next = db.begin_transaction().await.expect("next transaction");
        let expected = ExpectedGitPublication {
            publication_version: first.publication_version,
            manifest_sha256: first.manifest_sha256,
        };
        let second = compare_and_publish_git(
            &mut next,
            GitPublicationRequest {
                community_id: community,
                repo_id: &repo,
                owner_pubkey: &owner,
                expected: Some(&expected),
                manifest_sha256: &digest(3),
                pusher_pubkey: &owner_bytes,
                policy: &policy,
            },
        )
        .await
        .expect("next publication");
        next.commit().await.expect("commit next");
        assert!(matches!(
            second,
            GitPublicationOutcome::Published(GitPublication {
                publication_version: 2,
                ..
            })
        ));
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn media_visibility_is_transaction_owned_and_rollback_leaves_no_row() {
        let (db, community) = setup().await;
        let publication = MediaPublication {
            sha256: digest(4),
            object_key: format!("{}.jpg", digest(4)),
            extension: "jpg".into(),
            mime_type: "image/jpeg".into(),
            object_size: 17,
            metadata: serde_json::json!({"synthetic": true}),
            thumbnail_key: None,
            publication_version: 1,
        };
        let mut rolled_back = db.begin_transaction().await.expect("transaction");
        publish_media(&mut rolled_back, community, &publication)
            .await
            .expect("staged publication");
        rolled_back.rollback().await.expect("rollback");
        assert!(db
            .media_publication(community, &publication.sha256)
            .await
            .expect("publication read")
            .is_none());

        let mut committed = db.begin_transaction().await.expect("transaction");
        publish_media(&mut committed, community, &publication)
            .await
            .expect("publication");
        committed.commit().await.expect("commit");
        assert_eq!(
            db.media_publication(community, &publication.sha256)
                .await
                .expect("publication read")
                .expect("published row")
                .object_key,
            publication.object_key
        );
    }
}
