//! Exercise the publication helper used by create and snapshot import, not a
//! parallel relay-choice model. A live workspace read would hit the decoy relay.
use super::*;
use axum::{body::Bytes, http::StatusCode, routing::post, Router};
use nostr::JsonUtil;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn captured_profile_destination_survives_identity_and_relay_switch() {
    for rejected in [false, true] {
        let events = Arc::new(Mutex::new(Vec::<nostr::Event>::new()));
        let received = events.clone();
        let router = Router::new().route(
            "/events",
            post(move |body: Bytes| {
                let received = received.clone();
                async move {
                    received
                        .lock()
                        .unwrap()
                        .push(nostr::Event::from_json(body).unwrap());
                    if rejected {
                        StatusCode::FORBIDDEN
                    } else {
                        StatusCode::OK
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let captured = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let state = crate::app_state::build_app_state();
        state
            .install_local_workspace(captured.clone(), Some(nostr::Keys::generate()))
            .unwrap();
        let owner = crate::owner_authorization::OwnerAuthorizationScope::capture(&state).unwrap();
        let agent = nostr::Keys::generate();
        state
            .install_local_workspace("http://127.0.0.1:1".into(), Some(nostr::Keys::generate()))
            .unwrap();
        assert!(owner.check_current(&state).is_err());
        let persona: crate::managed_agents::AgentDefinition =
            serde_json::from_value(serde_json::json!({
                "id": "imported", "display_name": "Imported", "system_prompt": "",
                "description": "Imported description", "created_at": "", "updated_at": ""
            }))
            .unwrap();
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            publish_persona_profile(
                &state,
                &owner.relay_base,
                &agent,
                "Imported",
                None,
                &persona,
                None,
            ),
        )
        .await
        .unwrap();
        server.abort();
        assert_eq!(error.is_some(), rejected, "{error:?}");
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        event.verify().unwrap();
        assert_eq!(event.pubkey, agent.public_key());
        assert_eq!(event.kind, nostr::Kind::Metadata);
        let content: serde_json::Value = serde_json::from_str(&event.content).unwrap();
        assert_eq!(content["about"], "Imported description");
    }
}
