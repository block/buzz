//! Effective-launch resolution tests — `resolve_effective_agent_env` and
//! `resolve_effective_harness_descriptor`.
//!
//! Split out of `readiness.rs`'s inline `mod tests` to keep that module
//! under the desktop file-size ratchet. Included via `#[path]`; `super::*`
//! resolves against `readiness.rs`, matching the `storage_tests.rs` convention.

use std::collections::BTreeMap;

use super::*;
use crate::managed_agents::discovery::known_acp_runtime_exact;

// ── resolve_effective_agent_env ─────────────────────────────────────────

#[test]
fn resolve_effective_agent_env_user_env_wins_over_structured_fields() {
    // User env_vars must win over baked defaults; in OSS builds baked map is empty,
    // so this validates the user-env layer is present in the output.
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
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_policy_pending: false,
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
        effort_level: None,
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

#[test]
fn resolve_effective_harness_descriptor_uses_preset_args_for_pinned_command() {
    // An agent pinned to a known ACP runtime via agent_command_override,
    // with no runtime id and no instance args, must take the launch args
    // from the matching PRESET_HARNESSES entry. Otherwise the bare command
    // (e.g. `omp`) launches the interactive TUI, which times out at ACP
    // `initialize` under headless Buzz (no controlling TTY).
    let mk = |command: &str| crate::managed_agents::types::ManagedAgentRecord {
        pubkey: "k".to_string(),
        name: "agent".to_string(),
        agent_command_override: Some(command.to_string()),
        agent_args: vec![],
        runtime: None,
        persona_id: None,
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: String::new(),
        mcp_command: String::new(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_policy_pending: false,
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
        effort_level: None,
    };
    let global = crate::managed_agents::GlobalAgentConfig::default();

    let desc =
        |command: &str| resolve_effective_harness_descriptor(&mk(command), &[], &global).unwrap();

    // ACP-mode runtimes resolve to `acp`.
    assert_eq!(desc("devin").args, vec!["acp".to_string()]);
    assert_eq!(desc("omp").args, vec!["acp".to_string()]);
    assert_eq!(desc("opencode").args, vec!["acp".to_string()]);
    assert_eq!(desc("kimi").args, vec!["acp".to_string()]);
    assert_eq!(desc("cursor-agent").args, vec!["acp".to_string()]);
    assert_eq!(desc("openclaw").args, vec!["acp".to_string()]);
    // Grok launches its agent stdio mode (not the TUI).
    assert_eq!(
        desc("grok").args,
        vec![
            "agent".to_string(),
            "--always-approve".to_string(),
            "stdio".to_string()
        ]
    );

    // Explicit instance args always win over the preset fallback.
    let mut explicit = mk("omp");
    explicit.agent_args = vec!["acp".to_string(), "--foo".to_string()];
    let explicit_desc = resolve_effective_harness_descriptor(&explicit, &[], &global).unwrap();
    assert_eq!(
        explicit_desc.args,
        vec!["acp".to_string(), "--foo".to_string()]
    );

    // An unknown command with no preset keeps the (empty) instance args.
    assert!(desc("my-fancy-cli").args.is_empty());
}
