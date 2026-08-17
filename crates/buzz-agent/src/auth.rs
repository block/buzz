//! Token sources for the LLM transport layer.
//!
//! [`TokenSource`] decouples request auth from `Config::api_key`: providers
//! can supply a static string ([`StaticTokenSource`]) or a refreshable OAuth
//! 2.0 PKCE engine ([`PkceOAuthTokenSource`]). Engines own their own cache
//! and refresh logic; the [`Llm`] just asks for a bearer per request.
//!
//! The PKCE engine implements RFC 6749 + RFC 7636 with on-disk token
//! caching keyed by `sha256(discovery_url|client_id|scopes)`. It's the
//! same shape goose uses for Databricks, but we own the wire format and
//! cache directory so the two are independently upgradable.
//!
//! First-use (cache empty) requires a browser: the engine opens
//! `authorization_endpoint` in `webbrowser`, listens on `127.0.0.1:0`,
//! captures the redirect, and exchanges the code for a token. Subsequent
//! calls hit the cache and silently refresh when expired.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine;
use fs2::FileExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use tokio::sync::{watch, Mutex};

use crate::types::AgentError;

/// Buffer before `expires_at` to consider a cached token "still good".
/// Keeps us off the cliff if the clock or the server's clock drifts.
const TOKEN_REFRESH_LEEWAY: Duration = Duration::from_secs(60);

/// Wall-clock budget for the interactive browser dance. Goose uses 60s.
/// We match: any longer and the user has gone to lunch.
const BROWSER_AUTH_TIMEOUT: Duration = Duration::from_secs(60);

/// Per-request network timeout for every OAuth HTTP call (discovery, refresh
/// grant, code exchange). Without this, a hung provider connection would stall
/// the caller — and, worse, stall every same-key caller waiting on the
/// cross-process lock this holder owns.
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Longest an in-flight auth attempt can legitimately run: cold discovery
/// (`30s`) + browser wait (`60s`) + code exchange (`30s`), plus a failed
/// refresh (`30s`) ahead of the browser. Rounded to `150s`. A waiter derives
/// its lock-wait bound from this so it never times out ahead of a healthy
/// holder.
const AUTH_ATTEMPT_DEADLINE: Duration = Duration::from_secs(150);

/// How long a same-key caller waits to acquire the cross-process lock before
/// giving up with [`AuthError::LockTimeout`]. Deliberately longer than
/// [`AUTH_ATTEMPT_DEADLINE`] so a waiter outlasts any legitimate holder rather
/// than timing out mid-flow.
const LOCK_WAIT_TIMEOUT: Duration = Duration::from_secs(165);

/// Poll interval for deadline-aware lock acquisition. `try_lock` is
/// non-blocking, so we sleep between attempts rather than blocking a worker.
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How long a failed interactive (browser) attempt suppresses automatic
/// re-launch for the same key. Long enough that a spurned dropdown does not
/// re-pop a browser on the next debounced refresh, short enough that a user
/// who fixes the problem is not locked out.
const COOLDOWN_DURATION: Duration = Duration::from_secs(300);

/// Why an auth acquisition wants a token, which decides whether it may open a
/// browser and whether it honors a cooldown.
///
/// - [`Auto`](Self::Auto): passive Desktop discovery (create/edit/defaults/
///   onboarding). May open a browser, but honors an unexpired cooldown and
///   returns its recorded outcome instead of re-launching.
/// - [`UserInitiated`](Self::UserInitiated): an explicit human action — the
///   saved-agent model picker or `buzz-agent auth databricks`. May open a
///   browser and *bypasses* the cooldown (the user asked for it now).
/// - [`Headless`](Self::Headless): managed-runtime inference and provider
///   preflight. Never opens a browser; may consume another attempt's cached
///   success but never becomes the initiator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthIntent {
    Auto,
    UserInitiated,
    Headless,
}

impl AuthIntent {
    /// `true` for the intents permitted to open a browser.
    fn may_open_browser(self) -> bool {
        matches!(self, Self::Auto | Self::UserInitiated)
    }

    /// `true` for the one intent that honors a recorded cooldown on read.
    fn honors_cooldown(self) -> bool {
        matches!(self, Self::Auto)
    }
}

/// Typed result of an auth acquisition. `Ok` carries the bearer; the error
/// arm classifies *why* no token was produced so callers (and, in Phase 2, the
/// Tauri boundary) can branch on a stable code instead of matching display
/// text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// No cached token, no refresh grant, and the caller may not open a
    /// browser (`Headless`).
    NoCredential,
    /// The user (or provider) rejected the browser authorization.
    Denied,
    /// The browser flow was not completed within [`BROWSER_AUTH_TIMEOUT`].
    TimedOut,
    /// Every browser-launch strategy failed, so the flow never started.
    BrowserOpenFailed,
    /// An OAuth network call (discovery/refresh/exchange) could not reach the
    /// provider or timed out.
    NetworkUnavailable,
    /// A refresh-token grant was rejected (dead/rotated refresh token) and the
    /// caller may not fall back to a browser.
    RefreshRejected,
    /// The authorization-code exchange itself was rejected by the token
    /// endpoint (distinct from a refresh rejection).
    ExchangeFailed,
    /// Could not acquire the cross-process auth lock within
    /// [`LOCK_WAIT_TIMEOUT`].
    LockTimeout,
}

impl AuthError {
    /// Stable machine-readable code. Phase 2 serializes this across the Tauri
    /// boundary (the `project_git_merge_error` `{code, message}` precedent) so
    /// the Desktop formatter switches on the code, never on display text.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoCredential => "no_credential",
            Self::Denied => "denied",
            Self::TimedOut => "timed_out",
            Self::BrowserOpenFailed => "browser_open_failed",
            Self::NetworkUnavailable => "network_unavailable",
            Self::RefreshRejected => "refresh_rejected",
            Self::ExchangeFailed => "exchange_failed",
            Self::LockTimeout => "lock_timeout",
        }
    }

    /// `true` for the browser-attempt outcomes worth recording in the cooldown
    /// sidecar — the failures that would otherwise re-pop a browser on the
    /// next automatic attempt. Non-browser failures (no credential, refresh
    /// rejection, lock timeout, network) are not recorded.
    fn is_cooldown_worthy(&self) -> bool {
        matches!(
            self,
            Self::Denied | Self::TimedOut | Self::BrowserOpenFailed | Self::ExchangeFailed
        )
    }

    /// Reconstruct a recorded outcome from its [`code`](Self::code). Only the
    /// cooldown-worthy variants round-trip; any other code (a forward-compat
    /// sidecar written by a newer buzz-agent) yields `None`, so a stale or
    /// unrecognized record is treated as "no cooldown" rather than a hard
    /// failure.
    fn from_code(code: &str) -> Option<Self> {
        match code {
            "denied" => Some(Self::Denied),
            "timed_out" => Some(Self::TimedOut),
            "browser_open_failed" => Some(Self::BrowserOpenFailed),
            "exchange_failed" => Some(Self::ExchangeFailed),
            _ => None,
        }
    }

    fn message(&self) -> String {
        match self {
            Self::NoCredential => {
                "no cached Databricks token; run `buzz-agent auth databricks` first".into()
            }
            Self::Denied => "Databricks authorization was denied".into(),
            Self::TimedOut => "Databricks browser authorization timed out".into(),
            Self::BrowserOpenFailed => "could not open a browser for Databricks sign-in".into(),
            Self::NetworkUnavailable => "could not reach Databricks to authenticate".into(),
            Self::RefreshRejected => "Databricks rejected the refresh token; sign in again".into(),
            Self::ExchangeFailed => "Databricks rejected the authorization code".into(),
            Self::LockTimeout => "timed out waiting for a concurrent Databricks sign-in".into(),
        }
    }
}

impl From<AuthError> for AgentError {
    /// Map a typed auth failure onto the crate error the [`TokenSource`] trait
    /// returns. Auth-decision failures become [`AgentError::LlmAuth`] so the
    /// caller's retry loop stops instead of hammering a rejected credential;
    /// purely infrastructural failures (network, lock contention) become
    /// [`AgentError::Llm`], matching the pre-coordinator classification of a
    /// discovery/network error.
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::NetworkUnavailable | AuthError::LockTimeout => AgentError::Llm(e.message()),
            AuthError::NoCredential
            | AuthError::Denied
            | AuthError::TimedOut
            | AuthError::BrowserOpenFailed
            | AuthError::RefreshRejected
            | AuthError::ExchangeFailed => AgentError::LlmAuth(e.message()),
        }
    }
}

/// Opens a URL for the interactive browser step. Injected so the PKCE
/// continuation (callback listener, verifier, timeout) stays alive across the
/// launch: the coordinator calls this *while* the localhost listener is
/// bound, so a launch failure never leaves a returned URL pointing at a torn
/// down listener. Desktop (Phase 2) supplies the Tauri opener; the CLI uses
/// [`DefaultBrowserOpener`], which prints the URL and opens the system
/// browser.
pub trait BrowserOpener: Send + Sync {
    /// Attempt to present `url` to the user. Returning `Err` means every
    /// launch strategy for this opener failed; the coordinator then reports
    /// [`AuthError::BrowserOpenFailed`] without waiting on a listener nobody
    /// will reach.
    fn open(&self, url: &str) -> Result<(), String>;
}

/// Default opener: print the URL (so a user on a headless box can copy it)
/// and open the system browser. Printing is itself a launch strategy, so this
/// never reports failure — the URL is always visible to the waiting user.
pub struct DefaultBrowserOpener;

impl BrowserOpener for DefaultBrowserOpener {
    fn open(&self, url: &str) -> Result<(), String> {
        eprintln!("Opening browser for authentication. If it doesn't open, visit:\n  {url}");
        let _ = webbrowser::open(url);
        Ok(())
    }
}

