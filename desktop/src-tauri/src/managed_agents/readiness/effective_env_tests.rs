use std::collections::BTreeMap;

use super::*;
use crate::managed_agents::discovery::known_acp_runtime_exact;

#[test]
fn resolve_effective_agent_env_user_env_wins_over_structured_fields() {
    // A record whose env_vars explicitly set provider/model must win over
    // any baked defaults. In OSS test builds the baked map is empty, so
    // this test validates the user-env layer is present in the output.
    let mut env_vars = BTreeMap::new();
    env_vars.insert("BUZZ_AGENT_PROVIDER".to_string(), "anthropic".to_string());
    env_vars.insert(
        "BUZZ_AGENT_MODEL".to_string(),
        "claude-opus-4-5".to_string(),
    );

    // Minimal record: only the fields resolve_effective_agent_env reads.
    let record = crate::managed_agents::types::ManagedAgentRecord {
        pubkey: "test-pubkey".to_string(),
        name: "test-agent".to_string(),
        persona_id: None,
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: "buzz-agent".to_string(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars,
        filesystem_isolation: None,
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
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
    };

    let runtime = known_acp_runtime_exact("buzz-agent");
    let effective = resolve_effective_agent_env(&record, &[], runtime, &Default::default());

    // User env_vars must be present in the output (last-write-wins).
    assert_eq!(
        effective.env.get("BUZZ_AGENT_PROVIDER").map(String::as_str),
        Some("anthropic")
    );
    assert_eq!(
        effective.env.get("BUZZ_AGENT_MODEL").map(String::as_str),
        Some("claude-opus-4-5")
    );
}
