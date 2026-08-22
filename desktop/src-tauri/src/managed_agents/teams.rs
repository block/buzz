use std::{fs, path::PathBuf};

use tauri::AppHandle;

use crate::{
    managed_agents::{managed_agents_base_dir, ManagedAgentRecord, TeamRecord},
    util::now_iso,
};

use super::team_repair::team_persona_key;

pub(crate) fn teams_store_path(app: &AppHandle) -> Result<PathBuf, String> {
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

const BUILT_IN_TEAMS: &[BuiltInTeam] = &[BuiltInTeam {
    id: "builtin-team:welcome",
    name: "Welcome Team",
    description: Some("A friendly starter trio ready to help you plan, create, and ship."),
    persona_ids: &["builtin:fizz", "builtin:honey", "builtin:bumble"],
}];

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
            instructions: None,
            persona_ids: team.persona_ids.iter().map(|s| s.to_string()).collect(),
            is_builtin: true,
            // Built-in teams are never shareable to the catalog.
            shared: false,
            catalog_source: None,
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

/// Read and merge built-in teams without persisting changes.
///
/// Returns the merged, sorted team list. No file is written — callers that
/// only need the current logical state (e.g. the snapshot-import pre-read)
/// use this to avoid a write-on-load side effect.
pub(crate) fn load_teams_readonly(path: &std::path::Path) -> Result<Vec<TeamRecord>, String> {
    let now = now_iso();

    let records = if path.exists() {
        let content = fs::read_to_string(path)
            .map_err(|error| format!("failed to read teams store: {error}"))?;
        serde_json::from_str::<Vec<TeamRecord>>(&content)
            .map_err(|error| format!("failed to parse teams store: {error}"))?
    } else {
        Vec::new()
    };

    let (mut records, _changed) = merge_teams(records, &now);
    sort_teams(&mut records);
    Ok(records)
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

/// Names of managed agents that still reference `team` — either via the
/// legacy `persona_team_dir` link (directory-backed teams only) or the
/// `team_id` field (every team kind, all agents created after the team_id
/// seam landed). Used to block team deletion while agents still depend on it.
fn agents_referencing_team<'a>(
    agents: &'a [ManagedAgentRecord],
    team: &TeamRecord,
) -> Vec<&'a str> {
    let persona_key = team_persona_key(team);
    agents
        .iter()
        .filter(|a| {
            a.persona_team_dir
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                == Some(persona_key)
                || a.team_id.as_deref() == Some(team.id.as_str())
        })
        .map(|a| a.name.as_str())
        .collect()
}

/// Delete a team, cascading removal of its sourced personas and backing dir.
///
/// Returns the d-tags of the personas removed by the cascade so the caller can
/// enqueue NIP-09 tombstones for them — without this, the team coordinate is
/// tombstoned but the orphaned kind:30175 persona heads stay live on the relay.
/// For JSON-only teams (no `source_dir`), nothing cascades and the returned
/// vec is empty. For catalog-adopted teams (`catalog_source` present), member
/// copies matching this publication's provenance are deactivated (re-activatable
/// on re-add), not deleted.
pub fn delete_team_with_cascade(app: &AppHandle, team_id: &str) -> Result<Vec<String>, String> {
    let mut teams = load_teams(app)?;
    let team = teams
        .iter()
        .find(|record| record.id == team_id)
        .ok_or_else(|| format!("team {team_id} not found"))?;

    validate_team_deletion(team)?;

    let agents = crate::managed_agents::load_managed_agents(app)?;
    let referencing = agents_referencing_team(&agents, team);
    if !referencing.is_empty() {
        return Err(format!(
            "Cannot delete team \"{team_id}\": {} agent(s) still reference it ({}). \
             Delete or reconfigure them first.",
            referencing.len(),
            referencing.join(", ")
        ));
    }

    let mut cascaded_persona_d_tags = Vec::new();

    if team.source_dir.is_some() {
        // Directory-backed team: cascade personas + backing directory too.
        // Match on the shared key (directory name) so legacy UUID-id teams
        // still cascade correctly.
        let persona_key = team_persona_key(team).to_string();

        // 1. Remove all PersonaRecords sourced from this team
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

        // 2. Remove directory
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
    } else if let Some(catalog_source) = &team.catalog_source.clone() {
        // Catalog-adopted team: deactivate member copies matching this
        // publication that no remaining team references AND no standalone
        // managed agent depends on — a copy backing an agent must stay active
        // so the agent keeps working.
        let mut personas = super::load_personas(app)?;
        let managed_agents = crate::managed_agents::load_managed_agents(app)?;

        // The teams remaining AFTER this one is removed — used to check
        // whether any copy is still referenced before deactivating it.
        let remaining_teams: Vec<_> = teams.iter().filter(|t| t.id != team_id).collect();

        let changed = deactivate_catalog_member_copies_with_ref_check(
            &mut personas,
            &catalog_source.owner_pubkey,
            &catalog_source.team_d_tag,
            &remaining_teams,
            &managed_agents,
        );

        // Remove the team record from the working slice; save both atomically.
        teams.retain(|record| record.id != team_id);

        let personas_path = super::managed_agents_store_path(app)?;
        let teams_path = teams_store_path(app)?;
        let personas_to_write = personas.clone();
        let teams_to_write = teams.clone();

        // Byte-snapshot both stores before writing so a save failure rolls
        // back both, via the same commit primitive as catalog adoption (I6).
        let personas_snap = crate::managed_agents::storage::snapshot_store(&personas_path)?;
        let teams_snap = crate::managed_agents::storage::snapshot_store(&teams_path)?;

        crate::managed_agents::storage::commit_stores_with_snapshots(
            &personas_path,
            &teams_path,
            personas_snap,
            teams_snap,
            || {
                if changed {
                    super::save_personas(app, &personas_to_write)?;
                }
                Ok(())
            },
            || save_teams(app, &teams_to_write),
        )?;

        return Ok(cascaded_persona_d_tags);
    }

    // Remove TeamRecord
    teams.retain(|record| record.id != team_id);
    save_teams(app, &teams)?;
    Ok(cascaded_persona_d_tags)
}

