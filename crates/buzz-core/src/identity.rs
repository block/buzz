//! Portable cryptographic identity and authorization port contracts.
//!
//! The signed event author, authenticated caller, replication peer, and local
//! authorization decision are deliberately separate values. Transport adapters
//! verify evidence; policy adapters decide access; the relay kernel continues
//! to preserve the exact signed event envelope.

use std::fmt;
use std::future::Future;

use nostr::{Event, Filter, PublicKey};
use serde::{Deserialize, Serialize};

use crate::replication::ReplicationSourceId;

/// A security subject recognized by relay policy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Principal {
    /// A person or agent identified by a Nostr public key.
    Nostr {
        /// Public key that proved control at the transport boundary.
        pubkey: PublicKey,
    },
    /// A relay node with a stable identifier such as a DID.
    RelayNode {
        /// Stable node identifier; the active verification key may rotate.
        id: String,
    },
    /// A deployment-controlled system actor.
    System {
        /// Stable deployment-local system identifier.
        id: String,
    },
}

impl Principal {
    /// Returns the Nostr public key when this is a Nostr principal.
    pub fn nostr_pubkey(&self) -> Option<&PublicKey> {
        match self {
            Self::Nostr { pubkey } => Some(pubkey),
            Self::RelayNode { .. } | Self::System { .. } => None,
        }
    }

    /// Returns the stable relay-node identifier when this is a node principal.
    pub fn relay_node_id(&self) -> Option<&str> {
        match self {
            Self::RelayNode { id } => Some(id),
            Self::Nostr { .. } | Self::System { .. } => None,
        }
    }
}

/// Cryptographic method used to establish a principal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationMethod {
    /// NIP-42 WebSocket challenge-response.
    Nip42,
    /// NIP-98 signed HTTP request.
    Nip98,
    /// A relay node signed a destination-issued challenge.
    RelayNodeChallenge,
    /// A mutually authenticated peer channel established the node.
    MutuallyAuthenticatedPeerChannel,
    /// An explicitly trusted in-process caller used by a local adapter.
    TrustedLocal,
}

/// Ephemeral result of successful authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticatedPrincipal {
    /// Security subject that proved control.
    pub principal: Principal,
    /// Proof method used by the adapter.
    pub method: AuthenticationMethod,
    /// Relay, URL, or peer audience to which the proof was bound.
    pub audience: String,
    /// Unix timestamp at which the adapter accepted the proof.
    pub authenticated_at: u64,
    /// Optional Unix timestamp after which this context cannot be reused.
    pub expires_at: Option<u64>,
    /// Event ID or adapter-owned proof identifier, never reusable secret data.
    pub proof_id: Option<String>,
}

/// Origin under which a signed event reaches the append authorizer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum AppendContext {
    /// A person or agent submits an event directly.
    Direct,
    /// An authenticated relay peer transports existing durable history.
    Replication {
        /// Destination-configured source stream.
        source: ReplicationSourceId,
    },
    /// A deployment component creates a newly signed system event.
    System,
}

/// Relay read surface subject to equivalent disclosure policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadOperation {
    /// Finite event query.
    Query,
    /// Matching event count.
    Count,
    /// Historical phase of a subscription.
    HistoricalSubscription,
    /// Live phase of a subscription.
    LiveDelivery,
}

/// Stable machine-readable reason for an identity or access denial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityDenialCode {
    /// The operation requires an authenticated caller.
    AuthenticationRequired,
    /// Evidence was malformed, incorrectly signed, or used a revoked key.
    InvalidEvidence,
    /// Evidence fell outside its freshness window.
    EvidenceExpired,
    /// Evidence was signed for another relay, URL, or peer.
    AudienceMismatch,
    /// Single-use evidence was already consumed.
    ReplayDetected,
    /// A direct caller did not match the event author.
    AuthorMismatch,
    /// A claimed delegation was invalid or did not cover the operation.
    DelegationInvalid,
    /// No destination-controlled peer binding exists.
    PeerUnbound,
    /// The authenticated relay node did not match the configured source.
    SourceMismatch,
    /// Local policy denied the requested operation or scope.
    ScopeDenied,
    /// Local policy denied disclosure of one candidate event.
    EventDisclosureDenied,
}

