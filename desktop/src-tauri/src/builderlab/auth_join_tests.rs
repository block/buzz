//! Self-join through the real preview dispatcher, command, remote signer and HTTP relay.
use super::*;
use crate::{app_state::AppState, native_identity::SignerMode};
use tauri::Manager;

#[tokio::test]
async fn remote_join_ipc_requires_current_workspace_generation_and_publishes_self_join() {
    let api = MockApi::new().await;
    let state = crate::app_state::build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let signer = state.active_signer().unwrap();
    let generation = signer.generation().unwrap();
    let relay = api.base.as_str().to_string();
    *state.relay_url_override.lock().unwrap() = Some(relay.clone());
    state
        .native_auth
        .activate_workspace(&signer, &relay)
        .unwrap();
    let app = tauri::test::mock_builder()
        .manage(state)
        .invoke_handler(crate::native_identity::preview_dispatch(
            tauri::generate_handler![crate::commands::join_channel],
        ))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let view = tauri::WebviewWindowBuilder::new(&app, "join", Default::default())
        .build()
        .unwrap();
    let requests = api.state.clone();
    tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(move || {
            let invoke = |generation: Option<u64>| {
                tauri::test::get_ipc_response(
                    &view,
                    tauri::webview::InvokeRequest {
                        cmd: "join_channel".into(),
                        callback: tauri::ipc::CallbackFn(0),
                        error: tauri::ipc::CallbackFn(1),
                        url: if cfg!(any(windows, target_os = "android")) {
                            "http://tauri.localhost"
                        } else {
                            "tauri://localhost"
                        }
                        .parse()
                        .unwrap(),
                        body: tauri::ipc::InvokeBody::Json(serde_json::json!({
                            "channelId": "00000000-0000-0000-0000-000000000001",
                            "expectedGeneration": generation,
                        })),
                        headers: Default::default(),
                        invoke_key: tauri::test::INVOKE_KEY.into(),
                    },
                )
            };
            let before = requests.requests.lock().unwrap().len();
            for invalid in [None, Some(generation + 1)] {
                assert!(invoke(invalid).is_err());
            }
            assert_eq!(requests.requests.lock().unwrap().len(), before);
            assert!(invoke(Some(generation)).is_ok());
            let posts = requests.requests.lock().unwrap();
            let body = &posts
                .iter()
                .find(|request| request.0 == "events")
                .unwrap()
                .2;
            let event: nostr::Event = serde_json::from_value(body.clone()).unwrap();
            event.verify().unwrap();
            assert_eq!(event.kind.as_u16(), 9021);
            assert_eq!(event.pubkey, requests.keys.public_key());
            assert!(event
                .tags
                .iter()
                .any(|tag| tag.as_slice() == ["h", "00000000-0000-0000-0000-000000000001"]));
            drop(posts);
            app.state::<AppState>().native_auth.clear().unwrap();
            let before = requests.requests.lock().unwrap().len();
            assert!(invoke(Some(generation)).is_err());
            assert_eq!(requests.requests.lock().unwrap().len(), before);
        }),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn join_command_does_not_adopt_a_replacement_generation_after_dispatch() {
    let api = MockApi::new().await;
    let state = crate::app_state::build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let old = state.active_signer().unwrap().generation();
    api.login(&state.native_auth, "B").await.unwrap();
    let signer = state.active_signer().unwrap();
    let relay = api.base.as_str().to_string();
    *state.relay_url_override.lock().unwrap() = Some(relay.clone());
    state
        .native_auth
        .activate_workspace(&signer, &relay)
        .unwrap();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let before = api.state.requests.lock().unwrap().len();
    assert!(crate::commands::join_channel(
        "00000000-0000-0000-0000-000000000001".into(),
        old,
        app.state::<AppState>()
    )
    .await
    .is_err());
    assert_eq!(api.state.requests.lock().unwrap().len(), before);
}
