//! Credential-free lifecycle and admission-loss notifications.

use std::fmt;

use buzz_core::CommunityId;
use sqlx::{Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

/// Closed dependency coordinate advanced by a lifecycle transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleInvalidationSelector {
    /// Exact federated principal fingerprint.
    Principal([u8; 32]),
    /// Exact Nostr event-author fingerprint.
    EventAuthor([u8; 32]),
    /// Exact binding generation and its selector fingerprint.
    Binding {
        /// Privacy-safe selector fingerprint.
        fingerprint: [u8; 32],
        /// First invalid binding version.
        version_floor: u64,
    },
    /// Entire server-resolved authorization domain.
    Domain([u8; 32]),
}

/// Post-commit notice that exact authorization dependencies became stale.
///
/// A notice is emitted only after the transaction that advanced the durable
/// invalidation floor commits. It contains neither credentials nor raw
/// principal identifiers.
#[derive(Clone, PartialEq, Eq)]
pub struct LifecycleInvalidationNotice {
    authorization_domain: CommunityId,
    operation_id: Uuid,
    generation: u64,
    selectors: Vec<LifecycleInvalidationSelector>,
}

impl LifecycleInvalidationNotice {
    /// Construct a non-empty post-commit invalidation notice.
    pub fn new(
        authorization_domain: CommunityId,
        operation_id: Uuid,
        generation: u64,
        selectors: Vec<LifecycleInvalidationSelector>,
    ) -> Option<Self> {
        if authorization_domain.as_uuid().is_nil()
            || operation_id.is_nil()
            || generation == 0
            || selectors.is_empty()
        {
            return None;
        }
        Some(Self {
            authorization_domain,
            operation_id,
            generation,
            selectors,
        })
    }

    /// Server-resolved domain whose generation advanced.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Lifecycle operation that advanced the floor.
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    /// New durable invalidation generation.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Exact privacy-safe selectors invalidated by the transition.
    pub fn selectors(&self) -> &[LifecycleInvalidationSelector] {
        &self.selectors
    }
}

impl fmt::Debug for LifecycleInvalidationNotice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LifecycleInvalidationNotice([REDACTED])")
    }
}

/// Post-commit notice that one previously issued admission is no longer valid.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AdmissionLossNotice {
    authorization_domain: CommunityId,
    operation_id: Uuid,
    lease_id: Uuid,
    binding_id: Uuid,
    binding_version: u64,
    invalidation_generation: u64,
}

impl AdmissionLossNotice {
    /// Construct a notice from committed durable dependency coordinates.
    pub const fn new(
        authorization_domain: CommunityId,
        operation_id: Uuid,
        lease_id: Uuid,
        binding_id: Uuid,
        binding_version: u64,
        invalidation_generation: u64,
    ) -> Option<Self> {
        if authorization_domain.as_uuid().is_nil()
            || operation_id.is_nil()
            || lease_id.is_nil()
            || binding_id.is_nil()
            || binding_version == 0
            || invalidation_generation == 0
        {
            None
        } else {
            Some(Self {
                authorization_domain,
                operation_id,
                lease_id,
                binding_id,
                binding_version,
                invalidation_generation,
            })
        }
    }

    /// Server-resolved domain.
    pub const fn authorization_domain(self) -> CommunityId {
        self.authorization_domain
    }

    /// Admission-loss operation.
    pub const fn operation_id(self) -> Uuid {
        self.operation_id
    }

    /// Invalidated in-memory lease.
    pub const fn lease_id(self) -> Uuid {
        self.lease_id
    }

    /// Exact binding generation that lost admission.
    pub const fn binding(self) -> (Uuid, u64) {
        (self.binding_id, self.binding_version)
    }

    /// Durable generation after the loss.
    pub const fn invalidation_generation(self) -> u64 {
        self.invalidation_generation
    }
}

impl fmt::Debug for AdmissionLossNotice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AdmissionLossNotice([REDACTED])")
    }
}

/// Fail-closed durable invalidation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LifecycleInvalidationError {
    /// A selector, generation floor, or operation coordinate was malformed.
    #[error("invalid lifecycle invalidation coordinate")]
    InvalidInput,
    /// The operator has not activated invalidation for this domain.
    #[error("lifecycle invalidation domain is not active")]
    DomainInactive,
    /// Durable invalidation state was unavailable.
    #[error("lifecycle invalidation storage is unavailable")]
    StorageUnavailable,
}

