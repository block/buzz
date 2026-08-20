//! Card archive: on-disk persistence for minted trading cards.
//!
//! Each mint writes two plain files to the cards dir — `<stem>.agent.png`
//! and a `<stem>.json` sidecar — plus a best-effort `<stem>.thumb.jpg` for
//! gallery grids. There is no shared index to corrupt: listing scans
//! sidecars, and a card whose PNG is missing is skipped rather than failing
//! the whole list.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::MintedCard;
use crate::managed_agents::agent_snapshot::MemoryLevel;

/// Sidecar metadata for one archived card PNG. Stored as `<stem>.json` next
/// to `<stem>.agent.png` in the cards dir — two plain files per mint, no
/// shared index to corrupt. Listing scans sidecars; a card whose PNG is
/// missing is skipped rather than failing the whole list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedCardMeta {
    /// Unique on-disk PNG file name within the cards dir.
    pub stored_file_name: String,
    /// Suggested save-as name, e.g. `eva.agent.png`.
    pub file_name: String,
    /// The id the card was minted for (instance pubkey or definition slug).
    pub agent_id: String,
    pub agent_name: String,
    pub designer_notes: String,
    pub locked: bool,
    /// Memory embedded in this card's snapshot. Defaults to `None` when the
    /// sidecar predates the field — every pre-field mint was minted with
    /// `MemoryLevel::None` (it was structural), so the default is honest.
    #[serde(default)]
    pub memory_level: MemoryLevel,
    /// ISO-8601 mint timestamp.
    pub minted_at: String,
    /// Small JPEG preview for gallery grids, base64. Populated by
    /// `list_agent_cards` from the sidecar thumb file — never stored in the
    /// JSON sidecar itself.
    #[serde(default, skip_deserializing)]
    pub thumb_jpeg_base64: Option<String>,
}

fn cards_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = crate::managed_agents::managed_agents_base_dir(app)?.join("cards");
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create cards dir: {e}"))?;
    Ok(dir)
}

/// Persist a freshly minted card to the archive. Failures are surfaced to the
/// caller (which logs and continues) — an archive write must never fail a
/// mint the user already paid for.
pub(super) fn archive_minted_card(
    app: &AppHandle,
    agent_id: &str,
    agent_name: &str,
    card: &MintedCard,
    bytes: &[u8],
) -> Result<ArchivedCardMeta, String> {
    let dir = cards_dir(app)?;
    let stem = format!(
        "{}-{}",
        crate::util::slugify(agent_name, "agent", 50),
        uuid::Uuid::new_v4()
    );
    let stored_file_name = format!("{stem}.agent.png");
    let meta = ArchivedCardMeta {
        stored_file_name: stored_file_name.clone(),
        file_name: card.file_name.clone(),
        agent_id: agent_id.to_string(),
        agent_name: agent_name.to_string(),
        designer_notes: card.designer_notes.clone(),
        locked: card.locked,
        memory_level: card.memory_level,
        minted_at: crate::util::now_iso(),
        thumb_jpeg_base64: None,
    };
    // PNG first, sidecar second: a crash between the two leaves an orphaned
    // PNG (invisible to the list), never a sidecar pointing at nothing.
    std::fs::write(dir.join(&stored_file_name), bytes)
        .map_err(|e| format!("failed to write archived card: {e}"))?;
    let meta_json = serde_json::to_string_pretty(&meta)
        .map_err(|e| format!("failed to serialize card metadata: {e}"))?;
    std::fs::write(dir.join(format!("{stem}.json")), meta_json)
        .map_err(|e| format!("failed to write card metadata: {e}"))?;
    // Thumb last and best-effort: the gallery grid falls back to lazy
    // full-card loading for a card whose thumb is missing.
    if let Ok(thumb) = encode_card_thumb(bytes) {
        let _ = std::fs::write(dir.join(format!("{stem}.thumb.jpg")), thumb);
    }
    Ok(meta)
}

/// Downscale card PNG bytes to a small JPEG for gallery grids. The full card
/// is ~1500x2250 PNG (megabytes); shipping that per card over IPC just to
/// draw a grid tile is waste.
fn encode_card_thumb(bytes: &[u8]) -> Result<Vec<u8>, String> {
    const THUMB_WIDTH: u32 = 300;
    let img = image::load_from_memory(bytes).map_err(|e| format!("thumb decode: {e}"))?;
    let scale = THUMB_WIDTH as f64 / img.width() as f64;
    let thumb = img.resize(
        THUMB_WIDTH,
        (img.height() as f64 * scale).round().max(1.0) as u32,
        image::imageops::FilterType::Triangle,
    );
    let mut out = Vec::new();
    // JPEG has no alpha; cards are opaque, so flatten unconditionally.
    let rgb = image::DynamicImage::ImageRgb8(thumb.to_rgb8());
    rgb.write_to(
        &mut std::io::Cursor::new(&mut out),
        image::ImageFormat::Jpeg,
    )
    .map_err(|e| format!("thumb encode: {e}"))?;
    Ok(out)
}

