use super::runtime::find_duplicate_keyed_name;
use super::{ManagedAgentRecord, RespondTo};

fn keyed(name: &str, pubkey: &str) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: pubkey.to_string(),
        name: name.to_string(),
        private_key_nsec: "nsec1fake".to_string(),
        persona_id: None,
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
        runtime_pid: None,
        backend: crate::managed_agents::BackendKind::Local,
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
        relay_mesh: None,
        auto_restart_on_config_change: false,
        definition_respond_to: None,
        definition_respond_to_allowlist: vec![],
        definition_parallelism: None,
    }
}

fn keyless(name: &str) -> ManagedAgentRecord {
    let mut r = keyed(name, "keyless");
    r.private_key_nsec = String::new();
    r.pubkey = String::new();
    r
}

#[test]
fn exact_case_collision_detected() {
    let records = vec![keyed("Duncan", "pk1")];
    assert!(find_duplicate_keyed_name(&records, "Duncan", None).is_some());
}

#[test]
fn case_insensitive_collision_detected() {
    let records = vec![keyed("Duncan", "pk1")];
    assert!(find_duplicate_keyed_name(&records, "duncan", None).is_some());
    assert!(find_duplicate_keyed_name(&records, "DUNCAN", None).is_some());
}

#[test]
fn whitespace_normalized_before_comparison() {
    let records = vec![keyed("  Duncan  ", "pk1")];
    assert!(find_duplicate_keyed_name(&records, "Duncan", None).is_some());
}

#[test]
fn no_match_when_names_differ() {
    let records = vec![keyed("Duncan", "pk1")];
    assert!(find_duplicate_keyed_name(&records, "Paul", None).is_none());
}

#[test]
fn keyless_records_excluded() {
    let records = vec![keyless("Duncan")];
    assert!(find_duplicate_keyed_name(&records, "Duncan", None).is_none());
}

#[test]
fn exclude_pubkey_skips_self() {
    let records = vec![keyed("Duncan", "pk1")];
    assert!(find_duplicate_keyed_name(&records, "Duncan", Some("pk1")).is_none());
}

#[test]
fn exclude_pubkey_still_catches_other_duplicate() {
    let records = vec![keyed("Duncan", "pk1"), keyed("duncan", "pk2")];
    assert!(find_duplicate_keyed_name(&records, "Duncan", Some("pk1")).is_some());
}

#[test]
fn multiple_keyed_records_first_match_returned() {
    let records = vec![
        keyed("Duncan", "pk1"),
        keyed("duncan", "pk2"),
        keyed("DUncAn", "pk3"),
    ];
    let found = find_duplicate_keyed_name(&records, "Duncan", None);
    assert!(found.is_some());
    assert_eq!(found.unwrap().pubkey, "pk1");
}

#[test]
fn empty_record_list_no_match() {
    let records: Vec<ManagedAgentRecord> = vec![];
    assert!(find_duplicate_keyed_name(&records, "Duncan", None).is_none());
}
