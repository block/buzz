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
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::error::Result;
use crate::relay_members::{write_nip43_membership_snapshot, MembershipWrite};
use crate::{CommunityId, Db};

mod claim_owner;
mod revocation;
pub use claim_owner::{claim_relay_membership_with_owner, MembershipClaimResult};
pub(crate) use revocation::{revoke_banned_issuer_invites, revoke_restricted_agent_invites};

/// Outcome of a v2 invite claim. Expected invalid/expired/exhausted states are
/// typed variants so the relay layer can map them to distinct HTTP responses
/// without inspecting database errors.
#[derive(Debug, PartialEq)]
pub enum ClaimOutcome {
    /// A new relay member was inserted. `use_count` is the post-increment count;
    /// `uses_remaining` is `None` for unlimited invites.
    Joined {
        /// Post-claim use count.
        use_count: i32,
        /// Remaining slots, or `None` when the invite is unlimited.
        uses_remaining: Option<i32>,
    },
    /// The claimer was already a member. `use_count` was NOT incremented.
    AlreadyMember {
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
    /// The issuer's invitation was permanently revoked.
    Revoked,
    /// The issuer or its owner is currently banned or timed out.
    IssuerRestricted,
    /// The claimant or its verified owner is banned or timed out.
    Restricted,
}

/// A completed claim and whether its committed transaction first recorded an owner.
pub struct InviteClaimResult {
    /// Membership or denial outcome.
    pub outcome: ClaimOutcome,
    /// Notify ownerless sockets only after a successful commit created the link.
    pub owner_link_created: bool,
    /// User rows first created by this successful transaction.
    pub users_created: u64,
}

impl From<ClaimOutcome> for InviteClaimResult {
    fn from(outcome: ClaimOutcome) -> Self {
        Self {
            outcome,
            owner_link_created: false,
            users_created: 0,
        }
    }
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

fn is_canonical_invite_pubkey(pubkey: &str) -> bool {
    pubkey.len() == 64
        && pubkey
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Mint a v2 invite: generate a 32-byte random secret, hash it, persist the
/// row, and return the plaintext code plus metadata.
///
/// `ttl_secs` must be in the shared invite lifetime range.
/// `max_uses` must be `None` (unlimited) or `Some(1..=10000)`.
pub async fn mint_relay_invite(
    pool: &PgPool,
    community: CommunityId,
    created_by: &str,
    ttl_secs: u64,
    max_uses: Option<i32>,
) -> Result<MintedInvite> {
    validate_mint_inputs(ttl_secs, max_uses)?;
    if !is_canonical_invite_pubkey(created_by) {
        return Err(crate::error::DbError::InvalidData(
            "invalid invitation issuer".into(),
        ));
    }

    // Generate 32 random bytes and encode as base64url — this is the secret.
    let secret: [u8; V2_SECRET_LEN] = rand::random();
    let code = encode_v2_code(&secret);
    let token_hash = hash_v2_code(&code);
    let now = Utc::now();
    let expires_at = now + chrono::Duration::seconds(ttl_secs as i64);

    // Mint a v2 opaque invite inside the same lifecycle gate as every other
    // community-scoped database write. The trigger remains the final backstop,
    // but this typed guard keeps a quiescing community from surfacing as an
    // opaque SQLSTATE/HTTP 500 at the API boundary.
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    lock_admission(tx.conn(), community).await?;
    if principal_restricted(tx.conn(), community, created_by).await? {
        return Err(crate::DbError::InviteRestricted);
    }
    let row = sqlx::query(
        "INSERT INTO relay_invites \
         (community_id, token_hash, max_uses, expires_at, created_by, legacy_admission) \
         VALUES ($1, $2, $3, $4, $5, false) \
         RETURNING id",
    )
    .bind(community.as_uuid())
    .bind(token_hash.as_slice())
    .bind(max_uses)
    .bind(expires_at)
    .bind(created_by)
    .fetch_one(tx.conn())
    .await?;
    tx.commit().await?;

    let invite_id: uuid::Uuid = row.try_get("id")?;

    Ok(MintedInvite {
        code,
        expires_at,
        max_uses,
        uses_remaining: max_uses,
        invite_id,
    })
}

/// Lock invitation admission against community restriction and owner writes.
/// Call before reading restrictions or locking an invite row, and hold to commit.
/// A joined claim holds it through its NIP-43 membership snapshot, so mint, ban,
/// timeout and first owner link wait for that snapshot, which grows with the
/// member count. Claims reject unknown codes before taking it.
pub(crate) async fn lock_admission(
    tx: &mut sqlx::PgConnection,
    community: CommunityId,
) -> Result<()> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(\
         hashtextextended('invite-admission:' || $1::text, 0))",
    )
    .bind(community.as_uuid())
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Authoritative, transaction-local lookup; nonmembers need no user row.
pub(crate) async fn principal_restricted(
    tx: &mut sqlx::PgConnection,
    community: CommunityId,
    pubkey: &str,
) -> Result<bool> {
    if !is_canonical_invite_pubkey(pubkey) {
        return Err(crate::DbError::InvalidData(
            "invalid invitation principal".into(),
        ));
    }
    let bytes = hex::decode(pubkey)
        .map_err(|_| crate::DbError::InvalidData("invalid invitation principal".into()))?;
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM community_bans \
         WHERE community_id = $1 AND pubkey IN (\
             SELECT $2::bytea UNION ALL \
             SELECT agent_owner_pubkey FROM users WHERE community_id = $1 AND pubkey = $2\
         ) AND ((banned AND (ban_expires_at IS NULL OR ban_expires_at > clock_timestamp())) \
                OR muted_until > clock_timestamp()))",
    )
    .bind(community.as_uuid())
    .bind(bytes)
    .fetch_one(&mut *tx)
    .await?)
}

