//! NIP-42 challenge/response authentication.
//!
//! 1. Relay sends `["AUTH", "<challenge>"]` via [`generate_challenge`].
//! 2. Client signs a kind:22242 event with challenge + relay tags.
//! 3. Relay validates via [`verify_nip42_event`].
//!
//! AUTH events are **never** stored or logged (may contain bearer tokens).

use buzz_core::client_binding_bootstrap::ClientBindingScopeV1;
use buzz_core::CommunityId;
use chrono::{DateTime, TimeDelta, Utc};
use nostr::{Event, Kind, TagKind, Timestamp};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::error::AuthError;
use crate::foundation::{
    ActiveLocalBinding, AuthorizationError, AuthorizationFinalizer, AuthorizationInput,
    LocalAuthorizationPolicy, LocalBindingResolution, PreparedAuthorization, ProofTransport,
    RouteCapability, VerifiedNostrProof,
};

/// Normalize a relay URL for comparison.
///
/// Uses the `url` crate for proper parsing rather than string manipulation.
/// Normalizes localhost variants to 127.0.0.1 and strips trailing slashes
/// (the `url` crate handles the latter automatically via path normalization).
fn normalize_relay_url(raw: &str) -> String {
    let mut parsed = match Url::parse(raw) {
        Ok(u) => u,
        Err(_) => return raw.to_string(),
    };
    // Treat localhost variants as equivalent by normalizing to 127.0.0.1.
    if let Some(host) = parsed.host_str() {
        if host == "localhost" || host == "::1" {
            let _ = parsed.set_host(Some("127.0.0.1"));
        }
    }
    let path = parsed.path().trim_end_matches('/').to_string();
    parsed.set_path(&path);
    parsed.to_string()
}

const TIMESTAMP_TOLERANCE_SECS: u64 = 60;

fn exact_tag_content<'a>(event: &'a Event, kind: TagKind<'_>) -> Option<&'a str> {
    let mut matching = event.tags.iter().filter(|tag| tag.kind() == kind);
    let tag = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    tag.content()
}

/// Generate a random NIP-42 challenge (32 CSPRNG bytes, hex-encoded).
pub fn generate_challenge() -> String {
    let bytes: [u8; 32] = rand::random();
    hex::encode(bytes)
}

/// Verify a NIP-42 AUTH event.
///
/// Checks kind, signature, challenge, relay URL, and timestamp (±60s).
/// CPU-bound (Schnorr verify) — call via `spawn_blocking` in async contexts.
pub fn verify_nip42_event(
    event: &Event,
    expected_challenge: &str,
    relay_url: &str,
) -> Result<(), AuthError> {
    if event.kind != Kind::Authentication {
        return Err(AuthError::InvalidSignature);
    }

    buzz_core::verify_event(event).map_err(|_| AuthError::InvalidSignature)?;

    let challenge =
        exact_tag_content(event, TagKind::Challenge).ok_or(AuthError::ChallengeMismatch)?;

    if challenge != expected_challenge {
        return Err(AuthError::ChallengeMismatch);
    }

    let relay = exact_tag_content(event, TagKind::Relay).ok_or(AuthError::RelayUrlMismatch)?;

    if normalize_relay_url(relay) != normalize_relay_url(relay_url) {
        return Err(AuthError::RelayUrlMismatch);
    }

    let now = Timestamp::now().as_secs();
    let event_ts = event.created_at.as_secs();
    let delta = now.abs_diff(event_ts);
    if delta > TIMESTAMP_TOLERANCE_SECS {
        return Err(AuthError::EventExpired);
    }

    Ok(())
}

