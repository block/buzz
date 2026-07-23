//! `export_agent_snapshot` / `encode_agent_snapshot_for_send` Tauri commands
//! and their supporting helpers.
//!
//! Import-side commands and helpers live in `snapshot::import` to keep this
//! file under the 1000-line gate.
//!
//! Split from `personas/mod.rs` to keep that file under the line-count gate.

use serde::Serialize;
use tauri::{AppHandle, State};

use super::super::export_util::save_bytes_with_dialog;
use crate::{
    app_state::AppState,
    commands::engrams::get_agent_memory,
    managed_agents::{
        agent_snapshot::{
            build_snapshot, encode_snapshot_json, encode_snapshot_png, AgentSnapshotMemoryEntry,
            MemoryLevel,
        },
        load_agent_definitions, load_managed_agents, ManagedAgentRecord,
    },
};

pub(crate) mod import;

// Re-export import-side commands so callers see a flat `snapshot::` namespace.
pub use import::{confirm_agent_snapshot_import, preview_agent_snapshot_import};

// ── Pure resolver (testable without AppHandle) ────────────────────────────────

/// Inner resolver operating on pre-fetched slices — testable without
/// `AppHandle`.
///
/// Search order:
///   1. Keyed instances: match `id` against `pubkey` (exact) then `slug`.
///   2. Keyless definitions: match `id` against `slug`.
///
/// Returns `(definition_record, is_definition)`.  `is_definition` is `true`
/// when the result came from the definitions slice — the caller must not call
/// `get_agent_memory` against it (definitions have no keypair).
pub(crate) fn resolve_from_lists<'a>(
    id: &str,
    instances: &'a [ManagedAgentRecord],
    definitions: &'a [ManagedAgentRecord],
) -> Result<(&'a ManagedAgentRecord, bool), String> {
    if let Some(record) = instances
        .iter()
        .find(|a| a.pubkey == id || a.slug.as_deref() == Some(id))
    {
        return Ok((record, false));
    }
    if let Some(record) = definitions.iter().find(|a| a.slug.as_deref() == Some(id)) {
        return Ok((record, true));
    }
    Err(format!("agent {id:?} not found"))
}

/// Validate that `memory_source_pubkey` is an appropriate source for a
/// memory-bearing snapshot export.
///
/// For definition exports (`is_definition == true`), the instance must be
/// known and its `persona_id` must equal `def_slug`.
/// For direct instance exports, the pubkey must match the instance itself.
///
/// Returns the validated pubkey string on success.
pub(crate) fn validate_memory_source(
    memory_source_pubkey: &str,
    is_definition: bool,
    def_id: &str,
    instances: &[ManagedAgentRecord],
) -> Result<String, String> {
    let mpk = memory_source_pubkey.trim();
    if mpk.is_empty() {
        return Err(
            "memory_source_pubkey is required when memory_level is not 'none'. \
             Pass the pubkey of a linked agent instance."
                .to_string(),
        );
    }

    if is_definition {
        // Definition export: the supplied pubkey must be a keyed instance
        // whose persona_id equals the definition slug.
        let linked = instances
            .iter()
            .find(|a| a.pubkey == mpk)
            .ok_or_else(|| format!("memory_source_pubkey {mpk:?} is not a known agent"))?;
        if linked.persona_id.as_deref() != Some(def_id) {
            return Err(format!(
                "memory_source_pubkey {mpk:?} is not linked to definition {def_id:?}"
            ));
        }
    } else {
        // Instance export: the pubkey must match the instance itself.
        // This prevents cross-agent memory pairing.
        if mpk != def_id {
            return Err(format!(
                "memory_source_pubkey {mpk:?} does not match agent {def_id:?}"
            ));
        }
    }

    Ok(mpk.to_string())
}

/// The encoded bytes and suggested filename for a snapshot, produced by the
/// shared materialization path. Used by both the save-to-disk command and the
/// native-send command so both paths are byte-identical for the same inputs.
pub(crate) struct SnapshotPayload {
    pub bytes: Vec<u8>,
    pub filename: String,
}

