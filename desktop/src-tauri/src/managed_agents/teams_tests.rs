//! Unit tests for `managed_agents/teams.rs`.
//!
//! Kept in a sibling file so `teams.rs` stays under the 1000-line gate;
//! `#[path]`-included from there.

use super::{
    encode_team_json, merge_teams, merge_teams_impl, parse_team_json, sort_teams,
    validate_team_deletion, BuiltInTeam,
};
use crate::managed_agents::{AgentDefinition, TeamRecord};

fn team(id: &str, name: &str) -> TeamRecord {
    TeamRecord {
        id: id.to_string(),
        name: name.to_string(),
        description: None,
        persona_ids: Vec::new(),
        is_builtin: false,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
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

fn persona(id: &str, name: &str, prompt: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: name.to_string(),
        avatar_url: None,
        system_prompt: prompt.to_string(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: std::collections::BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        mcp_toolsets: None,
        parallelism: None,
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
        "name": "Fizz",
    });
    let bytes = serde_json::to_vec(&json).unwrap();
    let parsed = parse_team_json(&bytes).unwrap();
    assert!(parsed.personas.is_empty());
    assert_eq!(parsed.name, "Fizz");
}

#[test]
fn merge_teams_adds_missing_built_ins() {
    let synthetic = BuiltInTeam {
        id: "builtin-team:test",
        name: "Test Team",
        description: Some("A synthetic test team."),
        persona_ids: &["builtin:test-persona"],
    };

    let (records, changed) =
        merge_teams_impl(&[synthetic], &[], Vec::new(), "2026-05-07T00:00:00Z");

    assert!(changed);
    assert_eq!(records.len(), 1);
    assert!(records.iter().all(|r| r.is_builtin));
    assert_eq!(records[0].id, "builtin-team:test");
}

#[test]
fn merge_teams_preserves_user_customizations_to_builtin() {
    let synthetic = BuiltInTeam {
        id: "builtin-team:test",
        name: "Test Team",
        description: None,
        persona_ids: &["builtin:test-persona"],
    };
    let mut customized = team("builtin-team:test", "Test Team (mine)");
    customized.is_builtin = true;
    customized.persona_ids = vec!["builtin:test-persona".to_string()];

    let (records, _changed) =
        merge_teams_impl(&[synthetic], &[], vec![customized], "2026-05-07T00:00:00Z");

    let found = records
        .iter()
        .find(|t| t.id == "builtin-team:test")
        .expect("synthetic built-in should exist");
    assert_eq!(found.name, "Test Team (mine)");
    assert_eq!(found.persona_ids, vec!["builtin:test-persona".to_string()]);
    assert!(found.is_builtin);
}

#[test]
fn merge_teams_preserves_unrelated_user_teams() {
    let synthetic = BuiltInTeam {
        id: "builtin-team:test",
        name: "Test Team",
        description: None,
        persona_ids: &[],
    };
    let user_team = team("user-uuid", "My Team");

    let (records, _changed) =
        merge_teams_impl(&[synthetic], &[], vec![user_team], "2026-05-07T00:00:00Z");

    assert!(records.iter().any(|t| t.id == "user-uuid"));
    assert!(records.iter().any(|t| t.id == "builtin-team:test"));
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
    // canonical built-in id, merge_teams_impl should restore the flag.
    let synthetic = BuiltInTeam {
        id: "builtin-team:test",
        name: "Test Team",
        description: None,
        persona_ids: &[],
    };
    let mut downgraded = team("builtin-team:test", "Test Team");
    downgraded.is_builtin = false;

    let (records, changed) =
        merge_teams_impl(&[synthetic], &[], vec![downgraded], "2026-05-07T00:00:00Z");

    assert!(changed);
    let found = records
        .iter()
        .find(|t| t.id == "builtin-team:test")
        .expect("synthetic built-in should exist");
    assert!(found.is_builtin);
}

#[test]
fn validate_team_deletion_rejects_built_ins() {
    let mut built_in = team("builtin-team:fizz", "Fizz");
    built_in.is_builtin = true;

    let err = validate_team_deletion(&built_in).unwrap_err();
    assert_eq!(err, "Built-in teams cannot be deleted.");
}

// Migration pins — exercise the real merge_teams wrapper (with production consts).

#[test]
fn migration_pristine_fizz_is_purged() {
    // A stored record that exactly matches the retired Fizz seed is dropped
    // on load — the user never touched it, so nothing is lost.
    let pristine = TeamRecord {
        id: "builtin-team:fizz".to_string(),
        name: "Fizz".to_string(),
        description: Some("Fizz works carefully and collaboratively.".to_string()),
        persona_ids: vec!["builtin:fizz".to_string()],
        is_builtin: true,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let (records, changed) = merge_teams(vec![pristine], "2026-07-01T00:00:00Z");

    assert!(changed);
    assert!(!records.iter().any(|t| t.id == "builtin-team:fizz"));
}

#[test]
fn migration_customized_fizz_is_demoted_to_user_team() {
    // A stored Fizz that was renamed (or had a persona added) is retained
    // but demoted to a user-owned team so the user can edit or delete it.
    let customized = TeamRecord {
        id: "builtin-team:fizz".to_string(),
        name: "Fizz (customized)".to_string(),
        description: Some("Fizz works carefully and collaboratively.".to_string()),
        persona_ids: vec!["builtin:fizz".to_string(), "extra:persona".to_string()],
        is_builtin: true,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let (records, changed) = merge_teams(vec![customized], "2026-07-01T00:00:00Z");

    assert!(changed);
    let demoted = records
        .iter()
        .find(|t| t.id == "builtin-team:fizz")
        .expect("customized fizz should be retained as a user-owned team");
    assert!(!demoted.is_builtin);
    assert_eq!(demoted.updated_at, "2026-07-01T00:00:00Z");
}

#[test]
fn migration_empty_store_stays_empty() {
    // Fresh installs see no teams and no spurious changed flag.
    let (records, changed) = merge_teams(Vec::new(), "2026-07-01T00:00:00Z");

    assert!(!changed);
    assert!(records.is_empty());
}