/// Whether a restriction row permanently revokes an invite from this principal.
/// Unrepaired legacy rows treat every retained restriction as history; the
/// maintenance repair clears that marker after its historical decision. New
/// and repaired rows only treat a currently active ban as permanent revocation.
async fn has_revoking_restriction(
    tx: &mut sqlx::PgConnection,
    community: CommunityId,
    pubkey: &[u8],
    legacy_admission: bool,
) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM community_bans restriction \
         WHERE restriction.community_id = $1 \
           AND (restriction.pubkey = $2 OR restriction.pubkey = ( \
               SELECT agent_owner_pubkey FROM users \
               WHERE community_id = $1 AND pubkey = $2)) \
           AND ($3 OR (restriction.banned \
               AND (restriction.ban_expires_at IS NULL \
                    OR restriction.ban_expires_at > clock_timestamp()))))",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(legacy_admission)
    .fetch_one(&mut *tx)
    .await?)
}

async fn persist_claim_revocation(
    tx: &mut sqlx::PgConnection,
    community: CommunityId,
    invite_id: uuid::Uuid,
) -> Result<()> {
    sqlx::query(
        "UPDATE relay_invites SET revoked_at = clock_timestamp() \
         WHERE community_id = $1 AND id = $2 AND revoked_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(invite_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
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
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Maintenance,
    )
    .await?;
    // Keep the original global expiry order and 1,000-row ceiling. The state
    // read only narrows candidates; admission below is the write authority.
    // Exclude fenced tenants so they cannot fill the LIMIT page.
    let candidates = sqlx::query(
        "SELECT invite.community_id, invite.id FROM relay_invites invite \
         JOIN communities community ON community.id = invite.community_id \
         WHERE invite.expires_at < $1 AND community.deletion_state = 'active' \
           AND community.deleted_at IS NULL \
         ORDER BY invite.expires_at LIMIT $2",
    )
    .bind(cutoff)
    .bind(RETENTION_SWEEP_BATCH_SIZE)
    .fetch_all(&mut *connection)
    .await?;
    drop(connection);

    let mut ordered_communities = Vec::new();
    let mut ids_by_community = std::collections::HashMap::<Uuid, Vec<Uuid>>::new();
    for row in candidates {
        let community: Uuid = row.try_get("community_id")?;
        let id: Uuid = row.try_get("id")?;
        if !ids_by_community.contains_key(&community) {
            ordered_communities.push(community);
        }
        ids_by_community.entry(community).or_default().push(id);
    }

    let ids_by_community = &ids_by_community;
    let deleted = crate::sweep_admitted_communities(
        pool,
        ordered_communities,
        crate::observability::WriterOperation::Maintenance,
        "relay_invite_reap",
        |id, mut tx| async move {
            let result = sqlx::query(
                "DELETE FROM relay_invites WHERE community_id = $1 AND id = ANY($2) AND expires_at < $3",
            )
            .bind(id)
            .bind(&ids_by_community[&id])
            .bind(cutoff)
            .execute(tx.conn())
            .await?;
            tx.commit().await?;
            Ok(result.rows_affected())
        },
    )
    .await;
    Ok(deleted.into_iter().sum())
}

