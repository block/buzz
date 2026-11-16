//! Agent GIF search via the relay's GIPHY proxy.
//!
//! `buzz gifs search` posts to the relay-relative endpoint advertised in the
//! NIP-11 `gif` descriptor. The agent holds no provider credential. The relay
//! proxies GIPHY and returns only public GIF metadata.
//!
//! Sending a GIF is a normal message whose content contains the `cdn_url`
//! returned by search. There is no special send-path handling and no imeta.
//!
//! The desktop twin of this module is `normalizeGiphyGifs` in
//! `desktop/src/features/gifs/api.ts`. Both must apply the same rules.

use crate::client::BuzzClient;
use crate::error::CliError;

/// Gate: `supported_extensions` must contain this value.
const REQUIRED_EXTENSION: &str = "buzz-gif";
/// Gate: `gif.provider` must be this value.
const REQUIRED_PROVIDER: &str = "giphy";

// ---------------------------------------------------------------------------
// Safe relay-relative path validation
// ---------------------------------------------------------------------------

/// Validate that a NIP-11-advertised path is a safe relay-relative path.
///
/// Mirrors the desktop `safeRelayPath` contract in
/// `desktop/src/features/gifs/api.ts` exactly:
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
// Locale
// ---------------------------------------------------------------------------

/// Locale to send to the relay. Reads `LANG` first, falls back to `en_US`.
/// The relay derives the GIPHY `lang` code from the first subtag.
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
/// tests (which inject a synthetic document directly). Separating the pure
/// parse logic from the I/O call makes the descriptor gates directly testable
/// without a fake HTTP server.
///
/// Returns the validated relay-relative search path.
pub(crate) fn parse_gif_descriptor_info(info: &serde_json::Value) -> Result<String, CliError> {
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

    // Gate 2: `gif.provider` must be `"giphy"`.
    let gif = info.get("gif").ok_or_else(|| {
        CliError::Other("relay advertises buzz-gif but has no \"gif\" descriptor".to_string())
    })?;
    let provider = gif.get("provider").and_then(|v| v.as_str()).unwrap_or("");
    if provider != REQUIRED_PROVIDER {
        return Err(CliError::Other(format!(
            "unsupported GIF provider \"{provider}\" (only \"{REQUIRED_PROVIDER}\" is supported)"
        )));
    }

    // Gate 3: the search path must be present and pass the safe-path check.
    let search = gif
        .get("search")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if !safe_relay_path(&search) {
        return Err(CliError::Other(format!(
            "relay gif descriptor search path is not a safe relay-relative path: {search:?}"
        )));
    }

    Ok(search)
}

