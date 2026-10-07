use reqwest::Method;
use serde::de::DeserializeOwned;

use crate::app_state::AppState;

use super::{
    build_nip98_auth_header, classify_request_error, parse_json_response,
    relay_api_base_url_with_override, relay_error_message,
};

/// Execute an authenticated GET against the active relay and decode its JSON body.
pub async fn get_relay_json<T: DeserializeOwned>(
    state: &AppState,
    path_with_query: &str,
) -> Result<T, String> {
    if !path_with_query.starts_with('/') {
        return Err("relay GET path must begin with '/'".to_string());
    }
    crate::relay_admission::wait_for_rate_limit().await;
    let url = format!(
        "{}{}",
        relay_api_base_url_with_override(state),
        path_with_query
    );
    let auth = build_nip98_auth_header(&Method::GET, &url, &[], state)?;
    let response = state
        .http_client
        .get(&url)
        .header("Authorization", auth)
        .send()
        .await
        .map_err(|error| classify_request_error(&error))?;
    if !response.status().is_success() {
        return Err(relay_error_message(response).await);
    }
    parse_json_response(response).await
}
