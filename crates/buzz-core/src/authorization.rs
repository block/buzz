//! Provider-free authorization values shared by the relay, database, and UI.
//!
//! These types carry only local authorization state. They deliberately cannot
//! contain federated credentials, issuer/subject identifiers, profile data, or
//! the client-binding bootstrap epoch.

use crate::CommunityId;
use chrono::{DateTime, Duration, Utc};
use nostr::PublicKey;
use std::fmt;
use thiserror::Error;
use uuid::Uuid;

const MAX_CURRENT_BINDING_FRESHNESS: Duration = Duration::seconds(300);

/// Opaque, database-derived token fencing an observable authorization result.
///
/// The token is intentionally not restricted to a lease. Status and other
/// read-only authorization observations use the same fence so a consumer can
/// reject a result after any dependency changes. Public construction validates
/// representation only: it is not self-authenticating authority. Composition
/// trusts its configured local resolver and must atomically recheck the full
/// evidence tuple against PostgreSQL immediately before allocation or delivery.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuthorizationLeaseFence([u8; 32]);

impl AuthorizationLeaseFence {
    /// Construct a fence from the exact bytes allocated by the authoritative
    /// PostgreSQL transaction. The caller is responsible for obtaining these
    /// bytes from the configured authoritative resolver.
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, AuthorizationLeaseFenceError> {
        if bytes == [0; 32] {
            return Err(AuthorizationLeaseFenceError::ZeroFence);
        }
        Ok(Self(bytes))
    }

    /// Return the opaque fence bytes for an exact equality comparison or a
    /// database bind.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for AuthorizationLeaseFence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationLeaseFence([REDACTED])")
    }
}

/// Privacy-safe evidence that a Nostr author has a current local binding.
///
/// This is the sole status-evidence value shared with kind 24244 consumers.
/// It contains no federated principal, assertion, capability, display name,
/// connection identifier, or client-binding bootstrap epoch. Constructing a
/// shape-valid value does not authorize a request or make it presentable; the
/// configured resolver must atomically recheck the complete tuple first.
#[derive(Clone, PartialEq, Eq)]
pub struct CanonicalCurrentBindingEvidence {
    authorization_domain: CommunityId,
    event_author_pubkey: PublicKey,
    binding_id: Uuid,
    binding_version: u64,
    policy_revision: u64,
    invalidation_generation: u64,
    authority_epoch: u64,
    fence: AuthorizationLeaseFence,
    observed_at: DateTime<Utc>,
    fresh_until: DateTime<Utc>,
}

