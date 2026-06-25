use tauri::{AppHandle, State};
use uuid::Uuid;

use super::export_util::save_json_with_dialog;
use crate::{
    app_state::AppState,
    managed_agents::{
        delete_team_with_cascade, encode_team_json, ensure_persona_ids_are_active,
        import_team_from_directory as do_import_team, load_personas, load_teams, parse_team_json,
        save_teams, sync_team_from_dir as do_sync_team, try_regenerate_nest, CreateTeamRequest,
        ParsedTeamPreview, SyncResult, TeamRecord, UpdateTeamRequest,
    },
    util::now_iso,
};

fn trim_required(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is required"));
    }
    Ok(trimmed.to_string())
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value.and_then(|candidate| {
        let trimmed = candidate.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

/// Retain a freshly authored team event in the local store, flagged for relay
/// sync. Called inside a command's `managed_agents_store_lock`-held body after
/// `save_teams`; the background flush loop publishes it out-of-band.
///
/// Mirrors `commands::personas::retain_persona_pending`. Built-in teams are not
/// owner-authored, so the caller skips them — this helper assumes the team is
/// publishable. Best-effort: a failure here is logged and swallowed so a
/// retention hiccup never blocks the disk-authoritative write.
///
/// Unlike `retain_managed_agent_pending`, this has no projection-equality
/// short-circuit: teams have no start/stop runtime churn, so a republish only
/// happens on an actual user edit. The guard is intentionally omitted.
fn retain_team_pending(app: &AppHandle, state: &AppState, team: &TeamRecord) {
    use crate::managed_agents::{
        managed_agents_base_dir,
        persona_events::monotonic_created_at,
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
        team_events::build_team_event,
    };
    use buzz_core_pkg::kind::KIND_TEAM;
    use nostr::JsonUtil;

    let result = (|| -> Result<(), String> {
        let conn = open_retention_db(&managed_agents_base_dir(app)?.join("retention.db"))?;
        let (pubkey, event) = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            let pubkey = keys.public_key().to_hex();
            // Monotonic created_at: bump past the retained head (NIP-AP step 3).
            let prior =
                get_retained_event(&conn, KIND_TEAM, &pubkey, &team.id)?.map(|row| row.created_at);
            let event = build_team_event(team)?
                .custom_created_at(monotonic_created_at(prior))
                .sign_with_keys(&keys)
                .map_err(|e| format!("failed to sign team event: {e}"))?;
            (pubkey, event)
        };
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_TEAM,
                pubkey,
                d_tag: team.id.clone(),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: team-retain: {e}");
    }
}

/// Purge a deleted team's pending row and enqueue a NIP-09 tombstone, both
/// inside the `managed_agents_store_lock`-held delete body.
///
/// Mirrors `commands::personas::tombstone_persona_pending`: the team row is
/// purged first so an unpublished edit can never resurrect it after the
/// tombstone publishes, then the kind:5 tombstone is retained at its own
/// `(5, pubkey, d_tag)` coordinate with `pending_sync = 1`. Best-effort: a
/// failure is logged and swallowed so a retention hiccup never blocks the
/// disk-authoritative delete.
fn tombstone_team_pending(app: &AppHandle, state: &AppState, d_tag: &str) {
    use crate::managed_agents::{
        managed_agents_base_dir,
        retention::{
            delete_retained_event, open_retention_db, retain_event, tombstone_retention_d_tag,
            RetainedEvent,
        },
        team_events::build_team_delete,
    };
    use buzz_core_pkg::kind::KIND_TEAM;
    use nostr::JsonUtil;

    const KIND_DELETE: u32 = 5;

    let result = (|| -> Result<(), String> {
        let (pubkey, event) = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            let pubkey = keys.public_key().to_hex();
            let event = build_team_delete(d_tag, &pubkey)?
                .sign_with_keys(&keys)
                .map_err(|e| format!("failed to sign team tombstone: {e}"))?;
            (pubkey, event)
        };
        let conn = open_retention_db(&managed_agents_base_dir(app)?.join("retention.db"))?;
        delete_retained_event(&conn, KIND_TEAM, &pubkey, d_tag)?;
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_DELETE,
                pubkey,
                // Key by the target coordinate so cross-kind d-tag tombstones
                // occupy distinct rows (F2c).
                d_tag: tombstone_retention_d_tag(KIND_TEAM, d_tag),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: team-tombstone: {e}");
    }
}

#[tauri::command]
pub fn list_teams(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<TeamRecord>, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    load_teams(&app)
}

#[tauri::command]
pub fn create_team(
    input: CreateTeamRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TeamRecord, String> {
    let name = trim_required(&input.name, "Team name")?;
    let description = trim_optional(input.description);
    let now = now_iso();

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let personas = load_personas(&app)?;
    ensure_persona_ids_are_active(&personas, &input.persona_ids)?;
    let mut teams = load_teams(&app)?;
    let team = TeamRecord {
        id: Uuid::new_v4().to_string(),
        name,
        description,
        persona_ids: input.persona_ids,
        is_builtin: false,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: now.clone(),
        updated_at: now,
    };
    teams.push(team.clone());
    save_teams(&app, &teams)?;
    // Created teams are always non-builtin; publish to the relay.
    retain_team_pending(&app, &state, &team);
    Ok(team)
}

#[tauri::command]
pub fn update_team(
    input: UpdateTeamRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TeamRecord, String> {
    let name = trim_required(&input.name, "Team name")?;
    let description = trim_optional(input.description);

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let personas = load_personas(&app)?;
    ensure_persona_ids_are_active(&personas, &input.persona_ids)?;
    let mut teams = load_teams(&app)?;
    let team = teams
        .iter_mut()
        .find(|record| record.id == input.id)
        .ok_or_else(|| format!("team {} not found", input.id))?;

    team.name = name;
    team.description = description;
    team.persona_ids = input.persona_ids;
    team.updated_at = now_iso();

    let updated = team.clone();
    save_teams(&app, &teams)?;
    // Built-in teams are not owner-authored — never publish them.
    if !updated.is_builtin {
        retain_team_pending(&app, &state, &updated);
    }
    Ok(updated)
}

#[tauri::command]
pub fn delete_team(id: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let cascaded_persona_d_tags = delete_team_with_cascade(&app, &id)?;
    // delete_team_with_cascade rejects built-in teams via validate_team_deletion,
    // so reaching here means this team was owner-published — tombstone it. The
    // d_tag is the team id, captured before the record left the store.
    tombstone_team_pending(&app, &state, &id);
    // Tombstone the cascaded personas too, so their orphaned kind:30175 heads
    // don't linger on the relay (F4). Each d-tag was captured pre-removal.
    for persona_d_tag in &cascaded_persona_d_tags {
        super::personas::tombstone_persona_pending(&app, &state, persona_d_tag);
    }
    try_regenerate_nest(&app);
    Ok(())
}

#[tauri::command]
pub fn install_team_from_directory(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    symlink: Option<bool>,
) -> Result<TeamRecord, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let source = std::path::PathBuf::from(&path);
    if !source.is_dir() {
        return Err(format!("team path is not a directory: {path}"));
    }
    let result = do_import_team(&app, &source, symlink.unwrap_or(false))?;
    try_regenerate_nest(&app);
    Ok(result)
}

#[tauri::command]
pub fn sync_team_directory(
    app: AppHandle,
    state: State<'_, AppState>,
    team_id: String,
) -> Result<SyncResult, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let result = do_sync_team(&app, &team_id)?;
    try_regenerate_nest(&app);
    Ok(result)
}

#[tauri::command]
pub async fn pick_team_directory(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app.dialog().file().blocking_pick_folder();
    Ok(path.map(|p| p.to_string()))
}

const MAX_TEAM_JSON_BYTES: usize = 5 * 1024 * 1024;

#[tauri::command]
pub async fn export_team_to_json(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    // Load team and personas under lock, then drop lock before dialog.
    let (team, personas) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let teams = load_teams(&app)?;
        let team = teams
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| format!("team {id} not found"))?;
        let personas = load_personas(&app)?;
        (team, personas)
    };

    let json_bytes = encode_team_json(&team, &personas)?;

    let slug = crate::util::slugify(&team.name, "team", 50);
    let filename = format!("{slug}.team.json");
    save_json_with_dialog(&app, &filename, &json_bytes).await
}

