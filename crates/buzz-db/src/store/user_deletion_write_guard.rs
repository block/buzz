//! First serving integration: new stream/forum messages and agent ownership.
//! Other writes are not fenced here; real deletion must remain disabled until
//! every serving path and external-work drain has been integrated.

use buzz_core::{kind, CommunityId};
use nostr::{Event, PublicKey};
use sqlx::{Postgres, Transaction};

use super::user_deletion_fences::{lock_exclusive, lock_shared, read_locked, FenceIdentity};
use crate::{DbError, Result};

pub(crate) async fn guard_message(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    event: &Event,
) -> Result<()> {
    if !matches!(
        u32::from(event.kind.as_u16()),
        kind::KIND_STREAM_MESSAGE
            | kind::KIND_STREAM_MESSAGE_V2
            | kind::KIND_FORUM_POST
            | kind::KIND_FORUM_COMMENT
    ) {
        return Ok(());
    }

    let signer = event.pubkey;
    let owner = owner(tx, community, signer).await?;
    let mut identities = vec![
        FenceIdentity::user(signer),
        FenceIdentity::agent(community, signer),
    ];
    if let Some(owner) = owner {
        identities.push(FenceIdentity::user(owner));
    }
    // Global signer lock also coordinates the first assignment of an owner.
    // Never derive the owner from a cached session or an event tag.
    lock_shared(tx, &identities).await?;
    if owner != self::owner(tx, community, signer).await? {
        // Restart in a new transaction: adding another owner lock now could
        // invert the global-before-scoped ordering. No write has occurred.
        return Err(DbError::AccessDenied(
            "agent ownership changed; retry the write".into(),
        ));
    }
    check_signer(tx, community, signer).await
}

pub(crate) async fn guard_owner_assignment(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    agent: &[u8],
    owner: &[u8],
) -> Result<()> {
    let agent = parse_key(agent)?;
    let owner = parse_key(owner)?;
    lock_shared(
        tx,
        &[FenceIdentity::user(agent), FenceIdentity::user(owner)],
    )
    .await?;
    // Ownership must not change while a message holds the scoped shared lock.
    lock_exclusive(tx, &[FenceIdentity::agent(community, agent)]).await?;
    if read_locked(tx, FenceIdentity::user(owner)).await?.is_some() {
        return Err(DbError::UserDeletionRestricted);
    }
    check_signer(tx, community, agent).await
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
    if read_locked(tx, FenceIdentity::user(signer))
        .await?
        .is_some()
        || read_locked(tx, FenceIdentity::agent(community, signer))
            .await?
            .is_some()
    {
        return Err(DbError::UserDeletionRestricted);
    }
    Ok(())
}
