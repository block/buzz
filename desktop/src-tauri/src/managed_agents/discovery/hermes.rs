use super::KnownAcpRuntime;

pub(super) const HERMES_RUNTIME: KnownAcpRuntime = KnownAcpRuntime {
    id: "hermes",
    label: "Hermes Agent",
    commands: &["hermes-acp"],
    aliases: &["hermes-agent"],
    avatar_url: "",
    // Hermes terminal subprocesses intentionally sanitize signing secrets.
    // Give Hermes the credential-scoped Buzz MCP shell so message sends can
    // emit recipient p-tags without exposing the key to generic tools.
    mcp_command: Some("buzz-dev-mcp"),
    mcp_hooks: true,
    underlying_cli: Some("hermes"),
    cli_install_commands: &[],
    cli_install_commands_windows: &[],
    adapter_install_commands: &[],
    cli_install_instructions_url: "https://hermes-agent.nousresearch.com/docs",
    adapter_install_instructions_url: "https://hermes-agent.nousresearch.com/docs",
    cli_install_hint: "Install and configure Hermes Agent before using this runtime.",
    adapter_install_hint: "Hermes Agent provides the hermes-acp adapter.",
    skill_dir: Some(".hermes/skills"),
    supports_acp_model_switching: true,
    model_env_var: None,
    provider_env_var: None,
    provider_locked: false,
    default_env: &[],
    config_file_path: None,
    config_file_format: None,
    supports_acp_native_config: false,
    thinking_env_var: None,
    max_tokens_env_var: None,
    context_limit_env_var: None,
    max_rounds_env_var: None,
    required_normalized_fields: &[],
    login_hint: Some("Run `hermes setup` to configure a provider."),
    auth_probe_args: None,
};

#[cfg(test)]
mod tests {
    use super::super::{known_acp_runtime, normalize_agent_args};

    #[test]
    fn runtime_uses_credential_scoped_buzz_mcp() {
        let runtime = known_acp_runtime("/Users/test/.local/bin/hermes-acp")
            .expect("hermes-acp should resolve as a known runtime");

        assert_eq!(runtime.id, "hermes");
        assert_eq!(runtime.mcp_command, Some("buzz-dev-mcp"));
        assert!(runtime.mcp_hooks);
        assert_eq!(
            normalize_agent_args("hermes-acp", vec!["acp".into()]),
            Vec::<String>::new()
        );
    }
}
