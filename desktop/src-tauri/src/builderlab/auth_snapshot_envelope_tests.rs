//! Real import validate/preview and post-image-generation card preparation phases.
//! Remote IPC/workspace entry stays closed; no NIP-AE memory reader is exercised.
use super::*;
use crate::{
    app_state::build_app_state_for_mode,
    commands::{
        card::prepare_card_bytes,
        snapshot::import::{build_agent_snapshot_import_preview, decode_snapshot_for_import},
    },
    managed_agents::{agent_snapshot::*, agent_snapshot_envelope::*},
    native_identity::SignerMode,
    owner_authorization::OwnerAuthorizationScope,
};

fn manifest() -> AgentSnapshot {
    serde_json::from_value(serde_json::json!({
        "format":"buzz-agent-snapshot", "version":1,
        "definition":{"name":"Envelope bee", "systemPrompt":"private 🐝"},
        "profile":{"displayName":"Envelope bee"},
        "memory":{"level":"none", "entries":[]}
    }))
    .unwrap()
}

#[tokio::test]
async fn snapshot_envelope_real_remote_import_preview_confirm_and_card_phases() {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let owner = OwnerAuthorizationScope::capture(&state).unwrap();
    let agent = Keys::generate();
    let snapshot = manifest();
    let local_png =
        encode_locked_snapshot_png(&snapshot, &api.state.keys, &agent.public_key(), None).unwrap();
    // Actual shared preview/confirmation decoder: rightful remote owner, no record or agent secret.
    let (decoded, locked) = decode_snapshot_for_import(&local_png, Some(&owner), &[], &state)
        .await
        .unwrap();
    assert_eq!(decoded, snapshot);
    let preview = build_agent_snapshot_import_preview(&decoded, locked).unwrap();
    assert!(preview.locked);
    assert_eq!(
        serde_json::from_str::<AgentSnapshot>(&preview.manifest_json).unwrap(),
        snapshot
    );
    let png = prepare_card_bytes(
        &state,
        &snapshot,
        Some((&owner, &agent.public_key())),
        &local_png,
    )
    .await
    .unwrap();
    let ChunkPayload::Locked(envelope) =
        parse_chunk_payload(&extract_chunk_payload_png(&png).unwrap()).unwrap()
    else {
        panic!("not locked")
    };
    assert_eq!(
        decrypt_envelope(&envelope, api.state.keys.secret_key()).unwrap(),
        snapshot
    );
    assert_eq!(
        decrypt_envelope(&envelope, agent.secret_key()).unwrap(),
        snapshot
    );
    assert_eq!(
        decode_snapshot_for_import(&png, Some(&owner), &[], &state)
            .await
            .unwrap()
            .0,
        snapshot
    );
    assert!(state.signing_keys().is_err());
    assert!(state.local_identity_keys().is_err());
    assert_eq!(
        state.identity_storage(),
        crate::identity_storage::IdentityStorage::Absent
    );
    assert!(
        !serde_json::to_value(state.native_identity_status().unwrap()).unwrap()["workspaceActive"]
            .as_bool()
            .unwrap()
    );
    assert!(crate::owner_authorization::require_owned_workspace(&state).is_err());
    let requests = api.state.requests.lock().unwrap();
    let crypto: Vec<_> = requests
        .iter()
        .filter(|r| r.0.ends_with("/encrypt") || r.0.ends_with("/decrypt"))
        .collect();
    assert_eq!(crypto.len(), 4); // import, encrypt, actual final PNG decrypt, confirm
    for request in crypto {
        assert_eq!(request.1.as_deref(), Some("credential-A"));
        assert_eq!(request.2["peer_pubkey"], agent.public_key().to_hex());
    }
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.0 == "v1/auth/login/exchange")
            .count(),
        1
    );
}

