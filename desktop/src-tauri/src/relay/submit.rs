use super::*;

/// Response from `POST /events`.
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct SubmitEventResponse {
    pub event_id: String,
    pub accepted: bool,
    pub message: String,
}

/// Build an `EventBuilder` from the events module, sign it with the user's keys,
/// and POST the signed event to `/events` with NIP-98 auth.
pub async fn submit_event_at(
    builder: nostr::EventBuilder,
    state: &AppState,
    api_base_url: &str,
) -> Result<SubmitEventResponse, String> {
    crate::relay_admission::wait_for_rate_limit().await;
    // All synchronous work (signing) must complete before any .await
    // so the MutexGuard is dropped and the future remains Send.
    let url = format!("{}/events", api_base_url.trim_end_matches('/'));
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

/// Build, sign, and submit an event to the currently active workspace relay.
pub async fn submit_event(
    builder: nostr::EventBuilder,
    state: &AppState,
) -> Result<SubmitEventResponse, String> {
    let api_base_url = relay_api_base_url_with_override(state);
    submit_event_at(builder, state, &api_base_url).await
}
