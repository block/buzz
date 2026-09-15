//! Locked-card import tests for `decode_snapshot_for_import`.
//!
//! Kept in a sibling file so `snapshot/tests.rs` stays under the
//! 1500-line gate; `#[path]`-included from there as a child module,
//! so `super::*` still resolves to the shared test helpers.

use super::*;
use crate::commands::personas::snapshot::import::{
    decode_snapshot_for_import, parse_snapshot_payload_from_bytes,
};
use crate::managed_agents::agent_snapshot_envelope::{
    encode_locked_snapshot_png, encrypt_snapshot_envelope, ChunkPayload, LOCKED_CARD_REFUSAL,
};

/// Build a keyed instance record holding real key material, so the
/// agent-endpoint unlock path resolves exactly as production does.
fn record_for(agent: &nostr::Keys) -> ManagedAgentRecord {
    ManagedAgentRecord {
        session_policy: Default::default(),
        pubkey: agent.public_key().to_hex(),
        slug: None,
        persona_id: Some("locked-test".to_string()),
        private_key_nsec: nostr::ToBech32::to_bech32(agent.secret_key()).unwrap(),
        ..make_definition("")
    }
}

fn locked_png(owner: &nostr::Keys, agent: &nostr::Keys) -> (AgentSnapshot, Vec<u8>) {
    let snapshot = make_snapshot(MemoryLevel::None, vec![]);
    let png = encode_locked_snapshot_png(&snapshot, owner, &agent.public_key(), None).unwrap();
    (snapshot, png)
}

/// Owner identity key unlocks a locked card; `locked` is reported true.
#[tokio::test]
async fn owner_endpoint_unlocks_locked_png() {
    let state = crate::app_state::build_app_state();
    let scope = crate::owner_authorization::OwnerAuthorizationScope::capture(&state).unwrap();
    let owner = state.local_identity_keys().unwrap();
    let agent = nostr::Keys::generate();
    let (snapshot, png) = locked_png(&owner, &agent);
    let (decoded, locked) = decode_snapshot_for_import(&png, Some(&scope), &[], &state)
        .await
        .unwrap();
    assert_eq!(decoded, snapshot);
    assert!(locked);
}

/// A local managed-agent record holding the agent nsec unlocks the card
/// even when the owner identity does not match (e.g. re-import on the
/// agent's own machine under a different owner identity).
#[tokio::test]
async fn agent_record_endpoint_unlocks_locked_png() {
    let state = crate::app_state::build_app_state();
    let owner = state.local_identity_keys().unwrap();
    let agent = nostr::Keys::generate();
    let (snapshot, png) = locked_png(&owner, &agent);
    let other_state = crate::app_state::build_app_state();
    other_state
        .replace_local_identity_keys(nostr::Keys::generate())
        .unwrap();
    let other_scope =
        crate::owner_authorization::OwnerAuthorizationScope::capture(&other_state).unwrap();
    let records = vec![record_for(&agent)];
    let (decoded, locked) =
        decode_snapshot_for_import(&png, Some(&other_scope), &records, &other_state)
            .await
            .unwrap();
    assert_eq!(decoded, snapshot);
    assert!(locked);
}

/// No matching endpoint → only the locked-card refusal, nothing else.
#[tokio::test]
async fn stranger_fails_closed_with_refusal_only() {
    let state = crate::app_state::build_app_state();
    let owner = state.local_identity_keys().unwrap();
    let agent = nostr::Keys::generate();
    let (_snapshot, png) = locked_png(&owner, &agent);
    let stranger_state = crate::app_state::build_app_state();
    stranger_state
        .replace_local_identity_keys(nostr::Keys::generate())
        .unwrap();
    let stranger =
        crate::owner_authorization::OwnerAuthorizationScope::capture(&stranger_state).unwrap();
    let unrelated_record = record_for(&nostr::Keys::generate());
    let err =
        decode_snapshot_for_import(&png, Some(&stranger), &[unrelated_record], &stranger_state)
            .await
            .unwrap_err();
    assert_eq!(err, LOCKED_CARD_REFUSAL);
    // And with no key material at all.
    let err = decode_snapshot_for_import(&png, None, &[], &state)
        .await
        .unwrap_err();
    assert_eq!(err, LOCKED_CARD_REFUSAL);
}

