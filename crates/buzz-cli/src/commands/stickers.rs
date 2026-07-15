use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use nostr::{Event, EventBuilder, Kind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sonar_stickers::signal::{
    import_signal_pack_with_options, ImportedSignalSticker, SignalImportOptions, SignalPackLink,
};
use sonar_stickers::{
    build_installed_packs_tags, build_pack_tags, is_allowed_sticker_mime,
    parse_installed_pack_list, parse_pack_event, validate_shortcode, InstalledPackList,
    PackAddress, Sticker, StickerPack, STICKER_PACK_KIND, USER_STICKER_PACKS_KIND,
};
use zeroize::Zeroizing;

use crate::client::{normalize_write_response, BlobDescriptor, BuzzClient};
use crate::error::CliError;

const MAX_STICKER_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Serialize)]
struct PackSummary {
    address: String,
    title: String,
    description: Option<String>,
    cover_url: Option<String>,
    sticker_count: usize,
    available: bool,
    installed: bool,
    event_id: String,
}

/// JSON input for `stickers create` and `stickers update`.
///
/// Asset paths may be absolute or relative to the manifest file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackManifest {
    identifier: String,
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    cover_file: Option<PathBuf>,
    #[serde(default)]
    cover_alt: Option<String>,
    stickers: Vec<StickerManifest>,
    #[serde(default)]
    license: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StickerManifest {
    shortcode: String,
    file: PathBuf,
    #[serde(default)]
    alt: Option<String>,
    #[serde(default)]
    emoji: Option<String>,
}

#[derive(Clone)]
struct PreparedAsset {
    bytes: Vec<u8>,
    mime: String,
    sha256: String,
    width: u32,
    height: u32,
}

fn parse_events(raw: &str, context: &str) -> Result<Vec<Event>, CliError> {
    serde_json::from_str(raw)
        .map_err(|e| CliError::Other(format!("failed to parse {context}: {e}")))
}

fn sticker_error(context: &str, err: impl std::fmt::Display) -> CliError {
    CliError::Usage(format!("{context}: {err}"))
}

/// NIP-01 addressable-event head: newest timestamp, then smallest event id.
fn canonical_head(events: impl IntoIterator<Item = Event>) -> Option<Event> {
    events.into_iter().max_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| b.id.cmp(&a.id))
    })
}

