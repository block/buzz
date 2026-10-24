use super::*;
use p256::ecdsa::{signature::Signer, Signature, SigningKey};

const APP_ID: &str = "TEAM.test.app";
const TRANSCRIPT: &[u8] = b"buzz.push.delegate.v1\n{\"challenge\":\"test-only\"}";

fn signing_key() -> SigningKey {
    SigningKey::from_slice(&[7; 32]).unwrap()
}

fn auth_data(extensions: &[u8]) -> Vec<u8> {
    let mut data = Sha256::digest(APP_ID.as_bytes()).to_vec();
    data.push(if extensions.is_empty() { 0x40 } else { 0xc0 });
    data.extend_from_slice(&1u32.to_be_bytes());
    data.extend_from_slice(extensions);
    data
}

fn signed_assertion(data: &[u8]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(data);
    hash.update(Sha256::digest(TRANSCRIPT));
    let signature: Signature = signing_key().sign(&hash.finalize());
    let mut buffer = [0u8; 2048];
    let mut encoder =
        minicbor::Encoder::new(minicbor::encode::write::Cursor::new(buffer.as_mut_slice()));
    encoder
        .map(2)
        .unwrap()
        .str("authenticatorData")
        .unwrap()
        .bytes(data)
        .unwrap()
        .str("signature")
        .unwrap()
        .bytes(signature.to_der().as_bytes())
        .unwrap();
    let length = encoder.writer().position();
    buffer[..length].to_vec()
}

fn verify(cbor: &[u8]) -> Result<VerifiedAssertion, AppAttestError> {
    let verifier = AppAttestVerifier::new(
        APP_ID.into(),
        include_bytes!("../tests/fixtures/apple-app-attestation-root.pem").to_vec(),
    )
    .unwrap();
    verifier.verify_assertion(
        &STANDARD.encode(cbor),
        TRANSCRIPT,
        signing_key()
            .verifying_key()
            .to_encoded_point(false)
            .as_ref(),
        0,
        "challenge",
        "challenge",
    )
}

#[test]
fn legacy_assertion_round_trip() {
    assert_eq!(
        verify(&signed_assertion(&auth_data(&[]))).unwrap().counter,
        1
    );
}

#[test]
fn production_verifier_accepts_extended_assertion() {
    // General unknown extension with a nested array, not an SE-specific length.
    let extension = b"\xa1\x66future\x83\x01\xf5\x62ok";
    assert_eq!(
        verify(&signed_assertion(&auth_data(extension)))
            .unwrap()
            .counter,
        1
    );
}

// Every conformance case exercises the shipped verification boundary.
mod conformance {
    use super::*;
    use ciborium::Value;

