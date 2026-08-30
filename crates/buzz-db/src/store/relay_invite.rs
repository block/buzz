//! Use-limited relay invite persistence (v2 opaque tokens).
//!
//! Unlike the stateless v1 HMAC invite tokens in `buzz-relay::invite_token`,
//! v2 invites are backed by durable rows in `relay_invites`. The table stores
//! only `SHA-256(code)` — never the reusable bearer secret — so a leaked
//! database does not immediately yield valid invite codes.
//!
//! Every lookup binds both `(community_id, token_hash)` to prevent cross-tenant
//! authorization seams: a code minted on tenant A presented to tenant B returns
//! `Invalid`, not a membership.
//!
//! ## Atomic redemption
//!
//! `claim_relay_invite` executes the full redemption in one PostgreSQL
//! transaction: `SELECT FOR UPDATE` on the invite row, membership insert,
//! join-policy evidence insert, and `use_count` increment all commit together.
//! `FOR UPDATE` serializes concurrent claims for one invite across relay
//! processes — exactly one claimant can win the final slot.

use buzz_core::invite::{
    encode_v2_code, hash_v2_code, MAX_INVITE_TTL_SECS, MAX_INVITE_USES, MIN_INVITE_TTL_SECS,
    V2_SECRET_LEN,
};
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Row as _, Transaction};

use crate::error::{DbError, Result};
use crate::{CommunityId, Db};

/// Outcome of a v2 invite claim. Expected invalid/expired/exhausted states are
/// typed variants so the relay layer can map them to distinct HTTP responses
/// without inspecting database errors.
#[derive(Debug, PartialEq)]
pub enum ClaimOutcome {
    /// A new relay membership or guest-channel grant was inserted.
    Joined {
        /// Whether this claim created the relay membership row.
        new_relay_member: bool,
        /// Relay role granted by the invite.
        role: String,
        /// Guest channel grant, or `None` for ordinary member invites.
        channel_id: Option<uuid::Uuid>,
        /// Post-claim use count.
        use_count: i32,
        /// Remaining slots, or `None` when the invite is unlimited.
        uses_remaining: Option<i32>,
    },
    /// The claimer was already a member. `use_count` was NOT incremented.
    AlreadyMember {
        /// Existing relay role.
        role: String,
        /// Guest channel carried by the invite, if any.
        channel_id: Option<uuid::Uuid>,
        /// Current use count (unchanged by this claim).
        use_count: i32,
        /// Remaining slots, or `None` when the invite is unlimited.
        uses_remaining: Option<i32>,
    },
    /// The invite's `expires_at` has passed.
    Expired,
    /// The invite's use budget is fully consumed.
    Exhausted,
    /// No invite row matches `(community_id, token_hash)`.
    Invalid,
    /// The invite was explicitly revoked by an authorized administrator.
    Revoked,
    /// The guest invite's channel is missing, deleted, open, or a DM.
    ChannelUnavailable,
    /// Claiming as a guest would demote the channel's last active owner.
    ChannelRoleConflict,
    /// The full relay member was explicitly removed from this channel.
    ChannelAccessRemoved,
    /// A relay administrator removed this identity from the community.
    RelayAccessRemoved,
    /// This guest identity already holds a grant for another channel.
    GuestChannelConflict,
}

/// A freshly minted v2 invite, including the plaintext code and metadata.
#[derive(Debug)]
pub struct MintedInvite {
    /// The full v2 code string (`v2.<base64url secret>`). Returned to the caller
    /// exactly once; the database stores only the SHA-256 hash.
    pub code: String,
    /// When the invite expires (UTC).
    pub expires_at: DateTime<Utc>,
    /// `None` means unlimited; `Some(n)` means at most `n` uses.
    pub max_uses: Option<i32>,
    /// Remaining uses at mint time (equals `max_uses` when bounded, `None`
    /// when unlimited).
    pub uses_remaining: Option<i32>,
    /// The invite's database-generated UUID.
    pub invite_id: uuid::Uuid,
    /// Relay role this invite grants.
    pub role: String,
    /// Guest channel grant, or `None` for ordinary member invites.
    pub channel_id: Option<uuid::Uuid>,
}

/// Metadata for an active, unclaimed guest invite.
///
/// The bearer code is intentionally absent: only the one-time mint response
/// contains it. This summary is safe to use for administrative list/revoke
/// controls without making an existing link recoverable from the database.
#[derive(Debug, PartialEq)]
pub struct ActiveGuestInvite {
    /// Community-scoped database identifier used by the revoke endpoint.
    pub invite_id: uuid::Uuid,
    /// When the invite expires (UTC).
    pub expires_at: DateTime<Utc>,
    /// When the invite was minted (UTC).
    pub created_at: DateTime<Utc>,
}

fn validate_mint_inputs(ttl_secs: u64, max_uses: Option<i32>) -> Result<()> {
    if !(MIN_INVITE_TTL_SECS..=MAX_INVITE_TTL_SECS).contains(&ttl_secs) {
        return Err(crate::error::DbError::InvalidData(format!(
            "ttl_secs must be between {MIN_INVITE_TTL_SECS} and {MAX_INVITE_TTL_SECS}"
        )));
    }

    if let Some(max_uses) = max_uses {
        if !(1..=MAX_INVITE_USES).contains(&max_uses) {
            return Err(crate::error::DbError::InvalidData(format!(
                "max_uses must be between 1 and {MAX_INVITE_USES}"
            )));
        }
    }

    Ok(())
}

fn validate_guest_invite_uses(
    max_uses: Option<i32>,
    guest_channel_id: Option<uuid::Uuid>,
) -> Result<()> {
    if guest_channel_id.is_some() && max_uses != Some(1) {
        return Err(DbError::InvalidData(
            "channel guest invites must allow exactly 1 use".to_owned(),
        ));
    }
    Ok(())
}

/// Every active guest link must remain visible in the desktop revoke list.
const MAX_ACTIVE_GUEST_INVITES_PER_CHANNEL: i64 = 100;

/// Mint a v2 invite: generate a 32-byte random secret, hash it, persist the
/// row, and return the plaintext code plus metadata.
///
/// `ttl_secs` must be in the shared invite lifetime range.
/// `max_uses` must be `None` (unlimited) or `Some(1..=10000)` for community
/// invites. Channel guest invites must use `Some(1)` so every link grants
/// access to exactly one identity.
pub async fn mint_relay_invite(
    pool: &PgPool,
    community: CommunityId,
    created_by: &str,
    ttl_secs: u64,
    max_uses: Option<i32>,
    guest_channel_id: Option<uuid::Uuid>,
) -> Result<MintedInvite> {
    validate_mint_inputs(ttl_secs, max_uses)?;
    validate_guest_invite_uses(max_uses, guest_channel_id)?;

    let mut tx = pool.begin().await?;
    crate::deletion::DeletionStore::new(pool.clone())
        .guard_transaction(&mut tx, community)
        .await?;
    let created_by_bytes = hex::decode(created_by)
        .map_err(|_| DbError::InvalidData("invite creator pubkey must be hex".to_owned()))?;
    if created_by_bytes.len() != 32 {
        return Err(DbError::InvalidData(format!(
            "invite creator pubkey must be 32 bytes, got {}",
            created_by_bytes.len()
        )));
    }

    // Serialize relay authority, moderation, and invite creation for this
    // identity. HTTP checks provide early feedback, but this transaction is the
    // authoritative boundary against concurrent demotion, removal, or banning.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!(
            "buzz_relay_invite_claim:{}:{created_by}",
            community.as_uuid()
        ))
        .execute(&mut *tx)
        .await?;
    let relay_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM relay_members \
         WHERE community_id = $1 AND pubkey = $2 FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(created_by)
    .fetch_optional(&mut *tx)
    .await?;
    if !matches!(relay_role.as_deref(), Some("owner" | "admin")) {
        return Err(DbError::AccessDenied(
            "only active relay owners and admins can create invites".to_owned(),
        ));
    }
    let banned: bool = sqlx::query_scalar(
        "SELECT COALESCE(( \
             SELECT banned AND (ban_expires_at IS NULL OR ban_expires_at > now()) \
             FROM community_bans \
             WHERE community_id = $1 AND pubkey = $2 \
         ), false)",
    )
    .bind(community.as_uuid())
    .bind(&created_by_bytes)
    .fetch_one(&mut *tx)
    .await?;
    if banned {
        return Err(DbError::AccessDenied(
            "banned relay administrators cannot create invites".to_owned(),
        ));
    }

    let guest_channel_generation = if let Some(channel_id) = guest_channel_id {
        // Guest admission is channel authority, not relay-wide authority. Use
        // the same membership lock as channel role changes so an owner/admin
        // cannot mint after concurrently losing that role.
        crate::channel_members::acquire_channel_membership_lock(&mut tx, community, channel_id)
            .await?;

        let generation: Option<i64> = sqlx::query_scalar(
            "SELECT guest_invite_generation FROM channels \
             WHERE community_id = $1 AND id = $2 \
               AND visibility = 'private' \
               AND channel_type <> 'dm' \
               AND archived_at IS NULL \
               AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(generation) = generation else {
            return Err(DbError::AccessDenied(
                "guest invites require an active private non-DM channel".to_owned(),
            ));
        };

        let authorized: bool = sqlx::query_scalar(
            "SELECT EXISTS(\
                SELECT 1 FROM channel_members \
                WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 \
                  AND role IN ('owner', 'admin') AND removed_at IS NULL\
            )",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .bind(created_by_bytes)
        .fetch_one(&mut *tx)
        .await?;
        if !authorized {
            return Err(DbError::AccessDenied(
                "only active channel owners and admins can create guest invites".to_owned(),
            ));
        }

        let active_invites: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM relay_invites \
             WHERE community_id = $1 AND channel_id = $2 AND role = 'guest' \
               AND revoked_at IS NULL AND expires_at > now() \
               AND max_uses = 1 AND use_count = 0 \
               AND channel_generation = $3",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .bind(generation)
        .fetch_one(&mut *tx)
        .await?;
        if active_invites >= MAX_ACTIVE_GUEST_INVITES_PER_CHANNEL {
            return Err(DbError::AccessDenied(format!(
                "a channel can have at most {MAX_ACTIVE_GUEST_INVITES_PER_CHANNEL} active guest links"
            )));
        }
        Some(generation)
    } else {
        None
    };

    // Generate 32 random bytes and encode as base64url — this is the secret.
    let secret: [u8; V2_SECRET_LEN] = rand::random();
    let code = encode_v2_code(&secret);
    let token_hash = hash_v2_code(&code);
    let now = Utc::now();
    let expires_at = now + chrono::Duration::seconds(ttl_secs as i64);

    let role = if guest_channel_id.is_some() {
        "guest"
    } else {
        "member"
    };
    let row = sqlx::query(
        "INSERT INTO relay_invites \
            (community_id, token_hash, role, channel_id, channel_generation, max_uses, expires_at, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         RETURNING id",
    )
    .bind(community.as_uuid())
    .bind(token_hash.as_slice())
    .bind(role)
    .bind(guest_channel_id)
    .bind(guest_channel_generation)
    .bind(max_uses)
    .bind(expires_at)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await?;
    let invite_id: uuid::Uuid = row.try_get("id")?;
    tx.commit().await?;

    Ok(MintedInvite {
        code,
        expires_at,
        max_uses,
        uses_remaining: max_uses,
        invite_id,
        role: role.to_owned(),
        channel_id: guest_channel_id,
    })
}

