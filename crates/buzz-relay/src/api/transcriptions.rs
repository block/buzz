//! Relay-owned speech-to-text proxy.
//!
//! Mobile composers offer dictation, but an app cannot keep a provider
//! credential secret. This endpoint keeps the operator's key on the relay: an
//! authenticated community member posts one bounded M4A clip, the relay
//! forwards it to an OpenAI-compatible `audio/transcriptions` endpoint, and
//! only the transcript text comes back.
//!
//! Audio and transcripts are never logged or stored. Upstream failures map to
//! generic errors; provider bodies never cross the relay boundary.
//!
//! The body is raw audio rather than a Nostr event for the same reason Blossom
//! uploads are: it is binary media with no place in a signed event.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{DefaultBodyLimit, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::post,
    Extension, Router,
};
use serde_json::Value;

use crate::config::TranscriptionConfig;
use crate::state::AppState;

use buzz_auth::LimitType;

use super::{api_error, bridge, read_capped, CappedReadError};

/// NIP-11 `supported_extensions` entry for relay-proxied transcription.
pub(crate) const EXTENSION: &str = "buzz-transcription";
/// Relay-relative transcription endpoint.
pub(crate) const TRANSCRIPTIONS_PATH: &str = "/audio/transcriptions";
/// Largest accepted audio body: about 7 minutes of the mobile recorder's
/// AAC-LC (96 kbps, 44.1 kHz mono M4A), which stops at 5 minutes (about
/// 3.5 MiB). Each in-flight request holds the body about three times
/// (request, multipart copy, send buffer), so the cap stays small.
pub const MAX_AUDIO_BYTES: usize = 5 * 1024 * 1024;
/// The one accepted request `Content-Type`: the mobile recorder's M4A.
const AUDIO_CONTENT_TYPE: &str = "audio/mp4";
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_UPSTREAM_RESPONSE_BYTES: usize = 256 * 1024;

type ApiError = (StatusCode, Json<Value>);

/// The transcription route, or `None` when the operator has not configured
/// it: then the path is simply unrouted. It carries its own audio-sized body
/// limit because the shared API router caps bodies at 1 MB.
pub(crate) fn router(state: Arc<AppState>) -> Option<Router> {
    let config = Arc::new(state.config.transcription.clone()?);
    Some(
        Router::new()
            .route(TRANSCRIPTIONS_PATH, post(transcribe))
            .route_layer(middleware::from_fn(require_nostr_proof))
            .layer(DefaultBodyLimit::max(MAX_AUDIO_BYTES))
            .layer(Extension(config))
            .with_state(state),
    )
}

/// Refuses a request carrying no `Authorization: Nostr …` proof before its
/// body is buffered, so only a caller presenting a proof can make the relay
/// hold up to [`MAX_AUDIO_BYTES`] before admission. The test is the one
/// `bridge::check_bridge_proof` applies, and the refusal is the 401 and
/// access-log outcome it gives a missing proof; admission verifies the rest.
async fn require_nostr_proof(request: Request, next: Next) -> Response {
    let has_proof = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("Nostr "));
    if has_proof {
        return next.run(request).await;
    }
    let slot = request
        .extensions()
        .get::<crate::access::AccessSlot>()
        .cloned()
        .unwrap_or_default();
    slot.attempt(crate::access::AuthMethod::Nip98, None, false);
    slot.finish(crate::access::AuthResult::Failed("missing_auth"));
    api_error(StatusCode::UNAUTHORIZED, "missing Nostr auth").into_response()
}

fn is_m4a(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|essence| essence.trim().eq_ignore_ascii_case(AUDIO_CONTENT_TYPE))
}

