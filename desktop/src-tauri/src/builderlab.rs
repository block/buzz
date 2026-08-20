use std::{collections::HashMap, sync::Mutex, time::Duration};

use axum::{
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use tauri_plugin_opener::OpenerExt;
use tokio::{net::TcpListener, sync::oneshot};
use url::Url;

const BUILDERLAB_API_BASE_URL: &str = "https://app.builderlab.xyz/api/goose";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const BB_SESSION_CREDENTIAL_HEADER: &str = "X-BB-Session-Credential";
// Builderlab enforces an Origin check on the identity bind endpoints. Browsers
// attach this automatically; the desktop reqwest client must set it explicitly
// or challenge/verify fail with `invalid_origin`. It also seeds the challenge
// body's `origin` field so both agree.
const BUILDERLAB_ORIGIN: &str = "https://app.builderlab.xyz";
const AUTH_COMPLETE_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Buzz authentication complete</title>
  <style>
    :root {
      color-scheme: light;
      font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      color: #231e1e;
      background: #d7d72e;
    }

    * {
      box-sizing: border-box;
    }

    body {
      min-height: 100vh;
      min-height: 100dvh;
      margin: 0;
      display: grid;
      place-items: center;
      padding: 24px;
      background-color: #d7d72e;
      background-image: radial-gradient(circle, rgba(35, 30, 30, 0.16) 1.2px, transparent 1.3px);
      background-size: 37px 37px;
    }

    main {
      width: min(100%, 560px);
      padding: clamp(32px, 8vw, 64px);
      border: 2px solid #231e1e;
      border-radius: 28px;
      background: #d7e7f6;
      box-shadow: 8px 8px 0 #231e1e;
    }

    .bee {
      display: block;
      width: 72px;
      height: auto;
      margin-bottom: 40px;
      color: #231e1e;
    }

    .eyebrow {
      display: inline-flex;
      align-items: center;
      min-height: 32px;
      margin: 0 0 20px;
      padding: 6px 14px;
      border-radius: 999px;
      background: #d7d72e;
      font-size: 14px;
      font-weight: 600;
      letter-spacing: 0.01em;
    }

    h1 {
      max-width: 440px;
      margin: 0;
      font-size: clamp(40px, 9vw, 64px);
      font-weight: 600;
      letter-spacing: -0.055em;
      line-height: 0.95;
    }

    p {
      max-width: 390px;
      margin: 24px 0 0;
      font-size: 18px;
      letter-spacing: -0.02em;
      line-height: 1.45;
    }

    @media (max-width: 480px) {
      body {
        padding: 16px;
      }

      main {
        padding: 32px 28px 36px;
        border-radius: 22px;
        box-shadow: 6px 6px 0 #231e1e;
      }

      .bee {
        width: 60px;
        margin-bottom: 32px;
      }
    }
  </style>
</head>
<body>
  <main>
    <svg class="bee" viewBox="0 0 466 309" role="img" aria-label="Buzz">
      <defs>
        <mask id="bee-mask">
          <rect width="466" height="309" fill="black"/>
          <circle cx="91.7" cy="154.5" r="91.7" fill="white"/>
          <circle cx="374.3" cy="154.5" r="91.7" fill="white"/>
          <rect x="128" width="210" height="309" rx="34" fill="white"/>
          <ellipse cx="193.3" cy="84.4" rx="27" ry="27" fill="black"/>
          <ellipse cx="276" cy="84.4" rx="27" ry="27" fill="black"/>
          <rect x="166.3" y="157.2" width="136.9" height="38.3" rx="5" fill="black"/>
          <rect x="166.9" y="235.1" width="136.2" height="37.6" rx="5" fill="black"/>
        </mask>
      </defs>
      <rect width="466" height="309" fill="currentColor" mask="url(#bee-mask)"/>
    </svg>
    <div class="eyebrow">Authentication complete</div>
    <h1>You&rsquo;re signed in.</h1>
    <p>You can close this window and return to Buzz.</p>
  </main>
</body>
</html>"#;

/// Keyring key holding the Builderlab session credential.
///
/// The credential used to live only in `BuilderlabSession`, i.e. in process
/// memory, so quitting the app signed the Hosted communities page out while
/// the Buzz identity itself — which *is* persisted — came back fine. Keep it
/// beside that identity in the OS keyring instead.
const SESSION_CREDENTIAL_KEY: &str = "builderlab.session";

fn credential_store() -> &'static crate::secret_store::SecretStore {
    crate::secret_store::SecretStore::shared(crate::app_state::keyring_service())
}