#[tokio::test]
async fn snapshot_envelope_local_preparation_counterpart_and_refusal() {
    let state = build_app_state_for_mode(SignerMode::Local);
    let owner = OwnerAuthorizationScope::capture(&state).unwrap();
    let keys = state.local_identity_keys().unwrap();
    let agent = Keys::generate();
    let snapshot = manifest();
    let png = prepare_card_bytes(&state, &snapshot, Some((&owner, &agent.public_key())), &[])
        .await
        .unwrap();
    assert_eq!(
        decode_snapshot_for_import(&png, Some(&owner), &[], &state)
            .await
            .unwrap()
            .0,
        snapshot
    );
    let mut envelope = encrypt_snapshot_envelope(&snapshot, &keys, &agent.public_key()).unwrap();
    envelope.encryption.ciphertext = "malformed".into();
    assert_eq!(
        decode_snapshot_for_import(
            &serde_json::to_vec(&envelope).unwrap(),
            Some(&owner),
            &[],
            &state
        )
        .await
        .unwrap_err(),
        LOCKED_CARD_REFUSAL
    );
}

#[tokio::test]
async fn snapshot_envelope_rejects_structure_endpoints_and_caps_before_http() {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let owner = OwnerAuthorizationScope::capture(&state).unwrap();
    let agent = Keys::generate();
    let envelope =
        encrypt_snapshot_envelope(&manifest(), &api.state.keys, &agent.public_key()).unwrap();
    let before = api.state.requests.lock().unwrap().len();
    for case in 0..10 {
        let mut bad = envelope.clone();
        match case {
            0 => bad.encryption.owner_pubkey = Keys::generate().public_key().to_hex(),
            1 => bad.encryption.agent_pubkey = bad.encryption.owner_pubkey.clone(),
            2 => bad.encryption.agent_pubkey = "f".repeat(64),
            3 => bad.encryption.owner_pubkey = bad.encryption.owner_pubkey.to_uppercase(),
            4 => bad.version = 2,
            5 => bad.encryption.scheme = "nip44-v3".into(),
            6 => bad.format = "buzz-team-snapshot-encrypted".into(),
            7 => bad.encryption.ciphertext = "a".repeat(MAX_LOCKED_CIPHERTEXT_BYTES + 1),
            8 => bad.encryption.ciphertext.clear(),
            _ => std::mem::swap(
                &mut bad.encryption.owner_pubkey,
                &mut bad.encryption.agent_pubkey,
            ),
        }
        assert!(
            decode_snapshot_for_import(
                &serde_json::to_vec(&bad).unwrap(),
                Some(&owner),
                &[],
                &state
            )
            .await
            .is_err(),
            "{case}"
        );
    }
    let oversized = format!(
        "{{\"format\":\"{LOCKED_FORMAT}\",\"padding\":\"{}\"}}",
        "x".repeat(MAX_LOCKED_ENVELOPE_JSON_BYTES)
    );
    assert!(
        decode_snapshot_for_import(oversized.as_bytes(), Some(&owner), &[], &state)
            .await
            .unwrap_err()
            .contains("maximum size")
    );
    let mut snapshot = manifest();
    snapshot.definition.system_prompt = Some("x".repeat(65535));
    assert!(
        prepare_card_bytes(&state, &snapshot, Some((&owner, &agent.public_key())), &[])
            .await
            .unwrap_err()
            .contains("too large to lock")
    );
    assert!(prepare_card_bytes(
        &state,
        &manifest(),
        Some((&owner, &owner.signer.public_key())),
        &[]
    )
    .await
    .unwrap_err()
    .contains("differ"));
    let invalid = nostr::PublicKey::from_hex(&"f".repeat(64)).unwrap();
    assert!(
        prepare_card_bytes(&state, &manifest(), Some((&owner, &invalid)), &[])
            .await
            .is_err()
    );
    assert_eq!(api.state.requests.lock().unwrap().len(), before);
}

