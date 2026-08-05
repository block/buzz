//! Durable, current-only client verification-status revisions.
//!
//! Allocation is transaction-owned: the exact active binding, membership,
//! invalidation generation/floors, database-clock freshness, revision row,
//! authority epoch, and idempotency receipt are committed together.

use buzz_core::CommunityId;
use sqlx::{Postgres, Row, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::authorization_invalidation::AuthorizationSelector;
use crate::Db;

const CURRENT_KIND: &str = "client.status.current.v1";
const WITHDRAW_KIND: &str = "client.status.withdraw.v1";

/// Exact private requirement for one current-status issuance.
pub struct CurrentStatusAllocation<'a> {
    /// Server-resolved authorization domain.
    pub community_id: CommunityId,
    /// Exact event-author key.
    pub event_author_pubkey: &'a [u8; 32],
    /// Stable active binding ID.
    pub binding_id: Uuid,
    /// Exact positive binding version.
    pub binding_version: u64,
    /// Opaque current provider policy version.
    pub policy_version: &'a str,
    /// Invalidation generation captured before provider evaluation.
    pub evaluation_generation: u64,
    /// Database-clock freshness boundary.
    pub fresh_until: u64,
    /// Stable issuance operation ID.
    pub operation_id: Uuid,
    /// Exact event-input fingerprint.
    pub request_fingerprint: [u8; 32],
}

/// Exact private requirement for an opaque withdrawal.
pub struct WithdrawalStatusAllocation<'a> {
    /// Server-resolved authorization domain.
    pub community_id: CommunityId,
    /// Exact event-author key.
    pub event_author_pubkey: &'a [u8; 32],
    /// Revision of the actual current issuance being withdrawn.
    pub supersedes_revision: u64,
    /// Fingerprint of the durable current issuance receipt.
    pub issuance_fingerprint: [u8; 32],
    /// Stable withdrawal operation ID.
    pub operation_id: Uuid,
    /// Exact withdrawal-input fingerprint.
    pub request_fingerprint: [u8; 32],
}

/// Result of a transaction-owned revision allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocatedStatusRevision {
    /// Strictly positive revision.
    pub revision: u64,
    /// Domain-wide durable status floor after this allocation.
    pub floor: u64,
}

