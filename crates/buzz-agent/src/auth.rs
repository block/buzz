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

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use tokio::sync::Mutex;

use crate::types::AgentError;

/// Buffer before `expires_at` to consider a cached token "still good".
/// Keeps us off the cliff if the clock or the server's clock drifts.
const TOKEN_REFRESH_LEEWAY: Duration = Duration::from_secs(60);

/// Wall-clock budget for the interactive browser dance. Goose uses 60s.
/// We match: any longer and the user has gone to lunch.
const BROWSER_AUTH_TIMEOUT: Duration = Duration::from_secs(60);

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
    /// Single-flight guard: only one refresh/browser flow at a time, even
    /// if many tool calls land concurrently.
    state: Mutex<Option<CachedToken>>,
}

impl PkceOAuthTokenSource {
    pub fn new(cfg: PkceOAuthConfig) -> Result<Arc<Self>, AgentError> {
        let cache_path = cache_path_for(&cfg)?;
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AgentError::Llm(format!("oauth cache dir {parent:?}: {e}")))?;
        }
        let initial = read_cache(&cache_path);
        Ok(Arc::new(Self {
            cfg,
            http: Client::new(),
            cache_path,
            state: Mutex::new(initial),
        }))
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
    async fn refresh(
        &self,
        endpoints: &OidcEndpoints,
        refresh_token: &str,
    ) -> Result<CachedToken, AgentError> {
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &self.cfg.client_id),
        ];
        let resp = self
            .http
            .post(&endpoints.token_endpoint)
            .form(&params)
            .send()
            .await
            .map_err(|e| AgentError::Llm(format!("oauth refresh: {e}")))?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AgentError::Llm(format!("oauth refresh failed: {body}")));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| AgentError::Llm(format!("oauth refresh json: {e}")))?;
        token_from_response(&v, Some(refresh_token))
    }

    /// Run the full browser-mediated Authorization Code + PKCE flow.
    /// Caller must hold a TTY/browser: this opens a window and blocks.
    pub async fn interactive_login(&self) -> Result<(), AgentError> {
        let endpoints = self.endpoints().await?;
        let token = browser_pkce_flow(&self.http, &self.cfg, &endpoints).await?;
        let mut state = self.state.lock().await;
        self.save(&mut state, token)?;
        Ok(())
    }
}

#[async_trait]
impl TokenSource for PkceOAuthTokenSource {
    async fn bearer(&self) -> Result<String, AgentError> {
        let mut state = self.state.lock().await;

        // 1. In-memory cache hit, still fresh.
        if let Some(tok) = state.as_ref() {
            if !is_expired(tok) {
                return Ok(tok.access_token.clone());
            }
        }

        // 2. Re-read disk — another process may have refreshed already.
        if let Some(disk_tok) = read_cache(&self.cache_path) {
            if !is_expired(&disk_tok) {
                let bearer = disk_tok.access_token.clone();
                *state = Some(disk_tok);
                return Ok(bearer);
            }
        }

        // 3. Try refresh if we have a refresh token. Discover endpoints once
        //    here — deliberately hoisted above the refresh-token check so the
        //    browser flow at step 5 (which also needs them) reuses this call.
        let endpoints = self.endpoints().await?;
        let refresh = state.as_ref().and_then(|t| t.refresh_token.clone());
        if let Some(rt) = refresh {
            match self.refresh(&endpoints, &rt).await {
                Ok(fresh) => {
                    let bearer = fresh.access_token.clone();
                    self.save(&mut state, fresh)?;
                    return Ok(bearer);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "oauth refresh failed; falling back to browser flow");
                }
            }

            // 4. Re-read disk after refresh failure — another process may have won the race.
            if let Some(disk_tok) = read_cache(&self.cache_path) {
                if !is_expired(&disk_tok) {
                    let bearer = disk_tok.access_token.clone();
                    *state = Some(disk_tok);
                    return Ok(bearer);
                }
            }
        }