async fn fetch_pack_event(
    client: &BuzzClient,
    address: &PackAddress,
) -> Result<Option<Event>, CliError> {
    let filter = serde_json::json!({
        "kinds": [STICKER_PACK_KIND],
        "authors": [address.author_pubkey_hex],
        "#d": [address.identifier],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let events = parse_events(&raw, "sticker pack query")?;
    Ok(canonical_head(
        events
            .into_iter()
            .filter(|event| parse_pack_event(event).is_ok_and(|pack| pack.address == *address))
            .collect::<Vec<_>>(),
    ))
}

async fn fetch_own_installed(client: &BuzzClient) -> Result<InstalledPackList, CliError> {
    let Some(event) = fetch_own_installed_event(client).await? else {
        return Ok(InstalledPackList::default());
    };
    parse_installed_pack_list(&event)
        .map_err(|e| CliError::Other(format!("invalid installed sticker list: {e}")))
}

async fn fetch_own_installed_event(client: &BuzzClient) -> Result<Option<Event>, CliError> {
    let filter = serde_json::json!({
        "kinds": [USER_STICKER_PACKS_KIND],
        "authors": [client.keys().public_key().to_hex()],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let events = parse_events(&raw, "installed sticker list query")?;
    Ok(canonical_head(events))
}

async fn fetch_approved_revisions(
    client: &BuzzClient,
) -> Result<HashMap<String, String>, CliError> {
    let filter = serde_json::json!({
        "kinds": [buzz_core::kind::KIND_STICKER_CATALOG],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let events = parse_events(&raw, "workspace sticker catalog query")?;
    let Some(event) = canonical_head(events) else {
        return Ok(HashMap::new());
    };
    approved_revisions_from_event(&event)
        .ok_or_else(|| CliError::Other("workspace sticker catalog is malformed".into()))
}

fn approved_revisions_from_event(event: &Event) -> Option<HashMap<String, String>> {
    if u32::from(event.kind.as_u16()) != buzz_core::kind::KIND_STICKER_CATALOG
        || !event.content.is_empty()
    {
        return None;
    }

    let mut saw_protected = false;
    let mut revisions = HashMap::new();
    for tag in event.tags.iter() {
        let fields = tag.as_slice();
        match fields.first().map(String::as_str) {
            Some("-") if fields.len() == 1 && !saw_protected => saw_protected = true,
            Some("a") if fields.len() == 3 => {
                if revisions.len() >= buzz_core::stickers::MAX_STICKER_CATALOG_PACKS {
                    return None;
                }
                let coordinate = fields.get(1)?;
                let approved_event_id = fields.get(2)?;
                let address = PackAddress::parse(coordinate).ok()?;
                if address.coordinate() != *coordinate
                    || approved_event_id.len() != 64
                    || !approved_event_id
                        .chars()
                        .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
                    || revisions
                        .insert(coordinate.clone(), approved_event_id.clone())
                        .is_some()
                {
                    return None;
                }
            }
            _ => return None,
        }
    }
    saw_protected.then_some(revisions)
}

async fn cmd_list(client: &BuzzClient) -> Result<(), CliError> {
    let installed = fetch_own_installed(client).await?;
    let approved_revisions = fetch_approved_revisions(client).await?;
    let installed_coordinates: HashSet<String> = installed
        .packs
        .iter()
        .map(PackAddress::coordinate)
        .collect();
    let filter = serde_json::json!({
        "kinds": [STICKER_PACK_KIND],
    });
    let raw = client.query(&filter).await?;
    let events = parse_events(&raw, "sticker pack list")?;

    let mut invalid_events = 0usize;
    let mut packs = Vec::new();
    for event in events {
        let pack = match parse_pack_event(&event) {
            Ok(pack) => pack,
            Err(_) => {
                invalid_events += 1;
                continue;
            }
        };
        let address = pack.address.coordinate();
        let installed = installed_coordinates.contains(&address);
        let available = approved_revisions
            .get(&address)
            .is_some_and(|approved_id| approved_id == &event.id.to_hex());
        if !available && !installed {
            continue;
        }
        packs.push(PackSummary {
            address: address.clone(),
            title: pack.title,
            description: pack.description,
            cover_url: pack.cover.map(|cover| cover.url),
            sticker_count: pack.stickers.len(),
            available,
            installed,
            event_id: event.id.to_hex(),
        });
    }
    packs.sort_by(|a, b| a.title.cmp(&b.title).then(a.address.cmp(&b.address)));
    let available_count = packs.iter().filter(|pack| pack.available).count();
    println!(
        "{}",
        serde_json::json!({
            "packs": packs,
            "available_count": available_count,
            "installed_count": installed_coordinates.len(),
            "invalid_events": invalid_events,
        })
    );
    Ok(())
}

async fn cmd_show(client: &BuzzClient, address: &str) -> Result<(), CliError> {
    let address = PackAddress::parse(address).map_err(|e| sticker_error("invalid address", e))?;
    let event = fetch_pack_event(client, &address)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("sticker pack not found: {address}")))?;
    let pack = parse_pack_event(&event)
        .map_err(|e| CliError::Other(format!("invalid sticker pack event: {e}")))?;
    let installed = fetch_own_installed(client).await?.packs.contains(&address);
    let available = fetch_approved_revisions(client)
        .await?
        .get(&address.coordinate())
        .is_some_and(|approved_id| approved_id == &event.id.to_hex());
    println!(
        "{}",
        serde_json::json!({
            "pack": pack,
            "available": available,
            "installed": installed,
            "event_id": event.id.to_hex(),
            "created_at": event.created_at.as_secs(),
        })
    );
    Ok(())
}

async fn publish_installed(client: &BuzzClient, list: InstalledPackList) -> Result<(), CliError> {
    let builder = EventBuilder::new(Kind::Custom(USER_STICKER_PACKS_KIND), "")
        .tags(build_installed_packs_tags(&list));
    let event = client.sign_event(builder)?;
    let event_id = event.id;
    let response = client.submit_event(event).await?;
    let current = fetch_own_installed_event(client)
        .await?
        .ok_or_else(|| CliError::Conflict("installed sticker list has no current head".into()))?;
    if current.id != event_id {
        return Err(CliError::Conflict(format!(
            "installed sticker list write {} was superseded by {}",
            event_id.to_hex(),
            current.id.to_hex()
        )));
    }
    println!("{}", normalize_write_response(&response));
    Ok(())
}

async fn cmd_install(client: &BuzzClient, address: &str) -> Result<(), CliError> {
    let address = PackAddress::parse(address).map_err(|e| sticker_error("invalid address", e))?;
    let pack_event = fetch_pack_event(client, &address).await?.ok_or_else(|| {
        CliError::Usage(format!("cannot install missing sticker pack: {address}"))
    })?;
    if fetch_approved_revisions(client)
        .await?
        .get(&address.coordinate())
        .is_none_or(|approved_id| approved_id != &pack_event.id.to_hex())
    {
        return Err(CliError::Usage(format!(
            "sticker pack is not available in this workspace catalog: {address}"
        )));
    }
    let mut installed = fetch_own_installed(client).await?;
    if installed.packs.contains(&address) {
        println!(
            "{}",
            serde_json::json!({"accepted": true, "message": "already installed"})
        );
        return Ok(());
    }
    installed.packs.push(address);
    publish_installed(client, InstalledPackList::new(installed.packs)).await
}

async fn cmd_uninstall(client: &BuzzClient, address: &str) -> Result<(), CliError> {
    let address = PackAddress::parse(address).map_err(|e| sticker_error("invalid address", e))?;
    let mut installed = fetch_own_installed(client).await?;
    let before = installed.packs.len();
    installed.packs.retain(|pack| pack != &address);
    if installed.packs.len() == before {
        println!(
            "{}",
            serde_json::json!({"accepted": true, "message": "not installed"})
        );
        return Ok(());
    }
    publish_installed(client, InstalledPackList::new(installed.packs)).await
}

fn require_https_upload_target(client: &BuzzClient) -> Result<(), CliError> {
    let relay = url::Url::parse(client.relay_url())
        .map_err(|e| CliError::Usage(format!("invalid relay URL: {e}")))?;
    if relay.scheme() != "https" {
        return Err(CliError::Usage(
            "Sonar sticker assets require HTTPS URLs; use an HTTPS Buzz relay".into(),
        ));
    }
    Ok(())
}

fn sniff_sticker_asset(bytes: Vec<u8>, source: &str) -> Result<PreparedAsset, CliError> {
    if bytes.len() > MAX_STICKER_BYTES {
        return Err(CliError::Usage(format!(
            "{source}: sticker is {} bytes (max {MAX_STICKER_BYTES})",
            bytes.len()
        )));
    }
    let mime = infer::get(&bytes)
        .map(|kind| {
            if kind.mime_type() == "image/png"
                && buzz_core::stickers::apng_frame_count(&bytes).is_some()
            {
                "image/apng".to_owned()
            } else {
                kind.mime_type().to_ascii_lowercase()
            }
        })
        .ok_or_else(|| CliError::Usage(format!("{source}: unrecognized image bytes")))?;
    if !is_allowed_sticker_mime(&mime) {
        return Err(CliError::Usage(format!(
            "{source}: unsupported sticker type {mime}; expected WebP, PNG/APNG, or GIF"
        )));
    }
    let dimensions = imagesize::blob_size(&bytes)
        .map_err(|_| CliError::Usage(format!("{source}: cannot read image dimensions")))?;
    if dimensions.width == 0
        || dimensions.height == 0
        || dimensions.width > 4096
        || dimensions.height > 4096
    {
        return Err(CliError::Usage(format!(
            "{source}: sticker dimensions must be between 1x1 and 4096x4096"
        )));
    }
    let width = u32::try_from(dimensions.width)
        .map_err(|_| CliError::Usage(format!("{source}: sticker width is too large")))?;
    let height = u32::try_from(dimensions.height)
        .map_err(|_| CliError::Usage(format!("{source}: sticker height is too large")))?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    Ok(PreparedAsset {
        bytes,
        mime,
        sha256,
        width,
        height,
    })
}

fn prepare_file(path: &Path) -> Result<PreparedAsset, CliError> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| CliError::Other(format!("cannot access {}: {e}", path.display())))?;
    if !metadata.is_file() {
        return Err(CliError::Usage(format!("{} is not a file", path.display())));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| CliError::Other(format!("failed to read {}: {e}", path.display())))?;
    sniff_sticker_asset(bytes, &path.display().to_string())
}

fn resolve_asset_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}

fn parse_dim(dim: Option<&str>) -> Result<(Option<u32>, Option<u32>), CliError> {
    let Some(dim) = dim else {
        return Ok((None, None));
    };
    let Some((width, height)) = dim.split_once('x') else {
        return Err(CliError::Other(format!("invalid upload dimensions: {dim}")));
    };
    let width = width
        .parse()
        .map_err(|_| CliError::Other(format!("invalid upload dimensions: {dim}")))?;
    let height = height
        .parse()
        .map_err(|_| CliError::Other(format!("invalid upload dimensions: {dim}")))?;
    Ok((Some(width), Some(height)))
}

async fn upload_assets(
    client: &BuzzClient,
    assets: impl IntoIterator<Item = PreparedAsset>,
) -> Result<HashMap<String, BlobDescriptor>, CliError> {
    let mut uploaded = HashMap::new();
    for asset in assets {
        if uploaded.contains_key(&asset.sha256) {
            continue;
        }
        let descriptor = client.upload_bytes(asset.bytes, &asset.mime).await?;
        if descriptor.sha256 != asset.sha256 {
            return Err(CliError::Other(
                "relay returned a different hash for uploaded sticker".into(),
            ));
        }
        if descriptor.mime_type != asset.mime {
            return Err(CliError::Other(format!(
                "relay returned MIME {} for uploaded {}",
                descriptor.mime_type, asset.mime
            )));
        }
        uploaded.insert(asset.sha256, descriptor);
    }
    Ok(uploaded)
}

fn sticker_from_descriptor(
    shortcode: &str,
    asset: &PreparedAsset,
    descriptor: &BlobDescriptor,
    alt: Option<String>,
    emoji: Option<String>,
) -> Result<Sticker, CliError> {
    if let Some(dim) = descriptor.dim.as_deref() {
        let (width, height) = parse_dim(Some(dim))?;
        if width != Some(asset.width) || height != Some(asset.height) {
            return Err(CliError::Other(format!(
                "relay returned dimensions {dim} for a {}x{} sticker",
                asset.width, asset.height
            )));
        }
    }
    Sticker::new(
        shortcode,
        descriptor.url.clone(),
        asset.sha256.clone(),
        asset.mime.clone(),
        Some(asset.width),
        Some(asset.height),
        alt,
        emoji,
    )
    .map_err(|e| sticker_error("invalid uploaded sticker", e))
}

async fn publish_pack(client: &BuzzClient, pack: StickerPack) -> Result<(), CliError> {
    let address = pack.address.clone();
    let builder =
        EventBuilder::new(Kind::Custom(STICKER_PACK_KIND), "").tags(build_pack_tags(&pack));
    let event = client.sign_event(builder)?;
    let event_id = event.id;
    let response = client.submit_event(event).await?;
    let current = fetch_pack_event(client, &address)
        .await?
        .ok_or_else(|| CliError::Conflict(format!("sticker pack {address} has no current head")))?;
    if current.id != event_id {
        return Err(CliError::Conflict(format!(
            "sticker pack write {} was superseded by {}",
            event_id.to_hex(),
            current.id.to_hex()
        )));
    }
    println!("{}", normalize_write_response(&response));
    Ok(())
}

fn load_manifest(path: &Path) -> Result<PackManifest, CliError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| CliError::Other(format!("failed to read {}: {e}", path.display())))?;
    serde_json::from_str(&raw)
        .map_err(|e| CliError::Usage(format!("invalid sticker manifest {}: {e}", path.display())))
}

