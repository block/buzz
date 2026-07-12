use std::{fs, path::PathBuf};

use tauri::AppHandle;

use crate::{
    managed_agents::{managed_agents_base_dir, AgentDefinition, TeamRecord},
    util::now_iso,
};

use super::team_repair::team_persona_key;

#[derive(Debug, Clone, serde::Serialize)]
pub struct TeamPersonaPreview {
    pub display_name: String,
    pub system_prompt: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ParsedTeamPreview {
    pub name: String,
    pub description: Option<String>,
    pub personas: Vec<TeamPersonaPreview>,
}

fn teams_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("teams.json"))
}

fn sort_teams(records: &mut [TeamRecord]) {
    records.sort_by(|left, right| {
        let left_builtin = if left.is_builtin { 0 } else { 1 };
        let right_builtin = if right.is_builtin { 0 } else { 1 };
        left_builtin
            .cmp(&right_builtin)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.id.cmp(&right.id))
    });
}

struct BuiltInTeam {
    id: &'static str,
    name: &'static str,
    description: Option<&'static str>,
    persona_ids: &'static [&'static str],
}

const BUILT_IN_TEAMS: &[BuiltInTeam] = &[];

// Built-in teams that have been retired. A stored copy that still exactly
// matches its seed is purged on load (the user never touched it); customized
// copies are demoted to user-owned teams by the retirement loop in
// merge_teams_impl.
const RETIRED_BUILT_IN_TEAMS: &[BuiltInTeam] = &[BuiltInTeam {
    id: "builtin-team:fizz",
    name: "Fizz",
    description: Some("Fizz works carefully and collaboratively."),
    persona_ids: &["builtin:fizz"],
}];

fn built_in_team_records(built_ins: &[BuiltInTeam], now: &str) -> Vec<TeamRecord> {
    built_ins
        .iter()
        .map(|team| TeamRecord {
            id: team.id.to_string(),
            name: team.name.to_string(),
            description: team.description.map(|s| s.to_string()),
            persona_ids: team.persona_ids.iter().map(|s| s.to_string()).collect(),
            is_builtin: true,
            source_dir: None,
            is_symlink: false,
            symlink_target: None,
            version: None,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        })
        .collect()
}

fn built_in_team_order(built_ins: &[BuiltInTeam], id: &str) -> Option<usize> {
    built_ins.iter().position(|team| team.id == id)
}

/// Add missing built-in teams, purge pristine retired teams, demote stale
/// built-ins, and preserve any user customizations to existing built-in teams
/// (name, description, persona membership). Returns the merged list and whether
/// the store changed.
fn merge_teams(stored: Vec<TeamRecord>, now: &str) -> (Vec<TeamRecord>, bool) {
    merge_teams_impl(BUILT_IN_TEAMS, RETIRED_BUILT_IN_TEAMS, stored, now)
}

fn merge_teams_impl(
    built_ins: &[BuiltInTeam],
    retired: &[BuiltInTeam],
    mut stored: Vec<TeamRecord>,
    now: &str,
) -> (Vec<TeamRecord>, bool) {
    let mut changed = false;

    // Seed missing built-ins / re-promote existing ones that were downgraded.
    for built_in in built_in_team_records(built_ins, now) {
        if let Some(existing) = stored.iter_mut().find(|record| record.id == built_in.id) {
            if !existing.is_builtin {
                existing.is_builtin = true;
                existing.updated_at = now.to_string();
                changed = true;
            }
        } else {
            stored.push(built_in);
            changed = true;
        }
    }

    // Purge stored copies that are still pristine w.r.t. a retired seed. The
    // user never touched them, so there is nothing to preserve.
    let before = stored.len();
    stored.retain(|record| {
        !retired.iter().any(|seed| {
            record.is_builtin
                && record.id == seed.id
                && record.name == seed.name
                && record.description.as_deref() == seed.description
                && record
                    .persona_ids
                    .iter()
                    .map(String::as_str)
                    .eq(seed.persona_ids.iter().copied())
                && record.source_dir.is_none()
                && !record.is_symlink
        })
    });
    if stored.len() != before {
        changed = true;
    }

    // Demote any stored team flagged as built-in whose id is no longer in
    // built_ins (e.g. a built-in that has been retired). The record stays so
    // existing references keep working; it becomes a user-owned custom team
    // they can edit or delete.
    for record in stored.iter_mut() {
        if record.is_builtin && built_in_team_order(built_ins, &record.id).is_none() {
            record.is_builtin = false;
            record.updated_at = now.to_string();
            changed = true;
        }
    }

    (stored, changed)
}

