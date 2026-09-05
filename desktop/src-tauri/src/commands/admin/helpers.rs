//! HTTP helpers for the desktop admin surface.
//!
//! NIP-98 authenticated fetch/mutation wrappers and response-reading utilities
//! used by the Tauri command implementations in `mod.rs`.

use super::client;
use super::{AdminMutationError, ATTACHMENT_CAP, ERROR_BODY_CAP};

/// Fetch a JSON endpoint with NIP-98 auth, one 401-retry, and a size cap.
pub(super) async fn fetch_admin_json(
    url: &str,
    cap: u64,
    state: &tauri::State<'_, crate::app_state::AppState>,
) -> Result<Vec<u8>, String> {
    use crate::relay::build_nip98_auth_header_for_keys;

    let keys = state.signing_keys()?;
    let http_client = client::ADMIN_CLIENT
        .get()
        .ok_or_else(|| "admin client not initialised".to_string())?;

    let auth_header = build_nip98_auth_header_for_keys(&keys, &reqwest::Method::GET, url, &[])
        .map_err(|e| format!("nip98 build failed: {e}"))?;

    let resp = http_client
        .get(url)
        .header(reqwest::header::AUTHORIZATION, &auth_header)
        .send()
        .await
        .map_err(|e| crate::relay::classify_request_error(&e))?;

    // One retry on 401 with a fresh NIP-98 event (new nonce).
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let auth_header2 = build_nip98_auth_header_for_keys(&keys, &reqwest::Method::GET, url, &[])
            .map_err(|e| format!("nip98 build failed on retry: {e}"))?;
        let resp2 = http_client
            .get(url)
            .header(reqwest::header::AUTHORIZATION, auth_header2)
            .send()
            .await
            .map_err(|e| crate::relay::classify_request_error(&e))?;
        return read_admin_response(resp2, cap, ERROR_BODY_CAP).await;
    }

    read_admin_response(resp, cap, ERROR_BODY_CAP).await
}

/// POST a JSON body with NIP-98 auth (payload sha256 in the tag), one 401-retry, size cap.
pub(super) async fn post_admin_json(
    url: &str,
    body: &[u8],
    cap: u64,
    state: &tauri::State<'_, crate::app_state::AppState>,
) -> Result<Vec<u8>, AdminMutationError> {
    mutation_admin_json(reqwest::Method::POST, url, body, cap, state).await
}

/// PATCH a JSON body with NIP-98 auth (payload sha256), one 401-retry, size cap.
pub(super) async fn patch_admin_json(
    url: &str,
    body: &[u8],
    cap: u64,
    state: &tauri::State<'_, crate::app_state::AppState>,
) -> Result<Vec<u8>, AdminMutationError> {
    mutation_admin_json(reqwest::Method::PATCH, url, body, cap, state).await
}

/// PUT a JSON body with NIP-98 auth (payload sha256), one 401-retry, size cap.
pub(super) async fn put_admin_json(
    url: &str,
    body: &[u8],
    cap: u64,
    state: &tauri::State<'_, crate::app_state::AppState>,
) -> Result<Vec<u8>, AdminMutationError> {
    mutation_admin_json(reqwest::Method::PUT, url, body, cap, state).await
}

/// DELETE with NIP-98 auth (no body), one 401-retry, size cap.
pub(super) async fn delete_admin_json(
    url: &str,
    cap: u64,
    state: &tauri::State<'_, crate::app_state::AppState>,
) -> Result<Vec<u8>, String> {
    use crate::relay::build_nip98_auth_header_for_keys;

    let keys = state.signing_keys()?;
    let http_client = client::ADMIN_CLIENT
        .get()
        .ok_or_else(|| "admin client not initialised".to_string())?;

    let auth_header = build_nip98_auth_header_for_keys(&keys, &reqwest::Method::DELETE, url, &[])
        .map_err(|e| format!("nip98 build failed: {e}"))?;

    let resp = http_client
        .delete(url)
        .header(reqwest::header::AUTHORIZATION, &auth_header)
        .send()
        .await
        .map_err(|e| crate::relay::classify_request_error(&e))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let auth_header2 =
            build_nip98_auth_header_for_keys(&keys, &reqwest::Method::DELETE, url, &[])
                .map_err(|e| format!("nip98 build failed on retry: {e}"))?;
        let resp2 = http_client
            .delete(url)
            .header(reqwest::header::AUTHORIZATION, auth_header2)
            .send()
            .await
            .map_err(|e| crate::relay::classify_request_error(&e))?;
        return read_admin_response(resp2, cap, ERROR_BODY_CAP).await;
    }

    read_admin_response(resp, cap, ERROR_BODY_CAP).await
}

