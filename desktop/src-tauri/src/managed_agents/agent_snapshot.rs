//! `buzz-agent-snapshot v1` — manifest type, encoder, and decoder stubs.
//!
//! An agent snapshot is a portable, shareable representation of an agent
//! definition. It captures:
//!   - **definition** — behavioral config (prompt, runtime, model, …)
//!   - **profile** — kind:0 presentation (name, about, avatar)
//!   - **memory** — optional, owner-decrypted engrams at one of three levels
//!
//! Two encodings are supported:
//!   - `.agent.json` — canonical snapshot manifest
//!   - `.agent.png` — avatar image with manifest in a `buzz_agent_snapshot`
//!     tEXt chunk
//!
//! Both formats may carry memory at any level. Memory entries are plaintext,
//! so callers must require an explicit opt-in before exporting them.
//!
//! **Zip is NOT in v1** — deferred to v2 for skills bundling.
//!
//! # Secret exclusion
//!
//! The following fields are NEVER serialized:
//!   - `private_key_nsec` / any private key material
//!   - `auth_tag` (NIP-OA)
//!   - `env_vars` (API keys / credentials)
//!   - `relay_url` (machine-local endpoint)
//!   - `acp_command` / `agent_command` / `agent_command_override` / `agent_args`
//!     (machine-local harness paths)
//!   - `mcp_command` (machine-local)
//!   - runtime state: `runtime_pid`, `backend_agent_id`, `backend` blob,
//!     `provider_binary_path`, `last_*`
//!   - lineage ids: `persona_id`, `team_id`, `source_team`, `source_team_persona_slug`,
//!     `persona_source_version`
//!   - internal bookkeeping: `start_on_app_launch`,
//!     `auto_restart_on_config_change`
//!
//! The portable `sourceIsBuiltIn` hint preserves how the exported definition
//! should be described in an import preview. It never grants built-in status
//! to the newly imported definition.
//!
//! These exclusions are enforced by construction (only explicit fields are
//! placed into `AgentSnapshotDefinition`) and asserted by unit tests.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use png::{BitDepth, ColorType, Decoder, Encoder};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

use crate::managed_agents::types::ManagedAgentRecord;

// ── Constants ────────────────────────────────────────────────────────────────

/// tEXt chunk keyword used in `.agent.png` files.
pub const PNG_CHUNK_KEYWORD: &str = "buzz_agent_snapshot";

/// Maximum avatar size (bytes) to inline as a data URL. Avatars larger than
/// this are stored as a URL reference instead.
const MAX_AVATAR_INLINE_BYTES: usize = 2 * 1024 * 1024; // 2 MB

/// Format discriminator — used for sniffing and validation.
pub const FORMAT_DISCRIMINATOR: &str = "buzz-agent-snapshot";

/// Version of the manifest format produced by this module.
pub const FORMAT_VERSION: u32 = 1;

// ── Memory level ─────────────────────────────────────────────────────────────

/// How much memory to bundle in the snapshot.
///
/// The default is `None` — config-only export, safest for sharing. Memory
/// entries are plaintext in the output file; users must opt in explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLevel {
    /// Export definition + profile only. No memory. (Default)
    #[default]
    None,
    /// Export definition + profile + `core` memory only.
    Core,
    /// Export definition + profile + `core` + all `mem/*` entries.
    Everything,
}

// ── Manifest sub-types ────────────────────────────────────────────────────────

/// Behavioral definition — what makes the agent do what it does.
///
/// Fields mirror `ManagedAgentRecord` definition-level fields. Only the subset
/// meaningful across environments is included; machine-local / secret fields
/// are deliberately absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotDefinition {
    pub name: String,
    /// Portable source classification for import-preview metadata. Imported
    /// definitions are still created as custom agents with fresh identities.
    #[serde(default)]
    pub source_is_builtin: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
    /// Allowlist entries. These are flagged during import — they come from the
    /// source environment and are meaningless on the importer's relay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub respond_to_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_pool: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turn_duration_seconds: Option<u64>,
}

/// kind:0 presentation fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotProfile {
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Avatar inlined as a `data:image/...;base64,…` URI (≤ 2 MB),
    /// or a URL fallback if the image exceeds the size limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_data_url: Option<String>,
    /// Present when the avatar exceeds MAX_AVATAR_INLINE_BYTES and is stored
    /// by reference rather than inlined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// A single decrypted memory entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotMemoryEntry {
    pub slug: String,
    pub body: String,
}

