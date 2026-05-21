//! NIP-42 challenge/response authentication.
//!
//! 1. Relay sends `["AUTH", "<challenge>"]` via [`generate_challenge`].
//! 2. Client signs a kind:22242 event with challenge + relay tags.
//! 3. Relay validates via [`verify_nip42_event`].
//!
//! AUTH events are **never** stored or logged (may contain bearer tokens).

use nostr::{Event, Kind, TagKind, Timestamp};
use url::Url;

use crate::error::AuthError;

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

    sprout_core::verify_event(event).map_err(|_| AuthError::InvalidSignature)?;

    let challenge = event
        .tags
        .find(TagKind::Challenge)
        .and_then(|t| t.content())
        .ok_or(AuthError::ChallengeMismatch)?;

    if challenge != expected_challenge {
        return Err(AuthError::ChallengeMismatch);
    }

    let relay = event
        .tags
        .find(TagKind::Relay)
        .and_then(|t| t.content())
        .ok_or(AuthError::RelayUrlMismatch)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, RelayUrl, Timestamp};

    const TEST_RELAY: &str = "wss://relay.example.com";

    fn make_auth_event(keys: &Keys, challenge: &str, relay_url: &str) -> Event {
        let url = RelayUrl::parse(relay_url).expect("valid relay url");
        EventBuilder::auth(challenge, url)
            .sign_with_keys(keys)
            .expect("signing failed")
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
