//! Unit tests for `managed_agents/teams.rs`.
//!
//! Kept in a sibling file so `teams.rs` stays under the 1000-line gate;
//! `#[path]`-included from there.

use super::{
    agents_referencing_team, deactivate_catalog_member_copies_with_ref_check, load_teams_readonly,
    merge_teams, merge_teams_impl, sort_teams, validate_team_deletion, BuiltInTeam,
};
use crate::managed_agents::{
    AgentDefinition, ManagedAgentRecord, TeamMemberCatalogSource, TeamRecord,
};

fn team(id: &str, name: &str) -> TeamRecord {
    TeamRecord {
        id: id.to_string(),
        name: name.to_string(),
        description: None,
        instructions: None,
        persona_ids: Vec::new(),
        is_builtin: false,
        shared: false,
        catalog_source: None,
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

// ── agents_referencing_team ─────────────────────────────────────────────

fn managed_agent(name: &str) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: name.to_string(),
        name: name.to_string(),
        persona_id: None,
        team_id: None,
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: "ws://localhost:3000".to_string(),
        avatar_url: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: "buzz-agent".to_string(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 300,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: std::collections::BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: false,
        runtime_pid: None,
        backend: crate::managed_agents::BackendKind::Local,
        backend_agent_id: None,
        provider_policy_pending: false,
        provider_binary_path: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: crate::managed_agents::RespondTo::OwnerOnly,
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        relay_mesh: None,
        effort_level: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: vec![],
        definition_parallelism: None,
    }
}

/// A new-style agent (created after the `team_id` seam landed) that links to
/// a JSON-only team purely via `team_id` — the only kind of team that carries
/// no `source_dir`/`persona_team_dir` at all — must still be caught, or the
/// "team in use" delete guard silently never fires for it.
#[test]
fn agents_referencing_team_matches_on_team_id() {
    let t = team("json-team-1", "Json Team");
    let mut linked = managed_agent("Linked Agent");
    linked.team_id = Some("json-team-1".to_string());
    let unrelated = managed_agent("Unrelated Agent");

    let agents = vec![linked, unrelated];
    let referencing = agents_referencing_team(&agents, &t);

    assert_eq!(referencing, vec!["Linked Agent"]);
}

/// Legacy pack-backed agents that predate the `team_id` field record their
/// link solely via `persona_team_dir` (matched against the team's directory
/// name) — this path must keep working after the `team_id` check was added.
#[test]
fn agents_referencing_team_matches_on_persona_team_dir() {
    let mut t = team("uuid-1", "Dir Team");
    t.source_dir = Some(std::path::PathBuf::from("/teams/com.example.pack"));
    let mut legacy = managed_agent("Legacy Agent");
    legacy.persona_team_dir = Some(std::path::PathBuf::from("/installed/com.example.pack"));
    let unrelated = managed_agent("Unrelated Agent");

    let agents = vec![legacy, unrelated];
    let referencing = agents_referencing_team(&agents, &t);

    assert_eq!(referencing, vec!["Legacy Agent"]);
}

#[test]
fn agents_referencing_team_empty_when_no_matches() {
    let t = team("json-team-2", "Json Team");
    let agents = vec![managed_agent("Agent A"), managed_agent("Agent B")];

    assert!(agents_referencing_team(&agents, &t).is_empty());
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
        instructions: None,
        persona_ids: vec!["builtin:fizz".to_string()],
        is_builtin: true,
        shared: false,
        catalog_source: None,
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
        instructions: None,
        persona_ids: vec!["builtin:fizz".to_string(), "extra:persona".to_string()],
        is_builtin: true,
        shared: false,
        catalog_source: None,
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
fn welcome_team_is_seeded_and_idempotent() {
    let (records, changed) = merge_teams(Vec::new(), "2026-07-01T00:00:00Z");

    assert!(changed);
    assert_eq!(records.len(), 1);
    let welcome = &records[0];
    assert_eq!(welcome.id, "builtin-team:welcome");
    assert_eq!(welcome.name, "Welcome Team");
    assert_eq!(
        welcome.description.as_deref(),
        Some("A friendly starter trio ready to help you plan, create, and ship.")
    );
    assert_eq!(
        welcome.persona_ids,
        vec![
            "builtin:fizz".to_string(),
            "builtin:honey".to_string(),
            "builtin:bumble".to_string(),
        ]
    );
    assert!(welcome.is_builtin);

    let expected = serde_json::to_value(&records).unwrap();
    let (records_after_second_merge, changed) = merge_teams(records, "2026-07-02T00:00:00Z");
    assert!(!changed);
    assert_eq!(
        serde_json::to_value(records_after_second_merge).unwrap(),
        expected
    );
}

#[test]
fn welcome_team_seed_does_not_overwrite_customization() {
    let (mut records, _) = merge_teams(Vec::new(), "2026-07-01T00:00:00Z");
    let welcome = records
        .iter_mut()
        .find(|team| team.id == "builtin-team:welcome")
        .expect("welcome team should be seeded");
    welcome.name = "My Welcome Team".to_string();
    welcome.description = Some("My customized starter team.".to_string());
    welcome.persona_ids = vec!["builtin:honey".to_string()];

    let (records, changed) = merge_teams(records, "2026-07-02T00:00:00Z");

    assert!(!changed);
    let welcome = records
        .iter()
        .find(|team| team.id == "builtin-team:welcome")
        .expect("customized welcome team should be preserved");
    assert_eq!(welcome.name, "My Welcome Team");
    assert_eq!(
        welcome.description.as_deref(),
        Some("My customized starter team.")
    );
    assert_eq!(welcome.persona_ids, vec!["builtin:honey".to_string()]);
    assert!(welcome.is_builtin);
}

// ── load_teams_readonly tests ──────────────────────────────────────────

#[test]
fn load_teams_readonly_absent_file_performs_no_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("teams.json");

    // File does not exist.
    assert!(!path.exists());

    let records = load_teams_readonly(&path).unwrap();

    // Returns the merged built-in list without persisting it.
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, "builtin-team:welcome");

    // The file must still NOT exist — no write-on-load side effect.
    assert!(
        !path.exists(),
        "load_teams_readonly must not create the file"
    );
}