/// Advance the domain generation and exact selector floors in one lifecycle transaction.
///
/// The caller inserts the canonical operation receipt in the same transaction;
/// v30's deferred foreign keys then prove that no invalidation can survive a
/// rolled-back lifecycle effect.
pub async fn advance_lifecycle_invalidation_tx(
    transaction: &mut Transaction<'_, Postgres>,
    authorization_domain: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    selectors: Vec<LifecycleInvalidationSelector>,
) -> Result<LifecycleInvalidationNotice, LifecycleInvalidationError> {
    if authorization_domain.as_uuid().is_nil()
        || operation_id.is_nil()
        || request_fingerprint == [0; 32]
        || selectors.is_empty()
        || selectors.iter().any(invalid_selector)
    {
        return Err(LifecycleInvalidationError::InvalidInput);
    }
    let mut canonical = selectors;
    canonical.sort_by_key(selector_sort_key);
    if canonical
        .windows(2)
        .any(|pair| selector_sort_key(&pair[0]) == selector_sort_key(&pair[1]))
    {
        return Err(LifecycleInvalidationError::InvalidInput);
    }

    let current_generation: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT current_generation
        FROM authorization_invalidation_domains
        WHERE community_id = $1
        FOR UPDATE
        "#,
    )
    .bind(authorization_domain.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| LifecycleInvalidationError::StorageUnavailable)?;
    let current_generation =
        current_generation.ok_or(LifecycleInvalidationError::DomainInactive)?;
    let next_generation = current_generation
        .checked_add(1)
        .ok_or(LifecycleInvalidationError::StorageUnavailable)?;
    let advanced: Option<i64> = sqlx::query_scalar(
        r#"
        UPDATE authorization_invalidation_domains
        SET current_generation = $2,
            updated_at = GREATEST(
                transaction_timestamp(),
                updated_at + INTERVAL '1 microsecond'
            )
        WHERE community_id = $1 AND current_generation = $3
        RETURNING current_generation
        "#,
    )
    .bind(authorization_domain.as_uuid())
    .bind(next_generation)
    .bind(current_generation)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| LifecycleInvalidationError::StorageUnavailable)?;
    let generation = u64::try_from(advanced.ok_or(LifecycleInvalidationError::StorageUnavailable)?)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(LifecycleInvalidationError::StorageUnavailable)?;

    for selector in &canonical {
        let (kind, fingerprint, binding_floor) = selector_storage_parts(selector);
        let write = sqlx::query(
            r#"
            INSERT INTO authorization_invalidation_floors (
                community_id, selector_kind, selector_fingerprint,
                floor_generation, binding_version_floor,
                relationship_revision_floor, operation_id,
                request_fingerprint
            ) VALUES ($1, $2, $3, $4, $5, NULL, $6, $7)
            ON CONFLICT (community_id, selector_kind, selector_fingerprint)
            DO UPDATE SET
                floor_generation = GREATEST(
                    authorization_invalidation_floors.floor_generation,
                    EXCLUDED.floor_generation
                ),
                binding_version_floor = CASE
                    WHEN EXCLUDED.selector_kind = 3 THEN GREATEST(
                        authorization_invalidation_floors.binding_version_floor,
                        EXCLUDED.binding_version_floor
                    )
                    ELSE NULL
                END,
                operation_id = EXCLUDED.operation_id,
                request_fingerprint = EXCLUDED.request_fingerprint,
                updated_at = GREATEST(
                    transaction_timestamp(),
                    authorization_invalidation_floors.updated_at + INTERVAL '1 microsecond'
                )
            "#,
        )
        .bind(authorization_domain.as_uuid())
        .bind(kind)
        .bind(fingerprint.as_slice())
        .bind(i64::try_from(generation).map_err(|_| LifecycleInvalidationError::InvalidInput)?)
        .bind(
            binding_floor
                .map(i64::try_from)
                .transpose()
                .map_err(|_| LifecycleInvalidationError::InvalidInput)?,
        )
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .execute(&mut **transaction)
        .await
        .map_err(|_| LifecycleInvalidationError::StorageUnavailable)?;
        if write.rows_affected() != 1 {
            return Err(LifecycleInvalidationError::StorageUnavailable);
        }
    }

    LifecycleInvalidationNotice::new(authorization_domain, operation_id, generation, canonical)
        .ok_or(LifecycleInvalidationError::InvalidInput)
}

fn invalid_selector(selector: &LifecycleInvalidationSelector) -> bool {
    match selector {
        LifecycleInvalidationSelector::Principal(fingerprint)
        | LifecycleInvalidationSelector::EventAuthor(fingerprint)
        | LifecycleInvalidationSelector::Domain(fingerprint) => *fingerprint == [0; 32],
        LifecycleInvalidationSelector::Binding {
            fingerprint,
            version_floor,
        } => *fingerprint == [0; 32] || *version_floor == 0,
    }
}

fn selector_sort_key(selector: &LifecycleInvalidationSelector) -> (i16, [u8; 32]) {
    match selector {
        LifecycleInvalidationSelector::Principal(fingerprint) => (1, *fingerprint),
        LifecycleInvalidationSelector::EventAuthor(fingerprint) => (2, *fingerprint),
        LifecycleInvalidationSelector::Binding { fingerprint, .. } => (3, *fingerprint),
        LifecycleInvalidationSelector::Domain(fingerprint) => (5, *fingerprint),
    }
}

fn selector_storage_parts(
    selector: &LifecycleInvalidationSelector,
) -> (i16, [u8; 32], Option<u64>) {
    match selector {
        LifecycleInvalidationSelector::Principal(fingerprint) => (1, *fingerprint, None),
        LifecycleInvalidationSelector::EventAuthor(fingerprint) => (2, *fingerprint, None),
        LifecycleInvalidationSelector::Binding {
            fingerprint,
            version_floor,
        } => (3, *fingerprint, Some(*version_floor)),
        LifecycleInvalidationSelector::Domain(fingerprint) => (5, *fingerprint, None),
    }
}

#[cfg(test)]
mod invalidation_tests {
    use super::*;

    #[test]
    fn selector_order_is_closed_and_rejects_sentinels() {
        let principal = LifecycleInvalidationSelector::Principal([1; 32]);
        let binding = LifecycleInvalidationSelector::Binding {
            fingerprint: [2; 32],
            version_floor: 7,
        };
        assert_eq!(selector_sort_key(&principal), (1, [1; 32]));
        assert_eq!(selector_storage_parts(&binding), (3, [2; 32], Some(7)));
        assert!(invalid_selector(&LifecycleInvalidationSelector::Domain(
            [0; 32]
        )));
    }
}
