use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;
use uuid::Uuid;

use crate::app_state::{AppState, KEYRING_SERVICE};
use crate::secret_store::SecretStore;

const GOOGLE_CALENDAR_CREDENTIAL_KEY: &str = "google-calendar-oauth";
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_CALENDAR_EVENTS_READONLY_SCOPE: &str =
    "https://www.googleapis.com/auth/calendar.events.readonly";
const OAUTH_CALLBACK_PATH: &str = "/";
const OAUTH_CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GoogleCalendarCredential {
    access_token: Option<String>,
    connected_at: i64,
    expires_at: Option<i64>,
    refresh_token: String,
    scope: Option<String>,
    token_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GoogleCalendarStatus {
    configured: bool,
    connected: bool,
    connected_at: Option<i64>,
    scopes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct GoogleCalendarEvent {
    id: String,
    title: String,
    starts_at: String,
    ends_at: String,
    all_day: bool,
    join_url: Option<String>,
    html_url: Option<String>,
    transparency: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: Option<String>,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    scope: Option<String>,
    token_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleEventsResponse {
    items: Option<Vec<GoogleRawEvent>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleRawEvent {
    id: Option<String>,
    summary: Option<String>,
    status: Option<String>,
    start: Option<GoogleRawEventTime>,
    end: Option<GoogleRawEventTime>,
    hangout_link: Option<String>,
    html_link: Option<String>,
    conference_data: Option<GoogleConferenceData>,
    transparency: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleRawEventTime {
    date: Option<String>,
    date_time: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleConferenceData {
    entry_points: Option<Vec<GoogleConferenceEntryPoint>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleConferenceEntryPoint {
    entry_point_type: Option<String>,
    uri: Option<String>,
}

fn google_calendar_client_id() -> Option<String> {
    std::env::var("BUZZ_GOOGLE_CALENDAR_CLIENT_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            option_env!("BUZZ_GOOGLE_CALENDAR_CLIENT_ID")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn google_calendar_client_secret() -> Option<String> {
    std::env::var("BUZZ_GOOGLE_CALENDAR_CLIENT_SECRET")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            option_env!("BUZZ_GOOGLE_CALENDAR_CLIENT_SECRET")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn credential_store() -> &'static SecretStore {
    SecretStore::shared(KEYRING_SERVICE)
}

fn now_ts() -> i64 {
    Utc::now().timestamp()
}

fn load_credential() -> Result<Option<GoogleCalendarCredential>, String> {
    let Some(raw) = credential_store().load(GOOGLE_CALENDAR_CREDENTIAL_KEY)? else {
        return Ok(None);
    };
    serde_json::from_str(&raw).map_err(|e| format!("parse Google Calendar credential: {e}"))
}

fn save_credential(credential: &GoogleCalendarCredential) -> Result<(), String> {
    let raw = serde_json::to_string(credential)
        .map_err(|e| format!("serialize Google Calendar credential: {e}"))?;
    credential_store().store(GOOGLE_CALENDAR_CREDENTIAL_KEY, &raw)
}

fn scopes(scope: Option<&str>) -> Vec<String> {
    scope
        .unwrap_or("")
        .split_whitespace()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn status_from_credential(credential: Option<&GoogleCalendarCredential>) -> GoogleCalendarStatus {
    GoogleCalendarStatus {
        configured: google_calendar_client_id().is_some(),
        connected: credential.is_some(),
        connected_at: credential.map(|value| value.connected_at),
        scopes: scopes(credential.and_then(|value| value.scope.as_deref())),
    }
}

fn pkce_verifier() -> String {
    [
        Uuid::new_v4().simple().to_string(),
        Uuid::new_v4().simple().to_string(),
        Uuid::new_v4().simple().to_string(),
    ]
    .join("")
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn oauth_state() -> String {
    [
        Uuid::new_v4().simple().to_string(),
        Uuid::new_v4().simple().to_string(),
    ]
    .join("")
}

fn oauth_authorization_url(
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> Result<Url, String> {
    let mut url = Url::parse(GOOGLE_AUTH_URL).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", GOOGLE_CALENDAR_EVENTS_READONLY_SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("prompt", "consent")
        .append_pair("access_type", "offline");
    Ok(url)
}

fn callback_response(title: &str, body: &str) -> String {
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head>\
         <body><main style=\"font-family: system-ui, sans-serif; max-width: 34rem; margin: 4rem auto;\">\
         <h1>{title}</h1><p>{body}</p></main></body></html>"
    );
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    )
}

async fn wait_for_oauth_callback(
    listener: TcpListener,
    expected_state: &str,
) -> Result<String, String> {
    let (mut stream, _) = tokio::time::timeout(OAUTH_CALLBACK_TIMEOUT, listener.accept())
        .await
        .map_err(|_| "Timed out waiting for Google Calendar authorization.".to_string())?
        .map_err(|e| format!("accept OAuth callback: {e}"))?;

    let mut buffer = [0_u8; 8192];
    let bytes_read = stream
        .read(&mut buffer)
        .await
        .map_err(|e| format!("read OAuth callback: {e}"))?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let request_target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| "Invalid OAuth callback request.".to_string())?;
    let callback_url = Url::parse(&format!("http://127.0.0.1{request_target}"))
        .map_err(|e| format!("parse OAuth callback: {e}"))?;

    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in callback_url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }

    let result = if callback_url.path() != OAUTH_CALLBACK_PATH {
        Err("Google returned an unexpected OAuth callback path.".to_string())
    } else if let Some(error) = error {
        Err(format!("Google Calendar authorization failed: {error}"))
    } else if state.as_deref() != Some(expected_state) {
        Err("Google Calendar authorization state did not match.".to_string())
    } else {
        code.ok_or_else(|| "Google Calendar authorization returned no code.".to_string())
    };

    let (title, body) = if result.is_ok() {
        (
            "Google Calendar authorization received",
            "Return to Buzz to finish connecting Google Calendar.",
        )
    } else {
        (
            "Google Calendar connection failed",
            "Return to Buzz to try connecting again.",
        )
    };
    let response = callback_response(title, body);
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;

    result
}

async fn exchange_code_for_token(
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    client_id: &str,
    state: &AppState,
) -> Result<GoogleTokenResponse, String> {
    let mut form = vec![
        ("client_id", client_id.to_string()),
        ("code", code.to_string()),
        ("code_verifier", code_verifier.to_string()),
        ("grant_type", "authorization_code".to_string()),
        ("redirect_uri", redirect_uri.to_string()),
    ];
    if let Some(client_secret) = google_calendar_client_secret() {
        form.push(("client_secret", client_secret));
    }

    let response = state
        .http_client
        .post(GOOGLE_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Google OAuth token exchange failed: {e}"))?;
    parse_google_token_response(response).await
}

async fn refresh_access_token(
    credential: &GoogleCalendarCredential,
    client_id: &str,
    state: &AppState,
) -> Result<GoogleTokenResponse, String> {
    let mut form = vec![
        ("client_id", client_id.to_string()),
        ("refresh_token", credential.refresh_token.clone()),
        ("grant_type", "refresh_token".to_string()),
    ];
    if let Some(client_secret) = google_calendar_client_secret() {
        form.push(("client_secret", client_secret));
    }

    let response = state
        .http_client
        .post(GOOGLE_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Google Calendar token refresh failed: {e}"))?;
    parse_google_token_response(response).await
}

async fn parse_google_token_response(
    response: reqwest::Response,
) -> Result<GoogleTokenResponse, String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("read Google token response: {e}"))?;
    if !status.is_success() {
        return Err(format!("Google token request failed ({status}): {body}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("parse Google token response: {e}"))
}

async fn access_token(state: &AppState) -> Result<String, String> {
    let client_id = google_calendar_client_id().ok_or_else(|| {
        "Set BUZZ_GOOGLE_CALENDAR_CLIENT_ID to enable Google Calendar.".to_string()
    })?;
    let mut credential = load_credential()?
        .ok_or_else(|| "Connect Google Calendar before reading events.".to_string())?;

    if let (Some(token), Some(expires_at)) = (&credential.access_token, credential.expires_at) {
        if expires_at > now_ts() + 60 {
            return Ok(token.clone());
        }
    }

    let token = refresh_access_token(&credential, &client_id, state).await?;
    let access_token = token
        .access_token
        .ok_or_else(|| "Google token refresh returned no access token.".to_string())?;
    credential.access_token = Some(access_token.clone());
    credential.expires_at = token.expires_in.map(|seconds| now_ts() + seconds);
    credential.scope = token.scope.or(credential.scope);
    credential.token_type = token.token_type.or(credential.token_type);
    save_credential(&credential)?;
    Ok(access_token)
}

fn is_http_url(value: &str) -> bool {
    Url::parse(value)
        .map(|url| url.scheme() == "http" || url.scheme() == "https")
        .unwrap_or(false)
}

fn event_join_url(event: &GoogleRawEvent) -> Option<String> {
    event
        .conference_data
        .as_ref()
        .and_then(|data| data.entry_points.as_ref())
        .and_then(|entry_points| {
            entry_points
                .iter()
                .find(|entry| entry.entry_point_type.as_deref() == Some("video"))
                .and_then(|entry| entry.uri.as_deref())
                .filter(|value| is_http_url(value))
                .map(ToOwned::to_owned)
                .or_else(|| {
                    entry_points
                        .iter()
                        .filter_map(|entry| entry.uri.as_deref())
                        .find(|value| is_http_url(value))
                        .map(ToOwned::to_owned)
                })
        })
        .or_else(|| {
            event
                .hangout_link
                .as_deref()
                .filter(|value| is_http_url(value))
                .map(ToOwned::to_owned)
        })
}

fn convert_google_event(event: GoogleRawEvent) -> Option<GoogleCalendarEvent> {
    if event.status.as_deref() == Some("cancelled") {
        return None;
    }

    let join_url = event_join_url(&event);
    let html_url = event
        .html_link
        .as_ref()
        .filter(|value| is_http_url(value))
        .cloned();
    let start = event.start?;
    let end = event.end?;
    let starts_at = start.date_time.or(start.date)?;
    let ends_at = end.date_time.or(end.date)?;
    let all_day = !starts_at.contains('T') && !ends_at.contains('T');

    Some(GoogleCalendarEvent {
        id: event
            .id
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string()),
        title: event.summary.unwrap_or_else(|| "Busy".to_string()),
        starts_at,
        ends_at,
        all_day,
        join_url,
        html_url,
        transparency: event.transparency,
    })
}

#[tauri::command]
pub fn get_google_calendar_status() -> Result<GoogleCalendarStatus, String> {
    if google_calendar_client_id().is_none() {
        return Ok(status_from_credential(None));
    }

    Ok(status_from_credential(load_credential()?.as_ref()))
}

#[tauri::command]
pub async fn connect_google_calendar(
    state: State<'_, AppState>,
) -> Result<GoogleCalendarStatus, String> {
    let client_id = google_calendar_client_id().ok_or_else(|| {
        "Set BUZZ_GOOGLE_CALENDAR_CLIENT_ID to enable Google Calendar.".to_string()
    })?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("bind Google Calendar OAuth callback: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("read OAuth callback address: {e}"))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");
    let code_verifier = pkce_verifier();
    let state_token = oauth_state();
    let auth_url = oauth_authorization_url(
        &client_id,
        &redirect_uri,
        &state_token,
        &pkce_challenge(&code_verifier),
    )?;

    tauri_plugin_opener::open_url(auth_url.as_str(), None::<&str>)
        .map_err(|e| format!("open Google authorization page: {e}"))?;

    let code = wait_for_oauth_callback(listener, &state_token).await?;
    let token =
        exchange_code_for_token(&code, &code_verifier, &redirect_uri, &client_id, &state).await?;
    let refresh_token = token.refresh_token.ok_or_else(|| {
        "Google did not return a refresh token. Disconnect and try again.".to_string()
    })?;
    let credential = GoogleCalendarCredential {
        access_token: token.access_token,
        connected_at: now_ts(),
        expires_at: token.expires_in.map(|seconds| now_ts() + seconds),
        refresh_token,
        scope: token.scope,
        token_type: token.token_type,
    };
    save_credential(&credential)?;

    Ok(status_from_credential(Some(&credential)))
}

#[tauri::command]
pub fn disconnect_google_calendar() -> Result<GoogleCalendarStatus, String> {
    credential_store().delete(GOOGLE_CALENDAR_CREDENTIAL_KEY)?;
    Ok(status_from_credential(None))
}

#[tauri::command]
pub async fn get_google_calendar_events(
    time_min: String,
    time_max: String,
    state: State<'_, AppState>,
) -> Result<Vec<GoogleCalendarEvent>, String> {
    let access_token = access_token(&state).await?;
    let mut url = Url::parse("https://www.googleapis.com/calendar/v3/calendars/primary/events")
        .map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("timeMin", &time_min)
        .append_pair("timeMax", &time_max)
        .append_pair("singleEvents", "true")
        .append_pair("orderBy", "startTime")
        .append_pair("showDeleted", "false")
        .append_pair("conferenceDataVersion", "1")
        .append_pair("maxResults", "50");

    let response = state
        .http_client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("Google Calendar events request failed: {e}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("read Google Calendar events response: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "Google Calendar events request failed ({status}): {body}"
        ));
    }

    let raw: GoogleEventsResponse =
        serde_json::from_str(&body).map_err(|e| format!("parse Google Calendar events: {e}"))?;
    Ok(raw
        .items
        .unwrap_or_default()
        .into_iter()
        .filter_map(convert_google_event)
        .collect())
}
