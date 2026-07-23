//! Tests for `propagate_persona_name_rename` — the helper that propagates a
//! persona definition's display_name change to linked agent instances.

use super::*;

fn agent(persona_id: &str, name: &str, display_name: Option<&str>) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: format!("pubkey-{name}"),
        name: name.to_string(),
        persona_id: Some(persona_id.to_string()),
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: String::new(),
        agent_command: String::new(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 0,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: std::collections::BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: String::new(),
        updated_at: String::new(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: Default::default(),
        respond_to_allowlist: vec![],
        display_name: display_name.map(str::to_string),
        slug: None,
        runtime: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: vec![],
        definition_parallelism: None,
        relay_mesh: None,
    }
}

#[test]
fn test_rename_propagates_to_matching_instance() {
    // An instance whose `name` equals the OLD persona display_name must get
    // both `name` and `display_name` updated to the new value.
    let mut records = vec![agent("persona-1", "Paul", Some("Paul"))];

    let renamed = propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Paul Atreides");

    assert_eq!(
        renamed,
        vec!["pubkey-Paul".to_string()],
        "must report the renamed record's pubkey"
    );
    assert_eq!(records[0].name, "Paul Atreides", "name must be updated");
    assert_eq!(
        records[0].display_name,
        Some("Paul Atreides".to_string()),
        "display_name must be updated"
    );
    // The relay-profile sync params use `record.name`; after rename it carries
    // the new display_name, so the relay profile will be published with the correct name.
    assert_eq!(records[0].name, "Paul Atreides");
}

#[test]
fn test_rename_skips_pool_named_instance() {
    // A pool-named instance (e.g. "Birch") has a name DIFFERENT from the
    // persona display_name. It must keep its individualised name.
    let mut records = vec![agent("persona-1", "Birch", Some("Birch"))];

    let renamed = propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Paul Atreides");

    assert!(
        renamed.is_empty(),
        "pool-named instance must not be reported as renamed"
    );
    assert_eq!(records[0].name, "Birch", "pool name must be preserved");
    assert_eq!(
        records[0].display_name,
        Some("Birch".to_string()),
        "pool display_name must be preserved"
    );
}

#[test]
fn test_rename_propagates_both_name_and_display_name() {
    // Explicit dual-field check: BOTH `name` and `display_name` must be
    // updated so the relay profile and the local UI are consistent.
    let mut records = vec![agent("persona-1", "OldName", None)];

    propagate_persona_name_rename(&mut records, "persona-1", "OldName", "NewName");

    assert_eq!(records[0].name, "NewName");
    assert_eq!(records[0].display_name, Some("NewName".to_string()));
}

#[test]
fn test_rename_only_affects_linked_persona() {
    // An instance linked to a DIFFERENT persona must not be touched, even
    // if it happens to carry the same display_name.
    let mut records = vec![
        agent("persona-1", "Paul", Some("Paul")),
        agent("persona-2", "Paul", Some("Paul")),
    ];

    propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Paul Atreides");

    assert_eq!(records[0].name, "Paul Atreides", "linked instance renamed");
    assert_eq!(
        records[1].name, "Paul",
        "unrelated persona's instance untouched"
    );
}

#[test]
fn test_rename_renames_all_matching_instances_in_one_pass() {
    // Several instances may carry the definition name (multi-instance deploys
    // without a name pool): one call renames every match and reports each
    // pubkey, which is what the relay profile sync collection keys on.
    let mut records = vec![
        agent("persona-1", "Paul", Some("Paul")),
        agent("persona-1", "Paul", Some("Paul")),
        agent("persona-1", "Birch", Some("Birch")),
    ];
    records[1].pubkey = "pubkey-Paul-2".to_string();

    let renamed = propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Duncan Idaho");

    assert_eq!(
        renamed,
        vec!["pubkey-Paul".to_string(), "pubkey-Paul-2".to_string()],
        "every matching instance's pubkey must be reported"
    );
    assert_eq!(records[0].name, "Duncan Idaho");
    assert_eq!(records[1].name, "Duncan Idaho");
    assert_eq!(records[2].name, "Birch", "pool-named instance untouched");
}
