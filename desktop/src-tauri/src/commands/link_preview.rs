use std::{future::Future, io::Cursor, net::IpAddr, time::Duration};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use futures_util::StreamExt;
use image::ImageDecoder;
use reqwest::{
    header::{ACCEPT, CONTENT_TYPE, LOCATION, USER_AGENT},
    redirect::Policy,
};
use serde::Serialize;
use url::Url;

#[path = "link_preview_image_retry.rs"]
mod image_retry;
#[path = "link_preview_rate_limit.rs"]
mod rate_limit;
#[path = "link_preview_youtube.rs"]
mod youtube;

use rate_limit::{
    image_host_cooldown_remaining, image_host_gate, retry_after_duration, set_image_host_cooldown,
};

const MAX_PREVIEW_FETCH_BYTES: usize = 256 * 1024;
const MAX_IMAGE_FETCH_BYTES: usize = 2 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 4096;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_SANITIZED_DIMENSION: u32 = 1200;
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
    fetch_link_preview_metadata_with(href, fetch_link_preview_metadata_for_url).await
}

/// Run metadata fetching without a native deadline. The renderer owns the
/// finite budget after a composer generation is promoted into a send.
async fn fetch_link_preview_metadata_with<F, Fut>(
    href: String,
    fetch: F,
) -> Result<Option<LinkPreviewMetadata>, String>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<Option<LinkPreviewMetadata>, String>>,
{
    fetch(href).await
}

async fn fetch_link_preview_metadata_for_url(
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
                    Some(image_url) => {
                        Some(fetch_sanitized_image_with_retry(image_url, false).await)
                    }
                    None => None,
                }
            },
            async {
                match favicon_url {
                    Some(favicon_url) => Some(fetch_sanitized_image(favicon_url, true).await),
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
        Some(Err(ImageFetchError::Transient { retry_after, .. })) => {
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

    request
        .send()
        .await
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
    Transient {
        retry_after: Option<Duration>,
        retry_inline: bool,
    },
    Rejected,
}

async fn fetch_sanitized_image_with_retry(
    url: Url,
    preserve_transparency: bool,
) -> Result<(String, String), ImageFetchError> {
    image_retry::retry_transient_image_fetch(|| {
        fetch_sanitized_image(url.clone(), preserve_transparency)
    })
    .await
}

async fn fetch_sanitized_image(
    mut url: Url,
    preserve_transparency: bool,
) -> Result<(String, String), ImageFetchError> {
    validate_public_https_url(&url)
        .await
        .map_err(|_| ImageFetchError::Rejected)?;
    let mut redirect_count = 0;
    let mut waited_for_cooldown = false;
    while redirect_count <= MAX_REDIRECTS {
        if let Some(retry_after) = image_host_cooldown_remaining(&url) {
            if waited_for_cooldown {
                return Err(ImageFetchError::Transient {
                    retry_after: Some(retry_after),
                    retry_inline: false,
                });
            }
            waited_for_cooldown = true;
            tokio::time::sleep(retry_after).await;
            continue;
        }

        let host_gate = image_host_gate(&url);
        let _host_guard = host_gate.lock().await;
        if image_host_cooldown_remaining(&url).is_some() {
            continue;
        }
        let response = send_pinned_request(&url, "image/jpeg,image/png,image/webp")
            .await
            .map_err(|_| ImageFetchError::Transient {
                retry_after: None,
                retry_inline: true,
            })?;
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
            redirect_count += 1;
            waited_for_cooldown = false;
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
                return Err(ImageFetchError::Transient {
                    retry_after,
                    retry_inline: status != reqwest::StatusCode::TOO_MANY_REQUESTS,
                });
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
#[path = "link_preview_tests.rs"]
mod tests;