/// Reject deletion of built-in teams. Mirrors `validate_persona_deletion`
/// for personas — built-ins always come back via `merge_teams` on the
/// next load, so blocking the delete avoids a confusing "keeps coming
/// back" UX.
pub fn validate_team_deletion(team: &TeamRecord) -> Result<(), String> {
    if team.is_builtin {
        return Err("Built-in teams cannot be deleted.".to_string());
    }
    Ok(())
}

pub fn load_teams(app: &AppHandle) -> Result<Vec<TeamRecord>, String> {
    let path = teams_store_path(app)?;
    let now = now_iso();

    let records = if path.exists() {
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read teams store: {error}"))?;
        serde_json::from_str::<Vec<TeamRecord>>(&content)
            .map_err(|error| format!("failed to parse teams store: {error}"))?
    } else {
        Vec::new()
    };

    let (mut records, changed) = merge_teams(records, &now);
    sort_teams(&mut records);

    if changed || !path.exists() {
        save_teams(app, &records)?;
    }

    Ok(records)
}

pub fn save_teams(app: &AppHandle, records: &[TeamRecord]) -> Result<(), String> {
    let mut sorted = records.to_vec();
    sort_teams(&mut sorted);

    let path = teams_store_path(app)?;
    let payload = serde_json::to_vec_pretty(&sorted)
        .map_err(|error| format!("failed to serialize teams store: {error}"))?;
    crate::managed_agents::storage::atomic_write_json(&path, &payload)
}

/// Teams directory: `<AppDataDir>/agents/teams/`
pub(super) fn teams_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = managed_agents_base_dir(app)?.join("teams");
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create teams dir: {e}"))?;
    Ok(dir)
}

/// Validate team/pack ID: only `[a-zA-Z0-9._-]+` allowed (zip-slip defense).
pub(crate) fn validate_team_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("team ID is empty".into());
    }
    if id.len() > 128 {
        return Err(format!("team ID too long: {} chars (max 128)", id.len()));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(format!(
            "team ID contains invalid characters: \"{id}\". Only [a-zA-Z0-9._-] allowed."
        ));
    }
    if id.starts_with('.') {
        return Err(format!("team ID \"{id}\" must not start with '.'"));
    }
    if !id.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err(format!(
            "team ID \"{id}\" must contain at least one alphanumeric character"
        ));
    }
    Ok(())
}

/// Copy a directory tree, skipping symlinks (zip-slip defense).
fn copy_dir_no_symlinks(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("failed to create {}: {e}", dst.display()))?;
    for entry in fs::read_dir(src).map_err(|e| format!("failed to read {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
        let ft = entry
            .file_type()
            .map_err(|e| format!("file type error: {e}"))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ft.is_symlink() {
            continue;
        } else if ft.is_dir() {
            copy_dir_no_symlinks(&src_path, &dst_path)?;
        } else if ft.is_file() {
            fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("failed to copy {}: {e}", src_path.display()))?;
        }
    }
    Ok(())
}

