//! NIP-11 relay information document.

use serde::{Deserialize, Serialize};

use crate::connection::MAX_FRAME_BYTES;

/// NIPs supported by this relay, advertised in the NIP-11 document.
/// Kept as a module-level constant so tests can verify it without constructing
/// a full `Config` (which reads env vars and races with config.rs tests).
pub(crate) const SUPPORTED_NIPS: &[u32] = &[1, 2, 10, 11, 16, 17, 23, 25, 29, 33, 38, 42, 43, 50];

/// Relay information document served at `GET /` with `Accept: application/nostr+json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayInfo {
    /// Human-readable relay name.
    pub name: String,
    /// Human-readable relay description.
    pub description: String,
    /// Relay operator's public key (hex), if published.
    pub pubkey: Option<String>,
    /// Contact address for the relay operator.
    pub contact: Option<String>,
    /// NIPs supported by this relay.
    pub supported_nips: Vec<u32>,
    /// URL of the relay software repository.
    pub software: String,
    /// Relay software version string.
    pub version: String,
    /// Protocol and resource limits advertised to clients.
    pub limitation: Option<RelayLimitation>,
    /// Relay's own signing pubkey (NIP-11 `self` field, NIP-43).
    #[serde(rename = "self", skip_serializing_if = "Option::is_none")]
    pub relay_self: Option<String>,
}

/// Protocol and resource limits advertised in the NIP-11 document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayLimitation {
    /// Maximum WebSocket frame size in bytes.
    pub max_message_length: Option<u64>,
    /// Maximum number of concurrent subscriptions per connection.
    pub max_subscriptions: Option<u32>,
    /// Maximum number of filters per subscription.
    pub max_filters: Option<u32>,
    /// Maximum value of the `limit` field in a filter.
    pub max_limit: Option<u32>,
    /// Maximum length of a subscription ID string.
    pub max_subid_length: Option<u32>,
    /// Minimum proof-of-work difficulty required for events.
    pub min_pow_difficulty: Option<u32>,
    /// Whether NIP-42 authentication is required before subscribing or
    /// publishing events.
    pub auth_required: bool,
    /// Whether payment is required to use the relay.
    pub payment_required: bool,
    /// Whether writes are restricted to authorized pubkeys.
    pub restricted_writes: bool,
}

/// Canonical `RelayLimitation` advertised by this relay.
///
/// `auth_required` is always `true`: the REQ, EVENT, and COUNT handlers
/// unconditionally reject connections that are not in
/// `AuthState::Authenticated`. This is independent of the REST API token
/// toggle (`config.require_auth_token`).
fn relay_limitation() -> RelayLimitation {
    RelayLimitation {
        max_message_length: Some(MAX_FRAME_BYTES as u64),
        max_subscriptions: Some(1024),
        max_filters: Some(10),
        max_limit: Some(10_000),
        max_subid_length: Some(256),
        min_pow_difficulty: None,
        auth_required: true,
        payment_required: false,
        restricted_writes: true,
    }
}

impl RelayInfo {
    /// Builds the relay's NIP-11 information document.
    ///
    /// `relay_pubkey` is the relay's own signing pubkey (hex), advertised as the
    /// NIP-11 `self` field for NIP-43 membership verification.
    pub fn build(relay_pubkey: Option<&str>) -> Self {
        Self {
            name: "Sprout Relay".to_string(),
            description: "Sprout — private team communication relay".to_string(),
            pubkey: None,
            contact: None,
            supported_nips: SUPPORTED_NIPS.to_vec(),
            software: "https://github.com/sprout-rs/sprout".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            limitation: Some(relay_limitation()),
            relay_self: relay_pubkey.map(|s| s.to_string()),
        }
    }
}

/// Axum handler that returns the NIP-11 relay information document as JSON.
pub async fn relay_info_handler(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::state::AppState>>,
) -> axum::response::Json<RelayInfo> {
    // Only advertise the NIP-11 `self` field when a stable relay key is configured.
    // Ephemeral (auto-generated) keys change on restart, making signed events unverifiable.
    let relay_pubkey = if state.config.relay_private_key.is_some() {
        Some(state.relay_keypair.public_key().to_hex())
    } else {
        None
    };
    axum::response::Json(RelayInfo::build(relay_pubkey.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_nips_includes_nip23_and_nip33() {
        // Tests the production SUPPORTED_NIPS constant directly — no Config::from_env()
        // needed, avoiding the env-var race with config.rs tests.
        assert!(
            SUPPORTED_NIPS.contains(&23),
            "NIP-23 (long-form content) must be advertised"
        );
        assert!(
            SUPPORTED_NIPS.contains(&33),
            "NIP-33 (parameterized replaceable) must be advertised"
        );
    }

    #[test]
    fn supported_nips_includes_nip38() {
        assert!(
            SUPPORTED_NIPS.contains(&38),
            "NIP-38 (user statuses) must be advertised"
        );
    }

    #[test]
    fn auth_required_is_advertised_true() {
        // REQ, EVENT, and COUNT all unconditionally require
        // `AuthState::Authenticated` (see `crates/sprout-relay/src/handlers/`).
        // The NIP-11 doc must reflect that or clients (e.g. the desktop pair
        // flow) misroute unauthenticated peers.
        assert!(relay_limitation().auth_required);
    }

    #[test]
    fn supported_nips_are_sorted() {
        let mut sorted = SUPPORTED_NIPS.to_vec();
        sorted.sort();
        assert_eq!(
            SUPPORTED_NIPS,
            &sorted[..],
            "supported_nips should be sorted"
        );
    }
}
