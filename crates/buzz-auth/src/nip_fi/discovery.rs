//! NIP-11 federated-identity discovery output.
//!
//! [`FederatedIdentityDiscovery`] serializes to the `federated_identity`
//! object required by the NIP-FI.md "Discovery" section of the NIP-11 relay
//! information document.
//!
//! ## Privacy invariants
//!
//! The discovery object MUST NOT contain: enrollment mode, TOFU posture,
//! issuer URLs, audiences, claim names, tenant IDs, or deployment-local
//! identifiers. For a fixed set of claimed profiles the complete output is
//! byte-identical across every enrollment policy and lifecycle state.
//! [FI-TRACE-DISCOVERY-PRIVATE]
//!
//! ## Offline-jwt residual bound
//!
//! `maximum_residual_upstream_revocation_seconds` is `null` for `offline-jwt`
//! deployments. An offline-jwt deployment MUST NOT advertise a finite value
//! here (NIP-FI.md:259-266).

use serde::{Deserialize, Serialize};

/// The `assertion_freshness` sub-object in the `federated_identity` discovery
/// document. Describes the claimed freshness posture without exposing any
/// issuer or deployment-private state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssertionFreshnessDiscovery {
    /// The wire string identifying the freshness class.
    pub class: FreshnessClassDiscovery,
    /// `null` for `offline-jwt`; advertising a finite bound here requires a
    /// live status witness that is not yet implemented.
    pub maximum_residual_upstream_revocation_seconds: Option<u64>,
}

/// The freshness class as a stable NIP-FI wire string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FreshnessClassDiscovery {
    /// No revocation bound is claimed; JWKS snapshot validation only.
    OfflineJwt,
}

/// The `federated_identity` NIP-11 discovery object. Fields never expose
/// enrollment mode, issuer, audience, or private state.
/// [FI-TRACE-DISCOVERY-PRIVATE]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FederatedIdentityDiscovery {
    /// Fixed value `"client-attached"` for the core NIP-FI transport mode.
    pub core: String,
    /// The freshness contract claimed by this deployment.
    pub assertion_freshness: AssertionFreshnessDiscovery,
}

impl FederatedIdentityDiscovery {
    /// The only supported posture: claims no residual revocation bound, which
    /// is the honest description of JWKS-only assertion verification.
    pub fn offline_jwt() -> Self {
        Self {
            core: "client-attached".to_owned(),
            assertion_freshness: AssertionFreshnessDiscovery {
                class: FreshnessClassDiscovery::OfflineJwt,
                maximum_residual_upstream_revocation_seconds: None,
            },
        }
    }
}
