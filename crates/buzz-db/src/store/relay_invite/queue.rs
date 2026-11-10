//! Short-lived shared claims; results commit with membership and its signed roster.

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};
use tokio::time::timeout_at;
use uuid::Uuid;

use super::{ClaimCredential, ClaimRequest, ClaimResult, MAX_CLAIM_BATCH_SIZE};
use crate::relay_members::MembershipWrite;
use crate::{AdmittedTx, CommunityId, Db, DbError, Result};

const MAX_QUEUED_ROWS: i64 = 2048;
const CLEANUP_BATCH_SIZE: i64 = 64;
const REAP_CANDIDATES: i64 = 64;

/// Maximum time a queued invitation may wait for its atomic claim commit.
pub const INVITE_CLAIM_TTL: Duration = Duration::from_secs(12);

/// Result of a bounded, community-scoped enqueue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnqueuedClaim {
    /// No durable invite matches the presented v2 hash.
    Invalid,
    /// The community's bounded pending/result storage is full.
    Full,
    /// The request was committed to the shared queue.
    Queued(Uuid),
}

/// A durable claim outcome, available only after the claim transaction commits.
#[derive(Clone, Serialize, Deserialize)]
pub enum StoredClaimResult {
    /// The protocol-specific outcome, including ordinary v2 denials.
    Success(ClaimResult),
    /// The verified owner conflicts with its recorded identity.
    OwnerMismatch,
    /// A legacy claimant or verified owner is restricted.
    Restricted,
    /// The unprocessed claim's twelve-second lifetime elapsed.
    Expired,
}

/// A completed or expired request in the caller's community.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompletedClaim {
    /// Identifier returned by enqueue.
    pub id: Uuid,
    /// Committed outcome, or expiry of an unprocessed request.
    pub result: StoredClaimResult,
}

/// One committed claim whose follow-up belongs to the shared processor.
pub struct ProcessedInviteClaim {
    /// Durable queue identifier.
    pub id: Uuid,
    /// Canonical claimant key, decoded at the database boundary.
    pub pubkey: [u8; 32],
    /// Outcome committed with membership and its signed roster.
    pub result: StoredClaimResult,
}

/// Owns one global batch through follow-up and readiness publication.
/// Dropping it releases the processor session; a successor can recover live work.
pub struct ProcessedInviteBatch {
    /// Claims requiring one coalesced follow-up, including ordinary denials.
    pub claims: Vec<ProcessedInviteClaim>,
    /// Conservative monotonic deadline derived from the database's claim expiry.
    pub deadline: Instant,
    pool: sqlx::PgPool,
    community: CommunityId,
    _processor: crate::SessionLeader,
}

impl ProcessedInviteBatch {
    /// Publish readiness after the global follow-up attempt, then release ownership.
    /// Pending default-channel tokens remain independently recoverable after failure.
    pub async fn complete(self) -> Result<()> {
        let Self {
            claims,
            deadline,
            pool,
            community,
            _processor: processor,
            ..
        } = self;
        let ids: Vec<Uuid> = claims.iter().map(|claim| claim.id).collect();
        let operation = async move {
            let mut tx = crate::begin_community_write_transaction(
                &pool,
                community,
                crate::observability::WriterOperation::Authorization,
            )
            .await?;
            sqlx::query("UPDATE relay_invite_claims SET completed_at=clock_timestamp() WHERE community_id=$1 AND id=ANY($2) AND result IS NOT NULL AND completed_at IS NULL")
                .bind(community.as_uuid()).bind(ids).execute(tx.conn()).await?;
            tx.commit().await
        };
        let result = timeout_at(deadline.into(), operation).await;
        drop(processor);
        match result {
            Ok(result) => result,
            Err(_) => Err(DbError::InvalidData(
                "invite follow-up readiness deadline elapsed".into(),
            )),
        }
    }
}

