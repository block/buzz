use serde_json::Value;
use std::time::Duration;

const INVITE_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_INVITE_RESPONSE_BYTES: u64 = 1_048_576;

/// POST a signed invite request through native networking so the desktop app
/// does not depend on browser CORS policy. Redirects are disabled because the
/// NIP-98 authorization event is bound to the exact URL.
#[tauri::command]
pub async fn post_invite_api(
    url: String,
    authorization: String,
    body: String,
) -> Result<Value, String> {
    let parsed = reqwest::Url::parse(&url).map_err(|_| "invalid invite URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.path().starts_with("/api/invites")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("invalid invite URL".to_string());
    }

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("failed to build invite client: {error}"))?;
    let response = client
        .post(parsed)
        .header(reqwest::header::AUTHORIZATION, authorization)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .timeout(INVITE_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("invite request failed: {error}"))?;

    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_INVITE_RESPONSE_BYTES)
    {
        return Err("relay returned oversized invite response".to_string());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("reading invite response failed: {error}"))?;
    if bytes.len() as u64 > MAX_INVITE_RESPONSE_BYTES {
        return Err("relay returned oversized invite response".to_string());
    }
    let json: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "relay returned malformed invite response".to_string())?;
    if !status.is_success() {
        return Err(json
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16())));
    }
    Ok(json)
}