/// Fail-closed result of minting an origin-sealed NIP-42 authorization proof.
#[derive(Debug, Error)]
pub enum Nip42AuthorizationProofError {
    /// The signed NIP-42 event failed cryptographic or protocol verification.
    #[error(transparent)]
    Authentication(#[from] AuthError),
    /// Server-derived binding coordinates were nil, zero, or already expired.
    #[error("invalid NIP-42 authorization proof binding")]
    InvalidBinding,
}

/// Verify a NIP-42 event and mint its origin-sealed authorization proof.
///
/// The caller supplies only server-resolved routing coordinates and hashes of
/// already canonicalized request context. This function performs NIP-42
/// signature/challenge/relay/freshness verification before invoking the
/// crate-private proof constructor. The expiry is exclusive and must still be
/// in the future according to trusted relay time.
#[allow(clippy::too_many_arguments)]
pub fn verify_nip42_authorization_proof(
    event: &Event,
    expected_challenge: &str,
    relay_url: &str,
    authorization_domain: CommunityId,
    request_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
    bound_assertion_fingerprint: Option<[u8; 32]>,
    delegation_conditions_fingerprint: Option<[u8; 32]>,
    expires_at: DateTime<Utc>,
) -> Result<VerifiedNostrProof, Nip42AuthorizationProofError> {
    verify_nip42_event(event, expected_challenge, relay_url)?;
    if expires_at <= Utc::now() {
        return Err(Nip42AuthorizationProofError::InvalidBinding);
    }
    VerifiedNostrProof::from_verifier(
        authorization_domain,
        event.pubkey,
        ProofTransport::Nip42,
        request_fingerprint,
        target_fingerprint,
        transport_context_fingerprint,
        bound_assertion_fingerprint,
        delegation_conditions_fingerprint,
        expires_at,
    )
    .ok_or(Nip42AuthorizationProofError::InvalidBinding)
}

/// Exact server-derived coordinates for one opted-in status connection.
pub struct Nip42BindingStatusCoordinates {
    authorization_domain: CommunityId,
    relay_url: Box<str>,
    event_id: [u8; 32],
    actor: nostr::PublicKey,
    request_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
}

impl Nip42BindingStatusCoordinates {
    /// Bind the signed scope to the relay-owned connection generation.
    pub fn new(
        authorization_domain: CommunityId,
        relay_url: &str,
        connection_id: Uuid,
        relay_signer: nostr::PublicKey,
        event: &Event,
    ) -> Result<Self, Nip42AuthorizationProofError> {
        let scope = ClientBindingScopeV1::from_verified_auth_event(event)
            .map_err(|_| Nip42AuthorizationProofError::InvalidBinding)?;
        let parsed_relay =
            Url::parse(relay_url).map_err(|_| Nip42AuthorizationProofError::InvalidBinding)?;
        if authorization_domain.as_uuid().is_nil()
            || connection_id.is_nil()
            || event.id.to_bytes() == [0; 32]
            || scope.relay_signer() != relay_signer
            || !matches!(parsed_relay.scheme(), "ws" | "wss")
            || parsed_relay.host_str().is_none()
            || !parsed_relay.username().is_empty()
            || parsed_relay.password().is_some()
            || parsed_relay.query().is_some()
            || parsed_relay.fragment().is_some()
        {
            return Err(Nip42AuthorizationProofError::InvalidBinding);
        }
        let event_id = event.id.to_bytes();
        let target_fingerprint = nip42_status_digest(
            b"buzz:client-status-connection:v1",
            &[
                authorization_domain.as_uuid().as_bytes(),
                connection_id.as_bytes(),
                event.pubkey.as_bytes(),
                relay_signer.as_bytes(),
                scope.connection_epoch().as_str().as_bytes(),
            ],
        );
        let request_fingerprint = nip42_status_digest(
            b"buzz:nip-fi:nip42-binding-status-request:v1",
            &[
                authorization_domain.as_uuid().as_bytes(),
                &event_id,
                &target_fingerprint,
            ],
        );
        let transport_context_fingerprint = nip42_status_digest(
            b"buzz:nip-fi:nip42-binding-status-transport:v1",
            &[
                authorization_domain.as_uuid().as_bytes(),
                parsed_relay.as_str().as_bytes(),
                connection_id.as_bytes(),
                &event_id,
            ],
        );
        Ok(Self {
            authorization_domain,
            relay_url: parsed_relay.as_str().into(),
            event_id,
            actor: event.pubkey,
            request_fingerprint,
            target_fingerprint,
            transport_context_fingerprint,
        })
    }