/// Revoke an invite under the same row lock used by claims.
///
/// Exactly one of a concurrent claim or revoke can observe the invite as live.
/// Guest invite revocation also revalidates channel-level owner/admin authority.
pub async fn revoke_relay_invite(
    pool: &PgPool,
    community: CommunityId,
    invite_id: uuid::Uuid,
    revoked_by: &str,
) -> Result<Option<String>> {
    let revoked_by_bytes = hex::decode(revoked_by)
        .map_err(|_| DbError::InvalidData("invite revoker pubkey must be hex".to_owned()))?;
    if revoked_by_bytes.len() != 32 {
        return Err(DbError::InvalidData(
            "invite revoker pubkey must be 32 bytes".to_owned(),
        ));
    }

    let mut tx = pool.begin().await?;
    let invite = sqlx::query(
        "SELECT role, channel_id, revoked_at \
         FROM relay_invites \
         WHERE community_id = $1 AND id = $2 \
         FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(invite_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(invite) = invite else {
        tx.rollback().await?;
        return Ok(None);
    };
    let role: String = invite.try_get("role")?;
    let channel_id: Option<uuid::Uuid> = invite.try_get("channel_id")?;
    let revoked_at: Option<DateTime<Utc>> = invite.try_get("revoked_at")?;
    if revoked_at.is_some() {
        tx.commit().await?;
        return Ok(Some(role));
    }

    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!(
            "buzz_relay_invite_claim:{}:{revoked_by}",
            community.as_uuid()
        ))
        .execute(&mut *tx)
        .await?;
    if let Some(channel_id) = channel_id {
        crate::channel_members::acquire_channel_membership_lock(&mut tx, community, channel_id)
            .await?;
    }

    let relay_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM relay_members \
         WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community.as_uuid())
    .bind(revoked_by)
    .fetch_optional(&mut *tx)
    .await?;
    if !matches!(relay_role.as_deref(), Some("owner" | "admin")) {
        return Err(DbError::AccessDenied(
            "only active relay owners and admins can revoke invites".to_owned(),
        ));
    }
    if role == "guest" {
        let Some(channel_id) = channel_id else {
            return Err(DbError::InvalidData(
                "guest invite is missing channel_id".to_owned(),
            ));
        };
        let authorized: bool = sqlx::query_scalar(
            "SELECT EXISTS(\
                SELECT 1 FROM channel_members \
                WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 \
                  AND role IN ('owner', 'admin') AND removed_at IS NULL\
            )",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .bind(&revoked_by_bytes)
        .fetch_one(&mut *tx)
        .await?;
        if !authorized {
            return Err(DbError::AccessDenied(
                "only active channel owners and admins can revoke guest invites".to_owned(),
            ));
        }
    }

    let banned: bool = sqlx::query_scalar(
        "SELECT COALESCE(( \
             SELECT banned AND (ban_expires_at IS NULL OR ban_expires_at > now()) \
             FROM community_bans \
             WHERE community_id = $1 AND pubkey = $2 \
         ), false)",
    )
    .bind(community.as_uuid())
    .bind(&revoked_by_bytes)
    .fetch_one(&mut *tx)
    .await?;
    if banned {
        return Err(DbError::AccessDenied(
            "banned relay administrators cannot revoke invites".to_owned(),
        ));
    }

    sqlx::query(
        "UPDATE relay_invites SET revoked_at = now(), revoked_by = $1 \
         WHERE community_id = $2 AND id = $3 AND revoked_at IS NULL",
    )
    .bind(revoked_by)
    .bind(community.as_uuid())
    .bind(invite_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(role))
}