/// Memory section of the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotMemory {
    /// Indicates what was included at export time.
    pub level: MemoryLevel,
    /// Decrypted memory entries. Empty when `level == None`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<AgentSnapshotMemoryEntry>,
}

// ── Top-level manifest ────────────────────────────────────────────────────────

/// The top-level `buzz-agent-snapshot v1` manifest.
///
/// Serializes to / from JSON. Embedded in `.agent.json` directly, or in the
/// `buzz_agent_snapshot` tEXt chunk of a `.agent.png` (base64-encoded).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshot {
    /// Fixed discriminator for format sniffing.
    pub format: String,
    /// Schema version. This module produces version 1.
    pub version: u32,
    pub definition: AgentSnapshotDefinition,
    pub profile: AgentSnapshotProfile,
    pub memory: AgentSnapshotMemory,
}

// ── Builder / encoder ────────────────────────────────────────────────────────

/// Materialize a snapshot manifest from a `ManagedAgentRecord`.
///
/// `memory_entries` is the pre-fetched, owner-decrypted set from
/// `get_agent_memory`; this function does NOT call the Tauri command — that
/// is the caller's responsibility so this fn stays pure and testable.
///
/// `memory_level` controls what ends up in the `memory` section. `avatar_bytes`
/// is the raw image for the agent (loaded from disk or fetched); when `None`
/// or too large, falls back to the `avatar_url` string on the record.
pub fn build_snapshot(
    record: &ManagedAgentRecord,
    memory_level: MemoryLevel,
    memory_entries: Vec<AgentSnapshotMemoryEntry>,
    avatar_bytes: Option<&[u8]>,
) -> AgentSnapshot {
    // ── Definition ─────────────────────────────────────────────────────
    // Use definition-level fields (respond_to, allowlist, parallelism) for
    // portability — instance-level equivalents are spawn-time snapshots and
    // would be stale.
    let definition = AgentSnapshotDefinition {
        name: record
            .display_name
            .clone()
            .unwrap_or_else(|| record.name.clone()),
        source_is_builtin: record.is_builtin,
        system_prompt: record.system_prompt.clone(),
        runtime: record.runtime.clone(),
        model: record.model.clone(),
        provider: record.provider.clone(),
        parallelism: record.definition_parallelism.or(Some(record.parallelism)),
        respond_to: record.definition_respond_to.clone(),
        respond_to_allowlist: record.definition_respond_to_allowlist.clone(),
        name_pool: record.name_pool.clone(),
        idle_timeout_seconds: record.idle_timeout_seconds,
        max_turn_duration_seconds: record.max_turn_duration_seconds,
    };

    // ── Profile ─────────────────────────────────────────────────────────
    let (avatar_data_url, avatar_url_ref) = resolve_avatar(record, avatar_bytes);
    let profile = AgentSnapshotProfile {
        display_name: record
            .display_name
            .clone()
            .unwrap_or_else(|| record.name.clone()),
        about: None, // kind:0 `about` not yet surfaced in ManagedAgentRecord
        avatar_data_url,
        avatar_url: avatar_url_ref,
    };

    // ── Memory ─────────────────────────────────────────────────────────
    let memory = AgentSnapshotMemory {
        level: memory_level,
        entries: memory_entries,
    };

    AgentSnapshot {
        format: FORMAT_DISCRIMINATOR.to_string(),
        version: FORMAT_VERSION,
        definition,
        profile,
        memory,
    }
}

/// Resolve the avatar for export.
///
/// Returns `(data_url, url_ref)`:
///   - `data_url` is set when the avatar fits within `MAX_AVATAR_INLINE_BYTES`.
///   - `url_ref` is set when we can only record a URL (too large / no bytes).
fn resolve_avatar(
    record: &ManagedAgentRecord,
    avatar_bytes: Option<&[u8]>,
) -> (Option<String>, Option<String>) {
    if let Some(bytes) = avatar_bytes {
        if bytes.len() <= MAX_AVATAR_INLINE_BYTES {
            // Detect MIME type from magic bytes.
            let mime = if bytes.starts_with(b"\x89PNG") {
                "image/png"
            } else if bytes.starts_with(b"\xff\xd8\xff") {
                "image/jpeg"
            } else if bytes.starts_with(b"GIF8") {
                "image/gif"
            } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
                "image/webp"
            } else {
                "image/png" // safe default for unknown
            };
            let data_url = format!("data:{};base64,{}", mime, STANDARD.encode(bytes));
            return (Some(data_url), None);
        }
    }
    // Fall back to URL reference (caller provided a URL avatar or bytes were
    // too large).
    let url_ref = record
        .avatar_url
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    (None, url_ref)
}

