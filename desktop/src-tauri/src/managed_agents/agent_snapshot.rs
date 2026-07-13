//! `buzz-agent-snapshot v1` — manifest type, encoder, and decoder stubs.
//!
//! An agent snapshot is a portable, shareable representation of an agent
//! definition. It captures:
//!   - **definition** — behavioral config (prompt, runtime, model, …)
//!   - **profile** — kind:0 presentation (name, about, avatar)
//!   - **memory** — optional, owner-decrypted engrams at one of three levels
//!
//! Two encodings are supported:
//!   - `.agent.json` — canonical, may carry memory at any level
//!   - `.agent.png` — avatar image with manifest in a `buzz_agent_snapshot`
//!     tEXt chunk; memory MUST be `None` (PNG files are casually shared and
//!     would leak plaintext memory into chat uploads)
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
//!   - lineage ids: `persona_id`, `source_team`, `source_team_persona_slug`,
//!     `persona_team_dir`, `persona_name_in_team`, `persona_source_version`
//!   - internal bookkeeping: `start_on_app_launch`,
//!     `auto_restart_on_config_change`, `is_builtin`
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
///
/// **Rejects** any snapshot whose `memory.level != None` — memory in a PNG
/// file is a security hazard (images are casually shared, pasted into chats).
pub fn encode_snapshot_png(
    snapshot: &AgentSnapshot,
    avatar_bytes: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    if snapshot.memory.level != MemoryLevel::None || !snapshot.memory.entries.is_empty() {
        return Err(
            "Cannot write memory to a .agent.png file — use .agent.json for memory-bearing \
             snapshots. PNG images are casually shared and would expose memory as plaintext."
                .to_string(),
        );
    }

    // Manifest → JSON → base64 for the tEXt chunk payload.
    let json_bytes = encode_snapshot_json(snapshot)?;
    let chunk_text = STANDARD.encode(&json_bytes);

    // Use the avatar bytes as the PNG image when available; otherwise produce
    // a minimal 1×1 transparent placeholder.
    let png_bytes = match avatar_bytes.filter(|b| !b.is_empty()) {
        Some(bytes) if bytes.starts_with(b"\x89PNG") => {
            // Already a PNG — inject the tEXt chunk.
            inject_text_chunk(bytes, PNG_CHUNK_KEYWORD, &chunk_text)?
        }
        Some(_bytes) => {
            // Non-PNG avatar (JPEG, etc.) — re-encode as 1×1 placeholder and
            // carry the avatar via the manifest's `profile.avatar_data_url`.
            // This keeps the PNG valid while preserving the avatar in the JSON.
            make_png_with_text(PNG_CHUNK_KEYWORD, &chunk_text)?
        }
        None => make_png_with_text(PNG_CHUNK_KEYWORD, &chunk_text)?,
    };

    Ok(png_bytes)
}

/// Decode a manifest from a `.agent.png` tEXt chunk.
pub fn decode_snapshot_png(png_bytes: &[u8]) -> Result<AgentSnapshot, String> {
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

    let json_bytes = STANDARD
        .decode(chunk_text.trim())
        .map_err(|e| format!("Invalid base64 in PNG chunk: {e}"))?;

    decode_snapshot_json(&json_bytes)
}

// ── Validation ────────────────────────────────────────────────────────────────

