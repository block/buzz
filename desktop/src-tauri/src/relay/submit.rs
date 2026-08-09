use super::*;

/// Stable publication classification for exact signed audit events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignedEventSubmitError {
    /// No authoritative relay decision was received; exact-ID retry is safe.
    Transient,
    /// The relay authoritatively rejected the immutable signed event.
    Permanent,
}

fn signed_event_submit_error(status: reqwest::StatusCode) -> SignedEventSubmitError {
    if status.is_server_error()
        || matches!(
            status,
            reqwest::StatusCode::REQUEST_TIMEOUT
                | reqwest::StatusCode::TOO_EARLY
                | reqwest::StatusCode::TOO_MANY_REQUESTS
        )
    {
        SignedEventSubmitError::Transient
    } else {
        SignedEventSubmitError::Permanent
    }
}

/// Response from `POST /events`.
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct SubmitEventResponse {
    pub event_id: String,
    pub accepted: bool,
    pub message: String,
}

/// Submit an immutable signed audit event with retry-safe error classification.
pub async fn submit_signed_event_classified(
    event: &nostr::Event,
    state: &AppState,
) -> Result<SubmitEventResponse, SignedEventSubmitError> {
    crate::relay_admission::wait_for_rate_limit().await;
    let url = format!("{}/events", relay_api_base_url_with_override(state));
    let body_bytes = event.as_json().into_bytes();
    crate::egress_guard::assert_no_key_backup_bytes(&body_bytes, "classified relay event submit")
        .map_err(|_| SignedEventSubmitError::Permanent)?;
    let auth_header = {
        let keys = state
            .signing_keys()
            .map_err(|_| SignedEventSubmitError::Transient)?;
        build_nip98_auth_header_for_keys(&keys, &Method::POST, &url, &body_bytes)
            .map_err(|_| SignedEventSubmitError::Transient)?
    };
    let response = state
        .http_client
        .post(&url)
        .header("Authorization", auth_header)
        .header("Content-Type", "application/json")
        .body(body_bytes)
        .send()
        .await
        .map_err(|_| SignedEventSubmitError::Transient)?;
    if !response.status().is_success() {
        return Err(signed_event_submit_error(response.status()));
    }
    let result: SubmitEventResponse = parse_json_response(response)
        .await
        .map_err(|_| SignedEventSubmitError::Transient)?;
    if result.event_id != event.id.to_hex() {
        return Err(SignedEventSubmitError::Transient);
    }
    if !result.accepted && !result.message.starts_with("duplicate:") {
        return Err(SignedEventSubmitError::Permanent);
    }
    Ok(result)
}

/// POST an already-signed event to an explicit relay with an explicit owner.
///
/// Deferred/scoped publication uses this form so a workspace or identity
/// switch cannot retarget either the event or its NIP-98 authentication after
/// the operation captured its `(relay, owner)` scope.
pub async fn submit_signed_event_at_with_keys(
    event: &nostr::Event,
    state: &AppState,
    api_base_url: &str,
    keys: &nostr::Keys,
) -> Result<SubmitEventResponse, String> {
    if event.pubkey != keys.public_key() {
        return Err("signed event does not match the publishing identity".to_string());
    }
    crate::relay_admission::wait_for_rate_limit().await;
    let url = format!("{}/events", api_base_url.trim_end_matches('/'));
    let body_bytes = event.as_json().into_bytes();
    crate::egress_guard::assert_no_key_backup_bytes(&body_bytes, "relay event submit")?;
    let auth_header = build_nip98_auth_header_for_keys(keys, &Method::POST, &url, &body_bytes)?;

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

/// Sign with an explicit identity and POST the event to an explicit relay.
///
/// The caller owns the signer lifetime. This is important for deferred work:
/// an in-process identity swap cannot retarget the event or its NIP-98 auth
/// after the caller has validated which identity the operation belongs to.
pub async fn submit_event_at_with_keys(
    builder: nostr::EventBuilder,
    state: &AppState,
    api_base_url: &str,
    keys: &nostr::Keys,
) -> Result<SubmitEventResponse, String> {
    let event = builder
        .sign_with_keys(keys)
        .map_err(|e| format!("failed to sign event: {e}"))?;
    submit_signed_event_at_with_keys(&event, state, api_base_url, keys).await
}

/// Build and submit an event to the currently active workspace relay.
pub async fn submit_event(
    builder: nostr::EventBuilder,
    state: &AppState,
) -> Result<SubmitEventResponse, String> {
    let api_base_url = relay_api_base_url_with_override(state);
    let keys = state.signing_keys()?;
    submit_event_at_with_keys(builder, state, &api_base_url, &keys).await
}

#[cfg(test)]
mod tests {
    use super::{signed_event_submit_error, SignedEventSubmitError};

    #[test]
    fn exact_signed_event_statuses_distinguish_retryable_from_permanent_rejection() {
        for status in [
            reqwest::StatusCode::REQUEST_TIMEOUT,
            reqwest::StatusCode::TOO_EARLY,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert_eq!(
                signed_event_submit_error(status),
                SignedEventSubmitError::Transient
            );
        }
        for status in [
            reqwest::StatusCode::BAD_REQUEST,
            reqwest::StatusCode::UNAUTHORIZED,
            reqwest::StatusCode::FORBIDDEN,
        ] {
            assert_eq!(
                signed_event_submit_error(status),
                SignedEventSubmitError::Permanent
            );
        }
    }

    #[tokio::test]
    async fn classified_submit_rejects_key_backup_material_before_network() {
        const NCRYPTSEC: &str = "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p";
        let state = crate::app_state::build_app_state();
        *state.relay_url_override.lock().unwrap() = Some("ws://127.0.0.1:9".to_string());
        let keys = nostr::Keys::generate();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), NCRYPTSEC)
            .sign_with_keys(&keys)
            .unwrap();

        assert!(matches!(
            super::submit_signed_event_classified(&event, &state).await,
            Err(SignedEventSubmitError::Permanent)
        ));
    }
}
