use super::*;
use crate::app_state::build_app_state_for_mode;
use tauri::Manager;

#[test]
fn remote_bootstrap_never_evaluates_local_env_or_key_generation() {
    assert!(bootstrap_local_identity(SignerMode::Remote, || {
        panic!("remote bootstrap attempted local env/key loading or placeholder generation")
    })
    .is_none());
    let state = build_app_state_for_mode(SignerMode::Remote);
    assert_eq!(state.identity_storage(), IdentityStorage::Absent);
    assert!(!state
        .identity_lost
        .load(std::sync::atomic::Ordering::Acquire));
    assert!(!state
        .keyring_locked
        .load(std::sync::atomic::Ordering::Acquire));
    assert!(!state
        .managed_agent_restore_pending
        .load(std::sync::atomic::Ordering::Acquire));
    assert!(state.local_identity_keys().is_err());
    assert!(state.signing_keys().is_err());
    assert!(state.active_signer().is_err());
    assert!(state.legacy_local_signer().is_err());
    assert!(state.identity_public_key().is_err());
}

#[test]
fn remote_import_fails_before_persistence_and_never_installs_local_keys() {
    let state = build_app_state_for_mode(SignerMode::Remote);
    let temp = tempfile::tempdir().unwrap();
    let keys = Keys::generate(); // simulated supplied local key, not bootstrap
    let result =
        crate::commands::commit_imported_identity(&state, temp.path(), keys.clone(), |_| {
            panic!("remote import called persistence")
        });
    assert!(result.unwrap_err().contains("unsupported"));
    assert!(state
        .replace_local_identity_keys(keys)
        .unwrap_err()
        .contains("unsupported"));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    assert!(state.active_signer().is_err());
}

#[tokio::test]
async fn native_status_and_signing_ipc_report_absence_not_recovery_or_fallback() {
    let app = tauri::test::mock_builder()
        .manage(build_app_state_for_mode(SignerMode::Remote))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    assert!(
        crate::app_state::resolve_persisted_identity(app.handle(), &app.state())
            .unwrap_err()
            .contains("unsupported")
    );
    let status = serde_json::to_value(get_native_identity_status(app.state()).unwrap()).unwrap();
    assert_eq!(
        status,
        serde_json::json!({
            "mode": "remote", "authState": "signed-out", "publicIdentity": null,
            "generation": 0, "workspaceActive": false
        })
    );
    assert!(crate::commands::get_identity(app.state()).is_err());
    assert!(crate::commands::get_nsec(app.state())
        .unwrap_err()
        .contains("unsupported"));
    assert!(
        crate::commands::sign_event(1, "test".into(), None, vec![], app.state(), None)
            .await
            .unwrap_err()
            .contains("signed out")
    );
}

#[test]
fn local_bootstrap_and_recovery_capabilities_remain_distinct() {
    let state = build_app_state_for_mode(SignerMode::Local);
    let keys = state.local_identity_keys().unwrap();
    assert_eq!(state.identity_public_key().unwrap(), keys.public_key());
    assert_eq!(
        state.active_signer().unwrap().public_key(),
        keys.public_key()
    );
    state
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(state.active_signer().is_err());
    assert!(state.signing_keys().is_err());
    assert_eq!(
        state.legacy_local_signer().unwrap().public_key(),
        keys.public_key()
    );
    let status = serde_json::to_value(state.native_identity_status().unwrap()).unwrap();
    assert_eq!(status["mode"], "local");
    assert_eq!(status["publicIdentity"], keys.public_key().to_hex());
}

#[test]
#[ignore = "runs under an actual native remote build, independently from the local suite"]
fn compiled_remote_bootstrap_is_keyless() {
    assert_eq!(SignerMode::compiled(), SignerMode::Remote);
    let state = build_app_state_for_mode(SignerMode::compiled());
    assert!(state.is_remote_identity());
    assert!(state.local_identity_keys().is_err());
    assert!(state.active_signer().is_err());
    assert!(!crate::commands::is_shared_identity());
}

#[test]
fn remote_preview_dispatch_denies_effects_before_registered_handler_and_keeps_local() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    for mode in [SignerMode::Local, SignerMode::Remote] {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let app = tauri::test::mock_builder()
            .manage(crate::app_state::build_app_state_for_mode(mode))
            .invoke_handler(super::preview_dispatch(move |invoke| {
                counted.fetch_add(1, Ordering::SeqCst);
                invoke.resolver.resolve("reached");
                true
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let view = tauri::WebviewWindowBuilder::new(&app, "dispatch", Default::default())
            .build()
            .unwrap();
        for command in [
            "create_managed_agent",
            "import_identity",
            "open_dm",
            "update_profile",
            "send_channel_message",
            "get_channels",
            "get_native_identity_status",
        ] {
            let result = tauri::test::get_ipc_response(
                &view,
                tauri::webview::InvokeRequest {
                    cmd: command.into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: if cfg!(any(windows, target_os = "android")) {
                        "http://tauri.localhost"
                    } else {
                        "tauri://localhost"
                    }
                    .parse()
                    .unwrap(),
                    body: tauri::ipc::InvokeBody::Json(serde_json::json!({})),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.into(),
                },
            );
            assert_eq!(
                result.is_ok(),
                mode == SignerMode::Local || command == "get_native_identity_status",
                "{command}"
            );
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            if mode == SignerMode::Local { 7 } else { 1 }
        );
    }
}