/// List active, unclaimed guest invites for one channel.
///
/// The actor must still be an active relay owner/admin and an active
/// owner/admin of the channel. The query never returns bearer codes because
/// only their hashes are persisted.
pub async fn list_active_guest_invites(
    pool: &PgPool,
    community: CommunityId,
    channel_id: uuid::Uuid,
    actor: &str,
) -> Result<Vec<ActiveGuestInvite>> {
    let actor_bytes = hex::decode(actor)
        .map_err(|_| DbError::InvalidData("invite actor pubkey must be hex".to_owned()))?;
    if actor_bytes.len() != 32 {
        return Err(DbError::InvalidData(
            "invite actor pubkey must be 32 bytes".to_owned(),
        ));
    }

    let authorized: bool = sqlx::query_scalar(
        "SELECT EXISTS(\
            SELECT 1 \
            FROM relay_members rm \
            JOIN channel_members cm \
              ON cm.community_id = rm.community_id \
             AND cm.pubkey = decode(rm.pubkey, 'hex') \
            WHERE rm.community_id = $1 \
              AND rm.pubkey = $2 \
              AND rm.role IN ('owner', 'admin') \
              AND cm.channel_id = $3 \
              AND cm.role IN ('owner', 'admin') \
              AND cm.removed_at IS NULL \
              AND NOT EXISTS (\
                  SELECT 1 FROM community_bans cb \
                  WHERE cb.community_id = rm.community_id \
                    AND cb.pubkey = cm.pubkey \
                    AND cb.banned \
                    AND (cb.ban_expires_at IS NULL OR cb.ban_expires_at > now())\
              )\
        )",
    )
    .bind(community.as_uuid())
    .bind(actor)
    .bind(channel_id)
    .fetch_one(pool)
    .await?;
    if !authorized {
        return Err(DbError::AccessDenied(
            "only active channel owners and admins can list guest invites".to_owned(),
        ));
    }

    let rows = sqlx::query(
        "SELECT ri.id, ri.expires_at, ri.created_at \
         FROM relay_invites ri \
         JOIN channels ch \
           ON ch.community_id = ri.community_id AND ch.id = ri.channel_id \
         WHERE ri.community_id = $1 \
           AND ri.channel_id = $2 \
           AND ri.role = 'guest' \
           AND ri.revoked_at IS NULL \
           AND ri.expires_at > now() \
           AND ri.max_uses = 1 \
           AND ri.use_count = 0 \
           AND ri.channel_generation = ch.guest_invite_generation \
           AND ch.visibility = 'private' \
           AND ch.channel_type <> 'dm' \
           AND ch.archived_at IS NULL \
           AND ch.deleted_at IS NULL \
         ORDER BY ri.created_at DESC",
    )
    .bind(community.as_uuid())
    .bind(channel_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(ActiveGuestInvite {
                invite_id: row.try_get("id")?,
                expires_at: row.try_get("expires_at")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect()
}

async fn insert_policy_acceptance(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    pubkey: &str,
    policy_version: Option<&str>,
) -> Result<()> {
    if let Some(version) = policy_version {
        sqlx::query(
            "INSERT INTO join_policy_acceptances (community_id, pubkey, policy_version) \
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(pubkey)
        .bind(version)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn add_guest_channel_grant(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    channel_id: uuid::Uuid,
    pubkey_hex: &str,
    granted_by: &str,
) -> Result<bool> {
    let pubkey = hex::decode(pubkey_hex)
        .map_err(|_| DbError::InvalidData("claimer pubkey must be lowercase hex".to_owned()))?;
    if pubkey.len() != 32 {
        return Err(DbError::InvalidData(format!(
            "claimer pubkey must be 32 bytes, got {}",
            pubkey.len()
        )));
    }
    let granted_by_bytes = hex::decode(granted_by)
        .map_err(|_| DbError::InvalidData("invite creator pubkey must be hex".to_owned()))?;
    if granted_by_bytes.len() != 32 {
        return Err(DbError::InvalidData(format!(
            "invite creator pubkey must be 32 bytes, got {}",
            granted_by_bytes.len()
        )));
    }

    sqlx::query(
        "INSERT INTO channel_members \
            (community_id, channel_id, pubkey, role, invited_by) \
         VALUES ($1, $2, $3, 'guest'::member_role, $4) \
         ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET \
            removed_at = NULL, removed_by = NULL, \
            role = 'guest'::member_role, \
            invited_by = EXCLUDED.invited_by",
    )
    .bind(community.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .bind(granted_by_bytes)
    .execute(&mut **tx)
    .await?;

    let inserted = sqlx::query(
        "INSERT INTO relay_guest_channels \
            (community_id, guest_pubkey, channel_id, granted_by) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(pubkey_hex)
    .bind(channel_id)
    .bind(granted_by)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0;

    Ok(inserted)
}

async fn add_full_member_channel_access(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    channel_id: uuid::Uuid,
    pubkey_hex: &str,
    invited_by: &str,
) -> Result<bool> {
    let pubkey = hex::decode(pubkey_hex)
        .map_err(|_| DbError::InvalidData("claimer pubkey must be lowercase hex".to_owned()))?;
    let invited_by = hex::decode(invited_by)
        .map_err(|_| DbError::InvalidData("invite creator pubkey must be hex".to_owned()))?;
    if pubkey.len() != 32 || invited_by.len() != 32 {
        return Err(DbError::InvalidData(
            "invite pubkeys must each be 32 bytes".to_owned(),
        ));
    }

    let changed = sqlx::query(
        "INSERT INTO channel_members \
            (community_id, channel_id, pubkey, role, invited_by) \
         VALUES ($1, $2, $3, 'member'::member_role, $4) \
         ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET \
            removed_at = NULL, removed_by = NULL, \
            role = 'member'::member_role, \
            invited_by = EXCLUDED.invited_by \
         WHERE channel_members.removed_at IS NOT NULL \
           AND channel_members.removed_by IS NULL",
    )
    .bind(community.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .bind(invited_by)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0;

    Ok(changed)
}

fn log_claim_outcome(
    community: CommunityId,
    invite_id: Option<uuid::Uuid>,
    outcome: &'static str,
    max_uses: Option<i32>,
    use_count: Option<i32>,
) {
    tracing::info!(
        community = %community,
        invite_id = ?invite_id,
        outcome,
        max_uses = ?max_uses,
        use_count = ?use_count,
        "relay invite claim completed"
    );
}

/// Maximum rows deleted by one retention sweep so cleanup cannot monopolize
/// the invite table on a busy deployment.
const RETENTION_SWEEP_BATCH_SIZE: i64 = 1_000;

/// Delete one bounded batch of invite rows expired before `cutoff`.
///
/// The relay calls this from its leader-only periodic tick. Ordering by the
/// expiry index makes old rows drain first without turning cleanup into an
/// unbounded transaction.
pub async fn reap_expired_relay_invites(pool: &PgPool, cutoff: DateTime<Utc>) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM relay_invites \
         WHERE (community_id, id) IN (\
             SELECT community_id, id FROM relay_invites \
             WHERE expires_at < $1 \
               AND community_write_allowed(community_id) \
             ORDER BY expires_at \
             LIMIT $2\
         )",
    )
    .bind(cutoff)
    .bind(RETENTION_SWEEP_BATCH_SIZE)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// Atomically claim a v2 relay invite.
///
/// Executes the full redemption in one PostgreSQL transaction:
/// 1. Hash the presented code.
/// 2. `SELECT ... FOR UPDATE` on the invite row scoped by `(community, token_hash)`.
/// 3. If no row → `Invalid`.
/// 4. If `expires_at <= now()` → `Expired`.
/// 5. Validate a guest invite's channel, if present.
/// 6. Check existing relay membership and guest-channel grant.
/// 7. If the requested authority already exists, insert policy evidence (if
///    configured), commit, and return `AlreadyMember` without incrementing.
/// 8. If `max_uses` is set and `use_count >= max_uses`, return `Exhausted`.
/// 9. Insert relay membership and/or the guest's channel membership and grant.
/// 10. Insert join-policy acceptance evidence (if configured).
/// 11. Increment `use_count` and commit.
///
/// `FOR UPDATE` serializes concurrent claims so exactly one claimant wins the
/// final slot. Membership insertion, policy evidence, and consumption share
/// one commit — a failure in any rolls back all.
pub async fn claim_relay_invite(
    pool: &PgPool,
    community: CommunityId,
    token_hash: &[u8; 32],
    claimer_pubkey: &str,
    policy_version: Option<&str>,
) -> Result<ClaimOutcome> {
    let mut tx = pool.begin().await?;

    // 2. SELECT FOR UPDATE — lock the invite row for the duration of this txn.
    let row = sqlx::query(
        "SELECT id, role, channel_id, channel_generation, created_by, max_uses, use_count, expires_at, revoked_at \
         FROM relay_invites \
         WHERE community_id = $1 AND token_hash = $2 \
         FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(token_hash)
    .fetch_optional(&mut *tx)
    .await?;

    // 3. No matching invite.
    let Some(invite) = row else {
        tx.rollback().await?;
        log_claim_outcome(community, None, "invalid", None, None);
        return Ok(ClaimOutcome::Invalid);
    };

    let invite_id: uuid::Uuid = invite.try_get("id")?;
    let invite_role: String = invite.try_get("role")?;
    let channel_id: Option<uuid::Uuid> = invite.try_get("channel_id")?;
    let channel_generation: Option<i64> = invite.try_get("channel_generation")?;
    let created_by: String = invite.try_get("created_by")?;
    let max_uses: Option<i32> = invite.try_get("max_uses")?;
    let use_count: i32 = invite.try_get("use_count")?;
    let expires_at: DateTime<Utc> = invite.try_get("expires_at")?;
    let revoked_at: Option<DateTime<Utc>> = invite.try_get("revoked_at")?;

    if revoked_at.is_some() {
        tx.rollback().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "revoked",
            max_uses,
            Some(use_count),
        );
        return Ok(ClaimOutcome::Revoked);
    }

    // Expiry is checked before membership deliberately. An expired bearer must
    // not authorize fresh policy-acceptance evidence, even for an existing
    // member; exhausted-but-live invites remain valid for idempotent retries.
    if expires_at <= Utc::now() {
        tx.rollback().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "expired",
            max_uses,
            Some(use_count),
        );
        return Ok(ClaimOutcome::Expired);
    }

    if invite_role != "member" && invite_role != "guest" {
        return Err(DbError::InvalidData(format!(
            "invalid relay invite role: {invite_role}"
        )));
    }
    let guest_channel_id = if invite_role == "guest" {
        let Some(channel_id) = channel_id else {
            return Err(DbError::InvalidData(
                "guest invite is missing channel_id".to_owned(),
            ));
        };
        Some(channel_id)
    } else {
        None
    };

    // Different invite rows can be claimed concurrently for the same identity.
    // Serialize that identity so a guest claim cannot race and defeat a full
    // member promotion through another invite.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!(
            "buzz_relay_invite_claim:{}:{claimer_pubkey}",
            community.as_uuid()
        ))
        .execute(&mut *tx)
        .await?;

    let relay_access_removed: bool = sqlx::query_scalar(
        "SELECT EXISTS(\
            SELECT 1 FROM relay_member_invite_blocks \
            WHERE community_id = $1 AND pubkey = $2\
        )",
    )
    .bind(community.as_uuid())
    .bind(claimer_pubkey)
    .fetch_one(&mut *tx)
    .await?;
    if relay_access_removed {
        sqlx::query(
            "UPDATE relay_invites SET revoked_at = now(), revoked_by = $1 \
             WHERE community_id = $2 AND id = $3 AND revoked_at IS NULL",
        )
        .bind(claimer_pubkey)
        .bind(community.as_uuid())
        .bind(invite_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "relay_access_removed",
            max_uses,
            Some(use_count),
        );
        return Ok(ClaimOutcome::RelayAccessRemoved);
    }

    let existing_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM relay_members WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community.as_uuid())
    .bind(claimer_pubkey)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(channel_id) = guest_channel_id {
        // Serialize the last-owner check and every later channel-membership
        // read/write with add_member and remove_member. Taking this lock before
        // the check prevents a concurrent owner removal from making the
        // snapshot stale before the guest upsert demotes the claimant.
        crate::channel_members::acquire_channel_membership_lock(&mut tx, community, channel_id)
            .await?;

        // Re-check eligibility only after taking the same lock used by
        // visibility, archive, and delete transitions. A pre-lock check can go
        // stale while this claim waits and admit a guest after the channel is
        // no longer eligible.
        let current_generation: Option<i64> = sqlx::query_scalar(
            "SELECT guest_invite_generation FROM channels \
             WHERE community_id = $1 AND id = $2 \
               AND visibility = 'private' \
               AND channel_type <> 'dm' \
               AND archived_at IS NULL \
               AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .fetch_optional(&mut *tx)
        .await?;
        if current_generation.is_none() {
            tx.rollback().await?;
            log_claim_outcome(
                community,
                Some(invite_id),
                "channel_unavailable",
                max_uses,
                Some(use_count),
            );
            return Ok(ClaimOutcome::ChannelUnavailable);
        }
        if channel_generation != current_generation {
            tx.rollback().await?;
            log_claim_outcome(
                community,
                Some(invite_id),
                "channel_invite_invalidated",
                max_uses,
                Some(use_count),
            );
            return Ok(ClaimOutcome::Revoked);
        }

        if existing_role.as_deref() == Some("guest") {
            let existing_guest_channel: Option<uuid::Uuid> = sqlx::query_scalar(
                "SELECT channel_id FROM relay_guest_channels \
                 WHERE community_id = $1 AND guest_pubkey = $2",
            )
            .bind(community.as_uuid())
            .bind(claimer_pubkey)
            .fetch_optional(&mut *tx)
            .await?;
            if existing_guest_channel.is_some_and(|existing| existing != channel_id) {
                tx.rollback().await?;
                return Ok(ClaimOutcome::GuestChannelConflict);
            }
        }

        let guest_would_remove_last_owner: bool = sqlx::query_scalar(
            "SELECT EXISTS(\
                SELECT 1 FROM channel_members target \
                WHERE target.community_id = $1 \
                  AND target.channel_id = $2 \
                  AND target.pubkey = $3 \
                  AND target.role = 'owner' \
                  AND target.removed_at IS NULL \
                  AND NOT EXISTS (\
                      SELECT 1 FROM channel_members other \
                      WHERE other.community_id = target.community_id \
                        AND other.channel_id = target.channel_id \
                        AND other.role = 'owner' \
                        AND other.removed_at IS NULL \
                        AND other.pubkey <> target.pubkey\
                  )\
            )",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .bind(
            hex::decode(claimer_pubkey)
                .map_err(|_| DbError::InvalidData("claimer pubkey must be hex".to_owned()))?,
        )
        .fetch_one(&mut *tx)
        .await?;
        if existing_role.as_deref().is_none_or(|role| role == "guest")
            && guest_would_remove_last_owner
        {
            tx.rollback().await?;
            return Ok(ClaimOutcome::ChannelRoleConflict);
        }
    }

    // A full relay member claiming a channel invite still needs an active
    // membership in that private channel. A guest retry is idempotent only
    // when both the exact grant and its active channel membership exist.
    let already_authorized = match (&existing_role, guest_channel_id) {
        (Some(role), None) if role != "guest" => Some(role.clone()),
        (Some(_), None) => None,
        (Some(role), Some(channel_id)) if role != "guest" => {
            let active: bool = sqlx::query_scalar(
                "SELECT EXISTS(\
                    SELECT 1 FROM channel_members \
                    WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 \
                      AND removed_at IS NULL\
                )",
            )
            .bind(community.as_uuid())
            .bind(channel_id)
            .bind(
                hex::decode(claimer_pubkey)
                    .map_err(|_| DbError::InvalidData("claimer pubkey must be hex".to_owned()))?,
            )
            .fetch_one(&mut *tx)
            .await?;
            active.then(|| role.clone())
        }
        (Some(role), Some(channel_id)) => {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(\
                    SELECT 1 FROM relay_guest_channels rgc \
                    JOIN channel_members cm \
                      ON cm.community_id = rgc.community_id \
                     AND cm.channel_id = rgc.channel_id \
                     AND cm.pubkey = decode(rgc.guest_pubkey, 'hex') \
                     AND cm.removed_at IS NULL \
                    WHERE rgc.community_id = $1 \
                      AND rgc.guest_pubkey = $2 \
                      AND rgc.channel_id = $3\
                )",
            )
            .bind(community.as_uuid())
            .bind(claimer_pubkey)
            .bind(channel_id)
            .fetch_one(&mut *tx)
            .await?;
            exists.then(|| role.clone())
        }
        (None, _) => None,
    };

    if let Some(role) = already_authorized {
        insert_policy_acceptance(&mut tx, community, claimer_pubkey, policy_version).await?;
        tx.commit().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "already_member",
            max_uses,
            Some(use_count),
        );
        return Ok(ClaimOutcome::AlreadyMember {
            role,
            channel_id: guest_channel_id,
            use_count,
            uses_remaining: max_uses.map(|mu| mu - use_count),
        });
    }

    // A durable guest grant proves this identity already consumed this invite's
    // authority. Repair a system-removed roster row, but never reverse an
    // explicit member removal (`removed_by IS NOT NULL`). The normal removal
    // path deletes the grant atomically; this guard also fails closed for stale
    // grants left by an older relay or manual database repair.
    let guest_grant_is_repairable =
        if let (Some("guest"), Some(channel_id)) = (existing_role.as_deref(), guest_channel_id) {
            sqlx::query_scalar(
                "SELECT EXISTS(\
                    SELECT 1 FROM relay_guest_channels rgc \
                    JOIN channel_members cm \
                      ON cm.community_id = rgc.community_id \
                     AND cm.channel_id = rgc.channel_id \
                     AND cm.pubkey = decode(rgc.guest_pubkey, 'hex') \
                    WHERE rgc.community_id = $1 \
                      AND rgc.guest_pubkey = $2 \
                      AND rgc.channel_id = $3 \
                      AND cm.removed_at IS NOT NULL \
                      AND cm.removed_by IS NULL\
                )",
            )
            .bind(community.as_uuid())
            .bind(claimer_pubkey)
            .bind(channel_id)
            .fetch_one(&mut *tx)
            .await?
        } else {
            false
        };
    if let (true, Some(channel_id)) = (guest_grant_is_repairable, guest_channel_id) {
        add_guest_channel_grant(&mut tx, community, channel_id, claimer_pubkey, &created_by)
            .await?;
        insert_policy_acceptance(&mut tx, community, claimer_pubkey, policy_version).await?;
        tx.commit().await?;
        return Ok(ClaimOutcome::AlreadyMember {
            role: "guest".to_owned(),
            channel_id: Some(channel_id),
            use_count,
            uses_remaining: max_uses.map(|mu| mu - use_count),
        });
    }

    // A channel guest link is also a convenient invitation for an existing
    // full relay member, but it must never override an explicit kick. Return a
    // typed denial instead of claiming success without restoring authority.
    let full_member_was_explicitly_removed = if let (Some(role), Some(channel_id)) =
        (existing_role.as_deref(), guest_channel_id)
    {
        if role != "guest" {
            sqlx::query_scalar(
                "SELECT EXISTS(\
                        SELECT 1 FROM channel_members \
                        WHERE community_id = $1 \
                          AND channel_id = $2 \
                          AND pubkey = $3 \
                          AND removed_at IS NOT NULL \
                          AND removed_by IS NOT NULL\
                    )",
            )
            .bind(community.as_uuid())
            .bind(channel_id)
            .bind(
                hex::decode(claimer_pubkey)
                    .map_err(|_| DbError::InvalidData("claimer pubkey must be hex".to_owned()))?,
            )
            .fetch_one(&mut *tx)
            .await?
        } else {
            false
        }
    } else {
        false
    };
    if full_member_was_explicitly_removed {
        // The removed member possesses this bearer. Revoke it so switching to
        // a new identity cannot turn the same leaked link into fresh access.
        sqlx::query(
            "UPDATE relay_invites SET revoked_at = now(), revoked_by = $1 \
             WHERE community_id = $2 AND id = $3 AND revoked_at IS NULL",
        )
        .bind(claimer_pubkey)
        .bind(community.as_uuid())
        .bind(invite_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(ClaimOutcome::ChannelAccessRemoved);
    }

    if let Some(mu) = max_uses {
        if use_count >= mu {
            tx.rollback().await?;
            log_claim_outcome(
                community,
                Some(invite_id),
                "exhausted",
                max_uses,
                Some(use_count),
            );
            return Ok(ClaimOutcome::Exhausted);
        }
    }

    // The relay membership row and the channel grant share this transaction.
    // A concurrent claim through another invite can win the membership insert;
    // the exact guest grant still determines whether this invite is consumed.
    let promoted_guest = invite_role == "member" && existing_role.as_deref() == Some("guest");
    if promoted_guest {
        crate::relay_members::lock_guest_grant_channels(&mut tx, community, claimer_pubkey).await?;
    }
    let inserted = if promoted_guest {
        sqlx::query(
            "UPDATE relay_members SET role = 'member', added_by = 'invite', updated_at = now() \
             WHERE community_id = $1 AND pubkey = $2 AND role = 'guest'",
        )
        .bind(community.as_uuid())
        .bind(claimer_pubkey)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0
    } else {
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, $3, 'invite') \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(claimer_pubkey)
        .bind(&invite_role)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0
    };
    if promoted_guest {
        crate::relay_members::promote_guest_channel_grants_locked(
            &mut tx,
            community,
            claimer_pubkey,
        )
        .await?;
    }

    let full_member_channel_inserted =
        if let (Some(channel_id), Some(role)) = (guest_channel_id, existing_role.as_deref()) {
            if role != "guest" {
                add_full_member_channel_access(
                    &mut tx,
                    community,
                    channel_id,
                    claimer_pubkey,
                    &created_by,
                )
                .await?
            } else {
                false
            }
        } else {
            false
        };

    let grant_inserted = if let Some(channel_id) = guest_channel_id {
        if existing_role.as_deref().is_none_or(|role| role == "guest") {
            add_guest_channel_grant(&mut tx, community, channel_id, claimer_pubkey, &created_by)
                .await?
        } else {
            false
        }
    } else {
        false
    };

    insert_policy_acceptance(&mut tx, community, claimer_pubkey, policy_version).await?;

    let authority_inserted = if guest_channel_id.is_some() {
        grant_inserted || full_member_channel_inserted
    } else {
        inserted
    };
    if !authority_inserted {
        let role = sqlx::query_scalar(
            "SELECT role FROM relay_members WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community.as_uuid())
        .bind(claimer_pubkey)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(ClaimOutcome::AlreadyMember {
            role,
            channel_id: guest_channel_id,
            use_count,
            uses_remaining: max_uses.map(|mu| mu - use_count),
        });
    }

    let new_use_count = use_count + 1;
    sqlx::query("UPDATE relay_invites SET use_count = $1 WHERE community_id = $2 AND id = $3")
        .bind(new_use_count)
        .bind(community.as_uuid())
        .bind(invite_id)
        .execute(&mut *tx)
        .await?;

    // 11. Commit.
    tx.commit().await?;

    let new_uses_remaining = max_uses.map(|mu| mu - new_use_count);

    log_claim_outcome(
        community,
        Some(invite_id),
        "joined",
        max_uses,
        Some(new_use_count),
    );

    let granted_role = if guest_channel_id.is_some() {
        existing_role.unwrap_or(invite_role)
    } else {
        invite_role
    };

    Ok(ClaimOutcome::Joined {
        new_relay_member: inserted,
        role: granted_role,
        channel_id: guest_channel_id,
        use_count: new_use_count,
        uses_remaining: new_uses_remaining,
    })
}

impl Db {
    /// Mints a v2 use-limited relay invite. The plaintext code is returned
    /// exactly once; only its SHA-256 hash is persisted.
    ///
    /// `max_uses` is `None` for unlimited or `Some(1..=10000)`.
    /// `ttl_secs` must be in the shared invite lifetime range.
    #[datastore_span(name = "mint_relay_invite", system = "postgresql")]
    pub async fn mint_relay_invite(
        &self,
        community: CommunityId,
        created_by: &str,
        ttl_secs: u64,
        max_uses: Option<i32>,
        guest_channel_id: Option<uuid::Uuid>,
    ) -> Result<MintedInvite> {
        mint_relay_invite(
            &self.pool,
            community,
            created_by,
            ttl_secs,
            max_uses,
            guest_channel_id,
        )
        .await
    }

    /// Revoke an invite under the same row lock used by claims.
    #[datastore_span(name = "revoke_relay_invite", system = "postgresql")]
    pub async fn revoke_relay_invite(
        &self,
        community: CommunityId,
        invite_id: uuid::Uuid,
        revoked_by: &str,
    ) -> Result<Option<String>> {
        revoke_relay_invite(&self.pool, community, invite_id, revoked_by).await
    }

    /// List active, unclaimed guest invites for a channel.
    #[datastore_span(name = "list_active_guest_invites", system = "postgresql")]
    pub async fn list_active_guest_invites(
        &self,
        community: CommunityId,
        channel_id: uuid::Uuid,
        actor: &str,
    ) -> Result<Vec<ActiveGuestInvite>> {
        list_active_guest_invites(&self.pool, community, channel_id, actor).await
    }

    /// Delete one bounded batch of invites expired before `cutoff`.
    #[datastore_span(name = "reap_expired_relay_invites", system = "postgresql")]
    pub async fn reap_expired_relay_invites(&self, cutoff: DateTime<Utc>) -> Result<u64> {
        reap_expired_relay_invites(&self.pool, cutoff).await
    }

    /// Atomically claims a v2 relay invite. The full redemption (membership
    /// insert, policy evidence, use_count increment) runs in one PostgreSQL
    /// transaction with `FOR UPDATE` on the invite row.
    ///
    /// `token_hash` is the SHA-256 of the presented v2 code (32 bytes).
    #[datastore_span(name = "claim_relay_invite", system = "postgresql")]
    pub async fn claim_relay_invite(
        &self,
        community: CommunityId,
        token_hash: &[u8; 32],
        claimer_pubkey: &str,
        policy_version: Option<&str>,
    ) -> Result<ClaimOutcome> {
        claim_relay_invite(
            &self.pool,
            community,
            token_hash,
            claimer_pubkey,
            policy_version,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay_members::is_relay_member;
    use sha2::Digest;
    use sqlx::PgPool;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    async fn setup_pool() -> PgPool {
        PgPool::connect(&test_database_url())
            .await
            .expect("connect to test DB")
    }

    fn test_database_url() -> String {
        std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned())
    }

    async fn create_scratch_database(prefix: &str) -> (PgPool, String, String) {
        let admin_url = test_database_url();
        let admin = PgPool::connect(&admin_url)
            .await
            .expect("connect to test database server");
        let name = format!("{}_{}", prefix, Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create scratch database");
        let path_start = admin_url
            .rfind('/')
            .expect("database URL has a path segment");
        let scratch_url = format!("{}/{}", &admin_url[..path_start], name);
        (admin, name, scratch_url)
    }

    async fn drop_scratch_database(admin: PgPool, db: crate::Db, name: &str) {
        db.pool.close().await;
        drop(db);
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(&admin)
        .await
        .expect("drop scratch database");
        admin.close().await;
    }

    async fn make_test_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("relay-invite-test-{}.example", id.simple()))
            .execute(pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    async fn delete_test_community(pool: &PgPool, community: CommunityId) {
        let mut tx = pool.begin().await.expect("begin test cleanup");
        sqlx::query("DELETE FROM relay_invites WHERE community_id = $1")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("delete test invites");
        sqlx::query("DELETE FROM relay_members WHERE community_id = $1")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("delete test members");
        sqlx::query("DELETE FROM channels WHERE community_id = $1")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("delete test channels");
        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("delete test community");
        tx.commit().await.expect("commit test cleanup");
    }

    fn test_pubkey() -> String {
        format!("{:064x}", Uuid::new_v4().as_u128())
    }

    async fn make_private_channel(
        pool: &PgPool,
        community: CommunityId,
        creator_hex: &str,
    ) -> Uuid {
        crate::relay_members::add_relay_member(pool, community, creator_hex, "owner", None)
            .await
            .expect("add relay owner for channel invite tests");
        let creator = hex::decode(creator_hex).expect("creator hex");
        crate::channel::create_channel(
            pool,
            community,
            "client-room",
            crate::channel::ChannelType::Stream,
            crate::channel::ChannelVisibility::Private,
            None,
            &creator,
            None,
        )
        .await
        .expect("create private channel")
        .id
    }

    async fn use_count(pool: &PgPool, community: CommunityId, invite_id: Uuid) -> i32 {
        sqlx::query_scalar(
            "SELECT use_count FROM relay_invites WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(invite_id)
        .fetch_one(pool)
        .await
        .expect("read invite use_count")
    }

    #[test]
    fn mint_validation_rejects_invalid_bounds_before_database_access() {
        for (ttl, max_uses) in [
            (MIN_INVITE_TTL_SECS - 1, None),
            (MAX_INVITE_TTL_SECS + 1, None),
            (3600, Some(0)),
            (3600, Some(-1)),
            (3600, Some(MAX_INVITE_USES + 1)),
        ] {
            let error = validate_mint_inputs(ttl, max_uses).expect_err("invalid mint contract");
            assert!(matches!(error, crate::DbError::InvalidData(_)), "{error:?}");
        }
    }

    #[test]
    fn channel_guest_invites_require_exactly_one_use() {
        let channel_id = Uuid::new_v4();
        validate_guest_invite_uses(Some(1), Some(channel_id)).expect("one-use guest invite");
        for max_uses in [None, Some(2), Some(MAX_INVITE_USES)] {
            let error = validate_guest_invite_uses(max_uses, Some(channel_id))
                .expect_err("guest invite must be one-use");
            assert!(matches!(error, crate::DbError::InvalidData(_)), "{error:?}");
        }
        validate_guest_invite_uses(None, None).expect("unlimited community invite");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn mint_after_quiescing_returns_typed_fence_without_persisting() {
        let (admin, database_name, database_url) =
            create_scratch_database("relay_invite_fence").await;
        let db = crate::Db::new(&crate::DbConfig {
            database_url,
            max_connections: 5,
            min_connections: 0,
            ..crate::DbConfig::default()
        })
        .await
        .expect("connect invite deletion test DB");
        db.migrate().await.expect("migrate invite deletion test DB");
        let pool = db.pool.clone();
        let store = db.deletion_store();
        let host = format!("relay-invite-fence-{}.example", Uuid::new_v4().simple());
        let community = db
            .ensure_configured_community(&host)
            .await
            .expect("create fenced invite community")
            .id;
        let request = store
            .submit(&host, "owner", None)
            .await
            .expect("submit deletion request");
        let empty_digest = hex::encode(sha2::Sha256::digest([]));
        let inventory = crate::deletion::FrozenInventory {
            schema: store
                .inventory_schema(community)
                .await
                .expect("inventory schema"),
            storage: crate::deletion::StorageManifest {
                version: 4,
                prefixes: [
                    format!("_meta/{community}/"),
                    format!("_uploads/{community}/"),
                    format!("repos/{community}/"),
                ]
                .into_iter()
                .map(|prefix| crate::deletion::PrefixManifest {
                    prefix,
                    object_count: 0,
                    total_bytes: 0,
                    keys_digest: empty_digest.clone(),
                })
                .collect(),
            },
        };
        store
            .freeze_inventory(request.id, &inventory)
            .await
            .expect("freeze inventory");
        store
            .approve(request.id, "owner", None)
            .await
            .expect("approve deletion");
        let claim = store
            .claim_specific(
                request.id,
                "executor",
                crate::deletion::DEFAULT_LEASE_DURATION,
            )
            .await
            .expect("claim deletion")
            .expect("runnable deletion");
        store
            .begin_quiescing(&claim.lease)
            .await
            .expect("begin quiescing");

        let error = mint_relay_invite(&pool, community, "owner", 3600, Some(1), None)
            .await
            .expect_err("quiescing must reject invite minting");
        assert!(matches!(error, crate::error::DbError::AccessDenied(_)));

        let invite_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM relay_invites WHERE community_id = $1")
                .bind(community.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count relay invites");
        assert_eq!(invite_count, 0, "rejected mint must not persist an invite");

        drop(store);
        drop(pool);
        drop_scratch_database(admin, db, &database_name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bounded_claim_exhausts_and_existing_member_retry_does_not_consume() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let first = test_pubkey();
        let second = test_pubkey();
        let invite = mint_relay_invite(&pool, community, "owner", 3600, Some(1), None)
            .await
            .expect("mint bounded invite");
        let hash = hash_v2_code(&invite.code);

        assert_eq!(
            claim_relay_invite(&pool, community, &hash, &first, None)
                .await
                .expect("first claim"),
            ClaimOutcome::Joined {
                new_relay_member: true,
                role: "member".to_owned(),
                channel_id: None,
                use_count: 1,
                uses_remaining: Some(0),
            }
        );
        assert_eq!(
            claim_relay_invite(&pool, community, &hash, &first, None)
                .await
                .expect("idempotent retry"),
            ClaimOutcome::AlreadyMember {
                role: "member".to_owned(),
                channel_id: None,
                use_count: 1,
                uses_remaining: Some(0),
            }
        );
        assert_eq!(
            claim_relay_invite(&pool, community, &hash, &second, None)
                .await
                .expect("exhausted claim"),
            ClaimOutcome::Exhausted
        );
        assert_eq!(use_count(&pool, community, invite.invite_id).await, 1);
        assert!(is_relay_member(&pool, community, &first)
            .await
            .expect("first membership"));
        assert!(!is_relay_member(&pool, community, &second)
            .await
            .expect("second membership"));
        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn revoked_invite_cannot_be_claimed() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint guest invite");

        assert!(
            revoke_relay_invite(&pool, community, invite.invite_id, &creator)
                .await
                .expect("revoke invite")
                .is_some()
        );
        assert_eq!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&invite.code),
                &test_pubkey(),
                None,
            )
            .await
            .expect("claim revoked invite"),
            ClaimOutcome::Revoked
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_claims_serialize_the_final_slot() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let first = test_pubkey();
        let second = test_pubkey();
        let invite = mint_relay_invite(&pool, community, "owner", 3600, Some(1), None)
            .await
            .expect("mint bounded invite");
        let hash = hash_v2_code(&invite.code);

        let (first_outcome, second_outcome) = tokio::join!(
            claim_relay_invite(&pool, community, &hash, &first, None),
            claim_relay_invite(&pool, community, &hash, &second, None),
        );
        let outcomes = [
            first_outcome.expect("first concurrent claim"),
            second_outcome.expect("second concurrent claim"),
        ];
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, ClaimOutcome::Joined { .. }))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, ClaimOutcome::Exhausted))
                .count(),
            1
        );
        assert_eq!(use_count(&pool, community, invite.invite_id).await, 1);
        let admitted = is_relay_member(&pool, community, &first)
            .await
            .expect("first membership") as u8
            + is_relay_member(&pool, community, &second)
                .await
                .expect("second membership") as u8;
        assert_eq!(admitted, 1);
        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn expiry_and_tenant_scope_return_typed_failures() {
        let pool = setup_pool().await;
        let community_a = make_test_community(&pool).await;
        let community_b = make_test_community(&pool).await;
        let invite = mint_relay_invite(&pool, community_a, "owner", 3600, Some(2), None)
            .await
            .expect("mint invite");
        let hash = hash_v2_code(&invite.code);

        assert_eq!(
            claim_relay_invite(&pool, community_b, &hash, &test_pubkey(), None)
                .await
                .expect("cross-tenant claim"),
            ClaimOutcome::Invalid
        );

        sqlx::query(
            "UPDATE relay_invites SET expires_at = now() - interval '1 second' \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community_a.as_uuid())
        .bind(invite.invite_id)
        .execute(&pool)
        .await
        .expect("expire invite");
        assert_eq!(
            claim_relay_invite(&pool, community_a, &hash, &test_pubkey(), None)
                .await
                .expect("expired claim"),
            ClaimOutcome::Expired
        );
        assert_eq!(use_count(&pool, community_a, invite.invite_id).await, 0);
        delete_test_community(&pool, community_a).await;
        delete_test_community(&pool, community_b).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn retention_sweep_deletes_only_invites_older_than_cutoff() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let old = mint_relay_invite(&pool, community, "owner", 3600, Some(1), None)
            .await
            .expect("mint old invite");
        let recent = mint_relay_invite(&pool, community, "owner", 3600, Some(1), None)
            .await
            .expect("mint recent invite");
        let cutoff = Utc::now() - chrono::Duration::days(30);

        sqlx::query("UPDATE relay_invites SET expires_at = $1 WHERE community_id = $2 AND id = $3")
            .bind(cutoff - chrono::Duration::seconds(1))
            .bind(community.as_uuid())
            .bind(old.invite_id)
            .execute(&pool)
            .await
            .expect("age old invite");

        assert_eq!(
            reap_expired_relay_invites(&pool, cutoff)
                .await
                .expect("reap expired invites"),
            1
        );
        let remaining: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM relay_invites WHERE community_id = $1 ORDER BY id")
                .bind(community.as_uuid())
                .fetch_all(&pool)
                .await
                .expect("read remaining invites");
        assert_eq!(remaining, vec![recent.invite_id]);

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn retention_sweep_skips_quiescing_tenant_while_active_bystanders_progress() {
        let (admin, database_name, database_url) =
            create_scratch_database("relay_invite_liveness").await;
        let db = crate::Db::new(&crate::DbConfig {
            database_url,
            max_connections: 5,
            min_connections: 0,
            ..crate::DbConfig::default()
        })
        .await
        .expect("connect invite liveness database");
        db.migrate()
            .await
            .expect("migrate invite liveness database");
        let pool = db.pool.clone();
        let active_a = make_test_community(&pool).await;
        let target = make_test_community(&pool).await;
        let active_x = make_test_community(&pool).await;
        let cutoff = Utc::now();
        for community in [active_a, target, active_x] {
            sqlx::query(
                "INSERT INTO relay_invites \
                 (community_id, token_hash, expires_at, created_by) \
                 VALUES ($1, $2, $3, 'test')",
            )
            .bind(community.as_uuid())
            .bind(sha2::Sha256::digest(community.as_uuid().as_bytes()).as_slice())
            .bind(cutoff - chrono::Duration::seconds(1))
            .execute(&pool)
            .await
            .expect("seed expired invite");
        }
        let mut lifecycle = pool.begin().await.expect("begin lifecycle fixture");
        sqlx::query(
            "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                    set_config('buzz.deletion_fence_generation', '0', true)",
        )
        .bind(target.to_string())
        .execute(&mut *lifecycle)
        .await
        .expect("authorize lifecycle fixture");
        sqlx::query("UPDATE communities SET deletion_state = 'quiescing' WHERE id = $1")
            .bind(target.as_uuid())
            .execute(&mut *lifecycle)
            .await
            .expect("quiesce target");
        lifecycle.commit().await.expect("commit lifecycle fixture");

        assert_eq!(
            reap_expired_relay_invites(&pool, cutoff)
                .await
                .expect("reap active bystanders"),
            2
        );
        let remaining: Vec<Uuid> =
            sqlx::query_scalar("SELECT community_id FROM relay_invites ORDER BY community_id")
                .fetch_all(&pool)
                .await
                .expect("read remaining invite attribution");
        assert_eq!(remaining, vec![*target.as_uuid()]);

        drop(pool);
        drop_scratch_database(admin, db, &database_name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unlimited_invites_count_each_new_member() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let invite = mint_relay_invite(&pool, community, "owner", 3600, None, None)
            .await
            .expect("mint unlimited invite");
        let hash = hash_v2_code(&invite.code);

        for (expected_count, pubkey) in [(1, test_pubkey()), (2, test_pubkey())] {
            assert_eq!(
                claim_relay_invite(&pool, community, &hash, &pubkey, None)
                    .await
                    .expect("unlimited claim"),
                ClaimOutcome::Joined {
                    new_relay_member: true,
                    role: "member".to_owned(),
                    channel_id: None,
                    use_count: expected_count,
                    uses_remaining: None,
                }
            );
        }
        assert_eq!(use_count(&pool, community, invite.invite_id).await, 2);
        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn guest_claim_is_channel_scoped_idempotent_and_removed_atomically() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint guest invite");
        let hash = hash_v2_code(&invite.code);

        assert_eq!(
            claim_relay_invite(&pool, community, &hash, &guest, None)
                .await
                .expect("claim guest invite"),
            ClaimOutcome::Joined {
                new_relay_member: true,
                role: "guest".to_owned(),
                channel_id: Some(channel_id),
                use_count: 1,
                uses_remaining: Some(0),
            }
        );
        assert_eq!(
            crate::relay_members::get_guest_channel_ids(&pool, community, &guest)
                .await
                .expect("guest channels"),
            vec![channel_id]
        );
        assert_eq!(
            crate::channel::get_member_role(
                &pool,
                community,
                channel_id,
                &hex::decode(&guest).expect("guest hex"),
            )
            .await
            .expect("guest role"),
            Some("guest".to_owned())
        );
        assert!(matches!(
            claim_relay_invite(&pool, community, &hash, &guest, None)
                .await
                .expect("idempotent guest retry"),
            ClaimOutcome::AlreadyMember {
                role,
                channel_id: Some(id),
                use_count: 1,
                ..
            } if role == "guest" && id == channel_id
        ));

        assert_eq!(
            crate::relay_members::remove_relay_member(&pool, community, &guest)
                .await
                .expect("remove guest"),
            crate::relay_members::RemoveResult::Removed
        );
        assert!(
            crate::relay_members::get_guest_channel_ids(&pool, community, &guest)
                .await
                .expect("guest grants after removal")
                .is_empty()
        );
        assert_eq!(
            crate::channel::get_member_role(
                &pool,
                community,
                channel_id,
                &hex::decode(&guest).expect("guest hex"),
            )
            .await
            .expect("guest role after removal"),
            None
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn one_guest_identity_cannot_claim_a_second_channel() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let first_channel = make_private_channel(&pool, community, &creator).await;
        let second_channel = crate::channel::create_channel(
            &pool,
            community,
            "second-client-room",
            crate::channel::ChannelType::Stream,
            crate::channel::ChannelVisibility::Private,
            None,
            &hex::decode(&creator).expect("creator hex"),
            None,
        )
        .await
        .expect("create second private channel")
        .id;
        let first_invite = mint_relay_invite(
            &pool,
            community,
            &creator,
            3600,
            Some(1),
            Some(first_channel),
        )
        .await
        .expect("mint first guest invite");
        let second_invite = mint_relay_invite(
            &pool,
            community,
            &creator,
            3600,
            Some(1),
            Some(second_channel),
        )
        .await
        .expect("mint second guest invite");

        assert!(matches!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&first_invite.code),
                &guest,
                None,
            )
            .await
            .expect("claim first channel"),
            ClaimOutcome::Joined {
                channel_id: Some(id),
                ..
            } if id == first_channel
        ));
        assert_eq!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&second_invite.code),
                &guest,
                None,
            )
            .await
            .expect("claim second channel"),
            ClaimOutcome::GuestChannelConflict
        );
        assert_eq!(
            crate::relay_members::get_guest_channel_ids(&pool, community, &guest)
                .await
                .expect("guest channel"),
            vec![first_channel]
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn guest_invite_mint_requires_channel_owner_or_admin_authority() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let outsider = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;

        let error = mint_relay_invite(&pool, community, &outsider, 3600, Some(1), Some(channel_id))
            .await
            .expect_err("non-channel admin must not mint guest access");
        assert!(matches!(error, DbError::AccessDenied(ref message)
                if message.contains("channel owners and admins")));

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn invite_mint_serializes_with_concurrent_ban() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let creator_bytes = hex::decode(&creator).expect("creator hex");

        let mut ban = pool.begin().await.expect("begin ban transaction");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!(
                "buzz_relay_invite_claim:{}:{creator}",
                community.as_uuid()
            ))
            .execute(&mut *ban)
            .await
            .expect("ban acquires identity lock");
        sqlx::query(
            "INSERT INTO community_bans \
                (community_id, pubkey, banned, actor_pubkey) \
             VALUES ($1, $2, true, $3)",
        )
        .bind(community.as_uuid())
        .bind(&creator_bytes)
        .bind(&creator_bytes)
        .execute(&mut *ban)
        .await
        .expect("stage ban");

        let mint_pool = pool.clone();
        let creator_for_mint = creator.clone();
        let mut mint = tokio::spawn(async move {
            mint_relay_invite(
                &mint_pool,
                community,
                &creator_for_mint,
                3600,
                Some(1),
                Some(channel_id),
            )
            .await
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(750), &mut mint)
                .await
                .is_err(),
            "mint must wait for the concurrent moderation decision"
        );

        ban.commit().await.expect("commit ban");
        let error = tokio::time::timeout(std::time::Duration::from_secs(10), mint)
            .await
            .expect("mint proceeds after ban")
            .expect("mint task panicked")
            .expect_err("banned admin cannot mint");
        assert!(matches!(error, DbError::AccessDenied(ref message)
                if message.contains("banned relay administrators")));

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn full_member_claiming_guest_invite_gains_channel_membership_without_guest_scope() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let member = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
             VALUES ($1, $2, 'member', 'test')",
        )
        .bind(community.as_uuid())
        .bind(&member)
        .execute(&pool)
        .await
        .expect("insert full relay member");
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint channel invite");

        assert!(matches!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&invite.code),
                &member,
                None,
            )
            .await
            .expect("claim channel invite"),
            ClaimOutcome::Joined {
                new_relay_member: false,
                role,
                channel_id: Some(id),
                use_count: 1,
                ..
            } if role == "member" && id == channel_id
        ));
        assert_eq!(
            crate::channel::get_member_role(
                &pool,
                community,
                channel_id,
                &hex::decode(&member).expect("member hex"),
            )
            .await
            .expect("channel membership"),
            Some("member".to_owned())
        );
        assert!(
            crate::relay_members::get_guest_channel_ids(&pool, community, &member)
                .await
                .expect("guest grants")
                .is_empty()
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn full_member_guest_invite_never_reverses_explicit_channel_removal() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let member = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
             VALUES ($1, $2, 'member', 'test')",
        )
        .bind(community.as_uuid())
        .bind(&member)
        .execute(&pool)
        .await
        .expect("insert full relay member");

        let first_invite =
            mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
                .await
                .expect("mint first channel invite");
        assert!(matches!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&first_invite.code),
                &member,
                None,
            )
            .await
            .expect("claim first channel invite"),
            ClaimOutcome::Joined { .. }
        ));

        sqlx::query(
            "UPDATE channel_members \
             SET removed_at = now(), removed_by = $1 \
             WHERE community_id = $2 AND channel_id = $3 AND pubkey = $4",
        )
        .bind(hex::decode(&creator).expect("creator hex"))
        .bind(community.as_uuid())
        .bind(channel_id)
        .bind(hex::decode(&member).expect("member hex"))
        .execute(&pool)
        .await
        .expect("explicitly remove member");

        let second_invite =
            mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
                .await
                .expect("mint second channel invite");
        assert!(matches!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&second_invite.code),
                &member,
                None,
            )
            .await
            .expect("claim after explicit removal"),
            ClaimOutcome::ChannelAccessRemoved
        ));
        assert_eq!(
            crate::channel::get_member_role(
                &pool,
                community,
                channel_id,
                &hex::decode(&member).expect("member hex"),
            )
            .await
            .expect("channel membership"),
            None,
            "a guest invite must not reverse an explicit channel removal"
        );
        assert_eq!(
            use_count(&pool, community, second_invite.invite_id).await,
            0,
            "a denied claim must not be recorded as a successful use"
        );
        assert_eq!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&second_invite.code),
                &test_pubkey(),
                None,
            )
            .await
            .expect("reusing a revoked bearer"),
            ClaimOutcome::Revoked,
            "a kicked member's bearer must not work under a fresh identity"
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn relay_admin_removal_blocks_and_revokes_bearer_reentry() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let removed_member = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        crate::relay_members::add_relay_member(
            &pool,
            community,
            &removed_member,
            "member",
            Some(&creator),
        )
        .await
        .expect("add full relay member");
        let blocked_invite =
            mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
                .await
                .expect("mint guest invite");

        assert_eq!(
            crate::relay_members::remove_relay_member_and_block_invites(
                &pool,
                community,
                &removed_member,
            )
            .await
            .expect("administratively remove member"),
            crate::relay_members::RemoveResult::Removed
        );
        assert_eq!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&blocked_invite.code),
                &removed_member,
                None,
            )
            .await
            .expect("removed identity claim"),
            ClaimOutcome::RelayAccessRemoved
        );
        assert_eq!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&blocked_invite.code),
                &test_pubkey(),
                None,
            )
            .await
            .expect("same bearer under a fresh identity"),
            ClaimOutcome::Revoked
        );
        assert_eq!(
            use_count(&pool, community, blocked_invite.invite_id).await,
            0
        );

        crate::relay_members::add_relay_member(
            &pool,
            community,
            &removed_member,
            "member",
            Some(&creator),
        )
        .await
        .expect("explicitly re-add member");
        let replacement =
            mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
                .await
                .expect("mint replacement invite");
        assert!(matches!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&replacement.code),
                &removed_member,
                None,
            )
            .await
            .expect("claim after explicit re-add"),
            ClaimOutcome::Joined { role, .. } if role == "member"
        ));

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn active_guest_invite_list_is_authorized_and_tracks_revoke_and_claim() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let first = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint first guest invite");
        let second = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint second guest invite");

        let mut listed_ids: Vec<Uuid> =
            list_active_guest_invites(&pool, community, channel_id, &creator)
                .await
                .expect("list active guest invites")
                .into_iter()
                .map(|invite| invite.invite_id)
                .collect();
        listed_ids.sort();
        let mut expected_ids = vec![first.invite_id, second.invite_id];
        expected_ids.sort();
        assert_eq!(listed_ids, expected_ids);

        let error = list_active_guest_invites(&pool, community, channel_id, &test_pubkey())
            .await
            .expect_err("outsider cannot list guest invites");
        assert!(matches!(error, DbError::AccessDenied(_)));

        revoke_relay_invite(&pool, community, first.invite_id, &creator)
            .await
            .expect("revoke first invite");
        let remaining = list_active_guest_invites(&pool, community, channel_id, &creator)
            .await
            .expect("list after revoke");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].invite_id, second.invite_id);

        claim_relay_invite(
            &pool,
            community,
            &hash_v2_code(&second.code),
            &test_pubkey(),
            None,
        )
        .await
        .expect("claim second invite");
        assert!(
            list_active_guest_invites(&pool, community, channel_id, &creator)
                .await
                .expect("list after claim")
                .is_empty()
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn guest_claim_normalizes_stale_elevated_role_but_never_demotes_last_owner() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let former_admin = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        crate::channel::add_member(
            &pool,
            community,
            channel_id,
            &hex::decode(&former_admin).expect("admin hex"),
            buzz_core::channel::MemberRole::Admin,
            Some(&hex::decode(&creator).expect("creator hex")),
        )
        .await
        .expect("add channel admin");
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint guest invite");

        assert!(matches!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&invite.code),
                &former_admin,
                None,
            )
            .await
            .expect("claim as former admin"),
            ClaimOutcome::Joined { role, .. } if role == "guest"
        ));
        assert_eq!(
            crate::channel::get_member_role(
                &pool,
                community,
                channel_id,
                &hex::decode(&former_admin).expect("admin hex"),
            )
            .await
            .expect("normalized role"),
            Some("guest".to_owned())
        );

        assert_eq!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&invite.code),
                &creator,
                None,
            )
            .await
            .expect("last owner claim"),
            ClaimOutcome::ChannelRoleConflict
        );
        assert_eq!(
            crate::channel::get_member_role(
                &pool,
                community,
                channel_id,
                &hex::decode(&creator).expect("creator hex"),
            )
            .await
            .expect("owner role"),
            Some("owner".to_owned())
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn guest_claim_serializes_last_owner_check_with_channel_membership_writes() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint guest invite");
        let invite_hash = hash_v2_code(&invite.code);

        let mut holder = pool.begin().await.expect("begin lock holder");
        crate::channel_members::acquire_channel_membership_lock(&mut holder, community, channel_id)
            .await
            .expect("holder acquires membership key");

        let claim_pool = pool.clone();
        let mut claim = tokio::spawn(async move {
            claim_relay_invite(&claim_pool, community, &invite_hash, &creator, None).await
        });

        let blocked = tokio::time::timeout(std::time::Duration::from_millis(750), &mut claim).await;
        assert!(
            blocked.is_err(),
            "guest claim completed its last-owner check without the shared membership lock"
        );

        holder.rollback().await.expect("release membership key");
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(10), claim)
                .await
                .expect("claim proceeds after lock release")
                .expect("claim task panicked")
                .expect("claim result"),
            ClaimOutcome::ChannelRoleConflict
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn guest_claim_rechecks_channel_eligibility_after_membership_lock() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint guest invite");
        let invite_hash = hash_v2_code(&invite.code);

        let mut transition = pool.begin().await.expect("begin visibility transition");
        crate::channel_members::acquire_channel_membership_lock(
            &mut transition,
            community,
            channel_id,
        )
        .await
        .expect("transition acquires membership key");
        sqlx::query(
            "UPDATE channels SET visibility = 'open' \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .execute(&mut *transition)
        .await
        .expect("stage visibility change");

        let claim_pool = pool.clone();
        let mut claim = tokio::spawn(async move {
            claim_relay_invite(&claim_pool, community, &invite_hash, &guest, None).await
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(750), &mut claim)
                .await
                .is_err(),
            "guest claim must wait for the eligibility transition"
        );

        transition.commit().await.expect("commit visibility change");
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(10), claim)
                .await
                .expect("claim proceeds after transition")
                .expect("claim task panicked")
                .expect("claim result"),
            ClaimOutcome::ChannelUnavailable
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn guest_reclaim_reactivates_membership_after_partial_removal() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint bounded guest invite");
        let hash = hash_v2_code(&invite.code);
        assert!(matches!(
            claim_relay_invite(&pool, community, &hash, &guest, None)
                .await
                .expect("initial claim"),
            ClaimOutcome::Joined {
                use_count: 1,
                uses_remaining: Some(0),
                ..
            }
        ));
        sqlx::query(
            "UPDATE channel_members \
             SET removed_at = now(), removed_by = NULL \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .bind(hex::decode(&guest).expect("guest hex"))
        .execute(&pool)
        .await
        .expect("simulate system-only partial removal");

        assert!(matches!(
            claim_relay_invite(&pool, community, &hash, &guest, None)
                .await
                .expect("reclaim guest invite"),
            ClaimOutcome::AlreadyMember {
                role,
                channel_id: Some(id),
                use_count: 1,
                uses_remaining: Some(0),
                ..
            } if role == "guest" && id == channel_id
        ));
        assert_eq!(
            crate::relay_members::get_guest_channel_ids(&pool, community, &guest)
                .await
                .expect("reactivated guest channels"),
            vec![channel_id]
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn guest_reclaim_never_reverses_an_explicit_channel_removal() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint guest invite");
        let hash = hash_v2_code(&invite.code);
        claim_relay_invite(&pool, community, &hash, &guest, None)
            .await
            .expect("initial claim");

        // Simulate the stale-grant state an older relay could leave if channel
        // removal committed but grant cleanup failed.
        sqlx::query(
            "UPDATE channel_members \
             SET removed_at = now(), removed_by = $1 \
             WHERE community_id = $2 AND channel_id = $3 AND pubkey = $4",
        )
        .bind(hex::decode(&creator).expect("creator hex"))
        .bind(community.as_uuid())
        .bind(channel_id)
        .bind(hex::decode(&guest).expect("guest hex"))
        .execute(&pool)
        .await
        .expect("simulate explicit removal with stale grant");

        assert_eq!(
            claim_relay_invite(&pool, community, &hash, &guest, None)
                .await
                .expect("reclaim after explicit removal"),
            ClaimOutcome::Exhausted
        );
        assert!(
            crate::relay_members::get_guest_channel_ids(&pool, community, &guest)
                .await
                .expect("guest channels")
                .is_empty(),
            "an explicit removal must remain revoked"
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn channel_removal_revokes_guest_admission_when_last_grant_is_removed() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint guest invite");
        let unused = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint second guest invite");
        claim_relay_invite(&pool, community, &hash_v2_code(&invite.code), &guest, None)
            .await
            .expect("claim guest invite");

        crate::channel::remove_member(
            &pool,
            community,
            channel_id,
            &hex::decode(&guest).expect("guest hex"),
            &hex::decode(&creator).expect("creator hex"),
        )
        .await
        .expect("remove guest from channel");

        assert!(
            crate::relay_members::get_guest_channel_ids(&pool, community, &guest)
                .await
                .expect("active guest channels")
                .is_empty()
        );
        assert!(
            !crate::relay_members::revoke_guest_channel(&pool, community, &guest, channel_id)
                .await
                .expect("grant was already revoked atomically")
        );
        assert!(
            crate::relay_members::get_relay_member(&pool, community, &guest)
                .await
                .expect("relay guest lookup")
                .is_none(),
            "the last channel grant must also remove relay admission"
        );
        assert_eq!(
            claim_relay_invite(&pool, community, &hash_v2_code(&unused.code), &guest, None,)
                .await
                .expect("claim second bearer after administrative removal"),
            ClaimOutcome::RelayAccessRemoved,
            "an administrator-removed guest must not recreate admission with another link"
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn voluntary_guest_leave_does_not_block_a_fresh_invite() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let guest_bytes = hex::decode(&guest).expect("guest hex");
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let first = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint first guest invite");
        let second = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint second guest invite");

        claim_relay_invite(&pool, community, &hash_v2_code(&first.code), &guest, None)
            .await
            .expect("claim first guest invite");
        crate::channel::remove_member(&pool, community, channel_id, &guest_bytes, &guest_bytes)
            .await
            .expect("guest leaves voluntarily");

        assert!(matches!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&second.code),
                &guest,
                None,
            )
            .await
            .expect("claim fresh invite after voluntary leave"),
            ClaimOutcome::Joined {
                role,
                channel_id: Some(id),
                ..
            } if role == "guest" && id == channel_id
        ));

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn archive_and_unarchive_does_not_restore_guest_authority() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint guest invite");
        let unused = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint unused guest invite");
        claim_relay_invite(&pool, community, &hash_v2_code(&invite.code), &guest, None)
            .await
            .expect("claim guest invite");

        let revoked = crate::channel::archive_channel(&pool, community, channel_id)
            .await
            .expect("archive channel");
        assert_eq!(revoked, vec![hex::decode(&guest).expect("guest hex")]);
        crate::channel::unarchive_channel(&pool, community, channel_id)
            .await
            .expect("unarchive channel");

        assert!(
            crate::relay_members::get_guest_channel_ids(&pool, community, &guest)
                .await
                .expect("guest channels")
                .is_empty()
        );
        assert!(
            crate::relay_members::get_relay_member(&pool, community, &guest)
                .await
                .expect("relay guest")
                .is_none()
        );
        assert_eq!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&unused.code),
                &test_pubkey(),
                None,
            )
            .await
            .expect("claim pre-archive link after unarchive"),
            ClaimOutcome::Revoked,
            "unarchive must not revive an unused pre-archive bearer"
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn deleting_channel_revokes_guest_authority() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint guest invite");
        claim_relay_invite(&pool, community, &hash_v2_code(&invite.code), &guest, None)
            .await
            .expect("claim guest invite");

        let (deleted, revoked) = crate::channel::soft_delete_channel(&pool, community, channel_id)
            .await
            .expect("delete channel");
        assert!(deleted);
        assert_eq!(revoked, vec![hex::decode(&guest).expect("guest hex")]);
        assert!(
            crate::relay_members::get_relay_member(&pool, community, &guest)
                .await
                .expect("relay guest")
                .is_none()
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn revoking_guest_grant_also_removes_channel_roster_entry() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint guest invite");
        claim_relay_invite(&pool, community, &hash_v2_code(&invite.code), &guest, None)
            .await
            .expect("claim guest invite");

        assert!(
            crate::relay_members::revoke_guest_channel(&pool, community, &guest, channel_id)
                .await
                .expect("revoke guest channel")
        );
        assert!(
            crate::channel::get_member_role(
                &pool,
                community,
                channel_id,
                &hex::decode(&guest).expect("guest hex"),
            )
            .await
            .expect("channel membership")
            .is_none(),
            "revocation must remove the guest from the channel roster"
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn making_channel_open_atomically_revokes_guests_without_rearming() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let guest_bytes = hex::decode(&guest).expect("guest hex");
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint guest invite");
        let unused = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint unused guest invite");
        claim_relay_invite(&pool, community, &hash_v2_code(&invite.code), &guest, None)
            .await
            .expect("claim guest invite");

        assert_eq!(
            crate::relay_members::open_channel_and_revoke_guests(&pool, community, channel_id,)
                .await
                .expect("open channel and revoke guests"),
            vec![guest_bytes.clone()]
        );
        assert!(
            crate::relay_members::get_relay_member(&pool, community, &guest)
                .await
                .expect("relay member after open")
                .is_none()
        );
        assert!(
            crate::channel::get_member_role(&pool, community, channel_id, &guest_bytes)
                .await
                .expect("channel membership after open")
                .is_none()
        );

        crate::channel::update_channel(
            &pool,
            community,
            channel_id,
            crate::channel::ChannelUpdate {
                visibility: Some("private".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("make channel private again");
        assert!(
            crate::relay_members::get_guest_channel_ids(&pool, community, &guest)
                .await
                .expect("guest channels after re-private")
                .is_empty(),
            "old guest authority must not silently return"
        );
        assert_eq!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&unused.code),
                &test_pubkey(),
                None,
            )
            .await
            .expect("claim pre-open link after re-private"),
            ClaimOutcome::Revoked,
            "making a channel private again must not revive an old bearer"
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn member_invite_promotes_an_existing_guest() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let guest_invite =
            mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
                .await
                .expect("mint guest invite");
        claim_relay_invite(
            &pool,
            community,
            &hash_v2_code(&guest_invite.code),
            &guest,
            None,
        )
        .await
        .expect("claim guest invite");

        let member_invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), None)
            .await
            .expect("mint member invite");
        assert!(matches!(
            claim_relay_invite(
                &pool,
                community,
                &hash_v2_code(&member_invite.code),
                &guest,
                None,
            )
            .await
            .expect("claim member invite"),
            ClaimOutcome::Joined {
                role,
                channel_id: None,
                use_count: 1,
                ..
            } if role == "member"
        ));
        assert_eq!(
            crate::relay_members::get_relay_member(&pool, community, &guest)
                .await
                .expect("relay member")
                .expect("member row")
                .role,
            "member"
        );
        assert!(
            crate::relay_members::get_guest_channel_ids(&pool, community, &guest)
                .await
                .expect("guest grants after promotion")
                .is_empty()
        );
        assert_eq!(
            crate::channel::get_member_role(
                &pool,
                community,
                channel_id,
                &hex::decode(&guest).expect("guest hex"),
            )
            .await
            .expect("channel membership"),
            Some("member".to_owned()),
            "promotion preserves private-channel access as a normal member"
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn relay_role_promotion_cleans_guest_grants_and_preserves_channel_access() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let guest = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
            .await
            .expect("mint guest invite");
        claim_relay_invite(&pool, community, &hash_v2_code(&invite.code), &guest, None)
            .await
            .expect("claim guest invite");

        assert!(
            crate::relay_members::update_relay_member_role(&pool, community, &guest, "member",)
                .await
                .expect("promote relay guest")
        );
        assert!(
            crate::relay_members::get_guest_channel_ids(&pool, community, &guest)
                .await
                .expect("guest grants after role promotion")
                .is_empty()
        );
        assert_eq!(
            crate::channel::get_member_role(
                &pool,
                community,
                channel_id,
                &hex::decode(&guest).expect("guest hex"),
            )
            .await
            .expect("channel membership"),
            Some("member".to_owned())
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_guest_and_member_invites_cannot_lose_full_member_promotion() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = test_pubkey();
        let claimer = test_pubkey();
        let channel_id = make_private_channel(&pool, community, &creator).await;
        let guest_invite =
            mint_relay_invite(&pool, community, &creator, 3600, Some(1), Some(channel_id))
                .await
                .expect("mint guest invite");
        let member_invite = mint_relay_invite(&pool, community, &creator, 3600, Some(1), None)
            .await
            .expect("mint member invite");
        let guest_hash = hash_v2_code(&guest_invite.code);
        let member_hash = hash_v2_code(&member_invite.code);

        let (guest_outcome, member_outcome) = tokio::join!(
            claim_relay_invite(&pool, community, &guest_hash, &claimer, None),
            claim_relay_invite(&pool, community, &member_hash, &claimer, None),
        );
        assert!(matches!(
            guest_outcome.expect("guest claim"),
            ClaimOutcome::Joined { .. }
        ));
        assert!(matches!(
            member_outcome.expect("member claim"),
            ClaimOutcome::Joined { role, .. } if role == "member"
        ));
        assert_eq!(
            crate::relay_members::get_relay_member(&pool, community, &claimer)
                .await
                .expect("relay member lookup")
                .expect("relay member")
                .role,
            "member"
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn policy_evidence_failure_rolls_back_membership_and_consumption() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let pubkey = test_pubkey();
        let invite = mint_relay_invite(&pool, community, "owner", 3600, Some(1), None)
            .await
            .expect("mint bounded invite");
        let hash = hash_v2_code(&invite.code);

        let error = claim_relay_invite(&pool, community, &hash, &pubkey, Some("too-short"))
            .await
            .expect_err("policy CHECK must reject an invalid version");
        assert!(matches!(error, crate::DbError::Sqlx(_)), "{error:?}");
        assert!(!is_relay_member(&pool, community, &pubkey)
            .await
            .expect("membership after rollback"));
        assert_eq!(use_count(&pool, community, invite.invite_id).await, 0);

        assert!(matches!(
            claim_relay_invite(&pool, community, &hash, &pubkey, None)
                .await
                .expect("claim after rollback"),
            ClaimOutcome::Joined { use_count: 1, .. }
        ));
        delete_test_community(&pool, community).await;
    }
}