/// Deactivate non-built-in personas whose provenance matches
/// `(owner_pubkey, team_d_tag)` AND that are not referenced by any remaining
/// team's `persona_ids` or any managed agent's `persona_id`.
///
/// The agent case is critical: deleting a catalog team must not archive a copy
/// a standalone managed agent depends on, which would leave the agent pointing
/// at a hidden inactive definition. Returns `true` when any record changed.
pub(crate) fn deactivate_catalog_member_copies_with_ref_check(
    personas: &mut [super::AgentDefinition],
    owner_pubkey: &str,
    team_d_tag: &str,
    remaining_teams: &[&super::TeamRecord],
    managed_agents: &[super::ManagedAgentRecord],
) -> bool {
    let mut changed = false;
    for persona in personas.iter_mut() {
        if persona.is_builtin {
            continue;
        }
        let is_copy = persona
            .team_catalog_source
            .as_ref()
            .is_some_and(|s| s.owner_pubkey == owner_pubkey && s.team_d_tag == team_d_tag);
        if !is_copy || !persona.is_active {
            continue;
        }
        // Skip copies still referenced by another remaining team.
        let still_in_team = remaining_teams
            .iter()
            .any(|t| t.persona_ids.iter().any(|id| id == &persona.id));
        // Skip copies that a standalone managed agent was created from.
        let still_in_agent = managed_agents
            .iter()
            .any(|a| a.persona_id.as_deref() == Some(persona.id.as_str()));
        if still_in_team || still_in_agent {
            continue;
        }
        persona.is_active = false;
        changed = true;
    }
    changed
}

#[cfg(test)]
#[path = "teams_tests.rs"]
mod tests;

/// Test-only seam for [`delete_team_with_cascade`] that takes explicit file
/// paths instead of an `AppHandle`. Mirrors the catalog-adopted deletion path
/// (the only path that uses the byte-rollback boundary) without requiring a
/// full Tauri runtime.
///
/// Only the catalog-adopted path is covered by this seam because that is the
/// path with the byte-rollback boundary. Directory-backed team deletion
/// requires filesystem operations that are best left to integration tests.
#[cfg(test)]
pub(crate) fn delete_catalog_team_at(
    personas_path: &std::path::Path,
    teams_path: &std::path::Path,
    team_id: &str,
) -> Result<(), String> {
    // Read raw JSON without the merge-in-built-ins side effect so the test
    // stores reflect exactly what delete_team_with_cascade writes (which also
    // reads via load_teams, not load_teams_readonly, and never writes back
    // built-ins in the middle of a delete).
    let personas: Vec<super::AgentDefinition> = if personas_path.exists() {
        let json = std::fs::read_to_string(personas_path)
            .map_err(|e| format!("failed to read personas: {e}"))?;
        serde_json::from_str(&json).map_err(|e| format!("failed to parse personas: {e}"))?
    } else {
        Vec::new()
    };
    let teams: Vec<TeamRecord> = if teams_path.exists() {
        let json = std::fs::read_to_string(teams_path)
            .map_err(|e| format!("failed to read teams: {e}"))?;
        serde_json::from_str(&json).map_err(|e| format!("failed to parse teams: {e}"))?
    } else {
        Vec::new()
    };

    let team = teams
        .iter()
        .find(|t| t.id == team_id)
        .ok_or_else(|| format!("team {team_id} not found"))?;

    let catalog_source = team
        .catalog_source
        .as_ref()
        .ok_or_else(|| "delete_catalog_team_at only handles catalog-adopted teams".to_string())?
        .clone();

    let mut personas_mut = personas;
    let remaining_teams: Vec<&TeamRecord> = teams.iter().filter(|t| t.id != team_id).collect();

    // No managed agents in the test seam — pass an empty slice. Test coverage
    // for the agent-reference preservation path lives in teams_tests.rs.
    deactivate_catalog_member_copies_with_ref_check(
        &mut personas_mut,
        &catalog_source.owner_pubkey,
        &catalog_source.team_d_tag,
        &remaining_teams,
        &[],
    );

    let new_teams: Vec<TeamRecord> = teams.into_iter().filter(|t| t.id != team_id).collect();

    let personas_snap = super::storage::snapshot_store(personas_path)?;
    let teams_snap = super::storage::snapshot_store(teams_path)?;

    super::storage::commit_stores_with_snapshots(
        personas_path,
        teams_path,
        personas_snap,
        teams_snap,
        || {
            let json = serde_json::to_vec_pretty(&personas_mut)
                .map_err(|e| format!("failed to serialize personas: {e}"))?;
            super::storage::atomic_write_json(personas_path, &json)
        },
        || {
            let mut sorted = new_teams.clone();
            sort_teams(&mut sorted);
            let json = serde_json::to_vec_pretty(&sorted)
                .map_err(|e| format!("failed to serialize teams: {e}"))?;
            super::storage::atomic_write_json(teams_path, &json)
        },
    )?;

    Ok(())
}