fn processor_key(community: CommunityId) -> i64 {
    let mut hash = Sha256::new();
    hash.update(b"invite-processor:");
    hash.update(community.as_uuid().as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(bytes)
}

async fn cleanup(tx: &mut AdmittedTx) -> Result<u64> {
    let community = tx.community();
    Ok(sqlx::query(
        "DELETE FROM relay_invite_claims WHERE community_id = $1 AND id IN (\
         SELECT id FROM relay_invite_claims WHERE community_id = $1 \
         AND ((result IS NULL AND expires_at <= clock_timestamp()) \
              OR (result IS NOT NULL AND completed_at IS NULL \
                  AND expires_at <= clock_timestamp() - interval '60 seconds') \
              OR completed_at <= clock_timestamp() - interval '60 seconds') \
         ORDER BY queue_order, id LIMIT $2 FOR UPDATE SKIP LOCKED)",
    )
    .bind(community.as_uuid())
    .bind(CLEANUP_BATCH_SIZE)
    .execute(tx.conn())
    .await?
    .rows_affected())
}

fn stored_result(result: &Result<ClaimResult>) -> Result<StoredClaimResult> {
    match result {
        Ok(result) => Ok(StoredClaimResult::Success(result.clone())),
        Err(DbError::InviteOwnerMismatch) => Ok(StoredClaimResult::OwnerMismatch),
        Err(DbError::InviteRestricted) => Ok(StoredClaimResult::Restricted),
        Err(_) => Err(DbError::InvalidData(
            "unexpected per-claim queue outcome".into(),
        )),
    }
}

fn queued_pubkey(value: &str) -> Result<[u8; 32]> {
    hex::decode(value)
        .map_err(|_| DbError::InvalidData("invalid queued claimant key".into()))?
        .try_into()
        .map_err(|_| DbError::InvalidData("invalid queued claimant key".into()))
}

impl Db {
    /// Queue a caller-verified claim without persisting bearer codes or auth data.
    /// Unknown v2 hashes are rejected before queue serialization or insertion.
    #[datastore_span(name = "enqueue_invite_claim", system = "postgresql")]
    pub async fn enqueue_invite_claim(
        &self,
        community: CommunityId,
        request: &ClaimRequest,
    ) -> Result<EnqueuedClaim> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        if let ClaimCredential::V2 { token_hash } = &request.credential {
            if !super::invite_known(&mut tx, community, token_hash).await? {
                tx.rollback().await?;
                return Ok(EnqueuedClaim::Invalid);
            }
        }
        // Every enqueue holds this short lock through count and insert. Claim
        // processing uses the separate, existing invitation admission lock.
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('invite-queue:' || $1::text, 0))",
        )
        .bind(community.as_uuid())
        .execute(tx.conn())
        .await?;
        cleanup(&mut tx).await?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM relay_invite_claims WHERE community_id = $1")
                .bind(community.as_uuid())
                .fetch_one(tx.conn())
                .await?;
        if count >= MAX_QUEUED_ROWS {
            tx.commit().await?;
            return Ok(EnqueuedClaim::Full);
        }
        let id = Uuid::new_v4();
        sqlx::query(
            "WITH timing AS (SELECT clock_timestamp() AS enqueued) \
             INSERT INTO relay_invite_claims (community_id, id, request, queued_at, expires_at) \
             SELECT $1, $2, $3, enqueued, enqueued + $4 * interval '1 second' FROM timing",
        )
        .bind(community.as_uuid())
        .bind(id)
        .bind(serde_json::to_value(request)?)
        .bind(INVITE_CLAIM_TTL.as_secs_f64())
        .execute(tx.conn())
        .await?;
        tx.commit().await?;
        Ok(EnqueuedClaim::Queued(id))
    }

    /// Commit up to [`MAX_CLAIM_BATCH_SIZE`] pending claims and their signed roster.
    /// Admission is independent of channel follow-up already owned by a finisher.
    /// `None` means admission is busy, no live pending requests remain, or the
    /// selected transaction expired and rolled back.
    #[datastore_span(name = "try_admit_invite_claims", system = "postgresql")]
    pub async fn try_admit_invite_claims(
        &self,
        community: CommunityId,
        relay_keypair: &nostr::Keys,
    ) -> Result<Option<MembershipWrite<Vec<ProcessedInviteClaim>>>> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let admitted: bool = sqlx::query_scalar(
            "SELECT pg_try_advisory_xact_lock(hashtextextended('invite-admission:' || $1::text, 0))",
        ).bind(community.as_uuid()).fetch_one(tx.conn()).await?;
        if !admitted {
            tx.rollback().await?;
            return Ok(None);
        }
        let observed = Instant::now();
        let rows = sqlx::query(
            "SELECT id, request, expires_at, clock_timestamp() AS observed_at \
             FROM relay_invite_claims WHERE community_id=$1 AND result IS NULL \
             AND expires_at>clock_timestamp() ORDER BY queue_order,id LIMIT $2 FOR UPDATE",
        )
        .bind(community.as_uuid())
        .bind(MAX_CLAIM_BATCH_SIZE as i64)
        .fetch_all(tx.conn())
        .await?;
        if rows.is_empty() {
            tx.rollback().await?;
            return Ok(None);
        }
        let mut ids = Vec::with_capacity(rows.len());
        let mut requests = Vec::with_capacity(rows.len());
        let mut pubkeys = Vec::with_capacity(rows.len());
        let mut lifetime = Duration::MAX;
        for row in rows {
            ids.push(row.try_get::<Uuid, _>("id")?);
            let request: ClaimRequest = serde_json::from_value(row.try_get("request")?)?;
            pubkeys.push(queued_pubkey(&request.pubkey)?);
            requests.push(request);
            let expires: DateTime<Utc> = row.try_get("expires_at")?;
            let now: DateTime<Utc> = row.try_get("observed_at")?;
            lifetime = lifetime.min((expires - now).to_std().unwrap_or_default());
        }
        // Query, transfer and decode time consume the database expiry budget.
        let deadline = observed + lifetime;
        let operation = async move {
            let applied = super::apply_claim_batch(&mut tx, &requests, relay_keypair).await?;
            let results = applied
                .write
                .outcome
                .iter()
                .map(stored_result)
                .collect::<Result<Vec<_>>>()?;
            let values = results
                .iter()
                .map(serde_json::to_value)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            sqlx::query(
                "UPDATE relay_invite_claims AS claim SET result=outcome.result \
                 FROM unnest($2::uuid[],$3::jsonb[]) AS outcome(id,result) \
                 WHERE claim.community_id=$1 AND claim.id=outcome.id",
            )
            .bind(community.as_uuid())
            .bind(&ids)
            .bind(&values)
            .execute(tx.conn())
            .await?;
            tx.commit().await?;
            let claims = ids
                .into_iter()
                .zip(pubkeys)
                .zip(results)
                .map(|((id, pubkey), result)| ProcessedInviteClaim { id, pubkey, result })
                .collect();
            Ok(MembershipWrite {
                outcome: claims,
                snapshot: applied.into_committed().snapshot,
            })
        };
        match timeout_at(deadline.into(), operation).await {
            Ok(result) => result.map(Some),
            Err(_) => Ok(None),
        }
    }

    /// Own the oldest live admitted claims through follow-up and readiness.
    /// Dropping the batch makes the same durable outcomes recoverable by the
    /// next finisher without applying membership or consuming invitations again.
    #[datastore_span(name = "try_finish_invite_claims", system = "postgresql")]
    pub async fn try_finish_invite_claims(
        &self,
        community: CommunityId,
    ) -> Result<Option<ProcessedInviteBatch>> {
        let Some(processor) = self
            .try_lock_session_leader(processor_key(community))
            .await?
        else {
            return Ok(None);
        };
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let observed = Instant::now();
        let rows = sqlx::query(
            "SELECT id, request->>'pubkey' AS pubkey, result, expires_at, clock_timestamp() AS observed_at \
             FROM relay_invite_claims WHERE community_id=$1 AND completed_at IS NULL \
             AND result IS NOT NULL AND expires_at>clock_timestamp() \
             ORDER BY queue_order,id LIMIT $2",
        )
        .bind(community.as_uuid()).bind(MAX_CLAIM_BATCH_SIZE as i64)
        .fetch_all(tx.conn()).await?;
        if rows.is_empty() {
            tx.rollback().await?;
            return Ok(None);
        }
        let mut claims = Vec::with_capacity(rows.len());
        let mut lifetime = Duration::MAX;
        for row in rows {
            claims.push(ProcessedInviteClaim {
                id: row.try_get("id")?,
                pubkey: queued_pubkey(row.try_get::<&str, _>("pubkey")?)?,
                result: serde_json::from_value(row.try_get("result")?)?,
            });
            let expires: DateTime<Utc> = row.try_get("expires_at")?;
            let now: DateTime<Utc> = row.try_get("observed_at")?;
            lifetime = lifetime.min((expires - now).to_std().unwrap_or_default());
        }
        let deadline = observed + lifetime;
        tx.commit().await?;
        Ok(Some(ProcessedInviteBatch {
            claims,
            deadline,
            pool: self.pool.clone(),
            community,
            _processor: processor,
        }))
    }

    /// Read only stored outcomes or expired unprocessed IDs from this community.
    /// Pending and missing IDs are omitted; this read always uses the writer.
    #[datastore_span(name = "invite_claim_results", system = "postgresql")]
    pub async fn invite_claim_results(
        &self,
        community: CommunityId,
        ids: &[Uuid],
    ) -> Result<Vec<CompletedClaim>> {
        if ids.len() > MAX_QUEUED_ROWS as usize {
            return Err(DbError::InvalidData(
                "too many invite claim result IDs".into(),
            ));
        }
        let rows = sqlx::query(
            "SELECT id, result, completed_at, expires_at <= clock_timestamp() AS expired \
             FROM relay_invite_claims WHERE community_id = $1 AND id = ANY($2)",
        )
        .bind(community.as_uuid())
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;
        let mut completed = Vec::new();
        for row in rows {
            let value: Option<serde_json::Value> = row.try_get("result")?;
            let expired: bool = row.try_get("expired")?;
            let ready: Option<DateTime<Utc>> = row.try_get("completed_at")?;
            let result = match value {
                Some(value) if ready.is_some() || expired => serde_json::from_value(value)?,
                None if expired => StoredClaimResult::Expired,
                _ => continue,
            };
            completed.push(CompletedClaim {
                id: row.try_get("id")?,
                result,
            });
        }
        Ok(completed)
    }

    /// Remove bounded batches of expired queue/result rows from active tenants.
    #[datastore_span(name = "reap_expired_invite_claims", system = "postgresql")]
    pub async fn reap_expired_invite_claims(&self) -> Result<u64> {
        let communities: Vec<Uuid> = sqlx::query_scalar(
            "(SELECT claim.community_id FROM relay_invite_claims claim \
             JOIN communities community ON community.id=claim.community_id \
             WHERE community.deletion_state='active' AND community.deleted_at IS NULL \
             AND claim.result IS NULL AND claim.expires_at<=clock_timestamp() \
             ORDER BY claim.expires_at,claim.community_id LIMIT $1) \
             UNION ALL \
             (SELECT claim.community_id FROM relay_invite_claims claim \
             JOIN communities community ON community.id=claim.community_id \
             WHERE community.deletion_state='active' AND community.deleted_at IS NULL \
             AND claim.result IS NOT NULL AND claim.completed_at IS NULL \
             AND claim.expires_at<=clock_timestamp()-interval '60 seconds' \
             ORDER BY claim.expires_at,claim.community_id LIMIT $1) \
             UNION ALL \
             (SELECT claim.community_id FROM relay_invite_claims claim \
             JOIN communities community ON community.id=claim.community_id \
             WHERE community.deletion_state='active' AND community.deleted_at IS NULL \
             AND claim.completed_at<=clock_timestamp()-interval '60 seconds' \
             ORDER BY claim.completed_at,claim.community_id LIMIT $1)",
        )
        .bind(REAP_CANDIDATES)
        .fetch_all(&self.pool)
        .await?;
        let communities = communities.into_iter().collect::<BTreeSet<_>>();
        let counts = crate::sweep_admitted_communities(
            &self.pool,
            communities,
            crate::observability::WriterOperation::Maintenance,
            "invite_claim_reap",
            |_, mut tx| async move {
                let count = cleanup(&mut tx).await?;
                tx.commit().await?;
                Ok(count)
            },
        )
        .await;
        Ok(counts.into_iter().sum())
    }
}
