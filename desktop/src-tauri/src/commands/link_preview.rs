use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{
    header::{ACCEPT, CONTENT_TYPE, USER_AGENT},
    redirect::Policy,
};
use url::Url;

const MAX_TITLE_FETCH_BYTES: usize = 256 * 1024;
const TITLE_FETCH_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_METADATA_REDIRECTS: usize = 3;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreviewMetadata {
    pub href: String,
    pub site_name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub favicon_url: Option<String>,
}

/// Fetch generic OpenGraph metadata (title/description/image/site name) for an
/// arbitrary http(s) URL. Returns `None` when the page yields no usable title.
#[tauri::command]
pub async fn fetch_link_preview_metadata(
    href: String,
) -> Result<Option<LinkPreviewMetadata>, String> {
    let url = Url::parse(href.trim()).map_err(|error| format!("invalid URL: {error}"))?;
    if url.scheme() != "https" && url.scheme() != "http" {
        return Ok(None);
    }
    if url.host_str().is_none() {
        return Ok(None);
    }

    let client = reqwest::Client::builder()
        .redirect(Policy::limited(MAX_METADATA_REDIRECTS))
        .pool_idle_timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(1)
        .build()
        .map_err(|error| format!("link preview metadata client failed: {error}"))?;

    let request = client
        .get(url.as_str())
        .header(
            ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(USER_AGENT, "Buzz Desktop link preview");

    let response = tokio::time::timeout(TITLE_FETCH_TIMEOUT, request.send())
        .await
        .map_err(|_| "link preview metadata request timed out".to_string())?
        .map_err(|error| format!("link preview metadata request failed: {error}"))?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let is_html = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("text/html"))
        .unwrap_or(true);
    if !is_html {
        return Ok(None);
    }

    let final_url = response.url().clone();
    let body = read_limited_text(response).await?;
    Ok(extract_link_preview_metadata(&final_url, &body))
}

fn extract_link_preview_metadata(url: &Url, html: &str) -> Option<LinkPreviewMetadata> {
    let title = meta_content(html, &["og:title", "twitter:title"])
        .or_else(|| extract_title_tag(html))
        .map(|title| normalize_meta_text(&title, 180))
        .filter(|title| !title.is_empty())?;

    let description = meta_content(
        html,
        &["og:description", "twitter:description", "description"],
    )
    .map(|value| normalize_meta_text(&value, 300))
    .filter(|value| !value.is_empty());

    let site_name = meta_content(html, &["og:site_name"])
        .map(|value| normalize_meta_text(&value, 80))
        .filter(|value| !value.is_empty());

    let image_url = meta_content(html, &["og:image", "twitter:image"])
        .and_then(|value| resolve_href(url, value.trim()));

    let favicon_url = extract_favicon_href(html)
        .and_then(|value| resolve_href(url, value.trim()))
        .or_else(|| resolve_href(url, "/favicon.ico"));

    Some(LinkPreviewMetadata {
        href: url.to_string(),
        site_name,
        title: Some(title),
        description,
        image_url,
        favicon_url,
    })
}

/// Return the `content` of the first `<meta>` tag matching any of `names`
/// (checked against both `property` and `name` attributes), in `names` order.
fn meta_content(html: &str, names: &[&str]) -> Option<String> {
    for name in names {
        let lower = html.to_ascii_lowercase();
        let mut search_from = 0;

        while let Some(relative_start) = lower[search_from..].find("<meta") {
            let start = search_from + relative_start;
            let Some(relative_end) = lower[start..].find('>') else {
                break;
            };
            let end = start + relative_end + 1;
            let tag = &html[start..end];

            let matches_name = attr_value(tag, "property")
                .or_else(|| attr_value(tag, "name"))
                .map(|value| value.eq_ignore_ascii_case(name))
                .unwrap_or(false);

            if matches_name {
                if let Some(content) = attr_value(tag, "content") {
                    if !content.trim().is_empty() {
                        return Some(content);
                    }
                }
            }

            search_from = end;
        }
    }

    None
}

