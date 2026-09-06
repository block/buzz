use super::*;
use crate::app_state::build_app_state;
use std::sync::atomic::Ordering;

fn app_with_policy(
    policy: Result<DeviceAgentPolicy, String>,
) -> tauri::App<tauri::test::MockRuntime> {
    let state = build_app_state();
    state.agent_device_policy.set(policy).unwrap();
    // A missing guard must fail before filesystem/network access in these tests.
    state.keyring_locked.store(true, Ordering::Release);
    tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap()
}

#[test]
fn client_only_native_key_generation_refuses_before_minting() {
    let app = app_with_policy(Ok(DeviceAgentPolicy {
        client_only: true,
        ..Default::default()
    }));
    let error = generate_agent_keys(app.handle(), "Notebook", None).unwrap_err();
    assert!(error.contains("client-only"));
}

#[test]
fn invalid_policy_cannot_fall_back_to_key_generation() {
    let app = app_with_policy(Err("policy unreadable".into()));
    assert_eq!(
        generate_agent_keys(app.handle(), "Notebook", None).unwrap_err(),
        "policy unreadable"
    );
    assert!(is_client_only(app.handle()));
}

#[test]
fn hosting_native_key_generation_still_works() {
    let app = app_with_policy(Ok(DeviceAgentPolicy::default()));
    assert!(generate_agent_keys(app.handle(), "Notebook", None).is_ok());
}

#[test]
fn archive_guard_protects_remote_hex_and_npub_but_allows_other_identities() {
    use nostr::ToBech32;
    let remote = nostr::Keys::generate().public_key();
    let mut policy = DeviceAgentPolicy {
        unique_names: true,
        ..Default::default()
    };
    policy.preferred_agents.push(model::PreferredAgent {
        name: "Scout".into(),
        pubkey: remote.to_hex(),
        owner_pubkey: "owner".into(),
        relay_url: "https://relay.example".into(),
        persona_id: None,
    });
    let app = app_with_policy(Ok(policy));
    assert!(require_identity_archive(app.handle(), &remote.to_hex()).is_err());
    assert!(require_identity_archive(app.handle(), &remote.to_bech32().unwrap()).is_err());
    assert!(
        require_identity_archive(app.handle(), &nostr::Keys::generate().public_key().to_hex())
            .is_ok()
    );
}

#[test]
fn unique_name_native_mint_allows_new_name_and_refuses_reserved_identity() {
    let policy: DeviceAgentPolicy = serde_json::from_str(
        r#"{
        "client_only":false,"unique_names":true,"preferred_agents":[{
        "relay_url":"https://relay.example","owner_pubkey":"owner","name":"Scout",
        "pubkey":"remote-key","persona_id":"remote-persona"}]}"#,
    )
    .unwrap();
    let app = app_with_policy(Ok(policy));
    assert!(generate_agent_keys(app.handle(), "Notebook", None).is_ok());
    assert!(generate_agent_keys(app.handle(), " scout ", None)
        .unwrap_err()
        .contains("another device"));
    assert!(generate_agent_keys(app.handle(), "Renamed", Some("remote-persona")).is_err());
    assert!(!is_client_only(app.handle()));
    assert!(pauses_sync(app.handle()));
}

#[tokio::test]
async fn client_only_skips_actual_pending_flush_without_requesting_signing_keys() {
    let app = app_with_policy(Ok(DeviceAgentPolicy {
        client_only: true,
        ..Default::default()
    }));
    let result = crate::managed_agents::persona_events::flush_active_pending_events(
        app.handle(),
        &app.state(),
    )
    .await;
    assert_eq!(result, Ok(0));
    let host = app_with_policy(Ok(DeviceAgentPolicy::default()));
    assert!(
        crate::managed_agents::persona_events::flush_active_pending_events(
            host.handle(),
            &host.state()
        )
        .await
        .is_err()
    );
}
