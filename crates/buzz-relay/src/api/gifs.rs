//! Relay-owned KLIPY GIF metadata/search proxy.
//!
//! KLIPY requires a provider credential, but desktop applications cannot keep
//! build-time credentials secret. These narrow endpoints keep the key on the
//! operator's relay while returning only KLIPY-hosted media URLs and metadata;
//! GIF bytes are never downloaded, cached, or stored by Buzz.
//!
//! Search and share reporting are the only relay endpoints. Sending a selected
//! GIF is a normal message containing its CDN URL, and clients render that URL
//! through the existing image path. No GIF bytes transit the relay.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::Json,
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;

use crate::state::AppState;

use buzz_auth::LimitType;

use super::{api_error, bridge, internal_error, relay_members};

const KLIPY_API_ROOT: &str = "https://api.klipy.com/api/v1/";
pub(crate) const SEARCH_PATH: &str = "/gifs/search";
pub(crate) const SHARE_PATH: &str = "/gifs/share";
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_UPSTREAM_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// Build the dedicated KLIPY client. Redirects are disabled: the API key rides
/// in the request path, so following a provider 3xx could replay a key-bearing
/// URL to an attacker-chosen host. With no redirect policy, a 3xx comes back as
/// a non-success status that the handlers map to a generic `502`, and the
/// `Location` target is never read or forwarded.
pub fn build_gif_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(UPSTREAM_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("static GIF HTTP client configuration")
}

#[derive(Debug, Deserialize)]
/// Client-owned search context forwarded to KLIPY by the relay.
pub struct SearchRequest {
    /// Empty means trending; otherwise this is the user's search text.
    query: String,
    /// Stable anonymous installation identifier required by KLIPY.
    customer_id: String,
    /// Desktop locale used to localize provider results.
    locale: String,
}

#[derive(Debug, Deserialize)]
/// Client-owned share context forwarded to KLIPY by the relay.
pub struct ShareRequest {
    /// Provider slug for the selected GIF.
    slug: String,
    /// Stable anonymous installation identifier required by KLIPY.
    customer_id: String,
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

fn klipy_url(
    api_key: &str,
    path: &[&str],
    query: &[(&str, &str)],
) -> Result<url::Url, (StatusCode, Json<Value>)> {
    let mut url = url::Url::parse(KLIPY_API_ROOT)
        .map_err(|_| internal_error("invalid static KLIPY API root"))?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| internal_error("invalid static KLIPY API root"))?;
        segments.pop_if_empty().push(api_key);
        for segment in path {
            segments.push(segment);
        }
    }
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query.iter().copied());
    }
    Ok(url)
}

fn klipy_share_request(
    client: &reqwest::Client,
    api_key: &str,
    request: &ShareRequest,
) -> Result<reqwest::RequestBuilder, (StatusCode, Json<Value>)> {
    let url = klipy_url(api_key, &["gifs", "share", request.slug.trim()], &[])?;
    Ok(client
        .post(url)
        .json(&serde_json::json!({ "customer_id": request.customer_id })))
}

async fn authenticate(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    body: &[u8],
) -> Result<(buzz_core::TenantContext, nostr::PublicKey), (StatusCode, Json<Value>)> {
    let raw_host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
        })?;

    let expected_url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, path);
    let bridge::VerifiedBridgeAuth {
        pubkey,
        event_id_bytes,
        signed_created_at,
    } = bridge::verify_bridge_auth_with_options(
        headers,
        "POST",
        &expected_url,
        Some(body),
        true,
        true,
    )?;
    bridge::enforce_http_admission(state, &tenant, &pubkey).await?;
    bridge::check_nip98_replay(state, &tenant, event_id_bytes).await?;
    relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey.to_bytes(),
        relay_members::extract_auth_tag_header(headers),
        signed_created_at,
    )
    .await?;

    Ok((tenant, pubkey))
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
                "KLIPY upstream request failed"
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

fn successful_search_payload(upstream: &Value) -> Result<Value, (StatusCode, Json<Value>)> {
    if upstream.get("result").and_then(Value::as_bool) != Some(true) {
        tracing::warn!("KLIPY search returned an unsuccessful result");
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "GIF provider rejected the search request",
        ));
    }
    let data = upstream.get("data").cloned().unwrap_or(Value::Null);
    Ok(serde_json::json!({ "result": true, "data": data }))
}

/// Search or browse trending KLIPY GIF metadata for an authenticated member.
pub async fn search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let Some(config) = state.config.klipy.as_ref() else {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "GIF search is not configured",
        ));
    };
    let (tenant, pubkey) = authenticate(&state, &headers, SEARCH_PATH, &body).await?;
    let request: SearchRequest = serde_json::from_slice(&body)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid GIF search JSON"))?;
    validate_text("query", &request.query, 200, true)?;
    validate_text("customer_id", &request.customer_id, 128, false)?;
    validate_text("locale", &request.locale, 32, false)?;
    enforce_search_admission(&state, &tenant, &pubkey).await?;

    let endpoint = if request.query.trim().is_empty() {
        "trending"
    } else {
        "search"
    };
    let mut query = vec![
        ("page", "1"),
        ("per_page", "24"),
        ("customer_id", request.customer_id.as_str()),
        ("locale", request.locale.as_str()),
    ];
    if !request.query.trim().is_empty() {
        query.push(("q", request.query.trim()));
    }
    let url = klipy_url(config.api_key(), &["gifs", endpoint], &query)?;
    let response = send_upstream(state.gif_http_client.get(url)).await?;
    if !response.status().is_success() {
        tracing::warn!(status = response.status().as_u16(), "KLIPY search failed");
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "GIF provider rejected the search request",
        ));
    }

    // Never forward the provider response wholesale. KLIPY may report an
    // application-level failure with HTTP 200 and include request details in
    // its error fields. Allowlist only successful result data so credentials
    // and provider diagnostics cannot cross the relay boundary.
    let upstream = limited_json(response).await?;
    Ok(Json(successful_search_payload(&upstream)?))
}

