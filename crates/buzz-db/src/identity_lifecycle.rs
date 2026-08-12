//! Canonical transaction pre-locking for local identity lifecycle mutations.
//!
//! Lifecycle callers pass semantic coordinates through
//! [`IdentityLifecycleLockCoordinates`] and call
//! [`lock_identity_lifecycle_transaction`] before any lifecycle DML. The SQL
//! advisory-key namespace and numeric ordering remain private to this module;
//! callers must not derive or acquire these locks independently.

use std::fmt;

use buzz_core::CommunityId;
use nostr::PublicKey;
use sqlx::{Postgres, Transaction};

use crate::{DbError, Result};

/// Typed principal and old/new author-key coordinates for one lifecycle
/// transaction.
///
/// A principal may be absent only for key-only inactive revocation. Duplicate
/// old/new keys are accepted and deduplicated before locking.
#[derive(Clone)]
pub struct IdentityLifecycleLockCoordinates {
    community_id: CommunityId,
    principal_fingerprint: Option<[u8; 32]>,
    old_event_author_pubkey: Option<PublicKey>,
    new_event_author_pubkey: Option<PublicKey>,
}

impl IdentityLifecycleLockCoordinates {
    /// Construct canonical lifecycle lock coordinates.
    ///
    /// At least one principal or key coordinate is required. The community is
    /// server-resolved and must not contain the nil sentinel.
    pub fn new(
        community_id: CommunityId,
        principal_fingerprint: Option<[u8; 32]>,
        old_event_author_pubkey: Option<PublicKey>,
        new_event_author_pubkey: Option<PublicKey>,
    ) -> Result<Self> {
        if community_id.as_uuid().is_nil() {
            return Err(DbError::InvalidData(
                "identity lifecycle lock domain must not be nil".to_owned(),
            ));
        }
        if principal_fingerprint.is_none()
            && old_event_author_pubkey.is_none()
            && new_event_author_pubkey.is_none()
        {
            return Err(DbError::InvalidData(
                "identity lifecycle lock requires a principal or key coordinate".to_owned(),
            ));
        }
        Ok(Self {
            community_id,
            principal_fingerprint,
            old_event_author_pubkey,
            new_event_author_pubkey,
        })
    }
}

impl fmt::Debug for IdentityLifecycleLockCoordinates {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("IdentityLifecycleLockCoordinates([REDACTED])")
    }
}

/// Acquire every lifecycle principal and old/new-key advisory lock in one
/// canonical transaction order.
///
/// PostgreSQL derives the same domain-separated signed `BIGINT` keys used by
/// the migration triggers. This helper sorts and deduplicates them before
/// acquiring each transaction-scoped lock. The caller must invoke it before
/// reading lifecycle eligibility or issuing lifecycle DML and retain the same
/// transaction through receipt, history, selector, consumption, and binding
/// writes.
pub async fn lock_identity_lifecycle_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    coordinates: &IdentityLifecycleLockCoordinates,
) -> Result<()> {
    let old_key = coordinates
        .old_event_author_pubkey
        .map(|public_key| public_key.to_bytes().to_vec());
    let new_key = coordinates
        .new_event_author_pubkey
        .map(|public_key| public_key.to_bytes().to_vec());
    let principal = coordinates
        .principal_fingerprint
        .map(|fingerprint| fingerprint.to_vec());
    let lock_keys: Vec<i64> = sqlx::query_scalar(
        "WITH coordinates(scope, fingerprint) AS ( \
             VALUES \
                ('principal'::text, $2::bytea), \
                ('key'::text, $3::bytea), \
                ('key'::text, $4::bytea) \
         ) \
         SELECT DISTINCT hashtextextended( \
             'buzz:identity-lifecycle-coordinate:v1:' || scope || ':' || \
             $1::uuid::text || ':' || encode(fingerprint, 'hex'), 0 \
         ) AS lock_key \
         FROM coordinates \
         WHERE fingerprint IS NOT NULL \
         ORDER BY lock_key",
    )
    .bind(coordinates.community_id.as_uuid())
    .bind(principal)
    .bind(old_key)
    .bind(new_key)
    .fetch_all(&mut **transaction)
    .await?;

    for lock_key in lock_keys {
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nostr::Keys;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn coordinates_reject_nil_domain_and_empty_scope() {
        let key = Keys::generate().public_key();
        assert!(IdentityLifecycleLockCoordinates::new(
            CommunityId::from_uuid(Uuid::nil()),
            Some([1_u8; 32]),
            Some(key),
            None,
        )
        .is_err());
        assert!(IdentityLifecycleLockCoordinates::new(
            CommunityId::from_uuid(Uuid::new_v4()),
            None,
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn coordinates_are_redacted_and_accept_duplicate_keys() {
        let key = Keys::generate().public_key();
        let coordinates = IdentityLifecycleLockCoordinates::new(
            CommunityId::from_uuid(Uuid::new_v4()),
            Some([2_u8; 32]),
            Some(key),
            Some(key),
        )
        .expect("valid coordinates");
        assert_eq!(
            format!("{coordinates:?}"),
            "IdentityLifecycleLockCoordinates([REDACTED])"
        );
    }
}
