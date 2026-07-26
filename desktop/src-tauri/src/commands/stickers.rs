//! Sticker authoring commands: validated sticker image upload and Signal
//! sticker-pack import. Split out of `media.rs` to keep that module under the
//! 1000-line file-size ceiling.

use serde::Serialize;
use tauri::State;
use zeroize::Zeroize;

use crate::app_state::AppState;
use crate::relay::relay_api_base_url_with_override;

use super::media::{do_upload, sanitize_filename, BlobDescriptor};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedStickerDraft {
    identifier: String,
    title: String,
    author: Option<String>,
    cover: Option<ImportedStickerAsset>,
    stickers: Vec<ImportedStickerAsset>,
    skipped_sticker_ids: Vec<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedStickerAsset {
    shortcode: String,
    url: String,
    sha256: String,
    mime: String,
    width: Option<u32>,
    height: Option<u32>,
    alt: Option<String>,
    emoji: Option<String>,
}

const MAX_SONAR_STICKER_BYTES: usize = 4 * 1024 * 1024;

fn require_https_sticker_relay(state: &AppState) -> Result<(), String> {
    let base_url = relay_api_base_url_with_override(state);
    let parsed =
        url::Url::parse(&base_url).map_err(|_| "The active relay URL is invalid.".to_string())?;
    if parsed.scheme() != "https" {
        return Err("Sonar sticker assets require an HTTPS relay.".to_string());
    }
    Ok(())
}

fn validate_sticker_image(bytes: &[u8], cover_only: bool) -> Result<(String, u32, u32), String> {
    if bytes.is_empty() || bytes.len() > MAX_SONAR_STICKER_BYTES {
        return Err("Sticker images must be non-empty and at most 4 MiB.".to_string());
    }
    let sniffed = infer::get(bytes)
        .map(|kind| kind.mime_type())
        .ok_or_else(|| "Could not recognize that sticker image.".to_string())?;
    let mime =
        if sniffed == "image/png" && buzz_core_pkg::stickers::apng_frame_count(bytes).is_some() {
            "image/apng"
        } else {
            sniffed
        };
    if !matches!(
        mime,
        "image/webp" | "image/png" | "image/apng" | "image/gif"
    ) {
        return Err("Sonar stickers must be WebP, PNG, APNG, or GIF.".to_string());
    }
    if cover_only && mime != "image/webp" {
        return Err(
            "Sticker pack covers must be WebP because the Sonar image tag has no MIME field."
                .to_string(),
        );
    }
    // Read geometry from the bounded file header before any pixel allocation.
    // The upload path does not need decoded pixels, so avoid a full decode.
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Sticker image data is invalid.".to_string())?
        .into_dimensions()
        .map_err(|_| "Sticker image data is invalid.".to_string())?;
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("Sticker dimensions must be between 1x1 and 4096x4096.".to_string());
    }
    Ok((mime.to_string(), width, height))
}

fn validate_official_signal_sticker_link(link: &str) -> bool {
    url::Url::parse(link).is_ok_and(|parsed| {
        parsed.scheme() == "https"
            && parsed.host_str() == Some("signal.art")
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.port().is_none()
            && parsed.path().trim_end_matches('/') == "/addstickers"
            && sonar_stickers::signal::SignalPackLink::parse(link).is_ok()
    })
}

async fn upload_imported_signal_sticker(
    sticker: sonar_stickers::signal::ImportedSignalSticker,
    state: &State<'_, AppState>,
    cover_only: bool,
) -> Result<ImportedStickerAsset, String> {
    let (mime, width, height) = validate_sticker_image(&sticker.bytes, cover_only)?;
    let descriptor = do_upload(sticker.bytes, &mime, state, None).await?;
    Ok(ImportedStickerAsset {
        shortcode: sticker.shortcode,
        url: descriptor.url,
        sha256: descriptor.sha256,
        mime,
        width: Some(width),
        height: Some(height),
        alt: Some(format!("Sticker {}", sticker.id)),
        emoji: sticker.emoji,
    })
}

