#![allow(clippy::result_large_err)] // Test closures return the real Axum admission response.

use super::*;
use crate::nip_fi_core::tests::ScriptedVerifier;
use crate::nip_fi_http::{admit_nip_fi_http_on_state, Nip98Proof};
use axum::{
    body::to_bytes,
    http::{header, HeaderValue, Request},
};
use buzz_auth::{VerifierError, CLIENT_ATTACHED_HEADER};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tower::ServiceExt;

struct Source {
    required: AtomicBool,
    calls: AtomicUsize,
    unavailable: bool,
}
impl PolicySource for Source {
    fn lookup<'a>(
        &'a self,
        host: &'a str,
    ) -> BoxFuture<'a, Result<Option<CommunityAuthPolicy>, ()>> {
        async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.unavailable {
                return Err(());
            }
            if host == "unknown.example" {
                return Ok(None);
            }
            let required = host == "required.example" && self.required.load(Ordering::SeqCst);
            Ok(Some(CommunityAuthPolicy {
                community_id: uuid::Uuid::nil(),
                requires_nip_fi: required,
                policy_generation: i64::from(required),
            }))
        }
        .boxed()
    }
}
fn headers(host: &str, assertion: bool) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(header::HOST, HeaderValue::from_str(host).unwrap());
    if assertion {
        h.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer aaa.bbb.ccc"),
        );
    }
    h
}
async fn fixture(
    mode: NipFiMode,
) -> (
    Arc<crate::state::AppState>,
    Arc<Source>,
    nostr::PublicKey,
    Arc<ScriptedVerifier>,
) {
    let base = crate::state::tests::test_state_with_unreachable_database_url(
        "postgres://127.0.0.1:1/none",
    )
    .await;
    let mut state = (*base).clone();
    Arc::make_mut(&mut state.config).nip_fi.mode = mode;
    Arc::make_mut(&mut state.config)
        .nip_fi
        .max_connection_lifetime_secs = 3600;
    Arc::make_mut(&mut state.config).nip_fi.communities =
        crate::nip_fi_core::test_support::any_host("https://relay.example");
    let key = nostr::Keys::generate().public_key();
    let verifier = Arc::new(ScriptedVerifier::new(Ok(Some(key))));
    state.nip_fi_verifier = Some(verifier.clone());
    state.nip_fi_deny_map = Some(Arc::new(buzz_auth::NipFiDenyMap::new(100, vec![])));
    let source = Arc::new(Source {
        required: AtomicBool::new(true),
        calls: AtomicUsize::new(0),
        unavailable: false,
    });
    state.nip_fi_policy = source.clone();
    for host in ["free.example", "required.example"] {
        state.tenant_host_cache.insert(
            host.into(),
            (
                buzz_core::CommunityId::from_uuid(uuid::Uuid::nil()),
                std::time::Instant::now(),
            ),
        );
    }
    (Arc::new(state), source, key, verifier)
}

#[test]
fn effective_mode_truth_table() {
    for global in [
        NipFiMode::Off,
        NipFiMode::Shadow,
        NipFiMode::Enforce,
        NipFiMode::DenyProtected,
    ] {
        for required in [false, true] {
            let expected = match (global, required) {
                (NipFiMode::DenyProtected, _) => NipFiMode::DenyProtected,
                (NipFiMode::Shadow, true) => NipFiMode::Enforce,
                _ => global,
            };
            assert_eq!(effective(global, required), expected);
        }
    }
}

