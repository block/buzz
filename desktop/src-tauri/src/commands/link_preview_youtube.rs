use percent_encoding::percent_decode_str;
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;
use url::Url;

use super::{
    apply_image_result, fetch_sanitized_image, normalize_metadata_description,
    normalize_metadata_text, read_limited_bytes, send_pinned_request, ImageFetchError,
    LinkPreviewImageFetchState, LinkPreviewMetadata, PREVIEW_FETCH_TIMEOUT,
};

const MAX_OEMBED_FETCH_BYTES: usize = 64 * 1024;
const OEMBED_ENDPOINT: &str = "https://www.youtube.com/oembed";

#[derive(Deserialize)]
struct OEmbedResponse {
    title: String,
    author_name: Option<String>,
    provider_name: Option<String>,
    thumbnail_url: Option<String>,
}

pub(super) fn is_video_url(url: &Url) -> bool {
    let Some(host) = url.host_str().map(|host| host.to_ascii_lowercase()) else {
        return false;
    };
    match host.as_str() {
        "youtu.be" | "www.youtu.be" => url
            .path_segments()
            .and_then(|mut segments| segments.next())
            .is_some_and(|segment| !segment.is_empty()),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com" => {
            (url.path() == "/watch"
                && url
                    .query_pairs()
                    .any(|(key, value)| key == "v" && !value.is_empty()))
                || ["shorts", "live", "embed"].iter().any(|prefix| {
                    url.path_segments().is_some_and(|mut segments| {
                        segments.next() == Some(prefix)
                            && segments.next().is_some_and(|segment| !segment.is_empty())
                    })
                })
        }
        _ => false,
    }
}

pub(super) async fn fetch_oembed_metadata(
    video_url: &Url,
) -> Result<Option<LinkPreviewMetadata>, String> {
    let oembed_url = oembed_url(video_url)?;
    let response = send_pinned_request(&oembed_url, "application/json").await?;
    let Some((mut metadata, thumbnail_url)) = parse_oembed_response(response).await? else {
        return Ok(None);
    };
    let image_result = match thumbnail_url {
        Some(thumbnail_url) => Some(
            tokio::time::timeout(
                PREVIEW_FETCH_TIMEOUT,
                fetch_sanitized_image(thumbnail_url, false),
            )
            .await
            .unwrap_or(Err(ImageFetchError::Transient { retry_after: None })),
        ),
        None => None,
    };
    apply_image_result(&mut metadata, image_result);
    Ok(Some(metadata))
}

fn oembed_url(video_url: &Url) -> Result<Url, String> {
    let mut canonical_video_url = video_url.clone();
    if matches!(
        video_url.host_str(),
        Some("youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com")
    ) {
        let mut segments = video_url.path_segments();
        if segments.as_mut().and_then(|segments| segments.next()) == Some("embed") {
            let encoded_video_id = segments
                .and_then(|mut segments| segments.next())
                .ok_or_else(|| "YouTube embed URL has no video ID".to_string())?;
            let video_id = percent_decode_str(encoded_video_id)
                .decode_utf8()
                .map_err(|_| "YouTube embed URL has an invalid video ID".to_string())?;
            if !video_id.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            }) {
                return Err("YouTube embed URL has an invalid video ID".to_string());
            }
            canonical_video_url.set_path("/watch");
            canonical_video_url.set_query(None);
            canonical_video_url
                .query_pairs_mut()
                .append_pair("v", &video_id);
            canonical_video_url.set_fragment(None);
        }
    }

    let mut oembed_url = Url::parse(OEMBED_ENDPOINT)
        .map_err(|error| format!("invalid YouTube oEmbed endpoint: {error}"))?;
    oembed_url
        .query_pairs_mut()
        .append_pair("format", "json")
        .append_pair("url", canonical_video_url.as_str());
    Ok(oembed_url)
}

async fn parse_oembed_response(
    response: reqwest::Response,
) -> Result<Option<(LinkPreviewMetadata, Option<Url>)>, String> {
    if !response.status().is_success() || !is_json_response(&response) {
        return Ok(None);
    }
    let body = read_limited_bytes(response, MAX_OEMBED_FETCH_BYTES).await?;
    let response: OEmbedResponse = match serde_json::from_slice(&body) {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };
    Ok(oembed_metadata(response))
}

fn oembed_metadata(response: OEmbedResponse) -> Option<(LinkPreviewMetadata, Option<Url>)> {
    let title = normalize_metadata_text(&response.title)?;
    let thumbnail_url = response
        .thumbnail_url
        .as_deref()
        .and_then(|thumbnail| Url::parse(thumbnail).ok());
    let metadata = LinkPreviewMetadata {
        title,
        site_name: response
            .provider_name
            .as_deref()
            .and_then(normalize_metadata_text)
            .or_else(|| Some("YouTube".to_string())),
        description: response
            .author_name
            .as_deref()
            .and_then(normalize_metadata_description),
        image_data_url: None,
        image_domain: None,
        image_fetch_state: LinkPreviewImageFetchState::None,
        image_retry_after_ms: None,
        favicon_data_url: None,
    };
    Some((metadata, thumbnail_url))
}

