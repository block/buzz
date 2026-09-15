use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn actual_http_submit_uses_captured_generation_and_discards_late_result() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let signer = state.active_signer().unwrap();
    let result = crate::relay::submit_event_at(
        EventBuilder::text_note("foreground success"),
        &state,
        api.base.as_str(),
        &signer,
    )
    .await
    .unwrap();
    assert!(result.accepted);
    api.hold("events");
    let submit = crate::relay::submit_event_at(
        EventBuilder::text_note("held result"),
        &state,
        api.base.as_str(),
        &signer,
    );
    let logout = async {
        api.arrived().await;
        state.native_auth.clear().unwrap();
    };
    let (result, ()) = tokio::join!(submit, logout);
    assert!(result.is_err());
    api.release();
    api.login(&state.native_auth, "B").await.unwrap();
    let count = api.state.requests.lock().unwrap().len();
    assert!(crate::relay::submit_event_at(
        EventBuilder::text_note("old capability"),
        &state,
        api.base.as_str(),
        &signer
    )
    .await
    .is_err());
    assert_eq!(api.state.requests.lock().unwrap().len(), count);
}

#[tokio::test]
async fn native_ws_foreground_auth_uses_new_credential_for_same_key_and_fences_eose() {
    let api = MockApi::new().await;
    let owner = BuilderlabSession::default();
    api.login(&owner, "A").await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (requests, mut request_rx) = tokio::sync::mpsc::channel(4);
    let allow_eose = Arc::new(Semaphore::new(0));
    let permit = allow_eose.clone();
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        for _ in 0..2 {
            let (stream, _) = listener.accept().await.unwrap();
            let requests = requests.clone();
            let permit = permit.clone();
            connections.spawn(async move {
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                socket
                    .send(Message::Text(r#"["AUTH","native-test"]"#.into()))
                    .await
                    .unwrap();
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                    match frame[0].as_str() {
                        Some("AUTH") => {
                            let event: nostr::Event =
                                serde_json::from_value(frame[1].clone()).unwrap();
                            event.verify().unwrap();
                            socket
                                .send(Message::Text(
                                    serde_json::json!(["OK", event.id, true, ""])
                                        .to_string()
                                        .into(),
                                ))
                                .await
                                .unwrap();
                        }
                        Some("REQ") => {
                            requests.send(()).await.unwrap();
                            permit.acquire().await.unwrap().forget();
                            let _ = socket
                                .send(Message::Text(
                                    serde_json::json!(["EOSE", frame[1]]).to_string().into(),
                                ))
                                .await;
                        }
                        _ => {}
                    }
                }
            });
        }
        while connections.join_next().await.is_some() {}
    });
    let client = crate::native_relay_client::NativeRelayClient::default();
    let first = client
        .session_with_signer(url.clone(), owner.active_signer().unwrap())
        .await;
    let old = first.handle();
    let fetch = tokio::spawn(async move {
        old.fetch_events(serde_json::json!({"kinds":[1]}), Duration::from_secs(4))
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), request_rx.recv())
        .await
        .unwrap()
        .unwrap();
    api.login(&owner, "B").await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(1), fetch)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    allow_eose.add_permits(2);
    let second = client
        .session_with_signer(url, owner.active_signer().unwrap())
        .await;
    assert!(second
        .fetch_events(serde_json::json!({"kinds":[1]}), Duration::from_secs(4))
        .await
        .unwrap()
        .is_empty());
    let credentials = api
        .state
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.0 == "v1/buzz/identity/sign")
        .map(|r| r.1.clone().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(credentials, ["credential-A", "credential-B"]);
    owner.clear().unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn dropping_browser_ipc_releases_listener_and_pending_generation() {
    let owner = BuilderlabSession::default();
    let api = MockApi::new().await;
    let (tx, rx) = oneshot::channel::<String>();
    let attempt = owner.begin().unwrap();
    let base = api.base.clone();
    let task = tokio::spawn(async move {
        auth::browser_code(&attempt, &base, |url| {
            tx.send(url.into()).unwrap();
            Ok(())
        })
        .await
    });
    let url = Url::parse(&rx.await.unwrap()).unwrap();
    let callback =
        Url::parse(&url.query_pairs().find(|(k, _)| k == "returnTo").unwrap().1).unwrap();
    task.abort();
    let _ = task.await;
    assert_eq!(owner.status().unwrap().0, "signed-out");
    // Abort of the owned server is scheduled, so allow it one bounded drain.
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if tokio::net::TcpStream::connect(("127.0.0.1", callback.port().unwrap()))
                .await
                .is_err()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn current_sign_auth_denial_revokes_owner_without_exposing_body() {
    let api = MockApi::new().await;
    let owner = BuilderlabSession::default();
    api.login(&owner, "A").await.unwrap();
    *api.state.fail.lock().unwrap() = Some(("v1/buzz/identity/sign".into(), 403));
    let error = owner
        .active_signer()
        .unwrap()
        .sign_event(EventBuilder::text_note("denied"))
        .await
        .unwrap_err();
    assert!(!error.contains("DO_NOT_LOG_SECRET"));
    assert_eq!(owner.status().unwrap().0, "signed-out");
}

#[tokio::test]
async fn auth_decode_bounds_and_expiry_mismatch_fail_keyless() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    let owner = state.native_auth.clone();
    *api.state.identity.lock().unwrap() = Some("a".repeat(20_000));
    assert!(api
        .login(&owner, "oversize")
        .await
        .unwrap_err()
        .contains("oversized"));
    *api.state.identity.lock().unwrap() = None;
    for path in ["v1/auth/login/exchange", "v1/auth/me", "v1/buzz/identity"] {
        *api.state.fail.lock().unwrap() = Some((path.into(), 200));
        assert!(api
            .login(&owner, "malformed-json")
            .await
            .unwrap_err()
            .contains("malformed"));
        assert!(state.active_signer().is_err());
        assert!(state.local_identity_keys().is_err());
    }
    *api.state.fail.lock().unwrap() = None;
    api.hold("v1/auth/me");
    let attempt = owner.begin().unwrap();
    let base = api.base.clone();
    let task = tokio::spawn(async move {
        auth::finish_login(attempt, &base, "expiry-mismatch".into(), true).await
    });
    api.arrived().await;
    *api.state.expiry.lock().unwrap() = (Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
    api.release();
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .contains("expiry did not match"));
    assert_eq!(owner.status().unwrap().0, "signed-out");
}

#[tokio::test]
async fn auth_redirect_never_contacts_destination_or_forwards_code() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let location = format!("http://{}/leak", destination.local_addr().unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let router = Router::new().route(
        "/v1/auth/login/exchange",
        post(move || async move {
            (
                StatusCode::TEMPORARY_REDIRECT,
                [(axum::http::header::LOCATION, location)],
            )
        }),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let owner = BuilderlabSession::default();
    assert!(
        auth::finish_login(owner.begin().unwrap(), &base, "one-use-code".into(), true)
            .await
            .unwrap_err()
            .contains("307")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), destination.accept())
            .await
            .is_err()
    );
    assert!(owner.active_signer().is_err());
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn profile_edit_does_not_continue_under_same_owner_reauthentication() {
    use tauri::Manager;
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    *state.relay_url_override.lock().unwrap() = Some(api.base.to_string());
    let original = state.active_signer().unwrap();
    state
        .native_auth
        .activate_workspace(&original, api.base.as_str())
        .unwrap();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    api.hold("query");
    let edit = crate::commands::update_profile(
        Some("Stale edit".into()),
        None,
        None,
        None,
        original.generation(),
        app.state(),
    );
    let replace = async {
        api.arrived().await;
        api.login(&app.state::<crate::app_state::AppState>().native_auth, "B")
            .await
            .unwrap();
        api.release();
    };
    let (result, ()) = tokio::join!(edit, replace);
    assert!(result.is_err());
    let current = app
        .state::<crate::app_state::AppState>()
        .active_signer()
        .unwrap();
    assert_eq!(current.public_key(), original.public_key());
    assert_ne!(current.generation(), original.generation());
    assert!(!api
        .state
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|(path, _, _)| path == "events"));
}

