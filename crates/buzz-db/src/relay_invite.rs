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

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row as _};

use crate::error::Result;
use crate::CommunityId;

/// Maximum supported `max_uses` value. Matches the migration CHECK constraint.
pub const MAX_INVITE_USES: i32 = 10_000;

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

/// Hash a v2 code string to its SHA-256 digest (32 bytes).
///
/// The code is the full `v2.<base64url>` string. We hash the entire string so
/// the relay never needs to parse or decode the secret on the claim path.
pub fn hash_invite_code(code: &str) -> [u8; 32] {
    Sha256::digest(code.as_bytes()).into()
}

/// Mint a v2 invite: generate a 32-byte random secret, hash it, persist the
/// row, and return the plaintext code plus metadata.
///
/// `ttl_secs` is clamped to the same `[60, MAX_INVITE_TTL_SECS]` range as v1.
/// `max_uses` must be `None` (unlimited) or `Some(1..=10000)`.
pub async fn mint_relay_invite(
    pool: &PgPool,
    community: CommunityId,
    created_by: &str,
    ttl_secs: u64,
    max_uses: Option<i32>,
) -> Result<MintedInvite> {
    // Generate 32 random bytes and encode as base64url — this is the secret.
    let secret: [u8; 32] = rand::random();
    let code = format!("v2.{}", URL_SAFE_NO_PAD.encode(secret));
    let token_hash = hash_invite_code(&code);

    // Clamp TTL the same way v1 does.
    let max_ttl = 30 * 24 * 60 * 60; // 30 days, matches invite_token::MAX_INVITE_TTL_SECS
    let ttl = ttl_secs.clamp(60, max_ttl);
    let now = Utc::now();
    let expires_at = now + chrono::Duration::seconds(ttl as i64);

    // Validate max_uses range (defense-in-depth; the DB CHECK also enforces this).
    if let Some(mu) = max_uses {
        if !(1..=MAX_INVITE_USES).contains(&mu) {
            return Err(crate::error::DbError::InvalidData(format!(
                "max_uses must be between 1 and {MAX_INVITE_USES}"
            )));
        }
    }

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
    token_hash: &[u8],
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
        return Ok(ClaimOutcome::Invalid);
    };

    let invite_id: uuid::Uuid = invite.try_get("id")?;
    let max_uses: Option<i32> = invite.try_get("max_uses")?;
    let use_count: i32 = invite.try_get("use_count")?;
    let expires_at: DateTime<Utc> = invite.try_get("expires_at")?;

    // 4. Expired check.
    if expires_at <= Utc::now() {
        tx.rollback().await?;
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
        return Ok(ClaimOutcome::AlreadyMember {
            use_count,
            uses_remaining: uses_remaining(),
        });
    }

    // 7. Capacity check.
    if let Some(mu) = max_uses {
        if use_count >= mu {
            tx.rollback().await?;
            return Ok(ClaimOutcome::Exhausted);
        }
    }

    // 8. Insert relay member.
    sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, 'member', 'invite') \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(claimer_pubkey)
    .execute(&mut *tx)
    .await?;

    // 9. Insert join-policy acceptance evidence.
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

    tracing::info!(
        community = %community,
        invite_id = %invite_id,
        use_count = new_use_count,
        max_uses = ?max_uses,
        "relay invite claimed"
    );

    Ok(ClaimOutcome::Joined {
        use_count: new_use_count,
        uses_remaining: new_uses_remaining,
    })
}
