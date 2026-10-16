use crate::error::Result;
use crate::{CommunityId, DbError};
use sqlx::PgPool;

/// Result of a committed legacy invitation claim.
pub struct MembershipClaimResult {
    /// Whether this claim inserted membership.
    pub inserted: bool,
    /// Whether the same commit first recorded verified ownership.
    pub owner_link_created: bool,
    /// User rows first created by this successful transaction.
    pub users_created: u64,
}

/// Check existing and presented owner identities without creating a user row.
pub(super) async fn claimant_restricted(
    tx: &mut sqlx::PgConnection,
    community: CommunityId,
    claimant: &str,
    verified_owner: Option<&str>,
) -> Result<bool> {
    if super::principal_restricted(tx, community, claimant).await? {
        return Ok(true);
    }
    let Some(owner) = verified_owner else {
        return Ok(false);
    };
    if super::principal_restricted(tx, community, owner).await? {
        return Ok(true);
    }
    let recorded: Option<Option<Vec<u8>>> = sqlx::query_scalar(
        "SELECT agent_owner_pubkey FROM users WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community.as_uuid())
    .bind(
        hex::decode(claimant)
            .map_err(|_| DbError::InvalidData("invalid invitation principal".into()))?,
    )
    .fetch_optional(&mut *tx)
    .await?;
    let owner_bytes =
        hex::decode(owner).map_err(|_| DbError::InvalidData("invalid invitation owner".into()))?;
    if recorded
        .flatten()
        .is_some_and(|existing| existing != owner_bytes)
    {
        return Err(DbError::InviteOwnerMismatch);
    }
    Ok(false)
}

#[derive(Default)]
pub(super) struct OwnerPersistence {
    pub(super) link_created: bool,
    pub(super) users_created: u64,
}

/// Called only on an admitted claim, while the admission lock is still held.
pub(super) async fn persist_claim_owner(
    tx: &mut sqlx::PgConnection,
    community: CommunityId,
    claimant: &str,
    verified_owner: Option<&str>,
) -> Result<OwnerPersistence> {
    let Some(owner) = verified_owner else {
        return Ok(OwnerPersistence::default());
    };
    let owner_bytes =
        hex::decode(owner).map_err(|_| DbError::InvalidData("invalid invitation owner".into()))?;
    let claimant_bytes = hex::decode(claimant)
        .map_err(|_| DbError::InvalidData("invalid invitation principal".into()))?;
    let mut users_created = 0;
    for key in [&owner_bytes, &claimant_bytes] {
        users_created += sqlx::query(
            "INSERT INTO users (community_id, pubkey) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(key)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    let link_created = sqlx::query("UPDATE users SET agent_owner_pubkey=$3 WHERE community_id=$1 AND pubkey=$2 AND agent_owner_pubkey IS NULL")
        .bind(community.as_uuid()).bind(&claimant_bytes).bind(&owner_bytes).execute(&mut *tx).await?.rows_affected() > 0;
    // Defensive parity with other owner writers; admission already rejects banned owners.
    if link_created {
        super::revoke_restricted_agent_invites(tx, community, &claimant_bytes, &owner_bytes)
            .await?;
    }
    Ok(OwnerPersistence {
        link_created,
        users_created,
    })
}

/// Atomically claim membership and caller-verified ownership after policy checks.
pub async fn claim_relay_membership_with_owner(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &str,
    role: &str,
    policy_version: Option<&str>,
    verified_owner: Option<&str>,
) -> Result<MembershipClaimResult> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    super::lock_admission(tx.conn(), community).await?;
    if claimant_restricted(tx.conn(), community, pubkey, verified_owner).await? {
        return Err(DbError::InviteRestricted);
    }
    let inserted = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, $3, 'invite') \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(role)
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
        .bind(pubkey)
        .bind(version)
        .execute(tx.conn())
        .await?;
    }

    let owner = persist_claim_owner(tx.conn(), community, pubkey, verified_owner).await?;
    tx.commit().await?;
    Ok(MembershipClaimResult {
        inserted,
        owner_link_created: owner.link_created,
        users_created: owner.users_created,
    })
}
