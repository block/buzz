//! Twilio request signature validation.
//!
//! Twilio signs every webhook request it sends: HMAC-SHA1 over the full
//! request URL with each POST parameter's key+value appended directly
//! (sorted by key, no separators), keyed by the account's Auth Token,
//! base64-encoded into the `X-Twilio-Signature` header.
//!
//! This is the sole trust boundary for inbound SMS — Twilio cannot hold a
//! Nostr key, so NIP-42/NIP-98 don't apply here. A request that fails this
//! check must be rejected before any relay event is synthesized from it.
//!
//! Reference: <https://www.twilio.com/docs/usage/security#validating-requests>

use std::collections::BTreeMap;

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// Build the string Twilio signs: the request URL followed by each POST
/// parameter's key and value concatenated directly, in ascending key order.
///
/// `params` must already be the parsed (not URL-encoded) form values — a
/// `BTreeMap` is used by the caller specifically so this sees keys in sorted
/// order without needing to sort here.
fn signing_string(url: &str, params: &BTreeMap<String, String>) -> String {
    let mut s = String::with_capacity(
        url.len() + params.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>(),
    );
    s.push_str(url);
    for (k, v) in params {
        s.push_str(k);
        s.push_str(v);
    }
    s
}

/// Validate an inbound Twilio request's `X-Twilio-Signature` header.
///
/// `url` must be the exact full URL Twilio was configured to POST to
/// (scheme + host + path, matching what Twilio itself signed — a mismatched
/// scheme or a stray trailing slash produces a signature mismatch even for a
/// genuine Twilio request). `provided_signature_b64` is the raw header value.
///
/// Returns `false` for a malformed (non-base64) signature as well as a
/// genuine mismatch — callers should map both to the same rejection so the
/// endpoint doesn't become an oracle for probing signature validity.
pub fn validate_signature(
    url: &str,
    params: &BTreeMap<String, String>,
    auth_token: &str,
    provided_signature_b64: &str,
) -> bool {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    let Ok(provided_bytes) = STANDARD.decode(provided_signature_b64) else {
        return false;
    };
    let msg = signing_string(url, params);
    let Ok(mut mac) = HmacSha1::new_from_slice(auth_token.as_bytes()) else {
        // HMAC accepts any key length, so this is unreachable in practice —
        // kept as a graceful `false` rather than `.expect()` per the
        // no-unwrap-in-prod-paths convention.
        return false;
    };
    mac.update(msg.as_bytes());
    mac.verify_slice(&provided_bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test vector independently computed two ways outside this codebase
    /// (`openssl dgst -sha1 -hmac ... -binary | openssl base64`, and Python's
    /// `hmac`/`hashlib` standard library) — both agree on
    /// `driuZp25+vgewC8363biLUvmnmI=` for this input, so this is a real
    /// verification of the algorithm, not a self-fulfilling assertion.
    fn known_good() -> (
        &'static str,
        BTreeMap<String, String>,
        &'static str,
        &'static str,
    ) {
        let url = "https://example.com/hooks/sms/inbound";
        let mut params = BTreeMap::new();
        params.insert("From".to_string(), "+15551234567".to_string());
        params.insert("Body".to_string(), "Hello".to_string());
        let auth_token = "test-auth-token";
        let expected_sig = "driuZp25+vgewC8363biLUvmnmI=";
        (url, params, auth_token, expected_sig)
    }

    #[test]
    fn signing_string_sorts_params_and_concatenates_without_separators() {
        let (url, params, _, _) = known_good();
        assert_eq!(
            signing_string(url, &params),
            "https://example.com/hooks/sms/inboundBodyHelloFrom+15551234567"
        );
    }

    #[test]
    fn validate_signature_accepts_independently_computed_vector() {
        let (url, params, auth_token, expected_sig) = known_good();
        assert!(validate_signature(url, &params, auth_token, expected_sig));
    }

    #[test]
    fn validate_signature_rejects_wrong_auth_token() {
        let (url, params, _, expected_sig) = known_good();
        assert!(!validate_signature(
            url,
            &params,
            "wrong-token",
            expected_sig
        ));
    }

    #[test]
    fn validate_signature_rejects_tampered_params() {
        let (url, mut params, auth_token, expected_sig) = known_good();
        params.insert("Body".to_string(), "Goodbye".to_string());
        assert!(!validate_signature(url, &params, auth_token, expected_sig));
    }

    #[test]
    fn validate_signature_rejects_wrong_url() {
        let (_, params, auth_token, expected_sig) = known_good();
        assert!(!validate_signature(
            "https://example.com/hooks/sms/other",
            &params,
            auth_token,
            expected_sig
        ));
    }

    #[test]
    fn validate_signature_rejects_malformed_base64() {
        let (url, params, auth_token, _) = known_good();
        assert!(!validate_signature(
            url,
            &params,
            auth_token,
            "not-valid-base64!!"
        ));
    }

    #[test]
    fn validate_signature_rejects_empty_signature() {
        let (url, params, auth_token, _) = known_good();
        assert!(!validate_signature(url, &params, auth_token, ""));
    }
}
