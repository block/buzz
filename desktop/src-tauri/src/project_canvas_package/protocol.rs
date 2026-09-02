use percent_encoding::percent_decode_str;
use tauri::http::{self, Method, StatusCode};

use super::{
    manifest::{mime_type, validate_relative_path, MAX_FILE_BYTES},
    normalized_pubkey, ActiveLoad, ProjectCanvasRuntime,
};

const PEOPLE_READ_CAPABILITY: &str = "project.people.read";

pub(super) const DOCUMENT_CSP: &str = "default-src 'none'; script-src 'self' buzz-canvas: http://buzz-canvas.localhost; style-src 'self' buzz-canvas: http://buzz-canvas.localhost; img-src 'self' buzz-canvas: http://buzz-canvas.localhost data: blob:; media-src 'self' buzz-canvas: http://buzz-canvas.localhost blob:; font-src 'self' buzz-canvas: http://buzz-canvas.localhost; connect-src 'none'; webrtc 'block'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; worker-src 'none'; frame-ancestors tauri: http://tauri.localhost http://localhost:*";
pub(super) const PERMISSIONS_POLICY: &str = "accelerometer=(), camera=(), clipboard-read=(), clipboard-write=(), display-capture=(), fullscreen=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), publickey-credentials-get=(), screen-wake-lock=(), usb=()";

pub(super) fn handle(
    runtime: &ProjectCanvasRuntime,
    request: &http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    if !cfg!(target_os = "macos") {
        return response(
            StatusCode::FORBIDDEN,
            "text/plain; charset=utf-8",
            b"sandboxed project canvases are unavailable on this platform".to_vec(),
        );
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return response(
            StatusCode::METHOD_NOT_ALLOWED,
            "text/plain; charset=utf-8",
            b"method not allowed".to_vec(),
        );
    }

    match route(runtime, request.uri().path()) {
        Ok((content_type, mut body)) => {
            if request.method() == Method::HEAD {
                body.clear();
            }
            response(StatusCode::OK, content_type, body)
        }
        Err((status, message)) => {
            response(status, "text/plain; charset=utf-8", message.into_bytes())
        }
    }
}

pub(super) fn route(
    runtime: &ProjectCanvasRuntime,
    raw_path: &str,
) -> Result<(&'static str, Vec<u8>), (StatusCode, String)> {
    let decoded = percent_decode_str(raw_path)
        .decode_utf8()
        .map_err(|_| bad_request("request path must be UTF-8"))?;
    if decoded.contains('\\') || decoded.contains('\0') {
        return Err(bad_request("invalid project canvas request path"));
    }
    let mut parts = decoded.trim_start_matches('/').split('/');
    let load_id = parts.next().unwrap_or_default();
    if uuid::Uuid::parse_str(load_id)
        .map(|id| id.simple().to_string())
        .as_deref()
        != Ok(load_id)
    {
        return Err(bad_request("invalid project canvas load id"));
    }
    let load = runtime
        .load(load_id)
        .map_err(internal_error)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "project canvas load not found".to_string(),
            )
        })?;

    let remainder: Vec<&str> = parts.collect();
    match remainder.as_slice() {
        [] | [""] | ["index.html"] => Ok(("text/html; charset=utf-8", shell())),
        ["__buzz", "bootstrap.js"] => Ok((
            "text/javascript; charset=utf-8",
            bootstrap(&load).map_err(internal_error)?,
        )),
        ["__buzz", "sdk.js"] => Ok((
            "text/javascript; charset=utf-8",
            include_str!("sdk.js").as_bytes().to_vec(),
        )),
        ["__buzz", "sdk.css"] => Ok((
            "text/css; charset=utf-8",
            include_str!("sdk.css").as_bytes().to_vec(),
        )),
        ["__buzz", "avatar", pubkey] => serve_avatar(runtime, &load, pubkey),
        ["package", rest @ ..] if !rest.is_empty() => serve_package_file(&load, rest),
        _ => Err((StatusCode::NOT_FOUND, "not found".to_string())),
    }
}

/// Serves an avatar the host published for this frame's project.
///
/// The bytes never enter the RPC port, so a people lookup stays inside its
/// message ceiling however many faces it carries, and each image is fetched
/// only when a widget actually renders it. Nothing here reaches the network: a
/// pubkey the host has not published is a 404, which the SDK leaves as
/// initials.
///
/// The project comes from the load, never from the request, so a frame can
/// only ever read avatars published for the project it is bound to.
fn serve_avatar(
    runtime: &ProjectCanvasRuntime,
    load: &ActiveLoad,
    pubkey: &str,
) -> Result<(&'static str, Vec<u8>), (StatusCode, String)> {
    if !load
        .granted_capabilities
        .iter()
        .any(|capability| capability == PEOPLE_READ_CAPABILITY)
    {
        return Err((
            StatusCode::FORBIDDEN,
            format!("project canvas package was not granted {PEOPLE_READ_CAPABILITY}"),
        ));
    }
    let pubkey = normalized_pubkey(pubkey).map_err(bad_request)?;
    let avatar = runtime
        .avatar(&load.binding, &pubkey)
        .map_err(internal_error)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "not found".to_string()))?;
    Ok((avatar.content_type, avatar.bytes.as_ref().clone()))
}

