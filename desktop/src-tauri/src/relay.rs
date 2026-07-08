use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use sha2::{Digest, Sha256};

// nostr 0.36 alias — required for cross-version bridging with buzz-sdk.

use crate::app_state::AppState;

const DEFAULT_RELAY_WS_URL: &str = "ws://localhost:3000";

// A reached-but-malformed 2xx body is NOT a connectivity failure, so this
// message must never carry the "relay unreachable:" prefix the frontend
// classifier keys on. Extracted to a const so a test can pin that contract.
const MALFORMED_RESPONSE_MESSAGE: &str = "relay returned malformed response: not valid JSON";

fn configured_env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn relay_ws_url() -> String {
    configured_env_var("BUZZ_RELAY_URL")
        .or_else(|| option_env!("BUZZ_DESKTOP_BUILD_RELAY_URL").map(str::to_string))
        .unwrap_or_else(|| DEFAULT_RELAY_WS_URL.to_string())
}

/// Read the workspace relay URL override, if set. Returns `None` when no
/// override is active or when the mutex is poisoned (best-effort).
fn workspace_relay_override(state: &AppState) -> Option<String> {
    state
        .relay_url_override
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
}

/// Returns the relay WebSocket URL, checking the workspace override first.
/// Precedence: workspace override > env vars > build-time vars > default.
pub fn relay_ws_url_with_override(state: &AppState) -> String {
    workspace_relay_override(state).unwrap_or_else(relay_ws_url)
}

/// Returns the relay HTTP API base URL, checking the workspace override first.
/// Precedence: workspace override > env vars > build-time vars > default.
pub fn relay_api_base_url_with_override(state: &AppState) -> String {
    match workspace_relay_override(state) {
        Some(url) => relay_http_base_url(&url),
        None => relay_api_base_url(),
    }
}

/// Selects the relay a managed agent should use for a relay operation.
///
/// An explicit per-agent `relay_url` always wins (highest precedence), pinning
/// the agent to that relay regardless of the active workspace. An empty or
/// whitespace-only `relay_url` falls back to the active workspace relay, which
/// resolves at read-time so a never-set record reconciles, spawns, and re-syncs
/// on the session's relay instead of a stale stored value. Uniform for both
/// Local and Provider backends.
pub fn effective_agent_relay_url(record_relay: &str, workspace_relay: &str) -> String {
    let pinned = record_relay.trim();
    if pinned.is_empty() {
        workspace_relay.to_string()
    } else {
        pinned.to_string()
    }
}

pub fn relay_http_base_url(relay_url: &str) -> String {
    let trimmed = relay_url.trim().trim_end_matches('/');

    if let Some(suffix) = trimmed.strip_prefix("wss://") {
        return format!("https://{}", suffix);
    }

    if let Some(suffix) = trimmed.strip_prefix("ws://") {
        return format!("http://{}", suffix);
    }

    trimmed.to_string()
}

pub fn relay_api_base_url() -> String {
    if let Some(base) = configured_env_var("BUZZ_RELAY_HTTP") {
        return base.trim_end_matches('/').to_string();
    }

    if let Some(base) = option_env!("BUZZ_DESKTOP_BUILD_RELAY_HTTP") {
        return base.trim().trim_end_matches('/').to_string();
    }

    relay_http_base_url(&relay_ws_url())
}

// ── NIP-98 HTTP auth ────────────────────────────────────────────────────────

pub fn build_nip98_auth_header(
    method: &Method,
    url: &str,
    body: &[u8],
    state: &AppState,
) -> Result<String, String> {
    let keys = state.keys.lock().map_err(|error| error.to_string())?;
    build_nip98_auth_header_for_keys(&keys, method, url, body)
}

