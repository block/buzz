use std::time::Duration;

use reqwest::Method;
use serde::de::DeserializeOwned;

use crate::app_state::AppState;

use super::{
    build_authenticated_relay_request, build_nip98_auth_header, classify_request_error,
    parse_json_response, relay_api_base_url_with_override, relay_error_message,
};

/// Execute an authenticated JSON POST against the active relay and decode its JSON body.
pub async fn post_relay_json<T: DeserializeOwned>(
    state: &AppState,
    path: &str,
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<T, String> {
    if !path.starts_with('/') {
        return Err("relay POST path must begin with '/'".to_string());
    }
    crate::relay_admission::wait_for_rate_limit().await;
    let url = format!("{}{}", relay_api_base_url_with_override(state), path);
    let body_bytes =
        serde_json::to_vec(body).map_err(|error| format!("body serialization failed: {error}"))?;
    let auth = build_nip98_auth_header(&Method::POST, &url, &body_bytes, state)?;
    let response = build_authenticated_relay_request(
        &state.http_client,
        Method::POST,
        &url,
        &auth,
        Some(body_bytes),
        None,
        Some(timeout),
    )
    .send()
    .await
    .map_err(|error| classify_request_error(&error))?;
    if !response.status().is_success() {
        return Err(relay_error_message(response).await);
    }
    parse_json_response(response).await
}
