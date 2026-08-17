//! Concurrency-matrix tests for the Databricks auth coordinator.
//!
//! The coordinator single-flights OAuth acquisition per cache key. Within one
//! process, same-key callers coalesce on an in-memory `INFLIGHT` registry
//! *before* the file lock; across processes, they serialize on an OS advisory
//! lock and share success through the on-disk cache, with failures coalesced
//! through a durable cooldown sidecar. These tests drive the public API
//! (`acquire_with_intent`, `interactive_login`) with an injected
//! [`BrowserOpener`] that scripts the localhost callback instead of popping a
//! real window — the browser step becomes deterministic and countable.
//!
//! Two `PkceOAuthTokenSource` instances in ONE process do not model two
//! processes: the `INFLIGHT` registry intercepts them before the file lock, so
//! same-process tests exercise the in-memory single-flight, not the
//! cross-process protocol. The genuinely cross-process claims — lock
//! contention, crash release, cooldown sharing across a process boundary, and
//! one-grant/one-cache under a real race — are proved with the `lock-holder`
//! and `auth-worker` helper binaries, each a real second process on the same
//! lock file and cache. The lock-primitive and lock-timeout edges live in the
//! in-crate `auth::tests` module where the private helpers are reachable.

use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::Form;
use axum::{routing::get, routing::post, Json, Router};
use buzz_agent::auth::{
    AuthError, AuthIntent, BrowserOpener, PkceOAuthConfig, PkceOAuthTokenSource,
};
use serde::Deserialize;
use serde_json::json;
use tempfile::TempDir;

// ---- scripted browser opener --------------------------------------------

/// What the scripted "user" does when the coordinator opens a browser.
#[derive(Clone, Copy)]
enum Script {
    /// Redirect with a valid `code`+`state` → the flow exchanges it for a
    /// token and succeeds.
    Approve,
    /// Redirect with `error=access_denied` → the flow returns `Denied`.
    Deny,
    /// Every launch strategy fails → the flow returns `BrowserOpenFailed`
    /// without waiting on a listener nobody will reach.
    FailToOpen,
}

/// A [`BrowserOpener`] that counts launches and drives the localhost callback
/// on a background thread, so the caller's callback wait observes the redirect
/// exactly as a real browser would deliver it.
#[derive(Clone)]
struct ScriptedOpener {
    script: Script,
    calls: Arc<AtomicU64>,
}

impl ScriptedOpener {
    fn new(script: Script) -> Self {
        Self {
            script,
            calls: Arc::new(AtomicU64::new(0)),
        }
    }

    fn call_count(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl BrowserOpener for ScriptedOpener {
    fn open(&self, url: &str) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let query = match self.script {
            Script::FailToOpen => return Err("no browser available".into()),
            Script::Approve => "code=scripted-code",
            Script::Deny => "error=access_denied",
        };
        // Pull the loopback redirect target and the anti-CSRF state out of the
        // authorize URL, then fire the callback from a separate thread so this
        // synchronous `open()` returns and the flow proceeds to await it.
        let parsed = url::Url::parse(url).expect("authorize URL must parse");
        let redirect = parsed
            .query_pairs()
            .find(|(k, _)| k == "redirect_uri")
            .map(|(_, v)| v.into_owned())
            .expect("authorize URL carries redirect_uri");
        let state = parsed
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned())
            .expect("authorize URL carries state");
        let redirect = url::Url::parse(&redirect).expect("redirect_uri must parse");
        // The coordinator's listener binds 127.0.0.1; connect there directly so
        // the callback can't land on an IPv6 `localhost` (::1) with no listener.
        let port = redirect.port().expect("loopback redirect carries a port");
        // `state` is base64url (no reserved characters), safe to inline.
        let request = format!(
            "GET /?{query}&state={state} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        std::thread::spawn(move || {
            // A real browser holds the connection open until the callback page
            // responds; do the same so hyper dispatches the request before the
            // socket closes (a bare write+drop races the server and is lost).
            if let Ok(mut sock) = TcpStream::connect(("127.0.0.1", port)) {
                use std::io::Read;
                let _ = sock.write_all(request.as_bytes());
                let _ = sock.flush();
                let mut discard = Vec::new();
                let _ = sock.read_to_end(&mut discard);
            }
        });
        Ok(())
    }
}

// ---- stub OIDC provider --------------------------------------------------

#[derive(Deserialize)]
struct TokenForm {
    grant_type: String,
}

struct Stub {
    base: String,
    /// authorization-code exchanges served (browser flows completed).
    code_grants: Arc<AtomicU64>,
    /// refresh-token grants served.
    refresh_grants: Arc<AtomicU64>,
}