pub fn build_nip98_auth_header_for_keys(
    keys: &Keys,
    method: &Method,
    url: &str,
    body: &[u8],
) -> Result<String, String> {
    let payload_hash = hex::encode(Sha256::digest(body));

    // Nonce ensures unique event IDs even for identical requests in the same second.
    // Without this, rapid-fire calls (e.g. query → submit → re-query) with the same
    // body produce identical NIP-98 event hashes and trigger relay replay detection.
    let nonce_hex = uuid::Uuid::new_v4().to_string();

    let tags = vec![
        Tag::parse(vec!["u", url]).map_err(|error| format!("url tag failed: {error}"))?,
        Tag::parse(vec!["method", method.as_str()])
            .map_err(|error| format!("method tag failed: {error}"))?,
        Tag::parse(vec!["payload", &payload_hash])
            .map_err(|error| format!("payload tag failed: {error}"))?,
        Tag::parse(vec!["nonce", &nonce_hex])
            .map_err(|error| format!("nonce tag failed: {error}"))?,
    ];

    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|error| format!("sign failed: {error}"))?;

    Ok(format!(
        "Nostr {}",
        BASE64.encode(event.as_json().as_bytes())
    ))
}

// ── Error handling ──────────────────────────────────────────────────────────

/// Classify a `send()` failure into a stable, URL-free error string.
///
/// The returned string always starts with `"relay unreachable:"` so the
/// frontend connectivity classifier can detect it with a simple prefix check.
pub(crate) fn classify_request_error(e: &reqwest::Error) -> String {
    let display = e.to_string().to_lowercase();
    if e.is_timeout() {
        "relay unreachable: request timed out".to_string()
    } else if e.is_connect() {
        "relay unreachable: could not connect to relay".to_string()
    } else if display.contains("dns") || display.contains("failed to lookup") {
        "relay unreachable: relay host not found".to_string()
    } else {
        "relay unreachable: network error".to_string()
    }
}

/// Detect responses that were intercepted by a captive portal or auth proxy.
///
/// Returns `Some(msg)` when the response clearly did not come from the relay:
/// - Cloudflare Access redirect (final URL on `*.cloudflareaccess.com`)
/// - Any other HTML response (proxy login page, captive portal, etc.)
///
/// Pure function: takes the already-extracted host and content-type strings so
/// it can be unit-tested without constructing a real `reqwest::Response`.
fn classify_intercepted_response(final_host: &str, content_type: &str) -> Option<String> {
    let host = final_host.to_lowercase();
    let ct = content_type.to_lowercase();

    // Cloudflare Access intercepts requests and redirects to its own domain.
    // Label-boundary check prevents `notcloudflareaccess.com.evil.example` from
    // matching.
    if host == "cloudflareaccess.com" || host.ends_with(".cloudflareaccess.com") {
        return Some(
            "relay unreachable: network sign-in required (Cloudflare Access / VPN) \
             — re-authenticate and reconnect"
                .to_string(),
        );
    }

    // Generic HTML body from any other proxy or captive portal.
    if ct.contains("text/html") {
        return Some(
            "relay unreachable: relay returned an unexpected HTML page \
             (VPN or proxy sign-in?)"
                .to_string(),
        );
    }

    None
}

/// Deserialize a successful response as JSON, guarding against intercepted pages.
///
/// Extracts the final URL host and `Content-Type` header before consuming the
/// response body. If the response looks like a captive-portal page, returns the
/// appropriate `"relay unreachable:"` message instead of attempting JSON parsing.
/// URL details are deliberately omitted from error strings so raw URLs are never
/// surfaced in the UI.
pub(crate) async fn parse_json_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, String> {
    let final_host = response.url().host_str().unwrap_or("").to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if let Some(msg) = classify_intercepted_response(&final_host, &content_type) {
        return Err(msg);
    }

    // A successful HTTP response whose body fails to deserialize means the relay
    // was reached but returned something unexpected (protocol mismatch, relay bug,
    // corrupted body) — NOT a connectivity failure. Keep it off the
    // "relay unreachable:" bucket so it surfaces loudly instead of being treated
    // as a transient unreachable-relay condition. The reqwest error detail is
    // dropped because it contains the raw URL.
    response
        .json::<T>()
        .await
        .map_err(|_| MALFORMED_RESPONSE_MESSAGE.to_string())
}

pub async fn relay_error_message(response: reqwest::Response) -> String {
    let status = response.status();

    // Check for intercepted/proxy responses before reading the body.
    let final_host = response.url().host_str().unwrap_or("").to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if let Some(msg) = classify_intercepted_response(&final_host, &content_type) {
        return msg;
    }

    // Real relay error: extract the structured message field if available.
    let body = response.text().await.unwrap_or_default();

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(message) = value.get("message").and_then(serde_json::Value::as_str) {
            return format!("relay returned {status}: {message}");
        }

        if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
            return format!("relay returned {status}: {error}");
        }
    }

    // Non-JSON, non-HTML body: emit status only — no raw body in the UI.
    format!("relay returned {status}")
}