// ── JSON encoding / decoding ──────────────────────────────────────────────────

/// Encode the manifest to pretty-printed JSON bytes.
pub fn encode_snapshot_json(snapshot: &AgentSnapshot) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(snapshot).map_err(|e| format!("Failed to serialize snapshot: {e}"))
}

/// Decode a manifest from JSON bytes.
pub fn decode_snapshot_json(bytes: &[u8]) -> Result<AgentSnapshot, String> {
    let snapshot: AgentSnapshot =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid snapshot JSON: {e}"))?;
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

// ── PNG encoding / decoding ───────────────────────────────────────────────────

/// Encode a snapshot into a `.agent.png` — avatar as the image body, manifest
/// in the `buzz_agent_snapshot` tEXt chunk.
pub fn encode_snapshot_png(
    snapshot: &AgentSnapshot,
    avatar_bytes: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    if snapshot.memory.level == MemoryLevel::None && !snapshot.memory.entries.is_empty() {
        return Err(
            "Cannot write a snapshot with memory.level 'none' and non-empty memory entries."
                .to_string(),
        );
    }

    // Manifest → JSON for the tEXt chunk payload. The payload/PNG composition
    // is shared with the locked-card encoder in `agent_snapshot_envelope`;
    // plain cards remain byte-identical to the pre-envelope encoder.
    let json_bytes = encode_snapshot_json(snapshot)?;
    encode_chunk_payload_png(&json_bytes, avatar_bytes)
}

/// Encode arbitrary chunk-payload JSON (plain manifest or locked envelope)
/// into a PNG carrying it base64-encoded in the `buzz_agent_snapshot` tEXt
/// chunk. Shared by the plain encoder above and
/// `agent_snapshot_envelope::encode_locked_snapshot_png`.
pub(crate) fn encode_chunk_payload_png(
    json_bytes: &[u8],
    avatar_bytes: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let chunk_text = STANDARD.encode(json_bytes);

    // Use the avatar as the PNG image body, transcoding decodable non-PNG
    // avatars. Fall back to a minimal 1×1 transparent placeholder only when
    // there is no avatar or it cannot be decoded.
    let png_bytes = match avatar_bytes.filter(|bytes| !bytes.is_empty()) {
        Some(bytes) => {
            let encoded_avatar = if bytes.starts_with(b"\x89PNG") {
                inject_text_chunk(bytes, PNG_CHUNK_KEYWORD, &chunk_text).or_else(|_| {
                    transcode_avatar_to_png_with_text(bytes, PNG_CHUNK_KEYWORD, &chunk_text)
                })
            } else {
                transcode_avatar_to_png_with_text(bytes, PNG_CHUNK_KEYWORD, &chunk_text)
            };

            match encoded_avatar {
                Ok(png_bytes) => png_bytes,
                Err(_) => make_png_with_text(PNG_CHUNK_KEYWORD, &chunk_text)?,
            }
        }
        None => make_png_with_text(PNG_CHUNK_KEYWORD, &chunk_text)?,
    };

    Ok(png_bytes)
}

/// Extract and base64-decode the raw `buzz_agent_snapshot` chunk payload
/// (JSON bytes) from a PNG, without interpreting it. The payload may be a
/// plain manifest or a locked envelope — callers dispatch on the parsed
/// `format` via `agent_snapshot_envelope::parse_chunk_payload`.
pub(crate) fn extract_chunk_payload_png(png_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let decoder = Decoder::new(Cursor::new(png_bytes));
    let reader = decoder
        .read_info()
        .map_err(|e| format!("Invalid PNG: {e}"))?;
    let info = reader.info();

    let chunk_text = info
        .uncompressed_latin1_text
        .iter()
        .find(|c| c.keyword == PNG_CHUNK_KEYWORD)
        .map(|c| c.text.as_str())
        .ok_or_else(|| "PNG does not contain a buzz_agent_snapshot tEXt chunk".to_string())?;

    STANDARD
        .decode(chunk_text.trim())
        .map_err(|e| format!("Invalid base64 in PNG chunk: {e}"))
}

/// Decode a manifest from a `.agent.png` tEXt chunk.
///
/// Plain snapshots only — a locked (encrypted) chunk payload fails here with
/// the manifest format error. Import paths that must handle locked cards go
/// through `agent_snapshot_envelope::parse_chunk_payload` instead.
pub fn decode_snapshot_png(png_bytes: &[u8]) -> Result<AgentSnapshot, String> {
    let json_bytes = extract_chunk_payload_png(png_bytes)?;
    decode_snapshot_json(&json_bytes)
}

// ── Validation ────────────────────────────────────────────────────────────────

/// Validate that the manifest has the correct format/version and required
/// fields. Returns an error string on failure.
pub(crate) fn validate_snapshot(snapshot: &AgentSnapshot) -> Result<(), String> {
    if snapshot.format != FORMAT_DISCRIMINATOR {
        return Err(format!(
            "Unsupported snapshot format: {:?} (expected {:?})",
            snapshot.format, FORMAT_DISCRIMINATOR
        ));
    }
    if snapshot.version != 1 {
        return Err(format!(
            "Unsupported snapshot version: {} (expected 1)",
            snapshot.version
        ));
    }
    if snapshot.definition.name.trim().is_empty() {
        return Err("Snapshot definition.name is empty".to_string());
    }
    if snapshot.profile.display_name.trim().is_empty() {
        return Err("Snapshot profile.displayName is empty".to_string());
    }
    Ok(())
}

// ── PNG helpers ───────────────────────────────────────────────────────────────

/// Decode a `data:<mime>;base64,<data>` URL back to raw bytes.
/// Returns `None` if `url` is not a data URL or decoding fails.
pub fn decode_avatar_data_url(url: &str) -> Option<Vec<u8>> {
    let rest = url.strip_prefix("data:")?;
    let comma_pos = rest.find(',')?;
    let header = &rest[..comma_pos];
    let b64 = &rest[comma_pos + 1..];
    if !header.contains("base64") {
        return None;
    }
    STANDARD.decode(b64.trim()).ok()
}

/// Build a minimal 1×1 transparent PNG with a single tEXt chunk.
pub(crate) fn make_png_with_text(keyword: &str, text: &str) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let mut enc = Encoder::new(Cursor::new(&mut buf), 1, 1);
        enc.set_color(ColorType::Rgba);
        enc.set_depth(BitDepth::Eight);
        enc.add_text_chunk(keyword.to_string(), text.to_string())
            .map_err(|e| format!("Failed to add tEXt chunk: {e}"))?;
        let mut w = enc
            .write_header()
            .map_err(|e| format!("Failed to write PNG header: {e}"))?;
        w.write_image_data(&[0, 0, 0, 0])
            .map_err(|e| format!("Failed to write PNG image data: {e}"))?;
    }
    Ok(buf)
}

