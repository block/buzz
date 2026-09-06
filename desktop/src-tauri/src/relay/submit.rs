use super::*;

/// Every attempt gets a 30-second request deadline; an ambiguous timeout may
/// therefore take up to 60 seconds before failure. The shared client remains
/// unbounded for long-running model and media work.
const EVENT_SUBMIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

async fn send_event_http_request_once(
    http_client: &reqwest::Client,
    url: &str,
    auth_header: &str,
    auth_tag: Option<&str>,
    body_bytes: &[u8],
    timeout: std::time::Duration,
) -> Result<reqwest::Response, String> {
    let mut request = http_client
        .post(url)
        .header("Authorization", auth_header)
        .header("Content-Type", "application/json")
        .timeout(timeout);
    if let Some(tag) = auth_tag {
        request = request.header("x-auth-tag", tag);
    }
    request
        .body(body_bytes.to_vec())
        .send()
        .await
        .map_err(|error| classify_request_error(&error))
}

const EVENT_SUBMIT_MAX_ATTEMPTS: usize = 2;

fn is_event_submit_timeout(error: &str) -> bool {
    error == "relay unreachable: request timed out"
}

/// Send an already-signed event, retrying the identical bytes once if the
/// request times out before response headers arrive.
///
/// This headers-only variant preserves callers that intentionally treat any
/// successful HTTP status as completion. NIP-98 auth is rebuilt for each
/// attempt because each auth event is single-use. A non-success status body is
/// deliberately left to the caller so rate-limit handling stays unchanged.
pub(crate) async fn send_event_http_request_with_keys(
    http_client: &reqwest::Client,
    url: &str,
    keys: &Keys,
    auth_tag: Option<&str>,
    body_bytes: &[u8],
) -> Result<reqwest::Response, EventSubmitHttpError> {
    send_event_http_request_with_keys_and_timeout(
        http_client,
        url,
        keys,
        auth_tag,
        body_bytes,
        EVENT_SUBMIT_TIMEOUT,
    )
    .await
}

/// Fully consume one event-submit response, retrying the exact signed event on
/// an ambiguous timeout. Keeping this generic lets JSON and legacy text callers
/// share the retry boundary while preserving their established parse errors.
async fn consume_event_http_response_with_keys<T, Consume, ResponseFuture, ClassifyResponse>(
    http_client: &reqwest::Client,
    url: &str,
    keys: &Keys,
    auth_tag: Option<&str>,
    body_bytes: &[u8],
    consume_response: Consume,
    classify_response_error: ClassifyResponse,
) -> Result<T, EventSubmitHttpError>
where
    Consume: FnMut(reqwest::Response) -> ResponseFuture,
    ResponseFuture: std::future::Future<Output = Result<T, String>>,
    ClassifyResponse: Fn(String) -> EventSubmitHttpError,
{
    submit_event_response_with_keys_and_timeout(
        EventSubmitRequest {
            http_client,
            url,
            keys,
            auth_tag,
            body_bytes,
            timeout: EVENT_SUBMIT_TIMEOUT,
        },
        consume_response,
        classify_response_error,
    )
    .await
}

async fn send_event_http_request_with_keys_and_timeout(
    http_client: &reqwest::Client,
    url: &str,
    keys: &Keys,
    auth_tag: Option<&str>,
    body_bytes: &[u8],
    timeout: std::time::Duration,
) -> Result<reqwest::Response, EventSubmitHttpError> {
    let mut last_timeout = None;

    for _ in 0..EVENT_SUBMIT_MAX_ATTEMPTS {
        let auth_header = build_nip98_auth_header_for_keys(keys, &Method::POST, url, body_bytes)
            .map_err(EventSubmitHttpError::Auth)?;
        match send_event_http_request_once(
            http_client,
            url,
            &auth_header,
            auth_tag,
            body_bytes,
            timeout,
        )
        .await
        {
            Ok(response) => return Ok(response),
            Err(error) if is_event_submit_timeout(&error) => {
                last_timeout = Some(EventSubmitHttpError::Request(error));
            }
            Err(error) => return Err(EventSubmitHttpError::Request(error)),
        }
    }

    Err(last_timeout.unwrap_or_else(|| {
        EventSubmitHttpError::Request("relay unreachable: request timed out".to_string())
    }))
}

/// Which phase of a fully-consumed event submission failed.
///
/// Snapshot callers retain distinct prefixes for relay rejections while
/// leaving transport and successful-response decode failures unwrapped.
#[derive(Debug)]
pub(crate) enum EventSubmitHttpError {
    Auth(String),
    Request(String),
    Rejected(String),
    Response(String),
}

impl EventSubmitHttpError {
    fn is_timeout(&self) -> bool {
        is_event_submit_timeout(self.message())
    }

    fn message(&self) -> &str {
        match self {
            Self::Auth(message)
            | Self::Request(message)
            | Self::Rejected(message)
            | Self::Response(message) => message,
        }
    }

    pub(crate) fn into_message(self) -> String {
        match self {
            Self::Auth(message)
            | Self::Request(message)
            | Self::Rejected(message)
            | Self::Response(message) => message,
        }
    }
}

#[derive(Clone, Copy)]
struct EventSubmitRequest<'a> {
    http_client: &'a reqwest::Client,
    url: &'a str,
    keys: &'a Keys,
    auth_tag: Option<&'a str>,
    body_bytes: &'a [u8],
    timeout: std::time::Duration,
}