/// Enforce the output size ceiling after encoding.
///
/// Called by `materialize_snapshot_bytes` before returning bytes to the caller.
/// Both the save-to-disk and send commands therefore reject an oversized result
/// before any file dialog or upload, ensuring that a successfully sent/saved
/// snapshot is always within the importer's acceptance bounds.
///
/// Extracted as a pure named function so tests can prove the guard logic
/// directly (boundary-1 / boundary / boundary+1 for both formats) without
/// constructing a full `AppHandle`/`AppState`.
pub(crate) fn validate_snapshot_encode_size(bytes_len: usize, is_png: bool) -> Result<(), String> {
    if is_png {
        if bytes_len > import::MAX_SNAPSHOT_PNG_BYTES {
            return Err(format!(
                "Snapshot exceeds the {} MiB size limit for .agent.png files. \
                 Reduce the avatar image size or use JSON format.",
                import::MAX_SNAPSHOT_PNG_BYTES / (1024 * 1024)
            ));
        }
    } else if bytes_len > import::MAX_SNAPSHOT_JSON_BYTES {
        return Err(format!(
            "Snapshot exceeds the {} MiB size limit for .agent.json files. \
             Reduce memory size or use a config-only snapshot.",
            import::MAX_SNAPSHOT_JSON_BYTES / (1024 * 1024)
        ));
    }
    Ok(())
}

/// Parse a `memory_level` string to `MemoryLevel`.
fn parse_memory_level(s: &str) -> Result<MemoryLevel, String> {
    match s {
        "none" | "" => Ok(MemoryLevel::None),
        "core" => Ok(MemoryLevel::Core),
        "everything" => Ok(MemoryLevel::Everything),
        other => Err(format!(
            "Invalid memory_level: {other:?} (expected 'none', 'core', or 'everything')"
        )),
    }
}

/// Parse a `format` string to a PNG flag.
fn parse_format_is_png(s: &str) -> Result<bool, String> {
    match s {
        "json" | "" => Ok(false),
        "png" => Ok(true),
        other => Err(format!(
            "Invalid format: {other:?} (expected 'json' or 'png')"
        )),
    }
}

