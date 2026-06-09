use std::{fs, path::PathBuf};

use tauri::AppHandle;

use crate::{
    managed_agents::{managed_agents_base_dir, PersonaRecord, TeamRecord},
    util::now_iso,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Built-in teams
// ---------------------------------------------------------------------------

struct BuiltInTeam {
    id: &'static str,
    name: &'static str,
    description: Option<&'static str>,
    persona_ids: &'static [&'static str],
}

const BUILT_IN_TEAMS: &[BuiltInTeam] = &[BuiltInTeam {
    id: "builtin-team:kit-scout",
    name: "Kit & Scout",
    description: Some("Kit orchestrates and builds; Scout researches, plans, and reviews."),
    persona_ids: &["builtin:kit", "builtin:scout"],
}];

fn built_in_team_records(now: &str) -> Vec<TeamRecord> {
    BUILT_IN_TEAMS
        .iter()
        .map(|team| TeamRecord {
            id: team.id.to_string(),
            name: team.name.to_string(),
            description: team.description.map(|s| s.to_string()),
            persona_ids: team.persona_ids.iter().map(|s| s.to_string()).collect(),
            is_builtin: true,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        })
        .collect()
}

fn built_in_team_order(id: &str) -> Option<usize> {
    BUILT_IN_TEAMS.iter().position(|team| team.id == id)
}

