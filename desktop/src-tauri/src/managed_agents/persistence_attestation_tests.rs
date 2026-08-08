use super::*;
use crate::secret_store::KeyringProbe;
const PUBKEY: &str = "b7c6f2f6e0a94d5f8f2f0c8f4e9a1b2c3d4e5f60718293a4b5c6d7e8f9012ab";

fn inputs(inline: bool, probe: Option<KeyringProbe>) -> AttestationInputs<'static> {
    AttestationInputs {
        agent_pubkey: PUBKEY,
        auth_tag: Some(r#"{"kind":"nip-oa","sig":"public"}"#),
        inline_key_present: inline,
        keyring_probe: probe,
        parallelism: 1,
        stock_release_id: "buzz-desktop@0.5.7",
        issued_at: "2026-08-08T12:00:00Z",
    }
}

#[test]
fn keyring_backed_agent_attests_os_keyring_without_inline_fallback() {
    let attestation =
        build_agent_persistence_attestation(&inputs(false, Some(KeyringProbe::Present)))
            .expect("attestation");
    assert_eq!(
        attestation.persistence_backend,
        PersistenceBackend::OsKeyring
    );
    assert!(!attestation.inline_fallback);
    assert_eq!(
        attestation.schema_version,
        AGENT_PERSISTENCE_ATTESTATION_SCHEMA_V1
    );
    assert!(verify_attestation_hash(&attestation));
}

#[test]
fn inline_key_attests_inline_file_regardless_of_probe() {
    for probe in [
        Some(KeyringProbe::Present),
        Some(KeyringProbe::ReachableButEmpty),
        Some(KeyringProbe::Unreachable),
        None,
    ] {
        let attestation =
            build_agent_persistence_attestation(&inputs(true, probe)).expect("inline attestation");
        assert_eq!(
            attestation.persistence_backend,
            PersistenceBackend::InlineFile
        );
        assert!(attestation.inline_fallback);
    }
}

#[test]
fn missing_credential_fails_closed() {
    let error =
        build_agent_persistence_attestation(&inputs(false, Some(KeyringProbe::ReachableButEmpty)))
            .expect_err("must fail");
    assert_eq!(error, "attestation_credential_missing");
    let error = build_agent_persistence_attestation(&inputs(false, None))
        .expect_err("must fail without keyring backend");
    assert_eq!(error, "attestation_credential_missing");
}

#[test]
fn unreachable_keyring_fails_closed_instead_of_guessing() {
    let error =
        build_agent_persistence_attestation(&inputs(false, Some(KeyringProbe::Unreachable)))
            .expect_err("must fail");
    assert_eq!(error, "attestation_keyring_unreachable");
}

#[test]
fn attestation_hash_binds_the_payload() {
    let attestation =
        build_agent_persistence_attestation(&inputs(false, Some(KeyringProbe::Present)))
            .expect("attestation");
    assert!(verify_attestation_hash(&attestation));
    let mut tampered = attestation.clone();
    tampered.parallelism = 8;
    assert!(!verify_attestation_hash(&tampered));
    let mut substituted = attestation;
    substituted.agent_pubkey =
        "0000000000000000000000000000000000000000000000000000000000000000".to_string();
    assert!(!verify_attestation_hash(&substituted));
}

#[test]
fn public_identity_hash_tracks_pubkey_and_auth_tag() {
    let with_tag = build_agent_persistence_attestation(&inputs(false, Some(KeyringProbe::Present)))
        .expect("attestation");
    let mut no_tag_inputs = inputs(false, Some(KeyringProbe::Present));
    no_tag_inputs.auth_tag = None;
    let without_tag = build_agent_persistence_attestation(&no_tag_inputs).expect("attestation");
    assert_ne!(
        with_tag.public_identity_hash,
        without_tag.public_identity_hash
    );
}

#[test]
fn serialized_attestation_exposes_only_the_public_schema_fields() {
    let attestation =
        build_agent_persistence_attestation(&inputs(false, Some(KeyringProbe::Present)))
            .expect("attestation");
    let value: serde_json::Value =
        serde_json::to_value(&attestation).expect("serialize attestation");
    let object = value.as_object().expect("attestation is an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "agent_pubkey",
            "attestation_hash",
            "inline_fallback",
            "issued_at",
            "parallelism",
            "persistence_backend",
            "public_identity_hash",
            "schema_version",
            "stock_release_id",
        ]
    );
    let serialized = value.to_string();
    assert!(!serialized.contains("nsec"));
}