/// Persist the credential. Best-effort: a machine whose keyring is unreachable
/// still gets a working session for this run, which is what it had before.
fn persist_credential(credential: &str) {
    if let Err(error) = credential_store().store(SESSION_CREDENTIAL_KEY, credential) {
        tracing::warn!(%error, "builderlab: could not persist the session credential");
    }
}

/// Read the persisted credential.
///
/// A keyring that cannot be read is *not* "no credential": reporting the user
/// as signed out when the store is merely unavailable invites them to sign in
/// again over a session that is still perfectly good. The error is returned so
/// the caller can say the storage failed.
fn stored_credential() -> Result<Option<String>, String> {
    credential_store()
        .load(SESSION_CREDENTIAL_KEY)
        .map_err(|error| format!("could not read the stored Builderlab session: {error}"))
}

/// Why a `/v1/auth/me` check did not return a user.
///
/// Only an explicit authentication rejection proves the credential is bad. A
/// timeout, a DNS failure, a 5xx, a 429, or a body that does not parse says
/// nothing about the credential — deleting it on those turns a blip in the
/// service into a permanent sign-out, since the credential is gone from the
/// keyring as well as from memory.
#[derive(Debug)]
enum SessionCheckError {
    /// The service rejected the credential itself (HTTP 401 or 403).
    Rejected(String),
    /// Anything else. The credential is kept for the next attempt.
    Transient(String),
}

impl SessionCheckError {
    fn message(self) -> String {
        match self {
            SessionCheckError::Rejected(message) | SessionCheckError::Transient(message) => message,
        }
    }

    fn invalidates_credential(&self) -> bool {
        matches!(self, SessionCheckError::Rejected(_))
    }
}

/// Whether an HTTP status is the service saying "this credential is not good".
///
/// 401 and 403 only. 429 in particular is *not* here: rate limiting says the
/// caller asked too often, not that the session expired.
fn status_rejects_credential(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN
}

/// The credential, plus the generation it was read at.
///
/// Session checks are `await`ed, so a login or a logout can land while one is
/// in flight. Every mutation bumps `generation`, and a check applies its result
/// only if the generation it started from is still current — otherwise an older
/// rejected request deletes a credential that was just exchanged, or an older
/// success reports auth for a session the user already signed out of.
#[derive(Default)]
struct SessionState {
    stored: Option<StoredSession>,
    generation: u64,
}

#[derive(Default)]
pub(crate) struct BuilderlabSession(Mutex<SessionState>);

impl BuilderlabSession {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, SessionState>, String> {
        self.0.lock().map_err(|error| error.to_string())
    }

    /// The current credential and generation, without touching the keyring.
    fn current(&self) -> Result<Option<(String, u64)>, String> {
        let state = self.lock()?;
        Ok(state
            .stored
            .as_ref()
            .map(|stored| (stored.credential.clone(), state.generation)))
    }

    /// Install a freshly exchanged credential in memory and in the keyring.
    ///
    /// Both stores are written under the lock so a concurrent hydrate or clear
    /// cannot interleave between them and leave the two disagreeing.
    fn replace(&self, credential: String) -> Result<(), String> {
        let mut state = self.lock()?;
        persist_credential(&credential);
        state.stored = Some(StoredSession { credential });
        state.generation += 1;
        Ok(())
    }

    /// Adopt the persisted credential into memory, if memory is still empty.
    ///
    /// Returns the credential now in effect. If a login landed while the
    /// keyring was being read, that newer credential wins — hydration must
    /// never resurrect an older session over it, nor over a logout.
    fn hydrate(&self, credential: String) -> Result<(String, u64), String> {
        let mut state = self.lock()?;
        if let Some(stored) = state.stored.as_ref() {
            return Ok((stored.credential.clone(), state.generation));
        }
        state.stored = Some(StoredSession {
            credential: credential.clone(),
        });
        state.generation += 1;
        Ok((credential, state.generation))
    }

