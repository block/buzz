//! `export_agent_snapshot` Tauri command and its supporting resolver.
//!
//! Split from `personas/mod.rs` to keep that file under the line-count gate.

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

/// Resolve the agent record to use as the definition source for a snapshot
/// export.
///
/// Search order:
///   1. Keyed instances: match `id` against `pubkey` (exact) then `slug`.
///   2. Keyless definitions: match `id` against `slug`.
///
/// Returns `(definition_record, is_definition)`.  `is_definition` is `true`
/// when the result came from the keyless-definition store — the caller must
/// not call `get_agent_memory` against it (definitions have no keypair).
pub fn resolve_snapshot_export_target(
    app: &AppHandle,
    id: &str,
) -> Result<(ManagedAgentRecord, bool), String> {
    // Try keyed instances first.
    let instances = load_managed_agents(app)?;
    if let Some(record) = instances
        .into_iter()
        .find(|a| a.pubkey == id || a.slug.as_deref() == Some(id))
    {
        return Ok((record, false));
    }

    // Fall back to keyless definitions (matched by slug only — they have no
    // pubkey).
    let definitions = load_agent_definitions(app)?;
    if let Some(record) = definitions
        .into_iter()
        .find(|a| a.slug.as_deref() == Some(id))
    {
        return Ok((record, true));
    }

    Err(format!("agent {id:?} not found"))
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
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    // ── Parse inputs ────────────────────────────────────────────────────────
    let memory_level = match memory_level.as_str() {
        "none" | "" => MemoryLevel::None,
        "core" => MemoryLevel::Core,
        "everything" => MemoryLevel::Everything,
        other => {
            return Err(format!(
                "Invalid memory_level: {other:?} (expected 'none', 'core', or 'everything')"
            ))
        }
    };

    let is_png = match format.as_str() {
        "json" | "" => false,
        "png" => true,
        other => {
            return Err(format!(
                "Invalid format: {other:?} (expected 'json' or 'png')"
            ))
        }
    };

    // Eagerly reject PNG + memory — avoid an unnecessary relay round-trip.
    if is_png && memory_level != MemoryLevel::None {
        return Err(
            "Cannot export memory to .agent.png — use JSON format for memory-bearing \
             snapshots."
                .to_string(),
        );
    }

    // ── Load definition record and memory-source instance under lock ─────────
    let (record, memory_pubkey) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;

        let (def_record, is_definition) = resolve_snapshot_export_target(&app, &id)?;

        let memory_pubkey = if memory_level != MemoryLevel::None {
            let mpk = memory_source_pubkey.as_deref().unwrap_or("").trim();
            if mpk.is_empty() {
                return Err(
                    "memory_source_pubkey is required when memory_level is not 'none'. \
                     Pass the pubkey of a linked agent instance."
                        .to_string(),
                );
            }

            if is_definition {
                // Validate the supplied pubkey is an instance linked to this
                // definition. persona_id on a keyed instance holds the definition
                // slug.
                let def_slug = def_record.slug.as_deref().unwrap_or("");
                let instances = load_managed_agents(&app)?;
                let linked = instances
                    .iter()
                    .find(|a| a.pubkey == mpk)
                    .ok_or_else(|| format!("memory_source_pubkey {mpk:?} is not a known agent"))?;
                if linked.persona_id.as_deref() != Some(def_slug) {
                    return Err(format!(
                        "memory_source_pubkey {mpk:?} is not linked to definition {id:?}"
                    ));
                }
            } else {
                // Direct keyed-instance export: pubkey must match the instance
                // itself (do not allow cross-agent memory pairing).
                if mpk != def_record.pubkey {
                    return Err(format!(
                        "memory_source_pubkey {mpk:?} does not match agent {id:?}"
                    ));
                }
            }
            Some(mpk.to_string())
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

    // ── Encode and save ──────────────────────────────────────────────────────
    let slug = crate::util::slugify(&display_name, "agent", 50);

    if is_png {
        let png_bytes = encode_snapshot_png(&snapshot, avatar_bytes.as_deref())
            .map_err(|e| format!("Failed to encode .agent.png: {e}"))?;
        let filename = format!("{slug}.agent.png");
        save_bytes_with_dialog(&app, &filename, "PNG image", &["png"], &png_bytes).await
    } else {
        let json_bytes = encode_snapshot_json(&snapshot)
            .map_err(|e| format!("Failed to encode .agent.json: {e}"))?;
        let filename = format!("{slug}.agent.json");
        save_bytes_with_dialog(&app, &filename, "Agent snapshot", &["json"], &json_bytes).await
    }
}
