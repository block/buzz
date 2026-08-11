use std::{io::Cursor, net::IpAddr, time::Duration};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use futures_util::StreamExt;
use image::ImageDecoder;
use reqwest::{
    header::{ACCEPT, CONTENT_TYPE, LOCATION, USER_AGENT},
    redirect::Policy,
};
use serde::Serialize;
use url::Url;

#[path = "link_preview_rate_limit.rs"]
mod rate_limit;
#[path = "link_preview_youtube.rs"]
mod youtube;

use rate_limit::{image_host_cooldown_remaining, retry_after_duration, set_image_host_cooldown};

const MAX_PREVIEW_FETCH_BYTES: usize = 256 * 1024;
const MAX_IMAGE_FETCH_BYTES: usize = 2 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 4096;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_SANITIZED_DIMENSION: u32 = 1200;
const PREVIEW_FETCH_TIMEOUT: Duration = Duration::from_secs(4);
const PREVIEW_TOTAL_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REDIRECTS: usize = 3;
const MAX_METADATA_CHARS: usize = 180;
const MAX_METADATA_DESCRIPTION_CHARS: usize = 280;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum LinkPreviewImageFetchState {
    None,
    Image,
    TransientFailure,
    Rejected,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreviewMetadata {
    title: String,
    site_name: Option<String>,
    description: Option<String>,
    image_data_url: Option<String>,
    image_domain: Option<String>,
    image_fetch_state: LinkPreviewImageFetchState,
    image_retry_after_ms: Option<u64>,
    favicon_data_url: Option<String>,
}

#[tauri::command]
pub async fn fetch_link_preview_metadata(
    href: String,
) -> Result<Option<LinkPreviewMetadata>, String> {
    tokio::time::timeout(
        PREVIEW_TOTAL_TIMEOUT,
        fetch_link_preview_metadata_inner(href),
    )
    .await
    .map_err(|_| "link preview request timed out".to_string())?
}

async fn fetch_link_preview_metadata_inner(
    href: String,
) -> Result<Option<LinkPreviewMetadata>, String> {
    let mut url = Url::parse(href.trim()).map_err(|error| format!("invalid URL: {error}"))?;
    validate_public_https_url(&url).await?;

    if youtube::is_video_url(&url) {
        return youtube::fetch_oembed_metadata(&url).await;
    }

    for redirect_count in 0..=MAX_REDIRECTS {
        let response = send_pinned_request(&url, "text/html,application/xhtml+xml;q=0.9").await?;

        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Ok(None);
            }
            let Some(location) = response.headers().get(LOCATION) else {
                return Ok(None);
            };
            let location = location
                .to_str()
                .map_err(|_| "link preview redirect has an invalid location".to_string())?;
            url = url
                .join(location)
                .map_err(|error| format!("invalid link preview redirect: {error}"))?;
            validate_public_https_url(&url).await?;
            continue;
        }

        if !response.status().is_success() || !is_html_response(&response) {
            return Ok(None);
        }
        let body = read_bytes_prefix(response, MAX_PREVIEW_FETCH_BYTES).await?;
        let body = String::from_utf8_lossy(&body);
        let Some(mut metadata) = extract_link_preview_metadata(&body) else {
            return Ok(None);
        };
        let image_url = extract_image_url(&body, &url);
        let favicon_url = extract_favicon_url(&body, &url);
        let (image_result, favicon_result) = tokio::join!(
            async {
                match image_url {
                    Some(image_url) => Some(
                        tokio::time::timeout(
                            PREVIEW_FETCH_TIMEOUT,
                            fetch_sanitized_image(image_url, false),
                        )
                        .await
                        .unwrap_or(Err(ImageFetchError::Transient { retry_after: None })),
                    ),
                    None => None,
                }
            },
            async {
                match favicon_url {
                    Some(favicon_url) => tokio::time::timeout(
                        PREVIEW_FETCH_TIMEOUT,
                        fetch_sanitized_image(favicon_url, true),
                    )
                    .await
                    .ok(),
                    None => None,
                }
            }
        );

        apply_image_result(&mut metadata, image_result);
        if let Some(Ok((data_url, _))) = favicon_result {
            metadata.favicon_data_url = Some(data_url);
        }
        return Ok(Some(metadata));
    }

    Ok(None)
}

