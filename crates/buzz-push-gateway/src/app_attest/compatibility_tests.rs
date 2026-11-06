//! Signed enrollment fixtures exercise structure independently of path trust.
use super::testing::{build_test_attestation_with_auth_data, TEST_ROOT_CA_CERT_PEM};
use super::{error::AppAttestError as Error, proof};
use base64::{engine::general_purpose::STANDARD, Engine};

const APP: &str = "TEAMID.test.compatibility";
const TRANSCRIPT: &str = "test enrollment transcript";

fn verify(change: impl FnOnce(&mut Vec<u8>)) -> Result<(), Error> {
    let fixture = build_test_attestation_with_auth_data(TRANSCRIPT, APP, change);
    enroll(&fixture.cbor, &fixture.key_id).map(|_| ())
}

fn enroll(bytes: &[u8], key_id: &str) -> Result<proof::Enrollment, Error> {
    proof::enroll(
        bytes,
        TRANSCRIPT.as_bytes(),
        APP,
        key_id,
        TEST_ROOT_CA_CERT_PEM,
        false,
    )
}

#[test]
fn signed_enrollment_accepts_optional_extensions_and_cose_variants() {
    for tail in [
        &b""[..],
        &b"\xa0"[..],
        &b"\xa1\x66future\x82\xc1\xf8\xff\x01"[..],
    ] {
        verify(|auth| auth.extend_from_slice(tail)).unwrap();
    }
    // COSE's x coordinate remains the same; y may use its parity form.
    verify(|auth| {
        let mut key: ciborium::Value = ciborium::from_reader(&auth[87..]).unwrap();
        if let ciborium::Value::Map(fields) = &mut key {
            for (label, value) in fields.iter_mut() {
                if label == &ciborium::Value::Integer((-3).into()) {
                    let ciborium::Value::Bytes(y) = value else {
                        panic!("fixture y")
                    };
                    *value = ciborium::Value::Bool(y[31] & 1 == 1);
                }
            }
            fields.reverse();
        }
        auth.truncate(87);
        ciborium::into_writer(&key, auth).unwrap();
    })
    .unwrap();
}

#[test]
fn signed_enrollment_preserves_fixed_hash_credential_framing() {
    // Upstream reports nonstandard length fields on iOS 26.x. These are
    // synthetic compatibility vectors, not captures from that OS version.
    for length in [0_u16, 31, 32, 33, 0x89f4, u16::MAX] {
        verify(|auth| auth[53..55].copy_from_slice(&length.to_be_bytes())).unwrap();
        assert!(verify(|auth| {
            auth[53..55].copy_from_slice(&length.to_be_bytes());
            auth[55] ^= 1;
        })
        .is_err());
    }
    // Actual missing/extra credential bytes must not shift the COSE boundary.
    assert!(verify(|auth| {
        auth.remove(55);
    })
    .is_err());
    assert!(verify(|auth| auth.insert(55, 0)).is_err());
}

#[test]
fn signed_enrollment_rejects_incomplete_or_inconsistent_credential_data() {
    assert!(verify(|auth| auth.truncate(87)).is_err()); // Old fixture omitted COSE.
    assert!(verify(|auth| auth[55] ^= 1).is_err());
    assert!(verify(|auth| auth[36] = 1).is_err());
    assert!(verify(|auth| auth.extend_from_slice(b"\xa0\x00")).is_err());
    assert!(verify(|auth| auth.extend_from_slice(b"\xa2\x61x\x01\x61x\x02")).is_err());
    assert!(verify(|auth| {
        // Valid key with the wrong algorithm label remains invalid even signed.
        let mut key: ciborium::Value = ciborium::from_reader(&auth[87..]).unwrap();
        if let ciborium::Value::Map(fields) = &mut key {
            for (label, value) in fields {
                if label == &ciborium::Value::Integer(3.into()) {
                    *value = ciborium::Value::Integer((-8).into());
                }
            }
        }
        auth.truncate(87);
        ciborium::into_writer(&key, auth).unwrap();
    })
    .is_err());
}

#[test]
fn certificate_nonce_authenticates_every_extension_byte() {
    let fixture = build_test_attestation_with_auth_data(TRANSCRIPT, APP, |a| {
        a.extend_from_slice(b"\xa1\x61x\x01")
    });
    // Even the compatibility length field must remain nonce-authenticated.
    for offset in [None, Some(53), Some(54)] {
        let mut envelope: ciborium::Value = ciborium::from_reader(fixture.cbor.as_slice()).unwrap();
        if let ciborium::Value::Map(fields) = &mut envelope {
            for (key, value) in fields {
                if key == &ciborium::Value::Text("authData".into()) {
                    let ciborium::Value::Bytes(auth) = value else {
                        panic!("authData")
                    };
                    let index = offset.unwrap_or(auth.len() - 1);
                    auth[index] ^= 1;
                }
            }
        }
        let mut changed = Vec::new();
        ciborium::into_writer(&envelope, &mut changed).unwrap();
        assert!(matches!(
            enroll(&changed, &fixture.key_id),
            Err(Error::InvalidNonce)
        ));
    }
    assert!(enroll(&fixture.cbor, &STANDARD.encode([0; 32])).is_err());
}

