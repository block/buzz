//! Closed wire types for the stateful gateway.

use serde::{Deserialize, Serialize};

pub const MAX_REQUEST_BYTES: usize = 8 * 1024;
/// Maximum decoded Apple App Attest object accepted by the verifier.
pub const MAX_APP_ATTESTATION_BYTES: usize = 16 * 1024;
/// Enrollment carries the maximum App Attest object as standard base64 plus a
/// bounded APNs endpoint and the closed JSON envelope. Other gateway requests
/// remain subject to `MAX_REQUEST_BYTES`.
pub const MAX_ENROLL_REQUEST_BYTES: usize =
    MAX_APP_ATTESTATION_BYTES.div_ceil(3) * 4 + MAX_ENDPOINT_HEX_BYTES * 2 + 1024;
pub const MAX_GRANT_BYTES: usize = 4096;
pub const MAX_ENDPOINT_HEX_BYTES: usize = 512;
pub const APNS_RECONNECT_PAYLOAD: &[u8] =
    br#"{"aps":{"alert":{"body":"Reconnect to your relay now"},"mutable-content":1}}"#;
pub const WIRE_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppProfile {
    BuzzIosDogfood,
    /// Browser/PWA installations delivered over Web Push (VAPID). Enrolled by
    /// a NIP-98-authenticated member pubkey instead of App Attest.
    BuzzWeb,
}
impl AppProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BuzzIosDogfood => "buzz-ios-dogfood",
            Self::BuzzWeb => "buzz-web",
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
    pub v: u8,
    pub endpoint_grant: String,
    pub request_id: uuid::Uuid,
    pub expires_at: i64,
    /// Content-free routing hint, sent by the relay only for `buzz-web`
    /// leases. Ignored (never forwarded) for APNs deliveries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<WebPushHint>,
}

/// Closed, content-free routing hint for Web Push deep links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebPushHint {
    /// Lease subscription class: `dm`, `mention`, `thread_reply`, `needs_action` or `default`.
    pub class: String,
    /// Lowercase hex event id.
    pub event_id: String,
    /// Channel UUID, when the event belongs to a channel.
    pub channel_id: Option<String>,
    /// Thread root event id (lowercase hex) when the event is a thread reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,
}

fn is_hex_id(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl WebPushHint {
    pub fn is_valid(&self) -> bool {
        matches!(
            self.class.as_str(),
            "default" | "dm" | "mention" | "thread_reply" | "needs_action"
        ) && is_hex_id(&self.event_id)
            && self.thread_root_id.as_deref().is_none_or(is_hex_id)
            && self
                .channel_id
                .as_ref()
                .is_none_or(|c| c.len() == 36 && uuid::Uuid::parse_str(c).is_ok())
    }
}

/// `POST /v1/web/installations` (NIP-98 authenticated by the member pubkey).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebInstallationRequest {
    pub v: u8,
    pub subscription: crate::webpush::WebPushSubscription,
    pub expires_at: i64,
}

/// `POST /v1/web/delegations` (NIP-98, same member pubkey as the installation).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebDelegationRequest {
    pub v: u8,
    pub installation_handle: uuid::Uuid,
    pub generation: i64,
    pub relay_pubkey: String,
    pub not_before: i64,
    pub expires_at: i64,
}

/// `POST /v1/web/installations/revoke` (NIP-98, same member pubkey).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebRevokeRequest {
    pub v: u8,
    pub installation_handle: uuid::Uuid,
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
    pub app_profile: AppProfile,
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

/// Direct app enrollment. `attestation` is Apple's CBOR object and `key_id` is
/// the App Attest key identifier, both base64 encoded. The attested key is the
/// installation authority; no second application signing key is introduced.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationEnrollRequest {
    pub v: u8,
    pub challenge_id: uuid::Uuid,
    pub challenge: String,
    pub key_id: String,
    pub attestation: String,
    pub app_profile: AppProfile,
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