/// Plain snapshots pass through unchanged with `locked == false`, with or
/// without key material in scope.
#[tokio::test]
async fn plain_snapshot_passes_through_unlocked() {
    use crate::managed_agents::agent_snapshot::encode_snapshot_png;
    let snapshot = make_snapshot(MemoryLevel::None, vec![]);
    let png = encode_snapshot_png(&snapshot, None).unwrap();
    let state = crate::app_state::build_app_state();
    let scope = crate::owner_authorization::OwnerAuthorizationScope::capture(&state).unwrap();
    let (decoded, locked) = decode_snapshot_for_import(&png, Some(&scope), &[], &state)
        .await
        .unwrap();
    assert_eq!(decoded, snapshot);
    assert!(!locked);
    let (decoded, locked) = decode_snapshot_for_import(&png, None, &[], &state)
        .await
        .unwrap();
    assert_eq!(decoded, snapshot);
    assert!(!locked);
}

/// The memory-consistency guard fires AFTER decryption too: a locked
/// envelope whose plaintext declares level none + non-empty entries is
/// rejected even for a legitimate endpoint.
#[tokio::test]
async fn decrypted_manifest_memory_consistency_enforced() {
    let state = crate::app_state::build_app_state();
    let scope = crate::owner_authorization::OwnerAuthorizationScope::capture(&state).unwrap();
    let owner = state.local_identity_keys().unwrap();
    let agent = nostr::Keys::generate();
    let malformed = make_snapshot(
        MemoryLevel::None,
        vec![AgentSnapshotMemoryEntry {
            slug: "core".to_string(),
            body: "leaked".to_string(),
        }],
    );
    // encrypt_snapshot_envelope does not guard memory consistency (the
    // PNG encoder does), so this constructs the malicious payload.
    let envelope = encrypt_snapshot_envelope(&malformed, &owner, &agent.public_key()).unwrap();
    let json = serde_json::to_vec(&envelope).unwrap();
    let err = decode_snapshot_for_import(&json, Some(&scope), &[], &state)
        .await
        .unwrap_err();
    assert!(
        err.contains("'none' but entries are present"),
        "post-decrypt consistency guard must fire, got: {err}"
    );
}

/// Transit validation (`fetch_snapshot_bytes` path) accepts a locked PNG
/// without any key material — structural validation only, no decryption.
#[tokio::test]
async fn transit_validation_accepts_locked_png_without_keys() {
    let state = crate::app_state::build_app_state();
    let owner = state.local_identity_keys().unwrap();
    let agent = nostr::Keys::generate();
    let (_snapshot, png) = locked_png(&owner, &agent);
    let payload = parse_snapshot_payload_from_bytes(&png).unwrap();
    assert!(matches!(payload, ChunkPayload::Locked(_)));
}

/// The keyless plain decoder refuses locked cards with the refusal.
#[tokio::test]
async fn plain_decoder_refuses_locked_cards() {
    let state = crate::app_state::build_app_state();
    let owner = state.local_identity_keys().unwrap();
    let agent = nostr::Keys::generate();
    let (_snapshot, png) = locked_png(&owner, &agent);
    let err = decode_snapshot_from_bytes(&png).unwrap_err();
    assert_eq!(err, LOCKED_CARD_REFUSAL);
}

#[tokio::test]
async fn local_recovery_still_allows_only_independent_agent_unlock_or_plain() {
    let state = crate::app_state::build_app_state();
    let owner = state.local_identity_keys().unwrap();
    let agent = nostr::Keys::generate();
    let (snapshot, png) = locked_png(&owner, &agent);
    let record = record_for(&agent);
    for flag in [&state.identity_lost, &state.keyring_locked] {
        flag.store(true, std::sync::atomic::Ordering::Release);
        assert!(crate::owner_authorization::OwnerAuthorizationScope::capture(&state).is_err());
        assert!(decode_snapshot_for_import(&png, None, &[], &state)
            .await
            .is_err());
        assert_eq!(
            decode_snapshot_for_import(&png, None, std::slice::from_ref(&record), &state)
                .await
                .unwrap()
                .0,
            snapshot
        );
        let plain = crate::managed_agents::agent_snapshot::encode_snapshot_json(&snapshot).unwrap();
        assert_eq!(
            decode_snapshot_for_import(&plain, None, &[], &state)
                .await
                .unwrap()
                .0,
            snapshot
        );
        flag.store(false, std::sync::atomic::Ordering::Release);
    }
}