/// How the stub's token endpoint answers a `refresh_token` grant. Lets a test
/// distinguish the three ways a refresh can fail so it can assert the
/// coordinator classifies each correctly: a `401` is a real credential
/// rejection (dead refresh token), a `500` is a transient provider fault, and
/// a hang models a slow/unreachable provider that must trip the per-request
/// HTTP timeout. Authorization-code grants are never affected.
#[derive(Clone, Copy)]
enum RefreshMode {
    /// `200` with a fresh access token.
    Succeed,
    /// `401 invalid_grant` — the grant itself is rejected.
    Reject,
    /// `500` — a provider-side fault, transient rather than a credential
    /// decision.
    ServerError,
    /// A 4xx with the given OAuth `error` code in the body. Lets a test assert
    /// the coordinator treats `invalid_grant` (any 4xx) as a dead grant, but
    /// every other error code — and any non-`invalid_grant` status like `429`
    /// — as infrastructural rather than a credential rejection.
    ClientError(axum::http::StatusCode, &'static str),
    /// Sleep `d` before answering, so the caller's per-request HTTP timeout
    /// elapses first (a transport timeout, not a verdict from the provider).
    Hang(Duration),
}

/// Boot a stub provider. `reject_refresh` makes the token endpoint 401 every
/// refresh-token grant (a dead refresh token); authorization-code grants
/// always succeed with a fresh token.
async fn spawn_stub(reject_refresh: bool) -> Stub {
    spawn_stub_with(if reject_refresh {
        RefreshMode::Reject
    } else {
        RefreshMode::Succeed
    })
    .await
}

/// Boot a stub provider whose refresh-token grant follows `mode`. Discovery and
/// authorization-code grants always succeed instantly regardless of `mode`.
async fn spawn_stub_with(mode: RefreshMode) -> Stub {
    let code_grants = Arc::new(AtomicU64::new(0));
    let refresh_grants = Arc::new(AtomicU64::new(0));

    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let disco_base = base.clone();

    let discovery = move || {
        let base = disco_base.clone();
        async move {
            Json(json!({
                "authorization_endpoint": format!("{base}/authorize"),
                "token_endpoint": format!("{base}/token"),
            }))
        }
    };

    let code_for_token = code_grants.clone();
    let refresh_for_token = refresh_grants.clone();
    let app = Router::new()
        // Two discovery paths so distinct-host tests derive distinct cache
        // keys (the key hashes the discovery URL) from one stub.
        .route("/disco/a", get(discovery.clone()))
        .route("/disco/b", get(discovery))
        .route(
            "/token",
            post(move |Form(form): Form<TokenForm>| {
                let code_grants = code_for_token.clone();
                let refresh_grants = refresh_for_token.clone();
                let mode = mode;
                async move {
                    if form.grant_type == "refresh_token" {
                        let n = refresh_grants.fetch_add(1, Ordering::SeqCst) + 1;
                        // A hang delays the answer so the caller's per-request
                        // HTTP timeout can elapse first (transport timeout, not
                        // a credential decision).
                        if let RefreshMode::Hang(d) = mode {
                            tokio::time::sleep(d).await;
                        }
                        return match mode {
                            RefreshMode::Reject => (
                                axum::http::StatusCode::UNAUTHORIZED,
                                Json(json!({ "error": "invalid_grant" })),
                            ),
                            RefreshMode::ServerError => (
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                Json(json!({ "error": "temporarily_unavailable" })),
                            ),
                            RefreshMode::ClientError(status, error) => {
                                (status, Json(json!({ "error": error })))
                            }
                            RefreshMode::Succeed | RefreshMode::Hang(_) => (
                                axum::http::StatusCode::OK,
                                Json(json!({
                                    "access_token": format!("refreshed-token-{n}"),
                                    "refresh_token": "rotated-refresh",
                                    "expires_in": 3600,
                                })),
                            ),
                        };
                    }
                    let n = code_grants.fetch_add(1, Ordering::SeqCst) + 1;
                    (
                        axum::http::StatusCode::OK,
                        Json(json!({
                            "access_token": format!("browser-token-{n}"),
                            "refresh_token": "browser-refresh",
                            "expires_in": 3600,
                        })),
                    )
                }
            }),
        );

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Stub {
        base,
        code_grants,
        refresh_grants,
    }
}

fn config(stub: &Stub, disco_path: &str, cache_dir: &std::path::Path) -> PkceOAuthConfig {
    PkceOAuthConfig {
        discovery_url: format!("{}{disco_path}", stub.base),
        client_id: "test-client".into(),
        scopes: vec!["offline_access".into()],
        cache_namespace: "databricks".into(),
        cache_dir_override: Some(cache_dir.to_path_buf()),
    }
}

fn future_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600
}

fn cache_file_path(cfg: &PkceOAuthConfig, cache_dir: &std::path::Path) -> std::path::PathBuf {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(cfg.discovery_url.as_bytes());
    h.update(b"|");
    h.update(cfg.client_id.as_bytes());
    h.update(b"|");
    h.update(cfg.scopes.join(",").as_bytes());
    let hash = hex::encode(h.finalize());
    cache_dir
        .join(&cfg.cache_namespace)
        .join(format!("{hash}.json"))
}