/// Asynchronous source of a bearer token. The [`Llm`] calls this per
/// request, so impls are expected to be cheap on the cache-hit path.
#[async_trait]
pub trait TokenSource: Send + Sync {
    async fn bearer(&self) -> Result<String, AgentError>;

    /// Return a bearer token from cache or refresh, **never** opening a browser.
    ///
    /// The default delegates to [`bearer`](Self::bearer) — correct for token
    /// sources (e.g. static API keys) that can never trigger a browser flow.
    /// [`PkceOAuthTokenSource`] overrides this to stop before the browser step.
    async fn bearer_no_browser(&self) -> Result<String, AgentError> {
        self.bearer().await
    }

    /// Force a fresh bearer after the server rejected the current one (401).
    ///
    /// `rejected` is the exact access token that just got the 401. Unlike
    /// [`bearer`](Self::bearer), which trusts the local expiry clock, this is
    /// driven by the server's verdict: the cached token looked valid to us
    /// (well within its local expiry) but the provider rejected it — clock
    /// skew, server-side revocation, or a node that never saw it. The clock
    /// therefore can't decide whether to refresh; the caller passes the
    /// rejected token so the impl can refresh unless a concurrent caller has
    /// *already* replaced it. Implementations must obtain a new token without
    /// any interactive step, so a headless harness never hangs. The default
    /// returns the existing bearer — correct for sources that can't refresh
    /// (a static key); the caller's retry then fails terminally rather than
    /// looping.
    async fn refresh_now(&self, _rejected: &str) -> Result<String, AgentError> {
        self.bearer().await
    }
}

/// A token that never changes for the life of the process.
pub struct StaticTokenSource(String);

impl StaticTokenSource {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }
}

#[async_trait]
impl TokenSource for StaticTokenSource {
    async fn bearer(&self) -> Result<String, AgentError> {
        Ok(self.0.clone())
    }
}

/// Static config for an OAuth 2.0 Authorization Code + PKCE provider.
///
/// The `discovery_url` must return a JSON document with at least
/// `authorization_endpoint` and `token_endpoint` (RFC 8414). The
/// `cache_namespace` is the directory under `~/.config/buzz-agent/oauth/`
/// the token JSON lives in — separates providers' caches cleanly.
#[derive(Debug, Clone)]
pub struct PkceOAuthConfig {
    pub discovery_url: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub cache_namespace: String,
    /// When `Some`, the engine writes tokens here instead of
    /// `~/.config/buzz-agent/oauth/<cache_namespace>/`. Production code
    /// leaves this `None`. Integration tests use it to avoid stomping on
    /// a shared `$HOME` when running in parallel.
    pub cache_dir_override: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct CachedToken {
    access_token: String,
    refresh_token: Option<String>,
    /// Unix seconds. `None` means the server didn't advertise an expiry;
    /// we use it without checking and rely on refresh on 401.
    expires_at: Option<u64>,
}

#[derive(Debug, Clone)]
struct OidcEndpoints {
    authorization_endpoint: String,
    token_endpoint: String,
}

/// Typed result of a refresh-token grant, so the coordinator can separate an
/// actual credential rejection from a transient fault.
///
/// - [`Refreshed`](Self::Refreshed): a fresh token — success.
/// - [`Rejected`](Self::Rejected): the token endpoint returned an
///   `invalid_grant` error (dead/rotated refresh token). This is the only
///   outcome that becomes [`AuthError::RefreshRejected`] for `Headless` or
///   drives a browser fallback for interactive intents.
/// - [`Network`](Self::Network): transport error, timeout, 5xx, any 4xx that
///   is not `invalid_grant` (e.g. `invalid_request`, `invalid_client`, 429),
///   an unparseable error body, or an undecodable/malformed success body —
///   infrastructural or misconfiguration, never a credential decision, so it
///   surfaces as [`AuthError::NetworkUnavailable`] and never pops a browser.
enum RefreshOutcome {
    Refreshed(CachedToken),
    Rejected,
    Network,
}

/// PKCE OAuth token source with on-disk refresh cache.
///
/// First call:
///   1. Loads from cache if present and unexpired.
///   2. Otherwise tries `refresh_token` if cached.
///   3. Otherwise runs the full browser flow.
///
/// Subsequent calls hit an in-memory copy of the cached token and only
/// touch disk/network if the access token is past `expires_at`.
pub struct PkceOAuthTokenSource {
    cfg: PkceOAuthConfig,
    http: Client,
    cache_path: PathBuf,
    /// Injected browser launcher, called inside [`browser_pkce_flow`] while the
    /// localhost listener is live. Production uses [`DefaultBrowserOpener`];
    /// Phase 2 supplies the Tauri opener.
    opener: Arc<dyn BrowserOpener>,
    /// In-memory single-flight *and* fast-path cache. The cross-process file
    /// lock serializes slow-path work; this cell keeps the fast path off disk
    /// during a turn and off the lock entirely.
    state: Mutex<Option<CachedToken>>,
}

impl PkceOAuthTokenSource {
    /// Construct with the default browser opener (prints the URL and opens the
    /// system browser). This is the signature every production call site uses.
    pub fn new(cfg: PkceOAuthConfig) -> Result<Arc<Self>, AgentError> {
        Self::new_with(cfg, Arc::new(DefaultBrowserOpener))
    }

    /// Construct with an injected [`BrowserOpener`]. Tests substitute a
    /// recording/failing opener to exercise the browser branch without a real
    /// window; Phase 2 Desktop injects the Tauri opener.
    pub fn new_with(
        cfg: PkceOAuthConfig,
        opener: Arc<dyn BrowserOpener>,
    ) -> Result<Arc<Self>, AgentError> {
        Self::new_with_http_timeout(cfg, opener, HTTP_REQUEST_TIMEOUT)
    }

