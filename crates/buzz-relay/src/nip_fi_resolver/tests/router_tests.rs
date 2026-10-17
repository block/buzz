//! Database-resolver admission through the real router (plan criteria 1, 2,
//! 3, 4, 5, 6f, 7e, 8). Root and audio upgrades and the protected-HTTP guard all
//! run against a resolver over an in-memory store that counts lookups; the
//! assertion verifier is the production one with a seeded key snapshot.
//! The relay database is unreachable, so a request that clears NIP-FI ends
//! at the next step: the upgrades' tenant lookup or the route's extractor.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use buzz_auth::{
    FederatedAssertionVerifier, FreshnessClass, HttpJwksFetcher, IssuerJwksConfig, IssuerPolicy,
    IssuerRegistry, JwksSourceContract, NipFiMode, ProductionJwksSource, TokenClass,
};
use tower::ServiceExt;

use super::*;
use crate::state::AppState;

/// A second registered issuer. Never seeded with keys, so any verification
/// that reaches its key source is `authorization unavailable`.
const ISS_B: &str = "https://issuer-b.example";
const KID: &str = "resolver-router-test-key";
// Same public ES256 fixture as the router and command tests.
const PRIVATE_KEY_PEM: &str =
    "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\nWZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\nzhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n-----END PRIVATE KEY-----\n"; // sadscan:disable np.pem.1 kingfisher.privkey.2 -- public ES256 test fixture

const UNAVAILABLE_BODY: &[u8] = b"authorization unavailable\n";
const REJECTED_BODY: &[u8] = b"evidence rejected\n";

/// Host → bound issuers, counting every per-host lookup.
#[derive(Default)]
struct Bindings {
    rows: Mutex<HashMap<String, Vec<String>>>,
    lookups: Mutex<Vec<String>>,
    lookup_fails: std::sync::atomic::AtomicBool,
}

impl Bindings {
    fn bind(&self, label: &str, issuers: &[&str]) {
        let issuers = issuers.iter().map(|i| (*i).to_owned()).collect();
        self.rows.lock().unwrap().insert(host(label), issuers);
    }
    fn unbind(&self, label: &str) {
        self.rows.lock().unwrap().remove(&host(label));
    }
    fn lookups_for(&self, raw_host: &str) -> usize {
        let lookups = self.lookups.lock().unwrap();
        lookups.iter().filter(|h| *h == raw_host).count()
    }
}

impl BindingSource for Bindings {
    fn load<'a>(
        &'a self,
        _registry: &'a [String],
        _max_rows: usize,
    ) -> BoxFuture<'a, Result<Result<Vec<(String, String)>, RowCapExceeded>, String>> {
        async move {
            let rows =
                self.rows
                    .lock()
                    .unwrap()
                    .iter()
                    .fold(Vec::new(), |mut acc, (h, issuers)| {
                        acc.extend(issuers.iter().map(|i| (h.clone(), i.clone())));
                        acc
                    });
            Ok(Ok(rows))
        }
        .boxed()
    }

    fn lookup<'a>(
        &'a self,
        host: &'a str,
        _registry: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<String>, String>> {
        async move {
            self.lookups.lock().unwrap().push(host.to_owned());
            if self.lookup_fails.load(Ordering::SeqCst) {
                return Err("pool closed".into());
            }
            Ok(self
                .rows
                .lock()
                .unwrap()
                .get(host)
                .cloned()
                .unwrap_or_default())
        }
        .boxed()
    }
}

fn jwks_contract(issuer: &str) -> JwksSourceContract {
    JwksSourceContract::new(format!("{issuer}/.well-known/jwks.json"), 300, 86_400)
        .expect("valid JWKS contract")
}

fn issuer_policy(issuer: &str) -> IssuerPolicy {
    IssuerPolicy::new(
        issuer.to_owned(),
        vec!["https://communities.buzz.xyz".to_owned()],
        TokenClass::DedicatedNipFi,
        FreshnessClass::OfflineJwt,
        vec![jsonwebtoken::Algorithm::ES256],
        30,
        3600,
        None,
        jwks_contract(issuer),
    )
    .expect("valid issuer policy")
}

/// A valid assertion from `iss` for `https://<raw_host>`, signed by the
/// fixture key.
fn mint(iss: &str, raw_host: &str) -> String {
    mint_for(iss, raw_host, &nostr::Keys::generate().public_key())
}