/// Allocation failure classified by whether PostgreSQL commit was attempted.
#[derive(Debug, Error)]
pub enum ClientStatusAllocationError {
    /// Current private authority did not satisfy the exact requirement.
    #[error("client status authority is not current")]
    NotCurrent,
    /// A stable operation ID was reused for different input.
    #[error("client status operation conflicts with a committed request")]
    ConflictingRetry,
    /// Input could not be represented safely.
    #[error("client status allocation input is invalid")]
    InvalidInput,
    /// PostgreSQL failed before commit was attempted; the transaction rolls back.
    #[error("client status allocation failed before commit")]
    Database(#[source] sqlx::Error),
    /// PostgreSQL commit acknowledgement was ambiguous. The receipt decides.
    #[error("client status commit acknowledgement is ambiguous")]
    CommitUnknown(#[source] sqlx::Error),
}

impl From<sqlx::Error> for ClientStatusAllocationError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl Db {
    /// Read an exact committed status allocation after ambiguous commit acknowledgement.
    pub async fn committed_status_revision(
        &self,
        community_id: CommunityId,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
    ) -> Result<Option<AllocatedStatusRevision>, ClientStatusAllocationError> {
        let row = sqlx::query(
            "SELECT request_fingerprint, result_payload FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(operation_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let fingerprint: Vec<u8> = row.try_get("request_fingerprint")?;
        let payload: Vec<u8> = row.try_get("result_payload")?;
        if fingerprint.as_slice() != request_fingerprint {
            return Err(ClientStatusAllocationError::ConflictingRetry);
        }
        Ok(Some(decode_receipt_payload(&payload)?))
    }

    /// Allocate or replay one strictly monotonic current-status revision.
    pub async fn allocate_current_status_revision(
        &self,
        request: CurrentStatusAllocation<'_>,
    ) -> Result<AllocatedStatusRevision, ClientStatusAllocationError> {
        validate_common(
            request.community_id,
            request.event_author_pubkey,
            request.operation_id,
            request.binding_version,
        )?;
        if request.policy_version.is_empty()
            || request.evaluation_generation > i64::MAX as u64
            || request.fresh_until > i64::MAX as u64
        {
            return Err(ClientStatusAllocationError::InvalidInput);
        }
        let mut tx = self.begin_transaction().await.map_err(db_error)?;
        lock_scope(&mut tx, request.community_id, request.event_author_pubkey).await?;
        let (issuer, subject) = validate_current_authority(&mut tx, &request).await?;
        validate_invalidation(
            &mut tx,
            request.community_id,
            request.evaluation_generation,
            request.binding_id,
            request.binding_version,
            request.event_author_pubkey,
            request.policy_version,
            &issuer,
            &subject,
        )
        .await?;
        if let Some(revision) = replay_revision(
            &mut tx,
            request.community_id,
            request.operation_id,
            CURRENT_KIND,
            request.request_fingerprint,
        )
        .await?
        {
            tx.commit()
                .await
                .map_err(ClientStatusAllocationError::CommitUnknown)?;
            return Ok(revision);
        }
        let allocated = next_revision(&mut tx, request.community_id).await?;
        sqlx::query(
            "INSERT INTO client_status_revisions \
             (community_id, event_author_pubkey, revision, disposition, binding_id, binding_version) \
             VALUES ($1, $2, $3, 'current', $4, $5) \
         ON CONFLICT (community_id, event_author_pubkey) DO UPDATE SET \
           revision=EXCLUDED.revision, disposition='current', binding_id=EXCLUDED.binding_id, \
           binding_version=EXCLUDED.binding_version, supersedes_revision=NULL, \
           updated_at=clock_timestamp()",
        )
        .bind(request.community_id.as_uuid())
        .bind(request.event_author_pubkey.as_slice())
        .bind(allocated as i64)
        .bind(request.binding_id)
        .bind(request.binding_version as i64)
        .execute(&mut *tx)
        .await?;
        insert_receipt(
            &mut tx,
            request.community_id,
            request.operation_id,
            CURRENT_KIND,
            request.request_fingerprint,
            allocated,
        )
        .await?;
        tx.commit()
            .await
            .map_err(ClientStatusAllocationError::CommitUnknown)?;
        Ok(AllocatedStatusRevision {
            revision: allocated,
            floor: allocated,
        })
    }

    /// Allocate or replay a withdrawal strictly after its exact current receipt.
    pub async fn allocate_withdrawn_status_revision(
        &self,
        request: WithdrawalStatusAllocation<'_>,
    ) -> Result<AllocatedStatusRevision, ClientStatusAllocationError> {
        validate_common(
            request.community_id,
            request.event_author_pubkey,
            request.operation_id,
            request.supersedes_revision,
        )?;
        let mut tx = self.begin_transaction().await.map_err(db_error)?;
        lock_scope(&mut tx, request.community_id, request.event_author_pubkey).await?;
        if let Some(revision) = replay_revision(
            &mut tx,
            request.community_id,
            request.operation_id,
            WITHDRAW_KIND,
            request.request_fingerprint,
        )
        .await?
        {
            tx.commit()
                .await
                .map_err(ClientStatusAllocationError::CommitUnknown)?;
            return Ok(revision);
        }
        let issuance_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_kind=$2 AND request_fingerprint=$3 \
               AND octet_length(result_payload) IN (8,16) \
               AND substring(result_payload FROM 1 FOR 8)=$4)",
        )
        .bind(request.community_id.as_uuid())
        .bind(CURRENT_KIND)
        .bind(request.issuance_fingerprint.as_slice())
        .bind(request.supersedes_revision.to_be_bytes().as_slice())
        .fetch_one(&mut *tx)
        .await?;
        if !issuance_exists {
            return Err(ClientStatusAllocationError::NotCurrent);
        }
        let row: Option<(i64, String, Option<i64>)> = sqlx::query_as(
            "SELECT revision, disposition, supersedes_revision FROM client_status_revisions \
             WHERE community_id=$1 AND event_author_pubkey=$2 FOR UPDATE",
        )
        .bind(request.community_id.as_uuid())
        .bind(request.event_author_pubkey.as_slice())
        .fetch_optional(&mut *tx)
        .await?;
        let Some((revision, disposition, prior_supersedes)) = row else {
            return Err(ClientStatusAllocationError::NotCurrent);
        };
        let receipt_revision = request.supersedes_revision as i64;
        let superseded_revision = if disposition == "current" {
            if receipt_revision > revision {
                return Err(ClientStatusAllocationError::NotCurrent);
            }
            revision
        } else if disposition == "withdrawn"
            && prior_supersedes.is_some_and(|superseded| receipt_revision <= superseded)
            && revision > receipt_revision
        {
            prior_supersedes.expect("withdrawn status has a superseded revision")
        } else {
            return Err(ClientStatusAllocationError::NotCurrent);
        };
        let floor: i64 = sqlx::query_scalar(
            "SELECT status_revision FROM authorization_authority_epochs \
             WHERE community_id=$1 FOR UPDATE",
        )
        .bind(request.community_id.as_uuid())
        .fetch_one(&mut *tx)
        .await?;
        let allocated = if disposition == "current" || revision < floor {
            next_revision(&mut tx, request.community_id).await?
        } else {
            revision as u64
        };
        if allocated <= superseded_revision as u64 {
            return Err(ClientStatusAllocationError::NotCurrent);
        }
        sqlx::query(
            "UPDATE client_status_revisions SET revision=$3, disposition='withdrawn', \
             binding_id=NULL, binding_version=NULL, supersedes_revision=$4, \
             updated_at=clock_timestamp() \
             WHERE community_id=$1 AND event_author_pubkey=$2",
        )
        .bind(request.community_id.as_uuid())
        .bind(request.event_author_pubkey.as_slice())
        .bind(allocated as i64)
        .bind(superseded_revision)
        .execute(&mut *tx)
        .await?;
        insert_receipt(
            &mut tx,
            request.community_id,
            request.operation_id,
            WITHDRAW_KIND,
            request.request_fingerprint,
            allocated,
        )
        .await?;
        tx.commit()
            .await
            .map_err(ClientStatusAllocationError::CommitUnknown)?;
        Ok(AllocatedStatusRevision {
            revision: allocated,
            floor: allocated,
        })
    }
}

fn validate_common(
    community_id: CommunityId,
    pubkey: &[u8; 32],
    operation_id: Uuid,
    positive: u64,
) -> Result<(), ClientStatusAllocationError> {
    if community_id.as_uuid().is_nil()
        || operation_id.is_nil()
        || positive == 0
        || positive > i64::MAX as u64
        || pubkey.iter().all(|byte| *byte == 0)
    {
        return Err(ClientStatusAllocationError::InvalidInput);
    }
    Ok(())
}

fn db_error(error: crate::DbError) -> ClientStatusAllocationError {
    match error {
        crate::DbError::Sqlx(error) => ClientStatusAllocationError::Database(error),
        _ => ClientStatusAllocationError::InvalidInput,
    }
}

async fn lock_scope(
    tx: &mut Transaction<'static, Postgres>,
    community_id: CommunityId,
    pubkey: &[u8; 32],
) -> Result<(), ClientStatusAllocationError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!(
            "client-status:{}:{}",
            community_id,
            hex::encode(pubkey)
        ))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn validate_current_authority(
    tx: &mut Transaction<'static, Postgres>,
    request: &CurrentStatusAllocation<'_>,
) -> Result<(String, String), ClientStatusAllocationError> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT binding.issuer, binding.uid FROM identity_bindings binding \
         JOIN identity_principals principal ON principal.community_id=binding.community_id \
           AND principal.issuer=binding.issuer AND principal.uid=binding.uid \
         JOIN relay_members member ON member.community_id=binding.community_id \
           AND member.pubkey=encode(binding.pubkey, 'hex') \
         WHERE binding.community_id=$1 AND binding.binding_id=$2 AND binding.pubkey=$3 \
           AND binding.binding_version=$4 AND binding.binding_state='active' \
           AND principal.disabled_at IS NULL \
           AND NOT EXISTS (SELECT 1 FROM identity_revoked_keys revoked \
             WHERE revoked.community_id=binding.community_id AND revoked.pubkey=binding.pubkey) \
         FOR SHARE OF binding, principal, member",
    )
    .bind(request.community_id.as_uuid())
    .bind(request.binding_id)
    .bind(request.event_author_pubkey.as_slice())
    .bind(request.binding_version as i64)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(principal) = row else {
        return Err(ClientStatusAllocationError::NotCurrent);
    };
    let fresh: bool =
        sqlx::query_scalar("SELECT clock_timestamp() < to_timestamp($1::double precision)")
            .bind(request.fresh_until as f64)
            .fetch_one(&mut **tx)
            .await?;
    if !fresh {
        return Err(ClientStatusAllocationError::NotCurrent);
    }
    Ok(principal)
}