/// Import a team from a local directory in open plugin format.
///
/// Copies the directory into `<data>/agents/teams/<resolved.id>/`,
/// creates PersonaRecords for each persona, creates a TeamRecord with source_dir set.
pub fn import_team_from_directory(
    app: &AppHandle,
    source_dir: &std::path::Path,
    symlink: bool,
) -> Result<TeamRecord, String> {
    use uuid::Uuid;

    // 1. Validate + resolve at source
    let resolved = buzz_persona_pkg::resolve::resolve_pack(source_dir)
        .map_err(|e| format!("team directory validation failed: {e}"))?;

    // 2. Sanitize team ID
    validate_team_id(&resolved.id)?;

    // 3. Check for existing team with same ID
    let teams_base = teams_dir(app)?;
    let dest = teams_base.join(&resolved.id);
    if dest.exists() {
        return Err(format!(
            "Team \"{}\" is already installed. Delete it first or use sync.",
            resolved.id
        ));
    }

    // 4. Determine install mode: symlink or copy
    let source_is_symlink = fs::symlink_metadata(source_dir)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    let use_symlink = symlink || source_is_symlink;

    if use_symlink {
        // Resolve the canonical target for symlink
        let canonical = fs::canonicalize(source_dir)
            .map_err(|e| format!("failed to resolve symlink target: {e}"))?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&canonical, &dest)
                .map_err(|e| format!("failed to create symlink: {e}"))?;
        }
        #[cfg(not(unix))]
        {
            // Fallback to copy on non-unix
            copy_dir_no_symlinks(source_dir, &dest)?;
        }
    } else {
        copy_dir_no_symlinks(source_dir, &dest)?;
    }

    // 5. Re-validate the copy/symlink target (defense-in-depth)
    let re_resolved = buzz_persona_pkg::resolve::resolve_pack(&dest).map_err(|e| {
        // Clean up on failure
        if use_symlink {
            let _ = fs::remove_file(&dest);
        } else {
            let _ = fs::remove_dir_all(&dest);
        }
        format!("team re-validation failed after install: {e}")
    })?;

    // 6. Create PersonaRecords
    let now = now_iso();
    let new_personas: Vec<AgentDefinition> = re_resolved
        .personas
        .iter()
        .map(|p| AgentDefinition {
            id: Uuid::new_v4().to_string(),
            display_name: p.display_name.clone(),
            avatar_url: p.avatar.clone(),
            system_prompt: p.system_prompt.clone(),
            runtime: p.runtime.clone(),
            model: p.model.clone(),
            provider: p.llm_provider.clone(),
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            source_team: Some(resolved.id.clone()),
            source_team_persona_slug: Some(p.name.clone()),
            env_vars: crate::managed_agents::env_vars::filter_derived_provider_model_env_vars(
                p.runtime_env_vars.iter().cloned(),
            ),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            mcp_toolsets: None,
            parallelism: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        })
        .collect();

    let persona_ids: Vec<String> = new_personas.iter().map(|p| p.id.clone()).collect();

    // 7. Save personas
    let mut personas = super::load_personas(app)?;
    personas.extend(new_personas);
    super::save_personas(app, &personas)?;

    // 8. Create and save TeamRecord
    let symlink_target = if use_symlink {
        fs::canonicalize(source_dir)
            .ok()
            .map(|p| p.display().to_string())
    } else {
        None
    };

    let team = TeamRecord {
        id: resolved.id,
        name: resolved.name,
        description: if resolved.description.is_empty() {
            None
        } else {
            Some(resolved.description)
        },
        persona_ids,
        is_builtin: false,
        source_dir: Some(dest),
        is_symlink: use_symlink,
        symlink_target,
        version: Some(resolved.version),
        created_at: now.clone(),
        updated_at: now,
    };

    let mut teams = load_teams(app)?;
    teams.push(team.clone());
    save_teams(app, &teams)?;

    Ok(team)
}

/// Delete a team, cascading removal of its sourced personas and backing dir.
///
/// Returns the d-tags of the personas removed by the cascade so the caller can
/// enqueue NIP-09 tombstones for them — without this, the team coordinate is
/// tombstoned but the orphaned kind:30175 persona heads stay live on the relay.
/// For JSON-only teams (no `source_dir`), nothing cascades and the returned
/// vec is empty.
pub fn delete_team_with_cascade(app: &AppHandle, team_id: &str) -> Result<Vec<String>, String> {
    let mut teams = load_teams(app)?;
    let team = teams
        .iter()
        .find(|record| record.id == team_id)
        .ok_or_else(|| format!("team {team_id} not found"))?;

    validate_team_deletion(team)?;

    let mut cascaded_persona_d_tags = Vec::new();

    if team.source_dir.is_some() {
        // Directory-backed team: full cascade
        // Match on the shared key (directory name) so legacy UUID-id teams
        // still cascade correctly.
        let persona_key = team_persona_key(team).to_string();

        // 1. Check no managed agents reference these personas
        let agents = crate::managed_agents::load_managed_agents(app)?;
        let referencing: Vec<&str> = agents
            .iter()
            .filter(|a| {
                a.persona_team_dir
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    == Some(persona_key.as_str())
            })
            .map(|a| a.name.as_str())
            .collect();
        if !referencing.is_empty() {
            return Err(format!(
                "Cannot delete team \"{team_id}\": {} agent(s) still reference it ({}). \
                 Delete or reconfigure them first.",
                referencing.len(),
                referencing.join(", ")
            ));
        }

        // 2. Remove all PersonaRecords sourced from this team
        let mut personas = super::load_personas(app)?;
        // Capture the d-tag of each cascaded persona BEFORE removal so the
        // caller can tombstone its kind:30175 coordinate on the relay.
        cascaded_persona_d_tags = personas
            .iter()
            .filter(|p| p.source_team.as_deref() == Some(persona_key.as_str()))
            .map(super::persona_events::persona_d_tag)
            .collect();
        personas.retain(|p| p.source_team.as_deref() != Some(persona_key.as_str()));
        super::save_personas(app, &personas)?;

        // 3. Remove directory
        if let Some(source_dir) = &team.source_dir {
            if source_dir.exists() {
                let is_symlink = fs::symlink_metadata(source_dir)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                if is_symlink {
                    fs::remove_file(source_dir)
                        .map_err(|e| format!("failed to remove team symlink: {e}"))?;
                } else {
                    fs::remove_dir_all(source_dir)
                        .map_err(|e| format!("failed to remove team directory: {e}"))?;
                }
            }
        }
    }

    // 4. Remove TeamRecord
    teams.retain(|record| record.id != team_id);
    save_teams(app, &teams)?;
    Ok(cascaded_persona_d_tags)
}

