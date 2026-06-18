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

    reconcile_legacy_command_names_in_file(&dir.path().join("agents/managed-agents.json"));

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

    reconcile_legacy_command_names_in_file(&dir.path().join("agents/managed-agents.json"));

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

    reconcile_legacy_command_names_in_file(&dir.path().join("agents/managed-agents.json"));

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

    reconcile_legacy_command_names_in_file(&path);

    assert_eq!(before, std::fs::read_to_string(&path).unwrap());
}

#[test]
fn reconcile_legacy_command_names_rewrites_persona_runtime() {
    let dir = tempfile::tempdir().unwrap();
    write_personas_json(
        dir.path(),
        &serde_json::json!([{
            "id": "persona-1",
            "display_name": "Brain",
            "runtime": "sprout-agent"
        }]),
    );

    reconcile_legacy_persona_runtimes_in_file(&dir.path().join("agents/personas.json"));

    let records = read_personas_json(dir.path());
    assert_eq!(records[0]["runtime"], "buzz-agent");
}

#[test]
fn reconcile_legacy_command_names_rewrites_runtime_after_provider_migration() {
    let dir = tempfile::tempdir().unwrap();
    write_personas_json(
        dir.path(),
        &serde_json::json!([{
            "id": "persona-1",
            "display_name": "Brain",
            "provider": "sprout-agent"
        }]),
    );
    let path = dir.path().join("agents/personas.json");

    rename_provider_to_runtime_in_personas(&path);
    reconcile_legacy_persona_runtimes_in_file(&path);

    let records = read_personas_json(dir.path());
    assert_eq!(records[0]["runtime"], "buzz-agent");
    assert!(records[0].get("provider").is_none());
}

#[test]
fn reconcile_legacy_command_names_preserves_non_legacy_persona_runtime() {
    let dir = tempfile::tempdir().unwrap();
    write_personas_json(
        dir.path(),
        &serde_json::json!([{
            "id": "persona-1",
            "display_name": "Solo",
            "runtime": "goose"
        }]),
    );
    let path = dir.path().join("agents/personas.json");
    let before = std::fs::read_to_string(&path).unwrap();

    reconcile_legacy_persona_runtimes_in_file(&path);

    assert_eq!(before, std::fs::read_to_string(&path).unwrap());
}

#[test]
fn rewrite_legacy_persona_md_runtime_rewrites_frontmatter_only() {
    let content = concat!(
        "---\n",
        "name: brain\n",
        "display_name: Brain\n",
        "description: Test persona\n",
        "runtime: sprout-agent\n",
        "---\n",
        "Body mentions runtime: sprout-agent.\n",
    );

    let updated = rewrite_legacy_persona_md_runtime(content).unwrap();

    assert!(updated.contains("runtime: buzz-agent\n"));
    assert!(updated.contains("Body mentions runtime: sprout-agent.\n"));
}

#[test]
fn reconcile_legacy_team_persona_runtime_files_rewrites_persona_md() {
    let dir = tempfile::tempdir().unwrap();
    let teams_dir = dir.path().join("agents/teams/com.example.team/agents");
    std::fs::create_dir_all(&teams_dir).unwrap();
    let persona_path = teams_dir.join("brain.persona.md");
    std::fs::write(
        &persona_path,
        concat!(
            "---\n",
            "name: brain\n",
            "display_name: Brain\n",
            "description: Test persona\n",
            "runtime: sprout-agent\n",
            "---\n",
            "Prompt\n",
        ),
    )
    .unwrap();

    reconcile_legacy_team_persona_runtime_files(&dir.path().join("agents/teams"));

    let updated = std::fs::read_to_string(persona_path).unwrap();
    assert!(updated.contains("runtime: buzz-agent\n"));
}
