use super::*;

/// Response from `POST /events`.
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct SubmitEventResponse {
    pub event_id: String,
    pub accepted: bool,
    pub message: String,
}

/// Why a signed-event submission did not complete, as far as the caller can
/// tell what the relay did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitFailure {
    /// The relay answered with an HTTP error. `message` is the relay's own
    /// text rendered for the UI (`relay returned <status>: <reason>`).
    Refused { status: u16, message: String },
    /// Transport failure, malformed answer, or a 2xx that did not accept the
    /// event: nothing is known about what the relay stored.
    Unknown(String),
}

impl SubmitFailure {
    /// The relay refused the event only because its `created_at` is outside
    /// the relay's freshness window. The relay checks the window before it
    /// stores anything, so such an event is certainly not stored.
    pub fn is_stale_timestamp(&self) -> bool {
        matches!(
            self,
            Self::Refused { status: 400, message }
                if message.contains(buzz_core_pkg::artifact::STALE_EVENT_TIMESTAMP_REJECTION)
        )
    }

    pub fn into_message(self) -> String {
        match self {
            Self::Refused { message, .. } | Self::Unknown(message) => message,
        }
    }
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
    submit_signed_event_classified(event, state, api_base_url, keys)
        .await
        .map_err(SubmitFailure::into_message)
}

/// [`submit_signed_event_at_with_keys`], but a failure says whether the relay
/// explicitly refused the event or nothing is known about what it did.
pub async fn submit_signed_event_classified(
    event: &nostr::Event,
    state: &AppState,
    api_base_url: &str,
    keys: &nostr::Keys,
) -> Result<SubmitEventResponse, SubmitFailure> {
    if event.pubkey != keys.public_key() {
        return Err(SubmitFailure::Unknown(
            "signed event does not match the publishing identity".to_string(),
        ));
    }
    crate::relay_admission::wait_for_rate_limit().await;
    let url = format!("{}/events", api_base_url.trim_end_matches('/'));
    let body_bytes = event.as_json().into_bytes();
    crate::egress_guard::assert_no_key_backup_bytes(&body_bytes, "relay event submit")
        .map_err(SubmitFailure::Unknown)?;
    let auth_header = build_nip98_auth_header_for_keys(keys, &Method::POST, &url, &body_bytes)
        .map_err(SubmitFailure::Unknown)?;

    let response = build_authenticated_relay_request(
        &state.http_client,
        Method::POST,
        &url,
        &auth_header,
        Some(body_bytes),
        None,
        None,
    )
    .send()
    .await
    .map_err(|e| SubmitFailure::Unknown(classify_request_error(&e)))?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        return Err(SubmitFailure::Refused {
            status,
            message: relay_error_message(response).await,
        });
    }

    let result: SubmitEventResponse = parse_json_response(response)
        .await
        .map_err(SubmitFailure::Unknown)?;
    if !result.accepted {
        return Err(SubmitFailure::Unknown(format!(
            "relay rejected event: {}",
            result.message
        )));
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

/// Sign with an explicit identity, submit to an explicit HTTP API base URL,
/// and also return the signed event's `created_at`.
///
/// Callers that persist a timestamp as an event cursor (e.g. the Projects
/// conversation opener) need the signed event's own second — a
/// post-publication clock read can land a second later and permanently
/// exclude other events stamped in the event's real second.
///
/// The explicit base (rather than a re-read of the workspace override at
/// submit time) matters for the same callers: they validated a tenant scope
/// against the resolved base earlier in the same command, and re-resolving
/// here would reopen the window where a workspace switch retargets the event
/// after the check passed. The explicit `keys` close the sibling window: the
/// relay URL and the signing keys mutate under separate locks during a
/// workspace switch, so re-reading the keys here could sign — and NIP-98
/// authenticate — the event as the *new* tenant's identity after the caller
/// validated the old one. The caller passes the exact snapshot it asserted.
pub async fn submit_event_at_created_at(
    builder: nostr::EventBuilder,
    state: &AppState,
    api_base_url: &str,
    keys: &nostr::Keys,
) -> Result<(SubmitEventResponse, i64), String> {
    let event = builder
        .sign_with_keys(keys)
        .map_err(|e| format!("failed to sign event: {e}"))?;
    let created_at = event.created_at.as_secs() as i64;
    let result = submit_signed_event_at_with_keys(&event, state, api_base_url, keys).await?;
    Ok((result, created_at))
}

/// Like `submit_event_with_keys`, but also returns the signed event's
/// `created_at` — same cursor rationale as [`submit_event_at_created_at`].
pub async fn submit_event_with_keys_created_at(
    builder: nostr::EventBuilder,
    state: &AppState,
    keys: &nostr::Keys,
    auth_tag: Option<&str>,
) -> Result<(SubmitEventResponse, i64), String> {
    let event = builder
        .sign_with_keys(keys)
        .map_err(|e| format!("failed to sign event: {e}"))?;
    let created_at = event.created_at.as_secs() as i64;
    let result = super::submit_signed_event_with_keys(&event, state, keys, auth_tag).await?;
    Ok((result, created_at))
}
