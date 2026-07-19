use super::*;

#[test]
fn test_extract_server_authority_default_ports() {
    assert_eq!(
        extract_server_authority("https://relay.example.com"),
        Some("relay.example.com".to_string())
    );
    assert_eq!(
        extract_server_authority("https://relay.example.com:443"),
        Some("relay.example.com".to_string())
    );
    assert_eq!(
        extract_server_authority("http://relay.example.com:80"),
        Some("relay.example.com".to_string())
    );
}

#[test]
fn test_extract_server_authority_non_default_ports() {
    assert_eq!(
        extract_server_authority("http://localhost:3000"),
        Some("localhost:3000".to_string())
    );
    assert_eq!(
        extract_server_authority("https://relay.example.com:8443"),
        Some("relay.example.com:8443".to_string())
    );
}

#[test]
fn test_extract_server_authority_ipv6() {
    assert_eq!(
        extract_server_authority("http://[::1]:3000"),
        Some("[::1]:3000".to_string())
    );
}

#[test]
fn test_extract_server_authority_invalid() {
    assert_eq!(extract_server_authority("not-a-url"), None);
    assert_eq!(extract_server_authority(""), None);
}

#[test]
fn test_sign_blossom_get_auth_header_shape() {
    let keys = Keys::generate();
    let header = sign_blossom_get_auth_header(&keys, "http://localhost:3000", 600).unwrap();
    let b64 = header.strip_prefix("Nostr ").expect("Nostr scheme prefix");
    let json = URL_SAFE_NO_PAD.decode(b64).unwrap();
    let event = nostr::Event::from_json(std::str::from_utf8(&json).unwrap()).unwrap();

    assert_eq!(event.kind, Kind::from(24242));
    event.verify().expect("valid signature");

    let tag = |name: &str| -> Option<String> {
        event.tags.iter().find_map(|t| {
            let v = t.as_slice();
            (v.first().map(String::as_str) == Some(name)).then(|| v[1].clone())
        })
    };
    assert_eq!(tag("t").as_deref(), Some("get"));
    assert_eq!(tag("server").as_deref(), Some("localhost:3000"));
    // Server-scoped token: no x tag (BUD-01 allows x OR server).
    assert!(tag("x").is_none());
    let expiration: u64 = tag("expiration").unwrap().parse().unwrap();
    let now = Timestamp::now().as_secs();
    assert!(expiration > now && expiration <= now + 600);
}

#[test]
fn test_sign_blossom_get_auth_header_invalid_base_url() {
    let keys = Keys::generate();
    assert!(sign_blossom_get_auth_header(&keys, "not-a-url", 600).is_err());
}

#[test]
fn test_detect_and_validate_mime_jpeg() {
    // Minimal JPEG: SOI + EOI
    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0];
    assert_eq!(detect_and_validate_mime(&jpeg).unwrap(), "image/jpeg");
}

#[test]
fn test_detect_and_validate_mime_accepts_text_as_octet_stream() {
    // Plain text has no magic bytes — infer returns None, so it's accepted
    // as opaque binary (served as a download). This is the common Slack case.
    let text = b"hello world";
    assert_eq!(
        detect_and_validate_mime(text).unwrap(),
        "application/octet-stream"
    );
}

#[test]
fn test_detect_and_validate_mime_rejects_html() {
    let html = b"<!DOCTYPE html><html><body><script>alert(1)</script></body></html>";
    assert!(detect_and_validate_mime(html).is_err());
}

#[test]
fn test_image_sanitizer_bakes_exif_orientation() {
    let source = image::RgbImage::from_fn(2, 3, |x, y| {
        image::Rgb([(x * 80) as u8, (y * 60) as u8, 32])
    });
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 95)
        .encode_image(&source)
        .unwrap();

    // Minimal little-endian Exif IFD with Orientation=6 (rotate 90°).
    let mut exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0".to_vec();
    exif.extend_from_slice(&[
        0x12, 0x01, // Orientation tag
        0x03, 0x00, // SHORT
        0x01, 0x00, 0x00, 0x00, // count=1
        0x06, 0x00, 0x00, 0x00, // value=6
        0x00, 0x00, 0x00, 0x00, // next IFD
    ]);
    let segment_len = (exif.len() + 2) as u16;
    let mut oriented = encoded[..2].to_vec();
    oriented.extend_from_slice(&[0xff, 0xe1]);
    oriented.extend_from_slice(&segment_len.to_be_bytes());
    oriented.extend_from_slice(&exif);
    oriented.extend_from_slice(&encoded[2..]);

    let sanitized = sanitize_image_for_upload(oriented, "image/jpeg").unwrap();
    let decoded =
        image::load_from_memory_with_format(&sanitized, image::ImageFormat::Jpeg).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (3, 2));
    assert!(!sanitized.windows(6).any(|bytes| bytes == b"Exif\0\0"));
}

#[test]
fn test_animated_png_and_webp_are_not_flattened() {
    let mut apng = b"\x89PNG\r\n\x1a\n".to_vec();
    apng.extend_from_slice(&8u32.to_be_bytes());
    apng.extend_from_slice(b"acTL");
    apng.extend_from_slice(&[0; 8]);
    apng.extend_from_slice(&[0; 4]);
    assert!(is_animated_image(&apng, "image/png"));
    assert_eq!(
        sanitize_image_for_upload(apng.clone(), "image/png").unwrap(),
        apng
    );

    let mut webp = b"RIFF\x0c\0\0\0WEBPANIM".to_vec();
    webp.extend_from_slice(&0u32.to_le_bytes());
    assert!(is_animated_image(&webp, "image/webp"));
    assert_eq!(
        sanitize_image_for_upload(webp.clone(), "image/webp").unwrap(),
        webp
    );
}

#[test]
fn test_legacy_upload_retry_statuses_are_narrow() {
    assert!(should_retry_legacy_upload(reqwest::StatusCode::NOT_FOUND));
    assert!(should_retry_legacy_upload(
        reqwest::StatusCode::METHOD_NOT_ALLOWED
    ));
    assert!(!should_retry_legacy_upload(
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    ));
    assert!(!should_retry_legacy_upload(
        reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE
    ));
}

#[test]
fn test_sanitize_filename() {
    assert_eq!(sanitize_filename("report.pdf"), "report.pdf");
    // Strips directory components and traversal.
    assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
    assert_eq!(sanitize_filename("/abs/path/notes.txt"), "notes.txt");
    assert_eq!(sanitize_filename(r"C:\Users\me\doc.docx"), "doc.docx");
    // Empty / separator-only falls back.
    assert_eq!(sanitize_filename(""), "file");
    assert_eq!(sanitize_filename("/"), "file");
    // Control chars removed.
    assert_eq!(sanitize_filename("a\nb\tc.txt"), "abc.txt");
}
