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
    ///
    /// Used only by Unix-only tests (refresh-error classification).
    /// Gated to suppress dead-code warnings on Windows.
    #[cfg(unix)]
    ServerError,
    /// A 4xx with the given OAuth `error` code in the body. Lets a test assert
    /// the coordinator treats `invalid_grant` (any 4xx) as a dead grant, but
    /// every other error code — and any non-`invalid_grant` status like `429`
    /// — as infrastructural rather than a credential rejection.
    ///
    /// Used only by Unix-only tests (refresh-error classification).
    /// Gated to suppress dead-code warnings on Windows.
    #[cfg(unix)]
    ClientError(axum::http::StatusCode, &'static str),
    /// Sleep `d` before answering, so the caller's per-request HTTP timeout
    /// elapses first (a transport timeout, not a verdict from the provider).
    ///
    /// Used only by Unix-only tests (refresh-timeout classification).
    /// Gated to suppress dead-code warnings on Windows.
    #[cfg(unix)]
    Hang(Duration),
    /// `200` returning the same fixed access token on every grant, regardless
    /// of how many are served. Models a provider that re-issues an identical
    /// access token, so a bounded rerun can hand back the exact bytes the
    /// caller already reported 401-rejected.
    ///
    /// Used only by Unix-only tests (rejected-token neutralization, sticky
    /// reissuance). Gated to suppress dead-code warnings on Windows.
    #[cfg(unix)]
    SucceedSticky(&'static str),
}

/// How the stub's token endpoint answers an `authorization_code` grant (the
/// browser code exchange). Lets a test drive the exchange classifier: a
/// `401 invalid_grant` is a genuine rejected code (`ExchangeFailed`), while a
/// `429`, a `500`, and a malformed `200` are transient/provider faults that
/// must classify as `NetworkUnavailable` rather than poisoning the cooldown.
#[derive(Clone, Copy)]
enum ExchangeMode {
    /// `200` with a fresh access token — the browser flow completes.
    Succeed,
    /// A failing status carrying the given OAuth `error` body. Only a 4xx
    /// `invalid_grant` is a true code rejection; every other status/error is
    /// infrastructural.
    Fail(axum::http::StatusCode, &'static str),
    /// `200` whose body lacks an `access_token` — a malformed success the
    /// provider should never send, so it is a fault, not a rejected code.
    MalformedSuccess,
    /// Sleep `d` before answering, so the caller's per-request HTTP timeout
    /// elapses first (a transport timeout, not a verdict from the provider).
    Hang(Duration),
    /// `200` returning the same fixed access token on every authorization-code
    /// exchange. Models a provider that re-issues an identical access token, so
    /// a browser sign-in (reached after a dead refresh) can hand back the exact
    /// bytes the caller reported 401-rejected.
    ///
    /// Used only by Unix-only tests (sticky browser exchange after dead refresh).
    /// Gated to suppress dead-code warnings on Windows.
    #[cfg(unix)]
    SucceedSticky(&'static str),
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
    spawn_stub_with_modes(mode, ExchangeMode::Succeed).await
}

/// Boot a stub whose authorization-code exchange follows `exchange`. Refresh
/// grants succeed; used by the exchange-classifier tests.
async fn spawn_stub_with_exchange(exchange: ExchangeMode) -> Stub {
    spawn_stub_with_modes(RefreshMode::Succeed, exchange).await
}

/// Boot a stub provider whose refresh-token grant follows `refresh` and whose
/// authorization-code grant follows `exchange`. Discovery always succeeds.
async fn spawn_stub_with_modes(refresh: RefreshMode, exchange: ExchangeMode) -> Stub {
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
                let refresh = refresh;
                let exchange = exchange;
                async move {
                    if form.grant_type == "refresh_token" {
                        let n = refresh_grants.fetch_add(1, Ordering::SeqCst) + 1;
                        // A hang delays the answer so the caller's per-request
                        // HTTP timeout can elapse first (transport timeout, not
                        // a credential decision).
                        #[cfg(unix)]
                        if let RefreshMode::Hang(d) = refresh {
                            tokio::time::sleep(d).await;
                        }
                        return match refresh {
                            RefreshMode::Reject => (
                                axum::http::StatusCode::UNAUTHORIZED,
                                Json(json!({ "error": "invalid_grant" })),
                            ),
                            #[cfg(unix)]
                            RefreshMode::ServerError => (
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                Json(json!({ "error": "temporarily_unavailable" })),
                            ),
                            #[cfg(unix)]
                            RefreshMode::ClientError(status, error) => {
                                (status, Json(json!({ "error": error })))
                            }
                            RefreshMode::Succeed => (
                                axum::http::StatusCode::OK,
                                Json(json!({
                                    "access_token": format!("refreshed-token-{n}"),
                                    "refresh_token": "rotated-refresh",
                                    "expires_in": 3600,
                                })),
                            ),
                            #[cfg(unix)]
                            RefreshMode::Hang(_) => (
                                axum::http::StatusCode::OK,
                                Json(json!({
                                    "access_token": format!("refreshed-token-{n}"),
                                    "refresh_token": "rotated-refresh",
                                    "expires_in": 3600,
                                })),
                            ),
                            #[cfg(unix)]
                            RefreshMode::SucceedSticky(tok) => (
                                axum::http::StatusCode::OK,
                                Json(json!({
                                    "access_token": tok,
                                    "refresh_token": "rotated-refresh",
                                    "expires_in": 3600,
                                })),
                            ),
                        };
                    }
                    let n = code_grants.fetch_add(1, Ordering::SeqCst) + 1;
                    // A hang delays the answer so the caller's per-request HTTP
                    // timeout can elapse first (transport timeout, not a code
                    // decision), mirroring the refresh path above.
                    if let ExchangeMode::Hang(d) = exchange {
                        tokio::time::sleep(d).await;
                    }
                    match exchange {
                        ExchangeMode::Succeed => (
                            axum::http::StatusCode::OK,
                            Json(json!({
                                "access_token": format!("browser-token-{n}"),
                                "refresh_token": "browser-refresh",
                                "expires_in": 3600,
                            })),
                        ),
                        ExchangeMode::Fail(status, error) => {
                            (status, Json(json!({ "error": error })))
                        }
                        ExchangeMode::MalformedSuccess => (
                            axum::http::StatusCode::OK,
                            Json(json!({ "token_type": "bearer" })),
                        ),
                        #[cfg(unix)]
                        ExchangeMode::SucceedSticky(tok) => (
                            axum::http::StatusCode::OK,
                            Json(json!({
                                "access_token": tok,
                                "refresh_token": "browser-refresh",
                                "expires_in": 3600,
                            })),
                        ),
                        // Reached only after the sleep above; answer as a
                        // success the caller has already abandoned.
                        ExchangeMode::Hang(_) => (
                            axum::http::StatusCode::OK,
                            Json(json!({
                                "access_token": format!("browser-token-{n}"),
                                "refresh_token": "browser-refresh",
                                "expires_in": 3600,
                            })),
                        ),
                    }
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

/// Control handle for a stub whose refresh response is held until the parent
/// explicitly releases it. Used by the cross-process digest test to establish
/// deterministic ordering: the parent waits for `request_received` (proves A
/// holds the lock and is mid-refresh), then spawns B, waits for B's snapshot
/// marker, and finally calls `release()` before joining both workers.
#[cfg(unix)]
struct RefreshGate {
    /// Notified by the stub once it has received the first refresh request.
    request_received: Arc<tokio::sync::Notify>,
    /// Parent signals this to let the stub return the response.
    proceed: Arc<tokio::sync::Notify>,
}

#[cfg(unix)]
impl RefreshGate {
    /// Asynchronously wait until the stub has received A's refresh request.
    async fn wait_for_request(&self) {
        self.request_received.notified().await;
    }

    /// Release the held refresh response so the stub replies to A.
    fn release(&self) {
        self.proceed.notify_one();
    }
}

/// Shape of the refresh response returned by [`spawn_stub_with_held_refresh`].
///
/// - `Sticky(tok)` — every refresh returns `200 OK` with `access_token: tok`.
/// - `Reject` — every refresh returns `401 Unauthorized` with `invalid_grant`.
#[cfg(unix)]
enum HeldRefreshResponse {
    Sticky(&'static str),
    Reject,
}

/// Spawn a stub that holds the FIRST refresh request until the parent calls
/// [`RefreshGate::release()`], then replies according to `response`.
/// Subsequent refresh requests skip the gate and reply immediately with the
/// same shape. Code-grant (`authorization_code`) requests are always answered
/// immediately with a fresh browser token.
///
/// Returns the stub (for `refresh_grants` / `code_grants` assertions) and the
/// control gate. Used by the cross-process held-refresh tests.
#[cfg(unix)]
async fn spawn_stub_with_held_refresh(response: HeldRefreshResponse) -> (Stub, RefreshGate) {
    let code_grants = Arc::new(AtomicU64::new(0));
    let refresh_grants = Arc::new(AtomicU64::new(0));
    let request_received = Arc::new(tokio::sync::Notify::new());
    let proceed = Arc::new(tokio::sync::Notify::new());

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
    let received_for_handler = request_received.clone();
    let proceed_for_handler = proceed.clone();
    // Track whether the first refresh has been released yet. Once the first
    // grant is released, subsequent grants return immediately.
    let first_released = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reject = matches!(response, HeldRefreshResponse::Reject);
    let sticky_tok = match response {
        HeldRefreshResponse::Sticky(tok) => tok,
        HeldRefreshResponse::Reject => "",
    };

    let app = Router::new()
        .route("/disco/a", get(discovery.clone()))
        .route("/disco/b", get(discovery))
        .route(
            "/token",
            post(move |Form(form): Form<TokenForm>| {
                let code_grants = code_for_token.clone();
                let refresh_grants = refresh_for_token.clone();
                let received = received_for_handler.clone();
                let proceed = proceed_for_handler.clone();
                let first_released = first_released.clone();
                async move {
                    if form.grant_type == "refresh_token" {
                        refresh_grants.fetch_add(1, Ordering::SeqCst);
                        // Hold only the first refresh request; once released,
                        // all subsequent requests return immediately.
                        if !first_released.swap(true, Ordering::SeqCst) {
                            received.notify_one();
                            proceed.notified().await;
                        }
                        return if reject {
                            (
                                axum::http::StatusCode::UNAUTHORIZED,
                                Json(json!({ "error": "invalid_grant" })),
                            )
                        } else {
                            (
                                axum::http::StatusCode::OK,
                                Json(json!({
                                    "access_token": sticky_tok,
                                    "refresh_token": "rotated-refresh",
                                    "expires_in": 3600,
                                })),
                            )
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

    let stub = Stub {
        base,
        code_grants,
        refresh_grants,
    };
    let gate = RefreshGate {
        request_received,
        proceed,
    };
    (stub, gate)
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

/// The cross-process attempt sidecar path for a config, matching the
/// coordinator's `append_ext(cache_path, "attempt")`. Used by tests that
/// inspect the generation counter directly after a cross-process adoption to
/// verify the adopter did not re-write a new generation.
fn attempt_sidecar_path(cfg: &PkceOAuthConfig, cache_dir: &std::path::Path) -> std::path::PathBuf {
    let mut p = cache_file_path(cfg, cache_dir).into_os_string();
    p.push(".attempt");
    p.into()
}

/// The cross-process advisory lock path for a config, matching the
/// coordinator's `append_ext(cache_path, "lock")`. Used to point the
/// out-of-process lock-holder helper at the exact file the coordinator
/// contends on.
#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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
#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

// ---- mixed-intent coalescing must not leak an Auto cooldown to a user -----
//
// `Auto` and `UserInitiated` disagree on cooldown policy: `Auto` honors a
// recorded cooldown and returns its `Denied`/`TimedOut` without a browser,
// while `UserInitiated` bypasses the cooldown and opens a fresh sign-in. If
// both coalesced onto one in-process slot, a user's explicit action arriving
// behind an `Auto` leader would inherit the leader's suppressed result and
// silently get *nothing* — no browser, no bypass. Keying the single-flight
// slot by the full intent keeps the two from sharing a slot.

#[tokio::test]
async fn test_userinitiated_joiner_does_not_inherit_auto_cooldown_result() {
    // Race an Auto caller and a UserInitiated caller on one key. `join!` polls
    // the Auto future first: it becomes the in-process leader, takes the file
    // lock, and opens a browser that is DENIED — and it yields on the callback
    // wait while still holding the lock and its INFLIGHT slot. The
    // UserInitiated caller is then polled *while the Auto attempt is in flight*.
    //
    // Before the fix, both intents keyed the single-flight slot by browser
    // capability alone, so the UserInitiated caller joined the Auto leader's
    // slot and inherited its `Denied` — never opening its own browser, never
    // getting the cooldown bypass it promises. Keying by the full intent keeps
    // them apart: the UserInitiated caller runs its own flow, bypasses the
    // cooldown the Auto denial recorded, and signs in on its own browser.
    //
    // Distinct openers make the coalescing visible: if the UserInitiated caller
    // had inherited the Auto result, its `approve` opener would never fire.
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();
    let deny = ScriptedOpener::new(Script::Deny);
    let approve = ScriptedOpener::new(Script::Approve);

    let auto = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(deny.clone()),
    )
    .unwrap();
    let user = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(approve.clone()),
    )
    .unwrap();

    let (auto_res, user_res) = tokio::join!(
        auto.acquire_with_intent(AuthIntent::Auto, None),
        user.acquire_with_intent(AuthIntent::UserInitiated, None),
    );
    assert_eq!(
        auto_res,
        Err(AuthError::Denied),
        "the Auto leader observes its own browser denial"
    );
    let bearer =
        user_res.expect("the UserInitiated caller runs its own sign-in, not the Auto slot");
    assert!(
        bearer.starts_with("browser-token-"),
        "UserInitiated got a fresh browser token, not the Auto leader's Denied: {bearer}"
    );
    assert_eq!(
        deny.call_count(),
        1,
        "the Auto leader opened exactly one (denied) browser"
    );
    assert_eq!(
        approve.call_count(),
        1,
        "the UserInitiated caller opened its own browser instead of inheriting the Auto denial"
    );
}

// ---- a joiner must never inherit its own rejected token -------------------
//
// The in-process slot is keyed by (lock path, intent) only, so a 401-recovery
// joiner shares a leader that ran with a *different* `rejected` value. If the
// leader publishes a token equal to THIS caller's rejected bytes — e.g. its
// refresh produced exactly the generation the joiner just reported 401 — the
// joiner would retry the provider with the credentials it already knows are
// dead. The joiner must instead detect the collision and run its own bounded
// acquisition, obtaining a token that differs from its `rejected`.

#[cfg(unix)]
#[tokio::test]
async fn test_joiner_never_receives_its_own_rejected_token() {
    // Two concurrent `Headless` 401-recovery callers on one key, each rejecting
    // a DIFFERENT bearer. The seeded cache token is expired, so neither caller
    // is satisfied by the fast path (or the under-lock re-read) — both must go
    // to the live refresh grant, which is what makes the leader slow enough to
    // join. `join!` polls A first: it registers the INFLIGHT slot as leader,
    // takes the file lock, and yields on its refresh HTTP call while holding
    // the slot. B is then polled *while A is in flight* and joins A's slot.
    //
    // A's refresh yields `refreshed-token-1` and saves it. That is exactly the
    // bearer B passed as `rejected` (B held gen-1 and was 401'd on it). Before
    // the fix, B — a joiner keyed only by intent — received A's published
    // `refreshed-token-1`: the precise bytes it just reported rejected. The fix
    // makes B detect `published == own rejected`, fall through to its own
    // acquisition, and refresh again to `refreshed-token-2`. The rerun goes
    // straight to the leader body (not back through the registry), and its
    // under-lock re-read rejects A's freshly-saved gen-1 (it equals B's
    // `rejected`), so B can neither re-join the dead generation's slot, adopt
    // its own rejected bytes from disk, nor loop.
    let stub = spawn_stub(false).await; // refresh always succeeds
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    // Expired access token with a live refresh token: the expiry forces both
    // callers past the cache into the refresh grant regardless of their
    // distinct `rejected` values.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "expired-seed",
            "refresh_token": "live-refresh",
            "expires_at": 1u64,
        }),
    );

    let a = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();
    let b = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();

    let (ra, rb) = tokio::join!(
        a.acquire_with_intent(AuthIntent::Headless, Some("rejected-by-a")),
        b.acquire_with_intent(AuthIntent::Headless, Some("refreshed-token-1")),
    );

    assert_eq!(
        ra,
        Ok("refreshed-token-1".to_string()),
        "the leader refreshes to gen-1, which differs from its own rejected value"
    );
    let b_token = rb.expect("the joiner runs its own acquisition instead of inheriting gen-1");
    assert_ne!(
        b_token, "refreshed-token-1",
        "the joiner must never receive the exact bytes it reported 401-rejected"
    );
    assert_eq!(
        b_token, "refreshed-token-2",
        "the joiner refreshed once more to a token that differs from its rejected value"
    );
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        2,
        "exactly two refreshes: the leader's, then the joiner's single bounded rerun — no loop"
    );
    assert_eq!(
        opener.call_count(),
        0,
        "a live refresh recovers both callers without any browser"
    );
}

// ---- a bounded rerun that re-issues the rejected bytes must fail typed -----
//
// The joiner-collision fix reruns its own bounded acquisition when the leader
// publishes the joiner's own rejected token. That rerun is only safe if it,
// too, refuses to hand back the rejected bytes: a provider that re-issues an
// identical access token on refresh would otherwise let the exact 401'd
// credential escape through the rerun. The coordinator guards the refresh
// success at the persistence boundary (`finish`), so both a plain leader and
// this rerun terminate with a typed auth error before caching the rejected
// token rather than returning it.

#[cfg(unix)]
#[tokio::test]
async fn test_joiner_rerun_reissuing_rejected_token_fails_typed_not_loop() {
    // A sticky provider returns ONE fixed access token on every refresh. Leader
    // A rejects a different value, so its refresh to the sticky token is a
    // clean success it publishes and caches. Joiner B rejected exactly the
    // sticky token: it collides with A's published result, reruns its own
    // bounded acquisition, and that rerun's refresh hands back the sticky token
    // again — B's own rejected bytes. The persistence-boundary guard turns that
    // into a terminal `RefreshRejected` (Headless, no browser) instead of
    // returning the dead credential or looping.
    let stub = spawn_stub_with(RefreshMode::SucceedSticky("sticky-token")).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "expired-seed",
            "refresh_token": "live-refresh",
            "expires_at": 1u64,
        }),
    );

    let a = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();
    let b = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();

    let (ra, rb) = tokio::join!(
        a.acquire_with_intent(AuthIntent::Headless, Some("rejected-by-a")),
        b.acquire_with_intent(AuthIntent::Headless, Some("sticky-token")),
    );

    assert_eq!(
        ra,
        Ok("sticky-token".to_string()),
        "the leader's refresh yields the sticky token, which differs from its own rejected value"
    );
    assert_eq!(
        rb,
        Err(AuthError::RefreshRejected),
        "the joiner's rerun re-issued its own rejected bytes and must fail typed, not return them"
    );
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        2,
        "exactly two refreshes: the leader's, then the joiner's single bounded rerun — no loop"
    );
    assert_eq!(
        opener.call_count(),
        0,
        "a headless collision never opens a browser"
    );
}

// ---- a joiner with a DIFFERENT rejected must not inherit a rejection-relative failure ---
//
// When a leader A rejects token X (its own `rejected`) and the refresh yields
// X again — causing `finish()` to return `RefreshRejected` — that failure is
// scoped to A's specific rejected token. A joiner B waiting on the same slot
// with a *different* rejected token Y must NOT adopt that failure: the refresh
// grant of X is a perfectly valid token for B (B only rejected Y). The slot
// publishes A's rejected-token digest; B detects the mismatch and reruns its
// own `acquire_leader` — which finds X already in the cache from A's successful
// write (X was issued but not cached because A had it as `rejected`, but in
// Carl's scenario there was NO prior good token — the refresh just minted X
// which IS good for B), and returns it.
//
// Concrete scenario: A rejected X, refresh re-issues X → A gets RefreshRejected.
// B rejected Y (different), refresh would yield X for B → B succeeds.

#[cfg(unix)]
#[tokio::test]
async fn test_joiner_with_different_rejected_does_not_inherit_leaders_rejection_failure() {
    // Sticky provider always returns "X" on every refresh grant.
    let stub = spawn_stub_with(RefreshMode::SucceedSticky("X")).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "expired-seed",
            "refresh_token": "live-refresh",
            "expires_at": 1u64,
        }),
    );

    // A rejected "X" (same as what the provider always issues). The refresh
    // re-issues "X", `finish()` returns RefreshRejected — the failure is
    // rejection-relative to A's own rejected bytes.
    //
    // B rejected "Y" (different). It should NOT inherit A's RefreshRejected:
    // the provider can give B "X", which is valid for B.
    let a = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();
    let b = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();

    let (ra, rb) = tokio::join!(
        a.acquire_with_intent(AuthIntent::Headless, Some("X")),
        b.acquire_with_intent(AuthIntent::Headless, Some("Y")),
    );

    assert_eq!(
        ra,
        Err(AuthError::RefreshRejected),
        "A's refresh re-issued its own rejected token X — typed failure for A"
    );
    assert_eq!(
        rb,
        Ok("X".to_string()),
        "B's rejected was Y (not X), so B reruns and its refresh yields X — a valid token for B"
    );
    assert_eq!(
        opener.call_count(),
        0,
        "headless callers never open a browser"
    );
    // At least two refresh grants: A's, then B's rerun.
    assert!(
        stub.refresh_grants.load(Ordering::SeqCst) >= 2,
        "B must have run its own refresh (rerun, not adoption)"
    );
}

