use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn config() -> EnterpriseLoginConfig {
    serde_json::from_value(serde_json::json!({
        "signerUrl":"https://signer.example/cash-app/goose/", "issuer":"https://issuer.example/",
        "clientId":"native-client", "audience":"https://signer-api.example", "organization":"org_corp",
        "connection":"corporate", "redirectUri":"http://127.0.0.1:45871/enterprise-callback"
    })).unwrap()
}
fn tokens() -> EnterpriseOAuthTokens {
    serde_json::from_value(serde_json::json!({"access_token":"synthetic-access", "refresh_token":"synthetic-rotation", "expires_in":300, "token_type":"Bearer"})).unwrap()
}
fn stored(expires_at: u64) -> StoredTokens {
    StoredTokens {
        config: config(),
        tokens: tokens(),
        expires_at,
        identity: ManagedIdentity {
            pubkey: nostr::Keys::generate().public_key().to_hex(),
            relay_ws_url: "wss://buzz.example".into(),
            relay_http_url: "https://buzz.example".into(),
        },
        rotation_pending: false,
    }
}
#[derive(Default)]
struct Io {
    refreshes: AtomicUsize,
    saves: Mutex<Vec<bool>>,
    fail_save: usize,
    fail_refresh: bool,
    cancel_during_refresh: Option<CancellationToken>,
}
impl CorporateSessionIo for Io {
    fn persist(&self, stored: &StoredTokens) -> Result<(), String> {
        let mut saves = self.saves.lock().unwrap();
        if self.fail_save == saves.len() + 1 {
            return Err("secure storage unavailable".into());
        }
        saves.push(stored.rotation_pending);
        Ok(())
    }
    fn refresh<'a>(
        &'a self,
        _: &'a EnterpriseLoginConfig,
        refresh: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<EnterpriseOAuthTokens, String>> + Send + 'a>> {
        Box::pin(async move {
            assert_eq!(refresh, "synthetic-rotation");
            assert_eq!(*self.saves.lock().unwrap(), vec![true]);
            self.refreshes.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            if let Some(cancel) = &self.cancel_during_refresh {
                cancel.cancel();
            }
            if self.fail_refresh {
                return Err("refresh denied".into());
            }
            Ok(tokens())
        })
    }
}
fn session(io: Arc<Io>) -> Arc<CorporateSession> {
    let stored = stored(0);
    Arc::new(CorporateSession {
        config: config(),
        client: RemoteIdentityClient::new(&config().signer_url).unwrap(),
        identity: stored.identity.clone(),
        tokens: AsyncMutex::new(stored),
        cancelled: CancellationToken::new(),
        io,
        media_reads: Default::default(),
    })
}
#[test]
fn absent_session_has_no_local_fallback() {
    let owner = EnterpriseIdentity::default();
    assert!(owner.event_signer().is_err());
    assert!(owner.managed_identity().is_err());
}
#[tokio::test]
async fn production_authorization_owner_singleflights_and_atomically_persists_rotation() {
    let io = Arc::new(Io::default());
    let session = session(io.clone());
    let original = session.identity.clone();
    let (a, b, c) = tokio::join!(
        session.credentials(),
        session.credentials(),
        session.credentials()
    );
    assert!(a.is_ok() && b.is_ok() && c.is_ok());
    assert_eq!(io.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(*io.saves.lock().unwrap(), vec![true, false]);
    assert_eq!(session.identity, original);
    assert!(!session.tokens.lock().await.rotation_pending);
}
#[tokio::test]
async fn every_storage_or_refresh_failure_cancels_owner_and_never_replays_token() {
    for (fail_save, fail_refresh, refreshes, saved) in [
        (1, false, 0, vec![]),
        (2, false, 1, vec![true]),
        (0, true, 1, vec![true]),
    ] {
        let io = Arc::new(Io {
            fail_save,
            fail_refresh,
            ..Io::default()
        });
        let session = session(io.clone());
        assert!(session.credentials().await.is_err());
        assert!(session.cancelled.is_cancelled());
        assert!(session.credentials().await.is_err());
        assert_eq!(io.refreshes.load(Ordering::SeqCst), refreshes);
        assert_eq!(*io.saves.lock().unwrap(), saved);
    }
}
#[tokio::test]
async fn logout_during_refresh_discards_response_and_leaves_durable_rotation_tombstone() {
    let cancel = CancellationToken::new();
    let io = Arc::new(Io {
        cancel_during_refresh: Some(cancel.clone()),
        ..Io::default()
    });
    let mut session = session(io.clone());
    Arc::get_mut(&mut session).unwrap().cancelled = cancel;
    assert!(session.credentials().await.is_err());
    assert_eq!(*io.saves.lock().unwrap(), vec![true]);
    assert!(session.tokens.lock().await.rotation_pending);
}
#[tokio::test]
async fn invalidation_keeps_cancelled_owner_so_status_cannot_restore_old_disk_login() {
    let owner = EnterpriseIdentity::default();
    let session = session(Arc::new(Io::default()));
    *owner.current.lock().unwrap() = Some(session.clone());
    let signer = owner.event_signer().unwrap();
    owner.invalidate().await.unwrap();
    assert!(owner.current.lock().unwrap().is_some());
    assert!(owner.managed_identity().is_err());
    assert!(session.credentials().await.is_err());
    let unsigned =
        nostr::EventBuilder::new(nostr::Kind::TextNote, "old operation").build(signer.public_key());
    assert!(signer.sign(unsigned).await.is_err());
}
#[tokio::test]
async fn restore_refuses_interrupted_rotation_or_different_build_without_network() {
    let owner = EnterpriseIdentity::default();
    let mut pending = stored(now().unwrap() + 120);
    pending.rotation_pending = true;
    assert!(owner.install(config(), pending).await.is_err());
    let mut changed = stored(now().unwrap() + 120);
    changed.config.organization = "another-org".into();
    assert!(owner.install(config(), changed).await.is_err());
    assert!(owner.event_signer().is_err());
}

#[test]
#[ignore = "run with a synthetic BUZZ_BUILD_ENTERPRISE config to test the actual compiled mode"]
fn compiled_corporate_mode_has_no_key_and_rejects_import_before_persistence() {
    assert!(enabled());
    assert!(build_config().unwrap().is_some());
    let state = crate::app_state::build_app_state();
    assert!(state.keys.lock().unwrap().is_none());
    assert!(state.signing_keys().is_err());
    assert!(state.event_signer().is_err());
    let mut called = false;
    let result = crate::commands::commit_imported_identity(
        &state,
        std::path::Path::new("/unused"),
        nostr::Keys::generate(),
        |_| {
            called = true;
            Ok(crate::identity_storage::IdentityStorage::Ephemeral)
        },
    );
    assert!(result.is_err());
    assert!(!called);
}

// These invoke the three production transport consumers, not just proof minting.
#[tokio::test]
#[ignore = "requires synthetic compiled corporate configuration; never live credentials"]
async fn compiled_corporate_media_consumers_never_transport_on_denial_or_logout() {
    assert!(enabled());
    let requests = Arc::new(AtomicUsize::new(0));
    let recorded = requests.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/media/blob",
                axum::routing::get(move || {
                    let recorded = recorded.clone();
                    async move {
                        recorded.fetch_add(1, Ordering::SeqCst);
                        "must not reach media"
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });
    let mut outcomes = Vec::new();
    for reason in ["missing", "logout", "proof-denial"] {
        let state = crate::app_state::build_app_state();
        *state.relay_url_override.lock().unwrap() = Some(base.clone());
        if reason != "missing" {
            // A valid HTTPS identity; the stale local relay origin must fail the
            // real owner's proof check, not become optional auth on a stale URL.
            let current = session(Arc::new(Io::default()));
            *state.enterprise.current.lock().unwrap() = Some(current);
            if reason == "logout" {
                state.enterprise.invalidate().await.unwrap();
            }
        }
        let response = crate::media_proxy::proxy_handler_with_state(
            &state,
            &state.http_client,
            axum::http::Request::builder()
                .uri("/media/blob")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
        outcomes.push((reason, "streaming proxy", response.status().as_u16()));
        let response = crate::media_proxy::handle_buzz_media_with_state(
            &state,
            &tauri::http::Request::builder()
                .uri("buzz-media://localhost/media/blob")
                .body(Vec::new())
                .unwrap(),
        )
        .await;
        outcomes.push((reason, "protocol proxy", response.status().as_u16()));
        let result = crate::commands::media_download::fetch_blob_bytes_with_cap(
            &format!("{base}/media/blob"),
            &state,
            1024,
            None,
        )
        .await;
        outcomes.push((reason, "download", if result.is_err() { 403 } else { 200 }));
    }
    server.abort();
    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "unsigned transport: {outcomes:?}"
    );
    assert!(
        outcomes.iter().all(|(_, _, status)| *status == 403),
        "{outcomes:?}"
    );
}

#[tokio::test]
async fn oss_media_consumers_preserve_optional_unsigned_auth() {
    assert!(!enabled(), "run OSS suite without corporate build config");
    let requests = Arc::new(AtomicUsize::new(0));
    let recorded = requests.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/media/blob",
                axum::routing::get(move |headers: axum::http::HeaderMap| {
                    let recorded = recorded.clone();
                    async move {
                        assert!(!headers.contains_key("authorization"));
                        recorded.fetch_add(1, Ordering::SeqCst);
                        "blob"
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = None;
    *state.relay_url_override.lock().unwrap() = Some(base.clone());
    let streaming = crate::media_proxy::proxy_handler_with_state(
        &state,
        &state.http_client,
        axum::http::Request::builder()
            .uri("/media/blob")
            .body(axum::body::Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(streaming.status(), 200);
    let protocol = crate::media_proxy::handle_buzz_media_with_state(
        &state,
        &tauri::http::Request::builder()
            .uri("buzz-media://localhost/media/blob")
            .body(Vec::new())
            .unwrap(),
    )
    .await;
    assert_eq!(protocol.status(), 200);
    assert_eq!(
        crate::commands::media_download::fetch_blob_bytes_with_cap(
            &format!("{base}/media/blob"),
            &state,
            1024,
            None,
        )
        .await
        .unwrap(),
        b"blob"
    );
    server.abort();
    assert_eq!(requests.load(Ordering::SeqCst), 3);
}
