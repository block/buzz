//! T4 migration: lift pack-level instructions into `TeamRecord.instructions`
//! and detach all directory-backed teams from their file-layer plumbing.

use std::collections::HashMap;
use std::path::Path;

use crate::managed_agents::{ManagedAgentRecord, TeamRecord};

/// Lift pack instructions into `TeamRecord.instructions` and detach
/// directory-backed teams from their source directories.
///
/// Runs on app launch if any `TeamRecord` still has `source_dir` set.
/// Both output files are written atomically (temp-file + rename), so a crash
/// mid-write leaves the previous version intact and the migration can safely
/// retry on next boot.
///
/// Steps (written last so the idempotency gate stays open until both files
/// are committed):
///
/// 1. Build a persona-key → team ID map from the directory-backed teams.
/// 2. Backfill `team_id` and clear `persona_team_dir`/`persona_name_in_team`
///    on every `ManagedAgentRecord`.
/// 3. Read `instructions.md` from each team's `source_dir`; populate
///    `instructions` if the field is not already set.
/// 4. Clear `source_dir`, `is_symlink`, `symlink_target`, `version` on each
///    directory-backed `TeamRecord`.
pub fn detach_directory_backed_teams(app: &tauri::AppHandle) {
    let Ok(base_dir) = crate::managed_agents::managed_agents_base_dir(app) else {
        return;
    };
    match detach_directory_backed_teams_in_dir(&base_dir) {
        Ok(0) => {}
        Ok(n) => eprintln!("buzz-desktop: detach-dir-teams: detached {n} directory-backed team(s)"),
        Err(e) => eprintln!("buzz-desktop: detach-dir-teams: {e}"),
    }
}

/// Core logic, decoupled from the Tauri `AppHandle` for testing.
///
/// `base_dir` is the managed-agents base directory (`<AppDataDir>/agents/`).
/// Returns the number of teams detached (0 = nothing to do).
pub(super) fn detach_directory_backed_teams_in_dir(base_dir: &Path) -> Result<usize, String> {
    let teams_path = base_dir.join("teams.json");
    let agents_path = base_dir.join("managed-agents.json");

    if !teams_path.exists() {
        return Ok(0);
    }

    let teams_content = std::fs::read_to_string(&teams_path)
        .map_err(|e| format!("failed to read teams.json: {e}"))?;
    let mut teams: Vec<TeamRecord> = serde_json::from_str(&teams_content)
        .map_err(|e| format!("failed to parse teams.json: {e}"))?;

    if !teams.iter().any(|t| t.source_dir.is_some()) {
        return Ok(0);
    }

    // Build persona-key → team ID map before clearing source_dirs.
    // persona-key = the directory name (pack manifest ID) or, for teams without
    // a source_dir, the team UUID. This matches the value stored in
    // AgentDefinition.source_team / ManagedAgentRecord.source_team.
    let persona_key_to_team_id: HashMap<String, String> = teams
        .iter()
        .filter(|t| t.source_dir.is_some())
        .map(|t| {
            let key = t
                .source_dir
                .as_deref()
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or(&t.id)
                .to_string();
            (key, t.id.clone())
        })
        .collect();

    // Step 1 (agents first so the idempotency gate on teams.json stays open
    // until both files are successfully written):
    if agents_path.exists() {
        let agents_content = std::fs::read_to_string(&agents_path)
            .map_err(|e| format!("failed to read managed-agents.json: {e}"))?;
        let mut agents: Vec<ManagedAgentRecord> = serde_json::from_str(&agents_content)
            .map_err(|e| format!("failed to parse managed-agents.json: {e}"))?;

        let mut agents_changed = false;
        for agent in agents.iter_mut() {
            // Backfill team_id from source_team (absorbed from the definition,
            // equals the pack directory name / persona key).
            if agent.team_id.is_none() {
                if let Some(source_team) = agent.source_team.as_deref() {
                    if let Some(team_id) = persona_key_to_team_id.get(source_team) {
                        agent.team_id = Some(team_id.clone());
                        agents_changed = true;
                    }
                }
            }
            // F1 (Thufir): clear instance-side pack plumbing. These fields
            // drove BUZZ_ACP_PERSONA_PACK / BUZZ_ACP_PERSONA_NAME env vars
            // at spawn, which T3 already removed. Clearing removes the dead
            // data so T6 can safely delete the consuming code.
            if agent.persona_team_dir.is_some() || agent.persona_name_in_team.is_some() {
                agent.persona_team_dir = None;
                agent.persona_name_in_team = None;
                agents_changed = true;
            }
        }

        if agents_changed {
            let payload = serde_json::to_vec_pretty(&agents)
                .map_err(|e| format!("failed to serialize managed-agents.json: {e}"))?;
            crate::managed_agents::atomic_write_json_restricted(&agents_path, &payload)?;
        }
    }

    // Step 2: lift instructions + detach each directory-backed team.
    // Written last — clearing source_dir closes the idempotency gate.
    let now = chrono::Utc::now().to_rfc3339();
    let mut detached = 0usize;
    for team in teams.iter_mut() {
        let Some(source_dir) = team.source_dir.take() else {
            continue;
        };

        // Lift instructions.md content if the field is not already populated.
        // If reading fails, log and leave instructions as-is (never corrupt).
        if team.instructions.is_none() {
            let instructions_path = source_dir.join("instructions.md");
            if instructions_path.exists() {
                match std::fs::read_to_string(&instructions_path) {
                    Ok(content) => {
                        let trimmed = content.trim().to_string();
                        if !trimmed.is_empty() {
                            team.instructions = Some(trimmed);
                        }
                    }
                    Err(e) => eprintln!(
                        "buzz-desktop: detach-dir-teams: team {}: \
                         failed to read instructions.md (preserving existing value): {e}",
                        team.id
                    ),
                }
            }
        }

        // F3 (Thufir): clear all file-layer fields. The UI badges off
        // isSymlink — leaving it set without a source_dir is inconsistent.
        team.is_symlink = false;
        team.symlink_target = None;
        team.version = None;
        team.updated_at = now.clone();
        detached += 1;
    }

    let payload = serde_json::to_vec_pretty(&teams)
        .map_err(|e| format!("failed to serialize teams.json: {e}"))?;
    crate::managed_agents::atomic_write_json(&teams_path, &payload)?;

    Ok(detached)
}