// ---- in-process joiner state reconciliation (P1 regressions) ---------------
//
// These tests drive two independently constructed same-key sources through real
// leader/joiner acquisition and verify that subsequent public reads on both
// sources reflect the shared outcome — not the stale or absent credential each
// source carried before joining.
//
// The coordinator's in-process single-flight coalesces callers on a shared
// `InflightSlot`. On the old bearer-only publication path the joiner's own
// `state` cell was never updated, so:
//   - success: B's next plain `bearer()` served the locally-fresh-but-rejected
//     token X rather than the just-acquired Y (memory won over disk).
//   - failure: B's matching rejected X remained live; its next `bearer()` still
//     served it.
//   - no-persistence (Windows): B's state stayed empty; its next headless read
//     returned `NoCredential` instead of Y and a second browser opened.
//
// All three tests exercise the full `finish()` → `acquire_locked()` →
// `acquire_leader()` → `LeaderGuard::complete()` → joiner wiring.

// Unix-specific: the seed provides a live refresh token. The non-Unix constructor
// does not read the disk cache, so without a seed in memory A's headless path
// returns NoCredential rather than RefreshRejected.
#[cfg(unix)]
#[tokio::test]
async fn test_inprocess_joiner_reconciles_stale_state_after_shared_success() {
    // Scenario: A and B both loaded a locally-fresh-but-401'd token X. A leads,
    // refreshes to Y. B joins and wakes to Ok(Y). Without reconciliation B's
    // state still holds unexpired X, so B's next plain bearer() serves X — the
    // exact token the caller just reported 401-rejected.
    //
    // `join!` polls A first: A registers the INFLIGHT slot as leader, takes the
    // file lock, and yields on the refresh HTTP call. B is polled while A is in
    // flight, finds the slot, and joins.
    //
    // Mutation check (no state reconciliation): B.state stays Some(unexpired-X).
    // The subsequent bearer() call on B hits the memory cache (X is not expired,
    // rejected=None so identity check passes), and `a_next == b_next` FAILS
    // because ra_next = Y and rb_next = X.
    let stub = spawn_stub(false).await; // refresh returns fresh token
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::FailToOpen); // headless — no browser
    let cfg = config(&stub, "/disco/a", cache.path());

    // Unexpired X with a live refresh token: both A and B load it as their
    // initial state via the constructor's `read_cache` call.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "stale-X",
            "refresh_token": "live-refresh",
            "expires_at": future_secs(),  // NOT expired — locally fresh
        }),
    );

    let a = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();
    let b = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();

    // Both 401-recovery callers on the same key. A becomes leader (polled
    // first), refreshes to "refreshed-token-1", B joins A's slot.
    let (ra, rb) = tokio::join!(
        a.acquire_with_intent(AuthIntent::Headless, Some("stale-X")),
        b.acquire_with_intent(AuthIntent::Headless, Some("stale-X")),
    );

    assert_eq!(
        ra,
        Ok("refreshed-token-1".to_string()),
        "leader (A) receives the refreshed token"
    );
    assert_eq!(
        rb,
        Ok("refreshed-token-1".to_string()),
        "joiner (B) receives the leader's token"
    );
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        1,
        "exactly one refresh — B joined A's slot rather than running its own"
    );

    // After the join, both sources must hold the new token in state. Subsequent
    // plain bearer() calls (rejected=None) on both must return Y, not stale X.
    let ra_next = a
        .acquire_with_intent(AuthIntent::Headless, None)
        .await
        .expect("A subsequent read must return the refreshed token");
    let rb_next = b
        .acquire_with_intent(AuthIntent::Headless, None)
        .await
        .expect("B subsequent read must return the refreshed token, not stale X");

    assert_eq!(ra_next, "refreshed-token-1", "A subsequent read returns Y");
    assert_eq!(
        rb_next, "refreshed-token-1",
        "B subsequent read returns Y, not stale X — \
         mutation check: fails if joiner state was not reconciled (bearer-only publication)"
    );
    // No second refresh: both subsequent reads hit the in-memory cache.
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        1,
        "subsequent reads hit the in-memory cache — no second network call"
    );
}