fn apply_image_result(
    metadata: &mut LinkPreviewMetadata,
    image_result: Option<Result<(String, String), ImageFetchError>>,
) {
    match image_result {
        Some(Ok((data_url, domain))) => {
            metadata.image_data_url = Some(data_url);
            metadata.image_domain = Some(domain);
            metadata.image_fetch_state = LinkPreviewImageFetchState::Image;
        }
        Some(Err(ImageFetchError::Transient { retry_after })) => {
            metadata.image_fetch_state = LinkPreviewImageFetchState::TransientFailure;
            metadata.image_retry_after_ms =
                retry_after.and_then(|duration| u64::try_from(duration.as_millis()).ok());
        }
        Some(Err(ImageFetchError::Rejected)) => {
            metadata.image_fetch_state = LinkPreviewImageFetchState::Rejected;
        }
        None => {}
    }
}

async fn validate_public_https_url(url: &Url) -> Result<(), String> {
    if url.scheme() != "https" || url.username() != "" || url.password().is_some() {
        return Err("link previews require an HTTPS URL without credentials".to_string());
    }
    if url.port().is_some_and(|port| port != 443) {
        return Err("link previews require the default HTTPS port".to_string());
    }

    let host = url
        .host_str()
        .ok_or_else(|| "link preview URL has no host".to_string())?;
    resolve_public_addresses(host).await.map(|_| ())
}

async fn resolve_public_addresses(host: &str) -> Result<Vec<IpAddr>, String> {
    let host = host.to_string();
    let addresses = tokio::net::lookup_host((host.as_str(), 443))
        .await
        .map_err(|error| format!("link preview DNS resolution failed: {error}"))?
        .map(|address| address.ip())
        .collect::<Vec<_>>();

    if addresses.is_empty() {
        return Err("link preview DNS resolution returned no addresses".to_string());
    }
    if addresses.iter().any(buzz_core_pkg::network::is_private_ip) {
        return Err("link preview host resolved to a private or reserved address".to_string());
    }

    Ok(addresses)
}

async fn send_pinned_request(url: &Url, accept: &str) -> Result<reqwest::Response, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "link preview URL has no host".to_string())?;
    let addresses = resolve_public_addresses(host).await?;
    let socket_addresses = addresses
        .into_iter()
        .map(|address| std::net::SocketAddr::new(address, 443))
        .collect::<Vec<_>>();
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .pool_max_idle_per_host(0)
        .resolve_to_addrs(host, &socket_addresses)
        .build()
        .map_err(|error| format!("link preview client failed: {error}"))?;
    let request = client
        .get(url.as_str())
        .header(ACCEPT, accept)
        .header(USER_AGENT, "Buzz Desktop link preview");

    tokio::time::timeout(PREVIEW_FETCH_TIMEOUT, request.send())
        .await
        .map_err(|_| "link preview request timed out".to_string())?
        .map_err(|error| format!("link preview request failed: {error}"))
}

fn is_html_response(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            let mime = value.split(';').next().unwrap_or_default().trim();
            mime.eq_ignore_ascii_case("text/html")
                || mime.eq_ignore_ascii_case("application/xhtml+xml")
        })
        .unwrap_or(false)
}

