//! Sticker authoring commands: validated sticker image upload, Signal
//! sticker-pack import, and Nostr sticker-pack import from public relays.
//! Split out of `media.rs` to keep that module under the 1000-line file-size
//! ceiling.

use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tauri::State;
use zeroize::Zeroize;

use crate::app_state::AppState;
use crate::relay::relay_api_base_url_with_override;

use super::media::{do_upload, sanitize_filename, BlobDescriptor};
use super::sticker_relay::{fetch_events, validate_relay_hint, RelayHint};

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

/// Map an import failure onto a message that points at the actual cause.
///
/// The link is validated against the official `signal.art` shape before any
/// request is made, so once we are here the link's *form* is already known
/// good. Reporting every downstream failure as "check the link" therefore
/// sends the user to re-examine the one thing that cannot be wrong. A
/// transport failure in particular is worth calling out on its own:
/// `cdn.signal.org` is served under Signal's own private root CA, so a client
/// trusting only the public roots cannot complete the handshake — the link is
/// fine and re-entering it will never help.
fn signal_import_error_message(error: &sonar_stickers::StickerError) -> String {
    match error {
        sonar_stickers::StickerError::Http(_) => {
            "Could not reach Signal to download that pack. This is a network or \
             TLS failure, not a problem with the link."
                .to_string()
        }
        sonar_stickers::StickerError::Crypto(_) => {
            "Could not decrypt that Signal pack — the pack key in the link may be wrong."
                .to_string()
        }
        _ => "Could not import that Signal sticker pack.".to_string(),
    }
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
    // Safe to log: Signal's URLs carry the pack *id*, never the pack key —
    // the key is only used locally to decrypt what comes back. Discarding the
    // error entirely left every failure indistinguishable, which is how a TLS
    // handshake failure came to be reported as a bad link.
    let imported = imported_result.map_err(|error| {
        eprintln!("buzz-desktop: signal sticker import failed: {error}");
        signal_import_error_message(&error)
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
    relays: Vec<RelayHint>,
}

/// Parse a Sonar sticker pack link
/// (`https://…/stickers?a=30031:<author>:<identifier>&relay=wss://…`) or a
/// bare `30031:<author>:<identifier>` coordinate. Relay hints must pass
/// [`validate_relay_hint`] (wss-only outside local development, no
/// credentials) and are capped so a crafted link cannot fan the app out to
/// dozens of sockets.
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
            valid_relays.push(validate_relay_hint(relay)?);
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
    relays: &[RelayHint],
) -> Result<sonar_stickers::StickerPack, String> {
    for relay in relays {
        if let Ok(Some(pack)) = fetch_nostr_sticker_pack_from_relay(address, relay).await {
            return Ok(pack);
        }
    }
    Err("Could not find that sticker pack on the link's relays.".to_string())
}

/// Canonical replaceable-event head ordering: newest `created_at` wins, and
/// on a timestamp tie the lexicographically smallest event ID wins. Mirrors
/// `canonical_head` in `crates/buzz-cli/src/commands/stickers.rs` so imported
/// packs resolve the same head regardless of relay arrival order.
fn replaces_head(candidate: &nostr::Event, current: &nostr::Event) -> bool {
    candidate.created_at > current.created_at
        || (candidate.created_at == current.created_at && candidate.id < current.id)
}

/// Keep `candidate` only if it is the requested kind and author, carries a
/// valid signature, and beats the current head. Relay responses are otherwise
/// unauthenticated: a hostile hint can return an event that merely claims the
/// requested kind and pubkey with a forged ID or signature, and the importer
/// republishes the pack under the importing user's identity.
fn accept_pack_candidate(
    candidate: nostr::Event,
    address: &sonar_stickers::PackAddress,
    head: &mut Option<nostr::Event>,
) {
    if candidate.kind.as_u16() != sonar_stickers::STICKER_PACK_KIND
        || candidate.pubkey.to_hex() != address.author_pubkey_hex
        || candidate.verify().is_err()
    {
        return;
    }
    if head
        .as_ref()
        .is_none_or(|current| replaces_head(&candidate, current))
    {
        *head = Some(candidate);
    }
}

async fn fetch_nostr_sticker_pack_from_relay(
    address: &sonar_stickers::PackAddress,
    relay: &RelayHint,
) -> Result<Option<sonar_stickers::StickerPack>, String> {
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
    // PACK_FETCH_TIMEOUT is the whole per-hint budget (DNS, connect, request,
    // read loop), so a link full of slow hints cannot multiply it.
    let events = fetch_events(relay, &request, subscription_id, PACK_FETCH_TIMEOUT).await?;
    let mut newest: Option<nostr::Event> = None;
    for event in events {
        accept_pack_candidate(event, address, &mut newest);
    }
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

    fn signed_pack_event(keys: &nostr::Keys, created_at: u64, content: &str) -> nostr::Event {
        nostr::EventBuilder::new(
            nostr::Kind::Custom(sonar_stickers::STICKER_PACK_KIND),
            content,
        )
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(keys)
        .expect("sign event")
    }

    #[test]
    fn canonical_head_prefers_newest_timestamp() {
        let keys = nostr::Keys::generate();
        let older = signed_pack_event(&keys, 100, "old-pack");
        let newer = signed_pack_event(&keys, 200, "new-pack");
        assert!(replaces_head(&newer, &older));
        assert!(!replaces_head(&older, &newer));
        assert!(!replaces_head(&older, &older));
    }

    #[test]
    fn canonical_head_ties_break_to_smallest_event_id() {
        let keys = nostr::Keys::generate();
        let a = signed_pack_event(&keys, 100, "pack-a");
        let b = signed_pack_event(&keys, 100, "pack-b");
        // Distinct signatures ⇒ distinct IDs; whichever is smaller must win.
        assert_ne!(a.id, b.id);
        let (small, large) = if a.id < b.id { (a, b) } else { (b, a) };
        assert!(replaces_head(&small, &large));
        assert!(!replaces_head(&large, &small));
    }

    #[test]
    fn tampered_event_fails_verification() {
        let keys = nostr::Keys::generate();
        let mut event = signed_pack_event(&keys, 100, "pack");
        assert!(event.verify().is_ok());
        // Forging the content (or any field) invalidates the Schnorr
        // signature over the event ID — the import path must skip it.
        event.content = "forged pack".into();
        assert!(event.verify().is_err());
    }

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
            parsed
                .relays
                .iter()
                .map(|relay| relay.url.clone())
                .collect::<Vec<_>>(),
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
        // plaintext ws hint to a non-loopback host (SSRF / downgrade)
        assert!(parse_nostr_sticker_pack_link(&format!(
            "https://sonarprivacy.xyz/stickers?a=30031:{AUTHOR}:x&relay=ws%3A%2F%2F10.0.0.1%3A3000"
        ))
        .is_err());
        // relay hint carrying credentials
        assert!(parse_nostr_sticker_pack_link(&format!(
            "https://sonarprivacy.xyz/stickers?a=30031:{AUTHOR}:x&relay=wss%3A%2F%2Fu%3Ap%40relay.example"
        ))
        .is_err());
        // garbage
        assert!(parse_nostr_sticker_pack_link("hello world").is_err());
    }

    #[test]
    fn pack_candidates_must_carry_a_valid_signature_from_the_requested_author() {
        let keys = nostr::Keys::generate();
        let address = sonar_stickers::PackAddress::parse(&format!(
            "30031:{}:pack",
            keys.public_key().to_hex()
        ))
        .expect("address");

        let mut head = None;
        let mut forged = signed_pack_event(&keys, 300, "pack");
        forged.content = "forged pack".into();
        accept_pack_candidate(forged, &address, &mut head);
        assert!(head.is_none(), "forged signature must be rejected");

        // Correctly signed, but by a different key than the link's author.
        let impostor = nostr::Keys::generate();
        accept_pack_candidate(
            signed_pack_event(&impostor, 300, "pack"),
            &address,
            &mut head,
        );
        assert!(head.is_none(), "wrong author must be rejected");

        let genuine = signed_pack_event(&keys, 100, "pack");
        accept_pack_candidate(genuine.clone(), &address, &mut head);
        assert_eq!(head.as_ref().map(|event| event.id), Some(genuine.id));
    }

    #[test]
    fn pack_candidates_resolve_the_canonical_head_regardless_of_arrival_order() {
        let keys = nostr::Keys::generate();
        let address = sonar_stickers::PackAddress::parse(&format!(
            "30031:{}:pack",
            keys.public_key().to_hex()
        ))
        .expect("address");
        let a = signed_pack_event(&keys, 100, "pack-a");
        let b = signed_pack_event(&keys, 100, "pack-b");
        let expected = a.id.min(b.id);

        for order in [vec![a.clone(), b.clone()], vec![b, a]] {
            let mut head = None;
            for event in order {
                accept_pack_candidate(event, &address, &mut head);
            }
            assert_eq!(head.map(|event| event.id), Some(expected));
        }
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

    /// A transport failure must not be reported as a bad link. `cdn.signal.org`
    /// is served under Signal's own private root CA, so a client trusting only
    /// the public roots fails the handshake on a perfectly valid link — the
    /// old blanket "check the link and try again" sent users to re-examine the
    /// one thing already validated before the request was made.
    #[test]
    fn transport_failure_is_not_reported_as_a_bad_link() {
        let message = signal_import_error_message(&sonar_stickers::StickerError::Http(
            "GET https://cdn.signal.org/stickers/abc/manifest.proto: invalid peer certificate"
                .to_string(),
        ));
        assert!(message.contains("network or TLS"), "{message}");
        assert!(
            !message.to_ascii_lowercase().contains("check the link"),
            "{message}"
        );
    }

    #[test]
    fn decrypt_failure_points_at_the_pack_key() {
        let message = signal_import_error_message(&sonar_stickers::StickerError::Crypto(
            "AES-CBC decrypt failed".to_string(),
        ));
        assert!(message.contains("pack key"), "{message}");
    }

    /// The underlying detail is logged, never surfaced — it can carry the pack
    /// id and upstream URLs, which is diagnostic noise for a user-facing toast.
    #[test]
    fn user_facing_message_never_leaks_the_underlying_detail() {
        let secret = "GET https://cdn.signal.org/stickers/deadbeef/manifest.proto";
        let message =
            signal_import_error_message(&sonar_stickers::StickerError::Http(secret.to_string()));
        assert!(!message.contains(secret), "{message}");
        assert!(!message.contains("cdn.signal.org"), "{message}");
    }
}
