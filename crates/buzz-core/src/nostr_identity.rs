//! NIP-19 identity helpers for human and configuration boundaries.
//!
//! Signed Nostr events, tags, filters, and database rows remain protocol-native
//! hex. These helpers are for the boundary immediately before or after those
//! protocol values: CLI arguments, environment variables, logs, and custom
//! JSON APIs.

use nostr::{FromBech32, PublicKey, SecretKey, ToBech32};
use thiserror::Error;

/// Encoding used by an accepted key input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInputEncoding {
    /// Canonical NIP-19 bech32 (`npub1…` or `nsec1…`).
    Nip19,
    /// Compatibility-only 64-character hexadecimal input.
    LegacyHex,
}

/// Error returned when a human-facing Nostr key cannot be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IdentityError {
    /// A public key was neither a valid npub nor a valid legacy hex key.
    #[error("expected a valid npub")]
    InvalidPublicKey,
    /// A private key was neither a valid nsec nor a valid legacy hex key.
    #[error("expected a valid nsec")]
    InvalidSecretKey,
    /// A validated public key could not be encoded as NIP-19.
    #[error("failed to encode npub")]
    PublicKeyEncoding,
    /// A validated secret key could not be encoded as NIP-19.
    #[error("failed to encode nsec")]
    SecretKeyEncoding,
}

/// Stable placeholder for a malformed public identity at a diagnostic boundary.
///
/// Logs and custom human-facing projections must never fall back to echoing the
/// original value, because that would reintroduce raw hex (or arbitrary input).
pub const INVALID_PUBLIC_KEY_DISPLAY: &str = "<invalid-pubkey>";

fn strip_nostr_scheme(input: &str) -> &str {
    input
        .get(..6)
        .filter(|prefix| prefix.eq_ignore_ascii_case("nostr:"))
        .map_or(input, |_| &input[6..])
}

fn has_mixed_ascii_case(input: &str) -> bool {
    input
        .chars()
        .any(|character| character.is_ascii_lowercase())
        && input
            .chars()
            .any(|character| character.is_ascii_uppercase())
}

/// Parse a canonical npub or a compatibility-only 64-character hex public key.
///
/// The returned [`PublicKey`] should be converted to hex only at a Nostr
/// protocol or database boundary.
pub fn parse_public_key_compat(
    input: &str,
) -> Result<(PublicKey, KeyInputEncoding), IdentityError> {
    let input = input.trim();
    let input = strip_nostr_scheme(input);
    if input
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("npub1"))
    {
        if has_mixed_ascii_case(input) {
            return Err(IdentityError::InvalidPublicKey);
        }
        let canonical = input.to_ascii_lowercase();
        let key =
            PublicKey::from_bech32(&canonical).map_err(|_| IdentityError::InvalidPublicKey)?;
        key.xonly().map_err(|_| IdentityError::InvalidPublicKey)?;
        return Ok((key, KeyInputEncoding::Nip19));
    }
    if input.len() == 64 && input.chars().all(|character| character.is_ascii_hexdigit()) {
        let key = PublicKey::from_hex(input).map_err(|_| IdentityError::InvalidPublicKey)?;
        key.xonly().map_err(|_| IdentityError::InvalidPublicKey)?;
        return Ok((key, KeyInputEncoding::LegacyHex));
    }
    Err(IdentityError::InvalidPublicKey)
}

/// Parse a canonical nsec or a compatibility-only 64-character hex secret key.
///
/// Errors deliberately never include the supplied secret.
pub fn parse_secret_key_compat(
    input: &str,
) -> Result<(SecretKey, KeyInputEncoding), IdentityError> {
    let input = input.trim();
    let input = strip_nostr_scheme(input);
    if input
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("nsec1"))
    {
        if has_mixed_ascii_case(input) {
            return Err(IdentityError::InvalidSecretKey);
        }
        let canonical = input.to_ascii_lowercase();
        return SecretKey::from_bech32(&canonical)
            .map(|key| (key, KeyInputEncoding::Nip19))
            .map_err(|_| IdentityError::InvalidSecretKey);
    }
    if input.len() == 64 && input.chars().all(|character| character.is_ascii_hexdigit()) {
        return SecretKey::from_hex(input)
            .map(|key| (key, KeyInputEncoding::LegacyHex))
            .map_err(|_| IdentityError::InvalidSecretKey);
    }
    Err(IdentityError::InvalidSecretKey)
}

/// Format a validated public key in canonical human-facing npub form.
pub fn public_key_to_npub(public_key: &PublicKey) -> Result<String, IdentityError> {
    public_key
        .to_bech32()
        .map_err(|_| IdentityError::PublicKeyEncoding)
}

/// Normalize an npub (or compatibility-only legacy hex public key) to npub.
pub fn canonical_npub(input: &str) -> Result<String, IdentityError> {
    let (public_key, _) = parse_public_key_compat(input)?;
    public_key_to_npub(&public_key)
}

/// Format a string public identity for logs without ever falling back to hex.
pub fn canonical_npub_or_invalid(input: &str) -> String {
    canonical_npub(input).unwrap_or_else(|_| INVALID_PUBLIC_KEY_DISPLAY.to_string())
}

/// Format a validated public identity for logs without ever falling back to hex.
pub fn public_key_to_npub_or_invalid(public_key: &PublicKey) -> String {
    public_key_to_npub(public_key).unwrap_or_else(|_| INVALID_PUBLIC_KEY_DISPLAY.to_string())
}

/// Format protocol/database public-key bytes as npub.
pub fn public_key_bytes_to_npub(bytes: &[u8]) -> Result<String, IdentityError> {
    let public_key = PublicKey::from_slice(bytes).map_err(|_| IdentityError::InvalidPublicKey)?;
    public_key
        .xonly()
        .map_err(|_| IdentityError::InvalidPublicKey)?;
    public_key_to_npub(&public_key)
}