async fn read_bytes_prefix(response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::with_capacity(limit);

    while bytes.len() < limit {
        let Some(chunk) = stream.next().await else {
            break;
        };
        let chunk = chunk.map_err(|error| format!("reading link preview failed: {error}"))?;
        let remaining = limit - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    Ok(bytes)
}

async fn read_limited_bytes(response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("reading link preview failed: {error}"))?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err("link preview response exceeded the size limit".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn extract_favicon_url(html: &str, page_url: &Url) -> Option<Url> {
    let lower = html.to_ascii_lowercase();
    let mut search_from = 0;
    let mut fallback = None;

    while let Some(relative_start) = lower[search_from..].find("<link") {
        let start = search_from + relative_start;
        let Some(relative_end) = lower[start..].find('>') else {
            break;
        };
        let end = start + relative_end + 1;
        let tag = &html[start..end];
        let rel = attr_value(tag, "rel");
        let is_icon = rel.as_ref().is_some_and(|value| {
            value.split_ascii_whitespace().any(|token| {
                token.eq_ignore_ascii_case("icon") || token.eq_ignore_ascii_case("apple-touch-icon")
            })
        });
        if is_icon {
            if let Some(href) = attr_value(tag, "href") {
                if let Ok(url) = page_url.join(href.trim()) {
                    let declared_type = attr_value(tag, "type");
                    let is_supported_raster = declared_type.as_ref().is_some_and(|value| {
                        matches!(
                            value.to_ascii_lowercase().as_str(),
                            "image/jpeg" | "image/png" | "image/webp"
                        )
                    }) || matches!(
                        url.path()
                            .rsplit_once('.')
                            .map(|(_, extension)| extension.to_ascii_lowercase())
                            .as_deref(),
                        Some("jpg" | "jpeg" | "png" | "webp")
                    );
                    if is_supported_raster {
                        return Some(url);
                    }
                    fallback.get_or_insert(url);
                }
            }
        }
        search_from = end;
    }

    fallback
}

fn extract_image_url(html: &str, page_url: &Url) -> Option<Url> {
    let raw = extract_meta_content(html, "property", "og:image")
        .or_else(|| extract_meta_content(html, "property", "og:image:secure_url"))
        .or_else(|| extract_meta_content(html, "name", "twitter:image"))?;
    page_url.join(raw.trim()).ok()
}

#[derive(Debug, PartialEq)]
enum ImageFetchError {
    Transient { retry_after: Option<Duration> },
    Rejected,
}

async fn fetch_sanitized_image(
    mut url: Url,
    preserve_transparency: bool,
) -> Result<(String, String), ImageFetchError> {
    validate_public_https_url(&url)
        .await
        .map_err(|_| ImageFetchError::Rejected)?;
    for redirect_count in 0..=MAX_REDIRECTS {
        if let Some(retry_after) = image_host_cooldown_remaining(&url) {
            return Err(ImageFetchError::Transient {
                retry_after: Some(retry_after),
            });
        }
        let response = send_pinned_request(&url, "image/jpeg,image/png,image/webp")
            .await
            .map_err(|_| ImageFetchError::Transient { retry_after: None })?;
        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err(ImageFetchError::Rejected);
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(ImageFetchError::Rejected)?;
            url = url.join(location).map_err(|_| ImageFetchError::Rejected)?;
            validate_public_https_url(&url)
                .await
                .map_err(|_| ImageFetchError::Rejected)?;
            continue;
        }
        if !response.status().is_success() {
            let status = response.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::REQUEST_TIMEOUT
                || status == reqwest::StatusCode::TOO_EARLY
                || status.is_server_error()
            {
                let retry_after = retry_after_duration(&response);
                if let Some(retry_after) = retry_after {
                    set_image_host_cooldown(&url, retry_after);
                }
                return Err(ImageFetchError::Transient { retry_after });
            }
            return Err(ImageFetchError::Rejected);
        }
        let declared_mime = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase()
            })
            .ok_or(ImageFetchError::Rejected)?;
        if !matches!(
            declared_mime.as_str(),
            "image/jpeg" | "image/png" | "image/webp"
        ) {
            return Err(ImageFetchError::Rejected);
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_IMAGE_FETCH_BYTES as u64)
        {
            return Err(ImageFetchError::Rejected);
        }
        let bytes = read_limited_bytes(response, MAX_IMAGE_FETCH_BYTES)
            .await
            .map_err(|_| ImageFetchError::Rejected)?;
        let data_url = tokio::task::spawn_blocking(move || {
            sanitize_image(&bytes, &declared_mime, preserve_transparency)
        })
        .await
        .map_err(|_| ImageFetchError::Rejected)?
        .map_err(|_| ImageFetchError::Rejected)?;
        let domain = url.host_str().unwrap_or_default().to_string();
        return Ok((data_url, domain));
    }
    Err(ImageFetchError::Rejected)
}

