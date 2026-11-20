//! Listing and targeted revocation of v2 invites.
//!
//! Listing never returns a code or its hash. Revocation runs the same
//! transaction gate as an owner/admin mint: the admission lock, then the
//! actor's role row `FOR SHARE`, then the restriction check, all before the
//! invite row is locked. An unknown invite is answered before the admission
//! lock, so it never queues behind claims. Owners and admins may revoke any
//! invite; any other member only one they created.
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use super::{lock_admission, principal_restricted};
use crate::error::Result;
use crate::relay_members::{is_admin, lock_member_role};
use crate::{CommunityId, Db};

/// Largest page [`list_relay_invites`] returns.
pub const MAX_INVITE_LIST_LIMIT: i64 = 200;

/// Which invite a revoke targets. Both are looked up inside one community.
#[derive(Debug, Clone, Copy)]
pub enum InviteSelector<'a> {
    /// The invite's id.
    Id(Uuid),
    /// SHA-256 of the invite's v2 code.
    TokenHash(&'a [u8; 32]),
}

/// One invite as listed to an owner or admin. Holds no code or hash.
#[derive(Debug, Clone, PartialEq)]
pub struct InviteSummary {
    /// The invite's id.
    pub id: Uuid,
    /// Hex pubkey of the member who minted it.
    pub created_by: String,
    /// When it was minted.
    pub created_at: DateTime<Utc>,
    /// When it expires; `None` never expires.
    pub expires_at: Option<DateTime<Utc>>,
    /// `None` is unlimited.
    pub max_uses: Option<i32>,
    /// Claims that added a member.
    pub use_count: i32,
    /// When it was revoked, if it was.
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Whether a listed invite can still be claimed, and if not, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteStatus {
    /// Claimable.
    Active,
    /// Revoked, whatever else is true.
    Revoked,
    /// Past its expiry.
    Expired,
    /// Every use consumed.
    Exhausted,
}

impl InviteStatus {
    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::Exhausted => "exhausted",
        }
    }
}

impl InviteSummary {
    /// Status at `now`, in claim precedence: revoked, expired, exhausted.
    pub fn status(&self, now: DateTime<Utc>) -> InviteStatus {
        if self.revoked_at.is_some() {
            InviteStatus::Revoked
        } else if self.expires_at.is_some_and(|at| at <= now) {
            InviteStatus::Expired
        } else if self.max_uses.is_some_and(|max| self.use_count >= max) {
            InviteStatus::Exhausted
        } else {
            InviteStatus::Active
        }
    }
}

/// A revoked invite. `revoked_at` is the first revocation's time.
#[derive(Debug, Clone, PartialEq)]
pub struct RevokedInvite {
    /// The invite's id.
    pub id: Uuid,
    /// When it was first revoked.
    pub revoked_at: DateTime<Utc>,
}

/// List `community`'s v2 invites, newest first. `before` is the
/// `(created_at, id)` of the last invite on the previous page; it need not
/// still exist. `limit` is clamped to `1..=MAX_INVITE_LIST_LIMIT`.
pub async fn list_relay_invites(
    pool: &PgPool,
    community: CommunityId,
    limit: i64,
    before: Option<(DateTime<Utc>, Uuid)>,
) -> Result<Vec<InviteSummary>> {
    let (before_at, before_id) = before.unzip();
    let rows = sqlx::query(
        "SELECT id, created_by, created_at, expires_at, max_uses, use_count, revoked_at \
         FROM relay_invites \
         WHERE community_id = $1 \
           AND ($2::timestamptz IS NULL OR (created_at, id) < ($2, $3::uuid)) \
         ORDER BY created_at DESC, id DESC LIMIT $4",
    )
    .bind(community.as_uuid())
    .bind(before_at)
    .bind(before_id)
    .bind(limit.clamp(1, MAX_INVITE_LIST_LIMIT))
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(InviteSummary {
                id: row.try_get("id")?,
                created_by: row.try_get("created_by")?,
                created_at: row.try_get("created_at")?,
                expires_at: row.try_get("expires_at")?,
                max_uses: row.try_get("max_uses")?,
                use_count: row.try_get("use_count")?,
                revoked_at: row.try_get("revoked_at")?,
            })
        })
        .collect()
}