    /// Construct with an injected opener *and* an explicit per-request HTTP
    /// timeout. Only the refresh-timeout integration test passes the timeout
    /// argument: it drives a hung token endpoint against a short bound so the
    /// per-request timeout classification (`NetworkUnavailable`, never
    /// `RefreshRejected`) is exercised in real time. A paused-clock test can't
    /// do this — tokio auto-advances into the timer while the real loopback
    /// discovery call is still in flight, tripping the timeout on the wrong
    /// request. Every production and other-test path goes through
    /// [`new`](Self::new) or [`new_with`](Self::new_with) at the default
    /// [`HTTP_REQUEST_TIMEOUT`].
    pub fn new_with_http_timeout(
        cfg: PkceOAuthConfig,
        opener: Arc<dyn BrowserOpener>,
        http_timeout: Duration,
    ) -> Result<Arc<Self>, AgentError> {
        let cache_path = cache_path_for(&cfg)?;
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AgentError::Llm(format!("oauth cache dir {parent:?}: {e}")))?;
        }
        // Every OAuth HTTP call inherits this timeout so a hung provider can
        // never stall the caller — nor the same-key callers waiting on the
        // cross-process lock this holder owns. Construction is fallible, so a
        // build failure propagates rather than silently falling back to an
        // untimed client — an untimed client would restore exactly the
        // unbounded-HTTP-under-lock failure the timeout exists to prevent.
        let http = Client::builder()
            .timeout(http_timeout)
            .build()
            .map_err(|e| AgentError::Llm(format!("oauth http client: {e}")))?;
        let initial = read_cache(&cache_path);
        Ok(Arc::new(Self {
            cfg,
            http,
            cache_path,
            opener,
            state: Mutex::new(initial),
        }))
    }

    /// Path of the cross-process advisory lock file guarding slow-path auth
    /// for this cache key. Co-located with the cache so it shares the
    /// per-key directory and `$HOME` override.
    fn lock_path(&self) -> PathBuf {
        append_ext(&self.cache_path, "lock")
    }

    /// Path of the cooldown sidecar recording the last browser-attempt
    /// failure for this cache key.
    fn cooldown_path(&self) -> PathBuf {
        append_ext(&self.cache_path, "cooldown")
    }

    /// Discover authorization + token endpoints from the well-known URL.
    async fn endpoints(&self) -> Result<OidcEndpoints, AgentError> {
        let v: Value = self
            .http
            .get(&self.cfg.discovery_url)
            .send()
            .await
            .map_err(|e| AgentError::Llm(format!("oauth discovery: {e}")))?
            .error_for_status()
            .map_err(|e| AgentError::Llm(format!("oauth discovery status: {e}")))?
            .json()
            .await
            .map_err(|e| AgentError::Llm(format!("oauth discovery json: {e}")))?;
        let auth = v
            .get("authorization_endpoint")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AgentError::Llm("oauth discovery: authorization_endpoint missing".into())
            })?
            .to_string();
        let token = v
            .get("token_endpoint")
            .and_then(Value::as_str)
            .ok_or_else(|| AgentError::Llm("oauth discovery: token_endpoint missing".into()))?
            .to_string();
        Ok(OidcEndpoints {
            authorization_endpoint: auth,
            token_endpoint: token,
        })
    }

    /// Persist a token to disk and the in-memory cell.
    ///
    /// The cache holds both the access and refresh tokens, so the on-disk
    /// file is written owner-only (`0o600` on Unix) via an atomic
    /// inode-swapping rename — see [`write_private_cache`].
    fn save(&self, state: &mut Option<CachedToken>, token: CachedToken) -> Result<(), AgentError> {
        let body = serde_json::to_vec_pretty(&token)
            .map_err(|e| AgentError::Llm(format!("oauth cache serialize: {e}")))?;
        write_private_cache(&self.cache_path, &body).map_err(|e| {
            AgentError::Llm(format!("oauth cache write {:?}: {e}", self.cache_path))
        })?;
        *state = Some(token);
        Ok(())
    }

    /// Exchange a refresh token for a fresh access token.
    ///
    /// The outcome is typed so the caller can tell an actual credential
    /// rejection apart from a transient fault. Only a token-endpoint rejection
    /// of the grant itself (a 4xx `invalid_grant`-class response) is a dead
    /// refresh token; a transport failure, timeout, 5xx, or an
    /// undecodable/malformed response is infrastructural and must never be
    /// mistaken for a credential decision (it would otherwise pop a browser or
    /// return `RefreshRejected` when nothing was actually rejected).
    async fn refresh(&self, endpoints: &OidcEndpoints, refresh_token: &str) -> RefreshOutcome {
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &self.cfg.client_id),
        ];
        let resp = match self
            .http
            .post(&endpoints.token_endpoint)
            .form(&params)
            .send()
            .await
        {
            Ok(resp) => resp,
            // Transport error or the per-request timeout elapsed: no verdict
            // from the provider, so this is infrastructural, not a rejection.
            Err(e) => {
                tracing::warn!(error = %e, "oauth refresh transport failure");
                return RefreshOutcome::Network;
            }
        };
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            // Per RFC 6749 §5.2 only `error == "invalid_grant"` means the
            // refresh token itself is dead (expired/revoked) — the one failure
            // a browser sign-in can repair. Every other 4xx (`invalid_request`,
            // `invalid_client`, `unsupported_grant_type`, `invalid_scope`, 408,
            // 429, …), an unparseable error body, and all 5xx are
            // infrastructural or misconfiguration: a browser can't fix them, so
            // they stay in the non-credential bucket and surface as
            // `NetworkUnavailable` without ever popping a browser.
            if status.is_client_error()
                && serde_json::from_str::<Value>(&body)
                    .ok()
                    .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
                    .as_deref()
                    == Some("invalid_grant")
            {
                tracing::warn!(status = %status, body = %body, "oauth refresh grant rejected");
                return RefreshOutcome::Rejected;
            }
            tracing::warn!(status = %status, body = %body, "oauth refresh not repairable by browser");
            return RefreshOutcome::Network;
        }
        let v: Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "oauth refresh response decode failure");
                return RefreshOutcome::Network;
            }
        };
        match token_from_response(&v, Some(refresh_token)) {
            Ok(token) => RefreshOutcome::Refreshed(token),
            Err(e) => {
                tracing::warn!(error = %e, "oauth refresh response missing access_token");
                RefreshOutcome::Network
            }
        }
    }

    /// Run the full browser-mediated Authorization Code + PKCE flow and cache
    /// the result. Routes through the coordinator as a [`UserInitiated`]
    /// acquisition: it may open a browser, bypasses (and clears) any cooldown,
    /// and single-flights with concurrent callers on the cross-process lock. A
    /// still-valid cached token short-circuits to success without re-prompting.
    ///
    /// This is the no-rejected convenience: it trusts the local expiry clock,
    /// so a not-yet-expired cached token is accepted. When the caller already
    /// knows the cached bearer was rejected by the server (a 401), it must use
    /// [`acquire_with_intent`](Self::acquire_with_intent) with `rejected` set
    /// so the stale-but-fresh token can't short-circuit the sign-in.
    ///
    /// [`UserInitiated`]: AuthIntent::UserInitiated
    pub async fn interactive_login(&self) -> Result<(), AgentError> {
        self.acquire(AuthIntent::UserInitiated, None).await?;
        Ok(())
    }

    /// Public entry for passive Desktop discovery and the saved-model picker
    /// (Phase 2): acquire a bearer under an explicit [`AuthIntent`], returning
    /// the typed [`AuthError`] so the caller can branch on a stable `code`
    /// rather than display text. The [`TokenSource`] trait methods wrap this
    /// and flatten the error into [`AgentError`].
    ///
    /// `rejected` carries the exact access token the provider just 401'd, if
    /// any. With `rejected = None` a locally-fresh cached token is a hit (the
    /// normal discovery path). With `rejected = Some(t)` the expiry clock is
    /// untrustworthy — the rejected token looked fresh — so a cached token
    /// equal to `t` is *not* a hit: the acquisition refreshes, and for `Auto`
    /// or `UserInitiated` falls through to a browser when the refresh grant is
    /// dead. This is what lets the saved-picker recovery path say "this
    /// locally-fresh bearer was just rejected — replace it" instead of
    /// re-returning the dead token, which `refresh_now`'s hardcoded
    /// [`Headless`](AuthIntent::Headless) can never escalate to a browser.
    pub async fn acquire_with_intent(
        &self,
        intent: AuthIntent,
        rejected: Option<&str>,
    ) -> Result<String, AuthError> {
        self.acquire(intent, rejected).await
    }

    /// Return a usable cached bearer, applying the identity rule for a
    /// 401-driven acquisition.
    ///
    /// `rejected = None` (normal): a not-yet-expired cached token is a hit.
    /// `rejected = Some(t)`: the expiry clock is untrustworthy — the rejected
    /// token looked locally fresh — so a hit requires the cached token to
    /// *differ* from `t`, meaning a sibling already replaced it. Checks the
    /// in-memory cell first, then re-reads disk (a sibling process may have
    /// written a newer token) and adopts it into the cell on a hit.
    fn cached_hit(
        &self,
        state: &mut Option<CachedToken>,
        rejected: Option<&str>,
    ) -> Option<String> {
        let usable = |tok: &CachedToken| match rejected {
            Some(r) => tok.access_token != r,
            None => !is_expired(tok),
        };
        if let Some(tok) = state.as_ref() {
            if usable(tok) {
                return Some(tok.access_token.clone());
            }
        }
        if let Some(disk) = read_cache(&self.cache_path) {
            if usable(&disk) {
                let bearer = disk.access_token.clone();
                *state = Some(disk);
                return Some(bearer);
            }
        }
        None
    }

    /// Discover OIDC endpoints once per flow, memoizing into `slot` so the
    /// refresh and browser branches share a single discovery call. A discovery
    /// failure (unreachable URL or malformed document) maps to
    /// [`AuthError::NetworkUnavailable`] — the infrastructural bucket, so the
    /// caller's retry loop treats it as transient rather than as an auth
    /// decision.
    async fn discover<'a>(
        &self,
        slot: &'a mut Option<OidcEndpoints>,
    ) -> Result<&'a OidcEndpoints, AuthError> {
        if slot.is_none() {
            let eps = self
                .endpoints()
                .await
                .map_err(|_| AuthError::NetworkUnavailable)?;
            *slot = Some(eps);
        }
        Ok(slot.as_ref().expect("endpoints just populated"))
    }

    /// The single acquisition entry point behind every [`TokenSource`] method.
    ///
    /// `intent` decides browser and cooldown policy; `rejected` (`Some` only on
    /// a 401-driven refresh) switches cache checks from clock-based to
    /// identity-based. The fast path returns a usable cached token without
    /// touching the lock or the network. Otherwise the slow path serializes
    /// every same-key caller — in this process *and* across processes — on the
    /// cross-process advisory lock, so concurrent dialogs coalesce onto one
    /// refresh/browser flow instead of racing browsers.
    async fn acquire(
        &self,
        intent: AuthIntent,
        rejected: Option<&str>,
    ) -> Result<String, AuthError> {
        // Fast path: no lock, no network. `try_lock` rather than `lock().await`
        // so a caller arriving while a leader holds `state` across its browser
        // flow does not block here — it falls through to the in-process
        // registry below and joins the leader instead of waiting out the whole
        // flow and then racing in as a second leader. A cache hit is still
        // served without the file lock; a miss (or contention) coalesces.
        {
            if let Ok(mut state) = self.state.try_lock() {
                if let Some(hit) = self.cached_hit(&mut state, rejected) {
                    return Ok(hit);
                }
            }
        }

        // In-process single-flight (see [`INFLIGHT`]). Keyed by (lock path,
        // browser capability): browser-capable callers coalesce with each
        // other, so a caller that was already waiting when the leader's attempt
        // was in flight shares the leader's result instead of taking the lock
        // after it and launching a second browser. A `Headless` caller never
        // shares a browser-capable slot (and vice versa), so a racing inference
        // call is neither handed an interactive failure nor able to deny an
        // explicit sign-in its browser — those two intents still coordinate
        // only through the cross-process file lock.
        let key: InflightKey = (self.lock_path(), intent.may_open_browser());
        let (slot, is_leader) = {
            let mut reg = inflight_registry();
            match reg.get(&key) {
                Some(existing) => (existing.clone(), false),
                None => {
                    let slot = Arc::new(InflightSlot::new());
                    reg.insert(key.clone(), slot.clone());
                    (slot, true)
                }
            }
        };
        if !is_leader {
            // Pre-existing joiner: observe the leader's outcome.
            return slot.wait().await;
        }

        // Leader: run the real flow, then evict + publish. The guard makes
        // eviction and joiner wake-up happen even if this future is cancelled
        // or panics, so a dropped leader can never wedge its joiners or leave a
        // dead slot that turns later callers into joiners of nothing.
        let guard = LeaderGuard::new(key, slot);
        let result = self.acquire_leader(intent, rejected).await;
        guard.complete(result)
    }

    /// The leader's slow-path body: take the cross-process lock, then run the
    /// bounded acquisition under it. Split out so [`acquire`] can wrap it in
    /// the in-process single-flight without the lock/deadline logic bleeding
    /// into the joiner path.
    async fn acquire_leader(
        &self,
        intent: AuthIntent,
        rejected: Option<&str>,
    ) -> Result<String, AuthError> {
        // Slow path: one flow at a time per cache key. The waiter's deadline
        // exceeds a healthy holder's attempt deadline, so it never gives up on
        // a live holder.
        let deadline = std::time::Instant::now() + LOCK_WAIT_TIMEOUT;
        let _guard = acquire_auth_lock(&self.lock_path(), deadline).await?;

        // Bound the whole locked attempt so a wedged flow can't hold the lock
        // past the waiters' patience. The deadline is passed *into*
        // `acquire_locked` rather than wrapped around it in a cancelling
        // `tokio::time::timeout`: a cancel drops the future at an arbitrary
        // await point, which would skip the cooldown write for a timed-out
        // interactive attempt and let the next `Auto` caller re-pop a browser.
        // Threading the deadline lets every interactive timeout exit through
        // the common outcome writer while the lock is still held.
        let attempt_deadline = std::time::Instant::now() + AUTH_ATTEMPT_DEADLINE;
        self.acquire_locked(intent, rejected, attempt_deadline)
            .await
    }

    /// Slow-path body, run while holding the cross-process auth lock.
    ///
    /// `attempt_deadline` bounds the whole locked flow. Discovery and refresh
    /// are each bounded by the HTTP client's per-request timeout; the browser
    /// flow is wrapped in the *remaining* budget so a total-deadline expiry
    /// during the interactive step surfaces as [`AuthError::TimedOut`] through
    /// the same arm that records the cooldown — never as a cancellation that
    /// drops the guard without writing it.
    async fn acquire_locked(
        &self,
        intent: AuthIntent,
        rejected: Option<&str>,
        attempt_deadline: std::time::Instant,
    ) -> Result<String, AuthError> {
        let mut state = self.state.lock().await;

        // Re-check under the lock: a holder we queued behind may have already
        // produced a token (this process or a sibling wrote the cache).
        if let Some(hit) = self.cached_hit(&mut state, rejected) {
            return Ok(hit);
        }

        // Refresh-token grant, if we have one. Endpoints are discovered lazily
        // here (and reused by the browser branch) so a no-refresh headless
        // failure never depends on reaching the discovery URL.
        let mut endpoints: Option<OidcEndpoints> = None;
        let mut refresh_failed = false;
        if let Some(rt) = state.as_ref().and_then(|t| t.refresh_token.clone()) {
            let eps = self.discover(&mut endpoints).await?;
            match self.refresh(eps, &rt).await {
                RefreshOutcome::Refreshed(fresh) => return self.finish(&mut state, fresh),
                // A transient fault (transport/timeout/5xx/decode) is not a
                // credential decision: never fall through to a browser or
                // report RefreshRejected. A sibling may have written a fresh
                // token while we ran, so honor that first; otherwise this is
                // infrastructural and surfaces as NetworkUnavailable.
                RefreshOutcome::Network => {
                    if let Some(hit) = self.cached_hit(&mut state, rejected) {
                        return Ok(hit);
                    }
                    return Err(AuthError::NetworkUnavailable);
                }
                // The token endpoint rejected the grant: a dead refresh token.
                // A sibling may still have won the race while we ran; if not,
                // fall through to a browser (interactive) or RefreshRejected
                // (headless).
                RefreshOutcome::Rejected => {
                    if let Some(hit) = self.cached_hit(&mut state, rejected) {
                        return Ok(hit);
                    }
                    refresh_failed = true;
                }
            }
        }

        // No token from cache or refresh. Browser or terminal failure.
        if !intent.may_open_browser() {
            return Err(if refresh_failed {
                AuthError::RefreshRejected
            } else {
                AuthError::NoCredential
            });
        }

        let cooldown_path = self.cooldown_path();
        if intent.honors_cooldown() {
            // A recent browser attempt failed; surface its recorded outcome
            // instead of re-popping a browser on this automatic attempt.
            if let Some(recorded) = read_cooldown(&cooldown_path) {
                return Err(recorded);
            }
        } else {
            // An explicit user retry clears any prior suppression.
            clear_cooldown(&cooldown_path);
        }

        let eps = self.discover(&mut endpoints).await?;
        // Wrap the browser flow in the *remaining* attempt budget so the total
        // locked time never exceeds `attempt_deadline` (and thus never
        // outlasts a waiter's `LOCK_WAIT_TIMEOUT`). A deadline expiry maps to
        // `TimedOut`, which is cooldown-worthy, so it flows through the same
        // writer arm below instead of being dropped by a cancel that would
        // release the lock without recording the cooldown.
        let remaining = attempt_deadline.saturating_duration_since(std::time::Instant::now());
        let flow = browser_pkce_flow(&self.http, &self.cfg, eps, self.opener.as_ref());
        let outcome = match tokio::time::timeout(remaining, flow).await {
            Ok(result) => result,
            Err(_) => Err(AuthError::TimedOut),
        };
        match outcome {
            // `finish` clears the cooldown on success.
            Ok(fresh) => self.finish(&mut state, fresh),
            Err(e) => {
                if e.is_cooldown_worthy() {
                    write_cooldown(&cooldown_path, &e);
                }
                Err(e)
            }
        }
    }

    /// Persist a freshly-obtained token, clear any cooldown, and return its
    /// bearer. A cache-write failure maps to [`AuthError::NetworkUnavailable`]
    /// (the infrastructural bucket) — the token was valid but couldn't be
    /// persisted, which the caller should treat as transient, not as a
    /// credential rejection.
    fn finish(
        &self,
        state: &mut Option<CachedToken>,
        token: CachedToken,
    ) -> Result<String, AuthError> {
        let bearer = token.access_token.clone();
        self.save(state, token)
            .map_err(|_| AuthError::NetworkUnavailable)?;
        clear_cooldown(&self.cooldown_path());
        Ok(bearer)
    }
}

