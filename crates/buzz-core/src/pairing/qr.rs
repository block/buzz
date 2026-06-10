//! NIP-AB QR code URI encoding and decoding.
//!
//! The QR code encodes a `nostrpair://` URI that the scanning device uses to
//! bootstrap a pairing session. The URI carries:
//!
//! - The source device's ephemeral public key (hex, 64 chars)
//! - A 32-byte session secret shared between both devices (hex, 64 chars)
//! - One or more relay URLs where the pairing messages will be exchanged
//! - A protocol version (`v=1`)
//!
//! # URI format
//!
//! ```text
//! nostrpair://<source_pubkey_hex>?secret=<session_secret_hex>&relay=<url-encoded-relay>&v=1
//! ```
//!
//! Multiple relays are represented as repeated `relay=` parameters:
//!
//! ```text
//! nostrpair://abc123...?secret=def456...&relay=wss%3A%2F%2Frelay1.example.com&relay=wss%3A%2F%2Frelay2.example.com&v=1
//! ```
//!
//! All characters unsafe in a query-parameter value (`:`, `/`, `?`, `#`,
//! `&`, `=`, `%`, and space) are percent-encoded.

use nostr::PublicKey;
use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use zeroize::Zeroize;

use super::PairingError;

// ── Data types ────────────────────────────────────────────────────────────────

/// Data encoded in the QR code displayed by the source device.
#[derive(Debug, Clone)]
pub struct QrPayload {
    /// The source device's ephemeral public key.
    pub source_pubkey: PublicKey,
    /// 32-byte session secret shared between both devices.
    ///
    /// This is generated fresh for each pairing session and never reused.
    pub session_secret: [u8; 32],
    /// One or more relay URLs where pairing messages will be exchanged.
    pub relays: Vec<String>,
    /// Protocol version. Always `1` for this implementation.
    ///
    /// Encoded as `v=1` in the URI. Absent in legacy URIs; defaults to `1`
    /// on decode for backward compatibility. Values > 1 are rejected.
    pub version: u32,
}

/// Zero the session secret on drop using `zeroize` to prevent dead-store
/// elimination by the compiler (plain `fill(0)` can be optimized away).
impl Drop for QrPayload {
    fn drop(&mut self) {
        self.session_secret.zeroize();
    }
}

// ── Encoding ──────────────────────────────────────────────────────────────────

/// Encode a [`QrPayload`] as a `nostrpair://` URI.
///
/// Relay URLs are percent-encoded (`:` → `%3A`, `/` → `%2F`) so they can
/// safely appear as query parameter values.
///
/// # Example
///
/// ```
/// use sprout_core::pairing::qr::{QrPayload, encode_qr};
/// use nostr::Keys;
///
/// let keys = Keys::generate();
/// let payload = QrPayload {
///     source_pubkey: keys.public_key(),
///     session_secret: [0u8; 32],
///     relays: vec!["wss://relay.example.com".to_string()],
///     version: 1,
/// };
/// let uri = encode_qr(&payload);
/// assert!(uri.starts_with("nostrpair://"));
/// ```
pub fn encode_qr(payload: &QrPayload) -> String {
    let pubkey_hex = payload.source_pubkey.to_hex();
    let secret_hex = hex::encode(payload.session_secret);

    let mut uri = format!("nostrpair://{}?secret={}", pubkey_hex, secret_hex);

    for relay in &payload.relays {
        uri.push_str("&relay=");
        uri.push_str(&url_encode(relay));
    }

    uri.push_str("&v=1");

    uri
}

// ── Decoding ──────────────────────────────────────────────────────────────────