/// A `multipart/form-data` body for the OpenAI-compatible endpoint: model,
/// JSON response format and the M4A file.
fn multipart_body(boundary: &str, model: &str, audio: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(audio.len() + 512);
    for (name, value) in [("model", model), ("response_format", "json")] {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
             filename=\"audio.m4a\"\r\nContent-Type: {AUDIO_CONTENT_TYPE}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(audio);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// Maps a transport failure, while sending or while reading the response,
/// to a generic error. Only whether it timed out is logged.
fn upstream_error(error: reqwest::Error) -> ApiError {
    tracing::warn!(
        timeout = error.is_timeout(),
        "transcription upstream request failed"
    );
    if error.is_timeout() {
        api_error(
            StatusCode::GATEWAY_TIMEOUT,
            "transcription provider timed out",
        )
    } else {
        api_error(
            StatusCode::BAD_GATEWAY,
            "transcription provider is unavailable",
        )
    }
}

/// Sends one clip upstream and returns only its transcript text. The
/// per-request timeout overrides the shared client's default.
async fn request_transcript(
    client: &reqwest::Client,
    config: &TranscriptionConfig,
    audio: &[u8],
) -> Result<String, ApiError> {
    let boundary = format!("buzz-{}", uuid::Uuid::new_v4().simple());
    let response = client
        .post(config.endpoint().clone())
        .bearer_auth(config.api_key())
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .timeout(UPSTREAM_TIMEOUT)
        .body(multipart_body(&boundary, config.model(), audio))
        .send()
        .await
        .map_err(upstream_error)?;
    if !response.status().is_success() {
        // Status only: provider bodies can echo request details.
        tracing::warn!(
            status = response.status().as_u16(),
            "transcription upstream rejected the request"
        );
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "transcription provider rejected the request",
        ));
    }
    let body = read_capped(response, MAX_UPSTREAM_RESPONSE_BYTES)
        .await
        .map_err(|error| match error {
            CappedReadError::TooLarge => api_error(
                StatusCode::BAD_GATEWAY,
                "transcription provider response was too large",
            ),
            CappedReadError::Read(error) => upstream_error(error),
        })?;
    serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|value| value.get("text")?.as_str().map(str::to_owned))
        .ok_or_else(|| {
            api_error(
                StatusCode::BAD_GATEWAY,
                "transcription provider returned an invalid response",
            )
        })
}

/// Transcribe one audio clip for an authenticated community member.
pub async fn transcribe(
    State(state): State<Arc<AppState>>,
    Extension(config): Extension<Arc<TranscriptionConfig>>,
    slot: crate::access::AccessSlot,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    transcribe_inner(state, &config, slot, headers, body)
        .await
        .into_response()
}

#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
async fn transcribe_inner(
    state: Arc<AppState>,
    config: &TranscriptionConfig,
    slot: crate::access::AccessSlot,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, Response> {
    // Admission first: NIP-98 + NIP-FI must fire before any application-level
    // check. [FI-TRACE-HTTP-INGRESS]
    let (tenant, pubkey) =
        bridge::authenticate_member_post(&state, &slot, &headers, TRANSCRIPTIONS_PATH, &body)
            .await?;

    if !is_m4a(&headers) {
        return Err(api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported audio content type",
        )
        .into_response());
    }
    if body.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "audio body is empty").into_response());
    }
    // Taken before the quota so a busy relay does not spend the member's
    // allowance; held until the upstream call ends.
    let _upstream_slot = config.upstream_slots().try_acquire().map_err(|_| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "transcription is busy; try again shortly",
        )
        .into_response()
    })?;
    super::enforce_provider_quota(
        &state,
        &tenant,
        &pubkey,
        LimitType::Transcriptions,
        state.auth.config().rate_limits.transcriptions_per_min,
        "transcription",
        "buzz_transcription_rejections_total",
    )
    .await
    .map_err(IntoResponse::into_response)?;

    let text = request_transcript(&state.provider_http_client, config, &body)
        .await
        .map_err(IntoResponse::into_response)?;
    Ok(Json(serde_json::json!({ "text": text })))
}