// ── HTTP bridge: POST /query ────────────────────────────────────────────────

/// Execute a one-shot query via the relay's HTTP bridge (`POST /query`).
///
/// Filters are serialized as a JSON array. The request is authenticated with
/// a NIP-98 event signed by the user's keys. Returns the deserialized array of
/// events.
pub async fn query_relay(
    state: &AppState,
    filters: &[serde_json::Value],
) -> Result<Vec<nostr::Event>, String> {
    query_relay_at(state, &relay_api_base_url_with_override(state), filters).await
}

/// Like [`query_relay`] but targets an explicit HTTP API base URL instead of
/// the workspace override. Used when a query must hit a specific relay (e.g.
/// reconciling an agent's profile on the relay where it was published).
pub async fn query_relay_at(
    state: &AppState,
    api_base_url: &str,
    filters: &[serde_json::Value],
) -> Result<Vec<nostr::Event>, String> {
    let url = format!("{}/query", api_base_url);
    let body_bytes =
        serde_json::to_vec(filters).map_err(|e| format!("filter serialization failed: {e}"))?;
    let auth = build_nip98_auth_header(&Method::POST, &url, &body_bytes, state)?;

    let response = state
        .http_client
        .post(&url)
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| classify_request_error(&e))?;

    if !response.status().is_success() {
        return Err(relay_error_message(response).await);
    }

    parse_json_response(response).await
}

// ── Command response parsing ────────────────────────────────────────────────

/// Parse a command-event OK message of the form `"response:<json>"`.
///
/// Buzz's command kinds (e.g. 41010, 30620, 46020) acknowledge writes via
/// relay OK messages whose payload is a `response:`-prefixed JSON document.
/// This helper strips the prefix and deserializes the remainder as `T`.
pub fn parse_command_response<T: DeserializeOwned>(message: &str) -> Result<T, String> {
    // Try the spec format first: "response:{...}".
    if let Some(json) = message.strip_prefix("response:") {
        return serde_json::from_str(json).map_err(|e| format!("response parse failed: {e}"));
    }
    // Fallback: raw JSON (backward compat for relays that omit the prefix).
    serde_json::from_str(message)
        .map_err(|e| format!("expected 'response:' prefix or valid JSON, got: {message} ({e})"))
}

// ── Profile event builder ───────────────────────────────────────────────────

/// Build a signed kind:0 profile event, optionally injecting a verified NIP-OA auth tag.
///
/// This is a pure function (no I/O) extracted from `sync_managed_agent_profile` so that
/// the event-building and auth-tag-injection logic can be unit tested without HTTP calls.
///
/// `buzz-sdk` uses `nostr 0.36` while the desktop crate uses `nostr 0.37`. Cross-version
/// bridging is done via hex-encoded public keys and raw tag slices — both versions share the
/// same wire format.
fn build_profile_event(
    agent_keys: &nostr::Keys,
    display_name: &str,
    avatar_url: Option<&str>,
    auth_tag_json: Option<&str>,
) -> Result<nostr::Event, String> {
    let builder = crate::events::build_profile(Some(display_name), None, avatar_url, None, None)?;

    let builder = if let Some(tag_json) = auth_tag_json {
        // Bridge nostr 0.37 PublicKey → nostr 0.36 PublicKey via hex encoding.
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        let compat_pubkey = nostr::PublicKey::from_hex(&agent_pubkey_hex)
            .map_err(|e| format!("failed to convert agent pubkey for auth verification: {e}"))?;

        // Verify Schnorr signature before injecting into profile event.
        buzz_sdk_pkg::nip_oa::verify_auth_tag(tag_json, &compat_pubkey)
            .map_err(|e| format!("auth tag verification failed for profile event: {e}"))?;

        // parse_auth_tag returns a nostr 0.36 Tag; bridge to nostr 0.37 via raw slice.
        let compat_tag = buzz_sdk_pkg::nip_oa::parse_auth_tag(tag_json)
            .map_err(|e| format!("failed to parse verified auth tag: {e}"))?;
        let tag = nostr::Tag::parse(compat_tag.as_slice())
            .map_err(|e| format!("failed to convert auth tag to nostr 0.37: {e}"))?;
        builder.tags([tag])
    } else {
        builder
    };

    builder
        .sign_with_keys(agent_keys)
        .map_err(|e| format!("failed to sign profile event: {e}"))
}