/// Atomically claim a v2 relay invite.
///
/// Executes the full redemption in one PostgreSQL transaction:
/// 1. Hash the presented code.
/// 2. `SELECT ... FOR UPDATE` on the invite row scoped by `(community, token_hash)`.
/// 3. If no row → `Invalid`.
/// 4. If `expires_at <= now()` → `Expired`.
/// 5. Check existing membership.
/// 6. If already a member → insert policy evidence (if configured), commit,
///    return `AlreadyMember` (no increment).
/// 7. If `max_uses` is set and `use_count >= max_uses` → `Exhausted`.
/// 8. Insert relay member with role `member`, `added_by = 'invite'`.
/// 9. Insert join-policy acceptance evidence (if configured).
/// 10. Increment `use_count`.
/// 11. Commit.
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
    relay_keypair: &nostr::Keys,
) -> Result<MembershipWrite<ClaimOutcome>> {
    let write = claim_relay_invite_with_owner(
        pool,
        community,
        token_hash,
        claimer_pubkey,
        policy_version,
        None,
        relay_keypair,
    )
    .await?;
    Ok(MembershipWrite {
        outcome: write.outcome.outcome,
        snapshot: write.snapshot,
    })
}

/// Claim an invite with a caller-verified owner while keeping restriction checks,
/// owner persistence, membership, policy evidence, invite consumption, and the
/// NIP-43 membership snapshot in one admitted transaction.
pub async fn claim_relay_invite_with_owner(
    pool: &PgPool,
    community: CommunityId,
    token_hash: &[u8; 32],
    claimer_pubkey: &str,
    policy_version: Option<&str>,
    verified_owner: Option<&str>,
    relay_keypair: &nostr::Keys,
) -> Result<MembershipWrite<InviteClaimResult>> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;

    // Unknown codes are free to forge, so reject them after the tenant fence
    // but before the community-wide admission lock; the locked read below
    // stays authoritative.
    let known: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM relay_invites \
         WHERE community_id = $1 AND token_hash = $2)",
    )
    .bind(community.as_uuid())
    .bind(token_hash)
    .fetch_one(tx.conn())
    .await?;
    if !known {
        tx.rollback().await?;
        log_claim_outcome(community, None, "invalid", None, None);
        return Ok(MembershipWrite::unchanged(ClaimOutcome::Invalid.into()));
    }

    lock_admission(tx.conn(), community).await?;
    if claim_owner::claimant_restricted(tx.conn(), community, claimer_pubkey, verified_owner)
        .await?
    {
        tx.rollback().await?;
        log_claim_outcome(community, None, "restricted", None, None);
        return Ok(MembershipWrite::unchanged(ClaimOutcome::Restricted.into()));
    }

    // `FOR UPDATE` serializes concurrent claims so exactly one claimant wins
    // the final slot, while admission serializes issuer/owner restriction changes.
    let row = sqlx::query(
        "SELECT id, max_uses, use_count, expires_at, created_by, revoked_at, legacy_admission \
         FROM relay_invites \
         WHERE community_id = $1 AND token_hash = $2 \
         FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(token_hash)
    .fetch_optional(tx.conn())
    .await?;

    let Some(invite) = row else {
        tx.rollback().await?;
        log_claim_outcome(community, None, "invalid", None, None);
        return Ok(MembershipWrite::unchanged(ClaimOutcome::Invalid.into()));
    };

    let invite_id: uuid::Uuid = invite.try_get("id")?;
    let max_uses: Option<i32> = invite.try_get("max_uses")?;
    let use_count: i32 = invite.try_get("use_count")?;
    let expires_at: DateTime<Utc> = invite.try_get("expires_at")?;
    if expires_at <= Utc::now() {
        tx.rollback().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "expired",
            max_uses,
            Some(use_count),
        );
        return Ok(MembershipWrite::unchanged(ClaimOutcome::Expired.into()));
    }

    let revoked_at: Option<DateTime<Utc>> = invite.try_get("revoked_at")?;
    let issuer: String = invite.try_get("created_by")?;
    let legacy_admission: bool = invite.try_get("legacy_admission")?;
    if revoked_at.is_some() {
        tx.rollback().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "revoked",
            max_uses,
            Some(use_count),
        );
        return Ok(MembershipWrite::unchanged(ClaimOutcome::Revoked.into()));
    }
    if !is_canonical_invite_pubkey(&issuer) {
        if legacy_admission {
            persist_claim_revocation(tx.conn(), community, invite_id).await?;
            tx.commit().await?;
            log_claim_outcome(
                community,
                Some(invite_id),
                "revoked",
                max_uses,
                Some(use_count),
            );
            return Ok(MembershipWrite::unchanged(ClaimOutcome::Revoked.into()));
        }
        tx.rollback().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "invalid",
            max_uses,
            Some(use_count),
        );
        return Ok(MembershipWrite::unchanged(ClaimOutcome::Invalid.into()));
    }

    let issuer_bytes = hex::decode(&issuer)
        .map_err(|_| crate::DbError::InvalidData("invalid invitation issuer".into()))?;
    if has_revoking_restriction(tx.conn(), community, &issuer_bytes, legacy_admission).await? {
        persist_claim_revocation(tx.conn(), community, invite_id).await?;
        tx.commit().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "revoked",
            max_uses,
            Some(use_count),
        );
        return Ok(MembershipWrite::unchanged(ClaimOutcome::Revoked.into()));
    }
    if principal_restricted(tx.conn(), community, &issuer).await? {
        tx.rollback().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "issuer_restricted",
            max_uses,
            Some(use_count),
        );
        return Ok(MembershipWrite::unchanged(
            ClaimOutcome::IssuerRestricted.into(),
        ));
    }

    let uses_remaining = || max_uses.map(|mu| mu - use_count);
    let existing =
        sqlx::query("SELECT 1 FROM relay_members WHERE community_id = $1 AND pubkey = $2")
            .bind(community.as_uuid())
            .bind(claimer_pubkey)
            .fetch_optional(tx.conn())
            .await?;

    if existing.is_some() {
        if let Some(version) = policy_version {
            sqlx::query(
                "INSERT INTO join_policy_acceptances (community_id, pubkey, policy_version) \
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(community.as_uuid())
            .bind(claimer_pubkey)
            .bind(version)
            .execute(tx.conn())
            .await?;
        }
        let owner =
            claim_owner::persist_claim_owner(tx.conn(), community, claimer_pubkey, verified_owner)
                .await?;
        tx.commit().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "already_member",
            max_uses,
            Some(use_count),
        );
        return Ok(MembershipWrite::unchanged(InviteClaimResult {
            outcome: ClaimOutcome::AlreadyMember {
                use_count,
                uses_remaining: uses_remaining(),
            },
            owner_link_created: owner.link_created,
            users_created: owner.users_created,
        }));
    }

    if max_uses.is_some_and(|max_uses| use_count >= max_uses) {
        tx.rollback().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "exhausted",
            max_uses,
            Some(use_count),
        );
        return Ok(MembershipWrite::unchanged(ClaimOutcome::Exhausted.into()));
    }

    let inserted = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, 'member', 'invite') \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(claimer_pubkey)
    .execute(tx.conn())
    .await?
    .rows_affected()
        > 0;

    if let Some(version) = policy_version {
        sqlx::query(
            "INSERT INTO join_policy_acceptances (community_id, pubkey, policy_version) \
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(claimer_pubkey)
        .bind(version)
        .execute(tx.conn())
        .await?;
    }

    if !inserted {
        let owner =
            claim_owner::persist_claim_owner(tx.conn(), community, claimer_pubkey, verified_owner)
                .await?;
        tx.commit().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "already_member",
            max_uses,
            Some(use_count),
        );
        return Ok(MembershipWrite::unchanged(InviteClaimResult {
            outcome: ClaimOutcome::AlreadyMember {
                use_count,
                uses_remaining: uses_remaining(),
            },
            owner_link_created: owner.link_created,
            users_created: owner.users_created,
        }));
    }

    let new_use_count = use_count + 1;
    sqlx::query("UPDATE relay_invites SET use_count = $1 WHERE community_id = $2 AND id = $3")
        .bind(new_use_count)
        .bind(community.as_uuid())
        .bind(invite_id)
        .execute(tx.conn())
        .await?;

    let owner =
        claim_owner::persist_claim_owner(tx.conn(), community, claimer_pubkey, verified_owner)
            .await?;
    let snapshot = write_nip43_membership_snapshot(&mut tx, relay_keypair).await?;
    tx.commit().await?;

    let new_uses_remaining = max_uses.map(|mu| mu - new_use_count);
    log_claim_outcome(
        community,
        Some(invite_id),
        "joined",
        max_uses,
        Some(new_use_count),
    );

    Ok(MembershipWrite {
        outcome: InviteClaimResult {
            outcome: ClaimOutcome::Joined {
                use_count: new_use_count,
                uses_remaining: new_uses_remaining,
            },
            owner_link_created: owner.link_created,
            users_created: owner.users_created,
        },
        snapshot: Some(snapshot),
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
    ) -> Result<MintedInvite> {
        mint_relay_invite(&self.pool, community, created_by, ttl_secs, max_uses).await
    }

    /// Claim an invite and persist caller-verified ownership atomically.
    #[datastore_span(name = "claim_relay_invite_with_owner", system = "postgresql")]
    pub async fn claim_relay_invite_with_owner(
        &self,
        community: CommunityId,
        token_hash: &[u8; 32],
        claimer_pubkey: &str,
        policy_version: Option<&str>,
        verified_owner: Option<&str>,
        relay_keypair: &nostr::Keys,
    ) -> Result<MembershipWrite<InviteClaimResult>> {
        claim_relay_invite_with_owner(
            &self.pool,
            community,
            token_hash,
            claimer_pubkey,
            policy_version,
            verified_owner,
            relay_keypair,
        )
        .await
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
        relay_keypair: &nostr::Keys,
    ) -> Result<MembershipWrite<ClaimOutcome>> {
        claim_relay_invite(
            &self.pool,
            community,
            token_hash,
            claimer_pubkey,
            policy_version,
            relay_keypair,
        )
        .await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::relay_members::is_relay_member;
    use sha2::Digest;
    use sqlx::PgPool;
    use uuid::Uuid;

    pub(super) async fn setup_pool() -> PgPool {
        PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB")
    }

    async fn create_scratch_database(prefix: &str) -> (PgPool, String, String) {
        let admin_url = crate::test_support::database_url();
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

    pub(super) async fn make_test_community(pool: &PgPool) -> CommunityId {
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
        // Membership changes write the community's kind:13534 list.
        sqlx::query("DELETE FROM events WHERE community_id = $1")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("delete test membership lists");
        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("delete test community");
        tx.commit().await.expect("commit test cleanup");
    }

    pub(super) fn test_pubkey() -> String {
        format!("{:064x}", Uuid::new_v4().as_u128())
    }

    pub(super) async fn use_count(pool: &PgPool, community: CommunityId, invite_id: Uuid) -> i32 {
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
            .submit(Some(&host), "owner", None, None)
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

        let error = mint_relay_invite(&pool, community, &test_pubkey(), 3600, Some(1))
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
        let invite = mint_relay_invite(&pool, community, &test_pubkey(), 3600, Some(1))
            .await
            .expect("mint bounded invite");
        let hash = hash_v2_code(&invite.code);

        let keys = nostr::Keys::generate();
        let joined = claim_relay_invite(&pool, community, &hash, &first, None, &keys)
            .await
            .expect("first claim");
        crate::relay_members::assert_membership_list_matches(
            &pool,
            community,
            &keys,
            joined.snapshot.as_ref(),
        )
        .await;
        assert_eq!(
            joined.outcome,
            ClaimOutcome::Joined {
                use_count: 1,
                uses_remaining: Some(0),
            }
        );
        assert_eq!(
            claim_relay_invite(
                &pool,
                community,
                &hash,
                &first,
                None,
                &nostr::Keys::generate()
            )
            .await
            .map(|write| write.outcome)
            .expect("idempotent retry"),
            ClaimOutcome::AlreadyMember {
                use_count: 1,
                uses_remaining: Some(0),
            }
        );
        assert_eq!(
            claim_relay_invite(
                &pool,
                community,
                &hash,
                &second,
                None,
                &nostr::Keys::generate()
            )
            .await
            .map(|write| write.outcome)
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
    async fn concurrent_claims_serialize_the_final_slot() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let first = test_pubkey();
        let second = test_pubkey();
        let invite = mint_relay_invite(&pool, community, &test_pubkey(), 3600, Some(1))
            .await
            .expect("mint bounded invite");
        let hash = hash_v2_code(&invite.code);

        let keys = nostr::Keys::generate();
        let (first_outcome, second_outcome) = tokio::join!(
            claim_relay_invite(&pool, community, &hash, &first, None, &keys),
            claim_relay_invite(&pool, community, &hash, &second, None, &keys),
        );
        let outcomes = [
            first_outcome
                .map(|write| write.outcome)
                .expect("first concurrent claim"),
            second_outcome
                .map(|write| write.outcome)
                .expect("second concurrent claim"),
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
        let invite = mint_relay_invite(&pool, community_a, &test_pubkey(), 3600, Some(2))
            .await
            .expect("mint invite");
        let hash = hash_v2_code(&invite.code);

        assert_eq!(
            claim_relay_invite(
                &pool,
                community_b,
                &hash,
                &test_pubkey(),
                None,
                &nostr::Keys::generate()
            )
            .await
            .map(|write| write.outcome)
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
            claim_relay_invite(
                &pool,
                community_a,
                &hash,
                &test_pubkey(),
                None,
                &nostr::Keys::generate()
            )
            .await
            .map(|write| write.outcome)
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
        let old = mint_relay_invite(&pool, community, &test_pubkey(), 3600, Some(1))
            .await
            .expect("mint old invite");
        let recent = mint_relay_invite(&pool, community, &test_pubkey(), 3600, Some(1))
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
        let invite = mint_relay_invite(&pool, community, &test_pubkey(), 3600, None)
            .await
            .expect("mint unlimited invite");
        let hash = hash_v2_code(&invite.code);

        for (expected_count, pubkey) in [(1, test_pubkey()), (2, test_pubkey())] {
            assert_eq!(
                claim_relay_invite(
                    &pool,
                    community,
                    &hash,
                    &pubkey,
                    None,
                    &nostr::Keys::generate()
                )
                .await
                .map(|write| write.outcome)
                .expect("unlimited claim"),
                ClaimOutcome::Joined {
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
    async fn policy_evidence_failure_rolls_back_membership_and_consumption() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let pubkey = test_pubkey();
        let invite = mint_relay_invite(&pool, community, &test_pubkey(), 3600, Some(1))
            .await
            .expect("mint bounded invite");
        let hash = hash_v2_code(&invite.code);

        let error = claim_relay_invite(
            &pool,
            community,
            &hash,
            &pubkey,
            Some("too-short"),
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect_err("policy CHECK must reject an invalid version");
        assert!(matches!(error, crate::DbError::Sqlx(_)), "{error:?}");
        assert!(!is_relay_member(&pool, community, &pubkey)
            .await
            .expect("membership after rollback"));
        assert_eq!(use_count(&pool, community, invite.invite_id).await, 0);

        assert!(matches!(
            claim_relay_invite(
                &pool,
                community,
                &hash,
                &pubkey,
                None,
                &nostr::Keys::generate()
            )
            .await
            .map(|write| write.outcome)
            .expect("claim after rollback"),
            ClaimOutcome::Joined { use_count: 1, .. }
        ));
        delete_test_community(&pool, community).await;
    }
}

#[cfg(test)]
mod revocation_postgres_tests;

#[cfg(test)]
#[path = "relay_invite/upgrade_postgres_tests.rs"]
mod upgrade_postgres_tests;

#[cfg(test)]
mod owner_admission_postgres_tests;

#[cfg(test)]
mod writer_protocol_postgres_tests;