/// Shared production encoding path.
///
/// Resolves the agent definition, validates inputs, fetches optional memory,
/// builds the snapshot manifest, and encodes to the requested byte format.
/// Does **not** open any file dialog or write to disk — both the save-to-disk
/// and native-send commands call this and then apply their own I/O side effect.
///
/// **Invariants preserved (identical for both callers):**
/// - JSON/PNG format selection from the parsed `is_png` flag (magic-byte sniffing is import-only)
/// - Memory-source pubkey validation
/// - Secret exclusion (env_vars never enter the manifest via `build_snapshot`)
/// - Output filename derived from the agent display name
pub(crate) async fn materialize_snapshot_bytes(
    id: String,
    memory_source_pubkey: Option<String>,
    memory_level: MemoryLevel,
    is_png: bool,
    avatar_png_data_url: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SnapshotPayload, String> {
    // ── Load definition record and memory-source instance under lock ─────────
    let (record, memory_pubkey) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;

        let instances = load_managed_agents(&app)?;
        let definitions = load_agent_definitions(&app)?;
        let (def_record, is_definition) = resolve_from_lists(&id, &instances, &definitions)
            .map(|(r, is_def)| (r.clone(), is_def))?;

        let memory_pubkey = if memory_level != MemoryLevel::None {
            let mpk = memory_source_pubkey.as_deref().unwrap_or("");
            let def_id = if is_definition {
                def_record.slug.as_deref().unwrap_or("")
            } else {
                &def_record.pubkey
            };
            Some(validate_memory_source(
                mpk,
                is_definition,
                def_id,
                &instances,
            )?)
        } else {
            None
        };

        (def_record, memory_pubkey)
    };

    let display_name = record
        .display_name
        .clone()
        .unwrap_or_else(|| record.name.clone());

    // ── Resolve avatar bytes ─────────────────────────────────────────────────
    // If the avatar_url is a data URL we decode it inline; otherwise we keep
    // it as an external reference in the manifest (the importer will use it).
    let avatar_bytes: Option<Vec<u8>> = record
        .avatar_url
        .as_deref()
        .and_then(crate::managed_agents::agent_snapshot::decode_avatar_data_url);

    // ── Fetch memory ─────────────────────────────────────────────────────────
    let memory_entries: Vec<AgentSnapshotMemoryEntry> = if let Some(pubkey) = memory_pubkey {
        let listing = get_agent_memory(pubkey, app.clone(), state).await?;
        let mut entries = Vec::new();
        if let Some(core) = listing.core {
            entries.push(AgentSnapshotMemoryEntry {
                slug: core.slug,
                body: core.body,
            });
        }
        if memory_level == MemoryLevel::Everything {
            for mem in listing.memories {
                entries.push(AgentSnapshotMemoryEntry {
                    slug: mem.slug,
                    body: mem.body,
                });
            }
        }
        entries
    } else {
        Vec::new()
    };

    // ── Build manifest ───────────────────────────────────────────────────────
    let snapshot = build_snapshot(
        &record,
        memory_level,
        memory_entries,
        avatar_bytes.as_deref(),
    );

    // ── Encode ───────────────────────────────────────────────────────────────
    let slug = crate::util::slugify(&display_name, "agent", 50);

    if is_png {
        let png_body_avatar_bytes =
            resolve_png_body_avatar_bytes(avatar_png_data_url.as_deref(), avatar_bytes);
        let png_bytes = encode_snapshot_png(&snapshot, png_body_avatar_bytes.as_deref())
            .map_err(|e| format!("Failed to encode .agent.png: {e}"))?;
        validate_snapshot_encode_size(png_bytes.len(), true)?;
        Ok(SnapshotPayload {
            bytes: png_bytes,
            filename: format!("{slug}.agent.png"),
        })
    } else {
        let json_bytes = encode_snapshot_json(&snapshot)
            .map_err(|e| format!("Failed to encode .agent.json: {e}"))?;
        validate_snapshot_encode_size(json_bytes.len(), false)?;
        Ok(SnapshotPayload {
            bytes: json_bytes,
            filename: format!("{slug}.agent.json"),
        })
    }
}

/// Choose bytes for the PNG image body without changing the source avatar the
/// manifest preserves for import.
fn resolve_png_body_avatar_bytes(
    avatar_png_data_url: Option<&str>,
    store_avatar_bytes: Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    avatar_png_data_url
        .and_then(crate::managed_agents::agent_snapshot::decode_avatar_data_url)
        .or(store_avatar_bytes)
}

/// Export an agent definition as a `buzz-agent-snapshot v1` file.
///
/// `id` is a definition slug or a keyed-instance pubkey.
/// `memory_source_pubkey` is required when `memory_level != "none"` — it must
/// be a keyed-instance pubkey whose `persona_id` matches `id` (validated
/// server-side so the UI cannot supply a mismatched pairing).
/// `memory_level` is one of `"none"`, `"core"`, or `"everything"`.
/// `format` is either `"json"` or `"png"`.
///
/// The user picks the save path via the OS dialog. Returns `true` when the
/// file was written, `false` when the dialog was cancelled.
#[tauri::command]
pub async fn export_agent_snapshot(
    id: String,
    memory_source_pubkey: Option<String>,
    memory_level: String,
    format: String,
    avatar_png_data_url: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let memory_level = parse_memory_level(&memory_level)?;
    let is_png = parse_format_is_png(&format)?;

    let payload = materialize_snapshot_bytes(
        id,
        memory_source_pubkey,
        memory_level,
        is_png,
        avatar_png_data_url,
        app.clone(),
        state,
    )
    .await?;

    if is_png {
        save_bytes_with_dialog(
            &app,
            &payload.filename,
            "PNG image",
            &["png"],
            &payload.bytes,
        )
        .await
    } else {
        save_bytes_with_dialog(
            &app,
            &payload.filename,
            "Agent snapshot",
            &["json"],
            &payload.bytes,
        )
        .await
    }
}