// ── Managed-agent profile sync ──────────────────────────────────────────────

/// Sync a managed agent's kind:0 profile event to the relay using NIP-98 auth.
///
/// The agent signs its own profile event and the NIP-98 HTTP-auth event, so no
/// API token is required.
pub async fn sync_managed_agent_profile(
    state: &AppState,
    relay_url: &str,
    agent_keys: &nostr::Keys,
    display_name: &str,
    avatar_url: Option<&str>,
    auth_tag: Option<&str>, // NIP-OA auth tag JSON
) -> Result<(), String> {
    // Build a signed kind:0 profile event (with optional NIP-OA auth tag).
    let event = build_profile_event(agent_keys, display_name, avatar_url, auth_tag)?;
    let event_json = event.as_json();
    let body_bytes = event_json.into_bytes();

    let url = format!("{}/events", relay_http_base_url(relay_url));
    let auth = build_nip98_auth_header_for_keys(agent_keys, &Method::POST, &url, &body_bytes)?;

    let mut request = state
        .http_client
        .post(&url)
        .header("Authorization", auth)
        .header("Content-Type", "application/json");
    if let Some(tag) = auth_tag {
        request = request.header("x-auth-tag", tag);
    }
    let response = request
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| classify_request_error(&e))?;

    if !response.status().is_success() {
        let msg = relay_error_message(response).await;
        return Err(format!(
            "Created the agent, but could not sync its profile metadata: {msg}"
        ));
    }

    Ok(())
}

// ── Agent profile query ─────────────────────────────────────────────────────

/// Query the relay for an agent's kind:0 profile event.
///
/// Queries the relay identified by `relay_url`. Callers uniformly pass the
/// relay resolved by `effective_agent_relay_url` for every agent regardless of
/// backend — an explicit per-agent pin, or the active workspace relay when the
/// agent has none — so the query targets the host the profile is actually
/// published to.
///
/// Returns the parsed profile content (display_name, picture) if a kind:0 event
/// exists for the given pubkey, or `None` if no profile is published.
pub async fn query_agent_profile(
    state: &AppState,
    relay_url: &str,
    agent_pubkey: &str,
) -> Result<Option<AgentProfileInfo>, String> {
    let filter = serde_json::json!({
        "authors": [agent_pubkey],
        "kinds": [0],
        "limit": 1
    });

    let events = query_relay_at(state, &relay_http_base_url(relay_url), &[filter]).await?;

    let Some(event) = events.first() else {
        return Ok(None);
    };

    let Ok(content) = serde_json::from_str::<serde_json::Value>(&event.content) else {
        return Ok(None);
    };

    Ok(Some(AgentProfileInfo {
        display_name: content
            .get("display_name")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        picture: content
            .get("picture")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }))
}

/// Parsed fields from a kind:0 profile event.
#[derive(Debug, Clone)]
pub struct AgentProfileInfo {
    pub display_name: Option<String>,
    pub picture: Option<String>,
}

// ── Signed-event submission ─────────────────────────────────────────────────

/// Response from `POST /events`.
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct SubmitEventResponse {
    pub event_id: String,
    pub accepted: bool,
    pub message: String,
}

/// Build an `EventBuilder` from the events module, sign it with the user's keys,
/// and POST the signed event to `/events` with NIP-98 auth.
pub async fn submit_event(
    builder: nostr::EventBuilder,
    state: &AppState,
) -> Result<SubmitEventResponse, String> {
    // All synchronous work (signing) must complete before any .await
    // so the MutexGuard is dropped and the future remains Send.
    let url = format!("{}/events", relay_api_base_url_with_override(state));
    let (auth_header, body_bytes) = {
        let keys = state.signing_keys()?;
        let event = builder
            .sign_with_keys(&keys)
            .map_err(|e| format!("failed to sign event: {e}"))?;
        let body = event.as_json().into_bytes();
        let auth = build_nip98_auth_header_for_keys(&keys, &Method::POST, &url, &body)?;
        (auth, body)
    }; // keys dropped here

    let response = state
        .http_client
        .post(&url)
        .header("Authorization", auth_header)
        .header("Content-Type", "application/json")
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| classify_request_error(&e))?;

    if !response.status().is_success() {
        return Err(relay_error_message(response).await);
    }

    let result: SubmitEventResponse = parse_json_response(response).await?;

    if !result.accepted {
        return Err(format!("relay rejected event: {}", result.message));
    }

    Ok(result)
}

