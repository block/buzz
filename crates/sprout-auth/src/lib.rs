#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `sprout-auth` — Authentication and authorization for the Sprout relay.
//!
//! ## Auth paths
//!
//! | Path | Transport | Description |
//! |------|-----------|-------------|
//! | NIP-42 | WebSocket | Challenge/response; client signs kind:22242 event |
//! | NIP-98 | HTTP | Signed kind:27235 event in `Authorization: Nostr` header |
//!
//! ## Security invariants
//!
//! - **AUTH events (kind:22242) are NEVER stored or logged.**
//! - All paths produce an [`AuthContext`] bound to the connection.
//! - No JWT validation, no token management, no IdP runtime dependency.

/// Channel access checking trait and helpers.
pub mod access;
/// Authentication error types.
pub mod error;
/// NIP-42 challenge–response authentication.
pub mod nip42;
/// NIP-98 HTTP Auth verification (kind:27235).
pub mod nip98;
/// Per-connection rate limiting.
pub mod rate_limit;
/// OAuth scope parsing and enforcement.
pub mod scope;

pub use access::{check_read_access, check_write_access, require_scope, ChannelAccessChecker};
pub use error::AuthError;
pub use nip42::{generate_challenge, verify_nip42_event};
pub use nip98::verify_nip98_event;
pub use rate_limit::{
    ip_rate_limit_key, rate_limit_key, LimitType, RateLimitConfig, RateLimitResult, RateLimiter,
};
pub use scope::{parse_scopes, Scope};

#[cfg(any(test, feature = "test-utils"))]
pub use access::MockAccessChecker;
#[cfg(any(test, feature = "test-utils"))]
pub use rate_limit::AlwaysAllowRateLimiter;

/// How the connection was authenticated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    /// NIP-42 challenge/response — Schnorr signature over kind:22242.
    Nip42,
    /// NIP-98 HTTP Auth — Schnorr signature over kind:27235.
    Nip98,
}

/// The result of a successful authentication, bound to a connection.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// The authenticated Nostr public key.
    pub pubkey: nostr::PublicKey,
    /// Permission scopes granted to this connection.
    pub scopes: Vec<Scope>,
    /// Channel restriction (reserved for future per-channel access control).
    ///
    /// `None` means unrestricted.
    pub channel_ids: Option<Vec<uuid::Uuid>>,
    /// How the connection was authenticated.
    pub auth_method: AuthMethod,
    /// NIP-OA verified owner pubkey (if authenticated via owner attestation).
    ///
    /// `None` for direct relay members or non-NIP-OA auth paths.
    /// Set by the relay membership gate when NIP-OA fallback succeeds.
    pub agent_owner_pubkey: Option<nostr::PublicKey>,
}

impl AuthContext {
    /// Returns `true` if this context includes the given [`Scope`].
    pub fn has_scope(&self, scope: &Scope) -> bool {
        self.scopes.contains(scope)
    }
}

/// Top-level authentication configuration, typically loaded from the relay's TOML config file.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AuthConfig {
    /// Per-user and per-IP rate limit thresholds.
    #[serde(default)]
    pub rate_limits: RateLimitConfig,
}

/// Simplified auth service — NIP-42 and NIP-98 only.
/// No JWT validation, no token management, no IdP runtime dependency.
#[derive(Debug, Clone)]
pub struct AuthService {
    config: AuthConfig,
}

impl AuthService {
    /// Create a new `AuthService` with the given configuration.
    pub fn new(config: AuthConfig) -> Self {
        Self { config }
    }

    /// Return a reference to the auth configuration.
    pub fn config(&self) -> &AuthConfig {
        &self.config
    }

