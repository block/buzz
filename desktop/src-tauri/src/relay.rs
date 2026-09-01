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

// Per-request deadline for the `POST /query` HTTP bridge, covering both the
// header exchange and full body consumption. The shared `http_client` sets no
// client-level timeout — deliberately, because it is also used for long-running
// STT/TTS model downloads, builderlab auth, and the media proxy — so a stalled
// or half-open `/query` connection would otherwise leave the request pending
// forever, hanging the caller (e.g. a thread-history load that never resolves
// and shows a permanent skeleton). A per-request timeout scoped to `/query`
// bounds that without affecting the client's other users. A timeout surfaces
// through `classify_request_error` as the stable `"relay unreachable: request
// timed out"` string. Set above the 25s WS history timeout so a slow-but-live
// relay is not cut off before the WebSocket path would be.
const QUERY_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

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
pub(crate) fn workspace_relay_override(state: &AppState) -> Option<String> {
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
/// Always the active workspace relay. The legacy per-record `relay_url` pin is
/// deliberately IGNORED (agents-everywhere, #2122): every agent is eligible on
/// every community, and the pair the caller is acting on is identified by the
/// workspace relay, never by a stored pin. The record field is still parsed
/// and persisted untouched — old records need no migration and a rollback to a
/// pin-honoring build reads the same file — so the parameter stays in the
/// signature as documentation of what is being ignored at the one choke point
/// all agent relay resolution flows through. Resolving at read-time also means
/// a stale stored value can never leak into reconcile, spawn, or profile sync.
/// Uniform for both Local and Provider backends.
pub fn effective_agent_relay_url(_record_relay: &str, workspace_relay: &str) -> String {
    workspace_relay.to_string()
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

mod scope;
pub use scope::{
    assert_expected_relay_scope, assert_expected_signer, bind_expected_relay_scope,
    bind_expected_signer, ScopedWorkspaceRelay,
};

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

/// Preserve a body-consumption timeout as the stable connectivity classification.
///
/// `send()` resolves once response headers arrive, so a body that stalls past
/// the request deadline trips the timeout during body consumption rather than
/// at `send()`. That is a connectivity failure, not a malformed body or a plain
/// status error. Both body-consumption paths — the 2xx `parse_json_response`
/// and the non-2xx `relay_error_message` — route their consumption error
/// through this one helper so a stalled body can never be classified as
/// "request timed out" on one path while the other buries it under a malformed
/// or status label. Returns `Some("relay unreachable: request timed out")` for
/// a timeout; `None` otherwise, leaving the caller to apply its own non-timeout
/// label.
fn classify_body_timeout(e: &reqwest::Error) -> Option<String> {
    e.is_timeout().then(|| classify_request_error(e))
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
    //
    // A body-consumption timeout is the exception: `send()` resolves once
    // headers arrive, so a body that stalls past the request deadline trips the
    // timeout HERE rather than at send(). That is a connectivity failure, not a
    // malformed body, so route it through `classify_body_timeout` — the same
    // helper the non-2xx error-body path uses — to preserve the stable
    // "relay unreachable: request timed out" label.
    response.json::<T>().await.map_err(|e| {
        classify_body_timeout(&e).unwrap_or_else(|| MALFORMED_RESPONSE_MESSAGE.to_string())
    })
}

/// Extract the `retry in Ns` hint from a rate-limit error string.
///
/// Matches the canonical format emitted by the relay in both HTTP 429 bodies
/// and CLOSED/NOTICE messages: `quota exceeded; retry in 4s`.
fn extract_retry_in_hint(body: &str) -> Option<u64> {
    let re_match = body.find("retry in ")?;
    let after = &body[re_match + "retry in ".len()..];
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u64>().ok()
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
    // `text()` consumes the body, which — like the 2xx path — can trip the
    // request deadline if the relay sends status headers then stalls the body.
    // Preserve that timeout as the stable connectivity classification via the
    // shared helper instead of letting `unwrap_or_default` swallow it into a
    // bare status label. A non-timeout body error still degrades to an empty
    // body → status-only message, exactly as before.
    let body = match response.text().await {
        Ok(body) => body,
        Err(e) => {
            if let Some(timeout) = classify_body_timeout(&e) {
                return timeout;
            }
            String::new()
        }
    };

    // 429 Too Many Requests → typed `relay rate-limited:` prefix so the TS
    // client can activate the rate-limit gate without confusing it with a
    // connectivity failure (`relay unreachable:`). Also arm the Rust-side
    // admission gate here — the one place every relay HTTP error funnels
    // through — so the next relay-backed command waits out the quota window
    // instead of burning it (see `relay_admission`).
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let hint = extract_retry_in_hint(&body);
        // Clamp the hint to MAX_HINT_SECONDS before arming the Rust gate AND
        // before embedding it in the returned string. Every consumer (Rust gate
        // via `activate_rate_limit` and TS gate via `applyTauriRateLimitIfNeeded`)
        // must see the same capped value — a single policy point prevents the TS
        // gate from receiving an uncapped hint from an untrusted relay.
        let capped_hint = hint.map(|s| s.min(crate::relay_admission::MAX_HINT_SECONDS));
        crate::relay_admission::activate_rate_limit(capped_hint);
        if let Some(secs) = capped_hint {
            return format!("relay rate-limited: retry in {secs}s");
        }
        return "relay rate-limited: quota exceeded".to_string();
    }

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
    crate::relay_admission::wait_for_rate_limit().await;
    let url = format!("{}/query", api_base_url);
    let body_bytes =
        serde_json::to_vec(filters).map_err(|e| format!("filter serialization failed: {e}"))?;
    let auth = build_nip98_auth_header(&Method::POST, &url, &body_bytes, state)?;
    send_query_request(
        &state.http_client,
        &url,
        &auth,
        None,
        body_bytes,
        QUERY_REQUEST_TIMEOUT,
    )
    .await
}

pub async fn query_relay_at_with_keys(
    state: &AppState,
    api_base_url: &str,
    filters: &[serde_json::Value],
    keys: &Keys,
    auth_tag: Option<&str>,
) -> Result<Vec<nostr::Event>, String> {
    crate::relay_admission::wait_for_rate_limit().await;
    let url = format!("{}/query", api_base_url);
    let body_bytes =
        serde_json::to_vec(filters).map_err(|e| format!("filter serialization failed: {e}"))?;
    let auth = build_nip98_auth_header_for_keys(keys, &Method::POST, &url, &body_bytes)?;
    send_query_request(
        &state.http_client,
        &url,
        &auth,
        auth_tag,
        body_bytes,
        QUERY_REQUEST_TIMEOUT,
    )
    .await
}

/// Issue an authenticated `POST /query` and parse the response, applying the
/// per-request `timeout` that bounds a stalled or half-open relay connection.
///
/// Both `/query` builders funnel through this one helper so the timeout can
/// never be applied to one builder and dropped from the other, and so a test
/// can drive the real send/timeout/classify path with a short deadline against
/// a stalled loopback. A timeout surfaces through `classify_request_error` as
/// the stable `"relay unreachable: request timed out"` string.
async fn send_query_request(
    http_client: &reqwest::Client,
    url: &str,
    auth: &str,
    auth_tag: Option<&str>,
    body_bytes: Vec<u8>,
    timeout: std::time::Duration,
) -> Result<Vec<nostr::Event>, String> {
    let mut request = http_client
        .post(url)
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .timeout(timeout);
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
    about: Option<&str>,
    auth_tag_json: Option<&str>,
) -> Result<nostr::Event, String> {
    let builder = crate::events::build_profile(Some(display_name), None, avatar_url, about, None)?;

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
/// API token is required. `about` carries the agent's authored public
/// description (see `managed_agents::record_effective_description`); the
/// relay treats kind:0
/// fields as absolute, so passing `None` clears any previously published about.
pub async fn sync_managed_agent_profile(
    state: &AppState,
    relay_url: &str,
    agent_keys: &nostr::Keys,
    display_name: &str,
    avatar_url: Option<&str>,
    about: Option<&str>,
    auth_tag: Option<&str>, // NIP-OA auth tag JSON
) -> Result<(), String> {
    crate::relay_admission::wait_for_rate_limit().await;
    // Build a signed kind:0 profile event (with optional NIP-OA auth tag).
    let event = build_profile_event(agent_keys, display_name, avatar_url, about, auth_tag)?;
    let event_json = event.as_json();
    let body_bytes = event_json.into_bytes();
    crate::egress_guard::assert_no_key_backup_bytes(&body_bytes, "agent profile sync")?;

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
            "Could not sync the agent's profile metadata: {msg}"
        ));
    }

    Ok(())
}

