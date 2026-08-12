//! Recoverable, fail-closed live invalidation registry.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use buzz_auth::BoundedAuthorizationLease;
use buzz_core::CommunityId;
use buzz_db::lifecycle_invalidation::{AdmissionLossNotice, LifecycleInvalidationNotice};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::RuntimeAuthorizationError;

/// Typed durable change observed from lifecycle or protected-object authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidationNotice {
    /// Entire domain generation advanced.
    Domain {
        /// Server-resolved authorization domain.
        domain: CommunityId,
        /// New durable invalidation generation.
        generation: u64,
    },
    /// One exact lease lost admission.
    Lease {
        /// Server-resolved authorization domain.
        domain: CommunityId,
        /// Exact lease identifier.
        lease_id: Uuid,
        /// New durable invalidation generation.
        generation: u64,
    },
    /// A binding generation became invalid.
    Binding {
        /// Server-resolved authorization domain.
        domain: CommunityId,
        /// Exact binding identifier.
        binding_id: Uuid,
        /// First invalid binding version.
        version_floor: u64,
        /// New durable invalidation generation.
        generation: u64,
    },
    /// A delegated relationship generation became invalid.
    Relationship {
        /// Server-resolved authorization domain.
        domain: CommunityId,
        /// Exact relationship identifier.
        relationship_id: Uuid,
        /// First invalid relationship revision.
        revision_floor: u64,
        /// New durable invalidation generation.
        generation: u64,
    },
    /// Protected-object authority advanced.
    AuthorityEpoch {
        /// Server-resolved authorization domain.
        domain: CommunityId,
        /// First invalid authority epoch.
        epoch_floor: u64,
        /// New durable invalidation generation.
        generation: u64,
    },
    /// Local authorization policy or configuration advanced.
    Policy {
        /// Server-resolved authorization domain.
        domain: CommunityId,
        /// First invalid policy revision.
        revision_floor: u64,
        /// New durable invalidation generation.
        generation: u64,
    },
}

impl InvalidationNotice {
    const fn domain(self) -> CommunityId {
        match self {
            Self::Domain { domain, .. }
            | Self::Lease { domain, .. }
            | Self::Binding { domain, .. }
            | Self::Relationship { domain, .. }
            | Self::AuthorityEpoch { domain, .. }
            | Self::Policy { domain, .. } => domain,
        }
    }

