//! Tauri commands for exporting and importing `buzz-team-snapshot v1` files.
//!
//! Team snapshots are definition-only templates: importing creates key-less
//! agent definitions and one team record. It never mints agent keys, auth tags,
//! managed-agent instances, or restores member memory.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    commands::{export_util::save_bytes_with_dialog, personas::resolve_snapshot_import_behavior},
    managed_agents::team_snapshot::{
        build_team_snapshot, decode_team_snapshot_json, decode_team_snapshot_png,
        encode_team_snapshot_json, encode_team_snapshot_png, TeamSnapshot,
    },
    managed_agents::{
        agent_snapshot::{build_snapshot, AgentSnapshot, MemoryLevel},
        load_personas, load_teams, save_personas, save_teams, AgentDefinition, TeamRecord,
    },
    util::now_iso,
};

/// Team snapshots have a combined 25 MiB JSON / 50 MiB PNG payload cap.
/// Every member is validated before any persistent write.
pub(crate) const MAX_TEAM_SNAPSHOT_JSON_BYTES: usize = 25 * 1024 * 1024;
pub(crate) const MAX_TEAM_SNAPSHOT_PNG_BYTES: usize = 50 * 1024 * 1024;

const PNG_MAGIC: [u8; 4] = [0x89, 0x50, 0x4e, 0x47];
const ZIP_MAGIC_PREFIX: [u8; 2] = [0x50, 0x4b];
const LEGACY_TEAM_ERROR: &str =
    "Legacy team files are no longer supported. Export a buzz-team-snapshot v1 .team.json or .team.png instead.";

/// Decode a canonical team snapshot, rejecting retired flat team JSON and
/// persona-pack ZIP files with a migration-oriented error.
pub(crate) fn decode_team_snapshot_from_bytes(file_bytes: &[u8]) -> Result<TeamSnapshot, String> {
    if file_bytes.starts_with(&PNG_MAGIC) {
        if file_bytes.len() > MAX_TEAM_SNAPSHOT_PNG_BYTES {
            return Err(format!(
                "Team snapshot file is too large ({} MiB). PNG snapshots must be under 50 MiB.",
                file_bytes.len() / (1024 * 1024)
            ));
        }
        return decode_team_snapshot_png(file_bytes);
    }

    if file_bytes.len() > MAX_TEAM_SNAPSHOT_JSON_BYTES {
        return Err(format!(
            "Team snapshot file is too large ({} MiB). JSON snapshots must be under 25 MiB.",
            file_bytes.len() / (1024 * 1024)
        ));
    }

    // Detect the retired schema before attempting canonical deserialization so
    // old `.team.json` attachments never look like malformed new snapshots.
    let value: serde_json::Value = match serde_json::from_slice(file_bytes) {
        Ok(value) => value,
        Err(_) if file_bytes.starts_with(&ZIP_MAGIC_PREFIX) => {
            return Err(LEGACY_TEAM_ERROR.to_string());
        }
        Err(error) => return Err(format!("Invalid team snapshot JSON: {error}")),
    };
    if value.get("format").and_then(serde_json::Value::as_str)
        != Some(crate::managed_agents::team_snapshot::FORMAT_DISCRIMINATOR)
        && value.get("version").and_then(serde_json::Value::as_u64) == Some(1)
        && value.get("type").and_then(serde_json::Value::as_str) == Some("team")
    {
        return Err(LEGACY_TEAM_ERROR.to_string());
    }

    decode_team_snapshot_json(file_bytes)
}

fn parse_format_is_png(format: &str) -> Result<bool, String> {
    match format {
        "json" | "" => Ok(false),
        "png" => Ok(true),
        other => Err(format!(
            "Invalid format: {other:?} (expected 'json' or 'png')"
        )),
    }
}

fn effective_avatar(member: &AgentSnapshot) -> Option<String> {
    member
        .profile
        .avatar_data_url
        .clone()
        .or_else(|| member.profile.avatar_url.clone())
}

