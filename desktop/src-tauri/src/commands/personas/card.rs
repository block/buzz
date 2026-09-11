//! `mint_agent_card` / `save_agent_card` Tauri commands — Agent Trading Cards.
//!
//! Mints a collectible trading-card PNG for an agent via one OpenAI Responses
//! API call (designer model + native `image_generation` tool), then embeds the
//! agent's `buzz_agent_snapshot` manifest through the existing snapshot
//! encoder so the card IS an importable `.agent.png`.
//!
//! Boundary rules (agreed with Wren, buzz-agent-trading-cards thread):
//! - Snapshot construction/injection reuses `agent_snapshot.rs` — cards
//!   inherit manifest-v1 behavior, exclusions, and size checks. No card-only
//!   wire format exists.
//! - Memory inclusion is opt-in and shares the export flow's semantics: the
//!   same three levels (`none`/`core`/`everything`), the same owner-gated
//!   `get_agent_memory` fetch, and a memory source DERIVED from the resolved
//!   instance (never caller-supplied), so cross-agent memory pairing is
//!   structurally impossible. The default is `none`; the encoder still
//!   rejects `none` + entries.
//! - The 10 MiB `.agent.png` ceiling is enforced on the FINAL bytes (after
//!   resize + chunk injection) via `validate_snapshot_encode_size`.
//! - Round-trip verification decodes the final bytes and compares the logical
//!   manifest before anything is returned to the frontend.
//! - The API key is resolved through the same env layering the agent runtime
//!   uses (global config < persona < agent record) and never leaves Rust.
//!   It is never logged.

use super::super::export_util::save_bytes_with_dialog;
use super::snapshot::{
    materialize_snapshot_description, memory_entries_from_listing, parse_memory_level,
    resolve_from_lists, validate_snapshot_encode_size,
};
use crate::{
    app_state::AppState,
    commands::engrams::get_agent_memory,
    managed_agents::{
        agent_snapshot::{
            build_snapshot, decode_avatar_data_url, decode_snapshot_png, encode_snapshot_png,
            extract_chunk_payload_png, MemoryLevel,
        },
        agent_snapshot_envelope::{
            decrypt_envelope, encode_locked_snapshot_png, parse_chunk_payload, ChunkPayload,
        },
        load_agent_definitions, load_global_agent_config, load_managed_agents, load_personas,
        save_global_agent_config, validate_global_config,
    },
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

/// The Buzz card frame template — Tyler's gold-honeycomb base. Generation
/// input only: it never participates in the snapshot manifest, PNG chunk,
/// import decoder, or attachment validation. Embedded at compile time for
/// deterministic packaging (see `card_template_decodes` test).
const CARD_TEMPLATE_PNG: &[u8] = include_bytes!("../../../assets/card_template.png");

/// Designer model driving copy + art direction.
const DESIGNER_MODEL: &str = "gpt-5.6-sol";
/// Image model invoked natively via the Responses `image_generation` tool.
const IMAGE_MODEL: &str = "gpt-image-2";
/// Final card width in pixels (2:3 portrait → 1500x2250).
const CARD_WIDTH: u32 = 1500;
/// Longest edge for the real avatar inlined into an unlocked card's manifest.
/// Kind:0 pictures render small; 512px keeps the doubly-base64-encoded
/// manifest chunk modest next to the 1500-wide card body.
const MANIFEST_AVATAR_MAX_DIM: u32 = 512;
/// Upper bound for a fetched avatar (pre-resize input to the model).
const MAX_AVATAR_FETCH_BYTES: usize = 10 * 1024 * 1024;
/// One mint is a single long API call (~2–3 minutes observed).
const MINT_TIMEOUT_SECS: u64 = 600;

/// Error prefix the frontend matches to route the user to provider settings
/// instead of showing a raw failure.
pub(crate) const NO_KEY_ERROR_PREFIX: &str = "NO_OPENAI_KEY:";

/// Wire shape returned by `mint_agent_card`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MintedCard {
    /// Final `.agent.png` bytes (chunk-injected, round-trip verified),
    /// base64-encoded for the IPC boundary.
    pub card_png_base64: String,
    /// Suggested filename, e.g. `eva.agent.png`.
    pub file_name: String,
    /// Designer commentary emitted alongside the image (may be empty).
    pub designer_notes: String,
    /// True when the embedded snapshot is NIP-44-encrypted to the
    /// (owner, agent) pair — only their nsecs can import this card.
    pub locked: bool,
    /// How much memory is embedded in the card's snapshot ("none"/"core"/
    /// "everything"). The viewer's import disclosure depends on this.
    pub memory_level: MemoryLevel,
}