    /// Exact opaque connection target used by the delivery owner.
    pub const fn target_fingerprint(&self) -> [u8; 32] {
        self.target_fingerprint
    }
}

impl std::fmt::Debug for Nip42BindingStatusCoordinates {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Nip42BindingStatusCoordinates([REDACTED])")
    }
}

/// Purpose-sealed proof for the fixed binding-status capability.
pub struct VerifiedBindingStatusProof {
    proof: VerifiedNostrProof,
}

impl VerifiedBindingStatusProof {
    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.proof.authorization_domain()
    }

    /// Exact directly authenticated author.
    pub const fn actor_pubkey(&self) -> nostr::PublicKey {
        self.proof.actor_pubkey()
    }

    /// Exact connection target sealed into the proof.
    pub const fn target_fingerprint(&self) -> &[u8; 32] {
        self.proof.target_fingerprint()
    }

    /// Exclusive proof lifetime bound.
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.proof.expires_at()
    }

    /// Prepare the fixed direct binding-status authorization.
    pub fn prepare_authorization(
        self,
        binding: ActiveLocalBinding,
        policy: LocalAuthorizationPolicy,
        authoritative_now: DateTime<Utc>,
    ) -> Result<PreparedAuthorization, AuthorizationError> {
        let domain = self.proof.authorization_domain();
        let input = AuthorizationInput::new(
            domain,
            Uuid::new_v4(),
            self.proof,
            RouteCapability::BindingStatus,
        )?;
        AuthorizationFinalizer::prepare(
            input,
            LocalBindingResolution::bound_key(binding),
            policy,
            authoritative_now,
        )
    }
}

impl std::fmt::Debug for VerifiedBindingStatusProof {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("VerifiedBindingStatusProof([REDACTED])")
    }
}

/// Verify the exact AUTH event and mint a fixed-purpose status proof.
pub fn verify_nip42_binding_status_proof(
    event: &Event,
    expected_challenge: &str,
    coordinates: &Nip42BindingStatusCoordinates,
    authoritative_now: DateTime<Utc>,
) -> Result<VerifiedBindingStatusProof, Nip42AuthorizationProofError> {
    verify_nip42_event(event, expected_challenge, &coordinates.relay_url)?;
    if event.id.to_bytes() != coordinates.event_id
        || event.pubkey != coordinates.actor
        || event.created_at.as_secs().abs_diff(
            u64::try_from(authoritative_now.timestamp())
                .map_err(|_| Nip42AuthorizationProofError::InvalidBinding)?,
        ) > TIMESTAMP_TOLERANCE_SECS
    {
        return Err(Nip42AuthorizationProofError::InvalidBinding);
    }
    let expires_at = authoritative_now
        .checked_add_signed(TimeDelta::minutes(5))
        .ok_or(Nip42AuthorizationProofError::InvalidBinding)?;
    let proof = VerifiedNostrProof::from_verifier(
        coordinates.authorization_domain,
        event.pubkey,
        ProofTransport::Nip42,
        coordinates.request_fingerprint,
        coordinates.target_fingerprint,
        coordinates.transport_context_fingerprint,
        None,
        None,
        expires_at,
    )
    .ok_or(Nip42AuthorizationProofError::InvalidBinding)?;
    Ok(VerifiedBindingStatusProof { proof })
}

