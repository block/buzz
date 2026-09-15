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
use crate::media_read::MediaReadScope;

/// Defense-in-depth cap: refuse to buffer responses larger than this into RAM.
/// The buffered protocol applies this to actual bytes, including range requests.
/// Oversized full GETs with Content-Length get 413 before reading the body;
/// oversized chunked protocol responses fail during bounded buffering.
const MAX_PROXY_RESPONSE: u64 = 20 * 1024 * 1024;

#[derive(Clone)]
struct ProxyState {
    app_handle: tauri::AppHandle,
}

// This query field binds element loads (which cannot set IPC headers) to the
// mounted renderer. It is not a credential and is never forwarded upstream.
const GENERATION_QUERY: &str = "__buzz_generation";

fn media_path(uri: &http::Uri) -> Result<(String, Option<u64>), String> {
    if !uri.path().starts_with("/media/") {
        return Err("not found".into());
    }
    let mut generation = None;
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()) {
        if key == GENERATION_QUERY {
            if generation.is_some()
                || value.is_empty()
                || !value.bytes().all(|b| b.is_ascii_digit())
            {
                return Err("invalid media generation".into());
            }
            generation = Some(
                value
                    .parse::<u64>()
                    .map_err(|_| "invalid media generation")?,
            );
        } else {
            query.append_pair(&key, &value);
        }
    }
    let query = query.finish();
    let path = if query.is_empty() {
        uri.path().to_owned()
    } else {
        format!("{}?{query}", uri.path())
    };
    Ok((path, generation))
}

async fn proxy_handler(AxumState(state): AxumState<ProxyState>, req: Request) -> Response {
    proxy_response(
        &state.app_handle.state::<AppState>(),
        req.uri(),
        req.headers(),
    )
    .await
}

// Shared by the loopback HTTP handler and custom protocol; neither goes through
// Tauri dispatch, so admission and the entire body lifetime belong here.
/// Admit a renderer media read and stream only while its captured session is valid.
pub(crate) async fn proxy_response(
    state: &AppState,
    uri: &http::Uri,
    request_headers: &HeaderMap,
) -> Response {
    let origin = request_headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !origin.is_empty() && origin != "tauri://localhost" && origin != "http://tauri.localhost" {
        return proxy_error(StatusCode::FORBIDDEN, "forbidden: invalid origin");
    }
    let (path, generation) = match media_path(uri) {
        Ok(value) => value,
        Err(_) => return proxy_error(StatusCode::BAD_REQUEST, "invalid media URL"),
    };
    let scope = match MediaReadScope::capture_renderer(state, generation) {
        Ok(scope) => scope,
        Err(_) => return proxy_error(StatusCode::UNAUTHORIZED, "media session unavailable"),
    };
    proxy_response_captured(state, &path, request_headers, scope).await
}

async fn proxy_response_captured(
    state: &AppState,
    path: &str,
    request_headers: &HeaderMap,
    scope: MediaReadScope,
) -> Response {
    let result = scope
        .run(async {
            let mut upstream = state
                .media_fetch_client
                .get(format!("{}{path}", scope.base))
                .timeout(std::time::Duration::from_secs(120));
            if let Some(auth) = scope.authorization().await? {
                upstream = upstream.header("authorization", auth);
            }
            if let Some(range) = request_headers.get("range") {
                upstream = upstream.header("range", range);
            }
            upstream.send().await.map_err(|e| e.to_string())
        })
        .await;
    let resp = match result {
        Ok(resp) => resp,
        Err(_) => {
            return proxy_error(
                StatusCode::BAD_GATEWAY,
                "media request failed or session ended",
            )
        }
    };
    let status = resp.status();
    let mut headers = HeaderMap::new();
    for key in [
        "content-type",
        "content-range",
        "accept-ranges",
        "content-length",
        "cache-control",
        "etag",
        "last-modified",
    ] {
        if let Some(value) = resp.headers().get(key) {
            headers.insert(key, value.clone());
        }
    }
    if scope.requires_session() {
        // Content addressing is not authorization. Do not let a webview cache
        // satisfy requests after native revocation without reaching this guard.
        headers.insert("cache-control", HeaderValue::from_static("no-store"));
        headers.remove("etag");
        headers.remove("last-modified");
    }
    if !request_headers.contains_key("range")
        && resp
            .content_length()
            .is_some_and(|len| len > MAX_PROXY_RESPONSE)
    {
        return proxy_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "response too large — use range requests for video playback",
        );
    }
    let upstream = Box::pin(resp.bytes_stream());
    let stream =
        futures_util::stream::try_unfold((scope, upstream), |(scope, mut upstream)| async move {
            let chunk = scope
                .run(async { upstream.try_next().await.map_err(|e| e.to_string()) })
                .await
                .map_err(std::io::Error::other)?;
            Ok::<_, std::io::Error>(chunk.map(|chunk| (chunk, (scope, upstream))))
        });
    (status, headers, Body::from_stream(stream)).into_response()
}

fn proxy_error(status: StatusCode, message: &'static str) -> Response {
    (status, [("cache-control", "no-store")], message).into_response()
}

/// Spawn a localhost HTTP proxy that streams media via reqwest, avoiding the
/// Tauri protocol handler's requirement to buffer the entire response into
/// `Vec<u8>`. Returns the OS-assigned port.
pub async fn spawn_media_proxy(app_handle: tauri::AppHandle) -> u16 {
    let proxy_state = ProxyState { app_handle };

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

    eprintln!("buzz-desktop: media proxy listening on 127.0.0.1:{port}");
    port
}

/// Proxy media requests through the Rust backend so they traverse the VPN tunnel.
///
/// WKWebView's networking stack bypasses the VPN tunnel, causing 403s from Cloudflare Access.
/// This handler routes `buzz-media://localhost/{path}` through reqwest, which
/// runs in the Tauri process and goes through the VPN.
pub async fn handle_buzz_media<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    request: &http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    let state = app.state::<AppState>();
    let (path, generation) = match media_path(request.uri()) {
        Ok(parsed) => parsed,
        Err(_) => return error_response(400, "invalid media URL"),
    };
    let scope = match MediaReadScope::capture_renderer(&state, generation) {
        Ok(scope) => scope,
        Err(_) => return error_response(401, "media session unavailable"),
    };
    let result = scope
        .run(async {
            let response =
                proxy_response_captured(&state, &path, request.headers(), scope.clone()).await;
            let (parts, body) = response.into_parts();
            // Enforce the buffering cap on actual bytes, including chunked responses.
            let bytes = axum::body::to_bytes(body, MAX_PROXY_RESPONSE as usize)
                .await
                .map_err(|e| e.to_string())?;
            Ok(http::Response::from_parts(parts, bytes.to_vec()))
        })
        .await;
    result.unwrap_or_else(|_| error_response(502, "media body unavailable or session ended"))
}

fn error_response(status: u16, msg: &str) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .header("cache-control", "no-store")
        .body(msg.as_bytes().to_vec())
        .unwrap_or_else(|_| {
            http::Response::builder()
                .status(500)
                .body(Vec::new())
                .unwrap()
        })
}
