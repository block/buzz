//! Per-user Google account connection + instant Google Meet creation.
//!
//! Buzz's own Huddle voice feature has poor call quality (custom
//! relay-fanout audio, not WebRTC — see `huddle/mod.rs`). This is a
//! deliberately separate, much simpler alternative: each user connects their
//! own Google account (OAuth 2.0 + PKCE, no client-side secret required to
//! be confidential per RFC 8252), and "start a meeting" calls the Google
//! Meet API directly to create an instant meeting space, entirely offloading
//! call audio/video to Google's own infrastructure.
//!
//! Unlike Huddle, this needs **no new relay event kind** — BuilderLab's
//! hardcoded per-kind allowlist (the wall that killed adding video to
//! Huddle) never comes into play, because the resulting join link is posted
//! as an ordinary channel message, exactly like sharing any other URL.
//!
//! The OAuth loopback-server flow mirrors `builderlab.rs`'s
//! `start_builderlab_login` (local `TcpListener` + a tiny `axum` router +
//! `oneshot` channel + `app.opener()`), which is the proven pattern for
//! "open the system browser, wait for a redirect back to a local port" in
//! this codebase. Unlike Builderlab, there is no in-memory session — the
//! durable state is just the refresh token, stored in the OS keyring
//! (`secret_store::SecretStore`, the same store used for the Nostr identity
//! and agent API keys).

use std::{collections::HashMap, sync::Mutex, time::Duration};

use axum::{
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri_plugin_opener::OpenerExt;
use tokio::{net::TcpListener, sync::oneshot};
use url::Url;

/// Drive uploads, which reuse this module's account connection and token
/// refresh rather than owning a second OAuth client of their own.
pub(crate) mod drive;

use crate::app_state::keyring_service;
use crate::secret_store::SecretStore;

const GOOGLE_OAUTH_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_MEET_API_BASE: &str = "https://meet.googleapis.com/v2";
// Two scopes on one connection, both deliberately narrow:
//
// `meetings.space.created` is principal-scoped — a token carrying it can only
// see or manage spaces it itself created, matching "each user connects their
// own account" rather than a shared org-wide meeting inbox.
//
// `drive.file` is the per-file Drive scope: the app can create files and manage
// only the files it created, and can see nothing else in the user's Drive. It
// is **not** a restricted scope, so it needs no Google security assessment and
// the consent screen carries no scary warning. `drive.readonly`/`drive` — which
// browsing an existing folder would require — are both restricted, which is why
// folder browsing is not a feature. See `docs/google-drive-integration-spec.md`.
const GOOGLE_SCOPES: &str = concat!(
    "https://www.googleapis.com/auth/meetings.space.created",
    " ",
    "https://www.googleapis.com/auth/drive.file",
);
/// The Drive half of {@link GOOGLE_SCOPES}, matched against a refreshed token's
/// granted scope. Accounts connected before Drive shipped hold a token without
/// it; their refresh still succeeds, so without this check the first Drive
/// upload would fail with a raw 403 instead of "reconnect your account".
pub(crate) const GOOGLE_DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
// Named for Meet because that is what first stored it, and renaming the key
// would silently disconnect every account already connected.
const GOOGLE_MEET_REFRESH_TOKEN_KEY: &str = "google_meet_refresh_token";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Baked in at build time by `build.rs` from `BUZZ_BUILD_GOOGLE_MEET_CLIENT_ID`
/// — `option_env!`, not `env!`, because OSS/unconfigured builds must still
/// compile with this feature simply reporting "not configured" at runtime.
fn google_client_id() -> Option<&'static str> {
    option_env!("BUZZ_DESKTOP_BUILD_GOOGLE_MEET_CLIENT_ID")
}

/// Google's "Desktop app" OAuth client type issues a client secret, and its
/// token endpoint conventionally still expects it even though installed
/// apps can't keep it confidential (RFC 8252) — send it when configured,
/// but PKCE is what actually secures this flow, not the secret's secrecy.
fn google_client_secret() -> Option<&'static str> {
    option_env!("BUZZ_DESKTOP_BUILD_GOOGLE_MEET_CLIENT_SECRET")
}

#[derive(Default)]
pub(crate) struct GoogleMeetLogin(Mutex<Option<PendingLogin>>);

struct PendingLogin {
    id: uuid::Uuid,
    cancel: oneshot::Sender<()>,
}

struct CallbackState {
    nonce: String,
    sender: Mutex<Option<oneshot::Sender<Result<String, String>>>>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    /// Space-separated list of what Google actually granted, which is not
    /// necessarily what was asked for — a token minted before Drive shipped
    /// keeps its original, narrower scope across every refresh.
    #[serde(default)]
    scope: Option<String>,
}

/// A freshly refreshed access token plus what it is allowed to do.
pub(crate) struct GoogleAccessToken {
    pub access_token: String,
    granted_scope: String,
}

