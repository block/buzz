//! The two registered self-crypto commands with installed native login and real
//! loopback NIP44. Adapter protocol matrices live in remote_signer/capabilities_tests.
use super::*;
use crate::{app_state::build_app_state_for_mode, native_identity::SignerMode, AppState};
use nostr::nips::nip44;
use tauri::Manager;

type App = tauri::App<tauri::test::MockRuntime>;
const TEXT: &str = "  secret plaintext 🐝 e\u{301} é \u{fffd}\0\r\n\t\u{1f}  ";

fn app(mode: SignerMode) -> App {
    tauri::test::mock_builder()
        .manage(build_app_state_for_mode(mode))
        .invoke_handler(tauri::generate_handler![
            crate::commands::nip44_encrypt_to_self,
            crate::commands::nip44_decrypt_from_self,
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap()
}

async fn command(
    app: &App,
    encrypt: bool,
    input: String,
    generation: Option<u64>,
) -> Result<String, String> {
    if encrypt {
        crate::commands::nip44_encrypt_to_self(input, generation, app.state()).await
    } else {
        crate::commands::nip44_decrypt_from_self(input, generation, app.state()).await
    }
}

fn ciphertext(keys: &Keys) -> String {
    nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        TEXT,
        nip44::Version::V2,
    )
    .unwrap()
}

#[tokio::test]
async fn registered_self_crypto_remote_exact_text_pinned_peer_and_keyless_session_reuse() {
    let api = MockApi::new().await;
    let app = app(SignerMode::Remote);
    let state = app.state::<AppState>();
    api.login(&state.native_auth, "A").await.unwrap();
    let generation = state.active_signer().unwrap().generation();
    let status = serde_json::to_value(state.native_identity_status().unwrap()).unwrap();
    assert_eq!(status["workspaceActive"], false);
    for text in [TEXT, "\u{fffd}", "\0\r\n\t", "a"] {
        let encrypted = command(&app, true, text.into(), generation).await.unwrap();
        assert_eq!(
            nip44::decrypt(
                api.state.keys.secret_key(),
                &api.state.keys.public_key(),
                &encrypted
            )
            .unwrap(),
            text
        );
        assert_eq!(
            command(&app, false, encrypted, generation).await.unwrap(),
            text
        );
    }
    assert_eq!(
        state.identity_storage(),
        crate::identity_storage::IdentityStorage::Absent
    );
    assert!(state.local_identity_keys().is_err());
    assert!(state.signing_keys().is_err());
    assert_eq!(
        serde_json::to_value(state.native_identity_status().unwrap()).unwrap(),
        status
    );
    let requests = api.state.requests.lock().unwrap();
    let crypto: Vec<_> = requests
        .iter()
        .filter(|r| r.0.ends_with("/encrypt") || r.0.ends_with("/decrypt"))
        .collect();
    assert_eq!(crypto.len(), 8);
    for request in crypto {
        assert_eq!(request.1.as_deref(), Some("credential-A"));
        assert_eq!(
            request.2["peer_pubkey"],
            api.state.keys.public_key().to_hex()
        );
        assert_eq!(request.2.as_object().unwrap().len(), 2);
    }
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.0 == "v1/auth/login/exchange")
            .count(),
        1
    );
    assert!(!requests.iter().any(|r| r.0.ends_with("/sign")));
}

#[tokio::test]
async fn registered_self_crypto_rejects_signed_out_missing_and_wrong_generation_before_http() {
    let api = MockApi::new().await;
    let app = app(SignerMode::Remote);
    for authenticated in [false, true] {
        if authenticated {
            api.login(&app.state::<AppState>().native_auth, "A")
                .await
                .unwrap();
        }
        let current = app.state::<AppState>().native_auth.status().unwrap().2;
        let before = api.state.requests.lock().unwrap().len();
        for generation in [None, Some(current.wrapping_add(1))] {
            for encrypt in [false, true] {
                let input = if encrypt {
                    TEXT.into()
                } else {
                    ciphertext(&api.state.keys)
                };
                let error = command(&app, encrypt, input, generation).await.unwrap_err();
                assert!(
                    error.contains(if authenticated {
                        "current native identity generation"
                    } else {
                        "signed out"
                    }),
                    "{error}"
                );
            }
        }
        assert_eq!(api.state.requests.lock().unwrap().len(), before);
    }
    // Pure crypto does not open unrelated generationless publishing/socket gates.
    assert!(
        crate::commands::sign_event(1, TEXT.into(), None, vec![], app.state(), None)
            .await
            .unwrap_err()
            .contains("current native identity generation")
    );
    assert!(crate::commands::create_auth_event(
        "challenge".into(),
        api.base.to_string(),
        app.state(),
        None
    )
    .await
    .unwrap_err()
    .contains("current native identity generation"));
}