    /// Drop the credential from memory and from the keyring.
    ///
    /// The keyring failure is returned rather than logged: reporting a
    /// successful sign-out while the credential is still on disk means the next
    /// launch hydrates it and silently signs the user back in.
    fn clear(&self) -> Result<(), String> {
        let mut state = self.lock()?;
        state.stored = None;
        state.generation += 1;
        credential_store()
            .delete(SESSION_CREDENTIAL_KEY)
            .map_err(|error| format!("could not delete the stored Builderlab session: {error}"))
    }

    /// Drop the credential, but only if it is still the one that was checked.
    fn clear_if_current(&self, generation: u64) -> Result<(), String> {
        {
            let state = self.lock()?;
            if state.generation != generation {
                // A login or logout landed while the check was in flight; its
                // verdict is about a credential that is no longer in use.
                return Ok(());
            }
        }
        self.clear()
    }
}

#[derive(Default)]
pub(crate) struct BuilderlabLogin(Mutex<Option<PendingLogin>>);

struct PendingLogin {
    id: uuid::Uuid,
    cancel: oneshot::Sender<()>,
}

struct StoredSession {
    credential: String,
}

#[derive(Debug, Deserialize)]
struct LoginExchangeResponse {
    session_credential: String,
    expires_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BuilderlabAuthInfo {
    expires_at: String,
    email: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthMeResponse {
    email: Option<String>,
    name: Option<String>,
    expires_at: String,
}

struct CallbackState {
    nonce: String,
    sender: Mutex<Option<oneshot::Sender<Result<String, String>>>>,
}

async fn login_callback(
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
            .unwrap_or_else(|| "Authentication callback did not include a code".to_owned())),
    };
    if let Some(sender) = state
        .sender
        .lock()
        .expect("callback sender poisoned")
        .take()
    {
        let _ = sender.send(result);
    }

    Html(AUTH_COMPLETE_HTML).into_response()
}

fn api_url(path: &str) -> Result<Url, String> {
    Url::parse(&format!("{BUILDERLAB_API_BASE_URL}{path}"))
        .map_err(|error| format!("invalid Builderlab API URL: {error}"))
}

fn login_url(return_to: &str) -> Result<Url, String> {
    let mut login_url = api_url("/v1/auth/login")?;
    login_url
        .query_pairs_mut()
        .append_pair("type", "cli")
        .append_pair("product", "buzz")
        .append_pair("returnTo", return_to);
    Ok(login_url)
}

async fn authenticated_user(
    client: &reqwest::Client,
    credential: &str,
) -> Result<AuthMeResponse, SessionCheckError> {
    let url = api_url("/v1/auth/me").map_err(SessionCheckError::Transient)?;
    let response = client
        .get(url)
        .header(BB_SESSION_CREDENTIAL_HEADER, credential)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| {
            SessionCheckError::Transient(format!("Builderlab session check failed: {error}"))
        })?;
    let status = response.status();
    if !status.is_success() {
        let message = format!("Builderlab session check failed with HTTP {status}");
        return Err(if status_rejects_credential(status) {
            SessionCheckError::Rejected(message)
        } else {
            SessionCheckError::Transient(message)
        });
    }
    // A success status with a body we cannot read says nothing about the
    // credential — a proxy or a deploy mid-flight can produce it.
    response.json().await.map_err(|error| {
        SessionCheckError::Transient(format!("invalid Builderlab session response: {error}"))
    })
}