/// [`mint`] asserting `key`.
fn mint_for(iss: &str, raw_host: &str, key: &nostr::PublicKey) -> String {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    let now = chrono::Utc::now().timestamp();
    let claims = serde_json::json!({
        "iss": iss,
        "aud": format!("https://{raw_host}"),
        "sub": "resolver-router-test",
        "iat": now,
        "exp": now + 600,
        "nostr_pubkey": key.to_hex(),
    });
    let mut header = Header::new(Algorithm::ES256);
    header.typ = Some("nip-fi+jwt".to_owned());
    header.kid = Some(KID.to_owned());
    let key = EncodingKey::from_ec_pem(PRIVATE_KEY_PEM.as_bytes()).expect("test EC key");
    encode(&header, &claims, &key).expect("sign assertion")
}

/// One relay pod: router state with the database resolver installed.
struct Pod {
    resolver: Arc<DatabaseResolver>,
    state: Arc<AppState>,
}

async fn pod(store: &Arc<Bindings>, clock: &Arc<ManualClock>, mode: NipFiMode) -> Pod {
    let base =
        crate::state::tests::test_state_with_database_url("postgres://127.0.0.1:1/none").await;
    pod_on(base, store, clock, mode).await
}

/// [`pod`] over a caller-built base state (its tenant store).
async fn pod_on(
    base: Arc<AppState>,
    store: &Arc<Bindings>,
    clock: &Arc<ManualClock>,
    mode: NipFiMode,
) -> Pod {
    let resolver = Arc::new(DatabaseResolver::with_clock(
        Arc::clone(store) as Arc<dyn BindingSource>,
        policy(),
        vec![ISS.to_owned(), ISS_B.to_owned()],
        ResolverBounds::default(),
        Arc::clone(clock) as Arc<dyn Clock>,
    ));
    resolver.refresh().await.expect("startup load");

    let mut state = (*base).clone();
    let config = Arc::make_mut(&mut state.config);
    config.nip_fi.mode = mode;
    config.nip_fi.source = crate::nip_fi_config::CommunitySource::Database;
    config.nip_fi.registry = {
        let mut registry = IssuerRegistry::new();
        registry.insert(issuer_policy(ISS));
        registry.insert(issuer_policy(ISS_B));
        registry
    };
    let key_source = Arc::new(
        ProductionJwksSource::new(
            [ISS, ISS_B]
                .map(|issuer| IssuerJwksConfig {
                    issuer: issuer.to_owned(),
                    contract: jwks_contract(issuer),
                })
                .into(),
            HttpJwksFetcher::new(),
        )
        .expect("key source"),
    );
    let jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(serde_json::json!({
        "kty": "EC", "crv": "P-256", "alg": "ES256", "use": "sig", "kid": KID,
        "x": "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI",
        "y": "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA",
    }))
    .expect("test JWK");
    key_source
        .seed_snapshot_for_test(ISS, jsonwebtoken::jwk::JwkSet { keys: vec![jwk] })
        .await;
    state.nip_fi_verifier = Some(Arc::new(FederatedAssertionVerifier::new(
        state.config.nip_fi.registry.clone(),
        Arc::clone(&key_source),
    )));
    state.nip_fi_jwks_source = Some(key_source);
    state.nip_fi_resolver = Some(Arc::clone(&resolver));
    Pod {
        resolver,
        state: Arc::new(state),
    }
}

/// The three NIP-FI admission sites: root upgrade, audio upgrade, protected
/// HTTP (the router guard).
fn sites() -> [String; 3] {
    [
        "/".to_owned(),
        format!("/huddle/{}/audio", uuid::Uuid::new_v4()),
        "/workflows/wf/runs".to_owned(),
    ]
}

impl Pod {
    async fn send(&self, path: &str, raw_host: &str, token: Option<&str>) -> (StatusCode, Vec<u8>) {
        self.send_signed(path, raw_host, token, None).await
    }

    /// [`Pod::send`] with an optional `Authorization` (NIP-98) header.
    async fn send_signed(
        &self,
        path: &str,
        raw_host: &str,
        token: Option<&str>,
        authorization: Option<String>,
    ) -> (StatusCode, Vec<u8>) {
        let mut builder = Request::get(path)
            .header(axum::http::header::HOST, raw_host)
            .header("Upgrade", "websocket")
            .header("Connection", "Upgrade")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
        if let Some(token) = token {
            builder = builder.header("Nostr-Federated-Identity", format!("Bearer {token}"));
        }
        if let Some(authorization) = authorization {
            builder = builder.header(axum::http::header::AUTHORIZATION, authorization);
        }
        let response = crate::router::build_router(Arc::clone(&self.state))
            .oneshot(builder.body(Body::empty()).expect("request"))
            .await
            .expect("router response");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        (status, body.to_vec())
    }

