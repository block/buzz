use super::test_support::*;

// ── detach_directory_backed_teams_in_dir tests ──────────────────────────

fn write_teams_json(base: &std::path::Path, records: &serde_json::Value) {
    std::fs::create_dir_all(base.join("agents")).unwrap();
    std::fs::write(
        base.join("agents/teams.json"),
        serde_json::to_vec_pretty(records).unwrap(),
    )
    .unwrap();
}

fn read_teams_json(base: &std::path::Path) -> Vec<serde_json::Value> {
    let content = std::fs::read_to_string(base.join("agents/teams.json")).unwrap();
    serde_json::from_str(&content).unwrap()
}

#[test]
fn detach_lifts_instructions_from_instructions_md() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    // Create a team directory with instructions.md
    let team_dir = base.join("teams/com.example.myteam");
    std::fs::create_dir_all(&team_dir).unwrap();
    std::fs::write(team_dir.join("instructions.md"), "Always be helpful.").unwrap();

    // Write teams.json with a directory-backed team
    write_teams_json(
        base,
        &serde_json::json!([{
            "id": "team-uuid-1",
            "name": "My Team",
            "persona_ids": [],
            "is_builtin": false,
            "source_dir": team_dir.to_str().unwrap(),
            "is_symlink": false,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }]),
    );

    let n = super::detach::detach_directory_backed_teams_in_dir(&base.join("agents")).unwrap();
    assert_eq!(n, 1);

    let teams = read_teams_json(base);
    assert_eq!(teams[0]["instructions"], "Always be helpful.");
    assert!(teams[0].get("source_dir").is_none_or(|v| v.is_null()));
    assert_eq!(teams[0]["is_symlink"], false);
}

#[test]
fn detach_skips_lift_if_instructions_already_set() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    let team_dir = base.join("teams/com.example.myteam");
    std::fs::create_dir_all(&team_dir).unwrap();
    std::fs::write(team_dir.join("instructions.md"), "From disk.").unwrap();

    write_teams_json(
        base,
        &serde_json::json!([{
            "id": "team-uuid-1",
            "name": "My Team",
            "instructions": "Already set.",
            "persona_ids": [],
            "is_builtin": false,
            "source_dir": team_dir.to_str().unwrap(),
            "is_symlink": false,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }]),
    );

    super::detach::detach_directory_backed_teams_in_dir(&base.join("agents")).unwrap();

    let teams = read_teams_json(base);
    // Should keep the pre-existing instructions, not overwrite with disk content
    assert_eq!(teams[0]["instructions"], "Already set.");
}

#[test]
fn detach_clears_is_symlink_and_version() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    let team_dir = base.join("teams/com.example.myteam");
    std::fs::create_dir_all(&team_dir).unwrap();

    write_teams_json(
        base,
        &serde_json::json!([{
            "id": "team-uuid-1",
            "name": "My Team",
            "persona_ids": [],
            "is_builtin": false,
            "source_dir": team_dir.to_str().unwrap(),
            "is_symlink": true,
            "symlink_target": "/some/external/path",
            "version": "1.2.3",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }]),
    );

    super::detach::detach_directory_backed_teams_in_dir(&base.join("agents")).unwrap();

    let teams = read_teams_json(base);
    assert_eq!(teams[0]["is_symlink"], false);
    assert!(teams[0].get("symlink_target").is_none_or(|v| v.is_null()));
    assert!(teams[0].get("version").is_none_or(|v| v.is_null()));
    assert!(teams[0].get("source_dir").is_none_or(|v| v.is_null()));
}

#[test]
fn detach_backfills_team_id_on_agents() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    let team_dir = base.join("teams/com.example.myteam");
    std::fs::create_dir_all(&team_dir).unwrap();

    write_teams_json(
        base,
        &serde_json::json!([{
            "id": "team-uuid-1",
            "name": "My Team",
            "persona_ids": ["persona-1"],
            "is_builtin": false,
            "source_dir": team_dir.to_str().unwrap(),
            "is_symlink": false,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }]),
    );

    // Agent with source_team = directory name, no team_id, has persona_team_dir
    write_agents_json(
        base,
        &serde_json::json!([{
            "pubkey": "aaaa",
            "name": "agent-1",
            "persona_id": "persona-1",
            "source_team": "com.example.myteam",
            "persona_team_dir": team_dir.to_str().unwrap(),
            "persona_name_in_team": "agent-slug",
            "relay_url": "wss://relay.example.com",
            "acp_command": "claude",
            "agent_command": "claude",
            "mcp_command": "claude",
            "agent_args": [],
            "turn_timeout_seconds": 60,
            "parallelism": 1,
            "start_on_app_launch": false,
            "auto_restart_on_config_change": true,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }]),
    );

    super::detach::detach_directory_backed_teams_in_dir(&base.join("agents")).unwrap();

    let agents = read_agents_json(base);
    assert_eq!(agents[0]["team_id"], "team-uuid-1");
    assert!(agents[0]
        .get("persona_team_dir")
        .is_none_or(|v| v.is_null()));
    assert!(agents[0]
        .get("persona_name_in_team")
        .is_none_or(|v| v.is_null()));
}