fn sanitize_image(
    bytes: &[u8],
    declared_mime: &str,
    preserve_transparency: bool,
) -> Result<String, String> {
    let sniffed = infer::get(bytes)
        .map(|kind| kind.mime_type())
        .ok_or_else(|| "link preview image magic bytes are unsupported".to_string())?;
    if sniffed != declared_mime {
        return Err("link preview image content type does not match its bytes".to_string());
    }
    let format = match sniffed {
        "image/jpeg" => image::ImageFormat::Jpeg,
        "image/png" => image::ImageFormat::Png,
        "image/webp" => image::ImageFormat::WebP,
        _ => return Err("link preview image type is unsupported".to_string()),
    };
    if declares_animation(bytes, format) {
        return Err("animated link preview images are unsupported".to_string());
    }

    let reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| "link preview image is malformed".to_string())?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err("link preview image dimensions exceed safe limits".to_string());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_PIXELS * 4);
    decoder
        .set_limits(limits)
        .map_err(|_| "link preview image exceeds safe decoding limits".to_string())?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = image::DynamicImage::from_decoder(decoder)
        .map_err(|_| "link preview image could not be decoded".to_string())?;
    decoded.apply_orientation(orientation);
    let decoded = decoded.thumbnail(MAX_SANITIZED_DIMENSION, MAX_SANITIZED_DIMENSION);
    let mut output = Vec::new();
    if preserve_transparency && decoded.color().has_alpha() {
        decoded
            .write_to(&mut Cursor::new(&mut output), image::ImageFormat::Png)
            .map_err(|_| "link preview image could not be sanitized".to_string())?;
        return Ok(format!(
            "data:image/png;base64,{}",
            BASE64_STANDARD.encode(output)
        ));
    }
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 82)
        .encode_image(&decoded)
        .map_err(|_| "link preview image could not be sanitized".to_string())?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        BASE64_STANDARD.encode(output)
    ))
}

fn declares_animation(bytes: &[u8], format: image::ImageFormat) -> bool {
    match format {
        image::ImageFormat::Png => bytes.windows(4).any(|chunk| chunk == b"acTL"),
        image::ImageFormat::WebP => {
            bytes.len() >= 21
                && bytes.starts_with(b"RIFF")
                && &bytes[8..12] == b"WEBP"
                && ((&bytes[12..16] == b"VP8X" && bytes[20] & 0x02 != 0)
                    || bytes.windows(4).any(|chunk| chunk == b"ANIM"))
        }
        _ => false,
    }
}

fn extract_link_preview_metadata(html: &str) -> Option<LinkPreviewMetadata> {
    let title = extract_meta_content(html, "property", "og:title")
        .or_else(|| extract_meta_content(html, "name", "twitter:title"))
        .or_else(|| extract_title_tag(html))
        .and_then(|value| normalize_metadata_text(&value))?;
    let site_name = extract_meta_content(html, "property", "og:site_name")
        .and_then(|value| normalize_metadata_text(&value));
    let description = extract_meta_content(html, "property", "og:description")
        .or_else(|| extract_meta_content(html, "name", "twitter:description"))
        .and_then(|value| normalize_metadata_description(&value));

    Some(LinkPreviewMetadata {
        title,
        site_name,
        description,
        image_data_url: None,
        image_domain: None,
        image_fetch_state: LinkPreviewImageFetchState::None,
        image_retry_after_ms: None,
        favicon_data_url: None,
    })
}

fn extract_meta_content(html: &str, key_attr: &str, key_value: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut search_from = 0;

    while let Some(relative_start) = lower[search_from..].find("<meta") {
        let start = search_from + relative_start;
        let Some(relative_end) = lower[start..].find('>') else {
            break;
        };
        let end = start + relative_end + 1;
        let tag = &html[start..end];
        if attr_value(tag, key_attr).is_some_and(|value| value.eq_ignore_ascii_case(key_value)) {
            if let Some(content) = attr_value(tag, "content") {
                return Some(content);
            }
        }
        search_from = end;
    }

    None
}

fn extract_title_tag(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let content_start = start + lower[start..].find('>')? + 1;
    let content_end = content_start + lower[content_start..].find("</title>")?;
    Some(decode_html_entities(&html[content_start..content_end]))
}

fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let attr = attr.to_ascii_lowercase();
    let mut search_from = 0;

    while let Some(relative_start) = lower[search_from..].find(&attr) {
        let name_start = search_from + relative_start;
        let name_end = name_start + attr.len();
        let before = lower[..name_start].chars().last();
        let after = lower[name_end..].chars().next();
        let has_name_boundary = !matches!(before, Some(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_')
            && !matches!(after, Some(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_');

        if has_name_boundary {
            let rest = &tag[name_end..];
            let equals_offset = rest.find('=')?;
            let value = rest[equals_offset + 1..].trim_start();
            let quote = value.chars().next()?;
            if quote == '"' || quote == '\'' {
                let value_body = &value[quote.len_utf8()..];
                let value_end = value_body.find(quote)?;
                return Some(decode_html_entities(&value_body[..value_end]));
            }
            let value_end = value
                .find(|c: char| c.is_ascii_whitespace() || c == '>')
                .unwrap_or(value.len());
            return Some(decode_html_entities(&value[..value_end]));
        }
        search_from = name_end;
    }

    None
}

fn normalize_metadata_text(raw: &str) -> Option<String> {
    let mut normalized = decode_html_entities(raw)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for suffix in [
        " - Google Docs",
        " - Google Sheets",
        " - Google Slides",
        " - Google Drive",
    ] {
        if let Some(stripped) = normalized.strip_suffix(suffix) {
            normalized = stripped.trim().to_string();
            break;
        }
    }
    if matches!(
        normalized.as_str(),
        "" | "Sign in - Google Accounts" | "Google Docs" | "Google Sheets" | "Google Slides"
    ) {
        return None;
    }
    Some(normalized.chars().take(MAX_METADATA_CHARS).collect())
}

fn normalize_metadata_description(raw: &str) -> Option<String> {
    let decoded = decode_html_entities(raw)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let normalized = decoded
        .split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = normalized.trim();
    if normalized.is_empty() {
        return None;
    }
    Some(
        normalized
            .chars()
            .take(MAX_METADATA_DESCRIPTION_CHARS)
            .collect(),
    )
}

fn decode_html_entities(value: &str) -> String {
    let mut decoded = value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">");

    while let Some(start) = decoded.find("&#") {
        let Some(relative_end) = decoded[start..].find(';') else {
            break;
        };
        let end = start + relative_end + 1;
        let entity = &decoded[start + 2..end - 1];
        let parsed = entity
            .strip_prefix('x')
            .or_else(|| entity.strip_prefix('X'))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .or_else(|| entity.parse::<u32>().ok());
        let Some(ch) = parsed.and_then(char::from_u32) else {
            break;
        };
        decoded.replace_range(start..end, &ch.to_string());
    }
    decoded
}

#[cfg(test)]
mod tests {
    use super::rate_limit::MAX_IMAGE_RETRY_AFTER;
    use super::{
        apply_image_result, declares_animation, extract_favicon_url, extract_image_url,
        extract_link_preview_metadata, is_html_response, read_bytes_prefix, retry_after_duration,
        sanitize_image, ImageFetchError, LinkPreviewImageFetchState, LinkPreviewMetadata,
        MAX_METADATA_DESCRIPTION_CHARS,
    };
    use axum::{body::Body, http::Response, routing::get, Router};
    use base64::Engine as _;
    use bytes::Bytes;
    use futures_util::stream;
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
    use std::{convert::Infallible, io::Cursor};
    use url::Url;

    async fn test_response(router: Router, path: &str) -> reqwest::Response {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        reqwest::get(format!("http://{address}{path}"))
            .await
            .unwrap()
    }

    #[test]
    fn metadata_prefers_open_graph_and_reads_site_name() {
        let html = r#"<meta content="Buzz" property="og:site_name">
          <meta content="Rich previews &amp; cards" property="og:title">
          <meta content="Safe &amp; useful previews" property="og:description">
          <meta name="twitter:title" content="Twitter fallback"><title>Fallback</title>"#;
        assert_eq!(
            extract_link_preview_metadata(html),
            Some(LinkPreviewMetadata {
                title: "Rich previews & cards".to_string(),
                site_name: Some("Buzz".to_string()),
                description: Some("Safe & useful previews".to_string()),
                image_data_url: None,
                image_domain: None,
                image_fetch_state: LinkPreviewImageFetchState::None,
                image_retry_after_ms: None,
                favicon_data_url: None,
            })
        );
    }

    #[test]
    fn image_results_preserve_absence_and_classify_recovery() {
        let mut metadata = extract_link_preview_metadata("<title>Preview result</title>").unwrap();
        apply_image_result(&mut metadata, None);
        assert_eq!(metadata.image_fetch_state, LinkPreviewImageFetchState::None);

        apply_image_result(
            &mut metadata,
            Some(Err(ImageFetchError::Transient {
                retry_after: Some(std::time::Duration::from_secs(15)),
            })),
        );
        assert_eq!(
            metadata.image_fetch_state,
            LinkPreviewImageFetchState::TransientFailure
        );
        assert_eq!(metadata.image_retry_after_ms, Some(15_000));

        apply_image_result(
            &mut metadata,
            Some(Ok((
                "data:image/jpeg;base64,abc".to_string(),
                "images.example.com".to_string(),
            ))),
        );
        assert_eq!(
            metadata.image_fetch_state,
            LinkPreviewImageFetchState::Image
        );
        assert_eq!(metadata.image_domain.as_deref(), Some("images.example.com"));
    }

    #[test]
    fn metadata_falls_back_to_twitter_then_title() {
        assert_eq!(
            extract_link_preview_metadata("<meta content='Tweet title' name='twitter:title'>")
                .map(|metadata| metadata.title),
            Some("Tweet title".to_string())
        );
        assert_eq!(
            extract_link_preview_metadata("<title> Plain   title </title>")
                .map(|metadata| metadata.title),
            Some("Plain title".to_string())
        );
    }

    #[test]
    fn metadata_preserves_description_line_breaks() {
        let html = r#"<meta property="og:title" content="Tweet title">
          <meta property="og:description" content="First paragraph.&#10;&#10;Agents:&#10;- One&#10;- Two">"#;
        assert_eq!(
            extract_link_preview_metadata(html).and_then(|metadata| metadata.description),
            Some("First paragraph.\n\nAgents:\n- One\n- Two".to_string())
        );
    }

    #[test]
    fn metadata_description_supports_standard_x_posts() {
        let description = "x".repeat(MAX_METADATA_DESCRIPTION_CHARS + 1);
        let html = format!(
            r#"<meta property="og:title" content="Long post"><meta property="og:description" content="{description}">"#
        );
        let extracted = extract_link_preview_metadata(&html)
            .and_then(|metadata| metadata.description)
            .unwrap();
        assert_eq!(extracted.chars().count(), MAX_METADATA_DESCRIPTION_CHARS);
    }

    #[test]
    fn favicon_metadata_resolves_relative_icon_links() {
        let page = Url::parse("https://example.com/articles/one").unwrap();
        let html = r#"<link rel="stylesheet" href="styles.css">
          <link href="../favicon.png" rel="shortcut icon">"#;
        assert_eq!(
            extract_favicon_url(html, &page).unwrap().as_str(),
            "https://example.com/favicon.png"
        );
    }

    #[test]
    fn favicon_metadata_prefers_a_supported_raster_candidate() {
        let page = Url::parse("https://github.com/block/buzz").unwrap();
        let html = r#"<link rel="mask-icon" href="https://assets.example/favicon.svg">
          <link rel="alternate icon" type="image/png" href="https://assets.example/favicon.png">
          <link rel="icon" type="image/svg+xml" href="https://assets.example/favicon.svg">"#;
        assert_eq!(
            extract_favicon_url(html, &page).unwrap().as_str(),
            "https://assets.example/favicon.png"
        );
    }

    #[test]
    fn favicon_metadata_uses_touch_icon_before_unsupported_ico() {
        let page = Url::parse("https://twitter.com/tellaho").unwrap();
        let html = r#"<link rel="icon" href="/favicon.ico">
          <link rel="apple-touch-icon" sizes="192x192" href="/apple-touch-icon.png">"#;
        assert_eq!(
            extract_favicon_url(html, &page).unwrap().as_str(),
            "https://twitter.com/apple-touch-icon.png"
        );
    }

    #[test]
    fn image_metadata_resolves_relative_urls_and_prefers_open_graph() {
        let page = Url::parse("https://example.com/articles/one").unwrap();
        let html = r#"<meta name="twitter:image" content="https://cdn.example/twitter.jpg">
          <meta property="og:image" content="../preview.png">"#;
        assert_eq!(
            extract_image_url(html, &page).unwrap().as_str(),
            "https://example.com/preview.png"
        );
    }

    #[tokio::test]
    async fn oversized_html_uses_metadata_within_the_bounded_prefix() {
        const LIMIT: usize = 256;
        let metadata = r#"<meta property="og:title" content="Prefix title"><meta property="og:image" content="https://example.com/preview.png">"#;
        let body = format!("{metadata}{}", "x".repeat(LIMIT));
        let response = test_response(
            Router::new().route(
                "/declared",
                get(move || {
                    let body = body.clone();
                    async move {
                        Response::builder()
                            .header("content-type", "text/html")
                            .body(Body::from(body))
                            .unwrap()
                    }
                }),
            ),
            "/declared",
        )
        .await;
        assert!(response
            .content_length()
            .is_some_and(|size| size > LIMIT as u64));
        assert!(is_html_response(&response));

        let prefix = read_bytes_prefix(response, LIMIT).await.unwrap();
        assert_eq!(prefix.len(), LIMIT);
        let html = String::from_utf8_lossy(&prefix);
        assert_eq!(
            extract_link_preview_metadata(&html).map(|metadata| metadata.title),
            Some("Prefix title".to_string())
        );
        assert!(extract_image_url(&html, &Url::parse("https://example.com").unwrap()).is_some());
    }

    #[tokio::test]
    async fn image_retry_after_uses_bounded_delta_seconds() {
        let response = test_response(
            Router::new().route(
                "/rate-limited",
                get(|| async {
                    Response::builder()
                        .status(429)
                        .header("retry-after", "900")
                        .body(Body::empty())
                        .unwrap()
                }),
            ),
            "/rate-limited",
        )
        .await;
        assert_eq!(
            retry_after_duration(&response),
            Some(std::time::Duration::from_secs(900))
        );

        let response = test_response(
            Router::new().route(
                "/excessive",
                get(|| async {
                    Response::builder()
                        .status(429)
                        .header("retry-after", "7200")
                        .body(Body::empty())
                        .unwrap()
                }),
            ),
            "/excessive",
        )
        .await;
        assert_eq!(retry_after_duration(&response), Some(MAX_IMAGE_RETRY_AFTER));
    }

    #[tokio::test]
    async fn oversized_chunked_html_ignores_metadata_beyond_the_bounded_prefix() {
        const LIMIT: usize = 256;
        let response = test_response(
            Router::new().route(
                "/chunked",
                get(|| async {
                    let chunks = stream::iter([
                        Ok::<_, Infallible>(Bytes::from(vec![b'x'; LIMIT])),
                        Ok(Bytes::from_static(
                            br#"<meta property="og:title" content="Too late"><meta property="og:image" content="https://example.com/late.png">"#,
                        )),
                    ]);
                    Response::builder()
                        .header("content-type", "text/html")
                        .body(Body::from_stream(chunks))
                        .unwrap()
                }),
            ),
            "/chunked",
        )
        .await;
        assert_eq!(response.content_length(), None);

        let prefix = read_bytes_prefix(response, LIMIT).await.unwrap();
        assert_eq!(prefix.len(), LIMIT);
        let html = String::from_utf8_lossy(&prefix);
        assert_eq!(extract_link_preview_metadata(&html), None);
        assert_eq!(
            extract_image_url(&html, &Url::parse("https://example.com").unwrap()),
            None
        );
    }

    #[test]
    fn sanitizer_rejects_mime_mismatch_and_outputs_static_jpeg() {
        let source = DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, Rgb([10, 20, 30])));
        let mut png = Cursor::new(Vec::new());
        source.write_to(&mut png, ImageFormat::Png).unwrap();
        assert!(sanitize_image(png.get_ref(), "image/jpeg", false).is_err());
        let sanitized = sanitize_image(png.get_ref(), "image/png", false).unwrap();
        assert!(sanitized.starts_with("data:image/jpeg;base64,"));
    }

    #[test]
    fn favicon_sanitizer_preserves_png_transparency() {
        let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([36, 41, 47, 0])));
        let mut png = Cursor::new(Vec::new());
        source.write_to(&mut png, ImageFormat::Png).unwrap();

        let sanitized = sanitize_image(png.get_ref(), "image/png", true).unwrap();
        assert!(sanitized.starts_with("data:image/png;base64,"));
        let encoded = sanitized.split_once(',').unwrap().1;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert!(image::load_from_memory(&bytes).unwrap().color().has_alpha());
    }

    #[test]
    fn animation_markers_are_rejected_before_decode() {
        let mut apng = b"\x89PNG\r\n\x1a\n".to_vec();
        apng.extend_from_slice(b"junkacTLjunk");
        assert!(declares_animation(&apng, ImageFormat::Png));

        let mut webp = b"RIFF\x00\x00\x00\x00WEBPVP8X\x0a\x00\x00\x00".to_vec();
        webp.push(0x02);
        assert!(declares_animation(&webp, ImageFormat::WebP));
    }

    #[test]
    fn metadata_requires_a_non_empty_title() {
        assert_eq!(extract_link_preview_metadata("<title>   </title>"), None);
        assert_eq!(extract_link_preview_metadata("<html></html>"), None);
    }
}