/// POST an already-signed event to `/events` with NIP-98 auth.
///
/// The persona flush loop drains pre-signed events from the retention store,
/// so it must publish them verbatim — re-signing through `submit_event` would
/// mint a new `created_at`/signature and break the compare-and-clear that
/// `mark_synced` relies on. Only the NIP-98 request auth is signed here (with
/// the owner keys), and that lock is dropped before the `.await`.
pub async fn submit_signed_event(
    event: &nostr::Event,
    state: &AppState,
) -> Result<SubmitEventResponse, String> {
    let url = format!("{}/events", relay_api_base_url_with_override(state));
    let body_bytes = event.as_json().into_bytes();
    let auth_header = {
        let keys = state.signing_keys()?;
        build_nip98_auth_header_for_keys(&keys, &Method::POST, &url, &body_bytes)?
    }; // keys dropped here

    let response = state
        .http_client
        .post(&url)
        .header("Authorization", auth_header)
        .header("Content-Type", "application/json")
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| classify_request_error(&e))?;

    if !response.status().is_success() {
        return Err(relay_error_message(response).await);
    }

    let result: SubmitEventResponse = parse_json_response(response).await?;

    if !result.accepted {
        return Err(format!("relay rejected event: {}", result.message));
    }

    Ok(result)
}

