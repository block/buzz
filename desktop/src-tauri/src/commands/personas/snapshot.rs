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
    if let Some(record) = definitions
        .iter()
        .find(|a| a.slug.as_deref() == Some(id))
    {
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
        // Direct instance export: pubkey must match the instance itself to
        // prevent cross-agent memory pairing.
        if mpk != def_id {
            return Err(format!(
                "memory_source_pubkey {mpk:?} does not match agent {def_id:?}"
            ));
        }
    }

    Ok(mpk.to_string())
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

        let instances = load_managed_agents(&app)?;
        let definitions = load_agent_definitions(&app)?;
        let (def_record, is_definition) =
            resolve_from_lists(&id, &instances, &definitions)
                .map(|(r, is_def)| (r.clone(), is_def))?;

        let memory_pubkey = if memory_level != MemoryLevel::None {
            let mpk = memory_source_pubkey.as_deref().unwrap_or("");
            let def_id = if is_definition {
                def_record.slug.as_deref().unwrap_or("")
            } else {
                &def_record.pubkey
            };
            Some(validate_memory_source(mpk, is_definition, def_id, &instances)?)
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



// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::{BackendKind, ManagedAgentRecord, RespondTo};
    use std::collections::BTreeMap;

    /// Build a minimal keyless definition record (matched by slug, no keypair).
    /// This is the shape stored in the definitions file — no pubkey, no persona_id.
    fn make_definition(slug: &str) -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: String::new(),
            slug: Some(slug.to_string()),
            name: slug.to_string(),
            display_name: None,
            persona_id: None,
            private_key_nsec: String::new(),
            auth_tag: None,
            relay_url: String::new(),
            avatar_url: None,
            acp_command: String::new(),
            agent_command: String::new(),
            agent_command_override: None,
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 0,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 1,
            system_prompt: None,
            model: None,
            provider: None,
            persona_source_version: None,
            mcp_toolsets: None,
            env_vars: BTreeMap::new(),
            start_on_app_launch: false,
            auto_restart_on_config_change: false,
            runtime_pid: None,
            backend: BackendKind::Local,
            backend_agent_id: None,
            provider_binary_path: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: String::new(),
            updated_at: String::new(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: RespondTo::default(),
            respond_to_allowlist: vec![],
            runtime: None,
            name_pool: vec![],
            is_builtin: false,
            is_active: false,
            source_team: None,
            source_team_persona_slug: None,
            definition_respond_to: None,
            definition_respond_to_allowlist: vec![],
            definition_mcp_toolsets: None,
            definition_parallelism: None,
            relay_mesh: None,
        }
    }

    /// Build a minimal keyed instance. Real instances minted by `create_persona`
    /// have `slug: None` and link to their definition via `persona_id`.
    fn make_instance(pubkey: &str, persona_id: &str) -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: pubkey.to_string(),
            slug: None,
            persona_id: Some(persona_id.to_string()),
            ..make_definition("")
        }
    }

    // ── Joint happy path ──────────────────────────────────────────────────────
    //
    // Production record shape: a keyless definition (slug = "my-agent") and
    // a keyed instance (slug = None, pubkey = "instance-pk", persona_id =
    // "my-agent") live in separate stores.
    //
    // This one test exercises the full resolver → validator composition:
    //   1. Resolving "my-agent" finds the *definition* (the instance has no
    //      slug so the instance search misses it).
    //   2. The linked instance pubkey validates as the memory source.

    #[test]
    fn definition_slug_resolves_to_definition_and_linked_instance_is_valid_memory_source() {
        let def = make_definition("my-agent");
        let inst = make_instance("instance-pk", "my-agent");

        let defs = vec![def];
        let instances = vec![inst];

        // Step 1 — resolution: slug finds the definition, not the instance.
        let (record, is_def) = resolve_from_lists("my-agent", &instances, &defs).unwrap();
        assert!(is_def, "slug 'my-agent' must resolve to the definition, not the instance");
        assert_eq!(record.slug.as_deref(), Some("my-agent"));

        // Step 2 — memory source validation: instance-pk is persona_id-linked.
        let def_slug = record.slug.as_deref().unwrap_or("");
        let result = validate_memory_source("instance-pk", is_def, def_slug, &instances);
        assert_eq!(
            result.unwrap(),
            "instance-pk",
            "linked keyed instance must be accepted as the memory source"
        );
    }

    // ── Resolver edge cases ───────────────────────────────────────────────────

    #[test]
    fn resolve_by_pubkey_finds_keyed_instance() {
        let inst = make_instance("pubkey-xyz", "my-agent");
        let instances = vec![inst];
        let (record, is_def) = resolve_from_lists("pubkey-xyz", &instances, &[]).unwrap();
        assert!(!is_def);
        assert_eq!(record.pubkey, "pubkey-xyz");
    }

    #[test]
    fn resolve_unknown_id_returns_error() {
        let result = resolve_from_lists("ghost", &[], &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ghost"));
    }

    // ── Validator fail-closed cases ───────────────────────────────────────────

    #[test]
    fn memory_export_without_pubkey_fails() {
        let result = validate_memory_source("", true, "my-agent", &[]);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("memory_source_pubkey is required"),
            "empty pubkey must be rejected with a clear message"
        );
    }

    #[test]
    fn definition_export_with_instance_linked_to_other_definition_fails() {
        // Instance persona_id points to "other-agent", not "my-agent".
        let inst = make_instance("instance-pk", "other-agent");
        let instances = vec![inst];
        let result = validate_memory_source("instance-pk", true, "my-agent", &instances);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("is not linked to definition"),
            "mismatched persona_id must fail closed"
        );
    }

    #[test]
    fn direct_instance_export_with_nonmatching_memory_pubkey_fails() {
        // Cross-agent memory pairing: memory pubkey differs from instance pubkey.
        let result = validate_memory_source("other-agent-pk", false, "agent-pk", &[]);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("does not match agent"),
            "cross-agent memory pairing must fail closed"
        );
    }
}
