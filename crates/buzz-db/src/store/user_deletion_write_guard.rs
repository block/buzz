//! Guard shared persistent-event insertion paths.
//! Other writes are not fenced here; real deletion must remain disabled until
//! every serving path and external-work drain has been integrated.

use buzz_core::CommunityId;
use nostr::{Event, PublicKey};
use sqlx::{Postgres, Transaction};

use super::user_deletion_fences::{lock_shared, FenceIdentity};
use crate::{DbError, Result};

pub(crate) async fn guard_event_author(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    event: &Event,
) -> Result<()> {
    let signer = event.pubkey;
    let owner = owner(tx, community, signer).await?;
    let mut identities = vec![
        FenceIdentity::user(signer),
        FenceIdentity::agent(community, signer),
    ];
    if let Some(owner) = owner {
        identities.push(FenceIdentity::user(owner));
    }
    // Never derive the owner from a cached session or an event tag.
    lock_shared(tx, &identities).await?;
    if owner != self::owner(tx, community, signer).await? {
        // Restart in a new transaction: adding another owner lock now could
        // invert the global-before-scoped ordering. The caller must roll back.
        return Err(DbError::EventAuthorOwnershipChanged);
    }
    check_signer(tx, community, signer).await
}

async fn owner(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    signer: PublicKey,
) -> Result<Option<PublicKey>> {
    let owner: Option<Option<Vec<u8>>> = sqlx::query_scalar(
        "SELECT agent_owner_pubkey FROM users WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community.as_uuid())
    .bind(signer.to_bytes().as_slice())
    .fetch_optional(&mut **tx)
    .await?;
    owner.flatten().map(|key| parse_key(&key)).transpose()
}

fn parse_key(key: &[u8]) -> Result<PublicKey> {
    PublicKey::from_slice(key)
        .map_err(|_| DbError::InvalidData("invalid user-deletion identity key".into()))
}

async fn check_signer(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    signer: PublicKey,
) -> Result<()> {
    // Read the signer's restrictions, not the owner's. Check both possible
    // scopes even if the profile/ownership row is absent: those mutable rows
    // must not determine whether a permanent restriction still applies.
    let restricted: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_deletion_fences
         WHERE pubkey = $1 AND (scope_community_id IS NULL OR scope_community_id = $2))",
    )
    .bind(signer.to_hex())
    .bind(community.as_uuid())
    .fetch_one(&mut **tx)
    .await?;
    if restricted {
        return Err(DbError::UserDeletionRestricted);
    }
    Ok(())
}
