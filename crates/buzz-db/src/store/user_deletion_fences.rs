//! Operator-global restriction storage and identity locks; no serving or worker
//! path calls these helpers yet. Installing fences does not itself enforce them.
//!
//! Callers use a writer-pool READ COMMITTED transaction. Acquire existing
//! community guards first, then `lock_request` for the worker, then all human
//! locks, then scoped agent locks.
//! Within each group acquire the whole set in one call (sorted here), never
//! upgrade shared locks to exclusive. Gate workers may discover agents after
//! locking humans, then lock the complete agent set. Keep the transaction open
//! through the protected write/checkpoint; commit or rollback releases the locks.

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use nostr::PublicKey;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::entity_deletion::{lock_owned, validate_approved_inventory, EntityLease};
use crate::{DbError, Result};

/// A stable lock/restriction identity, independent of mutable profile rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FenceIdentity {
    pubkey: PublicKey,
    scope: Option<CommunityId>,
}

impl FenceIdentity {
    /// A human key restricted across the operator's hosted communities.
    pub fn user(pubkey: PublicKey) -> Self {
        Self {
            pubkey,
            scope: None,
        }
    }

    /// An agent key restricted only within its ownership community.
    pub fn agent(community: CommunityId, pubkey: PublicKey) -> Self {
        Self {
            pubkey,
            scope: Some(community),
        }
    }

    fn scope_id(self) -> Option<Uuid> {
        self.scope.map(|scope| *scope.as_uuid())
    }

    fn lock_key(self) -> i64 {
        let mut hash = Sha256::new();
        hash.update(b"buzz:user-deletion-fence:v1\0");
        hash.update([u8::from(self.scope.is_some())]);
        if let Some(scope) = self.scope {
            hash.update(scope.as_uuid().as_bytes());
        }
        hash.update(self.pubkey.to_bytes());
        let digest = hash.finalize();
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&digest[..8]);
        i64::from_be_bytes(bytes)
    }
}

/// Durable evidence of the first restriction on an identity and scope.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct UserDeletionFence {
    /// Stable restriction identifier.
    pub id: Uuid,
    /// Canonical lowercase public key.
    pub pubkey: String,
    /// Agent scope, or NULL for a global human restriction.
    pub scope_community_id: Option<Uuid>,
    /// Request that first installed this restriction.
    pub request_id: Uuid,
    /// Verified owner at fencing time; absent for a human restriction.
    pub source_owner_pubkey: Option<String>,
    /// Time of the first insertion; retries preserve it.
    pub blocked_at: DateTime<Utc>,
}

/// Acquire shared identity locks for normal writes. Does not check restrictions.
pub async fn lock_shared(
    tx: &mut Transaction<'_, Postgres>,
    identities: &[FenceIdentity],
) -> Result<()> {
    lock(tx, identities, false).await
}

/// Acquire exclusive identity locks for fencing. Does not install restrictions.
pub async fn lock_exclusive(
    tx: &mut Transaction<'_, Postgres>,
    identities: &[FenceIdentity],
) -> Result<()> {
    lock(tx, identities, true).await
}

async fn lock(
    tx: &mut Transaction<'_, Postgres>,
    identities: &[FenceIdentity],
    exclusive: bool,
) -> Result<()> {
    if identities.len() > 4096 {
        return Err(DbError::DeletionSafety(
            "too many user-deletion identity locks".into(),
        ));
    }
    let (isolation, timeout): (String, String) = sqlx::query_as(
        "SELECT current_setting('transaction_isolation'), current_setting('lock_timeout')",
    )
    .fetch_one(&mut **tx)
    .await?;
    if isolation != "read committed" {
        return Err(DbError::DeletionSafety(
            "user-deletion locks require READ COMMITTED".into(),
        ));
    }
    // Bound each lock wait without widening a caller's shorter timeout. Restore
    // the original setting on success; a SQL error requires caller rollback.
    sqlx::query(
        "SELECT set_config('lock_timeout',
         (EXTRACT(EPOCH FROM LEAST(NULLIF(current_setting('lock_timeout')::interval,
          interval '0'), interval '5 seconds')) * 1000)::bigint::text, true)",
    )
    .execute(&mut **tx)
    .await?;
    let mut keys: Vec<_> = identities
        .iter()
        .map(|identity| (identity.scope.is_some(), identity.lock_key()))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    for (_, key) in keys {
        let query = if exclusive {
            "SELECT pg_advisory_xact_lock($1)"
        } else {
            "SELECT pg_advisory_xact_lock_shared($1)"
        };
        sqlx::query(query).bind(key).execute(&mut **tx).await?;
    }
    sqlx::query("SELECT set_config('lock_timeout', $1, true)")
        .bind(timeout)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Lock and read the exact identity restriction in the caller's transaction.