/// Pick and upload one strictly validated Sonar sticker asset.
///
/// Unlike the generic image picker, this command rejects JPEG, images above
/// 4 MiB or 4096 pixels, and non-WebP covers before any network request.
#[tauri::command]
pub async fn pick_and_upload_sticker_image(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    cover_only: bool,
) -> Result<Option<BlobDescriptor>, String> {
    use tauri_plugin_dialog::DialogExt;

    require_https_sticker_relay(&state)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Sonar sticker images", &["webp", "png", "apng", "gif"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(file_path) = rx.await.map_err(|_| "dialog cancelled".to_string())? else {
        return Ok(None);
    };
    let path = file_path.as_path().ok_or("invalid path")?.to_path_buf();
    let read_path = path.clone();
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        use std::io::Read;

        // Inspect and read through the same open handle so a path swap cannot
        // change the inode between validation and upload.
        let file = std::fs::File::open(&read_path)
            .map_err(|error| format!("Could not open sticker image: {error}"))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("Could not inspect sticker image: {error}"))?;
        if !metadata.is_file() || metadata.len() > MAX_SONAR_STICKER_BYTES as u64 {
            return Err("Sticker images must be regular files no larger than 4 MiB.".to_string());
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_SONAR_STICKER_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Could not read sticker image: {error}"))?;
        if bytes.len() > MAX_SONAR_STICKER_BYTES {
            return Err("Sticker images must be regular files no larger than 4 MiB.".to_string());
        }
        Ok(bytes)
    })
    .await
    .map_err(|error| format!("Sticker image reader failed: {error}"))??;
    let (mime, _, _) = validate_sticker_image(&bytes, cover_only)?;
    let mut descriptor = do_upload(bytes, &mime, &state, None).await?;
    descriptor.filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(sanitize_filename);
    Ok(Some(descriptor))
}

/// Import and decrypt a Signal sticker pack entirely in trusted Rust, then
/// upload the authenticated plaintext assets through Buzz's existing Blossom
/// path. The Signal link (including its pack key) is consumed by this command
/// and is never returned, persisted, or logged.
#[tauri::command]
pub async fn import_signal_sticker_pack(
    mut signal_link: String,
    state: State<'_, AppState>,
) -> Result<ImportedStickerDraft, String> {
    require_https_sticker_relay(&state)?;
    if !validate_official_signal_sticker_link(signal_link.trim()) {
        signal_link.zeroize();
        return Err("Enter a valid https://signal.art/addstickers/ link.".to_string());
    }
    let imported_result = sonar_stickers::signal::import_signal_pack_with_options(
        signal_link.trim(),
        sonar_stickers::signal::SignalImportOptions {
            accept_invalid_certs: false,
            skip_failed_stickers: true,
        },
    )
    .await;
    signal_link.zeroize();
    let imported = imported_result.map_err(|_| {
        "Could not import that Signal sticker pack. Check the link and try again.".to_string()
    })?;

    let mut stickers = Vec::with_capacity(imported.stickers.len());
    for sticker in imported.stickers {
        stickers.push(upload_imported_signal_sticker(sticker, &state, false).await?);
    }
    let cover = match imported.cover {
        Some(cover) => upload_imported_signal_sticker(cover, &state, true)
            .await
            .ok(),
        None => None,
    };

    Ok(ImportedStickerDraft {
        identifier: imported.pack_id,
        title: imported.title,
        author: imported.author,
        cover,
        stickers,
        skipped_sticker_ids: imported.skipped_sticker_ids,
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sonar_sticker_validator_rejects_jpeg_and_non_webp_cover() {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("encode png");
        assert_eq!(
            validate_sticker_image(png.get_ref(), false)
                .expect("ordinary png sticker")
                .0,
            "image/png"
        );
        assert!(validate_sticker_image(png.get_ref(), true).is_err());

        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .expect("encode jpeg");
        assert!(validate_sticker_image(jpeg.get_ref(), false).is_err());
    }

    #[test]
    fn signal_sticker_link_requires_official_https_host() {
        let valid = format!(
            "https://signal.art/addstickers/#pack_id={}&pack_key={}",
            "a".repeat(32),
            "b".repeat(64)
        );
        assert!(validate_official_signal_sticker_link(&valid));
        assert!(!validate_official_signal_sticker_link(
            &valid.replace("signal.art", "example.com")
        ));
        assert!(!validate_official_signal_sticker_link(
            &valid.replace("https://", "http://")
        ));
    }
}