fn nip42_status_digest(label: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update((label.len() as u64).to_be_bytes());
    digest.update(label);
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    digest.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::client_binding_bootstrap::CLIENT_BINDING_SCOPE_TAG;
    use nostr::{EventBuilder, Keys, Kind, RelayUrl, Tag, Timestamp};

    const TEST_RELAY: &str = "wss://relay.example.com";

    fn make_auth_event(keys: &Keys, challenge: &str, relay_url: &str) -> Event {
        let url = RelayUrl::parse(relay_url).expect("valid relay url");
        EventBuilder::auth(challenge, url)
            .sign_with_keys(keys)
            .expect("signing failed")
    }

    fn make_status_auth_event(
        keys: &Keys,
        relay: &Keys,
        challenge: &str,
        relay_url: &str,
    ) -> Event {
        let url = RelayUrl::parse(relay_url).expect("valid relay url");
        let relay_signer = relay.public_key().to_hex();
        let epoch = Uuid::new_v4().to_string();
        EventBuilder::auth(challenge, url)
            .tag(
                Tag::parse([
                    CLIENT_BINDING_SCOPE_TAG,
                    "1",
                    epoch.as_str(),
                    relay_signer.as_str(),
                ])
                .expect("status scope"),
            )
            .sign_with_keys(keys)
            .expect("signing failed")
    }

    #[test]
    fn binding_status_proof_is_exact_connection_and_relay_bound() {
        let author = Keys::generate();
        let relay = Keys::generate();
        let challenge = generate_challenge();
        let event = make_status_auth_event(&author, &relay, &challenge, TEST_RELAY);
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let connection = Uuid::new_v4();
        let coordinates = Nip42BindingStatusCoordinates::new(
            domain,
            TEST_RELAY,
            connection,
            relay.public_key(),
            &event,
        )
        .expect("coordinates");
        let other = Nip42BindingStatusCoordinates::new(
            domain,
            TEST_RELAY,
            Uuid::new_v4(),
            relay.public_key(),
            &event,
        )
        .expect("other coordinates");
        assert_ne!(coordinates.target_fingerprint(), other.target_fingerprint());
        let proof = verify_nip42_binding_status_proof(&event, &challenge, &coordinates, Utc::now())
            .expect("purpose-sealed status proof");
        assert_eq!(proof.authorization_domain(), domain);
        assert_eq!(proof.actor_pubkey(), author.public_key());
        assert_eq!(
            proof.target_fingerprint(),
            &coordinates.target_fingerprint()
        );
        assert!(Nip42BindingStatusCoordinates::new(
            domain,
            TEST_RELAY,
            connection,
            Keys::generate().public_key(),
            &event,
        )
        .is_err());
    }

    fn make_auth_event_with_tags(keys: &Keys, tags: Vec<Tag>) -> Event {
        EventBuilder::new(Kind::Authentication, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("signing failed")
    }

    fn auth_tag(kind: &str, value: &str) -> Tag {
        Tag::parse([kind, value]).expect("valid auth tag")
    }

    #[test]
    fn challenge_is_64_hex_chars_and_unique() {
        let c1 = generate_challenge();
        let c2 = generate_challenge();
        assert_eq!(c1.len(), 64);
        assert!(c1.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(c1, c2);
    }

    #[test]
    fn valid_event_passes() {
        let keys = Keys::generate();
        let challenge = generate_challenge();
        let event = make_auth_event(&keys, &challenge, TEST_RELAY);
        assert!(verify_nip42_event(&event, &challenge, TEST_RELAY).is_ok());
    }

    #[test]
    fn required_tags_are_order_independent() {
        let keys = Keys::generate();
        let challenge = generate_challenge();
        let event = make_auth_event_with_tags(
            &keys,
            vec![
                auth_tag("challenge", &challenge),
                auth_tag("relay", TEST_RELAY),
            ],
        );
        let reversed = make_auth_event_with_tags(
            &keys,
            vec![
                auth_tag("relay", TEST_RELAY),
                auth_tag("challenge", &challenge),
            ],
        );

        assert!(verify_nip42_event(&event, &challenge, TEST_RELAY).is_ok());
        assert!(verify_nip42_event(&reversed, &challenge, TEST_RELAY).is_ok());
    }

    #[test]
    fn challenge_tag_must_appear_exactly_once() {
        let keys = Keys::generate();
        let challenge = generate_challenge();
        let cases = [
            ("missing", vec![auth_tag("relay", TEST_RELAY)]),
            (
                "equal duplicate",
                vec![
                    auth_tag("challenge", &challenge),
                    auth_tag("challenge", &challenge),
                    auth_tag("relay", TEST_RELAY),
                ],
            ),
            (
                "matching then conflicting",
                vec![
                    auth_tag("challenge", &challenge),
                    auth_tag("challenge", "wrong"),
                    auth_tag("relay", TEST_RELAY),
                ],
            ),
            (
                "conflicting then matching",
                vec![
                    auth_tag("challenge", "wrong"),
                    auth_tag("relay", TEST_RELAY),
                    auth_tag("challenge", &challenge),
                ],
            ),
        ];

        for (case, tags) in cases {
            let event = make_auth_event_with_tags(&keys, tags);
            assert!(
                matches!(
                    verify_nip42_event(&event, &challenge, TEST_RELAY),
                    Err(AuthError::ChallengeMismatch)
                ),
                "challenge case {case} must fail closed"
            );
        }
    }

    #[test]
    fn relay_tag_must_appear_exactly_once() {
        let keys = Keys::generate();
        let challenge = generate_challenge();
        let other_relay = "wss://other.example.com";
        let cases = [
            ("missing", vec![auth_tag("challenge", &challenge)]),
            (
                "equal duplicate",
                vec![
                    auth_tag("relay", TEST_RELAY),
                    auth_tag("challenge", &challenge),
                    auth_tag("relay", TEST_RELAY),
                ],
            ),
            (
                "matching then conflicting",
                vec![
                    auth_tag("relay", TEST_RELAY),
                    auth_tag("relay", other_relay),
                    auth_tag("challenge", &challenge),
                ],
            ),
            (
                "conflicting then matching",
                vec![
                    auth_tag("relay", other_relay),
                    auth_tag("challenge", &challenge),
                    auth_tag("relay", TEST_RELAY),
                ],
            ),
        ];

        for (case, tags) in cases {
            let event = make_auth_event_with_tags(&keys, tags);
            assert!(
                matches!(
                    verify_nip42_event(&event, &challenge, TEST_RELAY),
                    Err(AuthError::RelayUrlMismatch)
                ),
                "relay case {case} must fail closed"
            );
        }
    }

    #[test]
    fn wrong_challenge_rejected() {
        let keys = Keys::generate();
        let challenge = generate_challenge();
        let event = make_auth_event(&keys, &challenge, TEST_RELAY);
        assert!(matches!(
            verify_nip42_event(&event, "wrong", TEST_RELAY),
            Err(AuthError::ChallengeMismatch)
        ));
    }

    #[test]
    fn wrong_kind_rejected() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "not auth")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        assert!(matches!(
            verify_nip42_event(&event, "x", TEST_RELAY),
            Err(AuthError::InvalidSignature)
        ));
    }

    #[test]
    fn expired_event_rejected() {
        let keys = Keys::generate();
        let challenge = generate_challenge();
        let url = RelayUrl::parse(TEST_RELAY).unwrap();
        let old_ts = Timestamp::from(Timestamp::now().as_secs().saturating_sub(120));
        let event = EventBuilder::auth(&challenge, url)
            .custom_created_at(old_ts)
            .sign_with_keys(&keys)
            .expect("sign");
        assert!(matches!(
            verify_nip42_event(&event, &challenge, TEST_RELAY),
            Err(AuthError::EventExpired)
        ));
    }

    #[test]
    fn wrong_relay_rejected() {
        let keys = Keys::generate();
        let challenge = generate_challenge();
        let event = make_auth_event(&keys, &challenge, "wss://other.example.com");
        assert!(matches!(
            verify_nip42_event(&event, &challenge, TEST_RELAY),
            Err(AuthError::RelayUrlMismatch)
        ));
    }

    #[test]
    fn localhost_and_127_are_equivalent() {
        let a = normalize_relay_url("ws://localhost:3030");
        let b = normalize_relay_url("ws://127.0.0.1:3030");
        assert_eq!(a, b);
    }

    #[test]
    fn trailing_slash_normalized() {
        let a = normalize_relay_url("wss://relay.example.com/");
        let b = normalize_relay_url("wss://relay.example.com");
        assert_eq!(a, b);
    }
}
