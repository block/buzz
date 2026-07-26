//! Sticker authoring commands: validated sticker image upload, Signal
//! sticker-pack import, and Nostr sticker-pack import from public relays.
//! Split out of `media.rs` to keep that module under the 1000-line file-size
//! ceiling.

use std::time::Duration;

use buzz_ws_client::{NostrWsConnection, RelayMessage};
use serde::Serialize;
use serde_json::json;
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
    description: Option<String>,
    license: Option<String>,
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
    // Optional fields must be omitted (not null) so the frontend's
    // `=== undefined` checks in publishStickerPack behave.
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
        description: None,
        license: None,
        cover,
        stickers,
        skipped_sticker_ids: imported.skipped_sticker_ids,
    })
}

// ── Nostr pack link import ─────────────────────────────────────────────────

/// Maximum relay hints accepted from a sticker pack link.
const MAX_PACK_LINK_RELAYS: usize = 8;
/// Per-relay connect + query budget when fetching a pack from public relays.
const PACK_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

struct NostrStickerPackLink {
    address: sonar_stickers::PackAddress,
    relays: Vec<String>,
}

/// Parse a Sonar sticker pack link
/// (`https://…/stickers?a=30031:<author>:<identifier>&relay=wss://…`) or a
/// bare `30031:<author>:<identifier>` coordinate. Relay hints must be ws(s)
/// URLs and are capped so a crafted link cannot fan the app out to dozens of
/// sockets.
fn parse_nostr_sticker_pack_link(input: &str) -> Result<NostrStickerPackLink, String> {
    const INVALID: &str = "Enter a Sonar sticker pack link \
        (https://…/stickers?a=30031:…) or a 30031:<author>:<identifier> coordinate.";
    let trimmed = input.trim();
    if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        let url = url::Url::parse(trimmed).map_err(|_| INVALID.to_string())?;
        if url.scheme() != "https" {
            return Err("Sticker pack links must use HTTPS.".to_string());
        }
        let mut coordinate: Option<String> = None;
        let mut relays: Vec<String> = Vec::new();
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "a" => coordinate = Some(value.into_owned()),
                "relay" => relays.push(value.into_owned()),
                _ => {}
            }
        }
        let coordinate = coordinate.ok_or_else(|| INVALID.to_string())?;
        let address =
            sonar_stickers::PackAddress::parse(&coordinate).map_err(|_| INVALID.to_string())?;
        let mut valid_relays = Vec::with_capacity(relays.len());
        for relay in &relays {
            let parsed =
                url::Url::parse(relay).map_err(|_| format!("Invalid relay hint: {relay}"))?;
            if !matches!(parsed.scheme(), "wss" | "ws") {
                return Err(format!("Relay hints must be ws(s) URLs: {relay}"));
            }
            valid_relays.push(relay.clone());
        }
        valid_relays.truncate(MAX_PACK_LINK_RELAYS);
        return Ok(NostrStickerPackLink {
            address,
            relays: valid_relays,
        });
    }
    let address = sonar_stickers::PackAddress::parse(trimmed).map_err(|_| INVALID.to_string())?;
    Ok(NostrStickerPackLink {
        address,
        relays: Vec::new(),
    })
}

/// Fetch the newest kind:30031 event for `address` from the link's relay
/// hints. Read-only unauthenticated REQs; the first hint yielding a parseable
/// pack wins, so offline or censoring relays fall through to the next one.
async fn fetch_nostr_sticker_pack(
    address: &sonar_stickers::PackAddress,
    relays: &[String],
) -> Result<sonar_stickers::StickerPack, String> {
    for relay in relays {
        if let Ok(Some(pack)) = fetch_nostr_sticker_pack_from_relay(address, relay).await {
            return Ok(pack);
        }
    }
    Err("Could not find that sticker pack on the link's relays.".to_string())
}

async fn fetch_nostr_sticker_pack_from_relay(
    address: &sonar_stickers::PackAddress,
    relay: &str,
) -> Result<Option<sonar_stickers::StickerPack>, String> {
    let mut conn = tokio::time::timeout(PACK_FETCH_TIMEOUT, NostrWsConnection::connect(relay))
        .await
        .map_err(|_| "relay connect timed out".to_string())?
        .map_err(|error| error.to_string())?;
    let subscription_id = "sonar-sticker-import";
    let request = json!([
        "REQ",
        subscription_id,
        {
            "kinds": [sonar_stickers::STICKER_PACK_KIND],
            "authors": [address.author_pubkey_hex],
            "#d": [address.identifier],
            "limit": 4,
        }
    ]);
    conn.send_raw(&request)
        .await
        .map_err(|error| error.to_string())?;
    let mut newest: Option<nostr::Event> = None;
    let deadline = tokio::time::Instant::now() + PACK_FETCH_TIMEOUT;
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            break;
        }
        let message = match conn.next_event(deadline - now).await {
            Ok(message) => message,
            Err(_) => break,
        };
        match message {
            RelayMessage::Event {
                subscription_id: id,
                event,
            } if id == subscription_id
                && event.kind.as_u16() == sonar_stickers::STICKER_PACK_KIND
                && event.pubkey.to_hex() == address.author_pubkey_hex
                && newest
                    .as_ref()
                    .is_none_or(|current| event.created_at > current.created_at) =>
            {
                newest = Some(*event);
            }
            RelayMessage::Eose {
                subscription_id: id,
            } if id == subscription_id => break,
            RelayMessage::Closed {
                subscription_id: id,
                ..
            } if id == subscription_id => break,
            _ => {}
        }
    }
    let _ = conn.disconnect().await;
    let Some(event) = newest else {
        return Ok(None);
    };
    let pack = sonar_stickers::parse_pack_event(&event).map_err(|error| error.to_string())?;
    // The relay matched on author + #d loosely; verify the parsed pack is the
    // exact address the user asked for.
    if pack.address != *address {
        return Ok(None);
    }
    Ok(Some(pack))
}