/// Shared implementation for POST/PATCH/PUT with NIP-98 payload sha256 binding.
///
/// Returns a typed [`AdminMutationError`] so the caller can distinguish a
/// relay-authoritative failure (a status was received) from a transport or
/// pre-send failure (no relay answer). `?` on the `String`-producing steps
/// (auth build, `send()` classification) converts via `From<String>` to a
/// no-status error, which is correct: none of those reached a relay verdict.
pub(super) async fn mutation_admin_json(
    method: reqwest::Method,
    url: &str,
    body: &[u8],
    cap: u64,
    state: &tauri::State<'_, crate::app_state::AppState>,
) -> Result<Vec<u8>, AdminMutationError> {
    use crate::relay::build_nip98_auth_header_for_keys;

    let keys = state.signing_keys()?;
    let http_client = client::ADMIN_CLIENT
        .get()
        .ok_or_else(|| "admin client not initialised".to_string())?;

    // NIP-98 §4: for body-bearing requests, include a `payload` tag over the
    // SHA-256 of the exact request body bytes.
    let auth_header = build_nip98_auth_header_for_keys(&keys, &method, url, body)
        .map_err(|e| format!("nip98 build failed: {e}"))?;

    let send_request = |auth: String| {
        http_client
            .request(method.clone(), url)
            .header(reqwest::header::AUTHORIZATION, auth)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_vec())
            .send()
    };

    let resp = send_request(auth_header)
        .await
        .map_err(|e| crate::relay::classify_request_error(&e))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let auth_header2 = build_nip98_auth_header_for_keys(&keys, &method, url, body)
            .map_err(|e| format!("nip98 build failed on retry: {e}"))?;
        let resp2 = send_request(auth_header2)
            .await
            .map_err(|e| crate::relay::classify_request_error(&e))?;
        return read_admin_mutation_response(resp2, cap, ERROR_BODY_CAP).await;
    }

    read_admin_mutation_response(resp, cap, ERROR_BODY_CAP).await
}

/// Stream and validate an attachment response, enforcing Content-Type, size,
/// and the cap.
pub(super) async fn finish_attachment_response(
    resp: reqwest::Response,
    expected_mime: &str,
    expected_size: u64,
) -> Result<tauri::ipc::Response, String> {
    use futures_util::StreamExt;

    if resp.status().is_redirection() {
        return Err("admin_attachment_redirect".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!(
            "admin_attachment_relay_error_{}",
            resp.status().as_u16()
        ));
    }

    // Verify Content-Type before reading the body.
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if content_type != expected_mime.trim().to_ascii_lowercase() {
        return Err("admin_attachment_mime_mismatch".to_string());
    }

    // Content-Length preflight.
    if let Some(cl) = resp.content_length() {
        if cl > ATTACHMENT_CAP {
            return Err("admin_attachment_too_large".to_string());
        }
        if cl != expected_size {
            return Err("admin_attachment_size_mismatch".to_string());
        }
    }

    // Stream with running byte counter.
    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "admin_attachment_stream_error".to_string())?;
        if bytes.len() as u64 + chunk.len() as u64 > ATTACHMENT_CAP {
            return Err("admin_attachment_too_large".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }

    // Final size check.
    if bytes.len() as u64 != expected_size {
        return Err("admin_attachment_size_mismatch".to_string());
    }

    Ok(tauri::ipc::Response::new(bytes))
}

/// Read a response body up to `success_cap` bytes on 2xx, `error_cap` on
/// non-2xx. Redirects are treated as errors (the no-redirect client surfaced
/// them rather than following).
pub(super) async fn read_admin_response(
    resp: reqwest::Response,
    success_cap: u64,
    error_cap: u64,
) -> Result<Vec<u8>, String> {
    use futures_util::StreamExt;

    if resp.status().is_redirection() {
        return Err(format!(
            "admin API returned a {} redirect (not followed)",
            resp.status()
        ));
    }

    let (is_success, cap) = if resp.status().is_success() {
        (true, success_cap)
    } else {
        (false, error_cap)
    };

    if let Some(cl) = resp.content_length() {
        if cl > cap {
            return Err(format!(
                "admin response too large ({cl} bytes, cap {cap} bytes)"
            ));
        }
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("admin response stream error: {e}"))?;
        if bytes.len() as u64 + chunk.len() as u64 > cap {
            return Err(format!("admin response too large (cap {cap} bytes)"));
        }
        bytes.extend_from_slice(&chunk);
    }

    if !is_success {
        let body = String::from_utf8_lossy(&bytes);
        return Err(format!("admin API error: {body}"));
    }

    Ok(bytes)
}

/// Read a mutation response, preserving the relay's HTTP status in the error.
///
/// Mirrors [`read_admin_response`]'s size discipline and message wording so the
/// UI's message parsing is unchanged, but on a non-2xx it returns an
/// [`AdminMutationError`] tagged with the received status and whether the full
/// body was read. Only a status with a complete body (`authoritative`) is a
/// verdict the UI treats as definitive; a redirect, an over-cap body, or a
/// mid-stream read failure carries the status as `partial` — the relay answered
/// but the outcome is unknown, so the caller preserves the idempotency key and
/// lets the retry dedupe against any commit that landed.
async fn read_admin_mutation_response(
    resp: reqwest::Response,
    success_cap: u64,
    error_cap: u64,
) -> Result<Vec<u8>, AdminMutationError> {
    use futures_util::StreamExt;

    let status = resp.status();

    if status.is_redirection() {
        return Err(AdminMutationError::partial(
            status,
            format!("admin API returned a {status} redirect (not followed)"),
        ));
    }

    let (is_success, cap) = if status.is_success() {
        (true, success_cap)
    } else {
        (false, error_cap)
    };

    if let Some(cl) = resp.content_length() {
        if cl > cap {
            return Err(AdminMutationError::partial(
                status,
                format!("admin response too large ({cl} bytes, cap {cap} bytes)"),
            ));
        }
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            AdminMutationError::partial(status, format!("admin response stream error: {e}"))
        })?;
        if bytes.len() as u64 + chunk.len() as u64 > cap {
            return Err(AdminMutationError::partial(
                status,
                format!("admin response too large (cap {cap} bytes)"),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }

    if !is_success {
        let body = String::from_utf8_lossy(&bytes);
        return Err(AdminMutationError::authoritative(
            status,
            format!("admin API error: {body}"),
        ));
    }

    Ok(bytes)
}