/// Wire shape returned by `encode_agent_snapshot_for_send`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodedSnapshotPayload {
    /// Raw snapshot bytes as a byte array. The frontend passes these directly
    /// to `uploadMediaBytes` (which accepts `number[]`).
    pub file_bytes: Vec<u8>,
    /// Suggested filename, e.g. `my-agent.agent.json`.
    pub file_name: String,
}

/// Encode a `buzz-agent-snapshot v1` payload in memory and return the raw
/// bytes to the frontend for the native-send path.
///
/// Performs identical resolution, validation, and encoding as
/// `export_agent_snapshot` (via the shared `materialize_snapshot_bytes` path)
/// but **never opens a file dialog**. The frontend passes the returned bytes
/// through `uploadMediaBytes` → message construction → channel/DM send.
#[tauri::command]
pub async fn encode_agent_snapshot_for_send(
    id: String,
    memory_source_pubkey: Option<String>,
    memory_level: String,
    format: String,
    avatar_png_data_url: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<EncodedSnapshotPayload, String> {
    let memory_level = parse_memory_level(&memory_level)?;
    let is_png = parse_format_is_png(&format)?;

    let payload = materialize_snapshot_bytes(
        id,
        memory_source_pubkey,
        memory_level,
        is_png,
        avatar_png_data_url,
        app,
        state,
    )
    .await?;

    Ok(EncodedSnapshotPayload {
        file_bytes: payload.bytes,
        file_name: payload.filename,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod png_body_tests {
    use super::*;
    use base64::Engine as _;
    use png::Decoder;

    #[test]
    fn frontend_raster_becomes_png_body_without_replacing_manifest_source() {
        let avatar = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            3,
            2,
            image::Rgb([0x12, 0x34, 0x56]),
        ));
        let mut source_png = Vec::new();
        avatar
            .write_to(
                &mut std::io::Cursor::new(&mut source_png),
                image::ImageFormat::Png,
            )
            .unwrap();
        let avatar_data_url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&source_png)
        );
        let snapshot = crate::managed_agents::agent_snapshot::AgentSnapshot {
            format: crate::managed_agents::agent_snapshot::FORMAT_DISCRIMINATOR.to_string(),
            version: crate::managed_agents::agent_snapshot::FORMAT_VERSION,
            definition: crate::managed_agents::agent_snapshot::AgentSnapshotDefinition {
                name: "Agent".to_string(),
                system_prompt: None,
                runtime: None,
                model: None,
                provider: None,
                parallelism: None,
                respond_to: None,
                respond_to_allowlist: vec![],
                name_pool: vec![],
                idle_timeout_seconds: None,
                max_turn_duration_seconds: None,
            },
            profile: crate::managed_agents::agent_snapshot::AgentSnapshotProfile {
                display_name: "Agent".to_string(),
                about: None,
                avatar_data_url: None,
                avatar_url: Some("https://relay.example/media/avatar.png".to_string()),
            },
            memory: crate::managed_agents::agent_snapshot::AgentSnapshotMemory {
                level: MemoryLevel::None,
                entries: vec![],
            },
        };
        let body_bytes = resolve_png_body_avatar_bytes(Some(&avatar_data_url), None);
        let png_bytes = encode_snapshot_png(&snapshot, body_bytes.as_deref()).unwrap();
        let reader = Decoder::new(std::io::Cursor::new(&png_bytes))
            .read_info()
            .unwrap();
        let decoded = import::decode_snapshot_from_bytes(&png_bytes).unwrap();

        assert_eq!((reader.info().width, reader.info().height), (3, 2));
        assert_eq!(decoded.profile.avatar_url, snapshot.profile.avatar_url);
    }
}
