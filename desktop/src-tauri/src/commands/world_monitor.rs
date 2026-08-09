use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{Query, State},
    response::Html,
    routing::get,
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use buzz_command_sources_pkg::{
    mcp_http::{McpHttpClient, McpHttpError},
    oauth::{WorldMonitorOAuthCredentials, WorldMonitorOAuthStore},
    usage::WorldMonitorUsageLedger,
    DEFAULT_WORLD_MONITOR_ENDPOINT, WORLD_MONITOR_OAUTH_FILENAME,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::{net::TcpListener, sync::oneshot};
use url::Url;

const DAILY_LIMIT: u8 = 25;
const AUTH_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const REGISTRATION_ENDPOINT: &str = "https://api.worldmonitor.app/oauth/register";
const AUTHORISATION_ENDPOINT: &str = "https://api.worldmonitor.app/oauth/authorize";
const TOKEN_ENDPOINT: &str = "https://api.worldmonitor.app/oauth/token";

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldMonitorConnectionStatus {
    NotConnected,
    Connected,
    Reauthorise,
    Unavailable,
    QuotaLimited,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldMonitorConnectionView {
    pub endpoint: String,
    pub status: WorldMonitorConnectionStatus,
    pub brief_used: u8,
    pub brief_limit: u8,
    pub direct_used: u8,
    pub direct_limit: u8,
}

#[derive(Deserialize)]
struct RegistrationResponse {
    client_id: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    token_type: String,
}

struct CallbackState {
    expected_state: String,
    sender: Mutex<Option<oneshot::Sender<Result<String, String>>>>,
}

fn oauth_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(WORLD_MONITOR_OAUTH_FILENAME))
        .map_err(|_| command_error())
}

fn usage_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join("world-monitor-usage.json"))
        .map_err(|_| command_error())
}

fn oauth_store(app: &AppHandle) -> Result<WorldMonitorOAuthStore, String> {
    WorldMonitorOAuthStore::new(oauth_path(app)?).map_err(|_| command_error())
}

fn command_error() -> String {
    "World Monitor MCP connection is unavailable.".to_string()
}

fn usage_counts(app: &AppHandle) -> (u8, u8) {
    usage_path(app)
        .ok()
        .and_then(|path| {
            WorldMonitorUsageLedger::new(path)
                .snapshot(chrono::Local::now())
                .ok()
        })
        .map_or((0, 0), |snapshot| {
            (snapshot.brief_used, snapshot.direct_used)
        })
}

fn view(app: &AppHandle, status: WorldMonitorConnectionStatus) -> WorldMonitorConnectionView {
    let (brief_used, direct_used) = usage_counts(app);
    WorldMonitorConnectionView {
        endpoint: DEFAULT_WORLD_MONITOR_ENDPOINT.to_string(),
        status,
        brief_used,
        brief_limit: DAILY_LIMIT,
        direct_used,
        direct_limit: DAILY_LIMIT,
    }
}

fn status_from_store(app: &AppHandle) -> WorldMonitorConnectionStatus {
    let Ok(store) = oauth_store(app) else {
        return WorldMonitorConnectionStatus::Unavailable;
    };
    match store.load() {
        Ok(Some(credentials)) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(i64::MAX);
            if credentials.refresh_expires_at() <= now {
                WorldMonitorConnectionStatus::Reauthorise
            } else {
                WorldMonitorConnectionStatus::Connected
            }
        }
        Ok(None) => WorldMonitorConnectionStatus::NotConnected,
        Err(_) => WorldMonitorConnectionStatus::Unavailable,
    }
}

#[tauri::command]
pub async fn get_world_monitor_connection(
    app: AppHandle,
) -> Result<WorldMonitorConnectionView, String> {
    Ok(view(&app, status_from_store(&app)))
}

#[tauri::command]
pub async fn connect_world_monitor_oauth(
    app: AppHandle,
) -> Result<WorldMonitorConnectionView, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| command_error())?;
    let port = listener.local_addr().map_err(|_| command_error())?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| command_error())?;
    let registration = client
        .post(REGISTRATION_ENDPOINT)
        .json(&serde_json::json!({
            "redirect_uris": [redirect_uri],
            "client_name": "Command Adviser",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "scope": "mcp"
        }))
        .send()
        .await
        .map_err(|_| command_error())?;
    if !registration.status().is_success() {
        return Err(command_error());
    }
    let registration: RegistrationResponse =
        registration.json().await.map_err(|_| command_error())?;
    validate_oauth_value(&registration.client_id, 512)?;

    let verifier = random_urlsafe(48);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state_value = random_urlsafe(32);
    let (sender, receiver) = oneshot::channel();
    let callback_state = Arc::new(CallbackState {
        expected_state: state_value.clone(),
        sender: Mutex::new(Some(sender)),
    });
    let router = Router::new()
        .route("/callback", get(oauth_callback))
        .with_state(callback_state);
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let mut authorisation = Url::parse(AUTHORISATION_ENDPOINT).map_err(|_| command_error())?;
    authorisation
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &registration.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state_value)
        .append_pair("scope", "mcp")
        .append_pair("resource", DEFAULT_WORLD_MONITOR_ENDPOINT);
    if app
        .opener()
        .open_url(authorisation.as_str(), None::<&str>)
        .is_err()
    {
        server.abort();
        return Err(command_error());
    }

    let code = match tokio::time::timeout(AUTH_TIMEOUT, receiver).await {
        Ok(Ok(Ok(code))) => code,
        Ok(Ok(Err(error))) => {
            server.abort();
            return Err(error);
        }
        _ => {
            server.abort();
            return Err("World Monitor sign-in timed out.".to_string());
        }
    };
    server.abort();
    validate_oauth_value(&code, 2048)?;

    let exchange = client
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("client_id", registration.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
        ])
        .send()
        .await
        .map_err(|_| command_error())?;
    if !exchange.status().is_success() {
        return Err("World Monitor sign-in was not completed.".to_string());
    }
    let token: TokenResponse = exchange.json().await.map_err(|_| command_error())?;
    if !token.token_type.eq_ignore_ascii_case("bearer") {
        return Err(command_error());
    }
    let credentials = WorldMonitorOAuthCredentials::from_exchange(
        registration.client_id,
        token.access_token,
        token.refresh_token,
        token.expires_in,
    )
    .map_err(|_| command_error())?;
    oauth_store(&app)?
        .save(&credentials)
        .map_err(|_| command_error())?;
    Ok(view(&app, WorldMonitorConnectionStatus::Connected))
}

