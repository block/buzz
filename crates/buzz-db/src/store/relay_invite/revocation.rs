//! Application-owned revocation, under the admission lock and in the writer's commit.
use crate::{error::Result, CommunityId};

/// Durably revoke an active banned principal's invites and its verified agents' invites.
/// The caller holds the admission lock before writing restriction or invite rows.
pub(crate) async fn revoke_banned_issuer_invites(
    tx: &mut sqlx::PgConnection,
    community: CommunityId,
    pubkey: &[u8],
) -> Result<()> {
    sqlx::query(
        "UPDATE relay_invites SET revoked_at = clock_timestamp() \
         WHERE community_id = $1 AND revoked_at IS NULL \
           AND (created_by = encode($2::bytea, 'hex') OR created_by IN (\
               SELECT encode(pubkey, 'hex') FROM users \
               WHERE community_id = $1 AND agent_owner_pubkey = $2\
           )) AND EXISTS (\
               SELECT 1 FROM community_bans WHERE community_id = $1 AND pubkey = $2 \
                 AND banned AND (ban_expires_at IS NULL OR ban_expires_at > clock_timestamp())\
           )",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Durably revoke an agent's invites when its newly verified owner is actively banned.
/// Timeouts block admission temporarily but never durably revoke invites.
pub(crate) async fn revoke_restricted_agent_invites(
    tx: &mut sqlx::PgConnection,
    community: CommunityId,
    agent_pubkey: &[u8],
    owner_pubkey: &[u8],
) -> Result<()> {
    sqlx::query(
        "UPDATE relay_invites SET revoked_at = clock_timestamp() \
         WHERE community_id = $1 AND created_by = encode($2::bytea, 'hex') \
           AND revoked_at IS NULL AND EXISTS (\
               SELECT 1 FROM community_bans WHERE community_id = $1 AND pubkey = $3 \
                 AND banned AND (ban_expires_at IS NULL OR ban_expires_at > clock_timestamp())\
           )",
    )
    .bind(community.as_uuid())
    .bind(agent_pubkey)
    .bind(owner_pubkey)
    .execute(&mut *tx)
    .await?;
    Ok(())
}