// ── Card archive ──────────────────────────────────────────────────────────────

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
fn archive_minted_card(
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

// ── Key resolution ────────────────────────────────────────────────────────────

/// Pure layering: global env < persona env < agent record env, then the
/// process environment as a development fallback. Returns the first
/// non-empty value for `key`.
pub(crate) fn resolve_env_from_layers(
    key: &str,
    global_env: &std::collections::BTreeMap<String, String>,
    persona_env: &std::collections::BTreeMap<String, String>,
    record_env: &std::collections::BTreeMap<String, String>,
    process_value: Option<String>,
) -> Option<String> {
    for layer in [record_env, persona_env, global_env] {
        if let Some(v) = layer.get(key) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    process_value.filter(|k| !k.trim().is_empty())
}

/// Pure classification: same four env inputs as `resolve_env_from_layers`,
/// returns which layer supplies `OPENAI_API_KEY` (agent > persona > global >
/// process > none).
pub(crate) fn resolve_key_layer(
    global_env: &std::collections::BTreeMap<String, String>,
    persona_env: &std::collections::BTreeMap<String, String>,
    record_env: &std::collections::BTreeMap<String, String>,
    process_value: Option<String>,
) -> &'static str {
    let key = "OPENAI_API_KEY";
    let nonempty = |m: &std::collections::BTreeMap<String, String>| {
        m.get(key).is_some_and(|v| !v.trim().is_empty())
    };
    if nonempty(record_env) {
        return "agent";
    }
    if nonempty(persona_env) {
        return "persona";
    }
    if nonempty(global_env) {
        return "global";
    }
    let proc = process_value.as_deref().unwrap_or("");
    if !proc.trim().is_empty() {
        return "process";
    }
    "none"
}

/// The Responses endpoint to post mints to. `OPENAI_BASE_URL` (same env
/// layering as the key) overrides the default host, supporting endpoints and
/// proxies that speak the OpenAI Responses shape with Bearer auth. Azure
/// OpenAI is NOT covered by this override alone — it uses its own URL scheme
/// and `api-key` auth header, which would need a real driver.
pub(crate) fn responses_url(base_url: Option<String>) -> String {
    let base = base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    format!("{}/responses", base.trim_end_matches('/'))
}

// ── Prompt construction ───────────────────────────────────────────────────────

/// Build the designer instructions. Pure so tests can pin the contract:
/// style-match-the-avatar is DEFAULT behavior; owner directions (art AND
/// card text) take primacy over those style defaults, but never over the
/// fixed contract (frame identity, geometry, text fidelity).
pub(crate) fn build_card_instructions(
    agent_name: &str,
    persona_notes: &str,
    style_notes: &str,
) -> String {
    let owner_directions = if style_notes.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\nOWNER'S DIRECTIONS — these override the default art-style and copy guidance \
             below wherever they conflict (they cannot change the frame, layout, or \
             text-fidelity requirements). The owner may direct the art, the card text \
             (type line, ability, flavor), or both:\n{style_notes}\n"
        )
    };
    format!(
        r#"You are designing one premium collectible trading card for the Buzz agent "{agent_name}".

Input image 1 is the official Buzz card frame template (gold honeycomb border, dark interior, name banner top, hex badge top-right, text box lower third). Input image 2 is the agent's avatar — study its exact art style: medium, pixel grid if any, palette, shading, background motifs.

Persona notes for the card copy:
{persona_notes}
{owner_directions}
First, write professional trading-card copy at Magic: The Gathering editorial quality:
- a type line (e.g. "Legendary Agent — Team Lead"),
- ONE keyworded ability: short bolded ability name + one sentence of crisp rules text written like real MTG rules (present tense, precise, no fluff),
- ONE italic flavor-text line, evocative and short, the kind that gets quoted.
Where the owner's directions specify card text, use their wording within the 220-character text-box limit below (edited only for spelling; if their text exceeds the limit, condense it minimally while keeping their words and intent); invent copy only for the parts they left open.
Keep total text-box copy under 220 characters so it renders cleanly.

Then generate the finished card with the image tool, exactly 1024x1536 portrait:
- The frame must follow input image 1 faithfully: same gold honeycomb border, same layout, honey drip detail.
- Default art style: match input image 2's art style EXACTLY — same medium, same pixel density if pixel art, same palette, same background honeycomb-lattice sky. It must look like the same artist drew a larger scene: the character in a confident pose, conjuring glowing golden hexagons. The owner's directions above override any of this default styling where they conflict.
- Name banner: "{agent_name}" plus the type line beneath it in smaller type.
- Text box: the ability name in bold, rules text in regular, then the flavor line in italics, cleanly typeset like a real MTG card — professional kerning, no misspellings, hyphenate nothing.
- Top-right hex badge: one small emblem of your choice, no text.
Render all text with perfect fidelity."#
    )
}

