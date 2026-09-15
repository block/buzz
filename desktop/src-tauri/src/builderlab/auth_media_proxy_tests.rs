//! Loopback upstreams exercising the real proxy core and Tauri protocol handler.
use super::*;
use axum::{body::Body, http::HeaderMap, routing::get};
use futures_util::StreamExt;
use tauri::Manager;

struct MediaServer {
    base: String,
    arrived: Arc<Semaphore>,
    release: Arc<Semaphore>,
    requests: Arc<Mutex<Vec<(String, HeaderMap)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for MediaServer {
    fn drop(&mut self) {
        self.release.add_permits(100);
        self.task.abort();
    }
}
impl MediaServer {
    async fn new(hold_headers: bool) -> Self {
        let arrived = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (a, r, seen) = (arrived.clone(), release.clone(), requests.clone());
        let router = Router::new().route(
            "/media/blob",
            get(move |uri: axum::http::Uri, headers: HeaderMap| {
                let (a, r, seen) = (a.clone(), r.clone(), seen.clone());
                async move {
                    seen.lock().unwrap().push((uri.to_string(), headers));
                    if hold_headers {
                        a.add_permits(1);
                        r.acquire().await.unwrap().forget();
                    }
                    let body =
                        futures_util::stream::once(async { Ok::<_, std::io::Error>("first") })
                            .chain(futures_util::stream::once(async move {
                                a.add_permits(1);
                                r.acquire().await.unwrap().forget();
                                Ok::<_, std::io::Error>("late")
                            }));
                    (
                        [("cache-control", "public, max-age=31536000, immutable")],
                        Body::from_stream(body),
                    )
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            base,
            arrived,
            release,
            requests,
            task,
        }
    }
    async fn arrived(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.arrived.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
    }
    fn activate(&self, state: &crate::app_state::AppState) -> u64 {
        *state.relay_url_override.lock().unwrap() = Some(self.base.clone());
        let signer = state.active_signer().unwrap();
        state
            .native_auth
            .activate_workspace(&signer, &self.base)
            .unwrap();
        signer.generation().unwrap()
    }
    fn uri(&self, generation: u64) -> axum::http::Uri {
        format!("/media/blob?thumb=1&__buzz_generation={generation}")
            .parse()
            .unwrap()
    }
}

#[tokio::test]
async fn media_proxy_reauth_rejects_old_urls_and_discards_ready_late_chunks() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let server = MediaServer::new(false).await;
    let generation = server.activate(&state);
    let uri = server.uri(generation);
    let response = crate::media_proxy::proxy_response(&state, &uri, &HeaderMap::new()).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let mut body = response.into_body().into_data_stream();
    assert_eq!(body.next().await.unwrap().unwrap(), "first");
    server.arrived().await;
    api.login(&state.native_auth, "B").await.unwrap();
    let current = server.activate(&state);
    assert_ne!(current, generation);
    server.release.add_permits(1);
    assert!(tokio::time::timeout(Duration::from_secs(3), body.next())
        .await
        .unwrap()
        .unwrap()
        .is_err());
    for uri in [
        uri,
        "/media/blob".parse().unwrap(),
        "/media/blob?__buzz_generation=bad".parse().unwrap(),
        format!("/media/blob?__buzz_generation={current}&__buzz_generation={current}")
            .parse()
            .unwrap(),
    ] {
        let response = crate::media_proxy::proxy_response(&state, &uri, &HeaderMap::new()).await;
        assert!(response.status().is_client_error());
    }
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "/media/blob?thumb=1");
    assert!(requests[0].1.contains_key("authorization"));
    let sign_requests: Vec<_> = api
        .state
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|(path, _, _)| path == "v1/buzz/identity/sign")
        .cloned()
        .collect();
    assert_eq!(
        sign_requests.len(),
        1,
        "rejected loads must not mint B's auth"
    );
    assert_eq!(sign_requests[0].1.as_deref(), Some("credential-A"));
}

#[tokio::test]
async fn media_proxy_logout_cancels_held_upstream_headers() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let server = MediaServer::new(true).await;
    let uri = server.uri(server.activate(&state));
    let headers = HeaderMap::new();
    let read = crate::media_proxy::proxy_response(&state, &uri, &headers);
    let clear = async {
        server.arrived().await;
        state.native_auth.clear().unwrap();
    };
    let (response, ()) =
        tokio::time::timeout(Duration::from_secs(8), async { tokio::join!(read, clear) })
            .await
            .unwrap();
    assert_eq!(response.status(), 502);
    assert_eq!(response.headers()["cache-control"], "no-store");
}

