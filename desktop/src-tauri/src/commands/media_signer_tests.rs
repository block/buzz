use super::*;
use crate::active_user_signer::tests::ControlledSigner;

#[tokio::test]
async fn blossom_auth_uses_async_signer_and_preserves_upload_scope() {
    let controlled = ControlledSigner::new(false);
    let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
    let task = tokio::spawn(async move {
        sign_blossom_upload_auth(&signer, "abc123", 300, "https://relay.example:8443").await
    });
    controlled.wait_entered().await;
    assert!(!task.is_finished());
    controlled.release.notify_one();
    let event = task.await.unwrap().unwrap();
    assert_eq!(event.pubkey, controlled.keys.public_key());
    assert_eq!(event.kind.as_u16(), 24242);
    assert_eq!(event.content, "Upload buzz-media");
    assert_eq!(event.tags.len(), 4);
    for tag in [
        ["t", "upload"],
        ["x", "abc123"],
        ["server", "relay.example:8443"],
    ] {
        assert!(event.tags.iter().any(|actual| actual.as_slice() == tag));
    }
    event.verify().unwrap();

    let controlled = ControlledSigner::new(true);
    let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
    let task = tokio::spawn(async move {
        sign_blossom_get_auth_header(&signer, "https://relay.example", 600).await
    });
    controlled.wait_entered().await;
    assert!(!task.is_finished());
    controlled.release.notify_one();
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .contains("deliberate signer failure"));
}

#[tokio::test]
async fn captured_upload_keeps_relay_owner_and_body_through_legacy_retry() {
    use axum::{
        body::Bytes,
        http::{HeaderMap, StatusCode},
        routing::put,
        Json, Router,
    };
    use std::sync::{Arc, Mutex};
    let requests = Arc::new(Mutex::new(Vec::<(HeaderMap, Bytes)>::new()));
    let first = requests.clone();
    let second = requests.clone();
    let router = Router::new()
        .route("/upload", put(move |headers: HeaderMap, body: Bytes| {
            first.lock().unwrap().push((headers, body));
            async { StatusCode::NOT_FOUND }
        }))
        .route("/media/upload", put(move |headers: HeaderMap, body: Bytes| {
            second.lock().unwrap().push((headers, body));
            async { Json(serde_json::json!({"url":"http://original/media/blob", "sha256":"test", "size":4, "type":"application/octet-stream", "uploaded":1})) }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let controlled = ControlledSigner::new(false);
    let state = crate::app_state::build_app_state();
    state
        .replace_local_identity_keys(controlled.keys.clone())
        .unwrap();
    *state.test_signer.lock().unwrap() =
        Some(ActiveUserSigner::new(controlled.clone()).await.unwrap());
    *state.relay_url_override.lock().unwrap() = Some(base.clone());
    let upload = MediaUploadScope::capture(&state, None).unwrap();
    let send = do_upload::<tauri::Wry>(
        vec![1, 2, 3, 4],
        "application/octet-stream",
        &state,
        None,
        None,
        &upload,
    );
    let replace = async {
        controlled.wait_entered().await;
        state
            .install_local_workspace("http://127.0.0.1:1".into(), Some(nostr::Keys::generate()))
            .unwrap();
        controlled.release.notify_one();
    };
    let (result, ()) = tokio::join!(send, replace);
    server.abort();
    result.unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for (headers, body) in requests.iter() {
        assert_eq!(body.as_ref(), &[1, 2, 3, 4]);
        let auth = headers["authorization"]
            .to_str()
            .unwrap()
            .strip_prefix("Nostr ")
            .unwrap();
        let event = nostr::Event::from_json(URL_SAFE_NO_PAD.decode(auth).unwrap()).unwrap();
        event.verify().unwrap();
        assert_eq!(event.pubkey, controlled.keys.public_key());
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice()
                == ["server", extract_server_authority(&base).unwrap().as_str()]));
        assert!(
            event
                .tags
                .iter()
                .any(|tag| tag.as_slice()
                    == ["x", hex::encode(Sha256::digest([1, 2, 3, 4])).as_str()])
        );
    }
}

#[tokio::test]
async fn local_upload_scope_drop_preserves_existing_worker_cancellation() {
    let state = crate::app_state::build_app_state();
    let parent = CancellationToken::new();
    let scope = MediaUploadScope::capture(&state, Some(&parent)).unwrap();
    let worker_token = scope.cancellation.clone();
    assert!(!worker_token.is_cancelled());
    drop(scope);
    assert!(!worker_token.is_cancelled());
    assert!(!parent.is_cancelled());
    let scope = MediaUploadScope::capture(&state, Some(&parent)).unwrap();
    parent.cancel();
    assert!(scope.cancellation.is_cancelled());
}
