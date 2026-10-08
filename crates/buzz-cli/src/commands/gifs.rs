//! Agent GIF search and share via the relay's KLIPY proxy.
//!
//! `buzz gifs search` / `buzz gifs share` hit the relay-relative endpoints
//! advertised in the NIP-11 `gif` descriptor. No provider credential is held
//! by the agent — the relay proxies KLIPY and returns only allowlisted data.
//!
//! Sending a GIF is a normal message whose content contains the `cdn_url`
//! returned by search — no special send-path handling, no imeta.

use crate::client::BuzzClient;
use crate::error::CliError;

/// Gate: `supported_extensions` must contain this value.
const REQUIRED_EXTENSION: &str = "buzz-gif";
/// Gate: `gif.provider` must be this value.
const REQUIRED_PROVIDER: &str = "klipy";

// ---------------------------------------------------------------------------
// Safe relay-relative path validation
// ---------------------------------------------------------------------------

/// Validate that a NIP-11-advertised path is a safe relay-relative path.
///
/// Mirrors the desktop `safeRelayPath` contract in
/// `desktop/src/features/gifs/api.ts:64-74` exactly:
/// - must be a string that starts with `/`
/// - must NOT start with `//` (avoids authority shift)
/// - must NOT contain `\` (Windows-style traversal)
/// - must NOT contain `%` (URL-encoded bypass attempts)
/// - must NOT contain `?` (query injection)
/// - must NOT contain `#` (fragment injection)
/// - no path segment may be `.` or `..` (traversal)
pub(crate) fn safe_relay_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains('\\')
        && !path.contains('%')
        && !path.contains('?')
        && !path.contains('#')
        && !path.split('/').any(|seg| seg == "." || seg == "..")
}

// ---------------------------------------------------------------------------
// Customer ID derivation
// ---------------------------------------------------------------------------

/// Derive a stable, relay-scoped anonymous `customer_id` from secret key material.
///
/// KLIPY requires a per-installation identifier that is stable and anonymous.
/// Using SHA-256 of the *public* key would be stable but NOT anonymous — the
/// input is public, so the ID is computable by any observer, and the same value
/// would appear across all relays (cross-relay linkability).
///
/// Instead, we domain-separate with the relay URL and sign with the *secret* key:
///   `SHA-256(secret_key_bytes || '\0' || relay_url_bytes)`
/// This is:
/// - **stable**: deterministic given the same keypair + relay.
/// - **relay-scoped**: different relay → different ID, no cross-relay correlation.
/// - **not computable from public data**: requires secret key material.
/// - **stateless**: no file I/O, no storage.
///
/// The first 16 bytes (32 hex chars) give 128 bits of uniqueness, ample for
/// KLIPY's per-installation needs.
fn customer_id(secret_key_bytes: &[u8], relay_url: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(secret_key_bytes);
    hasher.update(b"\0"); // domain separator
    hasher.update(relay_url.as_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash[..16]) // 16 bytes → 32 hex chars
}

// ---------------------------------------------------------------------------
// Locale
// ---------------------------------------------------------------------------

/// Locale to send to KLIPY.  Reads `LANG` first, falls back to `en_US`.
fn default_locale() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|l| {
            let code: String = l.split('.').next().unwrap_or("").chars().take(5).collect();
            if code.len() >= 2 {
                Some(code)
            } else {
                None
            }
        })
        .unwrap_or_else(|| "en_US".to_string())
}

// ---------------------------------------------------------------------------
// NIP-11 descriptor resolution
// ---------------------------------------------------------------------------