async fn invite_exists(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
    selector: InviteSelector<'_>,
) -> Result<bool> {
    let query = match selector {
        InviteSelector::Id(id) => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM relay_invites WHERE community_id = $1 AND id = $2)",
        )
        .bind(community.as_uuid())
        .bind(id),
        InviteSelector::TokenHash(hash) => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM relay_invites \
             WHERE community_id = $1 AND token_hash = $2)",
        )
        .bind(community.as_uuid())
        .bind(hash.as_slice()),
    };
    Ok(query.fetch_one(conn).await?)
}

/// Revoke one invite in `community` on behalf of `actor`.
///
/// `Ok(None)` when no invite in `community` matches. Revoking a revoked
/// invite succeeds and keeps the first `revoked_at`. Refuses with
/// [`InviteRevokeForbidden`](crate::DbError::InviteRevokeForbidden) unless
/// `actor` is a member who is an owner or admin or who created the invite,
/// and with [`InviteRestricted`](crate::DbError::InviteRestricted) when
/// `actor` is banned or timed out. Existing members are never removed.
pub async fn revoke_relay_invite(
    pool: &PgPool,
    community: CommunityId,
    actor: &str,
    selector: InviteSelector<'_>,
) -> Result<Option<RevokedInvite>> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    // An unknown selector is answered without queueing behind claims on the
    // admission lock. These unlocked reads only refuse; the locked path
    // below re-reads everything it authorizes a write on.
    if !invite_exists(tx.conn(), community, selector).await? {
        if !crate::relay_members::is_relay_member_on(tx.conn(), community, actor).await? {
            return Err(crate::DbError::InviteRevokeForbidden);
        }
        if principal_restricted(tx.conn(), community, actor).await? {
            return Err(crate::DbError::InviteRestricted);
        }
        tx.rollback().await?;
        return Ok(None);
    }
    lock_admission(tx.conn(), community).await?;
    let role = lock_member_role(tx.conn(), community, actor).await?;
    if role.is_none() {
        return Err(crate::DbError::InviteRevokeForbidden);
    }
    if principal_restricted(tx.conn(), community, actor).await? {
        return Err(crate::DbError::InviteRestricted);
    }

    let row = match selector {
        InviteSelector::Id(id) => {
            sqlx::query(
                "SELECT id, created_by, revoked_at FROM relay_invites \
                 WHERE community_id = $1 AND id = $2 FOR UPDATE",
            )
            .bind(community.as_uuid())
            .bind(id)
            .fetch_optional(tx.conn())
            .await?
        }
        InviteSelector::TokenHash(hash) => {
            sqlx::query(
                "SELECT id, created_by, revoked_at FROM relay_invites \
                 WHERE community_id = $1 AND token_hash = $2 FOR UPDATE",
            )
            .bind(community.as_uuid())
            .bind(hash.as_slice())
            .fetch_optional(tx.conn())
            .await?
        }
    };
    let Some(row) = row else {
        tx.rollback().await?;
        return Ok(None);
    };
    let id: Uuid = row.try_get("id")?;
    let created_by: String = row.try_get("created_by")?;
    if !is_admin(role) && created_by != actor {
        return Err(crate::DbError::InviteRevokeForbidden);
    }
    if let Some(revoked_at) = row.try_get::<Option<DateTime<Utc>>, _>("revoked_at")? {
        tx.rollback().await?;
        return Ok(Some(RevokedInvite { id, revoked_at }));
    }
    let revoked_at: DateTime<Utc> = sqlx::query_scalar(
        "UPDATE relay_invites SET revoked_at = clock_timestamp() \
         WHERE community_id = $1 AND id = $2 RETURNING revoked_at",
    )
    .bind(community.as_uuid())
    .bind(id)
    .fetch_one(tx.conn())
    .await?;
    tx.commit().await?;
    Ok(Some(RevokedInvite { id, revoked_at }))
}

impl Db {
    /// See [`list_relay_invites`].
    #[datastore_span(name = "list_relay_invites", system = "postgresql")]
    pub async fn list_relay_invites(
        &self,
        community: CommunityId,
        limit: i64,
        before: Option<(DateTime<Utc>, Uuid)>,
    ) -> Result<Vec<InviteSummary>> {
        list_relay_invites(&self.pool, community, limit, before).await
    }

    /// See [`revoke_relay_invite`].
    #[datastore_span(name = "revoke_relay_invite", system = "postgresql")]
    pub async fn revoke_relay_invite(
        &self,
        community: CommunityId,
        actor: &str,
        selector: InviteSelector<'_>,
    ) -> Result<Option<RevokedInvite>> {
        revoke_relay_invite(&self.pool, community, actor, selector).await
    }
}
