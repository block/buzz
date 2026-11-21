//! Relay-owned GIPHY GIF metadata/search proxy.
//!
//! GIPHY requires a provider credential, but desktop applications cannot keep
//! build-time credentials secret. This narrow endpoint keeps the key on the
//! operator's relay and forwards GIPHY's `data` array of GIF objects (ids,
//! slugs, titles and rendition URLs) unchanged. The relay does not download,
//! cache or store GIF bytes. Clients validate the item fields they use.
//!
//! GIPHY's published best practices ask integrators not to proxy API calls.
//! Operators who enable this route accept that tension; see docs/gif-search.md.
//!
//! Search is the only relay endpoint. Sending a selected GIF is a normal
//! message containing its CDN URL, and clients render that URL through the
//! existing image path. No GIF bytes transit the relay.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;

use crate::state::AppState;

use buzz_auth::LimitType;

use super::{api_error, bridge, internal_error};

const GIPHY_API_ROOT: &str = "https://api.giphy.com/v1/gifs/";
pub(crate) const SEARCH_PATH: &str = "/gifs/search";
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_UPSTREAM_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// GIPHY caps the `q` parameter at 50 characters.
const MAX_QUERY_CHARS: usize = 50;
const RESULTS_PER_PAGE: &str = "24";

/// Build the dedicated GIPHY client. Redirects are disabled: the API key rides
/// in the request query string, so following a provider 3xx could replay a
/// key-bearing URL to an attacker-chosen host. With no redirect policy, a 3xx
/// comes back as a non-success status that the handler maps to a generic
/// `502`, and the `Location` target is never read or forwarded.
pub fn build_gif_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(UPSTREAM_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("static GIF HTTP client configuration")
}

#[derive(Debug, Deserialize)]
/// Client-owned search context forwarded to GIPHY by the relay.
pub struct SearchRequest {
    /// Empty means trending; otherwise this is the user's search text.
    query: String,
    /// Stable anonymous installation identifier, forwarded as GIPHY `customer_id`.
    customer_id: String,
    /// Desktop locale (BCP-47, for example `en-US`). The relay derives the
    /// two-letter GIPHY `lang` code from its first subtag.
    locale: String,
}

fn validate_text(
    name: &str,
    value: &str,
    max_chars: usize,
    allow_empty: bool,
) -> Result<(), (StatusCode, Json<Value>)> {
    let count = value.chars().count();
    if (!allow_empty && value.trim().is_empty()) || count > max_chars {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            &format!(
                "{name} must be {} through {max_chars} characters",
                if allow_empty { 0 } else { 1 }
            ),
        ));
    }
    Ok(())
}

/// Derive the GIPHY `lang` value (ISO 639-1, two ASCII letters) from a BCP-47
/// locale such as `en-US` or `pt_BR`. Returns `None` when the first subtag is
/// not a two-letter code, so the relay omits `lang` instead of sending junk.
fn giphy_lang(locale: &str) -> Option<String> {
    let first = locale.trim().split(['-', '_']).next().unwrap_or_default();
    if first.len() == 2 && first.chars().all(|c| c.is_ascii_alphabetic()) {
        Some(first.to_ascii_lowercase())
    } else {
        None
    }
}

/// Build a GIPHY request URL. The credential is appended as the `api_key`
/// query parameter, as GIPHY requires, so the caller must never log the URL.
fn giphy_url(
    api_key: &str,
    endpoint: &str,
    query: &[(&str, &str)],
) -> Result<url::Url, (StatusCode, Json<Value>)> {
    let mut url = url::Url::parse(GIPHY_API_ROOT)
        .map_err(|_| internal_error("invalid static GIPHY API root"))?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| internal_error("invalid static GIPHY API root"))?;
        segments.pop_if_empty().push(endpoint);
    }
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("api_key", api_key);
        pairs.extend_pairs(query.iter().copied());
    }
    Ok(url)
}

async fn send_upstream(
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, (StatusCode, Json<Value>)> {
    request
        .timeout(UPSTREAM_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(
                timeout = error.is_timeout(),
                "GIPHY upstream request failed"
            );
            api_error(StatusCode::BAD_GATEWAY, "GIF provider is unavailable")
        })
}

