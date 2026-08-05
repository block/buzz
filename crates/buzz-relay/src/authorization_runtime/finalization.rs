//! Compile-stable finalization types for the protected-transport slice.
//!
//! The invalidation slice installs the runtime finalizer. This module exposes
//! only the sealed types needed to compile transport coupling before that
//! implementation is present.

use std::fmt;

use buzz_auth::{AuthorizationProfileId, FederatedPrincipal, PolicyVersion};
use buzz_core::CommunityId;
use nostr::PublicKey;
use uuid::Uuid;

/// Server-owned activation mode for one exact authorization domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationMode {
    /// Do not evaluate federated identity or provider policy.
    Off,
    /// Evaluate read-only provider policy without authority changes.
    Shadow,
    /// Produce display-only output after full verification.
    VerifyOnly,
    /// Issue bounded protected access after finalization.
    Enforce,
}

/// Direct provider decision sealed for atomic first enrollment.
#[must_use]
pub struct EnrollmentDisposition {
    authorization_domain: CommunityId,
    actor_pubkey: PublicKey,
    principal: FederatedPrincipal,
    profile_id: AuthorizationProfileId,
    policy_version: PolicyVersion,
    correlation_id: Uuid,
    expires_at: u64,
}

impl EnrollmentDisposition {
    /// Exact server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Direct actor whose key was attested.
    pub const fn actor_pubkey(&self) -> PublicKey {
        self.actor_pubkey
    }

    /// Literal issuer-qualified principal staged for enrollment.
    pub const fn principal(&self) -> &FederatedPrincipal {
        &self.principal
    }

    /// Installed provider profile that made the decision.
    pub const fn profile_id(&self) -> &AuthorizationProfileId {
        &self.profile_id
    }

    /// Provider policy revision that made the decision.
    pub const fn policy_version(&self) -> &PolicyVersion {
        &self.policy_version
    }

    /// Exact decision correlation identifier.
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Earliest authoritative expiry bound for enrollment.
    pub const fn expires_at(&self) -> u64 {
        self.expires_at
    }
}

impl fmt::Debug for EnrollmentDisposition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnrollmentDisposition")
            .field("evidence", &"[redacted]")
            .finish()
    }
}
