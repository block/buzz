//! Shared production admission across native replacement, queued effects and IPC.
use super::*;
use crate::{
    app_state::{build_app_state_for_mode, AppState},
    media_read::MediaReadScope,
    native_identity::SignerMode,
    user_operation::UserOperationScope,
};
use tauri::Manager;

fn activate(state: &AppState) {
    let signer = state.active_signer().unwrap();
    let relay = crate::relay::relay_ws_url_with_override(state);
    state
        .native_auth
        .activate_workspace(&signer, &relay)
        .unwrap();
}

#[tokio::test]
async fn native_replacement_and_foreground_effect_share_admission_without_recursive_auth_lock() {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    activate(&state);
    assert!(Arc::ptr_eq(
        &state.operation_generation,
        &state.native_auth.operation_generation
    ));
    let operation = UserOperationScope::capture(&state).unwrap();
    let signer = state.active_signer().unwrap();
    let admission = operation.admit(&state).unwrap();
    // These nested reads occur in real agent pending-retention/start helpers.
    assert_eq!(
        state.active_signer().unwrap().public_key(),
        signer.public_key()
    );
    assert!(state.native_auth.workspace_active().unwrap());
    assert_eq!(state.native_auth.status().unwrap().0, "authenticated");
    std::thread::scope(|threads| {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let session = state.native_auth.clone();
        threads.spawn(move || {
            entered_tx.send(()).unwrap();
            session.clear().unwrap();
            done_tx.send(()).unwrap();
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(done_rx.try_recv().is_err());
        assert!(state.native_auth.operation_generation.try_lock().is_err());
        drop(admission);
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    assert!(operation.admit(&state).is_err());
    api.login(&state.native_auth, "B").await.unwrap();
    activate(&state);
    assert_eq!(
        signer.public_key(),
        state.active_signer().unwrap().public_key()
    );
    assert!(operation.admit(&state).is_err());
    assert!(UserOperationScope::capture(&state)
        .unwrap()
        .admit(&state)
        .is_ok());
}

#[tokio::test]
async fn save_dialog_and_queued_clipboard_effect_cannot_write_after_same_owner_reauth() {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    activate(&state);
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let state = app.state::<AppState>();
    let scope = MediaReadScope::capture_command(&state).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("download.txt");
    std::fs::write(&path, b"original").unwrap();
    let clipboard_path = temp.path().join("clipboard-effect");
    let destination = clipboard_path.clone();
    let (queued, result) =
        crate::commands::admitted_media_effect(app.handle().clone(), scope.clone(), move |_| {
            std::fs::write(destination, b"must not write").map_err(|e| e.to_string())
        });
    // Reauthentication proceeds independently of the canceled picker future.
    let (selected_tx, selected_rx) = tokio::sync::oneshot::channel();
    let save = crate::commands::save_download(
        &scope,
        &state,
        async { selected_rx.await.map_err(|e| e.to_string()) },
        b"stale",
    );
    let replace = async {
        api.login(&state.native_auth, "B").await.unwrap();
        activate(&state);
        let _ = selected_tx.send(Some(path.clone()));
    };
    let (saved, ()) = tokio::join!(save, replace);
    assert!(saved.is_err());
    queued();
    assert!(result.await.unwrap().is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert!(!clipboard_path.exists());
    let current = MediaReadScope::capture_command(&state).unwrap();
    assert!(crate::commands::save_download(
        &current,
        &state,
        async { Ok(Some(path.clone())) },
        b"current"
    )
    .await
    .unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), b"current");
    assert!(
        !crate::commands::save_download(&current, &state, async { Ok(None) }, b"canceled")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn registered_lists_reject_reauth_between_dispatch_and_argument_capture() {
    for command in ["list_managed_agents", "list_personas", "list_teams"] {
        assert_registered_list_rejects_reauth(command).await;
    }
}

async fn assert_registered_list_rejects_reauth(command: &'static str) {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    activate(&state);
    let old = state.active_signer().unwrap().generation().unwrap();
    let dispatch_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = dispatch_count.clone();
    let runtime = tokio::runtime::Handle::current();
    let base = api.base.clone();
    let handler: fn(tauri::ipc::Invoke<tauri::test::MockRuntime>) -> bool = tauri::generate_handler![
        crate::commands::list_managed_agents,
        crate::commands::list_personas,
        crate::commands::list_teams
    ];
    let app = tauri::test::mock_builder()
        .manage(state)
        .invoke_handler(crate::native_identity::preview_dispatch(move |invoke| {
            // Actual dispatcher already accepted old metadata. Replace before
            // the actual generated command captures its arguments.
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let state = invoke.message.state_ref().get::<AppState>();
            runtime
                .block_on(auth::finish_login(
                    state.native_auth.begin().unwrap(),
                    &base,
                    "B".into(),
                    true,
                ))
                .unwrap();
            activate(&state);
            assert_ne!(state.active_signer().unwrap().generation(), Some(old));
            handler(invoke)
        }))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let response = tokio::task::spawn_blocking(move || {
        tauri::test::get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: command.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: if cfg!(windows) {
                    "http://tauri.localhost"
                } else {
                    "tauri://localhost"
                }
                .parse()
                .unwrap(),
                body: tauri::ipc::InvokeBody::Json(serde_json::json!({"expectedGeneration":old})),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.into(),
            },
        )
    })
    .await
    .unwrap();
    assert!(response.is_err(), "{command} accepted stale authority");
    assert_eq!(dispatch_count.load(std::sync::atomic::Ordering::SeqCst), 1);
}