    fn value(bytes: &[u8]) -> Result<Value, &'static str> {
        ciborium::from_reader(bytes).map_err(|_| "CBOR test fixture")
    }

    fn verify_bytes(
        cbor: &[u8],
        transcript: &[u8],
        app: &str,
        key: &[u8],
        previous: u32,
    ) -> Result<u32, &'static str> {
        let verifier = AppAttestVerifier::new(
            app.into(),
            include_bytes!("../tests/fixtures/apple-app-attestation-root.pem").to_vec(),
        )
        .map_err(|_| "configuration")?;
        verifier
            .verify_assertion(
                &STANDARD.encode(cbor),
                transcript,
                key,
                previous,
                "fresh",
                "fresh",
            )
            .map(|verified| verified.counter)
            .map_err(|_| "invalid")
    }

    fn verify(cbor: &[u8]) -> Result<u32, &'static str> {
        verify_bytes(
            cbor,
            TRANSCRIPT,
            APP_ID,
            signing_key()
                .verifying_key()
                .to_encoded_point(false)
                .as_ref(),
            0,
        )
    }

    #[test]
    fn varied_extensions_and_legacy_verify() {
        for ext in [
            b"".as_slice(),
            b"\xa0",
            b"\xa1\x66future\x83\x01\xf5\x62ok",
            b"\xbf\x66future\x9f\x01\xf5\xff\xff",
            b"\xa2\x61a\x18\x01\x61b\x43abc",
            b"\xa2\x61b\x43abc\x61a\x18\x01",
            b"\xa1\x61x\xa1\x01\xc1\x00",
        ] {
            assert_eq!(verify(&signed_assertion(&auth_data(ext))), Ok(1));
        }
    }

    #[test]
    fn malformed_signed_extensions_rejected() {
        for ext in [
            b"\xa1".as_slice(),
            b"\xa0\x00",
            b"\x81\x01",
            b"\xa2\x61x\x01\x61x\x02",
            b"\xa1\x61x\xa2\x01\x01\x18\x01\x02",
            b"\xa1\x01\x01",
            b"\xa1\x61x\xff",
        ] {
            assert!(
                verify(&signed_assertion(&auth_data(ext))).is_err(),
                "{ext:?}"
            );
        }
        for len in 0..37 {
            assert!(verify(&signed_assertion(&auth_data(&[])[..len])).is_err());
        }
    }

    #[test]
    fn all_truncations_and_envelope_ambiguity_rejected() {
        let cbor = signed_assertion(&auth_data(b"\xa1\x61x\x01"));
        for len in 0..cbor.len() {
            assert!(verify(&cbor[..len]).is_err());
        }
        let mut trailing = cbor.clone();
        trailing.push(0);
        assert!(verify(&trailing).is_err());
        let mut duplicate = cbor.clone();
        duplicate[0] = 0xa3;
        duplicate.extend_from_slice(b"\x69signature\x40");
        assert!(verify(&duplicate).is_err());
    }

    #[test]
    fn signed_bytes_identity_key_and_replay_preserved() {
        let cbor = signed_assertion(&auth_data(b"\xa1\x61x\x01"));
        let key = signing_key().verifying_key().to_encoded_point(false);
        assert!(verify_bytes(&cbor, b"changed", APP_ID, key.as_ref(), 0).is_err());
        assert!(verify_bytes(&cbor, TRANSCRIPT, "other.app", key.as_ref(), 0).is_err());
        assert!(verify_bytes(&cbor, TRANSCRIPT, APP_ID, key.as_ref(), 1).is_err());
        assert!(verify_bytes(&cbor, TRANSCRIPT, APP_ID, key.as_ref(), 2).is_err());
        let other = SigningKey::from_slice(&[8; 32]).unwrap();
        assert!(verify_bytes(
            &cbor,
            TRANSCRIPT,
            APP_ID,
            other.verifying_key().to_encoded_point(false).as_ref(),
            0
        )
        .is_err());
        let mut decoded = value(&cbor).unwrap();
        if let Value::Map(ref mut entries) = decoded {
            for (k, v) in entries {
                if k.as_text() == Some("authenticatorData") {
                    if let Value::Bytes(data) = v {
                        *data.last_mut().unwrap() = 2;
                    }
                }
            }
        }
        let mut changed = Vec::new();
        ciborium::into_writer(&decoded, &mut changed).unwrap();
        assert!(verify(&changed).is_err());
    }

    #[test]
    fn extension_size_is_not_a_format_constant() {
        for len in 0..512 {
            let extension = Value::Map(vec![(
                Value::Text(format!("future_{len}")),
                Value::Bytes(vec![0x71; len]),
            )]);
            let mut bytes = Vec::new();
            ciborium::into_writer(&extension, &mut bytes).unwrap();
            assert_eq!(verify(&signed_assertion(&auth_data(&bytes))), Ok(1));
        }
    }

    #[test]
    #[ignore = "requires protected local capture; never commit raw device assertions"]
    fn captured_device_assertion_verifies_offline() {
        let root = std::path::PathBuf::from(std::env::var("APP_ATTEST_EVIDENCE_DIR").unwrap());
        let capture: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("se-attest-diagnostic.json")).unwrap())
                .unwrap();
        let stored: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("gateway-verification-input.json")).unwrap(),
        )
        .unwrap();
        let assertion = STANDARD
            .decode(capture["assertion"].as_str().unwrap())
            .unwrap();
        let transcript = STANDARD
            .decode(capture["transcript"].as_str().unwrap())
            .unwrap();
        let key = STANDARD
            .decode(stored["public_key"].as_str().unwrap().replace('\n', ""))
            .unwrap();
        assert_eq!(
            verify_bytes(
                &assertion,
                &transcript,
                "DE7S2975CH.com.tombrow.buzz.mobile",
                &key,
                stored["assertion_counter"].as_u64().unwrap() as u32
            ),
            Ok(64)
        );
    }
}

