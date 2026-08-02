//! Explicit development-only App Attest sentinel verification.
//!
//! This module is absent unless the non-default `dev-app-attest-bypass` Cargo
//! feature is enabled. Runtime configuration adds a second, independent gate.

use crate::app_attest::{AppAttestError, VerifiedAssertion, VerifiedAttestation};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use sha2::{Digest, Sha256};

const ATTESTATION_PREFIX: &[u8] = b"buzz-dev-app-attest-v1:";
const ASSERTION_SENTINEL: &[u8] = b"buzz-dev-app-assertion-v1";
const PUBLIC_KEY_PREFIX: &[u8] = b"buzz-dev-app-attest-public-key-v1:";
const NONCE_BYTES: usize = 32;

pub fn assertion_sentinel() -> String {
    STANDARD.encode(ASSERTION_SENTINEL)
}

pub fn verify_attestation(
    attestation_b64: &str,
    key_id_b64: &str,
    client_data: &[u8],
) -> Result<VerifiedAttestation, AppAttestError> {
    let attestation = STANDARD
        .decode(attestation_b64)
        .map_err(|_| AppAttestError::Invalid)?;
    let supplied_key_id = STANDARD
        .decode(key_id_b64)
        .map_err(|_| AppAttestError::Invalid)?;
    let expected_key_id = Sha256::digest(&attestation);
    if !attestation.starts_with(ATTESTATION_PREFIX)
        || attestation.len() != ATTESTATION_PREFIX.len() + NONCE_BYTES
        || supplied_key_id.as_slice() != expected_key_id.as_slice()
        || client_data.is_empty()
    {
        return Err(AppAttestError::Invalid);
    }
    let mut public_key = PUBLIC_KEY_PREFIX.to_vec();
    public_key.extend_from_slice(&expected_key_id);
    Ok(VerifiedAttestation {
        key_id: expected_key_id.to_vec(),
        public_key,
    })
}

pub fn verify_assertion(
    assertion_b64: &str,
    client_data: &[u8],
    public_key: &[u8],
    previous_counter: u32,
    challenge: &str,
    stored_challenge: &str,
) -> Result<VerifiedAssertion, AppAttestError> {
    let assertion = STANDARD
        .decode(assertion_b64)
        .map_err(|_| AppAttestError::Invalid)?;
    if assertion != ASSERTION_SENTINEL
        || client_data.is_empty()
        || !public_key.starts_with(PUBLIC_KEY_PREFIX)
        || public_key.len() != PUBLIC_KEY_PREFIX.len() + 32
        || challenge.is_empty()
        || challenge != stored_challenge
    {
        return Err(AppAttestError::Invalid);
    }
    Ok(VerifiedAssertion {
        counter: previous_counter
            .checked_add(1)
            .ok_or(AppAttestError::Invalid)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attestation(nonce: u8) -> (String, String) {
        let mut sentinel = ATTESTATION_PREFIX.to_vec();
        sentinel.extend_from_slice(&[nonce; NONCE_BYTES]);
        let key_id = Sha256::digest(&sentinel);
        (STANDARD.encode(sentinel), STANDARD.encode(key_id))
    }

    #[test]
    fn versioned_attestation_derives_a_unique_bound_key_id() {
        let (attestation_a, key_id_a) = attestation(1);
        let verified_a = verify_attestation(
            &attestation_a,
            &key_id_a,
            b"canonical enrollment transcript",
        )
        .unwrap();
        assert_eq!(verified_a.key_id.len(), 32);
        assert!(verified_a.public_key.starts_with(PUBLIC_KEY_PREFIX));

        let (attestation_b, key_id_b) = attestation(2);
        let verified_b = verify_attestation(
            &attestation_b,
            &key_id_b,
            b"canonical enrollment transcript",
        )
        .unwrap();
        assert_ne!(verified_a.key_id, verified_b.key_id);

        for (attestation, key, transcript) in [
            ("bad".to_owned(), key_id_a.clone(), b"transcript".as_slice()),
            (attestation_a.clone(), key_id_b, b"transcript"),
            (attestation_a, key_id_a, b""),
        ] {
            assert!(verify_attestation(&attestation, &key, transcript).is_err());
        }
    }

    #[test]
    fn bad_attestation_sentinel_is_rejected_before_good_sentinel_is_accepted() {
        let (good_attestation, key_id) = attestation(1);
        let bad_attestation = STANDARD.encode(b"buzz-dev-app-attest-v1:bad");

        assert!(verify_attestation(&bad_attestation, &key_id, b"transcript").is_err());
        assert!(verify_attestation(&good_attestation, &key_id, b"transcript").is_ok());
    }

    #[test]
    fn exact_assertion_sentinel_and_stored_dev_marker_advance_counter() {
        let (attestation, key_id) = attestation(1);
        let public_key = verify_attestation(&attestation, &key_id, b"transcript")
            .unwrap()
            .public_key;
        let verified = verify_assertion(
            &assertion_sentinel(),
            b"canonical assertion transcript",
            &public_key,
            7,
            "challenge",
            "challenge",
        )
        .unwrap();
        assert_eq!(verified.counter, 8);

        assert!(verify_assertion(
            "bad",
            b"canonical assertion transcript",
            &public_key,
            7,
            "challenge",
            "challenge",
        )
        .is_err());
        assert!(verify_assertion(
            &assertion_sentinel(),
            b"canonical assertion transcript",
            b"not-the-development-marker",
            7,
            "challenge",
            "challenge",
        )
        .is_err());
        assert!(verify_assertion(
            &assertion_sentinel(),
            b"canonical assertion transcript",
            &public_key,
            u32::MAX,
            "challenge",
            "challenge",
        )
        .is_err());
    }
}