/// Report a selected GIF to KLIPY so the provider can update Recents.
pub async fn share(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let Some(config) = state.config.klipy.as_ref() else {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "GIF search is not configured",
        ));
    };
    authenticate(&state, &headers, SHARE_PATH, &body).await?;
    let request: ShareRequest = serde_json::from_slice(&body)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid GIF share JSON"))?;
    validate_text("slug", &request.slug, 200, false)?;
    validate_text("customer_id", &request.customer_id, 128, false)?;

    let response = send_upstream(klipy_share_request(
        &state.gif_http_client,
        config.api_key(),
        &request,
    )?)
    .await?;
    if !response.status().is_success() {
        tracing::warn!(status = response.status().as_u16(), "KLIPY share failed");
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "GIF provider rejected the share request",
        ));
    }

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    async fn unconfigured_test_state() -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("test config");
        config.klipy = None;
        config.redis_url = "redis://127.0.0.1:1".to_string();

        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://buzz:buzz_dev@127.0.0.1:1/buzz") // sadscan:disable np.postgres.1
            .expect("lazy test database pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("lazy test Redis pool");
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
    fn klipy_url_encodes_credentials_as_a_path_segment() {
        let url = klipy_url(
            "key/with spaces",
            &["gifs", "search"],
            &[("customer_id", "customer")],
        )
        .expect("static URL is valid");

        assert_eq!(
            url.as_str(),
            "https://api.klipy.com/api/v1/key%2Fwith%20spaces/gifs/search?customer_id=customer"
        );
    }

    #[test]
    fn klipy_share_request_uses_slug_path_and_customer_body() {
        let request = ShareRequest {
            slug: " ship/it ".to_string(),
            customer_id: "customer-123".to_string(),
        };
        let built = klipy_share_request(&reqwest::Client::new(), "secret-key", &request)
            .expect("share request builds")
            .build()
            .expect("share request is valid");

        assert_eq!(built.method(), reqwest::Method::POST);
        assert_eq!(
            built.url().as_str(),
            "https://api.klipy.com/api/v1/secret-key/gifs/share/ship%2Fit"
        );
        assert_eq!(
            built.body().and_then(reqwest::Body::as_bytes),
            Some(br#"{"customer_id":"customer-123"}"#.as_slice())
        );
    }

    #[test]
    fn validation_bounds_provider_control_fields() {
        assert!(validate_text("query", "", 200, true).is_ok());
        assert!(validate_text("customer_id", "", 128, false).is_err());
        assert!(validate_text("query", &"x".repeat(201), 200, true).is_err());
    }

    #[test]
    fn successful_payload_strips_provider_errors_and_unknown_fields() {
        let payload = successful_search_payload(&serde_json::json!({
            "result": true,
            "data": { "data": [] },
            "errors": { "message": ["request used secret-key"] },
            "debug": "secret-key"
        }))
        .expect("successful payload");

        assert_eq!(
            payload,
            serde_json::json!({ "result": true, "data": { "data": [] } })
        );
    }

    #[test]
    fn unsuccessful_payload_is_rejected_without_provider_details() {
        let (status, body) = successful_search_payload(&serde_json::json!({
            "result": false,
            "errors": { "message": ["request used secret-key"] }
        }))
        .expect_err("unsuccessful provider payload must be rejected");

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        let serialized = serde_json::to_string(&body.0).expect("serialize generic error");
        assert!(!serialized.contains("secret-key"));
    }

    /// A provider 3xx must never cause a second connection, and the error
    /// surfaced past the shared send/reject path must leak neither the API key
    /// (carried in the request path) nor the redirect target.
    ///
    /// Mutation check: swapping `build_gif_http_client`'s redirect policy back
    /// to the default makes the client follow the 302, the redirect listener
    /// records a request, and this test fails on the `redirect_hits` assertion.
    #[tokio::test]
    async fn gif_client_refuses_provider_redirects_without_leaking_secrets() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        const SECRET_KEY: &str = "super-secret-klipy-key";

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

        // Fake upstream: answers the key-bearing path with a 302 whose Location
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
                    &format!("/{SECRET_KEY}/gifs/search"),
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
        let response =
            send_upstream(client.get(format!("http://{upstream_addr}/{SECRET_KEY}/gifs/search")))
                .await
                .expect("request completes without following the redirect");

        // The redirect was not followed: the client surfaces the 3xx itself.
        assert!(response.status().is_redirection());
        assert!(!response.status().is_success());
        assert_eq!(redirect_hits.load(Ordering::SeqCst), 0);

        // The shared reject path (both handlers gate on `!is_success`) returns a
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
