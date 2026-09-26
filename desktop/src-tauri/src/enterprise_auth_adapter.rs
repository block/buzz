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
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tauri_plugin_opener::OpenerExt;
use tokio::{net::TcpListener, sync::oneshot};
use url::Url;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const ENTERPRISE_AUTH_ATTEMPT_PREFIX: &str = "enterprise-auth-";
const CANCELED_LOGIN_ATTEMPT_TOMBSTONE_LIMIT: usize = 64;
const AUTHORIZATION_HEADER: &str = "Authorization";

const AUTH_COMPLETE_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Buzz authentication complete</title>
</head>
<body>
  <main>
    <h1>You&rsquo;re signed in.</h1>
    <p>You can close this window and return to Buzz.</p>
  </main>
</body>
</html>"#;

#[derive(Default)]
pub(crate) struct EnterpriseAuthSession(Mutex<Option<StoredSession>>);

#[derive(Default)]
pub(crate) struct EnterpriseAuthLogin(Mutex<EnterpriseAuthLoginState>);

#[derive(Default)]
struct EnterpriseAuthLoginState {
    pending: Option<PendingLogin>,
    canceled_attempts: VecDeque<String>,
}

struct PendingLogin {
    id: String,
    cancel: oneshot::Sender<()>,
}

struct StoredSession {
    token: String,
}

#[derive(Debug, Deserialize)]
struct EnterpriseExchangeResponse {
    session_token: String,
    expires_at: String,
    email: Option<String>,
    profile_projection: Option<EnterpriseProfileProjectionResponse>,
}

#[derive(Debug, Deserialize)]
struct EnterpriseSessionResponse {
    expires_at: String,
    email: Option<String>,
    profile_projection: Option<EnterpriseProfileProjectionResponse>,
}