#[tauri::command]
pub(crate) async fn start_builderlab_login(
    app: tauri::AppHandle,
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
    login: tauri::State<'_, BuilderlabLogin>,
) -> Result<BuilderlabAuthInfo, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| format!("could not start local authentication callback: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("could not read local authentication callback: {error}"))?
        .port();
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let return_to = format!("http://127.0.0.1:{port}/callback/{nonce}");
    let (sender, receiver) = oneshot::channel();
    let callback_state = std::sync::Arc::new(CallbackState {
        nonce: nonce.clone(),
        sender: Mutex::new(Some(sender)),
    });
    let router = Router::new()
        .route("/callback/{nonce}", get(login_callback))
        .with_state(callback_state);
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let login_url = login_url(&return_to)?;
    if let Err(error) = app.opener().open_url(login_url.as_str(), None::<&str>) {
        server.abort();
        return Err(format!("could not open Builderlab authentication: {error}"));
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

    let exchange_code = tokio::select! {
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
                return Err("Builderlab authentication timed out".to_owned());
            }
        },
        _ = &mut cancel_receiver => {
            server.abort();
            return Err("Builderlab authentication canceled".to_owned());
        }
    };
    server.abort();

    let response = app_state
        .http_client
        .post(api_url("/v1/auth/login/exchange")?)
        .json(&serde_json::json!({ "code": exchange_code }))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| format!("Builderlab code exchange failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Builderlab code exchange failed with HTTP {}",
            response.status()
        ));
    }
    let exchanged: LoginExchangeResponse = response
        .json()
        .await
        .map_err(|error| format!("invalid Builderlab code exchange response: {error}"))?;
    if exchanged.session_credential.is_empty() {
        return Err("Builderlab code exchange returned an empty credential".to_owned());
    }

    let me = authenticated_user(&app_state.http_client, &exchanged.session_credential)
        .await
        .map_err(SessionCheckError::message)?;
    if exchanged.expires_at != me.expires_at {
        return Err("Builderlab session expiry did not match code exchange".to_owned());
    }
    let info = BuilderlabAuthInfo {
        expires_at: me.expires_at.clone(),
        email: me.email,
        name: me.name,
    };
    {
        let mut pending = login.0.lock().map_err(|error| error.to_string())?;
        if pending
            .as_ref()
            .is_none_or(|pending| pending.id != login_id)
        {
            return Err("Builderlab authentication canceled".to_owned());
        }
        *pending = None;
    }
    session.replace(exchanged.session_credential)?;
    Ok(info)
}