fn extract_favicon_href(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut search_from = 0;

    while let Some(relative_start) = lower[search_from..].find("<link") {
        let start = search_from + relative_start;
        let Some(relative_end) = lower[start..].find('>') else {
            break;
        };
        let end = start + relative_end + 1;
        let tag = &html[start..end];

        let is_icon = attr_value(tag, "rel")
            .map(|rel| {
                rel.to_ascii_lowercase().split_whitespace().any(|token| {
                    token == "icon" || token == "shortcut" || token == "apple-touch-icon"
                })
            })
            .unwrap_or(false);

        if is_icon {
            if let Some(href) = attr_value(tag, "href") {
                if !href.trim().is_empty() {
                    return Some(href);
                }
            }
        }

        search_from = end;
    }

    None
}

fn resolve_href(base: &Url, href: &str) -> Option<String> {
    let resolved = base.join(href).ok()?;
    if resolved.scheme() != "https" && resolved.scheme() != "http" {
        return None;
    }
    Some(resolved.to_string())
}

fn normalize_meta_text(raw: &str, max_chars: usize) -> String {
    decode_html_entities(raw)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

#[tauri::command]
pub async fn fetch_link_preview_title(href: String) -> Result<Option<String>, String> {
    let url = Url::parse(href.trim()).map_err(|error| format!("invalid URL: {error}"))?;
    if !is_supported_google_link(&url) {
        return Ok(None);
    }

    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .pool_idle_timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(1)
        .build()
        .map_err(|error| format!("link preview title client failed: {error}"))?;

    let request = client
        .get(url.as_str())
        .header(
            ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(USER_AGENT, "Buzz Desktop link preview");

    let response = tokio::time::timeout(TITLE_FETCH_TIMEOUT, request.send())
        .await
        .map_err(|_| "link preview title request timed out".to_string())?
        .map_err(|error| format!("link preview title request failed: {error}"))?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let is_html = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("text/html"))
        .unwrap_or(true);
    if !is_html {
        return Ok(None);
    }

    let body = read_limited_text(response).await?;
    Ok(extract_google_title(&body))
}

fn is_supported_google_link(url: &Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }

    let Some(host) = url.host_str().map(|host| host.to_ascii_lowercase()) else {
        return false;
    };
    let segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();

    match host.trim_start_matches("www.") {
        "docs.google.com" => {
            matches!(
                segments.as_slice(),
                ["document", "d", _, ..]
                    | ["spreadsheets", "d", _, ..]
                    | ["presentation", "d", _, ..]
            )
        }
        "drive.google.com" => {
            matches!(segments.as_slice(), ["file", "d", _, ..])
                || matches!(segments.as_slice(), ["drive", "folders", _, ..])
                || (segments.first() == Some(&"open")
                    && url.query_pairs().any(|(key, _)| key == "id"))
        }
        _ => false,
    }
}

async fn read_limited_text(response: reqwest::Response) -> Result<String, String> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("reading title response failed: {error}"))?;
        if bytes.len() + chunk.len() > MAX_TITLE_FETCH_BYTES {
            let remaining = MAX_TITLE_FETCH_BYTES.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..remaining]);
            break;
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn extract_google_title(html: &str) -> Option<String> {
    extract_meta_title(html)
        .or_else(|| extract_title_tag(html))
        .and_then(|title| normalize_google_title(&title))
}