/// Re-reads a directory-backed team and reconciles with stored records.
pub fn sync_team_from_dir(
    app: &AppHandle,
    team_id: &str,
) -> Result<crate::managed_agents::SyncResult, String> {
    use uuid::Uuid;

    let teams = load_teams(app)?;
    let team = teams
        .iter()
        .find(|t| t.id == team_id)
        .ok_or_else(|| format!("team {team_id} not found"))?;

    let source_dir = team
        .source_dir
        .as_ref()
        .ok_or_else(|| format!("team {team_id} is not directory-backed"))?;

    // Personas reference the team's directory name (pack manifest ID) in
    // source_team, which may differ from team_id for pre-backfill teams.
    let persona_key = team_persona_key(team).to_string();

    if !source_dir.exists() {
        return Err(format!(
            "team directory does not exist: {}",
            source_dir.display()
        ));
    }

    // Resolve current state of the directory
    let resolved = buzz_persona_pkg::resolve::resolve_pack(source_dir)
        .map_err(|e| format!("failed to resolve team directory: {e}"))?;

    let mut personas = super::load_personas(app)?;
    let now = now_iso();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut updated = Vec::new();

    // Find existing personas for this team
    let existing_slugs: Vec<(String, String)> = personas
        .iter()
        .filter(|p| p.source_team.as_deref() == Some(persona_key.as_str()))
        .map(|p| {
            (
                p.source_team_persona_slug.clone().unwrap_or_default(),
                p.id.clone(),
            )
        })
        .collect();

    // Check for new personas in directory
    for dir_persona in &resolved.personas {
        if let Some((_slug, persona_id)) = existing_slugs
            .iter()
            .find(|(slug, _)| slug == &dir_persona.name)
        {
            // Existing persona — check for content changes
            if let Some(record) = personas.iter_mut().find(|p| p.id == *persona_id) {
                let mut changed = false;
                if record.display_name != dir_persona.display_name {
                    record.display_name = dir_persona.display_name.clone();
                    changed = true;
                }
                if record.system_prompt != dir_persona.system_prompt {
                    record.system_prompt = dir_persona.system_prompt.clone();
                    changed = true;
                }
                if record.avatar_url != dir_persona.avatar {
                    record.avatar_url = dir_persona.avatar.clone();
                    changed = true;
                }
                if record.runtime != dir_persona.runtime {
                    record.runtime = dir_persona.runtime.clone();
                    changed = true;
                }
                if record.model != dir_persona.model {
                    record.model = dir_persona.model.clone();
                    changed = true;
                }
                if record.provider != dir_persona.llm_provider {
                    record.provider = dir_persona.llm_provider.clone();
                    changed = true;
                }
                if changed {
                    record.updated_at = now.clone();
                    updated.push(persona_id.clone());
                }
            }
        } else {
            // New persona — create record
            let new_persona = AgentDefinition {
                id: Uuid::new_v4().to_string(),
                display_name: dir_persona.display_name.clone(),
                avatar_url: dir_persona.avatar.clone(),
                system_prompt: dir_persona.system_prompt.clone(),
                runtime: dir_persona.runtime.clone(),
                model: dir_persona.model.clone(),
                provider: dir_persona.llm_provider.clone(),
                name_pool: Vec::new(),
                is_builtin: false,
                is_active: true,
                source_team: Some(persona_key.clone()),
                source_team_persona_slug: Some(dir_persona.name.clone()),
                env_vars: crate::managed_agents::env_vars::filter_derived_provider_model_env_vars(
                    dir_persona.runtime_env_vars.iter().cloned(),
                ),
                respond_to: None,
                respond_to_allowlist: Vec::new(),
                mcp_toolsets: None,
                parallelism: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            };
            added.push(new_persona.id.clone());
            personas.push(new_persona);
        }
    }

    // Check for personas removed from directory
    let dir_slugs: Vec<&str> = resolved.personas.iter().map(|p| p.name.as_str()).collect();
    let to_remove: Vec<String> = existing_slugs
        .iter()
        .filter(|(slug, _)| !dir_slugs.contains(&slug.as_str()))
        .map(|(_, id)| id.clone())
        .collect();

    // Only remove if no active managed agent uses the persona
    let agents = crate::managed_agents::load_managed_agents(app)?;
    for persona_id in &to_remove {
        let in_use = agents
            .iter()
            .any(|a| a.persona_id.as_deref() == Some(persona_id));
        if !in_use {
            personas.retain(|p| p.id != *persona_id);
            removed.push(persona_id.clone());
        }
    }

    // Update team metadata if changed
    let mut teams = load_teams(app)?;
    let mut metadata_changed = false;
    if let Some(team_record) = teams.iter_mut().find(|t| t.id == team_id) {
        if team_record.name != resolved.name {
            team_record.name = resolved.name;
            metadata_changed = true;
        }
        let new_desc = if resolved.description.is_empty() {
            None
        } else {
            Some(resolved.description)
        };
        if team_record.description != new_desc {
            team_record.description = new_desc;
            metadata_changed = true;
        }
        let new_version = Some(resolved.version);
        if team_record.version != new_version {
            team_record.version = new_version;
            metadata_changed = true;
        }
        // Update persona_ids to reflect current state
        let current_ids: Vec<String> = personas
            .iter()
            .filter(|p| p.source_team.as_deref() == Some(persona_key.as_str()))
            .map(|p| p.id.clone())
            .collect();
        if team_record.persona_ids != current_ids {
            team_record.persona_ids = current_ids;
            metadata_changed = true;
        }
        if metadata_changed {
            team_record.updated_at = now;
        }
    }

    super::save_personas(app, &personas)?;
    save_teams(app, &teams)?;

    Ok(crate::managed_agents::SyncResult {
        personas_added: added,
        personas_removed: removed,
        personas_updated: updated,
        metadata_changed,
    })
}

