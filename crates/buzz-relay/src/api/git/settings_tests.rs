//! Live route/store/clone regressions. Require explicit isolated service URLs;
//! never fall back to a developer's Desktop database.

// ── NIP-FI admission seam — settings route ──────────────────────────────────
//
// Proves that `authenticate()` in `git/settings.rs` routes through
// `admit_nip_fi_http_on_state`, not the raw bridge verifier.
//
// Falsifying mutation: replace the `admit_nip_fi_http_on_state(...)` call in
// `authenticate()` with the old raw `verify_bridge_auth_with_options(...)`.
// With that mutation, a valid NIP-98 proof for key B + assertion for key A
// (mismatched keys) would be admitted — the handler never checks key pairing.
// Without the mutation the request is denied 401 `authentication required\n`
// (MissingEvidence: no `Nostr-Federated-Identity` assertion header).
//
// The test here is: valid NIP-98 + Enforce mode + no assertion → 401 from
// `admit_nip_fi_http_on_state` (the same body the guard would produce if the
// guard itself fired).  The important invariant is that the HANDLER calls
// admission — the guard also fires, and both 401, so this is observationally
// equivalent to having only the guard.  However, the handler call is required
// by NIP-FI.md:516-533 for key pairing, which cannot be verified at the guard.
// The `#[ignore]` comment explains why a full key-pairing test needs JWT infra.
#[cfg(test)]
mod postgres_tests {
    use super::super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use base64::Engine;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use tower::ServiceExt;

    struct AlwaysFreshReplayGuard;