fn extract_meta_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut search_from = 0;

    while let Some(relative_start) = lower[search_from..].find("<meta") {
        let start = search_from + relative_start;
        let Some(relative_end) = lower[start..].find('>') else {
            break;
        };
        let end = start + relative_end + 1;
        let tag = &html[start..end];
        let lower_tag = &lower[start..end];

        if lower_tag.contains("og:title") || lower_tag.contains("twitter:title") {
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
    Some(html[content_start..content_end].to_string())
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
            let lower_rest = &lower[name_end..];
            let equals_offset = lower_rest.find('=')?;
            let value_start = name_end + equals_offset + 1;
            let value = tag[value_start..].trim_start();
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

fn normalize_google_title(raw_title: &str) -> Option<String> {
    let mut title = decode_html_entities(raw_title)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    for suffix in [
        " - Google Docs",
        " - Google Sheets",
        " - Google Slides",
        " - Google Drive",
    ] {
        if let Some(stripped) = title.strip_suffix(suffix) {
            title = stripped.trim().to_string();
            break;
        }
    }

    match title.as_str() {
        ""
        | "Document"
        | "Spreadsheet"
        | "Presentation"
        | "Drive file"
        | "Drive folder"
        | "Google Docs"
        | "Google Sheets"
        | "Google Slides"
        | "Google Drive"
        | "Sign in - Google Accounts" => None,
        _ => Some(title.chars().take(180).collect()),
    }
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
        let parsed = if let Some(hex) = entity
            .strip_prefix('x')
            .or_else(|| entity.strip_prefix('X'))
        {
            u32::from_str_radix(hex, 16).ok()
        } else {
            entity.parse::<u32>().ok()
        };

        let Some(ch) = parsed.and_then(char::from_u32) else {
            break;
        };
        decoded.replace_range(start..end, &ch.to_string());
    }

    decoded
}

#[cfg(test)]
mod tests {
    use super::{extract_google_title, extract_link_preview_metadata, is_supported_google_link};
    use url::Url;

    #[test]
    fn metadata_extracts_open_graph_fields() {
        let html = r#"
          <html>
            <head>
              <meta property="og:site_name" content="Example News">
              <meta property="og:title" content="Big &amp; important story">
              <meta property="og:description" content="  A story   about things.  ">
              <meta property="og:image" content="/images/story.png">
              <link rel="icon" href="/favicon.svg">
              <title>Fallback title</title>
            </head>
          </html>
        "#;
        let url = Url::parse("https://news.example.com/story/1").unwrap();

        let metadata = extract_link_preview_metadata(&url, html).unwrap();
        assert_eq!(metadata.site_name.as_deref(), Some("Example News"));
        assert_eq!(metadata.title.as_deref(), Some("Big & important story"));
        assert_eq!(
            metadata.description.as_deref(),
            Some("A story about things.")
        );
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://news.example.com/images/story.png")
        );
        assert_eq!(
            metadata.favicon_url.as_deref(),
            Some("https://news.example.com/favicon.svg")
        );
    }

    #[test]
    fn metadata_falls_back_to_title_tag_and_default_favicon() {
        let html = "<html><head><title>Plain page</title></head></html>";
        let url = Url::parse("https://example.com/page").unwrap();

        let metadata = extract_link_preview_metadata(&url, html).unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Plain page"));
        assert_eq!(metadata.site_name, None);
        assert_eq!(metadata.description, None);
        assert_eq!(metadata.image_url, None);
        assert_eq!(
            metadata.favicon_url.as_deref(),
            Some("https://example.com/favicon.ico")
        );
    }

    #[test]
    fn metadata_requires_a_title() {
        let url = Url::parse("https://example.com/").unwrap();
        assert_eq!(extract_link_preview_metadata(&url, "<html></html>"), None);
        assert_eq!(
            extract_link_preview_metadata(&url, "<title>   </title>"),
            None
        );
    }

    #[test]
    fn title_prefers_open_graph_title() {
        let html = r#"
          <html>
            <head>
              <meta property="og:title" content="Composer links &amp; previews - Google Docs">
              <title>Fallback - Google Docs</title>
            </head>
          </html>
        "#;

        assert_eq!(
            extract_google_title(html).as_deref(),
            Some("Composer links & previews")
        );
    }

    #[test]
    fn title_ignores_generic_google_titles() {
        assert_eq!(
            extract_google_title("<title>Sign in - Google Accounts</title>"),
            None
        );
        assert_eq!(extract_google_title("<title>Google Docs</title>"), None);
    }

    #[test]
    fn supported_urls_are_google_file_links_only() {
        assert!(is_supported_google_link(
            &Url::parse("https://docs.google.com/document/d/abc/edit").unwrap()
        ));
        assert!(is_supported_google_link(
            &Url::parse("https://docs.google.com/spreadsheets/d/abc/edit").unwrap()
        ));
        assert!(is_supported_google_link(
            &Url::parse("https://drive.google.com/file/d/abc/view").unwrap()
        ));
        assert!(!is_supported_google_link(
            &Url::parse("https://example.com/document/d/abc/edit").unwrap()
        ));
        assert!(!is_supported_google_link(
            &Url::parse("http://docs.google.com/document/d/abc/edit").unwrap()
        ));
    }
}
