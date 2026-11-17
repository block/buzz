//! Android installation authentication: the Key Attestation seam used at
//! enrollment, and verification of the installation calls the attested Keystore
//! key signs afterward.
//!
//! Enrollment hands a certificate chain to an [`AndroidAttestationVerifier`].
//! The production verifier (Key Attestation chain validation) is a separate
//! change; until it exists the gateway starts with [`PendingAttestationVerifier`],
//! which rejects every chain. The test-only verifier below is compiled only
//! into test builds, so no configuration can select it.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
use thiserror::Error;

use crate::model::MAX_ANDROID_SIGNATURE_BYTES;

/// The attested key an accepted chain proves. `public_key` is the uncompressed
/// SEC1 encoding of an ECDSA P-256 key generated in secure hardware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAndroidKey {
    pub public_key: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum AndroidAttestationError {
    #[error("invalid Android attestation")]
    Invalid,
}

/// Verifies an Android Key Attestation chain (DER certificates, leaf first).
///
/// `client_data` is the exact enrollment transcript. An implementation must
/// accept the chain only when the attestation challenge equals the SHA-256 of
/// `client_data`, as App Attest's client data binds the transcript on iOS.
pub trait AndroidAttestationVerifier: Send + Sync {
    fn verify(
        &self,
        chain: &[Vec<u8>],
        client_data: &[u8],
    ) -> Result<VerifiedAndroidKey, AndroidAttestationError>;
}

/// Rejects every chain. It stands in for the real verifier so that a gateway
/// configured for Android fails closed until Key Attestation verification ships.
pub struct PendingAttestationVerifier;

impl AndroidAttestationVerifier for PendingAttestationVerifier {
    fn verify(
        &self,
        _: &[Vec<u8>],
        _: &[u8],
    ) -> Result<VerifiedAndroidKey, AndroidAttestationError> {
        tracing::warn!(
            operation = "attestation",
            reason = "unavailable",
            "Android Key Attestation verification is not implemented in this build"
        );
        Err(AndroidAttestationError::Invalid)
    }
}

/// Verify an installation call signed by the attested Keystore key: a base64
/// DER ECDSA P-256 signature over the same transcript bytes iOS signs. The
/// signature has no counter; replay protection is the single-use challenge
/// inside the transcript, which the caller consumes after this succeeds.
pub fn verify_signature(
    public_key: &[u8],
    transcript: &[u8],
    signature_b64: &str,
) -> Result<(), AndroidAttestationError> {
    if signature_b64.len() > MAX_ANDROID_SIGNATURE_BYTES.div_ceil(3) * 4 {
        return Err(AndroidAttestationError::Invalid);
    }
    let der = STANDARD
        .decode(signature_b64)
        .map_err(|_| AndroidAttestationError::Invalid)?;
    let signature = Signature::from_der(&der).map_err(|_| AndroidAttestationError::Invalid)?;
    // Keystore does not normalize `s`, so accept either form.
    let signature = signature.normalize_s();
    VerifyingKey::from_sec1_bytes(public_key)
        .map_err(|_| AndroidAttestationError::Invalid)?
        .verify(transcript, &signature)
        .map_err(|_| AndroidAttestationError::Invalid)
}

/// Test-only attestation verifier. Its "chain" is `[public key, SHA-256 of the
/// transcript]`: enough to prove that enrollment binds the transcript and
/// stores the proven key, without any Google certificate machinery.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use sha2::{Digest, Sha256};

    pub(crate) struct TestAttestationVerifier;

    impl AndroidAttestationVerifier for TestAttestationVerifier {
        fn verify(
            &self,
            chain: &[Vec<u8>],
            client_data: &[u8],
        ) -> Result<VerifiedAndroidKey, AndroidAttestationError> {
            match chain {
                [public_key, challenge]
                    if challenge.as_slice() == Sha256::digest(client_data).as_slice() =>
                {
                    Ok(VerifiedAndroidKey {
                        public_key: public_key.clone(),
                    })
                }
                _ => Err(AndroidAttestationError::Invalid),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{signature::Signer, SigningKey};

    const TRANSCRIPT: &[u8] = b"buzz.push.delegate.v1\n{\"challenge\":\"test-only\"}";

    fn key() -> SigningKey {
        SigningKey::from_slice(&[7; 32]).unwrap()
    }
    fn public_key() -> Vec<u8> {
        key().verifying_key().to_sec1_point(false).as_ref().to_vec()
    }
    fn encode(signature: &Signature) -> String {
        STANDARD.encode(signature.to_der().as_bytes())
    }

    #[test]
    fn keystore_signatures_verify_in_either_s_form_and_nothing_else_does() {
        let low: Signature = key().sign(TRANSCRIPT);
        let low = low.normalize_s();
        // Keystore does not normalize `s`; the high form is the same signature.
        let high = Signature::from_scalars(
            *low.r(),
            p256::NonZeroScalar::new(-*low.s().as_ref()).unwrap(),
        )
        .unwrap();
        assert_ne!(low, high);
        for signature in [&low, &high] {
            verify_signature(&public_key(), TRANSCRIPT, &encode(signature)).unwrap();
        }
        let good = encode(&low);
        for (public_key, transcript, signature) in [
            (public_key(), b"altered".as_slice(), good.clone()),
            (
                SigningKey::from_slice(&[9; 32])
                    .unwrap()
                    .verifying_key()
                    .to_sec1_point(false)
                    .as_ref()
                    .to_vec(),
                TRANSCRIPT,
                good.clone(),
            ),
            (public_key(), TRANSCRIPT, "not base64!".to_owned()),
            (public_key(), TRANSCRIPT, STANDARD.encode(b"not der")),
            (
                public_key(),
                TRANSCRIPT,
                STANDARD.encode([0u8; MAX_ANDROID_SIGNATURE_BYTES + 1]),
            ),
            (vec![4; 65], TRANSCRIPT, good.clone()),
        ] {
            assert!(verify_signature(&public_key, transcript, &signature).is_err());
        }
    }
}
