//! The real project owner selector and scoped announcement publisher.
use super::*;
use crate::active_user_signer::tests::ControlledSigner;
use base64::Engine;
use std::sync::Arc;

fn input(owner: String) -> ProjectOwnerAnnouncementInput {
    ProjectOwnerAnnouncementInput {
        target_owner: owner,
        kind: 30617,
        content: "repository".into(),
        created_at: Some(123),
        tags: vec![vec!["d".into(), "repo".into()]],
    }
}

#[tokio::test]
async fn live_project_user_signer_delay_and_failure_preserve_recovery_contract() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    for fail in [false, true] {
        let controlled = ControlledSigner::new(fail);
        let state = Arc::new(crate::app_state::build_app_state());
        *state.test_signer.lock().unwrap() =
            Some(ActiveUserSigner::new(controlled.clone()).await.unwrap());
        *state.relay_url_override.lock().unwrap() = Some("http://127.0.0.1:1".into());
        let owner = controlled.keys.public_key().to_hex();
        let identity = project_owner_identity_with(&state, &owner, || {
            panic!("direct owner must not load agents")
        })
        .unwrap();
        assert!(identity.credential_keys.is_none());
        let task_state = state.clone();
        let task = tokio::spawn(async move {
            publish_owner_announcement_in_scope(input(owner), &task_state, identity).await
        });
        controlled.wait_entered().await;
        assert!(state.managed_agents_store_lock.try_lock().is_ok());
        state.replace_local_identity_keys(Keys::generate()).unwrap();
        *state.test_signer.lock().unwrap() = None;
        controlled.release.notify_one();
        if fail {
            assert!(task
                .await
                .unwrap()
                .err()
                .unwrap()
                .contains("deliberate signer failure"));
        } else {
            controlled.wait_entered().await;
            controlled.release.notify_one();
            let result = task.await.unwrap().unwrap();
            assert!(result.publication_error.is_some()); // signed bytes survive relay failure
            let event = Event::from_json(result.event).unwrap();
            event.verify().unwrap();
            assert_eq!(event.pubkey, controlled.keys.public_key());
            assert_eq!(event.created_at.as_secs(), 123);
            assert_eq!(event.kind.as_u16(), 30617);
            assert_eq!(event.content, "repository");
        }
    }
}

#[tokio::test]
async fn live_project_managed_owner_does_not_invoke_failing_active_signer() {
    use axum::{body::Bytes, http::HeaderMap, routing::post, Router};
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let router = Router::new().route("/events", post(move |headers: HeaderMap, body: Bytes| {
        let tx = tx.clone();
        async move {
            let event = Event::from_json(&body).unwrap();
            tx.send((headers, event.clone())).await.unwrap();
            axum::Json(serde_json::json!({"event_id": event.id.to_hex(), "accepted":true,"message":""}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let controlled = ControlledSigner::new(true);
    let state = crate::app_state::build_app_state();
    *state.test_signer.lock().unwrap() =
        Some(ActiveUserSigner::new(controlled.clone()).await.unwrap());
    *state.relay_url_override.lock().unwrap() = Some(base.clone());
    let managed = Keys::generate();
    let owner = managed.public_key().to_hex();
    let record = serde_json::from_value(serde_json::json!({
        "pubkey": owner, "name":"repo-owner", "private_key_nsec": managed.secret_key().to_secret_hex(),
        "auth_tag":"stored-owner-attestation", "relay_url":base,
        "acp_command":"buzz-acp", "agent_command":"goose", "agent_args":[],
        "mcp_command":"", "turn_timeout_seconds": 300, "created_at":"", "updated_at":""
    })).unwrap();
    let identity = project_owner_identity_with(&state, &owner, || Ok(vec![record])).unwrap();
    assert_eq!(identity.signer.public_key(), managed.public_key());
    *state.relay_url_override.lock().unwrap() = Some("http://127.0.0.1:1".into());
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        publish_owner_announcement_in_scope(input(owner), &state, identity),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(result.publication_error.is_none());
    let (headers, event) = rx.recv().await.unwrap();
    event.verify().unwrap();
    assert_eq!(event.pubkey, managed.public_key());
    assert_eq!(headers["x-auth-tag"], "stored-owner-attestation");
    let auth = Event::from_json(
        base64::engine::general_purpose::STANDARD
            .decode(
                headers["authorization"]
                    .to_str()
                    .unwrap()
                    .strip_prefix("Nostr ")
                    .unwrap(),
            )
            .unwrap(),
    )
    .unwrap();
    auth.verify().unwrap();
    assert_eq!(auth.pubkey, event.pubkey);
    assert!(auth
        .tags
        .iter()
        .any(|t| t.as_slice() == ["u", &format!("{base}/events")]));
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(20),
        controlled.entered.notified()
    )
    .await
    .is_err());
    server.abort();
}