#[tokio::test]
async fn mode_stored_requirement_and_proof_matrix() {
    for global in [NipFiMode::Off, NipFiMode::Shadow, NipFiMode::Enforce] {
        for stored in [false, true] {
            let (state, source, key, verifier) = fixture(global).await;
            source.required.store(stored, Ordering::SeqCst);
            let required = global.enforces() || (global.observes_only() && stored);
            for signed in [false, true] {
                for assertion in [false, true] {
                    let result = admit_nip_fi_http_on_state(
                        &state,
                        &headers("required.example", assertion),
                        &crate::access::AccessSlot::default(),
                        || {
                            Ok(if signed {
                                Nip98Proof::new(key, ())
                            } else {
                                Nip98Proof::unsigned(key, ())
                            })
                        },
                    )
                    .await;
                    if required && (!signed || !assertion) {
                        assert_eq!(
                            result.err().unwrap().status(),
                            StatusCode::UNAUTHORIZED,
                            "{global:?} stored={stored} signed={signed} assertion={assertion}"
                        );
                    } else {
                        assert!(
                            result.is_ok(),
                            "{global:?} stored={stored} signed={signed} assertion={assertion}"
                        );
                    }
                }
            }
            if global.is_off() {
                assert_eq!(
                    source.calls.load(Ordering::SeqCst),
                    0,
                    "Off never reads FI policy for admission"
                );
                assert_eq!(verifier.calls(), 0, "Off never evaluates assertions");
            }
        }
    }
}