fn validate_pack_text(
    title: &str,
    description: Option<&str>,
    cover_alt: Option<&str>,
    license: Option<&str>,
    stickers: &[StickerManifest],
) -> Result<(), CliError> {
    if title.trim().is_empty() || title.chars().count() > 80 {
        return Err(CliError::Usage(
            "sticker pack title must contain 1..80 characters".into(),
        ));
    }
    if description.is_some_and(|value| value.chars().count() > 500) {
        return Err(CliError::Usage(
            "sticker pack description must be at most 500 characters".into(),
        ));
    }
    if cover_alt.is_some_and(|value| value.chars().count() > 160) {
        return Err(CliError::Usage(
            "sticker cover alt text must be at most 160 characters".into(),
        ));
    }
    if license.is_some_and(|value| value.chars().count() > 160) {
        return Err(CliError::Usage(
            "sticker pack license must be at most 160 characters".into(),
        ));
    }
    if stickers.is_empty() || stickers.len() > 200 {
        return Err(CliError::Usage(
            "sticker pack must contain 1..200 stickers".into(),
        ));
    }
    for sticker in stickers {
        if sticker
            .alt
            .as_ref()
            .is_some_and(|value| value.chars().count() > 160)
        {
            return Err(CliError::Usage(format!(
                "sticker {} alt text must be at most 160 characters",
                sticker.shortcode
            )));
        }
        if sticker
            .emoji
            .as_ref()
            .is_some_and(|value| value.chars().count() > 8)
        {
            return Err(CliError::Usage(format!(
                "sticker {} emoji must be at most 8 characters",
                sticker.shortcode
            )));
        }
    }
    Ok(())
}