// ── Agent profile query ─────────────────────────────────────────────────────

/// Query the relay for an agent's kind:0 profile event.
///
/// Queries the relay identified by `relay_url`. Callers uniformly pass the
/// relay resolved by `effective_agent_relay_url` for every agent regardless of
/// backend — always the active workspace relay — so the query targets the host
/// the profile is actually published to.
///
/// Returns the parsed profile content (display_name, picture, about) if a
/// kind:0 event exists for the given pubkey, or `None` if no profile is
/// published.
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
        about: content
            .get("about")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }))
}

/// Parsed fields from a kind:0 profile event.
#[derive(Debug, Clone)]
pub struct AgentProfileInfo {
    pub display_name: Option<String>,
    pub picture: Option<String>,
    /// Published public description (kind:0 `about`).
    pub about: Option<String>,
}

// ── Signed-event submission ─────────────────────────────────────────────────

mod get;
pub use get::get_relay_json;

mod submit;
pub use submit::{
    submit_event, submit_event_at_created_at, submit_event_at_with_keys,
    submit_event_with_keys_created_at, submit_signed_event_at_with_keys, SubmitEventResponse,
};

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
    let event = builder
        .sign_with_keys(keys)
        .map_err(|e| format!("failed to sign event: {e}"))?;
    submit_signed_event_with_keys(&event, state, keys, auth_tag).await
}

/// POST an already-signed event using the same explicit identity for NIP-98.
pub async fn submit_signed_event_with_keys(
    event: &nostr::Event,
    state: &AppState,
    keys: &Keys,
    auth_tag: Option<&str>,
) -> Result<SubmitEventResponse, String> {
    if event.pubkey != keys.public_key() {
        return Err("signed event does not match the publishing identity".to_string());
    }
    crate::relay_admission::wait_for_rate_limit().await;
    let url = format!("{}/events", relay_api_base_url_with_override(state));
    let body_bytes = event.as_json().into_bytes();
    crate::egress_guard::assert_no_key_backup_bytes(&body_bytes, "signed event submit (keys)")?;
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
mod tests;