// Unix-specific: refresh token is required for a headless rejection path.
#[cfg(unix)]
#[tokio::test]
async fn test_inprocess_joiner_neutralizes_rejected_on_matching_shared_failure() {
    // Scenario: A and B both carry unexpired X as their rejected token. A leads,
    // attempts a refresh, gets 401 (RefreshRejected). B joins and wakes to the
    // shared failure. Without reconciliation B's state still holds unexpired X,
    // so B's next plain bearer() serves it — the rejected credential reappears.
    //
    // With reconciliation, expire_rejected is called under lock, so X is
    // force-expired in B's state and cannot be served again.
    //
    // Mutation check (no expire_rejected call on the joiner Err path): B.state
    // still holds unexpired X after the join. B's next bearer() (rejected=None)
    // hits the memory cache and returns X. The assertion `rb_next != Ok("stale-X")`
    // FAILS — the rejected credential reappears.
    let stub = spawn_stub(true).await; // reject_refresh=true → 401 on every refresh
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::FailToOpen); // headless — no browser
    let cfg = config(&stub, "/disco/a", cache.path());

    // Unexpired X with a live (but destined-to-be-rejected) refresh token.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "stale-X",
            "refresh_token": "live-refresh",
            "expires_at": future_secs(),  // NOT expired — locally fresh
        }),
    );

    let a = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();
    let b = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();

    let (ra, rb) = tokio::join!(
        a.acquire_with_intent(AuthIntent::Headless, Some("stale-X")),
        b.acquire_with_intent(AuthIntent::Headless, Some("stale-X")),
    );

    assert_eq!(
        ra,
        Err(AuthError::RefreshRejected),
        "leader (A) gets RefreshRejected — dead refresh"
    );
    assert_eq!(
        rb,
        Err(AuthError::RefreshRejected),
        "joiner (B) shares the leader's RefreshRejected failure"
    );
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        1,
        "exactly one refresh attempt — B joined the failure rather than retrying"
    );

    // After the shared failure, B must not be able to serve stale X on a
    // subsequent plain bearer() call. Without reconciliation, B.state still
    // holds unexpired X and the next bearer() would return it.
    let rb_next = b.acquire_with_intent(AuthIntent::Headless, None).await;
    assert_ne!(
        rb_next,
        Ok("stale-X".to_string()),
        "B must not serve the rejected token after adopting a matching shared failure — \
         mutation check: fails if the joiner Err path skips expire_rejected"
    );
}

// Non-Unix-specific: disk persistence is disabled on Windows, so the only way
// for B to retain Y after joining is in-memory state reconciliation. On Unix
// the disk can provide Y as a fallback, masking a reconciliation failure.
#[cfg(not(unix))]
#[tokio::test]
async fn test_inprocess_joiner_populates_empty_state_no_second_acquisition() {
    // Scenario: A and B both start with empty state (no disk token on non-Unix).
    // A leads, opens a browser, exchanges the code for Y. B joins A's slot and
    // wakes to Ok(Y). Without reconciliation, B.state stays None. B's next
    // headless acquire returns NoCredential instead of Y, and a second browser
    // would open if UserInitiated.
    //
    // Mutation check (no state reconciliation): B.state stays None. The
    // subsequent headless acquire on B returns Err(NoCredential) instead of
    // Ok("browser-token-1") — the assertion FAILS.
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();
    let approve = ScriptedOpener::new(Script::Approve);

    let a = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(approve.clone()),
    )
    .unwrap();
    let b = PkceOAuthTokenSource::new_with(
        config(&stub, "/disco/a", cache.path()),
        Arc::new(approve.clone()),
    )
    .unwrap();

    // Both start with empty state — UserInitiated falls through to a browser.
    let (ra, rb) = tokio::join!(
        a.acquire_with_intent(AuthIntent::UserInitiated, None),
        b.acquire_with_intent(AuthIntent::UserInitiated, None),
    );

    assert_eq!(
        ra,
        Ok("browser-token-1".to_string()),
        "leader (A) gets the browser token"
    );
    assert_eq!(
        rb,
        Ok("browser-token-1".to_string()),
        "joiner (B) shares the leader's browser token"
    );
    assert_eq!(
        approve.call_count(),
        1,
        "exactly one browser opened — B joined rather than launching its own"
    );
    assert_eq!(
        stub.code_grants.load(Ordering::SeqCst),
        1,
        "exactly one authorization-code exchange"
    );

    // B's subsequent headless acquire must return Y from in-memory state without
    // a second browser. Without reconciliation, B.state is None and headless
    // returns NoCredential (no disk fallback on non-Unix).
    let rb_next = b
        .acquire_with_intent(AuthIntent::Headless, None)
        .await
        .expect(
            "B subsequent headless read must return Y from in-memory state, not NoCredential — \
             mutation check: fails if joiner state was not reconciled (bearer-only publication)",
        );
    assert_eq!(
        rb_next, "browser-token-1",
        "B retains Y in memory for subsequent headless reads"
    );
    // No second browser: B's subsequent read hit the in-memory cache.
    assert_eq!(
        approve.call_count(),
        1,
        "no second browser opened — B's subsequent headless read hit the in-memory cache"
    );
    assert_eq!(
        stub.code_grants.load(Ordering::SeqCst),
        1,
        "no second code exchange"
    );
}