/// Decode a `nostrpair://` URI into a [`QrPayload`].
///
/// # Errors
///
/// Returns [`PairingError::InvalidQr`] if:
/// - The scheme is not `nostrpair`
/// - The public key is not a valid 64-char hex string
/// - The `secret` parameter is missing or not a valid 64-char hex string
/// - No `relay` parameters are present
pub fn decode_qr(uri: &str) -> Result<QrPayload, PairingError> {
    // NIP-AB §QR Code Format: URI length MUST NOT exceed 2048 characters.
    if uri.len() > 2048 {
        return Err(PairingError::InvalidQr(format!(
            "URI exceeds 2048-character limit ({} chars)",
            uri.len()
        )));
    }

    // Split scheme from the rest.
    let rest = uri
        .strip_prefix("nostrpair://")
        .ok_or_else(|| PairingError::InvalidQr("URI must start with nostrpair://".into()))?;

    // Split pubkey from query string.
    let (pubkey_hex, query) = match rest.split_once('?') {
        Some((pk, q)) => (pk, q),
        None => {
            return Err(PairingError::InvalidQr(
                "missing query string (expected ?secret=…&relay=…)".into(),
            ))
        }
    };

    // Validate pubkey: must be exactly 64 lowercase hex chars (NIP-AB §QR Code Format).
    if pubkey_hex.len() != 64 || !pubkey_hex.chars().all(is_lowercase_hex) {
        return Err(PairingError::InvalidQr(format!(
            "pubkey must be 64 lowercase hex chars, got {:?}",
            pubkey_hex
        )));
    }
    let source_pubkey = PublicKey::from_hex(pubkey_hex)
        .map_err(|e| PairingError::InvalidQr(format!("invalid pubkey: {e}")))?;

    // Parse query parameters.
    let mut secret_hex: Option<&str> = None;
    let mut relays: Vec<String> = Vec::new();
    let mut version: Option<u32> = None;

    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            match key {
                "secret" => secret_hex = Some(value),
                "relay" => relays.push(url_decode(value)),
                "v" => version = value.parse::<u32>().ok(),
                _ => {} // ignore unknown params
            }
        }
    }

    // Default to version 1 if absent (backward compat); reject unsupported versions.
    let version = version.unwrap_or(1);
    if version != 1 {
        return Err(PairingError::InvalidQr(format!(
            "unsupported protocol version {version}, expected 1"
        )));
    }

    // Validate secret: must be exactly 64 hex chars.
    let secret_str = secret_hex
        .ok_or_else(|| PairingError::InvalidQr("missing 'secret' query parameter".into()))?;

    if secret_str.len() != 64 || !secret_str.chars().all(is_lowercase_hex) {
        return Err(PairingError::InvalidQr(format!(
            "secret must be 64 lowercase hex chars, got {:?}",
            secret_str
        )));
    }
    let secret_bytes = hex::decode(secret_str)
        .map_err(|e| PairingError::InvalidQr(format!("invalid secret hex: {e}")))?;
    let session_secret: [u8; 32] = secret_bytes
        .try_into()
        .map_err(|_| PairingError::InvalidQr("secret must be exactly 32 bytes".into()))?;

    // NIP-AB §Test Vectors: all-zeros session_secret MUST be rejected.
    if session_secret == [0u8; 32] {
        return Err(PairingError::InvalidQr(
            "session_secret must not be all zeros".into(),
        ));
    }

    // Must have at least one relay.
    if relays.is_empty() {
        return Err(PairingError::InvalidQr(
            "at least one 'relay' query parameter is required".into(),
        ));
    }

    // Validate relay URLs — parse fully and require WebSocket scheme + host.
    // Prefix-matching alone would accept malformed URLs that crash downstream.
    for relay in &relays {
        let parsed = url::Url::parse(relay)
            .map_err(|e| PairingError::InvalidQr(format!("invalid relay URL {:?}: {e}", relay)))?;
        match parsed.scheme() {
            "wss" | "ws" => {}
            other => {
                return Err(PairingError::InvalidQr(format!(
                    "relay URL must use wss:// or ws:// scheme, got {:?}",
                    other
                )));
            }
        }
        if parsed.host().is_none() {
            return Err(PairingError::InvalidQr(format!(
                "relay URL has no host: {:?}",
                relay
            )));
        }
    }

    Ok(QrPayload {
        source_pubkey,
        session_secret,
        relays,
        version,
    })
}

// ── URL encoding helpers ──────────────────────────────────────────────────────

