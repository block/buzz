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

#[tokio::test]
#[ignore = "requires Postgres"]
async fn optional_leases_on_another_pod_expire_before_acceptance_with_or_without_invalidation() {
    use buzz_db::community_auth::{UpgradeOutcome, UpgradeRequest};
    let base = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .unwrap();
    let owner = Keys::generate().public_key().to_hex();
    let issuer = "https://lease-issuer.example";
    for notify in [false, true] {
        let host = format!("lease-{}.example:8443", uuid::Uuid::new_v4().simple());
        let created = base
            .db
            .create_community_with_owner_and_nip_fi_issuers(
                &host,
                &owner,
                &[issuer.into()],
                &Keys::generate(),
            )
            .await
            .unwrap();
        let buzz_db::CreateCommunityWithOwnerResult::Created(row) = created.outcome else {
            panic!("fixture creation");
        };
        let mut pod_a = (*base).clone();
        Arc::make_mut(&mut pod_a.config).nip_fi.mode = NipFiMode::Shadow;
        Arc::make_mut(&mut pod_a.config)
            .nip_fi
            .max_connection_lifetime_secs = 3600;
        Arc::make_mut(&mut pod_a.config).nip_fi.communities =
            crate::nip_fi_core::test_support::any_host("https://relay.example");
        let key = Keys::generate().public_key();
        pod_a.nip_fi_verifier = Some(Arc::new(crate::nip_fi_core::tests::ScriptedVerifier::new(
            Ok(Some(key)),
        )));
        pod_a.nip_fi_deny_map = Some(Arc::new(buzz_auth::NipFiDenyMap::new(100, vec![])));
        pod_a.nip_fi_policy = Arc::new(base.db.clone());
        pod_a.tenant_host_cache = Arc::new(crate::host_cache::HostCache::new());
        let mut pod_b = pod_a.clone();
        // One connection proves rollback after a refused lease leaves THAT
        // connection reusable, rather than checking an unrelated pool slot.
        pod_b.db = buzz_db::Db::from_pool(
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect(&crate::test_support::database_url())
                .await
                .unwrap(),
        );
        pod_b.nip_fi_policy = Arc::new(pod_b.db.clone());
        pod_b.tenant_host_cache = Arc::new(crate::host_cache::HostCache::new());
        pod_b.tenant_host_reads = Arc::new(crate::shared_reads::SharedReads::new(
            10_000,
            std::time::Duration::from_millis(500),
        ));
        let mut h = HeaderMap::new();
        h.insert(header::HOST, host.to_uppercase().parse().unwrap());
        assert!(!resolve(&pod_b, &h).await.unwrap().stored_required);
        assert!(pod_b.tenant_host_cache.policy(&host).is_some());
        assert!(
            crate::nip_fi_http::admit_nip_fi_http_on_state(
                &pod_b,
                &h,
                &crate::access::AccessSlot::default(),
                || Ok(crate::nip_fi_http::Nip98Proof::new(key, ())),
            )
            .await
            .is_ok(),
            "optional community must retain legacy admission"
        );
        // The deletion lock is a deterministic barrier AFTER the lease wait
        // but BEFORE the row lock. It also lets this test remap the host while
        // acceptance is pending, without racing a timer.
        let mut barrier = base.db.pool().begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
            .bind(row.id.as_uuid())
            .execute(&mut *barrier)
            .await
            .unwrap();
        let request = UpgradeRequest {
            operation_id: uuid::Uuid::new_v4(),
            community_id: *row.id.as_uuid(),
            expected_host: host.clone(),
            actor_pubkey: owner.clone(),
            expected_owner_pubkey: owner.clone(),
            expected_issuer: issuer.into(),
            operator_pubkey: owner.clone(),
        };
        let acceptance = {
            let db = pod_a.db.clone();
            let request = request.clone();
            tokio::spawn(async move { db.enable_community_nip_fi(&request, Ok(())).await })
        };
        // Observe the production exclusive host lock. This condition is held
        // until our deletion barrier is released, independent of scheduling.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let held: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory'
                     AND classid = $1::int::oid AND objid = hashtext($2)::oid
                     AND objsubid = 2 AND mode = 'ExclusiveLock' AND granted)",
                )
                .bind(0x4649504ci32)
                .bind(&host)
                .fetch_one(base.db.pool())
                .await
                .unwrap();
                if held {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // A new fill through another pod cannot obtain an optional lease while
        // acceptance owns the host lock. Its connection remains reusable.
        assert!(pod_b
            .db
            .community_auth_policy_lease_for_host(&host.to_uppercase())
            .await
            .is_err());
        assert_eq!(
            sqlx::query_scalar::<_, i32>("SELECT 1")
                .fetch_one(pod_b.db.pool())
                .await
                .unwrap(),
            1
        );
        if notify {
            // The normal mutation invalidates locally and the cross-pod
            // consumer applies exactly this message. No Redis is required for
            // consistency: the false branch deliberately drops it entirely.
            pod_a.invalidate_community_host(&buzz_core::TenantContext::resolved(row.id, &host));
            pod_b.apply_cache_invalidation(
                row.id,
                buzz_pubsub::cache_invalidation::CacheInvalidation::CommunityHost {
                    host: host.clone(),
                },
            );
        }
        barrier.commit().await.unwrap();
        let UpgradeOutcome::Accepted(_) = acceptance.await.unwrap().unwrap() else {
            panic!("acceptance");
        };
        assert!(
            pod_b.tenant_host_cache.policy(&host).is_none(),
            "optional lease survived commit"
        );
        assert!(resolve(&pod_b, &h).await.unwrap().stored_required);
        assert_eq!(
            crate::nip_fi_http::admit_nip_fi_http_on_state(
                &pod_b,
                &h,
                &crate::access::AccessSlot::default(),
                || Ok(crate::nip_fi_http::Nip98Proof::new(key, ())),
            )
            .await
            .err()
            .unwrap()
            .status(),
            StatusCode::UNAUTHORIZED,
            "a post-commit admission on another pod must require FI"
        );

        // A distinct optional target remapped while acceptance holds only the
        // host lease lock must refuse, rather than commit under the wrong key.
        let host2 = format!("remap-{}.example", uuid::Uuid::new_v4().simple());
        let created = base
            .db
            .create_community_with_owner_and_nip_fi_issuers(
                &host2,
                &owner,
                &[issuer.into()],
                &Keys::generate(),
            )
            .await
            .unwrap();
        let buzz_db::CreateCommunityWithOwnerResult::Created(row2) = created.outcome else {
            panic!("fixture");
        };
        let mut barrier = base.db.pool().begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
            .bind(row2.id.as_uuid())
            .execute(&mut *barrier)
            .await
            .unwrap();
        let barrier_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *barrier)
            .await
            .unwrap();
        let request2 = UpgradeRequest {
            operation_id: uuid::Uuid::new_v4(),
            community_id: *row2.id.as_uuid(),
            expected_host: host2.clone(),
            ..request.clone()
        };
        let remapped = format!("new-{host2}");
        let acceptance = {
            let db = pod_a.db.clone();
            tokio::spawn(async move { db.enable_community_nip_fi(&request2, Ok(())).await })
        };
        // Reaching the deletion-lock wait proves the exclusive lease lock was
        // acquired and the full lease interval elapsed, before mutable checks.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory'
                    AND NOT granted AND $1 = ANY(pg_blocking_pids(pid)))",
                )
                .bind(barrier_pid)
                .fetch_one(base.db.pool())
                .await
                .unwrap();
                if waiting {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        sqlx::query("UPDATE communities SET host = $2 WHERE id = $1")
            .bind(row2.id.as_uuid())
            .bind(&remapped)
            .execute(base.db.pool())
            .await
            .unwrap();
        barrier.commit().await.unwrap();
        assert_eq!(
            acceptance.await.unwrap().unwrap(),
            UpgradeOutcome::Refused("host_conflict")
        );
        assert!(
            !pod_b
                .db
                .community_auth_policy_for_host(&remapped)
                .await
                .unwrap()
                .unwrap()
                .requires_nip_fi
        );
    }
}

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

