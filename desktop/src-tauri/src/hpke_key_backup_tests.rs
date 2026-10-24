use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use nostr::{Keys, SecretKey, ToBech32};
use rustls::crypto::hpke::{EncapsulatedSecret, Hpke, HpkePrivateKey};

use super::*;

fn test_secret() -> SecretKey {
    SecretKey::from_slice(&[0x42; 32]).expect("fixed test secret is valid")
}

fn test_backup_id() -> Uuid {
    Uuid::parse_str("018f6f4e-83d1-7a4e-8c8e-4f914f7d31a2").expect("fixed UUID is valid")
}

fn test_recipient() -> (rustls::crypto::hpke::HpkePublicKey, HpkePrivateKey) {
    DH_KEM_P256_HKDF_SHA256_AES_256
        .generate_key_pair()
        .expect("generate test HPKE recipient")
}

fn enrollment(public_key: &[u8]) -> HpkeBackupEnrollment {
    HpkeBackupEnrollment::new(
        "backup-key-2026-09",
        "backup/staging",
        "subject:01J8A4M7Q1G4X8S8M51FS7K2HX",
        test_backup_id(),
        public_key,
    )
    .expect("valid test enrollment")
}

fn open_envelope(
    envelope: &HpkeBackupEnvelope,
    private_key: &HpkePrivateKey,
) -> Result<Vec<u8>, rustls::Error> {
    let aad = envelope
        .associated_data()
        .map_err(|error| rustls::Error::General(error.to_string()))?;
    let enc = envelope
        .encapsulated_key_bytes()
        .map_err(|error| rustls::Error::General(error.to_string()))?;
    let ciphertext = envelope
        .ciphertext_bytes()
        .map_err(|error| rustls::Error::General(error.to_string()))?;
    DH_KEM_P256_HKDF_SHA256_AES_256.open(
        &EncapsulatedSecret(enc),
        HPKE_INFO_V1,
        &aad,
        &ciphertext,
        private_key,
    )
}

#[test]
fn receiver_recovers_exact_secret_and_matching_pubkey() {
    let (public_key, private_key) = test_recipient();
    let secret = test_secret();
    let envelope = seal_nostr_secret(&secret, &enrollment(&public_key.0)).expect("seal backup");

    let opened = open_envelope(&envelope, &private_key).expect("open backup");
    assert_eq!(opened, secret.as_secret_bytes());
    assert_eq!(
        envelope.nostr_pubkey,
        Keys::new(secret).public_key().to_hex()
    );
    assert_eq!(envelope.enc.len(), 87);
    assert_eq!(envelope.ciphertext.len(), 64);
    assert_eq!(envelope.encapsulated_key_bytes().unwrap().len(), 65);
    assert_eq!(envelope.ciphertext_bytes().unwrap().len(), 48);
}

#[test]
fn every_backup_uses_a_fresh_hpke_context() {
    let (public_key, private_key) = test_recipient();
    let enrollment = enrollment(&public_key.0);
    let secret = test_secret();

    let first = seal_nostr_secret(&secret, &enrollment).expect("first seal");
    let second = seal_nostr_secret(&secret, &enrollment).expect("second seal");

    assert_ne!(first.enc, second.enc);
    assert_ne!(first.ciphertext, second.ciphertext);
    assert_eq!(open_envelope(&first, &private_key).unwrap(), [0x42; 32]);
    assert_eq!(open_envelope(&second, &private_key).unwrap(), [0x42; 32]);
}

