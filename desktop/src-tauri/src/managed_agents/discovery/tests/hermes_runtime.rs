use crate::managed_agents::discovery::{
    known_acp_runtime, known_acp_runtime_exact, normalize_agent_args,
};

/// Hermes is registered, resolvable by id, primary command, and alias.
#[test]
fn test_hermes_runtime_is_registered_and_resolvable() {
    let hermes = known_acp_runtime_exact("hermes").expect("hermes must be in the catalog");
    assert_eq!(hermes.commands, &["hermes"]);
    assert_eq!(known_acp_runtime("hermes").map(|r| r.id), Some("hermes"));
    assert_eq!(
        known_acp_runtime("hermes-agent").map(|r| r.id),
        Some("hermes"),
        "the hermes-agent alias must resolve to the hermes runtime"
    );
}

/// `hermes acp` is built into the CLI, so the runtime declares no separate
/// adapter but still needs CLI install commands on every platform.
#[test]
fn test_hermes_has_cli_install_but_no_adapter() {
    let hermes = known_acp_runtime_exact("hermes").unwrap();
    assert!(
        !hermes.cli_install_commands_for_os().is_empty(),
        "hermes must have install commands on every platform"
    );
    assert!(
        hermes.adapter_install_commands.is_empty(),
        "hermes ships ACP mode in the CLI — no adapter package"
    );
    assert!(hermes.adapter_install_instructions_url.is_empty());
}

/// Hermes owns provider/model selection in its own config.yaml, so Buzz must
/// not advertise env-var-driven model or provider injection for it.
#[test]
fn test_hermes_does_not_expose_model_or_provider_env() {
    let hermes = known_acp_runtime_exact("hermes").unwrap();
    assert!(hermes.model_env_var.is_none());
    assert!(hermes.provider_env_var.is_none());
    assert!(hermes.provider_locked);
    assert!(hermes.required_normalized_fields.is_empty());
    assert_eq!(hermes.config_file_path, Some("~/.hermes/config.yaml"));
}

/// Hermes resolves skills from `~/.hermes/skills` and `skills.external_dirs`,
/// so it declares no nest-relative skill directory for Buzz to symlink into.
#[test]
fn test_hermes_declares_no_nest_skill_dir() {
    let hermes = known_acp_runtime_exact("hermes").unwrap();
    assert!(hermes.skill_dir.is_none());
}

/// The Hermes CLI is launched as `hermes acp`, the same shape as Goose.
#[test]
fn test_hermes_defaults_to_acp_subcommand() {
    assert_eq!(
        normalize_agent_args("hermes", Vec::new()),
        vec!["acp".to_string()]
    );
    assert_eq!(
        normalize_agent_args("hermes", vec!["".into()]),
        vec!["acp".to_string()]
    );
}