#[async_trait]
impl TokenSource for PkceOAuthTokenSource {
    /// Acquire a bearer for a request. Routes through the coordinator as a
    /// [`Headless`](AuthIntent::Headless) acquisition: it serves a cached or
    /// refreshed token but never opens a browser, so a managed runtime with no
    /// interactive display can never hang on inference. First-use auth is the
    /// job of `buzz-agent auth databricks` ([`interactive_login`]).
    ///
    /// [`interactive_login`]: PkceOAuthTokenSource::interactive_login
    async fn bearer(&self) -> Result<String, AgentError> {
        self.acquire(AuthIntent::Headless, None)
            .await
            .map_err(Into::into)
    }

    /// Identical to [`bearer`](Self::bearer) for this source — both are
    /// headless. Retained as a distinct method so callers can state the
    /// no-browser requirement at the call site (and so other [`TokenSource`]
    /// impls that *would* browse in `bearer` can still expose a safe path).
    async fn bearer_no_browser(&self) -> Result<String, AgentError> {
        self.acquire(AuthIntent::Headless, None)
            .await
            .map_err(Into::into)
    }

    /// Force a fresh bearer after the server rejected `rejected` with a 401.
    ///
    /// A [`Headless`](AuthIntent::Headless) acquisition keyed by token
    /// *identity* rather than the expiry clock: a 401 means the cached token
    /// was rejected while still locally fresh, so [`is_expired`] would wrongly
    /// keep it. Passing `rejected` makes the coordinator run the refresh-token
    /// grant unless a concurrent caller already replaced the token, in which
    /// case that newer token is returned without a second grant. Never opens a
    /// browser; a dead refresh token surfaces terminally so the retry loop
    /// stops instead of hanging.
    async fn refresh_now(&self, rejected: &str) -> Result<String, AgentError> {
        self.acquire(AuthIntent::Headless, Some(rejected))
            .await
            .map_err(Into::into)
    }
}

// ---- helpers -------------------------------------------------------------

/// Aborts a spawned task when dropped. Used to guarantee the localhost
/// callback server doesn't outlive a failed/abandoned PKCE attempt.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn is_expired(t: &CachedToken) -> bool {
    let Some(exp) = t.expires_at else {
        return false;
    };
    now_secs() + TOKEN_REFRESH_LEEWAY.as_secs() >= exp
}

fn cache_path_for(cfg: &PkceOAuthConfig) -> Result<PathBuf, AgentError> {
    let mut h = sha2::Sha256::new();
    h.update(cfg.discovery_url.as_bytes());
    h.update(b"|");
    h.update(cfg.client_id.as_bytes());
    h.update(b"|");
    h.update(cfg.scopes.join(",").as_bytes());
    let hash = hex::encode(h.finalize());

    let dir = match &cfg.cache_dir_override {
        Some(p) => p.join(&cfg.cache_namespace),
        None => dirs::home_dir()
            .ok_or_else(|| AgentError::Llm("oauth cache: home directory not found".into()))?
            .join(".config")
            .join("buzz-agent")
            .join("oauth")
            .join(&cfg.cache_namespace),
    };
    Ok(dir.join(format!("{hash}.json")))
}

/// Append `ext` as an extra extension onto `base` (e.g. `<hash>.json` →
/// `<hash>.json.lock`). Keeps the lock and cooldown sidecars in the same
/// per-key directory as the cache, so they inherit its `$HOME` override and
/// owner-only parent without a second key derivation.
fn append_ext(base: &Path, ext: &str) -> PathBuf {
    let mut name = base.as_os_str().to_owned();
    name.push(".");
    name.push(ext);
    PathBuf::from(name)
}