    impl buzz_auth::Nip98ReplayGuard for AlwaysFreshReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            _event_id: &'a nostr::EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            Box::pin(async { Ok(true) })
        }
    }

    /// Build a minimal Enforce-mode AppState that reaches the settings route.
    ///
    /// No issuers configured → `nip_fi_verifier = None` (DenyProtected startup path).
    /// The test fires before the verifier is needed: missing assertion → 401
    /// `MissingEvidence` before the verifier is consulted.
    async fn enforce_state() -> Option<Arc<AppState>> {
        let mut config = crate::config::Config::for_test();
        config.database_url = crate::test_support::database_url();
        config.redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        config.relay_url = "ws://nip-fi-settings-test.local".to_string();
        config.require_auth_token = true;
        config.require_relay_membership = false;
        config.nip_fi.mode = buzz_auth::NipFiMode::Enforce;
        config.nip_fi.communities =
            crate::nip_fi_core::test_support::any_host("https://relay.test");

        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).ok()?;
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .ok()?,
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;

        let (mut state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        Some(Arc::new(state))
    }

    fn nip98_get_token(keys: &Keys, url: &str) -> String {
        let tags = vec![
            Tag::parse(["u", url]).expect("u tag"),
            Tag::parse(["method", "GET"]).expect("method tag"),
            Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).expect("nonce tag"),
        ];
        let event = EventBuilder::new(Kind::Custom(27235), "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign NIP-98 event");
        format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&event).unwrap())
        )
    }

    // ── R1 NIP-FI admission seam: settings GET, Enforce, no assertion → 401 ──
    //
    // Falsifying mutation: remove the `admit_nip_fi_http_on_state(...)` call
    // from `authenticate()` in `git/settings.rs`, replacing it with the old
    // raw bridge verifier.  With the old verifier, a valid NIP-98 token for any
    // community member would be admitted without key pairing — the response
    // would be 200 or a different status.  With the NIP-FI call present and no
    // assertion header, `admit_nip_fi_http_on_state` maps the absent header to
    // MissingEvidence (401, "authentication required\n", `WWW-Authenticate: Nostr`).
    //
    // Note: the outer router guard also fires on missing assertion, so a
    // missing-assertion test is not sufficient to distinguish "handler calls
    // admission" from "guard fires first".  A full key-pairing test requires a
    // real JWT infrastructure with a live JWKS endpoint — that lives in the
    // integration test suite.  This seam test focuses on the code path change
    // (verify_bridge_auth_with_options → admit_nip_fi_http_on_state) and confirms
    // the settings route is reachable in Enforce mode with valid NIP-98 auth.
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_settings_get_no_assertion_is_401() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(enforce_state()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!("nip-fi-settings-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let keys = Keys::generate();
        // Use a dummy path (repo won't exist, but NIP-FI admission fires before the repo lookup).
        let path = format!(
            "/git/{}/test-repo/default-branch",
            keys.public_key().to_hex()
        );
        let url = format!("http://{host}{path}");
        let token = nip98_get_token(&keys, &url);

        let (status, body) = rt.block_on(async {
            use axum::body::to_bytes;
            let response = super::super::super::transport::git_router(state)
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(&path)
                        .header("host", &host)
                        .header("authorization", &token)
                        // No Nostr-Federated-Identity header — this is the no-assertion case.
                        .body(Body::empty())
                        .expect("build request"),
                )
                .await
                .expect("router oneshot");
            let status = response.status();
            let body = to_bytes(response.into_body(), 4096)
                .await
                .unwrap_or_default();
            (status, body)
        });

        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "NIP-FI Enforce: settings GET with valid NIP-98 + no assertion must deny 401 \
             [FI-TRACE-AUTHORITY-UNIFORM]. Falsifying mutation: replace \
             admit_nip_fi_http_on_state() in authenticate() with the raw bridge verifier — \
             a mismatched-key request would then be admitted."
        );
        // MissingEvidence body: "authentication required\n"
        assert_eq!(
            body.as_ref(),
            b"authentication required\n",
            "missing assertion must produce MissingEvidence body, not a NIP-98 auth challenge \
             or other error"
        );
    }

    // ── NIP-FI settings via build_router: key-pairing, OFF, POST protection ──
    //
    // These tests exercise the settings route through `build_router` (the full
    // relay router), which includes the `nip_fi_assertion_guard` middleware.
    // The key-pairing tests require a real `FederatedAssertionVerifier` seeded
    // with a static test key, so assertions signed by a known PKCS#8 key can
    // carry a chosen `nostr_pubkey` claim.
    //
    // ## Test key constants
    //
    // Same P-256 key as buzz-auth/src/nip_fi/verifier/tests.rs so
    // the construction pattern can be reviewed against a known-good example.
    const TEST_EC_PKCS8_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\nWZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\nzhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n-----END PRIVATE KEY-----\n"; // sadscan:disable np.pem.1 kingfisher.privkey.2 -- unchanged public ES256 test fixture
    const TEST_JWK_X: &str = "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI";
    const TEST_JWK_Y: &str = "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA";
    const TEST_KID: &str = "test-key-1";
    const TEST_ISSUER: &str = "https://issuer.test";
    const TEST_AUDIENCE: &str = "https://relay.test";

    /// Build an Enforce-mode state with a real `FederatedAssertionVerifier`
    /// seeded with the static test key.  Used for key-pairing tests.
    async fn enforce_state_with_verifier() -> Option<Arc<AppState>> {
        use buzz_auth::{
            FederatedAssertionVerifier, FreshnessClass, IssuerPolicy, IssuerRegistry,
            StaticIssuerKeySource, TokenClass, VerifyAssertion,
        };
        use jsonwebtoken::{jwk::JwkSet, Algorithm};

        let mut config = crate::config::Config::for_test();
        config.database_url = crate::test_support::database_url();
        config.redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        config.relay_url = "ws://nip-fi-settings-pairing-test.local".to_string();
        config.require_auth_token = true;
        config.require_relay_membership = false;
        config.nip_fi.mode = buzz_auth::NipFiMode::Enforce;
        config.nip_fi.communities =
            crate::nip_fi_core::test_support::any_host("https://relay.test");

        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).ok()?;
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .ok()?,
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;

        let (mut state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);

        // Build the verifier with a static key.
        let jwks: JwkSet = serde_json::from_value(serde_json::json!({
            "keys": [{
                "kty": "EC",
                "crv": "P-256",
                "use": "sig",
                "alg": "ES256",
                "kid": TEST_KID,
                "x": TEST_JWK_X,
                "y": TEST_JWK_Y
            }]
        }))
        .expect("valid test JWKS");

        let hard_deadline = chrono::Utc::now() + chrono::Duration::seconds(3600);
        let key_set = buzz_auth::AssertionKeySet::new_for_test(
            TEST_ISSUER.to_owned(),
            1,
            jwks,
            hard_deadline,
        )
        .expect("valid test key set");

        let jwks_contract = buzz_auth::JwksSourceContract::new(
            format!("{TEST_ISSUER}/.well-known/jwks.json"),
            300,
            3600,
        )
        .expect("valid jwks contract");

        let policy = IssuerPolicy::new(
            TEST_ISSUER.to_owned(),
            vec![TEST_AUDIENCE.to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![Algorithm::ES256],
            60,
            3600,
            None,
            jwks_contract,
        )
        .expect("valid issuer policy");

        let mut registry = IssuerRegistry::new();
        registry.insert(policy);

        let verifier: Arc<dyn VerifyAssertion> = Arc::new(FederatedAssertionVerifier::new(
            registry,
            StaticIssuerKeySource::new([key_set]),
        ));
        state.nip_fi_verifier = Some(verifier);

        Some(Arc::new(state))
    }

    /// Build an Off-mode state (no verifier needed).
    async fn off_state() -> Option<Arc<AppState>> {
        let mut config = crate::config::Config::for_test();
        config.database_url = crate::test_support::database_url();
        config.redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        config.relay_url = "ws://nip-fi-settings-off-test.local".to_string();
        config.require_auth_token = false;
        config.require_relay_membership = false;
        config.nip_fi.mode = buzz_auth::NipFiMode::Off;

        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).ok()?;
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .ok()?,
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;

        let (mut state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);

        Some(Arc::new(state))
    }

    /// Mint a signed ES256 NIP-FI assertion with the given `nostr_pubkey` claim.
    ///
    /// Uses the same static PKCS#8 PEM and key constants as the verifier above.
    fn mint_assertion(nostr_pubkey_hex: &str) -> String {
        use jsonwebtoken::{Algorithm, EncodingKey, Header};

        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "iss": TEST_ISSUER,
            "aud": TEST_AUDIENCE,
            "iat": now,
            "exp": now + 600,
            "sub": "test-subject",
            "nostr_pubkey": nostr_pubkey_hex,
        });
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(TEST_KID.to_owned());
        // NIP-FI dedicated assertion type
        header.typ = Some("nip-fi+jwt".to_owned());
        let key =
            EncodingKey::from_ec_pem(TEST_EC_PKCS8_PEM.as_bytes()).expect("valid test EC PEM");
        jsonwebtoken::encode(&header, &claims, &key).expect("sign assertion")
    }

    /// Drive a GET request through `build_router` for the settings path.
    /// Returns `(status, response_headers, body)` — helpers that previously
    /// discarded headers have been updated so callers can assert the full
    /// contract (Content-Type, WWW-Authenticate, etc.).
    async fn settings_get_via_build_router(
        state: Arc<AppState>,
        host: &str,
        path: &str,
        auth_token: &str,
        assertion: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, bytes::Bytes) {
        use axum::body::to_bytes;
        use axum::http::Request;
        use tower::ServiceExt;
        let mut builder = Request::builder()
            .method("GET")
            .uri(path)
            .header("host", host)
            .header("authorization", auth_token);
        if let Some(a) = assertion {
            builder = builder.header(buzz_auth::CLIENT_ATTACHED_HEADER, format!("Bearer {a}"));
        }
        let response = crate::router::build_router(state)
            .oneshot(
                builder
                    .body(axum::body::Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router oneshot");
        let status = response.status();
        let resp_headers = response.headers().clone();
        let body = to_bytes(response.into_body(), 4096)
            .await
            .unwrap_or_default();
        (status, resp_headers, body)
    }

    /// Drive a POST request through `build_router` for the settings path.
    /// Returns `(status, response_headers, body)`.
    async fn settings_post_via_build_router(
        state: Arc<AppState>,
        host: &str,
        path: &str,
        auth_token: &str,
        body_bytes: &[u8],
        assertion: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, bytes::Bytes) {
        use axum::body::to_bytes;
        use axum::http::Request;
        use tower::ServiceExt;
        let mut builder = Request::builder()
            .method("POST")
            .uri(path)
            .header("host", host)
            .header("authorization", auth_token)
            .header("content-type", "application/json");
        if let Some(a) = assertion {
            builder = builder.header(buzz_auth::CLIENT_ATTACHED_HEADER, format!("Bearer {a}"));
        }
        let response = crate::router::build_router(state)
            .oneshot(
                builder
                    .body(axum::body::Body::from(body_bytes.to_vec()))
                    .expect("build request"),
            )
            .await
            .expect("router oneshot");
        let status = response.status();
        let resp_headers = response.headers().clone();
        let body = to_bytes(response.into_body(), 4096)
            .await
            .unwrap_or_default();
        (status, resp_headers, body)
    }

    fn nip98_token_for_method(keys: &Keys, url: &str, method: &str, body: Option<&[u8]>) -> String {
        use sha2::{Digest, Sha256};
        let mut tags = vec![
            Tag::parse(["u", url]).expect("u tag"),
            Tag::parse(["method", method]).expect("method tag"),
            Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).expect("nonce tag"),
        ];
        if let Some(b) = body {
            let hex = hex::encode(Sha256::digest(b));
            tags.push(Tag::parse(["payload", &hex]).expect("payload tag"));
        }
        let event = EventBuilder::new(Kind::Custom(27235), "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign NIP-98 event");
        format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&event).unwrap())
        )
    }

    // ── Settings via build_router: Enforce + valid-assertion-key-A + NIP-98-key-B → 403 ──
    //
    // The assertion claims key-A (`nostr_pubkey = pubkey_a`).  The NIP-98 is
    // signed by key-B.  `admit_nip_fi_http` Step 6 (key pairing) fires → 403
    // authorization_denied.
    //
    // Falsifying mutation: remove the key-pairing check in `admit_nip_fi_http`
    // (the `Some(k) if k == proven_pubkey` match arm).  The admission succeeds
    // → the handler returns a non-403 response (404 for a missing repo) →
    // this test's `assert_eq(403)` fires.
    #[test]
    #[ignore = "requires Postgres"]
    fn settings_build_router_enforce_key_mismatch_denied_403() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(enforce_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-settings-pairing-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let key_a = Keys::generate();
        let key_b = Keys::generate();

        // Assertion claims key-A.
        let assertion = mint_assertion(&key_a.public_key().to_hex());
        // NIP-98 signed by key-B.
        let path = format!(
            "/git/{}/test-repo/default-branch",
            key_a.public_key().to_hex()
        );
        let url = format!("http://{host}{path}");
        let auth = nip98_get_token(&key_b, &url);

        let (status, resp_headers, body) = rt.block_on(settings_get_via_build_router(
            state,
            &host,
            &path,
            &auth,
            Some(&assertion),
        ));

        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "NIP-FI Enforce: assertion-for-A + NIP-98-for-B must deny 403 authorization_denied \
             [FI-INV-05]. Falsifying mutation: remove key-pairing check in admit_nip_fi_http → \
             admission succeeds → non-403 response."
        );
        assert_eq!(
            body.as_ref(),
            b"authorization denied\n",
            "key mismatch denial MUST produce authorization_denied body bytes"
        );
        assert_eq!(
            resp_headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
            "text/plain; charset=utf-8",
            "key mismatch 403 Content-Type MUST be text/plain; charset=utf-8."
        );
        assert!(
            resp_headers.get("www-authenticate").is_none(),
            "key mismatch 403 MUST NOT carry WWW-Authenticate."
        );
    }

    // ── Settings via build_router: Enforce + same-key assertion + NIP-98 → not-403 ──
    //
    // Positive control: assertion and NIP-98 both prove the same key → key
    // pairing passes.  The handler proceeds to the repository lookup → 404
    // (no such repo) or 200.  Either way, NOT 403 authorization_denied.
    //
    // Without this positive control an always-denying implementation would
    // satisfy the negative tests above while being broken.
    #[test]
    #[ignore = "requires Postgres"]
    fn settings_build_router_enforce_same_key_not_denied() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(enforce_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-settings-samekey-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let key = Keys::generate();

        // Assertion claims the same key that signs the NIP-98.
        let assertion = mint_assertion(&key.public_key().to_hex());
        let path = format!(
            "/git/{}/test-repo/default-branch",
            key.public_key().to_hex()
        );
        let url = format!("http://{host}{path}");
        let auth = nip98_get_token(&key, &url);

        let (status, _resp_headers, body) = rt.block_on(settings_get_via_build_router(
            state,
            &host,
            &path,
            &auth,
            Some(&assertion),
        ));

        // Admission passes → handler proceeds to repo lookup → repo does not
        // exist in the test DB → 404.
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "NIP-FI Enforce: same-key assertion + NIP-98 MUST reach handler → 404 \
             (repo does not exist). \
             If 403: key pairing is wrongly denying, or the land refused. \
             If 401: NIP-FI outer guard is wrongly denying a valid assertion. \
             Body: {body:?}"
        );
    }

    // ── Settings via build_router: POST — wrong method (GET token) → 403 ───────
    //
    // A GET NIP-98 token WITH a payload hash matching the POST body (method=GET,
    // payload tag present) on a POST request: method mismatch fails NIP-98 verification.
    // In NIP-FI Enforce mode,
    // `admit_nip_fi_http` maps a present-but-failing NIP-98 to
    // `EvidenceRejected` (403 "evidence rejected\n"), not 401.
    //
    // Falsifying mutation: change `EvidenceRejected` to `MissingEvidence` in
    // `admit_nip_fi_http` → returns 401 → assertion fires.
    #[test]
    #[ignore = "requires Postgres"]
    fn settings_build_router_enforce_post_wrong_method_is_403() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(enforce_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-settings-post-wrong-method-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let key = Keys::generate();
        let assertion = mint_assertion(&key.public_key().to_hex());
        let path = format!(
            "/git/{}/test-repo/default-branch",
            key.public_key().to_hex()
        );
        let url = format!("http://{host}{path}");

        // GET token WITH the correct POST body hash: Authorization header IS
        // present, payload tag exists (passes bridge.rs require_payload check),
        // but method=GET on a POST request fails NIP-98 method verification →
        // EvidenceRejected (403). Using nip98_get_token (no payload tag) would
        // have the token rejected earlier (missing-payload at bridge.rs:125-137)
        // before method validation runs.
        let post_body = b"{\"branch\":\"main\",\"expected_manifest\":\"abc\"}";
        let get_token = nip98_token_for_method(&key, &url, "GET", Some(post_body));

        let (status, resp_headers, body) = rt.block_on(settings_post_via_build_router(
            state,
            &host,
            &path,
            &get_token,
            post_body,
            Some(&assertion),
        ));

        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "NIP-FI Enforce: GET token on a POST settings request must deny 403 EvidenceRejected \
             (present but invalid Authorization → EvidenceRejected, not MissingEvidence). \
             Falsifying mutation: flip the DenialClass mapping for present-but-failing NIP-98 \
             to MissingEvidence → 401 → assertion fires."
        );
        assert_eq!(
            body.as_ref(),
            b"evidence rejected\n",
            "EvidenceRejected body must be exact contract bytes"
        );
        assert_eq!(
            resp_headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
            "text/plain; charset=utf-8",
            "EvidenceRejected 403 Content-Type MUST be text/plain; charset=utf-8."
        );
        assert!(
            resp_headers.get("www-authenticate").is_none(),
            "EvidenceRejected 403 MUST NOT carry WWW-Authenticate."
        );
    }

    // ── Settings via build_router: POST — correct method, no payload tag → 403 ─
    //
    // A POST NIP-98 token without a payload tag is present but invalid
    // (settings POST requires a hash-bound body per NIP-FI.md:619-637).
    // In Enforce mode, present-but-failing NIP-98 → EvidenceRejected (403).
    //
    // Falsifying mutation: set `require_payload = false` in `authenticate()`
    // for POST → payload-tag check skipped → NIP-98 succeeds → non-403 status.
    #[test]
    #[ignore = "requires Postgres"]
    fn settings_build_router_enforce_post_missing_payload_tag_is_403() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(enforce_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-settings-post-no-payload-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let key = Keys::generate();
        let assertion = mint_assertion(&key.public_key().to_hex());
        let path = format!(
            "/git/{}/test-repo/default-branch",
            key.public_key().to_hex()
        );
        let url = format!("http://{host}{path}");

        // POST token with correct method but no payload tag: settings handler
        // requires a hash-bound body; missing tag → NIP-98 fails → 403.
        let post_token_no_payload = nip98_token_for_method(&key, &url, "POST", None);
        let post_body = b"{\"branch\":\"main\",\"expected_manifest\":\"abc\"}";

        let (status, resp_headers, body) = rt.block_on(settings_post_via_build_router(
            state,
            &host,
            &path,
            &post_token_no_payload,
            post_body,
            Some(&assertion),
        ));

        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "NIP-FI Enforce: POST token without payload tag must deny 403 EvidenceRejected \
             (present but missing payload tag → NIP-98 failure → EvidenceRejected). \
             Falsifying mutation: remove require_payload=true from authenticate() → tag \
             check skipped → different status."
        );
        assert_eq!(
            body.as_ref(),
            b"evidence rejected\n",
            "EvidenceRejected body must be exact contract bytes"
        );
        assert_eq!(
            resp_headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
            "text/plain; charset=utf-8",
            "Missing-payload 403 Content-Type MUST be text/plain; charset=utf-8."
        );
    }

    // ── Settings via build_router: POST — same-key, valid payload → reaches handler ─
    //
    // A correctly signed POST NIP-98 token (method=POST, payload tag matching
    // the body) with a matching assertion passes NIP-FI admission and reaches
    // the handler.  The handler returns a non-NIP-FI error (404 repo not found
    // or similar), proving admission succeeded.
    //
    // Positive control: without this, an always-denying admission implementation
    // could pass all three POST tests above without testing real admission.
    //
    // Falsifying mutation: remove `admit_nip_fi_http_on_state` from
    // `authenticate()` → admission skipped → but request still reaches handler
    // (non-denial result), so the positive control would not fire.  Combined
    // with the negative controls above, the full set distinguishes correct
    // admission from both always-deny and always-admit implementations.
    #[test]
    #[ignore = "requires Postgres"]
    fn settings_build_router_enforce_post_same_key_valid_payload_reaches_handler() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(enforce_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-settings-post-ok-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let key = Keys::generate();
        let assertion = mint_assertion(&key.public_key().to_hex());
        let path = format!(
            "/git/{}/test-repo/default-branch",
            key.public_key().to_hex()
        );
        let url = format!("http://{host}{path}");
        let post_body = b"{\"branch\":\"main\",\"expected_manifest\":\"abc\"}";

        // Correct POST token: method=POST + payload tag hash-bound to the body.
        let post_token = nip98_token_for_method(&key, &url, "POST", Some(post_body));

        let (status, _resp_headers, _body) = rt.block_on(settings_post_via_build_router(
            state,
            &host,
            &path,
            &post_token,
            post_body,
            Some(&assertion),
        ));

        // Admission passes; handler reaches `authorize_git_read` which returns
        // 404 (no repo in this fresh community).  NIP-FI denial codes are 401/403,
        // not 404 — so 404 proves the NIP-FI gate passed and the handler ran.
        //
        // Note: quota checking (`enforce_http_admission`) follows NIP-FI admission
        // at settings.rs:204-206.  A 503 from Redis/quota outage would follow
        // admission, not precede it — but Redis availability is verified by the
        // non-503 assertion below.
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "NIP-FI Enforce: same-key valid POST token MUST reach the handler and return 404 \
             (repo not found in fresh community). \
             401 = NIP-FI admission blocked; 403 = key pairing failed; \
             either means admission did not pass. \
             Falsifying mutation: make verifier always-deny → 403 instead of 404."
        );
        assert_ne!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "NIP-FI Enforce: same-key valid POST token MUST NOT return 503. \
             503 means quota or Redis outage after NIP-FI admission — \
             ensure Redis is reachable for this test."
        );
    }

    // ── Settings via build_router: Off mode + valid NIP-98 → not blocked ─────
    //
    // Off mode must not apply NIP-FI admission.  A valid NIP-98 GET request
    // (no assertion) reaches the handler and gets a non-NIP-FI result.
    //
    // Falsifying mutation: change Off-mode to Enforce → NIP-FI guard fires →
    // 401 MissingEvidence → assertion fires (non-401 expected).
    #[test]
    #[ignore = "requires Postgres"]
    fn settings_build_router_off_mode_valid_nip98_reaches_handler() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(off_state()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-settings-off-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let key = Keys::generate();
        let path = format!(
            "/git/{}/test-repo/default-branch",
            key.public_key().to_hex()
        );
        let url = format!("http://{host}{path}");
        let auth = nip98_get_token(&key, &url);

        let (status, _resp_headers, _body) = rt.block_on(settings_get_via_build_router(
            state, &host, &path, &auth,
            None, // No assertion — Off mode must not require one.
        ));

        // In Off mode: NIP-FI guard does not fire; request reaches handler.
        // The handler returns 404 (no repo in fresh community) — a non-NIP-FI response.
        // 404 proves the request was not blocked by NIP-FI admission.
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "Off mode: a valid NIP-98 GET with no assertion MUST reach the handler and \
             return 404 (repo not found in fresh community). \
             401 means NIP-FI fired (Off mode incorrectly applying active-mode guard). \
             Falsifying mutation: set mode=Enforce → guard fires → 401."
        );
    }

    // ── Settings via build_router: Enforce + POST + key-mismatch → 403 ────────
    //
    // A valid assertion for key-A, but the Authorization header is NIP-98-signed
    // by key-B (different key), is a pairing mismatch.  In NIP-FI Enforce mode,
    // `admit_nip_fi_http` maps `AuthorizationDenied` to 403 `authorization denied\n`.
    //
    // This proves the pairing gate fires BEFORE any repo lookup — the path
    // `/git/{key-B-hex}/test-repo/default-branch` uses key-B as owner, so
    // a mismatch is caught at admission.
    //
    // Falsifying mutation: remove the pairing check from `admit_nip_fi_http` →
    // the request reaches the repo-not-found handler → 404 → assertion fires.
    //
    // Complement: the `settings_build_router_enforce_key_mismatch_denied_403`
    // test above covers GET key-mismatch with the GET proof; this covers POST
    // key-mismatch with a payload-bound proof.
    #[test]
    #[ignore = "requires Postgres"]
    fn settings_build_router_enforce_post_key_mismatch_is_403() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(enforce_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-settings-post-mismatch-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        // key_a: the assertion's nostr_pubkey (the "identity" identity).
        // key_b: the NIP-98 signer (different key — mismatch).
        let key_a = Keys::generate();
        let key_b = Keys::generate();
        let assertion = mint_assertion(&key_a.public_key().to_hex());

        // Path uses key_a as owner to make it a plausible repo path.
        let path = format!(
            "/git/{}/test-repo/default-branch",
            key_a.public_key().to_hex()
        );
        let url = format!("http://{host}{path}");
        let post_body = b"{\"branch\":\"main\",\"expected_manifest\":\"abc\"}";

        // key_b signs the NIP-98 token; assertion claims key_a.  Mismatch.
        let post_token = nip98_token_for_method(&key_b, &url, "POST", Some(post_body));

        let (status, resp_headers, body) = rt.block_on(settings_post_via_build_router(
            state,
            &host,
            &path,
            &post_token,
            post_body,
            Some(&assertion),
        ));

        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Enforce mode POST: key-mismatch (key_b NIP-98 vs key_a assertion) MUST return 403. \
             Falsifying mutation: remove pairing check → request reaches handler → 404."
        );
        assert_eq!(
            body.as_ref(),
            b"authorization denied\n",
            "key-mismatch POST 403 body must be exact 'authorization denied\\n'"
        );
        assert_eq!(
            resp_headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
            "text/plain; charset=utf-8",
            "key-mismatch POST 403 Content-Type must be text/plain; charset=utf-8"
        );
        assert!(
            resp_headers.get("www-authenticate").is_none(),
            "key-mismatch POST 403 MUST NOT carry WWW-Authenticate"
        );
    }

    // ── Settings via build_router: Enforce + POST + wrong payload hash → 403 ─
    //
    // A valid assertion for key-A, NIP-98 signed by key-A (same key), but the
    // NIP-98 token's `payload` tag has a SHA-256 that does NOT match the actual
    // request body.  `admit_nip_fi_http` enforces payload binding and denies with
    // `EvidenceRejected` 403.
    //
    // This proves the payload-hash verification is active independently of key
    // pairing — a wrong hash is caught before any repo lookup.
    //
    // Falsifying mutation: remove payload-hash verification from
    // `make_nip98_closure_for_admission` → wrong hash passes → handler reached
    // → 404 instead of 403 → assertion fires.
    #[test]
    #[ignore = "requires Postgres"]
    fn settings_build_router_enforce_post_wrong_payload_hash_is_403() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(enforce_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-settings-post-hash-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let key = Keys::generate();
        let assertion = mint_assertion(&key.public_key().to_hex());

        let path = format!(
            "/git/{}/test-repo/default-branch",
            key.public_key().to_hex()
        );
        let url = format!("http://{host}{path}");
        let actual_body = b"{\"branch\":\"main\",\"expected_manifest\":\"abc\"}";
        let wrong_body = b"{\"branch\":\"wrong-branch\",\"expected_manifest\":\"xyz\"}";

        // Token is signed against wrong_body's hash, but we send actual_body.
        // The token claims the hash of wrong_body, so the payload tag doesn't
        // match actual_body → EvidenceRejected.
        let wrong_hash_token = nip98_token_for_method(&key, &url, "POST", Some(wrong_body));

        let (status, resp_headers, body) = rt.block_on(settings_post_via_build_router(
            state,
            &host,
            &path,
            &wrong_hash_token,
            actual_body,
            Some(&assertion),
        ));

        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Enforce mode POST: wrong payload hash (token bound to different body) MUST return 403. \
             Falsifying mutation: remove payload-hash check → wrong hash passes → \
             request reaches handler → 404 instead of 403."
        );
        assert_eq!(
            body.as_ref(),
            b"evidence rejected\n",
            "Wrong payload hash 403 body MUST be exact 'evidence rejected\\n'. \
             [FI-TRACE-DENIAL-ORACLE]"
        );
        assert_eq!(
            resp_headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
            "text/plain; charset=utf-8",
            "Wrong payload hash 403 Content-Type MUST be text/plain; charset=utf-8."
        );
        assert!(
            resp_headers.get("www-authenticate").is_none(),
            "Wrong payload hash 403 MUST NOT carry WWW-Authenticate."
        );
    }

    // ── Settings via build_router: Off mode + valid NIP-98 POST → reaches handler ─
    //
    // Off mode must not apply NIP-FI admission on POST.  A valid NIP-98 POST
    // (no assertion) reaches the handler and gets a non-NIP-FI result (404).
    //
    // Falsifying mutation: change Off mode to Enforce → NIP-FI guard fires →
    // 401 MissingEvidence → assertion fires (expected 404).
    #[test]
    #[ignore = "requires Postgres"]
    fn settings_build_router_off_mode_post_reaches_handler() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(off_state()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-settings-off-post-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let key = Keys::generate();
        let path = format!(
            "/git/{}/test-repo/default-branch",
            key.public_key().to_hex()
        );
        let url = format!("http://{host}{path}");
        let post_body = b"{\"branch\":\"main\",\"expected_manifest\":\"abc\"}";
        let post_token = nip98_token_for_method(&key, &url, "POST", Some(post_body));

        let (status, _resp_headers, _body) = rt.block_on(settings_post_via_build_router(
            state,
            &host,
            &path,
            &post_token,
            post_body,
            None, // No assertion — Off mode must not require one.
        ));

        // In Off mode: NIP-FI guard does not fire → reaches handler → 404 (no repo).
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "Off mode POST: a valid NIP-98 POST with no assertion MUST reach the handler \
             and return 404 (repo not found in fresh community). \
             401 means NIP-FI fired (Off mode incorrectly applying active-mode guard). \
             Falsifying mutation: set mode=Enforce → guard fires → 401."
        );
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn smart_git_records_the_presented_owner_before_crediting_a_role() {
        use axum::extract::FromRequestParts;
        use buzz_core::channel::MemberRole;
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
                .await
                .unwrap();
            buzz_db::Db::from_pool(pool).migrate().await.unwrap();
        }
        let state = off_state().await.unwrap();
        let host = format!("owner-admission-{}.example", uuid::Uuid::new_v4().simple());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .unwrap()
            .id;
        let agent = Keys::generate();
        let owner = Keys::generate();
        let creator = Keys::generate();
        state
            .db
            .ensure_user(community, agent.public_key().as_bytes())
            .await
            .unwrap();
        let channel = uuid::Uuid::new_v4();
        state
            .db
            .create_channel_with_id(
                community,
                channel,
                "owner-admission",
                buzz_db::channel::ChannelType::Stream,
                buzz_db::channel::ChannelVisibility::Open,
                None,
                creator.public_key().as_bytes(),
                None,
            )
            .await
            .unwrap();
        state
            .db
            .add_member(
                community,
                channel,
                agent.public_key().as_bytes(),
                MemberRole::Admin,
                Some(creator.public_key().as_bytes()),
            )
            .await
            .unwrap();
        let path = format!(
            "/git/{}/repo/info/refs?service=git-upload-pack",
            agent.public_key().to_hex()
        );
        let url = format!("http://{host}/git/{}/repo", agent.public_key().to_hex());
        let request = |owner: &Keys| {
            let auth = buzz_sdk::nip_oa::compute_auth_tag(owner, &agent.public_key(), "").unwrap();
            let parts: Vec<String> = serde_json::from_str(&auth).unwrap();
            let event = EventBuilder::new(Kind::Custom(27235), "")
                .tags([
                    Tag::parse(["u", &url]).unwrap(),
                    Tag::parse(["method", "GET"]).unwrap(),
                    Tag::parse(parts).unwrap(),
                ])
                .sign_with_keys(&agent)
                .unwrap();
            let token = format!(
                "Nostr {}",
                base64::engine::general_purpose::STANDARD
                    .encode(serde_json::to_vec(&event).unwrap())
            );
            Request::builder()
                .uri(&path)
                .header("host", &host)
                .header("authorization", token)
                .body(Body::empty())
                .unwrap()
                .into_parts()
                .0
        };
        let authenticated = super::super::super::transport::GitAuth::from_request_parts(
            &mut request(&owner),
            &state,
        )
        .await;
        assert!(
            authenticated.is_ok(),
            "valid signed Git request must reach role authorization"
        );
        let role = super::super::super::authority::credited_role(
            &state.db,
            community,
            channel,
            &super::super::super::land::coordinate(&agent.public_key().to_hex(), "repo"),
            agent.public_key().as_bytes(),
            None,
            !state.config.require_relay_membership,
        )
        .await
        .unwrap();
        assert_eq!(
            role,
            Ok(MemberRole::Admin),
            "first presented owner is outside channel; independent admin membership remains"
        );

        // The recorded owner is first-write-wins: a different one is refused.
        let other = Keys::generate();
        let refused = super::super::super::transport::GitAuth::from_request_parts(
            &mut request(&other),
            &state,
        )
        .await;
        let status = refused.err().expect("a second owner is refused").status();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn a_restricted_land_preserves_owner_admission() {
        use axum::extract::FromRequestParts;
        use buzz_core::channel::MemberRole;
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
                .await
                .unwrap();
            buzz_db::Db::from_pool(pool).migrate().await.unwrap();
        }
        let mut state = off_state().await.unwrap();
        let host = format!("owner-admission-{}.example", uuid::Uuid::new_v4().simple());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .unwrap()
            .id;
        let agent = Keys::generate();
        let owner = Keys::generate();
        let creator = Keys::generate();
        state
            .db
            .ensure_user(community, agent.public_key().as_bytes())
            .await
            .unwrap();
        let channel = uuid::Uuid::new_v4();
        state
            .db
            .create_channel_with_id(
                community,
                channel,
                "owner-admission",
                buzz_db::channel::ChannelType::Stream,
                buzz_db::channel::ChannelVisibility::Open,
                None,
                creator.public_key().as_bytes(),
                None,
            )
            .await
            .unwrap();
        state
            .db
            .add_member(
                community,
                channel,
                agent.public_key().as_bytes(),
                MemberRole::Admin,
                Some(creator.public_key().as_bytes()),
            )
            .await
            .unwrap();
        let path = format!(
            "/git/{}/repo/info/refs?service=git-upload-pack",
            agent.public_key().to_hex()
        );
        let url = format!("http://{host}/git/{}/repo", agent.public_key().to_hex());
        let request = |owner: &Keys| {
            let auth = buzz_sdk::nip_oa::compute_auth_tag(owner, &agent.public_key(), "").unwrap();
            let parts: Vec<String> = serde_json::from_str(&auth).unwrap();
            let event = EventBuilder::new(Kind::Custom(27235), "")
                .tags([
                    Tag::parse(["u", &url]).unwrap(),
                    Tag::parse(["method", "GET"]).unwrap(),
                    Tag::parse(parts).unwrap(),
                ])
                .sign_with_keys(&agent)
                .unwrap();
            let token = format!(
                "Nostr {}",
                base64::engine::general_purpose::STANDARD
                    .encode(serde_json::to_vec(&event).unwrap())
            );
            Request::builder()
                .uri(&path)
                .header("host", &host)
                .header("authorization", token)
                .body(Body::empty())
                .unwrap()
                .into_parts()
                .0
        };
        let authenticated = super::super::super::transport::GitAuth::from_request_parts(
            &mut request(&owner),
            &state,
        )
        .await;
        assert!(
            authenticated.is_ok(),
            "valid signed Git request must reach role authorization"
        );
        let role = super::super::super::authority::credited_role(
            &state.db,
            community,
            channel,
            &super::super::super::land::coordinate(&agent.public_key().to_hex(), "repo"),
            agent.public_key().as_bytes(),
            None,
            !state.config.require_relay_membership,
        )
        .await
        .unwrap();
        assert_eq!(
            role,
            Ok(MemberRole::Admin),
            "first presented owner is outside channel; independent admin membership remains"
        );

        // The recorded owner is first-write-wins: a different one is refused.
        let other = Keys::generate();
        let refused = super::super::super::transport::GitAuth::from_request_parts(
            &mut request(&other),
            &state,
        )
        .await;
        let status = refused.err().expect("a second owner is refused").status();
        assert_eq!(status, StatusCode::FORBIDDEN);
        state
            .db
            .ensure_user(community, owner.public_key().as_bytes())
            .await
            .unwrap();
        state
            .db
            .add_member(
                community,
                channel,
                owner.public_key().as_bytes(),
                MemberRole::Admin,
                Some(creator.public_key().as_bytes()),
            )
            .await
            .unwrap();
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1,$2,'member')",
        )
        .bind(community.as_uuid())
        .bind(owner.public_key().to_hex())
        .execute(&pool)
        .await
        .unwrap();
        let config =
            Arc::make_mut(&mut Arc::get_mut(&mut state).expect("fixture owns state").config);
        config.require_relay_membership = true;
        config.allow_nip_oa_auth = true;
        assert!(
            super::super::super::transport::GitAuth::from_request_parts(
                &mut request(&owner),
                &state
            )
            .await
            .is_ok(),
            "owner admits agent on restricted relay"
        );
        let repo = buzz_core::governance::RepoCoordinate {
            owner: agent.public_key().to_hex(),
            name: "repo".into(),
        };
        state
            .db
            .reserve_repo_name(community, &repo.name, &repo.owner)
            .await
            .unwrap();
        let announcement = EventBuilder::new(Kind::Custom(30617), "")
            .tags([
                Tag::parse(["d", "repo"]).unwrap(),
                Tag::parse(["buzz-channel", &channel.to_string()]).unwrap(),
            ])
            .sign_with_keys(&agent)
            .unwrap();
        state
            .db
            .insert_event(community, &announcement, None)
            .await
            .unwrap();
        let guard = buzz_deletion::acquire_serving_write(&state.db, community, "restricted-land")
            .await
            .unwrap();
        let result = super::super::super::land::land(
            &state,
            guard.lease(),
            super::super::super::land::Change {
                kind: "push",
                repo: &repo,
                actor: agent.public_key().as_bytes(),
                grant: None,
                seen: None,
                new: "review-manifest",
                updates: &[],
            },
            async |_tx, authority| {
                assert_eq!(
                    authority.role,
                    Some(MemberRole::Admin),
                    "land must preserve valid owner admission and independent channel role"
                );
                Ok(super::super::super::gate::Decision {
                    verdict: Ok(()),
                    counted: Default::default(),
                })
            },
        )
        .await;
        assert!(result.is_ok());
        guard.finish().await.unwrap();

        // An admitted agent whose owner leaves the community is credited nothing.
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1,$2,'member')",
        )
        .bind(community.as_uuid())
        .bind(agent.public_key().to_hex())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM relay_members WHERE community_id=$1 AND pubkey=$2")
            .bind(community.as_uuid())
            .bind(owner.public_key().to_hex())
            .execute(&pool)
            .await
            .unwrap();
        let decided = std::sync::atomic::AtomicBool::new(false);
        let guard = buzz_deletion::acquire_serving_write(&state.db, community, "owner-left")
            .await
            .unwrap();
        let _ = super::super::super::land::land(
            &state,
            guard.lease(),
            super::super::super::land::Change {
                kind: "default_branch",
                repo: &repo,
                actor: agent.public_key().as_bytes(),
                grant: None,
                seen: Some("review-manifest"),
                new: "owner-left-manifest",
                updates: &[],
            },
            async |_tx, authority| {
                decided.store(true, std::sync::atomic::Ordering::SeqCst);
                assert_eq!(authority.role, None, "an unadmitted owner admits no agent");
                Ok(super::super::super::gate::Decision {
                    verdict: Ok(()),
                    counted: Default::default(),
                })
            },
        )
        .await;
        guard.finish().await.unwrap();
        assert!(decided.into_inner(), "the land must reach its decision");
    }
} // mod postgres_tests

