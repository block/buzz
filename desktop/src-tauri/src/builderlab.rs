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
fn session_header(credential: &str) -> Result<reqwest::header::HeaderValue, String> {
    let mut value = reqwest::header::HeaderValue::from_str(credential)
        .map_err(|_| "Invalid identity session")?;
    value.set_sensitive(true);
    Ok(value)
}

/// A committed key/community change must close sockets using the previous employee session.
pub(crate) fn invalidate_changed_employee_context(state: &crate::app_state::AppState) {
    use tauri::Manager;
    let handle = state
        .app_handle
        .lock()
        .ok()
        .and_then(|handle| handle.clone());
    if let Some(handle) = handle {
        if let Some(session) = handle.try_state::<BuilderlabSession>() {
            let _ = session.request_context(state);
            notify_session_changed(state);
        }
    }
}
pub(crate) fn notify_session_changed(state: &crate::app_state::AppState) {
    use tauri::Emitter;
    if let Some(handle) = state.app_handle.lock().ok().and_then(|value| value.clone()) {
        let _ = handle.emit("builderlab-session-changed", ());
    }
}

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
pub(crate) struct BuilderlabSession(Mutex<Option<StoredSession>>, tokio::sync::Mutex<()>);

#[derive(Default)]
pub(crate) struct BuilderlabLogin(Mutex<Option<PendingLogin>>);

struct PendingLogin {
    id: uuid::Uuid,
    cancel: oneshot::Sender<()>,
}

struct StoredSession {
    credential: String,
    api_base: &'static str,
    employee: Option<EmployeeBinding>,
    assertion: Option<CachedAssertion>,
    refusal: Option<String>,
    cancel: tokio_util::sync::CancellationToken,
}

pub(crate) struct EmployeeBinding {
    pub(crate) pubkey: String,
    pub(crate) expires_at: i64,
    pub(crate) relay: String,
}
struct CachedAssertion {
    value: String,
    expires_at: i64,
}

mod employee_relay;
mod employee_response;
pub(crate) use employee_relay::{
    employee_evidence, requires_employee, send_employee_media_request, send_employee_request,
    EmployeeEvidence,
};
pub(crate) use employee_response::{
    employee_response_end, guard_employee_response, ProtectedAssertion,
};

impl BuilderlabSession {
    pub(crate) fn employee_expiry(&self, state: &crate::app_state::AppState) -> Option<i64> {
        self.request_context(state).ok().flatten()?;
        let stored = self.0.lock().ok()?;
        let stored = stored.as_ref()?;
        // A received CLI credential alone is not a connected Buzz identity.
        stored.assertion.as_ref()?;
        stored.employee.as_ref().map(|employee| employee.expires_at)
    }
    pub(crate) fn invalidate_employee_assertion(&self) -> Result<(), String> {
        if let Some(stored) = self
            .0
            .lock()
            .map_err(|_| "Square session unavailable")?
            .as_mut()
        {
            stored.assertion = None;
        }
        Ok(())
    }
    pub(crate) fn employee_can_retry(&self, state: &crate::app_state::AppState) -> bool {
        if self.request_context(state).ok().flatten().is_none() {
            return false;
        }
        self.0
            .lock()
            .ok()
            .and_then(|stored| {
                stored.as_ref().map(|stored| {
                    stored.employee.is_some()
                        && (stored.refusal.is_some() || stored.assertion.is_none())
                })
            })
            .unwrap_or(false)
    }
    fn refuse_employee_access(&self, credential: &str, reason: String) -> Result<(), String> {
        let mut stored = self.0.lock().map_err(|_| "Square session unavailable")?;
        if let Some(stored) = stored
            .as_mut()
            .filter(|stored| stored.credential == credential)
        {
            stored.cancel.cancel();
            stored.assertion = None;
            stored.refusal = Some(reason);
        }
        Ok(())
    }
    pub(crate) fn clear_failed_employee_login(
        &self,
        credential: &str,
        canceled: bool,
    ) -> Result<(), String> {
        let mut stored = self.0.lock().map_err(|_| "Square session unavailable")?;
        if stored.as_ref().is_some_and(|value| {
            value.credential == credential && (canceled || value.refusal.is_none())
        }) {
            if let Some(previous) = stored.take() {
                previous.cancel.cancel();
            }
        }
        Ok(())
    }
    fn replace(&self, next: StoredSession) -> Result<(), String> {
        let mut current = self.0.lock().map_err(|error| error.to_string())?;
        if let Some(previous) = current.replace(next) {
            previous.cancel.cancel();
        }
        Ok(())
    }
    fn clear(&self) -> Result<(), String> {
        if let Some(previous) = self.0.lock().map_err(|error| error.to_string())?.take() {
            previous.cancel.cancel();
        }
        Ok(())
    }
    pub(crate) fn clear_if_credential(&self, credential: &str) -> Result<(), String> {
        let mut stored = self.0.lock().map_err(|_| "Square session unavailable")?;
        if stored
            .as_ref()
            .is_some_and(|value| value.credential == credential)
        {
            if let Some(previous) = stored.take() {
                previous.cancel.cancel();
            }
        }
        Ok(())
    }

