use axum::{
    body::Body,
    extract::{Request, State as AxumState},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use futures_util::TryStreamExt;
use tauri::{http, Manager};
use tokio::net::TcpListener;

use crate::app_state::AppState;
use crate::relay;

/// Defense-in-depth cap: refuse to buffer responses larger than this into RAM.
/// Range requests (≤16 MiB from server) always fit. Full GETs for huge videos
/// get a clear 413 instead of OOM — the <video> element always uses range
/// requests for seeking, so this only catches edge cases.
const MAX_PROXY_RESPONSE: u64 = 20 * 1024 * 1024;

#[derive(Clone)]
struct ProxyState {
    client: reqwest::Client,
    app_handle: tauri::AppHandle,
}

async fn proxy_handler(AxumState(state): AxumState<ProxyState>, req: Request) -> Response {
    // Allow requests with no Origin (e.g. <video> element fetches) or from
    // the Tauri webview origin. Blocks cross-origin JS fetches from other
    // tabs/apps while letting HTML media resource loads through.
    let origin = req
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !origin.is_empty() && origin != "tauri://localhost" && origin != "http://tauri.localhost" {
        return (StatusCode::FORBIDDEN, "forbidden: invalid origin").into_response();
    }

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    // Resolve relay URL dynamically so workspace switches take effect immediately.
    let app_state = state.app_handle.state::<AppState>();
    let base_url = relay::relay_api_base_url_with_override(&app_state);
    let upstream_url = format!("{base_url}{path_and_query}");

    let has_range = req.headers().contains_key("range");

    let mut upstream = state
        .client
        .get(&upstream_url)
        .timeout(std::time::Duration::from_secs(120));

    if let Some(range) = req.headers().get("range") {
        if let Ok(v) = range.to_str() {
            upstream = upstream.header("range", v);
        }
    }

    let resp = match upstream.send().await {
        Ok(r) => r,
        Err(_) => {
            return (StatusCode::BAD_GATEWAY, "upstream request failed").into_response();
        }
    };

    let status =
        StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let mut headers = HeaderMap::new();
    for key in &[
        "content-type",
        "content-range",
        "accept-ranges",
        "content-length",
        // Cache-related headers — let WKWebView's HTTP cache do its job so
        // images don't re-fetch on every channel switch. Media URLs are
        // content-addressed (sha256 in path), so the relay sends
        // `Cache-Control: public, max-age=31536000, immutable` and we
        // forward it verbatim. `etag`/`last-modified` are forwarded for
        // future-proofing if upstream ever adds them.
        "cache-control",
        "etag",
        "last-modified",
    ] {
        if let Some(val) = resp.headers().get(*key) {
            if let Ok(v) = HeaderValue::from_bytes(val.as_bytes()) {
                headers.insert(*key, v);
            }
        }
    }

    // OOM guard for non-range full GETs (same 20 MB cap as the protocol handler).
    if !has_range {
        if let Some(cl) = headers.get("content-length") {
            if let Ok(len) = cl.to_str().unwrap_or("0").parse::<u64>() {
                if len > MAX_PROXY_RESPONSE {
                    return (
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "response too large — use range requests for video playback",
                    )
                        .into_response();
                }
            }
        }
    }

    // Stream the body — no buffering.
    let stream = resp
        .bytes_stream()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
    let body = Body::from_stream(stream);

    (status, headers, body).into_response()
}

/// Spawn a localhost HTTP proxy that streams media via reqwest, avoiding the
/// Tauri protocol handler's requirement to buffer the entire response into
/// `Vec<u8>`. Returns the OS-assigned port.
pub async fn spawn_media_proxy(http_client: reqwest::Client, app_handle: tauri::AppHandle) -> u16 {
    let proxy_state = ProxyState {
        client: http_client,
        app_handle,
    };

    let app = Router::new()
        .route("/media/{*path}", get(proxy_handler))
        .with_state(proxy_state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind media proxy");
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    eprintln!("sprout-desktop: media proxy listening on 127.0.0.1:{port}");
    port
}

/// Proxy media requests through the Rust backend so they traverse the WARP tunnel.
///
/// WKWebView's networking stack bypasses WARP, causing 403s from Cloudflare Access.
/// This handler routes `sprout-media://localhost/{path}` through reqwest, which
/// runs in the Tauri process and goes through WARP.
pub async fn handle_sprout_media(
    app: &tauri::AppHandle,
    request: &http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    use tauri::Manager;

    let state = app.state::<AppState>();
    let base = relay::relay_api_base_url_with_override(&state);

    // Preserve path + query (thumbnails may have query params).
    // Only proxy /media/ paths — reject anything else.
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    if !path_and_query.starts_with("/media/") {
        return error_response(404, "not found");
    }

    let has_range = request.headers().contains_key("range");
    let upstream_url = format!("{base}{path_and_query}");

    // Forward Range header if present — enables video seeking through the proxy.
    let mut upstream = state
        .http_client
        .get(&upstream_url)
        .timeout(std::time::Duration::from_secs(60));
    if let Some(range) = request.headers().get("range") {
        if let Ok(v) = range.to_str() {
            upstream = upstream.header("range", v);
        }
    }

    let result = upstream.send().await;

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();

            // Propagate range-related headers so <video> seeking works.
            let content_range = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let accept_ranges = resp
                .headers()
                .get("accept-ranges")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let content_length = resp
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            // Propagate cache-related headers so WKWebView's HTTP cache
            // can avoid re-fetching content-addressed media on every
            // channel switch. The relay sends
            // `Cache-Control: public, max-age=31536000, immutable`;
            // `etag`/`last-modified` are forwarded if upstream supplies them.
            let cache_control = resp
                .headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let etag = resp
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let last_modified = resp
                .headers()
                .get("last-modified")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            // OOM guard: if this is a non-range GET and the upstream body is
            // larger than our cap, bail with 413 instead of buffering into RAM.
            // Tauri's protocol handler requires Vec<u8> so we can't truly stream.
            if !has_range {
                if let Some(ref cl) = content_length {
                    if let Ok(len) = cl.parse::<u64>() {
                        if len > MAX_PROXY_RESPONSE {
                            return error_response(
                                413,
                                "response too large — use range requests for video playback",
                            );
                        }
                    }
                }
            }

            match resp.bytes().await {
                Ok(bytes) => {
                    let mut builder = http::Response::builder()
                        .status(status)
                        .header("content-type", &content_type);
                    if let Some(ref cr) = content_range {
                        builder = builder.header("content-range", cr);
                    }
                    if let Some(ref ar) = accept_ranges {
                        builder = builder.header("accept-ranges", ar);
                    }
                    if let Some(ref cl) = content_length {
                        builder = builder.header("content-length", cl);
                    }
                    if let Some(ref cc) = cache_control {
                        builder = builder.header("cache-control", cc);
                    }
                    if let Some(ref e) = etag {
                        builder = builder.header("etag", e);
                    }
                    if let Some(ref lm) = last_modified {
                        builder = builder.header("last-modified", lm);
                    }
                    builder
                        .body(bytes.to_vec())
                        .unwrap_or_else(|_| error_response(500, "response build failed"))
                }
                Err(_) => error_response(502, "failed to read upstream body"),
            }
        }
        Err(_) => error_response(502, "upstream request failed"),
    }
}

fn error_response(status: u16, msg: &str) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(msg.as_bytes().to_vec())
        .unwrap_or_else(|_| {
            http::Response::builder()
                .status(500)
                .body(Vec::new())
                .unwrap()
        })
}