/// Encode a team as a JSON blob for export. The format includes the team's
/// name, description, and the full persona data for each member (so the
/// import side can recreate personas that don't exist locally).
pub fn encode_team_json(
    team: &TeamRecord,
    personas: &[AgentDefinition],
) -> Result<Vec<u8>, String> {
    let mut missing_persona_ids = Vec::new();
    let mut resolved_personas = Vec::with_capacity(team.persona_ids.len());

    for persona_id in &team.persona_ids {
        let Some(persona) = personas
            .iter()
            .find(|candidate| candidate.id == *persona_id)
        else {
            missing_persona_ids.push(persona_id.clone());
            continue;
        };

        resolved_personas.push(serde_json::json!({
            "displayName": persona.display_name,
            "systemPrompt": persona.system_prompt,
            "avatarUrl": persona.avatar_url,
        }));
    }

    if !missing_persona_ids.is_empty() {
        return Err(format!(
            "Team {} references missing personas: {}. Repair the team before exporting.",
            team.name,
            missing_persona_ids.join(", ")
        ));
    }

    let map = serde_json::json!({
        "version": 1,
        "type": "team",
        "name": team.name,
        "description": team.description,
        "personas": resolved_personas,
    });

    serde_json::to_vec_pretty(&map).map_err(|e| format!("Failed to serialize team JSON: {e}"))
}

/// Parse a team JSON file. Returns the team name, description, and embedded persona previews.
pub fn parse_team_json(json_bytes: &[u8]) -> Result<ParsedTeamPreview, String> {
    let v: serde_json::Value =
        serde_json::from_slice(json_bytes).map_err(|e| format!("Invalid JSON: {e}"))?;

    let version = v.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
    if version != 1 {
        return Err(format!("Unsupported team version: {version}"));
    }

    let file_type = v.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if file_type != "team" {
        return Err("Not a team export file".to_string());
    }

    let name = v
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return Err("Team name is empty".to_string());
    }

    let description = v
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let personas = v
        .get("personas")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let display_name = p
                        .get("displayName")
                        .and_then(|v| v.as_str())?
                        .trim()
                        .to_string();
                    let system_prompt = p
                        .get("systemPrompt")
                        .and_then(|v| v.as_str())?
                        .trim()
                        .to_string();
                    let avatar_url = p
                        .get("avatarUrl")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    if display_name.is_empty() || system_prompt.is_empty() {
                        return None;
                    }
                    Some(TeamPersonaPreview {
                        display_name,
                        system_prompt,
                        avatar_url,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ParsedTeamPreview {
        name,
        description,
        personas,
    })
}

#[cfg(test)]
#[path = "teams_tests.rs"]
mod tests;