/// Private device evidence stays outside the repository. This exercises the
/// production verification boundary.
#[test]
#[ignore = "requires protected same-key SDK comparison captures"]
fn same_key_sdk26_and_sdk27_assertions_verify() {
    let root = std::path::PathBuf::from(std::env::var("APP_ATTEST_SDK_EVIDENCE_DIR").unwrap());
    let mut previous = 0;
    let mut enrolled_key = None;
    for build in ["2601", "2702"] {
        let fixture: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("verifier-input-{build}.json"))).unwrap(),
        )
        .unwrap();
        let verifier = AppAttestVerifier::new(
            fixture["app_id"].as_str().unwrap().into(),
            include_bytes!("../tests/fixtures/apple-app-attestation-root.pem").to_vec(),
        )
        .unwrap();
        let assertion = fixture["assertion"].as_str().unwrap();
        let transcript = STANDARD
            .decode(fixture["transcript"].as_str().unwrap())
            .unwrap();
        let key = STANDARD
            .decode(fixture["public_key"].as_str().unwrap())
            .unwrap();
        if let Some(expected) = &enrolled_key {
            assert_eq!(
                &key, expected,
                "the SDK comparison must retain the same key"
            );
        } else {
            enrolled_key = Some(key.clone());
        }
        let verified = verifier
            .verify_assertion(assertion, &transcript, &key, previous, "fresh", "fresh")
            .unwrap_or_else(|_| panic!("SDK probe build {build} must verify"));
        assert_eq!(
            u64::from(verified.counter),
            fixture["counter"].as_u64().unwrap()
        );
        assert!(verifier
            .verify_assertion(
                assertion,
                &transcript,
                &key,
                verified.counter,
                "fresh",
                "fresh"
            )
            .is_err());
        let mut wrong_transcript = transcript.clone();
        wrong_transcript.push(0);
        assert!(verifier
            .verify_assertion(
                assertion,
                &wrong_transcript,
                &key,
                previous,
                "fresh",
                "fresh"
            )
            .is_err());
        previous = verified.counter;
    }
}

#[cfg(feature = "personal-dev-app-attest")]
#[test]
#[ignore = "requires protected SDK comparison enrollment capture"]
fn sdk26_and_sdk27_device_enrollments_verify_with_exact_environment() {
    let root = std::path::PathBuf::from(std::env::var("APP_ATTEST_SDK_EVIDENCE_DIR").unwrap());
    for build in ["2601", "2703"] {
        let fixture: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("enrollment-input-{build}.json"))).unwrap(),
        )
        .unwrap();
        let transcript = STANDARD
            .decode(fixture["transcript"].as_str().unwrap())
            .unwrap();
        let verifier = AppAttestVerifier::with_environment(
            fixture["app_id"].as_str().unwrap().into(),
            include_bytes!("../tests/fixtures/apple-app-attestation-root.pem").to_vec(),
            AppAttestEnvironment::Development,
        )
        .unwrap();
        let verified = verifier
            .verify_attestation(
                fixture["attestation"].as_str().unwrap(),
                fixture["key_id"].as_str().unwrap(),
                &transcript,
            )
            .unwrap();
        let expected: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("verifier-input-{build}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(
            verified.public_key,
            STANDARD
                .decode(expected["public_key"].as_str().unwrap())
                .unwrap()
        );
        assert_eq!(
            verifier
                .verify_assertion(
                    expected["assertion"].as_str().unwrap(),
                    &STANDARD
                        .decode(expected["transcript"].as_str().unwrap())
                        .unwrap(),
                    &verified.public_key,
                    0,
                    "fresh",
                    "fresh",
                )
                .unwrap()
                .counter,
            expected["counter"].as_u64().unwrap() as u32
        );
        let production = AppAttestVerifier::new(
            fixture["app_id"].as_str().unwrap().into(),
            include_bytes!("../tests/fixtures/apple-app-attestation-root.pem").to_vec(),
        )
        .unwrap();
        assert!(production
            .verify_attestation(
                fixture["attestation"].as_str().unwrap(),
                fixture["key_id"].as_str().unwrap(),
                &transcript
            )
            .is_err());
    }
}