/// Durable record of the last browser-attempt failure for a cache key. Written
/// while holding the auth lock so concurrent writers can't interleave, read by
/// `Auto` callers to decide whether to suppress an automatic browser re-launch.
#[derive(Debug, Serialize, Deserialize)]
struct CooldownRecord {
    /// [`AuthError::code`] of the failure being cooled down.
    code: String,
    /// Unix seconds after which the cooldown lapses and an `Auto` caller may
    /// launch a browser again.
    until: u64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Return the still-active cooldown outcome for `path`, if any.
///
/// `None` when the sidecar is absent, unparseable, expired, or records a code
/// this build doesn't recognize — every one of those means "no active
/// cooldown", so the caller proceeds to a normal attempt. An expired record is
/// removed opportunistically so the directory doesn't accumulate stale files.
fn read_cooldown(path: &Path) -> Option<AuthError> {
    let body = fs::read(path).ok()?;
    let record: CooldownRecord = serde_json::from_slice(&body).ok()?;
    if record.until > now_secs() {
        AuthError::from_code(&record.code)
    } else {
        let _ = fs::remove_file(path);
        None
    }
}

/// Record `err` as a fresh cooldown at `path`, expiring [`COOLDOWN_DURATION`]
/// from now. Best-effort: a write failure only means the next automatic
/// attempt may re-pop a browser, never a hard auth failure, so errors are
/// swallowed. Called while holding the auth lock.
fn write_cooldown(path: &Path, err: &AuthError) {
    let record = CooldownRecord {
        code: err.code().to_string(),
        until: now_secs() + COOLDOWN_DURATION.as_secs(),
    };
    if let Ok(body) = serde_json::to_vec(&record) {
        let _ = write_private_cache(path, &body);
    }
}

/// Remove any cooldown sidecar at `path`. Called on a successful acquisition
/// (the problem is resolved) and by `UserInitiated` callers that bypass the
/// cooldown (an explicit retry clears the suppression). Best-effort.
fn clear_cooldown(path: &Path) {
    let _ = fs::remove_file(path);
}

/// Hold on the cross-process auth lock. Dropping it (or the owning process
/// dying) releases the OS advisory lock — no PID files, no manual break.
#[derive(Debug)]
struct AuthLockGuard(fs::File);

impl Drop for AuthLockGuard {
    fn drop(&mut self) {
        // Explicit for intent; closing the fd would release it regardless.
        let _ = FileExt::unlock(&self.0);
    }
}

/// Acquire the cross-process auth lock at `path`, polling until `deadline`.
///
/// `fs2::FileExt::try_lock_exclusive` maps to `flock(LOCK_EX | LOCK_NB)` on
/// Unix and `LockFileEx` on Windows — advisory, per–open-file-description, so
/// a lock taken on one handle blocks every other handle (same process or not),
/// which is exactly the cross-process single-flight guarantee we want. The
/// try-lock is non-blocking, so we poll on [`LOCK_POLL_INTERVAL`] rather than
/// parking a worker thread in a blocking `lock_exclusive()`. Contention is
/// reported as [`fs2::lock_contended_error`] (`EWOULDBLOCK`/`EACCES` on Unix,
/// `ERROR_LOCK_VIOLATION` on Windows); we match its `raw_os_error` and retry.
/// Any other error is a real fault and returns [`AuthError::LockTimeout`]. A
/// waiter whose `deadline` lapses also returns [`AuthError::LockTimeout`];
/// because the caller sets that deadline longer than [`AUTH_ATTEMPT_DEADLINE`],
/// a healthy holder always finishes first.
async fn acquire_auth_lock(
    path: &Path,
    deadline: std::time::Instant,
) -> Result<AuthLockGuard, AuthError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| AuthError::LockTimeout)?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|_| AuthError::LockTimeout)?;
    let contended = fs2::lock_contended_error().raw_os_error();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(AuthLockGuard(file)),
            Err(e) if e.raw_os_error() == contended => {
                if std::time::Instant::now() >= deadline {
                    return Err(AuthError::LockTimeout);
                }
                tokio::time::sleep(LOCK_POLL_INTERVAL).await;
            }
            Err(_) => return Err(AuthError::LockTimeout),
        }
    }
}

/// Key for the in-process single-flight registry: the cross-process lock path
/// (one per cache key) paired with whether the caller may open a browser.
/// Browser-capable callers (`Auto`/`UserInitiated`) coalesce with each other;
/// a `Headless` caller keys separately so it neither inherits an interactive
/// failure nor denies an explicit sign-in its browser — those two still
/// coordinate through the cross-process file lock, not this registry.
type InflightKey = (PathBuf, bool);

/// Process-global registry of in-flight auth attempts, the in-process
/// counterpart to [`acquire_auth_lock`]'s cross-process file lock. The file
/// lock serializes work across processes and shares *success* via a cache
/// re-read, but a queued caller that acquires the lock after a browser denial
/// would clear the sidecar and pop a second browser. This registry closes that
/// gap: a caller that arrives while a leader's attempt is in flight joins the
/// leader's [`InflightSlot`] and receives the *same* result — success or
/// failure — instead of taking the lock afterward and launching again. Guarded
/// by a `std::sync::Mutex` because every critical section is a cheap map lookup
/// with no `.await` held.
static INFLIGHT: LazyLock<std::sync::Mutex<HashMap<InflightKey, Arc<InflightSlot>>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Lock the in-flight registry, recovering from a poisoned mutex rather than
/// panicking: the only work done under this lock is map lookups that can't
/// leave inconsistent state, so a poison from an unrelated panic must not wedge
/// every future auth attempt.
fn inflight_registry() -> std::sync::MutexGuard<'static, HashMap<InflightKey, Arc<InflightSlot>>> {
    INFLIGHT.lock().unwrap_or_else(|e| e.into_inner())
}

/// The shared result of one leader's auth attempt, awaited by any joiner that
/// arrived while the leader was in flight. A `watch` channel gives us
/// publish-once plus wait-for-publish in one primitive: the leader publishes
/// exactly once through [`LeaderGuard`]; joiners clone the published result.
struct InflightSlot {
    tx: watch::Sender<Option<Result<String, AuthError>>>,
    rx: watch::Receiver<Option<Result<String, AuthError>>>,
}

impl InflightSlot {
    fn new() -> Self {
        let (tx, rx) = watch::channel(None);
        Self { tx, rx }
    }

    /// Block until the leader publishes, then clone out its result.
    ///
    /// `borrow_and_update` marks the current value seen before awaiting, so a
    /// publish that lands between the read and the `changed()` await is not a
    /// lost wakeup — the version has advanced, so `changed()` returns at once.
    /// A closed channel (leader dropped without publishing — which
    /// [`LeaderGuard`]'s `Drop` prevents) surfaces as a transient so the caller
    /// retries rather than hangs.
    async fn wait(&self) -> Result<String, AuthError> {
        let mut rx = self.rx.clone();
        loop {
            if let Some(result) = rx.borrow_and_update().clone() {
                return result;
            }
            if rx.changed().await.is_err() {
                return Err(AuthError::NetworkUnavailable);
            }
        }
    }

    /// Publish `result` to every waiting joiner. A send error means no joiners
    /// remain, which is fine.
    fn publish(&self, result: Result<String, AuthError>) {
        let _ = self.tx.send(Some(result));
    }
}

/// RAII owner of a leader's in-flight slot. Guarantees the slot is evicted from
/// [`INFLIGHT`] and a result published to joiners even if the leader future is
/// cancelled or panics: a leader that skipped this would leave a dead slot that
/// turns every later caller into a joiner of an attempt that never publishes,
/// wedging them until `LOCK_WAIT_TIMEOUT`.
struct LeaderGuard {
    key: InflightKey,
    slot: Arc<InflightSlot>,
    done: bool,
}

impl LeaderGuard {
    fn new(key: InflightKey, slot: Arc<InflightSlot>) -> Self {
        Self {
            key,
            slot,
            done: false,
        }
    }

    /// Normal completion: evict the slot, publish `result` to joiners, and
    /// return it to the leader. Evicting *before* publishing means a caller
    /// arriving after this point starts a fresh attempt (a later explicit
    /// retry may launch), while joiners already holding the slot still receive
    /// the result. `Drop` covers the cancel/panic path.
    fn complete(mut self, result: Result<String, AuthError>) -> Result<String, AuthError> {
        self.done = true;
        Self::evict(&self.key, &self.slot);
        self.slot.publish(result.clone());
        result
    }

    /// Remove this leader's slot from the registry, but only if it is still the
    /// same slot — defends against evicting a successor a later attempt may
    /// have installed under the same key.
    fn evict(key: &InflightKey, slot: &Arc<InflightSlot>) {
        let mut reg = inflight_registry();
        if reg
            .get(key)
            .is_some_and(|existing| Arc::ptr_eq(existing, slot))
        {
            reg.remove(key);
        }
    }
}

impl Drop for LeaderGuard {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // Cancelled or panicked before `complete`: evict so later callers start
        // fresh, and wake joiners with a transient error so they retry rather
        // than hang on a leader that will never publish.
        Self::evict(&self.key, &self.slot);
        self.slot.publish(Err(AuthError::NetworkUnavailable));
    }
}

/// Load a cached token, enforcing the owner-only invariant on load.
///
/// Owner-only permissions are a cache *lifecycle* invariant, not just a
/// write-path property: a world-readable cache left by an older buzz-agent
/// (or any tampering) must be tightened the moment we touch it, before the
/// tokens are used — otherwise a file that never expires stays exposed until
/// some future refresh happens to rewrite it. Every load path (initial and
/// cross-process re-reads) funnels through here, so the repair covers them
/// all. Returns `None` when the cache is absent, unreadable, unparseable, or
/// cannot be secured; the caller then falls through to refresh/browser.
fn read_cache(path: &Path) -> Option<CachedToken> {
    let body = read_private_cache(path).ok()?;
    serde_json::from_slice(&body).ok()
}

/// Open the cache, reject symlinks, tighten loose permissions to `0o600`, and
/// return its bytes.
///
/// On Unix `O_NOFOLLOW` rejects a symlinked cache path at the kernel level
/// (no stat/open TOCTOU), and `fchmod` on the already-open handle repairs a
/// loose mode against the pinned inode rather than re-resolving the path.
/// A cache that exists but cannot be secured is an error, so the caller fails
/// closed instead of using an exposed file.
#[cfg(unix)]
fn read_private_cache(path: &Path) -> io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)?;

    let meta = file.metadata()?;
    if !meta.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "oauth cache is not a regular file",
        ));
    }
    // Tighten in place on the open fd if any group/other bit is set. fchmod
    // targets the inode we already hold, so no attacker can swap the path
    // between the check and the repair.
    if meta.permissions().mode() & 0o077 != 0 {
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }

    let mut body = Vec::new();
    file.read_to_end(&mut body)?;
    Ok(body)
}