#[tauri::command]
pub async fn disconnect_world_monitor(
    app: AppHandle,
) -> Result<WorldMonitorConnectionView, String> {
    oauth_store(&app)?.clear().map_err(|_| command_error())?;
    Ok(view(&app, WorldMonitorConnectionStatus::NotConnected))
}

#[tauri::command]
pub async fn test_world_monitor_connection(
    app: AppHandle,
) -> Result<WorldMonitorConnectionView, String> {
    let store = oauth_store(&app)?;
    if store.load().map_err(|_| command_error())?.is_none() {
        return Ok(view(&app, WorldMonitorConnectionStatus::NotConnected));
    }
    let client = McpHttpClient::world_monitor(DEFAULT_WORLD_MONITOR_ENDPOINT, store)
        .map_err(|_| command_error())?;
    let status = match client.list_tools().await {
        Ok(result)
            if result
                .get("tools")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|tools| {
                    tools.iter().any(|tool| {
                        tool.get("name").and_then(serde_json::Value::as_str)
                            == Some("get_country_risk")
                    })
                }) =>
        {
            WorldMonitorConnectionStatus::Connected
        }
        Ok(_) => WorldMonitorConnectionStatus::Unavailable,
        Err(McpHttpError::Unauthorized) => WorldMonitorConnectionStatus::Reauthorise,
        Err(McpHttpError::RateLimited) => WorldMonitorConnectionStatus::QuotaLimited,
        Err(_) => WorldMonitorConnectionStatus::Unavailable,
    };
    Ok(view(&app, status))
}

async fn oauth_callback(
    State(state): State<Arc<CallbackState>>,
    Query(query): Query<HashMap<String, String>>,
) -> Html<&'static str> {
    let received_state = query.get("state").map(String::as_str).unwrap_or("");
    let state_matches = received_state.len() == state.expected_state.len()
        && received_state
            .as_bytes()
            .ct_eq(state.expected_state.as_bytes())
            .into();
    let result = if !state_matches {
        Err("World Monitor sign-in state did not match.".to_string())
    } else if let Some(error) = query.get("error") {
        Err(format!("World Monitor sign-in was declined: {error}"))
    } else {
        query
            .get("code")
            .filter(|code| !code.is_empty())
            .cloned()
            .ok_or_else(|| "World Monitor sign-in returned no code.".to_string())
    };
    if let Ok(mut sender) = state.sender.lock() {
        if let Some(sender) = sender.take() {
            let _ = sender.send(result);
        }
    }
    Html(
        "<!doctype html><title>Command Adviser</title><main><h1>World Monitor connected</h1><p>You can return to Command Adviser.</p></main>",
    )
}

fn random_urlsafe(length: usize) -> String {
    let mut bytes = vec![0_u8; length];
    rand::fill(bytes.as_mut_slice());
    URL_SAFE_NO_PAD.encode(bytes)
}

fn validate_oauth_value(value: &str, maximum: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > maximum
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(command_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_view_serialisation_has_only_status_and_usage() {
        let value = serde_json::to_value(WorldMonitorConnectionView {
            endpoint: DEFAULT_WORLD_MONITOR_ENDPOINT.to_string(),
            status: WorldMonitorConnectionStatus::Connected,
            brief_used: 3,
            brief_limit: 25,
            direct_used: 4,
            direct_limit: 25,
        })
        .expect("serialize");
        assert_eq!(
            value
                .as_object()
                .expect("object")
                .keys()
                .collect::<Vec<_>>(),
            [
                "briefLimit",
                "briefUsed",
                "directLimit",
                "directUsed",
                "endpoint",
                "status"
            ]
        );
        let serialized = value.to_string();
        assert!(!serialized.contains("access_token"));
        assert!(!serialized.contains("refresh_token"));
    }

    #[test]
    fn oauth_values_are_bounded() {
        assert!(validate_oauth_value("abc-123", 32).is_ok());
        assert!(validate_oauth_value("", 32).is_err());
        assert!(validate_oauth_value("contains space", 32).is_err());
        assert!(validate_oauth_value(&"x".repeat(33), 32).is_err());
    }
}