    /// Verify a NIP-42 AUTH event and return an [`AuthContext`].
    ///
    /// Pure cryptographic verification — no network calls, no JWT, no tokens.
    pub async fn verify_auth_event(
        &self,
        auth_event: nostr::Event,
        expected_challenge: &str,
        relay_url: &str,
    ) -> Result<AuthContext, AuthError> {
        // Verify NIP-42 signature (spawn_blocking for CPU-bound Schnorr verify)
        let event_clone = auth_event.clone();
        let challenge_owned = expected_challenge.to_string();
        let relay_owned = relay_url.to_string();
        tokio::task::spawn_blocking(move || {
            verify_nip42_event(&event_clone, &challenge_owned, &relay_owned)
        })
        .await
        .map_err(|_| AuthError::Internal("spawn_blocking panicked".into()))??;

        // In pure Nostr mode, all authenticated connections get full scopes.
        // Per-channel access is enforced by the relay's membership checks (NIP-29).
        Ok(AuthContext {
            pubkey: auth_event.pubkey,
            scopes: Scope::all_known(),
            channel_ids: None,
            auth_method: AuthMethod::Nip42,
            agent_owner_pubkey: None, // Set later by relay membership gate if NIP-OA
        })
    }
}

/// Derive a deterministic Nostr pubkey from a username string.
///
/// Uses `SHA-256("sprout-test-key:{username}")` as the secret key material.
/// This matches the derivation used by the desktop's `set_test_identity` function,
/// allowing the relay to resolve usernames to Nostr pubkeys in dev mode.
///
/// # ⚠️ SECURITY — Dev/test only
///
/// This function is gated behind `#[cfg(any(test, feature = "dev"))]`
/// and **must never be compiled into a production release build**.
///
/// - The derived keys are deterministic and predictable from the username alone.
/// - Any attacker who knows a username can compute the corresponding private key.
#[cfg(any(test, feature = "dev"))]
pub fn derive_pubkey_from_username(username: &str) -> Result<nostr::PublicKey, AuthError> {
    use sha2::{Digest, Sha256};
    let seed = format!("sprout-test-key:{username}");
    let hash: [u8; 32] = Sha256::digest(seed.as_bytes()).into();
    let secret_key = nostr::SecretKey::from_slice(&hash)
        .map_err(|e| AuthError::Internal(format!("key derivation failed: {e}")))?;
    Ok(nostr::Keys::new(secret_key).public_key())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, RelayUrl};

    fn make_auth_event(keys: &Keys, challenge: &str, relay_url: &str) -> nostr::Event {
        let url = RelayUrl::parse(relay_url).expect("valid url");
        EventBuilder::auth(challenge, url)
            .sign_with_keys(keys)
            .expect("signing failed")
    }

    fn test_service() -> AuthService {
        AuthService::new(AuthConfig::default())
    }

    #[test]
    fn auth_context_scope_check() {
        let keys = Keys::generate();
        let ctx = AuthContext {
            pubkey: keys.public_key(),
            scopes: vec![Scope::MessagesRead, Scope::ChannelsRead],
            channel_ids: None,
            auth_method: AuthMethod::Nip42,
            agent_owner_pubkey: None,
        };
        assert!(ctx.has_scope(&Scope::MessagesRead));
        assert!(!ctx.has_scope(&Scope::MessagesWrite));
    }

    #[tokio::test]
    async fn nip42_auth_succeeds() {
        let keys = Keys::generate();
        let challenge = generate_challenge();
        let relay = "wss://relay.example.com";
        let event = make_auth_event(&keys, &challenge, relay);

        let ctx = test_service()
            .verify_auth_event(event, &challenge, relay)
            .await
            .expect("NIP-42 auth should succeed");

        assert_eq!(ctx.pubkey, keys.public_key());
        assert_eq!(ctx.auth_method, AuthMethod::Nip42);
        assert!(ctx.has_scope(&Scope::MessagesRead));
        assert!(ctx.has_scope(&Scope::MessagesWrite));
    }

    #[tokio::test]
    async fn wrong_challenge_rejected() {
        let keys = Keys::generate();
        let challenge = generate_challenge();
        let relay = "wss://relay.example.com";
        let event = make_auth_event(&keys, &challenge, relay);

        let result = test_service()
            .verify_auth_event(event, "wrong-challenge", relay)
            .await;
        assert!(matches!(result, Err(AuthError::ChallengeMismatch)));
    }

    #[tokio::test]
    async fn wrong_kind_rejected() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "not auth")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");

        let result = test_service()
            .verify_auth_event(event, &generate_challenge(), "wss://relay.example.com")
            .await;
        assert!(matches!(result, Err(AuthError::InvalidSignature)));
    }
}