#[test]
fn malformed_and_invalid_recipient_keys_are_rejected() {
    let short = [0x04; 64];
    assert!(matches!(
        HpkeBackupEnrollment::new("key", "service", "owner", test_backup_id(), &short),
        Err(HpkeBackupError::InvalidRecipientKey(_))
    ));

    let compressed = [0x02; 33];
    assert!(
        HpkeBackupEnrollment::new("key", "service", "owner", test_backup_id(), &compressed)
            .is_err()
    );

    let mut wrong_prefix = [0u8; P256_PUBLIC_KEY_LEN];
    wrong_prefix[0] = 0x03;
    assert!(
        HpkeBackupEnrollment::new("key", "service", "owner", test_backup_id(), &wrong_prefix)
            .is_err()
    );

    let mut off_curve = [0u8; P256_PUBLIC_KEY_LEN];
    off_curve[0] = 0x04;
    let enrollment =
        HpkeBackupEnrollment::new("key", "service", "owner", test_backup_id(), &off_curve)
            .expect("structurally encoded point reaches provider validation");
    assert_eq!(
        seal_nostr_secret(&test_secret(), &enrollment),
        Err(HpkeBackupError::EncryptionFailed)
    );
}

#[test]
fn metadata_ciphertext_and_encapsulation_tampering_fail_closed() {
    let (public_key, private_key) = test_recipient();
    let envelope = seal_nostr_secret(&test_secret(), &enrollment(&public_key.0)).unwrap();

    let mut recipient_key_id_tampered = envelope.clone();
    recipient_key_id_tampered
        .recipient_key_id
        .push_str("-different");
    let mut namespace_tampered = envelope.clone();
    namespace_tampered.service_namespace.push_str("-different");
    let mut owner_tampered = envelope.clone();
    owner_tampered.owner_id.push_str("-different");
    let mut backup_id_tampered = envelope.clone();
    backup_id_tampered.backup_id = Uuid::new_v4().hyphenated().to_string();
    let mut pubkey_tampered = envelope.clone();
    pubkey_tampered.nostr_pubkey = Keys::generate().public_key().to_hex();
    for tampered in [
        recipient_key_id_tampered,
        namespace_tampered,
        owner_tampered,
        backup_id_tampered,
        pubkey_tampered,
    ] {
        assert!(open_envelope(&tampered, &private_key).is_err());
    }

    let aad = envelope.associated_data().unwrap();
    let enc = envelope.encapsulated_key_bytes().unwrap();
    let ciphertext = envelope.ciphertext_bytes().unwrap();
    assert!(DH_KEM_P256_HKDF_SHA256_AES_256
        .open(
            &EncapsulatedSecret(enc),
            b"buzz/nsec-backup/v2",
            &aad,
            &ciphertext,
            &private_key,
        )
        .is_err());

    let mut ciphertext_tampered = envelope.clone();
    let mut ciphertext = ciphertext_tampered.ciphertext_bytes().unwrap();
    ciphertext[0] ^= 1;
    ciphertext_tampered.ciphertext = URL_SAFE_NO_PAD.encode(ciphertext);
    assert!(open_envelope(&ciphertext_tampered, &private_key).is_err());

    let mut enc_tampered = envelope.clone();
    let mut enc = enc_tampered.encapsulated_key_bytes().unwrap();
    let last = enc.len() - 1;
    enc[last] ^= 1;
    enc_tampered.enc = URL_SAFE_NO_PAD.encode(enc);
    assert!(open_envelope(&enc_tampered, &private_key).is_err());
}

#[test]
fn wrong_recipient_cannot_open() {
    let (public_key, _) = test_recipient();
    let (_, wrong_private_key) = test_recipient();
    let envelope = seal_nostr_secret(&test_secret(), &enrollment(&public_key.0)).unwrap();
    assert!(open_envelope(&envelope, &wrong_private_key).is_err());
}

#[test]
fn enrollment_rejects_invalid_context_fields() {
    let (public_key, _) = test_recipient();
    assert!(
        HpkeBackupEnrollment::new("", "service", "owner", test_backup_id(), &public_key.0).is_err()
    );
    assert!(HpkeBackupEnrollment::new(
        "key",
        "x".repeat(256),
        "owner",
        test_backup_id(),
        &public_key.0
    )
    .is_err());
    assert!(HpkeBackupEnrollment::new(
        "key",
        "service",
        "owner\nvalue",
        test_backup_id(),
        &public_key.0
    )
    .is_err());
}

