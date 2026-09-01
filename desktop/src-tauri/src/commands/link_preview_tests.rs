use super::rate_limit::MAX_IMAGE_RETRY_AFTER;
use super::{
    apply_image_result, declares_animation, extract_favicon_url, extract_image_url,
    extract_link_preview_metadata, fetch_sanitized_image_using, is_html_response,
    read_bytes_prefix, retry_after_duration, retryable_image_cooldown, sanitize_image,
    ImageFetchError, LinkPreviewImageFetchState, LinkPreviewMetadata, MAX_INLINE_IMAGE_COOLDOWN,
    MAX_METADATA_DESCRIPTION_CHARS,
};
use axum::{body::Body, http::Response, routing::get, Router};
use base64::Engine as _;
use bytes::Bytes;
use futures_util::stream;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use std::{
    convert::Infallible,
    io::Cursor,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;
use url::Url;

async fn start_test_server(router: Router) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    address
}

async fn test_response(router: Router, path: &str) -> reqwest::Response {
    let address = start_test_server(router).await;
    reqwest::get(format!("http://{address}{path}"))
        .await
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn first_rate_limit_and_queued_host_request_share_one_cooldown_boundary() {
    let cooldown = std::time::Duration::from_secs(20);
    let rate_limited_path = "/rate-limited.png";
    let success_path = "/success.png";
    let url = Url::parse(&format!(
        "https://rate-limit-regression.example{rate_limited_path}"
    ))
    .unwrap();
    let attempts = Arc::new(Mutex::new(0));
    let collision_attempts = Arc::new(Mutex::new(0));
    let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, Rgb([10, 20, 30])));
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).unwrap();
    let image_bytes = png.into_inner();
    let server_attempts = Arc::clone(&attempts);
    let address = start_test_server(Router::new().route(
        "/{image}",
        get(
            move |axum::extract::Path(image): axum::extract::Path<String>| {
                let image_bytes = image_bytes.clone();
                let server_attempts = Arc::clone(&server_attempts);
                async move {
                    if image == "rate-limited.png" {
                        let attempt = {
                            let mut attempts = server_attempts.lock().unwrap();
                            *attempts += 1;
                            *attempts
                        };
                        if attempt == 1 {
                            return Response::builder()
                                .status(429)
                                .header("retry-after", cooldown.as_secs())
                                .body(Body::empty())
                                .unwrap();
                        }
                    }
                    Response::builder()
                        .header("content-type", "image/png")
                        .body(Body::from(image_bytes))
                        .unwrap()
                }
            },
        ),
    ))
    .await;
    let test_client = reqwest::Client::new();
    let request = move |url: Url, _accept: &'static str| {
        let test_client = test_client.clone();
        async move {
            test_client
                .get(format!("http://{address}{}", url.path()))
                .send()
                .await
                .map_err(|error| error.to_string())
        }
    };
    let collision_request = {
        let collision_attempts = Arc::clone(&collision_attempts);
        let test_client = reqwest::Client::new();
        move |url: Url, _accept: &'static str| {
            let collision_attempts = Arc::clone(&collision_attempts);
            let test_client = test_client.clone();
            async move {
                *collision_attempts.lock().unwrap() += 1;
                test_client
                    .get(format!("http://{address}{}", url.path()))
                    .send()
                    .await
                    .map_err(|error| error.to_string())
            }
        }
    };
    let validate = |_url: Url| async { Ok(()) };
    let first = tokio::spawn(fetch_sanitized_image_using(
        url.clone(),
        false,
        validate,
        request.clone(),
    ));
    while super::image_host_cooldown_remaining(&url).is_none() {
        tokio::task::yield_now().await;
    }
    assert!(!first.is_finished());
    assert_eq!(*attempts.lock().unwrap(), 1);
    assert_eq!(super::image_host_cooldown_remaining(&url), Some(cooldown));

    let colliding_url = (0..10_000)
        .map(|index| {
            Url::parse(&format!("https://collision-{index}.example{success_path}")).unwrap()
        })
        .find(|candidate| {
            std::ptr::eq(
                super::image_host_gate(candidate),
                super::image_host_gate(&url),
            )
        })
        .expect("a different host sharing the bounded gate stripe");

    let (collision_started_tx, collision_started_rx) = oneshot::channel();
    tokio::spawn(async move {
        let collision =
            fetch_sanitized_image_using(colliding_url, false, validate, collision_request);
        tokio::pin!(collision);
        assert!(futures_util::poll!(&mut collision).is_pending());
        collision_started_tx.send(()).ok();
        assert!(collision.await.is_ok());
    });
    collision_started_rx.await.unwrap();
    assert_eq!(*collision_attempts.lock().unwrap(), 1);
    assert_eq!(*attempts.lock().unwrap(), 1);

    let queued = tokio::spawn(fetch_sanitized_image_using(url, false, validate, request));
    tokio::task::yield_now().await;
    assert!(!queued.is_finished());
    assert_eq!(*attempts.lock().unwrap(), 1);

    tokio::time::advance(cooldown - std::time::Duration::from_millis(1)).await;
    assert!(!first.is_finished());
    assert!(!queued.is_finished());
    assert_eq!(*attempts.lock().unwrap(), 1);

    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    let (first, queued) = tokio::join!(first, queued);
    assert!(first.unwrap().is_ok());
    assert!(queued.unwrap().is_ok());
    assert_eq!(*attempts.lock().unwrap(), 3);
}

#[test]
fn image_cooldown_wait_is_short_and_one_shot() {
    let url = Url::parse("https://bounded-cooldown.example/image.png").unwrap();
    let mut waited = false;
    assert_eq!(
        retryable_image_cooldown(&url, Some(MAX_INLINE_IMAGE_COOLDOWN), &mut waited,),
        Some(MAX_INLINE_IMAGE_COOLDOWN)
    );
    assert!(waited);
    assert_eq!(
        retryable_image_cooldown(&url, Some(MAX_INLINE_IMAGE_COOLDOWN), &mut waited,),
        None
    );
    let excessive_url = Url::parse("https://excessive-cooldown.example/image.png").unwrap();
    let mut excessive_waited = false;
    assert_eq!(
        retryable_image_cooldown(
            &excessive_url,
            Some(MAX_INLINE_IMAGE_COOLDOWN + std::time::Duration::from_secs(1)),
            &mut excessive_waited,
        ),
        None
    );
    assert!(!excessive_waited);
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