    const fn generation(self) -> u64 {
        match self {
            Self::Domain { generation, .. }
            | Self::Lease { generation, .. }
            | Self::Binding { generation, .. }
            | Self::Relationship { generation, .. }
            | Self::AuthorityEpoch { generation, .. }
            | Self::Policy { generation, .. } => generation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveCoordinate {
    lease_id: Uuid,
    domain: CommunityId,
    binding_id: Uuid,
    binding_version: u64,
    relationship: Option<(Uuid, u64)>,
    policy_revision: u64,
    invalidation_generation: u64,
    authority_epoch: u64,
    expires_at: DateTime<Utc>,
}

impl LiveCoordinate {
    fn from_lease(lease: &BoundedAuthorizationLease) -> Self {
        let (_, domain) = lease.dependency_snapshot().identity();
        let (binding_id, binding_version) = lease.binding();
        let (policy_revision, invalidation_generation, authority_epoch) =
            lease.dependency_versions();
        Self {
            lease_id: lease.lease_id(),
            domain,
            binding_id,
            binding_version,
            relationship: lease.delegated_relationship(),
            policy_revision,
            invalidation_generation,
            authority_epoch,
            expires_at: lease.expires_at(),
        }
    }
}

struct RegistryEntry {
    coordinate: LiveCoordinate,
    cancel: CancellationToken,
}

struct RegistryInner {
    entries: DashMap<Uuid, RegistryEntry>,
    generations: DashMap<CommunityId, u64>,
    ready: AtomicBool,
    mutation_lock: std::sync::Mutex<()>,
}

/// In-memory lease registry driven by durable monotonic notices.
#[derive(Clone)]
pub struct InvalidationRegistry {
    inner: Arc<RegistryInner>,
}

impl InvalidationRegistry {
    /// Construct unready. Production must reconcile authoritative generations
    /// before any registration can succeed.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                entries: DashMap::new(),
                generations: DashMap::new(),
                ready: AtomicBool::new(false),
                mutation_lock: std::sync::Mutex::new(()),
            }),
        }
    }

    /// Whether the feed has completed authoritative reconciliation and has not
    /// subsequently observed a gap or loss.
    pub fn is_ready(&self) -> bool {
        self.inner.ready.load(Ordering::Acquire)
    }

    /// Register one finalized lease. Generation is checked before and after
    /// insertion to close the notice-before-registration race.
    pub fn register(
        &self,
        lease: &BoundedAuthorizationLease,
    ) -> Result<InvalidationRegistration, RuntimeAuthorizationError> {
        self.register_coordinate(LiveCoordinate::from_lease(lease))
    }

    fn register_coordinate(
        &self,
        coordinate: LiveCoordinate,
    ) -> Result<InvalidationRegistration, RuntimeAuthorizationError> {
        if !self.is_ready()
            || self.current_generation(coordinate.domain)
                != Some(coordinate.invalidation_generation)
        {
            return Err(RuntimeAuthorizationError::InvalidationUnavailable);
        }
        let registration_id = Uuid::new_v4();
        let cancel = CancellationToken::new();
        self.inner.entries.insert(
            registration_id,
            RegistryEntry {
                coordinate: coordinate.clone(),
                cancel: cancel.clone(),
            },
        );
        if !self.is_ready()
            || self.current_generation(coordinate.domain)
                != Some(coordinate.invalidation_generation)
        {
            cancel.cancel();
            self.inner.entries.remove(&registration_id);
            return Err(RuntimeAuthorizationError::InvalidationUnavailable);
        }
        Ok(InvalidationRegistration {
            registration_id,
            cancel,
            inner: Arc::clone(&self.inner),
        })
    }

    /// Apply one typed post-commit notice. A generation gap withdraws readiness
    /// and cancels every live authorization in that domain until reconciliation.
    pub fn apply(&self, notice: InvalidationNotice) -> Result<usize, RuntimeAuthorizationError> {
        let _mutation = self
            .inner
            .mutation_lock
            .lock()
            .map_err(|_| RuntimeAuthorizationError::InvalidationUnavailable)?;
        let domain = notice.domain();
        let generation = notice.generation();
        if domain.as_uuid().is_nil() || generation == 0 {
            return Err(RuntimeAuthorizationError::InvalidationUnavailable);
        }
        let current = self.current_generation(domain).unwrap_or(0);
        if self.is_ready() && current != 0 && generation > current.saturating_add(1) {
            self.inner.ready.store(false, Ordering::Release);
            self.cancel_domain(domain);
            return Err(RuntimeAuthorizationError::InvalidationUnavailable);
        }
        if generation > current {
            self.inner.generations.insert(domain, generation);
        }

        let mut cancelled = 0;
        for entry in &self.inner.entries {
            if entry.coordinate.domain == domain && notice_matches(&notice, &entry.coordinate) {
                entry.cancel.cancel();
                cancelled += 1;
            }
        }
        Ok(cancelled)
    }

    /// Consume the database authority's credential-free lifecycle notice. It exports
    /// selector fingerprints rather than the raw coordinates held by this
    /// registry, so the relay conservatively invalidates the complete domain.
    pub fn apply_lifecycle(
        &self,
        notice: &LifecycleInvalidationNotice,
    ) -> Result<usize, RuntimeAuthorizationError> {
        self.apply(InvalidationNotice::Domain {
            domain: notice.authorization_domain(),
            generation: notice.generation(),
        })
    }

    /// Consume the database authority's exact admission-loss notice without widening its lease
    /// selector.
    pub fn apply_admission_loss(
        &self,
        notice: AdmissionLossNotice,
    ) -> Result<usize, RuntimeAuthorizationError> {
        self.apply(InvalidationNotice::Lease {
            domain: notice.authorization_domain(),
            lease_id: notice.lease_id(),
            generation: notice.invalidation_generation(),
        })
    }

    /// Mark feed loss, withdraw readiness, and cancel every protected operation.
    pub fn feed_lost(&self) -> usize {
        let _mutation = match self.inner.mutation_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.inner.ready.store(false, Ordering::Release);
        self.cancel_all()
    }

    /// Recover only from one complete authoritative domain-generation snapshot.
    pub fn reconcile(
        &self,
        state: ReconciledInvalidationState,
    ) -> Result<usize, RuntimeAuthorizationError> {
        let _mutation = self
            .inner
            .mutation_lock
            .lock()
            .map_err(|_| RuntimeAuthorizationError::InvalidationUnavailable)?;
        self.inner.ready.store(false, Ordering::Release);
        if !state.complete {
            self.cancel_all();
            return Err(RuntimeAuthorizationError::InvalidationUnavailable);
        }
        let mut seen = HashSet::new();
        let mut generations = BTreeMap::new();
        for (domain, generation) in state.domain_generations {
            if domain.as_uuid().is_nil() || generation == 0 || !seen.insert(domain) {
                self.cancel_all();
                return Err(RuntimeAuthorizationError::InvalidationUnavailable);
            }
            generations.insert(domain, generation);
        }
        let local_now = Utc::now();
        if state.observed_at > local_now + chrono::Duration::seconds(30)
            || state.observed_at < local_now - chrono::Duration::seconds(300)
        {
            self.cancel_all();
            return Err(RuntimeAuthorizationError::InvalidationUnavailable);
        }

        // A complete authoritative snapshot cannot forget a previously known
        // domain or move its durable floor backwards.
        for previous in &self.inner.generations {
            if generations
                .get(previous.key())
                .is_none_or(|generation| *generation < *previous.value())
            {
                self.cancel_all();
                return Err(RuntimeAuthorizationError::InvalidationUnavailable);
            }
        }

        self.inner.generations.clear();
        for (domain, generation) in &generations {
            self.inner.generations.insert(*domain, *generation);
        }
        let mut cancelled = 0;
        for entry in &self.inner.entries {
            let current = generations
                .get(&entry.coordinate.domain)
                .copied()
                .unwrap_or(0);
            if current != entry.coordinate.invalidation_generation
                || state.observed_at >= entry.coordinate.expires_at
            {
                entry.cancel.cancel();
                cancelled += 1;
            }
        }
        self.inner.ready.store(true, Ordering::Release);
        Ok(cancelled)
    }

    fn current_generation(&self, domain: CommunityId) -> Option<u64> {
        self.inner.generations.get(&domain).map(|value| *value)
    }

    fn cancel_domain(&self, domain: CommunityId) -> usize {
        let mut cancelled = 0;
        for entry in &self.inner.entries {
            if entry.coordinate.domain == domain {
                entry.cancel.cancel();
                cancelled += 1;
            }
        }
        cancelled
    }

    fn cancel_all(&self) -> usize {
        let mut cancelled = 0;
        for entry in &self.inner.entries {
            entry.cancel.cancel();
            cancelled += 1;
        }
        cancelled
    }
}