    /// Every site's outcome for a token from `iss` presented at `label`.
    async fn every_site(&self, iss: &str, label: &str) -> Vec<(StatusCode, Vec<u8>)> {
        let token = mint(iss, &host(label));
        let mut outcomes = Vec::new();
        for path in sites() {
            outcomes.push(self.send(&path, &host(label), Some(&token)).await);
        }
        outcomes
    }
}

fn unavailable() -> (StatusCode, Vec<u8>) {
    (StatusCode::SERVICE_UNAVAILABLE, UNAVAILABLE_BODY.to_vec())
}

fn rejected() -> (StatusCode, Vec<u8>) {
    (StatusCode::FORBIDDEN, REJECTED_BODY.to_vec())
}

/// The request cleared NIP-FI admission and was answered by the next step:
/// the upgrades' tenant lookup (no reachable database) or the protected
/// route's path extractor (`wf` is not a UUID).
fn cleared_nip_fi(outcomes: &[(StatusCode, Vec<u8>)]) -> bool {
    outcomes.iter().all(|(status, body)| match *status {
        StatusCode::NOT_FOUND => body == b"relay: no community is configured for this host",
        StatusCode::BAD_REQUEST => body.starts_with(b"Invalid URL: Cannot parse `workflow_id`"),
        _ => false,
    })
}

fn setup() -> (Arc<Bindings>, Arc<ManualClock>) {
    (
        Arc::new(Bindings::default()),
        Arc::new(ManualClock::default()),
    )
}

// ── Criterion 1: one resolution per admission, no suffix fallback ───────────

#[tokio::test]
async fn each_admission_resolves_its_host_exactly_once() {
    let (store, clock) = setup();
    let p = pod(&store, &clock, NipFiMode::Enforce).await;
    // Bound after the startup load: every admission takes the miss lookup,
    // and a found binding is never cached, so lookups count resolutions.
    store.bind("fresh", &[ISS]);
    let token = mint(ISS, &host("fresh"));
    for (n, path) in sites().iter().enumerate() {
        let outcome = p.send(path, &host("fresh"), Some(&token)).await;
        assert!(
            cleared_nip_fi(std::slice::from_ref(&outcome)),
            "{path}: {outcome:?}"
        );
        assert_eq!(store.lookups_for(&host("fresh")), n + 1, "{path}");
    }
}

#[tokio::test]
async fn one_label_suffix_host_with_no_rows_is_unavailable_at_every_site() {
    let (store, clock) = setup();
    let p = pod(&store, &clock, NipFiMode::Enforce).await;
    assert_eq!(p.every_site(ISS, "nobody").await, vec![unavailable(); 3]);
}

// ── Criterion 2: an issuer the community does not list never reaches keys ───

#[tokio::test]
async fn unlisted_issuer_is_rejected_before_its_key_source() {
    let (store, clock) = setup();
    store.bind("only-a", &[ISS]);
    store.bind("lists-b", &[ISS_B]);
    let p = pod(&store, &clock, NipFiMode::Enforce).await;
    // Issuer B has no key snapshot. Where B is listed, verification reaches
    // that key source and is unavailable; where it is not, the allowlist
    // rejects the same kind of token first, so the missing keys never show.
    assert_eq!(p.every_site(ISS_B, "lists-b").await, vec![unavailable(); 3]);
    assert_eq!(p.every_site(ISS_B, "only-a").await, vec![rejected(); 3]);
    assert!(cleared_nip_fi(&p.every_site(ISS, "only-a").await));
}

// ── Criterion 3: removing the community's issuer revokes its tokens ─────────

#[tokio::test]
async fn removing_the_issuer_denies_a_previously_valid_token() {
    let (store, clock) = setup();
    store.bind("shared", &[ISS, ISS_B]);
    let p = pod(&store, &clock, NipFiMode::Enforce).await;
    assert!(cleared_nip_fi(&p.every_site(ISS, "shared").await));

    store.bind("shared", &[ISS_B]);
    p.resolver.refresh().await.expect("refresh");
    assert_eq!(p.every_site(ISS, "shared").await, vec![rejected(); 3]);
}