#[cfg(test)]
#[path = "transcriptions/postgres_tests.rs"]
mod postgres_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{HeaderValue, Request},
    };
    use std::sync::Mutex;
    use tower::ServiceExt;

    const SECRET_KEY: &str = "sk-test-secret-transcription-key";

    pub(super) fn config_for(base_url: &str) -> TranscriptionConfig {
        TranscriptionConfig::from_values(Some(SECRET_KEY), Some(base_url), Some("test-model"))
            .expect("valid config")
            .expect("enabled")
    }

    fn client() -> reqwest::Client {
        crate::api::build_provider_http_client()
    }

    /// Serves `app` on an ephemeral port and returns its base URL.
    pub(super) async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub upstream");
        let address = listener.local_addr().expect("stub address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve stub");
        });
        (format!("http://{address}/v1"), server)
    }

    fn error_text(error: &ApiError) -> String {
        serde_json::to_string(&error.1 .0).expect("serialize error")
    }

    #[test]
    fn only_m4a_is_accepted() {
        for (raw, expected) in [
            ("audio/mp4", true),
            ("Audio/MP4; codecs=mp4a.40.2", true),
            ("audio/wav", false),
            ("application/json", false),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(raw));
            assert_eq!(is_m4a(&headers), expected, "{raw}");
        }
        assert!(!is_m4a(&HeaderMap::new()));
    }

    /// The upstream receives the operator key, the configured model and the
    /// exact audio bytes; only the transcript text comes back.
    #[tokio::test]
    async fn upstream_success_returns_only_the_transcript() {
        type Seen = Option<(HeaderMap, Vec<u8>)>;
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let recorder = seen.clone();
        let (base, server) = serve(Router::new().route(
            "/v1/audio/transcriptions",
            post(move |headers: HeaderMap, body: axum::body::Bytes| {
                *recorder.lock().unwrap() = Some((headers, body.to_vec()));
                async {
                    Json(serde_json::json!({
                        "text": "hello from the stub",
                        "usage": { "seconds": 3 },
                    }))
                }
            }),
        ))
        .await;
        let audio = b"\x00\x01fake-m4a-bytes\r\n--not-a-boundary".to_vec();

        let text = request_transcript(&client(), &config_for(&base), &audio)
            .await
            .expect("transcript");
        server.abort();

        assert_eq!(text, "hello from the stub");
        let (headers, body) = seen.lock().unwrap().take().expect("stub was called");
        assert_eq!(
            headers.get(header::AUTHORIZATION).unwrap(),
            &format!("Bearer {SECRET_KEY}")
        );
        let content_type = headers.get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
        assert!(content_type.starts_with("multipart/form-data; boundary=buzz-"));
        assert!(body.windows(audio.len()).any(|window| window == audio));
        let rendered = String::from_utf8_lossy(&body);
        assert!(rendered.contains("name=\"model\"\r\n\r\ntest-model\r\n"));
        assert!(rendered.contains("filename=\"audio.m4a\"\r\nContent-Type: audio/mp4"));
    }

    /// Every upstream failure becomes a fixed, generic relay error that leaks
    /// neither the key nor the provider body. Redirects are refused by the
    /// shared client itself, covered by
    /// `api::provider_http_client_tests::provider_client_refuses_redirects_without_leaking_secrets`.
    #[tokio::test]
    async fn upstream_failures_map_to_generic_errors() {
        let leaky = format!("invalid key {SECRET_KEY}");
        let leaky_for_error = leaky.clone();
        let (base, server) = serve(
            Router::new()
                .route(
                    "/error/v1/audio/transcriptions",
                    post(move || {
                        let body = leaky_for_error.clone();
                        async move { (StatusCode::UNAUTHORIZED, body) }
                    }),
                )
                .route(
                    "/not-json/v1/audio/transcriptions",
                    post(|| async { "plain text" }),
                )
                .route(
                    "/no-text/v1/audio/transcriptions",
                    post(|| async { Json(serde_json::json!({ "error": SECRET_KEY })) }),
                )
                .route(
                    "/huge/v1/audio/transcriptions",
                    post(|| async { "x".repeat(MAX_UPSTREAM_RESPONSE_BYTES + 1) }),
                ),
        )
        .await;
        let origin = base.trim_end_matches("/v1");

        for path in ["error", "not-json", "no-text", "huge"] {
            let error = request_transcript(
                &client(),
                &config_for(&format!("{origin}/{path}/v1")),
                b"audio",
            )
            .await
            .expect_err("upstream failure must not succeed");
            assert_eq!(error.0, StatusCode::BAD_GATEWAY, "{path}");
            let text = error_text(&error);
            assert!(!text.contains(SECRET_KEY), "{path}: {text}");
            assert!(!text.contains(&leaky), "{path}: {text}");
        }
        server.abort();
    }

    /// An upstream that never answers is cut off at the transcription
    /// deadline, not the shared client's shorter GIF default, and reported
    /// as a gateway timeout. Paused time makes the 60 s instant.
    #[tokio::test(start_paused = true)]
    async fn upstream_timeout_is_a_gateway_timeout() {
        let (base, server) = serve(Router::new().route(
            "/v1/audio/transcriptions",
            post(std::future::pending::<&'static str>),
        ))
        .await;
        let started = tokio::time::Instant::now();
        // The outer deadline turns a missing request timeout into a failure
        // instead of a hang.
        let error = tokio::time::timeout(
            UPSTREAM_TIMEOUT * 2,
            request_transcript(&client(), &config_for(&base), b"audio"),
        )
        .await
        .expect("the request deadline fires first")
        .expect_err("a silent upstream must time out");
        server.abort();
        assert_eq!(error.0, StatusCode::GATEWAY_TIMEOUT);
        assert!(started.elapsed() >= UPSTREAM_TIMEOUT);
    }

    /// The production route admits audio above axum's 2 MB extractor default
    /// and rejects anything above [`MAX_AUDIO_BYTES`] before authentication
    /// or any upstream call. Unconfigured, there is no route at all.
    #[tokio::test]
    async fn only_a_configured_relay_mounts_and_advertises_the_route() {
        let unconfigured = crate::state::tests::test_state_with_unreachable_database_url(
            "postgres://buzz:buzz_dev@127.0.0.1:1/buzz", // sadscan:disable np.postgres.1
        )
        .await;
        unconfigured.tenant_host_cache.insert(
            "relay.example".into(),
            (
                buzz_core::CommunityId::from_uuid(uuid::Uuid::nil()),
                std::time::Instant::now(),
            ),
        );
        let mut state = (*unconfigured).clone();
        Arc::make_mut(&mut state.config).transcription = Some(config_for("http://127.0.0.1:1/v1"));
        let state = Arc::new(state);

        // The served NIP-11 document advertises exactly what is mounted.
        for (state, configured) in [(&unconfigured, false), (&state, true)] {
            let document = serde_json::to_value(
                crate::nip11::nip11_document(state, "relay.example")
                    .await
                    .expect("NIP-11 document"),
            )
            .expect("serialize");
            let extension = serde_json::json!(EXTENSION);
            assert_eq!(
                document["supported_extensions"]
                    .as_array()
                    .expect("extensions")
                    .contains(&extension),
                configured
            );
            let descriptor = configured.then(|| serde_json::json!({ "path": TRANSCRIPTIONS_PATH }));
            assert_eq!(document.get("transcription").cloned(), descriptor);
            assert_eq!(router(state.clone()).is_some(), configured);
        }

        let routes = router(state).expect("configured route");

        // No Nostr proof: 401 before the body is read at all.
        let polled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let body_polled = polled.clone();
        let unread = futures_util::stream::once(async move {
            body_polled.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok::<_, std::io::Error>(axum::body::Bytes::from(vec![0_u8; MAX_AUDIO_BYTES]))
        });
        let response = routes
            .clone()
            .oneshot(
                Request::post(TRANSCRIPTIONS_PATH)
                    .header(header::CONTENT_TYPE, "audio/mp4")
                    .body(Body::from_stream(unread))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(!polled.load(std::sync::atomic::Ordering::SeqCst));

        // With a proof present, the audio-sized body cap applies.
        let request = |len: usize| {
            Request::post(TRANSCRIPTIONS_PATH)
                .header(header::AUTHORIZATION, "Nostr unverified")
                .header(header::CONTENT_TYPE, "audio/mp4")
                .header(header::CONTENT_LENGTH, len)
                .body(Body::from(vec![0_u8; len]))
                .expect("request")
        };

        let over = routes
            .clone()
            .oneshot(request(MAX_AUDIO_BYTES + 1))
            .await
            .expect("response");
        assert_eq!(over.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let under = routes
            .oneshot(request(3 * 1024 * 1024))
            .await
            .expect("response");
        assert_ne!(under.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