    fn request_context(
        &self,
        app_state: &crate::app_state::AppState,
    ) -> Result<Option<(String, &'static str)>, String> {
        let mut stored = self.0.lock().map_err(|error| error.to_string())?;
        if let Some(session) = stored.as_ref() {
            if let Some(expected) = &session.employee {
                if app_state.signing_keys()?.public_key().to_hex() != expected.pubkey
                    || chrono::Utc::now().timestamp() >= expected.expires_at
                    || crate::relay::relay_ws_url_with_override(app_state) != expected.relay
                {
                    session.cancel.cancel();
                    *stored = None;
                    return Err("Buzz identity changed. Sign in with Square again.".to_owned());
                }
            }
        }
        Ok(stored
            .as_ref()
            .map(|session| (session.credential.clone(), session.api_base)))
    }

    fn api_origin(&self, app_state: &crate::app_state::AppState) -> Result<String, String> {
        let (_, base) = self.request_context(app_state)?.ok_or("Sign in first")?;
        Url::parse(base)
            .map(|url| url.origin().ascii_serialization())
            .map_err(|_| "Invalid identity origin".to_owned())
    }

    pub(crate) fn finish_employee_login(
        &self,
        login: &BuilderlabLogin,
        id: uuid::Uuid,
        credential: String,
        api_base: &'static str,
        employee: EmployeeBinding,
    ) -> Result<(), String> {
        let pending = login.0.lock().map_err(|error| error.to_string())?;
        if pending.as_ref().is_none_or(|pending| pending.id != id) {
            return Err("Square sign-in canceled".to_owned());
        }
        self.replace(StoredSession {
            credential,
            api_base,
            employee: Some(employee),
            assertion: None,
            refusal: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        })?;
        // The login remains cancellable through first assertion/key enrollment.
        Ok(())
    }
}

impl BuilderlabLogin {
    pub(crate) fn complete_employee_login(&self, id: uuid::Uuid) -> Result<(), String> {
        let mut pending = self.0.lock().map_err(|_| "Square sign-in unavailable")?;
        if pending.as_ref().is_none_or(|value| value.id != id) {
            return Err("Square sign-in canceled".to_owned());
        }
        *pending = None;
        Ok(())
    }

    pub(crate) fn begin_employee_login(
        &self,
    ) -> Result<(uuid::Uuid, oneshot::Receiver<()>), String> {
        let id = uuid::Uuid::new_v4();
        let (cancel, receiver) = oneshot::channel();
        let mut pending = self.0.lock().map_err(|error| error.to_string())?;
        if let Some(previous) = pending.take() {
            let _ = previous.cancel.send(());
        }
        *pending = Some(PendingLogin { id, cancel });
        Ok((id, receiver))
    }
}