impl GoogleAccessToken {
    /// True if Google granted `scope` on this token. Compared over
    /// whitespace-separated entries rather than by substring, so one scope
    /// cannot be mistaken for another that merely contains it.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.granted_scope
            .split_whitespace()
            .any(|granted| granted == scope)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpaceResponse {
    #[serde(default)]
    meeting_uri: Option<String>,
    #[serde(default)]
    meeting_code: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoogleMeetInfo {
    meeting_uri: String,
    meeting_code: Option<String>,
}

/// Random, high-entropy PKCE code verifier (RFC 7636 wants 43-128 chars from
/// an unreserved-character alphabet). Three concatenated UUIDv4s give 48
/// bytes of OS-RNG-backed randomness without pulling in a `rand` dependency
/// this crate doesn't otherwise need; base64url-no-pad encoding both keeps
/// it in the allowed alphabet and lands comfortably in range (~64 chars).
fn generate_pkce_verifier() -> String {
    let mut bytes = Vec::with_capacity(48);
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

async fn oauth_callback(
    Path(nonce): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    AxumState(state): AxumState<std::sync::Arc<CallbackState>>,
) -> Response {
    if nonce != state.nonce {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }

    let result = match query.get("code").filter(|code| !code.is_empty()) {
        Some(code) => Ok(code.clone()),
        None => Err(query
            .get("error_description")
            .or_else(|| query.get("error"))
            .cloned()
            .unwrap_or_else(|| "Google did not return an authorization code".to_owned())),
    };
    if let Some(sender) = state
        .sender
        .lock()
        .expect("callback sender poisoned")
        .take()
    {
        let _ = sender.send(result);
    }

    Html(crate::builderlab::AUTH_COMPLETE_HTML).into_response()
}

/// Exchange an authorization code (first connect) or refresh token
/// (subsequent calls) for tokens. `params` carries the grant-specific pairs;
/// `client_id`/`client_secret` are appended here so callers don't repeat it.
async fn exchange_token(
    http_client: &reqwest::Client,
    client_id: &str,
    mut params: Vec<(&str, String)>,
) -> Result<TokenResponse, String> {
    params.push(("client_id", client_id.to_string()));
    if let Some(secret) = google_client_secret() {
        params.push(("client_secret", secret.to_string()));
    }

    let response = http_client
        .post(GOOGLE_OAUTH_TOKEN_URL)
        .form(&params)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| format!("Google token request failed: {error}"))?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Google token request failed: {body}"));
    }
    response
        .json()
        .await
        .map_err(|error| format!("invalid Google token response: {error}"))
}

/// Opens the system browser for Google sign-in, waits for the redirect back
/// to a local loopback port, exchanges the resulting code for tokens, and
/// stores the refresh token in the OS keyring. Requires `access_type=offline`
/// + `prompt=consent` so a refresh token is actually issued — Google omits
/// it on repeat consents without `prompt=consent`.
#[tauri::command]
pub(crate) async fn start_google_meet_connect(
    app: tauri::AppHandle,
    app_state: tauri::State<'_, crate::app_state::AppState>,
    login: tauri::State<'_, GoogleMeetLogin>,
) -> Result<(), String> {
    let client_id = google_client_id()
        .ok_or_else(|| "Google Meet is not configured for this build".to_owned())?;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| format!("could not start local authentication callback: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("could not read local authentication callback: {error}"))?
        .port();
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback/{nonce}");

    let (sender, receiver) = oneshot::channel();
    let callback_state = std::sync::Arc::new(CallbackState {
        nonce: nonce.clone(),
        sender: Mutex::new(Some(sender)),
    });
    let router = Router::new()
        .route("/callback/{nonce}", get(oauth_callback))
        .with_state(callback_state);
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let verifier = generate_pkce_verifier();
    let challenge = pkce_challenge(&verifier);

    let mut authorize_url = Url::parse(GOOGLE_OAUTH_AUTHORIZE_URL)
        .map_err(|error| format!("invalid Google authorization URL: {error}"))?;
    authorize_url
        .query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", GOOGLE_SCOPES)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");

    if let Err(error) = app.opener().open_url(authorize_url.as_str(), None::<&str>) {
        server.abort();
        return Err(format!("could not open Google sign-in: {error}"));
    }

    let login_id = uuid::Uuid::new_v4();
    let (cancel_sender, mut cancel_receiver) = oneshot::channel();
    {
        let mut pending = login.0.lock().map_err(|error| error.to_string())?;
        if let Some(previous) = pending.take() {
            let _ = previous.cancel.send(());
        }
        *pending = Some(PendingLogin {
            id: login_id,
            cancel: cancel_sender,
        });
    }

    let code = tokio::select! {
        result = tokio::time::timeout(LOGIN_TIMEOUT, receiver) => match result {
            Ok(Ok(Ok(code))) => code,
            Ok(Ok(Err(error))) => {
                server.abort();
                return Err(error);
            }
            Ok(Err(_)) => {
                server.abort();
                return Err("local authentication callback stopped unexpectedly".to_owned());
            }
            Err(_) => {
                server.abort();
                return Err("Google sign-in timed out".to_owned());
            }
        },
        _ = &mut cancel_receiver => {
            server.abort();
            return Err("Google sign-in canceled".to_owned());
        }
    };
    server.abort();

    {
        let mut pending = login.0.lock().map_err(|error| error.to_string())?;
        if pending
            .as_ref()
            .is_none_or(|pending| pending.id != login_id)
        {
            return Err("Google sign-in canceled".to_owned());
        }
        *pending = None;
    }

    let token = exchange_token(
        &app_state.http_client,
        client_id,
        vec![
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code".to_string()),
            ("code", code),
            ("code_verifier", verifier),
        ],
    )
    .await?;
    let refresh_token = token.refresh_token.ok_or_else(|| {
        "Google did not return a refresh token — reconnect and make sure to approve access when prompted".to_owned()
    })?;

    SecretStore::shared(keyring_service())
        .store(GOOGLE_MEET_REFRESH_TOKEN_KEY, &refresh_token)
        .map_err(|error| format!("could not save Google account: {error}"))?;

    Ok(())
}

#[tauri::command]
pub(crate) fn cancel_google_meet_connect(
    login: tauri::State<'_, GoogleMeetLogin>,
) -> Result<(), String> {
    if let Some(pending) = login.0.lock().map_err(|error| error.to_string())?.take() {
        let _ = pending.cancel.send(());
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn get_google_meet_connection_status() -> Result<bool, String> {
    Ok(SecretStore::shared(keyring_service())
        .load(GOOGLE_MEET_REFRESH_TOKEN_KEY)
        .map_err(|error| format!("could not check Google account status: {error}"))?
        .is_some())
}

#[tauri::command]
pub(crate) fn disconnect_google_meet_account() -> Result<(), String> {
    SecretStore::shared(keyring_service())
        .delete(GOOGLE_MEET_REFRESH_TOKEN_KEY)
        .map_err(|error| format!("could not disconnect Google account: {error}"))
}

/// Exchange the stored refresh token for a usable access token.
///
/// Shared by every Google-backed feature (Meet spaces, Drive uploads) so there
/// is one place that knows how the connection is stored and one definition of
/// what "your connection expired" means.
pub(crate) async fn google_access_token(
    app_state: &crate::app_state::AppState,
) -> Result<GoogleAccessToken, String> {
    let client_id =
        google_client_id().ok_or_else(|| "Google is not configured for this build".to_owned())?;
    let store = SecretStore::shared(keyring_service());
    let refresh_token = store
        .load(GOOGLE_MEET_REFRESH_TOKEN_KEY)
        .map_err(|error| format!("could not read Google account: {error}"))?
        .ok_or_else(|| "Connect your Google account first".to_owned())?;

    let refreshed = exchange_token(
        &app_state.http_client,
        client_id,
        vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token),
        ],
    )
    .await;
    let token = match refreshed {
        Ok(token) => token,
        Err(error) => {
            // A revoked/expired refresh token is the likeliest cause. Clear
            // the stale one so the UI cleanly falls back to "connect your
            // account" instead of retrying a token that will never work.
            let _ = store.delete(GOOGLE_MEET_REFRESH_TOKEN_KEY);
            return Err(format!(
                "Your Google account connection expired — reconnect it and try again ({error})"
            ));
        }
    };

    Ok(GoogleAccessToken {
        access_token: token.access_token,
        granted_scope: token.scope.unwrap_or_default(),
    })
}

/// Refreshes the stored token and creates a new, empty Google Meet space —
/// the "instant meeting" flow (no calendar event, no scheduled time).
#[tauri::command]
pub(crate) async fn create_instant_google_meet(
    app_state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<GoogleMeetInfo, String> {
    let token = google_access_token(&app_state).await?;

    let response = app_state
        .http_client
        .post(format!("{GOOGLE_MEET_API_BASE}/spaces"))
        .bearer_auth(&token.access_token)
        .json(&serde_json::json!({}))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| format!("could not create Google Meet: {error}"))?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("could not create Google Meet: {body}"));
    }
    let space: SpaceResponse = response
        .json()
        .await
        .map_err(|error| format!("invalid Google Meet response: {error}"))?;
    let meeting_uri = space
        .meeting_uri
        .ok_or_else(|| "Google Meet did not return a join link".to_owned())?;

    Ok(GoogleMeetInfo {
        meeting_uri,
        meeting_code: space.meeting_code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_verifier_is_in_range_and_url_safe() {
        let verifier = generate_pkce_verifier();
        assert!(verifier.len() >= 43 && verifier.len() <= 128);
        assert!(verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn pkce_challenge_is_deterministic_and_differs_from_verifier() {
        let verifier = generate_pkce_verifier();
        let challenge_a = pkce_challenge(&verifier);
        let challenge_b = pkce_challenge(&verifier);
        assert_eq!(challenge_a, challenge_b);
        assert_ne!(challenge_a, verifier);
    }
}
