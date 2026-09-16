//! Exercise raw bytes, command-argument admission, remote signing and actual PUT.
//! The preview intentionally still gates uploads; this tests the registered
//! command beneath that gate, not staging/UI availability.
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use tauri::Manager;

#[tokio::test]
async fn registered_raw_upload_pins_generation_through_put_and_rejects_late_result() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let arrived = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let received = Arc::new(Mutex::new(Vec::new()));
    let router = Router::new().route("/upload", axum::routing::put({
        let arrived = arrived.clone();
        let release = release.clone();
        let received = received.clone();
        let public_key = api.state.keys.public_key();
        let base = base.clone();
        move |headers: axum::http::HeaderMap, bytes: axum::body::Bytes| {
            let (arrived, release, received, base) = (arrived.clone(), release.clone(), received.clone(), base.clone());
            async move {
                let auth = headers["authorization"].to_str().unwrap();
                let event = nostr::Event::from_json(URL_SAFE_NO_PAD.decode(auth.strip_prefix("Nostr ").unwrap()).unwrap()).unwrap();
                event.verify().unwrap();
                assert_eq!(event.pubkey, public_key);
                assert_eq!(event.kind, nostr::Kind::from(24242));
                let hash = hex::encode(Sha256::digest(&bytes));
                assert_eq!(headers["x-sha-256"], hash);
                assert!(event.tags.iter().any(|tag| tag.as_slice() == ["x", hash.as_str()]));
                let count = {
                    let mut received = received.lock().unwrap();
                    received.push(bytes.to_vec());
                    received.len()
                };
                if count == 2 {
                    arrived.add_permits(1);
                    release.acquire().await.unwrap().forget();
                }
                Json(serde_json::json!({"url":format!("{base}/media/{hash}"),"sha256":hash,"size":bytes.len(),"type":"application/octet-stream","uploaded":1}))
            }
        }
    }));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    *state.relay_url_override.lock().unwrap() = Some(base.clone());
    let signer = state.active_signer().unwrap();
    state
        .native_auth
        .activate_workspace(&signer, &base)
        .unwrap();
    let generation = signer.generation().unwrap();
    let app = tauri::test::mock_builder()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            crate::commands::upload_media_bytes_raw
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let call = move |generation: u64| {
        let webview = webview.clone();
        tokio::task::spawn_blocking(move || {
            let mut headers = tauri::http::HeaderMap::new();
            headers.insert(
                "x-buzz-identity-generation",
                generation.to_string().parse().unwrap(),
            );
            headers.insert(
                "x-buzz-filename",
                URL_SAFE_NO_PAD.encode("notes 🐝.txt").parse().unwrap(),
            );
            tauri::test::get_ipc_response(
                &webview,
                tauri::webview::InvokeRequest {
                    cmd: "upload_media_bytes_raw".into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: if cfg!(windows) {
                        "http://tauri.localhost"
                    } else {
                        "tauri://localhost"
                    }
                    .parse()
                    .unwrap(),
                    body: tauri::ipc::InvokeBody::Raw(b"raw body, not JSON".to_vec()),
                    headers,
                    invoke_key: tauri::test::INVOKE_KEY.into(),
                },
            )
        })
    };
    let result = call(generation)
        .await
        .unwrap()
        .unwrap()
        .deserialize::<serde_json::Value>()
        .unwrap();
    assert_eq!(result["filename"], "notes 🐝.txt");
    let pending = call(generation);
    tokio::time::timeout(Duration::from_secs(5), arrived.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let state = app.state::<crate::app_state::AppState>();
    api.login(&state.native_auth, "B").await.unwrap();
    state
        .native_auth
        .activate_workspace(&state.active_signer().unwrap(), &base)
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    release.add_permits(1);
    assert!(call(generation).await.unwrap().is_err());
    assert_eq!(
        *received.lock().unwrap(),
        vec![b"raw body, not JSON".to_vec(); 2]
    );
    let credentials = api
        .state
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.0 == "v1/buzz/identity/sign")
        .map(|r| r.1.clone().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(credentials, ["credential-A", "credential-A"]);
    server.abort();
}