// ---- a browser success that re-issues the rejected bytes must fail typed ---
//
// The 401-recovery invariant lives at `finish`'s persistence boundary, so it
// must hold on the browser-success path too — not just refresh. An
// interactive caller whose refresh is dead falls through to a browser sign-in;
// if that exchange re-issues the exact token the caller reported 401-rejected
// (a provider reusing an access token within its validity window), the guard
// must terminate typed before caching it rather than hand back the dead
// bearer. A single interactive leader exercises the path; the colliding-joiner
// rerun routes through the same boundary.

#[cfg(unix)]
#[tokio::test]
async fn test_interactive_browser_reissuing_rejected_token_fails_typed_not_loop() {
    // Refresh 401s (dead), so an interactive intent falls through to the
    // browser; the exchange stickily returns one fixed token on every grant.
    let stub = spawn_stub_with_modes(
        RefreshMode::Reject,
        ExchangeMode::SucceedSticky("sticky-browser"),
    )
    .await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    // Expired seed with a (dead) refresh token: the caller misses the cache,
    // its refresh is rejected, and it browses.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "expired-seed",
            "refresh_token": "dead-refresh",
            "expires_at": 1u64,
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();

    // The caller reports the sticky browser token as its rejected bearer, so
    // the browser exchange hands back exactly those bytes.
    let result = src
        .acquire_with_intent(AuthIntent::UserInitiated, Some("sticky-browser"))
        .await;

    assert_eq!(
        result,
        Err(AuthError::NetworkUnavailable),
        "a browser success equal to the rejected bytes must fail typed, not return them"
    );
    assert_eq!(
        opener.call_count(),
        1,
        "the interactive attempt browsed exactly once — no loop re-launching the browser"
    );
    assert_eq!(
        stub.code_grants.load(Ordering::SeqCst),
        1,
        "exactly one code exchange — the guard fails terminally instead of retrying"
    );
}

// ---- a rejected re-issue must not poison the cache for later callers -------
//
// The persistence-boundary guard's whole purpose: a rejected-aware acquisition
// that a provider answers with the exact 401'd bytes must not leave those bytes
// cached as fresh. Before the fix, `finish()` persisted first and the guard
// fired after, so the dead token survived on disk and in memory — the next
// plain `bearer()` (`rejected = None`) and any freshly constructed source would
// serve it straight from the cache with no re-validation. These two regressions
// prove the cache is untouched after the typed failure, on both the refresh and
// the browser re-issue paths.