fn imported_asset_from_sonar(sticker: sonar_stickers::Sticker) -> ImportedStickerAsset {
    ImportedStickerAsset {
        shortcode: sticker.shortcode,
        url: sticker.url,
        sha256: sticker.sha256,
        mime: sticker.mime,
        width: sticker.width,
        height: sticker.height,
        alt: sticker.alt,
        emoji: sticker.emoji,
    }
}

/// Import a Sonar/Nostr sticker pack already published on public relays. The
/// link carries no secrets, so unlike the Signal path nothing is zeroized;
/// the pack's content-addressed asset URLs are kept as-is (the relay's
/// approval-gated sticker cache re-fetches and hash-verifies them), and the
/// importer republishes the pack under their own key through the normal
/// publish path.
#[tauri::command]
pub async fn import_nostr_sticker_pack(
    link: String,
    state: State<'_, AppState>,
) -> Result<ImportedStickerDraft, String> {
    require_https_sticker_relay(&state)?;
    let parsed = parse_nostr_sticker_pack_link(&link)?;
    if parsed.relays.is_empty() {
        return Err(
            "That pack link has no relay hints. Use a link that includes relay= parameters."
                .to_string(),
        );
    }
    let pack = fetch_nostr_sticker_pack(&parsed.address, &parsed.relays).await?;
    Ok(ImportedStickerDraft {
        identifier: pack.address.identifier,
        title: pack.title,
        author: Some(pack.address.author_pubkey_hex),
        description: pack.description,
        license: pack.license,
        cover: pack.cover.map(imported_asset_from_sonar),
        stickers: pack
            .stickers
            .into_iter()
            .map(imported_asset_from_sonar)
            .collect(),
        skipped_sticker_ids: Vec::new(),
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const AUTHOR: &str = "7215b2db8754494fd3452b7f2d28b56e23863b95446bf68d79f980a7ad5ec7cd";

    #[test]
    fn imported_asset_omits_none_fields_so_frontend_undefined_checks_pass() {
        let asset = ImportedStickerAsset {
            shortcode: "s0".into(),
            url: "https://blossom.example/abc.webp".into(),
            sha256: "a".repeat(64),
            mime: "image/webp".into(),
            width: None,
            height: None,
            alt: None,
            emoji: None,
        };
        let value = serde_json::to_value(&asset).expect("serialize");
        let object = value.as_object().expect("object");
        // publishStickerPack validates with `=== undefined`; serializing None
        // as null fails `null !== undefined && null < 1` for dimension-less
        // stickers (the common case for packs without dim fields).
        for key in ["width", "height", "alt", "emoji"] {
            assert!(!object.contains_key(key), "key {key} must be omitted");
        }
    }

    #[test]
    fn nostr_pack_link_parses_coordinate_and_relay_hints() {
        let link = format!(
            "https://sonarprivacy.xyz/stickers?a=30031:{AUTHOR}:signal-abc&relay=wss%3A%2F%2Frelay.damus.io&relay=wss%3A%2F%2Fnos.lol"
        );
        let parsed = parse_nostr_sticker_pack_link(&link).expect("valid link");
        assert_eq!(parsed.address.author_pubkey_hex, AUTHOR);
        assert_eq!(parsed.address.identifier, "signal-abc");
        assert_eq!(
            parsed.relays,
            vec![
                "wss://relay.damus.io".to_string(),
                "wss://nos.lol".to_string()
            ]
        );
    }

    #[test]
    fn nostr_pack_link_parses_bare_coordinate_without_relays() {
        let parsed = parse_nostr_sticker_pack_link(&format!("30031:{AUTHOR}:my-pack"))
            .expect("valid coordinate");
        assert_eq!(parsed.address.identifier, "my-pack");
        assert!(parsed.relays.is_empty());
    }

    #[test]
    fn nostr_pack_link_rejects_bad_inputs() {
        // http (not https) pack links
        assert!(
            parse_nostr_sticker_pack_link("http://sonarprivacy.xyz/stickers?a=30031:x:y").is_err()
        );
        // missing ?a= coordinate
        assert!(parse_nostr_sticker_pack_link(
            "https://sonarprivacy.xyz/stickers?relay=wss://nos.lol"
        )
        .is_err());
        // non-30031 coordinate
        assert!(parse_nostr_sticker_pack_link(&format!(
            "https://sonarprivacy.xyz/stickers?a=30030:{AUTHOR}:x"
        ))
        .is_err());
        // non-ws relay hint
        assert!(parse_nostr_sticker_pack_link(&format!(
            "https://sonarprivacy.xyz/stickers?a=30031:{AUTHOR}:x&relay=https%3A%2F%2Fevil.example"
        ))
        .is_err());
        // garbage
        assert!(parse_nostr_sticker_pack_link("hello world").is_err());
    }

    #[test]
    fn nostr_pack_link_caps_relay_hints() {
        let relays = (0..12)
            .map(|index| format!("relay=wss%3A%2F%2Fr{index}.example"))
            .collect::<Vec<_>>()
            .join("&");
        let link = format!("https://sonarprivacy.xyz/stickers?a=30031:{AUTHOR}:x&{relays}");
        let parsed = parse_nostr_sticker_pack_link(&link).expect("valid link");
        assert_eq!(parsed.relays.len(), MAX_PACK_LINK_RELAYS);
    }

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
