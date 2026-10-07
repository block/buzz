//! Narrow App Attest verification boundary. Production enrollment accepts only
//! Apple production AAGUID material by default; personal development requires
//! an explicit build feature and environment. No cryptographic checks are skipped.
use crate::config::AppAttestEnvironment;
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
    environment: AppAttestEnvironment,
}
impl AppAttestVerifier {
    /// Construct a production-only verifier.
    pub fn new(app_id: String, apple_root_cert_pem: Vec<u8>) -> Result<Self, AppAttestError> {
        Self::with_environment(
            app_id,
            apple_root_cert_pem,
            AppAttestEnvironment::Production,
        )
    }

    /// Construct a verifier for exactly one server-selected attestation environment.
    pub fn with_environment(
        app_id: String,
        apple_root_cert_pem: Vec<u8>,
        environment: AppAttestEnvironment,
    ) -> Result<Self, AppAttestError> {
        if app_id.is_empty()
            || Sha256::digest(&apple_root_cert_pem).as_slice() != APPLE_APP_ATTEST_ROOT_PEM_SHA256
        {
            return Err(AppAttestError::Invalid);
        }
        Ok(Self {
            app_id,
            apple_root_cert_pem,
            environment,
        })
    }
    /// Exact server-selected environment accepted by this verifier.
    pub fn environment(&self) -> AppAttestEnvironment {
        self.environment
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
        verify_attestation_environment(&cbor, self.environment)?;
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

// The dependency's development feature accepts both Apple environments. Fence
// the exact signed authData here, then let its full verifier validate those same
// bytes (chain, nonce, app ID, counter, public key and credential ID).
fn verify_attestation_environment(
    cbor: &[u8],
    environment: AppAttestEnvironment,
) -> Result<(), AppAttestError> {
    let mut decoder = minicbor::Decoder::new(cbor);
    let count = decoder
        .map()
        .map_err(|_| AppAttestError::Invalid)?
        .ok_or(AppAttestError::Invalid)?;
    if count != 3 {
        return Err(AppAttestError::Invalid);
    }
    let mut auth_data = None;
    let mut format_seen = false;
    let mut statement_seen = false;
    for _ in 0..count {
        match decoder.str().map_err(|_| AppAttestError::Invalid)? {
            "authData" if auth_data.is_none() => {
                auth_data = Some(decoder.bytes().map_err(|_| AppAttestError::Invalid)?);
            }
            "fmt" if !format_seen => {
                if decoder.str().map_err(|_| AppAttestError::Invalid)? != "apple-appattest" {
                    return Err(AppAttestError::Invalid);
                }
                format_seen = true;
            }
            "attStmt" if !statement_seen => {
                // appattest 0.1.1 treats indefinite containers as empty without
                // consuming their contents. Accept only the definite schema so
                // its cursor cannot reinterpret nested fields as root fields.
                if decoder.map().map_err(|_| AppAttestError::Invalid)? != Some(2) {
                    return Err(AppAttestError::Invalid);
                }
                let mut certs_seen = false;
                let mut receipt_seen = false;
                for _ in 0..2 {
                    match decoder.str().map_err(|_| AppAttestError::Invalid)? {
                        "x5c" if !certs_seen => {
                            let count = decoder
                                .array()
                                .map_err(|_| AppAttestError::Invalid)?
                                .filter(|count| (1..=3).contains(count))
                                .ok_or(AppAttestError::Invalid)?;
                            for _ in 0..count {
                                decoder.bytes().map_err(|_| AppAttestError::Invalid)?;
                            }
                            certs_seen = true;
                        }
                        "receipt" if !receipt_seen => {
                            decoder.bytes().map_err(|_| AppAttestError::Invalid)?;
                            receipt_seen = true;
                        }
                        _ => return Err(AppAttestError::Invalid),
                    }
                }
                statement_seen = true;
            }
            _ => return Err(AppAttestError::Invalid),
        }
    }
    if decoder.position() != cbor.len() {
        return Err(AppAttestError::Invalid);
    }
    let expected: &[u8] = match environment {
        AppAttestEnvironment::Production => b"appattest\0\0\0\0\0\0\0",
        #[cfg(feature = "personal-dev-app-attest")]
        AppAttestEnvironment::Development => b"appattestdevelop",
    };
    if auth_data.and_then(|data| data.get(37..53)) != Some(expected) {
        return Err(AppAttestError::Invalid);
    }
    Ok(())
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
            environment: AppAttestEnvironment::Production,
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
        #[cfg(not(feature = "personal-dev-app-attest"))]
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

    #[cfg(feature = "personal-dev-app-attest")]
    #[test]
    fn development_mode_preserves_verification_and_rejects_production() {
        let dev = fixture(WRONG_AAGUID_FIXTURE_JSON);
        let mut v = verifier(&dev.app_id, dev.root_cert_pem.as_bytes());
        v.environment = AppAttestEnvironment::Development;
        let cbor = STANDARD.decode(&dev.attestation_b64).unwrap();
        assert!(
            verify_attestation_environment(&cbor, v.environment).is_ok(),
            "environment parser"
        );
        verify_dependency(
            &dev,
            &dev.app_id,
            &dev.challenge,
            &dev.key_id_b64,
            dev.root_cert_pem.as_bytes(),
        )
        .expect("development dependency");
        assert!(v
            .verify_attestation(
                &dev.attestation_b64,
                &dev.key_id_b64,
                dev.challenge.as_bytes()
            )
            .is_ok());
        let prod = fixture(GOOD_FIXTURE_JSON);
        assert!(v
            .verify_attestation(
                &prod.attestation_b64,
                &prod.key_id_b64,
                prod.challenge.as_bytes()
            )
            .is_err());
        assert!(v
            .verify_attestation(&dev.attestation_b64, &dev.key_id_b64, b"wrong challenge")
            .is_err());
        assert!(v
            .verify_attestation(
                &dev.attestation_b64,
                &STANDARD.encode([0; 32]),
                dev.challenge.as_bytes()
            )
            .is_err());
        v.app_id = "OTHER.wrong.app".into();
        assert!(v
            .verify_attestation(
                &dev.attestation_b64,
                &dev.key_id_b64,
                dev.challenge.as_bytes()
            )
            .is_err());
        v.app_id = dev.app_id;
        v.apple_root_cert_pem = fixture(WRONG_ROOT_FIXTURE_JSON).root_cert_pem.into_bytes();
        assert!(v
            .verify_attestation(
                &dev.attestation_b64,
                &dev.key_id_b64,
                dev.challenge.as_bytes()
            )
            .is_err());
    }

    #[cfg(feature = "personal-dev-app-attest")]
    #[test]
    fn nested_statement_cannot_bypass_environment_fence() {
        for (json, environment, decoy_aaguid) in [
            (
                WRONG_AAGUID_FIXTURE_JSON,
                AppAttestEnvironment::Production,
                &b"appattest\0\0\0\0\0\0\0"[..],
            ),
            (
                GOOD_FIXTURE_JSON,
                AppAttestEnvironment::Development,
                &b"appattestdevelop"[..],
            ),
        ] {
            let fixture = fixture(json);
            let original = STANDARD.decode(&fixture.attestation_b64).unwrap();
            let mut decoder = minicbor::Decoder::new(&original);
            let count = decoder.map().unwrap().unwrap();
            let mut auth = &[][..];
            let mut statement = &[][..];
            for _ in 0..count {
                let key = decoder.str().unwrap();
                let start = decoder.position();
                decoder.skip().unwrap();
                match key {
                    "authData" => auth = &original[start..decoder.position()],
                    "attStmt" => statement = &original[start..decoder.position()],
                    _ => {}
                }
            }
            let mut decoy = [0u8; 53];
            decoy[37..53].copy_from_slice(decoy_aaguid);
            let mut buffer = vec![0; original.len() + 256];
            let mut encoder =
                minicbor::Encoder::new(minicbor::encode::write::Cursor::new(buffer.as_mut_slice()));
            encoder
                .map(5)
                .unwrap()
                .str("fmt")
                .unwrap()
                .str("apple-appattest")
                .unwrap()
                .str("authData")
                .unwrap()
                .bytes(&decoy)
                .unwrap()
                .str("attStmt")
                .unwrap()
                .begin_map()
                .unwrap()
                .str("attStmt")
                .unwrap();
            use minicbor::encode::Write;
            encoder.writer_mut().write_all(statement).unwrap();
            encoder.str("authData").unwrap();
            encoder.writer_mut().write_all(auth).unwrap();
            encoder
                .end()
                .unwrap()
                .str("padding1")
                .unwrap()
                .u8(0)
                .unwrap()
                .str("padding2")
                .unwrap()
                .u8(0)
                .unwrap();
            let length = encoder.writer().position();
            let malicious = &buffer[..length];
            // Prove this envelope reaches valid signed material in the dependency.
            Attestation::from_cbor_bytes(malicious)
                .unwrap()
                .verify(
                    &fixture.challenge,
                    &fixture.app_id,
                    &fixture.key_id_b64,
                    fixture.root_cert_pem.as_bytes(),
                )
                .unwrap();
            let mut verifier = verifier(&fixture.app_id, fixture.root_cert_pem.as_bytes());
            verifier.environment = environment;
            assert!(
                verifier
                    .verify_attestation(
                        &STANDARD.encode(malicious),
                        &fixture.key_id_b64,
                        fixture.challenge.as_bytes(),
                    )
                    .is_err(),
                "cross-environment nested statement accepted: {environment:?}"
            );
        }
    }

    #[test]
    fn environment_parser_rejects_ambiguous_or_truncated_auth_data() {
        for data in [vec![], vec![0xa0], vec![0xa1, 0x68], vec![0xbf, 0xff]] {
            assert!(
                verify_attestation_environment(&data, AppAttestEnvironment::Production).is_err()
            );
        }
        let mut data = [0u8; 256];
        let mut encoder =
            minicbor::Encoder::new(minicbor::encode::write::Cursor::new(data.as_mut_slice()));
        let mut auth = [0u8; 53];
        auth[37..53].copy_from_slice(b"appattest\0\0\0\0\0\0\0");
        encoder
            .map(2)
            .unwrap()
            .str("authData")
            .unwrap()
            .bytes(&auth)
            .unwrap()
            .str("authData")
            .unwrap()
            .bytes(&auth)
            .unwrap();
        let length = encoder.writer().position();
        assert!(
            verify_attestation_environment(&data[..length], AppAttestEnvironment::Production)
                .is_err()
        );
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
