//! Random delivery capabilities. Only their SHA-256 digest is stored.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("invalid endpoint grant")]
pub struct GrantError;

/// Issue a fresh 256-bit capability and its storage digest.
pub fn issue() -> Result<(String, [u8; 32]), GrantError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| GrantError)?;
    let grant = format!("g2_{}", URL_SAFE_NO_PAD.encode(bytes));
    let hash = hash(&grant)?;
    Ok((grant, hash))
}

/// Validate the canonical envelope and hash the entire presented capability.
pub fn hash(grant: &str) -> Result<[u8; 32], GrantError> {
    let encoded = grant.strip_prefix("g2_").ok_or(GrantError)?;
    if encoded.len() != 43
        || URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| GrantError)?
            .len()
            != 32
    {
        return Err(GrantError);
    }
    Ok(Sha256::digest(grant.as_bytes()).into())
}

/// Use a digest prefix for indexed lookup; authorization compares all 256 bits.
pub fn id(hash: &[u8; 32]) -> uuid::Uuid {
    let mut id = [0u8; 16];
    id.copy_from_slice(&hash[..16]);
    uuid::Uuid::from_bytes(id)
}

#[cfg(test)]
pub(crate) fn test_hash(id: uuid::Uuid) -> [u8; 32] {
    let mut hash = [0u8; 32];
    hash[..16].copy_from_slice(id.as_bytes());
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_are_random_canonical_and_hashed_in_full() {
        let (first, digest) = issue().unwrap();
        let (second, _) = issue().unwrap();
        assert_ne!(first, second);
        assert_eq!(first.len(), 46);
        assert_eq!(hash(&first).unwrap(), digest);
        assert_eq!(
            digest.as_slice(),
            Sha256::digest(first.as_bytes()).as_slice()
        );
        for invalid in [
            "",
            "old.AAAA",
            "g2_AAAA",
            &format!("{first}="),
            &"g2_A".repeat(1000),
        ] {
            assert!(hash(invalid).is_err());
        }
        let mut collision = digest;
        collision[31] ^= 1;
        assert_eq!(id(&collision), id(&digest));
        assert_ne!(collision, digest);
    }
}