/// The cross-process advisory lock path for a config, matching the
/// coordinator's `append_ext(cache_path, "lock")`. Used to point the
/// out-of-process lock-holder helper at the exact file the coordinator
/// contends on.
fn lock_file_path(cfg: &PkceOAuthConfig, cache_dir: &std::path::Path) -> std::path::PathBuf {
    let mut p = cache_file_path(cfg, cache_dir).into_os_string();
    p.push(".lock");
    p.into()
}

fn seed_cache(cfg: &PkceOAuthConfig, cache_dir: &std::path::Path, body: serde_json::Value) {
    let path = cache_file_path(cfg, cache_dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();
}

// ---- acceptance matrix ---------------------------------------------------

#[tokio::test]
async fn test_same_key_concurrent_callers_share_one_browser_attempt() {
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);

    // Two independent sources on the same key in ONE process. The in-memory
    // INFLIGHT registry coalesces them before the file lock, so this proves the
    // in-process single-flight — one leader runs the browser flow, the other
    // joins its published result. The genuine cross-process race is
    // `test_crossprocess_two_coordinators_race_to_one_grant_and_cache`.
    let a = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(opener.clone()),
    )
    .unwrap();
    let b = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(opener.clone()),
    )
    .unwrap();

    let (ra, rb) = tokio::join!(
        a.acquire_with_intent(AuthIntent::Auto, None),
        b.acquire_with_intent(AuthIntent::Auto, None),
    );
    let ta = ra.expect("first caller authenticates");
    let tb = rb.expect("second caller authenticates");

    // One browser launch, one code exchange, one shared token.
    assert_eq!(
        opener.call_count(),
        1,
        "only one browser attempt for one key"
    );
    assert_eq!(
        stub.code_grants.load(Ordering::SeqCst),
        1,
        "exactly one authorization-code exchange"
    );
    assert_eq!(ta, tb, "both callers observe the same token");
    assert_eq!(ta, "browser-token-1");
}

#[tokio::test]
async fn test_denied_then_auto_reads_cooldown_without_second_launch() {
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Deny);

    let src = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(opener.clone()),
    )
    .unwrap();

    let first = src.acquire_with_intent(AuthIntent::Auto, None).await;
    assert_eq!(
        first,
        Err(AuthError::Denied),
        "first Auto attempt is denied"
    );
    assert_eq!(opener.call_count(), 1);

    // The denial wrote a cooldown; a subsequent Auto caller reads it and
    // returns the recorded outcome instead of popping a second browser.
    let second = src.acquire_with_intent(AuthIntent::Auto, None).await;
    assert_eq!(
        second,
        Err(AuthError::Denied),
        "queued Auto caller honors the cooldown"
    );
    assert_eq!(
        opener.call_count(),
        1,
        "cooldown suppresses the second browser launch"
    );
}

#[tokio::test]
async fn test_userinitiated_retry_bypasses_cooldown_and_reopens() {
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();

    // First attempt: denied, writes a cooldown.
    let deny_opener = ScriptedOpener::new(Script::Deny);
    let denier = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(deny_opener.clone()),
    )
    .unwrap();
    assert_eq!(
        denier
            .acquire_with_intent(AuthIntent::UserInitiated, None)
            .await,
        Err(AuthError::Denied)
    );

    // The user explicitly retries: UserInitiated bypasses (and clears) the
    // cooldown and opens a fresh browser, which now succeeds.
    let approve_opener = ScriptedOpener::new(Script::Approve);
    let retrier = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(approve_opener.clone()),
    )
    .unwrap();
    let token = retrier
        .acquire_with_intent(AuthIntent::UserInitiated, None)
        .await
        .expect("explicit retry re-launches the browser and succeeds");
    assert_eq!(token, "browser-token-1");
    assert_eq!(
        approve_opener.call_count(),
        1,
        "UserInitiated retry launches despite the prior cooldown"
    );

    // Cooldown cleared on success: a follow-up Auto now sees a valid token,
    // never the stale denial.
    let auto = retrier.acquire_with_intent(AuthIntent::Auto, None).await;
    assert_eq!(auto, Ok("browser-token-1".to_string()));
}

#[tokio::test]
async fn test_distinct_hosts_do_not_inherit_cooldown() {
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();

    // Host A is denied and records a cooldown under key A.
    let deny_opener = ScriptedOpener::new(Script::Deny);
    let host_a = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(deny_opener.clone()),
    )
    .unwrap();
    assert_eq!(
        host_a.acquire_with_intent(AuthIntent::Auto, None).await,
        Err(AuthError::Denied)
    );

    // Host B is a different key (different discovery URL). It must NOT inherit
    // A's cooldown: an Auto caller launches its own browser and succeeds.
    let approve_opener = ScriptedOpener::new(Script::Approve);
    let host_b = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/b", cache.path()),
        Arc::new(approve_opener.clone()),
    )
    .unwrap();
    let token = host_b
        .acquire_with_intent(AuthIntent::Auto, None)
        .await
        .expect("distinct host is unaffected by another key's cooldown");
    assert_eq!(token, "browser-token-1");
    assert_eq!(approve_opener.call_count(), 1);
}