#[tokio::test]
async fn registered_self_crypto_logout_expiry_and_same_key_replacement_discard_pending_results() {
    for encrypt in [false, true] {
        for action in ["logout", "expiry", "replace"] {
            let api = MockApi::new().await;
            let app = app(SignerMode::Remote);
            let state = app.state::<AppState>();
            if action == "expiry" {
                *api.state.expiry.lock().unwrap() =
                    (Utc::now() + chrono::Duration::milliseconds(700)).to_rfc3339();
            }
            api.login(&state.native_auth, "A").await.unwrap();
            let held = state.active_signer().unwrap();
            let generation = held.generation();
            let path = if encrypt {
                "v1/buzz/identity/encrypt"
            } else {
                "v1/buzz/identity/decrypt"
            };
            let input = if encrypt {
                TEXT.into()
            } else {
                ciphertext(&api.state.keys)
            };
            api.hold(path);
            let invalidate = async {
                api.arrived().await;
                match action {
                    "logout" => state.native_auth.clear().unwrap(),
                    "replace" => {
                        api.login(&state.native_auth, "B").await.unwrap();
                    }
                    _ => tokio::time::sleep(Duration::from_millis(750)).await,
                }
            };
            let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
                tokio::join!(
                    command(&app, encrypt, input.clone(), generation),
                    invalidate
                )
            })
            .await
            .unwrap();
            assert!(
                result.unwrap_err().contains("canceled or expired"),
                "{action}"
            );
            assert!(held.check_valid().is_err());
            api.release();
            let before = api.state.requests.lock().unwrap().len();
            assert!(command(&app, encrypt, input.clone(), generation)
                .await
                .is_err());
            assert_eq!(api.state.requests.lock().unwrap().len(), before);
            if action == "replace" {
                let new = state.active_signer().unwrap();
                assert_eq!(new.public_key(), held.public_key());
                assert_ne!(new.generation(), generation);
                command(&app, encrypt, input, new.generation())
                    .await
                    .unwrap();
            }
            let requests = api.state.requests.lock().unwrap();
            let credentials: Vec<_> = requests
                .iter()
                .filter(|r| r.0 == path)
                .map(|r| r.1.as_deref().unwrap())
                .collect();
            assert_eq!(
                credentials,
                if action == "replace" {
                    vec!["credential-A", "credential-B"]
                } else {
                    vec!["credential-A"]
                }
            );
        }
    }
}