#[allow(clippy::too_many_arguments)]
async fn validate_invalidation(
    tx: &mut Transaction<'static, Postgres>,
    community_id: CommunityId,
    evaluation_generation: u64,
    binding_id: Uuid,
    binding_version: u64,
    pubkey: &[u8; 32],
    policy_version: &str,
    issuer: &str,
    subject: &str,
) -> Result<(), ClientStatusAllocationError> {
    let generation: Option<i64> = sqlx::query_scalar(
        "SELECT generation FROM authorization_invalidation_domains \
         WHERE community_id=$1 FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await?;
    if generation != Some(evaluation_generation as i64) {
        return Err(ClientStatusAllocationError::NotCurrent);
    }
    let selectors = [
        AuthorizationSelector::domain(),
        AuthorizationSelector::principal(issuer, subject)
            .map_err(|_| ClientStatusAllocationError::InvalidInput)?,
        AuthorizationSelector::nostr_key(*pubkey),
        AuthorizationSelector::binding(binding_id, binding_version)
            .map_err(|_| ClientStatusAllocationError::InvalidInput)?,
        AuthorizationSelector::policy_version(policy_version)
            .map_err(|_| ClientStatusAllocationError::InvalidInput)?,
    ];
    for selector in selectors {
        let row = sqlx::query(
            "SELECT generation, sticky_deny, binding_version_floor \
             FROM authorization_invalidation_floors WHERE community_id=$1 \
               AND selector_kind=$2 AND selector_fingerprint=$3 FOR SHARE",
        )
        .bind(community_id.as_uuid())
        .bind(selector.kind().as_str())
        .bind(selector.fingerprint().as_slice())
        .fetch_optional(&mut **tx)
        .await?;
        let Some(row) = row else { continue };
        let floor_generation: i64 = row.try_get("generation")?;
        let sticky: bool = row.try_get("sticky_deny")?;
        let version_floor: Option<i64> = row.try_get("binding_version_floor")?;
        if sticky
            || floor_generation > evaluation_generation as i64
            || version_floor.is_some_and(|floor| binding_version <= floor as u64)
        {
            return Err(ClientStatusAllocationError::NotCurrent);
        }
    }
    Ok(())
}

async fn replay_revision(
    tx: &mut Transaction<'static, Postgres>,
    community_id: CommunityId,
    operation_id: Uuid,
    operation_kind: &str,
    request_fingerprint: [u8; 32],
) -> Result<Option<AllocatedStatusRevision>, ClientStatusAllocationError> {
    let row = sqlx::query(
        "SELECT operation_kind, request_fingerprint, result_payload \
         FROM authorization_operation_receipts \
         WHERE community_id=$1 AND operation_id=$2 FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let kind: String = row.try_get("operation_kind")?;
    let fingerprint: Vec<u8> = row.try_get("request_fingerprint")?;
    let payload: Vec<u8> = row.try_get("result_payload")?;
    if kind != operation_kind || fingerprint.as_slice() != request_fingerprint {
        return Err(ClientStatusAllocationError::ConflictingRetry);
    }
    Ok(Some(decode_receipt_payload(&payload)?))
}

fn decode_receipt_payload(
    payload: &[u8],
) -> Result<AllocatedStatusRevision, ClientStatusAllocationError> {
    let (revision_bytes, floor_bytes) = match payload.len() {
        // Compatibility with receipts written before allocation-time floors
        // were retained. The allocation revision was also its floor.
        8 => (&payload[..8], &payload[..8]),
        16 => (&payload[..8], &payload[8..16]),
        _ => return Err(ClientStatusAllocationError::ConflictingRetry),
    };
    let revision = u64::from_be_bytes(
        revision_bytes
            .try_into()
            .map_err(|_| ClientStatusAllocationError::ConflictingRetry)?,
    );
    let floor = u64::from_be_bytes(
        floor_bytes
            .try_into()
            .map_err(|_| ClientStatusAllocationError::ConflictingRetry)?,
    );
    if revision == 0 || floor == 0 || revision < floor {
        return Err(ClientStatusAllocationError::ConflictingRetry);
    }
    Ok(AllocatedStatusRevision { revision, floor })
}

async fn next_revision(
    tx: &mut Transaction<'static, Postgres>,
    community_id: CommunityId,
) -> Result<u64, ClientStatusAllocationError> {
    let revision: i64 = sqlx::query_scalar(
        "UPDATE authorization_authority_epochs SET \
           authority_epoch=authority_epoch+1, status_revision=status_revision+1, \
           updated_at=clock_timestamp() WHERE community_id=$1 RETURNING status_revision",
    )
    .bind(community_id.as_uuid())
    .fetch_one(&mut **tx)
    .await?;
    u64::try_from(revision).map_err(|_| ClientStatusAllocationError::InvalidInput)
}

async fn insert_receipt(
    tx: &mut Transaction<'static, Postgres>,
    community_id: CommunityId,
    operation_id: Uuid,
    operation_kind: &str,
    request_fingerprint: [u8; 32],
    revision: u64,
) -> Result<(), ClientStatusAllocationError> {
    let mut result_payload = Vec::with_capacity(16);
    result_payload.extend_from_slice(&revision.to_be_bytes());
    result_payload.extend_from_slice(&revision.to_be_bytes());
    sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id, operation_id, operation_kind, request_fingerprint, \
          result_version, result_payload, lease_expires_at) \
         VALUES ($1,$2,$3,$4,1,$5,clock_timestamp()+interval '100 years')",
    )
    .bind(community_id.as_uuid())
    .bind(operation_id)
    .bind(operation_kind)
    .bind(request_fingerprint.as_slice())
    .bind(result_payload)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn current_replay_withdrawal_and_revocation_are_transaction_owned() {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_owned());
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("test database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrations");
        let db = Db::from_pool(pool);
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let binding_id = Uuid::new_v4();
        let author = [0x41; 32];
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("status-{}.example", community.as_uuid()))
            .execute(&db.pool)
            .await
            .expect("community");
        sqlx::query("INSERT INTO identity_principals (community_id,issuer,uid) VALUES ($1,$2,$3)")
            .bind(community.as_uuid())
            .bind("https://idp.example")
            .bind("subject")
            .execute(&db.pool)
            .await
            .expect("principal");
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id,issuer,uid,pubkey,source,binding_id,binding_version, \
              binding_state,binding_provenance) \
             VALUES ($1,$2,$3,$4,'jwt_npub',$5,1,'active','attested_key')",
        )
        .bind(community.as_uuid())
        .bind("https://idp.example")
        .bind("subject")
        .bind(author.as_slice())
        .bind(binding_id)
        .execute(&db.pool)
        .await
        .expect("binding");
        sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'member')")
            .bind(community.as_uuid())
            .bind(hex::encode(author))
            .execute(&db.pool)
            .await
            .expect("member");
        sqlx::query(
            "INSERT INTO authorization_invalidation_domains (community_id) VALUES ($1) \
             ON CONFLICT (community_id) DO NOTHING",
        )
        .bind(community.as_uuid())
        .execute(&db.pool)
        .await
        .expect("invalidation domain");
        let generation: i64 = sqlx::query_scalar(
            "SELECT generation FROM authorization_invalidation_domains WHERE community_id=$1",
        )
        .bind(community.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("generation");
        let fresh_until = chrono::Utc::now().timestamp() as u64 + 300;
        let operation_id = Uuid::new_v4();
        let allocate = |operation_id, fingerprint| CurrentStatusAllocation {
            community_id: community,
            event_author_pubkey: &author,
            binding_id,
            binding_version: 1,
            policy_version: "policy-v1",
            evaluation_generation: generation as u64,
            fresh_until,
            operation_id,
            request_fingerprint: fingerprint,
        };
        let first = db
            .allocate_current_status_revision(allocate(operation_id, [1; 32]))
            .await
            .expect("first current");
        let replay = db
            .allocate_current_status_revision(allocate(operation_id, [1; 32]))
            .await
            .expect("exact replay");
        assert_eq!(first, replay);
        let second = db
            .allocate_current_status_revision(allocate(Uuid::new_v4(), [2; 32]))
            .await
            .expect("new issuance");
        assert!(second.revision > first.revision);
        let withdrawal_operation = Uuid::new_v4();
        let withdrawn = db
            .allocate_withdrawn_status_revision(WithdrawalStatusAllocation {
                community_id: community,
                event_author_pubkey: &author,
                supersedes_revision: second.revision,
                issuance_fingerprint: [2; 32],
                operation_id: withdrawal_operation,
                request_fingerprint: [3; 32],
            })
            .await
            .expect("withdrawal");
        assert!(withdrawn.revision > second.revision);
        let fanout = db
            .allocate_withdrawn_status_revision(WithdrawalStatusAllocation {
                community_id: community,
                event_author_pubkey: &author,
                supersedes_revision: first.revision,
                issuance_fingerprint: [1; 32],
                operation_id: Uuid::new_v4(),
                request_fingerprint: [4; 32],
            })
            .await
            .expect("older displayed current receives the same withdrawal");
        assert_eq!(fanout, withdrawn);

        sqlx::query(
            "UPDATE authorization_authority_epochs \
             SET authority_epoch=authority_epoch+1, status_revision=status_revision+1 \
             WHERE community_id=$1",
        )
        .bind(community.as_uuid())
        .execute(&db.pool)
        .await
        .expect("advance unrelated durable status floor");
        let delayed_replay = db
            .allocate_withdrawn_status_revision(WithdrawalStatusAllocation {
                community_id: community,
                event_author_pubkey: &author,
                supersedes_revision: second.revision,
                issuance_fingerprint: [2; 32],
                operation_id: withdrawal_operation,
                request_fingerprint: [3; 32],
            })
            .await
            .expect("exact delayed fan-out replay retains allocation-time floor");
        assert_eq!(delayed_replay, withdrawn);

        let reissued = db
            .allocate_current_status_revision(allocate(Uuid::new_v4(), [6; 32]))
            .await
            .expect("a fresh current status can replace a withdrawn projection");
        assert!(reissued.revision > withdrawn.revision);
        let row: (String, Option<i64>) = sqlx::query_as(
            "SELECT disposition, supersedes_revision FROM client_status_revisions \
             WHERE community_id=$1 AND event_author_pubkey=$2",
        )
        .bind(community.as_uuid())
        .bind(author.as_slice())
        .fetch_one(&db.pool)
        .await
        .expect("reissued projection");
        assert_eq!(row, ("current".to_owned(), None));

        assert!(matches!(
            db.allocate_withdrawn_status_revision(WithdrawalStatusAllocation {
                community_id: community,
                event_author_pubkey: &author,
                supersedes_revision: second.revision,
                issuance_fingerprint: [9; 32],
                operation_id: Uuid::new_v4(),
                request_fingerprint: [5; 32],
            })
            .await,
            Err(ClientStatusAllocationError::NotCurrent)
        ));
    }
}