#[tokio::test]
async fn test_browser_open_failure_is_typed_and_retryable_by_user() {
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();

    // Every launch strategy fails: the flow reports the typed BrowserOpenFailed
    // without waiting on a listener nobody will reach.
    let fail_opener = ScriptedOpener::new(Script::FailToOpen);
    let failing = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(fail_opener.clone()),
    )
    .unwrap();
    let result = failing
        .acquire_with_intent(AuthIntent::UserInitiated, None)
        .await;
    assert_eq!(
        result,
        Err(AuthError::BrowserOpenFailed),
        "a failed launch surfaces as the typed BrowserOpenFailed"
    );
    assert_eq!(fail_opener.call_count(), 1);

    // A failed launch writes a cooldown, but a UserInitiated retry bypasses it
    // and reopens — a transient "no browser" (e.g. race with a display coming
    // up) must never wedge an explicit user sign-in.
    let approve_opener = ScriptedOpener::new(Script::Approve);
    let retrier = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(approve_opener.clone()),
    )
    .unwrap();
    let token = retrier
        .acquire_with_intent(AuthIntent::UserInitiated, None)
        .await
        .expect("explicit retry reopens despite the prior launch failure");
    assert_eq!(token, "browser-token-1");
    assert_eq!(approve_opener.call_count(), 1);
}

#[tokio::test]
async fn test_headless_dead_refresh_returns_refresh_rejected_without_browser() {
    let stub = spawn_stub(true).await; // refresh grants 401
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    // Expired token WITH a refresh token, but the server rejects the refresh
    // grant (dead/rotated). A Headless caller must classify this terminally as
    // RefreshRejected and never open a browser.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "stale",
            "refresh_token": "dead-refresh",
            "expires_at": 1u64,
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    let result = src.acquire_with_intent(AuthIntent::Headless, None).await;
    assert_eq!(
        result,
        Err(AuthError::RefreshRejected),
        "Headless dead-refresh is terminal RefreshRejected"
    );
    assert_eq!(opener.call_count(), 0, "Headless never opens a browser");
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        1,
        "the refresh grant was attempted exactly once"
    );
}

