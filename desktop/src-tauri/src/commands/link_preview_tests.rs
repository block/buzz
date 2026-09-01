use super::rate_limit::MAX_IMAGE_RETRY_AFTER;
use super::retryable_image_cooldown;
use super::{
    apply_image_result, declares_animation, extract_favicon_url, extract_image_url,
    extract_link_preview_metadata, is_html_response, read_bytes_prefix, retry_after_duration,
    sanitize_image, wait_for_image_host_cooldown, ImageFetchError, LinkPreviewImageFetchState,
    LinkPreviewMetadata, MAX_METADATA_DESCRIPTION_CHARS,
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

#[tokio::test(start_paused = true)]
async fn first_rate_limit_and_queued_host_request_share_one_cooldown_boundary() {
    let cooldown = std::time::Duration::from_secs(60);
    let url = Url::parse("https://rate-limit-regression.example/image.png").unwrap();
    let first = async {
        let mut waited = false;
        let retry_after = retryable_image_cooldown(&url, Some(cooldown), &mut waited).unwrap();
        tokio::time::sleep(retry_after).await;
        assert_eq!(
            retryable_image_cooldown(&url, Some(cooldown), &mut waited),
            None
        );
    };
    let queued = async {
        let mut waited = false;
        assert!(wait_for_image_host_cooldown(&mut waited, cooldown).await);
    };
    tokio::pin!(first);

    assert!(futures_util::poll!(&mut first).is_pending());
    assert_eq!(super::image_host_cooldown_remaining(&url), Some(cooldown));
    tokio::pin!(queued);
    assert!(futures_util::poll!(&mut queued).is_pending());
    tokio::time::advance(cooldown - std::time::Duration::from_millis(1)).await;
    assert!(futures_util::poll!(&mut first).is_pending());
    assert!(futures_util::poll!(&mut queued).is_pending());
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    first.await;
    queued.await;
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
            retry_inline: false,
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