async fn cmd_manifest(
    client: &BuzzClient,
    file: &str,
    require_existing: bool,
) -> Result<(), CliError> {
    require_https_upload_target(client)?;
    let manifest_path = Path::new(file);
    let manifest = load_manifest(manifest_path)?;
    validate_pack_text(
        &manifest.title,
        manifest.description.as_deref(),
        manifest.cover_alt.as_deref(),
        manifest.license.as_deref(),
        &manifest.stickers,
    )?;
    let base = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let address = PackAddress::new(
        client.keys().public_key().to_hex(),
        manifest.identifier.clone(),
    )
    .map_err(|e| sticker_error("invalid pack identifier", e))?;
    let existing = fetch_pack_event(client, &address).await?.is_some();
    if require_existing && !existing {
        return Err(CliError::Usage(format!(
            "cannot update missing sticker pack: {address}"
        )));
    }
    if !require_existing && existing {
        return Err(CliError::Usage(format!(
            "sticker pack already exists; use `buzz stickers update --file {file}`"
        )));
    }

    let mut seen_shortcodes = HashSet::new();
    let mut seen_hashes = HashSet::new();
    let mut prepared_stickers = Vec::with_capacity(manifest.stickers.len());
    for entry in &manifest.stickers {
        validate_shortcode(&entry.shortcode)
            .map_err(|e| sticker_error("invalid sticker shortcode", e))?;
        if !seen_shortcodes.insert(entry.shortcode.clone()) {
            return Err(CliError::Usage(format!(
                "duplicate sticker shortcode: {}",
                entry.shortcode
            )));
        }
        let asset = prepare_file(&resolve_asset_path(base, &entry.file))?;
        if !seen_hashes.insert(asset.sha256.clone()) {
            return Err(CliError::Usage(format!(
                "duplicate sticker content hash: {}",
                asset.sha256
            )));
        }
        prepared_stickers.push(asset);
    }
    let prepared_cover = manifest
        .cover_file
        .as_deref()
        .map(|path| prepare_file(&resolve_asset_path(base, path)))
        .transpose()?;
    if prepared_cover
        .as_ref()
        .is_some_and(|cover| cover.mime != "image/webp")
    {
        return Err(CliError::Usage(
            "sticker pack cover must be WebP because the Sonar image tag has no MIME field".into(),
        ));
    }

    let all_assets = prepared_stickers
        .iter()
        .cloned()
        .chain(prepared_cover.iter().cloned());
    let uploaded = upload_assets(client, all_assets).await?;
    let stickers = manifest
        .stickers
        .iter()
        .zip(&prepared_stickers)
        .map(|(entry, asset)| {
            let descriptor = uploaded
                .get(&asset.sha256)
                .ok_or_else(|| CliError::Other("uploaded sticker descriptor missing".into()))?;
            sticker_from_descriptor(
                &entry.shortcode,
                asset,
                descriptor,
                entry.alt.clone(),
                entry.emoji.clone(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let cover = prepared_cover
        .as_ref()
        .map(|asset| {
            let descriptor = uploaded
                .get(&asset.sha256)
                .ok_or_else(|| CliError::Other("uploaded cover descriptor missing".into()))?;
            sticker_from_descriptor("cover", asset, descriptor, manifest.cover_alt.clone(), None)
        })
        .transpose()?;
    let pack = StickerPack::new(
        address,
        manifest.title,
        manifest.description,
        cover,
        stickers,
        manifest.license,
    )
    .map_err(|e| sticker_error("invalid sticker pack", e))?;
    publish_pack(client, pack).await
}

fn sniff_signal_sticker(asset: &ImportedSignalSticker) -> Result<PreparedAsset, CliError> {
    let prepared = sniff_sticker_asset(asset.bytes.clone(), "Signal sticker asset")?;
    if prepared.sha256 != asset.sha256 {
        return Err(CliError::Other(
            "Signal sticker hash changed after authenticated import".into(),
        ));
    }
    Ok(prepared)
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn short_emoji(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_chars(value, 8))
}

async fn cmd_import(
    client: &BuzzClient,
    signal_link: &str,
    identifier: Option<String>,
    title: Option<String>,
    skip_missing_signal_stickers: bool,
) -> Result<(), CliError> {
    require_https_upload_target(client)?;
    validate_signal_link(signal_link)?;
    let imported = import_signal_pack_with_options(
        signal_link,
        SignalImportOptions {
            accept_invalid_certs: false,
            skip_failed_stickers: skip_missing_signal_stickers,
        },
    )
    .await
    .map_err(|e| CliError::Other(format!("Signal sticker import failed: {e}")))?;

    let pack_title = title.unwrap_or_else(|| truncate_chars(&imported.title, 80));
    if pack_title.trim().is_empty() || pack_title.chars().count() > 80 {
        return Err(CliError::Usage(
            "sticker pack title must contain 1..80 characters".into(),
        ));
    }

    let identifier = identifier.unwrap_or_else(|| format!("signal-{}", imported.pack_id));
    let address = PackAddress::new(client.keys().public_key().to_hex(), identifier)
        .map_err(|e| sticker_error("invalid pack identifier", e))?;
    if fetch_pack_event(client, &address).await?.is_some() {
        return Err(CliError::Usage(format!(
            "sticker pack already exists: {address}; choose --identifier or use a manifest update"
        )));
    }

    let mut seen_shortcodes = HashSet::new();
    let mut seen_hashes = HashSet::new();
    let mut prepared_stickers = Vec::with_capacity(imported.stickers.len());
    for sticker in &imported.stickers {
        validate_shortcode(&sticker.shortcode)
            .map_err(|e| sticker_error("invalid imported shortcode", e))?;
        if !seen_shortcodes.insert(sticker.shortcode.clone()) {
            return Err(CliError::Usage(format!(
                "duplicate imported shortcode: {}",
                sticker.shortcode
            )));
        }
        let asset = sniff_signal_sticker(sticker)?;
        if !seen_hashes.insert(asset.sha256.clone()) {
            return Err(CliError::Usage(format!(
                "duplicate imported sticker hash: {}",
                asset.sha256
            )));
        }
        prepared_stickers.push(asset);
    }
    let prepared_cover = imported
        .cover
        .as_ref()
        .map(sniff_signal_sticker)
        .transpose()?
        .filter(|cover| cover.mime == "image/webp");
    let all_assets = prepared_stickers
        .iter()
        .cloned()
        .chain(prepared_cover.iter().cloned());
    let uploaded = upload_assets(client, all_assets).await?;

    let stickers = imported
        .stickers
        .iter()
        .zip(&prepared_stickers)
        .map(|(source, asset)| {
            let descriptor = uploaded
                .get(&asset.sha256)
                .ok_or_else(|| CliError::Other("uploaded sticker descriptor missing".into()))?;
            let alt = match source.emoji.as_deref() {
                Some(emoji) if !emoji.is_empty() => Some(truncate_chars(
                    &format!("Signal sticker {} {emoji}", source.id),
                    160,
                )),
                _ => Some(format!("Signal sticker {}", source.id)),
            };
            sticker_from_descriptor(
                &source.shortcode,
                asset,
                descriptor,
                alt,
                short_emoji(source.emoji.as_deref()),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let cover = prepared_cover
        .as_ref()
        .map(|asset| {
            let descriptor = uploaded
                .get(&asset.sha256)
                .ok_or_else(|| CliError::Other("uploaded cover descriptor missing".into()))?;
            sticker_from_descriptor(
                "cover",
                asset,
                descriptor,
                Some("Sticker pack cover".into()),
                None,
            )
        })
        .transpose()?
        .or_else(|| {
            stickers
                .iter()
                .find(|sticker| sticker.mime == "image/webp")
                .cloned()
        });
    let description = imported
        .author
        .as_deref()
        .map(str::trim)
        .filter(|author| !author.is_empty())
        .map_or_else(
            || Some("Imported from a Signal sticker pack.".to_owned()),
            |author| {
                Some(truncate_chars(
                    &format!("Imported from a Signal sticker pack by {author}."),
                    500,
                ))
            },
        );
    let pack = StickerPack::new(address, pack_title, description, cover, stickers, None)
        .map_err(|e| sticker_error("invalid imported sticker pack", e))?;
    publish_pack(client, pack).await
}

fn validate_signal_link(link: &str) -> Result<(), CliError> {
    let url =
        url::Url::parse(link).map_err(|_| CliError::Usage("invalid Signal sticker link".into()))?;
    if url.scheme() != "https"
        || url.host_str() != Some("signal.art")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path().trim_end_matches('/') != "/addstickers"
    {
        return Err(CliError::Usage(
            "expected an https://signal.art/addstickers/ link".into(),
        ));
    }
    SignalPackLink::parse(link)
        .map(|_| ())
        .map_err(|error| CliError::Usage(format!("invalid Signal sticker link: {error}")))
}

fn read_signal_link(path: &str) -> Result<Zeroizing<String>, CliError> {
    let mut link = String::new();
    if path == "-" {
        std::io::stdin()
            .take(8 * 1024)
            .read_to_string(&mut link)
            .map_err(|e| CliError::Other(format!("failed to read Signal link: {e}")))?;
    } else {
        std::fs::File::open(path)
            .map_err(|e| CliError::Other(format!("failed to open Signal link file: {e}")))?
            .take(8 * 1024)
            .read_to_string(&mut link)
            .map_err(|e| CliError::Other(format!("failed to read Signal link file: {e}")))?;
    }
    let link = Zeroizing::new(link.trim().to_owned());
    if link.is_empty() {
        return Err(CliError::Usage("Signal link input is empty".into()));
    }
    Ok(link)
}

pub async fn dispatch(cmd: crate::StickersCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::StickersCmd;
    match cmd {
        StickersCmd::List => cmd_list(client).await,
        StickersCmd::Show { address } => cmd_show(client, &address).await,
        StickersCmd::Install { address } => cmd_install(client, &address).await,
        StickersCmd::Uninstall { address } => cmd_uninstall(client, &address).await,
        StickersCmd::Import {
            signal_link_file,
            identifier,
            title,
            skip_missing_signal_stickers,
        } => {
            let signal_link = read_signal_link(&signal_link_file)?;
            cmd_import(
                client,
                signal_link.as_str(),
                identifier,
                title,
                skip_missing_signal_stickers,
            )
            .await
        }
        StickersCmd::Create { file } => cmd_manifest(client, &file, false).await,
        StickersCmd::Update { file } => cmd_manifest(client, &file, true).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_rejects_unknown_fields() {
        let raw = r#"{
            "identifier":"party",
            "title":"Party",
            "stickers":[{"shortcode":"wave","file":"wave.webp","wat":1}]
        }"#;
        assert!(serde_json::from_str::<PackManifest>(raw).is_err());
    }

    #[test]
    fn manifest_parses_relative_assets() {
        let raw = r#"{
            "identifier":"party-pack",
            "title":"Party",
            "description":"A small pack",
            "cover_file":"cover.png",
            "stickers":[
                {"shortcode":"wave","file":"wave.webp","emoji":"👋"}
            ],
            "license":"CC0"
        }"#;
        let manifest: PackManifest = serde_json::from_str(raw).expect("valid manifest");
        assert_eq!(manifest.identifier, "party-pack");
        assert_eq!(manifest.stickers[0].shortcode, "wave");
        assert_eq!(manifest.cover_file, Some(PathBuf::from("cover.png")));
    }

    #[test]
    fn resolve_asset_paths_relative_to_manifest() {
        assert_eq!(
            resolve_asset_path(Path::new("/tmp/pack"), Path::new("wave.webp")),
            PathBuf::from("/tmp/pack/wave.webp")
        );
        assert_eq!(
            resolve_asset_path(Path::new("/tmp/pack"), Path::new("/assets/wave.webp")),
            PathBuf::from("/assets/wave.webp")
        );
    }

    #[test]
    fn installed_list_preserves_order_and_deduplicates() {
        let a = PackAddress::new("a".repeat(64), "one").expect("address");
        let b = PackAddress::new("b".repeat(64), "two").expect("address");
        let list = InstalledPackList::new(vec![a.clone(), b.clone(), a]);
        assert_eq!(
            list.packs,
            vec![PackAddress::new("a".repeat(64), "one").expect("address"), b]
        );
    }

    #[test]
    fn sticker_sniffer_rejects_unknown_signal_bytes() {
        let error = sniff_sticker_asset(vec![1, 2, 3, 4], "Signal sticker asset")
            .err()
            .expect("unknown bytes rejected");
        assert!(error.to_string().contains("unrecognized image bytes"));
    }

    #[test]
    fn signal_link_requires_official_https_shape() {
        let valid = format!(
            "https://signal.art/addstickers/#pack_id={}&pack_key={}",
            "a".repeat(32),
            "b".repeat(64)
        );
        assert!(validate_signal_link(&valid).is_ok());
        assert!(validate_signal_link(&valid.replace("signal.art", "example.com")).is_err());
        assert!(validate_signal_link(&valid.replace("https://", "http://")).is_err());
        assert!(validate_signal_link(&valid.replace("addstickers", "other")).is_err());
    }

    #[test]
    fn canonical_head_uses_smallest_id_for_equal_timestamp() {
        let keys = nostr::Keys::generate();
        let timestamp = nostr::Timestamp::from(1234);
        let a = EventBuilder::new(Kind::TextNote, "a")
            .custom_created_at(timestamp)
            .sign_with_keys(&keys)
            .expect("event a");
        let b = EventBuilder::new(Kind::TextNote, "b")
            .custom_created_at(timestamp)
            .sign_with_keys(&keys)
            .expect("event b");
        let expected = std::cmp::min(a.id, b.id);
        assert_eq!(canonical_head(vec![a, b]).expect("head").id, expected);
    }

    #[test]
    fn catalog_revision_requires_event_id_field() {
        let keys = nostr::Keys::generate();
        let coordinate = format!("30031:{}:party", keys.public_key().to_hex());
        let approved_id = "a".repeat(64);
        let event = EventBuilder::new(Kind::Custom(13536), "")
            .tags([
                nostr::Tag::parse(["a", &coordinate, &approved_id]).expect("revision tag"),
                nostr::Tag::parse(["a", &format!("30031:{}:old", "b".repeat(64))])
                    .expect("legacy tag"),
            ])
            .sign_with_keys(&keys)
            .expect("catalog");
        assert!(approved_revisions_from_event(&event).is_none());

        let valid = EventBuilder::new(Kind::Custom(13536), "")
            .tags([
                nostr::Tag::parse(["-"]).expect("protected tag"),
                nostr::Tag::parse(["a", &coordinate, &approved_id]).expect("revision tag"),
            ])
            .sign_with_keys(&keys)
            .expect("catalog");
        let revisions = approved_revisions_from_event(&valid).expect("valid catalog");
        assert_eq!(revisions.get(&coordinate), Some(&approved_id));
        assert_eq!(revisions.len(), 1);
    }
}
