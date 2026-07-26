use super::runtime_metadata::KnownAcpRuntime;

pub(crate) const CURSOR_AVATAR_URL: &str = "https://cursor.com/brand/icon.svg";

pub(crate) const CURSOR_ACP_RUNTIME: KnownAcpRuntime = KnownAcpRuntime {
    id: "cursor",
    label: "Cursor Agent",
    commands: &["agent", "cursor-agent"],
    aliases: &[],
    avatar_url: CURSOR_AVATAR_URL,
    mcp_command: None,
    mcp_hooks: false,
    underlying_cli: None,
    cli_install_commands: &["curl https://cursor.com/install -fsS | bash"],
    // Cursor's documented installer is Unix/WSL. Native Windows support
    // remains intentionally absent; discovery provides WSL guidance.
    cli_install_commands_windows: &[],
    adapter_install_commands: &[],
    cli_install_instructions_url: "https://docs.cursor.com/en/cli/installation",
    adapter_install_instructions_url: "",
    cli_install_hint: "Install Cursor Agent CLI. On Windows, use Cursor CLI from WSL.",
    adapter_install_hint: "",
    skill_dir: Some(".cursor/skills"),
    supports_acp_model_switching: false,
    model_env_var: None,
    provider_env_var: None,
    provider_locked: true,
    default_env: &[],
    config_file_path: Some("~/.cursor/cli-config.json"),
    config_file_format: Some("json"),
    supports_acp_native_config: false,
    thinking_env_var: None,
    max_tokens_env_var: None,
    context_limit_env_var: None,
    required_normalized_fields: &[],
    login_hint: Some("Run `agent login` or `cursor-agent login` to authenticate with Cursor."),
    auth_probe_args: None,
    native_acp: true,
    startup_model_arg: Some("--model"),
};
