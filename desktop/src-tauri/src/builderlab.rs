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

#[derive(Default)]
pub(crate) struct BuilderlabSession(Mutex<Option<StoredSession>>);

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

    Html(
        "<!doctype html><meta charset=utf-8><title>Buzz authentication complete</title>\
         <h1>Authentication complete</h1><p>You can close this window and return to Buzz.</p>",
    )
    .into_response()
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

    let me = authenticated_user(&app_state.http_client, &exchanged.session_credential).await?;
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
    *session.0.lock().map_err(|error| error.to_string())? = Some(StoredSession {
        credential: exchanged.session_credential,
    });
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
            name: me.name,
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