#[tokio::test]
async fn test_interactive_dead_refresh_converts_to_browser() {
    let stub = spawn_stub(true).await; // refresh grants 401
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    // Same dead-refresh seed, but an interactive intent must fall through to a
    // browser flow instead of failing terminally.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "stale",
            "refresh_token": "dead-refresh",
            "expires_at": 1u64,
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    let token = src
        .acquire_with_intent(AuthIntent::UserInitiated, None)
        .await
        .expect("interactive intent recovers via the browser");
    assert_eq!(token, "browser-token-1");
    assert_eq!(opener.call_count(), 1, "interactive intent opens a browser");
    assert_eq!(stub.refresh_grants.load(Ordering::SeqCst), 1);
    assert_eq!(stub.code_grants.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_headless_expired_token_live_refresh_recovers_silently() {
    let stub = spawn_stub(false).await; // refresh succeeds
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "stale",
            "refresh_token": "live-refresh",
            "expires_at": 1u64,
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    let token = src
        .acquire_with_intent(AuthIntent::Headless, None)
        .await
        .expect("live refresh recovers a Headless caller silently");
    assert_eq!(token, "refreshed-token-1");
    assert_eq!(opener.call_count(), 0, "no browser on a live refresh");
    assert_eq!(stub.refresh_grants.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_interactive_login_reuses_valid_cache_without_browser() {
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    // A still-valid cached token short-circuits interactive_login: an explicit
    // sign-in should not re-prompt when a good token is already present.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "already-valid",
            "refresh_token": "rt",
            "expires_at": future_secs(),
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    src.interactive_login()
        .await
        .expect("interactive_login succeeds off the valid cache");
    assert_eq!(
        opener.call_count(),
        0,
        "a valid cached token means no browser prompt"
    );
}

// ---- locally-fresh rejected bearer (401) recovery ------------------------
//
// The saved-model picker's recovery path: model discovery 401s a bearer that
// still looks locally fresh (its `expires_at` is in the future) and whose
// refresh grant is dead. Passing that exact token as `rejected` makes the
// clock untrustworthy, so the acquisition must not short-circuit on the fresh
// cache. `Auto` and `UserInitiated` then convert to a browser; `Headless`
// stays terminal with `RefreshRejected`. Seeding a *future*-expiry token is
// what distinguishes this from the expired-token refresh path.

/// Seed a not-yet-expired access token with a (dead) refresh token and return
/// the access token so the caller can pass it as `rejected`.
fn seed_fresh_rejectable(cfg: &PkceOAuthConfig, cache_dir: &std::path::Path) -> String {
    let access = "fresh-but-rejected";
    seed_cache(
        cfg,
        cache_dir,
        json!({
            "access_token": access,
            "refresh_token": "dead-refresh",
            "expires_at": future_secs(),
        }),
    );
    access.to_string()
}

#[tokio::test]
async fn test_auto_rejected_fresh_bearer_with_dead_refresh_launches_browser() {
    let stub = spawn_stub(true).await; // refresh grants 401
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());
    let rejected = seed_fresh_rejectable(&cfg, cache.path());

    // The token is locally fresh, so without `rejected` it would be a cache
    // hit and never reach the browser. Passing it as rejected forces the
    // clock-based hit to fail, the dead refresh to be attempted, and an Auto
    // caller to fall through to the browser.
    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    let token = src
        .acquire_with_intent(AuthIntent::Auto, Some(&rejected))
        .await
        .expect("Auto recovers a rejected-but-fresh bearer via the browser");
    assert_eq!(token, "browser-token-1");
    assert_eq!(opener.call_count(), 1, "Auto launches a browser to recover");
    assert_eq!(stub.refresh_grants.load(Ordering::SeqCst), 1);
    assert_eq!(stub.code_grants.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_headless_rejected_fresh_bearer_with_dead_refresh_returns_refresh_rejected() {
    let stub = spawn_stub(true).await; // refresh grants 401
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());
    let rejected = seed_fresh_rejectable(&cfg, cache.path());

    // Same locally-fresh rejected seed, but a Headless caller cannot open a
    // browser: a dead refresh is terminal RefreshRejected, never a launch.
    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    let result = src
        .acquire_with_intent(AuthIntent::Headless, Some(&rejected))
        .await;
    assert_eq!(
        result,
        Err(AuthError::RefreshRejected),
        "Headless dead-refresh on a rejected fresh bearer is terminal"
    );
    assert_eq!(opener.call_count(), 0, "Headless never opens a browser");
    assert_eq!(stub.refresh_grants.load(Ordering::SeqCst), 1);
}

// ---- refresh transport failures are not credential rejections ------------
//
// A refresh that never gets a verdict from the token endpoint — a per-request
// timeout, or a 5xx — is infrastructural, not a dead credential. It must
// surface as `NetworkUnavailable` and never pop a browser or return
// `RefreshRejected`, which would misreport a transient fault as a rotated
// token and (for interactive intents) prompt a needless sign-in.

#[tokio::test]
async fn test_refresh_timeout_is_network_unavailable_not_rejected() {
    // The token endpoint hangs far longer than the injected per-request HTTP
    // timeout, so the refresh call times out at the transport layer with no
    // verdict from the provider. A short real-time timeout is injected rather
    // than pausing the clock: under `start_paused` tokio auto-advances into
    // the timer while the real loopback discovery GET is still in flight, so
    // discovery — not the refresh — would trip the timeout, and the refresh
    // would never even be attempted. Real time keeps the timeout attached to
    // the request that actually hangs, which the `refresh_grants == 1` guard
    // below proves.
    let stub = spawn_stub_with(RefreshMode::Hang(Duration::from_secs(30))).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    // Expired token with a refresh token: the coordinator attempts the refresh,
    // which hangs past the HTTP timeout. A Headless caller must classify the
    // timeout as NetworkUnavailable, not RefreshRejected.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "stale",
            "refresh_token": "slow-refresh",
            "expires_at": 1u64,
        }),
    );

    let src = PkceOAuthTokenSource::new_with_http_timeout(
        cfg,
        Arc::new(opener.clone()),
        Duration::from_millis(300),
    )
    .unwrap();
    let result = src.acquire_with_intent(AuthIntent::Headless, None).await;
    assert_eq!(
        result,
        Err(AuthError::NetworkUnavailable),
        "a refresh transport timeout is infrastructural, not a rejection"
    );
    assert_eq!(
        opener.call_count(),
        0,
        "a timed-out refresh never becomes a credential decision"
    );
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        1,
        "the refresh was attempted exactly once before timing out"
    );
}

#[tokio::test]
async fn test_refresh_server_error_is_network_unavailable_not_rejected() {
    let stub = spawn_stub_with(RefreshMode::ServerError).await; // refresh 500s
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    // A 5xx is a provider-side fault, not a grant rejection: an interactive
    // intent must NOT pop a browser off it, and it must surface as
    // NetworkUnavailable rather than RefreshRejected.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "stale",
            "refresh_token": "server-error-refresh",
            "expires_at": 1u64,
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    let result = src
        .acquire_with_intent(AuthIntent::UserInitiated, None)
        .await;
    assert_eq!(
        result,
        Err(AuthError::NetworkUnavailable),
        "a refresh 5xx is transient, not a credential rejection"
    );
    assert_eq!(
        opener.call_count(),
        0,
        "a provider 5xx must not trigger an interactive browser fallback"
    );
    assert_eq!(stub.refresh_grants.load(Ordering::SeqCst), 1);
}

