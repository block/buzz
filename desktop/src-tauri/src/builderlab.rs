use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::Duration,
};

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

const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const ENTERPRISE_LOGIN_ATTEMPT_PREFIX: &str = "enterprise-login-";
const CANCELED_LOGIN_ATTEMPT_TOMBSTONE_LIMIT: usize = 64;
const BB_SESSION_CREDENTIAL_HEADER: &str = "X-BB-Session-Credential";
// Builderlab enforces an Origin check on the identity bind endpoints. Browsers
// attach this automatically; the desktop reqwest client must set it explicitly
// or challenge/verify fail with `invalid_origin`. It also seeds the challenge
// body's `origin` field so both agree.
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

#[derive(Default)]
pub(crate) struct BuilderlabSession(Mutex<Option<StoredSession>>);

#[derive(Default)]
pub(crate) struct BuilderlabLogin(Mutex<BuilderlabLoginState>);

#[derive(Default)]
struct BuilderlabLoginState {
    pending: Option<PendingLogin>,
    canceled_attempts: VecDeque<String>,
}

struct PendingLogin {
    id: String,
    cancel: oneshot::Sender<()>,
}

struct StoredSession {
    credential: String,
}

#[derive(Debug, Deserialize)]
struct LoginExchangeResponse {
    session_credential: String,
    expires_at: String,
    username: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BuilderlabAuthInfo {
    expires_at: String,
    email: Option<String>,
    username: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthMeResponse {
    email: Option<String>,
    expires_at: String,
    username: Option<String>,
    name: Option<String>,
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

fn builderlab_api_base_url() -> &'static str {
    option_env!("BUZZ_DESKTOP_BUILD_BUILDERLAB_API_BASE_URL")
        .unwrap_or("https://app.builderlab.xyz/api/goose")
}

fn builderlab_origin() -> Result<String, String> {
    let url = Url::parse(builderlab_api_base_url())
        .map_err(|error| format!("invalid Builderlab API URL: {error}"))?;
    Ok(url.origin().ascii_serialization())
}

fn api_url(path: &str) -> Result<Url, String> {
    Url::parse(&format!("{}{}", builderlab_api_base_url(), path))
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

fn remember_canceled_attempt(state: &mut BuilderlabLoginState, attempt_id: String) {
    if state
        .canceled_attempts
        .iter()
        .any(|canceled| canceled == &attempt_id)
    {
        return;
    }
    if state.canceled_attempts.len() >= CANCELED_LOGIN_ATTEMPT_TOMBSTONE_LIMIT {
        state.canceled_attempts.pop_front();
    }
    state.canceled_attempts.push_back(attempt_id);
}

fn is_canceled_attempt(state: &BuilderlabLoginState, attempt_id: &str) -> bool {
    state
        .canceled_attempts
        .iter()
        .any(|canceled| canceled == attempt_id)
}

fn clear_matching_pending_login(login: &BuilderlabLogin, login_id: &str) -> Result<(), String> {
    let mut state = login.0.lock().map_err(|error| error.to_string())?;
    if state
        .pending
        .as_ref()
        .is_some_and(|pending| pending.id == login_id)
    {
        state.pending = None;
    }
    Ok(())
}

fn register_pending_builderlab_login(
    login: &BuilderlabLogin,
    login_id: &str,
    cancel: oneshot::Sender<()>,
    is_enterprise_attempt: bool,
) -> Result<bool, String> {
    let mut state = login.0.lock().map_err(|error| error.to_string())?;
    if is_enterprise_attempt && is_canceled_attempt(&state, login_id) {
        return Ok(false);
    }
    if state.pending.is_some() {
        if is_enterprise_attempt
            && !state
                .pending
                .as_ref()
                .is_some_and(|pending| pending.id.starts_with(ENTERPRISE_LOGIN_ATTEMPT_PREFIX))
        {
            return Err("Builderlab authentication is already in progress".to_owned());
        }
        if let Some(previous) = state.pending.take() {
            let _ = previous.cancel.send(());
        }
    }
    state.pending = Some(PendingLogin {
        id: login_id.to_owned(),
        cancel,
    });
    Ok(true)
}

fn commit_builderlab_login_session(
    login: &BuilderlabLogin,
    session: &BuilderlabSession,
    login_id: &str,
    credential: String,
) -> Result<(), String> {
    commit_builderlab_login_session_with_hook(login, session, login_id, credential, || {})
}

fn commit_builderlab_login_session_with_hook(
    login: &BuilderlabLogin,
    session: &BuilderlabSession,
    login_id: &str,
    credential: String,
    before_session_lock: impl FnOnce(),
) -> Result<(), String> {
    // Keep this lock order consistent everywhere a Builderlab login result is
    // committed: first BuilderlabLogin, then BuilderlabSession. Holding the login
    // lock through the session write prevents cancel/replacement from
    // interleaving after the ownership check but before credential storage.
    let mut login_state = login.0.lock().map_err(|error| error.to_string())?;
    if login_state
        .pending
        .as_ref()
        .is_none_or(|pending| pending.id != login_id)
    {
        return Err("Builderlab authentication canceled".to_owned());
    }
    before_session_lock();
    *session.0.lock().map_err(|error| error.to_string())? = Some(StoredSession { credential });
    login_state.pending = None;
    Ok(())
}

async fn abort_builderlab_login(
    login: &BuilderlabLogin,
    server: tokio::task::JoinHandle<()>,
    login_id: &str,
    error: String,
) -> Result<BuilderlabAuthInfo, String> {
    server.abort();
    clear_matching_pending_login(login, login_id)?;
    Err(error)
}

async fn authenticated_user(
    client: &reqwest::Client,
    credential: &str,
) -> Result<AuthMeResponse, String> {
    let response = client
        .get(api_url("/v1/auth/me")?)
        .header(BB_SESSION_CREDENTIAL_HEADER, credential)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| format!("Builderlab session check failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Builderlab session check failed with HTTP {}",
            response.status()
        ));
    }
    response
        .json()
        .await
        .map_err(|error| format!("invalid Builderlab session response: {error}"))
}

#[tauri::command]
pub(crate) async fn start_builderlab_login(
    app: tauri::AppHandle,
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
    login: tauri::State<'_, BuilderlabLogin>,
    attempt_id: Option<String>,
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

    let login_id = attempt_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let is_enterprise_attempt = attempt_id.is_some();
    let (cancel_sender, mut cancel_receiver) = oneshot::channel();
    match register_pending_builderlab_login(&login, &login_id, cancel_sender, is_enterprise_attempt)
    {
        Ok(true) => {}
        Ok(false) => {
            server.abort();
            return Err("Builderlab authentication canceled".to_owned());
        }
        Err(error) => {
            server.abort();
            return Err(error);
        }
    }

    let login_url = match login_url(&return_to) {
        Ok(login_url) => login_url,
        Err(error) => return abort_builderlab_login(&login, server, &login_id, error).await,
    };
    if let Err(error) = app.opener().open_url(login_url.as_str(), None::<&str>) {
        return abort_builderlab_login(
            &login,
            server,
            &login_id,
            format!("could not open Builderlab authentication: {error}"),
        )
        .await;
    }

    let exchange_code = tokio::select! {
        result = tokio::time::timeout(LOGIN_TIMEOUT, receiver) => match result {
            Ok(Ok(Ok(code))) => code,
            Ok(Ok(Err(error))) => {
                return abort_builderlab_login(&login, server, &login_id, error).await;
            }
            Ok(Err(_)) => {
                return abort_builderlab_login(
                    &login,
                    server,
                    &login_id,
                    "local authentication callback stopped unexpectedly".to_owned(),
                )
                .await;
            }
            Err(_) => {
                return abort_builderlab_login(
                    &login,
                    server,
                    &login_id,
                    "Builderlab authentication timed out".to_owned(),
                )
                .await;
            }
        },
        _ = &mut cancel_receiver => {
            return abort_builderlab_login(
                &login,
                server,
                &login_id,
                "Builderlab authentication canceled".to_owned(),
            )
            .await;
        }
    };
    server.abort();

    let exchange_url = match api_url("/v1/auth/login/exchange") {
        Ok(exchange_url) => exchange_url,
        Err(error) => {
            clear_matching_pending_login(&login, &login_id)?;
            return Err(error);
        }
    };
    let response = match app_state
        .http_client
        .post(exchange_url)
        .json(&serde_json::json!({ "code": exchange_code }))
        .timeout(Duration::from_secs(30))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            clear_matching_pending_login(&login, &login_id)?;
            return Err(format!("Builderlab code exchange failed: {error}"));
        }
    };
    if !response.status().is_success() {
        let status = response.status();
        clear_matching_pending_login(&login, &login_id)?;
        return Err(format!(
            "Builderlab code exchange failed with HTTP {status}"
        ));
    }
    let exchanged: LoginExchangeResponse = match response.json().await {
        Ok(exchanged) => exchanged,
        Err(error) => {
            clear_matching_pending_login(&login, &login_id)?;
            return Err(format!(
                "invalid Builderlab code exchange response: {error}"
            ));
        }
    };
    if exchanged.session_credential.is_empty() {
        clear_matching_pending_login(&login, &login_id)?;
        return Err("Builderlab code exchange returned an empty credential".to_owned());
    }

    let me = match authenticated_user(&app_state.http_client, &exchanged.session_credential).await {
        Ok(me) => me,
        Err(error) => {
            clear_matching_pending_login(&login, &login_id)?;
            return Err(error);
        }
    };
    if exchanged.expires_at != me.expires_at {
        clear_matching_pending_login(&login, &login_id)?;
        return Err("Builderlab session expiry did not match code exchange".to_owned());
    }
    let info = BuilderlabAuthInfo {
        expires_at: me.expires_at.clone(),
        email: me.email,
        username: normalized_auth_field(me.username)
            .or_else(|| normalized_auth_field(exchanged.username)),
        name: normalized_auth_field(me.name).or_else(|| normalized_auth_field(exchanged.name)),
    };
    commit_builderlab_login_session(&login, &session, &login_id, exchanged.session_credential)?;
    Ok(info)
}