/// Transcode a decodable avatar to PNG and add the snapshot manifest chunk.
fn transcode_avatar_to_png_with_text(
    avatar_bytes: &[u8],
    keyword: &str,
    text: &str,
) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory(avatar_bytes)
        .map_err(|e| format!("Failed to decode avatar image: {e}"))?;
    let mut png_bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode avatar as PNG: {e}"))?;
    inject_text_chunk(&png_bytes, keyword, text)
}

/// Inject a tEXt chunk into an existing PNG by re-encoding it.
///
/// Re-decodes the image data via the `png` crate and writes a fresh PNG with
/// the extra chunk inserted after IHDR. This preserves the image content while
/// adding our metadata.
fn inject_text_chunk(png_bytes: &[u8], keyword: &str, text: &str) -> Result<Vec<u8>, String> {
    let decoder = Decoder::new(Cursor::new(png_bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("Failed to decode source PNG: {e}"))?;

    let info = reader.info().clone();
    let width = info.width;
    let height = info.height;
    let color_type = info.color_type;
    let bit_depth = info.bit_depth;
    let buf_size = reader
        .output_buffer_size()
        .ok_or_else(|| "PNG output buffer size unavailable".to_string())?;
    let mut pixel_buf = vec![0u8; buf_size];
    reader
        .next_frame(&mut pixel_buf)
        .map_err(|e| format!("Failed to read PNG frame: {e}"))?;

    let mut out = Vec::new();
    {
        let mut enc = Encoder::new(Cursor::new(&mut out), width, height);
        enc.set_color(color_type);
        enc.set_depth(bit_depth);
        enc.add_text_chunk(keyword.to_string(), text.to_string())
            .map_err(|e| format!("Failed to add tEXt chunk: {e}"))?;
        let mut w = enc
            .write_header()
            .map_err(|e| format!("Failed to write PNG header: {e}"))?;
        w.write_image_data(&pixel_buf)
            .map_err(|e| format!("Failed to write PNG pixel data: {e}"))?;
    }
    Ok(out)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "agent_snapshot_tests.rs"]
mod tests;
