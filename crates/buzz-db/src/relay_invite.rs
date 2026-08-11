//! Relay invite persistence for generic v2 invites and bound v3 handoffs.
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
use chrono::{DateTime, Utc};
use sha2::{Digest as _, Sha256};
use sqlx::{PgPool, Postgres, Row as _, Transaction};
use std::collections::BTreeSet;

use crate::error::Result;
use crate::CommunityId;

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

    // Generate 32 random bytes and encode as base64url — this is the secret.
    let secret: [u8; V2_SECRET_LEN] = rand::random();
    let code = encode_v2_code(&secret);
    let token_hash = hash_v2_code(&code);
    let now = Utc::now();
    let expires_at = now + chrono::Duration::seconds(ttl_secs as i64);

    let row = sqlx::query(
        "INSERT INTO relay_invites (community_id, token_hash, max_uses, expires_at, created_by) \
         VALUES ($1, $2, $3, $4, $5) \
         RETURNING id",
    )
    .bind(community.as_uuid())
    .bind(token_hash.as_slice())
    .bind(max_uses)
    .bind(expires_at)
    .bind(created_by)
    .fetch_one(pool)
    .await?;

    let invite_id: uuid::Uuid = row.try_get("id")?;

    Ok(MintedInvite {
        code,
        expires_at,
        max_uses,
        uses_remaining: max_uses,
        invite_id,
    })
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
) -> Result<ClaimOutcome> {
    let mut tx = pool.begin().await?;

    // 2. SELECT FOR UPDATE — lock the invite row for the duration of this txn.
    let row = sqlx::query(
        "SELECT id, max_uses, use_count, expires_at \
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
    let max_uses: Option<i32> = invite.try_get("max_uses")?;
    let use_count: i32 = invite.try_get("use_count")?;
    let expires_at: DateTime<Utc> = invite.try_get("expires_at")?;

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

    let uses_remaining = || max_uses.map(|mu| mu - use_count);

    // 5. Check existing membership.
    let existing =
        sqlx::query("SELECT 1 FROM relay_members WHERE community_id = $1 AND pubkey = $2")
            .bind(community.as_uuid())
            .bind(claimer_pubkey)
            .fetch_optional(&mut *tx)
            .await?;

    if existing.is_some() {
        // 6. Already a member — insert policy evidence but do NOT increment.
        if let Some(version) = policy_version {
            sqlx::query(
                "INSERT INTO join_policy_acceptances (community_id, pubkey, policy_version) \
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(community.as_uuid())
            .bind(claimer_pubkey)
            .bind(version)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "already_member",
            max_uses,
            Some(use_count),
        );
        return Ok(ClaimOutcome::AlreadyMember {
            use_count,
            uses_remaining: uses_remaining(),
        });
    }

    // 7. Capacity check.
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

    // 8. Insert relay member. The conflict branch covers a claimant admitted
    // concurrently through a different invite: only the transaction that
    // actually inserted membership may consume this invite.
    let inserted = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, 'member', 'invite') \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(claimer_pubkey)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;

    // 9. Insert join-policy acceptance evidence. This is required for both a
    // new member and a claimant whose concurrent membership insert won first.
    if let Some(version) = policy_version {
        sqlx::query(
            "INSERT INTO join_policy_acceptances (community_id, pubkey, policy_version) \
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(claimer_pubkey)
        .bind(version)
        .execute(&mut *tx)
        .await?;
    }

    if !inserted {
        tx.commit().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "already_member",
            max_uses,
            Some(use_count),
        );
        return Ok(ClaimOutcome::AlreadyMember {
            use_count,
            uses_remaining: uses_remaining(),
        });
    }

    // 10. Increment use_count (for every new member, even unlimited).
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

    Ok(ClaimOutcome::Joined {
        use_count: new_use_count,
        uses_remaining: new_uses_remaining,
    })
}

/// Prefix reserved for public-key-bound identity handoffs.
pub const IDENTITY_HANDOFF_PREFIX: &str = "v3.";

/// Number of random bytes encoded in a v3 identity-handoff code.
pub const IDENTITY_HANDOFF_SECRET_LEN: usize = 32;

/// Fixed v3 identity-handoff lifetime: one hour.
pub const IDENTITY_HANDOFF_TTL_SECS: i32 = 60 * 60;

const IDENTITY_HANDOFF_RETENTION_DAYS: i32 = 30;
const IDENTITY_HANDOFF_RETENTION_BATCH_SIZE: i64 = 1_000;
const INCARNATION_DIGEST_DOMAIN: &[u8] = b"buzz.identity-handoff-incarnation.v1\0";

/// The durable state of a public-key-bound identity handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityHandoffState {
    /// The handoff is live and may be claimed by its bound public key.
    Active,
    /// The bound public key completed the handoff.
    Claimed,
    /// A newer handoff replaced this one.
    Superseded,
    /// Link revocation invalidated this handoff.
    Invalidated,
    /// The database-defined deadline was reached.
    Expired,
}

impl IdentityHandoffState {
    fn from_database(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "claimed" => Ok(Self::Claimed),
            "superseded" => Ok(Self::Superseded),
            "invalidated" => Ok(Self::Invalidated),
            "expired" => Ok(Self::Expired),
            _ => Err(crate::DbError::InvalidData(
                "identity handoff has an unknown state".to_owned(),
            )),
        }
    }
}