/// Validate that the manifest has the correct format/version and required
/// fields. Returns an error string on failure.
fn validate_snapshot(snapshot: &AgentSnapshot) -> Result<(), String> {
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
fn make_png_with_text(keyword: &str, text: &str) -> Result<Vec<u8>, String> {
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
mod tests {
    use super::*;
    use crate::managed_agents::types::{BackendKind, ManagedAgentRecord, RespondTo};
    use std::collections::BTreeMap;

    /// Build a minimal `ManagedAgentRecord` for testing. Only the fields
    /// relevant to snapshot export are filled; the rest use defaults.
    fn minimal_record() -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: "deadbeef".to_string(),
            name: "Test Agent".to_string(),
            display_name: Some("Test Agent Display".to_string()),
            persona_id: Some("SENTINEL_PERSONA_ID".to_string()), // MUST NOT appear in snapshot
            private_key_nsec: "nsec1secret".to_string(),         // MUST NOT appear in snapshot
            auth_tag: Some("auth-tag-secret".to_string()),       // MUST NOT appear in snapshot
            relay_url: "wss://relay.example.com".to_string(),    // MUST NOT appear in snapshot
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            acp_command: "/usr/local/bin/acp".to_string(), // MUST NOT appear in snapshot
            agent_command: "goose".to_string(),            // MUST NOT appear in snapshot
            agent_command_override: Some("goose-override".to_string()), // MUST NOT appear
            agent_args: vec!["--arg".to_string()],         // MUST NOT appear in snapshot
            mcp_command: "mcp-server".to_string(),         // MUST NOT appear in snapshot
            turn_timeout_seconds: 120,                     // deprecated, MUST NOT appear
            idle_timeout_seconds: Some(30),
            max_turn_duration_seconds: Some(600),
            parallelism: 2,
            system_prompt: Some("You are a test agent.".to_string()),
            model: Some("claude-opus-4".to_string()),
            provider: Some("anthropic".to_string()),
            persona_source_version: Some("v1.0".to_string()), // MUST NOT appear
            env_vars: {
                let mut m = BTreeMap::new();
                m.insert("API_KEY".to_string(), "secret123".to_string()); // MUST NOT appear
                m
            },
            start_on_app_launch: true,
            auto_restart_on_config_change: true,
            runtime_pid: Some(12345), // MUST NOT appear
            backend: BackendKind::Provider {
                // MUST NOT appear — carries a provider secret
                id: "SENTINEL_BACKEND_ID".to_string(),
                config: serde_json::json!({"api_key": "SENTINEL_BACKEND_SECRET"}),
            },
            backend_agent_id: Some("SENTINEL_BACKEND_AGENT_ID".to_string()), // MUST NOT appear
            provider_binary_path: Some("/usr/bin/SENTINEL_PROVIDER_BINARY".to_string()), // MUST NOT appear
            persona_team_dir: Some(std::path::PathBuf::from("SENTINEL_TEAM_DIR")), // MUST NOT appear
            persona_name_in_team: Some("SENTINEL_NAME_IN_TEAM".to_string()), // MUST NOT appear
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
            last_started_at: Some("2024-01-03T00:00:00Z".to_string()), // MUST NOT appear
            last_stopped_at: None,
            last_exit_code: Some(0), // MUST NOT appear
            last_error: Some("SENTINEL_LAST_ERROR".to_string()), // MUST NOT appear
            last_error_code: Some(42), // MUST NOT appear
            respond_to: RespondTo::default(),
            respond_to_allowlist: vec!["pubkey1hex".to_string()],
            slug: Some("test-agent".to_string()),
            runtime: Some("goose".to_string()),
            name_pool: vec!["Alice".to_string(), "Bob".to_string()],
            is_builtin: false,
            is_active: true,
            source_team: Some("team-id-123".to_string()), // MUST NOT appear
            source_team_persona_slug: Some("lep".to_string()), // MUST NOT appear
            definition_respond_to: Some("allowlist".to_string()),
            definition_respond_to_allowlist: vec!["abc123def".to_string()],
            definition_parallelism: Some(4),
            relay_mesh: None,
        }
    }

    // ── Round-trip tests ──────────────────────────────────────────────────────

    #[test]
    fn json_round_trip_config_only() {
        let record = minimal_record();
        let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], None);
        let bytes = encode_snapshot_json(&snapshot).unwrap();
        let parsed = decode_snapshot_json(&bytes).unwrap();
        assert_eq!(parsed, snapshot);
    }

    #[test]
    fn json_round_trip_with_memory() {
        let record = minimal_record();
        let entries = vec![
            AgentSnapshotMemoryEntry {
                slug: "core".to_string(),
                body: "I am a test agent.".to_string(),
            },
            AgentSnapshotMemoryEntry {
                slug: "mem/research".to_string(),
                body: "Some research notes.".to_string(),
            },
        ];
        let snapshot = build_snapshot(&record, MemoryLevel::Everything, entries, None);
        let bytes = encode_snapshot_json(&snapshot).unwrap();
        let parsed = decode_snapshot_json(&bytes).unwrap();
        assert_eq!(parsed, snapshot);
    }

    #[test]
    fn png_round_trip_no_memory() {
        let record = minimal_record();
        let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], None);
        let png_bytes = encode_snapshot_png(&snapshot, None).unwrap();
        let parsed = decode_snapshot_png(&png_bytes).unwrap();
        assert_eq!(parsed.definition.name, snapshot.definition.name);
        assert_eq!(parsed.profile.display_name, snapshot.profile.display_name);
        assert_eq!(parsed.memory.level, MemoryLevel::None);
    }

    #[test]
    fn png_round_trip_with_avatar_png() {
        // Build a minimal PNG avatar.
        let avatar = make_png_with_text("dummy", "value").unwrap();
        let record = minimal_record();
        let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], Some(&avatar));
        // Avatar should be inlined as a data URL.
        assert!(snapshot
            .profile
            .avatar_data_url
            .as_deref()
            .unwrap_or("")
            .starts_with("data:image/png;base64,"));

        let png_bytes = encode_snapshot_png(&snapshot, Some(&avatar)).unwrap();
        let parsed = decode_snapshot_png(&png_bytes).unwrap();
        assert_eq!(parsed.definition.name, snapshot.definition.name);
    }

    // ── PNG memory guard ──────────────────────────────────────────────────────

    #[test]
    fn png_export_with_core_memory_is_rejected() {
        let record = minimal_record();
        let entries = vec![AgentSnapshotMemoryEntry {
            slug: "core".to_string(),
            body: "secret memory".to_string(),
        }];
        let snapshot = build_snapshot(&record, MemoryLevel::Core, entries, None);
        let result = encode_snapshot_png(&snapshot, None);
        assert!(result.is_err(), "Expected error for PNG with memory");
        let err = result.unwrap_err();
        assert!(
            err.contains("Cannot write memory to a .agent.png"),
            "Error message should explain the PNG memory restriction, got: {err}"
        );
    }

    #[test]
    fn png_export_with_everything_memory_is_rejected() {
        let record = minimal_record();
        let entries = vec![AgentSnapshotMemoryEntry {
            slug: "mem/notes".to_string(),
            body: "private notes".to_string(),
        }];
        let snapshot = build_snapshot(&record, MemoryLevel::Everything, entries, None);
        let result = encode_snapshot_png(&snapshot, None);
        assert!(result.is_err());
    }

    #[test]
    fn png_export_with_no_memory_succeeds() {
        let record = minimal_record();
        let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], None);
        assert!(encode_snapshot_png(&snapshot, None).is_ok());
    }

    #[test]
    fn png_export_rejects_none_level_with_nonempty_entries() {
        // Inconsistent state: level == None but entries is non-empty.
        // The encoder must reject this to prevent a memory-leak bypass.
        let record = minimal_record();
        let entries = vec![AgentSnapshotMemoryEntry {
            slug: "core".to_string(),
            body: "leaked memory".to_string(),
        }];
        // Build with entries, then override level to None in the struct.
        let mut snapshot = build_snapshot(&record, MemoryLevel::Core, entries, None);
        snapshot.memory.level = MemoryLevel::None; // force inconsistency
        let result = encode_snapshot_png(&snapshot, None);
        assert!(
            result.is_err(),
            "PNG encoder must reject level=None with non-empty entries"
        );
        assert!(
            result.unwrap_err().contains("Cannot write memory"),
            "Error must explain the PNG memory restriction"
        );
    }

    // ── Secret exclusion tests ────────────────────────────────────────────────
    //
    // These tests assert that every field in the exclusion list is absent from
    // the serialized snapshot. We serialize to JSON and assert the key is NOT
    // present.

    fn snapshot_json_string(record: &ManagedAgentRecord) -> String {
        let snapshot = build_snapshot(record, MemoryLevel::None, vec![], None);
        let bytes = encode_snapshot_json(&snapshot).unwrap();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn secret_exclusion_private_key_nsec_absent() {
        let record = minimal_record();
        let json = snapshot_json_string(&record);
        assert!(
            !json.contains("nsec1secret"),
            "nsec must not appear in snapshot"
        );
        assert!(
            !json.contains("privateKeyNsec") && !json.contains("private_key_nsec"),
            "privateKeyNsec field must not appear in snapshot"
        );
    }

    #[test]
    fn secret_exclusion_auth_tag_absent() {
        let record = minimal_record();
        let json = snapshot_json_string(&record);
        assert!(
            !json.contains("auth-tag-secret"),
            "auth_tag value must not appear in snapshot"
        );
        assert!(
            !json.contains("authTag") && !json.contains("auth_tag"),
            "authTag field must not appear in snapshot"
        );
    }

    #[test]
    fn secret_exclusion_env_vars_absent() {
        let record = minimal_record();
        let json = snapshot_json_string(&record);
        assert!(
            !json.contains("API_KEY") && !json.contains("secret123"),
            "env_vars content must not appear in snapshot"
        );
        assert!(
            !json.contains("envVars") && !json.contains("env_vars"),
            "envVars field must not appear in snapshot"
        );
    }

    #[test]
    fn secret_exclusion_relay_url_absent() {
        let record = minimal_record();
        let json = snapshot_json_string(&record);
        assert!(
            !json.contains("wss://relay.example.com"),
            "relay_url value must not appear in snapshot"
        );
        assert!(
            !json.contains("relayUrl") && !json.contains("relay_url"),
            "relayUrl field must not appear in snapshot"
        );
    }

    #[test]
    fn snapshot_omits_removed_mcp_toolsets_config() {
        let record = minimal_record();
        let json = snapshot_json_string(&record);
        assert!(
            !json.contains("mcpToolsets") && !json.contains("mcp_toolsets"),
            "removed MCP toolsets config must not re-enter snapshots"
        );
    }

    #[test]
    fn secret_exclusion_machine_commands_absent() {
        let record = minimal_record();
        let json = snapshot_json_string(&record);
        // acp_command / agent_command / agent_command_override / agent_args / mcp_command
        assert!(
            !json.contains("/usr/local/bin/acp"),
            "acp_command path must not appear"
        );
        assert!(
            !json.contains("acpCommand") && !json.contains("acp_command"),
            "acpCommand field must not appear"
        );
        assert!(
            !json.contains("agentCommand") && !json.contains("agent_command"),
            "agentCommand field must not appear"
        );
        assert!(
            !json.contains("mcpCommand") && !json.contains("mcp_command"),
            "mcpCommand field must not appear"
        );
    }

    #[test]
    fn secret_exclusion_runtime_state_absent() {
        let record = minimal_record();
        let json = snapshot_json_string(&record);
        assert!(
            !json.contains("runtimePid") && !json.contains("runtime_pid"),
            "runtimePid must not appear"
        );
        assert!(
            !json.contains("backendAgentId") && !json.contains("backend_agent_id"),
            "backendAgentId must not appear"
        );
        assert!(
            !json.contains("SENTINEL_BACKEND_AGENT_ID"),
            "backendAgentId value must not appear"
        );
        assert!(
            !json.contains("providerBinaryPath") && !json.contains("provider_binary_path"),
            "providerBinaryPath must not appear"
        );
        assert!(
            !json.contains("SENTINEL_PROVIDER_BINARY"),
            "providerBinaryPath value must not appear"
        );
        assert!(
            !json.contains("lastStartedAt") && !json.contains("last_started_at"),
            "lastStartedAt must not appear"
        );
        assert!(
            !json.contains("lastExitCode") && !json.contains("last_exit_code"),
            "lastExitCode must not appear"
        );
        // backend blob — neither the type tag nor provider secret must leak.
        assert!(
            !json.contains("\"backend\"") && !json.contains("backend"),
            "backend field must not appear"
        );
        assert!(
            !json.contains("SENTINEL_BACKEND_ID") && !json.contains("SENTINEL_BACKEND_SECRET"),
            "backend config values must not appear"
        );
        // last_error / last_error_code
        assert!(
            !json.contains("lastError") && !json.contains("last_error"),
            "lastError must not appear"
        );
        assert!(
            !json.contains("SENTINEL_LAST_ERROR"),
            "lastError value must not appear"
        );
        assert!(
            !json.contains("lastErrorCode") && !json.contains("last_error_code"),
            "lastErrorCode must not appear"
        );
    }

    #[test]
    fn secret_exclusion_lineage_ids_absent() {
        let record = minimal_record();
        let json = snapshot_json_string(&record);
        assert!(
            !json.contains("team-id-123"),
            "source_team value must not appear"
        );
        assert!(
            !json.contains("sourceTeam") && !json.contains("source_team"),
            "sourceTeam field must not appear"
        );
        assert!(
            !json.contains("sourceTeamPersonaSlug"),
            "sourceTeamPersonaSlug must not appear"
        );
        assert!(
            !json.contains("personaSourceVersion") && !json.contains("persona_source_version"),
            "personaSourceVersion must not appear"
        );
        // personaId
        assert!(
            !json.contains("personaId") && !json.contains("persona_id"),
            "personaId field must not appear"
        );
        assert!(
            !json.contains("SENTINEL_PERSONA_ID"),
            "personaId value must not appear"
        );
        // personaTeamDir
        assert!(
            !json.contains("personaTeamDir") && !json.contains("persona_team_dir"),
            "personaTeamDir field must not appear"
        );
        assert!(
            !json.contains("SENTINEL_TEAM_DIR"),
            "personaTeamDir value must not appear"
        );
        // personaNameInTeam
        assert!(
            !json.contains("personaNameInTeam") && !json.contains("persona_name_in_team"),
            "personaNameInTeam field must not appear"
        );
        assert!(
            !json.contains("SENTINEL_NAME_IN_TEAM"),
            "personaNameInTeam value must not appear"
        );
    }

    // ── Definition field presence tests ──────────────────────────────────────

    #[test]
    fn definition_fields_present_in_snapshot() {
        let record = minimal_record();
        let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], None);

        assert_eq!(snapshot.definition.name, "Test Agent Display");
        assert_eq!(
            snapshot.definition.system_prompt.as_deref(),
            Some("You are a test agent.")
        );
        assert_eq!(snapshot.definition.runtime.as_deref(), Some("goose"));
        assert_eq!(snapshot.definition.model.as_deref(), Some("claude-opus-4"));
        assert_eq!(snapshot.definition.provider.as_deref(), Some("anthropic"));
        assert_eq!(snapshot.definition.name_pool, vec!["Alice", "Bob"]);
        // definition_respond_to maps to respond_to in the snapshot definition
        assert_eq!(snapshot.definition.respond_to.as_deref(), Some("allowlist"));
        // definition_respond_to_allowlist should be included
        assert!(!snapshot.definition.respond_to_allowlist.is_empty());
    }

    #[test]
    fn profile_fields_present_in_snapshot() {
        let record = minimal_record();
        let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], None);
        assert_eq!(snapshot.profile.display_name, "Test Agent Display");
        // No bytes → should fall back to avatar_url
        assert_eq!(
            snapshot.profile.avatar_url.as_deref(),
            Some("https://example.com/avatar.png")
        );
        assert!(snapshot.profile.avatar_data_url.is_none());
    }

    #[test]
    fn avatar_inlined_when_under_size_limit() {
        let record = minimal_record();
        let small_png = make_png_with_text("k", "v").unwrap();
        let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], Some(&small_png));
        assert!(snapshot.profile.avatar_data_url.is_some());
        assert!(snapshot.profile.avatar_url.is_none());
    }

    #[test]
    fn avatar_url_fallback_when_over_size_limit() {
        let mut record = minimal_record();
        record.avatar_url = Some("https://example.com/big.png".to_string());
        // Synthesize oversized avatar bytes (> 2 MB) — just a large zeroed vec.
        let big_bytes = vec![0u8; MAX_AVATAR_INLINE_BYTES + 1];
        let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], Some(&big_bytes));
        assert!(snapshot.profile.avatar_data_url.is_none());
        assert_eq!(
            snapshot.profile.avatar_url.as_deref(),
            Some("https://example.com/big.png")
        );
    }

    // ── Format/version validation ─────────────────────────────────────────────

    #[test]
    fn invalid_format_discriminator_is_rejected() {
        let mut snapshot = build_snapshot(&minimal_record(), MemoryLevel::None, vec![], None);
        snapshot.format = "not-a-buzz-snapshot".to_string();
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let result = decode_snapshot_json(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported snapshot format"));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let mut snapshot = build_snapshot(&minimal_record(), MemoryLevel::None, vec![], None);
        snapshot.version = 99;
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let result = decode_snapshot_json(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported snapshot version"));
    }
}
