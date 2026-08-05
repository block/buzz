#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-auth` — Authentication and authorization for the Buzz relay.
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
/// Versioned, transport-neutral authorization context.
pub mod context;
/// Authentication error types.
pub mod error;
/// NIP-42 challenge–response authentication.
pub mod nip42;
/// NIP-98 HTTP Auth verification (kind:27235).
pub mod nip98;
/// NIP-98 replay protection — shared, community-scoped, atomic seen-set.
pub mod nip98_replay;
/// Provider-neutral authorization policy and validated capability snapshots.
pub mod provider;
/// Per-connection rate limiting.
pub mod rate_limit;
/// OAuth scope parsing and enforcement.
pub mod scope;

pub use access::{check_read_access, check_write_access, require_scope, ChannelAccessChecker};
pub use context::{
    resolve_current_federated_policy, AdmissionExpiry, AssertionExpiry, AssertionNotBefore,
    AssertionTransport, AuthContext, AuthContextError, AuthContextInput, AuthContextV1,
    AuthContextVersion, AuthMethod, AuthTransport, AuthorityAdapterError, AuthorityAdapterFuture,
    AuthorizationReason, AuthorizedCommunityAccess, BindingResolutionRequest, BindingSource,
    BindingVersion, CapabilityFinalizationSeal, CurrentPolicyRequest, CurrentPolicyResolutionSink,
    DelegationCapability, DelegationExpiry, DirectBindingResolutionSink, EnrollmentMode,
    ExistingBindingResolutionSink, FederatedAuthorityAdapter, FederatedAuthorization,
    FederatedIdentityRequirement, FederatedPrincipal, NostrAuthority, ResolvedFederatedPolicy,
    VerifiedFederatedAssertion, VerifiedKeyAttestation, VerifiedNostrProof, VerifiedOwnerAdmission,
    VerifiedTransportDelegation, VersionedBindingRef,
};
pub use error::AuthError;
pub use nip42::{generate_challenge, verify_nip42_event};
pub use nip98::verify_nip98_event;
pub use nip98_replay::{
    nip98_replay_key, nip98_replay_key_for_scope, Nip98ReplayGuard, DEFAULT_REPLAY_TTL_SECS,
    MAX_REPLAY_TTL_SECS,
};
pub use provider::{
    AuthorizationAuthority, AuthorizationCapability, AuthorizationClock, AuthorizationDenial,
    AuthorizationDenialReason, AuthorizationOutcome, AuthorizationProfileId, AuthorizationProvider,
    AuthorizationProviderFuture, AuthorizationRequest, AuthorizationRuntime, CapabilitySet,
    CapabilitySnapshot, DecisionSource, PolicyVersion, ProviderAllow, ProviderAllowReason,
    ProviderAuthorizationError, ProviderContractError, ProviderDecision, ProviderTimeout,
    ProviderUnavailable, ProviderUnavailableReason, RetryAfter, MAX_PROVIDER_FRESHNESS_SECONDS,
    MAX_PROVIDER_TIMEOUT,
};
pub use rate_limit::{
    ip_rate_limit_key, rate_limit_key, LimitType, RateLimitConfig, RateLimitResult, RateLimiter,
};
pub use scope::{parse_scopes, Scope};

/// Existing NIP authentication result stored on a relay connection.
///
/// This remains separate from [`AuthContext`], which is finalized only after
/// transport authentication and every configured authorization policy pass.
#[derive(Clone)]
pub struct ConnectionAuthContext {
    /// The authenticated Nostr public key.
    pub pubkey: nostr::PublicKey,
    /// Permission scopes granted to this connection.
    pub scopes: Vec<Scope>,
    /// Channel restriction (`None` means unrestricted).
    pub channel_ids: Option<Vec<uuid::Uuid>>,
    /// How the connection was authenticated.
    pub auth_method: AuthMethod,
    /// NIP-OA verified owner pubkey, when present.
    pub agent_owner_pubkey: Option<nostr::PublicKey>,
}

impl std::fmt::Debug for ConnectionAuthContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectionAuthContext")
            .field("pubkey", &"[redacted]")
            .field("scopes", &"[redacted]")
            .field("channel_ids", &"[redacted]")
            .field("auth_method", &self.auth_method)
            .field("agent_owner_pubkey", &"[redacted]")
            .finish()
    }
}

impl ConnectionAuthContext {
    /// Returns `true` if this context includes the given [`Scope`].
    pub fn has_scope(&self, scope: &Scope) -> bool {
        self.scopes.contains(scope)
    }
}

#[cfg(any(test, feature = "test-utils"))]
pub use access::MockAccessChecker;
#[cfg(any(test, feature = "test-utils"))]
pub use nip98_replay::AlwaysFreshReplayGuard;
#[cfg(any(test, feature = "test-utils"))]
pub use rate_limit::AlwaysAllowRateLimiter;

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

    /// Verify a NIP-42 AUTH event and return a [`ConnectionAuthContext`].
    ///
    /// Pure cryptographic verification — no network calls, no JWT, no tokens.
    pub async fn verify_auth_event(
        &self,
        auth_event: nostr::Event,
        expected_challenge: &str,
        relay_url: &str,
    ) -> Result<ConnectionAuthContext, AuthError> {
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
        Ok(ConnectionAuthContext {
            pubkey: auth_event.pubkey,
            scopes: Scope::all_known(),
            channel_ids: None,
            auth_method: AuthMethod::Nip42,
            agent_owner_pubkey: None,
        })
    }
}

/// Derive a deterministic Nostr pubkey from a username string.
///
/// Uses `SHA-256("buzz-test-key:{username}")` as the secret key material.
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
    let seed = format!("buzz-test-key:{username}");
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
    fn connection_auth_context_scope_check() {
        let keys = Keys::generate();
        let context = ConnectionAuthContext {
            pubkey: keys.public_key(),
            scopes: vec![Scope::MessagesRead, Scope::ChannelsRead],
            channel_ids: None,
            auth_method: AuthMethod::Nip42,
            agent_owner_pubkey: None,
        };

        assert!(context.has_scope(&Scope::MessagesRead));
        assert!(!context.has_scope(&Scope::MessagesWrite));
    }

    #[test]
    fn connection_auth_context_debug_redacts_authorization_data() {
        let actor = Keys::generate();
        let owner = Keys::generate();
        let channel_id = uuid::Uuid::new_v4();
        let context = ConnectionAuthContext {
            pubkey: actor.public_key(),
            scopes: vec![Scope::MessagesRead],
            channel_ids: Some(vec![channel_id]),
            auth_method: AuthMethod::Nip42,
            agent_owner_pubkey: Some(owner.public_key()),
        };

        assert_eq!(
            format!("{context:?}"),
            concat!(
                "ConnectionAuthContext { pubkey: \"[redacted]\", scopes: \"[redacted]\", ",
                "channel_ids: \"[redacted]\", auth_method: Nip42, ",
                "agent_owner_pubkey: \"[redacted]\" }"
            )
        );
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