async fn enforce_search_admission(
    state: &AppState,
    tenant: &buzz_core::TenantContext,
    pubkey: &nostr::PublicKey,
) -> Result<(), (StatusCode, Json<Value>)> {
    let limit = state.auth.config().rate_limits.gif_searches_per_min;
    match crate::admission::check_principal(
        state.admission_rate_limiter.as_ref(),
        tenant,
        pubkey,
        LimitType::GifSearches,
        60,
        limit,
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(crate::admission::AdmissionError::Exceeded { reset_in_secs }) => {
            metrics::counter!("buzz_gif_search_rejections_total", "reason" => "quota").increment(1);
            Err(api_error(
                StatusCode::TOO_MANY_REQUESTS,
                &format!("rate-limited: GIF search quota exceeded; retry in {reset_in_secs}s"),
            ))
        }
        Err(crate::admission::AdmissionError::Unavailable) => Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "rate-limited: GIF search admission unavailable",
        )),
    }
}

async fn limited_json(response: reqwest::Response) -> Result<Value, (StatusCode, Json<Value>)> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_UPSTREAM_RESPONSE_BYTES as u64)
    {
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "GIF provider response was too large",
        ));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            api_error(
                StatusCode::BAD_GATEWAY,
                "GIF provider response could not be read",
            )
        })?;
        if body.len().saturating_add(chunk.len()) > MAX_UPSTREAM_RESPONSE_BYTES {
            return Err(api_error(
                StatusCode::BAD_GATEWAY,
                "GIF provider response was too large",
            ));
        }
        body.extend_from_slice(&chunk);
    }

    serde_json::from_slice(&body).map_err(|_| {
        api_error(
            StatusCode::BAD_GATEWAY,
            "GIF provider returned an invalid response",
        )
    })
}

/// Reduce a GIPHY envelope to `{ "data": [...] }`.
///
/// GIPHY reports success through a numeric `meta.status`, which must be
/// exactly 200. A missing, non-numeric or non-200 status is rejected. Only the
/// `data` array is forwarded; `meta` (which carries a `response_id`) and
/// `pagination` are dropped so provider diagnostics never cross the relay
/// boundary.
fn successful_search_payload(upstream: &Value) -> Result<Value, (StatusCode, Json<Value>)> {
    let meta_status = upstream
        .get("meta")
        .and_then(|meta| meta.get("status"))
        .and_then(Value::as_u64);
    if meta_status != Some(200) {
        tracing::warn!("GIPHY search returned a missing or unsuccessful meta status");
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "GIF provider rejected the search request",
        ));
    }
    let Some(data) = upstream.get("data").filter(|data| data.is_array()) else {
        tracing::warn!("GIPHY search returned no data array");
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "GIF provider returned an invalid response",
        ));
    };
    Ok(serde_json::json!({ "data": data }))
}

/// Search or browse trending GIPHY GIF metadata for an authenticated member.
pub async fn search(
    State(state): State<Arc<AppState>>,
    slot: crate::access::AccessSlot,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    search_inner(state, slot, headers, body)
        .await
        .into_response()
}

