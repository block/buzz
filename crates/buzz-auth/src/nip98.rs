//! NIP-98 HTTP Auth verification (kind:27235).
//!
//! NIP-98 is the standard Nostr HTTP Auth pattern used by Nostr.build, Blossom, and
//! other Nostr HTTP services. It is **stateless** — no WebSocket session required.
//!
//! The client signs a short-lived kind:27235 event containing the target URL, HTTP method,
//! and an optional SHA-256 hash of the request body, then sends it as:
//!
//! ```text
//! Authorization: Nostr <base64(JSON-serialized-event)>
//! ```
//!
//! ## Verification steps
//!
//! 1. Parse JSON into a `nostr::Event`
//! 2. Verify `kind == 27235` (`Kind::HttpAuth`)
//! 3. Verify Schnorr signature via `sprout_core::verify_event`
//! 4. Verify `created_at` within ±60 seconds of server time
//! 5. Verify `["u", <url>]` tag matches `expected_url` (normalised: case-insensitive
//!    scheme/host, trailing slash stripped)
//! 6. Verify `["method", <method>]` tag matches `expected_method` (case-insensitive)
//! 7. If `["payload", <hash>]` tag is present **and** `body` is `Some`: verify
//!    `SHA-256(body) == hex(payload_tag)`. This prevents body-substitution attacks.
//! 8. Return `event.pubkey` on success.

use nostr::{Alphabet, Event, Kind, SingleLetterTag, TagKind, Timestamp};
use sha2::{Digest, Sha256};
use url::Url;

use crate::error::AuthError;

const TIMESTAMP_TOLERANCE_SECS: u64 = 60;

/// Verify a NIP-98 HTTP Auth event (kind:27235).
///
/// # Parameters
///
/// - `event_json` — the raw JSON string of the Nostr event (decoded from base64 by the caller).
/// - `expected_url` — the canonical URL of the request being authenticated.
///   For reverse-proxy deployments, reconstruct from `X-Forwarded-Proto` / `X-Forwarded-Host`
///   before passing here.
/// - `expected_method` — the HTTP method (e.g. `"POST"`). Compared case-insensitively.
/// - `body` — raw request body bytes. If `Some` and a `payload` tag is present in the event,
///   the SHA-256 hash of `body` must match the tag value. If `None`, the `payload` tag is
///   ignored (clients SHOULD include it for POST requests, but it is not required).
///
/// # Returns
///
/// The authenticated `nostr::PublicKey` on success.
///
/// # Errors
///
/// Returns [`AuthError::Nip98Invalid`] with a descriptive message for any verification failure.
/// The message is safe for server logs but should not be forwarded verbatim to clients.
pub fn verify_nip98_event(
    event_json: &str,
    expected_url: &str,
    expected_method: &str,
    body: Option<&[u8]>,
) -> Result<nostr::PublicKey, AuthError> {
    // 1. Parse JSON.
    let event: Event = serde_json::from_str(event_json)
        .map_err(|e| AuthError::Nip98Invalid(format!("event JSON parse error: {e}")))?;

    // 2. Verify kind == 27235.
    if event.kind != Kind::HttpAuth {
        return Err(AuthError::Nip98Invalid(format!(
            "expected kind 27235, got {}",
            event.kind.as_u16()
        )));
    }

    // 3. Verify Schnorr signature (also verifies event ID hash).
    sprout_core::verify_event(&event)
        .map_err(|_| AuthError::Nip98Invalid("invalid Schnorr signature".to_string()))?;

    // 4. Verify created_at within ±60 seconds of now.
    let now = Timestamp::now().as_secs();
    let event_ts = event.created_at.as_secs();
    let delta = now.abs_diff(event_ts);
    if delta > TIMESTAMP_TOLERANCE_SECS {
        return Err(AuthError::Nip98Invalid(format!(
            "event timestamp outside ±{TIMESTAMP_TOLERANCE_SECS}s window (delta: {delta}s)"
        )));
    }

    // 5. Verify `u` tag matches expected_url (normalised).
    // NIP-98 uses the single-letter "u" tag, not the multi-letter "url" tag.
    let u_tag = event
        .tags
        .find(TagKind::SingleLetter(SingleLetterTag::lowercase(
            Alphabet::U,
        )))
        .and_then(|t| t.content())
        .ok_or_else(|| AuthError::Nip98Invalid("missing `u` tag".to_string()))?;

    if normalize_url(u_tag) != normalize_url(expected_url) {
        return Err(AuthError::Nip98Invalid(format!(
            "URL mismatch: event has `{u_tag}`, expected `{expected_url}`"
        )));
    }

    // 6. Verify `method` tag matches expected_method (case-insensitive).
    let method_tag = event
        .tags
        .find(TagKind::Method)
        .and_then(|t| t.content())
        .ok_or_else(|| AuthError::Nip98Invalid("missing `method` tag".to_string()))?;

    if !method_tag.eq_ignore_ascii_case(expected_method) {
        return Err(AuthError::Nip98Invalid(format!(
            "method mismatch: event has `{method_tag}`, expected `{expected_method}`"
        )));
    }

    // 7. If `payload` tag present AND body is Some: verify SHA-256(body) == payload hex.
    let payload_tag = event.tags.find(TagKind::Payload).and_then(|t| t.content());

    if let (Some(payload_hex), Some(body_bytes)) = (payload_tag, body) {
        let computed: [u8; 32] = Sha256::digest(body_bytes).into();
        let computed_hex = hex::encode(computed);
        if computed_hex != payload_hex {
            return Err(AuthError::Nip98Invalid(
                "payload tag SHA-256 mismatch: request body does not match signed hash".to_string(),
            ));
        }
    }

    // 8. Return the authenticated pubkey.
    Ok(event.pubkey)
}