        // 5. No usable cache: full browser dance.
        let fresh = browser_pkce_flow(&self.http, &self.cfg, &endpoints).await?;
        let bearer = fresh.access_token.clone();
        self.save(&mut state, fresh)?;
        Ok(bearer)
    }

    async fn bearer_no_browser(&self) -> Result<String, AgentError> {
        self.try_bearer_no_browser().await
    }

    /// Force-refresh after a 401, never touching the browser flow.
    ///
    /// `rejected` is the access token the server just 401'd. Coalescing keys
    /// off token *identity*, not the expiry clock: a 401 means the token was
    /// rejected while it still looked locally fresh, so `is_expired()` would
    /// say "keep it" and no grant would ever run. Instead, under the lock we
    /// compare the current cached token to `rejected` — if they differ, a
    /// concurrent caller (this process or a sibling) already refreshed, so we
    /// return the new token without burning a second grant. If they still
    /// match, this is the rejected token and we run the refresh-token grant
    /// unconditionally. The whole check→refresh→save runs under one lock hold
    /// so concurrent callers serialize. On any failure the refresh token is
    /// preserved (never nulled) and the error is terminal `LlmAuth` — no
    /// browser, no hang.
    async fn refresh_now(&self, rejected: &str) -> Result<String, AgentError> {
        let mut state = self.state.lock().await;

        // 1. Coalesce by identity: if the cached token (in-memory, then disk)
        //    is no longer the one the server rejected, someone already
        //    refreshed it. Return that instead of grabbing another grant.
        if let Some(tok) = state.as_ref() {
            if tok.access_token != rejected {
                return Ok(tok.access_token.clone());
            }
        }
        if let Some(disk_tok) = read_cache(&self.cache_path) {
            if disk_tok.access_token != rejected {
                let bearer = disk_tok.access_token.clone();
                *state = Some(disk_tok);
                return Ok(bearer);
            }
        }

        // 2. The cached token is still the rejected one. Run the refresh-token
        //    grant unconditionally — the expiry clock can't be trusted here, a
        //    locally-fresh token is exactly what got 401'd.
        let refresh = state.as_ref().and_then(|t| t.refresh_token.clone());
        let Some(rt) = refresh else {
            return Err(AgentError::LlmAuth(
                "token rejected and no refresh token available".into(),
            ));
        };
        let endpoints = self.endpoints().await?;
        match self.refresh(&endpoints, &rt).await {
            Ok(fresh) => {
                let bearer = fresh.access_token.clone();
                self.save(&mut state, fresh)?;
                Ok(bearer)
            }
            // 3. Refresh token is itself dead. Terminal — surfacing LlmAuth
            //    stops the retry loop instead of falling to the browser flow,
            //    which would hang a headless harness.
            Err(e) => Err(AgentError::LlmAuth(format!("token refresh failed: {e}"))),
        }
    }
}

impl PkceOAuthTokenSource {
    /// Return a bearer token from cache or refresh, **never** opening a browser.
    ///
    /// Follows the same steps as [`bearer`](TokenSource::bearer) but stops at
    /// step 4 — if no usable token is available after cache + refresh attempts,
    /// returns `Err(LlmAuth(...))` instead of launching the browser PKCE flow.
    /// Used by model-discovery paths that must not block on user interaction.
    pub(crate) async fn try_bearer_no_browser(&self) -> Result<String, AgentError> {
        let mut state = self.state.lock().await;

        // 1. In-memory cache hit, still fresh.
        if let Some(tok) = state.as_ref() {
            if !is_expired(tok) {
                return Ok(tok.access_token.clone());
            }
        }

        // 2. Re-read disk — another process may have refreshed already.
        if let Some(disk_tok) = read_cache(&self.cache_path) {
            if !is_expired(&disk_tok) {
                let bearer = disk_tok.access_token.clone();
                *state = Some(disk_tok);
                return Ok(bearer);
            }
        }

        // 3. Try refresh if we have a refresh token.  Endpoints are discovered
        //    lazily here — only when a refresh token is actually present — so
        //    that an unreachable OIDC discovery URL cannot prevent the
        //    no-token/no-cache path from returning `LlmAuth` (graceful
        //    fallback) instead of `Llm` (hard error).
        let refresh = state.as_ref().and_then(|t| t.refresh_token.clone());
        if let Some(rt) = refresh {
            let endpoints = self.endpoints().await?;
            match self.refresh(&endpoints, &rt).await {
                Ok(fresh) => {
                    let bearer = fresh.access_token.clone();
                    self.save(&mut state, fresh)?;
                    return Ok(bearer);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "oauth refresh failed during model discovery");
                }
            }

            // 4. Re-read disk after refresh failure.
            if let Some(disk_tok) = read_cache(&self.cache_path) {
                if !is_expired(&disk_tok) {
                    let bearer = disk_tok.access_token.clone();
                    *state = Some(disk_tok);
                    return Ok(bearer);
                }
            }
        }