/// Submit one already-signed event and fully consume the relay response.
///
/// A timeout is ambiguous because relay ingest may complete before either the
/// headers or body reach Desktop. Rebuild NIP-98 request auth and retry the
/// exact same serialized event once. Ordinary event ingest treats a repeated
/// event ID as accepted without another insert or dispatch, so this reconciles
/// an accepted-first-attempt/body-stall without re-signing the user action
/// under a second ID.
async fn submit_event_response_with_keys_and_timeout<T, Consume, ResponseFuture, ClassifyResponse>(
    request: EventSubmitRequest<'_>,
    mut consume_response: Consume,
    classify_response_error: ClassifyResponse,
) -> Result<T, EventSubmitHttpError>
where
    Consume: FnMut(reqwest::Response) -> ResponseFuture,
    ResponseFuture: std::future::Future<Output = Result<T, String>>,
    ClassifyResponse: Fn(String) -> EventSubmitHttpError,
{
    let mut last_timeout = None;

    for _ in 0..EVENT_SUBMIT_MAX_ATTEMPTS {
        let auth_header = build_nip98_auth_header_for_keys(
            request.keys,
            &Method::POST,
            request.url,
            request.body_bytes,
        )
        .map_err(EventSubmitHttpError::Auth)?;
        let response = match send_event_http_request_once(
            request.http_client,
            request.url,
            &auth_header,
            request.auth_tag,
            request.body_bytes,
            request.timeout,
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                let error = EventSubmitHttpError::Request(error);
                if error.is_timeout() {
                    last_timeout = Some(error);
                    continue;
                }
                return Err(error);
            }
        };

        if !response.status().is_success() {
            // The status line makes this a definite rejection, even if reading
            // its detail body later times out. Only ambiguous outcomes retry.
            return Err(EventSubmitHttpError::Rejected(
                relay_error_message(response).await,
            ));
        }

        match consume_response(response).await {
            Ok(result) => return Ok(result),
            Err(error) => {
                let error = classify_response_error(error);
                if error.is_timeout() {
                    last_timeout = Some(error);
                    continue;
                }
                return Err(error);
            }
        }
    }

    Err(last_timeout.unwrap_or_else(|| {
        EventSubmitHttpError::Request("relay unreachable: request timed out".to_string())
    }))
}

pub(crate) async fn submit_event_json_with_keys<T: DeserializeOwned>(
    http_client: &reqwest::Client,
    url: &str,
    keys: &Keys,
    auth_tag: Option<&str>,
    body_bytes: &[u8],
) -> Result<T, String> {
    consume_event_http_response_with_keys(
        http_client,
        url,
        keys,
        auth_tag,
        body_bytes,
        parse_json_response::<T>,
        EventSubmitHttpError::Response,
    )
    .await
    .map_err(EventSubmitHttpError::into_message)
}

/// Submit an event and return its successful response body as text.
///
/// This variant preserves the snapshot import paths' established non-timeout
/// body-read detail while keeping the body inside the idempotent retry boundary.
pub(crate) async fn submit_event_text_with_keys(
    http_client: &reqwest::Client,
    url: &str,
    keys: &Keys,
    auth_tag: Option<&str>,
    body_bytes: &[u8],
) -> Result<String, EventSubmitHttpError> {
    consume_event_http_response_with_keys(
        http_client,
        url,
        keys,
        auth_tag,
        body_bytes,
        |response| async move {
            crate::commands::engram_submit_response::read_engram_submit_response(response).await
        },
        EventSubmitHttpError::Response,
    )
    .await
}

#[cfg(test)]
pub(super) async fn send_event_http_request_for_test(
    http_client: &reqwest::Client,
    url: &str,
    body_bytes: Vec<u8>,
    timeout: std::time::Duration,
) -> Result<reqwest::Response, String> {
    send_event_http_request_once(
        http_client,
        url,
        "Nostr test-auth",
        None,
        &body_bytes,
        timeout,
    )
    .await
}

#[cfg(test)]
pub(super) async fn send_event_http_request_with_keys_for_test(
    http_client: &reqwest::Client,
    url: &str,
    keys: &Keys,
    body_bytes: &[u8],
    timeout: std::time::Duration,
) -> Result<reqwest::Response, String> {
    send_event_http_request_with_keys_and_timeout(http_client, url, keys, None, body_bytes, timeout)
        .await
        .map_err(EventSubmitHttpError::into_message)
}

#[cfg(test)]
pub(super) async fn submit_event_json_with_keys_for_test<T: DeserializeOwned>(
    http_client: &reqwest::Client,
    url: &str,
    keys: &Keys,
    body_bytes: &[u8],
    timeout: std::time::Duration,
) -> Result<T, String> {
    submit_event_response_with_keys_and_timeout(
        EventSubmitRequest {
            http_client,
            url,
            keys,
            auth_tag: None,
            body_bytes,
            timeout,
        },
        parse_json_response::<T>,
        EventSubmitHttpError::Response,
    )
    .await
    .map_err(EventSubmitHttpError::into_message)
}

#[cfg(test)]
pub(super) async fn submit_event_text_with_keys_for_test(
    http_client: &reqwest::Client,
    url: &str,
    keys: &Keys,
    body_bytes: &[u8],
    timeout: std::time::Duration,
) -> Result<String, EventSubmitHttpError> {
    submit_event_response_with_keys_and_timeout(
        EventSubmitRequest {
            http_client,
            url,
            keys,
            auth_tag: None,
            body_bytes,
            timeout,
        },
        |response| async move {
            crate::commands::engram_submit_response::read_engram_submit_response(response).await
        },
        EventSubmitHttpError::Response,
    )
    .await
}

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
    let result: SubmitEventResponse =
        submit_event_json_with_keys(&state.http_client, &url, keys, None, &body_bytes).await?;
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