#[test]
fn detach_is_idempotent_on_second_run() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    let team_dir = base.join("teams/com.example.myteam");
    std::fs::create_dir_all(&team_dir).unwrap();
    std::fs::write(team_dir.join("instructions.md"), "Team rules.").unwrap();

    write_teams_json(
        base,
        &serde_json::json!([{
            "id": "team-uuid-1",
            "name": "My Team",
            "persona_ids": [],
            "is_builtin": false,
            "source_dir": team_dir.to_str().unwrap(),
            "is_symlink": false,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }]),
    );

    // First run
    let n1 = super::detach::detach_directory_backed_teams_in_dir(&base.join("agents")).unwrap();
    assert_eq!(n1, 1);

    // Second run — should be a no-op
    let n2 = super::detach::detach_directory_backed_teams_in_dir(&base.join("agents")).unwrap();
    assert_eq!(n2, 0);

    // Instructions should still be set from first run
    let teams = read_teams_json(base);
    assert_eq!(teams[0]["instructions"], "Team rules.");
}

#[test]
fn detach_skips_non_directory_backed_teams() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    write_teams_json(
        base,
        &serde_json::json!([{
            "id": "team-uuid-1",
            "name": "Pure JSON Team",
            "instructions": "Existing instructions.",
            "persona_ids": [],
            "is_builtin": false,
            "is_symlink": false,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }]),
    );

    let n = super::detach::detach_directory_backed_teams_in_dir(&base.join("agents")).unwrap();
    assert_eq!(n, 0);

    // File should not have been modified
    let teams = read_teams_json(base);
    assert_eq!(teams[0]["instructions"], "Existing instructions.");
}

#[cfg(unix)]
#[test]
fn detach_retries_after_teams_write_failure() {
    // Write-seam interruption: make the agents/ directory read-only so the
    // teams.json atomic write fails mid-detach. The agents.json write uses
    // atomic_write_json_restricted (tmp+rename in the same dir), which also
    // fails under a read-only parent. On retry (directory made writable),
    // the still-open source_dir gate lets the migration complete.
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    let team_dir = base.join("teams/com.example.myteam");
    std::fs::create_dir_all(&team_dir).unwrap();
    std::fs::write(team_dir.join("instructions.md"), "Team rules.").unwrap();

    write_teams_json(
        base,
        &serde_json::json!([{
            "id": "team-uuid-1",
            "name": "My Team",
            "persona_ids": ["persona-1"],
            "is_builtin": false,
            "source_dir": team_dir.to_str().unwrap(),
            "is_symlink": false,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }]),
    );
    write_agents_json(
        base,
        &serde_json::json!([{
            "pubkey": "aaaa",
            "name": "agent-1",
            "persona_id": "persona-1",
            "source_team": "com.example.myteam",
            "persona_team_dir": team_dir.to_str().unwrap(),
            "persona_name_in_team": "agent-slug",
            "relay_url": "wss://relay.example.com",
            "acp_command": "claude",
            "agent_command": "claude",
            "mcp_command": "claude",
            "agent_args": [],
            "turn_timeout_seconds": 60,
            "parallelism": 1,
            "start_on_app_launch": false,
            "auto_restart_on_config_change": true,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }]),
    );

    // Make agents/ read-only so the atomic write (tmp file creation) fails.
    let agents_dir = base.join("agents");
    std::fs::set_permissions(&agents_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    // Boot 1: detach fails because neither file can be written.
    let result = super::detach::detach_directory_backed_teams_in_dir(&agents_dir);
    assert!(
        result.is_err(),
        "detach must fail when directory is read-only"
    );

    // source_dir gate is still open — teams.json is unchanged.
    // Restore permissions to read the file.
    std::fs::set_permissions(&agents_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    let teams_after_fail = read_teams_json(base);
    assert!(
        teams_after_fail[0]
            .get("source_dir")
            .is_some_and(|v| !v.is_null()),
        "source_dir must remain set after failed detach"
    );

    // Boot 2: retry with writable directory — should succeed.
    let n2 = super::detach::detach_directory_backed_teams_in_dir(&agents_dir).unwrap();
    assert_eq!(n2, 1, "retry detach must succeed");

    let teams_after_retry = read_teams_json(base);
    assert!(
        teams_after_retry[0]
            .get("source_dir")
            .is_none_or(|v| v.is_null()),
        "source_dir cleared after successful retry"
    );
    assert_eq!(
        teams_after_retry[0]["instructions"], "Team rules.",
        "instructions lifted on retry"
    );
}