#[tokio::test]
async fn upload_http_response_is_canceled_by_same_owner_reauthentication() {
    use axum::{routing::put, Json, Router};
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let original = state.active_signer().unwrap();
    let entered = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let arrived = entered.clone();
    let resume = release.clone();
    let router = Router::new().route("/upload", put(move || {
        let arrived = arrived.clone();
        let resume = resume.clone();
        async move {
            arrived.add_permits(1);
            resume.acquire().await.unwrap().forget();
            Json(serde_json::json!({"url":"http://test/media/blob", "sha256":"test", "size":1, "type":"image/png", "uploaded":1}))
        }
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    *state.relay_url_override.lock().unwrap() =
        Some(format!("http://{}", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(1, 1)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let upload = crate::commands::media::upload_image_bytes(png.into_inner(), &state);
    let replace = async {
        tokio::time::timeout(Duration::from_secs(5), entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        api.login(&state.native_auth, "B").await.unwrap();
        // Leave the HTTP response held: cancellation must resolve without it.
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(upload, replace)
    })
    .await
    .unwrap();
    release.add_permits(1);
    server.abort();
    assert!(result.is_err());
    let current = state.active_signer().unwrap();
    assert_eq!(original.public_key(), current.public_key());
    assert_ne!(original.generation(), current.generation());
}
