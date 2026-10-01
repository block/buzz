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
const AUTH_COMPLETE_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Orbit authentication complete</title>
  <style>
    :root {
      color-scheme: light;
      font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      color: #231e1e;
      background: #99b1ef;
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
      background-color: #99b1ef;
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
      background: #99b1ef;
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
    <svg class="bee" viewBox="0 0 306 300" fill="none" role="img" aria-label="Orbit" xmlns="http://www.w3.org/2000/svg">
      <g filter="url(#filter0_n_2469_12)">
        <path d="M218.969 140.798C226.756 162.191 227.01 185.601 219.688 207.158C212.367 228.715 197.91 247.129 178.705 259.355C159.501 271.582 136.699 276.889 114.069 274.401C91.4394 271.912 70.3361 261.776 54.2475 245.669C38.1589 229.561 28.0482 208.446 25.5865 185.813C23.1248 163.18 28.4595 140.385 40.7089 121.195C52.9583 102.005 71.389 87.5692 92.9546 80.2737C114.52 72.9781 137.93 73.2595 159.314 81.0714" stroke="url(#paint0_linear_2469_12)" stroke-width="50"/>
      </g>
      <g filter="url(#filter1_n_2469_12)">
        <path d="M222 114.5C212.5 117.5 165.931 140.588 118 184C162.763 134.13 182.672 109.441 188.5 80C192.844 52.1103 192.943 33.4859 188.5 0C205.886 46.4008 212.5 54.5 225 52.5C235 49 250.545 45.778 305.5 0C257.259 53.7062 248.5 72.5 249.5 77.5C250.5 82.5 259.315 100.442 302 114.5C269.479 107.603 231.5 111.5 222 114.5Z" fill="url(#paint1_linear_2469_12)"/>
      </g>
      <defs>
        <filter id="filter0_n_2469_12" x="0" y="50" width="250" height="250" filterUnits="userSpaceOnUse" color-interpolation-filters="sRGB">
          <feFlood flood-opacity="0" result="BackgroundImageFix"/>
          <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
          <feTurbulence type="fractalNoise" baseFrequency="2 2" stitchTiles="stitch" numOctaves="3" result="noise" seed="8811" />
          <feColorMatrix in="noise" type="luminanceToAlpha" result="alphaNoise" />
          <feComponentTransfer in="alphaNoise" result="coloredNoise1">
            <feFuncA type="discrete" tableValues="1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 "/>
          </feComponentTransfer>
          <feComposite operator="in" in2="shape" in="coloredNoise1" result="noise1Clipped" />
          <feFlood flood-color="rgba(0, 0, 0, 0.25)" result="color1Flood" />
          <feComposite operator="in" in2="noise1Clipped" in="color1Flood" result="color1" />
          <feMerge result="effect1_noise_2469_12">
            <feMergeNode in="shape" />
            <feMergeNode in="color1" />
          </feMerge>
        </filter>
        <filter id="filter1_n_2469_12" x="118" y="0" width="187.5" height="184" filterUnits="userSpaceOnUse" color-interpolation-filters="sRGB">
          <feFlood flood-opacity="0" result="BackgroundImageFix"/>
          <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
          <feTurbulence type="fractalNoise" baseFrequency="2 2" stitchTiles="stitch" numOctaves="3" result="noise" seed="9203" />
          <feColorMatrix in="noise" type="luminanceToAlpha" result="alphaNoise" />
          <feComponentTransfer in="alphaNoise" result="coloredNoise1">
            <feFuncA type="discrete" tableValues="1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 "/>
          </feComponentTransfer>
          <feComposite operator="in" in2="shape" in="coloredNoise1" result="noise1Clipped" />
          <feFlood flood-color="rgba(0, 0, 0, 0.25)" result="color1Flood" />
          <feComposite operator="in" in2="noise1Clipped" in="color1Flood" result="color1" />
          <feMerge result="effect1_noise_2469_12">
            <feMergeNode in="shape" />
            <feMergeNode in="color1" />
          </feMerge>
        </filter>
        <linearGradient id="paint0_linear_2469_12" x1="125" y1="50" x2="-22.4998" y2="222" gradientUnits="userSpaceOnUse">
          <stop offset="0.0432692" stop-color="white"/>
          <stop offset="0.807692" stop-color="#B9B9BE"/>
        </linearGradient>
        <linearGradient id="paint1_linear_2469_12" x1="211.75" y1="0" x2="211.75" y2="184" gradientUnits="userSpaceOnUse">
          <stop stop-color="#F109ED"/>
          <stop offset="0.831731" stop-color="#3D42D9"/>
        </linearGradient>
      </defs>
    </svg>
    <div class="eyebrow">Authentication complete</div>
    <h1>You&rsquo;re signed in.</h1>
    <p>You can close this window and return to Orbit.</p>
  </main>
</body>
</html>"##;

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
    fn auth_complete_page_uses_orbit_brand() {
        for expected in [
            "<title>Orbit authentication complete</title>",
            "#99b1ef",
            "#231e1e",
            "#d7e7f6",
            "aria-label=\"Orbit\"",
            "return to Orbit",
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
