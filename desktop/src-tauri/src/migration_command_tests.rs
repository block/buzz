use super::test_support::*;
use super::*;

#[test]
fn reconcile_legacy_command_names_rewrites_renamed_sidecars() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Brain",
            "acp_command": "sprout-acp",
            "agent_command": "sprout-agent",
            "mcp_command": "sprout-dev-mcp"
        }]),
    );

    reconcile_legacy_command_names_in_file(&dir.path().join("agents/managed-agents.json")).unwrap();

    let records = read_agents_json(dir.path());
    assert_eq!(records[0]["acp_command"], "buzz-acp");
    assert_eq!(records[0]["agent_command"], "buzz-agent");
    assert_eq!(records[0]["mcp_command"], "buzz-dev-mcp");
}

#[test]
fn reconcile_legacy_command_names_updates_removed_mcp_server_for_buzz_agent() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Brain",
            "acp_command": "sprout-acp",
            "agent_command": "sprout-agent",
            "mcp_command": "sprout-mcp-server"
        }]),
    );

    reconcile_legacy_command_names_in_file(&dir.path().join("agents/managed-agents.json")).unwrap();

    let records = read_agents_json(dir.path());
    assert_eq!(records[0]["acp_command"], "buzz-acp");
    assert_eq!(records[0]["agent_command"], "buzz-agent");
    assert_eq!(records[0]["mcp_command"], "buzz-dev-mcp");
}

#[test]
fn reconcile_legacy_command_names_clears_removed_mcp_server_for_goose() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Solo",
            "acp_command": "sprout-acp",
            "agent_command": "goose",
            "mcp_command": "sprout-mcp-server"
        }]),
    );

    reconcile_legacy_command_names_in_file(&dir.path().join("agents/managed-agents.json")).unwrap();

    let records = read_agents_json(dir.path());
    assert_eq!(records[0]["acp_command"], "buzz-acp");
    assert_eq!(records[0]["agent_command"], "goose");
    assert_eq!(records[0]["mcp_command"], "");
}

#[test]
fn reconcile_legacy_command_names_preserves_custom_commands() {
    let dir = tempfile::tempdir().unwrap();
    let json = serde_json::json!([{
        "name": "Custom",
        "acp_command": "custom-acp",
        "agent_command": "custom-agent",
        "mcp_command": "custom-mcp"
    }]);
    write_agents_json(dir.path(), &json);
    let path = dir.path().join("agents/managed-agents.json");
    let before = std::fs::read_to_string(&path).unwrap();

    reconcile_legacy_command_names_in_file(&path).unwrap();

    assert_eq!(before, std::fs::read_to_string(&path).unwrap());
}

// Tests for `reconcile_legacy_persona_runtimes_in_file`,
// `rewrite_legacy_persona_md_runtime`, and
// `reconcile_legacy_team_persona_runtime_files` were removed alongside those
// deleted functions. The scoped pipeline's `_in_dir` variants carry the
// equivalent coverage.