/// Encode raw image bytes as a `data:image/png;base64,` URL, downscaling to
/// `max_dim` on the longest edge so request payloads stay small.
fn image_data_url(bytes: &[u8], max_dim: u32) -> Result<String, String> {
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(png_bytes_resized(bytes, max_dim)?)
    ))
}

/// Re-encode an image as PNG, downscaling so neither side exceeds `max_dim`.
fn png_bytes_resized(bytes: &[u8], max_dim: u32) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("Failed to decode image: {e}"))?;
    let img = if img.width().max(img.height()) > max_dim {
        img.resize(max_dim, max_dim, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode image: {e}"))?;
    Ok(png)
}

// ── Response parsing ──────────────────────────────────────────────────────────

/// Extract the generated image (base64) and any designer text from a
/// Responses API payload. Pure for testability.
pub(crate) fn extract_card_output(resp: &serde_json::Value) -> Result<(String, String), String> {
    let output = resp
        .get("output")
        .and_then(|o| o.as_array())
        .ok_or_else(|| "Responses payload has no output array".to_string())?;

    let mut image_b64 = None;
    let mut notes = Vec::new();
    for item in output {
        match item.get("type").and_then(|t| t.as_str()) {
            Some("image_generation_call") => {
                if let Some(result) = item.get("result").and_then(|r| r.as_str()) {
                    image_b64 = Some(result.to_string());
                }
            }
            Some("message") => {
                if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                    for c in content {
                        if c.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                            if let Some(text) = c.get("text").and_then(|t| t.as_str()) {
                                notes.push(text.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let image_b64 = image_b64.ok_or_else(|| {
        let types: Vec<&str> = output
            .iter()
            .filter_map(|i| i.get("type").and_then(|t| t.as_str()))
            .collect();
        format!("No image in Responses output (item types: {types:?})")
    })?;
    Ok((image_b64, notes.join("\n")))
}

// ── Commands ──────────────────────────────────────────────────────────────────

/// Save an `OPENAI_API_KEY` into the global Agent Defaults env for card
/// minting — a narrow seam with deliberately different semantics from the
/// general `set_global_agent_config`:
///
/// - **No agent restarts.** The general command stops/restarts every running
///   local agent whose effective env changes, because agent env is baked at
///   spawn time. The mint command re-reads the config from disk on every
///   mint, so minting needs no restart — and a card setup must never disrupt
///   running agents as a side effect. Agents pick the key up naturally on
///   their next (re)start.
/// - **Read-modify-write of the latest on-disk config.** The config is
///   re-read immediately before the single-key insert + write (under the
///   managed-agents store lock, which serializes it against the other card
///   and agent-store commands), so a settings save that landed after this
///   dialog opened is not clobbered with a stale dialog-open snapshot.
///   (The general settings editor performs its own whole-config write; as
///   today, the last writer wins between the two surfaces.)
///
/// Standard global-config validation still applies (POSIX key shape,
/// reserved-key reject, size caps) — this is not a validation bypass.
#[tauri::command]
pub fn card_mint_save_openai_key(
    key: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("API key cannot be empty.".to_string());
    }

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;

    let mut config = load_global_agent_config(&app)?;
    config.env_vars.insert("OPENAI_API_KEY".to_string(), key);
    validate_global_config(&config)?;
    save_global_agent_config(&app, &config)
}

/// Report which env layer resolves the OpenAI key for a card mint of agent
/// `id` — same layering as `mint_agent_card`. Delegates to `resolve_key_layer`
/// for the classification; see that helper for the return-value contract.
#[tauri::command]
pub fn card_mint_key_status(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;

    let instances = load_managed_agents(&app)?;
    let definitions = load_agent_definitions(&app)?;
    let (record, _) = resolve_from_lists(&id, &instances, &definitions)?;

    let global = load_global_agent_config(&app).unwrap_or_default();
    let personas = load_personas(&app).unwrap_or_default();
    let persona_env = record
        .persona_id
        .as_deref()
        .and_then(|pid| personas.iter().find(|p| p.id == pid))
        .map(|p| p.env_vars.clone())
        .unwrap_or_default();

    Ok(resolve_key_layer(
        &global.env_vars,
        &persona_env,
        &record.env_vars,
        std::env::var("OPENAI_API_KEY").ok(),
    )
    .to_string())
}

/// Mint a trading card for the agent identified by `id` (instance pubkey,
/// instance slug, or definition slug — same resolution as snapshot export).
///
/// When `lock` is true the embedded manifest is NIP-44-encrypted to the
/// (owner, agent) pair per the locked-envelope contract — this requires a
/// linked agent instance (the second key endpoint); bare definitions cannot
/// be locked.
///
/// When `memory_level` is `"core"` or `"everything"`, the owner's decrypted
/// memory for the agent is embedded in the manifest — same levels and fetch
/// as snapshot export. The memory source is always the resolved instance
/// itself (derived, never caller-supplied), so it requires a linked instance;
/// bare definitions can only mint `"none"` (the default).
///
/// Returns the final, chunk-injected, round-trip-verified `.agent.png` bytes.
/// Reroll = call again; the command holds no session state.
#[tauri::command]
pub async fn mint_agent_card(
    id: String,
    style_notes: Option<String>,
    lock: Option<bool>,
    memory_level: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MintedCard, String> {
    let lock = lock.unwrap_or(false);
    let memory_level = parse_memory_level(memory_level.as_deref().unwrap_or(""))?;
    // ── Resolve the record + API key under lock ──────────────────────────────
    let (mut record, is_definition, api_key, base_url) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;

        let instances = load_managed_agents(&app)?;
        let definitions = load_agent_definitions(&app)?;
        let (record, is_definition) =
            resolve_from_lists(&id, &instances, &definitions).map(|(r, d)| (r.clone(), d))?;
        let mut record = record;
        materialize_snapshot_description(&mut record, is_definition, &definitions);
        let global = load_global_agent_config(&app).unwrap_or_default();
        let personas = load_personas(&app).unwrap_or_default();
        let persona_env = record
            .persona_id
            .as_deref()
            .and_then(|pid| personas.iter().find(|p| p.id == pid))
            .map(|p| p.env_vars.clone())
            .unwrap_or_default();

        let api_key = resolve_env_from_layers(
            "OPENAI_API_KEY",
            &global.env_vars,
            &persona_env,
            &record.env_vars,
            std::env::var("OPENAI_API_KEY").ok(),
        )
        .ok_or_else(|| {
            format!(
                "{NO_KEY_ERROR_PREFIX} No OPENAI_API_KEY found. Add one in the agent's \
                 environment variables or global agent settings to mint cards."
            )
        })?;
        let base_url = resolve_env_from_layers(
            "OPENAI_BASE_URL",
            &global.env_vars,
            &persona_env,
            &record.env_vars,
            std::env::var("OPENAI_BASE_URL").ok(),
        );

        (record, is_definition, api_key, base_url)
    };

    // ── Locking needs its two exact key endpoints up front, BEFORE the
    //    API spend: the owner identity secret and the agent instance pubkey.
    let lock_keys = if lock {
        if is_definition {
            return Err(
                "Locked cards need a linked agent instance — this persona has never been \
                 started, so there is no agent key to lock to."
                    .to_string(),
            );
        }
        let owner_keys = state.signing_keys()?;
        // Same canonical check the envelope decoder enforces (incl. curve
        // validation) — a non-point record pubkey must fail BEFORE the API
        // spend, not at post-mint encryption.
        let agent_pubkey = crate::managed_agents::agent_snapshot_envelope::parse_canonical_pubkey(
            "agentPubkey",
            &record.pubkey,
        )
        .map_err(|_| {
            "Agent record has an invalid pubkey (not a canonical x-only key).".to_string()
        })?;
        if owner_keys.public_key() == agent_pubkey {
            return Err("Cannot lock a card to itself: owner and agent keys match.".to_string());
        }
        Some((owner_keys, agent_pubkey))
    } else {
        None
    };

    // ── Memory needs a keyed instance, resolved up front BEFORE the API
    //    spend — the memory source is always the resolved instance itself
    //    (derived, never caller-supplied), so cross-agent pairing cannot be
    //    expressed. A failed fetch fails the mint here, not after payment.
    let memory_entries = if memory_level == MemoryLevel::None {
        Vec::new()
    } else {
        if is_definition {
            return Err(
                "Cards with memory need a linked agent instance — this persona has never \
                 been started, so there is no agent memory to include."
                    .to_string(),
            );
        }
        let listing = get_agent_memory(record.pubkey.clone(), app.clone(), state.clone()).await?;
        memory_entries_from_listing(listing, memory_level)
    };

    let display_name = record
        .display_name
        .clone()
        .unwrap_or_else(|| record.name.clone());

    // ── Prefer the agent's own kind:0 profile picture ────────────────────────
    // The record's `avatar_url` is a stale presentation snapshot: with
    // agent-managed profiles the agent updates its own kind:0 `picture` and
    // desktop reconciliation is disabled (`agent_settings.rs`), so the relay
    // profile — not the local record — is the live source of truth for how the
    // agent looks. Definitions have no keypair and thus no kind:0; they keep
    // the record's avatar. A relay error fails the mint here, BEFORE the API
    // spend (same fail-early rule as the key/memory guards above) — minting
    // with the wrong face wastes the spend it was supposed to protect.
    if !is_definition {
        let relay_url = crate::relay::effective_agent_relay_url(
            &record.relay_url,
            &crate::relay::relay_ws_url_with_override(&state),
        );
        let profile = crate::relay::query_agent_profile(&state, &relay_url, &record.pubkey)
            .await
            .map_err(|e| format!("Could not read the agent's profile for its avatar: {e}"))?;
        record.avatar_url = preferred_avatar_url(
            profile.and_then(|info| info.picture),
            record.avatar_url.take(),
        );
    }

    // ── Resolve avatar bytes (data URL, else fetch) ──────────────────────────
    let avatar_bytes = match record.avatar_url.as_deref() {
        Some(url) if url.starts_with("data:") => decode_avatar_data_url(url)
            .ok_or_else(|| "Agent avatar data URL could not be decoded.".to_string())?,
        Some(url) if url.starts_with("http://") || url.starts_with("https://") => {
            // Relay-hosted avatars (kind:0 pictures under the relay's /media/)
            // require Blossom get-auth. Mint the header ONLY for same-origin URLs
            // so the token never leaves the relay (same contract as
            // `media_download.rs`).
            let relay_base = crate::relay::relay_api_base_url_with_override(&state);
            let auth = is_same_origin(url, &relay_base)
                .then(|| crate::commands::media::mint_media_get_auth(&state, &relay_base))
                .flatten();
            fetch_avatar(url, auth.as_deref()).await?
        }
        _ => {
            return Err(
                "Agent has no avatar image. Set an avatar before minting a card.".to_string(),
            )
        }
    };

    // ── Build the manifest now (with any requested memory) so a broken agent
    //    fails before we spend minutes on the API call. ───────────────────────
    let manifest_avatar = manifest_avatar_bytes(
        lock_keys.is_some(),
        &avatar_bytes,
        record.avatar_url.as_deref(),
    )?;
    let snapshot = build_snapshot(
        &record,
        memory_level,
        memory_entries,
        manifest_avatar.as_deref(),
    );

    // ── One Responses API call ───────────────────────────────────────────────
    // For locked mints, prove the manifest (including any embedded memory)
    // fits the NIP-44 plaintext cap BEFORE spending minutes on the API call
    // (same fail-early rule as the memory guard above).
    if lock_keys.is_some() {
        let json_len =
            crate::managed_agents::agent_snapshot::encode_snapshot_json(&snapshot)?.len();
        if json_len > buzz_core_pkg::engram::NIP44_PLAINTEXT_MAX {
            let hint = if memory_level == MemoryLevel::None {
                "Reduce the avatar size or mint an unlocked card."
            } else {
                "Include less memory, reduce the avatar size, or mint an unlocked card."
            };
            return Err(format!(
                "Agent manifest is too large to lock ({json_len} bytes; the encrypted \
                 format caps at {}). {hint}",
                buzz_core_pkg::engram::NIP44_PLAINTEXT_MAX
            ));
        }
    }
    let instructions = build_card_instructions(
        &display_name,
        snapshot.definition.system_prompt.as_deref().unwrap_or(""),
        style_notes.as_deref().unwrap_or(""),
    );
    let body = serde_json::json!({
        "model": DESIGNER_MODEL,
        "reasoning": {"effort": "high"},
        "instructions": "You are a senior TCG card designer and MTG rules editor.",
        "input": [{
            "role": "user",
            "content": [
                {"type": "input_text", "text": instructions},
                {"type": "input_image", "image_url": image_data_url(CARD_TEMPLATE_PNG, 1024)?},
                {"type": "input_image", "image_url": image_data_url(&avatar_bytes, 1024)?},
            ],
        }],
        "tools": [{
            "type": "image_generation",
            "model": IMAGE_MODEL,
            "quality": "high",
            "size": "1024x1536",
            "output_format": "png",
        }],
        "tool_choice": "required",
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(MINT_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
    let resp = client
        .post(responses_url(base_url))
        .bearer_auth(&api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Card mint request failed: {e}"))?;

    let status = resp.status();
    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Card mint response was not JSON: {e}"))?;
    if !status.is_success() {
        // Never echo the request (it embeds nothing secret, but keep the
        // failure surface small); the OpenAI error body is safe to surface.
        let detail = payload
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(format!("Card mint failed (HTTP {status}): {detail}"));
    }

    let (image_b64, designer_notes) = extract_card_output(&payload)?;
    let raw_card = STANDARD
        .decode(image_b64.as_bytes())
        .map_err(|e| format!("Generated image was not valid base64: {e}"))?;

    // ── Resize to 1500-wide, inject chunk via the existing encoder ──────────
    let card_img = image::load_from_memory(&raw_card)
        .map_err(|e| format!("Generated image could not be decoded: {e}"))?;
    let scale = CARD_WIDTH as f64 / card_img.width() as f64;
    let card_img = card_img.resize(
        CARD_WIDTH,
        (card_img.height() as f64 * scale).round() as u32,
        image::imageops::FilterType::Lanczos3,
    );
    let mut card_png = Vec::new();
    card_img
        .write_to(
            &mut std::io::Cursor::new(&mut card_png),
            image::ImageFormat::Png,
        )
        .map_err(|e| format!("Failed to encode card PNG: {e}"))?;

    let final_bytes = match &lock_keys {
        None => encode_snapshot_png(&snapshot, Some(&card_png))
            .map_err(|e| format!("Failed to embed agent snapshot in card: {e}"))?,
        Some((owner_keys, agent_pubkey)) => {
            encode_locked_snapshot_png(&snapshot, owner_keys, agent_pubkey, Some(&card_png))
                .map_err(|e| format!("Failed to embed locked agent snapshot in card: {e}"))?
        }
    };

    // ── Verify: size ceiling + round-trip on the FINAL bytes ────────────────
    // Locked cards: extract the actual chunk, parse the envelope, decrypt
    // with the owner key, then compare the logical manifest (ciphertext is
    // nondeterministic — never compare bytes).
    validate_snapshot_encode_size(final_bytes.len(), true)?;
    let decoded = match &lock_keys {
        None => decode_snapshot_png(&final_bytes)
            .map_err(|e| format!("Card failed round-trip verification: {e}"))?,
        Some((owner_keys, _)) => {
            let payload = extract_chunk_payload_png(&final_bytes)
                .map_err(|e| format!("Card failed round-trip verification: {e}"))?;
            match parse_chunk_payload(&payload)
                .map_err(|e| format!("Card failed round-trip verification: {e}"))?
            {
                ChunkPayload::Locked(envelope) => {
                    decrypt_envelope(&envelope, owner_keys.secret_key())
                        .map_err(|e| format!("Card failed round-trip verification: {e}"))?
                }
                ChunkPayload::Plain(_) => {
                    return Err(
                        "Card round-trip verification failed: expected a locked envelope."
                            .to_string(),
                    )
                }
            }
        }
    };
    if decoded != snapshot {
        return Err("Card round-trip verification failed: manifest mismatch.".to_string());
    }

    let slug = crate::util::slugify(&display_name, "agent", 50);
    let minted = MintedCard {
        card_png_base64: STANDARD.encode(&final_bytes),
        file_name: format!("{slug}.agent.png"),
        designer_notes,
        locked: lock_keys.is_some(),
        memory_level,
    };

    // Archive best-effort: the mint is already paid for and verified, so a
    // failed archive write logs and continues — it never fails the mint.
    if let Err(e) = archive_minted_card(&app, &id, &display_name, &minted, &final_bytes) {
        eprintln!("buzz-desktop: card-archive: failed to archive minted card: {e}");
    }

    Ok(minted)
}

/// The avatar the mint should use: the agent's kind:0 `picture` when one is
/// published and non-blank, else the local record's `avatar_url`.
///
/// Pure so the precedence is unit-testable without a relay: a blank or
/// whitespace-only `picture` must NOT shadow a real record avatar.
fn preferred_avatar_url(
    kind0_picture: Option<String>,
    record_avatar_url: Option<String>,
) -> Option<String> {
    kind0_picture
        .filter(|p| !p.trim().is_empty())
        .or(record_avatar_url)
}

/// The avatar bytes the card manifest should inline.
///
/// Unlocked cards must carry the agent's REAL avatar inline: the PNG body is
/// the generated card artwork, and the importer only adopts the body as the
/// avatar when the manifest carries no inline bytes (`import.rs`) — without
/// these bytes an imported agent would wear the card as its face. Downscaled
/// to [`MANIFEST_AVATAR_MAX_DIM`] so the manifest tEXt chunk stays small.
///
/// Locked cards keep the data-URL-only behavior: the whole manifest must fit
/// the NIP-44 plaintext cap (65 KB), which cannot carry inline pixels, and a
/// locked envelope never reaches the import body override anyway.
fn manifest_avatar_bytes(
    locked: bool,
    avatar_bytes: &[u8],
    record_avatar_url: Option<&str>,
) -> Result<Option<Vec<u8>>, String> {
    if locked {
        return Ok(decode_avatar_data_url(record_avatar_url.unwrap_or("")));
    }
    png_bytes_resized(avatar_bytes, MANIFEST_AVATAR_MAX_DIM)
        .map(Some)
        .map_err(|e| format!("Failed to inline the agent avatar into the card manifest: {e}"))
}

/// True when `url` shares an origin (scheme, host, port) with `relay_base`.
///
/// Gate for attaching the minted media get-auth header — the token must never
/// be sent to a non-relay origin (same contract as `validate_download_url` in
/// `media_download.rs`, but non-fatal: a foreign origin just fetches
/// unauthenticated instead of failing the mint).
fn is_same_origin(url: &str, relay_base: &str) -> bool {
    match (url::Url::parse(url), url::Url::parse(relay_base)) {
        (Ok(u), Ok(b)) => u.origin() == b.origin(),
        _ => false,
    }
}

/// Fetch an avatar over HTTP with a hard size cap.
///
/// `auth` is an optional pre-minted Blossom get-auth header value, attached
/// verbatim — the caller is responsible for only supplying it for
/// relay-origin URLs. Redirects are not followed when auth is present
/// (redirect-hop guard, same rule as `media_download.rs`).
///
/// The cap bounds network and memory, not just the final buffer: the
/// Content-Length header is checked before any body bytes are read, and the
/// body is streamed with a running count so a missing or dishonest header
/// still cannot exceed the cap (same contract as `media_download.rs`).
async fn fetch_avatar(url: &str, auth: Option<&str>) -> Result<Vec<u8>, String> {
    use futures_util::StreamExt;

    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30));
    if auth.is_some() {
        // Never let a relay 3xx forward the auth header across origins.
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    let client = builder
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
    let mut req = client.get(url);
    if let Some(auth) = auth {
        req = req.header("authorization", auth);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Failed to fetch agent avatar: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Avatar fetch failed: HTTP {}", resp.status()));
    }

    if let Some(content_length) = resp.content_length() {
        if content_length > MAX_AVATAR_FETCH_BYTES as u64 {
            return Err("Agent avatar is too large to use as card input.".to_string());
        }
    }

    let mut bytes = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Failed to read avatar bytes: {e}"))?;
        append_within_avatar_cap(&mut bytes, &chunk)?;
    }
    Ok(bytes)
}

/// Append a body chunk to the avatar buffer, rejecting before the append if
/// the total would cross `MAX_AVATAR_FETCH_BYTES`. Split out so the cap
/// boundary is unit-testable without an HTTP server.
fn append_within_avatar_cap(buf: &mut Vec<u8>, chunk: &[u8]) -> Result<(), String> {
    if buf.len() + chunk.len() > MAX_AVATAR_FETCH_BYTES {
        return Err("Agent avatar is too large to use as card input.".to_string());
    }
    buf.extend_from_slice(chunk);
    Ok(())
}

/// Save previously minted card bytes to disk via the OS save dialog.
///
/// Re-validates the bytes (chunk parses as a plain manifest or a
/// structurally valid locked envelope, size within the import ceiling) so a
/// corrupted preview can never be written as a `.agent.png`. No decryption
/// happens here — the mint already round-trip-verified with the real key.
#[tauri::command]
pub async fn save_agent_card(
    card_png_base64: String,
    file_name: String,
    app: AppHandle,
) -> Result<bool, String> {
    let bytes = STANDARD
        .decode(card_png_base64.as_bytes())
        .map_err(|e| format!("Card bytes were not valid base64: {e}"))?;
    validate_snapshot_encode_size(bytes.len(), true)?;
    let payload = extract_chunk_payload_png(&bytes)
        .map_err(|e| format!("Refusing to save: card failed snapshot validation: {e}"))?;
    parse_chunk_payload(&payload)
        .map_err(|e| format!("Refusing to save: card failed snapshot validation: {e}"))?;

    let safe_name = if file_name.ends_with(".agent.png") && !file_name.contains(['/', '\\']) {
        file_name
    } else {
        "card.agent.png".to_string()
    };
    save_bytes_with_dialog(&app, &safe_name, "Agent card", &["png"], &bytes).await
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
