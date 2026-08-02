use super::KnownAcpRuntime;

pub(super) const GROK_BUILD_AVATAR_URL: &str = "https://x.ai/favicon.ico";

pub(super) fn default_args() -> Vec<String> {
    vec!["agent".into(), "--always-approve".into(), "stdio".into()]
}

pub(super) const RUNTIME: KnownAcpRuntime = KnownAcpRuntime {
    id: "grok",
    label: "Grok Build",
    commands: &["grok"],
    aliases: &[],
    avatar_url: GROK_BUILD_AVATAR_URL,
    mcp_command: None,
    mcp_hooks: false,
    underlying_cli: Some("grok"),
    cli_install_commands: &["curl -fsSL https://x.ai/cli/install.sh | bash"],
    cli_install_commands_windows: &["powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \"irm https://x.ai/cli/install.ps1 | iex\""],
    adapter_install_commands: &[],
    cli_install_instructions_url: "https://docs.x.ai/build/overview",
    adapter_install_instructions_url: "",
    cli_install_hint: "Buzz talks to Grok Build through the Grok CLI's native ACP mode. Authenticate with `grok login`.",
    adapter_install_hint: "",
    skill_dir: Some(".grok/skills"),
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
    required_normalized_fields: &[],
    login_hint: None,
    auth_probe_args: None,
};