/// Non-Unix fallback: read the cache as-is. Owner-only enforcement is the
/// Windows DACL work deferred behind the [`create_private_temp_file`] seam.
#[cfg(not(unix))]
fn read_private_cache(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path)
}

/// Removes a temp file on drop unless it was already renamed away. Keeps a
/// failed/partial write from leaving a stray token file behind.
struct TmpFileGuard<'a>(&'a Path);

impl Drop for TmpFileGuard<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0);
    }
}

/// A per-write-unique temp suffix so concurrent savers — sibling threads or
/// separate processes sharing `$HOME` — never collide on one temp path.
/// Falls back to a timestamp if the RNG is unavailable rather than panicking
/// mid-auth.
fn unique_suffix() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_ok() {
        return hex::encode(bytes);
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

/// Write `body` to `path` as an owner-only file via an atomic rename.
///
/// The cache holds both the refresh and access tokens, so it must never be
/// readable by other users. We create a uniquely-named temp file in the same
/// directory with owner-only protection at creation time — mode `0o600` on
/// Unix (see [`create_private_temp_file`]) — so it is never briefly
/// world/other readable, write and fsync it, then rename over the
/// destination. The rename swaps the inode/entry wholesale, so a pre-existing
/// cache file with loose permissions is *replaced* by the new private one;
/// its old mode never survives. `fs::rename` maps to
/// `MOVEFILE_REPLACE_EXISTING` on Windows, so the atomic replace holds on
/// both platforms; the Windows owner-only DACL is pending the unsafe-FFI
/// decision noted at the seam.
fn write_private_cache(path: &Path, body: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "oauth cache path has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("oauth-cache");
    let tmp = parent.join(format!(".{file_name}.{}.tmp", unique_suffix()));
    let guard = TmpFileGuard(&tmp);

    let mut f = create_private_temp_file(&tmp)?;
    f.write_all(body)?;
    f.sync_all()?;
    drop(f);

    fs::rename(&tmp, path)?;
    // The rename consumed the temp path; nothing left to clean up.
    std::mem::forget(guard);
    Ok(())
}

/// Create `tmp` for writing with owner-only permissions from the moment it
/// exists. Fails if the file already exists (`create_new`), which the
/// per-write-unique suffix makes effectively impossible.
#[cfg(unix)]
fn create_private_temp_file(tmp: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(tmp)
}

/// Non-Unix fallback: create the temp file if it does not already exist.
///
/// On Windows the owner-only equivalent is an explicit DACL set at creation
/// (`CreateFileW` with SDDL `D:P(A;;FA;;;OW)`, matching goose's
/// `private_file.rs`), but that FFI needs `unsafe`, which this crate forbids.
/// Reconciling the two — an isolated helper crate, a vetted safe dependency,
/// or descoping Windows — is an open decision escalated to the maintainer, so
/// this interim relies on the default per-user ACLs and drops the owner-only
/// implementation in behind this seam once the decision lands. `create_new`
/// fails if the file already exists.
#[cfg(not(unix))]
fn create_private_temp_file(tmp: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp)
}

/// Parse a token-endpoint JSON response. Fails loudly when `access_token`
/// is missing or empty — without this, a malformed server response would
/// be cached and `bearer()` would silently return `""` until the entry
/// expires or is deleted by hand.
fn token_from_response(
    v: &Value,
    fallback_refresh: Option<&str>,
) -> Result<CachedToken, AgentError> {
    let access_token = v
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AgentError::Llm("oauth: token response missing/empty access_token".into()))?
        .to_string();
    let refresh_token = v
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| fallback_refresh.map(str::to_string));
    let expires_at = v.get("expires_in").and_then(Value::as_u64).map(|secs| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            + secs
    });
    Ok(CachedToken {
        access_token,
        refresh_token,
        expires_at,
    })
}

/// PKCE pieces: URL-safe random verifier (~64 chars) and its SHA-256
/// challenge (RFC 7636 §4.2).
fn pkce_pair() -> Result<(String, String), AgentError> {
    let mut bytes = [0u8; 48];
    getrandom::fill(&mut bytes).map_err(|e| AgentError::Llm(format!("pkce rng: {e}")))?;
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    Ok((verifier, challenge))
}

fn random_state() -> Result<String, AgentError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| AgentError::Llm(format!("state rng: {e}")))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// Decide the OAuth callback result and the HTML page to serve.
///
/// Returns `(result, page)`: `result` carries the auth code (or a detail
/// string on failure) to the waiting flow via the oneshot channel; `page` is
/// the *static* HTML shown in the browser. The page never embeds any request
/// parameter — the `error` query value is attacker-influenceable, so
/// reflecting it would be an XSS sink on the localhost callback. Failure
/// detail travels only through `result`, which surfaces in the process error
/// and logs, never in the served markup.
fn callback_outcome(
    params: &std::collections::HashMap<String, String>,
    expected_state: &str,
) -> (Result<String, String>, String) {
    let result = match (params.get("code"), params.get("state")) {
        (Some(code), Some(st)) if st == expected_state => Ok(code.clone()),
        (Some(_), Some(_)) => Err("state mismatch".to_string()),
        _ => Err(params
            .get("error")
            .map(|e| sanitize_callback_detail(e))
            .unwrap_or_else(|| "missing code".into())),
    };
    let page = match result {
        Ok(_) => "<h2>Buzz: signed in</h2><p>You can close this window.</p>",
        Err(_) => "<h2>Buzz auth failed</h2><p>You can close this window and try again.</p>",
    }
    .to_string();
    (result, page)
}

/// Neutralize an attacker-controllable OAuth `error` value before it enters
/// an error string that later reaches the logs. Control characters (CR/LF in
/// particular) enable log-line injection, and an unbounded value could flood
/// the logs — replace control chars with spaces and cap the length.
fn sanitize_callback_detail(raw: &str) -> String {
    const MAX: usize = 200;
    raw.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX)
        .collect()
}