#[cfg(unix)]
#[tokio::test]
async fn test_sticky_refresh_rejection_does_not_poison_cache_for_later_callers() {
    // A sticky provider re-issues `sticky-token` on every refresh. A caller that
    // reports `sticky-token` as its rejected bearer gets a typed failure — and
    // the rejected bytes must never reach the cache.
    let stub = spawn_stub_with(RefreshMode::SucceedSticky("sticky-token")).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "expired-seed",
            "refresh_token": "live-refresh",
            "expires_at": 1u64,
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();
    let rejected = src
        .acquire_with_intent(AuthIntent::Headless, Some("sticky-token"))
        .await;
    assert_eq!(
        rejected,
        Err(AuthError::RefreshRejected),
        "a refresh that re-issues the rejected bytes fails typed"
    );
    assert_eq!(stub.refresh_grants.load(Ordering::SeqCst), 1);

    // The rejected bytes were never persisted: a fresh process reading the same
    // cache path finds the original expired seed, not `sticky-token`.
    let on_disk = std::fs::read_to_string(cache_file_path(&cfg, cache.path())).unwrap();
    assert!(
        on_disk.contains("expired-seed") && !on_disk.contains("sticky-token"),
        "the failed acquisition poisoned the on-disk cache: {on_disk}"
    );

    // A freshly constructed source over the same cache must therefore refresh
    // over the network to obtain the token — it cannot serve a cached poison.
    // Under the bug this was a lock-free cache hit and `refresh_grants` stayed
    // at 1; the fix forces a second refresh. `Headless, None` is the plain
    // `bearer()` path (rejected = None) with the typed error surfaced directly.
    let fresh = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    let token = fresh
        .acquire_with_intent(AuthIntent::Headless, None)
        .await
        .expect("a rejected=None caller legitimately obtains the current token");
    assert_eq!(token, "sticky-token");
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        2,
        "the fresh source re-validated over the network — it did not serve a cached poison"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_sticky_browser_rejection_does_not_poison_cache_for_later_callers() {
    // Refresh is dead, so an interactive caller browses; the exchange stickily
    // re-issues `sticky-browser`. A caller reporting those bytes as rejected
    // gets a typed failure, and the dead token must never reach the cache.
    let stub = spawn_stub_with_modes(
        RefreshMode::Reject,
        ExchangeMode::SucceedSticky("sticky-browser"),
    )
    .await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "expired-seed",
            "refresh_token": "dead-refresh",
            "expires_at": 1u64,
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();
    let rejected = src
        .acquire_with_intent(AuthIntent::UserInitiated, Some("sticky-browser"))
        .await;
    assert_eq!(
        rejected,
        Err(AuthError::NetworkUnavailable),
        "a browser exchange that re-issues the rejected bytes fails typed"
    );
    assert_eq!(opener.call_count(), 1);
    assert_eq!(stub.code_grants.load(Ordering::SeqCst), 1);

    // The rejected bytes were never persisted: the on-disk cache still holds
    // the expired seed, so no fresh process can restore `sticky-browser`.
    let on_disk = std::fs::read_to_string(cache_file_path(&cfg, cache.path())).unwrap();
    assert!(
        on_disk.contains("expired-seed") && !on_disk.contains("sticky-browser"),
        "the failed browser acquisition poisoned the on-disk cache: {on_disk}"
    );

    // A subsequent plain `bearer()` (Headless, `rejected = None`) reads that
    // un-poisoned cache: the seed is expired and its refresh is dead, so it
    // fails `RefreshRejected` — it never serves `sticky-browser` from cache.
    // Under the bug the poisoned cache made this a hit returning the dead bytes.
    let fresh_opener = ScriptedOpener::new(Script::Approve);
    let fresh = PkceOAuthTokenSource::new_with(cfg, Arc::new(fresh_opener.clone())).unwrap();
    let later = fresh.acquire_with_intent(AuthIntent::Headless, None).await;
    assert_eq!(
        later,
        Err(AuthError::RefreshRejected),
        "a fresh source must not serve the rejected browser token from cache"
    );
    assert_eq!(
        fresh_opener.call_count(),
        0,
        "a headless caller never browses"
    );
}

// ---- a 401 on a locally-fresh token neutralizes the cached copy -----------
//
// P1: the persistence-boundary guard refuses to *save* a re-issued rejected
// token, but the ORIGINAL cached copy — the exact bytes the provider just
// 401'd — is untouched. Because `is_expired` trusts only the clock, a later
// plain `bearer()` (`rejected = None`) or a freshly constructed source would
// serve that dead token straight from cache. `expire_rejected` force-expires
// the cached copy (memory and disk) under the lock the moment a caller reports
// it rejected, so no future caller and no fresh process can serve it, while the
// refresh token — not rejected, and the engine of recovery — stays intact.

#[cfg(unix)]
#[tokio::test]
async fn test_rejected_fresh_token_is_neutralized_for_a_fresh_process() {
    // The cached access token `A` is locally UNEXPIRED, and the provider
    // stickily re-issues `A` on refresh. A caller reports `A` as rejected: the
    // refresh hands back `A`, the guard fails typed without persisting it — and
    // the original unexpired `A` must not survive on disk for a fresh process.
    let stub = spawn_stub_with(RefreshMode::SucceedSticky("A")).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "A",
            "refresh_token": "live-refresh",
            "expires_at": future_secs(),
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();
    let rejected = src
        .acquire_with_intent(AuthIntent::Headless, Some("A"))
        .await;
    assert_eq!(
        rejected,
        Err(AuthError::RefreshRejected),
        "a refresh that re-issues the rejected bytes fails typed"
    );
    assert_eq!(stub.refresh_grants.load(Ordering::SeqCst), 1);

    // The on-disk copy of `A` was force-expired in place: the refresh token is
    // preserved, but the access token's expiry is neutralized so no clock-based
    // read can serve it. Under the bug it stayed at its future expiry.
    let on_disk: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(cache_file_path(&cfg, cache.path())).unwrap(),
    )
    .unwrap();
    assert_eq!(
        on_disk["access_token"], "A",
        "the entry is kept, not deleted"
    );
    assert_eq!(
        on_disk["refresh_token"], "live-refresh",
        "the refresh token — not rejected — survives for recovery"
    );
    assert_eq!(
        on_disk["expires_at"], 0,
        "the rejected access token was force-expired on disk"
    );

    // A freshly constructed source reading that cache must NOT serve `A` from
    // the clock: it sees the neutralized entry as expired and refreshes over
    // the network. Under the bug this was a lock-free cache hit returning the
    // dead `A` with `refresh_grants` frozen at 1.
    let fresh = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    let token = fresh
        .acquire_with_intent(AuthIntent::Headless, None)
        .await
        .expect("a rejected=None caller obtains the provider's current token");
    assert_eq!(token, "A");
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        2,
        "the fresh source re-validated over the network — it did not serve the neutralized cache"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_rejected_fresh_token_is_neutralized_for_the_same_source() {
    // The in-memory layer of the same neutralization: after the SAME source
    // fails a 401-recovery on unexpired `A`, its next plain `bearer()`
    // (`rejected = None`) must not serve `A` from the in-memory cell — it must
    // re-validate. `A` is sticky, so recovery returns `A` again, but only after
    // a real refresh grant (the discriminator: 1 cache hit vs. 2 grants).
    let stub = spawn_stub_with(RefreshMode::SucceedSticky("A")).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "A",
            "refresh_token": "live-refresh",
            "expires_at": future_secs(),
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    assert_eq!(
        src.acquire_with_intent(AuthIntent::Headless, Some("A"))
            .await,
        Err(AuthError::RefreshRejected),
    );
    assert_eq!(stub.refresh_grants.load(Ordering::SeqCst), 1);

    // Same source, plain bearer: the in-memory `A` was neutralized, so this is
    // a miss that refreshes rather than a cache hit. Under the bug the
    // unexpired in-memory `A` was served directly and `refresh_grants` stayed 1.
    let token = src
        .acquire_with_intent(AuthIntent::Headless, None)
        .await
        .expect("a subsequent plain bearer re-validates rather than serving the dead token");
    assert_eq!(token, "A");
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        2,
        "the same source re-validated in memory — it did not serve the neutralized token"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_rejected_fresh_token_neutralized_when_recovery_browses() {
    // The browser variant: `A` is unexpired but its refresh token is dead, so
    // an interactive 401-recovery falls through to the browser, whose exchange
    // stickily re-issues `A`. The guard fails typed without persisting it, and
    // the neutralized `A` must not survive for a later headless caller.
    let stub = spawn_stub_with_modes(RefreshMode::Reject, ExchangeMode::SucceedSticky("A")).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "A",
            "refresh_token": "dead-refresh",
            "expires_at": future_secs(),
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();
    let rejected = src
        .acquire_with_intent(AuthIntent::UserInitiated, Some("A"))
        .await;
    assert_eq!(
        rejected,
        Err(AuthError::NetworkUnavailable),
        "a browser exchange that re-issues the rejected bytes fails typed"
    );
    assert_eq!(opener.call_count(), 1);
    assert_eq!(stub.code_grants.load(Ordering::SeqCst), 1);

    // The unexpired `A` was force-expired on disk, so a fresh headless source
    // finds it unusable and — its refresh being dead — fails `RefreshRejected`
    // rather than serving `A`. Under the bug the still-fresh `A` was a cache
    // hit that returned the dead token.
    let fresh_opener = ScriptedOpener::new(Script::Approve);
    let fresh = PkceOAuthTokenSource::new_with(cfg, Arc::new(fresh_opener.clone())).unwrap();
    let later = fresh.acquire_with_intent(AuthIntent::Headless, None).await;
    assert_eq!(
        later,
        Err(AuthError::RefreshRejected),
        "a fresh source must not serve the neutralized rejected token from cache"
    );
    assert_eq!(
        fresh_opener.call_count(),
        0,
        "a headless caller never browses"
    );
}

// ---- P1-1 bounded three-stage neutralization: disk fallback paths -----------
//
// `expire_rejected()` neutralizes the on-disk token with three-stage fallback:
// 1. Atomic rewrite via `persist()` (temp-file + rename, owner-only perms).
// 2. In-place truncating overwrite via `OpenOptions::write().truncate(true)` —
//    succeeds even when the parent directory is non-writable, because only the
//    file's own mode matters for writing an existing file.
// 3. `remove_file` as a last resort.
//
// The primary case this tests: a 0600 token file under a 0500 parent directory.
// Temp-file creation (for the atomic path) fails with EACCES; the in-place
// write succeeds because the file itself is owner-writable. After the in-place
// overwrite the file still exists but carries `expires_at = 0`, so a later
// plain `bearer(None)` or a freshly constructed source reads the now-expired
// entry and re-validates over the network instead of serving the dead token.

#[cfg(unix)]
#[tokio::test]
async fn test_rejected_token_disk_neutralization_neutralizes_in_place_when_parent_blocks_rewrite() {
    use std::os::unix::fs::PermissionsExt as _;

    // Seed unexpired `A` with a live refresh. The provider stickily re-issues `A`.
    let stub = spawn_stub_with(RefreshMode::SucceedSticky("A")).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    // Create the token file inside a dedicated subdirectory so we can chmod
    // just that subdirectory non-writable without affecting the test harness.
    let token_dir = cache.path().join("protected");
    std::fs::create_dir_all(&token_dir).unwrap();

    // Override the config to use the protected subdir.
    let cfg = PkceOAuthConfig {
        cache_dir_override: Some(token_dir.clone()),
        ..cfg
    };
    let cache_file = cache_file_path(&cfg, &token_dir);

    seed_cache(
        &cfg,
        &token_dir,
        json!({
            "access_token": "A",
            "refresh_token": "live-refresh",
            "expires_at": future_secs(),
        }),
    );

    // Build the source: it reads `A` from disk into its in-memory cell.
    let src = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();

    // The token file lives at `token_dir/databricks/<hash>.json`. Its direct
    // parent is `token_dir/databricks/`, not `token_dir` itself — the
    // coordinator's `cache_path_for()` appends the namespace subdir. Assert
    // the relationship explicitly so a future path-resolution change breaks
    // loudly here instead of silently letting the atomic write succeed (which
    // would make the test vacuously pass even without the in-place fallback).
    let protected_dir = cache_file
        .parent()
        .expect("cache file must have a parent directory");
    assert_eq!(
        protected_dir,
        token_dir.join("databricks"),
        "cache file's direct parent is token_dir/databricks, not token_dir"
    );

    // Pre-create the advisory lock file so `acquire_auth_lock` can open it
    // even after the directory is made non-writable. The lock file must exist
    // before the chmod, because `OpenOptions::create(true)` on an existing
    // file succeeds regardless of parent-dir permissions, while creating a new
    // file in a 0500 directory would EACCES.
    let lock_file = {
        let mut p = cache_file.as_os_str().to_owned();
        p.push(".lock");
        std::path::PathBuf::from(p)
    };
    std::fs::File::create(&lock_file).expect("pre-create lock file before chmod");

    // Make the direct parent non-writable (0500): temp-file creation for the
    // atomic persist requires creating a new file in this directory → EACCES.
    // The file itself remains 0600 owner-writable, so the in-place fallback
    // path in `expire_rejected` can still open and truncate it.
    std::fs::set_permissions(protected_dir, std::fs::Permissions::from_mode(0o500)).unwrap();

    // Trigger 401-recovery: refresh stickily re-issues `A`, `finish()` rejects
    // it typed. `expire_rejected` runs: atomic persist fails (EACCES on parent),
    // in-place write succeeds (file mode 0600).
    let result = src
        .acquire_with_intent(AuthIntent::Headless, Some("A"))
        .await;
    assert_eq!(
        result,
        Err(AuthError::RefreshRejected),
        "typed failure returned; neutralization does not disrupt the recovery path"
    );
    assert_eq!(stub.refresh_grants.load(Ordering::SeqCst), 1);

    // Restore write permission so the test harness can clean up.
    std::fs::set_permissions(protected_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

    // The cache file still exists (in-place write, not removal), but its
    // `expires_at` should now be 0 — it was overwritten in-place.
    assert!(
        cache_file.is_file(),
        "in-place fallback: file still exists (not removed)"
    );
    let raw = std::fs::read(&cache_file).expect("cache file readable after in-place write");
    let cached: serde_json::Value =
        serde_json::from_slice(&raw).expect("cache file parseable after in-place write");
    assert_eq!(
        cached.get("expires_at").and_then(|v| v.as_u64()),
        Some(0),
        "in-place write set expires_at = 0: token is now expired on disk"
    );

    // A fresh source constructed after the neutralization must not serve `A`.
    let fresh_src = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();
    // The disk token is expired; bearer() falls through to refresh, which
    // stickily re-issues `A`, which `finish()` rejects again (no rejected
    // identity on this plain call — the disk is now expired, so the source
    // enters the refresh path, gets `A` back from the provider, and `finish()`
    // sees no rejection guard and would persist it). But with no `rejected`
    // passed here, a plain `bearer()` with the now-expired disk entry must
    // re-validate. If the in-place write succeeded, the disk token has
    // expires_at = 0 and `cached_hit` skips it, so the source goes to refresh.
    // We confirm `A` is not served as a cache hit: the stub records a second
    // refresh grant.
    let _ = fresh_src
        .acquire_with_intent(AuthIntent::Headless, None)
        .await;
    assert!(
        stub.refresh_grants.load(Ordering::SeqCst) >= 2,
        "fresh source did not serve `A` as a plain cache hit — it re-validated over the network"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_rejected_token_in_memory_neutralized_when_disk_neutralization_skipped() {
    // When `expire_rejected()` cannot read a matching disk entry (e.g. the cache
    // path is not a readable regular file), the disk layer is not neutralized,
    // but the IN-MEMORY layer is always neutralized unconditionally. This test
    // proves the in-memory safety path: even without disk neutralization, a
    // subsequent plain `bearer()` on the same source cannot serve the dead token
    // from the in-memory cell.
    let stub = spawn_stub_with(RefreshMode::SucceedSticky("A")).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());
    let cache_file = cache_file_path(&cfg, cache.path());

    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "A",
            "refresh_token": "live-refresh",
            "expires_at": future_secs(),
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();

    // Replace the cache file with a directory so `read_private_cache` inside
    // `expire_rejected` returns None (EISDIR on open). The disk branch is
    // skipped entirely — only the in-memory layer is neutralized.
    std::fs::remove_file(&cache_file).unwrap();
    std::fs::create_dir_all(&cache_file).unwrap();

    let result = src
        .acquire_with_intent(AuthIntent::Headless, Some("A"))
        .await;
    assert_eq!(result, Err(AuthError::RefreshRejected));
    assert_eq!(stub.refresh_grants.load(Ordering::SeqCst), 1);

    // In-memory layer: force-expired. The same source's next plain bearer()
    // must not serve `A` from the in-memory cell.
    let next = src.acquire_with_intent(AuthIntent::Headless, None).await;
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        2,
        "in-memory `A` was force-expired; same source went to the network rather than serving the dead token"
    );
    // The sticky refresh obtained `A` from the network (grant #2). The persist()
    // call fails because the cache path is now a directory — save() maps the
    // persist failure to NetworkUnavailable. This proves: (a) the in-memory
    // neutralization worked (the source re-validated rather than serving A from
    // the expired in-memory cell), and (b) the network was reached. The
    // NetworkUnavailable result is an expected artifact of the directory-as-
    // cache-path test setup, not a correctness gap.
    assert!(
        matches!(next, Err(AuthError::NetworkUnavailable)),
        "save() fails with NetworkUnavailable on persist failure (expected artifact of test setup)"
    );
    assert_ne!(
        next,
        Ok("A".to_owned()),
        "A was not served from the expired in-memory cell — network was reached"
    );

    // Cleanup the directory we created.
    std::fs::remove_dir(&cache_file).ok();
}

// ---- expired-sibling replacement must not satisfy a 401 recovery ----------
//
// After a 401, `rejected = Some(t)` makes the expiry clock untrustworthy, so a
// cache hit requires a token that both DIFFERS from `t` and is still unexpired.
// An expired sibling token — one that merely differs from the rejected bytes —
// must NOT be served as the replacement: doing so would skip the refresh the
// 401 demanded and hand back a token the provider will also reject.

#[cfg(unix)]
#[tokio::test]
async fn test_rejected_recovery_skips_expired_sibling_and_refreshes() {
    let stub = spawn_stub(false).await; // refresh succeeds
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    // The cached token is a DIFFERENT string from the rejected bytes, but it is
    // expired. Under the old "differs is enough" rule it would be returned as
    // the sibling replacement; the fix requires it to be unexpired too, so the
    // coordinator must fall through to the live refresh instead.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "expired-sibling",
            "refresh_token": "live-refresh",
            "expires_at": 1u64,
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    let token = src
        .acquire_with_intent(AuthIntent::Headless, Some("rejected-original"))
        .await
        .expect("an expired sibling forces a refresh rather than being reused");
    assert_eq!(
        token, "refreshed-token-1",
        "the expired sibling was not accepted; a fresh token was obtained"
    );
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        1,
        "the 401 recovery refreshed instead of reusing the expired sibling"
    );
    assert_eq!(opener.call_count(), 0, "a live refresh needs no browser");
}

// ---- code-exchange classifier: rejection vs. infrastructure --------------
//
// The browser code exchange must mirror the refresh classifier: only a 4xx
// `invalid_grant` establishes the authorization code was rejected (terminal,
// cooldown-worthy `ExchangeFailed`). A 429, any 5xx, and a malformed 2xx are a
// transient provider fault that must surface as `NetworkUnavailable` — never
// poisoning the 5-minute cooldown against a provider outage after callback.

#[tokio::test]
async fn test_exchange_invalid_grant_is_exchange_failed_and_cools_down() {
    let stub = spawn_stub_with_exchange(ExchangeMode::Fail(
        axum::http::StatusCode::UNAUTHORIZED,
        "invalid_grant",
    ))
    .await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    // A genuinely rejected code is terminal ExchangeFailed and is
    // cooldown-worthy: a following Auto caller reads the cooldown without a
    // second browser.
    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
    let first = src.acquire_with_intent(AuthIntent::Auto, None).await;
    assert_eq!(
        first,
        Err(AuthError::ExchangeFailed),
        "a 401 invalid_grant on the code exchange is a rejected grant"
    );
    assert_eq!(opener.call_count(), 1);

    let second = src.acquire_with_intent(AuthIntent::Auto, None).await;
    assert_eq!(
        second,
        Err(AuthError::ExchangeFailed),
        "the rejected exchange wrote a cooldown the next Auto caller honors"
    );
    assert_eq!(
        opener.call_count(),
        1,
        "the cooldown suppressed a second browser launch"
    );
}

#[tokio::test]
async fn test_exchange_transient_faults_are_network_unavailable_not_cooldown() {
    // A 429, a 500, and a malformed 2xx are provider faults, not rejected
    // codes: each must surface as NetworkUnavailable and leave no cooldown, so
    // a subsequent Auto caller retries with a fresh browser rather than
    // inheriting a suppressed outcome.
    let cases = [
        ExchangeMode::Fail(axum::http::StatusCode::TOO_MANY_REQUESTS, "slow_down"),
        ExchangeMode::Fail(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "temporarily_unavailable",
        ),
        ExchangeMode::MalformedSuccess,
    ];
    for exchange in cases {
        let stub = spawn_stub_with_exchange(exchange).await;
        let cache = TempDir::new().unwrap();
        let opener = ScriptedOpener::new(Script::Approve);
        let cfg = config(&stub, "/disco/a", cache.path());

        let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
        let result = src.acquire_with_intent(AuthIntent::Auto, None).await;
        assert_eq!(
            result,
            Err(AuthError::NetworkUnavailable),
            "a transient exchange fault is infrastructural, not a rejected code"
        );
        assert_eq!(opener.call_count(), 1);

        // No cooldown was written, so a second Auto caller launches again
        // rather than reading a suppressed outcome.
        let retry = src.acquire_with_intent(AuthIntent::Auto, None).await;
        assert_eq!(
            retry,
            Err(AuthError::NetworkUnavailable),
            "a transient exchange fault leaves no cooldown to suppress the retry"
        );
        assert_eq!(
            opener.call_count(),
            2,
            "no cooldown means the next Auto caller opens a fresh browser"
        );
    }
}

#[tokio::test]
async fn test_exchange_timeout_is_network_unavailable_not_cooldown() {
    // The code exchange hangs far longer than the injected per-request HTTP
    // timeout, so the exchange POST times out at the transport layer with no
    // verdict from the provider — the transport branch the classifier maps to
    // NetworkUnavailable. Like the refresh-timeout test, a short real-time
    // timeout is injected rather than pausing the clock: under `start_paused`
    // tokio would auto-advance into the timer while the real loopback
    // discovery/authorize round-trips are still in flight, tripping the timeout
    // on the wrong request. Real time keeps the timeout attached to the
    // exchange that actually hangs.
    let stub = spawn_stub_with_exchange(ExchangeMode::Hang(Duration::from_secs(30))).await;
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    let src = PkceOAuthTokenSource::new_with_http_timeout(
        cfg,
        Arc::new(opener.clone()),
        Duration::from_millis(300),
    )
    .unwrap();
    let result = src.acquire_with_intent(AuthIntent::Auto, None).await;
    assert_eq!(
        result,
        Err(AuthError::NetworkUnavailable),
        "an exchange transport timeout is infrastructural, not a rejected code"
    );
    assert_eq!(opener.call_count(), 1);

    // The timed-out exchange wrote no cooldown, so a second Auto caller launches
    // its own browser rather than inheriting a suppressed outcome.
    let retry = src.acquire_with_intent(AuthIntent::Auto, None).await;
    assert_eq!(
        retry,
        Err(AuthError::NetworkUnavailable),
        "an exchange transport timeout leaves no cooldown to suppress the retry"
    );
    assert_eq!(
        opener.call_count(),
        2,
        "no cooldown means the next Auto caller opens a fresh browser"
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

#[cfg(unix)]
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
    #[cfg(unix)]
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

#[cfg(unix)]
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

// ---- cross-process failure single-flight (attempt-record protocol) --------
//
// `INFLIGHT` coalesces same-key callers within one process before they reach
// the file lock, so two separate processes both queued on the lock do NOT
// share the in-process registry. Without the attempt-record protocol, a
// process that acquires the lock AFTER the holder fails would re-run the
// full flow from scratch — a second browser launch on `Denied`, or a second
// dead-refresh call on `RefreshRejected`. The attempt sidecar lets the
// second process detect that the predecessor completed while it was waiting
// and adopt its failure directly.

#[cfg(unix)]
#[tokio::test]
async fn test_crossprocess_waiting_headless_adopts_predecessor_refresh_rejected() {
    // Two real headless processes on one key. The cache holds an expired
    // token with a dead refresh. A wins the lock and calls the stub; the
    // stub holds A's response so B can deterministically snapshot gen=0
    // and queue on the lock before A completes. Once B's snapshot marker
    // fires, A is released: it gets `invalid_grant`, writes the attempt
    // sidecar (gen=1), and releases the lock. B acquires the lock, sees
    // gen=1 > snap=0, and adopts `RefreshRejected` — ONE refresh grant
    // total across both processes.
    //
    // This replaces the prior simultaneous-start design, which was not
    // deterministic: the instant-reject stub could complete A before B
    // ever snapshotted, giving B snap=1 and causing a spurious second
    // refresh grant.
    let (stub, gate) = spawn_stub_with_held_refresh(HeldRefreshResponse::Reject).await;
    let cache = TempDir::new().unwrap();
    let cfg = config(&stub, "/disco/a", cache.path());

    // Seed the shared cache: expired token with a dead refresh, so both
    // workers fall through to the refresh grant rather than a cache hit.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "stale",
            "refresh_token": "dead-refresh",
            "expires_at": 1u64,
        }),
    );

    let snapshot_b = cache.path().join("b.snapshot");

    // ---- Phase 1: spawn A. It acquires the lock and immediately calls the
    //              stub's refresh endpoint; the stub holds the response.
    let worker_a = spawn_worker(&cfg, cache.path(), "headless", "approve", "a", &[]);

    // ---- Phase 2: wait until the stub has received A's refresh request.
    //              This is an in-process await — no polling or timing.
    //              Once the stub is holding A's request, A owns the lock.
    gate.wait_for_request().await;

    // ---- Phase 3: spawn B with SNAPSHOT_MARKER. B starts, reads the
    //              attempt sidecar (gen=0, absent), emits its snapshot
    //              event, and then blocks on the lock behind A.
    let worker_b = spawn_worker(
        &cfg,
        cache.path(),
        "headless",
        "approve",
        "b",
        &[("AUTH_WORKER_SNAPSHOT_MARKER", snapshot_b.as_path())],
    );

    // ---- Phase 4: wait for B's snapshot marker. Proves B captured gen=0
    //              before A can record gen=1; lock queueing is not required
    //              for the temporal-generation discriminator to hold.
    wait_for_marker(&snapshot_b, "worker B snapshot").await;

    // ---- Phase 5: release A. Stub returns invalid_grant; A records
    //              RefreshRejected with gen=1 and releases the lock. B
    //              acquires the lock, sees gen=1 > snap=0, and adopts.
    gate.release();

    let (out_a, out_b) = tokio::join!(worker_a.join(), worker_b.join());

    // Both workers must report RefreshRejected.
    assert_eq!(
        out_a.result, "refresh_rejected",
        "worker A gets RefreshRejected on a dead refresh"
    );
    assert_eq!(
        out_b.result, "refresh_rejected",
        "worker B adopts RefreshRejected via the attempt sidecar"
    );
    assert_eq!(out_a.launches, 0, "headless never opens a browser");
    assert_eq!(out_b.launches, 0, "headless never opens a browser");

    // One refresh grant total: under the old protocol the second worker would
    // re-run the dead refresh independently; the attempt record prevents that.
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        1,
        "exactly one refresh grant across both headless processes"
    );
}