#[derive(Debug, Deserialize)]
struct EnterpriseProfileProjectionResponse {
    username: Option<String>,
    display_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseAuthInfo {
    expires_at: String,
    email: Option<String>,
    profile_projection: Option<EnterpriseProfileProjection>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnterpriseProfileProjection {
    username: String,
    display_name: String,
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

fn enterprise_adapter_base_url() -> Result<&'static str, String> {
    option_env!("BUZZ_DESKTOP_BUILD_ENTERPRISE_AUTH_ADAPTER_BASE_URL").ok_or_else(|| {
        "This Buzz build does not include an enterprise authentication adapter URL".to_owned()
    })
}

fn enterprise_profile_projection_enabled() -> bool {
    option_env!("BUZZ_DESKTOP_BUILD_ENTERPRISE_PROFILE_PROJECTION") == Some("1")
}

fn enterprise_api_url_from_base(base_url: &str, path: &str) -> Result<Url, String> {
    Url::parse(&format!("{}{}", base_url, path))
        .map_err(|error| format!("invalid enterprise authentication adapter URL: {error}"))
}

fn enterprise_api_url(path: &str) -> Result<Url, String> {
    enterprise_api_url_from_base(enterprise_adapter_base_url()?, path)
}

fn enterprise_login_url_from_base(
    base_url: &str,
    return_to: &str,
    handoff_challenge: &str,
) -> Result<Url, String> {
    let mut login_url = enterprise_api_url_from_base(base_url, "/v1/login/start")?;
    login_url
        .query_pairs_mut()
        .append_pair("return_to", return_to)
        .append_pair("handoff_challenge", handoff_challenge)
        .append_pair("handoff_challenge_method", "S256");
    Ok(login_url)
}

fn enterprise_login_url(return_to: &str, handoff_challenge: &str) -> Result<Url, String> {
    enterprise_login_url_from_base(enterprise_adapter_base_url()?, return_to, handoff_challenge)
}

fn new_handoff_secret() -> Result<String, String> {
    let mut secret = [0_u8; 32];
    getrandom::getrandom(&mut secret).map_err(|error| format!("entropy source: {error}"))?;
    Ok(URL_SAFE_NO_PAD.encode(secret))
}

fn handoff_challenge(secret: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(secret.as_bytes()))
}

fn enterprise_login_exchange_body(code: &str, handoff_secret: &str) -> serde_json::Value {
    serde_json::json!({
        "code": code,
        "handoff_secret": handoff_secret,
    })
}

fn bearer_value(token: &str) -> String {
    format!("Bearer {token}")
}

fn clear_enterprise_session_if_token_matches(
    session: &EnterpriseAuthSession,
    token: &str,
) -> Result<(), String> {
    let mut stored = session.0.lock().map_err(|error| error.to_string())?;
    if stored.as_ref().is_some_and(|stored| stored.token == token) {
        *stored = None;
    }
    Ok(())
}

fn remember_canceled_attempt(state: &mut EnterpriseAuthLoginState, attempt_id: String) {
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

fn is_canceled_attempt(state: &EnterpriseAuthLoginState, attempt_id: &str) -> bool {
    state
        .canceled_attempts
        .iter()
        .any(|canceled| canceled == attempt_id)
}

fn clear_matching_pending_login(login: &EnterpriseAuthLogin, login_id: &str) -> Result<(), String> {
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

fn register_pending_enterprise_login(
    login: &EnterpriseAuthLogin,
    login_id: &str,
    cancel: oneshot::Sender<()>,
) -> Result<bool, String> {
    let mut state = login.0.lock().map_err(|error| error.to_string())?;
    if is_canceled_attempt(&state, login_id) {
        return Ok(false);
    }
    if state.pending.is_some() {
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

fn commit_enterprise_login_session(
    login: &EnterpriseAuthLogin,
    session: &EnterpriseAuthSession,
    login_id: &str,
    token: String,
) -> Result<(), String> {
    let mut login_state = login.0.lock().map_err(|error| error.to_string())?;
    if login_state
        .pending
        .as_ref()
        .is_none_or(|pending| pending.id != login_id)
    {
        return Err("Enterprise authentication canceled".to_owned());
    }
    *session.0.lock().map_err(|error| error.to_string())? = Some(StoredSession { token });
    login_state.pending = None;
    Ok(())
}

async fn abort_enterprise_login(
    login: &EnterpriseAuthLogin,
    server: tokio::task::JoinHandle<()>,
    login_id: &str,
    error: String,
) -> Result<EnterpriseAuthInfo, String> {
    server.abort();
    clear_matching_pending_login(login, login_id)?;
    Err(error)
}

fn normalized_auth_field(value: Option<String>) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn profile_projection(
    value: Option<EnterpriseProfileProjectionResponse>,
) -> Option<EnterpriseProfileProjection> {
    if !enterprise_profile_projection_enabled() {
        return None;
    }
    let value = value?;
    let username = normalized_auth_field(value.username)?;
    let display_name = normalized_auth_field(value.display_name)?;
    Some(EnterpriseProfileProjection {
        username,
        display_name,
    })
}

fn auth_info_from_session(value: EnterpriseSessionResponse) -> EnterpriseAuthInfo {
    EnterpriseAuthInfo {
        expires_at: value.expires_at,
        email: normalized_auth_field(value.email),
        profile_projection: profile_projection(value.profile_projection),
    }
}

fn enterprise_session_request_builder(
    client: &reqwest::Client,
    session_url: Url,
    token: &str,
) -> reqwest::RequestBuilder {
    client
        .get(session_url)
        .header(AUTHORIZATION_HEADER, bearer_value(token))
        .timeout(Duration::from_secs(30))
}

async fn authenticated_enterprise_user(
    client: &reqwest::Client,
    token: &str,
) -> Result<EnterpriseSessionResponse, String> {
    let response =
        enterprise_session_request_builder(client, enterprise_api_url("/v1/session")?, token)
            .send()
            .await
            .map_err(|error| format!("Enterprise authentication session check failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Enterprise authentication session check failed with HTTP {}",
            response.status()
        ));
    }
    response
        .json()
        .await
        .map_err(|error| format!("invalid enterprise authentication session response: {error}"))
}

#[tauri::command]
pub(crate) async fn start_enterprise_auth_login(
    app: tauri::AppHandle,
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, EnterpriseAuthSession>,
    login: tauri::State<'_, EnterpriseAuthLogin>,
    attempt_id: Option<String>,
) -> Result<EnterpriseAuthInfo, String> {
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
        .unwrap_or_else(|| format!("{ENTERPRISE_AUTH_ATTEMPT_PREFIX}{}", uuid::Uuid::new_v4()));
    let (cancel_sender, mut cancel_receiver) = oneshot::channel();
    match register_pending_enterprise_login(&login, &login_id, cancel_sender) {
        Ok(true) => {}
        Ok(false) => {
            server.abort();
            return Err("Enterprise authentication canceled".to_owned());
        }
        Err(error) => {
            server.abort();
            return Err(error);
        }
    }

    let handoff_secret = match new_handoff_secret() {
        Ok(secret) => secret,
        Err(error) => return abort_enterprise_login(&login, server, &login_id, error).await,
    };
    let login_url = match enterprise_login_url(&return_to, &handoff_challenge(&handoff_secret)) {
        Ok(login_url) => login_url,
        Err(error) => return abort_enterprise_login(&login, server, &login_id, error).await,
    };
    if let Err(error) = app.opener().open_url(login_url.as_str(), None::<&str>) {
        return abort_enterprise_login(
            &login,
            server,
            &login_id,
            format!("could not open enterprise authentication: {error}"),
        )
        .await;
    }

    let exchange_code = tokio::select! {
        result = tokio::time::timeout(LOGIN_TIMEOUT, receiver) => match result {
            Ok(Ok(Ok(code))) => code,
            Ok(Ok(Err(error))) => {
                return abort_enterprise_login(&login, server, &login_id, error).await;
            }
            Ok(Err(_)) => {
                return abort_enterprise_login(
                    &login,
                    server,
                    &login_id,
                    "local authentication callback stopped unexpectedly".to_owned(),
                )
                .await;
            }
            Err(_) => {
                return abort_enterprise_login(
                    &login,
                    server,
                    &login_id,
                    "Enterprise authentication timed out".to_owned(),
                )
                .await;
            }
        },
        _ = &mut cancel_receiver => {
            return abort_enterprise_login(
                &login,
                server,
                &login_id,
                "Enterprise authentication canceled".to_owned(),
            )
            .await;
        }
    };
    server.abort();

    let exchange_url = match enterprise_api_url("/v1/login/exchange") {
        Ok(exchange_url) => exchange_url,
        Err(error) => {
            clear_matching_pending_login(&login, &login_id)?;
            return Err(error);
        }
    };
    let response = match app_state
        .http_client
        .post(exchange_url)
        .json(&enterprise_login_exchange_body(
            &exchange_code,
            &handoff_secret,
        ))
        .timeout(Duration::from_secs(30))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            clear_matching_pending_login(&login, &login_id)?;
            return Err(format!(
                "Enterprise authentication code exchange failed: {error}"
            ));
        }
    };
    if !response.status().is_success() {
        let status = response.status();
        clear_matching_pending_login(&login, &login_id)?;
        return Err(format!(
            "Enterprise authentication code exchange failed with HTTP {status}"
        ));
    }
    let exchanged: EnterpriseExchangeResponse = match response.json().await {
        Ok(exchanged) => exchanged,
        Err(error) => {
            clear_matching_pending_login(&login, &login_id)?;
            return Err(format!(
                "invalid enterprise authentication code exchange response: {error}"
            ));
        }
    };
    if exchanged.session_token.is_empty() {
        clear_matching_pending_login(&login, &login_id)?;
        return Err("Enterprise authentication code exchange returned an empty token".to_owned());
    }

    let session_response =
        match authenticated_enterprise_user(&app_state.http_client, &exchanged.session_token).await
        {
            Ok(session_response) => session_response,
            Err(error) => {
                clear_matching_pending_login(&login, &login_id)?;
                return Err(error);
            }
        };
    if exchanged.expires_at != session_response.expires_at {
        clear_matching_pending_login(&login, &login_id)?;
        return Err(
            "Enterprise authentication session expiry did not match code exchange".to_owned(),
        );
    }
    let info = EnterpriseAuthInfo {
        expires_at: session_response.expires_at.clone(),
        email: normalized_auth_field(session_response.email)
            .or_else(|| normalized_auth_field(exchanged.email)),
        profile_projection: profile_projection(session_response.profile_projection)
            .or_else(|| profile_projection(exchanged.profile_projection)),
    };
    commit_enterprise_login_session(&login, &session, &login_id, exchanged.session_token)?;
    Ok(info)
}

#[tauri::command]
pub(crate) async fn get_enterprise_auth(
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, EnterpriseAuthSession>,
) -> Result<Option<EnterpriseAuthInfo>, String> {
    let stored = session
        .0
        .lock()
        .map_err(|error| error.to_string())?
        .as_ref()
        .map(|stored| stored.token.clone());
    let Some(token) = stored else {
        return Ok(None);
    };
    match authenticated_enterprise_user(&app_state.http_client, &token).await {
        Ok(session_response) => Ok(Some(auth_info_from_session(session_response))),
        Err(error) => {
            clear_enterprise_session_if_token_matches(&session, &token)?;
            Err(error)
        }
    }
}

fn cancel_enterprise_auth_login_attempt(
    login: &EnterpriseAuthLogin,
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
pub(crate) fn cancel_enterprise_auth_login(
    login: tauri::State<'_, EnterpriseAuthLogin>,
    attempt_id: Option<String>,
) -> Result<(), String> {
    cancel_enterprise_auth_login_attempt(&login, attempt_id.as_deref()).map(|_| ())
}

#[tauri::command]
pub(crate) fn clear_enterprise_auth(
    session: tauri::State<'_, EnterpriseAuthSession>,
) -> Result<(), String> {
    *session.0.lock().map_err(|error| error.to_string())? = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enterprise_login_url_uses_provider_neutral_contract_and_handoff_challenge() {
        let login = enterprise_login_url_from_base(
            "https://identity.example",
            "http://127.0.0.1:1234/callback/nonce",
            "challenge",
        )
        .unwrap();
        let query: HashMap<_, _> = login.query_pairs().into_owned().collect();

        assert_eq!(login.path(), "/v1/login/start");
        assert_eq!(
            query.get("return_to").map(String::as_str),
            Some("http://127.0.0.1:1234/callback/nonce")
        );
        assert_eq!(
            query.get("handoff_challenge").map(String::as_str),
            Some("challenge")
        );
        assert_eq!(
            query.get("handoff_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert!(!query.contains_key("type"));
        assert!(!query.contains_key("product"));
    }

    #[test]
    fn handoff_challenge_is_sha256_bound_to_secret() {
        assert_eq!(
            handoff_challenge("secret"),
            URL_SAFE_NO_PAD.encode(Sha256::digest(b"secret")),
        );
    }

    #[test]
    fn enterprise_login_exchange_requires_handoff_secret() {
        assert_eq!(
            enterprise_login_exchange_body("callback-code", "handoff-secret"),
            serde_json::json!({
                "code": "callback-code",
                "handoff_secret": "handoff-secret",
            })
        );
    }

    #[test]
    fn bearer_authorization_header_is_standard() {
        assert_eq!(AUTHORIZATION_HEADER, "Authorization");
        assert_eq!(bearer_value("token"), "Bearer token");
    }

    #[test]
    fn enterprise_adapter_does_not_use_builderlab_or_bbidentity_authorization_schemes() {
        assert_ne!(AUTHORIZATION_HEADER, "X-BB-Session-Credential");
        assert_ne!(bearer_value("token"), "BBIdentity token");
    }

    #[test]
    fn enterprise_session_request_uses_only_bearer_authorization() {
        let client = reqwest::Client::new();
        let request = enterprise_session_request_builder(
            &client,
            Url::parse("https://identity.example/v1/session").unwrap(),
            "session-token",
        )
        .build()
        .unwrap();

        assert_eq!(
            request
                .headers()
                .get(reqwest::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer session-token")
        );
        assert!(request.headers().get("X-BB-Session-Credential").is_none());
        assert!(request.headers().get("Nostr-Federated-Identity").is_none());
    }

    #[test]
    fn stale_session_check_failure_only_clears_the_token_it_checked() {
        let session = EnterpriseAuthSession::default();
        *session.0.lock().unwrap() = Some(StoredSession {
            token: "old-token".to_owned(),
        });

        clear_enterprise_session_if_token_matches(&session, "old-token").unwrap();
        assert!(session.0.lock().unwrap().is_none());

        *session.0.lock().unwrap() = Some(StoredSession {
            token: "fresh-token".to_owned(),
        });
        clear_enterprise_session_if_token_matches(&session, "old-token").unwrap();
        assert_eq!(
            session
                .0
                .lock()
                .unwrap()
                .as_ref()
                .map(|stored| stored.token.as_str()),
            Some("fresh-token"),
        );
    }

    #[test]
    fn cancel_before_registration_tombstones_enterprise_auth_attempt() {
        let login = EnterpriseAuthLogin::default();
        assert!(!cancel_enterprise_auth_login_attempt(&login, Some("enterprise-auth-a")).unwrap());

        let (cancel, _receiver) = oneshot::channel();
        assert!(!register_pending_enterprise_login(&login, "enterprise-auth-a", cancel).unwrap());
        assert!(login.0.lock().unwrap().pending.is_none());
    }

    #[test]
    fn stale_attempt_cannot_commit_enterprise_session_after_replacement() {
        let login = EnterpriseAuthLogin::default();
        let session = EnterpriseAuthSession::default();
        let (cancel, _receiver) = oneshot::channel();
        login.0.lock().unwrap().pending = Some(PendingLogin {
            id: "enterprise-auth-new".to_owned(),
            cancel,
        });

        let result = commit_enterprise_login_session(
            &login,
            &session,
            "enterprise-auth-old",
            "stale-token".to_owned(),
        );

        assert_eq!(result.unwrap_err(), "Enterprise authentication canceled");
        assert!(session.0.lock().unwrap().is_none());
    }

    #[test]
    fn profile_projection_defaults_off_even_when_adapter_returns_fields() {
        let value = EnterpriseProfileProjectionResponse {
            username: Some("alice".to_owned()),
            display_name: Some("Alice Example".to_owned()),
        };
        assert!(profile_projection(Some(value)).is_none());
    }
}