// ── Criterion 4: full admission per request; failed miss stores nothing ─────

#[tokio::test]
async fn second_request_after_unbind_and_refresh_is_denied() {
    let (store, clock) = setup();
    store.bind("gone", &[ISS]);
    let p = pod(&store, &clock, NipFiMode::Enforce).await;
    let token = mint(ISS, &host("gone"));
    let path = "/workflows/wf/runs";
    assert!(cleared_nip_fi(&[p
        .send(path, &host("gone"), Some(&token))
        .await]));

    store.unbind("gone");
    p.resolver.refresh().await.expect("refresh");
    assert_eq!(
        p.send(path, &host("gone"), Some(&token)).await,
        unavailable()
    );
}

#[tokio::test]
async fn closed_pool_miss_is_unavailable_everywhere_and_caches_nothing() {
    let (store, clock) = setup();
    let p = pod(&store, &clock, NipFiMode::Enforce).await;
    store.bind("late", &[ISS]);
    store.lookup_fails.store(true, Ordering::SeqCst);
    assert_eq!(p.every_site(ISS, "late").await, vec![unavailable(); 3]);
    assert_eq!(store.lookups_for(&host("late")), 3, "no negative entry");
    assert!(!p.resolver.negatively_cached(&host("late")));

    store.lookup_fails.store(false, Ordering::SeqCst);
    assert!(cleared_nip_fi(&p.every_site(ISS, "late").await));
}

// ── Criterion 6(f): the reserved command-audience host never admits ─────────

#[tokio::test]
async fn reserved_apex_is_unavailable_without_a_lookup() {
    let (store, clock) = setup();
    store
        .rows
        .lock()
        .unwrap()
        .insert("communities.buzz.xyz".to_owned(), vec![ISS.to_owned()]);
    let p = pod(&store, &clock, NipFiMode::Enforce).await;
    let token = mint(ISS, "communities.buzz.xyz");
    for path in sites() {
        let outcome = p.send(&path, "communities.buzz.xyz", Some(&token)).await;
        assert_eq!(outcome, unavailable(), "{path}");
    }
    assert_eq!(store.lookups_for("communities.buzz.xyz"), 0);
}

// ── Criterion 7(e): negative cache per pod; creating pod admits at once ─────

#[tokio::test]
async fn creation_admits_on_its_pod_now_and_elsewhere_after_negative_ttl() {
    let (store, clock) = setup();
    let p1 = pod(&store, &clock, NipFiMode::Enforce).await;
    let p2 = pod(&store, &clock, NipFiMode::Enforce).await;
    let path = "/workflows/wf/runs";
    let token = mint(ISS, &host("newco"));
    let h = host("newco");
    for p in [&p1, &p2] {
        assert_eq!(p.send(path, &h, Some(&token)).await, unavailable());
    }
    assert_eq!(store.lookups_for(&h), 2);

    // Created through P1, after both published their snapshots.
    store.bind("newco", &[ISS]);
    p1.resolver.record_creation(&h);
    assert!(cleared_nip_fi(&[p1.send(path, &h, Some(&token)).await]));
    assert_eq!(p2.send(path, &h, Some(&token)).await, unavailable());
    assert_eq!(store.lookups_for(&h), 3, "P1 looks up, P2 from cache");

    clock.set(secs(5.1));
    assert!(cleared_nip_fi(&[p2.send(path, &h, Some(&token)).await]));
    assert_eq!(store.lookups_for(&h), 4);
}

// ── Criterion 8: mode off never resolves or gates ───────────────────────────

#[tokio::test]
async fn off_mode_with_database_source_neither_gates_nor_resolves() {
    let (store, clock) = setup();
    store.bind("quiet", &[ISS]);
    let p = pod(&store, &clock, NipFiMode::Off).await;
    for path in sites() {
        let outcome = p.send(&path, &host("quiet"), None).await;
        assert!(
            cleared_nip_fi(std::slice::from_ref(&outcome)),
            "{path}: {outcome:?}"
        );
    }
    assert!(store.lookups.lock().unwrap().is_empty());
}