#[test]
fn aad_length_framing_prevents_field_boundary_collisions() {
    let (public_key, private_key) = test_recipient();
    let secret = test_secret();

    for (left, right) in [
        (["ab", "c", "owner"], ["a", "bc", "owner"]),
        (["key", "ab", "c"], ["key", "a", "bc"]),
    ] {
        assert_eq!(left.concat(), right.concat());
        let enrollment =
            HpkeBackupEnrollment::new(left[0], left[1], left[2], test_backup_id(), &public_key.0)
                .unwrap();
        let envelope = seal_nostr_secret(&secret, &enrollment).unwrap();
        assert_eq!(
            open_envelope(&envelope, &private_key).unwrap(),
            secret.as_secret_bytes()
        );

        let mut reassigned = envelope.clone();
        reassigned.recipient_key_id = right[0].into();
        reassigned.service_namespace = right[1].into();
        reassigned.owner_id = right[2].into();
        assert_ne!(
            envelope.associated_data().unwrap(),
            reassigned.associated_data().unwrap()
        );
        assert!(open_envelope(&reassigned, &private_key).is_err());
    }
}

#[test]
fn envelope_rejects_noncanonical_or_conflicting_metadata() {
    let (public_key, _) = test_recipient();
    let envelope = seal_nostr_secret(&test_secret(), &enrollment(&public_key.0)).unwrap();

    let mut wrong_version = envelope.clone();
    wrong_version.version = 2;
    assert!(wrong_version.associated_data().is_err());

    let mut wrong_suite = envelope.clone();
    wrong_suite.suite.aead_id = 1;
    assert!(wrong_suite.associated_data().is_err());

    let mut uppercase_pubkey = envelope.clone();
    uppercase_pubkey.nostr_pubkey.make_ascii_uppercase();
    assert!(uppercase_pubkey.associated_data().is_err());

    let mut padded_base64 = envelope.clone();
    padded_base64.enc.push('=');
    assert!(padded_base64.encapsulated_key_bytes().is_err());
}

#[test]
fn envelope_rejects_binary_fields_with_wrong_byte_lengths_before_decode() {
    let (public_key, _) = test_recipient();
    let envelope = seal_nostr_secret(&test_secret(), &enrollment(&public_key.0)).unwrap();

    let mut oversized_enc = envelope.clone();
    oversized_enc.enc.push_str(&"A".repeat(1_000_000));
    assert_eq!(
        oversized_enc.encapsulated_key_bytes(),
        Err(HpkeBackupError::InvalidField {
            field: "enc",
            reason: "unexpected encoded length",
        })
    );

    let mut short_enc = envelope.clone();
    short_enc.enc.pop();
    assert_eq!(
        short_enc.encapsulated_key_bytes(),
        Err(HpkeBackupError::InvalidField {
            field: "enc",
            reason: "unexpected encoded length",
        })
    );

    let mut multibyte_ciphertext = envelope.clone();
    multibyte_ciphertext.ciphertext = "é".repeat(envelope.ciphertext.len());
    assert_eq!(
        multibyte_ciphertext.ciphertext_bytes(),
        Err(HpkeBackupError::InvalidField {
            field: "ciphertext",
            reason: "unexpected encoded length",
        })
    );
}

#[test]
fn envelope_rejects_malformed_base64_and_wrong_encapsulation_prefix() {
    let (public_key, _) = test_recipient();
    let envelope = seal_nostr_secret(&test_secret(), &enrollment(&public_key.0)).unwrap();

    let mut malformed_enc = envelope.clone();
    malformed_enc.enc.replace_range(..1, "!");
    assert_eq!(
        malformed_enc.encapsulated_key_bytes(),
        Err(HpkeBackupError::InvalidField {
            field: "enc",
            reason: "invalid base64url",
        })
    );

    let mut malformed_ciphertext = envelope.clone();
    malformed_ciphertext.ciphertext.replace_range(..1, "!");
    assert_eq!(
        malformed_ciphertext.ciphertext_bytes(),
        Err(HpkeBackupError::InvalidField {
            field: "ciphertext",
            reason: "invalid base64url",
        })
    );

    let mut wrong_prefix = envelope.clone();
    let mut enc = URL_SAFE_NO_PAD.decode(&wrong_prefix.enc).unwrap();
    enc[0] = 0x03;
    wrong_prefix.enc = URL_SAFE_NO_PAD.encode(enc);
    assert_eq!(
        wrong_prefix.encapsulated_key_bytes(),
        Err(HpkeBackupError::InvalidField {
            field: "enc",
            reason: "expected uncompressed SEC1 prefix 0x04",
        })
    );
}

