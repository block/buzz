//! Bind the registered commands and the defensive human-only audio entry.
use super::*;
use crate::{app_state::build_app_state_for_mode, huddle, native_identity::SignerMode};
use tauri::Manager;

#[tokio::test]
async fn remote_human_audio_and_observer_commands_fail_before_any_effect() {
    for authenticated in [false, true] {
        let api = MockApi::new().await;
        let state = build_app_state_for_mode(SignerMode::Remote);
        // A separate listening socket detects relay/audio attempts without any
        // external relay/model service. Guarded commands must never contact it.
        let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
        *state.relay_url_override.lock().unwrap() =
            Some(format!("ws://{}", relay.local_addr().unwrap()));
        if authenticated {
            api.login(&state.native_auth, "A").await.unwrap();
            assert!(state.active_signer().is_ok());
        }
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let before = api.state.requests.lock().unwrap().len();
        let status = app.state::<crate::AppState>().native_auth.status().unwrap();
        let start = huddle::start_huddle(
            "parent".into(),
            vec![],
            None,
            app.handle().clone(),
            app.state(),
        )
        .await;
        assert!(start.unwrap_err().contains("generation-owned audio"));
        let join = huddle::join_huddle("parent".into(), "room".into(), None, app.state()).await;
        assert!(join.unwrap_err().contains("generation-owned audio"));
        let audio =
            huddle::relay_api::connect_audio_relay("room", Some("parent"), &app.state()).await;
        assert!(audio.unwrap_err().contains("generation-owned audio"));
        let observer = crate::commands::build_observer_control_event(
            Keys::generate().public_key().to_hex(),
            serde_json::json!({"action":"stop"}),
            app.state(),
            None,
        )
        .await;
        assert!(observer.unwrap_err().contains(if authenticated {
            "current native identity generation"
        } else {
            "signed out"
        }));
        let state = app.state::<crate::AppState>();
        {
            let hs = state.huddle().unwrap();
            assert_eq!(hs.phase, huddle::HuddlePhase::Idle);
            assert_eq!(hs.huddle_generation, 0);
            assert!(hs.ephemeral_channel_id.is_none());
            assert!(hs.parent_channel_id.is_none());
            assert!(hs.participants.is_empty());
            assert_eq!(state.native_auth.status().unwrap(), status);
            assert_eq!(api.state.requests.lock().unwrap().len(), before);
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(25), relay.accept())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn local_start_join_and_observer_still_reach_existing_logic() {
    let state = build_app_state_for_mode(SignerMode::Local);
    state.huddle().unwrap().phase = huddle::HuddlePhase::Connecting;
    let owner = state.identity_public_key().unwrap();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    assert!(huddle::start_huddle(
        "parent".into(),
        vec![],
        None,
        app.handle().clone(),
        app.state()
    )
    .await
    .unwrap_err()
    .contains("already in phase"));
    assert!(
        huddle::join_huddle("parent".into(), "room".into(), None, app.state())
            .await
            .unwrap_err()
            .contains("already in phase")
    );
    let agent = Keys::generate();
    let payload = serde_json::json!({"action":"stop"});
    let event = crate::commands::build_observer_control_event(
        agent.public_key().to_hex(),
        payload.clone(),
        app.state(),
        None,
    )
    .await
    .unwrap();
    let event = nostr::Event::from_json(event).unwrap();
    event.verify().unwrap();
    assert_eq!(event.pubkey, owner);
    let plaintext = nostr::nips::nip44::decrypt(agent.secret_key(), &owner, event.content).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&plaintext).unwrap(),
        payload
    );
}

#[tokio::test]
async fn local_human_and_independent_agent_audio_still_authenticate_on_actual_socket() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    for human in [true, false] {
        let state = build_app_state_for_mode(if human {
            SignerMode::Local
        } else {
            SignerMode::Remote
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        *state.relay_url_override.lock().unwrap() =
            Some(format!("ws://{}", listener.local_addr().unwrap()));
        let agent = Keys::generate();
        let expected = if human {
            state.identity_public_key().unwrap()
        } else {
            agent.public_key()
        };
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.send(Message::Text(
                serde_json::json!({"type":"challenge", "challenge":"local-fixture"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            let wire = ws.next().await.unwrap().unwrap().into_text().unwrap();
            let auth: serde_json::Value = serde_json::from_str(&wire).unwrap();
            let event: nostr::Event = serde_json::from_value(auth["event"].clone()).unwrap();
            event.verify().unwrap();
            assert_eq!(event.pubkey, expected);
            assert_eq!(event.kind.as_u16(), 22242);
            // Stop before pipelines/playback; the real admission and NIP-42
            // handshake have run. No microphone/model service is involved.
            ws.send(Message::Text(
                serde_json::json!({"type":"error", "message":"fixture-stop"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        });
        let result = if human {
            huddle::relay_api::connect_audio_relay("room", Some("parent"), &state)
                .await
                .map(|_| ())
        } else {
            let publishers = state.huddle().unwrap().local_tts_publishers.clone();
            huddle::relay_api::connect_tts_audio_publisher(
                "room",
                Some("parent"),
                &state,
                &agent,
                None,
                publishers,
            )
            .await
            .map(|_| ())
        };
        assert!(result.unwrap_err().contains("fixture-stop"));
        server.await.unwrap();
    }
}