// Shadow resolves as enforce does: a community bound after the startup load
// is found by the live lookup, so shadow records no `community` would-deny.
// Mutation: resolving shadow from memory only takes no lookup and records a
// `community`-stage denial at every site.
#[tokio::test]
async fn shadow_resolves_through_the_database_lookup_like_enforce() {
    let (store, clock) = setup();
    let p = pod(&store, &clock, NipFiMode::Shadow).await;
    store.bind("late", &[ISS]);
    let token = mint(ISS, &host("late"));
    let recorder = metrics_util::debugging::DebuggingRecorder::new();
    {
        let _guard = metrics::set_default_local_recorder(&recorder);
        for path in sites() {
            let outcome = p.send(&path, &host("late"), Some(&token)).await;
            assert!(
                cleared_nip_fi(std::slice::from_ref(&outcome)),
                "{path}: {outcome:?}"
            );
        }
    }
    assert_eq!(store.lookups_for(&host("late")), sites().len());
    let community_denials = recorder
        .snapshotter()
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, ..)| {
            let key = key.key();
            key.name() == "buzz_nip_fi_shadow_total"
                && key
                    .labels()
                    .any(|l| l.key() == "stage" && l.value() == "community")
        })
        .count();
    assert_eq!(community_denials, 0);
}

// ── Criterion 5: a disconnect covers communities created after it ──────────

#[tokio::test]
#[allow(clippy::result_large_err)] // the NIP-98 closure returns the production `Response` error type
async fn key_disconnected_before_its_community_exists_is_denied_there() {
    let recorder = metrics_util::debugging::DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _metrics = metrics::set_default_local_recorder(&recorder);

    let (store, clock) = setup();
    let p = pod(&store, &clock, NipFiMode::Enforce).await;
    let mut state = (*p.state).clone();
    let deny_map = Arc::new(buzz_auth::NipFiDenyMap::new(
        16,
        vec![buzz_auth::IssuerCapacity {
            issuer: ISS.to_owned(),
            capacity: 16,
        }],
    ));
    state.nip_fi_deny_map = Some(Arc::clone(&deny_map));
    let p = Pod {
        resolver: p.resolver,
        state: Arc::new(state),
    };

    // Disconnect K while H does not exist anywhere.
    let (k_keys, control_keys) = (nostr::Keys::generate(), nostr::Keys::generate());
    let (k, control) = (k_keys.public_key(), control_keys.public_key());
    let now = chrono::Utc::now();
    let merged = deny_map.merge_cross_pod_deny(ISS, &k, now + chrono::Duration::hours(1), now);
    assert!(matches!(merged, buzz_auth::CrossPodMergeResult::Merged));
    let h = host("after");
    assert_eq!(
        p.send("/", &h, Some(&mint_for(ISS, &h, &k))).await,
        unavailable()
    );

    // Provision H through this pod.
    store.bind("after", &[ISS]);
    p.resolver.record_creation(&h);

    let denied = (StatusCode::FORBIDDEN, b"authorization denied\n".to_vec());
    assert_eq!(p.send("/", &h, Some(&mint_for(ISS, &h, &k))).await, denied);
    assert!(cleared_nip_fi(&[p
        .send("/", &h, Some(&mint_for(ISS, &h, &control)))
        .await]));

    // Audio resolves H at upgrade and denies after NIP-42, at its deny-set
    // check, on an assertion verified against H's resolved binding.
    let audio = format!("/huddle/{}/audio", uuid::Uuid::new_v4());
    let (binding, _) = p.resolver.resolve(&h).await.expect("H resolves");
    let verifier = p.state.nip_fi_verifier.as_deref().expect("verifier");
    for (keys, refused) in [(&k_keys, true), (&control_keys, false)] {
        let token = mint_for(ISS, &h, &keys.public_key());
        assert!(cleared_nip_fi(&[p.send(&audio, &h, Some(&token)).await]));
        let assertion = verifier.verify_assertion(&token, &binding).expect("valid");
        let outcome = crate::audio::handler::test_support::deny_set_refuses(
            Arc::clone(&p.state),
            assertion,
            keys,
        )
        .await;
        assert_eq!(outcome, refused, "audio deny-set check");
    }

    let http_admit = |key: nostr::PublicKey| {
        let state = Arc::clone(&p.state);
        let h = h.clone();
        async move {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(axum::http::header::HOST, h.parse().unwrap());
            headers.insert(
                "Nostr-Federated-Identity",
                format!("Bearer {}", mint_for(ISS, &h, &key))
                    .parse()
                    .unwrap(),
            );
            crate::nip_fi_http::admit_nip_fi_http_on_state(&state, &headers, || {
                Ok(crate::nip_fi_http::Nip98Proof::new(key, ()))
            })
            .await
            .map(drop)
        }
    };
    let resp = http_admit(k)
        .await
        .expect_err("K is denied at HTTP admission");
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    assert_eq!((status, body.to_vec()), denied);
    assert!(http_admit(control).await.is_ok(), "control key is admitted");

    let http_denials: u64 = snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, ..)| {
            key.key().name() == "buzz_auth_failures_total"
                && key
                    .key()
                    .labels()
                    .any(|l| l.value() == "nip_fi_http_denied_pubkey")
        })
        .map(|(.., value)| match value {
            metrics_util::debugging::DebugValue::Counter(n) => n,
            _ => 0,
        })
        .sum();
    assert_eq!(http_denials, 1);
}