impl CanonicalCurrentBindingEvidence {
    /// Construct evidence from one authoritative, non-mutating database read.
    ///
    /// `fresh_until` is exclusive and may be at most 300 seconds after
    /// `observed_at`. Binding and policy revisions are strictly positive. This
    /// validates representation, while trust comes from the configured local
    /// resolver and its mandatory database recheck.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        authorization_domain: CommunityId,
        event_author_pubkey: PublicKey,
        binding_id: Uuid,
        binding_version: u64,
        policy_revision: u64,
        invalidation_generation: u64,
        authority_epoch: u64,
        fence: AuthorizationLeaseFence,
        observed_at: DateTime<Utc>,
        fresh_until: DateTime<Utc>,
    ) -> Result<Self, CurrentBindingEvidenceError> {
        if authorization_domain.as_uuid().is_nil() {
            return Err(CurrentBindingEvidenceError::InvalidAuthorizationDomain);
        }
        if binding_id.is_nil() {
            return Err(CurrentBindingEvidenceError::InvalidBindingId);
        }
        if binding_version == 0 {
            return Err(CurrentBindingEvidenceError::InvalidBindingVersion);
        }
        if policy_revision == 0 {
            return Err(CurrentBindingEvidenceError::InvalidPolicyRevision);
        }
        if authority_epoch == 0 {
            return Err(CurrentBindingEvidenceError::InvalidAuthorityEpoch);
        }
        if fresh_until <= observed_at {
            return Err(CurrentBindingEvidenceError::InvalidFreshnessWindow);
        }
        let maximum = observed_at
            .checked_add_signed(MAX_CURRENT_BINDING_FRESHNESS)
            .ok_or(CurrentBindingEvidenceError::InvalidFreshnessWindow)?;
        if fresh_until > maximum {
            return Err(CurrentBindingEvidenceError::InvalidFreshnessWindow);
        }

        Ok(Self {
            authorization_domain,
            event_author_pubkey,
            binding_id,
            binding_version,
            policy_revision,
            invalidation_generation,
            authority_epoch,
            fence,
            observed_at,
            fresh_until,
        })
    }

    /// The server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// The exact Nostr author for which current status was observed.
    pub const fn event_author_pubkey(&self) -> PublicKey {
        self.event_author_pubkey
    }

    /// Stable local binding identifier.
    pub const fn binding_id(&self) -> Uuid {
        self.binding_id
    }

    /// Positive local binding version.
    pub const fn binding_version(&self) -> u64 {
        self.binding_version
    }

    /// Positive monotonic local policy revision.
    pub const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }

    /// Current local invalidation generation.
    pub const fn invalidation_generation(&self) -> u64 {
        self.invalidation_generation
    }

    /// Current protected-object authority epoch.
    pub const fn authority_epoch(&self) -> u64 {
        self.authority_epoch
    }

    /// Opaque observable-state fence returned by PostgreSQL.
    pub const fn fence(&self) -> AuthorizationLeaseFence {
        self.fence
    }

    /// Authoritative PostgreSQL observation time.
    pub const fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }

    /// Exclusive freshness boundary.
    pub const fn fresh_until(&self) -> DateTime<Utc> {
        self.fresh_until
    }

    /// Whether this evidence is still fresh at an authoritative time.
    pub fn is_fresh_at(&self, authoritative_now: DateTime<Utc>) -> bool {
        authoritative_now >= self.observed_at && authoritative_now < self.fresh_until
    }

    /// Gate a final resolver recheck before status presentation.
    ///
    /// The configured resolver must return the exact tuple from its atomic
    /// PostgreSQL recheck. Any changed coordinate, stale timestamp, or caller
    /// error withholds presentation; it never changes authorization state.
    pub fn accepts_exact_recheck(
        &self,
        rechecked: &Self,
        authoritative_now: DateTime<Utc>,
    ) -> bool {
        self == rechecked && rechecked.is_fresh_at(authoritative_now)
    }
}

impl fmt::Debug for CanonicalCurrentBindingEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalCurrentBindingEvidence([REDACTED])")
    }
}

/// Construction errors for [`AuthorizationLeaseFence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AuthorizationLeaseFenceError {
    /// A zero fence is reserved as the unallocated sentinel.
    #[error("authorization lease fence is unallocated")]
    ZeroFence,
}