/// Sign an event with explicit keys and POST it to `/events` with NIP-98 auth.
///
/// Managed-agent flows use this to publish as the agent itself while still
/// including the stored NIP-OA auth tag when the relay requires owner-backed
/// membership.
pub async fn submit_event_with_keys(
    builder: nostr::EventBuilder,
    state: &AppState,
    keys: &Keys,
    auth_tag: Option<&str>,
) -> Result<SubmitEventResponse, String> {
    let url = format!("{}/events", relay_api_base_url_with_override(state));
    let event = builder
        .sign_with_keys(keys)
        .map_err(|e| format!("failed to sign event: {e}"))?;
    let body_bytes = event.as_json().into_bytes();
    let auth_header = build_nip98_auth_header_for_keys(keys, &Method::POST, &url, &body_bytes)?;

    let mut request = state
        .http_client
        .post(&url)
        .header("Authorization", auth_header)
        .header("Content-Type", "application/json");
    if let Some(tag) = auth_tag {
        request = request.header("x-auth-tag", tag);
    }

    let response = request
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| classify_request_error(&e))?;

    if !response.status().is_success() {
        return Err(relay_error_message(response).await);
    }

    let result: SubmitEventResponse = parse_json_response(response).await?;

    if !result.accepted {
        return Err(format!("relay rejected event: {}", result.message));
    }

    Ok(result)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{
        build_profile_event, classify_intercepted_response, effective_agent_relay_url,
        parse_command_response, relay_http_base_url, MALFORMED_RESPONSE_MESSAGE,
    };
    use serde::Deserialize;

    // ── effective_agent_relay_url: per-agent override precedence ─────────────

    #[test]
    fn explicit_relay_wins_over_workspace() {
        // An explicit per-agent relay pins the agent there regardless of the
        // active workspace — this is the override taking highest precedence.
        assert_eq!(
            effective_agent_relay_url("wss://relay.other.com", "wss://staging.example.com"),
            "wss://relay.other.com"
        );
    }

    #[test]
    fn explicit_relay_wins_even_when_equal_to_workspace() {
        // No special-casing when the pin happens to match the active workspace.
        assert_eq!(
            effective_agent_relay_url("wss://staging.example.com", "wss://staging.example.com"),
            "wss://staging.example.com"
        );
    }

    #[test]
    fn empty_relay_falls_back_to_workspace() {
        // A never-set record resolves to the active workspace relay at read-time,
        // so a stale stored default can never make it load-bearing.
        assert_eq!(
            effective_agent_relay_url("", "wss://staging.example.com"),
            "wss://staging.example.com"
        );
    }

    #[test]
    fn whitespace_only_relay_falls_back_to_workspace() {
        // Whitespace-only is treated as unset, same as empty.
        assert_eq!(
            effective_agent_relay_url("   ", "wss://staging.example.com"),
            "wss://staging.example.com"
        );
    }

    // ── relay_http_base_url scheme conversion ────────────────────────────────

    #[test]
    fn loopback_ws_localhost_preserves_authority() {
        // Tenant host-binding keys off the HTTP Host/authority. The desktop must
        // not rewrite localhost to 127.0.0.1, or local dev HTTP calls target a
        // different unmapped community than the WebSocket URL.
        assert_eq!(
            relay_http_base_url("ws://localhost:3000"),
            "http://localhost:3000"
        );
    }

    #[test]
    fn loopback_trailing_slash_removed_authority_preserved() {
        assert_eq!(
            relay_http_base_url("ws://localhost:3000/"),
            "http://localhost:3000"
        );
    }

    #[test]
    fn remote_wss_host_unchanged() {
        assert_eq!(
            relay_http_base_url("wss://relay.example.com"),
            "https://relay.example.com"
        );
    }

    #[test]
    fn loopback_ipv4_literal_unchanged() {
        assert_eq!(
            relay_http_base_url("ws://127.0.0.1:3000"),
            "http://127.0.0.1:3000"
        );
    }

    #[test]
    fn localhost_substring_host_unchanged() {
        assert_eq!(
            relay_http_base_url("ws://localhost.evil.com:3000"),
            "http://localhost.evil.com:3000"
        );
    }

    #[test]
    fn loopback_wss_localhost_preserves_authority() {
        assert_eq!(
            relay_http_base_url("wss://localhost:3000"),
            "https://localhost:3000"
        );
    }

    // ── classify_intercepted_response ────────────────────────────────────────

    #[test]
    fn intercepted_cloudflare_host_returns_some() {
        let result = classify_intercepted_response("sqprod.cloudflareaccess.com", "text/html");
        assert!(result.is_some());
        let msg = result.unwrap();
        assert!(
            msg.starts_with("relay unreachable:"),
            "should have unreachable prefix"
        );
        assert!(msg.contains("Cloudflare"), "should mention Cloudflare");
    }

    #[test]
    fn intercepted_cloudflare_apex_host_returns_some() {
        // The apex domain itself should also match.
        let result = classify_intercepted_response("cloudflareaccess.com", "application/json");
        assert!(result.is_some());
        let msg = result.unwrap();
        assert!(msg.starts_with("relay unreachable:"));
        assert!(msg.contains("Cloudflare"));
    }

    #[test]
    fn intercepted_non_cloudflare_html_returns_some() {
        let result =
            classify_intercepted_response("proxy.corporate.example", "text/html; charset=utf-8");
        assert!(result.is_some());
        let msg = result.unwrap();
        assert!(msg.starts_with("relay unreachable:"));
    }

    #[test]
    fn normal_relay_json_returns_none() {
        let result = classify_intercepted_response("relay.myapp.example.com", "application/json");
        assert!(result.is_none());
    }

    #[test]
    fn content_type_case_insensitive() {
        // Uppercase content-type must still be detected.
        let result = classify_intercepted_response("proxy.example.com", "TEXT/HTML");
        assert!(result.is_some());
        assert!(result.unwrap().starts_with("relay unreachable:"));
    }

    #[test]
    fn evil_suffix_does_not_match_cloudflare() {
        // A host whose suffix happens to contain the Cloudflare string but is
        // not actually a subdomain must NOT match.
        let result = classify_intercepted_response(
            "notcloudflareaccess.com.evil.example",
            "application/json",
        );
        assert!(
            result.is_none(),
            "false suffix match should not trigger Cloudflare branch"
        );
    }

    // classify_request_error requires a real reqwest::Error (not publicly
    // constructable) — tested indirectly through integration; skipped here.

    // ── parse_json_response malformed-body contract ──────────────────────────

    #[test]
    fn malformed_response_message_stays_off_unreachable_bucket() {
        // A reached-but-malformed 2xx body is not a connectivity failure. If this
        // message ever regains the "relay unreachable:" prefix, the frontend
        // classifier would misroute it as unreachable — pin that it never does.
        assert!(
            !MALFORMED_RESPONSE_MESSAGE.starts_with("relay unreachable:"),
            "malformed-response message must not match the unreachable prefix"
        );
    }

    // ── parse_command_response ───────────────────────────────────────────────

    #[derive(Debug, Deserialize, PartialEq)]
    struct ChannelCreated {
        channel_id: String,
    }

    #[test]
    fn parse_command_response_decodes_typed_payload() {
        let msg = r#"response:{"channel_id":"abc123"}"#;
        let parsed: ChannelCreated = parse_command_response(msg).expect("should parse");
        assert_eq!(
            parsed,
            ChannelCreated {
                channel_id: "abc123".to_string()
            }
        );
    }

    #[test]
    fn parse_command_response_accepts_raw_json_fallback() {
        // Backward-compat: relays that emit raw JSON (no prefix) still work.
        let msg = r#"{"channel_id":"abc"}"#;
        let parsed: ChannelCreated = parse_command_response(msg).expect("fallback parse");
        assert_eq!(
            parsed,
            ChannelCreated {
                channel_id: "abc".to_string()
            }
        );
    }

    #[test]
    fn parse_command_response_rejects_invalid_prefixed_json() {
        let msg = "response:not-json";
        let result: Result<ChannelCreated, _> = parse_command_response(msg);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("response parse failed"));
    }

    #[test]
    fn parse_command_response_rejects_garbage() {
        let msg = "totally not json or response";
        let result: Result<ChannelCreated, _> = parse_command_response(msg);
        assert!(result.is_err());
    }

    // ── build_profile_event ──────────────────────────────────────────────────

    /// Generate a valid NIP-OA auth tag JSON string signed by a fresh owner key
    /// and addressed to `agent_keys`.
    ///
    /// Uses `nostr_compat` (nostr 0.36) for the owner keys because
    /// `buzz_sdk_pkg::nip_oa::compute_auth_tag` expects nostr 0.36 types.
    /// The agent pubkey is bridged via hex encoding.
    fn make_valid_auth_tag(agent_keys: &nostr::Keys) -> String {
        let owner_keys = nostr::Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        let agent_compat_pubkey =
            nostr::PublicKey::from_hex(&agent_pubkey_hex).expect("valid hex pubkey should parse");
        buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_keys, &agent_compat_pubkey, "")
            .expect("compute_auth_tag should not fail with distinct keys")
    }

    #[test]
    fn profile_event_with_valid_auth_tag() {
        let agent_keys = nostr::Keys::generate();
        let tag_json = make_valid_auth_tag(&agent_keys);
        let event = build_profile_event(&agent_keys, "TestBot", None, Some(&tag_json))
            .expect("should succeed with a valid auth tag");

        // Exactly one "auth" tag must be present.
        let auth_tags: Vec<_> = event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"))
            .collect();
        assert_eq!(auth_tags.len(), 1, "expected exactly 1 auth tag");

        // Must be a kind:0 (Metadata) event.
        assert_eq!(event.kind, nostr::Kind::Metadata);
    }

    #[test]
    fn profile_event_without_auth_tag() {
        let agent_keys = nostr::Keys::generate();
        let event = build_profile_event(&agent_keys, "TestBot", None, None)
            .expect("should succeed without an auth tag");

        // No "auth" tags should be present.
        let auth_tags: Vec<_> = event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"))
            .collect();
        assert_eq!(auth_tags.len(), 0, "expected no auth tags");

        assert_eq!(event.kind, nostr::Kind::Metadata);
    }

    #[test]
    fn profile_event_rejects_invalid_auth_tag() {
        let agent_keys = nostr::Keys::generate();
        // Structurally valid JSON array but with a bogus signature — verification must fail.
        let bad_json = format!(r#"["auth","{}","","{}"]"#, "a".repeat(64), "b".repeat(128));
        let result = build_profile_event(&agent_keys, "TestBot", None, Some(&bad_json));
        assert!(result.is_err(), "should reject an invalid auth tag");
        assert!(
            result.unwrap_err().contains("verification failed"),
            "error message should mention verification failure"
        );
    }
}