impl Default for InvalidationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for InvalidationRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("InvalidationRegistry([REDACTED])")
    }
}

/// Complete authoritative generation snapshot used after startup or feed loss.
#[derive(Debug, Clone)]
pub struct ReconciledInvalidationState {
    /// The authoritative adapter proved that every installed domain was read.
    pub complete: bool,
    /// One current generation for every installed authorization domain.
    pub domain_generations: Vec<(CommunityId, u64)>,
    /// Authoritative database observation time.
    pub observed_at: DateTime<Utc>,
}

/// Owned cancellation guard for one live authorization.
pub struct InvalidationRegistration {
    registration_id: Uuid,
    cancel: CancellationToken,
    inner: Arc<RegistryInner>,
}

impl InvalidationRegistration {
    /// Token cancelled by matching notice, feed loss, or stale reconciliation.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Whether this live authorization has been invalidated.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

impl Drop for InvalidationRegistration {
    fn drop(&mut self) {
        self.inner.entries.remove(&self.registration_id);
    }
}

impl std::fmt::Debug for InvalidationRegistration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("InvalidationRegistration([REDACTED])")
    }
}

fn notice_matches(notice: &InvalidationNotice, coordinate: &LiveCoordinate) -> bool {
    match *notice {
        InvalidationNotice::Domain { .. } => true,
        InvalidationNotice::Lease { lease_id, .. } => coordinate.lease_id == lease_id,
        InvalidationNotice::Binding {
            binding_id,
            version_floor,
            ..
        } => coordinate.binding_id == binding_id && coordinate.binding_version < version_floor,
        InvalidationNotice::Relationship {
            relationship_id,
            revision_floor,
            ..
        } => coordinate
            .relationship
            .is_some_and(|(id, revision)| id == relationship_id && revision < revision_floor),
        InvalidationNotice::AuthorityEpoch { epoch_floor, .. } => {
            coordinate.authority_epoch < epoch_floor
        }
        InvalidationNotice::Policy { revision_floor, .. } => {
            coordinate.policy_revision < revision_floor
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn domain(value: u128) -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(value))
    }

    fn coordinate(domain: CommunityId, lease: u128, generation: u64) -> LiveCoordinate {
        LiveCoordinate {
            lease_id: Uuid::from_u128(lease),
            domain,
            binding_id: Uuid::from_u128(10),
            binding_version: 3,
            relationship: Some((Uuid::from_u128(20), 4)),
            policy_revision: 5,
            invalidation_generation: generation,
            authority_epoch: 6,
            expires_at: Utc::now() + Duration::minutes(5),
        }
    }

    #[test]
    fn registration_requires_reconciliation_and_closes_notice_races() {
        let registry = InvalidationRegistry::new();
        let domain = domain(1);
        assert!(registry
            .register_coordinate(coordinate(domain, 1, 1))
            .is_err());
        registry
            .reconcile(ReconciledInvalidationState {
                complete: true,
                domain_generations: vec![(domain, 2)],
                observed_at: Utc::now(),
            })
            .unwrap();
        assert!(registry
            .register_coordinate(coordinate(domain, 1, 1))
            .is_err());
        assert!(registry
            .register_coordinate(coordinate(CommunityId::from_uuid(Uuid::from_u128(2)), 2, 2,))
            .is_err());
        assert!(registry
            .register_coordinate(coordinate(domain, 1, 2))
            .is_ok());
    }

    #[test]
    fn exact_notice_cancels_related_and_preserves_unrelated() {
        let registry = InvalidationRegistry::new();
        let first_domain = domain(1);
        let second_domain = domain(2);
        registry
            .reconcile(ReconciledInvalidationState {
                complete: true,
                domain_generations: vec![(first_domain, 1), (second_domain, 1)],
                observed_at: Utc::now(),
            })
            .unwrap();
        let first = registry
            .register_coordinate(coordinate(first_domain, 1, 1))
            .unwrap();
        let second = registry
            .register_coordinate(coordinate(second_domain, 2, 1))
            .unwrap();
        assert_eq!(
            registry
                .apply(InvalidationNotice::Binding {
                    domain: first_domain,
                    binding_id: Uuid::from_u128(10),
                    version_floor: 4,
                    generation: 2,
                })
                .unwrap(),
            1
        );
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());

        let admission_loss = AdmissionLossNotice::new(
            second_domain,
            Uuid::from_u128(30),
            Uuid::from_u128(2),
            Uuid::from_u128(10),
            3,
            2,
        )
        .unwrap();
        assert_eq!(registry.apply_admission_loss(admission_loss).unwrap(), 1);
        assert!(second.is_cancelled());
    }

    #[test]
    fn feed_loss_and_generation_gap_withdraw_readiness_until_reconciled() {
        let registry = InvalidationRegistry::new();
        let domain = domain(1);
        registry
            .reconcile(ReconciledInvalidationState {
                complete: true,
                domain_generations: vec![(domain, 1)],
                observed_at: Utc::now(),
            })
            .unwrap();
        let registration = registry
            .register_coordinate(coordinate(domain, 1, 1))
            .unwrap();
        assert_eq!(
            registry.apply(InvalidationNotice::Domain {
                domain,
                generation: 3,
            }),
            Err(RuntimeAuthorizationError::InvalidationUnavailable)
        );
        assert!(!registry.is_ready());
        assert!(registration.is_cancelled());

        registry
            .reconcile(ReconciledInvalidationState {
                complete: true,
                domain_generations: vec![(domain, 3)],
                observed_at: Utc::now(),
            })
            .unwrap();
        assert!(registry.is_ready());
        assert_eq!(registry.feed_lost(), 1);
        assert!(!registry.is_ready());
    }

    #[test]
    fn reconciliation_rejects_omitted_or_regressed_domains_and_cancels_live_work() {
        for replacement in [vec![], vec![(domain(1), 1)]] {
            let registry = InvalidationRegistry::new();
            registry
                .reconcile(ReconciledInvalidationState {
                    complete: true,
                    domain_generations: vec![(domain(1), 2)],
                    observed_at: Utc::now(),
                })
                .unwrap();
            let registration = registry
                .register_coordinate(coordinate(domain(1), 1, 2))
                .unwrap();
            assert_eq!(
                registry.reconcile(ReconciledInvalidationState {
                    complete: true,
                    domain_generations: replacement,
                    observed_at: Utc::now(),
                }),
                Err(RuntimeAuthorizationError::InvalidationUnavailable)
            );
            assert!(!registry.is_ready());
            assert!(registration.is_cancelled());
        }
    }
}