/// A freshly minted v3 identity handoff.
#[derive(Debug)]
pub struct MintedIdentityHandoff {
    /// Full `v3.<hex-secret>` bearer code. Returned only from the mint call.
    pub code: String,
    /// Opaque, non-authorizing status reference.
    pub handoff_id: uuid::Uuid,
    /// Database-generated one-hour expiry.
    pub expires_at: DateTime<Utc>,
}

/// Result of attempting to mint a v3 identity handoff.
#[derive(Debug)]
pub enum MintIdentityHandoffOutcome {
    /// The handoff was created and any older live handoff was superseded.
    Minted(MintedIdentityHandoff),
    /// The supplied link incarnation has already been revoked.
    RevokedIncarnation,
}

/// Whether a successful v3 claim added relay membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityHandoffMembershipOutcome {
    /// The claim inserted membership for the bound key.
    Added,
    /// The bound key was already a relay member.
    AlreadyMember,
}

/// Typed result of a v3 identity-handoff claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityHandoffClaimOutcome {
    /// The live handoff was stamped claimed independently of membership state.
    Claimed {
        /// Whether this transaction inserted membership.
        membership: IdentityHandoffMembershipOutcome,
    },
    /// The handoff had already been claimed.
    AlreadyClaimed,
    /// The claimant does not match the bound public key.
    IdentityMismatch,
    /// The handoff reached its database-defined deadline.
    Expired,
    /// A newer mint replaced this handoff.
    Superseded,
    /// Link revocation invalidated this handoff.
    Invalidated,
    /// No handoff matches the community-scoped token digest.
    Invalid,
}

/// Result of installing a revoked-incarnation fence and invalidating handoffs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityHandoffInvalidation {
    /// True only when this call inserted the permanent fence.
    pub fence_created: bool,
    /// Number of active handoffs changed to invalidated.
    pub invalidated_count: u64,
}

/// Build the canonical v3 code for a random identity-handoff secret.
pub fn encode_identity_handoff_code(secret: &[u8; IDENTITY_HANDOFF_SECRET_LEN]) -> String {
    format!("{IDENTITY_HANDOFF_PREFIX}{}", hex::encode(secret))
}

/// Validate the canonical fixed-length v3 code shape without reading storage.
pub fn validate_identity_handoff_code(code: &str) -> bool {
    let Some(encoded) = code.strip_prefix(IDENTITY_HANDOFF_PREFIX) else {
        return false;
    };
    encoded.len() == IDENTITY_HANDOFF_SECRET_LEN * 2
        && encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Hash the complete v3 code for database lookup.
pub fn hash_identity_handoff_code(code: &str) -> [u8; 32] {
    Sha256::digest(code.as_bytes()).into()
}

fn identity_handoff_incarnation_digest(link_incarnation_id: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(INCARNATION_DIGEST_DOMAIN);
    hasher.update(link_incarnation_id.as_bytes());
    hasher.finalize().into()
}

fn normalize_identity_handoff_pubkey(pubkey: &str) -> Result<String> {
    if pubkey.len() != 64 || !pubkey.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(crate::DbError::InvalidData(
            "identity handoff requires a 64-character hexadecimal public key".to_owned(),
        ));
    }
    Ok(pubkey.to_ascii_lowercase())
}

fn validate_identity_handoff_incarnation(link_incarnation_id: &str) -> Result<()> {
    if !(16..=256).contains(&link_incarnation_id.len()) || !link_incarnation_id.is_ascii() {
        return Err(crate::DbError::InvalidData(
            "identity handoff link incarnation is malformed".to_owned(),
        ));
    }
    Ok(())
}

fn validate_identity_handoff_creator(created_by: &str) -> Result<()> {
    if created_by.is_empty() || created_by.len() > 256 {
        return Err(crate::DbError::InvalidData(
            "identity handoff creator is malformed".to_owned(),
        ));
    }
    Ok(())
}