/// Format protocol/database public-key bytes for logs without a raw fallback.
pub fn public_key_bytes_to_npub_or_invalid(bytes: &[u8]) -> String {
    public_key_bytes_to_npub(bytes).unwrap_or_else(|_| INVALID_PUBLIC_KEY_DISPLAY.to_string())
}

/// Export a validated secret key in canonical nsec form.
///
/// Call this only at an explicit secret export or protected persistence
/// boundary. An nsec is an encoding, not encryption, and must never be logged.
pub fn secret_key_to_nsec(secret_key: &SecretKey) -> Result<String, IdentityError> {
    secret_key
        .to_bech32()
        .map_err(|_| IdentityError::SecretKeyEncoding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[test]
    fn public_key_compat_normalizes_npub_and_hex() {
        let keys = Keys::generate();
        let npub = public_key_to_npub(&keys.public_key()).unwrap();
        let hex = keys.public_key().to_hex();

        let (from_npub, npub_encoding) = parse_public_key_compat(&npub).unwrap();
        let (from_hex, hex_encoding) = parse_public_key_compat(&hex.to_ascii_uppercase()).unwrap();

        assert_eq!(from_npub, keys.public_key());
        assert_eq!(from_hex, keys.public_key());
        assert_eq!(npub_encoding, KeyInputEncoding::Nip19);
        assert_eq!(hex_encoding, KeyInputEncoding::LegacyHex);
        assert_eq!(
            parse_public_key_compat(&format!("nostr:{npub}")).unwrap().0,
            keys.public_key()
        );
        assert_eq!(
            parse_public_key_compat(&npub.to_ascii_uppercase())
                .unwrap()
                .0,
            keys.public_key()
        );
        assert_eq!(
            parse_public_key_compat(&format!("NOSTR:{}", npub.to_ascii_uppercase()))
                .unwrap()
                .0,
            keys.public_key()
        );
        let mixed_case = format!("N{}", &npub[1..]);
        assert_eq!(
            parse_public_key_compat(&mixed_case),
            Err(IdentityError::InvalidPublicKey)
        );
    }

    #[test]
    fn public_key_compat_rejects_wrong_hrp_and_malformed_hex() {
        let keys = Keys::generate();
        let nsec = secret_key_to_nsec(keys.secret_key()).unwrap();
        assert_eq!(
            parse_public_key_compat(&nsec),
            Err(IdentityError::InvalidPublicKey)
        );
        assert_eq!(
            parse_public_key_compat(&"f".repeat(63)),
            Err(IdentityError::InvalidPublicKey)
        );

        // nostr's PublicKey parser decodes arbitrary 32-byte values; require
        // that the bytes are an actual secp256k1 x-only public key too.
        let invalid_point_hex = "ff".repeat(32);
        let invalid_point_npub = PublicKey::from_hex(&invalid_point_hex)
            .unwrap()
            .to_bech32()
            .unwrap();
        assert_eq!(
            parse_public_key_compat(&invalid_point_hex),
            Err(IdentityError::InvalidPublicKey)
        );
        assert_eq!(
            parse_public_key_compat(&invalid_point_npub),
            Err(IdentityError::InvalidPublicKey)
        );
    }

    #[test]
    fn canonical_display_normalizes_legacy_hex_and_never_echoes_invalid_input() {
        let keys = Keys::generate();
        let hex = keys.public_key().to_hex();
        let expected = public_key_to_npub(&keys.public_key()).unwrap();

        assert_eq!(canonical_npub(&hex).unwrap(), expected);
        assert_eq!(canonical_npub(&expected).unwrap(), expected);
        assert_eq!(canonical_npub_or_invalid(&hex), expected);
        assert_eq!(public_key_to_npub_or_invalid(&keys.public_key()), expected);
        assert_eq!(
            public_key_bytes_to_npub_or_invalid(keys.public_key().as_bytes()),
            expected
        );

        let invalid = "012345-not-a-public-key";
        let displayed = canonical_npub_or_invalid(invalid);
        assert_eq!(displayed, INVALID_PUBLIC_KEY_DISPLAY);
        assert!(!displayed.contains(invalid));
    }

    #[test]
    fn secret_key_compat_normalizes_nsec_and_hex_without_echoing_errors() {
        let keys = Keys::generate();
        let nsec = secret_key_to_nsec(keys.secret_key()).unwrap();
        let hex = keys.secret_key().to_secret_hex();

        let (from_nsec, nsec_encoding) = parse_secret_key_compat(&nsec).unwrap();
        let (from_hex, hex_encoding) = parse_secret_key_compat(&hex).unwrap();

        assert_eq!(from_nsec, *keys.secret_key());
        assert_eq!(from_hex, *keys.secret_key());
        assert_eq!(nsec_encoding, KeyInputEncoding::Nip19);
        assert_eq!(hex_encoding, KeyInputEncoding::LegacyHex);
        assert_eq!(
            parse_secret_key_compat(&nsec.to_ascii_uppercase())
                .unwrap()
                .0,
            *keys.secret_key()
        );
        assert_eq!(
            parse_secret_key_compat(&format!("NOSTR:{}", nsec.to_ascii_uppercase()))
                .unwrap()
                .0,
            *keys.secret_key()
        );
        let mixed_case = format!("N{}", &nsec[1..]);
        assert_eq!(
            parse_secret_key_compat(&mixed_case),
            Err(IdentityError::InvalidSecretKey)
        );

        let secret = "not-a-secret";
        let error = parse_secret_key_compat(secret).unwrap_err().to_string();
        assert!(!error.contains(secret));
        assert_eq!(error, "expected a valid nsec");
    }
}