// ---- 4xx classification: only `invalid_grant` is a dead refresh token -----
//
// RFC 6749 §5.2 uses 400/401 token responses for several `error` codes, but
// only `invalid_grant` means the refresh token is dead. Every other 4xx —
// `invalid_request`, `invalid_client`, `unsupported_grant_type`,
// `invalid_scope`, `408`, `429` — is a request/config/transient fault a
// browser cannot repair, so it must stay infrastructural (`NetworkUnavailable`)
// and never pop a browser. The classifier keys on the OAuth error body, not
// the bare status class.

#[tokio::test]
async fn test_refresh_400_invalid_grant_is_dead_grant_not_network() {
    // A 400 (not just 401) carrying `invalid_grant` is still a dead refresh
    // token, so a Headless caller must classify it terminally as
    // RefreshRejected — proving the decision is the body error, not the status.
    let stub = spawn_stub_with(RefreshMode::ClientError(
        axum::http::StatusCode::BAD_REQUEST,
        "invalid_grant",
    ))
    .await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "stale",
            "refresh_token": "dead-refresh",
            "expires_at": 1u64,
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    let result = src.acquire_with_intent(AuthIntent::Headless, None).await;
    assert_eq!(
        result,
        Err(AuthError::RefreshRejected),
        "a 400 invalid_grant is a dead refresh token, not infrastructural"
    );
    assert_eq!(opener.call_count(), 0, "Headless never opens a browser");
    assert_eq!(stub.refresh_grants.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_refresh_non_invalid_grant_4xx_is_network_unavailable_not_rejected() {
    // Every 4xx whose OAuth body is NOT `invalid_grant` is a request/config or
    // transient fault a browser cannot repair, so it must surface as
    // NetworkUnavailable and never pop a browser — even for an interactive
    // intent that COULD. Two representative cases prove the classifier keys on
    // the body `error`, not the status class: a 400 `invalid_request`
    // (malformed/misconfigured) and a 429 `slow_down` (transient rate limit).
    for (status, error, refresh_token) in [
        (
            axum::http::StatusCode::BAD_REQUEST,
            "invalid_request",
            "misconfigured-refresh",
        ),
        (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "slow_down",
            "rate-limited-refresh",
        ),
    ] {
        let stub = spawn_stub_with(RefreshMode::ClientError(status, error)).await;
        let cache = TempDir::new().unwrap();
        let opener = ScriptedOpener::new(Script::Approve);
        let cfg = config(&stub, "/disco/a", cache.path());

        seed_cache(
            &cfg,
            cache.path(),
            json!({
                "access_token": "stale",
                "refresh_token": refresh_token,
                "expires_at": 1u64,
            }),
        );

        let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
        let result = src
            .acquire_with_intent(AuthIntent::UserInitiated, None)
            .await;
        assert_eq!(
            result,
            Err(AuthError::NetworkUnavailable),
            "a non-invalid_grant 4xx ({status} {error}) is infrastructural, not a credential rejection"
        );
        assert_eq!(
            opener.call_count(),
            0,
            "a browser cannot repair {error}, so none is opened"
        );
        assert_eq!(stub.refresh_grants.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn test_two_concurrent_userinitiated_denials_share_one_browser() {
    // Two UserInitiated callers arrive together on one key. The first is the
    // leader and opens the browser; the second is a pre-existing joiner that
    // must receive the leader's SAME Denied result rather than acquire the
    // lock afterward, clear the cooldown, and pop a second browser. This is
    // the failure-sharing that a lock-alone protocol loses.
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Deny);

    let a = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(opener.clone()),
    )
    .unwrap();
    let b = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(opener.clone()),
    )
    .unwrap();

    let (ra, rb) = tokio::join!(
        a.acquire_with_intent(AuthIntent::UserInitiated, None),
        b.acquire_with_intent(AuthIntent::UserInitiated, None),
    );
    assert_eq!(ra, Err(AuthError::Denied), "leader observes the denial");
    assert_eq!(
        rb,
        Err(AuthError::Denied),
        "the joiner shares the leader's denial, not a fresh attempt"
    );
    assert_eq!(
        opener.call_count(),
        1,
        "one browser launch shared across both concurrent UserInitiated callers"
    );
}

// ---- genuine cross-process lock contention and crash release -------------
//
// The single-flight guarantee and its crash-release property are cross-process
// claims, so they need a real second process — not a second in-process handle —
// on the same lock file. The `lock-holder` helper binary takes the
// coordinator's advisory lock and holds it until killed; killing it models a
// crash mid-flow, and the kernel's release of the advisory lock is what lets
// the coordinator's successor proceed with no PID files and no lock breaking.

#[tokio::test]
async fn test_crossprocess_lock_holder_blocks_then_crash_release_lets_successor_proceed() {
    let stub = spawn_stub(false).await; // refresh succeeds once the lock is free
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    // Expired token with a LIVE refresh: a cache miss forces the coordinator
    // onto the slow path (it must take the lock), and once the lock is free the
    // refresh recovers a token without any browser — so success is a clean
    // signal that the successor proceeded.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "stale",
            "refresh_token": "live-refresh",
            "expires_at": 1u64,
        }),
    );

    let lock_path = lock_file_path(&cfg, cache.path());
    let ready_marker = cache.path().join("holder.ready");

    // A real second process grabs the lock and holds it.
    let mut holder = tokio::process::Command::new(env!("CARGO_BIN_EXE_lock-holder"))
        .env("LOCK_HELPER_PATH", &lock_path)
        .env("LOCK_HELPER_READY", &ready_marker)
        .kill_on_drop(true)
        .spawn()
        .expect("spawn the lock-holder helper process");

    // Synchronize on real lock ownership before racing the coordinator.
    for _ in 0..600 {
        if ready_marker.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        ready_marker.exists(),
        "lock-holder never signaled that it holds the lock"
    );

    // The coordinator cannot make progress while another process holds the
    // lock: it polls the advisory lock rather than stealing it.
    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    let task =
        tokio::spawn(async move { src.acquire_with_intent(AuthIntent::Headless, None).await });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !task.is_finished(),
        "coordinator must block while a live process holds the cross-process lock"
    );

    // Kill the holder: the kernel releases the advisory lock on process death,
    // with no PID file inspection or lock breaking on our side.
    holder.kill().await.expect("kill the lock holder");
    holder.wait().await.ok();

    let token = task
        .await
        .expect("acquisition task joins")
        .expect("successor proceeds once the crashed holder's lock is released");
    assert_eq!(
        token, "refreshed-token-1",
        "successor completes the refresh after acquiring the freed lock"
    );
    assert_eq!(
        opener.call_count(),
        0,
        "Headless successor recovers via refresh without a browser"
    );
}