/// Spin up a localhost callback server, hand the authorize URL to `opener`,
/// wait up to [`BROWSER_AUTH_TIMEOUT`] for the redirect, then exchange the
/// code for a token.
///
/// `opener` is invoked *after* the listener is bound and the abort guard is
/// armed, so a launch failure never returns a URL pointing at a torn-down
/// listener. Every failure is a typed [`AuthError`] so the coordinator can
/// record a cooldown (or not) by category: an open failure is
/// [`BrowserOpenFailed`], a redirect that never arrives is [`TimedOut`], a
/// provider-reported denial is [`Denied`], and a rejected code exchange is
/// [`ExchangeFailed`]; infrastructure faults (bind/exchange transport) are
/// [`NetworkUnavailable`].
///
/// [`BrowserOpenFailed`]: AuthError::BrowserOpenFailed
/// [`TimedOut`]: AuthError::TimedOut
/// [`Denied`]: AuthError::Denied
/// [`ExchangeFailed`]: AuthError::ExchangeFailed
/// [`NetworkUnavailable`]: AuthError::NetworkUnavailable
async fn browser_pkce_flow(
    http: &Client,
    cfg: &PkceOAuthConfig,
    endpoints: &OidcEndpoints,
    opener: &dyn BrowserOpener,
) -> Result<CachedToken, AuthError> {
    use axum::{extract::Query, response::Html, routing::get, Router};
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use tokio::sync::oneshot;

    let (verifier, challenge) = pkce_pair().map_err(|_| AuthError::NetworkUnavailable)?;
    let state = random_state().map_err(|_| AuthError::NetworkUnavailable)?;

    let (tx, rx) = oneshot::channel::<Result<String, String>>();
    let tx = Arc::new(Mutex::new(Some(tx)));

    let expected_state = state.clone();
    let app = Router::new().route(
        "/",
        get(move |Query(params): Query<HashMap<String, String>>| {
            let tx = Arc::clone(&tx);
            let expected = expected_state.clone();
            async move {
                let (result, page) = callback_outcome(&params, &expected);
                if let Some(sender) = tx.lock().await.take() {
                    let _ = sender.send(result);
                }
                Html(page)
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .map_err(|_| AuthError::NetworkUnavailable)?;
    let port = listener
        .local_addr()
        .map_err(|_| AuthError::NetworkUnavailable)?
        .port();
    let redirect_uri = format!("http://localhost:{port}");

    // `_server` is held until this function returns; the drop guard aborts
    // the axum task on every exit path (timeout, callback error, token
    // exchange failure, or success), so we never leak a listener bound to
    // 127.0.0.1 past the auth attempt.
    let _server = AbortOnDrop(tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    }));

    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        endpoints.authorization_endpoint,
        urlencoding::encode(&cfg.client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&cfg.scopes.join(" ")),
        urlencoding::encode(&state),
        urlencoding::encode(&challenge),
    );

    // Launch the browser while the listener is live. A launch failure aborts
    // before we wait on a redirect nobody can send.
    opener.open(&auth_url).map_err(|e| {
        tracing::warn!(error = %e, "oauth browser launch failed");
        AuthError::BrowserOpenFailed
    })?;

    let code = match tokio::time::timeout(BROWSER_AUTH_TIMEOUT, rx).await {
        // Timed out waiting for the redirect.
        Err(_) => return Err(AuthError::TimedOut),
        // Callback task dropped the sender without sending — treat as timeout.
        Ok(Err(_)) => return Err(AuthError::TimedOut),
        // Provider/user reported an error (denial, state mismatch, missing code).
        Ok(Ok(Err(detail))) => {
            tracing::warn!(detail = %detail, "oauth callback reported failure");
            return Err(AuthError::Denied);
        }
        Ok(Ok(Ok(code))) => code,
    };

    // Exchange code for token.
    let params = [
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect_uri),
        ("code_verifier", &verifier),
        ("client_id", &cfg.client_id),
    ];
    let resp = http
        .post(&endpoints.token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|_| AuthError::NetworkUnavailable)?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!(status = "error", body = %body, "oauth code exchange rejected");
        return Err(AuthError::ExchangeFailed);
    }
    let v: Value = resp.json().await.map_err(|_| AuthError::ExchangeFailed)?;
    token_from_response(&v, None).map_err(|_| AuthError::ExchangeFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn pkce_pair_produces_valid_challenge() {
        let (verifier, challenge) = pkce_pair().unwrap();
        assert!(verifier.len() >= 43);
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(sha2::Sha256::digest(verifier.as_bytes()));
        assert_eq!(expected, challenge);
    }

    #[test]
    fn cached_token_no_expiry_is_not_expired() {
        let t = CachedToken {
            access_token: "x".into(),
            refresh_token: None,
            expires_at: None,
        };
        assert!(!is_expired(&t));
    }

    #[test]
    fn cached_token_far_future_is_not_expired() {
        let future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let t = CachedToken {
            access_token: "x".into(),
            refresh_token: None,
            expires_at: Some(future),
        };
        assert!(!is_expired(&t));
    }

    #[test]
    fn cached_token_within_leeway_is_expired() {
        let near = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 10; // 10s away, leeway is 60s → counts as expired
        let t = CachedToken {
            access_token: "x".into(),
            refresh_token: None,
            expires_at: Some(near),
        };
        assert!(is_expired(&t));
    }

    #[test]
    fn cache_path_uses_platform_home_directory() {
        let cfg = PkceOAuthConfig {
            discovery_url: "https://example.com/.well-known".into(),
            client_id: "abc".into(),
            scopes: vec!["a".into(), "b".into()],
            cache_namespace: "demo".into(),
            cache_dir_override: None,
        };
        let p = cache_path_for(&cfg).unwrap();
        let expected_dir = dirs::home_dir()
            .unwrap()
            .join(".config")
            .join("buzz-agent")
            .join("oauth")
            .join("demo");
        assert_eq!(p.parent(), Some(expected_dir.as_path()));
        assert_eq!(p.extension().and_then(|s| s.to_str()), Some("json"));
    }

    #[test]
    fn token_from_response_uses_fallback_refresh() {
        let v: Value = serde_json::from_str(r#"{"access_token":"abc","expires_in":3600}"#).unwrap();
        let t = token_from_response(&v, Some("old-refresh")).unwrap();
        assert_eq!(t.access_token, "abc");
        assert_eq!(t.refresh_token.as_deref(), Some("old-refresh"));
        assert!(t.expires_at.is_some());
    }

    #[test]
    fn token_from_response_rejects_missing_access_token() {
        let v: Value = serde_json::from_str(r#"{"expires_in":3600}"#).unwrap();
        assert!(token_from_response(&v, None).is_err());
    }

    #[test]
    fn token_from_response_rejects_empty_access_token() {
        let v: Value = serde_json::from_str(r#"{"access_token":""}"#).unwrap();
        assert!(token_from_response(&v, None).is_err());
    }

    #[tokio::test]
    async fn test_bearer_reuses_disk_token_after_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = PkceOAuthConfig {
            discovery_url: "https://example.com/.well-known".into(),
            client_id: "test-client".into(),
            scopes: vec!["offline_access".into()],
            cache_namespace: "test".into(),
            cache_dir_override: Some(dir.path().to_path_buf()),
        };
        let source = PkceOAuthTokenSource::new(cfg).unwrap();

        // Expire the in-memory state.
        {
            let mut state = source.state.lock().await;
            *state = Some(CachedToken {
                access_token: "stale".into(),
                refresh_token: None,
                expires_at: Some(0), // long expired
            });
        }

        // Write a valid token to disk (simulating another process refreshing).
        let future_exp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 7200;
        let fresh_token = CachedToken {
            access_token: "fresh-from-disk".into(),
            refresh_token: Some("rt".into()),
            expires_at: Some(future_exp),
        };
        let body = serde_json::to_vec_pretty(&fresh_token).unwrap();
        fs::write(&source.cache_path, &body).unwrap();

        // bearer() should pick up the disk token without any network call.
        let result = source.bearer().await.unwrap();
        assert_eq!(result, "fresh-from-disk");
    }

    #[tokio::test]
    async fn test_bearer_headless_no_credential_is_terminal_without_browser() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = PkceOAuthConfig {
            // Unreachable discovery URL: if bearer() ever attempts discovery or
            // a browser flow, this test would hang or error differently. The
            // headless path must not touch either.
            discovery_url: "https://invalid.example.test/.well-known".into(),
            client_id: "test-client".into(),
            scopes: vec!["offline_access".into()],
            cache_namespace: "test".into(),
            cache_dir_override: Some(dir.path().to_path_buf()),
        };
        let source = PkceOAuthTokenSource::new(cfg).unwrap();

        // Expire the in-memory state with no refresh token.
        {
            let mut state = source.state.lock().await;
            *state = Some(CachedToken {
                access_token: "stale".into(),
                refresh_token: None,
                expires_at: Some(0),
            });
        }

        // Write an expired, refresh-less token to disk too.
        let expired_token = CachedToken {
            access_token: "also-stale".into(),
            refresh_token: None,
            expires_at: Some(0),
        };
        let body = serde_json::to_vec_pretty(&expired_token).unwrap();
        fs::write(&source.cache_path, &body).unwrap();

        // bearer() is a Headless acquisition: past the cache checks with no
        // refresh token, it returns terminally instead of opening a browser.
        // With no refresh token it never even discovers endpoints, so the
        // unreachable URL is never contacted — the error is a graceful
        // LlmAuth, not a hard Llm/discovery error.
        match source.bearer().await.unwrap_err() {
            AgentError::LlmAuth(_) => {} // correct: terminal, no browser
            other => panic!("expected terminal LlmAuth, got: {other:?}"),
        }
    }

    /// `bearer_no_browser` with an empty cache and no refresh token must
    /// return `LlmAuth` immediately — it must NOT attempt OIDC discovery even
    /// when the `discovery_url` is unreachable/invalid, and must never browse.
    /// This guards the regression where `endpoints()` was called
    /// unconditionally before the refresh-token check, causing an `Llm` error
    /// (hard failure) instead of the intended graceful `LlmAuth` fallback.
    #[tokio::test]
    async fn test_bearer_no_browser_empty_cache_no_refresh_returns_llm_auth_without_discovery() {
        let dir = tempfile::tempdir().unwrap();
        // Intentionally invalid/unreachable discovery URL — if endpoints() is
        // called, the test will get an `Llm` error and the assertion below fails.
        let cfg = PkceOAuthConfig {
            discovery_url: "https://invalid.example.test/.well-known/oauth-authorization-server"
                .into(),
            client_id: "test-client".into(),
            scopes: vec!["offline_access".into()],
            cache_namespace: "test".into(),
            cache_dir_override: Some(dir.path().to_path_buf()),
        };
        let source = PkceOAuthTokenSource::new(cfg).unwrap();

        // Empty in-memory state (no token, no refresh token).
        {
            let mut state = source.state.lock().await;
            *state = None;
        }

        // No disk cache file either — dir is empty.

        let result = source.bearer_no_browser().await;
        assert!(result.is_err(), "expected Err, got Ok");
        match result.unwrap_err() {
            AgentError::LlmAuth(_) => {} // correct: graceful fallback
            other => panic!(
                "expected LlmAuth (no discovery attempted), got: {other:?}\n\
                 This means endpoints() was called before the refresh-token check."
            ),
        }
    }

    // ---- callback HTML must never reflect input --------------------------

    #[test]
    fn test_callback_failure_page_omits_reflected_error_param() {
        // A hostile `error` query value carrying markup must not appear in
        // the served HTML — otherwise the localhost callback is an XSS sink.
        let payload = "<script>alert('xss')</script>";
        let mut params = std::collections::HashMap::new();
        params.insert("error".to_string(), payload.to_string());

        let (result, page) = callback_outcome(&params, "expected-state");

        // The failure detail still reaches the waiting flow via `result`...
        assert_eq!(result.as_ref().err().map(String::as_str), Some(payload));
        // ...but the browser page is static and inert.
        assert!(
            !page.contains(payload),
            "callback page reflected the raw error param: {page}"
        );
        assert!(
            !page.contains("<script>"),
            "callback page contains script tag"
        );
        assert!(
            page.contains("auth failed"),
            "unexpected failure page: {page}"
        );
    }

    #[test]
    fn test_callback_error_detail_strips_control_chars_and_caps_length() {
        // The `error` param feeds an error string that reaches the logs, so
        // CR/LF (log-line injection) must be neutralized and length bounded.
        let payload = format!("bad\r\nInjected: fake-log-line{}", "A".repeat(500));
        let mut params = std::collections::HashMap::new();
        params.insert("error".to_string(), payload);

        let (result, _page) = callback_outcome(&params, "expected-state");
        let detail = result.unwrap_err();

        assert!(
            !detail.contains('\r'),
            "carriage return survived: {detail:?}"
        );
        assert!(!detail.contains('\n'), "newline survived: {detail:?}");
        assert!(
            detail.len() <= 200,
            "detail not length-capped: {}",
            detail.len()
        );
        assert!(
            detail.starts_with("bad  Injected:"),
            "unexpected sanitized detail: {detail:?}"
        );
    }

    #[test]
    fn test_callback_state_mismatch_page_omits_reflected_code() {
        // A returned code with a mismatched state must be rejected, and the
        // page must not echo the (attacker-chosen) state or code values.
        let mut params = std::collections::HashMap::new();
        params.insert(
            "code".to_string(),
            "<img src=x onerror=alert(1)>".to_string(),
        );
        params.insert("state".to_string(), "attacker-state".to_string());

        let (result, page) = callback_outcome(&params, "expected-state");

        assert_eq!(
            result.as_ref().err().map(String::as_str),
            Some("state mismatch")
        );
        assert!(
            !page.contains("<img"),
            "callback page reflected the code param: {page}"
        );
    }

    #[test]
    fn test_callback_success_returns_code_and_static_page() {
        let mut params = std::collections::HashMap::new();
        params.insert("code".to_string(), "auth-code-123".to_string());
        params.insert("state".to_string(), "expected-state".to_string());

        let (result, page) = callback_outcome(&params, "expected-state");

        assert_eq!(
            result.as_ref().ok().map(String::as_str),
            Some("auth-code-123")
        );
        assert!(
            page.contains("signed in"),
            "unexpected success page: {page}"
        );
        // The success page is a fixed literal — no request data in it.
        assert!(!page.contains("auth-code-123"));
    }

    // ---- private atomic cache write --------------------------------------

    #[cfg(unix)]
    #[tokio::test]
    async fn test_save_writes_owner_only_cache_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let cfg = PkceOAuthConfig {
            discovery_url: "https://example.com/.well-known".into(),
            client_id: "test-client".into(),
            scopes: vec!["offline_access".into()],
            cache_namespace: "test".into(),
            cache_dir_override: Some(dir.path().to_path_buf()),
        };
        let source = PkceOAuthTokenSource::new(cfg).unwrap();

        {
            let mut state = source.state.lock().await;
            source
                .save(
                    &mut state,
                    CachedToken {
                        access_token: "secret-access".into(),
                        refresh_token: Some("secret-refresh".into()),
                        expires_at: None,
                    },
                )
                .unwrap();
        }

        let mode = fs::metadata(&source.cache_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "cache file must be owner-only, got {:o}",
            mode & 0o777
        );
        // No temp file left behind.
        let leftovers: Vec<_> = fs::read_dir(dir.path().join("test"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file leaked: {leftovers:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_save_repairs_preexisting_loose_mode_file() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let dir = tempfile::tempdir().unwrap();
        let cfg = PkceOAuthConfig {
            discovery_url: "https://example.com/.well-known".into(),
            client_id: "test-client".into(),
            scopes: vec!["offline_access".into()],
            cache_namespace: "test".into(),
            cache_dir_override: Some(dir.path().to_path_buf()),
        };
        let source = PkceOAuthTokenSource::new(cfg).unwrap();

        // Simulate a world-readable cache written by an older buzz-agent.
        fs::create_dir_all(source.cache_path.parent().unwrap()).unwrap();
        fs::write(&source.cache_path, b"{\"access_token\":\"old\"}").unwrap();
        fs::set_permissions(&source.cache_path, fs::Permissions::from_mode(0o644)).unwrap();
        let old_inode = fs::metadata(&source.cache_path).unwrap().ino();

        {
            let mut state = source.state.lock().await;
            source
                .save(
                    &mut state,
                    CachedToken {
                        access_token: "new-access".into(),
                        refresh_token: Some("new-refresh".into()),
                        expires_at: None,
                    },
                )
                .unwrap();
        }

        let meta = fs::metadata(&source.cache_path).unwrap();
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o600,
            "loose-mode cache file was not tightened to 0600"
        );
        // The rename swapped the inode — the loose-mode file is gone, not
        // chmod'd in place, so no window where the new tokens inherit 0644.
        assert_ne!(meta.ino(), old_inode, "cache inode was not replaced");
    }

    #[cfg(unix)]
    #[test]
    fn test_write_private_cache_concurrent_savers_all_succeed() {
        use std::os::unix::fs::PermissionsExt;

        // Unique tmp suffixes must let many concurrent writers to the SAME
        // destination each land a complete, private file — no tmp collision,
        // no failed rename. The fixed `*.json.tmp` name this replaces would
        // race (one writer's rename fails on the other's half-written tmp).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let path = Arc::new(path);

        let handles: Vec<_> = (0..16)
            .map(|i| {
                let path = Arc::clone(&path);
                std::thread::spawn(move || {
                    let body = format!("{{\"n\":{i}}}");
                    write_private_cache(&path, body.as_bytes())
                })
            })
            .collect();

        for h in handles {
            h.join()
                .unwrap()
                .expect("concurrent write_private_cache failed");
        }

        // Destination exists, is valid (one writer's complete body), and 0600.
        let contents = fs::read_to_string(path.as_ref()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert!(v.get("n").is_some(), "cache body was corrupted: {contents}");
        let mode = fs::metadata(path.as_ref()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        // No temp files leaked.
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files leaked: {leftovers:?}");
    }

    // ---- owner-only is a load-time invariant, not just a save-time one ----

    #[cfg(unix)]
    #[tokio::test]
    async fn test_bearer_cache_hit_tightens_preexisting_loose_mode_file() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        // A world-readable cache left by an older buzz-agent must be tightened
        // the moment we load it — on the plain cache-hit path, with no refresh
        // or save. Otherwise the pre-bug population stays exposed indefinitely.
        let dir = tempfile::tempdir().unwrap();
        let cfg = PkceOAuthConfig {
            discovery_url: "https://example.com/.well-known".into(),
            client_id: "test-client".into(),
            scopes: vec!["offline_access".into()],
            cache_namespace: "test".into(),
            cache_dir_override: Some(dir.path().to_path_buf()),
        };
        let cache_path = cache_path_for(&cfg).unwrap();
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();

        // A valid, unexpired token so bearer() takes the cache hit and never
        // touches the network or save path.
        let future_exp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 7200;
        let token = CachedToken {
            access_token: "loose-but-valid".into(),
            refresh_token: Some("rt".into()),
            expires_at: Some(future_exp),
        };
        fs::write(&cache_path, serde_json::to_vec_pretty(&token).unwrap()).unwrap();
        fs::set_permissions(&cache_path, fs::Permissions::from_mode(0o644)).unwrap();
        let old_inode = fs::metadata(&cache_path).unwrap().ino();

        // Constructing the source loads the cache — repair happens here.
        let source = PkceOAuthTokenSource::new(cfg).unwrap();
        let bearer = source.bearer().await.unwrap();
        assert_eq!(bearer, "loose-but-valid");

        let meta = fs::metadata(&cache_path).unwrap();
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o600,
            "existing loose-mode cache was not tightened on load"
        );
        // Repaired in place on the open fd — same inode, no rewrite/rename.
        assert_eq!(
            meta.ino(),
            old_inode,
            "load-time repair should fchmod in place, not replace the inode"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_read_cache_refuses_symlinked_cache_path() {
        // A cache path that is a symlink must be rejected, not followed — a
        // hostile symlink could redirect the read (and our chmod) at an
        // arbitrary file. `read_cache` returns None so the caller falls
        // through to a fresh flow instead of trusting the link target.
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real-token.json");
        fs::write(&target, b"{\"access_token\":\"via-symlink\"}").unwrap();

        let link = dir.path().join("cache.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(
            read_cache(&link).is_none(),
            "read_cache followed a symlinked cache path"
        );
    }

    // ---- cross-process advisory lock primitive --------------------------
    //
    // The full 165s waiter bound (`LOCK_WAIT_TIMEOUT`) is not exercisable in a
    // unit test, so these drive `acquire_auth_lock` with explicit deadlines to
    // pin the three properties the coordinator relies on: a contended waiter
    // times out (never blocks forever), a timeout leaves the *holder*
    // untouched (never cancels the in-flight attempt), and releasing the
    // holder — the RAII stand-in for a crashed process — lets a successor
    // proceed with no wedge and no lock-breaking.

    #[tokio::test]
    async fn test_lock_wait_times_out_and_leaves_holder_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json.lock");

        // Holder takes the lock with a generous deadline.
        let holder = acquire_auth_lock(&path, Instant::now() + Duration::from_secs(30))
            .await
            .expect("holder should acquire the free lock");

        // A waiter with an already-lapsed deadline must give up with
        // LockTimeout rather than block — this is the deadline-aware polling
        // that replaces a blocking `lock()`.
        let waiter = acquire_auth_lock(&path, Instant::now()).await;
        assert!(
            matches!(waiter, Err(AuthError::LockTimeout)),
            "contended waiter past its deadline must return LockTimeout, got {waiter:?}"
        );

        // The timeout did not cancel or steal the holder: a second immediate
        // waiter still cannot acquire, proving the holder is intact.
        let still_held = acquire_auth_lock(&path, Instant::now()).await;
        assert!(
            matches!(still_held, Err(AuthError::LockTimeout)),
            "holder must remain intact after a waiter times out, got {still_held:?}"
        );

        drop(holder);
    }

    #[tokio::test]
    async fn test_lock_timeout_leaves_cooldown_sidecar_byte_for_byte_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("cache.json.lock");
        let cooldown_path = dir.path().join("cache.json.cooldown");

        // A pre-existing cooldown sidecar written by an earlier interactive
        // failure. A waiter that can't take the lock must return before any
        // code that reads/clears/writes the cooldown, so these exact bytes
        // survive untouched — otherwise a lock-contended caller could clear a
        // live suppression and let the next Auto caller re-pop a browser.
        let original = br#"{"code":"denied","until":9999999999}"#;
        fs::write(&cooldown_path, original).unwrap();

        // Holder owns the lock (RAII stand-in for another live process).
        let holder = acquire_auth_lock(&lock_path, Instant::now() + Duration::from_secs(30))
            .await
            .expect("holder should acquire the free lock");

        // A waiter past its deadline gives up with LockTimeout — the `?` in
        // `acquire_leader` propagates this before `acquire_locked` (which owns
        // every sidecar mutation) is ever entered.
        let waiter = acquire_auth_lock(&lock_path, Instant::now()).await;
        assert!(
            matches!(waiter, Err(AuthError::LockTimeout)),
            "contended waiter past its deadline must return LockTimeout, got {waiter:?}"
        );

        let after = fs::read(&cooldown_path).unwrap();
        assert_eq!(
            after.as_slice(),
            original.as_slice(),
            "a lock timeout must leave the cooldown sidecar byte-for-byte untouched"
        );

        drop(holder);
    }
}