#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
async fn search_inner(
    state: Arc<AppState>,
    slot: crate::access::AccessSlot,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, Response> {
    // Admission first: NIP-98 + NIP-FI must fire before any application-level
    // check (including provider availability) so the denial contract wins over
    // config or request-validation errors. [FI-TRACE-HTTP-INGRESS]
    let (tenant, pubkey) =
        bridge::authenticate_member_post(&state, &slot, &headers, SEARCH_PATH, &body).await?;

    let Some(config) = state.config.giphy.as_ref() else {
        return Err(
            api_error(StatusCode::NOT_FOUND, "GIF search is not configured").into_response(),
        );
    };
    let request: SearchRequest = serde_json::from_slice(&body).map_err(|_| {
        api_error(StatusCode::BAD_REQUEST, "invalid GIF search JSON").into_response()
    })?;
    let query_text = request.query.trim();
    validate_text("query", query_text, MAX_QUERY_CHARS, true).map_err(|e| e.into_response())?;
    validate_text("customer_id", &request.customer_id, 128, false)
        .map_err(|e| e.into_response())?;
    validate_text("locale", &request.locale, 32, false).map_err(|e| e.into_response())?;
    enforce_search_admission(&state, &tenant, &pubkey)
        .await
        .map_err(|e| e.into_response())?;

    let endpoint = if query_text.is_empty() {
        "trending"
    } else {
        "search"
    };
    let lang = giphy_lang(&request.locale);
    let mut query = vec![
        ("limit", RESULTS_PER_PAGE),
        ("offset", "0"),
        ("customer_id", request.customer_id.as_str()),
    ];
    if !query_text.is_empty() {
        query.push(("q", query_text));
        if let Some(lang) = lang.as_deref() {
            query.push(("lang", lang));
        }
    }
    let url = giphy_url(config.api_key(), endpoint, &query).map_err(|e| e.into_response())?;
    let response = send_upstream(state.gif_http_client.get(url))
        .await
        .map_err(|e| e.into_response())?;
    if !response.status().is_success() {
        tracing::warn!(status = response.status().as_u16(), "GIPHY search failed");
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "GIF provider rejected the search request",
        )
        .into_response());
    }

    // Never forward the provider response wholesale. Allowlist only the
    // `data` array so provider diagnostics cannot cross the relay boundary.
    let upstream = limited_json(response)
        .await
        .map_err(|e| e.into_response())?;
    Ok(Json(
        successful_search_payload(&upstream).map_err(|e| e.into_response())?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    async fn unconfigured_test_state() -> Arc<AppState> {
        let mut config = crate::config::Config::for_test(); // [FI-TRACE-ENV-RACE]
        config.giphy = None;
        config.redis_url = "redis://127.0.0.1:1".to_string();

        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://buzz:buzz_dev@127.0.0.1:1/buzz") // sadscan:disable np.postgres.1
            .expect("lazy test database pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool =
            buzz_pubsub::redis_connection(&config.redis_url).expect("lazy test Redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("test pubsub"),
        );
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage =
            buzz_media::MediaStorage::new(&config.media).expect("test media storage config");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            None::<buzz_audit::AuditService>,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    #[tokio::test]
    async fn search_route_returns_not_found_before_auth_when_unconfigured() {
        let state = unconfigured_test_state().await;
        let response = Router::new()
            .route(SEARCH_PATH, axum::routing::post(search))
            .with_state(state)
            .oneshot(
                Request::post(SEARCH_PATH)
                    .body(Body::from("{}"))
                    .expect("search request"),
            )
            .await
            .expect("search response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn limited_json_rejects_oversized_streamed_bodies() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/oversized",
                    get(|| async {
                        (
                            [(header::CONTENT_TYPE, "application/json")],
                            "x".repeat(MAX_UPSTREAM_RESPONSE_BYTES + 1),
                        )
                    }),
                ),
            )
            .await
            .expect("serve oversized response");
        });
        let response = reqwest::get(format!("http://{address}/oversized"))
            .await
            .expect("test upstream response");
        let (status, _) = limited_json(response)
            .await
            .expect_err("oversized body must be rejected");

        server.abort();
        let _ = server.await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn giphy_url_sends_credentials_as_an_encoded_query_parameter() {
        let url = giphy_url(
            "key/with spaces&more",
            "search",
            &[("q", "cheese burgers"), ("customer_id", "customer")],
        )
        .expect("static URL is valid");

        assert_eq!(
            url.as_str(),
            "https://api.giphy.com/v1/gifs/search?api_key=key%2Fwith+spaces%26more&q=cheese+burgers&customer_id=customer"
        );
    }

    #[test]
    fn giphy_url_selects_the_trending_endpoint() {
        let url =
            giphy_url("secret-key", "trending", &[("limit", "24")]).expect("static URL is valid");
        assert_eq!(
            url.as_str(),
            "https://api.giphy.com/v1/gifs/trending?api_key=secret-key&limit=24"
        );
    }

    #[test]
    fn giphy_lang_uses_the_first_two_letter_subtag_only() {
        assert_eq!(giphy_lang("en-US").as_deref(), Some("en"));
        assert_eq!(giphy_lang("pt_BR").as_deref(), Some("pt"));
        assert_eq!(giphy_lang("DE").as_deref(), Some("de"));
        assert_eq!(giphy_lang("zh-Hant-TW").as_deref(), Some("zh"));
        assert_eq!(giphy_lang("eng-US"), None);
        assert_eq!(giphy_lang("1a-XX"), None);
        assert_eq!(giphy_lang(""), None);
    }

    #[test]
    fn validation_bounds_provider_control_fields() {
        assert!(validate_text("query", "", MAX_QUERY_CHARS, true).is_ok());
        assert!(
            validate_text("query", &"x".repeat(MAX_QUERY_CHARS), MAX_QUERY_CHARS, true).is_ok()
        );
        assert!(validate_text(
            "query",
            &"x".repeat(MAX_QUERY_CHARS + 1),
            MAX_QUERY_CHARS,
            true
        )
        .is_err());
        assert!(validate_text("customer_id", "", 128, false).is_err());
    }

    #[test]
    fn successful_payload_forwards_only_the_data_array() {
        let payload = successful_search_payload(&serde_json::json!({
            "data": [{ "id": "abc", "type": "gif" }],
            "pagination": { "total_count": 1, "count": 1, "offset": 0 },
            "meta": { "status": 200, "msg": "OK", "response_id": "secret-response-id" }
        }))
        .expect("successful payload");

        assert_eq!(
            payload,
            serde_json::json!({ "data": [{ "id": "abc", "type": "gif" }] })
        );
        assert!(!payload.to_string().contains("secret-response-id"));
    }

    #[test]
    fn unsuccessful_meta_status_is_rejected_without_provider_details() {
        let (status, body) = successful_search_payload(&serde_json::json!({
            "data": [],
            "meta": { "status": 429, "msg": "request used secret-key", "response_id": "x" }
        }))
        .expect_err("unsuccessful provider payload must be rejected");

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        let serialized = serde_json::to_string(&body.0).expect("serialize generic error");
        assert!(!serialized.contains("secret-key"));
    }

    #[test]
    fn missing_or_non_numeric_meta_status_is_rejected() {
        for upstream in [
            serde_json::json!({ "data": [] }),
            serde_json::json!({ "data": [], "meta": {} }),
            serde_json::json!({ "data": [], "meta": { "status": "200" } }),
            serde_json::json!({ "data": [], "meta": { "status": null } }),
            serde_json::json!({ "data": [], "meta": { "status": -200 } }),
        ] {
            let (status, _) = successful_search_payload(&upstream)
                .expect_err("payload without a numeric 200 meta status must be rejected");
            assert_eq!(status, StatusCode::BAD_GATEWAY, "{upstream}");
        }
    }

    #[test]
    fn payload_without_a_data_array_is_rejected() {
        let (status, _) = successful_search_payload(&serde_json::json!({
            "data": { "data": [] },
            "meta": { "status": 200 }
        }))
        .expect_err("object data must be rejected");
        assert_eq!(status, StatusCode::BAD_GATEWAY);

        let (status, _) =
            successful_search_payload(&serde_json::json!({ "meta": { "status": 200 } }))
                .expect_err("missing data must be rejected");
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    /// A provider 3xx must never cause a second connection, and the error
    /// surfaced past the shared send/reject path must leak neither the API key
    /// (carried in the request query string) nor the redirect target.
    ///
    /// Mutation check: swapping `build_gif_http_client`'s redirect policy back
    /// to the default makes the client follow the 302, the redirect listener
    /// records a request, and this test fails on the `redirect_hits` assertion.
    #[tokio::test]
    async fn gif_client_refuses_provider_redirects_without_leaking_secrets() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        const SECRET_KEY: &str = "super-secret-giphy-key";

        // Second listener: the redirect target. It must never be reached.
        let redirect_hits = Arc::new(AtomicUsize::new(0));
        let redirect_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind redirect target");
        let redirect_addr = redirect_listener.local_addr().expect("redirect address");
        let redirect_hits_server = redirect_hits.clone();
        let redirect_server = tokio::spawn(async move {
            axum::serve(
                redirect_listener,
                Router::new().route(
                    "/leaked",
                    get(move || {
                        redirect_hits_server.fetch_add(1, Ordering::SeqCst);
                        async { "reached the redirect target" }
                    }),
                ),
            )
            .await
            .expect("serve redirect target");
        });

        // Fake upstream: answers the key-bearing URL with a 302 whose Location
        // points at the second listener, exactly the disclosure vector.
        let redirect_location = format!("http://{redirect_addr}/leaked");
        let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake upstream");
        let upstream_addr = upstream_listener.local_addr().expect("upstream address");
        let location_header = redirect_location.clone();
        let upstream_server = tokio::spawn(async move {
            axum::serve(
                upstream_listener,
                Router::new().route(
                    "/v1/gifs/search",
                    get(move || {
                        let location = location_header.clone();
                        async move {
                            (
                                StatusCode::FOUND,
                                [(header::LOCATION, location)],
                                "provider body naming the secret-key",
                            )
                        }
                    }),
                ),
            )
            .await
            .expect("serve fake upstream");
        });

        let client = build_gif_http_client();
        let response = send_upstream(client.get(format!(
            "http://{upstream_addr}/v1/gifs/search?api_key={SECRET_KEY}"
        )))
        .await
        .expect("request completes without following the redirect");

        // The redirect was not followed: the client surfaces the 3xx itself.
        assert!(response.status().is_redirection());
        assert!(!response.status().is_success());
        assert_eq!(redirect_hits.load(Ordering::SeqCst), 0);

        // The shared reject path (the handler gates on `!is_success`) returns a
        // static generic error carrying no key and no redirect target.
        let (status, body) = api_error(
            StatusCode::BAD_GATEWAY,
            "GIF provider rejected the search request",
        );
        let serialized = serde_json::to_string(&body.0).expect("serialize generic error");
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(!serialized.contains(SECRET_KEY));
        assert!(!serialized.contains(&redirect_location));

        upstream_server.abort();
        redirect_server.abort();
        let _ = upstream_server.await;
        let _ = redirect_server.await;
    }
}