/// Build a definition from a team member snapshot without consuming its memory.
/// The ignored `memory` field is deliberate: definition-only imports have no
/// owner-to-agent key material or live instance to which memory could belong.
fn definition_from_snapshot(
    member: &AgentSnapshot,
    keep_allowlist: bool,
    now: &str,
) -> Result<AgentDefinition, String> {
    let behavior = resolve_snapshot_import_behavior(
        member.definition.respond_to.as_deref(),
        &member.definition.respond_to_allowlist,
        member.definition.parallelism,
        keep_allowlist,
    )?;
    let respond_to = (behavior.respond_to != crate::managed_agents::RespondTo::default())
        .then(|| behavior.respond_to.as_str().to_string());

    Ok(AgentDefinition {
        id: Uuid::new_v4().to_string(),
        display_name: member.profile.display_name.trim().to_string(),
        avatar_url: effective_avatar(member),
        system_prompt: member.definition.system_prompt.clone().unwrap_or_default(),
        runtime: member.definition.runtime.clone(),
        model: member.definition.model.clone(),
        provider: member.definition.provider.clone(),
        name_pool: member.definition.name_pool.clone(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: Default::default(),
        respond_to,
        respond_to_allowlist: behavior.respond_to_allowlist,
        parallelism: behavior.parallelism,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    })
}

fn build_import_definitions(
    snapshot: &TeamSnapshot,
    keep_allowlist: bool,
    now: &str,
) -> Result<Vec<AgentDefinition>, String> {
    snapshot
        .members
        .iter()
        .map(|member| definition_from_snapshot(member, keep_allowlist, now))
        .collect()
}

/// Assemble the one new team record that references freshly built definitions.
/// Keeping this pure lets tests verify the definition-only import shape without
/// creating an `AppHandle` or touching the on-disk stores.
fn build_import_team(
    snapshot: &TeamSnapshot,
    persona_ids: Vec<String>,
    now: &str,
) -> Result<TeamRecord, String> {
    let name = snapshot.team.name.trim();
    if name.is_empty() {
        return Err("Team snapshot name is empty.".to_string());
    }

    Ok(TeamRecord {
        id: Uuid::new_v4().to_string(),
        name: name.to_string(),
        description: snapshot.team.description.clone(),
        persona_ids,
        is_builtin: false,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    })
}

fn member_preview(member: &AgentSnapshot) -> TeamSnapshotMemberPreview {
    TeamSnapshotMemberPreview {
        display_name: member.profile.display_name.clone(),
        system_prompt: member.definition.system_prompt.clone(),
        avatar_url: effective_avatar(member),
        has_source_allowlist: !member.definition.respond_to_allowlist.is_empty(),
        source_allowlist_count: member.definition.respond_to_allowlist.len(),
    }
}

/// Preview metadata for one definition that will be imported with a team.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamSnapshotMemberPreview {
    pub display_name: String,
    pub system_prompt: Option<String>,
    pub avatar_url: Option<String>,
    pub has_source_allowlist: bool,
    pub source_allowlist_count: usize,
}

/// Materialized team snapshot preview. No write happens before confirmation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamSnapshotImportPreview {
    pub name: String,
    pub description: Option<String>,
    pub members: Vec<TeamSnapshotMemberPreview>,
    pub has_source_allowlist: bool,
}

/// Confirmation input for a definition-only team snapshot import.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamSnapshotImportConfirm {
    pub file_bytes: Vec<u8>,
    /// Applied uniformly to every member in v1.
    pub keep_allowlist: bool,
}

/// Result of a definition-only team snapshot import.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamSnapshotImportResult {
    pub team: TeamRecord,
    pub persona_ids: Vec<String>,
}

/// In-memory bytes for the native team sharing flow.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodedTeamSnapshotPayload {
    pub file_bytes: Vec<u8>,
    pub file_name: String,
}