/// Resolve the relay's `gif` descriptor from its NIP-11 document.
///
/// Returns the validated relay-relative search path.
/// Fails with a clear `CliError` if:
/// - the relay does not advertise `buzz-gif`
/// - the provider is not `giphy`
/// - the search path is absent or fails the `safe_relay_path` check
pub(crate) async fn resolve_gif_descriptor(client: &BuzzClient) -> Result<String, CliError> {
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
/// argument. Agents paste it as-is; no further processing is needed.
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

/// Normalize the GIPHY `data` array to typed `GifEntry` records.
///
/// Mirrors `normalizeGiphyGifs` in `desktop/src/features/gifs/api.ts`:
/// - skips items unless `type` is `"gif"`, `id` and `slug` are non-empty
///   strings, and `images` is an object
/// - GIPHY dimensions are JSON strings such as `"320"`. Only strings made of
///   ASCII digits parse. JSON numbers, signs, whitespace, and units are
///   rejected.
/// - rendition fallback order for `cdn_url` (sendable asset, needs url,
///   width, height, size): `downsized`, `original`, `fixed_width`
/// - rendition fallback order for `preview_url`: `fixed_width.webp`,
///   `fixed_width.url`, `fixed_width_small.webp`, `fixed_width_small.url`,
///   then the `cdn_url` itself
/// - an item with no sendable asset is silently skipped
/// - `title` falls back to `alt_text`, then to `"GIF"`
/// - malformed envelopes (no `data` array) return an error rather than a
///   silent empty array
pub(crate) fn normalize_gif_response(raw: &str) -> Result<Vec<GifEntry>, CliError> {
    let parsed: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| CliError::Other(format!("invalid GIF search response: {e}")))?;

    // The relay forwards GIPHY's `data` array as {"data": [...]}.
    // A missing outer envelope is an error, not a silent empty list.
    let items = parsed
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            CliError::Other("GIF search response missing expected envelope data array".to_string())
        })?;

    let mut out = Vec::new();
    for item in items {
        if item.get("type").and_then(|v| v.as_str()) != Some("gif") {
            continue;
        }
        if !matches!(item.get("id").and_then(|v| v.as_str()), Some(s) if !s.is_empty()) {
            continue;
        }
        let slug = match item.get("slug").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        let images = match item.get("images").filter(|v| v.is_object()) {
            Some(i) => i,
            None => continue,
        };

        // cdn_url: downsized, then original, then fixed_width.
        let (cdn_url, width, height) = match ["downsized", "original", "fixed_width"]
            .iter()
            .find_map(|name| complete_asset(images.get(name)))
        {
            Some(a) => a,
            None => continue,
        };

        // preview_url: fixed_width.webp, fixed_width.url, fixed_width_small.webp,
        // fixed_width_small.url, then the cdn_url itself.
        let fixed_width = images.get("fixed_width");
        let fixed_width_small = images.get("fixed_width_small");
        let preview_url = complete_image(fixed_width, fixed_width.and_then(|r| r.get("webp")))
            .or_else(|| complete_image(fixed_width, fixed_width.and_then(|r| r.get("url"))))
            .or_else(|| {
                complete_image(
                    fixed_width_small,
                    fixed_width_small.and_then(|r| r.get("webp")),
                )
            })
            .or_else(|| {
                complete_image(
                    fixed_width_small,
                    fixed_width_small.and_then(|r| r.get("url")),
                )
            })
            .map(|(url, _, _)| url)
            .unwrap_or_else(|| cdn_url.clone());

        let title = trimmed_non_empty(item.get("title"))
            .or_else(|| trimmed_non_empty(item.get("alt_text")))
            .unwrap_or_else(|| "GIF".to_string());

        out.push(GifEntry {
            cdn_url,
            slug,
            title,
            width,
            height,
            preview_url: Some(preview_url),
        });
    }

    Ok(out)
}