/// Max zip size for pack imports via team import (100 MB, same as persona zip limit).
const MAX_TEAM_ZIP_BYTES: usize = 100 * 1024 * 1024;

#[tauri::command]
pub fn parse_team_file(
    file_bytes: Vec<u8>,
    _file_name: String,
) -> Result<ParsedTeamPreview, String> {
    if file_bytes.is_empty() {
        return Err("File is empty.".to_string());
    }

    // Detect zip files (persona packs) BEFORE the JSON size check — zips can be larger.
    if file_bytes.len() >= 4 && file_bytes[..4] == [0x50, 0x4B, 0x03, 0x04] {
        if file_bytes.len() > MAX_TEAM_ZIP_BYTES {
            return Err("ZIP file is too large (max 100 MB).".to_string());
        }
        return parse_team_from_pack_zip(&file_bytes);
    }

    if file_bytes.len() > MAX_TEAM_JSON_BYTES {
        return Err("File is too large (max 5 MB).".to_string());
    }

    parse_team_json(&file_bytes)
}

/// Parse a persona pack zip as a team: pack name → team name, personas → members.
fn parse_team_from_pack_zip(zip_bytes: &[u8]) -> Result<ParsedTeamPreview, String> {
    use crate::managed_agents::TeamPersonaPreview;

    // Extract to tempdir and resolve the pack directly — gives us structured
    // access to pack name without parsing formatted strings.
    let tmp = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {e}"))?;
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Invalid ZIP archive: {e}"))?;

    let max_decompressed: usize = 100 * 1024 * 1024; // 100 MB
    let mut total_decompressed: usize = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry: {e}"))?;
        let safe_name = match entry.enclosed_name() {
            Some(name) => name.to_path_buf(),
            None => continue,
        };
        let out_path = tmp.path().join(&safe_name);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| format!("Failed to create dir: {e}"))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent dir: {e}"))?;
            }
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut data)
                .map_err(|e| format!("Read error: {e}"))?;
            total_decompressed += data.len();
            if total_decompressed > max_decompressed {
                return Err("ZIP decompressed content exceeds 100MB limit".to_string());
            }
            std::fs::write(&out_path, &data).map_err(|e| format!("Write error: {e}"))?;
        }
    }

    let pack_root = crate::managed_agents::find_plugin_json(tmp.path())
        .ok_or_else(|| "No .plugin/plugin.json found in ZIP".to_string())?;
    let resolved = buzz_persona_pkg::resolve::resolve_pack(&pack_root)
        .map_err(|e| format!("Pack validation failed: {e}"))?;

    if resolved.personas.is_empty() {
        return Err("Pack contains no personas.".to_string());
    }

    Ok(ParsedTeamPreview {
        name: resolved.name,
        description: if resolved.description.is_empty() {
            None
        } else {
            Some(resolved.description)
        },
        personas: resolved
            .personas
            .into_iter()
            .map(|p| TeamPersonaPreview {
                display_name: p.display_name,
                system_prompt: p.system_prompt,
                avatar_url: None,
            })
            .collect(),
    })
}
