//! Closed wire types for the stateful gateway.

use serde::{Deserialize, Serialize};

/// Maximum accepted size of a delivery request body in bytes.
pub const MAX_REQUEST_BYTES: usize = 8 * 1024;
/// Maximum encoded length of an endpoint grant envelope in bytes.
pub const MAX_GRANT_BYTES: usize = 4096;
/// Maximum length of a hex-encoded APNs endpoint token in bytes.
pub const MAX_ENDPOINT_HEX_BYTES: usize = 512;
/// Compiled-in APNs reconnect payload sent for every delivery.
pub const APNS_RECONNECT_PAYLOAD: &[u8] =
    br#"{"aps":{"alert":{"body":"Reconnect to your relay now"},"mutable-content":1}}"#;
/// Wire protocol version for gateway request/response bodies.
pub const WIRE_VERSION: u8 = 1;

/// App profile identifying an iOS app/release track (production vs sandbox).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppProfile {
    /// Buzz iOS production track.
    BuzzIosProduction,
    /// Buzz iOS sandbox track.
    BuzzIosSandbox,
}
impl AppProfile {
    /// Lowercase kebab-case string form of the profile.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BuzzIosProduction => "buzz-ios-production",
            Self::BuzzIosSandbox => "buzz-ios-sandbox",
        }
    }
}

/// Relay request. It deliberately has no application-payload field:
/// the gateway emits one compiled-in APNs reconnect payload for every delivery.
/// `endpoint_grant` is opaque authenticated ciphertext minted by the gateway
/// sealing key and persisted with the relay-owned lease.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryRequest {
    /// Wire version.
    pub v: u8,
    /// Opaque authenticated endpoint-grant ciphertext.
    pub endpoint_grant: String,
    /// Unique id of the delivery request.
    pub request_id: uuid::Uuid,
    /// Unix timestamp at which the request expires.
    pub expires_at: i64,
}

/// Opaque delivery capability plaintext. It contains no APNs token: the random
/// delegation id resolves through durable authority state, while the remaining
/// fields are authenticated fences that make stale or cross-relay use fail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointGrant {
    /// Wire version.
    pub v: u8,
    /// The durable delegation id backing the grant.
    pub delegation_id: uuid::Uuid,
    /// Nostr relay public key (hex) the grant is bound to.
    pub relay_pubkey: String,
    /// App profile of the installation.
    pub app_profile: AppProfile,
    /// Endpoint epoch the grant is bound to.
    pub endpoint_epoch: i64,
    /// Delegation generation at mint time.
    pub generation: i64,
    /// Unix timestamp at which the grant expires.
    pub expires_at: i64,
}

/// Request body for an installation enrollment challenge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationChallengeRequest {
    /// Wire version.
    pub v: u8,
}

/// Response body returning a freshly issued enrollment challenge.
#[derive(Debug, Clone, Serialize)]
pub struct InstallationChallengeResponse {
    /// Unique id of the issued challenge.
    pub challenge_id: uuid::Uuid,
    /// Base64-encoded challenge value.
    pub challenge: String,
    /// Unix timestamp at which the challenge expires.
    pub expires_at: i64,
}

/// Direct app enrollment. `attestation` is Apple's CBOR object and `key_id` is
/// the App Attest key identifier, both base64 encoded. The attested key is the
/// installation authority; no second application signing key is introduced.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationEnrollRequest {
    /// Wire version.
    pub v: u8,
    /// Unique id of the challenge being answered.
    pub challenge_id: uuid::Uuid,
    /// Base64-encoded challenge value.
    pub challenge: String,
    /// Base64-encoded App Attest key id.
    pub key_id: String,
    /// Base64-encoded Apple attestation CBOR.
    pub attestation: String,
    /// App profile to enroll.
    pub app_profile: AppProfile,
    /// Hex-encoded APNs endpoint token.
    pub endpoint: String,
    /// Endpoint epoch for the submitted token.
    pub endpoint_epoch: i64,
    /// Unix timestamp at which the installation should expire.
    pub expires_at: i64,
}

