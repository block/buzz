use super::*;

/// Response from `POST /events`.
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct SubmitEventResponse {
    pub event_id: String,
    pub accepted: bool,
    pub message: String,
}

/// POST an already-signed event to an explicit relay with an explicit owner.
///
/// Deferred/scoped publication uses this form so a workspace or identity
/// switch cannot retarget either the event or its NIP-98 authentication after
/// the operation captured its `(relay, owner)` scope.
pub(crate) async fn submit_signed_event_at_with_signer(
    event: &nostr::Event,
    state: &AppState,
    api_base_url: &str,
    signer: &ActiveUserSigner,
) -> Result<SubmitEventResponse, String> {
    signer
        .run(async {
            if event.pubkey != signer.public_key() {
                return Err("signed event does not match the publishing identity".to_string());
            }
            crate::relay_admission::wait_for_rate_limit().await;
            let url = format!("{}/events", api_base_url.trim_end_matches('/'));
            let body_bytes = event.as_json().into_bytes();
            crate::egress_guard::assert_no_key_backup_bytes(&body_bytes, "relay event submit")?;
            let auth_header =
                build_nip98_auth_header_for_signer(signer, &Method::POST, &url, &body_bytes)
                    .await?;

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
        })
        .await
}

/// Sign with an explicit identity and POST the event to an explicit relay.
///
/// The caller owns the signer lifetime. This is important for deferred work:
/// an in-process identity swap cannot retarget the event or its NIP-98 auth
/// after the caller has validated which identity the operation belongs to.
pub(crate) async fn submit_event_at_with_signer(
    builder: nostr::EventBuilder,
    state: &AppState,
    api_base_url: &str,
    signer: &ActiveUserSigner,
) -> Result<SubmitEventResponse, String> {
    let event = signer
        .sign_event(builder)
        .await
        .map_err(|e| format!("failed to sign event: {e}"))?;
    submit_signed_event_at_with_signer(&event, state, api_base_url, signer).await
}

/// Build and submit an event to the currently active workspace relay.
pub async fn submit_event(
    builder: nostr::EventBuilder,
    state: &AppState,
) -> Result<SubmitEventResponse, String> {
    let api_base_url = relay_api_base_url_with_override(state);
    let signer = state.active_signer()?;
    submit_event_at_with_signer(builder, state, &api_base_url, &signer).await
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
    signer: &ActiveUserSigner,
) -> Result<(SubmitEventResponse, i64), String> {
    let event = signer
        .sign_event(builder)
        .await
        .map_err(|e| format!("failed to sign event: {e}"))?;
    let created_at = event.created_at.as_secs() as i64;
    let result = submit_signed_event_at_with_signer(&event, state, api_base_url, signer).await?;
    Ok((result, created_at))
}

/// Submit an independently selected signer and return the signed event's
/// `created_at` — same cursor rationale as [`submit_event_at_created_at`].
pub async fn submit_event_with_signer_created_at(
    builder: nostr::EventBuilder,
    state: &AppState,
    relay_base: &str,
    signer: &ActiveUserSigner,
    auth_tag: Option<&str>,
) -> Result<(SubmitEventResponse, i64), String> {
    let event = signer
        .sign_event(builder)
        .await
        .map_err(|e| format!("failed to sign event: {e}"))?;
    let created_at = event.created_at.as_secs() as i64;
    let result =
        super::submit_signed_event_at_with_auth(&event, state, relay_base, signer, auth_tag)
            .await?;
    Ok((result, created_at))
}

// Identical captured-owner policy for live and generic event callers.
pub(crate) use submit_event_at_with_signer as submit_event_at;
