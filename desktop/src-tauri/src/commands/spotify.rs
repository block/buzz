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

const SPOTIFY_CREDENTIAL_KEY: &str = "spotify-oauth";
const SPOTIFY_AUTH_URL: &str = "https://accounts.spotify.com/authorize";
const SPOTIFY_TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const SPOTIFY_API_BASE_URL: &str = "https://api.spotify.com/v1";
const SPOTIFY_SCOPES: &str =
    "user-read-playback-state user-read-currently-playing user-modify-playback-state";
const OAUTH_CALLBACK_PATH: &str = "/oauth/spotify/callback";
const OAUTH_CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const DEFAULT_SPOTIFY_REDIRECT_PORT: u16 = 18202;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpotifyCredential {
    access_token: Option<String>,
    connected_at: i64,
    expires_at: Option<i64>,
    refresh_token: String,
    scope: Option<String>,
    token_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SpotifyStatus {
    configured: bool,
    connected: bool,
    connected_at: Option<i64>,
    scopes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SpotifyDevice {
    id: Option<String>,
    name: String,
    device_type: String,
    is_active: bool,
    is_restricted: bool,
    volume_percent: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SpotifyPlaybackState {
    context_uri: Option<String>,
    device: Option<SpotifyDevice>,
    is_playing: bool,
    item: Option<SpotifyPlaybackItem>,
    progress_ms: Option<i64>,
    timestamp: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SpotifyPlaybackItem {
    artists: Vec<String>,
    duration_ms: Option<i64>,
    image_url: Option<String>,
    item_type: Option<String>,
    name: String,
    uri: String,
}

#[derive(Debug, Deserialize)]
struct SpotifyTokenResponse {
    access_token: Option<String>,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    scope: Option<String>,
    token_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SpotifyDevicesResponse {
    devices: Vec<SpotifyRawDevice>,
}

#[derive(Debug, Deserialize)]
struct SpotifyRawDevice {
    id: Option<String>,
    #[serde(default)]
    is_active: bool,
    #[serde(default)]
    is_restricted: bool,
    #[serde(default)]
    name: String,
    #[serde(default, rename = "type")]
    device_type: String,
    volume_percent: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SpotifyRawPlaybackState {
    context: Option<SpotifyRawContext>,
    device: Option<SpotifyRawDevice>,
    #[serde(default)]
    is_playing: bool,
    item: Option<SpotifyRawPlaybackItem>,
    progress_ms: Option<i64>,
    timestamp: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SpotifyRawContext {
    uri: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SpotifyRawPlaybackItem {
    album: Option<SpotifyRawAlbum>,
    artists: Option<Vec<SpotifyRawArtist>>,
    duration_ms: Option<i64>,
    name: Option<String>,
    #[serde(rename = "type")]
    item_type: Option<String>,
    uri: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SpotifyRawAlbum {
    images: Option<Vec<SpotifyRawImage>>,
}

#[derive(Debug, Deserialize)]
struct SpotifyRawArtist {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SpotifyRawImage {
    url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotifyPlaybackInput {
    context_uri: Option<String>,
    device_id: Option<String>,
    position_ms: Option<u32>,
    uris: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotifyDeviceInput {
    device_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotifySeekInput {
    device_id: Option<String>,
    position_ms: u32,
}

#[derive(Debug, Serialize)]
struct SpotifyPlaybackBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    context_uri: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uris: Option<&'a [String]>,
}

fn spotify_client_id() -> Option<String> {
    std::env::var("BUZZ_SPOTIFY_CLIENT_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            option_env!("BUZZ_SPOTIFY_CLIENT_ID")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn spotify_redirect_port() -> u16 {
    std::env::var("BUZZ_SPOTIFY_REDIRECT_PORT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .filter(|port| *port != 0)
        .unwrap_or(DEFAULT_SPOTIFY_REDIRECT_PORT)
}

fn credential_store() -> &'static SecretStore {
    SecretStore::shared(KEYRING_SERVICE)
}

fn now_ts() -> i64 {
    Utc::now().timestamp()
}

fn load_credential() -> Result<Option<SpotifyCredential>, String> {
    let Some(raw) = credential_store().load(SPOTIFY_CREDENTIAL_KEY)? else {
        return Ok(None);
    };
    serde_json::from_str(&raw).map_err(|e| format!("parse Spotify credential: {e}"))
}

fn save_credential(credential: &SpotifyCredential) -> Result<(), String> {
    let raw = serde_json::to_string(credential)
        .map_err(|e| format!("serialize Spotify credential: {e}"))?;
    credential_store().store(SPOTIFY_CREDENTIAL_KEY, &raw)
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

fn status_from_credential(credential: Option<&SpotifyCredential>) -> SpotifyStatus {
    SpotifyStatus {
        configured: spotify_client_id().is_some(),
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
    let mut url = Url::parse(SPOTIFY_AUTH_URL).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SPOTIFY_SCOPES)
        .append_pair("state", state)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256");
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
        .map_err(|_| "Timed out waiting for Spotify authorization.".to_string())?
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
        Err("Spotify returned an unexpected OAuth callback path.".to_string())
    } else if let Some(error) = error {
        Err(format!("Spotify authorization couldn't complete: {error}"))
    } else if state.as_deref() != Some(expected_state) {
        Err("Spotify authorization state did not match.".to_string())
    } else {
        code.ok_or_else(|| "Spotify authorization returned no code.".to_string())
    };

    let (title, body) = if result.is_ok() {
        (
            "Spotify connected",
            "You can close this browser tab and return to Buzz.",
        )
    } else {
        (
            "Spotify connection incomplete",
            "Return to Buzz to try again.",
        )
    };
    let response = callback_response(title, body);
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;

    result
}

async fn parse_spotify_token_response(
    response: reqwest::Response,
) -> Result<SpotifyTokenResponse, String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("read Spotify token response: {e}"))?;
    if !status.is_success() {
        return Err(format!("Spotify token request returned {status}: {body}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("parse Spotify token response: {e}"))
}

async fn exchange_code_for_token(
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    client_id: &str,
    state: &AppState,
) -> Result<SpotifyTokenResponse, String> {
    let form = vec![
        ("client_id", client_id.to_string()),
        ("code", code.to_string()),
        ("code_verifier", code_verifier.to_string()),
        ("grant_type", "authorization_code".to_string()),
        ("redirect_uri", redirect_uri.to_string()),
    ];

    let response = state
        .http_client
        .post(SPOTIFY_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Spotify OAuth token exchange couldn't complete: {e}"))?;
    parse_spotify_token_response(response).await
}

async fn refresh_access_token(
    credential: &SpotifyCredential,
    client_id: &str,
    state: &AppState,
) -> Result<SpotifyTokenResponse, String> {
    let form = vec![
        ("client_id", client_id.to_string()),
        ("refresh_token", credential.refresh_token.clone()),
        ("grant_type", "refresh_token".to_string()),
    ];

    let response = state
        .http_client
        .post(SPOTIFY_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Spotify token refresh couldn't complete: {e}"))?;
    parse_spotify_token_response(response).await
}

async fn access_token(state: &AppState) -> Result<String, String> {
    let client_id = spotify_client_id()
        .ok_or_else(|| "Set BUZZ_SPOTIFY_CLIENT_ID to enable Spotify.".to_string())?;
    let mut credential = load_credential()?
        .ok_or_else(|| "Connect Spotify before controlling playback.".to_string())?;

    if let (Some(token), Some(expires_at)) = (&credential.access_token, credential.expires_at) {
        if expires_at > now_ts() + 60 {
            return Ok(token.clone());
        }
    }

    let token = refresh_access_token(&credential, &client_id, state).await?;
    let access_token = token
        .access_token
        .ok_or_else(|| "Spotify token refresh returned no access token.".to_string())?;
    credential.access_token = Some(access_token.clone());
    credential.expires_at = token.expires_in.map(|seconds| now_ts() + seconds);
    if let Some(refresh_token) = token.refresh_token {
        credential.refresh_token = refresh_token;
    }
    credential.scope = token.scope.or(credential.scope);
    credential.token_type = token.token_type.or(credential.token_type);
    save_credential(&credential)?;
    Ok(access_token)
}

fn convert_device(device: SpotifyRawDevice) -> SpotifyDevice {
    SpotifyDevice {
        id: device.id,
        name: device.name,
        device_type: device.device_type,
        is_active: device.is_active,
        is_restricted: device.is_restricted,
        volume_percent: device.volume_percent,
    }
}

fn convert_playback_item(item: SpotifyRawPlaybackItem) -> Option<SpotifyPlaybackItem> {
    let uri = item.uri?;
    let name = item.name.unwrap_or_else(|| "Untitled".to_string());
    let artists = item
        .artists
        .unwrap_or_default()
        .into_iter()
        .filter_map(|artist| artist.name)
        .collect();
    let image_url = item
        .album
        .and_then(|album| album.images)
        .and_then(|images| images.into_iter().find_map(|image| image.url));

    Some(SpotifyPlaybackItem {
        artists,
        duration_ms: item.duration_ms,
        image_url,
        item_type: item.item_type,
        name,
        uri,
    })
}

fn convert_playback_state(state: SpotifyRawPlaybackState) -> SpotifyPlaybackState {
    SpotifyPlaybackState {
        context_uri: state.context.and_then(|context| context.uri),
        device: state.device.map(convert_device),
        is_playing: state.is_playing,
        item: state.item.and_then(convert_playback_item),
        progress_ms: state.progress_ms,
        timestamp: state.timestamp,
    }
}

async fn spotify_json<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    context: &str,
) -> Result<T, String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("read Spotify {context} response: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "Spotify {context} request returned {status}: {body}"
        ));
    }
    serde_json::from_str(&body).map_err(|e| format!("parse Spotify {context} response: {e}"))
}

async fn spotify_empty(response: reqwest::Response, context: &str) -> Result<(), String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("read Spotify {context} response: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "Spotify {context} request returned {status}: {body}"
        ));
    }
    Ok(())
}

#[tauri::command]
pub fn get_spotify_status() -> Result<SpotifyStatus, String> {
    if spotify_client_id().is_none() {
        return Ok(status_from_credential(None));
    }

    Ok(status_from_credential(load_credential()?.as_ref()))
}

#[tauri::command]
pub async fn connect_spotify(state: State<'_, AppState>) -> Result<SpotifyStatus, String> {
    let client_id = spotify_client_id()
        .ok_or_else(|| "Set BUZZ_SPOTIFY_CLIENT_ID to enable Spotify.".to_string())?;
    let redirect_port = spotify_redirect_port();
    let listener = TcpListener::bind(("127.0.0.1", redirect_port))
        .await
        .map_err(|e| format!("bind Spotify OAuth callback on port {redirect_port}: {e}"))?;
    let redirect_uri = format!("http://127.0.0.1:{redirect_port}{OAUTH_CALLBACK_PATH}");
    let code_verifier = pkce_verifier();
    let state_token = oauth_state();
    let auth_url = oauth_authorization_url(
        &client_id,
        &redirect_uri,
        &state_token,
        &pkce_challenge(&code_verifier),
    )?;

    tauri_plugin_opener::open_url(auth_url.as_str(), None::<&str>)
        .map_err(|e| format!("open Spotify authorization page: {e}"))?;

    let code = wait_for_oauth_callback(listener, &state_token).await?;
    let token =
        exchange_code_for_token(&code, &code_verifier, &redirect_uri, &client_id, &state).await?;
    let refresh_token = token.refresh_token.ok_or_else(|| {
        "Spotify did not return a refresh token. Disconnect and try again.".to_string()
    })?;
    let credential = SpotifyCredential {
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
pub fn disconnect_spotify() -> Result<SpotifyStatus, String> {
    credential_store().delete(SPOTIFY_CREDENTIAL_KEY)?;
    Ok(status_from_credential(None))
}

#[tauri::command]
pub async fn get_spotify_devices(state: State<'_, AppState>) -> Result<Vec<SpotifyDevice>, String> {
    let access_token = access_token(&state).await?;
    let url = format!("{SPOTIFY_API_BASE_URL}/me/player/devices");
    let response = state
        .http_client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("Spotify devices request couldn't complete: {e}"))?;
    let raw: SpotifyDevicesResponse = spotify_json(response, "devices").await?;
    Ok(raw.devices.into_iter().map(convert_device).collect())
}

#[tauri::command]
pub async fn get_spotify_playback_state(
    state: State<'_, AppState>,
) -> Result<Option<SpotifyPlaybackState>, String> {
    let access_token = access_token(&state).await?;
    let url = format!("{SPOTIFY_API_BASE_URL}/me/player");
    let response = state
        .http_client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("Spotify playback state request couldn't complete: {e}"))?;

    if response.status().as_u16() == 204 {
        return Ok(None);
    }

    let raw: SpotifyRawPlaybackState = spotify_json(response, "playback state").await?;
    Ok(Some(convert_playback_state(raw)))
}

#[tauri::command]
pub async fn start_spotify_playback(
    input: Option<SpotifyPlaybackInput>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let access_token = access_token(&state).await?;
    let input = input.unwrap_or_default();
    let mut url =
        Url::parse(&format!("{SPOTIFY_API_BASE_URL}/me/player/play")).map_err(|e| e.to_string())?;
    if let Some(device_id) = input.device_id.as_deref().filter(|value| !value.is_empty()) {
        url.query_pairs_mut().append_pair("device_id", device_id);
    }

    let uris = input.uris.as_ref().filter(|values| !values.is_empty());
    let body = SpotifyPlaybackBody {
        context_uri: input
            .context_uri
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        position_ms: input.position_ms,
        uris: uris.map(Vec::as_slice),
    };
    let has_body = body.context_uri.is_some() || body.position_ms.is_some() || body.uris.is_some();
    let request = state.http_client.put(url).bearer_auth(access_token);
    let response = if has_body {
        request.json(&body).send().await
    } else {
        request.send().await
    }
    .map_err(|e| format!("Spotify playback request couldn't complete: {e}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("read Spotify playback response: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "Spotify playback request returned {status}: {body}"
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn pause_spotify_playback(
    input: Option<SpotifyDeviceInput>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let access_token = access_token(&state).await?;
    let mut url = Url::parse(&format!("{SPOTIFY_API_BASE_URL}/me/player/pause"))
        .map_err(|e| e.to_string())?;
    if let Some(device_id) = input
        .and_then(|input| input.device_id)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        url.query_pairs_mut().append_pair("device_id", &device_id);
    }

    let response = state
        .http_client
        .put(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("Spotify pause request couldn't complete: {e}"))?;
    spotify_empty(response, "pause").await
}

#[tauri::command]
pub async fn skip_spotify_next(
    input: Option<SpotifyDeviceInput>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let access_token = access_token(&state).await?;
    let mut url =
        Url::parse(&format!("{SPOTIFY_API_BASE_URL}/me/player/next")).map_err(|e| e.to_string())?;
    if let Some(device_id) = input
        .and_then(|input| input.device_id)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        url.query_pairs_mut().append_pair("device_id", &device_id);
    }

    let response = state
        .http_client
        .post(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("Spotify next track request couldn't complete: {e}"))?;
    spotify_empty(response, "next track").await
}

#[tauri::command]
pub async fn skip_spotify_previous(
    input: Option<SpotifyDeviceInput>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let access_token = access_token(&state).await?;
    let mut url = Url::parse(&format!("{SPOTIFY_API_BASE_URL}/me/player/previous"))
        .map_err(|e| e.to_string())?;
    if let Some(device_id) = input
        .and_then(|input| input.device_id)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        url.query_pairs_mut().append_pair("device_id", &device_id);
    }

    let response = state
        .http_client
        .post(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("Spotify previous track request couldn't complete: {e}"))?;
    spotify_empty(response, "previous track").await
}

#[tauri::command]
pub async fn seek_spotify_playback(
    input: SpotifySeekInput,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let access_token = access_token(&state).await?;
    let mut url =
        Url::parse(&format!("{SPOTIFY_API_BASE_URL}/me/player/seek")).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("position_ms", &input.position_ms.to_string());
    if let Some(device_id) = input
        .device_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        url.query_pairs_mut().append_pair("device_id", device_id);
    }

    let response = state
        .http_client
        .put(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("Spotify seek request couldn't complete: {e}"))?;
    spotify_empty(response, "seek").await
}
