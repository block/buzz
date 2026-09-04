//! Narrow App Attest verification boundary. Production enrollment accepts only
//! Apple production AAGUID material; unsupported devices have no bypass path.
use appattest::{assertion::Assertion, attestation::Attestation};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use byteorder::{BigEndian, ByteOrder};
use sha2::{Digest, Sha256};
use thiserror::Error;

const MAX_ASSERTION_BYTES: usize = 1024;
const APPLE_APP_ATTEST_ROOT_PEM_SHA256: [u8; 32] = [
    0xc7, 0x78, 0xd0, 0x9a, 0xc3, 0x41, 0xf7, 0xfd, 0x9f, 0x8f, 0x3b, 0x19, 0xe2, 0xb8, 0x15, 0xaf,
    0x6a, 0xed, 0x4a, 0xd4, 0x49, 0x0e, 0x1e, 0x92, 0xc0, 0x5c, 0xb3, 0x55, 0x21, 0x2a, 0x50, 0x13,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAttestation {
    pub key_id: Vec<u8>,
    pub public_key: Vec<u8>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedAssertion {
    pub counter: u32,
}
#[derive(Debug, Error)]
pub enum AppAttestError {
    #[error("invalid app attestation or assertion")]
    Invalid,
}

#[derive(Clone)]
pub struct AppAttestVerifier {
    app_id: String,
    apple_root_cert_pem: Vec<u8>,
}
impl AppAttestVerifier {
    pub fn new(app_id: String, apple_root_cert_pem: Vec<u8>) -> Result<Self, AppAttestError> {
        if app_id.is_empty()
            || Sha256::digest(&apple_root_cert_pem).as_slice() != APPLE_APP_ATTEST_ROOT_PEM_SHA256
        {
            return Err(AppAttestError::Invalid);
        }
        Ok(Self {
            app_id,
            apple_root_cert_pem,
        })
    }
    /// `client_data` is the exact canonical enrollment transcript represented by
    /// the challenge string passed to `attestKey`; callers must include every
    /// authority-bearing enrollment field in it.
    pub fn verify_attestation(
        &self,
        attestation_b64: &str,
        key_id_b64: &str,
        client_data: &[u8],
    ) -> Result<VerifiedAttestation, AppAttestError> {
        let cbor = STANDARD
            .decode(attestation_b64)
            .map_err(|_| AppAttestError::Invalid)?;
        if cbor.is_empty() || cbor.len() > crate::model::MAX_APP_ATTESTATION_BYTES {
            return Err(AppAttestError::Invalid);
        }
        let challenge = std::str::from_utf8(client_data).map_err(|_| AppAttestError::Invalid)?;
        let att = Attestation::from_cbor_bytes(&cbor).map_err(|_| AppAttestError::Invalid)?;
        let (public_key, _) = att
            .verify(
                challenge,
                &self.app_id,
                key_id_b64,
                &self.apple_root_cert_pem,
            )
            .map_err(|_| AppAttestError::Invalid)?;
        let key_id = STANDARD
            .decode(key_id_b64)
            .map_err(|_| AppAttestError::Invalid)?;
        if key_id.len() != 32 {
            return Err(AppAttestError::Invalid);
        }
        Ok(VerifiedAttestation {
            key_id,
            public_key: public_key.to_vec(),
        })
    }
    pub fn verify_assertion(
        &self,
        assertion_b64: &str,
        client_data: &[u8],
        public_key: &[u8],
        previous_counter: u32,
        challenge: &str,
        stored_challenge: &str,
    ) -> Result<VerifiedAssertion, AppAttestError> {
        let cbor = STANDARD
            .decode(assertion_b64)
            .map_err(|_| AppAttestError::Invalid)?;
        if cbor.is_empty() || cbor.len() > MAX_ASSERTION_BYTES {
            return Err(AppAttestError::Invalid);
        }
        let counter = assertion_counter(&cbor)?;
        let client_data_hash = Sha256::digest(client_data);
        Assertion::from_assertion(&cbor)
            .map_err(|_| AppAttestError::Invalid)?
            .verify(
                client_data_hash,
                challenge,
                &self.app_id,
                public_key,
                previous_counter,
                stored_challenge,
            )
            .map_err(|_| AppAttestError::Invalid)?;
        Ok(VerifiedAssertion { counter })
    }
}

/// App Attest assertion CBOR is a closed two-field map. Extracting signCount
/// from authenticatorData is safe only after the library verifies the same
/// bytes' RP ID, signature, and monotonic relation.
fn assertion_counter(cbor: &[u8]) -> Result<u32, AppAttestError> {
    let mut d = minicbor::Decoder::new(cbor);
    let count = d
        .map()
        .map_err(|_| AppAttestError::Invalid)?
        .ok_or(AppAttestError::Invalid)?;
    let mut auth = None;
    for _ in 0..count {
        let k = d.str().map_err(|_| AppAttestError::Invalid)?;
        match k {
            "authenticatorData" => auth = Some(d.bytes().map_err(|_| AppAttestError::Invalid)?),
            "signature" => {
                d.bytes().map_err(|_| AppAttestError::Invalid)?;
            }
            _ => return Err(AppAttestError::Invalid),
        }
    }
    let auth = auth
        .filter(|a| a.len() == 37)
        .ok_or(AppAttestError::Invalid)?;
    Ok(BigEndian::read_u32(&auth[33..37]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use appattest::error::AppAttestError as DependencyAppAttestError;
    use serde::Deserialize;

    const GOOD_FIXTURE_JSON: &str = include_str!("../tests/fixtures/app-attest-good.json");
    const WRONG_AAGUID_FIXTURE_JSON: &str =
        include_str!("../tests/fixtures/app-attest-wrong-aaguid.json");
    const WRONG_ROOT_FIXTURE_JSON: &str =
        include_str!("../tests/fixtures/app-attest-wrong-root.json");
    const APPLE_ROOT_CERT_PEM: &[u8] =
        include_bytes!("../tests/fixtures/apple-app-attestation-root.pem");

    #[derive(Deserialize)]
    struct Fixture {
        description: String,
        app_id: String,
        challenge: String,
        aaguid: String,
        attestation_b64: String,
        key_id_b64: String,
        root_cert_pem: String,
    }

    fn fixture(json: &str) -> Fixture {
        let fixture: Fixture = serde_json::from_str(json).expect("valid App Attest fixture JSON");
        assert!(!fixture.description.is_empty());
        fixture
    }

    fn verifier(app_id: &str, root_cert_pem: &[u8]) -> AppAttestVerifier {
        AppAttestVerifier {
            app_id: app_id.to_owned(),
            apple_root_cert_pem: root_cert_pem.to_vec(),
        }
    }

    fn verify_dependency(
        fixture: &Fixture,
        app_id: &str,
        challenge: &str,
        key_id_b64: &str,
        root_cert_pem: &[u8],
    ) -> Result<(), DependencyAppAttestError> {
        let cbor = STANDARD
            .decode(&fixture.attestation_b64)
            .expect("fixture attestation is base64");
        let attestation = Attestation::from_cbor_bytes(&cbor)?;
        let result = attestation
            .verify(challenge, app_id, key_id_b64, root_cert_pem)
            .map(|_| ());
        result
    }

    #[test]
    fn strict_verifier_accepts_good_fixture() {
        let fixture = fixture(GOOD_FIXTURE_JSON);
        assert_eq!(fixture.aaguid, "appattest");
        verify_dependency(
            &fixture,
            &fixture.app_id,
            &fixture.challenge,
            &fixture.key_id_b64,
            fixture.root_cert_pem.as_bytes(),
        )
        .expect("strict dependency verifier accepts the generated encoding");

        let verified = verifier(&fixture.app_id, fixture.root_cert_pem.as_bytes())
            .verify_attestation(
                &fixture.attestation_b64,
                &fixture.key_id_b64,
                fixture.challenge.as_bytes(),
            )
            .expect("shipped gateway wrapper accepts the generated encoding");
        assert_eq!(verified.key_id.len(), 32);
        assert_eq!(verified.public_key.len(), 65);
    }

    #[test]
    fn wrong_root_is_rejected() {
        let good = fixture(GOOD_FIXTURE_JSON);
        let wrong_root = fixture(WRONG_ROOT_FIXTURE_JSON);
        assert!(verify_dependency(
            &wrong_root,
            &wrong_root.app_id,
            &wrong_root.challenge,
            &wrong_root.key_id_b64,
            good.root_cert_pem.as_bytes(),
        )
        .is_err());
        assert!(verifier(&wrong_root.app_id, good.root_cert_pem.as_bytes())
            .verify_attestation(
                &wrong_root.attestation_b64,
                &wrong_root.key_id_b64,
                wrong_root.challenge.as_bytes(),
            )
            .is_err());
    }

    #[test]
    fn wrong_app_id_is_rejected() {
        let fixture = fixture(GOOD_FIXTURE_JSON);
        let wrong_app_id = "TEAMID.xyz.buzz.wrong";
        assert_eq!(
            verify_dependency(
                &fixture,
                wrong_app_id,
                &fixture.challenge,
                &fixture.key_id_b64,
                fixture.root_cert_pem.as_bytes(),
            ),
            Err(DependencyAppAttestError::InvalidAppID)
        );
        assert!(verifier(wrong_app_id, fixture.root_cert_pem.as_bytes())
            .verify_attestation(
                &fixture.attestation_b64,
                &fixture.key_id_b64,
                fixture.challenge.as_bytes(),
            )
            .is_err());
    }

    #[test]
    fn wrong_challenge_is_rejected() {
        let fixture = fixture(GOOD_FIXTURE_JSON);
        let wrong_challenge = "wrong-challenge";
        assert_eq!(
            verify_dependency(
                &fixture,
                &fixture.app_id,
                wrong_challenge,
                &fixture.key_id_b64,
                fixture.root_cert_pem.as_bytes(),
            ),
            Err(DependencyAppAttestError::InvalidNonce)
        );
        assert!(verifier(&fixture.app_id, fixture.root_cert_pem.as_bytes())
            .verify_attestation(
                &fixture.attestation_b64,
                &fixture.key_id_b64,
                wrong_challenge.as_bytes(),
            )
            .is_err());
    }

    #[test]
    fn wrong_aaguid_is_rejected_as_invalid_aaguid() {
        let fixture = fixture(WRONG_AAGUID_FIXTURE_JSON);
        assert_eq!(fixture.aaguid, "appattestdevelop");
        assert_eq!(
            verify_dependency(
                &fixture,
                &fixture.app_id,
                &fixture.challenge,
                &fixture.key_id_b64,
                fixture.root_cert_pem.as_bytes(),
            ),
            Err(DependencyAppAttestError::InvalidAAGUID)
        );
        assert!(verifier(&fixture.app_id, fixture.root_cert_pem.as_bytes())
            .verify_attestation(
                &fixture.attestation_b64,
                &fixture.key_id_b64,
                fixture.challenge.as_bytes(),
            )
            .is_err());
    }

    #[test]
    fn short_and_oversize_key_ids_are_rejected() {
        let fixture = fixture(GOOD_FIXTURE_JSON);
        for key_id_b64 in [STANDARD.encode([0x11; 31]), STANDARD.encode([0x22; 33])] {
            assert!(verify_dependency(
                &fixture,
                &fixture.app_id,
                &fixture.challenge,
                &key_id_b64,
                fixture.root_cert_pem.as_bytes(),
            )
            .is_err());
            assert!(verifier(&fixture.app_id, fixture.root_cert_pem.as_bytes())
                .verify_attestation(
                    &fixture.attestation_b64,
                    &key_id_b64,
                    fixture.challenge.as_bytes(),
                )
                .is_err());
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants, unexpected_cfgs)]
    fn gateway_test_build_does_not_define_testing_feature() {
        assert!(!cfg!(feature = "testing"));
    }

    #[test]
    fn constructor_still_pins_the_apple_root() {
        let fixture = fixture(GOOD_FIXTURE_JSON);
        assert!(
            AppAttestVerifier::new(fixture.app_id.clone(), APPLE_ROOT_CERT_PEM.to_vec()).is_ok()
        );
        assert!(
            AppAttestVerifier::new(fixture.app_id, fixture.root_cert_pem.as_bytes().to_vec(),)
                .is_err()
        );
    }
}