// ---- genuine cross-process coordinator races -----------------------------
//
// The `auth-worker` helper is a real second process running the PUBLIC
// coordinator API against the shared cache. Unlike two in-process handles
// (which the `INFLIGHT` registry coalesces before the file lock), these
// workers contend on the OS advisory lock and share success through the
// on-disk cache exactly as two Buzz processes on one machine would.

/// A spawned `auth-worker`: its child handle plus the file it writes its JSON
/// outcome to.
struct Worker {
    child: tokio::process::Child,
    result_path: std::path::PathBuf,
}

#[derive(Deserialize)]
struct WorkerOutcome {
    result: String,
    bearer: Option<String>,
    launches: u64,
}

impl Worker {
    /// Block until the worker exits, then parse its outcome file.
    async fn join(mut self) -> WorkerOutcome {
        let status = self.child.wait().await.expect("auth-worker joins");
        assert!(
            status.success(),
            "auth-worker exited with failure: {status}"
        );
        let body = std::fs::read(&self.result_path).expect("auth-worker wrote its outcome");
        serde_json::from_slice(&body).expect("auth-worker outcome parses")
    }
}

/// Spawn an `auth-worker` child against `cfg`'s shared cache. `extra` sets the
/// optional barrier-marker env vars ((name, path) pairs) a scenario needs to
/// order events across processes.
fn spawn_worker(
    cfg: &PkceOAuthConfig,
    cache_dir: &std::path::Path,
    intent: &str,
    script: &str,
    tag: &str,
    extra: &[(&str, &std::path::Path)],
) -> Worker {
    let result_path = cache_dir.join(format!("{tag}.result.json"));
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_auth-worker"));
    cmd.env("AUTH_WORKER_DISCOVERY_URL", &cfg.discovery_url)
        .env("AUTH_WORKER_CACHE_DIR", cache_dir)
        .env("AUTH_WORKER_NAMESPACE", &cfg.cache_namespace)
        .env("AUTH_WORKER_CLIENT_ID", &cfg.client_id)
        .env("AUTH_WORKER_SCOPES", cfg.scopes.join(","))
        .env("AUTH_WORKER_INTENT", intent)
        .env("AUTH_WORKER_SCRIPT", script)
        .env("AUTH_WORKER_RESULT", &result_path)
        .kill_on_drop(true);
    for (key, path) in extra {
        cmd.env(key, path);
    }
    let child = cmd.spawn().expect("spawn the auth-worker helper process");
    Worker { child, result_path }
}

async fn wait_for_marker(path: &std::path::Path, what: &str) {
    for _ in 0..1000 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what} ({})", path.display());
}