#[tokio::test]
async fn media_protocol_reauth_discards_buffer_and_never_waits_for_held_body() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let server = MediaServer::new(false).await;
    let uri = server.uri(server.activate(&state));
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    app.manage(state);
    let request = axum::http::Request::builder()
        .uri(uri)
        .body(Vec::new())
        .unwrap();
    let read = crate::media_proxy::handle_buzz_media(app.handle(), &request);
    let replace = async {
        server.arrived().await;
        let state = app.state::<crate::app_state::AppState>();
        api.login(&state.native_auth, "B").await.unwrap();
        server.activate(&state);
    };
    let (response, ()) = tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(read, replace)
    })
    .await
    .unwrap();
    assert_eq!(response.status(), 502);
    assert!(!response.body().windows(5).any(|part| part == b"first"));
    let stale = crate::media_proxy::handle_buzz_media(app.handle(), &request).await;
    assert_eq!(stale.status(), 401);
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn media_proxy_signing_failure_never_downgrades_to_unsigned_remote_read() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let server = MediaServer::new(false).await;
    let uri = server.uri(server.activate(&state));
    *api.state.fail.lock().unwrap() = Some(("v1/buzz/identity/sign".into(), 500));
    let response = crate::media_proxy::proxy_response(&state, &uri, &HeaderMap::new()).await;
    assert_eq!(response.status(), 502);
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn media_proxy_local_recovery_stays_unsigned_and_keeps_local_cache_policy() {
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Local);
    state
        .keyring_locked
        .store(true, std::sync::atomic::Ordering::Release);
    let server = MediaServer::new(false).await;
    *state.relay_url_override.lock().unwrap() = Some(server.base.clone());
    let response = crate::media_proxy::proxy_response(
        &state,
        &"/media/blob".parse().unwrap(),
        &HeaderMap::new(),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert!(!server.requests.lock().unwrap()[0]
        .1
        .contains_key("authorization"));
}

#[tokio::test]
async fn media_proxy_does_not_follow_upstream_redirects() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let destination = MediaServer::new(false).await;
    let target = format!("{}/media/blob", destination.base);
    let router = Router::new().route(
        "/media/blob",
        get(move || {
            let target = target.clone();
            async move {
                (
                    axum::http::StatusCode::TEMPORARY_REDIRECT,
                    [("location", target)],
                )
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    *state.relay_url_override.lock().unwrap() = Some(base.clone());
    let signer = state.active_signer().unwrap();
    state
        .native_auth
        .activate_workspace(&signer, &base)
        .unwrap();
    let response = crate::media_proxy::proxy_response(
        &state,
        &destination.uri(signer.generation().unwrap()),
        &HeaderMap::new(),
    )
    .await;
    task.abort();
    assert_eq!(response.status(), 307);
    assert!(
        !response.headers().contains_key("location"),
        "webview must not follow either"
    );
    assert!(destination.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn media_download_fetch_discards_held_body_after_reauthentication() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let server = MediaServer::new(false).await;
    server.activate(&state);
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    app.manage(state);
    // This command uses the same capped HTTP reader as download/save/clipboard.
    let fetch = crate::commands::fetch_media_bytes(
        format!("{}/media/blob", server.base),
        Some("reauth-fetch".into()),
        app.state(),
        crate::media_read::MediaReadScope::capture_command(&app.state()).unwrap(),
    );
    let replace = async {
        server.arrived().await;
        let state = app.state::<crate::app_state::AppState>();
        api.login(&state.native_auth, "B").await.unwrap();
        server.activate(&state);
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(fetch, replace)
    })
    .await
    .unwrap();
    assert!(result.is_err());
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn media_download_fetch_never_downgrades_failed_remote_signing() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let server = MediaServer::new(false).await;
    server.activate(&state);
    *api.state.fail.lock().unwrap() = Some(("v1/buzz/identity/sign".into(), 500));
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    app.manage(state);
    let result = crate::commands::fetch_media_bytes(
        format!("{}/media/blob", server.base),
        None,
        app.state(),
        crate::media_read::MediaReadScope::capture_command(&app.state()).unwrap(),
    )
    .await;
    assert!(result.is_err());
    assert!(server.requests.lock().unwrap().is_empty());
}
