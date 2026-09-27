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
        let mut config = crate::config::Config::from_env().ok()?;
        config.database_url = crate::test_support::database_url();
        config.redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        config.relay_url = "ws://nip-fi-settings-test.local".to_string();
        config.require_auth_token = true;
        config.require_relay_membership = false;
        config.nip_fi.mode = buzz_auth::NipFiMode::Enforce;

        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
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
    const TEST_EC_PKCS8_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\n\
        WZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\n\
        zhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n\
        -----END PRIVATE KEY-----\n";
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

        let mut config = crate::config::Config::from_env().ok()?;
        config.database_url = crate::test_support::database_url();
        config.redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        config.relay_url = "ws://nip-fi-settings-pairing-test.local".to_string();
        config.require_auth_token = true;
        config.require_relay_membership = false;
        config.nip_fi.mode = buzz_auth::NipFiMode::Enforce;

        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
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
        let mut config = crate::config::Config::from_env().ok()?;
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
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
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
             If 403: key pairing is wrongly denying, or authorize_management denied. \
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
            let mut config = crate::config::Config::from_env().unwrap();
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
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .unwrap();
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
                &self.state.git_store,
                &self.tenant,
                &self.owner.public_key().to_hex(),
                &self.repo,
            )
            .await
            .unwrap()
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
            f.set(&f.member, "main", None).await.0,
            StatusCode::FORBIDDEN,
            "push-capable channel admin is not a repo manager"
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
        let result = f.set(&f.maintainer, "main", None).await;
        assert_eq!(result.0, StatusCode::OK, "{result:?}");
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
    async fn default_branch_delegation_and_revocation() {
        let f = Fixture::new().await;
        let agent = Keys::generate();
        f.add(&agent).await;
        let tag = buzz_sdk::nip_oa::compute_auth_tag(&f.owner, &agent.public_key(), "").unwrap();
        assert_eq!(f.set(&agent, "main", None).await.0, StatusCode::FORBIDDEN);
        let limited =
            buzz_sdk::nip_oa::compute_auth_tag(&f.owner, &agent.public_key(), "kind=1").unwrap();
        assert_eq!(
            f.set(&agent, "main", Some(&limited)).await.0,
            StatusCode::FORBIDDEN
        );
        let expired =
            buzz_sdk::nip_oa::compute_auth_tag(&f.owner, &agent.public_key(), "created_at<1")
                .unwrap();
        assert_eq!(
            f.set(&agent, "main", Some(&expired)).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(f.set(&agent, "main", Some(&tag)).await.0, StatusCode::OK);
        // Optional credential does not take direct authority away.
        let absent_owner = Keys::generate();
        let own_tag =
            buzz_sdk::nip_oa::compute_auth_tag(&absent_owner, &f.owner.public_key(), "").unwrap();
        assert_eq!(
            f.set(&f.owner, "legacy", Some(&own_tag)).await.0,
            StatusCode::OK
        );
        // A human can administer a repository announced by their managed agent.
        f.state
            .db
            .set_agent_owner(
                f.tenant.community(),
                f.owner.public_key().as_bytes(),
                f.member.public_key().as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(f.set(&f.member, "main", None).await.0, StatusCode::OK);
        f.state
            .db
            .add_member(
                f.tenant.community(),
                f.channel,
                f.maintainer.public_key().as_bytes(),
                MemberRole::Owner,
                Some(f.owner.public_key().as_bytes()),
            )
            .await
            .unwrap();
        f.state
            .db
            .remove_member(
                f.tenant.community(),
                f.channel,
                f.owner.public_key().as_bytes(),
                f.owner.public_key().as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(
            f.set(&agent, "legacy", Some(&tag)).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            f.set(&f.owner, "legacy", None).await.0,
            StatusCode::NOT_FOUND
        );
        // Durable ban cascades even when the signer has independent maintainer rights.
        let ban_tag =
            buzz_sdk::nip_oa::compute_auth_tag(&f.member, &f.maintainer.public_key(), "").unwrap();
        f.state
            .db
            .ban_community_member(
                f.tenant.community(),
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
            .bind(f.tenant.community().as_uuid())
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
        let (_, changed) = a
            .set(
                &f.state.git_store,
                SetDefaultBranch {
                    branch: "main".into(),
                    expected_manifest: old_digest.clone(),
                },
            )
            .await
            .unwrap();
        assert!(changed);
        let loser = b
            .set(
                &f.state.git_store,
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
        let result = super::super::super::cas_publish::cas_publish(
            &f.state.git_store,
            &f.tenant,
            push.path(),
            &f.owner.public_key().to_hex(),
            &f.repo,
            &parent,
            limits(push.hydrated_bytes()),
        )
        .await;
        assert!(matches!(
            result,
            Err(super::super::super::cas_publish::CasError::Conflict { .. })
        ));
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
        super::super::super::cas_publish::cas_publish(
            &f.state.git_store,
            &f.tenant,
            push.path(),
            &f.owner.public_key().to_hex(),
            &f.repo,
            &parent,
            limits(push.hydrated_bytes()),
        )
        .await
        .unwrap();
        assert_eq!(
            stale
                .set(
                    &f.state.git_store,
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
        super::super::super::cas_publish::cas_publish(
            &f.state.git_store,
            &f.tenant,
            push.path(),
            &f.owner.public_key().to_hex(),
            &f.repo,
            &parent,
            limits(push.hydrated_bytes()),
        )
        .await
        .unwrap();
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
        super::super::super::cas_publish::cas_publish(
            &f.state.git_store,
            &f.tenant,
            push.path(),
            &f.owner.public_key().to_hex(),
            &f.repo,
            &parent,
            limits(push.hydrated_bytes()),
        )
        .await
        .unwrap();
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
        const NIP_FI_EC_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
            MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\n\
            WZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\n\
            zhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n\
            -----END PRIVATE KEY-----\n";
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
        Arc::make_mut(&mut enforced_state.config).nip_fi.mode = buzz_auth::NipFiMode::Enforce;
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
        // `authorize_management` at settings.rs:273-292 authorizes the repository
        // author OR a named maintainer.  The fixture signs the announcement with
        // `f.owner` (see fixture:1267-1342), so `named_manager(&auth.caller)` is
        // true and the request is authorized.  The POST requests `branch: "main"`,
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
             authorize_management authorizes the repo author; the seeded 'main' \
             branch exists and expected_manifest matches digest_before. \
             If 403: NIP-FI key-pairing or authorize_management is denying the owner. \
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
}