#[tokio::test]
async fn test_crossprocess_userinitiated_denial_shared_with_waiting_auto() {
    // Two real processes on one key. The child runs a UserInitiated flow that
    // is denied; while it holds the lock and its browser is open, the parent's
    // Auto coordinator is already WAITING on the cross-process lock. The child
    // must be released only once the parent is queued, so the denial the child
    // records is what the waiting Auto observes — one launch total, durable
    // Denied for both, across a genuine process boundary.
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();
    let cfg = config(&stub, "/disco/a", cache.path());

    let launched = cache.path().join("child.launched");
    let proceed = cache.path().join("child.proceed");
    let child = spawn_worker(
        &cfg,
        cache.path(),
        "userinitiated",
        "deny",
        "denier",
        &[
            ("AUTH_WORKER_LAUNCHED_MARKER", launched.as_path()),
            ("AUTH_WORKER_PROCEED_MARKER", proceed.as_path()),
        ],
    );

    // Wait until the child holds the lock and has opened its (scripted)
    // browser; its callback is withheld until we create `proceed`.
    wait_for_marker(&launched, "child browser launch").await;

    // The parent's Auto coordinator now contends for the same lock. It cannot
    // proceed while the child holds it, so it is a genuine cross-process
    // waiter.
    let parent = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(ScriptedOpener::new(Script::Approve)),
    )
    .unwrap();
    let auto =
        tokio::spawn(async move { parent.acquire_with_intent(AuthIntent::Auto, None).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !auto.is_finished(),
        "parent Auto must block while the child process holds the lock"
    );

    // Release the child's callback: it finishes the denial and writes the
    // cooldown sidecar, then drops the lock.
    std::fs::write(&proceed, b"go").unwrap();

    let child_outcome = child.join().await;
    assert_eq!(
        child_outcome.result, "denied",
        "child UserInitiated is denied"
    );
    assert_eq!(child_outcome.launches, 1, "child opens exactly one browser");

    let auto_result = auto.await.expect("parent Auto task joins");
    assert_eq!(
        auto_result,
        Err(AuthError::Denied),
        "the already-waiting Auto reads the child's durable denial"
    );
    assert_eq!(
        stub.code_grants.load(Ordering::SeqCst),
        0,
        "a denied flow never reaches the code exchange"
    );
}

#[tokio::test]
async fn test_crossprocess_two_coordinators_race_to_one_grant_and_cache() {
    // Two real coordinator processes race on one key from a cold cache. They
    // are released together (via a shared start marker) so both contend for the
    // lock. Exactly one wins the browser flow and performs the single code
    // grant; the other serializes behind the lock and adopts the winner's token
    // from the shared cache. Both must observe the same bearer, and the private
    // cache must hold exactly one parseable token artifact.
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();
    let cfg = config(&stub, "/disco/a", cache.path());

    let ready_a = cache.path().join("a.ready");
    let ready_b = cache.path().join("b.ready");
    let start = cache.path().join("start");

    let worker_a = spawn_worker(
        &cfg,
        cache.path(),
        "userinitiated",
        "approve",
        "a",
        &[
            ("AUTH_WORKER_READY_MARKER", ready_a.as_path()),
            ("AUTH_WORKER_START_MARKER", start.as_path()),
        ],
    );
    let worker_b = spawn_worker(
        &cfg,
        cache.path(),
        "userinitiated",
        "approve",
        "b",
        &[
            ("AUTH_WORKER_READY_MARKER", ready_b.as_path()),
            ("AUTH_WORKER_START_MARKER", start.as_path()),
        ],
    );

    // Both processes are built and about to acquire; release them together.
    wait_for_marker(&ready_a, "worker A ready").await;
    wait_for_marker(&ready_b, "worker B ready").await;
    std::fs::write(&start, b"go").unwrap();

    let (out_a, out_b) = tokio::join!(worker_a.join(), worker_b.join());
    assert_eq!(out_a.result, "ok", "worker A authenticates");
    assert_eq!(out_b.result, "ok", "worker B authenticates");
    let bearer_a = out_a.bearer.expect("worker A returns a bearer");
    let bearer_b = out_b.bearer.expect("worker B returns a bearer");
    assert_eq!(
        bearer_a, bearer_b,
        "both processes observe the same bearer from the shared cache"
    );

    // Exactly one browser launch and one code exchange across both processes.
    assert_eq!(
        out_a.launches + out_b.launches,
        1,
        "exactly one browser launch across the two coordinator processes"
    );
    assert_eq!(
        stub.code_grants.load(Ordering::SeqCst),
        1,
        "exactly one authorization-code exchange across both processes"
    );

    // The private cache holds exactly one parseable token artifact carrying the
    // shared bearer.
    let cache_path = cache_file_path(&cfg, cache.path());
    let raw = std::fs::read(&cache_path).expect("cache file exists");
    let cached: serde_json::Value =
        serde_json::from_slice(&raw).expect("cache holds one parseable token artifact");
    assert_eq!(
        cached.get("access_token").and_then(|v| v.as_str()),
        Some(bearer_a.as_str()),
        "the cached token is the shared bearer"
    );
}
