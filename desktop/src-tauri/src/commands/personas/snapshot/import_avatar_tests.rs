use super::materialize_import_avatar;
use std::cell::Cell;

#[tokio::test]
async fn inline_avatar_is_uploaded_and_replaced_with_hosted_url() {
    let uploaded = Cell::new(false);
    let result = materialize_import_avatar(
        Some("data:image/png;base64,iVBORw0KGgo="),
        Some("https://sender.invalid/avatar.png"),
        |bytes| {
            uploaded.set(true);
            async move {
                assert_eq!(bytes, b"\x89PNG\r\n\x1a\n");
                Ok("https://relay.example/media/avatar.png".to_string())
            }
        },
    )
    .await
    .unwrap();

    assert!(uploaded.get());
    assert_eq!(
        result.as_deref(),
        Some("https://relay.example/media/avatar.png")
    );
}

#[tokio::test]
async fn hosted_avatar_skips_upload() {
    let result =
        materialize_import_avatar(None, Some("https://sender.example/avatar.png"), |_| async {
            panic!("hosted avatars must not be uploaded")
        })
        .await
        .unwrap();

    assert_eq!(result.as_deref(), Some("https://sender.example/avatar.png"));
}

#[tokio::test]
async fn relay_sized_inline_avatar_becomes_bounded_signed_profile() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use image::ImageEncoder;
    use nostr::JsonUtil;

    let mut pixels = vec![0_u8; 512 * 512 * 4];
    let mut seed = 0x1234_5678_u32;
    for byte in &mut pixels {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *byte = seed as u8;
    }
    let mut source = Vec::new();
    image::codecs::png::PngEncoder::new(&mut source)
        .write_image(&pixels, 512, 512, image::ExtendedColorType::Rgba8)
        .unwrap();
    assert!(source.len() > 256 * 1024);
    let data_url = format!("data:image/png;base64,{}", STANDARD.encode(&source));
    assert!(data_url.len() > 256 * 1024);

    let avatar = materialize_import_avatar(Some(&data_url), None, |bytes| async move {
        let mime = crate::commands::media::detect_and_validate_mime(&bytes)?;
        assert_eq!(mime, "image/png");
        let sanitized = crate::commands::media::sanitize_image_for_upload(bytes, &mime)?;
        image::load_from_memory(&sanitized).map_err(|error| error.to_string())?;
        Ok("https://relay.example/media/avatar.png".to_string())
    })
    .await
    .unwrap()
    .unwrap();

    let event =
        crate::events::build_profile(Some("Imported agent"), None, Some(&avatar), None, None)
            .unwrap()
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap();
    assert!(event.content.len() < 64 * 1024);
    assert!(!event.content.contains("data:image/"));
    assert!(event
        .content
        .contains("https://relay.example/media/avatar.png"));
    assert!(event.as_json().len() < 256 * 1024);
}

#[tokio::test]
async fn upload_failure_aborts_avatar_materialization() {
    let result = materialize_import_avatar(
        Some("data:image/png;base64,iVBORw0KGgo="),
        None,
        |_| async { Err("relay upload failed".to_string()) },
    )
    .await;

    assert_eq!(result.unwrap_err(), "relay upload failed");
}

#[tokio::test]
async fn malformed_inline_avatar_fails_before_upload() {
    let result =
        materialize_import_avatar(Some("data:image/png;base64,not-base64!"), None, |_| async {
            panic!("malformed avatars must not be uploaded")
        })
        .await;

    assert_eq!(result.unwrap_err(), "Snapshot avatar data is malformed.");
}