        // No usable token — return error instead of opening a browser.
        Err(AgentError::LlmAuth(
            "no cached Databricks token; run `buzz-agent auth databricks` first".into(),
        ))
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
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now + TOKEN_REFRESH_LEEWAY.as_secs() >= exp
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

/// Spin up a localhost callback server, open the authorize URL in a
/// browser, wait up to [`BROWSER_AUTH_TIMEOUT`] for the redirect, then
/// exchange the code for a token.
async fn browser_pkce_flow(
    http: &Client,
    cfg: &PkceOAuthConfig,
    endpoints: &OidcEndpoints,
) -> Result<CachedToken, AgentError> {
    use axum::{extract::Query, response::Html, routing::get, Router};
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use tokio::sync::oneshot;

    let (verifier, challenge) = pkce_pair()?;
    let state = random_state()?;

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
        .map_err(|e| AgentError::Llm(format!("oauth callback bind: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| AgentError::Llm(format!("oauth callback addr: {e}")))?
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

    eprintln!("Opening browser for authentication. If it doesn't open, visit:\n  {auth_url}");
    let _ = webbrowser::open(&auth_url);

    let code = tokio::time::timeout(BROWSER_AUTH_TIMEOUT, rx)
        .await
        .map_err(|_| AgentError::Llm("oauth: browser auth timed out".into()))?
        .map_err(|_| AgentError::Llm("oauth: callback sender dropped".into()))?
        .map_err(|e| AgentError::Llm(format!("oauth callback: {e}")))?;

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
        .map_err(|e| AgentError::Llm(format!("oauth exchange: {e}")))?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(AgentError::Llm(format!("oauth exchange failed: {body}")));
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|e| AgentError::Llm(format!("oauth exchange json: {e}")))?;
    token_from_response(&v, None)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn test_bearer_falls_through_to_browser_when_disk_also_expired() {
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
                expires_at: Some(0),
            });
        }

        // Write an expired token to disk too.
        let expired_token = CachedToken {
            access_token: "also-stale".into(),
            refresh_token: None,
            expires_at: Some(0),
        };
        let body = serde_json::to_vec_pretty(&expired_token).unwrap();
        fs::write(&source.cache_path, &body).unwrap();

        // bearer() should fall through past the disk check.
        // It will fail at the endpoints() discovery call since there's no server,
        // which proves it didn't short-circuit on the expired disk token.
        let result = source.bearer().await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("oauth discovery"),
            "expected discovery error, got: {err_msg}"
        );
    }

    /// `try_bearer_no_browser` with an empty cache and no refresh token must
    /// return `LlmAuth` immediately — it must NOT attempt OIDC discovery even
    /// when the `discovery_url` is unreachable/invalid.  This guards the
    /// regression where `endpoints()` was called unconditionally before the
    /// refresh-token check, causing an `Llm` error (hard failure) instead of
    /// the intended graceful `LlmAuth` fallback.
    #[tokio::test]
    async fn test_try_bearer_no_browser_empty_cache_no_refresh_returns_llm_auth_without_discovery()
    {
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

        let result = source.try_bearer_no_browser().await;
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
}