///
/// For multiple identities, acquire the full shared set first. The caller must
/// refuse a restricted operation and retain this transaction through any write.
/// An agent lookup is scoped; it does not resolve or check the human owner.
/// The planned agent-write path locks the verified owner and agent together,
/// then reads the agent restriction only. This requires the gate to install the
/// human and all owned-agent restrictions atomically, and creation/assignment
/// paths to reject restricted owners. These callers are not wired in yet.
pub async fn find(
    tx: &mut Transaction<'_, Postgres>,
    identity: FenceIdentity,
) -> Result<Option<UserDeletionFence>> {
    lock_shared(tx, &[identity]).await?;
    read(tx, identity).await
}

async fn read(
    tx: &mut Transaction<'_, Postgres>,
    identity: FenceIdentity,
) -> Result<Option<UserDeletionFence>> {
    // Explicit predicates let PostgreSQL use the matching partial unique index.
    let sql = if identity.scope.is_some() {
        "SELECT id, pubkey, scope_community_id, request_id, source_owner_pubkey, blocked_at
         FROM user_deletion_fences WHERE pubkey = $1 AND scope_community_id = $2"
    } else {
        "SELECT id, pubkey, scope_community_id, request_id, source_owner_pubkey, blocked_at
         FROM user_deletion_fences WHERE pubkey = $1 AND scope_community_id IS NULL"
    };
    let query = sqlx::query_as(sql).bind(identity.pubkey.to_hex());
    let query = if let Some(scope) = identity.scope_id() {
        query.bind(scope)
    } else {
        query
    };
    Ok(query.fetch_optional(&mut **tx).await?)
}

/// Lock the parent request and validate the worker's lease and approved inventory.
///
/// Call before acquiring any identity locks in a fencing transaction. Each
/// insertion rechecks this guard; holding the row lock prevents another worker
/// from claiming the request until this transaction ends. The caller must still
/// verify that the identities and agent ownership belong to this request.
pub async fn lock_request(tx: &mut Transaction<'_, Postgres>, lease: &EntityLease) -> Result<()> {
    let request = lock_owned(tx, lease).await?;
    if request.entity_type != "user" {
        return Err(DbError::DeletionSafety(
            "fences require a user deletion request".into(),
        ));
    }
    validate_approved_inventory(tx, &request).await
}

/// Insert an idempotent restriction under a valid worker lease and identity lock.
///
/// For multiple identities, first call `lock_request`, then acquire the complete exclusive
/// identity set in order, verify agent ownership, and save its checkpoint in this
/// same transaction. A retry preserves the first row; conflicting provenance
/// errors. On any error roll back, including other fences/checkpoints.
pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    identity: FenceIdentity,
    lease: &EntityLease,
    source_owner: Option<PublicKey>,
) -> Result<UserDeletionFence> {
    if identity.scope.is_some() != source_owner.is_some() {
        return Err(DbError::DeletionSafety(
            "agent fences require an owner; user fences do not".into(),
        ));
    }
    lock_request(tx, lease).await?;
    let request_id = lease.request_id;
    lock_exclusive(tx, &[identity]).await?;
    let owner = source_owner.map(|key| key.to_hex());
    sqlx::query(
        "INSERT INTO user_deletion_fences (pubkey, scope_community_id, request_id, source_owner_pubkey)
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(identity.pubkey.to_hex()).bind(identity.scope_id()).bind(request_id).bind(&owner)
    .execute(&mut **tx).await?;
    let saved = read(tx, identity)
        .await?
        .ok_or_else(|| DbError::DeletionSafety("fence insertion returned no row".into()))?;
    if saved.request_id != request_id || saved.source_owner_pubkey != owner {
        return Err(DbError::DeletionSafety(
            "identity is already fenced by different provenance".into(),
        ));
    }
    Ok(saved)
}

#[cfg(test)]
#[path = "user_deletion_fences_postgres_tests.rs"]
mod postgres_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_mapping_is_stable_across_processes_and_releases() {
        // A change to these values would split the lock domain during rollout.
        let key =
            PublicKey::from_hex("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .unwrap();
        assert_eq!(FenceIdentity::user(key).lock_key(), -4153709860967480559);
        assert_eq!(
            FenceIdentity::agent(CommunityId::from_uuid(Uuid::from_u128(1)), key).lock_key(),
            -3253902268089525902,
        );
    }
}