fn shell() -> Vec<u8> {
    br#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Project Canvas</title>
</head>
<body>
  <main id="canvas-root"></main>
  <script src="./__buzz/bootstrap.js"></script>
</body>
</html>
"#
    .to_vec()
}

fn bootstrap(load: &ActiveLoad) -> Result<Vec<u8>, String> {
    let nonce = serde_json::to_string(&load.nonce)
        .map_err(|error| format!("encode project canvas nonce: {error}"))?;
    // The host-owned SDK loads before every package resource so packages can
    // rely on `window.buzzCanvas.sdk` from their first statement.
    let scripts = std::iter::once(Ok("\"./__buzz/sdk.js\"".to_string()))
        .chain(
            load.manifest
                .scripts
                .iter()
                .map(|script| serde_json::to_string(&package_url(script))),
        )
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("encode project canvas script URL: {error}"))?
        .join(",");
    let styles = std::iter::once(Ok("\"./__buzz/sdk.css\"".to_string()))
        .chain(
            load.manifest
                .styles
                .iter()
                .map(|style| serde_json::to_string(&package_url(style))),
        )
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("encode project canvas style URL: {error}"))?
        .join(",");
    let script = format!(
        r#"(() => {{
  "use strict";
  const protocolVersion = 1;
  const nonce = {nonce};
  const styles = [{styles}];
  const scripts = [{scripts}];
  let connected = false;

  const connect = (event) => {{
    const message = event.data;
    if (connected || event.source !== parent || !message ||
        message.type !== "host.connect" ||
        message.protocolVersion !== protocolVersion ||
        message.nonce !== nonce || event.ports.length !== 1) {{
      return;
    }}
    connected = true;
    window.removeEventListener("message", connect);
    const port = event.ports[0];
    Object.defineProperty(window, "buzzCanvas", {{
      value: Object.freeze({{
        packageBaseUrl: new URL("./package/", location.href).href,
        protocolVersion,
        port,
        sdk: {{}},
      }}),
      configurable: false,
      enumerable: false,
      writable: false,
    }});
    for (const href of styles) {{
      const link = document.createElement("link");
      link.rel = "stylesheet";
      link.href = href;
      document.head.append(link);
    }}
    let scriptIndex = 0;
    const loadNextScript = () => {{
      if (scriptIndex >= scripts.length) return;
      const packageScript = document.createElement("script");
      packageScript.src = scripts[scriptIndex++];
      packageScript.addEventListener("load", loadNextScript, {{ once: true }});
      packageScript.addEventListener("error", () => {{
        port.postMessage({{ type: "canvas.error", protocolVersion, message: "script failed to load" }});
      }}, {{ once: true }});
      document.body.append(packageScript);
    }};
    loadNextScript();
  }};

  window.addEventListener("message", connect);
  parent.postMessage({{ type: "canvas.ready", protocolVersion, nonce }}, "*");
}})();
"#
    );
    Ok(script.into_bytes())
}

fn serve_package_file(
    load: &ActiveLoad,
    segments: &[&str],
) -> Result<(&'static str, Vec<u8>), (StatusCode, String)> {
    if segments.iter().any(|segment| segment.is_empty()) {
        return Err(bad_request("invalid project canvas package path"));
    }
    let relative = validate_relative_path(&segments.join("/"))
        .map_err(|message| (StatusCode::BAD_REQUEST, message))?;
    let content_type = mime_type(&relative).ok_or_else(|| {
        (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported file type".to_string(),
        )
    })?;
    // Active loads own the exact validated bytes. The on-disk revision is a
    // recovery cache only; reopening it here would let a same-user editor
    // mutate a supposedly immutable frame between activation and a request.
    let bytes = load
        .files
        .get(&relative)
        .cloned()
        .ok_or_else(|| (StatusCode::NOT_FOUND, "not found".to_string()))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "file too large".to_string()));
    }
    Ok((content_type, bytes))
}

fn package_url(relative: &str) -> String {
    let encoded = relative
        .split('/')
        .map(|segment| {
            percent_encoding::utf8_percent_encode(segment, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("./package/{encoded}")
}

fn response(
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
) -> http::Response<Vec<u8>> {
    let fallback = body.clone();
    http::Response::builder()
        .status(status)
        .header("content-type", content_type)
        .header("content-security-policy", DOCUMENT_CSP)
        .header("permissions-policy", PERMISSIONS_POLICY)
        .header("referrer-policy", "no-referrer")
        .header("x-content-type-options", "nosniff")
        .header("x-dns-prefetch-control", "off")
        .header("cache-control", "no-store")
        .body(body)
        .unwrap_or_else(|_| http::Response::new(fallback))
}

fn bad_request(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, message.into())
}

fn internal_error(message: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, message.to_string())
}