// ── Reachable handlers over a real tenant store ─────────────────────────────

/// Requests reach their handlers: the tenant store is real PostgreSQL with a
/// row for each test host, and protected HTTP uses a valid workflow path with
/// a NIP-98 proof, so the handler itself calls `admit_nip_fi_http_on_state`.
/// The first tenant-store connection a request takes can move the resolver
/// clock, which places a deadline crossing inside the tenant-lookup await.
mod postgres_tests {
    use super::*;

    /// Pod over a one-connection pool whose next acquire, once armed, moves
    /// the clock to the armed time.
    struct TenantPod {
        pod: Pod,
        clock: Arc<ManualClock>,
        advance_on_acquire: Arc<Mutex<Option<Duration>>>,
        label: String,
    }

    async fn tenant_pod(
        store: &Arc<Bindings>,
        clock: &Arc<ManualClock>,
        mode: NipFiMode,
    ) -> TenantPod {
        let label = format!("t{}", uuid::Uuid::new_v4().simple());
        let advance_on_acquire = Arc::new(Mutex::new(None::<Duration>));
        let (hook, hook_clock) = (Arc::clone(&advance_on_acquire), Arc::clone(clock));
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .before_acquire(move |_, _| {
                if let Some(at) = hook.lock().unwrap().take() {
                    hook_clock.set(at);
                }
                Box::pin(async { Ok(true) })
            })
            .connect(&crate::test_support::database_url())
            .await
            .expect("PostgreSQL must be available");
        // Seeding leaves the pool's only connection idle, so every request
        // acquire reuses it and runs the hook.
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(uuid::Uuid::new_v4())
            .bind(host(&label))
            .execute(&pool)
            .await
            .expect("seed community");
        store.bind(&label, &[ISS]);
        let base = crate::state::tests::test_state_with_database_pool(pool).await;
        let pod = pod_on(base, store, clock, mode).await;
        TenantPod {
            pod,
            clock: Arc::clone(clock),
            advance_on_acquire,
            label,
        }
    }

    fn nip98(keys: &nostr::Keys, url: &str) -> String {
        use base64::Engine as _;
        let event = nostr::EventBuilder::new(nostr::Kind::HttpAuth, "")
            .tags([
                nostr::Tag::parse(["u", url]).unwrap(),
                nostr::Tag::parse(["method", "GET"]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&event).unwrap())
        )
    }

    impl TenantPod {
        /// Each admission site's outcome for one fresh key: root and audio
        /// upgrades, then the workflow-runs handler with a matching proof.
        /// With `crossing = Some((start, at))` every request resolves at
        /// `start` and its tenant lookup moves the clock to `at`.
        async fn every_site(
            &self,
            crossing: Option<(Duration, Duration)>,
        ) -> Vec<(StatusCode, Vec<u8>)> {
            let raw_host = host(&self.label);
            let keys = nostr::Keys::generate();
            let token = mint_for(ISS, &raw_host, &keys.public_key());
            let scheme = if self.pod.state.config.relay_url.starts_with("wss://") {
                "https"
            } else {
                "http"
            };
            let workflow = format!("/workflows/{}/runs", uuid::Uuid::new_v4());
            let requests = [
                ("/".to_owned(), None),
                (format!("/huddle/{}/audio", uuid::Uuid::new_v4()), None),
                (
                    workflow.clone(),
                    Some(nip98(&keys, &format!("{scheme}://{raw_host}{workflow}"))),
                ),
            ];
            let mut outcomes = Vec::new();
            for (path, proof) in requests {
                if let Some((start, at)) = crossing {
                    self.clock.set(start);
                    *self.advance_on_acquire.lock().unwrap() = Some(at);
                }
                outcomes.push(
                    self.pod
                        .send_signed(&path, &raw_host, Some(&token), proof)
                        .await,
                );
            }
            outcomes
        }
    }

