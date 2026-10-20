//! Opt-in journey against a disposable local relay, using the production IPC commands.
use crate::app_state::{build_app_state, AppState};
use crate::events;
use crate::relay::submit_signed_event_at_with_keys;
use nostr::{Event, Keys};
use serde_json::json;
use tauri::{test::MockRuntime, Manager};

use super::thread_catch_up::generate_with;
use super::thread_catch_up_artifacts::{
    correct_thread_catch_up, list_thread_catch_up_artifacts, share_thread_catch_up,
};
use super::thread_catch_up_process::AcpLaunch;
use tokio_util::sync::CancellationToken;

fn local_relay() -> String {
    let value = std::env::var("BUZZ_CATCH_UP_TEST_RELAY")
        .expect("set BUZZ_CATCH_UP_TEST_RELAY to an isolated local relay");
    let url = url::Url::parse(&value).unwrap();
    assert_eq!(url.scheme(), "http");
    assert!(matches!(url.host_str(), Some("127.0.0.1" | "[::1]")));
    assert!(url.username().is_empty() && url.password().is_none());
    value.trim_end_matches('/').to_string()
}

fn app_for(keys: Keys, relay: &str) -> tauri::App<MockRuntime> {
    let state = build_app_state();
    *state.keys.lock().unwrap() = keys;
    *state.relay_url_override.lock().unwrap() = Some(relay.replacen("http", "ws", 1));
    tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap()
}

async fn publish(state: &AppState, relay: &str, event: &Event, keys: &Keys) {
    submit_signed_event_at_with_keys(event, state, relay, keys)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires a disposable local relay at BUZZ_CATCH_UP_TEST_RELAY"]
async fn thread_catch_up_live_generate_share_correct_conflict_and_reload() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let relay = local_relay();
    let owner_keys = Keys::generate();
    let peer_keys = Keys::generate();
    let owner_app = app_for(owner_keys.clone(), &relay);
    let peer_app = app_for(peer_keys.clone(), &relay);
    let owner = owner_app.state::<AppState>();
    let peer = peer_app.state::<AppState>();
    let channel = uuid::Uuid::new_v4();

    let created = events::build_create_channel(
        channel,
        &format!("catch-up-test-{}", channel.simple()),
        "private",
        "stream",
        None,
        None,
    )
    .unwrap()
    .sign_with_keys(&owner_keys)
    .unwrap();
    publish(&owner, &relay, &created, &owner_keys).await;
    let added = events::build_add_member(channel, &peer_keys.public_key().to_hex(), None)
        .unwrap()
        .sign_with_keys(&owner_keys)
        .unwrap();
    publish(&owner, &relay, &added, &owner_keys).await;

    let root = events::build_message(
        channel,
        "Should we publish the synthetic example on Monday? Is the budget approved?",
        None,
        &[],
        &[],
        &[],
        &[],
        &[],
        None,
        &relay,
    )
    .unwrap()
    .sign_with_keys(&owner_keys)
    .unwrap();
    publish(&owner, &relay, &root, &owner_keys).await;
    let reply = events::build_message(
        channel,
        "Decision: publish on Monday only if review passes. Budget approval is still pending.",
        Some(&events::ThreadRef {
            root_event_id: root.id,
            parent_event_id: root.id,
        }),
        &[],
        &[],
        &[],
        &[],
        &[],
        None,
        &relay,
    )
    .unwrap()
    .sign_with_keys(&owner_keys)
    .unwrap();
    publish(&owner, &relay, &reply, &owner_keys).await;
    let scope = |keys: &Keys| {
        json!({
            "relayUrl": relay,
            "signerPubkey": keys.public_key().to_hex(),
            "channelId": channel,
            "rootEventId": root.id.to_hex()
        })
    };
    let program = std::path::PathBuf::from(
        std::env::var("BUZZ_CATCH_UP_TEST_AGENT").expect("set the freshly built buzz-agent path"),
    );
    assert!(program.is_absolute() && program.is_file());
    let launch = AcpLaunch {
        program,
        base_url: "http://127.0.0.1:11434/v1".into(),
        model: std::env::var("BUZZ_CATCH_UP_TEST_MODEL").unwrap_or_else(|_| "qwen3:4b".into()),
    };
    let generated = generate_with(
        &owner,
        &serde_json::from_value(scope(&owner_keys)).unwrap(),
        root.created_at.as_secs().saturating_sub(60),
        nostr::Timestamp::now().as_secs(),
        &launch,
        &CancellationToken::new(),
        std::time::Duration::from_secs(90),
    )
    .await
    .unwrap();
    let point = generated
        .points
        .iter()
        .find(|point| point.text.to_lowercase().contains("monday"))
        .expect("the model should include the explicit decision");
    assert!(point.text.to_lowercase().contains("review"));
    let point_id = point.id.clone();
    assert!(!generated.source_revisions.is_empty());
    let recap = serde_json::to_value(generated).unwrap();
    let shared = share_thread_catch_up(
        serde_json::from_value(json!({"scope":scope(&owner_keys),"recap":recap})).unwrap(),
        owner.clone(),
    )
    .await
    .unwrap();
    let original_event_id = shared.event_id.clone();
    // A new timestamp forces real relay identity-conflict recovery rather
    // than the identical-event replay path.
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    let retried = share_thread_catch_up(
        serde_json::from_value(json!({"scope":scope(&owner_keys),"recap":recap})).unwrap(),
        owner.clone(),
    )
    .await
    .unwrap();
    assert_eq!(retried.event_id, original_event_id);
    let corrected = correct_thread_catch_up(
        serde_json::from_value(json!({
            "scope":scope(&peer_keys),"artifactId":shared.artifact_id,
            "prevEventId":original_event_id,"pointId":point_id,
            "text":"Monday is conditional on completing review."
        }))
        .unwrap(),
        peer.clone(),
    )
    .await
    .unwrap();
    assert_eq!(corrected.corrections.len(), 1);
    assert_eq!(
        corrected.corrections[0].author_pubkey,
        peer_keys.public_key().to_hex()
    );
    assert_eq!(serde_json::to_value(&corrected.recap).unwrap(), recap);

    let stale = correct_thread_catch_up(
        serde_json::from_value(json!({
            "scope":scope(&owner_keys),"artifactId":corrected.artifact_id,
            "prevEventId":original_event_id,"pointId":point_id,"text":"Stale draft"
        }))
        .unwrap(),
        owner.clone(),
    )
    .await;
    assert!(stale.err().unwrap().starts_with("conflict:"));

    let reloaded_app = app_for(owner_keys.clone(), &relay);
    let reloaded = list_thread_catch_up_artifacts(
        serde_json::from_value(json!({"scope":scope(&owner_keys)})).unwrap(),
        reloaded_app.state(),
    )
    .await
    .unwrap();
    assert_eq!(reloaded.recaps.len(), 1);
    assert_eq!(reloaded.recaps[0].event_id, corrected.event_id);
    assert_eq!(reloaded.recaps[0].corrections.len(), 1);
    assert!(reloaded.recaps[0].history_complete);
    assert!(reloaded.complete);
    assert_eq!(reloaded.unreadable_count, 0);

    let outsider_keys = Keys::generate();
    let outsider_app = app_for(outsider_keys.clone(), &relay);
    let hidden = list_thread_catch_up_artifacts(
        serde_json::from_value(json!({"scope":scope(&outsider_keys)})).unwrap(),
        outsider_app.state(),
    )
    .await;
    assert!(hidden.map_or(true, |items| items.recaps.is_empty()));
}
