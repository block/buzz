//! Attributed upload leases and a bounded, primary-database drain observation.

use super::{CommunityId, DbError, DeletionStore, Duration, Result, ServingWriteLease};
use uuid::Uuid;

/// Outstanding media uploads in one community, including uncertain outcomes.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct MediaUploadActivity {
    /// Community where these uploads were admitted.
    pub community_id: Uuid,
    /// Attributed uploads whose heartbeat is still current.
    pub active: i64,
    /// Attributed uploads that expired without confirmed completion.
    pub unresolved: i64,
    /// Legacy uploads without an authoritative signer; cannot exclude this user.
    pub unattributed: i64,
}

impl DeletionStore {
    /// Save uploader evidence before releasing the admitted operation, including
    /// during community quiescing. An asynchronous audit enqueue is insufficient.
    pub async fn record_admitted_media_upload(
        &self,
        lease: &ServingWriteLease,
        sha256: &str,
    ) -> Result<()> {
        let actor = lease
            .actor_pubkey
            .as_deref()
            .ok_or_else(|| DbError::InvalidData("media lease has no signer".into()))?;
        if sha256.len() != 64
            || !sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(DbError::InvalidData("invalid media hash".into()));
        }
        self.bounded_serving_lease_sql(super::SERVING_WRITE_LEASE_SQL_TIMEOUT, async |conn| {
            let mut tx = super::serving_lease_transaction(conn).await?;
            self.guard_transaction_with_serving_lease(&mut tx, lease).await?;
            sqlx::query("INSERT INTO media_uploaders(community_id,sha256,pubkey) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
                .bind(lease.community_id.as_uuid()).bind(sha256).bind(actor).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(())
        }).await
    }

    /// Retain an upload before any remote mutation. A preparatory lease can be
    /// abandoned or reaped; once armed, even older reapers see an infinite lease.
    /// The real heartbeat deadline still bounds every new write and renewal.
    pub async fn begin_media_upload_write(&self, lease: &ServingWriteLease) -> Result<()> {
        self.bounded_serving_lease_sql(super::SERVING_WRITE_LEASE_SQL_TIMEOUT, async |conn| {
            let mut tx = super::serving_lease_transaction(conn).await?;
            super::lock_community_deletion_shared(&mut tx, lease.community_id).await?;
            let changed = sqlx::query(
                "UPDATE community_serving_write_leases l
                 SET media_write_started=true, lease_until='infinity'::timestamptz
                 FROM communities c
                 WHERE l.id=$1 AND l.community_id=$2 AND l.owner=$3 AND l.generation=$4
                   AND l.fence_generation=$5 AND l.operation='media_upload' AND l.actor_pubkey IS NOT NULL
                   AND l.media_lease_until >= now()
                   AND c.id=l.community_id AND c.deleted_at IS NULL
                   AND c.deletion_state IN ('active','quiescing')
                   AND c.deletion_fence_generation=l.fence_generation",
            ).bind(lease.id).bind(lease.community_id.as_uuid()).bind(&lease.owner)
                .bind(lease.generation).bind(lease.fence_generation).execute(&mut *tx).await?.rows_affected();
            if changed != 1 {
                return Err(DbError::AccessDenied("media upload lease is stale or missing".into()));
            }
            tx.commit().await?;
            Ok(())
        }).await
    }

    /// Register a verified media signer before the final upload access check.
    /// Owner attribution comes from a verified agent assertion, if supplied, or
    /// the same community's authoritative profile. Never pass a client claim.
    pub async fn acquire_media_upload_lease(
        &self,
        community: CommunityId,
        actor: &[u8],
        verified_owner: Option<&[u8]>,
        executor: &str,
        duration: Duration,
    ) -> Result<ServingWriteLease> {
        nostr::PublicKey::from_slice(actor)
            .map_err(|_| DbError::InvalidData("invalid upload signer".into()))?;
        if let Some(owner) = verified_owner {
            nostr::PublicKey::from_slice(owner)
                .map_err(|_| DbError::InvalidData("invalid verified upload owner".into()))?;
        }
        self.acquire_attributed_serving_write_lease(
            community,
            "media_upload",
            executor,
            duration,
            Some(actor),
            verified_owner,
        )
        .await
    }

    /// Observe uploads once, without waiting for any upload to finish.
    ///
    /// This does not block future admission. The caller must first establish
    /// that every serving replica rejects new uploads for this user and its
    /// agents, and that all replicas implement attributed lease tracking.
    /// Read current ownership as well as admission ownership, so reassignment
    /// cannot hide an existing upload from either affected account.
    pub async fn user_media_upload_activity(
        &self,
        pubkey: &str,
    ) -> Result<Vec<MediaUploadActivity>> {
        let pubkey = crate::user_deletion::normalize_user_deletion_pubkey(pubkey)?;
        let actor = hex::decode(pubkey).map_err(|e| DbError::InvalidData(e.to_string()))?;
        self.bounded_serving_lease_sql(super::SERVING_WRITE_LEASE_SQL_TIMEOUT, async |conn| {
            let mut tx = super::serving_lease_transaction(conn).await?;
            let standby: bool = sqlx::query_scalar("SELECT pg_is_in_recovery()")
                .fetch_one(&mut *tx).await?;
            if standby {
                return Err(DbError::DeletionSafety("media drain requires the primary database".into()));
            }
            let rows = sqlx::query_as::<_, MediaUploadActivity>(
                "SELECT l.community_id,
                    count(*) FILTER (WHERE l.actor_pubkey IS NOT NULL AND l.media_lease_until >= now()) AS active,
                    count(*) FILTER (WHERE l.actor_pubkey IS NOT NULL AND l.media_lease_until < now()) AS unresolved,
                    count(*) FILTER (WHERE l.actor_pubkey IS NULL) AS unattributed
                 FROM community_serving_write_leases l
                 WHERE l.operation = 'media_upload' AND (
                    l.actor_pubkey IS NULL OR l.actor_pubkey = $1 OR l.actor_owner_pubkey = $1 OR
                    EXISTS (SELECT 1 FROM users u WHERE u.community_id = l.community_id
                        AND u.pubkey = l.actor_pubkey AND u.agent_owner_pubkey = $1))
                 GROUP BY l.community_id ORDER BY l.community_id LIMIT 1001",
            ).bind(actor).fetch_all(&mut *tx).await?;
            if rows.len() > 1000 {
                return Err(DbError::DeletionSafety("media drain exceeds community limit; no partial success".into()));
            }
            tx.commit().await?;
            Ok(rows)
        }).await
    }
}

#[cfg(test)]
#[path = "media_uploads_postgres_tests.rs"]
mod postgres_tests;