#[tokio::test]
async fn registered_self_crypto_errors_are_body_free_auth_revokes_but_network_does_not() {
    for encrypt in [false, true] {
        for (fault, detail) in [
            (200, "malformed response"),
            (401, "authentication failed (HTTP 401)"),
            (403, "authentication failed (HTTP 403)"),
            (503, "transient HTTP 503"),
            (0, "transport failure"),
        ] {
            let api = MockApi::new().await;
            let app = app(SignerMode::Remote);
            let state = app.state::<AppState>();
            api.login(&state.native_auth, "A").await.unwrap();
            let held = state.active_signer().unwrap();
            let status = state.native_auth.status().unwrap();
            let op = if encrypt { "encrypt" } else { "decrypt" };
            if fault == 0 {
                api.server.abort();
                // Wait for the listening socket to be dropped, without a live request.
                tokio::time::timeout(Duration::from_secs(1), async {
                    while !api.server.is_finished() {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            } else {
                *api.state.fail.lock().unwrap() = Some((format!("v1/buzz/identity/{op}"), fault));
            }
            let input = if encrypt {
                TEXT.into()
            } else {
                ciphertext(&api.state.keys)
            };
            let error = command(&app, encrypt, input, held.generation())
                .await
                .unwrap_err();
            let auth_denial = fault == 401 || fault == 403;
            // Authentication denial revokes the installed adapter. The active
            // signer result fence then reports the existing native stale error.
            assert_eq!(
                error,
                if auth_denial {
                    "native authentication canceled or expired".to_owned()
                } else {
                    format!("nip44 {op} failed: remote signer: {detail}")
                }
            );
            for secret in [TEXT, "credential-A", "DO_NOT_LOG_SECRET"] {
                assert!(!format!("{error:?} {held:?}").contains(secret));
            }
            if auth_denial {
                assert!(held.check_valid().is_err());
                assert_eq!(state.native_auth.status().unwrap().0, "signed-out");
            } else {
                assert_eq!(state.native_auth.status().unwrap(), status);
                held.check_valid().unwrap();
            }
            if fault != 0 && !auth_denial {
                *api.state.fail.lock().unwrap() = None;
                let input = if encrypt {
                    TEXT.into()
                } else {
                    ciphertext(&api.state.keys)
                };
                command(&app, encrypt, input, held.generation())
                    .await
                    .unwrap();
                assert_eq!(
                    api.state
                        .requests
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|r| r.0 == "v1/auth/login/exchange")
                        .count(),
                    1
                );
            }
        }
    }
}

#[tokio::test]
async fn registered_self_crypto_local_roundtrip_errors_and_existing_recovery_guard_unchanged() {
    let app = app(SignerMode::Local);
    let state = app.state::<AppState>();
    let keys = state.local_identity_keys().unwrap();
    for generation in [None, Some(999)] {
        let encrypted = command(&app, true, TEXT.into(), generation).await.unwrap();
        assert_eq!(
            nip44::decrypt(keys.secret_key(), &keys.public_key(), &encrypted).unwrap(),
            TEXT
        );
        assert_eq!(
            command(&app, false, encrypted, generation).await.unwrap(),
            TEXT
        );
        assert_eq!(
            command(&app, false, ciphertext(&keys), generation)
                .await
                .unwrap(),
            TEXT
        );
    }
    for input in ["", "invalid-ciphertext"] {
        let baseline = nip44::decrypt(keys.secret_key(), &keys.public_key(), input).unwrap_err();
        assert_eq!(
            command(&app, false, input.into(), None).await.unwrap_err(),
            format!("nip44 decrypt failed: {baseline}")
        );
    }
    let baseline = nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        "",
        nip44::Version::V2,
    )
    .unwrap_err();
    assert_eq!(
        command(&app, true, "".into(), None).await.unwrap_err(),
        format!("nip44 encrypt failed: {baseline}")
    );
    for flag in [&state.identity_lost, &state.keyring_locked] {
        flag.store(true, std::sync::atomic::Ordering::Release);
        let baseline = state.signing_keys().unwrap_err();
        assert_eq!(
            command(&app, true, TEXT.into(), None).await.unwrap_err(),
            baseline
        );
        assert_eq!(
            command(&app, false, ciphertext(&keys), None)
                .await
                .unwrap_err(),
            baseline
        );
        flag.store(false, std::sync::atomic::Ordering::Release);
        assert_eq!(
            command(&app, false, ciphertext(&keys), None).await.unwrap(),
            TEXT
        );
    }
}

#[tokio::test]
async fn registered_self_crypto_ipc_camelcase_generation_and_legacy_local_shape() {
    for mode in [SignerMode::Local, SignerMode::Remote] {
        let api = MockApi::new().await;
        let app = app(mode);
        let remote = app.state::<AppState>().is_remote_identity();
        if remote {
            api.login(&app.state::<AppState>().native_auth, "A")
                .await
                .unwrap();
        }
        let generation = app
            .state::<AppState>()
            .active_signer()
            .unwrap()
            .generation();
        let view = tauri::WebviewWindowBuilder::new(&app, "self-crypto", Default::default())
            .build()
            .unwrap();
        let requests_before = api.state.requests.lock().unwrap().len();
        let invoke = move |command: &str, body: serde_json::Value| {
            tauri::test::get_ipc_response(
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
                    body: tauri::ipc::InvokeBody::Json(body),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.into(),
                },
            )
            .map(|body| body.deserialize::<String>().unwrap())
        };
        tokio::time::timeout(Duration::from_secs(5), tokio::task::spawn_blocking(move || {
            let encrypted = invoke("nip44_encrypt_to_self", serde_json::json!({"plaintext":TEXT}));
            if remote {
                assert!(encrypted.unwrap_err().as_str().unwrap().contains("current native identity generation"));
                let error = invoke("nip44_decrypt_from_self", serde_json::json!({"ciphertext":"not parsed before generation guard"})).unwrap_err();
                assert!(error.as_str().unwrap().contains("current native identity generation"));
                let encrypted = invoke("nip44_encrypt_to_self", serde_json::json!({"plaintext":TEXT,"expectedGeneration":generation})).unwrap();
                assert_eq!(invoke("nip44_decrypt_from_self", serde_json::json!({"ciphertext":encrypted,"expectedGeneration":generation})).unwrap(), TEXT);
            } else {
                assert_eq!(invoke("nip44_decrypt_from_self", serde_json::json!({"ciphertext":encrypted.unwrap()})).unwrap(), TEXT);
            }
        })).await.unwrap().unwrap();
        assert_eq!(
            api.state.requests.lock().unwrap().len() - requests_before,
            if remote { 2 } else { 0 }
        );
    }
}