#[tokio::test]
async fn snapshot_envelope_decrypted_body_and_memory_consistency_never_return_artifacts() {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let owner = OwnerAuthorizationScope::capture(&state).unwrap();
    let agent = Keys::generate();
    for memory in [false, true] {
        let mut bad = manifest();
        if memory {
            bad.memory.entries.push(AgentSnapshotMemoryEntry {
                slug: "core".into(),
                body: "invalid memory".into(),
            });
        } else {
            bad.version = 99;
        }
        let envelope =
            encrypt_snapshot_envelope(&bad, &api.state.keys, &agent.public_key()).unwrap();
        assert!(decode_snapshot_for_import(
            &serde_json::to_vec(&envelope).unwrap(),
            Some(&owner),
            &[],
            &state
        )
        .await
        .is_err());
        assert!(
            prepare_card_bytes(&state, &bad, Some((&owner, &agent.public_key())), &[])
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn snapshot_envelope_await_invalidation_discards_plaintext_and_final_roundtrip() {
    for phase in ["import", "encrypt", "roundtrip"] {
        for action in ["logout", "expiry", "replace", "relay"] {
            let api = MockApi::new().await;
            let state = build_app_state_for_mode(SignerMode::Remote);
            if action == "expiry" {
                *api.state.expiry.lock().unwrap() =
                    (Utc::now() + chrono::Duration::milliseconds(700)).to_rfc3339();
            }
            api.login(&state.native_auth, "A").await.unwrap();
            let owner = OwnerAuthorizationScope::capture(&state).unwrap();
            let agent = Keys::generate();
            let snapshot = manifest();
            let png =
                encode_locked_snapshot_png(&snapshot, &api.state.keys, &agent.public_key(), None)
                    .unwrap();
            api.hold(if phase == "encrypt" {
                "v1/buzz/identity/encrypt"
            } else {
                "v1/buzz/identity/decrypt"
            });
            let work = async {
                if phase == "import" {
                    decode_snapshot_for_import(&png, Some(&owner), &[], &state)
                        .await
                        .map(|_| ())
                } else {
                    prepare_card_bytes(&state, &snapshot, Some((&owner, &agent.public_key())), &[])
                        .await
                        .map(|_| ())
                }
            };
            let invalidate = async {
                api.arrived().await;
                match action {
                    "logout" => state.native_auth.clear().unwrap(),
                    "replace" => {
                        api.login(&state.native_auth, "B").await.unwrap();
                    }
                    "relay" => {
                        *state.relay_url_override.lock().unwrap() =
                            Some("ws://changed.example".into());
                    }
                    _ => tokio::time::sleep(Duration::from_millis(750)).await,
                }
                api.release();
            };
            let (result, ()) = tokio::time::timeout(Duration::from_secs(4), async {
                tokio::join!(work, invalidate)
            })
            .await
            .unwrap();
            assert!(result.is_err(), "{phase}/{action}");
        }
    }
}

#[tokio::test]
async fn snapshot_envelope_remote_errors_propagate_without_plain_fallback_or_transient_logout() {
    for phase in ["import", "encrypt", "roundtrip"] {
        for status in [200, 401, 403, 503] {
            let api = MockApi::new().await;
            let state = build_app_state_for_mode(SignerMode::Remote);
            api.login(&state.native_auth, "A").await.unwrap();
            let owner = OwnerAuthorizationScope::capture(&state).unwrap();
            let agent = Keys::generate();
            let snapshot = manifest();
            let png =
                encode_locked_snapshot_png(&snapshot, &api.state.keys, &agent.public_key(), None)
                    .unwrap();
            *api.state.fail.lock().unwrap() = Some((
                if phase == "encrypt" {
                    "v1/buzz/identity/encrypt"
                } else {
                    "v1/buzz/identity/decrypt"
                }
                .into(),
                status,
            ));
            let error = if phase == "import" {
                decode_snapshot_for_import(&png, Some(&owner), &[], &state)
                    .await
                    .unwrap_err()
            } else {
                prepare_card_bytes(&state, &snapshot, Some((&owner, &agent.public_key())), &[])
                    .await
                    .unwrap_err()
            };
            assert_ne!(error, LOCKED_CARD_REFUSAL);
            assert!(!error.contains("DO_NOT_LOG_SECRET"));
            assert!(!error.contains("credential-A"));
            assert_eq!(
                owner.signer.check_valid().is_ok(),
                status != 401 && status != 403
            );
        }
    }
}

#[tokio::test]
async fn snapshot_envelope_exact_serialized_byte_limit_and_supplied_memory() {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let owner = OwnerAuthorizationScope::capture(&state).unwrap();
    let agent = Keys::generate();
    let mut snapshot = manifest();
    snapshot.memory.level = MemoryLevel::Core;
    snapshot.memory.entries.push(AgentSnapshotMemoryEntry {
        slug: "core".into(),
        body: "supplied validated memory 🐝".into(),
    });
    snapshot.profile.avatar_url = Some("https://example.test/".into());
    // The inherited rust-nostr encoder (also used by this loopback server)
    // supports 65408 bytes, while server-produced NIP44 supports 65535.
    let padding = 65408 - encode_snapshot_json(&snapshot).unwrap().len();
    snapshot
        .profile
        .avatar_url
        .as_mut()
        .unwrap()
        .push_str(&"a".repeat(padding));
    assert_eq!(encode_snapshot_json(&snapshot).unwrap().len(), 65408);
    let png = prepare_card_bytes(&state, &snapshot, Some((&owner, &agent.public_key())), &[])
        .await
        .unwrap();
    assert_eq!(
        decode_snapshot_for_import(&png, Some(&owner), &[], &state)
            .await
            .unwrap()
            .0,
        snapshot
    );
    // Prove the envelope boundary still admits exactly 65535 serialized bytes
    // to HTTP, without replacing the library cipher to manufacture a fixture.
    snapshot
        .profile
        .avatar_url
        .as_mut()
        .unwrap()
        .push_str(&"a".repeat(127));
    assert_eq!(encode_snapshot_json(&snapshot).unwrap().len(), 65535);
    *api.state.fail.lock().unwrap() = Some(("v1/buzz/identity/encrypt".into(), 503));
    let before = api.state.requests.lock().unwrap().len();
    let error = prepare_card_bytes(&state, &snapshot, Some((&owner, &agent.public_key())), &[])
        .await
        .unwrap_err();
    assert!(!error.contains("too large to lock"));
    assert_eq!(api.state.requests.lock().unwrap().len(), before + 1);
    assert_eq!(
        api.state.requests.lock().unwrap().last().unwrap().2["plaintext"]
            .as_str()
            .unwrap()
            .len(),
        65535
    );
    let before = api.state.requests.lock().unwrap().len();
    snapshot.profile.avatar_url.as_mut().unwrap().push('a');
    assert_eq!(encode_snapshot_json(&snapshot).unwrap().len(), 65536);
    assert!(
        prepare_card_bytes(&state, &snapshot, Some((&owner, &agent.public_key())), &[])
            .await
            .unwrap_err()
            .contains("too large to lock")
    );
    for is_png in [false, true] {
        let mut bytes = vec![
            b'x';
            if is_png {
                10 * 1024 * 1024 + 1
            } else {
                5 * 1024 * 1024 + 1
            }
        ];
        if is_png {
            bytes[..4].copy_from_slice(b"\x89PNG");
        }
        assert!(
            decode_snapshot_for_import(&bytes, Some(&owner), &[], &state)
                .await
                .unwrap_err()
                .contains("too large")
        );
    }
    assert_eq!(api.state.requests.lock().unwrap().len(), before);
}

#[tokio::test]
async fn snapshot_envelope_closed_transport_does_not_logout_or_plain_fallback() {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let owner = OwnerAuthorizationScope::capture(&state).unwrap();
    let agent = Keys::generate();
    let snapshot = manifest();
    let png =
        encode_locked_snapshot_png(&snapshot, &api.state.keys, &agent.public_key(), None).unwrap();
    api.server.abort();
    while !api.server.is_finished() {
        tokio::task::yield_now().await;
    }
    assert!(decode_snapshot_for_import(&png, Some(&owner), &[], &state)
        .await
        .is_err());
    assert!(
        prepare_card_bytes(&state, &snapshot, Some((&owner, &agent.public_key())), &[])
            .await
            .is_err()
    );
    assert!(owner.signer.check_valid().is_ok());
}