/// Reject any archive file name that could escape the cards dir or name a
/// non-archive file. Archive names are generated by `archive_minted_card`
/// (slug + UUID), so a strict shape check loses nothing legitimate.
fn validate_archive_file_name(stored_file_name: &str) -> Result<(), String> {
    let valid = stored_file_name.ends_with(".agent.png")
        && !stored_file_name.contains(['/', '\\'])
        && !stored_file_name.contains("..");
    if !valid {
        return Err("Invalid archived card file name.".to_string());
    }
    Ok(())
}

/// List all archived cards, newest first.
#[tauri::command]
pub fn list_agent_cards(app: AppHandle) -> Result<Vec<ArchivedCardMeta>, String> {
    let dir = cards_dir(&app)?;
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("failed to read cards dir: {e}"))?;
    let mut cards = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(meta) = serde_json::from_str::<ArchivedCardMeta>(&content) else {
            // A malformed sidecar hides one card, never the archive.
            eprintln!(
                "buzz-desktop: card-archive: skipping malformed sidecar {}",
                path.display()
            );
            continue;
        };
        let mut meta = meta;
        if validate_archive_file_name(&meta.stored_file_name).is_ok()
            && dir.join(&meta.stored_file_name).is_file()
        {
            // Attach the pre-rendered grid thumb when present (best-effort).
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                meta.thumb_jpeg_base64 = std::fs::read(dir.join(format!("{stem}.thumb.jpg")))
                    .ok()
                    .map(|b| STANDARD.encode(&b));
            }
            cards.push(meta);
        }
    }
    // ISO-8601 sorts lexicographically; newest first.
    cards.sort_by(|a, b| b.minted_at.cmp(&a.minted_at));
    Ok(cards)
}

/// Load one archived card's PNG bytes as base64, keyed by its stored file
/// name (as returned by `list_agent_cards`).
#[tauri::command]
pub fn load_agent_card(stored_file_name: String, app: AppHandle) -> Result<String, String> {
    validate_archive_file_name(&stored_file_name)?;
    let bytes = std::fs::read(cards_dir(&app)?.join(&stored_file_name))
        .map_err(|e| format!("failed to read archived card: {e}"))?;
    Ok(STANDARD.encode(&bytes))
}

#[cfg(test)]
mod tests {
    use super::validate_archive_file_name;
    use super::{ArchivedCardMeta, MemoryLevel};

    #[test]
    fn archive_file_name_validation_rejects_escapes() {
        assert!(validate_archive_file_name("eva-1234.agent.png").is_ok());
        for bad in [
            "../escape.agent.png",
            "sub/dir.agent.png",
            "sub\\dir.agent.png",
            "not-a-card.png",
            "plain.json",
            "",
        ] {
            assert!(
                validate_archive_file_name(bad).is_err(),
                "expected rejection: {bad:?}"
            );
        }
    }

    #[test]
    fn archived_sidecar_without_memory_level_defaults_to_none() {
        // Every mint before the memory option existed embedded
        // MemoryLevel::None structurally, so old sidecars (no memoryLevel
        // field) must deserialize to None — the gallery's disclosure depends
        // on this being honest.
        let legacy = r#"{
        "storedFileName": "eva-1234.agent.png",
        "fileName": "eva.agent.png",
        "agentId": "abc",
        "agentName": "Eva",
        "designerNotes": "",
        "locked": false,
        "mintedAt": "2026-07-28T00:00:00Z"
    }"#;
        let meta: ArchivedCardMeta = serde_json::from_str(legacy).unwrap();
        assert_eq!(meta.memory_level, MemoryLevel::None);

        let with_level = legacy.replace(
            "\"locked\": false,",
            "\"locked\": false, \"memoryLevel\": \"everything\",",
        );
        let meta: ArchivedCardMeta = serde_json::from_str(&with_level).unwrap();
        assert_eq!(meta.memory_level, MemoryLevel::Everything);
    }
}
