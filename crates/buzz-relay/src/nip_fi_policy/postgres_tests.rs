use super::*;
use axum::{
    body::to_bytes,
    http::{header, Request},
};
use base64::Engine;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

struct Replay(Mutex<HashSet<[u8; 32]>>);
impl buzz_auth::Nip98ReplayGuard for Replay {
    fn try_mark_in_scope<'a>(
        &'a self,
        _scope: &'a str,
        id: &'a nostr::EventId,
        _ttl: u64,
    ) -> BoxFuture<'a, Result<bool, buzz_auth::AuthError>> {
        let inserted = self.0.lock().unwrap().insert(*id.as_bytes());
        async move { Ok(inserted) }.boxed()
    }
}

// Exercise actual Hyper upgrades, the primary host policy query and tenant
// binding together. A deletion request does not fence serving until quiescing.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn real_required_root_audio_upgrades_retain_policy_during_pending_deletion() {
    use crate::nip_fi_core::test_support::{
        assertion_verifier, sign_assertion, TEST_COMMUNITY_URI,
    };
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{client::IntoClientRequest, Error},
    };

    let base = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .unwrap();
    let host = format!("ws-required-{}.example", uuid::Uuid::new_v4().simple());
    let community = base.db.ensure_configured_community(&host).await.unwrap();
    sqlx::query(
        "UPDATE communities SET requires_nip_fi = true, policy_generation = 1 WHERE id = $1",
    )
    .bind(community.id.as_uuid())
    .execute(base.db.pool())
    .await
    .unwrap();
    let key = Keys::generate();
    let token = sign_assertion("nip-fi+jwt", &key.public_key().to_hex(), None);
    for global in [NipFiMode::Off, NipFiMode::Shadow] {
        let mut state = (*base).clone();
        let config = Arc::make_mut(&mut state.config);
        config.nip_fi.mode = global;
        config.nip_fi.max_connection_lifetime_secs = 3600;
        config.nip_fi.communities = crate::nip_fi_core::test_support::any_host(TEST_COMMUNITY_URI);
        state.nip_fi_verifier = Some(Arc::new(assertion_verifier(
            buzz_auth::TokenClass::DedicatedNipFi,
        )));
        state.nip_fi_deny_map = Some(Arc::new(buzz_auth::NipFiDenyMap::new(100, vec![])));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = crate::router::build_router(Arc::new(state))
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        for stage in ["none", "submitted", "inventoried", "approved"] {
            if stage == "none" {
                sqlx::query("DELETE FROM community_deletion_requests WHERE community_id = $1")
                    .bind(community.id.as_uuid())
                    .execute(base.db.pool())
                    .await
                    .unwrap();
            } else if stage == "submitted" {
                sqlx::query("INSERT INTO community_deletion_requests (community_id, community_host, requested_by) VALUES ($1, $2, 'policy-test')")
                    .bind(community.id.as_uuid()).bind(&host).execute(base.db.pool()).await.unwrap();
            } else {
                sqlx::query(
                    "UPDATE community_deletion_requests SET stage = $2 WHERE community_id = $1",
                )
                .bind(community.id.as_uuid())
                .bind(stage)
                .execute(base.db.pool())
                .await
                .unwrap();
            }
            for path in ["/", "/huddle/00000000-0000-4000-8000-000000000001/audio"] {
                for assertion in [false, true] {
                    let mut request = format!("ws://{address}{path}")
                        .into_client_request()
                        .unwrap();
                    request.headers_mut().insert("host", host.parse().unwrap());
                    if assertion {
                        request.headers_mut().insert(
                            "nostr-federated-identity",
                            format!("Bearer {token}").parse().unwrap(),
                        );
                    }
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        connect_async(request),
                    )
                    .await
                    .unwrap();
                    if assertion {
                        let (mut socket, response) =
                            result.expect("valid signed assertion must upgrade");
                        assert_eq!(
                            response.status(),
                            StatusCode::SWITCHING_PROTOCOLS,
                            "{global:?} {stage} {path}"
                        );
                        let _ = socket.close(None).await;
                    } else {
                        match result {
                            Err(Error::Http(response)) => assert_eq!(
                                response.status(),
                                StatusCode::UNAUTHORIZED,
                                "{global:?} {stage} {path}"
                            ),
                            other => panic!(
                                "missing assertion must deny: {global:?} {stage} {path}: {other:?}"
                            ),
                        }
                    }
                }
            }
        }
        server.abort();
        let _ = server.await;
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn signed_unsigned_free_controls_and_required_discovery_use_real_host_rows() {
    let base = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("isolated Postgres and local Redis");
    let mut state = (*base).clone();
    let keys = Keys::generate();
    let free = format!("free-{}.example", uuid::Uuid::new_v4().simple());
    let required = format!("required-{}.example", uuid::Uuid::new_v4().simple());
    for host in [&free, &required] {
        state.db.ensure_configured_community(host).await.unwrap();
    }
    sqlx::query(
        "UPDATE communities SET requires_nip_fi = true, policy_generation = 1 WHERE host = $1",
    )
    .bind(&required)
    .execute(state.db.pool())
    .await
    .unwrap();
    let config = Arc::make_mut(&mut state.config);
    config.nip_fi.mode = NipFiMode::Off;
    config.nip_fi.max_connection_lifetime_secs = 3600;
    config.nip_fi.communities = crate::nip_fi_core::test_support::any_host("https://relay.example");
    config.relay_url = "wss://relay.example".to_owned();
    config.require_relay_membership = false;
    config.require_auth_token = false;
    state.nip98_replay = Arc::new(Replay(Mutex::new(HashSet::new())));
    state.nip_fi_verifier = Some(Arc::new(crate::nip_fi_core::tests::ScriptedVerifier::new(
        Ok(Some(keys.public_key())),
    )));
    state.nip_fi_deny_map = Some(Arc::new(buzz_auth::NipFiDenyMap::new(100, vec![])));
    let state = Arc::new(state);
    let app = crate::router::build_router(state.clone());
    let body = br#"[{"kinds":[1]}]"#;
    for host in [&free, &required] {
        for signed in [false, true] {
            let mut request = Request::post("/count")
                .header(header::HOST, host)
                .header(header::CONTENT_TYPE, "application/json");
            if signed {
                let event = EventBuilder::new(Kind::Custom(27235), "")
                    .tags([
                        Tag::parse(["u", &format!("https://{host}/count")]).unwrap(),
                        Tag::parse(["method", "POST"]).unwrap(),
                        Tag::parse(["payload", &hex::encode(Sha256::digest(body))]).unwrap(),
                    ])
                    .sign_with_keys(&keys)
                    .unwrap();
                request = request.header(
                    header::AUTHORIZATION,
                    format!(
                        "Nostr {}",
                        base64::engine::general_purpose::STANDARD.encode(event.as_json())
                    ),
                );
            } else {
                request = request.header("X-Pubkey", keys.public_key().to_hex());
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::from(body.to_vec())).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            assert_eq!(
                status,
                if host == &free {
                    StatusCode::OK
                } else {
                    StatusCode::UNAUTHORIZED
                },
                "{host} signed={signed}: {}",
                String::from_utf8_lossy(&bytes)
            );
        }
        let response = app
            .clone()
            .oneshot(
                Request::get("/info")
                    .header(header::HOST, host)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let doc: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(doc["requires_nip_fi"], host == &required);
    }
    // Policy read overhead: 100 serial, uncached reads, measured by the real source.
    let start = std::time::Instant::now();
    for _ in 0..100 {
        assert!(
            state
                .db
                .community_auth_policy_for_host(&required)
                .await
                .unwrap()
                .unwrap()
                .requires_nip_fi
        );
    }
    eprintln!("100 primary policy queries: {:?}", start.elapsed());
}