fn build_team_export_snapshot(
    team: &TeamRecord,
    personas: &[AgentDefinition],
) -> Result<TeamSnapshot, String> {
    let members = team
        .persona_ids
        .iter()
        .map(|id| {
            let persona = personas
                .iter()
                .find(|persona| persona.id == *id)
                .ok_or_else(|| {
                    format!("team {} references missing agent definition {id}", team.id)
                })?;
            Ok(build_snapshot(
                &persona.clone().into_agent_record(),
                MemoryLevel::None,
                Vec::new(),
                None,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(build_team_snapshot(team, members))
}

async fn materialize_team_snapshot_bytes(
    id: String,
    is_png: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<EncodedTeamSnapshotPayload, String> {
    let (team, personas) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let team = load_teams(&app)?
            .into_iter()
            .find(|team| team.id == id)
            .ok_or_else(|| format!("team {id} not found"))?;
        (team, load_personas(&app)?)
    };

    let snapshot = build_team_export_snapshot(&team, &personas)?;
    let slug = crate::util::slugify(&team.name, "team", 50);
    let (file_bytes, file_name) = if is_png {
        let bytes = encode_team_snapshot_png(&snapshot)
            .map_err(|e| format!("Failed to encode .team.png: {e}"))?;
        if bytes.len() > MAX_TEAM_SNAPSHOT_PNG_BYTES {
            return Err(
                "Team snapshot exceeds the 50 MiB size limit for .team.png files.".to_string(),
            );
        }
        (bytes, format!("{slug}.team.png"))
    } else {
        let bytes = encode_team_snapshot_json(&snapshot)
            .map_err(|e| format!("Failed to encode .team.json: {e}"))?;
        if bytes.len() > MAX_TEAM_SNAPSHOT_JSON_BYTES {
            return Err(
                "Team snapshot exceeds the 25 MiB size limit for .team.json files.".to_string(),
            );
        }
        (bytes, format!("{slug}.team.json"))
    };
    Ok(EncodedTeamSnapshotPayload {
        file_bytes,
        file_name,
    })
}

/// Export a team template with each member's memory explicitly set to `none`.
#[tauri::command]
pub async fn export_team_snapshot(
    id: String,
    format: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let is_png = parse_format_is_png(&format)?;
    let payload = materialize_team_snapshot_bytes(id, is_png, app.clone(), state).await?;
    if is_png {
        save_bytes_with_dialog(
            &app,
            &payload.file_name,
            "PNG image",
            &["png"],
            &payload.file_bytes,
        )
        .await
    } else {
        save_bytes_with_dialog(
            &app,
            &payload.file_name,
            "Team snapshot",
            &["json"],
            &payload.file_bytes,
        )
        .await
    }
}

/// Encode a team template for the native send flow without opening a dialog.
#[tauri::command]
pub async fn encode_team_snapshot_for_send(
    id: String,
    format: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<EncodedTeamSnapshotPayload, String> {
    materialize_team_snapshot_bytes(id, parse_format_is_png(&format)?, app, state).await
}

/// Decode a team snapshot into a confirmation preview without writing anything.
#[tauri::command]
pub async fn preview_team_snapshot_import(
    file_bytes: Vec<u8>,
    _file_name: String,
) -> Result<TeamSnapshotImportPreview, String> {
    tokio::task::spawn_blocking(move || {
        let snapshot = decode_team_snapshot_from_bytes(&file_bytes)?;
        let members: Vec<_> = snapshot.members.iter().map(member_preview).collect();
        Ok(TeamSnapshotImportPreview {
            name: snapshot.team.name,
            description: snapshot.team.description,
            has_source_allowlist: members.iter().any(|member| member.has_source_allowlist),
            members,
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Import a team snapshot as key-less agent definitions plus one team record.
/// No keypair, NIP-OA auth tag, managed-agent instance, or memory entry is
/// created; member memory is intentionally inert template data.
#[tauri::command]
pub async fn confirm_team_snapshot_import(
    input: TeamSnapshotImportConfirm,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TeamSnapshotImportResult, String> {
    let snapshot = decode_team_snapshot_from_bytes(&input.file_bytes)?;
    let now = now_iso();
    // Resolve every member before locking or writing so an invalid allowlist
    // cannot leave a partially imported team behind.
    let definitions = build_import_definitions(&snapshot, input.keep_allowlist, &now)?;
    let persona_ids: Vec<String> = definitions
        .iter()
        .map(|definition| definition.id.clone())
        .collect();
    let imported_team = build_import_team(&snapshot, persona_ids.clone(), &now)?;

    let team = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut personas = load_personas(&app)?;
        personas.extend(definitions.iter().cloned());
        save_personas(&app, &personas)?;
        for definition in &definitions {
            crate::commands::personas::retain_persona_pending(&app, &state, definition);
        }

        let mut teams = load_teams(&app)?;
        teams.push(imported_team.clone());
        let team = imported_team;
        save_teams(&app, &teams)?;
        crate::commands::teams::retain_team_pending(&app, &state, &team);
        crate::managed_agents::try_regenerate_nest(&app);
        let _ = app.emit("agents-data-changed", ());
        team
    };

    Ok(TeamSnapshotImportResult { team, persona_ids })
}

#[cfg(test)]
mod tests;