#[tokio::test]
async fn test_crossprocess_userinitiated_waiter_adopts_predecessor_denial() {
    // The adoption contract is *temporal*, not intent-based. A `UserInitiated`
    // caller whose pre-queue snapshot is older than the current generation was
    // already queued while the predecessor ran and MUST adopt its same-intent
    // failure — exactly as the in-process `INFLIGHT` registry coalesces
    // same-intent `UserInitiated` callers onto one leader within a process.
    //
    // When process A (UserInitiated) gets `Denied` and process B
    // (UserInitiated) was queued *behind* it (B's snapshot predates A's write),
    // B adopts A's denial without opening a second browser. The result:
    // exactly one browser launch and zero code exchanges — one browser total
    // across both processes.
    //
    // Note: this is different from a *later* explicit user retry, which
    // arrives after A completes, snapshots the new generation, sees no advance,
    // and naturally runs its own attempt. That behavior is proved by
    // `test_crossprocess_post_failure_userinitiated_runs_own_attempt` below.
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();
    let cfg = config(&stub, "/disco/a", cache.path());

    let launched_a = cache.path().join("a.launched");
    let proceed_a = cache.path().join("a.proceed");

    // Worker A holds the lock and keeps its browser open until we signal it,
    // so B is certain to be queued behind A before A resolves.
    let worker_a = spawn_worker(
        &cfg,
        cache.path(),
        "userinitiated",
        "deny",
        "a",
        &[
            ("AUTH_WORKER_LAUNCHED_MARKER", launched_a.as_path()),
            ("AUTH_WORKER_PROCEED_MARKER", proceed_a.as_path()),
        ],
    );

    // Wait until A holds the lock and its browser is open.
    wait_for_marker(&launched_a, "worker A browser launch").await;

    // Worker B (also UserInitiated, approve-scripted) queues behind A on the
    // file lock. Even though B would succeed if it ran its own browser, it
    // must adopt A's denial since it was queued while A held the lock.
    //
    // SNAPSHOT_MARKER is emitted by the tracing layer in B's process after B
    // snapshots gen=0 and before it queues on the file lock — so observing it
    // proves B captured generation 0 before A records generation 1.
    let snapshot_b = cache.path().join("b.snapshot");
    let worker_b = spawn_worker(
        &cfg,
        cache.path(),
        "userinitiated",
        "approve",
        "b",
        &[("AUTH_WORKER_SNAPSHOT_MARKER", snapshot_b.as_path())],
    );
    // Wait until B has snapshotted gen=0, then release A.
    wait_for_marker(&snapshot_b, "worker B snapshot").await;

    // Release A: it denies, writes the cooldown + attempt sidecars, releases lock.
    std::fs::write(&proceed_a, b"go").unwrap();
    let out_a = worker_a.join().await;
    assert_eq!(out_a.result, "denied", "worker A is denied");
    assert_eq!(out_a.launches, 1, "worker A opens one browser");

    // Worker B adopts A's denial — it does not open a second browser even
    // though it is UserInitiated. Under the old contract B would open its own
    // browser and succeed; under the correct temporal contract it adopts.
    let out_b = worker_b.join().await;
    assert_eq!(
        out_b.result, "denied",
        "queued UserInitiated worker B adopts A's denial rather than re-running"
    );
    assert_eq!(
        out_b.launches, 0,
        "worker B adopts the denial without opening a browser"
    );
    assert_eq!(
        stub.code_grants.load(Ordering::SeqCst),
        0,
        "no code exchange — B adopted A's Denied without reaching the token endpoint"
    );
}