#[derive(Debug, Deserialize)]
struct LoginExchangeResponse {
    session_credential: String,
    expires_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BuilderlabAuthInfo {
    pub(crate) expires_at: String,
    pub(crate) email: Option<String>,
    pub(crate) name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuthMeResponse {
    pub(crate) subject: Option<String>,
    email: Option<String>,
    name: Option<String>,
    pub(crate) expires_at: String,
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
    api_url_for_base(BUILDERLAB_API_BASE_URL, path)
}

pub(crate) fn api_url_for_base(base: &str, path: &str) -> Result<Url, String> {
    Url::parse(&format!("{base}{path}"))
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

enum SessionCheckError {
    Rejected(String),
    Unavailable(String),
}

impl From<String> for SessionCheckError {
    fn from(message: String) -> Self {
        Self::Unavailable(message)
    }
}

impl SessionCheckError {
    fn message(self) -> String {
        match self {
            Self::Rejected(message) | Self::Unavailable(message) => message,
        }
    }
}

pub(crate) async fn authenticated_user(
    client: &reqwest::Client,
    credential: &str,
    api_base: &str,
) -> Result<AuthMeResponse, String> {
    check_authenticated_user(client, credential, api_base)
        .await
        .map_err(SessionCheckError::message)
}

async fn check_authenticated_user(
    client: &reqwest::Client,
    credential: &str,
    api_base: &str,
) -> Result<AuthMeResponse, SessionCheckError> {
    let builder = client
        .get(api_url_for_base(api_base, "/v1/auth/me")?)
        .header(BB_SESSION_CREDENTIAL_HEADER, session_header(credential)?)
        .timeout(Duration::from_secs(30));
    let request = if api_base == crate::employee_login::API_BASE {
        crate::employee_api::request(builder)?
    } else {
        builder
            .build()
            .map_err(|_| "Could not prepare Builderlab session check".to_owned())?
    };
    let response = client
        .execute(request)
        .await
        .map_err(|error| format!("Builderlab session check failed: {error}"))?;
    if !response.status().is_success() {
        let message = format!(
            "Builderlab session check failed with HTTP {}",
            response.status()
        );
        return Err(match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                SessionCheckError::Rejected(message)
            }
            _ => SessionCheckError::Unavailable(message),
        });
    }
    response.json().await.map_err(|error| {
        SessionCheckError::Unavailable(format!("invalid Builderlab session response: {error}"))
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

    let me = authenticated_user(
        &app_state.media_fetch_client,
        &exchanged.session_credential,
        BUILDERLAB_API_BASE_URL,
    )
    .await?;
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
        session.replace(StoredSession {
            credential: exchanged.session_credential,
            api_base: BUILDERLAB_API_BASE_URL,
            employee: None,
            assertion: None,
            refusal: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        })?;
        *pending = None;
    }
    notify_session_changed(&app_state);
    Ok(info)
}

#[tauri::command]
pub(crate) async fn get_builderlab_auth(
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<Option<BuilderlabAuthInfo>, String> {
    check_stored_auth(&app_state, &session).await
}

async fn check_stored_auth(
    app_state: &crate::app_state::AppState,
    session: &BuilderlabSession,
) -> Result<Option<BuilderlabAuthInfo>, String> {
    let stored = session.request_context(app_state)?;
    let Some((credential, api_base)) = stored else {
        return Ok(None);
    };
    match check_authenticated_user(&app_state.media_fetch_client, &credential, api_base).await {
        Ok(me) => Ok(Some(BuilderlabAuthInfo {
            expires_at: me.expires_at,
            email: me.email,
            name: me.name,
        })),
        Err(error) => {
            // Losing reachability does not revoke a credential. Keep it for a later
            // retry; only an authoritative authentication refusal ends the session.
            if matches!(&error, SessionCheckError::Rejected(_)) {
                session.clear_if_credential(&credential)?;
                notify_session_changed(app_state);
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
    login: tauri::State<'_, BuilderlabLogin>,
    app_state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<(), String> {
    if let Some(pending) = login.0.lock().map_err(|error| error.to_string())?.take() {
        let _ = pending.cancel.send(());
    }
    session.clear()?;
    notify_session_changed(&app_state);
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
    app_state: &crate::app_state::AppState,
    session: &BuilderlabSession,
    method: reqwest::Method,
    path: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let (credential, api_base) = session
        .request_context(app_state)?
        .ok_or_else(|| "Sign in first".to_owned())?;
    let origin = Url::parse(api_base)
        .map_err(|_| "Invalid identity origin")?
        .origin()
        .ascii_serialization();
    let response = app_state
        .media_fetch_client
        .request(method, api_url_for_base(api_base, path)?)
        .header(BB_SESSION_CREDENTIAL_HEADER, session_header(&credential)?)
        .header(reqwest::header::ORIGIN, &origin)
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
        &app_state,
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
        &app_state,
        &session,
        reqwest::Method::POST,
        "/v1/buzz/nostr-identities/challenge",
        serde_json::json!({ "origin": session.api_origin(&app_state)? }),
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
        &app_state,
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
        &app_state,
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
        &app_state,
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
        &app_state,
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
        &app_state,
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
        &app_state,
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
        &app_state,
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
        &app_state,
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

    #[tokio::test]
    async fn stored_session_survives_transient_checks_but_not_authentication_refusal() {
        for status in [
            StatusCode::OK, // Malformed successful responses are not revocation evidence.
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base =
                Box::leak(format!("http://{}", listener.local_addr().unwrap()).into_boxed_str());
            let server = tokio::spawn(async move {
                axum::serve(
                    listener,
                    Router::new().route(
                        "/v1/auth/me",
                        get(move || async move { (status, "invalid JSON") }),
                    ),
                )
                .await
                .unwrap();
            });
            let state = crate::app_state::build_app_state();
            let session = BuilderlabSession::default();
            let cancel = tokio_util::sync::CancellationToken::new();
            session
                .replace(StoredSession {
                    credential: "session-fixture".to_owned(),
                    api_base: base,
                    employee: None,
                    assertion: None,
                    refusal: None,
                    cancel: cancel.clone(),
                })
                .unwrap();
            assert!(check_stored_auth(&state, &session).await.is_err());
            let rejected = matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN);
            assert_eq!(
                session.request_context(&state).unwrap().is_none(),
                rejected,
                "{status}"
            );
            assert_eq!(cancel.is_cancelled(), rejected, "{status}");
            server.abort();
        }
    }

    #[tokio::test]
    async fn unreachable_session_endpoint_preserves_the_stored_credential() {
        // Close a reserved port without starting an HTTP server, so this request
        // cannot reuse a live pooled connection from a preceding response.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Box::leak(format!("http://{}", listener.local_addr().unwrap()).into_boxed_str());
        drop(listener);
        let state = crate::app_state::build_app_state();
        let session = BuilderlabSession::default();
        let cancel = tokio_util::sync::CancellationToken::new();
        session
            .replace(StoredSession {
                credential: "session-fixture".to_owned(),
                api_base: base,
                employee: None,
                assertion: None,
                refusal: None,
                cancel: cancel.clone(),
            })
            .unwrap();
        assert!(check_stored_auth(&state, &session).await.is_err());
        assert_eq!(
            session.request_context(&state).unwrap().unwrap().0,
            "session-fixture"
        );
        assert!(!cancel.is_cancelled());
    }

    #[test]
    fn employee_session_retains_origin_and_rejects_canceled_or_replaced_identity() {
        let state = crate::app_state::build_app_state();
        let keys = state.signing_keys().unwrap();
        let session = BuilderlabSession::default();
        let login = BuilderlabLogin::default();
        let (old, mut canceled) = login.begin_employee_login().unwrap();
        let (current, _) = login.begin_employee_login().unwrap();
        assert!(canceled.try_recv().is_ok());
        assert!(session
            .finish_employee_login(
                &login,
                old,
                "old".to_owned(),
                "https://square.builderlabstaging.xyz/api/goose",
                EmployeeBinding {
                    pubkey: keys.public_key().to_hex(),
                    expires_at: chrono::Utc::now().timestamp() + 240,
                    relay: crate::relay::relay_ws_url_with_override(&state),
                },
            )
            .is_err());
        session
            .finish_employee_login(
                &login,
                current,
                "current".to_owned(),
                "https://square.builderlabstaging.xyz/api/goose",
                EmployeeBinding {
                    pubkey: keys.public_key().to_hex(),
                    expires_at: chrono::Utc::now().timestamp() + 240,
                    relay: crate::relay::relay_ws_url_with_override(&state),
                },
            )
            .unwrap();
        let (credential, base) = session.request_context(&state).unwrap().unwrap();
        assert_eq!(credential, "current");
        assert_eq!(base, "https://square.builderlabstaging.xyz/api/goose");
        *state.keys.lock().unwrap() = nostr::Keys::generate();
        assert!(session.request_context(&state).is_err());
        assert!(session.request_context(&state).unwrap().is_none());
    }
    #[test]
    fn employee_provisional_login_remains_cancelable_and_cannot_clear_a_replacement() {
        let state = crate::app_state::build_app_state();
        let session = BuilderlabSession::default();
        let login = BuilderlabLogin::default();
        let (id, mut cancellation) = login.begin_employee_login().unwrap();
        session
            .finish_employee_login(
                &login,
                id,
                "provisional".to_owned(),
                BUILDERLAB_API_BASE_URL,
                EmployeeBinding {
                    pubkey: state.signing_keys().unwrap().public_key().to_hex(),
                    expires_at: chrono::Utc::now().timestamp() + 240,
                    relay: crate::relay::relay_ws_url_with_override(&state),
                },
            )
            .unwrap();
        // A later login/cancel still owns and invalidates this in-flight admission.
        let (new_id, _) = login.begin_employee_login().unwrap();
        assert!(cancellation.try_recv().is_ok());
        assert!(login.complete_employee_login(id).is_err());
        session
            .replace(StoredSession {
                credential: "replacement".to_owned(),
                api_base: BUILDERLAB_API_BASE_URL,
                employee: None,
                assertion: None,
                refusal: None,
                cancel: tokio_util::sync::CancellationToken::new(),
            })
            .unwrap();
        session.clear_if_credential("provisional").unwrap();
        assert_eq!(
            session.request_context(&state).unwrap().unwrap().0,
            "replacement"
        );
        login.complete_employee_login(new_id).unwrap();
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
