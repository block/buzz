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
use std::path::PathBuf;
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
    fn save(&self, state: &mut Option<CachedToken>, token: CachedToken) -> Result<(), AgentError> {
        let body = serde_json::to_vec_pretty(&token)
            .map_err(|e| AgentError::Llm(format!("oauth cache serialize: {e}")))?;
        // Atomic rename so a concurrent reader never sees a partial write.
        let tmp = self.cache_path.with_extension("json.tmp");
        fs::write(&tmp, &body)
            .map_err(|e| AgentError::Llm(format!("oauth cache write {tmp:?}: {e}")))?;
        fs::rename(&tmp, &self.cache_path)
            .map_err(|e| AgentError::Llm(format!("oauth cache rename: {e}")))?;
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

        // 1. Cache hit, still fresh.
        if let Some(tok) = state.as_ref() {
            if !is_expired(tok) {
                return Ok(tok.access_token.clone());
            }
        }

        // 2. Cache hit, expired, but we have a refresh token.
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
                    tracing::warn!(error = %e, "oauth refresh failed; falling back to browser flow");
                }
            }
        }

        // 3. No usable cache: full browser dance.
        let endpoints = self.endpoints().await?;
        let fresh = browser_pkce_flow(&self.http, &self.cfg, &endpoints).await?;
        let bearer = fresh.access_token.clone();
        self.save(&mut state, fresh)?;
        Ok(bearer)
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
        None => {
            let home = std::env::var("HOME")
                .map_err(|_| AgentError::Llm("oauth cache: $HOME not set".into()))?;
            PathBuf::from(home)
                .join(".config")
                .join("buzz-agent")
                .join("oauth")
                .join(&cfg.cache_namespace)
        }
    };
    Ok(dir.join(format!("{hash}.json")))
}

fn read_cache(path: &PathBuf) -> Option<CachedToken> {
    let body = fs::read(path).ok()?;
    serde_json::from_slice(&body).ok()
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
                let result = match (params.get("code"), params.get("state")) {
                    (Some(code), Some(st)) if st == &expected => Ok(code.clone()),
                    (Some(_), Some(_)) => Err("state mismatch".to_string()),
                    _ => Err(params
                        .get("error")
                        .cloned()
                        .unwrap_or_else(|| "missing code".into())),
                };
                if let Some(sender) = tx.lock().await.take() {
                    let _ = sender.send(result.clone());
                }
                match result {
                    Ok(_) => Html(
                        "<h2>Sprout: signed in</h2><p>You can close this window.</p>".to_string(),
                    ),
                    Err(e) => Html(format!("<h2>Sprout auth failed</h2><pre>{e}</pre>")),
                }
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
    fn cache_path_includes_namespace_and_hash() {
        // HOME is required; cargo test runs set it.
        let cfg = PkceOAuthConfig {
            discovery_url: "https://example.com/.well-known".into(),
            client_id: "abc".into(),
            scopes: vec!["a".into(), "b".into()],
            cache_namespace: "demo".into(),
            cache_dir_override: None,
        };
        let p = cache_path_for(&cfg).unwrap();
        assert!(p.to_string_lossy().contains("/buzz-agent/oauth/demo/"));
        assert!(p.extension().and_then(|s| s.to_str()) == Some("json"));
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
}
