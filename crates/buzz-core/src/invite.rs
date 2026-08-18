//! Shared pure contracts for relay invite links.
//!
//! The relay transport and database persistence layers both depend on
//! `buzz-core`. Protocol constants and deterministic v2 code operations live
//! here so neither layer becomes the accidental source of truth.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

/// Minimum invite lifetime accepted by the mint API: 60 seconds.
pub const MIN_INVITE_TTL_SECS: u64 = 60;

/// Default invite lifetime when the mint request omits `ttl_secs`: 72 hours.
pub const DEFAULT_INVITE_TTL_SECS: u64 = 72 * 60 * 60;

/// Maximum invite lifetime accepted by the mint API: 30 days.
pub const MAX_INVITE_TTL_SECS: u64 = 30 * 24 * 60 * 60;

/// Maximum supported `max_uses` value. Matches the database constraint.
pub const MAX_INVITE_USES: i32 = 10_000;

/// Prefix that distinguishes v2 opaque database-backed codes from v1 tokens.
pub const V2_PREFIX: &str = "v2.";

/// Number of random bytes encoded in a v2 invite code.
pub const V2_SECRET_LEN: usize = 32;

/// A malformed v2 opaque invite code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidV2InviteCode;

/// Build a canonical v2 code from its random secret.
pub fn encode_v2_code(secret: &[u8; V2_SECRET_LEN]) -> String {
    format!("{V2_PREFIX}{}", URL_SAFE_NO_PAD.encode(secret))
}

/// Validate the canonical v2 opaque-code shape without consulting storage.
///
/// A valid code is exactly `v2.` followed by the unpadded base64url encoding
/// of a 32-byte secret. The decode/re-encode comparison rejects aliases such
/// as padded or otherwise non-canonical encodings.
pub fn validate_v2_code(code: &str) -> Result<(), InvalidV2InviteCode> {
    let encoded = code.strip_prefix(V2_PREFIX).ok_or(InvalidV2InviteCode)?;
    let secret = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| InvalidV2InviteCode)?;
    if secret.len() != V2_SECRET_LEN || URL_SAFE_NO_PAD.encode(&secret) != encoded {
        return Err(InvalidV2InviteCode);
    }
    Ok(())
}

/// Hash the complete v2 code to the digest persisted by the database.
pub fn hash_v2_code(code: &str) -> [u8; 32] {
    Sha256::digest(code.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_code_round_trip_is_canonical() {
        let secret = [7_u8; V2_SECRET_LEN];
        let code = encode_v2_code(&secret);

        assert_eq!(validate_v2_code(&code), Ok(()));
        assert_eq!(
            code,
            format!("{V2_PREFIX}{}", URL_SAFE_NO_PAD.encode(secret))
        );
    }

    #[test]
    fn v2_validation_rejects_malformed_and_noncanonical_codes() {
        let valid = encode_v2_code(&[7_u8; V2_SECRET_LEN]);
        let short = format!(
            "{V2_PREFIX}{}",
            URL_SAFE_NO_PAD.encode([7_u8; V2_SECRET_LEN - 1])
        );

        for malformed in [
            "v2.",
            "v2.not-base64!",
            short.as_str(),
            &format!("{valid}="),
        ] {
            assert_eq!(
                validate_v2_code(malformed),
                Err(InvalidV2InviteCode),
                "accepted malformed v2 code: {malformed}"
            );
        }
    }

    #[test]
    fn v2_hash_covers_the_complete_code() {
        let code = encode_v2_code(&[7_u8; V2_SECRET_LEN]);

        let expected: [u8; 32] = Sha256::digest(code.as_bytes()).into();

        assert_eq!(hash_v2_code(&code), expected);
        assert_ne!(
            hash_v2_code(&code),
            hash_v2_code(code.trim_start_matches(V2_PREFIX))
        );
    }
}