/// A bearer token re-read from an on-disk OIDC session, refreshed as needed.
///
/// # Why this exists
///
/// The OpenAI-compatible provider used `StaticTokenSource`: the key was
/// resolved once, at process start, from `OPENAI_COMPAT_API_KEY`. An
/// environment variable cannot be changed from outside a running process, so
/// the ONLY way to pick up a renewed token was to restart the agent.
///
/// That made restarts load-bearing, and on 2026-08-06 that assumption broke a
/// six-agent fleet: a `systemctl --user` reload no longer reached units that
/// had moved to system scope, no restart happened, and every agent ran for ten
/// hours on a token that died after six. `StaticTokenSource::refresh_now`
/// returns the same dead string, so each 401 was terminal — the fleet took
/// mentions, failed, retried, and discarded them while every unit reported
/// `active (running)`.
///
/// Tools that do not have this problem (OpenClaw, Hermes) share one property:
/// they resolve the credential **per request**, from a file or keyring, rather
/// than capturing one at spawn. This is that, for the OpenAI-compat path.
///
/// # Shape
///
/// Reads the session JSON written by the Grok CLI (`~/.grok/auth.json`):
/// a single-entry object whose value carries `key`, `refresh_token`,
/// `oidc_client_id` and an RFC 3339 `expires_at`. On each `bearer()` the file
/// is consulted; if the token is inside `SKEW` of expiry it is refreshed via
/// the standard `refresh_token` grant and written back atomically.
///
/// Re-reading the file per call is deliberate beyond refresh: it also means an
/// external `grok login` is picked up without restarting anything.
pub struct GrokSessionTokenSource {
    path: PathBuf,
    token_url: String,
    http: Client,
    /// How long before a held refresh lock is presumed abandoned. A field
    /// rather than a constant so the staleness path is testable without a
    /// filetime dependency — backdating a file portably is more machinery than
    /// the behaviour being tested.
    stale_after: Duration,
    /// Serialises refreshes. Grok's OIDC refresh tokens ROTATE, so two
    /// concurrent refreshes race and the loser is left holding a token the
    /// server has already retired.
    lock: Mutex<()>,
}

/// Refresh this long before expiry. Generous because a refresh is cheap and a
/// dead token is not: 26 minutes of retries and then discarded work.
const GROK_REFRESH_SKEW: Duration = Duration::from_secs(600);

/// Cross-process refresh lock tuning. A refresh is one HTTP round trip, so a
/// peer holding the lock should be done well inside LOCK_ATTEMPTS * LOCK_POLL.
const LOCK_ATTEMPTS: usize = 60;
const LOCK_POLL: Duration = Duration::from_millis(100);
/// Longer than any plausible refresh, short enough that a killed process does
/// not wedge the fleet until someone notices.
const LOCK_STALE_AFTER: Duration = Duration::from_secs(30);

impl GrokSessionTokenSource {
    pub fn new(path: PathBuf, token_url: impl Into<String>) -> Self {
        Self {
            path,
            token_url: token_url.into(),
            http: Client::new(),
            stale_after: LOCK_STALE_AFTER,
            lock: Mutex::new(()),
        }
    }

    #[cfg(test)]
    fn with_stale_after(mut self, d: Duration) -> Self {
        self.stale_after = d;
        self
    }

