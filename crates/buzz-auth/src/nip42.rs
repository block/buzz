//! NIP-42 challenge/response authentication.
//!
//! 1. Relay sends `["AUTH", "<challenge>"]` via [`generate_challenge`].
//! 2. Client signs a kind:22242 event with challenge + relay tags.
//! 3. Relay validates via [`verify_nip42_event`].
//!
//! AUTH events are **never** stored or logged (may contain bearer tokens).

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use nostr::{Event, Kind, TagKind, Timestamp};
use thiserror::Error;
use url::Url;

use crate::error::AuthError;
use crate::foundation::{ProofTransport, VerifiedNostrProof};

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

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, RelayUrl, Tag, Timestamp};

    const TEST_RELAY: &str = "wss://relay.example.com";

    fn make_auth_event(keys: &Keys, challenge: &str, relay_url: &str) -> Event {
        let url = RelayUrl::parse(relay_url).expect("valid relay url");
        EventBuilder::auth(challenge, url)
            .sign_with_keys(keys)
            .expect("signing failed")
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