mod external_infra {
    use super::super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use base64::Engine;
    use buzz_core::channel::MemberRole;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;

    struct Fixture {
        state: Arc<AppState>,
        pool: sqlx::PgPool,
        tenant: TenantContext,
        owner: Keys,
        member: Keys,
        maintainer: Keys,
        channel: uuid::Uuid,
        repo: String,
        scratch: tempfile::TempDir,
    }

    impl Fixture {
        async fn new() -> Self {
            let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
                .expect("explicit isolated BUZZ_TEST_DATABASE_URL");
            let redis_url = std::env::var("BUZZ_TEST_REDIS_URL")
                .expect("explicit isolated BUZZ_TEST_REDIS_URL");
            let endpoint = std::env::var("BUZZ_TEST_S3_ENDPOINT")
                .expect("explicit isolated BUZZ_TEST_S3_ENDPOINT");
            let scratch = tempfile::tempdir().unwrap();
            let mut config = crate::config::Config::for_test(); // [FI-TRACE-ENV-RACE]
            config.database_url = database_url;
            config.redis_url = redis_url;
            config.relay_url = "ws://127.0.0.1".into();
            config.require_relay_membership = false;
            config.git_repo_path = scratch.path().to_path_buf();
            config.git_pack_cache_path = scratch.path().join("cache");
            config.media.s3_endpoint = endpoint;
            config.media.s3_bucket =
                std::env::var("BUZZ_TEST_S3_BUCKET").unwrap_or_else(|_| "buzz-git".into());
            config.media.s3_access_key = "buzz_dev".into();
            config.media.s3_secret_key = "buzz_dev_secret".into();
            let pool = sqlx::PgPool::connect(&config.database_url).await.unwrap();
            let db = buzz_db::Db::from_pool(pool.clone());
            // CI provisions schema/schema.sql with pgschema before this suite.
            // Only migration-backed local fixtures own the migration lifecycle.
            if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
                db.migrate().await.unwrap();
            }
            let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).unwrap();
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .unwrap(),
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media = buzz_media::MediaStorage::new(&config.media).unwrap();
            let (state, _) = AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow,
                Keys::generate(),
                media,
            );
            let state = Arc::new(state);
            let host = format!("settings-{}.example", uuid::Uuid::new_v4().simple());
            let community = state
                .db
                .ensure_configured_community(&host)
                .await
                .unwrap()
                .id;
            let tenant = TenantContext::resolved(community, &host);
            let owner = Keys::generate();
            let member = Keys::generate();
            let maintainer = Keys::generate();
            let channel = uuid::Uuid::new_v4();
            state
                .db
                .ensure_user(community, owner.public_key().as_bytes())
                .await
                .unwrap();
            state
                .db
                .create_channel_with_id(
                    community,
                    channel,
                    &format!("settings-{channel}"),
                    buzz_db::channel::ChannelType::Stream,
                    buzz_db::channel::ChannelVisibility::Open,
                    None,
                    owner.public_key().as_bytes(),
                    None,
                )
                .await
                .unwrap();
            for (key, role) in [
                (&member, MemberRole::Admin),
                (&maintainer, MemberRole::Member),
                (&owner, MemberRole::Owner),
            ] {
                state
                    .db
                    .ensure_user(community, key.public_key().as_bytes())
                    .await
                    .unwrap();
                state
                    .db
                    .add_member(
                        community,
                        channel,
                        key.public_key().as_bytes(),
                        role,
                        Some(owner.public_key().as_bytes()),
                    )
                    .await
                    .unwrap();
            }
            let repo = format!("repo-{}", uuid::Uuid::new_v4().simple());
            let announcement = EventBuilder::new(Kind::Custom(30617), "")
                .tags([
                    Tag::parse(["d", &repo]).unwrap(),
                    Tag::parse(["buzz-channel", &channel.to_string()]).unwrap(),
                    Tag::parse(["maintainers", &maintainer.public_key().to_hex()]).unwrap(),
                ])
                .sign_with_keys(&owner)
                .unwrap();
            state
                .db
                .insert_event(community, &announcement, None)
                .await
                .unwrap();
            state
                .db
                .reserve_repo_name(community, &repo, &owner.public_key().to_hex())
                .await
                .unwrap();
            let f = Self {
                state,
                pool,
                tenant,
                owner,
                member,
                maintainer,
                channel,
                repo,
                scratch,
            };
            f.seed_git().await;
            f
        }

        fn path(&self) -> String {
            format!(
                "/git/{}/{}/default-branch",
                self.owner.public_key().to_hex(),
                self.repo
            )
        }

        async fn snapshot(&self) -> DefaultBranchSnapshot {
            DefaultBranchSnapshot::load(
                &self.state,
                &self.tenant,
                crate::api::git::land::coordinate(&self.owner.public_key().to_hex(), &self.repo),
            )
            .await
            .unwrap()
        }

        /// Set the default branch from `snapshot` as the owner, below the
        /// HTTP route, so a test can hold a stale snapshot.
        async fn set_from(
            &self,
            snapshot: DefaultBranchSnapshot,
            request: SetDefaultBranch,
        ) -> Result<(DefaultBranchSnapshot, bool), Response> {
            let serving_write = buzz_deletion::acquire_serving_write(
                &self.state.db,
                self.tenant.community(),
                "git_default_branch_test",
            )
            .await
            .unwrap();
            let result = snapshot
                .set(
                    &self.state,
                    &serving_write,
                    &self.owner.public_key().to_bytes(),
                    None,
                    request,
                )
                .await;
            serving_write.finish().await.unwrap();
            result
        }

        async fn seed_git(&self) {
            let source = self.scratch.path().join("source");
            std::fs::create_dir(&source).unwrap();
            git(&source, &["init", "--initial-branch=legacy"]).await;
            git(&source, &["config", "user.name", "Git settings test"]).await;
            git(
                &source,
                &["config", "user.email", "git-settings@example.invalid"],
            )
            .await;
            git(&source, &["commit", "--allow-empty", "-m", "legacy"]).await;
            git(&source, &["branch", "main"]).await;
            git(&source, &["checkout", "main"]).await;
            std::fs::write(source.join("main.txt"), b"selected branch\n").unwrap();
            git(&source, &["add", "main.txt"]).await;
            git(&source, &["commit", "-m", "main"]).await;
            git(&source, &["checkout", "legacy"]).await;
            super::super::super::cas_publish::cas_publish(
                &self.state.git_store,
                &self.tenant,
                &source,
                &self.owner.public_key().to_hex(),
                &self.repo,
                &super::super::super::cas_publish::ParentState::fresh(),
                &super::super::super::cas_publish::snapshot_workspace(&source)
                    .await
                    .unwrap(),
                limits(0),
            )
            .await
            .unwrap();
        }

        async fn call(
            &self,
            key: &Keys,
            body: Option<Value>,
            tag: Option<&str>,
        ) -> (StatusCode, Value) {
            let body = body.map(|value| value.to_string());
            let method = if body.is_some() { "POST" } else { "GET" };
            let path = self.path();
            let token = token(
                key,
                method,
                &format!("http://{}{path}", self.tenant.host()),
                body.as_deref(),
            );
            let mut request = Request::builder()
                .method(method)
                .uri(&path)
                .header("host", self.tenant.host())
                .header("authorization", token);
            if let Some(tag) = tag {
                request = request.header("x-auth-tag", tag);
            }
            let request = request.body(Body::from(body.unwrap_or_default())).unwrap();
            response(
                super::super::super::transport::git_router(self.state.clone())
                    .oneshot(request)
                    .await
                    .unwrap(),
            )
            .await
        }

        async fn set(&self, key: &Keys, branch: &str, tag: Option<&str>) -> (StatusCode, Value) {
            let digest = self.snapshot().await.digest;
            self.call(
                key,
                Some(json!({"branch": branch, "expected_manifest": digest})),
                tag,
            )
            .await
        }

        /// Push what `push` holds through the land, as the owner, as
        /// receive-pack's finalize does.
        async fn land_push(
            &self,
            push: &super::super::super::hydrate::HydratedRepo,
            parent: &super::super::super::cas_publish::ParentState,
        ) -> super::super::super::land::Verdict {
            self.land_push_as(&self.owner, None, push, parent).await
        }

        /// As `land_push`, as `pusher`, judged as receive-pack judges.
        async fn land_push_as(
            &self,
            pusher: &Keys,
            grant: Option<&buzz_core::governance::GitGrant>,
            push: &super::super::super::hydrate::HydratedRepo,
            parent: &super::super::super::cas_publish::ParentState,
        ) -> super::super::super::land::Verdict {
            let workspace = super::super::super::cas_publish::snapshot_workspace(push.path())
                .await
                .unwrap();
            let prepared = super::super::super::cas_publish::prepare_publish(
                &self.state.git_store,
                push.path(),
                parent,
                &workspace,
                limits(push.hydrated_bytes()),
            )
            .await
            .unwrap();
            let updates = super::super::super::land::ref_updates(
                push.path(),
                &parent.parent.refs,
                &workspace.refs,
            )
            .await;
            let scan =
                super::super::super::scan::push(push.path(), &parent.parent.refs, &workspace.refs)
                    .await;
            let serving_write = buzz_deletion::acquire_serving_write(
                &self.state.db,
                self.tenant.community(),
                "git_publish_test",
            )
            .await
            .unwrap();
            let repo = super::super::super::land::coordinate(
                &self.owner.public_key().to_hex(),
                &self.repo,
            );
            let pusher = pusher.public_key().to_bytes();
            let verdict = super::super::super::land::land(
                &self.state,
                serving_write.lease(),
                super::super::super::land::Change {
                    kind: "push",
                    repo: &repo,
                    actor: &pusher,
                    grant,
                    seen: parent.parent_digest.as_deref(),
                    new: prepared.digest(),
                    updates: &updates,
                },
                async |tx, authority| {
                    super::super::super::gate::push(
                        tx,
                        authority,
                        &repo,
                        &pusher,
                        &updates,
                        scan,
                        !self.state.config.require_relay_membership,
                    )
                    .await
                },
            )
            .await
            .unwrap();
            if verdict == super::super::super::land::Verdict::Landed {
                super::super::super::land::write_cache(
                    &self.state.git_store,
                    &self.tenant,
                    &repo,
                    parent.if_match.as_ref(),
                    prepared.digest(),
                )
                .await;
            }
            serving_write.finish().await.unwrap();
            verdict
        }

        /// Publish `body` for `document` as `signer`, through ingest; the
        /// relay-authored head it installed.
        async fn govern(
            &self,
            signer: &Keys,
            document: &str,
            expected: &buzz_core::governance::ExpectedHead,
            body: &str,
        ) -> nostr::EventId {
            let event = buzz_core::governance::command_builder(
                *self.tenant.community().as_uuid(),
                &document.parse().unwrap(),
                expected,
                body,
            )
            .unwrap()
            .sign_with_keys(signer)
            .unwrap();
            let auth = crate::handlers::ingest::IngestAuth::Http {
                pubkey: signer.public_key(),
                scopes: vec![buzz_auth::Scope::ReposWrite],
                auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
            };
            crate::handlers::ingest::ingest_event(&self.state, &self.tenant, event, auth)
                .await
                .unwrap_or_else(|e| panic!("{document}: {e:?}"));
            let head: String = sqlx::query_scalar(
                "SELECT detail->'state'->>'id' FROM audit_log WHERE community_id=$1 \
                 AND action='governance_changed' AND detail->>'document'=$2 ORDER BY seq DESC LIMIT 1",
            )
            .bind(self.tenant.community().as_uuid())
            .bind(document)
            .fetch_one(&self.pool)
            .await
            .unwrap();
            nostr::EventId::from_hex(&head).unwrap()
        }

        /// Publish the community's floor, as its admin, so grants can be
        /// issued under its lifetime cap.
        async fn floor(&self) {
            let admin = self.owner.public_key().to_hex();
            sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'admin') ON CONFLICT (community_id,pubkey) DO UPDATE SET role='admin'")
                .bind(self.tenant.community().as_uuid()).bind(&admin).execute(&self.pool).await.unwrap();
            self.govern(
                &self.owner,
                "floor",
                &buzz_core::governance::ExpectedHead::Absent,
                r#"{"version":1,"body":{"type":"floor","rules":[],"labels":{},"max_grant_seconds":3600}}"#,
            )
            .await;
        }

        /// As `owner`, a Git grant tag for `agent` at `level` on the fixture
        /// repo, signed now, from a minute ago until `seconds` from now.
        fn grant(&self, owner: &Keys, agent: &Keys, level: &str, seconds: i64) -> String {
            let now = chrono::Utc::now().timestamp();
            buzz_sdk::nip_oa::compute_git_grant(
                owner,
                &agent.public_key(),
                &format!(
                    "repo={}:{}&git={level}&created_at>{}&created_at<{}",
                    self.owner.public_key().to_hex(),
                    self.repo,
                    now - 60,
                    now + seconds
                ),
            )
            .unwrap()
        }

        /// The doors of every recorded read of the fixture repo by `key`, in order.
        async fn reads(&self, key: &Keys) -> Vec<String> {
            sqlx::query_scalar(
                "SELECT detail->>'door' FROM audit_log WHERE community_id=$1 \
                 AND action='git_read' AND object_id=$2 AND actor_pubkey=$3 ORDER BY seq",
            )
            .bind(self.tenant.community().as_uuid())
            .bind(
                super::super::super::land::coordinate(
                    &self.owner.public_key().to_hex(),
                    &self.repo,
                )
                .to_string(),
            )
            .bind(key.public_key().to_bytes().to_vec())
            .fetch_all(&self.pool)
            .await
            .unwrap()
        }

        /// Ask upload-pack at `host`, as the member, for `tip`: served its
        /// pack exactly when `served`, else refused as not a ref it has.
        async fn assert_served(&self, host: &str, tip: &str, served: bool, label: &str) {
            let owner = self.owner.public_key().to_hex();
            let url = format!("http://{host}/git/{owner}/{}", self.repo);
            let want = format!("want {tip}\n");
            let request = Request::builder()
                .method("POST")
                .uri(format!("/git/{owner}/{}/git-upload-pack", self.repo))
                .header("host", host)
                .header("authorization", token(&self.member, "GET", &url, None))
                .body(Body::from(format!(
                    "{:04x}{want}00000009done\n",
                    want.len() + 4
                )))
                .unwrap();
            let response = super::super::super::transport::git_router(self.state.clone())
                .oneshot(request)
                .await
                .unwrap();
            let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
                .await
                .unwrap();
            let shown = String::from_utf8_lossy(&bytes);
            assert_eq!(
                bytes.windows(4).any(|w| w == b"PACK"),
                served,
                "{label}: {shown}"
            );
            if !served {
                assert!(
                    shown.contains(&format!("not our ref {tip}")),
                    "{label}: {shown}"
                );
            }
        }

        /// POST `filter` to the bridge's `/query` as `key`; the status.
        async fn query(&self, key: &Keys, filter: Value) -> StatusCode {
            let body = filter.to_string();
            let url = crate::api::bridge::nip98_expected_url(
                &self.state.config.relay_url,
                &self.tenant,
                "/query",
            );
            let request = Request::builder()
                .method("POST")
                .uri("/query")
                .header("host", self.tenant.host())
                .header("authorization", token(key, "POST", &url, Some(&body)))
                .body(Body::from(body))
                .unwrap();
            crate::router::build_router(self.state.clone())
                .oneshot(request)
                .await
                .unwrap()
                .status()
        }

        async fn add(&self, key: &Keys) {
            self.state
                .db
                .ensure_user(self.tenant.community(), key.public_key().as_bytes())
                .await
                .unwrap();
            self.state
                .db
                .add_member(
                    self.tenant.community(),
                    self.channel,
                    key.public_key().as_bytes(),
                    MemberRole::Bot,
                    Some(self.owner.public_key().as_bytes()),
                )
                .await
                .unwrap();
        }
    }

    fn limits(parent_hydrated_bytes: u64) -> super::super::super::cas_publish::PublishLimits {
        super::super::super::cas_publish::PublishLimits {
            parent_hydrated_bytes,
            max_pack_bytes: 1024 * 1024,
            max_repo_bytes: 2 * 1024 * 1024,
        }
    }

    async fn git(path: &std::path::Path, args: &[&str]) -> String {
        let mut command = tokio::process::Command::new("git");
        command.current_dir(path).args(args);
        super::super::super::transport::harden_git_env(&mut command);
        let result = command.output().await.unwrap();
        assert!(
            result.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    }

    fn token(keys: &Keys, method: &str, url: &str, body: Option<&str>) -> String {
        token_with_payload(
            keys,
            method,
            url,
            body.map(|body| Tag::parse(["payload", &hex::encode(Sha256::digest(body))]).unwrap()),
        )
    }

    fn token_with_payload(keys: &Keys, method: &str, url: &str, payload: Option<Tag>) -> String {
        let mut tags = vec![
            Tag::parse(["u", url]).unwrap(),
            Tag::parse(["method", method]).unwrap(),
            Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
        ];
        if let Some(payload) = payload {
            tags.push(payload);
        }
        let event = EventBuilder::new(Kind::Custom(27235), "")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap();
        format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&event).unwrap())
        )
    }

    async fn response(response: Response) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| json!({"error": String::from_utf8_lossy(&bytes)})),
        )
    }

    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn default_branch_route_permissions_and_protocol() {
        let f = Fixture::new().await;
        let before = f.snapshot().await;
        assert_eq!(
            f.call(&f.member, None, None).await.1["head"],
            "refs/heads/legacy"
        );
        assert_eq!(
            f.set(&f.maintainer, "main", None).await.0,
            StatusCode::FORBIDDEN,
            "being named in the announcement grants nothing; members do not manage"
        );
        assert_eq!(
            f.set(&Keys::generate(), "main", None).await.0,
            StatusCode::NOT_FOUND
        );
        for branch in [
            "",
            "absent",
            "../main",
            "refs/heads/main",
            "main.lock",
            "bad\nref",
            "main/",
            "-main",
            ".main",
        ] {
            assert_eq!(
                f.set(&f.owner, branch, None).await.0,
                StatusCode::BAD_REQUEST,
                "{branch:?}"
            );
        }
        assert_eq!(
            f.snapshot().await.digest,
            before.digest,
            "denials do not write"
        );
        let result = f.set(&f.member, "main", None).await;
        assert_eq!(
            result.0,
            StatusCode::OK,
            "channel admins manage: {result:?}"
        );
        assert_eq!(result.1["changed"], true);
        let after = f.snapshot().await;
        assert_eq!(after.manifest.head, "refs/heads/main");
        assert_eq!(after.manifest.refs, before.manifest.refs);
        assert_eq!(after.manifest.packs, before.manifest.packs);
        assert_eq!(after.manifest.parent.as_ref(), Some(&before.digest));
        let result = f.set(&f.owner, "main", None).await;
        assert_eq!(result.0, StatusCode::OK);
        assert_eq!(result.1["changed"], false);
        assert_eq!(f.snapshot().await.digest, after.digest);
        assert_eq!(
            f.call(
                &f.owner,
                Some(json!({"branch":"legacy", "expected_manifest": before.digest})),
                None
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        let notification_query = buzz_db::EventQuery {
            kinds: Some(vec![30618]),
            d_tag: Some(f.repo.clone()),
            global_only: true,
            ..buzz_db::EventQuery::for_community(f.tenant.community())
        };
        let events = f.state.db.query_events(&notification_query).await.unwrap();
        let event_ids: Vec<_> = events.iter().map(|e| e.event.id).collect();
        assert!(
            events.iter().any(|e| e
                .event
                .tags
                .iter()
                .any(|t| t.as_slice() == ["HEAD", "ref: refs/heads/main"])),
            "committed default notification: {events:?}"
        );

        // Strict credentials: each mutated property must be rejected at the real route.
        let body = json!({"branch":"legacy", "expected_manifest": after.digest}).to_string();
        let path = f.path();
        let url = format!("http://{}{path}", f.tenant.host());
        let requests = [
            token_with_payload(
                &f.owner,
                "POST",
                &url,
                Some(Tag::parse(["payload"]).unwrap()),
            ),
            token_with_payload(
                &f.owner,
                "POST",
                &url,
                Some(Tag::parse(["payload", ""]).unwrap()),
            ),
            token(&f.owner, "GET", &url, Some(&body)),
            token(&f.owner, "POST", &url, None),
            token(&f.owner, "POST", &url, Some("{}")),
            token(
                &f.owner,
                "POST",
                &url.replace(f.tenant.host(), "other.example"),
                Some(&body),
            ),
            token(
                &f.owner,
                "GET",
                url.trim_end_matches("/default-branch"),
                None,
            ),
        ];
        for token in requests {
            let request = Request::builder()
                .method("POST")
                .uri(&path)
                .header("host", f.tenant.host())
                .header("authorization", token)
                .body(Body::from(body.clone()))
                .unwrap();
            let status = super::super::super::transport::git_router(f.state.clone())
                .oneshot(request)
                .await
                .unwrap()
                .status();
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(
                f.snapshot().await.digest,
                after.digest,
                "auth denial changed pointer"
            );
            let denied_events = f.state.db.query_events(&notification_query).await.unwrap();
            assert_eq!(
                denied_events.iter().map(|e| e.event.id).collect::<Vec<_>>(),
                event_ids,
                "auth denial published kind:30618"
            );
        }
        let reusable = token(&f.owner, "GET", &url, None);
        for expected in [StatusCode::OK, StatusCode::UNAUTHORIZED] {
            let request = Request::builder()
                .uri(&path)
                .header("host", f.tenant.host())
                .header("authorization", &reusable)
                .body(Body::empty())
                .unwrap();
            assert_eq!(
                super::super::super::transport::git_router(f.state.clone())
                    .oneshot(request)
                    .await
                    .unwrap()
                    .status(),
                expected
            );
        }
        let other_host = format!("other-{}.example", uuid::Uuid::new_v4());
        f.state
            .db
            .ensure_configured_community(&other_host)
            .await
            .unwrap();
        let token = token(&f.owner, "GET", &format!("http://{other_host}{path}"), None);
        let request = Request::builder()
            .uri(&path)
            .header("host", &other_host)
            .header("authorization", token)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            super::super::super::transport::git_router(f.state.clone())
                .oneshot(request)
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn default_branch_owner_attestation_preserves_member_authority() {
        let f = Fixture::new().await;
        // Recording an owner preserves independently held channel authority.
        assert_eq!(f.set(&f.member, "main", None).await.0, StatusCode::OK);
        let unrelated = Keys::generate();
        let tag =
            buzz_sdk::nip_oa::compute_auth_tag(&unrelated, &f.member.public_key(), "").unwrap();
        assert_eq!(
            f.set(&f.member, "legacy", Some(&tag)).await.0,
            StatusCode::OK
        );
        assert_eq!(
            f.set(&f.member, "legacy", None).await.0,
            StatusCode::OK,
            "recorded ownership does not narrow independent membership"
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn default_branch_delegation_and_revocation() {
        let f = Fixture::new().await;
        let community = f.tenant.community();
        let put = |key: &Keys, role| {
            let (state, channel, key, by) = (
                Arc::clone(&f.state),
                f.channel,
                key.public_key(),
                f.owner.public_key(),
            );
            async move {
                state
                    .db
                    .add_member(
                        community,
                        channel,
                        key.as_bytes(),
                        role,
                        Some(by.as_bytes()),
                    )
                    .await
                    .unwrap();
            }
        };
        // An owner's credential does not lend the owner's authority: a bot
        // writes as a member, and members do not manage.
        f.floor().await;
        let bot = Keys::generate();
        f.add(&bot).await;
        let tag = buzz_sdk::nip_oa::compute_auth_tag(&f.owner, &bot.public_key(), "").unwrap();
        assert_eq!(
            f.set(&bot, "main", Some(&tag)).await.0,
            StatusCode::FORBIDDEN,
            "direct membership reads and pushes, but cannot manage"
        );
        let read = f.grant(&f.owner, &bot, "read", 1800);
        assert_eq!(
            f.set(&bot, "main", Some(&read)).await.0,
            StatusCode::FORBIDDEN
        );
        // Direct admin authority does not need its owner's channel role.
        // Once removed, the same agent relies on capped delegation.
        let agent = Keys::generate();
        f.add(&agent).await;
        put(&agent, MemberRole::Admin).await;
        let tag =
            buzz_sdk::nip_oa::compute_auth_tag(&f.maintainer, &agent.public_key(), "").unwrap();
        assert_eq!(
            f.set(&agent, "main", Some(&tag)).await.0,
            StatusCode::OK,
            "direct admin authority needs no grant"
        );
        f.state
            .db
            .remove_member(
                community,
                f.channel,
                agent.public_key().as_bytes(),
                f.owner.public_key().as_bytes(),
            )
            .await
            .unwrap();
        let push = f.grant(&f.maintainer, &agent, "push", 1800);
        let manage = f.grant(&f.maintainer, &agent, "manage", 1800);
        assert_eq!(
            f.set(&agent, "main", Some(&manage)).await.0,
            StatusCode::FORBIDDEN,
            "its owner is only a member"
        );
        put(&f.maintainer, MemberRole::Owner).await;
        assert_eq!(
            f.set(&agent, "main", Some(&push)).await.0,
            StatusCode::FORBIDDEN,
            "a push grant does not manage"
        );
        assert_eq!(f.set(&agent, "main", Some(&manage)).await.0, StatusCode::OK);
        assert_eq!(
            f.set(&agent, "legacy", None).await.0,
            StatusCode::NOT_FOUND,
            "a grant lives in the request: without it, not even a read"
        );
        // Another owner's grant is refused at the door.
        let foreign = f.grant(&f.owner, &agent, "manage", 1800);
        assert_eq!(
            f.set(&agent, "legacy", Some(&foreign)).await,
            (
                StatusCode::FORBIDDEN,
                json!({"error": crate::api::relay_members::OWNER_MISMATCH})
            ),
        );
        // Owning the announcer grants nothing.
        assert!(f
            .state
            .db
            .set_agent_owner(
                community,
                f.owner.public_key().as_bytes(),
                f.member.public_key().as_bytes()
            )
            .await
            .unwrap());
        put(&f.member, MemberRole::Member).await;
        assert_eq!(
            f.set(&f.member, "legacy", None).await.0,
            StatusCode::FORBIDDEN
        );
        // Nor does announcing it: the announcer outside the channel cannot
        // even read.
        f.state
            .db
            .remove_member(
                community,
                f.channel,
                f.owner.public_key().as_bytes(),
                f.owner.public_key().as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(
            f.set(&f.owner, "legacy", None).await.0,
            StatusCode::NOT_FOUND
        );
        // A durable ban cascades through a presented owner credential even
        // when the signer holds the role itself.
        let ban_tag =
            buzz_sdk::nip_oa::compute_auth_tag(&f.member, &f.maintainer.public_key(), "").unwrap();
        f.state
            .db
            .ban_community_member(
                community,
                f.member.public_key().as_bytes(),
                f.member.public_key().as_bytes(),
                Some("test"),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            f.set(&f.maintainer, "legacy", Some(&ban_tag)).await.0,
            StatusCode::FORBIDDEN
        );
        sqlx::query("UPDATE channels SET archived_at = NOW() WHERE community_id = $1 AND id = $2")
            .bind(community.as_uuid())
            .bind(f.channel)
            .execute(&f.pool)
            .await
            .unwrap();
        assert_eq!(
            f.set(&f.maintainer, "legacy", None).await.0,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn default_branch_push_races_and_fresh_clone() {
        let f = Fixture::new().await;
        let a = f.snapshot().await;
        let b = f.snapshot().await;
        let old_digest = a.digest.clone();
        let (_, changed) = f
            .set_from(
                a,
                SetDefaultBranch {
                    branch: "main".into(),
                    expected_manifest: old_digest.clone(),
                },
            )
            .await
            .unwrap();
        assert!(changed);
        let loser = f
            .set_from(
                b,
                SetDefaultBranch {
                    branch: "legacy".into(),
                    expected_manifest: old_digest,
                },
            )
            .await
            .err()
            .unwrap();
        assert_eq!(
            loser.status(),
            StatusCode::CONFLICT,
            "stale no-op must CAS too"
        );
        // Snapshot a push before the metadata update; it must not restore stale HEAD.
        let options = || super::super::super::hydrate::HydrationOptions {
            pack_cache: &f.state.git_pack_cache,
            scratch_dir: f.scratch.path(),
            max_pack_bytes: 1024 * 1024,
            max_repo_bytes: 2 * 1024 * 1024,
        };
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &f.owner.public_key().to_hex(),
            &f.repo,
            options(),
        )
        .await
        .unwrap();
        assert_eq!(f.set(&f.owner, "legacy", None).await.0, StatusCode::OK);
        assert_eq!(
            f.land_push(&push, &parent).await,
            super::super::super::land::Verdict::Superseded
        );
        // Other direction: a push deletes the candidate after settings loaded it.
        let stale = f.snapshot().await;
        let digest = stale.digest.clone();
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &f.owner.public_key().to_hex(),
            &f.repo,
            options(),
        )
        .await
        .unwrap();
        git(push.path(), &["update-ref", "-d", "refs/heads/main"]).await;
        assert_eq!(
            f.land_push(&push, &parent).await,
            super::super::super::land::Verdict::Landed
        );
        assert_eq!(
            f.set_from(
                stale,
                SetDefaultBranch {
                    branch: "main".into(),
                    expected_manifest: digest
                }
            )
            .await
            .err()
            .unwrap()
            .status(),
            StatusCode::CONFLICT
        );
        assert!(!f
            .snapshot()
            .await
            .manifest
            .refs
            .contains_key("refs/heads/main"));
        // Restore main and add release/v1, then select the non-main branch so
        // Git's initial-branch default cannot mask a lost hydrated HEAD.
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &f.owner.public_key().to_hex(),
            &f.repo,
            options(),
        )
        .await
        .unwrap();
        let main = git(&f.scratch.path().join("source"), &["rev-parse", "main"]).await;
        git(push.path(), &["update-ref", "refs/heads/main", main.trim()]).await;
        git(
            push.path(),
            &["update-ref", "refs/heads/release/v1", main.trim()],
        )
        .await;
        assert_eq!(
            f.land_push(&push, &parent).await,
            super::super::super::land::Verdict::Landed
        );
        assert_eq!(f.set(&f.owner, "release/v1", None).await.0, StatusCode::OK);
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &f.owner.public_key().to_hex(),
            &f.repo,
            options(),
        )
        .await
        .unwrap();
        git(
            push.path(),
            &["update-ref", "refs/heads/later", main.trim()],
        )
        .await;
        assert_eq!(
            f.land_push(&push, &parent).await,
            super::super::super::land::Verdict::Landed
        );
        assert_eq!(f.snapshot().await.manifest.head, "refs/heads/release/v1");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Add a reachable host alias for the same tenant solely in this fixture.
        sqlx::query("UPDATE communities SET host = $1 WHERE id = $2")
            .bind(addr.to_string())
            .bind(f.tenant.community().as_uuid())
            .execute(&f.pool)
            .await
            .unwrap();
        let router = super::super::super::transport::git_router(f.state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let repo_url = format!(
            "http://{addr}/git/{}/{}",
            f.owner.public_key().to_hex(),
            f.repo
        );
        let auth = format!(
            "http.extraHeader=Authorization: {}",
            token(&f.owner, "GET", &repo_url, None)
        );
        let refs = git(
            f.scratch.path(),
            &["-c", &auth, "ls-remote", "--symref", &repo_url, "HEAD"],
        )
        .await;
        assert!(refs.contains("ref: refs/heads/release/v1\tHEAD"), "{refs}");
        git(
            f.scratch.path(),
            &["-c", &auth, "clone", &repo_url, "clone"],
        )
        .await;
        assert_eq!(
            git(&f.scratch.path().join("clone"), &["symbolic-ref", "HEAD"])
                .await
                .trim(),
            "refs/heads/release/v1"
        );
        assert_eq!(
            std::fs::read(f.scratch.path().join("clone/main.txt")).unwrap(),
            b"selected branch\n"
        );
        server.abort();
    }

    /// Seeds unrecorded-read and archive-drops-record: once governance is in
    /// force, every door that shows the repo records whom it showed it to,
    /// by door, before showing it; a refused read records nothing, and
    /// archiving the channel keeps the record. Ungoverned reads are not
    /// recorded.
    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn reads_of_a_governed_repo_are_recorded_by_door() {
        let f = Fixture::new().await;
        let stranger = Keys::generate();
        assert_eq!(f.call(&f.member, None, None).await.0, StatusCode::OK);
        assert!(
            f.reads(&f.member).await.is_empty(),
            "ungoverned reads are not recorded"
        );

        f.floor().await;
        assert_eq!(f.call(&f.member, None, None).await.0, StatusCode::OK);
        assert_eq!(f.reads(&f.member).await, ["state"]);
        assert_eq!(f.call(&stranger, None, None).await.0, StatusCode::NOT_FOUND);
        assert!(
            f.reads(&stranger).await.is_empty(),
            "a refused read records nothing"
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        sqlx::query("UPDATE communities SET host = $1 WHERE id = $2")
            .bind(addr.to_string())
            .bind(f.tenant.community().as_uuid())
            .execute(&f.pool)
            .await
            .unwrap();
        let router = super::super::super::transport::git_router(f.state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let repo_url = format!(
            "http://{addr}/git/{}/{}",
            f.owner.public_key().to_hex(),
            f.repo
        );
        let auth = format!(
            "http.extraHeader=Authorization: {}",
            token(&f.member, "GET", &repo_url, None)
        );
        git(
            f.scratch.path(),
            &["-c", &auth, "clone", &repo_url, "clone"],
        )
        .await;
        server.abort();
        assert_eq!(
            f.reads(&f.member).await,
            ["state", "fetch", "fetch"],
            "a clone is an advertisement and a pack, each recorded"
        );

        f.state
            .db
            .archive_channel(f.tenant.community(), f.channel)
            .await
            .unwrap();
        assert_eq!(f.reads(&f.member).await, ["state", "fetch", "fetch"]);
    }

    /// Seeds state-for-community, and evidence-for-community as far as the
    /// relay has an evidence door: the doors that show a repository's state
    /// are the repository's, not the community's. A community member who
    /// owns another of its channels is refused at each and leaves no row;
    /// the repository's channel member is served at the same doors. The
    /// relay has no evidence read: evidence shows only through the land and
    /// its receipts, so the door that takes it stands in, and keeps nothing
    /// from the neighbor.
    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn a_place_elsewhere_in_the_community_reaches_no_repository_door() {
        use buzz_core::governance::{Evidence, EvidenceBody};
        let f = Fixture::new().await;
        f.floor().await;
        let community = f.tenant.community();
        let neighbor = Keys::generate();
        let elsewhere = uuid::Uuid::new_v4();
        f.state
            .db
            .ensure_user(community, neighbor.public_key().as_bytes())
            .await
            .unwrap();
        f.state
            .db
            .create_channel_with_id(
                community,
                elsewhere,
                &format!("elsewhere-{elsewhere}"),
                buzz_db::channel::ChannelType::Stream,
                buzz_db::channel::ChannelVisibility::Open,
                None,
                neighbor.public_key().as_bytes(),
                None,
            )
            .await
            .unwrap();
        sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'member')")
            .bind(community.as_uuid())
            .bind(neighbor.public_key().to_hex())
            .execute(&f.pool)
            .await
            .unwrap();

        let owner = f.owner.public_key().to_hex();
        let repo = super::super::super::land::coordinate(&owner, &f.repo).to_string();
        let repo_url = format!("http://{}/git/{owner}/{}", f.tenant.host(), f.repo);
        for (door, refused) in [
            ("fetch", StatusCode::NOT_FOUND),
            ("default branch", StatusCode::NOT_FOUND),
            ("effective", StatusCode::FORBIDDEN),
            ("receipts", StatusCode::FORBIDDEN),
            ("rules", StatusCode::FORBIDDEN),
        ] {
            for (key, expected) in [(&f.member, StatusCode::OK), (&neighbor, refused)] {
                let status = match door {
                    "fetch" => {
                        let request = Request::builder()
                            .uri(format!(
                                "/git/{owner}/{}/info/refs?service=git-upload-pack",
                                f.repo
                            ))
                            .header("host", f.tenant.host())
                            .header("authorization", token(key, "GET", &repo_url, None))
                            .body(Body::empty())
                            .unwrap();
                        super::super::super::transport::git_router(f.state.clone())
                            .oneshot(request)
                            .await
                            .unwrap()
                            .status()
                    }
                    "default branch" => f.call(key, None, None).await.0,
                    "effective" => {
                        f.query(key, json!([{"git_effective": 1, "repo": repo}]))
                            .await
                    }
                    "receipts" => {
                        f.query(key, json!([{"git_receipts": 1, "repo": repo}]))
                            .await
                    }
                    _ => {
                        f.query(
                            key,
                            json!([{"governance_head": 1, "document": format!("rules:{repo}")}]),
                        )
                        .await
                    }
                };
                assert_eq!(status, expected, "{door}");
            }
        }

        for (key, takes) in [(&f.member, true), (&neighbor, false)] {
            let approval = Evidence {
                community: *community.as_uuid(),
                repo: repo.clone(),
                git_ref: "refs/heads/main".into(),
                head: "a".repeat(40),
                base: "b".repeat(40),
                body: EvidenceBody::Approval { approved: true },
            }
            .builder()
            .unwrap()
            .sign_with_keys(key)
            .unwrap();
            let auth = crate::handlers::ingest::IngestAuth::Http {
                pubkey: key.public_key(),
                scopes: vec![buzz_auth::Scope::ReposWrite],
                auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
            };
            let taken =
                crate::handlers::ingest::ingest_event(&f.state, &f.tenant, approval, auth).await;
            match taken {
                Ok(_) => assert!(takes, "the neighbor's evidence was taken"),
                Err(crate::handlers::ingest::IngestError::AuthFailed(reason)) if !takes => {
                    assert!(reason.contains("governance authority required"), "{reason}")
                }
                Err(e) => panic!("evidence: {e:?}"),
            }
            let kept: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM audit_log WHERE community_id=$1 \
                 AND action='git_evidence' AND actor_pubkey=$2",
            )
            .bind(community.as_uuid())
            .bind(key.public_key().to_bytes().to_vec())
            .fetch_one(&f.pool)
            .await
            .unwrap();
            assert_eq!(kept, i64::from(takes), "evidence kept");
        }

        assert_eq!(
            f.reads(&f.member).await,
            ["fetch", "state", "state", "state", "state"]
        );
        assert!(
            f.reads(&neighbor).await.is_empty(),
            "a refusal records nothing"
        );
    }

    /// A read found ungoverned serves only what it picked before that
    /// decision. Each door is parked after it, while a floor goes up and a
    /// governed push lands a new commit on a new branch; resumed, it shows
    /// neither, and records nothing.
    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn a_read_found_ungoverned_serves_what_it_picked_before() {
        fn pkt(line: &str) -> String {
            format!("{:04x}{line}", line.len() + 4)
        }
        for door in [
            "advertisement",
            "tagged advertisement",
            "receive-pack advertisement",
            "upload-pack",
            "default branch",
        ] {
            let f = Fixture::new().await;
            let owner = f.owner.public_key().to_hex();
            let hydrate = |dir: &'static str| {
                let f = &f;
                let owner = owner.clone();
                async move {
                    let scratch = f.scratch.path().join(dir);
                    std::fs::create_dir(&scratch).unwrap();
                    super::super::super::hydrate::hydrate_for_write(
                        &f.state.git_store,
                        &f.tenant,
                        &owner,
                        &f.repo,
                        super::super::super::hydrate::HydrationOptions {
                            pack_cache: &f.state.git_pack_cache,
                            scratch_dir: &scratch,
                            max_pack_bytes: 1024 * 1024,
                            max_repo_bytes: 2 * 1024 * 1024,
                        },
                    )
                    .await
                    .unwrap()
                }
            };
            let _ = f.snapshot().await;
            if door == "tagged advertisement" {
                let (push, parent) = hydrate("tag").await;
                git(
                    push.path(),
                    &["update-ref", "refs/tags/v0", "refs/heads/main"],
                )
                .await;
                assert_eq!(
                    f.land_push(&push, &parent).await,
                    super::super::super::land::Verdict::Landed
                );
            }
            let before = f.snapshot().await.digest;
            let repo_url = format!("http://{}/git/{owner}/{}", f.tenant.host(), f.repo);
            let fetch_token = token(&f.member, "GET", &repo_url, None);
            let (method, path, body) = match door {
                "advertisement" | "tagged advertisement" => (
                    "GET",
                    "info/refs?service=git-upload-pack".to_string(),
                    String::new(),
                ),
                "receive-pack advertisement" => (
                    "GET",
                    "info/refs?service=git-receive-pack".to_string(),
                    String::new(),
                ),
                "upload-pack" => ("POST", "git-upload-pack".to_string(), String::new()),
                _ => ("GET", "default-branch".to_string(), String::new()),
            };
            // The governed commit, made now so upload-pack can want it.
            let (push, parent) = hydrate("governed").await;
            let commit = git(
                push.path(),
                &[
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@example.invalid",
                    "commit-tree",
                    "refs/heads/main^{tree}",
                    "-p",
                    "refs/heads/main",
                    "-m",
                    "governed",
                ],
            )
            .await;
            let commit = commit.trim().to_string();
            let main = git(push.path(), &["rev-parse", "refs/heads/main"]).await;
            let main = main.trim().to_string();
            let body = if door == "upload-pack" {
                format!("{}0000{}", pkt(&format!("want {commit}\n")), pkt("done\n"))
            } else {
                body
            };
            let request = Request::builder()
                .method(method)
                .uri(format!("/git/{owner}/{}/{path}", f.repo))
                .header("host", f.tenant.host())
                .header(
                    "authorization",
                    if door == "default branch" {
                        token(
                            &f.member,
                            "GET",
                            &format!("http://{}{}", f.tenant.host(), f.path()),
                            None,
                        )
                    } else {
                        fetch_token.clone()
                    },
                )
                .body(Body::from(body))
                .unwrap();
            let (arrived, release) =
                crate::nip_fi_test_hooks::git_read_decided_hook::arm(f.tenant.community());
            let router = super::super::super::transport::git_router(f.state.clone());
            let read = tokio::spawn(async move { router.oneshot(request).await.unwrap() });
            tokio::time::timeout(std::time::Duration::from_secs(30), arrived)
                .await
                .expect("the read decided ungoverned in time")
                .unwrap();
            f.floor().await;
            git(
                push.path(),
                &["update-ref", "refs/heads/governed-only", &commit],
            )
            .await;
            assert_eq!(
                f.land_push(&push, &parent).await,
                super::super::super::land::Verdict::Landed,
                "{door}"
            );
            assert_ne!(
                f.snapshot().await.digest,
                before,
                "{door}: the governed push landed"
            );
            release.notify_one();
            let response = read.await.unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
                .await
                .unwrap();
            let shown = String::from_utf8_lossy(&bytes);
            if door.ends_with("advertisement") {
                assert_eq!(status, StatusCode::OK, "{door}: {shown}");
                assert!(
                    shown.contains(&format!("{main} refs/heads/main")),
                    "{door} shows what it picked: {shown}"
                );
            }
            assert!(
                !shown.contains("governed-only"),
                "{door} showed the governed ref: {shown}"
            );
            if door == "upload-pack" {
                assert!(
                    shown.contains(&format!("not our ref {commit}")),
                    "{door} refuses what it did not pick: {shown}"
                );
            } else {
                assert!(
                    !shown.contains(&commit),
                    "{door} showed the governed commit: {shown}"
                );
            }
            assert!(
                !bytes.windows(4).any(|w| w == b"PACK"),
                "{door} served the governed commit"
            );
            if door == "default branch" {
                assert!(
                    shown.contains(&before),
                    "{door} shows what it picked: {shown}"
                );
            }
            assert!(
                f.reads(&f.member).await.is_empty(),
                "{door}: no row for an ungoverned read"
            );
        }
    }

    /// Membership works independently; without membership the owner grant
    /// must cover the repository, action and current floor lifetime.
    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn an_agent_pushes_by_membership_or_a_live_grant() {
        use super::super::super::land::Verdict;
        let f = Fixture::new().await;
        let agent = Keys::generate();
        f.add(&agent).await;
        f.state
            .db
            .add_member(
                f.tenant.community(),
                f.channel,
                agent.public_key().as_bytes(),
                MemberRole::Member,
                Some(f.owner.public_key().as_bytes()),
            )
            .await
            .unwrap();
        assert!(f
            .state
            .db
            .set_agent_owner(
                f.tenant.community(),
                agent.public_key().as_bytes(),
                f.owner.public_key().as_bytes()
            )
            .await
            .unwrap());
        f.floor().await;
        // Reconcile the legacy fixture manifest before a governed land.
        let _ = f.snapshot().await;
        let push = |branch: &'static str, grant: Option<buzz_core::governance::GitGrant>| {
            let f = &f;
            let agent = agent.clone();
            async move {
                let (push, parent) = super::super::super::hydrate::hydrate_for_write(
                    &f.state.git_store,
                    &f.tenant,
                    &f.owner.public_key().to_hex(),
                    &f.repo,
                    super::super::super::hydrate::HydrationOptions {
                        pack_cache: &f.state.git_pack_cache,
                        scratch_dir: f.scratch.path(),
                        max_pack_bytes: 1024 * 1024,
                        max_repo_bytes: 2 * 1024 * 1024,
                    },
                )
                .await
                .unwrap();
                git(push.path(), &["update-ref", branch, "refs/heads/main"]).await;
                f.land_push_as(&agent, grant.as_ref(), &push, &parent).await
            }
        };
        let refused = |v: &Verdict| matches!(v, Verdict::Refused(_));
        let now = chrono::Utc::now().timestamp() as u64;
        let repo = super::super::super::land::coordinate(&f.owner.public_key().to_hex(), &f.repo);
        let grant =
            |level, after: Option<u64>, before: Option<u64>| buzz_core::governance::GitGrant {
                owner: f.owner.public_key(),
                repo: Some(repo.clone()),
                level,
                after,
                before,
            };
        use buzz_core::governance::GrantAction::{Manage, Push, Read};
        let live = grant(Push, Some(now - 60), Some(now + 1800));

        assert_eq!(push("refs/heads/member", None).await, Verdict::Landed);
        assert_eq!(
            push(
                "refs/heads/member-expired",
                Some(grant(Push, Some(now - 120), Some(now - 60)))
            )
            .await,
            Verdict::Landed
        );
        f.state
            .db
            .remove_member(
                f.tenant.community(),
                f.channel,
                agent.public_key().as_bytes(),
                f.owner.public_key().as_bytes(),
            )
            .await
            .unwrap();
        let v = push("refs/heads/a", None).await;
        assert!(refused(&v), "no membership or grant: {v:?}");
        let v = push(
            "refs/heads/b",
            Some(grant(Read, Some(now - 60), Some(now + 1800))),
        )
        .await;
        assert!(refused(&v), "a read grant does not push: {v:?}");
        let other = buzz_core::governance::GitGrant {
            owner: Keys::generate().public_key(),
            ..live.clone()
        };
        let v = push("refs/heads/b", Some(other)).await;
        assert!(refused(&v), "only the recorded owner's grant counts: {v:?}");
        let elsewhere = buzz_core::governance::GitGrant {
            repo: Some(super::super::super::land::coordinate(
                &f.owner.public_key().to_hex(),
                "other",
            )),
            ..live.clone()
        };
        let v = push("refs/heads/b", Some(elsewhere)).await;
        assert!(refused(&v), "another repository's grant: {v:?}");
        let v = push(
            "refs/heads/b",
            Some(grant(Push, Some(now - 120), Some(now - 60))),
        )
        .await;
        assert!(refused(&v), "lapsed: {v:?}");
        let v = push(
            "refs/heads/b",
            Some(grant(Push, Some(now + 60), Some(now + 120))),
        )
        .await;
        assert!(refused(&v), "not yet valid: {v:?}");
        // The floor caps the window it may sign: both bounds, at most 3600s.
        let v = push("refs/heads/b", Some(grant(Push, None, Some(now + 60)))).await;
        assert!(refused(&v), "no lower bound under a cap: {v:?}");
        let v = push("refs/heads/b", Some(grant(Push, None, None))).await;
        assert!(refused(&v), "no bounds under a cap: {v:?}");
        let v = push(
            "refs/heads/b",
            Some(grant(Push, Some(now - 60), Some(now + 3541))),
        )
        .await;
        assert!(refused(&v), "one second over the cap: {v:?}");
        let v = push(
            "refs/heads/b",
            Some(grant(Push, Some(now - 60), Some(now + 3540))),
        )
        .await;
        assert_eq!(v, Verdict::Landed, "exactly the cap");
        assert_eq!(
            push("refs/heads/c", Some(live.clone())).await,
            Verdict::Landed
        );
        assert_eq!(
            push(
                "refs/heads/d",
                Some(grant(Manage, Some(now - 60), Some(now + 60)))
            )
            .await,
            Verdict::Landed,
            "manage includes push"
        );
        let all = buzz_core::governance::GitGrant {
            repo: None,
            ..live.clone()
        };
        assert_eq!(
            push("refs/heads/e", Some(all)).await,
            Verdict::Landed,
            "repo=*"
        );

        // A grant live at decision time must still be live at the swap.
        let before = f
            .state
            .db
            .git_repo_manifest(f.tenant.community(), &repo)
            .await
            .unwrap();
        let serving = buzz_deletion::acquire_serving_write(
            &f.state.db,
            f.tenant.community(),
            "expiry-review",
        )
        .await
        .unwrap();
        let ending = grant(
            Push,
            Some(now - 60),
            Some(chrono::Utc::now().timestamp() as u64 + 2),
        );
        let verdict = super::super::super::land::land(
            &f.state,
            serving.lease(),
            super::super::super::land::Change {
                kind: "push",
                repo: &repo,
                actor: agent.public_key().as_bytes(),
                grant: Some(&ending),
                seen: before.as_deref(),
                new: "expired-candidate",
                updates: &[],
            },
            async |_tx, authority| {
                assert!(
                    authority.valid_until.is_some(),
                    "live grant carries expiry into swap"
                );
                let permitted = authority
                    .role
                    .is_some_and(|r| r.has_at_least(MemberRole::Member));
                if permitted {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
                Ok(super::super::super::gate::Decision {
                    verdict: if permitted {
                        Ok(())
                    } else {
                        Err(vec![buzz_core::git_perms::Denial {
                            ref_name: "HEAD".into(),
                            reason: "membership required".into(),
                        }])
                    },
                    counted: Default::default(),
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(
            verdict,
            Verdict::Superseded,
            "expiry after decision must prevent swap"
        );
        assert_eq!(
            f.state
                .db
                .git_repo_manifest(f.tenant.community(), &repo)
                .await
                .unwrap(),
            before
        );
        serving.finish().await.unwrap();

        // A manage grant raises member authority, but this push only needs
        // member. Let the grant expire after deciding: the direct route wins.
        f.state
            .db
            .add_member(
                f.tenant.community(),
                f.channel,
                agent.public_key().as_bytes(),
                MemberRole::Member,
                Some(f.owner.public_key().as_bytes()),
            )
            .await
            .unwrap();
        let serving = buzz_deletion::acquire_serving_write(
            &f.state.db,
            f.tenant.community(),
            "member-expiry",
        )
        .await
        .unwrap();
        let ending = grant(
            Manage,
            Some(now - 60),
            Some(chrono::Utc::now().timestamp() as u64 + 2),
        );
        let verdict = super::super::super::land::land(
            &f.state,
            serving.lease(),
            super::super::super::land::Change {
                kind: "push",
                repo: &repo,
                actor: agent.public_key().as_bytes(),
                grant: Some(&ending),
                seen: before.as_deref(),
                new: before.as_deref().unwrap(),
                updates: &[],
            },
            async |_tx, authority| {
                assert_eq!(authority.role, Some(MemberRole::Member));
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                Ok(super::super::super::gate::Decision {
                    verdict: Ok(()),
                    counted: Default::default(),
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(
            verdict,
            Verdict::Landed,
            "independent membership survives grant expiry"
        );
        serving.finish().await.unwrap();
    }

    /// Seeds scan-tip-only, scan-governed-only and evidence-shows-quarantine,
    /// through the land: a secret in a commit below the tip of an ungoverned
    /// branch refuses the push, moves nothing, and upload-pack serves
    /// nothing it brought; the same history without it lands and is served.
    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn a_secret_anywhere_in_a_push_refuses_it() {
        use super::super::super::land::Verdict;
        let f = Fixture::new().await;
        let _ = f.snapshot().await;
        // Built at run time so no credential-shaped literal is in the source.
        let secret = ["AK", "IA", &"Q".repeat(16)].concat();
        let clean = "key = from the environment\n";
        for (branch, body, replaced, lands) in [
            (
                "refs/heads/scratch",
                format!("key = {secret}\n"),
                false,
                false,
            ),
            // A replace ref must not show the scan other bytes than the
            // ones the repository serves.
            (
                "refs/heads/masked",
                format!("key = {secret}\n"),
                true,
                false,
            ),
            ("refs/heads/clean", clean.into(), false, true),
        ] {
            let (push, parent) = super::super::super::hydrate::hydrate_for_write(
                &f.state.git_store,
                &f.tenant,
                &f.owner.public_key().to_hex(),
                &f.repo,
                super::super::super::hydrate::HydrationOptions {
                    pack_cache: &f.state.git_pack_cache,
                    scratch_dir: f.scratch.path(),
                    max_pack_bytes: 1024 * 1024,
                    max_repo_bytes: 2 * 1024 * 1024,
                },
            )
            .await
            .unwrap();
            let path = push.path();
            let file = path.join("blob");
            std::fs::write(&file, body).unwrap();
            let blob = git(path, &["hash-object", "-w", file.to_str().unwrap()]).await;
            std::fs::remove_file(&file).unwrap();
            let commit = |tree: String, parent: String, message: &'static str| async move {
                let id = [
                    "-c",
                    "user.name=scan",
                    "-c",
                    "user.email=scan@example.invalid",
                ];
                let mut args = id.to_vec();
                args.extend(["commit-tree", &tree, "-p", &parent, "-m", message]);
                git(path, &args).await.trim().to_string()
            };
            let main = git(path, &["rev-parse", "refs/heads/main"]).await;
            let main = main.trim().to_string();
            let mut tree = tokio::process::Command::new("git");
            tree.current_dir(path).arg("mktree");
            super::super::super::transport::harden_git_env(&mut tree);
            let mut tree = tree
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            tokio::io::AsyncWriteExt::write_all(
                tree.stdin.as_mut().unwrap(),
                format!("100644 blob {}\tconfig.txt\n", blob.trim()).as_bytes(),
            )
            .await
            .unwrap();
            drop(tree.stdin.take());
            let tree = String::from_utf8(tree.wait_with_output().await.unwrap().stdout).unwrap();
            let below = commit(tree.trim().to_string(), main, "add config").await;
            // The tip takes the file away again: only the commit below has it.
            let tip = commit(
                git(path, &["rev-parse", "refs/heads/main^{tree}"])
                    .await
                    .trim()
                    .to_string(),
                below,
                "remove config",
            )
            .await;
            git(path, &["update-ref", branch, &tip]).await;
            let mut refused = vec![branch.to_string()];
            if replaced {
                std::fs::write(&file, clean).unwrap();
                let mask = git(path, &["hash-object", "-w", file.to_str().unwrap()]).await;
                std::fs::remove_file(&file).unwrap();
                let replace = format!("refs/replace/{}", blob.trim());
                git(path, &["update-ref", &replace, mask.trim()]).await;
                refused.push(replace);
            }
            let before = f.snapshot().await.digest;
            let verdict = f.land_push(&push, &parent).await;
            if lands {
                assert_eq!(verdict, Verdict::Landed, "{branch}");
            } else {
                let denials = refused
                    .into_iter()
                    .map(|ref_name| buzz_core::git_perms::Denial {
                        ref_name,
                        reason: super::super::super::gate::SECRET_DETECTED.into(),
                    });
                assert_eq!(verdict, Verdict::Refused(denials.collect()), "{branch}");
            }
            // Seed evidence-shows-quarantine, at upload-pack: a fetch
            // wanting a refused push's tip is refused; the same fetch of a
            // landed tip is served its pack.
            f.assert_served(f.tenant.host(), &tip, lands, branch).await;
            if !lands {
                assert_eq!(f.snapshot().await.digest, before, "a refusal moves nothing");
            }
        }
    }

    /// As above, through receive-pack itself: `git push` of a branch whose
    /// secret sits below its tip is refused with the reason, moves nothing,
    /// and upload-pack serves nothing it brought; a clean push lands and is
    /// served.
    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn receive_pack_refuses_a_secret_and_serves_nothing_it_brought() {
        let f = Fixture::new().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        sqlx::query("UPDATE communities SET host = $1 WHERE id = $2")
            .bind(addr.to_string())
            .bind(f.tenant.community().as_uuid())
            .execute(&f.pool)
            .await
            .unwrap();
        // The pre-receive hook calls the policy door on the relay's own address.
        let mut state = (*f.state).clone();
        Arc::make_mut(&mut state.config).bind_addr = addr;
        let state = Arc::new(state);
        let router = super::super::super::transport::git_router(state.clone())
            .merge(super::super::super::git_policy_router(state))
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let repo_url = format!(
            "http://{addr}/git/{}/{}",
            f.owner.public_key().to_hex(),
            f.repo
        );
        let auth = format!(
            "http.extraHeader=Authorization: {}",
            token(&f.owner, "GET", &repo_url, None)
        );
        git(
            f.scratch.path(),
            &["-c", &auth, "clone", &repo_url, "clone"],
        )
        .await;
        let clone = f.scratch.path().join("clone");
        let id = [
            "-c",
            "user.name=scan",
            "-c",
            "user.email=scan@example.invalid",
        ];
        // Built at run time so no credential-shaped literal is in the source.
        let secret = ["AK", "IA", &"Q".repeat(16)].concat();
        for (branch, body, lands) in [
            ("scratch", format!("key = {secret}\n"), false),
            ("clean", "key = from the environment\n".to_string(), true),
        ] {
            git(&clone, &["checkout", "-q", "-B", branch, "origin/main"]).await;
            std::fs::write(clone.join("config.txt"), body).unwrap();
            git(&clone, &["add", "config.txt"]).await;
            git(
                &clone,
                &[&id[..], &["commit", "-qm", "add config"]].concat(),
            )
            .await;
            git(&clone, &["rm", "-q", "config.txt"]).await;
            git(
                &clone,
                &[&id[..], &["commit", "-qm", "remove config"]].concat(),
            )
            .await;
            let tip = git(&clone, &["rev-parse", "HEAD"]).await.trim().to_string();
            let before = f.snapshot().await.digest;
            let mut push = tokio::process::Command::new("git");
            push.current_dir(&clone)
                .args(["-c", &auth, "push", "--porcelain", "origin", branch]);
            super::super::super::transport::harden_git_env(&mut push);
            let pushed = push.output().await.unwrap();
            let said = String::from_utf8_lossy(&pushed.stdout);
            assert_eq!(pushed.status.success(), lands, "{branch}: {said}");
            if !lands {
                assert!(
                    said.contains(super::super::super::gate::SECRET_DETECTED),
                    "{branch}: {said}"
                );
                assert_eq!(f.snapshot().await.digest, before, "a refusal moves nothing");
            }
            f.assert_served(&addr.to_string(), &tip, lands, branch)
                .await;
        }
        server.abort();
    }

    /// A name announced as `<name>.git` is not a second door to `<name>`:
    /// a member refused on `r` is refused through the URL `r.git.git`, which
    /// moves nothing, so the repository's own writers still land. Ungoverned
    /// (the alias would be adopted) and governed (it would fork the cache).
    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn a_dot_git_alias_reaches_no_other_repository() {
        let mut failures = Vec::new();
        for governed in [false, true] {
            let arm = if governed { "governed" } else { "ungoverned" };
            let f = Fixture::new().await;
            let owner = f.owner.public_key().to_hex();
            let announce = |name: &str, protect: bool| {
                let mut tags = vec![
                    Tag::parse(["d", name]).unwrap(),
                    Tag::parse(["buzz-channel", &f.channel.to_string()]).unwrap(),
                ];
                if protect {
                    tags.push(
                        Tag::parse(["buzz-protect", "refs/heads/main", "push:admin"]).unwrap(),
                    );
                }
                EventBuilder::new(Kind::Custom(30617), "")
                    .tags(tags)
                    .custom_created_at(nostr::Timestamp::from(
                        nostr::Timestamp::now().as_secs() + 5,
                    ))
                    .sign_with_keys(&f.owner)
                    .unwrap()
            };
            // `r` needs an admin for main; `r.git`, stored as before this
            // relay refused such names, has no rules.
            let alias = format!("{}.git", f.repo);
            for event in [announce(&f.repo, true), announce(&alias, false)] {
                f.state
                    .db
                    .insert_event(f.tenant.community(), &event, None)
                    .await
                    .unwrap();
            }
            f.state
                .db
                .reserve_repo_name(f.tenant.community(), &alias, &owner)
                .await
                .unwrap();
            if governed {
                let repo = super::super::super::land::coordinate(&owner, &f.repo);
                f.govern(
                    &f.owner,
                    &format!("rules:{repo}"),
                    &buzz_core::governance::ExpectedHead::Absent,
                    r#"{"version":1,"body":{"type":"rules","rules":[]}}"#,
                )
                .await;
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            sqlx::query("UPDATE communities SET host = $1 WHERE id = $2")
                .bind(addr.to_string())
                .bind(f.tenant.community().as_uuid())
                .execute(&f.pool)
                .await
                .unwrap();
            let mut state = (*f.state).clone();
            Arc::make_mut(&mut state.config).bind_addr = addr;
            let state = Arc::new(state);
            let router = super::super::super::transport::git_router(state.clone())
                .merge(super::super::super::git_policy_router(state))
                .into_make_service_with_connect_info::<std::net::SocketAddr>();
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let url = |name: &str| format!("http://{addr}/git/{owner}/{name}");
            let auth = |key: &Keys, url: &str| {
                format!(
                    "http.extraHeader=Authorization: {}",
                    token(key, "GET", url, None)
                )
            };
            // The owner uses the URL's usual `.git` form: the same repository.
            let repo_url = url(&alias);
            git(
                f.scratch.path(),
                &[
                    "-c",
                    &auth(&f.owner, &repo_url),
                    "clone",
                    &repo_url,
                    "clone",
                ],
            )
            .await;
            let clone = f.scratch.path().join("clone");
            git(&clone, &["checkout", "-q", "main"]).await;
            std::fs::write(clone.join("member.txt"), b"member\n").unwrap();
            git(&clone, &["add", "member.txt"]).await;
            git(
                &clone,
                &[
                    "-c",
                    "user.name=m",
                    "-c",
                    "user.email=m@example.invalid",
                    "commit",
                    "-qm",
                    "member",
                ],
            )
            .await;
            let pointer =
                super::super::super::manifest::pointer_key(f.tenant.community(), &owner, &f.repo);
            let cache = || async {
                f.state
                    .git_store
                    .get_pointer(&pointer)
                    .await
                    .unwrap()
                    .unwrap()
                    .1
            };
            let push = |key: &Keys, url: String| {
                let auth = auth(key, &url);
                let clone = clone.clone();
                async move {
                    let mut push = tokio::process::Command::new("git");
                    push.current_dir(&clone).args([
                        "-c",
                        &auth,
                        "push",
                        "--porcelain",
                        &url,
                        "main",
                    ]);
                    super::super::super::transport::harden_git_env(&mut push);
                    push.output().await.unwrap()
                }
            };
            let before = cache().await;
            // Control: the member may not move `r`'s main.
            let control = push(&f.maintainer, repo_url.clone()).await;
            if control.status.success() {
                failures.push(format!("{arm}: control: the member moved r's main"));
            }
            // The alias: the same push, through `r.git.git`.
            let attack = push(&f.maintainer, url(&format!("{alias}.git"))).await;
            if attack.status.success() {
                failures.push(format!(
                    "{arm}: the member's push through {alias}.git was accepted: {}",
                    String::from_utf8_lossy(&attack.stdout)
                ));
            }
            if cache().await != before {
                failures.push(format!("{arm}: the alias moved r's object-store pointer"));
            }
            // `r`'s own writer still lands, on `r`.
            let landed = push(&f.owner, repo_url.clone()).await;
            if !landed.status.success() {
                failures.push(format!(
                    "{arm}: the owner's push to r was refused: {}",
                    String::from_utf8_lossy(&landed.stderr)
                ));
            } else if cache().await == before {
                failures.push(format!("{arm}: the owner's push did not move r"));
            }
            server.abort();
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The land, end to end against Postgres and MinIO: a refused push
    /// commits only its receipt, the object-store cache one land behind is
    /// advanced, a pointer written outside the land stops writes, and a
    /// re-announcement cannot move the repository to another channel.
    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn the_land_decides_records_and_owns_the_commit_point() {
        use super::super::super::land::{committed_for_write, coordinate, Verdict};
        use super::super::super::manifest::pointer_key;
        use super::super::super::store::{CasOutcome, Precond};
        let f = Fixture::new().await;
        let options = || super::super::super::hydrate::HydrationOptions {
            pack_cache: &f.state.git_pack_cache,
            scratch_dir: f.scratch.path(),
            max_pack_bytes: 1024 * 1024,
            max_repo_bytes: 2 * 1024 * 1024,
        };
        let owner = f.owner.public_key().to_hex();
        let repo = coordinate(&owner, &f.repo);
        let community = f.tenant.community();
        let object = repo.to_string();
        let receipts = |action: &'static str| {
            let (pool, object) = (f.pool.clone(), object.clone());
            async move {
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM audit_log WHERE community_id = $1 AND action = $2 AND object_id = $3",
                )
                .bind(community.as_uuid())
                .bind(action)
                .bind(object)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };

        // A stranger's push is refused at the land; only its receipt commits.
        let before = f.snapshot().await.digest;
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &owner,
            &f.repo,
            options(),
        )
        .await
        .unwrap();
        git(
            push.path(),
            &["update-ref", "refs/heads/stranger", "refs/heads/main"],
        )
        .await;
        let stranger = Keys::generate();
        assert!(matches!(
            f.land_push_as(&stranger, None, &push, &parent).await,
            Verdict::Refused(_)
        ));
        assert_eq!(f.snapshot().await.digest, before, "a refusal moves nothing");
        assert_eq!(receipts("git_refused").await, 1);
        assert_eq!(receipts("git_landed").await, 0);

        // A stale precondition is a land refusal: decided after the role and
        // receipted, so a member learns only that it may not, and nothing moves.
        let stale = json!({"branch": "main", "expected_manifest": "0".repeat(64)});
        let (status, body) = f.call(&f.maintainer, Some(stale.clone()), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(
            f.call(&f.owner, Some(stale), None).await.0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            f.snapshot().await.digest,
            before,
            "a stale request moves nothing"
        );
        assert_eq!(receipts("git_refused").await, 3);
        assert_eq!(receipts("git_landed").await, 0);
        let stages: Vec<String> = sqlx::query_scalar(
            "SELECT detail->>'stage' FROM audit_log WHERE community_id = $1 AND object_id = $2",
        )
        .bind(community.as_uuid())
        .bind(&object)
        .fetch_all(&f.pool)
        .await
        .unwrap();
        assert_eq!(stages, ["land"; 3], "every land receipt names its stage");

        // A land whose cache write was lost leaves the cache one land behind;
        // the next writer advances it rather than refusing.
        let a = f.snapshot().await;
        assert_eq!(f.set(&f.owner, "main", None).await.0, StatusCode::OK);
        assert_eq!(receipts("git_landed").await, 1);
        let b = f.snapshot().await;
        let pointer = pointer_key(f.tenant.community(), &owner, &f.repo);
        let behind = f
            .state
            .git_store
            .put_pointer(
                &pointer,
                a.digest.as_bytes(),
                Precond::IfMatch(b.etag.clone()),
            )
            .await
            .unwrap();
        assert!(matches!(behind, CasOutcome::Won(_)));
        let (_, digest, _) = committed_for_write(&f.state, &f.tenant, &repo)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(digest, b.digest, "the row is the commit point");
        let (_, cached) = f
            .state
            .git_store
            .get_pointer(&pointer)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&cached[..], b.digest.as_bytes(), "the cache caught up");

        // Re-announcing the repository in another channel moves no authority.
        let moved = EventBuilder::new(Kind::Custom(30617), "")
            .tags([
                Tag::parse(["d", &f.repo]).unwrap(),
                Tag::parse(["buzz-channel", &uuid::Uuid::new_v4().to_string()]).unwrap(),
            ])
            .custom_created_at(nostr::Timestamp::from(
                nostr::Timestamp::now().as_secs() + 5,
            ))
            .sign_with_keys(&f.owner)
            .unwrap();
        f.state
            .db
            .insert_event(f.tenant.community(), &moved, None)
            .await
            .unwrap();
        // The land judges by the pinned channel, so the owner, an admin
        // there, still lands; the tag it just announced moved nothing.
        let stale = f.snapshot().await;
        let expected_manifest = stale.digest.clone();
        f.set_from(
            stale,
            SetDefaultBranch {
                branch: "legacy".into(),
                expected_manifest,
            },
        )
        .await
        .unwrap();
        let b = f.snapshot().await;
        assert_eq!(receipts("git_landed").await, 2);

        // A pointer written outside the land stops every write.
        let mut foreign = b.manifest.clone();
        foreign.parent = None;
        foreign.published_at = Some(b.manifest.published_at.unwrap_or(0) + 1);
        let key = f
            .state
            .git_store
            .put_manifest(&foreign.canonical_bytes().unwrap())
            .await
            .unwrap();
        let (etag, _) = f
            .state
            .git_store
            .get_pointer(&pointer)
            .await
            .unwrap()
            .unwrap();
        f.state
            .git_store
            .put_pointer(
                &pointer,
                key.strip_prefix("manifests/").unwrap().as_bytes(),
                Precond::IfMatch(etag),
            )
            .await
            .unwrap();
        let refused = committed_for_write(&f.state, &f.tenant, &repo)
            .await
            .err()
            .unwrap();
        assert_eq!(refused.status(), StatusCode::CONFLICT);
    }

    #[derive(Default)]
    struct VecLandTracer(std::sync::Mutex<Vec<buzz_conformance::git_land::LandStep>>);

    impl buzz_conformance::git_land::LandTracer for VecLandTracer {
        fn record(&self, step: buzz_conformance::git_land::LandStep) {
            self.0.lock().unwrap().push(step);
        }
    }

    impl VecLandTracer {
        fn take(&self) -> Vec<buzz_conformance::git_land::LandStep> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    /// The governed land, end to end on real relay traces: a rule written
    /// through governance ingress refuses an unapproved push (receipted,
    /// nothing moves), an approval signed through evidence ingress lands it,
    /// and each recorded trace passes `check_land_trace` while the same trace
    /// with one fact broken fails it.
    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn governed_land_traces_check_against_the_land_model() {
        use super::super::super::land::{coordinate, Verdict};
        use buzz_conformance::git_land::{check_land_trace, LandAction};
        use buzz_core::governance::{Evidence, EvidenceBody};
        let mut f = Fixture::new().await;
        let tracer = Arc::new(VecLandTracer::default());
        let mut state = (*f.state).clone();
        state.land_tracer = tracer.clone();
        f.state = Arc::new(state);
        let owner = f.owner.public_key().to_hex();
        let repo = coordinate(&owner, &f.repo);
        let auth = |keys: &Keys| crate::handlers::ingest::IngestAuth::Http {
            pubkey: keys.public_key(),
            scopes: vec![buzz_auth::Scope::ReposWrite],
            auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
        };
        let rules = r#"{"version":1,"body":{"type":"rules","rules":[{"id":"review","refs":["refs/heads/main"],"labels":{},"require":[{"type":"review","count":1,"group":null}]}]}}"#;
        let command = buzz_core::governance::command_builder(
            *f.tenant.community().as_uuid(),
            &format!("rules:{repo}").parse().unwrap(),
            &buzz_core::governance::ExpectedHead::Absent,
            rules,
        )
        .unwrap()
        .sign_with_keys(&f.owner)
        .unwrap();
        crate::handlers::ingest::ingest_event(&f.state, &f.tenant, command, auth(&f.owner))
            .await
            .unwrap();

        let options = super::super::super::hydrate::HydrationOptions {
            pack_cache: &f.state.git_pack_cache,
            scratch_dir: f.scratch.path(),
            max_pack_bytes: 1024 * 1024,
            max_repo_bytes: 2 * 1024 * 1024,
        };
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &owner,
            &f.repo,
            options,
        )
        .await
        .unwrap();
        let base = git(push.path(), &["rev-parse", "refs/heads/main"]).await;
        let base = base.trim().to_string();
        let tip = git(
            push.path(),
            &[
                "-c",
                "user.name=land",
                "-c",
                "user.email=land@example.invalid",
                "commit-tree",
                "refs/heads/main^{tree}",
                "-p",
                &base,
                "-m",
                "governed",
            ],
        )
        .await;
        let tip = tip.trim().to_string();
        git(push.path(), &["update-ref", "refs/heads/main", &tip]).await;
        let required = ["decide", "receipt"];

        // Seed no-receipt-on-refusal: unapproved, refused, receipted.
        let before = f.snapshot().await.digest;
        assert!(matches!(
            f.land_push_as(&f.owner, None, &push, &parent).await,
            Verdict::Refused(_)
        ));
        assert_eq!(f.snapshot().await.digest, before, "a refusal moves nothing");
        let refused = tracer.take();
        check_land_trace(&refused, &required).unwrap();
        assert!(!refused.iter().any(|s| s.action.kind() == "land"));

        // An approval of exactly this tip on this target, through ingress.
        let approval = Evidence {
            community: *f.tenant.community().as_uuid(),
            repo: repo.to_string(),
            git_ref: "refs/heads/main".into(),
            head: tip.clone(),
            base: base.clone(),
            body: EvidenceBody::Approval { approved: true },
        }
        .builder()
        .unwrap()
        .sign_with_keys(&f.member)
        .unwrap();
        crate::handlers::ingest::ingest_event(&f.state, &f.tenant, approval, auth(&f.member))
            .await
            .unwrap();
        assert_eq!(
            f.land_push_as(&f.owner, None, &push, &parent).await,
            Verdict::Landed
        );
        let landed = tracer.take();
        check_land_trace(&landed, &["decide", "land", "receipt"]).unwrap();

        // Seed receipt-omits-evidence: the durable receipt names the
        // approval the decision counted.
        let counted = landed
            .iter()
            .find_map(|s| match &s.action {
                LandAction::Decide { evidence, .. } => Some(evidence.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(counted.len(), 1, "the decision counted the approval");
        let stored: Vec<String> = sqlx::query_scalar(
            "SELECT jsonb_array_elements_text(detail->'counted'->'refs/heads/main'->'evidence') \
             FROM audit_log WHERE community_id = $1 AND action = 'git_landed' AND object_id = $2",
        )
        .bind(f.tenant.community().as_uuid())
        .bind(repo.to_string())
        .fetch_all(&f.pool)
        .await
        .unwrap();
        assert_eq!(
            stored,
            counted.iter().map(|o| o.0.clone()).collect::<Vec<_>>()
        );

        // The checker sees a broken fact in this same real trace.
        let broken = |edit: &dyn Fn(&mut LandAction) -> bool| {
            let mut t = landed.clone();
            assert!(t.iter_mut().any(|s| edit(&mut s.action)), "nothing edited");
            check_land_trace(&t, &["decide", "land", "receipt"]).is_err()
        };
        assert!(
            broken(&|a| match a {
                LandAction::Decide { read_version, .. } => {
                    *read_version += 1;
                    true
                }
                _ => false,
            }),
            "authority-at-receive: decided on other authority than it landed on"
        );
        assert!(
            broken(&|a| match a {
                LandAction::Receipt { evidence, .. } => {
                    evidence.clear();
                    true
                }
                _ => false,
            }),
            "receipt-omits-evidence"
        );
        assert!(
            broken(&|a| match a {
                LandAction::Decide { valid_until, .. } => {
                    *valid_until = Some(0);
                    true
                }
                _ => false,
            }),
            "expiry-at-read: landed after the authority lapsed"
        );
        let mut dropped = refused.clone();
        dropped.retain(|s| s.action.kind() != "receipt");
        assert!(
            check_land_trace(&dropped, &required).is_err(),
            "no-receipt-on-refusal"
        );
    }

    struct UnavailableReplayGuard;

    impl buzz_auth::Nip98ReplayGuard for UnavailableReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            _event_id: &'a nostr::EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            Box::pin(async {
                Err(buzz_auth::AuthError::Nip98Invalid(
                    "injected backend failure".into(),
                ))
            })
        }
    }

    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn default_branch_replay_outage_and_deletion_fail_closed() {
        let mut f = Fixture::new().await;
        let before = f.snapshot().await.digest;
        let original = f.state.clone();
        let mut state = (*original).clone();
        state.nip98_replay = Arc::new(UnavailableReplayGuard);
        f.state = Arc::new(state);
        for body in [
            None,
            Some(json!({"branch":"main", "expected_manifest":before})),
        ] {
            let (status, body) = f.call(&f.owner, body, None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
            assert!(body.to_string().contains("replay check unavailable"));
        }
        assert_eq!(f.snapshot().await.digest, before);
        f.state = original;
        // Enter the deletion executor's transaction scope in this disposable
        // fixture; the DB correctly rejects unfenced ad-hoc state changes.
        let mut tx = f.pool.begin().await.unwrap();
        sqlx::query("SELECT set_config('buzz.deletion_executor_community', $1, true), set_config('buzz.deletion_fence_generation', '0', true)")
            .bind(f.tenant.community().to_string())
            .execute(&mut *tx).await.unwrap();
        sqlx::query("UPDATE communities SET deletion_state = 'quiescing' WHERE id = $1")
            .bind(f.tenant.community().as_uuid())
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_ne!(f.set(&f.owner, "main", None).await.0, StatusCode::OK);
        assert_eq!(f.snapshot().await.digest, before);
    }

    // ── NIP-FI state re-read: denied assertion does NOT advance stored digest ─
    //
    // Verifies that a POST to `set_default_branch` denied by NIP-FI admission
    // (key mismatch) leaves the stored snapshot digest unchanged.
    //
    // Proof structure:
    //   1. Snapshot the current digest before any NIP-FI requests.
    //   2. POST with a key-mismatch assertion (assertion key ≠ NIP-98 key) →
    //      403 EvidenceRejected.  The handler is never reached.
    //   3. POST with an invalid proof (syntactically malformed token) →
    //      403 EvidenceRejected.  The handler is never reached.
    //   4. Re-read the snapshot → digest is unchanged.
    //   5. POST with a same-key assertion (admission passes) → 200 OK (changed/not-changed).
    //   6. Re-read the snapshot → digest IS advanced if changed=true.
    //
    // This is the NIP-FI denial-no-write witness.  Source ordering at
    // settings.rs:189-207 supports current correctness; this test is the
    // regression guard.
    //
    // Falsifying mutation: call `DefaultBranchSnapshot::set` before NIP-FI
    // admission check → denied requests would modify stored state →
    // digest changes → step 4 assertion fires.
    //
    // Uses `build_router` (not `git_router`) so NIP-FI admission is exercised
    // at the router level.
    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn nip_fi_denied_assertion_does_not_advance_snapshot_digest() {
        use buzz_auth::{
            AssertionKeySet, FederatedAssertionVerifier, FreshnessClass, IssuerPolicy,
            IssuerRegistry, StaticIssuerKeySource, TokenClass, VerifyAssertion,
        };
        use jsonwebtoken::{jwk::JwkSet, Algorithm, EncodingKey, Header};

        let f = Fixture::new().await;
        let snapshot_before = f.snapshot().await;
        let digest_before = snapshot_before.digest.clone();

        // ── Inject NIP-FI Enforce + static verifier into the fixture state ──
        //
        // EC P-256 test key (PKCS#8 PEM) + matching public JWK.
        const NIP_FI_ISSUER: &str = "https://nip-fi-settings-test.invalid";
        const NIP_FI_AUDIENCE: &str = "https://relay.settings-test.invalid";
        const NIP_FI_KID: &str = "settings-test-key-1";
        const NIP_FI_EC_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\nWZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\nzhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n-----END PRIVATE KEY-----\n"; // sadscan:disable np.pem.1 kingfisher.privkey.2 -- unchanged public ES256 test fixture

        // Public key coordinates for the JWK (matches the private key above).
        const NIP_FI_JWK_X: &str = "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI";
        const NIP_FI_JWK_Y: &str = "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA";

        let jwks: JwkSet = serde_json::from_value(serde_json::json!({
            "keys": [{
                "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256",
                "kid": NIP_FI_KID,
                "x": NIP_FI_JWK_X,
                "y": NIP_FI_JWK_Y
            }]
        }))
        .expect("valid test JWKS");
        let hard_deadline = chrono::Utc::now() + chrono::Duration::seconds(3600);
        let key_set =
            AssertionKeySet::new_for_test(NIP_FI_ISSUER.to_owned(), 1, jwks, hard_deadline)
                .expect("valid test key set");
        let jwks_contract = buzz_auth::JwksSourceContract::new(
            format!("{NIP_FI_ISSUER}/.well-known/jwks.json"),
            300,
            3600,
        )
        .expect("valid jwks contract");
        let policy = IssuerPolicy::new(
            NIP_FI_ISSUER.to_owned(),
            vec![NIP_FI_AUDIENCE.to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![Algorithm::ES256],
            60,
            3600,
            None,
            jwks_contract,
        )
        .expect("valid issuer policy");
        let mut registry = IssuerRegistry::new();
        registry.insert(policy);
        let verifier: Arc<dyn VerifyAssertion> = Arc::new(FederatedAssertionVerifier::new(
            registry,
            StaticIssuerKeySource::new([key_set]),
        ));

        let mut enforced_state = (*f.state).clone();
        let config = Arc::make_mut(&mut enforced_state.config);
        config.nip_fi.mode = buzz_auth::NipFiMode::Enforce;
        config.nip_fi.communities =
            crate::nip_fi_core::test_support::any_host("https://relay.settings-test.invalid");
        enforced_state.nip_fi_verifier = Some(verifier);
        let enforced_state = Arc::new(enforced_state);

        // Helper: mint a NIP-FI assertion whose `nostr_pubkey` = `pubkey_hex`.
        let mint_assertion = |pubkey_hex: &str| -> String {
            let now = chrono::Utc::now().timestamp();
            let claims = serde_json::json!({
                "iss": NIP_FI_ISSUER,
                "aud": NIP_FI_AUDIENCE,
                "iat": now,
                "exp": now + 600,
                "sub": "test-subject",
                "nostr_pubkey": pubkey_hex,
            });
            let mut header = Header::new(Algorithm::ES256);
            header.kid = Some(NIP_FI_KID.to_owned());
            header.typ = Some("nip-fi+jwt".to_owned());
            let key =
                EncodingKey::from_ec_pem(NIP_FI_EC_PEM.as_bytes()).expect("valid test EC PEM");
            jsonwebtoken::encode(&header, &claims, &key).expect("sign assertion")
        };

        // Helper: build a NIP-98 POST token for the settings endpoint.
        let settings_path = f.path();
        let settings_url = format!("http://{}{settings_path}", f.tenant.host());
        let post_body =
            serde_json::json!({"branch": "main", "expected_manifest": digest_before}).to_string();
        let post_body_bytes = post_body.as_bytes();

        let build_post_request = |auth_token: String, assertion: Option<String>| {
            let mut builder = Request::builder()
                .method("POST")
                .uri(&settings_path)
                .header("host", f.tenant.host())
                .header("authorization", auth_token)
                .header("content-type", "application/json");
            if let Some(a) = assertion {
                builder = builder.header(buzz_auth::CLIENT_ATTACHED_HEADER, format!("Bearer {a}"));
            }
            builder
                .body(Body::from(post_body_bytes.to_vec()))
                .expect("build request")
        };

        // ── Step 2: key-mismatch assertion (key_a vs key_owner) → 403 ────────
        let key_a = Keys::generate(); // assertion identity ≠ NIP-98 signer
        let assertion_key_a = mint_assertion(&key_a.public_key().to_hex());
        // NIP-98 signed by owner (f.owner), assertion claims key_a — mismatch.
        let mismatch_token = token(&f.owner, "POST", &settings_url, Some(&post_body));
        let (status_mismatch, body_mismatch) = response(
            crate::router::build_router(Arc::clone(&enforced_state))
                .oneshot(build_post_request(mismatch_token, Some(assertion_key_a)))
                .await
                .expect("router oneshot"),
        )
        .await;
        assert_eq!(
            status_mismatch,
            StatusCode::FORBIDDEN,
            "Key-mismatch assertion MUST deny 403 (AuthorizationDenied). \
             The handler must NOT be reached."
        );
        assert_eq!(
            body_mismatch["error"].as_str().unwrap_or(""),
            "authorization denied\n",
            "Key-mismatch 403 body MUST be exact 'authorization denied\\n'. \
             Falsifying mutation: remove key-pairing check → handler reached → \
             different body."
        );

        // ── Step 3: malformed token → 403 EvidenceRejected ───────────────────
        let bad_token = "Nostr !!!bad!!!".to_string();
        let assertion_owner = mint_assertion(&f.owner.public_key().to_hex());
        let (status_malformed, body_malformed) = response(
            crate::router::build_router(Arc::clone(&enforced_state))
                .oneshot(build_post_request(bad_token, Some(assertion_owner.clone())))
                .await
                .expect("router oneshot"),
        )
        .await;
        assert_eq!(
            status_malformed,
            StatusCode::FORBIDDEN,
            "Malformed NIP-98 token MUST deny 403 (EvidenceRejected). \
             The handler must NOT be reached."
        );
        assert_eq!(
            body_malformed["error"].as_str().unwrap_or(""),
            "evidence rejected\n",
            "Malformed NIP-98 token 403 body MUST be exact 'evidence rejected\\n'. \
             Falsifying mutation: remap EvidenceRejected to MissingEvidence → \
             returns 401 instead of 403."
        );

        // ── Step 3b: wrong-payload-hash token → 403 EvidenceRejected ─────────
        // Same key (owner) + valid owner assertion + NIP-98 token whose
        // payload hash is computed from a DIFFERENT body ("wrong body"), but
        // the request sends `post_body_bytes`.  `admit_nip_fi_http` verifies the
        // payload tag before reaching the handler: hash mismatch → EvidenceRejected
        // 403.
        //
        // This is the isolated wrong-payload-hash witness.  It proves the hash
        // check fires independently of key pairing.
        //
        // Falsifying mutation: remove payload-hash verification from
        // `make_nip98_closure_for_admission` → wrong-hash token passes →
        // handler reached → non-403 result.
        let wrong_hash_token = token(
            &f.owner,
            "POST",
            &settings_url,
            Some("wrong body for hash mismatch"),
        );
        let assertion_owner_3b = mint_assertion(&f.owner.public_key().to_hex());
        let (status_wrong_hash, body_wrong_hash) = response(
            crate::router::build_router(Arc::clone(&enforced_state))
                .oneshot(build_post_request(
                    wrong_hash_token,
                    Some(assertion_owner_3b),
                ))
                .await
                .expect("router oneshot"),
        )
        .await;
        assert_eq!(
            status_wrong_hash,
            StatusCode::FORBIDDEN,
            "Wrong-hash NIP-98 POST MUST deny 403 (EvidenceRejected). \
             Token payload hash is bound to 'wrong body for hash mismatch', \
             but actual request body is post_body_bytes — hash mismatch. \
             Falsifying mutation: remove payload-hash check → handler reached."
        );
        assert_eq!(
            body_wrong_hash["error"].as_str().unwrap_or(""),
            "evidence rejected\n",
            "Wrong-hash 403 body MUST be exact 'evidence rejected\\n'. \
             Falsifying mutation: remap payload-hash EvidenceRejected → handler \
             reached → different body."
        );

        // ── Step 4: digest unchanged after all three denials ─────────────────
        // Three NIP-FI denials (key-mismatch, malformed, wrong-hash) MUST NOT
        // advance the stored snapshot digest.  All three are stopped before the
        // handler runs.
        let digest_after_denials = f.snapshot().await.digest;
        assert_eq!(
            digest_after_denials, digest_before,
            "Snapshot digest MUST be unchanged after NIP-FI denials. \
             Key-mismatch, malformed-token, and wrong-hash denials must NOT advance \
             stored state.  Falsifying mutation: call set_default_branch before \
             NIP-FI check → digest changes."
        );

        // ── Step 5: owner POST → 200 OK, changed=true, new digest ──────────────
        // Owner NIP-98 + owner assertion → pairing passes → handler reached.
        // The land authorizes admins of the repository's channel;
        // the fixture makes `f.owner` the channel's owner, so the request is
        // authorized.  The POST requests `branch: "main"`,
        // which exists in the seeded git store, so `set_default_branch` runs and
        // returns `changed: true` with a new HEAD digest.
        //
        // Falsifying mutation A: remove NIP-FI admission from the settings handler
        //   → the mismatched-key token above would have reached the handler (the outer
        //   guard verifies but does not pair keys; malformed tokens stay rejected there)
        //   → set_default_branch called → Step 4's assert_eq!(digest)
        //   fires before we get here.
        // Falsifying mutation B: always-deny pairing → 403 AuthorizationDenied →
        //   status != 200 → Step-5 assert_eq!(status_ok, OK) fires.
        let owner_token = token(&f.owner, "POST", &settings_url, Some(&post_body));
        let assertion_owner_step5 = mint_assertion(&f.owner.public_key().to_hex());
        let (status_ok, body_ok) = response(
            crate::router::build_router(Arc::clone(&enforced_state))
                .oneshot(build_post_request(owner_token, Some(assertion_owner_step5)))
                .await
                .expect("router oneshot"),
        )
        .await;
        assert_eq!(
            status_ok,
            StatusCode::OK,
            "Owner POST with valid NIP-FI assertion MUST return 200. \
             the land authorizes the repo author; the seeded 'main' \
             branch exists and expected_manifest matches digest_before. \
             If 403: NIP-FI key-pairing or the land is denying the owner. \
             If 401: NIP-FI outer guard is wrongly denying a valid assertion."
        );
        let ok_json: serde_json::Value = body_ok.clone();
        assert_eq!(
            ok_json.get("changed").and_then(|v| v.as_bool()),
            Some(true),
            "Owner POST 200 body MUST contain changed=true (branch was updated to 'main'). \
             Falsifying mutation: set_default_branch never called → changed=false."
        );

        // ── Step 6: digest changed after successful owner POST ────────────────
        let after_ok = f.snapshot().await;
        assert_eq!(
            after_ok.manifest.head, "refs/heads/main",
            "Owner POST MUST persist HEAD = refs/heads/main"
        );
        assert_eq!(
            after_ok.manifest.parent.as_ref(),
            Some(&digest_before),
            "Owner POST MUST link the new manifest to the pre-POST digest"
        );
        let digest_after_ok = after_ok.digest;
        assert_ne!(
            digest_after_ok, digest_before,
            "Digest MUST change after successful owner POST (branch updated to 'main'). \
             The three denials above left digest_before unchanged; the owner POST \
             updated the branch → new digest. \
             Falsifying mutation: set_default_branch skipped → digest unchanged."
        );
    }
    #[tokio::test]
    #[ignore = "requires PostgreSQL, Redis and MinIO"]
    async fn channel_metadata_cannot_weaken_a_label_scoped_land_rule() {
        use super::super::super::land::{coordinate, Verdict};
        use buzz_core::governance::{command_builder, ExpectedHead};
        let f = Fixture::new().await;
        let owner = f.owner.public_key().to_hex();
        let repo = coordinate(&owner, &f.repo);
        sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'admin') ON CONFLICT (community_id,pubkey) DO UPDATE SET role='admin'")
            .bind(f.tenant.community().as_uuid()).bind(&owner).execute(&f.pool).await.unwrap();
        let auth = crate::handlers::ingest::IngestAuth::Http {
            pubkey: f.owner.public_key(),
            scopes: vec![buzz_auth::Scope::ReposWrite],
            auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
        };
        let labels_id: buzz_core::governance::DocumentId =
            format!("labels:{repo}").parse().unwrap();
        for (document, body) in [
            (
                "floor".parse().unwrap(),
                r#"{"version":1,"body":{"type":"floor","rules":[{"id":"high-review","refs":["refs/heads/main"],"labels":{"tier":"high"},"require":[{"type":"review","count":1,"group":null}]}],"labels":{"tier":{"values":["high","low"],"default":"low"}},"max_grant_seconds":3600}}"#,
            ),
            (
                labels_id.clone(),
                r#"{"version":1,"body":{"type":"labels","values":{"tier":"high"}}}"#,
            ),
        ] {
            let event = command_builder(
                *f.tenant.community().as_uuid(),
                &document,
                &ExpectedHead::Absent,
                body,
            )
            .unwrap()
            .sign_with_keys(&f.owner)
            .unwrap();
            crate::handlers::ingest::ingest_event(&f.state, &f.tenant, event, auth.clone())
                .await
                .unwrap();
        }
        // Reconcile the legacy fixture manifest before attempting a governed land.
        let _ = f.snapshot().await;
        let options = super::super::super::hydrate::HydrationOptions {
            pack_cache: &f.state.git_pack_cache,
            scratch_dir: f.scratch.path(),
            max_pack_bytes: 1024 * 1024,
            max_repo_bytes: 2 * 1024 * 1024,
        };
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &owner,
            &f.repo,
            options,
        )
        .await
        .unwrap();
        let base = git(push.path(), &["rev-parse", "refs/heads/main"]).await;
        let base = base.trim().to_string();
        let tip = git(
            push.path(),
            &[
                "-c",
                "user.name=land",
                "-c",
                "user.email=land@example.invalid",
                "commit-tree",
                "refs/heads/main^{tree}",
                "-p",
                &base,
                "-m",
                "governed",
            ],
        )
        .await;
        let tip = tip.trim().to_string();
        git(push.path(), &["update-ref", "refs/heads/main", &tip]).await;

        assert!(
            matches!(f.land_push(&push, &parent).await, Verdict::Refused(_)),
            "protected high label requires review"
        );
        let edit = EventBuilder::new(Kind::Custom(9002), "")
            .tags([
                Tag::parse(["h", &f.channel.to_string()]).unwrap(),
                Tag::parse(["t", "tier:low"]).unwrap(),
            ])
            .sign_with_keys(&f.owner)
            .unwrap();
        let edit_auth = crate::handlers::ingest::IngestAuth::Http {
            pubkey: f.owner.public_key(),
            scopes: vec![
                buzz_auth::Scope::ChannelsWrite,
                buzz_auth::Scope::AdminChannels,
            ],
            auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
        };
        crate::handlers::ingest::ingest_event(&f.state, &f.tenant, edit, edit_auth)
            .await
            .unwrap();
        assert!(
            matches!(f.land_push(&push, &parent).await, Verdict::Refused(_)),
            "channel metadata must not weaken the land rule"
        );
        let head = f
            .state
            .db
            .governance_head(
                f.tenant.community(),
                &labels_id,
                f.owner.public_key().as_bytes(),
                None,
                !f.state.config.require_relay_membership,
            )
            .await
            .unwrap()
            .unwrap();
        let labels = command_builder(
            *f.tenant.community().as_uuid(),
            &labels_id,
            &ExpectedHead::Event(head.id.to_hex()),
            r#"{"version":1,"body":{"type":"labels","values":{"tier":"low"}}}"#,
        )
        .unwrap()
        .sign_with_keys(&f.owner)
        .unwrap();
        crate::handlers::ingest::ingest_event(&f.state, &f.tenant, labels, auth)
            .await
            .unwrap();
        assert_eq!(
            f.land_push(&push, &parent).await,
            Verdict::Landed,
            "a governed label change can alter the decision"
        );
    }

    /// NIP-IA archival is visibility, not access (NIP-IA.md:436): an archived
    /// identity's push lands.
    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn an_archived_identity_push_lands() {
        let f = Fixture::new().await;
        let owner = f.owner.public_key().to_hex();
        assert!(f
            .state
            .db
            .archive(
                f.tenant.community(),
                &owner,
                "self",
                &owner,
                None,
                None,
                &"c".repeat(64)
            )
            .await
            .unwrap());
        let before = f.snapshot().await.digest;
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &owner,
            &f.repo,
            super::super::super::hydrate::HydrationOptions {
                pack_cache: &f.state.git_pack_cache,
                scratch_dir: f.scratch.path(),
                max_pack_bytes: 1024 * 1024,
                max_repo_bytes: 2 * 1024 * 1024,
            },
        )
        .await
        .unwrap();
        git(
            push.path(),
            &["update-ref", "refs/heads/archived", "refs/heads/main"],
        )
        .await;
        assert_eq!(
            f.land_push(&push, &parent).await,
            super::super::super::land::Verdict::Landed
        );
        assert_ne!(f.snapshot().await.digest, before);
    }

    /// As a relay that predates the land pushes: hydrate the pointer, add
    /// `branch`, publish by the pointer CAS alone. The row does not move.
    async fn legacy_publish(
        f: &Fixture,
        branch: &str,
    ) -> (super::super::super::manifest::Manifest, String) {
        let owner = f.owner.public_key().to_hex();
        let scratch = f.scratch.path().join(branch);
        std::fs::create_dir(&scratch).unwrap();
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &owner,
            &f.repo,
            super::super::super::hydrate::HydrationOptions {
                pack_cache: &f.state.git_pack_cache,
                scratch_dir: &scratch,
                max_pack_bytes: 1024 * 1024,
                max_repo_bytes: 2 * 1024 * 1024,
            },
        )
        .await
        .unwrap();
        git(
            push.path(),
            &[
                "update-ref",
                &format!("refs/heads/{branch}"),
                "refs/heads/main",
            ],
        )
        .await;
        let published = super::super::super::cas_publish::cas_publish(
            &f.state.git_store,
            &f.tenant,
            push.path(),
            &owner,
            &f.repo,
            &parent,
            &super::super::super::cas_publish::snapshot_workspace(push.path())
                .await
                .unwrap(),
            limits(push.hydrated_bytes()),
        )
        .await
        .unwrap();
        let digest = published.manifest_key.strip_prefix("manifests/").unwrap();
        (published.manifest, digest.to_owned())
    }

    /// Mixed-version rollout: a relay without the land publishes by the
    /// pointer CAS alone, moving the cache m0 -> m1 (parent m0) while the
    /// row stays at m0. An ungoverned repository adopts m1, because it was
    /// built directly on the row: reads serve it and writes build on it. A
    /// governed one refuses writes with 409 and reads serve the row, hiding
    /// m1, until an operator reconciles it. So does an ungoverned one whose
    /// cache is more than one legacy push past the row.
    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn a_legacy_pointer_publish_is_adopted_only_when_ungoverned_and_direct() {
        use super::super::super::land::{committed_for_read, committed_for_write, coordinate};
        for governed in [false, true] {
            let f = Fixture::new().await;
            if governed {
                f.floor().await;
            }
            let repo = coordinate(&f.owner.public_key().to_hex(), &f.repo);
            let row = || async {
                f.state
                    .db
                    .git_repo_manifest(f.tenant.community(), &repo)
                    .await
                    .unwrap()
            };
            // A read seeds the row from the cache: row = cache = m0.
            let m0 = f.snapshot().await;
            assert_eq!(row().await.as_deref(), Some(m0.digest.as_str()));
            let (m1, d1) = legacy_publish(&f, "legacy-push").await;
            assert_eq!(m1.parent.as_deref(), Some(m0.digest.as_str()));
            assert_eq!(row().await.as_deref(), Some(m0.digest.as_str()));
            if governed {
                let refused = committed_for_write(&f.state, &f.tenant, &repo)
                    .await
                    .map(|_| ())
                    .expect_err("a legacy publish refuses governed writes");
                assert_eq!(refused.status(), StatusCode::CONFLICT);
                assert_eq!(
                    f.call(
                        &f.owner,
                        Some(json!({"branch": "main", "expected_manifest": m0.digest})),
                        None
                    )
                    .await
                    .0,
                    StatusCode::CONFLICT,
                    "the owner's governed write through the route refuses too"
                );
                let read = committed_for_read(&f.state, &f.tenant, &repo)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(read, m0.manifest, "governed reads serve the row");
                assert_eq!(row().await.as_deref(), Some(m0.digest.as_str()));
                continue;
            }
            // Each door adopts on its own: a write adopts m1, a read adopts
            // m2 (parent m1).
            let (_, digest, _) = committed_for_write(&f.state, &f.tenant, &repo)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(digest, d1, "writes build on the adopted push");
            assert_eq!(row().await.as_deref(), Some(d1.as_str()));
            let (_, d2) = legacy_publish(&f, "second").await;
            let read = committed_for_read(&f.state, &f.tenant, &repo)
                .await
                .unwrap()
                .unwrap();
            assert!(read.refs.contains_key("refs/heads/second"), "reads adopt");
            assert_eq!(row().await.as_deref(), Some(d2.as_str()));
            // Two legacy pushes past the row: the cache's parent is not the
            // row, so nothing is adopted.
            let (_, d3) = legacy_publish(&f, "third").await;
            let (m4, _) = legacy_publish(&f, "fourth").await;
            assert_eq!(m4.parent.as_deref(), Some(d3.as_str()));
            assert_eq!(
                committed_for_write(&f.state, &f.tenant, &repo)
                    .await
                    .map(|_| ())
                    .expect_err("an indirect legacy publish refuses")
                    .status(),
                StatusCode::CONFLICT
            );
            assert_eq!(row().await.as_deref(), Some(d2.as_str()));
        }
    }
}