/// Return the trimmed string value if it is a non-empty string.
fn trimmed_non_empty(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Parse a GIPHY string dimension such as `"320"` to an integer.
///
/// Mirrors `parseIntegerField` in the desktop (`/^\d+$/`). Only a JSON string
/// made of one or more ASCII digits parses. JSON numbers, signs, whitespace,
/// decimals, and units all return `None`.
fn parse_integer_field(value: Option<&serde_json::Value>) -> Option<u64> {
    let s = value?.as_str()?;
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u64>().ok()
}

/// A rendition image with a URL and positive integer dimensions.
///
/// `url` is taken from the given value (so callers can select `webp` or
/// `url`), while `width` and `height` come from the rendition itself.
/// Mirrors `completeImage` in the desktop.
fn complete_image(
    rendition: Option<&serde_json::Value>,
    url: Option<&serde_json::Value>,
) -> Option<(String, u64, u64)> {
    let rendition = rendition?;
    let url = url.and_then(|v| v.as_str()).filter(|s| !s.is_empty())?;
    let width = parse_integer_field(rendition.get("width"))?;
    let height = parse_integer_field(rendition.get("height"))?;
    if width == 0 || height == 0 {
        return None;
    }
    Some((url.to_string(), width, height))
}

/// A sendable rendition: a complete image whose `size` also parses.
///
/// Mirrors `completeAsset` in the desktop. The size value is only validated;
/// `GifEntry` does not carry it.
fn complete_asset(rendition: Option<&serde_json::Value>) -> Option<(String, u64, u64)> {
    let image = complete_image(rendition, rendition.and_then(|r| r.get("url")))?;
    parse_integer_field(rendition?.get("size"))?;
    Some(image)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// `buzz gifs search [--query <q>] [--locale <l>]`
///
/// Empty/omitted `query` returns GIPHY trending GIFs. Output is a JSON array
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
    let search_path = resolve_gif_descriptor(client).await?;
    let cid = client
        .credentials()
        .relay_scoped_customer_id(client.relay_url());
    let locale = locale.map(|l| l.to_string()).unwrap_or_else(default_locale);

    let body = serde_json::json!({
        "query": query,
        "customer_id": cid,
        "locale": locale,
    });
    let raw = client.post_json_authed(&search_path, &body).await?;
    normalize_gif_response(&raw)
}

pub async fn dispatch(cmd: crate::GifsCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        crate::GifsCmd::Search { query, locale } => {
            cmd_search(client, query.as_deref().unwrap_or(""), locale.as_deref()).await
        }
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
            "//attacker.example/search",       // protocol-relative: authority shift
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
    // default_locale
    // -----------------------------------------------------------------------

    #[test]
    fn default_locale_is_nonempty() {
        let locale = default_locale();
        assert!(!locale.is_empty());
    }

    // -----------------------------------------------------------------------
    // parse_gif_descriptor_info: production gate logic, no I/O
    // -----------------------------------------------------------------------

    #[test]
    fn descriptor_missing_extension_is_rejected() {
        let info = serde_json::json!({
            "supported_extensions": ["buzz-emoji"],
            "gif": { "provider": "giphy", "search": "/gifs/search" }
        });
        let err = parse_gif_descriptor_info(&info).unwrap_err();
        assert!(
            err.to_string().contains("buzz-gif"),
            "error must mention buzz-gif, got: {err}"
        );
    }

    #[test]
    fn descriptor_missing_gif_object_is_rejected() {
        let info = serde_json::json!({
            "supported_extensions": ["buzz-gif"]
        });
        let err = parse_gif_descriptor_info(&info).unwrap_err();
        assert!(
            err.to_string().contains("gif"),
            "error must mention the gif descriptor, got: {err}"
        );
    }

    #[test]
    fn descriptor_wrong_provider_is_rejected() {
        for provider in ["tenor", "legacy-provider", ""] {
            let info = serde_json::json!({
                "supported_extensions": ["buzz-gif"],
                "gif": { "provider": provider, "search": "/gifs/search" }
            });
            let err = parse_gif_descriptor_info(&info).unwrap_err();
            assert!(
                err.to_string().contains("unsupported GIF provider"),
                "provider {provider:?} must be rejected, got: {err}"
            );
        }
    }

    #[test]
    fn descriptor_unsafe_search_path_is_rejected() {
        for bad in ["//attacker.example/x", "/gifs/../admin", "", "gifs/search"] {
            let info = serde_json::json!({
                "supported_extensions": ["buzz-gif"],
                "gif": { "provider": "giphy", "search": bad }
            });
            let err = parse_gif_descriptor_info(&info).unwrap_err();
            assert!(
                err.to_string().contains("search path"),
                "search {bad:?} must be rejected with a search path error, got: {err}"
            );
        }
    }

    #[test]
    fn descriptor_valid_passes_with_search_only() {
        // GIPHY has no share endpoint. The descriptor carries only `search`.
        let info = serde_json::json!({
            "supported_extensions": ["buzz-gif"],
            "gif": { "provider": "giphy", "search": "/gifs/search" }
        });
        let search = parse_gif_descriptor_info(&info).unwrap();
        assert_eq!(search, "/gifs/search");
    }

    #[test]
    fn descriptor_ignores_extra_share_field() {
        // A stale `share` field must not be required or validated.
        let info = serde_json::json!({
            "supported_extensions": ["buzz-gif"],
            "gif": { "provider": "giphy", "search": "/gifs/search", "share": "//evil/x" }
        });
        let search = parse_gif_descriptor_info(&info).unwrap();
        assert_eq!(search, "/gifs/search");
    }

    // -----------------------------------------------------------------------
    // parse_integer_field: GIPHY string dimensions
    // -----------------------------------------------------------------------

    #[test]
    fn parse_integer_field_accepts_digit_strings_only() {
        let accepted = [("320", 320u64), ("0", 0), ("007", 7), ("1", 1)];
        for (input, expected) in accepted {
            let v = serde_json::Value::String(input.to_string());
            assert_eq!(
                parse_integer_field(Some(&v)),
                Some(expected),
                "{input:?} must parse"
            );
        }

        let rejected_strings = [
            "", "320px", " 320", "320 ", "+320", "-1", "3.0", "1e3", "abc",
        ];
        for input in rejected_strings {
            let v = serde_json::Value::String(input.to_string());
            assert_eq!(parse_integer_field(Some(&v)), None, "{input:?} must reject");
        }

        // JSON numbers, booleans, null, and absent values are rejected.
        assert_eq!(parse_integer_field(Some(&serde_json::json!(320))), None);
        assert_eq!(parse_integer_field(Some(&serde_json::json!(320.0))), None);
        assert_eq!(parse_integer_field(Some(&serde_json::json!(true))), None);
        assert_eq!(parse_integer_field(Some(&serde_json::Value::Null)), None);
        assert_eq!(parse_integer_field(None), None);
    }

    // -----------------------------------------------------------------------
    // normalize_gif_response
    // -----------------------------------------------------------------------

    /// Build a `{"data": [...]}` envelope from a list of GIF item values.
    fn envelope(items: Vec<serde_json::Value>) -> String {
        serde_json::json!({ "data": items }).to_string()
    }

    /// A GIPHY rendition object with string dimensions.
    fn rendition(url: &str, width: &str, height: &str, size: Option<&str>) -> serde_json::Value {
        let mut r = serde_json::json!({ "url": url, "width": width, "height": height });
        if let Some(s) = size {
            r["size"] = serde_json::Value::String(s.to_string());
        }
        r
    }

    /// A full GIPHY item with every rendition this module reads.
    fn full_item() -> serde_json::Value {
        let mut fixed_width = rendition(
            "https://media1.giphy.com/media/abc123/200w.gif",
            "200",
            "113",
            Some("30000"),
        );
        fixed_width["webp"] = serde_json::json!("https://media1.giphy.com/media/abc123/200w.webp");
        let mut fixed_width_small = rendition(
            "https://media1.giphy.com/media/abc123/100w.gif",
            "100",
            "56",
            Some("8000"),
        );
        fixed_width_small["webp"] =
            serde_json::json!("https://media1.giphy.com/media/abc123/100w.webp");
        serde_json::json!({
            "type": "gif",
            "id": "abc123",
            "slug": "ship-it-abc123",
            "title": "Ship it",
            "alt_text": "A rocket launching",
            "images": {
                "downsized": rendition("https://media1.giphy.com/media/abc123/giphy-downsized.gif", "320", "180", Some("900000")),
                "original": rendition("https://media1.giphy.com/media/abc123/giphy.gif", "640", "360", Some("3000000")),
                "fixed_width": fixed_width,
                "fixed_width_small": fixed_width_small
            }
        })
    }

    #[test]
    fn normalize_extracts_cdn_url_dims_and_webp_preview() {
        let entries = normalize_gif_response(&envelope(vec![full_item()])).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(
            e.cdn_url,
            "https://media1.giphy.com/media/abc123/giphy-downsized.gif"
        );
        assert_eq!(e.slug, "ship-it-abc123");
        assert_eq!(e.title, "Ship it");
        assert_eq!(e.width, 320);
        assert_eq!(e.height, 180);
        assert_eq!(
            e.preview_url.as_deref(),
            Some("https://media1.giphy.com/media/abc123/200w.webp")
        );
    }

    #[test]
    fn normalize_cdn_url_falls_back_downsized_original_fixed_width() {
        // No downsized: original wins.
        let mut item = full_item();
        item["images"].as_object_mut().unwrap().remove("downsized");
        let entries = normalize_gif_response(&envelope(vec![item.clone()])).unwrap();
        assert_eq!(
            entries[0].cdn_url,
            "https://media1.giphy.com/media/abc123/giphy.gif"
        );
        assert_eq!((entries[0].width, entries[0].height), (640, 360));

        // No downsized, no original: fixed_width wins.
        item["images"].as_object_mut().unwrap().remove("original");
        let entries = normalize_gif_response(&envelope(vec![item.clone()])).unwrap();
        assert_eq!(
            entries[0].cdn_url,
            "https://media1.giphy.com/media/abc123/200w.gif"
        );
        assert_eq!((entries[0].width, entries[0].height), (200, 113));

        // downsized present but missing `size` is not sendable: original wins.
        let mut item = full_item();
        item["images"]["downsized"]
            .as_object_mut()
            .unwrap()
            .remove("size");
        let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
        assert_eq!(
            entries[0].cdn_url,
            "https://media1.giphy.com/media/abc123/giphy.gif"
        );
    }

    #[test]
    fn normalize_rejects_non_digit_string_dimensions_for_sendable_asset() {
        // Each bad width on downsized makes it incomplete, so original wins.
        for bad in ["", "0", "320px", " 320", "-320", "3.5"] {
            let mut item = full_item();
            item["images"]["downsized"]["width"] = serde_json::json!(bad);
            let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
            assert_eq!(
                entries[0].cdn_url, "https://media1.giphy.com/media/abc123/giphy.gif",
                "width {bad:?} must make downsized incomplete"
            );
        }
        // A JSON number is rejected too, even though it looks valid.
        let mut item = full_item();
        item["images"]["downsized"]["width"] = serde_json::json!(320);
        let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
        assert_eq!(
            entries[0].cdn_url,
            "https://media1.giphy.com/media/abc123/giphy.gif"
        );
        // Zero height is rejected.
        let mut item = full_item();
        item["images"]["downsized"]["height"] = serde_json::json!("0");
        let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
        assert_eq!(
            entries[0].cdn_url,
            "https://media1.giphy.com/media/abc123/giphy.gif"
        );
        // A non-digit size is rejected; a "0" size is accepted (size >= 0).
        let mut item = full_item();
        item["images"]["downsized"]["size"] = serde_json::json!("900000 bytes");
        let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
        assert_eq!(
            entries[0].cdn_url,
            "https://media1.giphy.com/media/abc123/giphy.gif"
        );
        let mut item = full_item();
        item["images"]["downsized"]["size"] = serde_json::json!("0");
        let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
        assert_eq!(
            entries[0].cdn_url,
            "https://media1.giphy.com/media/abc123/giphy-downsized.gif"
        );
    }

    #[test]
    fn normalize_preview_chain() {
        // fixed_width.webp missing: fixed_width.url wins.
        let mut item = full_item();
        item["images"]["fixed_width"]
            .as_object_mut()
            .unwrap()
            .remove("webp");
        let entries = normalize_gif_response(&envelope(vec![item.clone()])).unwrap();
        assert_eq!(
            entries[0].preview_url.as_deref(),
            Some("https://media1.giphy.com/media/abc123/200w.gif")
        );

        // fixed_width gone entirely: fixed_width_small.webp wins.
        item["images"]
            .as_object_mut()
            .unwrap()
            .remove("fixed_width");
        let entries = normalize_gif_response(&envelope(vec![item.clone()])).unwrap();
        assert_eq!(
            entries[0].preview_url.as_deref(),
            Some("https://media1.giphy.com/media/abc123/100w.webp")
        );

        // fixed_width_small.webp empty string: fixed_width_small.url wins.
        item["images"]["fixed_width_small"]["webp"] = serde_json::json!("");
        let entries = normalize_gif_response(&envelope(vec![item.clone()])).unwrap();
        assert_eq!(
            entries[0].preview_url.as_deref(),
            Some("https://media1.giphy.com/media/abc123/100w.gif")
        );

        // No preview renditions at all: preview falls back to cdn_url.
        item["images"]
            .as_object_mut()
            .unwrap()
            .remove("fixed_width_small");
        let entries = normalize_gif_response(&envelope(vec![item.clone()])).unwrap();
        assert_eq!(
            entries[0].preview_url.as_deref(),
            Some(entries[0].cdn_url.as_str())
        );

        // fixed_width present but with a bad dimension: webp and url both
        // fail (dims come from the rendition), so fixed_width_small wins.
        let mut item = full_item();
        item["images"]["fixed_width"]["height"] = serde_json::json!("113px");
        let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
        assert_eq!(
            entries[0].preview_url.as_deref(),
            Some("https://media1.giphy.com/media/abc123/100w.webp")
        );
    }

    #[test]
    fn normalize_skips_item_with_no_sendable_asset() {
        // Only preview-sized renditions without `size`: nothing is sendable.
        let item = serde_json::json!({
            "type": "gif",
            "id": "x1",
            "slug": "no-asset",
            "title": "Broken",
            "images": {
                "fixed_width_still": rendition("https://media1.giphy.com/media/x1/200w_s.gif", "200", "113", None),
                "downsized": rendition("https://media1.giphy.com/media/x1/d.gif", "320", "180", None)
            }
        });
        let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
        assert!(entries.is_empty());

        // `images` absent or not an object: skipped.
        let mut item = full_item();
        item.as_object_mut().unwrap().remove("images");
        assert!(normalize_gif_response(&envelope(vec![item]))
            .unwrap()
            .is_empty());
        let mut item = full_item();
        item["images"] = serde_json::json!("nope");
        assert!(normalize_gif_response(&envelope(vec![item]))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn normalize_skips_non_gif_type_and_missing_id_or_slug() {
        let mut sticker = full_item();
        sticker["type"] = serde_json::json!("sticker");
        sticker["slug"] = serde_json::json!("sticker-slug");

        let mut no_type = full_item();
        no_type.as_object_mut().unwrap().remove("type");

        let mut no_id = full_item();
        no_id.as_object_mut().unwrap().remove("id");

        let mut empty_id = full_item();
        empty_id["id"] = serde_json::json!("");

        let mut numeric_id = full_item();
        numeric_id["id"] = serde_json::json!(123);

        let mut no_slug = full_item();
        no_slug.as_object_mut().unwrap().remove("slug");

        let mut empty_slug = full_item();
        empty_slug["slug"] = serde_json::json!("");

        let real = full_item();

        let entries = normalize_gif_response(&envelope(vec![
            sticker, no_type, no_id, empty_id, numeric_id, no_slug, empty_slug, real,
        ]))
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].slug, "ship-it-abc123");
    }

    #[test]
    fn normalize_title_falls_back_to_alt_text_then_gif() {
        // Whitespace-only title: alt_text wins (trimmed).
        let mut item = full_item();
        item["title"] = serde_json::json!("   ");
        item["alt_text"] = serde_json::json!("  A rocket launching  ");
        let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
        assert_eq!(entries[0].title, "A rocket launching");

        // Title absent and alt_text empty: "GIF".
        let mut item = full_item();
        item.as_object_mut().unwrap().remove("title");
        item["alt_text"] = serde_json::json!("");
        let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
        assert_eq!(entries[0].title, "GIF");

        // Both absent: "GIF".
        let mut item = full_item();
        item.as_object_mut().unwrap().remove("title");
        item.as_object_mut().unwrap().remove("alt_text");
        let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
        assert_eq!(entries[0].title, "GIF");

        // Non-string title: treated as absent.
        let mut item = full_item();
        item["title"] = serde_json::json!(42);
        item.as_object_mut().unwrap().remove("alt_text");
        let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
        assert_eq!(entries[0].title, "GIF");

        // Title is trimmed.
        let mut item = full_item();
        item["title"] = serde_json::json!("  Ship it  ");
        let entries = normalize_gif_response(&envelope(vec![item])).unwrap();
        assert_eq!(entries[0].title, "Ship it");
    }

    #[test]
    fn normalize_rejects_malformed_envelope() {
        // No `data` key: must error, not silently return [].
        assert!(normalize_gif_response(r#"{"gifs":[]}"#).is_err());
        // `data` is not an array.
        assert!(normalize_gif_response(r#"{"data":{"data":[]}}"#).is_err());
        assert!(normalize_gif_response(r#"{"data":null}"#).is_err());
        assert!(normalize_gif_response(r#"{"data":"x"}"#).is_err());
        // The legacy {"result", "data": {"data"}} envelope is rejected.
        assert!(normalize_gif_response(r#"{"result":true,"data":{"data":[]}}"#).is_err());
        // Top-level array is rejected.
        assert!(normalize_gif_response(r#"[]"#).is_err());
        // Invalid JSON is rejected.
        assert!(normalize_gif_response("not json").is_err());
    }

    #[test]
    fn normalize_empty_data_array_is_ok() {
        let entries = normalize_gif_response(r#"{"data":[]}"#).unwrap();
        assert!(entries.is_empty());
        // Extra GIPHY keys the relay might leave in are ignored.
        let entries =
            normalize_gif_response(r#"{"data":[],"meta":{"status":200},"pagination":{}}"#).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn normalize_non_object_items_are_skipped() {
        let raw = format!(
            r#"{{"data":[null, 1, "gif", [], {}]}}"#,
            serde_json::to_string(&full_item()).unwrap()
        );
        let entries = normalize_gif_response(&raw).unwrap();
        assert_eq!(entries.len(), 1);
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

    /// NIP-11 JSON that advertises a non-default search path.
    ///
    /// Production code must read the advertised path from NIP-11 and POST to
    /// it. Using a non-default path here means a hardcoded "/gifs/search" in
    /// production would target a 404 route and the tests would fail. This
    /// proves that the relay-advertised path is actually used.
    const ALT_SEARCH_PATH: &str = "/x/search-alt";

    fn alt_nip11() -> &'static str {
        r#"{"supported_extensions":["buzz-gif"],"gif":{"provider":"giphy","search":"/x/search-alt"}}"#
    }

    /// A simple fake relay: serves NIP-11 at `/info` advertising a non-default
    /// path, then captures POST bodies at that path.
    async fn fake_server(
        search_status: StatusCode,
        search_body: String,
    ) -> (String, Arc<Mutex<Vec<Captured>>>) {
        let captured: Arc<Mutex<Vec<Captured>>> = Arc::new(Mutex::new(Vec::new()));

        type S = (Arc<Mutex<Vec<Captured>>>, StatusCode, String);
        let state: S = (captured.clone(), search_status, search_body);

        let app = Router::new()
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
                    |State((cap, search_st, search_bd)): State<S>,
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
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), captured)
    }

    /// Client without an auth tag, used for basic NIP-98 / body / path tests.
    fn test_client(base_url: &str) -> BuzzClient {
        let keys = Keys::generate();
        BuzzClient::new(
            base_url.to_string(),
            crate::credentials::Credentials::from_keys(keys),
        )
        .unwrap()
    }

    /// Client with a synthetic `x-auth-tag`, used to assert that the header
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
            crate::credentials::Credentials::from_keys(keys).with_auth_tag(tag, tag_json.clone()),
        )
        .unwrap();
        (client, tag_json)
    }

    fn one_gif_response() -> String {
        serde_json::json!({"data":[
            {"type":"gif","id":"t1","slug":"test-slug","title":"Test","images":{
                "downsized":{"url":"https://media1.giphy.com/media/t1/giphy-downsized.gif","width":"320","height":"180","size":"50"}
            }}
        ]})
        .to_string()
    }

    // ── relay-advertised path binding ──────────────────────────────────────

    #[tokio::test]
    async fn search_posts_to_relay_advertised_path_not_hardcoded() {
        // Fake advertises ALT_SEARCH_PATH; hardcoded "/gifs/search" would 404.
        let (url, captured) = fake_server(StatusCode::OK, one_gif_response()).await;
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
        // Exactly the three contract fields, nothing else.
        assert_eq!(body.as_object().unwrap().len(), 3);
    }

    // ── x-auth-tag forwarded + NIP-98 deep assertions ──────────────────────

    #[tokio::test]
    async fn search_forwards_x_auth_tag_header() {
        let (url, captured) = fake_server(StatusCode::OK, one_gif_response()).await;
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
        let (url, captured) = fake_server(StatusCode::OK, one_gif_response()).await;
        let client = test_client(&url);

        cmd_search(&client, "cats", Some("en_US")).await.unwrap();

        let calls = captured.lock().unwrap();
        let call = calls
            .iter()
            .find(|c| c.path == ALT_SEARCH_PATH)
            .expect("search POST must arrive");

        // Decode "Nostr <base64>" to a JSON event
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

    // ── typed result through the full seam ─────────────────────────────────

    #[tokio::test]
    async fn search_entries_returns_top_level_cdn_url() {
        // Tests that cmd_search delegates to search_entries() which returns
        // typed output with cdn_url at the top level. A raw-passthrough
        // regression (no normalize_gif_response) would produce a different
        // struct shape and cdn_url would be absent.
        let (url, _) = fake_server(StatusCode::OK, one_gif_response()).await;
        let client = test_client(&url);

        let entries = search_entries(&client, "", None).await.unwrap();

        assert!(!entries.is_empty(), "must return at least one entry");
        assert_eq!(
            entries[0].cdn_url, "https://media1.giphy.com/media/t1/giphy-downsized.gif",
            "cdn_url must be the normalized top-level URL from images.downsized"
        );
        assert_eq!(entries[0].slug, "test-slug");
        assert_eq!(entries[0].width, 320);
        assert_eq!(entries[0].height, 180);
        // No fixed_width renditions: preview falls back to cdn_url.
        assert_eq!(
            entries[0].preview_url.as_deref(),
            Some("https://media1.giphy.com/media/t1/giphy-downsized.gif")
        );
    }

    #[tokio::test]
    async fn search_errors_on_legacy_nested_envelope() {
        // A relay still serving the old {"result":..,"data":{"data":[]}} shape
        // must surface an error instead of an empty list.
        let old = r#"{"result":true,"data":{"data":[]}}"#.to_string();
        let (url, _) = fake_server(StatusCode::OK, old).await;
        let client = test_client(&url);

        let err = match search_entries(&client, "", None).await {
            Ok(entries) => panic!("legacy envelope must error, got {} entries", entries.len()),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("envelope"),
            "error must mention the envelope, got: {err}"
        );
    }

    // ── negative gates ─────────────────────────────────────────────────────

    /// Serve a fixed NIP-11 body at `/info` and nothing else.
    async fn nip11_only_server(body: &'static str) -> String {
        let app = Router::new().route(
            "/info",
            axum::routing::get(move || async move {
                (
                    StatusCode::OK,
                    [("content-type", "application/nostr+json")],
                    body,
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn search_rejects_missing_extension_in_nip11() {
        let url = nip11_only_server(
            r#"{"supported_extensions":[],"gif":{"provider":"giphy","search":"/x/search-alt"}}"#,
        )
        .await;
        let client = test_client(&url);

        let err = cmd_search(&client, "test", None).await.unwrap_err();
        assert!(
            err.to_string().contains("buzz-gif"),
            "error must mention buzz-gif, got: {err}"
        );
    }

    #[tokio::test]
    async fn search_rejects_legacy_provider_in_nip11() {
        let url = nip11_only_server(
            r#"{"supported_extensions":["buzz-gif"],"gif":{"provider":"legacy-provider","search":"/x/search-alt","share":"/x/share-alt"}}"#,
        )
        .await;
        let client = test_client(&url);

        let err = cmd_search(&client, "test", None).await.unwrap_err();
        assert!(
            err.to_string().contains("unsupported GIF provider"),
            "error must reject the old provider, got: {err}"
        );
    }
}