impl IdentityDenialCode {
    /// Returns the stable wire token used by conformance tests.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthenticationRequired => "authentication_required",
            Self::InvalidEvidence => "invalid_evidence",
            Self::EvidenceExpired => "evidence_expired",
            Self::AudienceMismatch => "audience_mismatch",
            Self::ReplayDetected => "replay_detected",
            Self::AuthorMismatch => "author_mismatch",
            Self::DelegationInvalid => "delegation_invalid",
            Self::PeerUnbound => "peer_unbound",
            Self::SourceMismatch => "source_mismatch",
            Self::ScopeDenied => "scope_denied",
            Self::EventDisclosureDenied => "event_disclosure_denied",
        }
    }
}

impl fmt::Display for IdentityDenialCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Normative authorization result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum AuthorizationDecision {
    /// Policy admits the operation.
    Allowed,
    /// Policy denies the operation without mutating relay state.
    Denied {
        /// Stable denial category.
        code: IdentityDenialCode,
    },
}

impl AuthorizationDecision {
    /// Creates a denial with a stable code.
    pub const fn denied(code: IdentityDenialCode) -> Self {
        Self::Denied { code }
    }

    /// Returns whether policy admitted the operation.
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }

    /// Returns the denial code, when denied.
    pub const fn denial_code(&self) -> Option<IdentityDenialCode> {
        match self {
            Self::Allowed => None,
            Self::Denied { code } => Some(*code),
        }
    }
}

/// Successful binding of a replication source to an authenticated relay node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicationPeerBinding {
    /// Source stream admitted by destination configuration.
    pub source: ReplicationSourceId,
    /// Stable node principal that authenticated.
    pub relay_node_principal: String,
    /// Active verification method used for this proof.
    pub verification_method: String,
}

/// Transport evidence verifier producing an ephemeral principal.
pub trait IdentityAuthenticator {
    /// Transport- or adapter-specific cryptographic evidence.
    type Evidence;
    /// Operational or verification error.
    type Error;

    /// Verifies evidence for `audience` and returns its authenticated principal.
    fn authenticate(
        &self,
        evidence: Self::Evidence,
        audience: &str,
    ) -> impl Future<Output = Result<AuthenticatedPrincipal, Self::Error>>;
}

/// Policy port for one signed-event append.
pub trait AppendAuthorizer {
    /// Decides whether `principal` may append `event` under `context`.
    fn authorize_append(
        &self,
        principal: &AuthenticatedPrincipal,
        context: &AppendContext,
        event: &Event,
        destination_scope: &str,
    ) -> AuthorizationDecision;
}

/// Two-stage policy port for relay read surfaces.
pub trait ReadAuthorizer {
    /// Authorizes the requested filter set before querying candidates.
    fn authorize_query(
        &self,
        principal: &AuthenticatedPrincipal,
        operation: ReadOperation,
        filters: &[Filter],
        destination_scope: &str,
    ) -> AuthorizationDecision;

    /// Authorizes one candidate before returning, counting, or publishing it.
    fn authorize_event_delivery(
        &self,
        principal: &AuthenticatedPrincipal,
        operation: ReadOperation,
        event: &Event,
        destination_scope: &str,
    ) -> AuthorizationDecision;
}

/// Cryptographically binds a relay peer to destination-controlled trust.
pub trait ReplicationPeerAuthenticator {
    /// Adapter-specific peer proof.
    type Evidence;
    /// Operational or verification error.
    type Error;

    /// Authenticates `evidence` and binds it to `source` for `audience`.
    fn authenticate_peer(
        &self,
        evidence: Self::Evidence,
        audience: &str,
        source: &ReplicationSourceId,
    ) -> impl Future<Output = Result<ReplicationPeerBinding, Self::Error>>;
}

#[cfg(test)]
mod tests {
    use nostr::Keys;

    use super::*;

    #[test]
    fn principal_json_preserves_nostr_identity() {
        let principal = Principal::Nostr {
            pubkey: Keys::generate().public_key(),
        };
        let json = serde_json::to_string(&principal).expect("principal serializes");
        let decoded: Principal = serde_json::from_str(&json).expect("principal deserializes");
        assert_eq!(decoded, principal);
    }

    #[test]
    fn denial_codes_are_stable() {
        assert_eq!(
            IdentityDenialCode::AuthenticationRequired.as_str(),
            "authentication_required"
        );
        assert_eq!(
            IdentityDenialCode::EventDisclosureDenied.as_str(),
            "event_disclosure_denied"
        );
        assert!(!AuthorizationDecision::denied(IdentityDenialCode::ScopeDenied).is_allowed());
        assert_eq!(
            AuthorizationDecision::Allowed.denial_code(),
            None,
            "allowed decisions carry no denial code"
        );
    }
}