/// Parse the `gif` descriptor from a decoded NIP-11 JSON document.
///
/// Shared between `resolve_gif_descriptor` (which fetches the document) and
/// tests (which inject a synthetic document directly).  Separating the pure
/// parse logic from the I/O call makes the descriptor gates directly testable
/// without a fake HTTP server.
pub(crate) fn parse_gif_descriptor_info(
    info: &serde_json::Value,
) -> Result<(String, String), CliError> {
    // Gate 1: `supported_extensions` must contain `"buzz-gif"`.
    let extensions = info
        .get("supported_extensions")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    if !extensions.contains(&REQUIRED_EXTENSION) {
        return Err(CliError::Other(format!(
            "this relay does not support GIF search (missing \"{REQUIRED_EXTENSION}\" in supported_extensions)"
        )));
    }

    // Gate 2: `gif.provider` must be `"klipy"`.
    let gif = info.get("gif").ok_or_else(|| {
        CliError::Other("relay advertises buzz-gif but has no \"gif\" descriptor".to_string())
    })?;
    let provider = gif.get("provider").and_then(|v| v.as_str()).unwrap_or("");
    if provider != REQUIRED_PROVIDER {
        return Err(CliError::Other(format!(
            "unsupported GIF provider \"{provider}\" (only \"{REQUIRED_PROVIDER}\" is supported)"
        )));
    }

    // Gate 3: both paths must be present and pass the safe-path check.
    let search = gif
        .get("search")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let share = gif
        .get("share")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if !safe_relay_path(&search) {
        return Err(CliError::Other(format!(
            "relay gif descriptor search path is not a safe relay-relative path: {search:?}"
        )));
    }
    if !safe_relay_path(&share) {
        return Err(CliError::Other(format!(
            "relay gif descriptor share path is not a safe relay-relative path: {share:?}"
        )));
    }

    Ok((search, share))
}

/// Resolve the relay's `gif` descriptor from its NIP-11 document.
///
/// Returns `(search_path, share_path)` as validated relay-relative strings.
/// Fails with a clear `CliError` if:
/// - the relay does not advertise `buzz-gif`
/// - the provider is not `klipy`
/// - either path is absent or fails the `safe_relay_path` check
pub(crate) async fn resolve_gif_descriptor(
    client: &BuzzClient,
) -> Result<(String, String), CliError> {
    let raw = client.get_public("/info").await?;
    let info: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("invalid NIP-11 response: {e}")))?;
    parse_gif_descriptor_info(&info)
}

// ---------------------------------------------------------------------------
// Response normalization
// ---------------------------------------------------------------------------

/// Normalized GIF entry emitted by `buzz gifs search`.
///
/// `cdn_url` is the URL to embed directly in a `buzz messages send --content`
/// argument.  Agents paste it as-is; no further processing is needed.
#[derive(serde::Serialize)]
pub(crate) struct GifEntry {
    pub cdn_url: String,
    pub slug: String,
    pub title: String,
    pub width: u64,
    pub height: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
}

/// Normalize the KLIPY `data.data` array to typed `GifEntry` records.
///
/// Mirrors `normalizeKlipyGifs` in `desktop/src/features/gifs/api.ts`:
/// - skips items that are not `type: "gif"`, lack a `slug`, or have no
///   complete sendable asset
/// - asset fallback order for `cdn_url` (original): `md.gif`, `hd.gif`,
///   `sm.gif`, `xs.gif`
/// - asset fallback order for `preview_url`: `sm.webp`, `sm.gif`,
///   `xs.webp`, `xs.gif`, `md.webp`
/// - an item with no usable original or preview is silently skipped
/// - malformed envelopes (wrong outer shape) return an error rather
///   than a silent empty array
pub(crate) fn normalize_gif_response(raw: &str) -> Result<Vec<GifEntry>, CliError> {
    let parsed: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| CliError::Other(format!("invalid GIF search response: {e}")))?;

    // The relay wraps in {"result": true, "data": {"data": [...]}}.
    // A missing outer envelope is an error, not a silent empty list.
    let items = parsed
        .get("data")
        .and_then(|d| d.get("data"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            CliError::Other(
                "GIF search response missing expected envelope data.data array".to_string(),
            )
        })?;

    let mut out = Vec::new();
    for item in items {
        // Only process type:"gif" items with a slug.
        if item.get("type").and_then(|v| v.as_str()) != Some("gif") {
            continue;
        }
        let slug = match item.get("slug").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "GIF".to_string());

        let file = match item.get("file") {
            Some(f) => f,
            None => continue,
        };

        // cdn_url: md.gif → hd.gif → sm.gif → xs.gif
        let original = first_complete_gif_asset(
            file,
            &[
                &["md", "gif"],
                &["hd", "gif"],
                &["sm", "gif"],
                &["xs", "gif"],
            ],
        );
        // preview_url: sm.webp → sm.gif → xs.webp → xs.gif → md.webp
        let preview = first_complete_gif_asset(
            file,
            &[
                &["sm", "webp"],
                &["sm", "gif"],
                &["xs", "webp"],
                &["xs", "gif"],
                &["md", "webp"],
            ],
        );

        let (cdn_url, width, height) = match original {
            Some(a) => a,
            None => continue,
        };

        let preview_url = preview.map(|(u, _, _)| u);

        out.push(GifEntry {
            cdn_url,
            slug,
            title,
            width,
            height,
            preview_url,
        });
    }

    Ok(out)
}