mod properties {
    use super::*;
    use ciborium::Value;
    use proptest::prelude::*;

    fn optional_value() -> impl Strategy<Value = Value> {
        prop_oneof![
            any::<i64>().prop_map(|v| Value::Integer(v.into())),
            any::<bool>().prop_map(Value::Bool),
            proptest::collection::vec(any::<u8>(), 0..16).prop_map(Value::Bytes),
            "[a-z]{0,12}".prop_map(Value::Text),
        ]
        .prop_recursive(4, 32, 4, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
                proptest::collection::btree_map("[a-z]{1,6}", inner.clone(), 0..4).prop_map(|m| {
                    Value::Map(m.into_iter().map(|(k, v)| (Value::Text(k), v)).collect())
                }),
                inner.prop_map(|v| Value::Tag(1000, Box::new(v))),
            ]
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        #[test]
        fn signed_optional_values_preserve_byte_authentication(v in optional_value(), selector in any::<usize>()) {
            let mut tail = Vec::new();
            ciborium::into_writer(&Value::Map(vec![(Value::Text("future".into()), v)]), &mut tail).unwrap();
            prop_assert!(tail.len() < 800);
            let auth = auth_data(&tail);
            let signed = signed_assertion(&auth);
            prop_assert!(verify(&signed).is_ok());
            // Mutate an authenticated extension byte while retaining its original signature.
            let mut envelope: Value = ciborium::from_reader(signed.as_slice()).unwrap();
            let Value::Map(fields) = &mut envelope else { unreachable!() };
            for (key, value) in fields {
                if key.as_text() == Some("authenticatorData") {
                    let Value::Bytes(bytes) = value else { unreachable!() };
                    bytes[37 + selector % tail.len()] ^= 1;
                }
            }
            let mut changed = Vec::new();
            ciborium::into_writer(&envelope, &mut changed).unwrap();
            prop_assert!(verify(&changed).is_err());
            prop_assert!(verify(&signed[..selector % signed.len()]).is_err());
        }
    }

    #[test]
    fn maximum_counter_is_accepted_once_and_cannot_wrap() {
        let key = signing_key().verifying_key().to_encoded_point(false);
        let mut auth = auth_data(b"\xa0");
        auth[33..37].copy_from_slice(&u32::MAX.to_be_bytes());
        let signed = signed_assertion(&auth);
        assert_eq!(
            super::super::proof::assert(&signed, TRANSCRIPT, APP_ID, key.as_ref(), u32::MAX - 1)
                .unwrap()
                .counter,
            u32::MAX
        );
        assert!(
            super::super::proof::assert(&signed, TRANSCRIPT, APP_ID, key.as_ref(), u32::MAX)
                .is_err()
        );
        auth[33..37].copy_from_slice(&0u32.to_be_bytes());
        assert!(super::super::proof::assert(
            &signed_assertion(&auth),
            TRANSCRIPT,
            APP_ID,
            key.as_ref(),
            u32::MAX
        )
        .is_err());
    }
}