/// Add missing built-in teams, demote stale built-ins, and preserve any
/// user customizations to existing built-in teams (name, description,
/// persona membership). Returns the merged list and whether the store
/// changed.
fn merge_teams(mut stored: Vec<TeamRecord>, now: &str) -> (Vec<TeamRecord>, bool) {
    let mut changed = false;

    for built_in in built_in_team_records(now) {
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

    // Demote any stored team flagged as built-in whose id is no longer in
    // BUILT_IN_TEAMS (e.g. a built-in that has been retired). The record
    // stays so existing references keep working; it becomes a user-owned
    // custom team they can edit or delete.
    for record in stored.iter_mut() {
        if record.is_builtin && built_in_team_order(&record.id).is_none() {
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

// ---------------------------------------------------------------------------
// Team JSON export / import
// ---------------------------------------------------------------------------

/// Encode a team as a JSON blob for export. The format includes the team's
/// name, description, and the full persona data for each member (so the
/// import side can recreate personas that don't exist locally).
pub fn encode_team_json(team: &TeamRecord, personas: &[PersonaRecord]) -> Result<Vec<u8>, String> {
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
mod tests {
    use super::{
        encode_team_json, merge_teams, parse_team_json, sort_teams, validate_team_deletion,
        BUILT_IN_TEAMS,
    };
    use crate::managed_agents::{PersonaRecord, TeamRecord};

    fn team(id: &str, name: &str) -> TeamRecord {
        TeamRecord {
            id: id.to_string(),
            name: name.to_string(),
            description: None,
            persona_ids: Vec::new(),
            is_builtin: false,
            created_at: "2026-03-20T00:00:00Z".to_string(),
            updated_at: "2026-03-20T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn sort_teams_alphabetical_case_insensitive() {
        let mut teams = vec![team("3", "Zulu"), team("1", "alpha"), team("2", "Bravo")];
        sort_teams(&mut teams);

        let names: Vec<&str> = teams.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "Bravo", "Zulu"]);
    }

    #[test]
    fn sort_teams_breaks_ties_by_id() {
        let mut teams = vec![team("b", "same"), team("a", "same")];
        sort_teams(&mut teams);

        let ids: Vec<&str> = teams.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn sort_teams_empty_is_noop() {
        let mut teams: Vec<TeamRecord> = Vec::new();
        sort_teams(&mut teams);
        assert!(teams.is_empty());
    }

    // -----------------------------------------------------------------------
    // encode / parse round-trip tests
    // -----------------------------------------------------------------------

    fn persona(id: &str, name: &str, prompt: &str) -> PersonaRecord {
        PersonaRecord {
            id: id.to_string(),
            display_name: name.to_string(),
            avatar_url: None,
            system_prompt: prompt.to_string(),
            runtime: None,
            model: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            source_pack: None,
            source_pack_persona_slug: None,
            env_vars: std::collections::BTreeMap::new(),
            created_at: "2026-03-20T00:00:00Z".to_string(),
            updated_at: "2026-03-20T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn encode_parse_round_trip() {
        let t = team("t1", "My Team");
        let t = TeamRecord {
            description: Some("A great team".to_string()),
            persona_ids: vec!["p1".to_string(), "p2".to_string()],
            ..t
        };
        let personas = vec![
            persona("p1", "Alice", "You are Alice"),
            persona("p2", "Bob", "You are Bob"),
        ];

        let bytes = encode_team_json(&t, &personas).unwrap();
        let parsed = parse_team_json(&bytes).unwrap();

        assert_eq!(parsed.name, "My Team");
        assert_eq!(parsed.description.as_deref(), Some("A great team"));
        assert_eq!(parsed.personas.len(), 2);
        assert_eq!(parsed.personas[0].display_name, "Alice");
        assert_eq!(parsed.personas[0].system_prompt, "You are Alice");
        assert_eq!(parsed.personas[1].display_name, "Bob");
        assert_eq!(parsed.personas[1].system_prompt, "You are Bob");
    }

    #[test]
    fn encode_errors_for_missing_personas() {
        let t = TeamRecord {
            persona_ids: vec!["p1".to_string(), "missing".to_string()],
            ..team("t1", "Team")
        };
        let personas = vec![persona("p1", "Alice", "prompt")];

        let err = encode_team_json(&t, &personas).unwrap_err();

        assert_eq!(
            err,
            "Team Team references missing personas: missing. Repair the team before exporting."
        );
    }

    #[test]
    fn parse_team_json_invalid_version() {
        let json = serde_json::json!({
            "version": 99,
            "type": "team",
            "name": "X",
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let err = parse_team_json(&bytes).unwrap_err();
        assert!(err.contains("Unsupported team version"), "{err}");
    }

    #[test]
    fn parse_team_json_wrong_type() {
        let json = serde_json::json!({
            "version": 1,
            "type": "persona",
            "name": "X",
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let err = parse_team_json(&bytes).unwrap_err();
        assert!(err.contains("Not a team export file"), "{err}");
    }

    #[test]
    fn parse_team_json_empty_name() {
        let json = serde_json::json!({
            "version": 1,
            "type": "team",
            "name": "  ",
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let err = parse_team_json(&bytes).unwrap_err();
        assert!(err.contains("Team name is empty"), "{err}");
    }

    #[test]
    fn parse_team_json_skips_invalid_personas() {
        let json = serde_json::json!({
            "version": 1,
            "type": "team",
            "name": "Team",
            "personas": [
                { "displayName": "Good", "systemPrompt": "prompt" },
                { "displayName": "", "systemPrompt": "prompt" },
                { "displayName": "NoPrompt" },
            ],
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let parsed = parse_team_json(&bytes).unwrap();
        assert_eq!(parsed.personas.len(), 1);
        assert_eq!(parsed.personas[0].display_name, "Good");
    }

    #[test]
    fn parse_team_json_no_personas_key() {
        let json = serde_json::json!({
            "version": 1,
            "type": "team",
            "name": "Solo",
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let parsed = parse_team_json(&bytes).unwrap();
        assert!(parsed.personas.is_empty());
        assert_eq!(parsed.name, "Solo");
    }

    // -----------------------------------------------------------------------
    // merge_teams + validate_team_deletion tests
    // -----------------------------------------------------------------------

    #[test]
    fn merge_teams_adds_missing_built_ins() {
        let (records, changed) = merge_teams(Vec::new(), "2026-05-07T00:00:00Z");

        assert!(changed);
        assert_eq!(records.len(), BUILT_IN_TEAMS.len());
        assert!(records.iter().all(|record| record.is_builtin));
        let names: Vec<&str> = records.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Kit & Scout"]);
    }

    #[test]
    fn merge_teams_preserves_user_customizations_to_builtin() {
        let mut customized = team("builtin-team:kit-scout", "Kit & Scout (mine)");
        customized.is_builtin = true;
        customized.persona_ids = vec!["builtin:kit".to_string()];

        let (records, _changed) = merge_teams(vec![customized], "2026-05-07T00:00:00Z");

        let kit_scout = records
            .iter()
            .find(|t| t.id == "builtin-team:kit-scout")
            .expect("Kit & Scout built-in should exist");
        assert_eq!(kit_scout.name, "Kit & Scout (mine)");
        assert_eq!(kit_scout.persona_ids, vec!["builtin:kit".to_string()]);
        assert!(kit_scout.is_builtin);
    }

    #[test]
    fn merge_teams_preserves_unrelated_user_teams() {
        let user_team = team("user-uuid", "My Team");
        let (records, _changed) = merge_teams(vec![user_team], "2026-05-07T00:00:00Z");

        assert!(records.iter().any(|t| t.id == "user-uuid"));
        assert!(records.iter().any(|t| t.id == "builtin-team:kit-scout"));
    }

    #[test]
    fn merge_teams_demotes_retired_built_ins() {
        let mut retired = team("builtin-team:legacy", "Legacy");
        retired.is_builtin = true;

        let (records, changed) = merge_teams(vec![retired], "2026-05-07T00:00:00Z");

        assert!(changed);
        let demoted = records
            .iter()
            .find(|t| t.id == "builtin-team:legacy")
            .expect("retired built-in should be retained as a custom team");
        assert!(!demoted.is_builtin);
        assert_eq!(demoted.updated_at, "2026-05-07T00:00:00Z");
    }

    #[test]
    fn merge_teams_repromotes_existing_builtin_marked_as_custom() {
        // If someone hand-edits the store and flips is_builtin to false on a
        // canonical built-in id, merge_teams should restore the flag.
        let mut downgraded = team("builtin-team:kit-scout", "Kit & Scout");
        downgraded.is_builtin = false;

        let (records, changed) = merge_teams(vec![downgraded], "2026-05-07T00:00:00Z");

        assert!(changed);
        let kit_scout = records
            .iter()
            .find(|t| t.id == "builtin-team:kit-scout")
            .expect("Kit & Scout should exist");
        assert!(kit_scout.is_builtin);
    }

    #[test]
    fn validate_team_deletion_rejects_built_ins() {
        let mut built_in = team("builtin-team:kit-scout", "Kit & Scout");
        built_in.is_builtin = true;

        let err = validate_team_deletion(&built_in).unwrap_err();
        assert_eq!(err, "Built-in teams cannot be deleted.");
    }

    #[test]
    fn validate_team_deletion_allows_custom_teams() {
        let custom = team("user-uuid", "My Team");
        assert!(validate_team_deletion(&custom).is_ok());
    }
}