#[tokio::test]
async fn test_crossprocess_post_failure_userinitiated_runs_own_attempt() {
    // A `UserInitiated` caller that arrives *after* a failure — not queued
    // during it — snapshots the current (advanced) generation, sees no advance
    // when it acquires the lock, and runs its own attempt. "Later explicit user
    // retry bypasses" falls out of the temporal snapshot comparison without any
    // special case.
    let stub = spawn_stub(false).await;
    let cache = TempDir::new().unwrap();
    let cfg = config(&stub, "/disco/a", cache.path());

    // Worker A (UserInitiated, deny-scripted) runs to completion first. No
    // synchronization needed — we await it fully before constructing B.
    let worker_a = spawn_worker(&cfg, cache.path(), "userinitiated", "deny", "a", &[]);
    let out_a = worker_a.join().await;
    assert_eq!(out_a.result, "denied", "worker A is denied");
    assert_eq!(out_a.launches, 1, "worker A opens one browser");

    // Worker B arrives after A has fully completed and the attempt record is
    // already written with the new generation. B snapshots the current
    // (advanced) generation, acquires the lock, sees no further advance, and
    // runs its own browser flow — it should succeed.
    let worker_b = spawn_worker(&cfg, cache.path(), "userinitiated", "approve", "b", &[]);
    let out_b = worker_b.join().await;
    assert_eq!(
        out_b.result, "ok",
        "post-failure UserInitiated worker B runs its own flow and succeeds"
    );
    assert_eq!(
        out_b.launches, 1,
        "worker B opens its own browser (not inherited from A)"
    );
    assert_eq!(
        stub.code_grants.load(Ordering::SeqCst),
        1,
        "exactly one code exchange (worker B's own approval)"
    );
}

// ---- cross-process: adopter must NOT re-write the attempt generation -------
//
// Proves that an adopting process B does not advance the attempt-sidecar
// generation, so a third process C — which arrives AFTER A's failure but sees
// no generation advance (B didn't re-write) — correctly runs its own attempt.
//
// Protocol ordering (deterministic via markers, no timing):
//   1. A (UserInitiated, deny-scripted) holds the lock mid-browser via
//      LAUNCHED_MARKER + PROCEED_MARKER.
//   2. B (UserInitiated, deny-scripted) starts while A holds the lock.
//      B emits SNAPSHOT_MARKER after snapshotting gen=0 and before queueing
//      on the lock. Parent observes the marker, then signals A's proceed.
//   3. A: denial recorded, writes gen=1 to the attempt sidecar, releases lock.
//   4. B: acquires lock, sees gen=1 > snap=0, intent matches → adopts A's
//      denial. With the fix B does NOT re-write the sidecar. With the mutation
//      (restoring the deleted write_attempt at the adoption site) B writes
//      gen=2.
//   5. After A and B finish: assert sidecar generation == 1. This is the
//      discriminating assertion — it FAILS when the adoption-site re-write is
//      restored (gen becomes 2 instead of 1).
//   6. C (UserInitiated, approve-scripted) starts fresh. C's snapshot == gen
//      on disk (1 with fix, 2 with mutation). In both cases C sees no advance
//      and runs its own browser flow. code_grants increments by 1 for C.
//
// This test is cache-free (no seed_cache / disk-token assertions) so it runs
// on Windows as well as Unix.

#[tokio::test]
async fn test_crossprocess_adopter_does_not_advance_generation() {
    let stub = spawn_stub(false).await; // deny does not hit any endpoint
    let cache = TempDir::new().unwrap();
    let cfg = config(&stub, "/disco/a", cache.path());

    // ---- Phase 1: A holds the lock mid-browser ----------------------------
    let launched_a = cache.path().join("a.launched");
    let proceed_a = cache.path().join("a.proceed");

    let worker_a = spawn_worker(
        &cfg,
        cache.path(),
        "userinitiated",
        "deny",
        "a",
        &[
            ("AUTH_WORKER_LAUNCHED_MARKER", launched_a.as_path()),
            ("AUTH_WORKER_PROCEED_MARKER", proceed_a.as_path()),
        ],
    );

    // Wait until A holds the lock and its browser is open.
    wait_for_marker(&launched_a, "worker A browser launch").await;

    // ---- Phase 2: B queues behind A, snapshot barrier ---------------------
    // B is UserInitiated + deny-scripted, but B will adopt A's denial rather
    // than opening its own browser (B was queued while A held the lock).
    // SNAPSHOT_MARKER is emitted by the tracing layer in B's process after B
    // snapshots gen=0 and before it queues on the lock — so observing it
    // proves B captured generation 0 before A records generation 1.
    let snapshot_b = cache.path().join("b.snapshot");
    let worker_b = spawn_worker(
        &cfg,
        cache.path(),
        "userinitiated",
        "deny",
        "b",
        &[("AUTH_WORKER_SNAPSHOT_MARKER", snapshot_b.as_path())],
    );

    // Wait until B has snapshotted gen=0, then release A.
    wait_for_marker(&snapshot_b, "worker B snapshot").await;

    // ---- Phase 3: release A, let A fail and write gen=1 -------------------
    std::fs::write(&proceed_a, b"go").unwrap();
    let out_a = worker_a.join().await;
    assert_eq!(out_a.result, "denied", "worker A is denied");
    assert_eq!(out_a.launches, 1, "worker A opens exactly one browser");

    // ---- Phase 4: B adopts (does NOT re-write the sidecar) ----------------
    let out_b = worker_b.join().await;
    assert_eq!(
        out_b.result, "denied",
        "worker B adopts A's denial — it does not open a second browser"
    );
    assert_eq!(
        out_b.launches, 0,
        "worker B adopts without opening a browser"
    );

    // ---- Phase 5: discriminating generation check -------------------------
    // With the fix: sidecar gen == 1 (B did not re-write).
    // Mutation check: restore the deleted `write_attempt` at the adoption site
    // → B writes gen=2 → this assertion FAILS.
    let sidecar = attempt_sidecar_path(&cfg, cache.path());
    let raw = std::fs::read(&sidecar).expect("attempt sidecar written by A");
    let record: serde_json::Value = serde_json::from_slice(&raw).expect("sidecar parses as JSON");
    assert_eq!(
        record.get("generation").and_then(|v| v.as_u64()),
        Some(1),
        "adopter B must not advance the sidecar generation (gen must stay at 1, not 2)"
    );

    // ---- Phase 6: C runs its own attempt ----------------------------------
    // C arrives after A's failure. C's snapshot equals the on-disk generation
    // (1 with fix, 2 with mutation). Either way C sees no advance and runs its
    // own browser flow. But the sidecar check above already catches the
    // mutation; C proves the end-to-end behaviour.
    let worker_c = spawn_worker(&cfg, cache.path(), "userinitiated", "approve", "c", &[]);
    let out_c = worker_c.join().await;
    assert_eq!(
        out_c.result, "ok",
        "worker C (fresh arrival after A's failure) runs its own flow and succeeds"
    );
    assert_eq!(
        out_c.launches, 1,
        "worker C opens its own browser — not inherited from A or B"
    );
    assert_eq!(
        stub.code_grants.load(Ordering::SeqCst),
        1,
        "exactly one code exchange — C's own approval (A was denied; B adopted without exchange)"
    );
}

