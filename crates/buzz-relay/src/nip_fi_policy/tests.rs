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
                (_, true) => NipFiMode::Enforce,
                _ => global,
            };
            assert_eq!(effective(global, required), expected);
        }
    }
}

#[tokio::test]
async fn mixed_required_and_signed_unsigned_optional_admission() {
    for global in [NipFiMode::Off, NipFiMode::Shadow] {
        let (state, _, key, _) = fixture(global).await;
        for signed in [false, true] {
            // Optional controls retain both legacy proofs, regardless of attached FI.
            let h = headers("free.example", false);
            let result = admit_nip_fi_http_on_state(&state, &h, || {
                Ok(if signed {
                    Nip98Proof::new(key, ())
                } else {
                    Nip98Proof::unsigned(key, ())
                })
            })
            .await;
            assert!(result.is_ok(), "{global:?} signed={signed}");
            let h = headers("required.example", true);
            let result = admit_nip_fi_http_on_state(&state, &h, || {
                Ok(if signed {
                    Nip98Proof::new(key, ())
                } else {
                    Nip98Proof::unsigned(key, ())
                })
            })
            .await;
            if signed {
                assert!(result.is_ok());
            } else {
                assert_eq!(result.err().unwrap().status(), StatusCode::UNAUTHORIZED);
            }
        }
        let h = headers("required.example", false);
        assert_eq!(
            admit_nip_fi_http_on_state(&state, &h, || Ok(Nip98Proof::new(key, ())))
                .await
                .err()
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let (state, _, key, _) = fixture(NipFiMode::Enforce).await;
    let h = headers("free.example", false);
    assert_eq!(
        admit_nip_fi_http_on_state(&state, &h, || Ok(Nip98Proof::new(key, ())))
            .await
            .err()
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn required_retains_verifier_and_key_pairing_denials() {
    for global in [NipFiMode::Off, NipFiMode::Shadow] {
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
            let result =
                admit_nip_fi_http_on_state(&state, &headers("required.example", true), || {
                    Ok(Nip98Proof::new(key, ()))
                })
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
            admit_nip_fi_http_on_state(&base, &headers("required.example", true), || Ok(
                Nip98Proof::new(wrong, ())
            ))
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
    for global in [NipFiMode::Off, NipFiMode::Shadow] {
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
                let required = host == "required.example" || global.enforces();
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
        // One primary policy query per discovery request, no cross-host policy reuse.
        assert_eq!(source.calls.load(Ordering::SeqCst), 6);
    }
}

#[tokio::test]
async fn missing_dependencies_fail_closed_and_repair_precedes_evidence() {
    for global in [NipFiMode::Off, NipFiMode::Shadow, NipFiMode::DenyProtected] {
        let (base, _, key, _) = fixture(global).await;
        for missing in ["verifier", "deny_map", "policy"] {
            let mut state = (*base).clone();
            match missing {
                "verifier" => state.nip_fi_verifier = None,
                "deny_map" => state.nip_fi_deny_map = None,
                _ => {
                    state.nip_fi_policy = Arc::new(Source {
                        required: AtomicBool::new(true),
                        calls: AtomicUsize::new(0),
                        unavailable: true,
                    })
                }
            }
            let result =
                admit_nip_fi_http_on_state(&state, &headers("required.example", false), || {
                    Ok(Nip98Proof::new(key, ()))
                })
                .await;
            assert_eq!(
                result.err().unwrap().status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{global:?} {missing}"
            );
        }
    }
}

#[tokio::test]
async fn request_scope_reuses_one_read_without_publishing_optional_cache_entries() {
    let (state, source, key, _) = fixture(NipFiMode::Off).await;
    let h = headers("required.example", true);
    source.required.store(false, Ordering::SeqCst);
    let earlier = resolve(&state, &h).await.unwrap();
    source.required.store(true, Ordering::SeqCst);
    let newer = resolve(&state, &h).await.unwrap();
    assert!(!earlier.required);
    assert!(newer.required);
    with_policy(earlier, async {
        assert!(mode(&state, &h).await.is_off());
    })
    .await;
    // Finishing the old request cannot publish over the newer observation.
    assert!(resolve(&state, &h).await.unwrap().required);
    let before = source.calls.load(Ordering::SeqCst);
    with_policy(newer, async {
        for _ in 0..3 {
            assert!(mode(&state, &h).await.enforces());
        }
        assert!(
            admit_nip_fi_http_on_state(&state, &h, || Ok(Nip98Proof::new(key, ())))
                .await
                .is_ok()
        );
    })
    .await;
    assert_eq!(source.calls.load(Ordering::SeqCst), before);
}

#[tokio::test]
async fn unmapped_legacy_rejection_scope_never_yields_an_admission_proof() {
    let (state, _, key, _) = fixture(NipFiMode::Off).await;
    let policy = Policy {
        mode: NipFiMode::Off,
        required: false,
        stored_required: false,
        mapped: false,
    };
    with_policy(policy, async {
        let mut called = false;
        let response =
            admit_nip_fi_http_on_state(&state, &headers("unknown.example", false), || {
                called = true;
                Ok(Nip98Proof::new(key, ()))
            })
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