struct PausedPolicyRead {
    db: buzz_db::Db,
    read: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
impl PolicySource for PausedPolicyRead {
    fn lookup<'a>(
        &'a self,
        host: &'a str,
    ) -> BoxFuture<'a, Result<Option<CommunityAuthPolicy>, ()>> {
        async move {
            let result = self
                .db
                .community_auth_policy_for_host(host)
                .await
                .map_err(|_| ())?;
            assert!(
                result.is_none(),
                "the initial primary snapshot must miss the archived host"
            );
            self.read.notify_one();
            self.resume.notified().await;
            Ok(result)
        }
        .boxed()
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn bound_row_policy_wins_when_required_host_becomes_serving_after_initial_miss() {
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
    let host = format!(
        "unarchive-required-{}.example",
        uuid::Uuid::new_v4().simple()
    );
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
    for global in [NipFiMode::Shadow, NipFiMode::Enforce] {
        for path in ["/", "/huddle/00000000-0000-4000-8000-000000000001/audio"] {
            for assertion in [false, true] {
                sqlx::query("UPDATE communities SET archived_at = now() WHERE id = $1")
                    .bind(community.id.as_uuid())
                    .execute(base.db.pool())
                    .await
                    .unwrap();
                let pause = Arc::new(PausedPolicyRead {
                    db: base.db.clone(),
                    read: Default::default(),
                    resume: Default::default(),
                });
                let mut state = (*base).clone();
                let config = Arc::make_mut(&mut state.config);
                config.nip_fi.mode = global;
                config.nip_fi.max_connection_lifetime_secs = 3600;
                config.nip_fi.communities =
                    crate::nip_fi_core::test_support::any_host(TEST_COMMUNITY_URI);
                state.nip_fi_policy = pause.clone();
                state.nip_fi_verifier = Some(Arc::new(assertion_verifier(
                    buzz_auth::TokenClass::DedicatedNipFi,
                )));
                state.nip_fi_deny_map = Some(Arc::new(buzz_auth::NipFiDenyMap::new(100, vec![])));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let app = crate::router::build_router(Arc::new(state))
                    .into_make_service_with_connect_info::<std::net::SocketAddr>();
                let server = tokio::spawn(async move { axum::serve(listener, app).await });
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
                let connect = tokio::spawn(async move { connect_async(request).await });
                tokio::time::timeout(std::time::Duration::from_secs(5), pause.read.notified())
                    .await
                    .unwrap();
                // The initial primary query already observed no serving row.
                // Make that SAME required row serving before its result reaches
                // mode selection and the authoritative row-zero binding.
                sqlx::query("UPDATE communities SET archived_at = NULL WHERE id = $1")
                    .bind(community.id.as_uuid())
                    .execute(base.db.pool())
                    .await
                    .unwrap();
                pause.resume.notify_one();
                let result = tokio::time::timeout(std::time::Duration::from_secs(5), connect)
                    .await
                    .unwrap()
                    .unwrap();
                if assertion {
                    let (mut socket, response) = result.expect("valid bound-row evidence upgrades");
                    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
                    let _ = socket.close(None).await;
                } else {
                    match result {
                        Err(Error::Http(response)) => assert_eq!(
                            response.status(),
                            StatusCode::UNAUTHORIZED,
                            "{global:?} {path}"
                        ),
                        other => panic!(
                            "bound required row must not become free: {global:?} {path} {other:?}"
                        ),
                    }
                }
                server.abort();
                let _ = server.await;
            }
        }
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
    for global in [NipFiMode::Off, NipFiMode::Shadow, NipFiMode::Enforce] {
        let mut state = (*base).clone();
        let config = Arc::make_mut(&mut state.config);
        config.nip_fi.mode = global;
        config.nip_fi.max_connection_lifetime_secs = 3600;
        config.nip_fi.communities = crate::nip_fi_core::test_support::any_host(TEST_COMMUNITY_URI);
        state.nip_fi_verifier = Some(Arc::new(assertion_verifier(
            buzz_auth::TokenClass::DedicatedNipFi,
        )));
        state.nip_fi_deny_map = Some(Arc::new(buzz_auth::NipFiDenyMap::new(100, vec![])));
        if global.is_off() {
            state.nip_fi_verifier = None;
            state.nip_fi_deny_map = None;
            Arc::make_mut(&mut state.config)
                .nip_fi
                .max_connection_lifetime_secs = 0;
        }
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
                    if assertion || global.is_off() {
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
    // Stored requirements and primary policy binding apply in evaluating modes.
    config.nip_fi.mode = NipFiMode::Shadow;
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
    // The durable requirement remains stored, but Off admits ordinary HTTP
    // without FI evidence and advertises no effective requirement.
    let mut off = (*state).clone();
    Arc::make_mut(&mut off.config).nip_fi.mode = NipFiMode::Off;
    off.nip_fi_verifier = None;
    off.nip_fi_deny_map = None;
    let off_app = crate::router::build_router(Arc::new(off));
    let response = off_app
        .clone()
        .oneshot(
            Request::post("/count")
                .header(header::HOST, &required)
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-Pubkey", keys.public_key().to_hex())
                .body(Body::from(body.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Off ignores stored FI requirements"
    );
    let response = off_app
        .oneshot(
            Request::get("/info")
                .header(header::HOST, &required)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let doc: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(doc["requires_nip_fi"], false);
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

/// A host-cache entry must not choose data under another row's optional policy.
/// Mutation: ignore request_tenant in bind_tenant => the A-only member is admitted.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn remapped_host_uses_one_policy_tenant_for_query_and_discovery() {
    let base = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("isolated Postgres and local Redis");
    let host = format!("policy-remap-{}.example", uuid::Uuid::new_v4().simple());
    let old_host = format!("old-{host}");
    let old = base.db.ensure_configured_community(&host).await.unwrap();
    sqlx::query(
        "UPDATE communities SET requires_nip_fi = true, policy_generation = 1 WHERE id = $1",
    )
    .bind(old.id.as_uuid())
    .execute(base.db.pool())
    .await
    .unwrap();
    let caller = Keys::generate();
    let _ = base
        .db
        .add_relay_member(
            old.id,
            &caller.public_key().to_hex(),
            "member",
            None,
            &base.relay_keypair,
        )
        .await
        .unwrap();
    let secret = EventBuilder::new(Kind::TextNote, "old required community data")
        .sign_with_keys(&caller)
        .unwrap();
    base.db.insert_event(old.id, &secret, None).await.unwrap();
    // Prime the actual trunk cache before the operator remaps the same host.
    let bound = crate::tenant::bind_community(&*base, &host).await.unwrap();
    assert_eq!(bound.community(), old.id);
    let mut tx = base.db.pool().begin().await.unwrap();
    sqlx::query("UPDATE communities SET host = $1 WHERE id = $2")
        .bind(&old_host)
        .bind(old.id.as_uuid())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let new = base.db.ensure_configured_community(&host).await.unwrap();
    assert_ne!(old.id, new.id);
    // Refresh only the witness's cache age: this models an unexpired entry
    // deterministically, without requiring database setup to finish inside 1s.
    base.tenant_host_cache
        .insert(host.clone(), (old.id, std::time::Instant::now()));
    let mut state = (*base).clone();
    let config = Arc::make_mut(&mut state.config);
    // Stored requirements and primary policy binding apply in evaluating modes.
    config.nip_fi.mode = NipFiMode::Shadow;
    config.require_relay_membership = true;
    config.require_auth_token = true;
    config.relay_url = "wss://relay.example".into();
    state.nip98_replay = Arc::new(Replay(Mutex::new(HashSet::new())));
    let state = Arc::new(state);
    let app = crate::router::build_router(state.clone());
    let body = serde_json::to_vec(&serde_json::json!([{"ids":[secret.id.to_hex()]}])).unwrap();
    for is_new_member in [false, true] {
        if is_new_member {
            let _ = state
                .db
                .add_relay_member(
                    new.id,
                    &caller.public_key().to_hex(),
                    "member",
                    None,
                    &state.relay_keypair,
                )
                .await
                .unwrap();
        }
        state
            .tenant_host_cache
            .insert(host.clone(), (old.id, std::time::Instant::now()));
        let proof = EventBuilder::new(Kind::Custom(27235), "")
            .tags([
                Tag::parse(["u", &format!("https://{host}/query")]).unwrap(),
                Tag::parse(["method", "POST"]).unwrap(),
                Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
                Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
            ])
            .sign_with_keys(&caller)
            .unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::post("/query")
                    .header(header::HOST, &host)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(
                        header::AUTHORIZATION,
                        format!(
                            "Nostr {}",
                            base64::engine::general_purpose::STANDARD.encode(proof.as_json())
                        ),
                    )
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        assert_eq!(
            status,
            if is_new_member {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            },
            "membership must belong to the policy tenant: {}",
            String::from_utf8_lossy(&bytes)
        );
        if is_new_member {
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                serde_json::json!([]),
                "new community membership must never expose the old community's event"
            );
        }
        let response = app
            .clone()
            .oneshot(
                Request::get("/info")
                    .header(header::HOST, &host)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let doc: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(doc["requires_nip_fi"], false);
        assert_eq!(
            doc["read_state_snapshot"]["community_id"],
            new.id.as_uuid().to_string(),
            "discovery must describe the same row as its fresh optional policy"
        );
    }
}