#[test]
fn load_teams_readonly_surfaces_parse_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("teams.json");
    std::fs::write(&path, b"not valid json").unwrap();

    let result = load_teams_readonly(&path);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("failed to parse teams store"),
        "parse error must be surfaced"
    );
}

#[cfg(unix)]
#[test]
fn load_teams_readonly_surfaces_read_error() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("teams.json");
    std::fs::write(&path, b"[]").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

    let result = load_teams_readonly(&path);

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("failed to read teams store"),
        "read error must be surfaced"
    );
}

// ── deactivate_catalog_member_copies_with_ref_check ──────────────────────────

const OWNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const D_TAG: &str = "my-team";

fn catalog_copy(id: &str, owner: &str, d_tag: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: id.to_string(),
        avatar_url: None,
        system_prompt: String::new(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: Some(TeamMemberCatalogSource {
            owner_pubkey: owner.to_string(),
            team_d_tag: d_tag.to_string(),
            member_key: id.to_string(),
            projection_hash: "hash".to_string(),
        }),
        env_vars: Default::default(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn builtin_copy(id: &str) -> AgentDefinition {
    let mut p = catalog_copy(id, OWNER, D_TAG);
    p.is_builtin = true;
    p
}

#[test]
fn test_deactivate_catalog_member_copies_deactivates_matching_copies() {
    let mut personas = vec![
        catalog_copy("m1", OWNER, D_TAG),
        catalog_copy("m2", OWNER, D_TAG),
    ];
    let changed =
        deactivate_catalog_member_copies_with_ref_check(&mut personas, OWNER, D_TAG, &[], &[]);
    assert!(changed);
    assert!(!personas[0].is_active, "m1 should be deactivated");
    assert!(!personas[1].is_active, "m2 should be deactivated");
}

#[test]
fn test_deactivate_catalog_member_copies_skips_different_owner() {
    let other = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let mut personas = vec![catalog_copy("m1", other, D_TAG)];
    let changed =
        deactivate_catalog_member_copies_with_ref_check(&mut personas, OWNER, D_TAG, &[], &[]);
    assert!(!changed, "different owner must not be deactivated");
    assert!(personas[0].is_active);
}

#[test]
fn test_deactivate_catalog_member_copies_skips_different_d_tag() {
    let mut personas = vec![catalog_copy("m1", OWNER, "other-team")];
    let changed =
        deactivate_catalog_member_copies_with_ref_check(&mut personas, OWNER, D_TAG, &[], &[]);
    assert!(!changed, "different d-tag must not be deactivated");
    assert!(personas[0].is_active);
}

#[test]
fn test_deactivate_catalog_member_copies_skips_builtins() {
    // Built-in substitutions are local records, not copies — deleting the team
    // must never deactivate them.
    let mut personas = vec![builtin_copy("builtin:fizz")];
    let changed =
        deactivate_catalog_member_copies_with_ref_check(&mut personas, OWNER, D_TAG, &[], &[]);
    assert!(!changed, "built-in should not be deactivated");
    assert!(personas[0].is_active);
}

#[test]
fn test_deactivate_catalog_member_copies_skips_already_inactive() {
    let mut personas = vec![{
        let mut p = catalog_copy("m1", OWNER, D_TAG);
        p.is_active = false;
        p
    }];
    let changed =
        deactivate_catalog_member_copies_with_ref_check(&mut personas, OWNER, D_TAG, &[], &[]);
    assert!(
        !changed,
        "already-inactive record should not count as a change"
    );
}

#[test]
fn test_deactivate_catalog_member_copies_is_scoped_per_publication() {
    // A copy belonging to a DIFFERENT team by the same publisher must not be
    // deactivated — it belongs to a separate adoption.
    let mut personas = vec![
        catalog_copy("m1", OWNER, D_TAG),
        catalog_copy("m2", OWNER, "other-team"),
    ];
    deactivate_catalog_member_copies_with_ref_check(&mut personas, OWNER, D_TAG, &[], &[]);
    assert!(
        !personas[0].is_active,
        "m1 (matching) should be deactivated"
    );
    assert!(
        personas[1].is_active,
        "m2 (different d-tag) should remain active"
    );
}

// ── ref-check-specific behaviour ─────────────────────────────────────────────

#[test]
fn test_ref_check_preserves_copy_still_referenced_by_another_team() {
    // m1 is in both D_TAG (being deleted) and "team-two" (remaining).
    // Only D_TAG is being deleted, so m1 must stay active because team-two
    // still needs it.
    let mut personas = vec![catalog_copy("m1", OWNER, D_TAG)];
    let remaining = team("team-two", "Team Two");
    let remaining_with_m1: TeamRecord = TeamRecord {
        persona_ids: vec!["m1".to_string()],
        ..remaining
    };
    let remaining_teams: Vec<&TeamRecord> = vec![&remaining_with_m1];

    let changed = deactivate_catalog_member_copies_with_ref_check(
        &mut personas,
        OWNER,
        D_TAG,
        &remaining_teams,
        &[], // no managed agents in this test
    );

    assert!(!changed, "a referenced copy must not be deactivated");
    assert!(
        personas[0].is_active,
        "m1 is still referenced by team-two and must stay active"
    );
}

#[test]
fn test_ref_check_deactivates_copy_not_referenced_by_any_remaining_team() {
    // m1 is in D_TAG (being deleted) but not in any remaining team.
    let mut personas = vec![catalog_copy("m1", OWNER, D_TAG)];
    let unrelated_remaining = team("team-two", "Team Two");
    // team-two's persona_ids is empty, so m1 is not referenced.
    let remaining_teams: Vec<&TeamRecord> = vec![&unrelated_remaining];

    let changed = deactivate_catalog_member_copies_with_ref_check(
        &mut personas,
        OWNER,
        D_TAG,
        &remaining_teams,
        &[], // no managed agents in this test
    );

    assert!(changed, "unreferenced copy must be deactivated");
    assert!(!personas[0].is_active);
}

#[test]
fn test_ref_check_deactivates_one_but_preserves_another_in_same_call() {
    // m1 is referenced by a remaining team; m2 is not. The function must
    // deactivate m2 but leave m1 active in a single call.
    let mut personas = vec![
        catalog_copy("m1", OWNER, D_TAG),
        catalog_copy("m2", OWNER, D_TAG),
    ];
    let remaining_with_m1: TeamRecord = TeamRecord {
        persona_ids: vec!["m1".to_string()],
        ..team("team-two", "Team Two")
    };
    let remaining_teams: Vec<&TeamRecord> = vec![&remaining_with_m1];

    let changed = deactivate_catalog_member_copies_with_ref_check(
        &mut personas,
        OWNER,
        D_TAG,
        &remaining_teams,
        &[], // no managed agents in this test
    );

    assert!(changed, "at least one copy was deactivated");
    assert!(personas[0].is_active, "m1 is referenced — must stay active");
    assert!(
        !personas[1].is_active,
        "m2 is unreferenced — must be deactivated"
    );
}

#[test]
fn test_ref_check_preserves_copy_used_by_a_standalone_managed_agent() {
    // Thufir finding 1: adopt a catalog team, build a standalone managed agent
    // from one of its personas (persona_id = copy.id, no team_id), then delete
    // the catalog team. The persona copy must NOT be archived because the agent
    // still depends on it.
    //
    // Policy: preserve-not-block — deletion of the team succeeds, but copies
    // linked to a live agent stay active so the agent keeps working.
    let m1_id = "m1";
    let m2_id = "m2";
    let mut personas = vec![
        catalog_copy(m1_id, OWNER, D_TAG),
        catalog_copy(m2_id, OWNER, D_TAG),
    ];

    // A standalone managed agent whose persona_id points at the m1 copy.
    let mut agent = managed_agent("my-agent");
    agent.persona_id = Some(m1_id.to_string());

    let changed = deactivate_catalog_member_copies_with_ref_check(
        &mut personas,
        OWNER,
        D_TAG,
        &[], // no remaining teams reference either copy
        std::slice::from_ref(&agent),
    );

    assert!(changed, "m2 (unreferenced) must be deactivated");
    assert!(
        personas[0].is_active,
        "m1 is used by a managed agent and must stay active"
    );
    assert!(
        !personas[1].is_active,
        "m2 is not used by any agent and must be deactivated"
    );
}

// ── delete_catalog_team_at: production-path delete/persist/reload/re-add ──
//
// Tests that exercise the catalog-adopted team deletion path through the
// `delete_catalog_team_at` seam (which mirrors `delete_team_with_cascade`'s
// catalog branch without needing a Tauri AppHandle).

fn catalog_persona(id: &str, owner: &str, d_tag: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: id.to_string(),
        avatar_url: None,
        system_prompt: "Do the work.".to_string(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: Some(crate::managed_agents::TeamMemberCatalogSource {
            owner_pubkey: owner.to_string(),
            team_d_tag: d_tag.to_string(),
            member_key: id.to_string(),
            projection_hash: "a".repeat(64),
        }),
        env_vars: std::collections::BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

fn catalog_team(id: &str, owner: &str, d_tag: &str, persona_ids: Vec<String>) -> TeamRecord {
    TeamRecord {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        instructions: None,
        persona_ids,
        is_builtin: false,
        shared: false,
        catalog_source: Some(crate::managed_agents::TeamCatalogSource {
            owner_pubkey: owner.to_string(),
            team_d_tag: d_tag.to_string(),
        }),
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

fn write_stores(base: &std::path::Path, personas: &[AgentDefinition], teams: &[TeamRecord]) {
    std::fs::write(
        base.join("personas.json"),
        serde_json::to_string(personas).unwrap(),
    )
    .unwrap();
    std::fs::write(
        base.join("teams.json"),
        serde_json::to_string(teams).unwrap(),
    )
    .unwrap();
}

fn read_personas(base: &std::path::Path) -> Vec<AgentDefinition> {
    let json = std::fs::read_to_string(base.join("personas.json")).unwrap();
    serde_json::from_str(&json).unwrap()
}

fn read_teams(base: &std::path::Path) -> Vec<TeamRecord> {
    let json = std::fs::read_to_string(base.join("teams.json")).unwrap_or_default();
    serde_json::from_str(&json).unwrap_or_default()
}

#[test]
fn test_delete_catalog_team_deactivates_members_and_removes_team() {
    // Full lifecycle: add a catalog-adopted team with two members, delete it
    // via delete_catalog_team_at, then reload and verify the team is gone and
    // the member copies are deactivated.
    let dir = tempfile::tempdir().unwrap();
    let owner = "a".repeat(64);
    let d_tag = "team-alpha";

    let m1 = catalog_persona("m1", &owner, d_tag);
    let m2 = catalog_persona("m2", &owner, d_tag);
    let t = catalog_team(
        "team-abc",
        &owner,
        d_tag,
        vec!["m1".to_string(), "m2".to_string()],
    );
    write_stores(dir.path(), &[m1, m2], &[t]);

    let personas_path = dir.path().join("personas.json");
    let teams_path = dir.path().join("teams.json");

    super::delete_catalog_team_at(&personas_path, &teams_path, "team-abc").unwrap();

    let after_personas = read_personas(dir.path());
    let after_teams = read_teams(dir.path());

    assert_eq!(after_teams.len(), 0, "team must be removed");
    assert_eq!(
        after_personas.len(),
        2,
        "copies stay in store but deactivated"
    );
    assert!(
        !after_personas[0].is_active && !after_personas[1].is_active,
        "all copies must be deactivated"
    );
}

#[test]
fn test_delete_catalog_team_team_save_failure_rolls_back_both_stores() {
    // When the teams save fails, the byte-rollback must restore both personas
    // and teams to their pre-delete state. We simulate teams-save failure by
    // using commit_stores_with_snapshots with an injected failure on the
    // teams-write callback.
    use crate::managed_agents::storage;

    let dir = tempfile::tempdir().unwrap();
    let owner = "c".repeat(64);
    let d_tag = "team-gamma";

    let m1 = catalog_persona("m1", &owner, d_tag);
    let t = catalog_team("team-gamma-copy", &owner, d_tag, vec!["m1".to_string()]);
    let personas_path = dir.path().join("personas.json");
    let teams_path = dir.path().join("teams.json");
    write_stores(
        dir.path(),
        std::slice::from_ref(&m1),
        std::slice::from_ref(&t),
    );

    // Snapshot the original bytes for comparison.
    let orig_personas_bytes = std::fs::read(&personas_path).unwrap();
    let orig_teams_bytes = std::fs::read(&teams_path).unwrap();

    // Simulate the delete: personas-write succeeds, teams-write fails.
    let personas_snap = storage::snapshot_store(&personas_path).unwrap();
    let teams_snap = storage::snapshot_store(&teams_path).unwrap();

    let mut personas_mut = vec![m1.clone()];
    personas_mut[0].is_active = false;
    let personas_bytes = serde_json::to_vec_pretty(&personas_mut).unwrap();

    let result = storage::commit_stores_with_snapshots(
        &personas_path,
        &teams_path,
        personas_snap,
        teams_snap,
        || storage::atomic_write_json(&personas_path, &personas_bytes),
        || Err("simulated teams-write failure".to_string()),
    );

    assert!(result.is_err(), "write failure must propagate");
    // Both files must be restored to their original bytes.
    assert_eq!(
        std::fs::read(&personas_path).unwrap(),
        orig_personas_bytes,
        "personas must be restored to original bytes"
    );
    assert_eq!(
        std::fs::read(&teams_path).unwrap(),
        orig_teams_bytes,
        "teams must be restored to original bytes"
    );
}