#[tauri::command]
pub(crate) async fn get_builderlab_auth(
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<Option<BuilderlabAuthInfo>, String> {
    let stored = session
        .0
        .lock()
        .map_err(|error| error.to_string())?
        .as_ref()
        .map(|stored| stored.credential.clone());
    let Some(credential) = stored else {
        return Ok(None);
    };
    match authenticated_user(&app_state.http_client, &credential).await {
        Ok(me) => Ok(Some(BuilderlabAuthInfo {
            expires_at: me.expires_at,
            email: me.email,
            username: normalized_auth_field(me.username),
            name: normalized_auth_field(me.name),
        })),
        Err(error) => {
            *session
                .0
                .lock()
                .map_err(|lock_error| lock_error.to_string())? = None;
            Err(error)
        }
    }
}

fn normalized_auth_field(value: Option<String>) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn cancel_builderlab_login_attempt(
    login: &BuilderlabLogin,
    attempt_id: Option<&str>,
) -> Result<bool, String> {
    let mut state = login.0.lock().map_err(|error| error.to_string())?;
    if let Some(attempt_id) = attempt_id {
        remember_canceled_attempt(&mut state, attempt_id.to_owned());
    }
    let should_cancel = match (state.pending.as_ref(), attempt_id) {
        (Some(login), Some(attempt_id)) => login.id == attempt_id,
        (Some(_), None) => true,
        (None, _) => false,
    };
    if should_cancel {
        if let Some(pending) = state.pending.take() {
            let _ = pending.cancel.send(());
        }
    }
    Ok(should_cancel)
}

#[tauri::command]
pub(crate) fn cancel_builderlab_login(
    login: tauri::State<'_, BuilderlabLogin>,
    attempt_id: Option<String>,
) -> Result<(), String> {
    cancel_builderlab_login_attempt(&login, attempt_id.as_deref()).map(|_| ())
}

#[tauri::command]
pub(crate) fn clear_builderlab_auth(
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<(), String> {
    *session.0.lock().map_err(|error| error.to_string())? = None;
    Ok(())
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
        .0
        .lock()
        .map_err(|error| error.to_string())?
        .as_ref()
        .map(|stored| stored.credential.clone())
        .ok_or_else(|| "Sign in to Builderlab first".to_owned())?;
    let response = client
        .request(method, api_url(path)?)
        .header(BB_SESSION_CREDENTIAL_HEADER, credential)
        .header(reqwest::header::ORIGIN, builderlab_origin()?)
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
        serde_json::json!({ "origin": builderlab_origin()? }),
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
    fn cancel_with_attempt_id_only_cancels_matching_pending_login() {
        let login = BuilderlabLogin::default();
        let (cancel, mut receiver) = oneshot::channel();
        login.0.lock().unwrap().pending = Some(PendingLogin {
            id: "attempt-a".to_owned(),
            cancel,
        });

        assert!(!cancel_builderlab_login_attempt(&login, Some("attempt-b")).unwrap());
        assert!(login.0.lock().unwrap().pending.is_some());
        assert!(receiver.try_recv().is_err());

        assert!(cancel_builderlab_login_attempt(&login, Some("attempt-a")).unwrap());
        assert!(login.0.lock().unwrap().pending.is_none());
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn cancel_before_registration_tombstones_enterprise_attempt() {
        let login = BuilderlabLogin::default();
        assert!(!cancel_builderlab_login_attempt(&login, Some("enterprise-login-a")).unwrap());

        let (cancel, _receiver) = oneshot::channel();
        assert!(
            !register_pending_builderlab_login(&login, "enterprise-login-a", cancel, true,)
                .unwrap()
        );
        assert!(login.0.lock().unwrap().pending.is_none());
    }

    #[test]
    fn cancel_before_registration_does_not_block_legacy_attempts() {
        let login = BuilderlabLogin::default();
        assert!(!cancel_builderlab_login_attempt(&login, Some("legacy-flow")).unwrap());

        let (cancel, _receiver) = oneshot::channel();
        assert!(register_pending_builderlab_login(&login, "legacy-flow", cancel, false).unwrap());
        assert_eq!(
            login
                .0
                .lock()
                .unwrap()
                .pending
                .as_ref()
                .map(|pending| pending.id.as_str()),
            Some("legacy-flow"),
        );
    }

    #[test]
    fn canceled_attempt_tombstones_are_bounded() {
        let login = BuilderlabLogin::default();
        for index in 0..(CANCELED_LOGIN_ATTEMPT_TOMBSTONE_LIMIT + 5) {
            let attempt_id = format!("enterprise-login-{index}");
            assert!(!cancel_builderlab_login_attempt(&login, Some(&attempt_id)).unwrap());
        }

        let state = login.0.lock().unwrap();
        assert_eq!(
            state.canceled_attempts.len(),
            CANCELED_LOGIN_ATTEMPT_TOMBSTONE_LIMIT,
        );
        assert_eq!(
            state.canceled_attempts.front().map(String::as_str),
            Some("enterprise-login-5"),
        );
    }

    #[test]
    fn stale_attempt_cannot_commit_session_after_replacement() {
        let login = BuilderlabLogin::default();
        let session = BuilderlabSession::default();
        let (cancel, _receiver) = oneshot::channel();
        login.0.lock().unwrap().pending = Some(PendingLogin {
            id: "enterprise-login-new".to_owned(),
            cancel,
        });

        let result = commit_builderlab_login_session(
            &login,
            &session,
            "enterprise-login-old",
            "stale-credential".to_owned(),
        );

        assert_eq!(result.unwrap_err(), "Builderlab authentication canceled");
        assert!(session.0.lock().unwrap().is_none());
        assert_eq!(
            login
                .0
                .lock()
                .unwrap()
                .pending
                .as_ref()
                .map(|pending| pending.id.as_str()),
            Some("enterprise-login-new"),
        );
    }

    #[test]
    fn session_commit_holds_login_ownership_fence_through_write() {
        let login = BuilderlabLogin::default();
        let session = BuilderlabSession::default();
        let (cancel, _receiver) = oneshot::channel();
        login.0.lock().unwrap().pending = Some(PendingLogin {
            id: "enterprise-login-a".to_owned(),
            cancel,
        });

        commit_builderlab_login_session_with_hook(
            &login,
            &session,
            "enterprise-login-a",
            "fresh-credential".to_owned(),
            || {
                assert!(
                    login.0.try_lock().is_err(),
                    "login ownership must remain locked until after the session write",
                );
            },
        )
        .unwrap();

        assert!(login.0.lock().unwrap().pending.is_none());
        assert_eq!(
            session
                .0
                .lock()
                .unwrap()
                .as_ref()
                .map(|stored| stored.credential.as_str()),
            Some("fresh-credential"),
        );
    }

    #[test]
    fn cancel_without_attempt_id_preserves_existing_blind_cancel_behavior() {
        let login = BuilderlabLogin::default();
        let (cancel, mut receiver) = oneshot::channel();
        login.0.lock().unwrap().pending = Some(PendingLogin {
            id: "legacy-flow".to_owned(),
            cancel,
        });

        assert!(cancel_builderlab_login_attempt(&login, None).unwrap());
        assert!(login.0.lock().unwrap().pending.is_none());
        assert!(receiver.try_recv().is_ok());
    }

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
    fn blank_identity_profile_fields_normalize_to_none() {
        assert_eq!(normalized_auth_field(Some("  ".to_owned())), None);
        assert_eq!(
            normalized_auth_field(Some(" seiler ".to_owned())),
            Some("seiler".to_owned())
        );
    }

    #[test]
    fn builderlab_auth_json_uses_username_and_name_fields() {
        let exchange: LoginExchangeResponse = serde_json::from_value(serde_json::json!({
            "session_credential": "credential",
            "expires_at": "2099-01-01T00:00:00Z",
            "username": "seiler",
            "name": "Brad Seiler",
        }))
        .unwrap();
        assert_eq!(exchange.username.as_deref(), Some("seiler"));
        assert_eq!(exchange.name.as_deref(), Some("Brad Seiler"));

        let me: AuthMeResponse = serde_json::from_value(serde_json::json!({
            "email": "brad@example.com",
            "expires_at": "2099-01-01T00:00:00Z",
            "username": "seiler",
            "name": "Brad Seiler",
        }))
        .unwrap();
        assert_eq!(me.username.as_deref(), Some("seiler"));
        assert_eq!(me.name.as_deref(), Some("Brad Seiler"));

        let info = BuilderlabAuthInfo {
            expires_at: "2099-01-01T00:00:00Z".to_owned(),
            email: Some("brad@example.com".to_owned()),
            username: Some("seiler".to_owned()),
            name: Some("Brad Seiler".to_owned()),
        };
        assert_eq!(
            serde_json::to_value(info).unwrap(),
            serde_json::json!({
                "email": "brad@example.com",
                "expiresAt": "2099-01-01T00:00:00Z",
                "username": "seiler",
                "name": "Brad Seiler",
            })
        );
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
}