// ---- cross-process: a waiter with a different rejected must not inherit ----
//
// Cross-process mirror of the in-process test above: process A carries
// `rejected = "X"` and the refresh stickily re-issues "X" → A's attempt
// records RefreshRejected with `rejected_digest = sha256("X")`. Process B
// waits on the lock with `rejected = "Y"` (different). When B acquires the
// lock and reads the attempt record, the digest mismatch causes B to run its
// own attempt rather than adopt A's failure — B's refresh gets "X", which is
// valid for B, so B succeeds.
//
// Ordering is established with deterministic markers and the in-process stub
// gate, not timing:
//   1. A spawns (headless, rejected="X"). The stub holds A's refresh response
//      until the parent calls `gate.release()`.
//   2. Parent waits for `gate.wait_for_request()` — proves A has acquired the
//      lock and is mid-refresh (the request arrived at the stub).
//   3. Parent spawns B (headless, rejected="Y", SNAPSHOT_MARKER=b.snapshot).
//   4. Parent waits for B's snapshot marker — proves B has snapshotted gen=0
//      and is queued on the lock.
//   5. Parent calls `gate.release()`: stub returns "X" to A. A finishes with
//      RefreshRejected(digest(X)), writes sidecar gen=1, releases lock.
//   6. B acquires: gen=1 > snap=0, digest(Y) ≠ digest(X) → B runs its own
//      refresh → gets "X" → Ok("X").
//
// Mutation check (no digest gating): B adopts A's RefreshRejected →
// refresh_grants stays at 1 → `refresh_grants == 2` assertion FAILS.

#[cfg(unix)]
#[tokio::test]
async fn test_crossprocess_waiter_with_different_rejected_does_not_adopt_leaders_failure() {
    // Stub stickily returns "X" but holds each response until released.
    let (stub, gate) = spawn_stub_with_held_refresh(HeldRefreshResponse::Sticky("X")).await;
    let cache = TempDir::new().unwrap();
    let cfg = config(&stub, "/disco/a", cache.path());

    // Seed a token entry so both workers have a refresh token to exercise.
    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "expired-seed",
            "refresh_token": "live-refresh",
            "expires_at": 1u64,
        }),
    );

    let result_a = cache.path().join("a.result.json");
    let result_b = cache.path().join("b.result.json");
    let snapshot_b = cache.path().join("b.snapshot");

    // ---- Phase 1: spawn A. A will acquire the lock and immediately call the
    //              stub's refresh endpoint; the stub holds the response.
    let mut cmd_a = tokio::process::Command::new(env!("CARGO_BIN_EXE_auth-worker"));
    cmd_a
        .env("AUTH_WORKER_DISCOVERY_URL", &cfg.discovery_url)
        .env("AUTH_WORKER_CACHE_DIR", cache.path())
        .env("AUTH_WORKER_NAMESPACE", &cfg.cache_namespace)
        .env("AUTH_WORKER_CLIENT_ID", &cfg.client_id)
        .env("AUTH_WORKER_SCOPES", cfg.scopes.join(","))
        .env("AUTH_WORKER_INTENT", "headless")
        .env("AUTH_WORKER_SCRIPT", "failopen") // headless never browses
        .env("AUTH_WORKER_REJECTED", "X")
        .env("AUTH_WORKER_RESULT", &result_a)
        .kill_on_drop(true);

    let child_a = cmd_a.spawn().expect("spawn worker A");

    // ---- Phase 2: wait until the stub has received A's refresh request.
    //              This is an in-process await — no polling or timing needed.
    //              Once the stub is holding A's request, A owns the lock.
    gate.wait_for_request().await;

    // ---- Phase 3: spawn B with SNAPSHOT_MARKER.
    let mut cmd_b = tokio::process::Command::new(env!("CARGO_BIN_EXE_auth-worker"));
    cmd_b
        .env("AUTH_WORKER_DISCOVERY_URL", &cfg.discovery_url)
        .env("AUTH_WORKER_CACHE_DIR", cache.path())
        .env("AUTH_WORKER_NAMESPACE", &cfg.cache_namespace)
        .env("AUTH_WORKER_CLIENT_ID", &cfg.client_id)
        .env("AUTH_WORKER_SCOPES", cfg.scopes.join(","))
        .env("AUTH_WORKER_INTENT", "headless")
        .env("AUTH_WORKER_SCRIPT", "failopen")
        .env("AUTH_WORKER_REJECTED", "Y")
        .env("AUTH_WORKER_RESULT", &result_b)
        .env("AUTH_WORKER_SNAPSHOT_MARKER", &snapshot_b)
        .kill_on_drop(true);

    let child_b = cmd_b.spawn().expect("spawn worker B");

    // ---- Phase 4: wait for B's snapshot marker. The tracing layer in B fires
    //              this after B snapshots gen=0 and before it waits for the
    //              lock — proves B holds snap=0 and is queued behind A.
    wait_for_marker(&snapshot_b, "worker B snapshot").await;

    // ---- Phase 5: release A. Stub returns "X"; A records RefreshRejected
    //              with digest(X), advances gen to 1, releases the lock.
    gate.release();

    let worker_a = Worker {
        child: child_a,
        result_path: result_a,
    };
    let worker_b = Worker {
        child: child_b,
        result_path: result_b,
    };
    let (out_a, out_b) = tokio::join!(worker_a.join(), worker_b.join());

    // A (rejected=X): refresh returns "X" → RefreshRejected.
    // Sidecar: gen=1, result=refresh_rejected, rejected_digest=sha256("X").
    assert_eq!(
        out_a.result, "refresh_rejected",
        "worker A (rejected=X) must get RefreshRejected"
    );
    // B (rejected=Y): gen=1 > snap=0, digest(Y) ≠ digest(X) → B runs its
    // own refresh. B's refresh returns "X"; finish(rejected=Y, token=X) → Ok.
    assert_eq!(
        out_b.result, "ok",
        "worker B (rejected=Y) must succeed after rerunning — not adopt A's RefreshRejected"
    );
    // Mutation check (r8 shape, no digest gate): B adopts → refresh_grants
    // stays 1. With the digest fix: B reruns → refresh_grants = 2.
    assert_eq!(
        stub.refresh_grants.load(Ordering::SeqCst),
        2,
        "both workers run their own refresh — digest mismatch prevented adoption"
    );
}

// ---- P1-3 non-Unix read path disabled -----------------------------------
//
// On non-Unix platforms (Windows) token files written by older builds with
// default ACLs should not be consumed by new builds. `read_private_cache`
// returns an error on non-Unix (and opportunistically removes the legacy
// file), so `read_cache` yields `None` and the source behaves as if no
// cached token exists — memory-only cache on non-Unix.
//
// This test uses a cfg-gated stub: on Unix it only exercises the Unix read
// path (as a sanity check); the Windows behavior is proved by the
// `#[cfg(not(unix))]` branch of `read_private_cache` and verified by the
// Windows CI build + manual testing on the Windows runner. The test is written
// to compile on all platforms and asserts the platform-appropriate invariant.

#[tokio::test]
async fn test_non_unix_does_not_serve_legacy_on_disk_token() {
    // Seed a token that would be served from disk on Unix (unexpired, valid).
    let stub = spawn_stub(false).await; // fresh token on refresh/browser
    let cache = TempDir::new().unwrap();
    let opener = ScriptedOpener::new(Script::Approve);
    let cfg = config(&stub, "/disco/a", cache.path());

    seed_cache(
        &cfg,
        cache.path(),
        json!({
            "access_token": "legacy-windows-token",
            "refresh_token": "legacy-refresh",
            "expires_at": future_secs(),
        }),
    );

    let src = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();

    #[cfg(unix)]
    {
        // On Unix the cache is read and served directly from disk — this is the
        // expected behavior on a secured platform.
        let token = src
            .acquire_with_intent(AuthIntent::Headless, None)
            .await
            .expect("Unix serves the seeded token from disk");
        assert_eq!(token, "legacy-windows-token", "Unix: disk token served");
        assert_eq!(
            stub.refresh_grants.load(Ordering::SeqCst),
            0,
            "Unix: no refresh — the disk token was served directly"
        );
        // The seeded file is still on disk (not removed on Unix).
        assert!(
            cache_file_path(&cfg, cache.path()).exists(),
            "Unix: the cache file is preserved"
        );
    }

    #[cfg(not(unix))]
    {
        // On non-Unix `read_private_cache` refuses to read the legacy file and
        // attempts to remove it. Construction and bearer() behave as if no cache
        // exists — the source falls through to a browser flow.
        let token = src
            .acquire_with_intent(AuthIntent::Auto, None)
            .await
            .expect("non-Unix: browser flow succeeds (no disk token served)");
        assert_ne!(
            token, "legacy-windows-token",
            "non-Unix: legacy token must not be served from disk"
        );
        assert_eq!(
            stub.code_grants.load(Ordering::SeqCst),
            1,
            "non-Unix: browser flow ran — disk token was not served"
        );
        assert_eq!(
            stub.refresh_grants.load(Ordering::SeqCst),
            0,
            "non-Unix: no refresh grant — the source went straight to the browser flow"
        );
        // The legacy file should have been removed by read_private_cache.
        assert!(
            !cache_file_path(&cfg, cache.path()).exists(),
            "non-Unix: legacy cache file is removed by read_private_cache"
        );
        // No new token file was written (persist is a no-op on non-Unix).
        // (The token is held in memory only.)
        assert!(
            !cache_file_path(&cfg, cache.path()).exists(),
            "non-Unix: no new cache file created (memory-only)"
        );
    }
}
