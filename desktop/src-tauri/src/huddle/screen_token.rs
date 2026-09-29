//! Mint / release huddle screen-share LiveKit tokens via Tauri HTTP.
//!
//! Browser `fetch` from the Vite/Tauri webview origin is blocked by prod
//! `BUZZ_CORS_ORIGINS` (relay only allows its own origin). NIP-98 POSTs go
//! through reqwest here so CORS does not apply. LiveKit `room.connect` stays
//! in the JS client.

use reqwest::Method;
use serde_json::{json, Value};
use tauri::State;

use crate::app_state::AppState;
use crate::relay::{
    build_nip98_auth_header, classify_request_error, relay_api_base_url_with_override,
};

const SCREEN_TOKEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Mint a short-lived LiveKit token for huddle screen share.
///
/// On relay 503 / `screen_share_unavailable`, returns JSON
/// `{ "unavailable": true, "reason": "..." }` so the TS layer can hide Share.
/// Other non-2xx responses become `Err` with a message the hook already handles
/// (`screen_share_busy`, membership errors, etc.).
#[tauri::command]
pub async fn huddle_screen_token(
    channel_id: String,
    parent_channel_id: Option<String>,
    intent: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if channel_id.trim().is_empty() {
        return Err("channel_id is required".into());
    }
    let intent = intent.trim().to_ascii_lowercase();
    if intent != "subscribe" && intent != "publish" {
        return Err("intent must be subscribe or publish".into());
    }

    let base = relay_api_base_url_with_override(&state);
    let url = format!(
        "{}/api/huddle/{}/screen-token",
        base.trim_end_matches('/'),
        channel_id.trim()
    );

    let mut body = json!({ "intent": intent });
    if let Some(parent) = parent_channel_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        body["parent_channel_id"] = json!(parent);
    }
    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("screen-token body serialize: {e}"))?;

    post_screen_json(&state, &url, body_bytes, /*map_unavailable*/ true).await
}

/// Best-effort release of the relay one-sharer soft lock.
#[tauri::command]
pub async fn huddle_screen_stop(
    channel_id: String,
    parent_channel_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if channel_id.trim().is_empty() {
        return Err("channel_id is required".into());
    }

    let base = relay_api_base_url_with_override(&state);
    let url = format!(
        "{}/api/huddle/{}/screen-stop",
        base.trim_end_matches('/'),
        channel_id.trim()
    );

    let body = match parent_channel_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(parent) => json!({ "parent_channel_id": parent }),
        None => json!({}),
    };
    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("screen-stop body serialize: {e}"))?;

    let _ = post_screen_json(&state, &url, body_bytes, /*map_unavailable*/ false).await?;
    Ok(())
}

async fn post_screen_json(
    state: &AppState,
    url: &str,
    body_bytes: Vec<u8>,
    map_unavailable: bool,
) -> Result<Value, String> {
    crate::relay_admission::wait_for_rate_limit().await;

    let auth = build_nip98_auth_header(&Method::POST, url, &body_bytes, state)?;
    let response = state
        .http_client
        .post(url)
        .header("Authorization", &auth)
        .header("Content-Type", "application/json")
        .timeout(SCREEN_TOKEN_TIMEOUT)
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| classify_request_error(&e))?;

    let status = response.status();
    let final_host = response.url().host_str().unwrap_or("").to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Captive portal / proxy HTML — same guard as other relay HTTP paths.
    let host_l = final_host.to_lowercase();
    let ct_l = content_type.to_lowercase();
    if host_l == "cloudflareaccess.com" || host_l.ends_with(".cloudflareaccess.com") {
        return Err(
            "relay unreachable: network sign-in required (Cloudflare Access / VPN) \
             — re-authenticate and reconnect"
                .into(),
        );
    }
    if ct_l.contains("text/html") {
        return Err("relay unreachable: relay returned an unexpected HTML page \
             (VPN or proxy sign-in?)"
            .into());
    }

    let body_text = response.text().await.map_err(|e| {
        if e.is_timeout() {
            classify_request_error(&e)
        } else {
            "malformed relay response".to_string()
        }
    })?;
    let json: Value = serde_json::from_str(&body_text).unwrap_or_else(|_| json!({}));

    let error_code = json.get("error").and_then(Value::as_str);
    if map_unavailable && (status.as_u16() == 503 || error_code == Some("screen_share_unavailable"))
    {
        let reason = json
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("screen_share_unavailable");
        return Ok(json!({
            "unavailable": true,
            "reason": reason,
        }));
    }

    if !status.is_success() {
        let message = match (error_code, json.get("message").and_then(Value::as_str)) {
            (Some(err), Some(msg)) => format!("{err}: {msg}"),
            (Some(err), None) => err.to_string(),
            (None, Some(msg)) => msg.to_string(),
            (None, None) => format!("HTTP {}", status.as_u16()),
        };
        return Err(message);
    }

    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_payload_shape() {
        let v = json!({
            "unavailable": true,
            "reason": "LiveKit is not configured on this relay",
        });
        assert_eq!(v["unavailable"], true);
        assert!(v["reason"].as_str().unwrap().contains("LiveKit"));
    }
}