async fn lock_identity_handoff_key(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    expected_pubkey: &str,
) -> Result<()> {
    let lock_identity = format!("buzz_identity_handoff:{community}:{expected_pubkey}");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(lock_identity)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn normalize_expired_identity_handoffs(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    expected_pubkey: &str,
) -> Result<u64> {
    let result = sqlx::query(
        "UPDATE identity_handoffs \
         SET state = 'expired', terminal_at = transaction_timestamp() \
         WHERE community_id = $1 AND expected_pubkey = $2 AND state = 'active' \
           AND expires_at <= transaction_timestamp()",
    )
    .bind(community.as_uuid())
    .bind(expected_pubkey)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

/// Mint one one-hour identity handoff for a normalized public key.
///
/// The transaction takes the community/public-key advisory lock before it
/// normalizes expiry or supersedes any row. A durable revoked-incarnation fence
/// is checked under the same transaction before the new active row is inserted.
pub async fn mint_identity_handoff(
    pool: &PgPool,
    community: CommunityId,
    expected_pubkey: &str,
    link_incarnation_id: &str,
    created_by: &str,
) -> Result<MintIdentityHandoffOutcome> {
    let expected_pubkey = normalize_identity_handoff_pubkey(expected_pubkey)?;
    validate_identity_handoff_incarnation(link_incarnation_id)?;
    validate_identity_handoff_creator(created_by)?;
    let incarnation_hash = identity_handoff_incarnation_digest(link_incarnation_id);
    let secret: [u8; IDENTITY_HANDOFF_SECRET_LEN] = rand::random();
    let code = encode_identity_handoff_code(&secret);
    let token_hash = hash_identity_handoff_code(&code);

    let mut tx = pool.begin().await?;
    lock_identity_handoff_key(&mut tx, community, &expected_pubkey).await?;
    normalize_expired_identity_handoffs(&mut tx, community, &expected_pubkey).await?;

    let fenced = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS( \
             SELECT 1 FROM identity_handoff_revoked_incarnations \
             WHERE community_id = $1 AND incarnation_hash = $2 \
         )",
    )
    .bind(community.as_uuid())
    .bind(incarnation_hash.as_slice())
    .fetch_one(&mut *tx)
    .await?;
    if fenced {
        tx.rollback().await?;
        return Ok(MintIdentityHandoffOutcome::RevokedIncarnation);
    }

    // The advisory lock is always taken before UPDATE obtains row locks.
    sqlx::query(
        "UPDATE identity_handoffs \
         SET state = 'superseded', terminal_at = transaction_timestamp() \
         WHERE community_id = $1 AND expected_pubkey = $2 AND state = 'active'",
    )
    .bind(community.as_uuid())
    .bind(&expected_pubkey)
    .execute(&mut *tx)
    .await?;

    let row = sqlx::query(
        "INSERT INTO identity_handoffs ( \
             community_id, token_hash, expected_pubkey, incarnation_hash, \
             created_by, expires_at \
         ) VALUES ( \
             $1, $2, $3, $4, $5, \
             transaction_timestamp() + make_interval(secs => $6) \
         ) RETURNING id, expires_at",
    )
    .bind(community.as_uuid())
    .bind(token_hash.as_slice())
    .bind(&expected_pubkey)
    .bind(incarnation_hash.as_slice())
    .bind(created_by)
    .bind(IDENTITY_HANDOFF_TTL_SECS)
    .fetch_one(&mut *tx)
    .await?;
    let handoff_id = row.try_get("id")?;
    let expires_at = row.try_get("expires_at")?;
    tx.commit().await?;

    Ok(MintIdentityHandoffOutcome::Minted(MintedIdentityHandoff {
        code,
        handoff_id,
        expires_at,
    }))
}

fn terminal_claim_outcome(state: IdentityHandoffState) -> IdentityHandoffClaimOutcome {
    match state {
        IdentityHandoffState::Active => IdentityHandoffClaimOutcome::Invalid,
        IdentityHandoffState::Claimed => IdentityHandoffClaimOutcome::AlreadyClaimed,
        IdentityHandoffState::Superseded => IdentityHandoffClaimOutcome::Superseded,
        IdentityHandoffState::Invalidated => IdentityHandoffClaimOutcome::Invalidated,
        IdentityHandoffState::Expired => IdentityHandoffClaimOutcome::Expired,
    }
}

/// Atomically claim a v3 identity handoff with its exact bound public key.
///
/// Token lookup is deliberately unlocked. It reveals the lock identity, after
/// which the transaction takes the advisory lock and rereads the row `FOR
/// UPDATE`. The handoff is stamped claimed even when membership already exists.
pub async fn claim_identity_handoff(
    pool: &PgPool,
    community: CommunityId,
    token_hash: &[u8; 32],
    claimer_pubkey: &str,
    policy_version: Option<&str>,
) -> Result<IdentityHandoffClaimOutcome> {
    let claimer_pubkey = normalize_identity_handoff_pubkey(claimer_pubkey)?;
    let Some(lock_pubkey) = sqlx::query_scalar::<_, String>(
        "SELECT expected_pubkey FROM identity_handoffs \
         WHERE community_id = $1 AND token_hash = $2",
    )
    .bind(community.as_uuid())
    .bind(token_hash.as_slice())
    .fetch_optional(pool)
    .await?
    else {
        return Ok(IdentityHandoffClaimOutcome::Invalid);
    };

    let mut tx = pool.begin().await?;
    lock_identity_handoff_key(&mut tx, community, &lock_pubkey).await?;
    let row = sqlx::query(
        "SELECT id, expected_pubkey, state, \
                expires_at <= transaction_timestamp() AS is_expired \
         FROM identity_handoffs \
         WHERE community_id = $1 AND token_hash = $2 \
         FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(token_hash.as_slice())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.rollback().await?;
        return Ok(IdentityHandoffClaimOutcome::Invalid);
    };

    let handoff_id: uuid::Uuid = row.try_get("id")?;
    let expected_pubkey: String = row.try_get("expected_pubkey")?;
    let state = IdentityHandoffState::from_database(row.try_get("state")?)?;
    let is_expired: bool = row.try_get("is_expired")?;

    if state == IdentityHandoffState::Claimed {
        let membership_still_present = expected_pubkey == claimer_pubkey
            && sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS( \
                     SELECT 1 FROM relay_members \
                     WHERE community_id = $1 AND pubkey = $2 \
                 )",
            )
            .bind(community.as_uuid())
            .bind(&claimer_pubkey)
            .fetch_one(&mut *tx)
            .await?;
        tx.rollback().await?;
        return Ok(if membership_still_present {
            IdentityHandoffClaimOutcome::Claimed {
                membership: IdentityHandoffMembershipOutcome::AlreadyMember,
            }
        } else {
            IdentityHandoffClaimOutcome::AlreadyClaimed
        });
    }
    if state != IdentityHandoffState::Active {
        tx.rollback().await?;
        return Ok(terminal_claim_outcome(state));
    }
    if is_expired {
        sqlx::query(
            "UPDATE identity_handoffs \
             SET state = 'expired', terminal_at = transaction_timestamp() \
             WHERE community_id = $1 AND id = $2 AND state = 'active'",
        )
        .bind(community.as_uuid())
        .bind(handoff_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(IdentityHandoffClaimOutcome::Expired);
    }
    if expected_pubkey != claimer_pubkey {
        tx.rollback().await?;
        return Ok(IdentityHandoffClaimOutcome::IdentityMismatch);
    }

    let membership_added = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, 'member', 'invite') \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(&claimer_pubkey)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;

    if let Some(policy_version) = policy_version {
        sqlx::query(
            "INSERT INTO join_policy_acceptances (community_id, pubkey, policy_version) \
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(&claimer_pubkey)
        .bind(policy_version)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE identity_handoffs \
         SET state = 'claimed', terminal_at = transaction_timestamp() \
         WHERE community_id = $1 AND id = $2 AND state = 'active'",
    )
    .bind(community.as_uuid())
    .bind(handoff_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let membership = if membership_added {
        IdentityHandoffMembershipOutcome::Added
    } else {
        IdentityHandoffMembershipOutcome::AlreadyMember
    };
    Ok(IdentityHandoffClaimOutcome::Claimed { membership })
}

/// Read and normalize one v3 handoff after authenticating its stored binding.
///
/// The handoff ID is only a locator. Both expected public key and link
/// incarnation must match before expiry is normalized or state is returned.
pub async fn identity_handoff_status(
    pool: &PgPool,
    community: CommunityId,
    handoff_id: uuid::Uuid,
    expected_pubkey: &str,
    link_incarnation_id: &str,
) -> Result<Option<IdentityHandoffState>> {
    let expected_pubkey = normalize_identity_handoff_pubkey(expected_pubkey)?;
    validate_identity_handoff_incarnation(link_incarnation_id)?;
    let incarnation_hash = identity_handoff_incarnation_digest(link_incarnation_id);
    let mut tx = pool.begin().await?;
    lock_identity_handoff_key(&mut tx, community, &expected_pubkey).await?;

    let row = sqlx::query(
        "SELECT expected_pubkey, incarnation_hash, state, \
                expires_at <= transaction_timestamp() AS is_expired \
         FROM identity_handoffs \
         WHERE community_id = $1 AND id = $2 \
         FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(handoff_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.rollback().await?;
        return Ok(None);
    };

    let stored_pubkey: String = row.try_get("expected_pubkey")?;
    let stored_incarnation: Vec<u8> = row.try_get("incarnation_hash")?;
    if stored_pubkey != expected_pubkey || stored_incarnation.as_slice() != incarnation_hash {
        tx.rollback().await?;
        return Ok(None);
    }

    let mut state = IdentityHandoffState::from_database(row.try_get("state")?)?;
    let is_expired: bool = row.try_get("is_expired")?;
    if state == IdentityHandoffState::Active && is_expired {
        sqlx::query(
            "UPDATE identity_handoffs \
             SET state = 'expired', terminal_at = transaction_timestamp() \
             WHERE community_id = $1 AND id = $2 AND state = 'active'",
        )
        .bind(community.as_uuid())
        .bind(handoff_id)
        .execute(&mut *tx)
        .await?;
        state = IdentityHandoffState::Expired;
    }
    tx.commit().await?;
    Ok(Some(state))
}

/// Permanently fence one link incarnation and invalidate active handoffs.
///
/// Fence insertion and state changes share one transaction. The public key is
/// used only as the advisory-lock identity and handoff selector; the durable
/// fence contains only the community and domain-separated incarnation digest.
pub async fn invalidate_identity_handoffs(
    pool: &PgPool,
    community: CommunityId,
    expected_pubkey: &str,
    link_incarnation_id: &str,
) -> Result<IdentityHandoffInvalidation> {
    let expected_pubkey = normalize_identity_handoff_pubkey(expected_pubkey)?;
    validate_identity_handoff_incarnation(link_incarnation_id)?;
    let incarnation_hash = identity_handoff_incarnation_digest(link_incarnation_id);
    let mut tx = pool.begin().await?;
    lock_identity_handoff_key(&mut tx, community, &expected_pubkey).await?;

    let fence_created = sqlx::query(
        "INSERT INTO identity_handoff_revoked_incarnations (community_id, incarnation_hash) \
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(incarnation_hash.as_slice())
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;

    // The advisory lock is always acquired before UPDATE takes row locks.
    let invalidated_count = sqlx::query(
        "UPDATE identity_handoffs \
         SET state = 'invalidated', terminal_at = transaction_timestamp() \
         WHERE community_id = $1 AND expected_pubkey = $2 \
           AND incarnation_hash = $3 AND state = 'active'",
    )
    .bind(community.as_uuid())
    .bind(&expected_pubkey)
    .bind(incarnation_hash.as_slice())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;

    Ok(IdentityHandoffInvalidation {
        fence_created,
        invalidated_count,
    })
}

/// Delete one bounded batch of v3 handoffs past the 30-day terminal window.
///
/// Candidate discovery is unlocked. The transaction acquires every candidate
/// community/public-key advisory lock in sorted order before deleting any row,
/// preserving the advisory-before-row ordering used by all other transitions.
/// Revoked-incarnation fences are never removed by this cleanup.
pub async fn reap_terminal_identity_handoffs(pool: &PgPool) -> Result<u64> {
    let candidates = sqlx::query(
        "SELECT community_id, id, expected_pubkey \
         FROM identity_handoffs \
         WHERE terminal_at < transaction_timestamp() - make_interval(days => $1) \
            OR (state = 'active' AND \
                expires_at < transaction_timestamp() - make_interval(days => $1)) \
         ORDER BY COALESCE(terminal_at, expires_at), community_id, id \
         LIMIT $2",
    )
    .bind(IDENTITY_HANDOFF_RETENTION_DAYS)
    .bind(IDENTITY_HANDOFF_RETENTION_BATCH_SIZE)
    .fetch_all(pool)
    .await?;
    if candidates.is_empty() {
        return Ok(0);
    }

    let mut lock_keys = BTreeSet::new();
    let mut rows = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let community_id: uuid::Uuid = candidate.try_get("community_id")?;
        let handoff_id: uuid::Uuid = candidate.try_get("id")?;
        let expected_pubkey: String = candidate.try_get("expected_pubkey")?;
        lock_keys.insert((community_id, expected_pubkey.clone()));
        rows.push((community_id, handoff_id));
    }

    let mut tx = pool.begin().await?;
    for (community_id, expected_pubkey) in lock_keys {
        lock_identity_handoff_key(
            &mut tx,
            CommunityId::from_uuid(community_id),
            &expected_pubkey,
        )
        .await?;
    }

    let (community_ids, handoff_ids): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
    let deleted = sqlx::query(
        "DELETE FROM identity_handoffs AS handoff \
         USING UNNEST($1::uuid[], $2::uuid[]) AS target(community_id, handoff_id) \
         WHERE handoff.community_id = target.community_id \
           AND handoff.id = target.handoff_id \
           AND ( \
             handoff.terminal_at < transaction_timestamp() - make_interval(days => $3) \
             OR (handoff.state = 'active' AND \
                 handoff.expires_at < transaction_timestamp() - make_interval(days => $3)) \
           )",
    )
    .bind(community_ids)
    .bind(handoff_ids)
    .bind(IDENTITY_HANDOFF_RETENTION_DAYS)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay_members::is_relay_member;
    use sqlx::PgPool;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
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
        sqlx::query("DELETE FROM identity_handoffs WHERE community_id = $1")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("delete test identity handoffs");
        sqlx::query("DELETE FROM identity_handoff_revoked_incarnations WHERE community_id = $1")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("delete test identity handoff fences");
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

    fn test_incarnation() -> String {
        Uuid::new_v4().to_string()
    }

    fn minted_identity_handoff(outcome: MintIdentityHandoffOutcome) -> MintedIdentityHandoff {
        match outcome {
            MintIdentityHandoffOutcome::Minted(handoff) => handoff,
            MintIdentityHandoffOutcome::RevokedIncarnation => {
                panic!("fresh incarnation unexpectedly revoked")
            }
        }
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

    #[allow(clippy::too_many_arguments)]
    async fn insert_raw_identity_handoff(
        pool: &PgPool,
        community: CommunityId,
        handoff_id: Uuid,
        token_hash: [u8; 32],
        expected_pubkey: &str,
        incarnation_hash: [u8; 32],
        state: &str,
        expires_at: DateTime<Utc>,
        terminal_at: Option<DateTime<Utc>>,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO identity_handoffs ( \
                 community_id, id, token_hash, expected_pubkey, incarnation_hash, \
                 state, created_by, expires_at, terminal_at \
             ) VALUES ($1, $2, $3, $4, $5, $6, 'owner', $7, $8)",
        )
        .bind(community.as_uuid())
        .bind(handoff_id)
        .bind(token_hash.as_slice())
        .bind(expected_pubkey)
        .bind(incarnation_hash.as_slice())
        .bind(state)
        .bind(expires_at)
        .bind(terminal_at)
        .execute(pool)
        .await
        .map(|_| ())
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
    fn identity_handoff_codes_use_a_distinct_v3_namespace_and_domain_hashes() {
        let code = encode_identity_handoff_code(&[7_u8; IDENTITY_HANDOFF_SECRET_LEN]);
        assert!(code.starts_with("v3."));
        assert!(!code.starts_with(buzz_core::invite::V2_PREFIX));
        assert_eq!(hash_identity_handoff_code(&code).len(), 32);
        assert_ne!(
            identity_handoff_incarnation_digest("same-bytes"),
            hash_identity_handoff_code("same-bytes")
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn identity_handoff_mismatch_is_non_mutating_and_existing_member_claim_is_stamped() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let expected = test_pubkey();
        let mismatch = test_pubkey();
        let incarnation = test_incarnation();
        let minted = minted_identity_handoff(
            mint_identity_handoff(&pool, community, &expected, &incarnation, "owner")
                .await
                .expect("mint identity handoff"),
        );
        let token_hash = hash_identity_handoff_code(&minted.code);

        assert_eq!(
            claim_identity_handoff(&pool, community, &token_hash, &mismatch, None)
                .await
                .expect("mismatched claim"),
            IdentityHandoffClaimOutcome::IdentityMismatch
        );
        assert!(!is_relay_member(&pool, community, &mismatch)
            .await
            .expect("mismatched membership"));
        assert_eq!(
            identity_handoff_status(&pool, community, minted.handoff_id, &expected, &incarnation,)
                .await
                .expect("active status"),
            Some(IdentityHandoffState::Active)
        );
        assert_eq!(
            identity_handoff_status(
                &pool,
                community,
                minted.handoff_id,
                &expected,
                &test_incarnation(),
            )
            .await
            .expect("wrong-incarnation status"),
            None
        );

        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
             VALUES ($1, $2, 'member', 'admin')",
        )
        .bind(community.as_uuid())
        .bind(&expected)
        .execute(&pool)
        .await
        .expect("insert pre-existing member");

        assert_eq!(
            claim_identity_handoff(&pool, community, &token_hash, &expected, None)
                .await
                .expect("matching claim"),
            IdentityHandoffClaimOutcome::Claimed {
                membership: IdentityHandoffMembershipOutcome::AlreadyMember,
            }
        );
        assert_eq!(
            identity_handoff_status(&pool, community, minted.handoff_id, &expected, &incarnation,)
                .await
                .expect("claimed status"),
            Some(IdentityHandoffState::Claimed)
        );
        assert_eq!(
            claim_identity_handoff(&pool, community, &token_hash, &expected, None)
                .await
                .expect("idempotent matching retry"),
            IdentityHandoffClaimOutcome::Claimed {
                membership: IdentityHandoffMembershipOutcome::AlreadyMember,
            }
        );

        sqlx::query("DELETE FROM relay_members WHERE community_id = $1 AND pubkey = $2")
            .bind(community.as_uuid())
            .bind(&expected)
            .execute(&pool)
            .await
            .expect("remove claimed membership");
        assert_eq!(
            claim_identity_handoff(&pool, community, &token_hash, &expected, None)
                .await
                .expect("retry after membership removal"),
            IdentityHandoffClaimOutcome::AlreadyClaimed
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_identity_handoff_mints_leave_one_active_and_supersede_the_other() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let expected = test_pubkey();
        let incarnation = test_incarnation();

        let (first, second) = tokio::join!(
            mint_identity_handoff(&pool, community, &expected, &incarnation, "owner"),
            mint_identity_handoff(&pool, community, &expected, &incarnation, "owner"),
        );
        let first = minted_identity_handoff(first.expect("first mint"));
        let second = minted_identity_handoff(second.expect("second mint"));

        let states: Vec<String> = sqlx::query_scalar(
            "SELECT state FROM identity_handoffs \
             WHERE community_id = $1 AND expected_pubkey = $2 ORDER BY created_at, id",
        )
        .bind(community.as_uuid())
        .bind(&expected)
        .fetch_all(&pool)
        .await
        .expect("read handoff states");
        assert_eq!(states.iter().filter(|state| *state == "active").count(), 1);
        assert_eq!(
            states.iter().filter(|state| *state == "superseded").count(),
            1
        );
        assert_ne!(first.handoff_id, second.handoff_id);

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn identity_handoff_claim_at_database_expiry_boundary_is_expired() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let expected = test_pubkey();
        let incarnation = test_incarnation();
        let minted = minted_identity_handoff(
            mint_identity_handoff(&pool, community, &expected, &incarnation, "owner")
                .await
                .expect("mint identity handoff"),
        );
        sqlx::query(
            "UPDATE identity_handoffs SET expires_at = transaction_timestamp() \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(minted.handoff_id)
        .execute(&pool)
        .await
        .expect("set exact expiry boundary");

        assert_eq!(
            claim_identity_handoff(
                &pool,
                community,
                &hash_identity_handoff_code(&minted.code),
                &expected,
                None,
            )
            .await
            .expect("claim at expiry"),
            IdentityHandoffClaimOutcome::Expired
        );
        assert!(!is_relay_member(&pool, community, &expected)
            .await
            .expect("membership after expiry"));

        let stale = minted_identity_handoff(
            mint_identity_handoff(&pool, community, &expected, &incarnation, "owner")
                .await
                .expect("mint stale handoff"),
        );
        sqlx::query(
            "UPDATE identity_handoffs SET expires_at = transaction_timestamp() \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(stale.handoff_id)
        .execute(&pool)
        .await
        .expect("expire active handoff before replacement");
        let fresh = minted_identity_handoff(
            mint_identity_handoff(&pool, community, &expected, &incarnation, "owner")
                .await
                .expect("mint replacement"),
        );
        let stale_state: String = sqlx::query_scalar(
            "SELECT state FROM identity_handoffs WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(stale.handoff_id)
        .fetch_one(&pool)
        .await
        .expect("read normalized stale state");
        assert_eq!(stale_state, "expired");
        assert_eq!(
            identity_handoff_status(&pool, community, fresh.handoff_id, &expected, &incarnation,)
                .await
                .expect("fresh status"),
            Some(IdentityHandoffState::Active)
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn identity_handoff_claim_and_replacement_race_completes_without_deadlock() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let expected = test_pubkey();
        let incarnation = test_incarnation();
        let old = minted_identity_handoff(
            mint_identity_handoff(&pool, community, &expected, &incarnation, "owner")
                .await
                .expect("mint old handoff"),
        );
        let token_hash = hash_identity_handoff_code(&old.code);

        let (claim, replacement) = tokio::join!(
            claim_identity_handoff(&pool, community, &token_hash, &expected, None),
            mint_identity_handoff(&pool, community, &expected, &incarnation, "owner"),
        );
        assert!(matches!(
            claim.expect("racing claim"),
            IdentityHandoffClaimOutcome::Claimed { .. } | IdentityHandoffClaimOutcome::Superseded
        ));
        let replacement = minted_identity_handoff(replacement.expect("racing replacement"));
        assert_eq!(
            identity_handoff_status(
                &pool,
                community,
                replacement.handoff_id,
                &expected,
                &incarnation,
            )
            .await
            .expect("replacement status"),
            Some(IdentityHandoffState::Active)
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn revoked_incarnation_fence_survives_terminal_handoff_cleanup() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let expected = test_pubkey();
        let incarnation = test_incarnation();
        let minted = minted_identity_handoff(
            mint_identity_handoff(&pool, community, &expected, &incarnation, "owner")
                .await
                .expect("mint identity handoff"),
        );

        assert_eq!(
            invalidate_identity_handoffs(&pool, community, &expected, &incarnation)
                .await
                .expect("invalidate incarnation"),
            IdentityHandoffInvalidation {
                fence_created: true,
                invalidated_count: 1,
            }
        );
        assert!(matches!(
            mint_identity_handoff(&pool, community, &expected, &incarnation, "owner")
                .await
                .expect("delayed mint"),
            MintIdentityHandoffOutcome::RevokedIncarnation
        ));

        sqlx::query(
            "UPDATE identity_handoffs \
             SET terminal_at = transaction_timestamp() - interval '31 days' \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(minted.handoff_id)
        .execute(&pool)
        .await
        .expect("age terminal handoff");
        let untouched_expired_id = Uuid::new_v4();
        insert_raw_identity_handoff(
            &pool,
            community,
            untouched_expired_id,
            [9; 32],
            &expected,
            identity_handoff_incarnation_digest(&test_incarnation()),
            "active",
            Utc::now() + chrono::Duration::hours(1),
            None,
        )
        .await
        .expect("insert untouched expired handoff");
        sqlx::query(
            "UPDATE identity_handoffs \
             SET created_at = transaction_timestamp() - interval '32 days', \
                 expires_at = transaction_timestamp() - interval '31 days' \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(untouched_expired_id)
        .execute(&pool)
        .await
        .expect("age untouched expired handoff");
        let reaped = reap_terminal_identity_handoffs(&pool)
            .await
            .expect("reap retained handoffs");
        assert!(reaped >= 2);
        let retained_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_handoffs \
             WHERE community_id = $1 AND id = ANY($2::uuid[])",
        )
        .bind(community.as_uuid())
        .bind(vec![minted.handoff_id, untouched_expired_id])
        .fetch_one(&pool)
        .await
        .expect("verify retained handoff cleanup");
        assert_eq!(retained_rows, 0);
        assert!(matches!(
            mint_identity_handoff(&pool, community, &expected, &incarnation, "owner")
                .await
                .expect("mint after cleanup"),
            MintIdentityHandoffOutcome::RevokedIncarnation
        ));

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stale_invalidation_does_not_cancel_a_new_link_incarnation() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let expected = test_pubkey();
        let old_incarnation = test_incarnation();
        let new_incarnation = test_incarnation();
        let minted = minted_identity_handoff(
            mint_identity_handoff(&pool, community, &expected, &new_incarnation, "owner")
                .await
                .expect("mint new-incarnation handoff"),
        );

        assert_eq!(
            invalidate_identity_handoffs(&pool, community, &expected, &old_incarnation)
                .await
                .expect("invalidate stale incarnation"),
            IdentityHandoffInvalidation {
                fence_created: true,
                invalidated_count: 0,
            }
        );
        assert_eq!(
            identity_handoff_status(
                &pool,
                community,
                minted.handoff_id,
                &expected,
                &new_incarnation,
            )
            .await
            .expect("read new-incarnation status"),
            Some(IdentityHandoffState::Active)
        );

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn identity_handoff_claim_and_invalidation_race_never_leaves_active_state() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let expected = test_pubkey();
        let incarnation = test_incarnation();
        let minted = minted_identity_handoff(
            mint_identity_handoff(&pool, community, &expected, &incarnation, "owner")
                .await
                .expect("mint identity handoff"),
        );
        let token_hash = hash_identity_handoff_code(&minted.code);

        let (claim, invalidation) = tokio::join!(
            claim_identity_handoff(&pool, community, &token_hash, &expected, None),
            invalidate_identity_handoffs(&pool, community, &expected, &incarnation),
        );
        let claim = claim.expect("racing claim");
        let invalidation = invalidation.expect("racing invalidation");
        assert!(matches!(
            claim,
            IdentityHandoffClaimOutcome::Claimed { .. } | IdentityHandoffClaimOutcome::Invalidated
        ));
        assert!(invalidation.invalidated_count <= 1);
        assert!(matches!(
            identity_handoff_status(&pool, community, minted.handoff_id, &expected, &incarnation,)
                .await
                .expect("terminal status"),
            Some(IdentityHandoffState::Claimed | IdentityHandoffState::Invalidated)
        ));

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn identity_handoff_catalog_constraints_reject_malformed_and_duplicate_rows() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let handoff_id = Uuid::new_v4();
        let pubkey = test_pubkey();
        let future = Utc::now() + chrono::Duration::hours(1);
        insert_raw_identity_handoff(
            &pool, community, handoff_id, [1; 32], &pubkey, [2; 32], "active", future, None,
        )
        .await
        .expect("insert valid catalog row");

        macro_rules! assert_catalog_rejected {
            ($insert:expr) => {{
                let error = $insert
                    .await
                    .expect_err("catalog constraint must reject malformed row");
                assert!(matches!(error, sqlx::Error::Database(_)), "{error:?}");
            }};
        }

        assert_catalog_rejected!(insert_raw_identity_handoff(
            &pool,
            community,
            Uuid::new_v4(),
            [3; 32],
            &pubkey,
            [4; 32],
            "active",
            future,
            None,
        ));
        assert_catalog_rejected!(insert_raw_identity_handoff(
            &pool,
            community,
            Uuid::new_v4(),
            [5; 32],
            "not-a-pubkey",
            [6; 32],
            "active",
            future,
            None,
        ));
        assert_catalog_rejected!(insert_raw_identity_handoff(
            &pool,
            community,
            handoff_id,
            [7; 32],
            &test_pubkey(),
            [8; 32],
            "active",
            future,
            None,
        ));
        assert_catalog_rejected!(insert_raw_identity_handoff(
            &pool,
            community,
            Uuid::new_v4(),
            [1; 32],
            &test_pubkey(),
            [9; 32],
            "active",
            future,
            None,
        ));
        assert_catalog_rejected!(insert_raw_identity_handoff(
            &pool,
            community,
            Uuid::new_v4(),
            [10; 32],
            &test_pubkey(),
            [11; 32],
            "claimed",
            future,
            None,
        ));
        assert_catalog_rejected!(insert_raw_identity_handoff(
            &pool,
            community,
            Uuid::new_v4(),
            [12; 32],
            &test_pubkey(),
            [13; 32],
            "active",
            future,
            Some(Utc::now()),
        ));
        assert_catalog_rejected!(insert_raw_identity_handoff(
            &pool,
            community,
            Uuid::new_v4(),
            [14; 32],
            &test_pubkey(),
            [15; 32],
            "active",
            Utc::now() - chrono::Duration::seconds(1),
            None,
        ));

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bounded_claim_exhausts_and_existing_member_retry_does_not_consume() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let first = test_pubkey();
        let second = test_pubkey();
        let invite = mint_relay_invite(&pool, community, "owner", 3600, Some(1))
            .await
            .expect("mint bounded invite");
        let hash = hash_v2_code(&invite.code);

        assert_eq!(
            claim_relay_invite(&pool, community, &hash, &first, None)
                .await
                .expect("first claim"),
            ClaimOutcome::Joined {
                use_count: 1,
                uses_remaining: Some(0),
            }
        );
        assert_eq!(
            claim_relay_invite(&pool, community, &hash, &first, None)
                .await
                .expect("idempotent retry"),
            ClaimOutcome::AlreadyMember {
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
    async fn concurrent_claims_serialize_the_final_slot() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let first = test_pubkey();
        let second = test_pubkey();
        let invite = mint_relay_invite(&pool, community, "owner", 3600, Some(1))
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
        let invite = mint_relay_invite(&pool, community_a, "owner", 3600, Some(2))
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
        let old = mint_relay_invite(&pool, community, "owner", 3600, Some(1))
            .await
            .expect("mint old invite");
        let recent = mint_relay_invite(&pool, community, "owner", 3600, Some(1))
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
    async fn unlimited_invites_count_each_new_member() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let invite = mint_relay_invite(&pool, community, "owner", 3600, None)
            .await
            .expect("mint unlimited invite");
        let hash = hash_v2_code(&invite.code);

        for (expected_count, pubkey) in [(1, test_pubkey()), (2, test_pubkey())] {
            assert_eq!(
                claim_relay_invite(&pool, community, &hash, &pubkey, None)
                    .await
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
        let invite = mint_relay_invite(&pool, community, "owner", 3600, Some(1))
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