#[tokio::test]
async fn required_retains_verifier_and_key_pairing_denials() {
    for global in [NipFiMode::Shadow, NipFiMode::Enforce] {
        let (base, _, key, _) = fixture(global).await;
        for error in [
            VerifierError::UnknownIssuer,
            VerifierError::ClaimRejected,
            VerifierError::InvalidSignatureOrClaims,
            VerifierError::Expired,
            VerifierError::KeySourceUnavailable,
        ] {
            let mut state = (*base).clone();
            state.nip_fi_verifier = Some(Arc::new(ScriptedVerifier::new(Err(error))));
            let result = admit_nip_fi_http_on_state(
                &state,
                &headers("required.example", true),
                &crate::access::AccessSlot::default(),
                || Ok(Nip98Proof::new(key, ())),
            )
            .await;
            let expected = if error == VerifierError::KeySourceUnavailable {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::FORBIDDEN
            };
            assert_eq!(
                result.err().unwrap().status(),
                expected,
                "{global:?} {error:?}"
            );
        }
        let wrong = nostr::Keys::generate().public_key();
        assert_eq!(
            admit_nip_fi_http_on_state(
                &base,
                &headers("required.example", true),
                &crate::access::AccessSlot::default(),
                || Ok(Nip98Proof::new(wrong, ()))
            )
            .await
            .err()
            .unwrap()
            .status(),
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn protected_routes_cannot_bypass_required_rows() {
    for global in [NipFiMode::Shadow, NipFiMode::Enforce] {
        let (state, _, _, _) = fixture(global).await;
        let app = crate::router::build_router(state);
        for path in [
            "/",
            "/huddle/00000000-0000-4000-8000-000000000001/audio",
            "/query",
            "/count",
            "/events",
            "/buzz/v1/sync",
            "/git/o/r.git/info/refs?service=git-upload-pack",
            "/upload",
        ] {
            let mut req = Request::builder()
                .uri(path)
                .header(header::HOST, "required.example");
            if matches!(path, "/query" | "/count" | "/events") {
                req = req.method("POST");
            } else if path == "/upload" {
                req = req.method("PUT");
            }
            if path == "/" || path.contains("/audio") {
                req = req
                    .header("Upgrade", "websocket")
                    .header("Connection", "Upgrade")
                    .header("Sec-WebSocket-Version", "13")
                    .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
            }
            let response = app
                .clone()
                .oneshot(req.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{global:?} {path}"
            );
        }
    }
}

#[tokio::test]
async fn off_routes_ignore_stored_policy_and_unavailable_fi_dependencies() {
    let (base, source, _, _) = fixture(NipFiMode::Off).await;
    let control = crate::router::build_router(base.clone());
    let mut state = (*base).clone();
    state.nip_fi_verifier = None;
    state.nip_fi_deny_map = None;
    Arc::make_mut(&mut state.config)
        .nip_fi
        .max_connection_lifetime_secs = 0;
    let unavailable = Arc::new(Source {
        required: AtomicBool::new(true),
        calls: AtomicUsize::new(0),
        unavailable: true,
    });
    state.nip_fi_policy = unavailable.clone();
    let app = crate::router::build_router(Arc::new(state));
    for (method, path) in [
        ("GET", "/"),
        ("GET", "/huddle/00000000-0000-4000-8000-000000000001/audio"),
        ("POST", "/query"),
        ("POST", "/count"),
        ("POST", "/events"),
        ("GET", "/buzz/v1/sync"),
        ("GET", "/git/o/r.git/info/refs?service=git-upload-pack"),
        ("PUT", "/upload"),
        ("GET", "/info"),
    ] {
        let request = || {
            let mut req = Request::builder()
                .method(method)
                .uri(path)
                .header(header::HOST, "required.example");
            if path == "/" || path.contains("/audio") {
                req = req
                    .header("Upgrade", "websocket")
                    .header("Connection", "Upgrade")
                    .header("Sec-WebSocket-Version", "13")
                    .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
            }
            req.body(Body::empty()).unwrap()
        };
        let expected = control.clone().oneshot(request()).await.unwrap().status();
        let actual = app.clone().oneshot(request()).await.unwrap().status();
        assert_eq!(
            actual, expected,
            "Off changed legacy response for {method} {path}"
        );
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    assert_eq!(unavailable.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn off_upgrade_uses_cached_tenant_without_fi_reads_and_rejects_long_hosts() {
    let (state, source, _, _) = fixture(NipFiMode::Off).await;
    let tenant_id = uuid::Uuid::new_v4();
    state.tenant_host_cache.insert(
        "cached.example".into(),
        (
            buzz_core::CommunityId::from_uuid(tenant_id),
            std::time::Instant::now(),
        ),
    );
    // The actual database is unreachable. Only ordinary cached routing can
    // satisfy this call; a primary FI binding lookup cannot.
    let (tenant, mode) = bind_for_upgrade(&state, "cached.example").await.unwrap();
    assert_eq!(*tenant.community().as_uuid(), tenant_id);
    assert!(mode.is_off());
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    let long_host = "x".repeat(buzz_core::tenant::MAX_HOST_LEN + 1);
    state.tenant_host_cache.insert(
        long_host.clone(),
        (
            buzz_core::CommunityId::from_uuid(tenant_id),
            std::time::Instant::now(),
        ),
    );
    assert_eq!(
        bind_for_upgrade(&state, &long_host)
            .await
            .err()
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn discovery_is_host_specific_compatible_uncached_and_consistent() {
    for global in [NipFiMode::Off, NipFiMode::Shadow, NipFiMode::Enforce] {
        let (state, source, _, _) = fixture(global).await;
        let app = crate::router::build_router(state);
        for host in ["free.example", "required.example", "unknown.example"] {
            let mut docs = Vec::new();
            for path in ["/", "/info"] {
                let response = app
                    .clone()
                    .oneshot(
                        Request::get(path)
                            .header(header::HOST, host)
                            .header(header::ACCEPT, "application/nostr+json")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
                if host == "unknown.example" {
                    assert_eq!(response.status(), StatusCode::NOT_FOUND);
                    continue;
                }
                assert_eq!(response.status(), StatusCode::OK);
                let doc: serde_json::Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                        .unwrap();
                let required =
                    !global.is_off() && (host == "required.example" || global.enforces());
                let capability = global.evaluates() || required;
                assert_eq!(doc["requires_nip_fi"], required);
                assert_eq!(
                    doc["limitation"]["federated_identity"]
                        .as_bool()
                        .unwrap_or(false),
                    capability
                );
                assert_eq!(!doc["federated_identity"].is_null(), capability);
                docs.push(doc);
            }
            if docs.len() == 2 {
                assert_eq!(docs[0], docs[1]);
            }
        }
        // Serving hosts share a policy snapshot; unmapped results are not cached.
        assert_eq!(
            source.calls.load(Ordering::SeqCst),
            if global.is_off() { 0 } else { 4 }
        );
    }
}

#[tokio::test]
async fn missing_dependencies_fail_closed_and_repair_precedes_evidence() {
    for global in [
        NipFiMode::Off,
        NipFiMode::Shadow,
        NipFiMode::Enforce,
        NipFiMode::DenyProtected,
    ] {
        let (base, _, key, _) = fixture(global).await;
        for missing in ["verifier", "deny_map", "lifetime", "policy"] {
            let mut state = (*base).clone();
            match missing {
                "verifier" => state.nip_fi_verifier = None,
                "deny_map" => state.nip_fi_deny_map = None,
                "lifetime" => {
                    Arc::make_mut(&mut state.config)
                        .nip_fi
                        .max_connection_lifetime_secs = 0
                }
                _ => {
                    if !global.is_off() {
                        // This case exercises an unavailable COLD policy
                        // source, independent of the preceding valid leases.
                        state.tenant_host_cache.invalidate("required.example");
                    }
                    state.nip_fi_policy = Arc::new(Source {
                        required: AtomicBool::new(true),
                        calls: AtomicUsize::new(0),
                        unavailable: true,
                    })
                }
            }
            let result = admit_nip_fi_http_on_state(
                &state,
                &headers("required.example", false),
                &crate::access::AccessSlot::default(),
                || Ok(Nip98Proof::new(key, ())),
            )
            .await;
            if global.is_off() {
                assert!(result.is_ok(), "Off ignores missing FI {missing}");
            } else {
                assert_eq!(
                    result.err().unwrap().status(),
                    StatusCode::SERVICE_UNAVAILABLE,
                    "{global:?} {missing}"
                );
            }
            if global != NipFiMode::DenyProtected {
                assert!(!state.nip_fi_required_admission_ready() || missing == "policy");
                let app = crate::router::build_router(Arc::new(state));
                for path in ["/", "/info"] {
                    let response = app
                        .clone()
                        .oneshot(
                            Request::get(path)
                                .header(header::HOST, "required.example")
                                .header(header::ACCEPT, "application/nostr+json")
                                .body(Body::empty())
                                .unwrap(),
                        )
                        .await
                        .unwrap();
                    assert_eq!(
                        response.status(),
                        if global.is_off() {
                            StatusCode::OK
                        } else {
                            StatusCode::SERVICE_UNAVAILABLE
                        },
                        "{global:?} {missing} {path}"
                    );
                    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
                }
            }
        }
    }
}

#[tokio::test]
async fn request_scope_retains_its_snapshot_without_overwriting_newer_policy() {
    let (state, source, key, _) = fixture(NipFiMode::Shadow).await;
    let h = headers("required.example", true);
    source.required.store(false, Ordering::SeqCst);
    let earlier = resolve(&state, &h).await.unwrap();
    source.required.store(true, Ordering::SeqCst);
    state.tenant_host_cache.invalidate("required.example");
    let newer = resolve(&state, &h).await.unwrap();
    assert!(!earlier.required);
    assert!(newer.required);
    with_policy(earlier, async {
        assert!(mode(&state, &h).await.observes_only());
    })
    .await;
    // Finishing the old request cannot publish over the newer observation.
    assert!(resolve(&state, &h).await.unwrap().required);
    let before = source.calls.load(Ordering::SeqCst);
    with_policy(newer, async {
        for _ in 0..3 {
            assert!(mode(&state, &h).await.enforces());
        }
        assert!(admit_nip_fi_http_on_state(
            &state,
            &h,
            &crate::access::AccessSlot::default(),
            || Ok(Nip98Proof::new(key, ()))
        )
        .await
        .is_ok());
    })
    .await;
    assert_eq!(source.calls.load(Ordering::SeqCst), before);
}

struct PausedSource {
    read: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    calls: AtomicUsize,
}
impl PolicySource for PausedSource {
    fn lookup<'a>(
        &'a self,
        host: &'a str,
    ) -> BoxFuture<'a, Result<Option<CommunityAuthPolicy>, ()>> {
        async move {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.read.notify_one();
                self.resume.notified().await;
            }
            if host == "unknown.example" {
                return Ok(None);
            }
            Ok(Some(CommunityAuthPolicy {
                community_id: uuid::Uuid::nil(),
                requires_nip_fi: false,
                policy_generation: 0,
            }))
        }
        .boxed()
    }
}

#[tokio::test]
async fn concurrent_fills_share_one_read_and_a_drop_fences_publication_only() {
    let (base, _, _, _) = fixture(NipFiMode::Shadow).await;
    for (host, invalidated_host) in [
        ("team.example", None),
        ("team.example", Some("team.example")),
        ("team.example", Some("other.example")),
        ("unknown.example", None),
    ] {
        let invalidate = invalidated_host.is_some();
        let source = Arc::new(PausedSource {
            read: Default::default(),
            resume: Default::default(),
            calls: AtomicUsize::new(0),
        });
        let mut state = (*base).clone();
        state.tenant_host_cache = Arc::new(crate::host_cache::HostCache::new());
        state.nip_fi_policy = source.clone();
        let state = Arc::new(state);
        let leader = {
            let state = state.clone();
            tokio::spawn(async move { cached_lookup(&state, host).await })
        };
        source.read.notified().await;
        let observed_source = std::time::Instant::now();
        let mut waiters = Vec::new();
        for _ in 0..8 {
            let mut waiter = Box::pin(cached_lookup(&state, host));
            assert!(futures_util::poll!(&mut waiter).is_pending());
            waiters.push(waiter);
        }
        if let Some(invalidated_host) = invalidated_host {
            state.apply_cache_invalidation(
                buzz_core::CommunityId::from_uuid(uuid::Uuid::nil()),
                buzz_pubsub::cache_invalidation::CacheInvalidation::CommunityHost {
                    host: invalidated_host.into(),
                },
            );
        }
        source.resume.notify_one();
        let result = leader.await.unwrap().unwrap();
        assert_eq!(result.is_none(), host == "unknown.example");
        if !invalidate && result.is_some() {
            assert!(
                state
                    .tenant_host_cache
                    .policy_read_started_at(host)
                    .unwrap()
                    <= observed_source,
                "lease anchor must precede the paused source read"
            );
        }
        assert_eq!(
            state.tenant_host_cache.policy(host).is_none(),
            invalidate || result.is_none()
        );
        for waiter in waiters {
            assert_eq!(waiter.await.unwrap().is_none(), result.is_none());
        }
        let expected_reads = if result.is_none() {
            9
        } else if invalidate {
            2
        } else {
            1
        };
        assert_eq!(source.calls.load(Ordering::SeqCst), expected_reads);
        assert_eq!(
            state.tenant_host_cache.policy(host).is_none(),
            result.is_none()
        );
    }
}

#[tokio::test]
async fn unmapped_legacy_rejection_scope_never_yields_an_admission_proof() {
    let (state, _, key, _) = fixture(NipFiMode::Shadow).await;
    let policy = Policy {
        community_id: uuid::Uuid::nil(),
        mode: NipFiMode::Off,
        required: false,
        stored_required: false,

        mapped: false,
    };
    with_policy(policy, async {
        state.tenant_host_cache.insert(
            "unknown.example".into(),
            (
                buzz_core::CommunityId::from_uuid(uuid::Uuid::new_v4()),
                std::time::Instant::now(),
            ),
        );
        assert!(
            crate::nip_fi_shadow::bind_tenant(&state, &headers("unknown.example", false))
                .await
                .is_none(),
            "an authoritative miss cannot reuse a cached tenant"
        );

        let mut called = false;
        let response = admit_nip_fi_http_on_state(
            &state,
            &headers("unknown.example", false),
            &crate::access::AccessSlot::default(),
            || {
                called = true;
                Ok(Nip98Proof::new(key, ()))
            },
        )
        .await
        .err()
        .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            !called,
            "unmapped scope must refuse before credential extraction"
        );
    })
    .await;
}