/// Response body returning the created installation handle.
#[derive(Debug, Clone, Serialize)]
pub struct InstallationEnrollResponse {
    /// Opaque handle identifying the installation.
    pub installation_handle: uuid::Uuid,
    /// Endpoint epoch recorded for the installation.
    pub endpoint_epoch: i64,
    /// Unix timestamp at which the installation expires.
    pub expires_at: i64,
}

/// Request body delegating delivery authority to a relay.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationRequest {
    /// Wire version.
    pub v: u8,
    /// Unique id of the challenge being answered.
    pub challenge_id: uuid::Uuid,
    /// Base64-encoded challenge value.
    pub challenge: String,
    /// Handle of the installation delegating.
    pub installation_handle: uuid::Uuid,
    /// Endpoint epoch the delegation is bound to.
    pub endpoint_epoch: i64,
    /// Requested delegation generation.
    pub generation: i64,
    /// Nostr relay public key (hex) receiving the delegation.
    pub relay_pubkey: String,
    /// Unix timestamp from which the delegation is valid.
    pub not_before: i64,
    /// Unix timestamp at which the delegation expires.
    pub expires_at: i64,
    /// Base64-encoded App Attest assertion over the request transcript.
    pub assertion: String,
}

/// Response body returning the minted endpoint grant.
#[derive(Debug, Clone, Serialize)]
pub struct DelegationResponse {
    /// Opaque authenticated endpoint-grant ciphertext.
    pub endpoint_grant: String,
}

/// Request body rotating an installation's APNs endpoint token.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotateEndpointRequest {
    /// Wire version.
    pub v: u8,
    /// Unique id of the challenge being answered.
    pub challenge_id: uuid::Uuid,
    /// Base64-encoded challenge value.
    pub challenge: String,
    /// Handle of the installation rotating its endpoint.
    pub installation_handle: uuid::Uuid,
    /// Expected current endpoint epoch (compare-and-swap guard).
    pub endpoint_epoch: i64,
    /// New endpoint epoch after rotation.
    pub new_endpoint_epoch: i64,
    /// Hex-encoded new APNs endpoint token.
    pub endpoint: String,
    /// Base64-encoded App Attest assertion over the request transcript.
    pub assertion: String,
}

/// Request body revoking a relay's delegation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeDelegationRequest {
    /// Wire version.
    pub v: u8,
    /// Unique id of the challenge being answered.
    pub challenge_id: uuid::Uuid,
    /// Base64-encoded challenge value.
    pub challenge: String,
    /// Handle of the installation whose delegation is revoked.
    pub installation_handle: uuid::Uuid,
    /// Nostr relay public key (hex) losing the delegation.
    pub relay_pubkey: String,
    /// Delegation generation to revoke.
    pub generation: i64,
    /// Base64-encoded App Attest assertion over the request transcript.
    pub assertion: String,
}

/// Request body revoking an installation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeInstallationRequest {
    /// Wire version.
    pub v: u8,
    /// Unique id of the challenge being answered.
    pub challenge_id: uuid::Uuid,
    /// Base64-encoded challenge value.
    pub challenge: String,
    /// Handle of the installation to revoke.
    pub installation_handle: uuid::Uuid,
    /// Expected current endpoint epoch (compare-and-swap guard).
    pub endpoint_epoch: i64,
    /// New endpoint epoch after revocation.
    pub new_endpoint_epoch: i64,
    /// Base64-encoded App Attest assertion over the request transcript.
    pub assertion: String,
}

/// Generic mutation success response.
#[derive(Debug, Clone, Serialize)]
pub struct MutationResponse {
    /// Human-readable status string.
    pub status: &'static str,
}

/// Outcome of a delivery attempt, serialized as the response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", deny_unknown_fields)]
pub enum DeliveryResponse {
    /// APNs accepted the delivery.
    Accepted,
    /// The endpoint is permanently invalid.
    InvalidEndpoint {
        /// Delegation generation at invalidation.
        generation: i64,
        /// When the endpoint became invalid, if known.
        invalid_at: Option<i64>,
    },
    /// The delivery may be retried after an optional delay.
    Retry {
        /// Suggested delay before retry, in seconds.
        retry_after_seconds: Option<i64>,
    },
}

/// Error response body.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    /// Stable machine-readable error code.
    pub error: &'static str,
}
