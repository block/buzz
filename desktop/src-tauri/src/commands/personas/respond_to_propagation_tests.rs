//! Tests for `propagate_persona_respond_to` — definition respond-to edits must
//! reach linked instance records the harness actually reads.

use super::propagate::propagate_persona_respond_to;
use crate::managed_agents::{AgentDefinition, ManagedAgentRecord, RespondTo};

fn agent(persona_id: &str, name: &str) -> ManagedAgentRecord {
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
        respond_to: RespondTo::OwnerOnly,
        respond_to_allowlist: vec![],
        display_name: Some(name.to_string()),
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

fn definition(respond_to: Option<&str>, allowlist: Vec<&str>) -> AgentDefinition {
    AgentDefinition {
        id: "persona-1".to_string(),
        display_name: "Scout".to_string(),
        avatar_url: None,
        system_prompt: String::new(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: Default::default(),
        respond_to: respond_to.map(str::to_string),
        respond_to_allowlist: allowlist.into_iter().map(str::to_string).collect(),
        parallelism: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

#[test]
fn respond_to_anyone_propagates_to_linked_instances() {
    let mut records = vec![agent("persona-1", "Scout"), agent("persona-2", "Other")];
    let updated = propagate_persona_respond_to(
        &mut records,
        "persona-1",
        &definition(Some("anyone"), vec![]),
    )
    .expect("propagate");

    assert_eq!(updated, 1);
    assert_eq!(records[0].respond_to, RespondTo::Anyone);
    assert!(records[0].respond_to_allowlist.is_empty());
    assert_eq!(records[1].respond_to, RespondTo::OwnerOnly);
}

#[test]
fn respond_to_allowlist_propagates_mode_and_entries() {
    let allow = "a".repeat(64);
    let mut records = vec![agent("persona-1", "Scout")];
    let updated = propagate_persona_respond_to(
        &mut records,
        "persona-1",
        &definition(Some("allowlist"), vec![&allow]),
    )
    .expect("propagate");

    assert_eq!(updated, 1);
    assert_eq!(records[0].respond_to, RespondTo::Allowlist);
    assert_eq!(records[0].respond_to_allowlist, vec![allow]);
}

#[test]
fn respond_to_non_allowlist_preserves_instance_allowlist_entries() {
    let existing = "b".repeat(64);
    let mut records = vec![agent("persona-1", "Scout")];
    records[0].respond_to = RespondTo::Allowlist;
    records[0].respond_to_allowlist = vec![existing.clone()];

    let updated = propagate_persona_respond_to(
        &mut records,
        "persona-1",
        &definition(Some("anyone"), vec![]),
    )
    .expect("propagate");

    assert_eq!(updated, 1);
    assert_eq!(records[0].respond_to, RespondTo::Anyone);
    assert_eq!(records[0].respond_to_allowlist, vec![existing]);
}

#[test]
fn respond_to_skips_empty_pubkey_definition_rows() {
    let mut records = vec![agent("persona-1", "Scout")];
    records[0].pubkey.clear();
    let updated = propagate_persona_respond_to(
        &mut records,
        "persona-1",
        &definition(Some("anyone"), vec![]),
    )
    .expect("propagate");
    assert_eq!(updated, 0);
}