fn is_json_response(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Response, routing::get, Router};

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
    fn recognizes_supported_video_urls_only() {
        for href in [
            "https://www.youtube.com/watch?v=hLFs9JtMaRg",
            "https://m.youtube.com/watch?v=hLFs9JtMaRg&feature=share",
            "https://music.youtube.com/watch?v=hLFs9JtMaRg",
            "https://youtu.be/hLFs9JtMaRg?t=10",
            "https://www.youtube.com/shorts/hLFs9JtMaRg",
            "https://www.youtube.com/live/hLFs9JtMaRg",
            "https://www.youtube.com/embed/hLFs9JtMaRg",
        ] {
            assert!(is_video_url(&Url::parse(href).unwrap()), "{href}");
        }
        for href in [
            "https://www.youtube.com/",
            "https://www.youtube.com/watch",
            "https://www.youtube.com/watch?v=",
            "https://www.youtube.com/@buzz",
            "https://youtube.com.evil.example/watch?v=hLFs9JtMaRg",
            "https://notyoutube.com/watch?v=hLFs9JtMaRg",
        ] {
            assert!(!is_video_url(&Url::parse(href).unwrap()), "{href}");
        }
    }

    #[test]
    fn canonicalizes_embed_url_for_oembed() {
        for (video_url, expected_video_id) in [
            (
                "https://www.youtube.com/embed/dQw4w9WgXcQ?start=10#player",
                "dQw4w9WgXcQ",
            ),
            ("https://www.youtube.com/embed/%64Qw4w9WgXcQ", "dQw4w9WgXcQ"),
            ("https://www.youtube.com/embed/dQw4w9WgX%63Q", "dQw4w9WgXcQ"),
        ] {
            let oembed_url = oembed_url(&Url::parse(video_url).unwrap()).unwrap();
            let params = oembed_url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            assert_eq!(
                params.get("format").map(|value| value.as_ref()),
                Some("json")
            );
            let provider_video_url =
                Url::parse(params.get("url").expect("oEmbed URL parameter")).unwrap();
            assert_eq!(provider_video_url.path(), "/watch");
            assert_eq!(
                provider_video_url
                    .query_pairs()
                    .find(|(key, _)| key == "v")
                    .map(|(_, value)| value.into_owned()),
                Some(expected_video_id.to_string())
            );
        }
    }

    #[test]
    fn rejects_invalid_encoded_embed_video_ids() {
        for href in [
            "https://www.youtube.com/embed/video%2Fid",
            "https://www.youtube.com/embed/video%5Cid",
            "https://www.youtube.com/embed/video%00id",
            "https://www.youtube.com/embed/video%25id",
            "https://www.youtube.com/embed/video%252Fid",
            "https://www.youtube.com/embed/video%FFid",
        ] {
            assert!(oembed_url(&Url::parse(href).unwrap()).is_err(), "{href}");
        }
    }

    #[tokio::test]
    async fn response_requires_successful_bounded_json() {
        let valid_json =
            r#"{"title":"Video title","author_name":"Creator","provider_name":"YouTube"}"#;
        let response = test_response(
            Router::new().route(
                "/valid",
                get(move || async move {
                    Response::builder()
                        .header("content-type", "application/json; charset=UTF-8")
                        .body(Body::from(valid_json))
                        .unwrap()
                }),
            ),
            "/valid",
        )
        .await;
        let (metadata, _) = parse_oembed_response(response).await.unwrap().unwrap();
        assert_eq!(metadata.title, "Video title");

        for response in [
            test_response(
                Router::new().route(
                    "/not-found",
                    get(|| async {
                        Response::builder()
                            .status(404)
                            .header("content-type", "application/json")
                            .body(Body::from("{}"))
                            .unwrap()
                    }),
                ),
                "/not-found",
            )
            .await,
            test_response(
                Router::new().route(
                    "/html",
                    get(|| async {
                        Response::builder()
                            .header("content-type", "text/html")
                            .body(Body::from("<title>Not JSON</title>"))
                            .unwrap()
                    }),
                ),
                "/html",
            )
            .await,
        ] {
            assert_eq!(parse_oembed_response(response).await.unwrap(), None);
        }

        let oversized = vec![b' '; MAX_OEMBED_FETCH_BYTES + 1];
        let response = test_response(
            Router::new().route(
                "/oversized",
                get(move || {
                    let oversized = oversized.clone();
                    async move {
                        Response::builder()
                            .header("content-type", "application/json")
                            .body(Body::from(oversized))
                            .unwrap()
                    }
                }),
            ),
            "/oversized",
        )
        .await;
        assert!(parse_oembed_response(response).await.is_err());
    }

    #[test]
    fn converts_response_to_bounded_preview_metadata() {
        let (metadata, thumbnail_url) = oembed_metadata(OEmbedResponse {
            title: "  Video   title  ".to_string(),
            author_name: Some("Buzz Creator".to_string()),
            provider_name: Some("YouTube".to_string()),
            thumbnail_url: Some("https://i.ytimg.com/vi/example/hqdefault.jpg".to_string()),
        })
        .unwrap();
        assert_eq!(metadata.title, "Video title");
        assert_eq!(metadata.site_name.as_deref(), Some("YouTube"));
        assert_eq!(metadata.description.as_deref(), Some("Buzz Creator"));
        assert_eq!(metadata.image_fetch_state, LinkPreviewImageFetchState::None);
        assert_eq!(
            thumbnail_url.unwrap().as_str(),
            "https://i.ytimg.com/vi/example/hqdefault.jpg"
        );
    }

    #[test]
    fn rejects_titleless_response_and_ignores_invalid_thumbnail() {
        assert!(oembed_metadata(OEmbedResponse {
            title: "   ".to_string(),
            author_name: None,
            provider_name: None,
            thumbnail_url: None,
        })
        .is_none());

        let (metadata, thumbnail_url) = oembed_metadata(OEmbedResponse {
            title: "Video title".to_string(),
            author_name: None,
            provider_name: None,
            thumbnail_url: Some("not a URL".to_string()),
        })
        .unwrap();
        assert_eq!(metadata.site_name.as_deref(), Some("YouTube"));
        assert_eq!(thumbnail_url, None);
    }
}