/// Normalize a URL for comparison.
///
/// - Lowercases scheme and host (already done by the `url` crate).
/// - Strips trailing slash from path.
/// - Treats `localhost` and `::1` as equivalent to `127.0.0.1`.
fn normalize_url(raw: &str) -> String {
    let mut parsed = match Url::parse(raw) {
        Ok(u) => u,
        Err(_) => return raw.to_lowercase(),
    };
    if let Some(host) = parsed.host_str() {
        if host == "localhost" || host == "::1" {
            let _ = parsed.set_host(Some("127.0.0.1"));
        }
    }
    let path = parsed.path().trim_end_matches('/').to_string();
    parsed.set_path(&path);
    parsed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Timestamp};

    const TEST_URL: &str = "https://relay.example.com/api/tokens";
    const TEST_METHOD: &str = "POST";

    fn make_nip98_event(
        keys: &Keys,
        url: &str,
        method: &str,
        payload_hex: Option<&str>,
        created_at: Option<Timestamp>,
    ) -> String {
        use nostr::Tag;

        let mut tags = vec![
            Tag::parse(["u", url]).unwrap(),
            Tag::parse(["method", method]).unwrap(),
        ];
        if let Some(hex) = payload_hex {
            tags.push(Tag::parse(["payload", hex]).unwrap());
        }

        let mut builder = EventBuilder::new(Kind::HttpAuth, "").tags(tags);
        if let Some(ts) = created_at {
            builder = builder.custom_created_at(ts);
        }
        let event = builder.sign_with_keys(keys).expect("sign");
        serde_json::to_string(&event).expect("serialize")
    }

    #[test]
    fn valid_event_returns_pubkey() {
        let keys = Keys::generate();
        let json = make_nip98_event(&keys, TEST_URL, TEST_METHOD, None, None);
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, None);
        assert!(result.is_ok(), "verify failed: {:?}", result.err());
        assert_eq!(result.unwrap(), keys.public_key());
    }

    #[test]
    fn wrong_kind_rejected() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        let json = serde_json::to_string(&event).unwrap();
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, None);
        assert!(matches!(result, Err(AuthError::Nip98Invalid(_))));
    }

    #[test]
    fn expired_timestamp_rejected() {
        let keys = Keys::generate();
        let old_ts = Timestamp::from(Timestamp::now().as_secs().saturating_sub(120));
        let json = make_nip98_event(&keys, TEST_URL, TEST_METHOD, None, Some(old_ts));
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, None);
        assert!(matches!(result, Err(AuthError::Nip98Invalid(_))));
    }

    #[test]
    fn url_mismatch_rejected() {
        let keys = Keys::generate();
        let json = make_nip98_event(
            &keys,
            "https://other.example.com/api/tokens",
            TEST_METHOD,
            None,
            None,
        );
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, None);
        assert!(matches!(result, Err(AuthError::Nip98Invalid(_))));
    }

    #[test]
    fn method_mismatch_rejected() {
        let keys = Keys::generate();
        let json = make_nip98_event(&keys, TEST_URL, "GET", None, None);
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, None);
        assert!(matches!(result, Err(AuthError::Nip98Invalid(_))));
    }

    #[test]
    fn method_case_insensitive() {
        let keys = Keys::generate();
        let json = make_nip98_event(&keys, TEST_URL, "post", None, None);
        let result = verify_nip98_event(&json, TEST_URL, "POST", None);
        assert!(result.is_ok());
    }

    #[test]
    fn payload_tag_correct_hash_passes() {
        let keys = Keys::generate();
        let body = b"hello world";
        let hash: [u8; 32] = Sha256::digest(body).into();
        let hash_hex = hex::encode(hash);
        let json = make_nip98_event(&keys, TEST_URL, TEST_METHOD, Some(&hash_hex), None);
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, Some(body));
        assert!(result.is_ok());
    }

    #[test]
    fn payload_tag_wrong_hash_rejected() {
        let keys = Keys::generate();
        let body = b"hello world";
        let wrong_hex = "deadbeef".repeat(8); // 64 hex chars but wrong hash
        let json = make_nip98_event(&keys, TEST_URL, TEST_METHOD, Some(&wrong_hex), None);
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, Some(body));
        assert!(matches!(result, Err(AuthError::Nip98Invalid(_))));
    }

    #[test]
    fn payload_tag_absent_with_body_passes() {
        // payload tag is optional per spec; clients SHOULD include it but it's not required
        let keys = Keys::generate();
        let json = make_nip98_event(&keys, TEST_URL, TEST_METHOD, None, None);
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, Some(b"some body"));
        assert!(result.is_ok());
    }

    #[test]
    fn trailing_slash_normalized() {
        let keys = Keys::generate();
        let url_with_slash = "https://relay.example.com/api/tokens/";
        let json = make_nip98_event(&keys, url_with_slash, TEST_METHOD, None, None);
        // expected_url without trailing slash — should still match
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, None);
        assert!(result.is_ok());
    }

    #[test]
    fn localhost_normalized() {
        let keys = Keys::generate();
        let localhost_url = "http://localhost:3000/api/tokens";
        let loopback_url = "http://127.0.0.1:3000/api/tokens";
        let json = make_nip98_event(&keys, localhost_url, TEST_METHOD, None, None);
        let result = verify_nip98_event(&json, loopback_url, TEST_METHOD, None);
        assert!(result.is_ok());
    }
}
