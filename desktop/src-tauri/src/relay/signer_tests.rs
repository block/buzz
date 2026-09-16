use super::*;
use crate::active_user_signer::tests::ControlledSigner;
use std::sync::{atomic::Ordering, Arc};

#[tokio::test]
async fn scoped_submit_keeps_identity_relay_and_nip98_template_across_await() {
    use axum::{body::Bytes, http::HeaderMap, routing::post, Router};
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    crate::relay_admission::activate_rate_limit(Some(2));
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let app = Router::new().route("/events", post(move |headers: HeaderMap, body: Bytes| {
        let tx = tx.clone();
        async move {
            let event = nostr::Event::from_json(&body).unwrap();
            tx.send((headers, body)).await.unwrap();
            axum::Json(serde_json::json!({"event_id": event.id.to_hex(), "accepted": true, "message": ""}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let controlled = ControlledSigner::new(false);
    let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
    let state = Arc::new(crate::app_state::build_app_state());
    state
        .replace_local_identity_keys(controlled.keys.clone())
        .unwrap();
    *state.relay_url_override.lock().unwrap() = Some(base.clone());
    let captured_base = relay_api_base_url_with_override(&state);
    let task_state = state.clone();
    let task = tokio::spawn(async move {
        submit::submit_event_at_with_signer(
            EventBuilder::new(Kind::TextNote, "captured"),
            &task_state,
            &captured_base,
            &signer,
        )
        .await
    });
    controlled.wait_entered().await;
    assert!(!task.is_finished());
    assert!(rx.try_recv().is_err());
    state.replace_local_identity_keys(Keys::generate()).unwrap();
    *state.relay_url_override.lock().unwrap() = Some("http://127.0.0.1:1".into());
    controlled.release.notify_one();
    controlled.wait_entered().await;
    assert!(rx.try_recv().is_err());
    controlled.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let (headers, body) = rx.recv().await.unwrap();
    let event = nostr::Event::from_json(&body).unwrap();
    assert_eq!(event.pubkey, controlled.keys.public_key());
    assert_eq!(event.content, "captured");
    event.verify().unwrap();
    let encoded = headers["authorization"]
        .to_str()
        .unwrap()
        .strip_prefix("Nostr ")
        .unwrap();
    let auth = nostr::Event::from_json(BASE64.decode(encoded).unwrap()).unwrap();
    assert!(
        auth.created_at > event.created_at,
        "generic event keeps its pre-admission timestamp; NIP-98 is fresh after admission"
    );
    assert_eq!(auth.pubkey, event.pubkey);
    assert_eq!(auth.kind, Kind::HttpAuth);
    assert!(auth.content.is_empty());
    assert_eq!(auth.tags.len(), 4);
    let expected_url = format!("{base}/{}", "events");
    assert_eq!(
        auth.tags
            .find(nostr::TagKind::custom("u"))
            .unwrap()
            .content(),
        Some(expected_url.as_str())
    );
    assert_eq!(
        auth.tags
            .find(nostr::TagKind::custom("method"))
            .unwrap()
            .content(),
        Some("POST")
    );
    let hash = hex::encode(Sha256::digest(&body));
    assert_eq!(
        auth.tags
            .find(nostr::TagKind::custom("payload"))
            .unwrap()
            .content(),
        Some(hash.as_str())
    );
    assert!(auth
        .tags
        .find(nostr::TagKind::Nonce)
        .unwrap()
        .content()
        .is_some());
    auth.verify().unwrap();
    assert_eq!(controlled.public_key_reads.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn nip98_failure_and_agent_author_distinction() {
    let controlled = ControlledSigner::new(true);
    let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
    let task = tokio::spawn(async move {
        build_nip98_auth_header_for_signer(
            &signer,
            &Method::GET,
            "https://relay.example/query",
            &[],
        )
        .await
    });
    controlled.wait_entered().await;
    assert!(!task.is_finished());
    controlled.release.notify_one();
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .contains("deliberate signer failure"));

    let user = ActiveUserSigner::local(Keys::generate());
    let agent = Keys::generate();
    let event = build_profile_event(&agent, "Agent", None, None, None).unwrap();
    assert_eq!(event.pubkey, agent.public_key());
    assert_ne!(event.pubkey, user.public_key());
    let state = crate::app_state::build_app_state();
    let error =
        submit::submit_signed_event_at_with_signer(&event, &state, "http://127.0.0.1:1", &user)
            .await
            .unwrap_err();
    assert_eq!(error, "signed event does not match the publishing identity");
}

#[tokio::test(start_paused = true)]
async fn generic_submit_signing_failure_precedes_rate_admission() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    crate::relay_admission::activate_rate_limit(Some(300));
    let controlled = ControlledSigner::new(true);
    let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
    let state = crate::app_state::build_app_state();
    state
        .replace_local_identity_keys(controlled.keys.clone())
        .unwrap();
    *state.test_signer.lock().unwrap() = Some(signer);
    let before = tokio::time::Instant::now();
    let task = tokio::spawn(async move {
        submit::submit_event(EventBuilder::new(Kind::TextNote, "gate"), &state).await
    });
    controlled.wait_entered().await;
    assert_eq!(
        tokio::time::Instant::now(),
        before,
        "EVENT signing must precede admission"
    );
    controlled.release.notify_one();
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .contains("deliberate signer failure"));
    assert_eq!(
        tokio::time::Instant::now(),
        before,
        "signing failure must not wait for admission"
    );
    crate::relay_admission::reset_rate_limit_gate();
}