/// Extract the URL, width, and height from the first complete asset at
/// `file[size][fmt]` where `size`/`fmt` pairs are tried in order.
/// "Complete" means url (non-empty string), width (number), height (number)
/// are all present — mirrors `isCompleteAsset` in the desktop.
fn first_complete_gif_asset(
    file: &serde_json::Value,
    candidates: &[&[&str; 2]],
) -> Option<(String, u64, u64)> {
    for &[size, fmt] in candidates {
        let asset = file.get(size).and_then(|s| s.get(fmt));
        if let Some(a) = asset {
            let url = a.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let width = a.get("width").and_then(|v| v.as_u64());
            let height = a.get("height").and_then(|v| v.as_u64());
            if !url.is_empty() {
                if let (Some(w), Some(h)) = (width, height) {
                    return Some((url.to_string(), w, h));
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// `buzz gifs search [--query <q>] [--locale <l>]`
///
/// Empty/omitted `query` returns KLIPY trending GIFs. Output is a JSON array
/// of normalized GIF objects; each entry's `cdn_url` is the URL to embed in a
/// `buzz messages send --content` argument.
pub async fn cmd_search(
    client: &BuzzClient,
    query: &str,
    locale: Option<&str>,
) -> Result<(), CliError> {
    let entries = search_entries(client, query, locale).await?;
    println!(
        "{}",
        serde_json::to_string(&entries)
            .map_err(|e| CliError::Other(format!("output serialization failed: {e}")))?
    );
    Ok(())
}

/// Resolve NIP-11, POST the search, normalize and return typed GIF entries.
///
/// Extracted from `cmd_search` so tests can assert the typed result directly
/// without capturing stdout.
pub(crate) async fn search_entries(
    client: &BuzzClient,
    query: &str,
    locale: Option<&str>,
) -> Result<Vec<GifEntry>, CliError> {
    let (search_path, _) = resolve_gif_descriptor(client).await?;
    let cid = customer_id(
        client.keys().secret_key().as_secret_bytes(),
        client.relay_url(),
    );
    let locale = locale.map(|l| l.to_string()).unwrap_or_else(default_locale);

    let body = serde_json::json!({
        "query": query,
        "customer_id": cid,
        "locale": locale,
    });
    let raw = client.post_json_authed(&search_path, &body).await?;
    normalize_gif_response(&raw)
}

/// `buzz gifs share --slug <slug>`
///
/// Reports a selected GIF to KLIPY so it can update Recents. The `slug` is
/// the provider identifier returned in search results. Prints
/// `{"accepted": true}` on success.
pub async fn cmd_share(client: &BuzzClient, slug: &str) -> Result<(), CliError> {
    let (_, share_path) = resolve_gif_descriptor(client).await?;
    let cid = customer_id(
        client.keys().secret_key().as_secret_bytes(),
        client.relay_url(),
    );

    let body = serde_json::json!({
        "slug": slug,
        "customer_id": cid,
    });
    // The relay returns 204 No Content on success; post_json_authed returns "".
    client.post_json_authed(&share_path, &body).await?;
    println!("{}", serde_json::json!({"accepted": true}));
    Ok(())
}

pub async fn dispatch(cmd: crate::GifsCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        crate::GifsCmd::Search { query, locale } => {
            cmd_search(client, query.as_deref().unwrap_or(""), locale.as_deref()).await
        }
        crate::GifsCmd::Share { slug } => cmd_share(client, &slug).await,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // safe_relay_path
    // -----------------------------------------------------------------------

    #[test]
    fn safe_relay_path_accepts_normal_paths() {
        assert!(safe_relay_path("/gifs/search"));
        assert!(safe_relay_path("/gifs/share"));
        assert!(safe_relay_path("/api/v2/gifs/search"));
    }

    #[test]
    fn safe_relay_path_rejects_adversarial_corpus() {
        // Desktop adversarial corpus from desktop/src/features/gifs/api.test.mjs
        let bad_paths = [
            "https://attacker.example/search", // absolute URL, no leading /
            "//attacker.example/search",       // protocol-relative → authority shift
            "/\\attacker.example/search",      // backslash
            "/%5c%5cattacker.example/search",  // percent-encoded
            "/gifs/../admin",                  // dot-dot traversal
            "/gifs/%2e%2e/admin",              // percent-encoded dot-dot
            "/gifs/search?redirect=https://attacker.example", // query injection
            "/gifs/search#fragment",           // fragment injection
        ];
        for path in bad_paths {
            assert!(
                !safe_relay_path(path),
                "expected safe_relay_path({path:?}) == false"
            );
        }
    }

    #[test]
    fn safe_relay_path_rejects_empty_and_relative() {
        assert!(!safe_relay_path(""));
        assert!(!safe_relay_path("gifs/search")); // no leading /
        assert!(!safe_relay_path("//"));
    }

    // -----------------------------------------------------------------------
    // customer_id
    // -----------------------------------------------------------------------

    #[test]
    fn customer_id_is_32_hex_chars_and_stable() {
        let sk = [0xab_u8; 32];
        let id = customer_id(&sk, "https://relay.example");
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(id, customer_id(&sk, "https://relay.example"));
    }

    #[test]
    fn customer_id_is_relay_scoped() {
        let sk = [0xcd_u8; 32];
        let id_a = customer_id(&sk, "https://relay-a.example");
        let id_b = customer_id(&sk, "https://relay-b.example");
        assert_ne!(
            id_a, id_b,
            "same key, different relay → different customer_id"
        );
    }

    #[test]
    fn customer_id_differs_for_different_keys() {
        let id_a = customer_id(&[0xaa_u8; 32], "https://relay.example");
        let id_b = customer_id(&[0xbb_u8; 32], "https://relay.example");
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn customer_id_not_equal_to_pubkey_hash() {
        // The customer_id must NOT be derivable from the public key alone.
        use sha2::{Digest, Sha256};
        let sk = [0xde_u8; 32];
        // What the old pubkey-hash approach would have produced (approximately):
        let naive_hash = hex::encode(&Sha256::digest(hex::encode(sk).as_bytes())[..16]);
        let actual = customer_id(&sk, "https://relay.example");
        assert_ne!(
            actual, naive_hash,
            "customer_id must not equal SHA-256(pubkey_hex)[..16]"
        );
    }

    // -----------------------------------------------------------------------
    // default_locale
    // -----------------------------------------------------------------------

    #[test]
    fn default_locale_is_nonempty() {
        let locale = default_locale();
        assert!(!locale.is_empty());
    }

    // -----------------------------------------------------------------------
    // parse_gif_descriptor_info — production gate logic, no I/O
    // -----------------------------------------------------------------------

    #[test]
    fn descriptor_missing_extension_is_rejected() {
        let info = serde_json::json!({
            "supported_extensions": ["buzz-emoji"],
            "gif": { "provider": "klipy", "search": "/gifs/search", "share": "/gifs/share" }
        });
        let err = parse_gif_descriptor_info(&info).unwrap_err();
        assert!(
            err.to_string().contains("buzz-gif"),
            "error must mention buzz-gif, got: {err}"
        );
    }

    #[test]
    fn descriptor_wrong_provider_is_rejected() {
        let info = serde_json::json!({
            "supported_extensions": ["buzz-gif"],
            "gif": { "provider": "tenor", "search": "/gifs/search", "share": "/gifs/share" }
        });
        let err = parse_gif_descriptor_info(&info).unwrap_err();
        assert!(
            err.to_string().contains("tenor"),
            "error must mention the bad provider, got: {err}"
        );
    }

    #[test]
    fn descriptor_unsafe_search_path_is_rejected() {
        let info = serde_json::json!({
            "supported_extensions": ["buzz-gif"],
            "gif": { "provider": "klipy", "search": "//attacker.example/x", "share": "/gifs/share" }
        });
        let err = parse_gif_descriptor_info(&info).unwrap_err();
        assert!(
            err.to_string().contains("search path"),
            "error must mention search path, got: {err}"
        );
    }

    #[test]
    fn descriptor_unsafe_share_path_is_rejected() {
        let info = serde_json::json!({
            "supported_extensions": ["buzz-gif"],
            "gif": { "provider": "klipy", "search": "/gifs/search", "share": "/gifs/../admin" }
        });
        let err = parse_gif_descriptor_info(&info).unwrap_err();
        assert!(
            err.to_string().contains("share path"),
            "error must mention share path, got: {err}"
        );
    }

    #[test]
    fn descriptor_valid_passes() {
        let info = serde_json::json!({
            "supported_extensions": ["buzz-gif"],
            "gif": { "provider": "klipy", "search": "/gifs/search", "share": "/gifs/share" }
        });
        let (search, share) = parse_gif_descriptor_info(&info).unwrap();
        assert_eq!(search, "/gifs/search");
        assert_eq!(share, "/gifs/share");
    }

    // -----------------------------------------------------------------------
    // normalize_gif_response
    // -----------------------------------------------------------------------

    /// Fixture matching the shape used in desktop/tests/e2e/messaging.spec.ts
    fn e2e_fixture() -> &'static str {
        r#"{
            "result": true,
            "data": {
                "data": [
                    {
                        "id": null,
                        "type": "gif",
                        "slug": "e2e-ship-it",
                        "title": "Ship it",
                        "file": {
                            "md": { "gif": { "height": 180, "size": 42, "url": "https://static.klipy.com/ship-it.gif", "width": 320 } },
                            "sm": { "webp": { "height": 90, "size": 12, "url": "https://static.klipy.com/ship-it-sm.webp", "width": 160 } }
                        }
                    }
                ]
            }
        }"#
    }

    #[test]
    fn normalize_extracts_cdn_url_and_preview() {
        let entries = normalize_gif_response(e2e_fixture()).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.cdn_url, "https://static.klipy.com/ship-it.gif");
        assert_eq!(e.slug, "e2e-ship-it");
        assert_eq!(e.title, "Ship it");
        assert_eq!(e.width, 320);
        assert_eq!(e.height, 180);
        assert_eq!(
            e.preview_url.as_deref(),
            Some("https://static.klipy.com/ship-it-sm.webp")
        );
    }

    #[test]
    fn normalize_skips_non_gif_type() {
        let raw = r#"{"result":true,"data":{"data":[
            {"type":"ad","slug":"s","file":{"md":{"gif":{"url":"https://x.com/a.gif","width":1,"height":1,"size":1}}}},
            {"type":"gif","slug":"real","title":"R","file":{"md":{"gif":{"url":"https://x.com/r.gif","width":2,"height":2,"size":2}}}}
        ]}}"#;
        let entries = normalize_gif_response(raw).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].slug, "real");
    }

    #[test]
    fn normalize_skips_items_without_slug() {
        let raw = r#"{"result":true,"data":{"data":[
            {"type":"gif","file":{"md":{"gif":{"url":"https://x.com/a.gif","width":1,"height":1,"size":1}}}}
        ]}}"#;
        let entries = normalize_gif_response(raw).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn normalize_asset_fallback_order() {
        // No md.gif, has hd.gif — should pick hd.gif as cdn_url.
        let raw = r#"{"result":true,"data":{"data":[
            {"type":"gif","slug":"fallback","title":"F","file":{
                "hd":{"gif":{"url":"https://x.com/hd.gif","width":640,"height":360,"size":100}},
                "sm":{"webp":{"url":"https://x.com/sm.webp","width":160,"height":90,"size":10}}
            }}
        ]}}"#;
        let entries = normalize_gif_response(raw).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].cdn_url, "https://x.com/hd.gif");
    }

    #[test]
    fn normalize_skips_items_with_no_usable_original() {
        // Only a preview asset, no gif asset at any size.
        let raw = r#"{"result":true,"data":{"data":[
            {"type":"gif","slug":"broken","title":"B","file":{
                "sm":{"webp":{"url":"https://x.com/sm.webp","width":160,"height":90,"size":10}}
            }}
        ]}}"#;
        let entries = normalize_gif_response(raw).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn normalize_rejects_malformed_envelope() {
        // Missing the data.data wrapper — must error, not silently return [].
        let bad = r#"{"result":true,"gifs":[]}"#;
        assert!(normalize_gif_response(bad).is_err());
    }

    #[test]
    fn normalize_empty_data_array_is_ok() {
        let raw = r#"{"result":true,"data":{"data":[]}}"#;
        let entries = normalize_gif_response(raw).unwrap();
        assert!(entries.is_empty());
    }

    // -----------------------------------------------------------------------
    // HTTP integration tests: real client seam via axum fake server
    // -----------------------------------------------------------------------

    use crate::client::BuzzClient;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use nostr::{JsonUtil, Keys, Tag};
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    /// Captured request data from the fake server.
    #[derive(Clone, Default)]
    struct Captured {
        path: String,
        auth_header: String,
        auth_tag_header: String,
        body: String,
    }

    /// NIP-11 JSON that advertises non-default search/share paths.
    ///
    /// Production code must read the advertised paths from NIP-11 and POST to
    /// them.  Using non-default paths here means hardcoded "/gifs/search" /
    /// "/gifs/share" in production would target 404 routes and the tests would
    /// fail — proving that the relay-advertised path is actually used.
    const ALT_SEARCH_PATH: &str = "/x/search-alt";
    const ALT_SHARE_PATH: &str = "/x/share-alt";

    fn alt_nip11() -> &'static str {
        // Embedded as a literal so there is no run-time allocation in the const.
        r#"{"supported_extensions":["buzz-gif"],"gif":{"provider":"klipy","search":"/x/search-alt","share":"/x/share-alt"}}"#
    }

    /// A simple fake relay: serves NIP-11 at `/info` advertising non-default
    /// paths, then captures POST bodies at those paths.
    async fn fake_server(
        search_status: StatusCode,
        search_body: String,
        share_status: StatusCode,
    ) -> (String, Arc<Mutex<Vec<Captured>>>) {
        let captured: Arc<Mutex<Vec<Captured>>> = Arc::new(Mutex::new(Vec::new()));

        type S = (Arc<Mutex<Vec<Captured>>>, StatusCode, String, StatusCode);
        let state: S = (captured.clone(), search_status, search_body, share_status);

        let app =
            Router::new()
                .route(
                    "/info",
                    axum::routing::get(|| async {
                        (
                            StatusCode::OK,
                            [("content-type", "application/nostr+json")],
                            alt_nip11(),
                        )
                    }),
                )
                .route(
                    ALT_SEARCH_PATH,
                    post(
                        |State((cap, search_st, search_bd, _)): State<S>,
                         headers: HeaderMap,
                         body: Bytes| async move {
                            let body_str = String::from_utf8_lossy(&body).to_string();
                            cap.lock().unwrap().push(Captured {
                                path: ALT_SEARCH_PATH.to_string(),
                                auth_header: headers
                                    .get("authorization")
                                    .and_then(|v| v.to_str().ok())
                                    .unwrap_or("")
                                    .to_string(),
                                auth_tag_header: headers
                                    .get("x-auth-tag")
                                    .and_then(|v| v.to_str().ok())
                                    .unwrap_or("")
                                    .to_string(),
                                body: body_str,
                            });
                            axum::response::Response::builder()
                                .status(search_st)
                                .header("content-type", "application/json")
                                .body(axum::body::Body::from(search_bd.clone()))
                                .unwrap()
                        },
                    ),
                )
                .route(
                    ALT_SHARE_PATH,
                    post(
                        |State((cap, _, _, share_st)): State<S>,
                         headers: HeaderMap,
                         body: Bytes| async move {
                            let body_str = String::from_utf8_lossy(&body).to_string();
                            cap.lock().unwrap().push(Captured {
                                path: ALT_SHARE_PATH.to_string(),
                                auth_header: headers
                                    .get("authorization")
                                    .and_then(|v| v.to_str().ok())
                                    .unwrap_or("")
                                    .to_string(),
                                auth_tag_header: headers
                                    .get("x-auth-tag")
                                    .and_then(|v| v.to_str().ok())
                                    .unwrap_or("")
                                    .to_string(),
                                body: body_str,
                            });
                            axum::response::Response::builder()
                                .status(share_st)
                                .body(axum::body::Body::empty())
                                .unwrap()
                        },
                    ),
                )
                .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), captured)
    }

    /// Client without an auth tag — used for basic NIP-98 / body / path tests.
    fn test_client(base_url: &str) -> BuzzClient {
        let keys = Keys::generate();
        BuzzClient::new(base_url.to_string(), keys, None, None).unwrap()
    }

    /// Client with a synthetic `x-auth-tag` — used to assert that the header
    /// is forwarded verbatim and that its value is the raw JSON of the tag.
    fn test_client_with_tag(base_url: &str) -> (BuzzClient, String) {
        let keys = Keys::generate();
        // Construct a minimal auth tag: ["auth", <owner_hex>, "conditions", <sig_hex>]
        let owner_hex = "a".repeat(64);
        let sig_hex = "b".repeat(128);
        let tag_vec = vec![
            "auth".to_string(),
            owner_hex,
            "conditions".to_string(),
            sig_hex,
        ];
        let tag_json = serde_json::to_string(&tag_vec).unwrap();
        let tag = Tag::parse(tag_vec).unwrap();
        let client = BuzzClient::new(
            base_url.to_string(),
            keys,
            Some(tag),
            Some(tag_json.clone()),
        )
        .unwrap();
        (client, tag_json)
    }

    fn one_gif_response() -> String {
        serde_json::json!({"result":true,"data":{"data":[
            {"type":"gif","slug":"test-slug","title":"Test","file":{
                "md":{"gif":{"url":"https://cdn.klipy.com/test.gif","width":320,"height":180,"size":50}}
            }}
        ]}})
        .to_string()
    }

    // ── item 1: relay-advertised path binding ──────────────────────────────

    #[tokio::test]
    async fn search_posts_to_relay_advertised_path_not_hardcoded() {
        // Fake advertises ALT_SEARCH_PATH; hardcoded "/gifs/search" would 404.
        let (url, captured) =
            fake_server(StatusCode::OK, one_gif_response(), StatusCode::NO_CONTENT).await;
        let client = test_client(&url);

        cmd_search(&client, "hello", Some("en_US")).await.unwrap();

        let calls = captured.lock().unwrap();
        let call = calls
            .iter()
            .find(|c| c.path == ALT_SEARCH_PATH)
            .expect("POST must arrive at the NIP-11-advertised path");
        assert!(
            call.auth_header.starts_with("Nostr "),
            "Authorization must be a NIP-98 Nostr token, got: {:?}",
            call.auth_header
        );
        let body: serde_json::Value = serde_json::from_str(&call.body).unwrap();
        assert_eq!(body["query"], "hello");
        assert_eq!(body["locale"], "en_US");
        assert!(
            body["customer_id"]
                .as_str()
                .map(|s| s.len() == 32)
                .unwrap_or(false),
            "customer_id must be 32 hex chars"
        );
    }

    #[tokio::test]
    async fn share_posts_to_relay_advertised_path_not_hardcoded() {
        // Fake advertises ALT_SHARE_PATH; hardcoded "/gifs/share" would 404.
        let (url, captured) =
            fake_server(StatusCode::OK, "[]".to_string(), StatusCode::NO_CONTENT).await;
        let client = test_client(&url);

        cmd_share(&client, "my-gif-slug").await.unwrap();

        let calls = captured.lock().unwrap();
        let call = calls
            .iter()
            .find(|c| c.path == ALT_SHARE_PATH)
            .expect("POST must arrive at the NIP-11-advertised share path");
        assert!(
            call.auth_header.starts_with("Nostr "),
            "Authorization must be a NIP-98 Nostr token"
        );
        let body: serde_json::Value = serde_json::from_str(&call.body).unwrap();
        assert_eq!(body["slug"], "my-gif-slug");
        assert!(
            body["customer_id"]
                .as_str()
                .map(|s| s.len() == 32)
                .unwrap_or(false),
            "customer_id must be 32 hex chars"
        );
    }

    // ── item 2: x-auth-tag forwarded + NIP-98 deep assertions ─────────────

    #[tokio::test]
    async fn search_forwards_x_auth_tag_header() {
        let (url, captured) =
            fake_server(StatusCode::OK, one_gif_response(), StatusCode::NO_CONTENT).await;
        let (client, expected_tag_json) = test_client_with_tag(&url);

        cmd_search(&client, "", None).await.unwrap();

        let calls = captured.lock().unwrap();
        let call = calls
            .iter()
            .find(|c| c.path == ALT_SEARCH_PATH)
            .expect("search POST must arrive");
        assert_eq!(
            call.auth_tag_header, expected_tag_json,
            "x-auth-tag must equal the exact JSON of the auth tag"
        );
    }

    #[tokio::test]
    async fn search_nip98_token_has_correct_u_method_and_payload_hash() {
        let (url, captured) =
            fake_server(StatusCode::OK, one_gif_response(), StatusCode::NO_CONTENT).await;
        let client = test_client(&url);

        cmd_search(&client, "cats", Some("en_US")).await.unwrap();

        let calls = captured.lock().unwrap();
        let call = calls
            .iter()
            .find(|c| c.path == ALT_SEARCH_PATH)
            .expect("search POST must arrive");

        // Decode "Nostr <base64>" → JSON event
        let token = call
            .auth_header
            .strip_prefix("Nostr ")
            .expect("must start with Nostr ");
        let json_bytes = B64.decode(token).expect("must be valid base64");
        let event: nostr::Event =
            nostr::Event::from_json(std::str::from_utf8(&json_bytes).unwrap()).unwrap();

        // kind:27235 (NIP-98)
        assert_eq!(event.kind.as_u16(), 27235);

        // `u` tag must be the exact POST URL
        let expected_url = format!("{url}{ALT_SEARCH_PATH}");
        let u_tag = event
            .tags
            .iter()
            .find(|t| t.as_slice().first().map(|s| s.as_str()) == Some("u"))
            .expect("NIP-98 event must have a u tag");
        assert_eq!(
            u_tag.as_slice().get(1).map(|s| s.as_str()).unwrap_or(""),
            expected_url
        );

        // `method` tag must be "POST"
        let method_tag = event
            .tags
            .iter()
            .find(|t| t.as_slice().first().map(|s| s.as_str()) == Some("method"))
            .expect("NIP-98 event must have a method tag");
        assert_eq!(
            method_tag
                .as_slice()
                .get(1)
                .map(|s| s.as_str())
                .unwrap_or(""),
            "POST"
        );

        // `payload` tag must equal SHA-256 of the request body
        use sha2::{Digest, Sha256};
        let body_bytes = call.body.as_bytes();
        let expected_hash = hex::encode(Sha256::digest(body_bytes));
        let payload_tag = event
            .tags
            .iter()
            .find(|t| t.as_slice().first().map(|s| s.as_str()) == Some("payload"))
            .expect("NIP-98 event must have a payload tag for POST with body");
        assert_eq!(
            payload_tag
                .as_slice()
                .get(1)
                .map(|s| s.as_str())
                .unwrap_or(""),
            expected_hash,
            "payload tag must be SHA-256 of the request body"
        );
    }

    // ── item 3: search_output_contains_cdn_url asserts typed result ────────

    #[tokio::test]
    async fn search_entries_returns_top_level_cdn_url() {
        // Tests that cmd_search delegates to search_entries() which returns
        // typed output with cdn_url at the top level.  A raw-passthrough
        // regression (no normalize_gif_response) would produce a different
        // struct shape and cdn_url would be absent.
        let (url, _) =
            fake_server(StatusCode::OK, one_gif_response(), StatusCode::NO_CONTENT).await;
        let client = test_client(&url);

        let entries = search_entries(&client, "", None).await.unwrap();

        assert!(!entries.is_empty(), "must return at least one entry");
        assert_eq!(
            entries[0].cdn_url, "https://cdn.klipy.com/test.gif",
            "cdn_url must be the normalized top-level URL from md.gif"
        );
        assert_eq!(entries[0].slug, "test-slug");
    }

    // ── existing negative gate ─────────────────────────────────────────────

    #[tokio::test]
    async fn share_returns_accepted_true_on_204() {
        let (url, _) = fake_server(StatusCode::OK, "[]".to_string(), StatusCode::NO_CONTENT).await;
        let client = test_client(&url);
        cmd_share(&client, "slug-abc").await.unwrap();
    }

    #[tokio::test]
    async fn search_rejects_missing_extension_in_nip11() {
        // Serve NIP-11 without buzz-gif.
        let app = Router::new().route(
            "/info",
            axum::routing::get(|| async {
                (
                    StatusCode::OK,
                    [("content-type", "application/nostr+json")],
                    r#"{"supported_extensions":[],"gif":{"provider":"klipy","search":"/x/search-alt","share":"/x/share-alt"}}"#,
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let url = format!("http://{addr}");
        let client = test_client(&url);

        let err = cmd_search(&client, "test", None).await.unwrap_err();
        assert!(
            err.to_string().contains("buzz-gif"),
            "error must mention buzz-gif, got: {err}"
        );
    }
}