    /// Every site cleared NIP-FI and its handler went on past tenant
    /// binding: a oneshot request cannot upgrade, so root falls back to its
    /// NIP-11 document and audio refuses the upgrade, and the workflow handler
    /// passed admission to its rate limiter, whose Redis is unreachable.
    fn reached_handlers(outcomes: &[(StatusCode, Vec<u8>)]) -> bool {
        let body = |o: &(StatusCode, Vec<u8>)| String::from_utf8_lossy(&o.1).into_owned();
        match outcomes {
            [root, audio, http] => {
                root.0 == StatusCode::OK
                    && body(root).contains("\"federated_identity\"")
                    && audio.0 == StatusCode::UPGRADE_REQUIRED
                    && http.0 == StatusCode::SERVICE_UNAVAILABLE
                    && body(http).contains("shared admission unavailable")
            }
            _ => false,
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn every_reachable_handler_resolves_its_host_exactly_once() {
        let (store, clock) = setup();
        let p = tenant_pod(&store, &clock, NipFiMode::Enforce).await;
        // Absent from the published map, so every resolution is a counted miss.
        let raw_host = host(&p.label);
        store.unbind(&p.label);
        p.pod.resolver.refresh().await.expect("refresh");
        store.bind(&p.label, &[ISS]);

        let outcomes = p.every_site(None).await;
        assert!(reached_handlers(&outcomes), "{outcomes:?}");
        assert_eq!(store.lookups_for(&raw_host), 3, "one lookup per admission");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn snapshot_going_stale_during_the_tenant_lookup_admits_nowhere() {
        let (store, clock) = setup();
        let p = tenant_pod(&store, &clock, NipFiMode::Enforce).await;
        let inside = (secs(119.0), secs(119.9));
        assert!(
            reached_handlers(&p.every_site(Some(inside)).await),
            "control"
        );

        // Startup loaded at 0 s: fresh at resolution, stale at admission.
        let crossing = (secs(119.9), secs(120.1));
        assert_eq!(p.every_site(Some(crossing)).await, vec![unavailable(); 3]);
    }

    /// Each `buzz_nip_fi_shadow_total` series recorded while `run` ran, as
    /// sorted `route/stage/outcome` labels.
    async fn shadow_records<F: std::future::Future>(run: F) -> (F::Output, Vec<String>) {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let output = {
            let _guard = metrics::set_default_local_recorder(&recorder);
            run.await
        };
        let mut records: Vec<String> = recorder
            .snapshotter()
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(key, ..)| key.key().name() == "buzz_nip_fi_shadow_total")
            .map(|(key, ..)| {
                let label = |name: &str| {
                    let label = key.key().labels().find(|l| l.key() == name);
                    label.map(|l| l.value().to_owned()).unwrap_or_default()
                };
                format!("{}/{}/{}", label("route"), label("stage"), label("outcome"))
            })
            .collect();
        records.sort();
        (output, records)
    }

    // Shadow re-checks the binding's deadline where enforce does, after the
    // tenant lookup, and records enforce's refusal without changing any
    // response. Mutation: dropping the upgrade's observed deadline, or
    // resolving HTTP afresh instead of reusing the guard's binding, records
    // no `deadline` would-deny at that site.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn shadow_records_a_snapshot_going_stale_during_the_tenant_lookup() {
        let (store, clock) = setup();
        let p = tenant_pod(&store, &clock, NipFiMode::Shadow).await;
        let inside = (secs(119.0), secs(119.9));
        let (outcomes, records) = shadow_records(p.every_site(Some(inside))).await;
        assert!(reached_handlers(&outcomes), "control: {outcomes:?}");
        assert_eq!(records, ["http/admit/admit"], "control");

        let crossing = (secs(119.9), secs(120.1));
        let (outcomes, records) = shadow_records(p.every_site(Some(crossing))).await;
        assert!(reached_handlers(&outcomes), "{outcomes:?}");
        assert_eq!(
            records,
            [
                "audio/deadline/unavailable",
                "http/deadline/unavailable",
                "ws/deadline/unavailable"
            ]
        );
    }
}