#[test]
fn shared_signed_certificate_constraint_corpus() {
    super::testing::compare_certificate_constraints(
        super::certificate::verify_chain,
        super::certificate::verify_nonce,
    );
}

#[test]
fn certificate_rejections_name_the_failed_check() {
    use super::error::CertificateCheck as Check;
    use super::testing::{rejected_certificate_chains, test_root_der, valid_certificate_chain};
    let root = test_root_der();
    // A path rejection keeps webpki's full error, whose variant names the check.
    let path = |variant: &str| Err(variant.to_owned());
    let is = |check: Check| Ok(Error::InvalidCertificate(check));
    let expected = [
        ("wrong usage", is(Check::KeyUsage { index: 0 })),
        ("expired", path("CertExpired {")),
        ("unknown critical", path("UnsupportedCriticalExtension")),
        ("duplicate nonce", is(Check::Encoding { index: 0 })),
        ("trailing", path("TrailingData(Certificate)")),
    ];
    let cases = rejected_certificate_chains();
    assert_eq!(cases.len(), expected.len());
    for ((name, chain), (expected_name, expected)) in cases.iter().zip(expected) {
        assert_eq!(*name, expected_name);
        let error = super::certificate::verify_chain(chain, &root).unwrap_err();
        match expected {
            Ok(expected) => assert_eq!(error, expected, "{name}"),
            Err(variant) => assert!(
                matches!(&error, Error::InvalidCertificate(Check::Path(detail)) if detail.starts_with(&variant)),
                "{name}: {error:?}"
            ),
        }
    }
    assert_eq!(
        Error::InvalidCertificate(Check::Path("UnknownIssuer".into())).to_string(),
        "invalid certificate path or structure (path rejected: UnknownIssuer)"
    );
    assert_eq!(
        super::certificate::verify_chain(&[], &root),
        Err(Error::InvalidCertificate(Check::Chain))
    );
    assert_eq!(
        super::certificate::verify_chain(&valid_certificate_chain(), b"not a certificate"),
        Err(Error::InvalidCertificate(Check::Root))
    );
}

#[test]
fn enrolled_key_verifies_following_assertion() {
    use super::testing::{build_test_assertion, build_test_attestation};
    use sha2::{Digest, Sha256};
    let fixture = build_test_attestation(TRANSCRIPT, APP);
    let enrolled = enroll(&fixture.cbor, &fixture.key_id).unwrap();
    let hash = Sha256::digest(TRANSCRIPT.as_bytes());
    let assertion = build_test_assertion(APP, hash, 0, &fixture.device_key);
    let checked = proof::assert(
        &assertion,
        TRANSCRIPT.as_bytes(),
        APP,
        &enrolled.public_key,
        0,
    )
    .unwrap();
    assert_eq!(checked.counter, 1);
    assert!(proof::assert(
        &assertion,
        TRANSCRIPT.as_bytes(),
        APP,
        &enrolled.public_key,
        1
    )
    .is_err());
}

#[test]
fn apple_historical_fixture_has_structural_extensions_with_ed_clear() {
    // Apple's public example is historical, not evidence of current chain validity.
    // https://developer.apple.com/documentation/devicecheck/attestation-object-validation-guide
    let bytes = STANDARD
        .decode(include_str!("../../tests/fixtures/apple-structural-attestation.b64").trim())
        .unwrap();
    let envelope = super::cbor::complete(&bytes).unwrap();
    let auth = envelope.field("authData").unwrap().bytes().unwrap();
    assert_eq!(auth.len(), 226);
    assert_eq!(auth[32], 0x40);
    let super::cbor::Value::Array(chain) = envelope.field("attStmt").unwrap().field("x5c").unwrap()
    else {
        panic!("certificate chain")
    };
    let leaf = chain[0].bytes().unwrap();
    let public_key = super::certificate::public_key(leaf).unwrap();
    let key_id = super::certificate::key_id(&public_key);
    let parsed = super::authenticator::enrollment(
        auth,
        "1234567890.com.example.myapp",
        &key_id,
        &public_key,
        false,
    )
    .unwrap();
    assert_eq!(parsed.counter, 0);
    assert_eq!(parsed.flags, 0x40);
}