/// Construction errors for [`CanonicalCurrentBindingEvidence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CurrentBindingEvidenceError {
    /// The server-resolved authorization domain was nil.
    #[error("invalid authorization domain")]
    InvalidAuthorizationDomain,
    /// The stable binding identifier was nil.
    #[error("invalid binding id")]
    InvalidBindingId,
    /// Binding versions are strictly positive.
    #[error("invalid binding version")]
    InvalidBindingVersion,
    /// Policy revisions are strictly positive.
    #[error("invalid policy revision")]
    InvalidPolicyRevision,
    /// Protected-object authority epochs are strictly positive.
    #[error("invalid authority epoch")]
    InvalidAuthorityEpoch,
    /// The exclusive freshness window was empty or exceeded 300 seconds.
    #[error("invalid freshness window")]
    InvalidFreshnessWindow,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use nostr::Keys;

    fn evidence(
        observed_at: DateTime<Utc>,
        fresh_until: DateTime<Utc>,
    ) -> Result<CanonicalCurrentBindingEvidence, CurrentBindingEvidenceError> {
        CanonicalCurrentBindingEvidence::new(
            CommunityId::from_uuid(Uuid::from_u128(1)),
            Keys::generate().public_key(),
            Uuid::from_u128(2),
            1,
            1,
            0,
            1,
            AuthorizationLeaseFence::from_bytes([7; 32]).unwrap(),
            observed_at,
            fresh_until,
        )
    }

    #[test]
    fn freshness_is_exclusive_and_bounded_to_five_minutes() {
        let observed = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        let current = evidence(observed, observed + Duration::seconds(300)).unwrap();

        assert!(!current.is_fresh_at(observed - Duration::nanoseconds(1)));
        assert!(current.is_fresh_at(observed));
        assert!(current.is_fresh_at(observed + Duration::seconds(299)));
        assert!(!current.is_fresh_at(observed + Duration::seconds(300)));
        assert_eq!(
            evidence(observed, observed + Duration::seconds(301)),
            Err(CurrentBindingEvidenceError::InvalidFreshnessWindow)
        );
        assert_eq!(
            evidence(observed, observed),
            Err(CurrentBindingEvidenceError::InvalidFreshnessWindow)
        );
    }

    #[test]
    fn revisions_are_typed_and_positive() {
        let observed = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        let common = (
            CommunityId::from_uuid(Uuid::from_u128(1)),
            Keys::generate().public_key(),
            Uuid::from_u128(2),
            AuthorizationLeaseFence::from_bytes([1; 32]).unwrap(),
        );

        let no_binding_version = CanonicalCurrentBindingEvidence::new(
            common.0,
            common.1,
            common.2,
            0,
            1,
            0,
            1,
            common.3,
            observed,
            observed + Duration::seconds(1),
        );
        assert_eq!(
            no_binding_version,
            Err(CurrentBindingEvidenceError::InvalidBindingVersion)
        );

        let no_policy_revision = CanonicalCurrentBindingEvidence::new(
            common.0,
            common.1,
            common.2,
            1,
            0,
            0,
            1,
            common.3,
            observed,
            observed + Duration::seconds(1),
        );
        assert_eq!(
            no_policy_revision,
            Err(CurrentBindingEvidenceError::InvalidPolicyRevision)
        );

        let no_authority_epoch = CanonicalCurrentBindingEvidence::new(
            common.0,
            common.1,
            common.2,
            1,
            1,
            0,
            0,
            common.3,
            observed,
            observed + Duration::seconds(1),
        );
        assert_eq!(
            no_authority_epoch,
            Err(CurrentBindingEvidenceError::InvalidAuthorityEpoch)
        );
    }

    #[test]
    fn debug_output_redacts_author_and_fence_bytes() {
        let observed = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        let current = evidence(observed, observed + Duration::seconds(1)).unwrap();
        let rendered = format!("{current:?}");

        assert_eq!(rendered, "CanonicalCurrentBindingEvidence([REDACTED])");
        assert!(!rendered.contains(&current.event_author_pubkey().to_hex()));
        assert!(!rendered.contains(&hex::encode(current.fence().as_bytes())));
    }

    #[test]
    fn nil_identifiers_and_zero_fences_are_rejected() {
        let observed = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        let pubkey = Keys::generate().public_key();
        let fence = AuthorizationLeaseFence::from_bytes([1; 32]).unwrap();

        let nil_domain = CanonicalCurrentBindingEvidence::new(
            CommunityId::from_uuid(Uuid::nil()),
            pubkey,
            Uuid::from_u128(2),
            1,
            1,
            0,
            1,
            fence,
            observed,
            observed + Duration::seconds(1),
        );
        assert_eq!(
            nil_domain,
            Err(CurrentBindingEvidenceError::InvalidAuthorizationDomain)
        );

        let nil_binding = CanonicalCurrentBindingEvidence::new(
            CommunityId::from_uuid(Uuid::from_u128(1)),
            pubkey,
            Uuid::nil(),
            1,
            1,
            0,
            1,
            fence,
            observed,
            observed + Duration::seconds(1),
        );
        assert_eq!(
            nil_binding,
            Err(CurrentBindingEvidenceError::InvalidBindingId)
        );
        assert_eq!(
            AuthorizationLeaseFence::from_bytes([0; 32]),
            Err(AuthorizationLeaseFenceError::ZeroFence)
        );
    }

    #[test]
    fn final_recheck_requires_the_exact_fresh_tuple() {
        let observed = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        let current = evidence(observed, observed + Duration::seconds(10)).unwrap();
        assert!(current.accepts_exact_recheck(&current, observed + Duration::seconds(1)));
        assert!(!current.accepts_exact_recheck(&current, observed + Duration::seconds(10)));

        let changed_fence = CanonicalCurrentBindingEvidence::new(
            current.authorization_domain(),
            current.event_author_pubkey(),
            current.binding_id(),
            current.binding_version(),
            current.policy_revision(),
            current.invalidation_generation(),
            current.authority_epoch(),
            AuthorizationLeaseFence::from_bytes([8; 32]).unwrap(),
            current.observed_at(),
            current.fresh_until(),
        )
        .unwrap();
        assert!(!current.accepts_exact_recheck(&changed_fence, observed + Duration::seconds(1)));
    }
}