    fn read(&self) -> Result<(Value, String, CachedToken), AgentError> {
        let raw = fs::read_to_string(&self.path)
            .map_err(|e| AgentError::LlmAuth(format!("read {:?}: {e}", self.path)))?;
        let doc: Value = serde_json::from_str(&raw)
            .map_err(|e| AgentError::LlmAuth(format!("parse {:?}: {e}", self.path)))?;
        let (entry_key, entry) = doc
            .as_object()
            .and_then(|m| m.iter().next())
            .map(|(k, v)| (k.clone(), v.clone()))
            .ok_or_else(|| AgentError::LlmAuth(format!("{:?} is empty", self.path)))?;

        let access = entry
            .get("key")
            .or_else(|| entry.get("access_token"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if access.is_empty() {
            return Err(AgentError::LlmAuth(format!(
                "no access token in {:?}",
                self.path
            )));
        }
        let expires_at = entry
            .get("expires_at")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_secs);
        Ok((
            doc,
            entry_key,
            CachedToken {
                access_token: access,
                refresh_token: entry
                    .get("refresh_token")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                expires_at,
            },
        ))
    }

    fn expiring_within(tok: &CachedToken, skew: Duration) -> bool {
        match tok.expires_at {
            // No expiry metadata: trust it and let a 401 drive refresh_now.
            None => false,
            Some(exp) => now_secs() + skew.as_secs() >= exp,
        }
    }

    /// Acquire the cross-process refresh lock, or return the token a peer just
    /// wrote while we waited.
    ///
    /// The in-process `Mutex` is not enough: six agents share one session file,
    /// and Grok's refresh tokens ROTATE. Two processes refreshing concurrently
    /// both present the same refresh token, one wins, and the loser is left
    /// holding one the server has already retired — which is exactly how a
    /// fleet goes dark quietly.
    ///
    /// O_EXCL rather than flock so there is no new dependency, with a staleness
    /// steal so a process killed mid-refresh cannot wedge the fleet forever.
    async fn acquire_file_lock(&self) -> Result<Option<String>, AgentError> {
        let lock_path = self.path.with_extension("lock");
        for _ in 0..LOCK_ATTEMPTS {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(_) => return Ok(None), // acquired; caller refreshes
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // A peer is refreshing. Give it a moment, then check whether
                    // its result is already on disk — the common case, and it
                    // saves us burning a rotation.
                    tokio::time::sleep(LOCK_POLL).await;
                    if let Ok((_, _, tok)) = self.read() {
                        if !Self::expiring_within(&tok, GROK_REFRESH_SKEW) {
                            return Ok(Some(tok.access_token));
                        }
                    }
                    // Steal a lock whose owner evidently died.
                    if let Ok(md) = fs::metadata(&lock_path) {
                        if md
                            .modified()
                            .ok()
                            .and_then(|m| m.elapsed().ok())
                            .is_some_and(|age| age > self.stale_after)
                        {
                            let _ = fs::remove_file(&lock_path);
                        }
                    }
                }
                Err(e) => {
                    return Err(AgentError::LlmAuth(format!(
                        "refresh lock {lock_path:?}: {e}"
                    )))
                }
            }
        }
        Err(AgentError::LlmAuth(
            "timed out waiting for the refresh lock".into(),
        ))
    }

    fn release_file_lock(&self) {
        let _ = fs::remove_file(self.path.with_extension("lock"));
    }

    /// Refresh and write back. Caller must hold the in-process `lock`.
    async fn refresh_locked(&self) -> Result<String, AgentError> {
        if let Some(peer_token) = self.acquire_file_lock().await? {
            return Ok(peer_token); // a peer refreshed while we waited
        }
        let out = self.refresh_inner().await;
        self.release_file_lock();
        out
    }

    async fn refresh_inner(&self) -> Result<String, AgentError> {
        let (mut doc, entry_key, tok) = self.read()?;
        let refresh = tok.refresh_token.clone().ok_or_else(|| {
            AgentError::LlmAuth(format!(
                "{:?} has no refresh_token — run `grok login --oauth`",
                self.path
            ))
        })?;
        let client_id = doc
            .get(&entry_key)
            .and_then(|e| e.get("oidc_client_id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if client_id.is_empty() {
            return Err(AgentError::LlmAuth(format!(
                "{:?} has no oidc_client_id — run `grok login --oauth`",
                self.path
            )));
        }

        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", client_id.as_str()),
        ];
        let resp = self
            .http
            .post(&self.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| AgentError::LlmAuth(format!("grok refresh: {e}")))?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AgentError::LlmAuth(format!(
                "grok refresh rejected: {body}"
            )));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| AgentError::LlmAuth(format!("grok refresh json: {e}")))?;
        let new_access = v
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| AgentError::LlmAuth("grok refresh: no access_token".into()))?
            .to_string();

        // Persist so the Grok CLI and any sibling process see the same session.
        // Rotation means the NEW refresh token must be saved or the next
        // refresh presents one the server has already retired.
        if let Some(entry) = doc.get_mut(&entry_key) {
            entry["key"] = Value::String(new_access.clone());
            if let Some(rt) = v.get("refresh_token").and_then(Value::as_str) {
                entry["refresh_token"] = Value::String(rt.to_string());
            }
            if let Some(sec) = v.get("expires_in").and_then(Value::as_u64) {
                entry["expires_at"] = Value::String(format_rfc3339_secs(now_secs() + sec));
            }
        }
        let body = serde_json::to_vec_pretty(&doc)
            .map_err(|e| AgentError::LlmAuth(format!("grok session serialize: {e}")))?;
        let tmp = self.path.with_extension("json.tmp");
        // Atomic rename: a concurrent reader never sees a partial session, and
        // a crash mid-write cannot leave an unusable file.
        fs::write(&tmp, &body)
            .map_err(|e| AgentError::LlmAuth(format!("grok session write: {e}")))?;
        fs::rename(&tmp, &self.path)
            .map_err(|e| AgentError::LlmAuth(format!("grok session rename: {e}")))?;
        Ok(new_access)
    }
}