/// Percent-encode a relay URL for use as a query parameter value.
///
/// Uses `percent-encoding` crate's `NON_ALPHANUMERIC` set, which encodes
/// everything except ASCII alphanumerics. This is a strict superset of the
/// characters unsafe in query-parameter values (`:`, `/`, `?`, `#`, `&`,
/// `=`, `%`, space) — safe by construction.
fn url_encode(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// Percent-decode a query parameter value.
///
/// Falls back to lossy UTF-8 conversion for non-UTF-8 sequences (which
/// shouldn't appear in valid relay URLs, but we handle it safely).
fn url_decode(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// NIP-AB §QR Code Format requires lowercase hex only (`0-9`, `a-f`).
fn is_lowercase_hex(c: char) -> bool {
    c.is_ascii_digit() || ('a'..='f').contains(&c)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn make_payload(relays: Vec<String>) -> QrPayload {
        let keys = Keys::generate();
        QrPayload {
            source_pubkey: keys.public_key(),
            session_secret: [0xab; 32],
            relays,
            version: 1,
        }
    }

    // 1. Round-trip encode/decode
    #[test]
    fn round_trip_single_relay() {
        let original = make_payload(vec!["wss://relay.example.com".to_string()]);
        let uri = encode_qr(&original);
        let decoded = decode_qr(&uri).expect("decode should succeed");

        assert_eq!(original.source_pubkey, decoded.source_pubkey);
        assert_eq!(original.session_secret, decoded.session_secret);
        assert_eq!(original.relays, decoded.relays);
    }

    // 7. Handle multiple relays
    #[test]
    fn round_trip_multiple_relays() {
        let original = make_payload(vec![
            "wss://relay1.example.com".to_string(),
            "wss://relay2.example.com".to_string(),
            "wss://relay3.example.com".to_string(),
        ]);
        let uri = encode_qr(&original);
        let decoded = decode_qr(&uri).expect("decode should succeed");

        assert_eq!(decoded.relays.len(), 3);
        assert_eq!(decoded.relays, original.relays);
    }

    // 8. Handle URL-encoded relay URLs
    #[test]
    fn url_encoding_round_trip() {
        let relay = "wss://relay.example.com/path";
        let encoded = url_encode(relay);
        // NON_ALPHANUMERIC encodes dots too — stricter than necessary but safe.
        assert_eq!(encoded, "wss%3A%2F%2Frelay%2Eexample%2Ecom%2Fpath");
        let decoded = url_decode(&encoded);
        assert_eq!(decoded, relay);
    }

    #[test]
    fn round_trip_relay_with_path() {
        let original = make_payload(vec!["wss://relay.example.com/nostr".to_string()]);
        let uri = encode_qr(&original);
        let decoded = decode_qr(&uri).expect("decode should succeed");
        assert_eq!(decoded.relays[0], "wss://relay.example.com/nostr");
    }

    // 2. Reject missing scheme
    #[test]
    fn reject_missing_scheme() {
        let err = decode_qr("https://relay.example.com").unwrap_err();
        assert!(
            matches!(err, PairingError::InvalidQr(_)),
            "expected InvalidQr, got {err:?}"
        );
    }

    #[test]
    fn reject_wrong_scheme() {
        let err = decode_qr("nostr://abc").unwrap_err();
        assert!(matches!(err, PairingError::InvalidQr(_)));
    }

    // 3. Reject missing secret
    #[test]
    fn reject_missing_secret() {
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let relay_encoded = url_encode("wss://relay.example.com");
        let uri = format!("nostrpair://{}?relay={}", pubkey, relay_encoded);
        let err = decode_qr(&uri).unwrap_err();
        assert!(matches!(err, PairingError::InvalidQr(_)));
    }

    // 4. Reject missing relay
    #[test]
    fn reject_missing_relay() {
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let secret = hex::encode([0xab; 32]);
        let uri = format!("nostrpair://{}?secret={}", pubkey, secret);
        let err = decode_qr(&uri).unwrap_err();
        assert!(matches!(err, PairingError::InvalidQr(_)));
    }

    // 5. Reject invalid hex in pubkey
    #[test]
    fn reject_invalid_pubkey_hex() {
        let bad_pubkey = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"; // 64 chars, not hex
        let secret = hex::encode([0xab; 32]);
        let relay_encoded = url_encode("wss://relay.example.com");
        let uri = format!(
            "nostrpair://{}?secret={}&relay={}",
            bad_pubkey, secret, relay_encoded
        );
        let err = decode_qr(&uri).unwrap_err();
        assert!(matches!(err, PairingError::InvalidQr(_)));
    }

    // 6. Reject invalid hex in secret
    #[test]
    fn reject_invalid_secret_hex() {
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let bad_secret = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"; // 64 chars, not hex
        let relay_encoded = url_encode("wss://relay.example.com");
        let uri = format!(
            "nostrpair://{}?secret={}&relay={}",
            pubkey, bad_secret, relay_encoded
        );
        let err = decode_qr(&uri).unwrap_err();
        assert!(matches!(err, PairingError::InvalidQr(_)));
    }

    #[test]
    fn reject_short_pubkey() {
        let secret = hex::encode([0xab; 32]);
        let relay_encoded = url_encode("wss://relay.example.com");
        let uri = format!(
            "nostrpair://abc123?secret={}&relay={}",
            secret, relay_encoded
        );
        let err = decode_qr(&uri).unwrap_err();
        assert!(matches!(err, PairingError::InvalidQr(_)));
    }

    #[test]
    fn reject_short_secret() {
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let relay_encoded = url_encode("wss://relay.example.com");
        let uri = format!(
            "nostrpair://{}?secret=abc123&relay={}",
            pubkey, relay_encoded
        );
        let err = decode_qr(&uri).unwrap_err();
        assert!(matches!(err, PairingError::InvalidQr(_)));
    }

    #[test]
    fn reject_missing_query_string() {
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let uri = format!("nostrpair://{}", pubkey);
        let err = decode_qr(&uri).unwrap_err();
        assert!(matches!(err, PairingError::InvalidQr(_)));
    }

    #[test]
    fn reject_non_websocket_relay_scheme() {
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let secret = hex::encode([0xab; 32]);
        // http:// is not a valid relay scheme
        let relay_encoded = url_encode("https://evil.example.com");
        let uri = format!(
            "nostrpair://{}?secret={}&relay={}",
            pubkey, secret, relay_encoded
        );
        let err = decode_qr(&uri).unwrap_err();
        assert!(matches!(err, PairingError::InvalidQr(_)));
    }

    #[test]
    fn accept_ws_and_wss_relay_schemes() {
        let payload_wss = make_payload(vec!["wss://relay.example.com".to_string()]);
        let uri_wss = encode_qr(&payload_wss);
        assert!(decode_qr(&uri_wss).is_ok(), "wss:// should be accepted");

        let payload_ws = make_payload(vec!["ws://relay.example.com".to_string()]);
        let uri_ws = encode_qr(&payload_ws);
        assert!(decode_qr(&uri_ws).is_ok(), "ws:// should be accepted");
    }

    #[test]
    fn reject_relay_with_no_scheme() {
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let secret = hex::encode([0xab; 32]);
        let relay_encoded = url_encode("relay.example.com");
        let uri = format!(
            "nostrpair://{}?secret={}&relay={}",
            pubkey, secret, relay_encoded
        );
        let err = decode_qr(&uri).unwrap_err();
        assert!(matches!(err, PairingError::InvalidQr(_)));
    }

    #[test]
    fn uri_contains_scheme_and_pubkey() {
        let payload = make_payload(vec!["wss://relay.example.com".to_string()]);
        let uri = encode_qr(&payload);
        assert!(uri.starts_with("nostrpair://"));
        assert!(uri.contains(&payload.source_pubkey.to_hex()));
        assert!(uri.contains("secret="));
        assert!(uri.contains("relay="));
    }

    #[test]
    fn url_decode_case_insensitive() {
        // %3a and %2f (lowercase) should also decode
        assert_eq!(
            url_decode("wss%3a%2f%2frelay.example.com"),
            "wss://relay.example.com"
        );
    }

    #[test]
    fn round_trip_relay_with_query_params() {
        // Relay URL with query parameters containing &, =, and ?
        let original = make_payload(vec![
            "wss://relay.example.com/path?token=abc&flag=1".to_string()
        ]);
        let uri = encode_qr(&original);
        let decoded = decode_qr(&uri).expect("decode should succeed");
        assert_eq!(
            decoded.relays[0],
            "wss://relay.example.com/path?token=abc&flag=1"
        );
    }

    #[test]
    fn round_trip_relay_with_percent_and_hash() {
        let original = make_payload(vec!["wss://relay.example.com/path#frag%20ment".to_string()]);
        let uri = encode_qr(&original);
        let decoded = decode_qr(&uri).expect("decode should succeed");
        assert_eq!(
            decoded.relays[0],
            "wss://relay.example.com/path#frag%20ment"
        );
    }

    #[test]
    fn url_encode_reserved_chars() {
        let encoded = url_encode("wss://relay.com/path?a=1&b=2#frag");
        assert!(!encoded.contains('&'), "& must be encoded");
        assert!(!encoded.contains('='), "= must be encoded");
        assert!(!encoded.contains('?'), "? must be encoded");
        assert!(!encoded.contains('#'), "# must be encoded");
        let decoded = url_decode(&encoded);
        assert_eq!(decoded, "wss://relay.com/path?a=1&b=2#frag");
    }

    // Version field tests

    #[test]
    fn round_trip_with_version() {
        let original = make_payload(vec!["wss://relay.example.com".to_string()]);
        let uri = encode_qr(&original);
        assert!(uri.contains("&v=1"), "URI must contain &v=1: {uri}");
        let decoded = decode_qr(&uri).expect("decode should succeed");
        assert_eq!(decoded.version, 1);
        assert_eq!(original.source_pubkey, decoded.source_pubkey);
        assert_eq!(original.session_secret, decoded.session_secret);
        assert_eq!(original.relays, decoded.relays);
    }

    #[test]
    fn reject_unsupported_version() {
        let payload = make_payload(vec!["wss://relay.example.com".to_string()]);
        // Build a URI with v=2 manually.
        let uri = encode_qr(&payload).replace("&v=1", "&v=2");
        let err = decode_qr(&uri).unwrap_err();
        assert!(
            matches!(err, PairingError::InvalidQr(ref msg) if msg.contains("unsupported protocol version 2")),
            "expected unsupported version error, got {err:?}"
        );
    }

    #[test]
    fn default_version_when_absent() {
        // Strip the &v=1 from a well-formed URI to simulate a legacy QR code.
        let payload = make_payload(vec!["wss://relay.example.com".to_string()]);
        let uri = encode_qr(&payload).replace("&v=1", "");
        let decoded = decode_qr(&uri).expect("legacy URI without v= should decode as version 1");
        assert_eq!(decoded.version, 1, "missing v= should default to version 1");
    }

    // ── All-zeros session_secret rejection ────────────────────────────────

    #[test]
    fn reject_all_zeros_session_secret() {
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let zero_secret = "00".repeat(32); // 64 hex chars, all zeros
        let relay_encoded = url_encode("wss://relay.example.com");
        let uri = format!(
            "nostrpair://{}?secret={}&relay={}&v=1",
            pubkey, zero_secret, relay_encoded
        );
        let err = decode_qr(&uri).unwrap_err();
        assert!(
            matches!(err, PairingError::InvalidQr(ref msg) if msg.contains("all zeros")),
            "expected all-zeros rejection, got {err:?}"
        );
    }

    // ── Lowercase hex enforcement ─────────────────────────────────────────

    #[test]
    fn reject_uppercase_hex_in_pubkey() {
        let keys = Keys::generate();
        // Force uppercase in the pubkey hex
        let pubkey_upper = keys.public_key().to_hex().to_uppercase();
        let secret = hex::encode([0xab; 32]);
        let relay_encoded = url_encode("wss://relay.example.com");
        let uri = format!(
            "nostrpair://{}?secret={}&relay={}&v=1",
            pubkey_upper, secret, relay_encoded
        );
        let err = decode_qr(&uri).unwrap_err();
        assert!(
            matches!(err, PairingError::InvalidQr(ref msg) if msg.contains("lowercase")),
            "expected lowercase rejection for pubkey, got {err:?}"
        );
    }

    #[test]
    fn reject_uppercase_hex_in_secret() {
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let secret_upper = hex::encode([0xab; 32]).to_uppercase();
        let relay_encoded = url_encode("wss://relay.example.com");
        let uri = format!(
            "nostrpair://{}?secret={}&relay={}&v=1",
            pubkey, secret_upper, relay_encoded
        );
        let err = decode_qr(&uri).unwrap_err();
        assert!(
            matches!(err, PairingError::InvalidQr(ref msg) if msg.contains("lowercase")),
            "expected lowercase rejection for secret, got {err:?}"
        );
    }
}
