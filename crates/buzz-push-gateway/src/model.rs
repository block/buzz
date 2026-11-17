//! Closed wire types for the stateful gateway.

use serde::{Deserialize, Serialize};

pub const MAX_REQUEST_BYTES: usize = 8 * 1024;
/// Maximum decoded Apple App Attest object accepted by the verifier.
pub const MAX_APP_ATTESTATION_BYTES: usize = 16 * 1024;
/// Enrollment carries the maximum App Attest object as standard base64 plus a
/// bounded APNs endpoint and the closed JSON envelope. Other gateway requests
/// remain subject to `MAX_REQUEST_BYTES`.
pub const MAX_ENROLL_REQUEST_BYTES: usize = MAX_APP_ATTESTATION_BYTES.div_ceil(3) * 4
    + MAX_ENDPOINT_HEX_BYTES * 2
    + MAX_ANDROID_CHAIN_CERTIFICATES * 8
    + 1024;
pub const MAX_GRANT_BYTES: usize = 4096;
pub const MAX_ENDPOINT_HEX_BYTES: usize = 512;
/// Maximum FCM registration token length. It shares the token-custody limit
/// (`MAX_ENDPOINT_HEX_BYTES` plaintext bytes), well above real tokens.
pub const MAX_FCM_TOKEN_BYTES: usize = MAX_ENDPOINT_HEX_BYTES;
/// Maximum certificates in an Android Key Attestation chain.
pub const MAX_ANDROID_CHAIN_CERTIFICATES: usize = 8;
/// Maximum decoded size of a whole Android attestation chain, matching the App
/// Attest object limit.
pub const MAX_ANDROID_ATTESTATION_BYTES: usize = MAX_APP_ATTESTATION_BYTES;
/// Maximum decoded ECDSA P-256 DER signature accepted from an Android key.
pub const MAX_ANDROID_SIGNATURE_BYTES: usize = 80;
pub const APNS_RECONNECT_PAYLOAD: &[u8] =
    br#"{"aps":{"alert":{"body":"Reconnect to your relay now"},"sound":"default","mutable-content":1}}"#;
pub const WIRE_VERSION: u8 = 1;

/// Relay request. It deliberately has no application-payload field:
/// the gateway emits one compiled-in APNs reconnect payload for every delivery.
/// `endpoint_grant` is opaque authenticated ciphertext minted by the gateway
/// sealing key and persisted with the relay-owned lease.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryRequest {
    pub v: u8,
    pub endpoint_grant: String,
    pub request_id: uuid::Uuid,
    pub expires_at: i64,
}

/// Opaque delivery capability plaintext. It contains no APNs token: the random
/// delegation id resolves through durable authority state, while the remaining
/// fields are authenticated fences that make stale or cross-relay use fail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointGrant {
    pub v: u8,
    pub delegation_id: uuid::Uuid,
    pub relay_pubkey: String,
    pub endpoint_epoch: i64,
    pub generation: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationChallengeRequest {
    pub v: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallationChallengeResponse {
    pub challenge_id: uuid::Uuid,
    pub challenge: String,
    pub expires_at: i64,
}

/// Direct app enrollment. It carries exactly one attestation, and the gateway
/// derives the installation's platform from which one is present; there is no
/// platform field. `attestation` is Apple's App Attest CBOR object. `android_attestation`
/// is a Key Attestation certificate chain, leaf first, of base64 DER
/// certificates. `key_id` is the App Attest key identifier, required on iOS
/// and absent on Android: a Keystore key is generated from the attestation
/// challenge, so its id cannot be in the transcript that challenge binds, and
/// the gateway takes it as the SHA-256 of the attested public key. The attested key is
/// the installation authority; no second application signing key is introduced.
/// `endpoint` is a lowercase-hex APNs token on iOS and an FCM registration
/// token on Android.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationEnrollRequest {
    pub v: u8,
    pub challenge_id: uuid::Uuid,
    pub challenge: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_attestation: Option<Vec<String>>,
    pub endpoint: String,
    pub endpoint_epoch: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallationEnrollResponse {
    pub installation_handle: uuid::Uuid,
    pub endpoint_epoch: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationRequest {
    pub v: u8,
    pub challenge_id: uuid::Uuid,
    pub challenge: String,
    pub installation_handle: uuid::Uuid,
    pub endpoint_epoch: i64,
    pub generation: i64,
    pub relay_pubkey: String,
    pub not_before: i64,
    pub expires_at: i64,
    pub assertion: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DelegationResponse {
    pub endpoint_grant: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotateEndpointRequest {
    pub v: u8,
    pub challenge_id: uuid::Uuid,
    pub challenge: String,
    pub installation_handle: uuid::Uuid,
    pub endpoint_epoch: i64,
    pub new_endpoint_epoch: i64,
    pub endpoint: String,
    pub assertion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeDelegationRequest {
    pub v: u8,
    pub challenge_id: uuid::Uuid,
    pub challenge: String,
    pub installation_handle: uuid::Uuid,
    pub relay_pubkey: String,
    pub generation: i64,
    pub assertion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeInstallationRequest {
    pub v: u8,
    pub challenge_id: uuid::Uuid,
    pub challenge: String,
    pub installation_handle: uuid::Uuid,
    pub endpoint_epoch: i64,
    pub new_endpoint_epoch: i64,
    pub assertion: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MutationResponse {
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", deny_unknown_fields)]
pub enum DeliveryResponse {
    Accepted,
    InvalidEndpoint {
        generation: i64,
        invalid_at: Option<i64>,
    },
    Retry {
        retry_after_seconds: Option<i64>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub error: &'static str,
}