#[test]
fn serialized_envelope_contains_no_plaintext_or_nsec() {
    let (public_key, _) = test_recipient();
    let secret = test_secret();
    let envelope = seal_nostr_secret(&secret, &enrollment(&public_key.0)).unwrap();
    let json = serde_json::to_string(&envelope).unwrap();
    let raw_hex = hex::encode(secret.as_secret_bytes());
    let nsec = secret.to_bech32().unwrap();

    assert!(!json.contains(&raw_hex));
    assert!(!json.contains(&nsec));
    assert!(!json.contains("nsec1"));
}

#[test]
fn official_rfc9180_exact_suite_vector_opens() {
    // RFC 9180's JSON vector for the KEM/KDF/AEAD tuple used here.
    // Source pinned by the RFC itself:
    // https://github.com/cfrg/draft-irtf-cfrg-hpke/blob/
    // 5f503c564da00b0687b3de75f1dfbdfc4079ad31/test-vectors.json
    let info = hex::decode("4f6465206f6e2061204772656369616e2055726e").unwrap();
    let aad = hex::decode("436f756e742d30").unwrap();
    let plaintext =
        hex::decode("4265617574792069732074727574682c20747275746820626561757479").unwrap();
    let enc = hex::decode(
        "04c06b4f6bebc7bb495cb797ab753f911aff80aefb86fd8b6fcc35525f3ab5f03e0b21bd31a86c6048af3cb2d98e0d3bf01da5cc4c39ff5370d331a4f1f7d5a4e0",
    )
    .unwrap();
    let ciphertext = hex::decode(
        "58c61a45059d0c5704560e9d88b564a8b63f1364b8d1fcb3c4c6ddc1d291742465e902cd216f8908da49f8f96f",
    )
    .unwrap();
    let recipient_private = HpkePrivateKey::from(
        hex::decode("317f915db7bc629c48fe765587897e01e282d3e8445f79f27f65d031a88082b2").unwrap(),
    );

    let opened = DH_KEM_P256_HKDF_SHA256_AES_256
        .open(
            &EncapsulatedSecret(enc),
            &info,
            &aad,
            &ciphertext,
            &recipient_private,
        )
        .expect("open official CFRG vector");
    assert_eq!(opened, plaintext);
}

#[derive(serde::Deserialize)]
struct InteropFixture {
    recipient_public_key_sec1_hex: String,
    recipient_private_key_hex: String,
    nostr_secret_key_hex: String,
    aad_hex: String,
    envelope: HpkeBackupEnvelope,
}

#[test]
fn checked_in_v1_fixture_opens_and_reconstructs_aad() {
    let fixture: InteropFixture =
        serde_json::from_str(include_str!("testdata/hpke_nsec_backup_v1.json"))
            .expect("parse checked-in interop fixture");
    let recipient_public = hex::decode(&fixture.recipient_public_key_sec1_hex).unwrap();
    let recipient_private =
        HpkePrivateKey::from(hex::decode(&fixture.recipient_private_key_hex).unwrap());
    let secret = hex::decode(&fixture.nostr_secret_key_hex).unwrap();

    assert_eq!(recipient_public.len(), P256_PUBLIC_KEY_LEN);
    assert_eq!(
        hex::encode(fixture.envelope.associated_data().unwrap()),
        fixture.aad_hex
    );
    assert_eq!(
        open_envelope(&fixture.envelope, &recipient_private).unwrap(),
        secret
    );
}