#[tauri::command]
pub(crate) async fn get_builderlab_auth(
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<Option<BuilderlabAuthInfo>, String> {
    // A fresh process has nothing in memory; the credential from the last run
    // is in the keyring. Hydrate before deciding the page is signed out.
    let (credential, generation) = match session.current()? {
        Some(current) => current,
        None => {
            let Some(persisted) = stored_credential()? else {
                return Ok(None);
            };
            session.hydrate(persisted)?
        }
    };
    match authenticated_user(&app_state.http_client, &credential).await {
        Ok(me) => {
            // Only report auth for the session still in effect. An older check
            // completing after a logout would otherwise present the user as
            // signed in to an account they just left.
            if session.current()?.is_none_or(|(_, now)| now != generation) {
                return Ok(None);
            }
            Ok(Some(BuilderlabAuthInfo {
                expires_at: me.expires_at,
                email: me.email,
                name: me.name,
            }))
        }
        Err(error) => {
            // Only an explicit rejection means the credential is bad. On a
            // timeout, a 5xx or a malformed body it is kept, so a blip in the
            // service does not sign the user out of the next launch too.
            if error.invalidates_credential() {
                session.clear_if_current(generation)?;
            }
            Err(error.message())
        }
    }
}

#[tauri::command]
pub(crate) fn cancel_builderlab_login(
    login: tauri::State<'_, BuilderlabLogin>,
) -> Result<(), String> {
    if let Some(pending) = login.0.lock().map_err(|error| error.to_string())?.take() {
        let _ = pending.cancel.send(());
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn clear_builderlab_auth(
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<(), String> {
    session.clear()
}

#[derive(Debug, Deserialize)]
struct NostrIdentityChallenge {
    challenge_id: String,
    nonce: String,
    verification_code: String,
    origin: String,
    expires_at: String,
}

async fn authenticated_json(
    client: &reqwest::Client,
    session: &BuilderlabSession,
    method: reqwest::Method,
    path: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let credential = session
        .current()?
        .map(|(credential, _generation)| credential)
        .ok_or_else(|| "Sign in to Builderlab first".to_owned())?;
    let response = client
        .request(method, api_url(path)?)
        .header(BB_SESSION_CREDENTIAL_HEADER, credential)
        .header(reqwest::header::ORIGIN, BUILDERLAB_ORIGIN)
        .json(&body)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|error| format!("Builderlab request failed: {error}"))?;
    let status = response.status();
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("invalid Builderlab response: {error}"))?;
    if !status.is_success() {
        // Builderlab error responses carry a structured `{ error: { code,
        // message, setup_needed, ... } }` body. Pass those through as `Ok` so the
        // frontend's typed handling and friendly per-code messages apply, instead
        // of surfacing a raw JSON blob. Only fall back to a plain string when the
        // body isn't the expected shape.
        if value.get("error").is_some() {
            return Ok(value);
        }
        return Err(format!("Builderlab request failed (HTTP {status})."));
    }
    Ok(value)
}

#[tauri::command]
pub(crate) async fn get_builderlab_nostr_identity(
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<serde_json::Value, String> {
    authenticated_json(
        &app_state.http_client,
        &session,
        reqwest::Method::POST,
        "/v1/buzz/nostr-identities/current",
        serde_json::json!({}),
    )
    .await
}

#[tauri::command]
pub(crate) async fn bind_builderlab_nostr_identity(
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<serde_json::Value, String> {
    let challenge_value = authenticated_json(
        &app_state.http_client,
        &session,
        reqwest::Method::POST,
        "/v1/buzz/nostr-identities/challenge",
        serde_json::json!({ "origin": BUILDERLAB_ORIGIN }),
    )
    .await?;
    // A structured error here (e.g. missing_mapping) arrives as an object with an
    // `error` field rather than a challenge — hand it straight back so the
    // frontend maps it to a friendly message instead of hitting a deserialize
    // failure below.
    if challenge_value.get("error").is_some() {
        return Ok(challenge_value);
    }
    let challenge: NostrIdentityChallenge = serde_json::from_value(challenge_value)
        .map_err(|error| format!("invalid Nostr identity challenge: {error}"))?;
    let keys = app_state.signing_keys()?;
    let event = crate::commands::build_nostr_identity_binding_event(
        &keys,
        &challenge.challenge_id,
        &challenge.nonce,
        &challenge.verification_code,
        &challenge.origin,
        &challenge.expires_at,
    )?;
    authenticated_json(
        &app_state.http_client,
        &session,
        reqwest::Method::POST,
        "/v1/buzz/nostr-identities/verify",
        serde_json::json!({
            "challenge_id": challenge.challenge_id,
            "nonce": challenge.nonce,
            "signed_payload": nostr::JsonUtil::as_json(&event),
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn delete_builderlab_nostr_identity(
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<serde_json::Value, String> {
    authenticated_json(
        &app_state.http_client,
        &session,
        reqwest::Method::POST,
        "/v1/buzz/nostr-identities/delete",
        serde_json::json!({}),
    )
    .await
}

#[tauri::command]
pub(crate) async fn list_builderlab_communities(
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<serde_json::Value, String> {
    authenticated_json(
        &app_state.http_client,
        &session,
        reqwest::Method::POST,
        "/v1/buzz/communities/list",
        serde_json::json!({}),
    )
    .await
}

#[tauri::command]
pub(crate) async fn check_builderlab_community_name(
    name: String,
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<serde_json::Value, String> {
    authenticated_json(
        &app_state.http_client,
        &session,
        reqwest::Method::POST,
        "/v1/buzz/communities/availability",
        serde_json::json!({ "name": name }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn create_builderlab_community(
    name: String,
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<serde_json::Value, String> {
    authenticated_json(
        &app_state.http_client,
        &session,
        reqwest::Method::POST,
        "/v1/buzz/communities",
        serde_json::json!({ "name": name }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn archive_builderlab_community(
    community_id: String,
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<serde_json::Value, String> {
    authenticated_json(
        &app_state.http_client,
        &session,
        reqwest::Method::POST,
        "/v1/buzz/communities/archive",
        serde_json::json!({ "community_id": community_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn unarchive_builderlab_community(
    community_id: String,
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<serde_json::Value, String> {
    authenticated_json(
        &app_state.http_client,
        &session,
        reqwest::Method::POST,
        "/v1/buzz/communities/unarchive",
        serde_json::json!({ "community_id": community_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn transfer_builderlab_community(
    community_id: String,
    transferee_npub: String,
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<serde_json::Value, String> {
    // The Builderlab transfer endpoint expects camelCase keys, unlike the
    // archive/unarchive endpoints which take `community_id`; mirror the web
    // client's payload exactly.
    authenticated_json(
        &app_state.http_client,
        &session,
        reqwest::Method::POST,
        "/v1/buzz/communities/transfer",
        serde_json::json!({
            "communityId": community_id,
            "transfereeNpub": transferee_npub,
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_complete_page_uses_buzz_brand() {
        for expected in [
            "<title>Buzz authentication complete</title>",
            "#d7d72e",
            "#231e1e",
            "#d7e7f6",
            "aria-label=\"Buzz\"",
            "return to Buzz",
        ] {
            assert!(
                AUTH_COMPLETE_HTML.contains(expected),
                "authentication complete page is missing {expected}"
            );
        }
    }

    #[test]
    fn api_paths_stay_on_builderlab_api_origin() {
        let login = api_url("/v1/auth/login").unwrap();
        assert_eq!(
            login.origin().ascii_serialization(),
            "https://app.builderlab.xyz"
        );
        assert_eq!(login.path(), "/api/goose/v1/auth/login");
    }

    #[test]
    fn login_defaults_to_auth0_login() {
        let login = login_url("http://127.0.0.1:1234/callback/nonce").unwrap();
        let query: HashMap<_, _> = login.query_pairs().into_owned().collect();

        assert_eq!(query.get("type").map(String::as_str), Some("cli"));
        assert_eq!(query.get("product").map(String::as_str), Some("buzz"));
        assert_eq!(
            query.get("returnTo").map(String::as_str),
            Some("http://127.0.0.1:1234/callback/nonce")
        );
        assert!(!query.contains_key("screen_hint"));
    }

    #[test]
    fn only_a_confirmed_authentication_rejection_invalidates_a_session() {
        use reqwest::StatusCode;

        // The service saying "not you".
        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            assert!(
                status_rejects_credential(status),
                "HTTP {status} must invalidate the credential"
            );
        }

        // Everything else is the service having a bad day. Deleting the
        // credential on these turns a blip into a permanent sign-out, because
        // it is removed from the keyring as well as from memory.
        for status in [
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::NOT_FOUND,
            StatusCode::BAD_REQUEST,
        ] {
            assert!(
                !status_rejects_credential(status),
                "HTTP {status} must preserve the credential for a retry"
            );
        }
    }

    #[test]
    fn transient_failures_do_not_invalidate_but_rejections_do() {
        assert!(SessionCheckError::Rejected("nope".into()).invalidates_credential());
        // A transport failure, a timeout, or a body that would not parse.
        assert!(!SessionCheckError::Transient("dns".into()).invalidates_credential());
        assert_eq!(
            SessionCheckError::Transient("dns".into()).message(),
            "dns",
            "the caller still reports what went wrong"
        );
    }

    #[test]
    fn a_login_during_a_check_survives_that_check_s_rejection() {
        // The race: a check reads generation N, the user signs in again while
        // it is in flight, and the stale 401 comes back. It must not delete
        // the credential that just replaced the one it checked.
        let session = BuilderlabSession::default();
        session.0.lock().unwrap().stored.replace(StoredSession {
            credential: "old".into(),
        });
        let (credential, generation) = session.current().unwrap().expect("a session");
        assert_eq!(credential, "old");

        // A newer login lands, bumping the generation.
        {
            let mut state = session.0.lock().unwrap();
            state.stored = Some(StoredSession {
                credential: "new".into(),
            });
            state.generation += 1;
        }

        session
            .clear_if_current(generation)
            .expect("a stale verdict must be a no-op");

        let (credential, _) = session.current().unwrap().expect("session must survive");
        assert_eq!(
            credential, "new",
            "a stale rejection must not delete the credential that replaced it"
        );
    }

    #[test]
    fn hydration_never_overwrites_a_newer_credential() {
        // The keyring read is not instant, so a login can land first. The
        // credential from disk must not resurrect over it.
        let session = BuilderlabSession::default();
        {
            let mut state = session.0.lock().unwrap();
            state.stored = Some(StoredSession {
                credential: "new".into(),
            });
            state.generation += 1;
        }

        let (credential, _) = session.hydrate("from-disk".into()).unwrap();
        assert_eq!(
            credential, "new",
            "hydration must yield to the credential already in memory"
        );
    }
}