#[async_trait]
impl TokenSource for GrokSessionTokenSource {
    async fn bearer(&self) -> Result<String, AgentError> {
        let (_, _, tok) = self.read()?;
        if !Self::expiring_within(&tok, GROK_REFRESH_SKEW) {
            return Ok(tok.access_token);
        }
        let _g = self.lock.lock().await;
        // Re-check under the lock: a concurrent caller may have just refreshed,
        // and refreshing twice would burn a rotated token for nothing.
        let (_, _, tok) = self.read()?;
        if !Self::expiring_within(&tok, GROK_REFRESH_SKEW) {
            return Ok(tok.access_token);
        }
        self.refresh_locked().await
    }

    async fn refresh_now(&self, rejected: &str) -> Result<String, AgentError> {
        let _g = self.lock.lock().await;
        // The server rejected `rejected`, so the local clock is not to be
        // trusted here. Refresh unless someone already replaced it.
        let (_, _, tok) = self.read()?;
        if tok.access_token != rejected {
            return Ok(tok.access_token);
        }
        self.refresh_locked().await
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse the RFC 3339 stamp the Grok CLI writes (`2026-08-06T04:55:48.98Z`).
/// Deliberately tolerant: an unparseable expiry yields `None`, which means
/// "trust the token and let a 401 drive the refresh" rather than failing.
fn parse_rfc3339_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    let (date, rest) = s.split_once('T')?;
    let time = rest.split(['Z', '+']).next()?.split('.').next()?;
    let mut d = date.split('-');
    let (y, mo, da) = (
        d.next()?.parse::<i64>().ok()?,
        d.next()?.parse::<i64>().ok()?,
        d.next()?.parse::<i64>().ok()?,
    );
    let mut t = time.split(':');
    let (h, mi, se) = (
        t.next()?.parse::<i64>().ok()?,
        t.next()?.parse::<i64>().ok()?,
        t.next().unwrap_or("0").parse::<i64>().ok()?,
    );
    // Range-check before converting. Without this, "2026-13-99T99:99:99Z"
    // parses into a plausible-looking epoch far in the future, the token then
    // never looks expired, and the agent goes dark holding it — the exact
    // failure this type exists to prevent. Out of range must mean "unknown",
    // which routes to refresh-on-401.
    if !(1..=12).contains(&mo)
        || !(1..=31).contains(&da)
        || !(0..=23).contains(&h)
        || !(0..=59).contains(&mi)
        || !(0..=60).contains(&se)
        || !(1970..=9999).contains(&y)
    {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm) — no chrono dependency.
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + se;
    u64::try_from(secs).ok()
}

fn format_rfc3339_secs(epoch: u64) -> String {
    let days = (epoch / 86_400) as i64;
    let rem = epoch % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod grok_session_tests {
    use super::*;

    #[test]
    fn parses_the_stamp_the_grok_cli_writes() {
        // Real value observed in ~/.grok/auth.json on 2026-08-06.
        let t = parse_rfc3339_secs("2026-08-06T04:55:48.989687Z").expect("parses");
        assert_eq!(format_rfc3339_secs(t), "2026-08-06T04:55:48Z");
    }

    #[test]
    fn round_trips_across_epochs_and_leap_years() {
        // The civil-days conversion is hand-rolled to avoid a chrono dep, so
        // pin the cases that break naive implementations.
        for s in [
            "1970-01-01T00:00:00Z",
            "2000-02-29T12:00:00Z", // leap, century divisible by 400
            "2024-02-29T23:59:59Z",
            "2026-12-31T23:59:59Z",
            "2100-03-01T00:00:00Z", // century NOT a leap year
        ] {
            let secs = parse_rfc3339_secs(s).unwrap_or_else(|| panic!("parse {s}"));
            assert_eq!(format_rfc3339_secs(secs), s, "round-trip {s}");
        }
    }

    #[test]
    fn tolerates_offsets_and_missing_fractions() {
        assert_eq!(
            parse_rfc3339_secs("2026-08-06T04:55:48+00:00"),
            parse_rfc3339_secs("2026-08-06T04:55:48Z")
        );
    }

    /// An unparseable expiry must NOT be an error: it yields None, which means
    /// "use the token and let a 401 drive the refresh". Failing closed here
    /// would take a fleet down over a formatting change.
    #[test]
    fn unparseable_expiry_is_none_not_an_error() {
        for bad in ["", "never", "2026-13-99T99:99:99Z", "not-a-date"] {
            assert!(parse_rfc3339_secs(bad).is_none(), "{bad:?} should be None");
        }
    }

    #[test]
    fn expiry_window_triggers_refresh_before_death_not_after() {
        let mk = |exp: Option<u64>| CachedToken {
            access_token: "t".into(),
            refresh_token: None,
            expires_at: exp,
        };
        let now = now_secs();
        // Comfortably alive -> no refresh.
        assert!(!GrokSessionTokenSource::expiring_within(
            &mk(Some(now + 3600)),
            GROK_REFRESH_SKEW
        ));
        // Inside the skew -> refresh BEFORE it dies, which is the point.
        assert!(GrokSessionTokenSource::expiring_within(
            &mk(Some(now + 60)),
            GROK_REFRESH_SKEW
        ));
        // Already dead -> refresh.
        assert!(GrokSessionTokenSource::expiring_within(
            &mk(Some(now.saturating_sub(1))),
            GROK_REFRESH_SKEW
        ));
        // No metadata -> trust it; a 401 will drive refresh_now.
        assert!(!GrokSessionTokenSource::expiring_within(
            &mk(None),
            GROK_REFRESH_SKEW
        ));
    }

    #[test]
    fn reads_the_key_out_of_a_grok_shaped_session() {
        let dir = std::env::temp_dir().join(format!("grok-sess-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("auth.json");
        std::fs::write(
            &p,
            r#"{"https://auth.x.ai::abc":{"key":"tok-123","refresh_token":"r-1",
                 "oidc_client_id":"cid","expires_at":"2099-01-01T00:00:00Z"}}"#,
        )
        .unwrap();
        let src = GrokSessionTokenSource::new(p.clone(), "https://example.invalid/token");
        let (_, entry_key, tok) = src.read().expect("reads");
        assert_eq!(entry_key, "https://auth.x.ai::abc");
        assert_eq!(tok.access_token, "tok-123");
        assert_eq!(tok.refresh_token.as_deref(), Some("r-1"));
        assert!(!GrokSessionTokenSource::expiring_within(
            &tok,
            GROK_REFRESH_SKEW
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_or_empty_session_is_an_auth_error_not_a_panic() {
        let src = GrokSessionTokenSource::new(
            std::path::PathBuf::from("/nonexistent/auth.json"),
            "https://example.invalid/token",
        );
        assert!(matches!(src.read(), Err(AgentError::LlmAuth(_))));
    }
}

#[cfg(test)]
mod grok_lock_tests {
    use super::*;

    fn session(dir: &std::path::Path, expires: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join("auth.json");
        std::fs::write(
            &p,
            format!(
                r#"{{"https://auth.x.ai::t":{{"key":"tok","refresh_token":"r",
                     "oidc_client_id":"c","expires_at":"{expires}"}}}}"#
            ),
        )
        .unwrap();
        p
    }

    /// A held lock must NOT be bypassed — that is the rotation race.
    #[tokio::test]
    async fn a_held_lock_blocks_a_second_refresher() {
        let dir = std::env::temp_dir().join(format!("grok-lock-a-{}", std::process::id()));
        let p = session(&dir, "2020-01-01T00:00:00Z"); // expired
        let src = GrokSessionTokenSource::new(p.clone(), "https://example.invalid/token");
        // Simulate a peer mid-refresh.
        std::fs::write(p.with_extension("lock"), b"").unwrap();
        let start = std::time::Instant::now();
        let got = src.acquire_file_lock().await;
        // It must have waited rather than charging ahead, and given up rather
        // than stealing a fresh lock.
        assert!(start.elapsed() >= LOCK_POLL, "returned without waiting");
        // The peer never finished and the lock is fresh, so we must NOT have
        // acquired it — charging ahead here is the rotation race.
        match got {
            Err(_) => {} // timed out waiting: correct
            Ok(Some(_)) => panic!("adopted a token nobody wrote"),
            Ok(None) => panic!("acquired a lock a live peer still holds"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// If a peer finishes while we wait, take its token instead of burning a
    /// rotation of our own.
    #[tokio::test]
    async fn adopts_a_peers_result_instead_of_refreshing_again() {
        let dir = std::env::temp_dir().join(format!("grok-lock-b-{}", std::process::id()));
        let p = session(&dir, "2020-01-01T00:00:00Z");
        let lock = p.with_extension("lock");
        std::fs::write(&lock, b"").unwrap();
        let src = GrokSessionTokenSource::new(p.clone(), "https://example.invalid/token");
        let p2 = p.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            // Peer writes a fresh session, then releases.
            std::fs::write(
                &p2,
                r#"{"https://auth.x.ai::t":{"key":"NEW","refresh_token":"r2",
                     "oidc_client_id":"c","expires_at":"2099-01-01T00:00:00Z"}}"#,
            )
            .unwrap();
            std::fs::remove_file(p2.with_extension("lock")).ok();
        });
        let got = src.acquire_file_lock().await.expect("no error");
        assert_eq!(got.as_deref(), Some("NEW"), "should adopt the peer's token");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A process killed mid-refresh must not wedge the fleet forever.
    #[tokio::test]
    async fn steals_a_stale_lock() {
        let dir = std::env::temp_dir().join(format!("grok-lock-c-{}", std::process::id()));
        let p = session(&dir, "2020-01-01T00:00:00Z");
        let lock = p.with_extension("lock");
        std::fs::write(&lock, b"").unwrap();
        // Rather than backdating the file, shrink the staleness threshold to
        // zero: the lock is older than "instantly", so the steal path runs.
        let src = GrokSessionTokenSource::new(p.clone(), "https://example.invalid/token")
            .with_stale_after(Duration::from_millis(0));
        let got = src.acquire_file_lock().await.expect("no error");
        assert!(got.is_none(), "should have acquired after stealing");
        std::fs::remove_dir_all(&dir).ok();
    }
}
