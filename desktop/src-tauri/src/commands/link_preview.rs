use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{
    header::{ACCEPT, CONTENT_TYPE, USER_AGENT},
    redirect::Policy,
};
use url::Url;

const MAX_TITLE_FETCH_BYTES: usize = 256 * 1024;
const TITLE_FETCH_TIMEOUT: Duration = Duration::from_secs(4);

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
    use super::{extract_google_title, is_supported_google_link};
    use url::Url;

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
